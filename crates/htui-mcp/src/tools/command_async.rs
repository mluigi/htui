//! `command_start`, `command_wait` and `command_cancel` (MOD-152 D7-D9): a command that outlives
//! the call that started it.
//!
//! `command_start` checks its arguments as `command_run` does ([`check`]), reserves a place in the
//! session's [`LiveCommands`] (at most [`START_LIMIT`] queued or running), reads the lease and
//! queues the row, then hands it to a background task that runs it through the same
//! [`Runner::run`] — same fence, scrub, class limit and log (D1-D4) — and answers the row's id at
//! once. The task watches the entry's own token, which is a child of the session's: `command_cancel`
//! fires the one, the end of the session (lease dropped, host closed) the other, and either way the
//! child dies and the row ends `cancelled` with a line that says which. A lost lease ends it as
//! `command_run` does.
//!
//! `command_wait` answers when the row is terminal, a line matching `pattern` appears after
//! `after_line`, or the deadline (clamped to [`MAX_WAIT`]) passes. A live row of the session is read
//! from the registry, every other row of the run from the store (D5). The log is scrubbed as one
//! text, fail closed, before its tail, its matched line or a pattern sees it (D3, MOD-152 R1 F1);
//! a running row's scrubbed log is kept until it grows (R1 F5). Line numbers are those of the
//! command's output (R1 F2).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{CommandRunId, CommandRunStatus, RunId};
use htui_core::scrub::Scrubber;
use regex::Regex;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::command::{Checked, CommandArgs, Runner, Watch, check, class_limit, tick};
use super::command_log::{
    LogBuffer, Mode, ScrubbedLog, SharedLog, compile, cut_head, scrub, slice, withheld,
};
use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, store_error};
use crate::host::Session;
use crate::protocol::Progress;

/// D7: the most started commands one session may have queued or running.
pub(crate) const START_LIMIT: usize = 4;

/// MOD-152 R1 F4: the most ended entries a session's registry keeps; past it the oldest ended
/// go, and `command_cancel` answers them from the store.
pub(crate) const ENDED_KEEP: usize = 64;

/// D8: the longest a `command_wait` waits, whatever its deadline.
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(300);

/// The line a row `command_cancel` stopped opens with (D7).
const CANCELLED: &str = "[stopped: cancelled by command_cancel]\n";

/// The line a row the end of its session stopped opens with (D7).
const SESSION_ENDED: &str = "[stopped: the session ended]\n";

/// Lines and bytes a wait's `tail` answers at most.
const TAIL_LINES: usize = 20;
const TAIL_BYTES: usize = 4 * 1024;

/// The most of a matched line a wait answers, in bytes.
const LINE_BYTES: usize = 4 * 1024;

/// How long a wait lets output gather after a line was written before it looks again.
const RECHECK: Duration = Duration::from_millis(200);

/// How often a wait on a row the session does not run reads the store.
const POLL: Duration = Duration::from_secs(1);

/// How often a wait ticks progress (B-11).
const TICK_EVERY: Duration = Duration::from_secs(5);

/// How long `command_cancel` waits for the row to settle before it answers as the row stands.
const SETTLE: Duration = Duration::from_secs(30);

/// Where a started command stands, as its task last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Queued, waiting for admission.
    Queued,
    /// Admitted: the child runs.
    Running,
    /// The row ended; the store holds its log (D4).
    Ended {
        /// The row's status.
        status: CommandRunStatus,
        /// The exit code, when the command exited.
        exit_code: Option<i32>,
    },
}

impl Phase {
    /// The row's status as the tools answer it.
    pub(crate) const fn status(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Ended { status, .. } => status.as_str(),
        }
    }

    /// Whether the row ended.
    pub(crate) const fn ended(self) -> bool {
        matches!(self, Self::Ended { .. })
    }
}

/// `mutex` locked; a poisoned lock still holds its value.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One started command: its live log, a wake-up for whoever waits on it, its phase and its stop.
pub(crate) struct Live {
    /// The merged output so far; emptied once the row ended and the store holds it.
    log: SharedLog,
    /// The log as last scrubbed, with the [`LogBuffer::total`] it was read at (R1 F5).
    view: Mutex<Option<(u64, Arc<ScrubbedLog>)>>,
    /// When the row ended: the registry lets the oldest ended go first (R1 F4).
    ended_at: OnceLock<Instant>,
    /// Notified when a line was written or the phase moved.
    changed: Notify,
    /// The phase.
    state: watch::Sender<Phase>,
    /// Stops the command: fired by `command_cancel`, and by the session's own token, its parent.
    cancel: CancellationToken,
}

impl Live {
    fn new(cancel: CancellationToken) -> Self {
        Self {
            log: SharedLog::default(),
            view: Mutex::new(None),
            ended_at: OnceLock::new(),
            changed: Notify::new(),
            state: watch::Sender::new(Phase::Queued),
            cancel,
        }
    }

    /// The phase.
    pub(crate) fn phase(&self) -> Phase {
        *self.state.borrow()
    }

    /// The phase and, while the row has not ended, its complete lines scrubbed by `scrubber` as
    /// one text (D3, R1 F1) — read together, so an ended row is never read as an empty running
    /// one. The lines are copied under the lock and scrubbed outside it, once per size of the
    /// log (R1 F5).
    pub(crate) fn snapshot(&self, scrubber: &dyn Scrubber) -> (Phase, Option<Arc<ScrubbedLog>>) {
        let (phase, total, complete) = {
            let log = lock(&self.log);
            let phase = self.phase();
            if phase.ended() {
                return (phase, None);
            }
            let total = log.total();
            if let Some((read_at, view)) = lock(&self.view).as_ref()
                && *read_at == total
            {
                return (phase, Some(Arc::clone(view)));
            }
            (phase, total, log.complete())
        };
        let view = Arc::new(complete.scrubbed(scrubber));
        // Under the log's lock, so a row that ended meanwhile keeps nothing.
        let _log = lock(&self.log);
        if !self.phase().ended() {
            let mut cached = lock(&self.view);
            if cached.as_ref().is_none_or(|(read_at, _)| *read_at < total) {
                *cached = Some((total, Arc::clone(&view)));
            }
        }
        (phase, Some(view))
    }

    /// The row ended as `phase`: the log is in the store now, so the memory is let go.
    fn end(&self, phase: Phase) {
        {
            let mut log = lock(&self.log);
            // Set before the phase, so an ended entry always says when it ended.
            let _ = self.ended_at.set(Instant::now());
            self.state.send_replace(phase);
            *log = LogBuffer::default();
            *lock(&self.view) = None;
        }
        self.changed.notify_waiters();
    }
}

impl Watch for Live {
    fn log(&self) -> &SharedLog {
        &self.log
    }

    fn wrote(&self) {
        self.changed.notify_waiters();
    }

    fn running(&self) {
        self.state.send_replace(Phase::Running);
        self.changed.notify_waiters();
    }
}

/// What [`LiveCommands`] holds.
#[derive(Default)]
struct Registry {
    /// The entries, by row.
    entries: HashMap<CommandRunId, Arc<Live>>,
    /// Every row this session started, its entry kept or let go (R1 F4).
    started: HashSet<CommandRunId>,
}

impl Registry {
    /// R1 F4: lets the oldest ended entries go, past [`ENDED_KEEP`].
    fn prune(&mut self) {
        let mut ended: Vec<(Instant, CommandRunId)> = self
            .entries
            .iter()
            .filter_map(|(id, live)| live.ended_at.get().map(|at| (*at, *id)))
            .collect();
        let Some(excess) = ended.len().checked_sub(ENDED_KEEP).filter(|n| *n > 0) else {
            return;
        };
        ended.sort_unstable_by_key(|(at, _)| *at);
        for (_, id) in &ended[..excess] {
            self.entries.remove(id);
        }
    }
}

/// D7: a session's started commands, by row. An ended entry stays (its log let go), so
/// `command_cancel` answers how it ended; it no longer counts toward [`START_LIMIT`]. At most
/// [`ENDED_KEEP`] ended entries stay; the session still knows every row it started (R1 F4).
#[derive(Default, Clone)]
pub(crate) struct LiveCommands(Arc<Mutex<Registry>>);

impl LiveCommands {
    /// A new entry `id` stopped by `cancel`, or the [`START_LIMIT`] refusal.
    fn reserve(&self, id: CommandRunId, cancel: CancellationToken) -> Result<Arc<Live>, ToolError> {
        let mut registry = lock(&self.0);
        registry.prune();
        let busy = registry
            .entries
            .values()
            .filter(|live| !live.phase().ended())
            .count();
        if busy >= START_LIMIT {
            return Err(ToolError(format!(
                "refused: {START_LIMIT} commands are already queued or running in this session"
            )));
        }
        let live = Arc::new(Live::new(cancel));
        registry.entries.insert(id, Arc::clone(&live));
        registry.started.insert(id);
        Ok(live)
    }

    /// Drops the entry `id` (its row was never queued).
    fn forget(&self, id: CommandRunId) {
        let mut registry = lock(&self.0);
        registry.entries.remove(&id);
        registry.started.remove(&id);
    }

    /// R1 F4: lets the oldest ended entries go, past [`ENDED_KEEP`].
    fn prune(&self) {
        lock(&self.0).prune();
    }

    /// The entry `id`, if this session started it and still keeps it.
    pub(crate) fn get(&self, id: CommandRunId) -> Option<Arc<Live>> {
        lock(&self.0).entries.get(&id).cloned()
    }

    /// Whether this session started the row `id`.
    fn started(&self, id: CommandRunId) -> bool {
        lock(&self.0).started.contains(&id)
    }
}

/// A reserved entry whose row is not yet queued; dropped armed, it is forgotten.
struct Reserved<'a> {
    commands: &'a LiveCommands,
    id: CommandRunId,
    armed: bool,
}

impl Drop for Reserved<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.commands.forget(self.id);
        }
    }
}

/// Advertised with `command_run` (D10).
pub(crate) const START: ToolDef = ToolDef {
    name: "command_start",
    description: "Starts a build, test or run command through this box's command queue and returns its id at once; follow it with command_wait.",
    schema: schema_of::<CommandArgs>,
    advertised: |scope, _| scope.command_queue,
};

// A plain comment, not a doc: schemars would hand a doc to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    /// The command, as command_start answered it.
    command_run_id: String,
    /// Answer by this RFC 3339 time at the latest; at most 300 seconds from now.
    deadline: String,
    /// Answer as soon as a line matches this regular expression.
    pattern: Option<String>,
    /// Match only lines after this one, counted from 1; default 0. Pass the last answer's
    /// next_line to go on.
    after_line: Option<u32>,
}

/// Advertised with `command_run` (D10); pre-approved (D9: it runs nothing).
pub(crate) const WAIT: ToolDef = ToolDef {
    name: "command_wait",
    description: "Waits until a command ends, prints a line matching a pattern, or the deadline passes, and returns where it stands.",
    schema: schema_of::<WaitArgs>,
    advertised: |scope, _| scope.command_queue,
};

// A plain comment, not a doc: schemars would hand a doc to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CancelArgs {
    /// The command, as command_start answered it.
    command_run_id: String,
}

/// Advertised with `command_run` (D10).
pub(crate) const CANCEL: ToolDef = ToolDef {
    name: "command_cancel",
    description: "Stops a command this session started and returns how it ended.",
    schema: schema_of::<CancelArgs>,
    advertised: |scope, _| scope.command_queue,
};

/// `{"command_run_id": "<uuid>", "status": "queued"}`.
pub(crate) async fn start_call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let session = ctx.session;
    let checked = check(args(arguments)?, &session.scope, session.scrubber.as_ref())?;
    let limit = class_limit(&ctx.host, &session.scope, checked.class()).await?;
    let id = start(session, checked, limit).await?;
    Ok(json!({"command_run_id": id, "status": "queued"}))
}

/// D7: reserves an entry, queues `checked` under class `limit` and spawns the task that runs it;
/// the row's id.
pub(crate) async fn start<S: htui_core::store::WorkerStore + Clone + Send + Sync + 'static>(
    session: &Session<S>,
    checked: Checked,
    limit: u32,
) -> Result<CommandRunId, ToolError> {
    let id = CommandRunId::new();
    let live = session.commands.reserve(id, session.cancel.child_token())?;
    let mut reserved = Reserved {
        commands: &session.commands,
        id,
        armed: true,
    };
    let runner = Runner::of(session, limit);
    let queued = runner.enqueue(id, checked).await?;
    reserved.armed = false;
    let session_end = session.cancel.clone();
    let registry = session.commands.clone();
    drop(htui_agent::contained::spawn(async move {
        let halt = async {
            live.cancel.cancelled().await;
            if session_end.is_cancelled() {
                SESSION_ENDED
            } else {
                CANCELLED
            }
        };
        let phase = match runner.run(queued, None, halt, &*live).await {
            Ok(finished) => Phase::Ended {
                status: finished.status,
                exit_code: finished.exit_code,
            },
            Err(err) => {
                tracing::warn!(%id, reason = %err.0, "a started command ended early");
                let (status, exit_code) = runner.settled(id).await;
                Phase::Ended { status, exit_code }
            }
        };
        live.end(phase);
        registry.prune();
    }));
    Ok(id)
}

/// `text` as a row id, or the one-line refusal.
fn row_id(text: &str) -> Result<CommandRunId, ToolError> {
    text.parse()
        .map_err(|_| ToolError(format!("refused: command_run_id `{text}` is not an id")))
}

/// What a call about a row it cannot see answers.
fn not_found(id: CommandRunId) -> ToolError {
    ToolError(format!("not found: command_run {id}"))
}

/// `{"command_run_id", "status", "exit_code"}`, once the row settled.
pub(crate) async fn cancel_call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let CancelArgs { command_run_id } = args(arguments)?;
    cancel(ctx.session, row_id(&command_run_id)?).await
}

/// D7: stops the session's command `id` and answers how the row settled; a row the session did
/// not start is not found.
pub(crate) async fn cancel<S: htui_core::store::WorkerStore>(
    session: &Session<S>,
    id: CommandRunId,
) -> ToolResult {
    let Some(live) = session.commands.get(id) else {
        // R1 F4: a row this session started whose entry was let go has ended: the store says how.
        if !session.commands.started(id) {
            return Err(not_found(id));
        }
        let row = htui_core::store::WorkerStore::command_log_of_run(
            &session.store,
            session.scope.run_id,
            Some(id),
        )
        .await
        .map_err(store_error)?
        .ok_or_else(|| not_found(id))?
        .row;
        return Ok(json!({
            "command_run_id": id,
            "status": row.status.as_str(),
            "exit_code": row.exit_code,
        }));
    };
    live.cancel.cancel();
    let mut state = live.state.subscribe();
    let settled = tokio::time::timeout(SETTLE, state.wait_for(|phase| phase.ended()))
        .await
        .is_ok();
    let phase = live.phase();
    if !settled {
        tracing::warn!(%id, "a cancelled command had not settled in time");
    }
    // The store's row is the settled status; the task's word stands when the store cannot answer.
    let row = htui_core::store::WorkerStore::command_log_of_run(
        &session.store,
        session.scope.run_id,
        Some(id),
    )
    .await
    .ok()
    .flatten()
    .map(|stored| stored.row);
    Ok(match row {
        Some(row) => json!({
            "command_run_id": id,
            "status": row.status.as_str(),
            "exit_code": row.exit_code,
        }),
        None => json!({
            "command_run_id": id,
            "status": phase.status(),
            "exit_code": match phase {
                Phase::Ended { exit_code, .. } => exit_code,
                _ => None,
            },
        }),
    })
}

/// D8: `{"status", "exit_code", "matched", "line_no", "line", "tail", "next_line", "total_lines",
/// "deadline_reached"}`.
pub(crate) async fn wait_call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let read: WaitArgs = args(arguments)?;
    let session = ctx.session;
    let id = row_id(&read.command_run_id)?;
    let deadline = DateTime::parse_from_rfc3339(&read.deadline).map_err(|_| {
        ToolError(format!(
            "refused: deadline `{}` is not an RFC 3339 time",
            read.deadline
        ))
    })?;
    let pattern = read.pattern.as_deref().map(compile).transpose()?;
    let after = usize::try_from(read.after_line.unwrap_or(0)).unwrap_or(usize::MAX);
    let budget = budget(deadline.with_timezone(&Utc), session.clock.now());
    let source = Source {
        live: session.commands.get(id),
        store: &session.store,
        run: session.scope.run_id,
        id,
    };
    wait(
        &source,
        pattern.as_ref(),
        after,
        budget,
        session.scrubber.as_ref(),
        ctx.progress.as_ref(),
    )
    .await
}

/// D8: how long a wait for `deadline` may last from `now`: nothing for a deadline passed, at
/// most [`MAX_WAIT`].
pub(crate) fn budget(deadline: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (deadline - now)
        .to_std()
        .unwrap_or(Duration::ZERO)
        .min(MAX_WAIT)
}

/// Where a wait reads a row: the session's entry while it runs, the store otherwise (D5: a row of
/// the caller's run).
pub(crate) struct Source<'a, S> {
    /// The session's entry for the row, if it started it.
    live: Option<Arc<Live>>,
    /// The session's store.
    store: &'a S,
    /// The caller's run.
    run: RunId,
    /// The row.
    id: CommandRunId,
}

/// A row as a wait reads it.
struct View {
    /// The status as answered.
    status: &'static str,
    /// The exit code, once the row ended.
    exit_code: Option<i32>,
    /// Whether the row ended.
    terminal: bool,
    /// The log so far, scrubbed as one text: complete lines of a live row, the stored text of
    /// another.
    log: Arc<ScrubbedLog>,
}

impl<S: htui_core::store::WorkerStore> Source<'_, S> {
    /// The row now, its log scrubbed by `scrubber`.
    async fn view(&self, scrubber: &dyn Scrubber) -> Result<View, ToolError> {
        if let Some(live) = &self.live
            && let (phase, Some(log)) = live.snapshot(scrubber)
        {
            return Ok(View {
                status: phase.status(),
                exit_code: None,
                terminal: false,
                log,
            });
        }
        let stored =
            htui_core::store::WorkerStore::command_log_of_run(self.store, self.run, Some(self.id))
                .await
                .map_err(store_error)?
                .ok_or_else(|| not_found(self.id))?;
        let status = stored.row.status;
        Ok(View {
            status: status.as_str(),
            exit_code: stored.row.exit_code,
            terminal: !matches!(status, CommandRunStatus::Queued | CommandRunStatus::Running),
            log: Arc::new(match stored.log {
                Some(log) => ScrubbedLog::of(log.text, log.truncated, scrubber),
                None => ScrubbedLog::of(String::new(), false, scrubber),
            }),
        })
    }
}

/// D8: reads the row until it ended, a line after `after` matches `pattern`, or `budget` passed;
/// ticks progress every [`TICK_EVERY`] meanwhile.
pub(crate) async fn wait<S: htui_core::store::WorkerStore>(
    source: &Source<'_, S>,
    pattern: Option<&Regex>,
    after: usize,
    budget: Duration,
    scrubber: &dyn Scrubber,
    progress: Option<&Progress>,
) -> ToolResult {
    let until = Instant::now() + budget;
    let mut ticks = 0_u64;
    let mut ticked = Instant::now();
    loop {
        // Registered before the read, so a line written after it wakes this wait.
        let changed = source
            .live
            .as_ref()
            .filter(|live| !live.phase().ended())
            .map(|live| live.changed.notified());
        let view = source.view(scrubber).await?;
        let seen = seen(&view.log, pattern, after, scrubber);
        let now = Instant::now();
        let reached = now >= until;
        if seen.matched || view.terminal || reached {
            return Ok(json!({
                "status": view.status,
                "exit_code": view.exit_code,
                "matched": seen.matched,
                "line_no": seen.line_no,
                "line": seen.line,
                "tail": seen.tail,
                "next_line": seen.next_line,
                "total_lines": seen.total_lines,
                "deadline_reached": reached && !seen.matched && !view.terminal,
            }));
        }
        if now.duration_since(ticked) >= TICK_EVERY {
            ticked = now;
            tick(progress, &mut ticks).await;
        }
        let next_tick = ticked + TICK_EVERY;
        match changed {
            Some(changed) => {
                tokio::select! {
                    () = changed => {
                        // Let the output gather before reading it again.
                        tokio::time::sleep_until((Instant::now() + RECHECK).min(until)).await;
                    }
                    () = tokio::time::sleep_until(until.min(next_tick)) => {}
                }
            }
            None => tokio::time::sleep_until((now + POLL).min(until).min(next_tick)).await,
        }
    }
}

/// What a wait saw in a log.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    /// A line after `after` matched.
    pub(crate) matched: bool,
    /// The matched line's number in the command's output, counted from 1.
    pub(crate) line_no: Option<usize>,
    /// The matched line, scrubbed, cut to [`LINE_BYTES`].
    pub(crate) line: Option<String>,
    /// The last lines, scrubbed, fail closed.
    pub(crate) tail: String,
    /// The `after_line` that goes on after this answer: the match, or the last line read.
    pub(crate) next_line: usize,
    /// The number of the log's last line.
    pub(crate) total_lines: usize,
}

/// D3, D8: the first line after `after` that `pattern` matches in `log`, and its tail. The log
/// was scrubbed as one text before it is cut (MOD-152 R1 F1), and what is cut from it is scrubbed
/// again on its way out, fail closed; a log that could not be masked matches nothing.
pub(crate) fn seen(
    log: &ScrubbedLog,
    pattern: Option<&Regex>,
    after: usize,
    scrubber: &dyn Scrubber,
) -> Seen {
    let lines: Vec<&str> = log.text.lines().collect();
    let total_lines = log.offset + lines.len();
    let tail = slice(&log.text, &Mode::Tail(TAIL_LINES), TAIL_BYTES, log.offset).text;
    let tail = scrub(scrubber, tail).unwrap_or_else(|rule| withheld(&rule));
    let found = pattern.filter(|_| !log.withheld).and_then(|pattern| {
        lines
            .iter()
            .enumerate()
            .skip(after.saturating_sub(log.offset))
            .find(|(_, line)| pattern.is_match(line))
            .map(|(index, line)| {
                let line = cut_head(line, LINE_BYTES).to_owned();
                let line = scrub(scrubber, line).unwrap_or_else(|rule| withheld(&rule));
                (log.offset + index + 1, line)
            })
    });
    match found {
        Some((line_no, line)) => Seen {
            matched: true,
            line_no: Some(line_no),
            line: Some(line),
            tail,
            next_line: line_no,
            total_lines,
        },
        None => Seen {
            matched: false,
            line_no: None,
            line: None,
            tail,
            next_line: total_lines.max(after),
            total_lines,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::{TimeDelta, Utc};
    use htui_core::clock::SystemClock;
    use htui_core::fixtures::ids;
    use htui_core::model::{CommandRunId, CommandRunStatus, NewCommandRun, Transport};
    use htui_core::prompt::render::HostnameLine;
    use htui_core::scrub::{MinimalScrubber, Scrubber, Unmasked};
    use htui_core::store::{MemStore, StepFence, WorkerStore};
    use htui_orch::tools::ToolScope;
    use serde_json::{Value, json};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use super::{
        ENDED_KEEP, LiveCommands, MAX_WAIT, Phase, START_LIMIT, Source, budget, cancel, seen,
        start, wait,
    };
    use crate::host::Session;
    use crate::tools::HostCaps;
    use crate::tools::command::{CommandArgs, check};
    use crate::tools::command_log::{ScrubbedLog, compile, truncated_line};

    fn scrubber() -> MinimalScrubber {
        MinimalScrubber::new(["secret-4f1e".to_owned()])
    }

    /// `text` as a reader serves it: scrubbed whole.
    fn log(text: &str) -> ScrubbedLog {
        ScrubbedLog::of(text.to_owned(), false, &scrubber())
    }

    /// A session on the demo's `R2/prd` step and box, never connected to a client.
    fn session(store: &MemStore) -> Session<MemStore> {
        Session {
            scope: ToolScope {
                run_id: ids::RUN_2,
                step_id: ids::STEP_R2_PRD,
                project_id: ids::PROJECT_HTUI,
                item_id: None,
                box_id: ids::BOX,
                user: ids::USER,
                fence: StepFence::Unleased,
                output_kind: None,
                hostname: HostnameLine::Omitted,
                command_queue: true,
                cwd: std::env::temp_dir(),
                extra_dirs: Vec::<PathBuf>::new(),
                transport: Transport::Acp,
            },
            store: store.clone(),
            ask: None,
            cancel: CancellationToken::new(),
            search: None,
            scrubber: Arc::new(scrubber()),
            clock: Arc::new(SystemClock),
            caps: HostCaps { search: false },
            command_slot: Arc::new(tokio::sync::Semaphore::new(1)),
            commands: LiveCommands::default(),
        }
    }

    /// Another session's `build` row holding the box's one `build` slot: what the session's row
    /// queues behind, so no child ever runs.
    async fn hold_the_slot(store: &MemStore) {
        let held = store
            .enqueue_command(NewCommandRun {
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
            })
            .await
            .expect("enqueue");
        assert!(
            store
                .claim_command(held.id, Uuid::now_v7(), 1)
                .await
                .expect("claim")
                .is_some()
        );
    }

    /// A checked `build` of `command`.
    fn checked(session: &Session<MemStore>, command: &str) -> super::Checked {
        check(
            CommandArgs {
                class: "build".to_owned(),
                command: command.to_owned(),
                cwd: None,
                timeout_secs: None,
                repo: None,
            },
            &session.scope,
            session.scrubber.as_ref(),
        )
        .expect("checked")
    }

    /// Starts `command` queued behind the held slot.
    async fn start_queued(session: &Session<MemStore>, command: &str) -> CommandRunId {
        start(session, checked(session, command), 1)
            .await
            .expect("started")
    }

    /// Polls (in paused time) until the row `id` is `status`.
    async fn until_row(store: &MemStore, id: CommandRunId, status: CommandRunStatus) -> String {
        for _ in 0..600 {
            if let Some(row) = store
                .command_runs(ids::STEP_R2_PRD)
                .await
                .expect("rows")
                .into_iter()
                .find(|row| row.id == id && row.status == status)
            {
                let log = store
                    .command_log_of_run(ids::RUN_2, Some(id))
                    .await
                    .expect("read")
                    .and_then(|stored| stored.log)
                    .map(|log| log.text)
                    .unwrap_or_default();
                assert_eq!(row.output, None, "a row that never ran keeps no output");
                return log;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("row {id} never became {status}");
    }

    /// D7: a session holds at most `START_LIMIT` queued or running entries; an ended one no
    /// longer counts.
    #[test]
    fn the_registry_refuses_past_the_limit_and_frees_on_end() {
        let commands = LiveCommands::default();
        let mut entries = Vec::new();
        for _ in 0..START_LIMIT {
            entries.push(
                commands
                    .reserve(CommandRunId::new(), CancellationToken::new())
                    .expect("room"),
            );
        }
        let refused = commands
            .reserve(CommandRunId::new(), CancellationToken::new())
            .err()
            .expect("the fifth is refused");
        assert_eq!(
            refused.0,
            "refused: 4 commands are already queued or running in this session"
        );
        entries[0].end(Phase::Ended {
            status: CommandRunStatus::Done,
            exit_code: Some(0),
        });
        commands
            .reserve(CommandRunId::new(), CancellationToken::new())
            .expect("an ended entry frees its place");
    }

    /// MOD-152 R1 F2: past the cap a log's lines keep the numbers the command printed them
    /// under: `after_line` and the matched line count from the output's first line.
    #[test]
    fn a_pattern_counts_lines_from_the_offset() {
        let text = format!("{}c\nready\n", truncated_line(41));
        let log = ScrubbedLog::of(text, true, &scrubber());
        let pattern = compile("^ready$").expect("a pattern");
        let found = seen(&log, Some(&pattern), 42, &scrubber());
        assert_eq!(
            (found.line_no, found.next_line, found.total_lines),
            (Some(43), 43, 43)
        );
        assert!(!seen(&log, Some(&pattern), 43, &scrubber()).matched);
        assert_eq!(seen(&log, Some(&pattern), 3, &scrubber()).line_no, Some(43));
    }

    /// MOD-152 R1 F4: the registry keeps at most `ENDED_KEEP` ended entries, the oldest ended
    /// going first; `command_cancel` on a row whose entry went answers its stored status, and a
    /// row never started here is still not found. Real `true` children, in real time.
    #[tokio::test]
    async fn the_registry_lets_the_oldest_ended_entries_go() {
        let store = MemStore::demo();
        let session = session(&store);
        let mut started = Vec::new();
        for _ in 0..ENDED_KEEP + 6 {
            let id = start(&session, checked(&session, "true"), 1)
                .await
                .expect("started");
            let mut state = session
                .commands
                .get(id)
                .expect("an entry")
                .state
                .subscribe();
            tokio::time::timeout(
                Duration::from_secs(30),
                state.wait_for(|phase| phase.ended()),
            )
            .await
            .expect("the row ended in time")
            .expect("the entry");
            started.push(id);
        }
        // The last task prunes after it ends; give it the moment it needs.
        let registry = &session.commands;
        for _ in 0..100 {
            if registry.get(started[5]).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let kept: Vec<bool> = started
            .iter()
            .map(|id| registry.get(*id).is_some())
            .collect();
        assert_eq!(kept.iter().filter(|kept| **kept).count(), ENDED_KEEP);
        assert!(
            kept[..6].iter().all(|kept| !kept),
            "the oldest went: {kept:?}"
        );

        let answer = cancel(&session, started[0]).await.expect("an answer");
        assert_eq!(answer["status"], json!("done"), "{answer}");
        assert_eq!(answer["exit_code"], json!(0));
        assert_eq!(answer["command_run_id"], json!(started[0]));
        assert!(
            cancel(&session, CommandRunId::new()).await.is_err(),
            "a row the session did not start is not found"
        );
    }

    /// D8: a deadline is clamped to `MAX_WAIT`, and one already passed waits not at all.
    #[test]
    fn the_wait_budget_is_clamped() {
        let now = Utc::now();
        assert_eq!(budget(now + TimeDelta::hours(1), now), MAX_WAIT);
        assert_eq!(
            budget(now + TimeDelta::seconds(7), now),
            Duration::from_secs(7)
        );
        assert_eq!(budget(now - TimeDelta::seconds(7), now), Duration::ZERO);
    }

    /// D8: a pattern matches only lines after `after_line`, on scrubbed text; `next_line` goes
    /// on from the match, or from the last line read.
    #[test]
    fn a_pattern_matches_after_the_line_asked_for() {
        let text = "ready 1\nnoise\nready 2\nsecret-4f1e ready\n";
        let pattern = compile("^ready").expect("a pattern");
        let first = seen(&log(text), Some(&pattern), 0, &scrubber());
        assert_eq!(
            (first.matched, first.line_no, first.line.as_deref()),
            (true, Some(1), Some("ready 1"))
        );
        assert_eq!((first.next_line, first.total_lines), (1, 4));
        let second = seen(&log(text), Some(&pattern), first.next_line, &scrubber());
        assert_eq!(
            (second.line_no, second.line.as_deref()),
            (Some(3), Some("ready 2"))
        );
        let none = seen(&log(text), Some(&pattern), 3, &scrubber());
        assert!(!none.matched);
        assert_eq!(none.next_line, 4);
        assert_eq!(
            none.tail, "ready 1\nnoise\nready 2\n[REDACTED] ready\n",
            "the tail is scrubbed"
        );
        let masked = seen(
            &log(text),
            Some(&compile("secret").expect("a pattern")),
            0,
            &scrubber(),
        );
        assert!(!masked.matched, "a pattern never sees what was masked");
    }

    /// D3: lines that cannot be masked match nothing and their tail is withheld by rule.
    #[test]
    fn unmaskable_lines_match_nothing() {
        #[derive(Debug)]
        struct Refuses;
        impl Scrubber for Refuses {
            fn scrub(&self, _: &mut Value) -> Result<(), Unmasked> {
                Err(Unmasked {
                    path: String::new(),
                    rule: "test_rule",
                })
            }
        }
        let pattern = compile("x").expect("a pattern");
        let unmasked = ScrubbedLog::of("x\n".to_owned(), false, &Refuses);
        assert!(unmasked.withheld);
        let seen = seen(&unmasked, Some(&pattern), 0, &Refuses);
        assert!(!seen.matched);
        assert_eq!(
            seen.tail,
            "[log withheld: it matched credential rule test_rule]"
        );
    }

    /// D8: a wait on a live row whose deadline is far waits `MAX_WAIT`, then says the deadline
    /// was reached.
    #[tokio::test(start_paused = true)]
    async fn a_wait_ends_at_the_clamped_deadline() {
        let store = MemStore::demo();
        let session = session(&store);
        let id = CommandRunId::new();
        let live = session
            .commands
            .reserve(id, CancellationToken::new())
            .expect("room");
        let source = Source {
            live: Some(live),
            store: &store,
            run: ids::RUN_2,
            id,
        };
        let pattern = compile("never").expect("a pattern");
        let began = tokio::time::Instant::now();
        let answer = wait(
            &source,
            Some(&pattern),
            0,
            budget(Utc::now() + TimeDelta::hours(1), Utc::now()),
            &scrubber(),
            None,
        )
        .await
        .expect("an answer");
        assert_eq!(began.elapsed(), MAX_WAIT);
        assert_eq!(answer["deadline_reached"], json!(true));
        assert_eq!(answer["status"], json!("queued"));
    }

    /// D8: a wait answers as soon as a matching line is written after the line asked for.
    #[tokio::test(start_paused = true)]
    async fn a_wait_answers_a_line_written_while_it_waits() {
        use crate::tools::command::Watch;
        let store = MemStore::demo();
        let session = session(&store);
        let id = CommandRunId::new();
        let live = session
            .commands
            .reserve(id, CancellationToken::new())
            .expect("room");
        live.running();
        super::lock(live.log()).push(b"ready old\n");
        let writer = Arc::clone(&live);
        let source = Source {
            live: Some(live),
            store: &store,
            run: ids::RUN_2,
            id,
        };
        let pattern = compile("^ready").expect("a pattern");
        let write = async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            super::lock(writer.log()).push(b"noise\nready new\npart");
            writer.wrote();
        };
        let scrubber = scrubber();
        let began = tokio::time::Instant::now();
        let (answer, ()) = tokio::join!(
            wait(&source, Some(&pattern), 1, MAX_WAIT, &scrubber, None),
            write
        );
        let answer = answer.expect("an answer");
        assert!(
            began.elapsed() < Duration::from_secs(4),
            "{:?}",
            began.elapsed()
        );
        assert_eq!(answer["matched"], json!(true), "{answer}");
        assert_eq!(answer["line_no"], json!(3));
        assert_eq!(answer["line"], json!("ready new"));
        assert_eq!(answer["status"], json!("running"));
        assert_eq!(
            answer["total_lines"],
            json!(3),
            "the partial line is held back"
        );
    }

    /// D7: `command_cancel` on a row still queued cancels it; it never ran, and its log is the
    /// reason.
    #[tokio::test(start_paused = true)]
    async fn cancel_ends_a_queued_row() {
        let store = MemStore::demo();
        hold_the_slot(&store).await;
        let session = session(&store);
        let id = start_queued(&session, "echo never").await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(
            session.commands.get(id).expect("an entry").phase(),
            Phase::Queued
        );

        let answer = cancel(&session, id).await.expect("an answer");
        assert_eq!(answer["status"], json!("cancelled"), "{answer}");
        let log = until_row(&store, id, CommandRunStatus::Cancelled).await;
        assert_eq!(log, "[stopped: cancelled by command_cancel]\n");
        assert!(
            cancel(&session, CommandRunId::new()).await.is_err(),
            "a row the session did not start is not found"
        );
    }

    /// D7: the end of the session ends its queued rows `cancelled`, saying why.
    #[tokio::test(start_paused = true)]
    async fn the_end_of_the_session_ends_a_queued_row() {
        let store = MemStore::demo();
        hold_the_slot(&store).await;
        let session = session(&store);
        let id = start_queued(&session, "echo never").await;
        tokio::time::sleep(Duration::from_secs(3)).await;

        session.cancel.cancel();
        let log = until_row(&store, id, CommandRunStatus::Cancelled).await;
        assert_eq!(log, "[stopped: the session ended]\n");
        let mut state = session
            .commands
            .get(id)
            .expect("an entry")
            .state
            .subscribe();
        state
            .wait_for(|phase| phase.ended())
            .await
            .expect("the task ended the entry");
    }
}
