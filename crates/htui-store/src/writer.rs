//! An owned handle on a writable store (MOD-2 milestone 3, plan D26).
//!
//! [`Backend::writable`](crate::Backend::writable) hands out a `&PgStore`, which is the right
//! shape for a caller that writes and returns. A **recorder** is not that caller: it lives for a
//! whole chat session, inside a task the store worker spawned, and it is generic over
//! `S: WriteStore` — it needs something it can own.
//!
//! [`Writer`] is that something. Both arms are cheap handles — `PgStore` is a pool handle and
//! `MemStore` is an `Arc` — so a `Writer` is a clone of a handle, never a copy of a store.
//!
//! **Online only (MOD-25).** Between MOD-2 milestone 4 and MOD-25 a third arm buffered an offline
//! chat's rows under `<cache_dir>/pending/` for a later upload (plan D34). MOD-25 removed that
//! path: [`Backend::writer`](crate::Backend::writer) answers `None` off the server, where a chat is
//! refused with [`DATABASE_UNREACHABLE`], and [`Backend::writable`](crate::Backend::writable)
//! still answers `None` off the server, so nothing writes to Postgres unless the backend is
//! `Online`. A step write the store refuses is re-offered by the recorder itself, at the same
//! `seq` (`htui_agent::record`, MOD-40 plan D3); no buffer outlives the process.
//!
//! `MemStore` is reachable here and is not through `writable`, deliberately: `--demo` and every
//! chat-tab snapshot run against it, and a seam only the production backend can exercise is a seam
//! no test covers.

use chrono::{DateTime, TimeDelta, Utc};
use htui_core::model::{
    Agent, AgentBox, AgentId, AnswerOutcome, BindingChange, BoxEdit, BoxId, BoxProbe, BoxRecord,
    BoxRow, CancelRequest, ChatRunSpec, CitationKind, Claim, CommandRun, CoverageRow, Document,
    DocumentHead, DocumentId, GateOutcome, Item, ItemCitation, ItemFilter, ItemId, ItemKind,
    ItemKindId, ItemKindPatch, ItemPatch, ItemRequirement, ItemSummary, LinkGraph, NewCommandRun,
    NewDocument, NewItem, NewItemKind, NewNote, NewPersona, NewProject, NewPromptTemplate, NewRepo,
    NewRequirement, NewRequirementArea, NewRun, NewRunStep, NewSkill, NewSkillVersion,
    NewStepGraph, NewWorkspace, Note, OpenPermission, PermissionChoice, PermissionId,
    PermissionStatus, Persona, PersonaId, PersonaPatch, PhaseAgent, PhaseId, PhasePatch, Project,
    ProjectId, ProjectPatch, PromptScope, PromptTemplate, RelaySessionId, RelayView, Repo,
    RepoBoxPath, RepoId, RepoPatch, Requirement, RequirementArea, RequirementAreaId,
    RequirementFilter, RequirementId, RequirementPatch, RequirementRevision, RequirementSpec,
    RequirementUpdate, Resolution, ResolvedInput, Run, RunCommand, RunCommandId, RunCommandStatus,
    RunId, RunStatus, RunStep, RunStepCommit, RunStepTree, RunSummary, Scope, SessionEvent, Skill,
    SkillBinding, SkillBindingKey, SkillId, SkillPatch, SkillVersion, Status, StepGraph,
    StepGraphId, StepGraphPatch, StepGraphPhase, StepId, StepOpening, StepOutcome, StepPermission,
    StepStatus, ToolCallCount, UpstreamEntry, UserId, Workspace, WorkspaceBoxPath, WorkspaceId,
    WorkspacePatch, WorkspaceProject,
};
use htui_core::prompt::settings::SettingKey;
use htui_core::store::{
    CasOutcome, DeleteReach, DeleteTarget, MemStore, ParkOutcome, ReadStore, Result, SettingRung,
    StepFence, StoredSetting, UpdateOutcome, WriteStore,
};
use serde_json::Value;
use uuid::Uuid;

use crate::pg::PgStore;

/// A writable store a caller can hold.
#[derive(Debug, Clone)]
pub enum Writer {
    /// In-process (`--demo`, tests).
    Memory(MemStore),
    /// Postgres.
    Online(PgStore),
}

impl Writer {
    /// The backend label this writer belongs to, for the `Debug` of every task that holds one. Plan
    /// D42 also put it in the chat header, to tell a recorded conversation from one only on this
    /// disk; that went with the offline buffer (MOD-25), and the label left `ChatAccepted` in
    /// CLEAN-7.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            Self::Online(_) => "online",
        }
    }
}

/// D35's refusal for the registry writes, and MOD-2 D52's for a probe that has no server to
/// write its snapshot to: the same sentence in both places, on purpose.
/// A probe costs process spawns, so the caller checks this **before** it spawns anything rather
/// than discovering the refusal on the write. Writing a probe result somewhere local is the
/// local-only store's job (MOD-17), not this seam's.
pub const REGISTRY_ON_SERVER_ONLY: &str = "the agent registry is written on the server only";

/// The one sentence the whole prompt path answers with off the server (MOD-2 plan D109,
/// blueprint E-8).
/// It names **two** facts because they are the same fact from either end. Reading: four of the
/// assembler's inputs — `prompt_template`, `skill`, `skill_version`, `skill_binding`, `box_tool` —
/// have no cache mirror, so a preview cannot be assembled offline at all. Writing:
/// `run_step.trim_record` has nowhere to go offline — since MOD-25 there is no offline write path
/// at all, and the pending buffer before it carried `session_event` columns only.
/// One constant, so a user who meets the refusal from the preview and from a step's audit row
/// reads the same sentence and does not have to decide whether they are two problems.
pub const PROMPT_ON_SERVER_ONLY: &str = "the prompt path needs the server: templates, skills and box tools are not mirrored, and a \
     step's trim record has nowhere to go offline";

/// The one sentence a chat is refused with off the server (MOD-25).
/// `htui` is online-only: since MOD-25 [`Backend::writer`](crate::Backend::writer) answers `None`
/// on [`Backend::Offline`](crate::Backend::Offline), so a chat started on a box whose Postgres is
/// unreachable is **refused** rather than recorded into `<cache_dir>/pending/`. This is the
/// sentence it is refused with, and it echoes `R-STO-4`'s "the TUI opens in offline read-only mode
/// from the cache ... No item creation, no runs": the shell still browses the mirror, with the top
/// bar reading `offline · <age>`, and starts no run.
/// It does **not** name the database itself, because it is carried by
/// `StoreError::Unreachable`, whose `Display` already
/// prefixes `store unreachable: `. Reading the two together is the whole sentence; repeating the
/// word here made the rendered line say "unreachable" twice.
pub const DATABASE_UNREACHABLE: &str = "this box browses its read-only cache and starts no run";

/// [`DATABASE_UNREACHABLE`] for all 31 hierarchy methods (MOD-15 plan D2), reads included.
/// Reusing MOD-25's sentence rather than coining a thirty-second one is the decision, not an
/// economy. Writing: `htui` is online-only, so a workspace rename off the server is the same
/// event `R-STO-4` already words — "offline read-only mode … no item creation". Reading:
/// `workspace`, `repo`, `repo_box_path`, `workspace_box_path`, `step_graph` and
/// `step_graph_phase` have no mirror at all (`cache::MIRRORED_TABLES`), so offline they are
/// unreachable in the plainest sense of the word.
/// [`REGISTRY_ON_SERVER_ONLY`] and [`PROMPT_ON_SERVER_ONLY`] were the other candidates and both
/// name a different subsystem; a maintainer who met "the agent registry is written on the server
/// only" after renaming a project would go looking in the wrong place.
/// **Since MOD-4 milestone 1 (plan D5) it serves a second family**: ANA-2 §8's run seam, eighteen
/// [`WriteStore`] writers and five [`ReadStore`] reads, refused here with the same one sentence and
/// no other. A third constant was not coined for the reason a thirty-second one was not: queueing
/// a run off the server is the event `R-STO-4` already words as "no item creation, no runs", and a
/// user who meets the refusal from a rename and from a run must not have to decide whether they are
/// two problems. The fn keeps its MOD-15 name because the sentence is the shared thing, not the
/// Plain delegation: a `Writer` decides *which* store, never *what* a read means.
impl ReadStore for Writer {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        match self {
            Self::Memory(store) => store.items(scope, filter).await,
            Self::Online(pg) => pg.items(scope, filter).await,
        }
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        match self {
            Self::Memory(store) => store.item(id).await,
            Self::Online(pg) => pg.item(id).await,
        }
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        match self {
            Self::Memory(store) => store.links(id, hops).await,
            Self::Online(pg) => pg.links(id, hops).await,
        }
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        match self {
            Self::Memory(store) => store.documents(id).await,
            Self::Online(pg) => pg.documents(id).await,
        }
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        match self {
            Self::Memory(store) => store.notes(id).await,
            Self::Online(pg) => pg.notes(id).await,
        }
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        match self {
            Self::Memory(store) => store.runs(id).await,
            Self::Online(pg) => pg.runs(id).await,
        }
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        match self {
            Self::Memory(store) => store.step_events(step).await,
            Self::Online(pg) => pg.step_events(step).await,
        }
    }

    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        match self {
            Self::Memory(store) => store.document(id).await,
            Self::Online(pg) => pg.document(id).await,
        }
    }

    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        match self {
            Self::Memory(store) => store.documents_of_kinds(item, kinds).await,
            Self::Online(pg) => pg.documents_of_kinds(item, kinds).await,
        }
    }

    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        match self {
            Self::Memory(store) => store.upstream_summaries(id, hops, scope).await,
            Self::Online(pg) => pg.upstream_summaries(id, hops, scope).await,
        }
    }

    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        match self {
            Self::Memory(store) => store.project(id).await,
            Self::Online(pg) => pg.project(id).await,
        }
    }

    // ---- ANA-2 §8's five run reads (MOD-4 milestone 1, plan D1) --------------------------------
    //
    // Delegation, like every read above: a `Writer` decides *which* store, never *what* a read
    // means. A store that cannot record never gets this far: `Backend::writer` answers `None`
    // (MOD-25).

    async fn run(&self, id: RunId) -> Result<Option<Run>> {
        match self {
            Self::Memory(store) => store.run(id).await,
            Self::Online(pg) => pg.run(id).await,
        }
    }

    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>> {
        match self {
            Self::Memory(store) => store.run_steps(run).await,
            Self::Online(pg) => pg.run_steps(run).await,
        }
    }

    async fn step_trees(&self, step: StepId) -> Result<Vec<RunStepTree>> {
        match self {
            Self::Memory(store) => store.step_trees(step).await,
            Self::Online(pg) => pg.step_trees(step).await,
        }
    }

    async fn step_commits(&self, step: StepId) -> Result<Vec<RunStepCommit>> {
        match self {
            Self::Memory(store) => store.step_commits(step).await,
            Self::Online(pg) => pg.step_commits(step).await,
        }
    }

    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> Result<Vec<ResolvedInput>> {
        match self {
            Self::Memory(store) => store.resolve_inputs(item, run, kinds).await,
            Self::Online(pg) => pg.resolve_inputs(item, run, kinds).await,
        }
    }

    // ---- ANA-11 §5.1: requirements (MOD-38) ----

    async fn requirement_spec(&self, project: ProjectId) -> Result<Option<RequirementSpec>> {
        match self {
            Self::Memory(store) => store.requirement_spec(project).await,
            Self::Online(pg) => pg.requirement_spec(project).await,
        }
    }

    async fn requirement_areas(&self, project: ProjectId) -> Result<Vec<RequirementArea>> {
        match self {
            Self::Memory(store) => store.requirement_areas(project).await,
            Self::Online(pg) => pg.requirement_areas(project).await,
        }
    }

    async fn requirements(
        &self,
        project: ProjectId,
        filter: &RequirementFilter,
    ) -> Result<Vec<Requirement>> {
        match self {
            Self::Memory(store) => store.requirements(project, filter).await,
            Self::Online(pg) => pg.requirements(project, filter).await,
        }
    }

    async fn requirement(&self, id: RequirementId) -> Result<Option<Requirement>> {
        match self {
            Self::Memory(store) => store.requirement(id).await,
            Self::Online(pg) => pg.requirement(id).await,
        }
    }

    async fn requirement_revisions(
        &self,
        id: RequirementId,
    ) -> Result<Option<Vec<RequirementRevision>>> {
        match self {
            Self::Memory(store) => store.requirement_revisions(id).await,
            Self::Online(pg) => pg.requirement_revisions(id).await,
        }
    }

    async fn item_requirements(&self, item: ItemId) -> Result<Vec<ItemCitation>> {
        match self {
            Self::Memory(store) => store.item_requirements(item).await,
            Self::Online(pg) => pg.item_requirements(item).await,
        }
    }

    async fn requirement_coverage(&self, requirement: RequirementId) -> Result<Vec<CoverageRow>> {
        match self {
            Self::Memory(store) => store.requirement_coverage(requirement).await,
            Self::Online(pg) => pg.requirement_coverage(requirement).await,
        }
    }

    async fn tool_call_counts(&self, item: ItemId) -> Result<Vec<ToolCallCount>> {
        match self {
            Self::Memory(store) => store.tool_call_counts(item).await,
            Self::Online(pg) => pg.tool_call_counts(item).await,
        }
    }
}

impl WriteStore for Writer {
    async fn mint_item(&self, new: NewItem) -> Result<Item> {
        match self {
            Self::Memory(store) => store.mint_item(new).await,
            Self::Online(pg) => pg.mint_item(new).await,
        }
    }

    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        match self {
            Self::Memory(store) => store.update_item(id, expected_version, patch).await,
            Self::Online(pg) => pg.update_item(id, expected_version, patch).await,
        }
    }

    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        match self {
            Self::Memory(store) => store.transition(id, from, to).await,
            Self::Online(pg) => pg.transition(id, from, to).await,
        }
    }

    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> Result<usize> {
        match self {
            Self::Memory(store) => store.append_events(fence, events).await,
            Self::Online(pg) => pg.append_events(fence, events).await,
        }
    }

    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => {
                store
                    .set_step_usage(fence, step, usage, prompt_digest)
                    .await
            }
            Self::Online(pg) => pg.set_step_usage(fence, step, usage, prompt_digest).await,
        }
    }

    async fn upsert_agent(
        &self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Agent>> {
        match self {
            Self::Memory(store) => store.upsert_agent(agent, expected).await,
            Self::Online(pg) => pg.upsert_agent(agent, expected).await,
        }
    }

    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_agent_box(row).await,
            Self::Online(pg) => pg.upsert_agent_box(row).await,
        }
    }

    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool> {
        match self {
            Self::Memory(store) => {
                store
                    .set_agent_box_quota(agent_id, box_id, quota, quota_at)
                    .await
            }
            Self::Online(pg) => {
                pg.set_agent_box_quota(agent_id, box_id, quota, quota_at)
                    .await
            }
        }
    }

    async fn set_agent_box_enabled(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.set_agent_box_enabled(agent_id, box_id, enabled).await,
            Self::Online(pg) => pg.set_agent_box_enabled(agent_id, box_id, enabled).await,
        }
    }

    async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()> {
        match self {
            Self::Memory(store) => store.record_box_probe(probe).await,
            Self::Online(pg) => pg.record_box_probe(probe).await,
        }
    }

    async fn boxes(&self) -> Result<Vec<BoxRecord>> {
        match self {
            Self::Memory(store) => store.boxes().await,
            Self::Online(pg) => pg.boxes().await,
        }
    }

    async fn edit_box(
        &self,
        id: BoxId,
        expected: i32,
        edit: BoxEdit,
    ) -> Result<CasOutcome<BoxRow>> {
        match self {
            Self::Memory(store) => store.edit_box(id, expected, edit).await,
            Self::Online(pg) => pg.edit_box(id, expected, edit).await,
        }
    }

    async fn box_probe_spec(&self) -> Result<Option<StoredSetting>> {
        match self {
            Self::Memory(store) => store.box_probe_spec().await,
            Self::Online(pg) => pg.box_probe_spec().await,
        }
    }

    async fn set_box_probe_spec(
        &self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredSetting>>> {
        match self {
            Self::Memory(store) => store.set_box_probe_spec(overlay, expected).await,
            Self::Online(pg) => pg.set_box_probe_spec(overlay, expected).await,
        }
    }

    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()> {
        match self {
            Self::Memory(store) => store.start_chat_run(chat).await,
            Self::Online(pg) => pg.start_chat_run(chat).await,
        }
    }

    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.finish_chat_run(run, step, status, finished_at).await,
            Self::Online(pg) => pg.finish_chat_run(run, step, status, finished_at).await,
        }
    }

    async fn set_step_prompt(
        &self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.set_step_prompt(fence, step, digest, trim).await,
            Self::Online(pg) => pg.set_step_prompt(fence, step, digest, trim).await,
        }
    }

    // ---- MOD-15 milestone 1: the hierarchy (plan D2) ---------------------------------------
    //
    // Delegation, as every arm above: a `Writer` decides *which* store, never *what* a write
    // means. The two stores disagree about how all 31 of these are kept — `MemStore` keeps maps,
    // `PgStore` keeps rows — and that disagreement stays in the stores.

    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace> {
        match self {
            Self::Memory(store) => store.create_workspace(new).await,
            Self::Online(pg) => pg.create_workspace(new).await,
        }
    }

    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> Result<CasOutcome<Workspace>> {
        match self {
            Self::Memory(store) => store.update_workspace(id, expected, patch).await,
            Self::Online(pg) => pg.update_workspace(id, expected, patch).await,
        }
    }

    async fn workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        match self {
            Self::Memory(store) => store.workspace(id).await,
            Self::Online(pg) => pg.workspace(id).await,
        }
    }

    async fn upsert_workspace_project(&self, link: &WorkspaceProject) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_workspace_project(link).await,
            Self::Online(pg) => pg.upsert_workspace_project(link).await,
        }
    }

    async fn remove_workspace_project(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.remove_workspace_project(workspace, project).await,
            Self::Online(pg) => pg.remove_workspace_project(workspace, project).await,
        }
    }

    async fn workspace_projects(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceProject>> {
        match self {
            Self::Memory(store) => store.workspace_projects(workspace).await,
            Self::Online(pg) => pg.workspace_projects(workspace).await,
        }
    }

    async fn upsert_workspace_box_path(&self, path: &WorkspaceBoxPath) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_workspace_box_path(path).await,
            Self::Online(pg) => pg.upsert_workspace_box_path(path).await,
        }
    }

    async fn workspace_box_paths(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceBoxPath>> {
        match self {
            Self::Memory(store) => store.workspace_box_paths(workspace).await,
            Self::Online(pg) => pg.workspace_box_paths(workspace).await,
        }
    }

    async fn create_project(&self, new: NewProject) -> Result<Project> {
        match self {
            Self::Memory(store) => store.create_project(new).await,
            Self::Online(pg) => pg.create_project(new).await,
        }
    }

    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> Result<CasOutcome<Project>> {
        match self {
            Self::Memory(store) => store.update_project(id, expected, patch).await,
            Self::Online(pg) => pg.update_project(id, expected, patch).await,
        }
    }

    async fn create_repo(&self, new: NewRepo) -> Result<Repo> {
        match self {
            Self::Memory(store) => store.create_repo(new).await,
            Self::Online(pg) => pg.create_repo(new).await,
        }
    }

    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> Result<CasOutcome<Repo>> {
        match self {
            Self::Memory(store) => store.update_repo(id, expected, patch).await,
            Self::Online(pg) => pg.update_repo(id, expected, patch).await,
        }
    }

    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>> {
        match self {
            Self::Memory(store) => store.repos(project).await,
            Self::Online(pg) => pg.repos(project).await,
        }
    }

    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_repo_box_path(path).await,
            Self::Online(pg) => pg.upsert_repo_box_path(path).await,
        }
    }

    async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool> {
        match self {
            Self::Memory(store) => store.infer_repo_box_path(path).await,
            Self::Online(pg) => pg.infer_repo_box_path(path).await,
        }
    }

    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        match self {
            Self::Memory(store) => store.repo_box_paths(repo).await,
            Self::Online(pg) => pg.repo_box_paths(repo).await,
        }
    }

    async fn create_item_kind(&self, new: NewItemKind) -> Result<ItemKind> {
        match self {
            Self::Memory(store) => store.create_item_kind(new).await,
            Self::Online(pg) => pg.create_item_kind(new).await,
        }
    }

    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> Result<CasOutcome<ItemKind>> {
        match self {
            Self::Memory(store) => store.update_item_kind(id, expected, patch).await,
            Self::Online(pg) => pg.update_item_kind(id, expected, patch).await,
        }
    }

    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
        match self {
            Self::Memory(store) => store.item_kinds(project).await,
            Self::Online(pg) => pg.item_kinds(project).await,
        }
    }

    async fn delete_item_kind(&self, id: ItemKindId) -> Result<()> {
        match self {
            Self::Memory(store) => store.delete_item_kind(id).await,
            Self::Online(pg) => pg.delete_item_kind(id).await,
        }
    }

    async fn create_step_graph(&self, new: NewStepGraph) -> Result<StepGraph> {
        match self {
            Self::Memory(store) => store.create_step_graph(new).await,
            Self::Online(pg) => pg.create_step_graph(new).await,
        }
    }

    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> Result<CasOutcome<StepGraph>> {
        match self {
            Self::Memory(store) => store.update_step_graph(id, expected, patch).await,
            Self::Online(pg) => pg.update_step_graph(id, expected, patch).await,
        }
    }

    async fn step_graphs(&self, project: ProjectId) -> Result<Vec<StepGraph>> {
        match self {
            Self::Memory(store) => store.step_graphs(project).await,
            Self::Online(pg) => pg.step_graphs(project).await,
        }
    }

    async fn create_phase(&self, phase: &StepGraphPhase) -> Result<StepGraphPhase> {
        match self {
            Self::Memory(store) => store.create_phase(phase).await,
            Self::Online(pg) => pg.create_phase(phase).await,
        }
    }

    async fn create_phase_agents(&self, phase: PhaseId, agents: &[PhaseAgent]) -> Result<()> {
        match self {
            Self::Memory(store) => store.create_phase_agents(phase, agents).await,
            Self::Online(pg) => pg.create_phase_agents(phase, agents).await,
        }
    }

    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> Result<CasOutcome<StepGraphPhase>> {
        match self {
            Self::Memory(store) => store.update_phase(id, expected, patch).await,
            Self::Online(pg) => pg.update_phase(id, expected, patch).await,
        }
    }

    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        match self {
            Self::Memory(store) => store.phases(graph).await,
            Self::Online(pg) => pg.phases(graph).await,
        }
    }

    async fn append_prompt_template(
        &self,
        new: NewPromptTemplate,
        expected: Option<i32>,
    ) -> Result<CasOutcome<PromptTemplate>> {
        match self {
            Self::Memory(store) => store.append_prompt_template(new, expected).await,
            Self::Online(pg) => pg.append_prompt_template(new, expected).await,
        }
    }

    async fn skills(&self) -> Result<Vec<Skill>> {
        match self {
            Self::Memory(store) => store.skills().await,
            Self::Online(pg) => pg.skills().await,
        }
    }

    async fn skill_versions(&self, skill: SkillId) -> Result<Vec<SkillVersion>> {
        match self {
            Self::Memory(store) => store.skill_versions(skill).await,
            Self::Online(pg) => pg.skill_versions(skill).await,
        }
    }

    async fn skill_bindings(&self, project: Option<ProjectId>) -> Result<Vec<SkillBinding>> {
        match self {
            Self::Memory(store) => store.skill_bindings(project).await,
            Self::Online(pg) => pg.skill_bindings(project).await,
        }
    }

    async fn create_skill(&self, new: NewSkill) -> Result<(Skill, SkillVersion)> {
        match self {
            Self::Memory(store) => store.create_skill(new).await,
            Self::Online(pg) => pg.create_skill(new).await,
        }
    }

    async fn update_skill(
        &self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
    ) -> Result<CasOutcome<Skill>> {
        match self {
            Self::Memory(store) => store.update_skill(id, expected, patch).await,
            Self::Online(pg) => pg.update_skill(id, expected, patch).await,
        }
    }

    async fn add_skill_version(
        &self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
    ) -> Result<CasOutcome<SkillVersion>> {
        match self {
            Self::Memory(store) => store.add_skill_version(skill, expected, new).await,
            Self::Online(pg) => pg.add_skill_version(skill, expected, new).await,
        }
    }

    async fn set_skill_binding(
        &self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> Result<CasOutcome<Option<SkillBinding>>> {
        match self {
            Self::Memory(store) => store.set_skill_binding(key, expected, change).await,
            Self::Online(pg) => pg.set_skill_binding(key, expected, change).await,
        }
    }

    async fn personas(&self) -> Result<Vec<Persona>> {
        match self {
            Self::Memory(store) => store.personas().await,
            Self::Online(pg) => pg.personas().await,
        }
    }

    async fn create_persona(&self, new: NewPersona) -> Result<Persona> {
        match self {
            Self::Memory(store) => store.create_persona(new).await,
            Self::Online(pg) => pg.create_persona(new).await,
        }
    }

    async fn update_persona(
        &self,
        id: PersonaId,
        expected: DateTime<Utc>,
        patch: PersonaPatch,
    ) -> Result<CasOutcome<Persona>> {
        match self {
            Self::Memory(store) => store.update_persona(id, expected, patch).await,
            Self::Online(pg) => pg.update_persona(id, expected, patch).await,
        }
    }

    async fn delete_persona(&self, id: PersonaId) -> Result<()> {
        match self {
            Self::Memory(store) => store.delete_persona(id).await,
            Self::Online(pg) => pg.delete_persona(id).await,
        }
    }

    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<StoredSetting>> {
        match self {
            Self::Memory(store) => store.set_setting(rung, key, value, expected).await,
            Self::Online(pg) => pg.set_setting(rung, key, value, expected).await,
        }
    }

    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>> {
        match self {
            Self::Memory(store) => store.clear_setting(rung, key, expected).await,
            Self::Online(pg) => pg.clear_setting(rung, key, expected).await,
        }
    }

    async fn setting(&self, rung: SettingRung, key: SettingKey) -> Result<Option<StoredSetting>> {
        match self {
            Self::Memory(store) => store.setting(rung, key).await,
            Self::Online(pg) => pg.setting(rung, key).await,
        }
    }

    async fn delete_reach(&self, target: DeleteTarget) -> Result<Option<DeleteReach>> {
        match self {
            Self::Memory(store) => store.delete_reach(target).await,
            Self::Online(pg) => pg.delete_reach(target).await,
        }
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<DeleteReach> {
        match self {
            Self::Memory(store) => store.delete_workspace(id).await,
            Self::Online(pg) => pg.delete_workspace(id).await,
        }
    }

    async fn delete_project(&self, id: ProjectId) -> Result<DeleteReach> {
        match self {
            Self::Memory(store) => store.delete_project(id).await,
            Self::Online(pg) => pg.delete_project(id).await,
        }
    }

    // ---- ANA-2 §8's eighteen run writers (MOD-4 milestone 1, plan D1) -------------------------
    //
    // Delegation, for the reason above.

    async fn create_run(&self, new: NewRun) -> Result<Run> {
        match self {
            Self::Memory(store) => store.create_run(new).await,
            Self::Online(pg) => pg.create_run(new).await,
        }
    }

    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> Result<Claim> {
        match self {
            Self::Memory(store) => store.claim_run(run, box_id, owner, at, ttl).await,
            Self::Online(pg) => pg.claim_run(run, box_id, owner, at, ttl).await,
        }
    }

    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool> {
        match self {
            Self::Memory(store) => store.refresh_lease(run, owner, ttl).await,
            Self::Online(pg) => pg.refresh_lease(run, owner, ttl).await,
        }
    }

    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>> {
        match self {
            Self::Memory(store) => store.adopt_runs(box_id, owner, ttl).await,
            Self::Online(pg) => pg.adopt_runs(box_id, owner, ttl).await,
        }
    }

    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool> {
        match self {
            Self::Memory(store) => store.take_lease(run, box_id, owner, ttl).await,
            Self::Online(pg) => pg.take_lease(run, box_id, owner, ttl).await,
        }
    }

    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool> {
        match self {
            Self::Memory(store) => store.release_lease(run, owner).await,
            Self::Online(pg) => pg.release_lease(run, owner).await,
        }
    }

    async fn create_step(&self, new: NewRunStep) -> Result<RunStep> {
        match self {
            Self::Memory(store) => store.create_step(new).await,
            Self::Online(pg) => pg.create_step(new).await,
        }
    }

    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        match self {
            Self::Memory(store) => store.transition_run(run, from, to, at).await,
            Self::Online(pg) => pg.transition_run(run, from, to, at).await,
        }
    }

    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        match self {
            Self::Memory(store) => store.transition_step(step, from, to, at).await,
            Self::Online(pg) => pg.transition_step(step, from, to, at).await,
        }
    }

    async fn finish_step(
        &self,
        fence: StepFence,
        step: StepId,
        outcome: StepOutcome,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.finish_step(fence, step, outcome).await,
            Self::Online(pg) => pg.finish_step(fence, step, outcome).await,
        }
    }

    async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool> {
        match self {
            Self::Memory(store) => store.interrupt_step(step, note, at).await,
            Self::Online(pg) => pg.interrupt_step(step, note, at).await,
        }
    }

    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        match self {
            Self::Memory(store) => store.answer_gate(step, outcome, note, at).await,
            Self::Online(pg) => pg.answer_gate(step, outcome, note, at).await,
        }
    }

    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => {
                store
                    .select_fanout(run, position, attempt, winner, reason)
                    .await
            }
            Self::Online(pg) => {
                pg.select_fanout(run, position, attempt, winner, reason)
                    .await
            }
        }
    }

    async fn supersede_step(&self, step: StepId) -> Result<()> {
        match self {
            Self::Memory(store) => store.supersede_step(step).await,
            Self::Online(pg) => pg.supersede_step(step).await,
        }
    }

    async fn upsert_step_tree(
        &self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_step_tree(fence, step, trees).await,
            Self::Online(pg) => pg.upsert_step_tree(fence, step, trees).await,
        }
    }

    async fn record_commits(
        &self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.record_commits(fence, step, commits).await,
            Self::Online(pg) => pg.record_commits(fence, step, commits).await,
        }
    }

    async fn record_command_run(&self, new: NewCommandRun) -> Result<CommandRun> {
        match self {
            Self::Memory(store) => store.record_command_run(new).await,
            Self::Online(pg) => pg.record_command_run(new).await,
        }
    }

    async fn command_runs(&self, step: StepId) -> Result<Vec<CommandRun>> {
        match self {
            Self::Memory(store) => store.command_runs(step).await,
            Self::Online(pg) => pg.command_runs(step).await,
        }
    }

    async fn write_document(&self, new: NewDocument) -> Result<Document> {
        match self {
            Self::Memory(store) => store.write_document(new).await,
            Self::Online(pg) => pg.write_document(new).await,
        }
    }

    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> Result<()> {
        match self {
            Self::Memory(store) => store.promote_step(step, at).await,
            Self::Online(pg) => pg.promote_step(step, at).await,
        }
    }

    async fn record_opening(&self, step: StepId, opening: StepOpening) -> Result<()> {
        match self {
            Self::Memory(store) => store.record_opening(step, opening).await,
            Self::Online(pg) => pg.record_opening(step, opening).await,
        }
    }

    async fn pass_step(
        &self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        match self {
            Self::Memory(store) => store.pass_step(fence, step, note, at).await,
            Self::Online(pg) => pg.pass_step(fence, step, note, at).await,
        }
    }

    async fn park_step(&self, fence: StepFence, step: StepId) -> Result<ParkOutcome> {
        match self {
            Self::Memory(store) => store.park_step(fence, step).await,
            Self::Online(pg) => pg.park_step(fence, step).await,
        }
    }

    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> Result<()> {
        match self {
            Self::Memory(store) => store.fail_run(run, failure, at).await,
            Self::Online(pg) => pg.fail_run(run, failure, at).await,
        }
    }

    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.finish_run(run, to, failure, at).await,
            Self::Online(pg) => pg.finish_run(run, to, failure, at).await,
        }
    }

    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document> {
        match self {
            Self::Memory(store) => store.close_out(item, resolution, summary, commits).await,
            Self::Online(pg) => pg.close_out(item, resolution, summary, commits).await,
        }
    }

    async fn add_note(&self, note: NewNote) -> Result<Note> {
        match self {
            Self::Memory(store) => store.add_note(note).await,
            Self::Online(pg) => pg.add_note(note).await,
        }
    }

    // ---- ANA-11 §5.1: requirements and citations (MOD-38) ----

    async fn set_requirement_spec(
        &self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
    ) -> Result<CasOutcome<RequirementSpec>> {
        match self {
            Self::Memory(store) => {
                store
                    .set_requirement_spec(project, expected_version, owner_id, preamble)
                    .await
            }
            Self::Online(pg) => {
                pg.set_requirement_spec(project, expected_version, owner_id, preamble)
                    .await
            }
        }
    }

    async fn create_requirement_area(&self, new: NewRequirementArea) -> Result<RequirementArea> {
        match self {
            Self::Memory(store) => store.create_requirement_area(new).await,
            Self::Online(pg) => pg.create_requirement_area(new).await,
        }
    }

    async fn mint_requirement(
        &self,
        area: RequirementAreaId,
        new: NewRequirement,
    ) -> Result<Requirement> {
        match self {
            Self::Memory(store) => store.mint_requirement(area, new).await,
            Self::Online(pg) => pg.mint_requirement(area, new).await,
        }
    }

    async fn amend_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        amended_by: ItemId,
    ) -> Result<RequirementUpdate> {
        match self {
            Self::Memory(store) => {
                store
                    .amend_requirement(id, expected_version, patch, amended_by)
                    .await
            }
            Self::Online(pg) => {
                pg.amend_requirement(id, expected_version, patch, amended_by)
                    .await
            }
        }
    }

    async fn withdraw_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        withdrawn_by: ItemId,
        author_id: UserId,
        box_id: Option<BoxId>,
    ) -> Result<RequirementUpdate> {
        match self {
            Self::Memory(store) => {
                store
                    .withdraw_requirement(id, expected_version, withdrawn_by, author_id, box_id)
                    .await
            }
            Self::Online(pg) => {
                pg.withdraw_requirement(id, expected_version, withdrawn_by, author_id, box_id)
                    .await
            }
        }
    }

    async fn cite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        proposed_by: Option<StepId>,
    ) -> Result<ItemRequirement> {
        match self {
            Self::Memory(store) => store.cite(item, requirement, kind, proposed_by).await,
            Self::Online(pg) => pg.cite(item, requirement, kind, proposed_by).await,
        }
    }

    async fn uncite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.uncite(item, requirement, kind).await,
            Self::Online(pg) => pg.uncite(item, requirement, kind).await,
        }
    }

    async fn reconfirm(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<ItemRequirement> {
        match self {
            Self::Memory(store) => store.reconfirm(item, requirement, kind).await,
            Self::Online(pg) => pg.reconfirm(item, requirement, kind).await,
        }
    }

    // -- MOD-42: the permission and control relay (plan D1-D5, D12-D14)

    async fn open_permission(&self, open: OpenPermission) -> Result<PermissionId> {
        match self {
            Self::Memory(store) => store.open_permission(open).await,
            Self::Online(pg) => pg.open_permission(open).await,
        }
    }

    async fn permission(&self, id: PermissionId) -> Result<Option<StepPermission>> {
        match self {
            Self::Memory(store) => store.permission(id).await,
            Self::Online(pg) => pg.permission(id).await,
        }
    }

    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> Result<Option<PermissionChoice>> {
        match self {
            Self::Memory(store) => store.apply_permission(id, owner).await,
            Self::Online(pg) => pg.apply_permission(id, owner).await,
        }
    }

    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> Result<u64> {
        match self {
            Self::Memory(store) => store.settle_permissions(session, to).await,
            Self::Online(pg) => pg.settle_permissions(session, to).await,
        }
    }

    async fn request_cancel(
        &self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
    ) -> Result<CancelRequest> {
        match self {
            Self::Memory(store) => store.request_cancel(run, user, box_id).await,
            Self::Online(pg) => pg.request_cancel(run, user, box_id).await,
        }
    }

    async fn pending_commands(&self, owner: Uuid, box_id: BoxId) -> Result<Vec<RunCommand>> {
        match self {
            Self::Memory(store) => store.pending_commands(owner, box_id).await,
            Self::Online(pg) => pg.pending_commands(owner, box_id).await,
        }
    }

    async fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> Result<bool> {
        match self {
            Self::Memory(store) => store.resolve_command(id, to, resolution).await,
            Self::Online(pg) => pg.resolve_command(id, to, resolution).await,
        }
    }

    async fn relay_view(&self, item: ItemId) -> Result<RelayView> {
        match self {
            Self::Memory(store) => store.relay_view(item).await,
            Self::Online(pg) => pg.relay_view(item).await,
        }
    }

    async fn answer_permission(
        &self,
        id: PermissionId,
        option_id: &str,
        user: UserId,
        box_id: BoxId,
    ) -> Result<AnswerOutcome> {
        match self {
            Self::Memory(store) => store.answer_permission(id, option_id, user, box_id).await,
            Self::Online(pg) => pg.answer_permission(id, option_id, user, box_id).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Backend;
    use crate::CacheStore;
    use htui_core::fixtures::ids;
    use htui_core::store::StoreError;

    #[tokio::test]
    async fn a_memory_writer_round_trips_a_chat_run() {
        let store = MemStore::demo();
        let backend = Backend::memory(store.clone());
        let writer = backend.writer().expect("a memory backend is writable");
        assert_eq!(writer.label(), "memory");

        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let before = store.active_runs(&scope).await.expect("count");

        let chat = ChatRunSpec::mint(
            ids::PROJECT_HTUI,
            ids::BOX,
            ids::USER,
            Some(ids::AGENT_CLAUDE),
            None,
        );
        writer.start_chat_run(&chat).await.expect("the rows mint");
        assert_eq!(
            store.active_runs(&scope).await.expect("count"),
            before + 1,
            "a live chat counts as an active run"
        );

        writer
            .finish_chat_run(
                chat.run_id,
                chat.step_id,
                RunStatus::Done,
                htui_core::fixtures::demo_at(0, 0),
            )
            .await
            .expect("the run closes");
        assert_eq!(
            store.active_runs(&scope).await.expect("count"),
            before,
            "a closed chat stops counting"
        );
    }

    #[tokio::test]
    async fn an_offline_backend_hands_out_no_writer_and_no_user() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "writer-test", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        assert!(
            backend.writer().is_none(),
            "since MOD-25 there is no offline write path at all: a chat off the server is \
             refused, not buffered"
        );
        assert!(
            !backend.is_writable(),
            "and the server is still unreachable: the re-dial ticker keys on this"
        );
        assert!(
            matches!(backend.this_user().await, Err(StoreError::NotFound { .. })),
            "and no author for a run row: this mirror has never synced one (MOD-2 D33)"
        );
        cache.close().await;
    }

    #[tokio::test]
    async fn a_memory_backend_answers_the_fixture_user() {
        let backend = Backend::memory(MemStore::demo());
        assert_eq!(
            backend.this_user().await.expect("the fixture has a user"),
            ids::USER
        );

        let empty = Backend::memory(MemStore::new());
        assert!(
            matches!(empty.this_user().await, Err(StoreError::NotFound { .. })),
            "an empty store has no user to start a run as"
        );
    }
}
