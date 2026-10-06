//! `command_run` (MOD-11 M4, plan D14-D16, OQ-3, OQ-7, blueprint §11.2): a build, test or run
//! command through the box's queue.
//!
//! The call is validated (class, `cwd`, timeout), its command line scrubbed (I-5), and enqueued
//! as a `queued` `command_run` row of the session's step on the session's box. It then asks for
//! admission every [`COMMAND_ADMIT_POLL`] under the class limit D15 resolves per call, ticking
//! progress (B-11; a tick never waits long on a client that does not read); runs through
//! [`run_shell`] in the session's directory, beating the claim every [`COMMAND_HEARTBEAT`] (a
//! beat that answers `false` kills the child: the row was reaped or cancelled); scrubs the tail,
//! fail closed; finishes the row; and answers it. Everything a cancelled call must undo lives in
//! [`Enqueued`]'s drop (H-18): the row is cancelled and the child's process group dies with the
//! dropped future.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use htui_core::model::kind::{command_limit, resolve_command_limits};
use htui_core::model::{BoxId, CommandRun, CommandRunId, CommandRunStatus, NewCommandRun};
use htui_core::scrub::Scrubber;
use htui_core::store::traits::COMMAND_HEARTBEAT;
use htui_orch::verify::{ShellEnd, ShellRun, run_shell};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, scrubbed, store_error};
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
    // MOD-11 R1 L3: one call per session past this point, held until the call ends or is dropped.
    let _slot = std::sync::Arc::clone(&session.command_slot)
        .try_acquire_owned()
        .map_err(|_| ToolError(BUSY.to_owned()))?;

    // D15, read per call through the session's host: the box's own limits over the app's.
    let box_limits = ctx
        .host
        .box_row(scope.box_id)
        .await
        .map_err(store_error)?
        .and_then(|row| row.settings.get("command_limits").cloned());
    let app = ctx.host.app_settings().await.map_err(store_error)?;
    let limit = command_limit(
        &class_limits(scope.box_id, box_limits.as_ref(), &app),
        &class,
    );

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

    admit(&store, id, claimant, limit, progress.as_ref(), &mut ticks).await?;

    // OQ-3: the claim beats until the queue takes the row; that is `run_shell`'s `stop`.
    let stopped = AtomicBool::new(false);
    let beats = async {
        beat(&store, id, claimant, progress.as_ref(), &mut ticks).await;
        stopped.store(true, Ordering::SeqCst);
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

/// D14: asks until `id` is admitted, ticking progress while queued (B-11). Each ask is the
/// queued row's heartbeat.
async fn admit<S: htui_core::store::WorkerStore>(
    store: &S,
    id: CommandRunId,
    claimant: Uuid,
    limit: u32,
    progress: Option<&Progress>,
    ticks: &mut u64,
) -> Result<(), ToolError> {
    loop {
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

/// OQ-3: beats the claim every [`COMMAND_HEARTBEAT`], then ticks progress, and returns once a
/// beat answers `false` (reaped, cancelled). A beat the store fails is not a stop: a lasting
/// failure lets the row go stale, and the next beat after it answers `false`.
async fn beat<S: htui_core::store::WorkerStore>(
    store: &S,
    id: CommandRunId,
    claimant: Uuid,
    progress: Option<&Progress>,
    ticks: &mut u64,
) {
    loop {
        tokio::time::sleep(COMMAND_HEARTBEAT).await;
        if let Ok(false) = htui_core::store::WorkerStore::beat_command(store, id, claimant).await {
            return;
        }
        tick(progress, ticks).await;
    }
}

/// One progress tick, numbered from 1, when the client asked for progress. It waits at most
/// [`TICK_PATIENCE`] for room on the connection, then is dropped: a client that holds the
/// connection open without reading must not stop the beats and asks the row's liveness rests
/// on (a frozen heartbeat is reaped, and its class slot handed on while the child still runs).
async fn tick(progress: Option<&Progress>, ticks: &mut u64) {
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
