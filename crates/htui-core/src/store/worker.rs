//! The store surfaces the run supervisor and the engine compile against (MOD-41 plan D4).
//!
//! Three traits, each exactly its call sites at MOD-41's HEAD: [`RecorderStore`] for
//! `htui_agent::record`, [`WorkerStore`] for the engine, `gate` and `graph::resolve`, and
//! [`WorkerHost`] for the supervisor. None has a default body, so every implementor is found by
//! the compiler. They are **not** blanket-implemented over [`WriteStore`]: such an impl cannot
//! prove its futures `Send` (plan P-1), and the supervisor spawns them. Never `use` these traits in
//! a module that also sees [`ReadStore`]/[`WriteStore`] (E0034, plan D5): bound by path.
//!
//! Every method is declared `fn … -> impl Future<Output = Result<T>> + Send` (plan P-2), and an
//! implementor writes `async fn`. The `MemStore` impls below forward to the [`ReadStore`] or
//! [`WriteStore`] method of the same name by path (UFCS): this module defines the new traits, so
//! both families are in scope here and a method-call body would be ambiguous.
//!
//! `WorkerStore` is 59 methods: 13 [`ReadStore`] reads, 6 [`WriteStore`] reads, 25 writes,
//! `write_document`, MOD-42's three command methods, MOD-11's four agent writes and
//! `item_by_key` (plan D13, B-4), its five command-queue methods (plan D14) and MOD-12 M2's
//! `run_batch_spend`; [`RelayStore`] is nine (MOD-42 plan D2, MOD-70 plan D4).
//! `WorkerHost` is `writer` plus 20 reads (blueprint F-5); MOD-41 T7 adds the 22nd,
//! `queued_runs_on_box`; MOD-12 M1 adds seven (plan D3, D5, D6), the 29th `close_drained_batch`;
//! MOD-12 M2 adds two (`batch_spend`, `project_settings`), 31.

use std::collections::BTreeMap;
use std::future::Future;

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::model::link::{ItemLink, ProposeLink, WithdrawLink};
use crate::model::{
    AgentBox, AgentId, AgentSummary, BatchId, BoundSkill, BoxId, BoxInfo, BoxProfile, BoxRow,
    CancelRequest, Claim, CommandRun, CommandRunId, CommandRunStatus, Document, DocumentHead,
    DocumentId, FollowUpSettle, GateOutcome, Item, ItemId, ItemKind, ItemSummary, NewCommandRun,
    NewDocument, NewNote, NewRun, NewRunStep, Note, OpenPermission, PermissionChoice, PermissionId,
    PermissionStatus, PhaseAgent, PhaseId, Project, ProjectId, PromptScope, PromptTemplate,
    QueueBatch, QueueEntry, QueuedFollowUp, RelaySessionId, Repo, RepoBoxPath, RepoId, Resolution,
    ResolvedGraph, ResolvedInput, Run, RunCommand, RunCommandId, RunCommandStatus, RunId,
    RunStatus, RunStep, RunStepCommit, RunStepTree, RunSummary, Scope, SessionEvent, SettleOutcome,
    Status, StepGraphId, StepGraphPhase, StepId, StepOutcome, StepPermission, StepStatus,
    UpstreamEntry, UserId, WorkspaceSummary,
};
use crate::store::error::Result;
use crate::store::mem::MemStore;
use crate::store::traits::{ParkOutcome, ReadStore, StepFence, WriteStore};

/// What a session recorder writes (`htui_agent::record`, plan D4).
pub trait RecorderStore: Send + Sync {
    /// [`WriteStore::append_events`].
    fn append_events(
        &self,
        fence: StepFence,
        events: &[SessionEvent],
    ) -> impl Future<Output = Result<usize>> + Send;
    /// [`WriteStore::set_step_usage`].
    fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::set_agent_box_quota`].
    fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool>> + Send;
}

/// What `htui_agent::record::drive` writes and reads while a request is parked (MOD-42 plan D2,
/// D6), and its follow-up window (MOD-70 D4). Separate from [`RecorderStore`] so `drive` can take
/// a recorder store and a relay store as two type parameters: the spies implement only the first
/// (plan probe A1b).
pub trait RelayStore: Send + Sync {
    /// [`WriteStore::open_permission`].
    fn open_permission(
        &self,
        open: OpenPermission,
    ) -> impl Future<Output = Result<PermissionId>> + Send;
    /// [`WriteStore::permission`].
    fn permission(
        &self,
        id: PermissionId,
    ) -> impl Future<Output = Result<Option<StepPermission>>> + Send;
    /// [`WriteStore::apply_permission`].
    fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> impl Future<Output = Result<Option<PermissionChoice>>> + Send;
    /// [`WriteStore::settle_permissions`].
    fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> impl Future<Output = Result<u64>> + Send;
    /// [`WriteStore::open_follow_ups`].
    fn open_follow_ups(
        &self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::next_follow_up`].
    fn next_follow_up(
        &self,
        step: StepId,
        session: RelaySessionId,
    ) -> impl Future<Output = Result<Option<QueuedFollowUp>>> + Send;
    /// [`WriteStore::settle_follow_up`].
    fn settle_follow_up(
        &self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
    ) -> impl Future<Output = Result<SettleOutcome>> + Send;
    /// [`WriteStore::close_follow_ups`].
    fn close_follow_ups(
        &self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
    ) -> impl Future<Output = Result<u64>> + Send;
    /// [`WriteStore::close_dropped_follow_ups`].
    fn close_dropped_follow_ups(
        &self,
        run: RunId,
        owner: Uuid,
        reason: &str,
    ) -> impl Future<Output = Result<u64>> + Send;
}

/// Every store call of the engine, `gate`, `graph::resolve` and the progress sink (plan D4).
///
/// `graph::override_graph`'s eight extra methods are deliberately absent: it has no production
/// caller and keeps its [`WriteStore`] bound.
pub trait WorkerStore: RecorderStore + RelayStore {
    // -- 13 ReadStore reads
    /// [`ReadStore::item`].
    fn item(&self, id: ItemId) -> impl Future<Output = Result<Option<Item>>> + Send;
    /// [`ReadStore::documents`].
    fn documents(&self, id: ItemId) -> impl Future<Output = Result<Vec<DocumentHead>>> + Send;
    /// [`ReadStore::runs`].
    fn runs(&self, id: ItemId) -> impl Future<Output = Result<Vec<RunSummary>>> + Send;
    /// [`ReadStore::step_events`].
    fn step_events(
        &self,
        step: StepId,
    ) -> impl Future<Output = Result<Option<Vec<SessionEvent>>>> + Send;
    /// [`ReadStore::document`].
    fn document(&self, id: DocumentId) -> impl Future<Output = Result<Option<Document>>> + Send;
    /// [`ReadStore::documents_of_kinds`].
    fn documents_of_kinds(
        &self,
        item: ItemId,
        kinds: &[String],
    ) -> impl Future<Output = Result<Vec<Document>>> + Send;
    /// [`ReadStore::upstream_summaries`].
    fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> impl Future<Output = Result<Vec<UpstreamEntry>>> + Send;
    /// [`ReadStore::project`].
    fn project(&self, id: ProjectId) -> impl Future<Output = Result<Option<Project>>> + Send;
    /// [`ReadStore::run`].
    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>>> + Send;
    /// [`ReadStore::run_steps`].
    fn run_steps(&self, run: RunId) -> impl Future<Output = Result<Vec<RunStep>>> + Send;
    /// [`ReadStore::step_trees`].
    fn step_trees(&self, step: StepId) -> impl Future<Output = Result<Vec<RunStepTree>>> + Send;
    /// [`ReadStore::step_commits`].
    fn step_commits(&self, step: StepId)
    -> impl Future<Output = Result<Vec<RunStepCommit>>> + Send;
    /// [`ReadStore::resolve_inputs`].
    fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> impl Future<Output = Result<Vec<ResolvedInput>>> + Send;

    // -- 5 WriteStore reads
    /// [`WriteStore::repos`].
    fn repos(&self, project: ProjectId) -> impl Future<Output = Result<Vec<Repo>>> + Send;
    /// [`WriteStore::repo_box_paths`].
    fn repo_box_paths(&self, repo: RepoId)
    -> impl Future<Output = Result<Vec<RepoBoxPath>>> + Send;
    /// [`WriteStore::item_kinds`].
    fn item_kinds(&self, project: ProjectId) -> impl Future<Output = Result<Vec<ItemKind>>> + Send;
    /// [`WriteStore::phases`].
    fn phases(
        &self,
        graph: StepGraphId,
    ) -> impl Future<Output = Result<Vec<StepGraphPhase>>> + Send;
    /// [`WriteStore::command_runs`].
    fn command_runs(&self, step: StepId) -> impl Future<Output = Result<Vec<CommandRun>>> + Send;
    /// [`WriteStore::relay_view`]: the engine stales a dropped walk's parked requests with it
    /// (MOD-42 L-1).
    fn relay_view(
        &self,
        item: ItemId,
    ) -> impl Future<Output = Result<crate::model::RelayView>> + Send;

    // -- 25 writes
    /// [`WriteStore::transition`].
    fn transition(
        &self,
        id: ItemId,
        from: Status,
        to: Status,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::set_step_prompt`].
    fn set_step_prompt(
        &self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::create_run`].
    fn create_run(&self, new: NewRun) -> impl Future<Output = Result<Run>> + Send;
    /// [`WriteStore::claim_run`].
    fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> impl Future<Output = Result<Claim>> + Send;
    /// [`WriteStore::refresh_lease`].
    fn refresh_lease(
        &self,
        run: RunId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::adopt_runs`].
    fn adopt_runs(
        &self,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> impl Future<Output = Result<Vec<Run>>> + Send;
    /// [`WriteStore::take_lease`].
    fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::release_lease`].
    fn release_lease(&self, run: RunId, owner: Uuid) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::create_step`].
    fn create_step(&self, new: NewRunStep) -> impl Future<Output = Result<RunStep>> + Send;
    /// [`WriteStore::transition_run`].
    fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::transition_step`].
    fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::finish_step`].
    fn finish_step(
        &self,
        fence: StepFence,
        step: StepId,
        outcome: StepOutcome,
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::interrupt_step`].
    fn interrupt_step(
        &self,
        step: StepId,
        note: &str,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::answer_gate`].
    fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::select_fanout`].
    fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::supersede_step`].
    fn supersede_step(&self, step: StepId) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::upsert_step_tree`].
    fn upsert_step_tree(
        &self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::record_commits`].
    fn record_commits(
        &self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::record_command_run`].
    fn record_command_run(
        &self,
        new: NewCommandRun,
    ) -> impl Future<Output = Result<CommandRun>> + Send;
    /// [`WriteStore::promote_step`].
    fn promote_step(
        &self,
        step: StepId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::pass_step`].
    fn pass_step(
        &self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::park_step`].
    fn park_step(
        &self,
        fence: StepFence,
        step: StepId,
    ) -> impl Future<Output = Result<ParkOutcome>> + Send;
    /// [`WriteStore::finish_run`].
    fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::close_out`].
    fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> impl Future<Output = Result<Document>> + Send;
    /// [`WriteStore::add_note`].
    fn add_note(&self, note: NewNote) -> impl Future<Output = Result<Note>> + Send;

    // -- the progress sink's document write (plan D4, PRD D5: no production caller until MOD-11)
    /// [`WriteStore::write_document`].
    fn write_document(&self, new: NewDocument) -> impl Future<Output = Result<Document>> + Send;

    // -- MOD-42 (plan D2, D12, D13): the command side
    /// [`WriteStore::request_cancel`].
    fn request_cancel(
        &self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
    ) -> impl Future<Output = Result<CancelRequest>> + Send;
    /// [`WriteStore::pending_commands`].
    fn pending_commands(
        &self,
        owner: Uuid,
        box_id: BoxId,
    ) -> impl Future<Output = Result<Vec<RunCommand>>> + Send;
    /// [`WriteStore::resolve_command`].
    fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> impl Future<Output = Result<bool>> + Send;

    // -- MOD-11 (plan D13, B-4): the agent writes and the key lookup the MCP tools make
    /// [`WriteStore::write_step_document`].
    fn write_step_document(
        &self,
        fence: StepFence,
        new: NewDocument,
    ) -> impl Future<Output = Result<Document>> + Send;
    /// [`WriteStore::add_step_note`].
    fn add_step_note(
        &self,
        fence: StepFence,
        note: NewNote,
    ) -> impl Future<Output = Result<Note>> + Send;
    /// [`WriteStore::propose_link`].
    fn propose_link(
        &self,
        fence: StepFence,
        link: ProposeLink,
    ) -> impl Future<Output = Result<ItemLink>> + Send;
    /// [`WriteStore::withdraw_link`].
    fn withdraw_link(
        &self,
        fence: StepFence,
        link: WithdrawLink,
    ) -> impl Future<Output = Result<ItemLink>> + Send;
    /// [`WriteStore::item_by_key`].
    fn item_by_key(
        &self,
        project: ProjectId,
        key: &str,
    ) -> impl Future<Output = Result<Option<ItemId>>> + Send;
    /// [`WriteStore::lease_holds`].
    fn lease_holds(
        &self,
        run: RunId,
        fence: StepFence,
    ) -> impl Future<Output = Result<bool>> + Send;

    // -- MOD-11 M4 (plan D14): the command queue `command_run` drives
    /// [`WriteStore::enqueue_command`].
    fn enqueue_command(
        &self,
        new: NewCommandRun,
    ) -> impl Future<Output = Result<CommandRun>> + Send;
    /// [`WriteStore::claim_command`].
    fn claim_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        limit: u32,
    ) -> impl Future<Output = Result<Option<CommandRun>>> + Send;
    /// [`WriteStore::beat_command`].
    fn beat_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::finish_command`].
    fn finish_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        status: CommandRunStatus,
        exit_code: Option<i32>,
        output: Option<String>,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// [`WriteStore::cancel_command`].
    fn cancel_command(&self, id: CommandRunId) -> impl Future<Output = Result<bool>> + Send;

    /// MOD-12 M2 D5: the batch `run` was admitted under and that batch's spend
    /// (`MemStore::run_batch_spend`); `None` for a manual or chat run. A `WorkerStore` read because
    /// [`Run`] carries no `batch_id` and the mirror is untouched (plan D5).
    fn run_batch_spend(
        &self,
        run: RunId,
    ) -> impl Future<Output = Result<Option<(BatchId, Option<i64>)>>> + Send;
}

/// The supervisor's source (plan D4): the process's store and every read the runtime makes.
///
/// Implemented by `htui_store`'s `PgStore` (the headless worker's host) and `Backend` (the TUI's);
/// the reads named `Backend::…` below are inherent on both.
pub trait WorkerHost: Clone + Send + Sync + 'static {
    /// The store engines write through.
    type Store: WorkerStore + Clone + Send + Sync + 'static;

    /// The store, or `None` off the server (`Backend::writer`).
    fn writer(&self) -> Option<Self::Store>;

    // -- 9 of Backend's inherent reads
    /// `Backend::box_info`.
    fn box_info(&self) -> impl Future<Output = Result<Option<BoxInfo>>> + Send;
    /// `Backend::this_user`.
    fn this_user(&self) -> impl Future<Output = Result<UserId>> + Send;
    /// `Backend::app_settings`.
    fn app_settings(&self) -> impl Future<Output = Result<BTreeMap<String, Value>>> + Send;
    /// `Backend::box_profile`.
    fn box_profile(&self, id: BoxId) -> impl Future<Output = Result<Option<BoxProfile>>> + Send;
    /// `Backend::agents`.
    fn agents(&self) -> impl Future<Output = Result<Vec<AgentSummary>>> + Send;
    /// `Backend::box_row`.
    fn box_row(&self, id: BoxId) -> impl Future<Output = Result<Option<BoxRow>>> + Send;
    /// `Backend::repo_paths`.
    fn repo_paths(&self, box_id: BoxId) -> impl Future<Output = Result<Vec<RepoBoxPath>>> + Send;
    /// `Backend::workspaces`.
    fn workspaces(&self) -> impl Future<Output = Result<Vec<WorkspaceSummary>>> + Send;
    /// `Backend::active_runs_on_box`.
    fn active_runs_on_box(&self, box_id: BoxId) -> impl Future<Output = Result<usize>> + Send;

    // -- 5 ReadStore reads on the host
    /// [`ReadStore::item`].
    fn item(&self, id: ItemId) -> impl Future<Output = Result<Option<Item>>> + Send;
    /// [`ReadStore::documents`].
    fn documents(&self, id: ItemId) -> impl Future<Output = Result<Vec<DocumentHead>>> + Send;
    /// [`ReadStore::runs`].
    fn runs(&self, id: ItemId) -> impl Future<Output = Result<Vec<RunSummary>>> + Send;
    /// [`ReadStore::run`].
    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>>> + Send;
    /// [`ReadStore::run_steps`].
    fn run_steps(&self, run: RunId) -> impl Future<Output = Result<Vec<RunStep>>> + Send;

    // -- 6 inherent reads behind GraphSource
    /// `Backend::resolve_graph`.
    fn resolve_graph(
        &self,
        item: ItemId,
    ) -> impl Future<Output = Result<Option<ResolvedGraph>>> + Send;
    /// `Backend::phase_agents`.
    fn phase_agents(&self, phase: PhaseId) -> impl Future<Output = Result<Vec<PhaseAgent>>> + Send;
    /// `Backend::prompt_template`.
    fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> impl Future<Output = Result<Option<PromptTemplate>>> + Send;
    /// `Backend::agent_boxes`.
    fn agent_boxes(&self, box_id: BoxId) -> impl Future<Output = Result<Vec<AgentBox>>> + Send;
    /// `Backend::bound_skills`.
    fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> impl Future<Output = Result<Vec<BoundSkill>>> + Send;
    /// `Backend::missing_tags`.
    fn missing_tags(
        &self,
        item: ItemId,
        box_id: BoxId,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    // -- MOD-41 T7 (plan D11)
    /// `Backend::queued_runs_on_box`: this box's `queued` runs, `(id, queued_at)` in
    /// `(queued_at, id)` order.
    fn queued_runs_on_box(
        &self,
        box_id: BoxId,
    ) -> impl Future<Output = Result<Vec<(RunId, DateTime<Utc>)>>> + Send;

    // -- MOD-12 M1 (plan D3, D5, D6): the queue runner's reads and its one write
    /// `Backend::ready_items`: §7.4's ready items `box_id` can take, in queue order.
    fn ready_items(
        &self,
        scope: &Scope,
        box_id: BoxId,
    ) -> impl Future<Output = Result<Vec<ItemSummary>>> + Send;
    /// `Backend::running_runs_on_box`: `claim_run`'s slot count.
    fn running_runs_on_box(&self, box_id: BoxId) -> impl Future<Output = Result<usize>> + Send;
    /// `Backend::queue_entries`.
    fn queue_entries(&self, box_id: BoxId) -> impl Future<Output = Result<Vec<QueueEntry>>> + Send;
    /// `Backend::open_batch_of`.
    fn open_batch_of(
        &self,
        box_id: BoxId,
    ) -> impl Future<Output = Result<Option<QueueBatch>>> + Send;
    /// `Backend::batch_cancelled_items`: the items a cancel keeps out of `batch` (review H1).
    fn batch_cancelled_items(
        &self,
        batch: BatchId,
    ) -> impl Future<Output = Result<Vec<ItemId>>> + Send;
    /// `Backend::prune_finished_entries`.
    fn prune_finished_entries(&self, box_id: BoxId) -> impl Future<Output = Result<u64>> + Send;
    /// `Backend::close_drained_batch` (the drain, D3; review M1): `seen` is the items of the
    /// entries the runner read, empty when it read none (review R1 L2).
    fn close_drained_batch(
        &self,
        batch: BatchId,
        seen: &[ItemId],
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<QueueBatch>>> + Send;

    // -- MOD-12 M2 (plan D4): the runner's spend gate
    /// `Backend::batch_spend`: the batch's spend, `None` when no step reports a cost (plan D1).
    fn batch_spend(&self, batch: BatchId) -> impl Future<Output = Result<Option<i64>>> + Send;
    /// `Backend::project_settings`: the project's `settings` blob, read live at admission
    /// (plan D2); `None` for an unknown project.
    fn project_settings(
        &self,
        project: ProjectId,
    ) -> impl Future<Output = Result<Option<Value>>> + Send;
}

impl RecorderStore for MemStore {
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

impl RelayStore for MemStore {
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
    async fn open_follow_ups(
        &self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
    ) -> Result<bool> {
        WriteStore::open_follow_ups(self, run, step, session, owner).await
    }
    async fn next_follow_up(
        &self,
        step: StepId,
        session: RelaySessionId,
    ) -> Result<Option<QueuedFollowUp>> {
        WriteStore::next_follow_up(self, step, session).await
    }
    async fn settle_follow_up(
        &self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
    ) -> Result<SettleOutcome> {
        WriteStore::settle_follow_up(self, id, owner, to).await
    }
    async fn close_follow_ups(
        &self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
    ) -> Result<u64> {
        WriteStore::close_follow_ups(self, step, session, reason).await
    }
    async fn close_dropped_follow_ups(&self, run: RunId, owner: Uuid, reason: &str) -> Result<u64> {
        WriteStore::close_dropped_follow_ups(self, run, owner, reason).await
    }
}

impl WorkerStore for MemStore {
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
    async fn relay_view(&self, item: ItemId) -> Result<crate::model::RelayView> {
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
    async fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
        WriteStore::lease_holds(self, run, fence).await
    }
    async fn enqueue_command(&self, new: NewCommandRun) -> Result<CommandRun> {
        WriteStore::enqueue_command(self, new).await
    }
    async fn claim_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        limit: u32,
    ) -> Result<Option<CommandRun>> {
        WriteStore::claim_command(self, id, claimant, limit).await
    }
    async fn beat_command(&self, id: CommandRunId, claimant: Uuid) -> Result<bool> {
        WriteStore::beat_command(self, id, claimant).await
    }
    async fn finish_command(
        &self,
        id: CommandRunId,
        claimant: Uuid,
        status: CommandRunStatus,
        exit_code: Option<i32>,
        output: Option<String>,
    ) -> Result<bool> {
        WriteStore::finish_command(self, id, claimant, status, exit_code, output).await
    }
    async fn cancel_command(&self, id: CommandRunId) -> Result<bool> {
        WriteStore::cancel_command(self, id).await
    }
    async fn run_batch_spend(&self, run: RunId) -> Result<Option<(BatchId, Option<i64>)>> {
        MemStore::run_batch_spend(self, run).await
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    //! Compile-first pins (MOD-41 T3). This module sees both trait families through `super::*`,
    //! so it calls no method on a concrete store: only the generic `probe` does (E0034, plan D5).

    use super::*;

    /// Compiles only when `S` is a [`RecorderStore`].
    fn is_recorder<S: RecorderStore>() {}

    /// Compiles only when `S` is a [`WorkerStore`].
    fn is_worker<S: WorkerStore>() {}

    /// Compiles only when `S` is a [`RelayStore`] (MOD-42 plan D2).
    fn is_relay<S: RelayStore>() {}

    #[test]
    fn worker_store_is_object_of_the_engines_calls() {
        is_recorder::<MemStore>();
        is_relay::<MemStore>();
        is_worker::<MemStore>();
    }

    /// A generic worker-store future, as the supervisor will spawn one (plan P-4).
    async fn probe<S: WorkerStore + Clone + 'static>(s: S) -> Result<Option<Run>> {
        s.run(RunId::new()).await
    }

    #[tokio::test]
    async fn a_mem_store_worker_future_spawns() {
        let answer = tokio::spawn(probe(MemStore::demo()))
            .await
            .expect("the probe task does not panic");
        assert!(
            matches!(answer, Ok(None)),
            "a run id no row has reads as `None`, got {answer:?}"
        );
    }
}
