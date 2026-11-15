//! MOD-133 M1 (D10): the sweep's land arm, end to end through the run runtime.
//!
//! A `done` run whose snapshot integrates a repo by `push`, and which changed that repo, holds one
//! `pending` integration record (D4). One `sweep_once` of the box that executed the run takes the
//! record under the runtime's owner and lands it through the engine (the `FakeIsolator` answers
//! `Landed`), so the record settles and the item carries the landing note; that the settle
//! succeeds at all pins the sweep's owner to the engine's (`settle_integration` answers `false`
//! for any other owner, and nothing is noted). A record of a run another box executed is left
//! alone, and a process whose role does not execute the box (I-1) lands nothing.
//!
//! MOD-133 M2 E6: `Unblock` on an item whose landing was refused puts the record back to `pending`;
//! when the process that serves it executes the box, it lands the record at once, in the same task,
//! after the answer. A process that does not (a TUI on a `worker` box) only resets it, and the
//! executing process's sweep lands it.
//!
//! The ticker is one hour, so every sweep a case sees is an explicit `sweep_with`.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{
    BoxEdit, BoxId, Claim, Executor, GraphSnapshot, IntegrateMode, IntegrateSnapshot,
    IntegrationState, Isolation, ItemId, LandOutcome, NewItem, NewRepo, NewRun, NewRunStep,
    PremergeReport, RepoId, RunId, RunMode, RunStatus, RunStepCommit, RunStepTree, StepId,
    StepRole, StepStatus,
};
use htui_core::prompt::DiffBlock;
use htui_core::store::{CasOutcome, MemStore, ReadStore as _, StepFence, WriteStore as _};
use htui_orch::Isolator;
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use htui_orch::isolate::{
    ChangedPaths, EnvFiles, FanoutSlot, IsolatorFuture, LandRequest, PartTip, Prepared, ResetReport,
};
use htui_orch::{Command, CommandOutcome, UnblockCase};
use htui_store::Backend;
use htui_worker::{
    LiveChats, OrchReply, OrchRequest, ReplySink, Role, RunReply, RunRequest, RunRuntime,
};
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

/// How long a case waits for the runtime to settle.
const PATIENCE: Duration = Duration::from_secs(20);

/// The remote of the `push` repo, with a credential in it.
const REMOTE: &str = "https://bot:secret@forge.example/o/landed.git";

/// The commit the run's one step made.
const AFTER: &str = "1111111111111111111111111111111111111111";

/// The refusal a case scripts for the first landing.
const REFUSAL: &str =
    "non_fast_forward: main on the remote has moved; merge it locally, then retry";

/// The address an `Unblock` request is answered at.
const UNBLOCK_ADDR: u64 = 7;

/// The answers nobody reads: the sweep is unaddressed.
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

/// The demo plus a second box, its queued `RUN_2` cancelled (so no sweep claims it), and the
/// isolator every runtime of a case shares.
struct Harness {
    store: MemStore,
    isolator: Arc<FakeIsolator>,
    other_box: BoxId,
}

impl Harness {
    async fn new() -> Self {
        let mut data = htui_core::fixtures::demo_data();
        let other_box = BoxId::new();
        let mut other = data.boxes[0].clone();
        other.id = other_box;
        other.hostname = "OTHER-BOX".to_owned();
        data.boxes.push(other);
        let store = MemStore::from_demo(data);
        store
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the seeded run is queued and cancellable");
        Self {
            store,
            isolator: Arc::new(FakeIsolator::new()),
            other_box,
        }
    }

    /// A runtime of `role` over the fakes, its ticker an hour.
    fn runtime(&self, role: Role) -> RunRuntime<Backend, TestSink> {
        RunRuntime::with_parts(
            Arc::clone(&self.isolator) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()),
            DriverFactory::new(),
        )
        .with_sweep_every(Duration::from_secs(3600))
        .with_role(role)
    }

    /// One explicit sweep of `runtime`, settled.
    async fn sweep(&self, runtime: &mut RunRuntime<Backend, TestSink>) {
        runtime.sweep_with(&Backend::memory(self.store.clone()), &TestSink::default());
        assert!(
            runtime.settle(PATIENCE).await.is_empty(),
            "no task is stuck"
        );
    }

    /// A fresh `push` repo of `htui` named `name`, with [`REMOTE`].
    async fn push_repo(&self, name: &str) -> RepoId {
        self.store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: name.to_owned(),
                remote_url: Some(REMOTE.to_owned()),
                default_branch: "main".to_owned(),
                is_primary: false,
            })
            .await
            .expect("the repo is fresh")
            .id
    }

    /// A fresh open item of `htui`.
    async fn item(&self, title: &str) -> ItemId {
        self.store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: title.to_owned(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                target_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the item mints")
            .id
    }

    /// A `done` graph run of `item` executed on `on_box`, whose snapshot integrates `repo` by
    /// `push` and whose one done step committed [`AFTER`] in it: `finish_run(Done)` writes its
    /// `pending` record (D4).
    async fn pushed_run(&self, item: ItemId, repo: RepoId, on_box: BoxId) -> RunId {
        let snapshot = self
            .store
            .run(ids::RUN_1)
            .await
            .expect("MemStore never fails a read")
            .and_then(|run| run.graph_snapshot)
            .expect("RUN_1 carries a snapshot");
        let mut snapshot: GraphSnapshot =
            serde_json::from_value(snapshot).expect("RUN_1's snapshot decodes");
        snapshot.integrate.insert(
            repo,
            IntegrateSnapshot {
                mode: IntegrateMode::Push,
                remote: REMOTE.to_owned(),
                target_branch: "main".to_owned(),
            },
        );
        let at = Utc::now();
        let owner = Uuid::now_v7();
        let run = self
            .store
            .create_run(NewRun {
                id: RunId::new(),
                project_id: ids::PROJECT_HTUI,
                item_id: item,
                mode: RunMode::Manual,
                target_box_id: on_box,
                started_by: ids::USER,
                graph_snapshot: snapshot,
                repo_scope: vec![repo],
                queued_at: at,
                batch_id: None,
                workspace_id: None,
            })
            .await
            .expect("the run is created")
            .id;
        assert_eq!(
            self.store
                .claim_run(run, on_box, owner, at, TimeDelta::minutes(5))
                .await
                .expect("the claim answers"),
            Claim::Admitted
        );
        let step = self
            .store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index: 0,
                phase_name: "implement".to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: Some("opus".to_owned()),
                role: StepRole::Step,
                parent_step_id: None,
                node_key: None,
            })
            .await
            .expect("the step is created")
            .id;
        assert!(
            self.store
                .transition_step(step, StepStatus::Pending, StepStatus::Running, at)
                .await
                .expect("the move answers")
        );
        self.store
            .record_commits(
                StepFence::Lease(owner),
                step,
                &[RunStepCommit {
                    run_step_id: step,
                    repo_id: repo,
                    before_hash: "0".repeat(40),
                    after_hash: Some(AFTER.to_owned()),
                }],
            )
            .await
            .expect("the commit is recorded");
        assert!(
            self.store
                .transition_step(step, StepStatus::Running, StepStatus::Done, at)
                .await
                .expect("the move answers")
        );
        self.store
            .finish_run(run, RunStatus::Done, None, at)
            .await
            .expect("the run finishes done");
        assert_eq!(
            self.states(run).await,
            [IntegrationState::Pending],
            "finish_run(Done) wrote the record"
        );
        run
    }

    /// The states of `run`'s records.
    async fn states(&self, run: RunId) -> Vec<IntegrationState> {
        self.store
            .integrations(run)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|record| record.state)
            .collect()
    }

    /// `item`'s note bodies that mention an integration.
    async fn integration_notes(&self, item: ItemId) -> Vec<String> {
        self.store
            .notes(item)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|note| note.body)
            .filter(|body| body.starts_with("integration "))
            .collect()
    }

    /// `Unblock` on `item`, served by `runtime` and settled: the command's one answer.
    async fn unblock(&self, runtime: &mut RunRuntime<Backend, TestSink>, item: ItemId) -> RunReply {
        let sink = TestSink::default();
        let request = RunRequest::Orch(OrchRequest::Command(Command::Unblock { item }));
        let _ = runtime
            .serve_request(
                &Backend::memory(self.store.clone()),
                &sink,
                UNBLOCK_ADDR,
                request,
                &LiveChats::default(),
            )
            .await;
        assert!(
            runtime.settle(PATIENCE).await.is_empty(),
            "no task is stuck"
        );
        let answers: Vec<RunReply> = sink
            .0
            .lock()
            .expect("the sink")
            .iter()
            .filter(|(to, _)| *to == UNBLOCK_ADDR)
            .map(|(_, reply)| reply.clone())
            .collect();
        let [answer] = &answers[..] else {
            panic!("one answer: {answers:?}");
        };
        answer.clone()
    }

    /// `item`'s note bodies written by `Unblock`.
    async fn unblock_notes(&self, item: ItemId) -> Vec<String> {
        self.store
            .notes(item)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|note| note.body)
            .filter(|body| body.starts_with("unblocked"))
            .collect()
    }

    /// Sets this box's executor.
    async fn executor(&self, executor: Executor) {
        let edited = self
            .store
            .edit_box(
                ids::BOX,
                0,
                BoxEdit {
                    executor: Some(executor),
                    ..BoxEdit::default()
                },
            )
            .await
            .expect("the edit answers");
        assert!(matches!(edited, CasOutcome::Applied(_)), "{edited:?}");
    }
}

/// D10: one sweep of the executing box lands its pending record, through the engine, under the
/// engine's own owner (the settle is noted); a second sweep takes nothing.
#[tokio::test]
async fn one_sweep_lands_a_pending_record_of_this_boxs_run() {
    let h = Harness::new().await;
    let repo = h.push_repo("landed").await;
    let item = h.item("pushed").await;
    let run = h.pushed_run(item, repo, ids::BOX).await;

    let mut runtime = h.runtime(Role::Tui);
    h.sweep(&mut runtime).await;

    let requests = h.isolator.land_requests();
    assert_eq!(requests.len(), 1, "landed once: {requests:?}");
    assert_eq!(
        (
            requests[0].repo,
            requests[0].remote.as_str(),
            requests[0].target_branch.as_str(),
            requests[0].commits.clone()
        ),
        (repo, REMOTE, "main", vec![AFTER.to_owned()])
    );
    let records = h
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        (records[0].state, records[0].landed_hash.as_deref()),
        (IntegrationState::Landed, Some(AFTER))
    );
    assert_eq!(
        h.integration_notes(item).await,
        ["integration landed: repo `landed` main at 1111111"],
        "the settle was the engine owner's own, so the item is noted"
    );

    h.sweep(&mut runtime).await;
    assert_eq!(
        h.isolator.land_requests().len(),
        1,
        "a landed record is not taken again"
    );
}

/// D10 (D6's box filter): a record of a run another box executed is never landed here.
#[tokio::test]
async fn a_record_of_another_boxs_run_is_left_alone() {
    let h = Harness::new().await;
    let repo = h.push_repo("elsewhere").await;
    let item = h.item("pushed elsewhere").await;
    let run = h.pushed_run(item, repo, h.other_box).await;

    let mut runtime = h.runtime(Role::Tui);
    h.sweep(&mut runtime).await;

    assert!(h.isolator.land_requests().is_empty());
    assert_eq!(h.states(run).await, [IntegrationState::Pending]);
    assert!(h.integration_notes(item).await.is_empty());
}

/// D10, I-1: a process whose role does not execute the box lands nothing; the box's executing
/// process (here the worker) does, through the same arm.
#[tokio::test]
async fn a_sweep_whose_role_does_not_execute_the_box_lands_nothing() {
    let h = Harness::new().await;
    h.executor(Executor::Worker).await;
    let repo = h.push_repo("worker").await;
    let item = h.item("pushed by the worker").await;
    let run = h.pushed_run(item, repo, ids::BOX).await;

    let mut tui = h.runtime(Role::Tui);
    h.sweep(&mut tui).await;
    assert!(
        h.isolator.land_requests().is_empty(),
        "not the TUI's box to land"
    );
    assert_eq!(h.states(run).await, [IntegrationState::Pending]);

    let mut worker = h.runtime(Role::Worker);
    h.sweep(&mut worker).await;
    assert_eq!(h.isolator.land_requests().len(), 1);
    assert_eq!(h.states(run).await, [IntegrationState::Landed]);
}

/// Whether `answer` is `Unblocked` by a retry of `run`'s landing.
fn retried(answer: &RunReply, run: RunId) -> bool {
    matches!(
        answer,
        RunReply::Orch(OrchReply::Done(outcome))
            if matches!(**outcome, CommandOutcome::Unblocked {
                case: UnblockCase::RetryLanding(id),
                rest: None,
                ..
            } if id == run)
    )
}

/// MOD-133 M2 E6: a refused landing, retried by `Unblock` on the box's executing process (here the
/// TUI on a `tui` box), is reset and landed once, in the same task; the next sweep takes nothing.
#[tokio::test]
async fn an_unblock_on_the_executing_process_lands_the_reset_record_once() {
    let h = Harness::new().await;
    let repo = h.push_repo("retried").await;
    let item = h.item("refused, then retried").await;
    let run = h.pushed_run(item, repo, ids::BOX).await;
    h.isolator.script_land(LandOutcome::Refused {
        reason: REFUSAL.to_owned(),
    });
    let mut runtime = h.runtime(Role::Tui);
    h.sweep(&mut runtime).await;
    assert_eq!(h.states(run).await, [IntegrationState::Refused]);
    assert_eq!(h.isolator.land_requests().len(), 1);
    let unblock = async || {
        htui_worker::actions(
            &Backend::memory(h.store.clone()),
            item,
            &LiveChats::default(),
        )
        .await
        .expect("MemStore never fails a read")
        .unblock
    };
    assert_eq!(unblock().await, Ok(()), "`u` reads the refused record");

    let answer = h.unblock(&mut runtime, item).await;
    assert!(retried(&answer, run), "{answer:?}");
    assert_eq!(
        h.isolator.land_requests().len(),
        2,
        "the reset record is landed once, by the Unblock's own task"
    );
    assert_eq!(h.states(run).await, [IntegrationState::Landed]);
    let id = run.to_string();
    assert_eq!(
        h.unblock_notes(item).await,
        [format!(
            "unblocked: retrying the landing of 1 repo(s) of run {}",
            &id[id.len() - 8..]
        )]
    );

    h.sweep(&mut runtime).await;
    assert_eq!(
        h.isolator.land_requests().len(),
        2,
        "nothing is left for the sweep"
    );
    assert!(
        unblock()
            .await
            .is_err_and(|why| why.ends_with("nothing is blocked")),
        "a landed record holds nothing"
    );
}

/// MOD-133 M2 E6, I-1: on a `worker` box the TUI's `Unblock` only resets the refused record; the
/// worker's next sweep lands it.
#[tokio::test]
async fn a_tui_unblock_on_a_worker_box_only_resets_the_record() {
    let h = Harness::new().await;
    h.executor(Executor::Worker).await;
    let repo = h.push_repo("handed over").await;
    let item = h.item("refused on a worker box").await;
    let run = h.pushed_run(item, repo, ids::BOX).await;
    h.isolator.script_land(LandOutcome::Refused {
        reason: REFUSAL.to_owned(),
    });
    let mut worker = h.runtime(Role::Worker);
    h.sweep(&mut worker).await;
    assert_eq!(h.states(run).await, [IntegrationState::Refused]);

    let mut tui = h.runtime(Role::Tui);
    let answer = h.unblock(&mut tui, item).await;
    assert!(retried(&answer, run), "{answer:?}");
    assert_eq!(
        h.states(run).await,
        [IntegrationState::Pending],
        "reset, not landed: the TUI does not execute this box"
    );
    assert_eq!(h.isolator.land_requests().len(), 1);

    h.sweep(&mut worker).await;
    assert_eq!(h.states(run).await, [IntegrationState::Landed]);
    assert_eq!(h.isolator.land_requests().len(), 2);
}

/// What a [`Watched`] isolator does before each `land` reaches the [`FakeIsolator`].
trait BeforeLand: Send + Sync + std::fmt::Debug {
    fn before(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// The [`FakeIsolator`], with a [`BeforeLand`] hook in front of its `land`.
#[derive(Debug)]
struct Watched<B> {
    inner: Arc<FakeIsolator>,
    before: B,
}

impl<B: BeforeLand> Isolator for Watched<B> {
    fn prepare<'a>(
        &'a self,
        run: RunId,
        step: StepId,
        scope: &'a [RepoId],
        isolation: Isolation,
        slot: Option<FanoutSlot<'a>>,
    ) -> IsolatorFuture<'a, Prepared> {
        self.inner.prepare(run, step, scope, isolation, slot)
    }

    fn capture<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        self.inner.capture(step, trees)
    }

    fn base<'a>(&'a self, scope: &'a [RepoId]) -> IsolatorFuture<'a, BTreeMap<RepoId, String>> {
        self.inner.base(scope)
    }

    fn diff<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Option<DiffBlock>> {
        self.inner.diff(trees, commits)
    }

    fn changed_paths<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, ChangedPaths> {
        self.inner.changed_paths(trees, commits)
    }

    fn env_files<'a>(
        &'a self,
        cwd: &'a Path,
        extra_dirs: &'a [PathBuf],
    ) -> IsolatorFuture<'a, EnvFiles> {
        self.inner.env_files(cwd, extra_dirs)
    }

    fn reconcile<'a>(
        &'a self,
        winner: StepId,
        trees: &'a [RunStepTree],
        siblings: &'a [StepId],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        self.inner.reconcile(winner, trees, siblings)
    }

    fn premerge<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
        parts: &'a [PartTip],
    ) -> IsolatorFuture<'a, PremergeReport> {
        self.inner.premerge(step, trees, parts)
    }

    fn unmerged<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
        heads: &'a [RunStepCommit],
        parts: &'a [PartTip],
    ) -> IsolatorFuture<'a, Vec<StepId>> {
        self.inner.unmerged(step, trees, heads, parts)
    }

    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()> {
        self.inner.cleanup(run, trees)
    }

    fn reset<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, ResetReport> {
        self.inner.reset(step, trees)
    }

    fn release<'a>(&'a self, run: RunId) -> IsolatorFuture<'a, ()> {
        self.inner.release(run)
    }

    fn reacquire<'a>(
        &'a self,
        run: RunId,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, ()> {
        self.inner.reacquire(run, step, trees)
    }

    fn land<'a>(&'a self, request: &'a LandRequest) -> IsolatorFuture<'a, LandOutcome> {
        Box::pin(async move {
            self.before.before().await;
            self.inner.land(request).await
        })
    }
}

/// Before each `land`, whether this box still has an open batch: the sweep's admission closes an
/// empty one (`drained`), so a `false` read means admission ran before the land arm.
#[derive(Debug)]
struct BatchWatch {
    store: MemStore,
    /// One entry per `land`: whether the box's batch was still open then.
    open_at_land: StdMutex<Vec<bool>>,
}

impl BeforeLand for BatchWatch {
    fn before(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let open = self
                .store
                .open_batch_of(ids::BOX)
                .await
                .expect("MemStore never fails a read")
                .is_some();
            self.open_at_land
                .lock()
                .expect("no panic holds the watch")
                .push(open);
        })
    }
}

/// A gate a case holds shut: while `held`, each `land` is counted in `entries`, says so on
/// `entered`, and waits for a permit of `release` (handed back, so one permit opens it for good).
#[derive(Debug)]
struct LandGate {
    held: AtomicBool,
    entries: AtomicUsize,
    entered: Notify,
    release: Semaphore,
}

impl LandGate {
    fn new() -> Self {
        Self {
            held: AtomicBool::new(false),
            entries: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
}

impl BeforeLand for LandGate {
    fn before(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if !self.held.load(Ordering::SeqCst) {
                return;
            }
            self.entries.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            let _permit = self
                .release
                .acquire()
                .await
                .expect("the gate is never closed");
        })
    }
}

/// A runtime of `role` over `isolator` and the fakes, its ticker an hour.
fn runtime_over(isolator: Arc<dyn Isolator>, role: Role) -> RunRuntime<Backend, TestSink> {
    RunRuntime::with_parts(
        isolator,
        Arc::new(FakeVerifier::new()),
        DriverFactory::new(),
    )
    .with_sweep_every(Duration::from_secs(3600))
    .with_role(role)
}

/// F2 M1: the land arm runs after the sweep's admission and claims, so a remote that does not
/// answer never holds them up. Here the box's batch is open and empty: admission closes it
/// (`drained`), and the land arm, which lands the pending record in the same sweep, sees it
/// closed.
#[tokio::test]
async fn the_land_arm_runs_after_admission_and_claims() {
    let h = Harness::new().await;
    let repo = h.push_repo("after").await;
    let item = h.item("landed after the claims").await;
    let run = h.pushed_run(item, repo, ids::BOX).await;
    h.store
        .open_batch(ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the batch opens");
    let watch = Arc::new(Watched {
        inner: Arc::clone(&h.isolator),
        before: BatchWatch {
            store: h.store.clone(),
            open_at_land: StdMutex::new(Vec::new()),
        },
    });

    let mut runtime = runtime_over(Arc::clone(&watch) as Arc<dyn Isolator>, Role::Tui);
    h.sweep(&mut runtime).await;

    assert_eq!(h.states(run).await, [IntegrationState::Landed]);
    assert_eq!(
        *watch
            .before
            .open_at_land
            .lock()
            .expect("no panic holds the watch"),
        [false],
        "the batch was drained before the land arm ran"
    );
}

/// MOD-133 M2 R1 (F1): while the `Unblock`'s own land of the reset record is in flight, a sweep of
/// the same process leaves the record to it (the run counts as live in the process's walks, which
/// the land arm skips), so the record is pushed once, never twice under the one lease owner.
#[tokio::test]
async fn a_sweep_during_the_unblocks_land_leaves_the_record_to_it() {
    let h = Harness::new().await;
    let repo = h.push_repo("gated").await;
    let item = h.item("refused, retried, swept").await;
    let run = h.pushed_run(item, repo, ids::BOX).await;
    h.isolator.script_land(LandOutcome::Refused {
        reason: REFUSAL.to_owned(),
    });
    let gated = Arc::new(Watched {
        inner: Arc::clone(&h.isolator),
        before: LandGate::new(),
    });
    let mut runtime = runtime_over(Arc::clone(&gated) as Arc<dyn Isolator>, Role::Tui);
    h.sweep(&mut runtime).await;
    assert_eq!(h.states(run).await, [IntegrationState::Refused]);

    gated.before.held.store(true, Ordering::SeqCst);
    let backend = Backend::memory(h.store.clone());
    let sink = TestSink::default();
    let request = RunRequest::Orch(OrchRequest::Command(Command::Unblock { item }));
    let _ = runtime
        .serve_request(
            &backend,
            &sink,
            UNBLOCK_ADDR,
            request,
            &LiveChats::default(),
        )
        .await;
    tokio::time::timeout(PATIENCE, gated.before.entered.notified())
        .await
        .expect("the Unblock's own task lands the reset record");
    runtime.sweep_with(&backend, &TestSink::default());
    // A sweep that took the record too would reach the gate at once: the store is in memory.
    let second =
        tokio::time::timeout(Duration::from_secs(2), gated.before.entered.notified()).await;
    gated.before.release.add_permits(1);
    assert!(
        runtime.settle(PATIENCE).await.is_empty(),
        "no task is stuck"
    );

    assert!(
        second.is_err(),
        "the sweep landed the record while the Unblock's land was in flight"
    );
    assert_eq!(gated.before.entries.load(Ordering::SeqCst), 1);
    assert_eq!(
        h.isolator.land_requests().len(),
        2,
        "refused once, then landed once"
    );
    assert_eq!(h.states(run).await, [IntegrationState::Landed]);
    let answers = sink.0.lock().expect("the sink").clone();
    let [(UNBLOCK_ADDR, answer)] = &answers[..] else {
        panic!("one answer: {answers:?}");
    };
    assert!(retried(answer, run), "{answer:?}");
}
