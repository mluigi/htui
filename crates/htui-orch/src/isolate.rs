//! The two seams the walk holds beside the store: stage 2's [`Isolator`] (plan D6) and plan D8's
//! [`Clock`].
//!
//! `Isolator` is the whole of this milestone's stage 2: ANA-2 §4.6's four verbs behind a trait, so
//! that milestone 3's `gix` implementation lands without re-cutting `engine.rs`, and so that the
//! conformance suite can drive a walk that touches no filesystem at all. The four `R-ORCH-8`
//! isolation modes are milestone 3's; nothing here knows what a worktree is.
//!
//! `Clock` is here rather than in `engine.rs` for a build-order reason worth stating: `fake.rs`'s
//! `TestClock` implements it and `engine.rs` is still a stub, so the trait has to exist in a module
//! T3 owns. `isolate.rs` is the one it belongs in — these are the crate's two *injected*
//! non-store seams, both `Send + Sync`, both held by the engine as a `&'a` borrow, and both
//! doubled by `fake.rs`. T4's `engine.rs` uses them from here; the blueprint's §5.1 placement is
//! recorded as moved rather than silently ignored.

use core::fmt;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use chrono::{DateTime, SubsecRound as _, Utc};
use htui_core::model::{
    Isolation, RepoId, RunId, RunStepCommit, RunStepTree, StepId, TIMESTAMPTZ_DIGITS,
};
use htui_core::prompt::DiffBlock;

pub mod copy;
pub mod git;
pub mod real;

pub use real::{GixIsolator, IsolatorConfig, RepoCheckout};

/// The boxed future every [`Isolator`] method returns.
///
/// The shape of `htui_agent::driver::DriverFuture` (`crates/htui-agent/src/driver.rs:37`) and for
/// the same reason: an `Isolator` is held as `&dyn Isolator` — milestone 3's `gix` implementation
/// and `fake::FakeIsolator` must be swappable behind one borrow — and a plain `async fn`
/// in a trait is not dyn-compatible. `async_trait` is not used anywhere in this workspace.
pub type IsolatorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, IsolateError>> + Send + 'a>>;

/// Why an isolation verb refused.
///
/// Deliberately not a `StoreError`: an isolator touches no store (ANA-2 §4.6's verbs are about
/// trees and commits, and the engine is what persists what they return), so its failures are about
/// git and the filesystem and nothing else.
#[derive(Debug, thiserror::Error)]
pub enum IsolateError {
    /// ANA-2 §4.6's "Refused when" column (`docs/ANA-2.md:912`): a mode the repo cannot satisfy —
    /// a dirty tree under `local`, a second claimant under `shared_serialized`, a repo with no
    /// checkout on this box.
    #[error("isolation refused: {0}")]
    Refused(String),
    /// Milestone 3's `gix` layer, which is the only thing that will ever construct this.
    #[error("git: {0}")]
    Git(String),
    /// A path could not be created, read or removed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// One tree the step will work in, with the hash the step starts from.
///
/// `tree` is persisted verbatim by `WriteStore::upsert_step_tree` and `before_hash` becomes the
/// `before_hash` of the step's `run_step_commit` row, so the pair is exactly what stage 2 owes the
/// store and nothing more.
// `RunStepTree` derives `PartialEq` and not `Eq` (`crates/htui-core/src/model/run.rs:424-426`), so
// neither of these two can derive `Eq` either; the blueprint's §4.1 sketch says `Eq` and is wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTree {
    /// The `run_step_tree` row, ready for `upsert_step_tree`.
    pub tree: RunStepTree,
    /// The repo's `HEAD` at the moment the tree was prepared.
    pub before_hash: String,
}

/// Stage 2's result: the trees, and the directory the agent session runs in.
///
/// `cwd` is separate from any tree's `path` because a run whose `repo_scope` names two repos has
/// two trees and one session: the session's working directory is their common parent, and a run
/// with an empty scope (the demo fixture holds no repos) has no trees at all but still needs one.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    /// One entry per repo in the run's `repo_scope`, in the order the scope names them.
    pub trees: Vec<PreparedTree>,
    /// `SessionSpec.cwd` for the step's session.
    pub cwd: PathBuf,
    /// `SessionSpec.extra_dirs` (`crates/htui-agent/src/driver.rs:257-258`): every tree that is
    /// not under [`cwd`](Prepared::cwd).
    ///
    /// A `worktree` or `copy` step puts all of its trees under one common parent and leaves this
    /// empty; a `shared_serialized` or `local` step runs in the primary repo's own checkout, and
    /// the second repo of its scope is somewhere else on the box entirely (plan D28). An agent
    /// that cannot read that tree cannot work in it.
    pub extra_dirs: Vec<PathBuf>,
}

/// MOD-4 milestone 4 D54(a): the candidate of a fan-out group a [`prepare`](Isolator::prepare) is
/// for.
///
/// `base` is the group's `HEAD` per repo, read once by the engine through [`Isolator::base`] before
/// the first candidate of the group is prepared, or re-derived from the group's own
/// `run_step_commit` rows once one has them (M2 D16). Every candidate starts from it, never from
/// the repository's current `HEAD`: a comparison between candidates that started from different
/// commits compares nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanoutSlot<'a> {
    /// The candidate's `fanout_index`, `0..width`.
    pub index: i32,
    /// The phase's `fan_out`: how many candidates the group holds.
    pub width: i32,
    /// The group's base per repo of the scope.
    pub base: &'a BTreeMap<RepoId, String>,
}

/// Plan D92/D99: what [`Isolator::reset`] did. **`refused` non-empty ⇒ nothing was reset.**
///
/// A refusal is not an [`IsolateError`]: the sweep reads it as "this tree must not be touched" and
/// takes D93's park, which names every refused tree, rather than failing the adjudication of the
/// run it was recovering.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResetReport {
    /// `(repo, head)`: `htui/<step>` names `head` after the call, written now or found there.
    pub labelled: Vec<(RepoId, String)>,
    /// `(repo, reason)`: `dirty_tree_not_reset: <path>`, `label_conflict: …` (D114) or
    /// `local_moved: …` (blueprint A-6).
    pub refused: Vec<(RepoId, String)>,
}

/// ANA-2 §4.6's four verbs plus milestone 4's two reads and milestone 5's two recovery verbs; the
/// four are named there verbatim (`docs/ANA-2.md:1769`), and all eight sit behind plan D6's seam.
///
/// **An isolator touches no store.** The engine persists what these return —
/// `upsert_step_tree` after [`prepare`](Isolator::prepare), `record_commits` after
/// [`capture`](Isolator::capture) and [`reconcile`](Isolator::reconcile) — which is what keeps the
/// trait implementable by a fake that creates no filesystem state and by milestone 3's `gix` layer
/// that creates a great deal of it, without either learning what a `WriteStore` is.
///
/// `Debug` is required because the engine derives it and because a refusal that cannot name its
/// isolator is a worse bug report.
pub trait Isolator: Send + Sync + fmt::Debug {
    /// Stage 2: one tree per repo in `scope` under `isolation`, each carrying its repo's `HEAD` as
    /// `before_hash`.
    ///
    /// An empty `scope` is not an error — it is the demo fixture's own shape — and yields no trees
    /// and a `cwd` the session can still start in.
    ///
    /// A real isolator makes this idempotent for `worktree` and `copy` (plan D38): a second call
    /// for the same `(run, step)` finds the tree it made and reports the same `before_hash`. The
    /// trait does not promise it — the fake mints a fresh one per call — and the engine calls it
    /// once (`crates/htui-orch/src/engine.rs:1069-1073`).
    ///
    /// `slot` is `None` for a `fan_out = 1` phase and for the judge. A slot fixes `before_hash` to
    /// `slot.base[repo]` rather than to the repository's `HEAD` (MOD-4 milestone 4 D54(a)); a
    /// `shared_serialized` candidate is reset to it first, and refused on a dirty checkout (D56,
    /// D72); `local` never sees one, because `fan_out > 1` is refused at snapshot time.
    fn prepare<'a>(
        &'a self,
        run: RunId,
        step: StepId,
        scope: &'a [RepoId],
        isolation: Isolation,
        slot: Option<FanoutSlot<'a>>,
    ) -> IsolatorFuture<'a, Prepared>;

    /// Stage 5: the `after_hash` of each tree, `None` for a tree the step committed nothing to.
    ///
    /// The `before_hash` of each returned row is the one [`prepare`](Isolator::prepare) reported,
    /// so `record_commits` can be handed the result verbatim.
    fn capture<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>>;

    /// MOD-4 milestone 4 D54(b): the `HEAD` of every repo in `scope`, keyed by repo; an empty
    /// scope is an empty map.
    ///
    /// Read once per fan-out group, before any candidate is prepared, because
    /// `shared_serialized`'s sibling 1 cannot prepare until sibling 0 captures — so the base cannot
    /// come from candidate 0's own `prepare`.
    fn base<'a>(&'a self, scope: &'a [RepoId]) -> IsolatorFuture<'a, BTreeMap<RepoId, String>>;

    /// MOD-4 milestone 4 D54(c), D55: the step's work as a [`DiffBlock`], over its `run_step_tree`
    /// rows and the `run_step_commit` rows recorded for them.
    ///
    /// `None` when no row committed anything, and when `git` is unusable on this box: the diff is
    /// advisory input to the judge and to the loop's `previous_diff`, so its absence degrades a
    /// prompt rather than failing anything. One committed repo gives its range, stat and patch;
    /// several give `<name>:<before>..<after>` ranges joined by `, ` and the stats and patches
    /// concatenated under a `# repo <name>` line each, in the order of `commits`.
    fn diff<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Option<DiffBlock>>;

    /// §4.6 step 4: the winning candidate's branch merged into the primary tree, one commit row
    /// per repo.
    ///
    /// What a winner's reconciliation *is* depends on the mode it ran under (plan D25): for
    /// `worktree` and `copy` it is a real `git merge --no-ff` into the primary checkout and the
    /// returned `after_hash` is the merge commit; for `shared_serialized` and `local` it is the
    /// identity, because the step committed into the primary tree as it went.
    ///
    /// `siblings` are the other candidates of the winner's fan-out group, and empty for
    /// `fan_out = 1` (MOD-4 milestone 4 D54(d)). For a fan-out winner, `worktree` and `copy` merge
    /// the winner alone and leave the losers' `htui/<step>` labels where they are;
    /// `shared_serialized` moves the checked-out branch to the winner's label, which is safe only
    /// when the checkout is clean and sits at the group's base or at a sibling's label (D56).
    fn reconcile<'a>(
        &'a self,
        winner: StepId,
        trees: &'a [RunStepTree],
        siblings: &'a [StepId],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>>;

    /// Run-terminal cleanup, **never** at step end (ANA-2 invariant 6, `docs/ANA-2.md:128-131`,
    /// and §4.6's own cleanup rule, `:994-996`): the next attempt of a superseded step reads the
    /// tree the last one left.
    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()>;

    /// ANA-2 §4.9 `:1298` as plan D92 reads it: an unfinished step's trees put back at their
    /// `base_ref` so the retry starts where the step did. `worktree`/`copy` rows are never touched
    /// (OQ-8): the retry prepares a tree of its own.
    ///
    /// `shared_serialized`/`local` rows are checked **all first** (OQ-7: live dirt refuses), then
    /// a `shared_serialized` row whose `HEAD` moved is labelled `htui/<step>` and
    /// `reset --hard <base_ref>`; a `local` row whose `HEAD` moved is refused (`local_moved`,
    /// blueprint A-6) and never labelled or reset. The check is
    /// all-or-nothing across rows: a [`ResetReport`] with a refusal in it reset nothing anywhere.
    fn reset<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, ResetReport>;

    /// D99: drop every guard any step of `run` holds and touch no tree; an abandoning walk's
    /// release.
    ///
    /// [`cleanup`](Isolator::cleanup) also releases them, but it removes trees, which would
    /// destroy what the process that adopts the run has to read.
    fn release<'a>(&'a self, run: RunId) -> IsolatorFuture<'a, ()>;
}

/// The one place an instant enters the walk (plan D8).
///
/// Every seam writer milestone 1 shipped takes its `at` from the caller, so the engine owns the
/// clock; and `docs/ANA-2.md:1766-1768` requires the harness's settle snapshots to be sleep-free,
/// so the engine must be able to be handed a clock a test moves. `fake::TestClock` is that clock
/// (both are behind `test-support`, so neither is linked from here: a doc link into a gated module
/// is `broken_intra_doc_links` in a plain `cargo doc`).
pub trait Clock: Send + Sync {
    /// Now, already truncated to the column's resolution.
    ///
    /// The truncation is the contract, not an implementation detail: `TIMESTAMPTZ` keeps
    /// microseconds and `chrono` keeps nanoseconds, so an untruncated instant round-trips
    /// differently through Postgres than through `MemStore` and the two backends disagree about a
    /// column neither changed (`crates/htui-core/src/model/run.rs:272`).
    fn now(&self) -> DateTime<Utc>;
}

/// The production [`Clock`]: `Utc::now()`, truncated.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{SubsecRound as _, Utc};
    use htui_core::model::TIMESTAMPTZ_DIGITS;

    use super::{Clock as _, SystemClock};

    /// Plan D8's whole point: a stamp that survives a Postgres round trip unchanged.
    #[test]
    fn system_clock_is_microsecond_truncated() {
        let now = SystemClock.now();
        assert_eq!(now, now.trunc_subsecs(TIMESTAMPTZ_DIGITS));
        assert_eq!(now.timestamp_subsec_nanos() % 1_000, 0);
        assert!(
            (Utc::now() - now).num_seconds().abs() < 5,
            "the production clock is the wall clock"
        );
    }
}
