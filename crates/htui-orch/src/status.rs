//! Where the walk is, and whether it may take another step.
//!
//! Three things milestone 2 needs before anything can walk: ANA-2 §4.2's retry admission
//! predicate (plan D3), the typed failure vocabulary whose `Display` renders ANA-2's exact bytes
//! (plan D12), and the cursor the engine re-derives from `run_steps` on every call rather than
//! remembering across one (plan D16).

use core::fmt;

use htui_core::model::{GraphSnapshot, RunStep, StepId, StepStatus};

/// The `run_step.fanout_index` a `fan_out = 1` phase walks, and the one every fan-out slot has.
///
/// A fanned-out phase holds candidates at `0..fan_out` and a judge at `-1` (`docs/ANA-2.md` §4.5);
/// [`cursor`] reads such a slot as a group (plan D59), while [`latest_at`], [`next_attempt`] and
/// [`winner_at`]'s fallback keep reading index 0, which a group created whole always holds.
const WALKED_FANOUT_INDEX: i32 = 0;

/// ANA-2 §4.2's retry admission, read prospectively (plan D3): may `next_attempt` be *created*?
///
/// ANA-2 writes the predicate as the bare `attempt <= retry_limit + 1` four times (`:487`,
/// `:579`, `:646`, `:1562`) and prospectively once (`:716`). Only the prospective reading yields
/// the two attempts `:487` says the shipped `retry_limit = 1` permits — the retrospective one
/// reads the *existing* step's attempt and admits a third. One helper, so the sites cannot drift
/// apart again.
#[must_use]
pub const fn may_attempt(next_attempt: i32, retry_limit: i32) -> bool {
    next_attempt <= retry_limit + 1
}

/// Why a run stopped: `run.failure` text and refusal wording in one vocabulary (plan D12).
///
/// ANA-2 mixes prose strings and identifier-shaped codes with no rule, and two of them are
/// asserted verbatim by its validation criteria 6 and 14 (`docs/ANA-2.md:2095`, `:2101`). A typed
/// enum with one [`fmt::Display`] is how the engine reasons over variants while the bytes stay
/// exact; the same reason milestone 1 gave the refusal sentences of
/// `crates/htui-core/src/store/traits.rs:1053-1130` one named function apiece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
    /// Stage 3: an `input_kinds` entry resolved to no document (`docs/ANA-2.md:414`). Carries the
    /// `document.kind` that was missing.
    MissingInput(String),
    /// Stage 5: no document of the phase's `output_kind` was produced by this step (`:430`).
    MissingOutput,
    /// Stage 1: no candidate survives the inline-approval interlock (`:482`). The phase is gated
    /// and every candidate is CLI-only, so there is nothing to schedule it onto.
    MissingCapability,
    /// §4.4 escalation (`:746`): the review loop ran out of budget. Carries the attempt count.
    ReviewLoopExhausted(i32),
    /// §4.4 step 1 found no position to loop back to — a `review` at position 0, or one with no
    /// predecessor (plan D5). ANA-2 names the case and not the string.
    NoLoopTarget,
    /// A human rejected a gated step of a phase that cannot loop (`docs/ANA-2.md:578`, blueprint
    /// A-8). Carries the phase name.
    Rejected {
        /// `step_graph_phase.name` of the rejected step.
        phase: String,
    },
    /// Stage 1 left no eligible candidate (plan D62): the `R-AGT-8` walk skipped every agent for a
    /// reason other than the inline-approval interlock, or `graph::resolve` reached rung 4. An
    /// empty `detail` renders with no `; ` suffix, which is the `StartRun` rung-4 note.
    NoCandidateAgent {
        /// `step_graph_phase.name` of the phase that found nothing.
        phase: String,
        /// `<agent> (<reason>), …` for every skipped agent; empty when there was no walk.
        detail: String,
    },
    /// Every candidate of a fan-out group failed, so there is nothing to select (plan D48).
    NoSurvivingCandidate {
        /// `step_graph_phase.name` of the fanned-out phase.
        phase: String,
    },
    /// Plan D94: the sweep failed a step it found `running` after the lease expired, and parked
    /// the run (ANA-2 §4.9 `:1298-1299`). `run.failure` stays NULL (R-3): this is the typed
    /// reason of the sweep's park, not a column.
    Interrupted {
        /// `step_graph_phase.name` of the interrupted step.
        phase: String,
        /// Whether the step's trees were reset (D92), or left exactly as found (D93).
        reset: bool,
    },
    /// Plan D131: a `running` run whose latest step is `failed`, out of budget and not
    /// interrupted, is a settle that crashed before its `finish_run`. The step's own failure is
    /// not on any row, so the run ends on the budget that stopped it.
    RetryBudgetSpent {
        /// `step_graph_phase.name` of the failed step.
        phase: String,
        /// `run_step.attempt` of the failed step, the last the budget allowed.
        attempt: i32,
    },
}

impl fmt::Display for RunFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInput(kind) => write!(f, "missing input document: {kind}"),
            Self::MissingOutput => f.write_str("missing_output"),
            Self::MissingCapability => f.write_str("missing_capability: inline_approval"),
            Self::ReviewLoopExhausted(attempts) => {
                write!(f, "review loop exhausted after {attempts} attempts")
            }
            Self::NoLoopTarget => f.write_str("no_loop_target"),
            Self::Rejected { phase } => write!(f, "rejected: {phase}"),
            Self::NoCandidateAgent { phase, detail } if detail.is_empty() => {
                write!(f, "no_candidate_agent: phase `{phase}`")
            }
            Self::NoCandidateAgent { phase, detail } => {
                write!(f, "no_candidate_agent: phase `{phase}`; {detail}")
            }
            Self::NoSurvivingCandidate { phase } => write!(f, "no_surviving_candidate: {phase}"),
            Self::Interrupted { phase, reset: true } => write!(f, "interrupted: {phase}"),
            Self::Interrupted {
                phase,
                reset: false,
            } => write!(f, "interrupted, tree not reset: {phase}"),
            Self::RetryBudgetSpent { phase, attempt } => {
                write!(
                    f,
                    "retry budget spent: step `{phase}` attempt {attempt} failed"
                )
            }
        }
    }
}

/// Where the walk is, re-derived from `run_steps` on every call (plan D16).
///
/// The engine holds none of this across a call, which is what milestone 5's recovery sweep needs:
/// a sweep is only correct if there was no cross-restart state for it to have missed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cursor {
    /// No live or finished step at `position`: create `(position, attempt, 0)` at `pending`.
    Create {
        /// `run_step.position`, a dense index into `snapshot.phases`.
        position: i32,
        /// `run_step.attempt`, 1-based, from [`next_attempt`].
        attempt: i32,
    },
    /// A `pending` step exists at the first unfinished position: run it.
    Run(StepId),
    /// A `running`, `awaiting_approval` or `failed` step: the walk cannot advance on its own, and
    /// what happens next is a session, a human's gate answer or the review loop.
    Rest {
        /// The step the walk stopped on.
        step: StepId,
        /// Its status, so the caller need not read the row again.
        status: StepStatus,
    },
    /// Plan D59: a fan-out slot with fewer than `fan_out` candidates, or with one still
    /// `pending`: the engine creates the missing indices and drives the group together.
    Fan {
        /// `run_step.position` of the slot.
        position: i32,
        /// `run_step.attempt` of the slot.
        attempt: i32,
    },
    /// Plan D59: every candidate of a fan-out slot settled and none is `selected`: route the
    /// group (plan D49) — a winner, the judge, a human, or the group's failure.
    Select {
        /// `run_step.position` of the slot.
        position: i32,
        /// `run_step.attempt` of the slot.
        attempt: i32,
    },
    /// Every position of the snapshot has a `done` step at its latest attempt.
    Finished,
}

/// The latest-attempt step at `position`, or `None` when the walk has not reached it.
///
/// Only `fanout_index = 0` is considered: a `fan_out = 1` phase's one row, or any slot's first
/// candidate. [`cursor`] reads a fanned-out slot whole (plan D59).
#[must_use]
pub fn latest_at(steps: &[RunStep], position: i32) -> Option<&RunStep> {
    steps
        .iter()
        .filter(|step| step.position == position && step.fanout_index == WALKED_FANOUT_INDEX)
        .max_by_key(|step| step.attempt)
}

/// `max(attempt at position) + 1`, or `1` when nothing has run there.
///
/// Read on every call rather than carried, so `UNIQUE (run_id, position, attempt, fanout_index)`
/// cannot be tripped by a stale count (blueprint H-4).
#[must_use]
pub fn next_attempt(steps: &[RunStep], position: i32) -> i32 {
    latest_at(steps, position).map_or(1, |step| step.attempt + 1)
}

/// The `fanout_index` of a fan-out slot's judge row (`docs/ANA-2.md` §4.5, plan D53).
const JUDGE_FANOUT_INDEX: i32 = -1;

/// The candidates of the slot `(position, attempt)` — every row with `fanout_index >= 0` — in
/// `fanout_index` order (plan D59).
///
/// A single-candidate phase's slot is its one `fanout_index 0` row; the judge is never in the
/// group ([`judge_at`] answers it).
#[must_use]
pub fn group_at(steps: &[RunStep], position: i32, attempt: i32) -> Vec<&RunStep> {
    let mut group: Vec<&RunStep> = steps
        .iter()
        .filter(|step| step.position == position && step.attempt == attempt)
        .filter(|step| step.fanout_index >= 0)
        .collect();
    group.sort_by_key(|step| step.fanout_index);
    group
}

/// The judge row (`fanout_index = -1`) of the slot `(position, attempt)`, if one was created.
#[must_use]
pub fn judge_at(steps: &[RunStep], position: i32, attempt: i32) -> Option<&RunStep> {
    steps.iter().find(|step| {
        step.position == position
            && step.attempt == attempt
            && step.fanout_index == JUDGE_FANOUT_INDEX
    })
}

/// The slot's winner (plan D66): the candidate carrying `selected = true`, else its
/// `fanout_index 0` when nothing in the slot is selected.
///
/// The fallback is what makes a `fan_out = 1` phase read exactly as it did before fan-out: its one
/// row is never selected, and it is index 0. `None` only when the slot has no index 0 either.
#[must_use]
pub fn winner_at(steps: &[RunStep], position: i32, attempt: i32) -> Option<&RunStep> {
    let group = group_at(steps, position, attempt);
    group
        .iter()
        .find(|step| step.selected == Some(true))
        .or_else(|| {
            group
                .iter()
                .find(|step| step.fanout_index == WALKED_FANOUT_INDEX)
        })
        .copied()
}

/// Walks `snapshot.phases` in position order; the first position whose latest step is not `done`
/// decides.
///
/// `superseded` and `cancelled` at the latest attempt count as "no live step here" and yield
/// [`Cursor::Create`] at [`next_attempt`] — the lazy re-insertion plan D5's review loop relies on,
/// which supersedes the chain and leaves the creating to the walk.
///
/// A phase with `fan_out > 1` is read as a slot (plan D59, `group_cursor`); a `fan_out = 1`
/// phase is read exactly as it was before fan-out, at `fanout_index 0`.
///
/// Every status a row can hold is matched, including the ones §4.3 would not have produced: MOD-2
/// inserts `run_step` rows outside the status law on purpose, so the walk must never assume a row
/// passed through `can_move_to` (plan D17).
#[must_use]
pub fn cursor(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Cursor {
    for phase in &snapshot.phases {
        let position = phase.position;
        if phase.fan_out > 1 {
            match group_cursor(steps, position, phase.fan_out) {
                Some(stop) => return stop,
                None => continue,
            }
        }
        let Some(step) = latest_at(steps, position) else {
            return Cursor::Create {
                position,
                attempt: 1,
            };
        };
        match step.status {
            StepStatus::Done => {}
            StepStatus::Pending => return Cursor::Run(step.id),
            StepStatus::Running | StepStatus::AwaitingApproval | StepStatus::Failed => {
                return Cursor::Rest {
                    step: step.id,
                    status: step.status,
                };
            }
            StepStatus::Superseded | StepStatus::Cancelled => {
                return Cursor::Create {
                    position,
                    attempt: next_attempt(steps, position),
                };
            }
        }
    }
    Cursor::Finished
}

/// Plan D59 for one fanned-out position; `None` when the slot is complete and the walk goes on.
///
/// The order is load-bearing (blueprint §9.3, H-14): a `selected` `done` candidate completes the
/// position before anything else is read; a slot retired whole is re-created at `attempt + 1`; a
/// `running` candidate or judge rests the walk before a `pending` sibling is driven; a short or
/// `pending` slot is driven; anything else is settled and goes to selection.
///
/// The slot's attempt is the highest over its candidates (every `fanout_index >= 0` row) rather
/// than [`latest_at`]'s index-0 reading, which agrees whenever a group was created whole (plan
/// D71, blueprint F-K) and cannot split a slot when one was not.
fn group_cursor(steps: &[RunStep], position: i32, fan_out: i32) -> Option<Cursor> {
    let Some(attempt) = steps
        .iter()
        .filter(|step| step.position == position && step.fanout_index >= 0)
        .map(|step| step.attempt)
        .max()
    else {
        return Some(Cursor::Create {
            position,
            attempt: 1,
        });
    };
    let group = group_at(steps, position, attempt);
    if group
        .iter()
        .any(|step| step.selected == Some(true) && step.status == StepStatus::Done)
    {
        return None;
    }
    if group
        .iter()
        .all(|step| matches!(step.status, StepStatus::Superseded | StepStatus::Cancelled))
    {
        return Some(Cursor::Create {
            position,
            attempt: attempt + 1,
        });
    }
    if let Some(live) = group
        .iter()
        .copied()
        .chain(judge_at(steps, position, attempt))
        .find(|step| step.status == StepStatus::Running)
    {
        return Some(Cursor::Rest {
            step: live.id,
            status: live.status,
        });
    }
    let short = i32::try_from(group.len()).is_ok_and(|len| len < fan_out);
    if short || group.iter().any(|step| step.status == StepStatus::Pending) {
        return Some(Cursor::Fan { position, attempt });
    }
    Some(Cursor::Select { position, attempt })
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{GraphSnapshot, RunStep, StepId, StepStatus};

    use crate::status::{
        Cursor, RunFailure, cursor, group_at, judge_at, latest_at, may_attempt, next_attempt,
        winner_at,
    };

    /// The four `RUN_1` steps, the one `RUN_2` step, or whatever the fixture holds for a run.
    fn steps_of(run: htui_core::model::RunId) -> Vec<RunStep> {
        demo_data()
            .steps
            .into_iter()
            .filter(|step| step.run_id == run)
            .collect()
    }

    /// The `feature` graph's snapshot, as every fixture `graph` run carries it.
    fn snapshot() -> GraphSnapshot {
        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
            .expect("the fixture snapshot is a `GraphSnapshot`")
    }

    /// ANA-2 writes five of these as prose or as identifier-shaped codes and two of them are
    /// asserted verbatim by its own validation criteria 6 and 14 (`docs/ANA-2.md:2095`, `:2101`).
    /// A typed enum is only worth having if its `Display` is byte-exact, so this is the test the
    /// module was written against.
    #[test]
    fn run_failure_display_is_ana2s_bytes() {
        assert_eq!(
            RunFailure::MissingInput("plan".to_owned()).to_string(),
            "missing input document: plan",
            "`docs/ANA-2.md:414`"
        );
        assert_eq!(
            RunFailure::MissingOutput.to_string(),
            "missing_output",
            "`docs/ANA-2.md:430`"
        );
        assert_eq!(
            RunFailure::MissingCapability.to_string(),
            "missing_capability: inline_approval",
            "`docs/ANA-2.md:482`"
        );
        assert_eq!(
            RunFailure::ReviewLoopExhausted(2).to_string(),
            "review loop exhausted after 2 attempts",
            "`docs/ANA-2.md:746`"
        );
        assert_eq!(
            RunFailure::NoLoopTarget.to_string(),
            "no_loop_target",
            "plan D5's terminal review; ANA-2 names the case and not the string"
        );
        assert_eq!(
            RunFailure::Rejected {
                phase: "implement".to_owned(),
            }
            .to_string(),
            "rejected: implement",
            "`docs/ANA-2.md:578`, blueprint A-8"
        );
        assert_eq!(
            RunFailure::NoCandidateAgent {
                phase: "implement".to_owned(),
                detail: "claude (quota: status rejected)".to_owned(),
            }
            .to_string(),
            "no_candidate_agent: phase `implement`; claude (quota: status rejected)",
            "plan D62: a stage-1 walk that left nothing eligible"
        );
        assert_eq!(
            RunFailure::NoCandidateAgent {
                phase: "prd".to_owned(),
                detail: String::new(),
            }
            .to_string(),
            "no_candidate_agent: phase `prd`",
            "plan D62's `StartRun` rung-4 note carries no detail, so no `; `"
        );
        assert_eq!(
            RunFailure::NoSurvivingCandidate {
                phase: "implement".to_owned(),
            }
            .to_string(),
            "no_surviving_candidate: implement",
            "plan D48: every candidate of the group failed"
        );
        assert_eq!(
            RunFailure::Interrupted {
                phase: "implement".to_owned(),
                reset: true,
            }
            .to_string(),
            "interrupted: implement",
            "plan D94: the sweep reset the step's trees and parked it out of budget"
        );
        assert_eq!(
            RunFailure::Interrupted {
                phase: "implement".to_owned(),
                reset: false,
            }
            .to_string(),
            "interrupted, tree not reset: implement",
            "plan D93/D94: a tree the sweep must not reset, left as found (`:1299`)"
        );
        assert_eq!(
            RunFailure::RetryBudgetSpent {
                phase: "plan".to_owned(),
                attempt: 2,
            }
            .to_string(),
            "retry budget spent: step `plan` attempt 2 failed",
            "plan D131: the sweep ends a run whose settle crashed before its `finish_run`"
        );
    }

    /// `RUN_3`'s two `research` candidates with the selection moved to index 1: the winner is
    /// whichever row carries `selected = true`, not whichever sits at index 0 (plan D66).
    #[test]
    fn winner_at_prefers_the_selected_candidate() {
        let mut steps = steps_of(ids::RUN_3);
        for step in &mut steps {
            step.selected = Some(step.fanout_index == 1);
        }
        let winner = winner_at(&steps, 0, 1).expect("the slot has candidates");
        assert_eq!(winner.id, ids::STEP_R3_RESEARCH_B);
        assert_eq!(
            winner_at(&steps, 0, 2),
            None,
            "an attempt with no rows has no winner"
        );
    }

    /// A slot nothing has selected yet — a single-candidate phase, or a group still being driven —
    /// answers its `fanout_index 0`, which every slot has (plan D59, D66).
    #[test]
    fn winner_at_falls_back_to_index_zero_when_nothing_is_selected() {
        for selected in [None, Some(false)] {
            let mut steps = steps_of(ids::RUN_3);
            for step in &mut steps {
                step.selected = selected;
            }
            // Index 1 first, so the fallback cannot be "the first row in the slice".
            steps.reverse();
            let winner = winner_at(&steps, 0, 1).expect("the slot has candidates");
            assert_eq!(
                winner.id,
                ids::STEP_R3_RESEARCH_A,
                "`selected = {selected:?}` everywhere falls back to index 0"
            );
        }
    }

    /// The judge shares its slot with the candidates but is not one of them: `group_at` skips
    /// `fanout_index = -1`, orders by index, and `judge_at` answers the row it skipped.
    #[test]
    fn group_at_excludes_the_judge() {
        let mut steps = steps_of(ids::RUN_3);
        let mut judge = steps[0].clone();
        judge.id = StepId::new();
        judge.fanout_index = -1;
        judge.selected = None;
        let mut third = steps[0].clone();
        third.id = StepId::new();
        third.fanout_index = 2;
        third.selected = Some(false);
        let mut next = steps[0].clone();
        next.id = StepId::new();
        next.attempt = 2;
        // Out of index order on purpose, with a row of the next attempt mixed in.
        steps.insert(0, third.clone());
        steps.insert(1, judge.clone());
        steps.push(next);

        let group: Vec<_> = group_at(&steps, 0, 1).iter().map(|step| step.id).collect();
        assert_eq!(
            group,
            vec![ids::STEP_R3_RESEARCH_A, ids::STEP_R3_RESEARCH_B, third.id],
            "the candidates of `(0, 1)`, in `fanout_index` order, without the judge"
        );
        assert_eq!(judge_at(&steps, 0, 1).map(|step| step.id), Some(judge.id));
        assert_eq!(judge_at(&steps, 0, 2), None, "attempt 2 has no judge");
        assert!(group_at(&steps, 1, 1).is_empty());
    }

    /// Plan D3: the predicate is about the attempt that is *about to be created*, so the shipped
    /// `retry_limit = 1` permits two attempts and not three — which is what ANA-2 says in the same
    /// sentence it writes the bare form in (`docs/ANA-2.md:487`).
    #[test]
    fn may_attempt_is_prospective() {
        assert!(may_attempt(1, 1), "the first attempt is always admissible");
        assert!(may_attempt(2, 1), "one retry: the second attempt");
        assert!(
            !may_attempt(3, 1),
            "a third attempt against `retry_limit = 1` is the reading D3 refuses"
        );
        assert!(may_attempt(1, 0), "`retry_limit = 0` still permits one");
        assert!(!may_attempt(2, 0), "and only one");
    }

    #[test]
    fn latest_at_takes_the_highest_attempt_of_the_position() {
        let mut steps = steps_of(ids::RUN_1);
        let mut retry = steps[2].clone();
        retry.id = ids::STEP_R2_PRD; // any other id; the fixture mints no second attempt
        retry.attempt = 2;
        retry.status = StepStatus::Pending;
        steps.push(retry);

        let found = latest_at(&steps, 2).expect("position 2 has steps");
        assert_eq!(found.attempt, 2);
        assert_eq!(next_attempt(&steps, 2), 3);
        assert_eq!(
            next_attempt(&steps, 1),
            2,
            "one attempt exists at position 1"
        );
        assert_eq!(
            next_attempt(&steps, 9),
            1,
            "a position with no step starts at 1"
        );
        assert!(latest_at(&steps, 9).is_none());
    }

    /// `RUN_1` is the fixture's finished run: four `done` steps against the four-phase `feature`
    /// snapshot.
    #[test]
    fn cursor_on_four_done_steps_is_finished() {
        assert_eq!(cursor(&snapshot(), &steps_of(ids::RUN_1)), Cursor::Finished);
    }

    /// `RUN_2` holds one `pending` `prd` step and nothing else.
    #[test]
    fn cursor_on_a_pending_step_runs_it() {
        assert_eq!(
            cursor(&snapshot(), &steps_of(ids::RUN_2)),
            Cursor::Run(ids::STEP_R2_PRD)
        );
    }

    /// The position the walk has not reached yet is the one to create, at attempt 1.
    #[test]
    fn cursor_on_a_gap_creates_the_position() {
        let steps: Vec<RunStep> = steps_of(ids::RUN_1)
            .into_iter()
            .filter(|step| step.position < 2)
            .collect();
        assert_eq!(
            cursor(&snapshot(), &steps),
            Cursor::Create {
                position: 2,
                attempt: 1
            }
        );
    }

    /// Plan D5's lazy re-insertion: the review loop supersedes the implement step and leaves the
    /// creating to the walk, so a superseded latest attempt reads as "no live step here".
    #[test]
    fn cursor_after_a_supersede_creates_the_next_attempt() {
        let mut steps = steps_of(ids::RUN_1);
        steps[2].status = StepStatus::Superseded;
        assert_eq!(
            cursor(&snapshot(), &steps),
            Cursor::Create {
                position: 2,
                attempt: 2
            }
        );
    }

    /// A step the walk cannot advance past: `running`, `awaiting_approval` and `failed` all rest,
    /// and the engine's caller decides what happens next.
    #[test]
    fn cursor_rests_on_a_step_the_walk_cannot_advance() {
        for status in [
            StepStatus::Running,
            StepStatus::AwaitingApproval,
            StepStatus::Failed,
        ] {
            let mut steps = steps_of(ids::RUN_1);
            steps[3].status = status;
            assert_eq!(
                cursor(&snapshot(), &steps),
                Cursor::Rest {
                    step: steps[3].id,
                    status
                },
                "a `{status}` step at the last position rests"
            );
        }
    }

    /// Plan D59: a `fan_out = 1` phase is walked exactly as before fan-out — its cursor reads
    /// `fanout_index 0` only. `RUN_1`'s `review` has `fan_out = 1`, so a stray index-1 row at a
    /// later attempt (the shape of a loser or a judge) does not move the cursor off `Finished`.
    #[test]
    fn cursor_reads_the_zeroth_fanout_index_only() {
        let snapshot = snapshot();
        assert_eq!(
            snapshot.phases[3].fan_out, 1,
            "the fixture's `review` is single"
        );
        let mut steps = steps_of(ids::RUN_1);
        let mut loser = steps[3].clone();
        loser.id = ids::STEP_R3_RESEARCH_B;
        loser.fanout_index = 1;
        loser.attempt = 2;
        loser.status = StepStatus::Superseded;
        steps.push(loser);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Finished,
            "a non-zero fan-out index is not a `fan_out = 1` phase's cursor"
        );
    }

    /// `RUN_1`'s snapshot with `prd` (position 0) fanned out three ways, and `RUN_1`'s steps with
    /// its `prd` row replaced by `group`: one candidate per `(fanout_index, status, selected)`.
    fn fanned(group: &[(i32, StepStatus, Option<bool>)]) -> (GraphSnapshot, Vec<RunStep>) {
        let mut snapshot = snapshot();
        snapshot.phases[0].fan_out = 3;
        let mut steps = steps_of(ids::RUN_1);
        let prd = steps.remove(0);
        assert_eq!(prd.position, 0, "the first `RUN_1` row is `prd`");
        for (index, status, selected) in group {
            let mut candidate = prd.clone();
            candidate.id = StepId::new();
            candidate.fanout_index = *index;
            candidate.status = *status;
            candidate.selected = *selected;
            steps.push(candidate);
        }
        (snapshot, steps)
    }

    /// Plan D59's `Fan`: a group with a `pending` candidate, or with fewer than `fan_out`, is
    /// driven — the walk does not run index 0 alone and wait on the rest.
    #[test]
    fn cursor_drives_an_incomplete_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Done, None),
            (1, StepStatus::Pending, None),
            (2, StepStatus::Pending, None),
        ]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Fan {
                position: 0,
                attempt: 1
            },
            "a `pending` candidate is driven with its group"
        );

        let (snapshot, steps) = fanned(&[(0, StepStatus::Done, None), (1, StepStatus::Done, None)]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Fan {
                position: 0,
                attempt: 1
            },
            "two of three: the missing index is created by the drive"
        );
    }

    /// Plan D59's `Select`: every candidate settled — `done` or `failed` — and none selected.
    #[test]
    fn cursor_selects_a_settled_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Done, None),
            (1, StepStatus::Failed, None),
            (2, StepStatus::Done, None),
        ]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Select {
                position: 0,
                attempt: 1
            }
        );
    }

    /// A `selected` `done` candidate completes the position, wherever it sits in the group — here
    /// at index 2, with index 0 a superseded loser the old cursor would have re-created.
    #[test]
    fn cursor_passes_a_selected_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Superseded, Some(false)),
            (1, StepStatus::Superseded, Some(false)),
            (2, StepStatus::Done, Some(true)),
        ]);
        assert_eq!(cursor(&snapshot, &steps), Cursor::Finished);
    }

    /// A group retired whole — superseded by the review loop or cancelled by a retry — is "no
    /// live slot here", and the next attempt is created.
    #[test]
    fn cursor_creates_the_next_attempt_after_a_retired_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Superseded, Some(false)),
            (1, StepStatus::Cancelled, Some(false)),
            (2, StepStatus::Superseded, Some(true)),
        ]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Create {
                position: 0,
                attempt: 2
            }
        );
    }

    /// Blueprint H-14: a `running` candidate or a `running` judge rests the walk, and a `running`
    /// candidate outranks a `pending` sibling — one call never drives half a group twice.
    #[test]
    fn a_running_judge_rests() {
        let (snapshot, mut steps) = fanned(&[
            (0, StepStatus::Done, None),
            (1, StepStatus::Done, None),
            (2, StepStatus::Done, None),
        ]);
        let mut judge = steps[steps.len() - 1].clone();
        judge.id = StepId::new();
        judge.fanout_index = -1;
        judge.status = StepStatus::Running;
        steps.push(judge.clone());
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Rest {
                step: judge.id,
                status: StepStatus::Running
            }
        );

        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Running, None),
            (1, StepStatus::Pending, None),
            (2, StepStatus::Pending, None),
        ]);
        assert!(
            matches!(
                cursor(&snapshot, &steps),
                Cursor::Rest {
                    status: StepStatus::Running,
                    ..
                }
            ),
            "a live candidate rests the walk before its pending siblings are driven"
        );
    }
}
