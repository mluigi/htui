//! The six-stage walk of ANA-2 §4.2, in its fixed order, plus the [`AgentSelector`] and
//! [`SessionSink`] seams (plan D6, D13) and the stage-1 capability interlock
//! (`docs/ANA-2.md:475-483`).
//!
//! Two rules the whole file is written against. The engine **holds no state across a call** and
//! re-derives position, attempt and completion from the store on every one (plan D16) — milestone
//! 5's recovery sweep is only correct if this milestone never introduced any, and it is much
//! cheaper to not add the state than to remove it later. And it **never assumes a `run` row passed
//! through `can_move_to`** (plan D17): MOD-2's chat path inserts `run` and `run_step` rows outside
//! §4.3 on purpose, so every status read from a row is matched exhaustively and an unexpected one
//! parks rather than panicking.
//!
//! The engine also never writes a `document` (plan D13). Stage 5 requires one and validation
//! criterion 1 asserts four, but `document_write` is MOD-11's and MOD-11 is blocked on MOD-4 — so
//! the producer sits behind [`SessionSink`], which production answers with [`NoSink`] and a test
//! harness answers with a scripted document. Nothing here will have to be removed when MOD-11
//! lands.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};
use htui_agent::driver::{AgentDriver, DriverCaps, PermissionPolicy, SessionSpec, ToolExposure};
use htui_agent::event::DoneEvent;
use htui_agent::record::{Recorder, RunCap, pump};
use htui_agent::registry::caps_for;
use htui_core::model::{
    BoxId, BoxProfile, Document, Gate, GateOutcome, GraphSnapshot, Item, ItemId, NewNote, NewRun,
    NewRunStep, NoteId, Project, ProjectSettings, PromptScope, Run, RunId, RunStatus, RunStep,
    RunStepCommit, SnapshotCandidate, SnapshotPhase, Status, StepId, StepOutcome, StepStatus,
    TIMESTAMPTZ_DIGITS, UserId,
};
use htui_core::prompt::excerpt::{BUILTIN_ID, ExcerptAudit, ExcerptSet};
use htui_core::prompt::{
    AssembledPrompt, InputDocument, PromptSpec, TemplateRef, TemplateRole, TokenEstimator,
    assemble, settings,
};
use htui_core::scrub::Scrubber;
use htui_core::store::{StoreError, WriteStore};
use serde_json::Value;
use uuid::Uuid;

use crate::command::{Command, CommandOutcome, EngineError, GateAnswer, Rest};
use crate::gate::{self, GateContext, Landing, LoopOutcome, SettleInput};
use crate::graph::{self, GraphSource, ResolveError};
use crate::isolate::{Clock, Isolator, Prepared};
use crate::status::{Cursor, RunFailure, cursor, latest_at, next_attempt};

/// The only `run_step.fanout_index` milestone 2 walks; fan-out's others are milestone 4's.
const WALKED_FANOUT_INDEX: i32 = 0;

/// The phase whose output document carries ANA-2 §4.4's front-matter verdict (`:440`).
///
/// A name rather than a flag on the row, because that is what ANA-2 has: `:440` says "a
/// `review`-phase output document", and all four seeded graphs that have one call it `review`
/// (`crates/htui-core/src/seed.rs:88-160`). The `bug` graph's implement-shaped phase is `fix` and
/// its review phase is still `review`, which is the case that shows the name is the rule.
const REVIEW_PHASE: &str = "review";

/// How long a claim's lease runs before ANA-2 §4.9's sweep may adopt the run (`:1408`).
const LEASE_SECONDS: i64 = 120;

/// A ceiling on iterations of one [`Engine::run_to_rest`] call.
///
/// Every iteration either creates a step, runs one, or stops, and both the retry budget and the
/// review loop's are finite — so this is a net and not a bound. It exists because the walk is a
/// loop over rows the engine does not own: a row moved under it by another process (plan D17) must
/// end in a refusal a human can read, never in a spin.
const MAX_ITERATIONS: usize = 512;

// ---------------------------------------------------------------------------------------------
// Seams
// ---------------------------------------------------------------------------------------------

/// Stage 1's third duty (plan D6): which surviving candidate runs.
///
/// Fan-out picks `fan_out` of them and a judge compares the results; that is milestone 4's, and
/// this seam is what keeps `select.rs` landable without re-cutting the walk. The interlock that
/// decides *eligibility* is the engine's and is not behind this trait — a selector that could
/// re-admit a CLI-only agent to a gated phase would be a selector that can break `R-ORCH`'s
/// refusal (`docs/ANA-4.md:554`).
pub trait AgentSelector: Send + Sync {
    /// The candidate to run, or `None` to refuse the phase.
    ///
    /// `eligible` has already survived the capability interlock and is in `phase_agent.position`
    /// order, so "the first one" is "the highest-preference one".
    fn select<'c>(
        &self,
        phase: &SnapshotPhase,
        eligible: &'c [SnapshotCandidate],
    ) -> Option<&'c SnapshotCandidate>;
}

/// The only selector this milestone: the first candidate that survived stage 1.
#[derive(Debug, Default, Clone, Copy)]
pub struct FirstCandidate;

impl AgentSelector for FirstCandidate {
    fn select<'c>(
        &self,
        _phase: &SnapshotPhase,
        eligible: &'c [SnapshotCandidate],
    ) -> Option<&'c SnapshotCandidate> {
        eligible.first()
    }
}

/// Called once between stage 4 and stage 5, after the session's `done` (plan D13).
///
/// **The blueprint's §5.1 signature has no `item_id`**, and `RunStep` carries only `run_id`
/// (`crates/htui-core/src/model/run.rs:216-217`), so a sink drawn that way cannot construct a
/// `NewDocument` — `document.item_id` is not nullable. The engine holds the run at this point and
/// therefore holds the item, so it passes it. That is the one change to the drawn shape, and it is
/// what makes the seam able to do the only job it has.
///
/// Production is [`NoSink`]: MOD-11's `document_write` is the real producer and it calls
/// `WriteStore::write_document` itself, so the engine's sink stays empty forever.
#[allow(
    async_fn_in_trait,
    reason = "no `dyn SessionSink` is formed: the engine is generic over `K` (`store/traits.rs:62`)"
)]
pub trait SessionSink: Sync {
    /// Whatever has to happen between the session ending and the settle reading its artefact.
    ///
    /// # Errors
    /// The store's own refusals, which the walk raises as [`EngineError::Store`].
    async fn after_done(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        done: &DoneEvent,
    ) -> Result<(), StoreError>;
}

/// The production [`SessionSink`]: MOD-11 writes the document, not the orchestrator (plan D13).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoSink;

impl SessionSink for NoSink {
    async fn after_done(
        &self,
        _item: ItemId,
        _step: &RunStep,
        _phase: &SnapshotPhase,
        _done: &DoneEvent,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}

/// How the walk gets a driver for one session.
///
/// **The blueprint's §5.1 factory is `Fn(&SnapshotCandidate) -> Box<dyn AgentDriver>` and cannot
/// work**: `FakeDriver` plays its script once and refuses a second `start`
/// (`crates/htui-agent/src/fake.rs:178-180`), and a harness that scripts by `(phase name, attempt)`
/// — which is the only way a review loop's second attempt can differ from its first — has no way
/// to answer a factory that is handed neither. Both travel here. Milestone 3's registry ignores
/// them and builds per `(agent, box)` as it already does (`registry.rs:132`).
pub type DriverFor<'a> = &'a (dyn Fn(&SnapshotCandidate, &str, i32) -> Box<dyn AgentDriver> + Sync);

// ---------------------------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------------------------

/// Everything [`Engine::new`] borrows, as a struct literal rather than a builder.
///
/// A builder would let a caller forget one of thirteen fields and find out at run time; a literal
/// cannot compile until every one of them is named.
pub struct EngineParts<'a, S, G, I, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    /// The store. The engine names no concrete one (ANA-2 invariant 10).
    pub store: &'a S,
    /// Resolution's source for the four reads no `ReadStore` method answers (plan D19).
    pub graphs: &'a G,
    /// Stage 2 and stage 5's trees (plan D6).
    pub isolator: &'a I,
    /// Plan D8: the one place an instant enters the walk.
    pub clock: &'a C,
    /// Stage 1's selection.
    pub selector: &'a A,
    /// Plan D13's document producer, or [`NoSink`].
    pub sink: &'a K,
    /// One driver per session (see [`DriverFor`]).
    pub driver: DriverFor<'a>,
    /// `R-SEC-3`'s masker, handed to both the assembler and the recorder so they cannot disagree
    /// about what two identical prompts are (blueprint H-13).
    pub scrubber: &'a dyn Scrubber,
    /// The resolved `app_setting` map (blueprint A-2, H-11): those reads are inherent on both
    /// stores, so they are passed rather than read through a trait.
    pub app: BTreeMap<String, Value>,
    /// ANA-5 §4.2's box projection for the prompt (blueprint H-11: `box_profile` is inherent too).
    pub box_profile: BoxProfile,
    /// `run.target_box_id` and `claim_run`'s box.
    pub box_id: BoxId,
    /// `claim_run`'s liveness token (ANA-2 §4.9).
    pub owner: Uuid,
    /// `run.started_by` and `item_note.created_by`.
    pub user: UserId,
}

/// `Debug` is hand written for one field: a driver factory is a `dyn Fn` and `dyn Fn` is not
/// `Debug`, while `missing_debug_implementations` is a workspace lint. The same shape
/// `SessionSpec` uses (`crates/htui-agent/src/driver.rs:281-298`), for the same reason.
impl<S, G, I, C, A, K> core::fmt::Debug for EngineParts<'_, S, G, I, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EngineParts")
            .field("isolator", &self.isolator)
            .field("scrubber", &self.scrubber)
            .field("app", &self.app)
            .field("box_profile", &self.box_profile)
            .field("box_id", &self.box_id)
            .field("owner", &self.owner)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// ANA-2 §4.2's walk, generic over everything it touches and holding nothing between calls.
#[derive(Debug)]
pub struct Engine<'a, S, G, I, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    parts: EngineParts<'a, S, G, I, C, A, K>,
}

/// What [`Engine::resume`] found (ANA-2 §12 criterion 3, `docs/ANA-2.md:2090`).
///
/// **Not in the blueprint**, which names `Engine::resume` in §6.3 and gives it no return type. A
/// bare `Rest` cannot say *why* a walk did not advance, and "the graph moved under a parked run"
/// is exactly the fact criterion 3 is about, so it is a value rather than a note the caller has to
/// go looking for. The note is still written — it is what an operator reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// The live graph still hashes to the run's own `topology`; the walk ran.
    Walked(Rest),
    /// The live graph moved. **Nothing was advanced**: invariant 2 says a run walks its snapshot,
    /// and §4.9's resume says a mismatch is a human's decision, not the engine's.
    TopologyChanged {
        /// `run.graph_snapshot.topology`, the digest this run walks.
        snapshot: String,
        /// What the live graph hashes to now.
        live: String,
        /// Where the run is, unchanged.
        rest: Rest,
    },
}

impl<'a, S, G, I, C, A, K> Engine<'a, S, G, I, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    /// The walk over one set of borrowed parts.
    #[must_use]
    pub const fn new(parts: EngineParts<'a, S, G, I, C, A, K>) -> Self {
        Self { parts }
    }

    // -- commands (ANA-2 §6.2, blueprint §5.2) ------------------------------------------------

    /// Answers one of ANA-2 §6.2's three verbs and walks the run to its next rest.
    ///
    /// # Errors
    /// Every [`EngineError`]: the enabling guards of [`crate::command`], the store's refusals, the
    /// resolver's, the assembler's, the recorder's and the driver's.
    pub async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError> {
        match command {
            Command::StartRun {
                item,
                mode,
                repo_scope,
            } => self.start_run(item, mode, repo_scope).await,
            Command::AnswerGate { run, step, answer } => self.answer_gate(run, step, answer).await,
            Command::RetryStep { run, step } => self.retry_step(run, step).await,
        }
    }

    /// §6.2's `run`/`queue`: resolve, create, claim, walk.
    async fn start_run(
        &self,
        item: ItemId,
        mode: htui_core::model::RunMode,
        repo_scope: Option<Vec<htui_core::model::RepoId>>,
    ) -> Result<CommandOutcome, EngineError> {
        let item = self.item(item).await?;
        let resolved = graph::resolve(
            self.parts.store,
            self.parts.graphs,
            &item,
            mode,
            &self.parts.app,
            repo_scope.as_deref(),
        )
        .await?;

        let now = self.now();
        let id = RunId::new();
        // `create_run` moves the item `open | failed -> queued` inside its own transaction, so an
        // item at any other status is refused here — `queued`, `in_progress` and `awaiting_approval`
        // all mean some other run already owns it (`crates/htui-core/src/model/item.rs:46-60`).
        self.parts
            .store
            .create_run(NewRun {
                id,
                project_id: item.project_id,
                item_id: item.id,
                mode,
                target_box_id: self.parts.box_id,
                started_by: self.parts.user,
                graph_snapshot: resolved.snapshot,
                repo_scope: resolved.repo_scope,
                queued_at: now,
            })
            .await?;

        let lease = now + TimeDelta::try_seconds(LEASE_SECONDS).unwrap_or(TimeDelta::zero());
        // `Ok(false)` is ANA-2 §4.7's admission refusing: the box is at `max_concurrent_items` or
        // the scope overlaps a live run. Nothing is written, the run stays `queued`, and the
        // predicate itself stays milestone 1's (plan D6, R-2 untouched).
        if !self
            .parts
            .store
            .claim_run(id, self.parts.box_id, self.parts.owner, now, lease)
            .await?
        {
            return Err(EngineError::ClaimRefused { run: id });
        }

        let rest = self.run_to_rest(id).await?;
        Ok(CommandOutcome::Started { run: id, rest })
    }

    /// §6.2's `approve` / `reject with note` / `accept artifact`.
    async fn answer_gate(
        &self,
        run: RunId,
        step: StepId,
        answer: GateAnswer,
    ) -> Result<CommandOutcome, EngineError> {
        let run = self.run(run).await?;
        let snapshot = Self::snapshot_of(&run)?;
        let step = self.step(run.id, step).await?;
        let phase = Self::phase_at(&snapshot, step.position)?;
        let item = Self::item_of(&run)?;

        let has_output = self.output_of(item, &phase, step.id).await?.is_some();
        crate::command::answer_gate_enabled(&step, &phase, has_output, &answer)?;

        let now = self.now();
        let (outcome, note) = match &answer {
            GateAnswer::Approved => (GateOutcome::Approved, None),
            GateAnswer::Skipped => (GateOutcome::Skipped, None),
            GateAnswer::Rejected { note } => (GateOutcome::Rejected, Some(note.clone())),
        };
        self.parts
            .store
            .answer_gate(step.id, outcome, note.clone(), now)
            .await?;

        // Both answers unpark the run and the item first. For a rejection that is blueprint A-5's
        // order and it is not cosmetic: `awaiting_approval -> blocked` is **illegal** for an item
        // (`model/item.rs:54`), so an escalation that did not pass through `in_progress` first
        // would earn `StoreError::Constraint`.
        self.unpark(&run, now).await?;

        let rest = if let GateAnswer::Rejected { note } = &answer {
            self.after_rejection(&run, &snapshot, &step, &phase, note, now)
                .await?
        } else {
            self.run_to_rest(run.id).await?
        };
        Ok(CommandOutcome::Answered { rest })
    }

    /// What a human's rejection means: the review loop when the phase can loop, the end of the run
    /// when it cannot (ANA-2 `:577-578`, blueprint A-8).
    async fn after_rejection(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        step: &RunStep,
        phase: &SnapshotPhase,
        note: &str,
        now: DateTime<Utc>,
    ) -> Result<Rest, EngineError> {
        let loopable =
            phase.name == REVIEW_PHASE && gate::loop_target(snapshot, step.position).is_some();
        if !loopable {
            self.parts
                .store
                .finish_run(
                    run.id,
                    RunStatus::Failed,
                    Some(
                        &RunFailure::Rejected {
                            phase: phase.name.clone(),
                        }
                        .to_string(),
                    ),
                    now,
                )
                .await?;
            return Ok(Rest {
                run: RunStatus::Failed,
                position: Some(step.position),
                failure: Some(RunFailure::Rejected {
                    phase: phase.name.clone(),
                }),
            });
        }

        let run = self.run(run.id).await?;
        let ctx = self.gate_context(&run, snapshot);
        match gate::review_loop(&ctx, step, note).await? {
            LoopOutcome::Resumed { .. } => self.run_to_rest(run.id).await,
            LoopOutcome::Escalated { attempts, .. } => Ok(Rest {
                run: RunStatus::AwaitingApproval,
                position: Some(step.position),
                failure: Some(RunFailure::ReviewLoopExhausted(attempts)),
            }),
            LoopOutcome::NoTarget => Ok(Rest {
                run: RunStatus::Failed,
                position: Some(step.position),
                failure: Some(RunFailure::NoLoopTarget),
            }),
        }
    }

    /// §6.2's `retry`: supersede the answered step (or leave the failed one) and admit the next
    /// attempt.
    async fn retry_step(&self, run: RunId, step: StepId) -> Result<CommandOutcome, EngineError> {
        let run = self.run(run).await?;
        let snapshot = Self::snapshot_of(&run)?;
        let row = self.step(run.id, step).await?;
        let phase = Self::phase_at(&snapshot, row.position)?;
        let item = self.item(Self::item_of(&run)?).await?;

        // Blueprint F-J / R-4: `Status::can_move_to` has no `blocked -> in_progress` edge, so a
        // blocked item cannot be resumed until milestone 6 ships `Unblock`.
        if item.status == Status::Blocked {
            return Err(EngineError::ItemBlocked { item: item.id });
        }
        crate::command::retry_enabled(&row, &phase)?;

        let now = self.now();
        match row.status {
            // `retried -> superseded` is `answer_gate`'s own arm (`store/traits.rs:786-791`).
            StepStatus::AwaitingApproval => {
                self.parts
                    .store
                    .answer_gate(row.id, GateOutcome::Retried, None, now)
                    .await?;
            }
            // A `failed` step is left alone: `failed` reaches only `awaiting_approval` and
            // `cancelled` (`model/run.rs:126`), so superseding it is a `Constraint` (plan D5).
            StepStatus::Failed => {}
            _ => {
                return Err(EngineError::NotGated {
                    step: row.id,
                    status: row.status,
                    expected: "awaiting_approval | failed",
                });
            }
        }
        self.unpark(&run, now).await?;

        // Admitted rather than created outright, so the next attempt passes through the capability
        // interlock like any other — a phase whose only candidate lost its inline-approval
        // capability since the last attempt must not be retried onto it.
        let run = self.run(run.id).await?;
        let steps = self.parts.store.run_steps(run.id).await?;
        let attempt = next_attempt(&steps, row.position);
        if let Some(rest) = self.admit(&run, &snapshot, &phase, attempt).await? {
            return Ok(CommandOutcome::Retried { step: row.id, rest });
        }
        let rest = self.run_to_rest(run.id).await?;
        Ok(CommandOutcome::Retried { step: row.id, rest })
    }

    // -- the walk ------------------------------------------------------------------------------

    /// Stages 1–6 per position until the run rests: parked, terminal, or refused.
    ///
    /// Every iteration re-reads the run, its snapshot and its steps (plan D16). That is more store
    /// reads than a cached walk would make and it is the point: there is no cursor to be stale, so
    /// a row another process moved is seen on the next pass rather than overwritten.
    ///
    /// # Errors
    /// Every [`EngineError`].
    pub async fn run_to_rest(&self, run: RunId) -> Result<Rest, EngineError> {
        for _ in 0..MAX_ITERATIONS {
            let row = self.run(run).await?;
            match row.status {
                RunStatus::Running => {}
                // A `queued` run has not been claimed, and claiming is `claim_run`'s (plan D6).
                RunStatus::Queued => {
                    return Err(EngineError::RunStatus {
                        run,
                        status: row.status,
                        expected: "running",
                    });
                }
                RunStatus::AwaitingApproval
                | RunStatus::Done
                | RunStatus::Failed
                | RunStatus::Cancelled => {
                    return self.resting(&row).await;
                }
            }

            let snapshot = Self::snapshot_of(&row)?;
            let steps = self.parts.store.run_steps(run).await?;
            match cursor(&snapshot, &steps) {
                Cursor::Finished => {
                    let now = self.now();
                    self.parts
                        .store
                        .finish_run(run, RunStatus::Done, None, now)
                        .await?;
                    return Ok(Rest {
                        run: RunStatus::Done,
                        position: None,
                        failure: None,
                    });
                }
                Cursor::Rest { .. } => return self.resting(&row).await,
                Cursor::Create { position, attempt } => {
                    let phase = Self::phase_at(&snapshot, position)?;
                    if let Some(rest) = self.admit(&row, &snapshot, &phase, attempt).await? {
                        return Ok(rest);
                    }
                }
                Cursor::Run(step) => {
                    let Some(step) = steps.into_iter().find(|row| row.id == step) else {
                        continue;
                    };
                    let phase = Self::phase_at(&snapshot, step.position)?;
                    if let Some(rest) = self.walk_step(&row, &snapshot, step, &phase).await? {
                        return Ok(rest);
                    }
                }
            }
        }
        Err(EngineError::Store(StoreError::Constraint(format!(
            "run {run}: the walk made {MAX_ITERATIONS} passes without resting; a row moved under it"
        ))))
    }

    /// ANA-2 §12 criterion 3 (`docs/ANA-2.md:2090`): what §4.9's sweep does before it advances a
    /// run it did not start.
    ///
    /// Re-resolves the item's **live** graph and compares its [`graph::topology`] with the digest
    /// the run's own snapshot carries. On a mismatch the walk is not advanced and an `item_note`
    /// records both digests; invariant 2 means the run *could* keep walking its snapshot, and §4.9
    /// says the divergence is a human's decision rather than the engine's.
    ///
    /// Cut here because criterion 3 needs it; the rest of milestone 5's sweep — `adopt_runs`, the
    /// lease, the unfinished-step rule — is not.
    ///
    /// # Errors
    /// Every [`EngineError`].
    pub async fn resume(&self, run: RunId) -> Result<Resume, EngineError> {
        let row = self.run(run).await?;
        let snapshot = Self::snapshot_of(&row)?;
        let item = self.item(Self::item_of(&row)?).await?;
        let live = graph::resolve(
            self.parts.store,
            self.parts.graphs,
            &item,
            row.mode,
            &self.parts.app,
            Some(&row.repo_scope),
        )
        .await?;

        if live.snapshot.topology != snapshot.topology {
            let now = self.now();
            self.parts
                .store
                .add_note(NewNote {
                    id: NoteId::new(),
                    item_id: item.id,
                    body: format!(
                        "topology mismatch: run {run} walks `{}`, the live graph is `{}`; \
                         the walk is not advanced (ANA-2 §4.9, invariant 2)",
                        snapshot.topology, live.snapshot.topology
                    ),
                    created_by: self.parts.user,
                    box_id: Some(self.parts.box_id),
                    via_step_id: None,
                    created_at: now,
                })
                .await?;
            return Ok(Resume::TopologyChanged {
                snapshot: snapshot.topology,
                live: live.snapshot.topology,
                rest: self.resting(&row).await?,
            });
        }
        Ok(Resume::Walked(self.run_to_rest(run).await?))
    }

    // -- stage 1 -------------------------------------------------------------------------------

    /// **Stage 1 — admit.** The capability interlock, the selection, and the step row.
    ///
    /// `Some(rest)` is the refusal: no candidate survives the inline-approval interlock, the item
    /// goes to `blocked` **before** the run is failed (blueprint H-16 — the reverse order leaves
    /// the item `failed`, because `finish_run` mirrors `in_progress -> failed`), and an `item_note`
    /// records the reason. `None` means a `pending` step exists and the walk may run it.
    async fn admit(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        attempt: i32,
    ) -> Result<Option<Rest>, EngineError> {
        let mut eligible = Vec::with_capacity(phase.candidates.len());
        for candidate in &phase.candidates {
            // ANA-4 `:554`, verbatim: "`R-ORCH` gates that require an inline approval must not be
            // scheduled onto a CLI-only agent". Read from the **agent row** through
            // `registry::caps_for` (`registry.rs:152`) and never from a built driver — stage 1
            // refuses before a driver exists to ask (plan D6, fact-check F9).
            let Some(agent) = self.parts.graphs.agent(candidate.agent_id).await? else {
                continue;
            };
            let caps: DriverCaps = caps_for(&agent);
            if phase.gate_effective != Gate::Never
                && !(caps.permission_requests || caps.edit_proposals)
            {
                continue;
            }
            eligible.push(candidate.clone());
        }

        let Some(chosen) = self.parts.selector.select(phase, &eligible).cloned() else {
            return self.refuse_capability(run, phase).await.map(Some);
        };

        let _ = snapshot;
        self.parts
            .store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run.id,
                position: phase.position,
                attempt,
                fanout_index: WALKED_FANOUT_INDEX,
                phase_name: phase.name.clone(),
                agent_id: Some(chosen.agent_id),
                model: Some(chosen.model.clone()),
            })
            .await?;
        Ok(None)
    }

    /// `missing_capability: inline_approval` (`docs/ANA-2.md:482`), in blueprint H-16's order.
    async fn refuse_capability(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
    ) -> Result<Rest, EngineError> {
        let now = self.now();
        let failure = RunFailure::MissingCapability;
        if let Some(item) = run.item_id {
            self.parts
                .store
                .transition(item, Status::InProgress, Status::Blocked)
                .await?;
            self.parts
                .store
                .add_note(NewNote {
                    id: NoteId::new(),
                    item_id: item,
                    body: format!("{failure} (phase `{}`)", phase.name),
                    created_by: self.parts.user,
                    box_id: Some(self.parts.box_id),
                    via_step_id: None,
                    created_at: now,
                })
                .await?;
        }
        // The item is already `blocked`, so `finish_run`'s item mirror finds no legal move and
        // leaves it — which is the `_ =>` arm of its own table, and the reason the order matters.
        self.parts
            .store
            .finish_run(run.id, RunStatus::Failed, Some(&failure.to_string()), now)
            .await?;
        Ok(Rest {
            run: RunStatus::Failed,
            position: Some(phase.position),
            failure: Some(failure),
        })
    }

    // -- stages 2 to 6 -------------------------------------------------------------------------

    /// Stages 2 to 6 for one `pending` step. `Some(rest)` stops the walk.
    async fn walk_step(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        step: RunStep,
        phase: &SnapshotPhase,
    ) -> Result<Option<Rest>, EngineError> {
        let item = Self::item_of(run)?;
        let now = self.now();
        // `transition_step` stamps `started_at = COALESCE(started_at, at)`, so this instant *is*
        // the step's start. Read from the row the walk holds, it would still be `NULL` — the row
        // was read before the move — and the step deadline would be measured from the settle.
        let started_at = now;

        // -- stage 2: prepare -----------------------------------------------------------------
        if !self
            .parts
            .store
            .transition_step(step.id, StepStatus::Pending, StepStatus::Running, now)
            .await?
        {
            // Another process moved it; the next pass re-derives from rows (plan D16, D17).
            return Ok(None);
        }
        let prepared = self
            .parts
            .isolator
            .prepare(run.id, step.id, &run.repo_scope, phase.isolation)
            .await?;
        let trees: Vec<_> = prepared
            .trees
            .iter()
            .map(|tree| tree.tree.clone())
            .collect();
        self.parts.store.upsert_step_tree(step.id, &trees).await?;
        let before: Vec<RunStepCommit> = prepared
            .trees
            .iter()
            .map(|tree| RunStepCommit {
                run_step_id: step.id,
                repo_id: tree.tree.repo_id,
                before_hash: tree.before_hash.clone(),
                after_hash: None,
            })
            .collect();
        self.parts.store.record_commits(step.id, &before).await?;

        // -- stage 3: prompt ------------------------------------------------------------------
        let prompt = match self
            .assemble_prompt(run, snapshot, &step, phase, item)
            .await?
        {
            Ok(prompt) => prompt,
            Err(missing) => return self.fail_before_a_token(run, &step, phase, missing).await,
        };
        self.parts
            .store
            .set_step_prompt(
                step.id,
                &prompt.digest,
                &serde_json::to_value(&prompt.trim).unwrap_or(Value::Null),
            )
            .await?;

        // -- stage 4: session -----------------------------------------------------------------
        let (result, cap_breach) = self.session(run, &step, phase, &prompt).await?;
        if let Ok(done) = &result {
            self.parts.sink.after_done(item, &step, phase, done).await?;
        }

        // -- stage 5: settle ------------------------------------------------------------------
        let after = self.parts.isolator.capture(step.id, &trees).await?;
        self.parts.store.record_commits(step.id, &after).await?;
        let output = self.output_of(item, phase, step.id).await?;
        let now = self.now();
        let settled = gate::settle(&SettleInput {
            driver: &result,
            cap_breach,
            started_at,
            now,
            deadline_seconds: phase.deadline_seconds,
            output: output.as_ref(),
            // `verify.rs` is milestone 3's and every seeded phase has `verify_command: None`
            // (`crates/htui-core/src/seed.rs:233`), so this column is never written here.
            verify_outcome: None,
            is_review: phase.name == REVIEW_PHASE,
        });
        self.parts
            .store
            .finish_step(
                step.id,
                StepOutcome {
                    exit_code: None,
                    // `usage` and `trim_record` are `None` because `None` **leaves** the column
                    // (`model/run.rs:406-407`) and both are already written — the recorder summed
                    // the usage, `set_step_prompt` wrote the trim record.
                    usage: None,
                    trim_record: None,
                    verify_outcome: None,
                    verify_exit_code: None,
                    finished_at: now,
                },
            )
            .await?;

        // -- stage 6: gate --------------------------------------------------------------------
        let row = self.run(run.id).await?;
        let ctx = self.gate_context(&row, snapshot);
        match gate::apply(&ctx, &step, phase, settled).await? {
            Landing::Advance => Ok(None),
            Landing::Retry { position, attempt } => {
                let phase = Self::phase_at(snapshot, position)?;
                self.admit(&row, snapshot, &phase, attempt).await
            }
            Landing::Rest(rest) => Ok(Some(rest)),
        }
    }

    /// Stage 3's hard failure: a required input resolved to no document, before a token was spent
    /// (`docs/ANA-2.md:412-414`).
    async fn fail_before_a_token(
        &self,
        run: &Run,
        step: &RunStep,
        phase: &SnapshotPhase,
        kind: String,
    ) -> Result<Option<Rest>, EngineError> {
        let now = self.now();
        let failure = RunFailure::MissingInput(kind);
        self.parts
            .store
            .transition_step(step.id, StepStatus::Running, StepStatus::Failed, now)
            .await?;
        self.parts
            .store
            .finish_run(run.id, RunStatus::Failed, Some(&failure.to_string()), now)
            .await?;
        Ok(Some(Rest {
            run: RunStatus::Failed,
            position: Some(phase.position),
            failure: Some(failure),
        }))
    }

    /// Stage 3: the inputs, the pinned template and `assemble`.
    ///
    /// `Err(kind)` is the missing required input, which the caller turns into the hard failure —
    /// an inner `Result` rather than an [`EngineError`] variant because a missing input is not an
    /// engine fault, it is the run's own outcome.
    async fn assemble_prompt(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        step: &RunStep,
        phase: &SnapshotPhase,
        item: ItemId,
    ) -> Result<Result<AssembledPrompt, String>, EngineError> {
        let row = self.item(item).await?;
        let project = self.project(row.project_id).await?;
        let resolved = self
            .parts
            .store
            .resolve_inputs(item, run.id, &phase.input_kinds)
            .await?;
        let required = required_inputs(phase, &snapshot.phases);
        let mut documents = Vec::with_capacity(resolved.len());
        let mut notes = Vec::new();
        for input in resolved {
            match input.document {
                Some(document) => documents.push(InputDocument {
                    kind: document.kind,
                    version: document.version,
                    body: document.body,
                }),
                None if required.contains(&input.kind) => {
                    return Ok(Err(input.kind));
                }
                None => notes.push(format!(
                    "input `{}` is produced at a later position and has not run yet; \
                     it is optional on this attempt (plan D21)",
                    input.kind
                )),
            }
        }

        let template = self
            .parts
            .graphs
            .prompt_template(
                project.id,
                &phase.template.name,
                Some(phase.template.version),
            )
            .await?
            .ok_or_else(|| ResolveError::NoTemplate {
                phase: phase.name.clone(),
                name: phase.template.name.clone(),
                project: project.id,
            })?;

        let kind = self.item_kind_name(&row).await?;
        let upstream = self
            .parts
            .store
            .upstream_summaries(
                item,
                settings::resolve_hops(Some(&project.settings), &self.parts.app, &mut notes),
                // The engine is headless (`R-NF-3`) and holds no workspace, so the walk is bounded
                // by the item's own project — `PromptScope`'s own no-workspace case.
                &PromptScope::project_only(project.id),
            )
            .await?;
        let (caps, _scan, _deadline) = settings::resolve_excerpt_caps(&self.parts.app);

        let spec = PromptSpec {
            role: TemplateRole::of_name(&template.name),
            template: TemplateRef {
                name: template.name.clone(),
                version: template.version,
            },
            body: template.body.clone(),
            item_key: format!("{}:{}", project.slug, row.key),
            item_title: row.title.clone(),
            item_kind: kind,
            item_body: row.body.clone(),
            phase: phase.name.clone(),
            output_kind: Some(phase.output_kind.clone()),
            attempt: step.attempt,
            documents,
            upstream,
            box_profile: self.parts.box_profile.clone(),
            // `R-SKL-2`'s resolution is inherent on both stores (blueprint H-11) and the Runs tab
            // wires it at milestone 6; an empty list renders no section.
            skills: Vec::new(),
            // §4.5's ranker needs a resolved repo root and `repo_box_path` has no writer the walk
            // can reach, so the audit records the caps a pass *would* have run under and nothing
            // else — the shape `crates/htui/src/preview.rs:274-295` already ships.
            excerpts: ExcerptSet {
                files: Vec::new(),
                audit: ExcerptAudit {
                    provider_set: vec![BUILTIN_ID.to_owned()],
                    roots: Vec::new(),
                    considered: 0,
                    selected: 0,
                    caps,
                    files: Vec::new(),
                },
                notes: Vec::new(),
            },
            command_queue: phase.command_queue != htui_core::model::CommandQueue::Off,
            // Both are milestone 3's: `verify.rs` produces the first and the diff needs a real
            // isolator to have produced an `after_hash` worth rendering.
            verify_failure: None,
            previous_diff: None,
            judge: None,
            handoff: None,
            budget: settings::resolve_budget(
                phase.token_budget,
                Some(&project.settings),
                &self.parts.app,
            ),
            max_skill_tokens: settings::resolve_max_skill_tokens(&self.parts.app),
            estimator: TokenEstimator::DEFAULT,
            notes,
        };
        Ok(Ok(assemble(&spec, self.parts.scrubber)?))
    }

    /// **Stage 4 — session.** One driver, one session, one turn, one recorder.
    ///
    /// A closed stream is a [`htui_agent::error::DriverError`] and not a `done`
    /// (`crates/htui-agent/src/record.rs:1689-1691`), which is what makes settle's first rule
    /// reachable at all.
    async fn session(
        &self,
        run: &Run,
        step: &RunStep,
        phase: &SnapshotPhase,
        prompt: &AssembledPrompt,
    ) -> Result<
        (
            Result<DoneEvent, htui_agent::error::DriverError>,
            Option<htui_agent::record::CapBreach>,
        ),
        EngineError,
    > {
        let project = self.project(run.project_id).await?;
        let settings = Self::project_settings(&project);
        let candidate = Self::candidate_of(step, phase)?;
        let driver = (self.parts.driver)(&candidate, &phase.name, step.attempt);

        let spec = SessionSpec {
            agent_id: candidate.agent_id,
            step_id: step.id,
            cwd: self.cwd_of(run, step).await?,
            extra_dirs: Vec::new(),
            // `R-SEC-2`: ANA-7 resolves secrets and milestone 6 wires them; the walk invents none.
            env: BTreeMap::new(),
            model: Some(candidate.model.clone()),
            tools: ToolExposure::default(),
            mcp: Vec::new(),
            permission: PermissionPolicy::default(),
            retain_raw: settings.keep_raw_events,
            resume: None,
            budget_micros: settings.per_token_cap_run,
        };

        let mut recorder = Recorder::new(
            self.parts.store,
            self.parts.scrubber,
            step.id,
            settings.keep_raw_events,
            None,
        );
        if let Some(micros) = settings.per_token_cap_run {
            recorder = recorder.with_run_cap(RunCap {
                micros,
                // Nothing in this milestone spawns a process, so a grace of zero is the honest
                // figure; milestone 3's worker passes its own (`record.rs:157-159`).
                grace: std::time::Duration::ZERO,
            });
        }
        recorder
            .record_prompt(&prompt.text, prompt.payload_sections_value(), self.now())
            .await?;

        let mut session = driver.start(spec, prompt.text.clone()).await?;
        let result = pump(&mut *session, &mut recorder).await;
        let summary = recorder.finish().await?;
        Ok((result, summary.cap_breach))
    }

    // -- helpers -------------------------------------------------------------------------------

    /// Plan D8: every instant the walk hands a writer comes from here.
    fn now(&self) -> DateTime<Utc> {
        self.parts.clock.now()
    }

    /// The `GateContext` stage 6 and the review loop share.
    const fn gate_context<'r>(
        &'r self,
        run: &'r Run,
        snapshot: &'r GraphSnapshot,
    ) -> GateContext<'r, S, C> {
        GateContext {
            store: self.parts.store,
            clock: self.parts.clock,
            run,
            snapshot,
            user: self.parts.user,
            box_id: self.parts.box_id,
        }
    }

    /// The run and the item back to `running` / `in_progress` after a gate answer.
    async fn unpark(&self, run: &Run, now: DateTime<Utc>) -> Result<(), EngineError> {
        self.parts
            .store
            .transition_run(run.id, RunStatus::AwaitingApproval, RunStatus::Running, now)
            .await?;
        if let Some(item) = run.item_id {
            self.parts
                .store
                .transition(item, Status::AwaitingApproval, Status::InProgress)
                .await?;
        }
        Ok(())
    }

    /// Where the run is, without moving it.
    async fn resting(&self, run: &Run) -> Result<Rest, EngineError> {
        let snapshot = Self::snapshot_of(run)?;
        let steps = self.parts.store.run_steps(run.id).await?;
        let position = match cursor(&snapshot, &steps) {
            Cursor::Finished => None,
            Cursor::Create { position, .. } => Some(position),
            Cursor::Run(step) | Cursor::Rest { step, .. } => steps
                .iter()
                .find(|row| row.id == step)
                .map(|row| row.position),
        };
        Ok(Rest {
            run: run.status,
            position,
            failure: None,
        })
    }

    /// The run's own snapshot, decoded. Invariant 2: the walk reads this and never the live graph.
    fn snapshot_of(run: &Run) -> Result<GraphSnapshot, EngineError> {
        let value = run
            .graph_snapshot
            .clone()
            .ok_or(EngineError::SnapshotVersion { run: run.id, v: 0 })?;
        let snapshot: GraphSnapshot = serde_json::from_value(value)
            .map_err(|_| EngineError::SnapshotVersion { run: run.id, v: 0 })?;
        if snapshot.v != GraphSnapshot::V {
            return Err(EngineError::SnapshotVersion {
                run: run.id,
                v: snapshot.v,
            });
        }
        Ok(snapshot)
    }

    /// A graph run always has an item; a chat run (`item_id IS NULL`) is MOD-2's and never reaches
    /// the walk.
    fn item_of(run: &Run) -> Result<ItemId, EngineError> {
        run.item_id.ok_or(EngineError::RunStatus {
            run: run.id,
            status: run.status,
            expected: "a graph run, which has an item",
        })
    }

    /// The snapshot phase at `position`.
    fn phase_at(snapshot: &GraphSnapshot, position: i32) -> Result<SnapshotPhase, EngineError> {
        snapshot
            .phases
            .iter()
            .find(|phase| phase.position == position)
            .cloned()
            .ok_or_else(|| {
                EngineError::Store(StoreError::Constraint(format!(
                    "the run's snapshot has no phase at position {position}"
                )))
            })
    }

    /// The candidate a step was created for, denormalised back out of the row and the phase.
    ///
    /// The row is authority for `agent_id` and `model` — it is what a resume reads — and the phase
    /// supplies `agent_name`, which `run_step` does not carry.
    fn candidate_of(
        step: &RunStep,
        phase: &SnapshotPhase,
    ) -> Result<SnapshotCandidate, EngineError> {
        let agent_id = step.agent_id.ok_or_else(|| {
            EngineError::Store(StoreError::Constraint(format!(
                "step {} names no agent; stage 1 creates none without one",
                step.id
            )))
        })?;
        let named = phase
            .candidates
            .iter()
            .find(|candidate| candidate.agent_id == agent_id);
        Ok(SnapshotCandidate {
            agent_id,
            agent_name: named.map_or_else(String::new, |candidate| candidate.agent_name.clone()),
            model: step
                .model
                .clone()
                .or_else(|| named.map(|candidate| candidate.model.clone()))
                .unwrap_or_default(),
        })
    }

    /// The session's working directory, re-derived from the step's own trees.
    ///
    /// `prepare` is idempotent on the tree rows but its `Prepared::cwd` is not persisted, so the
    /// directory is asked for again rather than carried — which is plan D16 applied to a value
    /// that looks harmless to cache and would be the first piece of cross-call state.
    async fn cwd_of(&self, run: &Run, step: &RunStep) -> Result<std::path::PathBuf, EngineError> {
        let phase = Self::phase_at(&Self::snapshot_of(run)?, step.position)?;
        let Prepared { cwd, .. } = self
            .parts
            .isolator
            .prepare(run.id, step.id, &run.repo_scope, phase.isolation)
            .await?;
        Ok(cwd)
    }

    /// The document of `phase.output_kind` produced by **this step**, at its highest version.
    async fn output_of(
        &self,
        item: ItemId,
        phase: &SnapshotPhase,
        step: StepId,
    ) -> Result<Option<Document>, EngineError> {
        let mut mine: Vec<Document> = self
            .parts
            .store
            .documents_of_kinds(item, std::slice::from_ref(&phase.output_kind))
            .await?
            .into_iter()
            .filter(|document| document.produced_by_step_id == Some(step))
            .collect();
        mine.sort_by_key(|document| document.version);
        Ok(mine.pop())
    }

    /// One item row.
    async fn item(&self, id: ItemId) -> Result<Item, EngineError> {
        self.parts
            .store
            .item(id)
            .await?
            .ok_or(EngineError::Store(StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            }))
    }

    /// One run row.
    async fn run(&self, id: RunId) -> Result<Run, EngineError> {
        self.parts
            .store
            .run(id)
            .await?
            .ok_or(EngineError::Store(StoreError::NotFound {
                entity: "run",
                id: id.to_string(),
            }))
    }

    /// One project row.
    async fn project(&self, id: htui_core::model::ProjectId) -> Result<Project, EngineError> {
        self.parts
            .store
            .project(id)
            .await?
            .ok_or(EngineError::Store(StoreError::NotFound {
                entity: "project",
                id: id.to_string(),
            }))
    }

    /// One step row of `run`. There is no per-step read on the seam (blueprint H-11), which is why
    /// [`Command::AnswerGate`] and [`Command::RetryStep`] carry the run.
    async fn step(&self, run: RunId, step: StepId) -> Result<RunStep, EngineError> {
        self.parts
            .store
            .run_steps(run)
            .await?
            .into_iter()
            .find(|row| row.id == step)
            .ok_or(EngineError::Store(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            }))
    }

    /// `item_kind.name`, for the prompt's `item_kind` line.
    async fn item_kind_name(&self, item: &Item) -> Result<String, EngineError> {
        Ok(self
            .parts
            .store
            .item_kinds(item.project_id)
            .await?
            .into_iter()
            .find(|kind| kind.id == item.kind_id)
            .map_or_else(String::new, |kind| kind.name))
    }

    /// `project.settings`, read as defaults when the blob does not decode — [`ProjectSettings`]'s
    /// own rule, since every field of it defaults.
    fn project_settings(project: &Project) -> ProjectSettings {
        serde_json::from_value(project.settings.clone()).unwrap_or_default()
    }
}

/// Which of a phase's `input_kinds` are **required** (blueprint H-8, plan D21).
///
/// A kind is required unless a **later** position of the same snapshot produces it. The seeded
/// `implement` phase lists `review` in `input_kinds` (`crates/htui-core/src/seed.rs:87-90`) —
/// ANA-2 §4.1's own seed amendment, so the review loop can feed a review back — while §4.2 makes a
/// missing input a hard failure before a token is spent (`:412-414`). On attempt 1 no review
/// exists, so a literal reading fails validation criterion 1 at position 2, and the seed and the
/// contract cannot both be right as written.
///
/// This is the one reading under which both hold: a forward input is still mandatory, and the
/// loop's back-edge is absent exactly when it has not run yet. Its absence is recorded in the trim
/// record's notes rather than being silent.
#[must_use]
pub fn required_inputs(phase: &SnapshotPhase, phases: &[SnapshotPhase]) -> Vec<String> {
    phase
        .input_kinds
        .iter()
        .filter(|kind| {
            !phases
                .iter()
                .any(|later| later.position > phase.position && later.output_kind == **kind)
        })
        .cloned()
        .collect()
}

/// Whether a run has a live step at `position` this walk would have to wait on.
///
/// Exposed because milestone 6's Runs tab renders the same answer and milestone 5's sweep reads it
/// to decide what an adopted run owes; both would otherwise re-derive it from `run_steps`.
#[must_use]
pub fn live_step_at(steps: &[RunStep], position: i32) -> Option<&RunStep> {
    latest_at(steps, position).filter(|step| {
        matches!(
            step.status,
            StepStatus::Pending | StepStatus::Running | StepStatus::AwaitingApproval
        )
    })
}

/// The instant the production clock hands a writer, for a caller that has no [`Clock`] of its own.
#[must_use]
pub fn truncated(now: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::SubsecRound as _;
    now.trunc_subsecs(TIMESTAMPTZ_DIGITS)
}

/// Plan D13's harness-side document producer, wired to the seam the engine calls.
///
/// It lives here rather than in `fake.rs` because [`SessionSink`] is `engine.rs`'s trait and T3
/// shipped `FakeOrchestrator::after_done` with the shape to satisfy it — the impl is the one line
/// that joins them, and putting it beside the trait keeps `fake.rs` as T3 left it.
#[cfg(feature = "test-support")]
impl SessionSink for crate::fake::FakeOrchestrator {
    async fn after_done(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        _done: &DoneEvent,
    ) -> Result<(), StoreError> {
        Self::after_done(self, item, step, phase).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeDelta;
    use htui_agent::driver::{AgentDriver, DriverCaps};
    use htui_agent::event::StopReason;
    use htui_core::fixtures::ids;
    use htui_core::model::{
        BoxProfile, Gate, GateOutcome, GraphSnapshot, ItemPatch, NewRepo, NewStepGraph, PhaseId,
        PhasePatch, RepoId, RunMode, RunStatus, SnapshotCandidate, SnapshotPhase, Status,
        StepGraphId, StepGraphPhase, StepStatus,
    };
    use htui_core::scrub::MinimalScrubber;
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

    use super::{
        AgentSelector, Engine, EngineParts, FirstCandidate, NoSink, Resume, required_inputs,
    };
    use crate::command::{Command, CommandOutcome, EngineError, GateAnswer};
    use crate::fake::{FakeOrchestrator, ScriptedStep};
    use crate::isolate::Clock as _;
    use crate::status::RunFailure;

    /// Everything an `Engine` borrows from a `FakeOrchestrator`, held alive for one test.
    ///
    /// The driver factory is a closure over the orchestrator, which is what `FakeDriver`'s
    /// "one script, once" rule needs (blueprint H-12): a fresh driver per `(phase, attempt)`.
    struct Harness {
        orch: FakeOrchestrator,
        scrubber: MinimalScrubber,
        profile: BoxProfile,
    }

    impl Harness {
        async fn new() -> Self {
            let orch = FakeOrchestrator::demo();
            let profile = orch
                .store
                .box_profile(orch.box_id())
                .await
                .expect("MemStore never fails a read")
                .expect("the demo fixture registers this box");
            Self {
                orch,
                scrubber: MinimalScrubber::new([]),
                profile,
            }
        }

        /// The `app_setting` map, read per dispatch rather than once.
        ///
        /// It is read late because `MemStore::set_app_setting` (`mem.rs:430`) is the **only**
        /// writer a test can reach for `step_deadline_seconds`: `SettingKey` is a closed enum of
        /// ten and does not carry it (`prompt/settings.rs:151-172`), and `ProjectPatch` has no
        /// `settings` field at all (`model/hierarchy.rs:102-109`), so neither `set_setting` nor
        /// `update_project` can plant one.
        async fn app(&self) -> BTreeMap<String, serde_json::Value> {
            self.orch
                .store
                .app_settings()
                .await
                .expect("MemStore never fails a read")
        }

        /// Repoints the item at a clone of its graph whose phases `mutate` has edited.
        ///
        /// **`PhasePatch` carries five fields and `gate`, `retry_limit`, `isolation` and
        /// `fan_out` are none of them** (`model/kind.rs:221-232`), so the blueprint's
        /// `update_phase(review, expected, PhasePatch { gate: Some(Gate::Never), .. })` does not
        /// exist. `create_phase` takes a whole `StepGraphPhase`, so a clone with the row a test
        /// wants is the reachable edit — the same two writers `graph::override_graph` uses.
        async fn repoint(
            &self,
            item: htui_core::model::ItemId,
            mutate: impl Fn(&mut StepGraphPhase),
        ) {
            let row = self
                .orch
                .store
                .item(item)
                .await
                .expect("MemStore never fails a read")
                .expect("the fixture holds the item");
            let graph = self
                .orch
                .store
                .resolve_graph(item)
                .await
                .expect("MemStore never fails a read")
                .expect("the item resolves to a graph");
            let clone = self
                .orch
                .store
                .create_step_graph(NewStepGraph {
                    id: StepGraphId::new(),
                    project_id: row.project_id,
                    name: format!("{}-edited", row.key),
                    description: "a test's edit of the live graph".to_owned(),
                })
                .await
                .expect("the name is fresh");
            for phase in &graph.phases {
                let mut edited = StepGraphPhase {
                    id: PhaseId::new(),
                    graph_id: clone.id,
                    ..phase.phase.clone()
                };
                mutate(&mut edited);
                self.orch
                    .store
                    .create_phase(&edited)
                    .await
                    .expect("the clone accepts its phases");
            }
            self.orch
                .store
                .update_item(
                    item,
                    row.version,
                    ItemPatch {
                        step_graph_id: Some(Some(clone.id)),
                        author_id: row.created_by,
                        reason: "a test's edit of the live graph".to_owned(),
                        ..ItemPatch::default()
                    },
                )
                .await
                .expect("the item's version is current");
        }

        /// One dispatch through a freshly built engine. The engine holds nothing between calls
        /// (plan D16), so building one per command is the shape production has too.
        async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError> {
            let graphs = self.orch.graphs();
            let driver = |candidate: &SnapshotCandidate, phase: &str, attempt: i32| {
                let _ = candidate;
                Box::new(self.orch.driver_for(phase, attempt)) as Box<dyn AgentDriver>
            };
            let engine = Engine::new(EngineParts {
                store: &self.orch.store,
                graphs: &graphs,
                isolator: &self.orch.isolator,
                clock: &self.orch.clock,
                selector: &FirstCandidate,
                sink: &self.orch,
                driver: &driver,
                scrubber: &self.scrubber,
                app: self.app().await,
                box_profile: self.profile.clone(),
                box_id: self.orch.box_id(),
                owner: self.orch.owner(),
                user: self.orch.user(),
            });
            engine.dispatch(command).await
        }

        /// `Engine::resume`, over the same parts.
        async fn resume(&self, run: htui_core::model::RunId) -> Result<Resume, EngineError> {
            let graphs = self.orch.graphs();
            let driver = |candidate: &SnapshotCandidate, phase: &str, attempt: i32| {
                let _ = candidate;
                Box::new(self.orch.driver_for(phase, attempt)) as Box<dyn AgentDriver>
            };
            let engine = Engine::new(EngineParts {
                store: &self.orch.store,
                graphs: &graphs,
                isolator: &self.orch.isolator,
                clock: &self.orch.clock,
                selector: &FirstCandidate,
                sink: &self.orch,
                driver: &driver,
                scrubber: &self.scrubber,
                app: self.app().await,
                box_profile: self.profile.clone(),
                box_id: self.orch.box_id(),
                owner: self.orch.owner(),
                user: self.orch.user(),
            });
            engine.resume(run).await
        }

        /// `ids::HTUI_FEAT_3` is seeded **`queued`** with a live `RUN_2` on it
        /// (`crates/htui-core/src/fixtures.rs:899`, `:1377-1380`), and `create_run` moves an item
        /// `open | failed -> queued` only — so `StartRun` on it is a `Constraint` until the seeded
        /// run is ended. Cancelling `RUN_2` moves the item `queued -> open` in `finish_run`'s own
        /// transaction, which is the one-line prologue every case that walks `FEAT-3` needs.
        async fn free_feat_3(&self) {
            self.orch
                .store
                .finish_run(
                    ids::RUN_2,
                    RunStatus::Cancelled,
                    None,
                    self.orch.clock.now(),
                )
                .await
                .expect("the seeded run is queued and cancellable");
        }
    }

    /// Blueprint H-8 in one assertion: `review` is produced at position 3, so `implement` at
    /// position 2 may run without it; `plan` is produced earlier, so it may not.
    #[test]
    fn a_back_edge_input_is_optional_and_a_forward_one_is_not() {
        let snapshot: GraphSnapshot = serde_json::from_value(
            htui_core::fixtures::demo_data()
                .runs
                .into_iter()
                .find(|row| row.id == ids::RUN_1)
                .expect("the fixture holds RUN_1")
                .graph_snapshot
                .expect("RUN_1 carries a snapshot"),
        )
        .expect("the fixture snapshot is a `GraphSnapshot`");

        let implement = snapshot
            .phases
            .iter()
            .find(|phase| phase.name == "implement")
            .expect("the seeded `feature` graph has one");
        assert_eq!(implement.input_kinds, ["plan", "review"]);
        assert_eq!(
            required_inputs(implement, &snapshot.phases),
            ["plan"],
            "`review` is the loop's back edge and cannot exist on attempt 1 (plan D21)"
        );

        let review = snapshot
            .phases
            .iter()
            .find(|phase| phase.name == "review")
            .expect("the seeded `feature` graph has one");
        assert_eq!(
            required_inputs(review, &snapshot.phases),
            ["implement"],
            "nothing later produces `implement`, so it is required"
        );
    }

    /// The selector this milestone ships, and the one thing it must not be asked to decide.
    #[test]
    fn the_first_candidate_selector_takes_the_first_eligible() {
        let phase = SnapshotPhase {
            position: 0,
            name: "prd".to_owned(),
            fan_out: 1,
            gate: Gate::Always,
            gate_effective: Gate::Always,
            gate_hard: true,
            retry_limit: 1,
            input_kinds: Vec::new(),
            output_kind: "prd".to_owned(),
            isolation: htui_core::model::Isolation::Worktree,
            command_queue: htui_core::model::CommandQueue::Off,
            verify_command: None,
            deadline_seconds: Some(7200),
            template: htui_core::model::SnapshotTemplate {
                name: "prd".to_owned(),
                version: 1,
            },
            token_budget: None,
            candidates: Vec::new(),
            judge: None,
        };
        let candidates = vec![
            SnapshotCandidate {
                agent_id: ids::AGENT_CLAUDE,
                agent_name: "claude".to_owned(),
                model: "sonnet".to_owned(),
            },
            SnapshotCandidate {
                agent_id: ids::AGENT_CLAUDE_CLI,
                agent_name: "claude-cli".to_owned(),
                model: "default".to_owned(),
            },
        ];
        assert_eq!(
            FirstCandidate.select(&phase, &candidates),
            Some(&candidates[0])
        );
        assert_eq!(FirstCandidate.select(&phase, &[]), None);
    }

    /// The whole walk, once: `FEAT-3` reaches its first gate, four approvals finish it, and the
    /// last write is `finish_run` moving the item with the run.
    #[tokio::test]
    async fn a_feature_graph_walks_its_four_phases() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;

        let CommandOutcome::Started { run, rest } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the demo graph resolves and the box has a slot")
        else {
            panic!("`StartRun` answers `Started`");
        };

        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        assert_eq!(rest.position, Some(0), "every seeded phase gates `always`");
        let steps = harness.orch.steps(run).await;
        assert_eq!(steps.len(), 1);
        assert_eq!(
            (steps[0].position, steps[0].attempt, steps[0].fanout_index),
            (0, 1, 0)
        );
        assert_eq!(steps[0].status, StepStatus::AwaitingApproval);
        assert_eq!(steps[0].phase_name, "prd");
        assert!(
            steps[0].prompt_digest.is_some(),
            "stage 3 wrote the digest and the recorder rewrote the same one (blueprint H-13)"
        );
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::AwaitingApproval
        );

        for position in 0..4 {
            let steps = harness.orch.steps(run).await;
            let step = steps
                .iter()
                .find(|step| step.position == position)
                .expect("the walk reached this position");
            assert_eq!(step.status, StepStatus::AwaitingApproval, "at {position}");
            harness
                .dispatch(Command::AnswerGate {
                    run,
                    step: step.id,
                    answer: GateAnswer::Approved,
                })
                .await
                .expect("the step produced its document");
        }

        let row = harness.orch.run(run).await;
        assert_eq!(row.status, RunStatus::Done);
        assert_eq!(row.finished_at, Some(harness.orch.clock.now()));
        let item = harness.orch.item(ids::HTUI_FEAT_3).await;
        assert_eq!(
            item.status,
            Status::Done,
            "plan D7: `finish_run` mirrors it"
        );
        assert!(item.closed_at.is_some());

        let steps = harness.orch.steps(run).await;
        assert_eq!(steps.len(), 4);
        assert!(steps.iter().all(|step| step.status == StepStatus::Done));
        assert_eq!(
            steps
                .iter()
                .map(|step| step.phase_name.as_str())
                .collect::<Vec<_>>(),
            ["prd", "plan", "implement", "review"]
        );

        let documents = harness
            .orch
            .store
            .documents(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read");
        for kind in ["prd", "plan", "implement", "review"] {
            assert!(
                documents.iter().any(|head| head.kind == kind),
                "the sink wrote a `{kind}` document (plan D13)"
            );
        }
    }

    /// ANA-2 §12 criterion 5 (`:2096`): approve refuses without the artefact and writes nothing.
    #[tokio::test]
    async fn approve_without_the_document_is_refused_and_writes_nothing() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .orch
            .script("prd", 1, ScriptedStep::done_without_output());

        let CommandOutcome::Started { run, .. } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        let steps = harness.orch.steps(run).await;
        let step = steps[0].id;
        assert_eq!(steps[0].status, StepStatus::AwaitingApproval);

        let refused = harness
            .dispatch(Command::AnswerGate {
                run,
                step,
                answer: GateAnswer::Approved,
            })
            .await
            .expect_err("there is no `prd` document to approve");
        assert!(
            matches!(&refused, EngineError::MissingOutputForApproval { kind, .. } if kind == "prd"),
            "{refused}"
        );
        let after = harness.orch.steps(run).await;
        assert_eq!(after.len(), 1, "a refused approve creates no row");
        assert_eq!(after[0].status, StepStatus::AwaitingApproval);
    }

    /// `docs/ANA-2.md:412-414`: a required input that resolves to nothing fails the run before a
    /// token is spent, and the step's `prompt_digest` proves no prompt was assembled.
    #[tokio::test]
    async fn a_missing_required_input_fails_before_a_token() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;

        // A kind nothing in the graph produces, so the back-edge rule cannot excuse it.
        let graph = harness
            .orch
            .store
            .resolve_graph(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .expect("FEAT-3 resolves to the `feature` graph");
        let prd = &graph.phases[0].phase;
        harness
            .orch
            .store
            .update_phase(
                prd.id,
                prd.updated_at,
                PhasePatch {
                    input_kinds: Some(vec!["spec".to_owned()]),
                    ..PhasePatch::default()
                },
            )
            .await
            .expect("the phase exists");

        let CommandOutcome::Started { run, rest } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        assert_eq!(rest.run, RunStatus::Failed);
        assert_eq!(
            rest.failure,
            Some(RunFailure::MissingInput("spec".to_owned()))
        );
        let row = harness.orch.run(run).await;
        assert_eq!(row.failure.as_deref(), Some("missing input document: spec"));
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::Failed
        );
        let steps = harness.orch.steps(run).await;
        assert_eq!(steps[0].status, StepStatus::Failed);
        assert!(
            steps[0].prompt_digest.is_none(),
            "no prompt was assembled, so no token was spent"
        );
    }

    /// `docs/ANA-2.md:476-482`: a gated phase whose only candidate is CLI-only is refused, the item
    /// is `blocked` **and not** `failed` (blueprint H-16), and the reason is a readable note.
    #[tokio::test]
    async fn a_cli_only_candidate_is_refused_at_a_gated_phase() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .orch
            .with_candidates("prd", vec![(ids::AGENT_CLAUDE_CLI, "default")]);

        let CommandOutcome::Started { run, rest } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        assert_eq!(rest.run, RunStatus::Failed);
        assert_eq!(rest.failure, Some(RunFailure::MissingCapability));
        assert_eq!(
            harness.orch.run(run).await.failure.as_deref(),
            Some("missing_capability: inline_approval")
        );
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::Blocked,
            "the item is blocked before the run is failed (blueprint H-16)"
        );
        let notes = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read");
        assert!(
            notes
                .iter()
                .any(|note| note.body.contains("missing_capability: inline_approval")),
            "ANA-2 invariant 7: a refusal a human can read"
        );
        assert!(
            harness.orch.steps(run).await.is_empty(),
            "stage 1 refuses before a step row exists"
        );
    }

    /// `:430` and the `never` gate's retry cell: two attempts against `retry_limit = 1`, then the
    /// run fails with `missing_output`.
    #[tokio::test]
    async fn a_step_that_produced_nothing_retries_once_then_fails() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" {
                    phase.gate = Gate::Never;
                }
            })
            .await;
        harness
            .orch
            .script("prd", 1, ScriptedStep::done_without_output());
        harness
            .orch
            .script("prd", 2, ScriptedStep::done_without_output());

        let CommandOutcome::Started { run, rest } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        assert_eq!(rest.run, RunStatus::Failed);
        assert_eq!(rest.failure, Some(RunFailure::MissingOutput));
        assert_eq!(
            harness.orch.run(run).await.failure.as_deref(),
            Some("missing_output")
        );
        let steps = harness.orch.steps(run).await;
        assert_eq!(steps.len(), 2, "plan D3: `retry_limit = 1` permits two");
        assert_eq!(
            steps
                .iter()
                .map(|step| (step.position, step.attempt, step.status))
                .collect::<Vec<_>>(),
            [(0, 1, StepStatus::Failed), (0, 2, StepStatus::Failed)]
        );
    }

    /// `:439` and plan D8: a step that outlives its deadline settles `failed` with no sleep, and
    /// the reason is readable on the item (blueprint H-9, widened).
    #[tokio::test]
    async fn a_step_that_outlives_its_deadline_settles_failed() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        // The only reachable rung: `SettingKey` does not carry `step_deadline_seconds` and
        // `ProjectPatch` has no `settings` field, so the app rung is the one a test can plant.
        harness
            .orch
            .store
            .set_app_setting("step_deadline_seconds", serde_json::json!(1));
        harness.orch.advance_after_done(TimeDelta::seconds(5));

        let CommandOutcome::Started { run, rest } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        assert_eq!(rest.run, RunStatus::AwaitingApproval, "`always` parks it");
        assert_eq!(
            harness.orch.steps(run).await[0].status,
            StepStatus::AwaitingApproval
        );
        let notes = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read");
        assert!(
            notes
                .iter()
                .any(|note| note.body.contains("deadline elapsed")),
            "the settle's reason is readable: {notes:?}"
        );
    }

    /// ANA-2 §12 criterion 6 (`:2098`): a human's rejection loops once, then escalates.
    #[tokio::test]
    async fn a_review_rejection_loops_then_escalates() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .orch
            .script("review", 1, ScriptedStep::review("approve", "first"));
        harness
            .orch
            .script("review", 2, ScriptedStep::review("approve", "second"));

        let CommandOutcome::Started { run, .. } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        // Approve prd, plan and implement; the walk then parks at `review`.
        for _ in 0..3 {
            let steps = harness.orch.steps(run).await;
            let step = steps
                .iter()
                .find(|step| step.status == StepStatus::AwaitingApproval)
                .expect("a parked step");
            harness
                .dispatch(Command::AnswerGate {
                    run,
                    step: step.id,
                    answer: GateAnswer::Approved,
                })
                .await
                .expect("the artefact is there");
        }

        let steps = harness.orch.steps(run).await;
        let review = steps
            .iter()
            .find(|step| step.phase_name == "review" && step.attempt == 1)
            .expect("the walk reached the review");
        harness
            .dispatch(Command::AnswerGate {
                run,
                step: review.id,
                answer: GateAnswer::Rejected {
                    note: "no tests".to_owned(),
                },
            })
            .await
            .expect("a rejection loops");

        let steps = harness.orch.steps(run).await;
        let at = |position: i32, attempt: i32| {
            steps
                .iter()
                .find(|step| step.position == position && step.attempt == attempt)
                .unwrap_or_else(|| panic!("({position},{attempt}) exists"))
        };
        assert_eq!(at(2, 1).status, StepStatus::Superseded, "plan D5");
        assert_eq!(
            at(3, 1).status,
            StepStatus::Cancelled,
            "`failed -> superseded` is illegal, so the rejecting review is retired by the one \
             legal move it has; leaving it `failed` would rest the walk on it forever"
        );
        assert_eq!(
            at(3, 1).gate_outcome,
            Some(GateOutcome::Rejected),
            "`transition_step` moves `status` and nothing else, so the verdict survives"
        );
        assert_eq!(at(3, 1).gate_note.as_deref(), Some("no tests"));
        assert_eq!(
            at(2, 2).status,
            StepStatus::AwaitingApproval,
            "the loop resumed and the walk parked the new implement"
        );

        // The re-run implement reads the rejecting review through the ordinary `input_kinds`.
        let inputs = harness
            .orch
            .store
            .resolve_inputs(
                ids::HTUI_FEAT_3,
                run,
                &["plan".to_owned(), "review".to_owned()],
            )
            .await
            .expect("MemStore never fails a read");
        assert!(
            inputs
                .iter()
                .any(|input| input.kind == "review" && input.document.is_some()),
            "§4.4's carried review arrives through §4.2's resolver"
        );

        // Approve the second implement; the second review parks; reject it and the budget is out.
        harness
            .dispatch(Command::AnswerGate {
                run,
                step: at(2, 2).id,
                answer: GateAnswer::Approved,
            })
            .await
            .expect("the artefact is there");
        let steps = harness.orch.steps(run).await;
        let review2 = steps
            .iter()
            .find(|step| step.position == 3 && step.attempt == 2)
            .expect("the review re-ran at its own attempt");
        assert_eq!(review2.status, StepStatus::AwaitingApproval);

        harness
            .dispatch(Command::AnswerGate {
                run,
                step: review2.id,
                answer: GateAnswer::Rejected {
                    note: "still no tests".to_owned(),
                },
            })
            .await
            .expect("a second rejection escalates");

        let row = harness.orch.run(run).await;
        assert_eq!(
            row.status,
            RunStatus::AwaitingApproval,
            "escalate is not terminate"
        );
        assert!(
            row.failure.is_none(),
            "blueprint A-4: `run.failure` stays NULL"
        );
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::Blocked
        );
        let notes = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read");
        assert!(
            notes.iter().any(|note| {
                note.body.contains("review loop exhausted after 2 attempts")
                    && note.body.contains("implement")
                    && note.body.contains("attempt 2")
            }),
            "criterion 6's exact wording: {notes:?}"
        );
    }

    /// ANA-2 §12 criterion 7 (`:2103`): two identical `after_hash` values stop the loop *before*
    /// its retry budget says so.
    #[tokio::test]
    async fn two_identical_after_hashes_stop_the_loop_early() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;

        // The predicate's hash half needs a repo in scope; the demo fixture holds none
        // (`crates/htui-core/src/store/mem.rs:196`), so the case creates the primary and passes
        // `repo_scope: None`, which also exercises plan D14's happy path.
        let repo = RepoId::new();
        harness
            .orch
            .store
            .create_repo(NewRepo {
                id: repo,
                project_id: ids::PROJECT_HTUI,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the project has no repo yet");

        // `retry_limit = 3` on `implement`, so `may_attempt(3, 3)` would permit more: the loop has
        // to stop for the *other* reason or the case proves nothing.
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "implement" {
                    phase.retry_limit = 3;
                }
            })
            .await;
        for _ in 0..16 {
            harness.orch.isolator.script_after(Some("same"));
        }

        let CommandOutcome::Started { run, .. } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };
        assert_eq!(
            harness.orch.run(run).await.repo_scope,
            vec![repo],
            "plan D14: `None` resolves to the primary repo"
        );

        let approve_parked = async |times: usize| {
            for _ in 0..times {
                let steps = harness.orch.steps(run).await;
                let step = steps
                    .iter()
                    .find(|step| step.status == StepStatus::AwaitingApproval)
                    .expect("a parked step");
                harness
                    .dispatch(Command::AnswerGate {
                        run,
                        step: step.id,
                        answer: GateAnswer::Approved,
                    })
                    .await
                    .expect("the artefact is there");
            }
        };
        let reject_parked = async || {
            let steps = harness.orch.steps(run).await;
            let step = steps
                .iter()
                .find(|step| step.status == StepStatus::AwaitingApproval)
                .expect("a parked step")
                .clone();
            harness
                .dispatch(Command::AnswerGate {
                    run,
                    step: step.id,
                    answer: GateAnswer::Rejected {
                        note: format!("attempt {} is no better", step.attempt),
                    },
                })
                .await
                .expect("a rejection is answerable")
        };

        approve_parked(3).await; // prd, plan, implement 1
        reject_parked().await; // review 1 -> the loop resumes at implement 2
        approve_parked(1).await; // implement 2
        reject_parked().await; // review 2 -> identical `after_hash`, so the loop stops

        let row = harness.orch.run(run).await;
        assert_eq!(row.status, RunStatus::AwaitingApproval);
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::Blocked
        );
        let notes = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read");
        assert!(
            notes.iter().any(|note| {
                note.body.contains("no_progress_hash")
                    && note.body.contains("review loop exhausted after 2 attempts")
            }),
            "the budget permitted a third attempt; the predicate is what stopped it: {notes:?}"
        );
        assert!(
            !harness
                .orch
                .steps(run)
                .await
                .iter()
                .any(|step| step.attempt == 3),
            "no third implement attempt was created"
        );
    }

    /// §6.2's `retry`, on both statuses its guard admits, and the two it refuses for.
    #[tokio::test]
    async fn retry_admits_the_next_attempt_through_stage_one() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;

        let CommandOutcome::Started { run, .. } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };
        let parked = harness.orch.steps(run).await[0].id;

        let CommandOutcome::Retried { step, rest } = harness
            .dispatch(Command::RetryStep { run, step: parked })
            .await
            .expect("`retry_limit = 1` permits a second attempt")
        else {
            panic!("`RetryStep` answers `Retried`");
        };
        assert_eq!(step, parked);
        assert_eq!(rest.run, RunStatus::AwaitingApproval);

        let steps = harness.orch.steps(run).await;
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[0].status,
            StepStatus::Superseded,
            "`answer_gate(Retried)`"
        );
        assert_eq!((steps[1].position, steps[1].attempt), (0, 2));
        assert_eq!(steps[1].status, StepStatus::AwaitingApproval);

        // A third attempt is out of budget (plan D3, prospectively).
        let refused = harness
            .dispatch(Command::RetryStep {
                run,
                step: steps[1].id,
            })
            .await
            .expect_err("`retry_limit = 1` permits two attempts and no more");
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
    }

    /// ANA-2 §12 criterion 2 (`:2088`): a live run reads its snapshot, never the graph.
    #[tokio::test]
    async fn a_live_run_ignores_a_gate_edit() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;

        let CommandOutcome::Started { run, .. } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        harness
            .repoint(ids::HTUI_FEAT_3, |phase| phase.gate = Gate::Never)
            .await;

        // Every remaining position still parks, because the run walks the snapshot it was created
        // with and that snapshot says `always`.
        for position in 1..4 {
            let steps = harness.orch.steps(run).await;
            let step = steps
                .iter()
                .find(|step| step.status == StepStatus::AwaitingApproval)
                .expect("a parked step");
            harness
                .dispatch(Command::AnswerGate {
                    run,
                    step: step.id,
                    answer: GateAnswer::Approved,
                })
                .await
                .expect("the artefact is there");
            let steps = harness.orch.steps(run).await;
            assert!(
                steps
                    .iter()
                    .any(|step| step.position == position
                        && step.status == StepStatus::AwaitingApproval),
                "position {position} parked despite the live graph saying `never`"
            );
        }
    }

    /// ANA-2 §12 criterion 3 (`:2090`): the graph moved under a parked run.
    #[tokio::test]
    async fn a_topology_mismatch_parks_on_resume() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;

        let CommandOutcome::Started { run, .. } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };

        // `input_kinds` is one of `PhasePatch`'s five, and it is a `SnapshotPhase` field, so
        // editing it moves the topology digest — which is the whole content of criterion 3.
        let graph = harness
            .orch
            .store
            .resolve_graph(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .expect("FEAT-3 resolves");
        let prd = &graph.phases[0].phase;
        harness
            .orch
            .store
            .update_phase(
                prd.id,
                prd.updated_at,
                PhasePatch {
                    input_kinds: Some(vec!["spec".to_owned()]),
                    ..PhasePatch::default()
                },
            )
            .await
            .expect("the phase exists");

        let before = harness.orch.steps(run).await;
        let resumed = harness.resume(run).await.expect("the run is readable");
        let Resume::TopologyChanged { snapshot, live, .. } = resumed else {
            panic!("the live graph moved, so the digests differ: {resumed:?}");
        };
        assert_ne!(snapshot, live);
        assert_eq!(
            harness.orch.steps(run).await.len(),
            before.len(),
            "the walk is not advanced"
        );
        let notes = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read");
        assert!(
            notes
                .iter()
                .any(|note| note.body.contains("topology mismatch")),
            "{notes:?}"
        );

        // An unedited run resumes into the walk instead.
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        let CommandOutcome::Started { run, .. } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts")
        else {
            panic!("`StartRun` answers `Started`");
        };
        assert!(matches!(
            harness.resume(run).await.expect("the run is readable"),
            Resume::Walked(_)
        ));
    }

    /// The driver factory's contract: one driver per `(phase, attempt)`, because `FakeDriver`
    /// plays its script once (blueprint H-12).
    #[tokio::test]
    async fn the_driver_factory_carries_the_phase_and_the_attempt() {
        let orch = FakeOrchestrator::demo();
        orch.script("implement", 2, ScriptedStep::failing(StopReason::Refusal));
        let first = orch.driver_for("implement", 1);
        let second = orch.driver_for("implement", 2);
        assert_eq!(first.name(), "fake");
        assert_eq!(second.name(), "fake");
        assert_ne!(
            format!("{:?}", first.caps()),
            format!("{:?}", DriverCaps::default()),
            "the demo harness hands out full capabilities"
        );
        assert!(
            !std::ptr::eq(std::ptr::from_ref(&first), std::ptr::from_ref(&second)),
            "two attempts are two drivers"
        );
    }

    /// A `MemStore` with no run is a `NotFound`, not a panic — the walk never assumes a row.
    #[tokio::test]
    async fn an_unknown_run_is_not_found() {
        let harness = Harness::new().await;
        let refused = harness
            .resume(htui_core::model::RunId::new())
            .await
            .expect_err("no such run");
        assert!(matches!(
            refused,
            EngineError::Store(htui_core::store::StoreError::NotFound { entity: "run", .. })
        ));
        let _: &MemStore = &harness.orch.store;
    }

    /// `NoSink` is production's answer: the engine writes no document, ever (plan D13).
    #[tokio::test]
    async fn the_production_sink_writes_nothing() {
        use super::SessionSink as _;
        let orch = FakeOrchestrator::demo();
        let before = orch
            .store
            .documents(ids::HTUI_FEAT_1)
            .await
            .expect("MemStore never fails a read")
            .len();
        let steps = harness_steps();
        NoSink
            .after_done(
                ids::HTUI_FEAT_1,
                &steps,
                &phase_of(&orch).await,
                &htui_agent::event::DoneEvent {
                    stop_reason: StopReason::EndTurn,
                },
            )
            .await
            .expect("`NoSink` never fails");
        assert_eq!(
            orch.store
                .documents(ids::HTUI_FEAT_1)
                .await
                .expect("MemStore never fails a read")
                .len(),
            before
        );
    }

    fn harness_steps() -> htui_core::model::RunStep {
        htui_core::fixtures::demo_data()
            .steps
            .into_iter()
            .find(|step| step.run_id == ids::RUN_1 && step.position == 0)
            .expect("RUN_1 has a step at position 0")
    }

    async fn phase_of(orch: &FakeOrchestrator) -> SnapshotPhase {
        let item = orch
            .store
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture holds FEAT-1");
        crate::graph::resolve(
            &orch.store,
            &orch.graphs(),
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
        )
        .await
        .expect("the stand-in supplies rung 1")
        .snapshot
        .phases
        .remove(0)
    }
}
