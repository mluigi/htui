//! MOD-12 milestone 1 (blueprint §C.4): the queue runner, end to end through the run runtime.
//!
//! A queued item is admitted by the sweep of its box's executing process (plan D6), in queue
//! order (D4) and up to the box's free slots, as an `auto` run under the open batch (D7); a
//! paused box admits nothing (D2); finished entries are pruned and a batch left empty with no
//! live run closes `drained` (D3); and a walk that rests wakes the sweep, so the next entry is
//! admitted without waiting for the ticker (D8, the walk-end half).
//!
//! The harness is this file's own, trimmed from `htui`'s `run_worker` fixture: the demo with its
//! agents disabled, one scripted `acp` row ready on the demo box, and `RUN_2` cancelled. Only the
//! overlap cases (criterion 26) add the project's primary repo: with it, every run of the project
//! scopes that repo, and a live run (`running` or parked, invariant 6) holds off any other, so
//! the cases that need two runs at once leave the scope empty. Sessions play one `done` turn, each after taking one permit of the harness's
//! semaphore, so a case can hold walks `running` and release them. The items are ANA: an auto
//! run walks `research` (its soft gate downgraded to `never`, D10) and parks at the hard
//! `verdict` gate, which holds no slot (`claim_run` counts `running` only).
//!
//! The ticker is one hour, so every sweep a case sees is an explicit `sweep_with` or the D8
//! wake. The `_pg` cases return early without `HTUI_TEST_DATABASE_URL`.
//!
//! MOD-12 milestone 2 (blueprint §C.3) adds the runner's spend gate (plan D4): a costing harness
//! whose every session reports a fixed USD cost, and cases over the batch's spend against each
//! entry's project cap. The spend is always made by a run that is already parked when the cap is
//! planted, so its snapshot froze no batch cap and nothing cuts its sessions (blueprint H-6); a
//! run admitted under a cap is asserted admitted, never walked. The one exception is
//! (pg-overshoot), which plants the cap first on purpose: it walks the cut and the overshoot.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::Utc;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
    PermissionRequestId, SessionSpec,
};
use htui_agent::error::DriverError;
use htui_agent::event::{DoneEvent, DriverEnvelope, DriverEvent, StopReason, UsageEvent};
use htui_agent::fake::FakeDriver;
use htui_agent::registry::{DriverFactory, TransportBuilder};
use htui_core::fixtures::{demo_at, edit_agent, ids};
use htui_core::model::QueueSetting;
use htui_core::model::{
    Agent, AgentBox, AgentId, AgentSummary, BatchClose, BatchId, Billing, BoxEdit, DocumentId,
    Executor, ItemId, ItemKindId, MIN_BUDGET_FOR_NEW_ATTEMPT, NewDocument, NewItem, NewRepo,
    NewStepGraph, PER_TOKEN_CAP_BATCH, PhaseId, ProjectId, Resolution, RunId, RunMode, RunStatus,
    RunStep, RunSummary, SnapshotPhase, Status, StepGraphId, StepGraphPhase, StepStatus, Transport,
};
use htui_core::store::{CasOutcome, MemStore, QueueTarget, ReadStore, WriteStore};
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use htui_orch::{Command, Isolator};
use htui_store::{Backend, PgStore, testkit};
use htui_worker::{
    LiveChats, OrchReply, OrchRequest, ReplySink, Role, RunReply, RunRequest, RunRuntime,
    StepAuthor,
};
use serde_json::json;
use tokio::sync::Semaphore;

/// How long a case waits for anything before it calls the runtime stuck.
const PATIENCE: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------------------------
// The seed
// ---------------------------------------------------------------------------------------------

/// The scripted registry row: `acp`, so the factory reaches the fake by row data alone.
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

/// The scripted agent's `agent_box` on the demo box, probed ready (rung 3).
fn ready_on_box(agent_id: AgentId) -> AgentBox {
    let at = demo_at(0, 0);
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

/// `htui`'s `run_worker::seeded`, over any store: every fixture agent disabled, one scripted
/// row ready on the demo box, with `repo` the htui primary repo (the scope a default run resolves
/// to, and what makes two runs of the project overlap), and the seeded `queued` `RUN_2`
/// cancelled.
async fn seed<S: WriteStore>(store: &S, agents: Vec<AgentSummary>, repo: Repo) {
    for summary in agents {
        let mut row = summary.agent;
        row.enabled = false;
        edit_agent(store, &row).await.expect("the row is disabled");
    }
    let agent = AgentId::new();
    store
        .upsert_agent(&scripted_row(agent), None)
        .await
        .expect("the scripted row lands");
    store
        .upsert_agent_box(&ready_on_box(agent))
        .await
        .expect("the agent_box row lands");
    if repo == Repo::Primary {
        store
            .create_repo(NewRepo {
                id: htui_core::model::RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo project has no repo yet");
    }
    store
        .finish_run(ids::RUN_2, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("the seeded run is queued and cancellable");
}

/// Whether [`seed`] adds the project's primary repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repo {
    /// No repo: a default run's scope is empty, so no two runs overlap.
    None,
    /// The primary repo: every default run of the project overlaps every other live one.
    Primary,
}

/// A fresh `ANA` item of the htui project: `research` then `verdict` (hard).
async fn mint_ana<S: WriteStore>(store: &S, title: &str, priority: i16) -> ItemId {
    mint(store, title, priority, Vec::new()).await
}

async fn mint<S: WriteStore>(
    store: &S,
    title: &str,
    priority: i16,
    required_tags: Vec<String>,
) -> ItemId {
    mint_in(
        store,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA),
        title,
        priority,
        required_tags,
    )
    .await
}

/// A fresh `ANA` item of the agy project (MOD-12 M2's second project, its own caps).
async fn mint_agy<S: WriteStore>(store: &S, title: &str, priority: i16) -> ItemId {
    mint_in(
        store,
        (ids::PROJECT_AGY, ids::KIND_AGY_ANA),
        title,
        priority,
        Vec::new(),
    )
    .await
}

/// A fresh item of `kind` in `project`, on the demo box.
async fn mint_in<S: WriteStore>(
    store: &S,
    (project_id, kind_id): (ProjectId, ItemKindId),
    title: &str,
    priority: i16,
    required_tags: Vec<String>,
) -> ItemId {
    store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id,
            kind_id,
            title: title.to_owned(),
            body: String::new(),
            required_tags,
            touched_paths: Vec::new(),
            priority,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("the item mints")
        .id
}

// ---------------------------------------------------------------------------------------------
// The sessions
// ---------------------------------------------------------------------------------------------

/// The transport the scripted row reaches: every session plays one `done` turn after taking one
/// permit of the shared semaphore. With a `cost`, the turn first reports that many USD micros
/// (`htui-orch`'s `ScriptedStep::done_costing` shape), so a batch has a spend (MOD-12 M2).
#[derive(Debug)]
struct Hold {
    gate: Arc<Semaphore>,
    cost: Option<i64>,
}

impl TransportBuilder for Hold {
    fn build(
        &self,
        agent: &Agent,
        _on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, DriverError> {
        let mut events = Vec::new();
        if let Some(cost) = self.cost {
            events.push(ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
                cost_micros: Some(cost),
                ..UsageEvent::default()
            })));
        }
        events.push(ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })));
        Ok(Box::new(HeldDriver {
            inner: FakeDriver::new(agent.name.clone(), caps, Script::one_turn(events)),
            gate: Arc::clone(&self.gate),
        }))
    }
}

#[derive(Debug)]
struct HeldDriver {
    inner: FakeDriver,
    gate: Arc<Semaphore>,
}

impl AgentDriver for HeldDriver {
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
        let gate = Arc::clone(&self.gate);
        Box::pin(async move {
            let session = inner.await?;
            Ok(Box::new(Held {
                inner: session,
                gate: Some(gate),
            }) as Box<dyn AgentSession>)
        })
    }
}

/// A session that takes one permit before its first event.
#[derive(Debug)]
struct Held {
    inner: Box<dyn AgentSession>,
    gate: Option<Arc<Semaphore>>,
}

impl AgentSession for Held {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        self.inner.session_ref()
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            if let Some(gate) = self.gate.take() {
                gate.acquire()
                    .await
                    .expect("the gate is never closed")
                    .forget();
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
        self.gate = None;
        self.inner.cancel(grace)
    }
}

/// D203's author for tests: one document of the phase's `output_kind` per step.
#[derive(Debug)]
struct OutputAuthor;

impl StepAuthor for OutputAuthor {
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument> {
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

// ---------------------------------------------------------------------------------------------
// The sink
// ---------------------------------------------------------------------------------------------

/// Every answer the runtime gives, by address. `Unaddressed` cannot address a manual `StartRun`.
#[derive(Debug, Clone, Default)]
struct TestSink(Arc<StdMutex<Vec<(u64, RunReply)>>>);

impl ReplySink for TestSink {
    type Addr = u64;
    type Subscriber = u64;

    fn subscriber(addr: &u64) -> u64 {
        *addr
    }

    fn send(&self, to: &u64, reply: RunReply) {
        self.0.lock().expect("the sink").push((*to, reply));
    }
}

impl TestSink {
    /// The first answer at `addr`, waiting for it.
    async fn reply(&self, addr: u64) -> RunReply {
        let mut found = None;
        eventually("the answer", || {
            found = self
                .0
                .lock()
                .expect("the sink")
                .iter()
                .find(|(to, _)| *to == addr)
                .map(|(_, reply)| reply.clone());
            let ready = found.is_some();
            async move { ready }
        })
        .await;
        found.expect("found")
    }
}

// ---------------------------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------------------------

/// The seeded store and the gate its sessions take permits from.
struct Harness {
    store: MemStore,
    parts: Parts,
}

/// The fakes and the gate every runtime of a case shares.
#[derive(Clone)]
struct Parts {
    gate: Arc<Semaphore>,
    opened: Arc<AtomicBool>,
    isolator: Arc<FakeIsolator>,
    /// What every session reports spending, USD micros; `None` reports no usage.
    cost: Option<i64>,
}

impl Parts {
    fn new() -> Self {
        Self {
            gate: Arc::new(Semaphore::new(0)),
            opened: Arc::default(),
            isolator: Arc::new(FakeIsolator::new()),
            cost: None,
        }
    }

    /// [`Parts::new`] whose every session costs `micros` (MOD-12 M2).
    fn costing(micros: i64) -> Self {
        Self {
            cost: Some(micros),
            ..Self::new()
        }
    }

    /// Every session plays at once, from now on.
    fn open(&self) {
        if !self.opened.swap(true, Ordering::SeqCst) {
            self.gate.add_permits(Semaphore::MAX_PERMITS / 2);
        }
    }

    /// A runtime over the fakes: the claim scan on, the ticker an hour, the output author.
    fn runtime<H: htui_core::store::WorkerHost>(&self) -> RunRuntime<H, TestSink> {
        let mut factory = DriverFactory::new();
        factory.register(
            "acp",
            Box::new(Hold {
                gate: Arc::clone(&self.gate),
                cost: self.cost,
            }),
        );
        RunRuntime::with_parts(
            Arc::clone(&self.isolator) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()),
            factory,
        )
        .with_author(Arc::new(OutputAuthor))
        .with_sweep_every(Duration::from_secs(3600))
    }
}

impl Harness {
    /// The seeded demo without a repo; sessions hold until released.
    async fn new() -> Self {
        Self::over(MemStore::demo(), Repo::None).await
    }

    /// The seeded demo with the primary repo, so any two runs overlap; sessions hold.
    async fn overlapping() -> Self {
        Self::over(MemStore::demo(), Repo::Primary).await
    }

    /// The seeded demo whose box settings are `settings`.
    async fn with_box_settings(settings: serde_json::Value) -> Self {
        let mut data = htui_core::fixtures::demo_data();
        for row in &mut data.boxes {
            row.settings = settings.clone();
        }
        Self::over(MemStore::from_demo(data), Repo::None).await
    }

    async fn over(store: MemStore, repo: Repo) -> Self {
        let agents = store.agents().await.expect("the fixture's agents");
        seed(&store, agents, repo).await;
        Self {
            store,
            parts: Parts::new(),
        }
    }

    /// The seeded demo; every session plays at once.
    async fn open() -> Self {
        let harness = Self::new().await;
        harness.parts.open();
        harness
    }

    /// [`Harness::open`] whose every session costs `micros` (MOD-12 M2).
    async fn costing(micros: i64) -> Self {
        let mut harness = Self::new().await;
        harness.parts = Parts::costing(micros);
        harness.parts.open();
        harness
    }

    /// Plants `project`'s `per_token_cap_batch` (JSON, so a malformed one can be planted), merged
    /// into its current `settings`: `set_project_settings` replaces the blob (blueprint H-9).
    async fn cap_batch(&self, project: ProjectId, cap: serde_json::Value) {
        let mut settings = self
            .store
            .project_settings(project)
            .await
            .expect("the read answers")
            .expect("the project exists");
        if !settings.is_object() {
            settings = json!({});
        }
        settings
            .as_object_mut()
            .expect("an object")
            .insert(PER_TOKEN_CAP_BATCH.to_owned(), cap);
        self.store.set_project_settings(project, settings);
    }

    async fn batch_spend(&self, batch: BatchId) -> Option<i64> {
        self.store
            .batch_spend(batch)
            .await
            .expect("the read answers")
    }

    /// Whether `item` has a run admitted under `batch`.
    async fn admitted_under(&self, item: ItemId, batch: BatchId) -> bool {
        let members = self
            .store
            .batch_runs(batch)
            .await
            .expect("the read answers");
        self.runs(item)
            .await
            .iter()
            .any(|run| members.iter().any(|(member, _)| *member == run.id))
    }

    /// One explicit sweep, settled.
    async fn sweep(&self, runtime: &mut RunRuntime<Backend, TestSink>) {
        runtime.sweep_with(&self.backend(), &TestSink::default());
        settle(runtime).await;
    }

    fn backend(&self) -> Backend {
        Backend::memory(self.store.clone())
    }

    fn runtime(&self) -> RunRuntime<Backend, TestSink> {
        self.parts.runtime()
    }

    async fn queue(&self, item: ItemId) {
        self.store
            .queue_item(item, ids::BOX, ids::USER, Utc::now())
            .await
            .expect("the item queues");
    }

    async fn resume(&self) -> BatchId {
        self.store
            .open_batch(ids::BOX, ids::USER, Utc::now())
            .await
            .expect("the batch opens")
            .id
    }

    async fn runs(&self, item: ItemId) -> Vec<RunSummary> {
        self.store.runs(item).await.expect("the read answers")
    }

    async fn only_run(&self, item: ItemId) -> RunSummary {
        let runs = self.runs(item).await;
        assert_eq!(runs.len(), 1, "exactly one run of {item}: {runs:?}");
        runs.into_iter().next().expect("one run")
    }

    async fn status(&self, item: ItemId) -> Status {
        self.store
            .item(item)
            .await
            .expect("the read answers")
            .expect("the item exists")
            .status
    }
}

/// Polls `check` every 10 ms until it holds, or panics naming `what` after [`PATIENCE`].
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        if check().await {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} did not happen within {PATIENCE:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A manual `StartRun` over the item's default scope.
fn start_run(item: ItemId) -> RunRequest {
    RunRequest::Orch(OrchRequest::Command(Command::StartRun {
        item,
        mode: RunMode::Manual,
        repo_scope: None,
    }))
}

/// Settles `runtime`, asserting no task was stuck.
async fn settle<H: htui_core::store::WorkerHost>(runtime: &mut RunRuntime<H, TestSink>) {
    assert!(
        runtime.settle(PATIENCE).await.is_empty(),
        "no task is stuck"
    );
}

/// Whether `store`'s item has a run at `status`.
async fn has_run_at<S: ReadStore>(store: &S, item: ItemId, status: RunStatus) -> bool {
    store
        .runs(item)
        .await
        .expect("the read answers")
        .iter()
        .any(|run| run.status == status)
}

// ---------------------------------------------------------------------------------------------
// The cases over `Backend::Memory`
// ---------------------------------------------------------------------------------------------

/// (a) D2: no open batch, no admission.
#[tokio::test]
async fn a_paused_box_admits_nothing() {
    let h = Harness::open().await;
    let (x, y) = (
        mint_ana(&h.store, "x", 0).await,
        mint_ana(&h.store, "y", 0).await,
    );
    h.queue(x).await;
    h.queue(y).await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert!(h.runs(x).await.is_empty());
    assert!(h.runs(y).await.is_empty());
}

/// (b) D4, D6, D7, D8: queue order up to the cap, every run `auto` under the batch; the third
/// entry is admitted by the wake of a rested walk.
#[tokio::test]
async fn a_resumed_box_admits_in_queue_order_up_to_the_cap() {
    let h = Harness::new().await;
    let p0 = mint_ana(&h.store, "p0", 0).await;
    let p2 = mint_ana(&h.store, "p2", 2).await;
    let p1 = mint_ana(&h.store, "p1", 1).await;
    for item in [p0, p2, p1] {
        h.queue(item).await;
    }
    let batch = h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());

    eventually("p2 and p1 run", || async {
        has_run_at(&h.store, p2, RunStatus::Running).await
            && has_run_at(&h.store, p1, RunStatus::Running).await
    })
    .await;
    let (r2, r1) = (h.only_run(p2).await, h.only_run(p1).await);
    assert_eq!((r2.mode, r1.mode), (RunMode::Auto, RunMode::Auto));
    let mut members: Vec<RunId> = h
        .store
        .batch_runs(batch)
        .await
        .expect("the read answers")
        .into_iter()
        .map(|(run, _)| run)
        .collect();
    members.sort();
    let mut expected = vec![r2.id, r1.id];
    expected.sort();
    assert_eq!(
        members, expected,
        "the batch holds exactly the admitted runs"
    );
    assert!(
        h.runs(p0).await.is_empty(),
        "the cap is 2 and both slots are running"
    );

    h.parts.open();
    settle(&mut runtime).await;
    let r0 = h.only_run(p0).await;
    assert_eq!(r0.mode, RunMode::Auto);
    assert_eq!(r0.status, RunStatus::AwaitingApproval, "parked at verdict");
    assert!(
        h.store
            .batch_runs(batch)
            .await
            .expect("the read answers")
            .iter()
            .any(|(run, _)| *run == r0.id),
        "the woken admission is in the batch too"
    );
}

/// (c) D5: an entry `ready_items` does not hold waits, and is admitted once it is ready.
#[tokio::test]
async fn a_queued_item_that_is_not_ready_waits_then_runs() {
    let h = Harness::open().await;
    let item = mint_ana(&h.store, "blocked", 0).await;
    assert!(
        h.store
            .transition(item, Status::Open, Status::Blocked)
            .await
            .expect("open -> blocked is legal")
    );
    h.queue(item).await;
    h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert!(h.runs(item).await.is_empty(), "a blocked item is not ready");

    assert!(
        h.store
            .transition(item, Status::Blocked, Status::Open)
            .await
            .expect("blocked -> open is legal")
    );
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert_eq!(h.only_run(item).await.mode, RunMode::Auto);
}

/// (d) PRD D1, blueprint deviation 7: an item whose tags the box lacks is not ready, so it waits
/// `open` and unnoted, and the next entry is admitted in the same sweep.
#[tokio::test]
async fn a_queued_item_missing_a_tag_waits_unblocked() {
    let h = Harness::open().await;
    let cuda = mint(&h.store, "cuda", 5, vec!["cuda".to_owned()]).await;
    let plain = mint_ana(&h.store, "plain", 0).await;
    h.queue(cuda).await;
    h.queue(plain).await;
    let notes = h.store.notes(cuda).await.expect("the read answers").len();
    h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert!(h.runs(cuda).await.is_empty());
    assert_eq!(h.status(cuda).await, Status::Open);
    assert_eq!(
        h.store.notes(cuda).await.expect("the read answers").len(),
        notes,
        "no note"
    );
    assert_eq!(h.only_run(plain).await.mode, RunMode::Auto);
}

/// Repoints `item` at a clone of its graph whose first phase names a template no row has: the
/// conformance `repoint` recipe (`htui-orch`'s `conformance.rs`), over the store.
async fn repoint_at_no_template(store: &MemStore, item: ItemId) {
    let row = store
        .item(item)
        .await
        .expect("the read answers")
        .expect("the item exists");
    let graph = store
        .resolve_graph(item)
        .await
        .expect("the read answers")
        .expect("the item resolves to a graph");
    let clone = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: row.project_id,
            name: format!("{}-edited", row.key),
            description: "a queue case's edit of the live graph".to_owned(),
            is_override: false,
        })
        .await
        .expect("the name is fresh");
    let first = graph
        .phases
        .iter()
        .map(|phase| phase.phase.position)
        .min()
        .expect("the graph has phases");
    for phase in &graph.phases {
        let mut edited = StepGraphPhase {
            id: PhaseId::new(),
            graph_id: clone.id,
            ..phase.phase.clone()
        };
        if edited.position == first {
            "no-such-template".clone_into(&mut edited.template_name);
        }
        store
            .create_phase(&edited)
            .await
            .expect("the clone accepts its phases");
    }
    store
        .update_item(
            item,
            row.version,
            htui_core::model::ItemPatch {
                step_graph_id: Some(Some(clone.id)),
                author_id: row.created_by,
                reason: "a queue case's edit of the live graph".to_owned(),
                ..htui_core::model::ItemPatch::default()
            },
        )
        .await
        .expect("the item's version is current");
}

/// (d2) D6: an enqueue refusal (`NoTemplate` at resolution) is skipped and the next entry is
/// admitted in the same sweep.
#[tokio::test]
async fn an_enqueue_refusal_is_skipped_and_the_next_entry_admitted() {
    let h = Harness::open().await;
    let broken = mint_ana(&h.store, "broken", 5).await;
    repoint_at_no_template(&h.store, broken).await;
    let fine = mint_ana(&h.store, "fine", 0).await;
    h.queue(broken).await;
    h.queue(fine).await;
    h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert!(h.runs(broken).await.is_empty());
    assert_eq!(h.status(broken).await, Status::Open);
    assert_eq!(h.only_run(fine).await.mode, RunMode::Auto);
}

/// (e) Criterion 27, the pause half: a pause stops admission and cancels nothing.
#[tokio::test]
async fn pausing_stops_admission_and_leaves_running_runs_running() {
    let h = Harness::new().await;
    let items = [
        mint_ana(&h.store, "one", 2).await,
        mint_ana(&h.store, "two", 1).await,
        mint_ana(&h.store, "three", 0).await,
    ];
    for item in items {
        h.queue(item).await;
    }
    h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    eventually("two runs run", || async {
        has_run_at(&h.store, items[0], RunStatus::Running).await
            && has_run_at(&h.store, items[1], RunStatus::Running).await
    })
    .await;

    let closed = h
        .store
        .close_batch(ids::BOX, BatchClose::Paused, Utc::now())
        .await
        .expect("the write answers")
        .expect("a batch was open");
    assert_eq!(closed.closed_reason, Some(BatchClose::Paused));

    h.parts.open();
    settle(&mut runtime).await;
    for item in &items[..2] {
        let run = h.only_run(*item).await;
        assert_eq!(
            run.status,
            RunStatus::AwaitingApproval,
            "the pause cancelled nothing: the walk rested at verdict"
        );
    }
    assert!(
        h.runs(items[2]).await.is_empty(),
        "nothing admitted after the pause"
    );
    assert_eq!(
        h.store
            .open_batch_of(ids::BOX)
            .await
            .expect("the read answers"),
        None
    );
}

/// (f) D3: an entry whose item is closed is pruned, and a batch with no entry and no live run
/// closes `drained`.
#[tokio::test]
async fn a_drained_batch_closes_drained() {
    let h = Harness::open().await;
    let item = mint_ana(&h.store, "drains", 0).await;
    h.queue(item).await;
    let batch = h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    let run = h.only_run(item).await;
    assert_eq!(run.status, RunStatus::AwaitingApproval, "parked at verdict");

    h.store
        .finish_run(run.id, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("a parked run cancels");
    assert_eq!(h.status(item).await, Status::Open, "cancel moves it back");
    // VERIFY (blueprint §C.4 f): `transition(.., Closed)` is refused from every status (MOD-38
    // PRD D1); close-out is the only way in, and an `open` item closes `withdrawn`.
    h.store
        .close_out(
            item,
            Resolution::Withdrawn,
            NewDocument {
                id: DocumentId::new(),
                item_id: item,
                kind: "summary".to_owned(),
                title: "withdrawn".to_owned(),
                body: "withdrawn".to_owned(),
                produced_by_step_id: None,
                created_by: ids::USER,
                created_at: Utc::now(),
            },
            &[],
        )
        .await
        .expect("an open item closes withdrawn");

    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert!(
        h.store
            .queue_entries(ids::BOX)
            .await
            .expect("the read answers")
            .is_empty(),
        "the closed item's entry is pruned"
    );
    assert_eq!(
        h.store
            .open_batch_of(ids::BOX)
            .await
            .expect("the read answers"),
        None,
        "the batch closed"
    );
    // Nothing else closes a batch here: a pause now finds none open, and the batch kept its run.
    // No `MemStore` read returns a closed batch; (pg-f) reads its `drained` reason.
    assert_eq!(
        h.store
            .close_batch(ids::BOX, BatchClose::Paused, Utc::now())
            .await
            .expect("the write answers"),
        None
    );
    assert_eq!(
        h.store.batch_runs(batch).await.expect("the read answers"),
        vec![(run.id, RunStatus::Cancelled)]
    );
}

/// (f3) D3: a batch with no entry left stays open while a run of its own is live (parked at
/// `verdict`), and closes once that run is finished.
#[tokio::test]
async fn a_live_batch_run_keeps_the_drained_batch_open() {
    let h = Harness::open().await;
    let item = mint_ana(&h.store, "parks", 0).await;
    h.queue(item).await;
    let batch = h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    let run = h.only_run(item).await;
    assert_eq!(run.status, RunStatus::AwaitingApproval, "parked at verdict");

    assert!(
        h.store.dequeue_item(item).await.expect("the write answers"),
        "the entry leaves the queue; its run is untouched (D9)"
    );
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert_eq!(
        h.store
            .open_batch_of(ids::BOX)
            .await
            .expect("the read answers")
            .map(|open| open.id),
        Some(batch),
        "no entry is left, but the parked run is the batch's and live"
    );

    h.store
        .finish_run(run.id, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("a parked run cancels");
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert_eq!(
        h.store
            .open_batch_of(ids::BOX)
            .await
            .expect("the read answers"),
        None,
        "the batch's last run finished, so it drained"
    );
}

/// (f2) D3: a `blocked` entry keeps its place and the batch open.
#[tokio::test]
async fn a_blocked_entry_keeps_the_batch_open() {
    let h = Harness::open().await;
    let item = mint_ana(&h.store, "blocked", 0).await;
    assert!(
        h.store
            .transition(item, Status::Open, Status::Blocked)
            .await
            .expect("open -> blocked is legal")
    );
    h.queue(item).await;
    let batch = h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    assert_eq!(
        h.store
            .open_batch_of(ids::BOX)
            .await
            .expect("the read answers")
            .map(|open| open.id),
        Some(batch)
    );
    let entries = h
        .store
        .queue_entries(ids::BOX)
        .await
        .expect("the read answers");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.item_id)
            .collect::<Vec<_>>(),
        [item]
    );
}

/// (g1) Criterion 26, manual first: an auto run that overlaps a live manual one waits `queued`
/// (a parked run still holds its scope, invariant 6), and is claimed once the manual run ends.
#[tokio::test]
async fn a_manual_run_holds_off_an_overlapping_auto_run() {
    let h = Harness::overlapping().await;
    let backend = h.backend();
    let sink = TestSink::default();
    let mut runtime = h.runtime();
    let manual = mint_ana(&h.store, "manual", 0).await;
    let _ = runtime
        .serve_request(&backend, &sink, 1, start_run(manual), &LiveChats::default())
        .await;
    eventually("the manual run runs", || {
        has_run_at(&h.store, manual, RunStatus::Running)
    })
    .await;

    let auto = mint_ana(&h.store, "auto", 0).await;
    h.queue(auto).await;
    h.resume().await;
    runtime.sweep_with(&backend, &sink);
    eventually("the auto run exists", || async {
        !h.runs(auto).await.is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let waiting = h.only_run(auto).await;
    assert_eq!(
        waiting.status,
        RunStatus::Queued,
        "the claim refused it (Overlaps)"
    );
    assert_eq!(waiting.mode, RunMode::Auto);

    h.parts.open();
    settle(&mut runtime).await;
    let manual_run = h.only_run(manual).await;
    assert_eq!(manual_run.status, RunStatus::AwaitingApproval);
    assert_eq!(
        h.only_run(auto).await.status,
        RunStatus::Queued,
        "the parked manual run still holds the scope"
    );

    h.store
        .finish_run(manual_run.id, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("a parked run cancels");
    runtime.sweep_with(&backend, &sink);
    settle(&mut runtime).await;
    let auto_run = h.only_run(auto).await;
    assert_eq!(auto_run.id, waiting.id);
    assert_eq!(
        auto_run.status,
        RunStatus::AwaitingApproval,
        "claimed once the manual run ended"
    );
}

/// (g2) Criterion 26, auto first: a manual `StartRun` that overlaps a running auto run is
/// refused `Overlaps`, and its run is claimed once the auto run ends.
#[tokio::test]
async fn an_auto_run_holds_off_an_overlapping_manual_run() {
    let h = Harness::overlapping().await;
    let backend = h.backend();
    let sink = TestSink::default();
    let mut runtime = h.runtime();
    let auto = mint_ana(&h.store, "auto", 0).await;
    h.queue(auto).await;
    h.resume().await;
    runtime.sweep_with(&backend, &sink);
    eventually("the auto run runs", || {
        has_run_at(&h.store, auto, RunStatus::Running)
    })
    .await;

    let manual = mint_ana(&h.store, "manual", 0).await;
    let _ = runtime
        .serve_request(&backend, &sink, 7, start_run(manual), &LiveChats::default())
        .await;
    match sink.reply(7).await {
        RunReply::Failed { message, .. } => {
            assert!(message.contains("overlaps run"), "{message}");
        }
        other => panic!("the manual StartRun is refused, not {other:?}"),
    }
    assert_eq!(h.only_run(manual).await.status, RunStatus::Queued);

    h.parts.open();
    settle(&mut runtime).await;
    let auto_run = h.only_run(auto).await;
    assert_eq!(auto_run.status, RunStatus::AwaitingApproval);
    assert_eq!(
        h.only_run(manual).await.status,
        RunStatus::Queued,
        "the parked auto run still holds the scope"
    );

    h.store
        .finish_run(auto_run.id, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("a parked run cancels");
    runtime.sweep_with(&backend, &sink);
    settle(&mut runtime).await;
    assert_eq!(
        h.only_run(manual).await.status,
        RunStatus::AwaitingApproval,
        "claimed once the auto run ended"
    );
}

/// (h) D6: two runtimes over one store admit each item once (`create_run`'s status CAS).
#[tokio::test]
async fn two_runtimes_on_one_store_admit_each_item_once() {
    let h = Harness::open().await;
    let (x, y) = (
        mint_ana(&h.store, "x", 1).await,
        mint_ana(&h.store, "y", 0).await,
    );
    h.queue(x).await;
    h.queue(y).await;
    h.resume().await;
    let backend = h.backend();
    let mut first = h.runtime();
    let mut second = h.runtime();
    first.sweep_with(&backend, &TestSink::default());
    second.sweep_with(&backend, &TestSink::default());
    settle(&mut first).await;
    settle(&mut second).await;
    for item in [x, y] {
        let run = h.only_run(item).await;
        assert_eq!(run.mode, RunMode::Auto);
    }
}

/// (i) I-1: the TUI on a `worker` box admits nothing; that box's worker does.
#[tokio::test]
async fn a_tui_on_a_worker_box_admits_nothing() {
    let h = Harness::open().await;
    let edited = h
        .store
        .edit_box(
            ids::BOX,
            0,
            BoxEdit {
                executor: Some(Executor::Worker),
                ..BoxEdit::default()
            },
        )
        .await
        .expect("the edit answers");
    assert!(matches!(edited, CasOutcome::Applied(_)), "{edited:?}");
    let item = mint_ana(&h.store, "item", 0).await;
    h.queue(item).await;
    h.resume().await;
    let backend = h.backend();

    let mut tui = h.runtime();
    tui.sweep_with(&backend, &TestSink::default());
    settle(&mut tui).await;
    assert!(h.runs(item).await.is_empty(), "not the TUI's box to run");

    let mut worker = h.runtime().with_role(Role::Worker);
    worker.sweep_with(&backend, &TestSink::default());
    settle(&mut worker).await;
    assert_eq!(h.only_run(item).await.mode, RunMode::Auto);
}

/// (k) D8: with one slot and a single explicit sweep, the second entry can only be admitted by
/// the wake of the first walk's rest.
#[tokio::test]
async fn a_rested_walk_wakes_the_sweep() {
    let h = Harness::with_box_settings(json!({ "max_concurrent_items": 1 })).await;
    h.parts.open();
    let first = mint_ana(&h.store, "first", 1).await;
    let second = mint_ana(&h.store, "second", 0).await;
    h.queue(first).await;
    h.queue(second).await;
    h.resume().await;
    let mut runtime = h.runtime();
    runtime.sweep_with(&h.backend(), &TestSink::default());
    settle(&mut runtime).await;
    let (one, two) = (h.only_run(first).await, h.only_run(second).await);
    assert_eq!(one.status, RunStatus::AwaitingApproval);
    assert_eq!(two.status, RunStatus::AwaitingApproval);
    let parked_at = h
        .store
        .run_steps(one.id)
        .await
        .expect("the read answers")
        .iter()
        .filter_map(|step| step.finished_at)
        .max()
        .expect("the first run's steps finished");
    assert!(
        two.queued_at >= parked_at,
        "the second run was created after the first parked: {} < {parked_at}",
        two.queued_at
    );
}

/// (l) Review H1: a cancelled auto run sticks for the rest of its batch. The cancel moves the
/// item back to `open` and its entry stays, but the queue runner does not admit it again under
/// the batch that ran it; a pause and a resume (a new batch) do.
#[tokio::test]
async fn a_cancelled_auto_run_is_not_readmitted_in_its_batch() {
    let h = Harness::open().await;
    let item = mint_ana(&h.store, "cancelled", 0).await;
    h.queue(item).await;
    let batch = h.resume().await;
    let backend = h.backend();
    let sink = TestSink::default();
    let mut runtime = h.runtime();
    runtime.sweep_with(&backend, &sink);
    settle(&mut runtime).await;
    let run = h.only_run(item).await;
    assert_eq!(run.status, RunStatus::AwaitingApproval, "parked at verdict");

    let cancel = RunRequest::Orch(OrchRequest::Command(Command::CancelRun { run: run.id }));
    let _ = runtime
        .serve_request(&backend, &sink, 3, cancel, &LiveChats::default())
        .await;
    settle(&mut runtime).await;
    runtime.sweep_with(&backend, &sink);
    settle(&mut runtime).await;
    let only = h.only_run(item).await;
    assert_eq!(
        (only.id, only.status),
        (run.id, RunStatus::Cancelled),
        "the cancel sticks: no second run under the batch"
    );
    assert_eq!(h.status(item).await, Status::Open, "cancel moves it back");
    assert_eq!(
        h.store
            .queue_entries(ids::BOX)
            .await
            .expect("the read answers")
            .iter()
            .map(|entry| entry.item_id)
            .collect::<Vec<_>>(),
        [item],
        "the entry stays: queue membership is the user's call (D9)"
    );
    assert_eq!(
        h.store.batch_runs(batch).await.expect("the read answers"),
        vec![(run.id, RunStatus::Cancelled)]
    );

    h.store
        .close_batch(ids::BOX, BatchClose::Paused, Utc::now())
        .await
        .expect("the write answers")
        .expect("the batch was open");
    let next = h.resume().await;
    runtime.sweep_with(&backend, &sink);
    settle(&mut runtime).await;
    let runs = h.runs(item).await;
    assert_eq!(runs.len(), 2, "a new batch admits it once: {runs:?}");
    let again = runs
        .iter()
        .find(|row| row.id != run.id)
        .expect("the second run");
    assert_eq!(again.mode, RunMode::Auto);
    assert_eq!(
        h.store.batch_runs(next).await.expect("the read answers"),
        vec![(again.id, again.status)]
    );
}

/// (m) Review H2, criterion 27: a pause stops the closed batch's runs still waiting to be
/// claimed. Two runs are admitted; the second overlaps the first and waits `queued`. The pause
/// cancels it (its item back to `open`, its entry kept), so when the first run ends nothing of
/// the paused batch starts; a resume admits the item again.
#[tokio::test]
async fn pausing_cancels_the_batch_runs_still_waiting() {
    let h = Harness::overlapping().await;
    let (a, b) = (
        mint_ana(&h.store, "a", 1).await,
        mint_ana(&h.store, "b", 0).await,
    );
    h.queue(a).await;
    h.queue(b).await;
    let batch = h.resume().await;
    let backend = h.backend();
    let sink = TestSink::default();
    let mut runtime = h.runtime();
    runtime.sweep_with(&backend, &sink);
    eventually("one run runs and the other exists", || async {
        let (ra, rb) = (h.runs(a).await, h.runs(b).await);
        ra.len() == 1 && rb.len() == 1 && [ra[0].status, rb[0].status].contains(&RunStatus::Running)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (ra, rb) = (h.only_run(a).await, h.only_run(b).await);
    let (running, waiting, waiting_item) = if ra.status == RunStatus::Running {
        (ra, rb, b)
    } else {
        (rb, ra, a)
    };
    assert_eq!(running.status, RunStatus::Running);
    assert_eq!(
        waiting.status,
        RunStatus::Queued,
        "the claim refused it (Overlaps)"
    );

    h.store
        .close_batch(ids::BOX, BatchClose::Paused, Utc::now())
        .await
        .expect("the write answers")
        .expect("the batch was open");
    assert_eq!(
        h.only_run(waiting_item).await.status,
        RunStatus::Cancelled,
        "the pause cancelled the run still waiting"
    );
    assert_eq!(h.status(waiting_item).await, Status::Open);
    assert!(
        h.store
            .queue_entries(ids::BOX)
            .await
            .expect("the read answers")
            .iter()
            .any(|entry| entry.item_id == waiting_item),
        "the entry is kept"
    );

    h.parts.open();
    settle(&mut runtime).await;
    let rested = h
        .runs(if waiting_item == a { b } else { a })
        .await
        .into_iter()
        .next()
        .expect("the running run");
    assert_eq!(rested.id, running.id);
    assert_eq!(
        rested.status,
        RunStatus::AwaitingApproval,
        "the pause stopped nothing running"
    );
    h.store
        .finish_run(rested.id, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("a parked run cancels");
    runtime.sweep_with(&backend, &sink);
    settle(&mut runtime).await;
    let still = h.only_run(waiting_item).await;
    assert_eq!(
        (still.id, still.status),
        (waiting.id, RunStatus::Cancelled),
        "nothing of the paused batch starts"
    );
    assert_eq!(
        h.store
            .batch_runs(batch)
            .await
            .expect("the read answers")
            .len(),
        2
    );

    h.resume().await;
    runtime.sweep_with(&backend, &sink);
    settle(&mut runtime).await;
    assert_eq!(
        h.runs(waiting_item).await.len(),
        2,
        "a resume admits the cancelled item again"
    );
}

// ---------------------------------------------------------------------------------------------
// MOD-12 M2: the runner's spend gate (plan D4), over `Backend::Memory`
// ---------------------------------------------------------------------------------------------

/// What one parked htui ANA run spends under [`Harness::costing`]`(300)`: `research`'s session
/// and `verdict`'s, before the hard gate parks it.
const PARKED_SPEND: i64 = 600;

/// M2 (a)-(c), (e): a costing harness whose batch has spent [`PARKED_SPEND`] through `A1`, an
/// htui item now parked at `verdict`, with no cap planted yet (H-6).
async fn spent_batch() -> (Harness, RunRuntime<Backend, TestSink>, BatchId) {
    let h = Harness::costing(300).await;
    let a1 = mint_ana(&h.store, "a1", 0).await;
    h.queue(a1).await;
    let batch = h.resume().await;
    let mut runtime = h.runtime();
    h.sweep(&mut runtime).await;
    let run = h.only_run(a1).await;
    assert_eq!(run.status, RunStatus::AwaitingApproval, "parked at verdict");
    assert_eq!(
        h.batch_spend(batch).await,
        Some(PARKED_SPEND),
        "the parked run's sessions are the batch's spend"
    );
    (h, runtime, batch)
}

/// M2 (a) D2, D4: a batch whose spend reached the htui project's `per_token_cap_batch` admits
/// no further htui entry, and the loop goes on: an agy entry (no cap) behind it is admitted in
/// the same sweep.
#[tokio::test]
async fn a_batch_at_a_projects_cap_admits_only_other_projects() {
    let (h, mut runtime, batch) = spent_batch().await;
    h.cap_batch(ids::PROJECT_HTUI, json!(PARKED_SPEND)).await;
    let a2 = mint_ana(&h.store, "a2", 1).await;
    let b1 = mint_agy(&h.store, "b1", 0).await;
    h.queue(a2).await;
    h.queue(b1).await;
    h.sweep(&mut runtime).await;
    assert!(
        h.runs(a2).await.is_empty(),
        "the batch has spent the htui cap"
    );
    assert!(
        h.admitted_under(b1, batch).await,
        "agy has no cap, so its entry is admitted under the batch"
    );
}

/// M2 (b) D3, D4: what the batch has left under the cap is below `min_budget_for_new_attempt`,
/// so nothing is admitted.
#[tokio::test]
async fn a_remainder_below_the_minimum_admits_nothing() {
    let (h, mut runtime, _batch) = spent_batch().await;
    h.cap_batch(ids::PROJECT_HTUI, json!(PARKED_SPEND + 100))
        .await;
    h.store
        .set_app_setting(MIN_BUDGET_FOR_NEW_ATTEMPT, json!(200));
    let a2 = mint_ana(&h.store, "a2", 0).await;
    h.queue(a2).await;
    h.sweep(&mut runtime).await;
    assert!(
        h.runs(a2).await.is_empty(),
        "100 micros left is below the 200 a new attempt needs"
    );

    h.store
        .set_app_setting(MIN_BUDGET_FOR_NEW_ATTEMPT, json!(100));
    h.sweep(&mut runtime).await;
    assert!(
        !h.runs(a2).await.is_empty(),
        "exactly the minimum left is enough"
    );
}

/// M2 (c) D4: a stopped batch is left behind by a pause and a resume; the new batch has no spend
/// yet (unbounded), so the entry is admitted under it. Admission only (H-6).
#[tokio::test]
async fn a_pause_and_a_resume_open_a_batch_that_admits_again() {
    let (h, mut runtime, batch) = spent_batch().await;
    h.cap_batch(ids::PROJECT_HTUI, json!(PARKED_SPEND)).await;
    let a2 = mint_ana(&h.store, "a2", 0).await;
    h.queue(a2).await;
    h.sweep(&mut runtime).await;
    assert!(h.runs(a2).await.is_empty(), "stopped by the batch's cap");

    h.store
        .close_batch(ids::BOX, BatchClose::Paused, Utc::now())
        .await
        .expect("the write answers")
        .expect("the batch was open");
    let next = h.resume().await;
    assert_ne!(next, batch);
    assert_eq!(
        h.batch_spend(next).await,
        None,
        "a new batch has spent nothing"
    );
    h.sweep(&mut runtime).await;
    assert!(
        h.admitted_under(a2, next).await,
        "the new batch admits the entry"
    );
}

/// M2 (d) D4: a malformed cap fails its project's entries closed, and the next project's entry
/// is admitted in the same sweep.
#[tokio::test]
async fn a_malformed_cap_skips_its_project_and_admits_the_next() {
    let h = Harness::costing(300).await;
    h.cap_batch(ids::PROJECT_HTUI, json!("lots")).await;
    let a1 = mint_ana(&h.store, "a1", 1).await;
    let b1 = mint_agy(&h.store, "b1", 0).await;
    h.queue(a1).await;
    h.queue(b1).await;
    let batch = h.resume().await;
    let mut runtime = h.runtime();
    h.sweep(&mut runtime).await;
    assert!(
        h.runs(a1).await.is_empty(),
        "a cap that cannot be read admits nothing of its project"
    );
    assert!(h.admitted_under(b1, batch).await);
}

/// M2 (e) D4: a batch every entry of which is stopped stays open, keeps its entries and is not
/// drained (no new `closed_reason`), sweep after sweep.
#[tokio::test]
async fn a_stopped_batch_stays_open_and_keeps_its_entries() {
    let (h, mut runtime, batch) = spent_batch().await;
    h.cap_batch(ids::PROJECT_HTUI, json!(PARKED_SPEND)).await;
    let a2 = mint_ana(&h.store, "a2", 0).await;
    h.queue(a2).await;
    h.sweep(&mut runtime).await;
    h.sweep(&mut runtime).await;
    assert!(h.runs(a2).await.is_empty(), "stopped by the batch's cap");
    let open = h
        .store
        .open_batch_of(ids::BOX)
        .await
        .expect("the read answers")
        .expect("the stopped batch is still open");
    assert_eq!(open.id, batch);
    assert_eq!(open.closed_at, None);
    assert!(
        h.store
            .queue_entries(ids::BOX)
            .await
            .expect("the read answers")
            .iter()
            .any(|entry| entry.item_id == a2),
        "the stopped entry keeps its place"
    );
}

/// M2 (f) D1, D4: a manual run's spend is outside every batch, so a cap equal to it does not stop
/// the batch's first admission.
#[tokio::test]
async fn a_manual_runs_spend_does_not_count_against_the_batch() {
    let h = Harness::costing(300).await;
    let backend = h.backend();
    let sink = TestSink::default();
    let mut runtime = h.runtime();
    let manual = mint_ana(&h.store, "manual", 0).await;
    let _ = runtime
        .serve_request(&backend, &sink, 1, start_run(manual), &LiveChats::default())
        .await;
    settle(&mut runtime).await;
    assert_eq!(
        h.only_run(manual).await.status,
        RunStatus::AwaitingApproval,
        "a manual run keeps every gate, so it parks after its first session"
    );
    let manual_run = h.only_run(manual).await;
    let spent: i64 = h
        .store
        .run_steps(manual_run.id)
        .await
        .expect("the read answers")
        .iter()
        .filter_map(|step| step.usage.as_ref()?.get("cost_micros")?.as_i64())
        .sum();
    assert_eq!(spent, 300, "the manual run spent outside any batch");

    h.cap_batch(ids::PROJECT_HTUI, json!(300)).await;
    let a1 = mint_ana(&h.store, "a1", 0).await;
    h.queue(a1).await;
    let batch = h.resume().await;
    assert_eq!(h.batch_spend(batch).await, None, "no run of the batch yet");
    h.sweep(&mut runtime).await;
    assert!(
        h.admitted_under(a1, batch).await,
        "the manual run's 300 micros are not the batch's"
    );
}

// ---------------------------------------------------------------------------------------------
// The cases over Postgres
// ---------------------------------------------------------------------------------------------

async fn pg_runs(store: &PgStore, item: ItemId) -> Vec<RunSummary> {
    store.runs(item).await.expect("the read answers")
}

async fn pg_only_run(store: &PgStore, item: ItemId) -> RunSummary {
    let runs = pg_runs(store, item).await;
    assert_eq!(runs.len(), 1, "exactly one run of {item}: {runs:?}");
    runs.into_iter().next().expect("one run")
}

/// (pg-h) (h) over one database: two runtimes, each with its own pool, admit each item once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_runtimes_on_one_database_admit_each_item_once_pg() {
    let Some(db) = testkit::demo_db().await else {
        eprintln!("{}", testkit::SKIP);
        return;
    };
    seed(
        &db.store,
        db.store.agents().await.expect("the fixture's agents"),
        Repo::None,
    )
    .await;
    let other = testkit::fixture_box_store(&db.url).await;
    let parts = Parts::new();
    parts.open();
    let (x, y) = (
        mint_ana(&db.store, "x", 1).await,
        mint_ana(&db.store, "y", 0).await,
    );
    for item in [x, y] {
        db.store
            .queue_item(item, ids::BOX, ids::USER, Utc::now())
            .await
            .expect("the item queues");
    }
    db.store
        .open_batch(ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the batch opens");
    let mut first: RunRuntime<PgStore, TestSink> = parts.runtime();
    let mut second: RunRuntime<PgStore, TestSink> = parts.runtime();
    first.sweep_with(&db.store, &TestSink::default());
    second.sweep_with(&other, &TestSink::default());
    settle(&mut first).await;
    settle(&mut second).await;
    for item in [x, y] {
        assert_eq!(pg_only_run(&db.store, item).await.mode, RunMode::Auto);
    }
    db.drop_db().await;
}

/// (pg-f) (f) over Postgres, reading the closed row: the batch closes with reason `drained`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drained_batch_closes_drained_pg() {
    let Some(db) = testkit::demo_db().await else {
        eprintln!("{}", testkit::SKIP);
        return;
    };
    seed(
        &db.store,
        db.store.agents().await.expect("the fixture's agents"),
        Repo::None,
    )
    .await;
    let parts = Parts::new();
    parts.open();
    let item = mint_ana(&db.store, "drains", 0).await;
    db.store
        .queue_item(item, ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the item queues");
    let batch = db
        .store
        .open_batch(ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the batch opens")
        .id;
    let mut runtime: RunRuntime<PgStore, TestSink> = parts.runtime();
    runtime.sweep_with(&db.store, &TestSink::default());
    settle(&mut runtime).await;
    let run = pg_only_run(&db.store, item).await;
    assert_eq!(run.status, RunStatus::AwaitingApproval, "parked at verdict");

    db.store
        .finish_run(run.id, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("a parked run cancels");
    db.store
        .close_out(
            item,
            Resolution::Withdrawn,
            NewDocument {
                id: DocumentId::new(),
                item_id: item,
                kind: "summary".to_owned(),
                title: "withdrawn".to_owned(),
                body: "withdrawn".to_owned(),
                produced_by_step_id: None,
                created_by: ids::USER,
                created_at: Utc::now(),
            },
            &[],
        )
        .await
        .expect("an open item closes withdrawn");
    runtime.sweep_with(&db.store, &TestSink::default());
    settle(&mut runtime).await;

    let reason: Option<String> =
        sqlx::query_scalar("SELECT closed_reason FROM queue_batch WHERE id = $1")
            .bind(batch.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("the batch row is read");
    assert_eq!(reason.as_deref(), Some("drained"));
    db.drop_db().await;
}

/// (pg-g) (g1) over Postgres: the overlap serialises a manual and an auto run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_and_auto_overlap_serialise_on_postgres() {
    let Some(db) = testkit::demo_db().await else {
        eprintln!("{}", testkit::SKIP);
        return;
    };
    seed(
        &db.store,
        db.store.agents().await.expect("the fixture's agents"),
        Repo::Primary,
    )
    .await;
    let parts = Parts::new();
    let sink = TestSink::default();
    let mut runtime: RunRuntime<PgStore, TestSink> = parts.runtime();
    let manual = mint_ana(&db.store, "manual", 0).await;
    let _ = runtime
        .serve_request(
            &db.store,
            &sink,
            1,
            start_run(manual),
            &LiveChats::default(),
        )
        .await;
    eventually("the manual run runs", || {
        has_run_at(&db.store, manual, RunStatus::Running)
    })
    .await;

    let auto = mint_ana(&db.store, "auto", 0).await;
    db.store
        .queue_item(auto, ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the item queues");
    db.store
        .open_batch(ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the batch opens");
    runtime.sweep_with(&db.store, &sink);
    eventually("the auto run exists", || async {
        !pg_runs(&db.store, auto).await.is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let waiting = pg_only_run(&db.store, auto).await;
    assert_eq!(
        waiting.status,
        RunStatus::Queued,
        "the claim refused it (Overlaps)"
    );

    parts.open();
    settle(&mut runtime).await;
    let manual_run = pg_only_run(&db.store, manual).await;
    assert_eq!(manual_run.status, RunStatus::AwaitingApproval);
    assert_eq!(
        pg_only_run(&db.store, auto).await.status,
        RunStatus::Queued,
        "the parked manual run still holds the scope"
    );

    db.store
        .finish_run(manual_run.id, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("a parked run cancels");
    runtime.sweep_with(&db.store, &sink);
    settle(&mut runtime).await;
    assert_eq!(
        pg_only_run(&db.store, auto).await.status,
        RunStatus::AwaitingApproval,
        "claimed once the manual run ended"
    );
    match sink.reply(1).await {
        RunReply::Orch(OrchReply::Done(_)) => {}
        other => panic!("the manual StartRun walked, not {other:?}"),
    }
    db.drop_db().await;
}

/// (pg-overshoot) MOD-12 M2 D6, D7 (PRD metric "batch spend overshoot", the Pg half over
/// `SUM(run_step.usage)`; review R1 L5): the runtime's walk over Postgres, mirroring `htui-orch`'s
/// `a_batch_run_session_is_capped_at_the_batch_remainder`. A batch cap of 1 000 is planted
/// **before** admission, so the snapshot freezes it, and every session reports 600 micros.
/// `research` spends 600; `verdict` (ungated here, so the breach fails the step rather than
/// parking it) is handed the 400 left, spends 600 and is cut, its failure naming the batch. The
/// batch's Postgres sum is 1 200: past the cap by the one attempt that was in flight, no more. The
/// retry meets the walk's batch rule and no second `verdict` attempt is written; the next queue
/// entry is not admitted. Only batch-term behaviour is asserted: no `per_token_cap_run` is
/// planted, so the run term is unbounded throughout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_overshoots_its_cap_by_at_most_one_attempt_pg() {
    const COST: i64 = 600;
    const CAP: i64 = 1_000;
    let Some(db) = testkit::demo_db().await else {
        eprintln!("{}", testkit::SKIP);
        return;
    };
    seed(
        &db.store,
        db.store.agents().await.expect("the fixture's agents"),
        Repo::None,
    )
    .await;
    // `verdict` is a hard gate in the demo graph, which would park the cut attempt for a human;
    // ungated, the cut fails it and the walk tries again, as in the Mem case.
    sqlx::query(
        "UPDATE step_graph_phase SET gate = 'never', gate_hard = false WHERE name = 'verdict'",
    )
    .execute(&db.pool)
    .await
    .expect("verdict is ungated");
    let target = QueueTarget::Project(ids::PROJECT_HTUI);
    let token = db
        .store
        .queue_setting(target, QueueSetting::PerTokenCapBatch)
        .await
        .expect("the read answers")
        .expect("the project exists")
        .token;
    let planted = db
        .store
        .set_queue_setting(target, QueueSetting::PerTokenCapBatch, json!(CAP), token)
        .await
        .expect("the cap write answers");
    assert!(
        matches!(planted, CasOutcome::Applied(_)),
        "the cap lands: {planted:?}"
    );

    let parts = Parts::costing(COST);
    parts.open();
    let a1 = mint_ana(&db.store, "a1", 0).await;
    db.store
        .queue_item(a1, ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the item queues");
    let batch = db
        .store
        .open_batch(ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the batch opens")
        .id;
    let mut runtime: RunRuntime<PgStore, TestSink> = parts.runtime();
    runtime.sweep_with(&db.store, &TestSink::default());
    settle(&mut runtime).await;

    let run = pg_only_run(&db.store, a1).await;
    assert_eq!(run.mode, RunMode::Auto);
    let steps = db.store.run_steps(run.id).await.expect("the read answers");
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.phase_name.as_str(), step.attempt, step.status))
            .collect::<Vec<_>>(),
        [
            ("research", 1, StepStatus::Done),
            ("verdict", 1, StepStatus::Failed)
        ],
        "the recorder cut verdict, and no second verdict attempt was written"
    );
    // The recorder's own cut (`cap_exceeded`, then `done: cancelled`); its wording is not pinned
    // here. That the batch's remainder was the binding term is the failure's `(batch …)` below.
    let codes: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload->'code' FROM session_event WHERE run_step_id = $1 AND kind = 'error'",
    )
    .bind(steps[1].id.as_uuid())
    .fetch_all(&db.pool)
    .await
    .expect("the events read");
    assert_eq!(codes, [json!("cap_exceeded")], "the recorder cut verdict");
    let notes: Vec<String> = sqlx::query_scalar("SELECT body FROM item_note WHERE item_id = $1")
        .bind(a1.as_uuid())
        .fetch_all(&db.pool)
        .await
        .expect("the notes read");
    assert!(
        notes.iter().any(|note| note.contains(&format!(
            "step `verdict` attempt 1: cap breached (batch {batch})"
        ))),
        "the failure names the batch: {notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note.contains("batch cap reached (1200 of 1000 micros)")),
        "the retry met the walk's batch rule: {notes:?}"
    );
    assert_eq!(
        db.store
            .run(run.id)
            .await
            .expect("read")
            .map(|run| run.status),
        Some(RunStatus::Failed),
        "the refused retry ends the run"
    );

    let spent = db
        .store
        .batch_spend(batch)
        .await
        .expect("the Postgres sum answers")
        .expect("the batch has a spend");
    assert_eq!(spent, 2 * COST, "Σ run_step.usage over the batch");
    assert!(
        spent > CAP && spent - CAP <= COST,
        "the overshoot ({}) is at most the one attempt in flight ({COST})",
        spent - CAP
    );

    // The batch must still be open, so the next sweep declining to admit is the cap, not a
    // closed batch.
    let closed: Option<String> =
        sqlx::query_scalar("SELECT closed_reason FROM queue_batch WHERE id = $1")
            .bind(batch.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("the batch row is read");
    assert_eq!(closed, None, "the batch is still open after the overshoot");

    let a2 = mint_ana(&db.store, "a2", 1).await;
    db.store
        .queue_item(a2, ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the item queues");
    runtime.sweep_with(&db.store, &TestSink::default());
    settle(&mut runtime).await;
    assert!(
        pg_runs(&db.store, a2).await.is_empty(),
        "the batch is past its cap, so the next entry is not admitted"
    );
    assert_eq!(
        db.store.batch_spend(batch).await.expect("read"),
        Some(spent),
        "nothing was spent after the cut"
    );
    db.drop_db().await;
}
