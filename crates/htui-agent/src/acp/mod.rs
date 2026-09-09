//! The ACP transport (`docs/ANA-4.md` §4.2, §4.3, §4.4, §6.1; plan MOD-2 D17–D25).
//!
//! One `tokio` task per session owns the whole `Client.builder()…connect_with(…)` future, because
//! the SDK's foreground future borrows the connection and `ActiveSession::read_update` is a pull
//! API on a mutable borrow — neither can be handed around (§4.2). [`AcpSession`] is therefore a
//! handle over channels and nothing else.
//!
//! **Ordering rule, stated here and nowhere else.** Update order within a turn is the SDK's:
//! `ActiveSession::read_update` yields dispatches and the turn's stop reason from one ordered
//! channel. A `permission_request` (and an `fs/*` interception) arrives on the handler channel and
//! is emitted when the task observes it, so it is ordered only relative to the events already
//! forwarded; its correlation to a call is `tool_call_id`, never `seq` adjacency, which is what
//! `idx_session_event_tool` joins on (ANA-9 §4.3).
//!
//! **Deadlock rule, non-negotiable** (§4.2, ANA-4 risk 11): `SentRequest::block_task()` is called
//! only from the `connect_with` foreground future, never from a dispatch handler, and no handler
//! awaits a store write or file I/O — the three inbound handlers forward into an unbounded channel
//! and return.

pub mod auth;
pub mod client;
pub mod fs;
pub mod handshake;
pub mod map;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, InitializeRequest, NewSessionRequest, ReadTextFileResponse,
    RequestPermissionOutcome, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionConfigOptionValue, SetSessionConfigOptionRequest, WriteTextFileResponse,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Dispatch, SessionMessage};
use chrono::{DateTime, SubsecRound, Utc};
use htui_core::model::{Agent as AgentRow, AgentBox};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

// `run_auth`, not `run`: at this level the bare name says nothing about what is being run, and
// `handshake` beside it sets the precedent of a re-export that reads as a sentence.
pub use crate::acp::auth::{WireFlow, run as run_auth};
use crate::acp::client::{Inbound, InboundTx};
pub use crate::acp::handshake::{Handshake, handshake};
use crate::auth::{AuthFlow, AuthOutcome};
use crate::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
    PermissionRequestId, SessionSpec,
};
use crate::error::{DriverError, Result};
use crate::event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, ErrorEvent, OtherEvent,
    PermissionOptionKind, StopReason, TerminalReason, ToolResultEvent, ToolResultStatus,
};
use crate::launch::{AcpSettings, AgentLaunch, AgentSettings, ChildGuard, ResolvedLaunch, Spawned};
use crate::registry::TransportBuilder;

/// `other.update` of the row written when the agent offers no option for the requested model.
///
/// `SessionSpec.model` is advisory over ACP: §4.4 requires tolerating an agent that offers no
/// model option at all, and the step records what actually happened rather than failing.
pub const MODEL_UNAVAILABLE: &str = "model_unavailable";

/// `other.update` of the session banner (§4.4 "Session load and resume").
///
/// The agent-side session id has no column in ANA-9, so resuming a step is a query for this row.
pub const SESSION_STARTED: &str = "session_started";

/// `error.code` of the row written when the transport ends before the turn does.
pub const TRANSPORT_CLOSED: &str = "transport_closed";

/// `error.code` of a refused `fs/*` path (plan D22, [`fs::PathOutside`]).
pub const PATH_OUTSIDE_SESSION: &str = "path_outside_session";

/// Depth of the session task's event channel (plan D18).
///
/// Bounded and awaited: a slow consumer back-pressures the task, which stops reading the wire,
/// rather than losing an event. The *lossy* channel in this design is the recorder's UI copy, and
/// it is lossy on purpose (`crate::record`).
pub const EVENTS_CAPACITY: usize = 256;

/// The adapter id this transport registers under (plan D12).
pub const ADAPTER_ID: &str = "acp";

/// The grace window a session gets when its **handle** is dropped rather than cancelled.
///
/// Nobody is waiting for the rows, but the agent is still owed the `session/cancel` it would have
/// had from an explicit cancel, and a turn that ends on its own within the window ends cleanly.
pub const DROP_GRACE: Duration = Duration::from_secs(1);

/// How long the handshake may take before [`open_session`] gives up.
///
/// An agent that never answers `initialize` would otherwise hold the `ChatStart` request — and the
/// tab that issued it — forever.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------------------------

/// How [`DriverEnvelope::at`] is stamped.
///
/// A seam rather than a bare `Utc::now()`, because ANA-4 §11 criterion 2 compares replayed rows
/// byte for byte and the conformance suite compares `at`. Production stamps the wall clock; a test
/// stamps `epoch + n ms`, exactly as the fake does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// `Utc::now()` truncated to microseconds — `TIMESTAMPTZ`'s resolution, the rule the recorder
    /// already follows for the rows it authors itself.
    Wall,
    /// `epoch + n ms` for the *n*-th envelope of the session.
    Fixed {
        /// The session's zero point.
        epoch: DateTime<Utc>,
    },
}

impl Stamp {
    /// The capture time of the *n*-th envelope.
    #[must_use]
    pub fn at(self, n: u64) -> DateTime<Utc> {
        match self {
            Self::Wall => Utc::now().trunc_subsecs(6),
            Self::Fixed { epoch } => {
                epoch + chrono::TimeDelta::milliseconds(i64::try_from(n).unwrap_or(i64::MAX))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------------------------

/// The byte streams a session runs over, plus the child that owns them when there is one.
///
/// Held as `tokio` traits and adapted to `futures::io` exactly once, where the transport is built:
/// everything else in this crate speaks `tokio`.
pub struct AcpIo {
    /// The agent's stdout, as this client reads it.
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    /// The agent's stdin, as this client writes it.
    pub writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    /// `Some` when [`AcpDriver::start`] spawned the process; `None` for an in-process pair.
    pub child: Option<Spawned>,
}

impl AcpIo {
    /// The streams of a child this crate spawned, with the child carried along.
    ///
    /// Factored out of [`AcpDriver::start`]'s own spawn so the probe's tier 2 reaches the agent
    /// exactly the way a session does — one place decides what "piped stdio" means, and a probe
    /// that resolved a launch differently from the chat would be measuring the wrong box.
    ///
    /// # Errors
    /// [`DriverError::Spawn`] when stdin or stdout was not piped (already taken, or a spawn that
    /// did not request them).
    pub fn from_spawned(mut spawned: Spawned) -> Result<Self> {
        let writer = spawned
            .take_stdin()
            .ok_or_else(|| DriverError::Spawn("the agent's stdin was not piped".to_owned()))?;
        let reader = spawned
            .take_stdout()
            .ok_or_else(|| DriverError::Spawn("the agent's stdout was not piped".to_owned()))?;
        Ok(Self {
            reader: Box::new(reader.into_inner()),
            writer: Box::new(writer.into_inner()),
            child: Some(spawned),
        })
    }
}

impl core::fmt::Debug for AcpIo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AcpIo")
            .field("child", &self.child.is_some())
            .finish()
    }
}

/// Everything the session task needs besides the streams.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// `agent.name`, for the banner and the logs.
    pub agent_name: String,
    /// The parsed `agent.settings` (§5.2).
    pub settings: AgentSettings,
    /// How capture times are stamped.
    pub stamp: Stamp,
    /// How long `initialize` + `session/new` may take before [`open_session`] gives up.
    ///
    /// [`HANDSHAKE_TIMEOUT`] in production, filled in by [`AcpDriver::start`]. It is a field rather
    /// than the constant read at the point of use because the arm that matters — the one that has
    /// to kill the child it gave up on (D61) — is otherwise a minute-long test, and a minute-long
    /// test is one nobody runs.
    pub handshake_timeout: Duration,
}

// ---------------------------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------------------------

/// Where a session's byte streams come from.
enum IoSource {
    /// Spawn a child at `start`: what the probe recorded when there is a usable recording,
    /// otherwise the row's tools resolved and substituted.
    Spawn {
        /// The row's `launch` document.
        launch: Box<AgentLaunch>,
        /// `agent_box.probe.resolved`, when the snapshot passed D58's three row-side rules
        /// ([`ProbeSnapshot::recorded_launch`]). The fourth rule — the command still exists on
        /// disk — is I/O and is applied per session by [`AcpDriver::launch_for`], because a driver
        /// is built on the worker loop and a `stat` does not belong there.
        ///
        /// [`ProbeSnapshot::recorded_launch`]: crate::probe::ProbeSnapshot::recorded_launch
        recorded: Option<ResolvedLaunch>,
    },
    /// A pre-built pair, taken once — a real process starts once too.
    ///
    /// Boxed: `AcpIo` carries a `Spawned`, which is materially larger on Windows (the job-object
    /// handle), and an unboxed variant makes every `IoSource` pay for the test-support one there.
    /// Caught by `cargo clippy --target x86_64-pc-windows-msvc`, which is the only way this box
    /// compiles the Windows-side code at all.
    #[cfg(feature = "test-support")]
    Prepared(Mutex<Option<Box<AcpIo>>>),
}

/// Where a **login's** bytes come from (blueprint MOD-21 P-1).
///
/// [`IoSource`] is private to this module and cannot leave it: a login is driven from
/// `crate::auth::run`, which has to know whether it is spawning a child or was handed a pair — and
/// nothing else about how this driver was built. So the fork is answered once, here, as a value.
pub(crate) enum AuthSource {
    /// The row's launch, resolved for the login's directory. The caller owns it: whatever it
    /// writes into the environment reaches that one child and is never recorded (plan D16, H-11).
    Launch(ResolvedLaunch),
    /// A prepared pair, taken exactly as [`AcpDriver::io`] takes it. No process, so no policy to
    /// apply and no stderr to tap.
    #[cfg(feature = "test-support")]
    Prepared(AcpIo),
}

impl core::fmt::Debug for AuthSource {
    /// Which of the two, and nothing else: a [`ResolvedLaunch`] carries an environment, and the
    /// fact worth reading in a log line is which side of the fork a login is on (`R-SEC-2`).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let variant = match self {
            Self::Launch(_) => "Launch",
            #[cfg(feature = "test-support")]
            Self::Prepared(_) => "Prepared",
        };
        f.debug_tuple(variant).finish()
    }
}

/// The driver for `agent.transport = 'acp'`: one per row, holding no process.
pub struct AcpDriver {
    name: String,
    settings: AgentSettings,
    caps: DriverCaps,
    io: IoSource,
    stamp: Stamp,
}

impl core::fmt::Debug for AcpDriver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AcpDriver")
            .field("name", &self.name)
            .field("caps", &self.caps)
            .field("stamp", &self.stamp)
            // Whether, not what: a `ResolvedLaunch` carries an environment, and the fact worth
            // reading in a log line is which of D58's two paths this driver is on.
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

/// `tools::resolve` then `launch::resolve`: the pre-milestone-6 path, and D58's fallback.
///
/// Its own function rather than two lines inside [`AcpDriver::launch_for`] because it is reached
/// from three conditions there — no recording, a stale recording, and a snapshot D58 refuses — and
/// a reader should be able to see that all three land on the same code.
///
/// # Errors
/// [`DriverError::Unresolved`] naming the first tool that resolves nowhere or the first placeholder
/// with no entry; [`DriverError::Transport`] when the resolution machinery itself failed.
async fn resolve_now(launch: &AgentLaunch, cwd: &Path) -> Result<ResolvedLaunch> {
    let tools = crate::tools::resolve(launch.discovery.as_ref(), cwd).await?;
    crate::launch::resolve(launch, &tools)
}

impl AcpDriver {
    /// Builds a driver from a registry row.
    ///
    /// # Errors
    ///
    /// [`DriverError::Transport`] when `agent.launch` does not parse as the §5.1 document. An
    /// unreadable `agent.settings` is **not** fatal: it falls back to the documented defaults,
    /// exactly as `crate::registry` does, because the column is hand-editable and a session that
    /// cannot read it can still run.
    pub fn from_row(agent: &AgentRow, caps: DriverCaps) -> Result<Self> {
        Self::from_row_with_probe(agent, None, caps)
    }

    /// [`from_row`](Self::from_row) plus this box's `agent_box` row (plan D58).
    ///
    /// A usable snapshot is what `start` spawns, `--uid=` and all: the per-platform `args` a glob
    /// tool declares survive the probe and nothing else, because a `ToolMap` value is one string
    /// (blueprint H-3). What "usable" means is
    /// [`ProbeSnapshot::recorded_launch`](crate::probe::ProbeSnapshot::recorded_launch)'s three
    /// rules, plus the transport agreement below, plus a disk check
    /// [`launch_for`](Self::launch_for) applies per session.
    ///
    /// `on_box` is read here, once, rather than at every `start`: the snapshot is a document that
    /// does not change under a running driver, and parsing it on the worker loop that builds the
    /// driver keeps the parse off the session's critical path. A `probe` column that is `NULL` or
    /// does not parse is `None` and resolves exactly as before — the same tolerance
    /// [`ProbeSnapshot::from_row`](crate::probe::ProbeSnapshot::from_row) grants the Settings tab
    /// for the same hand-editable column.
    ///
    /// # Errors
    ///
    /// As [`from_row`](Self::from_row): [`DriverError::Transport`] when `agent.launch` does not
    /// parse. An unreadable `agent_box.probe` is never an error.
    pub fn from_row_with_probe(
        agent: &AgentRow,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Self> {
        let launch: AgentLaunch = serde_json::from_value(agent.launch.clone())
            .map_err(|err| DriverError::Transport(format!("agent.launch does not parse: {err}")))?;
        // The transport check is this side of `recorded_launch` rather than inside it because it
        // is the only one of D58's rules that compares the snapshot to something *outside* the
        // document — the row it was made from. `agent.transport` is hand-editable, and flipping it
        // to `acp` leaves a recording whose argv was resolved for a command-line agent: not stale,
        // wrong, and the row's own `launch` is what the edit was asking to be run.
        let recorded = on_box
            .and_then(crate::probe::ProbeSnapshot::from_row)
            .filter(|snapshot| snapshot.transport == agent.transport)
            .and_then(|snapshot| snapshot.recorded_launch().cloned());
        Ok(Self {
            name: agent.name.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Spawn {
                launch: Box::new(launch),
                recorded,
            },
            stamp: Stamp::Wall,
        })
    }

    /// What a spawn in `cwd` would launch, before launching it.
    ///
    /// The recorded launch when there is one **and** its `command` is still a file on disk;
    /// otherwise the row's tools are resolved now (`tools::resolve` → `launch::resolve`), which is
    /// the pre-milestone-6 path and carries no platform `args`. The disk check is not belt and
    /// braces: ANA-4 §4.6 records that `agy` self-updates in place, so a recorded path can name a
    /// version-numbered directory that is gone, and a chat must degrade *into* resolution rather
    /// than fail the request. A *file* and not merely an entry, because the same self-update can
    /// leave a directory where the binary used to be, and everything that is not spawnable belongs
    /// on the same side of this branch ([`crate::probe::is_file`]).
    ///
    /// It runs here, on the session's own task, for the same reason `tools::resolve` does: this is
    /// filesystem I/O, and `AgentRuntime`'s worker loop must not do any (`R-NF-3`). The command is
    /// checked exactly as recorded, so a bare name — an `HTUI_TOOL_*` override the probe took on
    /// trust — is resolved against this process's own directory and almost always falls back. That
    /// costs nothing: the fallback's first tier is that same override, so both paths answer with
    /// the same string (blueprint H-3).
    ///
    /// The environment is the row's, exactly as it resolved: a directory is all this takes,
    /// because the two callers that have more to add — a session's `spec.env`
    /// ([`launch_for`](Self::launch_for)) and a login's browser policy — add it to the value they
    /// are handed, on their own side of this call.
    ///
    /// # Errors
    /// [`DriverError::Unresolved`] and [`DriverError::Transport`] exactly as `tools::resolve` and
    /// `launch::resolve` return them. [`DriverError::Transport`] for a prepared transport, which
    /// spawns nothing and so has no launch to describe (blueprint H-16).
    pub async fn launch_in(&self, cwd: &Path) -> Result<ResolvedLaunch> {
        // A `match` rather than a `let`-else: without `test-support` there is one variant, and an
        // irrefutable `let`-else is a hard error rather than a dead branch the compiler forgives.
        let (launch, recorded) = match &self.io {
            IoSource::Spawn { launch, recorded } => (launch, recorded),
            #[cfg(feature = "test-support")]
            IoSource::Prepared(_) => {
                return Err(DriverError::Transport(
                    "a prepared transport spawns nothing".to_owned(),
                ));
            }
        };
        match recorded {
            Some(recorded) if crate::probe::is_file(Path::new(&recorded.command)).await => {
                Ok(recorded.clone())
            }
            Some(recorded) => {
                tracing::info!(
                    command = %recorded.command,
                    "the probe's recorded command is gone or is not a file; resolving again"
                );
                resolve_now(launch, cwd).await
            }
            None => resolve_now(launch, cwd).await,
        }
    }

    /// What this session would spawn, before spawning it: [`launch_in`](Self::launch_in) for the
    /// spec's `cwd`, with the spec's own environment applied over it.
    ///
    /// `spec.env` is applied **last** and wins (`R-SEC-2`): the row's environment holds paths, the
    /// spec's holds what the secret provider produced for this run.
    ///
    /// # Errors
    /// [`launch_in`](Self::launch_in)'s, unchanged.
    pub async fn launch_for(&self, spec: &SessionSpec) -> Result<ResolvedLaunch> {
        let mut resolved = self.launch_in(&spec.cwd).await?;
        resolved.env.extend(spec.env.clone());
        Ok(resolved)
    }

    /// Where a login's bytes come from, having spawned nothing (blueprint MOD-21 P-1).
    ///
    /// The one thing outside this module that needs to know [`IoSource`] exists, and it learns no
    /// more than which of the two it got. A session reaches the same fork through
    /// [`io`](Self::io), which spawns on the spot; a login cannot, because between resolving the
    /// launch and spawning it there is a browser policy to write into the value and a stderr tap
    /// to take off the child, and both belong to `auth::run` rather than here (plan D9, D16).
    ///
    /// # Errors
    /// [`launch_in`](Self::launch_in)'s for a row; [`DriverError::Transport`] for a prepared
    /// transport already used, or one whose slot a panic poisoned.
    pub(crate) async fn auth_source(&self, cwd: &Path) -> Result<AuthSource> {
        match &self.io {
            IoSource::Spawn { .. } => Ok(AuthSource::Launch(self.launch_in(cwd).await?)),
            #[cfg(feature = "test-support")]
            IoSource::Prepared(slot) => slot
                .lock()
                .map_err(|_| {
                    DriverError::Transport("the prepared transport is poisoned".to_owned())
                })?
                .take()
                .map(|io| AuthSource::Prepared(*io))
                .ok_or_else(|| {
                    DriverError::Transport("this driver's transport was already used".to_owned())
                }),
        }
    }

    /// This row's ACP settings, for a caller that has to speak the wire on its own.
    ///
    /// `auth::run` builds a [`WireFlow`] rather than a [`SessionOptions`], and the one thing the
    /// login's handshake reads out of the row is `client_capabilities` (plan D21).
    pub(crate) fn acp_settings(&self) -> &AcpSettings {
        &self.settings.acp
    }

    /// A driver over an in-process transport with a deterministic clock: the conformance harness.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn over(io: AcpIo, agent: &AgentRow, caps: DriverCaps, stamp: Stamp) -> Self {
        Self {
            name: agent.name.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Prepared(Mutex::new(Some(Box::new(io)))),
            stamp,
        }
    }

    /// The streams this session runs over.
    async fn io(&self, spec: &SessionSpec) -> Result<AcpIo> {
        match &self.io {
            IoSource::Spawn { .. } => {
                let resolved = self.launch_for(spec).await?;
                let spawned = crate::launch::spawn(&resolved, &spec.cwd).await?;
                AcpIo::from_spawned(spawned)
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

impl AgentDriver for AcpDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn caps(&self) -> DriverCaps {
        self.caps
    }

    fn start<'a>(
        &'a self,
        spec: SessionSpec,
        prompt: String,
    ) -> DriverFuture<'a, Box<dyn AgentSession>> {
        Box::pin(async move {
            let io = self.io(&spec).await?;
            let options = SessionOptions {
                agent_name: self.name.clone(),
                settings: self.settings.clone(),
                stamp: self.stamp,
                handshake_timeout: HANDSHAKE_TIMEOUT,
            };
            let session = open_session(io, spec, prompt, options).await?;
            Ok(Box::new(session) as Box<dyn AgentSession>)
        })
    }

    // The protocol carries the call, so this transport answers it (plan MOD-21 D10): `caps_from`
    // says `authenticate: true` for every `acp` row, and the contract test holds the two to each
    // other. The operation itself is `auth::run`'s, because everything between resolving the launch
    // and reading the outcome — the browser policy, the stderr tap, the idle clock — is
    // transport-neutral and belongs beside the types the caller sees (D9).
    fn authenticate<'a>(&'a self, flow: AuthFlow) -> DriverFuture<'a, AuthOutcome> {
        Box::pin(crate::auth::run::authenticate(self, flow))
    }
}

/// The [`TransportBuilder`] registered under [`ADAPTER_ID`].
#[derive(Debug, Default, Clone, Copy)]
pub struct AcpAdapter;

impl TransportBuilder for AcpAdapter {
    fn build(
        &self,
        agent: &AgentRow,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>> {
        Ok(Box::new(AcpDriver::from_row_with_probe(
            agent, on_box, caps,
        )?))
    }
}

// ---------------------------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------------------------

/// What the handle asks the session task to do.
#[derive(Debug)]
pub enum SessionCommand {
    /// Start a new turn with this text.
    FollowUp(String),
    /// Answer a parked permission request.
    AnswerPermission(PermissionRequestId, PermissionAnswer),
    /// End the session. Acknowledged **after** the process tree is gone, so a caller that awaited
    /// `cancel` knows there is nothing left running.
    Cancel {
        /// How long the graceful path may take before the tree is killed.
        grace: Duration,
        /// Answered last.
        done: oneshot::Sender<()>,
    },
}

/// A live ACP session: channel endpoints and handle-side bookkeeping only.
pub struct AcpSession {
    session_ref: AgentSessionRef,
    events: mpsc::Receiver<DriverEnvelope>,
    commands: mpsc::UnboundedSender<SessionCommand>,
    /// Envelopes drained while `cancel` waited for its acknowledgement; served before `events`.
    pending: VecDeque<DriverEnvelope>,
    /// Requests handed out and not yet answered. Non-empty means `next_event` refuses: the caller
    /// owes the agent an answer, and pulling past a parked request is what would stall the turn.
    parked: Vec<PermissionRequestId>,
    /// `false` between a handed-out `done` and the next accepted follow-up.
    turn_open: bool,
    /// The session is being cancelled: the task has answered every outstanding request itself, so
    /// a `permission_request` still in the buffer is history rather than an obligation. Without
    /// this, draining a cancel re-parks it and the very next pull refuses — leaving the
    /// synthesized results and the `done { cancelled }` unrecorded.
    cancelling: bool,
    /// The task has ended: `next_event` answers `Ok(None)`, everything else [`DriverError::Closed`].
    ended: bool,
    task: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for AcpSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AcpSession")
            .field("session_ref", &self.session_ref)
            .field("pending", &self.pending.len())
            .field("parked", &self.parked.len())
            .field("turn_open", &self.turn_open)
            .field("ended", &self.ended)
            .finish()
    }
}

impl AgentSession for AcpSession {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        Some(&self.session_ref)
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            if let Some(envelope) = self.pending.pop_front() {
                self.note(&envelope);
                return Ok(Some(envelope));
            }
            if self.ended {
                return Ok(None);
            }
            if let Some(parked) = self.parked.first() {
                return Err(DriverError::Transport(format!(
                    "permission request `{parked}` is parked: answer or cancel it before pulling again"
                )));
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
            self.send(SessionCommand::FollowUp(text))?;
            self.turn_open = true;
            Ok(())
        })
    }

    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Err(DriverError::Closed);
            }
            let Some(index) = self.parked.iter().position(|id| id == &request_id) else {
                return Err(DriverError::Transport(format!(
                    "no parked permission request `{request_id}`"
                )));
            };
            self.parked.remove(index);
            self.send(SessionCommand::AnswerPermission(request_id, answer))
        })
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Ok(());
            }
            self.cancelling = true;
            let (done, mut ack) = oneshot::channel();
            if self.send(SessionCommand::Cancel { grace, done }).is_err() {
                self.ended = true;
                return Ok(());
            }
            // Keep draining while the task shuts down: the synthesized results and the
            // `done { cancelled }` are ordinary events, and dropping them here would lose rows the
            // recorder still has to write.
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
            self.parked.clear();
            // **Not** `ended = true`: the cancel's own rows — the synthesized `tool_result`s and
            // the `done { cancelled }` — are ordinary events, and some of them may still be in the
            // channel when the acknowledgement wins the `select!` above. The caller pulls them
            // exactly as it pulls any other event, and the session ends when the channel does.
            // The task acknowledges *after* it has killed the process tree, and it returns
            // immediately afterwards; joining it here is what makes "cancel returned" mean "the
            // tree is gone and nothing of this session is still running" — which is exactly what
            // ANA-4 §11 criterion 11 measures.
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

impl AcpSession {
    /// Handle-side bookkeeping, applied as an envelope is handed out rather than as it arrives:
    /// what the caller has *seen* is what decides whether a follow-up or a pull is legal.
    fn note(&mut self, envelope: &DriverEnvelope) {
        match &envelope.event {
            DriverEvent::PermissionRequest(request) if !self.cancelling => {
                self.parked.push(request.request_id.clone());
            }
            DriverEvent::Done(_) => self.turn_open = false,
            _ => {}
        }
    }

    /// Sends a command, turning a dead task into [`DriverError::Closed`].
    fn send(&mut self, command: SessionCommand) -> Result<()> {
        if self.commands.send(command).is_err() {
            self.ended = true;
            return Err(DriverError::Closed);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------------------------

/// What the task reports once the session is open and the first prompt is on the wire.
struct Ready {
    session_ref: AgentSessionRef,
}

/// The handshake's one reply, and how far the handshake had got when it was owed.
///
/// **Shared rather than owned by the foreground future, and that is the whole point.**
/// `SessionBuilder::start_session` does not send `session/new` from the future that awaits it: it
/// issues the request from a task it spawns *on the connection* (`SDK/session.rs:885-905`). A
/// JSON-RPC error there fails a connection actor, and `run_until_connection_close` does
/// `background_result?` while still holding the foreground (`SDK/jsonrpc.rs:3556-3560`) — so the
/// foreground is **dropped**, [`session_main`]'s `session/new` error arm never runs, and a sender
/// owned by that future would be dropped with it. The caller then saw "nobody ever answered" for
/// what was in fact a perfectly clear refusal; on an unauthenticated box that refusal is the one
/// message worth reading. Holding the sender here lets [`run_session`] — which outlives the
/// foreground, because it owns it — answer on its behalf out of the connection's own error.
///
/// `step` is what makes that answer name the right thing. `initialize` travels on the foreground
/// and its errors come back to their own arm, so a connection error while `step` is still
/// `initialize` is a transport that died mid-handshake, not a refusal — and saying "session/new"
/// there would be pointing at a request that was never sent.
struct ReadyCell {
    /// `None` once somebody has answered: the handshake is answered exactly once, and a second
    /// answer would be a second `start` outcome for one call.
    sender: Option<oneshot::Sender<Result<Ready>>>,
    step: &'static str,
}

impl ReadyCell {
    /// A cell over `sender`, on the first step of the handshake.
    fn new(sender: oneshot::Sender<Result<Ready>>) -> Self {
        Self {
            sender: Some(sender),
            step: "initialize",
        }
    }
}

/// Answers the handshake, if nobody has yet.
///
/// `false` when there was nobody left to tell — either the handshake is already answered or
/// [`open_session`] has stopped listening — which is [`session_main`]'s cue that its handle will
/// never exist and there is nothing left to run for.
fn answer(ready: &Mutex<ReadyCell>, result: Result<Ready>) -> bool {
    let sender = ready
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .sender
        .take();
    sender.is_some_and(|sender| sender.send(result).is_ok())
}

/// Records the handshake step the foreground future is on, for an answer it may not get to make.
fn at_step(ready: &Mutex<ReadyCell>, step: &'static str) {
    ready.lock().unwrap_or_else(PoisonError::into_inner).step = step;
}

/// Answers an unanswered handshake out of the **connection future's** error, and says whether it
/// did.
///
/// This is the arm the vendor's `Authentication required` comes home on: the request that earned
/// it was sent from a connection actor, and an actor that fails first takes the foreground future
/// down with it before its own error arm can run ([`ReadyCell`]). The step is the one the
/// foreground had reached, so the message names the request that was actually refused, and the
/// child's stderr is appended exactly as the foreground's own arms append it — the guard still
/// holds the child here, because [`run_session`] kills only after this has read it.
fn answer_from_connection(
    ready: &Mutex<ReadyCell>,
    err: &agent_client_protocol::Error,
    child: &Mutex<ChildGuard>,
) -> bool {
    let (sender, step) = {
        let mut cell = ready.lock().unwrap_or_else(PoisonError::into_inner);
        (cell.sender.take(), cell.step)
    };
    let Some(sender) = sender else {
        return false;
    };
    sender.send(Err(handshake_error(step, err, child))).is_ok()
}

/// Opens a session: spawns the task, waits for the handshake, returns the handle.
///
/// # Errors
///
/// [`DriverError::Transport`] carrying whatever the handshake failed with, with the child's
/// captured stderr appended when there was a child.
///
/// **Three failing exits, and what each guarantees about the child** (D61). Every one of them
/// returns with the kill already sent, so a caller reporting the error is reporting the whole
/// outcome rather than leaving an adapter on the box for it to wonder about — but they differ in
/// how far past the signal they get, and the difference is worth naming because only one of them
/// is a compromise:
///
/// 1. **The agent never answered.** The task is aborted and awaited, which resolves as soon as the
///    runtime has dropped the future; the `ChildGuard`'s `Drop` has then signalled the tree.
///    Signalled but not *reaped* by us, because a `Drop` cannot await — the only exit that runs no
///    code of ours, and the only one that leaves the reap to `tokio::process`'s orphan queue
///    (blueprint H-1, and the arm's own comment for why that is enough).
/// 2. **The agent answered, and its answer was no.** The failure is composed on the task's own
///    timeline — by the foreground future for `initialize`, by [`answer_from_connection`] for the
///    `session/new` a connection actor refused — and the task is awaited afterwards, so the tree
///    is killed *and* reaped before this returns.
/// 3. **Nobody answered at all**, the sender dropped with the task: the handshake got as far as an
///    event the consumer was no longer there to receive. Awaited exactly as (2) is, because the
///    task is on its way to its own `kill` and returning before it arrives would make this the one
///    error that does not mean what the other two mean.
///
/// (2) and (3) bound the wait by the handshake timeout: a task that will not finish must not turn
/// a failed `start` into a hang.
pub async fn open_session(
    io: AcpIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
) -> Result<AcpSession> {
    let (events_tx, events_rx) = mpsc::channel(EVENTS_CAPACITY);
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();

    let timeout = options.handshake_timeout;
    let task = tokio::spawn(run_session(
        io,
        spec,
        prompt,
        options,
        Arc::new(Mutex::new(ReadyCell::new(ready_tx))),
        events_tx,
        commands_rx,
    ));

    match tokio::time::timeout(timeout, ready_rx).await {
        Err(_) => {
            task.abort();
            // D61. The aborted task drops its `ChildGuard`, whose `Drop` signals the kill;
            // awaiting the cancelled handle is **not** waiting for the adapter — it resolves as
            // soon as the runtime has dropped the future — and it is what makes this `Err` mean
            // "the kill has been sent" rather than "the kill will be sent shortly". A caller that
            // saw the error and went looking for the process would otherwise be racing the
            // scheduler.
            //
            // The reap is skipped — a `Drop` cannot await — and skipping it costs less than the
            // comment here used to claim. `tokio::process`'s own `Drop` (1.53's
            // `process/unix/reap.rs` `Reaper::drop`, and `pidfd_reaper.rs` for the pidfd path)
            // `try_wait`s the child and, failing that, pushes it onto the **global orphan queue**;
            // the process driver drains that queue on every park once `SIGCHLD` has arrived
            // (`runtime/process.rs:33`). So the exited child is reaped while the runtime is still
            // alive, not held until this process exits, and on Windows there is no zombie state
            // for it to be held in at all. Nobody should build the reaper task blueprint H-1
            // speculates about: tokio already is one.
            let _ = task.await;
            Err(DriverError::Transport(format!(
                "the agent did not complete its handshake within {}s",
                timeout.as_secs()
            )))
        }
        Ok(Ok(Ok(ready))) => Ok(AcpSession {
            session_ref: ready.session_ref,
            events: events_rx,
            commands: commands_tx,
            pending: VecDeque::new(),
            parked: Vec::new(),
            turn_open: true,
            cancelling: false,
            ended: false,
            task: Some(task),
        }),
        // The task is killing the child on its way out; waiting for it means `start` returning an
        // error also means the process is gone, rather than leaving one behind for the caller to
        // wonder about.
        Ok(Ok(Err(err))) => {
            let _ = tokio::time::timeout(timeout, task).await;
            Err(err)
        }
        // Exit (3), and now genuinely the fallback its message claims to be: since the sender is
        // shared, a refusal the foreground future never got to report is reported by `run_session`
        // instead, and reaching here means nothing on the task's side had an answer to give. It
        // waits for the same reason exit (2) does — the task is between "the handshake is over"
        // and `kill`, and returning first would make this the one error that leaves a process
        // behind.
        Ok(Err(_)) => {
            let _ = tokio::time::timeout(timeout, task).await;
            Err(DriverError::Transport(
                "the session task ended before the handshake".to_owned(),
            ))
        }
    }
}

/// The session task: owns the child, the connection, the parked responders and the mapper.
async fn run_session(
    io: AcpIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    ready: Arc<Mutex<ReadyCell>>,
    events: mpsc::Sender<DriverEnvelope>,
    commands: mpsc::UnboundedReceiver<SessionCommand>,
) {
    let AcpIo {
        reader,
        writer,
        child,
    } = io;
    let transport = ByteStreams::new(writer.compat_write(), reader.compat());

    // **The child is owned here, not by the foreground future.** `connect_with` runs the
    // connection actors and the foreground future under `future::select`, and an actor that fails
    // first returns early and **drops** the foreground future (`SDK/jsonrpc.rs:3555-3560`) — which
    // is exactly what a JSON-RPC error answering `session/prompt` causes. A `session_main` that
    // owned the process would be dropped mid-await and leave it running, so ownership sits on this
    // side of that boundary and the kill below happens whatever became of the future. It is a
    // `ChildGuard` and not a bare `Option<Spawned>` because ownership alone is not enough for the
    // one exit that runs no code of ours: an **aborted** task drops the guard, whose `Drop`
    // signals the kill (D61), which is what `open_session`'s handshake timeout relies on.
    let child = Arc::new(Mutex::new(ChildGuard::new(child)));

    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<Inbound>();
    let permission_tx: InboundTx = inbound_tx.clone();
    let read_tx: InboundTx = inbound_tx.clone();
    let write_tx: InboundTx = inbound_tx;

    let connected = Client
        .builder()
        .name("htui")
        .on_receive_request(
            async move |request, responder, _connection| {
                // ANA-4 §4.2 deadlock rule: forward and return. No await, no `block_task`.
                client::forward_permission(&permission_tx, request, responder);
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request, responder, _connection| {
                client::forward_read(&read_tx, request, responder);
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request, responder, _connection| {
                client::forward_write(&write_tx, request, responder);
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, {
            let child = Arc::clone(&child);
            let ready = Arc::clone(&ready);
            async move |cx: ConnectionTo<Agent>| {
                session_main(
                    cx, spec, prompt, options, &ready, events, commands, inbound_rx, &child,
                )
                .await;
                Ok(())
            }
        })
        .await;

    if let Err(err) = connected {
        // **Before the kill, and this is the whole of the headline fix.** A `session/new` the
        // agent refused fails a connection actor, which drops the foreground future before its own
        // error arm runs ([`ReadyCell`]); the error surfaces *here* instead, as the connection
        // future's value. Until this line it died in the `warn!` below and the caller was told
        // "the session task ended before the handshake" — the fallback for a handshake nobody
        // answered — for what an unauthenticated box answers very clearly indeed. Reading the
        // stderr tail also has to happen before `kill`, while the guard still holds the child.
        if answer_from_connection(&ready, &err, &child) {
            tracing::debug!(%err, "the ACP connection ended with the error `start` reported");
        } else {
            // Already answered, or nobody is listening: the log is the only reader left.
            tracing::warn!(%err, "the ACP connection ended with an error");
        }
    }
    // Unconditional: the foreground future may never have reached its own kill.
    kill(&child).await;
}

/// Task-side state, one per session.
struct TaskState {
    stamp: Stamp,
    n: u64,
    retain_raw: bool,
    mapper: map::Mapper,
    parked: HashMap<PermissionRequestId, ParkedRequest>,
    open_calls: Vec<String>,
    settled_calls: BTreeSet<String>,
    turn_open: bool,
}

/// A permission request waiting for an answer.
struct ParkedRequest {
    responder: agent_client_protocol::Responder<RequestPermissionResponse>,
    /// The options the agent offered, so a rejection can be recognised from the chosen id alone.
    options: Vec<crate::event::PermissionOption>,
    /// The call being gated, for the synthesized `tool_result` of a rejection.
    tool_call_id: Option<String>,
}

impl TaskState {
    fn new(stamp: Stamp, retain_raw: bool) -> Self {
        Self {
            stamp,
            n: 0,
            retain_raw,
            mapper: map::Mapper::new(),
            parked: HashMap::new(),
            open_calls: Vec::new(),
            settled_calls: BTreeSet::new(),
            turn_open: true,
        }
    }

    /// Wraps an event in its envelope, stamping the capture time.
    ///
    /// With `retain_raw` set, a row that has **no** wire message still carries a `raw` naming what
    /// produced it: the banner, the synthesized `tool_result` of a rejection or a cancel, and
    /// every `error` `htui` authored itself are all rows a replay has to explain, and ANA-4 §11
    /// criterion 4 asks that `keep_raw_events` populate `raw` on every row the driver authored,
    /// not only on the ones that happen to have arrived as a notification.
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
async fn emit(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    mut event: DriverEvent,
    raw: Option<Value>,
) -> bool {
    // §4.3's rule for `edit_proposal.accepted`, which the mapper cannot apply because it is a fact
    // about *this session* rather than about the update: a proposal whose call has already reached
    // a terminal state is an edit the agent was allowed to make, and the row says so.
    //
    // **Only** a settled call, never merely an un-gated one. The `claude` adapter sends the
    // `tool_call` carrying the diff *before* the `session/request_permission` that gates it, so
    // "no request is parked yet" is the normal state of an edit that is about to be asked about —
    // defaulting to `true` there would record almost every gated edit as accepted before the user
    // had seen it. `null` until the call settles is §4.3's own rule ("stays null only while a
    // request is parked"), read the safe way round. A rejection answered *after* the row was
    // flushed leaves it `null`, because the recorder has no update path for a flushed row; replay
    // stays total, since the rejection also synthesizes a `tool_result { failed, rejected }`.
    if let DriverEvent::EditProposal(proposal) = &mut event
        && proposal.accepted.is_none()
        && let Some(call) = proposal.tool_call_id.as_ref()
        && state.settled_calls.contains(call)
    {
        proposal.accepted = Some(true);
    }
    // A `tool_result` for a call a rejection or a cancel already settled is the protocol's own
    // late report: `htui` wrote the synthesized row, and a second one would be two results for one
    // call (§4.3 "Tool-call terminal states").
    if let DriverEvent::ToolResult(result) = &event
        && state.settled_calls.contains(&result.tool_call_id)
    {
        return true;
    }
    state.note(&event);
    let envelope = state.envelope(event, raw);
    events.send(envelope).await.is_ok()
}

/// One iteration's cause.
enum Step {
    Update(std::result::Result<SessionMessage, agent_client_protocol::Error>),
    Command(Option<SessionCommand>),
    Inbound(Option<Inbound>),
    Closed,
}

/// The foreground future: handshake, banner, first prompt, then the turn loop.
#[expect(
    clippy::too_many_arguments,
    reason = "the task owns nine distinct things; bundling them into a struct renames the arity \
              without reducing it"
)]
async fn session_main(
    cx: ConnectionTo<Agent>,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    ready: &Mutex<ReadyCell>,
    events: mpsc::Sender<DriverEnvelope>,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    mut inbound: mpsc::UnboundedReceiver<Inbound>,
    child: &Mutex<ChildGuard>,
) {
    let mut state = TaskState::new(options.stamp, spec.retain_raw);

    // 1. initialize. ANA-4 risk 11: `block_task` only here, never in a dispatch handler.
    let initialize = InitializeRequest::new(ProtocolVersion::V1)
        .client_capabilities(client::client_capabilities(
            &options.settings.acp.client_capabilities,
        ))
        .client_info(client::client_info());
    let init = match cx.send_request(initialize).block_task().await {
        Ok(response) => response,
        Err(err) => {
            answer(ready, Err(handshake_error("initialize", &err, child)));
            kill(child).await;
            return;
        }
    };

    // 2. session/new. ANA-4 risk 11: `block_task` only here.
    //
    // The step is recorded before the request goes out because this arm is the one that may not
    // run: `start_session` sends from a connection actor, and a refusal drops this future rather
    // than returning to it ([`ReadyCell`]). `run_session` then answers from the connection's own
    // error, and what it reads here is how it knows to call the failure `session/new`.
    at_step(ready, "session/new");
    let new_session =
        NewSessionRequest::new(spec.cwd.clone()).additional_directories(spec.extra_dirs.clone());
    let mut session = match cx
        .build_session_from(new_session)
        .block_task()
        .start_session()
        .await
    {
        Ok(session) => session,
        // Still reachable, and not dead code: a local `ensure_v1_session_protocol` refusal and an
        // internal error both return here without ever failing an actor.
        Err(err) => {
            answer(ready, Err(handshake_error("session/new", &err, child)));
            kill(child).await;
            return;
        }
    };
    let session_id = session.session_id().clone();
    let session_ref = AgentSessionRef::new(session_id.0.to_string());

    // 3. The banner is the session's first row (§4.4): resuming a step is a query for it.
    let models = model_values(&session, options.settings.acp.model_config_id.as_deref());
    let banner = DriverEvent::Other(OtherEvent {
        update: SESSION_STARTED.to_owned(),
        body: json!({
            "session_id": session_ref.as_str(),
            "protocol_version": 1,
            "agent_name": init
                .agent_info
                .as_ref()
                .map_or(options.agent_name.as_str(), |info| info.name.as_str()),
            "agent_version": init
                .agent_info
                .as_ref()
                .map_or("", |info| info.version.as_str()),
            "models": models,
        }),
    });
    let raw = serde_json::to_value(&init).ok();
    if !emit(&mut state, &events, banner, raw).await {
        kill(child).await;
        return;
    }

    // 4. The model, by option **id** (§3: categories are UX-only and MUST NOT decide correctness).
    if let Some(model) = spec.model.clone() {
        match model_option(
            &session,
            options.settings.acp.model_config_id.as_deref(),
            &model,
        ) {
            Some(config_id) => {
                // ANA-4 risk 11: `block_task` only here.
                let request = SetSessionConfigOptionRequest::new(
                    session_id.clone(),
                    config_id,
                    SessionConfigOptionValue::value_id(model.clone()),
                );
                if let Err(err) = cx.send_request(request).block_task().await {
                    tracing::warn!(%err, model = %model, "the agent refused the model selection");
                }
            }
            None => {
                let unavailable = DriverEvent::Other(OtherEvent {
                    update: MODEL_UNAVAILABLE.to_owned(),
                    body: json!({ "requested": model }),
                });
                if !emit(&mut state, &events, unavailable, None).await {
                    kill(child).await;
                    return;
                }
            }
        }
    }

    // 5. The handle may exist now: everything above is what `start` promised to have done. This is
    //    also where the sender leaves the cell on the happy path, so a connection error after it —
    //    a turn that dies on the wire — is the session's business and not `start`'s.
    if !answer(ready, Ok(Ready { session_ref })) {
        kill(child).await;
        return;
    }

    // 6. The first prompt opens turn 0. A send that fails still owes the turn a `done`: the
    //    handle has already been told the turn is open, and a caller pulling for one would wait
    //    forever.
    if let Err(err) = session.send_prompt(prompt) {
        let event = DriverEvent::Error(ErrorEvent {
            code: TRANSPORT_CLOSED.to_owned(),
            message: err.to_string(),
        });
        emit(&mut state, &events, event, None).await;
        close_turn(&mut state, &events, StopReason::Cancelled).await;
        kill(child).await;
        return;
    }

    // 7. The turn loop. Every branch resolves to a `Step` first so no borrow of `session` outlives
    //    the `select!` that produced it.
    loop {
        let step = tokio::select! {
            message = session.read_update() => Step::Update(message),
            command = commands.recv() => Step::Command(command),
            request = inbound.recv() => Step::Inbound(request),
            () = cx.incoming_closed() => Step::Closed,
        };

        match step {
            Step::Update(Ok(message)) => {
                if !on_message(&mut state, &events, message).await {
                    break;
                }
            }
            Step::Update(Err(err)) => {
                let event = DriverEvent::Error(ErrorEvent {
                    code: TRANSPORT_CLOSED.to_owned(),
                    message: err.to_string(),
                });
                emit(&mut state, &events, event, None).await;
                close_turn(&mut state, &events, StopReason::Cancelled).await;
                break;
            }
            Step::Command(Some(SessionCommand::FollowUp(text))) => {
                // Open the turn **before** the send: the handle already counts it as open, and a
                // failure has to be able to close it (`close_turn` is a no-op on a closed turn).
                state.turn_open = true;
                if let Err(err) = session.send_prompt(text) {
                    let event = DriverEvent::Error(ErrorEvent {
                        code: TRANSPORT_CLOSED.to_owned(),
                        message: err.to_string(),
                    });
                    emit(&mut state, &events, event, None).await;
                    close_turn(&mut state, &events, StopReason::Cancelled).await;
                    break;
                }
            }
            Step::Command(Some(SessionCommand::AnswerPermission(id, answer))) => {
                if !answer_permission(&mut state, &events, id, answer).await {
                    break;
                }
            }
            Step::Command(Some(SessionCommand::Cancel { grace, done })) => {
                cancel_session(&mut state, &events, &cx, &mut session, &session_id, grace).await;
                kill(child).await;
                let _ = done.send(());
                return;
            }
            // The handle is gone: nobody is reading, so end the session rather than leave a child
            // running for an audience that left.
            Step::Command(None) => {
                cancel_session(
                    &mut state,
                    &events,
                    &cx,
                    &mut session,
                    &session_id,
                    DROP_GRACE,
                )
                .await;
                break;
            }
            Step::Inbound(Some(request)) => {
                if !on_inbound(&mut state, &events, &spec, request).await {
                    break;
                }
            }
            // The handlers own the senders for the connection's life, so this is unreachable
            // today; treating it as a close rather than as a no-op keeps it from becoming a busy
            // loop if that ever changes.
            Step::Inbound(None) => break,
            Step::Closed => {
                let message = stderr_tail(child);
                let event = DriverEvent::Error(ErrorEvent {
                    code: TRANSPORT_CLOSED.to_owned(),
                    message,
                });
                emit(&mut state, &events, event, None).await;
                // They can no longer reach the wire, but the map must empty: a responder dropped
                // without an answer is what the SDK turns into an error response.
                answer_parked_cancelled(&mut state);
                close_turn(&mut state, &events, StopReason::Cancelled).await;
                break;
            }
        }
    }

    kill(child).await;
}

/// Handles one ordered message from the session channel.
async fn on_message(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    message: SessionMessage,
) -> bool {
    match message {
        SessionMessage::SessionMessage(Dispatch::Notification(notification)) => {
            if notification.method() != "session/update" {
                let event = DriverEvent::Other(OtherEvent {
                    update: notification.method().to_owned(),
                    body: notification.params.clone(),
                });
                return emit(state, events, event, None).await;
            }
            let update = notification
                .params
                .get("update")
                .cloned()
                .unwrap_or(Value::Null);
            let raw = json!({ "method": "session/update", "params": notification.params });
            for event in state.mapper.map(&update) {
                if !emit(state, events, event, Some(raw.clone())).await {
                    return false;
                }
            }
            true
        }
        // Defensive: the three requests this client serves are claimed by the builder handlers, so
        // anything reaching here is one the agent invented.
        SessionMessage::SessionMessage(Dispatch::Request(request, responder)) => {
            let _ = responder.respond_with_error(agent_client_protocol::Error::method_not_found());
            let event = DriverEvent::Other(OtherEvent {
                update: "unhandled_request".to_owned(),
                body: json!({ "method": request.method() }),
            });
            emit(state, events, event, None).await
        }
        SessionMessage::SessionMessage(Dispatch::Response(..)) => true,
        SessionMessage::StopReason(reason) => {
            let stop = map::stop_reason(&stop_reason_text(&reason));
            close_turn(state, events, stop).await
        }
        // `SessionMessage` is `#[non_exhaustive]`: a variant this build does not know is still an
        // event that happened, and §6.1's wildcard row is where it goes.
        other => {
            let event = DriverEvent::Other(OtherEvent {
                update: "unknown_session_message".to_owned(),
                body: json!({ "debug": format!("{other:?}") }),
            });
            emit(state, events, event, None).await
        }
    }
}

/// The wire text of a `StopReason`, through serde rather than a match on the SDK's enum.
fn stop_reason_text(reason: &agent_client_protocol::schema::v1::StopReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "end_turn".to_owned())
}

/// Closes the open turn: every call still open gets its synthesized result, then exactly one
/// `done`.
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
    // The turn's `done` comes from the `session/prompt` response, not from a notification, so its
    // `raw` is that response as the SDK delivered it.
    let raw = json!({
        "method": "session/prompt",
        "result": { "stopReason": stop_reason.as_str() },
    });
    emit(
        state,
        events,
        DriverEvent::Done(DoneEvent { stop_reason }),
        Some(raw),
    )
    .await
}

/// Answers a parked request, synthesizing the rejected call's result when the choice was a reject.
async fn answer_permission(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    id: PermissionRequestId,
    answer: PermissionAnswer,
) -> bool {
    let Some(parked) = state.parked.remove(&id) else {
        tracing::warn!(request = %id, "an answer arrived for a request that is not parked");
        return true;
    };
    // §4.3 distinguishes the two terminal reasons: a call the user said no to is `rejected`, a
    // call that ended because the session did is `cancelled`.
    let terminal = match &answer {
        PermissionAnswer::Selected(option_id) => parked
            .options
            .iter()
            .find(|option| &option.id == option_id)
            .is_some_and(|option| {
                matches!(
                    option.kind,
                    PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                )
            })
            .then_some(TerminalReason::Rejected),
        PermissionAnswer::Cancelled => Some(TerminalReason::Cancelled),
    };
    let outcome = match answer {
        PermissionAnswer::Selected(option_id) => {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
        }
        PermissionAnswer::Cancelled => RequestPermissionOutcome::Cancelled,
    };
    if let Err(err) = parked
        .responder
        .respond(RequestPermissionResponse::new(outcome))
    {
        tracing::warn!(%err, "the permission answer could not be sent");
    }
    // §4.3 "Tool-call terminal states": a denied call gets no `tool_result` on the wire, and a
    // replay that never sees one leaves the call spinning forever.
    if let Some(reason) = terminal
        && let Some(tool_call_id) = parked.tool_call_id
    {
        let event = DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id,
            status: ToolResultStatus::Failed,
            output: None,
            locations: Vec::new(),
            terminal_reason: Some(reason),
        });
        return emit(state, events, event, None).await;
    }
    true
}

/// Handles one inbound request the builder handlers forwarded.
async fn on_inbound(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    spec: &SessionSpec,
    request: Inbound,
) -> bool {
    match request {
        Inbound::Permission(request, responder) => {
            let id = client::request_id(responder.id());
            let event = client::permission_event(&request, id.clone());
            let raw = json!({
                "method": "session/request_permission",
                "params": serde_json::to_value(&*request).unwrap_or(Value::Null),
            });
            state.parked.insert(
                id,
                ParkedRequest {
                    responder,
                    options: event.options.clone(),
                    tool_call_id: event.tool_call_id.clone(),
                },
            );
            emit(
                state,
                events,
                DriverEvent::PermissionRequest(event),
                Some(raw),
            )
            .await
        }
        Inbound::ReadFile(request, responder) => {
            match fs::guard(&request.path, &spec.cwd, &spec.extra_dirs) {
                Ok(path) => match fs::read_current(&path).await {
                    Ok(text) => {
                        let window = fs::slice_lines(&text, request.line, request.limit);
                        let _ = responder.respond(ReadTextFileResponse::new(window));
                        true
                    }
                    Err(err) => {
                        let _ = responder
                            .respond_with_error(agent_client_protocol::Error::internal_error());
                        let event = DriverEvent::Error(ErrorEvent {
                            code: "read_failed".to_owned(),
                            message: err.to_string(),
                        });
                        emit(state, events, event, None).await
                    }
                },
                Err(outside) => {
                    let _ = responder
                        .respond_with_error(agent_client_protocol::Error::invalid_params());
                    refused(state, events, &outside).await
                }
            }
        }
        Inbound::WriteFile(request, responder) => {
            match fs::guard(&request.path, &spec.cwd, &spec.extra_dirs) {
                Ok(path) => {
                    // §4.3's adopted option: read, synthesize the diff, record it, then write.
                    let old = fs::read_current(&path).await.unwrap_or_default();
                    let display = path.to_string_lossy().into_owned();
                    let event = DriverEvent::EditProposal(EditProposalEvent {
                        // This write arrives outside any tool call the client can see, so the
                        // recorder's dedup key for it is `(None, path)` (§4.3).
                        tool_call_id: None,
                        diff: fs::unified_diff(&display, &old, &request.content),
                        path: display,
                        // The gate stayed where the protocol put it — the agent asked for this
                        // edit through `session/request_permission` — so a write that reaches here
                        // was accepted (§4.3).
                        accepted: Some(true),
                    });
                    if !emit(state, events, event, None).await {
                        return false;
                    }
                    match fs::write_text(&path, &request.content).await {
                        Ok(()) => {
                            let _ = responder.respond(WriteTextFileResponse::new());
                            true
                        }
                        Err(err) => {
                            let _ = responder
                                .respond_with_error(agent_client_protocol::Error::internal_error());
                            let event = DriverEvent::Error(ErrorEvent {
                                code: "write_failed".to_owned(),
                                message: err.to_string(),
                            });
                            emit(state, events, event, None).await
                        }
                    }
                }
                Err(outside) => {
                    let _ = responder
                        .respond_with_error(agent_client_protocol::Error::invalid_params());
                    refused(state, events, &outside).await
                }
            }
        }
    }
}

/// Records a refused path.
async fn refused(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    outside: &fs::PathOutside,
) -> bool {
    tracing::warn!(path = %outside.path, "an fs request named a path outside the session");
    let event = DriverEvent::Error(ErrorEvent {
        code: PATH_OUTSIDE_SESSION.to_owned(),
        message: outside.to_string(),
    });
    emit(state, events, event, None).await
}

/// The cancel sequence of `docs/ANA-4.md` §4.3 and plan D20, in order.
async fn cancel_session(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    cx: &ConnectionTo<Agent>,
    session: &mut agent_client_protocol::ActiveSession<'static, Agent>,
    session_id: &agent_client_protocol::schema::v1::SessionId,
    grace: Duration,
) {
    // 1. Every outstanding request is answered `cancelled` **first**: the spec makes it a MUST,
    //    and an agent waiting on a responder never gets to process the cancel notification.
    answer_parked_cancelled(state);

    // 2. Then the notification, and only while there is a turn to cancel.
    if state.turn_open
        && let Err(err) = cx.send_notification(CancelNotification::new(session_id.clone()))
    {
        tracing::warn!(%err, "session/cancel could not be sent");
    }

    // 3. The grace window: a `StopReason` that arrives is the turn's real ending and is recorded
    //    as such, rather than being overwritten by a synthesized `cancelled`.
    if state.turn_open && grace > Duration::ZERO {
        let deadline = tokio::time::sleep(grace);
        tokio::pin!(deadline);
        loop {
            let message = tokio::select! {
                () = &mut deadline => break,
                message = session.read_update() => message,
            };
            match message {
                Ok(SessionMessage::StopReason(reason)) => {
                    let stop = map::stop_reason(&stop_reason_text(&reason));
                    close_turn(state, events, stop).await;
                    break;
                }
                Ok(message) => {
                    if !on_message(state, events, message).await {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    // 4. Whatever is still open is cancelled, and the turn ends exactly once.
    close_turn(state, events, StopReason::Cancelled).await;
}

/// Answers every parked responder `cancelled` (§3: a MUST on cancellation).
fn answer_parked_cancelled(state: &mut TaskState) {
    for (id, parked) in state.parked.drain() {
        if let Err(err) = parked.responder.respond(RequestPermissionResponse::new(
            RequestPermissionOutcome::Cancelled,
        )) {
            tracing::warn!(%err, request = %id, "a parked request could not be cancelled");
        }
    }
}

/// Kills the process tree and reaps it, if there is one and nobody has yet.
///
/// `ChildGuard::kill_and_reap` takes `&mut self` and awaits, so the guard is **swapped out** for an
/// empty one under the lock and awaited outside it: no lock is ever held across an `.await`, and
/// the emptied guard left in the mutex makes a second call — and the `Drop` that eventually runs —
/// a no-op rather than a second signal at a pid the operating system may already have reissued.
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

/// A handshake failure, with the child's captured stderr appended when there is one.
fn handshake_error(
    step: &str,
    err: &agent_client_protocol::Error,
    child: &Mutex<ChildGuard>,
) -> DriverError {
    let stderr = stderr_tail(child);
    if stderr.is_empty() {
        DriverError::Transport(format!("{step} failed: {err}"))
    } else {
        DriverError::Transport(format!("{step} failed: {err}\n{stderr}"))
    }
}

/// The config option that offers `model`, by **id** — never by category (§3).
///
/// `settings.acp.model_config_id` wins when the row sets one; otherwise the first option that
/// actually lists the requested value is the one to set, which is what makes a per-installation
/// vocabulary (`opus[1m]`, `sonnet`, …) work without `htui` knowing it in advance.
fn model_option(
    session: &agent_client_protocol::ActiveSession<'static, Agent>,
    configured: Option<&str>,
    model: &str,
) -> Option<agent_client_protocol::schema::v1::SessionConfigId> {
    let options = session.config_options()?;
    for option in options {
        let Ok(document) = serde_json::to_value(option) else {
            continue;
        };
        let Some(id) = document.get("id").and_then(Value::as_str) else {
            continue;
        };
        let offers_model = values_of(&document).iter().any(|value| value == model);
        let chosen = match configured {
            Some(configured) => configured == id,
            None => offers_model,
        };
        if chosen {
            return Some(id.to_owned().into());
        }
    }
    None
}

/// The values a config option offers, read from its JSON so no `SessionConfigKind` variant is
/// named here — the kind is `#[serde(flatten)]`ed and its shape is the protocol's business.
fn values_of(document: &Value) -> Vec<String> {
    document
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    option
                        .get("value")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The model values the banner advertises: the configured option's, else the one that calls itself
/// the model selector.
fn model_values(
    session: &agent_client_protocol::ActiveSession<'static, Agent>,
    configured: Option<&str>,
) -> Vec<String> {
    let Some(options) = session.config_options() else {
        return Vec::new();
    };
    for option in options {
        let Ok(document) = serde_json::to_value(option) else {
            continue;
        };
        let id = document
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let is_model = match configured {
            Some(configured) => configured == id,
            None => {
                id == "model" || document.get("category").and_then(Value::as_str) == Some("model")
            }
        };
        if is_model {
            return values_of(&document);
        }
    }
    Vec::new()
}
