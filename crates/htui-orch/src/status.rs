//! Where the walk is, and whether it may take another step.
//!
//! Three things milestone 2 needs before anything can walk: ANA-2 §4.2's retry admission
//! predicate (plan D3), the typed failure vocabulary whose `Display` renders ANA-2's exact bytes
//! (plan D12), and the cursor the engine re-derives from `run_steps` on every call rather than
//! remembering across one (plan D16).

use core::fmt;

use htui_core::model::{GraphSnapshot, RunStep, StepId, StepStatus};

/// The only `run_step.fanout_index` milestone 2 walks.
///
/// Fan-out is milestone 4's: it adds candidates at `1..fan_out` and a judge at `-1`
/// (`docs/ANA-2.md` §4.5), and until then a run has exactly one step per `(position, attempt)`.
/// Named rather than spelled `0` at each of the three sites so milestone 4 can find them.
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
    /// Every position of the snapshot has a `done` step at its latest attempt.
    Finished,
}

/// The latest-attempt step at `position`, or `None` when the walk has not reached it.
///
/// Only `fanout_index = 0` is considered; fan-out's other indices and its `-1` judge step are
/// milestone 4's.
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

/// Walks `snapshot.phases` in position order; the first position whose latest step is not `done`
/// decides.
///
/// `superseded` and `cancelled` at the latest attempt count as "no live step here" and yield
/// [`Cursor::Create`] at [`next_attempt`] — the lazy re-insertion plan D5's review loop relies on,
/// which supersedes the chain and leaves the creating to the walk.
///
/// Every status a row can hold is matched, including the ones §4.3 would not have produced: MOD-2
/// inserts `run_step` rows outside the status law on purpose, so the walk must never assume a row
/// passed through `can_move_to` (plan D17).
#[must_use]
pub fn cursor(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Cursor {
    for phase in &snapshot.phases {
        let position = phase.position;
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

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{GraphSnapshot, RunStep, StepStatus};

    use crate::status::{Cursor, RunFailure, cursor, latest_at, may_attempt, next_attempt};

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

    /// Only `fanout_index = 0` is walked this milestone: `RUN_3`'s judge step would be `-1` and
    /// its losers `1..n`, and milestone 4 is what teaches the cursor about them.
    #[test]
    fn cursor_reads_the_zeroth_fanout_index_only() {
        let mut steps = steps_of(ids::RUN_1);
        let mut loser = steps[3].clone();
        loser.id = ids::STEP_R3_RESEARCH_B;
        loser.fanout_index = 1;
        loser.attempt = 2;
        loser.status = StepStatus::Superseded;
        steps.push(loser);
        assert_eq!(
            cursor(&snapshot(), &steps),
            Cursor::Finished,
            "a non-zero fan-out index is not the walk's cursor"
        );
    }
}
