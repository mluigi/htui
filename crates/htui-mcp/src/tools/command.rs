//! `command_run` (MOD-11 M4, plan D14-D16, OQ-3, OQ-7, blueprint §11.2): a build, test or run
//! command through the box's queue.
//!
//! The call is validated by [`check`] (class, `repo` and `cwd`, timeout), its command line scrubbed
//! (I-5), and [`Runner::enqueue`] enqueues it as a `queued` `command_run` row of the session's
//! step on the session's box. [`Runner::run`] then asks for admission every
//! [`COMMAND_ADMIT_POLL`] under the class limit D15 resolves per call, ticking
//! progress (B-11; a tick never waits long on a client that does not read); runs through
//! [`run_shell_with`] in the session's directory, its merged output streamed into a
//! [`LogBuffer`] that is scrubbed and stored as the row's log before the row ends (MOD-152 D4),
//! beating the claim every [`COMMAND_HEARTBEAT`] (a
//! beat that answers `false` kills the child: the row was reaped or cancelled); scrubs the tail,
//! fail closed; finishes the row; and answers it. Everything a cancelled call must undo lives in
//! [`Enqueued`]'s drop (H-18): the row is cancelled and the child's process group dies with the
//! dropped future.
//!
//! The queue's own writes take no fence, so the call reads the session's [`Lease`] (MOD-78 D3):
//! before it queues, at most once per heartbeat while queued, once at admission and on every
//! heartbeat while running. A lost lease answers `fenced: lease lost`; a running child is
//! killed and its row ended `cancelled` (D4). Only the read before the queue refuses on a store
//! failure; anywhere later a failed read is not a stop (D5).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use htui_core::clock::Clock;
use htui_core::model::kind::{command_limit, resolve_command_limits};
use htui_core::model::{
    BoxId, CommandLog, CommandRun, CommandRunId, CommandRunStatus, NewCommandRun, RunId, StepId,
};
use htui_core::scrub::Scrubber;
use htui_core::store::traits::COMMAND_HEARTBEAT;
use htui_core::store::{StepFence, StoreError};
use htui_orch::tools::{ExtraDirRefusal, ToolScope};
use htui_orch::verify::{ShellEnd, ShellRun, run_shell_with};
use serde_json::{Value, json};
use uuid::Uuid;

use super::command_log::{LogBuffer, SharedLog, TRUNCATED};
use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, scrubbed, store_error};
use crate::host::Session;
use crate::protocol::Progress;

/// How often a queued call asks for admission (blueprint §16).
pub(crate) const COMMAND_ADMIT_POLL: Duration = Duration::from_secs(1);

/// How long one progress tick may wait for room on the connection before it is dropped.
const TICK_PATIENCE: Duration = Duration::from_millis(250);

/// The longest timeout a call may ask for, and the default (OQ-7, H-13), in seconds.
pub(crate) const MAX_TIMEOUT_SECS: u64 = 1800;

/// The classes an agent may queue; `verify` is the orchestrator's (PRD OQ-6).
const CLASSES: [&str; 3] = ["build", "test", "run"];

/// MOD-11 R1 L3: the answer to a call while the session's previous one is queued or running.
pub(crate) const BUSY: &str = "refused: a command_run is already queued or running in this session";

/// The line a row stopped on a lost lease opens with (MOD-78 D4).
const LEASE_LOST: &str = "[stopped: fenced: lease lost]\n";

/// MOD-78 D1, D3: the lease the session's writes are fenced on, and the step a loss is
/// reported on.
#[derive(Debug, Clone, Copy)]
struct Lease {
    /// The session's run.
    run: RunId,
    /// The session's fence on it.
    fence: StepFence,
    /// The session's step.
    step: StepId,
}

impl Lease {
    /// The scope's run, fence and step.
    fn of(scope: &ToolScope) -> Self {
        Self {
            run: scope.run_id,
            fence: scope.fence,
            step: scope.step_id,
        }
    }

    /// MOD-78 D1: a lock-free read of whether the run still carries the fence's lease.
    async fn holds<S: htui_core::store::WorkerStore>(self, store: &S) -> Result<bool, StoreError> {
        htui_core::store::WorkerStore::lease_holds(store, self.run, self.fence).await
    }

    /// MOD-78 D3(b)-(d), D5: whether a read bounded by [`COMMAND_HEARTBEAT`] answers that the
    /// lease is lost. A failed read and one the store does not answer in time (a starved pool)
    /// are not a stop: a hung read must not hold back the beat after it and get the row reaped.
    async fn lost_now<S: htui_core::store::WorkerStore>(self, store: &S) -> bool {
        matches!(
            tokio::time::timeout(COMMAND_HEARTBEAT, self.holds(store)).await,
            Ok(Ok(false))
        )
    }

    /// The answer every fenced tool gives ([`store_error`]).
    fn lost(self) -> ToolError {
        store_error(StoreError::Fenced { step: self.step })
    }
}

/// MOD-78 D4: why [`beat`] returned, which is why [`run_shell_with`] stopped the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stopped {
    /// A beat answered `false`: the queue reaped or cancelled the row.
    Taken,
    /// The session's lease is lost.
    LeaseLost,
    /// The caller stopped the command (`command_cancel`, the end of the session); the line the
    /// row's output opens with, which says which (MOD-152 D7).
    Cancelled(&'static str),
}

/// What watches a command run (MOD-152 D7): the log its output is written into, told when a line
/// was written and when the command was admitted. A blocking `command_run` watches with a bare
/// [`SharedLog`]; a started command with its registry entry.
pub(crate) trait Watch: Sync {
    /// The log the output streams into.
    fn log(&self) -> &SharedLog;
    /// Output holding a newline was written.
    fn wrote(&self) {}
    /// The command was admitted and is about to run.
    fn running(&self) {}
}

impl Watch for SharedLog {
    fn log(&self) -> &SharedLog {
        self
    }
}

// The step, the box and the directory are the scope's (I-1): a `run_id` or a `step_id` is
// refused. A plain comment, not a doc: schemars would hand a doc to the agent as the schema's
// description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandArgs {
    /// `build`, `test` or `run`: the queue class whose per-box limit applies.
    #[schemars(schema_with = "class_schema")]
    pub(crate) class: String,
    /// The command line, run through `sh -c` (`cmd /C` on Windows).
    pub(crate) command: String,
    /// A directory relative to the session's working directory (or to `repo`); default the
    /// directory itself.
    pub(crate) cwd: Option<String>,
    /// Seconds before the command and everything it started are killed, 1 to 1800; default 1800.
    pub(crate) timeout_secs: Option<u64>,
    /// One of the session's other directories, by its last path component; default the session's
    /// own working directory.
    pub(crate) repo: Option<String>,
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

/// What [`check`] accepted: a command ready to be queued.
#[derive(Debug)]
pub(crate) struct Checked {
    /// `build`, `test` or `run`.
    class: String,
    /// The command line the shell runs.
    command: String,
    /// The command line as the row keeps it: scrubbed (I-5).
    stored_command: String,
    /// The directory it runs in.
    dir: PathBuf,
    /// Seconds before it is killed, 1 to [`MAX_TIMEOUT_SECS`].
    timeout: u64,
}

impl Checked {
    /// The queue class.
    pub(crate) fn class(&self) -> &str {
        &self.class
    }
}

/// Validates a call's arguments against the session's scope (MOD-152 D11 for `repo`): the class,
/// the command line, the directory — the session's own, or the `repo`'s, then `cwd` under it — and
/// the timeout; then scrubs the command line, fail closed, before anything is written (I-5).
pub(crate) fn check(
    arguments: CommandArgs,
    scope: &ToolScope,
    scrubber: &dyn Scrubber,
) -> Result<Checked, ToolError> {
    let CommandArgs {
        class,
        command,
        cwd,
        timeout_secs,
        repo,
    } = arguments;
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
    let base = match repo.as_deref() {
        None => scope.cwd.as_path(),
        Some(name) => scope.extra_dir(name).map_err(repo_refusal)?,
    };
    let dir = directory(base, cwd.as_deref())?;
    let timeout = timeout_secs.unwrap_or(MAX_TIMEOUT_SECS);
    if !(1..=MAX_TIMEOUT_SECS).contains(&timeout) {
        return Err(ToolError(format!(
            "refused: timeout_secs {timeout} is not 1..=1800"
        )));
    }
    // I-5: the row keeps the command line, so it is scrubbed, fail closed, before anything is
    // written; the shell runs what the agent wrote.
    let stored_command = scrubbed(scrubber, command.clone())?;
    Ok(Checked {
        class,
        command,
        stored_command,
        dir,
        timeout,
    })
}

/// MOD-152 D11: why a `repo` named no directory, as the one-line refusal.
fn repo_refusal(refusal: ExtraDirRefusal) -> ToolError {
    ToolError(match refusal {
        ExtraDirRefusal::Unknown { name, available } if available.is_empty() => {
            format!("refused: repo `{name}` is not one of: (none)")
        }
        ExtraDirRefusal::Unknown { name, available } => {
            format!(
                "refused: repo `{name}` is not one of: {}",
                available.join(", ")
            )
        }
        ExtraDirRefusal::Ambiguous { name } => {
            format!("refused: repo `{name}` names more than one directory")
        }
    })
}

/// `{"exit_code": n|null, "status": "done"|"failed"|"cancelled", "output": "...",
/// "truncated": bool, "command_run_id": "<uuid>"}`.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let session = ctx.session;
    let checked = check(args(arguments)?, &session.scope, session.scrubber.as_ref())?;
    // MOD-11 R1 L3: one call per session past this point, held until the call ends or is dropped.
    let _slot = Arc::clone(&session.command_slot)
        .try_acquire_owned()
        .map_err(|_| ToolError(BUSY.to_owned()))?;
    let limit = class_limit(&ctx.host, &session.scope, checked.class()).await?;
    let runner = Runner::of(session, limit);
    let queued = runner.enqueue(CommandRunId::new(), checked).await?;
    let finished = runner
        .run(
            queued,
            ctx.progress.as_ref(),
            std::future::pending(),
            &SharedLog::default(),
        )
        .await?;
    Ok(finished.answer())
}

/// D15, read per call through the session's host: the box's own limits over the app's, for
/// `class`.
pub(crate) async fn class_limit<H: htui_core::store::WorkerHost>(
    host: &H,
    scope: &ToolScope,
    class: &str,
) -> Result<u32, ToolError> {
    let box_limits = host
        .box_row(scope.box_id)
        .await
        .map_err(store_error)?
        .and_then(|row| row.settings.get("command_limits").cloned());
    let app = host.app_settings().await.map_err(store_error)?;
    Ok(command_limit(
        &class_limits(scope.box_id, box_limits.as_ref(), &app),
        class,
    ))
}

/// How a checked command ended, as [`Runner::run`] answers it.
#[derive(Debug)]
pub(crate) struct Finished {
    /// The `command_run` row.
    pub(crate) id: CommandRunId,
    /// The row's status when it ended.
    pub(crate) status: CommandRunStatus,
    /// The exit code, when the command exited.
    pub(crate) exit_code: Option<i32>,
    /// The scrubbed tail with the reason a killed command ended, as the row holds it.
    pub(crate) output: String,
    /// Whether more was printed than the tail kept.
    pub(crate) truncated: bool,
}

impl Finished {
    /// `command_run`'s answer.
    fn answer(&self) -> Value {
        json!({
            "exit_code": self.exit_code,
            "status": self.status.as_str(),
            "output": self.output,
            "truncated": self.truncated,
            "command_run_id": self.id,
        })
    }
}

/// What running one command needs beyond the call that asked for it: owned, so a background task
/// can hold it (MOD-152 D7). The limit is the class's, read by [`class_limit`] before.
pub(crate) struct Runner<S> {
    /// The session's store.
    store: S,
    /// The session's lease (MOD-78 D1).
    lease: Lease,
    /// The box the row is queued on.
    box_id: BoxId,
    /// The step the row belongs to.
    step: StepId,
    /// I-5: the host's scrubber.
    scrubber: Arc<dyn Scrubber>,
    /// The host's clock.
    clock: Arc<dyn Clock>,
    /// The class limit admission asks under.
    limit: u32,
}

impl<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static> Runner<S> {
    /// The session's runner under class `limit`.
    pub(crate) fn of(session: &Session<S>, limit: u32) -> Self {
        Self {
            store: session.store.clone(),
            lease: Lease::of(&session.scope),
            box_id: session.scope.box_id,
            step: session.scope.step_id,
            scrubber: Arc::clone(&session.scrubber),
            clock: Arc::clone(&session.clock),
            limit,
        }
    }

    /// Queues `checked` as the row `id`, once the lease is read (MOD-78 D3(a), R1: a lost lease
    /// queues nothing; a failed read refuses too, D5: nothing has been written yet, so there is
    /// nothing a refusal could strand). Dropped before [`run`](Self::run) ends it, the answer
    /// cancels the row (H-18).
    pub(crate) async fn enqueue(
        &self,
        id: CommandRunId,
        checked: Checked,
    ) -> Result<Queued<S>, ToolError> {
        let Checked {
            class,
            command,
            stored_command,
            dir,
            timeout,
        } = checked;
        let store = &self.store;
        let lease = self.lease;
        if !lease.holds(store).await.map_err(store_error)? {
            return Err(lease.lost());
        }
        let queued = htui_core::store::WorkerStore::enqueue_command(
            store,
            NewCommandRun {
                id,
                run_step_id: self.step,
                box_id: self.box_id,
                class,
                command: stored_command,
                cwd: dir.display().to_string(),
                status: CommandRunStatus::Queued,
                exit_code: None,
                output: None,
                queued_at: self.clock.now(),
                started_at: None,
                finished_at: None,
            },
        )
        .await
        .map_err(store_error)?;
        Ok(Queued {
            guard: Enqueued {
                store: Some(store.clone()),
                id: queued.id,
            },
            id: queued.id,
            command,
            dir,
            timeout,
        })
    }

    /// Waits for `queued`'s admission, runs it and ends its row.
    ///
    /// The lease is read at admission and on every heartbeat; a lost one answers `fenced: lease
    /// lost` and ends a running row `cancelled` (MOD-78). `halt` resolving — to the line the row's
    /// output then opens with — cancels a queued row, or stops the child and ends a running row
    /// `cancelled` (MOD-152 D7). The command's merged output is written into `watch`'s log as it
    /// arrives; once the command is over the log is scrubbed and stored before the row ends (D4),
    /// so whoever sees the row terminal can read it. Dropped before the row ends, the call
    /// cancels the row (H-18).
    pub(crate) async fn run(
        &self,
        queued: Queued<S>,
        progress: Option<&Progress>,
        halt: impl Future<Output = &'static str> + Send,
        watch: &impl Watch,
    ) -> Result<Finished, ToolError> {
        let Queued {
            mut guard,
            id,
            command,
            dir,
            timeout,
        } = queued;
        let store = &self.store;
        let scrubber = self.scrubber.as_ref();
        let lease = self.lease;
        let claimant = Uuid::now_v7();
        let mut ticks = 0_u64;
        tokio::pin!(halt);

        tokio::select! {
            admitted = admit(store, id, claimant, self.limit, lease, progress, &mut ticks) => admitted?,
            why = &mut halt => return self.cancel_queued(id, why, &mut guard).await,
        }
        // MOD-78 D3(b): a call that waited may have lost the lease since its last read; it must
        // not spawn a child for a heartbeat on it. The armed guard cancels the running row. A
        // failed read is not a stop (D5): the heartbeat reads again.
        if lease.lost_now(store).await {
            return Err(lease.lost());
        }
        watch.running();

        // OQ-3: the claim beats until the queue takes the row, the lease is lost (MOD-78 D3(c)) or
        // the caller stops it; that is `run_shell_with`'s `stop`.
        let stopped = OnceLock::new();
        let beats = async {
            let why = tokio::select! {
                why = beat(store, id, claimant, lease, progress, &mut ticks) => why,
                why = &mut halt => Stopped::Cancelled(why),
            };
            // Set once: `beats` runs once.
            let _ = stopped.set(why);
        };
        let sink = |chunk: &[u8]| {
            lock(watch.log()).push(chunk);
            if chunk.contains(&b'\n') {
                watch.wrote();
            }
        };
        let ran = run_shell_with(&command, &dir, Duration::from_secs(timeout), beats, &sink).await;

        let (status, exit_code) = match ran.ended {
            ShellEnd::Exited => (CommandRunStatus::Done, ran.exit_code),
            ShellEnd::TimedOut | ShellEnd::Signalled | ShellEnd::SpawnFailed => {
                (CommandRunStatus::Failed, None)
            }
        };
        let output = answer_output(scrubber, &ran, timeout);
        // D4: the log is written before the row ends. A failed write is not a stop: the row's own
        // tail still answers. The live log keeps its text until the row has ended, so a reader
        // of a running command never sees it empty.
        let buffer = {
            let mut live = lock(watch.log());
            if ran.ended == ShellEnd::SpawnFailed {
                // The sink saw nothing of a shell that never started; its reason is the log.
                live.push(ran.output.as_bytes());
            }
            live.clone()
        };
        let stored = buffer.into_stored(scrubber, id, self.clock.now());
        if let Err(err) = htui_core::store::WorkerStore::put_command_log(store, stored).await {
            tracing::warn!(%id, %err, "a command's log could not be written");
        }
        let truncated = ran.truncated;
        match stopped.get() {
            Some(Stopped::LeaseLost) => {
                finish_lease_lost(store, id, claimant, &output, &mut guard).await;
                return Err(lease.lost());
            }
            Some(Stopped::Cancelled(why)) => {
                let note = format!("{why}{output}");
                let ended = htui_core::store::WorkerStore::finish_command(
                    store,
                    id,
                    claimant,
                    CommandRunStatus::Cancelled,
                    None,
                    Some(note.clone()),
                )
                .await;
                // `Err`: the armed guard cancels the row.
                if ended.map_err(store_error)? {
                    guard.disarm();
                    return Ok(Finished {
                        id,
                        status: CommandRunStatus::Cancelled,
                        exit_code: None,
                        output: note,
                        truncated,
                    });
                }
            }
            Some(Stopped::Taken) => {}
            None => {
                let finished = htui_core::store::WorkerStore::finish_command(
                    store,
                    id,
                    claimant,
                    status,
                    exit_code,
                    Some(output.clone()),
                )
                .await
                .map_err(store_error)?;
                if finished {
                    guard.disarm();
                    return Ok(Finished {
                        id,
                        status,
                        exit_code,
                        output,
                        truncated,
                    });
                }
            }
        }
        guard.disarm();

        // The queue took the row from under the claim (reaped or cancelled): answer it as the
        // queue left it, with what the command printed before it was stopped.
        let row = stored_row(store, lease.step, id).await?;
        let output = match row.output {
            Some(note) if !note.is_empty() => format!("{note}\n{output}"),
            _ => format!("[stopped: the queue {} this command]\n{output}", row.status),
        };
        Ok(Finished {
            id,
            status: row.status,
            exit_code: None,
            output,
            truncated,
        })
    }

    /// MOD-152 D7: ends a row halted before it was admitted `cancelled`. It never ran, so its log
    /// is the reason (written first, D4); the row keeps no output.
    async fn cancel_queued(
        &self,
        id: CommandRunId,
        why: &'static str,
        guard: &mut Enqueued<S>,
    ) -> Result<Finished, ToolError> {
        let store = &self.store;
        let reason = CommandLog {
            command_run_id: id,
            text: why.to_owned(),
            truncated: false,
            total_bytes: 0,
            written_at: self.clock.now(),
        };
        if let Err(err) = htui_core::store::WorkerStore::put_command_log(store, reason).await {
            tracing::warn!(%id, %err, "a cancelled command's reason could not be written");
        }
        // `Err`: the armed guard cancels the row.
        htui_core::store::WorkerStore::cancel_command(store, id)
            .await
            .map_err(store_error)?;
        guard.disarm();
        let row = stored_row(store, self.step, id).await?;
        Ok(Finished {
            id,
            status: row.status,
            exit_code: row.exit_code,
            output: why.to_owned(),
            truncated: false,
        })
    }
}

impl<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static> Runner<S> {
    /// How the row `id` stands once [`run`](Self::run) answered an error (a lost lease, a store
    /// failure): as the store holds it, cancelled first when it is still queued or running. A
    /// store that cannot answer leaves it `cancelled` as far as the caller knows; the queue reaps
    /// the row.
    pub(crate) async fn settled(&self, id: CommandRunId) -> (CommandRunStatus, Option<i32>) {
        let open = |status| matches!(status, CommandRunStatus::Queued | CommandRunStatus::Running);
        if let Ok(row) = stored_row(&self.store, self.step, id).await
            && !open(row.status)
        {
            return (row.status, row.exit_code);
        }
        if let Err(err) = htui_core::store::WorkerStore::cancel_command(&self.store, id).await {
            tracing::warn!(%id, %err, "a started command's row could not be cancelled");
        }
        match stored_row(&self.store, self.step, id).await {
            Ok(row) if !open(row.status) => (row.status, row.exit_code),
            _ => (CommandRunStatus::Cancelled, None),
        }
    }
}

/// A row [`Runner::enqueue`] queued and [`Runner::run`] has not yet ended: dropped, it cancels
/// the row (H-18).
pub(crate) struct Queued<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static> {
    /// Cancels the row when dropped armed.
    guard: Enqueued<S>,
    /// The row.
    id: CommandRunId,
    /// The command line the shell runs.
    command: String,
    /// The directory it runs in.
    dir: PathBuf,
    /// Seconds before it is killed.
    timeout: u64,
}

/// `log` locked; a lock a panicking sink poisoned still holds the output.
fn lock(log: &SharedLog) -> std::sync::MutexGuard<'_, LogBuffer> {
    log.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// D15: the box's stored `command_limits` over the app's. A stored value that does not parse is
/// skipped by [`resolve_command_limits`] and warned about here, as the worker's `command_limits`
/// does: a box whose limits read as nothing must not fall back to the app's in silence.
fn class_limits(
    box_id: BoxId,
    stored: Option<&Value>,
    app: &BTreeMap<String, Value>,
) -> BTreeMap<String, u32> {
    if let Some(stored) = stored
        && let Err(err) = serde_json::from_value::<BTreeMap<String, u32>>(stored.clone())
    {
        tracing::warn!(%box_id, %err, "box.settings.command_limits does not parse; the app setting stands where it does not");
    }
    resolve_command_limits(stored, app)
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
pub(crate) async fn stored_row<S: htui_core::store::WorkerStore>(
    store: &S,
    step: StepId,
    id: CommandRunId,
) -> Result<CommandRun, ToolError> {
    htui_core::store::WorkerStore::command_runs(store, step)
        .await
        .map_err(store_error)?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| ToolError(format!("not found: command_run {id}")))
}

/// MOD-78 D4: ends a row whose lease was lost `cancelled`, keeping what the command printed
/// under the reason it was stopped, and disarms `guard` once the store answered.
async fn finish_lease_lost<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static>(
    store: &S,
    id: CommandRunId,
    claimant: Uuid,
    output: &str,
    guard: &mut Enqueued<S>,
) {
    let ended = htui_core::store::WorkerStore::finish_command(
        store,
        id,
        claimant,
        CommandRunStatus::Cancelled,
        None,
        Some(format!("{LEASE_LOST}{output}")),
    )
    .await;
    // `Ok(false)`: the queue ended the row first. `Err`: the armed guard cancels it.
    if ended.is_ok() {
        guard.disarm();
    }
}

/// D14: asks until `id` is admitted, ticking progress while queued (B-11). Each ask is the
/// queued row's heartbeat. MOD-78 D3(d): at most once per [`COMMAND_HEARTBEAT`] it reads the
/// lease first, the read before the enqueue counting as the first, and a lost one answers
/// `fenced: lease lost`; a failed read is not a stop (D5).
async fn admit<S: htui_core::store::WorkerStore>(
    store: &S,
    id: CommandRunId,
    claimant: Uuid,
    limit: u32,
    lease: Lease,
    progress: Option<&Progress>,
    ticks: &mut u64,
) -> Result<(), ToolError> {
    let mut read_at = tokio::time::Instant::now();
    loop {
        if read_at.elapsed() >= COMMAND_HEARTBEAT {
            read_at = tokio::time::Instant::now();
            if lease.lost_now(store).await {
                return Err(lease.lost());
            }
        }
        match htui_core::store::WorkerStore::claim_command(store, id, claimant, limit).await {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {
                tick(progress, ticks).await;
                tokio::time::sleep(COMMAND_ADMIT_POLL).await;
            }
            Err(err) => return Err(store_error(err)),
        }
    }
}

/// OQ-3: beats the claim every [`COMMAND_HEARTBEAT`], reads the lease (MOD-78 D3(c)), then
/// ticks progress, and returns why it stopped once a beat answers `false` (reaped, cancelled:
/// [`Stopped::Taken`]) or the lease is lost ([`Stopped::LeaseLost`]). A beat or a lease read the
/// store fails is not a stop (MOD-78 D5): a lasting failure lets the row go stale, and the next
/// beat after it answers `false`.
async fn beat<S: htui_core::store::WorkerStore>(
    store: &S,
    id: CommandRunId,
    claimant: Uuid,
    lease: Lease,
    progress: Option<&Progress>,
    ticks: &mut u64,
) -> Stopped {
    loop {
        tokio::time::sleep(COMMAND_HEARTBEAT).await;
        if let Ok(false) = htui_core::store::WorkerStore::beat_command(store, id, claimant).await {
            return Stopped::Taken;
        }
        if lease.lost_now(store).await {
            return Stopped::LeaseLost;
        }
        tick(progress, ticks).await;
    }
}

/// One progress tick, numbered from 1, when the client asked for progress. It waits at most
/// [`TICK_PATIENCE`] for room on the connection, then is dropped: a client that holds the
/// connection open without reading must not stop the beats and asks the row's liveness rests
/// on (a frozen heartbeat is reaped, and its class slot handed on while the child still runs).
pub(crate) async fn tick(progress: Option<&Progress>, ticks: &mut u64) {
    if let Some(progress) = progress {
        *ticks += 1;
        let _ = tokio::time::timeout(TICK_PATIENCE, progress.tick(*ticks)).await;
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use std::time::Duration;

    use chrono::{TimeDelta, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{BoxId, Claim, CommandRunId, CommandRunStatus, NewCommandRun};
    use htui_core::store::mem::MemFault;
    use htui_core::store::traits::COMMAND_HEARTBEAT;
    use htui_core::store::{MemStore, StepFence, WorkerStore};
    use serde_json::json;
    use uuid::Uuid;

    use super::{COMMAND_ADMIT_POLL, Lease, Stopped, class_limits};

    /// Counts the `WARN` events emitted while it is the default subscriber.
    struct Warnings(Arc<AtomicUsize>);

    impl tracing::Subscriber for Warnings {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() == tracing::Level::WARN {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// `class_limits` under a counting subscriber: the limits and how many warnings it logged.
    fn limits_and_warnings(
        stored: Option<&serde_json::Value>,
        app: &BTreeMap<String, serde_json::Value>,
    ) -> (BTreeMap<String, u32>, usize) {
        let warned = Arc::new(AtomicUsize::new(0));
        let limits = tracing::subscriber::with_default(Warnings(Arc::clone(&warned)), || {
            class_limits(BoxId::new(), stored, app)
        });
        (limits, warned.load(Ordering::SeqCst))
    }

    /// D15 (blueprint §11.2): a box value that does not parse is skipped — the app's setting
    /// stands for that class — and the caller warns, as the worker's `command_limits` does; a
    /// value that parses, or none at all, warns nothing.
    #[test]
    fn a_box_limit_that_does_not_parse_is_skipped_and_warned() {
        let app = BTreeMap::from([("command_limits".to_owned(), json!({"build": 2}))]);

        let (limits, warned) = limits_and_warnings(Some(&json!({"build": "many"})), &app);
        assert_eq!(limits.get("build"), Some(&2), "the app's limit stands");
        assert_eq!(warned, 1, "and the unparsable box value is warned about");

        let (limits, warned) = limits_and_warnings(Some(&json!({"build": 3})), &app);
        assert_eq!(limits.get("build"), Some(&3), "a parsed box value overlays");
        assert_eq!(warned, 0, "silently");

        let (_, warned) = limits_and_warnings(None, &app);
        assert_eq!(warned, 0, "no box value warns nothing");
    }

    /// A `Progress` whose connection is open but never read: once the writer blocks on the full
    /// pipe and the outbox fills, every tick would wait for ever.
    async fn stalled_progress() -> (super::Progress, tokio::io::DuplexStream) {
        use crate::protocol::{CallRefused, CallResult, Handler, Progress, ToolInfo, serve};
        use std::future::Future;
        use std::pin::Pin;
        use tokio::io::AsyncWriteExt;

        /// Hands the call's `Progress` out and never answers.
        struct Capture(std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Progress>>>);

        impl Handler for Capture {
            fn tools(&self) -> Vec<ToolInfo> {
                Vec::new()
            }
            fn call(
                &self,
                _: String,
                _: serde_json::Value,
                progress: Option<Progress>,
            ) -> Pin<Box<dyn Future<Output = Result<CallResult, CallRefused>> + Send>> {
                if let (Some(tx), Some(progress)) =
                    (self.0.lock().expect("the capture lock").take(), progress)
                {
                    let _ = tx.send(progress);
                }
                Box::pin(std::future::pending())
            }
        }

        let (tx, rx) = tokio::sync::oneshot::channel();
        let (mut client, server) = tokio::io::duplex(64);
        drop(htui_agent::contained::spawn(serve(
            server,
            Arc::new(Capture(std::sync::Mutex::new(Some(tx)))),
        )));
        let mut call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
            "name": "command_run", "arguments": {}, "_meta": {"progressToken": "stalled"}
        }})
        .to_string()
        .into_bytes();
        call.push(b'\n');
        client.write_all(&call).await.expect("the call is written");
        let progress = rx.await.expect("the call's progress");
        // Fill the outbox behind the writer that the unread pipe already blocks.
        let mut filled = false;
        for n in 0..1_000 {
            if tokio::time::timeout(Duration::from_secs(1), progress.tick(n))
                .await
                .is_err()
            {
                filled = true;
                break;
            }
        }
        assert!(filled, "the outbox never filled");
        (progress, client)
    }

    /// A queued `build` row on the demo's `R2/prd` step and box.
    fn queued() -> NewCommandRun {
        NewCommandRun {
            id: CommandRunId::new(),
            run_step_id: ids::STEP_R2_PRD,
            box_id: ids::BOX,
            class: "build".to_owned(),
            command: "make".to_owned(),
            cwd: "/srv".to_owned(),
            status: CommandRunStatus::Queued,
            exit_code: None,
            output: None,
            queued_at: Utc::now(),
            started_at: None,
            finished_at: None,
        }
    }

    /// The demo's `RUN_2`, on which no lease is held: what every scope before MOD-78 ran on.
    fn unleased() -> Lease {
        Lease {
            run: ids::RUN_2,
            fence: StepFence::Unleased,
            step: ids::STEP_R2_PRD,
        }
    }

    /// `RUN_2` claimed by `owner`, and the lease a session of its walk is fenced on.
    async fn leased(store: &MemStore, owner: Uuid) -> Lease {
        assert_eq!(
            store
                .claim_run(
                    ids::RUN_2,
                    ids::BOX,
                    owner,
                    Utc::now(),
                    TimeDelta::minutes(5)
                )
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "the walk claims the run"
        );
        Lease {
            run: ids::RUN_2,
            fence: StepFence::Lease(owner),
            step: ids::STEP_R2_PRD,
        }
    }

    /// Another process takes `RUN_2`'s lease from `owner`.
    async fn take_away(store: &MemStore, owner: Uuid) {
        assert!(
            store
                .release_lease(ids::RUN_2, owner)
                .await
                .expect("release")
        );
        assert!(
            store
                .take_lease(ids::RUN_2, ids::BOX, Uuid::now_v7(), TimeDelta::minutes(5))
                .await
                .expect("take"),
            "a stranger holds the lease now"
        );
    }

    /// A `running` row on the session's step, claimed by the returned claimant.
    async fn running(store: &MemStore) -> (CommandRunId, Uuid) {
        let row = store.enqueue_command(queued()).await.expect("enqueue");
        let claimant = Uuid::now_v7();
        assert!(
            store
                .claim_command(row.id, claimant, 1)
                .await
                .expect("claim")
                .is_some()
        );
        (row.id, claimant)
    }

    /// MOD-78 D3(d): a queued call whose walk lost its lease leaves the queue within two
    /// heartbeats, with the answer every fenced tool gives; the reads in the queue are
    /// rate-limited to one per heartbeat, so it does not stop before the first heartbeat.
    #[tokio::test(start_paused = true)]
    async fn a_waiter_on_a_lost_lease_stops_within_two_heartbeats() {
        let store = MemStore::demo();
        let owner = Uuid::now_v7();
        let lease = leased(&store, owner).await;
        let _ahead = running(&store).await;
        let waiter = store.enqueue_command(queued()).await.expect("enqueue");
        take_away(&store, owner).await;

        let mut ticks = 0;
        let admit = super::admit(
            &store,
            waiter.id,
            Uuid::now_v7(),
            1,
            lease,
            None,
            &mut ticks,
        );
        tokio::pin!(admit);
        assert!(
            tokio::time::timeout(COMMAND_HEARTBEAT - Duration::from_millis(1), &mut admit)
                .await
                .is_err(),
            "no lease read in the queue before a heartbeat has passed"
        );
        let refused = tokio::time::timeout(COMMAND_HEARTBEAT * 2, admit)
            .await
            .expect("the waiter stops within two heartbeats")
            .expect_err("a lost lease is not admitted");
        assert_eq!(refused.0, "fenced: lease lost");
    }

    /// MOD-78 D3(c): the heartbeat of a running command reads the lease, and a lost one stops it.
    #[tokio::test(start_paused = true)]
    async fn a_beat_on_a_lost_lease_answers_lease_lost() {
        let store = MemStore::demo();
        let owner = Uuid::now_v7();
        let lease = leased(&store, owner).await;
        let (id, claimant) = running(&store).await;
        take_away(&store, owner).await;

        let mut ticks = 0;
        let stopped = tokio::time::timeout(
            COMMAND_HEARTBEAT * 2,
            super::beat(&store, id, claimant, lease, None, &mut ticks),
        )
        .await
        .expect("the next beat reads the lost lease");
        assert_eq!(stopped, Stopped::LeaseLost);
    }

    /// MOD-78 D5: a lease read the store fails is not a stop, even on a lease that is in fact
    /// lost; the first read that answers stops the beat.
    #[tokio::test(start_paused = true)]
    async fn a_failed_lease_read_does_not_stop_the_beat() {
        let store = MemStore::demo();
        let owner = Uuid::now_v7();
        let lease = leased(&store, owner).await;
        let (id, claimant) = running(&store).await;
        take_away(&store, owner).await;
        store.set_fault(MemFault::LeaseHolds, true);

        let mut ticks = 0;
        let beat = super::beat(&store, id, claimant, lease, None, &mut ticks);
        tokio::pin!(beat);
        assert!(
            tokio::time::timeout(COMMAND_HEARTBEAT * 5, &mut beat)
                .await
                .is_err(),
            "a failed read does not stop the command"
        );
        store.set_fault(MemFault::LeaseHolds, false);
        let stopped = tokio::time::timeout(COMMAND_HEARTBEAT * 2, beat)
            .await
            .expect("the first read that answers stops it");
        assert_eq!(stopped, Stopped::LeaseLost);
    }

    /// ADV-1: a client that holds the connection open without reading does not stop the beats.
    /// The row was cancelled under the claim, so the first beat that is reached answers `false`
    /// and `beat` returns; a tick waiting on the stalled client would never let it get there.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_client_does_not_stop_the_heartbeat() {
        let (progress, _client) = stalled_progress().await;
        let store = MemStore::demo();
        let row = store.enqueue_command(queued()).await.expect("enqueue");
        let claimant = Uuid::now_v7();
        assert!(
            store
                .claim_command(row.id, claimant, 1)
                .await
                .expect("claim")
                .is_some()
        );
        assert!(store.cancel_command(row.id).await.expect("cancel"));
        let mut ticks = 0;
        let stopped = tokio::time::timeout(
            COMMAND_HEARTBEAT * 6,
            super::beat(
                &store,
                row.id,
                claimant,
                unleased(),
                Some(&progress),
                &mut ticks,
            ),
        )
        .await
        .expect("the beat is reached and answers false");
        assert_eq!(stopped, Stopped::Taken, "the queue took the row");
    }

    /// ADV-1: a client that holds the connection open without reading does not stop a queued
    /// call's asks (its only heartbeat): it is admitted once the slot ahead of it frees.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_client_does_not_stop_the_admission_asks() {
        let (progress, _client) = stalled_progress().await;
        let store = MemStore::demo();
        let ahead = store.enqueue_command(queued()).await.expect("enqueue");
        let holder = Uuid::now_v7();
        assert!(
            store
                .claim_command(ahead.id, holder, 1)
                .await
                .expect("claim")
                .is_some()
        );
        let waiter = store.enqueue_command(queued()).await.expect("enqueue");
        let mut ticks = 0;
        let admitted = super::admit(
            &store,
            waiter.id,
            Uuid::now_v7(),
            1,
            unleased(),
            Some(&progress),
            &mut ticks,
        );
        let frees = async {
            tokio::time::sleep(COMMAND_ADMIT_POLL * 5).await;
            assert!(
                store
                    .finish_command(ahead.id, holder, CommandRunStatus::Done, Some(0), None)
                    .await
                    .expect("finish")
            );
        };
        let (admitted, ()) = tokio::time::timeout(COMMAND_ADMIT_POLL * 60, async {
            tokio::join!(admitted, frees)
        })
        .await
        .expect("the waiter keeps asking and is admitted");
        admitted.expect("admitted");
    }
}
