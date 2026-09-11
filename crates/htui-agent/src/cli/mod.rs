//! The degraded CLI transport: an agent that speaks only its own headless JSON stream, reaching
//! the same chat tab, recorder, store rows and replay as an ACP one (`docs/ANA-4.md` §4.3, §4.4,
//! §6.2, §7; MOD-2 milestone 8).
//!
//! The split mirrors `acp/`: the supervisor and the session task live in this file, and the
//! wire → [`DriverEvent`] mapping lives alone in [`claude`], which imports no process type and is
//! unit-testable from a single recorded line.
//!
//! **What differs from `acp/`, stated once.** There is no protocol layer at all: a line out is a
//! user message, a line in is a JSON value, and nothing negotiates. So there is no handshake beyond
//! the first `system/init`, no permission channel (§4.3 fixes
//! `DriverCaps { permission_requests: false, edit_proposals: false, plans: false }` for this
//! transport and [`AgentSession::answer_permission`] answers [`DriverError::Unsupported`]), and a
//! cancel is a **signal** rather than a notification — which is why it is the one sequence in this
//! file written from measurements instead of from a specification (plan D81, findings F-1..F-3).
//!
//! [`DriverEvent`]: crate::event::DriverEvent

pub mod claude;

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use htui_core::model::{Agent as AgentRow, AgentBox};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
    PermissionRequestId, SessionSpec,
};
use crate::error::{DriverError, Result};
use crate::event::{
    DoneEvent, DriverEnvelope, DriverEvent, ErrorEvent, OtherEvent, SESSION_STARTED, Stamp,
    StopReason, TRANSPORT_CLOSED, TerminalReason, ToolResultEvent, ToolResultStatus,
};
use crate::launch::{
    AgentLaunch, AgentSettings, ChildGuard, ChildIo, CliSettings, ResolvedLaunch, StopSignal,
};
use crate::registry::TransportBuilder;

/// The adapter id this transport registers under (plan D12): `cli/<settings.cli.stream>`.
pub const ADAPTER_ID: &str = "cli/claude_stream_json";

/// The `settings.cli.stream` value that selects it — half of [`ADAPTER_ID`], and what a registry
/// row declares.
///
/// A **dialect** name, not an agent name (`R-AGT-5`): two rows may declare it, and nothing in this
/// module ever reads `agent.name` to decide anything.
pub const STREAM: &str = "claude_stream_json";

/// How long the first `system/init` may take before [`open_session`] gives up.
///
/// The CLI's login refusal prints to stderr and exits, which is EOF and is reported at once; this
/// bounds the other shape — an agent that hangs before saying anything — so a failed `ChatStart`
/// cannot hold the tab that issued it forever.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Depth of the session task's event channel; [`crate::acp::EVENTS_CAPACITY`]'s reason, and the
/// same number so the two transports back-pressure a slow consumer alike.
pub const EVENTS_CAPACITY: usize = 256;

/// The grace window a session gets when its **handle** is dropped rather than cancelled;
/// [`crate::acp::DROP_GRACE`]'s reason.
pub const DROP_GRACE: Duration = Duration::from_secs(1);

/// The most one stdout line may occupy before the reader forwards it unfinished.
///
/// One mebibyte, which is two orders of magnitude above the largest line any recorded transcript
/// holds (a `system/init` listing this box's tools and commands) and small enough that a child
/// writing without newlines cannot exhaust memory through it. See [`read_lines`].
const MAX_LINE_BYTES: u64 = 1024 * 1024;

/// `other.update` of a stdout line that is not JSON at all.
///
/// Not an error, and deliberately not a reason to stop reading (blueprint H-23): §6.2's rule for a
/// shape `htui` does not recognize is "stored verbatim", and a line a future release prints in
/// front of its stream — a warning, a progress bar, a crash trace — is exactly the thing a reader
/// of the transcript will want. No recorded transcript contains one (F-15), which is why this is
/// written from the rule rather than from a fixture.
pub const UNPARSED: &str = "<unparsed>";

// ---------------------------------------------------------------------------------------------
// The invocation (`docs/ANA-4.md` §4.4)
// ---------------------------------------------------------------------------------------------

/// The argv of `docs/ANA-4.md` §4.4, assembled from the row and the spec. Pure, and unit-tested as
/// a list rather than through a process.
///
/// Order, and every position in it is a decision:
///
/// 1. the row's own resolved `args` first, so a registry row that wraps the CLI in something (`npx`,
///    a shim) keeps its own leading arguments where that something expects them;
/// 2. the fixed flags §4.4 verified: `-p` with **no positional prompt**, because the prompt travels
///    on stdin as the first user message and an argv is visible in `ps` to every account on the box
///    (blueprint P-2);
/// 3. the row's `--permission-mode`, omitted when the row names none rather than guessed at;
/// 4. the session id **or** the resume id, never both — `--session-id` mints (D84), `--resume`
///    continues, and the CLI refuses the pair (blueprint H-18);
/// 5. the spec's model and extra directories;
/// 6. the budget, **only above zero** — see below;
/// 7. `settings.cli.extra_args` **last**, so an operator's repeated flag is the one the CLI keeps.
///
/// **The budget flag is omitted at zero, and that is a measurement, not a nicety** (plan F-10):
/// `--max-budget-usd 0` is refused before the CLI reads a byte of stdin — the process exits 1 with
/// no stdout at all — so passing it for an absent cap would turn "no cap" into "no turn".
#[must_use]
pub fn argv(
    row_args: &[String],
    cli: &CliSettings,
    spec: &SessionSpec,
    session_id: &str,
) -> Vec<String> {
    let mut args: Vec<String> = row_args.to_vec();
    // `--verbose` is what makes `stream-json` emit every envelope rather than the terminal
    // `result` alone, and `--include-partial-messages` is what turns the `stream_event` channel on
    // — the deltas the chat tab renders as the reply arrives (§4.4).
    args.extend(
        [
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
        ]
        .map(ToOwned::to_owned),
    );

    // Scoped so the borrow ends before `extra_args` is appended: the flags below are all pairs,
    // and spelling `push` twice per pair is what a reader has to check for a transposition.
    {
        let mut push = |flag: &str, value: &str| {
            args.push(flag.to_owned());
            args.push(value.to_owned());
        };
        if !cli.permission_mode.is_empty() {
            push("--permission-mode", &cli.permission_mode);
        }
        match spec.resume.as_ref() {
            Some(resume) => push("--resume", resume.as_str()),
            None => push("--session-id", session_id),
        }
        if let Some(model) = spec.model.as_deref() {
            push("--model", model);
        }
        for dir in &spec.extra_dirs {
            push("--add-dir", &dir.to_string_lossy());
        }
        if let Some(micros) = spec.budget_micros.filter(|micros| *micros > 0) {
            push("--max-budget-usd", &usd(micros));
        }
    }

    args.extend(cli.extra_args.iter().cloned());
    args
}

/// USD micros as the decimal `--max-budget-usd` takes: integer arithmetic, six places, no float.
///
/// `300` → `"0.000300"`, `1_500_000` → `"1.500000"`. A float round-trip is what this exists to
/// avoid — the recorder's client-side cap and the CLI's server-side one must read **one** number
/// (D83, D90), and two caps that disagree in the sixth decimal place are worse than one.
///
/// A negative figure never reaches here: `ProjectCaps::from_settings` refuses it and [`argv`] gates
/// on `> 0` besides. The `debug_assert!` says so where it would be violated, and the clamp keeps
/// the release build producing a well-formed decimal rather than the `-0.-000300` the naive
/// arithmetic would emit.
#[must_use]
pub fn usd(micros: i64) -> String {
    debug_assert!(micros >= 0, "a per-run cap in micros is never negative");
    let micros = micros.max(0);
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

/// One `--input-format stream-json` user message, as the line it is written as.
///
/// **The measured shape, not the documented one.** Every one of the fourteen recorded probe
/// transcripts wrote exactly these three nested keys and nothing else, and the CLI accepted all of
/// them (`tests/fixtures/claude_stream_json_*.jsonl`, plan T55/T56). The vendor SDK additionally
/// carries a `session_id` on each line; it is left off here because the fixtures are this
/// milestone's evidence and none of them contains one — the id is already on the argv, which is
/// where `--session-id` put it and where `system/init` echoes it back from.
fn stdin_line(text: &str) -> String {
    let line = json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    });
    // `Value`'s `Display` is compact JSON and cannot fail, which `serde_json::to_string` can only
    // promise through a `Result` nobody here could act on.
    format!("{line}\n")
}

// ---------------------------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------------------------

/// Where a session's byte streams come from; `crate::acp`'s fork, for its reasons.
enum IoSource {
    /// Spawn a child at `start`: what the probe recorded when there is a usable recording,
    /// otherwise the row's tools resolved and substituted (D58, [`crate::launch::launch_from`]).
    Spawn {
        /// The row's `launch` document.
        launch: Box<AgentLaunch>,
        /// `agent_box.probe.resolved`, when the snapshot passed D58's three row-side rules.
        recorded: Option<ResolvedLaunch>,
    },
    /// A pre-built pair, taken once — a real process starts once too.
    ///
    /// Boxed for `acp::IoSource::Prepared`'s reason: a [`ChildIo`] carries a `Spawned`, which is
    /// materially larger on Windows, and an unboxed variant would make every session pay for the
    /// test-support one there.
    #[cfg(feature = "test-support")]
    Prepared(Mutex<Option<Box<ChildIo>>>),
}

/// The driver for a `cli` row whose `settings.cli.stream` is [`STREAM`]: one per row, holding no
/// process.
pub struct CliDriver {
    name: String,
    /// `agent.models`, the banner's fallback when `system/init` names no model (D84).
    models: Vec<String>,
    /// `settings.cli` is what [`argv`] reads; `settings.usage.scope` is what every `usage` row the
    /// mapper writes is labelled with (§5.2, §7).
    settings: AgentSettings,
    caps: DriverCaps,
    io: IoSource,
    stamp: Stamp,
    /// `agent_box.version` — the probe's `claude --version` capture — the banner's `agent_version`
    /// fallback when `system/init` names no version.
    box_version: Option<String>,
}

impl core::fmt::Debug for CliDriver {
    /// `AcpDriver`'s fields, for its reason: a [`ResolvedLaunch`] carries an environment, so a log
    /// line says **which** of D58's two paths this driver is on and never what is on it.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CliDriver")
            .field("name", &self.name)
            .field("caps", &self.caps)
            .field("stamp", &self.stamp)
            .field(
                "recorded",
                &matches!(
                    self.io,
                    IoSource::Spawn {
                        recorded: Some(_),
                        ..
                    }
                ),
            )
            .finish()
    }
}

impl CliDriver {
    /// Builds a driver from a registry row.
    ///
    /// # Errors
    /// [`DriverError::Transport`] when `agent.launch` does not parse as the §5.1 document. An
    /// unreadable `agent.settings` is **not** fatal: it falls back to the documented defaults,
    /// exactly as `crate::registry` does, because the column is hand-editable and a session that
    /// cannot read it can still run — it simply passes no `--permission-mode`.
    pub fn from_row(agent: &AgentRow, caps: DriverCaps) -> Result<Self> {
        Self::from_row_with_probe(agent, None, caps)
    }

    /// [`from_row`](Self::from_row) plus this box's `agent_box` row (plan D58).
    ///
    /// `on_box` is read here, once, rather than at every `start`, and the transport check is
    /// applied on this side of `recorded_launch` for `AcpDriver::from_row_with_probe`'s reason:
    /// `agent.transport` is hand-editable, and a recording made for the *other* transport is not
    /// stale but wrong.
    ///
    /// # Errors
    /// As [`from_row`](Self::from_row). An unreadable `agent_box.probe` is never an error.
    pub fn from_row_with_probe(
        agent: &AgentRow,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Self> {
        let launch: AgentLaunch = serde_json::from_value(agent.launch.clone())
            .map_err(|err| DriverError::Transport(format!("agent.launch does not parse: {err}")))?;
        let recorded = on_box
            .and_then(crate::probe::ProbeSnapshot::from_row)
            .filter(|snapshot| snapshot.transport == agent.transport)
            .and_then(|snapshot| snapshot.recorded_launch().cloned());
        Ok(Self {
            name: agent.name.clone(),
            models: agent.models.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Spawn {
                launch: Box::new(launch),
                recorded,
            },
            stamp: Stamp::Wall,
            box_version: on_box.and_then(|on_box| on_box.version.clone()),
        })
    }

    /// What this session would spawn, before spawning it: D58's rules for the spec's `cwd`, with
    /// the spec's own environment applied over the row's.
    ///
    /// `spec.env` is applied **last** and wins (`R-SEC-2`): the row's environment holds paths, the
    /// spec's holds what the secret provider produced for this run. The argv is **not** assembled
    /// here — [`argv`] needs the minted session id, which is `start`'s to make.
    ///
    /// # Errors
    /// [`crate::launch::launch_from`]'s, unchanged; [`DriverError::Transport`] for a prepared
    /// transport, which spawns nothing and so has no launch to describe.
    pub async fn launch_for(&self, spec: &SessionSpec) -> Result<ResolvedLaunch> {
        let (launch, recorded) = match &self.io {
            IoSource::Spawn { launch, recorded } => (launch, recorded),
            #[cfg(feature = "test-support")]
            IoSource::Prepared(_) => {
                return Err(DriverError::Transport(
                    "a prepared transport spawns nothing".to_owned(),
                ));
            }
        };
        let mut resolved = crate::launch::launch_from(launch, recorded.as_ref(), &spec.cwd).await?;
        resolved.env.extend(spec.env.clone());
        Ok(resolved)
    }

    /// This row's `settings.cli`, or the documented defaults when the row carries no block.
    ///
    /// A `cli` row with no block cannot reach this driver — the registry derives the bare `cli`
    /// adapter id for it and answers [`DriverError::UnknownAdapter`] — but a driver built by hand
    /// can, and an empty block passes no `--permission-mode` and no `extra_args`, which is the same
    /// invocation minus the row's opinions.
    fn cli_settings(&self) -> CliSettings {
        self.settings.cli.clone().unwrap_or_default()
    }

    /// A driver over a prepared pair with a deterministic clock: the conformance harness.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn over(io: ChildIo, agent: &AgentRow, caps: DriverCaps, stamp: Stamp) -> Self {
        Self {
            name: agent.name.clone(),
            models: agent.models.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Prepared(Mutex::new(Some(Box::new(io)))),
            stamp,
            box_version: None,
        }
    }

    /// The streams this session runs over, with the child started if there is one to start.
    async fn io(&self, spec: &SessionSpec, session_id: &str) -> Result<ChildIo> {
        match &self.io {
            IoSource::Spawn { .. } => {
                let mut resolved = self.launch_for(spec).await?;
                resolved.args = argv(&resolved.args, &self.cli_settings(), spec, session_id);
                let spawned = crate::launch::spawn(&resolved, &spec.cwd).await?;
                ChildIo::from_spawned(spawned)
            }
            #[cfg(feature = "test-support")]
            IoSource::Prepared(slot) => slot
                .lock()
                .map_err(|_| {
                    DriverError::Transport("the prepared transport is poisoned".to_owned())
                })?
                .take()
                .map(|io| *io)
                .ok_or_else(|| {
                    DriverError::Transport("this driver's transport was already used".to_owned())
                }),
        }
    }
}

impl AgentDriver for CliDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn caps(&self) -> DriverCaps {
        self.caps
    }

    // No `authenticate` override: the default body answers `DriverError::Unsupported`, which is
    // what `caps_from`'s `authenticate: false` for every `cli` row promises (plan MOD-21 D10). The
    // vendor CLI's own login is not `htui`'s to drive and a stream adapter has nowhere to put a
    // method list.
    fn start<'a>(
        &'a self,
        spec: SessionSpec,
        prompt: String,
    ) -> DriverFuture<'a, Box<dyn AgentSession>> {
        Box::pin(async move {
            // **Minted before the spawn**, because the argv carries it (D84): `--session-id` is
            // what makes the id `htui`'s to choose rather than the agent's to report, which is what
            // `session_ref` promises a later step's `--resume`. A resuming session has an id
            // already and mints nothing — the two flags are exclusive (blueprint H-18) — so the
            // mint is skipped rather than made and thrown away.
            let session_id = spec.resume.as_ref().map_or_else(
                || Uuid::now_v7().to_string(),
                |resume| resume.as_str().to_owned(),
            );
            let io = self.io(&spec, &session_id).await?;
            let options = SessionOptions {
                agent_name: self.name.clone(),
                models: self.models.clone(),
                settings: self.settings.clone(),
                stamp: self.stamp,
                init_timeout: INIT_TIMEOUT,
                box_version: self.box_version.clone(),
                session_id,
            };
            let session = open_session(io, spec, prompt, options).await?;
            Ok(Box::new(session) as Box<dyn AgentSession>)
        })
    }
}

/// The [`TransportBuilder`] registered under [`ADAPTER_ID`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeStreamAdapter;

impl TransportBuilder for ClaudeStreamAdapter {
    fn build(
        &self,
        agent: &AgentRow,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>> {
        Ok(Box::new(CliDriver::from_row_with_probe(
            agent, on_box, caps,
        )?))
    }
}

// ---------------------------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------------------------

/// What the handle asks the session task to do.
///
/// No `AnswerPermission`: this transport announces no request, so there is nothing to answer
/// (§4.3, and [`AgentSession::answer_permission`] below).
#[derive(Debug)]
pub enum Command {
    /// Start a new turn with this text.
    FollowUp(String),
    /// End the session. Acknowledged **after** the process tree is gone, so a caller that awaited
    /// `cancel` knows there is nothing left running (§11 criterion 11).
    Cancel {
        /// How long the drain may take before the tree is killed (plan D81).
        grace: Duration,
        /// Answered last.
        done: oneshot::Sender<()>,
    },
}

/// A live CLI session: channel endpoints and handle-side bookkeeping only.
pub struct CliSession {
    /// The id `htui` minted and passed as `--session-id` (D84).
    session_ref: AgentSessionRef,
    events: mpsc::Receiver<DriverEnvelope>,
    commands: mpsc::UnboundedSender<Command>,
    /// Envelopes drained while `cancel` waited for its acknowledgement; served before `events`.
    pending: VecDeque<DriverEnvelope>,
    /// `false` between a handed-out `done` and the next accepted follow-up.
    turn_open: bool,
    /// The task has ended: `next_event` answers `Ok(None)`, everything else
    /// [`DriverError::Closed`].
    ended: bool,
    task: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for CliSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CliSession")
            .field("session_ref", &self.session_ref)
            .field("pending", &self.pending.len())
            .field("turn_open", &self.turn_open)
            .field("ended", &self.ended)
            .finish()
    }
}

impl AgentSession for CliSession {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        Some(&self.session_ref)
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            // No parked check, and nothing to write one about: this transport has no permission
            // channel, so a pull can never be refused for owing the agent an answer.
            if let Some(envelope) = self.pending.pop_front() {
                self.note(&envelope);
                return Ok(Some(envelope));
            }
            if self.ended {
                return Ok(None);
            }
            match self.events.recv().await {
                Some(envelope) => {
                    self.note(&envelope);
                    Ok(Some(envelope))
                }
                None => {
                    self.ended = true;
                    Ok(None)
                }
            }
        })
    }

    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Err(DriverError::Closed);
            }
            if text.is_empty() {
                return Err(DriverError::Transport(
                    "a follow-up must have text".to_owned(),
                ));
            }
            if self.turn_open {
                return Err(DriverError::Transport(
                    "a follow-up before the turn's done would interleave two turns".to_owned(),
                ));
            }
            self.send(Command::FollowUp(text))?;
            self.turn_open = true;
            Ok(())
        })
    }

    /// There is no permission request to answer, and the honest error says so about the
    /// **operation** rather than about the id.
    ///
    /// [`DriverError::Unsupported`] and not `Transport("no parked request …")`: the latter claims
    /// the id is unknown, when the truth is that this transport has no such channel at all — which
    /// is what `Unsupported` was added for, and `caps.permission_requests == false` is the
    /// predicate that pairs with it, exactly as `caps.authenticate` pairs with `authenticate`.
    ///
    /// [`DriverError::Closed`] is checked first because the trait's contract for every operation is
    /// "`Closed` once the session has ended": a session that is over should not be arguing about an
    /// operation it never had the chance to refuse.
    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        let _ = (request_id, answer);
        Box::pin(async move {
            if self.ended {
                return Err(DriverError::Closed);
            }
            Err(DriverError::Unsupported("answer_permission"))
        })
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Ok(());
            }
            let (done, mut ack) = oneshot::channel();
            if self.send(Command::Cancel { grace, done }).is_err() {
                // `send` has already drained the task's last rows into `pending` and marked the
                // session ended. A cancel of a session that is already over is `Ok`, not an error
                // — there is nothing left to stop — but the rows it wrote on its way out are still
                // owed to the recorder.
                return Ok(());
            }
            // Keep draining while the task shuts down: the drained `result`, the synthesized
            // results and the `done` are ordinary events, and dropping them here would lose rows
            // the recorder still has to write.
            loop {
                tokio::select! {
                    _ = &mut ack => break,
                    event = self.events.recv() => match event {
                        Some(envelope) => self.pending.push_back(envelope),
                        None => {
                            self.ended = true;
                            break;
                        }
                    },
                }
            }
            // **Not** `ended = true`: some of the cancel's own rows may still be in the channel
            // when the acknowledgement wins the `select!` above, and the caller pulls them exactly
            // as it pulls any other event. The task acknowledges *after* it has killed the process
            // tree and returns immediately afterwards, so joining it here is what makes "cancel
            // returned" mean "the tree is gone" — which is what §11 criterion 11 measures.
            if let Some(task) = self.task.take()
                && let Err(err) = task.await
                && !err.is_cancelled()
            {
                tracing::warn!(%err, "the session task panicked on its way out");
            }
            Ok(())
        })
    }
}

impl CliSession {
    /// Handle-side bookkeeping, applied as an envelope is handed out rather than as it arrives:
    /// what the caller has *seen* is what decides whether a follow-up is legal.
    fn note(&mut self, envelope: &DriverEnvelope) {
        if matches!(envelope.event, DriverEvent::Done(_)) {
            self.turn_open = false;
        }
    }

    /// Sends a command, turning a dead task into [`DriverError::Closed`].
    ///
    /// **The rows the task already wrote are rescued before the session is marked ended**, because
    /// a dead task is usually one that has just finished saying something: the EOF path writes
    /// `error{transport_closed}`, a synthesized `failed` result for every open call, and a
    /// `done{cancelled}` — and *then* the task returns, which is what makes this send fail.
    /// [`Self::next_event`] honours `ended` before it looks at the channel, so marking it first
    /// would answer `Ok(None)` over a queue still holding the turn's last rows, and the recorder
    /// would never write them. Draining into `pending` costs nothing and keeps the transcript
    /// complete on exactly the path where it is hardest to reconstruct.
    fn send(&mut self, command: Command) -> Result<()> {
        if self.commands.send(command).is_err() {
            self.drain_into_pending();
            self.ended = true;
            return Err(DriverError::Closed);
        }
        Ok(())
    }

    /// Moves everything the task has already queued into `pending`, which `next_event` serves
    /// ahead of the channel and ahead of `ended`.
    fn drain_into_pending(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.pending.push_back(event);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------------------------

/// Everything the session task needs besides the streams.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// `agent.name`, for the banner and the logs. The stream carries no agent identity of its own.
    pub agent_name: String,
    /// `agent.models`, the banner's fallback when `system/init` names no model.
    pub models: Vec<String>,
    /// The parsed `agent.settings` (§5.2).
    pub settings: AgentSettings,
    /// How capture times are stamped.
    pub stamp: Stamp,
    /// How long the first `system/init` may take before [`open_session`] gives up.
    ///
    /// [`INIT_TIMEOUT`] in production. A field rather than the constant read at the point of use
    /// for `acp::SessionOptions::handshake_timeout`'s reason: the arm that matters is the one that
    /// has to kill the child it gave up on, and a minute-long test is a test nobody runs.
    pub init_timeout: Duration,
    /// `agent_box.version`, the banner's second source for `agent_version`.
    pub box_version: Option<String>,
    /// The id passed as `--session-id`, or the one `--resume` continues (D84).
    pub session_id: String,
}

/// Opens a session: spawns the task, waits for `system/init`, returns the handle.
///
/// # Errors
///
/// [`DriverError::Transport`] carrying whatever went wrong, with the child's captured stderr
/// appended when there was a child. Three failing exits, and every one of them returns with the
/// kill already sent — `acp::open_session`'s three, for its reasons, with its one asymmetry:
///
/// 1. **The agent never said anything.** The task is aborted and awaited, which resolves as soon as
///    the runtime has dropped the future; the [`ChildGuard`]'s `Drop` has then signalled the tree.
///    Signalled but not *reaped* by us, because a `Drop` cannot await.
/// 2. **The agent ended before `system/init`** — an unauthenticated CLI prints its refusal to
///    stderr and exits, which is EOF. Composed on the task's own timeline with the stderr tail
///    attached, so the tree is killed *and* reaped before this returns.
/// 3. **Nobody answered at all**, the sender dropped with the task. Awaited exactly as (2) is.
pub async fn open_session(
    io: ChildIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
) -> Result<CliSession> {
    let (events_tx, events_rx) = mpsc::channel(EVENTS_CAPACITY);
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();

    let timeout = options.init_timeout;
    // Built here and not carried back from the task: over ACP the id is the *agent's* answer to
    // `session/new` and has to travel, but `--session-id` makes it `htui`'s own (D84) — so the task
    // answers only *whether* the stream opened, and a disagreement with what `system/init` echoes
    // is a `warn!` there rather than a value here (blueprint H-17).
    let session_ref = AgentSessionRef::new(options.session_id.clone());
    let task = tokio::spawn(run_session(
        io,
        spec,
        prompt,
        options,
        ready_tx,
        events_tx,
        commands_rx,
    ));

    match tokio::time::timeout(timeout, ready_rx).await {
        Err(_) => {
            // Exit (1). The aborted task drops its `ChildGuard`, whose `Drop` signals the kill;
            // awaiting the cancelled handle resolves as soon as the runtime has dropped the future
            // and is what makes this `Err` mean "the kill has been sent" rather than "the kill will
            // be sent shortly". The reap is tokio's orphan queue's, as `acp::open_session` explains
            // at length.
            task.abort();
            let _ = task.await;
            Err(DriverError::Transport(format!(
                "the agent did not send its `system/init` within {}s",
                timeout.as_secs()
            )))
        }
        Ok(Ok(Ok(()))) => Ok(CliSession {
            session_ref,
            events: events_rx,
            commands: commands_tx,
            pending: VecDeque::new(),
            // The first prompt has already gone out, so turn 0 is open before the caller has the
            // handle: a follow-up now would interleave two turns.
            turn_open: true,
            ended: false,
            task: Some(task),
        }),
        Ok(Ok(Err(err))) => {
            let _ = tokio::time::timeout(timeout, task).await;
            Err(err)
        }
        Ok(Err(_)) => {
            let _ = tokio::time::timeout(timeout, task).await;
            Err(DriverError::Transport(format!(
                "the session task ended before `system/init` for session {}",
                session_ref.as_str()
            )))
        }
    }
}

/// The session task: owns the child, the stdout reader and the mapper.
async fn run_session(
    io: ChildIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    ready: oneshot::Sender<Result<()>>,
    events: mpsc::Sender<DriverEnvelope>,
    commands: mpsc::UnboundedReceiver<Command>,
) {
    let ChildIo {
        reader,
        writer,
        child,
    } = io;
    // **The child is owned here**, by the task that outlives the `start` which created it, and
    // through a [`ChildGuard`] rather than a bare `Spawned` for the one exit that runs no code of
    // ours: an **aborted** task drops the guard, whose `Drop` signals the kill, which is what
    // [`open_session`]'s timeout arm relies on. `acp::run_session`'s shape, copied.
    let child = Arc::new(Mutex::new(ChildGuard::new(child)));

    // **The stdout reader is its own task**, and that is not decoration. `read_until` is not
    // cancellation-safe: driven directly from the `select!` below it would lose a partly-read line
    // every time a command won the race, which on this wire means losing a turn's `result`. A
    // channel receive *is* cancellation-safe, so the line splitting happens over there and the
    // supervisor only ever selects over whole lines.
    let (lines_tx, lines_rx) = mpsc::channel(EVENTS_CAPACITY);
    let reading = tokio::spawn(read_lines(reader, lines_tx));

    session_main(
        spec, prompt, options, ready, events, commands, lines_rx, writer, &child,
    )
    .await;

    // A spawned child's stdout ends when the kill below closes it, but a prepared pair's writer is
    // held by whoever built it and may never close: the reader is aborted rather than left waiting
    // on a stream this session has stopped caring about.
    reading.abort();
    // Unconditional, and a no-op after the cancel path's own kill.
    kill(&child).await;
}

/// Splits the agent's stdout into lines and forwards them, whatever bytes it finds.
///
/// `read_until` + `from_utf8_lossy` and **not** `lines()`, which is the stderr reader's rule
/// (`launch.rs`) for a sharper reason here (blueprint H-23): `lines()` answers `Err(InvalidData)`
/// on a single byte that is not UTF-8 and then **ends**, so one stray byte anywhere in a transcript
/// would end the stream mid-turn and be recorded as a transport that closed. The `\r` a Windows
/// child writes before its `\n` is stripped for the same reason — a CRLF line that reached the JSON
/// parser with its carriage return still attached would parse, and then the *last* line of the
/// stream would not.
///
/// **The line is capped** (review gate, LOW). `read_until` grows its buffer until it finds the
/// delimiter, so a child that writes megabytes without a newline — a crash dump, a binary blob, a
/// subprocess whose output got redirected into ours — would grow this buffer without limit inside
/// a supervisor whose whole job is to survive the agent misbehaving. At the cap the partial line is
/// forwarded as it stands and the reader keeps going, so the transcript records what arrived (it
/// lands in `other` as unparsed, §6.2's rule) rather than either truncating in silence or holding
/// the whole thing in memory. No recorded transcript comes near it: the largest real line in the
/// fixtures is a `system/init` of a few kilobytes.
async fn read_lines(reader: Box<dyn AsyncRead + Send + Unpin>, lines: mpsc::Sender<String>) {
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match (&mut reader)
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut buffer)
            .await
        {
            // End of stream.
            Ok(0) => break,
            Ok(_) => {}
            // A real I/O failure on the pipe: there is nothing left to read from a descriptor that
            // answers an error, and the supervisor reads the close as an EOF either way.
            Err(_) => break,
        }
        if buffer.last() == Some(&b'\n') {
            buffer.pop();
            if buffer.last() == Some(&b'\r') {
                buffer.pop();
            }
        }
        let line = String::from_utf8_lossy(&buffer).into_owned();
        // A blank line is not an envelope and carries nothing; forwarding it would make every
        // drain and every end-of-turn check step over it.
        if line.trim().is_empty() {
            continue;
        }
        if lines.send(line).await.is_err() {
            break;
        }
    }
}

/// One stdout line, classified once so the supervisor's three readers agree about it.
enum Line {
    /// A JSON envelope, for the mapper.
    Json(Value),
    /// Anything else, kept verbatim under [`UNPARSED`].
    Text(String),
}

/// A line as one of the two shapes that can arrive.
fn classify(text: String) -> Line {
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => Line::Json(value),
        Err(_) => Line::Text(text),
    }
}

/// Whether a line is the `system/init` that opens the stream (§6.2).
fn is_init(line: &Value) -> bool {
    claude::kind_of(line) == ("system", Some("init"))
}

/// The task's own state machine: the prompt, the banner, then the turn loop.
#[expect(
    clippy::too_many_arguments,
    reason = "the task owns nine distinct things; bundling them into a struct renames the arity \
              without reducing it"
)]
async fn session_main(
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    ready: oneshot::Sender<Result<()>>,
    events: mpsc::Sender<DriverEnvelope>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut lines: mpsc::Receiver<String>,
    writer: Box<dyn AsyncWrite + Send + Unpin>,
    child: &Mutex<ChildGuard>,
) {
    let mut state = TaskState::new(options.stamp, spec.retain_raw, options.settings.usage.scope);
    let session_ref = AgentSessionRef::new(options.session_id.clone());
    // An `Option` because a cancel **takes** it: closing stdin is the documented end of input for
    // `--input-format stream-json`, and the only way to close it is to drop it (plan D81 step 1).
    let mut writer = Some(writer);

    // 1. The prompt goes out first, on stdin, never on the argv (blueprint P-2). The turn is
    //    already open at this point in the sense that matters — a failure here still owes the
    //    caller an answer, and it gets one through `ready` rather than through a `done`, because
    //    the handle does not exist yet.
    if let Some(sink) = writer.as_mut()
        && let Err(err) = write_line(sink, &stdin_line(&prompt)).await
    {
        let _ = ready.send(Err(with_stderr(&err.to_string(), child)));
        kill(child).await;
        return;
    }

    // 2. Wait for `system/init`, buffering everything that arrives first.
    //
    //    **The buffer is required, and F-9 is why.** Without `--bare` — which D92 dropped from the
    //    seed because it refuses to read an OAuth login at all — three `system/hook_started` and
    //    three `system/hook_response` envelopes arrive *before* `init`, and in the recorded run
    //    before the first stdin line had even been written. Mapping them as they arrived would put
    //    an `other` row in front of the banner, and "the session banner is the step's first `other`
    //    row" is a conformance case (§4.4, D84). So they wait here and are released in arrival
    //    order behind the banner.
    let mut pre_init: Vec<Line> = Vec::new();
    let init = loop {
        match lines.recv().await {
            Some(text) => match classify(text) {
                Line::Json(value) if is_init(&value) => break value,
                line => pre_init.push(line),
            },
            // The stream ended without an `init`. This is where an unauthenticated box's refusal
            // surfaces: the CLI prints it to stderr and exits, so the stderr tail is the whole of
            // what the user needs to read.
            None => {
                let _ = ready.send(Err(with_stderr(
                    "the agent ended before sending `system/init`",
                    child,
                )));
                kill(child).await;
                return;
            }
        }
    };

    // 3. The banner, first row of the step (D84).
    if let Some(reported) = init.get("session_id").and_then(Value::as_str)
        && reported != session_ref.as_str()
    {
        // Blueprint H-17: the id `htui` minted is the one `--resume` will take, so the banner and
        // `session_ref` carry ours whatever the CLI echoes. A disagreement is worth a log line and
        // is not worth failing a session over.
        tracing::warn!(
            minted = %session_ref,
            reported = %reported,
            "the CLI reported a session id other than the one it was given"
        );
    }
    let banner = DriverEvent::Other(OtherEvent {
        update: SESSION_STARTED.to_owned(),
        body: json!({
            "session_id": session_ref.as_str(),
            // The stream negotiates nothing, so there is no version to report — and reporting a
            // `1` copied from ACP would be a claim about a handshake that never happened.
            "protocol_version": Value::Null,
            // The row's name: the stream carries no agent identity of its own.
            "agent_name": options.agent_name,
            "agent_version": agent_version(&init, options.box_version.as_deref()),
            "models": banner_models(&init, &options.models),
        }),
    });
    let raw = state.retain_raw.then(|| init.clone());
    if !emit(&mut state, &events, banner, raw).await {
        kill(child).await;
        return;
    }
    for line in pre_init {
        if !on_line(&mut state, &events, line).await {
            kill(child).await;
            return;
        }
    }

    // 4. The handle may exist now: everything above is what `start` promised to have done.
    if ready.send(Ok(())).is_err() {
        kill(child).await;
        return;
    }

    // 5. The turn loop. Both arms are cancellation-safe channel receives.
    loop {
        let step = tokio::select! {
            line = lines.recv() => Step::Line(line),
            command = commands.recv() => Step::Command(command),
        };
        match step {
            Step::Line(Some(text)) => {
                if !on_line(&mut state, &events, classify(text)).await {
                    break;
                }
            }
            // EOF. **Between turns this is a clean end** — F-1 measured it: the CLI runs a turn to
            // completion and exits 0 after its stdin closes, so a stream that stops when nothing is
            // open is a session that finished. **Mid-turn it is the milestone-3 defect class**
            // (`682a423`, "a stream ending before its `done` was recorded as a finished turn"), and
            // it is closed the same way `acp`'s `Step::Closed` closes it: an `error` naming the
            // transport, every open call given its synthesized `failed`, and exactly one
            // `done { cancelled }` — never a silent `Ok(None)`.
            Step::Line(None) => {
                if state.turn_open {
                    let event = DriverEvent::Error(ErrorEvent {
                        code: TRANSPORT_CLOSED.to_owned(),
                        message: stderr_tail(child),
                    });
                    emit(&mut state, &events, event, None).await;
                    close_turn(&mut state, &events, StopReason::Cancelled).await;
                }
                break;
            }
            Step::Command(Some(Command::FollowUp(text))) => {
                // Open the turn **before** the write: the handle already counts it as open, and a
                // failure has to be able to close it (`close_turn` is a no-op on a closed turn).
                state.turn_open = true;
                let sent = match writer.as_mut() {
                    Some(sink) => write_line(sink, &stdin_line(&text)).await,
                    // Only a cancel takes the writer, and a cancel returns from this loop — so this
                    // is unreachable, and a named error rather than a panic if it stops being.
                    None => Err(DriverError::Closed),
                };
                if let Err(err) = sent {
                    let event = DriverEvent::Error(ErrorEvent {
                        code: TRANSPORT_CLOSED.to_owned(),
                        message: err.to_string(),
                    });
                    emit(&mut state, &events, event, None).await;
                    close_turn(&mut state, &events, StopReason::Cancelled).await;
                    break;
                }
            }
            Step::Command(Some(Command::Cancel { grace, done })) => {
                cancel_session(&mut state, &events, &mut writer, &mut lines, child, grace).await;
                kill(child).await;
                // Last, so a caller that awaited `cancel` knows the tree is gone (criterion 11).
                let _ = done.send(());
                return;
            }
            // The handle is gone: nobody is reading, so end the session rather than leave a child
            // running for an audience that left.
            Step::Command(None) => {
                cancel_session(
                    &mut state,
                    &events,
                    &mut writer,
                    &mut lines,
                    child,
                    DROP_GRACE,
                )
                .await;
                break;
            }
        }
    }

    kill(child).await;
}

/// One iteration's cause.
enum Step {
    /// A stdout line, or the end of them.
    Line(Option<String>),
    /// A command from the handle, or the handle's disappearance.
    Command(Option<Command>),
}

/// The cancel sequence of plan D81, in the order **F-1, F-2 and F-3 measured** rather than the one
/// `docs/ANA-4.md` §4.4 hypothesised.
///
/// 1. **Close stdin.** The documented end of input for `--input-format stream-json` — and, F-1:
///    *not* a cancel. The CLI runs the open turn to completion and exits 0. So this step is what
///    stops the *next* turn, not this one.
/// 2. **`SIGINT` to the group.** F-2: this is the step that buys a terminal envelope. The CLI
///    answers with a real `result` — an error-shaped one (`subtype: "error_during_execution"`,
///    `is_error: true`) carrying `terminal_reason: "aborted_streaming"`, exit 0 — which the mapper
///    turns into `done { cancelled }` by reading that key rather than by remembering that *we* sent
///    the signal. On Windows there are no signals and `Spawned::signal` says so; that is a step
///    this platform does not have, not a failure, so the sequence goes on to the grace and the
///    kill, which is exactly what the Windows cancel has always been.
/// 3. **Drain for the grace window.** Whatever arrives is emitted in order, and a `done` that
///    arrives is the turn's **real** ending. This ordering is the other half of the milestone-3
///    defect: nothing is synthesized until the read side is exhausted or the deadline passes, so a
///    `result` still in the pipe can never be overtaken by a `done` `htui` invented.
/// 4. **Close the turn if it is still open**, and only then. Cancelling between turns ends the
///    session rather than a turn, so this is a no-op there — and a no-op after a drained `result`,
///    which is what keeps the cancel from writing a second `done`.
///
/// An EOF during the drain writes **no** `error { transport_closed }`: the cancel is the cause of
/// this stream ending, and the `done` below already says so. F-3 is the shape that reaches here —
/// SIGTERM (or a kill) leaves exit 143 and no `result` at all.
async fn cancel_session(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    writer: &mut Option<Box<dyn AsyncWrite + Send + Unpin>>,
    lines: &mut mpsc::Receiver<String>,
    child: &Mutex<ChildGuard>,
    grace: Duration,
) {
    if let Some(mut sink) = writer.take() {
        let _ = sink.shutdown().await;
    }
    interrupt(child);

    if state.turn_open && grace > Duration::ZERO {
        let deadline = tokio::time::sleep(grace);
        tokio::pin!(deadline);
        loop {
            let line = tokio::select! {
                () = &mut deadline => break,
                line = lines.recv() => line,
            };
            let Some(text) = line else { break };
            if !on_line(state, events, classify(text)).await {
                break;
            }
            // The drained `result` closed the turn: its `done` is the real one and there is nothing
            // left to wait for.
            if !state.turn_open {
                break;
            }
        }
    }

    close_turn(state, events, StopReason::Cancelled).await;
}

/// Asks the child's process **group** to stop, and treats a platform that cannot as a step it does
/// not have.
///
/// The group and not the pid: `process-wrap` makes the child a group leader at spawn, so this is a
/// `killpg` that reaches the helpers a CLI agent spawned for itself. A supervisor that signalled
/// only the pid it can name would leave the rest of the tree holding the pipe.
///
/// No lock is held across an await because nothing here awaits: `Spawned::signal` sends and
/// returns, and what the child does with the request is the child's business.
fn interrupt(child: &Mutex<ChildGuard>) {
    let mut guard = child.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(spawned) = guard.child_mut() else {
        return;
    };
    if let Err(err) = spawned.signal(StopSignal::Interrupt) {
        // Windows has no SIGINT and says so, and a group that has already exited answers `ESRCH`.
        // Neither is a reason to stop cancelling: the grace window and the tree kill follow either
        // way, and that pair *is* the Windows cancel.
        tracing::debug!(%err, "the interrupt step of the cancel was unavailable; going on");
    }
}

/// Task-side state, one per session.
struct TaskState {
    stamp: Stamp,
    n: u64,
    retain_raw: bool,
    mapper: claude::Mapper,
    /// Calls announced and not yet settled: what a cancel or an EOF owes a synthesized result.
    open_calls: Vec<String>,
    settled_calls: BTreeSet<String>,
    turn_open: bool,
}

impl TaskState {
    fn new(stamp: Stamp, retain_raw: bool, scope: crate::launch::UsageScope) -> Self {
        Self {
            stamp,
            n: 0,
            retain_raw,
            mapper: claude::Mapper::new(scope),
            open_calls: Vec::new(),
            settled_calls: BTreeSet::new(),
            // The first prompt goes out before this state is used for anything: turn 0 is open.
            turn_open: true,
        }
    }

    /// Wraps an event in its envelope, stamping the capture time.
    ///
    /// With `retain_raw` set, a row that has **no** wire line still carries a `raw` naming what
    /// produced it — the banner, the synthesized results of a cancel, and every `error` `htui`
    /// authored itself are rows a replay has to explain (§11 criterion 4). `acp::TaskState`'s rule,
    /// and the same synthesized shape so the two transports replay alike.
    fn envelope(&mut self, event: DriverEvent, raw: Option<Value>) -> DriverEnvelope {
        let at = self.stamp.at(self.n);
        self.n += 1;
        let raw = if self.retain_raw {
            Some(raw.unwrap_or_else(|| {
                json!({
                    "htui_synthesized": htui_core::model::EventKind::from(&event).as_str(),
                    "n": self.n - 1,
                })
            }))
        } else {
            None
        };
        DriverEnvelope { event, raw, at }
    }

    /// Notes what an outgoing event means for the call bookkeeping of §4.3.
    fn note(&mut self, event: &DriverEvent) {
        match event {
            DriverEvent::ToolCall(call) => {
                if !self.settled_calls.contains(&call.tool_call_id) {
                    self.open_calls.push(call.tool_call_id.clone());
                }
            }
            DriverEvent::ToolResult(result) => {
                self.settled_calls.insert(result.tool_call_id.clone());
                self.open_calls.retain(|id| id != &result.tool_call_id);
            }
            _ => {}
        }
    }
}

/// Sends one event, returning `false` once the consumer is gone.
///
/// `acp::emit`'s rules minus the one that cannot apply: this transport reports no `edit_proposal`
/// (§4.3 fixes `edit_proposals: false`), so there is no `accepted` to fill in.
async fn emit(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    event: DriverEvent,
    raw: Option<Value>,
) -> bool {
    // A `tool_result` for a call a cancel already settled is the stream's own late report: `htui`
    // wrote the synthesized row, and a second one would be two results for one call (§4.3
    // "Tool-call terminal states").
    if let DriverEvent::ToolResult(result) = &event
        && state.settled_calls.contains(&result.tool_call_id)
    {
        return true;
    }
    state.note(&event);
    let envelope = state.envelope(event, raw);
    events.send(envelope).await.is_ok()
}

/// Maps one classified line and emits whatever it produced.
async fn on_line(state: &mut TaskState, events: &mpsc::Sender<DriverEnvelope>, line: Line) -> bool {
    match line {
        Line::Json(value) => {
            // Cloned only when the project asked for it: `retain_raw` off means a transport does
            // not even allocate the verbatim value (`SessionSpec::retain_raw`).
            let raw = state.retain_raw.then(|| value.clone());
            for event in state.mapper.map(&value) {
                let ends_the_turn = matches!(event, DriverEvent::Done(_));
                if !emit(state, events, event, raw.clone()).await {
                    return false;
                }
                // The stream said the turn is over, so nothing after this may synthesize a second
                // `done` for it — not the EOF arm, and not a cancel's.
                if ends_the_turn {
                    state.turn_open = false;
                }
            }
            true
        }
        Line::Text(text) => {
            let event = DriverEvent::Other(OtherEvent {
                update: UNPARSED.to_owned(),
                body: json!({ "line": text }),
            });
            emit(state, events, event, None).await
        }
    }
}

/// Closes the open turn: every call still open gets its synthesized result, then exactly one
/// `done`.
///
/// A no-op when the turn is already closed, which is what makes it safe to call from the EOF arm,
/// the failed-write arm and the cancel alike — and what keeps a cancel that drained a real `result`
/// from writing a second ending for one turn.
async fn close_turn(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    stop_reason: StopReason,
) -> bool {
    if !state.turn_open {
        return true;
    }
    if stop_reason == StopReason::Cancelled {
        let open: Vec<String> = state.open_calls.clone();
        for tool_call_id in open {
            let event = DriverEvent::ToolResult(ToolResultEvent {
                tool_call_id,
                status: ToolResultStatus::Failed,
                output: None,
                locations: Vec::new(),
                terminal_reason: Some(TerminalReason::Cancelled),
            });
            if !emit(state, events, event, None).await {
                return false;
            }
        }
    }
    state.turn_open = false;
    emit(
        state,
        events,
        DriverEvent::Done(DoneEvent { stop_reason }),
        None,
    )
    .await
}

/// Writes one NDJSON line to the child's stdin and flushes it.
///
/// The flush matters: the CLI reads a line at a time, and a message sitting in a buffer is a turn
/// that never starts. A pathological prompt is bounded by this `await` on the task's own timeline
/// and never on the UI's (`R-NF-3`), and the stdout reader keeps draining throughout, so the child
/// cannot deadlock on its own output while this waits (blueprint H-14).
///
/// # Errors
/// [`DriverError::Transport`] naming the I/O failure.
async fn write_line(writer: &mut (dyn AsyncWrite + Send + Unpin), line: &str) -> Result<()> {
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|err| DriverError::Transport(format!("writing to the agent's stdin: {err}")))?;
    writer
        .flush()
        .await
        .map_err(|err| DriverError::Transport(format!("flushing the agent's stdin: {err}")))
}

/// The banner's `agent_version`, from the three sources in the order D84 ranks them.
///
/// `system/init.claude_code_version` is the key the recorded transcripts actually carry — there is
/// no `version` key on that envelope, which is the sort of thing only a fixture can settle. Then
/// the probe's own `claude --version` capture, then the empty string, which is `acp`'s fallback and
/// is a banner that says "unknown" rather than one that is missing a documented key.
fn agent_version(init: &Value, box_version: Option<&str>) -> String {
    init.get("claude_code_version")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| box_version.map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// The banner's `models`: what the stream says it is running, else what the row offers.
///
/// D84 said "the configured row"; `system/init.model` is the model this process actually selected,
/// and the stream's own answer is the more honest one — recorded as a clarification of D84 rather
/// than a reversal, with the row as the fallback it always was.
fn banner_models(init: &Value, row_models: &[String]) -> Vec<String> {
    init.get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(|model| vec![model.to_owned()])
        .unwrap_or_else(|| row_models.to_vec())
}

/// Kills the process tree and reaps it, if there is one and nobody has yet.
///
/// `ChildGuard::kill_and_reap` takes `&mut self` and awaits, so the guard is **swapped out** for an
/// empty one under the lock and awaited outside it: no lock is ever held across an `.await`, and
/// the emptied guard left behind makes a second call — and the `Drop` that eventually runs — a
/// no-op rather than a second signal at a pid the operating system may already have reissued.
async fn kill(child: &Mutex<ChildGuard>) {
    let mut taken = std::mem::replace(
        &mut *child.lock().unwrap_or_else(PoisonError::into_inner),
        ChildGuard::new(None),
    );
    taken.kill_and_reap().await;
}

/// What the child last wrote to stderr, for an error message.
fn stderr_tail(child: &Mutex<ChildGuard>) -> String {
    child
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .stderr_tail()
        .join("\n")
}

/// A failure with the child's captured stderr appended when there is any.
///
/// The CLI says why it will not run on stderr and nowhere else — "Not logged in · Please run
/// /login" is the whole of what an unauthenticated box gets — so a start failure that dropped it
/// would be the difference between a box you can fix and one that looks broken.
fn with_stderr(message: &str, child: &Mutex<ChildGuard>) -> DriverError {
    let stderr = stderr_tail(child);
    if stderr.is_empty() {
        DriverError::Transport(message.to_owned())
    } else {
        DriverError::Transport(format!("{message}\n{stderr}"))
    }
}
