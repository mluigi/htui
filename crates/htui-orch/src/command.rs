//! The three operator commands of ANA-2 §6.2 (`docs/ANA-2.md:1557-1572`) and their enabling
//! guards: `StartRun`, `AnswerGate` and `RetryStep`.
//!
//! §6.2's table has an "Enabled when" column, and the Runs tab (milestone 6) greys an action out
//! by exactly the rule the engine refuses it by. That only stays true if there is one rule, so the
//! rules live here as free functions over rows — no store handle, no clock, no engine — and both
//! callers read them. A guard that needs a row the caller has not got is a guard the Runs tab will
//! quietly reimplement.
//!
//! **[`Rest`] and [`EngineError`] live here rather than in `engine.rs`.** The blueprint's §5.1 puts
//! them beside the `Engine`, but [`CommandOutcome`] carries the first and the two guards return the
//! second, and `engine.rs` is still T4's stub — so they have to be in a module this task owns. The
//! home is defensible on its own terms: this is the vocabulary of the crate's command surface, and
//! milestone 6's Runs tab speaks it without ever constructing an `Engine`. T4 uses both from here.

use htui_core::model::{
    ItemId, RepoId, RunId, RunMode, RunStatus, RunStep, SnapshotPhase, StepId, StepStatus,
};
use htui_core::prompt::AssembleError;
use htui_core::store::StoreError;

use crate::graph::ResolveError;
use crate::isolate::IsolateError;
use crate::status::{RunFailure, may_attempt};

/// What a human asks the orchestrator to do, in manual mode (ANA-2 §6.2, `docs/ANA-2.md:1557`).
///
/// Three of §6.2's ten; `PromoteStep`, `CancelRun`, `CancelStep`, `OpenArtifact`, `SelectFanout`,
/// `AcceptArtifact`, `Unblock` and `CloseOut` belong to milestones 3 to 6 and are deliberately not
/// declared here — an enum arm with no dispatcher arm is a promise the walk has not made.
///
/// **`AnswerGate` and `RetryStep` carry `run` as well as `step`** because there is no per-step read
/// on the store seam: `ReadStore::run_steps` is per run
/// (`crates/htui-core/src/store/traits.rs:149`) and no `step(id)` exists (blueprint H-11). Asking
/// the caller for the run it already knows is cheaper than adding a twentieth read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `R-TUI-2`'s `run` and `queue` (`docs/ANA-2.md:1571-1572`): resolve the item's graph, create
    /// and claim a run, and walk it until it rests.
    StartRun {
        /// The item to run. Its status moves `open -> queued` inside `create_run`.
        item: ItemId,
        /// `manual` this milestone; `auto` is §4.10's and changes no gate until milestone 7.
        mode: RunMode,
        /// `None` resolves to the project's primary repo, or to nothing when it has none; an
        /// explicit empty scope on an item with a primary repo is refused (plan D14).
        repo_scope: Option<Vec<RepoId>>,
    },
    /// §6.2's `approve` and `reject with note`: a human answers a parked gate.
    AnswerGate {
        /// The run the step belongs to (blueprint H-11).
        run: RunId,
        /// The parked step.
        step: StepId,
        /// What the human decided.
        answer: GateAnswer,
    },
    /// §6.2's `retry`: supersede the answered or failed step and create the next attempt.
    RetryStep {
        /// The run the step belongs to (blueprint H-11).
        run: RunId,
        /// The step to retry.
        step: StepId,
    },
}

/// A human's verdict on a parked gate (`docs/ANA-2.md:1559-1560`).
///
/// `Skipped` is §4.8's "accept artifact" shape — the gate passes without the agent having earned
/// it — and is admitted by [`answer_gate_enabled`] on the same terms as `Approved` minus the
/// document, because the human is the producer in that path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateAnswer {
    /// The step's artefact is accepted; the walk advances to `position + 1`.
    Approved,
    /// The step's artefact is rejected, with the note that becomes the step's `gate_note` and,
    /// when the phase is loopable, the review loop's input (§4.4).
    Rejected {
        /// `run_step.gate_note`; ANA-2's `reject with note` action requires one.
        note: String,
    },
    /// The gate is passed without an approval verdict: the human supplied the artefact.
    Skipped,
}

/// Where a walk stopped, and why.
///
/// `position` is the snapshot position the walk was on when it stopped, and is `None` exactly when
/// the run finished — there is no position left to name.
///
/// **`failure` is populated only by the transition that caused the stop**, and is then the same
/// value that was written to `run.failure` (except on the escalation path, where a parked run's
/// `failure` stays NULL by blueprint A-4 and this is the only place the caller can read the
/// reason). It is `None` — even for a run whose `run.failure` is set — whenever the walk merely
/// *found* the run already terminal or already parked, from an earlier call or another process:
/// `RunFailure`'s `Display` is one-way by plan D12, so reconstructing the variant from the column
/// would mean a second copy of that grammar. `run.failure` is the durable record; this field is
/// what the call that ended the run reports about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rest {
    /// The run's status after the walk stopped.
    pub run: RunStatus,
    /// The position the walk rested on; `None` when every position is `done`.
    pub position: Option<i32>,
    /// Why, when the stop was a refusal or a failure.
    pub failure: Option<RunFailure>,
}

/// What a dispatched [`Command`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// [`Command::StartRun`]: the run that was created, and where its first walk stopped.
    Started {
        /// The new `run.id`.
        run: RunId,
        /// Where the walk rested.
        rest: Rest,
    },
    /// [`Command::AnswerGate`]: where the resumed walk stopped.
    Answered {
        /// Where the walk rested.
        rest: Rest,
    },
    /// [`Command::RetryStep`]: the step that was superseded, and where the resumed walk stopped.
    Retried {
        /// The step the retry replaced.
        step: StepId,
        /// Where the walk rested.
        rest: Rest,
    },
}

/// Why the engine refused, or what it could not get past.
///
/// The transparent arms are the seams the walk borrows — the store, resolution, the isolator, the
/// prompt assembler, the recorder and the driver — and carry their own sentences. The named arms
/// are the walk's own refusals, and each of them is a row a human can read, per ANA-2 invariant 7
/// (`docs/ANA-2.md:132-135`).
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The store refused a read or a write.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The item's graph could not be resolved into a snapshot.
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// The run is not in the status the command needs — a `queued` run handed to the walk, say.
    #[error("run {run} is `{status}`; expected `{expected}`")]
    RunStatus {
        /// The run read.
        run: RunId,
        /// What it actually is.
        status: RunStatus,
        /// What the command needs it to be.
        expected: &'static str,
    },
    /// The step is not in a status this command is enabled at (ANA-2 §6.2's "Enabled when").
    #[error("step {step} is `{status}`; expected `{expected}` (ANA-2 §6.2)")]
    NotGated {
        /// The step read.
        step: StepId,
        /// What it actually is.
        status: StepStatus,
        /// The statuses the command is enabled at.
        expected: &'static str,
    },
    /// ANA-2 §12 criterion 5 (`docs/ANA-2.md:2096`): approving a step that produced no artefact.
    #[error("step {step} has no `{kind}` document; approve needs one (ANA-2 §6.2)")]
    MissingOutputForApproval {
        /// The step that would have been approved.
        step: StepId,
        /// `step_graph_phase.output_kind`, named so the human knows what to write.
        kind: String,
    },
    /// The retry budget is spent (plan D3's prospective predicate).
    #[error("step {step} is at attempt {attempt}; retry_limit {retry_limit} permits no more")]
    RetryExhausted {
        /// The step that would have been retried.
        step: StepId,
        /// Its current `attempt`.
        attempt: i32,
        /// `SnapshotPhase::retry_limit`, the number of *additional* attempts (`:485-487`).
        retry_limit: i32,
    },
    /// Blueprint F-J / R-4: `Status::can_move_to` has no `blocked -> in_progress` edge
    /// (`crates/htui-core/src/model/item.rs:46-60`), so a blocked item cannot be resumed until
    /// milestone 6 ships `Unblock` (`docs/ANA-2.md:1568-1569`).
    #[error("item {item} is blocked; milestone 6's `Unblock` clears it (blueprint R-4)")]
    ItemBlocked {
        /// The item holding the walk.
        item: ItemId,
    },
    /// `claim_run` answered `Ok(false)`: the box is at `max_concurrent_items` or the run's scope
    /// overlaps a live one (ANA-2 §4.7). The run stays `queued` and the caller may retry later —
    /// the predicate itself is milestone 5's (plan D6, R-2).
    #[error("claim refused: the box is full or the scope overlaps (ANA-2 §4.7)")]
    ClaimRefused {
        /// The run that stayed queued.
        run: RunId,
    },
    /// The run carries a `graph_snapshot` at a version this engine does not read.
    #[error("run {run} snapshot v{v} is not readable by this engine")]
    SnapshotVersion {
        /// The run read.
        run: RunId,
        /// `GraphSnapshot::v` as stored.
        v: u32,
    },
    /// The run's own `graph_snapshot` cannot answer something the walk must ask of it: a position
    /// with no phase, or a step naming no agent.
    ///
    /// An **engine** invariant and not a store refusal. Invariant 2 makes the snapshot the single
    /// thing a live run reads (`docs/ANA-2.md:109-113`), so there is no second source to fall back
    /// to and nothing the store did wrong; reporting it as `StoreError::Constraint` would put an
    /// engine bug behind a variant milestone 6's `run_worker.rs` will be matching for store
    /// outages. The sentence is what a human reads, per invariant 7.
    #[error("run {run} snapshot: {reason}")]
    Snapshot {
        /// The run whose snapshot came up short.
        run: RunId,
        /// What was asked of it, in a readable sentence.
        reason: String,
    },
    /// The walk spent its whole iteration budget without reaching a rest.
    ///
    /// Every pass either creates a step, runs one, or stops, and both the retry budget and the
    /// review loop's are finite — so this is a net and not a bound: a row moved under the walk by
    /// another process (plan D17) ends in a refusal a human can read rather than in a spin.
    #[error("run {run}: the walk made {passes} passes without resting; a row moved under it")]
    Stalled {
        /// The run the walk could not rest.
        run: RunId,
        /// How many passes it made, which is the budget.
        passes: u32,
    },
    /// Stage 3 could not assemble the prompt.
    #[error(transparent)]
    Prompt(#[from] AssembleError),
    /// Stage 4's recorder refused a write, or found scrub residue.
    #[error(transparent)]
    Record(#[from] htui_agent::RecordError),
    /// The driver failed, or the session's stream closed before its `done`.
    #[error(transparent)]
    Driver(#[from] htui_agent::error::DriverError),
    /// Stage 2 or stage 5's isolation verb refused. **Not in the blueprint's §5.1 list**, which
    /// leaves the engine no way to carry an `Isolator`'s own failure; added here.
    #[error(transparent)]
    Isolate(#[from] IsolateError),
}

/// §6.2's "Enabled when" for `approve`, `reject with note` and `accept artifact`
/// (`docs/ANA-2.md:1559-1560`).
///
/// `has_output` is "a document of `phase.output_kind` produced by *this step* exists", which the
/// caller resolves with `ReadStore::documents_of_kinds`
/// (`crates/htui-core/src/store/traits.rs:100`) filtered on `produced_by_step_id`. It is passed
/// rather than read so that this stays a pure function of rows, and so that criterion 5's "and no
/// write happens" is structurally true: the guard cannot write, because it holds no store.
///
/// The `phase` argument is not in the blueprint's §4.4 sketch, which passes only the step: it is
/// needed because [`EngineError::MissingOutputForApproval`] names the missing `output_kind` and
/// `RunStep` carries `phase_name` but no kind. Its presence also makes the two guards symmetric.
///
/// # Errors
/// [`EngineError::NotGated`] when the step is not `awaiting_approval`;
/// [`EngineError::MissingOutputForApproval`] when [`GateAnswer::Approved`] meets a step with no
/// artefact.
pub fn answer_gate_enabled(
    step: &RunStep,
    phase: &SnapshotPhase,
    has_output: bool,
    answer: &GateAnswer,
) -> Result<(), EngineError> {
    if step.status != StepStatus::AwaitingApproval {
        return Err(EngineError::NotGated {
            step: step.id,
            status: step.status,
            expected: "awaiting_approval",
        });
    }
    // Only `Approved` needs the artefact. `Rejected` is a verdict on its absence as much as on its
    // content, and `Skipped` is §4.8's path where the human is the producer.
    if matches!(answer, GateAnswer::Approved) && !has_output {
        return Err(EngineError::MissingOutputForApproval {
            step: step.id,
            kind: phase.output_kind.clone(),
        });
    }
    Ok(())
}

/// §6.2's "Enabled when" for `retry` (`docs/ANA-2.md:1561`): the step is `awaiting_approval` or
/// `failed`, and the attempt about to be created is within budget.
///
/// The budget is [`may_attempt`] on `step.attempt + 1`, not on `step.attempt`: plan D3's
/// prospective reading, which is the only one under which the shipped `retry_limit = 1` permits the
/// two attempts `docs/ANA-2.md:487` says it does.
///
/// Two things this guard deliberately does **not** check, because they need rows it is not given:
/// that the run is non-terminal, and that the item is not `blocked`
/// ([`EngineError::ItemBlocked`], blueprint F-J). Both are the engine's, at dispatch.
///
/// # Errors
/// [`EngineError::NotGated`] for any other step status; [`EngineError::RetryExhausted`] when the
/// budget is spent.
pub fn retry_enabled(step: &RunStep, phase: &SnapshotPhase) -> Result<(), EngineError> {
    if !matches!(
        step.status,
        StepStatus::AwaitingApproval | StepStatus::Failed
    ) {
        return Err(EngineError::NotGated {
            step: step.id,
            status: step.status,
            expected: "awaiting_approval | failed",
        });
    }
    if !may_attempt(step.attempt + 1, phase.retry_limit) {
        return Err(EngineError::RetryExhausted {
            step: step.id,
            attempt: step.attempt,
            retry_limit: phase.retry_limit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{GraphSnapshot, RunStep, SnapshotPhase, StepStatus};

    use super::{EngineError, GateAnswer, answer_gate_enabled, retry_enabled};

    /// `RUN_1`'s `review` step, at position 3, and the snapshot phase it walked.
    fn review() -> (RunStep, SnapshotPhase) {
        let data = demo_data();
        let run = data
            .runs
            .iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1")
            .clone();
        let snapshot: GraphSnapshot =
            serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
                .expect("the fixture snapshot is a `GraphSnapshot`");
        let step = data
            .steps
            .into_iter()
            .find(|step| step.run_id == ids::RUN_1 && step.position == 3)
            .expect("RUN_1 has a step at position 3");
        let phase = snapshot
            .phases
            .into_iter()
            .find(|phase| phase.position == 3)
            .expect("the snapshot has a phase at position 3");
        (step, phase)
    }

    /// ANA-2 §12 criterion 5 (`docs/ANA-2.md:2096`) is a rule about rows, so it is testable without
    /// a store: the guard refuses, names the kind, and could not have written anything if it tried.
    #[test]
    fn approve_needs_the_output_document_and_the_gate() {
        let (mut step, phase) = review();
        step.status = StepStatus::AwaitingApproval;

        assert!(answer_gate_enabled(&step, &phase, true, &GateAnswer::Approved).is_ok());

        let refused = answer_gate_enabled(&step, &phase, false, &GateAnswer::Approved)
            .expect_err("approve with no artefact is refused");
        assert!(
            matches!(&refused, EngineError::MissingOutputForApproval { step: id, kind }
                if *id == step.id && *kind == phase.output_kind),
            "{refused}"
        );
        assert!(refused.to_string().contains(&phase.output_kind));

        // A rejection is a verdict on the absence as much as on the content, and `Skipped` is
        // §4.8's path where the human supplies the artefact.
        for answer in [
            GateAnswer::Rejected {
                note: "no tests".to_owned(),
            },
            GateAnswer::Skipped,
        ] {
            assert!(
                answer_gate_enabled(&step, &phase, false, &answer).is_ok(),
                "{answer:?} needs no document"
            );
        }
    }

    /// Every status but `awaiting_approval` is refused, including `done` — answering a gate twice
    /// is the double-click the Runs tab has to grey out.
    #[test]
    fn only_a_parked_step_can_be_answered() {
        let (mut step, phase) = review();
        for status in [
            StepStatus::Pending,
            StepStatus::Running,
            StepStatus::Done,
            StepStatus::Failed,
            StepStatus::Cancelled,
            StepStatus::Superseded,
        ] {
            step.status = status;
            let refused = answer_gate_enabled(&step, &phase, true, &GateAnswer::Approved)
                .expect_err("only `awaiting_approval` is answerable");
            assert!(
                matches!(refused, EngineError::NotGated { status: got, .. } if got == status),
                "`{status}` is not a gate"
            );
        }
    }

    /// Plan D3, read through the guard: `retry_limit = 1` retries attempt 1 and refuses attempt 2.
    #[test]
    fn retry_is_enabled_by_status_and_by_the_prospective_budget() {
        let (mut step, mut phase) = review();
        phase.retry_limit = 1;

        for status in [StepStatus::AwaitingApproval, StepStatus::Failed] {
            step.status = status;
            step.attempt = 1;
            assert!(
                retry_enabled(&step, &phase).is_ok(),
                "`{status}` is retryable"
            );
        }

        step.attempt = 2;
        let refused = retry_enabled(&step, &phase).expect_err("a third attempt is out of budget");
        assert!(
            matches!(
                refused,
                EngineError::RetryExhausted {
                    attempt: 2,
                    retry_limit: 1,
                    ..
                }
            ),
            "{refused}"
        );

        step.attempt = 1;
        step.status = StepStatus::Done;
        assert!(matches!(
            retry_enabled(&step, &phase).expect_err("a done step is not retryable"),
            EngineError::NotGated { .. }
        ));
    }

    /// The named refusals are what a human reads, so their bytes are part of the contract.
    #[test]
    fn the_named_refusals_say_what_went_wrong() {
        assert_eq!(
            EngineError::ItemBlocked {
                item: ids::HTUI_FEAT_3,
            }
            .to_string(),
            format!(
                "item {} is blocked; milestone 6's `Unblock` clears it (blueprint R-4)",
                ids::HTUI_FEAT_3
            )
        );
        assert_eq!(
            EngineError::ClaimRefused { run: ids::RUN_2 }.to_string(),
            "claim refused: the box is full or the scope overlaps (ANA-2 §4.7)"
        );
        assert_eq!(
            EngineError::SnapshotVersion {
                run: ids::RUN_2,
                v: 9,
            }
            .to_string(),
            format!(
                "run {} snapshot v9 is not readable by this engine",
                ids::RUN_2
            )
        );
        assert_eq!(
            EngineError::Snapshot {
                run: ids::RUN_2,
                reason: "no phase at position 9".to_owned(),
            }
            .to_string(),
            format!("run {} snapshot: no phase at position 9", ids::RUN_2),
            "ANA-2 invariant 7 wants a row a human can read, not a store refusal"
        );
        assert_eq!(
            EngineError::Stalled {
                run: ids::RUN_2,
                passes: 512,
            }
            .to_string(),
            format!(
                "run {}: the walk made 512 passes without resting; a row moved under it",
                ids::RUN_2
            )
        );
    }
}
