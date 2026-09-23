//! The gate: the three-valued settle outcome (plan D2), ANA-2 §4.2's gate table, the `review`
//! front-matter verdict parser (plan D10, ANA-5 §12 criterion 16) and `R-ORCH-3`'s review loop
//! with its no-progress predicate (plan D4, D5, D11).
//!
//! Stage 6 of the walk is the only place a gate is evaluated, and it is evaluated against a value
//! rather than against a driver: [`settle`] is a pure function of what the session produced, so the
//! whole of ANA-2 `:436-441` is table-testable without a store, a clock or an agent, and
//! [`apply`] is the only half that writes.

use chrono::{DateTime, TimeDelta, Utc};
use htui_agent::error::DriverError;
use htui_agent::event::{DoneEvent, StopReason};
use htui_agent::record::CapBreach;
use htui_core::model::{
    BoxId, Document, Gate, GateOutcome, GraphSnapshot, ItemId, NewNote, NoteId, Run, RunId,
    RunStatus, RunStep, SnapshotPhase, Status, StepId, StepStatus, UserId, VerifyOutcome,
};
use htui_core::prompt::digest::{canonical, sha256_hex};
use htui_core::store::{StoreError, WriteStore};

use crate::command::{EngineError, Rest, stale_run, stale_step};
use crate::isolate::Clock;
use crate::status::{RunFailure, latest_at, may_attempt, winner_at};

/// The phase name §4.4 step 1 looks for when it picks a loop target (`docs/ANA-2.md:713-715`).
///
/// Named rather than assumed adjacent, which is what makes the rule work for the seeded `bug`
/// graph, whose implement-shaped phase is called `fix` and is therefore *not* matched — it falls
/// through to "the immediately preceding position", which is the same answer.
const IMPLEMENT_PHASE: &str = "implement";

// ---------------------------------------------------------------------------------------------
// The verdict parser (plan D10, ANA-5 `:1322-1336`)
// ---------------------------------------------------------------------------------------------

/// What a `review` document's front matter says (ANA-5 `:1327-1331`, plan D10).
///
/// The grammar is exactly three lines at the very start of the body — `---`, `verdict: <value>`,
/// `---` — with no other keys.
///
/// **Only [`RequestChanges`](Verdict::RequestChanges) rejects.** ANA-5 `:1334-1336` says the
/// opposite in as many words ("a document whose first line is not `---` … is treated as
/// `request-changes` by MOD-4 with a note"), and plan **D10 overrides it**: treating silence as a
/// rejection burns `R-ORCH-3`'s retry budget on a parse bug, which is the expensive side of a coin
/// the document did not call. The unexpected value is not lost — it is recorded on the step's
/// `gate_note` by [`settle`], so an operator reading the Runs tab sees exactly what the agent
/// wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// `verdict: approve`.
    Approve,
    /// `verdict: request-changes`, the one value that rejects.
    RequestChanges,
    /// Three well-formed lines naming a value in neither vocabulary; carried verbatim (trimmed).
    Other(String),
    /// No front matter at all, or one that does not parse.
    Absent,
}

/// Line 2 of a well-formed three-line front matter, trimmed of its line ending, or `None`.
///
/// CRLF is folded here rather than by a caller: a review written on Windows is the same review,
/// and `document.body` is stored verbatim.
fn front_matter(body: &str) -> Option<&str> {
    let mut lines = body
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    // Exactly `---`, with no trailing whitespace: ANA-5 fixes the fence bytes, and a fence that is
    // "almost" right is a document whose author meant something this parser must not guess at.
    if lines.next()? != "---" {
        return None;
    }
    let verdict = lines.next()?;
    if lines.next()? != "---" {
        return None;
    }
    Some(verdict)
}

/// Parses a `review` document body's front matter (ANA-5 `:1327-1331`).
///
/// The `verdict:` key is matched case-insensitively — `VERDICT: Approve` is an approval — while the
/// *value* is trimmed and compared lowercased. [`Verdict::Other`] keeps the value as the agent
/// wrote it (trimmed only), because that string is what an operator reads back.
#[must_use]
pub fn parse_verdict(body: &str) -> Verdict {
    let Some(line) = front_matter(body) else {
        return Verdict::Absent;
    };
    let Some((key, value)) = line.split_once(':') else {
        return Verdict::Absent;
    };
    if !key.trim().eq_ignore_ascii_case("verdict") {
        return Verdict::Absent;
    }
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "approve" => Verdict::Approve,
        "request-changes" => Verdict::RequestChanges,
        _ => Verdict::Other(value.to_owned()),
    }
}

// ---------------------------------------------------------------------------------------------
// The settle outcome (plan D2, ANA-2 `:436-441`)
// ---------------------------------------------------------------------------------------------

/// Why a step settled `failed`, in the vocabulary of the step's own `gate_note`.
///
/// Distinct from [`RunFailure`] on purpose: `RunFailure` is `run.failure`'s vocabulary (plan D12)
/// and ANA-2 gives it no word for a refusal, a cap breach or an elapsed deadline, because those are
/// facts about *this step* and a later attempt may not repeat them. Only
/// [`MissingOutput`](StepFailure::MissingOutput) is both, which is why it is the one variant that
/// maps across ([`StepFailure::run_failure`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepFailure {
    /// The driver failed, or the stream closed before its `done` (`DriverError::Closed`).
    Driver(String),
    /// `done` with `refusal`, `max_tokens` or `max_turn_requests` (`docs/ANA-2.md:439`).
    Stopped(StopReason),
    /// `R-AGT-7`'s per-run cap was reached and the recorder cancelled the turn.
    CapBreached,
    /// The step outlived `SnapshotPhase::deadline_seconds` (plan D8's *step* deadline; the verify
    /// deadline of `:515` is milestone 3's and is a different clock).
    DeadlineElapsed,
    /// No document of the phase's `output_kind` was produced by this step (`:430`).
    MissingOutput,
    /// `run_step.verify_outcome = 'fail'`. Produced since milestone 3: `crate::verify` runs the
    /// phase's `verify_command` between stage 4 and stage 5 and the engine hands its outcome here.
    /// Milestone 2 matched it with no writer, which is why milestone 3 added one and no branch.
    VerifyFailed,
}

impl core::fmt::Display for StepFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Driver(message) => write!(f, "driver: {message}"),
            Self::Stopped(reason) => write!(f, "stop_reason: {reason}"),
            Self::CapBreached => f.write_str("cap breached"),
            Self::DeadlineElapsed => f.write_str("deadline elapsed"),
            Self::MissingOutput => f.write_str("missing_output"),
            Self::VerifyFailed => f.write_str("verify_outcome: fail"),
        }
    }
}

impl StepFailure {
    /// The `run.failure` vocabulary word for this failure, when D12 has one.
    #[must_use]
    pub fn run_failure(&self) -> Option<RunFailure> {
        match self {
            Self::MissingOutput => Some(RunFailure::MissingOutput),
            _ => None,
        }
    }

    /// What `run.failure` is written with when this failure ends the run.
    ///
    /// D12's word when there is one, this failure's own sentence otherwise: `finish_run` refuses a
    /// `failed` move with no text (blueprint A-3), so "no word for it" cannot mean "no text".
    #[must_use]
    pub fn run_failure_text(&self) -> String {
        self.run_failure()
            .map_or_else(|| self.to_string(), |failure| failure.to_string())
    }
}

/// ANA-2 §4.2's three-valued settle outcome (`docs/ANA-2.md:436-441`).
///
/// `ok` is **plan D2's** reading: the output document is present and `verify_outcome` is anything
/// but `fail`. §4.2's own table writes it as `verify_outcome IN ('pass','unavailable')`, which
/// excludes `NULL` — and `NULL` is the normal case, because every seeded phase has
/// `verify_command: None` (`crates/htui-core/src/seed.rs:233`). Read literally, §4.2 settles every
/// seeded phase `failed` and validation criterion 1 becomes unreachable; §4.3's step table states
/// the same guard as `verify_outcome != 'fail'` (`:637`) and is the form adopted here.
///
/// `stop_reason = cancelled` is deliberately **not** a settle outcome (`:441`): a cancelled step is
/// `cancelled`, which is a status and not a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle {
    /// The step produced its artefact and nothing failed.
    Ok {
        /// A `gate_note` the step earned without failing: the "unrecognised verdict" record of
        /// plan D10. `None` for every other `ok`.
        ///
        /// **Not in the blueprint's §5.4 sketch**, which returns a unit `Ok` and leaves D10's
        /// "recorded verbatim in `gate_note`" with nowhere to travel. One value out of `settle` is
        /// cheaper than a second return channel.
        note: Option<String>,
    },
    /// Any of `:439`'s six conditions, first hit winning.
    Failed(StepFailure),
    /// A `review` phase whose front matter said `request-changes` (`:440`).
    Rejected {
        /// The review's own verdict line, which ANA-2 `:748` makes the step's `gate_note`.
        verdict_line: String,
    },
}

/// Everything [`settle`] reads. A struct rather than eight arguments so the order of the checks is
/// stated once, in the function, and not re-stated at every call site.
#[derive(Debug)]
pub struct SettleInput<'a> {
    /// What stage 4's `pump` answered. A closed stream is an `Err`, not a `done`.
    pub driver: &'a Result<DoneEvent, DriverError>,
    /// `RecorderSummary::cap_breach` (`R-AGT-7`).
    pub cap_breach: Option<CapBreach>,
    /// When the step's own clock started: `run_step.started_at`, the `running` move, for a
    /// `fan_out = 1` step; the moment `prepare` answered for a fan-out candidate (plan D48), so a
    /// `shared_serialized` sibling is not charged for the per-repo lock wait.
    pub started_at: DateTime<Utc>,
    /// The clock's instant at stage 5.
    pub now: DateTime<Utc>,
    /// `SnapshotPhase::deadline_seconds`; `None` is "no step deadline".
    pub deadline_seconds: Option<u32>,
    /// The document of the phase's `output_kind` produced by **this step**, if there is one.
    pub output: Option<&'a Document>,
    /// `run_step.verify_outcome`, from `crate::verify` (milestone 3, plan D30).
    ///
    /// `None` when the phase named no `verify_command` — the seeded shape, and the normal case —
    /// or when the session did not finish, which is blueprint A-3: a crashed session is already
    /// `Failed` by this function's first rule and a verify on it would record nothing. `None` is
    /// **not** `unavailable`: the latter is a command that was asked for and could not run.
    pub verify_outcome: Option<VerifyOutcome>,
    /// Whether the phase's front matter is read at all: `SnapshotPhase::name == "review"`, which
    /// is what the walk passes (`crate::engine`'s stage 5) and not `output_kind`. The two agree on
    /// every seeded phase — `phase_row` writes `output_kind` equal to the name
    /// (`crates/htui-core/src/seed.rs:230`) — and a graph edited through `create_phase` can set
    /// them apart, so which one decides is worth stating.
    pub is_review: bool,
}

/// ANA-2 §4.2's settle, first hit winning (`docs/ANA-2.md:436-441`).
///
/// The order is the table's own and is load-bearing twice over: a driver error outranks a missing
/// document, so a crashed session is not reported as an agent that forgot to write; and the
/// deadline outranks the document, so a step that produced its artefact an hour late still fails.
#[must_use]
pub fn settle(input: &SettleInput<'_>) -> Settle {
    let done = match input.driver {
        Ok(done) => done,
        Err(error) => return Settle::Failed(StepFailure::Driver(error.to_string())),
    };
    match done.stop_reason {
        StopReason::Refusal | StopReason::MaxTokens | StopReason::MaxTurnRequests => {
            return Settle::Failed(StepFailure::Stopped(done.stop_reason));
        }
        // `end_turn` is the happy path; `cancelled` is a status, not a verdict (`:441`).
        StopReason::EndTurn | StopReason::Cancelled => {}
    }
    if input.cap_breach.is_some() {
        return Settle::Failed(StepFailure::CapBreached);
    }
    if deadline_elapsed(input) {
        return Settle::Failed(StepFailure::DeadlineElapsed);
    }
    let Some(output) = input.output else {
        return Settle::Failed(StepFailure::MissingOutput);
    };
    if input.verify_outcome == Some(VerifyOutcome::Fail) {
        return Settle::Failed(StepFailure::VerifyFailed);
    }
    if !input.is_review {
        return Settle::Ok { note: None };
    }
    match parse_verdict(&output.body) {
        Verdict::RequestChanges => Settle::Rejected {
            verdict_line: front_matter(&output.body)
                .unwrap_or("verdict: request-changes")
                .trim()
                .to_owned(),
        },
        Verdict::Approve | Verdict::Absent => Settle::Ok { note: None },
        Verdict::Other(value) => Settle::Ok {
            note: Some(format!("verdict: {value} (unrecognised)")),
        },
    }
}

/// `now > started_at + deadline_seconds`, with a deadline that does not fit a `TimeDelta` read as
/// "no deadline" rather than as an instant failure.
fn deadline_elapsed(input: &SettleInput<'_>) -> bool {
    let Some(seconds) = input.deadline_seconds else {
        return false;
    };
    let Some(window) = TimeDelta::try_seconds(i64::from(seconds)) else {
        return false;
    };
    input.now > input.started_at + window
}

// ---------------------------------------------------------------------------------------------
// The gate table and its writes (ANA-2 `:446-452`, blueprint §5.5)
// ---------------------------------------------------------------------------------------------

/// What the walk holds while it writes a gate answer or runs the loop.
///
/// A borrowed bundle rather than seven arguments repeated across [`apply`] and [`review_loop`]:
/// both need the same six things, and the engine builds it once per iteration.
#[derive(Debug)]
pub struct GateContext<'a, S: WriteStore, C: Clock + ?Sized> {
    /// The store every write goes through.
    pub store: &'a S,
    /// Plan D8's clock: every instant below comes from here and from nowhere else.
    pub clock: &'a C,
    /// The run row as it was read at the top of this iteration.
    pub run: &'a Run,
    /// The run's own `graph_snapshot`, decoded. Invariant 2: never the live graph.
    pub snapshot: &'a GraphSnapshot,
    /// `item_note.created_by` for the notes the escalation writes.
    pub user: UserId,
    /// `item_note.box_id`.
    pub box_id: BoxId,
}

impl<S: WriteStore, C: Clock + ?Sized> GateContext<'_, S, C> {
    /// The run's item, or `None` for a chat run (which the walk never produces).
    const fn item(&self) -> Option<ItemId> {
        self.run.item_id
    }

    /// One item compare-and-set, with a stale `from` treated as "already moved" (plan D17).
    ///
    /// Every pair passed to this is checked against `Status::can_move_to`
    /// (`crates/htui-core/src/model/item.rs:46-60`) at its call site, because an illegal pair is a
    /// `Constraint` and not an `Ok(false)` (blueprint H-1).
    async fn move_item(&self, from: Status, to: Status) -> Result<bool, EngineError> {
        let Some(item) = self.item() else {
            return Ok(false);
        };
        Ok(self.store.transition(item, from, to).await?)
    }
}

/// Where the gate left the walk.
///
/// **The blueprint's §5.5 signature returns a bare `Rest`**, which cannot express either of the
/// two outcomes that are not rests: a `never`/`on_failure` pass leaves the run `running` with more
/// positions to walk, and a `never` failure inside budget owes a *new attempt* that only stage 1
/// may create — the capability interlock has to run again, and `gate.rs` holds no selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    /// The step is `done`; the walk moves on.
    Advance,
    /// The review loop retired the chain (plan D5) and the walk re-derives from the rows, where
    /// the cursor creates the loop's next attempt. **Nothing is reconciled** (plan D135): the
    /// retired review's commits stay on its labelled branch and never reach the primary.
    Retired,
    /// The step is `failed` and the budget holds: stage 1 admits `(position, attempt)`.
    Retry {
        /// The step's own position.
        position: i32,
        /// The attempt to create.
        attempt: i32,
    },
    /// The walk stops here.
    Rest(Rest),
}

/// ANA-2 §4.2's gate table (`docs/ANA-2.md:446-452`), and the only half of the gate that writes.
///
/// Three rows of the table and their writes:
///
/// - **park** — `transition_step(Running -> AwaitingApproval)`, `transition_run(Running ->
///   AwaitingApproval)`, `transition(item, InProgress -> AwaitingApproval)`, in that order. Not one
///   transaction (blueprint F-K, H-10): the order is chosen so the forbidden interleaving — a run
///   waiting with no waiting step — cannot occur, and the composite writer is carried as R-5.
/// - **done + skipped** — `transition_step(Running -> Done)`. `gate_outcome = 'skipped'` is *not*
///   written: `answer_gate` is `awaiting_approval`-only
///   (`crates/htui-core/src/store/traits.rs:794-799`) and no writer sets the column on a `running`
///   step. That is blueprint **H-9**, carried with R-5; no milestone-2 criterion asserts `skipped`.
/// - **fail run** — `transition_step(Running -> Failed)` then `finish_run(run, Failed, …)`, which
///   mirrors the item in the same transaction (plan D7).
///
/// Every step and run compare-and-set honours `Ok(false)` (plan D125): the row was moved by
/// another writer, so the answer is [`EngineError::StaleWrite`] and nothing after the refused move
/// is written. The item moves keep plan D17's "already moved" reading.
///
/// # Errors
/// [`EngineError::StaleWrite`], and every [`EngineError`] the writes can raise.
pub async fn apply<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    step: &RunStep,
    phase: &SnapshotPhase,
    settle: Settle,
) -> Result<Landing, EngineError> {
    let now = ctx.clock.now();
    match (phase.gate_effective, settle) {
        // `always` parks on every outcome; `on_failure` and `never` park on none of the three
        // without first being told which one it is, so the two arms below split on the outcome.
        (Gate::Always, outcome) => {
            note_step(ctx, step, gate_note(&outcome).as_deref(), now).await?;
            park(ctx, step, phase, now).await
        }
        (Gate::OnFailure | Gate::Never, Settle::Ok { note }) => {
            note_step(ctx, step, note.as_deref(), now).await?;
            move_step(ctx, step.id, StepStatus::Running, StepStatus::Done, now).await?;
            Ok(Landing::Advance)
        }
        (Gate::OnFailure, outcome @ (Settle::Failed(_) | Settle::Rejected { .. })) => {
            note_step(ctx, step, gate_note(&outcome).as_deref(), now).await?;
            park(ctx, step, phase, now).await
        }
        (Gate::Never, Settle::Failed(failure)) => {
            note_step(ctx, step, Some(&failure.to_string()), now).await?;
            retry_or_fail(ctx, step, phase, &failure, now).await
        }
        (Gate::Never, Settle::Rejected { verdict_line }) => {
            // Both entry points converge on `review_loop` with identical rows, which is what plan
            // D4's "one routine" costs: the automatic path parks the step first, because
            // `answer_gate` is a compare-and-set on `awaiting_approval` and there is no other
            // writer that records `gate_outcome = 'rejected'` with the note.
            move_step(
                ctx,
                step.id,
                StepStatus::Running,
                StepStatus::AwaitingApproval,
                now,
            )
            .await?;
            reject_step(ctx.store, ctx.run.id, step.id, verdict_line.clone(), now).await?;
            let step = reread(ctx, step).await?;
            match review_loop(ctx, &step).await? {
                LoopOutcome::Resumed { .. } => Ok(Landing::Retired),
                LoopOutcome::Escalated { attempts, .. } => Ok(Landing::Rest(Rest {
                    run: RunStatus::AwaitingApproval,
                    position: Some(step.position),
                    failure: Some(RunFailure::ReviewLoopExhausted(attempts)),
                })),
                LoopOutcome::NoTarget => Ok(Landing::Rest(Rest {
                    run: RunStatus::Failed,
                    position: Some(step.position),
                    failure: Some(RunFailure::NoLoopTarget),
                })),
            }
        }
    }
}

/// The `gate_note` a settled outcome earns, if any.
fn gate_note(settle: &Settle) -> Option<String> {
    match settle {
        Settle::Ok { note } => note.clone(),
        Settle::Failed(failure) => Some(failure.to_string()),
        Settle::Rejected { verdict_line } => Some(verdict_line.clone()),
    }
}

/// Records why a step settled the way it did, on the **item** rather than on the step.
///
/// **Blueprint H-9, widened.** The blueprint puts these sentences on `run_step.gate_note`, and
/// there is no writer that can: `answer_gate` is a compare-and-set on `awaiting_approval`
/// (`crates/htui-core/src/store/traits.rs:794-799`) and `select_fanout` is fan-out's, so nothing
/// writes `gate_note` on a `running` step and nothing writes it at all without also *answering*
/// the gate the walk is about to park at. Dropping the sentence would leave a parked step with no
/// readable reason, which ANA-2 invariant 7 forbids (`docs/ANA-2.md:132-135`), so it goes to
/// `add_note` — a shipped writer, on the row a human is already reading. The `gate_note` column
/// stays NULL until a human answers, and the composite park writer is carried with **R-5**.
///
/// A settle with nothing to say writes nothing: the happy path adds no rows.
async fn note_step<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    step: &RunStep,
    note: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), EngineError> {
    let (Some(note), Some(item)) = (note, ctx.item()) else {
        return Ok(());
    };
    ctx.store
        .add_note(NewNote {
            id: NoteId::new(),
            item_id: item,
            body: format!(
                "step `{}` attempt {}: {note}",
                step.phase_name, step.attempt
            ),
            created_by: ctx.user,
            box_id: Some(ctx.box_id),
            via_step_id: Some(step.id),
            created_at: now,
        })
        .await?;
    Ok(())
}

/// The three compare-and-sets of a gate park, in step → run → item order (blueprint H-10).
async fn park<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    step: &RunStep,
    phase: &SnapshotPhase,
    now: DateTime<Utc>,
) -> Result<Landing, EngineError> {
    move_step(
        ctx,
        step.id,
        StepStatus::Running,
        StepStatus::AwaitingApproval,
        now,
    )
    .await?;
    move_run(ctx, RunStatus::Running, RunStatus::AwaitingApproval, now).await?;
    ctx.move_item(Status::InProgress, Status::AwaitingApproval)
        .await?;
    Ok(Landing::Rest(Rest {
        run: RunStatus::AwaitingApproval,
        position: Some(phase.position),
        failure: None,
    }))
}

/// §4.2's `never` + `failed` cell: retry while the budget holds, else end the run.
///
/// The step is failed either way; what differs is whether a successor is created. The successor is
/// **not** created here — the walk's cursor creates it at `next_attempt`, which is the one place
/// that reads `max(attempt) + 1` from rows and therefore the one place that cannot trip
/// `UNIQUE (run_id, position, attempt, fanout_index)` (blueprint H-4).
async fn retry_or_fail<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    step: &RunStep,
    phase: &SnapshotPhase,
    failure: &StepFailure,
    now: DateTime<Utc>,
) -> Result<Landing, EngineError> {
    move_step(ctx, step.id, StepStatus::Running, StepStatus::Failed, now).await?;
    // The successor is named rather than created: `cursor` reads a `failed` latest attempt as a
    // rest (`crate::status::cursor`), so the walk would stop here — and only stage 1 may create a
    // step, because the capability interlock has to run for every attempt (plan D6).
    if may_attempt(step.attempt + 1, phase.retry_limit) {
        return Ok(Landing::Retry {
            position: phase.position,
            attempt: step.attempt + 1,
        });
    }
    ctx.store
        .finish_run(
            ctx.run.id,
            RunStatus::Failed,
            Some(&failure.run_failure_text()),
            now,
        )
        .await?;
    Ok(Landing::Rest(Rest {
        run: RunStatus::Failed,
        position: Some(phase.position),
        failure: failure.run_failure(),
    }))
}

/// One step compare-and-set on the walk's path. `Ok(false)` is plan D125's
/// [`EngineError::StaleWrite`]: another writer moved the row first, and the walk writes nothing
/// further.
async fn move_step<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    step: StepId,
    from: StepStatus,
    to: StepStatus,
    now: DateTime<Utc>,
) -> Result<(), EngineError> {
    if ctx.store.transition_step(step, from, to, now).await? {
        Ok(())
    } else {
        Err(stale_step(ctx.run.id, step, from, to))
    }
}

/// An automatic rejection's `answer_gate(Rejected, note)` on a step the walk has just parked:
/// `awaiting_approval -> failed`, with `note` as its `gate_note`. The gate's review rejection and
/// the engine's judge failure (plan D51) both write it.
///
/// Plan D144: `answer_gate` is a compare-and-set on `awaiting_approval`, so `Ok(false)` is plan
/// D125's [`EngineError::StaleWrite`]. Another writer answered or moved the step between the park
/// and this write, and the walk writes nothing further.
pub(crate) async fn reject_step<S: WriteStore + ?Sized>(
    store: &S,
    run: RunId,
    step: StepId,
    note: String,
    now: DateTime<Utc>,
) -> Result<(), EngineError> {
    if store
        .answer_gate(step, GateOutcome::Rejected, Some(note), now)
        .await?
    {
        Ok(())
    } else {
        Err(stale_step(
            run,
            step,
            StepStatus::AwaitingApproval,
            StepStatus::Failed,
        ))
    }
}

/// [`move_step`] for the run row.
async fn move_run<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    from: RunStatus,
    to: RunStatus,
    now: DateTime<Utc>,
) -> Result<(), EngineError> {
    if ctx.store.transition_run(ctx.run.id, from, to, now).await? {
        Ok(())
    } else {
        Err(stale_run(ctx.run.id, from, to))
    }
}

/// Re-reads one step row after a write moved it (plan D16: nothing is remembered across a write).
async fn reread<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    step: &RunStep,
) -> Result<RunStep, EngineError> {
    let steps = ctx.store.run_steps(ctx.run.id).await?;
    steps
        .into_iter()
        .find(|row| row.id == step.id)
        .ok_or(EngineError::Store(StoreError::NotFound {
            entity: "run_step",
            id: step.id.to_string(),
        }))
}

// ---------------------------------------------------------------------------------------------
// The review loop (plan D4, D5, D11; ANA-2 §4.4 `:712-753`)
// ---------------------------------------------------------------------------------------------

/// Why the review loop stopped instead of running another iteration.
///
/// **Named `LoopStop` and not `StopReason`**, which is what the blueprint's §5.6 calls it: this
/// module already matches on `htui_agent::event::StopReason` inside [`settle`], and two types with
/// one name in one file is how a later milestone imports the wrong one — the same reason plan D15
/// refused a second `ResolvedPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStop {
    /// The implement phase's `retry_limit` is spent (`docs/ANA-2.md:716`).
    Exhausted,
    /// Two consecutive implement attempts produced an identical `after_hash` per repo, "both
    /// `NULL`" included (`:735-739`).
    NoProgressHash,
    /// Two consecutive review documents are byte-identical after canonicalisation (`:735-739`).
    NoProgressReview,
}

impl core::fmt::Display for LoopStop {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Exhausted => "exhausted",
            Self::NoProgressHash => "no_progress_hash",
            Self::NoProgressReview => "no_progress_review",
        })
    }
}

/// What one turn of the review loop decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopOutcome {
    /// The chain was retired; the walk resumes at `(position, attempt)`, which the cursor creates.
    Resumed {
        /// `p_impl`.
        position: i32,
        /// The attempt the cursor will create there.
        attempt: i32,
    },
    /// Parked for a human: run `awaiting_approval`, item `blocked`, note written.
    Escalated {
        /// `N` of `review loop exhausted after N attempts`: the implement attempt just judged.
        attempts: i32,
        /// Which predicate stopped it.
        reason: LoopStop,
    },
    /// §4.4 step 1 found no position to loop back to; the run failed with `no_loop_target`.
    NoTarget,
}

/// `R-ORCH-3`'s loop, one routine for both of plan D4's entry points (`docs/ANA-2.md:712-753`).
///
/// The caller has already left `review` at `failed` with `gate_outcome = 'rejected'` and its
/// `gate_note`, and has already moved the run to `running` and the item to `in_progress` if they
/// were parked (blueprint A-5) — so the human path and the automatic path arrive here with
/// identical rows and this function never asks which one it was.
///
/// **Retiring is status by status** (plan D5, fact-check F14b/F15): `pending`,
/// `awaiting_approval` and `done` are superseded; a `running` step is **cancelled**, because
/// `running -> superseded` is illegal and earns `StoreError::Constraint`
/// (`crates/htui-core/src/model/run.rs:114-117`); a `failed` step — the rejecting review itself —
/// is cancelled too, since `failed -> superseded` is illegal as well, and `transition_step` moves
/// only `status`, so its `gate_outcome` and `gate_note` survive (`retire_slot` explains why
/// plan D5's "left alone" cannot work); `superseded` and `cancelled` are already retired.
///
/// The successor step is **not** created here, unlike the blueprint's §5.6 step 6: retiring
/// `p_impl` makes the cursor answer `Create { p_impl, next_attempt }` on its own
/// (`crate::status::cursor`), so the new attempt goes through stage 1's capability interlock and
/// the selector like any other, and `next_attempt` stays the single reader of
/// `UNIQUE (run_id, position, attempt, fanout_index)`.
///
/// # Errors
/// Every [`EngineError`] the writes can raise, and [`EngineError::StaleWrite`] when a retired row
/// or, on escalation, the run was moved by another writer first (plan D125).
pub async fn review_loop<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    review: &RunStep,
) -> Result<LoopOutcome, EngineError> {
    let now = ctx.clock.now();
    let Some(target) = loop_target(ctx.snapshot, review.position) else {
        ctx.store
            .finish_run(
                ctx.run.id,
                RunStatus::Failed,
                Some(&RunFailure::NoLoopTarget.to_string()),
                now,
            )
            .await?;
        return Ok(LoopOutcome::NoTarget);
    };
    // By `position`, never by index. [`loop_target`] promises a phase whose `position` is `target`
    // and says nothing about where in the array it sits, and density is manufactured by
    // `graph::resolve`'s renumbering alone: `ck_run_graph_snapshot` checks non-null, `NewRun` takes
    // the blob as given, and milestone 5's sweep adopts runs this process did not mint. Read
    // positionally, a snapshot with a hole in it indexes past the array — a panic, not a refusal —
    // and an unsorted one silently reads a different phase's `retry_limit` and name. This is
    // `Engine::phase_at`'s lookup, written out because that one is `engine.rs`'s private associated
    // function and reaching it from here would mean naming the whole generic `Engine`.
    let Some(impl_phase) = ctx
        .snapshot
        .phases
        .iter()
        .find(|phase| phase.position == target)
    else {
        ctx.store
            .finish_run(
                ctx.run.id,
                RunStatus::Failed,
                Some(&RunFailure::NoLoopTarget.to_string()),
                now,
            )
            .await?;
        return Ok(LoopOutcome::NoTarget);
    };

    let steps = ctx.store.run_steps(ctx.run.id).await?;
    let Some(attempt) = latest_at(&steps, target).map(|step| step.attempt) else {
        ctx.store
            .finish_run(
                ctx.run.id,
                RunStatus::Failed,
                Some(&RunFailure::NoLoopTarget.to_string()),
                now,
            )
            .await?;
        return Ok(LoopOutcome::NoTarget);
    };

    if let Some(reason) = no_progress(ctx, &steps, target, review.position, attempt).await? {
        return escalate(ctx, review, impl_phase, attempt, reason, now).await;
    }
    if !may_attempt(attempt + 1, impl_phase.retry_limit) {
        return escalate(ctx, review, impl_phase, attempt, LoopStop::Exhausted, now).await;
    }

    retire(ctx, &steps, target, review.position, now).await?;
    Ok(LoopOutcome::Resumed {
        position: target,
        attempt: attempt + 1,
    })
}

/// §4.4 step 1: the greatest position below `review` named `implement`, else the one before it.
///
/// `None` when `review` is at position 0, or when the snapshot has no such position — plan D5's
/// terminal review, which fails the run with `no_loop_target`.
#[must_use]
pub fn loop_target(snapshot: &GraphSnapshot, review_position: i32) -> Option<i32> {
    let named = snapshot
        .phases
        .iter()
        .filter(|phase| phase.position < review_position && phase.name == IMPLEMENT_PHASE)
        .map(|phase| phase.position)
        .max();
    named.or_else(|| {
        let previous = review_position - 1;
        (previous >= 0
            && snapshot
                .phases
                .iter()
                .any(|phase| phase.position == previous))
        .then_some(previous)
    })
}

/// Plan D11's predicate, evaluated only when there is a previous attempt to compare against.
///
/// Both halves are computed from rows the step already wrote. The hash half compares the two
/// implement attempts' `run_step_commit` rows per repo, with "both `None`" counting as identical —
/// two attempts that committed nothing are exactly the failure mode ANA-2 `:735-739` names. The
/// review half hashes the two latest review documents of this run through
/// `prompt::digest::canonical`, the workspace's one normalisation, so a review re-emitted with
/// different line endings does not read as progress.
async fn no_progress<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    steps: &[RunStep],
    target: i32,
    review_position: i32,
    attempt: i32,
) -> Result<Option<LoopStop>, EngineError> {
    if attempt < 2 {
        return Ok(None);
    }
    if commits_are_identical(ctx, steps, target, attempt).await? {
        return Ok(Some(LoopStop::NoProgressHash));
    }
    if reviews_are_identical(ctx, steps, review_position).await? {
        return Ok(Some(LoopStop::NoProgressReview));
    }
    Ok(None)
}

/// The `after_hash` of every repo in scope, identical across the winners of attempts `a` and
/// `a - 1`.
///
/// An empty `repo_scope` — the demo fixture's own shape, since it holds no repos — cannot decide
/// anything, so it answers `false` and leaves the verdict to the review-body half.
///
/// Each attempt is read through [`winner_at`] (plan D66): the `selected` candidate of a fanned-out
/// slot, else its `fanout_index 0`, which is the one row a `fan_out = 1` slot holds. Reading index
/// 0 regardless would compare a loser whenever the judge picked another index.
///
/// **R-8 is closed with no predicate change** (plan D66). The winner's `after_hash` is its
/// post-reconcile hash, and two consecutive attempts can only agree on it when both are `NULL`:
/// attempt `a + 1` branches from a primary `HEAD` that already contains attempt `a`'s merge, so any
/// commit it makes is a descendant of both of attempt `a`'s hashes, pre- and post-merge, and
/// differs from each. Keeping the pre-merge hash as well would therefore change no answer, and
/// reconcile keeps a `NULL` `after_hash` `NULL`.
async fn commits_are_identical<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    steps: &[RunStep],
    target: i32,
    attempt: i32,
) -> Result<bool, EngineError> {
    if ctx.run.repo_scope.is_empty() {
        return Ok(false);
    }
    let Some(this) = winner_at(steps, target, attempt).map(|step| step.id) else {
        return Ok(false);
    };
    let Some(previous) = winner_at(steps, target, attempt - 1).map(|step| step.id) else {
        return Ok(false);
    };
    let this = ctx.store.step_commits(this).await?;
    let previous = ctx.store.step_commits(previous).await?;
    Ok(ctx.run.repo_scope.iter().all(|repo| {
        let hash = |rows: &[htui_core::model::RunStepCommit]| {
            rows.iter()
                .find(|row| row.repo_id == *repo)
                .map(|row| row.after_hash.clone())
        };
        // The outer `Option` is "this attempt has a row for the repo"; the inner is `after_hash`.
        // Both-absent and both-`None` are the same answer — neither attempt committed anything.
        hash(&this).flatten() == hash(&previous).flatten()
    }))
}

/// The two latest review documents of this run, byte-identical after canonicalisation.
async fn reviews_are_identical<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    steps: &[RunStep],
    review_position: i32,
) -> Result<bool, EngineError> {
    let Some(item) = ctx.item() else {
        return Ok(false);
    };
    let Some(kind) = ctx
        .snapshot
        .phases
        .iter()
        .find(|phase| phase.position == review_position)
        .map(|phase| phase.output_kind.clone())
    else {
        return Ok(false);
    };
    let mine: Vec<_> = steps
        .iter()
        .filter(|step| step.position == review_position)
        .map(|step| step.id)
        .collect();
    let mut documents: Vec<Document> = ctx
        .store
        .documents_of_kinds(item, &[kind])
        .await?
        .into_iter()
        .filter(|document| {
            document
                .produced_by_step_id
                .is_some_and(|step| mine.contains(&step))
        })
        .collect();
    documents.sort_by_key(|document| core::cmp::Reverse(document.version));
    let [newest, previous, ..] = documents.as_slice() else {
        return Ok(false);
    };
    Ok(sha256_hex(&canonical(&newest.body)) == sha256_hex(&canonical(&previous.body)))
}

/// §4.4 step 3, widened to the whole chain by plan D5 and split by status by fact-check F14b: every
/// position of `from..=to` is retired through [`retire_slot`].
async fn retire<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    steps: &[RunStep],
    from: i32,
    to: i32,
    now: DateTime<Utc>,
) -> Result<(), EngineError> {
    for position in from..=to {
        retire_slot(ctx, steps, position, now).await?;
    }
    Ok(())
}

/// Retires the latest slot at `position` (plan D66, blueprint F-C): every row at
/// `(position, max attempt over every row at position)`, candidates and judge alike.
///
/// A `fan_out = 1` slot is its one `fanout_index 0` row, so this is exactly the retirement the
/// loop has done since milestone 2. A fanned-out slot is retired whole — ANA-2 `:754-758`, "the
/// previous attempt's winner and losers are all `superseded`" — and its judge with it, so the
/// next attempt judges again. The attempt is the maximum over *all* rows rather than
/// [`latest_at`]'s index-0 reading, so a slot is never split across two attempts.
///
/// By M2 D5's status split: `pending`, `awaiting_approval` and `done` are superseded; `running`
/// and `failed` are cancelled; `superseded` and `cancelled` are already retired and left alone.
/// A position with no rows is skipped.
///
/// # Errors
/// Every [`EngineError`] `supersede_step` and `transition_step` can raise, and
/// [`EngineError::StaleWrite`] when a `running` or `failed` row was moved first (plan D125).
pub(crate) async fn retire_slot<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    steps: &[RunStep],
    position: i32,
    now: DateTime<Utc>,
) -> Result<(), EngineError> {
    let Some(attempt) = steps
        .iter()
        .filter(|step| step.position == position)
        .map(|step| step.attempt)
        .max()
    else {
        return Ok(());
    };
    let slot = steps
        .iter()
        .filter(|step| step.position == position && step.attempt == attempt);
    for step in slot {
        match step.status {
            StepStatus::Pending | StepStatus::AwaitingApproval | StepStatus::Done => {
                ctx.store.supersede_step(step.id).await?;
            }
            // `running -> superseded` and `failed -> superseded` are both illegal
            // (`crates/htui-core/src/model/run.rs:111-130`), and `cancelled` is the one legal
            // retirement either of them has. `transition_step` moves `status` and nothing else, so
            // the rejecting review keeps its `gate_outcome = 'rejected'` and its `gate_note` —
            // which is the half of ANA-2 §4.4 step 3 that matters (`:719`). A failed judge keeps
            // its `gate_note` the same way.
            //
            // **Plan D5 says a `failed` step is "left alone", and that reading cannot work**:
            // `cursor` reads a `failed` latest attempt as a rest, so the rejecting review at
            // `p_review` would stop the walk forever and the blueprint's own §7 rejection branch —
            // which creates `(3,2)` after `(2,2)` is approved — would be unreachable. D5's reason
            // for the exclusion was that `superseded` is illegal from `failed`, and it is; the
            // conclusion it drew from that is the part this corrects.
            StepStatus::Running | StepStatus::Failed => {
                move_step(ctx, step.id, step.status, StepStatus::Cancelled, now).await?;
            }
            StepStatus::Superseded | StepStatus::Cancelled => {}
        }
    }
    Ok(())
}

/// ANA-2 §4.4's escalation rows (`:741-753`), as blueprint A-4 amends them.
///
/// `run.failure` stays NULL: no shipped writer sets `failure` on a non-terminal run, and
/// `finish_run` is terminal-only by plan D7. The exact wording therefore lives in the `item_note`
/// — which is what criterion 6 asserts (`docs/ANA-2.md:2101-2102`) — and in the
/// [`RunFailure::ReviewLoopExhausted`] the caller receives. Carried as **R-3** for milestone 5.
async fn escalate<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    review: &RunStep,
    impl_phase: &SnapshotPhase,
    attempts: i32,
    reason: LoopStop,
    now: DateTime<Utc>,
) -> Result<LoopOutcome, EngineError> {
    // Plan D125: a run another writer moved first stops the escalation, with no item move and no
    // note after the refused move.
    move_run(ctx, RunStatus::Running, RunStatus::AwaitingApproval, now).await?;
    ctx.move_item(Status::InProgress, Status::Blocked).await?;
    if let Some(item) = ctx.item() {
        ctx.store
            .add_note(NewNote {
                id: NoteId::new(),
                item_id: item,
                body: format!(
                    "{}: phase `{}`, attempt {attempts}, stop reason `{reason}`",
                    RunFailure::ReviewLoopExhausted(attempts),
                    impl_phase.name
                ),
                created_by: ctx.user,
                box_id: Some(ctx.box_id),
                via_step_id: Some(review.id),
                created_at: now,
            })
            .await?;
    }
    Ok(LoopOutcome::Escalated { attempts, reason })
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use htui_agent::conformance::epoch;
    use htui_agent::error::DriverError;
    use htui_agent::event::{DoneEvent, StopReason};
    use htui_agent::record::CapBreach;
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{Document, GraphSnapshot, VerifyOutcome};

    use htui_core::model::{NewRepo, NewRunStep, RepoId, Run, RunStep, RunStepCommit, StepStatus};
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

    use super::{
        GateContext, LoopStop, Settle, SettleInput, StepFailure, Verdict, loop_target, no_progress,
        parse_verdict, retire_slot, settle,
    };
    use crate::isolate::SystemClock;

    /// A `review` document body, since `settle` reads the front matter off one.
    fn document(body: &str) -> Document {
        let mut document = demo_data()
            .documents
            .into_iter()
            .next()
            .expect("the fixture seeds documents");
        document.body = body.to_owned();
        document
    }

    /// The `feature` snapshot every fixture graph run carries.
    fn snapshot() -> GraphSnapshot {
        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
            .expect("the fixture snapshot is a `GraphSnapshot`")
    }

    /// A settle input that settles `ok`, which every case below then spoils in exactly one way.
    fn ok_input<'a>(
        driver: &'a Result<DoneEvent, DriverError>,
        output: Option<&'a Document>,
    ) -> SettleInput<'a> {
        SettleInput {
            driver,
            cap_breach: None,
            started_at: epoch(),
            now: epoch(),
            deadline_seconds: Some(7200),
            output,
            verify_outcome: None,
            is_review: false,
        }
    }

    fn done(stop: StopReason) -> Result<DoneEvent, DriverError> {
        Ok(DoneEvent { stop_reason: stop })
    }

    /// ANA-5 `:1327-1331`'s grammar, arm by arm, plus the two shapes that are *almost* it.
    #[test]
    fn the_verdict_parser_reads_ana5s_three_lines() {
        assert_eq!(
            parse_verdict("---\nverdict: approve\n---\nlooks good"),
            Verdict::Approve
        );
        assert_eq!(
            parse_verdict("---\nverdict: request-changes\n---\nno tests"),
            Verdict::RequestChanges
        );
        assert_eq!(
            parse_verdict("---\r\nverdict: approve\r\n---\r\nbody"),
            Verdict::Approve,
            "a review written on Windows is the same review"
        );
        assert_eq!(
            parse_verdict("---\nVERDICT: Approve\n---\n"),
            Verdict::Approve,
            "the key is matched case-insensitively and the value lowercased"
        );
        assert_eq!(
            parse_verdict("---\nverdict:   request-changes  \n---\n"),
            Verdict::RequestChanges,
            "the value is trimmed"
        );
    }

    /// Plan D10: only the exact value rejects, and the two "nearly" shapes are `Absent`, not
    /// rejections. ANA-5 `:1334-1336` says the opposite and D10 overrides it; this is that test.
    #[test]
    fn only_an_exact_request_changes_is_a_rejection() {
        for body in [
            "",
            "no front matter at all",
            "--- \nverdict: request-changes\n---\n", // trailing space on the fence
            "---\nverdict: request-changes\n--\n",   // a two-dash closing fence
            "---\nsummary: request-changes\n---\n",  // the wrong key
            "---\nrequest-changes\n---\n",           // no key at all
            "---\nverdict: approve\n",               // no closing fence
        ] {
            assert_eq!(
                parse_verdict(body),
                Verdict::Absent,
                "`{body:?}` is not a rejection (plan D10)"
            );
        }
        assert_eq!(
            parse_verdict("---\nverdict: Block\n---\n"),
            Verdict::Other("Block".to_owned()),
            "an unrecognised value is carried verbatim for the `gate_note`"
        );
    }

    /// ANA-2 `:436-441`'s table, one assertion per row, on a driver result that would otherwise
    /// settle `ok`.
    #[test]
    fn settle_reads_ana2s_three_values() {
        let output = document("body");

        let ended = done(StopReason::EndTurn);
        assert_eq!(
            settle(&ok_input(&ended, Some(&output))),
            Settle::Ok { note: None }
        );

        for stop in [
            StopReason::Refusal,
            StopReason::MaxTokens,
            StopReason::MaxTurnRequests,
        ] {
            let result = done(stop);
            assert_eq!(
                settle(&ok_input(&result, Some(&output))),
                Settle::Failed(StepFailure::Stopped(stop)),
                "`{stop}` settles failed"
            );
        }

        let cancelled = done(StopReason::Cancelled);
        assert_eq!(
            settle(&ok_input(&cancelled, Some(&output))),
            Settle::Ok { note: None },
            "`cancelled` is a status, not a settle outcome (`docs/ANA-2.md:441`)"
        );

        let closed: Result<DoneEvent, DriverError> = Err(DriverError::Closed);
        assert!(matches!(
            settle(&ok_input(&closed, Some(&output))),
            Settle::Failed(StepFailure::Driver(_))
        ));

        assert_eq!(
            settle(&ok_input(&ended, None)),
            Settle::Failed(StepFailure::MissingOutput),
            "`docs/ANA-2.md:430`"
        );
    }

    /// Plan D2's whole content: `NULL` is the normal case and must settle `ok`, because every
    /// seeded phase has `verify_command: None` and §4.2's literal `IN ('pass','unavailable')`
    /// would make validation criterion 1 unreachable.
    #[test]
    fn a_null_verify_outcome_settles_ok() {
        let output = document("body");
        let ended = done(StopReason::EndTurn);

        for outcome in [
            None,
            Some(VerifyOutcome::Pass),
            Some(VerifyOutcome::Unavailable),
        ] {
            let input = SettleInput {
                verify_outcome: outcome,
                ..ok_input(&ended, Some(&output))
            };
            assert_eq!(
                settle(&input),
                Settle::Ok { note: None },
                "{outcome:?} is not a failure (plan D2)"
            );
        }

        let failed = SettleInput {
            verify_outcome: Some(VerifyOutcome::Fail),
            ..ok_input(&ended, Some(&output))
        };
        assert_eq!(settle(&failed), Settle::Failed(StepFailure::VerifyFailed));
    }

    /// Plan D8's step deadline, evaluated against the clock and never against a sleep.
    #[test]
    fn a_step_that_outlives_its_deadline_settles_failed() {
        let output = document("body");
        let ended = done(StopReason::EndTurn);

        let late = SettleInput {
            deadline_seconds: Some(1),
            now: epoch() + TimeDelta::seconds(2),
            ..ok_input(&ended, Some(&output))
        };
        assert_eq!(
            settle(&late),
            Settle::Failed(StepFailure::DeadlineElapsed),
            "`docs/ANA-2.md:439`"
        );

        let exact = SettleInput {
            deadline_seconds: Some(1),
            now: epoch() + TimeDelta::seconds(1),
            ..ok_input(&ended, Some(&output))
        };
        assert_eq!(
            settle(&exact),
            Settle::Ok { note: None },
            "the deadline is exclusive: elapsed means past, not at"
        );

        let none = SettleInput {
            deadline_seconds: None,
            now: epoch() + TimeDelta::days(400),
            ..ok_input(&ended, Some(&output))
        };
        assert_eq!(settle(&none), Settle::Ok { note: None });
    }

    /// `R-AGT-7`'s cap outranks the document but not the driver: a session the recorder cancelled
    /// wrote no artefact either, and reporting it as `missing_output` would hide the reason.
    #[test]
    fn a_cap_breach_settles_failed_before_the_document_is_looked_at() {
        let ended = done(StopReason::EndTurn);
        let breached = SettleInput {
            cap_breach: Some(CapBreach {
                cap_micros: 1_000,
                spent_micros: 2_000,
                at: epoch(),
            }),
            ..ok_input(&ended, None)
        };
        assert_eq!(settle(&breached), Settle::Failed(StepFailure::CapBreached));
    }

    /// The front matter is read only for a `review` phase, and only `request-changes` rejects.
    #[test]
    fn a_review_verdict_decides_rejected_and_nothing_else_does() {
        let ended = done(StopReason::EndTurn);

        let rejecting = document("---\nverdict: request-changes\n---\nno tests");
        let input = SettleInput {
            is_review: true,
            ..ok_input(&ended, Some(&rejecting))
        };
        assert_eq!(
            settle(&input),
            Settle::Rejected {
                verdict_line: "verdict: request-changes".to_owned()
            }
        );

        let same_body_not_a_review = ok_input(&ended, Some(&rejecting));
        assert_eq!(
            settle(&same_body_not_a_review),
            Settle::Ok { note: None },
            "a non-review phase never reads front matter"
        );

        let odd = document("---\nverdict: Block\n---\nhmm");
        let input = SettleInput {
            is_review: true,
            ..ok_input(&ended, Some(&odd))
        };
        assert_eq!(
            settle(&input),
            Settle::Ok {
                note: Some("verdict: Block (unrecognised)".to_owned())
            },
            "plan D10: recorded, not rejected"
        );

        let silent = document("no front matter");
        let input = SettleInput {
            is_review: true,
            ..ok_input(&ended, Some(&silent))
        };
        assert_eq!(
            settle(&input),
            Settle::Ok { note: None },
            "silence is not a rejection (plan D10)"
        );
    }

    /// The step's `gate_note` vocabulary, which the blueprint fixes verbatim (§5.4).
    #[test]
    fn step_failure_display_is_the_gate_notes_vocabulary() {
        assert_eq!(
            StepFailure::Stopped(StopReason::Refusal).to_string(),
            "stop_reason: refusal"
        );
        assert_eq!(StepFailure::CapBreached.to_string(), "cap breached");
        assert_eq!(StepFailure::DeadlineElapsed.to_string(), "deadline elapsed");
        assert_eq!(StepFailure::MissingOutput.to_string(), "missing_output");
        assert_eq!(
            StepFailure::Driver("boom".to_owned()).to_string(),
            "driver: boom"
        );

        // Only `missing_output` has a `run.failure` word (plan D12); the rest fall back to their
        // own sentence, because `finish_run` refuses a `failed` move with no text (A-3).
        assert_eq!(
            StepFailure::MissingOutput.run_failure_text(),
            "missing_output"
        );
        assert!(StepFailure::CapBreached.run_failure().is_none());
        assert_eq!(StepFailure::CapBreached.run_failure_text(), "cap breached");
    }

    /// §4.4 step 1 on the seeded `feature` graph, and on the two shapes plan D5 calls terminal.
    #[test]
    fn the_loop_target_is_the_named_implement_phase() {
        let snapshot = snapshot();
        assert_eq!(
            loop_target(&snapshot, 3),
            Some(2),
            "`implement` is at position 2 of the seeded `feature` graph"
        );
        assert_eq!(
            loop_target(&snapshot, 2),
            Some(1),
            "no `implement` below position 2, so the immediately preceding position"
        );
        assert_eq!(
            loop_target(&snapshot, 0),
            None,
            "a review at position 0 is terminal (plan D5)"
        );
    }

    /// The stop reasons are read out of an `item_note` by a human and asserted by criterion 7.
    #[test]
    fn loop_stop_display_is_what_the_note_carries() {
        assert_eq!(LoopStop::Exhausted.to_string(), "exhausted");
        assert_eq!(LoopStop::NoProgressHash.to_string(), "no_progress_hash");
        assert_eq!(LoopStop::NoProgressReview.to_string(), "no_progress_review");
    }

    /// `RUN_3` as the demo fixture holds it, and its decoded snapshot.
    fn run_3() -> (Run, GraphSnapshot) {
        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_3)
            .expect("the fixture holds RUN_3");
        let snapshot = serde_json::from_value(run.graph_snapshot.clone().expect("a graph run"))
            .expect("the fixture snapshot is a `GraphSnapshot`");
        (run, snapshot)
    }

    /// Inserts a `research` row of `RUN_3` at `(0, attempt, fanout_index)` and walks it through
    /// `path` with legal compare-and-sets, starting from `pending`.
    async fn step(
        store: &MemStore,
        attempt: i32,
        fanout_index: i32,
        path: &[StepStatus],
    ) -> RunStep {
        let row = store
            .create_step(NewRunStep {
                id: htui_core::model::StepId::new(),
                run_id: ids::RUN_3,
                position: 0,
                attempt,
                fanout_index,
                phase_name: "research".to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: Some("opus".to_owned()),
            })
            .await
            .expect("the slot has room for this row");
        let mut from = StepStatus::Pending;
        for &to in path {
            assert!(
                store
                    .transition_step(row.id, from, to, epoch())
                    .await
                    .expect("a legal move"),
                "`{from} -> {to}` applied"
            );
            from = to;
        }
        row
    }

    /// The status of every `RUN_3` row, by id.
    async fn status_of(store: &MemStore, id: htui_core::model::StepId) -> StepStatus {
        store
            .run_steps(ids::RUN_3)
            .await
            .expect("RUN_3 exists")
            .into_iter()
            .find(|row| row.id == id)
            .expect("the row exists")
            .status
    }

    /// Plan D66: `RUN_3`'s slot `(0, 1)` retires whole — the `done` winner, a third candidate
    /// still `pending`, and the `done` judge are superseded; the loser that is already
    /// `superseded` is left as it is.
    #[tokio::test]
    async fn retire_supersedes_every_candidate_and_the_judge_of_the_slot() {
        let store = MemStore::demo();
        let (run, snapshot) = run_3();
        let ctx = GateContext {
            store: &store,
            clock: &SystemClock,
            run: &run,
            snapshot: &snapshot,
            user: ids::USER,
            box_id: ids::BOX,
        };
        let third = step(&store, 1, 2, &[]).await;
        let judge = step(&store, 1, -1, &[StepStatus::Running, StepStatus::Done]).await;

        let steps = store.run_steps(ids::RUN_3).await.expect("RUN_3 exists");
        retire_slot(&ctx, &steps, 0, epoch())
            .await
            .expect("every move is legal");

        for id in [
            ids::STEP_R3_RESEARCH_A,
            ids::STEP_R3_RESEARCH_B,
            third.id,
            judge.id,
        ] {
            assert_eq!(status_of(&store, id).await, StepStatus::Superseded);
        }
        retire_slot(&ctx, &steps, 7, epoch())
            .await
            .expect("a position with no rows is nothing to retire");
    }

    /// M2 D5's split, applied to a group: `running` and `failed` rows — candidates and the judge —
    /// are cancelled, and only the latest attempt's slot is touched.
    #[tokio::test]
    async fn retire_cancels_a_failed_candidate_and_a_failed_judge() {
        let store = MemStore::demo();
        let (run, snapshot) = run_3();
        let ctx = GateContext {
            store: &store,
            clock: &SystemClock,
            run: &run,
            snapshot: &snapshot,
            user: ids::USER,
            box_id: ids::BOX,
        };
        let live = step(&store, 2, 0, &[StepStatus::Running]).await;
        let failed = step(&store, 2, 1, &[StepStatus::Running, StepStatus::Failed]).await;
        let judge = step(&store, 2, -1, &[StepStatus::Running, StepStatus::Failed]).await;

        let steps = store.run_steps(ids::RUN_3).await.expect("RUN_3 exists");
        retire_slot(&ctx, &steps, 0, epoch())
            .await
            .expect("every move is legal");

        for id in [live.id, failed.id, judge.id] {
            assert_eq!(status_of(&store, id).await, StepStatus::Cancelled);
        }
        assert_eq!(
            status_of(&store, ids::STEP_R3_RESEARCH_A).await,
            StepStatus::Done,
            "attempt 1 is not the latest slot and is left alone"
        );
        assert_eq!(
            status_of(&store, ids::STEP_R3_RESEARCH_B).await,
            StepStatus::Superseded
        );
    }

    /// Plan D66: the hash half compares the winners of attempts 1 and 2. Attempt 2's winner sits
    /// at index 1, so reading index 0 — the pre-fan-out `at()` — would answer the opposite in both
    /// halves of this case.
    #[tokio::test]
    async fn no_progress_compares_the_two_winners_not_index_zero() {
        let store = MemStore::demo();
        let (mut run, snapshot) = run_3();
        let repo = RepoId::new();
        store
            .create_repo(NewRepo {
                id: repo,
                project_id: run.project_id,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo project has no repo yet");
        run.repo_scope = vec![repo];
        let ctx = GateContext {
            store: &store,
            clock: &SystemClock,
            run: &run,
            snapshot: &snapshot,
            user: ids::USER,
            box_id: ids::BOX,
        };
        let settled = [StepStatus::Running, StepStatus::Done];
        let loser = step(&store, 2, 0, &settled).await;
        let winner = step(&store, 2, 1, &settled).await;
        store
            .select_fanout(ids::RUN_3, 0, 2, winner.id, None)
            .await
            .expect("index 1 is a settled candidate of (0, 2)");
        let record = |step, after: &str| {
            let rows = [RunStepCommit {
                run_step_id: step,
                repo_id: repo,
                before_hash: "base".to_owned(),
                after_hash: Some(after.to_owned()),
            }];
            let store = &store;
            async move { store.record_commits(step, &rows).await.expect("recorded") }
        };
        let steps = store.run_steps(ids::RUN_3).await.expect("RUN_3 exists");

        // Attempt 1's winner (index 0, `selected`) and attempt 2's loser agree; the winner moved.
        record(ids::STEP_R3_RESEARCH_A, "same").await;
        record(loser.id, "same").await;
        record(winner.id, "moved").await;
        assert_eq!(
            no_progress(&ctx, &steps, 0, 1, 2).await.expect("reads"),
            None,
            "the winner made progress, whatever index 0 did"
        );

        // And the other way round: the winner repeats attempt 1, index 0 does not.
        record(loser.id, "moved").await;
        record(winner.id, "same").await;
        assert_eq!(
            no_progress(&ctx, &steps, 0, 1, 2).await.expect("reads"),
            Some(LoopStop::NoProgressHash),
            "the winner repeated attempt 1's winner"
        );
    }

    /// Plan D125 (review H1): every compare-and-set of the gate honours `Ok(false)`. A row another
    /// writer moved first — the stranger's sweep failing attempt 1 after this walk's lease lapsed —
    /// is a `StaleWrite`, and the gate writes nothing past the refused move: no `done`, no park,
    /// no `finish_run`.
    #[tokio::test]
    async fn a_stale_compare_and_set_in_the_gate_is_a_stale_write() {
        use crate::command::EngineError;
        use htui_core::model::{Gate, RunStatus};

        let (run, snapshot) = run_3();
        let phase_with = |gate: Gate| {
            let mut phase = snapshot.phases[0].clone();
            phase.gate_effective = gate;
            phase.retry_limit = 0;
            phase
        };
        let stale = |refused: &EngineError, row: &str, from: &str, to: &str| {
            assert!(
                matches!(
                    refused,
                    EngineError::StaleWrite { run: stale, row: got, from: f, to: t }
                        if *stale == ids::RUN_3 && got == row && f == from && t == to
                ),
                "{refused}"
            );
        };

        // `never` + ok on a step another writer failed: `running -> done` is refused.
        let store = MemStore::demo();
        let ctx = GateContext {
            store: &store,
            clock: &SystemClock,
            run: &run,
            snapshot: &snapshot,
            user: ids::USER,
            box_id: ids::BOX,
        };
        let failed = step(&store, 2, 0, &[StepStatus::Running, StepStatus::Failed]).await;
        let refused = super::apply(
            &ctx,
            &failed,
            &phase_with(Gate::Never),
            Settle::Ok { note: None },
        )
        .await
        .expect_err("the step is not `running`");
        stale(&refused, &format!("step {}", failed.id), "running", "done");
        assert_eq!(status_of(&store, failed.id).await, StepStatus::Failed);

        // `always` on the same step: the park's first move is refused.
        let refused = super::apply(
            &ctx,
            &failed,
            &phase_with(Gate::Always),
            Settle::Ok { note: None },
        )
        .await
        .expect_err("the step is not `running`");
        stale(
            &refused,
            &format!("step {}", failed.id),
            "running",
            "awaiting_approval",
        );
        assert_eq!(status_of(&store, failed.id).await, StepStatus::Failed);

        // `never` + a verdict: the automatic rejection's park is refused, and nothing is answered.
        let refused = super::apply(
            &ctx,
            &failed,
            &phase_with(Gate::Never),
            Settle::Rejected {
                verdict_line: "verdict: request-changes".to_owned(),
            },
        )
        .await
        .expect_err("the step is not `running`");
        stale(
            &refused,
            &format!("step {}", failed.id),
            "running",
            "awaiting_approval",
        );

        // `never` + failed out of budget on a step another writer finished: no `finish_run`.
        let done = step(&store, 3, 0, &[StepStatus::Running, StepStatus::Done]).await;
        let refused = super::apply(
            &ctx,
            &done,
            &phase_with(Gate::Never),
            Settle::Failed(StepFailure::MissingOutput),
        )
        .await
        .expect_err("the step is not `running`");
        stale(&refused, &format!("step {}", done.id), "running", "failed");
        assert_eq!(status_of(&store, done.id).await, StepStatus::Done);
        let row = store
            .run(ids::RUN_3)
            .await
            .expect("MemStore never fails a read")
            .expect("RUN_3 exists");
        assert_eq!(row.status, RunStatus::Done, "no `finish_run` was written");
        assert_eq!(row.failure, None);

        // A park whose step move lands but whose run move does not: `RUN_3` is `done`, not
        // `running`, so the run's compare-and-set is the one refused.
        let live = step(&store, 4, 0, &[StepStatus::Running]).await;
        let refused = super::apply(
            &ctx,
            &live,
            &phase_with(Gate::Always),
            Settle::Ok { note: None },
        )
        .await
        .expect_err("the run is not `running`");
        stale(&refused, "the run", "running", "awaiting_approval");
    }

    /// Plan D144 (review L-c): the automatic rejection's `answer_gate` is a compare-and-set on
    /// `awaiting_approval` like any other. A step another writer moved off `awaiting_approval`
    /// between the park and the answer answers `Ok(false)`, and that is a `StaleWrite`: no
    /// `gate_note`, and nothing after it. The test drives `reject_step` itself: no seam lands
    /// another writer between `apply`'s park and its answer, nor inside the engine's
    /// `fail_judge`, so that both call `reject_step` rests on reading them.
    #[tokio::test]
    async fn a_stale_automatic_rejection_is_a_stale_write() {
        use crate::command::EngineError;

        let store = MemStore::demo();
        let done = step(&store, 2, 0, &[StepStatus::Running, StepStatus::Done]).await;
        let refused = super::reject_step(
            &store,
            ids::RUN_3,
            done.id,
            "verdict: request-changes".to_owned(),
            chrono::Utc::now(),
        )
        .await
        .expect_err("the step is not `awaiting_approval`");
        assert!(
            matches!(
                &refused,
                EngineError::StaleWrite { run, row, from, to }
                    if *run == ids::RUN_3
                        && *row == format!("step {}", done.id)
                        && from == "awaiting_approval"
                        && to == "failed"
            ),
            "{refused}"
        );
        assert_eq!(status_of(&store, done.id).await, StepStatus::Done);
    }
}
