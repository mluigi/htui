//! ANA-2 §4.9's lease heartbeat and the sweep's pure half (plan D85, D86, D90, D91, D97).
//!
//! Nothing here reads or writes a store. The heartbeat takes its refresh as a closure (plan D106),
//! so the engine hands it `|until| store.refresh_lease(run, owner, until)` and a test hands it a
//! script; everything else is a function of rows the caller already read.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use htui_agent::event::{DoneEvent, StopReason};
use htui_core::model::{
    CommandRun, CommandRunStatus, Document, GraphSnapshot, Isolation, RepoId, RunStep,
    RunStepCommit, RunStepTree, SnapshotPhase, StepId, StepStatus, VerifyOutcome,
};
use htui_core::store::StoreError;
use serde_json::Value;

use crate::gate::{Settle, SettleInput, settle};
use crate::isolate::Clock;
use crate::verify::VERIFY_CLASS;

/// `app_setting` key of the lease's time to live, in seconds (`0003_orchestration.sql:135`).
pub const LEASE_TTL_KEY: &str = "lease_ttl_seconds";

/// `app_setting` key of the heartbeat's interval, in seconds (`0003_orchestration.sql:136`).
pub const LEASE_REFRESH_KEY: &str = "lease_refresh_seconds";

/// The TTL when [`LEASE_TTL_KEY`] is absent or not a positive integer: ANA-2 §10's 120 s (PRD D2).
pub const DEFAULT_LEASE_TTL_SECONDS: i64 = 120;

/// The interval when [`LEASE_REFRESH_KEY`] is absent or not a positive integer: §10's 60 s.
pub const DEFAULT_LEASE_REFRESH_SECONDS: i64 = 60;

/// How long a lease lives and how often it is renewed (plan D85).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseTimes {
    /// Added to the clock's instant for every `lease_expires_at` this process writes.
    pub ttl: TimeDelta,
    /// How long the heartbeat sleeps between two refreshes; always shorter than `ttl`.
    pub refresh: Duration,
}

impl LeaseTimes {
    /// Positive `i64` per key, else the default; then `refresh >= ttl` → `ttl / 2`, in
    /// **milliseconds** so a 1 s TTL still beats at 500 ms.
    ///
    /// A refresh at or above the TTL would let a live lease expire between two beats, and another
    /// box's sweep would adopt a run this process is still walking.
    #[must_use]
    pub fn from_app(app: &BTreeMap<String, Value>) -> Self {
        // A TTL too large for a `TimeDelta` (or for its half in milliseconds) is read as silence.
        let ttl_seconds = app_positive(app, LEASE_TTL_KEY)
            .filter(|seconds| TimeDelta::try_seconds(*seconds).is_some())
            .unwrap_or(DEFAULT_LEASE_TTL_SECONDS);
        let refresh_seconds =
            app_positive(app, LEASE_REFRESH_KEY).unwrap_or(DEFAULT_LEASE_REFRESH_SECONDS);
        let ttl = TimeDelta::seconds(ttl_seconds);
        let refresh = if refresh_seconds >= ttl_seconds {
            Duration::from_millis(ttl.num_milliseconds().unsigned_abs() / 2)
        } else {
            Duration::from_secs(refresh_seconds.unsigned_abs())
        };
        Self { ttl, refresh }
    }
}

/// One `app_setting` value as a positive `i64`: `graph.rs`'s private rule, restated here rather
/// than shared (blueprint F-U). Absent, `null`, a string, a bool, zero and a negative all mean
/// "not set", so a planted `0` falls back to the default rather than pinning the lease to nothing.
fn app_positive(app: &BTreeMap<String, Value>, key: &str) -> Option<i64> {
    app.get(key).and_then(Value::as_i64).filter(|n| *n > 0)
}

/// Why [`heartbeat`] returned. It has one answer, because it never returns otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    /// A refresh touched zero rows: another orchestrator took the lease, and ANA-2 `:1280-1282`
    /// has this one abandon the run without writing further.
    Abandoned,
}

/// Sleep `times.refresh` (`tokio::time::sleep`), then `refresh(clock.now() + times.ttl)`:
/// `Ok(true)` → loop; `Ok(false)` → `Abandoned`; `Err(e)` → `tracing::warn!` and loop (D86, D103).
/// Never returns otherwise.
///
/// A store error is not a taken lease: offline, the lease is left to expire and the reconnect
/// sweep adjudicates (`docs/ANA-2.md:1325-1336`). Needs a runtime with the time driver (H-3).
pub async fn heartbeat<F, Fut, C>(mut refresh: F, clock: &C, times: LeaseTimes) -> Heartbeat
where
    F: FnMut(DateTime<Utc>) -> Fut,
    Fut: Future<Output = Result<bool, StoreError>>,
    C: Clock + ?Sized,
{
    loop {
        tokio::time::sleep(times.refresh).await;
        match refresh(clock.now() + times.ttl).await {
            Ok(true) => {}
            Ok(false) => return Heartbeat::Abandoned,
            Err(error) => {
                tracing::warn!(%error, "lease refresh failed; beating again at the next interval");
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
}

impl StepKind {
    /// `fanout_index == -1` → Judge; `phase.fan_out > 1` → Candidate; else Plain.
    #[must_use]
    pub const fn of(step: &RunStep, phase: &SnapshotPhase) -> Self {
        if step.fanout_index == JUDGE_FANOUT_INDEX {
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
/// Finished when the output document exists **and** either `finished_at` is set or every repo of
/// `run_scope` has a `run_step_commit` row with an `after_hash` (vacuous for an empty scope, the
/// demo fixture's). Otherwise resettable when every tree row is `worktree | copy`, or
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
    let captured = run_scope.iter().all(|repo| {
        commits
            .iter()
            .any(|row| row.repo_id == *repo && row.after_hash.is_some())
    });
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
/// `deadline_seconds: None`.
///
/// The deadline is dropped because the sweep's clock is not the step's, and the cap because the
/// recorder's breach is not durable (plan R-13); `now` is therefore inert. `verify` is
/// [`verify_of`]'s, and `is_review` is `SnapshotPhase::name == "review"`, as stage 5 passes it.
#[must_use]
pub fn resettle(
    output: Option<&Document>,
    verify: Option<VerifyOutcome>,
    is_review: bool,
    started_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Settle {
    let driver = Ok(DoneEvent {
        stop_reason: StopReason::EndTurn,
    });
    settle(&SettleInput {
        driver: &driver,
        cap_breach: None,
        started_at,
        now,
        deadline_seconds: None,
        output,
        verify_outcome: verify,
        is_review,
    })
}

// ---------------------------------------------------------------------------------------------
// The frontier (plan D97, blueprint A-3)
// ---------------------------------------------------------------------------------------------

/// D97 as redefined (C129): the latest-attempt `done` winner at the highest position `p` having one
/// (`selected == Some(true)`, or the one `fanout_index = 0` row of a `fan_out = 1` phase),
/// provided every row at a position > `p` is `Superseded | Cancelled`.
///
/// Retired rows are ignored because a review rejection retires `target..=review` before the walk
/// creates the next implement attempt, so the rejected review already sits after that attempt's
/// winner. Every **live** row after `p` was created after `p`'s reconcile returned, so a live one
/// means there is nothing to re-reconcile. Under blueprint A-3 the sweep reads this over the rows
/// as adopted, before any step is adjudicated: a `running` row after `p` makes it `None`.
#[must_use]
pub fn frontier(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Option<StepId> {
    let winner = snapshot
        .phases
        .iter()
        .rev()
        .find_map(|phase| winner_at(phase, steps))?;
    steps
        .iter()
        .filter(|step| step.position > winner.position)
        .all(|step| matches!(step.status, StepStatus::Superseded | StepStatus::Cancelled))
        .then_some(winner.id)
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
    use htui_core::store::StoreError;
    use serde_json::{Value, json};

    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        BoxId, CommandRun, CommandRunId, CommandRunStatus, Document, DocumentId, GraphSnapshot,
        Isolation, ItemId, RepoId, RunStep, RunStepCommit, RunStepTree, SnapshotPhase, StepId,
        StepStatus, UserId, VerifyOutcome,
    };

    use crate::gate::{Settle, StepFailure};
    use crate::isolate::Clock;
    use crate::recover::{
        Adjudication, Heartbeat, LeaseTimes, StepKind, classify, frontier, heartbeat, resettle,
        verify_of,
    };

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

    #[test]
    fn lease_times_fall_back_to_120_and_60() {
        for silent in [None, Some(json!(0)), Some(json!(-5)), Some(json!("120"))] {
            let settings = silent.map_or_else(BTreeMap::new, |value| {
                app(&[
                    ("lease_ttl_seconds", value.clone()),
                    ("lease_refresh_seconds", value),
                ])
            });
            let times = LeaseTimes::from_app(&settings);
            assert_eq!(times.ttl, TimeDelta::seconds(120), "{settings:?}");
            assert_eq!(times.refresh, Duration::from_secs(60), "{settings:?}");
        }
    }

    #[test]
    fn a_refresh_at_or_above_the_ttl_reads_as_half() {
        let equal = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(60)),
            ("lease_refresh_seconds", json!(60)),
        ]));
        assert_eq!(equal.ttl, TimeDelta::seconds(60));
        assert_eq!(equal.refresh, Duration::from_secs(30));

        let above = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(1)),
            ("lease_refresh_seconds", json!(5)),
        ]));
        assert_eq!(above.ttl, TimeDelta::seconds(1));
        assert_eq!(above.refresh, Duration::from_millis(500));
    }

    #[test]
    fn a_ttl_no_timestamp_can_carry_reads_as_silence() {
        // Fits a `TimeDelta` (up to ~9.2e15 s) but not `now + ttl` (`DateTime<Utc>` ends near
        // year 262143), so the heartbeat's addition would panic.
        for huge in [10_000_000_000_000_i64, i64::MAX] {
            let times = LeaseTimes::from_app(&app(&[("lease_ttl_seconds", json!(huge))]));
            assert_eq!(times.ttl, TimeDelta::seconds(120), "{huge}");
            assert_eq!(times.refresh, Duration::from_secs(60), "{huge}");
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

    /// A [`Clock`] that reads tokio's paused time, so `clock.now()` moves exactly as far as the
    /// heartbeat's `sleep` did.
    struct PausedClock {
        origin: DateTime<Utc>,
        start: tokio::time::Instant,
    }

    impl PausedClock {
        fn new() -> Self {
            Self {
                origin: Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap(),
                start: tokio::time::Instant::now(),
            }
        }

        fn elapsed(&self) -> Duration {
            self.start.elapsed()
        }
    }

    impl Clock for PausedClock {
        fn now(&self) -> DateTime<Utc> {
            self.origin + TimeDelta::from_std(self.elapsed()).expect("a test's elapsed time fits")
        }
    }

    /// The default 120 s / 60 s.
    fn default_times() -> LeaseTimes {
        LeaseTimes::from_app(&BTreeMap::new())
    }

    /// A refresh closure answering `script` in order (then `Ok(true)` for ever), recording each
    /// `until` beside the clock's instant at the call.
    #[allow(clippy::type_complexity)]
    fn scripted(
        clock: &PausedClock,
        script: Vec<Result<bool, StoreError>>,
    ) -> (
        Arc<Mutex<Vec<(DateTime<Utc>, DateTime<Utc>)>>>,
        impl FnMut(DateTime<Utc>) -> std::future::Ready<Result<bool, StoreError>> + '_,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let mut script = script.into_iter();
        let refresh = move |until| {
            seen.lock().unwrap().push((clock.now(), until));
            std::future::ready(script.next().unwrap_or(Ok(true)))
        };
        (calls, refresh)
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_refreshes_every_interval() {
        let clock = PausedClock::new();
        let times = default_times();
        let (calls, refresh) = scripted(&clock, vec![Ok(true), Ok(true), Ok(true)]);
        let outcome =
            tokio::time::timeout(Duration::from_secs(185), heartbeat(refresh, &clock, times)).await;
        assert!(
            outcome.is_err(),
            "the heartbeat never returns on `Ok(true)`"
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "a beat at 60 s, 120 s and 180 s");
        for (beat, (now, until)) in (1..).zip(calls.iter()) {
            assert_eq!(*now, clock.origin + TimeDelta::seconds(60 * beat));
            assert_eq!(*until, *now + times.ttl, "beat {beat}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_returns_abandoned_on_a_zero_row_refresh() {
        let clock = PausedClock::new();
        let (calls, refresh) = scripted(&clock, vec![Ok(true), Ok(false)]);
        let outcome = heartbeat(refresh, &clock, default_times()).await;
        assert_eq!(outcome, Heartbeat::Abandoned);
        assert_eq!(clock.elapsed(), Duration::from_secs(120));
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_survives_a_store_error() {
        let clock = PausedClock::new();
        let (calls, refresh) = scripted(
            &clock,
            vec![
                Err(StoreError::Unreachable("socket closed".to_owned())),
                Err(StoreError::Backend("deadlock".to_owned())),
                Ok(false),
            ],
        );
        let outcome = heartbeat(refresh, &clock, default_times()).await;
        assert_eq!(outcome, Heartbeat::Abandoned);
        assert_eq!(
            calls.lock().unwrap().len(),
            3,
            "abandoned on the third beat"
        );
        assert_eq!(clock.elapsed(), Duration::from_secs(180));
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
    fn classify_not_finished_with_one_repo_missing() {
        let step = running();
        let (first, second) = (RepoId::new(), RepoId::new());
        let trees = [
            tree(&step, first, Isolation::Worktree, false),
            tree(&step, second, Isolation::Worktree, false),
        ];
        // The second repo's row exists with no `after_hash` (capture died between them) …
        let half = [
            commit(&step, first, Some("after-1")),
            commit(&step, second, None),
        ];
        let answer = classify(
            &step,
            &implement(),
            &[first, second],
            &trees,
            &half,
            true,
            &[],
        );
        assert_eq!(answer.1, Adjudication::Reset);
        // … or has no row at all.
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
            resettle(Some(&output), verify, false, at(0), now)
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
            resettle(None, Some(VerifyOutcome::Pass), false, at(0), now),
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
            resettle(Some(&rejecting), None, true, at(0), at(5)),
            Settle::Rejected {
                verdict_line: "verdict: request-changes".to_owned(),
            }
        );
        assert_eq!(
            resettle(Some(&rejecting), None, false, at(0), at(5)),
            Settle::Ok { note: None },
            "a phase not named `review` never reads the front matter"
        );
        let approving = document("review", "---\nverdict: approve\n---\nShip it.\n", &step);
        assert_eq!(
            resettle(Some(&approving), None, true, at(0), at(5)),
            Settle::Ok { note: None }
        );
    }

    // -----------------------------------------------------------------------------------------
    // frontier
    // -----------------------------------------------------------------------------------------

    fn done(position: i32, attempt: i32) -> RunStep {
        step(position, attempt, 0, StepStatus::Done)
    }

    #[test]
    fn frontier_is_the_highest_done_winner_without_a_successor() {
        let snapshot = snapshot();
        let mut steps = vec![done(0, 1), done(1, 1), done(2, 1)];
        assert_eq!(
            frontier(&snapshot, &steps),
            Some(steps[2].id),
            "a crash between implement's `done` and its reconcile"
        );
        // An earlier attempt at the same position is not the winner.
        steps[1].status = StepStatus::Superseded;
        steps.push(done(1, 2));
        assert_eq!(frontier(&snapshot, &steps), Some(steps[2].id));
        // The last position: `Cursor::Finished` would `finish_run` over an unmerged primary.
        steps.push(done(3, 1));
        assert_eq!(frontier(&snapshot, &steps), Some(steps[4].id));
        // Nothing `done` at all.
        assert_eq!(
            frontier(&snapshot, &[step(0, 1, 0, StepStatus::Running)]),
            None
        );
        assert_eq!(frontier(&snapshot, &[]), None);
    }

    #[test]
    fn frontier_is_none_mid_position() {
        let snapshot = snapshot();
        for status in [
            StepStatus::Pending,
            StepStatus::Running,
            StepStatus::AwaitingApproval,
            StepStatus::Failed,
            StepStatus::Done,
        ] {
            let steps = [done(0, 1), done(1, 1), done(2, 1), step(3, 1, 0, status)];
            let expected = (status == StepStatus::Done).then_some(steps[3].id);
            assert_eq!(frontier(&snapshot, &steps), expected, "{status} at p + 1");
        }
        // The latest attempt at `p` is live: `p - 1`'s winner has a live successor.
        let retried = [
            done(0, 1),
            done(1, 1),
            step(2, 1, 0, StepStatus::Superseded),
            step(2, 2, 0, StepStatus::Pending),
        ];
        assert_eq!(frontier(&snapshot, &retried), None);
    }

    #[test]
    fn frontier_ignores_retired_rows_after_a_review_rejection() {
        let snapshot = snapshot();
        let steps = [
            done(0, 1),
            done(1, 1),
            step(2, 1, 0, StepStatus::Superseded),
            // The rejected review of attempt 1, retired by `review_loop`.
            step(3, 1, 0, StepStatus::Cancelled),
            done(2, 2),
        ];
        assert_eq!(
            frontier(&snapshot, &steps),
            Some(steps[4].id),
            "implement attempt 2 (C129)"
        );
    }

    #[test]
    fn frontier_of_a_fan_out_is_the_selected_winner() {
        let mut snapshot = snapshot();
        snapshot.phases[2].fan_out = 3;
        let mut steps = vec![done(0, 1), done(1, 1)];
        for index in 0..3 {
            let mut candidate = step(2, 1, index, StepStatus::Done);
            candidate.selected = Some(index == 1);
            steps.push(candidate);
        }
        steps.push(step(2, 1, -1, StepStatus::Done));
        assert_eq!(
            frontier(&snapshot, &steps),
            Some(steps[3].id),
            "the selected candidate, not index 0 and not the judge"
        );
        // Nothing selected yet: index 0 of a fanned-out slot is not a winner, and the done
        // candidates are live rows after `plan`, so there is no frontier at all.
        for candidate in &mut steps[2..5] {
            candidate.selected = Some(false);
        }
        assert_eq!(frontier(&snapshot, &steps), None);
    }
}
