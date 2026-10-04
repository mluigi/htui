//! The TUI's adapter over `htui-worker`'s run runtime (MOD-4 milestone 6, plan D153; MOD-41 plan
//! D6, D7).
//!
//! The runtime itself — the request and reply shapes the views speak, the verdicts, the graph
//! source, the supervised tasks, the locks and the sweep — lives in [`htui_worker`], which links
//! no terminal crate so `htui worker` can run it headless. What stays here joins it to the store
//! worker's loop:
//!
//! - [`TuiReplies`], the reply sink over the loop's channel: a reply goes out as a
//!   [`ReplyEnvelope`] at its request's `seq` and origin, and a `RunStream` subscription is keyed
//!   on the origin;
//! - [`RunRuntime`] and [`RunServed`], the library's types over the TUI's [`Backend`] and
//!   [`ReplyAddr`];
//! - [`TuiRuns`], the loop's `serve` and `sweep` with the signatures they always had, over the
//!   library's `serve_request` and `sweep_with`, and its `poll_commands` (MOD-42 plan D13) over
//!   `poll_commands_with`;
//! - the re-exports, so every `crate::run_worker::X` path the views and tests name still resolves.
//!
//! The test module stays too: it drives the runtime through the TUI's store loop, and its
//! white-box cases reach the library's private state through `htui_worker::testing` (MOD-41 plan
//! D8).

use std::future::Future;

use htui_store::Backend;
use htui_worker::ReplySink;
pub use htui_worker::{
    ChatEnd, Enabled, FrameKind, HostGraphs, ItemActions, LiveChats, ORCH_NAMES, OrchReply,
    OrchRequest, PREEMPTED, ProgressSink, Promoted, REPOS_MOVED, RunActions, RunFrame, RunLocks,
    RunReply, RunRequest, StepActions, StepAuthor, UNBLOCK_MOVED, Via, WALK_PANICKED, actions,
};
use tokio::sync::mpsc;

use crate::agent_worker::ReplyAddr;
use crate::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest};

/// The TUI's run runtime (MOD-41 plan D7): the library's, over the [`Backend`] and the store
/// loop's channel.
pub type RunRuntime = htui_worker::RunRuntime<Backend, TuiReplies>;

/// What the TUI's runtime decided about one request, at a [`ReplyAddr`] (MOD-41 plan D7).
pub type RunServed = htui_worker::RunServed<ReplyAddr>;

/// The production run runtime the store loop serves `backend` with.
///
/// Outside tests a memory backend is only ever `htui --demo`, whose seeded `queued` run (`RUN_2`
/// on `FEAT-3`) is a showcase: its runtime never claim-scans, so no sweep walks that run with the
/// production isolator and drivers (MOD-41 finding T9-V1). Every other backend gets
/// [`htui_worker::RunRuntime::production`], whose sweep claims the box's queued rows (plan D14,
/// OQ-5).
#[must_use]
pub fn production_for(backend: &Backend) -> RunRuntime {
    match backend {
        Backend::Memory(_) => RunRuntime::production().without_claim_scan(),
        _ => RunRuntime::production(),
    }
}

impl From<RunReply> for StoreReply {
    fn from(reply: RunReply) -> Self {
        match reply {
            RunReply::Orch(reply) => Self::Orch(reply),
            RunReply::Frame(frame) => Self::RunStream(frame),
            RunReply::Actions(actions) => Self::RunActions(actions),
            RunReply::Failed { request, message } => Self::Failed { request, message },
        }
    }
}

/// The TUI's reply sink: the store loop's reply channel (MOD-41 plan D7). A reply goes out as a
/// [`ReplyEnvelope`] at its address's `seq` and origin; a subscription is keyed on the origin, so
/// a later `RunStream` from the same origin replaces the earlier one.
#[derive(Debug, Clone)]
pub struct TuiReplies(pub mpsc::UnboundedSender<ReplyEnvelope>);

impl ReplySink for TuiReplies {
    type Addr = ReplyAddr;
    type Subscriber = Origin;

    fn subscriber(addr: &ReplyAddr) -> Origin {
        addr.origin.clone()
    }

    fn send(&self, to: &ReplyAddr, reply: RunReply) {
        let _ = self.0.send(ReplyEnvelope {
            seq: to.seq,
            origin: to.origin.clone(),
            reply: reply.into(),
        });
    }
}

/// The store loop's `serve` and `sweep` over the TUI's runtime (MOD-41 blueprint B-3): today's
/// signatures, over [`RunRuntime::serve_request`] and [`RunRuntime::sweep_with`].
pub trait TuiRuns {
    /// One `Orch`, `RunStream` or `RunActions` envelope, answered at its `seq` and origin; any
    /// other request is refused with `not an orchestrator request`.
    fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
        live: &LiveChats,
    ) -> impl Future<Output = RunServed> + Send;

    /// One sweep tick.
    fn sweep(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>);

    /// MOD-42 plan D13: one command-poll tick over [`RunRuntime::poll_commands_with`].
    fn poll_commands(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        live: &LiveChats,
    );
}

impl TuiRuns for RunRuntime {
    async fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
        live: &LiveChats,
    ) -> RunServed {
        let request = match &envelope.request {
            StoreRequest::Orch(request) => RunRequest::Orch(request.clone()),
            StoreRequest::RunStream { item } => RunRequest::Stream { item: *item },
            StoreRequest::RunActions(item) => RunRequest::Actions(*item),
            other => {
                return RunServed::Reply(RunReply::Failed {
                    request: other.name(),
                    message: "not an orchestrator request".to_owned(),
                });
            }
        };
        let addr = ReplyAddr {
            seq: envelope.seq,
            origin: envelope.origin.clone(),
        };
        self.serve_request(backend, &TuiReplies(replies.clone()), addr, request, live)
            .await
    }

    fn sweep(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>) {
        self.sweep_with(backend, &TuiReplies(replies.clone()));
    }

    fn poll_commands(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        live: &LiveChats,
    ) {
        self.poll_commands_with(backend, &TuiReplies(replies.clone()), live.clone());
    }
}

/// The walk fixture is shared with `store_worker`'s promotion case (blueprint §8.10).
#[cfg(test)]
pub(crate) mod tests {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
    use htui_agent::conformance::{Script, ScriptEvent};
    use htui_agent::driver::{
        AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
        PermissionRequestId, SessionSpec,
    };
    use htui_agent::error::DriverError;
    use htui_agent::event::{
        DoneEvent, DriverEnvelope, DriverEvent, PermissionOption, PermissionOptionKind,
        PermissionRequestEvent, StopReason, ToolCallEvent, ToolKind,
    };
    use htui_agent::fake::FakeDriver;
    use htui_agent::registry::{DriverFactory, TransportBuilder};
    use htui_core::fixtures::{demo_at, edit_agent, ids};
    use htui_core::model::{
        Agent, AgentBox, AgentId, Billing, BoxEdit, BoxId, CancelRequest, ChatRunSpec, DocumentId,
        EventKind, Executor, Gate, Item, ItemId, NewDocument, NewRepo, NewRun, NewRunStep,
        PermissionId, PermissionStatus, RepoId, Resolution, Run, RunCommand, RunCommandId,
        RunCommandKind, RunCommandStatus, RunId, RunMode, RunStatus, RunStep, RunStepCommit,
        SnapshotPhase, Status, StepId, StepPermission, StepStatus, TIMESTAMPTZ_DIGITS, Transport,
    };
    use htui_core::store::mem::MemFault;
    use htui_core::store::{CasOutcome, MemStore, ReadStore as _, StepFence, WriteStore as _};
    use htui_orch::fake::{FakeIsolator, FakeVerifier};
    use htui_orch::{
        Clock, Command, CommandOutcome, EngineError, GateAnswer, GraphSource, Isolator,
    };
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore, Started};
    use serde_json::json;
    use tokio::sync::{Notify, mpsc};

    use htui_worker::{
        CANCEL_ALREADY_REQUESTED, CANCEL_GRACE, CANCEL_REQUESTED, Role, promote_needs_the_walker,
        testing, unknown_executor,
    };

    use super::{
        FrameKind, HostGraphs, LiveChats, ORCH_NAMES, OrchReply, OrchRequest, PREEMPTED, ReplySink,
        RunReply, RunRequest, RunRuntime, RunServed, StepAuthor, TuiReplies, TuiRuns as _,
        WALK_PANICKED, production_for,
    };
    use crate::agent_worker::AgentRuntime;
    use crate::store_worker::{
        self, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest, spawn_with_runtimes,
    };
    use crate::ui::tabs::TabId;
    use uuid::Uuid;

    /// The scripted registry row every walk test runs on (blueprint F-O): an `acp` row, because
    /// the fixture graphs gate every phase and stage 1's inline-approval interlock skips a `cli`
    /// row at a gated phase; the factory reaches the fake by row data alone.
    fn scripted_row(id: AgentId) -> Agent {
        Agent {
            id,
            name: "scripted".to_owned(),
            transport: Transport::Acp,
            billing: Billing::Subscription,
            models: Vec::new(),
            default_model: Some("sonnet".to_owned()),
            launch: json!({ "command": "unused", "args": [] }),
            settings: json!({}),
            enabled: true,
            created_at: demo_at(0, 0),
            updated_at: demo_at(0, 0),
        }
    }

    /// The scripted agent's `agent_box` on the demo box, probed ready (rung 3, plan D62).
    fn ready_on_box(agent_id: AgentId, at: DateTime<Utc>) -> AgentBox {
        AgentBox {
            agent_id,
            box_id: ids::BOX,
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(at),
            quota: None,
            quota_at: None,
            updated_at: at,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        }
    }

    /// Blueprint F-O: the demo with every fixture agent disabled, one scripted agent and its
    /// `agent_box` on the demo box, so rung 3 of the candidate chain names exactly it.
    async fn seeded_store() -> (MemStore, AgentId) {
        seeded(MemStore::demo()).await
    }

    /// [`seeded_store`]'s seeding over `store` (MOD-41 T9: a demo whose box settings a case
    /// edited).
    async fn seeded(store: MemStore) -> (MemStore, AgentId) {
        for summary in store.agents().await.expect("the fixture's agents") {
            let mut row = summary.agent;
            row.enabled = false;
            edit_agent(&store, &row).await.expect("the row is disabled");
        }
        let agent = AgentId::new();
        store
            .upsert_agent(&scripted_row(agent), None)
            .await
            .expect("the scripted row lands");
        store
            .upsert_agent_box(&ready_on_box(agent, demo_at(0, 0)))
            .await
            .expect("the agent_box row lands");
        // The demo project has no repo, and a promoted step chats in its own tree: the primary
        // repo is the scope a default `StartRun` resolves to.
        store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo project has no repo yet");
        // MOD-41 OQ-5: a TUI sweep on a `tui` box claims every queued row of its box, and the
        // demo seeds one (`RUN_2` on `FEAT-3`). Its walk would play the sessions a case scripts
        // for its own runs, so the fixture ends it: every run a case sees is one it made.
        store
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the seeded run is queued and cancellable");
        (store, agent)
    }

    // -----------------------------------------------------------------------------------------
    // The walk fixture (blueprint §8.9, F-O)
    // -----------------------------------------------------------------------------------------

    /// What one agent session does.
    #[derive(Debug, Clone)]
    enum Play {
        /// One turn, then `done`.
        Done,
        /// Waits on `release` before its first event; `reached` is raised when it starts waiting.
        Stall(Stall),
        /// The session's start panics.
        Panic,
        /// MOD-42 T4: `ToolCall`, `ParkPermission(request)`, then `Done`; the grace its session's
        /// `cancel` received is kept in the [`Park`]'s cell.
        Park(Park),
    }

    /// A parking session's request and the grace its `cancel` was given, if it was cancelled.
    #[derive(Debug, Clone)]
    struct Park {
        request: PermissionRequestEvent,
        grace: Arc<StdMutex<Option<Duration>>>,
    }

    impl Park {
        /// A request over one `execute` call, offering one allow and one reject option.
        fn new() -> Self {
            Self {
                request: PermissionRequestEvent {
                    request_id: PermissionRequestId::new("request-1"),
                    tool_call_id: Some("call-1".to_owned()),
                    options: vec![
                        PermissionOption {
                            id: "allow-once".to_owned(),
                            label: "Allow".to_owned(),
                            kind: PermissionOptionKind::AllowOnce,
                        },
                        PermissionOption {
                            id: "reject-once".to_owned(),
                            label: "Reject".to_owned(),
                            kind: PermissionOptionKind::RejectOnce,
                        },
                    ],
                },
                grace: Arc::default(),
            }
        }

        /// The grace the session's `cancel` received; `None` while it was never cancelled.
        fn grace(&self) -> Option<Duration> {
            *self.grace.lock().expect("the cell")
        }

        /// The script: the gated call, the parked request, then `done`.
        fn script(&self) -> Script {
            Script::one_turn(vec![
                ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
                    tool_call_id: "call-1".to_owned(),
                    title: "run the suite".to_owned(),
                    tool_kind: ToolKind::Execute,
                    input: json!({ "command": "cargo test" }),
                    locations: Vec::new(),
                })),
                ScriptEvent::ParkPermission(self.request.clone()),
                ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                })),
            ])
        }
    }

    /// A stalled session's three signals.
    #[derive(Debug, Clone, Default)]
    struct Stall {
        reached: Arc<Notify>,
        release: Arc<Notify>,
        dropped: Arc<AtomicBool>,
    }

    /// The sessions the fixture's builds play, in build order; [`Play::Done`] once it is empty.
    #[derive(Debug, Default)]
    struct Sessions(StdMutex<VecDeque<Play>>);

    impl Sessions {
        fn push(&self, play: Play) {
            self.0.lock().expect("the queue").push_back(play);
        }
    }

    /// The transport the scripted row reaches: one [`ScriptedDriver`] per session.
    #[derive(Debug)]
    struct Scripted(Arc<Sessions>);

    impl TransportBuilder for Scripted {
        fn build(
            &self,
            agent: &Agent,
            _on_box: Option<&AgentBox>,
            caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            let play = self
                .0
                .0
                .lock()
                .expect("the queue")
                .pop_front()
                .unwrap_or(Play::Done);
            let script = match &play {
                Play::Park(park) => park.script(),
                Play::Done | Play::Stall(_) | Play::Panic => {
                    Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                        stop_reason: StopReason::EndTurn,
                    }))])
                }
            };
            Ok(Box::new(ScriptedDriver {
                inner: FakeDriver::new(agent.name.clone(), caps, script),
                play,
            }))
        }
    }

    #[derive(Debug)]
    struct ScriptedDriver {
        inner: FakeDriver,
        play: Play,
    }

    impl AgentDriver for ScriptedDriver {
        fn name(&self) -> &str {
            self.inner.name()
        }

        fn caps(&self) -> DriverCaps {
            self.inner.caps()
        }

        fn start<'a>(
            &'a self,
            spec: SessionSpec,
            prompt: String,
        ) -> DriverFuture<'a, Box<dyn AgentSession>> {
            let inner = self.inner.start(spec, prompt);
            let play = self.play.clone();
            Box::pin(async move {
                let session = inner.await?;
                match play {
                    Play::Done => Ok(session),
                    Play::Stall(stall) => Ok(Box::new(Stalled {
                        inner: session,
                        stall,
                        released: false,
                    }) as Box<dyn AgentSession>),
                    Play::Panic => panic!("a scripted session panics"),
                    Play::Park(park) => Ok(Box::new(Graced {
                        inner: session,
                        grace: park.grace,
                    }) as Box<dyn AgentSession>),
                }
            })
        }
    }

    /// A session that keeps the grace its `cancel` was given (MOD-42 B-19).
    #[derive(Debug)]
    struct Graced {
        inner: Box<dyn AgentSession>,
        grace: Arc<StdMutex<Option<Duration>>>,
    }

    impl AgentSession for Graced {
        fn session_ref(&self) -> Option<&AgentSessionRef> {
            self.inner.session_ref()
        }

        fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
            self.inner.next_event()
        }

        fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
            self.inner.send_follow_up(text)
        }

        fn answer_permission<'a>(
            &'a mut self,
            request_id: PermissionRequestId,
            answer: PermissionAnswer,
        ) -> DriverFuture<'a, ()> {
            self.inner.answer_permission(request_id, answer)
        }

        fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
            *self.grace.lock().expect("the cell") = Some(grace);
            self.inner.cancel(grace)
        }
    }

    /// A session that waits on its [`Stall`] before its first event, and says when it is dropped.
    #[derive(Debug)]
    struct Stalled {
        inner: Box<dyn AgentSession>,
        stall: Stall,
        released: bool,
    }

    impl Drop for Stalled {
        fn drop(&mut self) {
            self.stall.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl AgentSession for Stalled {
        fn session_ref(&self) -> Option<&AgentSessionRef> {
            self.inner.session_ref()
        }

        fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
            Box::pin(async move {
                if !self.released {
                    self.stall.reached.notify_one();
                    self.stall.release.notified().await;
                    self.released = true;
                }
                self.inner.next_event().await
            })
        }

        fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
            self.inner.send_follow_up(text)
        }

        fn answer_permission<'a>(
            &'a mut self,
            request_id: PermissionRequestId,
            answer: PermissionAnswer,
        ) -> DriverFuture<'a, ()> {
            self.inner.answer_permission(request_id, answer)
        }

        fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
            // MOD-42 B-11: a cancelled session stalls no longer, so the graceful drain that
            // follows a cancel reads the turn's `done` at once instead of waiting out its bound.
            self.released = true;
            self.inner.cancel(grace)
        }
    }

    /// Blueprint H-2: `base + (tokio now - start)`, so a `start_paused` test moves the rows'
    /// timestamps with the heartbeat's own sleeps. It moves no lease fence: since MOD-41 T2 (plan
    /// D3) the heartbeat fences on tokio's monotonic clock, never on a `Clock`.
    #[derive(Debug)]
    struct TokioClock {
        base: DateTime<Utc>,
        start: tokio::time::Instant,
    }

    impl TokioClock {
        fn new() -> Self {
            Self {
                base: Utc::now(),
                start: tokio::time::Instant::now(),
            }
        }
    }

    impl Clock for TokioClock {
        fn now(&self) -> DateTime<Utc> {
            let elapsed = TimeDelta::from_std(self.start.elapsed()).expect("a test is short");
            (self.base + elapsed).trunc_subsecs(TIMESTAMPTZ_DIGITS)
        }
    }

    /// D203's author for tests: one document of the phase's `output_kind` per step.
    #[derive(Debug)]
    struct OutputAuthor;

    impl StepAuthor for OutputAuthor {
        fn document(
            &self,
            item: ItemId,
            step: &RunStep,
            phase: &SnapshotPhase,
        ) -> Option<NewDocument> {
            Some(NewDocument {
                id: DocumentId::new(),
                item_id: item,
                kind: phase.output_kind.clone(),
                title: format!("{} (attempt {})", phase.output_kind, step.attempt),
                body: "authored".to_owned(),
                produced_by_step_id: Some(step.id),
                created_by: ids::USER,
                created_at: Utc::now(),
            })
        }
    }

    /// The seeded store, the sessions its builds play, and the isolator its runs use.
    pub(crate) struct Fixture {
        pub(crate) store: MemStore,
        sessions: Arc<Sessions>,
        isolator: Arc<FakeIsolator>,
    }

    impl Fixture {
        pub(crate) async fn new() -> Self {
            Self::over(MemStore::demo()).await
        }

        /// The fixture over `store`, seeded as [`Self::new`]'s (MOD-41 T9: a demo whose box
        /// settings a case edited).
        async fn over(store: MemStore) -> Self {
            let (store, _) = seeded(store).await;
            Self {
                store,
                sessions: Arc::default(),
                isolator: Arc::new(FakeIsolator::new()),
            }
        }

        /// The registry the scripted row reaches the fake through, by row data (`acp`).
        fn factory(&self) -> DriverFactory {
            let mut factory = DriverFactory::new();
            factory.register("acp", Box::new(Scripted(Arc::clone(&self.sessions))));
            factory
        }

        /// Blueprint §8.9's runtime: the fakes, a tokio-time clock and the output author.
        pub(crate) fn runtime(&self) -> RunRuntime {
            RunRuntime::with_parts(
                Arc::clone(&self.isolator) as Arc<dyn Isolator>,
                Arc::new(FakeVerifier::new()),
                self.factory(),
            )
            .with_clock(Arc::new(TokioClock::new()))
            .with_author(Arc::new(OutputAuthor))
        }

        pub(crate) async fn run(&self, id: RunId) -> Run {
            self.store
                .run(id)
                .await
                .expect("the read answers")
                .expect("the run exists")
        }

        async fn item(&self, id: ItemId) -> Item {
            self.store
                .item(id)
                .await
                .expect("the read answers")
                .expect("the item exists")
        }

        async fn steps(&self, run: RunId) -> Vec<RunStep> {
            self.store.run_steps(run).await.expect("the read answers")
        }
    }

    /// How long a test waits for any one reply before it calls the worker stuck.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// A store worker over the fixture's store, with the run runtime under test.
    pub(crate) struct Worker {
        requests: mpsc::UnboundedSender<RequestEnvelope>,
        replies: mpsc::UnboundedReceiver<ReplyEnvelope>,
        seen: Vec<ReplyEnvelope>,
        seq: u64,
    }

    impl Worker {
        pub(crate) fn spawn(store: &MemStore, runtime: RunRuntime) -> Self {
            let (requests, requests_rx) = mpsc::unbounded_channel();
            let (replies_tx, replies) = mpsc::unbounded_channel();
            let _worker = spawn_with_runtimes(
                Started::detached(Backend::memory(store.clone())),
                requests_rx,
                replies_tx,
                AgentRuntime::new(DriverFactory::new()),
                runtime,
            );
            Self {
                requests,
                replies,
                seen: Vec::new(),
                seq: 0,
            }
        }

        /// Sends `request` from `origin` at the next `seq`.
        pub(crate) fn send(&mut self, origin: Origin, request: StoreRequest) -> u64 {
            self.seq += 1;
            self.send_at(origin, self.seq, request)
        }

        /// Sends `request` from `origin` at exactly `seq`.
        fn send_at(&mut self, origin: Origin, seq: u64, request: StoreRequest) -> u64 {
            self.seq = self.seq.max(seq);
            self.requests
                .send(RequestEnvelope {
                    seq,
                    origin,
                    request,
                })
                .expect("the worker is running");
            seq
        }

        /// The first non-stream reply at `seq`, waiting for it.
        pub(crate) async fn reply(&mut self, seq: u64) -> StoreReply {
            self.envelope(seq).await.reply
        }

        /// The first non-stream envelope at `seq`, waiting for it.
        pub(crate) async fn envelope(&mut self, seq: u64) -> ReplyEnvelope {
            self.envelope_within(seq, PATIENCE).await
        }

        /// [`Self::envelope`], waiting up to `patience` for each arrival.
        async fn envelope_within(&mut self, seq: u64, patience: Duration) -> ReplyEnvelope {
            loop {
                if let Some(at) = self.seen.iter().position(|envelope| {
                    envelope.seq == seq && !matches!(envelope.reply, StoreReply::RunStream(_))
                }) {
                    return self.seen.remove(at);
                }
                let envelope = tokio::time::timeout(patience, self.replies.recv())
                    .await
                    .unwrap_or_else(|_| panic!("no reply at seq {seq} within {patience:?}"))
                    .expect("the worker is running");
                self.seen.push(envelope);
            }
        }

        /// Whatever has arrived by now.
        fn drain(&mut self) {
            while let Ok(envelope) = self.replies.try_recv() {
                self.seen.push(envelope);
            }
        }
    }

    /// `StartRun` for `item`, manual, default scope.
    pub(crate) fn start_run(item: ItemId) -> StoreRequest {
        StoreRequest::Orch(OrchRequest::Command(Command::StartRun {
            item,
            mode: RunMode::Manual,
            repo_scope: None,
        }))
    }

    /// `future`, or a panic naming what did not happen.
    async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
        tokio::time::timeout(PATIENCE, future)
            .await
            .unwrap_or_else(|_| panic!("{what} did not happen within {PATIENCE:?}"))
    }

    /// The outcome of a `Done` reply, or a panic showing what came instead.
    fn outcome(reply: StoreReply) -> CommandOutcome {
        match reply {
            StoreReply::Orch(OrchReply::Done(outcome)) => *outcome,
            other => panic!("expected a command outcome, got {other:?}"),
        }
    }

    /// `R-NF-3`: a walk mid-session does not hold the loop — the `Workspaces` read asked after
    /// the `StartRun` is answered first, and the `StartRun` once its session ends.
    #[tokio::test]
    async fn an_orch_request_is_served_off_the_loop() {
        let fixture = Fixture::new().await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        within("the session starting", stall.reached.notified()).await;
        let workspaces = worker.send(Origin::App, StoreRequest::Workspaces);
        assert!(matches!(
            worker.reply(workspaces).await,
            StoreReply::Workspaces(_)
        ));
        worker.drain();
        assert!(
            !worker.seen.iter().any(|envelope| envelope.seq == start),
            "the walk is still in its session"
        );

        stall.release.notify_one();
        let CommandOutcome::Started { run, rest } = outcome(worker.reply(start).await) else {
            panic!("a start answers Started");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval, "`research` gates");
        assert_eq!(fixture.run(run).await.status, RunStatus::AwaitingApproval);
    }

    /// The one run of `item`, which a test has just started.
    pub(crate) async fn only_run(store: &MemStore, item: ItemId) -> RunId {
        let runs = store.runs(item).await.expect("the read answers");
        assert_eq!(runs.len(), 1, "one run of the item: {runs:?}");
        runs[0].id
    }

    /// The latest step at `position` of `run`.
    pub(crate) async fn step_at(fixture: &Fixture, run: RunId, position: i32) -> RunStep {
        fixture
            .steps(run)
            .await
            .into_iter()
            .filter(|step| step.position == position)
            .max_by_key(|step| step.attempt)
            .expect("a step at the position")
    }

    /// A parked run of `HTUI_ANA_2`: its `research` step awaits approval.
    pub(crate) async fn parked(fixture: &Fixture, worker: &mut Worker) -> (RunId, RunStep) {
        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        let CommandOutcome::Started { run, rest } = outcome(worker.reply(start).await) else {
            panic!("a start answers Started");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        (run, step_at(fixture, run, 0).await)
    }

    /// R-27: a second command on a run waits for the first to finish its walk, then is judged
    /// against the rows that walk left.
    #[tokio::test]
    async fn two_commands_on_one_run_are_serialised() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (run, research) = parked(&fixture, &mut worker).await;

        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let retry = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::RetryStep {
                run,
                step: research.id,
            })),
        );
        within("attempt 2's session starting", stall.reached.notified()).await;
        let answer = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        worker.drain();
        assert!(
            !worker
                .seen
                .iter()
                .any(|envelope| envelope.seq == answer || envelope.seq == retry),
            "neither command has answered while attempt 2 walks: {:?}",
            worker.seen
        );

        stall.release.notify_one();
        assert!(matches!(
            outcome(worker.reply(retry).await),
            CommandOutcome::Retried { .. }
        ));
        let StoreReply::Failed { request, message } = worker.reply(answer).await else {
            panic!("the answer is refused");
        };
        assert_eq!(request, "answer_gate");
        assert!(
            message.contains(&format!("step {} is `superseded`", research.id)),
            "the gate is judged after the retry moved the step: {message}"
        );
    }

    /// D157, D187, D188: `CancelRun` stops a walk mid-session instead of waiting hours for it,
    /// and the dropped walk's lease and guards are given back.
    #[tokio::test]
    async fn cancel_preempts_a_live_walk() {
        let fixture = Fixture::new().await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run })),
        );

        assert!(matches!(
            outcome(worker.reply(cancel).await),
            CommandOutcome::Cancelled { .. }
        ));
        assert!(
            matches!(worker.reply(start).await, StoreReply::Failed { request: "start_run", ref message } if message == PREEMPTED),
            "the preempted start is answered once, with the sentence"
        );
        assert!(
            stall.dropped.load(Ordering::SeqCst),
            "the session was dropped"
        );
        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Cancelled);
        assert!(
            row.lease_expires_at
                .is_some_and(|until| until <= Utc::now()),
            "the lease is given back: {:?}",
            row.lease_expires_at
        );
        assert_eq!(fixture.item(ids::HTUI_ANA_2).await.status, Status::Open);
        assert!(
            fixture.isolator.releases() >= 1,
            "the abandoned walk's guards were released"
        );
    }

    /// D182, D184: the verdicts are the engine's own guards over the rows.
    #[tokio::test]
    async fn run_actions_grey_by_the_engine_guards() {
        let fixture = Fixture::new().await;
        let runtime = RunRuntime::with_parts(
            Arc::clone(&fixture.isolator) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()),
            fixture.factory(),
        )
        .with_clock(Arc::new(TokioClock::new()));
        let mut worker = Worker::spawn(&fixture.store, runtime);
        let (_run, research) = parked(&fixture, &mut worker).await;

        let ask = worker.send(Origin::App, StoreRequest::RunActions(ids::HTUI_ANA_2));
        let StoreReply::RunActions(actions) = worker.reply(ask).await else {
            panic!("the verdicts");
        };
        assert_eq!(
            actions.steps[&research.id].approve,
            Err(EngineError::MissingOutputForApproval {
                step: research.id,
                kind: "research".to_owned(),
            }
            .to_string())
        );
        assert_eq!(actions.steps[&research.id].reject, Ok(()));

        let document = OutputAuthor
            .document(ids::HTUI_ANA_2, &research, &research_phase())
            .expect("the author writes one");
        fixture
            .store
            .write_document(document)
            .await
            .expect("the document lands");
        let ask = worker.send(Origin::App, StoreRequest::RunActions(ids::HTUI_ANA_2));
        let StoreReply::RunActions(actions) = worker.reply(ask).await else {
            panic!("the verdicts");
        };
        assert_eq!(actions.steps[&research.id].approve, Ok(()));
        assert!(actions.steps[&research.id].open.is_ok());
        assert_eq!(actions.steps[&research.id].promote, Ok(()));

        let live = super::actions(
            &Backend::memory(fixture.store.clone()),
            ids::HTUI_ANA_2,
            &LiveChats::of([StepId::new()]),
        )
        .await
        .expect("the verdicts");
        assert_eq!(
            live.steps[&research.id].promote,
            Err(EngineError::ChatLive { step: None }.to_string()),
            "a chat of this process is live elsewhere"
        );
    }

    /// D212 (review H3): while a step of a run is chatted with, the verbs that would move the run
    /// under the chat — approve, reject, retry, select and cancel — grey with the worker's own
    /// refusal. A chat on a step of no run of the item greys none of them.
    #[tokio::test]
    async fn run_actions_grey_the_run_s_verbs_while_a_chat_is_live_on_it() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (run, research) = parked(&fixture, &mut worker).await;
        let backend = Backend::memory(fixture.store.clone());

        let live = super::actions(&backend, ids::HTUI_ANA_2, &LiveChats::of([research.id]))
            .await
            .expect("the verdicts");
        let refusal = Err(EngineError::ChatLive {
            step: Some(research.id),
        }
        .to_string());
        let verdict = &live.steps[&research.id];
        for (verb, enabled) in [
            ("approve", &verdict.approve),
            ("reject", &verdict.reject),
            ("retry", &verdict.retry),
            ("select", &verdict.select),
            ("cancel", &live.runs[&run].cancel),
        ] {
            assert_eq!(enabled, &refusal, "{verb} greys while the chat is live");
        }

        let elsewhere = super::actions(&backend, ids::HTUI_ANA_2, &LiveChats::of([StepId::new()]))
            .await
            .expect("the verdicts");
        let verdict = &elsewhere.steps[&research.id];
        for (verb, enabled) in [
            ("approve", &verdict.approve),
            ("reject", &verdict.reject),
            ("retry", &verdict.retry),
            ("cancel", &elsewhere.runs[&run].cancel),
        ] {
            assert_eq!(
                enabled,
                &Ok(()),
                "{verb} stays enabled beside a chat on a step of no run of the item"
            );
        }
        // `select` is refused on a step that is not a fan-out candidate whatever is chatted with;
        // what matters is that the refusal is not the chat's.
        assert_ne!(
            &verdict.select, &refusal,
            "select is not greyed by a chat on a step of no run of the item"
        );
    }

    /// D212 (review H3), D200: a verb on a run one of whose steps is chatted with is refused inside
    /// its task with `ChatLive(Some)`'s sentence, the refusal is published for the run's item, and
    /// nothing moves.
    #[tokio::test]
    async fn a_verb_on_a_run_being_chatted_with_is_refused_and_published() {
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        let envelope = |seq: u64, origin: Origin, request: StoreRequest| RequestEnvelope {
            seq,
            origin,
            request,
        };
        let served = runtime
            .serve(
                &backend,
                &replies,
                &envelope(1, Origin::App, start_run(ids::HTUI_ANA_2)),
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred));
        assert!(runtime.settle(PATIENCE).await.is_empty());
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        let research = step_at(&fixture, run, 0).await;
        assert_eq!(research.status, StepStatus::AwaitingApproval);
        let backlog = Origin::Tab(TabId("backlog"));
        runtime
            .serve(
                &backend,
                &replies,
                &envelope(
                    2,
                    backlog.clone(),
                    StoreRequest::RunStream {
                        item: ids::HTUI_ANA_2,
                    },
                ),
                &LiveChats::default(),
            )
            .await;
        while answers.try_recv().is_ok() {}

        let live = LiveChats::of([research.id]);
        let refusal = EngineError::ChatLive {
            step: Some(research.id),
        }
        .to_string();
        let verbs = [
            Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            },
            Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Rejected {
                    note: "not yet".to_owned(),
                },
            },
            Command::RetryStep {
                run,
                step: research.id,
            },
            Command::SelectFanout {
                run,
                position: 0,
                attempt: research.attempt,
                winner: research.id,
            },
            Command::CancelRun { run },
        ];
        for (seq, command) in (3..).zip(verbs) {
            let request = StoreRequest::Orch(OrchRequest::Command(command));
            let name = request.name();
            let served = runtime
                .serve(
                    &backend,
                    &replies,
                    &envelope(seq, Origin::App, request),
                    &live,
                )
                .await;
            assert!(matches!(served, RunServed::Deferred), "{name}: {served:?}");
            assert!(runtime.settle(PATIENCE).await.is_empty());
            let (mut answered, mut published) = (None, false);
            while let Ok(answer) = answers.try_recv() {
                match answer.reply {
                    StoreReply::Failed { request, message } if answer.seq == seq => {
                        answered = Some((request, message));
                    }
                    StoreReply::RunStream(super::RunFrame {
                        kind: FrameKind::Error(sentence),
                        ..
                    }) if answer.origin == backlog => {
                        assert_eq!(sentence, refusal, "{name}");
                        published = true;
                    }
                    other => panic!("{name}: nothing else is answered or published: {other:?}"),
                }
            }
            assert_eq!(answered, Some((name, refusal.clone())));
            assert!(published, "{name}: the refusal is published for the item");
        }
        assert_eq!(fixture.run(run).await.status, RunStatus::AwaitingApproval);
        assert_eq!(
            step_at(&fixture, run, 0).await.status,
            StepStatus::AwaitingApproval
        );
    }

    /// D215 (review finding 6, changed by maintainer choice): `RunActions` is served from a
    /// tracked task like a command, so the loop awaits none of its reads, and it is answered once
    /// at the request's own origin and `seq` — a refusal included.
    #[tokio::test]
    async fn run_actions_are_answered_from_a_tracked_task() {
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        let backlog = Origin::Tab(TabId("backlog"));

        for (seq, item) in [(4, ids::HTUI_ANA_2), (5, ItemId::new())] {
            let served = runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: backlog.clone(),
                        request: StoreRequest::RunActions(item),
                    },
                    &LiveChats::default(),
                )
                .await;
            assert!(matches!(served, RunServed::Deferred), "{served:?}");
            assert_eq!(
                runtime.tasks_len(),
                1,
                "the verdicts task is tracked, so settle and shutdown bound it"
            );
            assert!(runtime.settle(PATIENCE).await.is_empty());
            let answer = answers.try_recv().expect("the verdicts are answered");
            assert_eq!((&answer.origin, answer.seq), (&backlog, seq));
            assert!(answers.try_recv().is_err(), "answered once");
            match answer.reply {
                StoreReply::RunActions(actions) if seq == 4 => {
                    assert_eq!(actions.item, item);
                    assert_eq!(actions.run, Ok(()), "an open item may start a run");
                }
                StoreReply::Failed { request, .. } if seq == 5 => {
                    assert_eq!(request, "run_actions", "an unknown item is refused by name");
                }
                other => panic!("seq {seq}: {other:?}"),
            }
        }
    }

    /// The `research` phase as the ANA graph's snapshot carries it, for the author.
    fn research_phase() -> SnapshotPhase {
        SnapshotPhase {
            output_kind: "research".to_owned(),
            ..serde_json::from_value(json!({
                "position": 0, "name": "research", "fan_out": 1, "gate": "always",
                "gate_effective": "always", "gate_hard": false, "retry_limit": 1,
                "input_kinds": [], "output_kind": "research", "isolation": "worktree",
                "command_queue": "fan_out_only", "verify_command": null,
                "deadline_seconds": null, "template": { "name": "research", "version": 1 },
                "token_budget": null, "candidates": [], "judge": null
            }))
            .expect("a phase")
        }
    }

    /// D177 (R-25): a cleanup retry runs on a terminal run and is refused on a live one.
    #[tokio::test]
    async fn cleanup_retries_a_terminal_run() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (run, _) = parked(&fixture, &mut worker).await;

        let early = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Cleanup { run }),
        );
        assert_eq!(
            match worker.reply(early).await {
                StoreReply::Failed { message, .. } => message,
                other => panic!("a live run is refused: {other:?}"),
            },
            EngineError::NotTerminal {
                run,
                status: RunStatus::AwaitingApproval,
            }
            .to_string()
        );

        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run })),
        );
        outcome(worker.reply(cancel).await);
        let before = fixture.isolator.cleanups();
        let again = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Cleanup { run }),
        );
        assert!(matches!(
            worker.reply(again).await,
            StoreReply::Orch(OrchReply::CleanedUp { run: cleaned }) if cleaned == run
        ));
        assert_eq!(fixture.isolator.cleanups(), before + 1);
    }

    /// D156: the production isolator is built at the first command and kept while the repo map
    /// does not move.
    #[tokio::test]
    async fn the_isolator_is_built_once_per_process() {
        let fixture = Fixture::new().await;
        let scratch = tempfile::tempdir().expect("a scratch root");
        let mut runtime = RunRuntime::new(fixture.factory())
            .with_clock(Arc::new(TokioClock::new()))
            .with_scratch_root(scratch.path().to_path_buf());
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        assert_eq!(
            runtime.isolator_builds(),
            0,
            "nothing is built before a command"
        );

        for (seq, item) in [(1, ids::HTUI_ANA_2), (2, ids::AGY_FEAT_1)] {
            let served = runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: Origin::App,
                        request: start_run(item),
                    },
                    &LiveChats::default(),
                )
                .await;
            assert!(matches!(served, RunServed::Deferred));
            assert!(runtime.settle(PATIENCE).await.is_empty());
        }
        let mut answered = 0;
        while answers.try_recv().is_ok() {
            answered += 1;
        }
        assert!(answered >= 2, "both starts were answered");
        assert_eq!(runtime.isolator_builds(), 1);
        assert_eq!(
            runtime.verifier_builds(),
            1,
            "the limits did not move either"
        );
    }

    /// MOD-76 D4 (R-55, review L-2): a real walking `StartRun` reads its parts before it mints its
    /// walk, so an edit to the box's `command_limits` alone reaches the next one, with no restart:
    /// the verifier is rebuilt and the isolator kept.
    #[tokio::test]
    async fn a_walking_start_run_applies_a_command_limits_edit() {
        let fixture = Fixture::new().await;
        let scratch = tempfile::tempdir().expect("a scratch root");
        let mut runtime = RunRuntime::new(fixture.factory())
            .with_clock(Arc::new(TokioClock::new()))
            .with_scratch_root(scratch.path().to_path_buf());
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();

        for (seq, item) in [(1, ids::HTUI_ANA_2), (2, ids::AGY_FEAT_1)] {
            if seq == 2 {
                assert!(
                    fixture
                        .store
                        .set_box_setting(ids::BOX, "command_limits", json!({"verify": 2})),
                    "the demo box has a row"
                );
            }
            let served = runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: Origin::App,
                        request: start_run(item),
                    },
                    &LiveChats::default(),
                )
                .await;
            assert!(matches!(served, RunServed::Deferred));
            assert!(runtime.settle(PATIENCE).await.is_empty());
        }
        assert_eq!(
            runtime.verifier_builds(),
            2,
            "the edit rebuilt the verifier"
        );
        assert_eq!(runtime.isolator_builds(), 1, "and kept the isolator");
    }

    /// D216 (review L7): a failed `box_row` read is an error — `singletons` then caches no
    /// verifier and the next command reads again — while a box with no row, or no
    /// `command_limits`, or one that does not parse, gets `{"verify": 1}`.
    #[tokio::test]
    async fn command_limits_fail_with_the_store_and_default_a_missing_value() {
        let root = tempfile::tempdir().expect("a throwaway mirror root");
        let cache = CacheStore::open(root.path(), "run-worker-limits", PgStore::schema_version())
            .await
            .expect("the mirror opens");
        let offline = Backend::Offline {
            cache: cache.clone(),
            since: Some(Utc::now()),
        };
        assert!(
            testing::command_limits(&offline, ids::BOX).await.is_err(),
            "a read that fails is not the default"
        );
        cache.close().await;

        let default = BTreeMap::from([("verify".to_owned(), 1)]);
        let limits_of = async |stored: Option<serde_json::Value>| {
            let mut data = htui_core::fixtures::demo_data();
            let row = data
                .boxes
                .iter_mut()
                .find(|row| row.id == ids::BOX)
                .expect("the demo box");
            match stored {
                Some(limits) => row.settings["command_limits"] = limits,
                None => row.settings = json!({}),
            }
            let backend = Backend::memory(MemStore::from_demo(data));
            testing::command_limits(&backend, ids::BOX)
                .await
                .expect("the read answers")
        };
        let memory = Backend::memory(MemStore::demo());
        assert_eq!(
            testing::command_limits(&memory, BoxId::new())
                .await
                .expect("the read answers"),
            default,
            "no row"
        );
        assert_eq!(limits_of(None).await, default, "no key");
        assert_eq!(
            limits_of(Some(json!("many"))).await,
            default,
            "a value that does not parse"
        );
        assert_eq!(
            limits_of(Some(json!({ "test": 4, "verify": 3 }))).await,
            BTreeMap::from([("test".to_owned(), 4), ("verify".to_owned(), 3)]),
            "a stored map"
        );
    }

    /// A run of `item` another process claimed an hour ago (`started_at`) under a lease that
    /// lapsed at once: `running`, its lease expired, its owner not this one. No step was created.
    async fn stranded(fixture: &Fixture, item: ItemId) -> RunId {
        stranded_on(fixture, item, ids::BOX).await
    }

    /// [`stranded`] on `box_id`: targeted at it and claimed there (MOD-42 D12 step 4).
    async fn stranded_on(fixture: &Fixture, item: ItemId, box_id: BoxId) -> RunId {
        let backend = Backend::memory(fixture.store.clone());
        let row = fixture.item(item).await;
        let app = fixture.store.app_settings().await.expect("the settings");
        let resolved = htui_orch::resolve(
            &fixture.store,
            &HostGraphs(backend),
            &row,
            RunMode::Manual,
            &app,
            None,
            ids::BOX,
        )
        .await
        .expect("the item resolves");
        let past = Utc::now() - TimeDelta::hours(1);
        let run = RunId::new();
        fixture
            .store
            .create_run(NewRun {
                id: run,
                project_id: row.project_id,
                item_id: row.id,
                mode: RunMode::Manual,
                target_box_id: box_id,
                started_by: ids::USER,
                graph_snapshot: resolved.snapshot,
                repo_scope: resolved.repo_scope,
                queued_at: past,
            })
            .await
            .expect("the run lands");
        let claim = fixture
            .store
            .claim_run(run, box_id, Uuid::now_v7(), past, TimeDelta::zero())
            .await
            .expect("the claim answers");
        assert!(claim.is_admitted(), "{claim}");
        run
    }

    /// A `queued` run of `item` another process enqueued five minutes ago and never claimed.
    async fn queued_by_another(fixture: &Fixture, item: ItemId) -> RunId {
        let backend = Backend::memory(fixture.store.clone());
        let row = fixture.item(item).await;
        let app = fixture.store.app_settings().await.expect("the settings");
        let resolved = htui_orch::resolve(
            &fixture.store,
            &HostGraphs(backend),
            &row,
            RunMode::Manual,
            &app,
            None,
            ids::BOX,
        )
        .await
        .expect("the item resolves");
        let run = RunId::new();
        fixture
            .store
            .create_run(NewRun {
                id: run,
                project_id: row.project_id,
                item_id: row.id,
                mode: RunMode::Manual,
                target_box_id: ids::BOX,
                started_by: ids::USER,
                graph_snapshot: resolved.snapshot,
                repo_scope: resolved.repo_scope,
                queued_at: Utc::now() - TimeDelta::minutes(5),
            })
            .await
            .expect("the run lands");
        run
    }

    /// Polls the store until `run` reaches `status`.
    async fn rests_at(fixture: &Fixture, run: RunId, status: RunStatus) {
        rests_within(fixture, run, status, PATIENCE).await;
    }

    /// [`rests_at`] with a patience of its own: a paused-clock case waits out sweep periods.
    async fn rests_within(fixture: &Fixture, run: RunId, status: RunStatus, patience: Duration) {
        tokio::time::timeout(patience, async {
            while fixture.run(run).await.status != status {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("run {run} did not reach `{status}` within {patience:?}"));
    }

    /// D158: the startup sweep adopts every stranded run and resumes each on its own task, so one
    /// run's session never waits on another's.
    #[tokio::test]
    async fn the_sweep_resumes_each_adopted_run_on_its_own_task() {
        let fixture = Fixture::new().await;
        let first = stranded(&fixture, ids::HTUI_ANA_2).await;
        let second = stranded(&fixture, ids::AGY_FEAT_1).await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let _worker = Worker::spawn(&fixture.store, fixture.runtime());

        within("one resumed session starting", stall.reached.notified()).await;
        within("the other run resting while the first stalls", async {
            loop {
                let rested = [first, second].len()
                    - [fixture.run(first).await, fixture.run(second).await]
                        .iter()
                        .filter(|row| row.status == RunStatus::Running)
                        .count();
                if rested == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        stall.release.notify_one();
        rests_at(&fixture, first, RunStatus::AwaitingApproval).await;
        rests_at(&fixture, second, RunStatus::AwaitingApproval).await;
    }

    /// D189: a sweep leaves a run a command of this process holds alone — its lease renewed to
    /// this owner by the adoption, and the run left to the holder.
    #[tokio::test]
    async fn a_sweep_skips_a_run_a_command_holds() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let mut runtime = fixture.runtime();
        let held = testing::probe(&runtime)
            .try_lock(run)
            .expect("nobody holds it");
        let (replies, _answers) = mpsc::unbounded_channel();

        runtime.sweep(&Backend::memory(fixture.store.clone()), &replies);
        assert!(runtime.settle(PATIENCE).await.is_empty());

        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Running, "not recovered");
        assert!(fixture.steps(run).await.is_empty(), "nothing walked");
        assert!(
            row.lease_expires_at.is_some_and(|until| until > Utc::now()),
            "the adoption leased it to this owner: {:?}",
            row.lease_expires_at
        );
        assert!(
            testing::probe(&runtime).is_dead_walk(run),
            "and the next free sweep gives it back"
        );
        drop(held);
    }

    /// R-12, D158: a walk task that panics is marked dead, answered once with the sentence, and
    /// the next sweep releases and adopts its run, which a healthy session then rests.
    #[tokio::test]
    async fn a_panicked_walk_is_adopted_by_the_next_sweep() {
        let fixture = Fixture::new().await;
        fixture.sessions.push(Play::Panic);
        let mut worker = Worker::spawn(
            &fixture.store,
            fixture
                .runtime()
                .with_sweep_every(Duration::from_millis(100)),
        );

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        assert!(
            matches!(worker.reply(start).await, StoreReply::Failed { request: "start_run", ref message } if message == WALK_PANICKED),
            "the panicked task's request is answered"
        );
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        rests_at(&fixture, run, RunStatus::AwaitingApproval).await;
    }

    /// The scripted agent's id in the fixture's registry.
    async fn scripted_agent(fixture: &Fixture) -> AgentId {
        fixture
            .store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.enabled)
            .expect("the fixture enables exactly the scripted row")
            .agent
            .id
    }

    /// MOD-23 re-review Low-1: a step admitted while its agent was on, still `pending` when the
    /// agent is switched off on this box, is not driven on it: `Kit::driver` answers the switch's
    /// refusal, so the step and the run fail as for any driver that refuses to start, with the
    /// switch's sentence, and the agent is never built.
    ///
    /// The step is written by hand under a stranded run: a walk that admitted it and died before
    /// driving it, which the startup sweep adopts and walks from the step's own `agent_id`.
    #[tokio::test]
    async fn a_pending_step_on_a_row_switched_off_since_admission_is_refused_unspawned() {
        let fixture = Fixture::new().await;
        let agent = scripted_agent(&fixture).await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let step = fixture
            .store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index: 0,
                phase_name: "research".to_owned(),
                agent_id: Some(agent),
                model: Some("sonnet".to_owned()),
            })
            .await
            .expect("the admitted step lands");
        fixture
            .store
            .set_agent_box_enabled(agent, ids::BOX, false)
            .await
            .expect("the switch lands");
        // A build pops this, so a queue still holding it is a driver never built.
        fixture.sessions.push(Play::Done);

        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        rests_at(&fixture, run, RunStatus::Failed).await;

        let sentence = "agent `scripted` is switched off on this box; Settings > Agents, t switches \
                        it on";
        assert_eq!(
            fixture.run(run).await.failure,
            Some(format!("agent spawn failed: {sentence}")),
            "the run fails with the switch's sentence"
        );
        let steps = fixture.steps(run).await;
        assert_eq!(steps.len(), 1, "nothing else was admitted: {steps:?}");
        assert_eq!(steps[0].id, step.id);
        assert_eq!(
            steps[0].status,
            StepStatus::Failed,
            "the step is failed, not lost"
        );
        assert_eq!(
            fixture.sessions.0.lock().expect("the queue").len(),
            1,
            "the switched-off agent was never built"
        );

        // A failed run is terminal: switching the agent back on does not resume it, and a retry of
        // its step is refused. The item takes a fresh run.
        fixture
            .store
            .set_agent_box_enabled(agent, ids::BOX, true)
            .await
            .expect("the switch lands");
        let retry = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::RetryStep {
                run,
                step: step.id,
            })),
        );
        assert!(
            matches!(
                worker.reply(retry).await,
                StoreReply::Failed {
                    request: "retry_step",
                    ..
                }
            ),
            "a terminal run takes no retry"
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Failed);
        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        assert!(
            matches!(
                outcome(worker.reply(start).await),
                CommandOutcome::Started { .. }
            ),
            "the item starts a fresh run on the agent switched back on"
        );
    }

    /// Blueprint §8.9: `settle` aborts a task that outlived its limit — the walk itself, not only
    /// its supervisor — so the stuck session is dropped and the run's lock is free again.
    #[tokio::test]
    async fn settle_aborts_the_walk_of_a_stuck_task() {
        let fixture = Fixture::new().await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred));
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;

        assert_eq!(runtime.settle(Duration::from_millis(100)).await, [run]);
        within("the stuck walk being dropped", async {
            while !stall.dropped.load(Ordering::SeqCst)
                || testing::probe(&runtime).try_lock(run).is_none()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
    }

    /// The UI is gone: a command spawned just before `shutdown` walks nothing once it runs — it
    /// is refused before it enqueues — and a sweep asked for after it spawns nothing.
    #[tokio::test]
    async fn a_command_that_outlives_shutdown_walks_nothing() {
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred));

        runtime.shutdown(Duration::from_secs(1)).await;
        let answer = answers.try_recv().expect("the start is answered");
        assert!(
            matches!(&answer.reply, StoreReply::Failed { request: "start_run", message } if message == PREEMPTED),
            "{:?}",
            answer.reply
        );
        assert!(
            fixture
                .store
                .runs(ids::HTUI_ANA_2)
                .await
                .expect("the read answers")
                .is_empty(),
            "nothing was enqueued"
        );

        runtime.sweep(&backend, &replies);
        assert_eq!(runtime.tasks_len(), 0, "a closed runtime sweeps nothing");
    }

    /// D214 (review L5): every sweep tick prunes, in one pass, the finished task handles, the
    /// parents of runs no task works on, and the locks nobody holds or waits for. A parent a task
    /// still works under and a held lock stay.
    #[tokio::test]
    async fn a_sweep_prunes_finished_tasks_idle_parents_and_free_locks() {
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();
        let envelope = |seq: u64, request: StoreRequest| RequestEnvelope {
            seq,
            origin: Origin::App,
            request,
        };
        runtime
            .serve(
                &backend,
                &replies,
                &envelope(1, start_run(ids::HTUI_ANA_2)),
                &LiveChats::default(),
            )
            .await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        runtime
            .serve(
                &backend,
                &replies,
                &envelope(
                    2,
                    StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run })),
                ),
                &LiveChats::default(),
            )
            .await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        let probe = testing::probe(&runtime);
        assert!(probe.has_parent(run), "the run's parent outlived its tasks");
        assert!(probe.has_lock_entry(run), "and so did its lock");
        let other = RunId::new();
        let held = probe.try_lock(other).expect("nobody holds it");
        let walking = probe.walk_child(other);

        runtime.sweep(&backend, &replies);
        within("the first sweep ending", async {
            while !probe.all_tasks_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        runtime.sweep(&backend, &replies);

        assert_eq!(
            runtime.tasks_len(),
            1,
            "the finished sweep's handle was pruned; the new one is tracked"
        );
        assert!(!probe.has_parent(run), "the idle parent was pruned");
        assert!(probe.has_parent(other), "a live one stays");
        assert!(!probe.has_lock_entry(run), "the free lock was pruned");
        assert!(probe.has_lock_entry(other), "a held one stays");
        drop((held, walking));
    }

    /// `shutdown` gives every task one shared window of `2 × grace`, not one each: the chats are
    /// cancelled beside it inside the same bounded quit (`docs/ANA-4.md` §11 criterion 11).
    #[tokio::test(start_paused = true)]
    async fn shutdown_gives_every_task_one_shared_window() {
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let (replies, _answers) = mpsc::unbounded_channel();
        let ctx = testing::probe(&runtime).task_ctx(
            Backend::memory(fixture.store.clone()),
            TuiReplies(replies),
            "stuck",
        );
        for _ in 0..3 {
            testing::spawn_supervised(testing::unaddressed(&ctx, "stuck"), std::future::pending());
        }

        let grace = Duration::from_secs(2);
        let began = tokio::time::Instant::now();
        runtime.shutdown(grace).await;
        assert!(
            began.elapsed() <= grace * 2,
            "three stuck tasks share one window: {:?}",
            began.elapsed()
        );
        assert_eq!(runtime.tasks_len(), 0);
    }

    /// D158: a resume that cannot read its parts leaves its run to the next sweep — the run joins
    /// the dead walks, whose pre-pass gives the adopted lease back — and the refusal is published
    /// for the run's item.
    #[tokio::test]
    async fn a_resume_that_cannot_read_its_parts_leaves_the_run_to_the_next_sweep() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        // A scratch root under a regular file cannot be created, so the production isolator,
        // and with it every part the resume reads, is refused.
        let scratch = tempfile::NamedTempFile::new().expect("a regular file");
        let runtime: RunRuntime = RunRuntime::new(fixture.factory())
            .with_clock(Arc::new(TokioClock::new()))
            .with_scratch_root(scratch.path().join("trees"));
        let (replies, mut answers) = mpsc::unbounded_channel();
        let backlog = Origin::Tab(TabId("backlog"));
        let probe = testing::probe(&runtime);
        probe.wire(&TuiReplies(replies.clone()));
        probe.subscribe(
            crate::agent_worker::ReplyAddr {
                seq: 3,
                origin: backlog.clone(),
            },
            ids::HTUI_ANA_2,
        );
        let ctx = probe.task_ctx(
            Backend::memory(fixture.store.clone()),
            TuiReplies(replies),
            "resume",
        );

        testing::resumed(ctx, run).await;
        assert!(
            probe.is_dead_walk(run),
            "the next sweep gives the adopted lease back"
        );
        let frame = answers.try_recv().expect("the refusal is published");
        assert_eq!((&frame.origin, frame.seq), (&backlog, 3));
        assert!(
            matches!(&frame.reply, StoreReply::RunStream(super::RunFrame { item, run: Some(of), kind: FrameKind::Error(_) })
                if *item == ids::HTUI_ANA_2 && *of == run),
            "{:?}",
            frame.reply
        );
        assert_eq!(
            runtime.isolator_builds(),
            0,
            "the isolator was refused, not built"
        );
    }

    /// M5 D84: only a run read to be resting triggers the claim retry. A status read that fails —
    /// the store is gone — retries nothing, so a queued run does not keep retrying itself for the
    /// whole outage.
    #[tokio::test]
    async fn a_failed_status_read_retries_no_claim() {
        let root = tempfile::tempdir().expect("a throwaway mirror root");
        let cache = CacheStore::open(root.path(), "run-worker-retry", PgStore::schema_version())
            .await
            .expect("the mirror opens");
        cache.close().await;
        let backend = Backend::Offline {
            cache,
            since: Some(Utc::now()),
        };
        let ended = RunId::new();
        assert!(backend.run(ended).await.is_err(), "every read fails");

        let fixture = Fixture::new().await;
        let runtime = fixture.runtime();
        let (replies, _answers) = mpsc::unbounded_channel();
        let probe = testing::probe(&runtime);
        let ctx = probe.task_ctx(backend, TuiReplies(replies), "walk");
        testing::set_task_run(&ctx, ended);
        let waiting = RunId::new();
        probe.queue(Utc::now(), waiting);

        testing::retry_claims(&ctx).await;
        assert_eq!(runtime.tasks_len(), 0, "no retry was spawned");
        assert!(
            probe.queued().iter().any(|(_, run)| *run == waiting),
            "the run still waits"
        );
    }

    /// M5 D84: a run whose claim was refused waits, and is claimed again once a walk of this
    /// process rests.
    #[tokio::test]
    async fn a_refused_claim_is_retried_when_a_walk_rests() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (first, _) = parked(&fixture, &mut worker).await;

        let start = worker.send(Origin::App, start_run(ids::HTUI_CLEAN_1));
        let StoreReply::Failed { message, .. } = worker.reply(start).await else {
            panic!("the second claim is refused");
        };
        assert!(
            message.contains("overlaps run"),
            "both runs scope the primary repo: {message}"
        );
        let second = only_run(&fixture.store, ids::HTUI_CLEAN_1).await;
        assert_eq!(fixture.run(second).await.status, RunStatus::Queued);

        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run: first })),
        );
        outcome(worker.reply(cancel).await);
        rests_at(&fixture, second, RunStatus::AwaitingApproval).await;
    }

    /// M5 D84, blueprint §8.6: queued runs are claimed again in `queued_at` order — a later run
    /// is not claimed while an earlier one's claim is undecided, so the earlier run gets the scope
    /// a rested walk freed.
    #[tokio::test]
    async fn queued_runs_are_claimed_again_in_queued_at_order() {
        let fixture = Fixture::new().await;
        let runtime = fixture.runtime();
        let shared = testing::probe(&runtime);
        let mut worker = Worker::spawn(&fixture.store, runtime);
        let (first, _) = parked(&fixture, &mut worker).await;
        // A third item of the project that may start a run.
        assert!(
            fixture
                .store
                .transition(ids::HTUI_FEAT_2, Status::Blocked, Status::Open)
                .await
                .expect("the reopen answers")
        );
        let mut queued = Vec::new();
        for item in [ids::HTUI_CLEAN_1, ids::HTUI_FEAT_2] {
            let start = worker.send(Origin::App, start_run(item));
            let StoreReply::Failed { message, .. } = worker.reply(start).await else {
                panic!("the claim is refused");
            };
            assert!(message.contains("overlaps run"), "{message}");
            queued.push(only_run(&fixture.store, item).await);
        }
        let [earlier, later] = queued[..] else {
            unreachable!("two starts")
        };
        let held = within("both runs waiting, the earlier one unheld", async {
            loop {
                let waiting = shared
                    .queued()
                    .iter()
                    .filter(|(_, run)| *run == earlier || *run == later)
                    .count();
                if waiting == 2
                    && let Some(held) = shared.try_lock(earlier)
                {
                    break held;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run: first })),
        );
        outcome(worker.reply(cancel).await);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            fixture.run(later).await.status,
            RunStatus::Queued,
            "the later run waits for the earlier one's claim"
        );

        drop(held);
        rests_at(&fixture, earlier, RunStatus::AwaitingApproval).await;
        assert_eq!(
            fixture.run(later).await.status,
            RunStatus::Queued,
            "and the earlier run's scope refuses it"
        );
    }

    /// `SetDsn` (MOD-15 D11 step 6): a walk belongs to the server it was claimed on, so a switch
    /// preempts it — the lease goes back to that server through `abandoned` — and the claim
    /// queue, the old server's runs, is forgotten.
    #[tokio::test]
    async fn a_server_switch_preempts_every_walk_and_forgets_the_queue() {
        let fixture = Fixture::new().await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred));
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        testing::probe(&runtime).queue(Utc::now(), RunId::new());

        runtime.forget_server();
        let answer = within("the old walk stopping", answers.recv())
            .await
            .expect("the runtime answers");
        assert!(
            matches!(&answer.reply, StoreReply::Failed { message, .. } if message == PREEMPTED),
            "{:?}",
            answer.reply
        );
        assert!(
            stall.dropped.load(Ordering::SeqCst),
            "the session was dropped"
        );
        assert!(
            fixture
                .run(run)
                .await
                .lease_expires_at
                .is_some_and(|until| until <= Utc::now()),
            "the lease went back to the server the walk was claimed on"
        );
        assert!(
            testing::probe(&runtime).queued().is_empty(),
            "the old server's queued runs are forgotten"
        );
    }

    /// A server switch forgets the production isolator, built from the old server's repo map and
    /// box: the new server's first command builds its own.
    #[tokio::test]
    async fn a_server_switch_rebuilds_the_isolator() {
        let fixture = Fixture::new().await;
        let scratch = tempfile::tempdir().expect("a scratch root");
        let mut runtime = RunRuntime::new(fixture.factory())
            .with_clock(Arc::new(TokioClock::new()))
            .with_scratch_root(scratch.path().to_path_buf());
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();

        for (seq, item) in [(1, ids::HTUI_ANA_2), (2, ids::AGY_FEAT_1)] {
            runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: Origin::App,
                        request: start_run(item),
                    },
                    &LiveChats::default(),
                )
                .await;
            assert!(runtime.settle(PATIENCE).await.is_empty());
            runtime.forget_server();
        }
        assert_eq!(runtime.isolator_builds(), 2);
    }

    /// The frames (not the acknowledgements) received so far, taken out of `seen`.
    fn frames(worker: &mut Worker) -> Vec<(Origin, u64, super::RunFrame)> {
        worker.drain();
        let (frames, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut worker.seen)
            .into_iter()
            .partition(|envelope| {
                matches!(&envelope.reply, StoreReply::RunStream(frame)
                    if !matches!(frame.kind, FrameKind::Subscribed))
            });
        worker.seen = rest;
        frames
            .into_iter()
            .map(|envelope| match envelope.reply {
                StoreReply::RunStream(frame) => (envelope.origin, envelope.seq, frame),
                _ => unreachable!("partitioned above"),
            })
            .collect()
    }

    /// Blueprint §0a point 3: every frame of an item goes to its subscribers at **their**
    /// subscription's `seq`, whoever asked for the command, and a re-subscription moves it.
    #[tokio::test]
    async fn run_stream_frames_carry_the_subscription_seq() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let backlog = Origin::Tab(TabId("backlog"));
        let chat = Origin::Tab(TabId("chat"));
        worker.send_at(
            backlog.clone(),
            7,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );
        worker.send_at(
            chat.clone(),
            9,
            StoreRequest::RunStream {
                item: ids::HTUI_FEAT_1,
            },
        );

        let start = worker.send_at(Origin::App, 10, start_run(ids::HTUI_ANA_2));
        let CommandOutcome::Started { run, .. } = outcome(worker.reply(start).await) else {
            panic!("a start answers Started");
        };
        let acks: Vec<(Origin, u64)> = worker
            .seen
            .iter()
            .filter(|envelope| {
                matches!(&envelope.reply, StoreReply::RunStream(frame)
                if matches!(frame.kind, FrameKind::Subscribed))
            })
            .map(|envelope| (envelope.origin.clone(), envelope.seq))
            .collect();
        assert_eq!(acks, [(backlog.clone(), 7), (chat.clone(), 9)]);

        let first = frames(&mut worker);
        assert!(
            first
                .iter()
                .any(|(_, _, frame)| matches!(frame.kind, FrameKind::Started)),
            "{first:?}"
        );
        assert!(
            first
                .iter()
                .any(|(_, _, frame)| matches!(frame.kind, FrameKind::SessionDone { .. })),
            "{first:?}"
        );
        assert!(
            first
                .iter()
                .any(|(_, _, frame)| matches!(frame.kind, FrameKind::Rested(_))),
            "{first:?}"
        );
        for (origin, seq, frame) in &first {
            assert_eq!((origin, *seq), (&backlog, 7), "{frame:?}");
            assert_eq!(frame.item, ids::HTUI_ANA_2);
        }

        worker.send_at(
            backlog.clone(),
            11,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );
        let research = step_at(&fixture, run, 0).await;
        let answer = worker.send_at(
            Origin::App,
            12,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        );
        outcome(worker.reply(answer).await);
        let later = frames(&mut worker);
        assert!(!later.is_empty());
        for (origin, seq, frame) in &later {
            assert_eq!((origin, *seq), (&backlog, 11), "{frame:?}");
        }
    }

    /// D200: a command that changes an item's rows with no walk — a reopen, a close-out — still
    /// publishes one frame for the item, so its Runs pane re-reads.
    #[tokio::test]
    async fn a_command_with_no_walk_still_publishes_a_frame() {
        for command in [
            Command::Unblock {
                item: ids::HTUI_FEAT_2,
            },
            Command::CloseOut {
                item: ids::HTUI_FEAT_2,
                resolution: Resolution::Withdrawn,
            },
        ] {
            let fixture = Fixture::new().await;
            let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
            let backlog = Origin::Tab(TabId("backlog"));
            worker.send_at(
                backlog.clone(),
                5,
                StoreRequest::RunStream {
                    item: ids::HTUI_FEAT_2,
                },
            );
            let request = StoreRequest::Orch(OrchRequest::Command(command));
            let name = request.name();
            let asked = worker.send(Origin::App, request);
            let done = outcome(worker.reply(asked).await);
            assert!(
                matches!(
                    done,
                    CommandOutcome::Unblocked { rest: None, .. } | CommandOutcome::ClosedOut { .. }
                ),
                "{name}: {done:?}"
            );
            let published = frames(&mut worker);
            assert_eq!(published.len(), 1, "{name}: {published:?}");
            let (origin, seq, frame) = &published[0];
            assert_eq!((origin, *seq), (&backlog, 5));
            assert_eq!(frame.item, ids::HTUI_FEAT_2);
            assert!(
                matches!(frame.kind, FrameKind::Changed),
                "{name}: {frame:?}"
            );
        }
    }

    /// D200: a command preempted while it waits for its run's lock publishes its refusal for the
    /// run's item, like the walk it waited behind.
    #[tokio::test]
    async fn a_command_preempted_in_the_lock_queue_publishes_its_refusal() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let backlog = Origin::Tab(TabId("backlog"));
        worker.send_at(
            backlog,
            1,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );
        let (run, research) = parked(&fixture, &mut worker).await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let retry = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::RetryStep {
                run,
                step: research.id,
            })),
        );
        within("attempt 2's session starting", stall.reached.notified()).await;
        let answer = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        frames(&mut worker);

        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run })),
        );
        outcome(worker.reply(cancel).await);
        for seq in [retry, answer] {
            assert!(
                matches!(worker.reply(seq).await, StoreReply::Failed { ref message, .. } if message == PREEMPTED)
            );
        }
        let refusals = frames(&mut worker)
            .into_iter()
            .filter(|(_, _, frame)| matches!(&frame.kind, FrameKind::Error(sentence) if sentence == PREEMPTED))
            .count();
        assert_eq!(
            refusals, 2,
            "the stopped walk and the command queued behind it"
        );
    }

    /// R-51: a command queued behind a live walk announces the wait with one `Waiting` frame, and
    /// the walk has not rested.
    #[tokio::test]
    async fn a_command_queued_behind_a_live_walk_publishes_a_waiting_frame() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        worker.send_at(
            Origin::Tab(TabId("backlog")),
            1,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );
        let (run, research) = parked(&fixture, &mut worker).await;
        frames(&mut worker);
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::RetryStep {
                run,
                step: research.id,
            })),
        );
        within("attempt 2's session starting", stall.reached.notified()).await;
        worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        let seen = frames(&mut worker);
        let waiting = seen
            .iter()
            .filter(|(_, _, frame)| {
                frame.run == Some(run) && matches!(frame.kind, FrameKind::Waiting)
            })
            .count();
        assert_eq!(waiting, 1, "one Waiting frame for the queued command");
        assert!(
            !seen
                .iter()
                .any(|(_, _, frame)| matches!(frame.kind, FrameKind::Rested(_))),
            "the walk has not rested"
        );
    }

    /// R-40: a step that goes live publishes a `Changed` frame at once, while its session is still
    /// running, and not only when the walk rests. The pane re-reads on `Changed`, so no new kind.
    #[tokio::test]
    async fn a_step_that_goes_live_publishes_a_frame_before_its_session_ends() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        worker.send_at(
            Origin::Tab(TabId("backlog")),
            1,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );
        let (run, research) = parked(&fixture, &mut worker).await;
        frames(&mut worker);
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::RetryStep {
                run,
                step: research.id,
            })),
        );
        within("attempt 2's session starting", stall.reached.notified()).await;
        let live = frames(&mut worker);
        assert!(
            live.iter()
                .any(|(_, _, frame)| frame.run == Some(run)
                    && matches!(frame.kind, FrameKind::Changed)),
            "the step went live and said so: {live:?}"
        );
    }

    /// D174, PRD `:197`: off the server every command is refused with MOD-25's one sentence and
    /// nothing is spawned, while the stream and the verdicts still answer from the mirror.
    #[tokio::test]
    async fn an_offline_backend_refuses_every_orch_request_with_one_sentence() {
        let root = tempfile::tempdir().expect("a throwaway mirror root");
        let cache = CacheStore::open(root.path(), "run-worker-offline", PgStore::schema_version())
            .await
            .expect("the mirror opens");
        htui_store::testkit::seed_mirror(&cache, &htui_core::fixtures::demo_data())
            .await
            .expect("the mirror is seeded");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: Some(Utc::now()),
        };
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let (replies, mut answers) = mpsc::unbounded_channel();
        let serve = async |runtime: &mut RunRuntime, seq: u64, request: StoreRequest| {
            runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: Origin::App,
                        request,
                    },
                    &LiveChats::default(),
                )
                .await
        };

        for (seq, request) in (1..).zip(every_orch_request()) {
            let name = request.name();
            let served = serve(&mut runtime, seq, StoreRequest::Orch(request)).await;
            assert!(
                matches!(&served, RunServed::Reply(RunReply::Failed { request, message })
                    if *request == name && message == DATABASE_UNREACHABLE),
                "{served:?}"
            );
        }
        assert_eq!(runtime.tasks_len(), 0, "nothing was spawned");
        assert_eq!(runtime.isolator_builds(), 0, "and nothing was built");

        let served = serve(
            &mut runtime,
            20,
            StoreRequest::RunStream {
                item: ids::HTUI_FEAT_1,
            },
        )
        .await;
        assert!(matches!(
            served,
            RunServed::Reply(RunReply::Frame(super::RunFrame {
                kind: FrameKind::Subscribed,
                ..
            }))
        ));

        let served = serve(&mut runtime, 21, StoreRequest::RunActions(ids::HTUI_FEAT_1)).await;
        assert!(matches!(served, RunServed::Deferred), "{served:?}");
        assert!(runtime.settle(PATIENCE).await.is_empty());
        let answer = answers.try_recv().expect("the verdicts are answered");
        assert_eq!(answer.seq, 21);
        let StoreReply::RunActions(actions) = answer.reply else {
            panic!("the verdicts are read from the mirror: {:?}", answer.reply);
        };
        let offline = Err(DATABASE_UNREACHABLE.to_owned());
        assert_eq!(actions.item, ids::HTUI_FEAT_1);
        assert_eq!(
            (&actions.run, &actions.unblock, &actions.close_out),
            (&offline, &offline, &offline)
        );
        for verdict in actions.runs.values() {
            assert_eq!((&verdict.cancel, &verdict.cleanup), (&offline, &offline));
        }
        for verdict in actions.steps.values() {
            for enabled in [
                &verdict.approve,
                &verdict.reject,
                &verdict.retry,
                &verdict.promote,
                &verdict.accept,
                &verdict.select,
            ] {
                assert_eq!(enabled, &offline);
            }
        }
        cache.close().await;
    }

    /// D175 (criterion 19 re-scoped): a store that stops answering lease refreshes fences the walk
    /// — its run joins the dead walks with a `LeaseLost` frame — and once the store answers again
    /// the next sweep adopts it and a second attempt rests it.
    #[tokio::test(start_paused = true)]
    async fn a_store_outage_fences_the_walk_and_the_sweep_adopts_it_after() {
        let fixture = Fixture::new().await;
        fixture
            .store
            .set_app_setting("lease_ttl_seconds", json!(30));
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let runtime = fixture.runtime();
        let shared = testing::probe(&runtime);
        let mut worker = Worker::spawn(&fixture.store, runtime);
        let backlog = Origin::Tab(TabId("backlog"));
        worker.send_at(
            backlog,
            1,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        fixture.store.set_fault(MemFault::RefreshLease, true);

        let StoreReply::Failed { message, .. } = worker
            .envelope_within(start, Duration::from_secs(600))
            .await
            .reply
        else {
            panic!("a fenced walk is refused");
        };
        fixture.store.set_fault(MemFault::RefreshLease, false);
        assert!(message.contains("lease"), "{message}");
        assert!(shared.is_dead_walk(run), "the run is a dead walk");
        assert!(
            frames(&mut worker).iter().any(
                |(_, _, frame)| matches!(&frame.kind, FrameKind::Error(sentence) if *sentence == message)
            ),
            "the fence is published"
        );
        assert!(stall.dropped.load(Ordering::SeqCst), "the walk was dropped");

        rests_within(
            &fixture,
            run,
            RunStatus::AwaitingApproval,
            Duration::from_secs(600),
        )
        .await;
        assert_eq!(
            step_at(&fixture, run, 0).await.attempt,
            2,
            "the adopted run walked a second attempt"
        );
        assert!(!shared.is_dead_walk(run));
    }

    /// MOD-37 M4 D3 (R-46): the store loop's `Online → Offline` swap preempts the walk at once,
    /// where the lease fence alone waits out `ttl - refresh`. Its release fails, the server being
    /// gone, so the run joins the dead walks and the next sweep adopts it for a second attempt.
    #[tokio::test(start_paused = true)]
    async fn an_offline_swap_preempts_the_walk_and_the_sweep_adopts_it() {
        let fixture = Fixture::new().await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred));
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        // The server is gone: the preempted walk cannot give its lease back.
        fixture.store.set_fault(MemFault::ReleaseLease, true);

        let t0 = tokio::time::Instant::now();
        runtime.preempt_walks();
        let answer = within("the walk stopping", answers.recv())
            .await
            .expect("the runtime answers");
        assert!(
            matches!(&answer.reply, StoreReply::Failed { message, .. } if message == PREEMPTED),
            "{:?}",
            answer.reply
        );
        assert!(
            t0.elapsed() < Duration::from_secs(1),
            "at once, not at the lease fence: {:?}",
            t0.elapsed()
        );
        assert!(
            stall.dropped.load(Ordering::SeqCst),
            "the session was dropped"
        );
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        assert!(
            testing::probe(&runtime).is_dead_walk(run),
            "a release that failed offline leaves a dead walk"
        );

        fixture.store.set_fault(MemFault::ReleaseLease, false);
        swept(&mut runtime, &fixture).await;
        rests_at(&fixture, run, RunStatus::AwaitingApproval).await;
        assert_eq!(
            step_at(&fixture, run, 0).await.attempt,
            2,
            "the adopted run walked a second attempt"
        );
        assert!(!testing::probe(&runtime).is_dead_walk(run));
    }

    /// MOD-37 M4 D3: unlike [`RunRuntime::forget_server`], the offline swap keeps the server: the
    /// claim queue stays, and the next command builds no new isolator. A walk's child token held
    /// across each `preempt_walks` makes it a real preempt (review L6): the walk goes, the
    /// isolator stays.
    #[tokio::test]
    async fn an_offline_swap_keeps_the_server() {
        let fixture = Fixture::new().await;
        let scratch = tempfile::tempdir().expect("a scratch root");
        let mut runtime = RunRuntime::new(fixture.factory())
            .with_clock(Arc::new(TokioClock::new()))
            .with_scratch_root(scratch.path().to_path_buf());
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();

        for (seq, item) in [(1, ids::HTUI_ANA_2), (2, ids::AGY_FEAT_1)] {
            runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: Origin::App,
                        request: start_run(item),
                    },
                    &LiveChats::default(),
                )
                .await;
            assert!(runtime.settle(PATIENCE).await.is_empty());
            let walked = RunId::new();
            let _walk = testing::probe(&runtime).walk_child(walked);
            assert!(testing::probe(&runtime).has_parent(walked));
            runtime.preempt_walks();
            assert!(
                !testing::probe(&runtime).has_parent(walked),
                "the swap preempted the live walk"
            );
        }
        assert_eq!(
            runtime.isolator_builds(),
            1,
            "the same server's isolator serves the second command"
        );

        let queued = RunId::new();
        let queued_at = Utc::now();
        testing::probe(&runtime).queue(queued_at, queued);
        runtime.preempt_walks();
        assert!(
            testing::probe(&runtime)
                .queued()
                .contains(&(queued_at, queued)),
            "the claim queue is kept"
        );
    }

    /// Plan D155: the five trait reads are the inherent reads of the same name.
    #[tokio::test]
    async fn backend_graphs_delegates_each_read() {
        let (store, agent) = seeded_store().await;
        let backend = Backend::memory(store);
        let graphs = HostGraphs(backend.clone());

        let resolved = GraphSource::resolve_graph(&graphs, ids::HTUI_ANA_2)
            .await
            .expect("the read answers");
        assert_eq!(
            resolved,
            backend
                .resolve_graph(ids::HTUI_ANA_2)
                .await
                .expect("the read answers")
        );
        let resolved = resolved.expect("the demo item has a graph");
        let phase = resolved.phases.first().expect("the graph has a phase");

        assert_eq!(
            GraphSource::phase_agents(&graphs, phase.phase.id)
                .await
                .expect("the read answers"),
            backend
                .phase_agents(phase.phase.id)
                .await
                .expect("the read answers")
        );
        assert_eq!(
            GraphSource::prompt_template(&graphs, ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers"),
            backend
                .prompt_template(ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers")
        );
        let row = GraphSource::agent(&graphs, agent)
            .await
            .expect("the read answers")
            .expect("the scripted row is registered");
        assert_eq!(row.name, "scripted");
        assert_eq!(
            GraphSource::agent(&graphs, AgentId::new())
                .await
                .expect("the read answers"),
            None,
            "an unknown id is no row"
        );
        let boxes = GraphSource::agent_boxes(&graphs, ids::BOX)
            .await
            .expect("the read answers");
        assert_eq!(
            boxes,
            backend
                .agent_boxes(ids::BOX)
                .await
                .expect("the read answers")
        );
        assert!(boxes.iter().any(|row| row.agent_id == agent));
    }

    /// One request per entry of `ORCH_NAMES`, in its order.
    fn every_orch_request() -> Vec<OrchRequest> {
        let (run, step) = (RunId::new(), StepId::new());
        vec![
            OrchRequest::Command(Command::StartRun {
                item: ids::HTUI_ANA_2,
                mode: RunMode::Manual,
                repo_scope: None,
            }),
            OrchRequest::Command(Command::AnswerGate {
                run,
                step,
                answer: GateAnswer::Approved,
            }),
            OrchRequest::Command(Command::RetryStep { run, step }),
            OrchRequest::Command(Command::CancelRun { run }),
            OrchRequest::Command(Command::SelectFanout {
                run,
                position: 0,
                attempt: 1,
                winner: step,
            }),
            OrchRequest::Command(Command::PromoteStep {
                run,
                step,
                chat_open: false,
            }),
            OrchRequest::Command(Command::AcceptArtifact {
                run,
                step,
                chat_live: false,
            }),
            OrchRequest::Command(Command::Unblock {
                item: ids::HTUI_ANA_2,
            }),
            OrchRequest::Command(Command::CloseOut {
                item: ids::HTUI_ANA_2,
                resolution: Resolution::Done,
            }),
            OrchRequest::CloseOutPreview {
                item: ids::HTUI_ANA_2,
            },
            OrchRequest::Cleanup { run },
        ]
    }

    /// Blueprint D209, §12: eleven distinct names, each the `name()` of its request, none shared
    /// with another `StoreRequest`.
    #[test]
    fn orch_names_are_eleven_distinct_request_names() {
        let requests = every_orch_request();
        assert_eq!(requests.len(), ORCH_NAMES.len());
        for (request, name) in requests.into_iter().zip(ORCH_NAMES) {
            assert_eq!(StoreRequest::Orch(request).name(), name);
        }
        let distinct: BTreeSet<&str> = ORCH_NAMES.into_iter().collect();
        assert_eq!(distinct.len(), 11);
        for other in [
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
            StoreRequest::Document(DocumentId::new()),
            StoreRequest::RunActions(ids::HTUI_ANA_2),
            StoreRequest::Runs(ids::HTUI_ANA_2),
        ] {
            assert!(!distinct.contains(other.name()), "{}", other.name());
        }
    }

    /// MOD-41 plan D7: a [`RunRequest`] is named exactly as the `StoreRequest` it was cut from, so
    /// a `Failed` reply's `request` still matches the Runs pane's and the Chat tab's lists.
    #[test]
    fn run_request_names_match_store_request_names() {
        let item = ids::HTUI_ANA_2;
        let mut pairs: Vec<(RunRequest, StoreRequest)> = every_orch_request()
            .into_iter()
            .map(|request| {
                (
                    RunRequest::Orch(request.clone()),
                    StoreRequest::Orch(request),
                )
            })
            .collect();
        pairs.push((
            RunRequest::Stream { item },
            StoreRequest::RunStream { item },
        ));
        pairs.push((RunRequest::Actions(item), StoreRequest::RunActions(item)));
        assert_eq!(pairs.len(), ORCH_NAMES.len() + 2);
        for (run, store) in pairs {
            assert_eq!(run.name(), store.name(), "{run:?}");
        }
    }

    /// MOD-41 plan D7: each [`RunReply`] reaches the loop as its one `StoreReply`, at the
    /// address's own `seq` and origin.
    #[test]
    fn tui_replies_map_every_run_reply() {
        let (replies, mut answers) = mpsc::unbounded_channel();
        let sink = TuiReplies(replies);
        let to = crate::agent_worker::ReplyAddr {
            seq: 7,
            origin: Origin::App,
        };
        let (item, run) = (ids::HTUI_ANA_2, RunId::new());
        for reply in [
            RunReply::Orch(OrchReply::CleanedUp { run }),
            RunReply::Frame(super::RunFrame::subscribed(item)),
            RunReply::Actions(Box::new(testing::unreachable_actions(item, String::new()))),
            RunReply::Failed {
                request: "run_actions",
                message: "gone".to_owned(),
            },
        ] {
            sink.send(&to, reply);
        }

        let mut got = Vec::new();
        while let Ok(envelope) = answers.try_recv() {
            assert_eq!((envelope.seq, &envelope.origin), (7, &Origin::App));
            got.push(envelope.reply);
        }
        assert_eq!(got.len(), 4, "one reply per send: {got:?}");
        assert!(
            matches!(&got[0], StoreReply::Orch(OrchReply::CleanedUp { run: of }) if *of == run),
            "{:?}",
            got[0]
        );
        assert!(
            matches!(&got[1], StoreReply::RunStream(super::RunFrame { item: of, run: None, kind: FrameKind::Subscribed })
                if *of == item),
            "{:?}",
            got[1]
        );
        assert!(
            matches!(&got[2], StoreReply::RunActions(actions) if actions.item == item),
            "{:?}",
            got[2]
        );
        assert!(
            matches!(&got[3], StoreReply::Failed { request: "run_actions", message } if message == "gone"),
            "{:?}",
            got[3]
        );
    }

    /// MOD-41 blueprint F-6 (plan D5's P-4): the runtime instantiates for the TUI's
    /// `(Backend, TuiReplies)` and the headless `(PgStore, Unaddressed)`, through both of its
    /// entry points: `sweep_with` (the sweep, `resumed`, `reclaim`, the retried claims) and
    /// `serve_request` (the verdicts and, through `spawn_task`, every command). That each spawned
    /// future is `Send` needs no test: the generic bodies of `sweep_with`, `serve_request` and
    /// `spawn_supervised` already prove it for any `H: WorkerHost` and `P: ReplySink`, or the
    /// library does not compile.
    #[test]
    fn a_generic_runtime_future_spawns() {
        fn generic<H: htui_core::store::WorkerHost, P: ReplySink>(
            runtime: &mut htui_worker::RunRuntime<H, P>,
            host: &H,
            sink: &P,
            addr: P::Addr,
            request: RunRequest,
        ) {
            runtime.sweep_with(host, sink);
            drop(runtime.serve_request(host, sink, addr, request, &LiveChats::default()));
        }
        let _ = generic::<Backend, TuiReplies>;
        let _ = generic::<PgStore, htui_worker::Unaddressed>;
    }

    /// Blueprint D183: with no runtime, the stream is acknowledged, the verdicts are read and the
    /// document is served; only a command is refused, by name.
    #[tokio::test]
    async fn a_try_serve_without_a_runtime_answers_the_stream_and_the_actions() {
        let backend = Backend::memory(MemStore::demo());

        let StoreReply::RunStream(frame) = store_worker::serve(
            &backend,
            &StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        )
        .await
        else {
            panic!("a subscription is acknowledged");
        };
        assert_eq!(frame.item, ids::HTUI_ANA_2);
        assert!(matches!(frame.kind, FrameKind::Subscribed));

        let StoreReply::RunActions(actions) =
            store_worker::serve(&backend, &StoreRequest::RunActions(ids::HTUI_ANA_2)).await
        else {
            panic!("the verdicts are read");
        };
        assert_eq!(actions.item, ids::HTUI_ANA_2);
        assert_eq!(actions.run, Ok(()), "an open item may start a run");

        let heads = backend
            .documents(ids::HTUI_FEAT_1)
            .await
            .expect("the demo documents");
        let head = heads.first().expect("the demo item has a document");
        let StoreReply::Document(document) =
            store_worker::serve(&backend, &StoreRequest::Document(head.id)).await
        else {
            panic!("the document is read");
        };
        assert_eq!(document.expect("the row exists").id, head.id);

        for request in every_orch_request() {
            let name = request.name();
            let reply = store_worker::serve(&backend, &StoreRequest::Orch(request)).await;
            assert!(
                matches!(&reply, StoreReply::Failed { request, message }
                    if *request == name && message == store_worker::NO_RUN_RUNTIME),
                "{reply:?}"
            );
        }
    }

    // -----------------------------------------------------------------------------------------
    // MOD-41 T9: the executor gate (I-1, plan D12, D13, OQ-4, OQ-5)
    // -----------------------------------------------------------------------------------------

    /// The box's executor set through its editor (MOD-41 plan D10).
    async fn set_executor(fixture: &Fixture, executor: Executor) {
        let row = fixture
            .store
            .box_row(ids::BOX)
            .await
            .expect("the read answers")
            .expect("the demo box");
        let edited = fixture
            .store
            .edit_box(
                ids::BOX,
                row.edit_version,
                BoxEdit {
                    executor: Some(executor),
                    ..BoxEdit::default()
                },
            )
            .await
            .expect("the edit answers");
        assert!(
            matches!(edited, CasOutcome::Applied(_)),
            "the executor was written: {edited:?}"
        );
    }

    /// The fixture over a demo whose box settings are `settings` (plan D9: a value the editor
    /// cannot write).
    async fn fixture_with_box_settings(settings: serde_json::Value) -> Fixture {
        let mut data = htui_core::fixtures::demo_data();
        for row in &mut data.boxes {
            row.settings = settings.clone();
        }
        Fixture::over(MemStore::from_demo(data)).await
    }

    /// One command served on `runtime` at `seq`, settled, and its answer.
    async fn served(
        runtime: &mut RunRuntime,
        fixture: &Fixture,
        seq: u64,
        request: StoreRequest,
    ) -> StoreReply {
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq,
                    origin: Origin::App,
                    request,
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred), "{served:?}");
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        drop(replies);
        let mut reply = None;
        while let Ok(envelope) = answers.try_recv() {
            if envelope.seq == seq && !matches!(envelope.reply, StoreReply::RunStream(_)) {
                reply = Some(envelope.reply);
            }
        }
        reply.expect("the command was answered")
    }

    /// One sweep of `runtime`, settled.
    async fn swept(runtime: &mut RunRuntime, fixture: &Fixture) {
        let (replies, _answers) = mpsc::unbounded_channel();
        runtime.sweep(&Backend::memory(fixture.store.clone()), &replies);
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
    }

    /// Plan D12 (`R` on a `worker` box): the run is only enqueued. The answer is `Started` at
    /// `queued`; nothing claims or walks it, not even this runtime's next sweep.
    #[tokio::test]
    async fn on_a_worker_box_start_run_only_queues() {
        let fixture = Fixture::new().await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime();

        let reply = served(&mut runtime, &fixture, 1, start_run(ids::HTUI_ANA_2)).await;
        let CommandOutcome::Started { run, rest } = outcome(reply) else {
            panic!("a start answers Started");
        };
        assert_eq!(
            (rest.run, rest.position, rest.failure),
            (RunStatus::Queued, None, None)
        );
        swept(&mut runtime, &fixture).await;
        let row = fixture.run(run).await;
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Queued, None),
            "the box's worker claims it, not the TUI"
        );
        assert!(fixture.steps(run).await.is_empty(), "nothing walked");
    }

    /// Plan D12 (`a` on a `worker` box): the answer is written under the lease, which is then
    /// given back: the run is `running`, unleased, and no step past the gate exists.
    #[tokio::test]
    async fn on_a_worker_box_an_answer_hands_back() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (run, research) = parked(&fixture, &mut worker).await;
        set_executor(&fixture, Executor::Worker).await;

        let answer = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        );
        let CommandOutcome::Answered { rest } = outcome(worker.reply(answer).await) else {
            panic!("an answer answers Answered");
        };
        assert_eq!(rest.run, RunStatus::Running, "handed back, not walked");
        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Running);
        assert!(
            row.lease_expires_at
                .is_some_and(|until| until <= Utc::now()),
            "the lease was released: {:?}",
            row.lease_expires_at
        );
        let steps = fixture.steps(run).await;
        assert_eq!(
            steps
                .iter()
                .map(|step| (step.position, step.status))
                .collect::<Vec<_>>(),
            [(0, StepStatus::Done)],
            "no step past the gate"
        );
    }

    /// I-1, plan D13: on a `worker` box the TUI's sweep adopts nothing, even a lapsed lease.
    #[tokio::test]
    async fn on_a_worker_box_the_tui_never_sweeps() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let before = fixture.run(run).await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime();

        swept(&mut runtime, &fixture).await;
        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Running);
        assert_eq!(
            row.lease_expires_at, before.lease_expires_at,
            "not adopted: the lease is still the lapsed stranger's"
        );
        assert!(fixture.steps(run).await.is_empty(), "nothing walked");
    }

    // -----------------------------------------------------------------------------------------
    // MOD-42 T4: the durable, graceful cancel (plan D11-D13, OQ-3, OQ-5)
    // -----------------------------------------------------------------------------------------

    /// `c` on `run`.
    fn cancel(run: RunId) -> StoreRequest {
        StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run }))
    }

    /// `p` on `step` of `run`, no chat open.
    fn promote(run: RunId, step: StepId) -> StoreRequest {
        StoreRequest::Orch(OrchRequest::Command(Command::PromoteStep {
            run,
            step,
            chat_open: false,
        }))
    }

    /// A [`stranded`] run of `item` whose live lease a stranger took for a day: the box's worker
    /// walking it.
    async fn walked_by_a_stranger(fixture: &Fixture, item: ItemId) -> RunId {
        let run = stranded(fixture, item).await;
        assert!(
            fixture
                .store
                .take_lease(run, ids::BOX, Uuid::now_v7(), TimeDelta::days(1))
                .await
                .expect("the take answers"),
            "the box's worker walks it, for a day"
        );
        run
    }

    /// The run commands of `run` (MemStore's test-support reader: no trait method lists them).
    fn commands_of(fixture: &Fixture, run: RunId) -> Vec<RunCommand> {
        fixture
            .store
            .command_rows()
            .into_iter()
            .filter(|row| row.run_id == run)
            .collect()
    }

    /// A pending cancel of `run`, written as a TUI on this box would (D12 step 2).
    async fn requested(fixture: &Fixture, run: RunId) -> RunCommandId {
        match fixture
            .store
            .request_cancel(run, ids::USER, ids::BOX)
            .await
            .expect("the request answers")
        {
            CancelRequest::Inserted(id) => id,
            other @ CancelRequest::AlreadyPending(_) => panic!("a first cancel inserts: {other:?}"),
        }
    }

    /// The relay row `id`.
    fn relay_row(fixture: &Fixture, id: PermissionId) -> StepPermission {
        fixture
            .store
            .relay_rows()
            .into_iter()
            .find(|row| row.id == id)
            .expect("the relay row exists")
    }

    /// The first `pending` relay row, once a walk parked one.
    async fn a_pending_request(fixture: &Fixture) -> StepPermission {
        within("a request parking", async {
            loop {
                if let Some(row) = fixture
                    .store
                    .relay_rows()
                    .into_iter()
                    .find(|row| row.status == PermissionStatus::Pending)
                {
                    return row;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
    }

    /// The `permission_answer` payloads in `step`'s log.
    async fn permission_answers(fixture: &Fixture, step: StepId) -> Vec<serde_json::Value> {
        fixture
            .store
            .step_events(step)
            .await
            .expect("the read answers")
            .unwrap_or_default()
            .into_iter()
            .filter(|row| row.kind == EventKind::PermissionAnswer)
            .map(|row| row.payload)
            .collect()
    }

    /// I-7 after a graceful cancel reached `parked`: its row reads `cancelled` and its step's
    /// log holds one `permission_answer {cancelled: true}`.
    async fn assert_answered_cancelled_once(fixture: &Fixture, parked: &StepPermission) {
        assert_eq!(
            relay_row(fixture, parked.id).status,
            PermissionStatus::Cancelled,
            "the parked request was answered `cancelled`"
        );
        let answers = permission_answers(fixture, parked.run_step_id).await;
        assert_eq!(answers.len(), 1, "I-7's row exactly once: {answers:?}");
        assert_eq!(answers[0]["cancelled"], true, "{answers:?}");
    }

    /// One command poll of `runtime`, settled.
    async fn polled(runtime: &mut RunRuntime, fixture: &Fixture) {
        let (replies, _answers) = mpsc::unbounded_channel();
        runtime.poll_commands(
            &Backend::memory(fixture.store.clone()),
            &replies,
            &LiveChats::default(),
        );
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
    }

    /// D11, D12 step 3, I-7: `c` on a run this process walks while its session is parked on a
    /// relayed request is durable and graceful: the cancel row is written and applied, the parked
    /// request is answered `cancelled` once, the session's cancel gets the runtime's grace, the
    /// step is cancelled by `cancel_leased` (never failed), and the walk's own requester hears
    /// that it was preempted.
    #[tokio::test]
    async fn an_in_process_cancel_answers_the_parked_request_and_ends_gracefully() {
        let fixture = Fixture::new().await;
        let park = Park::new();
        fixture.sessions.push(Play::Park(park.clone()));
        let runtime = fixture.runtime().with_cancel_grace(Duration::from_secs(3));
        let mut worker = Worker::spawn(&fixture.store, runtime);

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        let parked = a_pending_request(&fixture).await;
        let run = parked.run_id;
        let asked = worker.send(Origin::App, cancel(run));

        assert!(matches!(
            outcome(worker.reply(asked).await),
            CommandOutcome::Cancelled { .. }
        ));
        assert!(
            matches!(worker.reply(start).await, StoreReply::Failed { request: "start_run", ref message } if message == PREEMPTED),
            "the walk's own requester is answered once, as preempted"
        );
        assert_answered_cancelled_once(&fixture, &parked).await;
        assert_eq!(
            park.grace(),
            Some(Duration::from_secs(3)),
            "the session's cancel was given the runtime's grace"
        );
        let step = fixture
            .steps(run)
            .await
            .into_iter()
            .find(|step| step.id == parked.run_step_id)
            .expect("the parked step");
        assert_eq!(
            step.status,
            StepStatus::Cancelled,
            "cancel_leased settled it; the walk settled nothing (I-6)"
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        assert_eq!(
            commands_of(&fixture, run)
                .iter()
                .map(|row| row.status)
                .collect::<Vec<_>>(),
            [RunCommandStatus::Applied],
            "one cancel row, applied"
        );
    }

    /// D11 (R-38), I-7: `p` on a step whose session is parked on a relayed request preempts the
    /// walk gracefully: the request is answered `cancelled` before the promotion lands, and the
    /// promotion answers as it always has.
    #[tokio::test]
    async fn a_promote_preempt_is_graceful() {
        let fixture = Fixture::new().await;
        let park = Park::new();
        fixture.sessions.push(Play::Park(park.clone()));
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        let parked = a_pending_request(&fixture).await;
        let asked = worker.send(
            Origin::Tab(TabId("chat")),
            promote(parked.run_id, parked.run_step_id),
        );

        let reply = worker.reply(asked).await;
        assert!(
            matches!(&reply, StoreReply::Orch(OrchReply::Promoted { step, .. }) if *step == parked.run_step_id),
            "{reply:?}"
        );
        assert!(
            matches!(worker.reply(start).await, StoreReply::Failed { request: "start_run", ref message } if message == PREEMPTED),
            "the preempted walk's requester is answered once"
        );
        assert_answered_cancelled_once(&fixture, &parked).await;
        assert_eq!(park.grace(), Some(CANCEL_GRACE), "a graceful cancel");
        let step = step_at(&fixture, parked.run_id, 0).await;
        assert_eq!(step.id, parked.run_step_id);
        assert_eq!(step.status, StepStatus::AwaitingApproval);
        assert!(step.promoted_at.is_some(), "promoted");
        assert!(
            commands_of(&fixture, parked.run_id).is_empty(),
            "a promotion writes no cancel row"
        );
    }

    /// D12 steps 2 and 5 (replaces MOD-41's OQ-4 refusal): `c` on a `worker` box under the
    /// worker's live lease writes one pending cancel and answers that it was requested; the run
    /// itself is untouched until its executor applies the row.
    #[tokio::test]
    async fn on_a_worker_box_cancel_writes_a_pending_command_and_says_requested() {
        let fixture = Fixture::new().await;
        let run = walked_by_a_stranger(&fixture, ids::HTUI_ANA_2).await;
        let before = fixture.run(run).await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime();

        let reply = served(&mut runtime, &fixture, 1, cancel(run)).await;
        assert!(
            matches!(&reply, StoreReply::Failed { request: "cancel_run", message } if message == CANCEL_REQUESTED),
            "{reply:?}"
        );
        let commands = commands_of(&fixture, run);
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert_eq!(
            (
                commands[0].kind,
                commands[0].status,
                commands[0].issued_by,
                commands[0].issued_box
            ),
            (
                RunCommandKind::Cancel,
                RunCommandStatus::Pending,
                ids::USER,
                ids::BOX
            )
        );
        let row = fixture.run(run).await;
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Running, before.lease_expires_at),
            "the run is untouched"
        );
        assert!(fixture.steps(run).await.is_empty());
    }

    /// D1's one pending cancel per run: a second `c` answers that a cancel is already requested,
    /// and writes nothing.
    #[tokio::test]
    async fn a_second_cancel_says_already_requested() {
        let fixture = Fixture::new().await;
        let run = walked_by_a_stranger(&fixture, ids::HTUI_ANA_2).await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime();

        let first = served(&mut runtime, &fixture, 1, cancel(run)).await;
        assert!(
            matches!(&first, StoreReply::Failed { message, .. } if message == CANCEL_REQUESTED),
            "{first:?}"
        );
        let second = served(&mut runtime, &fixture, 2, cancel(run)).await;
        assert!(
            matches!(&second, StoreReply::Failed { request: "cancel_run", message } if message == CANCEL_ALREADY_REQUESTED),
            "{second:?}"
        );
        assert_eq!(commands_of(&fixture, run).len(), 1, "still one row");
    }

    /// D13: the worker's command poll applies a pending cancel of a run whose lease it holds: the
    /// run is cancelled and the row `applied`.
    #[tokio::test]
    async fn the_workers_command_poll_applies_a_pending_cancel() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime().with_role(Role::Worker);
        let owner = testing::probe(&runtime).owner();
        assert!(
            fixture
                .store
                .take_lease(run, ids::BOX, owner, TimeDelta::days(1))
                .await
                .expect("the take answers"),
            "the worker holds the run"
        );
        let id = requested(&fixture, run).await;

        polled(&mut runtime, &fixture).await;
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        let commands = commands_of(&fixture, run);
        assert_eq!(
            commands
                .iter()
                .map(|row| (row.id, row.status))
                .collect::<Vec<_>>(),
            [(id, RunCommandStatus::Applied)]
        );
    }

    /// D13: the TUI's store loop ticks the command poll beside its sweeper, so a pending cancel
    /// of a run this process holds is applied without any request.
    #[tokio::test]
    async fn the_store_loop_applies_a_pending_cancel_on_its_own() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let runtime = fixture.runtime();
        let owner = testing::probe(&runtime).owner();
        assert!(
            fixture
                .store
                .take_lease(run, ids::BOX, owner, TimeDelta::days(1))
                .await
                .expect("the take answers"),
            "this process holds the run"
        );
        let id = requested(&fixture, run).await;

        let _worker = Worker::spawn(&fixture.store, runtime);
        rests_at(&fixture, run, RunStatus::Cancelled).await;
        within("the row being resolved", async {
            while commands_of(&fixture, run)
                .iter()
                .any(|row| row.status != RunCommandStatus::Applied)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert_eq!(
            commands_of(&fixture, run)
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            [id]
        );
    }

    /// D12 step 4: a run executing on another box is that box's to cancel. The row stays
    /// `pending` and the answer says it was requested, with no `RunStatus` refusal.
    #[tokio::test]
    async fn a_cancel_of_a_run_on_another_box_stays_pending() {
        let mut data = htui_core::fixtures::demo_data();
        let mut elsewhere = data.boxes[0].clone();
        elsewhere.id = BoxId::new();
        elsewhere.hostname = "elsewhere".to_owned();
        let other = elsewhere.id;
        data.boxes.push(elsewhere);
        let fixture = Fixture::over(MemStore::from_demo(data)).await;
        let run = stranded_on(&fixture, ids::HTUI_ANA_2, other).await;
        let mut runtime = fixture.runtime();

        let reply = served(&mut runtime, &fixture, 1, cancel(run)).await;
        assert!(
            matches!(&reply, StoreReply::Failed { request: "cancel_run", message } if message == CANCEL_REQUESTED),
            "{reply:?}"
        );
        assert_eq!(
            commands_of(&fixture, run)
                .iter()
                .map(|row| row.status)
                .collect::<Vec<_>>(),
            [RunCommandStatus::Pending]
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Running);
    }

    /// B-4, D13: a pending cancel of a run that finished before its executor saw it is refused
    /// with the run's actual status at the next poll.
    #[tokio::test]
    async fn a_pending_cancel_of_a_finished_run_is_refused_with_its_status() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let id = requested(&fixture, run).await;
        fixture
            .store
            .finish_run(run, RunStatus::Done, None, Utc::now())
            .await
            .expect("the run finishes");
        let mut runtime = fixture.runtime();

        let frames = polled_watching(
            &mut runtime,
            &fixture,
            ids::HTUI_ANA_2,
            &LiveChats::default(),
        )
        .await;
        let commands = commands_of(&fixture, run);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            (commands[0].id, commands[0].status),
            (id, RunCommandStatus::Refused)
        );
        let resolution = commands[0].resolution.clone().unwrap_or_default();
        assert!(resolution.contains("done"), "{resolution}");
        assert_eq!(fixture.run(run).await.status, RunStatus::Done);
        assert!(
            !frames
                .iter()
                .any(|kind| matches!(kind, FrameKind::Error(_))),
            "B-10: a polled refusal is recorded on its row, never published: {frames:?}"
        );
    }

    /// D12 steps 3 and 5: a pending cancel whose run is already `cancelled` — another process on
    /// this box applied it and is still in `cancel_leased`'s cleanup, its row not yet resolved —
    /// is `applied` at the next poll, never `refused`, whichever process resolves the row first.
    #[tokio::test]
    async fn a_pending_cancel_of_a_cancelled_run_is_applied() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let id = requested(&fixture, run).await;
        fixture
            .store
            .finish_run(run, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("another process cancels the run");
        let mut runtime = fixture.runtime();

        let frames = polled_watching(
            &mut runtime,
            &fixture,
            ids::HTUI_ANA_2,
            &LiveChats::default(),
        )
        .await;
        let commands = commands_of(&fixture, run);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            (commands[0].id, commands[0].status),
            (id, RunCommandStatus::Applied)
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        assert!(
            !frames
                .iter()
                .any(|kind| matches!(kind, FrameKind::Error(_))),
            "a polled cancel publishes nothing: {frames:?}"
        );
    }

    /// One command poll of `runtime` under `live`, settled, while a pane watches `item`: the kinds
    /// of the frames published for it meanwhile.
    async fn polled_watching(
        runtime: &mut RunRuntime,
        fixture: &Fixture,
        item: ItemId,
        live: &LiveChats,
    ) -> Vec<FrameKind> {
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::Tab(TabId("backlog")),
                    request: StoreRequest::RunStream { item },
                },
                &LiveChats::default(),
            )
            .await;
        while answers.try_recv().is_ok() {}
        runtime.poll_commands(&backend, &replies, live);
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        drop(replies);
        let mut frames = Vec::new();
        while let Ok(envelope) = answers.try_recv() {
            if let StoreReply::RunStream(frame) = envelope.reply {
                frames.push(frame.kind);
            }
        }
        frames
    }

    /// B-10 (F-12): a polled cancel of a run one of whose steps a live chat drives stays
    /// `pending`, moves nothing and publishes nothing; the first poll after the chat closed
    /// applies it.
    #[tokio::test]
    async fn a_polled_cancel_under_a_live_chat_stays_pending_and_silent() {
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred));
        assert!(runtime.settle(PATIENCE).await.is_empty());
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        let research = step_at(&fixture, run, 0).await;
        assert_eq!(research.status, StepStatus::AwaitingApproval);
        let id = requested(&fixture, run).await;

        let frames = polled_watching(
            &mut runtime,
            &fixture,
            ids::HTUI_ANA_2,
            &LiveChats::of([research.id]),
        )
        .await;
        assert!(
            !frames
                .iter()
                .any(|kind| matches!(kind, FrameKind::Error(_))),
            "the chat's refusal of a polled cancel is not published: {frames:?}"
        );
        assert_eq!(
            commands_of(&fixture, run)
                .iter()
                .map(|row| (row.id, row.status))
                .collect::<Vec<_>>(),
            [(id, RunCommandStatus::Pending)]
        );
        assert_eq!(
            fixture.run(run).await.status,
            RunStatus::AwaitingApproval,
            "nothing moved under the chat"
        );

        polled(&mut runtime, &fixture).await;
        assert_eq!(
            fixture.run(run).await.status,
            RunStatus::Cancelled,
            "the row was this poll's to apply all along"
        );
        assert_eq!(
            commands_of(&fixture, run)
                .iter()
                .map(|row| (row.id, row.status))
                .collect::<Vec<_>>(),
            [(id, RunCommandStatus::Applied)]
        );
    }

    // -----------------------------------------------------------------------------------------
    // MOD-24 D3: commands before recovery (OQ-2)
    // -----------------------------------------------------------------------------------------

    /// `run`'s commands as `(id, status)` pairs.
    fn command_states(fixture: &Fixture, run: RunId) -> Vec<(RunCommandId, RunCommandStatus)> {
        commands_of(fixture, run)
            .iter()
            .map(|row| (row.id, row.status))
            .collect()
    }

    /// MOD-24 D3: a worker's sweep that finds a free (lapsed) run with a pending cancel applies
    /// the cancel before it recovers anything. The run is cancelled by `cancel_leased`, its row is
    /// `applied`, and no session is started: nothing was recovered or walked on.
    #[tokio::test]
    async fn a_sweep_applies_a_pending_cancel_before_it_recovers_a_free_run() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime().with_role(Role::Worker);
        let probe = testing::probe(&runtime);
        let id = requested(&fixture, run).await;

        swept(&mut runtime, &fixture).await;
        assert_eq!(
            fixture.run(run).await.status,
            RunStatus::Cancelled,
            "the cancel was applied before any recovery"
        );
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Applied)]
        );
        assert!(
            fixture.steps(run).await.is_empty(),
            "no session started, nothing recovered"
        );
        assert!(!probe.is_applying(id), "B-5's guard is free again");
    }

    /// MOD-24 D3, plan fact 5: the deterministic loss. The walk crashes (its lease fenced) inside
    /// the last, ungated phase, with that phase's document written and its commits captured, so a
    /// recovery would classify the step `Finished` and land the run `done` itself, refusing the
    /// user's pending cancel at the next poll. The sweep applies the cancel first instead: the run
    /// is cancelled, the last step is cancelled by `cancel_leased` (never `done`), and the row is
    /// `applied`.
    #[tokio::test(start_paused = true)]
    async fn a_cancel_beats_the_recovery_that_would_finish_the_run() {
        let mut data = htui_core::fixtures::demo_data();
        let mut last_kind = None;
        let mut phases = 0;
        let mut ana: Vec<_> = data
            .phases
            .iter_mut()
            .filter(|phase| phase.graph_id == ids::GRAPH_HTUI_ANA)
            .collect();
        ana.sort_by_key(|phase| phase.position);
        for phase in ana {
            assert_eq!(phase.fan_out, 1, "one session per phase: {}", phase.name);
            phase.gate = Gate::Never;
            phase.gate_hard = false;
            last_kind = Some(phase.output_kind.clone());
            phases += 1;
        }
        let last_kind = last_kind.expect("the graph has phases");
        assert!(phases > 1, "the crash is in a later phase");
        let fixture = Fixture::over(MemStore::from_demo(data)).await;
        fixture
            .store
            .set_app_setting("lease_ttl_seconds", json!(30));
        for _ in 1..phases {
            fixture.sessions.push(Play::Done);
        }
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut runtime = fixture.runtime();
        let probe = testing::probe(&runtime);
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred), "{served:?}");
        within(
            "the last phase's session starting",
            stall.reached.notified(),
        )
        .await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        let steps = fixture.steps(run).await;
        assert_eq!(steps.len(), phases, "{steps:?}");
        let last = steps
            .iter()
            .max_by_key(|step| step.position)
            .expect("a step")
            .clone();
        assert_eq!(last.status, StepStatus::Running);
        assert!(
            steps
                .iter()
                .filter(|step| step.id != last.id)
                .all(|step| step.status == StepStatus::Done),
            "every earlier phase finished: {steps:?}"
        );

        // H-21: the last phase's work, written while the walk's lease is live.
        fixture
            .store
            .write_document(NewDocument {
                id: DocumentId::new(),
                item_id: ids::HTUI_ANA_2,
                kind: last_kind,
                title: "the last phase's output".to_owned(),
                body: "authored".to_owned(),
                produced_by_step_id: Some(last.id),
                created_by: ids::USER,
                created_at: Utc::now(),
            })
            .await
            .expect("the document lands");
        let before = fixture
            .store
            .step_commits(last.id)
            .await
            .expect("the read answers");
        let captured: Vec<RunStepCommit> = fixture
            .run(run)
            .await
            .repo_scope
            .iter()
            .map(|repo| RunStepCommit {
                run_step_id: last.id,
                repo_id: *repo,
                before_hash: before
                    .iter()
                    .find(|row| row.repo_id == *repo)
                    .map_or_else(|| "0".repeat(40), |row| row.before_hash.clone()),
                after_hash: Some("a".repeat(40)),
            })
            .collect();
        assert!(!captured.is_empty(), "the run has a repo to capture");
        fixture
            .store
            .record_commits(StepFence::Lease(probe.owner()), last.id, &captured)
            .await
            .expect("the capture lands under the walk's lease");

        // The crash: the store stops renewing the lease, so the walk is fenced and dropped.
        fixture.store.set_fault(MemFault::RefreshLease, true);
        tokio::time::timeout(Duration::from_secs(600), async {
            while !probe.is_dead_walk(run) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the walk was fenced");
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        fixture.store.set_fault(MemFault::RefreshLease, false);
        assert!(stall.dropped.load(Ordering::SeqCst), "the walk was dropped");
        let id = requested(&fixture, run).await;

        swept(&mut runtime, &fixture).await;
        assert_eq!(
            fixture.run(run).await.status,
            RunStatus::Cancelled,
            "the cancel, not the recovery, ended the run"
        );
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Applied)]
        );
        let steps = fixture.steps(run).await;
        let crashed = steps
            .iter()
            .find(|step| step.id == last.id)
            .expect("the crashed step");
        assert_eq!(
            crashed.status,
            StepStatus::Cancelled,
            "`cancel_leased` settled it, no recovery finished it"
        );
        assert!(
            !steps
                .iter()
                .any(|step| step.position == last.position && step.status == StepStatus::Done),
            "nothing at the last position is done: {steps:?}"
        );
        assert!(!probe.is_dead_walk(run), "the cancel took the run back");
    }

    /// MOD-24 D3 versus the poll (B-5, H-16): a sweep and a command poll started in one tick both
    /// find the same pending cancel of a free run. The row is applied once, by whichever task
    /// takes B-5's guard first; the other waits for it (the sweep) or skips it (the poll), and no
    /// recovery overtakes it. Not TDD-red: before D3 the outcome depended on the interleaving, so
    /// this pins the guard rather than the fix.
    #[tokio::test]
    async fn a_sweep_and_a_poll_in_one_tick_apply_one_cancel_once() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime().with_role(Role::Worker);
        let probe = testing::probe(&runtime);
        let id = requested(&fixture, run).await;
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();

        runtime.sweep(&backend, &replies);
        runtime.poll_commands(&backend, &replies, &LiveChats::default());
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Applied)],
            "one row, applied once"
        );
        assert!(fixture.steps(run).await.is_empty(), "nothing recovered");
        assert!(!probe.is_applying(id));
    }

    /// MOD-24 D3 versus the poll (H-16), the deterministic shape: the poll's `cancel_run` takes
    /// B-5's guard and then a walk child on the run before it awaits the run lock, so the run reads
    /// live here while the poll holds the row. The sweep must wait for that row instead of
    /// skipping it as a walked run's: it stays pending, adopting and stepping nothing, until the
    /// guard drops, and the poll's cancel (played here by hand) is what ends the run.
    #[tokio::test]
    async fn a_sweep_waits_for_a_cancel_the_poll_holds_under_a_walk_child() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        set_executor(&fixture, Executor::Worker).await;
        let mut runtime = fixture.runtime().with_role(Role::Worker);
        let probe = testing::probe(&runtime);
        let id = requested(&fixture, run).await;
        let lapsed = fixture.run(run).await.lease_expires_at;
        // The poll's `cancel_run` between B-5's guard and its run lock.
        let held = probe.hold_applying(id).expect("nobody applies the row yet");
        let walk = probe.walk_child(run);
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();

        runtime.sweep(&backend, &replies);
        let finished = tokio::time::timeout(Duration::from_millis(500), async {
            while !probe.all_tasks_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(finished.is_err(), "the sweep waits for the poll's row");
        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Running);
        assert_eq!(row.lease_expires_at, lapsed, "nothing adopted the run");
        assert!(fixture.steps(run).await.is_empty(), "nothing stepped it");
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Pending)]
        );

        // The poll applies its cancel and lets go: the run lock, the walk child, then the row.
        fixture
            .store
            .finish_run(run, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the cancel lands");
        assert!(
            fixture
                .store
                .resolve_command(id, RunCommandStatus::Applied, None)
                .await
                .expect("the resolve answers"),
            "the row was pending"
        );
        drop(walk);
        drop(held);
        assert!(runtime.settle(PATIENCE).await.is_empty(), "the sweep ends");
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Applied)]
        );
        assert!(fixture.steps(run).await.is_empty(), "nothing recovered");
        assert!(!probe.is_applying(id));
    }

    /// B-5's guard, claimed a second time while a task holds it (the sweep's `cancel_run` behind
    /// the poll's, MOD-24 D3): the claim answers `None` at once and the holder keeps the row. The
    /// claim runs on its own thread, so a claim that never returns (the guard built eagerly and
    /// dropped under the set's own lock) fails the case instead of hanging it.
    #[tokio::test]
    async fn a_second_claim_of_a_held_row_is_refused_and_frees_nothing() {
        let fixture = Fixture::new().await;
        let runtime = fixture.runtime();
        let probe = testing::probe(&runtime);
        let id = RunCommandId::new();
        let held = probe
            .hold_applying(id)
            .expect("the first claim takes the row");
        let second = testing::probe(&runtime);
        let (answer, answered) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let refused = second.hold_applying(id).is_none();
            let _ = answer.send((refused, second.is_applying(id)));
        });
        let Ok((refused, still_held)) = answered.recv_timeout(Duration::from_secs(5)) else {
            // The stuck claim holds the set's lock for ever: dropping `held` would block too.
            std::mem::forget(held);
            panic!("a second claim of a held row never answered: B-5's guard deadlocked");
        };
        assert!(refused, "the row is held, so the second claim is refused");
        assert!(still_held, "the refused claim freed nothing");
        drop(held);
        assert!(!probe.is_applying(id), "the holder's drop frees the row");
    }

    /// MOD-24 D3 (review L4): the sweep's cancels apply to graph runs only. A chat run on this box
    /// reads `running` with no lease, so its pending cancel is in `pending_commands`, but the
    /// sweep never adopts a chat (D3b): there is no recovery to beat, and the row stays the poll's.
    #[tokio::test]
    async fn a_sweep_leaves_a_chat_runs_cancel_to_the_poll() {
        let fixture = Fixture::new().await;
        let project = fixture.item(ids::HTUI_ANA_2).await.project_id;
        let chat = ChatRunSpec::mint(project, ids::BOX, ids::USER, None, None);
        fixture
            .store
            .start_chat_run(&chat)
            .await
            .expect("the chat run lands");
        let id = requested(&fixture, chat.run_id).await;
        let mut runtime = fixture.runtime();
        let probe = testing::probe(&runtime);

        swept(&mut runtime, &fixture).await;
        assert_eq!(
            command_states(&fixture, chat.run_id),
            [(id, RunCommandStatus::Pending)],
            "the sweep left the chat's cancel to the poll"
        );
        assert_eq!(fixture.run(chat.run_id).await.status, RunStatus::Running);
        // The engine refuses a chat's cancel anyway; what the kind check spares is the attempt,
        // whose `cancel_run` would have minted the run's lock (pruned only at the next tick).
        assert!(
            !probe.has_lock_entry(chat.run_id),
            "the sweep never tried the chat's cancel"
        );
    }

    /// MOD-24 D3, regression guard: a run whose lease is live elsewhere is its holder's. Its
    /// pending cancel is not in `pending_commands(owner, box)`, and `adopt_runs` skips it too, so
    /// the sweep leaves the row `pending`, the run `running`, and walks nothing.
    #[tokio::test]
    async fn a_sweep_leaves_a_live_leases_cancel_to_its_holder() {
        let fixture = Fixture::new().await;
        let run = walked_by_a_stranger(&fixture, ids::HTUI_ANA_2).await;
        let id = requested(&fixture, run).await;
        let mut runtime = fixture.runtime();

        swept(&mut runtime, &fixture).await;
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Pending)]
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Running);
        assert!(fixture.steps(run).await.is_empty(), "nothing walked");
    }

    /// MOD-24 D3 (B5, D212): the sweep's cancels honour the live chats the last poll was handed.
    /// A free run one of whose steps a live chat drives keeps its pending cancel through the
    /// poll that refused it and through the sweep after it.
    #[tokio::test]
    async fn a_sweep_does_not_cancel_under_a_chat_the_poll_refused() {
        let fixture = Fixture::new().await;
        let agent = scripted_agent(&fixture).await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let step = fixture
            .store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index: 0,
                phase_name: "research".to_owned(),
                agent_id: Some(agent),
                model: Some("sonnet".to_owned()),
            })
            .await
            .expect("the step lands");
        let id = requested(&fixture, run).await;
        let mut runtime = fixture.runtime();

        polled_watching(
            &mut runtime,
            &fixture,
            ids::HTUI_ANA_2,
            &LiveChats::of([step.id]),
        )
        .await;
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Pending)],
            "the poll refused it under the chat"
        );
        swept(&mut runtime, &fixture).await;
        assert_eq!(
            command_states(&fixture, run),
            [(id, RunCommandStatus::Pending)],
            "and so did the sweep"
        );
        assert_ne!(fixture.run(run).await.status, RunStatus::Cancelled);
    }

    /// B-20 with `Preempt::IfLive` (plan D11, D12 step 1): `c` on a `queued` run whose claim this
    /// process is making stops that claim first, gracefully, then takes today's path: the run is
    /// cancelled from `queued`, with no cancel row and nothing walked.
    #[tokio::test]
    async fn a_cancel_of_a_queued_run_stops_its_live_claim_first() {
        let fixture = Fixture::new().await;
        let run = queued_by_another(&fixture, ids::HTUI_ANA_2).await;
        let mut runtime = fixture.runtime().with_cancel_grace(Duration::from_secs(60));
        let probe = testing::probe(&runtime);
        // This process's claim of the run, live.
        let claim = probe.walk_child(run);
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();

        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: cancel(run),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred), "{served:?}");
        within("the claim being asked to stop", async {
            while !testing::signalled(&claim) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        drop(claim);
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        drop(replies);
        let mut reply = None;
        while let Ok(envelope) = answers.try_recv() {
            if envelope.seq == 1 && !matches!(envelope.reply, StoreReply::RunStream(_)) {
                reply = Some(envelope.reply);
            }
        }
        let reply = reply.expect("the cancel was answered");
        assert!(
            matches!(&reply, StoreReply::Orch(OrchReply::Done(_))),
            "{reply:?}"
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        assert!(commands_of(&fixture, run).is_empty(), "no cancel row");
        assert!(fixture.steps(run).await.is_empty(), "nothing walked");
    }

    /// D12 steps 1 and 2 (review L-4): `c` read the run `queued`, but another process on this box
    /// claimed it before the queued CAS. The run is no longer queued, so the cancel is durable: one
    /// pending row and the `requested` answer, not the engine's raw `LeaseHeld` sentence.
    #[tokio::test]
    async fn a_cancel_whose_queued_run_is_claimed_meanwhile_is_requested() {
        let fixture = Fixture::new().await;
        let run = queued_by_another(&fixture, ids::HTUI_ANA_2).await;
        let mut runtime = fixture.runtime();
        let probe = testing::probe(&runtime);
        // Hold the run's lock so the cancel parks after its snapshot read, before the CAS.
        let held = probe.try_lock(run).expect("nobody holds it");
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();

        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: cancel(run),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred), "{served:?}");
        within("the cancel waiting for the run's lock", async {
            while probe.live_walks(run) < 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        // The box's worker claims the run, for a day.
        let claim = fixture
            .store
            .claim_run(
                run,
                ids::BOX,
                Uuid::now_v7(),
                Utc::now(),
                TimeDelta::days(1),
            )
            .await
            .expect("the claim answers");
        assert!(claim.is_admitted(), "{claim}");
        let before = fixture.run(run).await;
        drop(held);

        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        drop(replies);
        let mut reply = None;
        while let Ok(envelope) = answers.try_recv() {
            if envelope.seq == 1 && !matches!(envelope.reply, StoreReply::RunStream(_)) {
                reply = Some(envelope.reply);
            }
        }
        let reply = reply.expect("the cancel was answered");
        assert!(
            matches!(&reply, StoreReply::Failed { request: "cancel_run", message } if message == CANCEL_REQUESTED),
            "{reply:?}"
        );
        assert_eq!(
            commands_of(&fixture, run)
                .iter()
                .map(|row| (row.kind, row.status))
                .collect::<Vec<_>>(),
            [(RunCommandKind::Cancel, RunCommandStatus::Pending)],
            "one durable cancel row, left for the claimant"
        );
        let row = fixture.run(run).await;
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Running, before.lease_expires_at),
            "the claimant's run is untouched"
        );
    }

    /// D11: a command queued for the lock of a run being cancelled gracefully is refused at the
    /// cancel's signal, not at the token's drop, so the cancel ends as soon as the walk rests —
    /// well inside its grace — and the queued command is answered `PREEMPTED`.
    #[tokio::test]
    async fn a_command_queued_behind_a_cancelled_walk_leaves_at_the_signal() {
        let fixture = Fixture::new().await;
        let park = Park::new();
        fixture.sessions.push(Play::Park(park.clone()));
        let grace = Duration::from_secs(60);
        let runtime = fixture.runtime().with_cancel_grace(grace);
        let probe = testing::probe(&runtime);
        let mut worker = Worker::spawn(&fixture.store, runtime);

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        let parked = a_pending_request(&fixture).await;
        let run = parked.run_id;
        let queued = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: parked.run_step_id,
                answer: GateAnswer::Approved,
            })),
        );
        within("the gate answer queued for the run's lock", async {
            while probe.live_walks(run) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        let began = std::time::Instant::now();
        let asked = worker.send(Origin::App, cancel(run));

        assert!(matches!(
            outcome(worker.reply(asked).await),
            CommandOutcome::Cancelled { .. }
        ));
        assert!(
            began.elapsed() < grace / 2,
            "the cancel waited on the queued command: {:?}",
            began.elapsed()
        );
        assert!(
            matches!(worker.reply(queued).await, StoreReply::Failed { ref message, .. } if message == PREEMPTED),
            "the queued command is refused as preempted"
        );
        assert!(
            matches!(worker.reply(start).await, StoreReply::Failed { request: "start_run", ref message } if message == PREEMPTED),
            "the walk's own requester is answered once, as preempted"
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
    }

    /// D13, B-5: a tick while a poll is in flight starts none, and a later tick skips a row a
    /// task is still applying — one cancel task, the row applied once.
    #[tokio::test]
    async fn overlapping_command_polls_run_one_poll() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let id = requested(&fixture, run).await;
        let mut runtime = fixture.runtime();
        let probe = testing::probe(&runtime);
        let held = probe.try_lock(run).expect("nobody holds it");
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();
        let live = LiveChats::default();

        runtime.poll_commands(&backend, &replies, &live);
        runtime.poll_commands(&backend, &replies, &live);
        assert_eq!(
            runtime.tasks_len(),
            1,
            "the second tick found the first poll in flight"
        );
        within("the cancel task taking the row", async {
            while !probe.is_applying(id) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        runtime.poll_commands(&backend, &replies, &live);
        within("the third tick's poll ending", async {
            while probe.unfinished_tasks() > 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert_eq!(
            runtime.tasks_len(),
            2,
            "the blocked cancel and the third poll: the row being applied was skipped"
        );

        drop(held);
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
        assert_eq!(
            commands_of(&fixture, run)
                .iter()
                .map(|row| row.status)
                .collect::<Vec<_>>(),
            [RunCommandStatus::Applied]
        );
    }

    /// OQ-3: `p` on a step the box's worker walks is refused with the sentence naming the
    /// walker: a live session cannot be handed to this TUI's chat across processes.
    #[tokio::test]
    async fn promote_of_a_worker_walked_step_names_the_worker() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (run, research) = parked(&fixture, &mut worker).await;
        assert!(
            fixture
                .store
                .take_lease(run, ids::BOX, Uuid::now_v7(), TimeDelta::days(1))
                .await
                .expect("the take answers"),
            "the box's worker walks it, for a day"
        );
        set_executor(&fixture, Executor::Worker).await;

        let asked = worker.send(Origin::Tab(TabId("chat")), promote(run, research.id));
        let reply = worker.reply(asked).await;
        assert!(
            matches!(&reply, StoreReply::Failed { message, .. } if *message == promote_needs_the_walker(run)),
            "{reply:?}"
        );
        let step = step_at(&fixture, run, 0).await;
        assert!(step.promoted_at.is_none(), "nothing promoted");
    }

    /// OQ-5 pin: shutdown stays a hard drop. A parked walk ends within `2 × grace` with no
    /// graceful cancel; `Engine::abandoned` marks its relay row `stale` before the lease goes back
    /// (review L-1, blueprint A-11), so `relay_view` never lists it again.
    #[tokio::test]
    async fn shutdown_still_drops_a_parked_walk() {
        let fixture = Fixture::new().await;
        let park = Park::new();
        fixture.sessions.push(Play::Park(park.clone()));
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, _answers) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred));
        let parked = a_pending_request(&fixture).await;

        let grace = Duration::from_millis(500);
        let began = tokio::time::Instant::now();
        runtime.shutdown(grace).await;
        assert!(
            began.elapsed() <= grace * 2,
            "the walk ended within the window: {:?}",
            began.elapsed()
        );
        assert_eq!(park.grace(), None, "no graceful cancel on shutdown (OQ-5)");
        assert_eq!(
            relay_row(&fixture, parked.id).status,
            PermissionStatus::Stale,
            "a hard-dropped walk's request is staled, never answered `cancelled` (OQ-5, A-11)"
        );
        let view = fixture
            .store
            .relay_view(ids::HTUI_ANA_2)
            .await
            .expect("the view answers");
        assert!(
            view.permissions.is_empty(),
            "the released lease hides the row: {view:?}"
        );
    }

    /// Plan D9: an executor this build does not know fails closed. The TUI refuses `R` and the
    /// walking commands with the sentence naming it, writing nothing; `c` still cancels.
    #[tokio::test]
    async fn an_unknown_executor_refuses_start_and_walking_commands() {
        let fixture = fixture_with_box_settings(
            json!({ "executor": "container", "max_concurrent_items": 2 }),
        )
        .await;
        let refusal = unknown_executor(&Executor::Other("container".to_owned()));
        // A run parked by a runtime of the other role, which serves commands whatever the box
        // says: only the TUI's commands are gated by the executor (plan D9).
        let mut other = fixture.runtime().with_role(Role::Worker);
        let CommandOutcome::Started { run, rest } =
            outcome(served(&mut other, &fixture, 1, start_run(ids::HTUI_ANA_2)).await)
        else {
            panic!("a start answers Started");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let research = step_at(&fixture, run, 0).await;
        let mut runtime = fixture.runtime();

        let reply = served(&mut runtime, &fixture, 1, start_run(ids::HTUI_CLEAN_1)).await;
        assert!(
            matches!(&reply, StoreReply::Failed { request: "start_run", message } if *message == refusal),
            "{reply:?}"
        );
        assert!(
            fixture
                .store
                .runs(ids::HTUI_CLEAN_1)
                .await
                .expect("the read answers")
                .is_empty(),
            "nothing was enqueued"
        );
        let reply = served(
            &mut runtime,
            &fixture,
            2,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        )
        .await;
        assert!(
            matches!(&reply, StoreReply::Failed { request: "answer_gate", message } if *message == refusal),
            "{reply:?}"
        );
        assert_eq!(
            step_at(&fixture, run, 0).await.status,
            StepStatus::AwaitingApproval,
            "the gate is unanswered"
        );

        let reply = served(
            &mut runtime,
            &fixture,
            3,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run })),
        )
        .await;
        assert!(
            matches!(outcome(reply), CommandOutcome::Cancelled { .. }),
            "a cancel walks nothing, so it is not gated"
        );
        assert_eq!(fixture.run(run).await.status, RunStatus::Cancelled);
    }

    /// OQ-5: on a `tui` box the TUI's sweep claims a queued row another process left behind, and
    /// walks it.
    #[tokio::test]
    async fn a_tui_box_claims_a_queued_row_it_did_not_queue() {
        let fixture = Fixture::new().await;
        // Left `queued` by a process that enqueued it and died before its claim.
        let queued = queued_by_another(&fixture, ids::HTUI_ANA_2).await;
        let mut runtime = fixture.runtime();

        let (replies, _answers) = mpsc::unbounded_channel();
        runtime.sweep(&Backend::memory(fixture.store.clone()), &replies);
        rests_at(&fixture, queued, RunStatus::AwaitingApproval).await;
        let row = fixture.run(queued).await;
        assert_eq!(row.executing_box_id, Some(ids::BOX), "claimed on this box");
        assert!(!fixture.steps(queued).await.is_empty(), "and walked");
    }

    /// MOD-41 finding T9-V1: `--demo` is the production runtime over `MemStore::demo()`, whose
    /// seeded `queued` run (`RUN_2` on `FEAT-3`) is a showcase. A sweep there claims nothing, so
    /// no production isolator, driver or scratch tree ever touches it.
    #[tokio::test]
    async fn the_demo_stores_production_sweep_leaves_its_queued_run_queued() {
        let store = MemStore::demo();
        let backend = Backend::memory(store.clone());
        let mut runtime = production_for(&backend);

        let (replies, _answers) = mpsc::unbounded_channel();
        runtime.sweep(&backend, &replies);
        assert!(runtime.settle(PATIENCE).await.is_empty(), "no task stuck");

        let row = store
            .run(ids::RUN_2)
            .await
            .expect("the read answers")
            .expect("the demo's queued run");
        assert_eq!(
            (row.status, row.executing_box_id, row.lease_expires_at),
            (RunStatus::Queued, None, None),
            "the showcase run is not claimed"
        );
        let item = store
            .item(ids::HTUI_FEAT_3)
            .await
            .expect("the read answers")
            .expect("the demo item");
        assert_eq!(
            item.status,
            Status::Queued,
            "its item still shows it queued"
        );
        assert!(
            store
                .run_steps(ids::RUN_2)
                .await
                .expect("the read answers")
                .iter()
                .all(|step| step.status == StepStatus::Pending),
            "and nothing walked"
        );
    }

    /// Plan D13 (`tui → worker`): a TUI walk in flight when the box flips keeps its lease and its
    /// heartbeat and rests normally; a sweep meanwhile adopts nothing. A 6 s TTL beats every 2 s
    /// (plan D124), so the heartbeat is seen moving the lease after the flip.
    #[tokio::test]
    async fn flipping_to_worker_keeps_the_tuis_live_walk() {
        let fixture = Fixture::new().await;
        fixture.store.set_app_setting("lease_ttl_seconds", json!(6));
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut runtime = fixture.runtime();
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        // A run of another item whose foreign lease lapsed: what a `tui` box's sweep would adopt.
        let lapsed = stranded(&fixture, ids::AGY_FEAT_1).await;
        let before = fixture.run(lapsed).await;
        set_executor(&fixture, Executor::Worker).await;

        runtime.sweep(&backend, &replies);
        let flipped = fixture.run(run).await.lease_expires_at;
        assert!(flipped.is_some(), "leased when the box flips");
        within("a heartbeat after the flip", async {
            while fixture.run(run).await.lease_expires_at <= flipped {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Running, "the walk is still live");
        assert!(
            row.lease_expires_at.is_some_and(|until| until > Utc::now()),
            "and still leased: {:?}",
            row.lease_expires_at
        );
        assert!(!stall.dropped.load(Ordering::SeqCst), "the session lives");

        stall.release.notify_one();
        assert!(runtime.settle(PATIENCE).await.is_empty());
        assert_eq!(fixture.run(run).await.status, RunStatus::AwaitingApproval);
        drop(replies);
        let mut started = None;
        while let Ok(envelope) = answers.try_recv() {
            if envelope.seq == 1 && !matches!(envelope.reply, StoreReply::RunStream(_)) {
                started = Some(outcome(envelope.reply));
            }
        }
        assert!(
            matches!(&started, Some(CommandOutcome::Started { rest, .. }) if rest.run == RunStatus::AwaitingApproval),
            "the start rested normally: {started:?}"
        );
        let row = fixture.run(lapsed).await;
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Running, before.lease_expires_at),
            "the sweep in between adopted nothing: the lease is still the lapsed stranger's"
        );
        assert!(fixture.steps(lapsed).await.is_empty(), "nothing walked");
    }

    /// Plan D13 (`worker → tui`): the worker's walk in flight when the box flips back keeps its
    /// lease and its heartbeat and rests normally; the worker stops claiming at its next sweep
    /// (OQ-2), so a row queued meanwhile stays `queued` for the TUI, even once nothing holds its
    /// scope. A 6 s TTL beats every 2 s (plan D124).
    #[tokio::test]
    async fn flipping_to_tui_keeps_the_workers_live_walk() {
        let fixture = Fixture::new().await;
        fixture.store.set_app_setting("lease_ttl_seconds", json!(6));
        set_executor(&fixture, Executor::Worker).await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut runtime = fixture.runtime().with_role(Role::Worker);
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        runtime
            .serve(
                &backend,
                &replies,
                &RequestEnvelope {
                    seq: 1,
                    origin: Origin::App,
                    request: start_run(ids::HTUI_ANA_2),
                },
                &LiveChats::default(),
            )
            .await;
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        set_executor(&fixture, Executor::Tui).await;
        let waiting = queued_by_another(&fixture, ids::HTUI_CLEAN_1).await;

        runtime.sweep(&backend, &replies);
        let flipped = fixture.run(run).await.lease_expires_at;
        assert!(flipped.is_some(), "leased when the box flips");
        within("a heartbeat after the flip", async {
            while fixture.run(run).await.lease_expires_at <= flipped {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Running, "the walk is still live");
        assert!(
            row.lease_expires_at.is_some_and(|until| until > Utc::now()),
            "and still leased: {:?}",
            row.lease_expires_at
        );
        assert!(!stall.dropped.load(Ordering::SeqCst), "the session lives");

        stall.release.notify_one();
        assert!(runtime.settle(PATIENCE).await.is_empty());
        assert_eq!(fixture.run(run).await.status, RunStatus::AwaitingApproval);
        let mut started = None;
        while let Ok(envelope) = answers.try_recv() {
            if envelope.seq == 1 && !matches!(envelope.reply, StoreReply::RunStream(_)) {
                started = Some(outcome(envelope.reply));
            }
        }
        assert!(
            matches!(&started, Some(CommandOutcome::Started { rest, .. }) if rest.run == RunStatus::AwaitingApproval),
            "the start rested normally: {started:?}"
        );

        // Nothing holds the waiting row's scope any more, so only I-1 keeps the worker off it.
        fixture
            .store
            .finish_run(run, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the parked run is cancellable");
        swept(&mut runtime, &fixture).await;
        let row = fixture.run(waiting).await;
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Queued, None),
            "I-1: a `tui` box's queued row is the TUI's to claim"
        );
        assert!(fixture.steps(waiting).await.is_empty(), "nothing walked");
    }

    /// Blueprint F-17, B-7: a claim this process's queue retries is I-1's too. After the flip to
    /// `worker`, a walk that rests does not claim the waiting run: it leaves the queue and stays
    /// `queued` for the box's worker.
    #[tokio::test]
    async fn flipping_to_worker_stops_the_in_memory_claim_retry() {
        let fixture = Fixture::new().await;
        let runtime = fixture.runtime();
        let probe = testing::probe(&runtime);
        let mut worker = Worker::spawn(&fixture.store, runtime);
        let (first, _) = parked(&fixture, &mut worker).await;
        let start = worker.send(Origin::App, start_run(ids::HTUI_CLEAN_1));
        let StoreReply::Failed { message, .. } = worker.reply(start).await else {
            panic!("the second claim is refused");
        };
        assert!(message.contains("overlaps run"), "{message}");
        let second = only_run(&fixture.store, ids::HTUI_CLEAN_1).await;
        assert!(
            probe.queued().iter().any(|(_, run)| *run == second),
            "the refused run waits in the queue"
        );
        set_executor(&fixture, Executor::Worker).await;

        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run: first })),
        );
        outcome(worker.reply(cancel).await);
        within("the waiting run leaving the queue", async {
            while probe.queued().iter().any(|(_, run)| *run == second) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let row = fixture.run(second).await;
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Queued, None),
            "I-1: the box's worker claims it, not the TUI"
        );
    }

    /// Plan D9, fact-check F-M2: a malformed sibling key fails the whole-struct decode, but the
    /// gate reads the `executor` key alone, so the box is still a `worker` box: `R` only queues.
    #[tokio::test]
    async fn a_worker_box_with_a_bad_sibling_setting_is_still_a_worker_box() {
        let fixture =
            fixture_with_box_settings(json!({ "executor": "worker", "max_concurrent_items": "2" }))
                .await;
        let mut runtime = fixture.runtime();

        let reply = served(&mut runtime, &fixture, 1, start_run(ids::HTUI_ANA_2)).await;
        let CommandOutcome::Started { run, rest } = outcome(reply) else {
            panic!("a start answers Started");
        };
        assert_eq!(rest.run, RunStatus::Queued, "I-1 never fails open");
        let row = fixture.run(run).await;
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Queued, None)
        );
        assert!(fixture.steps(run).await.is_empty());
    }
}
