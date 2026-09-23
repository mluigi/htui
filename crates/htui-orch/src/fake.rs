//! Deterministic test doubles behind `test-support`: [`FakeIsolator`], [`TestClock`],
//! `FakeGraphSource` and `FakeOrchestrator` over `MemStore` + `FakeDriver` + `FakeIsolator`.
//!
//! The determinism rules are `htui_agent::fake`'s (`crates/htui-agent/src/fake.rs:99-286`): nothing
//! reads the wall clock, nothing reads the filesystem, and every value a double hands back is a
//! function of what a test scripted into it. ANA-2 asks for this in as many words — the fake
//! orchestrator is what the Runs tab's `insta` snapshots run against, "so snapshots stay
//! byte-stable with no sleeps" (`docs/ANA-2.md:1764-1768`).
//!
//! Note for whoever reads `FakeGraphSource`: `MemStore::phase_agents` returns `Vec::new()`
//! unconditionally (`crates/htui-core/src/store/mem.rs:470-473`) and the demo fixture seeds no
//! `agent_box` row (`crates/htui-core/src/store/mem.rs:109`), so rungs 1 and 3 of ANA-2 §4.1's
//! candidate chain are both empty on `MemStore::demo()` — the fake carries an explicit per-phase candidates map as
//! its documented stand-in (plan D20, blueprint A-1).

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
use htui_agent::conformance::{Script, ScriptEvent, epoch};
use htui_agent::driver::{AgentDriver, AgentSession, DriverCaps, DriverFuture, SessionSpec};
use htui_agent::error::DriverError;
use htui_agent::event::{DoneEvent, DriverEvent, ErrorEvent, StopReason, UsageEvent};
use htui_agent::fake::{FAKE_AGENT_NAME, FakeDriver};
use htui_core::fixtures::ids;
use htui_core::model::{
    Agent, AgentBox, AgentId, BoxId, Document, DocumentId, Isolation, Item, ItemId, NewDocument,
    PhaseAgent, PhaseId, ProjectId, PromptTemplate, RepoId, ResolvedGraph, Run, RunId, RunStep,
    RunStepCommit, RunStepTree, SnapshotPhase, StepId, TIMESTAMPTZ_DIGITS, UserId, VerifyOutcome,
};
use htui_core::prompt::DiffBlock;
use htui_core::store::{MemStore, ReadStore, Result, WriteStore};
use uuid::Uuid;

use crate::command::{Command, CommandOutcome, EngineError};
use crate::engine::SessionKey;
use crate::graph::GraphSource;
use crate::isolate::{
    Clock, FanoutSlot, IsolateError, Isolator, IsolatorFuture, Prepared, PreparedTree,
};
use crate::verify::{Verifier, VerifierFuture, VerifyReport, VerifyRequest};

/// The root every synthetic tree path hangs from. Nothing ever creates it.
const FAKE_TREE_ROOT: &str = "/fake/trees";

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
    /// Every [`reconcile`](Isolator::reconcile) call, as `(winner, siblings)`, in call order — so a
    /// case can tell which siblings plan D54(d) handed the isolator.
    reconciles: Mutex<Vec<(StepId, Vec<StepId>)>>,
    /// Plan D28's per-repo lock, one for the whole fake: a `shared_serialized`
    /// [`prepare`](Isolator::prepare) waits on it, and holds it until that step's
    /// [`capture`](Isolator::capture) or the run's [`cleanup`](Isolator::cleanup).
    serial: Arc<tokio::sync::Mutex<()>>,
    /// The `shared_serialized` guard per step that holds [`serial`](Self::serial).
    held: Mutex<BTreeMap<StepId, tokio::sync::OwnedMutexGuard<()>>>,
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

    /// Take `step`'s `shared_serialized` guard, if it holds one; dropping it frees the lock.
    fn release(&self, step: StepId) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.held
            .lock()
            .expect("no panic holds the fake isolator's lock")
            .remove(&step)
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
            *self
                .prepares
                .lock()
                .expect("no panic holds the fake isolator's lock") += 1;
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
                drop(self.release(step));
                let guard = Arc::clone(&self.serial).lock_owned().await;
                self.held
                    .lock()
                    .expect("no panic holds the fake isolator's lock")
                    .insert(step, guard);
            }
            let cwd = format!("{FAKE_TREE_ROOT}/{run}/{step}");
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
                            path: format!("{cwd}/{repo}"),
                            base_ref: base.clone(),
                            dirty: false,
                        },
                        before_hash: base,
                    }
                })
                .collect();
            Ok(Prepared {
                trees,
                cwd: PathBuf::from(cwd),
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
            drop(self.release(step));
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
            let _ = trees;
            self.reconciles
                .lock()
                .expect("no panic holds the fake isolator's lock")
                .push((winner, siblings.to_vec()));
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

    /// Nothing was created, so nothing is removed — but the call is still made by the engine and is
    /// still counted, so a case can assert that cleanup happened exactly once, at run end.
    ///
    /// It no longer ticks the **hash** counter (blueprint F-K): a counter two verbs share cannot
    /// answer "how many cleanups", and the synthetic hashes are the other one's job.
    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            let _ = (run, trees);
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

/// A [`Clock`] a test moves, starting at the fake driver's own origin.
///
/// `htui_agent::conformance::epoch()` (`crates/htui-agent/src/conformance.rs:276`) is the instant
/// every `FakeDriver` envelope is stamped from, so starting here means a step's store rows and its
/// session rows share one origin and a snapshot of both reads as one timeline. [`advance`] is how a
/// step-deadline case elapses time; no case ever sleeps.
///
/// [`advance`]: TestClock::advance
#[derive(Debug)]
pub struct TestClock {
    now: Mutex<DateTime<Utc>>,
}

impl Default for TestClock {
    fn default() -> Self {
        Self::at(epoch())
    }
}

impl TestClock {
    /// A clock at `htui_agent::conformance::epoch()`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A clock at `now`, truncated like every other instant the walk hands a writer (plan D8).
    #[must_use]
    pub fn at(now: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(now.trunc_subsecs(TIMESTAMPTZ_DIGITS)),
        }
    }

    /// Move the clock forward (or back, for a test that wants to).
    pub fn advance(&self, by: TimeDelta) {
        let mut now = self
            .now
            .lock()
            .expect("no panic holds the test clock's lock");
        *now = (*now + by).trunc_subsecs(TIMESTAMPTZ_DIGITS);
    }

    /// Put the clock at an exact instant.
    pub fn set(&self, to: DateTime<Utc>) {
        *self
            .now
            .lock()
            .expect("no panic holds the test clock's lock") = to.trunc_subsecs(TIMESTAMPTZ_DIGITS);
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self
            .now
            .lock()
            .expect("no panic holds the test clock's lock")
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

/// The five inherent orchestration reads of `MemStore`, as the trait `graph.rs` defines
/// (blueprint F-N).
///
/// Implemented on `MemStore` itself rather than on `&MemStore`: a trait impl on the reference would
/// make the engine's `G` be `&MemStore` and every call site `&&MemStore`. The engine borrows `&G`,
/// so the source is used "over `&MemStore`" exactly as plan D19 says either way. Local trait,
/// foreign type — the orphan rule permits it, and it lives behind `test-support` because
/// `MemStore` is the fake harness's store; milestone 6's `Backend` impl lives in `htui`.
///
/// Each body calls the *inherent* method of the same name. Method resolution prefers inherent
/// candidates over trait ones, so this is delegation and not recursion — and
/// [`the trait's own unit test`](self) calls all five through the trait to prove it.
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
}

/// A [`GraphSource`] over `MemStore` with a per-phase candidates map (plan D20).
///
/// **Rungs 1 and 3 of ANA-2 §4.1's candidate chain are both structurally empty here**, which is why
/// this type exists. Rung 1 is `phase_agent`, and `MemStore` holds no such table — `phase_agents`
/// returns `Vec::new()` unconditionally (`crates/htui-core/src/store/mem.rs:470-473`). Rung 3 is
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
        GraphSource::prompt_template(self.store, project, name, version).await
    }

    async fn agent(&self, id: AgentId) -> Result<Option<Agent>> {
        GraphSource::agent(self.store, id).await
    }

    async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
        GraphSource::agent_boxes(self.store, box_id).await
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

/// What [`FakeOrchestrator`] scripts by: `(phase, attempt)` and, for one session of a group,
/// `(fanout_index, call)` (plan D68).
type ScriptKey = (String, i32, Option<(i32, u32)>);

/// `MemStore` + `FakeDriver` + [`FakeIsolator`] + [`TestClock`] + [`FakeGraphSource`], which is
/// ANA-2's own description of the fake orchestrator (`docs/ANA-2.md:1764-1766`).
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
    /// Keyed `(phase, attempt, None)` for a `(phase, attempt)` script and
    /// `(phase, attempt, Some((fanout_index, call)))` for one session's (plan D68).
    scripts: Mutex<BTreeMap<ScriptKey, ScriptedStep>>,
    candidates: Mutex<BTreeMap<String, Vec<(AgentId, String)>>>,
    after_done_advance: Mutex<Option<TimeDelta>>,
    default_script: ScriptedStep,
    caps: DriverCaps,
    box_id: BoxId,
    user: UserId,
    owner: Uuid,
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
        let store = MemStore::demo();
        let user = store
            .this_user()
            .expect("the demo fixture seeds exactly one `app_user`");
        Self {
            store,
            isolator: FakeIsolator::new(),
            verifier: FakeVerifier::new(),
            clock: TestClock::new(),
            scripts: Mutex::new(BTreeMap::new()),
            candidates: Mutex::new(BTreeMap::new()),
            after_done_advance: Mutex::new(None),
            default_script: ScriptedStep::done_with_output("scripted output"),
            caps: FakeDriver::full_caps(),
            box_id: ids::BOX,
            user,
            owner: Uuid::now_v7(),
        }
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
        let scripted = self.script_for_key(key);
        match scripted.spawn_failure {
            Some(reason) => Box::new(RefusingDriver {
                name: FAKE_AGENT_NAME.to_owned(),
                caps: self.caps,
                reason,
            }),
            None => Box::new(FakeDriver::new(FAKE_AGENT_NAME, self.caps, scripted.script)),
        }
    }

    /// Plan D13's stand-in for MOD-11's `document_write`, called between stage 4 and stage 5.
    ///
    /// Elapses any [`advance_after_done`](Self::advance_after_done) first — that is what "after the
    /// session and before the settle" means for a deadline case — then writes the scripted output
    /// document, or none. T4's `impl SessionSink for FakeOrchestrator` forwards to this and adds
    /// nothing.
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

    use super::{
        FakeGraphSource, FakeIsolator, FakeOrchestrator, GraphSource, ScriptedStep, TestClock,
    };
    use crate::engine::SessionKey;
    use crate::graph::{ResolveError, Resolved, resolve};
    use crate::isolate::{Clock as _, FanoutSlot, Isolator as _};

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

    /// `impl GraphSource for MemStore` delegates to the five inherent reads of the same names.
    ///
    /// Method resolution prefers an inherent candidate over a trait one, so the bodies are
    /// delegation — but "prefers" is the kind of rule that is worth a test rather than a comment,
    /// because getting it wrong is an infinite recursion and not a compile error.
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
            "`MemStore` holds no `phase_agent` table (blueprint H-3)"
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

    /// Plan D8: the origin is the fake driver's, the resolution is the column's, and time moves
    /// only when a test moves it.
    #[test]
    fn test_clock_starts_at_the_drivers_epoch_and_only_a_test_moves_it() {
        let clock = TestClock::new();
        assert_eq!(clock.now(), epoch());
        assert_eq!(clock.now(), clock.now(), "no wall clock is read");

        clock.advance(TimeDelta::seconds(2));
        assert_eq!(clock.now(), epoch() + TimeDelta::seconds(2));

        clock.set(epoch() + TimeDelta::nanoseconds(1_500));
        assert_eq!(
            clock.now(),
            epoch() + TimeDelta::microseconds(1),
            "every instant the walk hands a writer is microsecond-truncated"
        );
    }
}
