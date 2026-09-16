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
    Agent, AgentBox, AgentId, AgentSummary, AppUser, BoundSkill, BoxId, BoxInfo, BoxProfile,
    BoxRow, BoxTool, ChatRunSpec, Document, DocumentHead, DocumentId, Item, ItemFilter, ItemId,
    ItemKind, ItemKindId, ItemKindPatch, ItemLink, ItemPatch, ItemRevision, ItemSummary, LinkEdge,
    LinkGraph, LinkKind, LinkNode, NewItem, NewItemKind, NewProject, NewRepo, NewStepGraph,
    NewWorkspace, Note, PhaseId, PhasePatch, Project, ProjectId, ProjectPatch, ProjectRef,
    PromptScope, PromptTemplate, Repo, RepoBoxPath, RepoId, RepoPatch, Run, RunId, RunKind,
    RunMode, RunStatus, RunStep, RunStepSummary, RunSummary, Scope, SessionEvent, Skill,
    SkillBinding, SkillId, SkillVersion, Status, StepGraph, StepGraphId, StepGraphPatch,
    StepGraphPhase, StepId, StepStatus, UpstreamEntry, UserId, Workspace, WorkspaceBoxPath,
    WorkspaceId, WorkspacePatch, WorkspaceProject, WorkspaceSummary, prompt_summary,
};
use crate::prompt::settings::{SettingKey, rung_refusal, validate};
use crate::prompt::template::TemplateRole;
use crate::store::error::{Result, StoreError};
use crate::store::traits::{
    CasOutcome, DeleteReach, DeleteTarget, ReadStore, SettingRung, StoredSetting, UpdateOutcome,
    WriteStore, chat_step_status, graph_not_in_project, invalid_prefix, item_kind_is_held,
    not_a_terminal_status, reserved_phase_name,
};

/// The store the TUI runs against in MOD-1: every row in process memory, cloned out under a lock
/// that is never held across an `.await` (plan D6).
#[derive(Debug, Clone, Default)]
pub struct MemStore {
    state: Arc<RwLock<State>>,
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
    /// `session_event`.
    events: Vec<SessionEvent>,
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
            events: data.events,
        };
        Self {
            state: Arc::new(RwLock::new(state)),
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
        if item.status != from {
            return Ok(false);
        }
        item.status = to;
        item.updated_at = now;
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

    /// A project with `settings = {}` and no secret provider (D9).
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
        let row = Project {
            id: new.id,
            slug: new.slug,
            name: new.name,
            description: new.description,
            secret_provider: None,
            secret_scope: None,
            settings: Value::Object(serde_json::Map::new()),
            created_by: new.created_by,
            created_at: now,
            updated_at: now,
        };
        self.projects.insert(row.id, row.clone());
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
                let token = expected_on_row(expected, key, "project")?;
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
                    return Err(StoreError::Constraint(format!(
                        "project.settings of `{id}` is not a JSON object, so `{key}` cannot be \
                         merged into it"
                    )));
                };
                map.insert(name.to_owned(), value.clone());
                project.updated_at = now;
                Ok(CasOutcome::Applied(StoredSetting {
                    value: Some(value),
                    updated_at: now,
                }))
            }
            SettingRung::Phase(id) => {
                let token = expected_on_row(expected, key, "step_graph_phase")?;
                let stored =
                    self.stored_setting(rung, key)
                        .ok_or_else(|| StoreError::NotFound {
                            entity: "step_graph_phase",
                            id: id.to_string(),
                        })?;
                if stored.updated_at != token {
                    return Ok(CasOutcome::Stale(stored));
                }
                let budget = value
                    .as_i64()
                    .and_then(|number| i32::try_from(number).ok())
                    .expect("validate narrows the phase rung to i32::MAX (flag C)");
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
                if let Some(map) = project.settings.as_object_mut() {
                    map.remove(name);
                }
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
    /// lists kept in step by hand. `phase_agents` and `run_step_commits` are `0` because this store
    /// holds neither table, and `workspace_box_paths` because a project is not a workspace.
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
            run_step_commits: 0,
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
    fn delete_project(&mut self, id: ProjectId) -> Result<DeleteReach> {
        let (reach, gone) = self.project_reach(id).ok_or_else(|| StoreError::NotFound {
            entity: "project",
            id: id.to_string(),
        })?;
        self.events
            .retain(|row| !gone.steps.contains(&row.run_step_id));
        self.steps.retain(|id, _| !gone.steps.contains(id));
        self.runs.retain(|id, _| !gone.runs.contains(id));
        self.documents
            .retain(|row| !gone.items.contains(&row.item_id));
        self.notes.retain(|row| !gone.items.contains(&row.item_id));
        self.revisions
            .retain(|(item, _), _| !gone.items.contains(item));
        self.links.retain(|row| {
            !gone.items.contains(&row.from_item_id) && !gone.items.contains(&row.to_item_id)
        });
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
}

/// A revision author must name a real `app_user` row (§5.5 `REFERENCES app_user(id)`); the nil
/// UUID is what `UserId::default()` yields, so it is rejected here rather than written and later
/// refused by MOD-6's `PgStore`. An author that is non-nil but unknown is out of scope: the
/// default [`MemStore::new`] holds no users at all (plan D7).
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
}

/// A row count as the `u64` [`DeleteReach`] holds, saturating rather than casting: `usize` is
/// never wider than `u64` on a target this ships to, and the `try_from` says so without an `as`.
fn rows(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// The CAS token a rung whose row always exists must be given (D8).
///
/// `expected: None` means "I expect no row", which only the `App` rung can mean: a project and a
/// phase exist before the setting does, so `None` there is misuse rather than an insert — and
/// refusing it is what keeps a caller from treating a missing token as a force-write.
fn expected_on_row(
    expected: Option<DateTime<Utc>>,
    key: SettingKey,
    entity: &str,
) -> Result<DateTime<Utc>> {
    expected.ok_or_else(|| {
        StoreError::Constraint(format!(
            "`{key}` on the {entity} rung needs the row's `updated_at`; `expected: None` is the \
             app_setting insert alone"
        ))
    })
}

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
        self.write(|state| state.delete_project(id))
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::MemStore;
    use crate::fixtures::ids;
    use crate::model::{AgentBox, AgentId, ChatRunSpec, ItemId, RunStatus, StepId, StepStatus};
    use crate::prompt::settings::SettingKey;
    use crate::store::error::StoreError;
    use crate::store::{CasOutcome, ReadStore as _, SettingRung, WriteStore as _};
    use chrono::Utc;
    use serde_json::{Value, json};

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
            &crate::model::Scope {
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
            &crate::model::Scope {
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
            .active_runs(&crate::model::Scope {
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
        assert_eq!(run.kind, crate::model::RunKind::Chat, "run.kind");
        assert_eq!(run.mode, crate::model::RunMode::Manual, "run.mode");
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

        let scope = crate::model::Scope {
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
        let probed_at = Utc::now() - chrono::TimeDelta::hours(3);
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
                .project_settings(crate::model::ProjectId::new())
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
                    .repo_box_paths
                    .iter()
                    .all(|row| state.repos.contains_key(&row.repo_id)),
                "every surviving repo path has a surviving repo"
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
}
