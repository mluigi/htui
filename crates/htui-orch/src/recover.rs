//! ANA-2 §4.9's lease heartbeat and the sweep's pure half (plan D85, D86, D90, D91, D97).
//!
//! Nothing here reads or writes a store. The heartbeat takes its refresh as a closure (plan D106),
//! so the engine hands it `|ttl| store.refresh_lease(run, owner, ttl)` and a test hands it a
//! script; everything else is a function of rows the caller already read.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use htui_agent::event::{DoneEvent, StopReason};
use htui_core::model::{
    CommandRun, CommandRunStatus, Document, GraphSnapshot, Isolation, RepoId, RunStep,
    RunStepCommit, RunStepTree, SnapshotPhase, StepId, StepRole, StepStatus, VerifyOutcome,
};
use htui_core::store::StoreError;
use serde_json::Value;

use crate::gate::{Settle, SettleInput, Unmerged, settle};
use crate::verify::VERIFY_CLASS;

/// `app_setting` key of the lease's time to live, in seconds (`0003_orchestration.sql:135`).
pub const LEASE_TTL_KEY: &str = "lease_ttl_seconds";

/// `app_setting` key of the heartbeat's interval, in seconds (`0003_orchestration.sql:136`).
pub const LEASE_REFRESH_KEY: &str = "lease_refresh_seconds";

/// The TTL when [`LEASE_TTL_KEY`] is absent, not a positive integer or over a year: ANA-2 §10's
/// 120 s (PRD D2).
pub const DEFAULT_LEASE_TTL_SECONDS: i64 = 120;

/// The interval when [`LEASE_REFRESH_KEY`] is absent or not a positive integer: §10's 60 s.
pub const DEFAULT_LEASE_REFRESH_SECONDS: i64 = 60;

/// `run_step.gate_note` of a step whose trees the sweep must not reset (ANA-2 `:1299`, plan D93):
/// the engine writes it, and the walk reads it on a part (MOD-139 M2 D14) so the park holds.
pub const NOT_RESET: &str = "interrupted, tree not reset";

/// The longest TTL [`LeaseTimes::from_app`] reads as set: one year, so `now + ttl` never
/// overflows a `DateTime<Utc>`, and the store's own bound (MOD-40 blueprint B27).
const MAX_LEASE_TTL_SECONDS: i64 = htui_core::store::MAX_LEASE_TTL.num_seconds();

/// How long a lease lives and how often it is renewed (plan D85).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseTimes {
    /// The TTL this process hands every lease write. The store adds it to its own clock (MOD-40
    /// plan D10); the heartbeat adds it to tokio's monotonic instant it sent the write at, which
    /// is where its self-fence is measured from (MOD-41 plan D3).
    pub ttl: TimeDelta,
    /// How long the heartbeat sleeps between two refreshes; at most a third of `ttl` (D124).
    pub refresh: Duration,
}

impl LeaseTimes {
    /// Positive `i64` per key (a TTL at most one year), else the default; then `refresh` is
    /// clamped to at most `ttl / 3` (plan D124), in **milliseconds** so a 1 s TTL still beats at
    /// 333 ms. The defaults therefore beat every 40 s, not §10's 60 s.
    ///
    /// The heartbeat fences itself one `refresh` before its last written lease lapses (D122), so
    /// a third leaves one whole interval of fast retries (D123) between the first failed beat and
    /// the fence, and one more between the fence and the lapse.
    #[must_use]
    pub fn from_app(app: &BTreeMap<String, Value>) -> Self {
        // A TTL above a year is read as silence: a `TimeDelta` holds far more (~9.2e15 s) than
        // `clock.now() + ttl` can carry (`DateTime<Utc>` ends near year 262143), and that addition
        // panics on overflow.
        let ttl_seconds = app_positive(app, LEASE_TTL_KEY)
            .filter(|seconds| *seconds <= MAX_LEASE_TTL_SECONDS)
            .unwrap_or(DEFAULT_LEASE_TTL_SECONDS);
        let refresh_seconds =
            app_positive(app, LEASE_REFRESH_KEY).unwrap_or(DEFAULT_LEASE_REFRESH_SECONDS);
        let ttl = TimeDelta::seconds(ttl_seconds);
        let third = Duration::from_millis(ttl.num_milliseconds().unsigned_abs() / 3);
        let refresh = Duration::from_secs(refresh_seconds.unsigned_abs()).min(third);
        Self { ttl, refresh }
    }
}

/// One `app_setting` value as a positive `i64`: `graph.rs`'s private rule, restated here rather
/// than shared (blueprint F-U). Absent, `null`, a string, a bool, zero and a negative all mean
/// "not set", so a planted `0` falls back to the default rather than pinning the lease to nothing.
fn app_positive(app: &BTreeMap<String, Value>, key: &str) -> Option<i64> {
    app.get(key).and_then(Value::as_i64).filter(|n| *n > 0)
}

/// Why [`heartbeat`] returned. It never returns otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    /// A refresh touched zero rows: another orchestrator took the lease, and ANA-2 `:1280-1282`
    /// has this one abandon the run without writing further.
    Abandoned,
    /// Plan D122: refreshes kept failing until the last lease this process wrote was within one
    /// `refresh` of lapsing. The walk cannot prove it still holds the run, so it stops before
    /// another box's sweep may adopt it.
    Expired,
}

/// Plan D123's floor on the sooner retry after a failed refresh.
const MIN_RETRY: Duration = Duration::from_secs(1);

/// Sleep, then `refresh(times.ttl)`: `Ok(true)` → the lease runs at least to the instant the
/// refresh was sent plus `times.ttl`, sleep `times.refresh`, loop; `Ok(false)` → `Abandoned`;
/// `Err(e)` → `tracing::warn!`, sleep `times.refresh / 4` (at least 1 s, D123) and loop (D86,
/// D103). Never returns otherwise.
///
/// **The self-fence (plan D122; MOD-40 plan OQ-2).** The store stamps a lease with its own clock
/// and does not say what it wrote (plan D10). The heartbeat instead tracks, on **tokio's
/// monotonic clock** (MOD-41 plan D3, MOD-40 blueprint F-38), the instant by which the last lease
/// it wrote successfully lapses at the earliest: the instant it sent that write plus the TTL. The
/// store read its clock after that send, so its expiry is no earlier, and a span measured on one
/// clock does not care how far apart two hosts' clocks are. It starts from `written`, the instant
/// the caller's `claim_run`, `take_lease` or the sweep's `renew_lease` was sent plus the TTL
/// (plan D143), which may lie before `now + times.ttl` at this call when the caller wrote other
/// rows in between; each successful refresh moves it to its own send instant plus the TTL. Once
/// refreshes have failed until tokio's `now >= last - times.refresh`, it returns `Expired`: the
/// walk cannot prove its lease, and one `refresh` before it lapses is where it stops, so no other
/// box's sweep adopts a run this walk still writes to. Neither the first beat nor a retry sleeps
/// past that fence, and a refresh still pending at the fence (a stuck connection) is dropped
/// there and read as `Expired` too. A wall-clock step while a lease is held can neither extend
/// nor cut the fence.
///
/// A single store error is not a taken lease: offline, the lease is left to expire and the
/// reconnect sweep adjudicates (`docs/ANA-2.md:1325-1336`). Needs a runtime with the time driver
/// (H-3).
pub async fn heartbeat<F, Fut>(
    mut refresh: F,
    written: tokio::time::Instant,
    times: LeaseTimes,
) -> Heartbeat
where
    F: FnMut(TimeDelta) -> Fut,
    Fut: Future<Output = Result<bool, StoreError>>,
{
    use tokio::time::Instant;

    let retry = (times.refresh / 4).max(MIN_RETRY);
    let ttl = times.ttl.to_std().unwrap_or(Duration::ZERO);
    // Blueprint F-4: an instant cannot go below the platform's origin, nor past its end; either
    // failure fences now, so the first check reads `Expired` rather than panicking.
    let fence_of = |until: Instant| {
        until
            .checked_sub(times.refresh)
            .unwrap_or_else(Instant::now)
    };
    let mut fence = fence_of(written);
    // Plan D143: a lease written well before this call fences sooner than one `refresh` away, and
    // the first beat must not sleep past it.
    let mut interval = times
        .refresh
        .min(fence.saturating_duration_since(Instant::now()));
    loop {
        tokio::time::sleep(interval).await;
        let sent = Instant::now();
        // Plan D122: a refresh that hangs is given up at the fence, not waited on past it.
        let left = fence.saturating_duration_since(sent);
        let Ok(answer) = tokio::time::timeout(left, refresh(times.ttl)).await else {
            tracing::warn!("lease refresh still pending at the fence; the walk stops");
            return Heartbeat::Expired;
        };
        match answer {
            Ok(true) => {
                fence = sent.checked_add(ttl).map_or_else(Instant::now, fence_of);
                interval = times.refresh;
            }
            Ok(false) => return Heartbeat::Abandoned,
            Err(error) => {
                let now = Instant::now();
                if now >= fence {
                    tracing::warn!(%error, "lease refresh failed at the fence; the walk stops");
                    return Heartbeat::Expired;
                }
                tracing::warn!(%error, "lease refresh failed; retrying sooner");
                interval = retry.min(fence - now);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Adjudicating one `running` step (plan D90, D92, D93)
// ---------------------------------------------------------------------------------------------

/// The `fanout_index` of a fan-out slot's judge row (`docs/ANA-2.md` §4.5); `status.rs` keeps
/// its own copy private.
const JUDGE_FANOUT_INDEX: i32 = -1;

/// Which of a slot's rows a step is, which decides what the sweep does with its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// The one row of a `fan_out = 1` phase.
    Plain,
    /// One candidate of a fanned-out slot (`fanout_index >= 0`, `fan_out > 1`).
    Candidate,
    /// The slot's judge (`fanout_index = -1`).
    Judge,
    /// MOD-139 M2 D14: one part of a divided node (`role = part`). Classified by the plain
    /// artefact test; a finished part is `done` and never landed, an unfinished one is reset and
    /// failed `interrupted` for its node's next parts round to retry.
    Part,
}

impl StepKind {
    /// A `part` row → Part (MOD-139 M2 D14, first); `fanout_index == -1` → Judge;
    /// `phase.fan_out > 1` → Candidate; else Plain.
    #[must_use]
    pub const fn of(step: &RunStep, phase: &SnapshotPhase) -> Self {
        if matches!(step.role, StepRole::Part) {
            Self::Part
        } else if step.fanout_index == JUDGE_FANOUT_INDEX {
            Self::Judge
        } else if phase.fan_out > 1 {
            Self::Candidate
        } else {
            Self::Plain
        }
    }
}

/// What a `running` step's rows say happened to it before the crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adjudication {
    /// D90. `verify*` are what `finish_step` must write when `finished_at` is `NULL`.
    Finished {
        /// [`verify_of`]'s outcome.
        verify: Option<VerifyOutcome>,
        /// [`verify_of`]'s exit code.
        verify_exit_code: Option<i32>,
    },
    /// D92: every row is resettable (vacuously, with no rows).
    Reset,
    /// D93: `(repo, path, base_ref)` of every row, for the note.
    NeverReset {
        /// Every `run_step_tree` row of the step, not only the dirty ones.
        trees: Vec<(RepoId, String, String)>,
    },
}

/// D90/D92/D93 over one `running` step. Seven arguments, which is clippy's limit.
///
/// Finished when the output document exists **and** either `finished_at` is set or the capture
/// landed. For a plain step, a judge or a part (MOD-139 M2 D14) the capture landed when every
/// repo of `run_scope` has a
/// `run_step_commit` row and at least one of them has an `after_hash`: stage 2 gives every scope
/// repo a row, and stage 5's capture batch lands in one transaction, so one `after_hash` proves
/// the batch. Every row without one stays unfinished, since a capture that never landed and one
/// that changed nothing look alike and retrying is safe. A candidate keeps the every-repo rule:
/// every scope repo needs an `after_hash`. Both are vacuous for an empty scope, the demo
/// fixture's. Otherwise resettable when every tree row is `worktree | copy`, or
/// `shared_serialized | local` with `dirty = false`. The rows passed are the step's own. A judge
/// is classified as well; the engine ignores the answer (D95).
#[must_use]
pub fn classify(
    step: &RunStep,
    phase: &SnapshotPhase,
    run_scope: &[RepoId],
    trees: &[RunStepTree],
    commits: &[RunStepCommit],
    output_present: bool,
    command_runs: &[CommandRun],
) -> (StepKind, Adjudication) {
    let kind = StepKind::of(step, phase);
    let row = |repo: &RepoId| commits.iter().find(|row| row.repo_id == *repo);
    let changed = |repo: &RepoId| row(repo).is_some_and(|row| row.after_hash.is_some());
    let captured = match kind {
        StepKind::Candidate => run_scope.iter().all(changed),
        StepKind::Plain | StepKind::Judge | StepKind::Part => {
            run_scope.iter().all(|repo| row(repo).is_some())
                && (run_scope.is_empty() || run_scope.iter().any(changed))
        }
    };
    if output_present && (step.finished_at.is_some() || captured) {
        let (verify, verify_exit_code) = verify_of(step, command_runs);
        return (
            kind,
            Adjudication::Finished {
                verify,
                verify_exit_code,
            },
        );
    }
    if trees.iter().all(resettable) {
        return (kind, Adjudication::Reset);
    }
    let trees = trees
        .iter()
        .map(|row| (row.repo_id, row.path.clone(), row.base_ref.clone()))
        .collect();
    (kind, Adjudication::NeverReset { trees })
}

/// ANA-2 §4.9 `:1298`: a separate tree is always resettable (the reset touches nothing, OQ-8); a
/// checkout used in place is resettable only when stage 2 found it clean (M3 D24).
const fn resettable(row: &RunStepTree) -> bool {
    match row.mode {
        Isolation::Worktree | Isolation::Copy => true,
        Isolation::SharedSerialized | Isolation::Local => !row.dirty,
    }
}

/// D91: the step's verify from rows. `finished_at` set → the row's own columns. Otherwise the last
/// `command_run` whose `class == VERIFY_CLASS`: `Done` + `Some(0)` → `Pass`; `Done` + other →
/// `Fail`; `Failed` → `Unavailable`; `Queued`/`Running` or none → `None`.
///
/// "Last" is the latest `queued_at`, the later row on a tie. The class filter is D116's: MOD-11's
/// queue writes other classes against the same step. A `cancelled` row never finished and reads
/// as `None`, like `running`. `Unavailable` carries no exit code, as the live walk writes it.
#[must_use]
pub fn verify_of(
    step: &RunStep,
    command_runs: &[CommandRun],
) -> (Option<VerifyOutcome>, Option<i32>) {
    if step.finished_at.is_some() {
        return (step.verify_outcome, step.verify_exit_code);
    }
    let last = command_runs
        .iter()
        .filter(|row| row.class == VERIFY_CLASS)
        .max_by_key(|row| row.queued_at);
    match last.map(|row| (row.status, row.exit_code)) {
        Some((CommandRunStatus::Done, Some(0))) => (Some(VerifyOutcome::Pass), Some(0)),
        Some((CommandRunStatus::Done, exit_code)) => (Some(VerifyOutcome::Fail), exit_code),
        Some((CommandRunStatus::Failed, _)) => (Some(VerifyOutcome::Unavailable), None),
        Some((
            CommandRunStatus::Queued | CommandRunStatus::Running | CommandRunStatus::Cancelled,
            _,
        ))
        | None => (None, None),
    }
}

/// D91: `gate::settle` with `driver: Ok(DoneEvent { stop_reason: EndTurn })`, `cap_breach: None`,
/// `deadline_seconds: None`, `deadline_cut: false` (recovery never has a live timer).
///
/// The deadline is dropped because the sweep's clock is not the step's, and the cap because the
/// recorder's breach is not durable (plan R-13); `now` is therefore inert. `verify` is
/// [`verify_of`]'s, and `reads_verdict` is `GraphSnapshot::reads_verdict` of the step's node (an
/// outgoing `changes_requested` edge, MOD-138 M2 D7), as stage 5 passes it. `unmerged` is a
/// recovered integrate's post-check over its recorded commits (MOD-139 M3 D8), `None` for every
/// other step.
#[must_use]
pub fn resettle(
    output: Option<&Document>,
    verify: Option<VerifyOutcome>,
    reads_verdict: bool,
    started_at: DateTime<Utc>,
    now: DateTime<Utc>,
    unmerged: Option<Unmerged<'_>>,
) -> Settle {
    let driver = Ok(DoneEvent {
        stop_reason: StopReason::EndTurn,
    });
    settle(&SettleInput {
        driver: &driver,
        cap_breach: None,
        cap_batch: None,
        started_at,
        now,
        deadline_seconds: None,
        deadline_cut: false,
        output,
        verify_outcome: verify,
        reads_verdict,
        unmerged,
    })
}

// ---------------------------------------------------------------------------------------------
// The frontiers (MOD-138 M3 D6; plan D97, blueprint A-3)
// ---------------------------------------------------------------------------------------------

/// MOD-138 M3 D6 (Q-1, A8, A9): every done winner that may not have landed, in
/// forward-topological order (ties by position). It replaced M2's position rule (plan D97 as
/// redefined, C129: the done winner at the highest position, when every row after it is
/// retired) in T4, which also removed that rule.
///
/// - **Members.** The done winner (`winner_at`: latest attempt, `selected`, or index 0 when
///   `fan_out = 1`) of each walked node.
/// - **The successor rule (Q-1, A8).** A winner is dropped when some forward successor of its
///   node has a row (any `fanout_index`, the judge included) that is neither `superseded` nor
///   `cancelled` and whose `started_at` is at or after the winner's `finished_at`. The walk
///   creates a successor only once its predecessor's future, landing included, returned, so such
///   a row proves the landing. An unstarted `pending` row proves nothing on its own, nor does an
///   older row a loop left outside its region: the winner re-lands, which is idempotent
///   (`merge_of`). A `running` successor drops only its own predecessor (A-3 per node).
/// - **A retried successor (T4, A8 narrowed).** A `pending` successor row beside another row of
///   that node, retired ones included, that started at or after the winner's `finished_at`
///   proves the landing too: that earlier attempt could start only once the winner landed, and a
///   human's retry (or the review loop) then retired it and admitted the `pending` one. This is
///   the position rule's reading of a live row after the winner, which the pre-M3 hand-back
///   retry pins (`a_retry_of_an_awaiting_step_handed_back_is_walked_by_the_adopter`). Retired rows
///   alone still prove nothing: under a frozen clock a loop's cancelled row ties with the re-run
///   winner it predates.
/// - **The in-place rule (A9).** A member whose phase is `local` or `shared_serialized` is kept
///   only when no done winner of the run, in any mode and member or not, has a later
///   `finished_at`: a later landing moved `HEAD`, so the earlier in-place node landed, and
///   re-landing it would refuse `primary_moved` (or, for `local`, record another node's commit).
///   A tie keeps it.
/// - **The landing rule (A9 over landings, T1 repair 1).** A done winner a loop later retired is
///   no longer a winner, yet its landing moved `HEAD` too. So an in-place member of a
///   `fan_out = 1` node is also dropped when any other row of the run, in any status and at any
///   position, has a `started_at` strictly after the member's `finished_at`. Under A3 nothing
///   that starts a session is admitted while an in-place future is in flight, and that future
///   lands before it returns, so such a row started after the landing. A tie keeps the member,
///   as A9's. A fanned-out winner lands at its selection, which no column stamps, so this rule
///   leaves it to the two above.
#[must_use]
pub fn frontiers(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Vec<StepId> {
    let graph = crate::walk::Graph::of(snapshot);
    let winners: Vec<(i32, &SnapshotPhase, &RunStep)> = graph
        .order()
        .into_iter()
        .filter_map(|position| {
            let phase = snapshot
                .phases
                .iter()
                .find(|phase| !phase.palette && phase.position == position)?;
            Some((position, phase, winner_at(phase, steps)?))
        })
        .collect();
    let proves = |successor: i32, winner: &RunStep| {
        let finished = winner.finished_at.unwrap_or(DateTime::<Utc>::MIN_UTC);
        let rows = || steps.iter().filter(|row| row.position == successor);
        let started_after =
            |row: &&RunStep| row.started_at.is_some_and(|started| started >= finished);
        rows().any(|row| {
            !matches!(row.status, StepStatus::Superseded | StepStatus::Cancelled)
                && started_after(&row)
        }) || (rows().any(|row| row.status == StepStatus::Pending)
            && rows().any(|row| started_after(&row)))
    };
    let landed_later = |winner: &RunStep| {
        winners
            .iter()
            .any(|(_, _, other)| other.finished_at > winner.finished_at)
    };
    let started_after = |phase: &SnapshotPhase, winner: &RunStep| {
        let finished = winner.finished_at.unwrap_or(DateTime::<Utc>::MIN_UTC);
        phase.fan_out == 1
            && steps.iter().any(|row| {
                row.id != winner.id && row.started_at.is_some_and(|started| started > finished)
            })
    };
    winners
        .iter()
        .filter(|(position, _, winner)| {
            !graph
                .forward_successors(*position)
                .into_iter()
                .any(|successor| proves(successor, winner))
        })
        .filter(|(_, phase, winner)| {
            !matches!(
                phase.isolation,
                Isolation::Local | Isolation::SharedSerialized
            ) || !(landed_later(winner) || started_after(phase, winner))
        })
        .map(|(_, _, winner)| winner.id)
        .collect()
}

/// The `done` winner of `phase`'s latest attempt, if it has one.
///
/// The attempt is the highest over every candidate row (`fanout_index >= 0`), as `status.rs`'s
/// slot cursor reads it; a judge row is never a winner. A fanned-out slot's index 0 wins only
/// when `selected`: until the selection is written the slot has no winner to reconcile.
fn winner_at<'a>(phase: &SnapshotPhase, steps: &'a [RunStep]) -> Option<&'a RunStep> {
    let mut candidates = steps
        .iter()
        .filter(|step| step.position == phase.position && step.fanout_index >= 0);
    let attempt = candidates.clone().map(|step| step.attempt).max()?;
    candidates.find(|step| {
        step.attempt == attempt
            && step.status == StepStatus::Done
            && (step.selected == Some(true) || (phase.fan_out == 1 && step.fanout_index == 0))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
    use htui_core::clock::{Clock as _, TestClock, epoch};
    use htui_core::store::StoreError;
    use serde_json::{Value, json};
    use tokio::time::Instant;

    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        BoxId, CommandRun, CommandRunId, CommandRunStatus, Document, DocumentId, EdgeKind,
        GraphSnapshot, Isolation, ItemId, RepoId, RunStep, RunStepCommit, RunStepTree,
        SnapshotPhase, StepId, StepRole, StepStatus, UserId, VerifyOutcome,
    };

    use crate::gate::{Settle, StepFailure};
    use crate::recover::{
        Adjudication, Heartbeat, LeaseTimes, StepKind, classify, frontiers, heartbeat, resettle,
        verify_of,
    };
    use crate::walk::tests as walk_fx;

    // -----------------------------------------------------------------------------------------
    // Lease times
    // -----------------------------------------------------------------------------------------

    fn app(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn lease_times_read_the_two_app_settings() {
        let times = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(300)),
            ("lease_refresh_seconds", json!(90)),
        ]));
        assert_eq!(times.ttl, TimeDelta::seconds(300));
        assert_eq!(times.refresh, Duration::from_secs(90));
    }

    /// §10's 120 s and 60 s, and then D124's clamp: 60 s is above a third of 120 s, so the
    /// heartbeat beats every 40 s.
    #[test]
    fn lease_times_fall_back_to_120_and_60_clamped_to_40() {
        for silent in [None, Some(json!(0)), Some(json!(-5)), Some(json!("120"))] {
            let settings = silent.map_or_else(BTreeMap::new, |value| {
                app(&[
                    ("lease_ttl_seconds", value.clone()),
                    ("lease_refresh_seconds", value),
                ])
            });
            let times = LeaseTimes::from_app(&settings);
            assert_eq!(times.ttl, TimeDelta::seconds(120), "{settings:?}");
            assert_eq!(times.refresh, Duration::from_secs(40), "{settings:?}");
        }
    }

    /// Plan D124: a refresh above a third of the TTL is clamped to that third, in milliseconds,
    /// so the self-fence (D122) always has two retry windows before the lease lapses.
    #[test]
    fn a_refresh_above_a_third_of_the_ttl_is_clamped() {
        let equal = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(60)),
            ("lease_refresh_seconds", json!(60)),
        ]));
        assert_eq!(equal.ttl, TimeDelta::seconds(60));
        assert_eq!(equal.refresh, Duration::from_secs(20));

        let half = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(90)),
            ("lease_refresh_seconds", json!(45)),
        ]));
        assert_eq!(
            half.refresh,
            Duration::from_secs(30),
            "half is above a third"
        );

        let third = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(90)),
            ("lease_refresh_seconds", json!(30)),
        ]));
        assert_eq!(third.refresh, Duration::from_secs(30), "a third is kept");

        let above = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(1)),
            ("lease_refresh_seconds", json!(5)),
        ]));
        assert_eq!(above.ttl, TimeDelta::seconds(1));
        assert_eq!(above.refresh, Duration::from_millis(333));
    }

    #[test]
    fn a_ttl_no_timestamp_can_carry_reads_as_silence() {
        // Fits a `TimeDelta` (up to ~9.2e15 s) but not `now + ttl` (`DateTime<Utc>` ends near
        // year 262143), so the heartbeat's addition would panic.
        for huge in [10_000_000_000_000_i64, i64::MAX] {
            let times = LeaseTimes::from_app(&app(&[("lease_ttl_seconds", json!(huge))]));
            assert_eq!(times.ttl, TimeDelta::seconds(120), "{huge}");
            assert_eq!(times.refresh, Duration::from_secs(40), "{huge}");
        }

        // One year is the longest TTL read as set.
        let year = 365 * 24 * 60 * 60;
        let times = LeaseTimes::from_app(&app(&[("lease_ttl_seconds", json!(year))]));
        assert_eq!(times.ttl, TimeDelta::seconds(year));
        let over = LeaseTimes::from_app(&app(&[("lease_ttl_seconds", json!(year + 1))]));
        assert_eq!(over.ttl, TimeDelta::seconds(120));
    }

    // -----------------------------------------------------------------------------------------
    // Heartbeat
    // -----------------------------------------------------------------------------------------

    /// The default 120 s TTL, and its refresh clamped to 40 s (D124).
    fn default_times() -> LeaseTimes {
        LeaseTimes::from_app(&BTreeMap::new())
    }

    /// `times.ttl` as the span a lease written at a tokio instant runs for (plan D3).
    fn ttl_of(times: LeaseTimes) -> Duration {
        times.ttl.to_std().expect("a test's TTL is positive")
    }

    /// A refresh closure answering `script` in order (then `Ok(true)` for ever), recording each
    /// TTL beside tokio's instant at the call.
    #[allow(clippy::type_complexity)]
    fn scripted(
        script: Vec<Result<bool, StoreError>>,
    ) -> (
        Arc<Mutex<Vec<(Instant, TimeDelta)>>>,
        impl FnMut(TimeDelta) -> std::future::Ready<Result<bool, StoreError>>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let mut script = script.into_iter();
        let refresh = move |ttl| {
            seen.lock().unwrap().push((Instant::now(), ttl));
            std::future::ready(script.next().unwrap_or(Ok(true)))
        };
        (calls, refresh)
    }

    /// `n` store errors in a row.
    fn unreachable(n: usize) -> Vec<Result<bool, StoreError>> {
        (0..n)
            .map(|_| Err(StoreError::Unreachable("socket closed".to_owned())))
            .collect()
    }

    /// The whole seconds after `start` of every recorded call.
    fn beats(calls: &Mutex<Vec<(Instant, TimeDelta)>>, start: Instant) -> Vec<u64> {
        calls
            .lock()
            .unwrap()
            .iter()
            .map(|(at, _)| (*at - start).as_secs())
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_refreshes_every_interval() {
        let start = Instant::now();
        let times = default_times();
        let (calls, refresh) = scripted(vec![Ok(true), Ok(true), Ok(true)]);
        let outcome = tokio::time::timeout(
            Duration::from_secs(125),
            heartbeat(refresh, start + ttl_of(times), times),
        )
        .await;
        assert!(
            outcome.is_err(),
            "the heartbeat never returns on `Ok(true)`"
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "a beat at 40 s, 80 s and 120 s");
        for (beat, (at, ttl)) in (1..).zip(calls.iter()) {
            assert_eq!(*at, start + Duration::from_secs(40 * beat));
            assert_eq!(
                *ttl, times.ttl,
                "beat {beat}: every refresh asks for the configured TTL"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_returns_abandoned_on_a_zero_row_refresh() {
        let start = Instant::now();
        let (calls, refresh) = scripted(vec![Ok(true), Ok(false)]);
        let outcome = heartbeat(refresh, start + ttl_of(default_times()), default_times()).await;
        assert_eq!(outcome, Heartbeat::Abandoned);
        assert_eq!(start.elapsed(), Duration::from_secs(80));
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    /// A store error is not a taken lease: the heartbeat retries (sooner, D123) and a zero-row
    /// answer inside the fence is still `Abandoned`.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_survives_a_store_error() {
        let start = Instant::now();
        let (calls, refresh) = scripted(vec![
            Err(StoreError::Unreachable("socket closed".to_owned())),
            Err(StoreError::Backend("deadlock".to_owned())),
            Ok(false),
        ]);
        let outcome = heartbeat(refresh, start + ttl_of(default_times()), default_times()).await;
        assert_eq!(outcome, Heartbeat::Abandoned);
        assert_eq!(
            beats(&calls, start),
            [40, 50, 60],
            "abandoned on the third beat"
        );
    }

    /// Plan D122: refreshes that keep failing stop the heartbeat one `refresh` before the last
    /// lease it wrote lapses. The lease the caller wrote runs to 120 s, so the fence is at 80 s,
    /// and the TTL is never reached.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_expires_before_the_lease_lapses_when_every_refresh_fails() {
        let start = Instant::now();
        let times = default_times();
        let (calls, refresh) = scripted(unreachable(100));
        let outcome = tokio::time::timeout(
            Duration::from_millis(600_500),
            heartbeat(refresh, start + ttl_of(times), times),
        )
        .await
        .expect("the heartbeat fences itself instead of beating for ever");
        assert_eq!(outcome, Heartbeat::Expired);
        assert_eq!(start.elapsed(), Duration::from_secs(80));
        assert!(
            Instant::now() < start + ttl_of(times),
            "stopped before the lease lapsed"
        );
        assert_eq!(beats(&calls, start), [40, 50, 60, 70, 80]);
    }

    /// Plan D143 (review L-a): the fence starts from the lease the caller actually wrote, not
    /// from the heartbeat's own start. That lease was written 30 s before the heartbeat began, so
    /// it lapses at 90 s and the fence stands at 50 s, where every refresh failing stops the walk.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_fences_from_the_until_its_caller_wrote() {
        let start = Instant::now();
        let times = default_times();
        let written = start + ttl_of(times) - Duration::from_secs(30);
        let (calls, refresh) = scripted(unreachable(100));
        let outcome = tokio::time::timeout(
            Duration::from_millis(600_500),
            heartbeat(refresh, written, times),
        )
        .await
        .expect("the heartbeat fences itself instead of beating for ever");
        assert_eq!(outcome, Heartbeat::Expired);
        assert_eq!(start.elapsed(), Duration::from_secs(50));
        assert!(
            Instant::now() < written,
            "stopped before the written lease lapsed"
        );
        assert_eq!(beats(&calls, start), [40, 50]);
    }

    /// Plan D143: a lease written long before the heartbeat began has its fence sooner than one
    /// `refresh` away, so the first beat lands at the fence rather than past it. The lease runs to
    /// 50 s, the fence stands at 10 s, and a first refresh that fails there stops the walk.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_first_beat_never_sleeps_past_the_fence() {
        let start = Instant::now();
        let times = default_times();
        let written = start + Duration::from_secs(50);
        let (calls, refresh) = scripted(unreachable(100));
        let outcome = tokio::time::timeout(
            Duration::from_millis(600_500),
            heartbeat(refresh, written, times),
        )
        .await
        .expect("the heartbeat fences itself instead of beating for ever");
        assert_eq!(outcome, Heartbeat::Expired);
        assert_eq!(start.elapsed(), Duration::from_secs(10));
        assert_eq!(beats(&calls, start), [10]);
    }

    /// Plan D122: a refresh that neither answers nor fails (a stuck connection) is raced against
    /// the fence, so the heartbeat still returns `Expired` there rather than waiting inside it
    /// while the lease lapses. The lease the caller wrote runs to 120 s: the one refresh starts at
    /// 40 s and is given up at the 80 s fence.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_expires_at_the_fence_while_a_refresh_hangs() {
        let start = Instant::now();
        let times = default_times();
        let calls = Mutex::new(0_u32);
        let refresh = |_ttl| {
            *calls.lock().unwrap() += 1;
            std::future::pending::<Result<bool, StoreError>>()
        };
        let outcome = tokio::time::timeout(
            Duration::from_millis(600_500),
            heartbeat(refresh, start + ttl_of(times), times),
        )
        .await
        .expect("the heartbeat gives up a hung refresh at the fence");
        assert_eq!(outcome, Heartbeat::Expired);
        assert_eq!(start.elapsed(), Duration::from_secs(80));
        assert_eq!(*calls.lock().unwrap(), 1, "the one refresh never answered");
    }

    /// Plan D122: the fence follows the last `until` written **successfully**. A success at 50 s
    /// writes 170 s, so the next run of failures fences at 130 s.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_a_successful_refresh_moves_the_fence() {
        let start = Instant::now();
        let mut script = unreachable(1);
        script.push(Ok(true));
        script.extend(unreachable(100));
        let (calls, refresh) = scripted(script);
        let outcome = tokio::time::timeout(
            Duration::from_millis(600_500),
            heartbeat(refresh, start + ttl_of(default_times()), default_times()),
        )
        .await
        .expect("the heartbeat fences itself instead of beating for ever");
        assert_eq!(outcome, Heartbeat::Expired);
        assert_eq!(start.elapsed(), Duration::from_secs(130));
        assert_eq!(beats(&calls, start), [40, 50, 90, 100, 110, 120, 130]);
    }

    /// Plan D123: after an error the next beat comes `refresh / 4` later (10 s of 40 s), and after
    /// a success the interval is the full `refresh` again.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_retries_sooner_after_a_store_error() {
        let start = Instant::now();
        let mut script = unreachable(1);
        script.push(Ok(true));
        let (calls, refresh) = scripted(script);
        let outcome = tokio::time::timeout(
            Duration::from_secs(95),
            heartbeat(refresh, start + ttl_of(default_times()), default_times()),
        )
        .await;
        assert!(outcome.is_err(), "the lease is held again");
        assert_eq!(beats(&calls, start), [40, 50, 90]);
    }

    /// Plan D123's floor: a refresh whose quarter is under a second retries after one second.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_retries_after_at_least_a_second() {
        let start = Instant::now();
        let times = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(9)),
            ("lease_refresh_seconds", json!(2)),
        ]));
        assert_eq!(times.refresh, Duration::from_secs(2));
        let mut script = unreachable(1);
        script.push(Ok(true));
        let (calls, refresh) = scripted(script);
        let outcome = tokio::time::timeout(
            Duration::from_secs(4),
            heartbeat(refresh, start + ttl_of(times), times),
        )
        .await;
        assert!(outcome.is_err(), "the lease is held again");
        assert_eq!(beats(&calls, start), [2, 3]);
    }

    /// MOD-41 plan D3: the fence is measured on tokio's monotonic clock alone. Every refresh
    /// fails, and each one steps a wall clock the heartbeat is never handed by a day forward; the
    /// heartbeat still returns `Expired` at exactly `written - refresh` of paused time, as if no
    /// wall clock existed.
    #[tokio::test(start_paused = true)]
    async fn heartbeat_fences_on_tokio_time_alone() {
        let start = Instant::now();
        let times = default_times();
        let written = start + ttl_of(times);
        let wall = TestClock::new();
        let refresh = |_ttl| {
            wall.advance(TimeDelta::days(1));
            std::future::ready(Err::<bool, _>(StoreError::Unreachable(
                "socket closed".to_owned(),
            )))
        };
        let outcome = tokio::time::timeout(
            Duration::from_millis(600_500),
            heartbeat(refresh, written, times),
        )
        .await
        .expect("the heartbeat fences itself instead of beating for ever");
        assert_eq!(outcome, Heartbeat::Expired);
        assert_eq!(
            Instant::now(),
            written - times.refresh,
            "the fence is `written - refresh` on tokio's clock"
        );
        assert_eq!(
            wall.now(),
            epoch() + TimeDelta::days(5),
            "five failed beats (40, 50, 60, 70, 80 s) each stepped the wall clock a day"
        );
    }

    // -----------------------------------------------------------------------------------------
    // Rows, hand-built; the snapshot decoded from the fixture's JSON (plan D106)
    // -----------------------------------------------------------------------------------------

    /// A fixed instant `minutes` after noon.
    fn at(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap() + TimeDelta::minutes(minutes)
    }

    /// [`GraphSnapshot`] under another name, only so blueprint C-5's grep for a struct literal
    /// (`GraphSnapshot` followed by a brace) stays empty over this file.
    type Snapshot = GraphSnapshot;

    /// The `feature` graph's snapshot, as every fixture `graph` run carries it (`status.rs`'s
    /// pattern): decoded, never a struct literal, so a field added to it cannot break this file.
    fn snapshot() -> Snapshot {
        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
            .expect("the fixture snapshot is a `GraphSnapshot`")
    }

    /// The snapshot's phase at `position`.
    fn phase(position: i32) -> SnapshotPhase {
        snapshot()
            .phases
            .into_iter()
            .find(|phase| phase.position == position)
            .expect("the feature graph has four phases")
    }

    /// The `implement` phase: position 2, `fan_out = 1`.
    fn implement() -> SnapshotPhase {
        let phase = phase(2);
        assert_eq!(phase.name, "implement");
        assert_eq!(phase.fan_out, 1);
        phase
    }

    fn step(position: i32, attempt: i32, fanout_index: i32, status: StepStatus) -> RunStep {
        RunStep {
            id: StepId::new(),
            run_id: ids::RUN_1,
            position,
            attempt,
            fanout_index,
            phase_name: snapshot().phases[usize::try_from(position).unwrap()]
                .name
                .clone(),
            agent_id: None,
            model: None,
            status,
            gate_outcome: None,
            gate_note: None,
            selected: None,
            exit_code: None,
            prompt_digest: None,
            trim_record: None,
            usage: None,
            isolation_path: None,
            started_at: Some(at(0)),
            finished_at: None,
            verify_outcome: None,
            verify_exit_code: None,
            promoted_at: None,
            branch: None,
            role: StepRole::Step,
            parent_step_id: None,
            node_key: None,
            updated_at: at(0),
        }
    }

    /// A `running` implement step, unfinished by its own columns.
    fn running() -> RunStep {
        step(2, 1, 0, StepStatus::Running)
    }

    fn tree(step: &RunStep, repo: RepoId, mode: Isolation, dirty: bool) -> RunStepTree {
        RunStepTree {
            run_step_id: step.id,
            repo_id: repo,
            mode,
            path: format!("/work/{repo}"),
            base_ref: format!("base-of-{repo}"),
            dirty,
        }
    }

    fn commit(step: &RunStep, repo: RepoId, after: Option<&str>) -> RunStepCommit {
        RunStepCommit {
            run_step_id: step.id,
            repo_id: repo,
            before_hash: "before".to_owned(),
            after_hash: after.map(str::to_owned),
        }
    }

    fn command_run(
        step: &RunStep,
        class: &str,
        status: CommandRunStatus,
        exit_code: Option<i32>,
        queued_at: DateTime<Utc>,
    ) -> CommandRun {
        CommandRun {
            id: CommandRunId::new(),
            run_step_id: step.id,
            box_id: BoxId::new(),
            class: class.to_owned(),
            command: "cargo test".to_owned(),
            cwd: "/work".to_owned(),
            status,
            exit_code,
            output: None,
            queued_at,
            started_at: Some(queued_at),
            finished_at: None,
        }
    }

    fn verify_run(step: &RunStep, status: CommandRunStatus, exit_code: Option<i32>) -> CommandRun {
        command_run(step, "verify", status, exit_code, at(1))
    }

    // -----------------------------------------------------------------------------------------
    // classify
    // -----------------------------------------------------------------------------------------

    #[test]
    fn classify_finished_by_finished_at() {
        let mut step = running();
        step.finished_at = Some(at(5));
        step.verify_outcome = Some(VerifyOutcome::Pass);
        step.verify_exit_code = Some(0);
        let repo = RepoId::new();
        let trees = [tree(&step, repo, Isolation::Worktree, false)];
        // No `after_hash` at all: `finished_at` alone completes the second half.
        let commits = [commit(&step, repo, None)];
        let answer = classify(&step, &implement(), &[repo], &trees, &commits, true, &[]);
        assert_eq!(
            answer,
            (
                StepKind::Plain,
                Adjudication::Finished {
                    verify: Some(VerifyOutcome::Pass),
                    verify_exit_code: Some(0),
                },
            )
        );
    }

    #[test]
    fn classify_finished_by_every_after_hash() {
        let step = running();
        let (first, second) = (RepoId::new(), RepoId::new());
        let trees = [
            tree(&step, first, Isolation::Worktree, false),
            tree(&step, second, Isolation::Worktree, false),
        ];
        let commits = [
            commit(&step, first, Some("after-1")),
            commit(&step, second, Some("after-2")),
        ];
        let runs = [verify_run(&step, CommandRunStatus::Done, Some(3))];
        let answer = classify(
            &step,
            &implement(),
            &[first, second],
            &trees,
            &commits,
            true,
            &runs,
        );
        assert_eq!(
            answer.1,
            Adjudication::Finished {
                verify: Some(VerifyOutcome::Fail),
                verify_exit_code: Some(3),
            },
            "`finished_at` is `NULL`, so the verify comes from the `command_run` row"
        );

        // An empty scope makes the commit half vacuous (the demo fixture's shape).
        let answer = classify(&step, &implement(), &[], &[], &[], true, &[]);
        assert_eq!(
            answer.1,
            Adjudication::Finished {
                verify: None,
                verify_exit_code: None,
            }
        );
    }

    #[test]
    fn classify_not_finished_without_the_document() {
        let mut step = running();
        step.finished_at = Some(at(5));
        let repo = RepoId::new();
        let trees = [tree(&step, repo, Isolation::Worktree, false)];
        let commits = [commit(&step, repo, Some("after"))];
        let answer = classify(&step, &implement(), &[repo], &trees, &commits, false, &[]);
        assert_eq!(answer, (StepKind::Plain, Adjudication::Reset));
    }

    #[test]
    fn classify_finished_when_the_capture_changed_some_repos() {
        let step = running();
        let (first, second) = (RepoId::new(), RepoId::new());
        let trees = [
            tree(&step, first, Isolation::Worktree, false),
            tree(&step, second, Isolation::Worktree, false),
        ];
        // The capture batch lands in one transaction, so one `after_hash` proves it landed; the
        // second repo's `None` is a repo the step did not change.
        let some = [
            commit(&step, first, Some("after-1")),
            commit(&step, second, None),
        ];
        let answer = classify(
            &step,
            &implement(),
            &[first, second],
            &trees,
            &some,
            true,
            &[],
        );
        assert_eq!(
            answer,
            (
                StepKind::Plain,
                Adjudication::Finished {
                    verify: None,
                    verify_exit_code: None,
                },
            )
        );
    }

    #[test]
    fn classify_not_finished_with_one_repo_missing() {
        let step = running();
        let (first, second) = (RepoId::new(), RepoId::new());
        let trees = [
            tree(&step, first, Isolation::Worktree, false),
            tree(&step, second, Isolation::Worktree, false),
        ];
        // The second repo has no row at all …
        let one = [commit(&step, first, Some("after-1"))];
        let answer = classify(
            &step,
            &implement(),
            &[first, second],
            &trees,
            &one,
            true,
            &[],
        );
        assert_eq!(answer.1, Adjudication::Reset);
        // An `after_hash` for a repo outside the scope completes nothing.
        let stranger = [
            commit(&step, first, Some("after-1")),
            commit(&step, RepoId::new(), Some("after-x")),
        ];
        let answer = classify(
            &step,
            &implement(),
            &[first, second],
            &trees,
            &stranger,
            true,
            &[],
        );
        assert_eq!(answer.1, Adjudication::Reset);
        // … and every row without an `after_hash` is a capture that never landed or one that
        // changed nothing; the two look alike, and retrying is safe.
        let none = [commit(&step, first, None), commit(&step, second, None)];
        let answer = classify(
            &step,
            &implement(),
            &[first, second],
            &trees,
            &none,
            true,
            &[],
        );
        assert_eq!(answer.1, Adjudication::Reset);
    }

    #[test]
    fn classify_resettable_and_not_per_mode_and_dirty() {
        let step = running();
        let repo = RepoId::new();
        let table = [
            (Isolation::Worktree, false, true),
            (Isolation::Worktree, true, true),
            (Isolation::Copy, false, true),
            (Isolation::Copy, true, true),
            (Isolation::SharedSerialized, false, true),
            (Isolation::SharedSerialized, true, false),
            (Isolation::Local, false, true),
            (Isolation::Local, true, false),
        ];
        for (mode, dirty, resettable) in table {
            let trees = [tree(&step, repo, mode, dirty)];
            let (_, answer) = classify(&step, &implement(), &[repo], &trees, &[], true, &[]);
            let expected = if resettable {
                Adjudication::Reset
            } else {
                Adjudication::NeverReset {
                    trees: vec![(repo, format!("/work/{repo}"), format!("base-of-{repo}"))],
                }
            };
            assert_eq!(answer, expected, "{mode} dirty={dirty}");
        }

        // One dirty in-place row is enough, and the note names every row, clean ones included.
        let clean = RepoId::new();
        let trees = [
            tree(&step, clean, Isolation::Worktree, false),
            tree(&step, repo, Isolation::Local, true),
        ];
        let (_, answer) = classify(&step, &implement(), &[clean, repo], &trees, &[], true, &[]);
        assert_eq!(
            answer,
            Adjudication::NeverReset {
                trees: vec![
                    (clean, format!("/work/{clean}"), format!("base-of-{clean}")),
                    (repo, format!("/work/{repo}"), format!("base-of-{repo}")),
                ],
            }
        );
    }

    #[test]
    fn classify_a_candidate() {
        let mut fanned = implement();
        fanned.fan_out = 3;
        let candidate = step(2, 1, 1, StepStatus::Running);
        assert_eq!(StepKind::of(&candidate, &fanned), StepKind::Candidate);
        let answer = classify(&candidate, &fanned, &[], &[], &[], false, &[]);
        assert_eq!(answer, (StepKind::Candidate, Adjudication::Reset));
        // Index 0 of a fanned-out slot is a candidate too, and the plain row of a `fan_out = 1`.
        let first = step(2, 1, 0, StepStatus::Running);
        assert_eq!(StepKind::of(&first, &fanned), StepKind::Candidate);
        assert_eq!(StepKind::of(&first, &implement()), StepKind::Plain);
    }

    #[test]
    fn classify_a_candidate_keeps_the_every_repo_rule() {
        let mut fanned = implement();
        fanned.fan_out = 3;
        let candidate = step(2, 1, 1, StepStatus::Running);
        let (first, second) = (RepoId::new(), RepoId::new());
        let trees = [
            tree(&candidate, first, Isolation::Worktree, false),
            tree(&candidate, second, Isolation::Worktree, false),
        ];
        let some = [
            commit(&candidate, first, Some("after-1")),
            commit(&candidate, second, None),
        ];
        let answer = classify(
            &candidate,
            &fanned,
            &[first, second],
            &trees,
            &some,
            true,
            &[],
        );
        assert_eq!(
            answer,
            (StepKind::Candidate, Adjudication::Reset),
            "a candidate is finished only when every scope repo has an `after_hash`"
        );
        let every = [
            commit(&candidate, first, Some("after-1")),
            commit(&candidate, second, Some("after-2")),
        ];
        let answer = classify(
            &candidate,
            &fanned,
            &[first, second],
            &trees,
            &every,
            true,
            &[],
        );
        assert_eq!(
            answer,
            (
                StepKind::Candidate,
                Adjudication::Finished {
                    verify: None,
                    verify_exit_code: None,
                },
            )
        );
    }

    #[test]
    fn classify_a_judge() {
        let mut fanned = implement();
        fanned.fan_out = 3;
        let judge = step(2, 1, -1, StepStatus::Running);
        let answer = classify(&judge, &fanned, &[], &[], &[], true, &[]);
        assert_eq!(
            answer,
            (
                StepKind::Judge,
                Adjudication::Finished {
                    verify: None,
                    verify_exit_code: None,
                },
            ),
            "a judge is classified as well; the engine ignores the answer (D95)"
        );
    }

    /// MOD-139 M2 D14: a part is its own kind, whatever its node's `fan_out`, and its capture is
    /// proved as a plain step's: one scope repo with an `after_hash` is enough.
    #[test]
    fn a_part_is_classified_by_the_plain_artefact_test() {
        let mut fanned = implement();
        fanned.fan_out = 3;
        let mut part = step(2, 1, 0, StepStatus::Running);
        part.role = StepRole::Part;
        assert_eq!(StepKind::of(&part, &implement()), StepKind::Part);
        assert_eq!(
            StepKind::of(&part, &fanned),
            StepKind::Part,
            "never a candidate"
        );
        let (first, second) = (RepoId::new(), RepoId::new());
        let trees = [
            tree(&part, first, Isolation::Worktree, false),
            tree(&part, second, Isolation::Worktree, false),
        ];
        let one = [
            commit(&part, first, Some("after-1")),
            commit(&part, second, None),
        ];
        let answer = classify(&part, &fanned, &[first, second], &trees, &one, true, &[]);
        assert_eq!(
            answer,
            (
                StepKind::Part,
                Adjudication::Finished {
                    verify: None,
                    verify_exit_code: None,
                },
            ),
            "one `after_hash` proves the capture batch, as for a plain step"
        );
        let none = [commit(&part, first, None), commit(&part, second, None)];
        let answer = classify(&part, &fanned, &[first, second], &trees, &none, true, &[]);
        assert_eq!(answer, (StepKind::Part, Adjudication::Reset));
    }

    #[test]
    fn classify_a_step_with_no_tree_is_vacuously_resettable() {
        let step = running();
        let answer = classify(&step, &implement(), &[RepoId::new()], &[], &[], true, &[]);
        assert_eq!(answer, (StepKind::Plain, Adjudication::Reset));
    }

    // -----------------------------------------------------------------------------------------
    // verify_of
    // -----------------------------------------------------------------------------------------

    #[test]
    fn verify_of_maps_command_runs_to_outcomes() {
        let step = running();
        let one = |status, exit_code| verify_of(&step, &[verify_run(&step, status, exit_code)]);
        assert_eq!(
            one(CommandRunStatus::Done, Some(0)),
            (Some(VerifyOutcome::Pass), Some(0))
        );
        assert_eq!(
            one(CommandRunStatus::Done, Some(101)),
            (Some(VerifyOutcome::Fail), Some(101))
        );
        assert_eq!(
            one(CommandRunStatus::Failed, None),
            (Some(VerifyOutcome::Unavailable), None)
        );
        assert_eq!(one(CommandRunStatus::Queued, None), (None, None));
        assert_eq!(one(CommandRunStatus::Running, None), (None, None));
        assert_eq!(verify_of(&step, &[]), (None, None));

        // A non-`verify` class is ignored, however late it was queued.
        let runs = [
            verify_run(&step, CommandRunStatus::Done, Some(0)),
            command_run(&step, "build", CommandRunStatus::Done, Some(1), at(9)),
        ];
        assert_eq!(
            verify_of(&step, &runs),
            (Some(VerifyOutcome::Pass), Some(0))
        );
        let runs = [command_run(
            &step,
            "build",
            CommandRunStatus::Done,
            Some(1),
            at(9),
        )];
        assert_eq!(verify_of(&step, &runs), (None, None));

        // The last `verify` row decides, whatever order the slice is in.
        let mut runs = [
            command_run(&step, "verify", CommandRunStatus::Done, Some(2), at(7)),
            command_run(&step, "verify", CommandRunStatus::Done, Some(0), at(3)),
        ];
        assert_eq!(
            verify_of(&step, &runs),
            (Some(VerifyOutcome::Fail), Some(2))
        );
        runs.reverse();
        assert_eq!(
            verify_of(&step, &runs),
            (Some(VerifyOutcome::Fail), Some(2))
        );

        // On a `queued_at` tie, the later row in the slice decides.
        let runs = [
            command_run(&step, "verify", CommandRunStatus::Done, Some(0), at(7)),
            command_run(&step, "verify", CommandRunStatus::Done, Some(3), at(7)),
        ];
        assert_eq!(
            verify_of(&step, &runs),
            (Some(VerifyOutcome::Fail), Some(3))
        );

        // `finished_at` wins: the row's own columns, even `NULL`, over any `command_run`.
        let mut finished = running();
        finished.finished_at = Some(at(5));
        let runs = [verify_run(&finished, CommandRunStatus::Done, Some(1))];
        assert_eq!(verify_of(&finished, &runs), (None, None));
        finished.verify_outcome = Some(VerifyOutcome::Unavailable);
        assert_eq!(
            verify_of(&finished, &runs),
            (Some(VerifyOutcome::Unavailable), None)
        );
    }

    // -----------------------------------------------------------------------------------------
    // resettle
    // -----------------------------------------------------------------------------------------

    fn document(kind: &str, body: &str, step: &RunStep) -> Document {
        Document {
            id: DocumentId::new(),
            item_id: ItemId::new(),
            kind: kind.to_owned(),
            version: 1,
            title: kind.to_owned(),
            body: body.to_owned(),
            produced_by_step_id: Some(step.id),
            created_by: UserId::new(),
            created_at: at(4),
        }
    }

    #[test]
    fn resettle_maps_command_runs_to_verify_outcomes() {
        let step = running();
        let output = document("implement", "done", &step);
        // A day after the step started: the sweep drops the deadline, so lateness fails nothing.
        let now = at(24 * 60);
        let settle_with = |runs: &[CommandRun]| {
            let (verify, _) = verify_of(&step, runs);
            resettle(Some(&output), verify, false, at(0), now, None)
        };
        assert_eq!(
            settle_with(&[verify_run(&step, CommandRunStatus::Done, Some(1))]),
            Settle::Failed(StepFailure::VerifyFailed)
        );
        assert_eq!(
            settle_with(&[verify_run(&step, CommandRunStatus::Done, Some(0))]),
            Settle::Ok { note: None }
        );
        assert_eq!(
            settle_with(&[verify_run(&step, CommandRunStatus::Failed, None)]),
            Settle::Ok { note: None },
            "`unavailable` never fails a step"
        );
        assert_eq!(settle_with(&[]), Settle::Ok { note: None });
        assert_eq!(
            resettle(None, Some(VerifyOutcome::Pass), false, at(0), now, None),
            Settle::Failed(StepFailure::MissingOutput)
        );
    }

    #[test]
    fn resettle_reads_a_review_verdict() {
        let step = step(3, 1, 0, StepStatus::Running);
        let rejecting = document(
            "review",
            "---\nverdict: request-changes\n---\nThe tests are missing.\n",
            &step,
        );
        assert_eq!(
            resettle(Some(&rejecting), None, true, at(0), at(5), None),
            Settle::Rejected {
                verdict_line: "verdict: request-changes".to_owned(),
            }
        );
        assert_eq!(
            resettle(Some(&rejecting), None, false, at(0), at(5), None),
            Settle::Ok { note: None },
            "a phase not named `review` never reads the front matter"
        );
        let approving = document("review", "---\nverdict: approve\n---\nShip it.\n", &step);
        assert_eq!(
            resettle(Some(&approving), None, true, at(0), at(5), None),
            Settle::Ok { note: None }
        );
    }

    // -----------------------------------------------------------------------------------------
    // frontiers (MOD-138 M3 D6, Q-1, A8, A9)
    // -----------------------------------------------------------------------------------------

    /// A hand-built graph (`walk.rs`'s builder) with every node at `worktree`.
    fn worktree(nodes: &[(&str, i32)], edges: &[(&str, &str, EdgeKind)]) -> Snapshot {
        let mut snapshot = walk_fx::graph(nodes, edges);
        for phase in &mut snapshot.phases {
            phase.isolation = Isolation::Worktree;
        }
        snapshot
    }

    /// `walk.rs`'s diamond `a → {b, c} → d` (with `d changes_requested → b`), every node at
    /// `worktree`.
    fn diamond() -> Snapshot {
        let mut snapshot = walk_fx::diamond();
        for phase in &mut snapshot.phases {
            phase.isolation = Isolation::Worktree;
        }
        snapshot
    }

    /// A row of node `name`, started and finished at the given minutes after noon.
    fn timed(
        snapshot: &Snapshot,
        name: &str,
        attempt: i32,
        status: StepStatus,
        started: Option<i64>,
        finished: Option<i64>,
    ) -> RunStep {
        let mut row = walk_fx::row(snapshot, name, attempt, status);
        row.started_at = started.map(at);
        row.finished_at = finished.map(at);
        row
    }

    /// Attempt 1 of `name`, `done`, started at `started` and finished at `finished`.
    fn landed(snapshot: &Snapshot, name: &str, started: i64, finished: i64) -> RunStep {
        timed(
            snapshot,
            name,
            1,
            StepStatus::Done,
            Some(started),
            Some(finished),
        )
    }

    const NEXT: EdgeKind = EdgeKind::Next;

    #[test]
    fn frontiers_holds_two_parallel_done_nodes() {
        let snapshot = diamond();
        let a = landed(&snapshot, "a", 0, 1);
        let b = landed(&snapshot, "b", 2, 3);
        let c = landed(&snapshot, "c", 2, 4);
        let rows = vec![a.clone(), b.clone(), c.clone()];
        assert_eq!(
            frontiers(&snapshot, &rows),
            [b.id, c.id],
            "`a` is proven by both arms; neither arm has a successor row"
        );
        let mut pending = rows.clone();
        pending.push(timed(&snapshot, "d", 1, StepStatus::Pending, None, None));
        assert_eq!(
            frontiers(&snapshot, &pending),
            [b.id, c.id],
            "A8: an unstarted `d` proves nothing"
        );
        let mut running = rows;
        running.push(timed(&snapshot, "d", 1, StepStatus::Running, Some(5), None));
        assert!(frontiers(&snapshot, &running).is_empty());
    }

    /// The M2 Q-C gap: on a chain laid out in reverse position order the node the walk finished
    /// last is not the highest position.
    #[test]
    fn frontiers_on_a_reordered_chain_is_the_node_finished_last() {
        let snapshot = worktree(
            &[("c", 0), ("b", 1), ("a", 2)],
            &[("a", "b", NEXT), ("b", "c", NEXT)],
        );
        let a = landed(&snapshot, "a", 0, 1);
        let b = landed(&snapshot, "b", 2, 3);
        let rows = [a, b.clone()];
        assert_eq!(
            frontiers(&snapshot, &rows),
            [b.id],
            "not `a`, the highest position, whose successor already landed"
        );
    }

    /// A `changes_requested` loop over one arm of a diamond: the re-run arm and the untouched one
    /// (whose successor's row was cancelled) are members; the re-land of the latter is
    /// idempotent (`merge_of`).
    #[test]
    fn frontiers_after_a_loop_on_a_branch() {
        let snapshot = diamond();
        let a = landed(&snapshot, "a", 0, 1);
        let b1 = timed(&snapshot, "b", 1, StepStatus::Superseded, Some(2), Some(3));
        let c = landed(&snapshot, "c", 2, 4);
        let d1 = timed(&snapshot, "d", 1, StepStatus::Cancelled, Some(5), Some(6));
        let b2 = timed(&snapshot, "b", 2, StepStatus::Done, Some(7), Some(8));
        assert_eq!(
            frontiers(&snapshot, &[a, b1, c.clone(), d1, b2.clone()]),
            [b2.id, c.id]
        );
    }

    /// A-3 per node: a running successor keeps its own predecessor off the frontier, and only
    /// that one.
    #[test]
    fn a_running_successor_keeps_only_its_predecessor_off() {
        let chain = worktree(&[("a", 0), ("b", 1)], &[("a", "b", NEXT)]);
        let a = landed(&chain, "a", 0, 1);
        let b = timed(&chain, "b", 1, StepStatus::Running, Some(2), None);
        assert!(frontiers(&chain, &[a, b]).is_empty());

        let snapshot = diamond();
        let a = landed(&snapshot, "a", 0, 1);
        let b = landed(&snapshot, "b", 2, 3);
        let c = timed(&snapshot, "c", 1, StepStatus::Running, Some(2), None);
        assert_eq!(
            frontiers(&snapshot, &[a, b.clone(), c]),
            [b.id],
            "a done, unlanded `b` beside a running `c`"
        );
    }

    /// A skipped successor has no row, so it proves nothing.
    #[test]
    fn a_skipped_successor_proves_nothing() {
        let snapshot = worktree(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "b", EdgeKind::If), ("a", "c", NEXT)],
        );
        let mut a = landed(&snapshot, "a", 0, 1);
        a.branch = Some(serde_json::json!({ "b": false }));
        assert_eq!(frontiers(&snapshot, std::slice::from_ref(&a)), [a.id]);
        let c = landed(&snapshot, "c", 2, 3);
        assert_eq!(frontiers(&snapshot, &[a, c.clone()]), [c.id]);
    }

    /// A fanned-out node's member is its selected candidate; any candidate row proves its
    /// predecessor.
    #[test]
    fn frontiers_of_a_fan_out_is_the_selected_winner() {
        let mut snapshot = worktree(&[("a", 0), ("f", 1)], &[("a", "f", NEXT)]);
        snapshot.phases[1].fan_out = 3;
        let a = landed(&snapshot, "a", 0, 1);
        let mut rows = vec![a];
        for index in 0..3 {
            let mut candidate = landed(&snapshot, "f", 2, 3);
            candidate.fanout_index = index;
            candidate.selected = Some(index == 1);
            rows.push(candidate);
        }
        let mut judge = landed(&snapshot, "f", 3, 4);
        judge.fanout_index = -1;
        rows.push(judge);
        assert_eq!(frontiers(&snapshot, &rows), [rows[2].id]);
        for candidate in &mut rows[1..4] {
            candidate.selected = Some(false);
        }
        assert!(
            frontiers(&snapshot, &rows).is_empty(),
            "nothing selected: no winner at `f`, and its rows prove `a`"
        );
    }

    /// A8: a loop whose region holds `n` but not `n`'s successor `s` leaves `s`'s older row; it
    /// started before the re-run `n` finished, so it does not hide `n`.
    #[test]
    fn an_off_region_successors_older_row_does_not_hide_a_re_run_winner() {
        let snapshot = worktree(
            &[("t", 0), ("n", 1), ("s", 2), ("r", 3)],
            &[
                ("t", "n", NEXT),
                ("n", "s", NEXT),
                ("n", "r", NEXT),
                ("r", "t", EdgeKind::ChangesRequested),
            ],
        );
        let rows = [
            timed(&snapshot, "t", 1, StepStatus::Superseded, Some(0), Some(1)),
            timed(&snapshot, "n", 1, StepStatus::Superseded, Some(2), Some(3)),
            landed(&snapshot, "s", 4, 5),
            timed(&snapshot, "r", 1, StepStatus::Cancelled, Some(4), Some(6)),
            timed(&snapshot, "t", 2, StepStatus::Done, Some(7), Some(8)),
            timed(&snapshot, "n", 2, StepStatus::Done, Some(9), Some(10)),
        ];
        assert_eq!(frontiers(&snapshot, &rows), [rows[5].id, rows[2].id]);
    }

    /// A8: a `pending` successor has not started, so its predecessor re-lands.
    #[test]
    fn a_pending_successor_re_lands_its_predecessor() {
        let snapshot = worktree(&[("a", 0), ("b", 1)], &[("a", "b", NEXT)]);
        let a = landed(&snapshot, "a", 0, 1);
        let b = timed(&snapshot, "b", 1, StepStatus::Pending, None, None);
        assert_eq!(frontiers(&snapshot, &[a.clone(), b]), [a.id]);
    }

    /// A8 narrowed (T4): a `pending` retry of a successor proves its predecessor when an earlier
    /// attempt of that successor started after the predecessor finished, retired or not; an
    /// earlier attempt that started before (a loop's older row) proves nothing.
    #[test]
    fn a_pending_retry_proves_its_predecessor_through_its_earlier_attempt() {
        let snapshot = worktree(&[("a", 0), ("b", 1)], &[("a", "b", NEXT)]);
        let a = landed(&snapshot, "a", 0, 1);
        let retry = timed(&snapshot, "b", 2, StepStatus::Pending, None, None);
        let after = timed(&snapshot, "b", 1, StepStatus::Superseded, Some(1), None);
        assert!(frontiers(&snapshot, &[a.clone(), after.clone(), retry.clone()]).is_empty());
        let before = timed(&snapshot, "b", 1, StepStatus::Superseded, Some(0), None);
        assert_eq!(frontiers(&snapshot, &[a.clone(), before, retry]), [a.id]);
        assert_eq!(
            frontiers(&snapshot, &[a.clone(), after]),
            [a.id],
            "a retired row alone proves nothing"
        );
    }

    /// A9: an in-place member is kept only when no done winner of the run finished later, in any
    /// mode, member or not; a tie keeps it.
    #[test]
    fn an_in_place_sink_followed_by_a_worktree_landing_is_not_a_member() {
        let mut snapshot = worktree(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "b", NEXT), ("a", "c", NEXT)],
        );
        snapshot.phases[1].isolation = Isolation::SharedSerialized;
        let a = landed(&snapshot, "a", 0, 1);
        let c = landed(&snapshot, "c", 2, 7);
        let b = |finished| landed(&snapshot, "b", 2, finished);
        assert_eq!(
            frontiers(&snapshot, &[a.clone(), b(5), c.clone()]),
            [c.id],
            "`c` landed after `b`, so `b` landed and re-landing it would refuse `primary_moved`"
        );
        let later = b(8);
        assert_eq!(
            frontiers(&snapshot, &[a.clone(), later.clone(), c.clone()]),
            [later.id, c.id]
        );
        let tied = b(7);
        assert_eq!(
            frontiers(&snapshot, &[a, tied.clone(), c.clone()]),
            [tied.id, c.id],
            "a tie keeps the member"
        );

        // A non-member that finished later moved `HEAD` too. At `fan_out = 1` the landing rule
        // drops `b` on its own here (`w` started after `b` finished); the fanned-out case below
        // is where A9's non-member half alone decides.
        let mut snapshot = worktree(
            &[("a", 0), ("b", 1), ("z", 2), ("w", 3)],
            &[("a", "b", NEXT), ("z", "w", NEXT)],
        );
        snapshot.phases[1].isolation = Isolation::Local;
        let rows = [
            landed(&snapshot, "a", 0, 1),
            landed(&snapshot, "b", 2, 5),
            landed(&snapshot, "z", 0, 9),
            timed(&snapshot, "w", 1, StepStatus::Running, Some(10), None),
        ];
        assert!(frontiers(&snapshot, &rows).is_empty());

        // A fanned-out in-place member lands at its selection, which no column stamps, so the
        // landing rule leaves it alone: only A9 over every done winner, the non-member `z`
        // included, drops `m`.
        let mut snapshot = worktree(
            &[("a", 0), ("m", 1), ("z", 2), ("w", 3)],
            &[("a", "m", NEXT), ("z", "w", NEXT)],
        );
        snapshot.phases[1].isolation = Isolation::SharedSerialized;
        snapshot.phases[1].fan_out = 2;
        let fanned = |z_finished| {
            let mut rows = vec![landed(&snapshot, "a", 0, 1)];
            for index in 0..2 {
                let mut candidate = landed(&snapshot, "m", 2, 5);
                candidate.fanout_index = index;
                candidate.selected = Some(index == 0);
                rows.push(candidate);
            }
            rows.push(landed(&snapshot, "z", 0, z_finished));
            rows.push(timed(
                &snapshot,
                "w",
                1,
                StepStatus::Running,
                Some(10),
                None,
            ));
            rows
        };
        let earlier = fanned(4);
        assert_eq!(
            frontiers(&snapshot, &earlier),
            [earlier[1].id],
            "`z` finished before `m`: `m` is kept, `w` started later notwithstanding"
        );
        assert!(
            frontiers(&snapshot, &fanned(9)).is_empty(),
            "non-member `z` finished after `m`, so `m` landed"
        );
    }

    /// A hand-built graph (`walk.rs`'s builder), every node at `shared_serialized`.
    fn in_place(nodes: &[(&str, i32)], edges: &[(&str, &str, EdgeKind)]) -> Snapshot {
        let mut snapshot = walk_fx::graph(nodes, edges);
        for phase in &mut snapshot.phases {
            phase.isolation = Isolation::SharedSerialized;
        }
        snapshot
    }

    /// A9 over landings (repair 1): an in-place winner whose only proof of landing is a row a
    /// loop later retired. `implement` attempt 1 started after `plan` finished, so `plan`
    /// landed; re-landing it would refuse `primary_moved`. Linear parity with M2's position rule,
    /// which found no frontier here either.
    #[test]
    fn an_in_place_winner_landed_before_a_retired_row_is_not_a_member() {
        let snapshot = in_place(
            &[("plan", 0), ("implement", 1), ("review", 2)],
            &[
                ("plan", "implement", NEXT),
                ("implement", "review", NEXT),
                ("review", "implement", EdgeKind::ChangesRequested),
            ],
        );
        let rows = [
            landed(&snapshot, "plan", 0, 1),
            timed(
                &snapshot,
                "implement",
                1,
                StepStatus::Superseded,
                Some(2),
                Some(3),
            ),
            timed(
                &snapshot,
                "review",
                1,
                StepStatus::Cancelled,
                Some(4),
                Some(5),
            ),
            timed(&snapshot, "implement", 2, StepStatus::Pending, None, None),
        ];
        assert!(
            frontiers(&snapshot, &rows).is_empty(),
            "`implement` attempt 1 started after `plan` finished, so `plan` landed"
        );
    }

    /// A9 over landings (repair 1), on a branch: the in-place sink `x` finished before `c`'s
    /// first attempt started, which a loop since retired.
    #[test]
    fn an_in_place_sink_landed_before_a_retired_arm_is_not_a_member() {
        let snapshot = in_place(
            &[("a", 0), ("x", 1), ("c", 2), ("d", 3)],
            &[
                ("a", "x", NEXT),
                ("a", "c", NEXT),
                ("c", "d", NEXT),
                ("d", "c", EdgeKind::ChangesRequested),
            ],
        );
        let rows = [
            landed(&snapshot, "a", 0, 1),
            landed(&snapshot, "x", 2, 3),
            timed(&snapshot, "c", 1, StepStatus::Superseded, Some(4), Some(5)),
            timed(&snapshot, "d", 1, StepStatus::Superseded, Some(6), Some(7)),
            timed(&snapshot, "c", 2, StepStatus::Running, Some(8), None),
        ];
        assert!(frontiers(&snapshot, &rows).is_empty());
    }

    /// The landing rule is strict, as A9's tie: a row started at the instant the in-place winner
    /// finished (a frozen clock) proves nothing, so the member is kept. A fanned-out winner lands
    /// at its selection, which no column stamps, so a row started after its `finished_at` does
    /// not drop it.
    #[test]
    fn the_landing_rule_keeps_a_tie_and_a_selected_winner() {
        let snapshot = in_place(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "c", NEXT), ("b", "c", NEXT)],
        );
        let a = landed(&snapshot, "a", 0, 1);
        let tied = timed(&snapshot, "b", 1, StepStatus::Running, Some(1), None);
        assert_eq!(frontiers(&snapshot, &[a.clone(), tied]), [a.id]);

        let mut snapshot = in_place(
            &[("f", 0), ("b", 1), ("c", 2)],
            &[("f", "c", NEXT), ("b", "c", NEXT)],
        );
        snapshot.phases[0].fan_out = 2;
        let mut rows = Vec::new();
        for index in 0..2 {
            let mut candidate = landed(&snapshot, "f", 0, 1);
            candidate.fanout_index = index;
            candidate.selected = Some(index == 0);
            rows.push(candidate);
        }
        rows.push(timed(&snapshot, "b", 1, StepStatus::Running, Some(2), None));
        assert_eq!(frontiers(&snapshot, &rows), [rows[0].id]);
    }
}
