//! Resolution: a live `step_graph` becomes the typed `GraphSnapshot` a run is decided by.
//!
//! ANA-2 §4.1's field chains (`docs/ANA-2.md:278-288`), the `topology` digest the snapshot row
//! reserves for this milestone (`crates/htui-core/src/model/run.rs:453`), the call into
//! [`crate::overlap::resolve`] that writes §4.7's scope into the snapshot (plan D79, D81) and
//! refuses an empty scope for an item with a primary repo (plan D14), and the override clone
//! that is deep over `step_graph_phase` and over the source phases' own `skill_binding` rows
//! (ANA-22 §2, MOD-9 D80).
//!
//! Resolution is the **only** place that reads `step_graph_phase`. Once a run exists, the walk
//! reads its own `graph_snapshot` and nothing else (invariant 2, `docs/ANA-2.md:109-113`), which
//! is what makes an edit to a phase mid-run a no-op for that run.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock, PoisonError};

use htui_core::model::{
    Agent, AgentBox, AgentId, Attachment, BindingChange, BoundSkill, BoxId, CapError, EdgeId, Gate,
    GraphBody, GraphSnapshot, Isolation, Item, ItemId, ItemPatch, NewStepGraph, NodeBody,
    PhaseAgent, PhaseId, PhasePart, Project, ProjectCaps, ProjectId, ProjectSettings,
    PromptTemplate, RepoId, ResolvedGraph, ResolvedPhase, RunMode, RuntimeChanges, SkillBinding,
    SkillBindingKey, SnapshotCandidate, SnapshotGraph, SnapshotJudge, SnapshotPersona,
    SnapshotPhase, SnapshotSettings, SnapshotTemplate, StepGraph, StepGraphEdge, StepGraphId,
    StepGraphPhase,
};
use htui_core::model::{
    EdgeKind, GraphScope, ItemFilter, NodeRole, PLAN_SUFFIX, Scope, SnapshotDivide, SnapshotEdge,
    SnapshotPart, SnapshotSeat, SnapshotSource, SourceHow, Split, WorkspaceId, edge_sort_key,
    sort_snapshot_edges,
};
use htui_core::model::{MemoryState, PlanScope, SnapshotMemory};
use htui_core::prompt::{MEMORY_TOKEN_CAP, TokenEstimator, select_memory};
use htui_core::store::{
    BindingFacts, CasOutcome, Result, StoreError, UpdateOutcome, WalkEdge, WalkNode, WriteStore,
    check_attachment, edge_predicate, forward_mask, graph_body_shape_refusal,
    graph_body_topology_refusal, graph_body_walk_refusal, references_no_row, walk_refusal,
};
use serde_json::Value;

/// `deadline_seconds`' built-in default: two hours (`docs/ANA-2.md:286`).
const DEFAULT_DEADLINE_SECONDS: u32 = 7200;
/// `app_setting.max_fan_out`'s built-in default (ANA-2 §10, §5.1's example).
const DEFAULT_MAX_FAN_OUT: u32 = 4;
/// `app_setting.max_agents_per_run`'s built-in default: 8, the maintainer's raise of ANA-2 §10's 6
/// so the seeded `feature` graph with a judged 3-way `implement` (seven agents) runs by default.
/// `0004_max_agents_per_run_default.sql` moves the seeded row to the same figure.
const DEFAULT_MAX_AGENTS_PER_RUN: u32 = 8;

/// What `htui-orch` needs from a store to build a [`GraphSnapshot`] and that no `ReadStore`
/// method answers (plan D19).
///
/// The eleven orchestration reads milestone 1 shipped are **inherent** on `MemStore`
/// (`crates/htui-core/src/store/mem.rs:447-662`) and on `htui-store`'s `Backend`, on no trait,
/// because the tables behind them are not mirrored. A crate generic over `S: ReadStore +
/// WriteStore` therefore cannot reach `resolve_graph`, `phase_agents`, `prompt_template` or the
/// agent registry at all, and three `GraphSnapshot` fields — `SnapshotPhase::candidates`,
/// `SnapshotTemplate::version` and the denormalised `SnapshotCandidate::agent_name` — have no
/// generic source whatsoever. This trait is that source, and it lives here rather than in
/// `htui-core` so invariant 10 holds in both directions: the engine never names a concrete store,
/// and `htui-store` never learns that an engine exists. `fake.rs` implements it for `MemStore`
/// (T3). The host cannot implement it for its store handle (both are foreign there, E0117), so
/// `htui_worker::HostGraphs` wraps a `WorkerHost` and implements it (MOD-4 plan D155; it was
/// `run_worker::BackendGraphs` over a `Backend` until MOD-41 T6). MOD-9
/// added `bound_skills` and MOD-7 milestone 3 `missing_tags`, for the same reason.
///
/// `app_setting` is deliberately **not** a method of this trait (blueprint A-2): those reads are
/// inherent too, so [`resolve`] takes the resolved map as a parameter, the shape
/// `htui_core::prompt::settings::resolve_budget` already uses.
///
/// Plain `async fn` with the targeted allow, mirroring `htui-core`'s own store seam
/// (`crates/htui-core/src/store/traits.rs:62`): the engine is already generic over `S: WorkerStore`
/// and takes `G` beside it, so no `dyn GraphSource` is ever formed and a boxed-future alias in the
/// shape of `htui_agent::driver::DriverFuture` would buy an allocation per read and nothing else.
#[allow(async_fn_in_trait)]
pub trait GraphSource: Sync {
    /// The graph an item runs under — `item.step_graph_id`, else the kind's `default_graph_id`
    /// (`docs/ANA-2.md:274-276`) — with its phases in `position` order. `None` when the item, its
    /// kind or the graph is gone.
    ///
    /// # Errors
    /// The backend's own failures.
    async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>>;

    /// Rung 1 of ANA-2 §4.1's candidate chain (`:288`): the phase's `phase_agent` rows in
    /// `position` order.
    ///
    /// An implementation over `MemStore` answers empty unconditionally — the store holds no such
    /// table — which is why T3's fake folds its stand-in candidates in here (plan D20).
    ///
    /// # Errors
    /// The backend's own failures.
    async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>>;

    /// One `prompt_template` by `(project, name)`: the pinned `version` when there is one, else
    /// the highest. `None` when the pin cannot be honoured.
    ///
    /// # Errors
    /// The backend's own failures.
    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>>;

    /// One `agent` row, for the denormalised `agent_name` of ANA-2 §5.1 and for
    /// `htui_agent::registry::caps_for`, which the stage-1 interlock calls before a driver exists
    /// to ask (plan D6).
    ///
    /// # Errors
    /// The backend's own failures.
    async fn agent(&self, id: AgentId) -> Result<Option<Agent>>;

    /// Every `agent_box` row of one box (plan D60, D62): rung 3 of ANA-2 §4.1's candidate chain
    /// reads it at resolution, and stage 1's `R-AGT-8` walk reads it fresh before every step,
    /// because the recorder latches quota mid-run and a listing taken earlier would be stale.
    ///
    /// # Errors
    /// The backend's own failures.
    async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>>;

    /// One step's skill candidates (MOD-9 D44): the global attachments, `project`'s, and — with
    /// `phase` — that phase's, resolved most-specific-wins by `htui_core::model::skill::resolve`
    /// and ordered `(position, name bytes)`. Inactive winners (`off`, `glob`, a pin with no
    /// version) are **included**: the assembler's `select` decides and records them.
    ///
    /// Inherent on both stores and on `Backend` (`pg/read.rs:1433`, `mem.rs:425`,
    /// `backend.rs:364`) for `prompt_template`'s reason: `skill*` is not mirrored.
    ///
    /// # Errors
    /// The backend's own failures; `Backend` offline refuses with `PROMPT_ON_SERVER_ONLY`.
    async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>>;

    /// `R-ORCH-10`'s read (MOD-7 milestone 3, D75): the entries of `item.required_tags` that
    /// `box_id` has neither probed nor declared (`box.probed_tags ∪ box.declared_tags`), compared
    /// as exact bytes, sorted by bytes and deduplicated. Empty when the box can run the item, and
    /// for an item that requires nothing. Inherent on both stores and on `Backend` (`mem.rs:678`,
    /// `pg/read.rs:1919`, `backend.rs:561`) for `prompt_template`'s reason: `box` is mirrored,
    /// but the mirror runs no orchestration reads.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown item or box, the item first; the backend's own
    /// failures; `Backend` offline refuses with its orchestration sentence.
    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>>;
}

/// What [`resolve`] answers: the snapshot and the scope, ready for
/// `NewRun` (`crates/htui-core/src/model/run.rs:352-378`).
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// `run.graph_snapshot`, with [`topology`] already computed.
    pub snapshot: GraphSnapshot,
    /// `run.repo_scope` (ANA-2 §4.7), from [`crate::overlap::resolve`].
    pub repo_scope: Vec<RepoId>,
}

/// MOD-141 M2 D4: the run limits [`resolve`] freezes into [`SnapshotSettings`], over the same
/// `app_setting` keys and code defaults; the graph editor's header shows exactly these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunBounds {
    /// `app_setting.max_fan_out`, else 4.
    pub max_fan_out: u32,
    /// `app_setting.max_agents_per_run`, else 8.
    pub max_agents_per_run: u32,
    /// `app_setting.max_parallel_sessions`, else
    /// [`SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS`].
    pub max_parallel_sessions: u32,
    /// `app_setting.max_plan_revisions`, else [`SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS`].
    pub max_plan_revisions: u32,
    /// `app_setting.max_child_runs`, else [`SnapshotSettings::DEFAULT_MAX_CHILD_RUNS`].
    pub max_child_runs: u32,
    /// `app_setting.runtime_changes`, else `ask`.
    pub runtime_changes: RuntimeChanges,
}

/// Why a graph could not be resolved into a snapshot.
///
/// Every variant is a refusal before a run row exists, which is the cheap end of ANA-2 §4.2's
/// "fail before a token is spent".
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Neither the item nor its kind names a graph (`docs/ANA-2.md:274-276`).
    #[error("item {0} has no step graph")]
    NoGraph(ItemId),
    /// A phase names a template the project does not hold at any version.
    #[error("phase `{phase}` names template `{name}` which project {project} does not hold")]
    NoTemplate {
        /// `step_graph_phase.name`.
        phase: String,
        /// `step_graph_phase.template_name`.
        name: String,
        /// The project searched.
        project: ProjectId,
    },
    /// ANA-2 §4.1 rung 4: `phase_agent` is empty, the project names no `default_agent_id`, and
    /// nothing else may be guessed (`docs/ANA-2.md:288`).
    #[error("phase `{phase}` has no candidate agent (ANA-2 §4.1 rung 4)")]
    NoCandidate {
        /// `step_graph_phase.name`.
        phase: String,
    },
    /// A cap key in `project.settings` is not a non-negative integer (or the blob is not an
    /// object). A run with an unreadable cap never starts: the cap is a spend guard, and reading
    /// it as absent would run uncapped.
    #[error("project {project}: {cause}")]
    ProjectCaps {
        /// The project whose settings were read.
        project: ProjectId,
        /// The offending key and what was found. Not named `source`: `CapError` is no `Error`.
        cause: CapError,
    },
    /// A candidate names an `agent` row that is not there, so `agent_name` cannot be denormalised.
    #[error("phase `{phase}` candidate {agent} names no agent row")]
    NoAgentRow {
        /// `step_graph_phase.name`.
        phase: String,
        /// The `agent.id` that resolved to nothing.
        agent: AgentId,
    },
    /// `local` isolation shares the maintainer's own working tree, so two candidates would write
    /// over each other (ANA-2 §4.6).
    #[error("phase `{phase}` is `local` with fan_out {fan_out} (ANA-2 §4.6)")]
    LocalFanOut {
        /// `step_graph_phase.name`.
        phase: String,
        /// `step_graph_phase.fan_out`.
        fan_out: i32,
    },
    /// MOD-138 M3 D7: two non-palette nodes that can be live together (neither reaches the other
    /// over forward edges, [`forward_mask`]'s classification) both resolve to `local`, the
    /// maintainer's one working tree. The first pair in position order. Every phase resolves to
    /// `phase.isolation.unwrap_or(default_isolation)`, so a per-phase `worktree` override on
    /// either node is the remedy, a `local` project's included.
    #[error(
        "phases `{first}` and `{second}` are both `local` and can run at the same time (neither \
         reaches the other over forward edges), and `local` shares one working tree; set \
         `worktree` isolation on one of them (MOD-138 M3 D7)"
    )]
    LocalConcurrent {
        /// The earlier node's key, by position.
        first: String,
        /// The later node's key.
        second: String,
    },
    /// Plan D64 (OQ-5): a `review` phase cannot fan out, because the loop's gate reads one review
    /// verdict and a group's review would need a judge of reviews nothing defines. MOD-138 M2 D10
    /// widens it from the name to the rule: any node that reads its verdict (an outgoing
    /// `changes_requested` edge) is refused too, and a phase called `review` still is.
    #[error(
        "phase `{phase}` is a review with fan_out {fan_out}; a review cannot fan out (plan D64)"
    )]
    ReviewFanOut {
        /// `step_graph_phase.name`.
        phase: String,
        /// `step_graph_phase.fan_out`.
        fan_out: i32,
    },
    /// Plan D63: a phase fans out above `app_setting.max_fan_out`, refused at admission and
    /// never silently truncated (ANA-2 `:868`).
    #[error("phase `{phase}` fans out {fan_out}; max_fan_out is {max} (ANA-2 :868)")]
    FanOutCap {
        /// `step_graph_phase.name`.
        phase: String,
        /// `step_graph_phase.fan_out`.
        fan_out: i32,
        /// [`SnapshotSettings::max_fan_out`], resolved.
        max: u32,
    },
    /// Plan D63 (OQ-1): `Σ fan_out` plus one judge per judged phase exceeds
    /// `app_setting.max_agents_per_run` (ANA-2 `:870-875`). Retry attempts are not counted.
    #[error("run plans {planned} agents; max_agents_per_run is {max} (ANA-2 :870-875)")]
    AgentCap {
        /// The planned agent count.
        planned: u32,
        /// [`SnapshotSettings::max_agents_per_run`], resolved.
        max: u32,
    },
    /// Plan D14: an empty `repo_scope` overlaps nothing, because `'{}' && x` is false in Postgres,
    /// while ANA-2 §4.7 intends an empty `touched_paths` to overlap the whole primary repo. An
    /// item queued with an empty scope would silently defeat `claim_run`'s admission predicate, so
    /// resolution — which is where paths become a scope — is where the hole is closed.
    #[error("item {item} names primary repo {repo}; an empty repo_scope is refused (plan D14)")]
    EmptyScopeWithPrimary {
        /// The item being queued.
        item: ItemId,
        /// The project's `is_primary` repo.
        repo: RepoId,
    },
    /// Plan D119: a `touched_paths` entry is qualified with a repo slug the project holds no
    /// `repo` row for (ANA-2 §4.7 `:1025`'s `repo:glob`). Refused whether or not the caller also
    /// requested a scope, because the name is wrong either way.
    #[error(
        "item {item} touches repo `{name}`, which the project does not carry (ANA-2 §4.7 `:1025`)"
    )]
    UnknownTouchedRepo {
        /// The item being queued.
        item: ItemId,
        /// The qualifier, as written before the `:`.
        name: String,
    },
    /// MOD-138 D9: the shared validator refused the live graph, or its frozen expansion. The
    /// reason is the sentence `replace_graph` would give for the same body.
    #[error("graph `{graph}` is not a valid step graph: {reason}")]
    InvalidGraph {
        /// `step_graph.name`.
        graph: String,
        /// The validator's sentence.
        reason: String,
    },
    /// MOD-138 D6: a graph M1 freezes but cannot walk yet; lifted by MOD-139, MOD-140 and M3.
    /// M2 lifted (c), authored edges (D10).
    #[error("phase `{node}` cannot run yet: {reason}")]
    NotYetRunnable {
        /// The node refused: the divided node (its planner agent, no default parts, or a
        /// `plan_scope` past its parts; MOD-139 M3 D10), the palette phase, or the fanned-out node
        /// with an `asks_you` or `fails` edge (blueprint A7). M3 D8 moved R1's mixed `asks_you` node to
        /// the shared validator (W5), so it is an [`ResolveError::InvalidGraph`].
        node: String,
        /// Why.
        reason: NotRunnable,
    },
    /// MOD-139 M3 D10 (blueprint P-11): a default-parts divided node admission would refuse,
    /// refused before any run exists. The sentence is the `RunFailure`'s, which names the node.
    #[error("{reason}")]
    DivideRefused {
        /// The divided node's name.
        node: String,
        /// What `divide::admission_refusal` answered.
        reason: crate::status::RunFailure,
    },
    /// The store said no.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// MOD-138 D6: why a graph this version cannot walk is refused before its run exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotRunnable {
    /// MOD-139 M2 D17 (PRD Q1): a divided node with a planner agent configured; MOD-140 runs it.
    #[error("a divided node's planner agent runs from MOD-140 on")]
    PlannerAgent,
    /// MOD-139 M3 D10: a divided node with no default parts; only its planner can plan them.
    #[error("a divided node with no default parts needs its planner, which runs from MOD-140 on")]
    NoParts,
    /// MOD-139 M3 D10: a divided node whose planner may plan past its own parts.
    #[error(
        "a divided node that plans past its own parts (`plan_scope` `successors` or `graph`) runs from MOD-140 and MOD-141 on"
    )]
    PlanScope,
    /// (b) A palette node.
    #[error("a palette node is placed by a planner, which runs from MOD-140 on")]
    Palette,
    /// Blueprint A7 (MOD-138 M2): a fanned-out node with an outgoing `asks_you` or `fails`
    /// edge. Group routing runs neither the `asks_you` stop nor the `fails` loop, so its winner
    /// would decline every `asks_you` edge in silence and a group failure would never loop. (M2
    /// lifted D6(c): authored edges otherwise resolve.)
    #[error("a fanned-out node's `asks_you` or `fails` edge runs from MOD-138 M3")]
    FanOutEdge,
}

/// `"sha256:"` + hex over `serde_json::to_string` of the **typed** phase slice (plan D9).
///
/// ANA-2 defines the hash as "over the canonical serialisation of `phases[]`" (`:1479`) and never
/// defines "canonical", so this is where it is defined: the envelope (`v`, `graph`, `mode`,
/// `settings`, `scope`, `personas`, `memory`) is excluded, field order is [`SnapshotPhase`]'s
/// declaration order and nulls are emitted, **except `persona`** (MOD-26 D9), which is skipped
/// when `None`, so a persona-less phase serialises exactly as before and only a bound phase's
/// digest names its persona. Any other field added to `SnapshotPhase` changes every digest and is a
/// `GraphSnapshot::V` question.
///
/// **Never route this through `serde_json::Value`.** `serde_json`'s `preserve_order` feature is
/// enabled in a whole-workspace build — `schemars` ← `agent-client-protocol-schema` ←
/// `htui-agent`, and Cargo feature unification hands it to `htui-core` too — and disabled under
/// `cargo test -p htui-orch`. A `Value` round trip therefore serialises in declaration order in
/// one build and alphabetical order in the other, yielding two digests for one graph. Serialising
/// the struct directly is immune, and this is a measured trap rather than a style preference.
///
/// # Errors
/// [`ResolveError::Store`] carrying the serialiser's own sentence. `SnapshotPhase` has no map keys
/// and no floats, so nothing in the shipped type can trip it — but a `.expect` here would be a
/// panic on a production path, and a field added to `SnapshotPhase` later is exactly the change
/// that would reach it.
pub fn topology(phases: &[SnapshotPhase]) -> std::result::Result<String, ResolveError> {
    let json = serde_json::to_string(phases).map_err(|err| {
        ResolveError::Store(StoreError::Constraint(format!(
            "the graph's phases do not serialise: {err}"
        )))
    })?;
    Ok(format!(
        "sha256:{}",
        htui_core::prompt::digest::sha256_hex(&json)
    ))
}

/// Builds ANA-2 §5.1's snapshot from the live graph, walking §4.1's chains once per field.
///
/// `app` is the resolved `app_setting` map — `MemStore::app_settings()` in a harness,
/// `Backend::app_settings()` at milestone 6 — passed rather than read through [`GraphSource`],
/// because the `app_setting` reads are inherent too (blueprint A-2).
///
/// `box_id` is the box the run will target: rung 3 of the candidate chain is "the single enabled
/// agent on the box" (plan D62), so which box is a resolution input.
///
/// `mode` is recorded on the snapshot and decides `gate_effective`: in auto mode every phase whose
/// gate is not hard is snapshotted `never` (`effective_gate`, MOD-12 D10). `resume` re-resolves
/// under the run's own mode (`engine.rs:3036-3044`), so the topology an auto run is compared
/// against is downgraded the same way (H-3).
///
/// The graph may be a workspace or library graph (MOD-138 D12): its scope is frozen in
/// `graph.scope`, and its phases' templates and personas still resolve in the **item's** project
/// (`prompt_template(project.id, ..)`), so one shared graph runs with each project's templates.
///
/// A resolved graph's snapshot also freezes the item's project memory (MOD-163 D11, D12): its
/// `Active` entries under the cap, copied with their text, so every step, retry and resume of the
/// run renders the same bytes.
///
/// # Errors
/// Any [`ResolveError`]; a missing `project` row is [`StoreError::NotFound`].
pub async fn resolve<S: htui_core::store::WorkerStore, G: GraphSource>(
    store: &S,
    source: &G,
    item: &Item,
    mode: RunMode,
    app: &BTreeMap<String, Value>,
    requested_scope: Option<&[RepoId]>,
    box_id: BoxId,
) -> std::result::Result<Resolved, ResolveError> {
    match Box::pin(resolve_or_refused(
        store,
        source,
        item,
        mode,
        app,
        requested_scope,
        box_id,
    ))
    .await?
    {
        Resolution::Resolved(resolved) => {
            let mut resolved = *resolved;
            // MOD-163 D11, D12 (maintainer decision P-1): the project's active memory is read
            // here, on a resolved graph only, so neither a refused resolution nor a v1 run's
            // resume (`resolve_or_refused`, phases only) reads it or can fail on it. Boxed for
            // the 2 MiB debug stack, as the graph read is.
            resolved.snapshot.memory = Box::pin(frozen_memory(store, item.project_id)).await?;
            Ok(resolved)
        }
        Resolution::Refused { refusal, .. } => Err(refusal),
    }
}

/// MOD-163 D11, D12: the project's `Active` memory entries, selected under
/// [`MEMORY_TOKEN_CAP`] by the shared pure `select` and frozen with their text — what
/// `GraphSnapshot.memory` holds for the whole run. `None` when the project has no active entry,
/// and when the store answers `None` ("not available here"), which is no entry.
async fn frozen_memory<S: htui_core::store::WorkerStore>(
    store: &S,
    project: ProjectId,
) -> std::result::Result<Option<SnapshotMemory>, ResolveError> {
    Ok(store
        .memory_entries(project, Some(vec![MemoryState::Active]))
        .await?
        .and_then(|entries| select_memory(&entries, MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT)))
}

/// What [`resolve_or_refused`] found (MOD-138 blueprint A6).
pub(crate) enum Resolution {
    /// The graph resolved: what [`resolve`] returns.
    Resolved(Box<Resolved>),
    /// A D6, D9 or M3 D7 refusal raised once the phases were built: a
    /// [`ResolveError::NotYetRunnable`], a [`ResolveError::InvalidGraph`] or a
    /// [`ResolveError::LocalConcurrent`]. `phases` are the live phases in position order,
    /// before any `divide` expansion (MOD-138 D11), which is the shape a v1 run is compared on.
    Refused {
        /// The live phases, as built.
        phases: Vec<SnapshotPhase>,
        /// The refusal [`resolve`] raises.
        refusal: ResolveError,
    },
}

/// [`resolve`], with its D6 and D9 refusals (blueprint A6) and M3 D7's `local` refusal handed
/// back beside the live phases built before them, so a v1 run's resume can compare those phases
/// on the v1 digest. Every other error is [`resolve`]'s, in its order. The shared validator's
/// refusal of the stored body comes before the phases are built, so it is held while they are; a
/// phase that then fails to build raises the validator's refusal, as [`resolve`] always has, with
/// no phases to compare.
///
/// # Errors
/// As [`resolve`], less the refusals carried by [`Resolution::Refused`].
pub(crate) async fn resolve_or_refused<S: htui_core::store::WorkerStore, G: GraphSource>(
    store: &S,
    source: &G,
    item: &Item,
    mode: RunMode,
    app: &BTreeMap<String, Value>,
    requested_scope: Option<&[RepoId]>,
    box_id: BoxId,
) -> std::result::Result<Resolution, ResolveError> {
    // Boxed: PgStore's read now holds a transaction, and the engine awaits this inside its own
    // futures (MOD-138 blueprint §4, the 2 MiB debug stack).
    let resolved = Box::pin(source.resolve_graph(item.id))
        .await?
        .ok_or(ResolveError::NoGraph(item.id))?;
    let project = store
        .project(item.project_id)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "project",
            id: item.project_id.to_string(),
        })?;
    // The caps are read strictly and first: a malformed cap refuses the resolve, and a malformed
    // *other* key must not turn the caps into `None` (the lenient decode below defaults the whole
    // struct), so the snapshot's caps never come from `project_settings`.
    let caps = ProjectCaps::from_settings(&project.settings).map_err(|cause| {
        ResolveError::ProjectCaps {
            project: project.id,
            cause,
        }
    })?;
    let settings = project_settings(&project);

    let ResolvedGraph {
        graph,
        phases: mut rows,
        edges: stored,
    } = resolved;
    // MOD-138 D10, D9: the body the store would hold, judged by the shared validator before a
    // phase is resolved. Blueprint A6: its refusal is held until the phases are built.
    let invalid = match live_graph_refusal(&graph, &rows, &stored) {
        Ok(()) => None,
        Err(refusal @ ResolveError::InvalidGraph { .. }) => Some(refusal),
        Err(err) => return Err(err),
    };

    // `UNIQUE (graph_id, position)` permits gaps, so the snapshot builder is what makes positions
    // dense `0..n-1` (`docs/ANA-2.md:269-272`). Sorting first means a gapped or out-of-order table
    // still yields the graph's own order rather than the store's.
    rows.sort_by_key(|row| row.phase.position);

    let built = async {
        let mut phases = Vec::with_capacity(rows.len());
        // MOD-26 D9 (B-18): each bound persona frozen once, by name and content.
        let mut personas: Vec<SnapshotPersona> = Vec::new();
        for (dense, row) in rows.iter().enumerate() {
            let position = i32::try_from(dense).map_err(|_| {
                StoreError::Constraint(format!("graph {} has more phases than i32", graph.id))
            })?;
            let mut phase = snapshot_phase(
                source, row, position, project.id, &settings, app, box_id, mode,
            )
            .await?;
            phase.persona = frozen_persona(row, &mut personas)?;
            phases.push(phase);
        }
        personas.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        Ok::<_, ResolveError>((phases, personas))
    }
    .await;
    let (phases, personas) = match (built, invalid) {
        (Ok(built), None) => built,
        (Ok((phases, _)), Some(refusal)) => return Ok(Resolution::Refused { phases, refusal }),
        // `resolve`'s order: the validator's refusal is raised before any phase's.
        (Err(_), Some(refusal)) | (Err(refusal), None) => return Err(refusal),
    };

    // MOD-138 D2: the stored edges by node key, in the one snapshot order (F13). Every end names
    // a row of this read: `live_graph_refusal` refused the rest as a graph that moved.
    let key = |id: PhaseId| {
        rows.iter()
            .find(|row| row.phase.id == id)
            .map(|row| row.phase.name.clone())
            .unwrap_or_default()
    };
    let mut edges: Vec<SnapshotEdge> = stored
        .iter()
        .map(|edge| SnapshotEdge {
            from: key(edge.from),
            to: key(edge.to),
            kind: edge.kind,
            predicate: edge_predicate(edge).cloned(),
            loop_limit: edge.loop_limit,
        })
        .collect();
    sort_snapshot_edges(&mut edges, &phases);

    // MOD-138 D11, then PRD Q1: the expansion is frozen and the validator checks it, before D6
    // refuses what this version cannot walk. Each refusal carries the unexpanded phases (A6).
    let unexpanded = phases.clone();
    let (phases, edges) = match expand_divide(&graph.name, phases, edges) {
        Ok(expanded) => expanded,
        Err(refusal @ ResolveError::InvalidGraph { .. }) => {
            return Ok(Resolution::Refused {
                phases: unexpanded,
                refusal,
            });
        }
        Err(err) => return Err(err),
    };
    if let Some(reason) = snapshot_walk_refusal(graph.id, &phases, &edges) {
        return Ok(Resolution::Refused {
            phases: unexpanded,
            refusal: ResolveError::InvalidGraph {
                graph: graph.name,
                reason,
            },
        });
    }
    // MOD-138 M2 D10, blueprint A7: a fanned-out node that reads its verdict is the plain
    // `ReviewFanOut` error; one with an `asks_you` or `fails` edge cannot run yet.
    match fan_out_edge_refusal(&phases, &edges) {
        Ok(()) => {}
        Err(refusal @ ResolveError::NotYetRunnable { .. }) => {
            return Ok(Resolution::Refused {
                phases: unexpanded,
                refusal,
            });
        }
        Err(err) => return Err(err),
    }
    if let Err(refusal) = not_yet_runnable(&phases) {
        return Ok(Resolution::Refused {
            phases: unexpanded,
            refusal,
        });
    }

    // M3 D7: carried like D6's, so a v1 run walking a `local` graph M2 resolved is still compared
    // on its phases at resume, and walks on while they match.
    if let Err(refusal) = local_concurrent_refusal(&phases, &edges) {
        return Ok(Resolution::Refused {
            phases: unexpanded,
            refusal,
        });
    }

    let bounds = run_bounds(app);
    let settings = SnapshotSettings {
        default_isolation: settings.default_isolation,
        per_token_cap_run: caps.run_micros,
        per_token_cap_batch: caps.batch_micros,
        max_fan_out: bounds.max_fan_out,
        max_agents_per_run: bounds.max_agents_per_run,
        // MOD-138 D13: the bounds, frozen over their code defaults like the caps above.
        max_parallel_sessions: bounds.max_parallel_sessions,
        max_plan_revisions: bounds.max_plan_revisions,
        max_child_runs: bounds.max_child_runs,
        runtime_changes: bounds.runtime_changes,
    };
    check_fan_out_caps(&phases, &settings)?;

    // Read before the literal: §4.7's scope is written into the snapshot (plan D79), and it is
    // resolved from the same `touched_paths` as `repo_scope`, so the two cannot disagree.
    let repos = store.repos(item.project_id).await?;
    let (repo_scope, scope) = crate::overlap::resolve(item, &repos, &phases, requested_scope)?;

    let snapshot = GraphSnapshot {
        v: GraphSnapshot::V,
        graph: SnapshotGraph {
            id: graph.id,
            name: graph.name,
            is_override: graph.is_override,
            scope: Some(graph.scope),
            revision: Some(graph.revision),
        },
        // MOD-138 D1: `item.step_graph_id` (an override included) pins the graph.
        source: Some(SnapshotSource {
            how: if item.step_graph_id.is_some() {
                SourceHow::Pin
            } else {
                SourceHow::KindDefault
            },
        }),
        topology: topology_v2(&phases, &edges)?,
        mode,
        phases,
        edges,
        settings,
        scope: Some(scope),
        personas,
        // MOD-163 maintainer decision P-1: [`resolve`] freezes the memory on a resolved graph;
        // a v1 run's resume, which keeps only these phases, never reads it.
        memory: None,
    };
    // MOD-139 M3 D10 (blueprint P-11): what admission would refuse, over the frozen settings and
    // expanded phases; after every `NotYetRunnable`, so each of those outranks it.
    if let Some(refusal) = divide_refusal(&snapshot) {
        return Err(refusal);
    }

    Ok(Resolution::Resolved(Box::new(Resolved {
        snapshot,
        repo_scope,
    })))
}

/// MOD-138 D10, D9: the live graph `resolve` read, judged before a phase is resolved.
///
/// An edge whose end is not a phase of the read is a graph that moved while it was being read
/// (the `override_graph` precedent): a `Constraint` saying so, so the caller starts again. Then
/// the shared validator's topology and walk rules over the same body `replace_graph` judges, so a
/// body the store would refuse at save is refused here with the same sentence.
fn live_graph_refusal(
    graph: &StepGraph,
    rows: &[ResolvedPhase],
    edges: &[StepGraphEdge],
) -> std::result::Result<(), ResolveError> {
    let read = |id: PhaseId| rows.iter().any(|row| row.phase.id == id);
    if edges.iter().any(|edge| !read(edge.from) || !read(edge.to)) {
        return Err(ResolveError::Store(StoreError::Constraint(format!(
            "graph `{}` moved while it was being resolved; start the run again",
            graph.name
        ))));
    }
    let body = GraphBody {
        graph: graph.clone(),
        nodes: rows
            .iter()
            .map(|row| NodeBody {
                phase: row.phase.clone(),
                agents: row.agents.clone(),
                parts: row.parts.clone(),
            })
            .collect(),
        edges: edges.to_vec(),
    };
    match graph_body_topology_refusal(graph.id, &body)
        .or_else(|| graph_body_walk_refusal(graph.id, &body))
    {
        Some(reason) => Err(ResolveError::InvalidGraph {
            graph: graph.name.clone(),
            reason,
        }),
        None => Ok(()),
    }
}

/// MOD-138 D6: what M1 freezes but cannot walk, first refusal wins: (0) MOD-139 M2 D17, the first
/// planner node, in position order, whose divided node configures a planner agent (named by the
/// divided node; checked first, so it beats every other divided-node refusal); (1) MOD-139 M3 D10,
/// per planner node in position order, a divided node with no default parts (`NoParts`), then one
/// whose `plan_scope` is neither unset nor `parts` (`PlanScope`); (b) the first palette node. M2
/// lifted (c), the refusal of authored edges (D10): the readiness walker runs them; M3 lifted (a),
/// the refusal of every divided node: a default-parts node runs, refused only by what admission
/// would refuse ([`divide_refusal`]).
fn not_yet_runnable(phases: &[SnapshotPhase]) -> std::result::Result<(), ResolveError> {
    let refuse = |node: &str, reason| {
        Err(ResolveError::NotYetRunnable {
            node: node.to_owned(),
            reason,
        })
    };
    if let Some(phase) = phases.iter().find(|phase| {
        phase.role == NodeRole::Planner
            && phase
                .divide
                .as_ref()
                .is_some_and(|divide| divide.planner.agent_id.is_some())
    }) {
        return refuse(
            crate::divide::divided_name(phase),
            NotRunnable::PlannerAgent,
        );
    }
    for phase in phases
        .iter()
        .filter(|phase| phase.role == NodeRole::Planner)
    {
        let name = crate::divide::divided_name(phase);
        if phase
            .divide
            .as_ref()
            .is_none_or(|divide| divide.default_parts.is_empty())
        {
            return refuse(name, NotRunnable::NoParts);
        }
        if !matches!(phase.plan_scope, None | Some(PlanScope::Parts)) {
            return refuse(name, NotRunnable::PlanScope);
        }
    }
    if let Some(phase) = phases.iter().find(|phase| phase.palette) {
        return refuse(&phase.name, NotRunnable::Palette);
    }
    Ok(())
}

/// MOD-139 M3 D10 (blueprint P-11): the first planner node, in position order, whose default
/// parts `divide::admission_refusal` refuses, as [`ResolveError::DivideRefused`] named by the
/// divided node. Run over the frozen snapshot (its settings and expanded phases), so resolve
/// refuses before any run exists what admission would refuse once one did.
fn divide_refusal(snapshot: &GraphSnapshot) -> Option<ResolveError> {
    snapshot
        .phases
        .iter()
        .filter(|phase| phase.role == NodeRole::Planner)
        .find_map(|node| {
            let mut parts = node.divide.as_ref()?.default_parts.clone();
            parts.sort_by_key(|part| part.position);
            crate::divide::admission_refusal(snapshot, node, &parts).map(|reason| {
                ResolveError::DivideRefused {
                    node: crate::divide::divided_name(node).to_owned(),
                    reason,
                }
            })
        })
}

/// MOD-138 M2 D10 and blueprint A7, over every fanned-out node in position order, first refusal
/// wins: a node that reads its verdict (an outgoing `changes_requested` edge, the rule
/// `GraphSnapshot::reads_verdict` reads) is [`ResolveError::ReviewFanOut`], the plain error
/// `snapshot_phase`'s name check raises; a node with an outgoing `asks_you` or `fails` edge is
/// [`NotRunnable::FanOutEdge`].
fn fan_out_edge_refusal(
    phases: &[SnapshotPhase],
    edges: &[SnapshotEdge],
) -> std::result::Result<(), ResolveError> {
    let mut fanned: Vec<&SnapshotPhase> = phases.iter().filter(|phase| phase.fan_out > 1).collect();
    fanned.sort_by_key(|phase| phase.position);
    for phase in fanned {
        let out = |kinds: &[EdgeKind]| {
            edges
                .iter()
                .any(|edge| edge.from == phase.name && kinds.contains(&edge.kind))
        };
        if out(&[EdgeKind::ChangesRequested]) {
            return Err(ResolveError::ReviewFanOut {
                phase: phase.name.clone(),
                fan_out: phase.fan_out,
            });
        }
        if out(&[EdgeKind::AsksYou, EdgeKind::Fails]) {
            return Err(ResolveError::NotYetRunnable {
                node: phase.name.clone(),
                reason: NotRunnable::FanOutEdge,
            });
        }
    }
    Ok(())
}

/// MOD-138 M3 D7, once the expansion passed the walk checks (so the forward edges are acyclic):
/// the first pair, in position order, of non-palette nodes that both resolved to
/// [`Isolation::Local`] and that neither reaches the other over forward edges (`next`, `if`, a
/// forward `asks_you`: [`forward_mask`], the walker's own split) is
/// [`ResolveError::LocalConcurrent`]. Such nodes can be live together, and `local` is one
/// working tree. The reachability is built here over the snapshot's node keys; an edge naming a
/// palette node or an unknown key is left out, as the walk leaves it out.
fn local_concurrent_refusal(
    phases: &[SnapshotPhase],
    edges: &[SnapshotEdge],
) -> std::result::Result<(), ResolveError> {
    let mut local: Vec<usize> = (0..phases.len())
        .filter(|&node| !phases[node].palette && phases[node].isolation == Isolation::Local)
        .collect();
    if local.len() < 2 {
        return Ok(());
    }
    local.sort_by_key(|&node| (phases[node].position, node));
    let nodes: Vec<WalkNode<'_>> = phases
        .iter()
        .map(|phase| WalkNode {
            name: &phase.name,
            palette: phase.palette,
        })
        .collect();
    let index = |key: &str| phases.iter().position(|phase| phase.name == key);
    let walk: Vec<WalkEdge> = edges
        .iter()
        .filter_map(|edge| {
            Some(WalkEdge {
                from: index(&edge.from)?,
                to: index(&edge.to)?,
                kind: edge.kind,
            })
        })
        .collect();
    let mut forward = vec![Vec::new(); phases.len()];
    for (edge, _) in walk
        .iter()
        .zip(forward_mask(&nodes, &walk))
        .filter(|(_, forward)| *forward)
    {
        forward[edge.from].push(edge.to);
    }
    // Every node each `local` node reaches over forward edges, itself included.
    let reached: Vec<Vec<bool>> = local
        .iter()
        .map(|&from| {
            let mut seen = vec![false; phases.len()];
            let mut queue = vec![from];
            seen[from] = true;
            while let Some(at) = queue.pop() {
                for &next in &forward[at] {
                    if !seen[next] {
                        seen[next] = true;
                        queue.push(next);
                    }
                }
            }
            seen
        })
        .collect();
    for (i, &first) in local.iter().enumerate() {
        for (j, &second) in local.iter().enumerate().skip(i + 1) {
            if !reached[i][second] && !reached[j][first] {
                return Err(ResolveError::LocalConcurrent {
                    first: phases[first].name.clone(),
                    second: phases[second].name.clone(),
                });
            }
        }
    }
    Ok(())
}

/// `"sha256:"` + hex over `serde_json::to_string` of the typed, borrowed `{ phases, edges }`
/// (MOD-138 D3): the digest a v2 snapshot's `topology` carries. The edges are read in the one
/// snapshot order ([`htui_core::model::edge_sort_key`]) whatever order they are handed in, so the
/// order cannot make two equal graphs differ (F13).
///
/// The rules of [`topology`] hold: never through `serde_json::Value` (finding 4; a predicate is
/// key-sorted by its own serialiser, F5), and the envelope (`v`, `graph`, `source`, `mode`,
/// `settings`, `scope`, `personas`, `memory`) is outside it.
///
/// # Errors
/// [`ResolveError::Store`] carrying the serialiser's own sentence, as [`topology`].
pub fn topology_v2(
    phases: &[SnapshotPhase],
    edges: &[SnapshotEdge],
) -> std::result::Result<String, ResolveError> {
    /// The digested shape; field order is the JSON's.
    #[derive(serde::Serialize)]
    struct TopologyV2<'a> {
        phases: &'a [SnapshotPhase],
        edges: Vec<&'a SnapshotEdge>,
    }
    let mut sorted: Vec<&SnapshotEdge> = edges.iter().collect();
    sorted.sort_by(|a, b| edge_sort_key(a, phases).cmp(&edge_sort_key(b, phases)));
    let json = serde_json::to_string(&TopologyV2 {
        phases,
        edges: sorted,
    })
    .map_err(|err| {
        ResolveError::Store(StoreError::Constraint(format!(
            "the graph's nodes and edges do not serialise: {err}"
        )))
    })?;
    Ok(format!(
        "sha256:{}",
        htui_core::prompt::digest::sha256_hex(&json)
    ))
}

/// MOD-138 D11: each `divide` node `N` at position `p` becomes `N:plan` (role `planner`, position
/// `p`, `N`'s `plan_scope`) and `N` (role `integrate`, `p + 1`, `plan_scope: None`), both
/// carrying `N`'s `divide` block; every later position shifts by one; edges into `N` move to
/// `N:plan`, edges out of `N` stay on `N`, and a synthetic `N:plan → N` `next` joins them. The
/// edges come back in snapshot order. A graph with no `divide` node comes back as it went in.
///
/// `graph` names the graph in the refusal.
///
/// # Errors
/// [`ResolveError::InvalidGraph`] when `N:plan` is already a node's key.
pub fn expand_divide(
    graph: &str,
    phases: Vec<SnapshotPhase>,
    edges: Vec<SnapshotEdge>,
) -> std::result::Result<(Vec<SnapshotPhase>, Vec<SnapshotEdge>), ResolveError> {
    if phases.iter().all(|phase| phase.divide.is_none()) {
        return Ok((phases, edges));
    }
    let mut divided = Vec::new();
    for phase in phases.iter().filter(|phase| phase.divide.is_some()) {
        let planner = format!("{}{PLAN_SUFFIX}", phase.name);
        if phases.iter().any(|other| other.name == planner) {
            return Err(ResolveError::InvalidGraph {
                graph: graph.to_owned(),
                reason: format!(
                    "phase name `{planner}` is taken by the expansion of divide phase `{}`",
                    phase.name
                ),
            });
        }
        divided.push((phase.name.clone(), planner));
    }

    let mut out = Vec::with_capacity(phases.len() + divided.len());
    for phase in phases {
        let at = i32::try_from(out.len()).map_err(|_| {
            StoreError::Constraint(format!("graph `{graph}` has more nodes than i32"))
        })?;
        if phase.divide.is_some() {
            out.push(SnapshotPhase {
                position: at,
                name: format!("{}{PLAN_SUFFIX}", phase.name),
                role: NodeRole::Planner,
                ..phase.clone()
            });
            out.push(SnapshotPhase {
                position: at.saturating_add(1),
                role: NodeRole::Integrate,
                plan_scope: None,
                ..phase
            });
        } else {
            out.push(SnapshotPhase {
                position: at,
                ..phase
            });
        }
    }

    let mut edges: Vec<SnapshotEdge> = edges
        .into_iter()
        .map(|mut edge| {
            if let Some((_, planner)) = divided.iter().find(|(name, _)| *name == edge.to) {
                edge.to.clone_from(planner);
            }
            edge
        })
        .collect();
    edges.extend(divided.into_iter().map(|(name, planner)| SnapshotEdge {
        from: planner,
        to: name,
        kind: EdgeKind::Next,
        predicate: None,
        loop_limit: None,
    }));
    sort_snapshot_edges(&mut edges, &out);
    Ok((out, edges))
}

/// MOD-138 D9 over a snapshot, by node key: [`walk_refusal`] with `palette` from the node. An
/// edge naming a key no node holds is skipped, as [`graph_body_walk_refusal`] skips one.
fn snapshot_walk_refusal(
    graph: StepGraphId,
    phases: &[SnapshotPhase],
    edges: &[SnapshotEdge],
) -> Option<String> {
    let nodes: Vec<WalkNode<'_>> = phases
        .iter()
        .map(|phase| WalkNode {
            name: &phase.name,
            palette: phase.palette,
        })
        .collect();
    let index = |key: &str| phases.iter().position(|phase| phase.name == key);
    let edges: Vec<WalkEdge> = edges
        .iter()
        .filter_map(|edge| {
            Some(WalkEdge {
                from: index(&edge.from)?,
                to: index(&edge.to)?,
                kind: edge.kind,
            })
        })
        .collect();
    walk_refusal(graph, &nodes, &edges)
}

/// MOD-26 D9: the name of the persona `row` binds, frozen once into `personas`. A `persona_id`
/// with no row is refused (unreachable under the FK and MemStore's write check) rather than
/// frozen un-narrowed.
fn frozen_persona(
    row: &ResolvedPhase,
    personas: &mut Vec<SnapshotPersona>,
) -> std::result::Result<Option<String>, ResolveError> {
    let Some(id) = row.phase.persona_id else {
        return Ok(None);
    };
    let Some(persona) = &row.persona else {
        return Err(ResolveError::Store(StoreError::Constraint(
            references_no_row("step_graph_phase.persona_id", id, "persona"),
        )));
    };
    if !personas.iter().any(|frozen| frozen.name == persona.name) {
        personas.push(SnapshotPersona::freeze(persona).map_err(|err| {
            ResolveError::Store(StoreError::Constraint(format!(
                "persona `{}` does not serialise: {err}",
                persona.name
            )))
        })?);
    }
    Ok(Some(persona.name.clone()))
}

/// `<item.key>-override`: a clone deep over `step_graph_phase` **and** over the source phases'
/// own attachments (ANA-22 §2, MOD-9 D80), written with `is_override` set.
///
/// Every phase-level `skill_binding` of a source phase is copied onto that phase's clone through
/// `set_skill_binding`, so the override's steps bind what the original's would. Project and
/// global attachments are not copied: they apply to the clone already, and a copy would be a
/// second row at the same level.
///
/// A source attachment that writer would refuse (D78's chain: a qualified glob naming a repo the
/// project no longer has (D79), a negative position, a pin naming no version, ...) is caught
/// **before** anything is written (D96): every copy is run through [`check_attachment`] first, so
/// such a row refuses the clone whole and leaves no orphan graph; the user fixes the attachment
/// and clones again.
///
/// Each source phase's `phase_agent` and `phase_part` rows are re-keyed to the cloned phase
/// (MOD-37 R-6), and the source graph's edges are copied, remapped onto the cloned phases with
/// fresh edge ids (MOD-137 D13), so the clone is deep over `step_graph_phase`, `phase_agent`,
/// `phase_part`, `step_graph_edge` and the phase attachments alike. The source may be a graph of
/// any scope the item may use (MOD-138 D12); the clone is always created in the item's project
/// (an override stays project-only), and a workspace or library phase carries no phase
/// attachment to copy. All of it but the attachments
/// is written by one [`WriteStore::replace_graph`] (revision 0 → 1 on a first override), whose
/// pure refusals ([`graph_body_shape_refusal`], [`graph_body_topology_refusal`],
/// [`graph_body_walk_refusal`]) are decided before the graph row is created.
///
/// What a failure after that leaves: the `<key>-override` graph row, which no item names yet —
/// empty when `replace_graph` refused it (a stale revision, a persona or agent row gone),
/// whole when an attachment copy failed or the item moved before its repoint (with the
/// attachments copied so far). Calling again adopts that orphan: a graph of the item's project
/// holding the name, marked `is_override` and named by no item, is the clone's target —
/// `replace_graph` at its revision, fresh phase ids, then the repoint — rather than a second
/// `create_step_graph` the `(scope, name)` unique would refuse for ever. A graph holding the name
/// that is no override, or that an item runs under, refuses with a `Constraint` naming it.
///
/// A second call on an item whose `step_graph_id` already names its override (`is_override`
/// **and** named `<key>-override`; an override by another name refuses) re-clones **from
/// the kind's default graph** (the override is never a source) into the same graph through one
/// `replace_graph`, with fresh phase and edge ids, and leaves `item.step_graph_id` alone (ANA-2
/// `:317-320`, MOD-137 D13). The previous clone's phases go with their candidates, parts, edges
/// and attachments; no `update_item` runs, so `item.version` does not move.
///
/// # Errors
/// [`ResolveError::NoGraph`] when the item resolves to no graph; [`ResolveError::Store`] for the
/// store's own refusals, including a `Constraint` when the item's `version` has moved under the
/// caller, and, before any write, a `Constraint` carrying [`check_attachment`]'s sentence when a
/// source phase attachment would be refused by `set_skill_binding` (for a stale `<repo>:` glob,
/// [`glob_names_unknown_repo`](htui_core::store::glob_names_unknown_repo)'s), or a `NotFound` for
/// an attachment whose skill is gone. Before any write too, a `Constraint` carrying
/// `replace_graph`'s own sentence for a body it would refuse on its face, one for an item running
/// under an override that is not its own, one for a `<key>-override` name already taken (see
/// above), and one when the source graph's edges name a phase its phase rows did not answer (it
/// moved while being read: override again). After the graph row exists, a `Constraint` when the
/// override graph moved between the read and the write (`replace_graph`'s `Stale`) or the item
/// moved before its repoint; both say "override again", which adopts or re-clones the graph.
pub async fn override_graph<S: WriteStore, G: GraphSource>(
    store: &S,
    source: &G,
    item: &Item,
) -> std::result::Result<StepGraph, ResolveError> {
    let resolved = source
        .resolve_graph(item.id)
        .await?
        .ok_or(ResolveError::NoGraph(item.id))?;
    let name = format!("{}-override", item.key);
    // MOD-137 D13: the item already runs under its own override, so this call re-clones it. An
    // override by another name is not the item's own, and re-cloning it would rename and
    // overwrite whatever it is (review C-L1).
    let reclone = resolved.graph.is_override && item.step_graph_id == Some(resolved.graph.id);
    if reclone && resolved.graph.name != name {
        return Err(ResolveError::Store(StoreError::Constraint(format!(
            "item {} runs under override graph `{}`, not its own `{name}`",
            item.key, resolved.graph.name
        ))));
    }

    // The clone's source: the kind's default graph on a re-override (the override is never a
    // source), else the graph the item runs under. Either is read whole through one
    // `graph_body` — rows, candidates, parts and edges together; `resolved` only says which graph
    // (review C-L3).
    let source_graph = if reclone {
        store
            .item_kinds(item.project_id)
            .await?
            .into_iter()
            .find(|kind| kind.id == item.kind_id)
            .ok_or(ResolveError::NoGraph(item.id))?
            .default_graph_id
    } else {
        resolved.graph.id
    };
    let GraphBody {
        graph: source_row,
        nodes: rows,
        edges: source_edges,
    } = store
        .graph_body(source_graph)
        .await?
        .ok_or(ResolveError::NoGraph(item.id))?;
    let source_name = source_row.name;

    // D96 (F-H): every source phase attachment must pass `set_skill_binding`'s whole rule chain
    // (D78's `check_attachment`) before the clone writes anything, so a row the writer would refuse
    // (a stale `<repo>:` glob, a negative position, a pin naming no version) refuses the clone
    // whole and leaves no orphan graph.
    let sources: Vec<PhaseId> = rows.iter().map(|node| node.phase.id).collect();
    let copies: Vec<SkillBinding> = store
        .skill_bindings(Some(item.project_id))
        .await?
        .into_iter()
        .filter(|binding| {
            binding
                .phase_id
                .is_some_and(|phase| sources.contains(&phase))
        })
        .collect();
    if !copies.is_empty() {
        let repos: Vec<String> = store
            .repos(item.project_id)
            .await?
            .into_iter()
            .map(|repo| repo.name)
            .collect();
        let project_slug = store
            .project(item.project_id)
            .await?
            .map_or_else(|| item.project_id.to_string(), |project| project.slug);
        let skills = store.skills().await?;
        for binding in &copies {
            let skill_name = skills
                .iter()
                .find(|skill| skill.id == binding.skill_id)
                .map(|skill| skill.name.as_str())
                .ok_or_else(|| StoreError::NotFound {
                    entity: "skill",
                    id: binding.skill_id.to_string(),
                })?;
            let versions: Vec<i32> = store
                .skill_versions(binding.skill_id)
                .await?
                .into_iter()
                .map(|version| version.version)
                .collect();
            // The copy's key names the cloned phase, which is minted only once the clone is
            // written. The key here names the source phase instead, and `phase_project` is the
            // project the writer will find for the cloned phase: the clone is a graph of
            // `item.project_id`. The phase id only feeds rule 2's sentence, which that fact keeps
            // from firing, exactly as it cannot fire for the copy.
            check_attachment(
                &BindingFacts {
                    key: SkillBindingKey {
                        skill: binding.skill_id,
                        project: Some(item.project_id),
                        phase: binding.phase_id,
                    },
                    phase_project: Some(item.project_id),
                    skill_name,
                    versions: &versions,
                    repos: &repos,
                    project_slug: &project_slug,
                },
                &Attachment::of(binding),
            )
            .map_err(|sentence| ResolveError::Store(StoreError::Constraint(sentence)))?;
        }
    }

    // The graph the clone is written into: the item's own override on a re-clone, else an
    // orphaned `<key>-override` an earlier call left (adopted, review C-M1), else a new graph.
    // The body is built before anything is written, so the target's id is known up front.
    let existing = if reclone {
        Some(resolved.graph.clone())
    } else {
        orphaned_override(store, item, &name).await?
    };
    let target_id = existing
        .as_ref()
        .map_or_else(StepGraphId::new, |graph| graph.id);
    // Old phase id → cloned phase id, in the source's order (D80).
    let cloned: Vec<(PhaseId, PhaseId)> =
        sources.iter().map(|&old| (old, PhaseId::new())).collect();
    let remap = |old: PhaseId| {
        cloned
            .iter()
            .find(|(from, _)| *from == old)
            .map(|&(_, id)| id)
    };
    let nodes: Vec<NodeBody> = rows
        .into_iter()
        .zip(&cloned)
        .map(|(node, &(_, id))| NodeBody {
            phase: StepGraphPhase {
                id,
                graph_id: target_id,
                ..node.phase
            },
            // The source phase's candidates and parts, re-keyed onto the clone (MOD-37 R-6).
            agents: node
                .agents
                .into_iter()
                .map(|agent| PhaseAgent {
                    phase_id: id,
                    ..agent
                })
                .collect(),
            parts: node
                .parts
                .into_iter()
                .map(|part| PhasePart {
                    phase_id: id,
                    ..part
                })
                .collect(),
        })
        .collect();
    // The source's edges, remapped onto the cloned phases with fresh ids (MOD-137 D13).
    let edges = source_edges
        .into_iter()
        .map(|edge| match (remap(edge.from), remap(edge.to)) {
            (Some(from), Some(to)) => Ok(StepGraphEdge {
                id: EdgeId::new(),
                graph_id: target_id,
                from,
                to,
                ..edge
            }),
            _ => Err(ResolveError::Store(StoreError::Constraint(format!(
                "graph `{source_name}` moved while it was being cloned; override again"
            )))),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let description = format!("Per-item override of `{source_name}` for {}", item.key);
    let mut body = GraphBody {
        graph: existing.clone().unwrap_or_else(|| {
            // Stands in for the row `create_step_graph` answers until it does; the pure rules
            // below read only its id.
            let now = chrono::Utc::now();
            StepGraph {
                id: target_id,
                scope: GraphScope::Project(item.project_id),
                name: name.clone(),
                description: description.clone(),
                is_override: true,
                revision: 0,
                created_at: now,
                updated_at: now,
            }
        }),
        nodes,
        edges,
    };
    body.graph.name.clone_from(&name);
    body.graph.description.clone_from(&description);
    // `replace_graph`'s pure refusals (shape, then topology, then MOD-138 D9's walk), decided on
    // the body before the graph row is created, as D96 decides the attachments: a body the store
    // would refuse leaves no orphan behind (review C-M1).
    if let Some(sentence) = graph_body_shape_refusal(target_id, &body)
        .or_else(|| graph_body_topology_refusal(target_id, &body))
        .or_else(|| graph_body_walk_refusal(target_id, &body))
    {
        return Err(ResolveError::Store(StoreError::Constraint(sentence)));
    }
    let target = match existing {
        Some(graph) => graph,
        None => {
            let created = store
                .create_step_graph(NewStepGraph {
                    id: target_id,
                    scope: GraphScope::Project(item.project_id),
                    name: name.clone(),
                    description,
                    is_override: true,
                })
                .await?;
            body.graph = StepGraph {
                name: name.clone(),
                description: body.graph.description,
                ..created.clone()
            };
            created
        }
    };
    let clone = match store
        .replace_graph(target.id, target.revision, body)
        .await?
    {
        CasOutcome::Applied(applied) => applied.graph,
        CasOutcome::Stale(_) => {
            // The graph stays unpointed or the item's own, so a second call re-reads it: it
            // adopts what this one leaves or re-clones the item's own (review C-M1).
            let verb = if reclone { "re-cloned" } else { "cloned" };
            return Err(ResolveError::Store(StoreError::Constraint(format!(
                "override graph `{name}` moved while it was being {verb}; override again"
            ))));
        }
    };

    // Every phase-level attachment of a source phase, onto its clone (D80). Project and global
    // rows apply to the clone already; a copy of them would be a second row at the same level.
    for binding in &copies {
        let Some(source) = binding.phase_id else {
            continue;
        };
        let Some(&(_, phase)) = cloned.iter().find(|(old, _)| *old == source) else {
            continue;
        };
        let key = SkillBindingKey {
            skill: binding.skill_id,
            project: Some(item.project_id),
            phase: Some(phase),
        };
        let copy = BindingChange::Attach(Attachment::of(binding));
        if let CasOutcome::Stale(_) = store.set_skill_binding(key, None, copy).await? {
            // A fresh phase id has no row; a `Stale` here is a store bug, said rather than hidden.
            return Err(ResolveError::Store(StoreError::Constraint(format!(
                "a skill attachment already sits on the cloned phase {phase}"
            ))));
        }
    }

    // A re-clone wrote into the graph the item already names: nothing to repoint, and the item's
    // version stays where it is (MOD-137 D13).
    if reclone {
        return Ok(clone);
    }

    // The item is repointed last, so a failure part-way through leaves an orphan graph rather than
    // an item naming a half-cloned one; the next call adopts that orphan (review C-M1).
    let outcome = store
        .update_item(
            item.id,
            item.version,
            ItemPatch {
                step_graph_id: Some(Some(clone.id)),
                author_id: item.created_by,
                reason: "graph_override".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await?;
    match outcome {
        UpdateOutcome::Updated(_) => Ok(clone),
        UpdateOutcome::Diverged { head, .. } => {
            Err(ResolveError::Store(StoreError::Constraint(format!(
                "item {} moved to version {} while an override graph was being cloned; override \
                 again",
                item.key, head.version
            ))))
        }
    }
}

/// The `<item.key>-override` graph an earlier [`override_graph`] left behind (review C-M1), or
/// `None` while the name is free.
///
/// The graph row is created before the clone's `replace_graph`, its attachment copies and the
/// item's repoint, so a failure in any of those leaves a graph holding the name that no item
/// names; the retry writes into it rather than tripping over the `(scope, name)` unique. A graph
/// holding the name that is no override, or that an item runs under, is not the clone's to write
/// over: a `Constraint` naming it.
///
/// Who runs under it is read item by item, since no seam method answers "the items naming this
/// graph". The scan only runs when the name is already taken — a retry, never the common path.
async fn orphaned_override<S: WriteStore>(
    store: &S,
    item: &Item,
    name: &str,
) -> std::result::Result<Option<StepGraph>, ResolveError> {
    let Some(graph) = store
        .step_graphs(item.project_id)
        .await?
        .into_iter()
        .find(|graph| graph.name == name)
    else {
        return Ok(None);
    };
    if !graph.is_override {
        return Err(ResolveError::Store(StoreError::Constraint(format!(
            "graph `{name}` already exists and is not an override; rename it and override again"
        ))));
    }
    // Only an item of the graph's own project can name it: an override is always a project graph
    // and `check_graph` never offers one (MOD-137 D4; MOD-138 D12 kept overrides so), and
    // `items` narrows by `project_ids` alone; no store reads the workspace, so the nil id stands
    // in for the one the project may not even have.
    let scope = Scope {
        workspace_id: WorkspaceId::default(),
        project_ids: vec![item.project_id],
    };
    for summary in store.items(&scope, &ItemFilter::default()).await? {
        let Some(other) = store.item(summary.id).await? else {
            continue;
        };
        if other.step_graph_id == Some(graph.id) && other.id == item.id {
            // The caller's copy of the item is older than the store's, which already runs under
            // this override: a call with the fresh item re-clones it.
            return Err(ResolveError::Store(StoreError::Constraint(format!(
                "item {} already runs under `{name}`; override again",
                item.key
            ))));
        }
        if other.step_graph_id == Some(graph.id) {
            return Err(ResolveError::Store(StoreError::Constraint(format!(
                "graph `{name}` already exists and item {} runs under it; rename it and override \
                 again",
                other.key
            ))));
        }
    }
    Ok(Some(graph))
}

/// Plan D63's two caps, in order: any phase above `max_fan_out` first, then the planned agent
/// count against `max_agents_per_run`. Both are refused before a run row exists (ANA-2
/// `:868-876`, "refused at admission … never silently truncated").
///
/// The planned count is `Σ fan_out` plus one for each *judged* phase, `fan_out > 1` with a judge
/// (OQ-1). A fanned-out phase with no judge is selected by a human and costs no agent. Retry
/// attempts are not counted, which is the recorded deviation from `:871-872`. A divided node
/// (MOD-139 M2 D8) counts its planner node as its planner agent (0 with none) plus its default
/// parts, and its integrate node as 1 ([`crate::divide::planned_sessions`]).
fn check_fan_out_caps(
    phases: &[SnapshotPhase],
    settings: &SnapshotSettings,
) -> std::result::Result<(), ResolveError> {
    let max = settings.max_fan_out;
    if let Some(phase) = phases
        .iter()
        .find(|phase| u32::try_from(phase.fan_out).is_ok_and(|fan_out| fan_out > max))
    {
        return Err(ResolveError::FanOutCap {
            phase: phase.name.clone(),
            fan_out: phase.fan_out,
            max,
        });
    }

    let planned = crate::divide::planned_sessions(phases, None);
    let max = settings.max_agents_per_run;
    if planned > max {
        return Err(ResolveError::AgentCap { planned, max });
    }
    Ok(())
}

/// `project.settings` as ANA-2 §4.7 reads it; a blob that does not decode is read as defaults
/// rather than as a failure, which is [`ProjectSettings`]'s own rule (every field defaults, so
/// `'{}'` decodes).
///
/// A decode failure is warned about, because it silently defaults every non-cap key; the caps are
/// read separately and strictly (`ProjectCaps::from_settings`), so they are not among them. The
/// warning is said once per `(project, updated_at)` and at `debug` after (CLEAN-11), so a resolve
/// per run does not repeat it but an edit that is still malformed is heard.
fn project_settings(project: &Project) -> ProjectSettings {
    match serde_json::from_value(project.settings.clone()) {
        Ok(settings) => settings,
        Err(err) => {
            let empty = match &project.settings {
                // `resolve` refuses a `null` before it gets here (`ProjectCaps::from_settings`);
                // kept so this function stays quiet on its own, without that read.
                Value::Null => true,
                Value::Object(map) => map.is_empty(),
                _ => false,
            };
            if !empty {
                static WARNED: OnceLock<Mutex<HashMap<ProjectId, i64>>> = OnceLock::new();
                let edit = project.updated_at.timestamp_micros();
                if first_decode_failure(WARNED.get_or_init(Mutex::default), project.id, edit) {
                    tracing::warn!(
                        project = %project.id,
                        %err,
                        "project.settings does not fully decode; its non-cap keys read as defaults"
                    );
                } else {
                    tracing::debug!(
                        project = %project.id,
                        %err,
                        "project.settings still does not fully decode"
                    );
                }
            }
            ProjectSettings::default()
        }
    }
}

/// True when `project`'s document at `edit` (`updated_at` in micros) is not the one last noted.
/// Only the last edit of each project is remembered, so the map holds one entry per project, as
/// `first_of` holds one batch. The map is a parameter so a test can use its own.
fn first_decode_failure(
    seen: &Mutex<HashMap<ProjectId, i64>>,
    project: ProjectId,
    edit: i64,
) -> bool {
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(project, edit)
        != Some(edit)
}

/// One `app_setting` value as a positive `i64`.
///
/// "Positive or it did not answer" is `htui_core::prompt::settings`'s rule for the same table
/// (`crates/htui-core/src/prompt/settings.rs:621-625`): absent, `null`, a string, a bool, zero and
/// negative all mean the rung is silent, so a planted `0` falls through to the next rung rather
/// than pinning a budget of nothing.
fn app_positive(app: &BTreeMap<String, Value>, key: &str) -> Option<i64> {
    app.get(key).and_then(Value::as_i64).filter(|n| *n > 0)
}

/// [`app_positive`] narrowed to `u32`; a value too large for the column is read as silence.
fn app_u32(app: &BTreeMap<String, Value>, key: &str) -> Option<u32> {
    app_positive(app, key).and_then(|n| u32::try_from(n).ok())
}

/// [`app_positive`] narrowed to `i32`.
fn app_i32(app: &BTreeMap<String, Value>, key: &str) -> Option<i32> {
    app_positive(app, key).and_then(|n| i32::try_from(n).ok())
}

/// MOD-141 M2 D4: [`RunBounds`] over `app`, each key under [`app_u32`]'s "positive or silent"
/// rule; `runtime_changes` a known word or `ask`.
#[must_use]
pub fn run_bounds(app: &BTreeMap<String, Value>) -> RunBounds {
    RunBounds {
        max_fan_out: app_u32(app, "max_fan_out").unwrap_or(DEFAULT_MAX_FAN_OUT),
        max_agents_per_run: app_u32(app, "max_agents_per_run")
            .unwrap_or(DEFAULT_MAX_AGENTS_PER_RUN),
        max_parallel_sessions: app_u32(app, "max_parallel_sessions")
            .unwrap_or(SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS),
        max_plan_revisions: app_u32(app, "max_plan_revisions")
            .unwrap_or(SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS),
        max_child_runs: app_u32(app, "max_child_runs")
            .unwrap_or(SnapshotSettings::DEFAULT_MAX_CHILD_RUNS),
        runtime_changes: app
            .get("runtime_changes")
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .unwrap_or_default(),
    }
}

/// ANA-2 §4.10's one line (`R-ORCH-6`, `R-ORCH-2`): in auto mode a phase whose gate is not hard is
/// snapshotted `never`, so its steps land `done` with `gate_outcome = skipped`; a hard gate, and
/// every gate of a manual run, is kept. Applied **only** here, at snapshot time (MOD-12 D10):
/// `run.mode` is fixed at insert, so no later edit can skip a gate a run has not reached.
#[must_use]
pub(crate) const fn effective_gate(mode: RunMode, gate: Gate, gate_hard: bool) -> Gate {
    if matches!(mode, RunMode::Auto) && !gate_hard {
        Gate::Never
    } else {
        gate
    }
}

/// One phase of the live graph, with every §4.1 chain walked (`docs/ANA-2.md:281-288`).
#[expect(
    clippy::too_many_arguments,
    reason = "every one of them is a rung some chain reads; a struct for them would be named once \
              and read once"
)]
async fn snapshot_phase<G: GraphSource>(
    source: &G,
    row: &ResolvedPhase,
    position: i32,
    project: ProjectId,
    settings: &ProjectSettings,
    app: &BTreeMap<String, Value>,
    box_id: BoxId,
    mode: RunMode,
) -> std::result::Result<SnapshotPhase, ResolveError> {
    let (phase, rung_one) = (&row.phase, row.agents.as_slice());
    let isolation = phase.isolation.unwrap_or(settings.default_isolation);
    if isolation == Isolation::Local && phase.fan_out > 1 {
        return Err(ResolveError::LocalFanOut {
            phase: phase.name.clone(),
            fan_out: phase.fan_out,
        });
    }
    if phase.name == "review" && phase.fan_out > 1 {
        return Err(ResolveError::ReviewFanOut {
            phase: phase.name.clone(),
            fan_out: phase.fan_out,
        });
    }

    let version = match phase.template_version {
        Some(pinned) => pinned,
        None => {
            source
                .prompt_template(project, &phase.template_name, None)
                .await?
                .ok_or_else(|| ResolveError::NoTemplate {
                    phase: phase.name.clone(),
                    name: phase.template_name.clone(),
                    project,
                })?
                .version
        }
    };

    Ok(SnapshotPhase {
        position,
        name: phase.name.clone(),
        fan_out: phase.fan_out,
        gate: phase.gate,
        gate_effective: effective_gate(mode, phase.gate, phase.gate_hard),
        gate_hard: phase.gate_hard,
        retry_limit: phase.retry_limit,
        input_kinds: phase.input_kinds.clone(),
        output_kind: phase.output_kind.clone(),
        isolation,
        command_queue: phase.command_queue,
        verify_command: phase.verify_command.clone(),
        // MOD-141 M2 D3: the phase rung first (a positive column), then the project, the app and
        // two hours. A NULL column resolves exactly as before, so no digest moves.
        deadline_seconds: Some(
            phase
                .deadline_seconds
                .filter(|seconds| *seconds > 0)
                .and_then(|seconds| u32::try_from(seconds).ok())
                .or(settings.step_deadline_seconds)
                .or_else(|| app_u32(app, "step_deadline_seconds"))
                .unwrap_or(DEFAULT_DEADLINE_SECONDS),
        ),
        template: SnapshotTemplate {
            name: phase.template_name.clone(),
            version,
        },
        token_budget: phase
            .token_budget
            .filter(|n| *n > 0)
            .or_else(|| settings.token_budget.filter(|n| *n > 0))
            .or_else(|| app_i32(app, "token_budget")),
        candidates: candidates(source, phase, rung_one, settings, box_id).await?,
        // MOD-141 M2 D3: the phase's judge with its own model, else the project's with none;
        // `None` means human selection (§4.5).
        judge: match phase
            .judge_agent_id
            .map(|id| (id, phase.judge_model.clone()))
            .or_else(|| settings.judge_agent_id.map(|id| (id, None)))
        {
            None => None,
            Some((id, model)) => Some(SnapshotJudge {
                agent_id: id,
                agent_name: agent_name(source, &phase.name, id).await?,
                model,
                template: judge_template(source, project).await?,
            }),
        },
        // MOD-26 D9 (B-18): `resolve` binds the persona after this call, where the snapshot's
        // persona list is built.
        persona: None,
        // MOD-138 D1: a live row is a `step`; `expand_divide` assigns `planner` and `integrate`.
        role: NodeRole::Step,
        palette: phase.palette,
        plan_scope: phase.plan_scope,
        divide: (phase.split == Split::Divide).then(|| divide_block(phase, &row.parts)),
    })
}

/// MOD-138 D1, D11: a `divide` row's frozen block, its seven columns as stored and its
/// `phase_part` rows by `position`.
fn divide_block(phase: &StepGraphPhase, parts: &[PhasePart]) -> SnapshotDivide {
    let mut parts: Vec<&PhasePart> = parts.iter().collect();
    parts.sort_by_key(|part| part.position);
    SnapshotDivide {
        planner: SnapshotSeat {
            agent_id: phase.planner_agent_id,
            model: phase.planner_model.clone(),
            persona_id: phase.planner_persona_id,
        },
        integrate: SnapshotSeat {
            agent_id: phase.integrate_agent_id,
            model: phase.integrate_model.clone(),
            persona_id: phase.integrate_persona_id,
        },
        max_parts: phase.max_parts,
        default_parts: parts
            .into_iter()
            .map(|part| SnapshotPart {
                position: part.position,
                key: part.key.clone(),
                title: part.title.clone(),
                brief: part.brief.clone(),
                owned_paths: part.owned_paths.clone(),
                acceptance: part.acceptance.clone(),
                agent_id: part.agent_id,
                model: part.model.clone(),
                persona_id: part.persona_id,
            })
            .collect(),
    }
}

/// ANA-2 §4.1's candidate chain (`docs/ANA-2.md:288`), all four rungs.
///
/// Rung 1 is `phase_agent` in `position` order; rung 2 is `project.settings.default_agent_id`;
/// rung 3 is "the single enabled agent on the box" — exactly one `agent` with `enabled` whose
/// `agent_box` row on `box_id` is `enabled` (plan D62) — read through
/// [`GraphSource::agent_boxes`], and asked only when rung 2 is silent, so a project default is never
/// overridden by the box. Rung 4 is the refusal. Rungs 2 and 3 model the candidate on the agent's
/// `default_model`, else its first `models` entry; an agent that names neither is not a candidate.
/// A phase that reaches rung 4 is a named refusal and never a silent empty walk.
async fn candidates<G: GraphSource>(
    source: &G,
    phase: &StepGraphPhase,
    rung_one: &[PhaseAgent],
    settings: &ProjectSettings,
    box_id: BoxId,
) -> std::result::Result<Vec<SnapshotCandidate>, ResolveError> {
    // `ResolvedGraph` already carries the phase's `phase_agent` rows on backends that hold the
    // table; asking the source again is how a backend that answers them separately (or a fake that
    // stands them in) gets its say.
    let mut rows = source.phase_agents(phase.id).await?;
    if rows.is_empty() {
        rows = rung_one.to_vec();
    }
    rows.sort_by_key(|row| row.position);

    if !rows.is_empty() {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(SnapshotCandidate {
                agent_id: row.agent_id,
                agent_name: agent_name(source, &phase.name, row.agent_id).await?,
                model: row.model,
            });
        }
        return Ok(out);
    }

    let Some(id) = settings.default_agent_id else {
        return rung_three(source, phase, box_id).await;
    };
    let agent = source
        .agent(id)
        .await?
        .ok_or_else(|| ResolveError::NoAgentRow {
            phase: phase.name.clone(),
            agent: id,
        })?;
    // `SnapshotCandidate.model` is not nullable: a candidate the engine cannot name a model for is
    // not a candidate, so the project default falls through to rung 4 rather than to an empty
    // string the driver would then be handed.
    let model = agent
        .default_model
        .clone()
        .or_else(|| agent.models.first().cloned())
        .ok_or_else(|| ResolveError::NoCandidate {
            phase: phase.name.clone(),
        })?;
    Ok(vec![SnapshotCandidate {
        agent_id: agent.id,
        agent_name: agent.name,
        model,
    }])
}

/// Rung 3 (plan D62): the one enabled agent whose `agent_box` row on `box_id` is enabled, else
/// rung 4's [`ResolveError::NoCandidate`].
///
/// "Single" is literal: two enabled agents on the box are ambiguous and nothing is guessed. A box
/// row naming an agent the registry does not hold, or one that is disabled, is not counted.
async fn rung_three<G: GraphSource>(
    source: &G,
    phase: &StepGraphPhase,
    box_id: BoxId,
) -> std::result::Result<Vec<SnapshotCandidate>, ResolveError> {
    let refused = || ResolveError::NoCandidate {
        phase: phase.name.clone(),
    };
    let mut enabled = Vec::new();
    for row in source.agent_boxes(box_id).await? {
        if !row.enabled {
            continue;
        }
        if let Some(agent) = source.agent(row.agent_id).await?
            && agent.enabled
        {
            enabled.push(agent);
        }
    }
    let [agent] = <[Agent; 1]>::try_from(enabled).map_err(|_| refused())?;
    let model = agent
        .default_model
        .clone()
        .or_else(|| agent.models.first().cloned())
        .ok_or_else(refused)?;
    Ok(vec![SnapshotCandidate {
        agent_id: agent.id,
        agent_name: agent.name,
        model,
    }])
}

/// The latest `judge` template of the project, pinned beside the judged phase at run creation
/// (plan D53; ANA-5's table, `:1862`). `None` when the project holds no `judge` template: that is
/// not a refusal, because `None` already means "the latest at judge time" for a snapshot written
/// before milestone 4.
async fn judge_template<G: GraphSource>(
    source: &G,
    project: ProjectId,
) -> std::result::Result<Option<SnapshotTemplate>, ResolveError> {
    Ok(source
        .prompt_template(project, "judge", None)
        .await?
        .map(|row| SnapshotTemplate {
            name: row.name,
            version: row.version,
        }))
}

/// `agent.name`, denormalised onto the snapshot so an offline reader needs no registry (§5.1).
async fn agent_name<G: GraphSource>(
    source: &G,
    phase: &str,
    id: AgentId,
) -> std::result::Result<String, ResolveError> {
    source
        .agent(id)
        .await?
        .map(|agent| agent.name)
        .ok_or_else(|| ResolveError::NoAgentRow {
            phase: phase.to_owned(),
            agent: id,
        })
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{DemoData, demo_data, ids};
    use htui_core::model::{
        Activation, EdgeId, EdgeKind, Gate, GraphBody, GraphScope, ItemFilter, ItemPatch, NewRepo,
        NewStepGraph, NodeBody, PersonaId, PhasePatch, PromptTemplateId, RepoId, RepoPatch,
        RepoScope, RunScope, Scope, SkillBinding, SkillBindingId, SnapshotPersona, Split,
        StepGraphEdge,
    };
    use htui_core::model::{
        EdgePredicate, NodeRole, PlanScope, RuntimeChanges, SnapshotDivide, SnapshotEdge,
        SnapshotSource, SourceHow, default_snapshot_edges,
    };
    use htui_core::store::traits::back_asks_you_beside_another_asks_you;
    use htui_core::store::{
        MemStore, ReadStore, StoreError, UpdateOutcome, forward_edges_form_a_cycle,
        glob_names_unknown_repo, graph_body_walk_refusal, negative_position, references_no_row,
        split_fan_out_mismatch,
    };
    use serde_json::json;

    use super::{
        Agent, AgentBox, AgentId, BTreeMap, BoundSkill, BoxId, CasOutcome, GraphSnapshot,
        GraphSource, Isolation, Item, ItemId, NotRunnable, PhaseAgent, PhaseId, PhasePart,
        ProjectId, PromptTemplate, ResolveError, Resolved, ResolvedGraph, Result, RunBounds,
        RunMode, SnapshotTemplate, StepGraph, StepGraphId, StepGraphPhase, Value, WriteStore,
        effective_gate, expand_divide, first_decode_failure, override_graph, project_settings,
        resolve, run_bounds, snapshot_walk_refusal, topology_v2,
    };

    /// CLEAN-11: a malformed document is first for its `(project, updated_at)` and not again; an
    /// edit that is still malformed is news. Over a local set, so no other test is in it.
    #[test]
    fn a_decode_failure_is_first_once_per_project_and_edit() {
        let seen = std::sync::Mutex::default();
        let project = ProjectId::new();
        assert!(first_decode_failure(&seen, project, 1), "a new document");
        assert!(!first_decode_failure(&seen, project, 1), "the same one");
        assert!(first_decode_failure(&seen, project, 2), "an edit");
        assert!(first_decode_failure(&seen, ProjectId::new(), 1), "another");
        assert_eq!(
            seen.lock().expect("unpoisoned").len(),
            2,
            "one entry per project"
        );
    }

    /// A `tracing` writer into a shared buffer, for a case that asserts a line was logged.
    #[derive(Clone, Default)]
    struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl LogBuffer {
        /// The captured lines that mention `needle`.
        fn lines_with(&self, needle: &str) -> Vec<String> {
            let bytes = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            String::from_utf8_lossy(&bytes)
                .lines()
                .filter(|line| line.contains(needle))
                .map(str::to_owned)
                .collect()
        }
    }

    /// CLEAN-11 (CLEAN-9 review): every resolve of a project whose settings do not decode used to
    /// warn. It warns once per `(project, updated_at)`, then at `debug`.
    #[test]
    fn a_malformed_project_settings_blob_warns_once_per_edit() {
        // Its own project id: the once-set is process-wide, and a shared id would make the
        // outcome depend on test order.
        let mut project = demo_data().projects.remove(0);
        project.id = ProjectId::new();
        project.settings = json!([1]);
        let logs = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let logs = logs.clone();
                move || logs.clone()
            })
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .finish();
        {
            let _guard = tracing::subscriber::set_default(subscriber);
            project_settings(&project);
            project_settings(&project);
            project.updated_at += chrono::Duration::seconds(1);
            project_settings(&project);
        }

        let warned = logs.lines_with("its non-cap keys read as defaults");
        assert_eq!(warned.len(), 2, "once per edit: {warned:?}");
        assert!(
            warned.iter().all(|line| line.contains(" WARN ")),
            "{warned:?}"
        );
        let again = logs.lines_with("still does not fully decode");
        assert_eq!(again.len(), 1, "the repeat: {again:?}");
        assert!(again[0].contains(" DEBUG "), "{again:?}");
    }

    /// The digest of the seeded `feature` graph, resolved against the demo fixture with one
    /// `claude`/`sonnet` candidate per phase. Computed once and pasted, which is the only way a
    /// pinned vector means anything.
    const FEATURE_TOPOLOGY: &str =
        "sha256:ce0b489ff772a6a1c84c7590dcf6ba60b68121716f6897decf628115bd7c959b";

    /// MOD-138 D3 (F14): the v2 digest of the same resolve, over its phases **and** its default
    /// edges, which `snapshot.topology` carries since `GraphSnapshot::V = 2`. Computed once and
    /// pasted, as [`FEATURE_TOPOLOGY`] is.
    const FEATURE_TOPOLOGY_V2: &str =
        "sha256:bdbf786844046bcf4101e4499f27230c2914ec8baa3611117af9cd2c1e076a5e";

    /// What a change to [`FEATURE_TOPOLOGY`] or [`FEATURE_TOPOLOGY_V2`] means, said where it will
    /// be read.
    const TOPOLOGY_MOVED: &str = "\
a `topology` digest of the seeded `feature` graph moved. The v1 one is sha256 over \
`serde_json::to_string` of the **typed** `&[SnapshotPhase]` in declaration order (plan D9); the \
v2 one (MOD-138 D3) is the same over the typed `{ phases, edges }`, edges in snapshot order. Either \
moves only when the phase struct gains or loses a field, when the seeded graph changes, or when a \
resolved value does — the template versions, the `7200` deadline default, the denormalised \
`agent_name`, the project's `token_budget`. Every one of those also changes the digest a *stored* \
snapshot would compute for the same graph, so re-pinning this literal is a `GraphSnapshot::V` \
question and not a test fix. Decide the version bump first, then paste the new digest.";

    /// A `GraphSource` over `&MemStore` with one scripted candidate per phase.
    ///
    /// T3's `FakeGraphSource` is the real one; this is the smallest thing that makes `graph.rs`
    /// testable on its own, and it exists for the reason plan D20 gives: the demo seeds no
    /// `phase_agent` row and no `agent_box` row, so rungs 1 and 3 of §4.1's chain are both empty
    /// here and a resolution with no stand-in would refuse every phase for a reason that is about
    /// the fixture rather than about the walk. Its `phase_agents` answers from `candidates`, not
    /// from the store; `resolve_graph` reads the store, so the rows `create_phase_agents` writes
    /// reach `override_graph` through it (MOD-37 R-6).
    struct TestSource<'a> {
        store: &'a MemStore,
        candidates: Vec<(AgentId, &'static str)>,
        /// MOD-26 B-18: a `persona_id` the first phase names with no `persona` row behind it, a
        /// shape neither store can write.
        unbacked: Option<PersonaId>,
        /// MOD-137 R1b: a graph this source re-writes (one no-op `replace_graph`, so its
        /// `revision` moves) right after it answers `resolve_graph` — an edit landing between the
        /// read and the clone's write.
        race: Option<StepGraphId>,
        /// MOD-138 D10: a phase (by name) this source leaves out of its `resolve_graph` answer
        /// while keeping the edges that name it — a read that raced a delete.
        dropped: Option<&'static str>,
    }

    impl<'a> TestSource<'a> {
        /// One `claude`/`sonnet` candidate per phase — the pair `RUN_1`'s own steps carry.
        fn claude(store: &'a MemStore) -> Self {
            Self {
                store,
                candidates: vec![(ids::AGENT_CLAUDE, "sonnet")],
                unbacked: None,
                race: None,
                dropped: None,
            }
        }

        /// No rung-1 candidate at all, so resolution falls through to the project rung.
        fn barren(store: &'a MemStore) -> Self {
            Self {
                store,
                candidates: Vec::new(),
                unbacked: None,
                race: None,
                dropped: None,
            }
        }
    }

    impl GraphSource for TestSource<'_> {
        async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
            let mut graph = self.store.resolve_graph(item).await?;
            if let (Some(name), Some(graph)) = (self.dropped, graph.as_mut()) {
                graph.phases.retain(|row| row.phase.name != name);
            }
            if let (Some(id), Some(graph)) = (self.unbacked, graph.as_mut()) {
                let first = graph
                    .phases
                    .iter_mut()
                    .min_by_key(|row| row.phase.position)
                    .expect("the graph has a phase");
                first.phase.persona_id = Some(id);
                first.persona = None;
            }
            if let Some(raced) = self.race {
                let body = self
                    .store
                    .graph_body(raced)
                    .await?
                    .expect("the raced graph is there");
                let outcome = self
                    .store
                    .replace_graph(raced, body.graph.revision, body)
                    .await?;
                assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
            }
            Ok(graph)
        }

        async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
            Ok(self
                .candidates
                .iter()
                .enumerate()
                .map(|(position, (agent_id, model))| PhaseAgent {
                    phase_id: phase,
                    position: i32::try_from(position).expect("a handful of candidates"),
                    agent_id: *agent_id,
                    model: (*model).to_owned(),
                })
                .collect())
        }

        async fn prompt_template(
            &self,
            project: ProjectId,
            name: &str,
            version: Option<i32>,
        ) -> Result<Option<PromptTemplate>> {
            self.store.prompt_template(project, name, version).await
        }

        async fn agent(&self, id: AgentId) -> Result<Option<Agent>> {
            Ok(self
                .store
                .agents()
                .await?
                .into_iter()
                .map(|row| row.agent)
                .find(|agent| agent.id == id))
        }

        async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
            self.store.agent_boxes(box_id).await
        }

        async fn bound_skills(
            &self,
            project: ProjectId,
            phase: Option<PhaseId>,
        ) -> Result<Vec<BoundSkill>> {
            self.store.bound_skills(project, phase).await
        }

        async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
            self.store.missing_tags(item, box_id).await
        }
    }

    /// The demo fixture with `htui`'s `project.settings` replaced.
    fn store_with_settings(settings: Value) -> MemStore {
        store_with(|data| {
            for project in &mut data.projects {
                if project.id == ids::PROJECT_HTUI {
                    project.settings = settings.clone();
                }
            }
        })
    }

    /// The demo fixture, edited.
    fn store_with(edit: impl FnOnce(&mut DemoData)) -> MemStore {
        let mut data = demo_data();
        edit(&mut data);
        MemStore::from_demo(data)
    }

    async fn feat_1(store: &MemStore) -> Item {
        store
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture holds HTUI_FEAT-1")
    }

    /// `resolve` with an empty `app_setting` map and no requested scope.
    async fn resolve_feat(
        store: &MemStore,
        source: &TestSource<'_>,
    ) -> std::result::Result<Resolved, ResolveError> {
        let item = feat_1(store).await;
        resolve(
            store,
            source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
    }

    /// The test this module was written against (plan D9, blueprint H-2/H-17).
    ///
    /// The vector is asserted in a crate-scoped run *and* in a workspace run, which is what makes
    /// the `preserve_order` divergence visible if it ever returns: `serde_json`'s `preserve_order`
    /// is feature-unified **on** under `cargo test --workspace` and **off** under
    /// `cargo test -p htui-orch`, so a digest taken through `serde_json::Value` would differ
    /// between the two invocations while both stayed green on their own.
    #[tokio::test]
    async fn feature_snapshot_topology_is_pinned() {
        let store = MemStore::demo();
        let resolved = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves");
        // MOD-138 F14: the v1 digest of the same phases is unchanged, because the v2 node fields
        // are skipped at their defaults; that is D1's cross-build byte-identity proof.
        assert_eq!(
            super::topology(&resolved.snapshot.phases).expect("`SnapshotPhase` serialises"),
            FEATURE_TOPOLOGY,
            "{TOPOLOGY_MOVED}"
        );
        assert_eq!(
            resolved.snapshot.topology, FEATURE_TOPOLOGY_V2,
            "{TOPOLOGY_MOVED}"
        );
        assert_eq!(
            resolved.snapshot.topology,
            topology_v2(&resolved.snapshot.phases, &resolved.snapshot.edges)
                .expect("the snapshot serialises"),
            "the snapshot carries the v2 digest of its own phases and edges"
        );
        assert_eq!(resolved.snapshot.v, 2, "{TOPOLOGY_MOVED}");
    }

    /// `UNIQUE (graph_id, position)` permits gaps; the snapshot builder is what closes them
    /// (`docs/ANA-2.md:269-272`).
    #[tokio::test]
    async fn positions_are_dense() {
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT {
                    // 0,1,2,3 becomes 0,10,20,30: same order, four gaps.
                    phase.position *= 10;
                }
            }
        });
        let resolved = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("a gapped graph still resolves");
        let positions: Vec<i32> = resolved
            .snapshot
            .phases
            .iter()
            .map(|phase| phase.position)
            .collect();
        let names: Vec<&str> = resolved
            .snapshot
            .phases
            .iter()
            .map(|phase| phase.name.as_str())
            .collect();
        assert_eq!(positions, [0, 1, 2, 3]);
        assert_eq!(names, ["prd", "plan", "implement", "review"]);
    }

    /// MOD-12 D10 (ANA-2 §4.10, criterion 23): in auto mode a gate that is not hard is `never`,
    /// whatever the phase says.
    #[test]
    fn auto_mode_downgrades_a_soft_gate_to_never() {
        for &gate in Gate::ALL {
            assert_eq!(
                effective_gate(RunMode::Auto, gate, false),
                Gate::Never,
                "an auto run skips a soft `{gate}`"
            );
        }
    }

    /// MOD-12 D10: a hard gate is kept in auto mode.
    #[test]
    fn auto_mode_keeps_a_hard_gate() {
        for &gate in Gate::ALL {
            assert_eq!(
                effective_gate(RunMode::Auto, gate, true),
                gate,
                "an auto run keeps a hard `{gate}`"
            );
        }
    }

    /// MOD-12 D10 (criterion 24): a manual run keeps every gate, hard or not.
    #[test]
    fn manual_mode_keeps_every_gate() {
        for &gate in Gate::ALL {
            for hard in [false, true] {
                assert_eq!(
                    effective_gate(RunMode::Manual, gate, hard),
                    gate,
                    "a manual run keeps `{gate}` (gate_hard = {hard})"
                );
            }
        }
    }

    /// MOD-12 D10: an auto snapshot of the seeded `feature` graph keeps its hard phases' gates and
    /// snapshots every other phase `never`; the mode is recorded on the snapshot.
    #[tokio::test]
    async fn an_auto_snapshot_downgrades_only_its_soft_phases() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let snapshot = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Auto,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("the seeded feature graph resolves")
        .snapshot;
        assert_eq!(snapshot.mode, RunMode::Auto);
        assert!(
            snapshot.phases.iter().any(|phase| phase.gate_hard),
            "the seeded `feature` graph has a hard phase"
        );
        assert!(
            snapshot.phases.iter().any(|phase| !phase.gate_hard),
            "the seeded `feature` graph has a soft phase"
        );
        for phase in &snapshot.phases {
            let expected = if phase.gate_hard {
                phase.gate
            } else {
                Gate::Never
            };
            assert_eq!(
                phase.gate_effective, expected,
                "`{}` (gate_hard = {}) under auto mode",
                phase.name, phase.gate_hard
            );
        }
    }

    /// One assertion per row of ANA-2 §4.1's chain table (`docs/ANA-2.md:281-288`), against the
    /// fixture's own values.
    #[tokio::test]
    async fn field_chains_walk_phase_project_app() {
        let store = MemStore::demo();
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        let prd = &snapshot.phases[0];
        let implement = &snapshot.phases[2];

        // Copied straight off the row.
        assert_eq!(prd.fan_out, 1);
        assert_eq!(prd.retry_limit, 1);
        assert_eq!(prd.output_kind, "prd");
        assert_eq!(prd.verify_command, None, "every seeded phase has none");
        assert_eq!(implement.input_kinds, ["plan", "review"]);
        assert!(
            prd.gate_hard,
            "`feature.prd` is one of the three seeded true"
        );
        assert!(!implement.gate_hard);

        // `gate_effective`: a manual snapshot never downgrades.
        assert_eq!(prd.gate, Gate::Always);
        assert_eq!(prd.gate_effective, prd.gate);

        // `isolation`: the phase says nothing, so the project rung answers, and the seed leaves
        // that at the built-in `worktree`.
        assert_eq!(prd.isolation, Isolation::Worktree);
        assert_eq!(snapshot.settings.default_isolation, Isolation::Worktree);

        // `template_version`: the phase pins none, so the latest in the project wins — which the
        // seed writes as version 1.
        assert_eq!(prd.template.name, "prd");
        assert_eq!(prd.template.version, 1);

        // `token_budget`: the phase carries none and the demo project's settings blob carries
        // 120_000, so the project rung answers.
        assert_eq!(prd.token_budget, Some(120_000));

        // `deadline_seconds`: the phase carries none, and neither does the project nor
        // `app_setting`, so the built-in two hours answers.
        assert_eq!(prd.deadline_seconds, Some(7200));

        // `candidates`: rung 1, with `agent_name` denormalised off the registry.
        assert_eq!(prd.candidates.len(), 1);
        assert_eq!(prd.candidates[0].agent_id, ids::AGENT_CLAUDE);
        assert_eq!(prd.candidates[0].agent_name, "claude");
        assert_eq!(prd.candidates[0].model, "sonnet");

        // `judge_agent_id`: no phase judge and no project judge, so none — human selection.
        assert_eq!(prd.judge, None);

        // The `app_setting` rungs of the envelope.
        assert_eq!(snapshot.settings.max_fan_out, 4);
        assert_eq!(snapshot.settings.max_agents_per_run, 8);
        assert_eq!(snapshot.mode, RunMode::Manual);
        assert_eq!(snapshot.graph.id, ids::GRAPH_HTUI_FEAT);
        assert!(!snapshot.graph.is_override);
    }

    /// The project rung of four chains at once, driven through `project.settings` — which no
    /// writer on the seam can reach, because `set_setting`'s `SettingKey` is a closed enum and
    /// `update_project` never touches the column (blueprint F-F).
    #[tokio::test]
    async fn the_project_rung_answers_when_the_phase_does_not() {
        let store = store_with_settings(json!({
            "default_isolation": "copy",
            "token_budget": 42,
            "step_deadline_seconds": 600,
            "judge_agent_id": ids::AGENT_AGY,
            "per_token_cap_run": 7,
            "per_token_cap_batch": 9,
        }));
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        let prd = &snapshot.phases[0];

        assert_eq!(prd.isolation, Isolation::Copy);
        assert_eq!(prd.token_budget, Some(42));
        assert_eq!(prd.deadline_seconds, Some(600));
        let judge = prd.judge.as_ref().expect("the project names a judge");
        assert_eq!(judge.agent_id, ids::AGENT_AGY);
        assert_eq!(judge.agent_name, "agy");
        assert_eq!(judge.model, None, "the project rung carries no model");
        assert_eq!(
            judge.template,
            Some(SnapshotTemplate {
                name: "judge".to_owned(),
                version: 1
            }),
            "the seeded `judge` template is pinned at resolution (plan D53)"
        );
        assert_eq!(snapshot.settings.default_isolation, Isolation::Copy);
        assert_eq!(snapshot.settings.per_token_cap_run, Some(7));
        assert_eq!(snapshot.settings.per_token_cap_batch, Some(9));
        assert_ne!(
            snapshot.topology, FEATURE_TOPOLOGY_V2,
            "a resolved value moving is a different topology, which is the point of the digest"
        );
    }

    /// MOD-141 M2 D3: a phase's own `deadline_seconds` answers before the project's; a phase
    /// that carries none still reads the project rung.
    #[tokio::test]
    async fn the_phase_deadline_answers_before_the_project() {
        let store = store_with_settings(json!({ "step_deadline_seconds": 600 }));
        replace_feature(&store, |body| {
            let prd = body
                .nodes
                .iter_mut()
                .find(|node| node.phase.name == "prd")
                .expect("the feature graph has a prd phase");
            prd.phase.deadline_seconds = Some(90);
        })
        .await;
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the edited feature graph resolves")
            .snapshot;

        assert_eq!(snapshot.phases[0].name, "prd");
        assert_eq!(
            snapshot.phases[0].deadline_seconds,
            Some(90),
            "the phase rung answers first"
        );
        for phase in &snapshot.phases[1..] {
            assert_eq!(
                phase.deadline_seconds,
                Some(600),
                "`{}` carries no deadline, so the project rung answers",
                phase.name
            );
        }
    }

    /// MOD-141 M2 D3: a phase's own judge answers with its own `judge_model`, before the
    /// project's judge; a phase that names none falls through to the project's, with no model.
    #[tokio::test]
    async fn a_phase_judge_carries_its_model_and_beats_the_project_judge() {
        let store = store_with_settings(json!({ "judge_agent_id": ids::AGENT_AGY }));
        replace_feature(&store, |body| {
            let implement = body
                .nodes
                .iter_mut()
                .find(|node| node.phase.name == "implement")
                .expect("the feature graph has an implement phase");
            implement.phase.judge_agent_id = Some(ids::AGENT_CLAUDE);
            implement.phase.judge_model = Some("opus".to_owned());
        })
        .await;
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the edited feature graph resolves")
            .snapshot;
        let judge_of = |name: &str| {
            snapshot
                .phases
                .iter()
                .find(|phase| phase.name == name)
                .unwrap_or_else(|| panic!("the snapshot has a `{name}` phase"))
                .judge
                .clone()
        };

        let implement = judge_of("implement").expect("implement names its own judge");
        assert_eq!(implement.agent_id, ids::AGENT_CLAUDE);
        assert_eq!(implement.agent_name, "claude");
        assert_eq!(
            implement.model.as_deref(),
            Some("opus"),
            "the phase's judge carries the phase's model"
        );
        assert_eq!(
            implement.template,
            Some(SnapshotTemplate {
                name: "judge".to_owned(),
                version: 1
            }),
            "the seeded `judge` template is pinned whichever rung names the judge"
        );

        let prd = judge_of("prd").expect("prd falls through to the project's judge");
        assert_eq!(prd.agent_id, ids::AGENT_AGY);
        assert_eq!(prd.agent_name, "agy");
        assert_eq!(prd.model, None, "the project rung carries no model");
    }

    /// MOD-141 M2 D3: the seeded phases carry no deadline and no judge, so the phase rung is
    /// silent and resolve answers exactly as before: the same digest, two hours, no judge.
    #[tokio::test]
    async fn null_phase_columns_resolve_as_before() {
        let store = MemStore::demo();
        let stored = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        assert!(
            stored.iter().all(|phase| phase.deadline_seconds.is_none()
                && phase.judge_agent_id.is_none()
                && phase.judge_model.is_none()),
            "every seeded phase leaves the three columns NULL"
        );

        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        assert_eq!(snapshot.v, GraphSnapshot::V, "{TOPOLOGY_MOVED}");
        assert_eq!(snapshot.topology, FEATURE_TOPOLOGY_V2, "{TOPOLOGY_MOVED}");
        for phase in &snapshot.phases {
            assert_eq!(
                phase.deadline_seconds,
                Some(7200),
                "`{}`: the built-in two hours",
                phase.name
            );
            assert_eq!(phase.judge, None, "`{}`: human selection", phase.name);
        }
    }

    /// An unrelated malformed key must not erase the caps: the lenient `ProjectSettings` decode
    /// defaults the whole struct, so the caps are read on their own (CLEAN-9 T2).
    #[tokio::test]
    async fn an_unrelated_bad_key_keeps_the_caps() {
        let store = store_with_settings(json!({
            "copy_exclude": 5,
            "per_token_cap_run": 700,
            "per_token_cap_batch": 900,
        }));
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("a bad non-cap key does not refuse the resolve")
            .snapshot;

        assert_eq!(snapshot.settings.per_token_cap_run, Some(700));
        assert_eq!(snapshot.settings.per_token_cap_batch, Some(900));
    }

    /// A malformed cap refuses the resolve, naming the project and the key.
    #[tokio::test]
    async fn a_bad_cap_key_refuses_the_resolve() {
        let store = store_with_settings(json!({ "per_token_cap_run": "x" }));
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("a cap that is not an integer is refused");

        assert!(
            matches!(
                &error,
                ResolveError::ProjectCaps { cause, .. } if cause.key == "per_token_cap_run"
            ),
            "{error}"
        );
    }

    /// The `app_setting` rung, and the "positive or the rung is silent" rule the prompt settings
    /// resolver already applies to the same table.
    #[tokio::test]
    async fn the_app_rung_answers_when_the_project_does_not() {
        let store = store_with_settings(json!({}));
        let item = feat_1(&store).await;
        let app: BTreeMap<String, Value> = [
            ("token_budget".to_owned(), json!(9_000)),
            ("step_deadline_seconds".to_owned(), json!(300)),
            ("max_fan_out".to_owned(), json!(2)),
            // Four, not less: `feature` plans one agent per phase, and D63 refuses a run that
            // plans more agents than this cap.
            ("max_agents_per_run".to_owned(), json!(4)),
        ]
        .into_iter()
        .collect();
        let snapshot = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Auto,
            &app,
            None,
            ids::BOX,
        )
        .await
        .expect("the seeded feature graph resolves")
        .snapshot;

        assert_eq!(snapshot.phases[0].token_budget, Some(9_000));
        assert_eq!(snapshot.phases[0].deadline_seconds, Some(300));
        assert_eq!(snapshot.settings.max_fan_out, 2);
        assert_eq!(snapshot.settings.max_agents_per_run, 4);
        assert_eq!(snapshot.mode, RunMode::Auto);

        let silent: BTreeMap<String, Value> = [
            ("token_budget".to_owned(), json!(0)),
            ("step_deadline_seconds".to_owned(), json!("600")),
            ("max_fan_out".to_owned(), json!(-1)),
        ]
        .into_iter()
        .collect();
        let snapshot = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &silent,
            None,
            ids::BOX,
        )
        .await
        .expect("the seeded feature graph resolves")
        .snapshot;
        assert_eq!(snapshot.phases[0].token_budget, None);
        assert_eq!(snapshot.phases[0].deadline_seconds, Some(7200));
        assert_eq!(snapshot.settings.max_fan_out, 4);
    }

    /// Rung 2 of the candidate chain: `project.settings.default_agent_id` with the agent's own
    /// `default_model`.
    #[tokio::test]
    async fn the_project_default_agent_is_the_second_candidate_rung() {
        let store = store_with_settings(json!({ "default_agent_id": ids::AGENT_AGY }));
        let snapshot = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect("the project default answers")
            .snapshot;
        let candidates = &snapshot.phases[0].candidates;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].agent_id, ids::AGENT_AGY);
        assert_eq!(candidates[0].agent_name, "agy");
        assert_eq!(candidates[0].model, "gemini-3.7-flash-high");
    }

    /// An `agent_box` row on the fixture box for `agent`, never probed and never latched.
    fn box_row(agent: AgentId, enabled: bool) -> AgentBox {
        AgentBox {
            agent_id: agent,
            box_id: ids::BOX,
            enabled,
            version: None,
            path: None,
            probed_at: None,
            quota: None,
            quota_at: None,
            updated_at: htui_agent::conformance::epoch(),
            probe: None,
        }
    }

    /// Plan D62: rung 3 is "exactly one `agent` with `enabled` whose `agent_box` on this box is
    /// `enabled`", modelled on the agent's `default_model`. A disabled box row is not a second
    /// enabled agent, so it does not spoil the rung.
    #[tokio::test]
    async fn rung_three_is_the_single_enabled_agent_on_this_box() {
        let store = MemStore::demo();
        store
            .upsert_agent_box(&box_row(ids::AGENT_AGY, true))
            .await
            .expect("the fixture box takes a row");
        store
            .upsert_agent_box(&box_row(ids::AGENT_CLAUDE, false))
            .await
            .expect("the fixture box takes a row");

        let snapshot = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect("rung 3 answers where rungs 1 and 2 are silent")
            .snapshot;
        let candidates = &snapshot.phases[0].candidates;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].agent_id, ids::AGENT_AGY);
        assert_eq!(candidates[0].agent_name, "agy");
        assert_eq!(candidates[0].model, "gemini-3.7-flash-high");
    }

    /// Two enabled agents on the box are not "the single" one: rung 3 guesses nothing and the chain
    /// falls through to rung 4's named refusal.
    #[tokio::test]
    async fn two_enabled_agents_are_not_a_rung_three() {
        let store = MemStore::demo();
        for agent in [ids::AGENT_AGY, ids::AGENT_CLAUDE] {
            store
                .upsert_agent_box(&box_row(agent, true))
                .await
                .expect("the fixture box takes a row");
        }
        let error = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect_err("two enabled agents on the box are ambiguous");
        assert_eq!(
            error,
            ResolveError::NoCandidate {
                phase: "prd".to_owned()
            }
        );

        // Rung 2 still wins over rung 3: a project default is asked for before the box is.
        let store = store_with_settings(json!({ "default_agent_id": ids::AGENT_CLAUDE }));
        store
            .upsert_agent_box(&box_row(ids::AGENT_AGY, true))
            .await
            .expect("the fixture box takes a row");
        let error = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect_err("rung 2 names `claude`, which has no model; rung 3 is not consulted");
        assert_eq!(
            error,
            ResolveError::NoCandidate {
                phase: "prd".to_owned()
            }
        );
    }

    /// Rung 4: refuse, by name. A phase that resolved to no candidate and said nothing would make
    /// the engine refuse every run for a reason the operator could not read.
    #[tokio::test]
    async fn no_candidate_is_a_named_refusal() {
        let store = MemStore::demo();
        let error = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect_err("the demo fixture answers no rung");
        assert_eq!(
            error,
            ResolveError::NoCandidate {
                phase: "prd".to_owned()
            }
        );

        // `claude`'s seed row carries neither a `default_model` nor any `models`, so the project
        // rung cannot name a model and falls through to the same refusal rather than handing the
        // driver an empty string.
        let store = store_with_settings(json!({ "default_agent_id": ids::AGENT_CLAUDE }));
        let error = resolve_feat(&store, &TestSource::barren(&store))
            .await
            .expect_err("a candidate with no model is not a candidate");
        assert_eq!(
            error,
            ResolveError::NoCandidate {
                phase: "prd".to_owned()
            }
        );
    }

    /// A phase naming a template the project does not hold cannot be resolved to a version, and a
    /// snapshot with an unresolvable template is a run that fails at stage 3.
    #[tokio::test]
    async fn a_missing_template_is_refused() {
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT && phase.name == "plan" {
                    phase.template_name = "nothing-seeds-this".to_owned();
                }
            }
        });
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("the template is not in the project");
        assert_eq!(
            error,
            ResolveError::NoTemplate {
                phase: "plan".to_owned(),
                name: "nothing-seeds-this".to_owned(),
                project: ids::PROJECT_HTUI,
            }
        );
    }

    /// `local` isolation is the maintainer's own working tree, so two candidates in it would write
    /// over each other (ANA-2 §4.6).
    #[tokio::test]
    async fn local_with_fan_out_is_refused() {
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT && phase.name == "implement" {
                    phase.isolation = Some(Isolation::Local);
                    phase.fan_out = 3;
                    // MOD-138 T4: resolve validates the row (`ck_phase_split_fan_out`).
                    phase.split = Split::Compete;
                }
            }
        });
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("`local` cannot fan out");
        assert_eq!(
            error,
            ResolveError::LocalFanOut {
                phase: "implement".to_owned(),
                fan_out: 3,
            }
        );

        // `local` at `fan_out = 1` is the ordinary case and resolves.
        let store = store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id == ids::GRAPH_HTUI_FEAT && phase.name == "implement" {
                    phase.isolation = Some(Isolation::Local);
                }
            }
        });
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("one local step is fine")
            .snapshot;
        assert_eq!(snapshot.phases[2].isolation, Isolation::Local);
    }

    /// Plan D14, and the only case an empty scope is legitimate in.
    #[tokio::test]
    async fn empty_scope_with_primary_is_refused() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let source = TestSource::claude(&store);

        // The demo fixture seeds no `repo` rows at all, so there is no primary to overlap with and
        // an empty scope is what the item honestly has.
        let resolved = resolve(
            &store,
            &source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            Some(&[]),
            ids::BOX,
        )
        .await
        .expect("a project with no repository resolves to an empty scope");
        assert_eq!(resolved.repo_scope, Vec::<RepoId>::new());

        let repo = store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the fixture project takes a repo");

        let error = resolve(
            &store,
            &source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            Some(&[]),
            ids::BOX,
        )
        .await
        .expect_err("an empty scope overlaps nothing, which defeats `claim_run`");
        assert_eq!(
            error,
            ResolveError::EmptyScopeWithPrimary {
                item: item.id,
                repo: repo.id,
            }
        );

        // No requested scope resolves to the primary repo, which is §4.7's "an empty
        // `touched_paths` overlaps the whole primary repo" expressed as a scope.
        let resolved = resolve(
            &store,
            &source,
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("the primary answers");
        assert_eq!(resolved.repo_scope, vec![repo.id]);
    }

    /// Plan D79: `StartRun`'s resolution writes §4.7's scope into the snapshot, beside the
    /// `repo_scope` it derives from the same `touched_paths`.
    #[tokio::test]
    async fn resolve_writes_the_scope_into_the_snapshot() {
        let store = MemStore::demo();
        let core = store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "core".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the fixture project takes a repo");
        let mut item = feat_1(&store).await;
        item.touched_paths = vec!["src/**".to_owned()];

        let resolved = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("the seeded feature graph resolves");
        assert_eq!(resolved.repo_scope, vec![core.id]);
        assert_eq!(
            resolved.snapshot.scope,
            Some(RunScope {
                repos: BTreeMap::from([(
                    core.id,
                    RepoScope {
                        isolated: true,
                        local: false,
                        prefixes: vec!["src/".to_owned()],
                    },
                )]),
            }),
            "every seeded phase resolves to the `worktree` default"
        );

        // A qualifier the project does not carry refuses before a run row exists.
        item.touched_paths = vec!["web:app/**".to_owned()];
        let error = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect_err("`web` is no repo of the project");
        assert_eq!(
            error,
            ResolveError::UnknownTouchedRepo {
                item: item.id,
                name: "web".to_owned(),
            }
        );
    }

    /// ANA-2 §4.1 with MOD-9 D80: the clone is deep over `step_graph_phase` **and** over the
    /// source phases' own attachments, each copied onto its cloned phase, and it is marked
    /// `is_override`. Project and global attachments are not copied: they already apply to the
    /// clone, and a copy would be a second row at the same level.
    ///
    /// This replaces `override_clone_leaves_bindings_alone`, whose "a cloned phase inherits the
    /// project's bindings and none of the original phase's" pinned the clone gap D80 closes.
    #[tokio::test]
    async fn override_clone_carries_phase_attachments_and_marks_itself() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let source = TestSource::claude(&store);

        let source_rows = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read");
        let project_level = store
            .bound_skills(ids::PROJECT_HTUI, None)
            .await
            .expect("MemStore never fails a read");
        let bound_before = store
            .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
            .await
            .expect("MemStore never fails a read");
        assert_ne!(
            bound_before, project_level,
            "the fixture's one phase-level binding is what makes this test able to fail"
        );

        let clone = override_graph(&store, &source, &item)
            .await
            .expect("the item takes an override");
        assert_eq!(clone.name, format!("{}-override", item.key));
        assert!(clone.is_override, "the clone is written as an override");
        assert!(
            store
                .step_graphs(ids::PROJECT_HTUI)
                .await
                .expect("MemStore never fails a read")
                .iter()
                .any(|graph| graph.id == clone.id && graph.is_override),
            "and reads back as one"
        );

        let original: Vec<StepGraphPhase> = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        let cloned: Vec<StepGraphPhase> = store
            .phases(clone.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(cloned.len(), original.len(), "every phase is copied");
        for (left, right) in original.iter().zip(&cloned) {
            assert_ne!(left.id, right.id, "a clone mints fresh phase ids");
            assert_eq!(left.name, right.name);
            assert_eq!(left.input_kinds, right.input_kinds);
            assert_eq!(left.gate, right.gate);
            assert_eq!(left.gate_hard, right.gate_hard);
            assert_eq!(left.template_name, right.template_name);
        }

        let implement = cloned
            .iter()
            .find(|phase| phase.name == "implement")
            .expect("the feature graph has an implement phase");
        let after = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read");
        let copies: Vec<&SkillBinding> = after
            .iter()
            .filter(|row| {
                row.phase_id
                    .is_some_and(|phase| cloned.iter().any(|clone| clone.id == phase))
            })
            .collect();
        assert_eq!(
            copies.len(),
            1,
            "the source's one phase-level row is copied, onto implement only: {copies:?}"
        );
        let copy = copies[0];
        assert_eq!(
            (copy.skill_id, copy.project_id, copy.phase_id),
            (
                ids::SKILL_RUST_STYLE,
                Some(ids::PROJECT_HTUI),
                Some(implement.id)
            ),
            "the copy sits on the cloned implement phase"
        );
        assert_eq!(
            (
                copy.pinned_version,
                copy.position,
                copy.activation,
                copy.globs.as_slice(),
                copy.languages.as_slice()
            ),
            (Some(1), 2, Activation::Always, &[][..], &[][..]),
            "and says what the source row says"
        );
        assert_eq!(
            store
                .bound_skills(ids::PROJECT_HTUI, Some(implement.id))
                .await
                .expect("MemStore never fails a read"),
            bound_before,
            "the cloned implement phase binds what the original one does"
        );
        assert_eq!(
            store
                .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
                .await
                .expect("MemStore never fails a read"),
            bound_before,
            "and the original phase's bindings are untouched"
        );
        let untouched: Vec<SkillBinding> = after
            .iter()
            .filter(|row| row.id != copy.id)
            .cloned()
            .collect();
        assert_eq!(
            untouched, source_rows,
            "the source rows are untouched, ids and updated_at included"
        );

        let repointed = store
            .item(item.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the item is still there");
        assert_eq!(repointed.step_graph_id, Some(clone.id));

        // Resolution now follows the override, and the two graphs have the same topology because
        // the clone copied every field the digest covers.
        let resolved = resolve(
            &store,
            &source,
            &repointed,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("the override resolves");
        assert_eq!(resolved.snapshot.graph.id, clone.id);
        // MOD-138 F14: the clone has the same keys and edges, so the same v2 digest too.
        assert_eq!(resolved.snapshot.topology, FEATURE_TOPOLOGY_V2);
    }

    /// MOD-37 R-6: the clone carries `phase_agent` too. Each source phase's candidate rows are
    /// written onto its clone, re-keyed to the cloned phase, and a phase with none gets none.
    #[tokio::test]
    async fn override_clone_carries_phase_agents() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let written: Vec<PhaseAgent> = [
            (1, ids::AGENT_AGY, "opus"),
            (0, ids::AGENT_CLAUDE, "sonnet"),
        ]
        .into_iter()
        .map(|(position, agent_id, model)| PhaseAgent {
            phase_id: ids::PHASE_HTUI_IMPLEMENT,
            position,
            agent_id,
            model: model.to_owned(),
        })
        .collect();
        store
            .create_phase_agents(ids::PHASE_HTUI_IMPLEMENT, &written)
            .await
            .expect("the source rows are written");
        let source_rows = store
            .phase_agents(ids::PHASE_HTUI_IMPLEMENT)
            .await
            .expect("MemStore never fails a read");

        let clone = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect("the item takes an override");

        let cloned = store
            .phases(clone.id)
            .await
            .expect("MemStore never fails a read");
        let implement = cloned
            .iter()
            .find(|phase| phase.name == "implement")
            .expect("the feature graph has an implement phase");
        assert_eq!(
            store
                .phase_agents(implement.id)
                .await
                .expect("MemStore never fails a read"),
            source_rows
                .iter()
                .map(|row| PhaseAgent {
                    phase_id: implement.id,
                    ..row.clone()
                })
                .collect::<Vec<_>>(),
            "the cloned implement phase carries the source rows, re-keyed"
        );
        for phase in cloned.iter().filter(|phase| phase.id != implement.id) {
            assert!(
                store
                    .phase_agents(phase.id)
                    .await
                    .expect("MemStore never fails a read")
                    .is_empty(),
                "`{}` had no candidate, so its clone has none",
                phase.name
            );
        }
        assert_eq!(
            store
                .phase_agents(ids::PHASE_HTUI_IMPLEMENT)
                .await
                .expect("MemStore never fails a read"),
            source_rows,
            "and the source rows are untouched"
        );
    }

    /// MOD-141 M2 D1 (blueprint C4): the clone spreads the source phase, so a phase's deadline
    /// and judge reach its clone, and a phase with none keeps none.
    #[tokio::test]
    async fn override_clone_keeps_the_deadline_and_the_judge() {
        let store = MemStore::demo();
        replace_feature(&store, |body| {
            let implement = body
                .nodes
                .iter_mut()
                .find(|node| node.phase.name == "implement")
                .expect("the feature graph has an implement phase");
            implement.phase.deadline_seconds = Some(900);
            implement.phase.judge_agent_id = Some(ids::AGENT_AGY);
            implement.phase.judge_model = Some("opus".to_owned());
        })
        .await;

        let clone = override_graph(&store, &TestSource::claude(&store), &feat_1(&store).await)
            .await
            .expect("the item takes an override");

        let cloned = store
            .phases(clone.id)
            .await
            .expect("MemStore never fails a read");
        assert!(
            cloned.iter().any(|phase| phase.name == "implement"),
            "the clone has an implement phase"
        );
        for phase in &cloned {
            let want = if phase.name == "implement" {
                (Some(900), Some(ids::AGENT_AGY), Some("opus"))
            } else {
                (None, None, None)
            };
            assert_eq!(
                (
                    phase.deadline_seconds,
                    phase.judge_agent_id,
                    phase.judge_model.as_deref()
                ),
                want,
                "`{}`'s clone keeps its deadline and judge",
                phase.name
            );
        }
    }

    /// MOD-9 D96 (the maintainer's "check first, then refuse"): a source phase attachment whose
    /// qualified glob names a repo the project no longer has would be refused by
    /// `set_skill_binding` (D79), so the clone refuses before it writes anything — no
    /// `-override` graph, no copied row, the item still on its graph. A qualified glob naming a
    /// repo the project has passes the same check, so the refusal names the stale one.
    #[tokio::test]
    async fn override_clone_refuses_a_stale_repo_glob_before_writing() {
        // Planted in the fixture rather than written: the writer refuses exactly this row.
        let store = store_with(|data| {
            let phase_row = data
                .skill_bindings
                .iter()
                .find(|row| row.phase_id == Some(ids::PHASE_HTUI_IMPLEMENT))
                .expect("the fixture's one phase-level binding")
                .clone();
            data.skill_bindings.push(SkillBinding {
                id: SkillBindingId::new(),
                skill_id: ids::SKILL_TESTS,
                pinned_version: None,
                activation: Activation::Glob,
                globs: vec!["core:src/**".to_owned(), "gone:**/*.rs".to_owned()],
                ..phase_row
            });
        });
        store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "core".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the fixture project takes a repo");
        let item = feat_1(&store).await;
        let slug = store
            .project(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture holds htui")
            .slug;
        let graphs_before = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read");
        let rows_before = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read");

        let error = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect_err("`gone` is no repo of the project");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(glob_names_unknown_repo(
                "gone:**/*.rs",
                "gone",
                &slug
            )))
        );

        let graphs_after = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read");
        assert!(
            !graphs_after
                .iter()
                .any(|graph| graph.name == format!("{}-override", item.key)),
            "no override graph is written: {graphs_after:?}"
        );
        assert_eq!(graphs_after, graphs_before, "nor any other graph");
        assert_eq!(
            store
                .skill_bindings(Some(ids::PROJECT_HTUI))
                .await
                .expect("MemStore never fails a read"),
            rows_before,
            "nor any attachment"
        );
        assert_eq!(
            store
                .item(item.id)
                .await
                .expect("MemStore never fails a read")
                .expect("the item is still there")
                .step_graph_id,
            item.step_graph_id,
            "and the item is not repointed"
        );
    }

    /// MOD-9 D96: the pre-check is `set_skill_binding`'s whole rule chain (D78), not its repo
    /// rule alone. A source phase attachment at `position` -1 (a row the writer refuses, so
    /// planted) refuses the clone with the writer's own sentence before anything is written.
    #[tokio::test]
    async fn override_clone_refuses_a_negative_position_before_writing() {
        let store = store_with(|data| {
            let phase_row = data
                .skill_bindings
                .iter_mut()
                .find(|row| row.phase_id == Some(ids::PHASE_HTUI_IMPLEMENT))
                .expect("the fixture's one phase-level binding");
            phase_row.position = -1;
        });
        let item = feat_1(&store).await;
        let graphs_before = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read");
        let rows_before = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read");

        let error = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect_err("the writer refuses a negative position");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(negative_position(-1)))
        );

        let graphs_after = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read");
        assert!(
            !graphs_after
                .iter()
                .any(|graph| graph.name == format!("{}-override", item.key)),
            "no override graph is written: {graphs_after:?}"
        );
        assert_eq!(graphs_after, graphs_before, "nor any other graph");
        assert_eq!(
            store
                .skill_bindings(Some(ids::PROJECT_HTUI))
                .await
                .expect("MemStore never fails a read"),
            rows_before,
            "nor any attachment"
        );
        assert_eq!(
            store
                .item(item.id)
                .await
                .expect("MemStore never fails a read")
                .expect("the item is still there")
                .step_graph_id,
            item.step_graph_id,
            "and the item is not repointed"
        );
    }

    /// `(from name, to name, kind, loop_limit)` of every edge, in the store's order: what a clone
    /// must copy, with the phase ids it must not.
    fn named_edges(
        phases: &[StepGraphPhase],
        edges: &[StepGraphEdge],
    ) -> Vec<(String, String, EdgeKind, Option<i32>)> {
        let name = |id: PhaseId| {
            phases
                .iter()
                .find(|phase| phase.id == id)
                .map(|phase| phase.name.clone())
                .expect("an edge joins two phases of its graph")
        };
        edges
            .iter()
            .map(|edge| (name(edge.from), name(edge.to), edge.kind, edge.loop_limit))
            .collect()
    }

    /// Every edge of `clone` is the clone's own: its `graph_id`, both endpoints among its phases,
    /// an id none of `elsewhere` holds.
    async fn assert_edges_are_the_clones(
        store: &MemStore,
        clone: StepGraphId,
        elsewhere: &[StepGraphEdge],
    ) {
        let phases = store
            .phases(clone)
            .await
            .expect("MemStore never fails a read");
        for edge in store
            .graph_edges(clone)
            .await
            .expect("MemStore never fails a read")
        {
            assert_eq!(edge.graph_id, clone, "the edge belongs to the clone");
            for end in [edge.from, edge.to] {
                assert!(
                    phases.iter().any(|phase| phase.id == end),
                    "edge {} ends at {end}, a phase of the clone",
                    edge.id
                );
            }
            assert!(
                !elsewhere.iter().any(|other| other.id == edge.id),
                "edge {} has a fresh id",
                edge.id
            );
        }
    }

    /// The `-override` graphs of `htui`.
    async fn overrides(store: &MemStore) -> Vec<StepGraph> {
        store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .filter(|graph| graph.is_override)
            .collect()
    }

    /// MOD-137 D13: the first override copies the kind default's edges too, remapped onto the
    /// cloned phases, in the one `replace_graph` that writes the clone (revision 0 → 1).
    #[tokio::test]
    async fn override_clone_copies_edges_remapped() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let default_phases = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        let default_edges = store
            .graph_edges(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        assert!(
            !default_edges.is_empty(),
            "the seeded feature graph has edges, which is what makes this test able to fail"
        );

        let clone = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect("the item takes an override");
        assert_eq!(clone.revision, 1, "one replace_graph over a fresh graph");

        let cloned_phases = store
            .phases(clone.id)
            .await
            .expect("MemStore never fails a read");
        let cloned_edges = store
            .graph_edges(clone.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            named_edges(&cloned_phases, &cloned_edges),
            named_edges(&default_phases, &default_edges),
            "the clone's edges say what the default's do"
        );
        assert_edges_are_the_clones(&store, clone.id, &default_edges).await;
    }

    /// MOD-137 D13 (ANA-2 `:317-320`): a second override re-clones from the kind's default graph
    /// into the same override graph, with fresh phase and edge ids; the user's edit of the override
    /// is not a source, and the item is neither repointed nor versioned.
    #[tokio::test]
    async fn re_override_reclones_from_the_kind_default_into_the_same_graph() {
        let store = MemStore::demo();
        let source = TestSource::claude(&store);
        let first = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the item takes an override");

        // The user edits the override: `prd` goes, with its edges.
        let mut body = store
            .graph_body(first.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the override is there");
        let first_phases: Vec<PhaseId> = body.nodes.iter().map(|node| node.phase.id).collect();
        let first_edges = body.edges.clone();
        let prd = body
            .nodes
            .iter()
            .find(|node| node.phase.name == "prd")
            .expect("the feature graph has a prd phase")
            .phase
            .id;
        body.nodes.retain(|node| node.phase.id != prd);
        body.edges.retain(|edge| edge.from != prd && edge.to != prd);
        let edited = store
            .replace_graph(first.id, body.graph.revision, body)
            .await
            .expect("the edit is a valid body");
        assert!(
            matches!(&edited, CasOutcome::Applied(body) if body.graph.revision == 2),
            "the edit applies: {edited:?}"
        );

        let pointed = feat_1(&store).await;
        assert_eq!(pointed.step_graph_id, Some(first.id));
        let again = override_graph(&store, &source, &pointed)
            .await
            .expect("the item takes a second override");
        assert_eq!(again.id, first.id, "the same graph, re-cloned");
        assert_eq!(again.revision, 3, "create 0, clone 1, edit 2, re-clone 3");
        assert_eq!(again.name, format!("{}-override", pointed.key));
        assert!(again.is_override);
        assert_eq!(overrides(&store).await.len(), 1, "no second override graph");

        let after = feat_1(&store).await;
        assert_eq!(
            after.step_graph_id,
            Some(first.id),
            "the item is not repointed"
        );
        assert_eq!(after.version, pointed.version, "nor versioned");

        let default_phases = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        let default_edges = store
            .graph_edges(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        let phases = store
            .phases(first.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            phases.iter().map(|phase| &phase.name).collect::<Vec<_>>(),
            default_phases
                .iter()
                .map(|phase| &phase.name)
                .collect::<Vec<_>>(),
            "the default's phases, `prd` included"
        );
        for phase in &phases {
            assert!(
                !first_phases.contains(&phase.id),
                "`{}` has a fresh id, not the first clone's",
                phase.name
            );
            assert!(
                !default_phases.iter().any(|row| row.id == phase.id),
                "nor the default's"
            );
        }
        let edges = store
            .graph_edges(first.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            named_edges(&phases, &edges),
            named_edges(&default_phases, &default_edges),
            "the default's edges, remapped"
        );
        let mut seen = first_edges;
        seen.extend(default_edges);
        assert_edges_are_the_clones(&store, first.id, &seen).await;
    }

    /// MOD-137 D13: the re-clone reads the kind default as it is now, so a phase and edges added
    /// to it after the first override reach the override.
    #[tokio::test]
    async fn re_override_picks_up_a_changed_kind_default() {
        let store = MemStore::demo();
        let source = TestSource::claude(&store);
        let first = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the item takes an override");

        let mut body = store
            .graph_body(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read")
            .expect("the kind default is there");
        let review = body
            .nodes
            .iter()
            .find(|node| node.phase.name == "review")
            .expect("the feature graph has a review phase")
            .phase
            .clone();
        let docs = PhaseId::new();
        let position = body
            .nodes
            .iter()
            .map(|node| node.phase.position)
            .max()
            .expect("the default has phases")
            + 1;
        body.nodes.push(NodeBody {
            phase: StepGraphPhase {
                id: docs,
                position,
                name: "docs".to_owned(),
                input_kinds: vec!["review".to_owned()],
                ..review.clone()
            },
            agents: Vec::new(),
            parts: Vec::new(),
        });
        for (from, to, kind, loop_limit) in [
            (review.id, docs, EdgeKind::Next, None),
            (docs, review.id, EdgeKind::Fails, Some(1)),
        ] {
            body.edges.push(StepGraphEdge {
                id: EdgeId::new(),
                graph_id: ids::GRAPH_HTUI_FEAT,
                from,
                to,
                kind,
                predicate: None,
                loop_limit,
            });
        }
        let changed = store
            .replace_graph(ids::GRAPH_HTUI_FEAT, body.graph.revision, body)
            .await
            .expect("the change is a valid body");
        assert!(
            matches!(changed, CasOutcome::Applied(_)),
            "the change applies: {changed:?}"
        );

        let again = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the item takes a second override");
        assert_eq!(again.id, first.id);

        let default_phases = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        let phases = store
            .phases(first.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            phases.iter().map(|phase| &phase.name).collect::<Vec<_>>(),
            default_phases
                .iter()
                .map(|phase| &phase.name)
                .collect::<Vec<_>>(),
            "the override has the default's phases now"
        );
        assert!(phases.iter().any(|phase| phase.name == "docs"));
        let edges = named_edges(
            &phases,
            &store
                .graph_edges(first.id)
                .await
                .expect("MemStore never fails a read"),
        );
        assert_eq!(
            edges,
            named_edges(
                &default_phases,
                &store
                    .graph_edges(ids::GRAPH_HTUI_FEAT)
                    .await
                    .expect("MemStore never fails a read"),
            ),
            "and its edges"
        );
        for edge in [
            ("review".to_owned(), "docs".to_owned(), EdgeKind::Next, None),
            (
                "docs".to_owned(),
                "review".to_owned(),
                EdgeKind::Fails,
                Some(1),
            ),
        ] {
            assert!(edges.contains(&edge), "{edge:?} reached the override");
        }
    }

    /// MOD-137 D13 with MOD-9 D80: a re-override copies each default-phase attachment onto the
    /// fresh clone phase; the first clone's copies go with its phases.
    #[tokio::test]
    async fn re_override_moves_phase_attachments_to_the_fresh_clone() {
        let store = MemStore::demo();
        let source = TestSource::claude(&store);
        let first = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the item takes an override");
        let first_phases: Vec<PhaseId> = store
            .phases(first.id)
            .await
            .expect("MemStore never fails a read")
            .iter()
            .map(|phase| phase.id)
            .collect();
        let source_row = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .find(|row| row.phase_id == Some(ids::PHASE_HTUI_IMPLEMENT))
            .expect("the fixture's one phase-level binding");

        override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the item takes a second override");

        let phases = store
            .phases(first.id)
            .await
            .expect("MemStore never fails a read");
        let implement = phases
            .iter()
            .find(|phase| phase.name == "implement")
            .expect("the feature graph has an implement phase");
        let rows = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read");
        assert!(
            !rows.iter().any(|row| row
                .phase_id
                .is_some_and(|phase| first_phases.contains(&phase))),
            "the first clone's copies are gone: {rows:?}"
        );
        let copies: Vec<&SkillBinding> = rows
            .iter()
            .filter(|row| {
                row.phase_id
                    .is_some_and(|phase| phases.iter().any(|clone| clone.id == phase))
            })
            .collect();
        assert_eq!(copies.len(), 1, "one copy, on implement only: {copies:?}");
        let copy = copies[0];
        assert_eq!(copy.phase_id, Some(implement.id));
        assert_eq!(
            (
                copy.skill_id,
                copy.pinned_version,
                copy.position,
                copy.activation,
                copy.globs.as_slice()
            ),
            (
                source_row.skill_id,
                source_row.pinned_version,
                source_row.position,
                source_row.activation,
                source_row.globs.as_slice()
            ),
            "and says what the source row says"
        );
        assert!(
            rows.contains(&source_row),
            "the source row is untouched, id and updated_at included"
        );
    }

    /// MOD-137 D13 with MOD-9 D96: the re-clone runs the same attachment pre-check before it
    /// writes. A default-phase glob naming a repo the project has renamed refuses the re-override
    /// with the writer's sentence, and the override is left as it was.
    #[tokio::test]
    async fn re_override_refuses_a_stale_attachment_before_writing() {
        let store = store_with(|data| {
            let phase_row = data
                .skill_bindings
                .iter()
                .find(|row| row.phase_id == Some(ids::PHASE_HTUI_IMPLEMENT))
                .expect("the fixture's one phase-level binding")
                .clone();
            data.skill_bindings.push(SkillBinding {
                id: SkillBindingId::new(),
                skill_id: ids::SKILL_TESTS,
                pinned_version: None,
                activation: Activation::Glob,
                globs: vec!["core:src/**".to_owned()],
                ..phase_row
            });
        });
        let repo = store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "core".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the fixture project takes a repo");
        let source = TestSource::claude(&store);
        let first = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("`core` is a repo of the project, so the first override passes");

        let renamed = store
            .update_repo(
                repo.id,
                repo.updated_at,
                RepoPatch {
                    name: Some("lib".to_owned()),
                    ..RepoPatch::default()
                },
            )
            .await
            .expect("the repo takes a new name");
        assert!(matches!(renamed, CasOutcome::Applied(_)), "{renamed:?}");
        let slug = store
            .project(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture holds htui")
            .slug;
        let body_before = store
            .graph_body(first.id)
            .await
            .expect("MemStore never fails a read");
        let rows_before = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read");
        let pointed = feat_1(&store).await;

        let error = override_graph(&store, &source, &pointed)
            .await
            .expect_err("`core` is no repo of the project any more");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(glob_names_unknown_repo(
                "core:src/**",
                "core",
                &slug
            )))
        );

        let body_after = store
            .graph_body(first.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            body_after.as_ref().map(|body| body.graph.revision),
            Some(1),
            "the override's revision is unchanged"
        );
        assert_eq!(body_after, body_before, "and so is its body");
        assert_eq!(
            store
                .skill_bindings(Some(ids::PROJECT_HTUI))
                .await
                .expect("MemStore never fails a read"),
            rows_before,
            "nor any attachment"
        );
        assert_eq!(feat_1(&store).await, pointed, "and the item is untouched");
    }

    /// The `<key>-override` graph's phase names, by position: what a clone of the feature default
    /// holds.
    async fn phase_names(store: &MemStore, graph: StepGraphId) -> Vec<String> {
        store
            .phases(graph)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|phase| phase.name)
            .collect()
    }

    /// MOD-137 R1b (review C-M1): a first override `replace_graph` refuses after the graph row is
    /// created leaves that row, which no item names. Calling again adopts it — `replace_graph` at
    /// its revision, then the repoint — instead of tripping over `(scope, name)` for ever.
    #[tokio::test]
    async fn a_first_override_refused_by_replace_graph_is_adopted_on_retry() {
        // A source phase naming a persona with no row: the store refuses it (an FK, so no pure
        // pre-check catches it), and only once the graph row exists.
        let ghost = PersonaId::new();
        let store = store_with(|data| {
            data.phases
                .iter_mut()
                .find(|phase| phase.id == ids::PHASE_HTUI_IMPLEMENT)
                .expect("the feature graph has an implement phase")
                .persona_id = Some(ghost);
        });
        let source = TestSource::claude(&store);
        let item = feat_1(&store).await;

        let error = override_graph(&store, &source, &item)
            .await
            .expect_err("the clone names a persona with no row");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(references_no_row(
                "step_graph_phase.persona_id",
                ghost,
                "persona",
            )))
        );
        let orphans = overrides(&store).await;
        assert_eq!(
            orphans.len(),
            1,
            "the refused clone left its row: {orphans:?}"
        );
        let orphan = orphans[0].clone();
        assert_eq!(orphan.name, format!("{}-override", item.key));
        assert!(phase_names(&store, orphan.id).await.is_empty());
        assert_eq!(feat_1(&store).await, item, "and the item is untouched");

        // The user fixes the source phase and overrides again.
        let implement = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .find(|phase| phase.id == ids::PHASE_HTUI_IMPLEMENT)
            .expect("the feature graph has an implement phase");
        let fixed = store
            .update_phase(
                implement.id,
                implement.updated_at,
                PhasePatch {
                    persona: Some(None),
                    ..PhasePatch::default()
                },
            )
            .await
            .expect("the phase drops its persona");
        assert!(matches!(fixed, CasOutcome::Applied(_)), "{fixed:?}");

        let clone = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the retry adopts the orphan");
        assert_eq!(
            clone.id, orphan.id,
            "the orphan is adopted, not created beside"
        );
        assert_eq!(
            clone.revision,
            orphan.revision + 1,
            "through one replace_graph"
        );
        assert_eq!(overrides(&store).await.len(), 1, "no second override graph");
        assert_eq!(
            phase_names(&store, clone.id).await,
            phase_names(&store, ids::GRAPH_HTUI_FEAT).await
        );
        assert_eq!(feat_1(&store).await.step_graph_id, Some(clone.id));
    }

    /// MOD-137 R1b (review C-M1): a body `replace_graph` refuses on its face — here a phase whose
    /// `split` disagrees with its `fan_out` — is refused by the same pure rules before the graph
    /// row is created, so it leaves no orphan.
    #[tokio::test]
    async fn a_body_the_store_refuses_on_its_face_leaves_no_orphan() {
        let store = store_with(|data| {
            data.phases
                .iter_mut()
                .find(|phase| phase.id == ids::PHASE_HTUI_IMPLEMENT)
                .expect("the feature graph has an implement phase")
                .fan_out = 2;
        });
        let item = feat_1(&store).await;
        let graphs_before = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read");

        let error = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect_err("`fan_out` 2 under `split` none is refused");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(split_fan_out_mismatch(
                "implement",
                Split::None,
                2
            )))
        );
        assert_eq!(
            store
                .step_graphs(ids::PROJECT_HTUI)
                .await
                .expect("MemStore never fails a read"),
            graphs_before,
            "no graph row is created"
        );
        assert_eq!(feat_1(&store).await, item, "and the item is untouched");
    }

    /// MOD-137 R1b (review C-M1): an item that moved while its override was being cloned leaves
    /// a whole clone no item names — phases, edges and the copied attachment. Overriding again
    /// adopts it, re-clones it with fresh phase ids (the stale attachment copy goes with the old
    /// phases) and repoints the item.
    #[tokio::test]
    async fn a_diverged_repoint_is_adopted_on_retry() {
        let store = MemStore::demo();
        let source = TestSource::claude(&store);
        let stale = feat_1(&store).await;
        let moved = store
            .update_item(
                stale.id,
                stale.version,
                ItemPatch {
                    title: Some("moved under the clone".to_owned()),
                    author_id: stale.created_by,
                    reason: "edit".to_owned(),
                    ..ItemPatch::default()
                },
            )
            .await
            .expect("the item takes an edit");
        assert!(matches!(moved, UpdateOutcome::Updated(_)), "{moved:?}");

        let error = override_graph(&store, &source, &stale)
            .await
            .expect_err("the repoint diverges");
        let head = feat_1(&store).await;
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(format!(
                "item {} moved to version {} while an override graph was being cloned; override \
                 again",
                stale.key, head.version
            )))
        );
        let orphans = overrides(&store).await;
        assert_eq!(orphans.len(), 1, "the clone is left whole: {orphans:?}");
        let orphan = orphans[0].clone();
        let orphan_phases: Vec<PhaseId> = store
            .phases(orphan.id)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|phase| phase.id)
            .collect();
        assert!(!orphan_phases.is_empty(), "with its phases");
        assert_eq!(
            head.step_graph_id, stale.step_graph_id,
            "and no item names it"
        );

        let clone = override_graph(&store, &source, &head)
            .await
            .expect("the retry adopts the orphan");
        assert_eq!(
            clone.id, orphan.id,
            "the orphan is adopted, not created beside"
        );
        assert_eq!(clone.revision, 2, "clone 1, re-clone 2");
        assert_eq!(overrides(&store).await.len(), 1, "no second override graph");
        assert_eq!(feat_1(&store).await.step_graph_id, Some(clone.id));

        let phases = store
            .phases(clone.id)
            .await
            .expect("MemStore never fails a read");
        assert!(
            phases
                .iter()
                .all(|phase| !orphan_phases.contains(&phase.id)),
            "fresh phase ids"
        );
        let copies: Vec<SkillBinding> = store
            .skill_bindings(Some(ids::PROJECT_HTUI))
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .filter(|row| {
                row.phase_id.is_some_and(|phase| {
                    phases.iter().any(|clone| clone.id == phase) || orphan_phases.contains(&phase)
                })
            })
            .collect();
        assert_eq!(
            copies.len(),
            1,
            "the source's one phase-level row, once, on the fresh clone: {copies:?}"
        );
    }

    /// MOD-137 R1b (review C-M1): a graph holding the `<key>-override` name that an item runs
    /// under is not an orphan, so the clone refuses rather than writing over it.
    #[tokio::test]
    async fn an_override_name_another_item_runs_under_is_not_adopted() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let taken = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Project(ids::PROJECT_HTUI),
                name: format!("{}-override", item.key),
                description: String::new(),
                is_override: true,
            })
            .await
            .expect("the graph is written");
        let other = store
            .items(
                &Scope {
                    workspace_id: ids::WORKSPACE_PLATFORM,
                    project_ids: vec![ids::PROJECT_HTUI],
                },
                &ItemFilter::default(),
            )
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .find(|row| row.id != item.id)
            .expect("htui holds a second item");
        let other = store
            .item(other.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the item is there");
        let pointed = store
            .update_item(
                other.id,
                other.version,
                ItemPatch {
                    step_graph_id: Some(Some(taken.id)),
                    author_id: other.created_by,
                    reason: "graph_override".to_owned(),
                    ..ItemPatch::default()
                },
            )
            .await
            .expect("the other item takes the graph");
        assert!(matches!(pointed, UpdateOutcome::Updated(_)), "{pointed:?}");

        let error = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect_err("the name is in use");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(format!(
                "graph `{}-override` already exists and item {} runs under it; rename it and \
                 override again",
                item.key, other.key
            )))
        );
        assert!(
            phase_names(&store, taken.id).await.is_empty(),
            "nothing is written into it"
        );
        assert_eq!(feat_1(&store).await, item, "and the item is untouched");
    }

    /// MOD-137 R1b (review C-M1): a caller holding the item from before its override landed is
    /// not told someone else holds the name: the item itself runs under it, and a call with the
    /// fresh item re-clones it.
    #[tokio::test]
    async fn a_stale_copy_of_an_overridden_item_is_told_to_override_again() {
        let store = MemStore::demo();
        let source = TestSource::claude(&store);
        let stale = feat_1(&store).await;
        let first = override_graph(&store, &source, &stale)
            .await
            .expect("the item takes an override");

        let error = override_graph(&store, &source, &stale)
            .await
            .expect_err("the copy predates the override");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(format!(
                "item {} already runs under `{}-override`; override again",
                stale.key, stale.key
            )))
        );
        let again = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the fresh item re-clones");
        assert_eq!((again.id, again.revision), (first.id, 2));
    }

    /// MOD-137 R1b (review C-M1): a plain graph holding the `<key>-override` name is never
    /// adopted: an override is only ever written over an override.
    #[tokio::test]
    async fn a_plain_graph_holding_the_override_name_is_not_adopted() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let plain = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Project(ids::PROJECT_HTUI),
                name: format!("{}-override", item.key),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("the graph is written");

        let error = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect_err("the name is in use");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(format!(
                "graph `{}-override` already exists and is not an override; rename it and \
                 override again",
                item.key
            )))
        );
        assert!(
            phase_names(&store, plain.id).await.is_empty(),
            "nothing is written into it"
        );
        assert_eq!(feat_1(&store).await, item, "and the item is untouched");
    }

    /// MOD-137 R1b (review C-L1): re-override trusts `is_override` *and* the name. An item
    /// running under an override that is not its own `<key>-override` is refused, naming the
    /// graph, and nothing is written.
    #[tokio::test]
    async fn re_override_refuses_an_override_that_is_not_the_items_own() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        let foreign = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Project(ids::PROJECT_HTUI),
                name: "someone-else-override".to_owned(),
                description: String::new(),
                is_override: true,
            })
            .await
            .expect("the graph is written");
        let pointed = store
            .update_item(
                item.id,
                item.version,
                ItemPatch {
                    step_graph_id: Some(Some(foreign.id)),
                    author_id: item.created_by,
                    reason: "graph_override".to_owned(),
                    ..ItemPatch::default()
                },
            )
            .await
            .expect("the item takes the graph");
        assert!(matches!(pointed, UpdateOutcome::Updated(_)), "{pointed:?}");
        let pointed = feat_1(&store).await;

        let error = override_graph(&store, &TestSource::claude(&store), &pointed)
            .await
            .expect_err("the override is not the item's own");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(format!(
                "item {} runs under override graph `someone-else-override`, not its own `{}-override`",
                pointed.key, pointed.key
            )))
        );
        let after = store
            .graph_body(foreign.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the graph is there");
        assert_eq!(after.graph.revision, 0, "nothing is written into it");
        assert_eq!(overrides(&store).await.len(), 1, "nor beside it");
        assert_eq!(feat_1(&store).await, pointed, "and the item is untouched");
    }

    /// MOD-137 R1b (review C-L4): an edit to the override between the re-clone's read and its
    /// `replace_graph` answers `Stale`, which refuses the re-override and writes nothing; the
    /// "override again" it says holds.
    #[tokio::test]
    async fn re_override_refuses_an_override_edited_under_it() {
        let store = MemStore::demo();
        let first = override_graph(&store, &TestSource::claude(&store), &feat_1(&store).await)
            .await
            .expect("the item takes an override");
        let pointed = feat_1(&store).await;
        let before = store
            .graph_body(first.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the override is there");

        let racing = TestSource {
            race: Some(first.id),
            ..TestSource::claude(&store)
        };
        let error = override_graph(&store, &racing, &pointed)
            .await
            .expect_err("the override moved under the re-clone");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(format!(
                "override graph `{}-override` moved while it was being re-cloned; override again",
                pointed.key
            )))
        );
        let after = store
            .graph_body(first.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the override is there");
        assert_eq!(
            after.graph.revision, 2,
            "the racing edit's bump, and no re-clone"
        );
        let ids_of = |body: &GraphBody| -> Vec<PhaseId> {
            body.nodes.iter().map(|node| node.phase.id).collect()
        };
        assert_eq!(
            ids_of(&after),
            ids_of(&before),
            "the first clone's phases stay"
        );
        assert_eq!(feat_1(&store).await, pointed, "and the item is untouched");

        let again = override_graph(&store, &TestSource::claude(&store), &pointed)
            .await
            .expect("overriding again re-clones");
        assert_eq!((again.id, again.revision), (first.id, 3));
    }

    /// MOD-137 R1b (review C-L4): a source edge naming a phase the source read did not answer
    /// (a torn read: Postgres reads the body statement by statement) refuses the clone before
    /// anything is written.
    #[tokio::test]
    async fn an_edge_outside_the_source_read_refuses_the_clone_before_writing() {
        let store = store_with(|data| {
            data.edges.push(StepGraphEdge {
                id: EdgeId::new(),
                graph_id: ids::GRAPH_HTUI_FEAT,
                from: ids::PHASE_HTUI_IMPLEMENT,
                to: PhaseId::new(),
                kind: EdgeKind::Next,
                predicate: None,
                loop_limit: None,
            });
        });
        let item = feat_1(&store).await;
        let graphs_before = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read");
        let source_name = graphs_before
            .iter()
            .find(|graph| graph.id == ids::GRAPH_HTUI_FEAT)
            .expect("the feature graph")
            .name
            .clone();

        let error = override_graph(&store, &TestSource::claude(&store), &item)
            .await
            .expect_err("the edge's endpoint is no phase of the read");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(format!(
                "graph `{source_name}` moved while it was being cloned; override again"
            )))
        );
        assert_eq!(
            store
                .step_graphs(ids::PROJECT_HTUI)
                .await
                .expect("MemStore never fails a read"),
            graphs_before,
            "no graph is written"
        );
        assert_eq!(feat_1(&store).await, item, "and the item is untouched");
    }

    /// An item whose kind names no graph and which names none itself cannot be run.
    #[tokio::test]
    async fn an_item_with_no_graph_is_refused() {
        let store = store_with(|data| data.graphs.retain(|graph| graph.id != ids::GRAPH_HTUI_FEAT));
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("the kind's default graph is gone");
        assert_eq!(error, ResolveError::NoGraph(ids::HTUI_FEAT_1));
    }

    /// MOD-138 D12: a library graph `libflow` holding a copy of `feature`'s `implement` and
    /// `review` rows (built row by row, so D7 gives it default edges), made `FEAT`'s kind default
    /// through `update_item_kind`, which takes a library graph since D12. Returns its id.
    async fn feat_on_a_library_graph(store: &MemStore) -> StepGraphId {
        let library = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Library,
                name: "libflow".to_owned(),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("a library graph");
        let feature = store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read");
        for (position, name) in [(0, "implement"), (1, "review")] {
            let row = feature
                .iter()
                .find(|row| row.name == name)
                .expect("the feature graph has the phase");
            store
                .create_phase(&StepGraphPhase {
                    id: PhaseId::new(),
                    graph_id: library.id,
                    position,
                    ..row.clone()
                })
                .await
                .expect("the phase lands");
        }
        let kind = store
            .item_kinds(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .find(|kind| kind.id == ids::KIND_HTUI_FEAT)
            .expect("the fixture's FEAT kind");
        let outcome = store
            .update_item_kind(
                kind.id,
                kind.updated_at,
                htui_core::model::ItemKindPatch {
                    default_graph_id: Some(library.id),
                    ..htui_core::model::ItemKindPatch::default()
                },
            )
            .await
            .expect("a kind takes a library graph (D12)");
        assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
        library.id
    }

    /// MOD-138 D12: an item whose kind defaults to a library graph runs it, frozen with the
    /// library scope, and its templates resolve in the **item's** project: `htui`'s own newer
    /// `review` template is the one pinned.
    #[tokio::test]
    async fn an_items_run_resolves_a_library_graph_with_the_projects_templates() {
        let store = store_with(|data| {
            let latest = data
                .templates
                .iter()
                .filter(|row| row.project_id == ids::PROJECT_HTUI && row.name == "review")
                .max_by_key(|row| row.version)
                .cloned()
                .expect("the fixture seeds a `review` template");
            data.templates.push(PromptTemplate {
                id: PromptTemplateId::new(),
                version: latest.version + 1,
                ..latest
            });
        });
        let library = feat_on_a_library_graph(&store).await;
        let newest = store
            .prompt_template(ids::PROJECT_HTUI, "review", None)
            .await
            .expect("MemStore never fails a read")
            .expect("htui's review template")
            .version;

        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("a library graph resolves")
            .snapshot;

        assert_eq!(snapshot.graph.id, library);
        assert_eq!(snapshot.graph.scope, Some(GraphScope::Library));
        assert_eq!(
            snapshot.source,
            Some(SnapshotSource {
                how: SourceHow::KindDefault
            })
        );
        assert_eq!(
            snapshot
                .phases
                .iter()
                .map(|phase| phase.name.as_str())
                .collect::<Vec<_>>(),
            ["implement", "review"]
        );
        assert_eq!(
            snapshot.phases[1].template,
            SnapshotTemplate {
                name: "review".to_owned(),
                version: newest,
            },
            "the template is the item's project's latest"
        );
        assert_eq!(
            snapshot.edges,
            default_snapshot_edges(&snapshot.phases),
            "built row by row, the library graph is default-edged"
        );
    }

    /// MOD-138 D12: overriding an item whose graph is a library graph clones it into the item's
    /// project, edges copied; a re-override re-clones from the library graph (the kind default).
    #[tokio::test]
    async fn an_override_of_a_library_graph_clones_into_the_project() {
        let store = MemStore::demo();
        let library = feat_on_a_library_graph(&store).await;
        let source = TestSource::claude(&store);
        let library_phases = store
            .phases(library)
            .await
            .expect("MemStore never fails a read");
        let library_edges = store
            .graph_edges(library)
            .await
            .expect("MemStore never fails a read");
        assert!(!library_edges.is_empty(), "the library graph has edges");

        let clone = override_graph(&store, &source, &feat_1(&store).await)
            .await
            .expect("the item takes an override of the library graph");
        assert_eq!(clone.scope, GraphScope::Project(ids::PROJECT_HTUI));
        assert!(clone.is_override);
        let phases = store
            .phases(clone.id)
            .await
            .expect("MemStore never fails a read");
        let edges = store
            .graph_edges(clone.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            named_edges(&phases, &edges),
            named_edges(&library_phases, &library_edges),
            "the library graph's edges, remapped onto the clone"
        );
        assert_edges_are_the_clones(&store, clone.id, &library_edges).await;

        let pointed = feat_1(&store).await;
        assert_eq!(pointed.step_graph_id, Some(clone.id));
        let again = override_graph(&store, &source, &pointed)
            .await
            .expect("a second override re-clones");
        assert_eq!(again.id, clone.id, "the same override graph");
        assert_eq!(again.scope, GraphScope::Project(ids::PROJECT_HTUI));
        let recloned = store
            .phases(again.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            recloned.iter().map(|row| &row.name).collect::<Vec<_>>(),
            library_phases
                .iter()
                .map(|row| &row.name)
                .collect::<Vec<_>>(),
            "re-cloned from the library graph, the kind's default"
        );
        assert!(
            recloned
                .iter()
                .all(|row| !phases.iter().any(|first| first.id == row.id)),
            "with fresh phase ids"
        );
        let graph = store
            .graph_body(library)
            .await
            .expect("MemStore never fails a read")
            .expect("the library graph is untouched");
        assert_eq!(graph.graph.scope, GraphScope::Library);
    }

    /// `feature`'s `phase` with `fan_out` set, and `htui`'s settings naming `agy` as the judge.
    fn store_fanned(graph: StepGraphId, phase: &str, fan_out: i32) -> MemStore {
        let store = store_with(|data| {
            for row in &mut data.phases {
                if row.graph_id == graph && row.name == phase {
                    row.fan_out = fan_out;
                    // MOD-138 T4: resolve validates the row, and `ck_phase_split_fan_out` holds
                    // `compete` to exactly `fan_out > 1`.
                    row.split = Split::for_fan_out(fan_out);
                }
            }
        });
        store.set_project_settings(
            ids::PROJECT_HTUI,
            json!({ "judge_agent_id": ids::AGENT_AGY }),
        );
        store
    }

    /// `resolve` for any fixture item, with an `app_setting` map and no requested scope.
    async fn resolve_item(
        store: &MemStore,
        item: ItemId,
        app: &BTreeMap<String, Value>,
    ) -> std::result::Result<Resolved, ResolveError> {
        let item = store
            .item(item)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture holds the item");
        resolve(
            store,
            &TestSource::claude(store),
            &item,
            RunMode::Manual,
            app,
            None,
            ids::BOX,
        )
        .await
    }

    /// Plan D53 and ANA-5's table (`:1862`): the judge template is resolved to a concrete version
    /// at run creation, beside the judged phase, so an edit to `judge` mid-run does not move a
    /// run that already exists. The latest version wins; a project with no `judge` template at all
    /// pins nothing and is not refused, because `None` already means "latest at judge time".
    #[tokio::test]
    async fn the_judge_template_is_pinned_beside_the_judged_phase() {
        let store = store_with(|data| {
            let latest = data
                .templates
                .iter()
                .find(|row| row.project_id == ids::PROJECT_HTUI && row.name == "judge")
                .cloned()
                .expect("the fixture seeds a `judge` template");
            data.templates.push(PromptTemplate {
                id: PromptTemplateId::new(),
                version: latest.version + 1,
                ..latest
            });
            for row in &mut data.phases {
                if row.graph_id == ids::GRAPH_HTUI_FEAT && row.name == "implement" {
                    row.fan_out = 2;
                    // MOD-138 T4: resolve validates the row (`ck_phase_split_fan_out`).
                    row.split = Split::Compete;
                }
            }
        });
        store.set_project_settings(
            ids::PROJECT_HTUI,
            json!({ "judge_agent_id": ids::AGENT_AGY }),
        );
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the fanned-out feature graph resolves")
            .snapshot;
        let implement = &snapshot.phases[2];
        assert_eq!(implement.name, "implement");
        assert_eq!(
            implement
                .judge
                .as_ref()
                .expect("the project names a judge")
                .template,
            Some(SnapshotTemplate {
                name: "judge".to_owned(),
                version: 2
            }),
            "the latest `judge` version at run creation is pinned"
        );
        assert_eq!(
            implement.template.name, "implement",
            "the judged phase keeps its own template beside the judge's"
        );

        let store = store_with(|data| data.templates.retain(|row| row.name != "judge"));
        store.set_project_settings(
            ids::PROJECT_HTUI,
            json!({ "judge_agent_id": ids::AGENT_AGY }),
        );
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("an absent judge template is not a refusal")
            .snapshot;
        let judge = snapshot.phases[2]
            .judge
            .as_ref()
            .expect("the project names a judge");
        assert_eq!(
            judge.template, None,
            "nothing to pin, so latest at judge time"
        );
    }

    /// Plan D63, first cap: `app_setting.max_fan_out`, refused before a run row exists.
    #[tokio::test]
    async fn a_fan_out_above_max_fan_out_is_refused_naming_both_figures() {
        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "implement", 5);
        let error = resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect_err("5 is above the built-in 4");
        assert_eq!(
            error,
            ResolveError::FanOutCap {
                phase: "implement".to_owned(),
                fan_out: 5,
                max: 4,
            }
        );
        assert_eq!(
            error.to_string(),
            "phase `implement` fans out 5; max_fan_out is 4 (ANA-2 :868)"
        );

        // The `app_setting` rung moves the cap, and the refusal names the resolved figure.
        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "implement", 3);
        let app: BTreeMap<String, Value> =
            [("max_fan_out".to_owned(), json!(2))].into_iter().collect();
        let error = resolve_item(&store, ids::HTUI_FEAT_1, &app)
            .await
            .expect_err("3 is above a planted 2");
        assert_eq!(
            error,
            ResolveError::FanOutCap {
                phase: "implement".to_owned(),
                fan_out: 3,
                max: 2,
            }
        );

        // At the cap is not above it.
        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "implement", 2);
        resolve_item(&store, ids::HTUI_FEAT_1, &app)
            .await
            .expect("2 is at a cap of 2");
    }

    /// Plan D63, second cap (OQ-1): `Σ fan_out` plus one per judged phase. On `feature` with
    /// `implement` at 3 and a judge that is `1 + 1 + 3 + 1 + 1 = 7`, under the built-in 8 — the
    /// maintainer raised the default from ANA-2's 6 so this ordinary judged fan-out runs
    /// (`0004_max_agents_per_run_default.sql`). At 4 it is exactly 8, which is allowed; `plan` at 2
    /// as well, both judged, is `1 + 2 + 1 + 3 + 1 + 1 = 9`, which is not. The old reading
    /// (blueprint F-F) survives as an explicit 6.
    #[tokio::test]
    async fn planned_agents_above_the_cap_are_refused_at_resolution() {
        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "implement", 3);
        let snapshot = resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect("7 planned agents is under the built-in 8")
            .snapshot;
        assert_eq!(snapshot.settings.max_agents_per_run, 8);

        let app: BTreeMap<String, Value> = [("max_agents_per_run".to_owned(), json!(6))]
            .into_iter()
            .collect();
        let error = resolve_item(&store, ids::HTUI_FEAT_1, &app)
            .await
            .expect_err("7 planned agents is above a planted 6");
        assert_eq!(error, ResolveError::AgentCap { planned: 7, max: 6 });
        assert_eq!(
            error.to_string(),
            "run plans 7 agents; max_agents_per_run is 6 (ANA-2 :870-875)"
        );

        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "implement", 4);
        resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect("8 planned agents is at the cap");

        let store = store_with(|data| {
            for row in &mut data.phases {
                if row.graph_id == ids::GRAPH_HTUI_FEAT {
                    match row.name.as_str() {
                        "plan" => row.fan_out = 2,
                        "implement" => row.fan_out = 3,
                        _ => {}
                    }
                    // MOD-138 T4: resolve validates the row (`ck_phase_split_fan_out`).
                    row.split = Split::for_fan_out(row.fan_out);
                }
            }
        });
        store.set_project_settings(
            ids::PROJECT_HTUI,
            json!({ "judge_agent_id": ids::AGENT_AGY }),
        );
        let error = resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect_err("9 planned agents is above the built-in 8");
        assert_eq!(error, ResolveError::AgentCap { planned: 9, max: 8 });

        // With no judge a fanned-out phase is judged by a human, which costs no agent.
        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "implement", 3);
        store.set_project_settings(ids::PROJECT_HTUI, json!({}));
        resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect("1 + 1 + 3 + 1 = 6 with no judge");

        // The `app_setting` rung moves this cap too.
        let app: BTreeMap<String, Value> = [("max_agents_per_run".to_owned(), json!(5))]
            .into_iter()
            .collect();
        let error = resolve_item(&store, ids::HTUI_FEAT_1, &app)
            .await
            .expect_err("6 is above a planted 5");
        assert_eq!(error, ResolveError::AgentCap { planned: 6, max: 5 });
    }

    /// Criterion 8's shape: `analysis` is `research` then `verdict`, so `research` at 3 with a
    /// judge plans `3 + 1 + 1 = 5`, under the built-in 8.
    #[tokio::test]
    async fn the_analysis_graph_fans_out_three_under_the_default_cap() {
        let store = store_fanned(ids::GRAPH_HTUI_ANA, "research", 3);
        let snapshot = resolve_item(&store, ids::HTUI_ANA_2, &BTreeMap::new())
            .await
            .expect("5 planned agents is under 8")
            .snapshot;
        let names: Vec<&str> = snapshot
            .phases
            .iter()
            .map(|phase| phase.name.as_str())
            .collect();
        assert_eq!(names, ["research", "verdict"]);
        assert_eq!(snapshot.phases[0].fan_out, 3);
        assert!(
            snapshot.phases[0].judge.is_some(),
            "the project names a judge"
        );
        assert_eq!(snapshot.settings.max_fan_out, 4);
        assert_eq!(snapshot.settings.max_agents_per_run, 8);
    }

    /// Plan D64 (OQ-5): a `review` phase cannot fan out, refused at snapshot time.
    #[tokio::test]
    async fn a_review_phase_cannot_fan_out() {
        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "review", 2);
        let error = resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect_err("`review` at fan_out 2 is refused");
        assert_eq!(
            error,
            ResolveError::ReviewFanOut {
                phase: "review".to_owned(),
                fan_out: 2,
            }
        );
        assert_eq!(
            error.to_string(),
            "phase `review` is a review with fan_out 2; a review cannot fan out (plan D64)"
        );

        let store = store_fanned(ids::GRAPH_HTUI_FEAT, "review", 1);
        resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect("`review` at fan_out 1 is the ordinary case");
    }

    /// The demo fixture with `FEAT`'s phases bound as `bind` names them: `(phase, persona id)`.
    fn store_bound(bind: &[(&str, PersonaId)]) -> MemStore {
        store_with(|data| {
            for phase in &mut data.phases {
                if phase.graph_id != ids::GRAPH_HTUI_FEAT {
                    continue;
                }
                if let Some((_, persona)) = bind.iter().find(|(name, _)| *name == phase.name) {
                    phase.persona_id = Some(*persona);
                }
            }
        })
    }

    /// The fixture's persona row `id`.
    fn persona_row(id: PersonaId) -> htui_core::model::Persona {
        demo_data()
            .personas
            .into_iter()
            .find(|row| row.id == id)
            .expect("the fixture seeds both personas")
    }

    /// MOD-26 D9 (B-18): `StartRun` freezes each bound persona once, sorted by name bytes, with
    /// `SnapshotPersona::freeze`'s digest, and each bound phase names its persona.
    #[tokio::test]
    async fn start_run_freezes_each_bound_persona_once() {
        let store = store_bound(&[
            ("prd", ids::PERSONA_REVIEWER),
            ("plan", ids::PERSONA_ARCHITECT),
            ("review", ids::PERSONA_REVIEWER),
        ]);
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the bound feature graph resolves")
            .snapshot;

        let expected: Vec<SnapshotPersona> = [ids::PERSONA_ARCHITECT, ids::PERSONA_REVIEWER]
            .into_iter()
            .map(|id| SnapshotPersona::freeze(&persona_row(id)).expect("a fixture persona freezes"))
            .collect();
        assert_eq!(
            snapshot.personas, expected,
            "one frozen persona per name, sorted by name bytes"
        );
        assert_eq!(
            snapshot
                .phases
                .iter()
                .map(|phase| (phase.name.as_str(), phase.persona.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("prd", Some("reviewer")),
                ("plan", Some("architect")),
                ("implement", None),
                ("review", Some("reviewer")),
            ],
            "each bound phase names its persona; the unbound one names none"
        );
    }

    /// MOD-26 D9, I-7: a persona-less graph keeps [`FEATURE_TOPOLOGY_V2`]; binding one phase moves
    /// it.
    #[tokio::test]
    async fn only_a_bound_phase_moves_the_topology() {
        let store = MemStore::demo();
        let bare = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves");
        assert_eq!(
            bare.snapshot.topology, FEATURE_TOPOLOGY_V2,
            "{TOPOLOGY_MOVED}"
        );
        assert!(
            bare.snapshot.personas.is_empty(),
            "no bound phase, no frozen persona"
        );

        let store = store_bound(&[("implement", ids::PERSONA_REVIEWER)]);
        let bound = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the bound feature graph resolves");
        assert_ne!(
            bound.snapshot.topology, FEATURE_TOPOLOGY_V2,
            "a bound phase's digest names its persona (MOD-26 D9)"
        );
    }

    /// MOD-26 B-18: a `persona_id` with no row behind it is refused at resolution rather than
    /// frozen un-narrowed.
    #[tokio::test]
    async fn an_unbacked_persona_id_is_refused_at_resolution() {
        let store = MemStore::demo();
        let id = PersonaId::new();
        let source = TestSource {
            unbacked: Some(id),
            ..TestSource::claude(&store)
        };
        let error = resolve_feat(&store, &source)
            .await
            .expect_err("an unbacked persona is refused");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(references_no_row(
                "step_graph_phase.persona_id",
                id,
                "persona",
            )))
        );
    }

    // ---- MOD-163 M2 T2: the run-start memory freeze ----

    /// Writes one active memory entry of `kind` into `project` through `WriteStore`, as the M3 pane
    /// would.
    async fn remember(
        store: &MemStore,
        project: ProjectId,
        kind: htui_core::model::MemoryKind,
        subject: &str,
        body: &str,
    ) -> htui_core::model::MemoryEntry {
        let scrubber = htui_core::scrub::MinimalScrubber::new([]);
        store
            .create_memory_entry(
                project,
                htui_core::model::NewMemoryEntry {
                    id: htui_core::model::MemoryEntryId::new(),
                    kind,
                    subject: htui_core::model::MemorySubject::new(subject, &scrubber)
                        .expect("a plain subject"),
                    body: htui_core::model::MemoryBody::new(body, &scrubber).expect("a plain body"),
                    created_by: ids::USER,
                    box_id: None,
                },
            )
            .await
            .expect("the store takes the entry")
    }

    /// MOD-163 D11, D12 (maintainer P-1): `resolve` freezes the project's `Active` entries, under
    /// the cap and with their text, in `select`'s order; a retired entry stays out, `total` counts
    /// only the active ones, and the topology is the memory-less graph's (no `V` bump).
    #[tokio::test]
    async fn resolve_freezes_the_projects_active_memory_with_its_text() {
        use htui_core::model::{MemoryKind, MemoryState, SnapshotMemoryEntry};
        use htui_core::prompt::{MEMORY_TOKEN_CAP, TokenEstimator, select_memory};

        let store = MemStore::demo();
        let first = remember(
            &store,
            ids::PROJECT_HTUI,
            MemoryKind::Gotcha,
            "Build cache",
            "Clear target/ when the sqlx offline data goes stale.",
        )
        .await;
        let second = remember(
            &store,
            ids::PROJECT_HTUI,
            MemoryKind::Decision,
            "Errors",
            "Library crates use thiserror; binaries use anyhow.",
        )
        .await;
        let retired = remember(
            &store,
            ids::PROJECT_HTUI,
            MemoryKind::Fact,
            "Old fact",
            "This one was retired before the run.",
        )
        .await;
        store
            .retire_memory_entry(retired.id, retired.version, ids::USER, None)
            .await
            .expect("the store retires the entry");

        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        let memory = snapshot
            .memory
            .clone()
            .expect("a project with active entries freezes them");
        assert_eq!(memory.estimator, "chars-v2");
        assert_eq!(memory.total, 2, "only the active entries count");

        let live =
            ReadStore::memory_entries(&store, ids::PROJECT_HTUI, Some(vec![MemoryState::Active]))
                .await
                .expect("MemStore never fails a read")
                .expect("MemStore holds every entry");
        let expected = select_memory(&live, MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT)
            .expect("two active entries");
        assert_eq!(
            memory, expected,
            "the freeze is `select` over the active entries"
        );
        let mut frozen: Vec<_> = memory.entries.iter().map(|entry| entry.id).collect();
        frozen.sort();
        let mut written = vec![first.id, second.id];
        written.sort();
        assert_eq!(
            frozen, written,
            "both active entries, never the retired one"
        );
        for entry in [&first, &second] {
            let snap = memory
                .entries
                .iter()
                .find(|snap| snap.id == entry.id)
                .expect("frozen");
            assert_eq!(
                snap,
                &SnapshotMemoryEntry::freeze(entry, htui_core::model::MemorySource::Person),
                "a copy of the text, the version and the person source"
            );
        }
        assert_eq!(
            snapshot.topology, FEATURE_TOPOLOGY_V2,
            "memory is outside the topology digest: {TOPOLOGY_MOVED}"
        );
    }

    /// MOD-163 D11: a project with no active entry freezes `None`, so the stored snapshot carries
    /// no `memory` key.
    #[tokio::test]
    async fn a_project_without_active_memory_freezes_none() {
        let store = MemStore::demo();
        let retired = remember(
            &store,
            ids::PROJECT_HTUI,
            htui_core::model::MemoryKind::Fact,
            "Gone",
            "Retired before the run.",
        )
        .await;
        store
            .retire_memory_entry(retired.id, retired.version, ids::USER, None)
            .await
            .expect("the store retires the entry");
        // Another project's entry is not this project's memory.
        remember(
            &store,
            ids::PROJECT_AGY,
            htui_core::model::MemoryKind::Fact,
            "Elsewhere",
            "Another project's note.",
        )
        .await;
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        assert_eq!(snapshot.memory, None);
        let json = serde_json::to_value(&snapshot).expect("a snapshot serialises");
        assert!(json.get("memory").is_none(), "{json}");
    }

    /// MOD-163 maintainer decision P-1: the freeze runs in [`resolve`], on a resolved graph only.
    /// `resolve_or_refused` — what a v1 run's resume calls, keeping only the live phases — reads
    /// no memory: its snapshot is memory-less even when the project has active entries.
    #[tokio::test]
    async fn resolve_or_refused_reads_no_memory_for_a_v1_resume() {
        let store = MemStore::demo();
        remember(
            &store,
            ids::PROJECT_HTUI,
            htui_core::model::MemoryKind::Fact,
            "Present",
            "An active entry the resume must not read.",
        )
        .await;
        let item = feat_1(&store).await;
        let resolution = super::resolve_or_refused(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("the seeded feature graph resolves");
        let super::Resolution::Resolved(live) = resolution else {
            panic!("the seeded feature graph is not refused");
        };
        assert_eq!(
            live.snapshot.memory, None,
            "resolve_or_refused never freezes memory (P-1)"
        );
        assert!(
            resolve_feat(&store, &TestSource::claude(&store))
                .await
                .expect("the seeded feature graph resolves")
                .snapshot
                .memory
                .is_some(),
            "resolve itself does"
        );
    }

    // ---- MOD-138 T4: resolve builds snapshot v2 ----

    /// `FEAT-1` re-pointed by `edit`, the rest of the demo fixture as it is.
    fn store_with_feat_1(edit: impl Fn(&mut Item)) -> MemStore {
        store_with(|data| {
            for item in &mut data.items {
                if item.id == ids::HTUI_FEAT_1 {
                    edit(item);
                }
            }
        })
    }

    /// The `feature` graph's phase called `name` in the demo fixture.
    fn feature_phase(data: &DemoData, name: &str) -> PhaseId {
        data.phases
            .iter()
            .find(|phase| phase.graph_id == ids::GRAPH_HTUI_FEAT && phase.name == name)
            .map(|phase| phase.id)
            .expect("the seeded feature graph has the phase")
    }

    /// The seeded `feature` graph, rewritten through `replace_graph` after `edit`.
    async fn replace_feature(store: &MemStore, edit: impl FnOnce(&mut GraphBody)) {
        let mut body = store
            .graph_body(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read")
            .expect("the seeded feature graph is there");
        edit(&mut body);
        let outcome = store
            .replace_graph(ids::GRAPH_HTUI_FEAT, body.graph.revision, body)
            .await
            .expect("the store takes the edited graph");
        assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
    }

    /// MOD-138 D10, D1 (plan acceptance): every seeded kind's default graph resolves to a v2
    /// snapshot whose edges are its default edges, carrying the graph's scope and revision and
    /// the kind-default source.
    #[tokio::test]
    async fn every_seeded_kind_resolves_to_its_default_edges() {
        let kinds: Vec<_> = demo_data()
            .kinds
            .into_iter()
            .filter(|kind| kind.project_id == ids::PROJECT_HTUI)
            .collect();
        assert_eq!(
            kinds.len(),
            htui_core::seed::KINDS.len(),
            "one per seed kind"
        );
        for kind in kinds {
            let store = store_with_feat_1(|item| {
                item.kind_id = kind.id;
                item.step_graph_id = None;
            });
            let snapshot = resolve_feat(&store, &TestSource::claude(&store))
                .await
                .unwrap_or_else(|err| panic!("kind `{}` resolves: {err}", kind.name))
                .snapshot;
            assert_eq!(snapshot.v, GraphSnapshot::V, "{}", kind.name);
            assert_eq!(snapshot.graph.id, kind.default_graph_id, "{}", kind.name);
            assert_eq!(
                snapshot.edges,
                default_snapshot_edges(&snapshot.phases),
                "kind `{}` freezes its default edges",
                kind.name
            );
            assert_eq!(
                snapshot.source,
                Some(SnapshotSource {
                    how: SourceHow::KindDefault
                }),
                "{}",
                kind.name
            );
            assert_eq!(
                snapshot.graph.scope,
                Some(GraphScope::Project(ids::PROJECT_HTUI)),
                "{}",
                kind.name
            );
            assert_eq!(snapshot.graph.revision, Some(0), "{}", kind.name);
            assert!(
                snapshot
                    .phases
                    .iter()
                    .all(|phase| phase.role == NodeRole::Step && !phase.palette),
                "kind `{}`: every seeded node is a plain step",
                kind.name
            );
            assert_eq!(
                snapshot.topology,
                topology_v2(&snapshot.phases, &snapshot.edges).expect("the snapshot serialises"),
                "{}",
                kind.name
            );
        }
    }

    /// MOD-138 D13, D1: the bounds are frozen from `app_setting` over their code defaults, and
    /// the source says whether the item pinned its graph. Neither is in the digest (D3).
    #[tokio::test]
    async fn a_snapshot_freezes_the_bounds_and_the_source() {
        let store = MemStore::demo();
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        let bounds = |snapshot: &GraphSnapshot| {
            (
                snapshot.settings.max_parallel_sessions,
                snapshot.settings.max_plan_revisions,
                snapshot.settings.max_child_runs,
                snapshot.settings.runtime_changes,
            )
        };
        assert_eq!(
            bounds(&snapshot),
            (4, 3, 8, RuntimeChanges::Ask),
            "defaults"
        );
        assert_eq!(
            snapshot.source,
            Some(SnapshotSource {
                how: SourceHow::KindDefault
            })
        );

        let app: BTreeMap<String, Value> = [
            ("max_parallel_sessions".to_owned(), json!(7)),
            ("max_plan_revisions".to_owned(), json!(2)),
            ("max_child_runs".to_owned(), json!(3)),
            ("runtime_changes".to_owned(), json!("off")),
        ]
        .into_iter()
        .collect();
        let item = feat_1(&store).await;
        let set = resolve(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &app,
            None,
            ids::BOX,
        )
        .await
        .expect("the seeded feature graph resolves")
        .snapshot;
        assert_eq!(bounds(&set), (7, 2, 3, RuntimeChanges::Off), "frozen");
        assert_eq!(set.topology, snapshot.topology, "the bounds are not hashed");

        let pinned = store_with_feat_1(|item| item.step_graph_id = Some(ids::GRAPH_HTUI_FEAT));
        let pin = resolve_feat(&pinned, &TestSource::claude(&pinned))
            .await
            .expect("the pinned graph resolves")
            .snapshot;
        assert_eq!(
            pin.source,
            Some(SnapshotSource {
                how: SourceHow::Pin
            })
        );
        assert_eq!(pin.topology, snapshot.topology, "the source is not hashed");
    }

    /// Every key [`run_bounds`] reads, each set to a value no code default holds.
    /// `max_agents_per_run` stays at or above the four agents `feature` plans, so it resolves.
    fn every_bound_set() -> BTreeMap<String, Value> {
        [
            ("max_fan_out".to_owned(), json!(2)),
            ("max_agents_per_run".to_owned(), json!(5)),
            ("max_parallel_sessions".to_owned(), json!(7)),
            ("max_plan_revisions".to_owned(), json!(1)),
            ("max_child_runs".to_owned(), json!(3)),
            ("runtime_changes".to_owned(), json!("off")),
        ]
        .into_iter()
        .collect()
    }

    /// MOD-141 M2 D4: over no `app_setting` row, every bound is its code default.
    #[test]
    fn run_bounds_are_the_code_defaults_over_no_row() {
        assert_eq!(
            run_bounds(&BTreeMap::new()),
            RunBounds {
                max_fan_out: 4,
                max_agents_per_run: 8,
                max_parallel_sessions: 4,
                max_plan_revisions: 3,
                max_child_runs: 8,
                runtime_changes: RuntimeChanges::Ask,
            }
        );
    }

    /// MOD-141 M2 D4: each key answers for itself, and a silent one (zero, negative, a string, a
    /// bool, `null`, too large for `u32`, an unknown word) falls through to its code default.
    #[test]
    fn run_bounds_read_each_key_and_fall_through_on_silence() {
        assert_eq!(
            run_bounds(&every_bound_set()),
            RunBounds {
                max_fan_out: 2,
                max_agents_per_run: 5,
                max_parallel_sessions: 7,
                max_plan_revisions: 1,
                max_child_runs: 3,
                runtime_changes: RuntimeChanges::Off,
            }
        );

        let silent: BTreeMap<String, Value> = [
            ("max_fan_out".to_owned(), json!(0)),
            ("max_agents_per_run".to_owned(), json!("6")),
            ("max_parallel_sessions".to_owned(), json!(-1)),
            ("max_plan_revisions".to_owned(), json!(true)),
            ("max_child_runs".to_owned(), json!(u64::from(u32::MAX) + 1)),
            ("runtime_changes".to_owned(), json!("maybe")),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            run_bounds(&silent),
            run_bounds(&BTreeMap::new()),
            "every silent key reads as its code default"
        );
        let null: BTreeMap<String, Value> = [("runtime_changes".to_owned(), Value::Null)]
            .into_iter()
            .collect();
        assert_eq!(run_bounds(&null).runtime_changes, RuntimeChanges::Ask);
    }

    /// MOD-141 M2 D4: what `resolve` freezes into the snapshot's settings is exactly
    /// [`run_bounds`] over the same `app_setting` map, so the editor's header cannot disagree.
    #[tokio::test]
    async fn resolve_freezes_exactly_run_bounds() {
        let store = MemStore::demo();
        let item = feat_1(&store).await;
        for app in [BTreeMap::new(), every_bound_set()] {
            let settings = resolve(
                &store,
                &TestSource::claude(&store),
                &item,
                RunMode::Manual,
                &app,
                None,
                ids::BOX,
            )
            .await
            .expect("the seeded feature graph resolves")
            .snapshot
            .settings;
            assert_eq!(
                RunBounds {
                    max_fan_out: settings.max_fan_out,
                    max_agents_per_run: settings.max_agents_per_run,
                    max_parallel_sessions: settings.max_parallel_sessions,
                    max_plan_revisions: settings.max_plan_revisions,
                    max_child_runs: settings.max_child_runs,
                    runtime_changes: settings.runtime_changes,
                },
                run_bounds(&app),
                "{app:?}"
            );
        }
    }

    /// MOD-138 D11, MOD-139 M3 D10: a `divide` node is expanded (so the validator checks the
    /// expansion), and a default-parts node admission would take resolves.
    #[tokio::test]
    async fn a_divide_node_is_expanded_and_resolves() {
        let store = MemStore::demo();
        replace_feature(&store, |body| {
            let node = body
                .nodes
                .iter_mut()
                .find(|node| node.phase.name == "implement")
                .expect("the feature graph has `implement`");
            node.phase.split = Split::Divide;
            node.phase.fan_out = 1;
            node.phase.max_parts = Some(2);
            node.parts = vec![PhasePart {
                phase_id: node.phase.id,
                position: 0,
                key: "api".to_owned(),
                title: "The API".to_owned(),
                brief: String::new(),
                owned_paths: vec!["src/api".to_owned()],
                acceptance: None,
                agent_id: None,
                model: None,
                persona_id: None,
            }];
        })
        .await;
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("a default-parts divided node resolves")
            .snapshot;
        let nodes: Vec<(&str, NodeRole)> = snapshot
            .phases
            .iter()
            .map(|phase| (phase.name.as_str(), phase.role))
            .collect();
        assert_eq!(nodes[2], ("implement:plan", NodeRole::Planner));
        assert_eq!(nodes[3], ("implement", NodeRole::Integrate));
    }

    /// The seeded `feature` graph with `implement` divided into one `api` part, its planner agent
    /// `planner`.
    async fn divide_feature(store: &MemStore, planner: Option<AgentId>) {
        replace_feature(store, |body| {
            let node = body
                .nodes
                .iter_mut()
                .find(|node| node.phase.name == "implement")
                .expect("the feature graph has `implement`");
            node.phase.split = Split::Divide;
            node.phase.fan_out = 1;
            node.phase.planner_agent_id = planner;
            node.parts = vec![PhasePart {
                phase_id: node.phase.id,
                position: 0,
                key: "api".to_owned(),
                title: "The API".to_owned(),
                brief: String::new(),
                owned_paths: vec!["src/api".to_owned()],
                acceptance: None,
                agent_id: None,
                model: None,
                persona_id: None,
            }];
        })
        .await;
    }

    /// MOD-139 M2 D17 (PRD Q1): a configured planner agent is refused first, before every other
    /// divided-node refusal, named by the divided node.
    #[tokio::test]
    async fn a_planner_agent_is_refused_before_the_divide() {
        let store = MemStore::demo();
        divide_feature(&store, Some(ids::AGENT_CLAUDE)).await;
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("a planner agent cannot run yet");
        assert_eq!(
            error,
            ResolveError::NotYetRunnable {
                node: "implement".to_owned(),
                reason: NotRunnable::PlannerAgent,
            }
        );
        assert!(error.to_string().contains("MOD-140"), "{error}");
    }

    /// The `implement` node of the seeded `feature` graph's body.
    fn implement_node(body: &mut GraphBody) -> &mut NodeBody {
        body.nodes
            .iter_mut()
            .find(|node| node.phase.name == "implement")
            .expect("the feature graph has `implement`")
    }

    /// MOD-139 M3 D10 (blueprint P-11): resolve refuses a configured planner agent first, then
    /// what admission would — no default parts (`NoParts`), a `plan_scope` past the parts
    /// (`PlanScope`), a palette node as ever, then `divide::admission_refusal`'s answer as
    /// `DivideRefused`, whose sentence is the `RunFailure`'s.
    #[tokio::test]
    async fn resolve_refuses_a_planner_agent_first_then_what_admission_would() {
        let refused = |reason| ResolveError::NotYetRunnable {
            node: "implement".to_owned(),
            reason,
        };

        let agent = MemStore::demo();
        divide_feature(&agent, Some(ids::AGENT_CLAUDE)).await;
        replace_feature(&agent, |body| implement_node(body).parts.clear()).await;
        assert_eq!(
            resolve_feat(&agent, &TestSource::claude(&agent))
                .await
                .expect_err("a planner agent beats no parts"),
            refused(NotRunnable::PlannerAgent)
        );

        let empty = MemStore::demo();
        divide_feature(&empty, None).await;
        replace_feature(&empty, |body| implement_node(body).parts.clear()).await;
        assert_eq!(
            resolve_feat(&empty, &TestSource::claude(&empty))
                .await
                .expect_err("no default parts needs the planner"),
            refused(NotRunnable::NoParts)
        );

        let scope = MemStore::demo();
        divide_feature(&scope, None).await;
        replace_feature(&scope, |body| {
            implement_node(body).phase.plan_scope = Some(PlanScope::Successors);
        })
        .await;
        assert_eq!(
            resolve_feat(&scope, &TestSource::claude(&scope))
                .await
                .expect_err("a planner planning past its parts"),
            refused(NotRunnable::PlanScope)
        );

        let palette = store_with(|data| {
            let prd = feature_phase(data, "prd");
            for phase in &mut data.phases {
                if phase.id == prd {
                    phase.palette = true;
                }
            }
            data.edges.retain(|edge| edge.from != prd && edge.to != prd);
        });
        divide_feature(&palette, None).await;
        assert_eq!(
            resolve_feat(&palette, &TestSource::claude(&palette))
                .await
                .expect_err("a palette node is still refused"),
            ResolveError::NotYetRunnable {
                node: "prd".to_owned(),
                reason: NotRunnable::Palette,
            }
        );

        let local = MemStore::demo();
        divide_feature(&local, None).await;
        replace_feature(&local, |body| {
            let node = implement_node(body);
            node.phase.isolation = Some(Isolation::Local);
            let mut ui = node.parts[0].clone();
            ui.position = 1;
            ui.key = "ui".to_owned();
            ui.title = "The UI".to_owned();
            ui.owned_paths = vec!["src/ui".to_owned()];
            node.parts.push(ui);
        })
        .await;
        let failure = crate::status::RunFailure::LocalCannotDivide {
            node: "implement".to_owned(),
            parts: 2,
        };
        let error = resolve_feat(&local, &TestSource::claude(&local))
            .await
            .expect_err("`local` runs one tree");
        assert_eq!(
            error,
            ResolveError::DivideRefused {
                node: "implement".to_owned(),
                reason: failure.clone(),
            }
        );
        assert_eq!(error.to_string(), failure.to_string());
    }

    /// MOD-139 M3 D10: the two new refusals name the milestone that lifts them.
    #[test]
    fn not_runnable_sentences_name_their_milestone() {
        assert_eq!(
            NotRunnable::NoParts.to_string(),
            "a divided node with no default parts needs its planner, which runs from MOD-140 on"
        );
        assert_eq!(
            NotRunnable::PlanScope.to_string(),
            "a divided node that plans past its own parts (`plan_scope` `successors` or `graph`) \
             runs from MOD-140 and MOD-141 on"
        );
    }

    /// MOD-139 M2 D8: a divided node counts its parts, not its planner node (no planner agent),
    /// and its integrate node once. The old count, `fan_out` per node, said 4.
    #[test]
    fn fan_out_caps_count_parts_not_the_planner() {
        use htui_core::model::{SnapshotPart, SnapshotPhase, SnapshotSettings};
        let (template, _) = crate::walk::tests::fixture();
        let template = template.phases[0].clone();
        let node = |name: &str, position: i32, role: NodeRole| SnapshotPhase {
            name: name.to_owned(),
            position,
            role,
            fan_out: 1,
            judge: None,
            ..template.clone()
        };
        let part = |key: &str| SnapshotPart {
            position: 0,
            key: key.to_owned(),
            title: key.to_owned(),
            brief: String::new(),
            owned_paths: Vec::new(),
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        };
        let divide = SnapshotDivide {
            default_parts: vec![part("api"), part("ui"), part("db")],
            ..SnapshotDivide::default()
        };
        let mut planner = node("implement:plan", 1, NodeRole::Planner);
        planner.divide = Some(divide.clone());
        let mut integrate = node("implement", 2, NodeRole::Integrate);
        integrate.divide = Some(divide);
        let phases = [
            node("prd", 0, NodeRole::Step),
            planner,
            integrate,
            node("review", 3, NodeRole::Step),
        ];
        let settings = |max_agents_per_run| SnapshotSettings {
            max_agents_per_run,
            max_fan_out: 8,
            ..template_settings()
        };
        assert_eq!(super::check_fan_out_caps(&phases, &settings(6)), Ok(()));
        assert_eq!(
            super::check_fan_out_caps(&phases, &settings(5)),
            Err(ResolveError::AgentCap { planned: 6, max: 5 })
        );
    }

    /// The fixture snapshot's settings.
    fn template_settings() -> htui_core::model::SnapshotSettings {
        crate::walk::tests::fixture().0.settings
    }

    /// The seeded `feature` resolve's phases and edges, for the pure tests below.
    async fn feature_phases_and_edges() -> (Vec<super::SnapshotPhase>, Vec<SnapshotEdge>) {
        let store = MemStore::demo();
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the seeded feature graph resolves")
            .snapshot;
        (snapshot.phases, snapshot.edges)
    }

    /// MOD-138 D11: `implement` becomes `implement:plan` (planner) then `implement` (integrate),
    /// later nodes shift by one, edges into it move to the planner and a synthetic `next` joins
    /// the two.
    #[tokio::test]
    async fn expand_divide_splits_reroutes_and_shifts() {
        let (mut phases, edges) = feature_phases_and_edges().await;
        let block = SnapshotDivide {
            max_parts: Some(2),
            ..SnapshotDivide::default()
        };
        phases[2].divide = Some(block.clone());
        phases[2].plan_scope = Some(PlanScope::Parts);
        let (phases, edges) = expand_divide("feature", phases, edges).expect("no key is taken");

        let nodes: Vec<(i32, &str, NodeRole)> = phases
            .iter()
            .map(|phase| (phase.position, phase.name.as_str(), phase.role))
            .collect();
        assert_eq!(
            nodes,
            [
                (0, "prd", NodeRole::Step),
                (1, "plan", NodeRole::Step),
                (2, "implement:plan", NodeRole::Planner),
                (3, "implement", NodeRole::Integrate),
                (4, "review", NodeRole::Step),
            ]
        );
        assert_eq!(
            phases[2].plan_scope,
            Some(PlanScope::Parts),
            "the planner's"
        );
        assert_eq!(
            phases[3].plan_scope, None,
            "the integrate node plans nothing"
        );
        assert_eq!(phases[2].divide.as_ref(), Some(&block));
        assert_eq!(phases[3].divide.as_ref(), Some(&block));

        let shape: Vec<(&str, &str, EdgeKind)> = edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str(), edge.kind))
            .collect();
        assert_eq!(
            shape,
            [
                ("prd", "plan", EdgeKind::Next),
                ("plan", "implement:plan", EdgeKind::Next),
                ("implement:plan", "implement", EdgeKind::Next),
                ("implement", "review", EdgeKind::Next),
                ("review", "implement:plan", EdgeKind::ChangesRequested),
            ],
            "the review loop is re-pointed at the planner"
        );
    }

    /// MOD-138 D11: a node already keyed `<name>:plan` refuses the expansion.
    #[tokio::test]
    async fn expand_divide_refuses_a_taken_plan_key() {
        let (mut phases, _) = feature_phases_and_edges().await;
        phases[1].name = "implement:plan".to_owned();
        phases[2].divide = Some(SnapshotDivide::default());
        let error = expand_divide("feature", phases, Vec::new()).expect_err("the key is taken");
        assert_eq!(
            error,
            ResolveError::InvalidGraph {
                graph: "feature".to_owned(),
                reason: "phase name `implement:plan` is taken by the expansion of divide phase \
                         `implement`"
                    .to_owned(),
            }
        );
    }

    /// MOD-138 D6(b): a palette node is placed by a planner, which runs from MOD-140 on.
    #[tokio::test]
    async fn a_palette_node_is_refused() {
        let store = store_with(|data| {
            let prd = feature_phase(data, "prd");
            for phase in &mut data.phases {
                if phase.id == prd {
                    phase.palette = true;
                }
            }
            // A palette node has no edges (MOD-137 rule e).
            data.edges.retain(|edge| edge.from != prd && edge.to != prd);
        });
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("a palette node cannot run yet");
        assert_eq!(
            error,
            ResolveError::NotYetRunnable {
                node: "prd".to_owned(),
                reason: NotRunnable::Palette,
            }
        );
    }

    /// MOD-138 blueprint A6: `resolve_or_refused` hands D6's refusal back beside the live phases
    /// it built first, in position order, which a v1 run's resume compares on its v1 digest.
    #[tokio::test]
    async fn a_refusal_carries_the_live_phases() {
        let store = store_with(|data| {
            let prd = feature_phase(data, "prd");
            for phase in &mut data.phases {
                if phase.id == prd {
                    phase.palette = true;
                }
            }
            data.edges.retain(|edge| edge.from != prd && edge.to != prd);
        });
        let item = feat_1(&store).await;
        let resolution = super::resolve_or_refused(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("only D6 refuses the graph");
        let super::Resolution::Refused { phases, refusal } = resolution else {
            panic!("a palette node is refused");
        };
        assert_eq!(
            refusal,
            ResolveError::NotYetRunnable {
                node: "prd".to_owned(),
                reason: NotRunnable::Palette,
            }
        );
        let names: Vec<(i32, &str, bool)> = phases
            .iter()
            .map(|phase| (phase.position, phase.name.as_str(), phase.palette))
            .collect();
        assert_eq!(
            names,
            [
                (0, "prd", true),
                (1, "plan", false),
                (2, "implement", false),
                (3, "review", false),
            ]
        );
    }

    /// MOD-138 M2 D10 (A8, replacing D6(c)'s `non_default_edges_are_refused`): a graph with
    /// authored edges resolves, and its snapshot carries them.
    #[tokio::test]
    async fn a_graph_with_authored_edges_resolves() {
        let store = MemStore::demo();
        replace_feature(&store, |body| {
            let id = |name: &str| {
                body.nodes
                    .iter()
                    .find(|node| node.phase.name == name)
                    .map(|node| node.phase.id)
                    .expect("the feature graph has the phase")
            };
            let (review, plan) = (id("review"), id("plan"));
            body.edges.push(StepGraphEdge {
                id: EdgeId::new(),
                graph_id: ids::GRAPH_HTUI_FEAT,
                from: review,
                to: plan,
                kind: EdgeKind::AsksYou,
                predicate: None,
                loop_limit: None,
            });
        })
        .await;
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("the readiness walker runs authored edges")
            .snapshot;
        assert!(
            snapshot
                .edges
                .iter()
                .any(|edge| (edge.from.as_str(), edge.to.as_str(), edge.kind)
                    == ("review", "plan", EdgeKind::AsksYou)),
            "{:?}",
            snapshot.edges
        );
    }

    /// MOD-138 M2 D10: `ReviewFanOut` is widened from the name to the rule. A node that reads its
    /// verdict (an outgoing `changes_requested` edge) may not fan out, whatever it is called.
    #[tokio::test]
    async fn a_verdict_reading_node_may_not_fan_out() {
        let store = store_with(|data| {
            let review = feature_phase(data, "review");
            for phase in &mut data.phases {
                if phase.id == review {
                    phase.name = "audit".to_owned();
                    phase.fan_out = 2;
                    phase.split = Split::for_fan_out(2);
                }
            }
        });
        store.set_project_settings(
            ids::PROJECT_HTUI,
            json!({ "judge_agent_id": ids::AGENT_AGY }),
        );
        let error = resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
            .await
            .expect_err("`audit` reads its verdict, so it cannot fan out");
        assert_eq!(
            error,
            ResolveError::ReviewFanOut {
                phase: "audit".to_owned(),
                fan_out: 2,
            }
        );
    }

    /// Blueprint A7: group routing never runs the `asks_you` stop or the `fails` loop, so a
    /// fanned-out node with either edge is refused as not yet runnable (M3 or MOD-139 lifts it).
    #[tokio::test]
    async fn an_asks_you_or_fails_node_may_not_fan_out() {
        for kind in [EdgeKind::AsksYou, EdgeKind::Fails] {
            let store = store_fanned(ids::GRAPH_HTUI_FEAT, "implement", 2);
            replace_feature(&store, |body| {
                let id = |name: &str| {
                    body.nodes
                        .iter()
                        .find(|node| node.phase.name == name)
                        .map(|node| node.phase.id)
                        .expect("the feature graph has the phase")
                };
                let (implement, plan) = (id("implement"), id("plan"));
                body.edges.push(StepGraphEdge {
                    id: EdgeId::new(),
                    graph_id: ids::GRAPH_HTUI_FEAT,
                    from: implement,
                    to: plan,
                    kind,
                    predicate: None,
                    loop_limit: kind.needs_loop_limit().then_some(1),
                });
            })
            .await;
            let error = resolve_item(&store, ids::HTUI_FEAT_1, &BTreeMap::new())
                .await
                .expect_err("a fanned-out node's `asks_you` or `fails` edge cannot run yet");
            assert_eq!(
                error,
                ResolveError::NotYetRunnable {
                    node: "implement".to_owned(),
                    reason: NotRunnable::FanOutEdge,
                },
                "{kind}"
            );
            assert!(error.to_string().contains("MOD-138 M3"), "{error}");
        }
    }

    /// The feature graph with `asks_you` edges `(from, to)` added (no `loop_limit`), resolved
    /// for FEAT-1.
    async fn resolve_with_asks_you(
        edges: &[(&str, &str)],
    ) -> std::result::Result<Resolved, ResolveError> {
        let store = MemStore::demo();
        replace_feature(&store, |body| with_asks_you(body, edges)).await;
        resolve_feat(&store, &TestSource::claude(&store)).await
    }

    /// `asks_you` edges `(from, to)` (no `loop_limit`) on the seeded feature graph's edges.
    fn with_asks_you(body: &mut GraphBody, edges: &[(&str, &str)]) {
        let id = |name: &str| {
            body.nodes
                .iter()
                .find(|node| node.phase.name == name)
                .map(|node| node.phase.id)
                .expect("the feature graph has the phase")
        };
        let ends: Vec<(PhaseId, PhaseId)> =
            edges.iter().map(|(from, to)| (id(from), id(to))).collect();
        for (from, to) in ends {
            body.edges.push(StepGraphEdge {
                id: EdgeId::new(),
                graph_id: ids::GRAPH_HTUI_FEAT,
                from,
                to,
                kind: EdgeKind::AsksYou,
                predicate: None,
                loop_limit: None,
            });
        }
    }

    /// The two mixed shapes of MOD-138 M3 D8 (W5) on the feature graph, with the node each names:
    /// forward plus back, and back plus back.
    const MIXED_ASKS_YOU: [(&str, [(&str, &str); 2]); 2] = [
        ("plan", [("plan", "review"), ("plan", "prd")]),
        ("review", [("review", "plan"), ("review", "prd")]),
    ];

    /// MOD-138 M3 D8 (W5, Q-3; was R1's resolve-only `MixedAsksYou`): a node with two or more
    /// outgoing `asks_you` edges, one of them back, is refused at save for good, naming the node.
    /// One answer cannot take every edge: the back edge loops and retires the node. Nothing is
    /// written.
    #[tokio::test]
    async fn a_node_with_a_back_asks_you_beside_another_asks_you_is_refused() {
        for (node, edges) in MIXED_ASKS_YOU {
            let store = MemStore::demo();
            let before = store
                .graph_body(ids::GRAPH_HTUI_FEAT)
                .await
                .expect("MemStore never fails a read")
                .expect("the seeded feature graph is there");
            let mut body = before.clone();
            with_asks_you(&mut body, &edges);
            let outcome = store
                .replace_graph(ids::GRAPH_HTUI_FEAT, before.graph.revision, body)
                .await;
            let sentence = back_asks_you_beside_another_asks_you(node, ids::GRAPH_HTUI_FEAT);
            assert!(
                matches!(&outcome, Err(StoreError::Constraint(said)) if *said == sentence),
                "{edges:?}: {outcome:?}"
            );
            let after = store
                .graph_body(ids::GRAPH_HTUI_FEAT)
                .await
                .expect("MemStore never fails a read")
                .expect("the seeded feature graph is there");
            assert_eq!(after, before, "{edges:?}: nothing written");
        }
    }

    /// MOD-138 M3 D8: a live graph holding the mixed shape (planted under `replace_graph`) is
    /// refused at resolve as an invalid graph with the store's sentence, no longer as a graph
    /// that cannot run yet.
    #[tokio::test]
    async fn a_live_mixed_asks_you_graph_is_refused_as_invalid() {
        for (node, edges) in MIXED_ASKS_YOU {
            let store = store_with(|data| {
                for (from, to) in edges {
                    let (from, to) = (feature_phase(data, from), feature_phase(data, to));
                    data.edges.push(StepGraphEdge {
                        id: EdgeId::new(),
                        graph_id: ids::GRAPH_HTUI_FEAT,
                        from,
                        to,
                        kind: EdgeKind::AsksYou,
                        predicate: None,
                        loop_limit: None,
                    });
                }
            });
            let error = resolve_feat(&store, &TestSource::claude(&store))
                .await
                .expect_err("a mixed `asks_you` node is refused");
            assert_eq!(
                error,
                ResolveError::InvalidGraph {
                    graph: "feature".to_owned(),
                    reason: back_asks_you_beside_another_asks_you(node, ids::GRAPH_HTUI_FEAT),
                },
                "{edges:?}"
            );
        }
    }

    /// MOD-138 M3 D8 (V8): W5 lives in `walk_refusal`, so resolve's post-expansion check refuses
    /// a divide-expanded graph whose integrate node keeps the mixed shape.
    #[tokio::test]
    async fn a_divide_expanded_mixed_asks_you_is_refused_after_expansion() {
        let (mut phases, mut edges) = feature_phases_and_edges().await;
        phases[2].divide = Some(SnapshotDivide {
            max_parts: Some(2),
            ..SnapshotDivide::default()
        });
        for to in ["plan", "review"] {
            edges.push(SnapshotEdge {
                from: "implement".to_owned(),
                to: to.to_owned(),
                kind: EdgeKind::AsksYou,
                predicate: None,
                loop_limit: None,
            });
        }
        let (phases, edges) = expand_divide("feature", phases, edges).expect("no key is taken");
        assert!(
            phases.iter().any(|phase| phase.name == "implement:plan"),
            "expanded"
        );
        assert_eq!(
            snapshot_walk_refusal(ids::GRAPH_HTUI_FEAT, &phases, &edges),
            Some(back_asks_you_beside_another_asks_you(
                "implement",
                ids::GRAPH_HTUI_FEAT
            ))
        );
    }

    // ---- MOD-138 M3 D7: `local` on two nodes that can be live together ----

    /// The feature graph as a diamond, `prd → {plan, implement} → review` over `next` edges (its
    /// review loop kept), with `isolation` set per named node.
    fn diamond(body: &mut GraphBody, isolation: &[(&str, Isolation)]) {
        let id = |name: &str| {
            body.nodes
                .iter()
                .find(|node| node.phase.name == name)
                .map(|node| node.phase.id)
                .expect("the feature graph has the phase")
        };
        let next: Vec<(PhaseId, PhaseId)> = [
            ("prd", "plan"),
            ("prd", "implement"),
            ("plan", "review"),
            ("implement", "review"),
        ]
        .iter()
        .map(|(from, to)| (id(from), id(to)))
        .collect();
        body.edges.retain(|edge| edge.kind != EdgeKind::Next);
        for (from, to) in next {
            body.edges.push(StepGraphEdge {
                id: EdgeId::new(),
                graph_id: ids::GRAPH_HTUI_FEAT,
                from,
                to,
                kind: EdgeKind::Next,
                predicate: None,
                loop_limit: None,
            });
        }
        for (name, mode) in isolation {
            for node in &mut body.nodes {
                if node.phase.name == *name {
                    node.phase.isolation = Some(*mode);
                }
            }
        }
    }

    /// The feature graph under `settings`, made a diamond with `isolation`, resolved for FEAT-1.
    async fn resolve_diamond(
        settings: Value,
        isolation: &[(&str, Isolation)],
    ) -> std::result::Result<Resolved, ResolveError> {
        let store = store_with_settings(settings);
        replace_feature(&store, |body| diamond(body, isolation)).await;
        resolve_feat(&store, &TestSource::claude(&store)).await
    }

    /// D7: `local` on a diamond's two arms, which neither reaches the other over forward edges,
    /// is refused naming both nodes (first pair in position order) and the per-phase `worktree`
    /// remedy.
    #[tokio::test]
    async fn local_on_a_diamonds_two_arms_is_refused() {
        let error = resolve_diamond(
            json!({}),
            &[("plan", Isolation::Local), ("implement", Isolation::Local)],
        )
        .await
        .expect_err("two `local` nodes that can be live together");
        assert_eq!(
            error,
            ResolveError::LocalConcurrent {
                first: "plan".to_owned(),
                second: "implement".to_owned(),
            }
        );
        assert_eq!(
            error.to_string(),
            "phases `plan` and `implement` are both `local` and can run at the same time (neither \
             reaches the other over forward edges), and `local` shares one working tree; set \
             `worktree` isolation on one of them (MOD-138 M3 D7)"
        );

        // A `local` project: every node is `local`, and the arms are still the first pair.
        let error = resolve_diamond(json!({"default_isolation": "local"}), &[])
            .await
            .expect_err("a `local` project's diamond");
        assert_eq!(
            error,
            ResolveError::LocalConcurrent {
                first: "plan".to_owned(),
                second: "implement".to_owned(),
            }
        );
    }

    /// D7, blueprint A6: `resolve_or_refused` hands `LocalConcurrent` back beside the live
    /// phases it built, unexpanded and in position order, so a v1 run's resume compares them on
    /// the v1 digest and walks on when they match, as it did before M3 refused a `local` diamond.
    #[tokio::test]
    async fn local_concurrent_carries_the_live_phases() {
        let store = store_with_settings(json!({"default_isolation": "local"}));
        replace_feature(&store, |body| diamond(body, &[])).await;
        let item = feat_1(&store).await;
        let resolution = super::resolve_or_refused(
            &store,
            &TestSource::claude(&store),
            &item,
            RunMode::Manual,
            &BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("D7 refuses the graph once its phases are built");
        let super::Resolution::Refused { phases, refusal } = resolution else {
            panic!("a `local` project's diamond is refused");
        };
        assert_eq!(
            refusal,
            ResolveError::LocalConcurrent {
                first: "plan".to_owned(),
                second: "implement".to_owned(),
            }
        );
        let names: Vec<(i32, &str)> = phases
            .iter()
            .map(|phase| (phase.position, phase.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [(0, "prd"), (1, "plan"), (2, "implement"), (3, "review")]
        );
    }

    /// D7's remedy and its edges: a `local` project resolves its chain, and its diamond once one
    /// arm says `worktree`; a diamond in `worktree`, or in `shared_serialized` on both arms,
    /// resolves; one `local` arm beside a `worktree` one resolves.
    #[tokio::test]
    async fn local_on_a_chain_or_off_the_diamonds_arms_resolves() {
        let store = store_with_settings(json!({"default_isolation": "local"}));
        let snapshot = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect("a `local` chain is one node at a time")
            .snapshot;
        assert!(
            snapshot
                .phases
                .iter()
                .all(|phase| phase.isolation == Isolation::Local),
            "{:?}",
            snapshot.phases
        );

        for (settings, isolation, what) in [
            (
                json!({"default_isolation": "local"}),
                &[("implement", Isolation::Worktree)][..],
                "a `local` project, one arm `worktree`",
            ),
            (json!({}), &[][..], "the default `worktree`"),
            (
                json!({}),
                &[
                    ("plan", Isolation::SharedSerialized),
                    ("implement", Isolation::SharedSerialized),
                ][..],
                "`shared_serialized` on both arms",
            ),
            (
                json!({}),
                &[("plan", Isolation::Local)][..],
                "one `local` arm",
            ),
        ] {
            let snapshot = resolve_diamond(settings, isolation)
                .await
                .unwrap_or_else(|error| panic!("{what} resolves: {error}"))
                .snapshot;
            assert_eq!(
                snapshot
                    .edges
                    .iter()
                    .filter(|edge| edge.kind == EdgeKind::Next)
                    .count(),
                4,
                "{what}: the diamond is frozen"
            );
        }
    }

    /// R1 (E1): the shapes the `asks_you` stop can answer still resolve: two forward `asks_you`
    /// edges out of one node, and a single back one.
    #[tokio::test]
    async fn forward_asks_you_edges_or_a_single_back_one_resolve() {
        for edges in [
            &[("prd", "implement"), ("prd", "review")][..],
            &[("review", "plan")][..],
        ] {
            let snapshot = resolve_with_asks_you(edges)
                .await
                .unwrap_or_else(|error| panic!("{edges:?} resolves: {error}"))
                .snapshot;
            let asks = snapshot
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::AsksYou)
                .count();
            assert_eq!(asks, edges.len(), "{:?}", snapshot.edges);
        }
    }

    /// MOD-138 D9: a live graph the validator refuses (a forward cycle, planted under
    /// `replace_graph`) is refused at resolve with the very sentence the store gives at save.
    #[tokio::test]
    async fn an_invalid_live_graph_is_refused_with_the_validators_sentence() {
        let store = store_with(|data| {
            let (implement, plan) = (
                feature_phase(data, "implement"),
                feature_phase(data, "plan"),
            );
            data.edges.push(StepGraphEdge {
                id: EdgeId::new(),
                graph_id: ids::GRAPH_HTUI_FEAT,
                from: implement,
                to: plan,
                kind: EdgeKind::Next,
                predicate: None,
                loop_limit: None,
            });
        });
        let body = store
            .graph_body(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore never fails a read")
            .expect("the seeded feature graph is there");
        let sentence =
            graph_body_walk_refusal(ids::GRAPH_HTUI_FEAT, &body).expect("the store refuses it");
        assert_eq!(
            sentence,
            forward_edges_form_a_cycle(&["plan", "implement", "plan"], ids::GRAPH_HTUI_FEAT)
        );
        let error = resolve_feat(&store, &TestSource::claude(&store))
            .await
            .expect_err("an invalid graph is refused");
        assert_eq!(
            error,
            ResolveError::InvalidGraph {
                graph: "feature".to_owned(),
                reason: sentence,
            }
        );
    }

    /// MOD-138 D10: an edge naming a phase the read did not return is a graph that moved while
    /// it was being resolved (the `override_graph` precedent).
    #[tokio::test]
    async fn an_edge_outside_the_read_is_refused_as_moved() {
        let store = MemStore::demo();
        let source = TestSource {
            dropped: Some("plan"),
            ..TestSource::claude(&store)
        };
        let error = resolve_feat(&store, &source)
            .await
            .expect_err("a torn read is refused");
        assert_eq!(
            error,
            ResolveError::Store(StoreError::Constraint(
                "graph `feature` moved while it was being resolved; start the run again".to_owned()
            ))
        );
    }

    /// MOD-138 D3: the v2 digest reads the edges in snapshot order whatever order it is handed,
    /// moves with any node or edge field, and does not depend on a predicate's key order (F5).
    #[tokio::test]
    async fn topology_v2_ignores_edge_order() {
        let (phases, edges) = feature_phases_and_edges().await;
        let digest = topology_v2(&phases, &edges).expect("the snapshot serialises");
        let mut reversed = edges.clone();
        reversed.reverse();
        assert_ne!(reversed, edges, "the reversal moves something");
        assert_eq!(topology_v2(&phases, &reversed).expect("serialises"), digest);

        let mut limited = edges.clone();
        let review_loop = limited
            .iter_mut()
            .find(|edge| edge.kind == EdgeKind::ChangesRequested)
            .expect("the seeded loop");
        review_loop.loop_limit = Some(9);
        assert_ne!(
            topology_v2(&phases, &limited).expect("serialises"),
            digest,
            "a loop limit"
        );
        let mut kinded = edges.clone();
        kinded[0].kind = EdgeKind::AsksYou;
        assert_ne!(
            topology_v2(&phases, &kinded).expect("serialises"),
            digest,
            "an edge kind"
        );
        let mut retried = phases.clone();
        retried[0].retry_limit += 1;
        assert_ne!(
            topology_v2(&retried, &edges).expect("serialises"),
            digest,
            "a node field"
        );

        let with_predicate = |pairs: &[(&str, Value)]| {
            let mut map = serde_json::Map::new();
            for (key, value) in pairs {
                map.insert((*key).to_owned(), value.clone());
            }
            let mut edges = edges.clone();
            edges.push(SnapshotEdge {
                from: "plan".to_owned(),
                to: "review".to_owned(),
                kind: EdgeKind::If,
                predicate: Some(EdgePredicate(Value::Object(map))),
                loop_limit: None,
            });
            topology_v2(&phases, &edges).expect("serialises")
        };
        let one = with_predicate(&[("verdict", json!("approve")), ("min", json!(2))]);
        let other = with_predicate(&[("min", json!(2)), ("verdict", json!("approve"))]);
        assert_eq!(one, other, "a predicate's key order is not hashed");
        assert_ne!(one, digest, "a predicate is");
    }
}
