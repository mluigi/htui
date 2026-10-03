//! `command_run` (MOD-11 M4, plan D14-D16, OQ-3, OQ-7, blueprint §11.2): a build, test or run
//! command through the box's queue.
//!
//! The call is validated (class, `cwd`, timeout), its command line scrubbed (I-5), and enqueued
//! as a `queued` `command_run` row of the session's step on the session's box. It then asks for
//! admission every [`COMMAND_ADMIT_POLL`] under the class limit D15 resolves per call, ticking
//! progress (B-11); runs through [`run_shell`] in the session's directory, beating the claim every
//! [`COMMAND_HEARTBEAT`] (a beat that answers `false` kills the child: the row was reaped or
//! cancelled); scrubs the tail, fail closed; finishes the row; and answers it. Everything a
//! cancelled call must undo lives in [`Enqueued`]'s drop (H-18): the row is cancelled and the
//! child's process group dies with the dropped future.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use htui_core::model::kind::{command_limit, resolve_command_limits};
use htui_core::model::{CommandRun, CommandRunId, CommandRunStatus, NewCommandRun};
use htui_core::scrub::Scrubber;
use htui_core::store::traits::COMMAND_HEARTBEAT;
use htui_orch::verify::{ShellEnd, ShellRun, run_shell};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, scrubbed, store_error};
use crate::protocol::Progress;

/// How often a queued call asks for admission (blueprint §16).
pub(crate) const COMMAND_ADMIT_POLL: Duration = Duration::from_secs(1);

/// The longest timeout a call may ask for, and the default (OQ-7, H-13), in seconds.
pub(crate) const MAX_TIMEOUT_SECS: u64 = 1800;

/// The classes an agent may queue; `verify` is the orchestrator's (PRD OQ-6).
const CLASSES: [&str; 3] = ["build", "test", "run"];

/// The line a cut tail opens with (OQ-7).
const TRUNCATED: &str = "[… earlier output truncated]\n";

// The step, the box and the directory are the scope's (I-1): a `run_id` or a `step_id` is
// refused. A plain comment, not a doc: schemars would hand a doc to the agent as the schema's
// description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandArgs {
    /// `build`, `test` or `run`: the queue class whose per-box limit applies.
    #[schemars(schema_with = "class_schema")]
    class: String,
    /// The command line, run through `sh -c` (`cmd /C` on Windows).
    command: String,
    /// A directory relative to the session's working directory; default the directory itself.
    cwd: Option<String>,
    /// Seconds before the command and everything it started are killed, 1 to 1800; default 1800.
    timeout_secs: Option<u64>,
}

/// `{"type": "string", "enum": ["build", "test", "run"]}`.
fn class_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({"type": "string", "enum": CLASSES})
}

/// Advertised when D16 exposed the queue to this session.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "command_run",
    description: "Runs a build, test or run command through this box's command queue and returns its output.",
    schema: schema_of::<CommandArgs>,
    advertised: |scope, _| scope.command_queue,
};

/// `{"exit_code": n|null, "status": "done"|"failed"|"cancelled", "output": "...",
/// "truncated": bool, "command_run_id": "<uuid>"}`.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let CommandArgs {
        class,
        command,
        cwd,
        timeout_secs,
    } = args(arguments)?;
    let session = ctx.session;
    let scope = &session.scope;
    if class == "verify" {
        return Err(ToolError(
            "refused: verify is the orchestrator's".to_owned(),
        ));
    }
    if !CLASSES.contains(&class.as_str()) {
        return Err(ToolError(format!(
            "refused: class `{class}` is not build, test or run"
        )));
    }
    if command.trim().is_empty() {
        return Err(ToolError("refused: the command is empty".to_owned()));
    }
    if command.contains('\0') {
        return Err(ToolError("refused: the command holds a NUL".to_owned()));
    }
    let dir = directory(&scope.cwd, cwd.as_deref())?;
    let timeout = timeout_secs.unwrap_or(MAX_TIMEOUT_SECS);
    if !(1..=MAX_TIMEOUT_SECS).contains(&timeout) {
        return Err(ToolError(format!(
            "refused: timeout_secs {timeout} is not 1..=1800"
        )));
    }
    // I-5: the row keeps the command line, so it is scrubbed, fail closed, before anything is
    // written; the shell runs what the agent wrote.
    let stored_command = scrubbed(session.scrubber.as_ref(), command.clone())?;

    // D15, read per call through the session's host: the box's own limits over the app's.
    let box_limits = ctx
        .host
        .box_row(scope.box_id)
        .await
        .map_err(store_error)?
        .and_then(|row| row.settings.get("command_limits").cloned());
    let app = ctx.host.app_settings().await.map_err(store_error)?;
    let limit = command_limit(&resolve_command_limits(box_limits.as_ref(), &app), &class);

    let store = session.store.clone();
    let queued = htui_core::store::WorkerStore::enqueue_command(
        &store,
        NewCommandRun {
            id: CommandRunId::new(),
            run_step_id: scope.step_id,
            box_id: scope.box_id,
            class,
            command: stored_command,
            cwd: dir.display().to_string(),
            status: CommandRunStatus::Queued,
            exit_code: None,
            output: None,
            queued_at: session.clock.now(),
            started_at: None,
            finished_at: None,
        },
    )
    .await
    .map_err(store_error)?;
    let mut guard = Enqueued {
        store: Some(store.clone()),
        id: queued.id,
    };
    let id = queued.id;
    let claimant = Uuid::now_v7();
    let progress = ctx.progress;
    let mut ticks = 0_u64;

    // D14: ask until admitted, ticking progress while queued (B-11).
    loop {
        match htui_core::store::WorkerStore::claim_command(&store, id, claimant, limit).await {
            Ok(Some(_)) => break,
            Ok(None) => {
                tick(progress.as_ref(), &mut ticks).await;
                tokio::time::sleep(COMMAND_ADMIT_POLL).await;
            }
            Err(err) => return Err(store_error(err)),
        }
    }

    // OQ-3: the claim beats every `COMMAND_HEARTBEAT`; a beat answering `false` (reaped,
    // cancelled) is `run_shell`'s `stop`. A beat the store fails is not a stop: a lasting
    // failure lets the row go stale, and the next beat after it answers `false`.
    let stopped = AtomicBool::new(false);
    let beats = async {
        loop {
            tokio::time::sleep(COMMAND_HEARTBEAT).await;
            tick(progress.as_ref(), &mut ticks).await;
            if let Ok(false) =
                htui_core::store::WorkerStore::beat_command(&store, id, claimant).await
            {
                stopped.store(true, Ordering::SeqCst);
                return;
            }
        }
    };
    let ran = run_shell(&command, &dir, Duration::from_secs(timeout), beats).await;

    let (status, exit_code) = match ran.ended {
        ShellEnd::Exited => (CommandRunStatus::Done, ran.exit_code),
        ShellEnd::TimedOut | ShellEnd::Signalled | ShellEnd::SpawnFailed => {
            (CommandRunStatus::Failed, None)
        }
    };
    let output = answer_output(session.scrubber.as_ref(), &ran, timeout);
    let finished = if stopped.load(Ordering::SeqCst) {
        false
    } else {
        htui_core::store::WorkerStore::finish_command(
            &store,
            id,
            claimant,
            status,
            exit_code,
            Some(output.clone()),
        )
        .await
        .map_err(store_error)?
    };
    guard.disarm();
    if finished {
        return Ok(json!({
            "exit_code": exit_code,
            "status": status.as_str(),
            "output": output,
            "truncated": ran.truncated,
            "command_run_id": id,
        }));
    }

    // The queue took the row from under the claim (reaped or cancelled): answer it as the queue
    // left it, with what the command printed before it was stopped.
    let row = stored_row(&store, scope.step_id, id).await?;
    let output = match row.output {
        Some(note) if !note.is_empty() => format!("{note}\n{output}"),
        _ => format!("[stopped: the queue {} this command]\n{output}", row.status),
    };
    Ok(json!({
        "exit_code": Value::Null,
        "status": row.status.as_str(),
        "output": output,
        "truncated": ran.truncated,
        "command_run_id": id,
    }))
}

/// OQ-7: the session's directory, or a relative path under it that exists, with no `..`.
fn directory(base: &Path, cwd: Option<&str>) -> Result<PathBuf, ToolError> {
    let Some(cwd) = cwd else {
        return Ok(base.to_path_buf());
    };
    let relative = Path::new(cwd);
    let inside = !cwd.is_empty()
        && relative
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if !inside {
        return Err(ToolError(format!(
            "refused: cwd `{cwd}` must be a relative path inside the session's directory, \
             without `..`"
        )));
    }
    let joined = base.join(relative);
    if !joined.is_dir() {
        return Err(ToolError(format!(
            "refused: cwd `{cwd}` is not a directory under the session's directory"
        )));
    }
    Ok(joined)
}

/// The output as stored and answered: the reason a killed command ended, the truncation marker
/// when the tail was cut, then the tail — scrubbed, fail closed (I-5: a tail that cannot be
/// masked is withheld by rule, never stored), with any NUL replaced (`TEXT` holds none).
fn answer_output(scrubber: &dyn Scrubber, ran: &ShellRun, timeout: u64) -> String {
    let mut value = Value::String(ran.output.replace('\0', "\u{fffd}"));
    let tail = match scrubber.scrub(&mut value) {
        Ok(()) => match value {
            Value::String(text) => text,
            other => other.to_string(),
        },
        Err(unmasked) => {
            return format!(
                "[output withheld: it matched credential rule {}]",
                unmasked.rule
            );
        }
    };
    let marker = if ran.truncated { TRUNCATED } else { "" };
    match ran.ended {
        ShellEnd::TimedOut => format!("[killed: the {timeout} s timeout elapsed]\n{marker}{tail}"),
        _ => format!("{marker}{tail}"),
    }
}

/// The row `id` of `step`, read back after the queue moved it.
async fn stored_row<S: htui_core::store::WorkerStore>(
    store: &S,
    step: htui_core::model::StepId,
    id: CommandRunId,
) -> Result<CommandRun, ToolError> {
    htui_core::store::WorkerStore::command_runs(store, step)
        .await
        .map_err(store_error)?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| ToolError(format!("not found: command_run {id}")))
}

/// One progress tick, numbered from 1, when the client asked for progress.
async fn tick(progress: Option<&Progress>, ticks: &mut u64) {
    if let Some(progress) = progress {
        *ticks += 1;
        progress.tick(*ticks).await;
    }
}

/// H-18: a queued or running row this call owns. Dropped armed — the call was cancelled, its
/// connection closed, or it failed after the enqueue — it cancels the row in a contained task.
struct Enqueued<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static> {
    /// The store the row is in; `None` once disarmed.
    store: Option<S>,
    /// The row.
    id: CommandRunId,
}

impl<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static> Enqueued<S> {
    /// The call finished the row itself (or the queue did): nothing to undo.
    fn disarm(&mut self) {
        self.store = None;
    }
}

impl<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static> Drop for Enqueued<S> {
    fn drop(&mut self) {
        let Some(store) = self.store.take() else {
            return;
        };
        let id = self.id;
        if tokio::runtime::Handle::try_current().is_ok() {
            drop(htui_agent::contained::spawn(async move {
                if let Err(err) = htui_core::store::WorkerStore::cancel_command(&store, id).await {
                    tracing::warn!(%id, %err, "a dropped command_run could not cancel its row");
                }
            }));
        }
    }
}
