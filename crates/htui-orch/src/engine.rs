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
use std::future::Future;

use futures::future::Either;

use chrono::{DateTime, TimeDelta, Utc};
use htui_agent::driver::{AgentDriver, PermissionPolicy, SessionSpec, ToolExposure};
use htui_agent::event::DoneEvent;
use htui_agent::record::{Recorder, RunCap, pump};
use htui_core::model::{
    BoxId, BoxProfile, CommandRunId, CommandRunStatus, Document, EventKind, Gate, GateOutcome,
    GraphSnapshot, Isolation, Item, ItemId, NewCommandRun, NewNote, NewRun, NewRunStep, NoteId,
    Project, ProjectSettings, PromptScope, RepoId, Run, RunId, RunStatus, RunStep, RunStepCommit,
    SnapshotCandidate, SnapshotPhase, SnapshotTemplate, Status, StepId, StepOutcome, StepStatus,
    TIMESTAMPTZ_DIGITS, UserId, VerifyOutcome,
};
use htui_core::prompt::excerpt::{BUILTIN_ID, ExcerptAudit, ExcerptSet};
use htui_core::prompt::{
    AssembledPrompt, DiffBlock, InputDocument, JudgeCandidate, PromptSpec, SectionName,
    TemplateRef, TemplateRole, TokenEstimator, TrimStrategy, VerifyFailure, assemble, settings,
};
use htui_core::scrub::Scrubber;
use htui_core::store::{StoreError, WriteStore};
use serde_json::Value;
use uuid::Uuid;

use crate::command::{Command, CommandOutcome, EngineError, GateAnswer, Rest};
use crate::fanout::{
    AUTO_WIN_REASON, CandidateView, HUMAN_PICK_REASON, HumanReason, JUDGE_KIND, JudgeFailure,
    Route, judge_candidate, judge_inputs, judge_phase, judge_phase_name, parse_judge_verdict,
    prefilter, route,
};
use crate::gate::{self, GateContext, Landing, LoopOutcome, Settle, SettleInput};
use crate::graph::{self, GraphSource, ResolveError};
use crate::isolate::{Clock, FanoutSlot, Isolator};
use crate::recover::{Heartbeat, LeaseTimes};
use crate::select::{self, SelectInput, Skipped, Walk};
use crate::status::{
    Cursor, RunFailure, cursor, group_at, judge_at, latest_at, may_attempt, next_attempt, winner_at,
};
use crate::verify::{Verifier, VerifyReport, VerifyRequest};

/// The phase whose output document carries ANA-2 §4.4's front-matter verdict (`:440`).
///
/// A name rather than a flag on the row, because that is what ANA-2 has: `:440` says "a
/// `review`-phase output document", and all four seeded graphs that have one call it `review`
/// (`crates/htui-core/src/seed.rs:88-160`). The `bug` graph's implement-shaped phase is `fix` and
/// its review phase is still `review`, which is the case that shows the name is the rule.
const REVIEW_PHASE: &str = "review";

/// The `prompt_template.name` of the judge role (`crates/htui-core/src/prompt/template.rs:54`),
/// read when the snapshot pinned no judge template (plan D53).
const JUDGE_TEMPLATE: &str = "judge";

/// `run_step.fanout_index` of a slot's judge row (`docs/ANA-2.md` §4.5, plan D51).
const JUDGE_FANOUT_INDEX: i32 = -1;

/// A failed judge's park reason when its row carries no `gate_note` — a crash between D51's two
/// failure writes left it `awaiting_approval`, or another writer failed it.
const JUDGE_FAILED: &str = "judge failed";

/// The `app_setting` key of stage 1's budget rule (plan D60 rule 5, OQ-6). Unseeded: absent is `0`.
const MIN_BUDGET_KEY: &str = "min_budget_for_new_attempt";

/// `no_candidate_agent`'s detail when the walk skipped nothing and the selector still declined
/// every eligible candidate (plan D62).
const SELECTOR_DECLINED: &str = "the selector declined every eligible candidate";

/// A ceiling on iterations of one [`Engine::run_to_rest`] call.
///
/// Every iteration either creates a step, runs one, or stops, and both the retry budget and the
/// review loop's are finite — so this is a net and not a bound. It exists because the walk is a
/// loop over rows the engine does not own: a row moved under it by another process (plan D17) must
/// end in a refusal a human can read, never in a spin.
const MAX_ITERATIONS: u32 = 512;

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
    /// The candidate to run as `fanout_index`, or `None` to refuse the phase.
    ///
    /// `eligible` has already survived stage 1's `R-AGT-8` walk and is in `phase_agent.position`
    /// order, so "the first one" is "the highest-preference one".
    ///
    /// Asked once per candidate of a group, with that candidate's `fanout_index` (plan D71): each
    /// candidate's agent, model, driver and session key come from its own answer, never from a
    /// group-level one, so a selector that spreads a group across agents needs no other change. A
    /// `fan_out = 1` phase asks with `0`; the judge (`-1`) is never asked, because its agent is the
    /// project's judge (plan D51).
    fn select<'c>(
        &self,
        phase: &SnapshotPhase,
        eligible: &'c [SnapshotCandidate],
        fanout_index: i32,
    ) -> Option<&'c SnapshotCandidate>;
}

/// The only selector this milestone: the first candidate that survived stage 1, whatever the
/// index — every candidate of a group runs on the same agent until MOD-36's weighted selector
/// (plan D71, OQ-7).
#[derive(Debug, Default, Clone, Copy)]
pub struct FirstCandidate;

impl AgentSelector for FirstCandidate {
    fn select<'c>(
        &self,
        _phase: &SnapshotPhase,
        eligible: &'c [SnapshotCandidate],
        _fanout_index: i32,
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
    /// `key` is the session's [`SessionKey`] (plan D68): three candidates share `(phase, attempt)`
    /// and a judge's two calls share its `fanout_index`, so a sink that produces a different
    /// document per session needs all four parts.
    ///
    /// # Errors
    /// The store's own refusals, which the walk raises as [`EngineError::Store`].
    async fn after_done(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        key: &SessionKey<'_>,
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
        _key: &SessionKey<'_>,
        _done: &DoneEvent,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}

/// What one agent session is (plan D68), for the driver factory and the [`SessionSink`].
///
/// `(phase, attempt)` alone named a session until fan-out: three candidates share it, and the
/// judge's two orderings share `(phase, attempt, -1)`, while `FakeDriver` refuses a second `start`
/// (`crates/htui-agent/src/fake.rs:178-180`) — so a harness that must play each a different script
/// needs the whole key. Production's registry ignores it and builds per `(agent, box)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionKey<'a> {
    /// `run_step.phase_name`: the phase's own name, or `<phase>:judge` for a judge.
    pub phase: &'a str,
    /// `run_step.attempt`, 1-based.
    pub attempt: i32,
    /// `run_step.fanout_index`: `0..fan_out` for a candidate, `-1` for the judge.
    pub fanout_index: i32,
    /// The judge's ordering, `0` forward and `1` reversed (plan D52); `0` for every other session.
    pub call: u32,
}

impl<'a> SessionKey<'a> {
    /// The key of `step`'s first (for a judge, forward) session.
    #[must_use]
    pub fn of(step: &'a RunStep) -> Self {
        Self {
            phase: &step.phase_name,
            attempt: step.attempt,
            fanout_index: step.fanout_index,
            call: 0,
        }
    }
}

/// What one pumped session answered: its `done`, or the driver error a closed stream is.
type SessionResult = Result<DoneEvent, htui_agent::error::DriverError>;

/// How the walk gets a driver for one session.
///
/// **The blueprint's §5.1 factory was `Fn(&SnapshotCandidate) -> Box<dyn AgentDriver>` and could
/// not work**: `FakeDriver` plays its script once and refuses a second `start`, and a harness that
/// scripts per session has no way to answer a factory that is handed nothing about the session. The
/// [`SessionKey`] travels here (plan D68). Milestone 3's registry ignores it and builds per
/// `(agent, box)` as it already does (`registry.rs:132`).
///
/// A closure bound to this type must annotate **both** parameter types (blueprint H-18), e.g.
/// `|_c: &SnapshotCandidate, key: &SessionKey<'_>| …`: the key's lifetime is higher-ranked, and
/// inference does not generalise an unannotated closure over it.
pub type DriverFor<'a> =
    &'a (dyn Fn(&SnapshotCandidate, &SessionKey<'_>) -> Box<dyn AgentDriver> + Sync);

// ---------------------------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------------------------

/// Everything [`Engine::new`] borrows, as a struct literal rather than a builder.
///
/// A builder would let a caller forget one of fourteen fields and find out at run time; a literal
/// cannot compile until every one of them is named.
pub struct EngineParts<'a, S, G, I, V, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    V: Verifier + ?Sized,
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
    /// Stage 5's `verify_command` (plan D30, D41). Production is a `ShellVerifier` built once per
    /// process — the `verify` class semaphore is per verifier, so one per step would serialise
    /// nothing (blueprint H-22).
    pub verifier: &'a V,
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
impl<S, G, I, V, C, A, K> core::fmt::Debug for EngineParts<'_, S, G, I, V, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    V: Verifier + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EngineParts")
            .field("isolator", &self.isolator)
            .field("verifier", &self.verifier)
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
pub struct Engine<'a, S, G, I, V, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    V: Verifier + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    parts: EngineParts<'a, S, G, I, V, C, A, K>,
}

/// What [`Engine::resume`] found (ANA-2 §12 criterion 3, `docs/ANA-2.md:2090`).
///
/// **Not in the blueprint**, which names `Engine::resume` in §6.3 and gives it no return type. A
/// bare `Rest` cannot say *why* a walk did not advance, and "the graph moved under a parked run"
/// is exactly the fact criterion 3 is about, so it is a value rather than a note the caller has to
/// go looking for. The note is still written — it is what an operator reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// The snapshot was walked. Either the live graph still hashes to the run's own `topology`,
    /// or the live graph no longer resolves under a start-time rule (a fan-out or agent cap, a
    /// review or local fan-out, no candidate) and so was not compared at all: the item's
    /// `live graph not comparable` note says which rule refused it.
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

/// One run [`Engine::sweep`] adopted, and what it left the run owing (plan D98, D117).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted {
    /// The adopted run.
    pub run: RunId,
    /// What the caller does next with it.
    pub next: Next,
}

/// What the sweep's adjudication left an adopted run owing (plan D98, blueprint A-4).
///
/// The sweep **walks nothing**: a [`Next::Walk`] run is walked by the caller through
/// [`Engine::resume`], which is criterion 3's topology check and D86's heartbeat, and which
/// milestone 6's `run_worker` calls once per run so one run's sessions never wait on another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// The run is `running` with its rows adjudicated: walk it on through [`Engine::resume`].
    Walk,
    /// The sweep parked the run for a human, and released its lease (plan D87, D94, D96).
    Parked(Rest),
    /// A re-settled step ended the run, and it was cleaned up (plan D36, D91).
    Finished(Rest),
    /// Blueprint A-4: this run's recovery failed with the error's `Display`. The failure is a
    /// `warn`, an `item_note` and a released lease, so another process may adopt the run at once,
    /// and the sweep went on with the next run.
    Error(String),
}

impl<'a, S, G, I, V, C, A, K> Engine<'a, S, G, I, V, C, A, K>
where
    S: WriteStore,
    G: GraphSource,
    I: Isolator + ?Sized,
    V: Verifier + ?Sized,
    C: Clock + ?Sized,
    A: AgentSelector + ?Sized,
    K: SessionSink + ?Sized,
{
    /// The walk over one set of borrowed parts.
    #[must_use]
    pub const fn new(parts: EngineParts<'a, S, G, I, V, C, A, K>) -> Self {
        Self { parts }
    }

    // -- commands (ANA-2 §6.2, blueprint §5.2) ------------------------------------------------

    /// Answers one of ANA-2 §6.2's three verbs and walks the run to its next rest.
    ///
    /// The walk runs under the lease's heartbeat, which sleeps on `tokio::time`: the caller needs
    /// a Tokio runtime with the time driver enabled (blueprint H-3).
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
            Command::CancelRun { run } => self.cancel_run(run).await,
            Command::SelectFanout {
                run,
                position,
                attempt,
                winner,
            } => self.select_fanout(run, position, attempt, winner).await,
        }
    }

    /// §6.2's `run`/`queue`: resolve, create, claim, walk.
    async fn start_run(
        &self,
        item: ItemId,
        mode: htui_core::model::RunMode,
        repo_scope: Option<Vec<RepoId>>,
    ) -> Result<CommandOutcome, EngineError> {
        let item = self.item(item).await?;
        let resolved = match graph::resolve(
            self.parts.store,
            self.parts.graphs,
            &item,
            mode,
            &self.parts.app,
            repo_scope.as_deref(),
            self.parts.box_id,
        )
        .await
        {
            Ok(resolved) => resolved,
            Err(ResolveError::NoCandidate { phase }) => {
                return Err(self.refuse_rung_four(&item, phase).await?.into());
            }
            Err(err) => return Err(err.into()),
        };

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

        self.claim(id).await
    }

    /// Plan D84: `claim_run`, then the leased walk — `start_run`'s tail, factored out so a run a
    /// refusal left `queued` can be re-attempted. No [`Command`] variant carries it (ANA-2 §6.2
    /// has none); milestone 6 decides the verb that calls it.
    ///
    /// The walk runs under the lease's heartbeat, which sleeps on `tokio::time`: the caller needs
    /// a Tokio runtime with the time driver enabled (blueprint H-3).
    ///
    /// # Errors
    /// [`EngineError::ClaimRefused`] naming the rule when `claim_run` does not admit the run,
    /// which then stays `queued` with nothing written; [`EngineError::LeaseLost`] when another
    /// orchestrator takes the lease mid-walk; every other [`EngineError`] the walk raises.
    pub async fn claim(&self, run: RunId) -> Result<CommandOutcome, EngineError> {
        let now = self.now();
        let lease = now + self.lease_times().ttl;
        // Anything but `Admitted` is ANA-2 §4.7's admission refusing: the box is at
        // `max_concurrent_items` or the scope overlaps a live run (plan D83). Nothing is written
        // and the run stays `queued`.
        let claim = self
            .parts
            .store
            .claim_run(run, self.parts.box_id, self.parts.owner, now, lease)
            .await?;
        if !claim.is_admitted() {
            return Err(EngineError::ClaimRefused { run, claim });
        }

        let rest = self.walk_leased(run, self.run_to_rest(run)).await?;
        Ok(CommandOutcome::Started { run, rest })
    }

    /// Plan D62's rung 4 at `StartRun`: no run row exists, so the refusal is written to the item —
    /// `open -> blocked`, then `no_candidate_agent: phase `<p>`` as a note — and handed back for
    /// the caller to raise. A `failed` item has no `blocked` edge (`model/item.rs:56`) and the
    /// compare-and-set answers `Ok(false)`, which leaves it where it is with the note still
    /// written: a refusal nobody can read is what invariant 7 forbids.
    async fn refuse_rung_four(
        &self,
        item: &Item,
        phase: String,
    ) -> Result<ResolveError, EngineError> {
        self.parts
            .store
            .transition(item.id, Status::Open, Status::Blocked)
            .await?;
        let failure = RunFailure::NoCandidateAgent {
            phase: phase.clone(),
            detail: String::new(),
        };
        self.note(item.id, failure.to_string(), None, self.now())
            .await?;
        Ok(ResolveError::NoCandidate { phase })
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
        let phase = Self::phase_at(run.id, &snapshot, step.position)?;
        let item = Self::item_of(&run)?;

        let has_output = self.output_of(item, &phase, step.id).await?.is_some();
        crate::command::answer_gate_enabled(&step, &phase, has_output, &answer)?;
        // Plan D108: after the pure guard and before the first write, so a live lease elsewhere
        // refuses with the gate still unanswered (blueprint F-N).
        self.take_lease(run.id).await?;

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

        // Blueprint F-M: everything after the unpark writes to a `running` run, so all of it —
        // the review loop or the reconcile, not only `run_to_rest` — runs under the heartbeat.
        let tail = async {
            if matches!(answer, GateAnswer::Rejected { .. }) {
                return self
                    .after_rejection(&run, &snapshot, &step, &phase, now)
                    .await;
            }
            // `answer_gate` moved the step `awaiting_approval -> done`
            // (`store/traits.rs:788-791`), so this is the human's half of plan D25's "after a step
            // reaches `done`". A refusal parks the run again, with the reason on the item.
            let step = self.step(run.id, step.id).await?;
            match self.reconcile_done_step(&run, &step, &[]).await? {
                Some(rest) => Ok(rest),
                None => self.run_to_rest(run.id).await,
            }
        };
        let rest = self.walk_leased(run.id, tail).await?;
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
            self.cleanup_run(run.id).await?;
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
        match gate::review_loop(&ctx, step).await? {
            LoopOutcome::Resumed { .. } => self.run_to_rest(run.id).await,
            LoopOutcome::Escalated { attempts, .. } => Ok(Rest {
                run: RunStatus::AwaitingApproval,
                position: Some(step.position),
                failure: Some(RunFailure::ReviewLoopExhausted(attempts)),
            }),
            LoopOutcome::NoTarget => {
                // `review_loop` failed the run itself (`gate.rs`'s three `NoTarget` sites), so
                // this is a terminal `finish_run` the engine did not write and still owes a
                // cleanup for (plan D36).
                self.cleanup_run(run.id).await?;
                Ok(Rest {
                    run: RunStatus::Failed,
                    position: Some(step.position),
                    failure: Some(RunFailure::NoLoopTarget),
                })
            }
        }
    }

    /// §6.2's `retry`: supersede the answered step (or leave the failed one) and admit the next
    /// attempt.
    async fn retry_step(&self, run: RunId, step: StepId) -> Result<CommandOutcome, EngineError> {
        let run = self.run(run).await?;
        // `command::retry_enabled` is a pure function of the step row and says so: the run's own
        // status is "the engine's, at dispatch" (`command.rs:266-268`). This is that check. On a
        // terminal run every write below is a no-op that still creates a step — `unpark`'s two
        // compare-and-sets find a stale `from` and answer `Ok(false)`, while `create_step` refuses
        // no terminal run on either backend — so the run would gain an orphan `pending` step it can
        // never walk, and the command would report success (ANA-2 §4.3's run table has no edge out
        // of `done`, `failed` or `cancelled`, `docs/ANA-2.md:614`).
        if !matches!(run.status, RunStatus::Running | RunStatus::AwaitingApproval) {
            return Err(EngineError::RunStatus {
                run: run.id,
                status: run.status,
                expected: "running | awaiting_approval",
            });
        }
        let snapshot = Self::snapshot_of(&run)?;
        let row = self.step(run.id, step).await?;
        let phase = Self::phase_at(run.id, &snapshot, row.position)?;
        let item = self.item(Self::item_of(&run)?).await?;

        // Blueprint F-J / R-4: `Status::can_move_to` has no `blocked -> in_progress` edge, so a
        // blocked item cannot be resumed until milestone 6 ships `Unblock`.
        if item.status == Status::Blocked {
            return Err(EngineError::ItemBlocked { item: item.id });
        }
        // Plan D65, blueprint F-D: any member of a fanned-out slot retries the whole group, and
        // before `retry_enabled` — a parked group's candidates are `done`, which that guard refuses.
        if phase.fan_out > 1 {
            return self.retry_group(&run, &snapshot, &row, &phase).await;
        }
        crate::command::retry_enabled(&row, &phase)?;
        self.take_lease(run.id).await?;

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
        let tail = async {
            let run = self.run(run.id).await?;
            let steps = self.parts.store.run_steps(run.id).await?;
            let attempt = next_attempt(&steps, row.position);
            match self.admit(&run, &snapshot, &phase, attempt).await? {
                Some(rest) => Ok(rest),
                None => self.run_to_rest(run.id).await,
            }
        };
        let rest = self.walk_leased(run.id, tail).await?;
        Ok(CommandOutcome::Retried { step: row.id, rest })
    }

    /// Plan D65's group retry: every member of the slot `row` belongs to is retired (candidates
    /// and judge, plan D66's `retire_slot`), the run unparks, and stage 1 admits the whole group
    /// at `attempt + 1` before the walk resumes.
    async fn retry_group(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        row: &RunStep,
        phase: &SnapshotPhase,
    ) -> Result<CommandOutcome, EngineError> {
        let steps = self.parts.store.run_steps(run.id).await?;
        // A member of a slot already retired names nothing a retry could replace: `retire_slot`
        // retires the position's **latest** slot, so only a member of that one is admitted.
        let latest = steps
            .iter()
            .filter(|step| step.position == row.position)
            .map(|step| step.attempt)
            .max()
            .unwrap_or(row.attempt);
        if row.attempt != latest {
            return Err(EngineError::StaleSlot {
                step: row.id,
                attempt: row.attempt,
                latest,
            });
        }
        let slot = group_at(&steps, row.position, row.attempt);
        crate::command::retry_group_enabled(run, &slot, phase)?;
        let attempt = row.attempt;
        self.take_lease(run.id).await?;

        let now = self.now();
        gate::retire_slot(&self.gate_context(run, snapshot), &steps, row.position, now).await?;
        self.unpark(run, now).await?;

        let tail = async {
            let run = self.run(run.id).await?;
            match self.admit(&run, snapshot, phase, attempt + 1).await? {
                Some(rest) => Ok(rest),
                None => self.run_to_rest(run.id).await,
            }
        };
        let rest = self.walk_leased(run.id, tail).await?;
        Ok(CommandOutcome::Retried { step: row.id, rest })
    }

    /// §6.2's `select` (plan D65, criterion 10): a human's pick of a parked slot's winner.
    ///
    /// `select_fanout` settles the slot in one transaction — the winner `selected`, the losers
    /// superseded, a live judge `done` — and leaves a judge that already failed alone, with its
    /// reason (`store/traits.rs:808-813`). The reason is also an `item_note` (plan D77): with no
    /// judge, or a failed one, `select_fanout` has nowhere to write it. Then the run unparks, the
    /// winner is reconciled with its siblings, and the walk goes on.
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
    ) -> Result<CommandOutcome, EngineError> {
        let run = self.run(run).await?;
        let snapshot = Self::snapshot_of(&run)?;
        let phase = Self::phase_at(run.id, &snapshot, position)?;
        let steps = self.parts.store.run_steps(run.id).await?;
        let slot = group_at(&steps, position, attempt);
        crate::command::select_enabled(&run, &slot, winner, position, attempt)?;
        let siblings = Self::siblings(&slot, winner);
        self.take_lease(run.id).await?;

        self.parts
            .store
            .select_fanout(
                run.id,
                position,
                attempt,
                winner,
                Some(HUMAN_PICK_REASON.to_owned()),
            )
            .await?;
        let now = self.now();
        let index = slot
            .iter()
            .find(|step| step.id == winner)
            .map_or(0, |step| step.fanout_index);
        self.note_selection(
            &run,
            winner,
            format!(
                "fan-out `{}` attempt {attempt}: candidate {index} {HUMAN_PICK_REASON}",
                phase.name
            ),
            now,
        )
        .await?;
        self.unpark(&run, now).await?;

        let tail = async {
            let run = self.run(run.id).await?;
            let winner = self.step(run.id, winner).await?;
            match self.reconcile_done_step(&run, &winner, &siblings).await? {
                Some(rest) => Ok(rest),
                None => self.run_to_rest(run.id).await,
            }
        };
        let rest = self.walk_leased(run.id, tail).await?;
        Ok(CommandOutcome::Selected { rest })
    }

    /// Plan D77: the reason a winner was chosen, as an `item_note` naming the winner.
    async fn note_selection(
        &self,
        run: &Run,
        winner: StepId,
        body: String,
        now: DateTime<Utc>,
    ) -> Result<(), EngineError> {
        match run.item_id {
            Some(item) => self.note(item, body, Some(winner), now).await,
            None => Ok(()),
        }
    }

    /// The slot's candidates other than `winner`, for the isolator's reconcile (plan D54(d)).
    fn siblings(slot: &[&RunStep], winner: StepId) -> Vec<StepId> {
        slot.iter()
            .map(|step| step.id)
            .filter(|id| *id != winner)
            .collect()
    }

    /// §6.2's `cancel run` (plan D45, ANA-2 §12 criterion 13).
    ///
    /// Every step that has not settled moves to `cancelled` — `pending`, `running`,
    /// `awaiting_approval` and `failed` all reach it (`model/run.rs`) — then the run does, then
    /// plan D36's cleanup runs. `Ok(false)` from a step move is ignored for plan D17's reason:
    /// another process settling a step under this call is not this call's to report, and the run
    /// is ending either way.
    ///
    /// The position is read **before** the moves, because after them there is no live step left
    /// for `cursor` to name and a caller would be told `None` — "the run finished" — about a run
    /// that was stopped at position 2.
    ///
    /// `pub` for the same reason [`cleanup_run`](Self::cleanup_run) is: milestone 6's Runs tab is
    /// the caller ANA-2 §6.2 writes this row for.
    ///
    /// # Errors
    /// [`EngineError::RunStatus`] for a run that has already finished; the store's own refusals.
    pub async fn cancel_run(&self, run: RunId) -> Result<CommandOutcome, EngineError> {
        let row = self.run(run).await?;
        crate::command::cancel_enabled(&row)?;
        let position = self.resting(&row).await?.position;

        let now = self.now();
        let steps = self.parts.store.run_steps(run).await?;
        for step in &steps {
            if step.status.is_terminal() {
                continue;
            }
            self.parts
                .store
                .transition_step(step.id, step.status, StepStatus::Cancelled, now)
                .await?;
        }
        // `finish_run` mirrors the item `queued | in_progress | awaiting_approval -> open`
        // (`store/traits.rs`), which is what frees it for a second run; `failure` stays NULL
        // because a cancel is a human's decision and not a failure.
        self.parts
            .store
            .finish_run(run, RunStatus::Cancelled, None, now)
            .await?;
        self.cleanup_run(run).await?;
        Ok(CommandOutcome::Cancelled {
            rest: Rest {
                run: RunStatus::Cancelled,
                position,
                failure: None,
            },
        })
    }

    // -- the lease (ANA-2 §4.9, plan D86, D87, D107, D108) ---------------------------------------

    /// Plan D86/D107: `walk` raced against [`crate::recover::heartbeat`] in this task.
    ///
    /// On the walk's answer, the lease is released when the run rests anywhere but `running`
    /// (D87); an `Err` writes nothing more, because the store may be what failed. On
    /// [`crate::recover::Heartbeat::Abandoned`] the walk is dropped where it stands, **before** anything else, then
    /// `Isolator::release` drops this process's guards for the run (D99), and the caller gets
    /// [`EngineError::LeaseLost`]. Generic over the walk's output, so one wrapper serves every
    /// command's post-unpark tail (blueprint F-M).
    ///
    /// **A runtime with a time driver is required** (blueprint H-3): the heartbeat sleeps on
    /// `tokio::time::sleep`, so every engine entry that walks — [`Self::dispatch`],
    /// [`Self::claim`] and [`Self::resume`] — panics outside a Tokio runtime built with the time
    /// driver enabled. Every shipped harness is `#[tokio::test]`; milestone 6's caller must build
    /// its runtime with `enable_time` (or `enable_all`).
    async fn walk_leased<T, F>(&self, run: RunId, walk: F) -> Result<T, EngineError>
    where
        F: Future<Output = Result<T, EngineError>>,
    {
        let times = self.lease_times();
        let (store, owner) = (self.parts.store, self.parts.owner);
        let beat = crate::recover::heartbeat(
            |until| store.refresh_lease(run, owner, until),
            self.parts.clock,
            times,
        );
        // Both boxed: `select` needs `Unpin`, and dropping `select`'s output does not drop a
        // stack-pinned walk (blueprint H-11). A boxed one goes when its box does.
        match futures::future::select(Box::pin(walk), Box::pin(beat)).await {
            Either::Left((out, beat)) => {
                drop(beat);
                // An `Err` writes nothing more: the store may be what failed.
                let out = out?;
                if self.run(run).await?.status != RunStatus::Running {
                    self.release_lease(run).await;
                }
                Ok(out)
            }
            Either::Right((Heartbeat::Abandoned, walk)) => {
                // Before anything else (plan D86): the walk must not write once the lease is gone.
                drop(walk);
                if let Err(err) = self.parts.isolator.release(run).await {
                    tracing::warn!(%run, %err, "releasing the run's guards after an abandon failed");
                }
                Err(EngineError::LeaseLost { run })
            }
        }
    }

    /// Plan D87/D108: `take_lease(run, box, owner, now, now + ttl)`, after a command's pure guard
    /// and before its first write.
    async fn take_lease(&self, run: RunId) -> Result<(), EngineError> {
        let now = self.now();
        let taken = self
            .parts
            .store
            .take_lease(
                run,
                self.parts.box_id,
                self.parts.owner,
                now,
                now + self.lease_times().ttl,
            )
            .await?;
        if taken {
            Ok(())
        } else {
            Err(EngineError::LeaseHeld { run })
        }
    }

    /// Plan D87: `refresh_lease(run, owner, now)`, so the lease reads as expired at once. A
    /// zero-row answer is ignored and an error is warned; neither is raised.
    async fn release_lease(&self, run: RunId) {
        match self
            .parts
            .store
            .refresh_lease(run, self.parts.owner, self.now())
            .await
        {
            // `false`: the lease is no longer ours, so there is nothing of ours to give back.
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(%run, %err, "releasing the lease failed; it expires at its TTL")
            }
        }
    }

    // -- the recovery sweep (ANA-2 §4.9, plan D89-D98, blueprint A-3, A-4, A-7) -----------------

    /// ANA-2 §4.9 `:1284-1307`, plan D98: adopt every `running` run on this box whose lease
    /// expired (never this process's own, D88), adjudicate each one's rows, and **walk nothing**:
    /// the caller walks each [`Next::Walk`] run through [`Self::resume`].
    ///
    /// Runs are recovered one at a time in `queued_at` order, each inside the leased walk
    /// (blueprint A-7), so another orchestrator taking the lease mid-recovery abandons it like any
    /// walk. A run left parked or finished has its lease released. One run's failure does not
    /// stop the sweep (blueprint A-4): it is [`Next::Error`], a `warn`, an `item_note` and a
    /// released lease, and the next run is recovered.
    ///
    /// The heartbeat sleeps on `tokio::time`, so the caller needs a Tokio runtime with the time
    /// driver enabled (blueprint H-3).
    ///
    /// # Errors
    /// Only `adopt_runs`' own store error: nothing was adopted then.
    pub async fn sweep(&self) -> Result<Vec<Adopted>, EngineError> {
        todo!("plan D98")
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
                    self.cleanup_run(run).await?;
                    return Ok(Rest {
                        run: RunStatus::Done,
                        position: None,
                        failure: None,
                    });
                }
                Cursor::Rest { .. } => return self.resting(&row).await,
                Cursor::Create { position, attempt } => {
                    let phase = Self::phase_at(run, &snapshot, position)?;
                    if let Some(rest) = self.admit(&row, &snapshot, &phase, attempt).await? {
                        return Ok(rest);
                    }
                }
                Cursor::Fan { position, attempt } => {
                    let phase = Self::phase_at(run, &snapshot, position)?;
                    if let Some(rest) = self.drive_group(&row, &snapshot, &phase, attempt).await? {
                        return Ok(rest);
                    }
                }
                Cursor::Select { position, attempt } => {
                    let phase = Self::phase_at(run, &snapshot, position)?;
                    if let Some(rest) = self.select_stage(&row, &snapshot, &phase, attempt).await? {
                        return Ok(rest);
                    }
                }
                Cursor::Run(step) => {
                    let Some(step) = steps.into_iter().find(|row| row.id == step) else {
                        continue;
                    };
                    let phase = Self::phase_at(run, &snapshot, step.position)?;
                    if let Some(rest) = self.walk_step(&row, &snapshot, step, &phase).await? {
                        return Ok(rest);
                    }
                }
            }
        }
        Err(EngineError::Stalled {
            run,
            passes: MAX_ITERATIONS,
        })
    }

    /// ANA-2 §12 criterion 3 (`docs/ANA-2.md:2090`): what §4.9's sweep does before it advances a
    /// run it did not start.
    ///
    /// Re-resolves the item's **live** graph and compares its [`graph::topology`] with the digest
    /// the run's own snapshot carries. On a mismatch the walk is not advanced and an `item_note`
    /// records both digests; invariant 2 means the run *could* keep walking its snapshot, and §4.9
    /// says the divergence is a human's decision rather than the engine's.
    ///
    /// A live graph that **refuses to resolve** for a reason that is a rule about starting a run —
    /// a cap an app setting lowered since (plan D63), a phase fanned out where it may not (D64,
    /// ANA-2 §4.6), a phase left with no candidate (§4.1 rung 4), a `touched_paths` entry naming a
    /// repo the project does not carry (plan D81) — has no topology to compare.
    /// That is not this run's business: invariant 2 says it walks its snapshot, so an `item_note`
    /// says the live graph could not be compared and the walk runs. Every other resolution error
    /// is raised.
    ///
    /// **The lease is taken first** (blueprint A-1): for a run the sweep adopted it is a renewal,
    /// for a released parked run it is harmless, and for a run a live stranger holds it is
    /// [`EngineError::LeaseHeld`] with nothing resolved or walked. The walk itself runs under the
    /// heartbeat. Milestone 5's sweep adopts and adjudicates but does not walk; this is the walk
    /// it hands a `Walk` run off to, and milestone 6's `run_worker` is the caller that does.
    /// That heartbeat sleeps on `tokio::time`, so the caller needs a Tokio runtime with the time
    /// driver enabled (blueprint H-3).
    ///
    /// # Errors
    /// [`EngineError::LeaseHeld`], [`EngineError::LeaseLost`], and every other [`EngineError`].
    pub async fn resume(&self, run: RunId) -> Result<Resume, EngineError> {
        self.take_lease(run).await?;
        let row = self.run(run).await?;
        let snapshot = Self::snapshot_of(&row)?;
        let item = self.item(Self::item_of(&row)?).await?;
        let live = match graph::resolve(
            self.parts.store,
            self.parts.graphs,
            &item,
            row.mode,
            &self.parts.app,
            Some(&row.repo_scope),
            self.parts.box_id,
        )
        .await
        {
            Ok(live) => live,
            Err(
                err @ (ResolveError::FanOutCap { .. }
                | ResolveError::AgentCap { .. }
                | ResolveError::ReviewFanOut { .. }
                | ResolveError::LocalFanOut { .. }
                | ResolveError::NoCandidate { .. }
                | ResolveError::UnknownTouchedRepo { .. }),
            ) => {
                let body = format!(
                    "live graph not comparable: run {run} walks its snapshot `{}` \
                     (invariant 2); the live graph does not resolve: {err}",
                    snapshot.topology
                );
                self.note(item.id, body, None, self.now()).await?;
                let rest = self.walk_leased(run, self.run_to_rest(run)).await?;
                return Ok(Resume::Walked(rest));
            }
            Err(err) => return Err(err.into()),
        };

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
            let rest = self.resting(&row).await?;
            // Nothing walks, so the lease just taken is given back unless the run is live.
            if row.status != RunStatus::Running {
                self.release_lease(run).await;
            }
            return Ok(Resume::TopologyChanged {
                snapshot: snapshot.topology,
                live: live.snapshot.topology,
                rest,
            });
        }
        let rest = self.walk_leased(run, self.run_to_rest(run)).await?;
        Ok(Resume::Walked(rest))
    }

    // -- stage 1 -------------------------------------------------------------------------------

    /// **Stage 1 — admit.** `R-AGT-8`'s walk, the selection, the substitution note, the step
    /// rows: one per `fanout_index` of `0..fan_out` (plan D71), so a `fan_out = 1` phase is the
    /// case of a single index.
    ///
    /// `Some(rest)` is the refusal: the walk left nothing the selector would take, the item goes
    /// to `blocked` **before** the run is failed (blueprint H-16 — the reverse order leaves the
    /// item `failed`, because `finish_run` mirrors `in_progress -> failed`), and an `item_note`
    /// records the reason (plan D62). `None` means the `pending` rows exist and the walk may run
    /// them.
    async fn admit(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        attempt: i32,
    ) -> Result<Option<Rest>, EngineError> {
        let indices: Vec<i32> = (0..phase.fan_out.max(1)).collect();
        self.admit_indices(run, snapshot, phase, attempt, &indices)
            .await
    }

    /// [`admit`](Self::admit) for the named `fanout_index`es only: `drive_group` creates the
    /// indices a slot is missing through this, after a crash between two `create_step`s.
    ///
    /// **Every index is selected before any row is written** (blueprint F-K): the selector is
    /// pure, and a `None` for index `i > 0` after index 0 was chosen must refuse the phase through
    /// plan D62's path with no partial group left behind. Each candidate's agent and model are its
    /// own answer's, never a group-level one (plan D71), and D60's substitution note is written per
    /// candidate whose choice passed over a higher-priority row (blueprint H-23).
    async fn admit_indices(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        attempt: i32,
        indices: &[i32],
    ) -> Result<Option<Rest>, EngineError> {
        let walk = self.stage_one(run, snapshot, phase).await?;
        let mut picks = Vec::with_capacity(indices.len());
        for &index in indices {
            let Some(chosen) = self
                .parts
                .selector
                .select(phase, &walk.eligible, index)
                .cloned()
            else {
                return self.refuse_no_candidate(run, phase, &walk).await.map(Some);
            };
            picks.push((index, chosen));
        }

        for (index, chosen) in picks {
            self.note_substitution(run, phase, attempt, index, &chosen, &walk)
                .await?;
            self.parts
                .store
                .create_step(NewRunStep {
                    id: StepId::new(),
                    run_id: run.id,
                    position: phase.position,
                    attempt,
                    fanout_index: index,
                    phase_name: phase.name.clone(),
                    agent_id: Some(chosen.agent_id),
                    model: Some(chosen.model),
                })
                .await?;
        }
        Ok(None)
    }

    /// Plan D60: `select::walk` over the phase's candidates, with its inputs read **fresh on every
    /// call** — the recorder latches quota mid-run (`crates/htui-core/src/model/agent.rs:79-85`),
    /// so a listing taken when the run started would be stale by its second step.
    ///
    /// The `agent` rows come from [`GraphSource::agent`] and the `agent_box` rows from
    /// [`GraphSource::agent_boxes`] for this engine's box; the spend is the run's own
    /// (`select::run_spend`), the cap the snapshot's `per_token_cap_run`, and the minimum
    /// `app_setting.min_budget_for_new_attempt`. The inline-approval interlock is rule 3 of the
    /// walk and still reads `registry::caps_for` from the **agent row**, never from a built driver
    /// (ANA-4 `:554`, plan D6).
    async fn stage_one(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
    ) -> Result<Walk, EngineError> {
        self.walk_candidates(run, snapshot, &phase.candidates, phase.gate_effective)
            .await
    }

    /// `select::walk` over `candidates` under `gate`, read fresh: [`stage_one`](Self::stage_one)'s
    /// body, which the judge's own walk (plan D51) shares with `gate` fixed at `never` — the judge
    /// answers no permission, so the inline-approval interlock does not apply to it.
    async fn walk_candidates(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        candidates: &[SnapshotCandidate],
        gate: Gate,
    ) -> Result<Walk, EngineError> {
        let mut agents = BTreeMap::new();
        for candidate in candidates {
            if agents.contains_key(&candidate.agent_id) {
                continue;
            }
            if let Some(agent) = self.parts.graphs.agent(candidate.agent_id).await? {
                agents.insert(candidate.agent_id, agent);
            }
        }
        let boxes: BTreeMap<_, _> = self
            .parts
            .graphs
            .agent_boxes(self.parts.box_id)
            .await?
            .into_iter()
            .map(|row| (row.agent_id, row))
            .collect();
        let steps = self.parts.store.run_steps(run.id).await?;
        Ok(select::walk(&SelectInput {
            candidates,
            agents: &agents,
            boxes: &boxes,
            gate_effective: gate,
            spent_micros: select::run_spend(&steps),
            cap_micros: snapshot.settings.per_token_cap_run,
            min_budget_micros: min_budget(&self.parts.app),
        }))
    }

    /// Plan D62's stage-1 refusal, in blueprint H-16's order: the item `in_progress -> blocked`,
    /// the note, then `finish_run(Failed)` and plan D36's cleanup.
    ///
    /// Two sentences. A walk whose every skip was the inline-approval interlock keeps the shipped
    /// `missing_capability: inline_approval` (`docs/ANA-2.md:482`), noted with its phase exactly as
    /// before. Anything else is `no_candidate_agent: phase `<p>`; <agent> (<reason>), …`, or, when
    /// the walk skipped nothing and the selector itself declined, that sentence.
    async fn refuse_no_candidate(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        walk: &Walk,
    ) -> Result<Rest, EngineError> {
        let now = self.now();
        let (failure, note) = if walk.only_inline_approval() {
            let failure = RunFailure::MissingCapability;
            let note = format!("{failure} (phase `{}`)", phase.name);
            (failure, note)
        } else {
            let summary = walk.summary();
            let failure = RunFailure::NoCandidateAgent {
                phase: phase.name.clone(),
                detail: if summary.is_empty() {
                    SELECTOR_DECLINED.to_owned()
                } else {
                    summary
                },
            };
            let note = failure.to_string();
            (failure, note)
        };
        if let Some(item) = run.item_id {
            self.parts
                .store
                .transition(item, Status::InProgress, Status::Blocked)
                .await?;
            self.note(item, note, None, now).await?;
        }
        // The item is already `blocked`, so `finish_run`'s item mirror finds no legal move and
        // leaves it — which is the `_ =>` arm of its own table, and the reason the order matters.
        self.parts
            .store
            .finish_run(run.id, RunStatus::Failed, Some(&failure.to_string()), now)
            .await?;
        self.cleanup_run(run.id).await?;
        Ok(Rest {
            run: RunStatus::Failed,
            position: Some(phase.position),
            failure: Some(failure),
        })
    }

    /// Plan D60's "never substitute silently" (`R-ORCH-10`): when the selector's choice ranks below
    /// candidates the walk skipped, one `item_note` names each of them with its reason, and the row
    /// that ran instead. Nothing is written when no skipped row outranked the choice.
    ///
    /// Written per candidate of a group (plan D71, blueprint H-23), so `fanout_index` is in the
    /// sentence.
    async fn note_substitution(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        attempt: i32,
        fanout_index: i32,
        chosen: &SnapshotCandidate,
        walk: &Walk,
    ) -> Result<(), EngineError> {
        let Some(item) = run.item_id else {
            return Ok(());
        };
        let passed_over = skipped_above(phase, walk, chosen);
        if passed_over.is_empty() {
            return Ok(());
        }
        let skipped = passed_over
            .iter()
            .map(|skipped| format!("{} ({})", skipped.agent_name, skipped.cause))
            .collect::<Vec<_>>()
            .join(", ");
        let body = format!(
            "stage 1 at `{}` attempt {attempt} (candidate {fanout_index}): skipped {skipped}; \
             chose {}/{}",
            phase.name, chosen.agent_name, chosen.model
        );
        self.note(item, body, None, self.now()).await
    }

    /// One `item_note` by this engine's user on this engine's box.
    async fn note(
        &self,
        item: ItemId,
        body: String,
        via_step_id: Option<StepId>,
        now: DateTime<Utc>,
    ) -> Result<(), EngineError> {
        self.parts
            .store
            .add_note(NewNote {
                id: NoteId::new(),
                item_id: item,
                body,
                created_by: self.parts.user,
                box_id: Some(self.parts.box_id),
                via_step_id,
                created_at: now,
            })
            .await?;
        Ok(())
    }

    // -- stages 2 to 6 -------------------------------------------------------------------------

    /// Stages 2 to 6 for one `pending` step. `Some(rest)` stops the walk.
    ///
    /// The `pending -> running` move is made here and everything past it is delegated, because
    /// once the step is live **no error may simply propagate**. ANA-2 §4.3 gives a `running` step
    /// exactly one orchestrator-owned exit for "driver error, cap breach, deadline elapsed, spawn
    /// failure": `failed` (`docs/ANA-2.md:639`). A `?` that escaped instead would leave the step
    /// `running` and the run `running` with nothing able to move either — `cursor` rests on a
    /// `running` step, both §6.2 guards refuse one, and no sweep adopts a run whose lease this
    /// process still holds (plan D88). So [`Self::fail_hard`] settles first and the error is
    /// re-raised after.
    async fn walk_step(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        step: RunStep,
        phase: &SnapshotPhase,
    ) -> Result<Option<Rest>, EngineError> {
        let now = self.now();

        // -- stage 2: prepare, beginning with the move that makes the step live ---------------
        if !self
            .parts
            .store
            .transition_step(step.id, StepStatus::Pending, StepStatus::Running, now)
            .await?
        {
            // Another process moved it; the next pass re-derives from rows (plan D16, D17).
            return Ok(None);
        }

        // `transition_step` stamps `started_at = COALESCE(started_at, at)`, so this instant *is*
        // the step's start. Read from the row the walk holds, it would still be `NULL` — the row
        // was read before the move — and the step deadline would be measured from the settle.
        match self.walk_live_step(run, snapshot, &step, phase, now).await {
            Ok(rest) => Ok(rest),
            Err(err) => {
                // `?` and not "report the original": if the settle write itself refused, the step
                // is *still* `running`, and a caller handed the original error would believe a row
                // that does not exist.
                self.fail_hard(run, &step, &err.to_string()).await?;
                Err(err)
            }
        }
    }

    /// Stages 2 to 6 of a step that is already `running`, so every error here is [`fail_hard`]'s.
    ///
    /// [`fail_hard`]: Self::fail_hard
    async fn walk_live_step(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        step: &RunStep,
        phase: &SnapshotPhase,
        started_at: DateTime<Utc>,
    ) -> Result<Option<Rest>, EngineError> {
        let item = Self::item_of(run)?;
        let prepared = self
            .parts
            .isolator
            .prepare(run.id, step.id, &run.repo_scope, phase.isolation, None)
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
            .assemble_prompt(run, snapshot, step, phase, item)
            .await?
        {
            Ok(prompt) => prompt,
            Err(missing) => return self.fail_before_a_token(run, step, phase, missing).await,
        };
        // `unwrap_or(Value::Null)` here wrote a **null** `trim_record` and said nothing: the row
        // that records which sections were dropped and why would silently become "there was no
        // record", which is the one thing `run_step.trim_record` exists to rule out.
        let trim = serde_json::to_value(&prompt.trim).map_err(|err| {
            htui_agent::RecordError::Encode(format!("the step's trim record: {err}"))
        })?;
        self.parts
            .store
            .set_step_prompt(step.id, &prompt.digest, &trim)
            .await?;

        // -- stage 4: session -----------------------------------------------------------------
        let session_cwd = prepared.cwd.clone();
        let (result, cap_breach) = self
            .session(run, step, phase, &prompt, prepared.cwd, prepared.extra_dirs)
            .await?;
        if let Ok(done) = &result {
            self.parts
                .sink
                .after_done(item, step, phase, &SessionKey::of(step), done)
                .await?;
        }

        // -- between stage 4 and stage 5: verify (plan D30, blueprint A-3) ---------------------
        let verify = self
            .verify(VerifyStage {
                run,
                step,
                phase,
                trees: &trees,
                started_at,
                session_cwd: &session_cwd,
                result: &result,
            })
            .await?;

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
            verify_outcome: verify.as_ref().map(|report| report.outcome),
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
                    // `None` leaves these two columns as well, which is what a phase with no
                    // `verify_command` owes: `NULL`, and not `unavailable`.
                    verify_outcome: verify.as_ref().map(|report| report.outcome),
                    verify_exit_code: verify.as_ref().and_then(|report| report.exit_code),
                    finished_at: now,
                },
            )
            .await?;

        // -- stage 6: gate --------------------------------------------------------------------
        let row = self.run(run.id).await?;
        let ctx = self.gate_context(&row, snapshot);
        match gate::apply(&ctx, step, phase, settled).await? {
            // Plan D25: the step is `done`, so the winner's tree is merged into the primary before
            // the walk advances. A refusal parks the run (blueprint H-2, R-7).
            Landing::Advance => {
                let step = self.step(run.id, step.id).await?;
                Ok(self.reconcile_done_step(&row, &step, &[]).await?)
            }
            Landing::Retry { position, attempt } => {
                let phase = Self::phase_at(run.id, snapshot, position)?;
                self.admit(&row, snapshot, &phase, attempt).await
            }
            Landing::Rest(rest) => {
                // `gate.rs` writes its own terminal `finish_run` — `retry_or_fail`'s exhausted
                // budget and `review_loop`'s `NoTarget` — and holds no isolator, so the cleanup
                // that owes it is here (plan D36). A park is not terminal and is not cleaned up.
                if rest.run.is_terminal() {
                    self.cleanup_run(run.id).await?;
                }
                Ok(Some(rest))
            }
        }
    }

    /// `verify_command`, between the session and the settle (`docs/ANA-2.md:491-493`, plan D30).
    ///
    /// **It runs only when the session produced a `Done`** (blueprint A-3). A crashed session is
    /// already `Failed` by settle's first rule, so spending a `cargo test` on a tree the agent
    /// never finished editing records nothing a human wants and delays the failure; on that path
    /// both columns stay `NULL` and no `command_run` row is written, which `run_step.verify_outcome
    /// IS NULL` tells apart from `unavailable`.
    ///
    /// The three outcomes differ only in what they are *recorded as*. `pass` and `fail` are both
    /// `CommandRunStatus::Done` — the command ran, and `run_step.verify_outcome` is where the two
    /// are told apart — while `unavailable` is `CommandRunStatus::Failed`, the row behind a command
    /// that never ran (`:515`). A `fail` is the only one that moves the settle
    /// (`crate::gate::settle`); `unavailable` never fails a step (`:443`).
    ///
    /// `None` — the phase named no command — writes nothing at all. That is the seeded shape of
    /// every phase and it is **not** `unavailable`.
    async fn verify(&self, stage: VerifyStage<'_>) -> Result<Option<VerifyReport>, EngineError> {
        let VerifyStage {
            run,
            step,
            phase,
            trees,
            started_at,
            session_cwd,
            result,
        } = stage;
        if result.is_err() {
            return Ok(None);
        }
        // The primary repo's tree, which is the one ANA-2 `:491-493` names. `repos` is read here
        // rather than carried down the frame because `is_primary` is a property of the repo row
        // and `RunStepTree` carries only the id (blueprint §7.2).
        let repos = self.parts.store.repos(run.project_id).await?;
        let primary = trees.iter().find(|tree| {
            repos
                .iter()
                .any(|repo| repo.id == tree.repo_id && repo.is_primary)
        });

        let report = self
            .parts
            .verifier
            .run(VerifyRequest {
                command: phase.verify_command.clone(),
                cwd: primary.map(|tree| std::path::PathBuf::from(&tree.path)),
                remaining: Self::remaining(phase, started_at, self.now()),
                step: step.id,
            })
            .await;
        let Some(report) = report else {
            return Ok(None);
        };

        self.parts
            .store
            .record_command_run(NewCommandRun {
                id: CommandRunId::new(),
                run_step_id: step.id,
                box_id: self.parts.box_id,
                class: crate::verify::VERIFY_CLASS.to_owned(),
                command: phase.verify_command.clone().unwrap_or_default(),
                // `command_run.cwd` is `NOT NULL` and an `unavailable` report for `no primary
                // tree` has no tree to name, so the session's own directory stands in
                // (blueprint H-23).
                cwd: primary.map_or_else(
                    || session_cwd.to_string_lossy().into_owned(),
                    |tree| tree.path.clone(),
                ),
                status: match report.outcome {
                    VerifyOutcome::Unavailable => CommandRunStatus::Failed,
                    VerifyOutcome::Pass | VerifyOutcome::Fail => CommandRunStatus::Done,
                },
                exit_code: report.exit_code,
                output: Some(report.output.clone()),
                // Every instant is the report's: this milestone writes finished rows only, so the
                // command was queued at the moment it started (plan D31, blueprint §11).
                queued_at: report.started_at,
                started_at: Some(report.started_at),
                finished_at: Some(report.finished_at),
            })
            .await?;
        Ok(Some(report))
    }

    /// What is left of the step deadline at the moment the verify starts (plan D30).
    ///
    /// `None` is "no deadline"; a deadline that has already passed is `Some(ZERO)`, which the
    /// verifier answers `unavailable` to without spawning. A `deadline_seconds` too large for a
    /// `TimeDelta` reads as no deadline, which is `gate::settle`'s own reading of the same column.
    fn remaining(
        phase: &SnapshotPhase,
        started_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<std::time::Duration> {
        let seconds = phase.deadline_seconds?;
        let deadline = started_at + TimeDelta::try_seconds(i64::from(seconds))?;
        Some(
            (deadline - now)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO),
        )
    }

    /// `running -> failed`, then the run, with `reason` as `run.failure` (`docs/ANA-2.md:639`).
    ///
    /// The one shape both of the walk's hard failures use: stage 3's missing input, and any error
    /// escaping the region where a step is live. `false` means the compare-and-set found the step
    /// somewhere other than `running` — another process settled it — and the run is then **not**
    /// failed, because it is no longer this call's to end (plan D17).
    async fn fail_hard(
        &self,
        run: &Run,
        step: &RunStep,
        reason: &str,
    ) -> Result<bool, EngineError> {
        let now = self.now();
        if !self
            .parts
            .store
            .transition_step(step.id, StepStatus::Running, StepStatus::Failed, now)
            .await?
        {
            return Ok(false);
        }
        self.parts
            .store
            .finish_run(run.id, RunStatus::Failed, Some(reason), now)
            .await?;
        // Plan D36's rule is "after every terminal `finish_run`", and this is the one both of the
        // walk's hard failures pass through — stage 3's missing input and any error escaping the
        // region where a step is live. Putting it here rather than at the two call sites is what
        // makes "every" true.
        self.cleanup_run(run.id).await?;
        Ok(true)
    }

    // -- fan-out: the group's drive, its selection and its judge (plan D48-D53, D58, D59) ------

    /// Plan D59's `Fan`: create the slot's missing candidates, then drive every `pending` one
    /// concurrently through stages 2 to 5 (plan D58). `Some(rest)` is a refusal that stopped the
    /// run; `None` means the next pass re-derives — normally `Select`.
    ///
    /// The prompt is assembled **once**, before any candidate is live (plan D58): every candidate
    /// records the same text and `prompt_digest`, and none reads `resolve_inputs` while a sibling
    /// writes its output. A missing required input therefore fails the group before a token is
    /// spent (plan D76, [`fail_group_before_a_token`](Self::fail_group_before_a_token)).
    ///
    /// The group's base is read before any candidate is prepared — from the slot's own
    /// `run_step_commit` rows once one has them (M2 D16), else a retired attempt's, else
    /// `Isolator::base` ([`group_base`](Self::group_base)) — and an error reading it propagates
    /// with the run still `running` at `Fan` (blueprint H-22): nothing is live yet. A store or
    /// isolator error may clear when the next call reads the base again. An
    /// [`EngineError::GroupBase`] refusal does not: it comes from the persisted retired rows alone,
    /// so every later call repeats it until those rows are fixed or the run is cancelled.
    ///
    /// `join_all` rather than a `JoinSet` because every candidate's future borrows `&self` and this
    /// frame's `prompt`, `base` and rows, and `join_all` is awaited here, so no `'static` bound
    /// applies. No `std` guard is held across its `.await`. Only a candidate's own failure write
    /// can raise: [`run_candidate`](Self::run_candidate) owns every other failure, so a failed
    /// sibling never fails the others (plan D48).
    ///
    /// **Dropping this future is the intended abandon** (plan D86). A walk whose lease another
    /// orchestrator took is dropped mid-`join_all`, and every candidate future stops where it
    /// stands: their rows stay `running`, and a `shared_serialized` guard taken at `prepare` stays
    /// held because `capture` never ran. Neither is this frame's to repair. The leased walk drops
    /// this process's guards for the run through `Isolator::release` (plan D99), and the adopting
    /// process's sweep adjudicates the rows the drop left (plan D95). A drop for any other reason
    /// is the same state; [`cleanup_run`](Self::cleanup_run) also releases the guards.
    async fn drive_group(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        attempt: i32,
    ) -> Result<Option<Rest>, EngineError> {
        let steps = self.parts.store.run_steps(run.id).await?;
        let present: Vec<i32> = group_at(&steps, phase.position, attempt)
            .iter()
            .map(|step| step.fanout_index)
            .collect();
        let missing: Vec<i32> = (0..phase.fan_out)
            .filter(|index| !present.contains(index))
            .collect();
        if !missing.is_empty()
            && let Some(rest) = self
                .admit_indices(run, snapshot, phase, attempt, &missing)
                .await?
        {
            return Ok(Some(rest));
        }

        let steps = self.parts.store.run_steps(run.id).await?;
        let slot = group_at(&steps, phase.position, attempt);
        let pending: Vec<&RunStep> = slot
            .iter()
            .copied()
            .filter(|step| step.status == StepStatus::Pending)
            .collect();
        let Some(first) = pending.first() else {
            return Ok(None);
        };
        let base = self
            .group_base(run, &steps, phase.position, attempt)
            .await?;
        let item = Self::item_of(run)?;
        let prompt = match self
            .assemble_prompt(run, snapshot, first, phase, item)
            .await?
        {
            Ok(prompt) => prompt,
            Err(missing) => {
                return self
                    .fail_group_before_a_token(run, phase, &pending, missing)
                    .await
                    .map(Some);
            }
        };

        let settled = futures::future::join_all(pending.iter().map(|step| {
            self.run_candidate(CandidateStage {
                run,
                phase,
                step,
                prompt: &prompt,
                base: &base,
            })
        }))
        .await;
        // Every candidate settled, so every failure is reported: the first is raised, and the
        // rest — which `?` alone would drop — are logged against the run.
        let mut first = None;
        for err in settled.into_iter().filter_map(Result::err) {
            if first.is_none() {
                first = Some(err);
            } else {
                tracing::warn!(run = %run.id, %err, "a further candidate's failure write failed; the first is raised");
            }
        }
        first.map_or(Ok(None), Err)
    }

    /// The group's base per repo (plan D54(b)): the `before_hash`es a candidate of the slot already
    /// recorded, else the base the latest retired attempt at the position started from, else the
    /// repositories' `HEAD`s read once through `Isolator::base`.
    ///
    /// A retried group — plan D65's `retry` or D49(1)'s `never` × `failed` cell — starts where its
    /// retired slot started, because ANA-2 `:754-758` supersedes that slot's winner and losers
    /// alike: a `shared_serialized` slot leaves the checkout at its last sibling's commit, and
    /// `HEAD` there is a loser. A retired attempt no candidate got past `prepare` in recorded no
    /// row and moved nothing, so the one before it is read. A retired slot with a `selected`
    /// winner was reconciled, and the checkout is where that reconcile put it (the review loop,
    /// plan D66), so the search stops there at `HEAD`.
    ///
    /// # Errors
    /// [`EngineError::GroupBase`] when the retired rows name two bases for a repository or none for
    /// one of the scope; every store and isolator error.
    async fn group_base(
        &self,
        run: &Run,
        steps: &[RunStep],
        position: i32,
        attempt: i32,
    ) -> Result<BTreeMap<RepoId, String>, EngineError> {
        for step in group_at(steps, position, attempt) {
            let commits = self.parts.store.step_commits(step.id).await?;
            if !commits.is_empty() {
                return Ok(commits
                    .into_iter()
                    .map(|commit| (commit.repo_id, commit.before_hash))
                    .collect());
            }
        }
        for retired in (1..attempt).rev() {
            let slot = group_at(steps, position, retired);
            if slot.iter().any(|step| step.selected == Some(true)) {
                break;
            }
            let mut bases: BTreeMap<RepoId, String> = BTreeMap::new();
            for step in &slot {
                for commit in self.parts.store.step_commits(step.id).await? {
                    match bases.get(&commit.repo_id) {
                        Some(base) if *base != commit.before_hash => {
                            return Err(EngineError::GroupBase {
                                run: run.id,
                                position,
                                attempt,
                                reason: format!(
                                    "attempt {retired} started repo {} from both {base} and {}",
                                    commit.repo_id, commit.before_hash
                                ),
                            });
                        }
                        Some(_) => {}
                        None => {
                            bases.insert(commit.repo_id, commit.before_hash);
                        }
                    }
                }
            }
            if bases.is_empty() {
                continue;
            }
            if let Some(repo) = run.repo_scope.iter().find(|repo| !bases.contains_key(repo)) {
                return Err(EngineError::GroupBase {
                    run: run.id,
                    position,
                    attempt,
                    reason: format!("attempt {retired} recorded no base for repo {repo}"),
                });
            }
            return Ok(bases);
        }
        Ok(self.parts.isolator.base(&run.repo_scope).await?)
    }

    /// Plan D76 (blueprint A-5): a required input missing from the group's one prompt. No
    /// candidate is live yet, so each `pending` one is moved `pending -> running -> failed` with
    /// a note — `pending -> failed` is illegal (`model/run.rs:113`) — and then the run fails
    /// exactly as a `fan_out = 1` step's missing input fails it.
    async fn fail_group_before_a_token(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        pending: &[&RunStep],
        kind: String,
    ) -> Result<Rest, EngineError> {
        let failure = RunFailure::MissingInput(kind);
        let now = self.now();
        for step in pending {
            if self
                .parts
                .store
                .transition_step(step.id, StepStatus::Pending, StepStatus::Running, now)
                .await?
            {
                self.fail_candidate(run, phase, step, &failure.to_string())
                    .await?;
            }
        }
        self.parts
            .store
            .finish_run(run.id, RunStatus::Failed, Some(&failure.to_string()), now)
            .await?;
        self.cleanup_run(run.id).await?;
        Ok(Rest {
            run: RunStatus::Failed,
            position: Some(phase.position),
            failure: Some(failure),
        })
    }

    /// One candidate's stages 2 to 5 (plan D48): it lands `done` or `failed` on its own and never
    /// parks, and an error once it is live fails **that candidate** — never the run, and never its
    /// siblings. The only error raised is a failure write that itself failed.
    ///
    /// A candidate that fails after `prepare` succeeded and before its `capture` ran is captured
    /// best-effort on the way out (plan D78, blueprint A-7): a `shared_serialized` guard is released
    /// only at `capture`, and D48 removed the `fail_hard -> cleanup_run` path that used to release
    /// it, so without this the siblings in the same `join_all` would wait on it forever.
    async fn run_candidate(&self, stage: CandidateStage<'_>) -> Result<(), EngineError> {
        let started_at = self.now();
        if !self
            .parts
            .store
            .transition_step(
                stage.step.id,
                StepStatus::Pending,
                StepStatus::Running,
                started_at,
            )
            .await?
        {
            return Ok(());
        }
        let mut trees = None;
        let mut captured = false;
        match self.candidate_live(&stage, &mut trees, &mut captured).await {
            Ok(()) => Ok(()),
            Err(err) => {
                if let (Some(trees), false) = (&trees, captured) {
                    self.release_trees(stage.step, trees).await;
                }
                self.fail_candidate(stage.run, stage.phase, stage.step, &err.to_string())
                    .await
            }
        }
    }

    /// Plan D78's best-effort capture: its commits are recorded when it answers, and every failure
    /// — the isolator's or the write's — is logged, because the candidate is failing already.
    async fn release_trees(&self, step: &RunStep, trees: &[htui_core::model::RunStepTree]) {
        match self.parts.isolator.capture(step.id, trees).await {
            Ok(after) => {
                if let Err(err) = self.parts.store.record_commits(step.id, &after).await {
                    tracing::warn!(step = %step.id, %err, "a failed candidate's commits were not recorded");
                }
            }
            Err(err) => {
                tracing::warn!(step = %step.id, %err, "a failed candidate's trees were not captured");
            }
        }
    }

    /// The live half of [`run_candidate`](Self::run_candidate). `trees` is set the moment `prepare`
    /// answered and `captured` the moment `capture` did, so the caller knows what to release.
    async fn candidate_live(
        &self,
        stage: &CandidateStage<'_>,
        trees: &mut Option<Vec<htui_core::model::RunStepTree>>,
        captured: &mut bool,
    ) -> Result<(), EngineError> {
        let CandidateStage {
            run,
            phase,
            step,
            prompt,
            base,
        } = *stage;
        let item = Self::item_of(run)?;

        // -- stage 2: prepare, from the group's base (plan D54(a)) ----------------------------
        let prepared = self
            .parts
            .isolator
            .prepare(
                run.id,
                step.id,
                &run.repo_scope,
                phase.isolation,
                Some(FanoutSlot {
                    index: step.fanout_index,
                    width: phase.fan_out,
                    base,
                }),
            )
            .await?;
        // The candidate's own clock starts here, not at its `running` move: a `shared_serialized`
        // `prepare` waits on the per-repo lock until the sibling before it is captured, and a
        // deadline that counted that wait would charge the last sibling for every session ahead
        // of it. `run_step.started_at` keeps the `running` instant.
        let started_at = self.now();
        let rows: Vec<_> = prepared
            .trees
            .iter()
            .map(|tree| tree.tree.clone())
            .collect();
        *trees = Some(rows.clone());
        self.parts.store.upsert_step_tree(step.id, &rows).await?;
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

        // -- stage 3: the group's one prompt (plan D58) ----------------------------------------
        let trim = serde_json::to_value(&prompt.trim).map_err(|err| {
            htui_agent::RecordError::Encode(format!("the step's trim record: {err}"))
        })?;
        self.parts
            .store
            .set_step_prompt(step.id, &prompt.digest, &trim)
            .await?;

        // -- stage 4: this candidate's own session (plan D68) ---------------------------------
        let key = SessionKey::of(step);
        let mut recorder = self.open_recorder(run, step, prompt).await?;
        let result = match self
            .drive_once(
                run,
                step,
                phase,
                &key,
                &prompt.text,
                prepared.cwd.clone(),
                prepared.extra_dirs,
                &mut recorder,
            )
            .await
        {
            Ok(result) => result,
            Err(refused) => {
                recorder.finish().await?;
                return Err(refused);
            }
        };
        let cap_breach = recorder.finish().await?.cap_breach;
        if let Ok(done) = &result {
            self.parts
                .sink
                .after_done(item, step, phase, &key, done)
                .await?;
        }
        let verify = self
            .verify(VerifyStage {
                run,
                step,
                phase,
                trees: &rows,
                started_at,
                session_cwd: &prepared.cwd,
                result: &result,
            })
            .await?;

        // -- stage 5: settle on the candidate's own terms (plan D48) ---------------------------
        let after = self.parts.isolator.capture(step.id, &rows).await?;
        *captured = true;
        self.parts.store.record_commits(step.id, &after).await?;
        let output = self.output_of(item, phase, step.id).await?;
        let now = self.now();
        // `verify_outcome: None`: a candidate's verify is the prefilter's to read (plan D49), not
        // its settle's — a `fail` here would make criterion 9's surviving candidate unselectable.
        let settled = gate::settle(&SettleInput {
            driver: &result,
            cap_breach,
            started_at,
            now,
            deadline_seconds: phase.deadline_seconds,
            output: output.as_ref(),
            verify_outcome: None,
            is_review: phase.name == REVIEW_PHASE,
        });
        self.parts
            .store
            .finish_step(
                step.id,
                StepOutcome {
                    exit_code: None,
                    usage: None,
                    trim_record: None,
                    // The **real** outcome, which is what the prefilter reads.
                    verify_outcome: verify.as_ref().map(|report| report.outcome),
                    verify_exit_code: verify.as_ref().and_then(|report| report.exit_code),
                    finished_at: now,
                },
            )
            .await?;
        match settled {
            Settle::Ok { note } => {
                self.parts
                    .store
                    .transition_step(step.id, StepStatus::Running, StepStatus::Done, now)
                    .await?;
                if let Some(note) = note {
                    self.note(item, note, Some(step.id), now).await?;
                }
                Ok(())
            }
            Settle::Failed(failure) => {
                self.fail_candidate(run, phase, step, &failure.to_string())
                    .await
            }
            // Plan D64 refuses `fan_out > 1` on `review` at resolution, so no candidate reads a
            // verdict; were one to, a rejection is still not a candidate's to act on.
            Settle::Rejected { verdict_line } => {
                self.fail_candidate(run, phase, step, &verdict_line).await
            }
        }
    }

    /// One candidate `running -> failed`, with the reason as an `item_note` naming it (plan D48).
    /// A step another process already moved is left alone and gets no note (plan D17).
    async fn fail_candidate(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        step: &RunStep,
        reason: &str,
    ) -> Result<(), EngineError> {
        let now = self.now();
        if !self
            .parts
            .store
            .transition_step(step.id, StepStatus::Running, StepStatus::Failed, now)
            .await?
        {
            return Ok(());
        }
        let Some(item) = run.item_id else {
            return Ok(());
        };
        self.note(
            item,
            format!(
                "fan-out candidate {} of `{}` attempt {}: {reason}",
                step.fanout_index, phase.name, step.attempt
            ),
            Some(step.id),
            now,
        )
        .await
    }

    /// Plan D59's `Select`: route a settled group (plan D49) and land it.
    ///
    /// A judge row already in the slot is a crash's leftover: a `failed` one re-parks with its
    /// own reason and is **not** re-run (D59), and a `pending` one is run with the passing set
    /// recomputed. Otherwise `fanout::route` decides — the group's retry or failure under `never`,
    /// D50's park, the one passing candidate's win (with plan D77's note), or the judge.
    async fn select_stage(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        attempt: i32,
    ) -> Result<Option<Rest>, EngineError> {
        let steps = self.parts.store.run_steps(run.id).await?;
        let slot: Vec<RunStep> = group_at(&steps, phase.position, attempt)
            .into_iter()
            .cloned()
            .collect();
        let views: Vec<CandidateView> = slot.iter().map(CandidateView::of).collect();
        let pre = prefilter(&views);

        if let Some(judge) = judge_at(&steps, phase.position, attempt) {
            match judge.status {
                StepStatus::Failed | StepStatus::AwaitingApproval => {
                    let reason = judge.gate_note.as_deref().unwrap_or(JUDGE_FAILED);
                    return self
                        .park_selection(run, phase, attempt, &slot, reason)
                        .await
                        .map(Some);
                }
                StepStatus::Pending => {
                    let passing = pre.passing.iter().map(|view| view.step).collect();
                    return self
                        .run_judge(run, snapshot, phase, attempt, passing, Some(judge.clone()))
                        .await;
                }
                StepStatus::Running
                | StepStatus::Done
                | StepStatus::Superseded
                | StepStatus::Cancelled => {}
            }
        }

        match route(phase.gate_effective, phase.judge.is_some(), &pre) {
            Route::GroupFailed => {
                let now = self.now();
                if may_attempt(attempt + 1, phase.retry_limit) {
                    // §4.2's `never` × `failed` cell for the whole group: retire it, and the
                    // cursor creates `attempt + 1` (plan D49(1)).
                    gate::retire_slot(
                        &self.gate_context(run, snapshot),
                        &steps,
                        phase.position,
                        now,
                    )
                    .await?;
                    return Ok(None);
                }
                let failure = RunFailure::NoSurvivingCandidate {
                    phase: phase.name.clone(),
                };
                self.parts
                    .store
                    .finish_run(run.id, RunStatus::Failed, Some(&failure.to_string()), now)
                    .await?;
                self.cleanup_run(run.id).await?;
                Ok(Some(Rest {
                    run: RunStatus::Failed,
                    position: Some(phase.position),
                    failure: Some(failure),
                }))
            }
            Route::Human(reason) => self
                .park_selection(run, phase, attempt, &slot, &reason.to_string())
                .await
                .map(Some),
            Route::AutoWin(winner) => {
                // The note before the selection (plan D77): once `select_fanout` lands, no later
                // pass routes this slot again, so a crash between the two writes would lose the
                // reason for good. This way round it costs, at worst, the note written twice.
                let index = slot
                    .iter()
                    .find(|step| step.id == winner)
                    .map_or(0, |step| step.fanout_index);
                self.note_selection(
                    run,
                    winner,
                    format!(
                        "fan-out `{}` attempt {attempt}: candidate {index} wins as {AUTO_WIN_REASON}",
                        phase.name
                    ),
                    self.now(),
                )
                .await?;
                self.parts
                    .store
                    .select_fanout(
                        run.id,
                        phase.position,
                        attempt,
                        winner,
                        Some(AUTO_WIN_REASON.to_owned()),
                    )
                    .await?;
                self.reconcile_winner(run, &slot, winner).await
            }
            Route::Judge(passing) => {
                self.run_judge(run, snapshot, phase, attempt, passing, None)
                    .await
            }
        }
    }

    /// The selected winner, re-read, reconciled with its slot's other candidates (plan D54(d)).
    async fn reconcile_winner(
        &self,
        run: &Run,
        slot: &[RunStep],
        winner: StepId,
    ) -> Result<Option<Rest>, EngineError> {
        let refs: Vec<&RunStep> = slot.iter().collect();
        let siblings = Self::siblings(&refs, winner);
        let winner = self.step(run.id, winner).await?;
        self.reconcile_done_step(run, &winner, &siblings).await
    }

    /// Plan D50's one park shape for a human selection: every candidate as it settled, `selected`
    /// NULL, the run `running -> awaiting_approval`, the item `in_progress -> awaiting_approval`,
    /// and one `item_note` naming the phase, the attempt, each candidate and the reason.
    ///
    /// No step is parked — `park_run`'s deviation from §4.3's propagation rule, for the same
    /// reason — and `run.failure` stays NULL (R-3). A `shared_serialized` group's note says where
    /// the maintainer's checkout was left: at the last sibling's commit, with the base and every
    /// sibling's label named (the plan's Risks row).
    async fn park_selection(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        attempt: i32,
        slot: &[RunStep],
        reason: &str,
    ) -> Result<Rest, EngineError> {
        let now = self.now();
        self.parts
            .store
            .transition_run(run.id, RunStatus::Running, RunStatus::AwaitingApproval, now)
            .await?;
        if let Some(item) = run.item_id {
            self.parts
                .store
                .transition(item, Status::InProgress, Status::AwaitingApproval)
                .await?;
            let candidates = slot
                .iter()
                .map(|step| {
                    format!(
                        "{} {} (verify {})",
                        step.fanout_index,
                        step.status,
                        step.verify_outcome.map_or("none", VerifyOutcome::as_str)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let mut body = format!(
                "fan-out `{}` attempt {attempt} awaits selection: {reason}; candidates: {candidates}",
                phase.name
            );
            if phase.isolation == Isolation::SharedSerialized {
                body.push_str(&self.shared_checkout_note(slot).await?);
            }
            self.note(item, body, None, now).await?;
        }
        Ok(Rest {
            run: RunStatus::AwaitingApproval,
            position: Some(phase.position),
            failure: None,
        })
    }

    /// The park note's `shared_serialized` sentence: where the checkout was left, and the base and
    /// each sibling's `htui/<step>` label, which are where a human finds the candidates.
    ///
    /// Only a sibling whose `capture` moved the checkout has a label (M3 D26 labels at capture, and
    /// only a moved `HEAD`), so only those are named. When none did — every sibling refused at
    /// `prepare`, say, on a dirty checkout (plan D72) — the checkout is where the group found it
    /// and the sentence says so instead of naming commits and branches `git` does not have.
    ///
    /// The checkout is at the last sibling's commit only when that sibling moved it. The last
    /// sibling that got past `prepare` (the one with `step_commit` rows) had it reset to the base
    /// (D56); if it then committed nothing — it failed before committing, say — the checkout is
    /// back at the base, and the sentence says that while still naming the earlier labels.
    async fn shared_checkout_note(&self, slot: &[RunStep]) -> Result<String, EngineError> {
        let mut ordered: Vec<&RunStep> = slot.iter().collect();
        ordered.sort_by_key(|step| step.fanout_index);
        let mut bases: Vec<String> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        let mut last_moved = false;
        for step in ordered {
            let commits = self.parts.store.step_commits(step.id).await?;
            if commits.is_empty() {
                continue;
            }
            let mut moved = false;
            for commit in commits {
                let base = format!("{}@{}", commit.repo_id, commit.before_hash);
                if !bases.contains(&base) {
                    bases.push(base);
                }
                moved |= commit.after_hash.is_some();
            }
            if moved {
                labels.push(format!("htui/{}", step.id));
            }
            last_moved = moved;
        }
        if labels.is_empty() {
            return Ok("; no sibling moved the shared checkout".to_owned());
        }
        let at = if last_moved {
            "stays at the last sibling's commit"
        } else {
            "is back at the base"
        };
        let labels = labels.join(", ");
        Ok(format!(
            "; the shared checkout {at} (base {}; labels {labels})",
            bases.join(", ")
        ))
    }

    /// The judge (plan D51-D53): its two prompts, its row, its two sessions, its verdict.
    ///
    /// **The order is blueprint H-8's.** The inputs and both prompts come first and no row is
    /// written for them; a candidate the trimmer dropped from the forward prompt sends the group to
    /// D50's park with no judge row at all (D53). Only then is the judge created — or `existing`,
    /// a crash's `pending` leftover, reused — walked, and moved `pending -> running`. Every failure
    /// from there on (an input that could not be built, an agent the walk skips, a session error,
    /// a verdict that does not parse, is out of range or disagrees) is [`fail_judge`]'s: the judge
    /// `failed` with the reason as its `gate_note`, and the group parked for a human.
    ///
    /// [`fail_judge`]: Self::fail_judge
    async fn run_judge(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        attempt: i32,
        passing: Vec<StepId>,
        existing: Option<RunStep>,
    ) -> Result<Option<Rest>, EngineError> {
        let steps = self.parts.store.run_steps(run.id).await?;
        let slot: Vec<RunStep> = group_at(&steps, phase.position, attempt)
            .into_iter()
            .cloned()
            .collect();
        let mut survivors: Vec<&RunStep> = slot
            .iter()
            .filter(|step| passing.contains(&step.id))
            .collect();
        survivors.sort_by_key(|step| step.fanout_index);

        // -- 1. inputs and both orderings, before any row (H-8) --------------------------------
        let prompts = self.judge_prompts(run, phase, attempt, &survivors).await?;
        if let Ok(JudgePrompts { forward, .. }) = &prompts
            && let Some(index) = dropped_candidate(forward)
        {
            let reason = HumanReason::CandidateDropped(index).to_string();
            return self
                .park_selection(run, phase, attempt, &slot, &reason)
                .await
                .map(Some);
        }

        // -- 2. the row, and the judge's own walk (D51, F-I) ----------------------------------
        let (judge, candidate) = self
            .judge_row(run, snapshot, phase, attempt, existing)
            .await?;
        let now = self.now();
        if !self
            .parts
            .store
            .transition_step(judge.id, StepStatus::Pending, StepStatus::Running, now)
            .await?
        {
            return Ok(None);
        }
        let prompts = match prompts {
            Ok(prompts) => prompts,
            Err(failure) => {
                return self
                    .fail_judge(run, phase, attempt, &judge, &slot, failure)
                    .await
                    .map(Some);
            }
        };
        let candidate = match candidate {
            Ok(candidate) => candidate,
            Err(failure) => {
                return self
                    .fail_judge(run, phase, attempt, &judge, &slot, failure)
                    .await
                    .map(Some);
            }
        };

        // -- 3. two fresh sessions, the second reversed (D52) ---------------------------------
        let indices: Vec<i32> = survivors.iter().map(|step| step.fanout_index).collect();
        let verdict = match self
            .judge_sessions(run, phase, attempt, &judge, &candidate, &prompts)
            .await
        {
            Ok(Ok(documents)) => decide(&documents, &indices),
            Ok(Err(failure)) => Err(failure),
            Err(err) => Err(JudgeFailure::SessionFailed(err.to_string())),
        };
        let (winner, reason) = match verdict {
            Ok(verdict) => verdict,
            Err(failure) => {
                return self
                    .fail_judge(run, phase, attempt, &judge, &slot, failure)
                    .await
                    .map(Some);
            }
        };

        // -- 4. the winner: `select_fanout` moves the judge `running -> done` with the reason --
        let Some(winner) = survivors
            .iter()
            .find(|step| step.fanout_index == winner)
            .map(|step| step.id)
        else {
            // `decide` range-checked it against these same rows.
            return Ok(None);
        };
        // The judge is `running` here, so an escaped `?` would leave it and the run `running`
        // forever: a store error is `fail_judge`'s like any other, and is still raised.
        if let Err(err) = self
            .settle_judge(run, phase, attempt, &judge, winner, reason)
            .await
        {
            if let Err(also) = self
                .fail_judge(
                    run,
                    phase,
                    attempt,
                    &judge,
                    &slot,
                    JudgeFailure::SessionFailed(err.to_string()),
                )
                .await
            {
                tracing::warn!(step = %judge.id, %also, "a judge whose settle failed was not parked");
            }
            return Err(err);
        }
        self.reconcile_winner(run, &slot, winner).await
    }

    /// Step 4 of [`run_judge`](Self::run_judge): the judge's settle, then `select_fanout` moving
    /// it `running -> done` beside the winner with the reason.
    async fn settle_judge(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        attempt: i32,
        judge: &RunStep,
        winner: StepId,
        reason: String,
    ) -> Result<(), EngineError> {
        let now = self.now();
        self.parts
            .store
            .finish_step(
                judge.id,
                StepOutcome {
                    exit_code: None,
                    usage: None,
                    trim_record: None,
                    verify_outcome: None,
                    verify_exit_code: None,
                    finished_at: now,
                },
            )
            .await?;
        self.parts
            .store
            .select_fanout(run.id, phase.position, attempt, winner, Some(reason))
            .await?;
        Ok(())
    }

    /// The judge's inputs and both orderings (plan D53), with nothing written.
    ///
    /// `task` is the seq-0 `prompt` text the lowest-index survivor recorded — replayed, not
    /// re-assembled (ANA-5 `:1256-1263`). One [`JudgeCandidate`] per survivor: its verify outcome
    /// and exit code, its output document, `Isolator::diff` over its rows (an error is a trim note,
    /// the diff is advisory, D55) and its last `command_run` output. The template is the snapshot's
    /// pinned one, else the latest `judge` row (D53); the budget is the judged phase's; every other
    /// section is empty, and `phase` renders as the **judged** phase's name (blueprint F-G).
    ///
    /// The inner `Err` is a [`JudgeFailure`] the caller lands on a judge row: a candidate with no
    /// recorded prompt or an assembly refusal is `judge_session_failed`, and no template at all is
    /// `judge_unavailable`.
    async fn judge_prompts(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        attempt: i32,
        survivors: &[&RunStep],
    ) -> Result<Result<JudgePrompts, JudgeFailure>, EngineError> {
        let item = Self::item_of(run)?;
        let row = self.item(item).await?;
        let project = self.project(row.project_id).await?;
        let mut notes = Vec::new();

        let Some(first) = survivors.first() else {
            return Ok(Err(JudgeFailure::SessionFailed(
                "no candidate to judge".to_owned(),
            )));
        };
        let task = self
            .parts
            .store
            .step_events(first.id)
            .await?
            .unwrap_or_default()
            .into_iter()
            .find(|event| event.seq == 0 && event.kind == EventKind::Prompt)
            .and_then(|event| {
                event
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        let Some(task) = task else {
            return Ok(Err(JudgeFailure::SessionFailed(format!(
                "candidate {} recorded no prompt to replay as the task",
                first.fanout_index
            ))));
        };

        let mut candidates = Vec::with_capacity(survivors.len());
        for step in survivors {
            let document =
                self.output_of(item, phase, step.id)
                    .await?
                    .map(|document| InputDocument {
                        kind: document.kind,
                        version: document.version,
                        body: document.body,
                    });
            let trees = self.parts.store.step_trees(step.id).await?;
            let commits = self.parts.store.step_commits(step.id).await?;
            let diff = match self.parts.isolator.diff(&trees, &commits).await {
                Ok(diff) => diff,
                Err(err) => {
                    notes.push(format!(
                        "judge_candidate:{} diff unavailable: {err}",
                        step.fanout_index
                    ));
                    None
                }
            };
            let verification_tail = self
                .parts
                .store
                .command_runs(step.id)
                .await?
                .pop()
                .and_then(|row| row.output);
            candidates.push(JudgeCandidate {
                fanout_index: step.fanout_index,
                verify: match step.verify_outcome {
                    Some(VerifyOutcome::Pass) => Some(true),
                    Some(VerifyOutcome::Fail) => Some(false),
                    Some(VerifyOutcome::Unavailable) | None => None,
                },
                exit_code: step.verify_exit_code,
                document,
                diff,
                verification_tail,
            });
        }

        let pinned = phase
            .judge
            .as_ref()
            .and_then(|judge| judge.template.as_ref());
        let template = match pinned {
            Some(pinned) => {
                self.parts
                    .graphs
                    .prompt_template(project.id, &pinned.name, Some(pinned.version))
                    .await?
            }
            None => {
                self.parts
                    .graphs
                    .prompt_template(project.id, JUDGE_TEMPLATE, None)
                    .await?
            }
        };
        let Some(template) = template else {
            return Ok(Err(JudgeFailure::Unavailable(format!(
                "no `{JUDGE_TEMPLATE}` template"
            ))));
        };

        let kind = self.item_kind_name(&row).await?;
        let (caps, _scan, _deadline) = settings::resolve_excerpt_caps(&self.parts.app);
        let spec = |reverse: bool| PromptSpec {
            role: TemplateRole::of_name(&template.name),
            template: TemplateRef {
                name: template.name.clone(),
                version: template.version,
            },
            body: template.body.clone(),
            item_key: format!("{}:{}", project.slug, row.key),
            item_title: row.title.clone(),
            item_kind: kind.clone(),
            item_body: row.body.clone(),
            // Blueprint F-G: the judge body renders `{{phase}}` as the phase being judged; only
            // the step row carries `<phase>:judge`.
            phase: phase.name.clone(),
            output_kind: Some(JUDGE_KIND.to_owned()),
            attempt,
            documents: Vec::new(),
            upstream: Vec::new(),
            box_profile: self.parts.box_profile.clone(),
            skills: Vec::new(),
            excerpts: no_excerpts(caps),
            command_queue: false,
            verify_failure: None,
            previous_diff: None,
            judge: Some(judge_inputs(task.clone(), candidates.clone(), reverse)),
            handoff: None,
            budget: settings::resolve_budget(
                phase.token_budget,
                Some(&project.settings),
                &self.parts.app,
            ),
            max_skill_tokens: settings::resolve_max_skill_tokens(&self.parts.app),
            estimator: TokenEstimator::DEFAULT,
            notes: notes.clone(),
        };
        // The assembler reverses the candidates itself (`prompt/mod.rs:573-580`), so both calls
        // hand it the same list and differ only in `reverse` (plan D53).
        let assembled = assemble(&spec(false), self.parts.scrubber)
            .and_then(|forward| Ok((forward, assemble(&spec(true), self.parts.scrubber)?)));
        Ok(match assembled {
            Ok((forward, reversed)) => Ok(JudgePrompts {
                forward,
                reversed,
                template: SnapshotTemplate {
                    name: template.name.clone(),
                    version: template.version,
                },
            }),
            Err(err) => Err(JudgeFailure::SessionFailed(err.to_string())),
        })
    }

    /// The judge's row and the candidate it runs as (plan D51, blueprint F-I).
    ///
    /// `existing` is reused as it is. Otherwise the row is created at `fanout_index = -1`, named
    /// `<phase>:judge`, whatever the judge's walk says — a judge the walk skips is still created and
    /// then failed, so the reason is on a row. The inner `Err` is that reason: no `agent` row (the
    /// row is then created with no agent, which `create_step` would otherwise refuse), no model
    /// to run, or the walk's own skip with the phase's gate read as `never`, because the judge
    /// answers no permission.
    async fn judge_row(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        phase: &SnapshotPhase,
        attempt: i32,
        existing: Option<RunStep>,
    ) -> Result<(RunStep, Result<SnapshotCandidate, JudgeFailure>), EngineError> {
        let Some(judge) = phase.judge.as_ref() else {
            let failure = JudgeFailure::Unavailable(HumanReason::NoJudge.to_string());
            let row = match existing {
                Some(row) => row,
                None => self.create_judge(run, phase, attempt, None, None).await?,
            };
            return Ok((row, Err(failure)));
        };
        let agent = self.parts.graphs.agent(judge.agent_id).await?;
        let (agent_id, model, choice) = match &agent {
            None => (
                None,
                None,
                Err(JudgeFailure::Unavailable("no agent row".to_owned())),
            ),
            Some(agent) => match judge_candidate(judge, agent) {
                None => (
                    Some(agent.id),
                    None,
                    Err(JudgeFailure::Unavailable("no model".to_owned())),
                ),
                Some(candidate) => {
                    let walk = self
                        .walk_candidates(
                            run,
                            snapshot,
                            core::slice::from_ref(&candidate),
                            Gate::Never,
                        )
                        .await?;
                    let model = Some(candidate.model.clone());
                    let choice = match walk.skipped.first() {
                        Some(skipped) => Err(JudgeFailure::Unavailable(skipped.cause.to_string())),
                        None => Ok(candidate),
                    };
                    (Some(agent.id), model, choice)
                }
            },
        };
        let row = match existing {
            Some(row) => row,
            None => {
                self.create_judge(run, phase, attempt, agent_id, model)
                    .await?
            }
        };
        Ok((row, choice))
    }

    /// `create_step` for a judge: `fanout_index = -1` at the group's slot, `<phase>:judge`.
    async fn create_judge(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        attempt: i32,
        agent_id: Option<htui_core::model::AgentId>,
        model: Option<String>,
    ) -> Result<RunStep, EngineError> {
        Ok(self
            .parts
            .store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run.id,
                position: phase.position,
                attempt,
                fanout_index: JUDGE_FANOUT_INDEX,
                phase_name: judge_phase_name(&phase.name),
                agent_id,
                model,
            })
            .await?)
    }

    /// The judge's two sessions under one recorder (plan D52), and the document each wrote.
    ///
    /// The judge runs in no tree: `prepare(…, &[], Local, None)` gives a scratch `cwd` and no row
    /// (D51). The forward text is `record_prompt` at seq 0 — `prompt_digest` and `set_step_prompt`
    /// are the forward prompt's — and the reversed text is a `follow_up` opening turn 1. The
    /// recorder is finished on every path; a cap breach over the two is `judge_session_failed`.
    async fn judge_sessions(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        attempt: i32,
        judge: &RunStep,
        candidate: &SnapshotCandidate,
        prompts: &JudgePrompts,
    ) -> Result<Result<[Document; 2], JudgeFailure>, EngineError> {
        let prepared = self
            .parts
            .isolator
            .prepare(run.id, judge.id, &[], Isolation::Local, None)
            .await?;
        let trim = serde_json::to_value(&prompts.forward.trim).map_err(|err| {
            htui_agent::RecordError::Encode(format!("the judge's trim record: {err}"))
        })?;
        self.parts
            .store
            .set_step_prompt(judge.id, &prompts.forward.digest, &trim)
            .await?;
        let jp = judge_phase(phase, prompts.template.clone(), candidate);

        let mut recorder = self.open_recorder(run, judge, &prompts.forward).await?;
        let calls = self
            .judge_calls(run, &jp, attempt, judge, prompts, &prepared, &mut recorder)
            .await;
        let finished = recorder.finish().await;
        let documents = match calls? {
            Ok(documents) => documents,
            Err(failure) => return Ok(Err(failure)),
        };
        if let Some(breach) = finished?.cap_breach {
            return Ok(Err(JudgeFailure::SessionFailed(format!(
                "{} ({breach:?})",
                gate::StepFailure::CapBreached
            ))));
        }
        Ok(Ok(documents))
    }

    /// Call 0 and call 1 of [`judge_sessions`](Self::judge_sessions): each drives a fresh session
    /// keyed `(<phase>:judge, attempt, -1, call)`, hands its `done` to the sink, and reads the
    /// newest `judge` document the judge step wrote — which must be **new** after each call.
    #[expect(
        clippy::too_many_arguments,
        reason = "the judge's row and phase, its two prompts, its tree and the recorder both calls \
                  share; bundling them would name a struct used here only"
    )]
    async fn judge_calls(
        &self,
        run: &Run,
        jp: &SnapshotPhase,
        attempt: i32,
        judge: &RunStep,
        prompts: &JudgePrompts,
        prepared: &crate::isolate::Prepared,
        recorder: &mut Recorder<'a, S>,
    ) -> Result<Result<[Document; 2], JudgeFailure>, EngineError> {
        let item = Self::item_of(run)?;
        let mut documents: Vec<Document> = Vec::with_capacity(2);
        for (call, text) in [(0, &prompts.forward.text), (1, &prompts.reversed.text)] {
            if call > 0 {
                recorder.record_follow_up(text, self.now()).await?;
            }
            let key = SessionKey {
                phase: &judge.phase_name,
                attempt,
                fanout_index: judge.fanout_index,
                call,
            };
            let done = match self
                .drive_once(
                    run,
                    judge,
                    jp,
                    &key,
                    text,
                    prepared.cwd.clone(),
                    prepared.extra_dirs.clone(),
                    recorder,
                )
                .await?
            {
                Ok(done) => done,
                Err(err) => return Ok(Err(JudgeFailure::SessionFailed(err.to_string()))),
            };
            self.parts
                .sink
                .after_done(item, judge, jp, &key, &done)
                .await?;
            let document = self.output_of(item, jp, judge.id).await?;
            match document {
                Some(document) if !documents.iter().any(|seen| seen.id == document.id) => {
                    documents.push(document);
                }
                _ => return Ok(Err(JudgeFailure::MissingDocument { call })),
            }
        }
        let [forward, reversed]: [Document; 2] =
            documents.try_into().map_err(|_| EngineError::Snapshot {
                run: run.id,
                reason: "the judge's two calls did not yield two documents".to_owned(),
            })?;
        Ok(Ok([forward, reversed]))
    }

    /// Plan D51's failure: the judge `running -> awaiting_approval`, then `answer_gate(Rejected,
    /// reason)` → `failed` with the reason as its `gate_note` — the two writes `gate::apply` uses
    /// for an automatic review rejection, and the only way a `gate_note` reaches a failed row —
    /// then D50's park, naming the same reason.
    async fn fail_judge(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        attempt: i32,
        judge: &RunStep,
        slot: &[RunStep],
        failure: JudgeFailure,
    ) -> Result<Rest, EngineError> {
        let now = self.now();
        self.parts
            .store
            .transition_step(
                judge.id,
                StepStatus::Running,
                StepStatus::AwaitingApproval,
                now,
            )
            .await?;
        self.parts
            .store
            .answer_gate(
                judge.id,
                GateOutcome::Rejected,
                Some(failure.to_string()),
                now,
            )
            .await?;
        let reason = HumanReason::JudgeFailed(failure).to_string();
        self.park_selection(run, phase, attempt, slot, &reason)
            .await
    }

    // -- plan D25's reconcile and plan D36's cleanup -------------------------------------------

    /// After a step reached `done` and before the walk advances: the winner into the primary tree
    /// (plan D25, ANA-2 `:975-988`).
    ///
    /// `Ok(None)` — carry on. `Ok(Some(rest))` — the run is parked, with the reason on the item.
    ///
    /// **Every error parks, including `Git` and `Io`** (blueprint H-2). The step is already `done`
    /// and `fail_hard` moves `running -> failed` only — `done -> failed` is illegal
    /// (`crates/htui-core/src/model/run.rs`) — so an escaped `?` would leave the run `running`
    /// with every step `done`, which `cursor` reads as `Finished` and the next pass would
    /// `finish_run(Done)` over an unmerged primary. A run parked with the sentence is also the
    /// better answer for a transient `index.lock` that outlived its three retries: nothing is
    /// lost, and the note tells the human what the tree is in.
    ///
    /// The trees are read back from rows rather than carried from stage 2 (plan D16): a step whose
    /// gate a human answered hours later is reconciled by a different call than the one that
    /// prepared it.
    ///
    /// **A parked run has no resume verb this milestone** (blueprint F-G, R-7): `AnswerGate` needs
    /// a step at `awaiting_approval` and every step here is `done`. Milestone 6's `Unblock`-shaped
    /// verb is where a retry of `reconcile` belongs.
    ///
    /// `siblings` are the other candidates of a fan-out winner's slot, handed to the isolator so a
    /// `shared_serialized` checkout left at a sibling's commit may be moved to the winner's (plan
    /// D56); empty for a `fan_out = 1` step.
    async fn reconcile_done_step(
        &self,
        run: &Run,
        step: &RunStep,
        siblings: &[StepId],
    ) -> Result<Option<Rest>, EngineError> {
        let trees = self.parts.store.step_trees(step.id).await?;
        match self
            .parts
            .isolator
            .reconcile(step.id, &trees, siblings)
            .await
        {
            Ok(after) => {
                // ANA-2 `:987-988`: the winner's `after_hash` becomes the merge commit.
                self.parts.store.record_commits(step.id, &after).await?;
                Ok(None)
            }
            Err(err) => {
                let bases = trees
                    .iter()
                    .map(|tree| format!("{}@{}", tree.repo_id, tree.base_ref))
                    .collect::<Vec<_>>()
                    .join(", ");
                let reason = if bases.is_empty() {
                    err.to_string()
                } else {
                    format!("{err} (before_hash: {bases})")
                };
                Ok(Some(self.park_run(run, step, &reason).await?))
            }
        }
    }

    /// Parks a run whose step is already `done`: run, item, then the note.
    ///
    /// Blueprint H-10's order with the step left alone — `gate::apply`'s park moves the step
    /// first because it is the thing being waited on, and here there is nothing to move. The item
    /// goes through `in_progress` on its own because that is where a running run's item is.
    ///
    /// `run.failure` is **not** written: `finish_run` is the only writer of that column and the
    /// run is not finished. The reason is the `item_note`, which is where every other park's is
    /// (`gate.rs`'s `note_step`, blueprint H-9).
    async fn park_run(&self, run: &Run, step: &RunStep, reason: &str) -> Result<Rest, EngineError> {
        let now = self.now();
        self.parts
            .store
            .transition_run(run.id, RunStatus::Running, RunStatus::AwaitingApproval, now)
            .await?;
        if let Some(item) = run.item_id {
            self.parts
                .store
                .transition(item, Status::InProgress, Status::AwaitingApproval)
                .await?;
            self.parts
                .store
                .add_note(NewNote {
                    id: NoteId::new(),
                    item_id: item,
                    body: format!(
                        "reconcile refused for step {} at position {}: {reason}; \
                         the step stays `done` and the run is parked",
                        step.id, step.position
                    ),
                    created_by: self.parts.user,
                    box_id: Some(self.parts.box_id),
                    via_step_id: Some(step.id),
                    created_at: now,
                })
                .await?;
        }
        Ok(Rest {
            run: RunStatus::AwaitingApproval,
            position: Some(step.position),
            failure: None,
        })
    }

    /// ANA-2 §4.6's cleanup, run-terminal and **never** at step end (invariant 6, `:994-1000`).
    ///
    /// Every step of the run, not just the last: a `shared_serialized` guard is released by
    /// `run_step_tree` row, and a step that never reached `capture` — a `fail_hard` path skips
    /// stage 5 entirely — is holding one nobody else will give back (T5, blueprint A-1, H-15).
    ///
    /// Cleanup failures are `warn`ed and never raised (plan D36): the run is already terminal when
    /// this runs, and a failed `rm` must not un-terminate it. No sweep retries a terminal run's
    /// cleanup, because the sweep adopts only `running` runs: milestone 6 owns that retry (R-25).
    ///
    /// `pub` because milestone 6's Runs tab calls it directly for a run the worker did not end.
    ///
    /// # Errors
    /// The store's own refusals only. The isolator's are logged.
    pub async fn cleanup_run(&self, run: RunId) -> Result<(), EngineError> {
        let steps = self.parts.store.run_steps(run).await?;
        let mut trees = Vec::new();
        for step in &steps {
            trees.extend(self.parts.store.step_trees(step.id).await?);
        }
        if let Err(err) = self.parts.isolator.cleanup(run, &trees).await {
            tracing::warn!(
                %run,
                %err,
                "run-terminal cleanup failed; no sweep retries a terminal run's cleanup (milestone 6)"
            );
        }
        Ok(())
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
        let failure = RunFailure::MissingInput(kind);
        if !self.fail_hard(run, step, &failure.to_string()).await? {
            // Another process moved the step out of `running`; the next pass re-derives from rows
            // rather than reporting a rest this call did not write (plan D16, D17).
            return Ok(None);
        }
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

        let role = TemplateRole::of_name(&template.name);
        let (verify_failure, previous_diff) =
            if matches!(role, TemplateRole::Phase) && step.attempt > 1 {
                self.forwarded(run, step, &mut notes).await?
            } else {
                (None, None)
            };

        let spec = PromptSpec {
            role,
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
            excerpts: no_excerpts(caps),
            command_queue: phase.command_queue != htui_core::model::CommandQueue::Off,
            // Plan D67: what the previous attempt's winner left — its failed verify and its diff —
            // forwarded to a phase step's next attempt; `None` on attempt 1 and for a judge.
            verify_failure,
            previous_diff,
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

    /// Plan D67 (M3 D32's forward): the loop's two sections for a phase step at `attempt > 1`,
    /// read off the previous attempt's winner (`status::winner_at`, plan D66) at the same position.
    ///
    /// `verify_failure` is `Some` only when that winner's `verify_outcome` is `fail`, built from its
    /// last `command_run` row; an exit code recorded nowhere cannot be rendered, so it becomes a
    /// trim note instead. `previous_diff` is `Isolator::diff` over the winner's tree and commit rows;
    /// an isolator error degrades the prompt with a note rather than failing the step, because the
    /// diff is advisory (D55).
    async fn forwarded(
        &self,
        run: &Run,
        step: &RunStep,
        notes: &mut Vec<String>,
    ) -> Result<(Option<VerifyFailure>, Option<DiffBlock>), EngineError> {
        let steps = self.parts.store.run_steps(run.id).await?;
        let Some(previous) = winner_at(&steps, step.position, step.attempt - 1) else {
            notes.push(format!(
                "no attempt {} at position {} to forward from (plan D67)",
                step.attempt - 1,
                step.position
            ));
            return Ok((None, None));
        };

        let verify_failure = if previous.verify_outcome == Some(VerifyOutcome::Fail) {
            let row = self.parts.store.command_runs(previous.id).await?.pop();
            match previous
                .verify_exit_code
                .or_else(|| row.as_ref().and_then(|row| row.exit_code))
            {
                Some(exit_code) => Some(VerifyFailure {
                    exit_code,
                    output: row.and_then(|row| row.output).unwrap_or_default(),
                }),
                None => {
                    notes.push(format!(
                        "verify_failure unavailable: step {} failed its verify_command and \
                         recorded no exit code",
                        previous.id
                    ));
                    None
                }
            }
        } else {
            None
        };

        let trees = self.parts.store.step_trees(previous.id).await?;
        let commits = self.parts.store.step_commits(previous.id).await?;
        let previous_diff = match self.parts.isolator.diff(&trees, &commits).await {
            Ok(diff) => diff,
            Err(err) => {
                notes.push(format!("previous_diff unavailable: {err}"));
                None
            }
        };
        Ok((verify_failure, previous_diff))
    }

    /// **Stage 4 — session.** One driver, one session, one turn, one recorder.
    ///
    /// A closed stream is a [`htui_agent::error::DriverError`] and not a `done`
    /// (`crates/htui-agent/src/record.rs:1689-1691`), which is what makes settle's first rule
    /// reachable at all.
    ///
    /// `cwd` is stage 2's, handed down the call frame rather than asked for again: `prepare` is
    /// not idempotent by contract (`crate::isolate::Isolator::prepare` promises nothing of the
    /// kind) and milestone 3's `gix` layer will do real work on every call, so a second one per
    /// step is a second set of trees and a second `before_hash`. Plan D16 forbids state held
    /// *across* calls; a parameter inside one is not that.
    ///
    /// [`open_recorder`](Self::open_recorder), one [`drive_once`](Self::drive_once) and the
    /// recorder's `finish`: the judge (plan D52) is the same three with a second `drive_once` in
    /// between, under the one recorder.
    async fn session(
        &self,
        run: &Run,
        step: &RunStep,
        phase: &SnapshotPhase,
        prompt: &AssembledPrompt,
        cwd: std::path::PathBuf,
        extra_dirs: Vec<std::path::PathBuf>,
    ) -> Result<(SessionResult, Option<htui_agent::record::CapBreach>), EngineError> {
        let mut recorder = self.open_recorder(run, step, prompt).await?;
        // A spawn failure is folded in rather than propagated straight out of the `?`: the
        // recorder has already written the prompt row, so it is closed out on this path exactly as
        // it is on a `pump` error. What it is *not* is a settle outcome — ANA-2 `:639` gives
        // "spawn failure" an unconditioned `running -> failed` and §4.2's gate table has no cell
        // for it — so it is re-raised instead of handed to `gate::apply`, which under `always`
        // would park a human at a gate on a session that never opened. `walk_step`'s hard failure
        // is what lands it in `failed`, under every gate.
        let result = match self
            .drive_once(
                run,
                step,
                phase,
                &SessionKey::of(step),
                &prompt.text,
                cwd,
                extra_dirs,
                &mut recorder,
            )
            .await
        {
            Ok(result) => result,
            Err(refused) => {
                recorder.finish().await?;
                return Err(refused);
            }
        };
        let summary = recorder.finish().await?;
        Ok((result, summary.cap_breach))
    }

    /// A step's [`Recorder`], with the run cap applied and `prompt` recorded as its seq-0 row.
    ///
    /// `record_prompt` also writes `run_step.prompt_digest`, so every session a step opens — a
    /// candidate's one, the judge's two (plan D52) — is recorded against the digest of the prompt
    /// it was opened with.
    async fn open_recorder(
        &self,
        run: &Run,
        step: &RunStep,
        prompt: &AssembledPrompt,
    ) -> Result<Recorder<'a, S>, EngineError> {
        let project = self.project(run.project_id).await?;
        let settings = Self::project_settings(&project);
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
        Ok(recorder)
    }

    /// One driver session under `recorder`: the driver for `key`, `start` with `text`, and the
    /// pump to its `done`.
    ///
    /// The outer `Err` is a driver that refused to **start** ([`EngineError::Driver`]) or a
    /// read that failed before it; the inner `Result` is what the pump answered, which is a settle
    /// input and not an engine fault. The recorder is the caller's to finish on either path.
    #[allow(
        clippy::too_many_arguments,
        reason = "the session's four coordinates and its three per-call inputs; a struct would be \
                  built at exactly two call sites and read here only"
    )]
    async fn drive_once(
        &self,
        run: &Run,
        step: &RunStep,
        phase: &SnapshotPhase,
        key: &SessionKey<'_>,
        text: &str,
        cwd: std::path::PathBuf,
        extra_dirs: Vec<std::path::PathBuf>,
        recorder: &mut Recorder<'a, S>,
    ) -> Result<SessionResult, EngineError> {
        let project = self.project(run.project_id).await?;
        let settings = Self::project_settings(&project);
        let candidate = Self::candidate_of(step, phase)?;
        let driver = (self.parts.driver)(&candidate, key);

        let spec = SessionSpec {
            agent_id: candidate.agent_id,
            step_id: step.id,
            cwd,
            // Plan D28: every tree that is not under `cwd` — the second repo of a
            // `shared_serialized` or `local` step, which lives in its own checkout somewhere else
            // on the box. An agent that cannot read it cannot work in it.
            extra_dirs,
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
        let mut session = driver.start(spec, text.to_owned()).await?;
        Ok(pump(&mut *session, recorder).await)
    }

    // -- helpers -------------------------------------------------------------------------------

    /// Plan D8: every instant the walk hands a writer comes from here.
    fn now(&self) -> DateTime<Utc> {
        self.parts.clock.now()
    }

    /// Plan D85: the lease's TTL and heartbeat interval, from `app_setting` with §10's defaults.
    fn lease_times(&self) -> LeaseTimes {
        LeaseTimes::from_app(&self.parts.app)
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
    ///
    /// **`failure` is always `None` here, deliberately.** [`Rest::failure`] is the typed
    /// [`RunFailure`] of the transition that *caused* the stop, and a rest is by definition not
    /// that transition: this is the arm a walk takes when it finds a run already terminal or
    /// already parked, possibly from an earlier call or another process. `run.failure` is a
    /// `String` and [`RunFailure`]'s `Display` is one-way by plan D12 — one vocabulary, one
    /// renderer — so parsing it back would mean a second copy of that grammar in a second module,
    /// which is the drift D12 exists to prevent. A caller that wants the reason of a stop it did
    /// not witness reads `run.failure`, which is the durable record and the row a human reads.
    async fn resting(&self, run: &Run) -> Result<Rest, EngineError> {
        let snapshot = Self::snapshot_of(run)?;
        let steps = self.parts.store.run_steps(run.id).await?;
        let position = match cursor(&snapshot, &steps) {
            Cursor::Finished => None,
            Cursor::Create { position, .. }
            | Cursor::Fan { position, .. }
            | Cursor::Select { position, .. } => Some(position),
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
    ///
    /// Only a genuine version mismatch is [`EngineError::SnapshotVersion`]. A row with no snapshot
    /// at all, or one whose blob does not decode, used to be reported as `v: 0` — a version the
    /// snapshot never had, with the serde error thrown away, so the operator was told the engine
    /// was too old to read a run it could in fact read nothing of.
    fn snapshot_of(run: &Run) -> Result<GraphSnapshot, EngineError> {
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
    ///
    /// A position the snapshot does not name is an engine invariant and not a store refusal:
    /// invariant 2 makes the snapshot the only thing a live run reads, so there is no second
    /// source to try and nothing the store did wrong (see [`EngineError::Snapshot`]).
    fn phase_at(
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

    /// The candidate a step was created for, denormalised back out of the row and the phase.
    ///
    /// The row is authority for `agent_id` and `model` — it is what a resume reads — and the phase
    /// supplies `agent_name`, which `run_step` does not carry.
    fn candidate_of(
        step: &RunStep,
        phase: &SnapshotPhase,
    ) -> Result<SnapshotCandidate, EngineError> {
        let agent_id = step.agent_id.ok_or_else(|| EngineError::Snapshot {
            run: step.run_id,
            reason: format!(
                "step {} names no agent; stage 1 creates none without one",
                step.id
            ),
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

    /// The document of `phase.output_kind` produced by **this step**, at its highest version.
    ///
    /// Read over **every** version's head (`ReadStore::documents`) and then by id, not through
    /// `documents_of_kinds`, which answers only the item's latest version of each kind
    /// (blueprint F-A): three fan-out candidates each write a version of one kind, and a
    /// latest-only read would find the output of one of them and settle the other two
    /// `missing_output`. The judge's two calls are the same shape — each call's verdict is the
    /// newest `judge` document *this* step wrote (plan D52).
    async fn output_of(
        &self,
        item: ItemId,
        phase: &SnapshotPhase,
        step: StepId,
    ) -> Result<Option<Document>, EngineError> {
        let newest = self
            .parts
            .store
            .documents(item)
            .await?
            .into_iter()
            .filter(|head| head.kind == phase.output_kind && head.produced_by_step_id == Some(step))
            .max_by_key(|head| head.version);
        let Some(head) = newest else {
            return Ok(None);
        };
        Ok(self.parts.store.document(head.id).await?)
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

/// One fan-out candidate's inputs to [`Engine::run_candidate`]: everything its stages 2 to 5 read
/// that the group shares (plan D58).
#[derive(Debug, Clone, Copy)]
struct CandidateStage<'s> {
    /// The run, re-read by the group's pass.
    run: &'s Run,
    /// The fanned-out phase.
    phase: &'s SnapshotPhase,
    /// The candidate, still `pending`.
    step: &'s RunStep,
    /// The group's one assembled prompt.
    prompt: &'s AssembledPrompt,
    /// The group's base per repo (plan D54(a)).
    base: &'s BTreeMap<RepoId, String>,
}

/// The judge's two orderings and the template they were assembled from (plan D52, D53).
#[derive(Debug)]
struct JudgePrompts {
    /// Call 0: the survivors in `fanout_index` order.
    forward: AssembledPrompt,
    /// Call 1: the same survivors reversed.
    reversed: AssembledPrompt,
    /// The `judge` row the prompts rendered, as the judge phase's template.
    template: SnapshotTemplate,
}

/// The `fanout_index` of a candidate the judge prompt's trimmer dropped outright (plan D53).
fn dropped_candidate(forward: &AssembledPrompt) -> Option<i32> {
    forward
        .trim
        .sections
        .iter()
        .find_map(|section| match section.name {
            SectionName::JudgeCandidate(index) if section.strategy == TrimStrategy::Dropped => {
                Some(index)
            }
            _ => None,
        })
}

/// Plan D52's verdict over the two calls' documents: each must parse, name one of `survivors`,
/// and agree with the other. The reason is call 0's for the winner, else `judge: <winner>`.
fn decide(documents: &[Document; 2], survivors: &[i32]) -> Result<(i32, String), JudgeFailure> {
    let [forward, reversed] = documents;
    let forward = parse_judge_verdict(&forward.body)?;
    let reversed = parse_judge_verdict(&reversed.body)?;
    for verdict in [&forward, &reversed] {
        if !survivors.contains(&verdict.winner) {
            return Err(JudgeFailure::OutOfRange {
                winner: verdict.winner,
                survivors: survivors.to_vec(),
            });
        }
    }
    if forward.winner != reversed.winner {
        return Err(JudgeFailure::Disagreement {
            forward: forward.winner,
            reversed: reversed.winner,
        });
    }
    let reason = forward
        .reasons
        .get(&forward.winner.to_string())
        .cloned()
        .unwrap_or_else(|| format!("judge: {}", forward.winner));
    Ok((forward.winner, reason))
}

/// Everything [`Engine::verify`] reads, as a struct rather than seven arguments.
///
/// [`crate::gate::SettleInput`]'s own reason, one stage earlier: the order the checks happen in is
/// stated once, in the function, and the call site names what it is handing over.
#[derive(Debug)]
struct VerifyStage<'a> {
    /// The run, for `project_id` — `repos` is what says which tree is the primary's.
    run: &'a Run,
    /// The step the report belongs to (`command_run.run_step_id`).
    step: &'a RunStep,
    /// The phase, for `verify_command` and `deadline_seconds`.
    phase: &'a SnapshotPhase,
    /// Stage 2's rows, in scope order.
    trees: &'a [htui_core::model::RunStepTree],
    /// Where the deadline's remainder is measured from: `run_step.started_at` for a `fan_out = 1`
    /// step, the moment `prepare` answered for a fan-out candidate (plan D48), so a
    /// `shared_serialized` sibling is not charged for the per-repo lock wait.
    started_at: DateTime<Utc>,
    /// Stage 2's `cwd`, the `command_run.cwd` fallback when there is no primary tree (H-23).
    session_cwd: &'a std::path::Path,
    /// Stage 4's answer: a verify runs only on `Ok` (blueprint A-3).
    result: &'a Result<DoneEvent, htui_agent::error::DriverError>,
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

/// `app_setting.min_budget_for_new_attempt` in USD micros (OQ-6), else `0`.
///
/// The key is unseeded, so a stray `0`, a negative, a string or a float all read as `0` —
/// `graph.rs`'s "positive or the rung is silent" rule for the same table (blueprint H-19).
fn min_budget(app: &BTreeMap<String, Value>) -> i64 {
    app.get(MIN_BUDGET_KEY)
        .and_then(Value::as_i64)
        .filter(|micros| *micros > 0)
        .unwrap_or(0)
}

/// An excerpt set with no files whose audit records the caps a pass *would* have run under.
///
/// §4.5's ranker needs a resolved repo root and `repo_box_path` has no writer the walk can reach,
/// so this is the shape `crates/htui/src/preview.rs:274-295` already ships — for a phase step and,
/// with nothing to rank at all, for the judge (plan D53).
fn no_excerpts(caps: htui_core::prompt::excerpt::ExcerptCaps) -> ExcerptSet {
    ExcerptSet {
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
    }
}

/// The rows `walk` skipped that rank above `chosen` in `phase.candidates` (plan D60).
///
/// The walk partitions the candidates in order, so pairing each candidate with the next eligible
/// row or else the next skipped one recovers every skipped row's rank exactly — even when one
/// agent is a candidate twice under two models.
fn skipped_above<'w>(
    phase: &SnapshotPhase,
    walk: &'w Walk,
    chosen: &SnapshotCandidate,
) -> Vec<&'w Skipped> {
    let mut eligible = walk.eligible.iter().peekable();
    let mut skipped = walk.skipped.iter();
    let mut above = Vec::new();
    for candidate in &phase.candidates {
        if eligible.next_if(|row| *row == candidate).is_some() {
            if candidate == chosen {
                break;
            }
        } else if let Some(row) = skipped.next() {
            above.push(row);
        }
    }
    above
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

/// Builds an [`Engine`] over a `FakeOrchestrator`'s parts and dispatches one command.
///
/// **This is the wiring T3's `FakeOrchestrator::dispatch` left as a `todo!()`**, and it lives here
/// rather than in `fake.rs` for the reason the file itself gives: every part is already public on
/// the orchestrator, and what was missing was `engine.rs`. Putting it beside the `Engine` keeps
/// `fake.rs` exactly as T3 shipped it, and lets `conformance.rs`'s `impl Orchestrate for
/// FakeOrchestrator` call one function instead of re-deciding fourteen fields per case.
///
/// A fresh `Engine` per command is not a concession to the test: the engine holds nothing across a
/// call (plan D16), so this is the shape milestone 6's Runs tab has too.
///
/// # Errors
/// Every [`EngineError`] the walk raises.
#[cfg(feature = "test-support")]
pub async fn dispatch_fake(
    orch: &crate::fake::FakeOrchestrator,
    command: Command,
) -> Result<CommandOutcome, EngineError> {
    let graphs = orch.graphs();
    let driver = |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key);
    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    let engine = Engine::new(fake_parts(orch, &graphs, &driver, &scrubber).await?);
    engine.dispatch(command).await
}

/// [`Engine::claim`] over the same parts (plan D84).
///
/// # Errors
/// Every [`EngineError`] the claim and the walk raise.
#[cfg(feature = "test-support")]
pub async fn claim_fake(
    orch: &crate::fake::FakeOrchestrator,
    run: RunId,
) -> Result<CommandOutcome, EngineError> {
    let graphs = orch.graphs();
    let driver = |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key);
    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    let engine = Engine::new(fake_parts(orch, &graphs, &driver, &scrubber).await?);
    engine.claim(run).await
}

/// [`Engine::resume`] over the same parts (ANA-2 §12 criterion 3).
///
/// # Errors
/// Every [`EngineError`] the walk raises.
#[cfg(feature = "test-support")]
pub async fn resume_fake(
    orch: &crate::fake::FakeOrchestrator,
    run: RunId,
) -> Result<Resume, EngineError> {
    let graphs = orch.graphs();
    let driver = |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key);
    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    let engine = Engine::new(fake_parts(orch, &graphs, &driver, &scrubber).await?);
    engine.resume(run).await
}

/// [`Engine::sweep`] over the same parts (plan D98): the second process's sweep in every
/// recovery case, over a `FakeOrchestrator::restarted` harness.
///
/// # Errors
/// As [`Engine::sweep`].
#[cfg(feature = "test-support")]
pub async fn sweep_fake(orch: &crate::fake::FakeOrchestrator) -> Result<Vec<Adopted>, EngineError> {
    let graphs = orch.graphs();
    let driver = |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key);
    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    let engine = Engine::new(fake_parts(orch, &graphs, &driver, &scrubber).await?);
    engine.sweep().await
}

/// The fourteen fields, filled from the harness.
///
/// `app` is read here rather than cached because `MemStore::set_app_setting` (`mem.rs:430`) is the
/// only writer a case can reach for `step_deadline_seconds` — `SettingKey` is a closed enum of ten
/// that does not carry it, and `ProjectPatch` has no `settings` field — so a map captured at
/// construction would silently ignore the one knob the deadline case has.
#[cfg(feature = "test-support")]
pub(crate) async fn fake_parts<'a>(
    orch: &'a crate::fake::FakeOrchestrator,
    graphs: &'a crate::fake::FakeGraphSource<'a>,
    driver: DriverFor<'a>,
    scrubber: &'a dyn Scrubber,
) -> Result<
    EngineParts<
        'a,
        htui_core::store::MemStore,
        crate::fake::FakeGraphSource<'a>,
        crate::fake::FakeIsolator,
        crate::fake::FakeVerifier,
        crate::fake::TestClock,
        FirstCandidate,
        crate::fake::FakeOrchestrator,
    >,
    EngineError,
> {
    let app = orch.store.app_settings().await?;
    let box_profile = orch
        .store
        .box_profile(orch.box_id())
        .await?
        .ok_or(EngineError::Store(StoreError::NotFound {
            entity: "box",
            id: orch.box_id().to_string(),
        }))?;
    Ok(EngineParts {
        store: &orch.store,
        graphs,
        isolator: &orch.isolator,
        verifier: &orch.verifier,
        clock: &orch.clock,
        selector: &FirstCandidate,
        sink: orch,
        driver,
        scrubber,
        app,
        box_profile,
        box_id: orch.box_id(),
        owner: orch.owner(),
        user: orch.user(),
    })
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
        key: &SessionKey<'_>,
        _done: &DoneEvent,
    ) -> Result<(), StoreError> {
        Self::after_done(self, item, step, phase, key).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeDelta;
    use htui_agent::driver::DriverCaps;
    use htui_agent::error::DriverError;
    use htui_agent::event::StopReason;
    use htui_core::fixtures::ids;
    use htui_core::model::{
        Gate, GateOutcome, GraphSnapshot, ItemPatch, NewRepo, NewRunStep, NewStepGraph, PhaseId,
        PhasePatch, RepoId, RunMode, RunStatus, SnapshotCandidate, SnapshotPhase, Status,
        StepGraphId, StepGraphPhase, StepId, StepStatus,
    };
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

    use super::{AgentSelector, FirstCandidate, NoSink, Resume, SessionKey, required_inputs};
    use crate::command::{Command, CommandOutcome, EngineError, GateAnswer};
    use crate::fake::{FakeOrchestrator, ScriptedStep};
    use crate::isolate::{Clock as _, IsolateError};
    use crate::status::RunFailure;

    /// Everything an `Engine` borrows from a `FakeOrchestrator`, held alive for one test.
    ///
    /// The driver factory is a closure over the orchestrator, which is what `FakeDriver`'s
    /// "one script, once" rule needs (blueprint H-12): a fresh driver per `(phase, attempt)`.
    /// The `FakeOrchestrator` plus the two edits a case has to make through real writers.
    ///
    /// `dispatch` and `resume` forward to [`super::dispatch_fake`] and [`super::resume_fake`],
    /// which is exactly what T5's conformance binding calls — so these tests exercise the wiring
    /// and not a second copy of it.
    struct Harness {
        orch: FakeOrchestrator,
    }

    impl Harness {
        async fn new() -> Self {
            Self {
                orch: FakeOrchestrator::demo(),
            }
        }

        async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError> {
            super::dispatch_fake(&self.orch, command).await
        }

        async fn resume(&self, run: htui_core::model::RunId) -> Result<Resume, EngineError> {
            super::resume_fake(&self.orch, run).await
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

        /// The HTUI project's primary repo, so an undeclared item's scope is that whole repo
        /// (ANA-2 §4.7) rather than nothing.
        async fn add_primary_repo(&self) -> RepoId {
            let repo = RepoId::new();
            self.orch
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
                .expect("the demo project has no repo yet");
            repo
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
            FirstCandidate.select(&phase, &candidates, 0),
            Some(&candidates[0])
        );
        assert_eq!(
            FirstCandidate.select(&phase, &candidates, 2),
            Some(&candidates[0]),
            "`FirstCandidate` ignores the index: every candidate of a group runs on the first \
             eligible agent (plan D71, OQ-7)"
        );
        assert_eq!(FirstCandidate.select(&phase, &[], 0), None);
    }

    /// A selector that records every `fanout_index` it is asked about and answers the candidate
    /// one past it, so a walk that ignored its answer — or asked with the wrong index — is visible.
    #[derive(Debug, Default)]
    struct IndexSelector {
        asked: std::sync::Mutex<Vec<i32>>,
    }

    impl AgentSelector for IndexSelector {
        fn select<'c>(
            &self,
            _phase: &SnapshotPhase,
            eligible: &'c [SnapshotCandidate],
            fanout_index: i32,
        ) -> Option<&'c SnapshotCandidate> {
            self.asked
                .lock()
                .expect("no panic holds the selector's lock")
                .push(fanout_index);
            let next = usize::try_from(fanout_index).ok()?.checked_add(1)?;
            eligible.get(next.checked_rem(eligible.len())?)
        }
    }

    /// Plan D71: stage 1 hands the selector the candidate's `fanout_index`, and the step row carries
    /// the selector's own answer rather than the first eligible candidate. A `fan_out = 1` phase
    /// asks once, with `0`.
    #[tokio::test]
    async fn a_selector_is_asked_with_the_fanout_index() {
        let orch = FakeOrchestrator::demo();
        orch.store
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, orch.clock.now())
            .await
            .expect("the seeded run is queued and cancellable");
        orch.with_candidates(
            "prd",
            vec![
                (ids::AGENT_CLAUDE, "sonnet"),
                (ids::AGENT_AGY, "gemini-3.7-flash-high"),
            ],
        );

        let graphs = orch.graphs();
        let driver =
            |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key);
        let scrubber = htui_core::scrub::MinimalScrubber::new([]);
        let selector = IndexSelector::default();
        let engine = super::Engine::new(super::EngineParts {
            store: &orch.store,
            graphs: &graphs,
            isolator: &orch.isolator,
            verifier: &orch.verifier,
            clock: &orch.clock,
            selector: &selector,
            sink: &orch,
            driver: &driver,
            scrubber: &scrubber,
            app: orch
                .store
                .app_settings()
                .await
                .expect("MemStore never fails a read"),
            box_profile: orch
                .store
                .box_profile(orch.box_id())
                .await
                .expect("MemStore never fails a read")
                .expect("the demo fixture seeds this box"),
            box_id: orch.box_id(),
            owner: orch.owner(),
            user: orch.user(),
        });

        let CommandOutcome::Started { run, .. } = engine
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
        let steps = orch.steps(run).await;
        assert_eq!(steps.len(), 1, "`prd` parks at its `always` gate");
        assert_eq!(
            (
                steps[0].fanout_index,
                steps[0].agent_id,
                steps[0].model.as_deref()
            ),
            (0, Some(ids::AGENT_AGY), Some("gemini-3.7-flash-high")),
            "the row is the selector's answer for index 0, not the first eligible candidate"
        );
        assert_eq!(
            *selector
                .asked
                .lock()
                .expect("no panic holds the selector's lock"),
            [0],
            "a `fan_out = 1` phase asks once, for index 0"
        );
    }

    /// Plan D71 across a group: the selector is asked once per `fanout_index`, every index before
    /// any row is written (blueprint F-K), and each candidate row carries **its own** answer — so a
    /// selector keyed on the index spreads one group over two agents with no other change.
    #[tokio::test]
    async fn a_fanout_keyed_selector_may_choose_differently_per_index() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" {
                    phase.fan_out = 2;
                }
            })
            .await;
        let orch = &harness.orch;
        orch.with_candidates(
            "prd",
            vec![
                (ids::AGENT_CLAUDE, "sonnet"),
                (ids::AGENT_AGY, "gemini-3.7-flash-high"),
            ],
        );

        let graphs = orch.graphs();
        let driver =
            |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key);
        let scrubber = htui_core::scrub::MinimalScrubber::new([]);
        let selector = IndexSelector::default();
        let engine = super::Engine::new(super::EngineParts {
            store: &orch.store,
            graphs: &graphs,
            isolator: &orch.isolator,
            verifier: &orch.verifier,
            clock: &orch.clock,
            selector: &selector,
            sink: orch,
            driver: &driver,
            scrubber: &scrubber,
            app: orch
                .store
                .app_settings()
                .await
                .expect("MemStore never fails a read"),
            box_profile: orch
                .store
                .box_profile(orch.box_id())
                .await
                .expect("MemStore never fails a read")
                .expect("the demo fixture seeds this box"),
            box_id: orch.box_id(),
            owner: orch.owner(),
            user: orch.user(),
        });

        let CommandOutcome::Started { run, rest } = engine
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
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(0)),
            "an `always` group parks for a human selection (plan D49(2))"
        );
        let steps = orch.steps(run).await;
        assert_eq!(
            steps
                .iter()
                .map(|step| (step.fanout_index, step.agent_id, step.status))
                .collect::<Vec<_>>(),
            [
                (0, Some(ids::AGENT_AGY), StepStatus::Done),
                (1, Some(ids::AGENT_CLAUDE), StepStatus::Done),
            ],
            "each candidate is its own selection, and both ran"
        );
        assert_eq!(
            *selector
                .asked
                .lock()
                .expect("no panic holds the selector's lock"),
            [0, 1],
            "asked once per index, in index order"
        );
    }

    /// Plan D65, blueprint F-D: `RetryStep` on any member of a parked group retires the whole slot
    /// and admits the whole group again at `attempt + 1`; out of budget, the guard refuses and
    /// nothing is written.
    #[tokio::test]
    async fn retry_on_a_parked_group_retries_the_whole_group() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" {
                    phase.fan_out = 2;
                    phase.retry_limit = 1;
                }
            })
            .await;
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
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(0)),
            "an `always` group parks for a human"
        );
        let member = harness
            .orch
            .steps(run)
            .await
            .into_iter()
            .find(|step| step.fanout_index == 1)
            .expect("the group has index 1")
            .id;

        let outcome = harness
            .dispatch(Command::RetryStep { run, step: member })
            .await
            .expect("a parked group retries as a whole");
        let CommandOutcome::Retried { step, rest } = outcome else {
            panic!("`RetryStep` answers `Retried`, not {outcome:?}");
        };
        assert_eq!(step, member);
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(0)),
            "attempt 2's group parks again"
        );
        let steps = harness.orch.steps(run).await;
        let slot = |attempt: i32| {
            steps
                .iter()
                .filter(|step| step.position == 0 && step.attempt == attempt)
                .map(|step| (step.fanout_index, step.status))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            slot(1),
            [(0, StepStatus::Superseded), (1, StepStatus::Superseded)],
            "the whole slot was retired, not the one member named"
        );
        assert_eq!(
            slot(2),
            [(0, StepStatus::Done), (1, StepStatus::Done)],
            "the whole group was admitted again"
        );

        let before = harness.orch.steps(run).await;
        let stale = harness
            .dispatch(Command::RetryStep { run, step: member })
            .await
            .expect_err("a member of the retired slot names nothing to retry");
        assert!(
            matches!(
                stale,
                EngineError::StaleSlot { step, attempt: 1, latest: 2 } if step == member
            ),
            "{stale}"
        );
        let current = before
            .iter()
            .find(|step| step.attempt == 2 && step.fanout_index == 0)
            .expect("attempt 2 has index 0")
            .id;
        let refused = harness
            .dispatch(Command::RetryStep { run, step: current })
            .await
            .expect_err("attempt 3 is out of budget");
        assert!(
            matches!(refused, EngineError::RetryExhausted { attempt: 2, .. }),
            "{refused}"
        );
        assert_eq!(
            harness.orch.steps(run).await,
            before,
            "and nothing was written"
        );
    }

    /// ANA-2 `:754-758`: a retried group starts from the base its retired slot started from, read
    /// off that slot's `before_hash` rows. Rows that disagree name no one base, so the group is
    /// refused rather than started from the checkout's `HEAD`, and no candidate is prepared.
    #[tokio::test]
    async fn a_retried_group_whose_retired_bases_disagree_is_refused() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        // A base is a row per repo in scope, and the demo fixture holds no repo.
        harness
            .orch
            .store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the project has no repo yet");
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" {
                    phase.fan_out = 2;
                    phase.retry_limit = 1;
                }
            })
            .await;
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
        let slot: Vec<_> = harness
            .orch
            .steps(run)
            .await
            .into_iter()
            .filter(|step| step.position == 0 && step.fanout_index >= 0)
            .collect();
        let mut moved = harness
            .orch
            .store
            .step_commits(slot[1].id)
            .await
            .expect("MemStore never fails a read");
        moved[0].before_hash = "fake:elsewhere".to_owned();
        harness
            .orch
            .store
            .record_commits(slot[1].id, &moved[..1])
            .await
            .expect("the row is the step's own");
        let prepares = harness.orch.isolator.prepares();

        let refused = harness
            .dispatch(Command::RetryStep {
                run,
                step: slot[0].id,
            })
            .await
            .expect_err("two bases for one repo name no base to retry from");
        assert!(
            matches!(
                &refused,
                EngineError::GroupBase { position: 0, attempt: 2, reason, .. }
                    if reason.contains("fake:elsewhere")
            ),
            "{refused}"
        );
        assert_eq!(
            harness.orch.isolator.prepares(),
            prepares,
            "no candidate of attempt 2 was prepared"
        );
    }

    /// Blueprint H-19: `min_budget_for_new_attempt` is unseeded, so only a positive integer is a
    /// minimum; `0`, a negative, a string and a float are all silence.
    /// A `judge` document whose body is `body`, for [`super::decide`].
    fn verdict(body: &str) -> htui_core::model::Document {
        htui_core::model::Document {
            id: htui_core::model::DocumentId::new(),
            item_id: ids::HTUI_ANA_2,
            kind: "judge".to_owned(),
            version: 1,
            title: "judge".to_owned(),
            body: body.to_owned(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    /// A fenced verdict naming `winner`, with `reasons` as `(index, reason)`.
    fn block(winner: i32, reasons: &[(i32, &str)]) -> htui_core::model::Document {
        let reasons: serde_json::Map<String, serde_json::Value> = reasons
            .iter()
            .map(|(index, reason)| (index.to_string(), serde_json::Value::from(*reason)))
            .collect();
        let json = serde_json::json!({ "winner": winner, "reasons": reasons });
        verdict(&format!("Compared.\n\n```json\n{json}\n```"))
    }

    /// Plan D52's verdict, every arm: agreement with call 0's reason, the `judge: <i>` fallback,
    /// a winner out of range on **either** call, a disagreement, and an unparseable call.
    #[test]
    fn decide_reads_both_calls_and_range_checks_each() {
        use crate::fanout::JudgeFailure;

        let survivors = [0, 2];
        assert_eq!(
            super::decide(
                &[block(2, &[(2, "shorter")]), block(2, &[(2, "later")])],
                &survivors
            ),
            Ok((2, "shorter".to_owned())),
            "the reason is call 0's"
        );
        assert_eq!(
            super::decide(
                &[block(0, &[(2, "not the winner")]), block(0, &[])],
                &survivors
            ),
            Ok((0, "judge: 0".to_owned())),
            "no reason for the winner"
        );
        for documents in [
            [block(1, &[]), block(0, &[])],
            [block(0, &[]), block(1, &[])],
        ] {
            assert_eq!(
                super::decide(&documents, &survivors),
                Err(JudgeFailure::OutOfRange {
                    winner: 1,
                    survivors: vec![0, 2],
                }),
                "an eliminated candidate is out of range on either call"
            );
        }
        assert_eq!(
            super::decide(&[block(0, &[]), block(2, &[])], &survivors),
            Err(JudgeFailure::Disagreement {
                forward: 0,
                reversed: 2,
            })
        );
        assert!(matches!(
            super::decide(&[block(0, &[]), verdict("candidate 0, surely")], &survivors),
            Err(JudgeFailure::Unparseable(_))
        ));
    }

    #[test]
    fn min_budget_reads_only_a_positive_integer() {
        let app = |value: serde_json::Value| {
            BTreeMap::from([("min_budget_for_new_attempt".to_owned(), value)])
        };
        assert_eq!(super::min_budget(&BTreeMap::new()), 0);
        assert_eq!(super::min_budget(&app(serde_json::json!(250_000))), 250_000);
        for silent in [
            serde_json::json!(0),
            serde_json::json!(-5),
            serde_json::json!("250000"),
            serde_json::json!(2.5),
        ] {
            assert_eq!(super::min_budget(&app(silent.clone())), 0, "{silent}");
        }
    }

    /// Plants `project.settings.per_token_cap_run = cap_micros` on FEAT-3's project, keeping every
    /// other key of the blob: `set_project_settings` replaces the whole document.
    async fn cap_feat_3(harness: &Harness, cap_micros: i64) {
        let project = harness.orch.item(ids::HTUI_FEAT_3).await.project_id;
        let mut settings = harness
            .orch
            .store
            .project_settings(project)
            .await
            .expect("MemStore never fails a read")
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        settings["per_token_cap_run"] = serde_json::json!(cap_micros);
        harness.orch.store.set_project_settings(project, settings);
    }

    /// Starts FEAT-3 with `prd` and `plan` edited to `never`, so the walk admits position after
    /// position with no human in it and every stage 1 past the first sees a run that has spent.
    async fn start_ungated(harness: &Harness) -> (htui_core::model::RunId, crate::command::Rest) {
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" || phase.name == "plan" {
                    phase.gate = Gate::Never;
                }
            })
            .await;
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
        (run, rest)
    }

    /// Plan D60 rule 5 **through `stage_one`'s wiring**, not `select::walk` by itself: the spend
    /// is the run's own `Σ usage.cost_micros`, the cap the snapshot's `per_token_cap_run`, and the
    /// minimum `app_setting.min_budget_for_new_attempt`. `prd` spends 600 of a 1000 cap, so
    /// `plan`'s stage 1 sees 400 left against 500 required and refuses the run. Any of the three
    /// inputs wired as `0`/`None` lets `plan` run instead.
    #[tokio::test]
    async fn stage_one_refuses_a_run_whose_remaining_budget_is_below_the_minimum() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        cap_feat_3(&harness, 1_000).await;
        harness
            .orch
            .store
            .set_app_setting("min_budget_for_new_attempt", serde_json::json!(500));
        harness
            .orch
            .script("prd", 1, ScriptedStep::done_costing("the prd", 600));

        let (run, rest) = start_ungated(&harness).await;
        let expected = RunFailure::NoCandidateAgent {
            phase: "plan".to_owned(),
            detail: "claude (budget: 400 micros left, 500 required)".to_owned(),
        };
        assert_eq!(
            (rest.run, rest.position, rest.failure.as_ref()),
            (RunStatus::Failed, Some(1), Some(&expected))
        );
        let steps = harness.orch.steps(run).await;
        assert_eq!(
            steps
                .iter()
                .map(|step| (step.phase_name.as_str(), step.status))
                .collect::<Vec<_>>(),
            [("prd", StepStatus::Done)],
            "`prd`'s stage 1 had no spend to compare, and `plan`'s refused before a step row"
        );
        assert_eq!(
            steps[0]
                .usage
                .as_ref()
                .and_then(|usage| usage.get("cost_micros"))
                .and_then(serde_json::Value::as_i64),
            Some(600),
            "the spend stage 1 read is the recorder's sum"
        );
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::Blocked
        );
    }

    /// Plan D67's two degradations: a failed verify with no exit code recorded anywhere, and an
    /// isolator whose `diff` errors, each become a trim note on attempt 2's prompt — neither
    /// section is rendered, and neither fails the step (the diff is advisory, D55).
    #[tokio::test]
    async fn a_forward_with_nothing_to_render_degrades_to_trim_notes() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" {
                    phase.gate = Gate::Never;
                    phase.retry_limit = 1;
                    phase.verify_command = Some("cargo test".to_owned());
                    phase.template_name = "implement".to_owned();
                }
            })
            .await;
        let mut report = crate::fake::FakeVerifier::fail(1);
        report.exit_code = None;
        harness.orch.verifier.script_report(report);
        harness.orch.isolator.fail_diff("index.lock held");

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
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(1)),
            "attempt 2 ran despite both degradations, passed, and `plan` parked"
        );
        let steps = harness.orch.steps(run).await;
        let first = steps
            .iter()
            .find(|step| step.position == 0 && step.attempt == 1)
            .expect("attempt 1 ran");
        assert_eq!(
            first.verify_exit_code, None,
            "the premise: no code recorded"
        );
        let second = steps
            .iter()
            .find(|step| step.position == 0 && step.attempt == 2)
            .expect("attempt 2 ran");
        assert_eq!(second.status, StepStatus::Done);
        let notes: Vec<String> = second
            .trim_record
            .as_ref()
            .and_then(|record| record.get("notes"))
            .and_then(serde_json::Value::as_array)
            .expect("attempt 2's trim record carries its notes")
            .iter()
            .filter_map(|note| note.as_str().map(str::to_owned))
            .collect();
        assert!(
            notes.contains(&format!(
                "verify_failure unavailable: step {} failed its verify_command and recorded no \
                 exit code",
                first.id
            )),
            "{notes:?}"
        );
        assert!(
            notes.iter().any(|note| {
                note.starts_with("previous_diff unavailable: ") && note.contains("index.lock held")
            }),
            "{notes:?}"
        );
        let events = harness
            .orch
            .store
            .step_events(second.id)
            .await
            .expect("MemStore never fails a read")
            .expect("attempt 2 recorded its session");
        let sections = events
            .iter()
            .find(|event| event.seq == 0)
            .expect("seq 0 is the prompt")
            .payload["sections"]
            .to_string();
        assert!(
            !sections.contains("verify_failure") && !sections.contains("previous_diff"),
            "neither section is rendered: {sections}"
        );
    }

    /// The cap half of plan D60 rule 2 through the same wiring: `prd` and `plan` each spend 600 —
    /// under the 1000 cap per session, so the recorder's own breach (plan D70) never fires — and
    /// `implement`'s stage 1 sees the run at 1200 of 1000 and refuses it with no minimum set.
    #[tokio::test]
    async fn stage_one_refuses_a_run_whose_spend_reached_the_cap() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        cap_feat_3(&harness, 1_000).await;
        harness
            .orch
            .script("prd", 1, ScriptedStep::done_costing("the prd", 600));
        harness
            .orch
            .script("plan", 1, ScriptedStep::done_costing("the plan", 600));

        let (run, rest) = start_ungated(&harness).await;
        let expected = RunFailure::NoCandidateAgent {
            phase: "implement".to_owned(),
            detail: "claude (quota: cap reached (1200 of 1000 micros))".to_owned(),
        };
        assert_eq!(
            (rest.run, rest.position, rest.failure.as_ref()),
            (RunStatus::Failed, Some(2), Some(&expected))
        );
        assert_eq!(
            harness
                .orch
                .steps(run)
                .await
                .iter()
                .map(|step| (step.phase_name.as_str(), step.status))
                .collect::<Vec<_>>(),
            [("prd", StepStatus::Done), ("plan", StepStatus::Done)]
        );
    }

    /// Plan D60's note names only the skipped rows that **outrank** the choice, by position — so a
    /// skipped row below it is not a substitution, and one agent listed twice under two models is
    /// ranked by its row and not by its id.
    #[test]
    fn only_skipped_rows_above_the_choice_are_substitutions() {
        use crate::select::{SkipCause, Skipped, Walk};
        let candidate = |agent_id, agent_name: &str, model: &str| SnapshotCandidate {
            agent_id,
            agent_name: agent_name.to_owned(),
            model: model.to_owned(),
        };
        let skipped = |agent_id, agent_name: &str| Skipped {
            agent_id,
            agent_name: agent_name.to_owned(),
            cause: SkipCause::InlineApproval,
        };
        let mut phase = phase_fixture();
        phase.candidates = vec![
            candidate(ids::AGENT_CLAUDE, "claude", "opus"),
            candidate(ids::AGENT_AGY, "agy", "gemini-3.7-flash-high"),
            candidate(ids::AGENT_CLAUDE, "claude", "sonnet"),
            candidate(ids::AGENT_CLAUDE_CLI, "claude-cli", "default"),
        ];
        let walk = Walk {
            eligible: vec![phase.candidates[1].clone(), phase.candidates[2].clone()],
            skipped: vec![
                skipped(ids::AGENT_CLAUDE, "claude"),
                skipped(ids::AGENT_CLAUDE_CLI, "claude-cli"),
            ],
        };
        let names = |chosen: &SnapshotCandidate| {
            super::skipped_above(&phase, &walk, chosen)
                .iter()
                .map(|row| row.agent_name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&phase.candidates[1]), ["claude"]);
        assert_eq!(
            names(&phase.candidates[2]),
            ["claude"],
            "`claude`/`sonnet` is eligible even though `claude`/`opus` was skipped"
        );
    }

    /// A gated `prd` phase with no candidates, for tests that fill in only what they read.
    fn phase_fixture() -> SnapshotPhase {
        SnapshotPhase {
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
        }
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

    /// Starts `FEAT-3`, which parks at `prd`, and hands back the run and that parked step.
    ///
    /// Every case that wants an error *inside* a walk needs the run id first, and a `StartRun`
    /// that fails on its first position never hands one over — so the failure is scripted onto
    /// position 1 and reached by approving position 0.
    async fn started(harness: &Harness) -> (htui_core::model::RunId, StepId) {
        harness.free_feat_3().await;
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
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let steps = harness.orch.steps(run).await;
        assert_eq!(steps.len(), 1);
        (run, steps[0].id)
    }

    /// `run.graph_snapshot`, decoded, for a case that has to prove which gate it ran under.
    async fn snapshot_of(harness: &Harness, run: htui_core::model::RunId) -> GraphSnapshot {
        serde_json::from_value(
            harness
                .orch
                .run(run)
                .await
                .graph_snapshot
                .expect("the walk created the run with a snapshot"),
        )
        .expect("the engine wrote a `GraphSnapshot`")
    }

    /// Plan D17: a `run_step` row at a position the run's snapshot has no phase for.
    ///
    /// MOD-2's chat path inserts `run` and `run_step` rows outside §4.3 on purpose, so this is a
    /// shape the walk has to refuse readably rather than assume away. It is an **engine**
    /// invariant — invariant 2 says the walk reads its snapshot and nothing else, so there is no
    /// second source to consult — and reporting it as `StoreError::Constraint` blamed the store
    /// for it. Milestone 6's `run_worker.rs` is the first consumer that will match on the variant.
    #[tokio::test]
    async fn a_position_the_snapshot_does_not_name_is_a_snapshot_refusal() {
        let harness = Harness::new().await;
        let (run, _) = started(&harness).await;

        let orphan = StepId::new();
        harness
            .orch
            .store
            .create_step(NewRunStep {
                id: orphan,
                run_id: run,
                position: 9,
                attempt: 1,
                fanout_index: 0,
                phase_name: "nowhere".to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: Some("sonnet".to_owned()),
            })
            .await
            .expect("`UNIQUE (run_id, position, attempt, fanout_index)` is free at 9");
        let now = harness.orch.clock.now();
        for (from, to) in [
            (StepStatus::Pending, StepStatus::Running),
            (StepStatus::Running, StepStatus::AwaitingApproval),
        ] {
            assert!(
                harness
                    .orch
                    .store
                    .transition_step(orphan, from, to, now)
                    .await
                    .expect("both moves are legal"),
                "`{from}` -> `{to}`"
            );
        }

        let refused = harness
            .dispatch(Command::AnswerGate {
                run,
                step: orphan,
                answer: GateAnswer::Approved,
            })
            .await
            .expect_err("the snapshot names no phase at position 9");
        assert!(
            matches!(
                &refused,
                EngineError::Snapshot { run: id, reason } if *id == run && reason.contains("position 9")
            ),
            "{refused}"
        );
    }

    /// One `prepare` per step, and not two.
    ///
    /// `Isolator::prepare` promises no idempotence (`crate::isolate::Isolator::prepare`) and
    /// `FakeIsolator` mints a fresh `before_hash` on every call, so a second one inside a single
    /// step is a second, *different* base for the same tree — and milestone 3's `gix` layer will
    /// do real work twice. The step's `cwd` is a value stage 2 already computed; carrying it
    /// through one call frame is not the cross-restart state plan D16 forbids.
    #[tokio::test]
    async fn a_walk_prepares_each_step_exactly_once() {
        let harness = Harness::new().await;
        let (run, _) = started(&harness).await;
        assert_eq!(
            harness.orch.isolator.prepares(),
            1,
            "one step has been walked"
        );

        for _ in 0..4 {
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

        assert_eq!(harness.orch.run(run).await.status, RunStatus::Done);
        assert_eq!(harness.orch.steps(run).await.len(), 4);
        assert_eq!(
            harness.orch.isolator.prepares(),
            4,
            "four steps, four `prepare` calls"
        );
    }

    /// ANA-2 `:639`: an error between the `pending -> running` move and the settle is an
    /// unconditioned `running -> failed`, with the orchestrator as its actor.
    ///
    /// Without it every `?` in stages 2 to 6 leaves the step `running` and the run `running`
    /// forever: `cursor` rests on a `running` step (`status.rs:144-152`), both §6.2 guards refuse
    /// one (`command.rs:241-247`, `:274-283`), and no sweep adopts a run whose lease this process
    /// still holds (plan D88). Stage 2 is the shortest of the ten such sites to drive.
    #[tokio::test]
    async fn an_error_after_the_running_move_fails_the_step_and_the_run() {
        let harness = Harness::new().await;
        let (run, prd) = started(&harness).await;
        harness
            .orch
            .isolator
            .refuse_prepare("no checkout for this repo on this box");

        let refused = harness
            .dispatch(Command::AnswerGate {
                run,
                step: prd,
                answer: GateAnswer::Approved,
            })
            .await
            .expect_err("stage 2 refused after the step was already `running`");
        assert!(
            matches!(
                &refused,
                EngineError::Isolate(IsolateError::Refused(why)) if why.contains("no checkout")
            ),
            "{refused}"
        );

        let steps = harness.orch.steps(run).await;
        let live = steps
            .iter()
            .find(|step| step.position == 1)
            .expect("the walk admitted `plan` before stage 2 refused");
        assert_eq!(
            live.status,
            StepStatus::Failed,
            "a step left `running` wedges the run: nothing else in this milestone can move it"
        );
        let row = harness.orch.run(run).await;
        assert_eq!(row.status, RunStatus::Failed);
        assert_eq!(
            row.failure.as_deref(),
            Some("isolation refused: no checkout for this repo on this box"),
            "invariant 7: the reason a human reads is the error's own sentence"
        );
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::Failed,
            "plan D7: `finish_run` mirrors the item"
        );
    }

    /// Plan D85: the claim's lease runs `app_setting.lease_ttl_seconds`, not a constant. The walk
    /// is made to fail at `prd`'s stage 2 so it raises, and a walk that raises writes nothing more
    /// (D107) — the lease on the row is still the one the claim wrote.
    #[tokio::test]
    async fn lease_times_come_from_app_settings() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .orch
            .store
            .set_app_setting("lease_ttl_seconds", serde_json::json!(300));
        harness
            .orch
            .isolator
            .refuse_prepare("no checkout for this repo on this box");
        let claimed_at = harness.orch.clock.now();

        let refused = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect_err("stage 2 refuses `prd`");
        assert!(matches!(refused, EngineError::Isolate(_)), "{refused}");
        let summary = harness
            .orch
            .store
            .runs(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .find(|run| run.id != ids::RUN_2)
            .expect("the refused walk's run exists");
        let run = harness.orch.run(summary.id).await;
        assert_eq!(
            run.lease_expires_at,
            Some(claimed_at + TimeDelta::seconds(300)),
            "the claim's lease is `now + lease_ttl_seconds`"
        );
    }

    /// Plan D86/D107: a heartbeat whose refresh touches zero rows abandons the walk. The walk is
    /// dropped, the isolator releases the run's guards once, the caller reads `LeaseLost`, and no
    /// row moves after the stranger's adoption — the walk writes nothing further.
    #[tokio::test(start_paused = true)]
    async fn a_walk_whose_lease_is_taken_is_abandoned_and_writes_nothing() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        /// Flags its own drop, so the test can tell the walk future really went away.
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let harness = Harness::new().await;
        let (run, _) = started(&harness).await;
        let now = harness.orch.clock.now();
        let ttl = crate::recover::LeaseTimes::from_app(&BTreeMap::new()).ttl;
        // The parked run, unparked by hand and leased to this harness again: a live walk's rows.
        assert!(
            harness
                .orch
                .store
                .transition_run(run, RunStatus::AwaitingApproval, RunStatus::Running, now)
                .await
                .expect("MemStore takes the move")
        );
        assert!(
            harness
                .orch
                .store
                .take_lease(run, ids::BOX, harness.orch.owner(), now, now + ttl)
                .await
                .expect("MemStore takes the lease"),
            "the released lease is this harness's to take back"
        );

        let graphs = harness.orch.graphs();
        let driver =
            |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| harness.orch.driver_for_key(key);
        let scrubber = htui_core::scrub::MinimalScrubber::new([]);
        let engine = super::Engine::new(
            super::fake_parts(&harness.orch, &graphs, &driver, &scrubber)
                .await
                .expect("the harness has a box"),
        );

        let dropped = Arc::new(AtomicBool::new(false));
        let probe = Dropped(Arc::clone(&dropped));
        let walk = async move {
            let _probe = probe;
            std::future::pending::<Result<crate::command::Rest, EngineError>>().await
        };
        let stranger = uuid::Uuid::now_v7();
        let steal = async {
            let adopted = harness
                .orch
                .store
                .adopt_runs(
                    ids::BOX,
                    stranger,
                    now + ttl + TimeDelta::seconds(1),
                    now + ttl + TimeDelta::days(1),
                )
                .await
                .expect("MemStore adopts");
            assert_eq!(
                adopted.iter().map(|row| row.id).collect::<Vec<_>>(),
                [run],
                "the stranger adopts the expired lease"
            );
            (harness.orch.run(run).await, harness.orch.steps(run).await)
        };
        let (walked, (run_after_steal, steps_after_steal)) =
            tokio::join!(engine.walk_leased(run, walk), steal);

        let refused = walked.expect_err("the heartbeat's refresh touched zero rows");
        assert!(
            matches!(refused, EngineError::LeaseLost { run: lost } if lost == run),
            "{refused}"
        );
        assert!(
            dropped.load(Ordering::SeqCst),
            "the walk future was dropped"
        );
        assert_eq!(
            harness.orch.isolator.releases(),
            1,
            "plan D99's release, once"
        );
        assert_eq!(
            harness.orch.run(run).await,
            run_after_steal,
            "nothing written to the run"
        );
        assert_eq!(
            harness.orch.steps(run).await,
            steps_after_steal,
            "nothing written to its steps"
        );
    }

    /// Plan D83 through `start_run`: a real refusal's `Display` names the holding run and the
    /// rule, and the refused run stays `queued`.
    #[tokio::test]
    async fn claim_refused_names_the_rule() {
        let harness = Harness::new().await;
        harness.add_primary_repo().await;
        let (holder, _) = started(&harness).await;

        let refused = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_ANA_2,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect_err("both items hold the whole primary repo");
        let EngineError::ClaimRefused { run, claim } = &refused else {
            panic!("a claim refusal, not {refused}");
        };
        let htui_core::model::Claim::Overlaps { with, rule } = claim else {
            panic!("an overlap, not {claim}");
        };
        assert_eq!(*with, holder, "the parked run still holds its scope");
        assert_eq!(
            refused.to_string(),
            format!("claim refused: overlaps run {holder} ({rule})")
        );
        assert_eq!(harness.orch.run(*run).await.status, RunStatus::Queued);
        assert_eq!(
            harness.orch.item(ids::HTUI_ANA_2).await.status,
            Status::Queued,
            "plan D84: a refused run and its item stay queued"
        );
    }

    /// Plan D81 on the resume path: `touched_paths` edited after `StartRun` to name a repo the
    /// project does not carry is a rule about starting a run, so the live graph is not
    /// comparable and the run walks its snapshot, with the note that says so.
    #[tokio::test]
    async fn resume_walks_a_run_whose_touched_paths_name_an_unknown_repo() {
        let harness = Harness::new().await;
        let (run, _) = started(&harness).await;
        let item = harness.orch.item(ids::HTUI_FEAT_3).await;
        harness
            .orch
            .store
            .update_item(
                item.id,
                item.version,
                ItemPatch {
                    touched_paths: Some(vec!["web:src/**".to_owned()]),
                    author_id: item.created_by,
                    reason: "a qualifier no repo carries".to_owned(),
                    ..ItemPatch::default()
                },
            )
            .await
            .expect("the item's version is current");

        let resumed = harness
            .resume(run)
            .await
            .expect("the run walks its snapshot");
        assert!(matches!(resumed, Resume::Walked(_)), "{resumed:?}");
        let notes = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read");
        assert!(
            notes
                .iter()
                .any(|note| note.body.contains("live graph not comparable")
                    && note.body.contains("`web`")),
            "{notes:?}"
        );
    }

    /// Blueprint A-1: `resume` takes the lease before it resolves or walks, so a run a live
    /// stranger holds is refused and nothing is advanced.
    #[tokio::test]
    async fn resume_refuses_a_run_a_live_stranger_holds() {
        let harness = Harness::new().await;
        let (run, _) = started(&harness).await;
        let now = harness.orch.clock.now();
        assert!(
            harness
                .orch
                .store
                .transition_run(run, RunStatus::AwaitingApproval, RunStatus::Running, now)
                .await
                .expect("MemStore takes the move")
        );
        let adopted = harness
            .orch
            .store
            .adopt_runs(
                ids::BOX,
                uuid::Uuid::now_v7(),
                now,
                now + TimeDelta::days(1),
            )
            .await
            .expect("MemStore adopts");
        assert_eq!(adopted.len(), 1, "the released lease is adoptable at once");
        let before = harness.orch.steps(run).await;

        let refused = harness
            .resume(run)
            .await
            .expect_err("a live lease elsewhere");
        assert!(
            matches!(refused, EngineError::LeaseHeld { run: held } if held == run),
            "{refused}"
        );
        assert_eq!(harness.orch.steps(run).await, before, "nothing was walked");
    }

    /// Plan D87: a walk that rests parked releases the lease (`lease_expires_at = now`), and an
    /// answer from the same process takes it back and walks on.
    #[tokio::test]
    async fn a_parked_walk_releases_its_lease_and_the_answer_takes_it_back() {
        let harness = Harness::new().await;
        let (run, prd) = started(&harness).await;
        let now = harness.orch.clock.now();
        assert_eq!(harness.orch.run(run).await.lease_expires_at, Some(now));

        let CommandOutcome::Answered { rest } = harness
            .dispatch(Command::AnswerGate {
                run,
                step: prd,
                answer: GateAnswer::Approved,
            })
            .await
            .expect("our own released lease is ours to take")
        else {
            panic!("`AnswerGate` answers `Answered`");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval, "parked at `plan`");
        assert_eq!(
            harness.orch.run(run).await.lease_expires_at,
            Some(harness.orch.clock.now()),
            "and released again at that park"
        );
    }

    /// `FEAT-3` with `prd` fanned out to two under its `always` gate: the group parks for a human
    /// at position 0 (plan D49(2)). Hands back the run and the slot's two candidates, by index.
    async fn parked_group(harness: &Harness) -> (htui_core::model::RunId, [StepId; 2]) {
        harness.free_feat_3().await;
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" {
                    phase.fan_out = 2;
                    phase.retry_limit = 1;
                }
            })
            .await;
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
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(0)),
            "an `always` group parks for a human"
        );
        let steps = harness.orch.steps(run).await;
        let member = |index: i32| {
            steps
                .iter()
                .find(|step| step.fanout_index == index)
                .expect("the group has both indices")
                .id
        };
        (run, [member(0), member(1)])
    }

    /// The parked run's released lease, renewed by this harness's owner for a day, and a second
    /// process over the same store (plan D118) to meet it — a live lease it does not hold.
    async fn a_live_stranger(harness: &Harness, run: htui_core::model::RunId) -> Harness {
        assert!(
            harness
                .orch
                .store
                .refresh_lease(
                    run,
                    harness.orch.owner(),
                    harness.orch.clock.now() + TimeDelta::days(1)
                )
                .await
                .expect("MemStore refreshes"),
            "the first process still owns the released lease, and renews it"
        );
        Harness {
            orch: harness.orch.restarted(),
        }
    }

    /// Plan D108 on the three unpark commands `AnswerGate`'s case does not reach: `RetryStep` on
    /// one step, `RetryStep` on a group member (plan D65's `retry_group`) and `SelectFanout` all
    /// meet a live lease another process holds after their pure guard and **before** their first
    /// write — `LeaseHeld`, and no row moved.
    #[tokio::test]
    async fn retry_and_select_meet_a_live_lease_before_their_first_write() {
        let harness = Harness::new().await;
        let (run, prd) = started(&harness).await;
        let other = a_live_stranger(&harness, run).await;
        let before = harness.orch.steps(run).await;
        let refused = other
            .dispatch(Command::RetryStep { run, step: prd })
            .await
            .expect_err("a live lease elsewhere");
        assert!(
            matches!(refused, EngineError::LeaseHeld { run: held } if held == run),
            "{refused}"
        );
        assert_eq!(
            harness.orch.steps(run).await,
            before,
            "retry_step: no `answer_gate(Retried)`, no new attempt"
        );
        assert_eq!(
            harness.orch.run(run).await.status,
            RunStatus::AwaitingApproval
        );

        let harness = Harness::new().await;
        let (run, [first, second]) = parked_group(&harness).await;
        let other = a_live_stranger(&harness, run).await;
        let before = harness.orch.steps(run).await;
        let refused = other
            .dispatch(Command::RetryStep { run, step: second })
            .await
            .expect_err("a live lease elsewhere");
        assert!(
            matches!(refused, EngineError::LeaseHeld { run: held } if held == run),
            "{refused}"
        );
        assert_eq!(
            harness.orch.steps(run).await,
            before,
            "retry_group: the slot is not retired and no group is admitted"
        );

        let refused = other
            .dispatch(Command::SelectFanout {
                run,
                position: 0,
                attempt: 1,
                winner: first,
            })
            .await
            .expect_err("a live lease elsewhere");
        assert!(
            matches!(refused, EngineError::LeaseHeld { run: held } if held == run),
            "{refused}"
        );
        assert_eq!(
            harness.orch.steps(run).await,
            before,
            "select_fanout: no winner `selected`, no loser superseded"
        );
        assert_eq!(
            harness.orch.run(run).await.status,
            RunStatus::AwaitingApproval,
            "and the run was not unparked"
        );
    }

    /// Plan D87 on the same three commands: each takes the lease (`now + ttl`), walks its tail
    /// under it, and gives it back (`lease_expires_at = now`) when the tail parks again.
    #[tokio::test]
    async fn a_retry_or_a_select_that_parks_releases_its_lease() {
        let harness = Harness::new().await;
        let (run, prd) = started(&harness).await;
        let CommandOutcome::Retried { rest, .. } = harness
            .dispatch(Command::RetryStep { run, step: prd })
            .await
            .expect("`retry_limit = 1` permits a second attempt")
        else {
            panic!("`RetryStep` answers `Retried`");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        assert_eq!(
            harness.orch.run(run).await.lease_expires_at,
            Some(harness.orch.clock.now()),
            "retry_step: released at the park"
        );

        let harness = Harness::new().await;
        let (run, [_, second]) = parked_group(&harness).await;
        let CommandOutcome::Retried { rest, .. } = harness
            .dispatch(Command::RetryStep { run, step: second })
            .await
            .expect("a parked group retries as a whole")
        else {
            panic!("`RetryStep` answers `Retried`");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        assert_eq!(
            harness.orch.run(run).await.lease_expires_at,
            Some(harness.orch.clock.now()),
            "retry_group: released at the park"
        );

        let harness = Harness::new().await;
        let (run, [first, _]) = parked_group(&harness).await;
        let CommandOutcome::Selected { rest } = harness
            .dispatch(Command::SelectFanout {
                run,
                position: 0,
                attempt: 1,
                winner: first,
            })
            .await
            .expect("a human selects the parked group's winner")
        else {
            panic!("`SelectFanout` answers `Selected`");
        };
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(1)),
            "the walk went on to `plan` and parked at its gate"
        );
        assert_eq!(
            harness.orch.run(run).await.lease_expires_at,
            Some(harness.orch.clock.now()),
            "select_fanout: released at the park"
        );
    }

    /// ANA-2 `:639` lists "spawn failure" beside driver error and deadline, with **no cell in
    /// §4.2's gate table** — so a session that never opened is not a settle outcome and must not
    /// be parked for a human by `always`. There is no artefact to approve and no session to read.
    #[tokio::test]
    async fn a_spawn_failure_fails_the_step_even_under_an_always_gate() {
        let harness = Harness::new().await;
        harness.orch.script(
            "plan",
            1,
            ScriptedStep::refusing_to_start("no `claude` on PATH"),
        );
        let (run, prd) = started(&harness).await;
        assert_eq!(
            snapshot_of(&harness, run).await.phases[1].gate_effective,
            Gate::Always,
            "the seeded `plan` phase gates `always`, which is the cell that would park a failure"
        );

        let refused = harness
            .dispatch(Command::AnswerGate {
                run,
                step: prd,
                answer: GateAnswer::Approved,
            })
            .await
            .expect_err("the driver refused to start");
        assert!(
            matches!(
                &refused,
                EngineError::Driver(DriverError::Spawn(why)) if why.contains("no `claude` on PATH")
            ),
            "{refused}"
        );

        let steps = harness.orch.steps(run).await;
        let live = steps
            .iter()
            .find(|step| step.position == 1)
            .expect("the walk admitted `plan` before the driver refused");
        assert_eq!(
            live.status,
            StepStatus::Failed,
            "`always` parks a settle, not a spawn failure (ANA-2 `:639`)"
        );
        assert_ne!(
            live.status,
            StepStatus::AwaitingApproval,
            "there is no artefact to approve and no session to read"
        );
        let row = harness.orch.run(run).await;
        assert_eq!(row.status, RunStatus::Failed);
        assert_eq!(
            row.failure.as_deref(),
            Some("agent spawn failed: no `claude` on PATH")
        );
    }

    /// A terminal run has nothing to retry onto: §6.2's `retry` needs a run a human can still
    /// influence, and `command::retry_enabled` deliberately does not check it (`command.rs:266-268`
    /// names the check as the engine's, at dispatch). Without it the guard passes on a `failed`
    /// run, `unpark` writes nothing, stage 1 creates an orphan `pending` step under a run that will
    /// never walk again, and `CommandOutcome::Retried` reports success.
    #[tokio::test]
    async fn retry_on_a_terminal_run_is_refused_and_writes_nothing() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;

        // A kind nothing in the graph produces: the run fails at stage 3 and the step is `failed`
        // at attempt 1, which is exactly the pair `retry_enabled` admits.
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

        let before = harness.orch.steps(run).await;
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].status, StepStatus::Failed);
        assert_eq!((before[0].position, before[0].attempt), (0, 1));

        let refused = harness
            .dispatch(Command::RetryStep {
                run,
                step: before[0].id,
            })
            .await
            .expect_err("a terminal run cannot be retried");
        assert!(
            matches!(
                &refused,
                EngineError::RunStatus {
                    status: RunStatus::Failed,
                    expected: "running | awaiting_approval",
                    ..
                }
            ),
            "{refused}"
        );

        let after = harness.orch.steps(run).await;
        assert_eq!(
            after
                .iter()
                .map(|step| (step.position, step.attempt, step.status))
                .collect::<Vec<_>>(),
            before
                .iter()
                .map(|step| (step.position, step.attempt, step.status))
                .collect::<Vec<_>>(),
            "the refusal writes nothing: no orphan `pending` step at (0,2)"
        );
        assert_eq!(harness.orch.run(run).await.status, RunStatus::Failed);
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
        assert_eq!(
            harness.orch.run(run).await.lease_expires_at,
            Some(harness.orch.clock.now()),
            "blueprint A-1: the lease `resume` took is given back, since nothing walks"
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
                &SessionKey::of(&steps),
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

    /// Walks `FEAT-3` to `done` by approving every gate it parks at.
    ///
    /// # Panics
    /// When a position never parks, which is what the caller is asserting it did.
    async fn walk_to_done(harness: &Harness) -> htui_core::model::RunId {
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
        for position in 0..4 {
            let steps = harness.orch.steps(run).await;
            let step = steps
                .iter()
                .find(|step| {
                    step.position == position && step.status == StepStatus::AwaitingApproval
                })
                .expect("the walk parked at this position");
            harness
                .dispatch(Command::AnswerGate {
                    run,
                    step: step.id,
                    answer: GateAnswer::Approved,
                })
                .await
                .expect("the step produced its document");
        }
        assert_eq!(harness.orch.run(run).await.status, RunStatus::Done);
        run
    }

    /// Plan D36: `Isolator::cleanup` had no caller anywhere until this milestone, and the rule is
    /// "after every terminal `finish_run`, once".
    ///
    /// Once and not four times: ANA-2 invariant 6 (`docs/ANA-2.md:128-131`) forbids cleaning up at
    /// step end, because the next attempt of a superseded step reads the tree the last one left.
    #[tokio::test]
    async fn a_finished_run_is_cleaned_up_once() {
        let harness = Harness::new().await;
        let _run = walk_to_done(&harness).await;
        assert_eq!(
            harness.orch.isolator.cleanups(),
            1,
            "one run, one cleanup, at the end"
        );
    }

    /// Blueprint H-2: a `reconcile` error lands on a step that is already `done`, and `fail_hard`
    /// moves `running -> failed` only — `done -> failed` is illegal (`model/run.rs`).
    ///
    /// So every `reconcile` failure parks the run instead: the step stays `done`, the run and the
    /// item go to `awaiting_approval`, and the reason is an `item_note` a human reads (F-G, R-7).
    /// The run is **not** terminal, so nothing is cleaned up.
    #[tokio::test]
    async fn a_reconcile_refusal_parks_the_run_with_the_step_done() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .repoint(ids::HTUI_FEAT_3, |phase| {
                if phase.name == "prd" {
                    phase.gate = Gate::Never;
                }
            })
            .await;
        harness.orch.isolator.refuse_reconcile("dirty_primary_tree");

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

        assert_eq!(
            (rest.run, rest.position, rest.failure),
            (RunStatus::AwaitingApproval, Some(0), None),
            "a parked run's `failure` stays NULL; the reason is the note"
        );
        let steps = harness.orch.steps(run).await;
        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].status,
            StepStatus::Done,
            "the step settled and passed its gate before `reconcile` ran"
        );
        assert_eq!(
            harness.orch.run(run).await.status,
            RunStatus::AwaitingApproval
        );
        assert_eq!(
            harness.orch.item(ids::HTUI_FEAT_3).await.status,
            Status::AwaitingApproval
        );
        let notes: Vec<String> = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|note| note.body)
            .collect();
        assert!(
            notes.iter().any(|body| body.contains("dirty_primary_tree")),
            "ANA-2 invariant 7: the reason is on the item: {notes:?}"
        );
        assert_eq!(
            harness.orch.isolator.cleanups(),
            0,
            "a parked run is not terminal, so its trees stay"
        );
    }

    /// Blueprint H-2's other half: a `reconcile` that failed rather than refused parks too.
    ///
    /// `IsolateError::Git` is a sentence about the verb and not about the tree — an `index.lock`
    /// that outlived D39's three retries, a hook that exited non-zero — and there is still no
    /// status a `done` step could be failed into. A run parked with the sentence keeps everything
    /// and tells the human what the primary is in; a run failed would have to lie about the step.
    #[tokio::test]
    async fn a_reconcile_that_failed_parks_with_the_git_sentence() {
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
            .isolator
            .fail_reconcile("git merge: Unable to create '.git/index.lock': File exists");

        let CommandOutcome::Started { run, rest } = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("a `reconcile` failure is never re-raised")
        else {
            panic!("`StartRun` answers `Started`");
        };

        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        assert_eq!(
            harness.orch.steps(run).await[0].status,
            StepStatus::Done,
            "`done -> failed` is illegal, which is the whole reason this parks"
        );
        let notes: Vec<String> = harness
            .orch
            .store
            .notes(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|note| note.body)
            .collect();
        assert!(
            notes
                .iter()
                .any(|body| body.contains("git merge: Unable to create")),
            "the verb's own sentence survives to the item: {notes:?}"
        );
    }

    /// The other terminal path plan D36 names: a step that never settled at all.
    ///
    /// `fail_hard` ends the run, so the trees it made have to go the same way a finished run's do.
    #[tokio::test]
    async fn a_hard_failure_still_cleans_up() {
        let harness = Harness::new().await;
        harness.free_feat_3().await;
        harness
            .orch
            .script("prd", 1, ScriptedStep::refusing_to_start("no binary"));

        let refused = harness
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect_err("a driver that cannot start is re-raised after the settle");
        assert!(matches!(refused, EngineError::Driver(_)), "{refused}");
        assert_eq!(
            harness.orch.isolator.cleanups(),
            1,
            "plan D36: after every terminal `finish_run`, `fail_hard`'s included"
        );
    }

    /// Plan D28 through the seam that carries it: a tree outside the session's `cwd` reaches the
    /// driver as `SessionSpec.extra_dirs`, which milestone 2 hard-coded empty.
    ///
    /// The spy refuses to start *after* recording, which is the cheapest way to read a spec out of
    /// a walk: the session never opens, so nothing downstream of stage 4 can rewrite what it saw.
    #[tokio::test]
    async fn a_tree_outside_the_cwd_reaches_the_session_as_an_extra_dir() {
        let orch = FakeOrchestrator::demo();
        orch.store
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, orch.clock.now())
            .await
            .expect("the seeded run is queued and cancellable");
        let elsewhere = std::path::PathBuf::from("/elsewhere/docs");
        orch.isolator.script_extra_dirs(vec![elsewhere.clone()]);

        let seen: std::sync::Arc<std::sync::Mutex<Option<htui_agent::driver::SessionSpec>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let graphs = orch.graphs();
        let spy = seen.clone();
        let driver = move |_candidate: &SnapshotCandidate,
                           _key: &SessionKey<'_>|
              -> Box<dyn htui_agent::driver::AgentDriver> {
            Box::new(SpecSpy { seen: spy.clone() })
        };
        let scrubber = htui_core::scrub::MinimalScrubber::new([]);
        let engine = super::Engine::new(
            super::fake_parts(&orch, &graphs, &driver, &scrubber)
                .await
                .expect("the fixture holds a box"),
        );
        let refused = engine
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect_err("the spy refuses to start");
        assert!(matches!(refused, EngineError::Driver(_)), "{refused}");

        let spec = seen
            .lock()
            .expect("no panic holds the spy's lock")
            .clone()
            .expect("stage 4 built a spec");
        assert_eq!(spec.extra_dirs, vec![elsewhere]);
    }

    /// An [`AgentDriver`](htui_agent::driver::AgentDriver) that records its `SessionSpec` and then
    /// refuses, so a test can read stage 4's argument without opening a session.
    #[derive(Debug)]
    struct SpecSpy {
        seen: std::sync::Arc<std::sync::Mutex<Option<htui_agent::driver::SessionSpec>>>,
    }

    impl htui_agent::driver::AgentDriver for SpecSpy {
        fn name(&self) -> &str {
            "spec-spy"
        }

        fn caps(&self) -> DriverCaps {
            htui_agent::fake::FakeDriver::full_caps()
        }

        fn start<'a>(
            &'a self,
            spec: htui_agent::driver::SessionSpec,
            prompt: String,
        ) -> htui_agent::driver::DriverFuture<'a, Box<dyn htui_agent::driver::AgentSession>>
        {
            Box::pin(async move {
                drop(prompt);
                *self.seen.lock().expect("no panic holds the spy's lock") = Some(spec);
                Err(DriverError::Spawn("the spy never starts".to_owned()))
            })
        }
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
            orch.box_id(),
        )
        .await
        .expect("the stand-in supplies rung 1")
        .snapshot
        .phases
        .remove(0)
    }
}
