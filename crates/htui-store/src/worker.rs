//! `htui_core::store`'s MOD-41 worker traits for the server-side stores (plan D4, D5).
//!
//! `RecorderStore`, `RelayStore` (MOD-42) and `WorkerStore` for [`PgStore`] and [`Writer`],
//! `WorkerHost` for [`PgStore`] (the headless worker's host) and [`Backend`] (the TUI's). Every
//! body forwards by path (UFCS) to the [`ReadStore`]/[`WriteStore`] method or the inherent read of
//! the same name, and the traits are named by path, never imported (plan D5: a receiver that sees
//! both families is E0034).
//! Nothing here is `pub`: the impls are the whole content.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};
use htui_core::model::link::{ItemLink, ProposeLink, WithdrawLink};
use htui_core::model::{
    AgentBox, AgentId, AgentSummary, BoundSkill, BoxId, BoxInfo, BoxProfile, BoxRow, CancelRequest,
    Claim, CommandRun, Document, DocumentHead, DocumentId, GateOutcome, Item, ItemId, ItemKind,
    NewCommandRun, NewDocument, NewNote, NewRun, NewRunStep, Note, OpenPermission,
    PermissionChoice, PermissionId, PermissionStatus, PhaseAgent, PhaseId, Project, ProjectId,
    PromptScope, PromptTemplate, RelaySessionId, Repo, RepoBoxPath, RepoId, Resolution,
    ResolvedGraph, ResolvedInput, Run, RunCommand, RunCommandId, RunCommandStatus, RunId,
    RunStatus, RunStep, RunStepCommit, RunStepTree, RunSummary, SessionEvent, Status, StepGraphId,
    StepGraphPhase, StepId, StepOutcome, StepPermission, StepStatus, UpstreamEntry, UserId,
    WorkspaceSummary,
};
use htui_core::store::{ParkOutcome, ReadStore, Result, StepFence, WriteStore};
use serde_json::Value;
use uuid::Uuid;

use crate::backend::Backend;
use crate::pg::PgStore;
use crate::writer::Writer;

impl htui_core::store::RecorderStore for PgStore {
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> Result<usize> {
        WriteStore::append_events(self, fence, events).await
    }
    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        WriteStore::set_step_usage(self, fence, step, usage, prompt_digest).await
    }
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::set_agent_box_quota(self, agent_id, box_id, quota, quota_at).await
    }
}

impl htui_core::store::RelayStore for PgStore {
    async fn open_permission(&self, open: OpenPermission) -> Result<PermissionId> {
        WriteStore::open_permission(self, open).await
    }
    async fn permission(&self, id: PermissionId) -> Result<Option<StepPermission>> {
        WriteStore::permission(self, id).await
    }
    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> Result<Option<PermissionChoice>> {
        WriteStore::apply_permission(self, id, owner).await
    }
    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> Result<u64> {
        WriteStore::settle_permissions(self, session, to).await
    }
}

impl htui_core::store::WorkerStore for PgStore {
    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        ReadStore::item(self, id).await
    }
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        ReadStore::documents(self, id).await
    }
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        ReadStore::runs(self, id).await
    }
    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        ReadStore::step_events(self, step).await
    }
    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        ReadStore::document(self, id).await
    }
    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        ReadStore::documents_of_kinds(self, item, kinds).await
    }
    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        ReadStore::upstream_summaries(self, id, hops, scope).await
    }
    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        ReadStore::project(self, id).await
    }
    async fn run(&self, id: RunId) -> Result<Option<Run>> {
        ReadStore::run(self, id).await
    }
    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>> {
        ReadStore::run_steps(self, run).await
    }
    async fn step_trees(&self, step: StepId) -> Result<Vec<RunStepTree>> {
        ReadStore::step_trees(self, step).await
    }
    async fn step_commits(&self, step: StepId) -> Result<Vec<RunStepCommit>> {
        ReadStore::step_commits(self, step).await
    }
    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> Result<Vec<ResolvedInput>> {
        ReadStore::resolve_inputs(self, item, run, kinds).await
    }
    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>> {
        WriteStore::repos(self, project).await
    }
    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        WriteStore::repo_box_paths(self, repo).await
    }
    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
        WriteStore::item_kinds(self, project).await
    }
    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        WriteStore::phases(self, graph).await
    }
    async fn command_runs(&self, step: StepId) -> Result<Vec<CommandRun>> {
        WriteStore::command_runs(self, step).await
    }
    async fn relay_view(&self, item: ItemId) -> Result<htui_core::model::RelayView> {
        WriteStore::relay_view(self, item).await
    }
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        WriteStore::transition(self, id, from, to).await
    }
    async fn set_step_prompt(
        &self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
    ) -> Result<()> {
        WriteStore::set_step_prompt(self, fence, step, digest, trim).await
    }
    async fn create_run(&self, new: NewRun) -> Result<Run> {
        WriteStore::create_run(self, new).await
    }
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> Result<Claim> {
        WriteStore::claim_run(self, run, box_id, owner, at, ttl).await
    }
    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool> {
        WriteStore::refresh_lease(self, run, owner, ttl).await
    }
    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>> {
        WriteStore::adopt_runs(self, box_id, owner, ttl).await
    }
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool> {
        WriteStore::take_lease(self, run, box_id, owner, ttl).await
    }
    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool> {
        WriteStore::release_lease(self, run, owner).await
    }
    async fn create_step(&self, new: NewRunStep) -> Result<RunStep> {
        WriteStore::create_step(self, new).await
    }
    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::transition_run(self, run, from, to, at).await
    }
    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::transition_step(self, step, from, to, at).await
    }
    async fn finish_step(
        &self,
        fence: StepFence,
        step: StepId,
        outcome: StepOutcome,
    ) -> Result<()> {
        WriteStore::finish_step(self, fence, step, outcome).await
    }
    async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool> {
        WriteStore::interrupt_step(self, step, note, at).await
    }
    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::answer_gate(self, step, outcome, note, at).await
    }
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> Result<()> {
        WriteStore::select_fanout(self, run, position, attempt, winner, reason).await
    }
    async fn supersede_step(&self, step: StepId) -> Result<()> {
        WriteStore::supersede_step(self, step).await
    }
    async fn upsert_step_tree(
        &self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
    ) -> Result<()> {
        WriteStore::upsert_step_tree(self, fence, step, trees).await
    }
    async fn record_commits(
        &self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> Result<()> {
        WriteStore::record_commits(self, fence, step, commits).await
    }
    async fn record_command_run(&self, new: NewCommandRun) -> Result<CommandRun> {
        WriteStore::record_command_run(self, new).await
    }
    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> Result<()> {
        WriteStore::promote_step(self, step, at).await
    }
    async fn pass_step(
        &self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::pass_step(self, fence, step, note, at).await
    }
    async fn park_step(&self, fence: StepFence, step: StepId) -> Result<ParkOutcome> {
        WriteStore::park_step(self, fence, step).await
    }
    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        WriteStore::finish_run(self, run, to, failure, at).await
    }
    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document> {
        WriteStore::close_out(self, item, resolution, summary, commits).await
    }
    async fn add_note(&self, note: NewNote) -> Result<Note> {
        WriteStore::add_note(self, note).await
    }
    async fn write_document(&self, new: NewDocument) -> Result<Document> {
        WriteStore::write_document(self, new).await
    }
    async fn request_cancel(
        &self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
    ) -> Result<CancelRequest> {
        WriteStore::request_cancel(self, run, user, box_id).await
    }
    async fn pending_commands(&self, owner: Uuid, box_id: BoxId) -> Result<Vec<RunCommand>> {
        WriteStore::pending_commands(self, owner, box_id).await
    }
    async fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> Result<bool> {
        WriteStore::resolve_command(self, id, to, resolution).await
    }
    async fn write_step_document(&self, fence: StepFence, new: NewDocument) -> Result<Document> {
        WriteStore::write_step_document(self, fence, new).await
    }
    async fn add_step_note(&self, fence: StepFence, note: NewNote) -> Result<Note> {
        WriteStore::add_step_note(self, fence, note).await
    }
    async fn propose_link(&self, fence: StepFence, link: ProposeLink) -> Result<ItemLink> {
        WriteStore::propose_link(self, fence, link).await
    }
    async fn withdraw_link(&self, fence: StepFence, link: WithdrawLink) -> Result<ItemLink> {
        WriteStore::withdraw_link(self, fence, link).await
    }
    async fn item_by_key(&self, project: ProjectId, key: &str) -> Result<Option<ItemId>> {
        WriteStore::item_by_key(self, project, key).await
    }
}

impl htui_core::store::RecorderStore for Writer {
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> Result<usize> {
        WriteStore::append_events(self, fence, events).await
    }
    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        WriteStore::set_step_usage(self, fence, step, usage, prompt_digest).await
    }
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::set_agent_box_quota(self, agent_id, box_id, quota, quota_at).await
    }
}

impl htui_core::store::RelayStore for Writer {
    async fn open_permission(&self, open: OpenPermission) -> Result<PermissionId> {
        WriteStore::open_permission(self, open).await
    }
    async fn permission(&self, id: PermissionId) -> Result<Option<StepPermission>> {
        WriteStore::permission(self, id).await
    }
    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> Result<Option<PermissionChoice>> {
        WriteStore::apply_permission(self, id, owner).await
    }
    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> Result<u64> {
        WriteStore::settle_permissions(self, session, to).await
    }
}

impl htui_core::store::WorkerStore for Writer {
    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        ReadStore::item(self, id).await
    }
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        ReadStore::documents(self, id).await
    }
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        ReadStore::runs(self, id).await
    }
    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        ReadStore::step_events(self, step).await
    }
    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        ReadStore::document(self, id).await
    }
    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        ReadStore::documents_of_kinds(self, item, kinds).await
    }
    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        ReadStore::upstream_summaries(self, id, hops, scope).await
    }
    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        ReadStore::project(self, id).await
    }
    async fn run(&self, id: RunId) -> Result<Option<Run>> {
        ReadStore::run(self, id).await
    }
    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>> {
        ReadStore::run_steps(self, run).await
    }
    async fn step_trees(&self, step: StepId) -> Result<Vec<RunStepTree>> {
        ReadStore::step_trees(self, step).await
    }
    async fn step_commits(&self, step: StepId) -> Result<Vec<RunStepCommit>> {
        ReadStore::step_commits(self, step).await
    }
    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> Result<Vec<ResolvedInput>> {
        ReadStore::resolve_inputs(self, item, run, kinds).await
    }
    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>> {
        WriteStore::repos(self, project).await
    }
    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        WriteStore::repo_box_paths(self, repo).await
    }
    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
        WriteStore::item_kinds(self, project).await
    }
    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        WriteStore::phases(self, graph).await
    }
    async fn command_runs(&self, step: StepId) -> Result<Vec<CommandRun>> {
        WriteStore::command_runs(self, step).await
    }
    async fn relay_view(&self, item: ItemId) -> Result<htui_core::model::RelayView> {
        WriteStore::relay_view(self, item).await
    }
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        WriteStore::transition(self, id, from, to).await
    }
    async fn set_step_prompt(
        &self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
    ) -> Result<()> {
        WriteStore::set_step_prompt(self, fence, step, digest, trim).await
    }
    async fn create_run(&self, new: NewRun) -> Result<Run> {
        WriteStore::create_run(self, new).await
    }
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> Result<Claim> {
        WriteStore::claim_run(self, run, box_id, owner, at, ttl).await
    }
    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool> {
        WriteStore::refresh_lease(self, run, owner, ttl).await
    }
    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>> {
        WriteStore::adopt_runs(self, box_id, owner, ttl).await
    }
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool> {
        WriteStore::take_lease(self, run, box_id, owner, ttl).await
    }
    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool> {
        WriteStore::release_lease(self, run, owner).await
    }
    async fn create_step(&self, new: NewRunStep) -> Result<RunStep> {
        WriteStore::create_step(self, new).await
    }
    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::transition_run(self, run, from, to, at).await
    }
    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::transition_step(self, step, from, to, at).await
    }
    async fn finish_step(
        &self,
        fence: StepFence,
        step: StepId,
        outcome: StepOutcome,
    ) -> Result<()> {
        WriteStore::finish_step(self, fence, step, outcome).await
    }
    async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool> {
        WriteStore::interrupt_step(self, step, note, at).await
    }
    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::answer_gate(self, step, outcome, note, at).await
    }
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> Result<()> {
        WriteStore::select_fanout(self, run, position, attempt, winner, reason).await
    }
    async fn supersede_step(&self, step: StepId) -> Result<()> {
        WriteStore::supersede_step(self, step).await
    }
    async fn upsert_step_tree(
        &self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
    ) -> Result<()> {
        WriteStore::upsert_step_tree(self, fence, step, trees).await
    }
    async fn record_commits(
        &self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> Result<()> {
        WriteStore::record_commits(self, fence, step, commits).await
    }
    async fn record_command_run(&self, new: NewCommandRun) -> Result<CommandRun> {
        WriteStore::record_command_run(self, new).await
    }
    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> Result<()> {
        WriteStore::promote_step(self, step, at).await
    }
    async fn pass_step(
        &self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        WriteStore::pass_step(self, fence, step, note, at).await
    }
    async fn park_step(&self, fence: StepFence, step: StepId) -> Result<ParkOutcome> {
        WriteStore::park_step(self, fence, step).await
    }
    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        WriteStore::finish_run(self, run, to, failure, at).await
    }
    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document> {
        WriteStore::close_out(self, item, resolution, summary, commits).await
    }
    async fn add_note(&self, note: NewNote) -> Result<Note> {
        WriteStore::add_note(self, note).await
    }
    async fn write_document(&self, new: NewDocument) -> Result<Document> {
        WriteStore::write_document(self, new).await
    }
    async fn request_cancel(
        &self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
    ) -> Result<CancelRequest> {
        WriteStore::request_cancel(self, run, user, box_id).await
    }
    async fn pending_commands(&self, owner: Uuid, box_id: BoxId) -> Result<Vec<RunCommand>> {
        WriteStore::pending_commands(self, owner, box_id).await
    }
    async fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> Result<bool> {
        WriteStore::resolve_command(self, id, to, resolution).await
    }
    async fn write_step_document(&self, fence: StepFence, new: NewDocument) -> Result<Document> {
        WriteStore::write_step_document(self, fence, new).await
    }
    async fn add_step_note(&self, fence: StepFence, note: NewNote) -> Result<Note> {
        WriteStore::add_step_note(self, fence, note).await
    }
    async fn propose_link(&self, fence: StepFence, link: ProposeLink) -> Result<ItemLink> {
        WriteStore::propose_link(self, fence, link).await
    }
    async fn withdraw_link(&self, fence: StepFence, link: WithdrawLink) -> Result<ItemLink> {
        WriteStore::withdraw_link(self, fence, link).await
    }
    async fn item_by_key(&self, project: ProjectId, key: &str) -> Result<Option<ItemId>> {
        WriteStore::item_by_key(self, project, key).await
    }
}

impl htui_core::store::WorkerHost for PgStore {
    type Store = PgStore;

    fn writer(&self) -> Option<PgStore> {
        Some(self.clone())
    }
    async fn box_info(&self) -> Result<Option<BoxInfo>> {
        PgStore::box_info(self).await
    }
    async fn this_user(&self) -> Result<UserId> {
        Ok(PgStore::this_user(self))
    }
    async fn app_settings(&self) -> Result<BTreeMap<String, Value>> {
        PgStore::app_settings(self).await
    }
    async fn box_profile(&self, id: BoxId) -> Result<Option<BoxProfile>> {
        PgStore::box_profile(self, id).await
    }
    async fn agents(&self) -> Result<Vec<AgentSummary>> {
        PgStore::agents(self).await
    }
    async fn box_row(&self, id: BoxId) -> Result<Option<BoxRow>> {
        PgStore::box_row(self, id).await
    }
    async fn repo_paths(&self, box_id: BoxId) -> Result<Vec<RepoBoxPath>> {
        PgStore::repo_paths(self, box_id).await
    }
    async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        PgStore::workspaces(self).await
    }
    async fn active_runs_on_box(&self, box_id: BoxId) -> Result<usize> {
        PgStore::active_runs_on_box(self, box_id).await
    }
    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        ReadStore::item(self, id).await
    }
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        ReadStore::documents(self, id).await
    }
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        ReadStore::runs(self, id).await
    }
    async fn run(&self, id: RunId) -> Result<Option<Run>> {
        ReadStore::run(self, id).await
    }
    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>> {
        ReadStore::run_steps(self, run).await
    }
    async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
        PgStore::resolve_graph(self, item).await
    }
    async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
        PgStore::phase_agents(self, phase).await
    }
    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>> {
        PgStore::prompt_template(self, project, name, version).await
    }
    async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
        PgStore::agent_boxes(self, box_id).await
    }
    async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>> {
        PgStore::bound_skills(self, project, phase).await
    }
    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
        PgStore::missing_tags(self, item, box_id).await
    }
    async fn queued_runs_on_box(&self, box_id: BoxId) -> Result<Vec<(RunId, DateTime<Utc>)>> {
        PgStore::queued_runs_on_box(self, box_id).await
    }
}

impl htui_core::store::WorkerHost for Backend {
    type Store = Writer;

    fn writer(&self) -> Option<Writer> {
        Backend::writer(self)
    }
    async fn box_info(&self) -> Result<Option<BoxInfo>> {
        Backend::box_info(self).await
    }
    async fn this_user(&self) -> Result<UserId> {
        Backend::this_user(self).await
    }
    async fn app_settings(&self) -> Result<BTreeMap<String, Value>> {
        Backend::app_settings(self).await
    }
    async fn box_profile(&self, id: BoxId) -> Result<Option<BoxProfile>> {
        Backend::box_profile(self, id).await
    }
    async fn agents(&self) -> Result<Vec<AgentSummary>> {
        Backend::agents(self).await
    }
    async fn box_row(&self, id: BoxId) -> Result<Option<BoxRow>> {
        Backend::box_row(self, id).await
    }
    async fn repo_paths(&self, box_id: BoxId) -> Result<Vec<RepoBoxPath>> {
        Backend::repo_paths(self, box_id).await
    }
    async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        Backend::workspaces(self).await
    }
    async fn active_runs_on_box(&self, box_id: BoxId) -> Result<usize> {
        Backend::active_runs_on_box(self, box_id).await
    }
    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        ReadStore::item(self, id).await
    }
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        ReadStore::documents(self, id).await
    }
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        ReadStore::runs(self, id).await
    }
    async fn run(&self, id: RunId) -> Result<Option<Run>> {
        ReadStore::run(self, id).await
    }
    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>> {
        ReadStore::run_steps(self, run).await
    }
    async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
        Backend::resolve_graph(self, item).await
    }
    async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
        Backend::phase_agents(self, phase).await
    }
    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>> {
        Backend::prompt_template(self, project, name, version).await
    }
    async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
        Backend::agent_boxes(self, box_id).await
    }
    async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>> {
        Backend::bound_skills(self, project, phase).await
    }
    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
        Backend::missing_tags(self, item, box_id).await
    }
    async fn queued_runs_on_box(&self, box_id: BoxId) -> Result<Vec<(RunId, DateTime<Utc>)>> {
        Backend::queued_runs_on_box(self, box_id).await
    }
}

#[cfg(test)]
mod tests {
    //! Compile-first pins (MOD-41 T3, plan P-4 on the real types). The traits are named by path
    //! here too, and no method is called on a concrete store: only the generic `probe` calls one.

    use super::*;

    /// Compiles only when `S` is a worker store.
    fn is_worker<S: htui_core::store::WorkerStore>() {}

    /// Compiles only when `H` is a worker host.
    fn is_host<H: htui_core::store::WorkerHost>() {}

    #[test]
    fn pg_and_writer_are_worker_stores() {
        is_worker::<PgStore>();
        is_worker::<Writer>();
    }

    #[test]
    fn backend_and_pg_are_worker_hosts() {
        is_host::<Backend>();
        is_host::<PgStore>();
    }

    /// A generic worker-store future, as the supervisor will spawn one.
    async fn probe<S: htui_core::store::WorkerStore + Clone + 'static>(
        s: S,
    ) -> Result<Option<Run>> {
        s.run(RunId::new()).await
    }

    /// Never called: it only has to type-check, which needs `probe`'s future to be `Send`.
    fn spawnable<S: htui_core::store::WorkerStore + Clone + 'static>(s: S) {
        drop(tokio::spawn(probe(s)));
    }

    #[test]
    fn pg_and_writer_worker_futures_spawn() {
        let _: fn(PgStore) = spawnable;
        let _: fn(Writer) = spawnable;
    }
}
