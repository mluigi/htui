//! In-memory store (`MemStore`), blueprint B.8.
//!
//! Every row of `docs/ANA-9.md` §5 the TUI reads lives in one [`std::sync::RwLock`]-guarded map,
//! and every trait method is a single call to the private `read` or `write` helper with a
//! plain, non-async closure that computes and clones owned values. The async bodies contain no
//! `.await` at all, so holding a lock guard across a suspension point is structurally impossible
//! rather than a convention (plan D6).
//!
//! The rules the store enforces are the ones MOD-6's `PgStore` will have to enforce too, which is
//! why they are pinned by `store::conformance` rather than by tests of this type: the per
//! `(project, prefix)` key counter and the absent delete path of §4.1, and the compare-and-set on
//! `version` with its `Diverged { head, ancestor }` answer of §4.2.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};

use serde_json::Value;

use crate::clock::Clock;
#[cfg(feature = "test-support")]
use crate::clock::TestClock;
use crate::model::link::{ProposeLink, WithdrawLink};
use crate::model::{
    Agent, AgentBox, AgentId, AgentSummary, AnswerOutcome, AnswerRefusal, AppUser,
    BOX_PROBE_SPEC_KEY, BatchClose, BatchId, BindingChange, BoundSkill, BoxEdit, BoxId, BoxInfo,
    BoxProbe, BoxProfile, BoxRecord, BoxRow, BoxSettings, BoxTool, CancelRequest, ChatRunSpec,
    CitationKind, Claim, CommandRun, CommandRunId, CommandRunStatus, CoverageRow,
    DEFAULT_MAX_CONCURRENT_ITEMS, Document, DocumentHead, DocumentId, EventKind, Executor,
    FOLLOW_UP_RUN_CANCELLED, FOLLOW_UP_SESSION_ENDED, FollowUpRefusal, FollowUpRequest,
    FollowUpSettle, FollowUpView, FollowUpWindow, GateOutcome, Item, ItemCitation, ItemFilter,
    ItemId, ItemKind, ItemKindId, ItemKindPatch, ItemLink, ItemPatch, ItemRequirement,
    ItemRevision, ItemSummary, LinkEdge, LinkGraph, LinkKind, LinkNode, NewCommandRun, NewDocument,
    NewFollowUp, NewItem, NewItemKind, NewNote, NewPersona, NewProject, NewPromptTemplate, NewRepo,
    NewRequirement, NewRequirementArea, NewRun, NewRunStep, NewSkill, NewSkillVersion,
    NewStepGraph, NewWorkspace, Note, OpenPermission, PermissionChoice, PermissionId,
    PermissionStatus, Persona, PersonaId, PersonaPatch, PhaseAgent, PhaseId, PhasePatch, Project,
    ProjectId, ProjectPatch, ProjectRef, PromptScope, PromptTemplate, PromptTemplateId, QueueBatch,
    QueueEntry, QueueSetting, QueuedFollowUp, RelaySessionId, RelayView, Repo, RepoBoxPath, RepoId,
    RepoPatch, Requirement, RequirementArea, RequirementAreaId, RequirementFilter, RequirementId,
    RequirementPatch, RequirementRevision, RequirementSpec, RequirementState, RequirementUpdate,
    Resolution, ResolvedGraph, ResolvedInput, ResolvedPhase, Run, RunCommand, RunCommandId,
    RunCommandKind, RunCommandStatus, RunId, RunKind, RunMode, RunStatus, RunStep, RunStepCommit,
    RunStepSummary, RunStepTree, RunSummary, Scope, SessionEvent, SettleOutcome, Skill,
    SkillBinding, SkillBindingId, SkillBindingKey, SkillId, SkillPatch, SkillVersion, Status,
    StepGraph, StepGraphId, StepGraphPatch, StepGraphPhase, StepId, StepOpening, StepOutcome,
    StepPermission, StepStatus, TIMESTAMPTZ_DIGITS, ToolCallCount, UpstreamEntry, UserId,
    WaitingCandidate, WaitingPermission, Workspace, WorkspaceBoxPath, WorkspaceId, WorkspacePatch,
    WorkspaceProject, WorkspaceSummary, canonical_declared_tags, missing_tags_failure, overlaps,
    prompt_summary, scope_of,
};
use crate::prompt::DEFAULT_TEMPLATES;
use crate::prompt::settings::{SettingKey, rung_refusal, validate};
use crate::prompt::template::TemplateRole;
use crate::seed;
use crate::store::error::{Result, StoreError};
use crate::store::traits::{
    BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN, BOX_PROBE_SPEC_NOT_AN_OBJECT, BOX_SETTINGS_NOT_AN_OBJECT,
    BindingFacts, COMMAND_STALE_AFTER, CasOutcome, DeleteReach, DeleteTarget,
    EXECUTOR_MUST_BE_KNOWN, ParkOutcome, QueueStored, QueueTarget, QueueToken, ReadStore,
    SettingRung, StepFence, StoredSetting, UpdateOutcome, WriteStore, already_exists,
    batch_is_closed, chat_step_status, check_attachment, citation_key, close_out_needs_a_summary,
    command_finish_status, command_not_claimable, command_not_queued, document_needs_a_step,
    expected_on_row, failure_disagrees_with_status, finish_run_item_mirror,
    finish_run_needs_a_terminal_status, graph_not_in_project, invalid_area_code, invalid_prefix,
    item_has_a_live_run, item_kind_is_held, item_not_in_project, lease_ttl_micros, legal_move,
    link_key, link_not_proposed_by_run, link_outside_project, new_persona_refusal,
    new_skill_refusal, not_a_fanout_candidate, not_a_terminal_status, note_needs_a_step,
    persona_is_bound, persona_patch_refusal, prompt_template_key, prompt_template_refusal,
    queue_target_refusal, queue_token_refusal, reaped_note, references_no_row,
    requirement_withdrawn, reserved_phase_name, resolution_not_closable, row_names_another_phase,
    row_names_another_step, run_is_terminal, self_link, skill_body_refusal, skill_patch_refusal,
    skill_version_key, step_document_refusal, step_is_not_promotable, step_note_refusal,
    step_slot_is_taken, step_writes_own_item, summary_names_another_item, winner_is_not_settled,
    withdrawn_requirement_cited,
};
use uuid::Uuid;

/// Where one [`MemStore`] handle reads "now" (MOD-40 plan D11, blueprint B21, B23).
///
/// `None` is the wall clock **untruncated**, exactly the stamps every `MemStore` wrote before
/// MOD-40: two back-to-back compare-and-sets share one microsecond about half the time (blueprint
/// P-11), and a truncated default would let a spent `updated_at` token read as current.
#[derive(Clone, Default)]
struct MemClock(Option<Arc<dyn Clock>>);

impl std::fmt::Debug for MemClock {
    /// Hand written: [`Clock`] carries no `Debug` supertrait.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "MemClock(injected)"
        } else {
            "MemClock(wall)"
        })
    }
}

impl MemClock {
    fn now(&self) -> DateTime<Utc> {
        self.0.as_ref().map_or_else(Utc::now, |clock| clock.now())
    }
}

/// The store the TUI runs against in MOD-1: every row in process memory, cloned out under a lock
/// that is never held across an `.await` (plan D6).
#[derive(Debug, Clone, Default)]
pub struct MemStore {
    state: Arc<RwLock<State>>,
    /// The writes [`MemStore::set_fault`] switched on, shared through clones as `state` is.
    #[cfg(feature = "test-support")]
    faults: Arc<RwLock<HashSet<MemFault>>>,
    /// This handle's clock (MOD-40 plan D11). **Not** shared through clones the way `state` is:
    /// a clone copies it, and [`MemStore::with_clock`] replaces it on one handle, so two handles
    /// on one set of rows can read two clocks, as two processes on one database do (blueprint
    /// B23).
    clock: MemClock,
}

/// MOD-4 plan D152: a call [`MemStore::set_fault`] can make fail, so a test can tell "the store
/// cannot answer" apart from "the row is gone".
///
/// A test seam only: nothing in production switches one on. A faulted call answers
/// [`StoreError::Unreachable`] before it touches any state, until it is switched off.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemFault {
    /// [`WriteStore::refresh_lease`].
    RefreshLease,
    /// [`WriteStore::release_lease`].
    ReleaseLease,
    /// [`WriteStore::transition`], the item compare-and-set.
    ItemTransition,
    /// [`WriteStore::record_opening`] (MOD-37 review N-4).
    RecordOpening,
    /// [`WriteStore::lease_holds`] (MOD-78 D5): a read, so `command_run` can tell a store blip
    /// from a lost lease.
    LeaseHolds,
}

/// A `step_permission` row with its `owner`, which [`StepPermission`] deliberately omits
/// (MOD-42 blueprint B-8).
#[derive(Debug, Clone)]
struct PermissionRow {
    /// The row as readers see it.
    row: StepPermission,
    /// `step_permission.owner`: the executor's lease owner at park time (I-2).
    owner: Uuid,
}

/// `run_command`'s MOD-70 columns of a `follow_up` row (blueprint B-2), beside the row as
/// [`PermissionRow::owner`] sits beside its row: [`RunCommand`] is unchanged (plan D5).
#[derive(Clone)]
struct FollowUpPayload {
    /// `run_command.run_step_id`.
    run_step_id: StepId,
    /// `run_command.text`: as typed while the row is `pending`, `None` once it resolves (I-5).
    text: Option<String>,
}

impl core::fmt::Debug for FollowUpPayload {
    /// The step and the text's length, never the text (I-5).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FollowUpPayload")
            .field("run_step_id", &self.run_step_id)
            .field("len", &self.text.as_ref().map(String::len))
            .finish()
    }
}

/// `command_run.claimed_by` and `heartbeat_at` (MOD-11 `0015`, B-16): the queue's liveness,
/// which [`CommandRun`] deliberately omits, as `run.lease_owner` is not a [`Run`] field. A
/// `queued` row has one too (R-3): its waiter's beat, with no claimant yet.
#[derive(Debug, Clone, Copy)]
struct CommandClaim {
    /// `command_run.claimed_by`: the claimant's per-call id; `None` while the row is `queued`.
    claimant: Option<Uuid>,
    /// `command_run.heartbeat_at`: the enqueue's instant, then each claim's of the `queued` row,
    /// then the admission's, then the claimant's last beat.
    heartbeat_at: DateTime<Utc>,
}

/// Every §5 table the TUI reads, keyed the way the queries of §7 look rows up.
#[derive(Debug, Default)]
struct State {
    /// `app_user`. Read by [`MemStore::this_user`], which is who a chat run is started by.
    users: HashMap<UserId, AppUser>,
    /// `box`.
    boxes: HashMap<BoxId, BoxRow>,
    /// The box this process runs on (`box.toml`), for the top bar.
    this_box: Option<BoxId>,
    /// `workspace`.
    workspaces: HashMap<WorkspaceId, Workspace>,
    /// `workspace_project`.
    workspace_projects: Vec<WorkspaceProject>,
    /// `workspace_box_path` (`R-BOX-4`). No fixture loads it: MOD-15 is its first writer.
    workspace_box_paths: Vec<WorkspaceBoxPath>,
    /// `project`.
    projects: HashMap<ProjectId, Project>,
    /// `repo`. No fixture loads it: MOD-15 is its first writer, and MOD-7 its second.
    repos: HashMap<RepoId, Repo>,
    /// `repo_box_path` (`R-BOX-4`). Empty for the reason [`State::repos`] is.
    repo_box_paths: Vec<RepoBoxPath>,
    /// `item_kind`.
    kinds: HashMap<ItemKindId, ItemKind>,
    /// `step_graph`, read by [`WriteStore::step_graphs`] since MOD-15 (plan D1).
    graphs: HashMap<StepGraphId, StepGraph>,
    /// `step_graph_phase`, read by [`WriteStore::phases`] since MOD-15 (plan D1).
    phases: Vec<StepGraphPhase>,
    /// `phase_agent`, keyed as its primary key is, so the map's order is `position` order within
    /// each phase. Written by [`WriteStore::create_phase_agents`] only (MOD-37 R-6); no fixture
    /// loads it.
    phase_agents: BTreeMap<(PhaseId, i32), PhaseAgent>,
    /// `prompt_template`, read by the inherent [`MemStore::prompt_templates`] (MOD-2 plan D102) and
    /// appended to only by [`WriteStore::append_prompt_template`] and the project seed (MOD-9 D1).
    templates: Vec<PromptTemplate>,
    /// `skill`, read by the inherent [`MemStore::bound_skills`] (MOD-2 plan D105) and
    /// [`WriteStore::skills`], written by [`WriteStore::create_skill`] and
    /// [`WriteStore::update_skill`] (MOD-9 milestone 3).
    skills: HashMap<SkillId, Skill>,
    /// `persona` (MOD-26 D1), read by [`WriteStore::personas`] and `resolve_graph`, written by
    /// [`WriteStore::create_persona`] and [`WriteStore::update_persona`]; global, so
    /// `delete_project` leaves it.
    personas: HashMap<PersonaId, Persona>,
    /// `skill_version`, resolved through [`SkillBinding::version_in_force`], read by
    /// [`WriteStore::skill_versions`] and appended to by [`WriteStore::create_skill`] and
    /// [`WriteStore::add_skill_version`].
    skill_versions: Vec<SkillVersion>,
    /// `skill_binding`, resolved through `model::skill::resolve`, read by
    /// [`WriteStore::skill_bindings`] and written by [`WriteStore::set_skill_binding`].
    skill_bindings: Vec<SkillBinding>,
    /// `box_tool`, projected by the inherent [`MemStore::box_profile`].
    box_tools: Vec<BoxTool>,
    /// `box.probe_spec_digest`, which `BoxRow` does not carry (D14). Written by
    /// [`WriteStore::record_box_probe`]; no fixture loads it.
    box_probe_digests: HashMap<BoxId, String>,
    /// `app_setting`, the last rung of the prompt's settings chain (`docs/ANA-5.md` §4.4), each
    /// value paired with the `updated_at` the `App` rung's compare-and-set compares against.
    ///
    /// The column is the CAS token because `app_setting` has no `version` column and this milestone
    /// adds no migration (PRD D8); the map carries it so `MemStore` can answer
    /// [`WriteStore::setting`] with a token at all. Empty unless something wrote it: no fixture
    /// loads it.
    app_settings: BTreeMap<String, (Value, DateTime<Utc>)>,
    /// `agent`, read by the inherent [`MemStore::agents`] (MOD-2 plan D3).
    agents: HashMap<AgentId, Agent>,
    /// `agent_box`, keyed as its composite primary key is. Empty until something probes a box:
    /// no fixture loads it (MOD-2 plan D3).
    agent_boxes: HashMap<(AgentId, BoxId), AgentBox>,
    /// `agent_box.user_off` (MOD-23 D242): the `(agent, box)` pairs the human switched off. A
    /// side set rather than an `AgentBox` field, because the column is not part of the row type.
    /// Written by [`WriteStore::set_agent_box_enabled`] only.
    agent_boxes_off: HashSet<(AgentId, BoxId)>,
    /// `item_key_counter` (§4.1): the highest number minted per `(project, prefix)`.
    item_key_counter: HashMap<(ProjectId, String), i32>,
    /// `item`.
    items: HashMap<ItemId, Item>,
    /// `item_revision`, keyed as its composite primary key is.
    revisions: HashMap<(ItemId, i32), ItemRevision>,
    /// `item_link`; tombstones are kept, a live edge has `deleted_at == None` (§5.5).
    links: Vec<ItemLink>,
    /// `item_note`.
    notes: Vec<Note>,
    /// `document`, bodies included.
    documents: Vec<Document>,
    /// `run`.
    runs: HashMap<RunId, Run>,
    /// `run_step`.
    steps: HashMap<StepId, RunStep>,
    /// `run_step_tree` (ANA-2 §4.6) keyed by the table's primary key, so
    /// [`ReadStore::step_trees`] comes out in `repo_id` order for free. No fixture loads it:
    /// MOD-4 is the table's first writer anywhere.
    step_trees: BTreeMap<(StepId, RepoId), RunStepTree>,
    /// `run_step_commit`, keyed the same way. `MemStore` held no such rows before MOD-4, which is
    /// why [`DeleteReach::run_step_commits`] used to be a hard-coded `0`.
    step_commits: BTreeMap<(StepId, RepoId), RunStepCommit>,
    /// `command_run` (`0001_init.sql:537`), keyed by its own id so a duplicate write is a lookup
    /// rather than a scan, and read back sorted by `(queued_at, id)`.
    ///
    /// A `BTreeMap` on the id alone and not on `(run_step_id, queued_at, id)`: the table's key is
    /// the id, `queued_at` is mutable in principle, and a step's rows are few enough that the
    /// filter-and-sort [`State::command_runs`] does is cheaper than a compound key that would have
    /// to be maintained. No fixture loads it — MOD-4 milestone 3 is its first writer anywhere.
    command_runs: BTreeMap<CommandRunId, CommandRun>,
    /// The liveness of every `running` `command_run` row a claim admitted (MOD-11 D14, B-16), by
    /// row id; a row leaves the map when it finishes, is cancelled or is reaped.
    command_claims: HashMap<CommandRunId, CommandClaim>,
    /// `run.lease_owner`, which is deliberately not a [`Run`] field: the mirror does not carry it
    /// and a reader has no use for another process's liveness token (blueprint F-S). Kept beside
    /// the run so [`WriteStore::refresh_lease`] can compare against it.
    lease_owners: HashMap<RunId, Uuid>,
    /// `run_step.opening` (MOD-37 M5), beside `steps` as `lease_owners` sits beside `runs`: the
    /// column is a [`RunStepSummary`] field and not a [`RunStep`] one.
    openings: HashMap<StepId, StepOpening>,
    /// `step_permission` (MOD-42 plan D1), by id; `owner` beside the row (B-8), as
    /// [`State::lease_owners`] sits beside `runs`.
    permissions: BTreeMap<PermissionId, PermissionRow>,
    /// `run_command` (MOD-42 plan D1), by id.
    run_commands: BTreeMap<RunCommandId, RunCommand>,
    /// `queue_entry` (MOD-12 D1), by item: an item is queued on at most one box. The entry keeps
    /// its item's project, which never moves.
    queue_entries: BTreeMap<ItemId, QueueEntry>,
    /// `queue_batch` (MOD-12 D2), by id.
    queue_batches: BTreeMap<BatchId, QueueBatch>,
    /// `run.batch_id` (MOD-12 D7), beside `runs` as `lease_owners` is: not a `Run` field.
    run_batches: HashMap<RunId, BatchId>,
    /// `run_command`'s MOD-70 columns for `follow_up` rows, by id (B-2): `run_step_id` and the
    /// text, `None` once the row resolves. The row itself is in `run_commands`.
    follow_up_payloads: BTreeMap<RunCommandId, FollowUpPayload>,
    /// `follow_up_window` (MOD-70 D1), by step.
    follow_up_windows: BTreeMap<StepId, FollowUpWindow>,
    /// `session_event`.
    events: Vec<SessionEvent>,
    /// `requirement_spec` (ANA-11 §4.4), keyed by its primary key, the project (MOD-38).
    requirement_specs: HashMap<ProjectId, RequirementSpec>,
    /// `requirement_area`.
    requirement_areas: HashMap<RequirementAreaId, RequirementArea>,
    /// `requirement_key_counter`: the highest number minted per area (MOD-38 plan D8).
    requirement_key_counter: HashMap<RequirementAreaId, i32>,
    /// `requirement`.
    requirements: HashMap<RequirementId, Requirement>,
    /// `requirement_revision`, append-only.
    requirement_revisions: Vec<RequirementRevision>,
    /// `item_requirement`; tombstones are kept, as [`State::links`] keeps them (plan D10).
    item_requirements: Vec<ItemRequirement>,
}

impl MemStore {
    /// An empty store (plan D7: fixtures are opt-in).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A store loaded with the demo fixture of blueprint §G.
    #[cfg(feature = "demo")]
    #[must_use]
    pub fn demo() -> Self {
        Self::from_demo(crate::fixtures::demo_data())
    }

    /// Who this process is, as far as an in-memory store can know (MOD-2 milestone 3).
    ///
    /// `PgStore` learns its user by seeding or reading `app_user` on connect; a `MemStore` has no
    /// connect step, so "this user" is the earliest-created row, with the id as the tiebreak so
    /// two rows stamped at the same instant still answer deterministically. `None` for an empty
    /// store, which is what makes a chat against `MemStore::new()` refuse instead of inventing an
    /// author for a `run` row.
    #[must_use]
    pub fn this_user(&self) -> Option<UserId> {
        self.read(|state| {
            state
                .users
                .values()
                .min_by(|left, right| {
                    left.created_at
                        .cmp(&right.created_at)
                        .then_with(|| left.id.cmp(&right.id))
                })
                .map(|user| user.id)
        })
    }

    /// A store loaded with the given fixture and nothing else.
    #[cfg(feature = "demo")]
    #[must_use]
    pub fn from_demo(data: crate::fixtures::DemoData) -> Self {
        let state = State {
            users: data.users.into_iter().map(|row| (row.id, row)).collect(),
            boxes: data.boxes.into_iter().map(|row| (row.id, row)).collect(),
            this_box: data.this_box,
            workspaces: data
                .workspaces
                .into_iter()
                .map(|row| (row.id, row))
                .collect(),
            workspace_projects: data.workspace_projects,
            workspace_box_paths: Vec::new(),
            projects: data.projects.into_iter().map(|row| (row.id, row)).collect(),
            repos: HashMap::new(),
            repo_box_paths: Vec::new(),
            kinds: data.kinds.into_iter().map(|row| (row.id, row)).collect(),
            graphs: data.graphs.into_iter().map(|row| (row.id, row)).collect(),
            phases: data.phases,
            phase_agents: BTreeMap::new(),
            templates: data.templates,
            skills: data.skills.into_iter().map(|row| (row.id, row)).collect(),
            personas: data.personas.into_iter().map(|row| (row.id, row)).collect(),
            skill_versions: data.skill_versions,
            skill_bindings: data.skill_bindings,
            box_tools: data.box_tools,
            box_probe_digests: HashMap::new(),
            app_settings: BTreeMap::new(),
            agents: data.agents.into_iter().map(|row| (row.id, row)).collect(),
            agent_boxes: HashMap::new(),
            agent_boxes_off: HashSet::new(),
            item_key_counter: data.item_key_counter,
            items: data.items.into_iter().map(|row| (row.id, row)).collect(),
            revisions: data
                .revisions
                .into_iter()
                .map(|row| ((row.item_id, row.version), row))
                .collect(),
            links: data.links,
            notes: data.notes,
            documents: data.documents,
            runs: data.runs.into_iter().map(|row| (row.id, row)).collect(),
            steps: data.steps.into_iter().map(|row| (row.id, row)).collect(),
            step_trees: BTreeMap::new(),
            step_commits: BTreeMap::new(),
            command_runs: BTreeMap::new(),
            command_claims: HashMap::new(),
            lease_owners: HashMap::new(),
            openings: HashMap::new(),
            permissions: BTreeMap::new(),
            run_commands: BTreeMap::new(),
            queue_entries: BTreeMap::new(),
            queue_batches: BTreeMap::new(),
            run_batches: HashMap::new(),
            follow_up_payloads: BTreeMap::new(),
            follow_up_windows: BTreeMap::new(),
            events: data.events,
            requirement_specs: data
                .requirement_specs
                .into_iter()
                .map(|row| (row.project_id, row))
                .collect(),
            requirement_areas: data
                .requirement_areas
                .into_iter()
                .map(|row| (row.id, row))
                .collect(),
            requirement_key_counter: data.requirement_key_counter,
            requirements: data
                .requirements
                .into_iter()
                .map(|row| (row.id, row))
                .collect(),
            requirement_revisions: data.requirement_revisions,
            item_requirements: data.item_requirements,
        };
        Self {
            state: Arc::new(RwLock::new(state)),
            #[cfg(feature = "test-support")]
            faults: Arc::default(),
            clock: MemClock::default(),
        }
    }

    /// How many items the store holds. Tests only; the UI counts what a query returned.
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.read(|state| state.items.len())
    }

    /// The workspaces of this store, ordered by name.
    ///
    /// Inherent rather than a [`ReadStore`] method: §6.1 is quoted verbatim and has no
    /// `workspaces()`, so the `Backend` enum of `htui-store` exposes hierarchy reads inherently
    /// (blueprint B.7).
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        Ok(self.read(State::workspace_summaries))
    }

    /// This box's row, projected for the top bar.
    pub async fn box_info(&self) -> Result<Option<BoxInfo>> {
        Ok(self.read(|state| {
            let id = state.this_box?;
            let row = state.boxes.get(&id)?;
            Some(BoxInfo {
                box_id: row.id,
                hostname: row.hostname.clone(),
                os_family: row.os_family,
                probed_tags: row.probed_tags.clone(),
                declared_tags: row.declared_tags.clone(),
                settings: row.settings.clone(),
            })
        }))
    }

    /// How many runs of the scope are active (`RunStatus::is_active`).
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize> {
        Ok(self.read(|state| {
            state
                .runs
                .values()
                .filter(|run| run.status.is_active() && scope.contains(run.project_id))
                .count()
        }))
    }

    /// The scope's projects, ordered by `workspace_project.position`.
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>> {
        Ok(self.read(|state| state.project_refs(scope)))
    }

    /// The agent registry, ordered by `agent.name`, each row carrying **this box's** `agent_box`
    /// when there is one.
    ///
    /// Inherent for the same reason the four reads above are, and one more: `agent` and
    /// `agent_box` are not mirrored (`docs/ANA-9.md` §4.4), so no offline backend could answer it
    /// (MOD-2 plan D3).
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn agents(&self) -> Result<Vec<AgentSummary>> {
        Ok(self.read(State::agent_summaries))
    }

    /// `project.settings` of one project, or `None` when this store holds no such project.
    ///
    /// Inherent for the reason [`MemStore::agents`] is — `Backend` dispatches over three stores and
    /// no trait method is needed — and read at all because the per-run token cap lives in this
    /// column (MOD-2 plan D70, `docs/ANA-4.md` §7 `:1143-1150`). The offline mirror carries
    /// `project.settings`, so every backend can answer it, which is why the cap needed no migration
    /// and no env stand-in.
    ///
    /// The whole document and not the two cap keys: reading the column is what the store owes, and
    /// what a caller makes of its contents is
    /// [`ProjectCaps::from_settings`](crate::model::ProjectCaps::from_settings)'s business.
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
        Ok(self.read(|state| state.projects.get(&project).map(|row| row.settings.clone())))
    }

    /// A project's `prompt_template` rows, ordered by `(name, version)` (`docs/ANA-5.md` §4.6).
    ///
    /// Inherent rather than a [`ReadStore`] method for the reason [`MemStore::agents`] is, plus
    /// the one that decided the other four prompt reads: `prompt_template` has no cache mirror
    /// (`cache_migrations/0001_mirror.sql` declares no such table), so no offline backend could
    /// answer it and the `Backend::Offline` arm refuses (plan D109).
    ///
    /// Every version, not the latest per name: the caller picks by `(name, version)` because a
    /// phase may pin `template_version`, and picking here would hide the pin.
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn prompt_templates(&self, project: ProjectId) -> Result<Vec<PromptTemplate>> {
        Ok(self.read(|state| {
            let mut rows: Vec<PromptTemplate> = state
                .templates
                .iter()
                .filter(|row| row.project_id == project)
                .cloned()
                .collect();
            rows.sort_by(|left, right| {
                left.name
                    .as_bytes()
                    .cmp(right.name.as_bytes())
                    .then_with(|| left.version.cmp(&right.version))
            });
            rows
        }))
    }

    /// The skill candidates of one step (ANA-22 §6 items 2-3): the global attachments, the
    /// project's, and — with `phase` — that phase's, resolved most-specific-wins by
    /// [`resolve`](crate::model::skill::resolve), exactly as `PgStore`'s two `SELECT`s are, so the
    /// rule has one definition rather than one per backend.
    ///
    /// Inactive winners are included — an `off` or `glob` attachment, and a winning pin that
    /// names no version (`version: None`, plan D39); the assembler's `select` decides and records
    /// them. A binding whose `skill` row is missing is dropped.
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>> {
        Ok(self.read(|state| {
            let rows = state
                .skill_bindings
                .iter()
                .filter(|binding| match binding.project_id {
                    None => true,
                    // `phase: None` with a phase row: `phase_id == phase` is false, so the row
                    // is excluded, as `PgStore`'s `b.phase_id = NULL` is.
                    Some(owner) => {
                        owner == project
                            && (binding.phase_id.is_none() || binding.phase_id == phase)
                    }
                })
                .filter_map(|binding| {
                    state
                        .skills
                        .get(&binding.skill_id)
                        .map(|skill| (binding.clone(), skill.name.clone()))
                })
                .collect();
            crate::model::skill::resolve(rows, &state.skill_versions)
        }))
    }

    /// One box projected for the prompt's `box` section, or `None` when no row has that id.
    ///
    /// The `box_tool` join is [`BoxProfile::project`](crate::model::BoxProfile::project)'s: name
    /// byte order, capped, `path` dropped.
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn box_profile(&self, id: BoxId) -> Result<Option<BoxProfile>> {
        Ok(self.read(|state| {
            let row = state.boxes.get(&id)?;
            let tools: Vec<BoxTool> = state
                .box_tools
                .iter()
                .filter(|tool| tool.box_id == id)
                .cloned()
                .collect();
            Some(BoxProfile::project(row, tools))
        }))
    }

    /// Every `app_setting` row, keyed by name: the last rung of the prompt's settings chain
    /// (`docs/ANA-5.md` §4.4).
    ///
    /// A [`BTreeMap`] and not a [`HashMap`]: the assembler's budget resolution records which rung
    /// answered, and a map iterated in hash order would make that record depend on the process's
    /// random state.
    ///
    /// A `MemStore` loads none of these from the fixture, so this is empty unless something wrote
    /// one. That is not a gap: plan D101 compiles the defaults into `prompt::settings::DEFAULTS`
    /// precisely because `app_setting` is the one prompt input with no mirror **and** no generic
    /// reader, so an absent row is the normal case, not a failure.
    ///
    /// The stored `updated_at` is projected away here: the settings chain resolves values, and the
    /// CAS token belongs to [`WriteStore::setting`], which is where an editor reads it.
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn app_settings(&self) -> Result<BTreeMap<String, Value>> {
        Ok(self.read(|state| {
            state
                .app_settings
                .iter()
                .map(|(key, (value, _))| (key.clone(), value.clone()))
                .collect()
        }))
    }

    /// Writes one `app_setting` row without validating it or comparing a token. **Tests only.**
    ///
    /// [`WriteStore::set_setting`] is the product writer since MOD-15, and it refuses every value
    /// the reader would clamp or ignore (plan D7). This one stays because the opposite is also
    /// worth testing: the resolvers' fall-through rule only fires on a stored value the validator
    /// would never have accepted, and there has to be a way to plant one.
    pub fn set_app_setting(&self, key: &str, value: Value) {
        let now = self.now();
        self.write(|state| state.app_settings.insert(key.to_owned(), (value, now)));
    }

    /// Replaces one project's `settings` blob without validation. **Tests only** — same reason as
    /// [`set_app_setting`](Self::set_app_setting): no seam writer reaches `project.settings`
    /// (`update_project` never touches the column), and a harness built on a finished store
    /// cannot rebuild it to plant, say, a `judge_agent_id` (plan D69).
    ///
    /// A project that is not there is left alone, as a no-op.
    pub fn set_project_settings(&self, project: ProjectId, settings: Value) {
        let now = self.now();
        self.write(|state| {
            if let Some(row) = state.projects.get_mut(&project) {
                row.settings = settings;
                row.updated_at = now;
            }
        });
    }

    /// Plants `project.secret_provider` and `project.secret_scope` without validation. **Tests
    /// only**, like [`set_project_settings`](Self::set_project_settings): the validated writer is
    /// `update_project`'s `secret` (MOD-10 M4); this stays the tests' unvalidated planter. A
    /// project that is not there is left alone.
    pub fn set_project_secret_columns(
        &self,
        project: ProjectId,
        provider: Option<&str>,
        scope: Option<&str>,
    ) {
        let now = self.now();
        self.write(|state| {
            if let Some(row) = state.projects.get_mut(&project) {
                row.secret_provider = provider.map(str::to_owned);
                row.secret_scope = scope.map(str::to_owned);
                row.updated_at = now;
            }
        });
    }

    /// One `item_kind` row, or `None` when no row has that id.
    ///
    /// The prompt's `{{item}}` section names the kind, and `item` carries only `kind_id`; `§6.1`
    /// returns the kind nowhere, so the assembler's caller reads it here.
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn item_kind(&self, id: ItemKindId) -> Result<Option<ItemKind>> {
        Ok(self.read(|state| state.kinds.get(&id).cloned()))
    }

    // ---- MOD-4 milestone 1: the eleven inherent reads of ANA-2 §8 ------------------------------
    //
    // Inherent rather than [`ReadStore`] methods for the reason [`MemStore::agents`] is: none of
    // `step_graph`, `step_graph_phase`, `phase_agent`, `prompt_template`, `agent_box`, `box`,
    // `repo_box_path` or `app_setting` is mirrored, so no offline backend could answer them and
    // `Backend` dispatches with a `match self` (plan D1, blueprint F-N). The five §8 names that
    // already existed — `agents`, `app_settings`, `item_kind`, `phases`, `repos` — are not
    // repeated here.

    /// One `step_graph` row, or `None`.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn step_graph(&self, id: StepGraphId) -> Result<Option<StepGraph>> {
        Ok(self.read(|state| state.graphs.get(&id).cloned()))
    }

    /// A phase's candidate agents in `position` order: the rows
    /// [`WriteStore::create_phase_agents`] wrote (MOD-37 R-6). The fixture seeds none, and the
    /// snapshot builder falls back to `project.settings.default_agent_id` when a phase has no
    /// candidate.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
        Ok(self.read(|state| state.phase_agents_of(phase)))
    }

    /// One `prompt_template` by `(project, name)`: the pinned `version` when there is one, else
    /// the highest. `None` when the pin cannot be honoured, which is
    /// [`SkillBinding::version_in_force`]'s rule for the same shape of question.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>> {
        Ok(self.read(|state| {
            state
                .templates
                .iter()
                .filter(|row| row.project_id == project && row.name == name)
                .filter(|row| version.is_none_or(|want| row.version == want))
                .max_by_key(|row| row.version)
                .cloned()
        }))
    }

    /// The graph an item runs under — its own `step_graph_id`, else its kind's default — with the
    /// phases in `position` order (ANA-2 §8). `None` when the item, its kind or the graph is gone.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
        Ok(self.read(|state| state.resolve_graph(item)))
    }

    /// Every `agent_box` row of one box, in `agent_id` byte order.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
        Ok(self.read(|state| {
            let mut rows: Vec<AgentBox> = state
                .agent_boxes
                .values()
                .filter(|row| row.box_id == box_id)
                .cloned()
                .collect();
            rows.sort_by_key(|row| row.agent_id);
            rows
        }))
    }

    /// One whole `box` row, unlike [`MemStore::box_info`]'s top-bar projection: the admission of
    /// §4.7 needs `settings`, and `R-ORCH-10`'s matching needs both tag lists.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn box_row(&self, id: BoxId) -> Result<Option<BoxRow>> {
        Ok(self.read(|state| state.boxes.get(&id).cloned()))
    }

    /// Every repo checkout path on one box, in `repo_id` byte order (`R-BOX-4`).
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn repo_paths(&self, box_id: BoxId) -> Result<Vec<RepoBoxPath>> {
        Ok(self.read(|state| {
            let mut rows: Vec<RepoBoxPath> = state
                .repo_box_paths
                .iter()
                .filter(|row| row.box_id == box_id)
                .cloned()
                .collect();
            rows.sort_by_key(|row| row.repo_id);
            rows
        }))
    }

    /// The scope's ready items this box can actually take: §7.4's store-side half, then
    /// `R-ORCH-10`'s capability half — `required_tags` must be a subset of the box's
    /// `probed_tags ∪ declared_tags`. In queue order: `priority DESC, created_at, id` (MOD-12 D4,
    /// ANA-2 criterion 22).
    ///
    /// The capability half lives here rather than in [`ItemFilter`] because the filter's `tags`
    /// conjunct is the caller's own vocabulary; this one is the machine's, and MOD-4 is the first
    /// caller that has a box to match against.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn ready_items(&self, scope: &Scope, box_id: BoxId) -> Result<Vec<ItemSummary>> {
        Ok(self.read(|state| {
            let capabilities = state.box_capabilities(box_id);
            let mut rows: Vec<ItemSummary> = state
                .item_summaries(
                    scope,
                    &ItemFilter {
                        ready: Some(true),
                        ..ItemFilter::default()
                    },
                )
                .into_iter()
                .filter(|row| {
                    row.required_tags
                        .iter()
                        .all(|tag| capabilities.contains(tag))
                })
                .collect();
            // `ItemId`'s `Ord` is uuid byte order, which is Postgres' uuid order.
            rows.sort_by_cached_key(|row| {
                let item = state.items.get(&row.id);
                (
                    core::cmp::Reverse(item.map_or(row.priority, |item| item.priority)),
                    item.map(|item| item.created_at),
                    row.id,
                )
            });
            rows
        }))
    }

    /// The `required_tags` of one item the box has neither probed nor declared, in byte order:
    /// what the Backlog renders beside an item it cannot start here (`R-ORCH-10`).
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown item (`"item"`) or box (`"box"`), the item looked
    /// up first.
    pub async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
        self.read(|state| {
            let required = state.require_item(item)?.required_tags.clone();
            if !state.boxes.contains_key(&box_id) {
                return Err(StoreError::NotFound {
                    entity: "box",
                    id: box_id.to_string(),
                });
            }
            Ok(state.missing_for(&required, box_id))
        })
    }

    /// How many runs hold a slot on one box: §4.7's admission count, which is `running` and
    /// `awaiting_approval` and **not** `queued` — a queued run occupies nothing yet.
    ///
    /// Distinct from [`MemStore::active_runs`], which counts a scope's live runs for the top bar
    /// and does count `queued`.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn active_runs_on_box(&self, box_id: BoxId) -> Result<usize> {
        Ok(self.read(|state| {
            state
                .runs
                .values()
                .filter(|row| {
                    row.executing_box_id == Some(box_id)
                        && matches!(row.status, RunStatus::Running | RunStatus::AwaitingApproval)
                })
                .count()
        }))
    }

    /// This box's `queued` runs, `(id, queued_at)` by `(queued_at, id)` (MOD-41 plan D11).
    /// `RunId`'s `Ord` is uuid byte order, which is Postgres' uuid order.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn queued_runs_on_box(&self, box_id: BoxId) -> Result<Vec<(RunId, DateTime<Utc>)>> {
        Ok(self.read(|state| {
            let mut rows: Vec<(RunId, DateTime<Utc>)> = state
                .runs
                .values()
                .filter(|row| row.target_box_id == box_id && row.status == RunStatus::Queued)
                .map(|row| (row.id, row.queued_at))
                .collect();
            rows.sort_by_key(|(id, at)| (*at, *id));
            rows
        }))
    }

    // ---- MOD-12 M1: the queue (plan D1-D9). Neither table is mirrored, so these are inherent,
    // dispatched by `Backend`; `PgStore` carries the same nine.

    /// MOD-12 D1: queue `item` on `box_id`. Idempotent: an item already queued (on any box)
    /// answers its stored entry unchanged. `at` is microsecond-truncated.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] `{ entity: "item" }` for an unknown item;
    /// [`StoreError::Constraint`] for an unknown box or user.
    pub async fn queue_item(
        &self,
        item: ItemId,
        box_id: BoxId,
        by: UserId,
        at: DateTime<Utc>,
    ) -> Result<QueueEntry> {
        self.write(|state| {
            let project_id = state.require_item(item)?.project_id;
            if let Some(stored) = state.queue_entries.get(&item) {
                return Ok(stored.clone());
            }
            if !state.boxes.contains_key(&box_id) {
                return Err(StoreError::Constraint(references_no_row(
                    "queue_entry.box_id",
                    box_id,
                    "box",
                )));
            }
            state.require_user(by, "queue_entry.queued_by")?;
            let entry = QueueEntry {
                item_id: item,
                project_id,
                box_id,
                position: None,
                queued_at: at.trunc_subsecs(TIMESTAMPTZ_DIGITS),
                queued_by: by,
            };
            state.queue_entries.insert(item, entry.clone());
            Ok(entry)
        })
    }

    /// MOD-12 D9: `item` leaves whatever queue holds it; `false` when none did. Never touches a
    /// run.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn dequeue_item(&self, item: ItemId) -> Result<bool> {
        Ok(self.write(|state| state.queue_entries.remove(&item).is_some()))
    }

    /// MOD-12 D4: `box_id`'s entries, `position NULLS LAST, queued_at, item_id`. `ItemId`'s `Ord`
    /// is uuid byte order, which is Postgres' uuid order.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn queue_entries(&self, box_id: BoxId) -> Result<Vec<QueueEntry>> {
        Ok(self.read(|state| {
            let mut rows: Vec<QueueEntry> = state
                .queue_entries
                .values()
                .filter(|entry| entry.box_id == box_id)
                .cloned()
                .collect();
            rows.sort_by_key(|entry| {
                (
                    entry.position.is_none(),
                    entry.position,
                    entry.queued_at,
                    entry.item_id,
                )
            });
            rows
        }))
    }

    /// MOD-12 D2: resume — the open batch of `box_id`, opened now under a fresh `BatchId` unless
    /// one is already open (idempotent; two racing resumes answer the same row).
    ///
    /// # Errors
    /// [`StoreError::Constraint`] for an unknown box or user.
    pub async fn open_batch(
        &self,
        box_id: BoxId,
        by: UserId,
        at: DateTime<Utc>,
    ) -> Result<QueueBatch> {
        self.write(|state| {
            if let Some(open) = state.open_batch_of(box_id) {
                return Ok(open.clone());
            }
            if !state.boxes.contains_key(&box_id) {
                return Err(StoreError::Constraint(references_no_row(
                    "queue_batch.box_id",
                    box_id,
                    "box",
                )));
            }
            state.require_user(by, "queue_batch.opened_by")?;
            let batch = QueueBatch {
                id: BatchId::new(),
                box_id,
                opened_at: at.trunc_subsecs(TIMESTAMPTZ_DIGITS),
                opened_by: by,
                closed_at: None,
                closed_reason: None,
            };
            state.queue_batches.insert(batch.id, batch.clone());
            Ok(batch)
        })
    }

    /// MOD-12 D2: `box_id`'s open batch, if any: whether its queue runs.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn open_batch_of(&self, box_id: BoxId) -> Result<Option<QueueBatch>> {
        Ok(self.read(|state| state.open_batch_of(box_id).cloned()))
    }

    /// MOD-12 D2, D3: close `box_id`'s open batch with `reason`; `None` when none was open.
    ///
    /// In the same closure every run of the batch still `queued` is cancelled through
    /// [`State::finish_run`], so its item goes back to `open` and its queue entry stays (review
    /// H2): a pause stops the runs nobody has claimed yet. A `drained` close finds none.
    ///
    /// # Errors
    /// Never in practice: a `queued` run always cancels. The signature matches `PgStore`'s so
    /// `Backend` can dispatch over both.
    pub async fn close_batch(
        &self,
        box_id: BoxId,
        reason: BatchClose,
        at: DateTime<Utc>,
    ) -> Result<Option<QueueBatch>> {
        let now = self.now();
        self.write(|state| {
            let Some(id) = state.open_batch_of(box_id).map(|batch| batch.id) else {
                return Ok(None);
            };
            let stamp = at.trunc_subsecs(TIMESTAMPTZ_DIGITS);
            let mut waiting: Vec<&Run> = state
                .run_batches
                .iter()
                .filter(|(_, of)| **of == id)
                .filter_map(|(run, _)| state.runs.get(run))
                .filter(|row| row.status == RunStatus::Queued)
                .collect();
            waiting.sort_by_key(|row| (row.queued_at, row.id));
            let waiting: Vec<RunId> = waiting.into_iter().map(|row| row.id).collect();
            for run in waiting {
                state.finish_run(run, RunStatus::Cancelled, None, stamp, now)?;
            }
            Ok(state.queue_batches.get_mut(&id).map(|batch| {
                batch.closed_at = Some(stamp);
                batch.closed_reason = Some(reason);
                batch.clone()
            }))
        })
    }

    /// MOD-12 D3 (review M1): the drain's close, of exactly `batch` and only while it is still
    /// drained: open, its box with no queue entry, and no run of its own `queued`, `running` or
    /// `awaiting_approval`; one closure, as `PgStore`'s one UPDATE. `None` when it did not close.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn close_drained_batch(
        &self,
        batch: BatchId,
        at: DateTime<Utc>,
    ) -> Result<Option<QueueBatch>> {
        Ok(self.write(|state| {
            let row = state.queue_batches.get(&batch)?;
            let box_id = row.box_id;
            if row.closed_at.is_some()
                || state
                    .queue_entries
                    .values()
                    .any(|entry| entry.box_id == box_id)
                || state
                    .run_batches
                    .iter()
                    .filter(|(_, of)| **of == batch)
                    .filter_map(|(run, _)| state.runs.get(run))
                    .any(|row| row.status.is_active())
            {
                return None;
            }
            let row = state.queue_batches.get_mut(&batch)?;
            row.closed_at = Some(at.trunc_subsecs(TIMESTAMPTZ_DIGITS));
            row.closed_reason = Some(BatchClose::Drained);
            Some(row.clone())
        }))
    }

    /// MOD-12 D3: drop `box_id`'s entries whose item is `done` or `closed`; how many went.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn prune_finished_entries(&self, box_id: BoxId) -> Result<u64> {
        Ok(self.write(|state| {
            let items = &state.items;
            let before = state.queue_entries.len();
            state.queue_entries.retain(|item, entry| {
                entry.box_id != box_id
                    || !items
                        .get(item)
                        .is_some_and(|row| matches!(row.status, Status::Done | Status::Closed))
            });
            rows(before - state.queue_entries.len())
        }))
    }

    /// MOD-12 D3, D9: the runs admitted under `batch`, `(id, status)` by `(queued_at, id)`.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn batch_runs(&self, batch: BatchId) -> Result<Vec<(RunId, RunStatus)>> {
        Ok(self.read(|state| {
            let mut runs: Vec<&Run> = state
                .run_batches
                .iter()
                .filter(|(_, of)| **of == batch)
                .filter_map(|(run, _)| state.runs.get(run))
                .collect();
            runs.sort_by_key(|row| (row.queued_at, row.id));
            runs.into_iter().map(|row| (row.id, row.status)).collect()
        }))
    }

    /// MOD-12 (review H1): the items with a `cancelled` run under `batch`, in uuid order (`ItemId`'s
    /// `Ord`, Postgres' uuid order). The queue runner admits none of them again under that batch.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn batch_cancelled_items(&self, batch: BatchId) -> Result<Vec<ItemId>> {
        Ok(self.read(|state| {
            let items: BTreeSet<ItemId> = state
                .run_batches
                .iter()
                .filter(|(_, of)| **of == batch)
                .filter_map(|(run, _)| state.runs.get(run))
                .filter(|row| row.status == RunStatus::Cancelled)
                .filter_map(|row| row.item_id)
                .collect();
            items.into_iter().collect()
        }))
    }

    /// MOD-12 M2 D1: `Σ run_step.usage["cost_micros"]` over the steps of every run admitted under
    /// `batch`; `None` when no step reports an integer cost. Computed, never stored (ANA-2 §4.10).
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn batch_spend(&self, batch: BatchId) -> Result<Option<i64>> {
        Ok(self.read(|state| state.batch_spend(batch)))
    }

    /// MOD-12 M2 D5: the batch `run` was admitted under, with [`MemStore::batch_spend`] of it;
    /// `None` for a manual or chat run, and for an unknown run.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s.
    pub async fn run_batch_spend(&self, run: RunId) -> Result<Option<(BatchId, Option<i64>)>> {
        Ok(self.read(|state| {
            state
                .run_batches
                .get(&run)
                .map(|batch| (*batch, state.batch_spend(*batch)))
        }))
    }

    /// MOD-12 D6: `claim_run`'s slot count — `running` runs executing on `box_id`, **not**
    /// `awaiting_approval` (that is [`MemStore::active_runs_on_box`]).
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn running_runs_on_box(&self, box_id: BoxId) -> Result<usize> {
        Ok(self.read(|state| {
            state
                .runs
                .values()
                .filter(|row| {
                    row.executing_box_id == Some(box_id) && row.status == RunStatus::Running
                })
                .count()
        }))
    }

    /// Every active run whose `repo_scope` intersects `scope`, in `queued_at` order: what §4.7's
    /// overlap refusal names. An empty `scope` intersects nothing (hazard H-10).
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn overlapping_runs(&self, scope: &[RepoId]) -> Result<Vec<Run>> {
        Ok(self.read(|state| {
            let mut rows: Vec<Run> = state
                .runs
                .values()
                .filter(|row| {
                    row.status.is_active() && row.repo_scope.iter().any(|repo| scope.contains(repo))
                })
                .cloned()
                .collect();
            rows.sort_by_key(|row| (row.queued_at, row.id));
            rows
        }))
    }

    /// Takes the read lock, runs `f`, drops the guard and returns `f`'s owned result.
    ///
    /// A poisoned lock is recovered rather than propagated: `State` mutations are infallible map
    /// inserts, so an unrelated panic must not take the store down for the rest of the process.
    fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        let guard = self.state.read().unwrap_or_else(PoisonError::into_inner);
        f(&guard)
    }

    /// Takes the write lock, runs `f`, drops the guard and returns `f`'s owned result.
    fn write<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut guard = self.state.write().unwrap_or_else(PoisonError::into_inner);
        f(&mut guard)
    }

    /// MOD-4 plan D152: `on` makes the write `fault` names answer [`StoreError::Unreachable`]
    /// before it touches any state, on this store and every clone of it, until a call with
    /// `on == false` switches it off. A test seam; see [`MemFault`].
    #[cfg(feature = "test-support")]
    pub fn set_fault(&self, fault: MemFault, on: bool) {
        let mut faults = self.faults.write().unwrap_or_else(PoisonError::into_inner);
        if on {
            faults.insert(fault);
        } else {
            faults.remove(&fault);
        }
    }

    /// This handle, reading `clock` for every stamp and every lease comparison it makes (MOD-40
    /// plan D10, D11). The rows and the fault switches stay shared with every clone; the clock is
    /// this handle's alone.
    ///
    /// A frozen clock stamps every write with one instant, so a compare-and-set on `updated_at`
    /// cannot tell two edits made at it apart: a case that checks a spent token moves the clock
    /// between the two edits (blueprint F-42).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = MemClock(Some(clock));
        self
    }

    /// A handle on the same rows whose clock is a `TestClock` frozen at `now`: a second
    /// process's view, for a case that stages a stranger at another instant than its own clock
    /// (MOD-40 blueprint B24).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn handle_at(&self, now: DateTime<Utc>) -> Self {
        self.clone().with_clock(Arc::new(TestClock::at(now)))
    }

    /// Every `step_permission` row, in id order (MOD-42 blueprint B-21): for tests that must see
    /// rows no trait method lists (stale, cancelled, another item's).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn relay_rows(&self) -> Vec<StepPermission> {
        self.read(|state| state.permissions.values().map(|p| p.row.clone()).collect())
    }

    /// Every `run_command` row, in id order (MOD-42 blueprint B-21).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn command_rows(&self) -> Vec<RunCommand> {
        self.read(|state| state.run_commands.values().cloned().collect())
    }

    /// Every `queue_batch` row, open or closed, in id order (MOD-12 blueprint C.5): no store
    /// method reads a closed batch back, so a test that checks a close's reason reads it here.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn batch_rows(&self) -> Vec<QueueBatch> {
        self.read(|state| state.queue_batches.values().cloned().collect())
    }

    /// Every `follow_up` row as the Runs pane would see it, with whether its text is still stored
    /// (MOD-70 blueprint B-1): `true` exactly while it is pending (I-5). In id order.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn follow_up_rows(&self) -> Vec<(FollowUpView, bool)> {
        self.read(|state| {
            state
                .run_commands
                .values()
                .filter_map(|row| {
                    let payload = state.follow_up_payloads.get(&row.id)?;
                    Some((
                        FollowUpView {
                            id: row.id,
                            run_id: row.run_id,
                            run_step_id: payload.run_step_id,
                            status: row.status,
                            resolution: row.resolution.clone(),
                            issued_at: row.issued_at,
                            resolved_at: row.resolved_at,
                        },
                        payload.text.is_some(),
                    ))
                })
                .collect()
        })
    }

    /// Every `follow_up_window` row, in step-id order (MOD-70 blueprint B-1).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn follow_up_windows(&self) -> Vec<FollowUpWindow> {
        self.read(|state| state.follow_up_windows.values().cloned().collect())
    }

    /// MOD-76 D4 (R-55): sets `settings[key]` on box `id`'s row, on this store and every clone
    /// of it, as no `BoxEdit` field writes it (`command_limits`). `edit_version` does not move.
    /// Whether the row exists.
    ///
    /// # Panics
    /// When the row's `settings` is neither an object nor `null` (`edit_box` refuses to write
    /// one, so only a hand-built fixture has it).
    #[cfg(feature = "test-support")]
    pub fn set_box_setting(&self, id: BoxId, key: &str, value: Value) -> bool {
        self.write(|state| {
            state
                .boxes
                .get_mut(&id)
                .map(|row| row.settings[key] = value)
                .is_some()
        })
    }

    /// This handle's "now": its injected clock, else `Utc::now()` untruncated (blueprint B21).
    fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// `Err(Unreachable)` when `fault` is switched on (plan D152), `Ok(())` otherwise.
    #[cfg(feature = "test-support")]
    fn check_fault(&self, fault: MemFault) -> Result<()> {
        let faults = self.faults.read().unwrap_or_else(PoisonError::into_inner);
        if faults.contains(&fault) {
            return Err(StoreError::Unreachable(format!(
                "MemFault::{fault:?} is switched on"
            )));
        }
        Ok(())
    }
}

impl State {
    /// [`MemStore::batch_spend`]'s body: `as_i64` skips a non-integer cost, as `select::run_spend`
    /// does and as Postgres' text guard does.
    fn batch_spend(&self, batch: BatchId) -> Option<i64> {
        let runs: HashSet<RunId> = self
            .run_batches
            .iter()
            .filter(|(_, of)| **of == batch)
            .map(|(run, _)| *run)
            .collect();
        self.steps
            .values()
            .filter(|step| runs.contains(&step.run_id))
            .filter_map(|step| step.usage.as_ref()?.get("cost_micros")?.as_i64())
            .fold(None, |total, cost| {
                Some(total.unwrap_or(0).saturating_add(cost))
            })
    }

    /// `project.slug`, or an empty string when the project is not loaded.
    fn project_slug(&self, id: ProjectId) -> String {
        self.projects
            .get(&id)
            .map_or_else(String::new, |project| project.slug.clone())
    }

    /// Switcher rows: every workspace with its projects, ordered by name then by position.
    fn workspace_summaries(&self) -> Vec<WorkspaceSummary> {
        let mut summaries: Vec<WorkspaceSummary> = self
            .workspaces
            .values()
            .map(|workspace| WorkspaceSummary {
                workspace_id: workspace.id,
                slug: workspace.slug.clone(),
                name: workspace.name.clone(),
                projects: self.projects_of(workspace.id),
            })
            .collect();
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        summaries
    }

    /// A workspace's projects, ordered by `workspace_project.position`.
    fn projects_of(&self, workspace_id: WorkspaceId) -> Vec<ProjectRef> {
        let mut memberships: Vec<&WorkspaceProject> = self
            .workspace_projects
            .iter()
            .filter(|member| member.workspace_id == workspace_id)
            .collect();
        memberships.sort_by_key(|member| member.position);
        memberships
            .into_iter()
            .filter_map(|member| {
                let project = self.projects.get(&member.project_id)?;
                Some(ProjectRef {
                    project_id: project.id,
                    slug: project.slug.clone(),
                    name: project.name.clone(),
                    position: member.position,
                })
            })
            .collect()
    }

    /// The scope's projects, ordered by position.
    fn project_refs(&self, scope: &Scope) -> Vec<ProjectRef> {
        self.projects_of(scope.workspace_id)
            .into_iter()
            .filter(|project| scope.contains(project.project_id))
            .collect()
    }

    /// Whether the item is ready in the store-side half of §7.4: open, and no live `blocked_by`
    /// edge to an item that is not terminal. The capability half is the caller's `tags` filter.
    fn is_ready(&self, item: &Item) -> bool {
        item.status == Status::Open
            && !self.links.iter().any(|link| {
                link.deleted_at.is_none()
                    && link.kind == LinkKind::BlockedBy
                    && link.from_item_id == item.id
                    && self
                        .items
                        .get(&link.to_item_id)
                        .is_some_and(|target| !target.status.is_terminal())
            })
    }

    /// Whether the item passes every conjunct of the filter.
    fn matches(&self, item: &Item, filter: &ItemFilter) -> bool {
        if filter
            .project_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(&item.project_id))
        {
            return false;
        }
        if filter
            .statuses
            .as_ref()
            .is_some_and(|statuses| !statuses.contains(&item.status))
        {
            return false;
        }
        if filter
            .tags
            .as_ref()
            .is_some_and(|tags| !tags.iter().all(|tag| item.required_tags.contains(tag)))
        {
            return false;
        }
        if filter.ready.is_some_and(|want| self.is_ready(item) != want) {
            return false;
        }
        if let Some(text) = &filter.text {
            let needle = text.to_lowercase();
            if !item.key.to_lowercase().contains(&needle)
                && !item.title.to_lowercase().contains(&needle)
            {
                return false;
            }
        }
        true
    }

    /// The scope's matching items, ordered by scope position, then key prefix, then key number:
    /// the Backlog list renders this order directly (blueprint B.8).
    fn item_summaries(&self, scope: &Scope, filter: &ItemFilter) -> Vec<ItemSummary> {
        let mut rows: Vec<(usize, &Item)> = self
            .items
            .values()
            .filter_map(|item| {
                let position = scope
                    .project_ids
                    .iter()
                    .position(|project| *project == item.project_id)?;
                self.matches(item, filter).then_some((position, item))
            })
            .collect();
        rows.sort_by(|(left_position, left), (right_position, right)| {
            left_position
                .cmp(right_position)
                .then_with(|| left.key_prefix.cmp(&right.key_prefix))
                .then_with(|| left.key_number.cmp(&right.key_number))
        });
        rows.into_iter().map(|(_, item)| item.summary()).collect()
    }

    /// One traversal node.
    fn link_node(&self, item: &Item, depth: u8) -> LinkNode {
        LinkNode {
            item_id: item.id,
            project_id: item.project_id,
            project_slug: self.project_slug(item.project_id),
            key: item.key.clone(),
            title: item.title.clone(),
            status: item.status,
            depth,
        }
    }

    /// Breadth-first traversal over live edges, followed in both directions and across projects
    /// (§5.5, §7.3). `hops == 0` returns the root alone.
    fn link_graph(&self, root: ItemId, hops: u8) -> Result<LinkGraph> {
        let root_item = self.items.get(&root).ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: root.to_string(),
        })?;

        let mut nodes = vec![self.link_node(root_item, 0)];
        let mut seen = vec![root];
        let mut frontier = vec![root];

        for depth in 1..=hops {
            let mut next: Vec<ItemId> = Vec::new();
            for current in &frontier {
                for link in self.links.iter().filter(|link| link.deleted_at.is_none()) {
                    let other = if link.from_item_id == *current {
                        link.to_item_id
                    } else if link.to_item_id == *current {
                        link.from_item_id
                    } else {
                        continue;
                    };
                    if seen.contains(&other) || next.contains(&other) {
                        continue;
                    }
                    if let Some(item) = self.items.get(&other) {
                        next.push(other);
                        seen.push(other);
                        nodes.push(self.link_node(item, depth));
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }

        let edges = self
            .links
            .iter()
            .filter(|link| {
                link.deleted_at.is_none()
                    && seen.contains(&link.from_item_id)
                    && seen.contains(&link.to_item_id)
            })
            .map(|link| LinkEdge {
                from_item_id: link.from_item_id,
                to_item_id: link.to_item_id,
                kind: link.kind,
            })
            .collect();

        Ok(LinkGraph { root, nodes, edges })
    }

    /// `docs/ANA-9.md` §7.3 as amended by `docs/ANA-5.md` §4.3, walked in memory.
    ///
    /// Four rules, each of which the SQL backends express in their own dialect and all four of
    /// which a reader of [`link_graph`](State::link_graph) above would get wrong by analogy:
    ///
    /// 1. **Directed.** Only `to_item_id` is followed. `link_graph` follows an edge from either
    ///    end and a conformance case pins that, so the two walks cannot share a step.
    /// 2. **Kind-filtered.** `blocked_by` and `origin` only; `relates` and `supersedes` are not
    ///    upstream, they are context.
    /// 3. **`MIN(depth)`.** `seen` is written on first arrival and breadth-first order means first
    ///    arrival is the nearest one, so a diamond renders its apex once, at the shorter depth.
    /// 4. **Canonical order**, by [`UpstreamEntry::sort_canonical`], so this backend and the two
    ///    SQL ones hand the assembler the same bytes.
    ///
    /// `hops` above [`MAX_UPSTREAM_HOPS`] is clamped to it; `hops == 0` returns nothing at all,
    /// which is the one place this differs from `link_graph`'s "hops 0 is the root alone" — the
    /// root is the step's own item and is never an upstream entry. `seen` is seeded with the root
    /// for that second reason as much as for the first: a cycle that returns to the root within
    /// the ceiling must not render it as an entry either.
    ///
    /// `in_scope` and the summary lookup are separate: an out-of-scope item's `summary` is not
    /// read, an in-scope one's is read and may still be `None`.
    fn upstream(&self, root: ItemId, hops: u8, scope: &PromptScope) -> Vec<UpstreamEntry> {
        let hops = hops.min(crate::store::MAX_UPSTREAM_HOPS);
        let mut entries: Vec<UpstreamEntry> = Vec::new();
        let mut seen = vec![root];
        let mut frontier = vec![root];

        for depth in 1..=hops {
            let mut next: Vec<ItemId> = Vec::new();
            for current in &frontier {
                for link in self.links.iter().filter(|link| {
                    link.deleted_at.is_none()
                        && matches!(link.kind, LinkKind::BlockedBy | LinkKind::Origin)
                        && link.from_item_id == *current
                }) {
                    let target = link.to_item_id;
                    if seen.contains(&target) {
                        continue;
                    }
                    seen.push(target);
                    let Some(item) = self.items.get(&target) else {
                        continue;
                    };
                    next.push(target);
                    entries.push(self.upstream_entry(item, depth, scope));
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }

        UpstreamEntry::sort_canonical(&mut entries);
        entries
    }

    /// One reached item classified against the walk's bound (`R-PRM-1`, `R-PRM-2`).
    fn upstream_entry(&self, item: &Item, depth: u8, scope: &PromptScope) -> UpstreamEntry {
        // `R-ENT-2`: with no workspace the bound is the one project, because there is no implicit
        // workspace row to ask.
        let in_scope = scope
            .workspace
            .map_or(item.project_id == scope.project, |ws| {
                self.workspace_projects
                    .iter()
                    .any(|member| member.workspace_id == ws && member.project_id == item.project_id)
            });
        UpstreamEntry {
            item_id: item.id,
            qualified_key: format!("{}:{}", self.project_slug(item.project_id), item.key),
            title: item.title.clone(),
            status: item.status,
            depth,
            in_scope,
            summary: in_scope
                .then(|| {
                    self.latest_document(item.id, "summary")
                        .map(|d| d.body.clone())
                })
                .flatten(),
        }
    }

    /// The highest-`version` `document` of one kind on one item.
    fn latest_document(&self, item: ItemId, kind: &str) -> Option<&Document> {
        self.documents
            .iter()
            .filter(|document| document.item_id == item && document.kind == kind)
            .max_by_key(|document| document.version)
    }

    /// The latest version of each named kind, in `kinds` order; an empty `kinds` means every kind
    /// the item has, in kind **byte** order (blueprint P-12).
    ///
    /// The `all.is_empty()` guard is not a shortcut: without it an item that has **no** documents
    /// resolves the empty `kinds` to an empty kind list and recurses on it forever, which is a
    /// stack overflow in a read path rather than an error a caller can see. `MemStore::demo`'s own
    /// `htui:FEAT-3` is such an item, and `documents_of_kinds(FEAT-3, &[])` is the call that
    /// found it.
    fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Vec<Document> {
        if kinds.is_empty() {
            let mut all: Vec<String> = self
                .documents
                .iter()
                .filter(|document| document.item_id == item)
                .map(|document| document.kind.clone())
                .collect();
            all.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            all.dedup();
            if all.is_empty() {
                return Vec::new();
            }
            return self.documents_of_kinds(item, &all);
        }
        kinds
            .iter()
            .filter_map(|kind| self.latest_document(item, kind).cloned())
            .collect()
    }

    /// The item's documents without their bodies, grouped by kind and ascending by version.
    fn document_heads(&self, id: ItemId) -> Vec<DocumentHead> {
        let mut heads: Vec<DocumentHead> = self
            .documents
            .iter()
            .filter(|document| document.item_id == id)
            .map(Document::head)
            .collect();
        heads.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.version.cmp(&right.version))
        });
        heads
    }

    /// The item's notes, ascending by `created_at`; ties keep insertion order.
    fn item_notes(&self, id: ItemId) -> Vec<Note> {
        let mut notes: Vec<Note> = self
            .notes
            .iter()
            .filter(|note| note.item_id == id)
            .cloned()
            .collect();
        notes.sort_by_key(|note| note.created_at);
        notes
    }

    /// `box.hostname` of the box a run is on: the executing one, else the target (blueprint B.4).
    fn run_hostname(&self, run: &Run) -> String {
        let id = run.executing_box_id.unwrap_or(run.target_box_id);
        self.boxes
            .get(&id)
            .map_or_else(String::new, |row| row.hostname.clone())
    }

    /// A run's steps, ordered by `(position, attempt, fanout_index)`.
    ///
    /// `prompt_tokens` and `trimmed` are [`prompt_summary`]'s projection of `run_step.trim_record`
    /// (plan D106), shared with the two SQL backends so the three cannot drift: the record itself
    /// is a whole JSON document and §6.1 returns neither it nor `prompt_digest`, so these two
    /// fields are the read seam's only trace of a written prompt audit.
    fn run_steps(&self, run: RunId) -> Vec<RunStepSummary> {
        let mut steps: Vec<&RunStep> = self
            .steps
            .values()
            .filter(|step| step.run_id == run)
            .collect();
        steps.sort_by_key(|step| (step.position, step.attempt, step.fanout_index));
        steps
            .into_iter()
            .map(|step| {
                let (prompt_tokens, trimmed) = prompt_summary(step.trim_record.as_ref());
                RunStepSummary {
                    id: step.id,
                    position: step.position,
                    attempt: step.attempt,
                    fanout_index: step.fanout_index,
                    phase_name: step.phase_name.clone(),
                    agent_id: step.agent_id,
                    model: step.model.clone(),
                    status: step.status,
                    gate_outcome: step.gate_outcome,
                    started_at: step.started_at,
                    finished_at: step.finished_at,
                    prompt_tokens,
                    trimmed,
                    usage: step.usage.clone(),
                    selected: step.selected,
                    exit_code: step.exit_code,
                    verify_outcome: step.verify_outcome,
                    promoted_at: step.promoted_at,
                    agent_name: step
                        .agent_id
                        .and_then(|id| self.agents.get(&id))
                        .map(|agent| agent.name.clone()),
                    gate_note: step.gate_note.clone(),
                    opening: self.openings.get(&step.id).copied(),
                }
            })
            .collect()
    }

    /// The item's runs with their steps, newest first (§5.8 `idx_run_item`).
    fn run_summaries(&self, id: ItemId) -> Vec<RunSummary> {
        let mut runs: Vec<&Run> = self
            .runs
            .values()
            .filter(|run| run.item_id == Some(id))
            .collect();
        runs.sort_by_key(|run| std::cmp::Reverse(run.queued_at));
        runs.into_iter()
            .map(|run| RunSummary {
                id: run.id,
                item_id: run.item_id,
                project_id: run.project_id,
                kind: run.kind,
                mode: run.mode,
                status: run.status,
                target_box_id: run.target_box_id,
                executing_box_id: run.executing_box_id,
                box_hostname: self.run_hostname(run),
                queued_at: run.queued_at,
                started_at: run.started_at,
                finished_at: run.finished_at,
                failure: run.failure.clone(),
                steps: self.run_steps(run.id),
            })
            .collect()
    }

    /// The step's replay log ordered by `seq`, or `None` when nothing is cached for it (§7.5).
    fn step_log(&self, step: StepId) -> Option<Vec<SessionEvent>> {
        let mut events: Vec<SessionEvent> = self
            .events
            .iter()
            .filter(|event| event.run_step_id == step)
            .cloned()
            .collect();
        if events.is_empty() {
            return None;
        }
        events.sort_by_key(|event| event.seq);
        Some(events)
    }

    /// MOD-72 plan D3: the item's `tool_call` rows per `(step, tool_kind)`, a kind that is not a
    /// JSON string counted as `other` (blueprint E3), in canonical order.
    fn tool_call_counts(&self, item: ItemId) -> Vec<ToolCallCount> {
        let steps: BTreeSet<StepId> = self
            .steps
            .values()
            .filter(|step| {
                self.runs
                    .get(&step.run_id)
                    .is_some_and(|run| run.item_id == Some(item))
            })
            .map(|step| step.id)
            .collect();
        let mut counts: BTreeMap<(StepId, String), u32> = BTreeMap::new();
        for event in &self.events {
            if event.kind != EventKind::ToolCall || !steps.contains(&event.run_step_id) {
                continue;
            }
            let kind = event
                .payload
                .get("tool_kind")
                .and_then(Value::as_str)
                .unwrap_or("other");
            let calls = counts
                .entry((event.run_step_id, kind.to_owned()))
                .or_insert(0);
            // Blueprint H-5: saturate, never wrap.
            *calls = calls.saturating_add(1);
        }
        let mut rows: Vec<ToolCallCount> = counts
            .into_iter()
            .map(|((step, tool_kind), calls)| ToolCallCount {
                step,
                tool_kind,
                calls,
            })
            .collect();
        ToolCallCount::sort_canonical(&mut rows);
        rows
    }

    /// MOD-69 plan D2: the scope's candidates, grouped and sorted by `WaitingCandidate::assemble`.
    fn waiting_candidates(&self, scope: &Scope) -> Vec<WaitingCandidate> {
        let parked: BTreeSet<ItemId> = self
            .runs
            .values()
            .filter(|run| run.status == RunStatus::AwaitingApproval)
            .filter_map(|run| run.item_id)
            .collect();
        let items: Vec<Item> = self
            .items
            .values()
            .filter(|item| scope.contains(item.project_id))
            .filter(|item| {
                matches!(item.status, Status::Blocked | Status::AwaitingApproval)
                    || parked.contains(&item.id)
            })
            .cloned()
            .collect();
        let wanted: BTreeSet<ItemId> = items.iter().map(|item| item.id).collect();
        let runs: Vec<Run> = self
            .runs
            .values()
            .filter(|run| run.status.is_active())
            .filter(|run| run.item_id.is_some_and(|item| wanted.contains(&item)))
            .cloned()
            .collect();
        let owners: BTreeSet<RunId> = runs.iter().map(|run| run.id).collect();
        let steps: Vec<RunStep> = self
            .steps
            .values()
            .filter(|step| owners.contains(&step.run_id))
            .cloned()
            .collect();
        WaitingCandidate::assemble(scope, items, runs, steps)
    }

    /// Mints an item: counter upsert, key assembly and revision 1, all in one lock (§7.1, §4.1).
    fn mint(&mut self, new: NewItem, now: DateTime<Utc>) -> Result<Item> {
        let kind = self.kinds.get(&new.kind_id).ok_or_else(|| {
            StoreError::Constraint(format!("item_kind `{}` does not exist", new.kind_id))
        })?;
        if kind.project_id != new.project_id {
            return Err(StoreError::Constraint(format!(
                "item_kind `{}` belongs to project `{}`, not `{}`",
                kind.prefix, kind.project_id, new.project_id
            )));
        }
        if self.items.contains_key(&new.id) {
            return Err(StoreError::Constraint(format!(
                "item `{}` already exists",
                new.id
            )));
        }
        // Before the counter moves: a §4.1 key number a refused mint consumed is never given back.
        require_author(new.created_by, "item.created_by")?;

        let prefix = kind.prefix.clone();
        let counter = self
            .item_key_counter
            .entry((new.project_id, prefix.clone()))
            .or_insert(0);
        *counter += 1;
        let key_number = *counter;

        let item = Item {
            id: new.id,
            project_id: new.project_id,
            kind_id: new.kind_id,
            key: format!("{prefix}-{key_number}"),
            key_prefix: prefix,
            key_number,
            title: new.title,
            body: new.body,
            status: Status::Open,
            priority: new.priority,
            required_tags: new.required_tags,
            touched_paths: new.touched_paths,
            step_graph_id: new.step_graph_id,
            version: 1,
            created_by: new.created_by,
            created_at: now,
            updated_at: now,
            closed_at: None,
            resolution: None,
        };

        self.revisions.insert(
            (item.id, 1),
            ItemRevision {
                item_id: item.id,
                version: 1,
                title: item.title.clone(),
                body: item.body.clone(),
                required_tags: item.required_tags.clone(),
                author_id: new.created_by,
                box_id: new.box_id,
                reason: "created".to_owned(),
                created_at: now,
            },
        );
        self.items.insert(item.id, item.clone());
        Ok(item)
    }

    /// Compare-and-set edit on `item.version` over exactly the §4.2 spec columns (§7.2).
    fn update(
        &mut self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
        now: DateTime<Utc>,
    ) -> Result<UpdateOutcome> {
        let item = self
            .items
            .get_mut(&id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            })?;

        if item.version != expected_version {
            let head = item.clone();
            let ancestor = self
                .revisions
                .get(&(id, expected_version))
                .cloned()
                .ok_or_else(|| StoreError::NotFound {
                    entity: "item_revision",
                    id: format!("{id}@{expected_version}"),
                })?;
            return Ok(UpdateOutcome::Diverged { head, ancestor });
        }

        // Everything that can refuse the edit runs here, after the compare-and-set and before the
        // first write: `item` is borrowed mutably, so an `Err` returned mid-apply would leave a
        // half-edited item with no version bump and no revision.
        require_author(patch.author_id, "item_revision.author_id")?;
        if let Some(kind_id) = patch.kind_id {
            let kind = self.kinds.get(&kind_id).ok_or_else(|| {
                StoreError::Constraint(format!("item_kind `{kind_id}` does not exist"))
            })?;
            if kind.project_id != item.project_id {
                return Err(StoreError::Constraint(format!(
                    "item_kind `{}` belongs to project `{}`, not `{}`",
                    kind.prefix, kind.project_id, item.project_id
                )));
            }
            item.kind_id = kind_id;
        }

        if let Some(title) = patch.title {
            item.title = title;
        }
        if let Some(body) = patch.body {
            item.body = body;
        }
        if let Some(required_tags) = patch.required_tags {
            item.required_tags = required_tags;
        }
        if let Some(priority) = patch.priority {
            item.priority = priority;
        }
        if let Some(touched_paths) = patch.touched_paths {
            item.touched_paths = touched_paths;
        }
        if let Some(step_graph_id) = patch.step_graph_id {
            item.step_graph_id = step_graph_id;
        }
        item.version += 1;
        item.updated_at = now;

        let head = item.clone();
        self.revisions.insert(
            (head.id, head.version),
            ItemRevision {
                item_id: head.id,
                version: head.version,
                title: head.title.clone(),
                body: head.body.clone(),
                required_tags: head.required_tags.clone(),
                author_id: patch.author_id,
                box_id: patch.box_id,
                reason: patch.reason,
                created_at: now,
            },
        );
        Ok(UpdateOutcome::Updated(head))
    }

    /// Compare-and-set on `status` alone: never bumps `version`, never writes a revision (§4.2).
    ///
    /// `closed_at` tracks the current status, not the history: it is set on a move to a terminal
    /// status and cleared on a move back to a live one (blueprint Errata).
    ///
    /// The order of MOD-4 plan D14 / D15: the row is looked up first, so a missing row is
    /// `NotFound` even when the pair is also illegal; then the ANA-2 §4.3 table refuses an illegal
    /// pair with `Constraint` **before** any write; only then does a stale `from` answer
    /// `Ok(false)`.
    fn transition(
        &mut self,
        id: ItemId,
        from: Status,
        to: Status,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let item = self
            .items
            .get_mut(&id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            })?;
        legal_move(from, to)?;
        if item.status != from {
            return Ok(false);
        }
        item.status = to;
        item.updated_at = now;
        // `legal_move` refuses every move to `closed` (MOD-38 PRD D1), so of the two terminal
        // statuses only `done` gets here; the Postgres twin's `CASE` names just that one.
        item.closed_at = to.is_terminal().then_some(now);
        Ok(true)
    }

    /// The registry rows, ordered by name, each joined to this box's `agent_box` (MOD-2 plan D3).
    fn agent_summaries(&self) -> Vec<AgentSummary> {
        let mut rows: Vec<&Agent> = self.agents.values().collect();
        rows.sort_by(|left, right| left.name.cmp(&right.name));
        rows.into_iter()
            .map(|agent| AgentSummary {
                agent: agent.clone(),
                on_box: self
                    .this_box
                    .and_then(|box_id| self.agent_boxes.get(&(agent.id, box_id)))
                    .cloned(),
                user_off: self
                    .this_box
                    .is_some_and(|box_id| self.agent_boxes_off.contains(&(agent.id, box_id))),
            })
            .collect()
    }

    /// MOD-40 plan D1: whether `row`'s run carries `fence`'s lease. `lease_owners` holds a run only
    /// while its lease names an owner ([`WriteStore::release_lease`] removes it), which is
    /// Postgres's `lease_owner IS NOT DISTINCT FROM $fence`. A function of the map rather than of
    /// `self`, so a caller can hold `steps` mutably while it asks.
    fn fence_holds(
        lease_owners: &HashMap<RunId, Uuid>,
        row: &RunStep,
        fence: StepFence,
    ) -> Result<()> {
        if lease_owners.get(&row.run_id).copied() == fence.owner() {
            Ok(())
        } else {
            Err(StoreError::Fenced { step: row.id })
        }
    }

    /// Appends events, skipping every `(run_step_id, seq)` already stored, and answers how many
    /// rows landed (§4.3).
    ///
    /// Every event is validated **before** the first insert, because Postgres does the whole batch
    /// in one statement: a batch naming a step that does not exist must write none of its rows.
    /// Existence first, then the fence (MOD-40 plan D1), lowest step first, both before the first
    /// insert.
    fn append_events(&mut self, fence: StepFence, events: &[SessionEvent]) -> Result<usize> {
        for event in events {
            if !self.steps.contains_key(&event.run_step_id) {
                return Err(StoreError::Constraint(format!(
                    "session_event.run_step_id `{}` references no run_step",
                    event.run_step_id
                )));
            }
        }
        // Then the fence, before any row is written; the lowest step first, which is the one
        // Postgres's `fenced` CTE names (MOD-40 blueprint B6).
        let named: BTreeSet<StepId> = events.iter().map(|event| event.run_step_id).collect();
        for step in named {
            if let Some(row) = self.steps.get(&step) {
                Self::fence_holds(&self.lease_owners, row, fence)?;
            }
        }

        let mut inserted = 0;
        for event in events {
            // The primary key is the backstop, in the batch as well as against what is stored.
            if self
                .events
                .iter()
                .any(|row| row.run_step_id == event.run_step_id && row.seq == event.seq)
            {
                continue;
            }
            self.events.push(event.clone());
            inserted += 1;
        }
        Ok(inserted)
    }

    /// `run_step.usage`, plus `prompt_digest` when one is supplied (`docs/ANA-4.md` §4.1).
    fn set_step_usage(
        &mut self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        Self::fence_holds(&self.lease_owners, row, fence)?;
        row.usage = Some(usage);
        if let Some(digest) = prompt_digest {
            row.prompt_digest = Some(digest);
        }
        row.updated_at = now;
        Ok(())
    }

    /// `run_step.prompt_digest` **and** `run_step.trim_record`, both, and nothing else
    /// (`docs/ANA-5.md` §4.4): the pre-flight audit the assembler writes before a session starts.
    ///
    /// Unconditional where [`set_step_usage`](State::set_step_usage)'s digest write is
    /// conditional: that one takes an `Option` because the chat path has no digest to offer on
    /// most calls (plan D97), while a caller of this one has assembled a prompt and always has
    /// both values.
    fn set_step_prompt(
        &mut self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        // MOD-41 plan D1: existence, then the fence, then the write.
        Self::fence_holds(&self.lease_owners, row, fence)?;
        row.prompt_digest = Some(digest.to_owned());
        row.trim_record = Some(trim.clone());
        row.updated_at = now;
        Ok(())
    }

    /// Creates or edits one agent under compare-and-set on `updated_at` (MOD-40 plan D5), in
    /// Postgres's order (blueprint F-18): the id and the token, then the name, then the write.
    ///
    /// `None` inserts the row as given, stamps included, and is `Stale` on a stored id. `Some(t)`
    /// writes every column but the stamps where the stored `updated_at` is `t`: `created_at` is
    /// the stored row's and `updated_at` is the clock, which is what Postgres's `BEFORE UPDATE`
    /// trigger does, written out.
    fn upsert_agent(
        &mut self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Agent>> {
        match (self.agents.get(&agent.id), expected) {
            (Some(stored), None) => return Ok(CasOutcome::Stale(stored.clone())),
            (Some(stored), Some(token)) if stored.updated_at != token => {
                return Ok(CasOutcome::Stale(stored.clone()));
            }
            (None, Some(_)) => {
                return Err(StoreError::NotFound {
                    entity: "agent",
                    id: agent.id.to_string(),
                });
            }
            (None, None) | (Some(_), Some(_)) => {}
        }
        if self
            .agents
            .values()
            .any(|row| row.id != agent.id && row.name == agent.name)
        {
            return Err(StoreError::Constraint(format!(
                "agent_name_key: another agent is already named `{}`",
                agent.name
            )));
        }
        let row = match self.agents.get(&agent.id) {
            Some(stored) => Agent {
                created_at: stored.created_at,
                updated_at: now,
                ..agent.clone()
            },
            None => agent.clone(),
        };
        self.agents.insert(agent.id, row.clone());
        Ok(CasOutcome::Applied(row))
    }

    /// Insert-or-update on the composite primary key `(agent_id, box_id)`, both referents required
    /// (§5.7).
    ///
    /// `quota` and `quota_at` are written by neither branch (MOD-2 plan D74): the insert forces
    /// them to `None`, which is the two columns the `INSERT` list no longer names, and the update
    /// puts the stored pair back, which is the `SET quota = EXCLUDED.quota` the statement no longer
    /// carries. The two backends have to agree here or the conformance case passes on one and
    /// fails on the other.
    ///
    /// On the update, `enabled` is ANDed with the per-box switch (MOD-23 D242), which is
    /// `SET enabled = EXCLUDED.enabled AND NOT user_off`: a row the human switched off on this box
    /// stays off whatever the probe proposes. The insert is unchanged, since a fresh row has no
    /// switch.
    fn upsert_agent_box(&mut self, row: &AgentBox, now: DateTime<Utc>) -> Result<()> {
        if !self.agents.contains_key(&row.agent_id) {
            return Err(StoreError::Constraint(format!(
                "agent_box.agent_id `{}` references no agent",
                row.agent_id
            )));
        }
        if !self.boxes.contains_key(&row.box_id) {
            return Err(StoreError::Constraint(format!(
                "agent_box.box_id `{}` references no box",
                row.box_id
            )));
        }
        let key = (row.agent_id, row.box_id);
        // Read before `get_mut` borrows `agent_boxes`: MOD-23 D242's veto.
        let switched_off = self.agent_boxes_off.contains(&key);
        match self.agent_boxes.get_mut(&key) {
            Some(stored) => {
                // D74: `*stored = row.clone()` is this backend's spelling of
                // `SET quota = EXCLUDED.quota`, so the stored pair is taken out and put back.
                let (quota, quota_at) = (stored.quota.clone(), stored.quota_at);
                *stored = row.clone();
                stored.quota = quota;
                stored.quota_at = quota_at;
                stored.enabled = row.enabled && !switched_off;
                stored.updated_at = now;
            }
            None => {
                // The two columns the `INSERT` list does not name.
                let fresh = AgentBox {
                    quota: None,
                    quota_at: None,
                    ..row.clone()
                };
                self.agent_boxes.insert(key, fresh);
            }
        }
        Ok(())
    }

    /// The two-column quota latch of `docs/ANA-4.md` §7 (plan D67): an existing row only, with
    /// `updated_at` bumped as `set_step_usage` bumps it and Postgres's `BEFORE UPDATE` trigger
    /// does it there, and newest `quota_at` wins (MOD-40 plan D4): an older one is `Ok(false)` and
    /// writes nothing, not even `updated_at`.
    fn set_agent_box_quota(
        &mut self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let row = self
            .agent_boxes
            .get_mut(&(agent_id, box_id))
            .ok_or_else(|| StoreError::NotFound {
                entity: "agent_box",
                id: format!("{agent_id}/{box_id}"),
            })?;
        if row.quota_at.is_some_and(|stored| stored > quota_at) {
            return Ok(false);
        }
        row.quota = Some(quota);
        row.quota_at = Some(quota_at);
        row.updated_at = now;
        Ok(true)
    }

    /// The per-box switch (MOD-23 D242): Postgres's one statement, written out. The referents in
    /// `upsert_agent_box`'s order and with its sentences; then an absent row is inserted bare, and
    /// a present one gets `enabled` (switched off) or the stored probe's verdict (switched on), with
    /// `updated_at` bumped as the trigger does it there.
    fn set_agent_box_enabled(
        &mut self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if !self.agents.contains_key(&agent_id) {
            return Err(StoreError::Constraint(format!(
                "agent_box.agent_id `{agent_id}` references no agent"
            )));
        }
        if !self.boxes.contains_key(&box_id) {
            return Err(StoreError::Constraint(format!(
                "agent_box.box_id `{box_id}` references no box"
            )));
        }
        let key = (agent_id, box_id);
        if enabled {
            self.agent_boxes_off.remove(&key);
        } else {
            self.agent_boxes_off.insert(key);
        }
        match self.agent_boxes.get_mut(&key) {
            Some(row) => {
                row.enabled = enabled && probe_says_ready(row.probe.as_ref());
                row.updated_at = now;
            }
            None => {
                // The column defaults of Postgres's two-column `INSERT`: never probed.
                self.agent_boxes.insert(
                    key,
                    AgentBox {
                        agent_id,
                        box_id,
                        enabled,
                        version: None,
                        path: None,
                        probed_at: None,
                        quota: None,
                        quota_at: None,
                        updated_at: now,
                        probe: None,
                    },
                );
            }
        }
        Ok(())
    }

    /// One box probe (MOD-7 D10): the nine probe columns, the whole `box_tool` set and the spec
    /// digest, or nothing. The checks run before any write, so a refusal leaves the store as it
    /// stood, as `PgStore`'s transaction rolls back.
    ///
    /// The box is looked up first, then the digest, then the tool names: `PgStore`'s
    /// `UPDATE .. WHERE id` answers an unknown box before any `CHECK` or key is consulted, so
    /// `NotFound` wins over every `Constraint` here too.
    fn record_box_probe(&mut self, probe: &BoxProbe, now: DateTime<Utc>) -> Result<()> {
        let id = probe.box_id;
        if !self.boxes.contains_key(&id) {
            return Err(StoreError::NotFound {
                entity: "box",
                id: id.to_string(),
            });
        }
        let digest = &probe.spec_digest;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(StoreError::Constraint(format!(
                "box.probe_spec_digest `{digest}` is not a lowercase sha256 hex digest"
            )));
        }
        let mut names = HashSet::new();
        if let Some(name) = probe
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .find(|name| !names.insert(*name))
        {
            return Err(StoreError::Constraint(format!(
                "box_tool `{name}` is listed twice for box `{}`",
                probe.box_id
            )));
        }
        let row = self
            .boxes
            .get_mut(&id)
            .expect("the box was looked up above, under the same lock");
        row.os_version.clone_from(&probe.os_version);
        row.cpu.clone_from(&probe.cpu);
        row.ram_mb = probe.ram_mb;
        row.gpu_present = probe.gpu_present;
        row.gpu_vendor.clone_from(&probe.gpu_vendor);
        row.probed_tags.clone_from(&probe.probed_tags);
        row.htui_version.clone_from(&probe.htui_version);
        row.last_probed_at = Some(probe.probed_at);
        row.updated_at = now;
        self.box_tools.retain(|tool| tool.box_id != id);
        self.box_tools
            .extend(probe.tools.iter().map(|tool| BoxTool {
                box_id: id,
                name: tool.name.clone(),
                version: tool.version.clone(),
                path: tool.path.clone(),
                probed_at: probe.probed_at,
            }));
        self.box_probe_digests.insert(id, digest.clone());
        Ok(())
    }

    /// Every box of `user` with its tools and recorded digest (MOD-7 D10, D18): boxes by id,
    /// tools by name bytes, the order `PgStore`'s `COLLATE "C"` gives. `None` (an empty store) has
    /// no boxes.
    fn box_records(&self, user: Option<UserId>) -> Vec<BoxRecord> {
        let Some(user) = user else {
            return Vec::new();
        };
        let mut rows: Vec<&BoxRow> = self
            .boxes
            .values()
            .filter(|row| row.user_id == user)
            .collect();
        rows.sort_by_key(|row| row.id);
        rows.into_iter()
            .map(|row| {
                let mut tools: Vec<BoxTool> = self
                    .box_tools
                    .iter()
                    .filter(|tool| tool.box_id == row.id)
                    .cloned()
                    .collect();
                tools.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
                BoxRecord {
                    row: row.clone(),
                    tools,
                    probe_spec_digest: self.box_probe_digests.get(&row.id).cloned(),
                }
            })
            .collect()
    }

    /// `edit_version` compare-and-set on one box of `user` (MOD-7 D41): `NotFound` for an unknown
    /// id or another user's box, then `Stale` for a spent token, then `Constraint` for a tag, an
    /// unknown executor or a non-object `settings` blob (MOD-41 plan D10), and only then the
    /// write. The executor is written as one key of the blob, so every other key survives. `now`
    /// stands in for Postgres's `set_updated_at` trigger.
    fn edit_box(
        &mut self,
        user: Option<UserId>,
        id: BoxId,
        expected: i32,
        edit: BoxEdit,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<BoxRow>> {
        let Some(row) = self.boxes.get(&id).filter(|row| Some(row.user_id) == user) else {
            return Err(StoreError::NotFound {
                entity: "box",
                id: id.to_string(),
            });
        };
        if row.edit_version != expected {
            return Ok(CasOutcome::Stale(row.clone()));
        }
        let tags = edit
            .declared_tags
            .as_deref()
            .map(canonical_declared_tags)
            .transpose()
            .map_err(StoreError::Constraint)?;
        if matches!(edit.executor, Some(Executor::Other(_))) {
            return Err(StoreError::Constraint(EXECUTOR_MUST_BE_KNOWN.to_owned()));
        }
        if edit.executor.is_some() && !row.settings.is_object() {
            return Err(StoreError::Constraint(
                BOX_SETTINGS_NOT_AN_OBJECT.to_owned(),
            ));
        }
        let row = self
            .boxes
            .get_mut(&id)
            .expect("the box was looked up above, under the same lock");
        if let Some(tags) = tags {
            row.declared_tags = tags;
        }
        if let Some(quirks) = edit.quirks {
            row.quirks = quirks;
        }
        if let Some(executor) = edit.executor {
            row.settings.as_object_mut().expect("checked above").insert(
                Executor::KEY.to_owned(),
                Value::String(executor.as_str().to_owned()),
            );
        }
        row.edit_version += 1;
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// The `run` / `run_step` pair of a free-standing chat, both a no-op when the id is already
    /// stored (plan D4: `ON CONFLICT (id) DO NOTHING`).
    fn start_chat_run(&mut self, chat: &ChatRunSpec) -> Result<()> {
        if !self.projects.contains_key(&chat.project_id) {
            return Err(StoreError::Constraint(format!(
                "run.project_id `{}` references no project",
                chat.project_id
            )));
        }
        if !self.boxes.contains_key(&chat.target_box_id) {
            return Err(StoreError::Constraint(format!(
                "run.target_box_id `{}` references no box",
                chat.target_box_id
            )));
        }
        require_author(chat.started_by, "run.started_by")?;
        if let Some(agent) = chat.agent_id
            && !self.agents.contains_key(&agent)
        {
            return Err(StoreError::Constraint(format!(
                "run_step.agent_id `{agent}` references no agent"
            )));
        }

        self.runs.entry(chat.run_id).or_insert_with(|| Run {
            id: chat.run_id,
            project_id: chat.project_id,
            item_id: None,
            kind: RunKind::Chat,
            mode: RunMode::Manual,
            status: RunStatus::Running,
            target_box_id: chat.target_box_id,
            executing_box_id: Some(chat.target_box_id),
            graph_snapshot: None,
            started_by: chat.started_by,
            queued_at: chat.started_at,
            started_at: Some(chat.started_at),
            finished_at: None,
            failure: None,
            repo_scope: Vec::new(),
            lease_box_id: None,
            lease_expires_at: None,
            updated_at: chat.started_at,
        });
        self.steps.entry(chat.step_id).or_insert_with(|| RunStep {
            id: chat.step_id,
            run_id: chat.run_id,
            position: 0,
            attempt: 1,
            fanout_index: 0,
            phase_name: chat.phase_name.clone(),
            agent_id: chat.agent_id,
            model: chat.model.clone(),
            status: StepStatus::Running,
            gate_outcome: None,
            gate_note: None,
            selected: None,
            exit_code: None,
            prompt_digest: None,
            trim_record: None,
            usage: None,
            isolation_path: None,
            started_at: Some(chat.started_at),
            finished_at: None,
            verify_outcome: None,
            verify_exit_code: None,
            promoted_at: None,
            updated_at: chat.started_at,
        });
        Ok(())
    }

    /// Closes both rows of a chat run (plan D4).
    fn finish_chat_run(
        &mut self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let step_status = chat_step_status(status)
            .ok_or_else(|| StoreError::Constraint(not_a_terminal_status(status)))?;
        if !self.runs.contains_key(&run) {
            return Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            });
        }
        if !self.steps.contains_key(&step) {
            return Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            });
        }
        if let Some(row) = self.runs.get_mut(&run) {
            row.status = status;
            row.finished_at = Some(finished_at);
            row.updated_at = now;
        }
        if let Some(row) = self.steps.get_mut(&step) {
            row.status = step_status;
            row.finished_at = Some(finished_at);
            row.updated_at = now;
        }
        Ok(())
    }

    // ---- MOD-15 milestone 1: the hierarchy (plan D1-D12) -------------------------------------
    //
    // The rules live here rather than in the trait arms below, for the reason `mint` and `update`
    // do: an arm is one line that takes the lock, so nothing that can refuse a write is spelled
    // twice, and `delete_reach` and `delete_project` share the one function that counts
    // (`project_reach`) rather than two that could drift apart.

    /// One phase row by id; `step_graph_phase` is a `Vec` because nothing looks it up by anything
    /// but `graph_id` and `position`.
    fn phase(&self, id: PhaseId) -> Option<&StepGraphPhase> {
        self.phases.iter().find(|row| row.id == id)
    }

    /// [`State::phase`] for a writer.
    fn phase_mut(&mut self, id: PhaseId) -> Option<&mut StepGraphPhase> {
        self.phases.iter_mut().find(|row| row.id == id)
    }

    /// The `app_user` a `created_by` column must reference; the FK's half of `require_author`.
    fn require_user(&self, id: UserId, column: &str) -> Result<()> {
        require_author(id, column)?;
        if self.users.contains_key(&id) {
            return Ok(());
        }
        Err(StoreError::Constraint(format!(
            "{column} `{id}` references no app_user"
        )))
    }

    fn create_workspace(&mut self, new: NewWorkspace, now: DateTime<Utc>) -> Result<Workspace> {
        self.require_user(new.created_by, "workspace.created_by")?;
        if self.workspaces.contains_key(&new.id) {
            return Err(StoreError::Constraint(format!(
                "workspace `{}` already exists",
                new.id
            )));
        }
        if self.workspaces.values().any(|row| row.slug == new.slug) {
            return Err(StoreError::Constraint(format!(
                "workspace.slug `{}` is taken",
                new.slug
            )));
        }
        let row = Workspace {
            id: new.id,
            slug: new.slug,
            name: new.name,
            description: new.description,
            created_by: new.created_by,
            created_at: now,
            updated_at: now,
        };
        self.workspaces.insert(row.id, row.clone());
        Ok(row)
    }

    /// Compare-and-set on `workspace.updated_at` (D3).
    fn update_workspace(
        &mut self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Workspace>> {
        let current = self
            .workspaces
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "workspace",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        if let Some(slug) = &patch.slug
            && self
                .workspaces
                .values()
                .any(|row| row.id != id && row.slug == *slug)
        {
            return Err(StoreError::Constraint(format!(
                "workspace.slug `{slug}` is taken"
            )));
        }
        let row = self
            .workspaces
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(slug) = patch.slug {
            row.slug = slug;
        }
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(description) = patch.description {
            row.description = description;
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// Inserts or repositions a link; the PK is `(workspace_id, project_id)` and there is no
    /// `updated_at` to compare (D3).
    fn upsert_workspace_project(&mut self, link: &WorkspaceProject) -> Result<()> {
        if !self.workspaces.contains_key(&link.workspace_id) {
            return Err(StoreError::Constraint(format!(
                "workspace_project.workspace_id `{}` references no workspace",
                link.workspace_id
            )));
        }
        if !self.projects.contains_key(&link.project_id) {
            return Err(StoreError::Constraint(format!(
                "workspace_project.project_id `{}` references no project",
                link.project_id
            )));
        }
        match self
            .workspace_projects
            .iter_mut()
            .find(|row| row.workspace_id == link.workspace_id && row.project_id == link.project_id)
        {
            Some(row) => row.position = link.position,
            None => self.workspace_projects.push(link.clone()),
        }
        Ok(())
    }

    /// Removes one link. The project survives it (D4).
    fn remove_workspace_project(
        &mut self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> Result<()> {
        let before = self.workspace_projects.len();
        self.workspace_projects
            .retain(|row| !(row.workspace_id == workspace && row.project_id == project));
        if self.workspace_projects.len() == before {
            return Err(StoreError::NotFound {
                entity: "workspace_project",
                id: format!("{workspace}/{project}"),
            });
        }
        Ok(())
    }

    /// A workspace's links, ordered by `position` then `project_id` bytes.
    fn workspace_project_rows(&self, workspace: WorkspaceId) -> Vec<WorkspaceProject> {
        let mut rows: Vec<WorkspaceProject> = self
            .workspace_projects
            .iter()
            .filter(|row| row.workspace_id == workspace)
            .cloned()
            .collect();
        rows.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.project_id.cmp(&right.project_id))
        });
        rows
    }

    /// Inserts or replaces this box's root path; one writer per `(workspace_id, box_id)`, so the
    /// replace needs no token either (`R-BOX-4`).
    fn upsert_workspace_box_path(
        &mut self,
        path: &WorkspaceBoxPath,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if !self.workspaces.contains_key(&path.workspace_id) {
            return Err(StoreError::Constraint(format!(
                "workspace_box_path.workspace_id `{}` references no workspace",
                path.workspace_id
            )));
        }
        if !self.boxes.contains_key(&path.box_id) {
            return Err(StoreError::Constraint(format!(
                "workspace_box_path.box_id `{}` references no box",
                path.box_id
            )));
        }
        let mut row = path.clone();
        row.updated_at = now;
        match self
            .workspace_box_paths
            .iter_mut()
            .find(|held| held.workspace_id == path.workspace_id && held.box_id == path.box_id)
        {
            Some(held) => *held = row,
            None => self.workspace_box_paths.push(row),
        }
        Ok(())
    }

    /// Every box's root path for a workspace, ordered by `box_id` bytes.
    fn workspace_box_path_rows(&self, workspace: WorkspaceId) -> Vec<WorkspaceBoxPath> {
        let mut rows: Vec<WorkspaceBoxPath> = self
            .workspace_box_paths
            .iter()
            .filter(|row| row.workspace_id == workspace)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.box_id);
        rows
    }

    /// A project with `settings = {}` and no secret provider (M1 D9), plus the thirty-five rows
    /// `seed` gives every project — five graphs, fifteen phases, five kinds, ten templates (M2
    /// D4).
    ///
    /// Validate, then mutate: `require_user`, the duplicate id and the duplicate slug all fire
    /// before the first insert, and nothing after it can fail — the rows come from `seed::KINDS`
    /// and `DEFAULT_TEMPLATES`, whose self-consistency is the `seed` module's unit tests' to
    /// prove, not this fn's to re-check (D5). None of `create_step_graph`, `create_phase` or
    /// `create_item_kind` is called: each validates against the maps and a refusal midway would
    /// leave a half-seeded project. `item_key_counter` is untouched; `mint` creates the row
    /// lazily. `settings` stays `{}`: `set_setting` is that column's only writer.
    fn create_project(&mut self, new: NewProject, now: DateTime<Utc>) -> Result<Project> {
        self.require_user(new.created_by, "project.created_by")?;
        if self.projects.contains_key(&new.id) {
            return Err(StoreError::Constraint(format!(
                "project `{}` already exists",
                new.id
            )));
        }
        if self.projects.values().any(|row| row.slug == new.slug) {
            return Err(StoreError::Constraint(format!(
                "project.slug `{}` is taken",
                new.slug
            )));
        }

        let created_by = new.created_by;
        let row = Project {
            id: new.id,
            slug: new.slug,
            name: new.name,
            description: new.description,
            secret_provider: None,
            secret_scope: None,
            settings: Value::Object(serde_json::Map::new()),
            created_by,
            created_at: now,
            updated_at: now,
        };
        let project_id = row.id;
        self.projects.insert(project_id, row.clone());

        for (position, kind) in seed::KINDS.iter().enumerate() {
            let graph = seed::graph_row(StepGraphId::new(), project_id, kind, now);
            let graph_id = graph.id;
            self.graphs.insert(graph_id, graph);
            for (phase_position, phase) in kind.phases.iter().enumerate() {
                self.phases.push(seed::phase_row(
                    PhaseId::new(),
                    graph_id,
                    phase_position as i32,
                    phase,
                    now,
                ));
            }
            let kind_row = seed::kind_row(
                ItemKindId::new(),
                project_id,
                graph_id,
                position as i32,
                kind,
                now,
            );
            self.kinds.insert(kind_row.id, kind_row);
        }
        for (name, _, body) in &DEFAULT_TEMPLATES {
            self.templates.push(seed::template_row(
                PromptTemplateId::new(),
                project_id,
                name,
                body,
                created_by,
                now,
            ));
        }

        Ok(row)
    }

    /// Compare-and-set on `project.updated_at`; `settings` is not this writer's (D8).
    fn update_project(
        &mut self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Project>> {
        let current = self
            .projects
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "project",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        if let Some(slug) = &patch.slug
            && self
                .projects
                .values()
                .any(|row| row.id != id && row.slug == *slug)
        {
            return Err(StoreError::Constraint(format!(
                "project.slug `{slug}` is taken"
            )));
        }
        let row = self
            .projects
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(slug) = patch.slug {
            row.slug = slug;
        }
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(description) = patch.description {
            row.description = description;
        }
        if let Some(secret) = patch.secret {
            (row.secret_provider, row.secret_scope) = match secret {
                Some(scope) => (
                    Some(crate::secret::INFISICAL.to_owned()),
                    Some(scope.to_column()),
                ),
                None => (None, None),
            };
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// Clears the project's current primary repo, `except` the row being written (D10).
    fn demote_primary_repos(&mut self, project: ProjectId, except: RepoId, now: DateTime<Utc>) {
        for row in self
            .repos
            .values_mut()
            .filter(|row| row.id != except && row.project_id == project && row.is_primary)
        {
            row.is_primary = false;
            row.updated_at = now;
        }
    }

    fn create_repo(&mut self, new: NewRepo, now: DateTime<Utc>) -> Result<Repo> {
        if !self.projects.contains_key(&new.project_id) {
            return Err(StoreError::Constraint(format!(
                "repo.project_id `{}` references no project",
                new.project_id
            )));
        }
        if self.repos.contains_key(&new.id) {
            return Err(StoreError::Constraint(format!(
                "repo `{}` already exists",
                new.id
            )));
        }
        if self
            .repos
            .values()
            .any(|row| row.project_id == new.project_id && row.name == new.name)
        {
            return Err(StoreError::Constraint(format!(
                "repo.name `{}` is taken in project `{}`",
                new.name, new.project_id
            )));
        }
        if new.is_primary {
            self.demote_primary_repos(new.project_id, new.id, now);
        }
        let row = Repo {
            id: new.id,
            project_id: new.project_id,
            name: new.name,
            remote_url: new.remote_url,
            default_branch: new.default_branch,
            is_primary: new.is_primary,
            created_at: now,
            updated_at: now,
        };
        self.repos.insert(row.id, row.clone());
        Ok(row)
    }

    /// Compare-and-set on `repo.updated_at`; promoting this row demotes the other primary in the
    /// same lock, so `uq_repo_primary` is never momentarily violated (D10).
    fn update_repo(
        &mut self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Repo>> {
        let current = self
            .repos
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "repo",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        if let Some(name) = &patch.name
            && self.repos.values().any(|row| {
                row.id != id && row.project_id == current.project_id && row.name == *name
            })
        {
            return Err(StoreError::Constraint(format!(
                "repo.name `{name}` is taken in project `{}`",
                current.project_id
            )));
        }
        if patch.is_primary == Some(true) {
            self.demote_primary_repos(current.project_id, id, now);
        }
        let row = self
            .repos
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(remote_url) = patch.remote_url {
            row.remote_url = remote_url;
        }
        if let Some(default_branch) = patch.default_branch {
            row.default_branch = default_branch;
        }
        if let Some(is_primary) = patch.is_primary {
            row.is_primary = is_primary;
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// A project's repos, ordered by `name` bytes.
    fn repo_rows(&self, project: ProjectId) -> Vec<Repo> {
        let mut rows: Vec<Repo> = self
            .repos
            .values()
            .filter(|row| row.project_id == project)
            .cloned()
            .collect();
        rows.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        rows
    }

    /// The two foreign keys of `repo_box_path`, repo first, then box: the checks both writers
    /// share, so their sentences cannot drift.
    fn check_repo_box_path_ids(&self, path: &RepoBoxPath) -> Result<()> {
        if !self.repos.contains_key(&path.repo_id) {
            return Err(StoreError::Constraint(format!(
                "repo_box_path.repo_id `{}` references no repo",
                path.repo_id
            )));
        }
        if !self.boxes.contains_key(&path.box_id) {
            return Err(StoreError::Constraint(format!(
                "repo_box_path.box_id `{}` references no box",
                path.box_id
            )));
        }
        Ok(())
    }

    /// Inserts or replaces this box's checkout path (`R-BOX-4`).
    fn upsert_repo_box_path(&mut self, path: &RepoBoxPath, now: DateTime<Utc>) -> Result<()> {
        self.check_repo_box_path_ids(path)?;
        let mut row = path.clone();
        row.updated_at = now;
        match self
            .repo_box_paths
            .iter_mut()
            .find(|held| held.repo_id == path.repo_id && held.box_id == path.box_id)
        {
            Some(held) => *held = row,
            None => self.repo_box_paths.push(row),
        }
        Ok(())
    }

    /// `WriteStore::infer_repo_box_path`'s twin (MOD-7 milestone 4, D104): the same two existence
    /// checks as `upsert_repo_box_path`, in the same order and with the same sentences, then a push
    /// only when no row holds `(repo_id, box_id)`. Answers whether it pushed.
    fn infer_repo_box_path(&mut self, path: &RepoBoxPath, now: DateTime<Utc>) -> Result<bool> {
        self.check_repo_box_path_ids(path)?;
        if self
            .repo_box_paths
            .iter()
            .any(|held| held.repo_id == path.repo_id && held.box_id == path.box_id)
        {
            return Ok(false);
        }
        let mut row = path.clone();
        row.updated_at = now;
        self.repo_box_paths.push(row);
        Ok(true)
    }

    /// Every box's checkout path for a repo, ordered by `box_id` bytes.
    fn repo_box_path_rows(&self, repo: RepoId) -> Vec<RepoBoxPath> {
        let mut rows: Vec<RepoBoxPath> = self
            .repo_box_paths
            .iter()
            .filter(|row| row.repo_id == repo)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.box_id);
        rows
    }

    /// The three `item_kind` rules the schema cannot express on its own (D11): the prefix CHECK in
    /// words, `(project, prefix)` and `(project, name)` uniqueness, and a `default_graph_id` that
    /// belongs to the kind's own project.
    fn check_item_kind(
        &self,
        project: ProjectId,
        prefix: &str,
        name: &str,
        graph: StepGraphId,
        except: Option<ItemKindId>,
    ) -> Result<()> {
        if !ItemKind::prefix_is_valid(prefix) {
            return Err(StoreError::Constraint(invalid_prefix(prefix)));
        }
        if !self.projects.contains_key(&project) {
            return Err(StoreError::Constraint(format!(
                "item_kind.project_id `{project}` references no project"
            )));
        }
        if self
            .graphs
            .get(&graph)
            .is_none_or(|row| row.project_id != project)
        {
            return Err(StoreError::Constraint(graph_not_in_project(graph, project)));
        }
        let clashes = |taken: &dyn Fn(&ItemKind) -> bool| {
            self.kinds
                .values()
                .any(|row| row.project_id == project && Some(row.id) != except && taken(row))
        };
        if clashes(&|row| row.prefix == prefix) {
            return Err(StoreError::Constraint(format!(
                "item_kind.prefix `{prefix}` is taken in project `{project}`"
            )));
        }
        if clashes(&|row| row.name == name) {
            return Err(StoreError::Constraint(format!(
                "item_kind.name `{name}` is taken in project `{project}`"
            )));
        }
        Ok(())
    }

    fn create_item_kind(&mut self, new: NewItemKind, now: DateTime<Utc>) -> Result<ItemKind> {
        self.check_item_kind(
            new.project_id,
            &new.prefix,
            &new.name,
            new.default_graph_id,
            None,
        )?;
        if self.kinds.contains_key(&new.id) {
            return Err(StoreError::Constraint(format!(
                "item_kind `{}` already exists",
                new.id
            )));
        }
        let row = ItemKind {
            id: new.id,
            project_id: new.project_id,
            prefix: new.prefix,
            name: new.name,
            description: new.description,
            default_graph_id: new.default_graph_id,
            position: new.position,
            updated_at: now,
        };
        self.kinds.insert(row.id, row.clone());
        Ok(row)
    }

    /// Compare-and-set on `item_kind.updated_at`. A prefix rename touches no `item` and no
    /// `item_key_counter`: the counter is keyed by `(project, prefix)`, so the next mint under the
    /// kind starts the new prefix at 1 and the old keys keep their text (PRD D12).
    fn update_item_kind(
        &mut self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<ItemKind>> {
        let current = self
            .kinds
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "item_kind",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        self.check_item_kind(
            current.project_id,
            patch.prefix.as_deref().unwrap_or(&current.prefix),
            patch.name.as_deref().unwrap_or(&current.name),
            patch.default_graph_id.unwrap_or(current.default_graph_id),
            Some(id),
        )?;
        let row = self
            .kinds
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(prefix) = patch.prefix {
            row.prefix = prefix;
        }
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(description) = patch.description {
            row.description = description;
        }
        if let Some(graph) = patch.default_graph_id {
            row.default_graph_id = graph;
        }
        if let Some(position) = patch.position {
            row.position = position;
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// A project's kinds, ordered by `position` then `prefix` bytes; `position` is not unique.
    fn item_kind_rows(&self, project: ProjectId) -> Vec<ItemKind> {
        let mut rows: Vec<ItemKind> = self
            .kinds
            .values()
            .filter(|row| row.project_id == project)
            .cloned()
            .collect();
        rows.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.prefix.as_bytes().cmp(right.prefix.as_bytes()))
        });
        rows
    }

    /// Deletes a kind nothing references, and names the count when something does (D6).
    fn delete_item_kind(&mut self, id: ItemKindId) -> Result<()> {
        let kind = self.kinds.get(&id).ok_or_else(|| StoreError::NotFound {
            entity: "item_kind",
            id: id.to_string(),
        })?;
        let held = rows(self.items.values().filter(|row| row.kind_id == id).count());
        if held > 0 {
            return Err(StoreError::Constraint(item_kind_is_held(
                &kind.prefix,
                held,
            )));
        }
        self.kinds.remove(&id);
        Ok(())
    }

    fn create_step_graph(&mut self, new: NewStepGraph, now: DateTime<Utc>) -> Result<StepGraph> {
        if !self.projects.contains_key(&new.project_id) {
            return Err(StoreError::Constraint(format!(
                "step_graph.project_id `{}` references no project",
                new.project_id
            )));
        }
        if self.graphs.contains_key(&new.id) {
            return Err(StoreError::Constraint(format!(
                "step_graph `{}` already exists",
                new.id
            )));
        }
        if self
            .graphs
            .values()
            .any(|row| row.project_id == new.project_id && row.name == new.name)
        {
            return Err(StoreError::Constraint(format!(
                "step_graph.name `{}` is taken in project `{}`",
                new.name, new.project_id
            )));
        }
        let row = StepGraph {
            id: new.id,
            project_id: new.project_id,
            name: new.name,
            description: new.description,
            is_override: new.is_override,
            created_at: now,
            updated_at: now,
        };
        self.graphs.insert(row.id, row.clone());
        Ok(row)
    }

    /// Compare-and-set on `step_graph.updated_at`.
    fn update_step_graph(
        &mut self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<StepGraph>> {
        let current = self
            .graphs
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "step_graph",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        if let Some(name) = &patch.name
            && self.graphs.values().any(|row| {
                row.id != id && row.project_id == current.project_id && row.name == *name
            })
        {
            return Err(StoreError::Constraint(format!(
                "step_graph.name `{name}` is taken in project `{}`",
                current.project_id
            )));
        }
        let row = self
            .graphs
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(description) = patch.description {
            row.description = description;
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// A project's graphs, ordered by `name` bytes.
    fn step_graph_rows(&self, project: ProjectId) -> Vec<StepGraph> {
        let mut rows: Vec<StepGraph> = self
            .graphs
            .values()
            .filter(|row| row.project_id == project)
            .cloned()
            .collect();
        rows.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        rows
    }

    /// The reserved-name rule and the two uniqueness rules of `step_graph_phase` (D11).
    fn check_phase(
        &self,
        graph: StepGraphId,
        position: i32,
        name: &str,
        except: Option<PhaseId>,
    ) -> Result<()> {
        if TemplateRole::of_name(name) != TemplateRole::Phase {
            return Err(StoreError::Constraint(reserved_phase_name(name)));
        }
        if !self.graphs.contains_key(&graph) {
            return Err(StoreError::Constraint(format!(
                "step_graph_phase.graph_id `{graph}` references no step_graph"
            )));
        }
        let clashes = |taken: &dyn Fn(&StepGraphPhase) -> bool| {
            self.phases
                .iter()
                .any(|row| row.graph_id == graph && Some(row.id) != except && taken(row))
        };
        if clashes(&|row| row.position == position) {
            return Err(StoreError::Constraint(format!(
                "step_graph_phase.position {position} is taken in graph `{graph}`"
            )));
        }
        if clashes(&|row| row.name == name) {
            return Err(StoreError::Constraint(format!(
                "step_graph_phase.name `{name}` is taken in graph `{graph}`"
            )));
        }
        Ok(())
    }

    /// Inserts a whole phase row; the caller's `updated_at` is discarded for the store's clock.
    fn create_phase(
        &mut self,
        phase: &StepGraphPhase,
        now: DateTime<Utc>,
    ) -> Result<StepGraphPhase> {
        self.check_phase(phase.graph_id, phase.position, &phase.name, None)?;
        if self.phase(phase.id).is_some() {
            return Err(StoreError::Constraint(format!(
                "step_graph_phase `{}` already exists",
                phase.id
            )));
        }
        // MOD-26 D5: Postgres's order, the unique indexes at insert and the foreign key after.
        if let Some(persona) = phase.persona_id {
            self.require_persona(persona)?;
        }
        let mut row = phase.clone();
        row.updated_at = now;
        self.phases.push(row.clone());
        Ok(row)
    }

    /// Inserts a phase's candidate rows, all or nothing (MOD-37 R-6): every refusal is decided
    /// before the first insert, and is a `Constraint` as `PgStore`'s is. An empty slice checks
    /// nothing.
    fn create_phase_agents(&mut self, phase: PhaseId, agents: &[PhaseAgent]) -> Result<()> {
        if agents.is_empty() {
            return Ok(());
        }
        if let Some(row) = agents.iter().find(|row| row.phase_id != phase) {
            return Err(StoreError::Constraint(row_names_another_phase(
                "phase_agent",
                row.phase_id,
                phase,
            )));
        }
        if self.phase(phase).is_none() {
            return Err(StoreError::Constraint(references_no_row(
                "phase_agent.phase_id",
                phase,
                "step_graph_phase",
            )));
        }
        let mut batch = HashSet::new();
        for row in agents {
            if !self.agents.contains_key(&row.agent_id) {
                return Err(StoreError::Constraint(references_no_row(
                    "phase_agent.agent_id",
                    row.agent_id,
                    "agent",
                )));
            }
            if self.phase_agents.contains_key(&(phase, row.position)) || !batch.insert(row.position)
            {
                return Err(StoreError::Constraint(already_exists(
                    "phase_agent",
                    format!("({phase}, {})", row.position),
                )));
            }
        }
        self.phase_agents.extend(
            agents
                .iter()
                .map(|row| ((row.phase_id, row.position), row.clone())),
        );
        Ok(())
    }

    /// A phase's `phase_agent` rows in `position` order, the map's own order.
    fn phase_agents_of(&self, phase: PhaseId) -> Vec<PhaseAgent> {
        self.phase_agents
            .range((phase, i32::MIN)..=(phase, i32::MAX))
            .map(|(_, row)| row.clone())
            .collect()
    }

    /// Compare-and-set on the phase's `updated_at` over [`PhasePatch`]'s six columns;
    /// `token_budget` is the `Phase` rung's and is not here (D8).
    fn update_phase(
        &mut self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<StepGraphPhase>> {
        let current = self
            .phase(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "step_graph_phase",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        self.check_phase(
            current.graph_id,
            patch.position.unwrap_or(current.position),
            patch.name.as_deref().unwrap_or(&current.name),
            Some(id),
        )?;
        if let Some(Some(persona)) = patch.persona {
            self.require_persona(persona)?;
        }
        let row = self
            .phase_mut(id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(position) = patch.position {
            row.position = position;
        }
        if let Some(template_name) = patch.template_name {
            row.template_name = template_name;
        }
        if let Some(gate_hard) = patch.gate_hard {
            row.gate_hard = gate_hard;
        }
        if let Some(input_kinds) = patch.input_kinds {
            row.input_kinds = input_kinds;
        }
        if let Some(persona) = patch.persona {
            row.persona_id = persona;
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// A graph's phases, ordered by `position`, which is unique per graph.
    fn phase_rows(&self, graph: StepGraphId) -> Vec<StepGraphPhase> {
        let mut rows: Vec<StepGraphPhase> = self
            .phases
            .iter()
            .filter(|row| row.graph_id == graph)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.position);
        rows
    }

    /// MOD-9 D1/D4/D18: the head of `(project, name)` is the token; token, then input, then keys.
    fn append_prompt_template(
        &mut self,
        new: NewPromptTemplate,
        expected: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<PromptTemplate>> {
        let head = self
            .templates
            .iter()
            .filter(|row| row.project_id == new.project_id && row.name == new.name)
            .max_by_key(|row| row.version)
            .cloned();
        match (head, expected) {
            (Some(head), _) if Some(head.version) != expected => {
                return Ok(CasOutcome::Stale(head));
            }
            (None, Some(_)) => {
                return Err(StoreError::NotFound {
                    entity: "prompt_template",
                    id: prompt_template_key(new.project_id, &new.name),
                });
            }
            _ => {}
        }
        if let Some(refusal) = prompt_template_refusal(&new.name, &new.body) {
            return Err(StoreError::Constraint(refusal));
        }
        self.require_user(new.created_by, "prompt_template.created_by")?;
        if !self.projects.contains_key(&new.project_id) {
            return Err(StoreError::Constraint(references_no_row(
                "prompt_template.project_id",
                new.project_id,
                "project",
            )));
        }
        if self.templates.iter().any(|row| row.id == new.id) {
            return Err(StoreError::Constraint(already_exists(
                "prompt_template",
                new.id,
            )));
        }
        let row = PromptTemplate {
            id: new.id,
            project_id: new.project_id,
            name: new.name,
            version: expected.unwrap_or(0) + 1,
            body: new.body,
            created_by: new.created_by,
            created_at: now,
            updated_at: now,
        };
        self.templates.push(row.clone());
        Ok(CasOutcome::Applied(row))
    }

    // ---- MOD-9 milestone 3: the skill writers (plan D75-D79, blueprint §3.3) ----------------

    /// D91: name byte order.
    fn skill_rows(&self) -> Vec<Skill> {
        let mut rows: Vec<Skill> = self.skills.values().cloned().collect();
        rows.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        rows
    }

    /// D91: one skill's versions by `version`.
    fn skill_version_rows(&self, skill: SkillId) -> Vec<SkillVersion> {
        let mut rows: Vec<SkillVersion> = self
            .skill_versions
            .iter()
            .filter(|row| row.skill_id == skill)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.version);
        rows
    }

    /// D91: `project_id == project`, sorted by `(skill_id, phase_id)` (`None < Some`, as `NULLS
    /// FIRST`; `Uuid`'s `Ord` is byte order, as Postgres's `uuid` comparison).
    fn skill_binding_rows(&self, project: Option<ProjectId>) -> Vec<SkillBinding> {
        let mut rows: Vec<SkillBinding> = self
            .skill_bindings
            .iter()
            .filter(|row| row.project_id == project)
            .cloned()
            .collect();
        rows.sort_by_key(|row| (row.skill_id.as_uuid(), row.phase_id.map(PhaseId::as_uuid)));
        rows
    }

    /// The row at a natural key, `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`.
    fn skill_binding_at(&self, key: SkillBindingKey) -> Option<&SkillBinding> {
        self.skill_bindings
            .iter()
            .find(|row| SkillBindingKey::of(row) == key)
    }

    /// Order: `new_skill_refusal`; `require_user(created_by, "skill.created_by")`; a taken id →
    /// `already_exists("skill", id)`; a taken name → `already_exists("skill", name)`. Writes the
    /// row and its version 1, both stamped `now`.
    fn create_skill(&mut self, new: NewSkill, now: DateTime<Utc>) -> Result<(Skill, SkillVersion)> {
        if let Some(refusal) = new_skill_refusal(&new.name, &new.description, &new.body) {
            return Err(StoreError::Constraint(refusal));
        }
        self.require_user(new.created_by, "skill.created_by")?;
        if self.skills.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists("skill", new.id)));
        }
        if self.skills.values().any(|row| row.name == new.name) {
            return Err(StoreError::Constraint(already_exists("skill", &new.name)));
        }
        let skill = Skill {
            id: new.id,
            name: new.name,
            description: new.description,
            created_by: new.created_by,
            created_at: now,
            updated_at: now,
        };
        let version = SkillVersion {
            skill_id: new.id,
            version: 1,
            body: new.body,
            source: new.source,
            created_by: new.created_by,
            created_at: now,
        };
        self.skills.insert(skill.id, skill.clone());
        self.skill_versions.push(version.clone());
        Ok((skill, version))
    }

    /// `update_workspace`'s shape: `NotFound("skill")` → `Stale(current)` →
    /// `skill_patch_refusal` → a name another row holds → apply both `Some` fields and stamp
    /// `now` (an all-`None` patch still stamps, as the Postgres trigger does).
    fn update_skill(
        &mut self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Skill>> {
        let current = self
            .skills
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "skill",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        if let Some(refusal) = skill_patch_refusal(&patch) {
            return Err(StoreError::Constraint(refusal));
        }
        if let Some(name) = &patch.name
            && self
                .skills
                .values()
                .any(|row| row.id != id && row.name == *name)
        {
            return Err(StoreError::Constraint(already_exists("skill", name)));
        }
        let row = self
            .skills
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(description) = patch.description {
            row.description = description;
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    // ---- MOD-26 milestone 1: the persona registry (plan D3-D5, blueprint §2.9) -------------

    /// Plan D4: name byte order (`COLLATE "C"`).
    fn persona_rows(&self) -> Vec<Persona> {
        let mut rows: Vec<Persona> = self.personas.values().cloned().collect();
        rows.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        rows
    }

    /// Order: `new_persona_refusal`; a taken id → `already_exists("persona", id)`; a taken name
    /// → `already_exists("persona", name)`. Both stamps are `now`.
    fn create_persona(&mut self, new: NewPersona, now: DateTime<Utc>) -> Result<Persona> {
        if let Some(refusal) = new_persona_refusal(&new) {
            return Err(StoreError::Constraint(refusal));
        }
        if self.personas.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists("persona", new.id)));
        }
        if self.personas.values().any(|row| row.name == new.name) {
            return Err(StoreError::Constraint(already_exists("persona", &new.name)));
        }
        let persona = Persona {
            id: new.id,
            name: new.name,
            description: new.description,
            body: new.body,
            tools: new.tools,
            permission: new.permission,
            created_at: now,
            updated_at: now,
        };
        self.personas.insert(persona.id, persona.clone());
        Ok(persona)
    }

    /// `update_skill`'s shape: `NotFound("persona")` → `Stale(current)` →
    /// `persona_patch_refusal` → a name another row holds → apply every `Some` field and stamp
    /// `now` (an all-`None` patch still stamps, as the Postgres trigger does).
    fn update_persona(
        &mut self,
        id: PersonaId,
        expected: DateTime<Utc>,
        patch: PersonaPatch,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Persona>> {
        let current = self
            .personas
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound {
                entity: "persona",
                id: id.to_string(),
            })?;
        if current.updated_at != expected {
            return Ok(CasOutcome::Stale(current));
        }
        if let Some(refusal) = persona_patch_refusal(&patch) {
            return Err(StoreError::Constraint(refusal));
        }
        if let Some(name) = &patch.name
            && self
                .personas
                .values()
                .any(|row| row.id != id && row.name == *name)
        {
            return Err(StoreError::Constraint(already_exists("persona", name)));
        }
        let row = self
            .personas
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(name) = patch.name {
            row.name = name;
        }
        if let Some(description) = patch.description {
            row.description = description;
        }
        if let Some(body) = patch.body {
            row.body = body;
        }
        if let Some(tools) = patch.tools {
            row.tools = tools;
        }
        if let Some(permission) = patch.permission {
            row.permission = permission;
        }
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// MOD-26 M2 D14: `NotFound("persona")` → [`persona_is_bound`] naming every phase that binds
    /// it (graph → project; a missing parent is named `?`, unreachable, B-5) → remove.
    fn delete_persona(&mut self, id: PersonaId) -> Result<()> {
        let persona = self.personas.get(&id).ok_or_else(|| StoreError::NotFound {
            entity: "persona",
            id: id.to_string(),
        })?;
        if self.phases.iter().any(|phase| phase.persona_id == Some(id)) {
            let holders: Vec<(String, String, String)> = self
                .phases
                .iter()
                .filter(|phase| phase.persona_id == Some(id))
                .map(|phase| {
                    let graph = self.graphs.get(&phase.graph_id);
                    let project = graph.and_then(|graph| self.projects.get(&graph.project_id));
                    (
                        project.map_or_else(|| "?".to_owned(), |row| row.slug.clone()),
                        graph.map_or_else(|| "?".to_owned(), |row| row.name.clone()),
                        phase.name.clone(),
                    )
                })
                .collect();
            return Err(StoreError::Constraint(persona_is_bound(
                &persona.name,
                &holders,
            )));
        }
        self.personas.remove(&id);
        Ok(())
    }

    /// MOD-26 D5: `fk_step_graph_phase_persona` in `references_no_row`'s words.
    fn require_persona(&self, id: PersonaId) -> Result<()> {
        if self.personas.contains_key(&id) {
            Ok(())
        } else {
            Err(StoreError::Constraint(references_no_row(
                "step_graph_phase.persona_id",
                id,
                "persona",
            )))
        }
    }

    /// D89's order; pushes version `expected + 1` stamped `now`.
    fn add_skill_version(
        &mut self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<SkillVersion>> {
        let head = self
            .skill_versions
            .iter()
            .filter(|row| row.skill_id == skill)
            .max_by_key(|row| row.version)
            .cloned();
        if let Some(head) = &head
            && head.version != expected
        {
            return Ok(CasOutcome::Stale(head.clone()));
        }
        if !self.skills.contains_key(&skill) {
            return Err(StoreError::NotFound {
                entity: "skill",
                id: skill.to_string(),
            });
        }
        if head.is_none() && expected != 0 {
            return Err(StoreError::NotFound {
                entity: "skill_version",
                id: skill_version_key(skill, expected),
            });
        }
        if let Some(refusal) = skill_body_refusal(&new.body) {
            return Err(StoreError::Constraint(refusal));
        }
        self.require_user(new.created_by, "skill_version.created_by")?;
        let row = SkillVersion {
            skill_id: skill,
            version: expected + 1,
            body: new.body,
            source: new.source,
            created_by: new.created_by,
            created_at: now,
        };
        self.skill_versions.push(row.clone());
        Ok(CasOutcome::Applied(row))
    }

    /// D90's order. An attach with no row pushes a fresh id; with a row it replaces the five
    /// editable columns in place and stamps `now` (the id is kept); a detach removes the row.
    fn set_skill_binding(
        &mut self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Option<SkillBinding>>> {
        let current = self.skill_binding_at(key).cloned();
        if current.as_ref().map(|row| row.updated_at) != expected {
            return Ok(CasOutcome::Stale(current));
        }
        let skill_name = self
            .skills
            .get(&key.skill)
            .map(|row| row.name.clone())
            .ok_or_else(|| StoreError::NotFound {
                entity: "skill",
                id: key.skill.to_string(),
            })?;
        let project_slug = match key.project {
            Some(id) => self
                .projects
                .get(&id)
                .map(|row| row.slug.clone())
                .ok_or_else(|| StoreError::NotFound {
                    entity: "project",
                    id: id.to_string(),
                })?,
            None => String::new(),
        };
        let phase_project = match key.phase {
            Some(id) => {
                let phase = self.phase(id).ok_or_else(|| StoreError::NotFound {
                    entity: "step_graph_phase",
                    id: id.to_string(),
                })?;
                self.graphs
                    .get(&phase.graph_id)
                    .map(|graph| graph.project_id)
            }
            None => None,
        };

        let attachment = match change {
            BindingChange::Detach => {
                if let Some(row) = current {
                    self.skill_bindings.retain(|other| other.id != row.id);
                }
                return Ok(CasOutcome::Applied(None));
            }
            BindingChange::Attach(attachment) => attachment,
        };
        let versions: Vec<i32> = self
            .skill_version_rows(key.skill)
            .iter()
            .map(|row| row.version)
            .collect();
        let repos: Vec<String> = key
            .project
            .map(|id| self.repo_rows(id))
            .unwrap_or_default()
            .into_iter()
            .map(|repo| repo.name)
            .collect();
        let stored = check_attachment(
            &BindingFacts {
                key,
                phase_project,
                skill_name: &skill_name,
                versions: &versions,
                repos: &repos,
                project_slug: &project_slug,
            },
            &attachment,
        )
        .map_err(StoreError::Constraint)?;

        let row = SkillBinding {
            id: current.map_or_else(SkillBindingId::new, |row| row.id),
            skill_id: key.skill,
            project_id: key.project,
            phase_id: key.phase,
            pinned_version: attachment.pinned_version,
            position: attachment.position,
            activation: attachment.activation,
            globs: stored.globs,
            languages: stored.languages,
            updated_at: now,
        };
        match self
            .skill_bindings
            .iter_mut()
            .find(|other| other.id == row.id)
        {
            Some(slot) => *slot = row.clone(),
            None => self.skill_bindings.push(row.clone()),
        }
        Ok(CasOutcome::Applied(Some(row)))
    }

    /// The value one rung currently holds for a key, for [`validate`]'s `not_above` peer.
    fn setting_value(&self, rung: SettingRung, key: SettingKey) -> Option<Value> {
        self.stored_setting(rung, key).and_then(|row| row.value)
    }

    /// One setting with the rung row's token, or `None` when the rung's row is absent (D8).
    fn stored_setting(&self, rung: SettingRung, key: SettingKey) -> Option<StoredSetting> {
        match rung {
            SettingRung::App => {
                self.app_settings
                    .get(key.key())
                    .map(|(value, updated_at)| StoredSetting {
                        value: Some(value.clone()),
                        updated_at: *updated_at,
                    })
            }
            SettingRung::Project(id) => self.projects.get(&id).map(|project| StoredSetting {
                value: key
                    .spec()
                    .project_key
                    .and_then(|name| project.settings.get(name).cloned()),
                updated_at: project.updated_at,
            }),
            SettingRung::Phase(id) => self.phase(id).map(|phase| StoredSetting {
                value: phase.token_budget.map(Value::from),
                updated_at: phase.updated_at,
            }),
        }
    }

    /// Writes one setting on one rung, after the whole of [`validate`] (D7, D8).
    ///
    /// Validation runs before the compare-and-set on purpose: a value the reader would clamp is
    /// refused whether or not the caller's token was current, so "your edit was stale" never
    /// stands in for "that number does not mean what you think".
    fn set_setting(
        &mut self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>> {
        if let Some(refusal) = rung_refusal(key, rung.flag()) {
            return Err(StoreError::Constraint(refusal));
        }
        let peer = key
            .spec()
            .not_above
            .and_then(|other| self.setting_value(rung, other));
        validate(key, rung.flag(), &value, peer.as_ref()).map_err(StoreError::Constraint)?;

        match rung {
            SettingRung::App => {
                let stored = self
                    .app_settings
                    .get(key.key())
                    .map(|(value, token)| (value.clone(), *token));
                // `expected: None` is "I expect no row" — the insert after a clear — so the two
                // that match are no row and no expectation, or a row whose token is the one held.
                let current = match (&stored, expected) {
                    (None, None) => true,
                    (Some((_, token)), Some(want)) => *token == want,
                    (None, Some(_)) | (Some(_), None) => false,
                };
                if current {
                    self.app_settings
                        .insert(key.key().to_owned(), (value.clone(), now));
                    return Ok(CasOutcome::Applied(StoredSetting {
                        value: Some(value),
                        updated_at: now,
                    }));
                }
                match stored {
                    Some((held, token)) => Ok(CasOutcome::Stale(StoredSetting {
                        value: Some(held),
                        updated_at: token,
                    })),
                    // A token for a row that is not there is a missing edit, not a stale one:
                    // there is nothing to hand back for the caller to reload from.
                    None => Err(StoreError::NotFound {
                        entity: "app_setting",
                        id: key.key().to_owned(),
                    }),
                }
            }
            SettingRung::Project(id) => {
                let token = expected
                    .ok_or_else(|| StoreError::Constraint(expected_on_row(key, "project")))?;
                let stored =
                    self.stored_setting(rung, key)
                        .ok_or_else(|| StoreError::NotFound {
                            entity: "project",
                            id: id.to_string(),
                        })?;
                if stored.updated_at != token {
                    return Ok(CasOutcome::Stale(stored));
                }
                let name = key
                    .spec()
                    .project_key
                    .expect("the rung check passed, so the spec names a project key");
                let project = self
                    .projects
                    .get_mut(&id)
                    .expect("the row was read a statement ago under the same lock");
                let Some(map) = project.settings.as_object_mut() else {
                    return Err(StoreError::Constraint(settings_not_an_object(id, key)));
                };
                map.insert(name.to_owned(), value.clone());
                project.updated_at = now;
                Ok(CasOutcome::Applied(StoredSetting {
                    value: Some(value),
                    updated_at: now,
                }))
            }
            SettingRung::Phase(id) => {
                let token = expected.ok_or_else(|| {
                    StoreError::Constraint(expected_on_row(key, "step_graph_phase"))
                })?;
                let stored =
                    self.stored_setting(rung, key)
                        .ok_or_else(|| StoreError::NotFound {
                            entity: "step_graph_phase",
                            id: id.to_string(),
                        })?;
                if stored.updated_at != token {
                    return Ok(CasOutcome::Stale(stored));
                }
                // `validate` has already narrowed this rung to `i32::MAX` (flag C), so the `None`
                // arm is unreachable — and it is still an error rather than an `expect`, because
                // "unreachable" here depends on a guard two modules away and a panic is a poor way
                // to find out it moved (review L6).
                let budget = value
                    .as_i64()
                    .and_then(|number| i32::try_from(number).ok())
                    .ok_or_else(|| {
                        StoreError::Constraint(format!(
                            "`{key}` = {value} does not fit `step_graph_phase.token_budget`, \
                             which is INTEGER"
                        ))
                    })?;
                let phase = self
                    .phase_mut(id)
                    .expect("the row was read a statement ago under the same lock");
                phase.token_budget = Some(budget);
                phase.updated_at = now;
                Ok(CasOutcome::Applied(StoredSetting {
                    value: Some(value),
                    updated_at: now,
                }))
            }
        }
    }

    /// Removes one setting from one rung under CAS; `Applied` always carries `value: None` (D8).
    fn clear_setting(
        &mut self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>> {
        if let Some(refusal) = rung_refusal(key, rung.flag()) {
            return Err(StoreError::Constraint(refusal));
        }
        match rung {
            SettingRung::App => {
                let Some((held, token)) = self
                    .app_settings
                    .get(key.key())
                    .map(|(value, token)| (value.clone(), *token))
                else {
                    return Err(StoreError::NotFound {
                        entity: "app_setting",
                        id: key.key().to_owned(),
                    });
                };
                if token != expected {
                    return Ok(CasOutcome::Stale(StoredSetting {
                        value: Some(held),
                        updated_at: token,
                    }));
                }
                self.app_settings.remove(key.key());
                // Flag D: a cleared App setting has no row left to carry a token, so the deleted
                // row's is what comes back and the next `set_setting` passes `expected: None`.
                Ok(CasOutcome::Applied(StoredSetting {
                    value: None,
                    updated_at: token,
                }))
            }
            SettingRung::Project(id) => {
                let stored =
                    self.stored_setting(rung, key)
                        .ok_or_else(|| StoreError::NotFound {
                            entity: "project",
                            id: id.to_string(),
                        })?;
                if stored.updated_at != expected {
                    return Ok(CasOutcome::Stale(stored));
                }
                let name = key
                    .spec()
                    .project_key
                    .expect("the rung check passed, so the spec names a project key");
                let project = self
                    .projects
                    .get_mut(&id)
                    .expect("the row was read a statement ago under the same lock");
                let Some(map) = project.settings.as_object_mut() else {
                    return Err(StoreError::Constraint(settings_not_an_object(id, key)));
                };
                map.remove(name);
                project.updated_at = now;
                Ok(CasOutcome::Applied(StoredSetting {
                    value: None,
                    updated_at: now,
                }))
            }
            SettingRung::Phase(id) => {
                let stored =
                    self.stored_setting(rung, key)
                        .ok_or_else(|| StoreError::NotFound {
                            entity: "step_graph_phase",
                            id: id.to_string(),
                        })?;
                if stored.updated_at != expected {
                    return Ok(CasOutcome::Stale(stored));
                }
                let phase = self
                    .phase_mut(id)
                    .expect("the row was read a statement ago under the same lock");
                phase.token_budget = None;
                phase.updated_at = now;
                Ok(CasOutcome::Applied(StoredSetting {
                    value: None,
                    updated_at: now,
                }))
            }
        }
    }

    /// One queue key with its token (MOD-12 M2 D9), or `None` when the target's row is absent:
    /// no `app_setting` row, no such project, or no box of `user` by that id.
    fn queue_setting(
        &self,
        user: Option<UserId>,
        target: QueueTarget,
        key: QueueSetting,
    ) -> Option<QueueStored> {
        let name = key.as_str();
        match target {
            QueueTarget::App => self.app_settings.get(name).map(|(value, at)| QueueStored {
                value: Some(value.clone()),
                token: QueueToken::Stamp(Some(*at)),
            }),
            QueueTarget::Project(id) => self.projects.get(&id).map(|project| QueueStored {
                value: project.settings.get(name).cloned(),
                token: QueueToken::Stamp(Some(project.updated_at)),
            }),
            QueueTarget::Box(id) => self
                .boxes
                .get(&id)
                .filter(|row| Some(row.user_id) == user)
                .map(|row| QueueStored {
                    value: row.settings.get(name).cloned(),
                    token: QueueToken::EditVersion(row.edit_version),
                }),
        }
    }

    /// Writes (`Some`) or clears (`None`) one queue key (MOD-12 M2 D9): the target and token
    /// refusals, the validator on a set, then the row as `set_setting`'s `App` and `Project` rungs
    /// and `edit_box`'s lookup and guards do. `now` stands in for Postgres's trigger.
    fn write_queue_setting(
        &mut self,
        user: Option<UserId>,
        target: QueueTarget,
        key: QueueSetting,
        value: Option<Value>,
        expected: QueueToken,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<QueueStored>> {
        if let Some(refusal) = queue_target_refusal(key, target) {
            return Err(StoreError::Constraint(refusal));
        }
        if let Some(refusal) = queue_token_refusal(key, target, expected, value.is_none()) {
            return Err(StoreError::Constraint(refusal));
        }
        if let Some(value) = &value {
            key.validate(value).map_err(StoreError::Constraint)?;
        }
        let name = key.as_str();
        match (target, expected) {
            (QueueTarget::App, QueueToken::Stamp(want)) => {
                let stored = self.queue_setting(user, target, key);
                // `Stamp(None)` is "I expect no row": no row and no expectation match, or a row
                // whose token is the one held.
                let current = match (&stored, want) {
                    (None, None) => true,
                    (Some(row), Some(_)) => row.token == expected,
                    (None, Some(_)) | (Some(_), None) => false,
                };
                if !current {
                    return match stored {
                        Some(row) => Ok(CasOutcome::Stale(row)),
                        None => Err(StoreError::NotFound {
                            entity: "app_setting",
                            id: name.to_owned(),
                        }),
                    };
                }
                Ok(CasOutcome::Applied(match value {
                    Some(value) => {
                        self.app_settings
                            .insert(name.to_owned(), (value.clone(), now));
                        QueueStored {
                            value: Some(value),
                            token: QueueToken::Stamp(Some(now)),
                        }
                    }
                    None => {
                        self.app_settings.remove(name);
                        QueueStored {
                            value: None,
                            token: QueueToken::Stamp(None),
                        }
                    }
                }))
            }
            (QueueTarget::Project(id), QueueToken::Stamp(Some(_))) => {
                let stored =
                    self.queue_setting(user, target, key)
                        .ok_or_else(|| StoreError::NotFound {
                            entity: "project",
                            id: id.to_string(),
                        })?;
                if stored.token != expected {
                    return Ok(CasOutcome::Stale(stored));
                }
                let project = self
                    .projects
                    .get_mut(&id)
                    .expect("the row was read a statement ago under the same lock");
                let Some(map) = project.settings.as_object_mut() else {
                    return Err(StoreError::Constraint(settings_not_an_object(id, key)));
                };
                match &value {
                    Some(value) => map.insert(name.to_owned(), value.clone()),
                    None => map.remove(name),
                };
                project.updated_at = now;
                Ok(CasOutcome::Applied(QueueStored {
                    value,
                    token: QueueToken::Stamp(Some(now)),
                }))
            }
            (QueueTarget::Box(id), QueueToken::EditVersion(want)) => {
                let Some(row) = self
                    .boxes
                    .get_mut(&id)
                    .filter(|row| Some(row.user_id) == user)
                else {
                    return Err(StoreError::NotFound {
                        entity: "box",
                        id: id.to_string(),
                    });
                };
                if row.edit_version != want {
                    return Ok(CasOutcome::Stale(QueueStored {
                        value: row.settings.get(name).cloned(),
                        token: QueueToken::EditVersion(row.edit_version),
                    }));
                }
                let Some(map) = row.settings.as_object_mut() else {
                    return Err(StoreError::Constraint(
                        BOX_SETTINGS_NOT_AN_OBJECT.to_owned(),
                    ));
                };
                match &value {
                    Some(value) => map.insert(name.to_owned(), value.clone()),
                    None => map.remove(name),
                };
                row.edit_version += 1;
                row.updated_at = now;
                Ok(CasOutcome::Applied(QueueStored {
                    value,
                    token: QueueToken::EditVersion(row.edit_version),
                }))
            }
            _ => unreachable!("queue_token_refusal answered every other target and token pair"),
        }
    }

    /// The `box_probe_spec` row with its token (MOD-51 D2): `app_settings` holds it beside the
    /// ten `SettingKey` rows, under a key no `SettingKey` spells.
    fn box_probe_spec(&self) -> Option<StoredSetting> {
        self.app_settings
            .get(BOX_PROBE_SPEC_KEY)
            .map(|(value, updated_at)| StoredSetting {
                value: Some(value.clone()),
                updated_at: *updated_at,
            })
    }

    /// The overlay's compare-and-set (MOD-51 D2): both refusals first, then the `App` rung's
    /// `(stored, expected)` rule of [`set_setting`](State::set_setting), except that a miss over
    /// no row is `Stale(None)` rather than `NotFound`. `now` stands in for Postgres's
    /// `DEFAULT now()` / `set_updated_at`.
    fn set_box_probe_spec(
        &mut self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<Option<StoredSetting>>> {
        if overlay.as_ref().is_some_and(|value| !value.is_object()) {
            return Err(StoreError::Constraint(
                BOX_PROBE_SPEC_NOT_AN_OBJECT.to_owned(),
            ));
        }
        if overlay.is_none() && expected.is_none() {
            return Err(StoreError::Constraint(
                BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN.to_owned(),
            ));
        }
        let stored = self.box_probe_spec();
        // `expected: None` is "I expect no row", as on the `App` rung.
        let current = match (&stored, expected) {
            (None, None) => true,
            (Some(row), Some(want)) => row.updated_at == want,
            (None, Some(_)) | (Some(_), None) => false,
        };
        if !current {
            return Ok(CasOutcome::Stale(stored));
        }
        Ok(CasOutcome::Applied(match overlay {
            Some(value) => {
                self.app_settings
                    .insert(BOX_PROBE_SPEC_KEY.to_owned(), (value.clone(), now));
                Some(StoredSetting {
                    value: Some(value),
                    updated_at: now,
                })
            }
            None => {
                self.app_settings.remove(BOX_PROBE_SPEC_KEY);
                None
            }
        }))
    }

    /// What a workspace delete reaches: its links and its box paths, and no project (D4).
    fn workspace_reach(&self, id: WorkspaceId) -> Option<DeleteReach> {
        if !self.workspaces.contains_key(&id) {
            return None;
        }
        Some(DeleteReach {
            workspace_links: rows(
                self.workspace_projects
                    .iter()
                    .filter(|row| row.workspace_id == id)
                    .count(),
            ),
            workspace_box_paths: rows(
                self.workspace_box_paths
                    .iter()
                    .filter(|row| row.workspace_id == id)
                    .count(),
            ),
            ..DeleteReach::default()
        })
    }

    /// What a project delete reaches, counted and identified in one pass (PRD D13).
    ///
    /// The counts and the id sets come out of the same predicates, which is what makes
    /// `delete_reach`'s report and `delete_project`'s act equal by construction rather than by two
    /// lists kept in step by hand. `workspace_box_paths` is `0` because a project is not a
    /// workspace. `run_step_commits` and `run_step_trees` were `0` until MOD-4 milestone 1 gave
    /// this store the two maps (plan D12), `command_runs` until milestone 3 gave it the third
    /// (plan D31), and `phase_agents` until MOD-37 R-6 gave it the fourth. MOD-38's six
    /// requirement counts come from the same pass (blueprint F1, §4.4).
    fn project_reach(&self, id: ProjectId) -> Option<(DeleteReach, ProjectReach)> {
        if !self.projects.contains_key(&id) {
            return None;
        }
        let items: HashSet<ItemId> = self
            .items
            .values()
            .filter(|row| row.project_id == id)
            .map(|row| row.id)
            .collect();
        let runs: HashSet<RunId> = self
            .runs
            .values()
            .filter(|row| {
                row.project_id == id || row.item_id.is_some_and(|item| items.contains(&item))
            })
            .map(|row| row.id)
            .collect();
        let steps: HashSet<StepId> = self
            .steps
            .values()
            .filter(|row| runs.contains(&row.run_id))
            .map(|row| row.id)
            .collect();
        let graphs: HashSet<StepGraphId> = self
            .graphs
            .values()
            .filter(|row| row.project_id == id)
            .map(|row| row.id)
            .collect();
        let phases: HashSet<PhaseId> = self
            .phases
            .iter()
            .filter(|row| graphs.contains(&row.graph_id))
            .map(|row| row.id)
            .collect();
        let repos: HashSet<RepoId> = self
            .repos
            .values()
            .filter(|row| row.project_id == id)
            .map(|row| row.id)
            .collect();
        let requirement_areas: HashSet<RequirementAreaId> = self
            .requirement_areas
            .values()
            .filter(|row| row.project_id == id)
            .map(|row| row.id)
            .collect();
        let requirements: HashSet<RequirementId> = self
            .requirements
            .values()
            .filter(|row| row.project_id == id)
            .map(|row| row.id)
            .collect();

        let reach = DeleteReach {
            workspace_links: rows(
                self.workspace_projects
                    .iter()
                    .filter(|row| row.project_id == id)
                    .count(),
            ),
            workspace_box_paths: 0,
            items: rows(items.len()),
            item_key_counters: rows(
                self.item_key_counter
                    .keys()
                    .filter(|(project, _)| *project == id)
                    .count(),
            ),
            item_kinds: rows(
                self.kinds
                    .values()
                    .filter(|row| row.project_id == id)
                    .count(),
            ),
            step_graphs: rows(graphs.len()),
            phases: rows(phases.len()),
            phase_agents: rows(
                self.phase_agents
                    .keys()
                    .filter(|(phase, _)| phases.contains(phase))
                    .count(),
            ),
            prompt_templates: rows(
                self.templates
                    .iter()
                    .filter(|row| row.project_id == id)
                    .count(),
            ),
            repos: rows(repos.len()),
            repo_box_paths: rows(
                self.repo_box_paths
                    .iter()
                    .filter(|row| repos.contains(&row.repo_id))
                    .count(),
            ),
            skill_bindings: rows(
                self.skill_bindings
                    .iter()
                    .filter(|row| row.project_id == Some(id))
                    .count(),
            ),
            runs: rows(runs.len()),
            run_steps: rows(steps.len()),
            session_events: rows(
                self.events
                    .iter()
                    .filter(|row| steps.contains(&row.run_step_id))
                    .count(),
            ),
            run_step_commits: rows(
                self.step_commits
                    .keys()
                    .filter(|(step, _)| steps.contains(step))
                    .count(),
            ),
            run_step_trees: rows(
                self.step_trees
                    .keys()
                    .filter(|(step, _)| steps.contains(step))
                    .count(),
            ),
            command_runs: rows(
                self.command_runs
                    .values()
                    .filter(|row| steps.contains(&row.run_step_id))
                    .count(),
            ),
            notes: rows(
                self.notes
                    .iter()
                    .filter(|row| items.contains(&row.item_id))
                    .count(),
            ),
            revisions: rows(
                self.revisions
                    .keys()
                    .filter(|(item, _)| items.contains(item))
                    .count(),
            ),
            links: rows(
                self.links
                    .iter()
                    .filter(|row| {
                        items.contains(&row.from_item_id) || items.contains(&row.to_item_id)
                    })
                    .count(),
            ),
            documents: rows(
                self.documents
                    .iter()
                    .filter(|row| items.contains(&row.item_id))
                    .count(),
            ),
            requirement_specs: rows(usize::from(self.requirement_specs.contains_key(&id))),
            requirement_areas: rows(requirement_areas.len()),
            requirement_key_counters: rows(
                self.requirement_key_counter
                    .keys()
                    .filter(|area| requirement_areas.contains(area))
                    .count(),
            ),
            requirements: rows(requirements.len()),
            requirement_revisions: rows(
                self.requirement_revisions
                    .iter()
                    .filter(|row| requirements.contains(&row.requirement_id))
                    .count(),
            ),
            // Either end, tombstones included, as `links` above (blueprint §4.4).
            item_requirements: rows(
                self.item_requirements
                    .iter()
                    .filter(|row| {
                        items.contains(&row.item_id) || requirements.contains(&row.requirement_id)
                    })
                    .count(),
            ),
        };
        Some((
            reach,
            ProjectReach {
                items,
                runs,
                steps,
                graphs,
                phases,
                repos,
                requirement_areas,
                requirements,
            },
        ))
    }

    /// Removes a workspace, its links and its box paths. Projects survive it
    /// (`0001_init.sql:162,174`).
    fn delete_workspace(&mut self, id: WorkspaceId) -> Result<DeleteReach> {
        let reach = self
            .workspace_reach(id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "workspace",
                id: id.to_string(),
            })?;
        self.workspace_projects.retain(|row| row.workspace_id != id);
        self.workspace_box_paths
            .retain(|row| row.workspace_id != id);
        self.workspaces.remove(&id);
        Ok(reach)
    }

    /// Removes a project and everything PRD D13 lists, in `0001_init.sql`'s cascade order: leaves
    /// first, so no row is taken before the row that counted it.
    ///
    /// `item_link` goes when **either** end is the project's, tombstones included: that is what
    /// takes the fixture's cross-project edge from `agy:FEAT-1` to `htui:FEAT-2`, which no
    /// per-project predicate would have reached.
    fn delete_project(&mut self, id: ProjectId, now: DateTime<Utc>) -> Result<DeleteReach> {
        let (reach, gone) = self.project_reach(id).ok_or_else(|| StoreError::NotFound {
            entity: "project",
            id: id.to_string(),
        })?;
        self.events
            .retain(|row| !gone.steps.contains(&row.run_step_id));
        self.step_trees
            .retain(|(step, _), _| !gone.steps.contains(step));
        self.step_commits
            .retain(|(step, _), _| !gone.steps.contains(step));
        self.command_runs
            .retain(|_, row| !gone.steps.contains(&row.run_step_id));
        let command_runs = &self.command_runs;
        self.command_claims
            .retain(|id, _| command_runs.contains_key(id));
        self.steps.retain(|id, _| !gone.steps.contains(id));
        self.openings.retain(|id, _| !gone.steps.contains(id));
        self.runs.retain(|id, _| !gone.runs.contains(id));
        self.lease_owners.retain(|id, _| !gone.runs.contains(id));
        // MOD-42 plan D1: both relay tables go with their run (`ON DELETE CASCADE`, F-18).
        self.permissions
            .retain(|_, p| !gone.runs.contains(&p.row.run_id));
        self.run_commands
            .retain(|_, c| !gone.runs.contains(&c.run_id));
        // MOD-12 D1, D7: an entry goes with its item (`ON DELETE CASCADE`), and `run.batch_id`
        // with its run.
        self.queue_entries
            .retain(|item, _| !gone.items.contains(item));
        self.run_batches.retain(|run, _| !gone.runs.contains(run));
        // MOD-70 plan D1: a window and a follow-up's columns go with their run too.
        self.follow_up_windows
            .retain(|_, w| !gone.runs.contains(&w.run_id));
        let run_commands = &self.run_commands;
        self.follow_up_payloads
            .retain(|id, _| run_commands.contains_key(id));
        self.documents
            .retain(|row| !gone.items.contains(&row.item_id));
        self.notes.retain(|row| !gone.items.contains(&row.item_id));
        self.revisions
            .retain(|(item, _), _| !gone.items.contains(item));
        self.links.retain(|row| {
            !gone.items.contains(&row.from_item_id) && !gone.items.contains(&row.to_item_id)
        });
        // MOD-38 blueprint §4.4: a citation goes with either end, as a link does, and one that
        // survives loses a proposing step that did not (`proposed_by_step_id ... ON DELETE SET
        // NULL`, whose UPDATE fires `trg_item_requirement_updated_at`); a revision goes with its
        // requirement, and one that survives in another project loses a deciding item that did
        // not (`amended_by_item_id ... ON DELETE SET NULL`).
        self.item_requirements.retain(|row| {
            !gone.items.contains(&row.item_id) && !gone.requirements.contains(&row.requirement_id)
        });
        for row in &mut self.item_requirements {
            if row
                .proposed_by_step_id
                .is_some_and(|step| gone.steps.contains(&step))
            {
                row.proposed_by_step_id = None;
                row.updated_at = now;
            }
        }
        self.requirement_revisions
            .retain(|row| !gone.requirements.contains(&row.requirement_id));
        for row in &mut self.requirement_revisions {
            if row
                .amended_by_item_id
                .is_some_and(|item| gone.items.contains(&item))
            {
                row.amended_by_item_id = None;
            }
        }
        self.requirements
            .retain(|id, _| !gone.requirements.contains(id));
        self.requirement_key_counter
            .retain(|area, _| !gone.requirement_areas.contains(area));
        self.requirement_areas
            .retain(|id, _| !gone.requirement_areas.contains(id));
        self.requirement_specs.remove(&id);
        self.items.retain(|id, _| !gone.items.contains(id));
        self.item_key_counter
            .retain(|(project, _), _| *project != id);
        self.skill_bindings.retain(|row| row.project_id != Some(id));
        self.kinds.retain(|_, row| row.project_id != id);
        self.phase_agents
            .retain(|(phase, _), _| !gone.phases.contains(phase));
        self.phases.retain(|row| !gone.phases.contains(&row.id));
        self.graphs.retain(|id, _| !gone.graphs.contains(id));
        self.templates.retain(|row| row.project_id != id);
        self.repo_box_paths
            .retain(|row| !gone.repos.contains(&row.repo_id));
        self.repos.retain(|id, _| !gone.repos.contains(id));
        self.workspace_projects.retain(|row| row.project_id != id);
        self.projects.remove(&id);
        Ok(reach)
    }

    // ---- MOD-4 milestone 1: graph runs (ANA-2 §8) --------------------------------------------
    //
    // Same discipline as the MOD-15 block above, and one more reason for it: the five writers
    // plan D6 calls transactions are single `write` closures on the arms below, so every rule
    // that can refuse one of them has to be reachable without taking the lock a second time.
    // Nothing here sets `updated_at` on a table that has none: `run_step_tree` and
    // `run_step_commit` ride their parent step's, as the mirror's cursor does (plan D9).

    /// One `run` row, or the `NotFound` every writer of it opens with (plan D14).
    fn require_run(&self, id: RunId) -> Result<&Run> {
        self.runs.get(&id).ok_or_else(|| StoreError::NotFound {
            entity: "run",
            id: id.to_string(),
        })
    }

    /// [`State::require_run`] for a `run_step`.
    fn require_step(&self, id: StepId) -> Result<&RunStep> {
        self.steps.get(&id).ok_or_else(|| StoreError::NotFound {
            entity: "run_step",
            id: id.to_string(),
        })
    }

    /// MOD-12 D2: `box_id`'s open `queue_batch`; `uq_queue_batch_open` keeps it at most one.
    fn open_batch_of(&self, box_id: BoxId) -> Option<&QueueBatch> {
        self.queue_batches
            .values()
            .find(|batch| batch.box_id == box_id && batch.closed_at.is_none())
    }

    /// [`State::require_run`] for an `item`.
    fn require_item(&self, id: ItemId) -> Result<&Item> {
        self.items.get(&id).ok_or_else(|| StoreError::NotFound {
            entity: "item",
            id: id.to_string(),
        })
    }

    /// The capability vocabulary of a box: `probed_tags ∪ declared_tags` (`R-ORCH-10`). Empty for
    /// a box this store does not hold, which is what makes every tagged item unready rather than
    /// ready for a machine nobody has described.
    fn box_capabilities(&self, box_id: BoxId) -> HashSet<String> {
        self.boxes
            .get(&box_id)
            .map(|row| {
                row.probed_tags
                    .iter()
                    .chain(row.declared_tags.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `R-ORCH-10`'s set (MOD-7 D76): the entries of `required` in neither `probed_tags` nor
    /// `declared_tags` of `box_id`, sorted by bytes and deduplicated. One helper, so
    /// `MemStore::missing_tags` and `claim_run` cannot order or dedup differently.
    fn missing_for(&self, required: &[String], box_id: BoxId) -> Vec<String> {
        let capabilities = self.box_capabilities(box_id);
        let mut missing: Vec<String> = required
            .iter()
            .filter(|tag| !capabilities.contains(*tag))
            .cloned()
            .collect();
        missing.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        missing.dedup();
        missing
    }

    /// `R-ORCH-9`'s three rungs: the box's own setting, else `app_setting`, else
    /// [`DEFAULT_MAX_CONCURRENT_ITEMS`]. A box whose `settings` blob does not decode falls through
    /// exactly as one that names no key does — the column is free-form JSON and a reader that
    /// refused would take the box out of service over a typo.
    fn max_concurrent_items(&self, box_id: BoxId) -> u32 {
        self.boxes
            .get(&box_id)
            .and_then(|row| serde_json::from_value::<BoxSettings>(row.settings.clone()).ok())
            .and_then(|settings| settings.max_concurrent_items)
            .or_else(|| {
                self.app_settings
                    .get("max_concurrent_items")
                    .and_then(|(value, _)| value.as_u64())
                    .and_then(|value| u32::try_from(value).ok())
            })
            .unwrap_or(DEFAULT_MAX_CONCURRENT_ITEMS)
    }

    /// A run's steps in `(position, attempt, fanout_index)` order: the judge (`-1`) first.
    ///
    /// Named apart from [`State::run_steps`], which is the same order projected to
    /// [`RunStepSummary`] for the Runs sub-tab; this one is the seam's row read.
    fn run_step_rows(&self, run: RunId) -> Vec<RunStep> {
        let mut rows: Vec<RunStep> = self
            .steps
            .values()
            .filter(|row| row.run_id == run)
            .cloned()
            .collect();
        rows.sort_by_key(|row| (row.position, row.attempt, row.fanout_index));
        rows
    }

    /// The step's `run_step_tree` rows; the map's key order is `repo_id` order.
    fn step_tree_rows(&self, step: StepId) -> Vec<RunStepTree> {
        self.step_trees
            .iter()
            .filter(|((id, _), _)| *id == step)
            .map(|(_, row)| row.clone())
            .collect()
    }

    /// The step's `run_step_commit` rows, ordered as [`State::step_tree_rows`] is.
    fn step_commit_rows(&self, step: StepId) -> Vec<RunStepCommit> {
        self.step_commits
            .iter()
            .filter(|((id, _), _)| *id == step)
            .map(|(_, row)| row.clone())
            .collect()
    }

    /// The step-produced arm's order (ANA-2 §4.2; MOD-73 plan D2), the SQL's three-armed `CASE`
    /// spelled out: this run's output 0, another run's 1, a document whose producing step is not
    /// held 2. [`State::resolve_input`] ranks only rows that have a `produced_by_step_id`; a
    /// hand-written document is the other arm and is never ranked here.
    fn input_rank(&self, document: &Document, run: RunId) -> u8 {
        match document
            .produced_by_step_id
            .and_then(|id| self.steps.get(&id))
        {
            Some(step) if step.run_id == run => 0,
            Some(_) => 1,
            None => 2,
        }
    }

    /// One kind of ANA-2 §4.2's resolver as amended by MOD-73 (plan D2): the newer of two picks.
    ///
    /// The **step-produced** pick is the resolver as it was: rows with a `produced_by_step_id`
    /// whose step is not a fan-out loser, best [`State::input_rank`] first, then the highest
    /// version. The **hand-written** pick is the highest version with no producing step. The arm
    /// is keyed on the column, not on whether the step is still held (plan D3); this store never
    /// holds a document whose step is gone anyway, because only `delete_project` drops steps and
    /// it drops the project's items' documents with them. `(item, kind, version)` is unique, so
    /// the two picks never tie.
    fn resolve_input(&self, item: ItemId, run: RunId, kind: &str) -> Option<Document> {
        let produced = self
            .documents
            .iter()
            .filter(|document| document.item_id == item && document.kind == kind)
            .filter(|document| document.produced_by_step_id.is_some())
            .filter(|document| {
                // `s.selected IS NOT FALSE`: a fan-out loser is excluded, `NULL` is not.
                document
                    .produced_by_step_id
                    .and_then(|id| self.steps.get(&id))
                    .is_none_or(|step| step.selected != Some(false))
            })
            .min_by_key(|document| {
                (
                    self.input_rank(document, run),
                    std::cmp::Reverse(document.version),
                )
            });
        let by_hand = self
            .documents
            .iter()
            .filter(|document| document.item_id == item && document.kind == kind)
            .filter(|document| document.produced_by_step_id.is_none())
            .max_by_key(|document| document.version);
        produced
            .into_iter()
            .chain(by_hand)
            .max_by_key(|document| document.version)
            .cloned()
    }

    /// ANA-2 §4.2's resolver: one entry per requested kind, in request order (plan D2).
    ///
    /// The empty-`kinds` case is [`State::documents_of_kinds`]'s — every kind the item has, in
    /// byte order — and cannot recurse the way that one had to guard against, because the kind
    /// list is resolved before the per-kind walk rather than by calling back into this function.
    fn resolve_inputs(&self, item: ItemId, run: RunId, kinds: &[String]) -> Vec<ResolvedInput> {
        let owned;
        let wanted = if kinds.is_empty() {
            let mut all: Vec<String> = self
                .documents
                .iter()
                .filter(|document| document.item_id == item)
                .map(|document| document.kind.clone())
                .collect();
            all.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            all.dedup();
            owned = all;
            owned.as_slice()
        } else {
            kinds
        };
        wanted
            .iter()
            .map(|kind| ResolvedInput {
                kind: kind.clone(),
                document: self.resolve_input(item, run, kind),
            })
            .collect()
    }

    /// The `run` row and the item's move to `queued`, together or not at all (plan D6).
    fn create_run(&mut self, new: NewRun, now: DateTime<Utc>) -> Result<Run> {
        // Plan D14, before anything else: the row is looked up first and legality second, so a
        // request that gets the item wrong *and* something else wrong answers `NotFound` for the
        // item. `PgStore` cannot order this any other way — its `SELECT ... FOR UPDATE` on `item`
        // is the transaction's first statement — so the emulation follows it.
        let item = self.require_item(new.item_id)?;
        let (status, holder) = (item.status, item.project_id);
        legal_move(status, Status::Queued)?;
        // Nothing in the schema ties `run.project_id` to `item.project_id`, so without this a run
        // lands under project A for project B's item — where `delete_project`, which counts by
        // `run.project_id`, would take it with the wrong project.
        if holder != new.project_id {
            return Err(StoreError::Constraint(item_not_in_project(
                new.item_id,
                new.project_id,
            )));
        }
        if self.runs.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists("run", new.id)));
        }
        if !self.projects.contains_key(&new.project_id) {
            return Err(StoreError::Constraint(references_no_row(
                "run.project_id",
                new.project_id,
                "project",
            )));
        }
        if !self.boxes.contains_key(&new.target_box_id) {
            return Err(StoreError::Constraint(references_no_row(
                "run.target_box_id",
                new.target_box_id,
                "box",
            )));
        }
        self.require_user(new.started_by, "run.started_by")?;
        for repo in &new.repo_scope {
            if !self.repos.contains_key(repo) {
                return Err(StoreError::Constraint(references_no_row(
                    "run.repo_scope",
                    repo,
                    "repo",
                )));
            }
        }
        // MOD-12 D7, H-6: `PgStore`'s `FOR SHARE` read of the batch, in the same place.
        if let Some(batch) = new.batch_id {
            let open = self
                .queue_batches
                .get(&batch)
                .ok_or_else(|| StoreError::NotFound {
                    entity: "queue_batch",
                    id: batch.to_string(),
                })?
                .closed_at
                .is_none();
            if !open {
                return Err(StoreError::Constraint(batch_is_closed(batch)));
            }
        }
        let snapshot = serde_json::to_value(&new.graph_snapshot).map_err(|error| {
            StoreError::Constraint(format!("run.graph_snapshot does not serialise: {error}"))
        })?;

        let row = Run {
            id: new.id,
            project_id: new.project_id,
            item_id: Some(new.item_id),
            kind: RunKind::Graph,
            mode: new.mode,
            status: RunStatus::Queued,
            target_box_id: new.target_box_id,
            executing_box_id: None,
            graph_snapshot: Some(snapshot),
            started_by: new.started_by,
            // Postgres keeps `timestamptz` to the microsecond (sqlx truncates toward zero), and
            // `(queued_at, id)` order must tie where it ties there (MOD-37 R-29).
            queued_at: new.queued_at.trunc_subsecs(TIMESTAMPTZ_DIGITS),
            started_at: None,
            finished_at: None,
            failure: None,
            repo_scope: new.repo_scope,
            lease_box_id: None,
            lease_expires_at: None,
            updated_at: now,
        };
        self.runs.insert(row.id, row.clone());
        if let Some(batch) = new.batch_id {
            self.run_batches.insert(row.id, batch);
        }
        self.transition(new.item_id, status, Status::Queued, now)?;
        Ok(row)
    }

    /// ANA-2 §4.7's admission and `R-ORCH-10`'s claim-time check (MOD-7 milestone 3, D80), decided
    /// before any write: a refusal writes nothing, except `MissingTags`, which fails the run and
    /// blocks its item.
    fn claim_run(
        &mut self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
        now: DateTime<Utc>,
    ) -> Result<Claim> {
        let claimed = self.require_run(run)?.clone();
        if !self.boxes.contains_key(&box_id) {
            return Err(StoreError::NotFound {
                entity: "box",
                id: box_id.to_string(),
            });
        }
        if claimed.status != RunStatus::Queued || claimed.target_box_id != box_id {
            return Ok(Claim::NotClaimable);
        }

        // R-ORCH-10 at claim (MOD-7 milestone 3, D80, D81): after claimability, before the slot
        // and the overlap, so a permanent refusal wins over a transient one. A run whose item row
        // is absent has no tags to check, as the Postgres join finds none (D94). A chat run is not
        // such a run: `start_chat_run` inserts at `running`, so it is `NotClaimable` at the status
        // half above and never reaches this check (MOD-58).
        let missing = claimed
            .item_id
            .and_then(|item| self.items.get(&item))
            .map(|row| self.missing_for(&row.required_tags, box_id))
            .unwrap_or_default();
        if !missing.is_empty() {
            if let Some(row) = self.runs.get_mut(&run) {
                row.status = RunStatus::Failed;
                row.failure = Some(missing_tags_failure(&missing));
                row.finished_at = row.finished_at.or(Some(at));
                row.updated_at = now;
            }
            if let Some(item) = claimed.item_id
                && self
                    .items
                    .get(&item)
                    .is_some_and(|row| row.status == Status::Queued)
            {
                // A stale item status is not a refusal, as in the admitted branch below.
                self.transition(item, Status::Queued, Status::Blocked, now)?;
            }
            return Ok(Claim::MissingTags { missing });
        }

        // Two predicates over two sets, which §4.7 draws apart on purpose. The slot count is
        // `status = 'running'` alone — "an `awaiting_approval` run consumes no compute and must
        // not hold a slot" — while the overlap predicate "ranges over non-terminal runs, including
        // `awaiting_approval` ones, because a parked run still owns its trees and its unmerged
        // branch" (invariant 6). A `queued` run is in neither: it has no `executing_box_id` and no
        // tree.
        let mut live: Vec<&Run> = self
            .runs
            .values()
            .filter(|row| {
                row.executing_box_id == Some(box_id)
                    && matches!(row.status, RunStatus::Running | RunStatus::AwaitingApproval)
            })
            .collect();
        let running = rows(
            live.iter()
                .filter(|row| row.status == RunStatus::Running)
                .count(),
        );
        let limit = self.max_concurrent_items(box_id);
        if running >= u64::from(limit) {
            return Ok(Claim::SlotFull { running, limit });
        }
        // §4.7's rules L, I, P over the rows that share a repo with the claim, the first hit in
        // `(queued_at, id)` order naming the holder (plan D83, D111). A scope-less snapshot reads
        // conservatively, so a pre-milestone-5 run overlaps on any shared repo; an empty
        // `repo_scope` shares none and is never refused for overlap (hazard H-10).
        live.sort_unstable_by_key(|row| (row.queued_at, row.id));
        let scope = |row: &Run| {
            scope_of(
                row.graph_snapshot.as_ref().unwrap_or(&Value::Null),
                &row.repo_scope,
            )
        };
        let mine = scope(&claimed);
        if let Some(verdict) = live
            .iter()
            .filter(|row| {
                row.repo_scope
                    .iter()
                    .any(|repo| claimed.repo_scope.contains(repo))
            })
            .find_map(|row| {
                overlaps(&mine, &scope(row)).map(|rule| Claim::Overlaps { with: row.id, rule })
            })
        {
            return Ok(verdict);
        }

        if let Some(row) = self.runs.get_mut(&run) {
            row.status = RunStatus::Running;
            row.executing_box_id = Some(box_id);
            row.started_at = row.started_at.or(Some(at));
            row.lease_box_id = Some(box_id);
            row.lease_expires_at = Some(now + ttl);
            row.updated_at = now;
        }
        self.lease_owners.insert(run, owner);
        if let Some(item) = claimed.item_id
            && self
                .items
                .get(&item)
                .is_some_and(|row| row.status == Status::Queued)
        {
            // A stale item status is not a refusal: the run is what is being claimed.
            self.transition(item, Status::Queued, Status::InProgress, now)?;
        }
        Ok(Claim::Admitted)
    }

    /// ANA-2 §4.9's heartbeat: a compare-and-set on `lease_owner`, not on the expiry.
    fn refresh_lease(
        &mut self,
        run: RunId,
        owner: Uuid,
        ttl: TimeDelta,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.require_run(run)?;
        if self.lease_owners.get(&run) != Some(&owner) {
            return Ok(false);
        }
        if let Some(row) = self.runs.get_mut(&run) {
            row.lease_expires_at = Some(now + ttl);
            row.updated_at = now;
        }
        Ok(true)
    }

    /// ANA-2 §4.9's sweep: every abandoned lease of a graph run on the box that is not already
    /// `owner`'s becomes `owner`'s (plan D88). Expiry is judged by this handle's clock (MOD-40
    /// plan D10). A chat run is never the sweep's: the engine cannot recover it, and leasing it
    /// fenced the chat's own unleased writes (MOD-24 D3b).
    fn adopt_runs(
        &mut self,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
        now: DateTime<Utc>,
    ) -> Vec<Run> {
        let mut abandoned: Vec<(DateTime<Utc>, RunId)> = self
            .runs
            .values()
            .filter(|row| {
                row.status == RunStatus::Running
                    && row.kind == RunKind::Graph
                    && row.executing_box_id == Some(box_id)
                    && row.lease_expires_at.is_none_or(|until| until <= now)
                    // Plan D88: never this process's own lease, even an expired one.
                    && self.lease_owners.get(&row.id) != Some(&owner)
            })
            .map(|row| (row.queued_at, row.id))
            .collect();
        abandoned.sort_unstable();

        let mut adopted = Vec::with_capacity(abandoned.len());
        for (_, id) in abandoned {
            self.lease_owners.insert(id, owner);
            if let Some(row) = self.runs.get_mut(&id) {
                row.lease_box_id = Some(box_id);
                row.lease_expires_at = Some(now + ttl);
                row.updated_at = now;
                adopted.push(row.clone());
            }
        }
        adopted
    }

    /// Plan D87: the lease of a run that is ours or free; `false` writes nothing.
    fn take_lease(
        &mut self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.require_run(run)?;
        let ours_or_free = self
            .lease_owners
            .get(&run)
            .is_none_or(|held| *held == owner);
        let Some(row) = self.runs.get_mut(&run) else {
            return Ok(false);
        };
        let takeable = matches!(row.status, RunStatus::Running | RunStatus::AwaitingApproval)
            && row.executing_box_id == Some(box_id)
            && (ours_or_free || row.lease_expires_at.is_none_or(|expiry| expiry <= now));
        if !takeable {
            return Ok(false);
        }
        row.lease_box_id = Some(box_id);
        row.lease_expires_at = Some(now + ttl);
        row.updated_at = now;
        self.lease_owners.insert(run, owner);
        Ok(true)
    }

    /// Plan D139: a compare-and-set on `lease_owner` that clears it; `false` writes nothing.
    fn release_lease(&mut self, run: RunId, owner: Uuid, now: DateTime<Utc>) -> Result<bool> {
        self.require_run(run)?;
        if self.lease_owners.get(&run) != Some(&owner) {
            return Ok(false);
        }
        self.lease_owners.remove(&run);
        if let Some(row) = self.runs.get_mut(&run) {
            row.lease_expires_at = Some(now);
            row.updated_at = now;
        }
        Ok(true)
    }

    /// MOD-78 D1: [`State::fence_holds`]'s predicate on a run. `lease_owners` holds a run only
    /// while its lease names an owner, so a missing entry is Postgres's `NULL`.
    fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
        self.require_run(run)?;
        Ok(self.lease_owners.get(&run).copied() == fence.owner())
    }

    /// A `run_step` at `pending` with every settle column `NULL`.
    fn create_step(&mut self, new: NewRunStep, now: DateTime<Utc>) -> Result<RunStep> {
        if self.steps.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists("run_step", new.id)));
        }
        if !self.runs.contains_key(&new.run_id) {
            return Err(StoreError::Constraint(references_no_row(
                "run_step.run_id",
                new.run_id,
                "run",
            )));
        }
        if let Some(agent) = new.agent_id
            && !self.agents.contains_key(&agent)
        {
            return Err(StoreError::Constraint(references_no_row(
                "run_step.agent_id",
                agent,
                "agent",
            )));
        }
        if self.steps.values().any(|row| {
            row.run_id == new.run_id
                && row.position == new.position
                && row.attempt == new.attempt
                && row.fanout_index == new.fanout_index
        }) {
            return Err(StoreError::Constraint(step_slot_is_taken(
                new.run_id,
                new.position,
                new.attempt,
                new.fanout_index,
            )));
        }

        let row = RunStep {
            id: new.id,
            run_id: new.run_id,
            position: new.position,
            attempt: new.attempt,
            fanout_index: new.fanout_index,
            phase_name: new.phase_name,
            agent_id: new.agent_id,
            model: new.model,
            status: StepStatus::Pending,
            gate_outcome: None,
            gate_note: None,
            selected: None,
            exit_code: None,
            prompt_digest: None,
            trim_record: None,
            usage: None,
            isolation_path: None,
            started_at: None,
            finished_at: None,
            verify_outcome: None,
            verify_exit_code: None,
            promoted_at: None,
            updated_at: now,
        };
        self.steps.insert(row.id, row.clone());
        Ok(row)
    }

    /// [`State::transition`]'s shape for `run.status`, with §4.3's two stamps.
    fn transition_run(
        &mut self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let row = self
            .runs
            .get_mut(&run)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            })?;
        legal_move(from, to)?;
        if row.status != from {
            return Ok(false);
        }
        row.status = to;
        if to == RunStatus::Running {
            row.started_at = row.started_at.or(Some(at));
        }
        if to.is_terminal() {
            row.finished_at = row.finished_at.or(Some(at));
        }
        row.updated_at = now;
        Ok(true)
    }

    /// The `run_step` twin of [`State::transition_run`].
    fn transition_step(
        &mut self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        legal_move(from, to)?;
        if row.status != from {
            return Ok(false);
        }
        row.status = to;
        if to == StepStatus::Running {
            row.started_at = row.started_at.or(Some(at));
        }
        if to.is_terminal() {
            row.finished_at = row.finished_at.or(Some(at));
        }
        row.updated_at = now;
        Ok(true)
    }

    /// The settle columns of [`StepOutcome`] and never `status`.
    fn finish_step(
        &mut self,
        fence: StepFence,
        step: StepId,
        outcome: StepOutcome,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        Self::fence_holds(&self.lease_owners, row, fence)?;
        row.exit_code = outcome.exit_code;
        // The two the assembler and the usage summer own: `None` leaves the column.
        if outcome.usage.is_some() {
            row.usage = outcome.usage;
        }
        if outcome.trim_record.is_some() {
            row.trim_record = outcome.trim_record;
        }
        row.verify_outcome = outcome.verify_outcome;
        row.verify_exit_code = outcome.verify_exit_code;
        row.finished_at = Some(outcome.finished_at);
        row.updated_at = now;
        Ok(())
    }

    /// Plan D89: `running -> failed` with the note, `gate_outcome` untouched.
    fn interrupt_step(
        &mut self,
        step: StepId,
        note: &str,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        if row.status != StepStatus::Running {
            return Ok(false);
        }
        row.status = StepStatus::Failed;
        row.gate_note = Some(note.to_owned());
        row.finished_at = row.finished_at.or(Some(at));
        row.updated_at = now;
        Ok(true)
    }

    /// MOD-37 R-5: `running -> done` with `gate_outcome = skipped`, under `fence`. Existence,
    /// then the fence, then the status, as Postgres decides them.
    fn pass_step(
        &mut self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        Self::fence_holds(&self.lease_owners, row, fence)?;
        if row.status != StepStatus::Running {
            return Ok(false);
        }
        row.status = StepStatus::Done;
        row.gate_outcome = Some(GateOutcome::Skipped);
        if let Some(note) = note {
            row.gate_note = Some(note.to_owned());
        }
        row.finished_at = row.finished_at.or(Some(at));
        row.updated_at = now;
        Ok(true)
    }

    /// `R-ORCH-2`'s four answers, a compare-and-set on `awaiting_approval`.
    fn answer_gate(
        &mut self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        if row.status != StepStatus::AwaitingApproval {
            return Ok(false);
        }
        row.status = match outcome {
            GateOutcome::Approved | GateOutcome::Skipped => StepStatus::Done,
            GateOutcome::Rejected => StepStatus::Failed,
            GateOutcome::Retried => StepStatus::Superseded,
        };
        row.gate_outcome = Some(outcome);
        row.gate_note = note;
        row.finished_at = row.finished_at.or(Some(at));
        row.updated_at = now;
        Ok(true)
    }

    /// ANA-2 §4.5's bookkeeping: everything is validated before the first candidate is touched,
    /// which is what makes the whole selection one transaction (plan D6).
    ///
    /// The judge row is settled directly rather than through [`legal_move`], but only from
    /// `pending | running | awaiting_approval`: §4.5 makes the judge's `done` part of the winner's
    /// outcome, so a judge that never left `pending` — a human answering the fan-out itself — must
    /// not turn the selection into a refusal, while a judge that already reached an outcome keeps
    /// it. §4.5's judge-failure path (`docs/ANA-2.md:858-862`) is the case that forces the guard:
    /// the judge parks the run at `awaiting_approval` with its reason in `gate_note` and *this
    /// method is the human's pick*, so settling it would be the `failed -> done` the §4.3 table
    /// rejects, over the top of the reason the human chose from. "Nothing is lost" is that
    /// sentence. It is the treatment the losers get for the same reason (hazard H-13), and a
    /// judge left alone is left alone whole: no `status`, no `gate_note`, no `updated_at`.
    fn select_fanout(
        &mut self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self.require_step(winner)?;
        if row.run_id != run
            || row.position != position
            || row.attempt != attempt
            || row.fanout_index < 0
        {
            return Err(StoreError::Constraint(not_a_fanout_candidate(
                winner, run, position, attempt,
            )));
        }
        if !matches!(row.status, StepStatus::AwaitingApproval | StepStatus::Done) {
            return Err(StoreError::Constraint(winner_is_not_settled(
                winner, row.status,
            )));
        }

        let slot =
            |row: &RunStep| row.run_id == run && row.position == position && row.attempt == attempt;
        let losers: Vec<StepId> = self
            .steps
            .values()
            .filter(|row| slot(row) && row.fanout_index >= 0 && row.id != winner)
            .map(|row| row.id)
            .collect();
        let judges: Vec<StepId> = self
            .steps
            .values()
            .filter(|row| slot(row) && row.fanout_index < 0)
            .map(|row| row.id)
            .collect();

        if let Some(row) = self.steps.get_mut(&winner) {
            row.selected = Some(true);
            row.status = StepStatus::Done;
            row.updated_at = now;
        }
        for id in losers {
            if let Some(row) = self.steps.get_mut(&id) {
                row.selected = Some(false);
                if matches!(
                    row.status,
                    StepStatus::Pending | StepStatus::AwaitingApproval | StepStatus::Done
                ) {
                    row.status = StepStatus::Superseded;
                }
                row.updated_at = now;
            }
        }
        for id in judges {
            if let Some(row) = self.steps.get_mut(&id)
                && matches!(
                    row.status,
                    StepStatus::Pending | StepStatus::Running | StepStatus::AwaitingApproval
                )
            {
                row.status = StepStatus::Done;
                row.gate_note.clone_from(&reason);
                row.updated_at = now;
            }
        }
        Ok(())
    }

    /// §4.4's loop half; the law's table is what refuses, not a stale compare-and-set.
    fn supersede_step(&mut self, step: StepId, now: DateTime<Utc>) -> Result<()> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        legal_move(row.status, StepStatus::Superseded)?;
        row.status = StepStatus::Superseded;
        row.updated_at = now;
        Ok(())
    }

    /// Every row of a tree or commit batch belongs to `step` and names a repo that exists; the
    /// whole batch is checked before the first insert, as [`State::append_events`] does.
    ///
    /// MOD-41 plan D1 (blueprint B-1): with `Some(fence)`, the step's run must carry that lease,
    /// checked after the step's existence and before the rows. `close_out`'s per-commit check
    /// passes `None`: close-out runs on a finished item with no lease, and Postgres's `close_out`
    /// checks existence only.
    fn check_step_batch(
        &self,
        table: &str,
        step: StepId,
        fence: Option<StepFence>,
        rows: impl IntoIterator<Item = (StepId, RepoId)>,
    ) -> Result<()> {
        let row = self.require_step(step)?;
        if let Some(fence) = fence {
            Self::fence_holds(&self.lease_owners, row, fence)?;
        }
        for (run_step_id, repo_id) in rows {
            if run_step_id != step {
                return Err(StoreError::Constraint(row_names_another_step(
                    table,
                    run_step_id,
                    step,
                )));
            }
            if !self.repos.contains_key(&repo_id) {
                return Err(StoreError::Constraint(references_no_row(
                    &format!("{table}.repo_id"),
                    repo_id,
                    "repo",
                )));
            }
        }
        Ok(())
    }

    /// `run_step_tree` upserted on `(run_step_id, repo_id)` (ANA-2 §4.6), and the one column of
    /// `run_step` the batch also speaks for: `isolation_path` (ANA-2 `:903`, plan D33).
    ///
    /// `run_step_tree` holds a path per repository and `run_step.isolation_path` holds one path,
    /// so the batch has to choose which of its trees the step *is*. The primary repo's, else the
    /// lowest `repo_id`'s — which is the order [`State::step_tree_rows`] answers in, so the chosen
    /// row is the first one a reader sees. The rule is spelled as one sort key,
    /// `(not primary, repo_id)`, rather than a `find` with a fallback, so that a project carrying
    /// two primaries — which the schema permits — still resolves the same way here and on
    /// Postgres instead of to whichever row the caller happened to list first.
    ///
    /// An empty batch names no tree and leaves the column alone rather than clearing it:
    /// `upsert_step_tree(step, &[])` is the "check the step, write nothing" call, and clearing
    /// would make it a write.
    fn upsert_step_tree(
        &mut self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.check_step_batch(
            "run_step_tree",
            step,
            Some(fence),
            trees.iter().map(|row| (row.run_step_id, row.repo_id)),
        )?;
        let chosen = trees
            .iter()
            .min_by_key(|row| {
                let primary = self
                    .repos
                    .get(&row.repo_id)
                    .is_some_and(|repo| repo.is_primary);
                (!primary, row.repo_id)
            })
            .map(|row| row.path.clone());
        for row in trees {
            self.step_trees.insert((step, row.repo_id), row.clone());
        }
        if let Some(path) = chosen {
            // `require_step` above already proved the row is here.
            if let Some(row) = self.steps.get_mut(&step) {
                row.isolation_path = Some(path);
                row.updated_at = now;
            }
        }
        Ok(())
    }

    /// `run_step_commit` upserted on the same key (`R-ORCH-11`).
    fn record_commits(
        &mut self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> Result<()> {
        self.check_step_batch(
            "run_step_commit",
            step,
            Some(fence),
            commits.iter().map(|row| (row.run_step_id, row.repo_id)),
        )?;
        for row in commits {
            self.step_commits.insert((step, row.repo_id), row.clone());
        }
        Ok(())
    }

    /// One `command_run` row, every column the caller's (plan D31).
    ///
    /// The three refusals are ordered as the contract states them: the step first, so an input
    /// wrong in two ways is `NotFound` rather than the `Constraint` the box alone would earn; then
    /// the box, which Postgres answers with a foreign key; then the id, which Postgres answers
    /// with the primary key.
    fn record_command_run(&mut self, new: NewCommandRun) -> Result<CommandRun> {
        self.require_step(new.run_step_id)?;
        if !self.boxes.contains_key(&new.box_id) {
            return Err(StoreError::Constraint(references_no_row(
                "command_run.box_id",
                new.box_id,
                "box",
            )));
        }
        if self.command_runs.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists(
                "command_run",
                new.id,
            )));
        }
        let row = CommandRun::from(new);
        self.command_runs.insert(row.id, row.clone());
        Ok(row)
    }

    /// MOD-11 D14, B-16: [`State::record_command_run`] for a row that has not started, its
    /// heartbeat stamped `now` (R-3: a waiter that dies before its first claim is reaped too).
    fn enqueue_command(&mut self, new: NewCommandRun, now: DateTime<Utc>) -> Result<CommandRun> {
        if new.status != CommandRunStatus::Queued
            || new.started_at.is_some()
            || new.finished_at.is_some()
            || new.exit_code.is_some()
            || new.output.is_some()
        {
            return Err(StoreError::Constraint(command_not_queued()));
        }
        let row = self.record_command_run(new)?;
        self.command_claims.insert(
            row.id,
            CommandClaim {
                claimant: None,
                heartbeat_at: now,
            },
        );
        Ok(row)
    }

    /// One `command_run` row, or the `NotFound` the queue's writers open with.
    fn require_command_run(&self, id: CommandRunId) -> Result<&CommandRun> {
        self.command_runs
            .get(&id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "command_run",
                id: id.to_string(),
            })
    }

    /// MOD-11 D14: the claim. The `write` closure this runs in is Postgres's advisory lock: the
    /// reap, the count and the admission see one state. Refusals (`NotFound`, not `queued`) come
    /// before the reap, as Postgres decides them on the row it locked first; then the asking row
    /// is beaten (its waiter is alive), then the pair's stale rows are reaped: `running` ones
    /// fail (OQ-3), `queued` ones whose waiter stopped asking are cancelled (R-3).
    fn claim_command(
        &mut self,
        id: CommandRunId,
        claimant: Uuid,
        limit: u32,
        now: DateTime<Utc>,
    ) -> Result<Option<CommandRun>> {
        let row = self.require_command_run(id)?;
        if row.status != CommandRunStatus::Queued {
            return Err(StoreError::Constraint(command_not_claimable(row.status)));
        }
        let (box_id, class) = (row.box_id, row.class.clone());
        let of_pair = |row: &CommandRun| row.box_id == box_id && row.class == class;
        // R-3: asking is the queued row's beat.
        self.command_claims.insert(
            id,
            CommandClaim {
                claimant: None,
                heartbeat_at: now,
            },
        );

        // OQ-3: the pair's stale `running` rows. A row no claim admitted (a `running` row
        // `record_command_run` wrote) goes by its start, then its queueing, as Postgres's
        // `COALESCE(heartbeat_at, started_at, queued_at)` does.
        let stale_before = now - COMMAND_STALE_AFTER;
        let claims = &self.command_claims;
        let stale: Vec<CommandRunId> = self
            .command_runs
            .values()
            .filter(|row| of_pair(row) && row.status == CommandRunStatus::Running)
            .filter(|row| {
                let beat = claims
                    .get(&row.id)
                    .map(|claim| claim.heartbeat_at)
                    .or(row.started_at)
                    .unwrap_or(row.queued_at);
                beat < stale_before
            })
            .map(|row| row.id)
            .collect();
        // R-3: the pair's `queued` rows whose waiter stopped asking. A row with no beat (one
        // `record_command_run` wrote `queued`) is never reaped, as Postgres's `NULL` is not.
        let orphans: Vec<CommandRunId> = self
            .command_runs
            .values()
            .filter(|row| of_pair(row) && row.status == CommandRunStatus::Queued)
            .filter(|row| {
                claims
                    .get(&row.id)
                    .is_some_and(|claim| claim.heartbeat_at < stale_before)
            })
            .map(|row| row.id)
            .collect();
        let reaps = stale
            .into_iter()
            .map(|id| (id, CommandRunStatus::Failed))
            .chain(
                orphans
                    .into_iter()
                    .map(|id| (id, CommandRunStatus::Cancelled)),
            );
        for (reaped, status) in reaps {
            self.command_claims.remove(&reaped);
            if let Some(row) = self.command_runs.get_mut(&reaped) {
                row.status = status;
                row.finished_at = Some(now);
                row.output = Some(match row.output.take() {
                    Some(output) => format!("{output}\n{}", reaped_note()),
                    None => reaped_note(),
                });
            }
        }

        let running = self
            .command_runs
            .values()
            .filter(|row| of_pair(row) && row.status == CommandRunStatus::Running)
            .count();
        let oldest = self
            .command_runs
            .values()
            .filter(|row| of_pair(row) && row.status == CommandRunStatus::Queued)
            .min_by(|left, right| {
                left.queued_at
                    .cmp(&right.queued_at)
                    .then_with(|| left.id.cmp(&right.id))
            })
            .map(|row| row.id);
        if running >= limit.max(1) as usize || oldest != Some(id) {
            return Ok(None);
        }
        let Some(row) = self.command_runs.get_mut(&id) else {
            return Ok(None);
        };
        row.status = CommandRunStatus::Running;
        row.started_at = Some(now);
        self.command_claims.insert(
            id,
            CommandClaim {
                claimant: Some(claimant),
                heartbeat_at: now,
            },
        );
        Ok(Some(row.clone()))
    }

    /// Whether `id` is `running` under `claimant`.
    fn holds_command(&self, id: CommandRunId, claimant: Uuid) -> bool {
        self.command_runs
            .get(&id)
            .is_some_and(|row| row.status == CommandRunStatus::Running)
            && self
                .command_claims
                .get(&id)
                .is_some_and(|claim| claim.claimant == Some(claimant))
    }

    /// MOD-11 D14: the claimant's beat.
    fn beat_command(&mut self, id: CommandRunId, claimant: Uuid, now: DateTime<Utc>) -> bool {
        if !self.holds_command(id, claimant) {
            return false;
        }
        if let Some(claim) = self.command_claims.get_mut(&id) {
            claim.heartbeat_at = now;
        }
        true
    }

    /// MOD-11 D14: the claimant's end of a row; the status is checked before anything is read.
    fn finish_command(
        &mut self,
        id: CommandRunId,
        claimant: Uuid,
        status: CommandRunStatus,
        exit_code: Option<i32>,
        output: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        if matches!(status, CommandRunStatus::Queued | CommandRunStatus::Running) {
            return Err(StoreError::Constraint(command_finish_status(status)));
        }
        if !self.holds_command(id, claimant) {
            return Ok(false);
        }
        self.command_claims.remove(&id);
        if let Some(row) = self.command_runs.get_mut(&id) {
            row.status = status;
            row.exit_code = exit_code;
            row.output = output;
            row.finished_at = Some(now);
        }
        Ok(true)
    }

    /// MOD-11 D14: `queued | running → cancelled`; a terminal row is left as it is.
    fn cancel_command(&mut self, id: CommandRunId, now: DateTime<Utc>) -> Result<bool> {
        let status = self.require_command_run(id)?.status;
        if !matches!(status, CommandRunStatus::Queued | CommandRunStatus::Running) {
            return Ok(false);
        }
        self.command_claims.remove(&id);
        if let Some(row) = self.command_runs.get_mut(&id) {
            row.status = CommandRunStatus::Cancelled;
            row.finished_at = Some(now);
        }
        Ok(true)
    }

    /// A step's `command_run` rows in `(queued_at, id)` order; an unknown step reads empty.
    fn command_runs(&self, step: StepId) -> Vec<CommandRun> {
        let mut rows: Vec<CommandRun> = self
            .command_runs
            .values()
            .filter(|row| row.run_step_id == step)
            .cloned()
            .collect();
        rows.sort_by(|left, right| {
            left.queued_at
                .cmp(&right.queued_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        rows
    }

    /// The document at `max(version) + 1` for its `(item, kind)` (plan D6).
    fn write_document(&mut self, new: NewDocument) -> Result<Document> {
        self.require_item(new.item_id)?;
        if self.documents.iter().any(|row| row.id == new.id) {
            return Err(StoreError::Constraint(already_exists("document", new.id)));
        }
        self.require_user(new.created_by, "document.created_by")?;
        if let Some(step) = new.produced_by_step_id
            && !self.steps.contains_key(&step)
        {
            return Err(StoreError::Constraint(references_no_row(
                "document.produced_by_step_id",
                step,
                "run_step",
            )));
        }
        let version = self
            .documents
            .iter()
            .filter(|row| row.item_id == new.item_id && row.kind == new.kind)
            .map(|row| row.version)
            .max()
            .unwrap_or(0)
            + 1;
        let row = Document {
            id: new.id,
            item_id: new.item_id,
            kind: new.kind,
            version,
            title: new.title,
            body: new.body,
            produced_by_step_id: new.produced_by_step_id,
            created_by: new.created_by,
            created_at: new.created_at,
        };
        self.documents.push(row.clone());
        Ok(row)
    }

    /// MOD-37 M5: the step's opening, replacing any earlier one; the step's `updated_at` moves.
    fn record_opening(
        &mut self,
        step: StepId,
        opening: StepOpening,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self
            .steps
            .get_mut(&step)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            })?;
        row.updated_at = now;
        self.openings.insert(step, opening);
        Ok(())
    }

    /// §4.8's promotion: the step, its run and its item, in one closure (plan D6).
    fn promote_step(&mut self, step: StepId, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<()> {
        let row = self.require_step(step)?;
        if !matches!(
            row.status,
            StepStatus::Failed | StepStatus::AwaitingApproval
        ) {
            return Err(StoreError::Constraint(step_is_not_promotable(
                step, row.status,
            )));
        }
        let run_id = row.run_id;
        let run = self.require_run(run_id)?;
        if run.status.is_terminal() {
            return Err(StoreError::Constraint(run_is_terminal(run_id, run.status)));
        }
        let run_status = run.status;
        let item_id = run.item_id;

        if let Some(row) = self.steps.get_mut(&step) {
            row.status = StepStatus::AwaitingApproval;
            row.promoted_at = Some(at);
            row.updated_at = now;
        }
        if run_status == RunStatus::Running
            && let Some(row) = self.runs.get_mut(&run_id)
        {
            row.status = RunStatus::AwaitingApproval;
            row.updated_at = now;
        }
        if let Some(item) = item_id
            && self
                .items
                .get(&item)
                .is_some_and(|row| row.status == Status::InProgress)
        {
            self.transition(item, Status::InProgress, Status::AwaitingApproval, now)?;
        }
        Ok(())
    }

    /// MOD-37 R-5: the gate's park, `promote_step`'s shape. Every check runs before the first
    /// write, so the park is all-or-nothing by construction, as Postgres's rollback makes it
    /// (review N1). The step, the fence, the step's status and the run's status run in
    /// Postgres's order. The item check after them is Mem-only: Postgres's item `UPDATE` is
    /// guarded by `status = 'in_progress'` and checks nothing, and `run.item_id`'s FK cascade
    /// guarantees the row Mem checks for. The item must exist and its
    /// `in_progress -> awaiting_approval` be legal; an item another writer moved is left where it
    /// is (plan D17's "already moved").
    fn park_step(
        &mut self,
        fence: StepFence,
        step: StepId,
        now: DateTime<Utc>,
    ) -> Result<ParkOutcome> {
        let row = self.require_step(step)?;
        Self::fence_holds(&self.lease_owners, row, fence)?;
        if row.status != StepStatus::Running {
            return Ok(ParkOutcome::StepMoved);
        }
        let run_id = row.run_id;
        let run = self.require_run(run_id)?;
        if run.status != RunStatus::Running {
            return Ok(ParkOutcome::RunMoved);
        }
        let move_item = match run.item_id {
            // A chat run has no item to move.
            None => None,
            Some(item) => {
                let row = self.items.get(&item).ok_or_else(|| StoreError::NotFound {
                    entity: "item",
                    id: item.to_string(),
                })?;
                legal_move(Status::InProgress, Status::AwaitingApproval)?;
                (row.status == Status::InProgress).then_some(item)
            }
        };

        // Every check passed: the three moves below cannot fail.
        if let Some(row) = self.steps.get_mut(&step) {
            row.status = StepStatus::AwaitingApproval;
            row.updated_at = now;
        }
        if let Some(row) = self.runs.get_mut(&run_id) {
            row.status = RunStatus::AwaitingApproval;
            row.updated_at = now;
        }
        if let Some(row) = move_item.and_then(|item| self.items.get_mut(&item)) {
            // `transition`'s write: `awaiting_approval` is live, so `closed_at` clears.
            row.status = Status::AwaitingApproval;
            row.updated_at = now;
            row.closed_at = None;
        }
        Ok(ParkOutcome::Parked)
    }

    /// §4.3's failure row, from any non-terminal status.
    fn fail_run(
        &mut self,
        run: RunId,
        failure: &str,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self
            .runs
            .get_mut(&run)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            })?;
        legal_move(row.status, RunStatus::Failed)?;
        row.status = RunStatus::Failed;
        row.failure = Some(failure.to_owned());
        row.finished_at = row.finished_at.or(Some(at));
        row.updated_at = now;
        Ok(())
    }

    /// Plan M2 D7: the run's terminal move and the item's mirror, one closure.
    ///
    /// The item half is [`State::promote_step`]'s shape — derive the target, then route it through
    /// [`State::transition`] so `closed_at` and `updated_at` follow one rule — and the run half is
    /// [`State::transition_run`]'s. What is new is the guard between them: the mirror is written
    /// only when no *other* run of the item is still active, so a second live run holds the item
    /// where it is rather than being lost.
    fn finish_run(
        &mut self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        // Plan D14's order, and every refusal is decided before the first write: lookup, then the
        // two rules this writer owns, then the law.
        let row = self.require_run(run)?;
        let (from, item_id) = (row.status, row.item_id);
        if !to.is_terminal() {
            return Err(StoreError::Constraint(finish_run_needs_a_terminal_status(
                run, to,
            )));
        }
        if failure.is_some() != (to == RunStatus::Failed) {
            return Err(StoreError::Constraint(failure_disagrees_with_status(
                run,
                to,
                failure.is_some(),
            )));
        }
        // A terminal row reaches nothing, so `run_is_terminal` would only say the same thing in a
        // second sentence: the law already refuses `done -> done`.
        legal_move(from, to)?;

        if let Some(row) = self.runs.get_mut(&run) {
            row.status = to;
            if let Some(text) = failure {
                row.failure = Some(text.to_owned());
            }
            row.finished_at = row.finished_at.or(Some(at));
            row.updated_at = now;
        }

        let Some(item) = item_id else {
            // A chat run has no item to mirror (§5.8: `item_id` is nullable for `kind = 'chat'`).
            return Ok(());
        };
        if self
            .runs
            .values()
            .any(|other| other.item_id == Some(item) && other.id != run && other.status.is_active())
        {
            return Ok(());
        }
        let Some(status) = self.items.get(&item).map(|row| row.status) else {
            return Ok(());
        };
        // §4.3's verdict table lives in `traits.rs` so `PgStore` binds the same answer this
        // matches on; `None` is plan D17's "leave an unexpected item alone".
        let Some(target) = finish_run_item_mirror(to, status) else {
            return Ok(());
        };
        self.transition(item, status, target, now)?;
        Ok(())
    }

    /// `R-TUI-9`'s three effects, one closure: everything refusable is decided first.
    fn close_out(
        &mut self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
        now: DateTime<Utc>,
    ) -> Result<Document> {
        let status = self.require_item(item)?.status;
        if let Some(live) = self
            .runs
            .values()
            .filter(|row| row.item_id == Some(item) && row.status.is_active())
            .min_by_key(|row| (row.queued_at, row.id))
        {
            return Err(StoreError::Constraint(item_has_a_live_run(
                item,
                live.id,
                live.status,
            )));
        }
        if summary.kind != "summary" {
            return Err(StoreError::Constraint(close_out_needs_a_summary(
                &summary.kind,
            )));
        }
        if summary.item_id != item {
            return Err(StoreError::Constraint(summary_names_another_item(
                item,
                summary.item_id,
            )));
        }
        // ANA-11 §4.2's law, not §4.3's table: close-out is the only way into `closed` (MOD-38
        // PRD D1), so `legal_move` would refuse every status.
        if !resolution.closes_from(status) {
            return Err(StoreError::Constraint(resolution_not_closable(
                item, status, resolution,
            )));
        }
        for row in commits {
            self.check_step_batch(
                "run_step_commit",
                row.run_step_id,
                None,
                std::iter::once((row.run_step_id, row.repo_id)),
            )?;
        }

        let document = self.write_document(summary)?;
        for row in commits {
            self.step_commits
                .insert((row.run_step_id, row.repo_id), row.clone());
        }
        // Plan D5: the direct write `transition` no longer makes, under the same `now`.
        let row = self
            .items
            .get_mut(&item)
            .expect("require_item found it above");
        row.status = Status::Closed;
        row.resolution = Some(resolution);
        row.updated_at = now;
        row.closed_at = Some(now);
        Ok(document)
    }

    /// ANA-2 invariant 7's refusal note; every foreign key is checked before the insert.
    fn add_note(&mut self, note: NewNote) -> Result<Note> {
        if self.notes.iter().any(|row| row.id == note.id) {
            return Err(StoreError::Constraint(already_exists("item_note", note.id)));
        }
        if !self.items.contains_key(&note.item_id) {
            return Err(StoreError::Constraint(references_no_row(
                "item_note.item_id",
                note.item_id,
                "item",
            )));
        }
        self.require_user(note.created_by, "item_note.created_by")?;
        if let Some(box_id) = note.box_id
            && !self.boxes.contains_key(&box_id)
        {
            return Err(StoreError::Constraint(references_no_row(
                "item_note.box_id",
                box_id,
                "box",
            )));
        }
        if let Some(step) = note.via_step_id
            && !self.steps.contains_key(&step)
        {
            return Err(StoreError::Constraint(references_no_row(
                "item_note.via_step_id",
                step,
                "run_step",
            )));
        }
        let row = Note {
            id: note.id,
            item_id: note.item_id,
            body: note.body,
            created_by: note.created_by,
            box_id: note.box_id,
            via_step_id: note.via_step_id,
            created_at: note.created_at,
        };
        self.notes.push(row.clone());
        Ok(row)
    }

    /// MOD-11 F-21: `step` exists and its run carries `fence`'s lease (`NotFound`, then
    /// `Fenced`: Postgres's `step_scope` order); answers the run's id, item and project.
    fn step_scope(
        &self,
        step: StepId,
        fence: StepFence,
    ) -> Result<(RunId, Option<ItemId>, ProjectId)> {
        let row = self.require_step(step)?;
        Self::fence_holds(&self.lease_owners, row, fence)?;
        let run = self.require_run(row.run_id)?;
        Ok((run.id, run.item_id, run.project_id))
    }

    /// MOD-11 D13: `step_scope`, then the step's own item, then the step-less write. Every
    /// refusal is decided before [`State::write_document`] writes.
    fn write_step_document(&mut self, fence: StepFence, new: NewDocument) -> Result<Document> {
        let Some(step) = new.produced_by_step_id else {
            return Err(StoreError::Constraint(document_needs_a_step()));
        };
        if let Some(refusal) = step_document_refusal(&new) {
            return Err(StoreError::Constraint(refusal));
        }
        let (_, item, _) = self.step_scope(step, fence)?;
        if item != Some(new.item_id) {
            return Err(StoreError::Constraint(step_writes_own_item(
                step,
                new.item_id,
            )));
        }
        self.write_document(new)
    }

    /// MOD-11 D13: [`State::write_step_document`]'s refusals for a note.
    fn add_step_note(&mut self, fence: StepFence, note: NewNote) -> Result<Note> {
        let Some(step) = note.via_step_id else {
            return Err(StoreError::Constraint(note_needs_a_step()));
        };
        if let Some(refusal) = step_note_refusal(&note) {
            return Err(StoreError::Constraint(refusal));
        }
        let (_, item, _) = self.step_scope(step, fence)?;
        if item != Some(note.item_id) {
            return Err(StoreError::Constraint(step_writes_own_item(
                step,
                note.item_id,
            )));
        }
        self.add_note(note)
    }

    /// `step_graph_id` of the item, else its kind's default (ANA-2 §8's `resolve_graph`).
    fn resolve_graph(&self, item: ItemId) -> Option<ResolvedGraph> {
        let row = self.items.get(&item)?;
        let graph_id = row.step_graph_id.or_else(|| {
            self.kinds
                .get(&row.kind_id)
                .map(|kind| kind.default_graph_id)
        })?;
        let graph = self.graphs.get(&graph_id)?.clone();
        let mut phases: Vec<&StepGraphPhase> = self
            .phases
            .iter()
            .filter(|phase| phase.graph_id == graph_id)
            .collect();
        phases.sort_by_key(|phase| phase.position);
        Some(ResolvedGraph {
            graph,
            phases: phases
                .into_iter()
                .map(|phase| ResolvedPhase {
                    phase: phase.clone(),
                    agents: self.phase_agents_of(phase.id),
                    // MOD-26 D6: the row the binding names; `require_persona` keeps it present.
                    persona: phase
                        .persona_id
                        .and_then(|id| self.personas.get(&id).cloned()),
                })
                .collect(),
        })
    }

    // ---- MOD-38: ANA-11 §5.1 requirements and citations --------------------------------------
    //
    // The MOD-15 discipline again: every refusal is decided before the first write, so a writer's
    // single `write` closure is its transaction. Every ordering compares text by bytes, which is
    // what `String`'s `Ord` does and what `PgStore`'s `COLLATE "C"` does (blueprint §4.1). Suspect
    // is derived on every read and stored nowhere (plan D11).

    /// A project's areas in `(position, code)` order.
    fn requirement_area_rows(&self, project: ProjectId) -> Vec<RequirementArea> {
        let mut rows: Vec<RequirementArea> = self
            .requirement_areas
            .values()
            .filter(|row| row.project_id == project)
            .cloned()
            .collect();
        rows.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.code.cmp(&right.code))
        });
        rows
    }

    /// Whether a requirement passes every conjunct of the filter (plan D14).
    fn requirement_matches(row: &Requirement, filter: &RequirementFilter) -> bool {
        if filter
            .area_codes
            .as_ref()
            .is_some_and(|codes| !codes.contains(&row.area_code))
        {
            return false;
        }
        if filter
            .states
            .as_ref()
            .is_some_and(|states| !states.contains(&row.state))
        {
            return false;
        }
        if filter
            .priorities
            .as_ref()
            .is_some_and(|priorities| !priorities.contains(&row.priority))
        {
            return false;
        }
        if let Some(text) = &filter.text {
            let needle = text.to_lowercase();
            if !row.key.to_lowercase().contains(&needle)
                && !row.body.to_lowercase().contains(&needle)
            {
                return false;
            }
        }
        true
    }

    /// A project's matching requirements in `(area_code, number)` order.
    fn requirement_rows(&self, project: ProjectId, filter: &RequirementFilter) -> Vec<Requirement> {
        let mut rows: Vec<Requirement> = self
            .requirements
            .values()
            .filter(|row| row.project_id == project && Self::requirement_matches(row, filter))
            .cloned()
            .collect();
        rows.sort_by(|left, right| {
            left.area_code
                .cmp(&right.area_code)
                .then_with(|| left.number.cmp(&right.number))
        });
        rows
    }

    /// A requirement's revisions in `version` order; empty for an unknown id.
    fn requirement_revision_rows(&self, id: RequirementId) -> Vec<RequirementRevision> {
        let mut rows: Vec<RequirementRevision> = self
            .requirement_revisions
            .iter()
            .filter(|row| row.requirement_id == id)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.version);
        rows
    }

    /// An item's live citations, each joined to its requirement as it is now, in
    /// `(area_code, number, kind)` order.
    fn item_citations(&self, item: ItemId) -> Vec<ItemCitation> {
        let mut rows: Vec<ItemCitation> = self
            .item_requirements
            .iter()
            .filter(|row| row.item_id == item && row.deleted_at.is_none())
            .filter_map(|row| {
                let requirement = self.requirements.get(&row.requirement_id)?;
                Some(ItemCitation {
                    requirement: requirement.clone(),
                    kind: row.kind,
                    requirement_version: row.requirement_version,
                    proposed_by_step_id: row.proposed_by_step_id,
                    suspect: requirement.makes_suspect(row.requirement_version),
                })
            })
            .collect();
        rows.sort_by(|left, right| {
            left.requirement
                .area_code
                .cmp(&right.requirement.area_code)
                .then_with(|| left.requirement.number.cmp(&right.requirement.number))
                .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
        });
        rows
    }

    /// A requirement's live citations, each with its item's status and resolution, in
    /// `(key_prefix, key_number, item id, kind)` order.
    fn coverage_rows(&self, requirement: RequirementId) -> Vec<CoverageRow> {
        let Some(head) = self.requirements.get(&requirement) else {
            return Vec::new();
        };
        let mut rows: Vec<(&Item, CoverageRow)> = self
            .item_requirements
            .iter()
            .filter(|row| row.requirement_id == requirement && row.deleted_at.is_none())
            .filter_map(|row| {
                let item = self.items.get(&row.item_id)?;
                Some((
                    item,
                    CoverageRow {
                        item: item.summary(),
                        kind: row.kind,
                        resolution: item.resolution,
                        requirement_version: row.requirement_version,
                        suspect: head.makes_suspect(row.requirement_version),
                    },
                ))
            })
            .collect();
        rows.sort_by(|(left_item, left), (right_item, right)| {
            left_item
                .key_prefix
                .cmp(&right_item.key_prefix)
                .then_with(|| left_item.key_number.cmp(&right_item.key_number))
                .then_with(|| left_item.id.cmp(&right_item.id))
                .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
        });
        rows.into_iter().map(|(_, row)| row).collect()
    }

    /// `requirement_revision.box_id` / `NewRequirement::box_id` must name a `box` row, if any.
    fn require_revision_box(&self, box_id: Option<BoxId>) -> Result<()> {
        match box_id {
            Some(box_id) if !self.boxes.contains_key(&box_id) => Err(StoreError::Constraint(
                references_no_row("requirement_revision.box_id", box_id, "box"),
            )),
            _ => Ok(()),
        }
    }

    /// Compare-and-set on `requirement_spec.version` (plan D9). `None` creates the header only
    /// where there is none; a token that matches no stored row is `Stale` with the row as it is.
    fn set_requirement_spec(
        &mut self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
        now: DateTime<Utc>,
    ) -> Result<CasOutcome<RequirementSpec>> {
        match (self.requirement_specs.get(&project), expected_version) {
            (Some(row), Some(version)) if row.version == version => {}
            (Some(row), _) => return Ok(CasOutcome::Stale(row.clone())),
            (None, Some(_)) => {
                return Err(StoreError::NotFound {
                    entity: "requirement_spec",
                    id: project.to_string(),
                });
            }
            (None, None) => {
                if !self.projects.contains_key(&project) {
                    return Err(StoreError::Constraint(references_no_row(
                        "requirement_spec.project_id",
                        project,
                        "project",
                    )));
                }
            }
        }
        self.require_user(owner_id, "requirement_spec.owner_id")?;

        let row = self
            .requirement_specs
            .entry(project)
            .and_modify(|row| row.version += 1)
            .or_insert_with(|| RequirementSpec {
                project_id: project,
                owner_id,
                preamble: String::new(),
                version: 1,
                updated_at: now,
            });
        row.owner_id = owner_id;
        row.preamble = preamble;
        row.updated_at = now;
        Ok(CasOutcome::Applied(row.clone()))
    }

    /// One `requirement_area`; the code CHECK is decided first, in [`invalid_area_code`]'s words.
    fn create_requirement_area(
        &mut self,
        new: NewRequirementArea,
        now: DateTime<Utc>,
    ) -> Result<RequirementArea> {
        if !RequirementArea::code_is_valid(&new.code) {
            return Err(StoreError::Constraint(invalid_area_code(&new.code)));
        }
        if !self.projects.contains_key(&new.project_id) {
            return Err(StoreError::Constraint(references_no_row(
                "requirement_area.project_id",
                new.project_id,
                "project",
            )));
        }
        if self.requirement_areas.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists(
                "requirement_area",
                new.id,
            )));
        }
        if self
            .requirement_areas
            .values()
            .any(|row| row.project_id == new.project_id && row.code == new.code)
        {
            return Err(StoreError::Constraint(already_exists(
                "requirement_area.code",
                &new.code,
            )));
        }
        let row = RequirementArea {
            id: new.id,
            project_id: new.project_id,
            code: new.code,
            title: new.title,
            description: new.description,
            position: new.position,
            updated_at: now,
        };
        self.requirement_areas.insert(row.id, row.clone());
        Ok(row)
    }

    /// Mints a requirement: counter upsert, key assembly and revision 1 in one lock (plan D8), as
    /// [`State::mint`] does for an item. Every refusal runs before the counter moves.
    fn mint_requirement(
        &mut self,
        area: RequirementAreaId,
        new: NewRequirement,
        now: DateTime<Utc>,
    ) -> Result<Requirement> {
        let area_row = self
            .requirement_areas
            .get(&area)
            .ok_or_else(|| StoreError::NotFound {
                entity: "requirement_area",
                id: area.to_string(),
            })?;
        if self.requirements.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists(
                "requirement",
                new.id,
            )));
        }
        self.require_user(new.created_by, "requirement.created_by")?;
        self.require_revision_box(new.box_id)?;

        let project_id = area_row.project_id;
        let area_code = area_row.code.clone();
        let counter = self.requirement_key_counter.entry(area).or_insert(0);
        *counter += 1;
        let number = *counter;

        let row = Requirement {
            id: new.id,
            project_id,
            area_id: area,
            key: format!("R-{area_code}-{number}"),
            area_code,
            number,
            body: new.body,
            rationale: new.rationale,
            priority: new.priority,
            state: RequirementState::Active,
            version: 1,
            created_by: new.created_by,
            created_at: now,
            updated_at: now,
        };
        self.requirement_revisions.push(RequirementRevision {
            requirement_id: row.id,
            version: 1,
            body: row.body.clone(),
            rationale: row.rationale.clone(),
            priority: row.priority,
            state: row.state,
            author_id: new.created_by,
            box_id: new.box_id,
            reason: "created".to_owned(),
            amended_by_item_id: None,
            created_at: now,
        });
        self.requirements.insert(row.id, row.clone());
        Ok(row)
    }

    /// The amend and the withdraw, which differ only in the citation the deciding item gets: a
    /// `withdraws` one also moves the row to `withdrawn` (PRD D3, plan D10).
    ///
    /// Checked NotFound, divergence, Constraint, and only then written: the row at `version + 1`,
    /// its revision naming the deciding item, and that item's citation at the new version.
    fn revise_requirement(
        &mut self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        decided_by: (ItemId, CitationKind),
        now: DateTime<Utc>,
    ) -> Result<RequirementUpdate> {
        let (item, kind) = decided_by;
        let head = self
            .requirements
            .get(&id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "requirement",
                id: id.to_string(),
            })?;
        if head.version != expected_version {
            let ancestor = self
                .requirement_revisions
                .iter()
                .find(|row| row.requirement_id == id && row.version == expected_version)
                .cloned()
                .ok_or_else(|| StoreError::NotFound {
                    entity: "requirement_revision",
                    id: format!("{id}@{expected_version}"),
                })?;
            return Ok(RequirementUpdate::Diverged {
                head: head.clone(),
                ancestor,
            });
        }
        if head.state == RequirementState::Withdrawn {
            return Err(StoreError::Constraint(requirement_withdrawn(&head.key)));
        }
        if !self.items.contains_key(&item) {
            return Err(StoreError::Constraint(references_no_row(
                "requirement_revision.amended_by_item_id",
                item,
                "item",
            )));
        }
        self.require_user(patch.author_id, "requirement_revision.author_id")?;
        self.require_revision_box(patch.box_id)?;

        let row = self
            .requirements
            .get_mut(&id)
            .expect("the row was read a statement ago under the same lock");
        if let Some(body) = patch.body {
            row.body = body;
        }
        if let Some(rationale) = patch.rationale {
            row.rationale = rationale;
        }
        if let Some(priority) = patch.priority {
            row.priority = priority;
        }
        if kind == CitationKind::Withdraws {
            row.state = RequirementState::Withdrawn;
        }
        row.version += 1;
        row.updated_at = now;
        let head = row.clone();

        self.requirement_revisions.push(RequirementRevision {
            requirement_id: id,
            version: head.version,
            body: head.body.clone(),
            rationale: head.rationale.clone(),
            priority: head.priority,
            state: head.state,
            author_id: patch.author_id,
            box_id: patch.box_id,
            reason: patch.reason,
            amended_by_item_id: Some(item),
            created_at: now,
        });
        self.upsert_citation(item, id, kind, head.version, now);
        Ok(RequirementUpdate::Updated(head))
    }

    /// `INSERT ... ON CONFLICT (item_id, requirement_id, kind) DO UPDATE`: the row stamped at
    /// `stamp` and live, a tombstone revived. A new row has no proposing step; an existing one
    /// keeps its own, which [`State::cite`] alone overwrites.
    fn upsert_citation(
        &mut self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        stamp: i32,
        now: DateTime<Utc>,
    ) -> &mut ItemRequirement {
        let at = match self.item_requirements.iter().position(|row| {
            row.item_id == item && row.requirement_id == requirement && row.kind == kind
        }) {
            Some(at) => at,
            None => {
                self.item_requirements.push(ItemRequirement {
                    item_id: item,
                    requirement_id: requirement,
                    kind,
                    requirement_version: stamp,
                    proposed_by_step_id: None,
                    created_at: now,
                    updated_at: now,
                    deleted_at: None,
                });
                self.item_requirements.len() - 1
            }
        };
        let row = &mut self.item_requirements[at];
        row.requirement_version = stamp;
        row.deleted_at = None;
        row.updated_at = now;
        row
    }

    /// The live citation of a triple, or the `NotFound` naming it that `uncite` and `reconfirm`
    /// answer (plan D10).
    fn live_citation_mut(
        &mut self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<&mut ItemRequirement> {
        self.item_requirements
            .iter_mut()
            .find(|row| {
                row.item_id == item
                    && row.requirement_id == requirement
                    && row.kind == kind
                    && row.deleted_at.is_none()
            })
            .ok_or_else(|| StoreError::NotFound {
                entity: "item_requirement",
                id: citation_key(item, requirement, kind),
            })
    }

    /// Plan D10's upsert, stamped at the requirement's current version. Refusals run item,
    /// requirement, withdrawn, step.
    fn cite(
        &mut self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        proposed_by: Option<StepId>,
        now: DateTime<Utc>,
    ) -> Result<ItemRequirement> {
        self.require_item(item)?;
        let head = self
            .requirements
            .get(&requirement)
            .ok_or_else(|| StoreError::NotFound {
                entity: "requirement",
                id: requirement.to_string(),
            })?;
        if head.state == RequirementState::Withdrawn
            && matches!(kind, CitationKind::Addresses | CitationKind::Reserves)
        {
            return Err(StoreError::Constraint(withdrawn_requirement_cited(
                &head.key, kind,
            )));
        }
        if let Some(step) = proposed_by
            && !self.steps.contains_key(&step)
        {
            return Err(StoreError::Constraint(references_no_row(
                "item_requirement.proposed_by_step_id",
                step,
                "run_step",
            )));
        }
        let stamp = head.version;
        let row = self.upsert_citation(item, requirement, kind, stamp, now);
        row.proposed_by_step_id = proposed_by;
        Ok(row.clone())
    }

    /// Tombstones a live citation.
    fn uncite(
        &mut self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self.live_citation_mut(item, requirement, kind)?;
        row.deleted_at = Some(now);
        row.updated_at = now;
        Ok(())
    }

    /// MOD-11 D13, B-6: the upsert. Refusals run self link, step, fence, own item, `to`, its
    /// project; then a new row is live and the step's, a tombstone revives as the step's, and a
    /// live row keeps its proposer (only `updated_at` moves, as Postgres's trigger moves it).
    fn propose_link(
        &mut self,
        fence: StepFence,
        link: ProposeLink,
        now: DateTime<Utc>,
    ) -> Result<ItemLink> {
        if link.from == link.to {
            return Err(StoreError::Constraint(self_link(link.from)));
        }
        let (_, item, project) = self.step_scope(link.step, fence)?;
        if item != Some(link.from) {
            return Err(StoreError::Constraint(step_writes_own_item(
                link.step, link.from,
            )));
        }
        if self.require_item(link.to)?.project_id != project {
            return Err(StoreError::Constraint(link_outside_project(link.to)));
        }
        let at = self.links.iter().position(|row| {
            row.from_item_id == link.from && row.to_item_id == link.to && row.kind == link.kind
        });
        let Some(at) = at else {
            let row = ItemLink {
                from_item_id: link.from,
                to_item_id: link.to,
                kind: link.kind,
                proposed_by_step_id: Some(link.step),
                created_at: now,
                updated_at: now,
                deleted_at: None,
            };
            self.links.push(row.clone());
            return Ok(row);
        };
        let row = &mut self.links[at];
        if row.deleted_at.is_some() {
            row.proposed_by_step_id = Some(link.step);
            row.deleted_at = None;
        }
        row.updated_at = now;
        Ok(row.clone())
    }

    /// MOD-11 D13, B-5: tombstones the live `(from, to, kind)` when a step of `link.step`'s run
    /// proposed it. Refusals run step, fence, own item, then `NotFound` for no live row and
    /// [`link_not_proposed_by_run`] for a live row of anyone else's.
    fn withdraw_link(
        &mut self,
        fence: StepFence,
        link: WithdrawLink,
        now: DateTime<Utc>,
    ) -> Result<ItemLink> {
        let (run, item, _) = self.step_scope(link.step, fence)?;
        if item != Some(link.from) {
            return Err(StoreError::Constraint(step_writes_own_item(
                link.step, link.from,
            )));
        }
        let key = link_key(link.from, link.to, link.kind);
        let steps = &self.steps;
        let Some(row) = self.links.iter_mut().find(|row| {
            row.from_item_id == link.from
                && row.to_item_id == link.to
                && row.kind == link.kind
                && row.deleted_at.is_none()
        }) else {
            return Err(StoreError::NotFound {
                entity: "item_link",
                id: key,
            });
        };
        let ours = row
            .proposed_by_step_id
            .and_then(|proposer| steps.get(&proposer))
            .is_some_and(|proposer| proposer.run_id == run);
        if !ours {
            return Err(StoreError::Constraint(link_not_proposed_by_run(&key)));
        }
        row.deleted_at = Some(now);
        row.updated_at = now;
        Ok(row.clone())
    }

    /// MOD-11 B-4: the item of `project` keyed `key`.
    fn item_by_key(&self, project: ProjectId, key: &str) -> Option<ItemId> {
        if key.contains('\0') {
            return None;
        }
        self.items
            .values()
            .find(|row| row.project_id == project && row.key == key)
            .map(|row| row.id)
    }

    /// Re-stamps a live citation at the requirement's current version, clearing suspect.
    /// Refusals run citation, then withdrawn: an `addresses` / `reserves` citation of a withdrawn
    /// requirement is not re-stamped, as `cite` would not stamp it (plan D10).
    fn reconfirm(
        &mut self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        now: DateTime<Utc>,
    ) -> Result<ItemRequirement> {
        self.live_citation_mut(item, requirement, kind)?;
        // A live citation's requirement exists (`ON DELETE CASCADE`), so a missing one can only
        // mean a missing citation, which the lookup above reports.
        let head = self.requirements.get(&requirement);
        if let Some(head) = head
            && head.state == RequirementState::Withdrawn
            && matches!(kind, CitationKind::Addresses | CitationKind::Reserves)
        {
            return Err(StoreError::Constraint(withdrawn_requirement_cited(
                &head.key, kind,
            )));
        }
        let stamp = head.map(|row| row.version);
        let row = self.live_citation_mut(item, requirement, kind)?;
        if let Some(stamp) = stamp {
            row.requirement_version = stamp;
        }
        row.updated_at = now;
        Ok(row.clone())
    }
}

/// The rows a project delete takes, identified once by [`State::project_reach`] so the report and
/// the act cannot disagree about which they were (D4).
#[derive(Debug)]
struct ProjectReach {
    /// `item` ids of the project.
    items: HashSet<ItemId>,
    /// `run` ids of the project, plus any run of one of its items.
    runs: HashSet<RunId>,
    /// `run_step` ids below those runs.
    steps: HashSet<StepId>,
    /// `step_graph` ids of the project.
    graphs: HashSet<StepGraphId>,
    /// `step_graph_phase` ids below those graphs.
    phases: HashSet<PhaseId>,
    /// `repo` ids of the project.
    repos: HashSet<RepoId>,
    /// `requirement_area` ids of the project (MOD-38).
    requirement_areas: HashSet<RequirementAreaId>,
    /// `requirement` ids of the project.
    requirements: HashSet<RequirementId>,
}

/// A row count as the `u64` [`DeleteReach`] holds, saturating rather than casting: `usize` is
/// never wider than `u64` on a target this ships to, and the `try_from` says so without an `as`.
fn rows(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// The stored probe's verdict when the per-box switch goes back on (MOD-23 D242, blueprint D249):
/// Postgres's `probe IS NULL OR COALESCE(probe->>'status' = 'ready', false)`, written out. No
/// document is `true`; a document is ready only when its `status` is the string `ready`, so a
/// JSON `null` and a document without a `status` are both `false`, as the `COALESCE` makes them.
fn probe_says_ready(probe: Option<&Value>) -> bool {
    probe.is_none_or(|doc| doc.get("status").and_then(Value::as_str) == Some("ready"))
}

/// The refusal both writers of `project.settings` give a blob that is not a JSON object (D7).
///
/// One sentence rather than two: `set_setting` cannot merge a key into a scalar and `clear_setting`
/// cannot remove one from it, and what stops both is the same fact — the document is not a
/// document. A wording per verb would be two sentences about one blob, which is the drift the text
/// helpers in [`store::traits`](crate::store::traits) exist to prevent (review L4).
///
/// Private, because `PgStore` cannot reach it: `project.settings` is `JSONB NOT NULL DEFAULT '{}'`
/// there and the merge is Postgres's own `||`.
fn settings_not_an_object(id: ProjectId, key: impl core::fmt::Display) -> String {
    format!("project.settings of `{id}` is not a JSON object, so `{key}` cannot be merged into it")
}

/// A revision author must name a real `app_user` row (§5.5 `REFERENCES app_user(id)`); the nil
/// UUID is what `UserId::default()` yields, so it is rejected here rather than written and later
/// refused by MOD-6's `PgStore`. An author that is non-nil but unknown is out of scope: the
/// default [`MemStore::new`] holds no users at all (plan D7).
fn require_author(id: UserId, column: &str) -> Result<()> {
    if id.as_uuid().is_nil() {
        return Err(StoreError::Constraint(format!(
            "{column} must reference an app_user; the nil UUID does not"
        )));
    }
    Ok(())
}

/// MOD-42 (plan D1-D5, D12-D14; blueprint §2.11): the relay's reference semantics. Every time is
/// the handle's clock, passed in as `now` (I-4); every status move is a compare-and-set (I-3).
impl State {
    /// The run's lease owner while its lease is live by `now`: Postgres's
    /// `r.lease_owner = p.owner AND r.lease_expires_at > clock_timestamp()`.
    fn live_owner(&self, run: RunId, now: DateTime<Utc>) -> Option<Uuid> {
        let expiry = self.runs.get(&run)?.lease_expires_at?;
        if expiry > now {
            self.lease_owners.get(&run).copied()
        } else {
            None
        }
    }

    /// `answered_by`/`issued_by` and `answered_box`/`issued_box` must name rows (blueprint B-7).
    fn require_actor(&self, user: UserId, box_id: BoxId, table: &str, by: &str) -> Result<()> {
        self.require_user(user, &format!("{table}.{by}_by"))?;
        if !self.boxes.contains_key(&box_id) {
            return Err(StoreError::Constraint(references_no_row(
                &format!("{table}.{by}_box"),
                box_id,
                "box",
            )));
        }
        Ok(())
    }

    fn open_permission(
        &mut self,
        open: OpenPermission,
        now: DateTime<Utc>,
    ) -> Result<PermissionId> {
        let step = self
            .steps
            .get(&open.run_step_id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: open.run_step_id.to_string(),
            })?;
        if step.run_id != open.run_id {
            return Err(StoreError::Constraint(format!(
                "step_permission.run_step_id `{}` is not a step of run `{}`",
                open.run_step_id, open.run_id
            )));
        }
        if self.lease_owners.get(&open.run_id) != Some(&open.owner) {
            return Err(StoreError::Fenced {
                step: open.run_step_id,
            });
        }
        if self.permissions.contains_key(&open.id) {
            return Err(StoreError::Constraint(already_exists(
                "step_permission",
                open.id,
            )));
        }
        if self
            .permissions
            .values()
            .any(|p| p.row.session == open.session && p.row.request_id == open.request_id)
        {
            return Err(StoreError::Constraint(format!(
                "step_permission (session, request_id) `({}, {})` already exists",
                open.session, open.request_id
            )));
        }
        // D5: an older session's open rows on the same step go stale.
        for p in self.permissions.values_mut() {
            if p.row.run_step_id == open.run_step_id
                && p.row.session != open.session
                && matches!(
                    p.row.status,
                    PermissionStatus::Pending | PermissionStatus::Answered
                )
            {
                p.row.status = PermissionStatus::Stale;
                p.row.resolved_at = Some(now);
            }
        }
        let id = open.id;
        self.permissions.insert(
            id,
            PermissionRow {
                row: StepPermission {
                    id,
                    run_id: open.run_id,
                    run_step_id: open.run_step_id,
                    session: open.session,
                    request_id: open.request_id,
                    tool_call_id: open.tool_call_id,
                    summary: open.summary,
                    options: open.options,
                    status: PermissionStatus::Pending,
                    option_id: None,
                    answered_by: None,
                    answered_box: None,
                    created_at: now,
                    answered_at: None,
                    resolved_at: None,
                },
                owner: open.owner,
            },
        );
        Ok(id)
    }

    fn apply_permission(
        &mut self,
        id: PermissionId,
        owner: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<PermissionChoice>> {
        let leased = |run: RunId| self.lease_owners.get(&run) == Some(&owner);
        let Some(p) = self.permissions.get(&id) else {
            return Err(StoreError::NotFound {
                entity: "step_permission",
                id: id.to_string(),
            });
        };
        if p.row.status != PermissionStatus::Answered || p.owner != owner || !leased(p.row.run_id) {
            return Ok(None);
        }
        let Some(p) = self.permissions.get_mut(&id) else {
            return Ok(None);
        };
        p.row.status = PermissionStatus::Applied;
        p.row.resolved_at = Some(now);
        Ok(p.row
            .option_id
            .clone()
            .map(|option_id| PermissionChoice { option_id }))
    }

    fn settle_permissions(
        &mut self,
        session: RelaySessionId,
        to: PermissionStatus,
        now: DateTime<Utc>,
    ) -> Result<u64> {
        if !matches!(to, PermissionStatus::Cancelled | PermissionStatus::Stale) {
            return Err(StoreError::Constraint(format!(
                "step_permission rows settle to `cancelled` or `stale`, not `{to}`"
            )));
        }
        let mut moved = 0;
        for p in self.permissions.values_mut() {
            if p.row.session == session
                && matches!(
                    p.row.status,
                    PermissionStatus::Pending | PermissionStatus::Answered
                )
            {
                p.row.status = to;
                p.row.resolved_at = Some(now);
                moved += 1;
            }
        }
        Ok(moved)
    }

    fn answer_permission(
        &mut self,
        id: PermissionId,
        option_id: &str,
        user: UserId,
        box_id: BoxId,
        now: DateTime<Utc>,
    ) -> Result<AnswerOutcome> {
        let Some(p) = self.permissions.get(&id) else {
            return Err(StoreError::NotFound {
                entity: "step_permission",
                id: id.to_string(),
            });
        };
        let (run, owner, status) = (p.row.run_id, p.owner, p.row.status);
        let offered = p.row.options.iter().any(|option| option.id == option_id);
        self.require_actor(user, box_id, "step_permission", "answered")?;
        let refusal = match status {
            PermissionStatus::Pending => None,
            PermissionStatus::Answered => Some(AnswerRefusal::Answered),
            PermissionStatus::Applied => Some(AnswerRefusal::Applied),
            PermissionStatus::Cancelled => Some(AnswerRefusal::Cancelled),
            PermissionStatus::Stale => Some(AnswerRefusal::Stale),
        }
        .or_else(|| (!offered).then_some(AnswerRefusal::NotOffered))
        .or_else(|| {
            (self.live_owner(run, now) != Some(owner)).then_some(AnswerRefusal::ExecutorGone)
        });
        if let Some(refusal) = refusal {
            return Ok(AnswerOutcome::Refused(refusal));
        }
        if let Some(p) = self.permissions.get_mut(&id) {
            p.row.status = PermissionStatus::Answered;
            p.row.option_id = Some(option_id.to_owned());
            p.row.answered_by = Some(user);
            p.row.answered_box = Some(box_id);
            p.row.answered_at = Some(now);
        }
        Ok(AnswerOutcome::Answered)
    }

    fn relay_view(&self, item: ItemId, now: DateTime<Utc>) -> RelayView {
        let of_item = |run: RunId| self.runs.get(&run).filter(|row| row.item_id == Some(item));
        let mut permissions: Vec<StepPermission> = self
            .permissions
            .values()
            .filter(|p| {
                p.row.status == PermissionStatus::Pending
                    && of_item(p.row.run_id).is_some()
                    && self.live_owner(p.row.run_id, now) == Some(p.owner)
            })
            .map(|p| p.row.clone())
            .collect();
        permissions.sort_unstable_by_key(|row| (row.created_at, row.id));
        let cancels: BTreeSet<RunId> = self
            .run_commands
            .values()
            .filter(|c| {
                c.kind == RunCommandKind::Cancel
                    && c.status == RunCommandStatus::Pending
                    && of_item(c.run_id).is_some_and(|run| !run.status.is_terminal())
            })
            .map(|c| c.run_id)
            .collect();
        RelayView {
            permissions,
            cancels: cancels.into_iter().collect(),
            follow_ups: self.follow_up_views(item),
        }
    }

    /// MOD-69 plan D4: `relay_view`'s predicate over the scope's item runs, joined to the item and
    /// the step.
    fn open_permissions(&self, scope: &Scope, now: DateTime<Utc>) -> Vec<WaitingPermission> {
        let mut rows: Vec<WaitingPermission> = self
            .permissions
            .values()
            .filter(|p| {
                p.row.status == PermissionStatus::Pending
                    && self.live_owner(p.row.run_id, now) == Some(p.owner)
            })
            .filter_map(|p| {
                let run = self.runs.get(&p.row.run_id)?;
                let item = self.items.get(&run.item_id?)?;
                if !scope.contains(item.project_id) {
                    return None;
                }
                let step = self.steps.get(&p.row.run_step_id)?;
                // Review L1: the Runs pane's rule, a sibling at the slot with a non-zero index.
                let step_fanned = self.steps.values().any(|sibling| {
                    sibling.run_id == step.run_id
                        && sibling.position == step.position
                        && sibling.attempt == step.attempt
                        && sibling.fanout_index != 0
                });
                Some(WaitingPermission {
                    item: item.id,
                    project: item.project_id,
                    item_key: item.key.clone(),
                    key_prefix: item.key_prefix.clone(),
                    key_number: item.key_number,
                    run_queued_at: run.queued_at,
                    step_position: step.position,
                    step_attempt: step.attempt,
                    step_fanout_index: step.fanout_index,
                    step_fanned,
                    phase_name: step.phase_name.clone(),
                    permission: p.row.clone(),
                })
            })
            .collect();
        WaitingPermission::sort_canonical(&mut rows);
        rows
    }

    fn request_cancel(
        &mut self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
        now: DateTime<Utc>,
    ) -> Result<CancelRequest> {
        self.require_run(run)?;
        self.require_actor(user, box_id, "run_command", "issued")?;
        // MOD-70 D5, B-14: both answers refuse the run's pending follow-ups, idempotently.
        if let Some(pending) = self
            .run_commands
            .values()
            .find(|c| {
                c.run_id == run
                    && c.kind == RunCommandKind::Cancel
                    && c.status == RunCommandStatus::Pending
            })
            .map(|c| c.id)
        {
            self.refuse_follow_ups(|row, _| row.run_id == run, FOLLOW_UP_RUN_CANCELLED, now);
            return Ok(CancelRequest::AlreadyPending(pending));
        }
        self.refuse_follow_ups(|row, _| row.run_id == run, FOLLOW_UP_RUN_CANCELLED, now);
        let id = RunCommandId::new();
        self.run_commands.insert(
            id,
            RunCommand {
                id,
                run_id: run,
                kind: RunCommandKind::Cancel,
                issued_by: user,
                issued_box: box_id,
                status: RunCommandStatus::Pending,
                resolution: None,
                issued_at: now,
                resolved_at: None,
            },
        );
        Ok(CancelRequest::Inserted(id))
    }

    fn pending_commands(&self, owner: Uuid, box_id: BoxId, now: DateTime<Utc>) -> Vec<RunCommand> {
        let applies = |run: RunId| {
            if self.lease_owners.get(&run) == Some(&owner) {
                return true;
            }
            let Some(row) = self.runs.get(&run) else {
                return false;
            };
            row.executing_box_id == Some(box_id)
                && (row.status.is_terminal()
                    || (matches!(row.status, RunStatus::Running | RunStatus::AwaitingApproval)
                        && (!self.lease_owners.contains_key(&run)
                            || row.lease_expires_at.is_none_or(|expiry| expiry <= now))))
        };
        let mut rows: Vec<RunCommand> = self
            .run_commands
            .values()
            // MOD-70 D5, I-9: cancels only; a follow-up is the walk's own, never a command.
            .filter(|c| {
                c.kind == RunCommandKind::Cancel
                    && c.status == RunCommandStatus::Pending
                    && applies(c.run_id)
            })
            .cloned()
            .collect();
        rows.sort_unstable_by_key(|row| (row.issued_at, row.id));
        rows
    }

    fn resolve_command(
        &mut self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        if to == RunCommandStatus::Pending {
            return Err(StoreError::Constraint(format!(
                "run_command `{id}` resolves to `applied` or `refused`, not `pending`"
            )));
        }
        let Some(row) = self.run_commands.get_mut(&id) else {
            return Err(StoreError::NotFound {
                entity: "run_command",
                id: id.to_string(),
            });
        };
        if row.status != RunCommandStatus::Pending {
            return Ok(false);
        }
        row.status = to;
        row.resolution = resolution;
        row.resolved_at = Some(now);
        // MOD-70 D5: a resolved row never keeps a follow-up's text.
        if let Some(payload) = self.follow_up_payloads.get_mut(&id) {
            payload.text = None;
        }
        Ok(true)
    }
}

/// MOD-70 (plan D1-D5, D9; blueprint §2.8): follow-ups for engine steps. Every time is the
/// handle's clock (I-4); every method is one closure under the write lock, so the Postgres races
/// collapse to "first one in".
impl State {
    /// Every `pending` follow-up `pick` selects moves to `refused` with `reason`, its text dropped
    /// (I-5). Answers how many moved.
    fn refuse_follow_ups(
        &mut self,
        pick: impl Fn(&RunCommand, &FollowUpPayload) -> bool,
        reason: &str,
        now: DateTime<Utc>,
    ) -> u64 {
        let mut moved = 0;
        for (id, row) in &mut self.run_commands {
            if row.kind != RunCommandKind::FollowUp || row.status != RunCommandStatus::Pending {
                continue;
            }
            let Some(payload) = self.follow_up_payloads.get_mut(id) else {
                continue;
            };
            if !pick(row, payload) {
                continue;
            }
            row.status = RunCommandStatus::Refused;
            row.resolution = Some(reason.to_owned());
            row.resolved_at = Some(now);
            payload.text = None;
            moved += 1;
        }
        moved
    }

    /// The step's `pending` follow-up, if any (D1: at most one).
    fn pending_follow_up(&self, step: StepId) -> Option<RunCommandId> {
        self.run_commands
            .values()
            .find(|c| {
                c.kind == RunCommandKind::FollowUp
                    && c.status == RunCommandStatus::Pending
                    && self
                        .follow_up_payloads
                        .get(&c.id)
                        .is_some_and(|p| p.run_step_id == step)
            })
            .map(|c| c.id)
    }

    fn request_follow_up(
        &mut self,
        new: NewFollowUp,
        now: DateTime<Utc>,
    ) -> Result<FollowUpRequest> {
        let step = self
            .steps
            .get(&new.run_step_id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "run_step",
                id: new.run_step_id.to_string(),
            })?;
        let (run, fanout_index, status) = (step.run_id, step.fanout_index, step.status);
        self.require_actor(new.issued_by, new.issued_box, "run_command", "issued")?;
        let refusal = if self
            .runs
            .get(&run)
            .is_some_and(|row| row.kind == RunKind::Chat)
        {
            Some(FollowUpRefusal::ChatRun)
        } else if fanout_index < 0 {
            Some(FollowUpRefusal::Judge)
        } else if status != StepStatus::Running {
            Some(FollowUpRefusal::NotRunning)
        } else if self.run_commands.values().any(|c| {
            c.run_id == run
                && c.kind == RunCommandKind::Cancel
                && c.status == RunCommandStatus::Pending
        }) {
            Some(FollowUpRefusal::Cancelling)
        } else if self.pending_follow_up(new.run_step_id).is_some() {
            Some(FollowUpRefusal::AlreadyQueued)
        } else {
            let window = self.follow_up_windows.get(&new.run_step_id);
            let live = self.live_owner(run, now);
            // B-4: the lease must be live under the window's owner; with no window, under anyone.
            let gone = match window {
                Some(w) => live != Some(w.owner),
                None => live.is_none(),
            };
            if gone {
                Some(FollowUpRefusal::ExecutorGone)
            } else {
                match window {
                    None => Some(FollowUpRefusal::NotStarted),
                    Some(w) if w.closed_at.is_some() => Some(FollowUpRefusal::SessionEnded),
                    Some(_) => None,
                }
            }
        };
        if let Some(refusal) = refusal {
            return Ok(FollowUpRequest::Refused(refusal));
        }
        if self.run_commands.contains_key(&new.id) {
            return Err(StoreError::Constraint(already_exists(
                "run_command",
                new.id,
            )));
        }
        self.run_commands.insert(
            new.id,
            RunCommand {
                id: new.id,
                run_id: run,
                kind: RunCommandKind::FollowUp,
                issued_by: new.issued_by,
                issued_box: new.issued_box,
                status: RunCommandStatus::Pending,
                resolution: None,
                issued_at: now,
                resolved_at: None,
            },
        );
        self.follow_up_payloads.insert(
            new.id,
            FollowUpPayload {
                run_step_id: new.run_step_id,
                text: Some(new.text.into_string()),
            },
        );
        Ok(FollowUpRequest::Queued(new.id))
    }

    fn open_follow_ups(
        &mut self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let row = self.steps.get(&step).ok_or_else(|| StoreError::NotFound {
            entity: "run_step",
            id: step.to_string(),
        })?;
        if row.run_id != run {
            return Err(StoreError::Constraint(format!(
                "follow_up_window.run_step_id `{step}` is not a step of run `{run}`"
            )));
        }
        // B-3: the owner alone, as every other executor-side write.
        if self.lease_owners.get(&run) != Some(&owner) {
            return Ok(false);
        }
        self.follow_up_windows.insert(
            step,
            FollowUpWindow {
                run_step_id: step,
                run_id: run,
                session,
                owner,
                opened_at: now,
                closed_at: None,
            },
        );
        self.refuse_follow_ups(
            |_, payload| payload.run_step_id == step,
            FOLLOW_UP_SESSION_ENDED,
            now,
        );
        Ok(true)
    }

    fn next_follow_up(&self, step: StepId, session: RelaySessionId) -> Option<QueuedFollowUp> {
        let window = self.follow_up_windows.get(&step)?;
        if window.session != session || window.closed_at.is_some() {
            return None;
        }
        let id = self.pending_follow_up(step)?;
        let text = self.follow_up_payloads.get(&id)?.text.clone()?;
        Some(QueuedFollowUp { id, text })
    }

    fn settle_follow_up(
        &mut self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
        now: DateTime<Utc>,
    ) -> Result<SettleOutcome> {
        // B-19: unknown, then not a follow-up, then not pending, then fenced.
        let Some(row) = self.run_commands.get(&id) else {
            return Err(StoreError::NotFound {
                entity: "run_command",
                id: id.to_string(),
            });
        };
        if row.kind != RunCommandKind::FollowUp {
            return Err(StoreError::Constraint(format!(
                "run_command `{id}` is a `{}`, not a `follow_up`",
                row.kind
            )));
        }
        if row.status != RunCommandStatus::Pending {
            return Ok(SettleOutcome::NotPending);
        }
        // B-3: the owner alone.
        if self.lease_owners.get(&row.run_id) != Some(&owner) {
            return Ok(SettleOutcome::Fenced);
        }
        let (status, resolution) = match to {
            FollowUpSettle::Applied => (RunCommandStatus::Applied, None),
            FollowUpSettle::Refused(sentence) => (RunCommandStatus::Refused, Some(sentence)),
        };
        if let Some(row) = self.run_commands.get_mut(&id) {
            row.status = status;
            row.resolution = resolution;
            row.resolved_at = Some(now);
        }
        if let Some(payload) = self.follow_up_payloads.get_mut(&id) {
            payload.text = None;
        }
        Ok(SettleOutcome::Settled)
    }

    fn close_follow_ups(
        &mut self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> u64 {
        // B-5: only while the step's window is this session's.
        let Some(window) = self.follow_up_windows.get_mut(&step) else {
            return 0;
        };
        if window.session != session {
            return 0;
        }
        window.closed_at = window.closed_at.or(Some(now));
        self.refuse_follow_ups(|_, payload| payload.run_step_id == step, reason, now)
    }

    fn close_dropped_follow_ups(
        &mut self,
        run: RunId,
        owner: Uuid,
        reason: &str,
        now: DateTime<Utc>,
    ) -> u64 {
        // B-3, F-22: the owner alone; otherwise nothing is written.
        if self.lease_owners.get(&run) != Some(&owner) {
            return 0;
        }
        for window in self.follow_up_windows.values_mut() {
            if window.run_id == run && window.closed_at.is_none() {
                window.closed_at = Some(now);
            }
        }
        self.refuse_follow_ups(|row, _| row.run_id == run, reason, now)
    }

    /// D5, B-13: the newest follow-up of each step of the item's non-terminal runs (the pending
    /// one if any, else the greatest `(issued_at, id)`), in `(issued_at, id)` order. No text.
    fn follow_up_views(&self, item: ItemId) -> Vec<FollowUpView> {
        let mut newest: BTreeMap<StepId, &RunCommand> = BTreeMap::new();
        for row in self.run_commands.values() {
            if row.kind != RunCommandKind::FollowUp {
                continue;
            }
            let live = self
                .runs
                .get(&row.run_id)
                .is_some_and(|run| run.item_id == Some(item) && !run.status.is_terminal());
            let Some(payload) = self.follow_up_payloads.get(&row.id) else {
                continue;
            };
            if !live {
                continue;
            }
            let key = |c: &RunCommand| (c.status == RunCommandStatus::Pending, c.issued_at, c.id);
            newest
                .entry(payload.run_step_id)
                .and_modify(|held| {
                    if key(row) > key(held) {
                        *held = row;
                    }
                })
                .or_insert(row);
        }
        let mut views: Vec<FollowUpView> = newest
            .into_iter()
            .map(|(step, row)| FollowUpView {
                id: row.id,
                run_id: row.run_id,
                run_step_id: step,
                status: row.status,
                resolution: row.resolution.clone(),
                issued_at: row.issued_at,
                resolved_at: row.resolved_at,
            })
            .collect();
        views.sort_unstable_by_key(|row| (row.issued_at, row.id));
        views
    }
}

impl ReadStore for MemStore {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        Ok(self.read(|state| state.item_summaries(scope, filter)))
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        Ok(self.read(|state| state.items.get(&id).cloned()))
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        self.read(|state| state.link_graph(id, hops))
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        Ok(self.read(|state| state.document_heads(id)))
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        Ok(self.read(|state| state.item_notes(id)))
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        Ok(self.read(|state| state.run_summaries(id)))
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        Ok(self.read(|state| state.step_log(step)))
    }

    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        Ok(self.read(|state| {
            state
                .documents
                .iter()
                .find(|document| document.id == id)
                .cloned()
        }))
    }

    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        Ok(self.read(|state| state.documents_of_kinds(item, kinds)))
    }

    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        Ok(self.read(|state| state.upstream(id, hops, scope)))
    }

    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        Ok(self.read(|state| state.projects.get(&id).cloned()))
    }

    // MOD-4 milestone 1: the five run reads of ANA-2 §8 that a mirrored table makes trait methods
    // (plan D1). All five are total; only the writers refuse.

    async fn run(&self, id: RunId) -> Result<Option<Run>> {
        Ok(self.read(|state| state.runs.get(&id).cloned()))
    }

    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>> {
        Ok(self.read(|state| state.run_step_rows(run)))
    }

    async fn step_trees(&self, step: StepId) -> Result<Vec<RunStepTree>> {
        Ok(self.read(|state| state.step_tree_rows(step)))
    }

    async fn step_commits(&self, step: StepId) -> Result<Vec<RunStepCommit>> {
        Ok(self.read(|state| state.step_commit_rows(step)))
    }

    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> Result<Vec<ResolvedInput>> {
        Ok(self.read(|state| state.resolve_inputs(item, run, kinds)))
    }

    // ---- ANA-11 §5.1 (MOD-38): requirements. Total, as the run reads above are ----

    async fn requirement_spec(&self, project: ProjectId) -> Result<Option<RequirementSpec>> {
        Ok(self.read(|state| state.requirement_specs.get(&project).cloned()))
    }

    async fn requirement_areas(&self, project: ProjectId) -> Result<Vec<RequirementArea>> {
        Ok(self.read(|state| state.requirement_area_rows(project)))
    }

    async fn requirements(
        &self,
        project: ProjectId,
        filter: &RequirementFilter,
    ) -> Result<Vec<Requirement>> {
        Ok(self.read(|state| state.requirement_rows(project, filter)))
    }

    async fn requirement(&self, id: RequirementId) -> Result<Option<Requirement>> {
        Ok(self.read(|state| state.requirements.get(&id).cloned()))
    }

    async fn requirement_revisions(
        &self,
        id: RequirementId,
    ) -> Result<Option<Vec<RequirementRevision>>> {
        Ok(Some(self.read(|state| state.requirement_revision_rows(id))))
    }

    async fn item_requirements(&self, item: ItemId) -> Result<Vec<ItemCitation>> {
        Ok(self.read(|state| state.item_citations(item)))
    }

    async fn requirement_coverage(&self, requirement: RequirementId) -> Result<Vec<CoverageRow>> {
        Ok(self.read(|state| state.coverage_rows(requirement)))
    }

    async fn tool_call_counts(&self, item: ItemId) -> Result<Vec<ToolCallCount>> {
        Ok(self.read(|state| state.tool_call_counts(item)))
    }

    async fn waiting_candidates(&self, scope: &Scope) -> Result<Vec<WaitingCandidate>> {
        Ok(self.read(|state| state.waiting_candidates(scope)))
    }
}

impl WriteStore for MemStore {
    async fn mint_item(&self, new: NewItem) -> Result<Item> {
        let now = self.now();
        self.write(|state| state.mint(new, now))
    }

    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        let now = self.now();
        self.write(|state| state.update(id, expected_version, patch, now))
    }

    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::ItemTransition)?;
        let now = self.now();
        self.write(|state| state.transition(id, from, to, now))
    }

    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> Result<usize> {
        self.write(|state| state.append_events(fence, events))
    }

    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.set_step_usage(fence, step, usage, prompt_digest, now))
    }

    async fn upsert_agent(
        &self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Agent>> {
        let now = self.now();
        self.write(|state| state.upsert_agent(agent, expected, now))
    }

    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()> {
        let now = self.now();
        self.write(|state| state.upsert_agent_box(row, now))
    }

    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.set_agent_box_quota(agent_id, box_id, quota, quota_at, now))
    }

    async fn set_agent_box_enabled(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.set_agent_box_enabled(agent_id, box_id, enabled, now))
    }

    async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()> {
        let now = self.now();
        self.write(|state| state.record_box_probe(probe, now))
    }

    async fn boxes(&self) -> Result<Vec<BoxRecord>> {
        let user = self.this_user();
        Ok(self.read(|state| state.box_records(user)))
    }

    async fn edit_box(
        &self,
        id: BoxId,
        expected: i32,
        edit: BoxEdit,
    ) -> Result<CasOutcome<BoxRow>> {
        // Both before the write lock: `this_user` takes the read lock (blueprint F-K).
        let user = self.this_user();
        let now = self.now();
        self.write(|state| state.edit_box(user, id, expected, edit, now))
    }

    async fn box_probe_spec(&self) -> Result<Option<StoredSetting>> {
        Ok(self.read(State::box_probe_spec))
    }

    async fn set_box_probe_spec(
        &self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredSetting>>> {
        let now = self.now();
        self.write(|state| state.set_box_probe_spec(overlay, expected, now))
    }

    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()> {
        self.write(|state| state.start_chat_run(chat))
    }

    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.finish_chat_run(run, step, status, finished_at, now))
    }

    async fn set_step_prompt(
        &self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.set_step_prompt(fence, step, digest, trim, now))
    }

    // MOD-15 milestone 1. Each arm takes the lock and hands one `State` method the store's clock;
    // every rule that can refuse is in `impl State`, so nothing here can disagree with `PgStore`
    // about what is legal, only about how it is stored.

    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace> {
        let now = self.now();
        self.write(|state| state.create_workspace(new, now))
    }

    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> Result<CasOutcome<Workspace>> {
        let now = self.now();
        self.write(|state| state.update_workspace(id, expected, patch, now))
    }

    async fn workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        Ok(self.read(|state| state.workspaces.get(&id).cloned()))
    }

    async fn upsert_workspace_project(&self, link: &WorkspaceProject) -> Result<()> {
        self.write(|state| state.upsert_workspace_project(link))
    }

    async fn remove_workspace_project(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> Result<()> {
        self.write(|state| state.remove_workspace_project(workspace, project))
    }

    async fn workspace_projects(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceProject>> {
        Ok(self.read(|state| state.workspace_project_rows(workspace)))
    }

    async fn upsert_workspace_box_path(&self, path: &WorkspaceBoxPath) -> Result<()> {
        let now = self.now();
        self.write(|state| state.upsert_workspace_box_path(path, now))
    }

    async fn workspace_box_paths(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceBoxPath>> {
        Ok(self.read(|state| state.workspace_box_path_rows(workspace)))
    }

    async fn create_project(&self, new: NewProject) -> Result<Project> {
        let now = self.now();
        self.write(|state| state.create_project(new, now))
    }

    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> Result<CasOutcome<Project>> {
        let now = self.now();
        self.write(|state| state.update_project(id, expected, patch, now))
    }

    async fn create_repo(&self, new: NewRepo) -> Result<Repo> {
        let now = self.now();
        self.write(|state| state.create_repo(new, now))
    }

    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> Result<CasOutcome<Repo>> {
        let now = self.now();
        self.write(|state| state.update_repo(id, expected, patch, now))
    }

    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>> {
        Ok(self.read(|state| state.repo_rows(project)))
    }

    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> Result<()> {
        let now = self.now();
        self.write(|state| state.upsert_repo_box_path(path, now))
    }

    async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.infer_repo_box_path(path, now))
    }

    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        Ok(self.read(|state| state.repo_box_path_rows(repo)))
    }

    async fn create_item_kind(&self, new: NewItemKind) -> Result<ItemKind> {
        let now = self.now();
        self.write(|state| state.create_item_kind(new, now))
    }

    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> Result<CasOutcome<ItemKind>> {
        let now = self.now();
        self.write(|state| state.update_item_kind(id, expected, patch, now))
    }

    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
        Ok(self.read(|state| state.item_kind_rows(project)))
    }

    async fn delete_item_kind(&self, id: ItemKindId) -> Result<()> {
        self.write(|state| state.delete_item_kind(id))
    }

    async fn create_step_graph(&self, new: NewStepGraph) -> Result<StepGraph> {
        let now = self.now();
        self.write(|state| state.create_step_graph(new, now))
    }

    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> Result<CasOutcome<StepGraph>> {
        let now = self.now();
        self.write(|state| state.update_step_graph(id, expected, patch, now))
    }

    async fn step_graphs(&self, project: ProjectId) -> Result<Vec<StepGraph>> {
        Ok(self.read(|state| state.step_graph_rows(project)))
    }

    async fn create_phase(&self, phase: &StepGraphPhase) -> Result<StepGraphPhase> {
        let now = self.now();
        self.write(|state| state.create_phase(phase, now))
    }

    async fn create_phase_agents(&self, phase: PhaseId, agents: &[PhaseAgent]) -> Result<()> {
        self.write(|state| state.create_phase_agents(phase, agents))
    }

    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> Result<CasOutcome<StepGraphPhase>> {
        let now = self.now();
        self.write(|state| state.update_phase(id, expected, patch, now))
    }

    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        Ok(self.read(|state| state.phase_rows(graph)))
    }

    async fn append_prompt_template(
        &self,
        new: NewPromptTemplate,
        expected: Option<i32>,
    ) -> Result<CasOutcome<PromptTemplate>> {
        let now = self.now();
        self.write(|state| state.append_prompt_template(new, expected, now))
    }

    async fn skills(&self) -> Result<Vec<Skill>> {
        Ok(self.read(State::skill_rows))
    }

    async fn skill_versions(&self, skill: SkillId) -> Result<Vec<SkillVersion>> {
        Ok(self.read(|state| state.skill_version_rows(skill)))
    }

    async fn skill_bindings(&self, project: Option<ProjectId>) -> Result<Vec<SkillBinding>> {
        Ok(self.read(|state| state.skill_binding_rows(project)))
    }

    async fn create_skill(&self, new: NewSkill) -> Result<(Skill, SkillVersion)> {
        let now = self.now();
        self.write(|state| state.create_skill(new, now))
    }

    async fn update_skill(
        &self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
    ) -> Result<CasOutcome<Skill>> {
        let now = self.now();
        self.write(|state| state.update_skill(id, expected, patch, now))
    }

    async fn add_skill_version(
        &self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
    ) -> Result<CasOutcome<SkillVersion>> {
        let now = self.now();
        self.write(|state| state.add_skill_version(skill, expected, new, now))
    }

    async fn set_skill_binding(
        &self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> Result<CasOutcome<Option<SkillBinding>>> {
        let now = self.now();
        self.write(|state| state.set_skill_binding(key, expected, change, now))
    }

    async fn personas(&self) -> Result<Vec<Persona>> {
        Ok(self.read(State::persona_rows))
    }

    async fn create_persona(&self, new: NewPersona) -> Result<Persona> {
        let now = self.now();
        self.write(|state| state.create_persona(new, now))
    }

    async fn update_persona(
        &self,
        id: PersonaId,
        expected: DateTime<Utc>,
        patch: PersonaPatch,
    ) -> Result<CasOutcome<Persona>> {
        let now = self.now();
        self.write(|state| state.update_persona(id, expected, patch, now))
    }

    async fn delete_persona(&self, id: PersonaId) -> Result<()> {
        self.write(|state| state.delete_persona(id))
    }

    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<StoredSetting>> {
        let now = self.now();
        self.write(|state| state.set_setting(rung, key, value, expected, now))
    }

    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>> {
        let now = self.now();
        self.write(|state| state.clear_setting(rung, key, expected, now))
    }

    async fn setting(&self, rung: SettingRung, key: SettingKey) -> Result<Option<StoredSetting>> {
        if let Some(refusal) = rung_refusal(key, rung.flag()) {
            return Err(StoreError::Constraint(refusal));
        }
        Ok(self.read(|state| state.stored_setting(rung, key)))
    }

    async fn queue_setting(
        &self,
        target: QueueTarget,
        key: QueueSetting,
    ) -> Result<Option<QueueStored>> {
        if let Some(refusal) = queue_target_refusal(key, target) {
            return Err(StoreError::Constraint(refusal));
        }
        let user = self.this_user();
        Ok(self.read(|state| state.queue_setting(user, target, key)))
    }

    async fn set_queue_setting(
        &self,
        target: QueueTarget,
        key: QueueSetting,
        value: Value,
        expected: QueueToken,
    ) -> Result<CasOutcome<QueueStored>> {
        // Both before the write lock: `this_user` takes the read lock (blueprint F-K).
        let user = self.this_user();
        let now = self.now();
        self.write(|state| state.write_queue_setting(user, target, key, Some(value), expected, now))
    }

    async fn clear_queue_setting(
        &self,
        target: QueueTarget,
        key: QueueSetting,
        expected: QueueToken,
    ) -> Result<CasOutcome<QueueStored>> {
        let user = self.this_user();
        let now = self.now();
        self.write(|state| state.write_queue_setting(user, target, key, None, expected, now))
    }

    async fn delete_reach(&self, target: DeleteTarget) -> Result<Option<DeleteReach>> {
        Ok(self.read(|state| match target {
            DeleteTarget::Workspace(id) => state.workspace_reach(id),
            DeleteTarget::Project(id) => state.project_reach(id).map(|(reach, _)| reach),
        }))
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<DeleteReach> {
        self.write(|state| state.delete_workspace(id))
    }

    async fn delete_project(&self, id: ProjectId) -> Result<DeleteReach> {
        let now = self.now();
        self.write(|state| state.delete_project(id, now))
    }

    // MOD-4 milestones 1 and 2, in ANA-2 §8's order. `create_run`, `claim_run`, `select_fanout`,
    // `write_document`, `promote_step`, `finish_run` and `close_out` are each a **single** `write`
    // closure, so plan M1 D6's and M2 D7's seven transactions are atomic here by construction and
    // not by discipline.

    async fn create_run(&self, new: NewRun) -> Result<Run> {
        let now = self.now();
        self.write(|state| state.create_run(new, now))
    }

    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> Result<Claim> {
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        self.write(|state| state.claim_run(run, box_id, owner, at, ttl, now))
    }

    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::RefreshLease)?;
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        self.write(|state| state.refresh_lease(run, owner, ttl, now))
    }

    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>> {
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        Ok(self.write(|state| state.adopt_runs(box_id, owner, ttl, now)))
    }

    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool> {
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        self.write(|state| state.take_lease(run, box_id, owner, ttl, now))
    }

    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::ReleaseLease)?;
        let now = self.now();
        self.write(|state| state.release_lease(run, owner, now))
    }

    async fn create_step(&self, new: NewRunStep) -> Result<RunStep> {
        let now = self.now();
        self.write(|state| state.create_step(new, now))
    }

    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.transition_run(run, from, to, at, now))
    }

    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.transition_step(step, from, to, at, now))
    }

    async fn finish_step(
        &self,
        fence: StepFence,
        step: StepId,
        outcome: StepOutcome,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.finish_step(fence, step, outcome, now))
    }

    async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.interrupt_step(step, note, at, now))
    }

    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.answer_gate(step, outcome, note, at, now))
    }

    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.select_fanout(run, position, attempt, winner, reason, now))
    }

    async fn supersede_step(&self, step: StepId) -> Result<()> {
        let now = self.now();
        self.write(|state| state.supersede_step(step, now))
    }

    async fn upsert_step_tree(
        &self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.upsert_step_tree(fence, step, trees, now))
    }

    async fn record_commits(
        &self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> Result<()> {
        self.write(|state| state.record_commits(fence, step, commits))
    }

    async fn record_command_run(&self, new: NewCommandRun) -> Result<CommandRun> {
        self.write(|state| state.record_command_run(new))
    }

    async fn command_runs(&self, step: StepId) -> Result<Vec<CommandRun>> {
        Ok(self.read(|state| state.command_runs(step)))
    }

    async fn write_document(&self, new: NewDocument) -> Result<Document> {
        self.write(|state| state.write_document(new))
    }

    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> Result<()> {
        let now = self.now();
        self.write(|state| state.promote_step(step, at, now))
    }

    async fn record_opening(&self, step: StepId, opening: StepOpening) -> Result<()> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::RecordOpening)?;
        let now = self.now();
        self.write(|state| state.record_opening(step, opening, now))
    }

    async fn pass_step(
        &self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.pass_step(fence, step, note, at, now))
    }

    async fn park_step(&self, fence: StepFence, step: StepId) -> Result<ParkOutcome> {
        let now = self.now();
        self.write(|state| state.park_step(fence, step, now))
    }

    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> Result<()> {
        let now = self.now();
        self.write(|state| state.fail_run(run, failure, at, now))
    }

    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.finish_run(run, to, failure, at, now))
    }

    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document> {
        let now = self.now();
        self.write(|state| state.close_out(item, resolution, summary, commits, now))
    }

    async fn add_note(&self, note: NewNote) -> Result<Note> {
        self.write(|state| state.add_note(note))
    }

    // ---- MOD-11 (plan D13, B-4..B-6): the agent writes, one `write` closure each ----

    async fn write_step_document(&self, fence: StepFence, new: NewDocument) -> Result<Document> {
        self.write(|state| state.write_step_document(fence, new))
    }

    async fn add_step_note(&self, fence: StepFence, note: NewNote) -> Result<Note> {
        self.write(|state| state.add_step_note(fence, note))
    }

    async fn propose_link(&self, fence: StepFence, link: ProposeLink) -> Result<ItemLink> {
        let now = self.now();
        self.write(|state| state.propose_link(fence, link, now))
    }

    async fn withdraw_link(&self, fence: StepFence, link: WithdrawLink) -> Result<ItemLink> {
        let now = self.now();
        self.write(|state| state.withdraw_link(fence, link, now))
    }

    async fn item_by_key(&self, project: ProjectId, key: &str) -> Result<Option<ItemId>> {
        Ok(self.read(|state| state.item_by_key(project, key)))
    }

    async fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::LeaseHolds)?;
        self.read(|state| state.lease_holds(run, fence))
    }

    // ---- MOD-11 M4 (plan D14, B-16): the command queue, one `write` closure each ----

    async fn enqueue_command(&self, new: NewCommandRun) -> Result<CommandRun> {
        let now = self.now();
        self.write(|state| state.enqueue_command(new, now))
    }

    async fn claim_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        limit: u32,
    ) -> Result<Option<CommandRun>> {
        let now = self.now();
        self.write(|state| state.claim_command(id, claimant, limit, now))
    }

    async fn beat_command(&self, id: CommandRunId, claimant: Uuid) -> Result<bool> {
        let now = self.now();
        Ok(self.write(|state| state.beat_command(id, claimant, now)))
    }

    async fn finish_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        status: CommandRunStatus,
        exit_code: Option<i32>,
        output: Option<String>,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.finish_command(id, claimant, status, exit_code, output, now))
    }

    async fn cancel_command(&self, id: CommandRunId) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.cancel_command(id, now))
    }

    // ---- ANA-11 §5.1 (MOD-38): requirements and citations, one `write` closure each ----

    async fn set_requirement_spec(
        &self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
    ) -> Result<CasOutcome<RequirementSpec>> {
        let now = self.now();
        self.write(|state| {
            state.set_requirement_spec(project, expected_version, owner_id, preamble, now)
        })
    }

    async fn create_requirement_area(&self, new: NewRequirementArea) -> Result<RequirementArea> {
        let now = self.now();
        self.write(|state| state.create_requirement_area(new, now))
    }

    async fn mint_requirement(
        &self,
        area: RequirementAreaId,
        new: NewRequirement,
    ) -> Result<Requirement> {
        let now = self.now();
        self.write(|state| state.mint_requirement(area, new, now))
    }

    async fn amend_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        amended_by: ItemId,
    ) -> Result<RequirementUpdate> {
        let now = self.now();
        self.write(|state| {
            state.revise_requirement(
                id,
                expected_version,
                patch,
                (amended_by, CitationKind::Amends),
                now,
            )
        })
    }

    async fn withdraw_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        withdrawn_by: ItemId,
        author_id: UserId,
        box_id: Option<BoxId>,
    ) -> Result<RequirementUpdate> {
        let now = self.now();
        let patch = RequirementPatch {
            author_id,
            box_id,
            reason: "withdrawn".to_owned(),
            ..RequirementPatch::default()
        };
        self.write(|state| {
            state.revise_requirement(
                id,
                expected_version,
                patch,
                (withdrawn_by, CitationKind::Withdraws),
                now,
            )
        })
    }

    async fn cite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        proposed_by: Option<StepId>,
    ) -> Result<ItemRequirement> {
        let now = self.now();
        self.write(|state| state.cite(item, requirement, kind, proposed_by, now))
    }

    async fn uncite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<()> {
        let now = self.now();
        self.write(|state| state.uncite(item, requirement, kind, now))
    }

    async fn reconfirm(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<ItemRequirement> {
        let now = self.now();
        self.write(|state| state.reconfirm(item, requirement, kind, now))
    }

    // -- MOD-42: the permission and control relay (plan D1-D5, D12-D14)

    async fn open_permission(&self, open: OpenPermission) -> Result<PermissionId> {
        let now = self.now();
        self.write(|state| state.open_permission(open, now))
    }

    async fn permission(&self, id: PermissionId) -> Result<Option<StepPermission>> {
        Ok(self.read(|state| state.permissions.get(&id).map(|p| p.row.clone())))
    }

    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> Result<Option<PermissionChoice>> {
        let now = self.now();
        self.write(|state| state.apply_permission(id, owner, now))
    }

    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> Result<u64> {
        let now = self.now();
        self.write(|state| state.settle_permissions(session, to, now))
    }

    async fn request_cancel(
        &self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
    ) -> Result<CancelRequest> {
        let now = self.now();
        self.write(|state| state.request_cancel(run, user, box_id, now))
    }

    async fn pending_commands(&self, owner: Uuid, box_id: BoxId) -> Result<Vec<RunCommand>> {
        let now = self.now();
        Ok(self.read(|state| state.pending_commands(owner, box_id, now)))
    }

    async fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.resolve_command(id, to, resolution, now))
    }

    async fn relay_view(&self, item: ItemId) -> Result<RelayView> {
        let now = self.now();
        Ok(self.read(|state| state.relay_view(item, now)))
    }

    async fn answer_permission(
        &self,
        id: PermissionId,
        option_id: &str,
        user: UserId,
        box_id: BoxId,
    ) -> Result<AnswerOutcome> {
        let now = self.now();
        self.write(|state| state.answer_permission(id, option_id, user, box_id, now))
    }

    async fn open_permissions(&self, scope: &Scope) -> Result<Vec<WaitingPermission>> {
        let now = self.now();
        Ok(self.read(|state| state.open_permissions(scope, now)))
    }

    // -- MOD-70: follow-ups for engine steps (plan D1-D5, D9)

    async fn request_follow_up(&self, new: NewFollowUp) -> Result<FollowUpRequest> {
        let now = self.now();
        self.write(|state| state.request_follow_up(new, now))
    }

    async fn open_follow_ups(
        &self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
    ) -> Result<bool> {
        let now = self.now();
        self.write(|state| state.open_follow_ups(run, step, session, owner, now))
    }

    async fn next_follow_up(
        &self,
        step: StepId,
        session: RelaySessionId,
    ) -> Result<Option<QueuedFollowUp>> {
        Ok(self.read(|state| state.next_follow_up(step, session)))
    }

    async fn settle_follow_up(
        &self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
    ) -> Result<SettleOutcome> {
        let now = self.now();
        self.write(|state| state.settle_follow_up(id, owner, to, now))
    }

    async fn close_follow_ups(
        &self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
    ) -> Result<u64> {
        let now = self.now();
        Ok(self.write(|state| state.close_follow_ups(step, session, reason, now)))
    }

    async fn close_dropped_follow_ups(&self, run: RunId, owner: Uuid, reason: &str) -> Result<u64> {
        let now = self.now();
        Ok(self.write(|state| state.close_dropped_follow_ups(run, owner, reason, now)))
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use std::sync::Arc;

    use super::{FollowUpPayload, MemStore};
    use crate::clock::{Clock as _, TestClock};
    use crate::fixtures::ids;
    use crate::model::{
        AgentBox, AgentId, AnswerOutcome, BatchClose, BoxId, BoxProbe, CancelRequest, ChatRunSpec,
        CitationKind, Claim, CommandRunId, CommandRunStatus, DocumentId, FOLLOW_UP_SESSION_ENDED,
        FollowUpRequest, FollowUpSettle, FollowUpText, FollowUpWindow, GateOutcome, GraphSnapshot,
        Isolation, ItemId, ItemKindPatch, NewCommandRun, NewDocument, NewFollowUp, NewItem,
        NewNote, NewProject, NewRepo, NewRequirement, NewRequirementArea, NewRun, NewRunStep,
        NoteId, OpenPermission, OverlapRule, PermissionId, PhaseAgent, Priority, ProbedTool,
        ProjectId, ProjectPatch, RelayOption, RelayOptionKind, RelaySessionId, RepoId,
        RequirementAreaId, RequirementId, RequirementPatch, RequirementUpdate, Resolution,
        RunCommand, RunCommandKind, RunCommandStatus, RunId, RunKind, RunMode, RunStatus,
        RunStepCommit, RunStepTree, Scope, SettleOutcome, SnapshotGraph, SnapshotSettings, Status,
        StepId, StepOutcome, StepStatus, TIMESTAMPTZ_DIGITS, UserId, VerifyOutcome,
        executor_scrub_refusal,
    };
    use crate::prompt::settings::SettingKey;
    use crate::prompt::{DEFAULT_TEMPLATES, body_of};
    use crate::store::error::StoreError;
    use crate::store::{
        CasOutcome, DeleteTarget, ReadStore as _, SettingRung, StepFence, WriteStore as _,
    };
    use chrono::{SubsecRound as _, TimeDelta, Utc};
    use serde_json::{Value, json};
    use uuid::Uuid;

    /// MOD-76 D4 (R-55): `set_box_setting` writes the key on every clone's row, leaves the
    /// other keys and `edit_version` alone, and answers whether the row exists.
    #[tokio::test]
    async fn a_box_setting_is_set_on_every_clone() {
        let store = MemStore::demo();
        let before = store
            .box_row(ids::BOX)
            .await
            .expect("the read")
            .expect("the demo box");
        let clone = store.clone();
        assert!(store.set_box_setting(ids::BOX, "command_limits", json!({ "verify": 3 })));
        let after = clone
            .box_row(ids::BOX)
            .await
            .expect("the read")
            .expect("the demo box");
        assert_eq!(after.settings["command_limits"], json!({ "verify": 3 }));
        assert_eq!(after.edit_version, before.edit_version);
        let mut kept = after.settings.clone();
        let mut was = before.settings.clone();
        kept.as_object_mut()
            .expect("an object")
            .remove("command_limits");
        if let Some(object) = was.as_object_mut() {
            object.remove("command_limits");
        }
        assert_eq!(kept, was, "no other key moves");
        assert!(!store.set_box_setting(BoxId::new(), "command_limits", json!({})));
    }

    /// MOD-40 plan D11 (T7): a `MemStore` stamps with the clock its handle was given, and a
    /// clone given another clock stamps with that one. Without a clock it stamps the wall clock,
    /// untruncated (blueprint B21).
    #[tokio::test]
    async fn a_mem_store_reads_its_clock() {
        let edit = |description: &str| ProjectPatch {
            description: Some(description.to_owned()),
            ..ProjectPatch::default()
        };
        let clock = TestClock::at(Utc::now() - TimeDelta::days(3));
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        clock.advance(TimeDelta::minutes(7));
        let project = store
            .project(ids::PROJECT_HTUI)
            .await
            .expect("read")
            .expect("fixture");
        let CasOutcome::Applied(edited) = store
            .update_project(project.id, project.updated_at, edit("first"))
            .await
            .expect("the edit must not fail")
        else {
            panic!("the token was read from the store");
        };
        assert_eq!(
            edited.updated_at,
            clock.now(),
            "the write is stamped by the handle's clock"
        );

        let later = store.handle_at(clock.now() + TimeDelta::hours(1));
        let CasOutcome::Applied(again) = later
            .update_project(project.id, edited.updated_at, edit("second"))
            .await
            .expect("the edit must not fail")
        else {
            panic!("the token is the first edit's");
        };
        assert_eq!(
            again.updated_at,
            clock.now() + TimeDelta::hours(1),
            "a handle's clock is its own; the rows are shared"
        );
        assert_eq!(
            store
                .project(ids::PROJECT_HTUI)
                .await
                .expect("read")
                .expect("fixture")
                .updated_at,
            again.updated_at,
            "both handles read one set of rows"
        );

        let wall = MemStore::demo();
        let before = Utc::now();
        let unclocked = wall
            .project(ids::PROJECT_HTUI)
            .await
            .expect("read")
            .expect("fixture");
        let CasOutcome::Applied(stamped) = wall
            .update_project(unclocked.id, unclocked.updated_at, edit("third"))
            .await
            .expect("the edit must not fail")
        else {
            panic!("the token was read from the store");
        };
        assert!(stamped.updated_at >= before, "no clock: the wall clock");
    }

    /// MOD-4 plan D152: a switched-on fault answers `Unreachable` on every clone, before the
    /// write looks at a row, and the write answers as before once it is switched off.
    #[tokio::test]
    async fn a_switched_on_fault_answers_unreachable_until_switched_off() {
        let store = MemStore::demo();
        let clone = store.clone();
        let ghost = RunId::new();

        store.set_fault(super::MemFault::ReleaseLease, true);
        assert!(
            matches!(
                clone.release_lease(ghost, Uuid::now_v7()).await,
                Err(StoreError::Unreachable(_))
            ),
            "the clone shares the switch, and the missing row is never looked up"
        );
        assert!(
            matches!(
                clone
                    .refresh_lease(ghost, Uuid::now_v7(), TimeDelta::zero())
                    .await,
                Err(StoreError::NotFound { .. })
            ),
            "only the named write fails"
        );

        store.set_fault(super::MemFault::ReleaseLease, false);
        assert!(
            matches!(
                clone.release_lease(ghost, Uuid::now_v7()).await,
                Err(StoreError::NotFound { .. })
            ),
            "switched off, the write answers as it did"
        );
    }

    /// Plan D69: the tests-only writer reaches `project.settings`, the column no seam writer
    /// touches, and both readers see it: the trait's `project` and the inherent
    /// `project_settings`. `updated_at` moves with it, as `set_app_setting`'s does.
    #[tokio::test]
    async fn set_project_settings_is_read_back_by_project() {
        let store = MemStore::demo();
        let before = store
            .project(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture project");
        let settings = json!({ "judge_agent_id": ids::AGENT_AGY, "token_budget": 42 });

        store.set_project_settings(ids::PROJECT_HTUI, settings.clone());

        let after = store
            .project(ids::PROJECT_HTUI)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture project");
        assert_eq!(after.settings, settings, "the blob is replaced whole");
        assert!(
            after.updated_at > before.updated_at,
            "the write stamps `updated_at`"
        );
        assert_eq!(
            store
                .project_settings(ids::PROJECT_HTUI)
                .await
                .expect("MemStore never fails a read"),
            Some(settings),
            "the inherent reader sees the same blob"
        );
    }

    /// MOD-10 M3 blueprint A-6: the tests-only setter plants both secret columns, raw, and
    /// stamps `updated_at`; `None` clears them; an unknown project is left alone.
    #[tokio::test]
    async fn set_project_secret_columns_plants_both() {
        let store = MemStore::demo();
        async fn read(store: &MemStore) -> crate::model::Project {
            store
                .project(ids::PROJECT_HTUI)
                .await
                .expect("MemStore never fails a read")
                .expect("the fixture project")
        }
        let before = read(&store).await;
        assert_eq!(
            (
                before.secret_provider.as_deref(),
                before.secret_scope.as_deref()
            ),
            (None, None),
            "the fixture project has no secret columns"
        );

        // No validation: a scope the provider would refuse is planted as is.
        store.set_project_secret_columns(ids::PROJECT_HTUI, Some("infisical"), Some("not json"));
        let after = read(&store).await;
        assert_eq!(
            (
                after.secret_provider.as_deref(),
                after.secret_scope.as_deref()
            ),
            (Some("infisical"), Some("not json"))
        );
        assert!(
            after.updated_at > before.updated_at,
            "the write stamps `updated_at`"
        );
        assert_eq!(
            (after.settings, after.name),
            (before.settings, before.name),
            "nothing else moves"
        );

        store.set_project_secret_columns(ids::PROJECT_HTUI, None, None);
        let cleared = read(&store).await;
        assert_eq!(
            (cleared.secret_provider, cleared.secret_scope),
            (None, None),
            "`None` clears both columns"
        );

        let unknown = ProjectId::new();
        store.set_project_secret_columns(unknown, Some("infisical"), Some("{}"));
        assert_eq!(
            store
                .project(unknown)
                .await
                .expect("MemStore never fails a read"),
            None,
            "an unknown project is a no-op, not an insert"
        );
    }

    /// The columns `store::conformance` cannot see, because §6.1 returns neither `run_step.usage`
    /// nor `run_step.prompt_digest` (plan D15(a)). Read straight out of `State`, which is what a
    /// unit test in this module is for; `pg_criteria.rs` asserts the same rule in SQL.
    #[tokio::test]
    async fn set_step_usage_writes_usage_every_time_and_the_digest_only_when_supplied() {
        let store = MemStore::demo();
        store
            .set_step_usage(
                StepFence::Unleased,
                ids::STEP_IMPL,
                json!({ "input_tokens": 7 }),
                Some("abc".to_owned()),
            )
            .await
            .expect("the first write lands");
        store
            .set_step_usage(
                StepFence::Unleased,
                ids::STEP_IMPL,
                json!({ "input_tokens": 9 }),
                None,
            )
            .await
            .expect("the second write lands");

        let step = store.read(|state| {
            state
                .steps
                .get(&ids::STEP_IMPL)
                .cloned()
                .expect("the fixture step")
        });
        assert_eq!(
            step.usage,
            Some(json!({ "input_tokens": 9 })),
            "usage is overwritten by every call"
        );
        assert_eq!(
            step.prompt_digest.as_deref(),
            Some("abc"),
            "a `None` digest leaves the stored one alone (plan D15(b))"
        );
    }

    /// `set_step_prompt` writes `prompt_digest` and `trim_record` and **nothing else**
    /// (`docs/ANA-5.md` §4.4).
    ///
    /// The conformance case can only see the two `RunStepSummary` fields, because §6.1 returns
    /// neither column; this reads `State` directly, which is what a unit test in this module is
    /// for. `usage` is the column that proves "nothing else": the fixture leaves it `None`, a
    /// `set_step_usage` call fills it, and a later `set_step_prompt` must not disturb it — the two
    /// writers of `prompt_digest` share the column and must not share anything more.
    #[tokio::test]
    async fn set_step_prompt_writes_both_columns() {
        let store = MemStore::demo();
        store
            .set_step_usage(
                StepFence::Unleased,
                ids::STEP_IMPL,
                json!({ "input_tokens": 7 }),
                None,
            )
            .await
            .expect("the usage write lands");
        store
            .set_step_prompt(
                StepFence::Unleased,
                ids::STEP_IMPL,
                "9f8e",
                &json!({ "estimated_after": 34_000, "sections": [], "v": 1 }),
            )
            .await
            .expect("the prompt write lands");

        let step = store.read(|state| {
            state
                .steps
                .get(&ids::STEP_IMPL)
                .cloned()
                .expect("the fixture step")
        });
        assert_eq!(
            step.prompt_digest.as_deref(),
            Some("9f8e"),
            "the digest is written unconditionally, unlike `set_step_usage`'s optional one"
        );
        assert_eq!(
            step.trim_record,
            Some(json!({ "estimated_after": 34_000, "sections": [], "v": 1 })),
            "the record is stored whole, not a projection of it"
        );
        assert_eq!(
            step.usage,
            Some(json!({ "input_tokens": 7 })),
            "the pre-flight audit does not touch the post-flight figure"
        );

        let unknown = store
            .set_step_prompt(StepFence::Unleased, StepId::new(), "9f8e", &json!({}))
            .await;
        assert!(
            matches!(
                unknown,
                Err(StoreError::NotFound {
                    entity: "run_step",
                    ..
                })
            ),
            "an unknown step is NotFound, got {unknown:?}"
        );
    }

    /// The read the preview makes for its skills section: `R-SKL-2`'s collapse over the fixture's
    /// two project bindings and one phase override (`docs/ANA-5.md` §4.2).
    ///
    /// The fixture is built so that three mistakes are each caught by a different assertion.
    /// Rendering `rust-style` twice — the bug OpenHands had to fix — shows up in the length.
    /// Ignoring the phase's `pinned_version` shows up as v2 where v1 is in force. And keeping the
    /// project order rather than the collapsed `(position, name bytes)` one shows up as
    /// `rust-style` before `tests`, because the phase binding sits at position 2 and `tests` at 0.
    #[tokio::test]
    async fn a_preview_style_bound_skills_read_collapses_overrides() {
        let store = MemStore::demo();

        let project_level = store
            .bound_skills(ids::PROJECT_HTUI, None)
            .await
            .expect("bound_skills must not fail");
        assert_eq!(
            project_level
                .iter()
                .map(|skill| (skill.name.as_str(), skill.version, skill.position))
                .collect::<Vec<_>>(),
            vec![("tests", Some(1), 0), ("rust-style", Some(2), 1)],
            "with no phase, the project bindings alone, each at its latest version"
        );

        let with_phase = store
            .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
            .await
            .expect("bound_skills must not fail");
        assert_eq!(
            with_phase
                .iter()
                .map(|skill| (skill.name.as_str(), skill.version, skill.position))
                .collect::<Vec<_>>(),
            vec![("tests", Some(1), 0), ("rust-style", Some(1), 2)],
            "the phase binding overrides the project one: once, pinned to v1, at position 2"
        );
        assert_eq!(
            with_phase[1].body, "Prefer `expect` with a reason.",
            "the pinned version's body, not the latest one's"
        );

        assert!(
            store
                .bound_skills(ids::PROJECT_AGY, None)
                .await
                .expect("bound_skills must not fail")
                .is_empty(),
            "a project with no bindings has no skills, not every skill"
        );
    }

    /// The demo data plus one global attachment (ANA-22 §6 item 2): skill `house`, v1 body
    /// `"House rules."`, attached with `project_id` and `phase_id` both `None`, at position 5.
    fn demo_with_a_global_skill() -> (MemStore, crate::model::SkillId) {
        let mut data = crate::fixtures::demo_data();
        let house = crate::model::SkillId::new();
        let now = Utc::now();
        data.skills.push(crate::model::Skill {
            id: house,
            name: "house".to_owned(),
            description: "House rules for every project.".to_owned(),
            created_by: ids::USER,
            created_at: now,
            updated_at: now,
        });
        data.skill_versions.push(crate::model::SkillVersion {
            skill_id: house,
            version: 1,
            body: "House rules.".to_owned(),
            source: json!({}),
            created_by: ids::USER,
            created_at: now,
        });
        data.skill_bindings.push(crate::model::SkillBinding {
            id: crate::model::SkillBindingId::new(),
            skill_id: house,
            project_id: None,
            phase_id: None,
            pinned_version: None,
            position: 5,
            activation: crate::model::Activation::Always,
            globs: Vec::new(),
            languages: Vec::new(),
            updated_at: now,
        });
        (MemStore::from_demo(data), house)
    }

    /// ANA-22 §6 item 2: a global attachment is a candidate of every project's steps, and sorts
    /// with the project's and the phase's by `(position, name bytes)`.
    #[tokio::test]
    async fn a_global_attachment_reaches_every_project() {
        use crate::model::SkillLevel;
        let (store, house) = demo_with_a_global_skill();

        let agy = store
            .bound_skills(ids::PROJECT_AGY, None)
            .await
            .expect("bound_skills must not fail");
        assert_eq!(
            agy.iter()
                .map(|s| (s.skill_id, s.level, s.version, s.position))
                .collect::<Vec<_>>(),
            vec![(house, SkillLevel::Global, Some(1), 5)],
            "a project with no attachment of its own still gets the global one"
        );
        assert_eq!(agy[0].body, "House rules.");

        let implement = store
            .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
            .await
            .expect("bound_skills must not fail");
        assert_eq!(
            implement
                .iter()
                .map(|s| (s.name.as_str(), s.level, s.version, s.position))
                .collect::<Vec<_>>(),
            vec![
                ("tests", SkillLevel::Project, Some(1), 0),
                ("rust-style", SkillLevel::Phase, Some(1), 2),
                ("house", SkillLevel::Global, Some(1), 5),
            ],
            "the global attachment joins the project's and the phase's, in (position, name) order"
        );
    }

    /// ANA-22 §7.1: `project_id` keeps `ON DELETE CASCADE`, which never fires for a NULL key, so
    /// a global attachment survives a project delete and the reach counts the project's own rows.
    #[tokio::test]
    async fn delete_project_keeps_global_attachments() {
        let (store, house) = demo_with_a_global_skill();

        let reach = store
            .delete_project(ids::PROJECT_HTUI)
            .await
            .expect("the delete lands");
        assert_eq!(
            reach.skill_bindings, 3,
            "the reach counts the project's own three attachments, not the global one"
        );
        assert_eq!(
            store
                .bound_skills(ids::PROJECT_AGY, None)
                .await
                .expect("bound_skills must not fail")
                .iter()
                .map(|s| s.skill_id)
                .collect::<Vec<_>>(),
            vec![house],
            "the global attachment is still every remaining project's"
        );
    }

    /// The four rules of the amended §7.3 walk that a reader would get wrong by analogy with
    /// [`ReadStore::links`], asserted against the same fixture from the same root.
    ///
    /// The conformance case pins the walk's output; this one pins it against `links`, because the
    /// two traversals sit twenty lines apart in this file and the failure mode is copying the
    /// wrong one. `links(AGY_FIX_1, 2)` is undirected, un-kind-filtered and unscoped, so it
    /// reaches strictly more items — and every extra item it reaches is a rule this walk keeps.
    #[tokio::test]
    async fn upstream_walk_is_directed_and_kind_filtered() {
        let store = MemStore::demo();
        let scope = crate::model::PromptScope::from_scope(
            &Scope {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
            },
            ids::PROJECT_AGY,
        );

        let upstream = store
            .upstream_summaries(ids::AGY_FIX_1, 2, &scope)
            .await
            .expect("the walk must not fail");
        let walked: Vec<ItemId> = upstream.iter().map(|entry| entry.item_id).collect();

        /// Every item [`ReadStore::links`] reaches, root included: the undirected comparison.
        async fn neighbourhood(store: &MemStore, root: ItemId, hops: u8) -> Vec<ItemId> {
            let graph = store
                .links(root, hops)
                .await
                .expect("the neighbourhood must not fail");
            graph.nodes.iter().map(|node| node.item_id).collect()
        }

        let around_fix_1 = neighbourhood(&store, ids::AGY_FIX_1, 2).await;
        for reached in &walked {
            assert!(
                around_fix_1.contains(reached),
                "the directed walk cannot reach what the undirected one does not"
            );
        }
        assert!(
            !walked.contains(&ids::AGY_FIX_1),
            "the root is the step's own item and is never an upstream entry"
        );
        assert_eq!(
            walked.iter().filter(|id| **id == ids::HTUI_ANA_1).count(),
            1,
            "the diamond's apex is reached twice at depth 2 and rendered once"
        );

        // Directed. `htui:ANA-1` has four live edges and every one of them points *at* it, so the
        // undirected `links` finds four neighbours and the upstream walk finds nothing at all.
        let htui = crate::model::PromptScope::from_scope(
            &Scope {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
            },
            ids::PROJECT_HTUI,
        );
        assert_eq!(
            neighbourhood(&store, ids::HTUI_ANA_1, 1).await.len(),
            5,
            "root + 4"
        );
        assert!(
            store
                .upstream_summaries(ids::HTUI_ANA_1, 1, &htui)
                .await
                .expect("the walk must not fail")
                .is_empty(),
            "every edge at ANA-1 is incoming, and `to_item_id` is the only one followed"
        );

        // Kind-filtered. `FEAT-3` has one `origin` edge and one `relates` edge, both outgoing.
        let feat_3: Vec<ItemId> = store
            .upstream_summaries(ids::HTUI_FEAT_3, 1, &htui)
            .await
            .expect("the walk must not fail")
            .iter()
            .map(|entry| entry.item_id)
            .collect();
        assert_eq!(
            feat_3,
            vec![ids::HTUI_ANA_1],
            "`relates` is context, not upstream: only the `origin` target is followed"
        );
        assert!(
            neighbourhood(&store, ids::HTUI_FEAT_3, 1)
                .await
                .contains(&ids::HTUI_FEAT_1),
            "…while `links` follows the same `relates` edge, which is what it is for"
        );
    }

    /// The chat pair of plan D4: both rows minted `running`, and `finish_chat_run` closing both.
    #[tokio::test]
    async fn a_chat_run_mints_both_rows_running_and_finish_closes_them() {
        let store = MemStore::demo();
        let before = store
            .active_runs(&Scope {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
            })
            .await
            .expect("active_runs must not fail");

        let chat = ChatRunSpec::mint(
            ids::PROJECT_HTUI,
            ids::BOX,
            ids::USER,
            Some(ids::AGENT_CLAUDE),
            Some("sonnet".to_owned()),
        );
        store.start_chat_run(&chat).await.expect("the mint lands");

        let (run, step) = store.read(|state| {
            (
                state.runs.get(&chat.run_id).cloned().expect("the run row"),
                state
                    .steps
                    .get(&chat.step_id)
                    .cloned()
                    .expect("the step row"),
            )
        });
        assert_eq!(run.kind, RunKind::Chat, "run.kind");
        assert_eq!(run.mode, RunMode::Manual, "run.mode");
        assert_eq!(run.item_id, None, "run.item_id is NULL for a chat");
        assert_eq!(run.status, RunStatus::Running, "run.status");
        assert_eq!(run.finished_at, None, "run.finished_at");
        assert_eq!(run.executing_box_id, Some(ids::BOX), "run.executing_box_id");
        assert_eq!(step.position, 0, "run_step.position");
        assert_eq!(step.attempt, 1, "run_step.attempt");
        assert_eq!(step.fanout_index, 0, "run_step.fanout_index");
        assert_eq!(step.phase_name, "chat", "run_step.phase_name");
        assert_eq!(step.status, StepStatus::Running, "run_step.status");
        assert_eq!(step.agent_id, Some(ids::AGENT_CLAUDE), "run_step.agent_id");

        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
        };
        assert_eq!(
            store.active_runs(&scope).await.expect("active_runs"),
            before + 1,
            "a running chat is an active run"
        );

        let at = Utc::now();
        store
            .finish_chat_run(chat.run_id, chat.step_id, RunStatus::Done, at)
            .await
            .expect("the close lands");
        assert_eq!(
            store.active_runs(&scope).await.expect("active_runs"),
            before,
            "closing it brings the count back (plan D4, assumption A1)"
        );
        let (run, step) = store.read(|state| {
            (
                state.runs.get(&chat.run_id).cloned().expect("the run row"),
                state
                    .steps
                    .get(&chat.step_id)
                    .cloned()
                    .expect("the step row"),
            )
        });
        assert_eq!(run.finished_at, Some(at), "run.finished_at");
        assert_eq!(
            step.status,
            StepStatus::Done,
            "run_step.status by the same name"
        );
        assert_eq!(step.finished_at, Some(at), "run_step.finished_at");
    }

    /// MOD-55 plan P5: a help turn minted through `for_edit_help` lands as the same chat pair, its
    /// step named `edit_help` rather than `chat`.
    #[tokio::test]
    async fn a_help_chat_run_records_edit_help() {
        let store = MemStore::demo();
        let help = ChatRunSpec::mint(
            ids::PROJECT_HTUI,
            ids::BOX,
            ids::USER,
            Some(ids::AGENT_CLAUDE),
            None,
        )
        .for_edit_help();
        store.start_chat_run(&help).await.expect("the mint lands");

        let (run, step) = store.read(|state| {
            (
                state.runs.get(&help.run_id).cloned().expect("the run row"),
                state
                    .steps
                    .get(&help.step_id)
                    .cloned()
                    .expect("the step row"),
            )
        });
        assert_eq!(step.phase_name, "edit_help", "run_step.phase_name");
        assert_eq!(run.kind, RunKind::Chat, "run.kind");
        assert_eq!(run.item_id, None, "run.item_id is NULL for a help turn");
    }

    /// `agents()` joins this box's `agent_box`, and only this box's.
    #[tokio::test]
    async fn agents_join_this_box_only() {
        let store = MemStore::demo();
        let plain = store.agents().await.expect("agents must not fail");
        assert_eq!(
            plain
                .iter()
                .map(|row| row.agent.name.as_str())
                .collect::<Vec<_>>(),
            vec!["agy", "claude", "claude-cli"],
            "ordered by agent.name"
        );
        assert!(
            plain.iter().all(|row| row.on_box.is_none()),
            "no fixture loads agent_box"
        );

        let now = Utc::now();
        let probed = AgentBox {
            agent_id: ids::AGENT_CLAUDE,
            box_id: ids::BOX,
            enabled: true,
            version: Some("1.2.3".to_owned()),
            path: Some("claude".to_owned()),
            probed_at: Some(now),
            quota: None,
            quota_at: None,
            updated_at: now,
            probe: Some(serde_json::json!({ "status": "missing" })),
        };
        store
            .upsert_agent_box(&probed)
            .await
            .expect("the probe row lands");

        let joined = store.agents().await.expect("agents must not fail");
        let claude = joined
            .iter()
            .find(|row| row.agent.name == "claude")
            .expect("claude is registered");
        assert_eq!(
            claude
                .on_box
                .as_ref()
                .and_then(|row| row.version.as_deref()),
            Some("1.2.3"),
            "this box's agent_box is joined in"
        );
        assert_eq!(
            claude
                .on_box
                .as_ref()
                .and_then(|row| row.probe.as_ref())
                .and_then(|probe| probe["status"].as_str()),
            Some("missing"),
            "the ANA-4 §4.6 snapshot rides the join as an opaque document (MOD-2 D44)"
        );
        assert!(
            joined
                .iter()
                .find(|row| row.agent.name == "agy")
                .expect("agy is registered")
                .on_box
                .is_none(),
            "an agent with no row for this box stays None"
        );

        store
            .upsert_agent_box(&AgentBox {
                probe: None,
                ..probed
            })
            .await
            .expect("the cleared row lands");
        assert_eq!(
            store
                .agents()
                .await
                .expect("agents must not fail")
                .iter()
                .find(|row| row.agent.name == "claude")
                .and_then(|row| row.on_box.as_ref())
                .and_then(|row| row.probe.as_ref()),
            None,
            "a second upsert with `probe: None` clears the snapshot"
        );
    }

    /// MOD-2 plan D67: the narrow setter writes `quota` and `quota_at` and touches no other
    /// column - the `probe` snapshot above all, which is the whole reason the latch does not go
    /// through `upsert_agent_box`.
    ///
    /// The read-back is here rather than in `store::conformance` because [`WriteStore`] has no
    /// registry read; `pg_criteria.rs` asserts the same claim in SQL, and the two together are
    /// what keeps the backends from drifting.
    #[tokio::test]
    async fn set_agent_box_quota_leaves_probe_and_version_alone() {
        let store = MemStore::demo();
        let snapshot = serde_json::json!({ "status": "ready", "source": "probe" });
        let probed_at = Utc::now() - TimeDelta::hours(3);
        store
            .upsert_agent_box(&AgentBox {
                agent_id: ids::AGENT_CLAUDE,
                box_id: ids::BOX,
                enabled: true,
                version: Some("1.2.3".to_owned()),
                path: Some("claude".to_owned()),
                probed_at: Some(probed_at),
                quota: None,
                quota_at: None,
                updated_at: probed_at,
                probe: Some(snapshot.clone()),
            })
            .await
            .expect("the probe row lands");
        let before = store
            .read(|state| {
                state
                    .agent_boxes
                    .get(&(ids::AGENT_CLAUDE, ids::BOX))
                    .cloned()
            })
            .expect("the row is stored");

        let quota = serde_json::json!({
            "source": "acp_meta_rate_limit",
            "spend": { "session_micros": 351, "currency": "USD" },
        });
        let quota_at = Utc::now();
        assert!(
            store
                .set_agent_box_quota(ids::AGENT_CLAUDE, ids::BOX, quota.clone(), quota_at)
                .await
                .expect("the latch lands on the probed row"),
            "a first latch writes"
        );

        let after = store
            .agents()
            .await
            .expect("agents must not fail")
            .into_iter()
            .find(|row| row.agent.id == ids::AGENT_CLAUDE)
            .expect("claude is registered")
            .on_box
            .expect("this box has an agent_box row");
        assert_eq!(after.quota.as_ref(), Some(&quota), "the document is stored");
        assert_eq!(after.quota_at, Some(quota_at), "and its timestamp with it");
        assert_eq!(
            after.probe.as_ref(),
            Some(&snapshot),
            "the §4.6 snapshot is byte-identical across the latch (D67)"
        );
        assert_eq!(
            after.version.as_deref(),
            Some("1.2.3"),
            "the setter is two columns wide: `version` is not one of them"
        );
        assert_eq!(after.path.as_deref(), Some("claude"), "nor is `path`");
        assert!(after.enabled, "nor is `enabled`");
        assert_eq!(
            after.probed_at,
            Some(probed_at),
            "nor is `probed_at`: a latch is not a probe"
        );
        assert!(
            after.updated_at > before.updated_at,
            "`updated_at` moves, as Postgres's `BEFORE UPDATE` trigger moves it"
        );

        let missing = store
            .set_agent_box_quota(AgentId::new(), ids::BOX, quota, quota_at)
            .await;
        assert!(
            matches!(
                missing,
                Err(StoreError::NotFound {
                    entity: "agent_box",
                    ..
                })
            ),
            "a row that has never been probed has no columns to latch into, got {missing:?}"
        );
    }

    /// MOD-23 (plan D242): this box's summary of one agent, read back through the inherent
    /// registry read the Settings tab uses.
    async fn this_box_summary(store: &MemStore, agent_id: AgentId) -> crate::model::AgentSummary {
        store
            .agents()
            .await
            .expect("agents must not fail")
            .into_iter()
            .find(|row| row.agent.id == agent_id)
            .expect("the agent is registered")
    }

    /// MOD-23 (plan D242): a probed `agent_box` row for `agent_id` on `ids::BOX`, as the probe
    /// writes one: `probe` is its §4.6 snapshot and `enabled` its verdict.
    fn probed_box(agent_id: AgentId, probe: Value, enabled: bool) -> AgentBox {
        let probed_at = Utc::now() - TimeDelta::hours(1);
        AgentBox {
            agent_id,
            box_id: ids::BOX,
            enabled,
            version: Some("1.2.3".to_owned()),
            path: Some("/usr/bin/agent".to_owned()),
            probed_at: Some(probed_at),
            quota: None,
            quota_at: None,
            updated_at: probed_at,
            probe: Some(probe),
        }
    }

    /// MOD-23 plan D242: the human vetoes and the probe cannot overrule. An upsert that says
    /// `enabled: true` over a switched-off row writes every other column it owns and leaves
    /// `enabled` false, which is Postgres's `enabled = EXCLUDED.enabled AND NOT user_off`.
    #[tokio::test]
    async fn a_switched_off_row_stays_off_under_an_upsert_that_says_enabled() {
        let store = MemStore::demo();
        let ready = probed_box(
            ids::AGENT_CLAUDE,
            json!({ "status": "ready", "source": "probe" }),
            true,
        );
        store
            .upsert_agent_box(&ready)
            .await
            .expect("the probe row lands");
        store
            .set_agent_box_enabled(ids::AGENT_CLAUDE, ids::BOX, false)
            .await
            .expect("the switch lands");

        store
            .upsert_agent_box(&AgentBox {
                version: Some("1.3.0".to_owned()),
                ..ready
            })
            .await
            .expect("a re-probe lands on a switched-off row");

        let summary = this_box_summary(&store, ids::AGENT_CLAUDE).await;
        let row = summary.on_box.expect("this box has an agent_box row");
        assert!(!row.enabled, "the re-probe cannot switch the row back on");
        assert!(summary.user_off, "the switch is reported");
        assert_eq!(
            row.version.as_deref(),
            Some("1.3.0"),
            "the upsert still wrote the columns it owns: only `enabled` is vetoed"
        );
    }

    /// MOD-23 plan D242, blueprint D249: switching on clears the veto and re-derives `enabled`
    /// from the stored probe, so a row the probe found unusable stays unusable and a probe
    /// document without a `status` reads as not ready rather than as an error.
    #[tokio::test]
    async fn switching_on_restores_the_probe_verdict() {
        let store = MemStore::demo();
        for (agent_id, probe, enabled) in [
            (
                ids::AGENT_CLAUDE,
                json!({ "status": "ready", "source": "probe" }),
                true,
            ),
            (
                ids::AGENT_AGY,
                json!({ "status": "unauthenticated", "source": "probe" }),
                false,
            ),
            (ids::AGENT_CLAUDE_CLI, json!({ "source": "probe" }), false),
        ] {
            store
                .upsert_agent_box(&probed_box(agent_id, probe, enabled))
                .await
                .expect("the probe row lands");
            store
                .set_agent_box_enabled(agent_id, ids::BOX, false)
                .await
                .expect("switched off");
            store
                .set_agent_box_enabled(agent_id, ids::BOX, true)
                .await
                .expect("switched back on");
        }

        let ready = this_box_summary(&store, ids::AGENT_CLAUDE).await;
        assert!(
            ready.on_box.as_ref().expect("a row").enabled,
            "a `ready` row switched back on is enabled"
        );
        assert!(!ready.user_off, "and the veto is gone");

        let unauthenticated = this_box_summary(&store, ids::AGENT_AGY).await;
        assert!(
            !unauthenticated.on_box.as_ref().expect("a row").enabled,
            "an `unauthenticated` row switched back on is still unavailable"
        );
        assert!(
            !unauthenticated.user_off,
            "but it is the probe that says so now, not the switch"
        );

        let status_less = this_box_summary(&store, ids::AGENT_CLAUDE_CLI).await;
        assert!(
            !status_less.on_box.as_ref().expect("a row").enabled,
            "a probe document without `status` is not ready (D249's COALESCE)"
        );
        assert!(!status_less.user_off, "and the switch-on landed");
    }

    /// MOD-23 plan D242: the switch is two columns wide, `user_off` and `enabled`. The probe's
    /// columns and the quota latch's pair are byte-identical across an off and an on, and
    /// `updated_at` moves as Postgres's `BEFORE UPDATE` trigger moves it.
    #[tokio::test]
    async fn the_switch_leaves_probe_version_path_probed_at_and_quota_alone() {
        let store = MemStore::demo();
        store
            .upsert_agent_box(&probed_box(
                ids::AGENT_CLAUDE,
                json!({ "status": "ready", "source": "probe", "tools": { "claude": "1.2.3" } }),
                true,
            ))
            .await
            .expect("the probe row lands");
        store
            .set_agent_box_quota(
                ids::AGENT_CLAUDE,
                ids::BOX,
                json!({ "source": "acp_meta_rate_limit", "spend": { "session_micros": 351 } }),
                Utc::now(),
            )
            .await
            .expect("the latch lands");
        let before = this_box_summary(&store, ids::AGENT_CLAUDE)
            .await
            .on_box
            .expect("a row");

        store
            .set_agent_box_enabled(ids::AGENT_CLAUDE, ids::BOX, false)
            .await
            .expect("switched off");
        store
            .set_agent_box_enabled(ids::AGENT_CLAUDE, ids::BOX, true)
            .await
            .expect("switched on");

        let after = this_box_summary(&store, ids::AGENT_CLAUDE)
            .await
            .on_box
            .expect("a row");
        assert_eq!(
            AgentBox {
                updated_at: before.updated_at,
                ..after.clone()
            },
            before,
            "`probe`, `version`, `path`, `probed_at`, `quota` and `quota_at` are untouched, and \
             a `ready` row switched off and on is enabled again"
        );
        assert!(
            after.updated_at > before.updated_at,
            "`updated_at` moves, as Postgres's `BEFORE UPDATE` trigger moves it"
        );
    }

    /// MOD-23 plan D242: `user_off` is this box's. A switch on another box's row of the same
    /// registry is not reported here, just as its `agent_box` row is not joined in.
    #[tokio::test]
    async fn agents_reports_user_off_for_this_box_only() {
        let store = MemStore::demo();
        let other = BoxId::new();
        let mut elsewhere = store
            .box_row(ids::BOX)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture box");
        elsewhere.id = other;
        elsewhere.hostname = "elsewhere".to_owned();
        store.write(|state| {
            state.boxes.insert(other, elsewhere);
        });

        store
            .set_agent_box_enabled(ids::AGENT_CLAUDE, ids::BOX, false)
            .await
            .expect("switched off here");
        store
            .set_agent_box_enabled(ids::AGENT_AGY, other, false)
            .await
            .expect("switched off elsewhere");

        assert!(
            this_box_summary(&store, ids::AGENT_CLAUDE).await.user_off,
            "this box's switch is reported"
        );
        let agy = this_box_summary(&store, ids::AGENT_AGY).await;
        assert!(!agy.user_off, "another box's switch is not this box's");
        assert!(agy.on_box.is_none(), "nor is its row");

        store
            .set_agent_box_enabled(ids::AGENT_CLAUDE, ids::BOX, true)
            .await
            .expect("switched back on here");
        assert!(
            !this_box_summary(&store, ids::AGENT_CLAUDE).await.user_off,
            "switching back on clears it"
        );
    }

    /// MOD-23 plan D242, blueprint F-2: the switch on an agent this box has never probed inserts
    /// a bare row, so the setting survives until the first probe. Every probe column is empty,
    /// which is how a reader tells it from a probed row.
    #[tokio::test]
    async fn a_switch_on_an_unprobed_agent_inserts_a_bare_row() {
        let store = MemStore::demo();
        assert!(
            this_box_summary(&store, ids::AGENT_AGY)
                .await
                .on_box
                .is_none(),
            "the demo holds no agent_box row"
        );

        store
            .set_agent_box_enabled(ids::AGENT_AGY, ids::BOX, false)
            .await
            .expect("an absent pair is written, not refused");

        let summary = this_box_summary(&store, ids::AGENT_AGY).await;
        assert!(summary.user_off, "the switch is reported");
        let row = summary.on_box.expect("the switch inserted a row");
        assert_eq!(
            AgentBox {
                agent_id: ids::AGENT_AGY,
                box_id: ids::BOX,
                enabled: false,
                version: None,
                path: None,
                probed_at: None,
                quota: None,
                quota_at: None,
                updated_at: row.updated_at,
                probe: None,
            },
            row,
            "a bare row: switched off, and never probed"
        );
    }

    /// MOD-40 plan D4: a latch older than the stored `quota_at` is `Ok(false)` and writes
    /// nothing — not the document, not the instant, and not `updated_at` either, which Postgres's
    /// `BEFORE UPDATE` trigger would not move for a guarded `UPDATE` that matched no row. The
    /// conformance case `an_older_quota_is_a_no_op` pins the order through the seam; this reads
    /// the stored pair back.
    #[tokio::test]
    async fn an_older_quota_leaves_the_stored_pair_alone() {
        let store = MemStore::demo();
        let probed_at = Utc::now() - TimeDelta::hours(3);
        store
            .upsert_agent_box(&AgentBox {
                agent_id: ids::AGENT_CLAUDE,
                box_id: ids::BOX,
                enabled: true,
                version: Some("1.2.3".to_owned()),
                path: Some("claude".to_owned()),
                probed_at: Some(probed_at),
                quota: None,
                quota_at: None,
                updated_at: probed_at,
                probe: Some(serde_json::json!({ "status": "ready", "source": "probe" })),
            })
            .await
            .expect("the probe row lands");
        let t2 = Utc::now();
        let t1 = t2 - TimeDelta::minutes(1);
        assert!(
            store
                .set_agent_box_quota(
                    ids::AGENT_CLAUDE,
                    ids::BOX,
                    serde_json::json!({ "v": 2 }),
                    t2
                )
                .await
                .expect("the newer latch finds its row"),
            "the first latch writes"
        );
        let stored = || {
            store
                .read(|state| {
                    state
                        .agent_boxes
                        .get(&(ids::AGENT_CLAUDE, ids::BOX))
                        .cloned()
                })
                .expect("the row is stored")
        };
        let before = stored();

        assert!(
            !store
                .set_agent_box_quota(
                    ids::AGENT_CLAUDE,
                    ids::BOX,
                    serde_json::json!({ "v": 1 }),
                    t1
                )
                .await
                .expect("the older latch finds its row"),
            "an older latch is refused"
        );
        assert_eq!(
            stored(),
            before,
            "a refused latch writes nothing, not even the trigger's stamp"
        );
    }

    /// MOD-7 D32: `MemStore` refuses a spec digest that is not 64 lowercase hex, as Postgres's
    /// `CHECK` on `box.probe_spec_digest` does, and writes nothing. The Postgres half is T1's
    /// `CHECK` test in `htui-store`.
    #[tokio::test]
    async fn a_probe_digest_that_is_not_hex_is_a_constraint() {
        let store = MemStore::demo();
        let before = store.boxes().await.expect("boxes must not fail");
        let good = crate::prompt::digest::sha256_hex("spec");
        for digest in [
            String::new(),
            "abc".to_owned(),
            good.to_uppercase(),
            format!("{good}0"),
            format!("{}g", &good[..63]),
        ] {
            let probe = BoxProbe {
                box_id: ids::BOX,
                os_version: "11".to_owned(),
                cpu: "cpu".to_owned(),
                ram_mb: Some(1024),
                gpu_present: false,
                gpu_vendor: None,
                tools: vec![ProbedTool {
                    name: "git".to_owned(),
                    version: "2.0".to_owned(),
                    path: "/usr/bin/git".to_owned(),
                }],
                probed_tags: vec!["x".to_owned()],
                htui_version: "0.0.0".to_owned(),
                spec_digest: digest.clone(),
                probed_at: Utc::now(),
            };
            let refused = store.record_box_probe(&probe).await;
            assert!(
                matches!(refused, Err(StoreError::Constraint(_))),
                "digest {digest:?} must be a Constraint, got {refused:?}"
            );
            assert_eq!(
                store.boxes().await.expect("boxes must not fail"),
                before,
                "a refused probe writes nothing"
            );
        }
    }

    /// MOD-7 D37 (blueprint; a deferred T2 finding): `boxes()` lists only this user's boxes, by
    /// ascending id, each box's tools by name bytes, the order `PgStore`'s `ORDER BY id` and
    /// `COLLATE "C"` give. Eight more boxes of the fixture user all sort **before** the fixture's
    /// own, and `State.boxes` is a `HashMap`, so only the sort puts nine rows in id order (an
    /// accidental hash order is about one in 362 880); another user's box and its tool never appear.
    #[tokio::test]
    async fn boxes_lists_only_this_user_s_boxes_in_id_order() {
        use crate::model::{AppUser, BoxTool};

        let mut data = crate::fixtures::demo_data();
        let template = data
            .boxes
            .iter()
            .find(|row| row.id == ids::BOX)
            .expect("the fixture box")
            .clone();
        let second = BoxId::from_uuid(Uuid::from_u128(1));
        let foreign = BoxId::from_uuid(Uuid::from_u128(2));
        assert!(
            second < ids::BOX && foreign < ids::BOX,
            "the planted ids sort before the fixture's"
        );
        let stranger = UserId::new();
        let now = Utc::now();
        data.users.push(AppUser {
            id: stranger,
            name: "stranger".to_owned(),
            email: None,
            created_at: now,
            updated_at: now,
        });
        let mut mine = template.clone();
        mine.id = second;
        mine.hostname = "SECOND-BOX".to_owned();
        let mut theirs = template;
        theirs.id = foreign;
        theirs.user_id = stranger;
        theirs.hostname = "ELSEWHERE".to_owned();
        data.boxes.push(mine);
        data.boxes.push(theirs);
        let extra: Vec<BoxId> = (3..=9)
            .map(|n| BoxId::from_uuid(Uuid::from_u128(n)))
            .collect();
        for (n, id) in extra.iter().enumerate() {
            let mut row = data
                .boxes
                .iter()
                .find(|row| row.id == second)
                .expect("the second box")
                .clone();
            row.id = *id;
            row.hostname = format!("EXTRA-{n}");
            data.boxes.push(row);
        }
        for (box_id, name) in [
            (second, "awk"),
            (second, "Zig"),
            (second, "_x"),
            (foreign, "leak"),
        ] {
            data.box_tools.push(BoxTool {
                box_id,
                name: name.to_owned(),
                version: "1".to_owned(),
                path: format!("/usr/bin/{name}"),
                probed_at: now,
            });
        }
        let store = MemStore::from_demo(data);
        assert_eq!(
            store.this_user(),
            Some(ids::USER),
            "the fixture user is still the oldest"
        );

        let records = store.boxes().await.expect("boxes must not fail");

        let listed: Vec<BoxId> = records.iter().map(|record| record.row.id).collect();
        let mut expected = vec![second];
        expected.extend(extra.iter().copied());
        expected.push(ids::BOX);
        assert_eq!(listed, expected, "this user's nine boxes, by ascending id");
        assert!(
            records.iter().all(|record| record.row.user_id == ids::USER),
            "no other user's box is listed: {records:?}"
        );
        let tools: Vec<&str> = records[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(tools, ["Zig", "_x", "awk"], "tools by name bytes");
        let fixture_tools: Vec<&str> = records[records.len() - 1]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(
            fixture_tools,
            ["cargo", "cmake", "git", "rustc"],
            "the fixture box keeps its own tools, sorted"
        );
    }

    /// MOD-7 milestone 2 (D41, fact-check): `edit_box` reaches only this user's boxes, as `boxes()`
    /// does. Another user's box is `NotFound`, even with its right token, and writes nothing; its
    /// `NotFound` also wins over a spent token and a refused tag (precedence). The Postgres half is
    /// `box_identity.rs::another_users_box_is_not_found_by_edit_box`.
    #[tokio::test]
    async fn edit_box_refuses_another_user_s_box_as_not_found() {
        use crate::model::{AppUser, BoxEdit};

        let mut data = crate::fixtures::demo_data();
        let mut theirs = data
            .boxes
            .iter()
            .find(|row| row.id == ids::BOX)
            .expect("the fixture box")
            .clone();
        let foreign = BoxId::from_uuid(Uuid::from_u128(2));
        let stranger = UserId::new();
        let now = Utc::now();
        data.users.push(AppUser {
            id: stranger,
            name: "stranger".to_owned(),
            email: None,
            created_at: now,
            updated_at: now,
        });
        theirs.id = foreign;
        theirs.user_id = stranger;
        theirs.hostname = "ELSEWHERE".to_owned();
        data.boxes.push(theirs);
        let store = MemStore::from_demo(data);
        assert_ne!(
            store.this_user(),
            Some(stranger),
            "the fixture user stays this user: the stranger was created later"
        );
        let before = store.box_row(foreign).await.expect("box_row never fails");
        assert!(before.is_some(), "the stranger's box is stored");

        let tags = |tags: &[&str]| BoxEdit {
            declared_tags: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
            quirks: None,
            executor: None,
        };
        let right_token = store.edit_box(foreign, 0, tags(&["gpu"])).await;
        assert!(
            matches!(right_token, Err(StoreError::NotFound { entity: "box", .. })),
            "another user's box is NotFound even with its right token, got {right_token:?}"
        );
        let precedence = store.edit_box(foreign, 7, tags(&["BAD"])).await;
        assert!(
            matches!(precedence, Err(StoreError::NotFound { entity: "box", .. })),
            "NotFound wins over a spent token and a refused tag, got {precedence:?}"
        );
        assert_eq!(
            store.box_row(foreign).await.expect("box_row never fails"),
            before,
            "the stranger's row is untouched"
        );
    }

    /// MOD-2 plan D70: `project.settings` is readable per project, because the per-run token cap
    /// lives in it and a chat reads it at `ChatStart`.
    ///
    /// A project the store does not hold answers `None` rather than an empty document: the caller
    /// refuses the chat, and an invented `{}` would silently mean "this project has no cap".
    #[tokio::test]
    async fn project_settings_reads_the_column_or_nothing() {
        let store = MemStore::demo();
        let settings = store
            .project_settings(ids::PROJECT_HTUI)
            .await
            .expect("the read must not fail")
            .expect("the demo fixture holds this project");
        assert!(
            settings.is_object(),
            "`project.settings` is `JSONB NOT NULL`, so the value is a document: {settings}"
        );
        assert_eq!(
            store
                .project_settings(ProjectId::new())
                .await
                .expect("the read must not fail"),
            None,
            "a project this store has never seen is absent, not unconfigured"
        );
    }

    /// MOD-2 plan D74: `upsert_agent_box` can neither set nor clear `quota` / `quota_at`, so a
    /// re-probe cannot discard a latch that landed after it read the row.
    ///
    /// The memory half of the claim `pg_criteria.rs` makes in SQL. Both paths, because both are
    /// `EXCLUDED.quota` in the statement being changed: an insert carrying a quota stores `None`,
    /// and a conflict carrying one preserves the stored pair.
    #[tokio::test]
    async fn upsert_agent_box_cannot_write_the_quota_columns() {
        let store = MemStore::demo();
        let stamp = Utc::now();
        let probed = AgentBox {
            agent_id: ids::AGENT_CLAUDE,
            box_id: ids::BOX,
            enabled: true,
            version: Some("1.2.3".to_owned()),
            path: Some("claude".to_owned()),
            probed_at: Some(stamp),
            quota: Some(json!({ "invented": "by the probe" })),
            quota_at: Some(stamp),
            updated_at: stamp,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        };
        store
            .upsert_agent_box(&probed)
            .await
            .expect("the insert lands");
        let fresh = store
            .read(|state| {
                state
                    .agent_boxes
                    .get(&(ids::AGENT_CLAUDE, ids::BOX))
                    .cloned()
            })
            .expect("the row is stored");
        assert_eq!(
            (fresh.quota.as_ref(), fresh.quota_at),
            (None, None),
            "the two columns are not in the INSERT list: a probe has no business seeding an \
             allowance it never observed"
        );
        assert_eq!(
            fresh.probe.as_ref(),
            probed.probe.as_ref(),
            "every other column the upsert does write is written"
        );

        let latched =
            json!({ "source": "acp_meta_rate_limit", "spend": { "session_micros": 351 } });
        let quota_at = Utc::now();
        store
            .set_agent_box_quota(ids::AGENT_CLAUDE, ids::BOX, latched.clone(), quota_at)
            .await
            .expect("the only writer of the two columns writes them");

        // The conflict path: the re-probe hands back the row it read *before* the latch.
        store
            .upsert_agent_box(&AgentBox {
                version: Some("1.3.0".to_owned()),
                quota: Some(json!({ "stale": "read before the latch" })),
                quota_at: None,
                ..probed.clone()
            })
            .await
            .expect("the update lands");
        let after = store
            .read(|state| {
                state
                    .agent_boxes
                    .get(&(ids::AGENT_CLAUDE, ids::BOX))
                    .cloned()
            })
            .expect("the row is stored");
        assert_eq!(
            after.quota.as_ref(),
            Some(&latched),
            "the stored document is still the latch's; the upsert's is discarded, not the other \
             way round (D74)"
        );
        assert_eq!(after.quota_at, Some(quota_at), "and its timestamp with it");
        assert_eq!(
            after.version.as_deref(),
            Some("1.3.0"),
            "the columns the upsert *does* own still take the new row's values"
        );

        // Nor does a `None` clear them, the way a `None` probe clears its own column — and nothing
        // else does either (review L-6).
        store
            .upsert_agent_box(&AgentBox {
                quota: None,
                quota_at: None,
                ..probed
            })
            .await
            .expect("the second update lands");
        let cleared = store
            .read(|state| {
                state
                    .agent_boxes
                    .get(&(ids::AGENT_CLAUDE, ids::BOX))
                    .cloned()
            })
            .expect("the row is stored");
        assert_eq!(
            cleared.quota.as_ref(),
            Some(&latched),
            "`upsert_agent_box` has no way to clear a latch either — and today **nothing** does: \
             `set_agent_box_quota` takes a `Value` and a `DateTime`, so the single writer can \
             replace the pair and not blank it (the gap is documented on the trait method, and \
             MOD-7's unregistration is the caller that turns both into `Option`s)"
        );
        assert_eq!(cleared.quota_at, Some(quota_at));
    }

    /// PRD's "`project.settings` loses nothing", asserted on the bytes rather than on `Value`
    /// equality, which is what `MemStore` can promise and JSONB cannot: Postgres normalises key
    /// order and number text, so `settings_project_rung_merges_keys` asserts per-key `Value`
    /// equality on both stores and this one asserts the stronger thing on the one store that can.
    ///
    /// The seeded blob carries keys MOD-4 and MOD-12 own and this writer has never heard of. A
    /// typed `ProjectSettings` round-trip — the shape D7 exists to refuse — would drop every one
    /// of them, and `1.5` is there because it would also be the first to come back as `1.5000001`.
    #[tokio::test]
    async fn set_setting_project_rung_leaves_unknown_keys_byte_identical() {
        let store = MemStore::demo();
        let seeded = json!({
            "token_budget": 90_000,
            "retention_days": 30,
            "keep_raw_events": true,
            "orchestration": { "max_parallel_steps": 3, "window": ["22:00", "06:00"] },
            "per_token_cap_run": 1.5,
        });
        let token = store.write(|state| {
            let project = state
                .projects
                .get_mut(&ids::PROJECT_HTUI)
                .expect("the fixture project");
            project.settings = seeded.clone();
            project.updated_at
        });
        let before = seeded.to_string();

        let written = store
            .set_setting(
                SettingRung::Project(ids::PROJECT_HTUI),
                SettingKey::UpstreamHops,
                json!(1),
                Some(token),
            )
            .await
            .expect("the merge lands");
        let CasOutcome::Applied(written) = written else {
            panic!("the token was read a statement ago: {written:?}");
        };

        /// `project.settings` as stored, read back through the trait rather than out of `State`:
        /// the merge has to be visible where the resolvers look.
        async fn settings(store: &MemStore, label: &str) -> Value {
            store
                .project(ids::PROJECT_HTUI)
                .await
                .unwrap_or_else(|error| panic!("{label}: the read must not fail: {error}"))
                .unwrap_or_else(|| panic!("{label}: the project survives its settings write"))
                .settings
        }

        let merged = settings(&store, "after the merge").await;
        for (key, value) in seeded.as_object().expect("a JSON object") {
            assert_eq!(
                merged.get(key).map(ToString::to_string),
                Some(value.to_string()),
                "`{key}` is byte-identical to what was there before the merge"
            );
        }
        let mut expected = seeded.clone();
        expected
            .as_object_mut()
            .expect("a JSON object")
            .insert("upstream_hops".to_owned(), json!(1));
        assert_eq!(
            merged.to_string(),
            expected.to_string(),
            "the document is the one that was there plus exactly one key"
        );

        store
            .clear_setting(
                SettingRung::Project(ids::PROJECT_HTUI),
                SettingKey::UpstreamHops,
                written.updated_at,
            )
            .await
            .expect("the clear lands");
        assert_eq!(
            settings(&store, "after the clear").await.to_string(),
            before,
            "the clear leaves the document exactly as the merge found it"
        );
    }

    /// Review L4: the two writers of `project.settings` agree about a blob that is not an object.
    ///
    /// `set_setting` refuses it - a key cannot be merged into a scalar - while `clear_setting` used
    /// to reach for `as_object_mut`, find `None`, drop the `remove` on the floor and answer
    /// `Applied` with a freshly advanced token. "The key is gone" and "the key was never reachable"
    /// are different facts and only one of them was true.
    ///
    /// Nothing in the tree writes a non-object today - `project.settings` is `JSONB NOT NULL
    /// DEFAULT '{}'` on Postgres and `create_project` seeds `{}` here - which is exactly why it is
    /// worth pinning: the refusal is the only thing standing between a hand-edited row and a clear
    /// that reports success.
    #[tokio::test]
    async fn clear_setting_refuses_a_project_settings_that_is_not_an_object() {
        let store = MemStore::demo();
        let rung = SettingRung::Project(ids::PROJECT_HTUI);
        let token = store.write(|state| {
            let project = state
                .projects
                .get_mut(&ids::PROJECT_HTUI)
                .expect("the fixture project");
            project.settings = json!("a hand-edited scalar");
            project.updated_at
        });

        let merged = store
            .set_setting(rung, SettingKey::UpstreamHops, json!(1), Some(token))
            .await;
        let cleared = store
            .clear_setting(rung, SettingKey::UpstreamHops, token)
            .await;
        match (merged, cleared) {
            (Err(StoreError::Constraint(on_set)), Err(StoreError::Constraint(on_clear))) => {
                assert_eq!(
                    on_clear, on_set,
                    "one blob, one sentence: the two writers say the same thing about it"
                );
                assert!(
                    on_clear.contains("is not a JSON object"),
                    "and it names what is wrong with the blob, got `{on_clear}`"
                );
            }
            other => panic!("both writers refuse a non-object, got {other:?}"),
        }

        let (settings, after) = store.read(|state| {
            let project = state
                .projects
                .get(&ids::PROJECT_HTUI)
                .expect("the fixture project");
            (project.settings.clone(), project.updated_at)
        });
        assert_eq!(
            settings,
            json!("a hand-edited scalar"),
            "a refused clear removed nothing"
        );
        assert_eq!(after, token, "and did not advance the token either");
    }

    /// MOD-42 I-4: every time the relay writes is the handle's clock (`clock_timestamp()` on
    /// Postgres), never a box's wall clock: opening, answering, applying and requesting a cancel
    /// on a handle frozen at `t` stamp exactly `t`.
    #[tokio::test]
    async fn relay_times_are_the_handles_clock() {
        // A `TestClock` truncates to Postgres's digits, so `t` is read back through one.
        let t = TestClock::at(Utc::now() - TimeDelta::days(2)).now();
        let store = MemStore::demo().handle_at(t);
        let owner = Uuid::now_v7();
        let run = store
            .create_run(graph_run(ids::HTUI_ANA_2, ids::PROJECT_HTUI, Vec::new()))
            .await
            .expect("the run is created")
            .id;
        assert_eq!(
            store
                .claim_run(run, ids::BOX, owner, t, TimeDelta::minutes(5))
                .await
                .expect("the claim lands"),
            Claim::Admitted
        );
        let step = store
            .create_step(new_step(run, 0, 1, 0))
            .await
            .expect("the step is created")
            .id;
        let open = |request: &str| OpenPermission {
            id: PermissionId::new(),
            run_id: run,
            run_step_id: step,
            session: RelaySessionId::new(),
            request_id: request.to_owned(),
            tool_call_id: None,
            summary: None,
            options: vec![RelayOption {
                id: "allow-once".to_owned(),
                label: "Allow once".to_owned(),
                kind: RelayOptionKind::AllowOnce,
            }],
            owner,
        };
        let id = store
            .open_permission(open("req-1"))
            .await
            .expect("the request parks");
        assert_eq!(
            store
                .answer_permission(id, "allow-once", ids::USER, ids::BOX)
                .await
                .expect("the answer lands"),
            AnswerOutcome::Answered
        );
        assert!(
            store
                .apply_permission(id, owner)
                .await
                .expect("the apply lands")
                .is_some(),
            "the owner applies the answer"
        );
        let row = store
            .permission(id)
            .await
            .expect("read")
            .expect("the row exists");
        assert_eq!(row.created_at, t, "created_at is the handle's clock");
        assert_eq!(
            row.answered_at,
            Some(t),
            "answered_at is the handle's clock"
        );
        assert_eq!(
            row.resolved_at,
            Some(t),
            "resolved_at is the handle's clock"
        );

        // MOD-70 I-4: the window's and the follow-up's times are the handle's clock too.
        assert!(
            store
                .transition_step(step, StepStatus::Pending, StepStatus::Running, t)
                .await
                .expect("the step runs")
        );
        let session = RelaySessionId::new();
        assert!(
            store
                .open_follow_ups(run, step, session, owner)
                .await
                .expect("the window opens")
        );
        let follow_up = NewFollowUp {
            id: crate::model::RunCommandId::new(),
            run_step_id: step,
            text: FollowUpText::new("use the smaller fixture".to_owned()).expect("prose"),
            issued_by: ids::USER,
            issued_box: ids::BOX,
        };
        let queued = follow_up.id;
        assert_eq!(
            store
                .request_follow_up(follow_up)
                .await
                .expect("the follow-up is written"),
            FollowUpRequest::Queued(queued)
        );
        assert_eq!(
            store
                .settle_follow_up(queued, owner, FollowUpSettle::Applied)
                .await
                .expect("the settle lands"),
            SettleOutcome::Settled
        );
        assert_eq!(
            store
                .close_follow_ups(step, session, FOLLOW_UP_SESSION_ENDED)
                .await
                .expect("the close lands"),
            0
        );
        let windows = store.follow_up_windows();
        assert_eq!(
            windows
                .iter()
                .map(|w| (w.opened_at, w.closed_at))
                .collect::<Vec<_>>(),
            vec![(t, Some(t))],
            "opened_at and closed_at are the handle's clock"
        );
        let rows = store.follow_up_rows();
        assert_eq!(
            rows.iter()
                .map(|(row, _)| (row.issued_at, row.resolved_at))
                .collect::<Vec<_>>(),
            vec![(t, Some(t))],
            "a follow-up's issued_at and resolved_at are the handle's clock"
        );

        assert!(matches!(
            store
                .request_cancel(run, ids::USER, ids::BOX)
                .await
                .expect("the cancel is written"),
            CancelRequest::Inserted(_)
        ));
        let commands = store.pending_commands(owner, ids::BOX).await.expect("read");
        assert_eq!(
            commands.iter().map(|row| row.issued_at).collect::<Vec<_>>(),
            vec![t],
            "issued_at is the handle's clock"
        );

        // No trait method reads a resolved command back, so the stored outcome is read here.
        let cancel = commands[0].id;
        assert!(
            store
                .resolve_command(
                    cancel,
                    RunCommandStatus::Refused,
                    Some("the run is already done".to_owned())
                )
                .await
                .expect("the resolve lands"),
            "the pending cancel is refused"
        );
        let resolved = store
            .command_rows()
            .into_iter()
            .find(|row| row.id == cancel)
            .expect("the command row survives its resolution");
        assert_eq!(
            (
                resolved.status,
                resolved.resolution.as_deref(),
                resolved.resolved_at
            ),
            (
                RunCommandStatus::Refused,
                Some("the run is already done"),
                Some(t)
            ),
            "the resolution is stored, and resolved_at is the handle's clock"
        );
    }

    /// MOD-70 I-5, blueprint B-1: a follow-up's text is stored exactly while the row is pending.
    /// One row is resolved by each path that resolves one — the executor's two settles, a close,
    /// a superseding open, a cancel, a dropped walk's close and `resolve_command` — and every one
    /// of them drops the text; the row left pending keeps it.
    #[tokio::test]
    async fn a_follow_ups_text_is_cleared_on_every_resolution() {
        let store = MemStore::demo();
        let owner = Uuid::now_v7();
        let at = Utc::now();
        let leased = |item: ItemId| {
            let store = &store;
            async move {
                let run = store
                    .create_run(graph_run(item, ids::PROJECT_HTUI, Vec::new()))
                    .await
                    .expect("the run is created")
                    .id;
                assert_eq!(
                    store
                        .claim_run(run, ids::BOX, owner, at, TimeDelta::minutes(5))
                        .await
                        .expect("the claim lands"),
                    Claim::Admitted
                );
                run
            }
        };
        let running = |run: RunId, position: i32| {
            let store = &store;
            async move {
                let step = store
                    .create_step(new_step(run, position, 1, 0))
                    .await
                    .expect("the step is created")
                    .id;
                assert!(
                    store
                        .transition_step(step, StepStatus::Pending, StepStatus::Running, at)
                        .await
                        .expect("the step runs")
                );
                let session = RelaySessionId::new();
                assert!(
                    store
                        .open_follow_ups(run, step, session, owner)
                        .await
                        .expect("the window opens")
                );
                (step, session)
            }
        };
        let queue = |step: StepId| {
            let store = &store;
            async move {
                let new = NewFollowUp {
                    id: crate::model::RunCommandId::new(),
                    run_step_id: step,
                    text: FollowUpText::new("use the smaller fixture".to_owned()).expect("prose"),
                    issued_by: ids::USER,
                    issued_box: ids::BOX,
                };
                let id = new.id;
                assert_eq!(
                    store.request_follow_up(new).await.expect("the enqueue"),
                    FollowUpRequest::Queued(id)
                );
                id
            }
        };
        let other = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: "follow-up".to_owned(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands")
            .id;
        let run_a = leased(ids::HTUI_ANA_2).await;
        let run_b = leased(other).await;

        let (s1, _) = running(run_a, 0).await;
        let applied = queue(s1).await;
        assert_eq!(
            store
                .settle_follow_up(applied, owner, FollowUpSettle::Applied)
                .await
                .expect("settle"),
            SettleOutcome::Settled
        );
        let (s2, _) = running(run_a, 1).await;
        let refused = queue(s2).await;
        assert_eq!(
            store
                .settle_follow_up(
                    refused,
                    owner,
                    FollowUpSettle::Refused(executor_scrub_refusal("anthropic_api_key"))
                )
                .await
                .expect("settle"),
            SettleOutcome::Settled
        );
        let (s3, w3) = running(run_a, 2).await;
        queue(s3).await;
        assert_eq!(
            store
                .close_follow_ups(s3, w3, FOLLOW_UP_SESSION_ENDED)
                .await
                .expect("close"),
            1
        );
        let (s4, _) = running(run_a, 3).await;
        queue(s4).await;
        assert!(
            store
                .open_follow_ups(run_a, s4, RelaySessionId::new(), owner)
                .await
                .expect("the superseding open")
        );
        let (s5, _) = running(run_a, 4).await;
        let resolved = queue(s5).await;
        assert!(
            store
                .resolve_command(
                    resolved,
                    RunCommandStatus::Refused,
                    Some("by hand".to_owned())
                )
                .await
                .expect("resolve")
        );
        let (s6, _) = running(run_a, 5).await;
        queue(s6).await;
        assert!(matches!(
            store
                .request_cancel(run_a, ids::USER, ids::BOX)
                .await
                .expect("the cancel"),
            CancelRequest::Inserted(_)
        ));
        let (b1, _) = running(run_b, 0).await;
        queue(b1).await;
        assert_eq!(
            store
                .close_dropped_follow_ups(run_b, owner, FOLLOW_UP_SESSION_ENDED)
                .await
                .expect("the dropped walk's close"),
            1
        );
        let (b2, _) = running(run_b, 1).await;
        let pending = queue(b2).await;

        let rows = store.follow_up_rows();
        assert_eq!(rows.len(), 8, "one row per path and the pending one");
        for (row, stored) in rows {
            assert_eq!(
                stored,
                row.id == pending,
                "the text is stored exactly while the row is pending, got {row:?}"
            );
            assert_eq!(
                row.status == RunCommandStatus::Pending,
                row.id == pending,
                "only the last row is pending, got {row:?}"
            );
        }
    }

    /// MOD-11 OQ-3: a `running` row whose claimant stopped beating holds its slot until the next
    /// claim of its `(box, class)` sees the heartbeat older than [`COMMAND_STALE_AFTER`]; that
    /// claim fails it with [`reaped_note`] and admits the next row. Exactly the stale-after bound
    /// is not yet stale, a beat moves the bound, and another class's row is not touched.
    /// Postgres's half: `pg_criteria.rs::a_stale_heartbeat_is_reaped_by_the_next_claim`.
    ///
    /// [`COMMAND_STALE_AFTER`]: crate::store::traits::COMMAND_STALE_AFTER
    /// [`reaped_note`]: crate::store::traits::reaped_note
    #[tokio::test]
    async fn a_stale_running_row_is_reaped_by_the_next_claim() {
        let t0 = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
        let clock = Arc::new(TestClock::at(t0));
        let store = MemStore::demo().with_clock(clock.clone());
        let row = |class: &str, at| NewCommandRun {
            id: CommandRunId::new(),
            run_step_id: ids::STEP_R2_PRD,
            box_id: ids::BOX,
            class: class.to_owned(),
            command: format!("make {class}"),
            cwd: "/srv".to_owned(),
            status: CommandRunStatus::Queued,
            exit_code: None,
            output: None,
            queued_at: at,
            started_at: None,
            finished_at: None,
        };
        let stuck = row("build", t0);
        let next = row("build", t0 + TimeDelta::seconds(1));
        let other = row("test", t0 + TimeDelta::seconds(2));
        for new in [&stuck, &next, &other] {
            store.enqueue_command(new.clone()).await.expect("queued");
        }
        let gone = Uuid::now_v7();
        let alive = Uuid::now_v7();
        assert!(
            store
                .claim_command(stuck.id, gone, 1)
                .await
                .expect("claim")
                .is_some()
        );
        assert!(
            store
                .claim_command(other.id, alive, 1)
                .await
                .expect("claim")
                .is_some()
        );
        assert_eq!(
            store.claim_command(next.id, alive, 1).await.expect("claim"),
            None,
            "the stuck row holds the one build slot"
        );

        clock.advance(crate::store::traits::COMMAND_STALE_AFTER);
        assert_eq!(
            store.claim_command(next.id, alive, 1).await.expect("claim"),
            None,
            "exactly three beats old is not yet stale"
        );
        assert!(
            store.beat_command(stuck.id, gone).await.expect("beat"),
            "a beat at the bound keeps the row alive"
        );
        clock.advance(crate::store::traits::COMMAND_STALE_AFTER);
        assert_eq!(
            store.claim_command(next.id, alive, 1).await.expect("claim"),
            None,
            "the beat moved the bound"
        );

        clock.advance(TimeDelta::seconds(1));
        let admitted = store.claim_command(next.id, alive, 1).await.expect("claim");
        assert_eq!(
            admitted.map(|row| (row.id, row.status)),
            Some((next.id, CommandRunStatus::Running)),
            "past the bound the stuck row is reaped and the next admitted"
        );
        let rows = store.command_runs(ids::STEP_R2_PRD).await.expect("rows");
        let reaped = rows.iter().find(|row| row.id == stuck.id).expect("stuck");
        assert_eq!(
            (reaped.status, reaped.finished_at, reaped.output.clone()),
            (
                CommandRunStatus::Failed,
                Some(clock.now()),
                Some(crate::store::traits::reaped_note())
            ),
            "failed, finished at the reaping claim, with the note"
        );
        assert!(
            !store.beat_command(stuck.id, gone).await.expect("beat"),
            "the reaped claimant's next beat answers false: its executor kills the child"
        );
        assert!(
            !store
                .finish_command(stuck.id, gone, CommandRunStatus::Done, Some(0), None)
                .await
                .expect("finish"),
            "and its finish lands nothing"
        );
        let untouched = rows.iter().find(|row| row.id == other.id).expect("other");
        assert_eq!(
            untouched.status,
            CommandRunStatus::Running,
            "another class's row is not reaped by a build claim, stale as it is"
        );
    }

    /// MOD-11 OQ-3 for a `queued` row (R-3): a row whose waiter died — a host killed outright, a
    /// call dropped before its guard existed, a cancel the store refused — would stand first in
    /// its `(box, class)` line for ever, so `enqueue_command` stamps its heartbeat and every
    /// claim of it beats it. The next claim of the pair that sees a `queued` row's heartbeat
    /// older than [`COMMAND_STALE_AFTER`] cancels it with [`reaped_note`]; a waiter that keeps
    /// asking is never reaped, and the asking row's own beat comes before the reap.
    /// Postgres's half: `pg_criteria.rs::an_orphaned_queued_row_is_reaped_by_the_next_claim`.
    ///
    /// [`COMMAND_STALE_AFTER`]: crate::store::traits::COMMAND_STALE_AFTER
    /// [`reaped_note`]: crate::store::traits::reaped_note
    #[tokio::test]
    async fn an_orphaned_queued_row_is_reaped_by_the_next_claim() {
        let t0 = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
        let clock = Arc::new(TestClock::at(t0));
        let store = MemStore::demo().with_clock(clock.clone());
        let row = |at| NewCommandRun {
            id: CommandRunId::new(),
            run_step_id: ids::STEP_R2_PRD,
            box_id: ids::BOX,
            class: "build".to_owned(),
            command: "make".to_owned(),
            cwd: "/srv".to_owned(),
            status: CommandRunStatus::Queued,
            exit_code: None,
            output: None,
            queued_at: at,
            started_at: None,
            finished_at: None,
        };
        let orphan = row(t0);
        let waiting = row(t0 + TimeDelta::seconds(1));
        let next = row(t0 + TimeDelta::seconds(2));
        for new in [&orphan, &waiting, &next] {
            store.enqueue_command(new.clone()).await.expect("queued");
        }
        let claimant = Uuid::now_v7();
        assert_eq!(
            store
                .claim_command(next.id, claimant, 1)
                .await
                .expect("claim"),
            None,
            "the orphan stands first in line"
        );

        clock.advance(crate::store::traits::COMMAND_STALE_AFTER);
        assert_eq!(
            store
                .claim_command(next.id, claimant, 1)
                .await
                .expect("claim"),
            None,
            "exactly three beats after its enqueue the orphan is not yet stale"
        );
        assert_eq!(
            store
                .claim_command(waiting.id, Uuid::now_v7(), 1)
                .await
                .expect("claim"),
            None,
            "the waiting row asks, which beats it"
        );

        clock.advance(TimeDelta::seconds(1));
        assert_eq!(
            store
                .claim_command(next.id, claimant, 1)
                .await
                .expect("claim"),
            None,
            "past the bound the orphan is reaped, and the waiter that kept asking is next"
        );
        let rows = store.command_runs(ids::STEP_R2_PRD).await.expect("rows");
        let status = |id| rows.iter().find(|row| row.id == id).expect("row").clone();
        let reaped = status(orphan.id);
        assert_eq!(
            (reaped.status, reaped.finished_at, reaped.output),
            (
                CommandRunStatus::Cancelled,
                Some(clock.now()),
                Some(crate::store::traits::reaped_note())
            ),
            "the orphan is cancelled, finished at the reaping claim, with the note"
        );
        assert_eq!(
            status(waiting.id).status,
            CommandRunStatus::Queued,
            "the asking waiter is untouched"
        );
        assert!(
            store
                .claim_command(waiting.id, Uuid::now_v7(), 1)
                .await
                .expect("claim")
                .is_some(),
            "and is admitted next"
        );
        assert!(
            matches!(
                store.claim_command(orphan.id, Uuid::now_v7(), 1).await,
                Err(StoreError::Constraint(_))
            ),
            "the reaped row is no longer claimable"
        );
    }

    /// PRD D13's cascade with no ghost left behind, read straight out of `State`.
    ///
    /// `project_delete_takes_everything_and_says_so` asserts the counts, which is all §6.1 can
    /// see: no trait reader returns a `prompt_template`, an `item_key_counter` or a
    /// `skill_binding`, and none returns a row that *should* have gone. This walks every map
    /// instead, and asserts the second thing a count cannot — that nothing surviving points at
    /// something that did not.
    #[tokio::test]
    async fn delete_project_leaves_no_row_in_any_map() {
        let store = MemStore::demo();
        let gone = ids::PROJECT_HTUI;
        // MOD-42 (F-18): the fixture seeds no relay row, so one of each is staged on `htui` runs.
        assert!(matches!(
            store
                .request_cancel(ids::RUN_2, ids::USER, ids::BOX)
                .await
                .expect("the cancel is written"),
            CancelRequest::Inserted(_)
        ));
        let owner = Uuid::now_v7();
        let staged = PermissionId::new();
        store
            .write(|state| {
                state.lease_owners.insert(ids::RUN_1, owner);
                state.open_permission(
                    OpenPermission {
                        id: staged,
                        run_id: ids::RUN_1,
                        run_step_id: ids::STEP_IMPL,
                        session: RelaySessionId::new(),
                        request_id: "req-1".to_owned(),
                        tool_call_id: None,
                        summary: None,
                        options: Vec::new(),
                        owner,
                    },
                    Utc::now(),
                )
            })
            .expect("the request parks");
        assert_eq!(
            (store.relay_rows().len(), store.command_rows().len()),
            (1, 1),
            "precondition: one relay row of each table"
        );
        // MOD-70 plan D1: a window and a pending follow-up on the same `htui` step, staged.
        let follow_up = crate::model::RunCommandId::new();
        store.write(|state| {
            state.follow_up_windows.insert(
                ids::STEP_IMPL,
                FollowUpWindow {
                    run_step_id: ids::STEP_IMPL,
                    run_id: ids::RUN_1,
                    session: RelaySessionId::new(),
                    owner,
                    opened_at: Utc::now(),
                    closed_at: None,
                },
            );
            state.run_commands.insert(
                follow_up,
                RunCommand {
                    id: follow_up,
                    run_id: ids::RUN_1,
                    kind: RunCommandKind::FollowUp,
                    issued_by: ids::USER,
                    issued_box: ids::BOX,
                    status: RunCommandStatus::Pending,
                    resolution: None,
                    issued_at: Utc::now(),
                    resolved_at: None,
                },
            );
            state.follow_up_payloads.insert(
                follow_up,
                FollowUpPayload {
                    run_step_id: ids::STEP_IMPL,
                    text: Some("use the smaller fixture".to_owned()),
                },
            );
        });
        assert_eq!(
            (
                store.follow_up_windows().len(),
                store.follow_up_rows().len()
            ),
            (1, 1),
            "precondition: one window and one follow-up"
        );
        // MOD-11 H-21: a claimed `command_run` row on an `htui` step, so the claim map has an
        // entry the cascade must take.
        let queued = NewCommandRun {
            id: CommandRunId::new(),
            run_step_id: ids::STEP_R2_PRD,
            box_id: ids::BOX,
            class: "build".to_owned(),
            command: "make".to_owned(),
            cwd: "/srv".to_owned(),
            status: CommandRunStatus::Queued,
            exit_code: None,
            output: None,
            queued_at: Utc::now(),
            started_at: None,
            finished_at: None,
        };
        store.enqueue_command(queued.clone()).await.expect("queued");
        assert!(
            store
                .claim_command(queued.id, Uuid::now_v7(), 1)
                .await
                .expect("claimed")
                .is_some(),
            "precondition: one claimed command run"
        );
        assert_eq!(
            store.read(|state| state.command_claims.len()),
            1,
            "precondition: its claim"
        );
        store.delete_project(gone).await.expect("the delete lands");

        store.read(|state| {
            assert!(!state.projects.contains_key(&gone), "the project itself");
            assert!(
                state
                    .permissions
                    .values()
                    .all(|p| state.runs.contains_key(&p.row.run_id))
                    && !state.permissions.contains_key(&staged),
                "step_permission goes with its run (MOD-42 plan D1)"
            );
            assert!(
                state
                    .run_commands
                    .values()
                    .all(|c| state.runs.contains_key(&c.run_id))
                    && state.run_commands.is_empty(),
                "run_command goes with its run (MOD-42 plan D1)"
            );
            assert!(
                state.follow_up_windows.is_empty() && state.follow_up_payloads.is_empty(),
                "follow_up_window and a follow-up's payload go with the run (MOD-70 plan D1)"
            );
            assert!(
                state
                    .command_claims
                    .keys()
                    .all(|id| state.command_runs.contains_key(id))
                    && state.command_claims.is_empty(),
                "a command_run's claim goes with its row (MOD-11 H-21)"
            );
            assert!(
                state.kinds.values().all(|row| row.project_id != gone),
                "item_kind"
            );
            assert!(
                state.graphs.values().all(|row| row.project_id != gone),
                "step_graph"
            );
            assert!(
                state.templates.iter().all(|row| row.project_id != gone),
                "prompt_template"
            );
            assert!(
                state
                    .skill_bindings
                    .iter()
                    .all(|row| row.project_id != Some(gone)),
                "skill_binding"
            );
            assert!(
                state.repos.values().all(|row| row.project_id != gone),
                "repo"
            );
            assert!(
                state.item_key_counter.keys().all(|(id, _)| *id != gone),
                "item_key_counter is keyed by (project, prefix) and goes with the project"
            );
            assert!(
                state.items.values().all(|row| row.project_id != gone),
                "item"
            );
            assert!(state.runs.values().all(|row| row.project_id != gone), "run");
            assert!(
                state
                    .workspace_projects
                    .iter()
                    .all(|row| row.project_id != gone),
                "workspace_project"
            );

            // Nothing that survived points at something that did not: the count could be right and
            // the cascade still leave a note on an item that is gone.
            assert!(
                state
                    .phases
                    .iter()
                    .all(|row| state.graphs.contains_key(&row.graph_id)),
                "every surviving phase has a surviving graph"
            );
            assert!(
                state
                    .revisions
                    .keys()
                    .all(|(item, _)| state.items.contains_key(item)),
                "every surviving revision has a surviving item"
            );
            assert!(
                state
                    .notes
                    .iter()
                    .all(|row| state.items.contains_key(&row.item_id)),
                "every surviving note has a surviving item"
            );
            assert!(
                state
                    .documents
                    .iter()
                    .all(|row| state.items.contains_key(&row.item_id)),
                "every surviving document has a surviving item"
            );
            assert!(
                state
                    .links
                    .iter()
                    .all(|row| state.items.contains_key(&row.from_item_id)
                        && state.items.contains_key(&row.to_item_id)),
                "every surviving link has both ends, tombstones included: this is what takes the \
                 fixture's cross-project edge from agy:FEAT-1 to htui:FEAT-2"
            );
            assert!(
                state
                    .steps
                    .values()
                    .all(|row| state.runs.contains_key(&row.run_id)),
                "every surviving step has a surviving run"
            );
            assert!(
                state
                    .events
                    .iter()
                    .all(|row| state.steps.contains_key(&row.run_step_id)),
                "every surviving event has a surviving step"
            );
            assert!(
                state
                    .step_trees
                    .keys()
                    .all(|(step, _)| state.steps.contains_key(step))
                    && state
                        .step_commits
                        .keys()
                        .all(|(step, _)| state.steps.contains_key(step)),
                "every surviving tree and commit has a surviving step (MOD-4's two new maps; the \
                 fixture seeds neither, so `trees_and_commits_upsert_on_their_repo_key_and_the_\
                 delete_counts_them` is where they are populated first)"
            );
            assert!(
                state
                    .repo_box_paths
                    .iter()
                    .all(|row| state.repos.contains_key(&row.repo_id)),
                "every surviving repo path has a surviving repo"
            );
            // MOD-38 blueprint §4.4: the six requirement tables go with the project too.
            assert!(
                !state.requirement_specs.contains_key(&gone)
                    && state
                        .requirement_areas
                        .values()
                        .all(|row| row.project_id != gone)
                    && state
                        .requirements
                        .values()
                        .all(|row| row.project_id != gone),
                "requirement_spec, requirement_area and requirement"
            );
            assert!(
                state
                    .requirement_key_counter
                    .keys()
                    .all(|area| state.requirement_areas.contains_key(area)),
                "every surviving requirement counter has a surviving area"
            );
            assert!(
                state
                    .requirement_revisions
                    .iter()
                    .all(|row| state.requirements.contains_key(&row.requirement_id)),
                "every surviving requirement revision has a surviving requirement"
            );
            assert!(
                state.item_requirements.iter().all(|row| {
                    state.items.contains_key(&row.item_id)
                        && state.requirements.contains_key(&row.requirement_id)
                }),
                "every surviving citation has both ends, tombstones included"
            );

            // …and what a project delete is not: the workspace, its sibling projects, and every
            // table `0001_init.sql` does not cascade from `project`.
            assert!(
                state.workspaces.contains_key(&ids::WORKSPACE_PLATFORM),
                "the workspace survives losing a project (D4)"
            );
            assert!(
                state.projects.contains_key(&ids::PROJECT_AGY)
                    && state.projects.contains_key(&ids::PROJECT_VULKAN),
                "sibling projects are untouched"
            );
            assert!(
                !state.skills.is_empty()
                    && !state.skill_versions.is_empty()
                    && !state.users.is_empty()
                    && !state.boxes.is_empty()
                    && !state.box_tools.is_empty()
                    && !state.agents.is_empty()
                    && !state.personas.is_empty(),
                "skill, skill_version, app_user, box, box_tool, agent and persona are not below a \
                 project"
            );
        });
    }

    /// MOD-38 blueprint §4.4: a revision in a surviving project that names a deleted item as its
    /// deciding item keeps the row and loses the reference, as `ON DELETE SET NULL` does on
    /// `requirement_revision.amended_by_item_id`; the deciding item's own citation goes with it.
    #[tokio::test]
    async fn delete_project_nulls_a_surviving_revisions_deciding_item() {
        let store = MemStore::demo();
        let area = store
            .create_requirement_area(NewRequirementArea {
                id: RequirementAreaId::new(),
                project_id: ids::PROJECT_AGY,
                code: "API".to_owned(),
                title: "API".to_owned(),
                description: String::new(),
                position: 0,
            })
            .await
            .expect("agy takes an area");
        let minted = store
            .mint_requirement(
                area.id,
                NewRequirement {
                    id: RequirementId::new(),
                    body: "Decided in another project.".to_owned(),
                    rationale: String::new(),
                    priority: Priority::Must,
                    created_by: ids::USER,
                    box_id: None,
                },
            )
            .await
            .expect("the mint lands");
        let amended = store
            .amend_requirement(
                minted.id,
                1,
                RequirementPatch {
                    body: Some("Amended by htui:FEAT-3.".to_owned()),
                    author_id: ids::USER,
                    reason: "amended".to_owned(),
                    ..RequirementPatch::default()
                },
                ids::HTUI_FEAT_3,
            )
            .await
            .expect("the amend lands");
        assert!(
            matches!(amended, RequirementUpdate::Updated(ref row) if row.version == 2),
            "precondition: {amended:?}"
        );

        let report = store
            .delete_project(ids::PROJECT_HTUI)
            .await
            .expect("the delete lands");
        assert_eq!(
            report.item_requirements, 6,
            "the fixture's five citations plus FEAT-3's `amends` of the agy requirement"
        );

        let revisions = store
            .requirement_revisions(minted.id)
            .await
            .expect("a read")
            .expect("MemStore keeps revisions");
        assert_eq!(
            revisions
                .iter()
                .map(|row| (row.version, row.amended_by_item_id))
                .collect::<Vec<_>>(),
            vec![(1, None), (2, None)],
            "revision 2 survives, and no longer names the deleted item"
        );
        assert_eq!(
            store
                .requirement(minted.id)
                .await
                .expect("a read")
                .map(|row| row.version),
            Some(2),
            "the agy requirement itself is untouched"
        );
        assert_eq!(
            store.requirement_coverage(minted.id).await.expect("a read"),
            Vec::new(),
            "the deleted item's citation went with it"
        );
    }

    /// MOD-38: a citation between two surviving rows keeps itself and loses a proposing step of
    /// the deleted project, as `ON DELETE SET NULL` does on
    /// `item_requirement.proposed_by_step_id`, whose UPDATE also moves `updated_at`.
    #[tokio::test]
    async fn delete_project_nulls_a_surviving_citations_proposing_step() {
        let store = MemStore::demo();
        let area = store
            .create_requirement_area(NewRequirementArea {
                id: RequirementAreaId::new(),
                project_id: ids::PROJECT_AGY,
                code: "API".to_owned(),
                title: "API".to_owned(),
                description: String::new(),
                position: 0,
            })
            .await
            .expect("agy takes an area");
        let minted = store
            .mint_requirement(
                area.id,
                NewRequirement {
                    id: RequirementId::new(),
                    body: "Proposed by an htui step.".to_owned(),
                    rationale: String::new(),
                    priority: Priority::Must,
                    created_by: ids::USER,
                    box_id: None,
                },
            )
            .await
            .expect("the mint lands");
        let cited = store
            .cite(
                ids::AGY_FEAT_1,
                minted.id,
                CitationKind::Addresses,
                Some(ids::STEP_IMPL),
            )
            .await
            .expect("the cite lands");
        assert_eq!(
            cited.proposed_by_step_id,
            Some(ids::STEP_IMPL),
            "precondition"
        );

        store
            .delete_project(ids::PROJECT_HTUI)
            .await
            .expect("the delete lands");

        let row = store
            .write(|state| {
                state
                    .item_requirements
                    .iter()
                    .find(|row| row.item_id == ids::AGY_FEAT_1 && row.requirement_id == minted.id)
                    .cloned()
            })
            .expect("the agy citation survives");
        assert_eq!(
            row.proposed_by_step_id, None,
            "the citation no longer names the deleted step"
        );
        assert!(
            row.updated_at > cited.updated_at,
            "the SET NULL moved updated_at, as the Postgres trigger does"
        );
    }

    /// A fresh project, authored by the fixture user, with the slug the test names.
    fn fresh_project(slug: &str) -> NewProject {
        NewProject {
            id: ProjectId::new(),
            slug: slug.to_owned(),
            name: slug.to_uppercase(),
            description: String::new(),
            created_by: ids::USER,
        }
    }

    /// The template *content* the seed writes, which no `WriteStore` reader can return
    /// (M1 D9: MOD-9 owns the editor). `project_create_seeds_the_catalogue` counts ten
    /// through `delete_reach`; this reads them through the inherent `prompt_templates`.
    #[tokio::test]
    async fn seeded_templates_carry_the_shipped_bodies() {
        let store = MemStore::demo();
        let project = store
            .create_project(fresh_project("seeded"))
            .await
            .expect("the create lands");

        let rows = store
            .prompt_templates(project.id)
            .await
            .expect("the inherent reader answers");
        let mut expected: Vec<&str> = DEFAULT_TEMPLATES.iter().map(|(name, ..)| *name).collect();
        expected.sort_unstable();
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            expected,
            "ten rows, one per default template, in the reader's name-byte order"
        );
        for row in &rows {
            assert_eq!(
                Some(row.body.as_str()),
                body_of(&row.name),
                "`{}` body",
                row.name
            );
            assert_eq!(row.version, 1, "`{}` is version 1", row.name);
            assert_eq!(row.created_by, ids::USER, "`{}` is the creator's", row.name);
            assert_eq!(row.project_id, project.id);
        }
    }

    /// D5: the seed writes no `item_key_counter` row; `mint` creates it on first use.
    #[tokio::test]
    async fn seed_never_writes_a_counter_row() {
        let store = MemStore::demo();
        let project = store
            .create_project(fresh_project("lazy"))
            .await
            .expect("the create lands");
        let counter = |prefix: &str| {
            store.read(|state| {
                state
                    .item_key_counter
                    .get(&(project.id, prefix.to_owned()))
                    .copied()
            })
        };
        assert!(
            store.read(|state| state
                .item_key_counter
                .keys()
                .all(|(id, _)| *id != project.id)),
            "no counter row of any prefix after the create"
        );

        let feat = store
            .item_kinds(project.id)
            .await
            .expect("kinds read")
            .into_iter()
            .find(|kind| kind.prefix == "FEAT")
            .expect("the seeded FEAT kind");
        let minted = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: project.id,
                kind_id: feat.id,
                title: "first".to_owned(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the first mint lands");
        assert_eq!(minted.key, "FEAT-1");
        assert_eq!(
            counter("FEAT"),
            Some(1),
            "the row exists only after the mint"
        );
        assert_eq!(counter("ANA"), None, "and only for the prefix that minted");
    }

    /// PRD D12's third fact: the counter row of the **old** prefix survives a rename.
    /// `item_kind_round_trip_and_prefix_rules` pins the other two (old key text kept, `ANL-1`
    /// next) on both stores; no trait reader sees `item_key_counter`, so this one is per backend.
    #[tokio::test]
    async fn renamed_prefix_leaves_the_old_counter_row() {
        let store = MemStore::demo();
        let project = ids::PROJECT_HTUI;
        let counter = |prefix: &str| {
            store.read(|state| {
                state
                    .item_key_counter
                    .get(&(project, prefix.to_owned()))
                    .copied()
            })
        };
        assert_eq!(
            counter("ANA"),
            Some(2),
            "the fixture minted ANA-1 and ANA-2"
        );

        let ana = store
            .item_kinds(project)
            .await
            .expect("kinds read")
            .into_iter()
            .find(|kind| kind.id == ids::KIND_HTUI_ANA)
            .expect("the fixture kind");
        let renamed = store
            .update_item_kind(
                ana.id,
                ana.updated_at,
                ItemKindPatch {
                    prefix: Some("ANL".to_owned()),
                    ..ItemKindPatch::default()
                },
            )
            .await
            .expect("the rename lands");
        assert!(matches!(renamed, CasOutcome::Applied(_)));
        assert_eq!(
            counter("ANA"),
            Some(2),
            "the old row is history, not garbage"
        );
        assert_eq!(
            counter("ANL"),
            None,
            "nothing minted under the new prefix yet"
        );

        let minted = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: project,
                kind_id: ids::KIND_HTUI_ANA,
                title: "after the rename".to_owned(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands");
        assert_eq!(minted.key, "ANL-1");
        assert_eq!(counter("ANL"), Some(1));
        assert_eq!(counter("ANA"), Some(2), "still");
    }

    // ---- MOD-4 milestone 1: the run seam (blueprint §2.8, §2.9) -------------------------------
    //
    // The fourteen conformance cases are the next commit's and pin these rules on every backend.
    // What follows is the narrowest per-method coverage this commit needs: the outcome, the
    // refusal and — for the five transactions of plan D6 — that a refusal leaves no half-written
    // row behind.

    /// A snapshot with no phases: enough to prove the column is written and decodes at `v = 1`.
    fn test_snapshot() -> GraphSnapshot {
        GraphSnapshot {
            v: GraphSnapshot::V,
            graph: SnapshotGraph {
                id: ids::GRAPH_HTUI_FEAT,
                name: "FEAT".to_owned(),
                is_override: false,
            },
            topology: "sha256:test".to_owned(),
            mode: RunMode::Manual,
            phases: Vec::new(),
            settings: SnapshotSettings {
                default_isolation: Isolation::Worktree,
                per_token_cap_run: None,
                per_token_cap_batch: None,
                max_fan_out: 4,
                max_agents_per_run: 6,
            },
            scope: None,
            personas: Vec::new(),
        }
    }

    fn graph_run(item: ItemId, project: ProjectId, scope: Vec<RepoId>) -> NewRun {
        NewRun {
            id: RunId::new(),
            project_id: project,
            item_id: item,
            mode: RunMode::Manual,
            target_box_id: ids::BOX,
            started_by: ids::USER,
            graph_snapshot: test_snapshot(),
            repo_scope: scope,
            queued_at: Utc::now(),
            batch_id: None,
        }
    }

    fn new_step(run: RunId, position: i32, attempt: i32, fanout_index: i32) -> NewRunStep {
        NewRunStep {
            id: StepId::new(),
            run_id: run,
            position,
            attempt,
            fanout_index,
            phase_name: "implement".to_owned(),
            agent_id: Some(ids::AGENT_CLAUDE),
            model: Some("opus".to_owned()),
        }
    }

    async fn a_repo(store: &MemStore, name: &str) -> RepoId {
        store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: name.to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: false,
            })
            .await
            .expect("the repo lands")
            .id
    }

    fn a_document(item: ItemId, kind: &str, step: Option<StepId>) -> NewDocument {
        NewDocument {
            id: DocumentId::new(),
            item_id: item,
            kind: kind.to_owned(),
            title: format!("{kind} of {item}"),
            body: String::new(),
            produced_by_step_id: step,
            created_by: ids::USER,
            created_at: Utc::now(),
        }
    }

    /// Drives a fresh step of `run` to `awaiting_approval` through the two legal moves.
    async fn gated_step(store: &MemStore, spec: NewRunStep) -> StepId {
        let id = spec.id;
        store.create_step(spec).await.expect("the step lands");
        let now = Utc::now();
        store
            .transition_step(id, StepStatus::Pending, StepStatus::Running, now)
            .await
            .expect("pending -> running is legal");
        store
            .transition_step(id, StepStatus::Running, StepStatus::AwaitingApproval, now)
            .await
            .expect("running -> awaiting_approval is legal");
        id
    }

    /// `create_run` is one transaction (plan D6): the `run` row and the item's move to `queued`
    /// land together, and an item the §4.3 law cannot move leaves no run row behind.
    #[tokio::test]
    async fn create_run_moves_the_item_and_writes_nothing_when_the_law_refuses() {
        let store = MemStore::demo();
        let new = graph_run(ids::HTUI_ANA_2, ids::PROJECT_HTUI, Vec::new());
        let id = new.id;
        let row = store
            .create_run(new)
            .await
            .expect("open -> queued is legal");

        assert_eq!(row.status, RunStatus::Queued, "a new run starts queued");
        assert_eq!(row.kind, RunKind::Graph, "create_run mints graph runs");
        assert_eq!(row.item_id, Some(ids::HTUI_ANA_2));
        assert_eq!(row.executing_box_id, None, "nothing has claimed it yet");
        assert_eq!(row.lease_box_id, None);
        assert_eq!(row.lease_expires_at, None);
        let snapshot: GraphSnapshot =
            serde_json::from_value(row.graph_snapshot.clone().expect("the snapshot is written"))
                .expect("it decodes as the typed form the caller passed");
        assert_eq!(snapshot.v, GraphSnapshot::V);
        assert_eq!(
            store.run(id).await.expect("the run reads back"),
            Some(row),
            "`run` answers the row `create_run` returned"
        );

        let item = store
            .item(ids::HTUI_ANA_2)
            .await
            .expect("the item reads back")
            .expect("the fixture item");
        assert_eq!(item.status, Status::Queued, "the item moved with the run");

        let blocked = graph_run(ids::HTUI_FEAT_1, ids::PROJECT_HTUI, Vec::new());
        let blocked_id = blocked.id;
        assert!(
            matches!(
                store.create_run(blocked).await,
                Err(StoreError::Constraint(_))
            ),
            "in_progress cannot move to queued (ANA-2 §4.3)"
        );
        assert_eq!(
            store.run(blocked_id).await.expect("the read is total"),
            None,
            "the refusal wrote no run row: create_run is one transaction"
        );

        let missing = graph_run(ItemId::new(), ids::PROJECT_HTUI, Vec::new());
        assert!(
            matches!(
                store.create_run(missing).await,
                Err(StoreError::NotFound { entity: "item", .. })
            ),
            "an unknown item is NotFound before the law is asked (plan D14)"
        );
    }

    /// The demo store with the fixture box's `box.settings` replaced by `settings`.
    fn demo_with_box_settings(settings: Value) -> MemStore {
        let mut data = crate::fixtures::demo_data();
        data.boxes
            .iter_mut()
            .find(|row| row.id == ids::BOX)
            .expect("the fixture box")
            .settings = settings;
        MemStore::from_demo(data)
    }

    /// MOD-41 plan D10: an executor edit writes that key alone; every other key of the blob,
    /// known or not, survives. The Postgres half is
    /// `pg_criteria.rs::edit_box_keeps_every_other_settings_key`.
    #[tokio::test]
    async fn edit_box_keeps_every_other_settings_key_in_memory() {
        use crate::model::{BoxEdit, Executor};

        let store = demo_with_box_settings(json!({
            "max_concurrent_items": 1,
            "command_limits": {"verify": 2},
            "x": true,
        }));
        let token = store
            .box_row(ids::BOX)
            .await
            .expect("box_row never fails")
            .expect("the fixture box")
            .edit_version;
        let written = store
            .edit_box(
                ids::BOX,
                token,
                BoxEdit {
                    executor: Some(Executor::Worker),
                    ..BoxEdit::default()
                },
            )
            .await
            .expect("the edit is answered");
        let CasOutcome::Applied(row) = written else {
            panic!("an executor edit on the current token applies, got {written:?}");
        };
        assert_eq!(
            row.settings,
            json!({
                "max_concurrent_items": 1,
                "command_limits": {"verify": 2},
                "x": true,
                "executor": "worker",
            }),
            "all four keys are present"
        );
        assert_eq!(row.edit_version, token + 1, "the edit moves the token");
    }

    /// MOD-41 plan D10: a `box.settings` blob that is not a JSON object cannot take the executor
    /// key. The edit is `Constraint(BOX_SETTINGS_NOT_AN_OBJECT)` and writes nothing, the token
    /// included; an edit that does not name the executor still applies to the same row. The
    /// Postgres half is `pg_criteria.rs::edit_box_refuses_a_non_object_settings_blob`.
    #[tokio::test]
    async fn edit_box_refuses_a_non_object_settings_blob_in_memory() {
        use crate::model::{BoxEdit, Executor};
        use crate::store::traits::BOX_SETTINGS_NOT_AN_OBJECT;

        for blob in [json!([]), json!("x")] {
            let store = demo_with_box_settings(blob.clone());
            let before = store
                .box_row(ids::BOX)
                .await
                .expect("box_row never fails")
                .expect("the fixture box");
            let refused = store
                .edit_box(
                    ids::BOX,
                    before.edit_version,
                    BoxEdit {
                        executor: Some(Executor::Tui),
                        ..BoxEdit::default()
                    },
                )
                .await;
            match refused {
                Err(StoreError::Constraint(said)) => {
                    assert_eq!(said, BOX_SETTINGS_NOT_AN_OBJECT, "{blob}");
                }
                other => panic!("{blob}: a non-object blob is a Constraint, got {other:?}"),
            }
            assert_eq!(
                store.box_row(ids::BOX).await.expect("box_row never fails"),
                Some(before.clone()),
                "{blob}: the refused edit wrote nothing, the token included"
            );

            let quirks = store
                .edit_box(
                    ids::BOX,
                    before.edit_version,
                    BoxEdit {
                        quirks: Some("no executor named".to_owned()),
                        ..BoxEdit::default()
                    },
                )
                .await
                .expect("the edit is answered");
            let CasOutcome::Applied(row) = quirks else {
                panic!("{blob}: a quirks-only edit applies, got {quirks:?}");
            };
            assert_eq!(row.quirks, "no executor named", "{blob}");
            assert_eq!(row.settings, blob, "{blob}: the blob is left as it was");

            // Precedence (trait doc): `NotFound`, then `Stale`, before the blob's `Constraint`.
            let tui = || BoxEdit {
                executor: Some(Executor::Tui),
                ..BoxEdit::default()
            };
            assert_eq!(
                store.edit_box(ids::BOX, before.edit_version, tui()).await,
                Ok(CasOutcome::Stale(row.clone())),
                "{blob}: a spent token is Stale before the non-object blob refuses"
            );
            let unknown = BoxId::new();
            match store.edit_box(unknown, row.edit_version, tui()).await {
                Err(StoreError::NotFound { entity: "box", id }) => {
                    assert_eq!(id, unknown.to_string(), "{blob}");
                }
                other => panic!("{blob}: an unknown box is NotFound, got {other:?}"),
            }
            assert_eq!(
                store.box_row(ids::BOX).await.expect("box_row never fails"),
                Some(row),
                "{blob}: neither refusal wrote anything"
            );
        }
    }

    /// MOD-41 plan D11: `queued_runs_on_box` answers this box's `queued` runs alone, by
    /// `(queued_at, id)`: not another box's queued run, not this box's running one. The Postgres
    /// half is `pg_criteria.rs::queued_runs_on_box_lists_this_boxs_queued_runs_in_queue_order`.
    #[tokio::test]
    async fn queued_runs_on_box_lists_this_boxs_queued_runs_in_queue_order() {
        let mut data = crate::fixtures::demo_data();
        let mut elsewhere = data
            .boxes
            .iter()
            .find(|row| row.id == ids::BOX)
            .expect("the fixture box")
            .clone();
        let other = BoxId::new();
        elsewhere.id = other;
        elsewhere.hostname = "ELSEWHERE".to_owned();
        data.boxes.push(elsewhere);
        let store = MemStore::from_demo(data);

        // The fixture already queues a run on this box, stamped at the fixture's clock: it leads.
        let fixture = store
            .queued_runs_on_box(ids::BOX)
            .await
            .expect("the read is answered");
        // Truncated as the store keeps it, since `expected` compares it with the read-back.
        let early = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
        assert!(
            fixture.iter().all(|(_, at)| *at < early),
            "the fixture's queued runs predate this case's, got {fixture:?}"
        );
        let late = early + TimeDelta::seconds(1);
        let queue = |id: RunId, title: &'static str, target: BoxId, queued_at| {
            let store = &store;
            async move {
                let item = store
                    .mint_item(NewItem {
                        id: ItemId::new(),
                        project_id: ids::PROJECT_HTUI,
                        kind_id: ids::KIND_HTUI_FEAT,
                        title: title.to_owned(),
                        body: String::new(),
                        required_tags: Vec::new(),
                        touched_paths: Vec::new(),
                        priority: 0,
                        step_graph_id: None,
                        created_by: ids::USER,
                        box_id: Some(ids::BOX),
                    })
                    .await
                    .expect("the mint lands")
                    .id;
                store
                    .create_run(NewRun {
                        id,
                        target_box_id: target,
                        queued_at,
                        ..graph_run(item, ids::PROJECT_HTUI, Vec::new())
                    })
                    .await
                    .expect("the run is queued")
                    .id
            }
        };
        // The tied pair is inserted larger id first, so insertion order disagrees with `id` order
        // and only the `id` tie-break puts the smaller one first.
        let (first, second) = {
            let (a, b) = (RunId::new(), RunId::new());
            if a < b { (a, b) } else { (b, a) }
        };
        let running = queue(RunId::new(), "running on this box", ids::BOX, early).await;
        let last = queue(RunId::new(), "queued last", ids::BOX, late).await;
        queue(second, "queued first, tied, larger id", ids::BOX, early).await;
        queue(first, "queued first, tied, smaller id", ids::BOX, early).await;
        let _theirs = queue(RunId::new(), "queued on another box", other, early).await;
        assert_eq!(
            store
                .claim_run(
                    running,
                    ids::BOX,
                    Uuid::now_v7(),
                    early,
                    TimeDelta::minutes(5)
                )
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "one run on this box is running, not queued"
        );

        let mut expected = fixture;
        expected.extend([(first, early), (second, early), (last, late)]);
        assert_eq!(
            store
                .queued_runs_on_box(ids::BOX)
                .await
                .expect("the read is answered"),
            expected,
            "this box's queued runs, the fixture's then this case's three, by `(queued_at, id)`"
        );
    }

    /// ANA-2 §4.7's admission: the repo-scope overlap refuses before the slot count does, and the
    /// slot count is `box.settings.max_concurrent_items`.
    #[tokio::test]
    async fn claim_run_refuses_an_overlapping_scope_and_a_full_box() {
        // MOD-40 plan D10: the lease is this handle's clock plus the TTL, so the case clocks its
        // handle and reads `at` from it (truncated, as every clock is).
        let clock = TestClock::at(Utc::now());
        let at = clock.now();
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        let repo = a_repo(&store, "core").await;
        let owner = Uuid::now_v7();
        let ttl = TimeDelta::minutes(5);
        let until = at + ttl;

        let queue = |item, project, scope: Vec<RepoId>| {
            let store = &store;
            async move {
                store
                    .create_run(graph_run(item, project, scope))
                    .await
                    .expect("the run is queued")
                    .id
            }
        };
        let first = queue(ids::HTUI_ANA_2, ids::PROJECT_HTUI, vec![repo]).await;
        let second = queue(ids::HTUI_CLEAN_1, ids::PROJECT_HTUI, vec![repo]).await;
        let third = queue(ids::AGY_FEAT_1, ids::PROJECT_AGY, Vec::new()).await;
        let fourth = queue(ids::AGY_FIX_1, ids::PROJECT_AGY, Vec::new()).await;

        assert_eq!(
            store
                .claim_run(first, ids::BOX, owner, at, ttl)
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "the first run is admitted"
        );
        let claimed = store
            .run(first)
            .await
            .expect("the run reads back")
            .expect("it exists");
        assert_eq!(claimed.status, RunStatus::Running);
        assert_eq!(claimed.executing_box_id, Some(ids::BOX));
        assert_eq!(claimed.started_at, Some(at), "`at` is the caller's clock");
        assert_eq!(claimed.lease_box_id, Some(ids::BOX));
        assert_eq!(claimed.lease_expires_at, Some(until));
        assert_eq!(
            store
                .item(ids::HTUI_ANA_2)
                .await
                .expect("the item reads back")
                .expect("it exists")
                .status,
            Status::InProgress,
            "the item moved queued -> in_progress with the claim"
        );

        assert_eq!(
            store
                .claim_run(second, ids::BOX, owner, at, ttl)
                .await
                .expect("the claim is answered"),
            Claim::Overlaps {
                with: first,
                rule: OverlapRule::NotIsolated
            },
            "an overlapping repo_scope is refused while one slot is still free"
        );
        assert_eq!(
            store
                .run(second)
                .await
                .expect("the run reads back")
                .expect("it exists")
                .status,
            RunStatus::Queued,
            "a refused claim writes nothing"
        );

        assert_eq!(
            store
                .claim_run(third, ids::BOX, owner, at, ttl)
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "an empty scope overlaps nothing"
        );
        assert_eq!(
            store
                .claim_run(fourth, ids::BOX, owner, at, ttl)
                .await
                .expect("the claim is answered"),
            Claim::SlotFull {
                running: 2,
                limit: 2
            },
            "`max_concurrent_items` is 2 on the fixture box"
        );
        assert_eq!(
            store
                .claim_run(first, ids::BOX, owner, at, ttl)
                .await
                .expect("the claim is answered"),
            Claim::NotClaimable,
            "a run that is not queued is not claimable"
        );

        assert!(
            matches!(
                store.claim_run(second, BoxId::new(), owner, at, ttl).await,
                Err(StoreError::NotFound { entity: "box", .. })
            ),
            "the box is looked up after the run"
        );
        assert!(
            matches!(
                store
                    .claim_run(RunId::new(), BoxId::new(), owner, at, ttl)
                    .await,
                Err(StoreError::NotFound { entity: "run", .. })
            ),
            "the run is looked up first"
        );
        assert_eq!(
            store
                .active_runs_on_box(ids::BOX)
                .await
                .expect("the count is answered"),
            2,
            "two runs hold the box"
        );
    }

    /// R-ORCH-10's claim-time check (MOD-7 milestone 3, D80, D81) and MOD-58 plan D3: a run aimed
    /// at another box is `NotClaimable` even when its item needs a tag the claiming box lacks, and
    /// the refusal writes nothing. Claimability is decided before the tag check, so an armed tag
    /// rule never gets a turn.
    #[tokio::test]
    async fn a_missing_tags_run_aimed_at_another_box_is_not_claimable() {
        let store = MemStore::demo();
        // A real second box row, because `create_run` looks `target_box_id` up in `state.boxes`
        // and refuses a run whose box is missing. A clone of the fixture's, so it carries the
        // probed and declared tags a real registration leaves behind, and a hostname of its own.
        // Neither of those is load-bearing: the rule reads the *claiming* box's tags, and that is
        // `ids::BOX` either way.
        let other = BoxId::new();
        let mut elsewhere = store
            .box_row(ids::BOX)
            .await
            .expect("MemStore never fails a read")
            .expect("the fixture box");
        elsewhere.id = other;
        elsewhere.hostname = "elsewhere".to_owned();
        store.write(|state| {
            state.boxes.insert(other, elsewhere);
        });

        let item = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: "needs a GPU toolchain".to_owned(),
                body: String::new(),
                required_tags: vec!["cuda".to_owned()],
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands")
            .id;
        assert_eq!(
            store
                .missing_tags(item, ids::BOX)
                .await
                .expect("the tags read back"),
            vec!["cuda".to_owned()],
            "the tag rule is armed: the claiming box probes rust/msvc/cmake and declares gpu"
        );
        let run = store
            .create_run(NewRun {
                target_box_id: other,
                ..graph_run(item, ids::PROJECT_HTUI, Vec::new())
            })
            .await
            .expect("the run is queued");
        assert_eq!(
            run.target_box_id, other,
            "the run is aimed at the second box, so this cannot pass by aiming at `ids::BOX`"
        );

        let at = Utc::now();
        let owner = Uuid::now_v7();
        assert_eq!(
            store
                .claim_run(run.id, ids::BOX, owner, at, TimeDelta::minutes(5))
                .await
                .expect("the claim is answered"),
            Claim::NotClaimable,
            "a run aimed at another box is not claimable, and that outranks the tag rule"
        );
        assert_eq!(
            store.run(run.id).await.expect("the run reads back"),
            Some(run.clone()),
            "the refusal wrote nothing to the run: not even the `MissingTags` failure row"
        );
        // `lease_owner` is not a `Run` field on this store, so the row comparison above cannot
        // see it: the refused branch has to be pinned through the CAS it would have gone through.
        assert!(
            !store
                .refresh_lease(run.id, owner, TimeDelta::minutes(5))
                .await
                .expect("the lease refresh is answered"),
            "the refusal took no lease"
        );
        assert_eq!(
            store
                .item(item)
                .await
                .expect("the item reads back")
                .expect("it exists")
                .status,
            Status::Queued,
            "the refusal wrote nothing to the item either: it is not blocked"
        );
    }

    /// Blueprint D94, both of its arms: a run with no item — `item_id` is `NULL`, or the row it
    /// names is gone — has no tags to check, so the tag check is never reached and the claim is
    /// admitted. `claim_run` asks `items.get`, not `require_item`, mirroring the Postgres join that
    /// finds no row.
    ///
    /// Both arms are defensive pins on shapes the store cannot currently mint: `NewRun.item_id` is
    /// a non-optional `ItemId` and there is no `delete_item` on the store, so no public call
    /// reaches either one. A chat run is not a vehicle either — `start_chat_run` inserts at
    /// `RunStatus::Running`, which is `NotClaimable` at the status half of the rule, so it never
    /// reaches the tag query. What the case is for is a future writer that made `item_id`
    /// nullable, or added a delete path.
    ///
    /// It leans on the fixture box's `max_concurrent_items: 2`: the first arm's run is still
    /// `running` on `ids::BOX` when the second claims, so only the second slot admits it.
    #[tokio::test]
    async fn a_run_with_no_item_is_never_refused_for_tags() {
        // MOD-40 plan D10: the lease is this handle's clock plus the TTL.
        let clock = TestClock::at(Utc::now());
        let at = clock.now();
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        let ttl = TimeDelta::minutes(5);
        let until = at + ttl;

        let run = store
            .create_run(graph_run(ids::HTUI_ANA_2, ids::PROJECT_HTUI, Vec::new()))
            .await
            .expect("the run is queued")
            .id;
        store.write(|state| {
            state.runs.get_mut(&run).expect("the run exists").item_id = None;
        });
        assert_eq!(
            store
                .claim_run(run, ids::BOX, Uuid::now_v7(), at, ttl)
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "no item, no tags, no refusal: the tag check is skipped"
        );
        let claimed = store
            .run(run)
            .await
            .expect("the run reads back")
            .expect("it exists");
        assert_eq!(claimed.status, RunStatus::Running);
        assert_eq!(claimed.executing_box_id, Some(ids::BOX));
        assert_eq!(
            claimed.lease_expires_at,
            Some(until),
            "the admitted claim wrote the lease its TTL asked for"
        );

        // A second item of its own, so the two arms share no state.
        let orphan = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: "its item is gone".to_owned(),
                body: String::new(),
                required_tags: vec!["cuda".to_owned()],
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands")
            .id;
        let orphaned = store
            .create_run(graph_run(orphan, ids::PROJECT_HTUI, Vec::new()))
            .await
            .expect("the run is queued")
            .id;
        store.write(|state| {
            state.items.remove(&orphan);
        });
        assert_eq!(
            store
                .claim_run(orphaned, ids::BOX, Uuid::now_v7(), at, ttl)
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "an item that is gone leaves no tags to refuse on; `items.get` is not `require_item`"
        );
        let claimed = store
            .run(orphaned)
            .await
            .expect("the run reads back")
            .expect("it exists");
        assert_eq!(claimed.status, RunStatus::Running);
        assert_eq!(claimed.executing_box_id, Some(ids::BOX));
    }

    /// ANA-2 §4.9: the heartbeat is a compare-and-set on `lease_owner`, and the sweep takes an
    /// expired lease from whoever held it.
    #[tokio::test]
    async fn a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it() {
        // MOD-40 plan D10: a lease is this handle's clock plus a TTL, so the case moves the
        // clock where it used to pass an instant, and asks for the TTL that lands on the old
        // expiry; every expiry below is the exact instant it was.
        let clock = TestClock::at(Utc::now());
        let at = clock.now();
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        let first_owner = Uuid::now_v7();
        let second_owner = Uuid::now_v7();
        let until = at + TimeDelta::minutes(5);

        let run = store
            .create_run(graph_run(ids::HTUI_ANA_2, ids::PROJECT_HTUI, Vec::new()))
            .await
            .expect("the run is queued")
            .id;
        assert_eq!(
            store
                .claim_run(run, ids::BOX, first_owner, at, until - at)
                .await
                .expect("the claim is answered"),
            Claim::Admitted
        );

        let extended = until + TimeDelta::minutes(5);
        assert!(
            store
                .refresh_lease(run, first_owner, extended - clock.now())
                .await
                .expect("the refresh is answered"),
            "the owner extends its own lease"
        );
        assert_eq!(
            store
                .run(run)
                .await
                .expect("the run reads back")
                .expect("it exists")
                .lease_expires_at,
            Some(extended)
        );
        assert!(
            !store
                .refresh_lease(
                    run,
                    second_owner,
                    extended + TimeDelta::minutes(5) - clock.now()
                )
                .await
                .expect("the refresh is answered"),
            "a stranger's heartbeat is zero rows, which means abandon"
        );
        assert!(
            matches!(
                store
                    .refresh_lease(RunId::new(), first_owner, extended - clock.now())
                    .await,
                Err(StoreError::NotFound { entity: "run", .. })
            ),
            "an unknown run is NotFound"
        );

        let swept = extended + TimeDelta::minutes(5);
        clock.set(extended - TimeDelta::seconds(1));
        assert!(
            store
                .adopt_runs(ids::BOX, second_owner, swept - clock.now())
                .await
                .expect("the sweep is answered")
                .is_empty(),
            "a live lease is not abandoned"
        );
        clock.set(extended + TimeDelta::seconds(1));
        let adopted = store
            .adopt_runs(ids::BOX, second_owner, swept - clock.now())
            .await
            .expect("the sweep is answered");
        assert_eq!(
            adopted.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![run],
            "the expired lease is adopted"
        );
        assert_eq!(adopted[0].lease_expires_at, Some(swept));
        assert!(
            !store
                .refresh_lease(run, first_owner, swept - clock.now())
                .await
                .expect("the refresh is answered"),
            "the old owner has lost it"
        );
        assert!(
            store
                .refresh_lease(run, second_owner, swept - clock.now())
                .await
                .expect("the refresh is answered"),
            "the new owner holds it"
        );
        let later = swept + TimeDelta::minutes(5);
        clock.set(swept + TimeDelta::seconds(1));
        assert!(
            store
                .adopt_runs(ids::BOX, second_owner, later - clock.now())
                .await
                .expect("the sweep is answered")
                .is_empty(),
            "a process never adopts its own lease, even expired (plan D88)"
        );
        assert_eq!(
            store
                .adopt_runs(ids::BOX, first_owner, later - clock.now())
                .await
                .expect("the sweep is answered")
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![run],
            "but a stranger's sweep adopts it"
        );
        assert!(
            store
                .adopt_runs(BoxId::new(), second_owner, TimeDelta::zero())
                .await
                .expect("the sweep is answered")
                .is_empty(),
            "a box that does not exist adopts nothing"
        );
    }

    /// `create_step`, the two compare-and-sets and the order `run_steps` returns: the judge
    /// (`fanout_index = -1`) sorts before its candidates.
    #[tokio::test]
    async fn step_creation_and_the_two_compare_and_sets_follow_the_law() {
        let store = MemStore::demo();
        let candidate = new_step(ids::RUN_2, 1, 1, 0);
        let step = candidate.id;
        let row = store
            .create_step(candidate)
            .await
            .expect("a step of an existing run lands");
        assert_eq!(row.status, StepStatus::Pending, "a step starts pending");
        assert_eq!(row.started_at, None);
        assert_eq!(row.selected, None);
        assert_eq!(row.promoted_at, None);

        assert!(
            matches!(
                store.create_step(new_step(ids::RUN_2, 1, 1, 0)).await,
                Err(StoreError::Constraint(_))
            ),
            "`(run_id, position, attempt, fanout_index)` is unique"
        );
        assert!(
            matches!(
                store.create_step(new_step(RunId::new(), 0, 1, 0)).await,
                Err(StoreError::Constraint(_))
            ),
            "an unknown run is a foreign key refusal, like append_events"
        );

        let judge = new_step(ids::RUN_2, 1, 1, -1);
        let judge_id = judge.id;
        store
            .create_step(judge)
            .await
            .expect("-1 is the judge step");
        let ids_in_order: Vec<StepId> = store
            .run_steps(ids::RUN_2)
            .await
            .expect("the steps read back")
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(
            ids_in_order,
            vec![ids::STEP_R2_PRD, judge_id, step],
            "(position, attempt, fanout_index) order puts the judge first"
        );
        assert!(
            store
                .run_steps(RunId::new())
                .await
                .expect("a list read is total")
                .is_empty(),
            "an unknown run has no steps, and is not NotFound"
        );

        let started = Utc::now();
        let finished = started + TimeDelta::minutes(1);
        assert!(
            store
                .transition_step(step, StepStatus::Pending, StepStatus::Running, started)
                .await
                .expect("the CAS is answered")
        );
        assert!(
            !store
                .transition_step(step, StepStatus::Pending, StepStatus::Running, started)
                .await
                .expect("the CAS is answered"),
            "a stale `from` on a legal pair is Ok(false)"
        );
        assert!(
            store
                .transition_step(step, StepStatus::Running, StepStatus::Done, finished)
                .await
                .expect("the CAS is answered")
        );
        let settled = store
            .run_steps(ids::RUN_2)
            .await
            .expect("the steps read back")
            .into_iter()
            .find(|row| row.id == step)
            .expect("the step");
        assert_eq!(settled.started_at, Some(started));
        assert_eq!(settled.finished_at, Some(finished));
        assert!(
            matches!(
                store
                    .transition_step(step, StepStatus::Done, StepStatus::Running, finished)
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "done reaches only superseded"
        );
        assert!(
            matches!(
                store
                    .transition_step(
                        StepId::new(),
                        StepStatus::Done,
                        StepStatus::Running,
                        finished
                    )
                    .await,
                Err(StoreError::NotFound {
                    entity: "run_step",
                    ..
                })
            ),
            "a missing row is NotFound even when the pair is illegal (plan D14)"
        );

        assert!(
            store
                .transition_run(ids::RUN_2, RunStatus::Queued, RunStatus::Running, started)
                .await
                .expect("the CAS is answered")
        );
        assert!(
            store
                .transition_run(ids::RUN_2, RunStatus::Running, RunStatus::Done, finished)
                .await
                .expect("the CAS is answered")
        );
        let run = store
            .run(ids::RUN_2)
            .await
            .expect("the run reads back")
            .expect("it exists");
        assert_eq!(run.started_at, Some(started));
        assert_eq!(run.finished_at, Some(finished));
        assert!(
            matches!(
                store
                    .transition_run(ids::RUN_2, RunStatus::Done, RunStatus::Queued, finished)
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "a terminal run reaches nothing"
        );
        assert_eq!(
            store
                .item(ids::HTUI_FEAT_3)
                .await
                .expect("the item reads back")
                .expect("it exists")
                .status,
            Status::Queued,
            "no run or step move touches the item"
        );
    }

    /// `supersede_step` is §4.4's loop half and `fail_run` is §4.3's failure row; both refuse
    /// through the law rather than through a stale compare-and-set.
    #[tokio::test]
    async fn supersede_and_fail_run_refuse_what_the_law_forbids() {
        let store = MemStore::demo();
        store
            .supersede_step(ids::STEP_R2_PRD)
            .await
            .expect("pending -> superseded is legal");
        assert!(
            matches!(
                store.supersede_step(ids::STEP_R2_PRD).await,
                Err(StoreError::Constraint(_))
            ),
            "superseded reaches nothing"
        );
        assert!(matches!(
            store.supersede_step(StepId::new()).await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));

        let at = Utc::now();
        store
            .fail_run(ids::RUN_2, "boom", at)
            .await
            .expect("queued -> failed is legal");
        let run = store
            .run(ids::RUN_2)
            .await
            .expect("the run reads back")
            .expect("it exists");
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.failure.as_deref(), Some("boom"));
        assert_eq!(run.finished_at, Some(at));
        assert!(
            matches!(
                store.fail_run(ids::RUN_2, "again", at).await,
                Err(StoreError::Constraint(_))
            ),
            "a terminal run cannot fail twice"
        );
        assert!(matches!(
            store.fail_run(RunId::new(), "boom", at).await,
            Err(StoreError::NotFound { entity: "run", .. })
        ));
    }

    /// `finish_step` writes the settle columns and never `status`; `usage` and `trim_record`
    /// `None` leave the column, every other field overwrites.
    #[tokio::test]
    async fn finish_step_settles_the_columns_and_leaves_usage_when_it_is_none() {
        let store = MemStore::demo();
        let finished = Utc::now();
        store
            .finish_step(
                StepFence::Unleased,
                ids::STEP_R2_PRD,
                StepOutcome {
                    exit_code: Some(0),
                    usage: Some(json!({ "input_tokens": 3 })),
                    trim_record: Some(json!({ "trimmed": [] })),
                    verify_outcome: Some(VerifyOutcome::Pass),
                    verify_exit_code: Some(0),
                    finished_at: finished,
                },
            )
            .await
            .expect("the settle lands");
        store
            .finish_step(
                StepFence::Unleased,
                ids::STEP_R2_PRD,
                StepOutcome {
                    exit_code: Some(1),
                    finished_at: finished,
                    ..StepOutcome::default()
                },
            )
            .await
            .expect("the second settle lands");

        let row = store.read(|state| {
            state
                .steps
                .get(&ids::STEP_R2_PRD)
                .cloned()
                .expect("the fixture step")
        });
        assert_eq!(
            row.status,
            StepStatus::Pending,
            "finish_step never moves status"
        );
        assert_eq!(row.exit_code, Some(1), "exit_code overwrites");
        assert_eq!(
            row.usage,
            Some(json!({ "input_tokens": 3 })),
            "a `None` usage leaves the column"
        );
        assert_eq!(
            row.trim_record,
            Some(json!({ "trimmed": [] })),
            "a `None` trim_record leaves the column"
        );
        assert_eq!(
            row.verify_outcome, None,
            "verify_outcome is not one of the two that leave"
        );
        assert!(matches!(
            store
                .finish_step(StepFence::Unleased, StepId::new(), StepOutcome::default())
                .await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));
    }

    /// `R-ORCH-2`'s four answers and §4.8's promotion, which lifts the step, its run and its item
    /// in one transaction.
    #[tokio::test]
    async fn gate_answers_write_their_outcome_and_promotion_lifts_the_run_and_the_item() {
        let store = MemStore::demo();
        let at = Utc::now();
        let approved = gated_step(&store, new_step(ids::RUN_2, 1, 1, 0)).await;
        let rejected = gated_step(&store, new_step(ids::RUN_2, 2, 1, 0)).await;
        let retried = gated_step(&store, new_step(ids::RUN_2, 3, 1, 0)).await;
        let skipped = gated_step(&store, new_step(ids::RUN_2, 4, 1, 0)).await;

        assert!(
            store
                .answer_gate(approved, GateOutcome::Approved, Some("ok".to_owned()), at)
                .await
                .expect("the answer is recorded")
        );
        assert!(
            !store
                .answer_gate(approved, GateOutcome::Approved, None, at)
                .await
                .expect("the answer is recorded"),
            "a step that is not awaiting cannot be answered twice"
        );
        for (step, outcome) in [
            (rejected, GateOutcome::Rejected),
            (retried, GateOutcome::Retried),
            (skipped, GateOutcome::Skipped),
        ] {
            assert!(
                store
                    .answer_gate(step, outcome, None, at)
                    .await
                    .expect("the answer is recorded")
            );
        }
        let by_id =
            |id: StepId| store.read(move |state| state.steps.get(&id).cloned().expect("the step"));
        assert_eq!(by_id(approved).status, StepStatus::Done);
        assert_eq!(by_id(approved).gate_outcome, Some(GateOutcome::Approved));
        assert_eq!(by_id(approved).gate_note.as_deref(), Some("ok"));
        assert_eq!(by_id(approved).finished_at, Some(at));
        assert_eq!(by_id(rejected).status, StepStatus::Failed);
        assert_eq!(by_id(retried).status, StepStatus::Superseded);
        assert_eq!(by_id(skipped).status, StepStatus::Done);
        assert!(matches!(
            store
                .answer_gate(StepId::new(), GateOutcome::Approved, None, at)
                .await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));

        store
            .transition_run(ids::RUN_2, RunStatus::Queued, RunStatus::Running, at)
            .await
            .expect("the run starts");
        store
            .transition(ids::HTUI_FEAT_3, Status::Queued, Status::InProgress)
            .await
            .expect("the item starts");
        store
            .promote_step(rejected, at)
            .await
            .expect("a failed step under a live run can be promoted");
        assert_eq!(by_id(rejected).status, StepStatus::AwaitingApproval);
        assert_eq!(by_id(rejected).promoted_at, Some(at));
        assert_eq!(
            store
                .run(ids::RUN_2)
                .await
                .expect("the run reads back")
                .expect("it exists")
                .status,
            RunStatus::AwaitingApproval
        );
        assert_eq!(
            store
                .item(ids::HTUI_FEAT_3)
                .await
                .expect("the item reads back")
                .expect("it exists")
                .status,
            Status::AwaitingApproval
        );
        assert!(
            matches!(
                store.promote_step(approved, at).await,
                Err(StoreError::Constraint(_))
            ),
            "a done step is not promotable"
        );
        assert!(matches!(
            store.promote_step(StepId::new(), at).await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));
    }

    /// ANA-2 §4.5's bookkeeping is one transaction (plan D6): a refused winner leaves every
    /// candidate exactly as it was.
    #[tokio::test]
    async fn select_fanout_settles_every_candidate_or_none() {
        let store = MemStore::demo();
        let at = Utc::now();
        let winner = gated_step(&store, new_step(ids::RUN_2, 1, 1, 0)).await;
        let loser = gated_step(&store, new_step(ids::RUN_2, 1, 1, 1)).await;
        let failed = gated_step(&store, new_step(ids::RUN_2, 1, 1, 2)).await;
        let elsewhere = gated_step(&store, new_step(ids::RUN_2, 2, 1, 0)).await;
        let judge = new_step(ids::RUN_2, 1, 1, -1);
        let judge_id = judge.id;
        store.create_step(judge).await.expect("the judge lands");
        store
            .answer_gate(failed, GateOutcome::Rejected, None, at)
            .await
            .expect("the third candidate fails");

        assert!(
            matches!(
                store.select_fanout(ids::RUN_2, 1, 1, elsewhere, None).await,
                Err(StoreError::Constraint(_))
            ),
            "a winner from another position is refused"
        );
        assert!(matches!(
            store
                .select_fanout(ids::RUN_2, 1, 1, StepId::new(), None)
                .await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));
        let by_id =
            |id: StepId| store.read(move |state| state.steps.get(&id).cloned().expect("the step"));
        assert_eq!(
            by_id(loser).selected,
            None,
            "a refused selection writes nothing at all"
        );

        store
            .select_fanout(ids::RUN_2, 1, 1, winner, Some("shorter diff".to_owned()))
            .await
            .expect("the selection lands");
        assert_eq!(by_id(winner).selected, Some(true));
        assert_eq!(by_id(winner).status, StepStatus::Done);
        assert_eq!(by_id(loser).selected, Some(false));
        assert_eq!(by_id(loser).status, StepStatus::Superseded);
        assert_eq!(by_id(failed).selected, Some(false));
        assert_eq!(
            by_id(failed).status,
            StepStatus::Failed,
            "a failed loser keeps its status"
        );
        assert_eq!(by_id(judge_id).status, StepStatus::Done);
        assert_eq!(by_id(judge_id).gate_note.as_deref(), Some("shorter diff"));
        assert_eq!(
            by_id(elsewhere).selected,
            None,
            "another position is not part of this fan-out"
        );
    }

    /// `run_step_tree` and `run_step_commit` upsert on `(run_step_id, repo_id)` and read back in
    /// `repo_id` order, and a project delete counts both — two tables `MemStore` has never held.
    #[tokio::test]
    async fn trees_and_commits_upsert_on_their_repo_key_and_the_delete_counts_them() {
        let store = MemStore::demo();
        let core = a_repo(&store, "core").await;
        let docs = a_repo(&store, "docs").await;
        let (first, second) = if core < docs {
            (core, docs)
        } else {
            (docs, core)
        };
        let tree = |repo: RepoId, dirty: bool| RunStepTree {
            run_step_id: ids::STEP_R2_PRD,
            repo_id: repo,
            mode: Isolation::Worktree,
            path: "/tmp/tree".to_owned(),
            base_ref: "main".to_owned(),
            dirty,
        };

        store
            .upsert_step_tree(
                StepFence::Unleased,
                ids::STEP_R2_PRD,
                &[tree(second, false), tree(first, false)],
            )
            .await
            .expect("both rows land");
        let rows = store
            .step_trees(ids::STEP_R2_PRD)
            .await
            .expect("the trees read back");
        assert_eq!(
            rows.iter().map(|row| row.repo_id).collect::<Vec<_>>(),
            vec![first, second],
            "repo_id order regardless of input order"
        );
        store
            .upsert_step_tree(StepFence::Unleased, ids::STEP_R2_PRD, &[tree(first, true)])
            .await
            .expect("the upsert replaces rather than inserts");
        let rows = store
            .step_trees(ids::STEP_R2_PRD)
            .await
            .expect("the trees read back");
        assert_eq!(rows.len(), 2, "still two rows");
        assert!(rows[0].dirty, "the row was replaced on its key");

        let stray = RunStepTree {
            run_step_id: ids::STEP_PLAN,
            ..tree(first, false)
        };
        assert!(
            matches!(
                store
                    .upsert_step_tree(StepFence::Unleased, ids::STEP_R2_PRD, &[stray])
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "a row for another step is refused"
        );
        assert!(
            matches!(
                store
                    .upsert_step_tree(
                        StepFence::Unleased,
                        ids::STEP_R2_PRD,
                        &[tree(RepoId::new(), false)]
                    )
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "an unknown repo is a foreign key refusal"
        );
        assert!(matches!(
            store
                .upsert_step_tree(StepFence::Unleased, StepId::new(), &[])
                .await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));
        store
            .upsert_step_tree(StepFence::Unleased, ids::STEP_R2_PRD, &[])
            .await
            .expect("an empty slice checks the step and writes nothing");

        let commit = |repo: RepoId, after: Option<&str>| RunStepCommit {
            run_step_id: ids::STEP_R2_PRD,
            repo_id: repo,
            before_hash: "abc".to_owned(),
            after_hash: after.map(ToOwned::to_owned),
        };
        store
            .record_commits(
                StepFence::Unleased,
                ids::STEP_R2_PRD,
                &[commit(second, None), commit(first, None)],
            )
            .await
            .expect("both rows land");
        store
            .record_commits(
                StepFence::Unleased,
                ids::STEP_R2_PRD,
                &[commit(first, Some("def"))],
            )
            .await
            .expect("the upsert replaces");
        let commits = store
            .step_commits(ids::STEP_R2_PRD)
            .await
            .expect("the commits read back");
        assert_eq!(
            commits.iter().map(|row| row.repo_id).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(commits[0].after_hash.as_deref(), Some("def"));
        assert!(matches!(
            store
                .record_commits(StepFence::Unleased, StepId::new(), &[])
                .await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));
        assert!(
            store
                .step_trees(StepId::new())
                .await
                .expect("a list read is total")
                .is_empty(),
            "an unknown step has no trees, and is not NotFound"
        );

        let reach = store
            .delete_reach(DeleteTarget::Project(ids::PROJECT_HTUI))
            .await
            .expect("the reach is counted")
            .expect("the project exists");
        assert_eq!(reach.run_step_trees, 2, "MemStore now holds run_step_tree");
        assert_eq!(reach.run_step_commits, 2, "and run_step_commit");
        let taken = store
            .delete_project(ids::PROJECT_HTUI)
            .await
            .expect("the delete lands");
        assert_eq!(taken, reach, "the report equals the act (PRD D13)");
        assert!(
            store
                .step_trees(ids::STEP_R2_PRD)
                .await
                .expect("the read is total")
                .is_empty(),
            "the rows went with their step"
        );
        assert!(
            store
                .step_commits(ids::STEP_R2_PRD)
                .await
                .expect("the read is total")
                .is_empty()
        );
    }

    /// The version is allocated inside the transaction, per `(item, kind)` (plan D6).
    #[tokio::test]
    async fn write_document_allocates_the_next_version_of_its_kind() {
        let store = MemStore::demo();
        let third = store
            .write_document(a_document(ids::HTUI_FEAT_1, "plan", Some(ids::STEP_PLAN)))
            .await
            .expect("the document lands");
        assert_eq!(third.version, 3, "the fixture holds plan v1 and v2");
        let fourth = store
            .write_document(a_document(ids::HTUI_FEAT_1, "plan", None))
            .await
            .expect("the document lands");
        assert_eq!(fourth.version, 4);
        let fresh = store
            .write_document(a_document(ids::HTUI_FEAT_1, "review", None))
            .await
            .expect("the document lands");
        assert_eq!(fresh.version, 1, "a kind with no rows starts at 1");

        assert!(matches!(
            store
                .write_document(a_document(ItemId::new(), "plan", None))
                .await,
            Err(StoreError::NotFound { entity: "item", .. })
        ));
        assert!(
            matches!(
                store
                    .write_document(a_document(ids::HTUI_FEAT_1, "plan", Some(StepId::new())))
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "an unknown producing step is a foreign key refusal"
        );
        let duplicate = NewDocument {
            id: third.id,
            ..a_document(ids::HTUI_FEAT_1, "plan", None)
        };
        assert!(matches!(
            store.write_document(duplicate).await,
            Err(StoreError::Constraint(_))
        ));
    }

    /// Plan D2: `resolve_inputs` prefers this run's output, skips a fan-out loser and reports a
    /// kind the item has no eligible row for — none of which `documents_of_kinds` does — and
    /// (MOD-73) reads a hand-written version newer than this run's output.
    #[tokio::test]
    async fn resolve_inputs_prefers_this_run_and_skips_a_loser() {
        let store = MemStore::demo();
        let winner = gated_step(&store, new_step(ids::RUN_2, 1, 1, 0)).await;
        let loser = gated_step(&store, new_step(ids::RUN_2, 1, 1, 1)).await;
        let by_winner = store
            .write_document(a_document(ids::HTUI_FEAT_3, "implementation", Some(winner)))
            .await
            .expect("v1 lands");
        let by_loser = store
            .write_document(a_document(ids::HTUI_FEAT_3, "implementation", Some(loser)))
            .await
            .expect("v2 lands");
        assert_eq!((by_winner.version, by_loser.version), (1, 2));
        store
            .select_fanout(ids::RUN_2, 1, 1, winner, None)
            .await
            .expect("the selection lands");

        let kinds = ["implementation".to_owned(), "nope".to_owned()];
        let resolved = store
            .resolve_inputs(ids::HTUI_FEAT_3, ids::RUN_2, &kinds)
            .await
            .expect("the resolver answers");
        assert_eq!(
            resolved
                .iter()
                .map(|row| row.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["implementation", "nope"],
            "one entry per requested kind, in request order"
        );
        assert_eq!(
            resolved[0].document.as_ref().map(|row| row.id),
            Some(by_winner.id),
            "`selected IS NOT FALSE` excludes the loser's higher version"
        );
        assert_eq!(
            resolved[1].document, None,
            "a kind the item has no eligible row for is carried as None"
        );
        assert_eq!(
            store
                .documents_of_kinds(ids::HTUI_FEAT_3, &["implementation".to_owned()])
                .await
                .expect("the shipped read answers")
                .first()
                .map(|row| row.id),
            Some(by_loser.id),
            "documents_of_kinds excludes no loser (plan D2's contrast)"
        );

        let hand = store
            .write_document(a_document(ids::HTUI_FEAT_1, "plan", None))
            .await
            .expect("v3 lands");
        let preferred = store
            .resolve_inputs(ids::HTUI_FEAT_1, ids::RUN_1, &["plan".to_owned()])
            .await
            .expect("the resolver answers");
        assert_eq!(
            preferred[0].document.as_ref().map(|row| row.id),
            Some(hand.id),
            "a hand-written version newer than this run's output outranks it (MOD-73)"
        );

        let all = store
            .resolve_inputs(ids::HTUI_FEAT_1, ids::RUN_1, &[])
            .await
            .expect("the resolver answers");
        assert_eq!(
            all.iter().map(|row| row.kind.as_str()).collect::<Vec<_>>(),
            vec!["plan", "prd"],
            "an empty `kinds` is every kind the item has, in byte order"
        );
        assert!(
            store
                .resolve_inputs(ItemId::new(), ids::RUN_1, &["plan".to_owned()])
                .await
                .expect("the read is total")[0]
                .document
                .is_none(),
            "an unknown item resolves every kind to None rather than refusing"
        );
    }

    /// `R-TUI-9`'s close-out is one transaction, refused while any run of the item is active.
    #[tokio::test]
    async fn close_out_refuses_a_live_run_and_otherwise_writes_all_three_effects() {
        let store = MemStore::demo();
        let repo = a_repo(&store, "core").await;
        let summary = |item: ItemId, kind: &str| NewDocument {
            ..a_document(item, kind, None)
        };
        let commits = [RunStepCommit {
            run_step_id: ids::STEP_IMPL,
            repo_id: repo,
            before_hash: "abc".to_owned(),
            after_hash: Some("def".to_owned()),
        }];

        assert!(
            matches!(
                store
                    .close_out(
                        ids::HTUI_FEAT_3,
                        Resolution::Withdrawn,
                        summary(ids::HTUI_FEAT_3, "summary"),
                        &[]
                    )
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "RUN_2 is queued, so FEAT-3 cannot be closed out"
        );
        assert!(
            matches!(
                store
                    .close_out(
                        ids::HTUI_FEAT_1,
                        Resolution::Done,
                        summary(ids::HTUI_FEAT_1, "plan"),
                        &[]
                    )
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "the document must be a summary"
        );
        assert!(
            matches!(
                store
                    .close_out(
                        ids::HTUI_FEAT_1,
                        Resolution::Done,
                        summary(ids::HTUI_ANA_2, "summary"),
                        &[]
                    )
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "the summary must name the item being closed"
        );
        assert!(
            matches!(
                store
                    .close_out(
                        ids::HTUI_ANA_2,
                        Resolution::Done,
                        summary(ids::HTUI_ANA_2, "summary"),
                        &[]
                    )
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "open does not close as done (ANA-11 §4.2)"
        );
        assert!(matches!(
            store
                .close_out(
                    ItemId::new(),
                    Resolution::Withdrawn,
                    summary(ItemId::new(), "summary"),
                    &[]
                )
                .await,
            Err(StoreError::NotFound { entity: "item", .. })
        ));
        assert_eq!(
            store
                .documents(ids::HTUI_FEAT_1)
                .await
                .expect("the heads read back")
                .len(),
            3,
            "no refusal wrote a document"
        );

        store
            .transition(ids::HTUI_FEAT_1, Status::InProgress, Status::Done)
            .await
            .expect("the item finishes");
        let written = store
            .close_out(
                ids::HTUI_FEAT_1,
                Resolution::Done,
                summary(ids::HTUI_FEAT_1, "summary"),
                &commits,
            )
            .await
            .expect("the close-out lands");
        assert_eq!(written.version, 1, "the first summary of the item");
        let item = store
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("the item reads back")
            .expect("it exists");
        assert_eq!(item.status, Status::Closed);
        assert_eq!(item.resolution, Some(Resolution::Done), "and says why");
        assert!(item.closed_at.is_some(), "closed_at tracks the status");
        assert_eq!(
            store
                .step_commits(ids::STEP_IMPL)
                .await
                .expect("the commits read back")
                .len(),
            1
        );
    }

    /// MOD-41 blueprint B-1: `close_out`'s per-commit check passes no fence. Close-out runs on a
    /// finished item with no lease of its own, so its commits land even on a run whose lease
    /// still names a dead owner, where a fenced `record_commits` under no lease is refused.
    #[tokio::test]
    async fn close_out_records_commits_without_a_fence() {
        let store = MemStore::demo();
        let repo = a_repo(&store, "core").await;
        let run = store.read(|state| {
            state
                .steps
                .get(&ids::STEP_IMPL)
                .expect("the fixture step")
                .run_id
        });
        assert!(
            store.read(|state| !state.lease_owners.contains_key(&run)),
            "the finished run's lease was released"
        );
        let dead = Uuid::now_v7();
        store.write(|state| state.lease_owners.insert(run, dead));
        let commit = RunStepCommit {
            run_step_id: ids::STEP_IMPL,
            repo_id: repo,
            before_hash: "abc".to_owned(),
            after_hash: Some("def".to_owned()),
        };
        assert!(
            matches!(
                store
                    .record_commits(
                        StepFence::Unleased,
                        ids::STEP_IMPL,
                        std::slice::from_ref(&commit)
                    )
                    .await,
                Err(StoreError::Fenced { step }) if step == ids::STEP_IMPL
            ),
            "the fenced writer is refused on a run whose lease names an owner"
        );

        store
            .transition(ids::HTUI_FEAT_1, Status::InProgress, Status::Done)
            .await
            .expect("the item finishes");
        store
            .close_out(
                ids::HTUI_FEAT_1,
                Resolution::Done,
                a_document(ids::HTUI_FEAT_1, "summary", None),
                std::slice::from_ref(&commit),
            )
            .await
            .expect("close-out checks the step, not a fence");
        assert_eq!(
            store
                .step_commits(ids::STEP_IMPL)
                .await
                .expect("the commits read back"),
            vec![commit],
            "the close-out's one commit row is stored"
        );
    }

    /// ANA-2 invariant 7's refusal note: every foreign key is checked before the insert.
    #[tokio::test]
    async fn add_note_writes_the_row_and_refuses_every_dangling_reference() {
        let store = MemStore::demo();
        let note = |item: ItemId, author, step| NewNote {
            id: NoteId::new(),
            item_id: item,
            body: "refused".to_owned(),
            created_by: author,
            box_id: Some(ids::BOX),
            via_step_id: step,
            created_at: Utc::now(),
        };
        let written = store
            .add_note(note(ids::HTUI_FEAT_1, ids::USER, Some(ids::STEP_IMPL)))
            .await
            .expect("the note lands");
        assert_eq!(
            store
                .notes(ids::HTUI_FEAT_1)
                .await
                .expect("the notes read back")
                .last(),
            Some(&written),
            "the returned row is the stored row"
        );
        for bad in [
            note(ItemId::new(), ids::USER, None),
            note(ids::HTUI_FEAT_1, UserId::new(), None),
            note(ids::HTUI_FEAT_1, ids::USER, Some(StepId::new())),
        ] {
            assert!(
                matches!(store.add_note(bad).await, Err(StoreError::Constraint(_))),
                "a dangling reference is a foreign key refusal"
            );
        }
        let duplicate = NewNote {
            id: written.id,
            ..note(ids::HTUI_FEAT_1, ids::USER, None)
        };
        assert!(matches!(
            store.add_note(duplicate).await,
            Err(StoreError::Constraint(_))
        ));
    }

    /// MOD-37 R-6: what `create_phase_agents` writes is what every inherent reader answers, and a
    /// project delete takes it. The store-side half is the conformance case
    /// `phase_agents_are_written_whole_and_counted`.
    #[tokio::test]
    async fn written_phase_agents_answer_every_reader() {
        let store = MemStore::demo();
        let implement = ids::PHASE_HTUI_IMPLEMENT;
        let row = |position: i32, agent_id: AgentId, model: &str| PhaseAgent {
            phase_id: implement,
            position,
            agent_id,
            model: model.to_owned(),
        };
        let first = row(0, ids::AGENT_CLAUDE, "sonnet");
        let second = row(1, ids::AGENT_AGY, "opus");
        store
            .create_phase_agents(implement, &[second.clone(), first.clone()])
            .await
            .expect("the rows are written");

        assert_eq!(
            store
                .phase_agents(implement)
                .await
                .expect("the read is total"),
            vec![first.clone(), second.clone()],
            "candidates come back in position order, not insertion order"
        );
        let resolved = store
            .resolve_graph(ids::HTUI_FEAT_1)
            .await
            .expect("the graph resolves")
            .expect("FEAT items have a default graph");
        assert!(
            resolved
                .phases
                .iter()
                .any(|phase| phase.phase.id == implement),
            "the implement phase is in the FEAT graph"
        );
        for phase in &resolved.phases {
            let expected = if phase.phase.id == implement {
                vec![first.clone(), second.clone()]
            } else {
                Vec::new()
            };
            assert_eq!(
                phase.agents, expected,
                "resolve_graph carries each phase's own candidates"
            );
        }

        let report = store
            .delete_project(ids::PROJECT_HTUI)
            .await
            .expect("the project is deleted");
        assert_eq!(report.phase_agents, 2, "the delete counts the two rows");
        assert!(
            store
                .phase_agents(implement)
                .await
                .expect("the read is total")
                .is_empty(),
            "and takes them with their phase"
        );
    }

    /// The eleven inherent reads of ANA-2 §8 that `Backend`'s `match self` will dispatch (plan
    /// D1, blueprint F-N): each answers from the fixture, and the two that take an id refuse an
    /// unknown one.
    #[tokio::test]
    async fn the_eleven_inherent_reads_answer_from_the_fixture() {
        let store = MemStore::demo();
        assert_eq!(
            store
                .step_graph(ids::GRAPH_HTUI_FEAT)
                .await
                .expect("the graph reads back")
                .map(|row| row.id),
            Some(ids::GRAPH_HTUI_FEAT)
        );
        assert!(
            store
                .phase_agents(ids::PHASE_HTUI_IMPLEMENT)
                .await
                .expect("the read is total")
                .is_empty(),
            "the fixture seeds no phase_agent row"
        );
        assert!(
            store
                .prompt_template(ids::PROJECT_HTUI, "implement", None)
                .await
                .expect("the template reads back")
                .is_some(),
            "no version pin means the latest"
        );
        assert!(
            store
                .prompt_template(ids::PROJECT_HTUI, "implement", Some(99))
                .await
                .expect("the template reads back")
                .is_none(),
            "a pin that cannot be honoured resolves to nothing"
        );
        let resolved = store
            .resolve_graph(ids::HTUI_FEAT_1)
            .await
            .expect("the graph resolves")
            .expect("FEAT items have a default graph");
        assert_eq!(resolved.graph.id, ids::GRAPH_HTUI_FEAT);
        assert_eq!(
            resolved
                .phases
                .iter()
                .map(|row| row.phase.position)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "phases in position order"
        );
        assert!(
            resolved.phases.iter().all(|row| row.agents.is_empty()),
            "the fixture seeds no phase_agent row"
        );
        assert!(
            store
                .agent_boxes(ids::BOX)
                .await
                .expect("the read is total")
                .is_empty(),
            "no fixture probes a box"
        );
        assert_eq!(
            store
                .box_row(ids::BOX)
                .await
                .expect("the box reads back")
                .map(|row| row.hostname),
            Some("DESKTOP-HTUI".to_owned())
        );
        assert!(
            store
                .repo_paths(ids::BOX)
                .await
                .expect("the read is total")
                .is_empty()
        );

        let needs_cuda = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: "needs a GPU toolchain".to_owned(),
                body: String::new(),
                required_tags: vec!["cuda".to_owned()],
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands")
            .id;
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let ready: Vec<ItemId> = store
            .ready_items(&scope, ids::BOX)
            .await
            .expect("the read is total")
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert!(
            ready.contains(&ids::HTUI_ANA_2),
            "an open, untagged item is ready"
        );
        assert!(
            !ready.contains(&needs_cuda),
            "a tag the box has neither probed nor declared holds the item back"
        );
        assert_eq!(
            store
                .missing_tags(needs_cuda, ids::BOX)
                .await
                .expect("the tags read back"),
            vec!["cuda".to_owned()]
        );
        assert!(matches!(
            store.missing_tags(ItemId::new(), ids::BOX).await,
            Err(StoreError::NotFound { entity: "item", .. })
        ));
        assert!(matches!(
            store.missing_tags(needs_cuda, BoxId::new()).await,
            Err(StoreError::NotFound { entity: "box", .. })
        ));

        assert_eq!(
            store
                .active_runs_on_box(ids::BOX)
                .await
                .expect("the count is answered"),
            0,
            "the fixture's only active run has not been claimed"
        );
        let repo = a_repo(&store, "core").await;
        let run = store
            .create_run(graph_run(ids::HTUI_ANA_2, ids::PROJECT_HTUI, vec![repo]))
            .await
            .expect("the run is queued")
            .id;
        let at = Utc::now();
        assert_eq!(
            store
                .claim_run(run, ids::BOX, Uuid::now_v7(), at, TimeDelta::minutes(5),)
                .await
                .expect("the claim is answered"),
            Claim::Admitted
        );
        assert_eq!(
            store
                .active_runs_on_box(ids::BOX)
                .await
                .expect("the count is answered"),
            1
        );
        assert_eq!(
            store
                .overlapping_runs(&[repo])
                .await
                .expect("the read is total")
                .into_iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![run]
        );
        assert!(
            store
                .overlapping_runs(&[RepoId::new()])
                .await
                .expect("the read is total")
                .is_empty()
        );
        assert!(
            store
                .overlapping_runs(&[])
                .await
                .expect("the read is total")
                .is_empty(),
            "an empty scope overlaps nothing (hazard H-10)"
        );
    }

    /// A ready, untagged FEAT in `htui` at `priority`, minted through `store`'s own clock.
    async fn ready_feat(store: &MemStore, id: ItemId, priority: i16) -> ItemId {
        store
            .mint_item(NewItem {
                id,
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: format!("queue order at priority {priority}"),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands")
            .id
    }

    /// `ready_items` under `htui` alone, as ids.
    async fn ready_htui(store: &MemStore) -> Vec<ItemId> {
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        store
            .ready_items(&scope, ids::BOX)
            .await
            .expect("the read is total")
            .into_iter()
            .map(|row| row.id)
            .collect()
    }

    /// MOD-12 D4, ANA-2 criterion 22: `ready_items` is queue order, `priority DESC, created_at`,
    /// and not the Backlog's key order. `HTUI_ANA_2` is priority 0 with `created_at =
    /// demo_at(0, 1)`, earlier than every mint; key order would put it first and the priority-1
    /// FEAT between the two priority-0 ones.
    #[tokio::test]
    async fn ready_items_are_in_queue_order() {
        let clock = Arc::new(TestClock::new());
        let store = MemStore::demo().with_clock(clock.clone());
        let p0a = ready_feat(&store, ItemId::new(), 0).await;
        clock.advance(TimeDelta::seconds(1));
        let p1 = ready_feat(&store, ItemId::new(), 1).await;
        clock.advance(TimeDelta::seconds(1));
        let p0b = ready_feat(&store, ItemId::new(), 0).await;

        assert_eq!(ready_htui(&store).await, [p1, ids::HTUI_ANA_2, p0a, p0b]);
    }

    /// MOD-12 D4: two items of one priority minted at one instant come back in `ItemId` order,
    /// which is Postgres' uuid order. The larger id is minted first, so key order disagrees.
    #[tokio::test]
    async fn ready_items_break_a_created_at_tie_by_id() {
        let store = MemStore::demo().with_clock(Arc::new(TestClock::new()));
        let high = ItemId::from_uuid(Uuid::from_u128(u128::MAX - 1));
        let low = ItemId::from_uuid(Uuid::from_u128(1));
        ready_feat(&store, high, 0).await;
        ready_feat(&store, low, 0).await;

        assert_eq!(ready_htui(&store).await, [ids::HTUI_ANA_2, low, high]);
    }

    /// MOD-9 D89: a skill with no version (a hand-written or imported row; `create_skill` always
    /// writes v1, so no writer can make one) takes version 1 at the token `0`, and any other token
    /// is `NotFound` on the missing version, keyed `"<skill>/v<n>"`.
    #[tokio::test]
    async fn a_skill_with_no_version_takes_version_one_at_zero() {
        let mut data = crate::fixtures::demo_data();
        let id = crate::model::SkillId::new();
        let at = Utc::now();
        data.skills.push(crate::model::Skill {
            id,
            name: "bare".to_owned(),
            description: String::new(),
            created_by: ids::USER,
            created_at: at,
            updated_at: at,
        });
        let store = MemStore::from_demo(data);
        let version = |body: &str| crate::model::NewSkillVersion {
            body: body.to_owned(),
            source: json!({}),
            created_by: ids::USER,
        };

        let missing = store.add_skill_version(id, 1, version("one")).await;
        match missing {
            Err(StoreError::NotFound {
                entity: "skill_version",
                id: key,
            }) => assert_eq!(key, format!("{id}/v1"), "the missing version is named"),
            other => panic!("a token on a skill with no version is NotFound, got {other:?}"),
        }

        let CasOutcome::Applied(v1) = store
            .add_skill_version(id, 0, version("one"))
            .await
            .expect("append at 0")
        else {
            panic!("the token 0 on a skill with no version applies");
        };
        assert_eq!(
            (v1.skill_id, v1.version, v1.body.as_str()),
            (id, 1, "one"),
            "the first version of a bare skill is v1"
        );
    }

    /// MOD-26 plan D6: `resolve_graph` carries the persona row a phase names, and `None` for every
    /// phase that names none.
    #[tokio::test]
    async fn resolve_graph_fills_the_bound_persona() {
        let store = MemStore::demo();
        let resolved = store
            .resolve_graph(ids::HTUI_FEAT_3)
            .await
            .expect("the graph resolves")
            .expect("FEAT items have a default graph");
        assert_eq!(resolved.graph.id, ids::GRAPH_HTUI_FEAT);
        assert!(
            resolved.phases.iter().all(|row| row.persona.is_none()),
            "no fixture phase is bound"
        );
        let first = resolved.phases[0].phase.clone();
        let reviewer = store
            .personas()
            .await
            .expect("the registry reads")
            .into_iter()
            .find(|row| row.name == "reviewer")
            .expect("the demo registry holds reviewer");
        let CasOutcome::Applied(bound) = store
            .update_phase(
                first.id,
                first.updated_at,
                crate::model::PhasePatch {
                    persona: Some(Some(reviewer.id)),
                    ..crate::model::PhasePatch::default()
                },
            )
            .await
            .expect("the binding is a valid edit")
        else {
            panic!("the token was read from the store");
        };

        let resolved = store
            .resolve_graph(ids::HTUI_FEAT_3)
            .await
            .expect("the graph resolves")
            .expect("FEAT items have a default graph");
        assert_eq!(resolved.phases[0].phase, bound, "the row as bound");
        assert_eq!(
            resolved.phases[0].persona.as_ref(),
            Some(&reviewer),
            "the bound phase carries the persona row"
        );
        assert!(
            resolved.phases[1..].iter().all(|row| row.persona.is_none()),
            "every other phase carries none"
        );
    }

    /// MOD-26 plan D4: a persona's two stamps are the handle's clock, on create and on update.
    #[tokio::test]
    async fn persona_times_are_the_handles_clock() {
        let clock = TestClock::at(Utc::now() - TimeDelta::days(2));
        // The clock's own reading, which is truncated to the column's microseconds.
        let t = clock.now();
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        let created = store
            .create_persona(crate::model::NewPersona {
                id: crate::model::PersonaId::new(),
                name: "scout".to_owned(),
                description: String::new(),
                body: "You scout.\n".to_owned(),
                tools: crate::model::PersonaTools::default(),
                permission: crate::model::PersonaPermission::default(),
            })
            .await
            .expect("a valid persona is created");
        assert_eq!(
            (created.created_at, created.updated_at),
            (t, t),
            "both stamps are the handle's clock"
        );

        clock.advance(TimeDelta::minutes(5));
        let CasOutcome::Applied(edited) = store
            .update_persona(
                created.id,
                created.updated_at,
                crate::model::PersonaPatch {
                    body: Some("You scout ahead.\n".to_owned()),
                    ..crate::model::PersonaPatch::default()
                },
            )
            .await
            .expect("a valid edit")
        else {
            panic!("the token was read from the store");
        };
        assert_eq!(
            (edited.created_at, edited.updated_at),
            (t, clock.now()),
            "an edit stamps updated_at only, with the handle's clock"
        );
    }

    /// MOD-26 M2 D14 (blueprint F-6, B-4): `persona_is_bound` sorts the `(slug, graph, phase)`
    /// triples, not the joined strings (`"web-app/x" < "web/x"` as strings, but `"web" <
    /// "web-app"` as the first component), counts a duplicate once, names at most five and counts
    /// the rest.
    #[test]
    fn persona_is_bound_sorts_by_the_triple_and_counts_the_rest() {
        let triple = |project: &str, graph: &str, phase: &str| {
            (project.to_owned(), graph.to_owned(), phase.to_owned())
        };
        assert_eq!(
            crate::store::persona_is_bound("reviewer", &[triple("htui", "feature", "review")]),
            "persona `reviewer` is bound to 1 phase (`htui/feature/review`); clear it in \
             Settings \u{203a} Kinds first"
        );
        assert_eq!(
            crate::store::persona_is_bound(
                "reviewer",
                &[
                    triple("web-app", "x", "p"),
                    triple("web", "x", "p"),
                    triple("web", "x", "p")
                ],
            ),
            "persona `reviewer` is bound to 2 phases (`web/x/p`, `web-app/x/p`); clear them in \
             Settings \u{203a} Kinds first",
            "triple order puts `web` before `web-app`, and the duplicate counts once"
        );
        let seven: Vec<(String, String, String)> = ["a", "b", "c", "d", "e", "f", "g"]
            .iter()
            .map(|phase| triple("htui", "feature", phase))
            .collect();
        assert_eq!(
            crate::store::persona_is_bound("architect", &seven),
            "persona `architect` is bound to 7 phases (`htui/feature/a`, `htui/feature/b`, \
             `htui/feature/c`, `htui/feature/d`, `htui/feature/e` and 2 more); clear them in \
             Settings \u{203a} Kinds first"
        );
    }

    // ---- MOD-12 M1: the queue store surface (blueprint §C.3) ---------------------------------

    /// A graph run of `item` admitted under `batch`.
    fn batch_run(item: ItemId, batch: Option<crate::model::BatchId>) -> NewRun {
        NewRun {
            batch_id: batch,
            ..graph_run(item, ids::PROJECT_HTUI, Vec::new())
        }
    }

    /// MOD-12 D1, D9: `queue_item` is idempotent (the stored entry answers a repeat unchanged),
    /// `dequeue_item` reports membership, and an unknown item is `NotFound`.
    #[tokio::test]
    async fn queue_item_is_idempotent_and_dequeue_reports_membership() {
        let store = MemStore::demo();
        let at = Utc::now();
        let entry = store
            .queue_item(ids::HTUI_ANA_2, ids::BOX, ids::USER, at)
            .await
            .expect("an open item queues");
        assert_eq!(entry.item_id, ids::HTUI_ANA_2);
        assert_eq!(
            entry.project_id,
            ids::PROJECT_HTUI,
            "the item's project, joined"
        );
        assert_eq!(entry.box_id, ids::BOX);
        assert_eq!(entry.position, None, "milestone 1 writes no position");
        assert_eq!(entry.queued_at, at.trunc_subsecs(TIMESTAMPTZ_DIGITS));
        assert_eq!(entry.queued_by, ids::USER);

        let again = store
            .queue_item(
                ids::HTUI_ANA_2,
                ids::BOX,
                ids::USER,
                at + TimeDelta::minutes(1),
            )
            .await
            .expect("a repeat is answered");
        assert_eq!(again, entry, "a repeat answers the stored entry unchanged");
        assert_eq!(
            store
                .queue_entries(ids::BOX)
                .await
                .expect("the read is total"),
            vec![entry],
        );

        assert!(store.dequeue_item(ids::HTUI_ANA_2).await.expect("dequeue"));
        assert!(
            !store.dequeue_item(ids::HTUI_ANA_2).await.expect("dequeue"),
            "nothing held it the second time"
        );
        assert!(
            store
                .queue_entries(ids::BOX)
                .await
                .expect("the read is total")
                .is_empty()
        );

        assert!(matches!(
            store
                .queue_item(ItemId::new(), ids::BOX, ids::USER, at)
                .await,
            Err(StoreError::NotFound { entity: "item", .. })
        ));
        assert!(matches!(
            store
                .queue_item(ids::HTUI_ANA_2, BoxId::new(), ids::USER, at)
                .await,
            Err(StoreError::Constraint(_))
        ));
        assert!(matches!(
            store
                .queue_item(ids::HTUI_ANA_2, ids::BOX, UserId::new(), at)
                .await,
            Err(StoreError::Constraint(_))
        ));
    }

    /// MOD-12 D2: a second resume answers the open batch; `open_batch_of` sees it until the close.
    #[tokio::test]
    async fn open_batch_answers_the_open_one() {
        let store = MemStore::demo();
        let at = Utc::now();
        assert_eq!(store.open_batch_of(ids::BOX).await.expect("read"), None);
        let first = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("the batch opens");
        assert_eq!(first.box_id, ids::BOX);
        assert_eq!(first.opened_at, at.trunc_subsecs(TIMESTAMPTZ_DIGITS));
        assert_eq!(first.opened_by, ids::USER);
        assert_eq!((first.closed_at, first.closed_reason), (None, None));
        let second = store
            .open_batch(ids::BOX, ids::USER, at + TimeDelta::minutes(1))
            .await
            .expect("a repeat is answered");
        assert_eq!(second, first, "one open batch per box");
        assert_eq!(
            store.open_batch_of(ids::BOX).await.expect("read"),
            Some(first.clone())
        );
        store
            .close_batch(ids::BOX, BatchClose::Paused, at)
            .await
            .expect("the close is answered")
            .expect("one was open");
        assert_eq!(store.open_batch_of(ids::BOX).await.expect("read"), None);
        let third = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("a resume after a pause opens a new batch");
        assert_ne!(third.id, first.id);
        assert!(matches!(
            store.open_batch(BoxId::new(), ids::USER, at).await,
            Err(StoreError::Constraint(_))
        ));
    }

    /// MOD-12 D2, D3: the close stamps its reason and instant once; a second close finds none.
    #[tokio::test]
    async fn close_batch_records_its_reason_once() {
        let store = MemStore::demo();
        let at = Utc::now();
        let open = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("the batch opens");
        let closed = store
            .close_batch(ids::BOX, BatchClose::Paused, at + TimeDelta::seconds(1))
            .await
            .expect("the close is answered")
            .expect("one was open");
        assert_eq!(closed.id, open.id);
        assert_eq!(closed.closed_reason, Some(BatchClose::Paused));
        assert_eq!(
            closed.closed_at,
            Some((at + TimeDelta::seconds(1)).trunc_subsecs(TIMESTAMPTZ_DIGITS))
        );
        assert_eq!(
            store
                .close_batch(ids::BOX, BatchClose::Drained, at)
                .await
                .expect("the close is answered"),
            None,
            "nothing is open any more"
        );
    }

    /// Review H2: a close cancels the batch's runs still `queued` (their items back to `open`,
    /// their entries kept) and leaves its claimed runs and every run outside it alone.
    #[tokio::test]
    async fn close_batch_cancels_only_the_batch_runs_still_queued() {
        let store = MemStore::demo();
        let at = Utc::now();
        let batch = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("the batch opens");
        store
            .queue_item(ids::HTUI_ANA_2, ids::BOX, ids::USER, at)
            .await
            .expect("the item queues");
        let waiting = store
            .create_run(batch_run(ids::HTUI_ANA_2, Some(batch.id)))
            .await
            .expect("an open batch admits");
        let claimed = store
            .create_run(batch_run(ids::HTUI_CLEAN_1, Some(batch.id)))
            .await
            .expect("an open batch admits");
        assert_eq!(
            store
                .claim_run(
                    claimed.id,
                    ids::BOX,
                    Uuid::now_v7(),
                    at,
                    TimeDelta::minutes(5)
                )
                .await
                .expect("the claim reads"),
            Claim::Admitted
        );
        let outside = store.run(ids::RUN_2).await.expect("read").expect("RUN_2");
        assert_eq!(outside.status, RunStatus::Queued, "the fixture's own run");

        let later = at + TimeDelta::seconds(1);
        store
            .close_batch(ids::BOX, BatchClose::Paused, later)
            .await
            .expect("the close is answered")
            .expect("one was open");
        let cancelled = store.run(waiting.id).await.expect("read").expect("the run");
        assert_eq!(cancelled.status, RunStatus::Cancelled);
        assert_eq!(
            cancelled.finished_at,
            Some(later.trunc_subsecs(TIMESTAMPTZ_DIGITS))
        );
        assert_eq!(
            store
                .item(ids::HTUI_ANA_2)
                .await
                .expect("read")
                .map(|item| item.status),
            Some(Status::Open),
            "finish_run's mirror: a cancelled queued run reopens its item"
        );
        assert_eq!(
            store
                .queue_entries(ids::BOX)
                .await
                .expect("read")
                .iter()
                .map(|entry| entry.item_id)
                .collect::<Vec<_>>(),
            [ids::HTUI_ANA_2],
            "the entry stays"
        );
        assert_eq!(
            store
                .run(claimed.id)
                .await
                .expect("read")
                .map(|run| run.status),
            Some(RunStatus::Running)
        );
        assert_eq!(
            store
                .run(ids::RUN_2)
                .await
                .expect("read")
                .map(|run| run.status),
            Some(RunStatus::Queued),
            "a run outside the batch is not the close's"
        );
        assert_eq!(
            store
                .claim_run(
                    waiting.id,
                    ids::BOX,
                    Uuid::now_v7(),
                    later,
                    TimeDelta::minutes(5)
                )
                .await
                .expect("the claim reads"),
            Claim::NotClaimable,
            "a claim after the close finds the run cancelled"
        );
        assert_eq!(
            store.batch_cancelled_items(batch.id).await.expect("read"),
            [ids::HTUI_ANA_2]
        );
    }

    /// Review M1: the drain's close is of exactly the batch it names, and only while that batch
    /// is still drained, so a pause and a resume between the drain's reads and its close are
    /// never undone.
    #[tokio::test]
    async fn close_drained_batch_closes_only_the_drained_batch_it_names() {
        let store = MemStore::demo();
        let at = Utc::now();
        let first = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("the batch opens");
        store
            .close_batch(ids::BOX, BatchClose::Paused, at)
            .await
            .expect("the close is answered")
            .expect("one was open");
        let second = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("a resume opens a new batch");
        assert_eq!(
            store
                .close_drained_batch(first.id, at)
                .await
                .expect("answered"),
            None,
            "the checked batch is already closed"
        );
        assert_eq!(
            store
                .open_batch_of(ids::BOX)
                .await
                .expect("read")
                .map(|open| open.id),
            Some(second.id),
            "the resume's batch stays open"
        );

        store
            .queue_item(ids::HTUI_ANA_2, ids::BOX, ids::USER, at)
            .await
            .expect("the item queues");
        assert_eq!(
            store
                .close_drained_batch(second.id, at)
                .await
                .expect("answered"),
            None,
            "an entry keeps it open"
        );
        assert!(store.dequeue_item(ids::HTUI_ANA_2).await.expect("dequeue"));
        let run = store
            .create_run(batch_run(ids::HTUI_ANA_2, Some(second.id)))
            .await
            .expect("an open batch admits");
        assert_eq!(
            store
                .close_drained_batch(second.id, at)
                .await
                .expect("answered"),
            None,
            "a live run of its own keeps it open"
        );
        store
            .finish_run(run.id, RunStatus::Cancelled, None, at)
            .await
            .expect("a queued run cancels");
        let closed = store
            .close_drained_batch(second.id, at)
            .await
            .expect("answered")
            .expect("drained now");
        assert_eq!(closed.id, second.id);
        assert_eq!(closed.closed_reason, Some(BatchClose::Drained));
        assert_eq!(store.open_batch_of(ids::BOX).await.expect("read"), None);
        assert_eq!(
            store
                .close_drained_batch(crate::model::BatchId::new(), at)
                .await
                .expect("answered"),
            None,
            "an unknown batch closes nothing"
        );
    }

    /// MOD-12 D3: only `done` and `closed` items leave the queue.
    #[tokio::test]
    async fn prune_finished_entries_drops_done_and_closed_items_only() {
        let store = MemStore::demo();
        let at = Utc::now();
        for item in [ids::HTUI_ANA_1, ids::HTUI_ANA_2] {
            store
                .queue_item(item, ids::BOX, ids::USER, at)
                .await
                .expect("the item queues");
        }
        assert_eq!(
            store
                .prune_finished_entries(ids::BOX)
                .await
                .expect("the prune is answered"),
            1,
            "ANA-1 is done"
        );
        let left: Vec<ItemId> = store
            .queue_entries(ids::BOX)
            .await
            .expect("the read is total")
            .into_iter()
            .map(|entry| entry.item_id)
            .collect();
        assert_eq!(left, [ids::HTUI_ANA_2]);
        assert_eq!(
            store
                .prune_finished_entries(ids::BOX)
                .await
                .expect("the prune is answered"),
            0
        );
    }

    /// MOD-12 D7, H-6: a run records its batch, and a closed or unknown batch refuses the run with
    /// nothing written.
    #[tokio::test]
    async fn create_run_records_its_batch_and_refuses_a_closed_one() {
        let store = MemStore::demo();
        let at = Utc::now();
        let batch = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("the batch opens");
        let run = store
            .create_run(batch_run(ids::HTUI_ANA_2, Some(batch.id)))
            .await
            .expect("an open batch admits");
        assert_eq!(
            store.batch_runs(batch.id).await.expect("read"),
            [(run.id, RunStatus::Queued)]
        );

        store
            .close_batch(ids::BOX, BatchClose::Paused, at)
            .await
            .expect("the close is answered");
        let status = |store: &MemStore| {
            store.read(|state| state.items.get(&ids::HTUI_CLEAN_1).map(|item| item.status))
        };
        let before = status(&store);
        let refused = batch_run(ids::HTUI_CLEAN_1, Some(batch.id));
        let refused_id = refused.id;
        assert!(matches!(
            store.create_run(refused).await,
            Err(StoreError::Constraint(message)) if message.contains("is closed")
        ));
        let unknown = batch_run(ids::HTUI_CLEAN_1, Some(crate::model::BatchId::new()));
        let unknown_id = unknown.id;
        assert!(matches!(
            store.create_run(unknown).await,
            Err(StoreError::NotFound {
                entity: "queue_batch",
                ..
            })
        ));
        for id in [refused_id, unknown_id] {
            assert_eq!(store.run(id).await.expect("read"), None, "nothing written");
        }
        assert_eq!(status(&store), before, "the item did not move");
        assert_eq!(
            store.batch_runs(batch.id).await.expect("read").len(),
            1,
            "the batch kept its one run"
        );
    }

    /// MOD-12 D1, D7: `delete_project` takes the project's entries and its runs' batch pairs, as
    /// Postgres' cascades do.
    #[tokio::test]
    async fn delete_project_takes_queue_entries_and_run_batches() {
        let store = MemStore::demo();
        let at = Utc::now();
        let batch = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("the batch opens");
        store
            .queue_item(ids::HTUI_ANA_2, ids::BOX, ids::USER, at)
            .await
            .expect("the item queues");
        store
            .create_run(batch_run(ids::HTUI_ANA_2, Some(batch.id)))
            .await
            .expect("an open batch admits");
        store
            .delete_project(ids::PROJECT_HTUI)
            .await
            .expect("the project goes");
        assert!(
            store
                .queue_entries(ids::BOX)
                .await
                .expect("read")
                .is_empty()
        );
        assert!(store.batch_runs(batch.id).await.expect("read").is_empty());
        store.read(|state| {
            assert!(state.queue_entries.is_empty(), "no entry remains");
            assert!(state.run_batches.is_empty(), "no run-batch pair remains");
        });
    }

    // ---- MOD-12 M2: batch spend (blueprint §C.1) -------------------------------------------

    /// A step of `run` at `position` whose `usage` is `usage` (MOD-12 M2 spend fixtures).
    async fn step_with_usage(store: &MemStore, run: RunId, position: i32, usage: Value) {
        let step = store
            .create_step(new_step(run, position, 0, 0))
            .await
            .expect("the step lands");
        store
            .set_step_usage(StepFence::Unleased, step.id, usage, None)
            .await
            .expect("the usage lands");
    }

    /// The batches and runs of [`spend_fixture`].
    struct SpendFixture {
        /// Two runs, costs 700 and 250, plus a `"x"` and a `1.5` cost that are skipped.
        a: crate::model::BatchId,
        /// One run costing 1 000.
        b: crate::model::BatchId,
        /// One run whose only step reports no cost.
        c: crate::model::BatchId,
        /// A run of batch A.
        a_run: RunId,
        /// A manual run costing 5 000, in no batch.
        manual: RunId,
    }

    /// MOD-12 M2 D1: three batches opened one after another on the demo box (one open batch per
    /// box), across projects, and a manual run beside them; `pg_criteria.rs` builds the same.
    async fn spend_fixture(store: &MemStore) -> SpendFixture {
        let at = Utc::now();
        let a = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("batch A opens");
        let a_run = store
            .create_run(batch_run(ids::HTUI_ANA_2, Some(a.id)))
            .await
            .expect("A admits ANA-2");
        let a_other = store
            .create_run(NewRun {
                project_id: ids::PROJECT_AGY,
                ..batch_run(ids::AGY_FEAT_1, Some(a.id))
            })
            .await
            .expect("A admits another project's item");
        step_with_usage(
            store,
            a_run.id,
            0,
            json!({"cost_micros": 700, "input_tokens": 9}),
        )
        .await;
        step_with_usage(store, a_run.id, 1, json!({"cost_micros": "x"})).await;
        step_with_usage(store, a_other.id, 0, json!({"cost_micros": 250})).await;
        step_with_usage(store, a_other.id, 1, json!({"cost_micros": 1.5})).await;
        store
            .close_batch(ids::BOX, BatchClose::Paused, at)
            .await
            .expect("the close")
            .expect("A was open");

        let b = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("batch B opens");
        let b_run = store
            .create_run(NewRun {
                project_id: ids::PROJECT_AGY,
                ..batch_run(ids::AGY_FIX_1, Some(b.id))
            })
            .await
            .expect("B admits");
        step_with_usage(store, b_run.id, 0, json!({"cost_micros": 1_000})).await;
        store
            .close_batch(ids::BOX, BatchClose::Paused, at)
            .await
            .expect("the close")
            .expect("B was open");

        let c = store
            .open_batch(ids::BOX, ids::USER, at)
            .await
            .expect("batch C opens");
        let c_run = store
            .create_run(batch_run(ids::HTUI_CLEAN_1, Some(c.id)))
            .await
            .expect("C admits");
        step_with_usage(store, c_run.id, 0, json!({"input_tokens": 3})).await;

        let manual = store
            .create_run(graph_run(
                ids::VULKAN_TOOL_1,
                ids::PROJECT_VULKAN,
                Vec::new(),
            ))
            .await
            .expect("a manual run");
        step_with_usage(store, manual.id, 0, json!({"cost_micros": 5_000})).await;
        SpendFixture {
            a: a.id,
            b: b.id,
            c: c.id,
            a_run: a_run.id,
            manual: manual.id,
        }
    }

    /// MOD-12 M2 D1: no step reporting an integer cost is `None`, not `Some(0)` (unknown is
    /// unbounded, OQ-6).
    #[tokio::test]
    async fn batch_spend_is_none_without_a_costed_step() {
        let store = MemStore::demo();
        let batch = store
            .open_batch(ids::BOX, ids::USER, Utc::now())
            .await
            .expect("the batch opens");
        let run = store
            .create_run(batch_run(ids::HTUI_ANA_2, Some(batch.id)))
            .await
            .expect("an open batch admits");
        assert_eq!(store.batch_spend(batch.id).await.expect("read"), None);
        step_with_usage(&store, run.id, 0, json!({"input_tokens": 3})).await;
        assert_eq!(
            store.batch_spend(batch.id).await.expect("read"),
            None,
            "a usage row without a cost"
        );
    }

    /// MOD-12 M2 D1: the sum is over the batch's runs only, across projects; a non-integer cost
    /// is skipped and a manual run is no batch's.
    #[tokio::test]
    async fn batch_spend_sums_only_the_batch_runs() {
        let store = MemStore::demo();
        let fixture = spend_fixture(&store).await;
        assert_eq!(store.batch_spend(fixture.a).await.expect("read"), Some(950));
        assert_eq!(
            store.batch_spend(fixture.b).await.expect("read"),
            Some(1_000)
        );
        assert_eq!(store.batch_spend(fixture.c).await.expect("read"), None);
        assert_eq!(
            store
                .batch_spend(crate::model::BatchId::new())
                .await
                .expect("read"),
            None,
            "an unknown batch has spent nothing known"
        );
    }

    /// MOD-12 M2 D5: a manual run is in no batch; a batch run answers its batch and its spend.
    #[tokio::test]
    async fn run_batch_spend_is_none_for_a_manual_run() {
        let store = MemStore::demo();
        let fixture = spend_fixture(&store).await;
        assert_eq!(
            store.run_batch_spend(fixture.manual).await.expect("read"),
            None
        );
        assert_eq!(
            store.run_batch_spend(fixture.a_run).await.expect("read"),
            Some((fixture.a, Some(950)))
        );
        assert_eq!(
            store.run_batch_spend(RunId::new()).await.expect("read"),
            None,
            "an unknown run"
        );
    }

    /// MOD-12 D6 (H-5): the runner's slot count is `claim_run`'s, `running` alone; a parked run
    /// holds no slot, while `active_runs_on_box` still counts it.
    #[tokio::test]
    async fn running_runs_on_box_counts_running_only() {
        let store = MemStore::demo();
        let at = Utc::now();
        let run = store
            .create_run(graph_run(ids::HTUI_ANA_2, ids::PROJECT_HTUI, Vec::new()))
            .await
            .expect("the run is queued")
            .id;
        let base = store.running_runs_on_box(ids::BOX).await.expect("read");
        assert_eq!(
            store
                .claim_run(run, ids::BOX, Uuid::now_v7(), at, TimeDelta::minutes(5))
                .await
                .expect("the claim is answered"),
            Claim::Admitted
        );
        assert_eq!(
            store.running_runs_on_box(ids::BOX).await.expect("read"),
            base + 1
        );
        let active = store.active_runs_on_box(ids::BOX).await.expect("read");
        assert!(
            store
                .transition_run(run, RunStatus::Running, RunStatus::AwaitingApproval, at)
                .await
                .expect("the park is answered")
        );
        assert_eq!(
            store.running_runs_on_box(ids::BOX).await.expect("read"),
            base,
            "a parked run holds no slot"
        );
        assert_eq!(
            store.active_runs_on_box(ids::BOX).await.expect("read"),
            active,
            "active_runs_on_box still counts it"
        );
    }
}
