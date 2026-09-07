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

pub mod client;
pub mod fs;
pub mod map;

use std::collections::{BTreeSet, HashMap, VecDeque};
#[cfg(feature = "test-support")]
use std::sync::Mutex;
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

use crate::acp::client::{Inbound, InboundTx};
use crate::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
    PermissionRequestId, SessionSpec,
};
use crate::error::{DriverError, Result};
use crate::event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, ErrorEvent, OtherEvent,
    PermissionOptionKind, StopReason, TerminalReason, ToolResultEvent, ToolResultStatus,
};
use crate::launch::{AgentLaunch, AgentSettings, Spawned};
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
}

// ---------------------------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------------------------

/// Where a session's byte streams come from.
enum IoSource {
    /// Resolve the row's tools, substitute them, and spawn the child at `start`.
    Spawn {
        /// The row's `launch` document.
        launch: Box<AgentLaunch>,
    },
    /// A pre-built pair, taken once — a real process starts once too.
    #[cfg(feature = "test-support")]
    Prepared(Mutex<Option<AcpIo>>),
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
            .finish()
    }
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
        let launch: AgentLaunch = serde_json::from_value(agent.launch.clone())
            .map_err(|err| DriverError::Transport(format!("agent.launch does not parse: {err}")))?;
        Ok(Self {
            name: agent.name.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Spawn {
                launch: Box::new(launch),
            },
            stamp: Stamp::Wall,
        })
    }

    /// A driver over an in-process transport with a deterministic clock: the conformance harness.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn over(io: AcpIo, agent: &AgentRow, caps: DriverCaps, stamp: Stamp) -> Self {
        Self {
            name: agent.name.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Prepared(Mutex::new(Some(io))),
            stamp,
        }
    }

    /// The streams this session runs over.
    async fn io(&self, spec: &SessionSpec) -> Result<AcpIo> {
        match &self.io {
            IoSource::Spawn { launch } => {
                let tools = crate::tools::resolve(launch.discovery.as_ref(), &spec.cwd).await?;
                let mut resolved = crate::launch::resolve(launch, &tools)?;
                // `SessionSpec.env` is the resolved-secret channel (`R-SEC-2`) and wins over the
                // row's own environment: the row holds placeholders and defaults, the spec holds
                // what the secret provider produced for this run.
                resolved.env.extend(spec.env.clone());
                let mut spawned = crate::launch::spawn(&resolved, &spec.cwd).await?;
                let writer = spawned.take_stdin().ok_or_else(|| {
                    DriverError::Spawn("the agent's stdin was not piped".to_owned())
                })?;
                let reader = spawned.take_stdout().ok_or_else(|| {
                    DriverError::Spawn("the agent's stdout was not piped".to_owned())
                })?;
                Ok(AcpIo {
                    reader: Box::new(reader.into_inner()),
                    writer: Box::new(writer.into_inner()),
                    child: Some(spawned),
                })
            }
            #[cfg(feature = "test-support")]
            IoSource::Prepared(slot) => slot
                .lock()
                .map_err(|_| {
                    DriverError::Transport("the prepared transport is poisoned".to_owned())
                })?
                .take()
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
            };
            let session = open_session(io, spec, prompt, options).await?;
            Ok(Box::new(session) as Box<dyn AgentSession>)
        })
    }
}

/// The [`TransportBuilder`] registered under [`ADAPTER_ID`].
#[derive(Debug, Default, Clone, Copy)]
pub struct AcpAdapter;

impl TransportBuilder for AcpAdapter {
    fn build(
        &self,
        agent: &AgentRow,
        _on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>> {
        Ok(Box::new(AcpDriver::from_row(agent, caps)?))
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
            DriverEvent::PermissionRequest(request) => {
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

/// Opens a session: spawns the task, waits for the handshake, returns the handle.
///
/// # Errors
///
/// [`DriverError::Transport`] carrying whatever the handshake failed with, with the child's
/// captured stderr appended when there was a child.
pub async fn open_session(
    io: AcpIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
) -> Result<AcpSession> {
    let (events_tx, events_rx) = mpsc::channel(EVENTS_CAPACITY);
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();

    let task = tokio::spawn(run_session(
        io,
        spec,
        prompt,
        options,
        ready_tx,
        events_tx,
        commands_rx,
    ));

    match ready_rx.await {
        Ok(Ok(ready)) => Ok(AcpSession {
            session_ref: ready.session_ref,
            events: events_rx,
            commands: commands_tx,
            pending: VecDeque::new(),
            parked: Vec::new(),
            turn_open: true,
            ended: false,
            task: Some(task),
        }),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(DriverError::Transport(
            "the session task ended before the handshake".to_owned(),
        )),
    }
}

/// The session task: owns the child, the connection, the parked responders and the mapper.
async fn run_session(
    io: AcpIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    ready: oneshot::Sender<Result<Ready>>,
    events: mpsc::Sender<DriverEnvelope>,
    commands: mpsc::UnboundedReceiver<SessionCommand>,
) {
    let AcpIo {
        reader,
        writer,
        child,
    } = io;
    let transport = ByteStreams::new(writer.compat_write(), reader.compat());

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
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            session_main(
                cx, spec, prompt, options, ready, events, commands, inbound_rx, child,
            )
            .await;
            Ok(())
        })
        .await;

    if let Err(err) = connected {
        tracing::warn!(%err, "the ACP connection ended with an error");
    }
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
    // about *this session's* outstanding requests rather than about the update: the row is `null`
    // **only** while a permission request for its call is parked; otherwise the edit is one the
    // agent has already been allowed to make, and the proposal records that.
    //
    // The one case this does not cover is a proposal whose request is answered with a rejection
    // afterwards: the row keeps `null`, because the recorder has no update path for a row it has
    // already flushed. Replay stays total — the rejection also synthesizes a
    // `tool_result { failed, terminal_reason: rejected }` for the same call — and filling it
    // retroactively is left to the milestone that gives the recorder that path.
    if let DriverEvent::EditProposal(proposal) = &mut event
        && proposal.accepted.is_none()
    {
        let gated = state.parked.values().any(|parked| {
            parked.tool_call_id.is_some() && parked.tool_call_id == proposal.tool_call_id
        });
        if !gated {
            proposal.accepted = Some(true);
        }
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
    ready: oneshot::Sender<Result<Ready>>,
    events: mpsc::Sender<DriverEnvelope>,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    mut inbound: mpsc::UnboundedReceiver<Inbound>,
    mut child: Option<Spawned>,
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
            let _ = ready.send(Err(handshake_error("initialize", &err, child.as_ref())));
            kill(&mut child).await;
            return;
        }
    };

    // 2. session/new. ANA-4 risk 11: `block_task` only here.
    let new_session =
        NewSessionRequest::new(spec.cwd.clone()).additional_directories(spec.extra_dirs.clone());
    let mut session = match cx
        .build_session_from(new_session)
        .block_task()
        .start_session()
        .await
    {
        Ok(session) => session,
        Err(err) => {
            let _ = ready.send(Err(handshake_error("session/new", &err, child.as_ref())));
            kill(&mut child).await;
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
        kill(&mut child).await;
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
                    kill(&mut child).await;
                    return;
                }
            }
        }
    }

    // 5. The handle may exist now: everything above is what `start` promised to have done.
    if ready.send(Ok(Ready { session_ref })).is_err() {
        kill(&mut child).await;
        return;
    }

    // 6. The first prompt opens turn 0.
    if let Err(err) = session.send_prompt(prompt) {
        let event = DriverEvent::Error(ErrorEvent {
            code: TRANSPORT_CLOSED.to_owned(),
            message: err.to_string(),
        });
        emit(&mut state, &events, event, None).await;
        kill(&mut child).await;
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
                if let Err(err) = session.send_prompt(text) {
                    let event = DriverEvent::Error(ErrorEvent {
                        code: TRANSPORT_CLOSED.to_owned(),
                        message: err.to_string(),
                    });
                    emit(&mut state, &events, event, None).await;
                    break;
                }
                state.turn_open = true;
            }
            Step::Command(Some(SessionCommand::AnswerPermission(id, answer))) => {
                if !answer_permission(&mut state, &events, id, answer).await {
                    break;
                }
            }
            Step::Command(Some(SessionCommand::Cancel { grace, done })) => {
                cancel_session(&mut state, &events, &cx, &mut session, &session_id, grace).await;
                kill(&mut child).await;
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
                    Duration::from_secs(0),
                )
                .await;
                break;
            }
            Step::Inbound(Some(request)) => {
                if !on_inbound(&mut state, &events, &spec, request).await {
                    break;
                }
            }
            Step::Inbound(None) => {}
            Step::Closed => {
                let message = child
                    .as_ref()
                    .map(|child| child.stderr_tail().join("\n"))
                    .unwrap_or_default();
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

    kill(&mut child).await;
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
    let rejected = match &answer {
        PermissionAnswer::Selected(option_id) => parked
            .options
            .iter()
            .find(|option| &option.id == option_id)
            .is_some_and(|option| {
                matches!(
                    option.kind,
                    PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                )
            }),
        PermissionAnswer::Cancelled => true,
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
    if rejected && let Some(tool_call_id) = parked.tool_call_id {
        let event = DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id,
            status: ToolResultStatus::Failed,
            output: None,
            locations: Vec::new(),
            terminal_reason: Some(TerminalReason::Rejected),
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

/// Kills the process tree and reaps it, if there is one.
async fn kill(child: &mut Option<Spawned>) {
    let Some(child) = child.as_mut() else { return };
    if let Err(err) = child.kill_tree().await {
        tracing::warn!(%err, "the agent's process tree did not die cleanly");
    }
    if let Err(err) = child.wait().await {
        tracing::warn!(%err, "the agent process could not be reaped");
    }
}

/// A handshake failure, with the child's captured stderr appended when there is one.
fn handshake_error(
    step: &str,
    err: &agent_client_protocol::Error,
    child: Option<&Spawned>,
) -> DriverError {
    let stderr = child
        .map(|child| child.stderr_tail().join("\n"))
        .unwrap_or_default();
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
