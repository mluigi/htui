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

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use chrono::{DateTime, Utc};

use serde_json::Value;

use crate::model::{
    Agent, AgentBox, AgentId, AgentSummary, AppUser, BoxId, BoxInfo, BoxRow, ChatRunSpec, Document,
    DocumentHead, Item, ItemFilter, ItemId, ItemKind, ItemKindId, ItemLink, ItemPatch,
    ItemRevision, ItemSummary, LinkEdge, LinkGraph, LinkKind, LinkNode, NewItem, Note, Project,
    ProjectId, ProjectRef, PromptTemplate, Run, RunId, RunKind, RunMode, RunStatus, RunStep,
    RunStepSummary, RunSummary, Scope, SessionEvent, Status, StepGraph, StepGraphId,
    StepGraphPhase, StepId, StepStatus, UserId, Workspace, WorkspaceId, WorkspaceProject,
    WorkspaceSummary,
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
    /// `prompt_template`. Held for the modules that read it: no §6.1 method exposes it yet (blueprint B.8).
    #[expect(dead_code, reason = "loaded now, read by MOD-2 / MOD-4 / MOD-15")]
    templates: Vec<PromptTemplate>,
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
    fn run_steps(&self, run: RunId) -> Vec<RunStepSummary> {
        let mut steps: Vec<&RunStep> = self
            .steps
            .values()
            .filter(|step| step.run_id == run)
            .collect();
        steps.sort_by_key(|step| (step.position, step.attempt, step.fanout_index));
        steps
            .into_iter()
            .map(|step| RunStepSummary {
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
                *stored = row.clone();
                stored.updated_at = now;
            }
            None => {
                self.agent_boxes
                    .insert((row.agent_id, row.box_id), row.clone());
            }
        }
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
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::MemStore;
    use crate::fixtures::ids;
    use crate::model::{AgentBox, ChatRunSpec, RunStatus, StepStatus};
    use crate::store::WriteStore as _;
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
            vec!["agy", "claude"],
            "ordered by agent.name"
        );
        assert!(
            plain.iter().all(|row| row.on_box.is_none()),
            "no fixture loads agent_box"
        );

        let now = Utc::now();
        store
            .upsert_agent_box(&AgentBox {
                agent_id: ids::AGENT_CLAUDE,
                box_id: ids::BOX,
                enabled: true,
                version: Some("1.2.3".to_owned()),
                path: Some("claude".to_owned()),
                probed_at: Some(now),
                quota: None,
                quota_at: None,
                updated_at: now,
            })
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
        assert!(
            joined
                .iter()
                .find(|row| row.agent.name == "agy")
                .expect("agy is registered")
                .on_box
                .is_none(),
            "an agent with no row for this box stays None"
        );
    }
}
