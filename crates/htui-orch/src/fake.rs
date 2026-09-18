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
//! unconditionally (`crates/htui-core/src/store/mem.rs:470-473`) and every demo agent is `enabled`
//! (`crates/htui-core/src/model/agent.rs:156`), so rungs 1 and 3 of ANA-2 §4.1's candidate chain
//! are both empty on `MemStore::demo()` — the fake carries an explicit per-phase candidates map as
//! its documented stand-in (plan D20, blueprint A-1).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
use htui_agent::conformance::epoch;
use htui_core::model::{
    Isolation, RepoId, RunId, RunStepCommit, RunStepTree, StepId, TIMESTAMPTZ_DIGITS,
};

use crate::isolate::{Clock, IsolateError, Isolator, IsolatorFuture, Prepared, PreparedTree};

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
    ) -> IsolatorFuture<'a, Prepared> {
        Box::pin(async move {
            let cwd = format!("{FAKE_TREE_ROOT}/{run}/{step}");
            let trees = scope
                .iter()
                .map(|repo| {
                    let base = format!("fake:base:{}", self.tick());
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
            })
        })
    }

    fn capture<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        Box::pin(async move { Ok(self.commits(step, trees)) })
    }

    /// Fan-out is milestone 4's, so the winner is the only candidate and a merge is an identity:
    /// this echoes [`capture`](Isolator::capture) and consumes a scripted value the same way.
    fn reconcile<'a>(
        &'a self,
        winner: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        Box::pin(async move { Ok(self.commits(winner, trees)) })
    }

    /// Nothing was created, so nothing is removed — but the call is still made by the engine and is
    /// still counted, so a case can assert that cleanup happened exactly once, at run end.
    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            let _ = (run, trees);
            self.tick();
            Ok::<(), IsolateError>(())
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

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use htui_agent::conformance::epoch;
    use htui_core::fixtures::ids;
    use htui_core::model::{Isolation, RepoId, StepId};

    use super::{FakeIsolator, TestClock};
    use crate::isolate::{Clock as _, Isolator as _};

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
            .prepare(ids::RUN_2, step, &scope, Isolation::Worktree)
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
            .prepare(ids::RUN_2, step, &[], Isolation::Local)
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
            .prepare(ids::RUN_2, step, &scope, Isolation::Copy)
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
