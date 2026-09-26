//! The nine operator commands of ANA-2 §6.2 (`docs/ANA-2.md:1557-1572`) and their enabling
//! guards: `StartRun`, `AnswerGate`, `RetryStep`, `CancelRun`, `SelectFanout`, `PromoteStep`,
//! `AcceptArtifact`, `Unblock` and `CloseOut`. §6.2's `CancelStep` and `OpenArtifact` are not
//! built (MOD-4 plan D178, D173): `CancelRun` is the one cancel, and an artefact is a read.
//!
//! §6.2's table has an "Enabled when" column, and the Runs tab (milestone 6) greys an action out
//! by exactly the rule the engine refuses it by. That only stays true if there is one rule, so the
//! rules live here as free functions over rows — no store handle, no clock, no engine — and both
//! callers read them. A guard that needs a row the caller has not got is a guard the Runs tab will
//! quietly reimplement.
//!
//! Milestone 6 makes that one function per verb (blueprint D184): [`retry_admitted`],
//! [`promote_enabled`], [`accept_enabled`], [`unblock_enabled`], [`close_out_enabled`] and
//! [`cleanup_enabled`] are what the engine calls after its reads and before its lease, and what
//! `run_worker`'s greying read calls over the same rows. [`start_enabled`] mirrors `create_run`'s
//! own compare-and-set and is the one the engine does not call.
//!
//! **[`Rest`] and [`EngineError`] live here rather than in `engine.rs`.** The blueprint's §5.1 puts
//! them beside the `Engine`, but [`CommandOutcome`] carries the first and the two guards return the
//! second, and `engine.rs` is still T4's stub — so they have to be in a module this task owns. The
//! home is defensible on its own terms: this is the vocabulary of the crate's command surface, and
//! milestone 6's Runs tab speaks it without ever constructing an `Engine`. T4 uses both from here.

use std::path::PathBuf;

use htui_agent::driver::AgentSessionRef;
use htui_core::model::{
    AgentId, Claim, DocumentId, GraphSnapshot, Item, ItemId, RepoId, Resolution, Run, RunId,
    RunMode, RunStatus, RunStep, SnapshotPhase, Status, StepId, StepStatus,
};
use htui_core::prompt::AssembleError;
use htui_core::store::StoreError;

use crate::graph::ResolveError;
use crate::isolate::IsolateError;
use crate::status::{Cursor, RunFailure, group_at, latest_at, may_attempt, resumable_park};

/// What a human asks the orchestrator to do, in manual mode (ANA-2 §6.2, `docs/ANA-2.md:1557`).
///
/// Nine of §6.2's ten. `CancelStep` is not declared (MOD-4 plan D178): `CancelRun` is the one
/// cancel, and a cancelled step under a live run has no row in §4.3's run table. `OpenArtifact`
/// is not a command at all (plan D173): the Runs tab reads the document, and a read is not an
/// orchestrator verb. `CancelRun` joined in milestone 3 (plan D45): ANA-2 §12 criterion 13 is
/// stated as "cancelling a run", and the cleanup it triggers is what the criterion is about.
/// `SelectFanout` joined in milestone 4 (plan D65): criterion 10's human pick after a judge
/// failure is that command. Milestone 6 adds `PromoteStep`, `AcceptArtifact`, `Unblock` and
/// `CloseOut` (plan D161, D163, D166, D167).
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
    /// §6.2's `cancel run` (plan D45): every live step to `cancelled`, the run to `cancelled`, and
    /// then ANA-2 §4.6's run-terminal cleanup.
    CancelRun {
        /// The run to stop. It carries no step: every one of them goes.
        run: RunId,
    },
    /// §6.2's `select` (`docs/ANA-2.md:1566`, criterion 10, plan D65): a human picks the winner of
    /// a fan-out slot the walk parked for selection (plan D50), and the walk goes on from it.
    SelectFanout {
        /// The parked run.
        run: RunId,
        /// The slot's `run_step.position`.
        position: i32,
        /// The slot's `run_step.attempt`.
        attempt: i32,
        /// The candidate that wins: a `fanout_index >= 0` step of the slot.
        winner: StepId,
    },
    /// §4.8's `promote to chat` (MOD-4 plan D163, blueprint D191, D192): the step keeps its row,
    /// is parked `awaiting_approval` with `promoted_at` set, and the answer says how its chat
    /// opens. No `run(kind = 'chat')` row is written.
    PromoteStep {
        /// The run the step belongs to (blueprint H-11).
        run: RunId,
        /// The step to promote.
        step: StepId,
        /// Blueprint D185: a chat of this process is live. A fact only the caller can know; the
        /// worker overwrites whatever a view sent, and a headless caller passes `false`.
        chat_open: bool,
    },
    /// §4.8's `accept artifact` (plan D166, blueprint D194): the promoted step's stage 5 — verify,
    /// capture, settle — and then the `AnswerGate(Approved)` tail, walking on at `position + 1`.
    AcceptArtifact {
        /// The run the step belongs to (blueprint H-11).
        run: RunId,
        /// The promoted step.
        step: StepId,
        /// Blueprint D185: a chat of this process is live **on this step**.
        chat_live: bool,
    },
    /// §4.3 verdict 1's `unblock` (plan D161): one human action clears a `blocked` item, in one of
    /// [`UnblockCase`]'s three ways.
    Unblock {
        /// The item to unblock.
        item: ItemId,
    },
    /// `R-TUI-9`'s close-out (plan D167): one `summary` document and the item `closed` as
    /// `resolution` (MOD-38 PRD D2). The engine builds the summary; the caller names the item
    /// and the resolution — until MOD-39, the one [`close_out_enabled`] answers.
    CloseOut {
        /// The item to close.
        item: ItemId,
        /// Why it closes (ANA-11 §4.2).
        resolution: Resolution,
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
    /// [`Command::SelectFanout`]: where the walk stopped after the winner was reconciled.
    Selected {
        /// Where the walk rested.
        rest: Rest,
    },
    /// [`Command::CancelRun`]: where the run stopped, which is where it was.
    Cancelled {
        /// The run's last position, and `Cancelled` as its status.
        rest: Rest,
    },
    /// [`Command::PromoteStep`]: the promoted step, where its run is parked, and how its chat
    /// opens.
    Promoted {
        /// The step, which keeps its id (criterion 17).
        step: StepId,
        /// Where the run rests: parked, since a promoted step waits on a human.
        rest: Rest,
        /// How the chat opens (blueprint D192). Boxed: it is the largest arm by far.
        opening: Box<Opening>,
    },
    /// [`Command::AcceptArtifact`]: where the walk stopped after the step was approved.
    Accepted {
        /// Where the walk rested.
        rest: Rest,
    },
    /// [`Command::Unblock`]: which of the three cases cleared the item.
    Unblocked {
        /// The item.
        item: ItemId,
        /// What was done (plan D161).
        case: UnblockCase,
        /// Where the resumed walk rested, for [`UnblockCase::Resume`]; `None` otherwise.
        rest: Option<Rest>,
    },
    /// [`Command::CloseOut`]: the `summary` document written, and the version it landed at.
    ClosedOut {
        /// The item, now `closed`.
        item: ItemId,
        /// `document.id` of the summary.
        summary: DocumentId,
        /// `document.version` of the summary.
        version: i32,
    },
}

/// How a promoted step's chat opens (MOD-4 plan D163, blueprint D192).
///
/// There is no `AssembledPrompt` here: it is not `Eq` (`CommandOutcome` is), and its trim record
/// must not be written over the step's. The text and its digest are all a chat needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    /// `run_step.agent_id`: the agent the step ran on, which the chat drives.
    pub agent_id: AgentId,
    /// `agent.name`, for the chat header.
    pub agent_name: String,
    /// `run_step.model`.
    pub model: Option<String>,
    /// `run_step.phase_name`.
    pub phase: String,
    /// The step's own primary tree, else its first (ANA-2 `:1208-1213`).
    pub cwd: PathBuf,
    /// Every other tree of the step.
    pub extra_dirs: Vec<PathBuf>,
    /// Resume the step's own session, or open a fresh one with the handoff prompt.
    pub path: OpeningPath,
}

/// Which of §4.8's two openings a promoted step gets (blueprint D192, D193).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpeningPath {
    /// The step's own agent session, resumed; `text` is `promote::RESUME_OPENING`, recorded as the
    /// chat's first `follow_up`.
    Resume {
        /// The session id the step's `session_started` banner recorded.
        session_ref: AgentSessionRef,
        /// The first message the chat sends.
        text: String,
    },
    /// A fresh session opened with the `handoff` role's assembled prompt.
    Handoff {
        /// The assembled, scrubbed handoff prompt.
        text: String,
        /// Its digest, as `AssembledPrompt::digest` computes it.
        digest: String,
    },
}

/// Which of §4.3 verdict 1's three ways `Unblock` clears an item (MOD-4 plan D161).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnblockCase {
    /// The item is `blocked` and no run of it is active: back to `open`, and a new run may start.
    Reopen,
    /// The item is `blocked` and this run is parked (an escalation, `gate.rs`'s `escalate`): the
    /// item follows the run to `awaiting_approval`, so the run's gate verbs reach it (R-4).
    FollowRun(RunId),
    /// The item is `awaiting_approval` over this run, parked where a crashed command or a
    /// refused reconcile left it (R-7, plan D132): the run is resumed.
    Resume(RunId),
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
    /// Plan D65: a fan-out retry named a member of a slot a later attempt already replaced, so
    /// there is nothing for it to retry — the position's latest slot is the only one admitted.
    /// Plan D134 applies the same rule to a single step's retry.
    #[error("step {step} is in attempt {attempt}; retry names a member of the latest, {latest}")]
    StaleSlot {
        /// The step named.
        step: StepId,
        /// Its `attempt`.
        attempt: i32,
        /// The position's latest attempt, whose members a retry may name.
        latest: i32,
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
    /// (`crates/htui-core/src/model/item.rs:46-60`), so a blocked item is not walked on until
    /// [`Command::Unblock`] clears it (`docs/ANA-2.md:1568-1569`, MOD-4 plan D161).
    #[error("item {item} is blocked; `Unblock` (`u`) clears it (ANA-2 §4.3)")]
    ItemBlocked {
        /// The item holding the walk.
        item: ItemId,
    },
    /// Plan D83: why `claim_run` refused — the box is at `max_concurrent_items`, or the run's
    /// scope overlaps a live one under ANA-2 §4.7's rules L, I or P. The run stays `queued`, and
    /// `Engine::claim` re-attempts it (plan D84).
    #[error("claim refused: {claim}")]
    ClaimRefused {
        /// The run that stayed queued.
        run: RunId,
        /// What `claim_run` answered; never [`Claim::Admitted`].
        claim: Claim,
    },
    /// Plan D86: a heartbeat's refresh touched zero rows, so another orchestrator took the lease
    /// and the walk was dropped where it stood, writing nothing further (ANA-2 `:1280-1282`).
    ///
    /// Plan D122 raises it too when refreshes kept failing until the lease was one interval from
    /// lapsing: past that point another box's sweep may take the lease at any moment, so the walk
    /// is treated as though it already had.
    #[error(
        "run {run}: its lease was taken by another orchestrator; this walk was abandoned (ANA-2 §4.9)"
    )]
    LeaseLost {
        /// The run whose walk was abandoned.
        run: RunId,
    },
    /// Plan D87 (and blueprint A-1's resume): the run's lease is live and this process does not
    /// hold it, so nothing was written. A lease this process took before a write that then failed
    /// stays live until its TTL (blueprint H-7), and reads the same way meanwhile. Only a **live**
    /// lease reads this way (plan D130): a run no lease can be taken on for another reason — not
    /// `running`/`awaiting_approval`, or executing elsewhere with its lease lapsed — is
    /// [`EngineError::RunStatus`].
    #[error("run {run}: another orchestrator holds a live lease (ANA-2 §4.9)")]
    LeaseHeld {
        /// The run whose lease is held elsewhere.
        run: RunId,
    },
    /// Plan D125: a compare-and-set on the walk's path answered `Ok(false)`: the row was not in
    /// the status this walk last read, because another writer moved it (typically another box's
    /// sweep, after this walk's lease lapsed). The walk stops at once and writes nothing further,
    /// in particular no reconcile. This supersedes ANA-2's offline narrative ("the live session
    /// keeps running"): a walk that cannot prove its lease stops. A few failure and recovery
    /// writes are exempt, and the engine's `move_step` names them and why (plan D144).
    ///
    /// Built by [`stale_step`] and [`stale_run`], so the two row shapes read alike.
    #[error(
        "run {run}: {row} was not `{from}` when this walk moved it to `{to}`; another writer moved \
         it first, so the walk stopped (ANA-2 §4.9)"
    )]
    StaleWrite {
        /// The run the walk was driving.
        run: RunId,
        /// The row the compare-and-set named: `step <id>` or `the run`.
        row: String,
        /// The status the walk expected the row to hold.
        from: String,
        /// The status the walk tried to move it to.
        to: String,
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
    /// Plan D65: `SelectFanout` on a slot holding one candidate — a `fan_out = 1` phase, whose one
    /// step is answered through `AnswerGate` instead.
    #[error(
        "run {run} position {position} attempt {attempt} holds one candidate; there is nothing to select"
    )]
    NotAFanout {
        /// The run named.
        run: RunId,
        /// The slot's position.
        position: i32,
        /// The slot's attempt.
        attempt: i32,
    },
    /// Plan D65: the slot already has a winner — a double click, or a judge that won the race.
    #[error("run {run} position {position} attempt {attempt} already selected step {selected}")]
    AlreadySelected {
        /// The run named.
        run: RunId,
        /// The slot's position.
        position: i32,
        /// The slot's attempt.
        attempt: i32,
        /// The candidate carrying `selected = true`.
        selected: StepId,
    },
    /// Plan D65: the named winner is not one of the slot's candidates — another slot's step, or
    /// the judge.
    #[error("step {step} is not a candidate of run {run} position {position} attempt {attempt}")]
    NotACandidate {
        /// The step named as the winner.
        step: StepId,
        /// The run named.
        run: RunId,
        /// The slot's position.
        position: i32,
        /// The slot's attempt.
        attempt: i32,
    },
    /// ANA-2 `:754-758`: a retried group starts from the base its retired slot started from, and
    /// that slot's `before_hash` rows do not name one — two bases for a repository, or a
    /// repository of the scope with none. Starting from `HEAD` instead would start the group
    /// from wherever a retired `shared_serialized` sibling left the checkout.
    #[error("run {run} position {position} attempt {attempt}: no base to start from: {reason}")]
    GroupBase {
        /// The run named.
        run: RunId,
        /// The group's position.
        position: i32,
        /// The attempt that would have started.
        attempt: i32,
        /// What the retired rows say, in a readable sentence.
        reason: String,
    },
    /// Stage 2 or stage 5's isolation verb refused. **Not in the blueprint's §5.1 list**, which
    /// leaves the engine no way to carry an `Isolator`'s own failure; added here.
    #[error(transparent)]
    Isolate(#[from] IsolateError),
    /// MOD-4 plan D161: `Unblock` found nothing it may clear. `why` names what holds the item —
    /// nothing, a walking run, or a gate a human answers instead.
    #[error("item {item} is `{status}`; {why}")]
    NotBlocked {
        /// The item named.
        item: ItemId,
        /// Its status.
        status: Status,
        /// What holds it, in a sentence ([`nothing_is_blocked`], [`run_is_walking`],
        /// [`run_waits_at_a_gate`]).
        why: String,
    },
    /// Plan D166: `AcceptArtifact` on a step that was never promoted (ANA-2 §4.8).
    #[error(
        "step {step} was not promoted to chat; accept artifact needs a promoted step (ANA-2 §4.8)"
    )]
    NotPromoted {
        /// The step named.
        step: StepId,
    },
    /// Blueprint D191: a fan-out candidate is one of several; the slot's winner is chosen by
    /// `SelectFanout`, not by promoting one of them.
    #[error("step {step} is a fan-out candidate; select a winner instead of promoting one")]
    PromoteCandidate {
        /// The candidate named.
        step: StepId,
    },
    /// Blueprint D185: a chat of this process is live. `None` for a promotion, which the one Chat
    /// tab cannot host beside another session; `Some` for an accept of the step being chatted with.
    #[error("{}", chat_is_live(.step))]
    ChatLive {
        /// The step whose chat is live, when the refusal is about one step.
        step: Option<StepId>,
    },
    /// Blueprint D194: the accepted artefact failed its `verify_command`. The outcome is recorded
    /// and the step stays promoted, so the fix belongs in the chat.
    #[error(
        "step {step}: the verify command failed (exit {}); fix it in the chat, then accept again",
        exit_code_text(.exit_code)
    )]
    AcceptVerifyFailed {
        /// The promoted step.
        step: StepId,
        /// The command's exit code, when it had one.
        exit_code: Option<i32>,
    },
    /// Plan D167: close-out needs a finished item (ANA-2 §4.10, `docs/ANA-2.md:1380-1392`).
    #[error(
        "item {item} is `{status}`; close-out needs `done`, `failed` or `blocked` (ANA-2 §4.10)"
    )]
    NotClosable {
        /// The item named.
        item: ItemId,
        /// Its status.
        status: Status,
    },
    /// Plan D177 (R-25): a manual cleanup retry is for a run that has finished.
    #[error("run {run} is `{status}`; a cleanup retry is for a finished run (R-25)")]
    NotTerminal {
        /// The run named.
        run: RunId,
        /// Its status.
        status: RunStatus,
    },
}

/// [`EngineError::ChatLive`]'s two sentences.
fn chat_is_live(step: &Option<StepId>) -> String {
    match step {
        None => "end the open chat first (Chat tab, Esc Esc)".to_owned(),
        Some(step) => {
            format!("step {step} is being chatted with; end that chat first (Chat tab, Esc Esc)")
        }
    }
}

/// [`EngineError::AcceptVerifyFailed`]'s exit code, or `none` for a command that had none.
fn exit_code_text(code: &Option<i32>) -> String {
    code.map_or_else(|| "none".to_owned(), |code| code.to_string())
}

/// [`EngineError::NotBlocked`]'s `why` when the item is not held by anything `Unblock` clears.
#[must_use]
pub fn nothing_is_blocked() -> String {
    "nothing is blocked".to_owned()
}

/// [`EngineError::NotBlocked`]'s `why` when a run of the item is walking or queued.
#[must_use]
pub fn run_is_walking(run: RunId, status: RunStatus) -> String {
    format!("run {run} is `{status}`")
}

/// [`EngineError::NotBlocked`]'s `why` when the item's run waits at a gate a human answers.
#[must_use]
pub fn run_waits_at_a_gate(run: RunId) -> String {
    format!("run {run} is parked at a gate; answer it")
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
/// **An interrupted step is exempt from the budget** (blueprint A-8): a `failed` step whose
/// `gate_note` starts with `interrupted` was failed by the recovery sweep after a crash, which
/// plan D92 says "is not the agent's failed settle". The budget bounds automatic retries, and a
/// human choosing to retry after a crash is not one.
///
/// `steps` are the run's rows: only the position's latest attempt ([`latest_at`]) is retryable
/// (plan D134), as a fan-out retry names only a member of the latest slot.
///
/// Two things this guard deliberately does **not** check, because they need rows it is not given:
/// that the run is non-terminal, and that the item is not `blocked`
/// ([`EngineError::ItemBlocked`], blueprint F-J). Both are [`retry_admitted`]'s, the whole order
/// the engine and the Runs tab share (blueprint D184).
///
/// # Errors
/// [`EngineError::NotGated`] for any other step status; [`EngineError::StaleSlot`] for an attempt
/// a later one replaced; [`EngineError::RetryExhausted`] when the budget is spent.
pub fn retry_enabled(
    steps: &[RunStep],
    step: &RunStep,
    phase: &SnapshotPhase,
) -> Result<(), EngineError> {
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
    // Plan D134: a later attempt already replaced this one, so there is nothing for a retry to
    // replace — and A-8's exemption below must not reach an older interrupted row.
    latest_attempt(steps, step)?;
    if !may_attempt(step.attempt + 1, phase.retry_limit) && !interrupted(step) {
        return Err(EngineError::RetryExhausted {
            step: step.id,
            attempt: step.attempt,
            retry_limit: phase.retry_limit,
        });
    }
    Ok(())
}

/// Plan D134: `step` is its position's latest attempt ([`latest_at`] over the run's `steps`), so
/// a verb on it names the row the walk reads. An attempt a later one replaced is
/// [`EngineError::StaleSlot`].
fn latest_attempt(steps: &[RunStep], step: &RunStep) -> Result<(), EngineError> {
    match latest_at(steps, step.position) {
        Some(latest) if latest.attempt != step.attempt => Err(EngineError::StaleSlot {
            step: step.id,
            attempt: step.attempt,
            latest: latest.attempt,
        }),
        _ => Ok(()),
    }
}

/// Blueprint A-8: a `failed` step whose `gate_note` the sweep wrote (`interrupted`, or
/// `interrupted, tree not reset`, plan D115). The prefix is the whole rule, as the blueprint
/// states it; no settle of the walk's own writes a note that starts with it.
fn interrupted(step: &RunStep) -> bool {
    step.status == StepStatus::Failed
        && step
            .gate_note
            .as_deref()
            .is_some_and(|note| note.starts_with("interrupted"))
}

/// §6.2's "Enabled when" for `select` (`docs/ANA-2.md:1566`, plan D65).
///
/// `slot` is `status::group_at(steps, position, attempt)`: the slot's candidates, judge excluded.
/// The run is `awaiting_approval` — D50's park is the only way a group reaches a human — the slot
/// holds more than one candidate, none of them is `selected` yet, and `winner` is one of them at
/// `done | awaiting_approval`, which is exactly what `WriteStore::select_fanout` accepts
/// (`crates/htui-core/src/store/traits.rs:821-824`) — so a pick this guard admits is never refused
/// by the write.
///
/// **A group parked because its winner's reconcile was refused is not selected again** (blueprint
/// R-7): it carries `selected = true` and is refused here as [`EngineError::AlreadySelected`].
/// `Unblock` (`u`, MOD-4 plan D161 case 3) resumes that run and retries the reconcile.
///
/// # Errors
/// [`EngineError::RunStatus`], [`EngineError::NotAFanout`], [`EngineError::AlreadySelected`],
/// [`EngineError::NotACandidate`] or [`EngineError::NotGated`], in that order.
pub fn select_enabled(
    run: &Run,
    slot: &[&RunStep],
    winner: StepId,
    position: i32,
    attempt: i32,
) -> Result<(), EngineError> {
    parked(run)?;
    if slot.len() < 2 {
        return Err(EngineError::NotAFanout {
            run: run.id,
            position,
            attempt,
        });
    }
    unselected(run, slot, position, attempt)?;
    let Some(candidate) = slot.iter().find(|step| step.id == winner) else {
        return Err(EngineError::NotACandidate {
            step: winner,
            run: run.id,
            position,
            attempt,
        });
    };
    if !matches!(
        candidate.status,
        StepStatus::Done | StepStatus::AwaitingApproval
    ) {
        return Err(EngineError::NotGated {
            step: candidate.id,
            status: candidate.status,
            expected: "done | awaiting_approval",
        });
    }
    Ok(())
}

/// §6.2's `retry` on a member of a parked fan-out slot (plan D65, blueprint F-D): the whole group
/// is retired and admitted again at `attempt + 1`.
///
/// [`retry_enabled`] cannot answer it: a parked group's candidates are `done`, and a candidate is
/// never `awaiting_approval` (D50). So the guard reads the run — `awaiting_approval` — the slot —
/// nothing `selected` — and the budget, [`may_attempt`] on the slot's `attempt + 1`.
///
/// # Errors
/// [`EngineError::RunStatus`], [`EngineError::NotAFanout`] for an empty slot,
/// [`EngineError::AlreadySelected`] or [`EngineError::RetryExhausted`].
pub fn retry_group_enabled(
    run: &Run,
    slot: &[&RunStep],
    phase: &SnapshotPhase,
) -> Result<(), EngineError> {
    parked(run)?;
    let Some(first) = slot.first() else {
        return Err(EngineError::NotAFanout {
            run: run.id,
            position: phase.position,
            attempt: 0,
        });
    };
    unselected(run, slot, first.position, first.attempt)?;
    if !may_attempt(first.attempt + 1, phase.retry_limit) {
        return Err(EngineError::RetryExhausted {
            step: first.id,
            attempt: first.attempt,
            retry_limit: phase.retry_limit,
        });
    }
    Ok(())
}

/// Plan D125's refusal for a `transition_step` that answered `Ok(false)` on the walk's path.
#[must_use]
pub fn stale_step(run: RunId, step: StepId, from: StepStatus, to: StepStatus) -> EngineError {
    EngineError::StaleWrite {
        run,
        row: format!("step {step}"),
        from: from.to_string(),
        to: to.to_string(),
    }
}

/// Plan D125's refusal for a `transition_run` that answered `Ok(false)` on the walk's path.
#[must_use]
pub fn stale_run(run: RunId, from: RunStatus, to: RunStatus) -> EngineError {
    EngineError::StaleWrite {
        run,
        row: "the run".to_owned(),
        from: from.to_string(),
        to: to.to_string(),
    }
}

/// Both group guards' first clause: the run is parked for a human (plan D50).
fn parked(run: &Run) -> Result<(), EngineError> {
    if run.status == RunStatus::AwaitingApproval {
        return Ok(());
    }
    Err(EngineError::RunStatus {
        run: run.id,
        status: run.status,
        expected: "awaiting_approval",
    })
}

/// Both group guards' slot clause: no candidate carries `selected = true` yet.
fn unselected(
    run: &Run,
    slot: &[&RunStep],
    position: i32,
    attempt: i32,
) -> Result<(), EngineError> {
    match slot.iter().find(|step| step.selected == Some(true)) {
        None => Ok(()),
        Some(selected) => Err(EngineError::AlreadySelected {
            run: run.id,
            position,
            attempt,
            selected: selected.id,
        }),
    }
}

/// §6.2's "Enabled when" for `cancel run` (plan D45): the run has not finished.
///
/// `queued`, `running` and `awaiting_approval` are exactly the three `RunStatus::can_move_to`
/// admits `cancelled` from (`crates/htui-core/src/model/run.rs`), and exactly
/// [`RunStatus::is_active`]'s three — so the guard is stated against the table rather than against
/// a list, and a status added to §4.3 cannot be silently left out of it.
///
/// A `queued` run has no steps and was never claimed; cancelling it is still legal and still ends
/// with a cleanup, because the run may have been claimed and crashed between the two reads.
///
/// **"Only if no session is live" is not checked here.** In this milestone's synchronous walk it
/// is true by construction — a step is `running` only inside one `dispatch` call, and a second
/// command cannot be dispatched while it is — and since milestone 6, where a session outlives a
/// dispatch, `run_worker` preempts the run's live walk before it dispatches the cancel (MOD-4
/// plan D157), and the engine takes the run's lease first (plan D179). A promoted step's chat is
/// no walk and outlives every one, so `run_worker` refuses the cancel while a chat is live on a
/// step of the run (plan D212).
///
/// # Errors
/// [`EngineError::RunStatus`] for a `done`, `failed` or `cancelled` run.
pub fn cancel_enabled(run: &Run) -> Result<(), EngineError> {
    if run.status.is_active() {
        return Ok(());
    }
    Err(EngineError::RunStatus {
        run: run.id,
        status: run.status,
        expected: "queued | running | awaiting_approval",
    })
}

/// The run's own snapshot, decoded. Invariant 2: the walk reads this and never the live graph.
///
/// Only a genuine version mismatch is [`EngineError::SnapshotVersion`]. A row with no snapshot
/// at all, or one whose blob does not decode, is [`EngineError::Snapshot`] with the serde error
/// kept, so the operator is not told the engine is too old to read a run it can read nothing of.
///
/// Public since milestone 6 (blueprint D184): the Runs tab's greying read decodes the same way.
///
/// # Errors
/// [`EngineError::Snapshot`] or [`EngineError::SnapshotVersion`].
pub fn snapshot_of(run: &Run) -> Result<GraphSnapshot, EngineError> {
    let value = run
        .graph_snapshot
        .clone()
        .ok_or_else(|| EngineError::Snapshot {
            run: run.id,
            reason: "the run carries no `graph_snapshot`; a graph run is created with one"
                .to_owned(),
        })?;
    let snapshot: GraphSnapshot =
        serde_json::from_value(value).map_err(|err| EngineError::Snapshot {
            run: run.id,
            reason: format!("`graph_snapshot` does not decode: {err}"),
        })?;
    if snapshot.v != GraphSnapshot::V {
        return Err(EngineError::SnapshotVersion {
            run: run.id,
            v: snapshot.v,
        });
    }
    Ok(snapshot)
}

/// The snapshot phase at `position`.
///
/// A position the snapshot does not name is an engine invariant and not a store refusal:
/// invariant 2 makes the snapshot the only thing a live run reads, so there is no second source
/// to try and nothing the store did wrong (see [`EngineError::Snapshot`]).
///
/// # Errors
/// [`EngineError::Snapshot`] naming the position.
pub fn phase_at(
    run: RunId,
    snapshot: &GraphSnapshot,
    position: i32,
) -> Result<SnapshotPhase, EngineError> {
    snapshot
        .phases
        .iter()
        .find(|phase| phase.position == position)
        .cloned()
        .ok_or_else(|| EngineError::Snapshot {
            run,
            reason: format!("no phase at position {position}"),
        })
}

/// §6.2's `retry` as the engine admits it (blueprint D184): `retry_step`'s whole order before
/// its lease, over the rows it read.
///
/// 1. The run is `running` or `awaiting_approval` — on a terminal run every write would be a
///    no-op that still creates an orphan `pending` step ([`EngineError::RunStatus`]).
/// 2. The item is not `blocked` ([`EngineError::ItemBlocked`], blueprint F-J).
/// 3. A member of a fanned-out slot (plan D65) retries the whole group: only a member of the
///    position's latest slot ([`EngineError::StaleSlot`]), then [`retry_group_enabled`].
/// 4. Otherwise [`retry_enabled`].
///
/// # Errors
/// The first refusal in that order.
pub fn retry_admitted(
    run: &Run,
    item: Status,
    steps: &[RunStep],
    step: &RunStep,
    phase: &SnapshotPhase,
) -> Result<(), EngineError> {
    if !matches!(run.status, RunStatus::Running | RunStatus::AwaitingApproval) {
        return Err(EngineError::RunStatus {
            run: run.id,
            status: run.status,
            expected: "running | awaiting_approval",
        });
    }
    if item == Status::Blocked
        && let Some(item) = run.item_id
    {
        return Err(EngineError::ItemBlocked { item });
    }
    if phase.fan_out > 1 {
        // `retire_slot` retires the position's **latest** slot, so only a member of that one
        // names something a retry could replace.
        let latest = steps
            .iter()
            .filter(|row| row.position == step.position)
            .map(|row| row.attempt)
            .max()
            .unwrap_or(step.attempt);
        if step.attempt != latest {
            return Err(EngineError::StaleSlot {
                step: step.id,
                attempt: step.attempt,
                latest,
            });
        }
        return retry_group_enabled(run, &group_at(steps, step.position, step.attempt), phase);
    }
    retry_enabled(steps, step, phase)
}

/// §6.2's `promote to chat` (`docs/ANA-2.md:1563`, MOD-4 plan D163, blueprint D191).
///
/// The run has not finished, the step is `running`, `awaiting_approval` or `failed`, the step is
/// its position's latest attempt, the item is not `blocked` (an escalated item needs `Unblock`
/// first, plan D161 case 2), the step is not a fan-out candidate, and no chat of this process is
/// live (blueprint D185: the Chat tab hosts one session).
///
/// `steps` are the run's rows. Plan D134's rule, as [`retry_enabled`] applies it: a `failed`
/// attempt a retry replaced stays `failed` for good, and promoting it would park a second step
/// beside the position's real one, which the walk never reads ([`latest_at`]).
///
/// # Errors
/// [`EngineError::RunStatus`], [`EngineError::NotGated`], [`EngineError::StaleSlot`],
/// [`EngineError::ItemBlocked`], [`EngineError::PromoteCandidate`] or [`EngineError::ChatLive`],
/// in that order.
pub fn promote_enabled(
    run: &Run,
    item: Status,
    steps: &[RunStep],
    step: &RunStep,
    phase: &SnapshotPhase,
    chat_open: bool,
) -> Result<(), EngineError> {
    if !matches!(run.status, RunStatus::Running | RunStatus::AwaitingApproval) {
        return Err(EngineError::RunStatus {
            run: run.id,
            status: run.status,
            expected: "running | awaiting_approval",
        });
    }
    if !matches!(
        step.status,
        StepStatus::Running | StepStatus::AwaitingApproval | StepStatus::Failed
    ) {
        return Err(EngineError::NotGated {
            step: step.id,
            status: step.status,
            expected: "running | awaiting_approval | failed",
        });
    }
    latest_attempt(steps, step)?;
    if item == Status::Blocked
        && let Some(item) = run.item_id
    {
        return Err(EngineError::ItemBlocked { item });
    }
    if phase.fan_out > 1 {
        return Err(EngineError::PromoteCandidate { step: step.id });
    }
    if chat_open {
        return Err(EngineError::ChatLive { step: None });
    }
    Ok(())
}

/// §4.8's `accept artifact` (`docs/ANA-2.md:1223-1233`, MOD-4 plan D166).
///
/// The step is parked, is its position's latest attempt (plan D134, as [`promote_enabled`]
/// checks it: a second line of defence, since accepting a replaced attempt would merge its old
/// tree), was promoted, produced its `output_kind` document (`has_output`, resolved as
/// [`answer_gate_enabled`]'s is), and no chat of this process is live on it. `steps` are the
/// run's rows.
///
/// # Errors
/// [`EngineError::NotGated`], [`EngineError::StaleSlot`], [`EngineError::NotPromoted`],
/// [`EngineError::MissingOutputForApproval`] or [`EngineError::ChatLive`], in that order.
pub fn accept_enabled(
    steps: &[RunStep],
    step: &RunStep,
    phase: &SnapshotPhase,
    has_output: bool,
    chat_live: bool,
) -> Result<(), EngineError> {
    if step.status != StepStatus::AwaitingApproval {
        return Err(EngineError::NotGated {
            step: step.id,
            status: step.status,
            expected: "awaiting_approval",
        });
    }
    latest_attempt(steps, step)?;
    if step.promoted_at.is_none() {
        return Err(EngineError::NotPromoted { step: step.id });
    }
    if !has_output {
        return Err(EngineError::MissingOutputForApproval {
            step: step.id,
            kind: phase.output_kind.clone(),
        });
    }
    if chat_live {
        return Err(EngineError::ChatLive {
            step: Some(step.id),
        });
    }
    Ok(())
}

/// §4.3 verdict 1's `unblock` (MOD-4 plan D161): which of [`UnblockCase`]'s three cases the item
/// is in, over the item's active runs and each one's cursor.
///
/// 1. `blocked` and no run is active: [`UnblockCase::Reopen`].
/// 2. `blocked` and an active run is parked: [`UnblockCase::FollowRun`].
/// 3. `awaiting_approval` over a parked run whose cursor is [`resumable_park`]:
///    [`UnblockCase::Resume`].
///
/// # Errors
/// [`EngineError::NotBlocked`] for anything else, naming what holds the item.
pub fn unblock_enabled(item: &Item, runs: &[(Run, Cursor)]) -> Result<UnblockCase, EngineError> {
    let active: Vec<&(Run, Cursor)> = runs
        .iter()
        .filter(|(run, _)| run.status.is_active())
        .collect();
    let parked = active
        .iter()
        .find(|(run, _)| run.status == RunStatus::AwaitingApproval);
    let why = match (item.status, parked, active.first()) {
        (Status::Blocked, _, None) => return Ok(UnblockCase::Reopen),
        (Status::Blocked, Some((run, _)), _) => return Ok(UnblockCase::FollowRun(run.id)),
        (Status::AwaitingApproval, Some((run, cursor)), _) if resumable_park(cursor) => {
            return Ok(UnblockCase::Resume(run.id));
        }
        (Status::AwaitingApproval, Some((run, _)), _) => run_waits_at_a_gate(run.id),
        (Status::Blocked | Status::AwaitingApproval, None, Some((run, _))) => {
            run_is_walking(run.id, run.status)
        }
        _ => nothing_is_blocked(),
    };
    Err(EngineError::NotBlocked {
        item: item.id,
        status: item.status,
        why,
    })
}

/// `R-TUI-9`'s close-out (MOD-4 plan D167, ANA-2 §4.10): no run of the item is active, and the
/// item is `done`, `failed` or `blocked` — the refusals `WriteStore::close_out` re-checks inside
/// its own transaction, stated here so the Runs tab greys the key by them — and the resolution
/// the Runs pane closes with ([`Resolution::default_for`], MOD-38 plan D6).
///
/// The store also closes an `open` item as one of the four non-success resolutions; until
/// MOD-39's picker the Runs pane does not offer that, so `default_for`'s `None` is refused here.
///
/// # Errors
/// [`EngineError::RunStatus`] naming the first live run, then [`EngineError::NotClosable`] when
/// `default_for(item.status)` is `None`.
pub fn close_out_enabled(item: &Item, runs: &[Run]) -> Result<Resolution, EngineError> {
    if let Some(live) = runs.iter().find(|run| run.status.is_active()) {
        return Err(EngineError::RunStatus {
            run: live.id,
            status: live.status,
            expected: "done | failed | cancelled",
        });
    }
    Resolution::default_for(item.status).ok_or(EngineError::NotClosable {
        item: item.id,
        status: item.status,
    })
}

/// MOD-4 plan D177 (R-25): a manual cleanup retry is for a run that has finished.
///
/// # Errors
/// [`EngineError::NotTerminal`] for a `queued`, `running` or `awaiting_approval` run.
pub fn cleanup_enabled(run: &Run) -> Result<(), EngineError> {
    if run.status.is_terminal() {
        return Ok(());
    }
    Err(EngineError::NotTerminal {
        run: run.id,
        status: run.status,
    })
}

/// §6.2's `run`/`queue` over the item row: the mirror of `create_run`'s own compare-and-set,
/// `open | failed -> queued` under the item law (`Status::can_move_to`).
///
/// **The engine does not call it**: `create_run` refuses inside its transaction, and
/// `legal_move` stays the authority. It exists so the Runs tab greys `R` by the same law, and
/// `start_enabled_is_the_law` pins the two together.
///
/// # Errors
/// [`EngineError::Store`] carrying `legal_move`'s own refusal.
pub fn start_enabled(item: &Item) -> Result<(), EngineError> {
    htui_core::store::legal_move(item.status, Status::Queued).map_err(EngineError::Store)
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        Claim, GraphSnapshot, Item, OverlapRule, Resolution, Run, RunStatus, RunStep,
        SnapshotPhase, Status, StepStatus,
    };
    use htui_core::store::StoreError;

    use htui_core::model::StepId;

    use super::{
        EngineError, GateAnswer, UnblockCase, accept_enabled, answer_gate_enabled, cleanup_enabled,
        close_out_enabled, nothing_is_blocked, promote_enabled, retry_admitted, retry_enabled,
        retry_group_enabled, run_is_walking, run_waits_at_a_gate, select_enabled, start_enabled,
        unblock_enabled,
    };
    use crate::status::Cursor;

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
                retry_enabled(std::slice::from_ref(&step), &step, &phase).is_ok(),
                "`{status}` is retryable"
            );
        }

        step.attempt = 2;
        let refused = retry_enabled(std::slice::from_ref(&step), &step, &phase)
            .expect_err("a third attempt is out of budget");
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
            retry_enabled(std::slice::from_ref(&step), &step, &phase)
                .expect_err("a done step is not retryable"),
            EngineError::NotGated { .. }
        ));
    }

    /// Blueprint A-8: a human's retry of a step the sweep failed as `interrupted` is not an
    /// automatic retry, so the budget does not bind it — while an agent's own failure at the same
    /// attempt still reads `RetryExhausted`.
    #[test]
    fn an_interrupted_step_is_retryable_past_its_budget() {
        let (mut step, mut phase) = review();
        phase.retry_limit = 0;
        step.status = StepStatus::Failed;
        step.attempt = 1;

        for note in ["interrupted", "interrupted, tree not reset"] {
            step.gate_note = Some(note.to_owned());
            assert!(
                retry_enabled(std::slice::from_ref(&step), &step, &phase).is_ok(),
                "`{note}` is a crash, not the agent's failed settle"
            );
        }

        step.gate_note = None;
        assert!(matches!(
            retry_enabled(std::slice::from_ref(&step), &step, &phase)
                .expect_err("an agent's failure spends the budget"),
            EngineError::RetryExhausted {
                attempt: 1,
                retry_limit: 0,
                ..
            }
        ));

        // The exemption is `failed`'s only: a parked step carrying the same note is budgeted.
        step.status = StepStatus::AwaitingApproval;
        step.gate_note = Some("interrupted".to_owned());
        assert!(matches!(
            retry_enabled(std::slice::from_ref(&step), &step, &phase)
                .expect_err("a parked step is budgeted"),
            EngineError::RetryExhausted { .. }
        ));
    }

    /// Plan D134 (review M4): only the position's latest attempt is retryable, so blueprint
    /// A-8's exemption cannot reach an interrupted attempt a later one already replaced. The
    /// latest interrupted attempt keeps it.
    #[test]
    fn only_the_latest_attempt_is_retryable() {
        let (mut step, mut phase) = review();
        phase.retry_limit = 0;
        step.status = StepStatus::Failed;
        step.attempt = 1;
        step.gate_note = Some("interrupted".to_owned());
        let mut later = step.clone();
        later.id = StepId::new();
        later.attempt = 2;
        let steps = [step.clone(), later.clone()];

        let refused =
            retry_enabled(&steps, &step, &phase).expect_err("attempt 2 replaced attempt 1");
        assert!(
            matches!(
                refused,
                EngineError::StaleSlot {
                    step: id,
                    attempt: 1,
                    latest: 2,
                } if id == step.id
            ),
            "{refused}"
        );
        assert!(
            retry_enabled(&steps, &later, &phase).is_ok(),
            "the latest interrupted attempt is still exempt from the budget"
        );
    }

    /// `RUN_3` parked on its `research` group: both candidates settled `done`, nothing selected,
    /// the run `awaiting_approval` — the state D50's park leaves behind — and its snapshot phase
    /// widened to the two candidates it holds.
    fn parked_group() -> (Run, Vec<RunStep>, SnapshotPhase) {
        let data = demo_data();
        let mut run = data
            .runs
            .iter()
            .find(|row| row.id == ids::RUN_3)
            .expect("the fixture holds RUN_3")
            .clone();
        run.status = RunStatus::AwaitingApproval;
        let mut steps: Vec<RunStep> = data
            .steps
            .into_iter()
            .filter(|step| step.run_id == ids::RUN_3)
            .collect();
        for step in &mut steps {
            step.status = StepStatus::Done;
            step.selected = None;
        }
        let (_, mut phase) = review();
        phase.position = 0;
        phase.fan_out = 2;
        phase.retry_limit = 1;
        (run, steps, phase)
    }

    /// Plan D65's guard, one refusal per clause, each naming what it found.
    #[test]
    fn select_is_enabled_only_on_a_parked_unselected_group_and_a_settled_candidate() {
        let (mut run, mut steps, _) = parked_group();
        let slot: Vec<&RunStep> = steps.iter().collect();
        assert!(select_enabled(&run, &slot, ids::STEP_R3_RESEARCH_B, 0, 1).is_ok());

        run.status = RunStatus::Running;
        assert!(
            matches!(
                select_enabled(&run, &slot, ids::STEP_R3_RESEARCH_B, 0, 1),
                Err(EngineError::RunStatus {
                    expected: "awaiting_approval",
                    ..
                })
            ),
            "a run that is not parked has nothing to select"
        );
        run.status = RunStatus::AwaitingApproval;

        let refused = select_enabled(&run, &slot[..1], ids::STEP_R3_RESEARCH_A, 0, 1)
            .expect_err("one candidate is not a fan-out");
        assert!(
            matches!(
                refused,
                EngineError::NotAFanout {
                    position: 0,
                    attempt: 1,
                    ..
                }
            ),
            "{refused}"
        );

        let stranger = StepId::new();
        let refused = select_enabled(&run, &slot, stranger, 0, 1)
            .expect_err("a step outside the slot is not a candidate");
        assert!(
            matches!(refused, EngineError::NotACandidate { step, .. } if step == stranger),
            "{refused}"
        );

        steps[1].status = StepStatus::Failed;
        let slot: Vec<&RunStep> = steps.iter().collect();
        let refused = select_enabled(&run, &slot, ids::STEP_R3_RESEARCH_B, 0, 1)
            .expect_err("a failed candidate cannot win");
        assert!(
            matches!(
                refused,
                EngineError::NotGated {
                    status: StepStatus::Failed,
                    expected: "done | awaiting_approval",
                    ..
                }
            ),
            "{refused}"
        );

        steps[0].selected = Some(true);
        let slot: Vec<&RunStep> = steps.iter().collect();
        let refused = select_enabled(&run, &slot, ids::STEP_R3_RESEARCH_B, 0, 1)
            .expect_err("a slot is selected once");
        assert!(
            matches!(
                refused,
                EngineError::AlreadySelected { selected, .. } if selected == ids::STEP_R3_RESEARCH_A
            ),
            "{refused}"
        );
        assert_eq!(
            refused.to_string(),
            format!(
                "run {} position 0 attempt 1 already selected step {}",
                ids::RUN_3,
                ids::STEP_R3_RESEARCH_A
            )
        );
    }

    /// Blueprint F-D: a parked group retries as a whole, on the run's status and the slot's
    /// budget — never on a candidate's own status, which is `done`.
    #[test]
    fn a_group_retry_needs_a_parked_run_an_unselected_slot_and_budget() {
        let (run, mut steps, mut phase) = parked_group();
        let slot: Vec<&RunStep> = steps.iter().collect();
        assert!(retry_group_enabled(&run, &slot, &phase).is_ok());

        phase.retry_limit = 0;
        let refused =
            retry_group_enabled(&run, &slot, &phase).expect_err("attempt 2 is out of budget");
        assert!(
            matches!(
                refused,
                EngineError::RetryExhausted {
                    attempt: 1,
                    retry_limit: 0,
                    ..
                }
            ),
            "{refused}"
        );
        phase.retry_limit = 1;

        steps[1].selected = Some(true);
        let slot: Vec<&RunStep> = steps.iter().collect();
        assert!(
            matches!(
                retry_group_enabled(&run, &slot, &phase),
                Err(EngineError::AlreadySelected { selected, .. }) if selected == ids::STEP_R3_RESEARCH_B
            ),
            "a selected slot has a winner and is not retried as a group"
        );
    }

    /// Plan D45 against §4.3's own table: the three statuses `cancelled` is reachable from are
    /// the three the guard admits, and no other.
    #[test]
    fn cancel_is_enabled_exactly_where_the_run_table_allows_it() {
        let mut run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");

        for status in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::AwaitingApproval,
        ] {
            run.status = status;
            assert!(
                status.can_move_to(RunStatus::Cancelled),
                "`{status}` reaches `cancelled` in §4.3's table"
            );
            assert!(
                super::cancel_enabled(&run).is_ok(),
                "`{status}` is cancellable"
            );
        }

        for status in [RunStatus::Done, RunStatus::Failed, RunStatus::Cancelled] {
            run.status = status;
            let refused =
                super::cancel_enabled(&run).expect_err("a finished run is not cancellable");
            assert!(
                matches!(
                    &refused,
                    EngineError::RunStatus {
                        status: got,
                        expected: "queued | running | awaiting_approval",
                        ..
                    } if *got == status
                ),
                "{refused}"
            );
        }
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
                "item {} is blocked; `Unblock` (`u`) clears it (ANA-2 §4.3)",
                ids::HTUI_FEAT_3
            )
        );
        assert_eq!(
            EngineError::ClaimRefused {
                run: ids::RUN_2,
                claim: Claim::SlotFull {
                    running: 2,
                    limit: 2,
                },
            }
            .to_string(),
            "claim refused: box full (2 of 2 running)"
        );
        assert_eq!(
            EngineError::ClaimRefused {
                run: ids::RUN_2,
                claim: Claim::Overlaps {
                    with: ids::RUN_1,
                    rule: OverlapRule::Paths,
                },
            }
            .to_string(),
            format!("claim refused: overlaps run {} (paths)", ids::RUN_1)
        );
        assert_eq!(
            EngineError::LeaseLost { run: ids::RUN_2 }.to_string(),
            format!(
                "run {}: its lease was taken by another orchestrator; this walk was abandoned \
                 (ANA-2 §4.9)",
                ids::RUN_2
            )
        );
        assert_eq!(
            EngineError::LeaseHeld { run: ids::RUN_2 }.to_string(),
            format!(
                "run {}: another orchestrator holds a live lease (ANA-2 §4.9)",
                ids::RUN_2
            )
        );
        assert_eq!(
            super::stale_step(
                ids::RUN_2,
                ids::STEP_R3_RESEARCH_A,
                StepStatus::Running,
                StepStatus::Done
            )
            .to_string(),
            format!(
                "run {}: step {} was not `running` when this walk moved it to `done`; another \
                 writer moved it first, so the walk stopped (ANA-2 §4.9)",
                ids::RUN_2,
                ids::STEP_R3_RESEARCH_A
            )
        );
        assert_eq!(
            super::stale_run(ids::RUN_2, RunStatus::Running, RunStatus::AwaitingApproval)
                .to_string(),
            format!(
                "run {}: the run was not `running` when this walk moved it to \
                 `awaiting_approval`; another writer moved it first, so the walk stopped \
                 (ANA-2 §4.9)",
                ids::RUN_2
            )
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

    /// Milestone 6's refusals, byte for byte: each is a sentence the Runs tab greys a key with.
    #[test]
    fn the_milestone_6_refusals_say_what_went_wrong() {
        let (item, step, run) = (ids::HTUI_FEAT_3, ids::STEP_R2_PRD, ids::RUN_2);
        let cases = [
            (
                EngineError::NotBlocked {
                    item,
                    status: Status::Open,
                    why: nothing_is_blocked(),
                },
                format!("item {item} is `open`; nothing is blocked"),
            ),
            (
                EngineError::NotBlocked {
                    item,
                    status: Status::Blocked,
                    why: run_is_walking(run, RunStatus::Running),
                },
                format!("item {item} is `blocked`; run {run} is `running`"),
            ),
            (
                EngineError::NotBlocked {
                    item,
                    status: Status::AwaitingApproval,
                    why: run_waits_at_a_gate(run),
                },
                format!(
                    "item {item} is `awaiting_approval`; run {run} is parked at a gate; answer it"
                ),
            ),
            (
                EngineError::NotPromoted { step },
                format!(
                    "step {step} was not promoted to chat; accept artifact needs a promoted step \
                     (ANA-2 §4.8)"
                ),
            ),
            (
                EngineError::PromoteCandidate { step },
                format!(
                    "step {step} is a fan-out candidate; select a winner instead of promoting one"
                ),
            ),
            (
                EngineError::ChatLive { step: None },
                "end the open chat first (Chat tab, Esc Esc)".to_owned(),
            ),
            (
                EngineError::ChatLive { step: Some(step) },
                format!(
                    "step {step} is being chatted with; end that chat first (Chat tab, Esc Esc)"
                ),
            ),
            (
                EngineError::AcceptVerifyFailed {
                    step,
                    exit_code: Some(101),
                },
                format!(
                    "step {step}: the verify command failed (exit 101); fix it in the chat, then \
                     accept again"
                ),
            ),
            (
                EngineError::AcceptVerifyFailed {
                    step,
                    exit_code: None,
                },
                format!(
                    "step {step}: the verify command failed (exit none); fix it in the chat, then \
                     accept again"
                ),
            ),
            (
                EngineError::NotClosable {
                    item,
                    status: Status::Open,
                },
                format!(
                    "item {item} is `open`; close-out needs `done`, `failed` or `blocked` (ANA-2 \
                     §4.10)"
                ),
            ),
            (
                EngineError::NotTerminal {
                    run,
                    status: RunStatus::Running,
                },
                format!("run {run} is `running`; a cleanup retry is for a finished run (R-25)"),
            ),
        ];
        for (refusal, bytes) in cases {
            assert_eq!(refusal.to_string(), bytes);
        }
    }

    /// `RUN_1` parked: the run row every milestone-6 guard reads.
    fn parked_run() -> Run {
        let mut run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        run.status = RunStatus::AwaitingApproval;
        run
    }

    /// `FEAT-3` at `status`.
    fn item_at(status: Status) -> Item {
        let mut item = demo_data()
            .items
            .into_iter()
            .find(|row| row.id == ids::HTUI_FEAT_3)
            .expect("the fixture holds FEAT-3");
        item.status = status;
        item
    }

    /// Blueprint D184: `retry_admitted` is `retry_step`'s whole order — the run, the item, the
    /// group route before `retry_enabled`, and `retry_enabled` last.
    #[test]
    fn retry_admitted_is_retry_step_s_order() {
        let (mut step, mut phase) = review();
        let mut run = parked_run();
        step.status = StepStatus::AwaitingApproval;
        step.attempt = 1;
        phase.retry_limit = 1;
        let steps = vec![step.clone()];
        assert!(retry_admitted(&run, Status::AwaitingApproval, &steps, &step, &phase).is_ok());

        let refused = retry_admitted(&run, Status::Blocked, &steps, &step, &phase)
            .expect_err("a blocked item is not walked on");
        assert!(
            matches!(refused, EngineError::ItemBlocked { item } if Some(item) == run.item_id),
            "{refused}"
        );

        run.status = RunStatus::Failed;
        let refused = retry_admitted(&run, Status::Blocked, &steps, &step, &phase)
            .expect_err("a finished run has nothing to retry onto");
        assert!(
            matches!(
                refused,
                EngineError::RunStatus {
                    status: RunStatus::Failed,
                    expected: "running | awaiting_approval",
                    ..
                }
            ),
            "the run is read before the item: {refused}"
        );
        run.status = RunStatus::AwaitingApproval;

        // A `done` member of a fanned-out slot is admitted: the group route comes before
        // `retry_enabled`, which refuses `done` (plan D65, blueprint F-D).
        phase.fan_out = 2;
        step.status = StepStatus::Done;
        let group = vec![step.clone()];
        assert!(retry_admitted(&run, Status::AwaitingApproval, &group, &step, &phase).is_ok());
        let mut later = step.clone();
        later.id = StepId::new();
        later.attempt = 2;
        let group = vec![step.clone(), later.clone()];
        let refused = retry_admitted(&run, Status::AwaitingApproval, &group, &step, &phase)
            .expect_err("attempt 2 replaced attempt 1");
        assert!(
            matches!(
                refused,
                EngineError::StaleSlot {
                    attempt: 1,
                    latest: 2,
                    ..
                }
            ),
            "{refused}"
        );
        let refused = retry_admitted(&run, Status::AwaitingApproval, &group, &later, &phase)
            .expect_err("a third attempt is out of budget");
        assert!(
            matches!(refused, EngineError::RetryExhausted { attempt: 2, .. }),
            "{refused}"
        );

        phase.fan_out = 1;
        let refused = retry_admitted(&run, Status::AwaitingApproval, &steps, &step, &phase)
            .expect_err("a single `done` step is not retryable");
        assert!(matches!(refused, EngineError::NotGated { .. }), "{refused}");
    }

    /// MOD-4 plan D163 (`docs/ANA-2.md:1563`): a live step of a live run, an item that is not
    /// blocked, no fan-out candidate, no open chat — each refusal naming its clause.
    #[test]
    fn promote_is_enabled_on_a_live_step_of_a_live_run() {
        let (mut step, phase) = review();
        let mut run = parked_run();
        let steps = [step.clone()];
        for status in [
            StepStatus::Running,
            StepStatus::AwaitingApproval,
            StepStatus::Failed,
        ] {
            step.status = status;
            assert!(
                promote_enabled(&run, Status::AwaitingApproval, &steps, &step, &phase, false)
                    .is_ok(),
                "`{status}` is promotable"
            );
        }
        for status in [
            StepStatus::Pending,
            StepStatus::Done,
            StepStatus::Superseded,
            StepStatus::Cancelled,
        ] {
            step.status = status;
            let refused =
                promote_enabled(&run, Status::AwaitingApproval, &steps, &step, &phase, false)
                    .expect_err("only a live step is promotable");
            assert!(
                matches!(
                    refused,
                    EngineError::NotGated {
                        expected: "running | awaiting_approval | failed",
                        ..
                    }
                ),
                "`{status}`: {refused}"
            );
        }

        for status in [RunStatus::Done, RunStatus::Failed, RunStatus::Cancelled] {
            run.status = status;
            let refused =
                promote_enabled(&run, Status::AwaitingApproval, &steps, &step, &phase, false)
                    .expect_err("a finished run has nothing to chat in");
            assert!(
                matches!(refused, EngineError::RunStatus { status: got, .. } if got == status),
                "the run is read first: {refused}"
            );
        }
        run.status = RunStatus::AwaitingApproval;
        step.status = StepStatus::Failed;

        // Plan D134: a `failed` attempt a retry replaced stays `failed`, and is not promotable.
        let mut later = step.clone();
        later.id = StepId::new();
        later.attempt = step.attempt + 1;
        let replaced = [step.clone(), later];
        let refused = promote_enabled(&run, Status::Blocked, &replaced, &step, &phase, false)
            .expect_err("a later attempt replaced it");
        assert!(
            matches!(refused, EngineError::StaleSlot { step: id, latest, .. } if id == step.id && latest == step.attempt + 1),
            "the slot is read before the item: {refused}"
        );

        let refused = promote_enabled(&run, Status::Blocked, &steps, &step, &phase, false)
            .expect_err("an escalated item follows its run first");
        assert!(
            matches!(refused, EngineError::ItemBlocked { .. }),
            "{refused}"
        );

        let mut fanned = phase.clone();
        fanned.fan_out = 3;
        let refused = promote_enabled(
            &run,
            Status::AwaitingApproval,
            &steps,
            &step,
            &fanned,
            false,
        )
        .expect_err("a candidate is selected, not promoted");
        assert!(
            matches!(refused, EngineError::PromoteCandidate { step: id } if id == step.id),
            "{refused}"
        );

        let refused = promote_enabled(&run, Status::AwaitingApproval, &steps, &step, &phase, true)
            .expect_err("the Chat tab hosts one session");
        assert!(
            matches!(refused, EngineError::ChatLive { step: None }),
            "{refused}"
        );
    }

    /// MOD-4 plan D166: accept needs a parked, promoted step with its document and no live chat.
    #[test]
    fn accept_needs_the_promotion_the_document_and_no_live_chat() {
        let (mut step, phase) = review();
        step.status = StepStatus::AwaitingApproval;
        step.promoted_at = chrono::DateTime::from_timestamp(1_788_393_600, 0);
        let steps = [step.clone()];
        assert!(accept_enabled(&steps, &step, &phase, true, false).is_ok());

        let refused = accept_enabled(&steps, &step, &phase, false, false).expect_err("no document");
        assert!(
            matches!(&refused, EngineError::MissingOutputForApproval { kind, .. } if *kind == phase.output_kind),
            "{refused}"
        );
        let refused = accept_enabled(&steps, &step, &phase, true, true).expect_err("a live chat");
        assert!(
            matches!(refused, EngineError::ChatLive { step: Some(id) } if id == step.id),
            "{refused}"
        );

        // Plan D134, the second line of defence: a replaced attempt is never accepted.
        let mut later = step.clone();
        later.id = StepId::new();
        later.attempt = step.attempt + 1;
        let replaced = [step.clone(), later];
        let refused =
            accept_enabled(&replaced, &step, &phase, true, false).expect_err("a later attempt");
        assert!(
            matches!(refused, EngineError::StaleSlot { step: id, .. } if id == step.id),
            "{refused}"
        );

        step.promoted_at = None;
        let refused =
            accept_enabled(&steps, &step, &phase, true, false).expect_err("never promoted");
        assert!(
            matches!(refused, EngineError::NotPromoted { step: id } if id == step.id),
            "{refused}"
        );

        step.status = StepStatus::Done;
        let refused = accept_enabled(&steps, &step, &phase, true, false).expect_err("not parked");
        assert!(
            matches!(
                refused,
                EngineError::NotGated {
                    expected: "awaiting_approval",
                    ..
                }
            ),
            "{refused}"
        );
    }

    /// MOD-4 plan D161's three cases in order, and the sentence naming what holds the item
    /// otherwise.
    #[test]
    fn unblock_names_its_three_cases_and_what_holds_the_item() {
        let parked = parked_run();
        let mut finished = parked.clone();
        finished.status = RunStatus::Failed;
        let mut running = parked.clone();
        running.status = RunStatus::Running;
        let gate = Cursor::Rest {
            step: ids::STEP_R2_PRD,
            status: StepStatus::AwaitingApproval,
        };
        let crashed = Cursor::Create {
            position: 1,
            attempt: 1,
        };

        let blocked = item_at(Status::Blocked);
        assert!(matches!(
            unblock_enabled(&blocked, &[]),
            Ok(UnblockCase::Reopen)
        ));
        assert!(
            matches!(
                unblock_enabled(&blocked, &[(finished.clone(), gate.clone())]),
                Ok(UnblockCase::Reopen)
            ),
            "a finished run is not active"
        );
        assert!(matches!(
            unblock_enabled(&blocked, &[(finished, gate.clone()), (parked.clone(), gate.clone())]),
            Ok(UnblockCase::FollowRun(run)) if run == parked.id
        ));
        let refused = unblock_enabled(&blocked, &[(running.clone(), crashed.clone())])
            .expect_err("a walking run holds the item");
        assert_eq!(
            refused.to_string(),
            format!(
                "item {} is `blocked`; run {} is `running`",
                blocked.id, running.id
            )
        );

        let waiting = item_at(Status::AwaitingApproval);
        assert!(matches!(
            unblock_enabled(&waiting, &[(parked.clone(), crashed)]),
            Ok(UnblockCase::Resume(run)) if run == parked.id
        ));
        let refused = unblock_enabled(&waiting, &[(parked.clone(), gate)])
            .expect_err("a real gate is answered, not unblocked");
        assert_eq!(
            refused.to_string(),
            format!(
                "item {} is `awaiting_approval`; run {} is parked at a gate; answer it",
                waiting.id, parked.id
            )
        );

        for status in [Status::Open, Status::Done, Status::AwaitingApproval] {
            let refused =
                unblock_enabled(&item_at(status), &[]).expect_err("nothing holds the item");
            assert!(
                matches!(&refused, EngineError::NotBlocked { status: got, why, .. } if *got == status && *why == nothing_is_blocked()),
                "{refused}"
            );
        }
    }

    /// MOD-4 plan D167: close-out is refused while a run of the item is active, then for an item
    /// §4.10 does not close; an item it closes answers the resolution the Runs pane closes it as
    /// (MOD-38 plan D6: `done` -> `done`, `failed` and `blocked` -> `withdrawn`).
    #[test]
    fn close_out_needs_no_live_run_and_a_closable_item() {
        let parked = parked_run();
        let mut finished = parked.clone();
        finished.status = RunStatus::Done;
        for (status, resolution) in [
            (Status::Done, Resolution::Done),
            (Status::Failed, Resolution::Withdrawn),
            (Status::Blocked, Resolution::Withdrawn),
        ] {
            assert_eq!(
                close_out_enabled(&item_at(status), std::slice::from_ref(&finished)).ok(),
                Some(resolution),
                "`{status}` closes as `{resolution}`"
            );
            assert_eq!(Resolution::default_for(status), Some(resolution));
        }
        let refused =
            close_out_enabled(&item_at(Status::Open), &[finished.clone(), parked.clone()])
                .expect_err("a live run");
        assert!(
            matches!(
                refused,
                EngineError::RunStatus {
                    run,
                    status: RunStatus::AwaitingApproval,
                    expected: "done | failed | cancelled",
                } if run == parked.id
            ),
            "the live run is named before the item: {refused}"
        );
        for status in [
            Status::Open,
            Status::Queued,
            Status::InProgress,
            Status::AwaitingApproval,
            Status::Closed,
        ] {
            let refused =
                close_out_enabled(&item_at(status), &[]).expect_err("not a finished item");
            assert!(
                matches!(refused, EngineError::NotClosable { status: got, .. } if got == status),
                "{refused}"
            );
        }
    }

    /// MOD-4 plan D177: the manual cleanup retry is for exactly the three terminal statuses.
    #[test]
    fn cleanup_is_for_a_finished_run() {
        let mut run = parked_run();
        for status in [RunStatus::Done, RunStatus::Failed, RunStatus::Cancelled] {
            run.status = status;
            assert!(cleanup_enabled(&run).is_ok(), "`{status}` is finished");
        }
        for status in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::AwaitingApproval,
        ] {
            run.status = status;
            assert!(
                matches!(cleanup_enabled(&run), Err(EngineError::NotTerminal { status: got, .. }) if got == status),
                "`{status}` is live"
            );
        }
    }

    /// Blueprint D184: `start_enabled` is `create_run`'s item law, row for row.
    #[test]
    fn start_enabled_is_the_law() {
        let mut item = item_at(Status::Open);
        for &status in Status::ALL {
            item.status = status;
            let admitted = start_enabled(&item);
            assert_eq!(
                admitted.is_ok(),
                status.can_move_to(Status::Queued),
                "`{status}`"
            );
            if let Err(refused) = admitted {
                assert!(
                    matches!(refused, EngineError::Store(StoreError::Constraint(_))),
                    "legal_move's own refusal: {refused}"
                );
            }
        }
    }
}
