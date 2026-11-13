//! Deterministic test doubles behind `test-support`: [`FakeIsolator`], [`TestClock`],
//! `FakeGraphSource` and `FakeOrchestrator` over `MemStore` + `FakeDriver` + `FakeIsolator`.
//!
//! The determinism rules are `htui_agent::fake`'s (`crates/htui-agent/src/fake.rs:99-286`): nothing
//! reads the wall clock, nothing reads the filesystem, and every value a double hands back is a
//! function of what a test scripted into it. ANA-2 asks for this in as many words — the fake
//! orchestrator is what the Runs tab's `insta` snapshots run against, "so snapshots stay
//! byte-stable with no sleeps" (`docs/ANA-2.md:1764-1768`).
//!
//! Note for whoever reads `FakeGraphSource`: the demo fixture seeds no `phase_agent` row, so
//! `MemStore::phase_agents` answers empty until something calls `create_phase_agents` (MOD-37
//! R-6), and it seeds no `agent_box` row either. Rungs 1 and 3 of ANA-2 §4.1's candidate chain
//! are both empty on `MemStore::demo()`, so the fake carries an explicit per-phase candidates map
//! as its documented stand-in (plan D20, blueprint A-1).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::TimeDelta;
use htui_agent::conformance::{Script, ScriptEvent, epoch};
use htui_agent::driver::{
    AgentDriver, AgentSession, DriverCaps, DriverFuture, McpServerSpec, PermissionPolicy,
    SessionSpec,
};
use htui_agent::error::DriverError;
use htui_agent::event::{
    DoneEvent, DriverEvent, ErrorEvent, PermissionRequestEvent, StopReason, ToolCallEvent,
    ToolKind, UsageEvent,
};
use htui_agent::fake::{FAKE_AGENT_NAME, FakeDriver, SpecSlot};
use htui_agent::record::{Control, Signal};
use htui_core::fixtures::ids;
use htui_core::model::Transport;
use htui_core::model::{
    Agent, AgentBox, AgentId, BatchId, BoundSkill, BoxId, Document, DocumentId, GraphSnapshot,
    Isolation, Item, ItemId, ItemState, NewDocument, PartMerge, PartPremerge, PhaseAgent, PhaseId,
    PremergeReport, ProjectId, PromptTemplate, RepoId, RepoPremerge, ResolvedGraph, Run, RunId,
    RunStep, RunStepCommit, RunStepTree, SnapshotPhase, StepId, UserId, VerifyOutcome,
    latest_graph_run,
};
use htui_core::prompt::DiffBlock;
use htui_core::secret::SecretSource;
use htui_core::store::{MemStore, ReadStore, Result, StoreError, WriteStore};
use tokio::sync::{Notify, watch};
use uuid::Uuid;

pub use htui_core::clock::TestClock;

/// MOD-144: the item's derived display state, its stored status over its latest graph run
/// ([`ItemState::of`], [`latest_graph_run`]).
///
/// # Panics
/// When the item is not there; `MemStore` never fails a read.
pub async fn state_in(store: &MemStore, id: ItemId) -> ItemState {
    let item = store
        .item(id)
        .await
        .expect("MemStore never fails a read")
        .expect("the item exists");
    let runs = store.runs(id).await.expect("MemStore never fails a read");
    ItemState::of(item.status, latest_graph_run(&runs).map(|run| run.status))
}

use crate::command::{Command, CommandOutcome, EngineError};
use crate::engine::{ControlFor, DeadWalks, PolicyFor, SessionKey, Tails};
use crate::graph::GraphSource;
use crate::isolate::{
    ChangedPaths, Clock, EnvFiles, FanoutSlot, IsolateError, Isolator, IsolatorFuture, PartTip,
    Prepared, PreparedTree, ResetReport,
};
use crate::tools::{ToolHost, ToolHostError, ToolLease, ToolScope};
use crate::verify::{Verifier, VerifierFuture, VerifyReport, VerifyRequest};

/// The root every synthetic tree path hangs from. Nothing ever creates it.
const FAKE_TREE_ROOT: &str = "/fake/trees";

/// The body a [`FakeOrchestrator::refuse_prompt`] template carries: a placeholder no role has,
/// which `assemble` refuses as `TemplateError::UnknownPlaceholder` (ANA-5 criterion 3's literal
/// case, MOD-4 plan D162).
pub const REFUSED_TEMPLATE_BODY: &str = "{{no_such_placeholder}}";

/// A reconcile stall: the 1-based call it takes, the signal that call raises, and the resume it
/// waits for — `None` for a call that never answers (blueprint F-A, plan D145).
type ReconcileStall = (u32, Arc<Notify>, Option<Arc<Notify>>);

/// An [`Isolator`] that creates **no filesystem state at all**.
///
/// The plan asks for a fake "creating no directory tree that outlives the test"; this one creates
/// none in the first place, which is strictly stronger and means a case can be run on a box with no
/// git, no repos and no write permission anywhere. Paths are strings of the shape
/// `/fake/trees/<run>/<step>/<repo>` and hashes are `fake:base:<n>` / `fake:after:<step>:<n>`,
/// where `n` is a per-isolator call counter — so two isolators never collide and one isolator never
/// repeats itself by accident, which is exactly what criterion 7's *deliberate* repeat
/// ([`script_after`](FakeIsolator::script_after)) has to be distinguishable from.
#[derive(Debug, Default)]
pub struct FakeIsolator {
    /// Scripted `after_hash` values, FIFO, one entry consumed per `capture`/`reconcile` call and
    /// applied to every repo of that call. `Some(None)` is "this step committed nothing".
    after: Mutex<VecDeque<Option<String>>>,
    /// The synthetic-hash counter, shared by `prepare` and the unscripted `capture` path.
    calls: Mutex<u32>,
    /// Scripted [`prepare`](Isolator::prepare) refusals, FIFO, one consumed per call.
    refusals: Mutex<VecDeque<String>>,
    /// Scripted [`reconcile`](Isolator::reconcile) failures, FIFO, one consumed per call.
    reconcile_failures: Mutex<VecDeque<IsolateError>>,
    /// How many times [`prepare`](Isolator::prepare) has been called, refusals included.
    prepares: Mutex<u32>,
    /// How many times [`cleanup`](Isolator::cleanup) has been called (plan D36: once per run, at
    /// the end).
    cleanups: Mutex<u32>,
    /// What [`prepare`](Isolator::prepare) reports as `Prepared.extra_dirs` (plan D28).
    extra_dirs: Mutex<Vec<PathBuf>>,
    /// What [`capture`](Isolator::capture) last reported per step, which is what the identity
    /// [`reconcile`](Isolator::reconcile) echoes.
    captured: Mutex<BTreeMap<StepId, Vec<RunStepCommit>>>,
    /// Scripted [`diff`](Isolator::diff) answers, FIFO, one consumed per call; unscripted is
    /// `None` (MOD-4 milestone 4 D54(c)); `Err` is a scripted [`fail_diff`](Self::fail_diff).
    diffs: Mutex<VecDeque<std::result::Result<Option<DiffBlock>, IsolateError>>>,
    /// The `run_step_id`s of the tree and commit rows each [`diff`](Isolator::diff) call was
    /// handed, in call order — so a case can tell *whose* rows plan D67 diffed.
    diff_requests: Mutex<Vec<DiffRequest>>,
    /// Scripted [`changed_paths`](Isolator::changed_paths) answers, FIFO, one consumed per call;
    /// unscripted is `ChangedPaths::default()` (MOD-9 D119); `Err` is a scripted
    /// [`fail_changed_paths`](Self::fail_changed_paths).
    changed: Mutex<VecDeque<std::result::Result<ChangedPaths, IsolateError>>>,
    /// The rows each [`changed_paths`](Isolator::changed_paths) call was handed, in call order, in
    /// [`DiffRequest`]'s shape (MOD-9 D135), so a case can tell *whose* rows a retry read.
    changed_path_requests: Mutex<Vec<DiffRequest>>,
    /// Every [`reconcile`](Isolator::reconcile) call, as `(winner, siblings)`, in call order — so a
    /// case can tell which siblings plan D54(d) handed the isolator.
    reconciles: Mutex<Vec<(StepId, Vec<StepId>)>>,
    /// Plan D28's per-repo lock, one for the whole fake: a `shared_serialized`
    /// [`prepare`](Isolator::prepare) waits on it, and holds it until that step's
    /// [`capture`](Isolator::capture), the run's [`cleanup`](Isolator::cleanup) or its
    /// [`release`](Isolator::release).
    serial: Arc<tokio::sync::Mutex<()>>,
    /// The `shared_serialized` guard per step that holds [`serial`](Self::serial), keyed by the
    /// run too (blueprint F-I) so [`release`](Isolator::release) can find a run's steps.
    held: Mutex<BTreeMap<(RunId, StepId), tokio::sync::OwnedMutexGuard<()>>>,
    /// Scripted [`reset`](Isolator::reset) refusals, FIFO, one consumed per call.
    reset_refusals: Mutex<VecDeque<String>>,
    /// How many times [`reset`](Isolator::reset) has been called, refusals included.
    resets: Mutex<u32>,
    /// How many times [`release`](Isolator::release) has been called.
    releases: Mutex<u32>,
    /// Blueprint F-A: the 1-based [`reconcile`](Isolator::reconcile) call that never returns, and
    /// the signal it raises first; with a resume signal (plan D145), the call that waits for it
    /// instead. One-shot each; MOD-138 M3 T4: several may be pending at once, one per call.
    reconcile_stall: Mutex<Vec<ReconcileStall>>,
    /// Where `prepare` roots its trees, when a case points it at a real directory (MOD-7
    /// milestone 4); `None` keeps `FAKE_TREE_ROOT/<run>/<step>`.
    tree_root: Mutex<Option<PathBuf>>,
    /// MOD-89: scripted [`env_files`](Isolator::env_files) answers, FIFO, one consumed per call;
    /// unscripted is `EnvFiles::default()`; `Err` is a scripted
    /// [`fail_env_files`](Self::fail_env_files). Not named `env_files`: a field and the trait
    /// method of one name read badly.
    env_file_answers: Mutex<VecDeque<std::result::Result<EnvFiles, IsolateError>>>,
    /// Every [`env_files`](Isolator::env_files) call's `(cwd, extra_dirs)`, in call order.
    env_file_requests: Mutex<Vec<(PathBuf, Vec<PathBuf>)>>,
    /// MOD-89 review L5: the signal the next [`env_files`](Isolator::env_files) call sends, and
    /// the grace of the cancel it sends, before it answers. One-shot.
    cancel_at_env_files: Mutex<Option<(watch::Sender<Signal>, std::time::Duration)>>,
    /// MOD-138 M3 T3: every `prepare`, `capture`, `reconcile`, `reset`, `cleanup` and `release`,
    /// in call order, refusals included, for the linear call-order case.
    call_log: Mutex<Vec<IsolatorCall>>,
    /// MOD-138 M3 T3 (F14): whether each `reconcile` yields once after it records itself, so two
    /// landings in flight interleave rather than each running to completion in one poll.
    interleave_reconciles: AtomicBool,
    /// MOD-138 M3 T3: the `reconcile` calls in progress now, and the most there ever were.
    reconciling: Mutex<(u32, u32)>,
    /// MOD-138 M3 T3: each step's open window, by `(step, Window)`, and whether it is in place:
    /// `prepare`..`capture` for a tree, the call itself for a `reconcile`.
    windows: Mutex<BTreeMap<(StepId, Window), bool>>,
    /// MOD-138 M3 T3: how many windows opened beside another step's open window while either of
    /// the two was in place (`local` or `shared_serialized`).
    in_place_overlaps: Mutex<u32>,
    /// MOD-139 M3 T1: scripted `premerge` answers, FIFO, one consumed per call.
    premerge_answers: Mutex<VecDeque<PremergeScript>>,
    /// `(integrate step, part steps in the order handed)` per `premerge` call.
    premerge_requests: Mutex<Vec<(StepId, Vec<StepId>)>>,
    /// `premerge` calls so far, refusals and stalls included.
    premerges: Mutex<u32>,
    /// 1-based `premerge` calls that never return, with their signal (one-shot each).
    premerge_stall: Mutex<Vec<(u32, Arc<Notify>)>>,
    /// Scripted `unmerged` answers by part key, or an error, FIFO, one consumed per call.
    unmerged_answers: Mutex<VecDeque<std::result::Result<Vec<String>, IsolateError>>>,
    /// `(integrate step, part steps)` per `unmerged` call.
    unmerged_requests: Mutex<Vec<(StepId, Vec<StepId>)>>,
    /// 1-based `reset` calls that never return (counted by `resets`), with their signal.
    reset_stall: Mutex<Vec<(u32, Arc<Notify>)>>,
    /// MOD-125 M3 D4: 1-based `prepare` calls that never return (counted by `prepares`), with
    /// their signal (one-shot each): a stop before the step's session opened.
    prepare_stall: Mutex<Vec<(u32, Arc<Notify>)>>,
    /// MOD-125 M3 D5a: how many times [`reacquire`](Isolator::reacquire) has been called,
    /// refusals included.
    reacquires: Mutex<u32>,
    /// MOD-125 M3 D5a: scripted [`reacquire`](Isolator::reacquire) refusals, FIFO, one consumed
    /// per call.
    reacquire_refusals: Mutex<VecDeque<String>>,
}

/// One scripted `premerge` answer (by part key, blueprint P-15: part step ids are minted at
/// admission, after a case starts its run).
#[derive(Debug, Clone)]
enum PremergeScript {
    /// The default report, with `key` `Conflicted { paths }` in every repo it committed in and
    /// the later parts that committed there `NotAttempted`.
    Conflict { key: String, paths: Vec<String> },
    /// `Err(IsolateError::Refused(reason))`.
    Refuse(String),
}

/// Takes the entry for the 1-based `call` out of `stalls` (one-shot), answering its signal.
fn stall_at(stalls: &Mutex<Vec<(u32, Arc<Notify>)>>, call: u32) -> Option<Arc<Notify>> {
    let mut stalls = stalls
        .lock()
        .expect("no panic holds the fake isolator's lock");
    let at = stalls.iter().position(|(n, _)| *n == call)?;
    Some(stalls.remove(at).1)
}

/// One [`FakeIsolator`] call, as [`FakeIsolator::call_log`] records it (MOD-138 M3 T3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatorCall {
    /// `prepare` for the step.
    Prepare(StepId),
    /// `capture` for the step.
    Capture(StepId),
    /// `reconcile` with the step as the winner.
    Reconcile(StepId),
    /// `reset` of the step's trees.
    Reset(StepId),
    /// The run-terminal `cleanup`.
    Cleanup(RunId),
    /// `release` of the run's guards.
    Release(RunId),
    /// MOD-139 M3: `premerge` of the **integrate** step's trees.
    Premerge(StepId),
    /// MOD-139 M3: `unmerged`, the post-check of the **integrate** step.
    Unmerged(StepId),
    /// MOD-125 M3 D5a: `reacquire` of a stopped step's trees.
    Reacquire(StepId),
}

/// Which of a step's two isolator windows is open (MOD-138 M3 T3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Window {
    /// `prepare` to `capture`: the step's agent may be working in its trees.
    Tree,
    /// Inside `reconcile`: the step is landing.
    Landing,
}

/// Closes a `reconcile`'s window and its in-progress count on every exit, the stall's drop
/// included.
struct Reconciling<'a> {
    isolator: &'a FakeIsolator,
    winner: StepId,
}

impl Drop for Reconciling<'_> {
    fn drop(&mut self) {
        self.isolator.close_window(self.winner, Window::Landing);
        let mut reconciling = self
            .isolator
            .reconciling
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reconciling.0 = reconciling.0.saturating_sub(1);
    }
}

/// One [`diff`](Isolator::diff) call as [`FakeIsolator`] saw it: the `run_step_id` of every tree
/// row, then of every commit row, in the order the engine passed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest {
    /// `RunStepTree::run_step_id` per tree row.
    pub trees: Vec<StepId>,
    /// `RunStepCommit::run_step_id` per commit row.
    pub commits: Vec<StepId>,
}

impl FakeIsolator {
    /// A fresh isolator: nothing scripted, counter at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue the `after_hash` the next [`capture`](Isolator::capture) reports for every repo.
    ///
    /// `Some(hash)` pins the value — criterion 7 queues two *identical* ones, which is the whole
    /// content of "two consecutive implement attempts producing an identical `after_hash`"
    /// (`docs/ANA-2.md:2103`). `None` is "the step committed nothing", the shape the recovery sweep
    /// of milestone 5 reads as an unfinished step.
    pub fn script_after(&self, hash: Option<&str>) {
        self.after
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(hash.map(str::to_owned));
    }

    /// Make the next [`prepare`](Isolator::prepare) refuse with `reason` instead of inventing
    /// trees.
    ///
    /// Stage 2 is the first thing the walk does after it moves a step `pending -> running`
    /// (`docs/ANA-2.md:633`), so this is the shortest way to put an error in the region where a
    /// step is live and no settle has happened yet — the region ANA-2 `:639` gives an
    /// unconditioned `running -> failed`.
    pub fn refuse_prepare(&self, reason: &str) {
        self.refusals
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(reason.to_owned());
    }

    /// Make the next [`reconcile`](Isolator::reconcile) refuse with `reason`.
    ///
    /// `reconcile` lands on a step that is already `done`, which is the one place in the walk
    /// where `fail_hard` cannot move the row (blueprint H-2) — so what a refusal here does is a
    /// park, and a test needs to be able to cause one.
    pub fn refuse_reconcile(&self, reason: &str) {
        self.push_reconcile_failure(IsolateError::Refused(reason.to_owned()));
    }

    /// Make the next [`reconcile`](Isolator::reconcile) fail with a `git` error rather than a
    /// refusal — an `index.lock` that outlived its three retries, say.
    ///
    /// The distinction is the one blueprint H-2 had to decide: a refusal is a sentence about the
    /// tree, a `Git` error is a sentence about the verb, and **both** park, because the step is
    /// `done` either way and there is no status a failure could be written to.
    pub fn fail_reconcile(&self, reason: &str) {
        self.push_reconcile_failure(IsolateError::Git(reason.to_owned()));
    }

    /// The queue both of the two above push to.
    fn push_reconcile_failure(&self, err: IsolateError) {
        self.reconcile_failures
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(err);
    }

    /// Root every tree [`prepare`](Isolator::prepare) reports at `dir`: `cwd` becomes `dir` and
    /// each repo's tree `dir/<repo id>`, for every step (MOD-7 milestone 4). The fake still creates
    /// nothing; a case that wants files under a tree writes them there itself, which is how the
    /// excerpt pass gets a readable root without a real isolator.
    pub fn root_trees_at(&self, dir: impl Into<PathBuf>) {
        *self
            .tree_root
            .lock()
            .expect("no panic holds the fake isolator's lock") = Some(dir.into());
    }

    /// Make every [`prepare`](Isolator::prepare) report `dirs` as its `extra_dirs`.
    ///
    /// The fake puts all of its trees under one `cwd`, which is the `worktree`/`copy` shape and
    /// leaves the list empty; plan D28's `local`/`shared_serialized` shape is the one that fills
    /// it, and this is how a case reaches it without a filesystem.
    pub fn script_extra_dirs(&self, dirs: Vec<PathBuf>) {
        *self
            .extra_dirs
            .lock()
            .expect("no panic holds the fake isolator's lock") = dirs;
    }

    /// Queue the answer of the next [`diff`](Isolator::diff): a block, or `None` for "nothing to
    /// show". An unscripted call answers `None`, so a case that never scripts one sees the shape
    /// of a box with no `git`.
    pub fn script_diff(&self, block: Option<DiffBlock>) {
        self.diffs
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Ok(block));
    }

    /// Make the next [`diff`](Isolator::diff) fail with a `git` error, the shape plan D67 degrades
    /// to a prompt note rather than a failed step (the diff is advisory, D55).
    pub fn fail_diff(&self, reason: &str) {
        self.diffs
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Err(IsolateError::Git(reason.to_owned())));
    }

    /// Every [`diff`](Isolator::diff) call so far, as the rows it was handed.
    #[must_use]
    pub fn diff_requests(&self) -> Vec<DiffRequest> {
        self.diff_requests
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .clone()
    }

    /// MOD-9 D119: queue the answer of the next [`changed_paths`](Isolator::changed_paths). An
    /// unscripted call answers `ChangedPaths::default()`, the shape of a box with no `git`.
    pub fn script_changed_paths(&self, paths: ChangedPaths) {
        self.changed
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Ok(paths));
    }

    /// MOD-9 D119: make the next [`changed_paths`](Isolator::changed_paths) fail with a `git`
    /// error, which the engine degrades to a prompt note (MOD-9 D122).
    pub fn fail_changed_paths(&self, reason: &str) {
        self.changed
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Err(IsolateError::Git(reason.to_owned())));
    }

    /// MOD-9 D135: every [`changed_paths`](Isolator::changed_paths) call so far, as the rows it
    /// was handed.
    #[must_use]
    pub fn changed_path_requests(&self) -> Vec<DiffRequest> {
        self.changed_path_requests
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .clone()
    }

    /// MOD-89 D2: queue the answer of the next [`env_files`](Isolator::env_files). An unscripted
    /// call answers `EnvFiles::default()`: trees that hold no env file.
    pub fn script_env_files(&self, files: EnvFiles) {
        self.env_file_answers
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Ok(files));
    }

    /// MOD-89 D2: make the next [`env_files`](Isolator::env_files) fail with a `git` error, which
    /// the engine reads as "nothing found" plus a `warn`.
    pub fn fail_env_files(&self, reason: &str) {
        self.env_file_answers
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Err(IsolateError::Git(reason.to_owned())));
    }

    /// MOD-89 review L5: the next [`env_files`](Isolator::env_files) call sends
    /// `Cancel { grace }` on `signal` before it answers, as a cancel landing mid-scan would.
    fn cancel_at_env_files(&self, signal: watch::Sender<Signal>, grace: std::time::Duration) {
        *self
            .cancel_at_env_files
            .lock()
            .expect("no panic holds the fake isolator's lock") = Some((signal, grace));
    }

    /// MOD-89: every [`env_files`](Isolator::env_files) call so far, as `(cwd, extra_dirs)`,
    /// failed ones included.
    #[must_use]
    pub fn env_file_requests(&self) -> Vec<(PathBuf, Vec<PathBuf>)> {
        self.env_file_requests
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .clone()
    }

    /// Every [`reconcile`](Isolator::reconcile) call so far, as `(winner, siblings)`, refusals
    /// included.
    #[must_use]
    pub fn reconciles(&self) -> Vec<(StepId, Vec<StepId>)> {
        self.reconciles
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .clone()
    }

    /// How many times [`prepare`](Isolator::prepare) has been called on this isolator.
    ///
    /// The trait promises no idempotence (`crate::isolate::Isolator::prepare`) and this fake mints
    /// a fresh `before_hash` per call, so "once per step" is a property a case has to be able to
    /// assert rather than assume.
    #[must_use]
    pub fn prepares(&self) -> u32 {
        *self
            .prepares
            .lock()
            .expect("no panic holds the fake isolator's lock")
    }

    /// How many times [`cleanup`](Isolator::cleanup) has been called.
    ///
    /// Plan D36 makes this exactly one per run, after the terminal `finish_run`, and ANA-2
    /// invariant 6 makes "not once per step" the whole point — so it is a count and not a flag.
    #[must_use]
    pub fn cleanups(&self) -> u32 {
        *self
            .cleanups
            .lock()
            .expect("no panic holds the fake isolator's lock")
    }

    /// Make the next [`reset`](Isolator::reset) refuse with `reason`, reported against the first
    /// row's repository — the shape D93's park reads from a real isolator's `dirty_tree_not_reset`.
    pub fn script_reset_refusal(&self, reason: &str) {
        self.reset_refusals
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(reason.to_owned());
    }

    /// How many times [`reset`](Isolator::reset) has been called, refusals included.
    #[must_use]
    pub fn resets(&self) -> u32 {
        *self
            .resets
            .lock()
            .expect("no panic holds the fake isolator's lock")
    }

    /// How many times [`release`](Isolator::release) has been called.
    #[must_use]
    pub fn releases(&self) -> u32 {
        *self
            .releases
            .lock()
            .expect("no panic holds the fake isolator's lock")
    }

    /// MOD-125 M3 D5a: how many times [`reacquire`](Isolator::reacquire) has been called,
    /// refusals included.
    #[must_use]
    pub fn reacquires(&self) -> u32 {
        *self
            .reacquires
            .lock()
            .expect("no panic holds the fake isolator's lock")
    }

    /// MOD-125 M3 D5a: make the next [`reacquire`](Isolator::reacquire) refuse with `reason` —
    /// the real isolator's `moved_while_stopped` — holding no guard afterwards.
    pub fn refuse_reacquire(&self, reason: &str) {
        self.reacquire_refusals
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(reason.to_owned());
    }

    /// Make the `n`-th [`reconcile`](Isolator::reconcile) call (1-based, counting every call this
    /// isolator has seen) record itself, signal the returned [`Notify`] and then **never return**
    /// (blueprint F-A, A-2).
    ///
    /// This is the crash between stage 6's `done` and the merge (plan D97): a *failed* reconcile
    /// parks the run, which no sweep adopts, so only a reconcile that never answers leaves the run
    /// `running` with an unmerged winner. The case drops the walk once the signal fires
    /// (`conformance::until_stalled`).
    ///
    /// MOD-138 M3 T4: stalls add up, so a concurrent walk can leave two winners unmerged.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn stall_nth_reconcile(&self, n: u32) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.reconcile_stall
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push((n, Arc::clone(&notify), None));
        notify
    }

    /// Make the `n`-th [`reconcile`](Isolator::reconcile) call (1-based, as
    /// [`stall_nth_reconcile`](Self::stall_nth_reconcile)) record itself, signal the first
    /// returned [`Notify`], and wait for the second before it answers as it otherwise would.
    ///
    /// Plan D145's seam: another writer moves a row while a sweep's recovery is inside the
    /// reconcile, so the write after it finds the row moved.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn pause_nth_reconcile(&self, n: u32) -> (Arc<Notify>, Arc<Notify>) {
        let (reached, resume) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        self.reconcile_stall
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push((n, Arc::clone(&reached), Some(Arc::clone(&resume))));
        (reached, resume)
    }

    /// MOD-139 M3 T1: the next [`premerge`](Isolator::premerge) answers the default report, but
    /// the part keyed `key` is `Conflicted { paths }` in every repo it committed in and every
    /// later part that committed there `NotAttempted`; a part with no commits in a repo reads
    /// `NoCommits` there, as GixIsolator answers (scripted by key, blueprint P-15). FIFO with
    /// [`refuse_premerge`](Self::refuse_premerge).
    pub fn conflict_premerge(&self, key: &str, paths: &[&str]) {
        self.premerge_answers
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(PremergeScript::Conflict {
                key: key.to_owned(),
                paths: paths.iter().map(|path| (*path).to_owned()).collect(),
            });
    }

    /// MOD-139 M3 T1: the next [`premerge`](Isolator::premerge) fails with
    /// `IsolateError::Refused(reason)`, which fails the step (plan D3).
    pub fn refuse_premerge(&self, reason: &str) {
        self.premerge_answers
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(PremergeScript::Refuse(reason.to_owned()));
    }

    /// MOD-139 M3 T1: the `n`-th [`premerge`](Isolator::premerge) call (1-based) logs itself,
    /// signals the returned [`Notify`] and then **never returns**: a crash mid-pre-merge.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn stall_nth_premerge(&self, n: u32) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.premerge_stall
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push((n, Arc::clone(&notify)));
        notify
    }

    /// MOD-139 M3 T1: the `n`-th [`reset`](Isolator::reset) call (1-based, as
    /// [`resets`](Self::resets) counts) logs itself, is counted, signals the returned [`Notify`]
    /// and then **never returns**: a crash before a round's end reset.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn stall_nth_reset(&self, n: u32) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.reset_stall
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push((n, Arc::clone(&notify)));
        notify
    }

    /// MOD-125 M3 D4: the `n`-th [`prepare`](Isolator::prepare) call (1-based, as
    /// [`prepares`](Self::prepares) counts) logs itself, is counted, signals the returned
    /// [`Notify`] and then **never returns**: a stop before the step's session opened, so its log
    /// holds no row.
    ///
    /// # Panics
    /// When the lock is poisoned, which no case does.
    pub fn stall_nth_prepare(&self, n: u32) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.prepare_stall
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push((n, Arc::clone(&notify)));
        notify
    }

    /// MOD-139 M3 T1: the next [`unmerged`](Isolator::unmerged) answers the steps of the parts
    /// keyed `keys`, in the order the parts were handed (an unknown key is ignored). Unscripted,
    /// it answers nothing. FIFO with [`fail_unmerged`](Self::fail_unmerged).
    pub fn script_unmerged(&self, keys: &[&str]) {
        self.unmerged_answers
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Ok(keys.iter().map(|key| (*key).to_owned()).collect()));
    }

    /// MOD-139 M3 R1 (ruling R-6): the next [`unmerged`](Isolator::unmerged) fails with
    /// `IsolateError::Git(reason)`, a post-check that could not read the trees. The call is still
    /// logged and recorded. FIFO with [`script_unmerged`](Self::script_unmerged).
    pub fn fail_unmerged(&self, reason: &str) {
        self.unmerged_answers
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push_back(Err(IsolateError::Git(reason.to_owned())));
    }

    /// MOD-139 M3 T1: every [`premerge`](Isolator::premerge) call so far, as `(integrate step,
    /// part steps in the order handed)`, refused and stalled ones included.
    #[must_use]
    pub fn premerge_requests(&self) -> Vec<(StepId, Vec<StepId>)> {
        self.premerge_requests
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .clone()
    }

    /// MOD-139 M3 T1: every [`unmerged`](Isolator::unmerged) call so far, as `(integrate step,
    /// part steps in the order handed)`.
    #[must_use]
    pub fn unmerged_requests(&self) -> Vec<(StepId, Vec<StepId>)> {
        self.unmerged_requests
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .clone()
    }

    /// The default pre-merge report, with `conflict` (a part key and its paths) stopping every
    /// repo at that part. Merge commits are `fake:merge:<part>:<n>`, `n` the hash counter's.
    fn premerge_report(
        &self,
        trees: &[RunStepTree],
        parts: &[PartTip],
        conflict: Option<&(String, Vec<String>)>,
    ) -> PremergeReport {
        let repos = trees
            .iter()
            .map(|tree| {
                let mut head = tree.base_ref.clone();
                let mut stopped = false;
                let parts = parts
                    .iter()
                    .map(|part| {
                        // GixIsolator's `committed_tip`: an `after_hash` equal to its
                        // `before_hash` committed nothing.
                        let tip = part
                            .commits
                            .iter()
                            .find(|commit| commit.repo_id == tree.repo_id)
                            .and_then(|commit| {
                                commit
                                    .after_hash
                                    .clone()
                                    .filter(|after| *after != commit.before_hash)
                            });
                        // `premerge_repo`'s order: no commits before a stopped repo.
                        let merge = if tip.is_none() {
                            PartMerge::NoCommits
                        } else if stopped {
                            PartMerge::NotAttempted
                        } else if let Some((_, paths)) =
                            conflict.filter(|(key, _)| *key == part.key)
                        {
                            stopped = true;
                            PartMerge::Conflicted {
                                paths: paths.clone(),
                            }
                        } else {
                            let commit = format!("fake:merge:{}:{}", part.step, self.tick());
                            head.clone_from(&commit);
                            PartMerge::Merged { commit }
                        };
                        PartPremerge {
                            step: part.step,
                            key: part.key.clone(),
                            tip,
                            merge,
                        }
                    })
                    .collect();
                RepoPremerge {
                    repo_id: tree.repo_id,
                    name: tree.repo_id.to_string(),
                    head,
                    parts,
                }
            })
            .collect();
        PremergeReport { repos }
    }

    /// The stall's signal and, for a pause, its resume, when `call` is the stalled one, taking
    /// them (one-shot).
    fn reconcile_stall_at(&self, call: usize) -> Option<(Arc<Notify>, Option<Arc<Notify>>)> {
        let mut stalls = self
            .reconcile_stall
            .lock()
            .expect("no panic holds the fake isolator's lock");
        let at = stalls
            .iter()
            .position(|(n, _, _)| usize::try_from(*n).is_ok_and(|n| n == call))?;
        let (_, notify, resume) = stalls.remove(at);
        Some((notify, resume))
    }

    /// Take `step`'s `shared_serialized` guard, if it holds one; dropping it frees the lock.
    ///
    /// Keyed by the step half alone (blueprint F-I): a step id names one run's step.
    fn release_step(&self, step: StepId) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let mut held = self
            .held
            .lock()
            .expect("no panic holds the fake isolator's lock");
        let key = held
            .keys()
            .find(|(_, held_step)| *held_step == step)
            .copied();
        key.and_then(|key| held.remove(&key))
    }

    /// MOD-138 M3 T3: every call this isolator has seen, in order (see [`IsolatorCall`]).
    #[must_use]
    pub fn call_log(&self) -> Vec<IsolatorCall> {
        self.call_log
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .clone()
    }

    /// MOD-138 M3 T3 (F14): from now on each `reconcile` yields once after it records itself, so
    /// two landings in flight are both inside `reconcile` at once unless the walk serialises them.
    pub fn interleave_reconciles(&self) {
        self.interleave_reconciles.store(true, Ordering::SeqCst);
    }

    /// MOD-138 M3 T3: the most `reconcile` calls that were in progress at once.
    #[must_use]
    pub fn reconcile_high_water(&self) -> u32 {
        self.reconciling
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .1
    }

    /// MOD-138 M3 T3: how many times a step's window (`prepare`..`capture`, or inside
    /// `reconcile`) opened while another step's was open and either of the two was in place
    /// (`local` or `shared_serialized`). A concurrent walk must keep this at `0` (blueprint A3).
    #[must_use]
    pub fn in_place_overlaps(&self) -> u32 {
        *self
            .in_place_overlaps
            .lock()
            .expect("no panic holds the fake isolator's lock")
    }

    fn log(&self, call: IsolatorCall) {
        self.call_log
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .push(call);
    }

    /// Opens `step`'s `window`, counting an overlap when another step's window is open and either
    /// is in place.
    fn open_window(&self, step: StepId, window: Window, in_place: bool) {
        let mut windows = self
            .windows
            .lock()
            .expect("no panic holds the fake isolator's lock");
        if windows
            .iter()
            .any(|(&(other, _), &other_in_place)| other != step && (in_place || other_in_place))
        {
            *self
                .in_place_overlaps
                .lock()
                .expect("no panic holds the fake isolator's lock") += 1;
        }
        windows.insert((step, window), in_place);
    }

    fn close_window(&self, step: StepId, window: Window) {
        self.windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(step, window));
    }

    /// The next synthetic-hash ordinal.
    fn tick(&self) -> u32 {
        let mut calls = self
            .calls
            .lock()
            .expect("no panic holds the fake isolator's lock");
        *calls += 1;
        *calls
    }

    /// One commit row per tree, scripted if a value is queued and synthetic otherwise.
    fn commits(&self, step: StepId, trees: &[RunStepTree]) -> Vec<RunStepCommit> {
        let scripted = self
            .after
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .pop_front();
        trees
            .iter()
            .map(|tree| RunStepCommit {
                run_step_id: step,
                repo_id: tree.repo_id,
                before_hash: tree.base_ref.clone(),
                after_hash: match &scripted {
                    Some(hash) => hash.clone(),
                    None => Some(format!("fake:after:{step}:{}", self.tick())),
                },
            })
            .collect()
    }
}

impl Isolator for FakeIsolator {
    fn prepare<'a>(
        &'a self,
        run: RunId,
        step: StepId,
        scope: &'a [RepoId],
        isolation: Isolation,
        slot: Option<FanoutSlot<'a>>,
    ) -> IsolatorFuture<'a, Prepared> {
        Box::pin(async move {
            self.log(IsolatorCall::Prepare(step));
            let call = {
                let mut prepares = self
                    .prepares
                    .lock()
                    .expect("no panic holds the fake isolator's lock");
                *prepares += 1;
                *prepares
            };
            if let Some(stalled) = stall_at(&self.prepare_stall, call) {
                stalled.notify_one();
                std::future::pending::<()>().await;
            }
            if let Some(reason) = self
                .refusals
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front()
            {
                return Err(IsolateError::Refused(reason));
            }
            if isolation == Isolation::SharedSerialized {
                // The real `prepare` suspends before it takes the lock (its git work is
                // `spawn_blocking`), so every sibling of a `join_all` has gone `running` before the
                // first of them starts its session. Without this await the fake would run each
                // candidate to completion in turn and hide what a sibling queued behind another
                // is charged for.
                tokio::task::yield_now().await;
                // A step prepared again (a resume) must not wait on its own guard.
                drop(self.release_step(step));
                let guard = Arc::clone(&self.serial).lock_owned().await;
                self.held
                    .lock()
                    .expect("no panic holds the fake isolator's lock")
                    .insert((run, step), guard);
            }
            self.open_window(
                step,
                Window::Tree,
                matches!(isolation, Isolation::Local | Isolation::SharedSerialized),
            );
            let root = self
                .tree_root
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .clone();
            // Unset, the bytes are the ones every existing case was written against.
            let synthetic = format!("{FAKE_TREE_ROOT}/{run}/{step}");
            let tree_path = |repo: &RepoId| match &root {
                Some(dir) => dir.join(repo.to_string()).to_string_lossy().into_owned(),
                None => format!("{synthetic}/{repo}"),
            };
            let trees = scope
                .iter()
                .map(|repo| {
                    // D54(a): a candidate starts from its group's base; anything else, and a repo
                    // the base does not name, gets a fresh synthetic one.
                    let base = slot
                        .and_then(|slot| slot.base.get(repo).cloned())
                        .unwrap_or_else(|| format!("fake:base:{}", self.tick()));
                    PreparedTree {
                        tree: RunStepTree {
                            run_step_id: step,
                            repo_id: *repo,
                            mode: isolation,
                            path: tree_path(repo),
                            base_ref: base.clone(),
                            dirty: false,
                        },
                        before_hash: base,
                    }
                })
                .collect();
            Ok(Prepared {
                trees,
                cwd: root.clone().unwrap_or_else(|| PathBuf::from(&synthetic)),
                // The fake puts every tree under one `cwd`, which is the `worktree`/`copy` shape
                // (plan D28), so this is empty unless a case scripted it with
                // [`script_extra_dirs`](FakeIsolator::script_extra_dirs).
                extra_dirs: self
                    .extra_dirs
                    .lock()
                    .expect("no panic holds the fake isolator's lock")
                    .clone(),
            })
        })
    }

    fn capture<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        Box::pin(async move {
            self.log(IsolatorCall::Capture(step));
            self.close_window(step, Window::Tree);
            drop(self.release_step(step));
            let commits = self.commits(step, trees);
            self.captured
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .insert(step, commits.clone());
            Ok(commits)
        })
    }

    /// For a fake that made no tree, a merge is the identity: this reports exactly what
    /// [`capture`](Isolator::capture) reported for the winner. `siblings` move nothing — there is
    /// no shared checkout for a sibling to have left anywhere — and are only recorded, for
    /// [`reconciles`](FakeIsolator::reconciles).
    ///
    /// **It does not consume a [`script_after`](FakeIsolator::script_after) value**, which is the
    /// one behavioural change T6 made to this double. Milestone 2 shipped `reconcile` echoing
    /// `capture`'s own body because nothing called it; now that the engine does, a `reconcile` that
    /// popped the queue would eat criterion 7's *second* scripted hash on the first step's advance
    /// and the loop would stop making sense. A step `capture` never ran for reports nothing, and
    /// `record_commits` over an empty batch writes nothing.
    fn reconcile<'a>(
        &'a self,
        winner: StepId,
        trees: &'a [RunStepTree],
        siblings: &'a [StepId],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        Box::pin(async move {
            self.log(IsolatorCall::Reconcile(winner));
            let call = {
                let mut reconciles = self
                    .reconciles
                    .lock()
                    .expect("no panic holds the fake isolator's lock");
                reconciles.push((winner, siblings.to_vec()));
                reconciles.len()
            };
            // MOD-138 M3 T3: the landing's window and the in-progress count, closed on every exit.
            {
                let mut reconciling = self
                    .reconciling
                    .lock()
                    .expect("no panic holds the fake isolator's lock");
                reconciling.0 += 1;
                reconciling.1 = reconciling.1.max(reconciling.0);
            }
            self.open_window(
                winner,
                Window::Landing,
                trees.iter().any(|tree| {
                    matches!(tree.mode, Isolation::Local | Isolation::SharedSerialized)
                }),
            );
            let _landing = Reconciling {
                isolator: self,
                winner,
            };
            if self.interleave_reconciles.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
            // Blueprint F-A: the crash between `done` and the merge. Recorded, signalled, and
            // then this call never answers; the case drops the walk.
            if let Some((stalled, resume)) = self.reconcile_stall_at(call) {
                stalled.notify_one();
                match resume {
                    Some(resume) => resume.notified().await,
                    None => std::future::pending::<()>().await,
                }
            }
            if let Some(err) = self
                .reconcile_failures
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front()
            {
                return Err(err);
            }
            Ok(self
                .captured
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .get(&winner)
                .cloned()
                .unwrap_or_default())
        })
    }

    /// MOD-139 M3 T1: logged first (under the integrate step), counted and recorded, then a
    /// [`stall_nth_premerge`](FakeIsolator::stall_nth_premerge) stall, then the next script;
    /// unscripted, the default report: per tree row every part with an `after_hash` there other
    /// than its `before_hash` `Merged` (`fake:merge:<part>:<n>`) and the rest `NoCommits`, `head`
    /// the last merge or the
    /// row's `base_ref`. The fake merges nothing (blueprint G-5).
    fn premerge<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
        parts: &'a [PartTip],
    ) -> IsolatorFuture<'a, PremergeReport> {
        Box::pin(async move {
            self.log(IsolatorCall::Premerge(step));
            let call = {
                let mut premerges = self
                    .premerges
                    .lock()
                    .expect("no panic holds the fake isolator's lock");
                *premerges += 1;
                *premerges
            };
            self.premerge_requests
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .push((step, parts.iter().map(|part| part.step).collect()));
            if let Some(stalled) = stall_at(&self.premerge_stall, call) {
                stalled.notify_one();
                std::future::pending::<()>().await;
            }
            let script = self
                .premerge_answers
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front();
            match script {
                Some(PremergeScript::Refuse(reason)) => Err(IsolateError::Refused(reason)),
                Some(PremergeScript::Conflict { key, paths }) => {
                    Ok(self.premerge_report(trees, parts, Some(&(key, paths))))
                }
                None => Ok(self.premerge_report(trees, parts, None)),
            }
        })
    }

    /// MOD-139 M3 T1: logged and recorded, then the next
    /// [`script_unmerged`](FakeIsolator::script_unmerged) keys as the handed parts' steps, in the
    /// order handed; unscripted, nothing.
    fn unmerged<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
        heads: &'a [RunStepCommit],
        parts: &'a [PartTip],
    ) -> IsolatorFuture<'a, Vec<StepId>> {
        Box::pin(async move {
            let _ = (trees, heads);
            self.log(IsolatorCall::Unmerged(step));
            self.unmerged_requests
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .push((step, parts.iter().map(|part| part.step).collect()));
            let keys = self
                .unmerged_answers
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front()
                .unwrap_or_else(|| Ok(Vec::new()))?;
            Ok(parts
                .iter()
                .filter(|part| keys.contains(&part.key))
                .map(|part| part.step)
                .collect())
        })
    }

    /// `fake:group-base:<repo>` per repo: stable, and **not** drawn from the hash counter, whose
    /// `fake:base:<n>` ordinals milestone 2's cases pin (blueprint H-13).
    fn base<'a>(&'a self, scope: &'a [RepoId]) -> IsolatorFuture<'a, BTreeMap<RepoId, String>> {
        Box::pin(async move {
            Ok(scope
                .iter()
                .map(|repo| (*repo, format!("fake:group-base:{repo}")))
                .collect())
        })
    }

    /// The next [`script_diff`](FakeIsolator::script_diff) or
    /// [`fail_diff`](FakeIsolator::fail_diff) answer, or `None` unscripted; the rows are recorded
    /// for [`diff_requests`](FakeIsolator::diff_requests) either way.
    fn diff<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Option<DiffBlock>> {
        Box::pin(async move {
            self.diff_requests
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .push(DiffRequest {
                    trees: trees.iter().map(|row| row.run_step_id).collect(),
                    commits: commits.iter().map(|row| row.run_step_id).collect(),
                });
            self.diffs
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front()
                .unwrap_or(Ok(None))
        })
    }

    /// The next [`script_changed_paths`](FakeIsolator::script_changed_paths) or
    /// [`fail_changed_paths`](FakeIsolator::fail_changed_paths) answer, or the empty set
    /// unscripted; the rows are recorded for
    /// [`changed_path_requests`](FakeIsolator::changed_path_requests) either way (MOD-9 D135).
    fn changed_paths<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, ChangedPaths> {
        Box::pin(async move {
            self.changed_path_requests
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .push(DiffRequest {
                    trees: trees.iter().map(|row| row.run_step_id).collect(),
                    commits: commits.iter().map(|row| row.run_step_id).collect(),
                });
            self.changed
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front()
                .unwrap_or_else(|| Ok(ChangedPaths::default()))
        })
    }

    /// The next [`script_env_files`](FakeIsolator::script_env_files) or
    /// [`fail_env_files`](FakeIsolator::fail_env_files) answer, or the empty set unscripted; the
    /// roots are recorded for [`env_file_requests`](FakeIsolator::env_file_requests) either way.
    /// Touches no filesystem: the paths are synthetic.
    fn env_files<'a>(
        &'a self,
        cwd: &'a Path,
        extra_dirs: &'a [PathBuf],
    ) -> IsolatorFuture<'a, EnvFiles> {
        Box::pin(async move {
            self.env_file_requests
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .push((cwd.to_path_buf(), extra_dirs.to_vec()));
            let cancel = self
                .cancel_at_env_files
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .take();
            if let Some((signal, grace)) = cancel {
                signal.send_replace(Signal::Cancel { grace });
            }
            self.env_file_answers
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front()
                .unwrap_or_else(|| Ok(EnvFiles::default()))
        })
    }

    /// Nothing was created, so nothing is removed — but the call is still made by the engine and is
    /// still counted, so a case can assert that cleanup happened exactly once, at run end.
    ///
    /// It no longer ticks the **hash** counter (blueprint F-K): a counter two verbs share cannot
    /// answer "how many cleanups", and the synthetic hashes are the other one's job.
    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            let _ = trees;
            self.log(IsolatorCall::Cleanup(run));
            self.windows
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .clear();
            // One run per fake in every case, so the run's guards are all of them.
            self.held
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .clear();
            *self
                .cleanups
                .lock()
                .expect("no panic holds the fake isolator's lock") += 1;
            Ok::<(), IsolateError>(())
        })
    }

    /// Nothing was created, so nothing is put back: an empty [`ResetReport`] unless a case
    /// scripted a refusal with [`script_reset_refusal`](FakeIsolator::script_reset_refusal).
    fn reset<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, ResetReport> {
        Box::pin(async move {
            self.log(IsolatorCall::Reset(step));
            self.close_window(step, Window::Tree);
            let call = {
                let mut resets = self
                    .resets
                    .lock()
                    .expect("no panic holds the fake isolator's lock");
                *resets += 1;
                *resets
            };
            // MOD-139 M3 T1: a crash before the reset answers (the round's end reset, say).
            if let Some(stalled) = stall_at(&self.reset_stall, call) {
                stalled.notify_one();
                std::future::pending::<()>().await;
            }
            let refusal = self
                .reset_refusals
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front();
            Ok(match (refusal, trees.first()) {
                (Some(reason), Some(first)) => ResetReport {
                    labelled: Vec::new(),
                    refused: vec![(first.repo_id, reason)],
                },
                _ => ResetReport::default(),
            })
        })
    }

    /// D99: every `shared_serialized` guard a step of `run` holds, dropped; counted.
    fn release<'a>(&'a self, run: RunId) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            self.log(IsolatorCall::Release(run));
            self.windows
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .clear();
            *self
                .releases
                .lock()
                .expect("no panic holds the fake isolator's lock") += 1;
            let released = {
                let mut held = self
                    .held
                    .lock()
                    .expect("no panic holds the fake isolator's lock");
                let (released, kept) = std::mem::take(&mut *held)
                    .into_iter()
                    .partition::<BTreeMap<_, _>, _>(|((held_run, _), _)| *held_run == run);
                *held = kept;
                released
            };
            // Dropped outside the `std` lock: dropping a guard wakes a waiter.
            drop(released);
            Ok(())
        })
    }

    /// MOD-125 M3 D5a: logged and counted; a scripted refusal answers with the step holding
    /// nothing. Otherwise a `shared_serialized` row retakes [`serial`](FakeIsolator::serial) as
    /// `prepare` does (its own guard dropped first, so a second call never waits on itself), held
    /// to the step's `capture`, and the step's tree window reopens. Nothing is minted or reset.
    fn reacquire<'a>(
        &'a self,
        run: RunId,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            self.log(IsolatorCall::Reacquire(step));
            *self
                .reacquires
                .lock()
                .expect("no panic holds the fake isolator's lock") += 1;
            let refusal = self
                .reacquire_refusals
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .pop_front();
            if let Some(reason) = refusal {
                drop(self.release_step(step));
                return Err(IsolateError::Refused(reason));
            }
            if trees
                .iter()
                .any(|tree| tree.mode == Isolation::SharedSerialized)
            {
                drop(self.release_step(step));
                let guard = Arc::clone(&self.serial).lock_owned().await;
                self.held
                    .lock()
                    .expect("no panic holds the fake isolator's lock")
                    .insert((run, step), guard);
            }
            self.open_window(
                step,
                Window::Tree,
                trees.iter().any(|tree| {
                    matches!(tree.mode, Isolation::Local | Isolation::SharedSerialized)
                }),
            );
            Ok(())
        })
    }
}

/// A [`Verifier`] that spawns nothing and answers what a case queued (plan D41).
///
/// The three outcomes of `docs/ANA-2.md:508-515` are what stage 5 has to be able to be shown, and
/// the only way to show them with a real [`ShellVerifier`](crate::verify::ShellVerifier) is to run
/// a process — which the suite's determinism rules forbid. So a case queues a report and the walk
/// consumes it.
///
/// **An unscripted call answers `None`**, which is not one of the three outcomes: it is "the phase
/// named no `verify_command`", the seeded shape of every phase (`crates/htui-core/src/seed.rs`)
/// and the reason fifteen of the seventeen cases never touch this double. A `None` report writes
/// no `command_run` row and leaves both columns `NULL`, which is exactly what `unavailable` is
/// **not** (T3's finding; `crate::verify`'s module doc).
#[derive(Debug, Default)]
pub struct FakeVerifier {
    /// Scripted reports, FIFO, one consumed per [`run`](Verifier::run) call.
    reports: Mutex<VecDeque<VerifyReport>>,
    /// How many times [`run`](Verifier::run) has been called, unscripted calls included.
    runs: Mutex<u32>,
    /// Every request's step and deadline remainder, in call order.
    remaining: Mutex<Vec<(StepId, Option<std::time::Duration>)>>,
}

impl FakeVerifier {
    /// A fresh verifier: nothing scripted, counter at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue the report the next [`run`](Verifier::run) answers.
    ///
    /// **The blueprint's §7.1 signature takes `Option<VerifyReport>`** and its §7.6 case bodies
    /// pass a bare `VerifyReport`; the two cannot both be right. The bare report wins, because
    /// scripting `None` is indistinguishable from scripting nothing — an unscripted call already
    /// answers `None` — so the `Option` would be a parameter with one reachable meaning.
    pub fn script_report(&self, report: VerifyReport) {
        self.reports
            .lock()
            .expect("no panic holds the fake verifier's lock")
            .push_back(report);
    }

    /// How many times the walk asked this verifier for a report.
    #[must_use]
    pub fn runs(&self) -> u32 {
        *self
            .runs
            .lock()
            .expect("no panic holds the fake verifier's lock")
    }

    /// What was left of each asked step's deadline when its verify started (plan D30), as
    /// `(step, remaining)` in call order: the budget the real verifier would have run under.
    #[must_use]
    pub fn remaining(&self) -> Vec<(StepId, Option<std::time::Duration>)> {
        self.remaining
            .lock()
            .expect("no panic holds the fake verifier's lock")
            .clone()
    }

    /// A `pass` at exit `0`, stamped at the fake driver's own origin so a report and a session row
    /// share one timeline (the [`TestClock`] convention).
    #[must_use]
    pub fn pass() -> VerifyReport {
        Self::report(VerifyOutcome::Pass, Some(0), "ok")
    }

    /// A `fail` at `code`.
    #[must_use]
    pub fn fail(code: i32) -> VerifyReport {
        Self::report(VerifyOutcome::Fail, Some(code), "failed")
    }

    /// An `unavailable` carrying the reason the operator reads out of `command_run.output`.
    #[must_use]
    pub fn unavailable(reason: &str) -> VerifyReport {
        Self::report(VerifyOutcome::Unavailable, None, reason)
    }

    /// The three constructors above, written once.
    fn report(outcome: VerifyOutcome, exit_code: Option<i32>, output: &str) -> VerifyReport {
        VerifyReport {
            outcome,
            exit_code,
            output: output.to_owned(),
            started_at: epoch(),
            finished_at: epoch(),
            truncated: false,
        }
    }
}

impl Verifier for FakeVerifier {
    fn run<'a>(&'a self, request: VerifyRequest) -> VerifierFuture<'a, Option<VerifyReport>> {
        Box::pin(async move {
            self.remaining
                .lock()
                .expect("no panic holds the fake verifier's lock")
                .push((request.step, request.remaining));
            *self
                .runs
                .lock()
                .expect("no panic holds the fake verifier's lock") += 1;
            self.reports
                .lock()
                .expect("no panic holds the fake verifier's lock")
                .pop_front()
        })
    }
}

/// The model the fake's stand-in candidate names (plan D20, blueprint A-1).
///
/// `(AGENT_CLAUDE, "sonnet")` is the pair the fixture's own `RUN_1` steps carry
/// (`crates/htui-core/src/fixtures.rs:1428-1438`), so a snapshot resolved by the fake matches the
/// recorded one. It has to be spelled out because `SnapshotCandidate::model` is **not** nullable
/// while `agent.default_model` is, and the seeded `claude` row's is `null` with an empty `models`
/// list (`crates/htui-core/seeds/agent_claude.json`): `graph::candidates` falls through to
/// `NoCandidate` rather than hand a driver an empty string, so a fake that did not name a model
/// would refuse every phase.
const STAND_IN_MODEL: &str = "sonnet";

/// The seven inherent orchestration reads of `MemStore`, as the trait `graph.rs` defines
/// (blueprint F-N).
///
/// Implemented on `MemStore` itself rather than on `&MemStore`: a trait impl on the reference would
/// make the engine's `G` be `&MemStore` and every call site `&&MemStore`. The engine borrows `&G`,
/// so the source is used "over `&MemStore`" exactly as plan D19 says either way. Local trait,
/// foreign type — the orphan rule permits it, and it lives behind `test-support` because
/// `MemStore` is the fake harness's store. Milestone 6's source over a host is
/// `htui_worker::HostGraphs`, a newtype (MOD-41 T6; it was `htui`'s `run_worker::BackendGraphs`):
/// `impl GraphSource for Backend` outside `htui-store` would be an orphan impl (E0117, MOD-4 plan
/// D155).
///
/// Each body calls the *inherent* method of the same name. Method resolution prefers inherent
/// candidates over trait ones, so this is delegation and not recursion — and
/// [`the trait's own unit test`](self) calls all seven through the trait to prove it.
impl GraphSource for MemStore {
    async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
        self.resolve_graph(item).await
    }

    async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
        self.phase_agents(phase).await
    }

    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>> {
        self.prompt_template(project, name, version).await
    }

    async fn agent(&self, id: AgentId) -> Result<Option<Agent>> {
        Ok(self
            .agents()
            .await?
            .into_iter()
            .map(|summary| summary.agent)
            .find(|agent| agent.id == id))
    }

    async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
        self.agent_boxes(box_id).await
    }

    async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>> {
        self.bound_skills(project, phase).await
    }

    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
        self.missing_tags(item, box_id).await
    }
}

/// A [`GraphSource`] over `MemStore` with a per-phase candidates map (plan D20).
///
/// **Rungs 1 and 3 of ANA-2 §4.1's candidate chain are both empty on the demo fixture**, which is
/// why this type exists. Rung 1 is `phase_agent`: `MemStore` holds the table since MOD-37 R-6, but
/// the fixture seeds no row, and this source answers rung 1 from its own map regardless. Rung 3 is
/// "the single enabled agent on the box" (plan D62), read from `agent_box`, and the demo fixture
/// seeds no `agent_box` row at all, so a resolution finds no enabled agent on the box.
/// Rung 2 — `project.settings.default_agent_id` — stays real in `graph.rs` and is empty on the
/// fixture too. A resolution with no stand-in would therefore refuse every phase with
/// `NoCandidate`, for a reason that is about the fixture and not about the walk.
///
/// The map is keyed by **phase name** because that is what a case knows; the `PhaseId -> name`
/// index is built as a side effect of [`resolve_graph`](GraphSource::resolve_graph), which
/// `graph::resolve` always calls before it asks for any phase's candidates.
#[derive(Debug)]
pub struct FakeGraphSource<'a> {
    store: &'a MemStore,
    /// Explicit per-phase candidates; an empty vec is "this phase has none" (the rung-4 case).
    candidates: BTreeMap<String, Vec<(AgentId, String)>>,
    /// What every phase the map does not name resolves to.
    fallback: Vec<(AgentId, String)>,
    /// Learned from `resolve_graph`, so `phase_agents` can answer by name.
    names: Mutex<BTreeMap<PhaseId, String>>,
    /// Template names whose body answers [`REFUSED_TEMPLATE_BODY`] (MOD-4 plan D162).
    refused: BTreeSet<String>,
    /// Template names this source answers `None` for, as if the project had no such row.
    missing: BTreeSet<String>,
}

impl<'a> FakeGraphSource<'a> {
    /// A source over `store` where every phase resolves to `(AGENT_CLAUDE, "sonnet")`.
    #[must_use]
    pub fn new(store: &'a MemStore) -> Self {
        Self {
            store,
            candidates: BTreeMap::new(),
            fallback: vec![(ids::AGENT_CLAUDE, STAND_IN_MODEL.to_owned())],
            names: Mutex::new(BTreeMap::new()),
            refused: BTreeSet::new(),
            missing: BTreeSet::new(),
        }
    }

    /// Name `phase`'s candidates explicitly, in preference order.
    #[must_use]
    pub fn with_candidates(mut self, phase: &str, agents: Vec<(AgentId, &str)>) -> Self {
        self.candidates.insert(
            phase.to_owned(),
            agents
                .into_iter()
                .map(|(id, model)| (id, model.to_owned()))
                .collect(),
        );
        self
    }

    /// Give `phase` no candidate at all: `graph::resolve` then walks to rung 2, finds the demo
    /// project names no `default_agent_id` either, and refuses with `ResolveError::NoCandidate` —
    /// ANA-2 §4.1's rung 4 (`docs/ANA-2.md:288`).
    #[must_use]
    pub fn without_candidates(mut self, phase: &str) -> Self {
        self.candidates.insert(phase.to_owned(), Vec::new());
        self
    }

    /// The candidates registered for a phase id, by the name `resolve_graph` learned for it.
    ///
    /// A phase id this source has never seen answers the fallback: the alternative is a silent
    /// empty walk, which is the failure mode plan D20 exists to prevent.
    fn registered(&self, phase: PhaseId) -> Vec<(AgentId, String)> {
        let names = self
            .names
            .lock()
            .expect("no panic holds the fake source's lock");
        names
            .get(&phase)
            .and_then(|name| self.candidates.get(name))
            .unwrap_or(&self.fallback)
            .clone()
    }
}

impl GraphSource for FakeGraphSource<'_> {
    async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
        let resolved = GraphSource::resolve_graph(self.store, item).await?;
        if let Some(graph) = &resolved {
            let mut names = self
                .names
                .lock()
                .expect("no panic holds the fake source's lock");
            for row in &graph.phases {
                names.insert(row.phase.id, row.phase.name.clone());
            }
        }
        Ok(resolved)
    }

    async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
        Ok(self
            .registered(phase)
            .into_iter()
            .enumerate()
            .map(|(position, (agent_id, model))| PhaseAgent {
                phase_id: phase,
                position: i32::try_from(position).expect("a handful of candidates"),
                agent_id,
                model,
            })
            .collect())
    }

    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>> {
        if self.missing.contains(name) {
            return Ok(None);
        }
        let template = GraphSource::prompt_template(self.store, project, name, version).await?;
        Ok(template.map(|mut template| {
            if self.refused.contains(name) {
                REFUSED_TEMPLATE_BODY.clone_into(&mut template.body);
            }
            template
        }))
    }

    async fn agent(&self, id: AgentId) -> Result<Option<Agent>> {
        GraphSource::agent(self.store, id).await
    }

    async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
        GraphSource::agent_boxes(self.store, box_id).await
    }

    async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>> {
        GraphSource::bound_skills(self.store, project, phase).await
    }

    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
        GraphSource::missing_tags(self.store, item, box_id).await
    }
}

/// One phase attempt's scripted behaviour: what the driver plays, and what the step "produced".
///
/// The `output` half is plan D13's, and it is the reason the engine writes no document ever: stage
/// 5 requires one and criterion 1 asserts four, but `document_write` is MOD-11's and MOD-11 is
/// blocked on MOD-4 (ANA-2 risk 4, `docs/ANA-2.md:2067`). Putting the stand-in producer in the
/// harness rather than in `engine.rs` keeps that gap honest: nothing in the engine will have to be
/// removed when MOD-11 lands.
#[derive(Debug, Clone)]
pub struct ScriptedStep {
    /// What `FakeDriver` plays for this attempt's session.
    pub script: Script,
    /// The body of the document written after `Done`, or `None` for a step that produced nothing.
    pub output: Option<String>,
    /// When set, no session is opened at all: [`FakeOrchestrator::driver_for`] hands out a driver
    /// whose `start` refuses with `DriverError::Spawn(reason)` (ANA-2 `:639`'s "spawn failure").
    pub spawn_failure: Option<String>,
}

impl ScriptedStep {
    /// The happy path: one turn ending `Done { EndTurn }`, and a document with `body`.
    #[must_use]
    pub fn done_with_output(body: &str) -> Self {
        Self {
            script: Self::ends(StopReason::EndTurn),
            output: Some(body.to_owned()),
            spawn_failure: None,
        }
    }

    /// MOD-42: one turn that asks for `request`'s call and parks on it — `ToolCall`, then
    /// `ParkPermission(request)`, then `Done { EndTurn }` once answered — and a document with
    /// `body`. The `ToolCall` (an `execute`) carries `request.tool_call_id`, or `call-1`.
    #[must_use]
    pub fn parks(request: PermissionRequestEvent, body: &str) -> Self {
        let call = request
            .tool_call_id
            .clone()
            .unwrap_or_else(|| "call-1".to_owned());
        Self::parks_on(
            ToolCallEvent {
                tool_call_id: call,
                title: "run the suite".to_owned(),
                tool_kind: ToolKind::Execute,
                input: serde_json::json!({ "command": "cargo test" }),
                locations: Vec::new(),
            },
            request,
            body,
        )
    }

    /// [`parks`](Self::parks) on a call of the case's own (MOD-11 R1 M2: one of htui's tools):
    /// `call`, then `request`, then `Done { EndTurn }`, and a document with `body`.
    #[must_use]
    pub fn parks_on(call: ToolCallEvent, request: PermissionRequestEvent, body: &str) -> Self {
        Self {
            script: Script::one_turn(vec![
                ScriptEvent::Emit(DriverEvent::ToolCall(call)),
                ScriptEvent::ParkPermission(request),
                ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                })),
            ]),
            output: Some(body.to_owned()),
            spawn_failure: None,
        }
    }

    /// The happy path with a price: one `usage` report of `cost_micros` USD micros, then `Done {
    /// EndTurn }`, and a document with `body`.
    ///
    /// The recorder sums the report into `run_step.usage["cost_micros"]`, which is the figure
    /// `select::run_spend` reads back — so this is how a case gives a run a spend for stage 1's
    /// cap and budget rules (plan D60 rules 2 and 5) to compare against.
    #[must_use]
    pub fn done_costing(body: &str, cost_micros: i64) -> Self {
        Self {
            script: Script::one_turn(vec![
                ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
                    cost_micros: Some(cost_micros),
                    ..UsageEvent::default()
                })),
                ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                })),
            ]),
            output: Some(body.to_owned()),
            spawn_failure: None,
        }
    }

    /// A step whose driver never starts: ANA-2 `:639`'s **spawn failure**.
    ///
    /// `FakeDriver` cannot express this — its `start` refuses only for an empty prompt or a script
    /// it has already played (`crates/htui-agent/src/fake.rs:166-180`), and neither is reachable
    /// from a walk — so the orchestrator hands out a [`RefusingDriver`] instead. The script is
    /// still carried and is simply never pulled, which is what a process that failed to spawn
    /// leaves behind.
    #[must_use]
    pub fn refusing_to_start(reason: &str) -> Self {
        Self {
            script: Self::ends(StopReason::EndTurn),
            output: None,
            spawn_failure: Some(reason.to_owned()),
        }
    }

    /// A judge call's verdict (plan D52): a paragraph, then exactly one fenced `json` block
    /// `{ "winner": <winner>, "reasons": { "<i>": "<reason>" } }`, and nothing after it — the
    /// shape the seeded judge body asks for (`crates/htui-core/src/prompt/defaults.rs:189-190`).
    #[must_use]
    pub fn judge(winner: i32, reasons: &[(i32, &str)]) -> Self {
        let reasons: serde_json::Map<String, serde_json::Value> = reasons
            .iter()
            .map(|(index, reason)| (index.to_string(), serde_json::Value::from(*reason)))
            .collect();
        let verdict = serde_json::json!({ "winner": winner, "reasons": reasons });
        Self::done_with_output(&format!(
            "The candidates were compared on the task, their verification and their diffs.\n\n\
             ```json\n{verdict}\n```"
        ))
    }

    /// A turn that ends cleanly and writes nothing: the `missing_output` path (`:430`).
    #[must_use]
    pub fn done_without_output() -> Self {
        Self {
            script: Self::ends(StopReason::EndTurn),
            output: None,
            spawn_failure: None,
        }
    }

    /// A `review` document whose body opens with ANA-5's three-line front matter.
    ///
    /// The grammar is fixed by `docs/ANA-5.md:1327-1331` and is exactly three lines — `---`,
    /// `verdict: <approve|request-changes>`, `---` — at the very start of the body, with no other
    /// keys. `verdict` is taken as given rather than typed, so a case can script a malformed one
    /// and pin plan D10's rule, which is that **only an exact `request-changes` rejects**: front
    /// matter that is absent, unparseable or carries any other value settles `ok` and the value is
    /// recorded verbatim for the operator. Silence is not a rejection — treating it as one would
    /// spend `R-ORCH-3`'s retry budget on a parse bug.
    #[must_use]
    pub fn review(verdict: &str, body: &str) -> Self {
        Self {
            script: Self::ends(StopReason::EndTurn),
            output: Some(format!("---\nverdict: {verdict}\n---\n{body}")),
            spawn_failure: None,
        }
    }

    /// A turn that ends on a stop reason ANA-2 `:439` settles `failed`: `Refusal`, `MaxTokens` or
    /// `MaxTurnRequests`.
    #[must_use]
    pub fn failing(stop: StopReason) -> Self {
        Self {
            script: Self::ends(stop),
            output: None,
            spawn_failure: None,
        }
    }

    /// A turn that emits `DriverEvent::Error` and then **no** `done`.
    ///
    /// The missing `done` is deliberate: `FakeDriver` answers the pull past the end of a turn with
    /// `DriverError::Transport` (`crates/htui-agent/src/fake.rs:426-428`), so this drives settle's
    /// first rule — a driver result that is `Err`, which §5.3 says a closed stream is — as well as
    /// leaving the error row ANA-2 `:439` names.
    #[must_use]
    pub fn erroring(code: &str, message: &str) -> Self {
        Self {
            script: Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Error(ErrorEvent {
                code: code.to_owned(),
                message: message.to_owned(),
            }))]),
            output: None,
            spawn_failure: None,
        }
    }

    /// One turn whose only event is `done`.
    fn ends(stop: StopReason) -> Script {
        Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: stop,
        }))])
    }
}

/// An [`AgentDriver`] that refuses to open a session at all (ANA-2 `:639`, "spawn failure").
///
/// It exists because `FakeDriver` cannot be made to refuse a *first* `start` from inside a walk,
/// and because `crates/htui-agent` is not this milestone's to edit. Everything but `start` is the
/// fake's: the name and the capability profile, so stage 1's interlock reads the same profile it
/// would have read for a driver that works.
#[derive(Debug)]
pub struct RefusingDriver {
    name: String,
    caps: DriverCaps,
    reason: String,
}

impl AgentDriver for RefusingDriver {
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
        let reason = self.reason.clone();
        Box::pin(async move {
            drop((spec, prompt));
            Err(DriverError::Spawn(reason))
        })
    }
}

/// MOD-11 B-9: the engine tests' [`ToolHost`]. It speaks no MCP and does no I/O (H-20): `open`
/// records the [`ToolScope`], counts the lease live until it drops, and lends a spec whose
/// [`FAKE_TOKEN_ENV`] names the scope ([`scope_for_token`](Self::scope_for_token)); a
/// `Transport::Cli` scope also gets a prompt port (B-21), whose asking end is dropped.
#[derive(Debug, Default)]
pub struct FakeToolHost {
    opened: Mutex<Vec<ToolScope>>,
    live: Arc<AtomicUsize>,
    fail: Mutex<Option<ToolHostError>>,
    closed: AtomicBool,
    portless: AtomicBool,
    advertised: Mutex<Vec<String>>,
}

impl FakeToolHost {
    /// Every scope `open` was asked for, in order (refused ones excluded).
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn opened(&self) -> Vec<ToolScope> {
        self.opened
            .lock()
            .expect("no panic holds the fake tool host's lock")
            .clone()
    }

    /// Leases opened and not yet dropped.
    #[must_use]
    pub fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    /// Every `open` answers `err` from now on.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn failing(self, err: ToolHostError) -> Self {
        *self
            .fail
            .lock()
            .expect("no panic holds the fake tool host's lock") = Some(err);
        self
    }

    /// The scope a lease's token names, as `McpHost` resolves a handshake.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn scope_for_token(&self, token: &str) -> Option<ToolScope> {
        let index: usize = token.strip_prefix("fake-token-")?.parse().ok()?;
        self.opened
            .lock()
            .expect("no panic holds the fake tool host's lock")
            .get(index)
            .cloned()
    }

    /// MOD-11 T9: every lease from now on comes back without a prompt port, even for a
    /// `Transport::Cli` scope — the host the engine's start-time guard refuses.
    #[must_use]
    pub fn without_prompt_ports(self) -> Self {
        self.portless.store(true, Ordering::SeqCst);
        self
    }

    /// MOD-11 R1 M2: every lease from now on names `tools` as advertised (none by default, so
    /// a case that does not ask sees no pre-approval). The fake reads no scope for it.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn advertising(self, tools: &[&str]) -> Self {
        *self
            .advertised
            .lock()
            .expect("no panic holds the fake tool host's lock") =
            tools.iter().map(|tool| (*tool).to_owned()).collect();
        self
    }

    /// Whether [`ToolHost::close`] was called (B-19).
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl ToolHost for FakeToolHost {
    fn open(&self, scope: ToolScope) -> std::result::Result<ToolLease, ToolHostError> {
        if let Some(err) = self
            .fail
            .lock()
            .expect("no panic holds the fake tool host's lock")
            .clone()
        {
            return Err(err);
        }
        let prompt = (scope.transport == Transport::Cli && !self.portless.load(Ordering::SeqCst))
            .then(|| htui_agent::prompt_bridge().0);
        let token = {
            let mut opened = self
                .opened
                .lock()
                .expect("no panic holds the fake tool host's lock");
            opened.push(scope);
            format!("fake-token-{}", opened.len() - 1)
        };
        self.live.fetch_add(1, Ordering::SeqCst);
        let live = Arc::clone(&self.live);
        let spec = McpServerSpec {
            name: "htui".to_owned(),
            command: "/fake/htui".to_owned(),
            args: vec!["mcp".to_owned()],
            env: BTreeMap::from([
                (FAKE_ADDR_ENV.to_owned(), "/fake/htui-mcp/s".to_owned()),
                (FAKE_TOKEN_ENV.to_owned(), token),
            ]),
        };
        let advertised = self
            .advertised
            .lock()
            .expect("no panic holds the fake tool host's lock")
            .clone();
        Ok(ToolLease::new(spec, prompt, move || {
            live.fetch_sub(1, Ordering::SeqCst);
        })
        .with_tools(advertised))
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// MOD-11 B-9: a session that writes its output document **through its scope's fence** when it
/// starts — what the agent's `document_write` call does — then plays its script.
///
/// The scope is the one the lease's token names on the [`FakeToolHost`]; a spec with no server
/// (no host) writes nothing and records a refusal. With `steal`, the run's lease is first lapsed and
/// taken by a stranger, so the write is fenced (I-3).
#[derive(Debug)]
struct ToolWriting {
    inner: FakeDriver,
    store: MemStore,
    clock: TestClock,
    host: Option<Arc<FakeToolHost>>,
    body: String,
    steal: Option<Uuid>,
    outcomes: Arc<Mutex<Vec<ToolWrite>>>,
}

impl ToolWriting {
    /// The write `document_write` would make for `spec`'s session.
    async fn write(&self, spec: &SessionSpec) -> ToolWrite {
        let scope = spec
            .mcp
            .first()
            .and_then(|server| server.env.get(FAKE_TOKEN_ENV))
            .zip(self.host.as_ref())
            .and_then(|(token, host)| host.scope_for_token(token))
            .ok_or_else(|| StoreError::Constraint("no htui server on this session".to_owned()))?;
        let (Some(item), Some(kind)) = (scope.item_id, scope.output_kind.clone()) else {
            return Err(StoreError::Constraint(
                "document_write is not advertised to this session".to_owned(),
            ));
        };
        if let Some(owner) = self.steal {
            let ttl = crate::recover::LeaseTimes::from_app(&BTreeMap::new()).ttl;
            self.store
                .refresh_lease(scope.run_id, owner, TimeDelta::zero())
                .await?;
            self.store
                .take_lease(scope.run_id, scope.box_id, Uuid::now_v7(), ttl)
                .await?;
        }
        self.store
            .write_step_document(
                scope.fence,
                NewDocument {
                    id: DocumentId::new(),
                    item_id: item,
                    kind: kind.clone(),
                    title: kind,
                    body: self.body.clone(),
                    produced_by_step_id: Some(scope.step_id),
                    created_by: scope.user,
                    created_at: self.clock.now(),
                },
            )
            .await
    }
}

impl AgentDriver for ToolWriting {
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
        Box::pin(async move {
            let outcome = self.write(&spec).await;
            self.outcomes
                .lock()
                .expect("no panic holds the fake orchestrator's lock")
                .push(outcome);
            self.inner.start(spec, prompt).await
        })
    }
}

/// How far [`FakeOrchestrator::restarted`] moves the clock past the crashed process's: ten
/// minutes, above every seeded (`lease_ttl_seconds = 120`) and fallback TTL, so every lease the
/// first process wrote has expired when the second one looks (plan D118).
pub const RESTART_GAP: TimeDelta = TimeDelta::minutes(10);

/// What [`FakeOrchestrator`] scripts by: `(phase, attempt)` and, for one session of a group,
/// `(fanout_index, call)` (plan D68).
type ScriptKey = (String, i32, Option<(i32, u32)>);

/// A [`SessionKey`] owned: `(phase, attempt, fanout_index, call)` (MOD-11 D9's per-key handles).
type OwnedKey = (String, i32, i32, u32);

/// `key`, owned.
fn owned(key: &SessionKey<'_>) -> OwnedKey {
    (
        key.phase.to_owned(),
        key.attempt,
        key.fanout_index,
        key.call,
    )
}

/// What [`FakeOrchestrator::writes_through_tools`] recorded: one `Ok` per landed write, the
/// store's refusal otherwise.
pub type ToolWrite = std::result::Result<Document, StoreError>;

/// The env key the [`FakeToolHost`]'s lease carries its token under, as `htui_mcp`'s does.
pub const FAKE_TOKEN_ENV: &str = "HTUI_MCP_TOKEN";

/// The env key the [`FakeToolHost`]'s lease carries its address under, as `htui_mcp`'s does.
pub const FAKE_ADDR_ENV: &str = "HTUI_MCP_ADDR";

/// One session's stall (blueprint A-2; MOD-40 T2): whether the document is written first, the
/// signal raised when the session stalls, and, for a suspend, the signal that wakes it.
type Stall = (bool, Arc<Notify>, Option<Arc<Notify>>);

/// MOD-42: the engines' policy and control lookups (plan D9, D10), owned so `fake_parts` can lend
/// them for `'a`.
struct Relays {
    signal: watch::Sender<Signal>,
    policies: Arc<Mutex<BTreeMap<AgentId, PermissionPolicy>>>,
    policy: Box<dyn Fn(AgentId) -> PermissionPolicy + Send + Sync>,
    control: Box<dyn Fn(RunId) -> Control + Send + Sync>,
}

impl Relays {
    /// One process's lookups over `policies`, with no signal sent.
    fn new(policies: Arc<Mutex<BTreeMap<AgentId, PermissionPolicy>>>) -> Self {
        let (signal, receiver) = watch::channel(Signal::Run);
        let read = Arc::clone(&policies);
        Self {
            signal,
            policies,
            policy: Box::new(move |agent| {
                read.lock()
                    .expect("no panic holds the fake orchestrator's lock")
                    .get(&agent)
                    .cloned()
                    .unwrap_or_default()
            }),
            control: Box::new(move |_run| Control::new(receiver.clone())),
        }
    }
}

impl core::fmt::Debug for Relays {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Relays")
            .field("signal", &*self.signal.borrow())
            .field("policies", &self.policies)
            .finish_non_exhaustive()
    }
}

/// `MemStore` + `FakeDriver` + [`FakeIsolator`] + [`TestClock`] + [`FakeGraphSource`], which is
/// ANA-2's own description of the fake orchestrator (`docs/ANA-2.md:1764-1766`).
///
/// The store handle reads [`clock`](Self::clock) (MOD-40 plan D11), so a lease the walk takes
/// expires by the clock the case moves.
///
/// **`dispatch` is T4's.** Everything the walk needs from the harness is here and is public —
/// [`graphs`](Self::graphs), [`driver_for`](Self::driver_for), [`after_done`](Self::after_done),
/// [`box_id`](Self::box_id), [`user`](Self::user), [`owner`](Self::owner) — so T4 builds an
/// `Engine` over borrowed parts and forwards to it, rather than re-deciding any of this.
#[derive(Debug)]
pub struct FakeOrchestrator {
    /// The store every case asserts against. Fresh per case, so `claim_run`'s per-box concurrency
    /// cap (`max_concurrent_items`, 2 on the fixture box) never leaks between them (blueprint
    /// H-15).
    pub store: MemStore,
    /// The isolator, so a case can script `after_hash` values.
    pub isolator: FakeIsolator,
    /// The verifier, so a case can script stage 5's outcome without a process (plan D41).
    pub verifier: FakeVerifier,
    /// The clock, so a case can elapse a step deadline without sleeping.
    pub clock: TestClock,
    /// Plan D140: this process's dead walks, borrowed by every engine built over it. A
    /// [`restarted`](Self::restarted) process starts with an empty set.
    pub dead_walks: DeadWalks,
    /// Keyed `(phase, attempt, None)` for a `(phase, attempt)` script and
    /// `(phase, attempt, Some((fanout_index, call)))` for one session's (plan D68).
    scripts: Mutex<BTreeMap<ScriptKey, ScriptedStep>>,
    candidates: Mutex<BTreeMap<String, Vec<(AgentId, String)>>>,
    after_done_advance: Mutex<Option<TimeDelta>>,
    /// Blueprint A-2, F-O: the sessions whose `after_done` never returns, keyed like `scripts`,
    /// each with whether the document is written first and the signal raised, and, for a suspend
    /// ([`suspend_after_done`](Self::suspend_after_done)), the signal that wakes it. Never carried
    /// by [`restarted`](Self::restarted).
    stalls: Mutex<BTreeMap<ScriptKey, Stall>>,
    /// MOD-125 M3 D9: the start failures the next driver of a key answers first
    /// ([`refuse_resume`](Self::refuse_resume)), keyed like `scripts` and taken by that driver.
    /// Never carried by [`restarted`](Self::restarted).
    resume_refusals: Mutex<BTreeMap<ScriptKey, Vec<DriverError>>>,
    /// MOD-4 plan D162: the phases whose pinned template stage 3 refuses. Carried by
    /// [`restarted`](Self::restarted): it is the graph's, not the process's.
    refused_prompts: Mutex<BTreeSet<String>>,
    /// MOD-37 review M-1: the template names the graph source answers `None` for. Carried by
    /// [`restarted`](Self::restarted), like `refused_prompts`.
    missing_templates: Mutex<BTreeSet<String>>,
    /// MOD-41 plan D12: who walks a command's tail in the engines built over this harness.
    /// [`Tails::Walk`] in [`demo`](Self::demo) and in [`restarted`](Self::restarted).
    tails: Mutex<Tails>,
    default_script: ScriptedStep,
    caps: DriverCaps,
    box_id: BoxId,
    user: UserId,
    owner: Uuid,
    /// MOD-42 plan D9, D10: the policy and control lookups every engine over this harness
    /// borrows. A [`restarted`](Self::restarted) process carries the policies (they are the
    /// agents') and none of the signal (it is the process's).
    relays: Relays,
    /// MOD-11 D4, B-9: the tool host every engine over this harness opens leases on, or none.
    /// Not carried by [`restarted`](Self::restarted): a listener is the process's.
    tools: Option<Arc<FakeToolHost>>,
    /// MOD-11 D9: each session's spec handle, taken before its driver is boxed.
    specs: Mutex<BTreeMap<OwnedKey, SpecSlot>>,
    /// MOD-11 B-9: the sessions that write their document through the tool, by exact key.
    tool_bodies: Mutex<BTreeMap<OwnedKey, String>>,
    /// Whether a tool write first lets a stranger take the run's lease.
    steal_before_tool_writes: AtomicBool,
    /// Every tool write's outcome, in order.
    tool_writes: Arc<Mutex<Vec<ToolWrite>>>,
    /// MOD-10 D15: the secret source every `*_fake` engine's `RunSecrets` resolves through, or
    /// none (a provider project is then refused). Carried by [`restarted`](Self::restarted): a
    /// source is the machine's, like the keyring.
    secret_source: Mutex<Option<Arc<dyn SecretSource>>>,
    /// MOD-138 M3 T3 (F14): every session's gauge, the holds and the interleaving switch. A
    /// [`restarted`](Self::restarted) process starts with a fresh one.
    sessions: Arc<Sessions>,
}

/// MOD-138 M3 T3 (F14): what [`FakeOrchestrator`] counts of its sessions. Counting adds no
/// await, so no case that does not ask for a hold or the interleaving changes.
#[derive(Debug, Default)]
struct Sessions {
    /// Sessions started and not yet dropped.
    live: AtomicUsize,
    /// The most that were ever live at once.
    high_water: AtomicUsize,
    /// Whether each session yields once before its first event.
    interleave: AtomicBool,
    /// The holds a case asked for, each matching `(phase, attempt)` sessions.
    holds: Mutex<Vec<Arc<HoldState>>>,
}

/// One [`FakeOrchestrator::hold_sessions`]: the keys it holds, how many held sessions are live
/// at their hold, and whether it was released.
#[derive(Debug)]
struct HoldState {
    keys: Vec<(String, i32)>,
    live: watch::Sender<usize>,
    released: watch::Sender<bool>,
    /// How many held sessions were cancelled after reaching their hold (a cut reached them).
    cancels: watch::Sender<usize>,
    /// [`SessionHold::hold_cancels`]: a held session's cancel waits for the release.
    hold_cancels: AtomicBool,
}

/// MOD-138 M3 T3 (F14): sessions held open before their first event, so a case can see them live
/// together. A cancel releases a held session, as a cut must. A serial walk never has two live,
/// so a case awaiting [`live`](Self::live) runs under `tokio::time::timeout`.
#[derive(Debug, Clone)]
pub struct SessionHold(Arc<HoldState>);

impl SessionHold {
    /// Returns once `n` held sessions have reached their hold.
    pub async fn live(&self, n: usize) {
        let mut seen = self.0.live.subscribe();
        // A closed channel cannot happen: `self` holds the sender.
        let _ = seen.wait_for(|live| *live >= n).await;
    }

    /// How many held sessions have reached their hold so far.
    #[must_use]
    pub fn reached(&self) -> usize {
        *self.0.live.borrow()
    }

    /// Lets every held session, now and later, play its script, and lets a held cancel through.
    pub fn release(&self) {
        self.0.released.send_replace(true);
    }

    /// From now on the cancel of a session that reached its hold (a cut) waits for
    /// [`release`](Self::release) before it drains, so a case can look at the walk while a cut
    /// sibling is still in flight (T3 repair 1).
    pub fn hold_cancels(&self) {
        self.0.hold_cancels.store(true, Ordering::SeqCst);
    }

    /// Returns once `n` sessions that reached their hold were cancelled.
    pub async fn cancelled(&self, n: usize) {
        let mut seen = self.0.cancels.subscribe();
        // A closed channel cannot happen: `self` holds the sender.
        let _ = seen.wait_for(|cancels| *cancels >= n).await;
    }
}

/// The driver [`FakeOrchestrator::driver_for_key`] hands out: the scripted one, its sessions
/// counted (MOD-138 M3 T3), the shape of [`ToolWriting`].
#[derive(Debug)]
struct Counted {
    inner: Box<dyn AgentDriver>,
    sessions: Arc<Sessions>,
    hold: Option<Arc<HoldState>>,
}

impl AgentDriver for Counted {
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
        Box::pin(async move {
            let inner = self.inner.start(spec, prompt).await?;
            let live = self.sessions.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.sessions.high_water.fetch_max(live, Ordering::SeqCst);
            Ok(Box::new(CountedSession {
                inner,
                sessions: Arc::clone(&self.sessions),
                hold: self.hold.clone(),
                first: true,
                arrived: false,
                cancelled: false,
            }) as Box<dyn AgentSession>)
        })
    }
}

/// A live session of [`Counted`]: counted from its start to its drop.
#[derive(Debug)]
struct CountedSession {
    inner: Box<dyn AgentSession>,
    sessions: Arc<Sessions>,
    hold: Option<Arc<HoldState>>,
    /// Before the first event: the interleaving yield and the hold still apply.
    first: bool,
    /// The hold counted this session live.
    arrived: bool,
    /// `cancel` was called: no hold applies any more.
    cancelled: bool,
}

impl Drop for CountedSession {
    fn drop(&mut self) {
        self.sessions.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AgentSession for CountedSession {
    fn session_ref(&self) -> Option<&htui_agent::driver::AgentSessionRef> {
        self.inner.session_ref()
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<htui_agent::event::DriverEnvelope>> {
        Box::pin(async move {
            if self.first {
                if self.sessions.interleave.load(Ordering::SeqCst) {
                    self.first = false;
                    tokio::task::yield_now().await;
                }
                if let Some(hold) = self.hold.clone()
                    && !self.cancelled
                {
                    if !self.arrived {
                        self.arrived = true;
                        hold.live.send_modify(|live| *live += 1);
                    }
                    let mut released = hold.released.subscribe();
                    // Dropped here by a cut (`drive` selects on its control): the next pull is
                    // the cancel's drain, which `cancelled` lets through.
                    let _ = released.wait_for(|released| *released).await;
                }
                self.first = false;
            }
            self.inner.next_event().await
        })
    }

    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
        self.inner.send_follow_up(text)
    }

    fn answer_permission<'a>(
        &'a mut self,
        request_id: htui_agent::driver::PermissionRequestId,
        answer: htui_agent::driver::PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        self.inner.answer_permission(request_id, answer)
    }

    fn cancel<'a>(&'a mut self, grace: std::time::Duration) -> DriverFuture<'a, ()> {
        self.cancelled = true;
        Box::pin(async move {
            if let Some(hold) = self.hold.clone()
                && self.arrived
            {
                hold.cancels.send_modify(|cancels| *cancels += 1);
                if hold.hold_cancels.load(Ordering::SeqCst) {
                    let mut released = hold.released.subscribe();
                    let _ = released.wait_for(|released| *released).await;
                }
            }
            self.inner.cancel(grace).await
        })
    }
}

impl FakeOrchestrator {
    /// The demo fixture, full driver capabilities, and a happy-path default script.
    ///
    /// The box is `ids::BOX` because `demo_data()` sets `this_box: Some(ids::BOX)`
    /// (`crates/htui-core/src/fixtures.rs:375`) and the user is `MemStore::this_user()`, which the
    /// one seeded `app_user` makes unambiguous.
    ///
    /// # Panics
    /// When the demo fixture seeds no `app_user`, which it always does.
    #[must_use]
    pub fn demo() -> Self {
        let clock = TestClock::new();
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        let user = store
            .this_user()
            .expect("the demo fixture seeds exactly one `app_user`");
        Self {
            store,
            isolator: FakeIsolator::new(),
            verifier: FakeVerifier::new(),
            clock,
            dead_walks: DeadWalks::new(),
            scripts: Mutex::new(BTreeMap::new()),
            candidates: Mutex::new(BTreeMap::new()),
            after_done_advance: Mutex::new(None),
            stalls: Mutex::new(BTreeMap::new()),
            resume_refusals: Mutex::new(BTreeMap::new()),
            refused_prompts: Mutex::new(BTreeSet::new()),
            missing_templates: Mutex::new(BTreeSet::new()),
            tails: Mutex::new(Tails::Walk),
            default_script: ScriptedStep::done_with_output("scripted output"),
            caps: FakeDriver::full_caps(),
            box_id: ids::BOX,
            user,
            owner: Uuid::now_v7(),
            relays: Relays::new(Arc::new(Mutex::new(BTreeMap::new()))),
            tools: None,
            specs: Mutex::new(BTreeMap::new()),
            tool_bodies: Mutex::new(BTreeMap::new()),
            steal_before_tool_writes: AtomicBool::new(false),
            tool_writes: Arc::new(Mutex::new(Vec::new())),
            secret_source: Mutex::new(None),
            sessions: Arc::new(Sessions::default()),
        }
    }

    /// A second orchestrator over the **same** store, as a process started after this one died
    /// (plan D118).
    ///
    /// The `MemStore` clone shares its rows (`Arc<RwLock<State>>`). The isolator and the verifier
    /// are fresh, because neither survives a process. The clock starts [`RESTART_GAP`] past this
    /// one's, the scripts, candidates, default script and capabilities are cloned, the box and user
    /// are the same, and the `owner` is **new** — which is what makes the lease this process wrote
    /// someone else's. The dead-walk set is new and empty too (plan D140). An
    /// `advance_after_done` is not carried, and neither is a
    /// [`stall_after_done`](Self::stall_after_done): the crash was the first process's.
    ///
    /// The store is a new handle on the same rows that reads the new clock, so this process judges
    /// every lease by its own clock, as a process with its own database session would (MOD-40
    /// blueprint B23).
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn restarted(&self) -> Self {
        let scripts = self
            .scripts
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .clone();
        let candidates = self
            .candidates
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .clone();
        let clock = TestClock::at(self.clock.now() + RESTART_GAP);
        Self {
            store: self.store.clone().with_clock(Arc::new(clock.clone())),
            isolator: FakeIsolator::new(),
            verifier: FakeVerifier::new(),
            clock,
            dead_walks: DeadWalks::new(),
            scripts: Mutex::new(scripts),
            candidates: Mutex::new(candidates),
            after_done_advance: Mutex::new(None),
            stalls: Mutex::new(BTreeMap::new()),
            resume_refusals: Mutex::new(BTreeMap::new()),
            refused_prompts: Mutex::new(
                self.refused_prompts
                    .lock()
                    .expect("no panic holds the fake orchestrator's lock")
                    .clone(),
            ),
            missing_templates: Mutex::new(
                self.missing_templates
                    .lock()
                    .expect("no panic holds the fake orchestrator's lock")
                    .clone(),
            ),
            tails: Mutex::new(Tails::Walk),
            default_script: self.default_script.clone(),
            caps: self.caps,
            box_id: self.box_id,
            user: self.user,
            owner: Uuid::now_v7(),
            relays: Relays::new(Arc::clone(&self.relays.policies)),
            tools: None,
            specs: Mutex::new(BTreeMap::new()),
            tool_bodies: Mutex::new(BTreeMap::new()),
            steal_before_tool_writes: AtomicBool::new(false),
            tool_writes: Arc::new(Mutex::new(Vec::new())),
            secret_source: Mutex::new(self.secret_source()),
            sessions: Arc::new(Sessions::default()),
        }
    }

    /// MOD-138 M3 T3 (F14): the most sessions this orchestrator's engines had live at once.
    #[must_use]
    pub fn live_sessions_high_water(&self) -> usize {
        self.sessions.high_water.load(Ordering::SeqCst)
    }

    /// MOD-138 M3 T3 (F14): from now on each session yields once before its first event, so the
    /// walk's admitted node futures interleave rather than each running to completion in turn.
    pub fn interleave_sessions(&self) {
        self.sessions.interleave.store(true, Ordering::SeqCst);
    }

    /// MOD-138 M3 T3 (F14): every session of the listed `(phase, attempt)`s started from now on
    /// waits before its first event until the returned hold is released, or the session is
    /// cancelled.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn hold_sessions(&self, keys: &[(&str, i32)]) -> SessionHold {
        let state = Arc::new(HoldState {
            keys: keys
                .iter()
                .map(|(phase, attempt)| ((*phase).to_owned(), *attempt))
                .collect(),
            live: watch::channel(0).0,
            released: watch::channel(false).0,
            cancels: watch::channel(0).0,
            hold_cancels: AtomicBool::new(false),
        });
        self.sessions
            .holds
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .push(Arc::clone(&state));
        SessionHold(state)
    }

    /// The hold `key`'s session waits on, if a case asked for one.
    fn hold_for(&self, key: &SessionKey<'_>) -> Option<Arc<HoldState>> {
        self.sessions
            .holds
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .iter()
            .find(|hold| {
                hold.keys
                    .iter()
                    .any(|(phase, attempt)| phase == key.phase && *attempt == key.attempt)
            })
            .cloned()
    }

    /// MOD-10 D15: every engine built over this harness from now on resolves a provider
    /// project's secrets through `source` (one fresh `RunSecrets` per `*_fake` call, D14).
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    pub fn set_secret_source(&self, source: Arc<dyn SecretSource>) {
        *self
            .secret_source
            .lock()
            .expect("no panic holds the fake orchestrator's lock") = Some(source);
    }

    /// The secret source `*_fake` hands each walk's `RunSecrets`, or `None`.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn secret_source(&self) -> Option<Arc<dyn SecretSource>> {
        self.secret_source
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .clone()
    }

    /// MOD-11 D4, B-9: every engine built over this harness opens a lease on `tools` per session.
    #[must_use]
    pub fn with_tool_host(mut self, tools: Arc<FakeToolHost>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// The tool host `fake_parts` lends the engine, or `None` (today's specs exactly).
    #[must_use]
    pub fn tool_host(&self) -> Option<Arc<dyn ToolHost>> {
        self.tools
            .as_ref()
            .map(|tools| Arc::clone(tools) as Arc<dyn ToolHost>)
    }

    /// MOD-11 D9: the spec the session at `key` was started with, or `None` before it started.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn spec_for(&self, key: &SessionKey<'_>) -> Option<SessionSpec> {
        self.specs
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .get(&owned(key))
            .and_then(SpecSlot::get)
    }

    /// MOD-11 B-9: the session at `key` writes its output document with `body` through its
    /// scope's fence when it starts, as an agent calling `document_write` would: kind, item, step
    /// and author are the scope's. Script the session `done_without_output` so the harness sink
    /// writes nothing (production's `author: None`).
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    pub fn writes_through_tools(&self, key: &SessionKey<'_>, body: &str) {
        self.tool_bodies
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(owned(key), body.to_owned());
    }

    /// Every tool write lets a stranger take the run's lease first, so it is fenced.
    pub fn take_lease_before_tool_writes(&self) {
        self.steal_before_tool_writes.store(true, Ordering::SeqCst);
    }

    /// Every tool write's outcome so far, in order.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn tool_writes(&self) -> Vec<ToolWrite> {
        self.tool_writes
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .clone()
    }

    /// MOD-42 plan D9: `agent`'s policy in every engine built over this harness.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    pub fn set_policy(&self, agent: AgentId, policy: PermissionPolicy) {
        self.relays
            .policies
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(agent, policy);
    }

    /// MOD-42 plan D10: every walk of this harness is asked to stop gracefully.
    pub fn cancel_walks(&self, grace: std::time::Duration) {
        self.relays.signal.send_replace(Signal::Cancel { grace });
    }

    /// MOD-89 review L5: [`cancel_walks`](Self::cancel_walks), sent by the next env-file scan
    /// while it runs — a cancel that lands mid-scan, after the walk's last check before it.
    pub fn cancel_walks_during_env_files(&self, grace: std::time::Duration) {
        self.isolator
            .cancel_at_env_files(self.relays.signal.clone(), grace);
    }

    /// The policy lookup `fake_parts` lends.
    #[must_use = "the lookup is lent to an engine's parts; dropping it lends nothing"]
    pub fn policy_for(&self) -> PolicyFor<'_> {
        &*self.relays.policy
    }

    /// The control lookup `fake_parts` lends.
    #[must_use = "the lookup is lent to an engine's parts; dropping it lends nothing"]
    pub fn control_for(&self) -> ControlFor<'_> {
        &*self.relays.control
    }

    /// Script one `(phase name, attempt)`: every session of that attempt that has no script of
    /// its own plays it. An unscripted attempt plays the default.
    pub fn script(&self, phase: &str, attempt: i32, step: ScriptedStep) {
        self.scripts
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert((phase.to_owned(), attempt, None), step);
    }

    /// Script one session exactly (plan D68): candidate `fanout_index` of `(phase, attempt)`, or
    /// the judge's ordering `call` at `fanout_index = -1` with `phase` = `<phase>:judge`. It wins
    /// over a [`script`](Self::script) of the same `(phase, attempt)`.
    pub fn script_candidate(
        &self,
        phase: &str,
        attempt: i32,
        fanout_index: i32,
        call: u32,
        step: ScriptedStep,
    ) {
        self.scripts
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(
                (phase.to_owned(), attempt, Some((fanout_index, call))),
                step,
            );
    }

    /// MOD-125 M3 D9: the next driver of `(phase, attempt)` (or of one session's `slot`, as
    /// [`script_candidate`](Self::script_candidate) keys it) answers its first `start` with
    /// `DriverError::Transport(reason)` and plays its script on the next: a resume that fails on
    /// the wire, then the fresh start that falls back. One-shot: the driver takes it.
    ///
    /// # Panics
    /// When the lock is poisoned, which no case does.
    pub fn refuse_resume(&self, phase: &str, attempt: i32, slot: Option<(i32, u32)>, reason: &str) {
        self.resume_refusals
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(
                (phase.to_owned(), attempt, slot),
                vec![DriverError::Transport(reason.to_owned())],
            );
    }

    /// The start failures [`refuse_resume`](Self::refuse_resume) queued for `key`, the exact
    /// session's before its `(phase, attempt)`'s, taken.
    fn take_resume_refusals(&self, key: &SessionKey<'_>) -> Vec<DriverError> {
        let mut refusals = self
            .resume_refusals
            .lock()
            .expect("no panic holds the fake orchestrator's lock");
        refusals
            .remove(&(
                key.phase.to_owned(),
                key.attempt,
                Some((key.fanout_index, key.call)),
            ))
            .or_else(|| refusals.remove(&(key.phase.to_owned(), key.attempt, None)))
            .unwrap_or_default()
    }

    /// Replace the driver capabilities every session is built with.
    ///
    /// The CLI-shaped profile — `permission_requests` and `edit_proposals` both false — is what
    /// stage 1's interlock refuses at a gated phase (`docs/ANA-2.md:476-482`). Note that the
    /// interlock reads `registry::caps_for(&agent)` from the *agent row*, not from the driver
    /// (plan D6), so this knob changes the session and the candidates map changes the refusal.
    #[must_use]
    pub fn with_caps(mut self, caps: DriverCaps) -> Self {
        self.caps = caps;
        self
    }

    /// Name a phase's candidates, overriding the `(AGENT_CLAUDE, "sonnet")` stand-in.
    pub fn with_candidates(&self, phase: &str, agents: Vec<(AgentId, &str)>) {
        self.candidates
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(
                phase.to_owned(),
                agents
                    .into_iter()
                    .map(|(id, model)| (id, model.to_owned()))
                    .collect(),
            );
    }

    /// Give a phase no candidate at all, for the rung-4 refusal.
    pub fn without_candidates(&self, phase: &str) {
        self.with_candidates(phase, Vec::new());
    }

    /// Make stage 3 refuse `phase`'s own prompt (MOD-4 plan D162, ANA-5 criterion 3): the
    /// graph source answers the phase's pinned template with [`REFUSED_TEMPLATE_BODY`], which
    /// `assemble` refuses as an unknown placeholder. Every seeded phase pins the template named
    /// after itself, so the template is found by the phase's name. Its version is unchanged, so
    /// resolution still pins it.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    pub fn refuse_prompt(&self, phase: &str) {
        self.refused_prompts
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(phase.to_owned());
    }

    /// Make the graph source answer `None` for the project's `name` template, as if the row were
    /// gone (MOD-37 review M-1).
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    pub fn drop_template(&self, name: &str) {
        self.missing_templates
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(name.to_owned());
    }

    /// Make one session's [`after_done`](Self::after_done) **never return** — a process killed
    /// between the driver's `done` and the settle (plan D100, blueprint A-2).
    ///
    /// Keyed like [`script_candidate`](Self::script_candidate) (blueprint F-O): `slot = None`
    /// stalls any session of `(phase, attempt)` without a stall of its own, and
    /// `Some((fanout_index, call))` one session exactly — a candidate, or a judge's call at
    /// `fanout_index = -1` under `phase = "<phase>:judge"`. The stall fires after any
    /// [`advance_after_done`](Self::advance_after_done) and, when `write_output`, after the
    /// scripted document is written; it then signals the returned [`Notify`] and awaits forever.
    /// One-shot: a later session with the same key plays normally.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn stall_after_done(
        &self,
        phase: &str,
        attempt: i32,
        slot: Option<(i32, u32)>,
        write_output: bool,
    ) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.stalls
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(
                (phase.to_owned(), attempt, slot),
                (write_output, Arc::clone(&notify), None),
            );
        notify
    }

    /// MOD-40 T2: [`stall_after_done`](Self::stall_after_done) with no document, as a
    /// **suspend** rather than a death — the session signals the first [`Notify`], then awaits
    /// the second and returns `Ok(None)`, and the walk settles on. A laptop lid, not a kill.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn suspend_after_done(
        &self,
        phase: &str,
        attempt: i32,
        slot: Option<(i32, u32)>,
    ) -> (Arc<Notify>, Arc<Notify>) {
        let stalled = Arc::new(Notify::new());
        let wake = Arc::new(Notify::new());
        self.stalls
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .insert(
                (phase.to_owned(), attempt, slot),
                (false, Arc::clone(&stalled), Some(Arc::clone(&wake))),
            );
        (stalled, wake)
    }

    /// The stall for one session, the exact key's before its `(phase, attempt)`'s, taken.
    fn take_stall(&self, key: &SessionKey<'_>) -> Option<Stall> {
        let mut stalls = self
            .stalls
            .lock()
            .expect("no panic holds the fake orchestrator's lock");
        stalls
            .remove(&(
                key.phase.to_owned(),
                key.attempt,
                Some((key.fanout_index, key.call)),
            ))
            .or_else(|| stalls.remove(&(key.phase.to_owned(), key.attempt, None)))
    }

    /// Elapse `by` between the driver's `done` and the settle, so a step outlives its deadline
    /// without anything sleeping (ANA-2 `:439`, plan D8).
    pub fn advance_after_done(&self, by: TimeDelta) {
        *self
            .after_done_advance
            .lock()
            .expect("no panic holds the fake orchestrator's lock") = Some(by);
    }

    /// The [`GraphSource`] the walk resolves through, built fresh so its `PhaseId -> name` index
    /// belongs to one resolution.
    #[must_use]
    pub fn graphs(&self) -> FakeGraphSource<'_> {
        let registered = self
            .candidates
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .clone();
        let mut source = FakeGraphSource::new(&self.store);
        source.candidates = registered;
        source.refused = self
            .refused_prompts
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .clone();
        source.missing = self
            .missing_templates
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
            .clone();
        source
    }

    /// The script for one `(phase, attempt)`'s index-0 session, or the default.
    #[must_use]
    pub fn script_for(&self, phase: &str, attempt: i32) -> ScriptedStep {
        self.script_for_key(&SessionKey {
            phase,
            attempt,
            fanout_index: 0,
            call: 0,
        })
    }

    /// The script for one session (plan D68): the exact key's, else its `(phase, attempt)`'s, else
    /// the default — so every script written before fan-out keeps working unchanged.
    #[must_use]
    pub fn script_for_key(&self, key: &SessionKey<'_>) -> ScriptedStep {
        let scripts = self
            .scripts
            .lock()
            .expect("no panic holds the fake orchestrator's lock");
        scripts
            .get(&(
                key.phase.to_owned(),
                key.attempt,
                Some((key.fanout_index, key.call)),
            ))
            .or_else(|| scripts.get(&(key.phase.to_owned(), key.attempt, None)))
            .cloned()
            .unwrap_or_else(|| self.default_script.clone())
    }

    /// One driver per session, which is `FakeDriver`'s own rule: it plays its script once and
    /// refuses a second `start` (`crates/htui-agent/src/fake.rs:178-180`, blueprint H-12).
    ///
    /// Boxed rather than a bare `FakeDriver` because
    /// [`ScriptedStep::refusing_to_start`] hands out a [`RefusingDriver`] instead, which is the
    /// only way a case can drive ANA-2 `:639`'s spawn failure without editing `htui-agent`.
    #[must_use]
    pub fn driver_for(&self, phase: &str, attempt: i32) -> Box<dyn AgentDriver> {
        self.driver_for_key(&SessionKey {
            phase,
            attempt,
            fanout_index: 0,
            call: 0,
        })
    }

    /// [`driver_for`](Self::driver_for) for one session, by its whole [`SessionKey`] (plan D68):
    /// the shape `engine::DriverFor` hands the walk.
    #[must_use]
    pub fn driver_for_key(&self, key: &SessionKey<'_>) -> Box<dyn AgentDriver> {
        // MOD-138 M3 T3: every session counted, and held when a case asked for it.
        Box::new(Counted {
            inner: self.scripted_driver(key),
            sessions: Arc::clone(&self.sessions),
            hold: self.hold_for(key),
        })
    }

    /// [`driver_for_key`](Self::driver_for_key)'s driver before it is counted.
    fn scripted_driver(&self, key: &SessionKey<'_>) -> Box<dyn AgentDriver> {
        let scripted = self.script_for_key(key);
        match scripted.spawn_failure {
            Some(reason) => Box::new(RefusingDriver {
                name: FAKE_AGENT_NAME.to_owned(),
                caps: self.caps,
                reason,
            }),
            None => {
                // MOD-125 M3 D9: a refused resume answers the first start, the script the next.
                let driver = FakeDriver::new(FAKE_AGENT_NAME, self.caps, scripted.script)
                    .failing_starts(self.take_resume_refusals(key));
                // MOD-11 D9: the handle is taken before the driver is boxed.
                self.specs
                    .lock()
                    .expect("no panic holds the fake orchestrator's lock")
                    .insert(owned(key), driver.spec_handle());
                let body = self
                    .tool_bodies
                    .lock()
                    .expect("no panic holds the fake orchestrator's lock")
                    .get(&owned(key))
                    .cloned();
                match body {
                    Some(body) => Box::new(ToolWriting {
                        inner: driver,
                        store: self.store.clone(),
                        clock: self.clock.clone(),
                        host: self.tools.clone(),
                        body,
                        steal: self
                            .steal_before_tool_writes
                            .load(Ordering::SeqCst)
                            .then_some(self.owner),
                        outcomes: Arc::clone(&self.tool_writes),
                    }),
                    None => Box::new(driver),
                }
            }
        }
    }

    /// Plan D13's stand-in for MOD-11's `document_write`, called between stage 4 and stage 5.
    ///
    /// Elapses any [`advance_after_done`](Self::advance_after_done) first — that is what "after the
    /// session and before the settle" means for a deadline case — then writes the scripted output
    /// document, or none. T4's `impl SessionSink for FakeOrchestrator` forwards to this and adds
    /// nothing. A [`stall_after_done`](Self::stall_after_done) of this session never returns; a
    /// [`suspend_after_done`](Self::suspend_after_done) returns `Ok(None)` once woken.
    ///
    /// # Errors
    /// The store's own refusals.
    pub async fn after_done(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        key: &SessionKey<'_>,
    ) -> Result<Option<Document>> {
        if let Some(by) = *self
            .after_done_advance
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
        {
            self.clock.advance(by);
        }
        if let Some((write_output, stalled, wake)) = self.take_stall(key) {
            if write_output {
                self.write_output(item, step, phase, key).await?;
            }
            stalled.notify_one();
            let Some(wake) = wake else {
                return std::future::pending().await;
            };
            wake.notified().await;
            return Ok(None);
        }
        self.write_output(item, step, phase, key).await
    }

    /// Blueprint D203's author, on the fake: writes `step`'s output document of `kind` with
    /// `body`, stamped by this harness's user and clock, as a promoted step's chat — or MOD-11's
    /// `document_write` — would. `run_worker`'s `StepAuthor` is the same seam behind the real
    /// worker; a case uses this one to give a promoted step the artefact `AcceptArtifact` needs.
    ///
    /// # Errors
    /// The store's own refusals.
    pub async fn author(
        &self,
        item: ItemId,
        step: &RunStep,
        kind: &str,
        body: &str,
    ) -> Result<Document> {
        self.store
            .write_document(NewDocument {
                id: DocumentId::new(),
                item_id: item,
                kind: kind.to_owned(),
                title: format!("{kind} (attempt {}, authored)", step.attempt),
                body: body.to_owned(),
                produced_by_step_id: Some(step.id),
                created_by: self.user,
                created_at: self.clock.now(),
            })
            .await
    }

    /// The scripted output document of one session, or none.
    async fn write_output(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        key: &SessionKey<'_>,
    ) -> Result<Option<Document>> {
        let Some(body) = self.script_for_key(key).output else {
            return Ok(None);
        };
        let document = self
            .store
            .write_document(NewDocument {
                id: DocumentId::new(),
                item_id: item,
                kind: phase.output_kind.clone(),
                title: format!("{} (attempt {})", phase.output_kind, step.attempt),
                body,
                produced_by_step_id: Some(step.id),
                created_by: self.user,
                created_at: self.clock.now(),
            })
            .await?;
        Ok(Some(document))
    }

    /// `run.target_box_id` for every run this harness starts.
    #[must_use]
    pub const fn box_id(&self) -> BoxId {
        self.box_id
    }

    /// `run.started_by` and `document.created_by`.
    #[must_use]
    pub const fn user(&self) -> UserId {
        self.user
    }

    /// `claim_run`'s owner token, minted once per orchestrator.
    #[must_use]
    pub const fn owner(&self) -> Uuid {
        self.owner
    }

    /// MOD-41 plan D12: who walks a command's tail in the engines built over this harness.
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    #[must_use]
    pub fn tails(&self) -> Tails {
        *self
            .tails
            .lock()
            .expect("no panic holds the fake orchestrator's lock")
    }

    /// Sets [`tails`](Self::tails) for every engine built from now on: a TUI on a `worker` box is
    /// [`Tails::HandBack`] (MOD-41 plan D12).
    ///
    /// # Panics
    /// When a lock is poisoned, which no case does.
    pub fn set_tails(&self, tails: Tails) {
        *self
            .tails
            .lock()
            .expect("no panic holds the fake orchestrator's lock") = tails;
    }

    /// The driver capabilities every session is built with.
    #[must_use]
    pub const fn caps(&self) -> DriverCaps {
        self.caps
    }

    /// Build an `Engine` over this harness's parts and dispatch one command (blueprint §4.4).
    ///
    /// One line, because every part it borrows is already public on this type and
    /// `crate::engine::dispatch_fake` is the single place that assembles them: the store is
    /// [`store`](Self::store), the graph source is [`graphs`](Self::graphs), the isolator and clock
    /// are the two public fields, the driver factory is [`driver_for`](Self::driver_for), the
    /// session sink is [`after_done`](Self::after_done), and the three identities are
    /// [`box_id`](Self::box_id) / [`user`](Self::user) / [`owner`](Self::owner). Forwarding rather
    /// than re-assembling is what keeps `engine.rs`'s own tests and the conformance binding on one
    /// wiring instead of two.
    ///
    /// # Errors
    /// Every [`EngineError`] the walk raises.
    pub async fn dispatch(
        &self,
        command: Command,
    ) -> std::result::Result<CommandOutcome, EngineError> {
        crate::engine::dispatch_fake(self, command).await
    }

    /// `Engine::claim` over the same parts (plan D84): re-attempt a run a refusal left `queued`.
    ///
    /// # Errors
    /// Every [`EngineError`] the claim and the walk raise.
    pub async fn claim(&self, run: RunId) -> std::result::Result<CommandOutcome, EngineError> {
        crate::engine::claim_fake(self, run).await
    }

    /// `Engine::enqueue_in_batch` over the same parts (MOD-12 M2): an `auto` run of `item` under
    /// `batch`, left `queued` for [`claim`](Self::claim).
    ///
    /// # Errors
    /// Every [`EngineError`] the enqueue raises.
    pub async fn enqueue_in_batch(
        &self,
        item: ItemId,
        batch: BatchId,
        workspace: Option<htui_core::model::WorkspaceId>,
    ) -> std::result::Result<RunId, EngineError> {
        crate::engine::enqueue_in_batch_fake(self, item, batch, workspace).await
    }

    /// `Engine::sweep` over the same parts (plan D98): adopt every expired lease on this box,
    /// adjudicate, and walk nothing.
    ///
    /// # Errors
    /// The adoption's own store error; a run's recovery failure is its `Next::Error`.
    pub async fn sweep(&self) -> std::result::Result<Vec<crate::engine::Adopted>, EngineError> {
        crate::engine::sweep_fake(self).await
    }

    /// `Engine::resume` over the same parts (ANA-2 §12 criterion 3).
    ///
    /// The twin of [`dispatch`](Self::dispatch), and the one the conformance suite's
    /// `topology_mismatch_parks_on_resume` drives: a resumed run re-resolves the live graph and
    /// compares its `topology` with the snapshot it was created with.
    ///
    /// # Errors
    /// Every [`EngineError`] the walk raises.
    pub async fn resume(
        &self,
        run: RunId,
    ) -> std::result::Result<crate::engine::Resume, EngineError> {
        crate::engine::resume_fake(self, run).await
    }

    /// The run's steps in `(position, attempt, fanout_index)` order.
    ///
    /// # Panics
    /// Never: `MemStore` fails no read.
    #[must_use]
    pub async fn steps(&self, run: RunId) -> Vec<RunStep> {
        self.store
            .run_steps(run)
            .await
            .expect("MemStore never fails a read")
    }

    /// One item row.
    ///
    /// # Panics
    /// When the item is not there, which in a case means the fixture moved.
    #[must_use]
    pub async fn item(&self, id: ItemId) -> Item {
        self.store
            .item(id)
            .await
            .expect("MemStore never fails a read")
            .expect("the case names an item the fixture holds")
    }

    /// The item's derived display state (MOD-144): its stored status over its latest graph run.
    ///
    /// # Panics
    /// When the item is not there, which in a case means the fixture moved.
    pub async fn state(&self, id: ItemId) -> ItemState {
        state_in(&self.store, id).await
    }

    /// One run row.
    ///
    /// # Panics
    /// When the run is not there, which in a case means the walk never created it.
    #[must_use]
    pub async fn run(&self, id: RunId) -> Run {
        self.store
            .run(id)
            .await
            .expect("MemStore never fails a read")
            .expect("the case names a run the walk created")
    }
}

/// The `settings` keys a v1 snapshot never carried: MOD-138 D1's bounds.
const V2_BOUND_KEYS: [&str; 4] = [
    "max_parallel_sessions",
    "max_plan_revisions",
    "max_child_runs",
    "runtime_changes",
];

/// MOD-138 T5: rewrites `run`'s stored snapshot as the v1 blob a pre-MOD-138 binary wrote: `v: 1`,
/// no `edges`, `source`, `graph.scope`/`revision`, the four bound keys removed from `settings`, and
/// `topology = graph::topology(&phases)` (the v1 digest). Through `MemStore::set_run_snapshot`.
///
/// `async` where the blueprint has it sync: `MemStore` reads a run only through `ReadStore::run`.
///
/// # Panics
/// When the run has no decodable snapshot, or a node that is not a linear `step` (v1 had none).
pub async fn rewrite_as_v1(store: &MemStore, run: RunId) {
    let blob = store
        .run(run)
        .await
        .expect("MemStore never fails a read")
        .expect("the run exists")
        .graph_snapshot
        .expect("the run carries a snapshot");
    let mut snapshot: GraphSnapshot =
        serde_json::from_value(blob).expect("the run's snapshot decodes");
    assert!(
        snapshot.phases.iter().all(|phase| phase.role.is_step()
            && !phase.palette
            && phase.plan_scope.is_none()
            && phase.divide.is_none()),
        "a v1 snapshot holds linear `step` nodes only: {:?}",
        snapshot.phases
    );
    snapshot.v = GraphSnapshot::V1;
    snapshot.edges.clear();
    snapshot.source = None;
    snapshot.graph.scope = None;
    snapshot.graph.revision = None;
    snapshot.topology = crate::graph::topology(&snapshot.phases).expect("the phases serialise");
    let mut blob = serde_json::to_value(&snapshot).expect("a snapshot serialises");
    let settings = blob
        .get_mut("settings")
        .and_then(serde_json::Value::as_object_mut)
        .expect("a snapshot's settings are an object");
    for key in V2_BOUND_KEYS {
        settings.remove(key);
    }
    assert!(store.set_run_snapshot(run, blob), "the run exists");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeDelta;
    use htui_agent::conformance::epoch;
    use htui_agent::driver::DriverCaps;
    use htui_agent::event::StopReason;
    use htui_agent::fake::FakeDriver;
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        Isolation, RepoId, RunMode, RunStep, SnapshotCandidate, SnapshotPhase, StepId,
    };
    use htui_core::prompt::DiffBlock;
    use htui_core::store::{MemStore, ReadStore as _};

    use super::IsolatorCall;
    use super::{FakeGraphSource, FakeIsolator, FakeOrchestrator, GraphSource, ScriptedStep};
    use crate::engine::SessionKey;
    use crate::graph::{ResolveError, Resolved, resolve};
    use crate::isolate::{
        ChangedPaths, Clock as _, EnvFiles, FanoutSlot, IsolateError, Isolator as _, PartTip,
        ResetReport,
    };

    /// `graph::resolve` of `HTUI_FEAT-1`, manual, no `app_setting`, no requested scope.
    async fn resolve_feat<G: GraphSource>(
        store: &MemStore,
        source: &G,
    ) -> Result<Resolved, ResolveError> {
        let item = store
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture holds HTUI_FEAT-1");
        resolve(
            store,
            source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
    }

    /// `RUN_1`'s first step, forced to attempt 1: the fixture's own `attempt: 0` is the bug ANA-2
    /// `:485-486` says MOD-4 corrects, and nothing here is a test of it.
    fn prd_step() -> RunStep {
        let mut step = demo_data()
            .steps
            .into_iter()
            .find(|step| step.run_id == ids::RUN_1 && step.position == 0)
            .expect("RUN_1 has a step at position 0");
        step.attempt = 1;
        step
    }

    /// The `prd` phase of the resolved `feature` snapshot.
    async fn prd_phase(store: &MemStore) -> SnapshotPhase {
        resolve_feat(store, &FakeGraphSource::new(store))
            .await
            .expect("the stand-in supplies rung 1")
            .snapshot
            .phases
            .remove(0)
    }

    fn repos() -> Vec<RepoId> {
        vec![RepoId::new(), RepoId::new()]
    }

    /// The fake's contract in one assertion: two repos, two trees, two distinct synthetic bases,
    /// and not one path that exists.
    #[tokio::test]
    async fn prepare_invents_a_tree_per_repo_and_touches_no_disk() {
        let isolator = FakeIsolator::new();
        let step = StepId::new();
        let scope = repos();
        let prepared = isolator
            .prepare(ids::RUN_2, step, &scope, Isolation::Worktree, None)
            .await
            .expect("the fake never refuses");

        assert_eq!(prepared.trees.len(), 2);
        assert_eq!(
            prepared.cwd,
            std::path::PathBuf::from(format!("/fake/trees/{}/{step}", ids::RUN_2))
        );
        assert!(
            !prepared.cwd.exists(),
            "the fake creates no filesystem state"
        );
        assert_eq!(prepared.trees[0].before_hash, "fake:base:1");
        assert_eq!(prepared.trees[1].before_hash, "fake:base:2");
        assert_eq!(prepared.trees[0].tree.base_ref, "fake:base:1");
        assert_eq!(prepared.trees[0].tree.mode, Isolation::Worktree);
        assert!(!prepared.trees[0].tree.dirty);
        assert_eq!(
            prepared.trees[1].tree.path,
            format!("/fake/trees/{}/{step}/{}", ids::RUN_2, scope[1])
        );
        assert!(!std::path::Path::new(&prepared.trees[1].tree.path).exists());

        let empty = isolator
            .prepare(ids::RUN_2, step, &[], Isolation::Local, None)
            .await
            .expect("an empty scope is the demo fixture's own shape");
        assert!(empty.trees.is_empty());
    }

    /// Criterion 7 needs two identical `after_hash` values on purpose, and the `missing_output`
    /// path needs a step that committed nothing; both are scripted, and an unscripted capture is
    /// distinguishable from either.
    #[tokio::test]
    async fn capture_is_scripted_first_and_synthetic_after() {
        let isolator = FakeIsolator::new();
        let step = StepId::new();
        let scope = repos();
        let trees: Vec<_> = isolator
            .prepare(ids::RUN_2, step, &scope, Isolation::Copy, None)
            .await
            .expect("the fake never refuses")
            .trees
            .into_iter()
            .map(|prepared| prepared.tree)
            .collect();

        isolator.script_after(Some("same"));
        isolator.script_after(Some("same"));
        isolator.script_after(None);

        let first = isolator.capture(step, &trees).await.expect("scripted");
        assert_eq!(
            first
                .iter()
                .map(|commit| commit.after_hash.clone())
                .collect::<Vec<_>>(),
            vec![Some("same".to_owned()); 2],
            "one scripted value applies to every repo of the call"
        );
        assert_eq!(first[0].before_hash, "fake:base:1");

        let second = isolator.capture(step, &trees).await.expect("scripted");
        assert_eq!(second[0].after_hash, first[0].after_hash);

        let nothing = isolator
            .capture(step, &trees)
            .await
            .expect("scripted `None`");
        assert_eq!(nothing[0].after_hash, None, "the step committed nothing");

        let synthetic = isolator.capture(step, &trees).await.expect("unscripted");
        assert_eq!(
            synthetic[0].after_hash,
            Some(format!("fake:after:{step}:3"))
        );
        assert_ne!(synthetic[0].after_hash, synthetic[1].after_hash);
    }

    /// MOD-4 milestone 4 D54: the fake's group base is stable per repo and costs no hash ordinal
    /// (blueprint H-13), and a slotted `prepare` starts from it.
    #[tokio::test]
    async fn the_fake_honours_the_slot_base() {
        let isolator = FakeIsolator::new();
        let scope = repos();
        let base = isolator.base(&scope).await.expect("the fake never refuses");
        assert_eq!(
            base,
            scope
                .iter()
                .map(|repo| (*repo, format!("fake:group-base:{repo}")))
                .collect::<BTreeMap<_, _>>()
        );
        assert!(
            isolator
                .base(&[])
                .await
                .expect("the fake never refuses")
                .is_empty()
        );

        let slotted = isolator
            .prepare(
                ids::RUN_2,
                StepId::new(),
                &scope,
                Isolation::Worktree,
                Some(FanoutSlot {
                    index: 1,
                    width: 3,
                    base: &base,
                }),
            )
            .await
            .expect("the fake never refuses");
        for (prepared, repo) in slotted.trees.iter().zip(&scope) {
            assert_eq!(prepared.before_hash, base[repo]);
            assert_eq!(prepared.tree.base_ref, base[repo]);
        }
        assert_eq!(isolator.prepares(), 1, "a slotted prepare is still counted");

        // `base` and the slotted `prepare` took no ordinal: the next unslotted one is the first.
        let plain = isolator
            .prepare(
                ids::RUN_2,
                StepId::new(),
                &scope[..1],
                Isolation::Worktree,
                None,
            )
            .await
            .expect("the fake never refuses");
        assert_eq!(plain.trees[0].before_hash, "fake:base:1");

        // A repo the base does not name falls back to a synthetic hash.
        let partial = BTreeMap::from([(scope[0], "fake:group-base:only".to_owned())]);
        let mixed = isolator
            .prepare(
                ids::RUN_2,
                StepId::new(),
                &scope,
                Isolation::Worktree,
                Some(FanoutSlot {
                    index: 0,
                    width: 2,
                    base: &partial,
                }),
            )
            .await
            .expect("the fake never refuses");
        assert_eq!(mixed.trees[0].before_hash, "fake:group-base:only");
        assert_eq!(mixed.trees[1].before_hash, "fake:base:2");
    }

    /// D54(c): the fake's `diff` answers what a case queued, FIFO, and `None` unscripted.
    #[tokio::test]
    async fn the_fake_diff_is_scripted_and_none_by_default() {
        let isolator = FakeIsolator::new();
        assert_eq!(
            isolator
                .diff(&[], &[])
                .await
                .expect("the fake never refuses"),
            None
        );
        let block = DiffBlock {
            range: "a..b".to_owned(),
            stat: " f | 1 +\n".to_owned(),
            diff: "diff --git a/f b/f\n".to_owned(),
        };
        isolator.script_diff(Some(block.clone()));
        isolator.script_diff(None);
        assert_eq!(
            isolator.diff(&[], &[]).await.expect("scripted"),
            Some(block)
        );
        assert_eq!(isolator.diff(&[], &[]).await.expect("scripted"), None);
        assert_eq!(isolator.diff(&[], &[]).await.expect("unscripted"), None);
    }

    /// MOD-9 D119/D135: the fake's `changed_paths` answers what a case queued, FIFO, the empty
    /// set unscripted, and records each call's rows.
    #[tokio::test]
    async fn scripted_changed_paths_round_trip() {
        let isolator = FakeIsolator::new();
        let step = StepId::new();
        let scope = repos();
        let trees: Vec<_> = isolator
            .prepare(ids::RUN_2, step, &scope, Isolation::Worktree, None)
            .await
            .expect("the fake never refuses")
            .trees
            .into_iter()
            .map(|prepared| prepared.tree)
            .collect();
        let commits = isolator
            .capture(step, &trees)
            .await
            .expect("the fake never refuses");

        assert_eq!(
            isolator
                .changed_paths(&trees, &commits)
                .await
                .expect("unscripted"),
            ChangedPaths::default()
        );
        let scripted = ChangedPaths {
            paths: vec![(scope[0], "src/lib.rs".to_owned())],
            truncated: true,
        };
        isolator.script_changed_paths(scripted.clone());
        assert_eq!(
            isolator
                .changed_paths(&trees, &commits)
                .await
                .expect("scripted"),
            scripted
        );
        assert_eq!(
            isolator
                .changed_paths(&trees, &commits)
                .await
                .expect("consumed once"),
            ChangedPaths::default()
        );
        isolator.fail_changed_paths("x");
        match isolator.changed_paths(&[], &[]).await {
            Err(IsolateError::Git(reason)) => assert_eq!(reason, "x"),
            other => panic!("expected a git error, got {other:?}"),
        }

        let requests = isolator.changed_path_requests();
        assert_eq!(
            requests.len(),
            4,
            "every call is recorded, the failed one included"
        );
        for request in &requests[..3] {
            assert_eq!(request.trees, vec![step; scope.len()]);
            assert_eq!(request.commits, vec![step; scope.len()]);
        }
        assert!(requests[3].trees.is_empty() && requests[3].commits.is_empty());
    }

    /// MOD-89: the fake's `env_files` answers what a case queued, FIFO, the empty set unscripted,
    /// a scripted failure as a `git` error, and records every call's `(cwd, extra_dirs)`.
    #[tokio::test]
    async fn env_files_answers_its_script_then_nothing_and_records_the_roots() {
        let isolator = FakeIsolator::new();
        let cwd = std::path::PathBuf::from("/fake/trees/run/step");
        let extra = vec![std::path::PathBuf::from("/fake/trees/docs")];
        let first = EnvFiles {
            paths: vec![".env".to_owned()],
            truncated: None,
        };
        let second = EnvFiles {
            paths: vec!["docs/.envrc".to_owned()],
            truncated: Some(crate::isolate::env_files::SCAN_CAP),
        };
        isolator.script_env_files(first.clone());
        isolator.script_env_files(second.clone());

        assert_eq!(
            isolator.env_files(&cwd, &extra).await.expect("scripted"),
            first
        );
        assert_eq!(
            isolator.env_files(&cwd, &[]).await.expect("scripted"),
            second
        );
        assert_eq!(
            isolator.env_files(&cwd, &extra).await.expect("unscripted"),
            EnvFiles::default()
        );
        isolator.fail_env_files("x");
        match isolator.env_files(&cwd, &[]).await {
            Err(IsolateError::Git(reason)) => assert_eq!(reason, "x"),
            other => panic!("expected a git error, got {other:?}"),
        }

        assert_eq!(
            isolator.env_file_requests(),
            vec![
                (cwd.clone(), extra.clone()),
                (cwd.clone(), Vec::new()),
                (cwd.clone(), extra.clone()),
                (cwd.clone(), Vec::new()),
            ],
            "every call is recorded, in order, the failed one included"
        );
    }

    /// D92/D99: the fake's `reset` is empty unless scripted, a scripted refusal is consumed once
    /// and names the first row's repository, and every call is counted.
    #[tokio::test]
    async fn the_fake_reset_is_scripted_and_empty_by_default() {
        let isolator = FakeIsolator::new();
        let step = StepId::new();
        let scope = repos();
        let trees: Vec<_> = isolator
            .prepare(ids::RUN_2, step, &scope, Isolation::SharedSerialized, None)
            .await
            .expect("the fake never refuses")
            .trees
            .into_iter()
            .map(|prepared| prepared.tree)
            .collect();
        assert_eq!(isolator.resets(), 0);

        assert_eq!(
            isolator.reset(step, &trees).await.expect("unscripted"),
            ResetReport::default()
        );
        isolator.script_reset_refusal("dirty_tree_not_reset: /fake/trees/x");
        assert_eq!(
            isolator.reset(step, &trees).await.expect("scripted"),
            ResetReport {
                labelled: Vec::new(),
                refused: vec![(
                    trees[0].repo_id,
                    "dirty_tree_not_reset: /fake/trees/x".to_owned()
                )],
            }
        );
        assert_eq!(
            isolator.reset(step, &trees).await.expect("consumed"),
            ResetReport::default(),
            "a scripted refusal answers once"
        );
        assert_eq!(isolator.resets(), 3, "every call is counted");
    }

    /// The rows a fake `premerge` or `unmerged` call is handed: one shared tree per repo of the
    /// scope, and per part key a part whose commit rows carry `after_hash` in the repos named.
    fn premerge_rows(
        scope: &[RepoId],
        parts: &[(&str, &[usize])],
    ) -> (Vec<htui_core::model::RunStepTree>, Vec<PartTip>) {
        let integrate = StepId::new();
        let trees = scope
            .iter()
            .map(|repo| htui_core::model::RunStepTree {
                run_step_id: integrate,
                repo_id: *repo,
                mode: Isolation::Worktree,
                path: format!("/fake/trees/integrate/{repo}"),
                base_ref: format!("fake:group-base:{repo}"),
                dirty: false,
            })
            .collect();
        let parts = parts
            .iter()
            .map(|(key, committed)| {
                let step = StepId::new();
                PartTip {
                    step,
                    key: (*key).to_owned(),
                    trees: Vec::new(),
                    commits: scope
                        .iter()
                        .enumerate()
                        .map(|(at, repo)| htui_core::model::RunStepCommit {
                            run_step_id: step,
                            repo_id: *repo,
                            before_hash: format!("fake:group-base:{repo}"),
                            after_hash: committed
                                .contains(&at)
                                .then(|| format!("fake:after:{key}:{at}")),
                        })
                        .collect(),
                }
            })
            .collect();
        (trees, parts)
    }

    /// MOD-139 M3 T1: unscripted, the fake's pre-merge merges every committed part of every repo
    /// in plan order and reads the rest `NoCommits`; `head` is the repo's last merge, else its
    /// base. The call is logged first, under the integrate step, with the parts it was handed.
    #[tokio::test]
    async fn the_fake_premerge_merges_by_default_and_logs_first() {
        use htui_core::model::PartMerge;

        let isolator = FakeIsolator::new();
        let scope = repos();
        let (trees, parts) = premerge_rows(&scope, &[("api", &[0, 1]), ("ui", &[0])]);
        let integrate = trees[0].run_step_id;

        let report = isolator
            .premerge(integrate, &trees, &parts)
            .await
            .expect("unscripted, the fake never refuses");
        assert_eq!(report.repos.len(), 2, "one entry per tree row");
        let first = &report.repos[0];
        assert_eq!(first.repo_id, scope[0]);
        assert_eq!(
            first.name,
            scope[0].to_string(),
            "the fake names a repo by its id"
        );
        let merges: Vec<_> = first
            .parts
            .iter()
            .map(|part| match &part.merge {
                PartMerge::Merged { commit } => commit.clone(),
                other => panic!("both parts committed in the first repo: {other:?}"),
            })
            .collect();
        assert_eq!(
            first.parts.iter().map(|part| part.step).collect::<Vec<_>>(),
            vec![parts[0].step, parts[1].step],
            "plan order"
        );
        assert_eq!(first.parts[0].key, "api");
        assert_eq!(first.parts[0].tip.as_deref(), Some("fake:after:api:0"));
        assert!(merges[0].starts_with(&format!("fake:merge:{}:", parts[0].step)));
        assert_eq!(first.head, merges[1], "head is the repo's last merge");

        let second = &report.repos[1];
        assert!(matches!(second.parts[0].merge, PartMerge::Merged { .. }));
        assert_eq!(second.parts[1].merge, PartMerge::NoCommits);
        assert_eq!(second.parts[1].tip, None);
        let PartMerge::Merged { commit } = &second.parts[0].merge else {
            unreachable!()
        };
        assert_eq!(&second.head, commit);

        let (none, none_parts) = premerge_rows(&scope[..1], &[("api", &[])]);
        let report = isolator
            .premerge(integrate, &none, &none_parts)
            .await
            .expect("unscripted");
        assert_eq!(
            report.repos[0].head, none[0].base_ref,
            "a repo with nothing merged keeps its base as head"
        );

        assert_eq!(
            isolator.call_log(),
            vec![
                IsolatorCall::Premerge(integrate),
                IsolatorCall::Premerge(integrate)
            ]
        );
        assert_eq!(
            isolator.premerge_requests(),
            vec![
                (integrate, vec![parts[0].step, parts[1].step]),
                (integrate, vec![none_parts[0].step]),
            ]
        );
        assert_eq!(
            isolator
                .premerge(integrate, &[], &parts)
                .await
                .expect("a repo-less run"),
            htui_core::model::PremergeReport::default(),
            "no tree, no repo entry (blueprint P-10)"
        );
    }

    /// MOD-139 M3 T1 (blueprint P-15): a conflict scripted by part key stops every repo at that
    /// part and marks the later ones `NotAttempted`; the script is FIFO and answers once.
    #[tokio::test]
    async fn a_scripted_premerge_conflict_stops_the_repo() {
        use htui_core::model::PartMerge;

        let isolator = FakeIsolator::new();
        let scope = repos();
        let (trees, parts) =
            premerge_rows(&scope, &[("api", &[0, 1]), ("ui", &[0, 1]), ("docs", &[0])]);
        let integrate = trees[0].run_step_id;

        isolator.conflict_premerge("ui", &["src/ui/a.rs"]);
        let report = isolator
            .premerge(integrate, &trees, &parts)
            .await
            .expect("a conflict is not an error");
        for repo in &report.repos {
            let PartMerge::Merged { commit } = &repo.parts[0].merge else {
                panic!("the part before the conflict merges: {:?}", repo.parts[0]);
            };
            assert_eq!(
                repo.parts[1].merge,
                PartMerge::Conflicted {
                    paths: vec!["src/ui/a.rs".to_owned()]
                }
            );
            assert_eq!(&repo.head, commit, "head stays at the last merge");
        }
        assert_eq!(
            report.repos[0].parts[2].merge,
            PartMerge::NotAttempted,
            "a later part that committed in the stopped repo is not attempted"
        );
        assert_eq!(
            report.repos[1].parts[2].merge,
            PartMerge::NoCommits,
            "a later part with no commits in the stopped repo reads `NoCommits`, as GixIsolator \
             answers"
        );
        assert_eq!(report.repos[1].parts[2].tip, None);

        let report = isolator
            .premerge(integrate, &trees, &parts)
            .await
            .expect("unscripted again");
        assert!(
            report.repos[0]
                .parts
                .iter()
                .all(|part| matches!(part.merge, PartMerge::Merged { .. })),
            "the script answered once"
        );
    }

    /// MOD-139 M3 T1: the fake reads a part's tip as GixIsolator's `committed_tip` does (an
    /// `after_hash` equal to its `before_hash` committed nothing), so its report shapes are ones
    /// the real isolator produces.
    #[tokio::test]
    async fn the_fake_premerge_reads_an_unmoved_tip_as_no_commits() {
        use htui_core::model::PartMerge;

        let isolator = FakeIsolator::new();
        let scope = repos();
        let (trees, mut parts) = premerge_rows(&scope[..1], &[("api", &[0]), ("ui", &[0])]);
        let integrate = trees[0].run_step_id;
        let unmoved = &mut parts[1].commits[0];
        unmoved.after_hash = Some(unmoved.before_hash.clone());

        let report = isolator
            .premerge(integrate, &trees, &parts)
            .await
            .expect("unscripted");
        let repo = &report.repos[0];
        assert!(matches!(repo.parts[0].merge, PartMerge::Merged { .. }));
        assert_eq!(
            repo.parts[1].merge,
            PartMerge::NoCommits,
            "`after_hash == before_hash` committed nothing"
        );
        assert_eq!(repo.parts[1].tip, None);
    }

    /// MOD-139 M3 T1: a refused pre-merge is an error, and the call is still logged.
    #[tokio::test]
    async fn a_refused_premerge_is_an_error() {
        let isolator = FakeIsolator::new();
        let (trees, parts) = premerge_rows(&repos(), &[("api", &[0])]);
        let integrate = trees[0].run_step_id;

        isolator.refuse_premerge("x");
        let err = isolator
            .premerge(integrate, &trees, &parts)
            .await
            .expect_err("scripted");
        assert!(
            matches!(&err, IsolateError::Refused(reason) if reason == "x"),
            "{err:?}"
        );
        assert_eq!(isolator.call_log(), vec![IsolatorCall::Premerge(integrate)]);
        isolator
            .premerge(integrate, &trees, &parts)
            .await
            .expect("the refusal answered once");
    }

    /// MOD-139 M3 T1 (blueprint P-15): `unmerged` answers nothing unscripted, and a script by part
    /// key answers those parts' steps in plan order, an unknown key ignored, once.
    #[tokio::test]
    async fn the_fake_unmerged_is_scripted_by_key_and_empty_by_default() {
        let isolator = FakeIsolator::new();
        let (trees, parts) =
            premerge_rows(&repos(), &[("api", &[0]), ("ui", &[0]), ("docs", &[0])]);
        let integrate = trees[0].run_step_id;
        let heads: Vec<_> = Vec::new();

        assert_eq!(
            isolator
                .unmerged(integrate, &trees, &heads, &parts)
                .await
                .expect("unscripted"),
            Vec::<StepId>::new()
        );
        isolator.script_unmerged(&["docs", "nope", "api"]);
        assert_eq!(
            isolator
                .unmerged(integrate, &trees, &heads, &parts)
                .await
                .expect("scripted"),
            vec![parts[0].step, parts[2].step],
            "plan order, the unknown key ignored"
        );
        assert!(
            isolator
                .unmerged(integrate, &trees, &heads, &parts)
                .await
                .expect("consumed")
                .is_empty()
        );
        assert_eq!(
            isolator.call_log(),
            vec![IsolatorCall::Unmerged(integrate); 3],
            "logged under the integrate step"
        );
        let handed = vec![parts[0].step, parts[1].step, parts[2].step];
        assert_eq!(
            isolator.unmerged_requests(),
            vec![(integrate, handed.clone()); 3]
        );
    }

    /// MOD-139 M3 T1: the `n`-th `premerge` and the `n`-th `reset` each log, signal and never
    /// return; the calls before them answer as usual.
    #[tokio::test]
    async fn premerge_and_reset_stalls_never_return() {
        let isolator = FakeIsolator::new();
        let (trees, parts) = premerge_rows(&repos(), &[("api", &[0])]);
        let integrate = trees[0].run_step_id;
        let wait = std::time::Duration::from_millis(50);

        let premerge = isolator.stall_nth_premerge(1);
        let reset = isolator.stall_nth_reset(2);

        let stalled = tokio::time::timeout(wait, isolator.premerge(integrate, &trees, &parts));
        let (elapsed, ()) = tokio::join!(stalled, premerge.notified());
        assert!(elapsed.is_err(), "the first premerge never answers");
        isolator
            .premerge(integrate, &trees, &parts)
            .await
            .expect("the stall was one-shot");

        isolator
            .reset(integrate, &trees)
            .await
            .expect("the first reset answers");
        let stalled = tokio::time::timeout(wait, isolator.reset(integrate, &trees));
        let (elapsed, ()) = tokio::join!(stalled, reset.notified());
        assert!(elapsed.is_err(), "the second reset never answers");
        assert_eq!(isolator.resets(), 2, "the stalled reset was counted");
        assert_eq!(
            isolator.call_log(),
            vec![
                IsolatorCall::Premerge(integrate),
                IsolatorCall::Premerge(integrate),
                IsolatorCall::Reset(integrate),
                IsolatorCall::Reset(integrate),
            ],
            "each stalled call was logged first"
        );
    }

    /// Whether a `shared_serialized` prepare of `run` is admitted within `wait` (and its guard
    /// then given back).
    async fn fake_isolator_admits(
        isolator: &FakeIsolator,
        run: htui_core::model::RunId,
        wait_ms: u64,
    ) -> bool {
        let admitted = tokio::time::timeout(
            std::time::Duration::from_millis(wait_ms),
            isolator.prepare(
                run,
                StepId::new(),
                &repos(),
                Isolation::SharedSerialized,
                None,
            ),
        )
        .await
        .is_ok();
        isolator.release(run).await.expect("release answers");
        admitted
    }

    /// MOD-125 M3 D5a: the fake's `reacquire` logs and counts itself, retakes the serial lock for
    /// a `shared_serialized` row (without waiting on a guard the step still holds) and keeps it to
    /// the step's `capture`; rows in no shared mode take nothing.
    #[tokio::test]
    async fn fake_isolate_reacquire_retakes_a_shared_guard_until_capture() {
        let isolator = FakeIsolator::new();
        let step = StepId::new();
        let prepared = isolator
            .prepare(
                ids::RUN_1,
                step,
                &repos(),
                Isolation::SharedSerialized,
                None,
            )
            .await
            .expect("the step prepares");
        let rows = prepared
            .trees
            .iter()
            .map(|prepared| prepared.tree.clone())
            .collect::<Vec<_>>();
        isolator.release(ids::RUN_1).await.expect("release answers");
        assert!(fake_isolator_admits(&isolator, ids::RUN_2, 5_000).await);

        isolator
            .reacquire(ids::RUN_1, step, &rows)
            .await
            .expect("the fake reacquires");
        isolator
            .reacquire(ids::RUN_1, step, &rows)
            .await
            .expect("a second reacquire does not wait on its own guard");
        assert_eq!(isolator.reacquires(), 2);
        assert!(
            !fake_isolator_admits(&isolator, ids::RUN_2, 50).await,
            "the reacquired guard is held"
        );
        let log = isolator.call_log();
        let reacquires = log
            .iter()
            .filter(|call| matches!(call, IsolatorCall::Reacquire(_)))
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(reacquires, vec![IsolatorCall::Reacquire(step); 2]);
        let released = log
            .iter()
            .position(|call| *call == IsolatorCall::Release(ids::RUN_1))
            .expect("the stop's release is logged");
        let reacquired = log
            .iter()
            .position(|call| *call == IsolatorCall::Reacquire(step))
            .expect("the reacquire is logged");
        assert!(released < reacquired, "{log:?}");

        isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");
        assert!(
            fake_isolator_admits(&isolator, ids::RUN_2, 5_000).await,
            "capture released the reacquired guard"
        );

        let local = StepId::new();
        let local_rows = rows
            .iter()
            .map(|row| htui_core::model::RunStepTree {
                run_step_id: local,
                mode: Isolation::Local,
                ..row.clone()
            })
            .collect::<Vec<_>>();
        isolator
            .reacquire(ids::RUN_1, local, &local_rows)
            .await
            .expect("a local row reacquires");
        assert!(
            fake_isolator_admits(&isolator, ids::RUN_2, 5_000).await,
            "a local row takes no guard"
        );
    }

    /// MOD-125 M3 D5a: `refuse_reacquire` queues the next `reacquire`'s refusal, which is
    /// answered verbatim with no guard held afterwards.
    #[tokio::test]
    async fn fake_isolate_refuse_reacquire_answers_the_reason_and_holds_nothing() {
        let isolator = FakeIsolator::new();
        let step = StepId::new();
        let prepared = isolator
            .prepare(
                ids::RUN_1,
                step,
                &repos(),
                Isolation::SharedSerialized,
                None,
            )
            .await
            .expect("the step prepares");
        let rows = prepared
            .trees
            .iter()
            .map(|prepared| prepared.tree.clone())
            .collect::<Vec<_>>();
        let reason = "moved_while_stopped: /x at a, base_ref b";
        isolator.refuse_reacquire(reason);

        let err = isolator
            .reacquire(ids::RUN_1, step, &rows)
            .await
            .expect_err("the scripted refusal answers");
        assert!(
            matches!(&err, IsolateError::Refused(refused) if refused == reason),
            "{err}"
        );
        assert_eq!(isolator.reacquires(), 1);
        assert!(
            fake_isolator_admits(&isolator, ids::RUN_2, 5_000).await,
            "a refused reacquire holds nothing, the prepare's guard included"
        );
        isolator
            .reacquire(ids::RUN_1, step, &rows)
            .await
            .expect("the queue held one refusal");
    }

    /// D99 over F-I's re-key: `release(run)` drops the guard a step of that run holds, and only
    /// that run's, so a second run's shared `prepare` is admitted.
    #[tokio::test]
    async fn the_fake_release_frees_its_serial_lock() {
        let isolator = std::sync::Arc::new(FakeIsolator::new());
        let scope = repos();
        isolator
            .prepare(
                ids::RUN_1,
                StepId::new(),
                &scope,
                Isolation::SharedSerialized,
                None,
            )
            .await
            .expect("the first run's step prepares");

        let mut waiting = {
            let isolator = std::sync::Arc::clone(&isolator);
            let scope = scope.clone();
            tokio::spawn(async move {
                isolator
                    .prepare(
                        ids::RUN_2,
                        StepId::new(),
                        &scope,
                        Isolation::SharedSerialized,
                        None,
                    )
                    .await
                    .map(|prepared| prepared.trees.len())
            })
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting)
                .await
                .is_err(),
            "the second run waits on the serial lock"
        );
        assert_eq!(isolator.releases(), 0);

        isolator
            .release(ids::RUN_2)
            .await
            .expect("releasing a run that holds nothing answers");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting)
                .await
                .is_err(),
            "another run's release frees nothing"
        );

        isolator.release(ids::RUN_1).await.expect("release answers");
        let admitted = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .expect("the released lock admits the second run")
            .expect("the task did not panic")
            .expect("the second run prepares");
        assert_eq!(admitted, 2);
        assert_eq!(isolator.releases(), 2, "every call is counted");
    }

    /// `impl GraphSource for MemStore` delegates to the seven inherent reads of the same names.
    ///
    /// Method resolution prefers an inherent candidate over a trait one, so the bodies are
    /// delegation — but "prefers" is the kind of rule that is worth a test rather than a comment,
    /// because getting it wrong is an infinite recursion and not a compile error. MOD-7 milestone
    /// 3 D75: the seventh read.
    #[tokio::test]
    async fn the_store_answers_the_source_without_recursing() {
        let store = MemStore::demo();
        let resolved = GraphSource::resolve_graph(&store, ids::HTUI_FEAT_1)
            .await
            .expect("MemStore never fails a read")
            .expect("FEAT-1 resolves to the `feature` graph");
        assert_eq!(resolved.phases.len(), 4);

        assert!(
            GraphSource::phase_agents(&store, resolved.phases[0].phase.id)
                .await
                .expect("MemStore never fails a read")
                .is_empty(),
            "the fixture seeds no `phase_agent` row (blueprint H-3)"
        );
        assert!(
            GraphSource::prompt_template(&store, ids::PROJECT_HTUI, "prd", None)
                .await
                .expect("MemStore never fails a read")
                .is_some()
        );
        let agent = GraphSource::agent(&store, ids::AGENT_CLAUDE)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture seeds `claude`");
        assert_eq!(agent.name, "claude");
        assert_eq!(
            (agent.default_model, agent.models),
            (None, Vec::new()),
            "which is exactly why the fake has to name a model itself"
        );
        assert!(
            GraphSource::agent_boxes(&store, ids::BOX)
                .await
                .expect("MemStore never fails a read")
                .is_empty(),
            "the demo fixture seeds no `agent_box` row, so rung 3 finds nothing on it (plan D62)"
        );
        // MOD-9 D44, D66: the sixth read.
        let skills =
            GraphSource::bound_skills(&store, ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
                .await
                .expect("MemStore never fails a read");
        assert_eq!(
            skills,
            store
                .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
                .await
                .expect("MemStore never fails a read"),
            "the trait answers what the inherent read answers"
        );
        assert_eq!(
            skills.len(),
            2,
            "`tests` from the project and `rust-style` from the phase, each once"
        );

        // MOD-7 milestone 3 D75: the seventh read.
        use htui_core::model::{ItemId, NewItem};
        use htui_core::store::WriteStore as _;
        let needs_cuda = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: "needs a GPU toolchain".to_owned(),
                body: String::new(),
                required_tags: vec!["cuda".to_owned()],
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands")
            .id;
        let missing = GraphSource::missing_tags(&store, needs_cuda, ids::BOX)
            .await
            .expect("MemStore never fails a read of a known item and box");
        assert_eq!(missing, vec!["cuda".to_owned()]);
        assert_eq!(
            missing,
            store
                .missing_tags(needs_cuda, ids::BOX)
                .await
                .expect("MemStore never fails a read of a known item and box"),
            "the trait answers what the inherent read answers"
        );
        assert!(
            GraphSource::missing_tags(&store, ids::HTUI_FEAT_3, ids::BOX)
                .await
                .expect("MemStore never fails a read of a known item and box")
                .is_empty(),
            "FEAT-3 requires `rust`, which the demo box probed"
        );
    }

    /// Plan D20's whole content: with the stand-in, the seeded `feature` graph resolves; the
    /// project's `token_budget` reaches the snapshot; and every phase names a model, which
    /// `SnapshotCandidate` requires and the `claude` row cannot supply.
    #[tokio::test]
    async fn the_stand_in_is_what_makes_the_demo_graph_resolve() {
        let store = MemStore::demo();
        let resolved = resolve_feat(&store, &FakeGraphSource::new(&store))
            .await
            .expect("the stand-in supplies rung 1");

        assert_eq!(
            resolved
                .snapshot
                .phases
                .iter()
                .map(|phase| phase.name.as_str())
                .collect::<Vec<_>>(),
            ["prd", "plan", "implement", "review"]
        );
        for phase in &resolved.snapshot.phases {
            assert_eq!(
                phase.candidates,
                vec![SnapshotCandidate {
                    agent_id: ids::AGENT_CLAUDE,
                    agent_name: "claude".to_owned(),
                    model: "sonnet".to_owned(),
                }],
                "`{}` resolves to the stand-in",
                phase.name
            );
            assert_eq!(
                phase.token_budget,
                Some(120_000),
                "the demo project's settings carry it"
            );
        }
        assert!(resolved.snapshot.topology.starts_with("sha256:"));
    }

    /// The map is keyed by phase *name*, and the `PhaseId -> name` index it needs is built by
    /// `resolve_graph` on the way past: registering `prd` must move `prd` and nothing else.
    #[tokio::test]
    async fn candidates_are_registered_by_phase_name() {
        let store = MemStore::demo();
        let source =
            FakeGraphSource::new(&store).with_candidates("prd", vec![(ids::AGENT_CLAUDE_CLI, "x")]);
        let resolved = resolve_feat(&store, &source)
            .await
            .expect("every other phase keeps the stand-in");

        assert_eq!(
            resolved.snapshot.phases[0].candidates[0].agent_id,
            ids::AGENT_CLAUDE_CLI
        );
        assert_eq!(
            resolved.snapshot.phases[0].candidates[0].agent_name,
            "claude-cli"
        );
        assert_eq!(resolved.snapshot.phases[0].candidates[0].model, "x");
        for phase in &resolved.snapshot.phases[1..] {
            assert_eq!(phase.candidates[0].agent_id, ids::AGENT_CLAUDE);
        }
    }

    /// ANA-2 §4.1 rung 4 (`docs/ANA-2.md:288`): no `phase_agent`, no project default, no guess.
    /// A named refusal, never a silent empty walk.
    #[tokio::test]
    async fn a_phase_with_no_candidate_is_refused_by_name() {
        let store = MemStore::demo();
        let source = FakeGraphSource::new(&store).without_candidates("implement");
        let refused = resolve_feat(&store, &source)
            .await
            .expect_err("rung 4 is a refusal");
        assert!(
            matches!(&refused, ResolveError::NoCandidate { phase } if phase == "implement"),
            "{refused}"
        );
    }

    /// ANA-5's grammar is three lines at the very start of the body, no other keys
    /// (`docs/ANA-5.md:1327-1331`), and it is this constructor that has to produce them.
    #[test]
    fn a_scripted_review_opens_with_ana5s_front_matter() {
        let step = ScriptedStep::review("request-changes", "no tests");
        let body = step.output.expect("a review always writes a document");
        assert_eq!(body, "---\nverdict: request-changes\n---\nno tests");
        assert_eq!(
            body.lines().take(3).collect::<Vec<_>>(),
            ["---", "verdict: request-changes", "---"]
        );

        assert!(ScriptedStep::done_without_output().output.is_none());
        assert_eq!(
            ScriptedStep::done_with_output("prd v1").output.as_deref(),
            Some("prd v1")
        );
        assert_eq!(
            ScriptedStep::failing(StopReason::Refusal)
                .script
                .turns
                .len(),
            1
        );
        assert_eq!(
            ScriptedStep::erroring("boom", "nope").script.turns[0]
                .events
                .len(),
            1,
            "an error and no `done`: the pull past the end is the driver error settle wants"
        );
    }

    /// One driver per session is `FakeDriver`'s rule, and the script is chosen by
    /// `(phase, attempt)` so the review loop's second attempt can differ from its first.
    #[test]
    fn the_orchestrator_scripts_by_phase_and_attempt() {
        let orch = FakeOrchestrator::demo();
        orch.script("implement", 2, ScriptedStep::done_without_output());

        assert_eq!(
            orch.script_for("implement", 1).output.as_deref(),
            Some("scripted output"),
            "an unscripted attempt plays the default"
        );
        assert!(orch.script_for("implement", 2).output.is_none());

        assert_eq!(orch.driver_for("implement", 1).name(), "fake");
        assert!(orch.caps().permission_requests);
        let cli = FakeOrchestrator::demo().with_caps(DriverCaps {
            permission_requests: false,
            edit_proposals: false,
            ..FakeDriver::full_caps()
        });
        assert!(!cli.driver_for("prd", 1).caps().edit_proposals);
    }

    /// Plan D68: one session is scripted by its whole key, and every key without a script of its
    /// own falls back to its `(phase, attempt)`'s, then to the default.
    #[test]
    fn the_orchestrator_scripts_by_session_key_with_the_attempt_fallback() {
        let orch = FakeOrchestrator::demo();
        orch.script("research", 1, ScriptedStep::done_with_output("group"));
        orch.script_candidate(
            "research",
            1,
            2,
            0,
            ScriptedStep::done_with_output("candidate 2"),
        );
        orch.script_candidate(
            "research:judge",
            1,
            -1,
            1,
            ScriptedStep::done_with_output("reversed"),
        );
        let key = |phase, fanout_index, call| SessionKey {
            phase,
            attempt: 1,
            fanout_index,
            call,
        };

        let output = |key: SessionKey<'_>| orch.script_for_key(&key).output;
        assert_eq!(
            output(key("research", 2, 0)).as_deref(),
            Some("candidate 2")
        );
        assert_eq!(
            output(key("research", 1, 0)).as_deref(),
            Some("group"),
            "an unscripted candidate plays its attempt's script"
        );
        assert_eq!(
            orch.script_for("research", 1).output.as_deref(),
            Some("group"),
            "`script_for` is index 0's key"
        );
        assert_eq!(
            output(key("research:judge", -1, 1)).as_deref(),
            Some("reversed")
        );
        assert_eq!(
            output(key("research:judge", -1, 0)).as_deref(),
            Some("scripted output"),
            "the forward call is a different session and plays the default"
        );
        assert_eq!(orch.driver_for_key(&key("research", 2, 0)).name(), "fake");
    }

    /// Plan D13: the harness is the document producer, the clock moves only here, and a step that
    /// scripted no output writes nothing at all — which is the `missing_output` path.
    #[tokio::test]
    async fn after_done_writes_the_scripted_document_and_elapses_time() {
        let orch = FakeOrchestrator::demo();
        orch.script("prd", 1, ScriptedStep::done_with_output("prd v1"));
        orch.advance_after_done(TimeDelta::seconds(3));

        let step = prd_step();
        let phase = prd_phase(&orch.store).await;
        let document = orch
            .after_done(ids::HTUI_FEAT_1, &step, &phase, &SessionKey::of(&step))
            .await
            .expect("the store accepts the write")
            .expect("the step scripted an output");

        assert_eq!(document.kind, "prd");
        assert_eq!(document.body, "prd v1");
        assert_eq!(document.title, "prd (attempt 1)");
        assert_eq!(document.produced_by_step_id, Some(step.id));
        assert_eq!(document.created_by, orch.user());
        assert_eq!(orch.clock.now(), epoch() + TimeDelta::seconds(3));
        assert_eq!(document.created_at, orch.clock.now());

        let quiet = FakeOrchestrator::demo();
        quiet.script("prd", 1, ScriptedStep::done_without_output());
        assert!(
            quiet
                .after_done(ids::HTUI_FEAT_1, &step, &phase, &SessionKey::of(&step))
                .await
                .expect("the store is not asked")
                .is_none(),
            "no scripted body, no document (ANA-2 `:429-430`)"
        );
    }

    /// Blueprint A-2, F-O: a stalled session writes its document when asked, signals, and never
    /// returns; the stall is one-shot and keyed like a script, and a restart does not carry it.
    #[tokio::test]
    async fn a_stalled_after_done_signals_and_never_returns() {
        use futures::FutureExt as _;

        let orch = FakeOrchestrator::demo();
        orch.script("prd", 1, ScriptedStep::done_with_output("prd v1"));
        let step = prd_step();
        let phase = prd_phase(&orch.store).await;
        let key = SessionKey::of(&step);

        let stalled = orch.stall_after_done("prd", 1, None, true);
        let restarted = orch.restarted();
        let mut call = Box::pin(orch.after_done(ids::HTUI_FEAT_1, &step, &phase, &key));
        assert!(
            call.as_mut().now_or_never().is_none(),
            "the call never answers"
        );
        stalled
            .notified()
            .now_or_never()
            .expect("the stall signalled first");
        drop(call);
        let written = orch
            .store
            .documents(ids::HTUI_FEAT_1)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .filter(|head| head.produced_by_step_id == Some(step.id))
            .count();
        assert_eq!(
            written, 1,
            "`write_output` wrote the document before stalling"
        );

        let again = orch
            .after_done(ids::HTUI_FEAT_1, &step, &phase, &key)
            .now_or_never()
            .expect("a stall is one-shot")
            .expect("the store accepts the write");
        assert!(again.is_some());
        assert!(
            restarted
                .after_done(ids::HTUI_FEAT_1, &step, &phase, &key)
                .now_or_never()
                .is_some(),
            "a restart carries no stall (plan D118)"
        );

        let quiet = FakeOrchestrator::demo();
        let other = quiet.stall_after_done("prd", 1, Some((1, 0)), false);
        assert!(
            quiet
                .after_done(ids::HTUI_FEAT_1, &step, &phase, &key)
                .now_or_never()
                .is_some(),
            "index 0 is not the stalled `(1, 0)` session"
        );
        let candidate = SessionKey {
            fanout_index: 1,
            ..key
        };
        let mut call = Box::pin(quiet.after_done(ids::HTUI_FEAT_1, &step, &phase, &candidate));
        assert!(call.as_mut().now_or_never().is_none());
        other
            .notified()
            .now_or_never()
            .expect("the exact key stalled");
    }

    /// Blueprint F-A: the n-th reconcile records itself, signals, and never returns; the others
    /// answer as before.
    #[tokio::test]
    async fn the_nth_reconcile_stalls_after_recording_itself() {
        use futures::FutureExt as _;

        let isolator = FakeIsolator::new();
        let stalled = isolator.stall_nth_reconcile(2);
        let (first, second, third) = (StepId::new(), StepId::new(), StepId::new());
        assert!(isolator.reconcile(first, &[], &[]).now_or_never().is_some());
        let siblings = [third];
        let mut call = isolator.reconcile(second, &[], &siblings);
        assert!(
            call.as_mut().now_or_never().is_none(),
            "the second never answers"
        );
        stalled
            .notified()
            .now_or_never()
            .expect("and signalled first");
        drop(call);
        assert!(isolator.reconcile(third, &[], &[]).now_or_never().is_some());
        assert_eq!(
            isolator.reconciles(),
            [
                (first, Vec::new()),
                (second, vec![third]),
                (third, Vec::new())
            ]
        );
    }

    /// MOD-138 M3 T4: two stalls are pending at once, each on its own call, and the call
    /// between them answers.
    #[tokio::test]
    async fn two_reconcile_stalls_stall_their_own_calls() {
        use futures::FutureExt as _;

        let isolator = FakeIsolator::new();
        let (two, four) = (
            isolator.stall_nth_reconcile(2),
            isolator.stall_nth_reconcile(4),
        );
        let steps = [StepId::new(), StepId::new(), StepId::new(), StepId::new()];
        assert!(
            isolator
                .reconcile(steps[0], &[], &[])
                .now_or_never()
                .is_some()
        );
        let mut second = isolator.reconcile(steps[1], &[], &[]);
        assert!(
            second.as_mut().now_or_never().is_none(),
            "the second stalls"
        );
        assert!(
            isolator
                .reconcile(steps[2], &[], &[])
                .now_or_never()
                .is_some()
        );
        let mut fourth = isolator.reconcile(steps[3], &[], &[]);
        assert!(
            fourth.as_mut().now_or_never().is_none(),
            "the fourth stalls"
        );
        two.notified().now_or_never().expect("the second signalled");
        four.notified()
            .now_or_never()
            .expect("the fourth signalled");
        drop((second, fourth));
        assert_eq!(isolator.reconciles().len(), 4);
    }

    /// MOD-138 M3 T3 (F14): with `interleave_reconciles` two landings are in progress at once
    /// and the high-water mark says so; without it each answers at its first poll. The call log
    /// records both, and a stalled call dropped still closes its count.
    #[tokio::test]
    async fn interleaved_reconciles_overlap_and_are_counted() {
        use futures::FutureExt as _;

        let isolator = FakeIsolator::new();
        let (first, second) = (StepId::new(), StepId::new());
        assert!(isolator.reconcile(first, &[], &[]).now_or_never().is_some());
        assert_eq!(isolator.reconcile_high_water(), 1);
        isolator.interleave_reconciles();
        let mut one = isolator.reconcile(first, &[], &[]);
        let mut two = isolator.reconcile(second, &[], &[]);
        assert!(one.as_mut().now_or_never().is_none(), "it yields once");
        assert!(two.as_mut().now_or_never().is_none());
        assert_eq!(isolator.reconcile_high_water(), 2);
        drop((one, two));
        assert_eq!(
            isolator.call_log(),
            [
                IsolatorCall::Reconcile(first),
                IsolatorCall::Reconcile(first),
                IsolatorCall::Reconcile(second),
            ]
        );
        assert_eq!(isolator.in_place_overlaps(), 0, "no tree is in place");
    }

    /// MOD-138 M3 T3: an in-place step's window opening beside another step's open window is an
    /// overlap, in either order; two `worktree` windows are not.
    #[tokio::test]
    async fn an_in_place_window_beside_another_is_an_overlap() {
        let isolator = FakeIsolator::new();
        let run = htui_core::model::RunId::new();
        let (a, b, c) = (StepId::new(), StepId::new(), StepId::new());
        for step in [a, b] {
            isolator
                .prepare(run, step, &[], Isolation::Worktree, None)
                .await
                .expect("the fake prepares");
        }
        assert_eq!(isolator.in_place_overlaps(), 0, "two worktree trees");
        isolator
            .prepare(run, c, &[], Isolation::Local, None)
            .await
            .expect("the fake prepares");
        assert_eq!(
            isolator.in_place_overlaps(),
            1,
            "an in-place tree beside them"
        );
        for step in [a, b, c] {
            isolator
                .capture(step, &[])
                .await
                .expect("the fake captures");
        }
        isolator
            .prepare(run, a, &[], Isolation::Local, None)
            .await
            .expect("the fake prepares");
        assert_eq!(isolator.in_place_overlaps(), 1, "alone, no overlap");
    }
}
