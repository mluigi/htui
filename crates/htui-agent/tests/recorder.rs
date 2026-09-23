//! Recorder unit tests (plan MOD-2 T4, design row D6).
//!
//! Every case drives a scripted [`DriverEnvelope`] vector through a [`Recorder`] into a
//! `MemStore::demo()` and asserts on the **persisted rows** read back through
//! [`ReadStore::step_events`]. No driver, no process and no transport is involved: the recorder is
//! the unit under test, and `docs/ANA-4.md` §11 criteria 2, 3 and 4 are statements about the
//! store, not about a `Vec` in memory.
//!
//! `run_step.usage` and `run_step.prompt_digest` have no reader on the `ReadStore` /`WriteStore`
//! seam (`RunStepSummary` carries neither, and a chat run's `item_id` is `NULL`, so
//! `ReadStore::runs` cannot reach the step at all). [`SpyStore`] therefore wraps `MemStore` and
//! records the [`WriteStore::set_step_usage`] calls the recorder makes, which is the recorder's
//! half of criterion 3; that `set_step_usage` then lands in `run_step` is T2's conformance case
//! `set_step_usage_writes_usage_and_digest`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::driver::{AgentSession, AgentSessionRef, DriverFuture, PermissionAnswer};
use htui_agent::error::DriverError;
use htui_agent::event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, OtherEvent, PermissionAnswerEvent,
    PermissionOption, PermissionOptionKind, PermissionRequestEvent, StopReason, TextChunk,
    ToolCallEvent, ToolKind, ToolResultEvent, ToolResultStatus, UsageEvent,
};
use htui_agent::record::{
    AnsweredBy, CHUNK_FLUSH_BYTES, CapBreach, QuotaLatch, RecordError, Recorder, RunCap, pump,
};
use htui_core::fixtures::ids;
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, BoxId, ChatRunSpec, Claim, CommandRun, Document,
    DocumentHead, DocumentId, EventKind, EventRole, GateOutcome, Item, ItemFilter, ItemId,
    ItemKind, ItemKindId, ItemKindPatch, ItemPatch, ItemSummary, LinkGraph, NewCommandRun,
    NewDocument, NewItem, NewItemKind, NewNote, NewProject, NewRepo, NewRun, NewRunStep,
    NewStepGraph, NewWorkspace, Note, PhaseId, PhasePatch, Project, ProjectId, ProjectPatch,
    PromptScope, Quota, QuotaSource, Repo, RepoBoxPath, RepoId, RepoPatch, ResolvedInput, Run,
    RunId, RunStatus, RunStep, RunStepCommit, RunStepTree, RunSummary, Scope, SessionEvent, Status,
    StepGraph, StepGraphId, StepGraphPatch, StepGraphPhase, StepId, StepOutcome, StepStatus,
    UpstreamEntry, Workspace, WorkspaceBoxPath, WorkspaceId, WorkspacePatch, WorkspaceProject,
    normalize,
};
use htui_core::prompt::settings::SettingKey;
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{
    CasOutcome, DeleteReach, DeleteTarget, MemStore, ReadStore, Result as StoreResult, SettingRung,
    StoreError, StoredSetting, UpdateOutcome, WriteStore,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// The one secret every case masks: the value a `SessionSpec.env` entry would carry.
const SECRET: &str = "fake-secret-9f8e7d";

/// A fixed capture time, so a persisted row is a function of the script and of nothing else.
fn at() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_788_393_600_000).expect("the demo epoch is a valid instant")
}

/// The scrubber every case uses: `MinimalScrubber` over the one env value.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([SECRET.to_owned()])
}

/// An envelope with no `raw`.
fn env(event: DriverEvent) -> DriverEnvelope {
    DriverEnvelope {
        event,
        raw: None,
        at: at(),
    }
}

/// An envelope carrying a verbatim wire message.
fn raw_env(event: DriverEvent, raw: Value) -> DriverEnvelope {
    DriverEnvelope {
        event,
        raw: Some(raw),
        at: at(),
    }
}

/// One assistant chunk in a named message group.
fn chunk(text: &str, message_id: &str) -> DriverEnvelope {
    env(DriverEvent::AssistantChunk(TextChunk {
        text: text.to_owned(),
        message_id: Some(message_id.to_owned()),
    }))
}

/// A minimal tool call.
fn tool_call(id: &str) -> DriverEnvelope {
    env(DriverEvent::ToolCall(ToolCallEvent {
        tool_call_id: id.to_owned(),
        title: "read a file".to_owned(),
        tool_kind: ToolKind::Read,
        input: json!({ "path": "README.md" }),
        locations: Vec::new(),
    }))
}

/// A tool result whose output is whatever the case wants scrubbed.
fn tool_result(id: &str, output: Value) -> DriverEnvelope {
    env(DriverEvent::ToolResult(ToolResultEvent {
        tool_call_id: id.to_owned(),
        status: ToolResultStatus::Completed,
        output: Some(output),
        locations: Vec::new(),
        terminal_reason: None,
    }))
}

/// The `run` / `run_step` pair every case records into. Minted once so two stores can be given
/// the *same* step id, which is what makes the criterion 2 replay comparison meaningful.
fn chat_spec() -> ChatRunSpec {
    ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        ids::BOX,
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        Some("sonnet".to_owned()),
    )
}

/// A demo store with the chat's two rows already minted.
async fn open_chat(chat: &ChatRunSpec) -> SpyStore {
    let store = SpyStore::demo();
    store
        .start_chat_run(chat)
        .await
        .expect("the chat run and step must mint");
    store
}

/// The persisted log of a step, in `seq` order.
async fn rows(store: &SpyStore, step: StepId) -> Vec<SessionEvent> {
    store
        .step_events(step)
        .await
        .expect("reading the log must not fail")
        .expect("the chat step has a log")
}

/// `payload.text` of a row.
fn text_of(row: &SessionEvent) -> &str {
    row.payload
        .get("text")
        .and_then(Value::as_str)
        .expect("the row carries a `text` key")
}

// ---------------------------------------------------------------------------------------------
// `SpyStore`: `MemStore` plus a log of the `set_step_usage` calls
// ---------------------------------------------------------------------------------------------

/// One `set_step_usage` call the recorder made.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageCall {
    /// The `usage` document written to `run_step.usage`.
    usage: Value,
    /// The digest argument: `Some` exactly once per step (plan D15(b), X8).
    prompt_digest: Option<String>,
}

/// One `set_agent_box_quota` call the recorder **attempted** (MOD-2 plan D66-D68).
///
/// An attempt and not a success: the latch is best-effort and gives up on a refusal, so "it did
/// not retry" is a statement about how many times it tried, and a log of successes could not make
/// it.
#[derive(Debug, Clone, PartialEq)]
struct QuotaCall {
    /// The row the latch named.
    key: (AgentId, BoxId),
    /// The §7 document offered.
    quota: Value,
    /// `agent_box.quota_at`, which mirrors the document's `observed_at`.
    quota_at: DateTime<Utc>,
}

/// A `WriteStore` that delegates everything to `MemStore`, remembers the `set_step_usage` and
/// `set_agent_box_quota` calls the read seam does not expose, and can be told to refuse the next
/// few appends or every quota latch.
#[derive(Debug)]
struct SpyStore {
    inner: MemStore,
    usage_calls: Mutex<Vec<UsageCall>>,
    quota_calls: Mutex<Vec<QuotaCall>>,
    /// Appends still to be refused before one is let through again.
    refuse_appends: Mutex<usize>,
    /// The error every `set_agent_box_quota` answers with, if any. Not consumed: a backend that
    /// fails once usually fails again, and the point of the case is what the recorder does with
    /// the second answer.
    refuse_quota: Mutex<Option<StoreError>>,
    /// Every `set_step_prompt` call so far, as `(step, digest)`.
    ///
    /// The pre-flight audit row is as invisible to the read seam as `run_step.usage` is:
    /// `RunStepSummary` carries the two derived figures and neither the digest nor the record, so
    /// the only way to assert *which digest* a caller wrote is to watch the call. MOD-2 milestone
    /// 9's `record_prompt` case is what reads it — the assembler and the recorder compute the same
    /// `Sha256` over the same text, and that is the claim.
    prompt_calls: Mutex<Vec<(StepId, String)>>,
}

impl SpyStore {
    fn demo() -> Self {
        Self {
            inner: MemStore::demo(),
            usage_calls: Mutex::new(Vec::new()),
            quota_calls: Mutex::new(Vec::new()),
            refuse_appends: Mutex::new(0),
            refuse_quota: Mutex::new(None),
            prompt_calls: Mutex::new(Vec::new()),
        }
    }

    /// Every `set_agent_box_quota` attempt so far, in order.
    fn quota_calls(&self) -> Vec<QuotaCall> {
        self.quota_calls
            .lock()
            .expect("the spy log is never poisoned")
            .clone()
    }

    /// Makes every quota latch fail with `error`, for as long as this store lives.
    fn refuse_quota_with(&self, error: StoreError) {
        *self
            .refuse_quota
            .lock()
            .expect("the spy log is never poisoned") = Some(error);
    }

    /// Every `set_step_usage` call so far, in order.
    fn usage_calls(&self) -> Vec<UsageCall> {
        self.usage_calls
            .lock()
            .expect("the spy log is never poisoned")
            .clone()
    }

    /// Every `set_step_prompt` call so far, as `(step, digest)`, in order.
    ///
    /// Unused until MOD-2 milestone 9's T65 asserts that the recorder recomputes the digest the
    /// assembler already wrote; it lands with the trait method so the double is complete rather
    /// than half-written.
    #[expect(dead_code, reason = "written by T62, read by T65's record_prompt case")]
    fn prompt_calls(&self) -> Vec<(StepId, String)> {
        self.prompt_calls
            .lock()
            .expect("the spy log is never poisoned")
            .clone()
    }

    /// Makes the next `count` `append_events` calls fail with the one store error that means
    /// *retrying may work*, writing nothing.
    fn refuse_next_appends(&self, count: usize) {
        *self
            .refuse_appends
            .lock()
            .expect("the spy log is never poisoned") = count;
    }

    /// Whether this append is one of the refused ones, consuming it if so.
    fn refuses_this_append(&self) -> bool {
        let mut left = self
            .refuse_appends
            .lock()
            .expect("the spy log is never poisoned");
        if *left == 0 {
            return false;
        }
        *left -= 1;
        true
    }
}

impl ReadStore for SpyStore {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> StoreResult<Vec<ItemSummary>> {
        self.inner.items(scope, filter).await
    }
    async fn item(&self, id: ItemId) -> StoreResult<Option<Item>> {
        self.inner.item(id).await
    }
    async fn links(&self, id: ItemId, hops: u8) -> StoreResult<LinkGraph> {
        self.inner.links(id, hops).await
    }
    async fn documents(&self, id: ItemId) -> StoreResult<Vec<DocumentHead>> {
        self.inner.documents(id).await
    }
    async fn notes(&self, id: ItemId) -> StoreResult<Vec<Note>> {
        self.inner.notes(id).await
    }
    async fn runs(&self, id: ItemId) -> StoreResult<Vec<RunSummary>> {
        self.inner.runs(id).await
    }
    async fn step_events(&self, step: StepId) -> StoreResult<Option<Vec<SessionEvent>>> {
        self.inner.step_events(step).await
    }
    async fn document(&self, id: DocumentId) -> StoreResult<Option<Document>> {
        self.inner.document(id).await
    }
    async fn documents_of_kinds(
        &self,
        item: ItemId,
        kinds: &[String],
    ) -> StoreResult<Vec<Document>> {
        self.inner.documents_of_kinds(item, kinds).await
    }
    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> StoreResult<Vec<UpstreamEntry>> {
        self.inner.upstream_summaries(id, hops, scope).await
    }
    async fn project(&self, id: ProjectId) -> StoreResult<Option<Project>> {
        self.inner.project(id).await
    }

    // ---- MOD-4 milestone 1: ANA-2 §8's five run reads ---------------------------------------
    //
    // Delegated, none of them logged: this spy watches `set_step_usage`, `set_step_prompt` and
    // the quota latch, and the recorder reads no orchestrated run. They are here because
    // `ReadStore` has no default bodies - a decorator owes every method.

    async fn run(&self, id: RunId) -> StoreResult<Option<Run>> {
        self.inner.run(id).await
    }
    async fn run_steps(&self, run: RunId) -> StoreResult<Vec<RunStep>> {
        self.inner.run_steps(run).await
    }
    async fn step_trees(&self, step: StepId) -> StoreResult<Vec<RunStepTree>> {
        self.inner.step_trees(step).await
    }
    async fn step_commits(&self, step: StepId) -> StoreResult<Vec<RunStepCommit>> {
        self.inner.step_commits(step).await
    }
    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> StoreResult<Vec<ResolvedInput>> {
        self.inner.resolve_inputs(item, run, kinds).await
    }
}

impl WriteStore for SpyStore {
    async fn mint_item(&self, new: NewItem) -> StoreResult<Item> {
        self.inner.mint_item(new).await
    }
    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> StoreResult<UpdateOutcome> {
        self.inner.update_item(id, expected_version, patch).await
    }
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> StoreResult<bool> {
        self.inner.transition(id, from, to).await
    }
    async fn append_events(&self, events: &[SessionEvent]) -> StoreResult<usize> {
        // The guard is dropped before the await: no lock is ever held across one.
        if self.refuses_this_append() {
            return Err(StoreError::Unreachable(
                "the spy store refused this append".to_owned(),
            ));
        }
        self.inner.append_events(events).await
    }
    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> StoreResult<()> {
        self.inner
            .set_step_usage(step, usage.clone(), prompt_digest.clone())
            .await?;
        self.usage_calls
            .lock()
            .expect("the spy log is never poisoned")
            .push(UsageCall {
                usage,
                prompt_digest,
            });
        Ok(())
    }
    async fn upsert_agent(&self, agent: &Agent) -> StoreResult<()> {
        self.inner.upsert_agent(agent).await
    }
    async fn upsert_agent_box(&self, row: &AgentBox) -> StoreResult<()> {
        self.inner.upsert_agent_box(row).await
    }
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> StoreResult<()> {
        // The attempt is logged before the outcome is decided, because a refused latch has to be
        // countable; both guards are taken and dropped with no `.await` in scope.
        self.quota_calls
            .lock()
            .expect("the spy log is never poisoned")
            .push(QuotaCall {
                key: (agent_id, box_id),
                quota: quota.clone(),
                quota_at,
            });
        let refusal = self
            .refuse_quota
            .lock()
            .expect("the spy log is never poisoned")
            .clone();
        match refusal {
            Some(error) => Err(error),
            None => {
                self.inner
                    .set_agent_box_quota(agent_id, box_id, quota, quota_at)
                    .await
            }
        }
    }
    async fn start_chat_run(&self, chat: &ChatRunSpec) -> StoreResult<()> {
        self.inner.start_chat_run(chat).await
    }
    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.inner
            .finish_chat_run(run, step, status, finished_at)
            .await
    }
    async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> StoreResult<()> {
        self.inner.set_step_prompt(step, digest, trim).await?;
        self.prompt_calls
            .lock()
            .expect("the spy log is never poisoned")
            .push((step, digest.to_owned()));
        Ok(())
    }

    // ---- MOD-15 milestone 1: the hierarchy -------------------------------------------------
    //
    // Delegated, none of them logged or refusable: this spy watches the append, the usage row,
    // the quota latch and the prompt row, and a recorder writes no hierarchy row. `WriteStore`
    // has no default bodies, so a decorator owes every method whether a case reaches it or not.

    async fn create_workspace(&self, new: NewWorkspace) -> StoreResult<Workspace> {
        self.inner.create_workspace(new).await
    }
    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> StoreResult<CasOutcome<Workspace>> {
        self.inner.update_workspace(id, expected, patch).await
    }
    async fn workspace(&self, id: WorkspaceId) -> StoreResult<Option<Workspace>> {
        self.inner.workspace(id).await
    }
    async fn upsert_workspace_project(&self, link: &WorkspaceProject) -> StoreResult<()> {
        self.inner.upsert_workspace_project(link).await
    }
    async fn remove_workspace_project(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> StoreResult<()> {
        self.inner
            .remove_workspace_project(workspace, project)
            .await
    }
    async fn workspace_projects(
        &self,
        workspace: WorkspaceId,
    ) -> StoreResult<Vec<WorkspaceProject>> {
        self.inner.workspace_projects(workspace).await
    }
    async fn upsert_workspace_box_path(&self, path: &WorkspaceBoxPath) -> StoreResult<()> {
        self.inner.upsert_workspace_box_path(path).await
    }
    async fn workspace_box_paths(
        &self,
        workspace: WorkspaceId,
    ) -> StoreResult<Vec<WorkspaceBoxPath>> {
        self.inner.workspace_box_paths(workspace).await
    }
    async fn create_project(&self, new: NewProject) -> StoreResult<Project> {
        self.inner.create_project(new).await
    }
    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> StoreResult<CasOutcome<Project>> {
        self.inner.update_project(id, expected, patch).await
    }
    async fn create_repo(&self, new: NewRepo) -> StoreResult<Repo> {
        self.inner.create_repo(new).await
    }
    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> StoreResult<CasOutcome<Repo>> {
        self.inner.update_repo(id, expected, patch).await
    }
    async fn repos(&self, project: ProjectId) -> StoreResult<Vec<Repo>> {
        self.inner.repos(project).await
    }
    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> StoreResult<()> {
        self.inner.upsert_repo_box_path(path).await
    }
    async fn repo_box_paths(&self, repo: RepoId) -> StoreResult<Vec<RepoBoxPath>> {
        self.inner.repo_box_paths(repo).await
    }
    async fn create_item_kind(&self, new: NewItemKind) -> StoreResult<ItemKind> {
        self.inner.create_item_kind(new).await
    }
    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> StoreResult<CasOutcome<ItemKind>> {
        self.inner.update_item_kind(id, expected, patch).await
    }
    async fn item_kinds(&self, project: ProjectId) -> StoreResult<Vec<ItemKind>> {
        self.inner.item_kinds(project).await
    }
    async fn delete_item_kind(&self, id: ItemKindId) -> StoreResult<()> {
        self.inner.delete_item_kind(id).await
    }
    async fn create_step_graph(&self, new: NewStepGraph) -> StoreResult<StepGraph> {
        self.inner.create_step_graph(new).await
    }
    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> StoreResult<CasOutcome<StepGraph>> {
        self.inner.update_step_graph(id, expected, patch).await
    }
    async fn step_graphs(&self, project: ProjectId) -> StoreResult<Vec<StepGraph>> {
        self.inner.step_graphs(project).await
    }
    async fn create_phase(&self, phase: &StepGraphPhase) -> StoreResult<StepGraphPhase> {
        self.inner.create_phase(phase).await
    }
    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> StoreResult<CasOutcome<StepGraphPhase>> {
        self.inner.update_phase(id, expected, patch).await
    }
    async fn phases(&self, graph: StepGraphId) -> StoreResult<Vec<StepGraphPhase>> {
        self.inner.phases(graph).await
    }
    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> StoreResult<CasOutcome<StoredSetting>> {
        self.inner.set_setting(rung, key, value, expected).await
    }
    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> StoreResult<CasOutcome<StoredSetting>> {
        self.inner.clear_setting(rung, key, expected).await
    }
    async fn setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
    ) -> StoreResult<Option<StoredSetting>> {
        self.inner.setting(rung, key).await
    }
    async fn delete_reach(&self, target: DeleteTarget) -> StoreResult<Option<DeleteReach>> {
        self.inner.delete_reach(target).await
    }
    async fn delete_workspace(&self, id: WorkspaceId) -> StoreResult<DeleteReach> {
        self.inner.delete_workspace(id).await
    }
    async fn delete_project(&self, id: ProjectId) -> StoreResult<DeleteReach> {
        self.inner.delete_project(id).await
    }

    // ---- MOD-4 milestone 1: ANA-2 §8's eighteen run writers ---------------------------------
    //
    // Delegated, none of them logged. The recorder mints nothing: it is handed a `step_id` and
    // writes events, usage and the prompt digest against it. Nothing in MOD-4 changes what this
    // spy counts, so nothing here records.

    async fn create_run(&self, new: NewRun) -> StoreResult<Run> {
        self.inner.create_run(new).await
    }
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> StoreResult<Claim> {
        self.inner
            .claim_run(run, box_id, owner, at, lease_until)
            .await
    }
    async fn refresh_lease(
        &self,
        run: RunId,
        owner: Uuid,
        until: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.refresh_lease(run, owner, until).await
    }
    async fn adopt_runs(
        &self,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> StoreResult<Vec<Run>> {
        self.inner.adopt_runs(box_id, owner, now, lease_until).await
    }
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.take_lease(run, box_id, owner, now, until).await
    }
    async fn create_step(&self, new: NewRunStep) -> StoreResult<RunStep> {
        self.inner.create_step(new).await
    }
    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.transition_run(run, from, to, at).await
    }
    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.transition_step(step, from, to, at).await
    }
    async fn finish_step(&self, step: StepId, outcome: StepOutcome) -> StoreResult<()> {
        self.inner.finish_step(step, outcome).await
    }
    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.answer_gate(step, outcome, note, at).await
    }
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> StoreResult<()> {
        self.inner
            .select_fanout(run, position, attempt, winner, reason)
            .await
    }
    async fn supersede_step(&self, step: StepId) -> StoreResult<()> {
        self.inner.supersede_step(step).await
    }
    async fn upsert_step_tree(&self, step: StepId, trees: &[RunStepTree]) -> StoreResult<()> {
        self.inner.upsert_step_tree(step, trees).await
    }
    async fn record_commits(&self, step: StepId, commits: &[RunStepCommit]) -> StoreResult<()> {
        self.inner.record_commits(step, commits).await
    }
    async fn record_command_run(&self, new: NewCommandRun) -> StoreResult<CommandRun> {
        self.inner.record_command_run(new).await
    }
    async fn command_runs(&self, step: StepId) -> StoreResult<Vec<CommandRun>> {
        self.inner.command_runs(step).await
    }
    async fn write_document(&self, new: NewDocument) -> StoreResult<Document> {
        self.inner.write_document(new).await
    }
    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> StoreResult<()> {
        self.inner.promote_step(step, at).await
    }
    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> StoreResult<()> {
        self.inner.fail_run(run, failure, at).await
    }
    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.inner.finish_run(run, to, failure, at).await
    }
    async fn close_out(
        &self,
        item: ItemId,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> StoreResult<Document> {
        self.inner.close_out(item, summary, commits).await
    }
    async fn add_note(&self, note: NewNote) -> StoreResult<Note> {
        self.inner.add_note(note).await
    }
}

// ---------------------------------------------------------------------------------------------
// A scrubber that masks a value whatever its type
// ---------------------------------------------------------------------------------------------

/// Masks the value under one top-level key, whatever JSON type that value has.
///
/// [`MinimalScrubber`] rewrites strings and object keys only, so it cannot make a `usage` payload -
/// five nullable **integers** - stop reading back as a [`UsageEvent`]: an unknown key is ignored by
/// serde and a number is never touched. ANA-7's real rule set is not so limited (MOD-10 replaces
/// the implementation behind this unchanged trait), and a number replaced by `[REDACTED]` is
/// exactly the shape that sends a row down the recorder's unreadable path. This is that scrubber,
/// one key wide, so the `usage` half of that path can be tested at all.
#[derive(Debug)]
struct MaskKey(&'static str);

impl htui_core::scrub::Scrubber for MaskKey {
    fn scrub(&self, value: &mut Value) -> Result<(), htui_core::scrub::Unmasked> {
        if let Some(slot) = value.get_mut(self.0) {
            *slot = Value::String("[REDACTED]".to_owned());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// A scripted `AgentSession`, for `pump` only
// ---------------------------------------------------------------------------------------------

/// Replays a fixed envelope vector through the [`AgentSession`] seam so [`pump`] has something to
/// drive. The real one is T6's `FakeDriver`; this is four stub methods and a queue.
#[derive(Debug)]
struct ScriptedSession {
    events: VecDeque<DriverEnvelope>,
    /// What [`AgentSession::cancel`] puts on the wire, replacing whatever was still queued.
    ///
    /// Both real transports do exactly this: a cancel drops the rest of the turn, synthesizes a
    /// `tool_result` for every call it terminated, and queues the turn's own `done`
    /// (`fake.rs:404-436`, `acp/mod.rs`'s `cancel_session`). A stub that answered a cancel with an
    /// empty queue could not exercise the cap's closing sequence at all, because the sequence is
    /// defined by what the cancel produces.
    on_cancel: Vec<DriverEnvelope>,
}

impl ScriptedSession {
    /// A session that plays `events` and produces nothing of its own on a cancel.
    fn new(events: Vec<DriverEnvelope>) -> Self {
        Self {
            events: VecDeque::from(events),
            on_cancel: Vec::new(),
        }
    }

    /// A session whose cancel drops what is queued and plays `on_cancel` instead.
    fn cancelling(events: Vec<DriverEnvelope>, on_cancel: Vec<DriverEnvelope>) -> Self {
        Self {
            events: VecDeque::from(events),
            on_cancel,
        }
    }
}

impl AgentSession for ScriptedSession {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        None
    }
    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move { Ok(self.events.pop_front()) })
    }
    fn send_follow_up<'a>(&'a mut self, _text: String) -> DriverFuture<'a, ()> {
        Box::pin(async move { Err(DriverError::Closed) })
    }
    fn answer_permission<'a>(
        &'a mut self,
        _request_id: htui_agent::driver::PermissionRequestId,
        _answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async move { Err(DriverError::Closed) })
    }
    fn cancel<'a>(&'a mut self, _grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            self.events.clear();
            self.events.extend(core::mem::take(&mut self.on_cancel));
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Criterion 2: coalescing and replay equality
// ---------------------------------------------------------------------------------------------

/// Twenty `assistant_message_chunk` values across two `message_id` groups, interleaved with one
/// `tool_call`, persist as exactly three `assistant_text` rows and one `tool_call` row in `seq`
/// order; replaying the identical vector into a second store yields byte-identical rows
/// (`docs/ANA-4.md` §11 criterion 2).
#[tokio::test]
async fn coalesces_chunks_and_replays_byte_identically() {
    let script: Vec<DriverEnvelope> = (0..10)
        .map(|i| chunk(&format!("a{i} "), "msg-a"))
        .chain(std::iter::once(tool_call("call-1")))
        .chain((10..15).map(|i| chunk(&format!("a{i} "), "msg-a")))
        .chain((15..20).map(|i| chunk(&format!("b{i} "), "msg-b")))
        .collect();
    assert_eq!(
        script
            .iter()
            .filter(|e| matches!(e.event, DriverEvent::AssistantChunk(_)))
            .count(),
        20,
        "the script is the criterion's twenty chunks"
    );

    let chat = chat_spec();
    let scrubber = scrubber();

    let first = open_chat(&chat).await;
    let mut recorder = Recorder::new(&first, &scrubber, chat.step_id, false, None);
    for envelope in script.clone() {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder
        .finish()
        .await
        .expect("the recorder must close cleanly");
    let a = rows(&first, chat.step_id).await;

    let kinds: Vec<EventKind> = a.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::AssistantText,
            EventKind::ToolCall,
            EventKind::AssistantText,
            EventKind::AssistantText
        ],
        "one row per contiguous run, the tool call between them"
    );
    assert_eq!(
        a.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "seq is gapless from 0 in emission order"
    );
    assert_eq!(text_of(&a[0]), "a0 a1 a2 a3 a4 a5 a6 a7 a8 a9 ");
    assert_eq!(text_of(&a[2]), "a10 a11 a12 a13 a14 ");
    assert_eq!(text_of(&a[3]), "b15 b16 b17 b18 b19 ");
    assert_eq!(
        a[1].tool_call_id.as_deref(),
        Some("call-1"),
        "the tool call id reaches its own column, which idx_session_event_tool joins on"
    );

    let second = open_chat(&chat).await;
    let mut recorder = Recorder::new(&second, &scrubber, chat.step_id, false, None);
    for envelope in script {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder
        .finish()
        .await
        .expect("the recorder must close cleanly");
    let b = rows(&second, chat.step_id).await;

    assert_eq!(
        a, b,
        "the same fixture replayed twice yields identical rows"
    );
    assert_eq!(
        serde_json::to_string(&a).expect("rows serialise"),
        serde_json::to_string(&b).expect("rows serialise"),
        "byte-identical, not merely equal: no wall-clock and no scheduling dependency"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 3: `seq`, `turn` and the prompt digest
// ---------------------------------------------------------------------------------------------

/// A prompt, two follow-ups and their turns: `seq` gapless `0..n`, `turn` `0,1,2`, the `prompt`
/// row at `seq = 0` / `turn = 0` / `role = htui`, and its payload `digest` equal to the digest the
/// recorder wrote through `set_step_usage` (`docs/ANA-4.md` §11 criterion 3).
#[tokio::test]
async fn seq_is_gapless_turns_count_and_digest_reaches_the_step() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record_prompt(
            "summarise the backlog",
            json!([{"name": "task", "tokens": 4, "trimmed": false}]),
            at(),
        )
        .await
        .expect("the prompt row must land");
    recorder
        .record(chunk("first answer", "m1"))
        .await
        .expect("recording must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    recorder
        .record_follow_up("and the risks?", at())
        .await
        .expect("the follow-up row must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    recorder
        .record_follow_up("thanks, stop there", at())
        .await
        .expect("the follow-up row must land");
    let summary = recorder
        .finish()
        .await
        .expect("the recorder must close cleanly");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("the log is short")).collect::<Vec<_>>(),
        "seq is gapless 0..n with exactly one writer"
    );
    assert_eq!(log[0].seq, 0, "the prompt is seq 0");
    assert_eq!(log[0].turn, 0, "the prompt is turn 0");
    assert_eq!(log[0].kind, EventKind::Prompt);
    assert_eq!(
        log[0].role,
        EventRole::Htui,
        "`htui` authors the prompt row"
    );

    let turns: Vec<i32> = log
        .iter()
        .filter(|row| matches!(row.kind, EventKind::Prompt | EventKind::FollowUp))
        .map(|row| row.turn)
        .collect();
    assert_eq!(turns, vec![0, 1, 2], "turn increments once per follow-up");
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::FollowUp)
            .map(|row| row.role)
            .collect::<Vec<_>>(),
        vec![EventRole::User, EventRole::User],
        "a follow-up is the user's row"
    );

    let expected = format!("{:x}", Sha256::digest(b"summarise the backlog"));
    let digest = log[0]
        .payload
        .get("digest")
        .and_then(Value::as_str)
        .expect("the prompt payload carries a digest");
    assert_eq!(digest, expected, "sha256 over the assembled prompt text");
    assert_eq!(
        summary.prompt_digest.as_deref(),
        Some(expected.as_str()),
        "the summary reports the same digest"
    );

    let carried: Vec<Option<String>> = store
        .usage_calls()
        .into_iter()
        .map(|call| call.prompt_digest)
        .collect();
    assert_eq!(
        carried.iter().filter(|d| d.is_some()).count(),
        1,
        "the digest is computed once and written once; later usage writes pass None"
    );
    assert_eq!(
        carried.into_iter().flatten().next(),
        Some(expected),
        "run_step.prompt_digest is the payload digest (X8: Some until milestone 9)"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 4: raw retention
// ---------------------------------------------------------------------------------------------

/// `retain_raw = false` leaves every `raw` NULL; the same script with `retain_raw = true`
/// produces the same rows with `raw` populated (`docs/ANA-4.md` §11 criterion 4).
#[tokio::test]
async fn raw_is_null_unless_retained() {
    let script = || {
        vec![
            raw_env(
                DriverEvent::AssistantChunk(TextChunk {
                    text: "hello ".to_owned(),
                    message_id: Some("m1".to_owned()),
                }),
                json!({ "sessionUpdate": "agent_message_chunk", "n": 1 }),
            ),
            raw_env(
                DriverEvent::AssistantChunk(TextChunk {
                    text: "world".to_owned(),
                    message_id: Some("m1".to_owned()),
                }),
                json!({ "sessionUpdate": "agent_message_chunk", "n": 2 }),
            ),
            raw_env(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
                json!({ "stopReason": "end_turn" }),
            ),
        ]
    };

    let chat = chat_spec();
    let scrubber = scrubber();

    let bare = open_chat(&chat).await;
    let mut recorder = Recorder::new(&bare, &scrubber, chat.step_id, false, None);
    for envelope in script() {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder.finish().await.expect("close");
    let without = rows(&bare, chat.step_id).await;

    let kept = open_chat(&chat).await;
    let mut recorder = Recorder::new(&kept, &scrubber, chat.step_id, true, None);
    for envelope in script() {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder.finish().await.expect("close");
    let with = rows(&kept, chat.step_id).await;

    assert!(
        without.iter().all(|row| row.raw.is_none()),
        "keep_raw_events = false means every row has raw IS NULL"
    );
    assert!(
        with.iter().all(|row| row.raw.is_some()),
        "keep_raw_events = true populates raw on every row"
    );
    assert_eq!(
        without,
        with.iter()
            .map(|row| SessionEvent {
                raw: None,
                ..row.clone()
            })
            .collect::<Vec<_>>(),
        "flipping the flag changes raw and nothing else"
    );
}

// ---------------------------------------------------------------------------------------------
// The remaining D6 obligations
// ---------------------------------------------------------------------------------------------

/// Trigger 4: a run longer than the 16 KiB bound is cut at the bound rather than buffered
/// unboundedly, and the cut is a function of byte counts alone (no timer, `docs/ANA-4.md` §4.1).
#[tokio::test]
async fn chunks_flush_at_the_16_kib_bound() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let piece = "x".repeat(1024);
    for _ in 0..17 {
        recorder
            .record(chunk(&piece, "m1"))
            .await
            .expect("recording must land");
    }
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 2, "17 KiB of chunks is cut once at the bound");
    assert_eq!(
        text_of(&log[0]).len(),
        CHUNK_FLUSH_BYTES,
        "the open run is flushed as soon as it reaches 16 KiB"
    );
    assert_eq!(
        text_of(&log[1]).len(),
        1024,
        "the remainder opens a new run"
    );
    assert_eq!(log[0].seq, 0);
    assert_eq!(log[1].seq, 1);
}

/// An unmapped protocol update lands as kind `other` with its body verbatim, which is what makes
/// "every event" hold without a schema change per protocol revision.
#[tokio::test]
async fn other_event_keeps_its_verbatim_body() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let body = json!({ "sessionId": "sess-1", "modes": ["default", "plan"], "n": 7 });
    recorder
        .record(env(DriverEvent::Other(OtherEvent {
            update: "session_info_update".to_owned(),
            body: body.clone(),
        })))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].kind, EventKind::Other);
    assert_eq!(log[0].role, EventRole::Agent);
    assert_eq!(
        log[0].payload.get("update").and_then(Value::as_str),
        Some("session_info_update")
    );
    assert_eq!(
        log[0].payload.get("body"),
        Some(&body),
        "the body is stored verbatim"
    );
}

/// `usage` rows are deltas; `run_step.usage` is their sum (`docs/ANA-4.md` §4.1, §7).
#[tokio::test]
async fn usage_deltas_sum_into_step_usage() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    for (input, output, cost) in [(10, 5, 100), (20, 7, 250), (0, 3, 1)] {
        recorder
            .record(env(DriverEvent::Usage(UsageEvent {
                input_tokens: Some(input),
                output_tokens: Some(output),
                cost_micros: Some(cost),
                ..UsageEvent::default()
            })))
            .await
            .expect("recording must land");
    }
    let summary = recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 3, "each usage report is its own row");
    assert!(log.iter().all(|row| row.kind == EventKind::Usage));

    let expected = json!({
        "input_tokens": 30,
        "output_tokens": 15,
        "cache_read_tokens": Value::Null,
        "cache_write_tokens": Value::Null,
        "cost_micros": 351,
    });
    assert_eq!(summary.usage, expected, "the summary carries the sum");
    let last = store
        .usage_calls()
        .pop()
        .expect("the recorder wrote run_step.usage");
    assert_eq!(
        last.usage, expected,
        "run_step.usage is the sum of the step's usage rows"
    );
}

/// A `usage` row masking made unreadable is **still summed**, because it is still persisted.
///
/// `run_step.usage` has two writers: this recorder, online, and `upload_pending`, for a chat that
/// happened offline. The uploader sums every persisted `usage` row through
/// `UsageTotals::from_rows`, so a row the recorder persisted but skipped would make an online step
/// and the same step uploaded from a buffer disagree - for exactly one row shape, which is the kind
/// of divergence nobody finds twice (MOD-2 plan D36). The last assertion is that property stated
/// directly: the recorder's document and the uploader's document over the very same rows.
#[tokio::test]
async fn an_unreadable_usage_row_is_still_summed_into_step_usage() {
    let chat = chat_spec();
    // A masked `output_tokens` is a string where an `Option<i64>` belongs, so the payload no longer
    // reads back as a `UsageEvent` and the row takes the recorder's unreadable path.
    let scrubber = MaskKey("output_tokens");
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    for (input, output, cost) in [(10, 5, 100), (20, 7, 250)] {
        recorder
            .record(env(DriverEvent::Usage(UsageEvent {
                input_tokens: Some(input),
                output_tokens: Some(output),
                cost_micros: Some(cost),
                ..UsageEvent::default()
            })))
            .await
            .expect("a masked usage key is not a recording failure");
    }
    let summary = recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Usage, EventKind::Usage],
        "each row keeps the kind the unscrubbed event decided"
    );
    assert_eq!(
        log[0].payload.get("output_tokens"),
        Some(&json!("[REDACTED]")),
        "and the masked document is what was persisted"
    );

    let expected = json!({
        "input_tokens": 30,
        "output_tokens": Value::Null,
        "cache_read_tokens": Value::Null,
        "cache_write_tokens": Value::Null,
        "cost_micros": 350,
    });
    assert_eq!(
        summary.usage, expected,
        "the keys masking left readable still count; the masked one adds nothing"
    );
    assert_eq!(
        store
            .usage_calls()
            .pop()
            .expect("an unreadable usage row still dirties run_step.usage")
            .usage,
        expected,
        "and the number reached run_step.usage"
    );
    assert_eq!(
        htui_core::model::UsageTotals::from_rows(&log).to_value(),
        summary.usage,
        "the recorder's sum and the uploader's sum over the same persisted rows are one document"
    );
}

// ---------------------------------------------------------------------------------------------
// The passive quota latch (`docs/ANA-4.md` §7 `:1131-1135`, plan D66-D68)
// ---------------------------------------------------------------------------------------------

/// The `_meta` rate-limit value of `tests/fixtures/claude_acp_turn.jsonl` line 5, verbatim: the
/// one real blob this milestone was designed against.
fn quota_blob() -> Value {
    json!({
        "status": "allowed",
        "resetsAt": 1_788_801_600_i64,
        "rateLimitType": "five_hour",
        "overageStatus": "rejected",
        "overageDisabledReason": "org_level_disabled",
        "isUsingOverage": false,
        "unifiedWindows": {
            "five_hour": { "utilization": 0.11, "resetsAt": 1_788_801_600_i64 },
            "seven_day": { "utilization": 0.62, "resetsAt": 1_788_854_400_i64 },
        },
    })
}

/// The §4.6 probe snapshot the latched row already carries: the document
/// `set_agent_box_quota` must leave byte-identical (plan D67).
fn probe_snapshot() -> Value {
    json!({ "status": "ready", "source": "probe" })
}

/// A `usage` report: a context reading always, a USD cost when `cost` is `Some`, and a vendor
/// rate-limit blob when `quota` is `Some`.
///
/// `cost: None` is the context-only shape ANA-4 §7 says every ACP turn opens with and `agy`
/// reports throughout.
fn usage(cost: Option<i64>, quota: Option<Value>) -> DriverEnvelope {
    env(DriverEvent::Usage(UsageEvent {
        // The delta, which is the key the recorder sums; the cumulative one is the mapper's and no
        // reader on this path looks at it (`UsageTotals::add_payload` reads five fixed names).
        cost_micros: cost,
        context_used: Some(22_913),
        context_size: Some(1_000_000),
        quota,
        ..UsageEvent::default()
    }))
}

/// The probed `agent_box` row a latch writes into, with `quota` NULL as a fresh row has it (D74).
async fn probe_agent_box(store: &SpyStore) {
    store
        .upsert_agent_box(&AgentBox {
            agent_id: ids::AGENT_CLAUDE,
            box_id: ids::BOX,
            enabled: true,
            version: Some("1.2.3".to_owned()),
            path: Some("agent".to_owned()),
            probed_at: Some(at()),
            quota: None,
            quota_at: None,
            updated_at: at(),
            probe: Some(probe_snapshot()),
        })
        .await
        .expect("the probed row lands");
}

/// The latch the worker builds at chat start: which row to write, plus the two row-side facts the
/// document carries. `source` is `agent.settings.quota.source` and `billing` is `agent.billing` —
/// never the agent's name (`R-AGT-5`).
fn latch(source: QuotaSource, billing: Billing) -> QuotaLatch {
    QuotaLatch {
        agent_id: ids::AGENT_CLAUDE,
        box_id: ids::BOX,
        source,
        billing,
    }
}

/// `agent_box` of this box, read back through the registry projection.
async fn on_box(store: &SpyStore) -> AgentBox {
    store
        .inner
        .agents()
        .await
        .expect("the registry reads")
        .into_iter()
        .find(|row| row.agent.id == ids::AGENT_CLAUDE)
        .expect("the agent is registered")
        .on_box
        .expect("this box has an agent_box row")
}

/// A `usage` row carrying a vendor rate-limit blob leaves `agent_box.quota` equal to §7's
/// seven-key document, with `quota_at` mirroring its `observed_at` (`docs/ANA-4.md:1112-1127`).
///
/// The document is compared against `htui_core::model::normalize` rather than against a literal:
/// the normalizer's own cases pin what the seven keys contain (`model/quota.rs`), and what this
/// case owns is that the recorder latches *that* document, for *that* row, at the capture time of
/// the row that produced it.
#[tokio::test]
async fn a_usage_row_carrying_a_blob_latches_the_seven_key_document() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    probe_agent_box(&store).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None)
        .with_quota_latch(latch(QuotaSource::AcpMetaRateLimit, Billing::Subscription));

    recorder
        .record(usage(Some(351), Some(quota_blob())))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let expected = normalize(
        QuotaSource::AcpMetaRateLimit,
        Billing::Subscription,
        Some(&quota_blob()),
        Some(351),
        at(),
    );
    assert_eq!(
        store.quota_calls(),
        vec![QuotaCall {
            key: (ids::AGENT_CLAUDE, ids::BOX),
            quota: expected.to_value(),
            quota_at: at(),
        }],
        "one usage row, one latch, on the row the latch named"
    );
    assert_eq!(
        expected.observed_at,
        at(),
        "`observed_at` is the row's capture time, so `quota_at` mirrors it (§7)"
    );

    let row = on_box(&store).await;
    assert_eq!(
        row.quota.as_ref(),
        Some(&expected.to_value()),
        "the document reached the column"
    );
    assert_eq!(row.quota_at, Some(at()));
    assert_eq!(
        row.probe.as_ref(),
        Some(&probe_snapshot()),
        "and the §4.6 snapshot beside it is byte-identical (D67)"
    );
    assert_eq!(
        Quota::from_value(row.quota.as_ref().expect("a document"))
            .expect("the stored document parses")
            .windows
            .iter()
            .map(|window| window.id.as_str())
            .collect::<Vec<_>>(),
        ["five_hour", "seven_day"],
        "both windows, sorted by id"
    );

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log[0].payload.get("quota"),
        Some(&quota_blob()),
        "and the `usage` row still carries the vendor blob verbatim (D66): the keys §7 does not \
         normalize are recorded rather than discarded"
    );
}

/// Blueprint H-3: a turn's **first** `usage` report carries no `_meta`, so a latch that fired on
/// every row would overwrite last session's windows with an empty document at the top of every
/// turn.
///
/// A costed first report is the sharp half of that, and the reason the rule is not "write once
/// anything is known": row 2 below *has* a spend figure and still says nothing, because a row
/// whose source reports an allowance has not reported one yet, and publishing `windows: []`
/// alongside a spend would erase what the column holds to add a number the `usage` rows already
/// carry. Once a blob has arrived it is remembered: the last row has no `_meta` of its own and
/// refreshes the spend with the windows left standing.
#[tokio::test]
async fn a_bare_first_row_latches_nothing_and_a_later_one_keeps_the_last_blob() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    probe_agent_box(&store).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None)
        .with_quota_latch(latch(QuotaSource::AcpMetaRateLimit, Billing::Subscription));

    // Context only: no blob, no cost, nothing to publish.
    recorder
        .record(usage(None, None))
        .await
        .expect("recording must land");
    assert!(
        store.quota_calls().is_empty(),
        "a bare first report writes nothing at all, so the standing document survives it"
    );
    assert_eq!(
        on_box(&store).await.quota,
        None,
        "and the column is untouched, not emptied"
    );

    // Costed, still no `_meta`: a spend figure is not worth an empty window list.
    recorder
        .record(usage(Some(100), None))
        .await
        .expect("recording must land");
    assert!(
        store.quota_calls().is_empty(),
        "a source that reports an allowance has said nothing until its first blob, whatever the \
         row spent (H-3)"
    );

    recorder
        .record(usage(Some(250), Some(quota_blob())))
        .await
        .expect("recording must land");
    recorder
        .record(usage(Some(1), None))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let calls = store.quota_calls();
    assert_eq!(calls.len(), 2, "the two rows that had something to say");
    assert_eq!(
        calls[0].quota,
        normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&quota_blob()),
            Some(350),
            at()
        )
        .to_value(),
        "the blob's row publishes the spend the two rows before it accumulated"
    );
    assert_eq!(
        calls[1].quota,
        normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&quota_blob()),
            Some(351),
            at()
        )
        .to_value(),
        "and the `_meta`-less row after it refreshed the spend and kept the windows"
    );
    let stored = Quota::from_value(&calls[1].quota).expect("the document parses");
    assert_eq!(stored.windows.len(), 2, "the windows are still there");
    assert_eq!(stored.spend.session_micros, Some(351));
}

/// A row whose `agent.settings.quota.source` is `none` latches **spend only**: no status, no
/// windows, not exhausted — and, with nothing spent, nothing at all.
///
/// This is the seeded live-ACP row that reports no allowance (plan D65): its quota column shows a
/// spend figure once it has cost something and `—` until then, by design rather than by omission.
/// The blob on the row is ignored because the *row* says its source reports none, which is D66's
/// selection rule: declared, never sniffed.
#[tokio::test]
async fn a_row_declaring_source_none_latches_spend_only() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    probe_agent_box(&store).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None)
        .with_quota_latch(latch(QuotaSource::None, Billing::PerToken));

    recorder
        .record(usage(None, None))
        .await
        .expect("recording must land");
    assert!(
        store.quota_calls().is_empty(),
        "context-only reports and no declared source: nothing to say, so nothing written"
    );

    recorder
        .record(usage(Some(100), Some(quota_blob())))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let calls = store.quota_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].quota,
        json!({
            "source": "none",
            "billing": "per_token",
            "status": Value::Null,
            "exhausted": false,
            "windows": [],
            "spend": { "session_micros": 100, "currency": "USD" },
            "observed_at": at(),
        }),
        "spend and the two row-side facts; the blob the transport sent is not this row's source"
    );
}

/// Review M-2 / plan D78: a row whose **billing is per-token** publishes its spend on the first
/// costed report, whatever its declared `quota.source` says.
///
/// The realistic instance is a `claude` row switched to API-key billing while
/// `settings.quota.source` still reads `acp_meta_rate_limit`. No allowance blob ever arrives on
/// that transport, so the source-based H-3 guard would wait for one forever and the column would
/// read `—` while the recorder holds the exact number `docs/ANA-4.md` §7 asks it to publish. §7
/// gives a per-token agent no `windows` at all, so there is nothing for H-3 to protect here and
/// nothing to wait for: the document is `windows: []` plus the spend, which is the
/// [`QuotaSource::None`] shape.
///
/// The subscription half of the same combination is asserted beside it, because the whole point of
/// D78 is that only `billing` moved: a subscription row with the same declared source still says
/// nothing until its blob arrives.
#[tokio::test]
async fn a_per_token_row_publishes_its_spend_without_waiting_for_an_allowance() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    probe_agent_box(&store).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None)
        .with_quota_latch(latch(QuotaSource::AcpMetaRateLimit, Billing::PerToken));

    // Context only: nothing spent yet, so nothing to say — the `QuotaSource::None` rule, which is
    // what a per-token row now follows.
    recorder
        .record(usage(None, None))
        .await
        .expect("recording must land");
    assert!(
        store.quota_calls().is_empty(),
        "a per-token row with nothing spent has nothing to publish either"
    );

    // The first costed row, still with no `_meta` of any kind: it publishes.
    recorder
        .record(usage(Some(100), None))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let calls = store.quota_calls();
    assert_eq!(
        calls.len(),
        1,
        "the first costed row publishes rather than waiting for a blob that never comes (D78)"
    );
    assert_eq!(
        calls[0].quota,
        json!({
            "source": "acp_meta_rate_limit",
            "billing": "per_token",
            "status": Value::Null,
            "exhausted": false,
            "windows": [],
            "spend": { "session_micros": 100, "currency": "USD" },
            "observed_at": at(),
        }),
        "§7: a per-token agent carries `spend` and no `windows`, so the declared source costs the \
         document nothing"
    );
    assert_eq!(
        on_box(&store).await.quota,
        Some(calls[0].quota.clone()),
        "and it reached the column"
    );

    // The same declared source under a subscription still waits, which is the half D78 left alone.
    let chat = chat_spec();
    let store = open_chat(&chat).await;
    probe_agent_box(&store).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None)
        .with_quota_latch(latch(QuotaSource::AcpMetaRateLimit, Billing::Subscription));
    recorder
        .record(usage(Some(100), None))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");
    assert!(
        store.quota_calls().is_empty(),
        "a subscription row that reports an allowance has said nothing until its first blob (H-3)"
    );
}

/// Plan D68: the latch is best-effort. A store that refuses it neither fails the turn nor gets
/// asked again — the `usage` rows are what durability rests on (`R-HIS-1`), and an advisory
/// allowance figure must not cost a session.
///
/// The two refusals that mean *stop asking* are answered by switching the latch off for the
/// session: `Unreachable` is the offline backend's refusal (`REGISTRY_ON_SERVER_ONLY`), and
/// `NotFound` means no `agent_box` row exists to latch into, which no later row of this session
/// will change. Anything else is retried per row, because it may well be transient.
#[tokio::test]
async fn a_refused_latch_neither_fails_the_turn_nor_retries() {
    /// Three costed `usage` rows through a recorder whose store answers every latch with `error`.
    async fn play(error: StoreError) -> (SpyStore, usize) {
        let chat = chat_spec();
        let scrubber = scrubber();
        let store = open_chat(&chat).await;
        probe_agent_box(&store).await;
        store.refuse_quota_with(error);
        let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None)
            .with_quota_latch(latch(QuotaSource::AcpMetaRateLimit, Billing::Subscription));
        for cost in [100, 250, 1] {
            recorder
                .record(usage(Some(cost), Some(quota_blob())))
                .await
                .expect("a refused latch is not a recording failure");
        }
        let summary = recorder.finish().await.expect("nor a failed session");
        assert_eq!(
            summary.usage["cost_micros"], 351,
            "the usage rows and their sum are unaffected"
        );
        let rows = rows(&store, chat.step_id).await;
        assert_eq!(rows.len(), 3, "three usage rows, none of them an error row");
        (store, 3)
    }

    let (store, rows) = play(StoreError::Unreachable(
        "the agent registry is written on the server only".to_owned(),
    ))
    .await;
    assert_eq!(
        store.quota_calls().len(),
        1,
        "an unreachable registry is asked once and then left alone for the session"
    );

    let (store, _) = play(StoreError::NotFound {
        entity: "agent_box",
        id: "unprobed".to_owned(),
    })
    .await;
    assert_eq!(
        store.quota_calls().len(),
        1,
        "a row that does not exist will not exist on the next report either"
    );

    let (store, _) = play(StoreError::Backend("the pool is busy".to_owned())).await;
    assert_eq!(
        store.quota_calls().len(),
        rows,
        "anything else may be transient, so the next report tries again"
    );
}

/// A `usage` row masking made unreadable is summed into `run_step.usage` and **not** latched: a
/// masked document is not a document to publish.
///
/// The row itself is still persisted verbatim, so the blob it carried is not lost — a later,
/// readable report of the same session republishes it. Latching a document assembled out of a
/// masked payload is the one thing that would put `[REDACTED]` into a column three readers render.
#[tokio::test]
async fn a_masked_usage_row_is_summed_but_not_latched() {
    let chat = chat_spec();
    // A masked `output_tokens` is a string where an `Option<i64>` belongs, so the payload stops
    // reading back as a `UsageEvent` and the row takes the recorder's unreadable path.
    let scrubber = MaskKey("output_tokens");
    let store = open_chat(&chat).await;
    probe_agent_box(&store).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None)
        .with_quota_latch(latch(QuotaSource::AcpMetaRateLimit, Billing::Subscription));

    recorder
        .record(env(DriverEvent::Usage(UsageEvent {
            output_tokens: Some(5),
            cost_micros: Some(100),
            quota: Some(quota_blob()),
            ..UsageEvent::default()
        })))
        .await
        .expect("a masked usage key is not a recording failure");
    let summary = recorder.finish().await.expect("close");

    assert_eq!(
        summary.usage["cost_micros"], 100,
        "the keys masking left readable are still summed (plan D36)"
    );
    assert!(
        store.quota_calls().is_empty(),
        "and the masked row publishes no allowance document"
    );
    assert_eq!(on_box(&store).await.quota, None);
    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log[0].payload.get("quota"),
        Some(&quota_blob()),
        "the row keeps the blob, so a later readable report can still publish it"
    );
}

// ---------------------------------------------------------------------------------------------
// The per-run cap (`docs/ANA-4.md` §7 `:1143-1150`, §11 criterion 8, plan D69-D70)
// ---------------------------------------------------------------------------------------------

/// The cap the cases below record under: `micros` USD and a zero grace, which is what
/// `RunCap.grace` riding on the configuration is for — a test that waited out a real cancel window
/// would pay for it once per case, and a `ScriptedSession` owns no child process to wait for.
fn run_cap(micros: i64) -> RunCap {
    RunCap {
        micros,
        grace: Duration::from_millis(0),
    }
}

/// The closing sequence of §11 criterion 8, with the one row that is allowed to sit inside it: a
/// `tool_result` the transport's own cancel synthesized for a call that was still open.
///
/// The conformance case pins adjacency with no open call, which is the strongest form of "within
/// one event". This is the other interleaving, and it is the *live* one — a cap breach mid-tool-call
/// is exactly when a cancel has a call to terminate (`docs/ANA-4.md` §4.3 makes synthesizing that
/// result a MUST, so the row cannot be suppressed to make the two closing rows adjacent). What
/// criterion 8 actually asserts survives it: the **last two** rows are `error` then `done`.
///
/// It also pins that the verdict is reported **once**. `enforce_breach` records whatever the cancel
/// produced through the same `record`, and a `check_cap` that answered a second time would send a
/// second cancel into a session that is already closing.
#[tokio::test]
async fn a_breach_is_reported_once_and_the_closing_rows_are_error_then_done() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, None).with_run_cap(run_cap(300));
    let mut session = ScriptedSession::cancelling(
        vec![
            tool_call("call-1"),
            usage(Some(100), None),
            usage(Some(250), None),
            // Never pulled: the cancel drops the rest of the turn, and a build that detected the
            // breach without cancelling would record this row and prove itself wrong.
            chunk("after the cap", "m1"),
        ],
        vec![
            tool_result("call-1", json!({ "text": "cancelled" })),
            env(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::Cancelled,
            })),
        ],
    );

    let stop = pump(&mut session, &mut recorder)
        .await
        .expect("the capped turn still reaches an end");
    assert_eq!(
        stop.stop_reason,
        StopReason::Cancelled,
        "`pump` reports the cancelled end its own breach arm produced"
    );

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            EventKind::ToolCall,
            EventKind::Usage,
            EventKind::Usage,
            EventKind::ToolResult,
            EventKind::Error,
            EventKind::Done,
        ],
        "the synthesized result of the open call lands between the breaching row and the error, \
         and the last two rows are still the closing pair"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..6).collect::<Vec<_>>(),
        "seq stays gapless across the closing sequence"
    );
    let error = &log[4];
    assert_eq!(error.payload["code"], json!("cap_exceeded"));
    assert_eq!(
        error.role,
        EventRole::Htui,
        "`htui` authored the row, as it authors every row it writes itself"
    );
    assert!(
        error.payload["message"]
            .as_str()
            .is_some_and(
                |message| message.contains("per_token_cap_run") && message.contains("estimated")
            ),
        "the message names the setting and says the figure is an estimate: {:?}",
        error.payload["message"]
    );
    assert_eq!(log[5].payload["stop_reason"], json!("cancelled"));
    assert_eq!(
        log.iter().filter(|row| row.kind == EventKind::Done).count(),
        1,
        "exactly one `done` per turn (§4.1): the transport's own was withheld"
    );

    let again = recorder
        .record(usage(Some(1_000), None))
        .await
        .expect("a later row still records");
    assert_eq!(
        again, None,
        "the verdict is spent: a second breach would cancel a session that is already closed"
    );
    let summary = recorder.finish().await.expect("close");
    assert_eq!(
        summary.cap_breach,
        Some(CapBreach {
            cap_micros: 300,
            spent_micros: 350,
            at: at(),
        }),
        "and the summary carries the one verdict, not the last row's arithmetic"
    );
}

/// Blueprint H-4: the agent's own `done` can arrive inside the grace window saying `end_turn` —
/// the ACP transport records whatever `StopReason` the adapter sent, and the fake queues its own.
/// Recording it as it stood would make the step's last row say the turn finished normally, which is
/// the one thing criterion 8 forbids.
///
/// `enforce_breach` is called directly here, which is the seam the worker's `run_turn` uses: the
/// case is about the shared sequence and not about which loop reached it.
#[tokio::test]
async fn a_transport_done_that_says_end_turn_is_still_recorded_cancelled() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, None).with_run_cap(run_cap(100));
    let mut session = ScriptedSession::cancelling(
        Vec::new(),
        vec![env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        }))],
    );

    let breach = recorder
        .record(usage(Some(100), None))
        .await
        .expect("recording must land")
        .expect("a spend equal to the cap has reached it");
    let stop = htui_agent::record::enforce_breach(&mut session, &mut recorder, breach)
        .await
        .expect("the closing rows must land");
    assert_eq!(
        stop.stop_reason,
        StopReason::Cancelled,
        "the end handed back to the turn loop is the cancelled one"
    );
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Usage, EventKind::Error, EventKind::Done]
    );
    assert_eq!(
        log[2].payload["stop_reason"],
        json!("cancelled"),
        "the transport said `end_turn`; the log says what happened"
    );
    assert_eq!(
        log[2].at,
        at(),
        "the withheld `done`'s own capture time is kept, so `keep_raw_events` still explains the \
         row"
    );
}

/// Blueprint H-5: a context-only `usage` row reports no USD cost, and a cap of `0` compared
/// against `cost_micros.unwrap_or(0)` would cancel a session that has spent nothing.
///
/// `0` is a real cap — it cancels on the first **costed** row — and this is the other half of that
/// sentence: until one arrives there is nothing to compare, which is `agy`'s whole shape (§7's own
/// table) and the opening rows of every `claude` turn.
#[tokio::test]
async fn no_cap_and_context_only_usage_never_breach() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, None).with_run_cap(run_cap(0));

    for _ in 0..3 {
        assert_eq!(
            recorder
                .record(usage(None, None))
                .await
                .expect("recording must land"),
            None,
            "a row that reports no USD cost cannot reach a cap denominated in USD micros"
        );
    }
    let summary = recorder.finish().await.expect("close");
    assert_eq!(summary.cap_breach, None);
    assert!(
        !rows(&store, chat.step_id)
            .await
            .iter()
            .any(|row| row.kind == EventKind::Error),
        "and no `cap_exceeded` row was written"
    );
}

/// A cap of `0` is a cap and not an absent one (plan D70): it cancels on the first row that reports
/// any USD cost at all, however small.
#[tokio::test]
async fn a_cap_of_zero_cancels_on_the_first_costed_row() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, None).with_run_cap(run_cap(0));

    assert_eq!(
        recorder
            .record(usage(None, None))
            .await
            .expect("recording must land"),
        None,
        "the context-only row before it has nothing to compare"
    );
    assert_eq!(
        recorder
            .record(usage(Some(1), None))
            .await
            .expect("recording must land"),
        Some(CapBreach {
            cap_micros: 0,
            spent_micros: 1,
            at: at(),
        }),
        "one micro against a cap of zero is `spent >= cap`"
    );
    recorder.finish().await.expect("close");
}

/// Blueprint H-2: the flush that writes the two closing rows can be refused, and the rows are then
/// **owed**, not lost.
///
/// The recorder's existing rule does the work — a refused flush commits nothing, keeps the numbered
/// batch in `unflushed` and re-offers it at the same `seq` — and the reason it covers criterion 8
/// is that the closing pair is buffered *before* that flush, so the breaching row and the two rows
/// about it are owed together. `finish` then writes all three, in order, gapless.
#[tokio::test]
async fn a_refused_flush_leaves_the_two_closing_rows_owed_not_lost() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, None).with_run_cap(run_cap(300));
    let mut session = ScriptedSession::cancelling(
        Vec::new(),
        vec![env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::Cancelled,
        }))],
    );

    let breach = recorder
        .record(usage(Some(350), None))
        .await
        .expect("recording must land")
        .expect("the cap is reached");
    store.refuse_next_appends(1);
    let refused = htui_agent::record::enforce_breach(&mut session, &mut recorder, breach).await;
    assert!(
        matches!(refused, Err(DriverError::Store(_))),
        "a store that refuses the closing flush is reported, got {refused:?}"
    );
    let summary = recorder.finish().await.expect("the retry lands");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Usage, EventKind::Error, EventKind::Done],
        "the closing pair is written by the next flush, in the order it was authored"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "at the very seq the refused flush had given them"
    );
    assert_eq!(
        summary.usage["cost_micros"], 350,
        "and the breaching row was summed exactly once, not once per attempt"
    );
}

/// Review L-1: the tab is told about the cap **before** the flush that could fail, so a store that
/// refuses the closing write costs a retry and not the two frames.
///
/// Plan D73 rules that the `error` row *is* how a breach becomes visible — the chat tab renders it
/// and nothing else on screen says a cap fired. Sending it after the flush meant the one path where
/// the rows are owed (blueprint H-2) was also the one path where the user was told nothing: the tab
/// got the transport error and `Ended{EndTurn}` while the log said `done{cancelled}`. The channel is
/// a `try_send` the same task drains, so moving the frames earlier cannot stall or fail.
#[tokio::test]
async fn the_closing_frames_reach_the_tab_even_when_their_flush_is_refused() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, Some(tx)).with_run_cap(run_cap(300));
    let mut session = ScriptedSession::cancelling(
        Vec::new(),
        vec![env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::Cancelled,
        }))],
    );

    let breach = recorder
        .record(usage(Some(350), None))
        .await
        .expect("recording must land")
        .expect("the cap is reached");
    store.refuse_next_appends(1);
    let refused = htui_agent::record::enforce_breach(&mut session, &mut recorder, breach).await;
    assert!(
        matches!(refused, Err(DriverError::Store(_))),
        "the caller still hears that the closing write failed, got {refused:?}"
    );

    let frames: Vec<DriverEvent> = core::iter::from_fn(|| rx.try_recv().ok())
        .map(|frame| frame.event)
        .collect();
    match frames.as_slice() {
        [
            DriverEvent::Usage(_),
            DriverEvent::Error(error),
            DriverEvent::Done(done),
        ] => {
            assert_eq!(error.code, "cap_exceeded");
            assert_eq!(
                done.stop_reason,
                StopReason::Cancelled,
                "the frame says what the log's last row says"
            );
        }
        other => panic!("the breaching row and both closing frames reach the tab: {other:?}"),
    }

    // And the rows are still owed rather than lost, which is what makes the frames honest.
    recorder.finish().await.expect("the retry lands");
    assert_eq!(
        rows(&store, chat.step_id)
            .await
            .iter()
            .map(|row| row.kind)
            .collect::<Vec<_>>(),
        vec![EventKind::Usage, EventKind::Error, EventKind::Done]
    );
}

/// Review M-1: a breach detected on a row whose flush the store then refuses is **still reported**.
///
/// The unreadable path is the one that could lose it. `record_unreadable` sums the masked payload
/// and compares the cap *before* its trailing flush, so when that flush fails the session is in the
/// worst possible state: the spend is counted, `cap_breached` is `Some` — which makes every later
/// `check_cap` answer `None` — and the caller has an error where the verdict should be. The worker
/// logs a recording error and carries on (`agent_worker::record`), so a build that returned the
/// error instead of the verdict would disarm the cap for the rest of the session on one masked row
/// plus one store fault: a run that was supposed to be bounded, running unbounded, with nothing on
/// screen saying so.
///
/// Nothing is traded for that: the refused batch stays in `unflushed` and the closing sequence the
/// verdict triggers writes it ahead of the two closing rows (blueprint H-2), which is what the row
/// order below asserts.
#[tokio::test]
async fn a_breach_survives_the_refused_flush_of_the_row_that_caused_it() {
    let chat = chat_spec();
    // A masked `output_tokens` sends the row down the unreadable path; `cost_micros` stays
    // readable, so the row is summed and the cap is compared exactly as on the readable path.
    let scrubber = MaskKey("output_tokens");
    let store = open_chat(&chat).await;
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, None).with_run_cap(run_cap(300));
    let mut session = ScriptedSession::cancelling(
        Vec::new(),
        vec![env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::Cancelled,
        }))],
    );

    store.refuse_next_appends(1);
    let breach = recorder
        .record(env(DriverEvent::Usage(UsageEvent {
            output_tokens: Some(5),
            cost_micros: Some(350),
            ..UsageEvent::default()
        })))
        .await
        .expect("a refused flush is not what the caller hears about: the breach is")
        .expect("the row was summed and reached the cap, so the cap fired");
    assert_eq!(
        breach,
        CapBreach {
            cap_micros: 300,
            spent_micros: 350,
            at: at(),
        },
        "the verdict names the cap, the spend that reached it and the row that did"
    );

    // And the verdict is answerable: the sequence the worker would run next lands the row the
    // refused flush was carrying, in front of the pair it writes itself.
    let stop = htui_agent::record::enforce_breach(&mut session, &mut recorder, breach)
        .await
        .expect("the closing sequence writes the owed row and the two closing ones");
    assert_eq!(stop.stop_reason, StopReason::Cancelled);
    let summary = recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Usage, EventKind::Error, EventKind::Done],
        "the owed row is re-offered ahead of the closing pair (blueprint H-2)"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "at the seq the refused flush had given it"
    );
    assert_eq!(log[1].payload["code"], json!("cap_exceeded"));
    assert_eq!(log[2].payload["stop_reason"], json!("cancelled"));
    assert_eq!(
        summary.usage["cost_micros"], 350,
        "and the breaching row was summed once, not once per attempt"
    );
    assert_eq!(
        summary.cap_breach,
        Some(breach),
        "the session's one verdict is the one the refused flush could not swallow"
    );
}

/// A secret from `SessionSpec.env` that reaches a tool result is `[REDACTED]` in the persisted row
/// (`R-SEC-3`, ANA-4 §9: scrub before either write path).
#[tokio::test]
async fn env_values_are_masked_in_persisted_rows() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, true, None);

    recorder
        .record(raw_env(
            DriverEvent::ToolResult(ToolResultEvent {
                tool_call_id: "call-1".to_owned(),
                status: ToolResultStatus::Completed,
                output: Some(json!({ "text": format!("export TOKEN={SECRET} && run") })),
                locations: Vec::new(),
                terminal_reason: None,
            }),
            json!({ "wire": format!("...{SECRET}...") }),
        ))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 1);
    let rendered = serde_json::to_string(&log[0]).expect("the row serialises");
    assert!(
        !rendered.contains(SECRET),
        "no persisted row may carry an env value, payload or raw"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "the occurrence is masked, not dropped: {rendered}"
    );
}

/// Fail-closed (`R-SEC-3`, plan D6): a credential the scrubber could not mask blocks that row's
/// write entirely, leaves one `error { code: "scrub_residue" }` in its place, and makes `finish`
/// report `RecordError::Unmasked`. The recorder does **not** touch the step's status: that is the
/// step owner's write (`finish_chat_run` in milestone 3, MOD-4 for graph steps).
#[tokio::test]
async fn scrub_residue_refuses_the_write() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record(chunk("before", "m1"))
        .await
        .expect("recording must land");
    recorder
        .record(tool_result(
            "call-1",
            json!({ "output": "leaked sk-ant-api03-abcdefghijklmnopqrstuvwx here" }),
        ))
        .await
        .expect("a refused row is not a recording failure");
    recorder
        .record(chunk("after", "m2"))
        .await
        .expect("recording must land");
    let outcome = recorder.finish().await;

    assert!(
        matches!(outcome, Err(RecordError::Unmasked(_))),
        "finish reports the residue, got {outcome:?}"
    );

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::ToolResult)
            .count(),
        0,
        "the offending row is dropped entirely"
    );
    let errors: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "exactly one scrub_residue row in its place"
    );
    assert_eq!(errors[0].role, EventRole::Htui);
    assert_eq!(
        errors[0].payload.get("code").and_then(Value::as_str),
        Some("scrub_residue")
    );
    let message = errors[0]
        .payload
        .get("message")
        .and_then(Value::as_str)
        .expect("the error row carries a message");
    assert!(
        message.starts_with("anthropic_api_key at "),
        "the message is `<rule> at <path>`, got {message}"
    );
    assert!(
        !serde_json::to_string(&log)
            .expect("the log serialises")
            .contains("sk-ant-"),
        "the offending text never reaches the store, not even through the error row"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("short")).collect::<Vec<_>>(),
        "dropping a row does not open a gap in seq"
    );
}

/// Edit proposals dedupe per `(tool_call_id, path)`: the buffered row is updated before the flush,
/// never written twice (plan D6, `docs/ANA-4.md` §4.3).
///
/// Run under **both** `retain_raw` settings, because the attribution of the update is only
/// observable when `raw` is kept: with `retain_raw = false` a proposal update that landed on the
/// wrong row would look identical to one that landed on the right one. The update owns the row's
/// `diff`, its `accepted`, its capture time and its wire message; the unrelated proposal buffered
/// after it owns none of them.
#[tokio::test]
async fn edit_proposals_dedupe_per_call_and_path() {
    let later = at() + chrono::TimeDelta::seconds(5);
    let proposal =
        |path: &str, diff: &str, accepted: Option<bool>, wire: &str, at| DriverEnvelope {
            event: DriverEvent::EditProposal(EditProposalEvent {
                tool_call_id: Some("call-1".to_owned()),
                path: path.to_owned(),
                diff: diff.to_owned(),
                accepted,
            }),
            raw: Some(json!({ "wire": wire })),
            at,
        };

    for retain_raw in [false, true] {
        let chat = chat_spec();
        let scrubber = scrubber();
        let store = open_chat(&chat).await;
        let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, retain_raw, None);

        recorder
            .record(proposal("src/a.rs", "@@ first", None, "a-first", at()))
            .await
            .expect("recording must land");
        recorder
            .record(proposal("src/b.rs", "@@ other file", None, "b-only", at()))
            .await
            .expect("recording must land");
        recorder
            .record(proposal(
                "src/a.rs",
                "@@ second",
                Some(true),
                "a-second",
                later,
            ))
            .await
            .expect("recording must land");
        recorder.finish().await.expect("close");

        let log = rows(&store, chat.step_id).await;
        assert_eq!(
            log.len(),
            2,
            "one row per (tool_call_id, path), not one per proposal"
        );
        assert_eq!(
            log[0].payload.get("path").and_then(Value::as_str),
            Some("src/a.rs")
        );
        assert_eq!(
            log[0].payload.get("diff").and_then(Value::as_str),
            Some("@@ second"),
            "the buffered row is updated in place"
        );
        assert_eq!(
            log[0].payload.get("accepted").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            log[1].payload.get("path").and_then(Value::as_str),
            Some("src/b.rs")
        );
        assert_eq!(
            log[0].at, later,
            "the deduped row is the update, so it carries the update's capture time"
        );
        assert_eq!(
            log[1].at,
            at(),
            "a row the update did not touch keeps its own capture time"
        );

        if retain_raw {
            assert_eq!(
                log[0].raw,
                Some(json!([{ "wire": "a-first" }, { "wire": "a-second" }])),
                "both wire messages behind the deduped row belong to that row"
            );
            assert_eq!(
                log[1].raw,
                Some(json!({ "wire": "b-only" })),
                "the update's raw may not be charged to whichever row was buffered last"
            );
        } else {
            assert!(
                log.iter().all(|row| row.raw.is_none()),
                "keep_raw_events = false means every row has raw IS NULL"
            );
        }
    }
}

/// T46 / plan D77: the dedup rule of `docs/ANA-4.md` §11 criterion 6 survives a **flush**.
///
/// The row that broke it live: `agy` re-announces a `tool_call` verbatim where the spec's example
/// sends `tool_call_update`, and the permission park/answer flushes in between, so the second
/// announcement found an empty index and opened a second row. The fix reserves the `seq` at the
/// first announcement and holds the row until the call closes, so what a flush in between costs is
/// nothing at all.
///
/// Four things are asserted, and each is a separate half of D77: **one** row for the key, carrying
/// the **latest** content, at the **first** announcement's `seq` — that is what makes replay, which
/// reads by `seq`, keep the true order of a row written late — and `seq` gapless over the whole log.
#[tokio::test]
async fn an_edit_proposal_reserves_its_seq_and_survives_a_flush() {
    let later = at() + chrono::TimeDelta::seconds(5);
    let proposal = |diff: &str, accepted: Option<bool>, at| DriverEnvelope {
        event: DriverEvent::EditProposal(EditProposalEvent {
            tool_call_id: Some("call-1".to_owned()),
            path: "src/a.rs".to_owned(),
            diff: diff.to_owned(),
            accepted,
        }),
        raw: None,
        at,
    };
    let request_id = htui_agent::driver::PermissionRequestId::new("req-1");

    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    // A live chat tab, because the other half of D77 is that **nothing the user sees moves**: the
    // render frame goes out at the announcement and only the database write waits.
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, Some(tx));

    recorder
        .record(tool_call("call-1"))
        .await
        .expect("recording must land");
    recorder
        .record(proposal("@@ first", None, at()))
        .await
        .expect("recording must land");
    // The real-world flush: the parked permission request is answered, and answering it writes.
    recorder
        .record_permission_answer(&request_id, Some("allow"), AnsweredBy::User, false, at())
        .await
        .expect("the answer must land");
    assert_eq!(
        rows(&store, chat.step_id)
            .await
            .iter()
            .filter(|row| row.kind == EventKind::EditProposal)
            .count(),
        0,
        "the proposal's `seq` is reserved, but its row is held rather than written (D77)"
    );
    let frames: Vec<DriverEvent> = core::iter::from_fn(|| rx.try_recv().ok())
        .map(|envelope| envelope.event)
        .collect();
    assert!(
        frames.iter().any(
            |frame| matches!(frame, DriverEvent::EditProposal(edit) if edit.diff == "@@ first")
        ),
        "the diff was on screen at the announcement, with the row still unwritten: {frames:?}"
    );

    recorder
        .record(proposal("@@ second", Some(true), later))
        .await
        .expect("recording must land");
    recorder
        .record(env(DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id: "call-1".to_owned(),
            status: ToolResultStatus::Completed,
            output: None,
            locations: Vec::new(),
            terminal_reason: None,
        })))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            EventKind::ToolCall,
            EventKind::EditProposal,
            EventKind::PermissionAnswer,
            EventKind::ToolResult,
        ],
        "one row per (tool_call_id, path) for the whole step, and it sits where it was announced"
    );
    let edit = &log[1];
    assert_eq!(
        edit.seq, 1,
        "the `seq` is the first announcement's, taken before the answer's row was numbered: a row \
         written late still replays in the order it happened"
    );
    assert_eq!(
        edit.payload.get("diff").and_then(Value::as_str),
        Some("@@ second"),
        "the held row carries the latest content"
    );
    assert_eq!(
        edit.payload.get("accepted").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        edit.at, later,
        "the held row *is* the update, so it carries the update's capture time"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("short")).collect::<Vec<_>>(),
        "and `seq` is still gapless 0..n, which is what a reservation buys over a deferral"
    );
}

/// T46's regression witness: the recorded `agy` transcript, replayed through the **recorder**.
///
/// `tests/acp_map.rs` pins what the mapper makes of this fixture — two `tool_call`s and three
/// `edit_proposal`s for one file write, because the adapter re-announces the call verbatim. This
/// case pins what the *recorder* does with that, which is where criterion 6 lives: exactly **one**
/// `edit_proposal` row for the write. The live run left three, at `seq` 12, 15 and 16.
///
/// The permission answer is interleaved after the second announcement because that is where it fell
/// live: `agy` requests permission for the write, and answering a parked request flushes. Without it
/// the same replay left two rows rather than three — the count is a function of the interleaving,
/// which is exactly why a fake that never flushes mid-call could not see the defect.
#[tokio::test]
async fn the_recorded_agy_write_leaves_one_edit_proposal_row() {
    let path = format!(
        "{}/tests/fixtures/agy_acp_turn.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    let mut mapper = htui_agent::acp::map::Mapper::new();
    let events: Vec<DriverEvent> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("a fixture line is JSON"))
        .filter(|line| line["method"] == "session/update")
        .flat_map(|line| mapper.map(&line["params"]["update"]))
        .collect();
    let announcements = events
        .iter()
        .filter(|event| matches!(event, DriverEvent::EditProposal(_)))
        .count();
    assert_eq!(
        announcements, 3,
        "the fixture is the one the defect was measured on: three announcements of one write"
    );

    let request_id = htui_agent::driver::PermissionRequestId::new("perm-1");
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt("write a file", json!({}), at())
        .await
        .expect("the prompt row must land");
    let mut seen = 0;
    for event in events {
        let announcement = matches!(event, DriverEvent::EditProposal(_));
        recorder
            .record(env(event))
            .await
            .expect("recording must land");
        if announcement {
            seen += 1;
            if seen == 2 {
                recorder
                    .record_permission_answer(
                        &request_id,
                        Some("allow"),
                        AnsweredBy::User,
                        false,
                        at(),
                    )
                    .await
                    .expect("the answer must land");
            }
        }
    }
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    let edits: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::EditProposal)
        .collect();
    assert_eq!(
        edits.len(),
        1,
        "one `edit_proposal` for the write, where the live run left three (seq 12, 15, 16)"
    );
    assert_eq!(
        edits[0].payload.get("path").and_then(Value::as_str),
        Some("/scratch/htui-agy-probe.txt")
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("short")).collect::<Vec<_>>(),
        "`seq` is gapless across the whole replayed turn"
    );
}

/// D77's two backstops: a held row whose tool call never closes is written by the **cancel** and by
/// **`finish`**, because a held row that was never written would be a `seq` gap — worse than the
/// duplicate the rule exists to prevent.
///
/// The cancel is the cap's (`enforce_breach`), which is the one shared cancel-and-close sequence and
/// therefore the only cancel path there is. `finish` is the backstop under everything, including a
/// turn that simply ends with a call still open.
#[tokio::test]
async fn a_held_edit_proposal_is_written_by_a_cancel_and_by_finish() {
    let proposal = env(DriverEvent::EditProposal(EditProposalEvent {
        tool_call_id: Some("call-1".to_owned()),
        path: "src/a.rs".to_owned(),
        diff: "@@ held".to_owned(),
        accepted: None,
    }));

    // `finish`: the call never closes and the turn never ends.
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);
    recorder
        .record(tool_call("call-1"))
        .await
        .expect("recording must land");
    recorder
        .record(proposal.clone())
        .await
        .expect("recording must land");
    let summary = recorder.finish().await.expect("close");
    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::ToolCall, EventKind::EditProposal],
        "`finish` writes what is still held"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1],
        "at the reserved `seq`, with no gap"
    );
    assert_eq!(
        summary.seq, 2,
        "and the summary's `seq` counts the row it authored"
    );

    // The cancel: the cap fires with the proposal still held, and the closing pair must not be
    // written over a hole.
    let chat = chat_spec();
    let store = open_chat(&chat).await;
    let mut recorder =
        Recorder::new(&store, &scrubber, chat.step_id, false, None).with_run_cap(run_cap(100));
    let mut session = ScriptedSession::cancelling(Vec::new(), Vec::new());
    recorder
        .record(proposal)
        .await
        .expect("recording must land");
    let breach = recorder
        .record(usage(Some(100), None))
        .await
        .expect("recording must land")
        .expect("a spend equal to the cap has reached it");
    htui_agent::record::enforce_breach(&mut session, &mut recorder, breach)
        .await
        .expect("the closing rows must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            EventKind::EditProposal,
            EventKind::Usage,
            EventKind::Error,
            EventKind::Done,
        ],
        "the cancel writes the held row before the closing pair, and criterion 8's last row still \
         says `cancelled`"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "no gap where the held row was"
    );
}

/// A secret split across two chunks only exists once the run is assembled, so the coalesced row
/// is scrubbed again at the flush; masking is idempotent, so the pieces already scrubbed at
/// capture cost a pass and change nothing.
#[tokio::test]
async fn a_secret_split_across_chunks_is_masked_at_the_flush() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let (head, tail) = SECRET.split_at(6);
    recorder
        .record(chunk(&format!("token {head}"), "m1"))
        .await
        .expect("recording must land");
    recorder
        .record(chunk(&format!("{tail} used"), "m1"))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 1, "one coalesced row");
    assert_eq!(
        text_of(&log[0]),
        "token [REDACTED] used",
        "the halves are masked once they are one string"
    );
}

/// The same at the fail-closed end: a credential neither chunk carries on its own still blocks the
/// coalesced row's write.
#[tokio::test]
async fn a_credential_split_across_chunks_is_refused_at_the_flush() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record(chunk("the key is sk", "m1"))
        .await
        .expect("neither half trips a rule on its own");
    recorder
        .record(chunk("-ant-api03-abcdefghijklmnop", "m1"))
        .await
        .expect("neither half trips a rule on its own");
    let outcome = recorder.finish().await;

    assert!(
        matches!(outcome, Err(RecordError::Unmasked(_))),
        "the assembled run is refused, got {outcome:?}"
    );
    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Error],
        "the coalesced row is dropped and the scrub_residue row takes its seq"
    );
    assert_eq!(log[0].seq, 0, "no gap opens where the dropped row was");
    assert!(
        !serde_json::to_string(&log)
            .expect("the log serialises")
            .contains("sk-ant-"),
        "the offending text never reaches the store"
    );
}

/// A paused chat tab must never stall the agent: the UI channel is bounded and `try_send`, every
/// row still persists, and the drops are counted (`docs/ANA-4.md` §4.1 "Persistence and the UI").
#[tokio::test]
async fn bounded_ui_channel_drops_are_counted() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, Some(tx));

    for i in 0..8 {
        recorder
            .record(env(DriverEvent::Usage(UsageEvent {
                input_tokens: Some(i),
                ..UsageEvent::default()
            })))
            .await
            .expect("a full UI channel never fails a recording");
    }
    assert_eq!(
        recorder.dropped(),
        7,
        "capacity 1, nothing drained: one frame delivered, the rest counted"
    );
    let summary = recorder.finish().await.expect("close");

    assert_eq!(summary.dropped, 7, "the summary reports the same count");
    assert_eq!(
        rows(&store, chat.step_id).await.len(),
        8,
        "a dropped render frame never costs a row"
    );
}

/// A `permission_answer` a **transport** reported is written under the role its author implies,
/// not under the `agent` every other driver row gets (plan D93).
///
/// This is the whole cost of the twelfth variant, and the reason it is not a second mechanism: the
/// role comes from [`AnsweredBy::role`], the same function `record_permission_answer` calls for the
/// answer a human gave. A transport that had to be trusted to *state* the role could state the
/// wrong one, and a row claiming the user chose when nobody was asked is the failure that matters.
#[tokio::test]
async fn a_policy_answer_a_transport_reported_is_htuis_row() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record(env(DriverEvent::PermissionAnswer(PermissionAnswerEvent {
            request_id: htui_agent::driver::PermissionRequestId::new("call-9"),
            tool_call_id: Some("call-9".to_owned()),
            option_id: None,
            by: AnsweredBy::Policy,
            cancelled: false,
            denied: true,
        })))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::PermissionAnswer]
    );
    assert_eq!(
        log[0].role,
        EventRole::Htui,
        "a policy answer is htui's row whoever reported it"
    );
    assert_eq!(
        log[0].tool_call_id.as_deref(),
        Some("call-9"),
        "the column `idx_session_event_tool` joins on carries the call the answer settled"
    );
    assert_eq!(
        log[0].payload.get("by").and_then(Value::as_str),
        Some("policy")
    );
    assert_eq!(
        log[0].payload.get("denied").and_then(Value::as_bool),
        Some(true),
        "the added key that tells a denial from an answer somebody gave"
    );
    assert_eq!(log[0].payload.get("option_id"), Some(&Value::Null));
}

/// `record_permission_answer` writes the `permission_answer` row of ANA-9 §4.3, with the role the
/// answer's author implies.
#[tokio::test]
async fn permission_answers_carry_their_author() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let request_id = htui_agent::driver::PermissionRequestId::new("req-1");
    recorder
        .record(env(DriverEvent::PermissionRequest(
            PermissionRequestEvent {
                request_id: request_id.clone(),
                tool_call_id: Some("call-1".to_owned()),
                options: vec![PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                }],
            },
        )))
        .await
        .expect("recording must land");
    recorder
        .record_permission_answer(&request_id, Some("allow"), AnsweredBy::User, false, at())
        .await
        .expect("the answer row must land");
    recorder
        .record_permission_answer(&request_id, None, AnsweredBy::Policy, true, at())
        .await
        .expect("the cancellation row must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            EventKind::PermissionRequest,
            EventKind::PermissionAnswer,
            EventKind::PermissionAnswer
        ]
    );
    assert_eq!(log[1].role, EventRole::User);
    assert_eq!(
        log[1].payload.get("option_id").and_then(Value::as_str),
        Some("allow")
    );
    assert_eq!(
        log[1].payload.get("by").and_then(Value::as_str),
        Some("user")
    );
    assert_eq!(
        log[2].role,
        EventRole::Htui,
        "a policy answer is htui's row"
    );
    assert_eq!(
        log[2].payload.get("option_id"),
        Some(&Value::Null),
        "a cancelled request has no option"
    );
    assert_eq!(
        log[2].payload.get("cancelled").and_then(Value::as_bool),
        Some(true)
    );
}

/// `pump` drives one turn to its `done` and records everything on the way (the seam MOD-4 and
/// milestone 3 call; T6's `FakeDriver` is what exercises it over a real script).
#[tokio::test]
async fn pump_drives_one_turn_to_done() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);
    let mut session = ScriptedSession::new(vec![
        chunk("thinking ", "m1"),
        tool_call("call-1"),
        chunk("done", "m1"),
        env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
        chunk("never reached", "m2"),
    ]);

    let done = pump(&mut session, &mut recorder)
        .await
        .expect("the turn must reach done");
    assert_eq!(done.stop_reason, StopReason::EndTurn);
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            EventKind::AssistantText,
            EventKind::ToolCall,
            EventKind::AssistantText,
            EventKind::Done
        ],
        "pump stops at done and leaves the rest of the queue for the next turn"
    );
    assert_eq!(session.events.len(), 1, "one envelope is left unread");
}

// ---------------------------------------------------------------------------------------------
// A store that fails, a turn that is refused, and a mask that goes too far
// ---------------------------------------------------------------------------------------------

/// A flush the store refuses commits **nothing**: the rows it numbered are still owed, `seq` has
/// not advanced, and the next flush writes them at the numbers they were given. Gapless `seq` with
/// exactly one writer is the guarantee that makes `PRIMARY KEY (run_step_id, seq)` a backstop
/// rather than the allocator, and it has to survive a store that fails once.
#[tokio::test]
async fn a_refused_append_costs_a_retry_and_never_a_seq() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    store.refuse_next_appends(1);
    let outcome = recorder
        .record_prompt("summarise the backlog", json!([]), at())
        .await;
    assert!(
        matches!(outcome, Err(RecordError::Store(_))),
        "the store refused the prompt row, got {outcome:?}"
    );
    assert!(
        store
            .step_events(chat.step_id)
            .await
            .expect("reading the log must not fail")
            .is_none(),
        "a refused append writes no row at all"
    );

    // The caller carries on rather than retrying: the prompt row is owed, not lost.
    recorder
        .record(chunk("an answer", "m1"))
        .await
        .expect("recording must land");
    let summary = recorder
        .finish()
        .await
        .expect("the recorder closes once the store takes the batch");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Prompt, EventKind::AssistantText],
        "the row the failed flush numbered is written by the next one, in its own order"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1],
        "seq 0 is not spent by a flush that wrote nothing"
    );
    assert_eq!(
        summary.seq,
        i32::try_from(summary.rows).expect("the log is short"),
        "every seq the recorder authored reached the store: rows == seq"
    );
    assert_eq!(
        summary.rows,
        log.len(),
        "the summary counts the stored rows"
    );
}

/// A follow-up the scrubber refuses still opens its turn: the `scrub_residue` row that stands in
/// for it, and every agent row that answers it, carry the new `turn`. Numbering them under the
/// turn that just ended would make `turn` a function of the scrubber (`docs/ANA-4.md` §4.1).
#[tokio::test]
async fn a_refused_follow_up_still_opens_the_next_turn() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record_prompt("summarise the backlog", json!([]), at())
        .await
        .expect("the prompt row must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    recorder
        .record_follow_up("use sk-ant-api03-abcdefghijklmnopqrstuvwx", at())
        .await
        .expect("a refused follow-up is not a recording failure");
    recorder
        .record(chunk("an answer", "m2"))
        .await
        .expect("recording must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    let outcome = recorder.finish().await;

    assert!(
        matches!(outcome, Err(RecordError::Unmasked(_))),
        "finish still reports the residue, got {outcome:?}"
    );
    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter()
            .map(|row| (row.kind, row.turn))
            .collect::<Vec<_>>(),
        vec![
            (EventKind::Prompt, 0),
            (EventKind::Done, 0),
            (EventKind::Error, 1),
            (EventKind::AssistantText, 1),
            (EventKind::Done, 1),
        ],
        "the refused follow-up opens turn 1 all the same"
    );
    assert_eq!(
        log[2].payload.get("code").and_then(Value::as_str),
        Some("scrub_residue"),
        "the row standing in for the follow-up is the residue row"
    );
}

/// An `agent.settings.env` value that happens to equal a closed-vocabulary wire string is masked
/// like any other occurrence, and the masked payload then no longer reads back as its own type.
/// Assumption A7 accepts that false positive **in masking**; it does not accept turning one into a
/// failed turn. The row is still persisted under the kind the unscrubbed event decided, and the
/// session carries on.
#[tokio::test]
async fn a_masked_vocabulary_token_costs_readability_not_the_turn() {
    let chat = chat_spec();
    // `read` is `tool_call.tool_kind`'s wire string as well as a plausible env value.
    let scrubber = MinimalScrubber::new(["read".to_owned()]);
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record(tool_call("call-1"))
        .await
        .expect("a masked vocabulary token is not a recording failure");
    recorder
        .record(chunk("carried on", "m1"))
        .await
        .expect("the turn continues");
    let summary = recorder
        .finish()
        .await
        .expect("a masking false positive never aborts the session");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::ToolCall, EventKind::AssistantText],
        "the row keeps the kind the unscrubbed event decided"
    );
    assert_eq!(
        log[0].payload.get("tool_kind").and_then(Value::as_str),
        Some("[REDACTED]"),
        "the false positive costs the value's readability and nothing else"
    );
    assert_eq!(
        log[0].tool_call_id.as_deref(),
        Some("call-1"),
        "the column is taken from the masked document, so it is still filled"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1],
        "an unreadable payload is a row like any other: no gap, no residue"
    );
    assert_eq!(summary.rows, 2);
}

/// What the render channel actually guarantees (module doc item 4, E2's ruling): it is offered the
/// **capture-time** scrub, one frame per chunk. A secret contained in one chunk never reaches the
/// tab; a secret split across two does, in cleartext, while the persisted row - scrubbed again
/// over the assembled run - carries neither half. Milestone 3 owns the chat tab and any stronger
/// render-path rule; this case exists so that weaker guarantee cannot drift silently.
#[tokio::test]
async fn the_render_channel_gets_capture_time_masking_only() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, Some(tx));

    let (head, tail) = SECRET.split_at(6);
    for text in [
        format!("whole {SECRET} here"),
        format!(" split {head}"),
        format!("{tail} done"),
    ] {
        recorder
            .record(chunk(&text, "m1"))
            .await
            .expect("recording must land");
    }
    let summary = recorder.finish().await.expect("close");
    assert_eq!(summary.dropped, 0, "the channel had room for every frame");

    let mut frames = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let DriverEvent::AssistantChunk(text) = frame.event else {
            panic!("every frame of this script is an assistant chunk");
        };
        frames.push(text.text);
    }
    assert_eq!(
        frames.len(),
        3,
        "one frame per chunk, not one per flushed row"
    );
    assert_eq!(
        frames[0], "whole [REDACTED] here",
        "a secret contained in one chunk is masked before the frame is offered"
    );
    assert!(
        frames[1].contains(head) && frames[2].contains(tail),
        "the halves of a split secret do reach the tab in cleartext: {frames:?}"
    );

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        text_of(&log[0]),
        "whole [REDACTED] here split [REDACTED] done",
        "the persisted row is masked over the assembled run, which is the stronger guarantee"
    );
}

/// `raw` reaches the persisted row **and** the render frame when a chat tab is attached: the
/// recorder hands the row its own copy only because there is a live channel that still needs the
/// envelope. With no channel the blob is moved instead, which no observer can tell apart - the
/// frame is what would notice, so the frame is what this case checks.
#[tokio::test]
async fn raw_reaches_both_the_row_and_a_live_render_channel() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, true, Some(tx));

    let wire = json!({ "sessionUpdate": "agent_message_chunk", "n": 1 });
    recorder
        .record(raw_env(
            DriverEvent::AssistantChunk(TextChunk {
                text: "hello".to_owned(),
                message_id: Some("m1".to_owned()),
            }),
            wire.clone(),
        ))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log[0].raw.as_ref(), Some(&wire), "the row keeps the blob");
    let frame = rx
        .try_recv()
        .expect("the attached tab is offered the frame");
    assert_eq!(
        frame.raw.as_ref(),
        Some(&wire),
        "and so does the frame: the row's copy is a copy, not a move"
    );
}

/// A session that closes without a `done` is a transport failure, not a silent success.
#[tokio::test]
async fn pump_reports_a_session_that_closed_without_done() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);
    let mut session = ScriptedSession::new(vec![chunk("half a sentence", "m1")]);

    let outcome = pump(&mut session, &mut recorder).await;
    assert!(
        matches!(outcome, Err(DriverError::Closed)),
        "a closed transport mid-turn is DriverError::Closed, got {outcome:?}"
    );
    recorder.finish().await.expect("close");
    assert_eq!(
        rows(&store, chat.step_id).await.len(),
        1,
        "what did arrive is still recorded"
    );
}
