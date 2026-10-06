//! MOD-41 T12 against live Postgres (blueprint §14.6): `htui worker`'s loop, its headless connect
//! and its binary.
//!
//! Cases 1 to 3 run `htui_worker::worker::run` in process, over `db.store`, with a
//! `RunRuntime<PgStore, Unaddressed>` in the worker role whose isolator and verifier are the fakes
//! and whose sessions play one turn (plan D15 with test parts). The TUI beside it is a minimal copy
//! of `runs_pg.rs`'s stack (blueprint F-29): the TUI's `RunRuntime` over a `Backend::Online`,
//! served through `TuiRuns` exactly as the store loop serves it, so a case reads the command's own
//! answer. The two runtimes share one fake isolator and verifier, as two processes on one box
//! share its filesystem.
//!
//! MOD-42 T6 (blueprint §9.2) adds two relay cases to the loop in process: a worker walk whose
//! session parks on a permission request is answered by a second `PgStore` client registered as
//! **another box** (as a TUI elsewhere holding the DSN would), and a cancel that client requests
//! ends a live worker walk through the worker loop's own command poll. Their worker sessions play
//! [`ParksOnce`] through [`Parts::runtime_with`]; the relay polls at its production interval.
//!
//! MOD-70 T4 (blueprint §8) adds three follow-up cases to the same relay setup: the parking
//! session has follow-up turns ([`ParksOnce::then`]), box B queues a follow-up while it is parked
//! and then answers or cancels, and each case reads the transcript's `follow_up` row and probes
//! `run_command` directly for `text IS NULL` (PRD metric 3), which no store method returns.
//!
//! Case 4 is the headless connect's refusal, and cases 5 to 8 spawn the `htui` binary with a
//! cleared environment whose `HOME` and `XDG_CONFIG_HOME` are a temporary directory, so nothing
//! reaches the developer's `~/.config/htui` (blueprint F-22). No case walks a run with production
//! parts. Cases 7 and 8 (a signal during startup) need no server and never skip.
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set, like every other Postgres-backed suite.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use htui::run_worker::{LiveChats, OrchReply, OrchRequest, RunServed, StepAuthor, TuiRuns as _};
use htui::store_worker::{Origin, RequestEnvelope, StoreReply, StoreRequest};
use htui::worker_cmd::{self, WorkerExit};
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::driver::{AgentDriver, DriverCaps, PermissionRequestId};
use htui_agent::event::{
    DoneEvent, DriverEvent, PermissionOption, PermissionOptionKind, PermissionRequestEvent,
    StopReason, TextChunk, ToolCallEvent, ToolKind,
};
use htui_agent::fake::FakeDriver;
use htui_agent::registry::{DriverFactory, TransportBuilder};
use htui_core::fixtures::{edit_agent, ids};
use htui_core::model::{
    Agent, AgentBox, AgentId, AnswerOutcome, Billing, BoxEdit, BoxId, DocumentId, EventKind,
    Executor, FOLLOW_UP_RUN_CANCELLED, FOLLOW_UP_SESSION_ENDED, FollowUpRefusal, FollowUpRequest,
    FollowUpText, ItemId, NewDocument, NewFollowUp, NewRepo, PermissionStatus, RepoId,
    RunCommandId, RunCommandStatus, RunId, RunMode, RunStatus, RunStep, SnapshotPhase, Status,
    StepId, StepPermission, StepStatus, Transport,
};
use htui_core::store::{CasOutcome, ReadStore as _, WriteStore as _};
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use htui_orch::{Command, CommandOutcome, GateAnswer, Isolator, Verifier};
use htui_store::pg::PoolSize;
use htui_store::{Backend, CacheStore, PgStore, testkit};
use htui_worker::worker::{self, COMMAND_POLL, WorkerConfig};
use htui_worker::{CANCEL_GRACE, Role, RunRuntime, Unaddressed};
use serde_json::json;
use tokio::sync::oneshot;

/// How long a case waits for a row to reach the state it expects.
const PATIENCE: Duration = Duration::from_secs(30);

/// Blueprint §14.6: a fast poll, a short beat, no walk grace.
const CONFIG: WorkerConfig = WorkerConfig {
    poll: Duration::from_millis(50),
    box_beat: Duration::from_secs(1),
    grace: Duration::ZERO,
};

/// Case 1's: [`CONFIG`] with the production box beat, so its start beat can land within
/// [`PATIENCE`] only if the first tick is at start (blueprint B-11), not one period out.
const START_BEAT: WorkerConfig = WorkerConfig {
    box_beat: htui_store::connect::BOX_HEARTBEAT,
    ..CONFIG
};
const _: () = assert!(
    START_BEAT.box_beat.as_secs() > PATIENCE.as_secs(),
    "a first beat one period out cannot land within the patience window"
);

// ---------------------------------------------------------------------------------------------
// The parts
// ---------------------------------------------------------------------------------------------

/// Blueprint D203's author: one document of the phase's `output_kind` per step. A `review`'s body
/// approves, so an ungated graph walks to `done`; gated, the review parks for a human either way.
#[derive(Debug)]
struct OutputAuthor;

impl StepAuthor for OutputAuthor {
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument> {
        let body = if phase.name == "review" {
            "---\nverdict: approve\n---\nfine"
        } else {
            "authored"
        };
        Some(NewDocument {
            id: DocumentId::new(),
            item_id: item,
            kind: phase.output_kind.clone(),
            title: format!("{} (attempt {})", phase.output_kind, step.attempt),
            body: body.to_owned(),
            produced_by_step_id: Some(step.id),
            created_by: ids::USER,
            created_at: Utc::now(),
        })
    }
}

/// The walk's transport: every session plays one turn and ends it.
#[derive(Debug)]
struct Walks;

impl TransportBuilder for Walks {
    fn build(
        &self,
        agent: &Agent,
        _on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, htui_agent::error::DriverError> {
        Ok(Box::new(FakeDriver::new(
            agent.name.clone(),
            caps,
            Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            }))]),
        )))
    }
}

/// MOD-42 T6: the first session a runtime builds plays a gated `execute` call and parks on its
/// permission request (one allow and one reject option), then ends its turn once answered; every
/// later session plays [`Walks`]'s one turn, so an ungated walk reaches `done` after the answer.
///
/// MOD-70 T4: [`ParksOnce::then`] gives that first session `follow_ups` more turns, each one
/// [`a_follow_up_turn`], which only a follow-up starts (B-9: the park is the rendezvous).
#[derive(Debug, Default)]
struct ParksOnce {
    built: AtomicBool,
    follow_ups: usize,
}

impl ParksOnce {
    /// The parking session, with `follow_ups` turns after the parked one.
    fn then(follow_ups: usize) -> Self {
        Self {
            built: AtomicBool::new(false),
            follow_ups,
        }
    }
}

/// MOD-70 T4: the turn a follow-up starts: a line of text, then the end of the turn.
fn a_follow_up_turn() -> Vec<ScriptEvent> {
    vec![
        ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
            text: "the edge cases are covered".to_owned(),
            message_id: None,
        })),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ]
}

impl TransportBuilder for ParksOnce {
    fn build(
        &self,
        agent: &Agent,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, htui_agent::error::DriverError> {
        if self.built.swap(true, Ordering::SeqCst) {
            return Walks.build(agent, on_box, caps);
        }
        let mut turns = vec![parked_turn()];
        turns.extend((0..self.follow_ups).map(|_| a_follow_up_turn()));
        Ok(Box::new(FakeDriver::new(
            agent.name.clone(),
            caps,
            Script::turns(turns),
        )))
    }
}

/// [`ParksOnce`]'s first turn: a gated `execute` call, its parked request, the end of the turn.
fn parked_turn() -> Vec<ScriptEvent> {
    vec![
        ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "run the suite".to_owned(),
            tool_kind: ToolKind::Execute,
            input: json!({ "command": "cargo test" }),
            locations: Vec::new(),
        })),
        ScriptEvent::ParkPermission(PermissionRequestEvent {
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
        }),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ]
}

/// The fake isolator and verifier one box's processes share.
#[derive(Clone)]
struct Parts {
    isolator: Arc<FakeIsolator>,
    verifier: Arc<FakeVerifier>,
}

impl Parts {
    fn new() -> Self {
        Self {
            isolator: Arc::new(FakeIsolator::new()),
            verifier: Arc::new(FakeVerifier::new()),
        }
    }

    /// A run runtime over these parts, whose sessions play [`Walks`].
    fn runtime<H: htui_core::store::WorkerHost, P: htui_worker::ReplySink>(
        &self,
    ) -> RunRuntime<H, P> {
        self.runtime_with(Box::new(Walks))
    }

    /// A run runtime over these parts, whose `acp` sessions `transport` builds (MOD-42 T6).
    fn runtime_with<H: htui_core::store::WorkerHost, P: htui_worker::ReplySink>(
        &self,
        transport: Box<dyn TransportBuilder>,
    ) -> RunRuntime<H, P> {
        let mut factory = DriverFactory::new();
        factory.register("acp", transport);
        RunRuntime::with_parts(
            Arc::clone(&self.isolator) as Arc<dyn Isolator>,
            Arc::clone(&self.verifier) as Arc<dyn Verifier>,
            factory,
        )
        .with_author(Arc::new(OutputAuthor))
    }
}

/// `runs_pg.rs`'s seed: the demo's agents disabled, one scripted `acp` row ready on this box, and
/// a primary repo for the demo project.
async fn seed(store: &PgStore) {
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        edit_agent(store, &row).await.expect("the row is disabled");
    }
    let agent_id = AgentId::new();
    let at = Utc::now();
    store
        .upsert_agent(
            &Agent {
                id: agent_id,
                name: "scripted".to_owned(),
                transport: Transport::Acp,
                billing: Billing::Subscription,
                models: Vec::new(),
                default_model: Some("sonnet".to_owned()),
                launch: json!({ "command": "unused", "args": [] }),
                settings: json!({}),
                enabled: true,
                created_at: at,
                updated_at: at,
            },
            None,
        )
        .await
        .expect("the scripted row lands");
    store
        .upsert_agent_box(&AgentBox {
            agent_id,
            box_id: store.this_box(),
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(at),
            quota: None,
            quota_at: None,
            updated_at: at,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        })
        .await
        .expect("the agent_box row lands");
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
    // `FEAT-3` is seeded `queued` under a live `RUN_2`; cancelling it moves the item to `open`.
    store
        .finish_run(ids::RUN_2, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("the seeded run is queued and cancellable");
}

// ---------------------------------------------------------------------------------------------
// The TUI and the worker
// ---------------------------------------------------------------------------------------------

/// The database, the mirror, and the TUI's run runtime over `Backend::Online`.
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    backend: Backend,
    parts: Parts,
    tui: Option<htui::run_worker::RunRuntime>,
    seq: u64,
}

impl Stack {
    /// The stack over a seeded demo database, or `None` (after `testkit::SKIP`) without a server.
    async fn new() -> Option<Self> {
        let db = testkit::demo_db().await?;
        seed(&db.store).await;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), "worker-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        let parts = Parts::new();
        let tui = parts.runtime();
        Some(Self {
            db,
            _root: root,
            cache,
            backend,
            parts,
            tui: Some(tui),
            seq: 0,
        })
    }

    /// One command from the TUI, its tasks settled, and its answer.
    async fn command(&mut self, command: Command) -> CommandOutcome {
        self.seq += 1;
        let seq = self.seq;
        let tui = self.tui.as_mut().expect("the TUI is still running");
        let (replies, mut answers) = tokio::sync::mpsc::unbounded_channel();
        let served = tui
            .serve(
                &self.backend,
                &replies,
                &RequestEnvelope {
                    seq,
                    origin: Origin::App,
                    request: StoreRequest::Orch(OrchRequest::Command(command)),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred), "{served:?}");
        assert!(tui.settle(PATIENCE).await.is_empty(), "no TUI task stuck");
        drop(replies);
        let mut reply = None;
        while let Ok(envelope) = answers.try_recv() {
            if envelope.seq == seq && !matches!(envelope.reply, StoreReply::RunStream(_)) {
                reply = Some(envelope.reply);
            }
        }
        match reply.expect("the command was answered") {
            StoreReply::Orch(OrchReply::Done(outcome)) => *outcome,
            other => panic!("expected a command outcome, got {other:?}"),
        }
    }

    /// `R` on `item`: the run and where its first walk rested.
    async fn start(&mut self, item: ItemId) -> (RunId, htui_orch::Rest) {
        match self
            .command(Command::StartRun {
                item,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
        {
            CommandOutcome::Started { run, rest } => (run, rest),
            other => panic!("a start answers Started, got {other:?}"),
        }
    }

    /// The TUI exits: its runtime shuts down (walks cancelled, leases given back) and is gone.
    async fn exit_tui(&mut self) {
        let mut tui = self.tui.take().expect("the TUI is still running");
        tui.shutdown(Duration::ZERO).await;
    }

    /// `htui worker`'s loop over this database, sharing the TUI's parts.
    fn spawn_worker(&self, config: WorkerConfig) -> Running {
        self.spawn_worker_with(config, Box::new(Walks))
    }

    /// [`Stack::spawn_worker`], whose `acp` sessions `transport` builds (MOD-42 T6).
    fn spawn_worker_with(
        &self,
        config: WorkerConfig,
        transport: Box<dyn TransportBuilder>,
    ) -> Running {
        let runtime: RunRuntime<PgStore, Unaddressed> =
            self.parts.runtime_with(transport).with_role(Role::Worker);
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(worker::run(
            self.db.store.clone(),
            runtime,
            config,
            async move {
                let _ = stopped.await;
            },
        ));
        Running { stop, task }
    }

    /// The box's executor, written through its editor (MOD-41 plan D10).
    async fn set_executor(&self, executor: Executor) {
        let row = self
            .db
            .store
            .box_row(ids::BOX)
            .await
            .expect("the read answers")
            .expect("the demo box");
        let edited = self
            .db
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

    /// Every phase of `htui`'s default `FEAT` graph ungated, so a walk reaches `done` alone.
    async fn ungate_feat(&self) {
        sqlx::query(
            "UPDATE step_graph_phase SET gate = 'never', gate_hard = false WHERE graph_id = $1",
        )
        .bind(ids::GRAPH_HTUI_FEAT.as_uuid())
        .execute(&self.db.pool)
        .await
        .expect("ungate the FEAT graph");
    }

    async fn run_row(&self, run: RunId) -> htui_core::model::Run {
        self.db
            .store
            .run(run)
            .await
            .expect("the read answers")
            .expect("the run exists")
    }

    async fn steps(&self, run: RunId) -> Vec<RunStep> {
        self.db
            .store
            .run_steps(run)
            .await
            .expect("the read answers")
    }

    /// The one step of `run` a gate is waiting on.
    async fn parked_step(&self, run: RunId) -> RunStep {
        let parked: Vec<RunStep> = self
            .steps(run)
            .await
            .into_iter()
            .filter(|step| step.status == StepStatus::AwaitingApproval)
            .collect();
        assert_eq!(parked.len(), 1, "one parked step: {parked:?}");
        parked.into_iter().next().expect("checked above")
    }

    /// `run.lease_owner`, which the `Run` row deliberately does not carry.
    async fn lease_owner(&self, run: RunId) -> Option<uuid::Uuid> {
        sqlx::query_scalar::<_, Option<uuid::Uuid>>("SELECT lease_owner FROM run WHERE id = $1")
            .bind(run.as_uuid())
            .fetch_one(&self.db.pool)
            .await
            .expect("read run.lease_owner")
    }

    /// Polls until `run`'s lease is given back: `release_lease` is its own write, after the walk
    /// has settled the run, so a rested run may still hold it for a moment.
    async fn released(&self, run: RunId) {
        let deadline = Instant::now() + PATIENCE;
        while let Some(owner) = self.lease_owner(run).await {
            assert!(
                Instant::now() < deadline,
                "run {run}'s lease was not given back within {PATIENCE:?}: {owner} holds it"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn last_seen(&self, id: BoxId) -> chrono::DateTime<Utc> {
        sqlx::query_scalar::<_, chrono::DateTime<Utc>>("SELECT last_seen_at FROM box WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(&self.db.pool)
            .await
            .expect("read box.last_seen_at")
    }

    /// Polls until `run` rests: not `queued` or `running`, and no step `running`.
    async fn rested(&self, run: RunId) -> htui_core::model::Run {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let row = self.run_row(run).await;
            let walking = self
                .steps(run)
                .await
                .iter()
                .any(|step| step.status == StepStatus::Running);
            if !matches!(row.status, RunStatus::Queued | RunStatus::Running) && !walking {
                return row;
            }
            assert!(
                Instant::now() < deadline,
                "run {run} did not rest within {PATIENCE:?}: it is `{}`",
                row.status
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Closes the mirror and drops the database. Every case calls this on its last line.
    async fn finish(mut self) {
        if let Some(mut tui) = self.tui.take() {
            tui.shutdown(Duration::ZERO).await;
        }
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

/// A worker loop on its own task, stopped through a `oneshot` as a signal would.
struct Running {
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    /// Resolves the shutdown future and awaits the loop's end.
    async fn stop(self) {
        let _ = self.stop.send(());
        tokio::time::timeout(PATIENCE, self.task)
            .await
            .expect("the worker stops within the patience window")
            .expect("the worker loop did not panic");
    }
}

/// `(position, attempt, fanout_index, phase, status)` of every step, in that order.
fn shape(steps: &[RunStep]) -> Vec<(i32, i32, i32, String, StepStatus)> {
    let mut shape: Vec<_> = steps
        .iter()
        .map(|step| {
            (
                step.position,
                step.attempt,
                step.fanout_index,
                step.phase_name.clone(),
                step.status,
            )
        })
        .collect();
    shape.sort_by_key(|step| (step.0, step.1, step.2));
    shape
}

// ---------------------------------------------------------------------------------------------
// Cases 1 to 3: the loop in process
// ---------------------------------------------------------------------------------------------

/// Plan D14, B-11: on a `worker` box a run the TUI queued is claimed by the worker and walked to
/// `done`, its lease given back; the worker marks the box seen at once, not a minute later.
#[tokio::test(flavor = "multi_thread")]
async fn a_headless_worker_drives_a_queued_run_to_rest() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    stack.ungate_feat().await;
    stack.set_executor(Executor::Worker).await;
    let (run, rest) = stack.start(ids::HTUI_FEAT_3).await;
    assert_eq!(
        rest.run,
        RunStatus::Queued,
        "the TUI only queues (plan D12)"
    );
    let seen = stack.last_seen(ids::BOX).await;

    let worker = stack.spawn_worker(START_BEAT);
    let deadline = Instant::now() + PATIENCE;
    while stack.last_seen(ids::BOX).await <= seen {
        assert!(
            Instant::now() < deadline,
            "the worker's first box beat is at start (blueprint B-11)"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let row = stack.rested(run).await;
    assert_eq!(
        row.status,
        RunStatus::Done,
        "the worker walked it to the end"
    );
    assert_eq!(row.lease_box_id, Some(ids::BOX));
    stack.released(run).await;
    let steps = stack.steps(run).await;
    assert!(
        !steps.is_empty() && steps.iter().all(|step| step.status == StepStatus::Done),
        "every step is done: {:?}",
        shape(&steps)
    );
    worker.stop().await;
    stack.finish().await;
}

/// I-1: a TUI that exits does not interrupt the worker's run. `R` answers `queued` and writes no
/// lease; the TUI shuts down; the worker settles the run to `done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_tui_exit_does_not_interrupt_a_worker_run() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    stack.ungate_feat().await;
    stack.set_executor(Executor::Worker).await;
    let (run, rest) = stack.start(ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::Queued);
    let row = stack.run_row(run).await;
    assert_eq!(
        (row.status, row.lease_expires_at),
        (RunStatus::Queued, None),
        "the TUI wrote no lease"
    );

    let worker = stack.spawn_worker(CONFIG);
    stack.exit_tui().await;
    let row = stack.rested(run).await;
    assert_eq!(row.status, RunStatus::Done, "the worker settled the run");
    stack.released(run).await;
    worker.stop().await;
    stack.finish().await;
}

/// `FEAT-3` walked in the TUI (executor `tui`) through `prd`, `plan` and `implement` to its
/// parked `review`.
async fn walk_to_review(stack: &mut Stack) -> (RunId, RunStep) {
    let (run, rest) = stack.start(ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "the TUI walked it");
    for _ in 0..3 {
        let parked = stack.parked_step(run).await;
        assert_ne!(
            parked.phase_name, "review",
            "three phases before the review"
        );
        let answered = stack
            .command(Command::AnswerGate {
                run,
                step: parked.id,
                answer: GateAnswer::Approved,
            })
            .await;
        assert!(
            matches!(answered, CommandOutcome::Answered { .. }),
            "{answered:?}"
        );
    }
    let review = stack.parked_step(run).await;
    assert_eq!(review.phase_name, "review");
    (run, review)
}

/// A rejected review, as the TUI answers it: `x` with a note.
fn reject(run: RunId, review: &RunStep) -> Command {
    Command::AnswerGate {
        run,
        step: review.id,
        answer: GateAnswer::Rejected {
            note: "no tests".to_owned(),
        },
    }
}

/// Plan D12, OQ-6: on a `worker` box the TUI records `x` on a review and hands the run back
/// (`running`, no lease); the worker walks the review loop to the same rest the TUI reaches in
/// process on a `tui` box.
#[tokio::test(flavor = "multi_thread")]
async fn an_answer_handed_back_is_finished_by_the_worker() {
    let Some(mut twin) = Stack::new().await else {
        return;
    };
    let (twin_run, twin_review) = walk_to_review(&mut twin).await;
    let in_process = twin.command(reject(twin_run, &twin_review)).await;
    let CommandOutcome::Answered { rest: twin_rest } = in_process else {
        panic!("an answer answers Answered, got {in_process:?}");
    };
    let expected = (
        twin.run_row(twin_run).await.status,
        shape(&twin.steps(twin_run).await),
        twin.db
            .store
            .item(ids::HTUI_FEAT_3)
            .await
            .expect("the read answers")
            .expect("the item")
            .status,
    );
    assert_eq!(twin_rest.run, expected.0);
    twin.finish().await;

    let mut stack = Stack::new().await.expect("the server answered the twin");
    let (run, review) = walk_to_review(&mut stack).await;
    stack.set_executor(Executor::Worker).await;
    let handed = stack.command(reject(run, &review)).await;
    let CommandOutcome::Answered { rest } = handed else {
        panic!("an answer answers Answered, got {handed:?}");
    };
    assert_eq!(rest.run, RunStatus::Running, "handed back, not walked");
    assert_eq!(stack.run_row(run).await.status, RunStatus::Running);
    assert_eq!(
        stack.lease_owner(run).await,
        None,
        "the lease was given back"
    );

    let worker = stack.spawn_worker(CONFIG);
    let row = stack.rested(run).await;
    let item: Status = stack
        .db
        .store
        .item(ids::HTUI_FEAT_3)
        .await
        .expect("the read answers")
        .expect("the item")
        .status;
    assert_eq!(
        (row.status, shape(&stack.steps(run).await), item),
        expected,
        "the worker's walk rests where the TUI's in-process walk does"
    );
    worker.stop().await;
    stack.finish().await;
}

// ---------------------------------------------------------------------------------------------
// MOD-42 T6: the relay across boxes
// ---------------------------------------------------------------------------------------------

/// PRD metric "answer → resume ≤ ~2 s": asserted below 3 s at the production relay poll.
const ANSWER_TO_DONE: Duration = Duration::from_secs(3);
const _: () = assert!(
    htui_agent::RELAY_POLL.as_millis() == 1000,
    "the answer case runs at the production 1 s relay poll (plan D8)"
);

/// Plan T6: a cancel ends a live worker walk within the grace, one command poll, and a margin.
const CANCEL_BOUND: Duration =
    Duration::from_secs(CANCEL_GRACE.as_secs() + COMMAND_POLL.as_secs() + 2);

/// A second store client registered as **another box** (blueprint §9.2), as a TUI elsewhere
/// holding the DSN; its `box.toml` lives in the returned directory, never the real home.
async fn another_box(db: &testkit::TestDb) -> (PgStore, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let identity = htui_store::identity::load_or_mint(root.path()).expect("mint box.toml");
    let client = PgStore::connect(&db.url, &identity)
        .await
        .expect("the second client connects")
        .store;
    assert_ne!(
        client.this_box(),
        ids::BOX,
        "the second client is another box"
    );
    (client, root)
}

/// The first pending request the second client's Runs pane would show for `item` (D14).
async fn pending_request(client: &PgStore, item: ItemId) -> StepPermission {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let view = client.relay_view(item).await.expect("the relay view reads");
        if let Some(row) = view.permissions.into_iter().next() {
            return row;
        }
        assert!(
            Instant::now() < deadline,
            "no request parked within {PATIENCE:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The `permission_answer` payloads in `step`'s log.
async fn permission_answers(store: &PgStore, step: StepId) -> Vec<serde_json::Value> {
    store
        .step_events(step)
        .await
        .expect("the log reads")
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row.kind == EventKind::PermissionAnswer)
        .map(|row| row.payload)
        .collect()
}

/// `run_command.status` of every command of `run`, in issue order: no store method lists them.
async fn command_statuses(stack: &Stack, run: RunId) -> Vec<RunCommandStatus> {
    sqlx::query_scalar::<_, String>(
        "SELECT status FROM run_command WHERE run_id = $1 ORDER BY issued_at, id",
    )
    .bind(run.as_uuid())
    .fetch_all(&stack.db.pool)
    .await
    .expect("read run_command")
    .iter()
    .map(|status| status.parse().expect("a known command status"))
    .collect()
}

/// Polls until `step` is `want`; the time it took.
async fn step_reaches(stack: &Stack, run: RunId, step: StepId, want: StepStatus) -> Duration {
    let started = Instant::now();
    loop {
        let status = stack
            .steps(run)
            .await
            .into_iter()
            .find(|row| row.id == step)
            .expect("the step exists")
            .status;
        if status == want {
            return started.elapsed();
        }
        assert!(
            started.elapsed() < PATIENCE,
            "step {step} is `{status}`, not `{want}`, after {PATIENCE:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// MOD-42 PRD metrics "cross-box answer" and "answer → resume": the worker walks a queued run
/// whose session parks on a permission request; a client on another box sees it in its relay
/// view and answers it; the worker applies the answer and the step is `done` within
/// [`ANSWER_TO_DONE`] of the answer, at the production 1 s relay poll. The executor, not the
/// answerer, echoes the answer into the step's log (I-1), as the user's.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_parked_step_resumes_on_an_answer_from_another_box() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    stack.ungate_feat().await;
    stack.set_executor(Executor::Worker).await;
    let (run, rest) = stack.start(ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::Queued, "the TUI only queues");
    let worker = stack.spawn_worker_with(CONFIG, Box::new(ParksOnce::default()));
    let (client, _client_root) = another_box(&stack.db).await;

    let parked = pending_request(&client, ids::HTUI_FEAT_3).await;
    assert_eq!(parked.run_id, run);
    assert_eq!(parked.status, PermissionStatus::Pending);
    assert_eq!(
        parked
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect::<Vec<_>>(),
        ["allow-once", "reject-once"]
    );
    let answered = client
        .answer_permission(
            parked.id,
            "allow-once",
            client.this_user(),
            client.this_box(),
        )
        .await
        .expect("the answer is written");
    assert_eq!(answered, AnswerOutcome::Answered);
    let took = step_reaches(&stack, run, parked.run_step_id, StepStatus::Done).await;
    assert!(
        took < ANSWER_TO_DONE,
        "the step resumed {took:?} after the answer, not within {ANSWER_TO_DONE:?}"
    );

    let row = client
        .permission(parked.id)
        .await
        .expect("the read answers")
        .expect("the row exists");
    assert_eq!(
        (row.status, row.option_id.as_deref(), row.answered_box),
        (
            PermissionStatus::Applied,
            Some("allow-once"),
            Some(client.this_box())
        ),
        "applied by the worker, answered from the other box"
    );
    let answers = permission_answers(&stack.db.store, parked.run_step_id).await;
    assert_eq!(answers.len(), 1, "the executor echoed it once: {answers:?}");
    assert_eq!(
        (&answers[0]["option_id"], &answers[0]["by"]),
        (&json!("allow-once"), &json!("user")),
        "{answers:?}"
    );
    assert_eq!(stack.rested(run).await.status, RunStatus::Done);
    stack.released(run).await;
    let view = client
        .relay_view(ids::HTUI_FEAT_3)
        .await
        .expect("the view reads");
    assert!(
        view.permissions.is_empty() && view.cancels.is_empty(),
        "{view:?}"
    );
    worker.stop().await;
    stack.finish().await;
}

/// MOD-42 PRD metric "worker-walk cancel within grace + poll", and the worker loop's own
/// command-poll arm (T4 covers it only here): a client on another box requests a cancel of a run
/// the worker is walking while its session is parked. The worker's poll picks the row up and
/// cancels gracefully: the parked request answered `cancelled` and recorded (I-7), the step and
/// the run `cancelled`, the command `applied`, all within [`CANCEL_BOUND`] of the request.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_from_another_box_ends_a_live_worker_walk() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    stack.ungate_feat().await;
    stack.set_executor(Executor::Worker).await;
    let (run, _) = stack.start(ids::HTUI_FEAT_3).await;
    let worker = stack.spawn_worker_with(CONFIG, Box::new(ParksOnce::default()));
    let (client, _client_root) = another_box(&stack.db).await;
    let parked = pending_request(&client, ids::HTUI_FEAT_3).await;

    let asked = Instant::now();
    let requested = client
        .request_cancel(run, client.this_user(), client.this_box())
        .await
        .expect("the cancel is written");
    assert!(
        matches!(requested, htui_core::model::CancelRequest::Inserted(_)),
        "{requested:?}"
    );
    let deadline = asked + PATIENCE;
    while stack.run_row(run).await.status != RunStatus::Cancelled {
        assert!(Instant::now() < deadline, "the run was never cancelled");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let took = asked.elapsed();
    assert!(
        took < CANCEL_BOUND,
        "the walk ended {took:?} after the request, not within {CANCEL_BOUND:?}"
    );

    let row = client
        .permission(parked.id)
        .await
        .expect("the read answers")
        .expect("the row exists");
    assert_eq!(
        row.status,
        PermissionStatus::Cancelled,
        "I-7: answered `cancelled`"
    );
    let answers = permission_answers(&stack.db.store, parked.run_step_id).await;
    assert_eq!(answers.len(), 1, "I-7's row exactly once: {answers:?}");
    assert_eq!(
        (
            &answers[0]["option_id"],
            &answers[0]["by"],
            &answers[0]["cancelled"]
        ),
        (&serde_json::Value::Null, &json!("policy"), &json!(true)),
        "I-7: `{{option_id: null, by: \"policy\", cancelled: true}}`: {answers:?}"
    );
    // `cancel_leased` settled the step; the walk settled nothing (I-6).
    step_reaches(&stack, run, parked.run_step_id, StepStatus::Cancelled).await;
    let deadline = Instant::now() + PATIENCE;
    while command_statuses(&stack, run).await != [RunCommandStatus::Applied] {
        assert!(
            Instant::now() < deadline,
            "the command was not applied: {:?}",
            command_statuses(&stack, run).await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stack.released(run).await;
    let view = client
        .relay_view(ids::HTUI_FEAT_3)
        .await
        .expect("the view reads");
    assert!(
        view.permissions.is_empty() && view.cancels.is_empty(),
        "{view:?}"
    );
    worker.stop().await;
    stack.finish().await;
}

// ---------------------------------------------------------------------------------------------
// MOD-70 T4: follow-ups across boxes
// ---------------------------------------------------------------------------------------------

/// The follow-up box B types. Nothing a pattern-only scrubber refuses (D2), so the scrubbed
/// `follow_up` row the executor records carries it as typed.
const FOLLOW_UP: &str = "also cover the edge cases";

/// Box B queues `text` for `step` as its own user from its own box (D3).
async fn request_follow_up(client: &PgStore, step: StepId, text: &str) -> FollowUpRequest {
    client
        .request_follow_up(NewFollowUp {
            id: RunCommandId::new(),
            run_step_id: step,
            text: FollowUpText::new(text.to_owned()).expect("the client accepts the text"),
            issued_by: client.this_user(),
            issued_box: client.this_box(),
        })
        .await
        .expect("the enqueue is answered")
}

/// [`request_follow_up`], which must be queued; the row's id.
async fn queued(client: &PgStore, step: StepId, text: &str) -> RunCommandId {
    match request_follow_up(client, step, text).await {
        FollowUpRequest::Queued(id) => id,
        refused @ FollowUpRequest::Refused(_) => {
            panic!("the follow-up was not queued: {refused:?}")
        }
    }
}

/// Box B answers `row` with its allow option.
async fn allow(client: &PgStore, row: &StepPermission) {
    let answered = client
        .answer_permission(row.id, "allow-once", client.this_user(), client.this_box())
        .await
        .expect("the answer is written");
    assert_eq!(answered, AnswerOutcome::Answered);
}

/// One `run_command` row of kind `follow_up`, as the direct SQL probe reads it: PRD metric 3's
/// `text IS NULL` has no store method, by design (OQ-6).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FollowUpRow {
    id: RunCommandId,
    status: RunCommandStatus,
    text_is_null: bool,
    resolution: Option<String>,
    issued_box: BoxId,
}

/// Every follow-up row of `step`, in issue order.
async fn follow_up_rows(pool: &sqlx::PgPool, step: StepId) -> Vec<FollowUpRow> {
    sqlx::query_as::<_, (uuid::Uuid, String, bool, Option<String>, uuid::Uuid)>(
        "SELECT id, status, text IS NULL, resolution, issued_box FROM run_command \
         WHERE kind = 'follow_up' AND run_step_id = $1 ORDER BY issued_at, id",
    )
    .bind(step.as_uuid())
    .fetch_all(pool)
    .await
    .expect("read the follow-up rows")
    .into_iter()
    .map(
        |(id, status, text_is_null, resolution, issued_box)| FollowUpRow {
            id: RunCommandId::from_uuid(id),
            status: status.parse().expect("a known command status"),
            text_is_null,
            resolution,
            issued_box: BoxId::from_uuid(issued_box),
        },
    )
    .collect()
}

/// `(windows, open windows)` of `step`'s `follow_up_window` rows: no store method lists them.
async fn follow_up_windows(pool: &sqlx::PgPool, step: StepId) -> (i64, i64) {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COUNT(*) FILTER (WHERE closed_at IS NULL) FROM follow_up_window \
         WHERE run_step_id = $1",
    )
    .bind(step.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read follow_up_window")
}

/// The `follow_up` rows of `step`'s log: `(turn, payload)`, in `seq` order.
async fn follow_up_events(store: &PgStore, step: StepId) -> Vec<(i32, serde_json::Value)> {
    store
        .step_events(step)
        .await
        .expect("the log reads")
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row.kind == EventKind::FollowUp)
        .map(|row| (row.turn, row.payload))
        .collect()
}

/// How many `done` rows `step`'s log holds: one per turn.
async fn dones(store: &PgStore, step: StepId) -> usize {
    store
        .step_events(step)
        .await
        .expect("the log reads")
        .unwrap_or_default()
        .iter()
        .filter(|row| row.kind == EventKind::Done)
        .count()
}

/// MOD-70 PRD metrics "applied across executors" (other-box worker) and "no unscrubbed
/// persistence": the worker on box A walks a step whose session parks; box B queues a follow-up
/// while it is parked, then answers. The worker sends the follow-up when the parked turn ends: the
/// step's log holds the scrubbed `follow_up` at `turn = 1` and a second `done`, the step completes
/// `done`, and the row is `applied` with its text gone, issued from B.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_walked_step_takes_a_follow_up_from_another_box() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    stack.ungate_feat().await;
    stack.set_executor(Executor::Worker).await;
    let (run, rest) = stack.start(ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::Queued, "the TUI only queues");
    let worker = stack.spawn_worker_with(CONFIG, Box::new(ParksOnce::then(1)));
    let (client, _client_root) = another_box(&stack.db).await;

    let parked = pending_request(&client, ids::HTUI_FEAT_3).await;
    assert_eq!(parked.run_id, run);
    let id = queued(&client, parked.run_step_id, FOLLOW_UP).await;
    let view = client
        .relay_view(ids::HTUI_FEAT_3)
        .await
        .expect("the view reads");
    assert_eq!(
        view.follow_ups
            .iter()
            .map(|row| (row.id, row.status))
            .collect::<Vec<_>>(),
        [(id, RunCommandStatus::Pending)],
        "B's pane shows its follow-up queued"
    );
    allow(&client, &parked).await;
    step_reaches(&stack, run, parked.run_step_id, StepStatus::Done).await;

    assert_eq!(
        follow_up_events(&stack.db.store, parked.run_step_id).await,
        [(1, json!({ "text": FOLLOW_UP }))],
        "the executor recorded the scrubbed follow-up once, as turn 1"
    );
    assert_eq!(
        dones(&stack.db.store, parked.run_step_id).await,
        2,
        "two turns, two `done`s"
    );
    assert_eq!(
        follow_up_rows(&stack.db.pool, parked.run_step_id).await,
        [FollowUpRow {
            id,
            status: RunCommandStatus::Applied,
            text_is_null: true,
            resolution: None,
            issued_box: client.this_box(),
        }],
        "applied by the worker, issued from B, its text gone (PRD metric 3)"
    );
    assert_eq!(
        follow_up_windows(&stack.db.pool, parked.run_step_id).await,
        (1, 0),
        "one window, closed at the session's exit"
    );
    assert_eq!(stack.rested(run).await.status, RunStatus::Done);
    stack.released(run).await;
    worker.stop().await;
    stack.finish().await;
}

/// MOD-70 D5, B-14: a cancel box B requests while its follow-up is pending refuses that
/// follow-up in the cancel's own transaction (`FOLLOW_UP_RUN_CANCELLED`, text gone); the worker
/// ends the walk, the run is `cancelled`, and no `follow_up` was ever recorded.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_from_another_box_refuses_its_pending_follow_up() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    stack.ungate_feat().await;
    stack.set_executor(Executor::Worker).await;
    let (run, _) = stack.start(ids::HTUI_FEAT_3).await;
    let worker = stack.spawn_worker_with(CONFIG, Box::new(ParksOnce::then(1)));
    let (client, _client_root) = another_box(&stack.db).await;
    let parked = pending_request(&client, ids::HTUI_FEAT_3).await;
    let id = queued(&client, parked.run_step_id, FOLLOW_UP).await;

    let requested = client
        .request_cancel(run, client.this_user(), client.this_box())
        .await
        .expect("the cancel is written");
    assert!(
        matches!(requested, htui_core::model::CancelRequest::Inserted(_)),
        "{requested:?}"
    );
    let refused = FollowUpRow {
        id,
        status: RunCommandStatus::Refused,
        text_is_null: true,
        resolution: Some(FOLLOW_UP_RUN_CANCELLED.to_owned()),
        issued_box: client.this_box(),
    };
    assert_eq!(
        follow_up_rows(&stack.db.pool, parked.run_step_id).await,
        [refused.clone()],
        "the cancel's own transaction refused it (B-14)"
    );
    let deadline = Instant::now() + PATIENCE;
    while stack.run_row(run).await.status != RunStatus::Cancelled {
        assert!(Instant::now() < deadline, "the run was never cancelled");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    step_reaches(&stack, run, parked.run_step_id, StepStatus::Cancelled).await;
    stack.released(run).await;

    assert_eq!(
        follow_up_rows(&stack.db.pool, parked.run_step_id).await,
        [refused],
        "the walk's own close changed nothing"
    );
    assert!(
        follow_up_events(&stack.db.store, parked.run_step_id)
            .await
            .is_empty(),
        "no follow-up was sent"
    );
    assert_eq!(
        follow_up_windows(&stack.db.pool, parked.run_step_id)
            .await
            .1,
        0,
        "no window is left open"
    );
    let view = client
        .relay_view(ids::HTUI_FEAT_3)
        .await
        .expect("the view reads");
    assert!(
        view.permissions.is_empty() && view.cancels.is_empty() && view.follow_ups.is_empty(),
        "{view:?}"
    );
    worker.stop().await;
    stack.finish().await;
}

/// How many follow-ups [`no_follow_up_is_left_pending_after_the_step_ends`]' client queues at
/// most; the parking session has a turn for each.
const MAX_QUEUED: usize = 3;

/// MOD-70 B-15, PRD "none pending after its step ends": box B queues a follow-up while the step
/// is parked, answers, and then keeps queueing as fast as Postgres answers until the step's
/// session refuses it, racing the walk's last turn-end check and its close however they
/// interleave. Whatever the race decided, once the step is `done` no follow-up of it is pending,
/// every one is resolved with its text gone (each applied one recorded as its own turn, each
/// refused one `FOLLOW_UP_SESSION_ENDED`), its window is closed, and a further enqueue is refused
/// `NotRunning`.
#[tokio::test(flavor = "multi_thread")]
async fn no_follow_up_is_left_pending_after_the_step_ends() {
    let Some(mut stack) = Stack::new().await else {
        return;
    };
    stack.ungate_feat().await;
    stack.set_executor(Executor::Worker).await;
    let (run, _) = stack.start(ids::HTUI_FEAT_3).await;
    let worker = stack.spawn_worker_with(CONFIG, Box::new(ParksOnce::then(MAX_QUEUED)));
    let (client, _client_root) = another_box(&stack.db).await;
    let parked = pending_request(&client, ids::HTUI_FEAT_3).await;
    let step = parked.run_step_id;
    queued(&client, step, FOLLOW_UP).await;
    allow(&client, &parked).await;

    let mut queued_rows = 1;
    let deadline = Instant::now() + PATIENCE;
    let ended = loop {
        assert!(Instant::now() < deadline, "the session never ended");
        match request_follow_up(&client, step, FOLLOW_UP).await {
            FollowUpRequest::Queued(_) => {
                queued_rows += 1;
                if queued_rows == MAX_QUEUED {
                    break None;
                }
            }
            FollowUpRequest::Refused(FollowUpRefusal::AlreadyQueued) => {}
            FollowUpRequest::Refused(
                refusal @ (FollowUpRefusal::SessionEnded | FollowUpRefusal::NotRunning),
            ) => break Some(refusal),
            refused @ FollowUpRequest::Refused(_) => {
                panic!("a live session's enqueue was refused otherwise: {refused:?}")
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    step_reaches(&stack, run, step, StepStatus::Done).await;

    let rows = follow_up_rows(&stack.db.pool, step).await;
    assert_eq!(
        rows.len(),
        queued_rows,
        "one row per queued follow-up: {rows:?} ({ended:?})"
    );
    assert!(
        rows.iter()
            .all(|row| row.status != RunCommandStatus::Pending && row.text_is_null),
        "every follow-up resolved, its text gone: {rows:?}"
    );
    let applied = rows
        .iter()
        .filter(|row| row.status == RunCommandStatus::Applied)
        .count();
    assert!(
        applied >= 1,
        "the one queued while parked was sent: {rows:?}"
    );
    assert!(
        rows.iter()
            .filter(|row| row.status == RunCommandStatus::Refused)
            .all(|row| row.resolution.as_deref() == Some(FOLLOW_UP_SESSION_ENDED)),
        "a follow-up the session missed is refused `SessionEnded`: {rows:?}"
    );
    let sent = follow_up_events(&stack.db.store, step).await;
    assert_eq!(
        sent.iter().map(|(turn, _)| *turn).collect::<Vec<_>>(),
        (1..=i32::try_from(applied).expect("a few turns")).collect::<Vec<_>>(),
        "each applied follow-up opened its own turn: {rows:?}"
    );
    assert_eq!(
        follow_up_windows(&stack.db.pool, step).await,
        (1, 0),
        "the step's one window is closed"
    );
    assert_eq!(
        request_follow_up(&client, step, FOLLOW_UP).await,
        FollowUpRequest::Refused(FollowUpRefusal::NotRunning),
        "a `done` step takes no follow-up"
    );
    assert!(
        follow_up_rows(&stack.db.pool, step)
            .await
            .iter()
            .all(|row| row.status != RunCommandStatus::Pending),
        "the refusal wrote nothing"
    );
    assert_eq!(stack.rested(run).await.status, RunStatus::Done);
    stack.released(run).await;
    worker.stop().await;
    stack.finish().await;
}

// ---------------------------------------------------------------------------------------------
// Case 4: the headless connect
// ---------------------------------------------------------------------------------------------

/// MOD-40 plan D8, MOD-41 plan D14: a schema with every migration pending is refused (exit 2)
/// with the pending sentence, and nothing is written: not even the bookkeeping table.
#[tokio::test(flavor = "multi_thread")]
async fn the_worker_refuses_a_pending_schema_and_writes_nothing() {
    let Some(db) = testkit::bare_db().await else {
        return;
    };
    // `bare_db` connects through the TUI's `PgStore::connect`, which makes the bookkeeping table;
    // without it the database is as bare as a new one.
    sqlx::query("DROP TABLE _sqlx_migrations")
        .execute(&db.pool)
        .await
        .expect("drop the bookkeeping table");
    let root = tempfile::tempdir().expect("a throwaway config root");

    let refused = match worker_cmd::connect(&db.url, root.path(), PoolSize::WORKER_DEFAULT).await {
        Ok(_) => panic!("a pending schema must be refused"),
        Err(refused) => refused,
    };
    assert!(
        matches!(refused, WorkerExit::Refused(_)),
        "a startup refusal: {refused:?}"
    );
    assert_eq!(refused.code(), 2);
    assert!(refused.to_string().contains("pending"), "{refused}");
    let tables: Vec<Option<bool>> = sqlx::query_scalar(
        "SELECT to_regclass(name) IS NULL FROM unnest(ARRAY['_sqlx_migrations', 'box']) AS name",
    )
    .fetch_all(&db.pool)
    .await
    .expect("ask for the tables");
    assert_eq!(
        tables,
        [Some(true), Some(true)],
        "neither `_sqlx_migrations` nor `box` exists"
    );
    db.drop_db().await;
}

// ---------------------------------------------------------------------------------------------
// Cases 5 to 8: the binary
// ---------------------------------------------------------------------------------------------

/// `dsn` with a password to look for, and that password: `sentinel-<uuid>` when the DSN carries
/// none (the sandbox's trust auth accepts any, blueprint F-27), else the DSN's own.
#[cfg(target_os = "linux")]
fn with_sentinel(dsn: &str) -> (String, String) {
    let authority = dsn.find("://").map_or(0, |i| i + 3);
    let end = dsn[authority..]
        .find(['/', '?'])
        .map_or(dsn.len(), |i| authority + i);
    let Some(at) = dsn[authority..end].rfind('@').map(|i| authority + i) else {
        panic!("the test DSN names no user");
    };
    if let Some(colon) = dsn[authority..at].find(':').map(|i| authority + i) {
        return (dsn.to_owned(), dsn[colon + 1..at].to_owned());
    }
    let sentinel = format!("sentinel-{}", uuid::Uuid::now_v7().simple());
    (format!("{}:{sentinel}{}", &dsn[..at], &dsn[at..]), sentinel)
}

/// The `htui` binary as `htui worker --dsn-stdin --log <home>/w.log`, with a cleared environment:
/// only `HOME` and `XDG_CONFIG_HOME` (the throwaway `home`) and `USERNAME=htui-ci`. Its stdin
/// holds `dsn` and is closed.
#[cfg(target_os = "linux")]
fn spawn_binary(home: &std::path::Path, dsn: &str) -> Reaped {
    use std::io::Write as _;

    let mut child = spawn_binary_waiting(home);
    let mut stdin = child.stdin.take().expect("a piped stdin");
    writeln!(stdin, "{dsn}").expect("write the DSN");
    drop(stdin);
    child
}

/// As [`spawn_binary`], but nothing is written to the child's stdin, which stays open.
#[cfg(target_os = "linux")]
fn spawn_binary_waiting(home: &std::path::Path) -> Reaped {
    use std::process::{Command as Process, Stdio};

    Reaped(
        Process::new(env!("CARGO_BIN_EXE_htui"))
            .arg("worker")
            .arg("--dsn-stdin")
            .arg("--log")
            .arg(home.join("w.log"))
            .env_clear()
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("USERNAME", "htui-ci")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the htui binary"),
    )
}

/// A spawned worker that is killed and reaped when the case ends, even by a panic: an orphan
/// would outlive the suite and hold whatever it inherited (the gate's `flock` among them).
#[cfg(target_os = "linux")]
struct Reaped(std::process::Child);

#[cfg(target_os = "linux")]
impl std::ops::Deref for Reaped {
    type Target = std::process::Child;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(target_os = "linux")]
impl std::ops::DerefMut for Reaped {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(target_os = "linux")]
impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The child's exit status within `limit`, polled.
#[cfg(target_os = "linux")]
async fn exit_within(child: &mut std::process::Child, limit: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().expect("poll the child") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("the worker did not exit within {limit:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// What the child wrote to stderr, once it has exited.
#[cfg(target_os = "linux")]
fn stderr_of(child: &mut std::process::Child) -> String {
    use std::io::Read as _;

    let mut text = String::new();
    child
        .stderr
        .take()
        .expect("a piped stderr")
        .read_to_string(&mut text)
        .expect("read stderr");
    text
}

/// PRD D3, R-STO-1: the DSN reaches the worker on stdin and never shows in its `argv`, its
/// environment or its log; `ready` is logged after the connect and the start beat lands; SIGTERM
/// is a clean exit 0.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn the_worker_binary_never_exposes_its_dsn() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let home = tempfile::tempdir().expect("a throwaway home");
    let (dsn, sentinel) = with_sentinel(&db.url);
    let mut child = spawn_binary(home.path(), &dsn);
    let log = home.path().join("w.log");

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("htui worker ready") {
            break;
        }
        if let Some(status) = child.try_wait().expect("poll the child") {
            panic!(
                "the worker exited ({status}) before it was ready: {}\nlog: {text}",
                stderr_of(&mut child)
            );
        }
        assert!(
            Instant::now() < deadline,
            "no `htui worker ready` within 60 s; log: {text}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pid = child.id();
    for file in ["cmdline", "environ"] {
        let bytes = std::fs::read(format!("/proc/{pid}/{file}")).expect("read /proc");
        assert!(
            !String::from_utf8_lossy(&bytes).contains(&sentinel),
            "the DSN's password is in /proc/{pid}/{file}"
        );
    }
    let identity = htui_store::identity::load_or_mint(&home.path().join("htui"))
        .expect("the worker minted box.toml under the throwaway home");
    let deadline = Instant::now() + PATIENCE;
    loop {
        let beat: Option<bool> =
            sqlx::query_scalar("SELECT last_seen_at > registered_at FROM box WHERE id = $1")
                .bind(identity.box_id.as_uuid())
                .fetch_optional(&db.pool)
                .await
                .expect("read the worker's box row");
        if beat == Some(true) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the worker's start beat never stamped its box (blueprint B-11): {beat:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let killed = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("run kill");
    assert!(killed.success(), "SIGTERM was delivered");
    let status = exit_within(&mut child, Duration::from_secs(10)).await;
    let stderr = stderr_of(&mut child);
    assert_eq!(status.code(), Some(0), "a clean shutdown: {stderr}");
    let text = std::fs::read_to_string(&log).expect("read the log");
    assert!(
        !text.contains(&sentinel),
        "the DSN's password is in the log"
    );
    assert!(
        !stderr.contains(&sentinel),
        "the DSN's password is on stderr"
    );
    db.drop_db().await;
}

/// Blueprint F-27: a DSN naming a database that does not exist is a startup refusal, exit 2
/// under any auth mode, and neither stderr nor the log echoes its password.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn the_worker_binary_refuses_a_dsn_it_cannot_use_without_echoing_it() {
    let Ok(maint) = std::env::var(testkit::ENV_URL) else {
        assert!(
            std::env::var("CI").is_err(),
            "{} is not set but CI is running",
            testkit::ENV_URL
        );
        println!("{}", testkit::SKIP);
        return;
    };
    let home = tempfile::tempdir().expect("a throwaway home");
    let missing = format!("htui_no_such_{}", uuid::Uuid::now_v7().simple());
    let (dsn, sentinel) = with_sentinel(&testkit::with_database(&maint, &missing));
    let mut child = spawn_binary(home.path(), &dsn);

    let status = exit_within(&mut child, Duration::from_secs(60)).await;
    let stderr = stderr_of(&mut child);
    assert_eq!(status.code(), Some(2), "a startup refusal: {stderr}");
    assert!(
        stderr.starts_with("htui: "),
        "the refusal is printed: {stderr}"
    );
    assert!(
        !stderr.contains(&sentinel),
        "the DSN's password is on stderr"
    );
    let text = std::fs::read_to_string(home.path().join("w.log")).expect("the refusal is logged");
    let sentence = stderr
        .trim_end()
        .strip_prefix("htui: ")
        .expect("checked above");
    // MOD-41 E-1: stderr and the log, at `warn`: an `error` line would be a GlitchTip report.
    assert!(
        text.lines()
            .any(|line| line.contains(sentence) && line.contains("WARN")),
        "the refusal is a `warn` line in the log: {text:?}"
    );
    assert!(
        !text.contains(&sentinel),
        "the DSN's password is in the log"
    );
}

/// Whether `pid` has installed its own SIGTERM handler (`SigCgt` in `/proc/<pid>/status`): from
/// then on a SIGTERM is the worker's to act on, not the default termination.
#[cfg(target_os = "linux")]
fn catches_sigterm(pid: u32) -> bool {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status
        .lines()
        .find_map(|line| line.strip_prefix("SigCgt:"))
        .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
        .is_some_and(|mask| mask & (1 << (15 - 1)) != 0)
}

/// Waits until the child's SIGTERM handler is in, then sends it SIGTERM.
#[cfg(target_os = "linux")]
async fn terminate_once_handled(child: &mut std::process::Child) {
    let pid = child.id();
    let deadline = Instant::now() + PATIENCE;
    while !catches_sigterm(pid) {
        if let Some(status) = child.try_wait().expect("poll the child") {
            panic!(
                "the worker exited ({status}) before its handlers were in: {}",
                stderr_of(child)
            );
        }
        assert!(
            Instant::now() < deadline,
            "the worker installed no SIGTERM handler"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let killed = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("run kill");
    assert!(killed.success(), "SIGTERM was delivered");
}

/// Plan D14, docs/htui-worker.md: SIGTERM stops the worker while it still waits for its DSN on
/// stdin, exit 0, and nothing is minted. Needs no server: the worker never gets that far.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn the_worker_binary_stops_on_a_signal_while_it_waits_for_its_dsn() {
    let home = tempfile::tempdir().expect("a throwaway home");
    let mut child = spawn_binary_waiting(home.path());
    let stdin = child.stdin.take().expect("a piped stdin");

    terminate_once_handled(&mut child).await;
    let status = exit_within(&mut child, Duration::from_secs(10)).await;
    drop(stdin);
    let stderr = stderr_of(&mut child);
    assert_eq!(status.code(), Some(0), "a clean shutdown: {stderr}");
    assert!(
        !home.path().join("htui").join("box.toml").exists(),
        "nothing was minted"
    );
    let text = std::fs::read_to_string(home.path().join("w.log")).unwrap_or_default();
    assert!(!text.contains("htui worker ready"), "never ready: {text}");
}

/// Plan D14: SIGTERM stops the worker in the middle of its connect, exit 0, without waiting for
/// `CONNECT_TIMEOUT`. The "server" accepts the connection and never answers. Needs no server.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn the_worker_binary_stops_on_a_signal_while_it_connects() {
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a silent listener");
    let port = silent.local_addr().expect("its address").port();
    let home = tempfile::tempdir().expect("a throwaway home");
    let mut child = spawn_binary(
        home.path(),
        &format!("postgres://htui:sentinel@127.0.0.1:{port}/htui"),
    );

    let (_held, _) = tokio::time::timeout(PATIENCE, silent.accept())
        .await
        .expect("the worker connects within the patience window")
        .expect("accept the worker");
    terminate_once_handled(&mut child).await;
    let status = exit_within(&mut child, htui_store::pg::CONNECT_TIMEOUT / 2).await;
    let stderr = stderr_of(&mut child);
    assert_eq!(status.code(), Some(0), "a clean shutdown: {stderr}");
    let text = std::fs::read_to_string(home.path().join("w.log")).unwrap_or_default();
    assert!(!text.contains("htui worker ready"), "never ready: {text}");
}
