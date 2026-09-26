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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use chrono::{DateTime, Utc};

use serde_json::Value;

use crate::model::{
    Agent, AgentBox, AgentId, AgentSummary, AppUser, BoundSkill, BoxEdit, BoxId, BoxInfo, BoxProbe,
    BoxProfile, BoxRecord, BoxRow, BoxSettings, BoxTool, ChatRunSpec, CitationKind, Claim,
    CommandRun, CommandRunId, CoverageRow, DEFAULT_MAX_CONCURRENT_ITEMS, Document, DocumentHead,
    DocumentId, GateOutcome, Item, ItemCitation, ItemFilter, ItemId, ItemKind, ItemKindId,
    ItemKindPatch, ItemLink, ItemPatch, ItemRequirement, ItemRevision, ItemSummary, LinkEdge,
    LinkGraph, LinkKind, LinkNode, NewCommandRun, NewDocument, NewItem, NewItemKind, NewNote,
    NewProject, NewRepo, NewRequirement, NewRequirementArea, NewRun, NewRunStep, NewStepGraph,
    NewWorkspace, Note, PhaseAgent, PhaseId, PhasePatch, Project, ProjectId, ProjectPatch,
    ProjectRef, PromptScope, PromptTemplate, PromptTemplateId, Repo, RepoBoxPath, RepoId,
    RepoPatch, Requirement, RequirementArea, RequirementAreaId, RequirementFilter, RequirementId,
    RequirementPatch, RequirementRevision, RequirementSpec, RequirementState, RequirementUpdate,
    Resolution, ResolvedGraph, ResolvedInput, ResolvedPhase, Run, RunId, RunKind, RunMode,
    RunStatus, RunStep, RunStepCommit, RunStepSummary, RunStepTree, RunSummary, Scope,
    SessionEvent, Skill, SkillBinding, SkillId, SkillVersion, Status, StepGraph, StepGraphId,
    StepGraphPatch, StepGraphPhase, StepId, StepOutcome, StepStatus, UpstreamEntry, UserId,
    Workspace, WorkspaceBoxPath, WorkspaceId, WorkspacePatch, WorkspaceProject, WorkspaceSummary,
    overlaps, prompt_summary, scope_of,
};
use crate::prompt::DEFAULT_TEMPLATES;
use crate::prompt::settings::{SettingKey, rung_refusal, validate};
use crate::prompt::template::TemplateRole;
use crate::seed;
use crate::store::error::{Result, StoreError};
use crate::store::traits::{
    CasOutcome, DeleteReach, DeleteTarget, ReadStore, SettingRung, StoredSetting, UpdateOutcome,
    WriteStore, already_exists, chat_step_status, citation_key, close_out_needs_a_summary,
    expected_on_row, failure_disagrees_with_status, finish_run_item_mirror,
    finish_run_needs_a_terminal_status, graph_not_in_project, invalid_area_code, invalid_prefix,
    item_has_a_live_run, item_kind_is_held, item_not_in_project, legal_move,
    not_a_fanout_candidate, not_a_terminal_status, references_no_row, requirement_withdrawn,
    reserved_phase_name, resolution_not_closable, row_names_another_step, run_is_terminal,
    step_is_not_promotable, step_slot_is_taken, summary_names_another_item, winner_is_not_settled,
    withdrawn_requirement_cited,
};
use uuid::Uuid;

/// The store the TUI runs against in MOD-1: every row in process memory, cloned out under a lock
/// that is never held across an `.await` (plan D6).
#[derive(Debug, Clone, Default)]
pub struct MemStore {
    state: Arc<RwLock<State>>,
    /// The writes [`MemStore::set_fault`] switched on, shared through clones as `state` is.
    #[cfg(feature = "test-support")]
    faults: Arc<RwLock<HashSet<MemFault>>>,
}

/// MOD-4 plan D152: a write [`MemStore::set_fault`] can make fail, so a test can tell "the store
/// cannot answer" apart from "the row is gone".
///
/// A test seam only: nothing in production switches one on. A faulted write answers
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
    /// `prompt_template`, read by the inherent [`MemStore::prompt_templates`] (MOD-2 plan D102).
    templates: Vec<PromptTemplate>,
    /// `skill`, read by the inherent [`MemStore::bound_skills`] (MOD-2 plan D105).
    skills: HashMap<SkillId, Skill>,
    /// `skill_version`, resolved through [`SkillBinding::version_in_force`].
    skill_versions: Vec<SkillVersion>,
    /// `skill_binding`, collapsed through [`BoundSkill::collapse`].
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
    /// `run.lease_owner`, which is deliberately not a [`Run`] field: the mirror does not carry it
    /// and a reader has no use for another process's liveness token (blueprint F-S). Kept beside
    /// the run so [`WriteStore::refresh_lease`] can compare against it.
    lease_owners: HashMap<RunId, Uuid>,
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
            templates: data.templates,
            skills: data.skills.into_iter().map(|row| (row.id, row)).collect(),
            skill_versions: data.skill_versions,
            skill_bindings: data.skill_bindings,
            box_tools: data.box_tools,
            box_probe_digests: HashMap::new(),
            app_settings: BTreeMap::new(),
            agents: data.agents.into_iter().map(|row| (row.id, row)).collect(),
            agent_boxes: HashMap::new(),
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
            lease_owners: HashMap::new(),
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

    /// The skills in force for a project, or for one phase of it: `R-SKL-2`'s collapse
    /// (`docs/ANA-5.md` §4.2), already resolved to a version and a body.
    ///
    /// `phase: None` asks for the project-level bindings alone. With a phase, the phase's bindings
    /// override the project's per `skill_id` and
    /// [`BoundSkill::collapse`](crate::model::BoundSkill::collapse) is what says so — this method
    /// resolves rows and calls that, exactly as `PgStore`'s two `SELECT`s do, so the rule has one
    /// definition rather than one per backend.
    ///
    /// A binding whose version cannot be resolved is dropped rather than rendered bodiless: see
    /// [`SkillBinding::version_in_force`](crate::model::SkillBinding::version_in_force) for why a
    /// pin that names no row resolves to nothing.
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
            let level = |want: Option<PhaseId>| -> Vec<BoundSkill> {
                state
                    .skill_bindings
                    .iter()
                    .filter(|binding| binding.project_id == project && binding.phase_id == want)
                    .filter_map(|binding| state.bind(binding))
                    .collect()
            };
            BoundSkill::collapse(
                level(None),
                phase.map(|id| level(Some(id))).unwrap_or_default(),
            )
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
        let now = Utc::now();
        self.write(|state| state.app_settings.insert(key.to_owned(), (value, now)));
    }

    /// Replaces one project's `settings` blob without validation. **Tests only** — same reason as
    /// [`set_app_setting`](Self::set_app_setting): no seam writer reaches `project.settings`
    /// (`update_project` never touches the column), and a harness built on a finished store
    /// cannot rebuild it to plant, say, a `judge_agent_id` (plan D69).
    ///
    /// A project that is not there is left alone, as a no-op.
    pub fn set_project_settings(&self, project: ProjectId, settings: Value) {
        let now = Utc::now();
        self.write(|state| {
            if let Some(row) = state.projects.get_mut(&project) {
                row.settings = settings;
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

    /// A phase's candidate agents in `position` order. Always empty here: `phase_agent` is a table
    /// this store does not hold, and the snapshot builder falls back to
    /// `project.settings.default_agent_id` when a phase has no candidate.
    ///
    /// # Errors
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
        let _ = phase;
        Ok(Vec::new())
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
    /// `probed_tags ∪ declared_tags`. Ordered as [`ReadStore::items`] orders.
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
            state
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
                .collect()
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
            let capabilities = state.box_capabilities(box_id);
            let mut missing: Vec<String> = required
                .into_iter()
                .filter(|tag| !capabilities.contains(tag))
                .collect();
            missing.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            missing.dedup();
            Ok(missing)
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

    /// One binding resolved to the skill, the version in force and the body (`R-SKL-2`).
    ///
    /// `None` when the skill row or the version in force is missing, which is
    /// [`SkillBinding::version_in_force`]'s "a pin that cannot be honoured renders nothing".
    fn bind(&self, binding: &SkillBinding) -> Option<BoundSkill> {
        let skill = self.skills.get(&binding.skill_id)?;
        let version = binding.version_in_force(&self.skill_versions)?;
        Some(BoundSkill {
            skill_id: skill.id,
            name: skill.name.clone(),
            version: version.version,
            position: binding.position,
            body: version.body.clone(),
        })
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
            })
            .collect()
    }

    /// Appends events, skipping every `(run_step_id, seq)` already stored, and answers how many
    /// rows landed (§4.3).
    ///
    /// Every event is validated **before** the first insert, because Postgres does the whole batch
    /// in one statement: a batch naming a step that does not exist must write none of its rows.
    fn append_events(&mut self, events: &[SessionEvent]) -> Result<usize> {
        for event in events {
            if !self.steps.contains_key(&event.run_step_id) {
                return Err(StoreError::Constraint(format!(
                    "session_event.run_step_id `{}` references no run_step",
                    event.run_step_id
                )));
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
        row.prompt_digest = Some(digest.to_owned());
        row.trim_record = Some(trim.clone());
        row.updated_at = now;
        Ok(())
    }

    /// Insert-or-update on `agent.id`, with `agent.name` unique across every other id (§5.7).
    ///
    /// `created_at` is the stored row's on an update, never the caller's, and `updated_at` is the
    /// clock: that is what Postgres's `BEFORE UPDATE` trigger does, written out.
    fn upsert_agent(&mut self, agent: &Agent, now: DateTime<Utc>) -> Result<()> {
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
        match self.agents.get_mut(&agent.id) {
            Some(stored) => {
                let created_at = stored.created_at;
                *stored = agent.clone();
                stored.created_at = created_at;
                stored.updated_at = now;
            }
            None => {
                self.agents.insert(agent.id, agent.clone());
            }
        }
        Ok(())
    }

    /// Insert-or-update on the composite primary key `(agent_id, box_id)`, both referents required
    /// (§5.7).
    ///
    /// `quota` and `quota_at` are written by neither branch (MOD-2 plan D74): the insert forces
    /// them to `None`, which is the two columns the `INSERT` list no longer names, and the update
    /// puts the stored pair back, which is the `SET quota = EXCLUDED.quota` the statement no longer
    /// carries. The two backends have to agree here or the conformance case passes on one and
    /// fails on the other.
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
        match self.agent_boxes.get_mut(&(row.agent_id, row.box_id)) {
            Some(stored) => {
                // D74: `*stored = row.clone()` is this backend's spelling of
                // `SET quota = EXCLUDED.quota`, so the stored pair is taken out and put back.
                let (quota, quota_at) = (stored.quota.clone(), stored.quota_at);
                *stored = row.clone();
                stored.quota = quota;
                stored.quota_at = quota_at;
                stored.updated_at = now;
            }
            None => {
                // The two columns the `INSERT` list does not name.
                let fresh = AgentBox {
                    quota: None,
                    quota_at: None,
                    ..row.clone()
                };
                self.agent_boxes.insert((row.agent_id, row.box_id), fresh);
            }
        }
        Ok(())
    }

    /// The two-column quota latch of `docs/ANA-4.md` §7 (plan D67): an existing row only, with
    /// `updated_at` bumped as `set_step_usage` bumps it and Postgres's `BEFORE UPDATE` trigger
    /// does it there.
    fn set_agent_box_quota(
        &mut self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let row = self
            .agent_boxes
            .get_mut(&(agent_id, box_id))
            .ok_or_else(|| StoreError::NotFound {
                entity: "agent_box",
                id: format!("{agent_id}/{box_id}"),
            })?;
        row.quota = Some(quota);
        row.quota_at = Some(quota_at);
        row.updated_at = now;
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
            phase_name: "chat".to_owned(),
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

    /// Inserts or replaces this box's checkout path (`R-BOX-4`).
    fn upsert_repo_box_path(&mut self, path: &RepoBoxPath, now: DateTime<Utc>) -> Result<()> {
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
            is_override: false,
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
        let mut row = phase.clone();
        row.updated_at = now;
        self.phases.push(row.clone());
        Ok(row)
    }

    /// Compare-and-set on the phase's `updated_at` over [`PhasePatch`]'s five columns;
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
    /// lists kept in step by hand. `phase_agents` is `0` because this store holds no such table,
    /// and `workspace_box_paths` because a project is not a workspace; `run_step_commits` and
    /// `run_step_trees` were `0` for the same reason until MOD-4 milestone 1 gave this store the
    /// two maps (plan D12), and `command_runs` until milestone 3 gave it the third (plan D31).
    /// MOD-38's six requirement counts come from the same pass (blueprint F1, §4.4).
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
            phase_agents: 0,
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
                    .filter(|row| row.project_id == id)
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
        self.steps.retain(|id, _| !gone.steps.contains(id));
        self.runs.retain(|id, _| !gone.runs.contains(id));
        self.lease_owners.retain(|id, _| !gone.runs.contains(id));
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
        self.skill_bindings.retain(|row| row.project_id != id);
        self.kinds.retain(|_, row| row.project_id != id);
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

    /// `ORDER BY (s.run_id = $run) DESC NULLS LAST` spelled out: this run's output, then another
    /// run's, then a document no step produced. Postgres sorts the `NULL` of the outer join last,
    /// and a `produced_by_step_id` whose step is gone joins to the same `NULL`.
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

    /// One kind of ANA-2 §4.2's resolver: the eligible documents ranked, best first.
    fn resolve_input(&self, item: ItemId, run: RunId, kind: &str) -> Option<Document> {
        self.documents
            .iter()
            .filter(|document| document.item_id == item && document.kind == kind)
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
            })
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
            queued_at: new.queued_at,
            started_at: None,
            finished_at: None,
            failure: None,
            repo_scope: new.repo_scope,
            lease_box_id: None,
            lease_expires_at: None,
            updated_at: now,
        };
        self.runs.insert(row.id, row.clone());
        self.transition(new.item_id, status, Status::Queued, now)?;
        Ok(row)
    }

    /// ANA-2 §4.7's admission, decided before the first write so a refusal writes nothing.
    fn claim_run(
        &mut self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
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
            row.lease_expires_at = Some(lease_until);
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
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.require_run(run)?;
        if self.lease_owners.get(&run) != Some(&owner) {
            return Ok(false);
        }
        if let Some(row) = self.runs.get_mut(&run) {
            row.lease_expires_at = Some(until);
            row.updated_at = now;
        }
        Ok(true)
    }

    /// ANA-2 §4.9's sweep: every abandoned lease on the box that is not already `owner`'s becomes
    /// `owner`'s (plan D88).
    fn adopt_runs(
        &mut self,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Vec<Run> {
        let mut abandoned: Vec<(DateTime<Utc>, RunId)> = self
            .runs
            .values()
            .filter(|row| {
                row.status == RunStatus::Running
                    && row.executing_box_id == Some(box_id)
                    && row.lease_expires_at.is_none_or(|until| until <= at)
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
                row.lease_expires_at = Some(lease_until);
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
        at: DateTime<Utc>,
        until: DateTime<Utc>,
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
            && (ours_or_free || row.lease_expires_at.is_none_or(|expiry| expiry <= at));
        if !takeable {
            return Ok(false);
        }
        row.lease_box_id = Some(box_id);
        row.lease_expires_at = Some(until);
        row.updated_at = now;
        self.lease_owners.insert(run, owner);
        Ok(true)
    }

    /// Plan D139: a compare-and-set on `lease_owner` that clears it; `false` writes nothing.
    fn release_lease(
        &mut self,
        run: RunId,
        owner: Uuid,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.require_run(run)?;
        if self.lease_owners.get(&run) != Some(&owner) {
            return Ok(false);
        }
        self.lease_owners.remove(&run);
        if let Some(row) = self.runs.get_mut(&run) {
            row.lease_expires_at = Some(at);
            row.updated_at = now;
        }
        Ok(true)
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
    fn check_step_batch(
        &self,
        table: &str,
        step: StepId,
        rows: impl IntoIterator<Item = (StepId, RepoId)>,
    ) -> Result<()> {
        self.require_step(step)?;
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
        step: StepId,
        trees: &[RunStepTree],
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.check_step_batch(
            "run_step_tree",
            step,
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
    fn record_commits(&mut self, step: StepId, commits: &[RunStepCommit]) -> Result<()> {
        self.check_step_batch(
            "run_step_commit",
            step,
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
                    // `phase_agent` is not a table this store holds (blueprint F-N).
                    agents: Vec::new(),
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

/// The refusal both writers of `project.settings` give a blob that is not a JSON object (D7).
///
/// One sentence rather than two: `set_setting` cannot merge a key into a scalar and `clear_setting`
/// cannot remove one from it, and what stops both is the same fact — the document is not a
/// document. A wording per verb would be two sentences about one blob, which is the drift the text
/// helpers in [`store::traits`](crate::store::traits) exist to prevent (review L4).
///
/// Private, because `PgStore` cannot reach it: `project.settings` is `JSONB NOT NULL DEFAULT '{}'`
/// there and the merge is Postgres's own `||`.
fn settings_not_an_object(id: ProjectId, key: SettingKey) -> String {
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
}

impl WriteStore for MemStore {
    async fn mint_item(&self, new: NewItem) -> Result<Item> {
        let now = Utc::now();
        self.write(|state| state.mint(new, now))
    }

    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        let now = Utc::now();
        self.write(|state| state.update(id, expected_version, patch, now))
    }

    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::ItemTransition)?;
        let now = Utc::now();
        self.write(|state| state.transition(id, from, to, now))
    }

    async fn append_events(&self, events: &[SessionEvent]) -> Result<usize> {
        self.write(|state| state.append_events(events))
    }

    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.set_step_usage(step, usage, prompt_digest, now))
    }

    async fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.upsert_agent(agent, now))
    }

    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.upsert_agent_box(row, now))
    }

    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.set_agent_box_quota(agent_id, box_id, quota, quota_at, now))
    }

    async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.record_box_probe(probe, now))
    }

    async fn boxes(&self) -> Result<Vec<BoxRecord>> {
        let user = self.this_user();
        Ok(self.read(|state| state.box_records(user)))
    }

    async fn edit_box(
        &self,
        _id: BoxId,
        _expected: i32,
        _edit: BoxEdit,
    ) -> Result<CasOutcome<BoxRow>> {
        todo!("MOD-7 milestone 2 T1: MemStore::edit_box")
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
        let now = Utc::now();
        self.write(|state| state.finish_chat_run(run, step, status, finished_at, now))
    }

    async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.set_step_prompt(step, digest, trim, now))
    }

    // MOD-15 milestone 1. Each arm takes the lock and hands one `State` method the store's clock;
    // every rule that can refuse is in `impl State`, so nothing here can disagree with `PgStore`
    // about what is legal, only about how it is stored.

    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace> {
        let now = Utc::now();
        self.write(|state| state.create_workspace(new, now))
    }

    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> Result<CasOutcome<Workspace>> {
        let now = Utc::now();
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
        let now = Utc::now();
        self.write(|state| state.upsert_workspace_box_path(path, now))
    }

    async fn workspace_box_paths(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceBoxPath>> {
        Ok(self.read(|state| state.workspace_box_path_rows(workspace)))
    }

    async fn create_project(&self, new: NewProject) -> Result<Project> {
        let now = Utc::now();
        self.write(|state| state.create_project(new, now))
    }

    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> Result<CasOutcome<Project>> {
        let now = Utc::now();
        self.write(|state| state.update_project(id, expected, patch, now))
    }

    async fn create_repo(&self, new: NewRepo) -> Result<Repo> {
        let now = Utc::now();
        self.write(|state| state.create_repo(new, now))
    }

    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> Result<CasOutcome<Repo>> {
        let now = Utc::now();
        self.write(|state| state.update_repo(id, expected, patch, now))
    }

    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>> {
        Ok(self.read(|state| state.repo_rows(project)))
    }

    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.upsert_repo_box_path(path, now))
    }

    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        Ok(self.read(|state| state.repo_box_path_rows(repo)))
    }

    async fn create_item_kind(&self, new: NewItemKind) -> Result<ItemKind> {
        let now = Utc::now();
        self.write(|state| state.create_item_kind(new, now))
    }

    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> Result<CasOutcome<ItemKind>> {
        let now = Utc::now();
        self.write(|state| state.update_item_kind(id, expected, patch, now))
    }

    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
        Ok(self.read(|state| state.item_kind_rows(project)))
    }

    async fn delete_item_kind(&self, id: ItemKindId) -> Result<()> {
        self.write(|state| state.delete_item_kind(id))
    }

    async fn create_step_graph(&self, new: NewStepGraph) -> Result<StepGraph> {
        let now = Utc::now();
        self.write(|state| state.create_step_graph(new, now))
    }

    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> Result<CasOutcome<StepGraph>> {
        let now = Utc::now();
        self.write(|state| state.update_step_graph(id, expected, patch, now))
    }

    async fn step_graphs(&self, project: ProjectId) -> Result<Vec<StepGraph>> {
        Ok(self.read(|state| state.step_graph_rows(project)))
    }

    async fn create_phase(&self, phase: &StepGraphPhase) -> Result<StepGraphPhase> {
        let now = Utc::now();
        self.write(|state| state.create_phase(phase, now))
    }

    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> Result<CasOutcome<StepGraphPhase>> {
        let now = Utc::now();
        self.write(|state| state.update_phase(id, expected, patch, now))
    }

    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        Ok(self.read(|state| state.phase_rows(graph)))
    }

    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<StoredSetting>> {
        let now = Utc::now();
        self.write(|state| state.set_setting(rung, key, value, expected, now))
    }

    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>> {
        let now = Utc::now();
        self.write(|state| state.clear_setting(rung, key, expected, now))
    }

    async fn setting(&self, rung: SettingRung, key: SettingKey) -> Result<Option<StoredSetting>> {
        if let Some(refusal) = rung_refusal(key, rung.flag()) {
            return Err(StoreError::Constraint(refusal));
        }
        Ok(self.read(|state| state.stored_setting(rung, key)))
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
        let now = Utc::now();
        self.write(|state| state.delete_project(id, now))
    }

    // MOD-4 milestones 1 and 2, in ANA-2 §8's order. `create_run`, `claim_run`, `select_fanout`,
    // `write_document`, `promote_step`, `finish_run` and `close_out` are each a **single** `write`
    // closure, so plan M1 D6's and M2 D7's seven transactions are atomic here by construction and
    // not by discipline.

    async fn create_run(&self, new: NewRun) -> Result<Run> {
        let now = Utc::now();
        self.write(|state| state.create_run(new, now))
    }

    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Claim> {
        let now = Utc::now();
        self.write(|state| state.claim_run(run, box_id, owner, at, lease_until, now))
    }

    async fn refresh_lease(&self, run: RunId, owner: Uuid, until: DateTime<Utc>) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::RefreshLease)?;
        let now = Utc::now();
        self.write(|state| state.refresh_lease(run, owner, until, now))
    }

    async fn adopt_runs(
        &self,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Vec<Run>> {
        let now = Utc::now();
        Ok(self.write(|state| state.adopt_runs(box_id, owner, at, lease_until, now)))
    }

    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<bool> {
        let stamp = Utc::now();
        self.write(|state| state.take_lease(run, box_id, owner, now, until, stamp))
    }

    async fn release_lease(&self, run: RunId, owner: Uuid, now: DateTime<Utc>) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::ReleaseLease)?;
        let stamp = Utc::now();
        self.write(|state| state.release_lease(run, owner, now, stamp))
    }

    async fn create_step(&self, new: NewRunStep) -> Result<RunStep> {
        let now = Utc::now();
        self.write(|state| state.create_step(new, now))
    }

    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = Utc::now();
        self.write(|state| state.transition_run(run, from, to, at, now))
    }

    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = Utc::now();
        self.write(|state| state.transition_step(step, from, to, at, now))
    }

    async fn finish_step(&self, step: StepId, outcome: StepOutcome) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.finish_step(step, outcome, now))
    }

    async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool> {
        let now = Utc::now();
        self.write(|state| state.interrupt_step(step, note, at, now))
    }

    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let now = Utc::now();
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
        let now = Utc::now();
        self.write(|state| state.select_fanout(run, position, attempt, winner, reason, now))
    }

    async fn supersede_step(&self, step: StepId) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.supersede_step(step, now))
    }

    async fn upsert_step_tree(&self, step: StepId, trees: &[RunStepTree]) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.upsert_step_tree(step, trees, now))
    }

    async fn record_commits(&self, step: StepId, commits: &[RunStepCommit]) -> Result<()> {
        self.write(|state| state.record_commits(step, commits))
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
        let now = Utc::now();
        self.write(|state| state.promote_step(step, at, now))
    }

    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.fail_run(run, failure, at, now))
    }

    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.finish_run(run, to, failure, at, now))
    }

    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document> {
        let now = Utc::now();
        self.write(|state| state.close_out(item, resolution, summary, commits, now))
    }

    async fn add_note(&self, note: NewNote) -> Result<Note> {
        self.write(|state| state.add_note(note))
    }

    // ---- ANA-11 §5.1 (MOD-38): requirements and citations, one `write` closure each ----

    async fn set_requirement_spec(
        &self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
    ) -> Result<CasOutcome<RequirementSpec>> {
        let now = Utc::now();
        self.write(|state| {
            state.set_requirement_spec(project, expected_version, owner_id, preamble, now)
        })
    }

    async fn create_requirement_area(&self, new: NewRequirementArea) -> Result<RequirementArea> {
        let now = Utc::now();
        self.write(|state| state.create_requirement_area(new, now))
    }

    async fn mint_requirement(
        &self,
        area: RequirementAreaId,
        new: NewRequirement,
    ) -> Result<Requirement> {
        let now = Utc::now();
        self.write(|state| state.mint_requirement(area, new, now))
    }

    async fn amend_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        amended_by: ItemId,
    ) -> Result<RequirementUpdate> {
        let now = Utc::now();
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
        let now = Utc::now();
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
        let now = Utc::now();
        self.write(|state| state.cite(item, requirement, kind, proposed_by, now))
    }

    async fn uncite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<()> {
        let now = Utc::now();
        self.write(|state| state.uncite(item, requirement, kind, now))
    }

    async fn reconfirm(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<ItemRequirement> {
        let now = Utc::now();
        self.write(|state| state.reconfirm(item, requirement, kind, now))
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::MemStore;
    use crate::fixtures::ids;
    use crate::model::{
        AgentBox, AgentId, BoxId, BoxProbe, ChatRunSpec, CitationKind, Claim, DocumentId,
        GateOutcome, GraphSnapshot, Isolation, ItemId, ItemKindPatch, NewDocument, NewItem,
        NewNote, NewProject, NewRepo, NewRequirement, NewRequirementArea, NewRun, NewRunStep,
        NoteId, OverlapRule, Priority, ProbedTool, ProjectId, RepoId, RequirementAreaId,
        RequirementId, RequirementPatch, RequirementUpdate, Resolution, RunId, RunKind, RunMode,
        RunStatus, RunStepCommit, RunStepTree, Scope, SnapshotGraph, SnapshotSettings, Status,
        StepId, StepOutcome, StepStatus, UserId, VerifyOutcome,
    };
    use crate::prompt::settings::SettingKey;
    use crate::prompt::{DEFAULT_TEMPLATES, body_of};
    use crate::store::error::StoreError;
    use crate::store::{CasOutcome, DeleteTarget, ReadStore as _, SettingRung, WriteStore as _};
    use chrono::{TimeDelta, Utc};
    use serde_json::{Value, json};
    use uuid::Uuid;

    /// MOD-4 plan D152: a switched-on fault answers `Unreachable` on every clone, before the
    /// write looks at a row, and the write answers as before once it is switched off.
    #[tokio::test]
    async fn a_switched_on_fault_answers_unreachable_until_switched_off() {
        let store = MemStore::demo();
        let clone = store.clone();
        let ghost = RunId::new();
        let now = Utc::now();

        store.set_fault(super::MemFault::ReleaseLease, true);
        assert!(
            matches!(
                clone.release_lease(ghost, Uuid::now_v7(), now).await,
                Err(StoreError::Unreachable(_))
            ),
            "the clone shares the switch, and the missing row is never looked up"
        );
        assert!(
            matches!(
                clone.refresh_lease(ghost, Uuid::now_v7(), now).await,
                Err(StoreError::NotFound { .. })
            ),
            "only the named write fails"
        );

        store.set_fault(super::MemFault::ReleaseLease, false);
        assert!(
            matches!(
                clone.release_lease(ghost, Uuid::now_v7(), now).await,
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

    /// The columns `store::conformance` cannot see, because §6.1 returns neither `run_step.usage`
    /// nor `run_step.prompt_digest` (plan D15(a)). Read straight out of `State`, which is what a
    /// unit test in this module is for; `pg_criteria.rs` asserts the same rule in SQL.
    #[tokio::test]
    async fn set_step_usage_writes_usage_every_time_and_the_digest_only_when_supplied() {
        let store = MemStore::demo();
        store
            .set_step_usage(
                ids::STEP_IMPL,
                json!({ "input_tokens": 7 }),
                Some("abc".to_owned()),
            )
            .await
            .expect("the first write lands");
        store
            .set_step_usage(ids::STEP_IMPL, json!({ "input_tokens": 9 }), None)
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
            .set_step_usage(ids::STEP_IMPL, json!({ "input_tokens": 7 }), None)
            .await
            .expect("the usage write lands");
        store
            .set_step_prompt(
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
            .set_step_prompt(StepId::new(), "9f8e", &json!({}))
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
            vec![("tests", 1, 0), ("rust-style", 2, 1)],
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
            vec![("tests", 1, 0), ("rust-style", 1, 2)],
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

    /// The chat pair of plan D4: the columns `pending.rs` writes, with `running` in place of the
    /// upload's terminal values, and `finish_chat_run` closing both rows.
    #[tokio::test]
    async fn a_chat_run_mints_the_two_rows_the_offline_upload_would() {
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
        store
            .set_agent_box_quota(ids::AGENT_CLAUDE, ids::BOX, quota.clone(), quota_at)
            .await
            .expect("the latch lands on the probed row");

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
        store.delete_project(gone).await.expect("the delete lands");

        store.read(|state| {
            assert!(!state.projects.contains_key(&gone), "the project itself");
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
                    .all(|row| row.project_id != gone),
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
                    && !state.agents.is_empty(),
                "skill, skill_version, app_user, box, box_tool and agent are not below a project"
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

    /// ANA-2 §4.7's admission: the repo-scope overlap refuses before the slot count does, and the
    /// slot count is `box.settings.max_concurrent_items`.
    #[tokio::test]
    async fn claim_run_refuses_an_overlapping_scope_and_a_full_box() {
        let store = MemStore::demo();
        let repo = a_repo(&store, "core").await;
        let owner = Uuid::now_v7();
        let at = Utc::now();
        let until = at + TimeDelta::minutes(5);

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
                .claim_run(first, ids::BOX, owner, at, until)
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
                .claim_run(second, ids::BOX, owner, at, until)
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
                .claim_run(third, ids::BOX, owner, at, until)
                .await
                .expect("the claim is answered"),
            Claim::Admitted,
            "an empty scope overlaps nothing"
        );
        assert_eq!(
            store
                .claim_run(fourth, ids::BOX, owner, at, until)
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
                .claim_run(first, ids::BOX, owner, at, until)
                .await
                .expect("the claim is answered"),
            Claim::NotClaimable,
            "a run that is not queued is not claimable"
        );

        assert!(
            matches!(
                store
                    .claim_run(second, BoxId::new(), owner, at, until)
                    .await,
                Err(StoreError::NotFound { entity: "box", .. })
            ),
            "the box is looked up after the run"
        );
        assert!(
            matches!(
                store
                    .claim_run(RunId::new(), BoxId::new(), owner, at, until)
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

    /// ANA-2 §4.9: the heartbeat is a compare-and-set on `lease_owner`, and the sweep takes an
    /// expired lease from whoever held it.
    #[tokio::test]
    async fn a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it() {
        let store = MemStore::demo();
        let first_owner = Uuid::now_v7();
        let second_owner = Uuid::now_v7();
        let at = Utc::now();
        let until = at + TimeDelta::minutes(5);

        let run = store
            .create_run(graph_run(ids::HTUI_ANA_2, ids::PROJECT_HTUI, Vec::new()))
            .await
            .expect("the run is queued")
            .id;
        assert_eq!(
            store
                .claim_run(run, ids::BOX, first_owner, at, until)
                .await
                .expect("the claim is answered"),
            Claim::Admitted
        );

        let extended = until + TimeDelta::minutes(5);
        assert!(
            store
                .refresh_lease(run, first_owner, extended)
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
                .refresh_lease(run, second_owner, extended + TimeDelta::minutes(5))
                .await
                .expect("the refresh is answered"),
            "a stranger's heartbeat is zero rows, which means abandon"
        );
        assert!(
            matches!(
                store
                    .refresh_lease(RunId::new(), first_owner, extended)
                    .await,
                Err(StoreError::NotFound { entity: "run", .. })
            ),
            "an unknown run is NotFound"
        );

        let swept = extended + TimeDelta::minutes(5);
        assert!(
            store
                .adopt_runs(
                    ids::BOX,
                    second_owner,
                    extended - TimeDelta::seconds(1),
                    swept
                )
                .await
                .expect("the sweep is answered")
                .is_empty(),
            "a live lease is not abandoned"
        );
        let adopted = store
            .adopt_runs(
                ids::BOX,
                second_owner,
                extended + TimeDelta::seconds(1),
                swept,
            )
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
                .refresh_lease(run, first_owner, swept)
                .await
                .expect("the refresh is answered"),
            "the old owner has lost it"
        );
        assert!(
            store
                .refresh_lease(run, second_owner, swept)
                .await
                .expect("the refresh is answered"),
            "the new owner holds it"
        );
        let later = swept + TimeDelta::minutes(5);
        assert!(
            store
                .adopt_runs(ids::BOX, second_owner, swept + TimeDelta::seconds(1), later)
                .await
                .expect("the sweep is answered")
                .is_empty(),
            "a process never adopts its own lease, even expired (plan D88)"
        );
        assert_eq!(
            store
                .adopt_runs(ids::BOX, first_owner, swept + TimeDelta::seconds(1), later)
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
                .adopt_runs(BoxId::new(), second_owner, swept, swept)
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
                .finish_step(StepId::new(), StepOutcome::default())
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
            .upsert_step_tree(ids::STEP_R2_PRD, &[tree(second, false), tree(first, false)])
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
            .upsert_step_tree(ids::STEP_R2_PRD, &[tree(first, true)])
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
                store.upsert_step_tree(ids::STEP_R2_PRD, &[stray]).await,
                Err(StoreError::Constraint(_))
            ),
            "a row for another step is refused"
        );
        assert!(
            matches!(
                store
                    .upsert_step_tree(ids::STEP_R2_PRD, &[tree(RepoId::new(), false)])
                    .await,
                Err(StoreError::Constraint(_))
            ),
            "an unknown repo is a foreign key refusal"
        );
        assert!(matches!(
            store.upsert_step_tree(StepId::new(), &[]).await,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ));
        store
            .upsert_step_tree(ids::STEP_R2_PRD, &[])
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
                ids::STEP_R2_PRD,
                &[commit(second, None), commit(first, None)],
            )
            .await
            .expect("both rows land");
        store
            .record_commits(ids::STEP_R2_PRD, &[commit(first, Some("def"))])
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
            store.record_commits(StepId::new(), &[]).await,
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
    /// kind the item has no eligible row for — none of which `documents_of_kinds` does.
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
            Some(ids::DOC_FEAT_1_PLAN_V2),
            "this run's output outranks a later hand-written version"
        );
        assert_ne!(
            preferred[0].document.as_ref().map(|row| row.id),
            Some(hand.id)
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
            "MemStore holds no phase_agent table"
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
            "no phase_agent table here either"
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
                .claim_run(
                    run,
                    ids::BOX,
                    Uuid::now_v7(),
                    at,
                    at + TimeDelta::minutes(5),
                )
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
}
