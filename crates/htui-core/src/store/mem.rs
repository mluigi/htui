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

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, PoisonError, RwLock};

use chrono::{DateTime, Utc};

use serde_json::Value;

use crate::model::{
    Agent, AgentBox, AgentId, AgentSummary, AppUser, BoundSkill, BoxId, BoxInfo, BoxProfile,
    BoxRow, BoxTool, ChatRunSpec, Document, DocumentHead, DocumentId, Item, ItemFilter, ItemId,
    ItemKind, ItemKindId, ItemLink, ItemPatch, ItemRevision, ItemSummary, LinkEdge, LinkGraph,
    LinkKind, LinkNode, NewItem, Note, PhaseId, Project, ProjectId, ProjectRef, PromptScope,
    PromptTemplate, Run, RunId, RunKind, RunMode, RunStatus, RunStep, RunStepSummary, RunSummary,
    Scope, SessionEvent, Skill, SkillBinding, SkillId, SkillVersion, Status, StepGraph,
    StepGraphId, StepGraphPhase, StepId, StepStatus, UpstreamEntry, UserId, Workspace, WorkspaceId,
    WorkspaceProject, WorkspaceSummary, prompt_summary,
};
use crate::store::error::{Result, StoreError};
use crate::store::traits::{
    ReadStore, UpdateOutcome, WriteStore, chat_step_status, not_a_terminal_status,
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
    /// `project`.
    projects: HashMap<ProjectId, Project>,
    /// `item_kind`.
    kinds: HashMap<ItemKindId, ItemKind>,
    /// `step_graph`. Held for the modules that read it: no §6.1 method exposes it yet (blueprint B.8).
    #[expect(dead_code, reason = "loaded now, read by MOD-2 / MOD-4 / MOD-15")]
    graphs: HashMap<StepGraphId, StepGraph>,
    /// `step_graph_phase`. Held for the modules that read it: no §6.1 method exposes it yet (blueprint B.8).
    #[expect(dead_code, reason = "loaded now, read by MOD-2 / MOD-4 / MOD-15")]
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
    /// `app_setting`, the last rung of the prompt's settings chain (`docs/ANA-5.md` §4.4).
    ///
    /// Empty unless a test sets it: no fixture loads it, and
    /// [`MemStore::set_app_setting`] is its only writer.
    app_settings: BTreeMap<String, Value>,
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
            projects: data.projects.into_iter().map(|row| (row.id, row)).collect(),
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
    /// A `MemStore` loads none of these from the fixture, so this is empty unless a test called
    /// [`MemStore::set_app_setting`]. That is not a gap: plan D101 compiles the defaults into
    /// `prompt::settings::DEFAULTS` precisely because `app_setting` is the one prompt input with
    /// no mirror **and** no generic reader, so an absent row is the normal case, not a failure.
    ///
    /// # Errors
    ///
    /// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
    pub async fn app_settings(&self) -> Result<BTreeMap<String, Value>> {
        Ok(self.read(|state| state.app_settings.clone()))
    }

    /// Writes one `app_setting` row. **Tests only**, and the only writer of that map.
    ///
    /// `app_setting` is seeded by migration `0002` and edited nowhere in the product (MOD-15 owns
    /// any editor), so this exists to let a test drive
    /// [`app_settings`](MemStore::app_settings)'s rung of the settings chain without a Postgres.
    pub fn set_app_setting(&self, key: &str, value: Value) {
        self.write(|state| state.app_settings.insert(key.to_owned(), value));
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
    /// `hops` above 2 is clamped to 2; `hops == 0` returns nothing at all, which is the one place
    /// this differs from `link_graph`'s "hops 0 is the root alone" — the root is the step's own
    /// item and is never an upstream entry.
    ///
    /// `in_scope` and the summary lookup are separate: an out-of-scope item's `summary` is not
    /// read, an in-scope one's is read and may still be `None`.
    fn upstream(&self, root: ItemId, hops: u8, scope: &PromptScope) -> Vec<UpstreamEntry> {
        let hops = hops.min(2);
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
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::MemStore;
    use crate::fixtures::ids;
    use crate::model::{AgentBox, AgentId, ChatRunSpec, ItemId, RunStatus, StepId, StepStatus};
    use crate::store::error::StoreError;
    use crate::store::{ReadStore as _, WriteStore as _};
    use chrono::Utc;
    use serde_json::json;

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
}
