//! The only task that owns a [`Backend`] (plan D4).
//!
//! The UI side holds two unbounded channels and nothing else: `R-NF-3` ("no store handle on the
//! render side") is a fact about this module's ownership, not a convention. [`serve`] is the pure
//! request -> reply function; [`spawn`] is a loop around it, and the test harness calls it inline
//! so snapshots need no sleeps (blueprint C.1, C.8).
//!
//! **The channels stay unbounded** (MOD-1 D4). A bounded pair would make `App::dispatch` either
//! `.await` on a full queue - on the render task, which `R-NF-3` forbids - or drop requests, and
//! the reply half would let a slow UI stall the one task that owns the store. Backpressure is
//! deferred until `PgStore` read latency has actually been measured against a populated database;
//! until then the queue depth is bounded in practice by the keystrokes a user can produce.

use chrono::{DateTime, Utc};
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::driver::{AgentSessionRef, DriverCaps, PermissionAnswer, PermissionRequestId};
use htui_agent::event::{DriverEnvelope, StopReason};
use htui_agent::probe::ProbeStatus;
use htui_core::model::{
    AgentId, AgentSummary, BoxInfo, DocumentHead, Item, ItemFilter, ItemId, ItemKindId,
    ItemKindPatch, ItemSummary, LinkGraph, Note, PhaseId, PhasePatch, ProjectId, ProjectPatch,
    RepoId, RepoPatch, RunSummary, Scope, SessionEvent, StepGraphId, StepGraphPatch, StepId,
    WorkspaceId, WorkspacePatch, WorkspaceSummary,
};
use htui_core::prompt::SettingKey;
use htui_core::store::{
    DeleteReach, DeleteTarget, ReadStore, Result as StoreResult, SettingRung, StoreError,
};
use htui_store::cache::refresh::{RefreshSettings, Refresher};
use htui_store::{Backend, ConnEvent, PgStore, Started, connect};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use crate::agent_worker::{AgentRuntime, Served};
use crate::catalogue::{self, CatalogueSnapshot};
use crate::hierarchy::{self, HierarchySnapshot, MirrorAfterDelete};
use crate::prompt_settings::{self, SettingsSnapshot};
use crate::ui::overlay::OverlayId;
use crate::ui::tabs::TabId;

/// Monotonic request counter, minted by `App::dispatch` (blueprint C.2).
pub type Seq = u64;

/// [`StoreRequest::name`] of the preview, named once so the deferred task's `Failed` replies carry
/// the same string the request does (`preview::run_preview` cannot call `name()` — it no longer has
/// the request).
pub const PROMPT_PREVIEW: &str = "prompt_preview";

/// Who asked, and therefore who the reply is addressed to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The shell itself: top-bar reads and the startup workspace list.
    App,
    /// A registered tab.
    Tab(TabId),
    /// An open overlay. A reply to a popped overlay is dropped.
    Overlay(OverlayId),
}

/// Everything the UI can ask the store for.
///
/// MOD-2 milestone 4 is the worked example of how this list grows: [`StoreRequest::StepEvents`]
/// is one variant, one arm inside [`serve`] and one [`StoreRequest::name`] arm, and the event
/// loop still does not change.
///
/// **No variant may carry a secret as a plain `String`.** This enum derives `Debug`, so does
/// [`RequestEnvelope`], and the tests print what came back with `{reply:?}`: a value here is one
/// `tracing::debug!` or one failed assertion away from a log. Nothing in MOD-15 milestone 3
/// carries one — every field below is a slug, a name, a branch or a path the user typed in the
/// open — but milestone 6's DSN entry does, and it owes a redacting newtype (a hand-written
/// `Debug` that prints `<redacted>`, no `Display`) rather than a `String` field. The masking end
/// of the same rule is [`TextField::masked`](crate::ui::TextField::masked).
#[derive(Debug, Clone)]
pub enum StoreRequest {
    /// Every workspace with its projects (switcher, startup scope).
    Workspaces,
    /// This box's row for the top bar. No probe: that is MOD-7.
    BoxInfo,
    /// How many runs of the scope are active (top bar).
    ActiveRuns {
        /// The workspace scope to count in.
        scope: Scope,
    },
    /// The Backlog list.
    Items {
        /// The workspace scope to read.
        scope: Scope,
        /// Conjunctive filter; `ItemFilter::default()` means "everything in scope".
        filter: ItemFilter,
    },
    /// One item with its body.
    Item(ItemId),
    /// The link neighbourhood of an item.
    Links {
        /// Root of the traversal.
        id: ItemId,
        /// How many hops to follow.
        hops: u8,
    },
    /// The item's documents without bodies.
    Documents(ItemId),
    /// The item's notes.
    Notes(ItemId),
    /// The item's runs with their steps.
    Runs(ItemId),
    /// The agent registry with this box's `agent_box` row, for the Settings tab (MOD-2 D14).
    ///
    /// Not scoped: `agent` is a global table, not a workspace one.
    Agents,
    /// The persisted replay log of one step (MOD-2 D38, `R-HIS-2`).
    ///
    /// A read like any other, answered once: a replay is history, not a stream, so it costs one
    /// request rather than a subscription that could still be arriving when the user leaves.
    StepEvents(StepId),
    /// Assemble the prompt one item would be given, and render nothing to the store (MOD-2 D102).
    ///
    /// Served as **deferred work on an owned [`Backend`] clone**, never in the worker's `select!`
    /// arm and never on the UI task (`R-NF-3`): the assembly reads eight tables and renders the
    /// whole prompt. The shape [`ProbeAgents`](Self::ProbeAgents) established, for the same reason.
    ///
    /// `template_name` is `None` on the read the Backlog tab issues with the other five, and
    /// `Some` once the user has cycled the picker; `scope` bounds the upstream walk (plan D95).
    /// **Nothing here writes**: no `set_step_prompt`, no `prompt` event, no run.
    PromptPreview {
        /// The item to preview.
        item: ItemId,
        /// Which template to render, or `None` for the project's default (blueprint D.4).
        template_name: Option<String>,
        /// The active workspace, which bounds the upstream walk.
        scope: Scope,
    },
    /// Start a chat: mint the `run` / `run_step` pair, open a session, send the first prompt
    /// (MOD-2 D27). Answered by [`StoreReply::ChatAccepted`] once the handshake is through, then
    /// by a [`StoreReply::Chat`] frame per event for the life of the session.
    ChatStart {
        /// The project the chat belongs to.
        project_id: ProjectId,
        /// Which registry row to talk to.
        agent_id: AgentId,
        /// `None` means the agent's `default_model`, and then the agent's own default.
        model: Option<String>,
        /// What the user typed.
        prompt: String,
    },
    /// A follow-up turn in a live chat.
    ChatSend {
        /// The step the chat records against.
        step_id: StepId,
        /// What the user typed.
        text: String,
    },
    /// An answer to a parked permission request (`R-TUI-6`).
    ChatAnswer {
        /// The step the chat records against.
        step_id: StepId,
        /// Which request is being answered.
        request_id: PermissionRequestId,
        /// The chosen option, or a cancellation.
        answer: PermissionAnswer,
    },
    /// End a live chat.
    ChatCancel {
        /// The step the chat records against.
        step_id: StepId,
    },
    /// Probe every enabled registry row on this box and write `agent_box` (MOD-2 D53, `R-AGT-6`).
    ///
    /// Served by the agent runtime's own task, never inside the loop: a probe spawns processes and
    /// may wait `HANDSHAKE_TIMEOUT` on each of them (`R-NF-3`). Answered exactly once, with
    /// [`StoreReply::Agents`] — the reply the Settings section already renders, so the probe needs
    /// no second arm there — or with [`StoreReply::Failed`].
    ProbeAgents,
    /// Pre-flight an adapter install for one registry row (MOD-20 D13, D18).
    ///
    /// One registry read, one `HEAD`, **no archive byte**: what this box would fetch and how it
    /// could be verified, so the user can say yes to a value rather than to a promise. Served by
    /// the agent runtime's own task and answered exactly once, with
    /// [`StoreReply::Install`]`(`[`InstallFrame::Plan`]`)` or with a failure.
    InstallPlan {
        /// Which registry row to plan for.
        agent_id: AgentId,
    },
    /// The user said `y` to exactly this plan (MOD-20 D12: the plan **is** the consent evidence).
    ///
    /// Many replies, all at this request's own `seq`: [`InstallFrame::Progress`] as the phases go
    /// by, then one of [`InstallFrame::Done`], [`InstallFrame::Cancelled`] or
    /// [`InstallFrame::Failed`]. The stream shape is [`StoreRequest::ChatStart`]'s, for the same
    /// reason: `App::is_fresh` passes every frame until the section confirms another install.
    InstallConfirm {
        /// The plan the consent pane rendered, unchanged. Boxed because it dwarfs every other
        /// variant of this enum.
        plan: Box<htui_agent::InstallPlan>,
    },
    /// Stop the running install (MOD-20 D18).
    ///
    /// Answered [`InstallFrame::Cancelling`] at once, at this request's own `seq`; the running
    /// install's own stream then ends [`InstallFrame::Cancelled`] at *its* `seq`. Cooperative,
    /// through a token, so the task can sweep its staging entry and send that last frame — an
    /// `abort()` could do neither, and stays the shutdown path.
    InstallCancel,
    /// Start a login for one registry row (MOD-21 D18, `R-AGT-9`).
    ///
    /// Served by the agent runtime's own task, never inside the loop: a flow spawns the adapter
    /// and then waits on a **human** in a browser, which is minutes (`R-NF-3`). Answered with
    /// [`StoreReply::Auth`]`(`[`AuthFrame::Methods`]`)` at this `seq` — the agent's own live method
    /// list, not the stored snapshot's — or, before anything is spawned, with
    /// [`StoreReply::Failed`].
    AuthStart {
        /// Which registry row to log in.
        agent_id: AgentId,
    },
    /// The user chose from the live method list (MOD-21 D7, D18).
    ///
    /// Every later frame of the flow — [`AuthFrame::Line`], [`AuthFrame::Url`],
    /// [`AuthFrame::Done`] and the rest — carries **this** request's `seq`, not the
    /// [`AuthStart`](Self::AuthStart)'s: `App::is_fresh` keys on the request *kind*
    /// (`app/state.rs:290-294`), so the chooser's answer is what later frames must be fresh
    /// against. The same plan-then-confirm split [`InstallConfirm`](Self::InstallConfirm) uses.
    AuthChoose {
        /// The method the user picked, or a logout.
        choice: AuthChoice,
    },
    /// Open the link the pane is showing, through `htui`'s own opener (MOD-21 D17).
    ///
    /// Answered exactly once at its own `seq`: [`AuthFrame::Opened`] when the opener was spawned —
    /// never "the browser opened", which `htui` cannot know — or [`StoreReply::Failed`]. A
    /// refusal here leaves the running flow and the pane exactly as they were (blueprint H-22).
    AuthOpen {
        /// The link, as the adapter printed it.
        url: String,
    },
    /// Stop the running login (MOD-21 D3, D13).
    ///
    /// Answered [`AuthFrame::Cancelling`] at once, at this request's own `seq`; the flow's own
    /// stream then ends [`AuthFrame::Cancelled`] at *its* `seq`. Cooperative, through a token, so
    /// the task can kill its child and send that last frame.
    AuthCancel,
    /// The top bar's store field and the pending-migration count (plan D11).
    StoreState,
    /// Apply the pending migrations (`R-STO-5`), after the user answered `y`.
    ApplyMigrations,

    // The twelve hierarchy requests of `Settings > Hierarchy` (MOD-15 milestone 3, D5/D6). Each
    // carries **only what the user typed**: `created_by` and the box a path belongs to are filled
    // in by [`crate::hierarchy::serve`] from `Backend` identity, so no view holds a `UserId` or a
    // `BoxId`. All twelve are served through one or-ed arm of [`try_serve`].
    /// The whole tree of one workspace for `Settings > Hierarchy` (D5).
    Hierarchy(WorkspaceId),
    /// `created_by` is the worker's (`Backend::this_user`); the section never holds a `UserId`.
    CreateWorkspace {
        /// `workspace.slug`, unique across the database.
        slug: String,
        /// `workspace.name`.
        name: String,
        /// `workspace.description`.
        description: String,
    },
    /// CAS on `updated_at` (M1 D3): `Stale` answers [`StoreReply::HierarchyStale`].
    UpdateWorkspace {
        /// The row to edit.
        id: WorkspaceId,
        /// The `updated_at` the editor opened on.
        expected: DateTime<Utc>,
        /// The columns to write.
        patch: WorkspacePatch,
    },
    /// This box's root for a workspace, canonicalised or refused (D8).
    SetWorkspaceRoot {
        /// The workspace the root belongs to.
        id: WorkspaceId,
        /// The path **as the user typed it**; the guard canonicalises or refuses it.
        path: String,
    },
    /// Creates a project and links it at `position = links.len()`.
    CreateProject {
        /// The workspace to link it into.
        workspace: WorkspaceId,
        /// `project.slug`, unique across the database.
        slug: String,
        /// `project.name`.
        name: String,
        /// `project.description`.
        description: String,
    },
    /// CAS on `updated_at`, as [`StoreRequest::UpdateWorkspace`].
    UpdateProject {
        /// The row to edit.
        id: ProjectId,
        /// The `updated_at` the editor opened on.
        expected: DateTime<Utc>,
        /// The columns to write.
        patch: ProjectPatch,
    },
    /// Creates a repo; `is_primary: true` demotes the project's current primary in the same
    /// transaction (M1 D10).
    CreateRepo {
        /// The project the repo belongs to.
        project: ProjectId,
        /// `repo.name`, unique within the project.
        name: String,
        /// `repo.remote_url`.
        remote_url: Option<String>,
        /// `repo.default_branch`.
        default_branch: String,
        /// `repo.is_primary`.
        is_primary: bool,
    },
    /// CAS on `updated_at`. `project` is the repo's owner, so the reply can re-read its workspace:
    /// the seam has no `repo(id)` reader and a `Stale` outcome must answer the same tree an
    /// `Applied` one does.
    UpdateRepo {
        /// The repo's project.
        project: ProjectId,
        /// The row to edit.
        id: RepoId,
        /// The `updated_at` the editor opened on.
        expected: DateTime<Utc>,
        /// The columns to write.
        patch: RepoPatch,
    },
    /// This box's checkout path for a repo, canonicalised or refused (D8).
    SetRepoPath {
        /// The repo's project, for the same reason [`StoreRequest::UpdateRepo`] carries it.
        project: ProjectId,
        /// The repo the checkout belongs to.
        repo: RepoId,
        /// The path **as the user typed it**.
        path: String,
    },
    /// Counts before the act (PRD D13); the reply is `None` when the target is already gone.
    DeleteReach(DeleteTarget),
    /// Removes a workspace, its links and its box paths; its projects survive (M1 D4).
    DeleteWorkspace(WorkspaceId),
    /// Removes a project and its whole history, then rebuilds the mirror from the worker (D10).
    DeleteProject(ProjectId),

    // MOD-15 milestone 4 (D5): the nine catalogue requests, served by [`crate::catalogue`]. Every
    // write carries the `Scope` because the tree the reply re-reads *is* the scope (M4 D7), and all
    // nine are served through one further or-ed arm of [`try_serve`].
    /// The catalogue of every project in the scope (M4 D2): kinds, graphs, phases. One request per
    /// event, never one per project — the staleness index keeps only the newest of a variant, so N
    /// requests of one variant would leave all but one project undrawn.
    Catalogue(Scope),
    /// Creates a kind in `project` with `graph` as its default graph. Answers
    /// [`StoreReply::Catalogue`].
    CreateKind {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The project the kind belongs to.
        project: ProjectId,
        /// `item_kind.prefix`, checked by `ItemKind::prefix_is_valid` before the statement.
        prefix: String,
        /// `item_kind.name`, unique within the project.
        name: String,
        /// `item_kind.description`.
        description: String,
        /// `item_kind.default_graph_id`, which must belong to `project`.
        graph: StepGraphId,
        /// `item_kind.position`.
        position: i32,
    },
    /// CAS on `item_kind.updated_at` (M1 D3): `Stale` answers [`StoreReply::CatalogueStale`]. A
    /// prefix change touches no `item` and no counter (PRD D12).
    UpdateKind {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The row to edit.
        id: ItemKindId,
        /// The `updated_at` the editor opened on.
        expected: DateTime<Utc>,
        /// The columns to write.
        patch: ItemKindPatch,
    },
    /// Deletes a kind no item holds; a held kind is refused by the seam with its own sentence
    /// (PRD D6/D10). Answers [`StoreReply::KindDeleted`] with the mirror rebuilt (M4 D12).
    DeleteKind {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The row to delete.
        id: ItemKindId,
    },
    /// Creates an empty graph in `project`; phases are added one at a time. Answers
    /// [`StoreReply::Catalogue`].
    CreateGraph {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The project the graph belongs to.
        project: ProjectId,
        /// `step_graph.name`, unique within the project.
        name: String,
        /// `step_graph.description`.
        description: String,
    },
    /// CAS on `step_graph.updated_at` (M1 D3): `Stale` answers [`StoreReply::CatalogueStale`].
    UpdateGraph {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The row to edit.
        id: StepGraphId,
        /// The `updated_at` the editor opened on.
        expected: DateTime<Utc>,
        /// The columns to write.
        patch: StepGraphPatch,
    },
    /// Creates a phase from `seed::phase_row`'s frozen defaults plus these five columns (M4 D9):
    /// the app is never a second source of ANA-2's defaults.
    CreatePhase {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The graph the phase belongs to.
        graph: StepGraphId,
        /// `step_graph_phase.name`, which is also its `output_kind` at create time (D17b).
        name: String,
        /// `step_graph_phase.position`, unique within the graph.
        position: i32,
        /// `step_graph_phase.template_name`.
        template_name: String,
        /// `step_graph_phase.gate_hard`.
        gate_hard: bool,
        /// `step_graph_phase.input_kinds`, replaced whole.
        input_kinds: Vec<String>,
    },
    /// CAS on `step_graph_phase.updated_at` (M1 D3) over [`PhasePatch`]'s five columns.
    UpdatePhase {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The row to edit.
        id: PhaseId,
        /// The `updated_at` the editor opened on.
        expected: DateTime<Utc>,
        /// The columns to write.
        patch: PhasePatch,
    },
    /// `token_budget` on the `Phase` rung (M1 D8, M4 D6): `Some` is `set_setting`, `None` is
    /// `clear_setting` so the project or app rung answers. CAS on the phase's `updated_at`.
    SetPhaseBudget {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The phase whose `token_budget` is written.
        phase: PhaseId,
        /// The phase's `updated_at` the editor opened on.
        expected: DateTime<Utc>,
        /// The budget to set, or `None` to clear it and let the rung below answer (D16).
        budget: Option<i64>,
    },
    /// The ten prompt keys on `App` plus the `Project`-rung keys of every scope project (M5 D3).
    PromptSettings(Scope),
    /// `set_setting` on `App` or `Project` (M5 D7). `expected: None` is "I expect no row", accepted
    /// on `App` only (`traits.rs:553-560`); the section always passes `Some(project.updated_at)`
    /// for a project. CAS on the rung row's `updated_at`.
    SetSetting {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which rung is written; never `Phase` from the prompt section (M5 D8).
        rung: SettingRung,
        /// Which key.
        key: SettingKey,
        /// The JSON to store: an integer, or an `f64` for the one fraction key (M5 D12).
        value: Value,
        /// The rung row's `updated_at` the editor opened on; `None` for an absent `App` row.
        expected: Option<DateTime<Utc>>,
    },
    /// `clear_setting` on `App` or `Project`, so the rung below answers (M5 D10).
    ClearSetting {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which rung is cleared.
        rung: SettingRung,
        /// Which key.
        key: SettingKey,
        /// The rung row's `updated_at` the editor opened on.
        expected: DateTime<Utc>,
    },
}

impl StoreRequest {
    /// Stable name of the request, used in [`StoreReply::Failed`] and in logs.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Workspaces => "workspaces",
            Self::BoxInfo => "box_info",
            Self::ActiveRuns { .. } => "active_runs",
            Self::Items { .. } => "items",
            Self::Item(_) => "item",
            Self::Links { .. } => "links",
            Self::Documents(_) => "documents",
            Self::Notes(_) => "notes",
            Self::Runs(_) => "runs",
            Self::Agents => "agents",
            Self::StepEvents(_) => "step_events",
            Self::PromptPreview { .. } => PROMPT_PREVIEW,
            Self::ChatStart { .. } => "chat_start",
            Self::ChatSend { .. } => "chat_send",
            Self::ChatAnswer { .. } => "chat_answer",
            Self::ChatCancel { .. } => "chat_cancel",
            Self::ProbeAgents => "probe_agents",
            Self::InstallPlan { .. } => "install_plan",
            Self::InstallConfirm { .. } => "install_confirm",
            Self::InstallCancel => "install_cancel",
            Self::AuthStart { .. } => "auth_start",
            Self::AuthChoose { .. } => "auth_choose",
            Self::AuthOpen { .. } => "auth_open",
            Self::AuthCancel => "auth_cancel",
            Self::StoreState => "store_state",
            Self::ApplyMigrations => "apply_migrations",
            // The twelve of `hierarchy::REQUEST_NAMES`, in that order. String literals, because
            // this stays a `const fn`.
            Self::Hierarchy(..) => "hierarchy",
            Self::CreateWorkspace { .. } => "create_workspace",
            Self::UpdateWorkspace { .. } => "update_workspace",
            Self::SetWorkspaceRoot { .. } => "set_workspace_root",
            Self::CreateProject { .. } => "create_project",
            Self::UpdateProject { .. } => "update_project",
            Self::CreateRepo { .. } => "create_repo",
            Self::UpdateRepo { .. } => "update_repo",
            Self::SetRepoPath { .. } => "set_repo_path",
            Self::DeleteReach(..) => "delete_reach",
            Self::DeleteWorkspace(..) => "delete_workspace",
            Self::DeleteProject(..) => "delete_project",
            // The nine of `catalogue::REQUEST_NAMES`, in that order (MOD-15 M4 D5).
            Self::Catalogue(..) => "catalogue",
            Self::CreateKind { .. } => "create_kind",
            Self::UpdateKind { .. } => "update_kind",
            Self::DeleteKind { .. } => "delete_kind",
            Self::CreateGraph { .. } => "create_graph",
            Self::UpdateGraph { .. } => "update_graph",
            Self::CreatePhase { .. } => "create_phase",
            Self::UpdatePhase { .. } => "update_phase",
            Self::SetPhaseBudget { .. } => "set_phase_budget",
            // The three of `prompt_settings::REQUEST_NAMES`, in that order (MOD-15 M5 D7).
            Self::PromptSettings(..) => "prompt_settings",
            Self::SetSetting { .. } => "set_setting",
            Self::ClearSetting { .. } => "clear_setting",
        }
    }
}

/// The answer to exactly one [`StoreRequest`].
#[derive(Debug, Clone)]
pub enum StoreReply {
    /// Answer to [`StoreRequest::Workspaces`], ordered by name.
    Workspaces(Vec<WorkspaceSummary>),
    /// Answer to [`StoreRequest::BoxInfo`]; `None` when no box row is registered.
    BoxInfo(Option<BoxInfo>),
    /// Answer to [`StoreRequest::ActiveRuns`].
    ActiveRuns(usize),
    /// Answer to [`StoreRequest::Items`].
    Items(Vec<ItemSummary>),
    /// Answer to [`StoreRequest::Item`]; boxed because `Item` dwarfs every other variant.
    Item(Box<Option<Item>>),
    /// Answer to [`StoreRequest::Links`].
    Links(LinkGraph),
    /// Answer to [`StoreRequest::Documents`].
    Documents(Vec<DocumentHead>),
    /// Answer to [`StoreRequest::Notes`].
    Notes(Vec<Note>),
    /// Answer to [`StoreRequest::Runs`].
    Runs(Vec<RunSummary>),
    /// Answer to [`StoreRequest::Agents`], ordered by `agent.name`.
    Agents(Vec<AgentSummary>),
    /// Answer to [`StoreRequest::StepEvents`], in `seq` order (MOD-2 D38).
    ///
    /// `events` is `None` when the backend does not hold the step — the mirror's window is the
    /// last N steps (`docs/ANA-9.md` §4.4) — and the view says "this step is not on this box".
    /// `Some(vec![])` is the other answer entirely: the step is here and recorded nothing.
    /// Collapsing the two would make an unsynced step look like an empty conversation, which is
    /// the one reading `R-HIS-1` forbids.
    ///
    /// `step_id` travels with the rows because the addressee may have asked twice: it is how a
    /// tab tells the replay it is showing from the one it asked for next.
    StepEvents {
        /// The step the rows belong to.
        step_id: StepId,
        /// The step's rows, or `None` for "not on this box".
        events: Option<Vec<SessionEvent>>,
    },
    /// Answer to [`StoreRequest::PromptPreview`] (MOD-2 D102), from the deferred task's own task.
    ///
    /// Boxed for the reason [`StoreReply::Item`] is: it carries a whole assembled prompt — the
    /// text, its digest, the section rows and the trim record — and would otherwise set the size of
    /// this enum for every other variant.
    ///
    /// A refusal is an answer too: `PromptPreview::outcome` is `Err` when the assembler declined
    /// (`prompt budget too small`, `skills exceed max_skill_tokens`, an unknown placeholder), and
    /// only a **store** failure comes back as [`StoreReply::Failed`].
    PromptPreview(Box<crate::preview::PromptPreview>),
    /// One frame of an adapter install (MOD-20 D18).
    ///
    /// The plan answers a [`StoreRequest::InstallPlan`]; every frame of a confirm answers the
    /// [`StoreRequest::InstallConfirm`] that started it, at that request's own `seq`; the
    /// acknowledgement of a cancel answers the [`StoreRequest::InstallCancel`] at *its* `seq`.
    Install(InstallFrame),
    /// One frame of a login (MOD-21 D18).
    ///
    /// The method list answers the [`StoreRequest::AuthStart`] that asked for it; every frame from
    /// the choice onwards answers the [`StoreRequest::AuthChoose`] at *that* request's `seq`; an
    /// [`AuthFrame::Opened`] answers its own [`StoreRequest::AuthOpen`]; and an
    /// [`AuthFrame::Cancelling`] answers the [`StoreRequest::AuthCancel`].
    Auth(AuthFrame),
    /// One frame of a live chat's stream (MOD-2 D27).
    ///
    /// Many of these answer one [`StoreRequest::ChatStart`], all carrying that request's `seq`, so
    /// `App::is_fresh` passes every one of them until the tab starts another chat.
    Chat(ChatFrame),
    /// The chat is open: its handshake finished and its first prompt is on the wire.
    ChatAccepted {
        /// The step every event of this chat is recorded against.
        step_id: StepId,
        /// The agent-side session id, for a later `session/load`.
        session_ref: Option<AgentSessionRef>,
        /// What this transport can do, for the tab's capability banner.
        caps: DriverCaps,
        /// `Writer::label()` of the store this chat records into: `memory` or `online` (MOD-2 D42;
        /// `buffered` is the third label, which no accepted chat has carried since MOD-25 made an
        /// offline one refuse).
        ///
        /// It travels on the acceptance because the tab must be able to say that a conversation is
        /// only on this disk, and it cannot ask: `R-NF-3` keeps every store handle on the worker's
        /// side, and inferring "offline therefore buffered" from the top bar would be a guess about
        /// a backend the tab does not hold.
        writer_label: &'static str,
    },
    /// Answer to [`StoreRequest::StoreState`].
    StoreState {
        /// `Backend::label()`: `memory`, `connecting`, `online` or `offline · <age>`.
        label: String,
        /// How many migrations are pending; `None` when the question does not apply — a memory
        /// backend, an offline one, or a connected one whose schema is up to date.
        migrations_pending: Option<usize>,
    },
    /// Answer to [`StoreRequest::ApplyMigrations`].
    MigrationsApplied {
        /// How many were pending before the run; `0` when there was nothing to do.
        applied: usize,
    },
    /// Answer to [`StoreRequest::Hierarchy`] and to every hierarchy write that applied: the tree
    /// as it is now (D6).
    ///
    /// A whole snapshot rather than the single row a write returned, so the section has one
    /// source of truth and never patches its rows locally. `None` only for a
    /// [`StoreRequest::Hierarchy`] of a workspace that does not exist — the nil startup scope, or
    /// one deleted elsewhere.
    Hierarchy(Option<Box<HierarchySnapshot>>),
    /// A CAS write found the row changed (M1 D3): the tree as it is now, for the editor to
    /// reload against (D7). Never a scope change — the editor is still open.
    HierarchyStale(Box<HierarchySnapshot>),
    /// Answer to [`StoreRequest::DeleteReach`]; `None` when the target is already gone.
    DeleteReach(Option<DeleteReach>),
    /// Answer to [`StoreRequest::DeleteWorkspace`] / [`StoreRequest::DeleteProject`]: what was
    /// removed, and what the mirror did about it (D10).
    Deleted {
        /// What was deleted.
        target: DeleteTarget,
        /// The rows it took, per table — the same counts the warning pane showed.
        reach: DeleteReach,
        /// What the worker did to the mirror afterwards.
        mirror: MirrorAfterDelete,
    },
    /// The scope's catalogue, freshly read: the answer to [`StoreRequest::Catalogue`] and to every
    /// catalogue write that applied (M4 D2/D7).
    Catalogue(Box<CatalogueSnapshot>),
    /// A catalogue write missed its CAS token (M4 D8, PRD D8): the tree as it is now, for the
    /// editor to reload against. The editor keeps its typed text and retries only on `Enter`.
    CatalogueStale(Box<CatalogueSnapshot>),
    /// A kind is gone and the mirror was rebuilt, or could not be (M4 D12).
    ///
    /// Its own variant rather than [`StoreReply::Deleted`] because `DeleteTarget` is `Workspace`
    /// or `Project` only, and widening it would be a seam change this milestone does not make.
    KindDeleted {
        /// What the worker did to the mirror after the delete.
        mirror: MirrorAfterDelete,
        /// The scope's catalogue without the kind.
        catalogue: Box<CatalogueSnapshot>,
    },
    /// The scope's prompt settings, freshly read: the answer to [`StoreRequest::PromptSettings`]
    /// and to every settings write that applied (M5 D3/D7).
    PromptSettings(Box<SettingsSnapshot>),
    /// A settings write missed its CAS token (M5 D14, PRD D8): the rungs as they are now, for the
    /// editor to reload against. The editor keeps its typed text and retries only on `Enter`.
    PromptSettingsStale(Box<SettingsSnapshot>),
    /// The store failed. `request` is [`StoreRequest::name`].
    Failed {
        /// Which request failed.
        request: &'static str,
        /// The `StoreError`, rendered through `Display`.
        message: String,
    },
}

/// One frame of a chat stream.
#[derive(Debug, Clone)]
pub enum ChatFrame {
    /// A recorded, **scrubbed** event: the copy the recorder made on its way to the store, never
    /// the transport's own (`R-SEC-3` — the screen may not be less masked than the row).
    ///
    /// The three kinds `htui` authors itself — `prompt`, `follow_up`, `permission_answer` — arrive
    /// here shaped as `other` events with those names. The recorder wrote the real row first; this
    /// is only the copy the tab renders.
    Event(Box<DriverEnvelope>),
    /// The session ended; `stop_reason` is `cancelled` when a turn was cut.
    Ended {
        /// Why the last turn ended.
        stop_reason: StopReason,
    },
    /// The session died. The run is closed `failed`.
    Failed {
        /// What went wrong, rendered.
        message: String,
    },
}

/// One frame of an adapter install (MOD-20 D18).
///
/// Deliberately the [`ChatFrame`] shape: a pre-flight, a progress stream and a terminal frame are
/// one request answered many times, and the section renders whichever it last received. Every
/// terminal frame — [`Done`](Self::Done), [`Cancelled`](Self::Cancelled),
/// [`Failed`](Self::Failed) — clears the section's in-flight state, so a stream that ends is a
/// pane that closes whatever the outcome was.
#[derive(Debug, Clone)]
pub enum InstallFrame {
    /// The pre-flight's answer: what this box would fetch, and what the consent pane renders.
    ///
    /// Boxed for the reason [`StoreReply::Item`] is: the plan carries every coordinate of an
    /// install and would otherwise set the size of this enum for every other variant.
    Plan(Box<htui_agent::InstallPlan>),
    /// Where the install has got to. At most one per
    /// [`PROGRESS_EVERY`](htui_agent::install::PROGRESS_EVERY), which is `event_loop`'s own tick:
    /// a faster stream would only queue frames nobody ever sees.
    Progress {
        /// Which step is running.
        phase: htui_agent::InstallPhase,
        /// How far into it.
        done: u64,
        /// The denominator, when the archive declared one.
        total: Option<u64>,
    },
    /// The pipeline finished and the probe has spoken. **The row was already written** when this
    /// arrives, so the [`StoreRequest::Agents`] the section issues on it reads the new one.
    Done(Box<htui_agent::InstallOutcome>),
    /// A [`StoreRequest::InstallCancel`] was accepted. Not the end of the stream: the install's
    /// own frames end it with [`Cancelled`](Self::Cancelled).
    Cancelling,
    /// The install stopped because it was asked to, leaving nothing the row's glob resolves.
    Cancelled,
    /// The install stopped for a reason the user may be able to act on.
    ///
    /// `manual` is `Some` exactly when the failure was the network, because that is the failure
    /// the user can route around by hand (MOD-20 D20); every other failure is a sentence on the
    /// status line.
    Failed {
        /// What went wrong, rendered.
        message: String,
        /// The by-hand steps, derived from the row and the registry helper.
        manual: Option<Box<htui_agent::ManualSteps>>,
    },
}

/// One frame of a login (MOD-21 D18).
///
/// The [`InstallFrame`] shape, for the same reason: a method list, a stream and a terminal frame
/// are one request answered many times, and the section renders whichever it last received. Every
/// terminal frame — [`Done`](Self::Done), [`Refused`](Self::Refused),
/// [`Cancelled`](Self::Cancelled), [`Idle`](Self::Idle), [`Failed`](Self::Failed) — clears the
/// section's in-flight state.
///
/// **Nothing here can hold a credential** (`R-SEC-2`, `R-ID-7`): every field is an id the agent
/// advertised, a sentence the agent itself wrote to its own stderr, a link it printed, or a status
/// the probe decided. `htui` never reads the credential a login leaves behind — it asks the probe
/// whether one exists.
#[derive(Debug, Clone)]
pub enum AuthFrame {
    /// The agent's own `initialize` answer, once, before the user is asked anything (MOD-21 D7).
    Methods {
        /// Every method the chooser may offer, in the agent's order.
        methods: Vec<AuthMethodInfo>,
        /// The agent advertised a logout verb, so the chooser offers one.
        logout: bool,
        /// `terminal`-typed methods, named so the chooser can say why they are missing. The spec
        /// forbids passing one to `authenticate` and `htui` never does (MOD-21 D4, D21).
        hidden: Vec<AuthMethodInfo>,
    },
    /// One line the adapter wrote to its own stderr, as it wrote it.
    Line(String),
    /// A `http(s)` link seen on that stream for the first time in this flow (MOD-21 D15).
    Url(String),
    /// The opener was **spawned**. Not "the browser opened": `htui` does not own that tree and
    /// cannot say (MOD-21 D17).
    Opened,
    /// The call returned, the row was **re-probed and written**, and this is the probe's verdict —
    /// never the flow's (MOD-21 D6, `R-AGT-6`).
    Done {
        /// Which call returned, so the notice can say "logged in" or "logged out".
        call: AuthCall,
        /// What `probe_agent` decided about this box afterwards.
        status: ProbeStatus,
    },
    /// The agent answered the call with a JSON-RPC error, in its own words (MOD-21 D5).
    ///
    /// An answer, not a failure: the live `-32602` names the variable the user has to set, which
    /// is the one sentence worth rendering verbatim. Nothing was written.
    Refused {
        /// The agent's own text, plus its stderr tail when there was one.
        message: String,
    },
    /// A [`StoreRequest::AuthCancel`] was accepted. Not the end of the stream: the flow's own
    /// frames end it with [`Cancelled`](Self::Cancelled).
    Cancelling,
    /// The flow stopped because it was asked to, leaving `agent_box` exactly as it found it.
    Cancelled,
    /// The flow went silent for this long — no line, no event, no choice — and was killed
    /// (MOD-21 D13).
    ///
    /// Its own frame rather than a [`Cancelled`](Self::Cancelled), so the notice can say what
    /// happened instead of implying the user did it.
    Idle {
        /// The cap that elapsed with no sign of life.
        after: std::time::Duration,
    },
    /// The flow died: the spawn, the wire, or the row write.
    Failed {
        /// What went wrong, rendered.
        message: String,
    },
}

/// A request on its way to the worker.
#[derive(Debug)]
pub struct RequestEnvelope {
    /// Staleness stamp (blueprint C.2).
    pub seq: Seq,
    /// Who asked.
    pub origin: Origin,
    /// What was asked.
    pub request: StoreRequest,
}

/// A reply on its way back to the UI.
#[derive(Debug, Clone)]
pub struct ReplyEnvelope {
    /// The `seq` of the request this answers.
    pub seq: Seq,
    /// Who asked, and therefore who receives it.
    pub origin: Origin,
    /// The answer.
    pub reply: StoreReply,
}

/// Serves one request. Pure: no channels, no state, no logging.
///
/// A `StoreError` becomes [`StoreReply::Failed`] rather than a panic or a dropped reply, so the
/// asking view always hears back exactly once.
///
/// The two connection-aware requests are answered here as far as a `&Backend` can answer them —
/// [`StoreRequest::StoreState`] without a pending count, [`StoreRequest::ApplyMigrations`] with
/// nothing to apply — because the pending count and the store it belongs to are state of the
/// [`spawn`] loop, which intercepts both before reaching here. This is the answer the test harness
/// and a `--demo` shell get, and both are right: a `MemStore` has no schema to migrate.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> StoreReply {
    match try_serve(backend, request).await {
        Ok(reply) => reply,
        Err(err) => failed(request.name(), &err),
    }
}

/// [`serve`] with the `StoreError` still visible, for the one caller that has to act on it.
///
/// [`spawn`] needs to tell [`StoreError::Unreachable`] from every other failure so it can drop an
/// `Online` backend onto the mirror, and [`StoreReply::Failed`] carries only rendered text - which
/// is what the asking view shows. Widening the reply with a machine-readable field would put a
/// store concept into every view for the benefit of one match in this file.
async fn try_serve(backend: &Backend, request: &StoreRequest) -> StoreResult<StoreReply> {
    Ok(match request {
        StoreRequest::Workspaces => StoreReply::Workspaces(backend.workspaces().await?),
        StoreRequest::BoxInfo => StoreReply::BoxInfo(backend.box_info().await?),
        StoreRequest::ActiveRuns { scope } => {
            StoreReply::ActiveRuns(backend.active_runs(scope).await?)
        }
        StoreRequest::Items { scope, filter } => {
            StoreReply::Items(backend.items(scope, filter).await?)
        }
        StoreRequest::Item(id) => StoreReply::Item(Box::new(backend.item(*id).await?)),
        StoreRequest::Links { id, hops } => StoreReply::Links(backend.links(*id, *hops).await?),
        StoreRequest::Documents(id) => StoreReply::Documents(backend.documents(*id).await?),
        StoreRequest::Notes(id) => StoreReply::Notes(backend.notes(*id).await?),
        StoreRequest::Runs(id) => StoreReply::Runs(backend.runs(*id).await?),
        StoreRequest::Agents => StoreReply::Agents(backend.agents().await?),
        // Served through the ordinary read path on purpose: an `Unreachable` from it drops an
        // `Online` backend onto the mirror exactly as any other read does, and the replay then
        // answers from whatever the mirror kept.
        StoreRequest::StepEvents(step) => StoreReply::StepEvents {
            step_id: *step,
            events: backend.step_events(*step).await?,
        },
        // The four chat requests need the worker loop's own state (the live sessions), and the
        // probe, the preview, the three install requests and MOD-21's four login ones need the
        // runtime that owns their tasks, so all thirteen are served ahead of this function, exactly
        // as `ApplyMigrations` is. One of them that reaches here at all belongs to a caller with no
        // runtime — the test harness without one — and saying so is more use than a panic.
        StoreRequest::PromptPreview { .. }
        | StoreRequest::ChatStart { .. }
        | StoreRequest::ChatSend { .. }
        | StoreRequest::ChatAnswer { .. }
        | StoreRequest::ChatCancel { .. }
        | StoreRequest::ProbeAgents
        | StoreRequest::InstallPlan { .. }
        | StoreRequest::InstallConfirm { .. }
        | StoreRequest::InstallCancel
        | StoreRequest::AuthStart { .. }
        | StoreRequest::AuthChoose { .. }
        | StoreRequest::AuthOpen { .. }
        | StoreRequest::AuthCancel => StoreReply::Failed {
            request: request.name(),
            message: "no agent runtime in this build".to_owned(),
        },
        // The twelve hierarchy requests, or-ed rather than guarded: this `match` has no wildcard,
        // and an arm with a guard does not count towards exhaustivity, so `_ if …` would be an
        // E0004 here (MOD-15 M3 plan F-12). The `?` is what keeps `spawn`'s `go_offline` working:
        // an `Unreachable` from `hierarchy::serve` still drops an `Online` backend onto the mirror
        // exactly as any other read does.
        StoreRequest::Hierarchy(..)
        | StoreRequest::CreateWorkspace { .. }
        | StoreRequest::UpdateWorkspace { .. }
        | StoreRequest::SetWorkspaceRoot { .. }
        | StoreRequest::CreateProject { .. }
        | StoreRequest::UpdateProject { .. }
        | StoreRequest::CreateRepo { .. }
        | StoreRequest::UpdateRepo { .. }
        | StoreRequest::SetRepoPath { .. }
        | StoreRequest::DeleteReach(..)
        | StoreRequest::DeleteWorkspace(..)
        | StoreRequest::DeleteProject(..) => hierarchy::serve(backend, request).await?,
        // The nine catalogue requests, or-ed for the same reason the twelve above are: a guard
        // does not count towards exhaustivity in a wildcard-free `match`, so `_ if …` would be an
        // E0004 here (MOD-15 M3 plan F-12, M4 plan F-2).
        StoreRequest::Catalogue(..)
        | StoreRequest::CreateKind { .. }
        | StoreRequest::UpdateKind { .. }
        | StoreRequest::DeleteKind { .. }
        | StoreRequest::CreateGraph { .. }
        | StoreRequest::UpdateGraph { .. }
        | StoreRequest::CreatePhase { .. }
        | StoreRequest::UpdatePhase { .. }
        | StoreRequest::SetPhaseBudget { .. } => catalogue::serve(backend, request).await?,
        // The three prompt settings requests, or-ed for the same reason the twenty-one above are:
        // a guard does not count towards exhaustivity in a wildcard-free `match`, so `_ if …`
        // would be an E0004 here (MOD-15 M3 plan F-12, M5 plan F-13).
        StoreRequest::PromptSettings(..)
        | StoreRequest::SetSetting { .. }
        | StoreRequest::ClearSetting { .. } => prompt_settings::serve(backend, request).await?,
        StoreRequest::StoreState => StoreReply::StoreState {
            label: backend.label(),
            migrations_pending: None,
        },
        StoreRequest::ApplyMigrations => StoreReply::MigrationsApplied { applied: 0 },
    })
}

/// Renders a store error into the reply the asking view receives.
fn failed(request: &'static str, err: &StoreError) -> StoreReply {
    StoreReply::Failed {
        request,
        message: err.to_string(),
    }
}

/// Spawns the worker. The `Backend` moves in and no other task can reach it afterwards (plan D4).
///
/// The loop is a `select!` over three sources — the UI's requests, the connect task's
/// [`ConnEvent`]s and a reconnect ticker — and it owns every backend swap:
///
/// - [`ConnEvent::Online`] replaces `Offline { cache, .. }` with `Online { pg, cache }` (the same
///   mirror, not a re-opened one) and starts the [`Refresher`].
/// - [`ConnEvent::MigrationsPending`] holds the store **aside** instead of swapping: the schema is
///   not one this binary has finished writing, so nothing reads through it and no refresher fills
///   the mirror from it. The count is reported in the next [`StoreReply::StoreState`], which is
///   what opens the migration prompt, and [`StoreRequest::ApplyMigrations`] is what makes the swap
///   happen. (Blueprint D.2 swaps here and applies through `Backend::writable()`;
///   `PgStore::apply_migrations` takes `&mut self`, which a `&PgStore` cannot give — same
///   components, correct ownership.)
/// - [`ConnEvent::Failed`] turns the first `connecting` into `offline · 0s` and nothing else.
/// - An `Online` backend that stops answering goes the other way: a [`StoreError::Unreachable`]
///   from a served request, or the same from the [`Refresher`]'s last pass, swaps `Online` back
///   for `Offline { since: Some(now) }` over the same mirror, aborts the refresher and lets the
///   ticker start dialling again. The request that noticed still gets its
///   [`StoreReply::Failed`]: exactly one reply per request, whatever the backend did.
/// - The ticker re-dials every [`connect::RECONNECT`], and only while there is a DSN to dial, no
///   writable backend and no store waiting for an answer to the migration prompt. Its missed-tick
///   behaviour is `Delay`, not the default `Burst`: a dial that overran its slot - a ten-second
///   acquire timeout inside a thirty-second interval, or a laptop that was suspended - must not
///   be followed by a queue of catch-up dials fired back to back.
///
/// `started` is the bundle [`connect::start`] hands back; `Started::detached` is the `--demo` and
/// test form, whose event channel is never written and whose ticker is disarmed, so the worker
/// behaves exactly as it did in MOD-1 (deviation from blueprint D.2's eight-argument `spawn`: the
/// pieces are the same, passed as the struct that already carries them).
pub fn spawn(
    started: Started,
    rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
) -> tokio::task::JoinHandle<()> {
    spawn_with(started, rx, tx, AgentRuntime::production())
}

/// [`spawn`] over a chosen [`AgentRuntime`], so a test can install its own transport registry.
///
/// The runtime lives **inside** this loop: a chat needs the backend (for the writer, the box, the
/// user and the registry row) and the reply sender, and this task is the only owner of both.
pub fn spawn_with(
    started: Started,
    mut rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    mut runtime: AgentRuntime,
) -> tokio::task::JoinHandle<()> {
    let Started {
        mut backend,
        mut events,
        events_tx,
        projects,
        settings,
        reconnect,
    } = started;

    tokio::spawn(async move {
        // How many migrations are waiting, and the store that is waiting to apply them.
        let mut pending: Option<usize> = None;
        let mut held: Option<PgStore> = None;
        let mut refresher: Option<Refresher> = None;
        // The refresher's last pass outcome, for as long as there is a refresher.
        let mut health: Option<watch::Receiver<Option<StoreError>>> = None;
        // `interval_at`, not `interval`: the first tick of an `interval` completes immediately,
        // which would re-dial in the same breath as `start`'s own attempt.
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + connect::RECONNECT,
            connect::RECONNECT,
        );
        // A dial that overran its slot must not then be followed by a burst of catch-up dials.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                envelope = rx.recv() => {
                    // The UI is gone; there is nobody left to answer.
                    let Some(envelope) = envelope else { break };

                    // Publish the scope so the refresher follows what the user is looking at
                    // (plan D9). On every `Items` request, not only on a change: the send is a
                    // pointer swap and a missed update mirrors the wrong project.
                    if let StoreRequest::Items { scope, .. } = &envelope.request {
                        projects.send_replace(scope.project_ids.clone());
                    }

                    let reply = match &envelope.request {
                        StoreRequest::StoreState => StoreReply::StoreState {
                            label: backend.label(),
                            migrations_pending: pending,
                        },
                        StoreRequest::ApplyMigrations if held.is_some() => {
                            // `if held.is_some()` above, so this cannot be the `None` arm; the
                            // store has to be moved out to be applied (`&mut self`).
                            match held.take() {
                                Some(mut pg) => match pg.apply_migrations().await {
                                    Ok(()) => {
                                        let applied = pending.take().unwrap_or(0);
                                        tracing::info!(applied, "schema migrations applied");
                                        go_online(
                                            &mut backend, pg, &mut refresher, &mut health,
                                            &projects, settings,
                                        ).await;
                                        StoreReply::MigrationsApplied { applied }
                                    }
                                    Err(err) => {
                                        // Still pending, still held: `y` can be answered again.
                                        held = Some(pg);
                                        failed("apply_migrations", &err)
                                    }
                                },
                                None => StoreReply::MigrationsApplied { applied: 0 },
                            }
                        }
                        // The chat requests need this loop's own state - the live sessions - and
                        // the probe, the installs and the logins need the runtime that owns their
                        // tasks, so all of them go to the runtime before `try_serve`, like
                        // `ApplyMigrations` above. An install reads the registry over the network
                        // and streams hundreds of megabytes, and a login waits on a human in a
                        // browser, so `Served::Deferred => continue` is the whole of `R-NF-3` for
                        // both: the arm returns having spawned a task and awaited nothing longer
                        // than `box_info()` (blueprint H-9).
                        StoreRequest::PromptPreview { .. }
                        | StoreRequest::ChatStart { .. }
                        | StoreRequest::ChatSend { .. }
                        | StoreRequest::ChatAnswer { .. }
                        | StoreRequest::ChatCancel { .. }
                        | StoreRequest::ProbeAgents
                        | StoreRequest::InstallPlan { .. }
                        | StoreRequest::InstallConfirm { .. }
                        | StoreRequest::InstallCancel
                        | StoreRequest::AuthStart { .. }
                        | StoreRequest::AuthChoose { .. }
                        | StoreRequest::AuthOpen { .. }
                        | StoreRequest::AuthCancel => {
                            match runtime.serve(&backend, &tx, &envelope).await {
                                Served::Reply(reply) => reply,
                                // The session task answers this request itself, once.
                                Served::Deferred => continue,
                                Served::Start { step_id, task } => {
                                    runtime.attach(step_id, tokio::spawn(task));
                                    continue;
                                }
                            }
                        }
                        other => match try_serve(&backend, other).await {
                            Ok(reply) => reply,
                            Err(err) => {
                                // This read is what noticed the server had gone. The asking view
                                // still hears back exactly once; the next read finds the mirror.
                                if matches!(err, StoreError::Unreachable(_)) {
                                    go_offline(&mut backend, &mut refresher, &mut health, &err);
                                }
                                failed(other.name(), &err)
                            }
                        },
                    };

                    let answer = ReplyEnvelope { seq: envelope.seq, origin: envelope.origin, reply };
                    if tx.send(answer).is_err() {
                        break;
                    }
                }

                Some(event) = events.recv() => match event {
                    ConnEvent::Online(pg) => {
                        pending = None;
                        held = None;
                        go_online(
                            &mut backend, pg, &mut refresher, &mut health, &projects, settings,
                        )
                        .await;
                        tracing::info!(label = backend.label(), "store online");
                    }
                    ConnEvent::MigrationsPending(pg, n) => {
                        pending = Some(n);
                        held = Some(pg);
                        // The mirror stays the read path and the top bar stops saying
                        // `connecting`: nothing reads through a schema this binary refuses to
                        // use until the user has answered the prompt.
                        backend.gave_up();
                        tracing::warn!(pending = n, "schema has pending migrations");
                    }
                    ConnEvent::Failed(why) => {
                        backend.gave_up();
                        tracing::warn!(%why, "connect failed");
                    }
                },

                err = lost_the_server(health.clone()) => {
                    // The refresher passes every `interval`, so it usually notices first.
                    go_offline(&mut backend, &mut refresher, &mut health, &err);
                }

                _ = ticker.tick(),
                    if reconnect.is_some() && !backend.is_writable() && held.is_none() =>
                {
                    if let Some(dial) = reconnect.clone() {
                        let sender = events_tx.clone();
                        tokio::spawn(async move {
                            let _ = sender.send(dial().await).await;
                        });
                    }
                }
            }
        }

        // The UI is gone. Every live chat is cancelled and awaited **before** this task returns:
        // dropping a session task at its first await orphans the agent process it spawned
        // (`docs/ANA-4.md` §11 criterion 11).
        runtime.shutdown(crate::agent_worker::CANCEL_GRACE).await;

        if let Some(refresher) = refresher {
            refresher.abort();
        }
    })
}

/// Swaps `Offline { cache, .. }` for `Online { pg, cache }` and (re)starts the refresher.
///
/// The mirror is moved across rather than re-opened: it is the file the shell has been reading
/// from since startup, and `CacheStore` is a handle on one pool.
async fn go_online(
    backend: &mut Backend,
    pg: PgStore,
    refresher: &mut Option<Refresher>,
    health: &mut Option<watch::Receiver<Option<StoreError>>>,
    projects: &watch::Sender<Vec<ProjectId>>,
    base: RefreshSettings,
) {
    let Some(cache) = backend.cache().cloned() else {
        tracing::error!("no mirror to go online over; keeping the current backend");
        return;
    };
    // Read before the move: `this_box`, `this_user` and the two cache settings are what only a
    // connected server knows (blueprint C.13's `RefreshSettings`).
    let settings = connect::refresh_settings(&pg, base).await;
    *backend = Backend::Online { pg, cache };
    if let Some(previous) = refresher.take() {
        previous.abort();
    }
    *refresher = spawn_refresher(backend, projects, settings);
    *health = refresher.as_ref().map(Refresher::health);
}

/// The reverse of [`go_online`]: an `Online` backend that lost its server falls back on the mirror.
///
/// Aborts the refresher - there is nothing left to mirror *from*, and its failing passes would
/// otherwise report the same loss every interval - and drops the health watch, which disarms the
/// `select!` arm that reads it. The reconnect ticker re-arms itself, its guard being
/// `!backend.is_writable()`.
///
/// **The refresher and the watch go first, and unconditionally.** Only the offline *age* may not
/// be restarted by a second notice - a read and the refresher racing to report the same drop - so
/// [`Backend::went_offline`] guards the log line and nothing else. Returning early on a backend
/// that is not `Online` would leave a live watch behind whose sender keeps republishing the same
/// `Unreachable`; [`lost_the_server`] would then resolve on every poll and the `select!` would
/// spin. A health watch may never outlive the `Online` backend it was armed for.
fn go_offline(
    backend: &mut Backend,
    refresher: &mut Option<Refresher>,
    health: &mut Option<watch::Receiver<Option<StoreError>>>,
    why: &StoreError,
) {
    if let Some(previous) = refresher.take() {
        previous.abort();
    }
    *health = None;
    if !backend.went_offline() {
        return;
    }
    tracing::warn!(%why, "store unreachable; falling back to the mirror");
}

/// Resolves with the error when the refresher reports an unreachable server, and never otherwise.
///
/// Takes the receiver by value - `watch::Receiver` is `Clone` and a clone keeps the seen version -
/// so the `select!` arm's body is free to reassign the worker's own `health`.
///
/// Parks forever when there is no refresher and when its sender is gone, which leaves the arm
/// disabled rather than spinning on a closed channel. A pass that succeeded, or that failed for
/// any other reason, is not a signal: the loop keeps waiting.
async fn lost_the_server(health: Option<watch::Receiver<Option<StoreError>>>) -> StoreError {
    let Some(mut passes) = health else {
        return std::future::pending().await;
    };
    loop {
        if passes.changed().await.is_err() {
            return std::future::pending().await;
        }
        let lost = match &*passes.borrow() {
            Some(StoreError::Unreachable(why)) => Some(why.clone()),
            _ => None,
        };
        if let Some(why) = lost {
            return StoreError::Unreachable(why);
        }
    }
}

/// The refresher for an online backend, or `None` for any other (blueprint D.2).
fn spawn_refresher(
    backend: &Backend,
    projects: &watch::Sender<Vec<ProjectId>>,
    settings: RefreshSettings,
) -> Option<Refresher> {
    let pg = backend.writable()?;
    let cache = backend.cache()?;
    Some(Refresher::spawn(
        pg.pool().clone(),
        cache.clone(),
        projects.subscribe(),
        settings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::fixtures::ids;
    use htui_core::model::BoxId;
    use htui_core::store::MemStore;
    use htui_store::{CacheStore, Identity};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// A worker over a backend that never connects, plus the two channel ends the UI holds.
    fn detached(
        backend: Backend,
    ) -> (
        mpsc::UnboundedSender<RequestEnvelope>,
        mpsc::UnboundedReceiver<ReplyEnvelope>,
        tokio::task::JoinHandle<()>,
    ) {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, rep_rx) = mpsc::unbounded_channel();
        let worker = spawn(Started::detached(backend), req_rx, rep_tx);
        (req_tx, rep_rx, worker)
    }

    /// One request through a spawned worker, and its reply.
    async fn round_trip(
        tx: &mpsc::UnboundedSender<RequestEnvelope>,
        rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
        request: StoreRequest,
    ) -> StoreReply {
        tx.send(RequestEnvelope {
            seq: 0,
            origin: Origin::App,
            request,
        })
        .expect("the worker is alive");
        rx.recv().await.expect("the worker answers").reply
    }

    async fn platform_scope(backend: &Backend) -> Scope {
        let StoreReply::Workspaces(workspaces) = serve(backend, &StoreRequest::Workspaces).await
        else {
            panic!("workspaces answered with the wrong variant")
        };
        let platform = workspaces
            .iter()
            .find(|w| w.slug == "platform")
            .expect("the demo fixture holds the `platform` workspace");
        Scope::from_workspace(platform)
    }

    #[tokio::test]
    async fn serve_workspaces_lists_the_demo_hierarchy() {
        let backend = demo();
        let StoreReply::Workspaces(workspaces) = serve(&backend, &StoreRequest::Workspaces).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].name, "Graphics");
        assert_eq!(workspaces[1].name, "Platform");
        assert_eq!(workspaces[1].projects.len(), 2);
    }

    #[tokio::test]
    async fn serve_box_info_answers_this_box() {
        let backend = demo();
        let StoreReply::BoxInfo(info) = serve(&backend, &StoreRequest::BoxInfo).await else {
            panic!("wrong reply variant")
        };
        assert_eq!(
            info.expect("the demo fixture registers this box").hostname,
            "DESKTOP-HTUI"
        );
    }

    #[tokio::test]
    async fn serve_active_runs_counts_the_queued_run() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let StoreReply::ActiveRuns(count) =
            serve(&backend, &StoreRequest::ActiveRuns { scope }).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(count, 1, "RUN_2 is the fixture's only active run");
    }

    #[tokio::test]
    async fn serve_items_returns_the_scope_in_store_order() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let StoreReply::Items(items) = serve(
            &backend,
            &StoreRequest::Items {
                scope,
                filter: ItemFilter::default(),
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(items.len(), 11, "eight htui items plus three agy items");
        assert_eq!(items[0].key, "ANA-1");
    }

    #[tokio::test]
    async fn serve_item_returns_the_body() {
        let backend = demo();
        let StoreReply::Item(item) = serve(&backend, &StoreRequest::Item(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        let item = item.expect("FEAT-1 is in the fixture");
        assert_eq!(item.key, "FEAT-1");
        assert!(!item.body.is_empty());
    }

    #[tokio::test]
    async fn serve_links_walks_one_hop() {
        let backend = demo();
        let StoreReply::Links(graph) = serve(
            &backend,
            &StoreRequest::Links {
                id: ids::HTUI_FEAT_2,
                hops: 1,
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(graph.root, ids::HTUI_FEAT_2);
        assert_eq!(
            graph.nodes.len(),
            3,
            "FEAT-2, the FEAT-1 it is blocked by, and the cross-project agy FEAT-1"
        );
    }

    #[tokio::test]
    async fn serve_documents_notes_and_runs_answer_their_variants() {
        let backend = demo();
        let StoreReply::Documents(docs) =
            serve(&backend, &StoreRequest::Documents(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(docs.len(), 3, "prd v1 plus plan v1 and v2");

        let StoreReply::Notes(notes) =
            serve(&backend, &StoreRequest::Notes(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(notes.len(), 2);

        let StoreReply::Runs(runs) = serve(&backend, &StoreRequest::Runs(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].steps.len(), 4);
    }

    /// The read behind replay (MOD-2 D38): the `plan` step's eight fixture rows, in `seq` order.
    #[tokio::test]
    async fn step_events_answers_the_rows_of_a_recorded_step() {
        let backend = demo();
        let StoreReply::StepEvents { step_id, events } =
            serve(&backend, &StoreRequest::StepEvents(ids::STEP_PLAN)).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(step_id, ids::STEP_PLAN, "the reply names the step it read");
        let events = events.expect("the fixture records the plan step");
        assert_eq!(events.len(), 8);
        assert!(
            events.windows(2).all(|pair| pair[0].seq < pair[1].seq),
            "the rows arrive in `seq` order, which is replay order"
        );
    }

    /// `None` is "not on this box", and every backend answers it for a step it holds no row of -
    /// including a step that exists and recorded nothing (`STEP_PRD`).
    #[tokio::test]
    async fn a_step_this_backend_does_not_hold_answers_none() {
        let backend = demo();
        for step in [ids::STEP_PRD, StepId::new()] {
            let StoreReply::StepEvents { step_id, events } =
                serve(&backend, &StoreRequest::StepEvents(step)).await
            else {
                panic!("wrong reply variant")
            };
            assert_eq!(step_id, step);
            assert_eq!(
                events, None,
                "no row for {step} is `None`, not an empty log"
            );
        }
    }

    /// `Some(vec![])` stays in the reply type even though no backend produces it (D38).
    ///
    /// All three read "no rows" as "not cached" (`mem.rs`'s `step_log`, `pg/read.rs`'s
    /// `then_some`, `cache/read.rs`'s early return), so an empty log is currently unreachable
    /// through this request. The type keeps it representable anyway: "the step is here and
    /// recorded nothing" is a different sentence from "the step is not on this box", and the day
    /// a backend can say the first one the view must not read it as the second.
    #[tokio::test]
    async fn a_served_step_log_is_never_empty_but_the_reply_can_be() {
        let backend = demo();
        for step in [ids::STEP_PRD, ids::STEP_PLAN, ids::STEP_R2_PRD] {
            let StoreReply::StepEvents { events, .. } =
                serve(&backend, &StoreRequest::StepEvents(step)).await
            else {
                panic!("wrong reply variant")
            };
            assert!(
                events.as_ref().is_none_or(|rows| !rows.is_empty()),
                "a backend answers rows or `None`, never an empty log: {step}"
            );
        }
        assert!(
            matches!(
                StoreReply::StepEvents {
                    step_id: ids::STEP_PRD,
                    events: Some(Vec::new()),
                },
                StoreReply::StepEvents {
                    events: Some(_),
                    ..
                }
            ),
            "and the reply can still carry one, which is the distinction D38 is about"
        );
    }

    /// A store error carries [`StoreRequest::name`] into the status line, this request included.
    #[tokio::test]
    async fn a_failed_step_events_read_names_the_request() {
        let identity = Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = PgStore::lazy(
            "postgres://nobody:nothing@127.0.0.1:1/none",
            &identity,
            std::time::Duration::from_millis(250),
        )
        .expect("a lazy pool opens no socket");
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "step-events-failed", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Online {
            pg,
            cache: cache.clone(),
        };

        let reply = serve(&backend, &StoreRequest::StepEvents(ids::STEP_PLAN)).await;
        assert!(
            matches!(
                &reply,
                StoreReply::Failed {
                    request: "step_events",
                    ..
                }
            ),
            "the asking view is told which read failed: {reply:?}"
        );

        cache.close().await;
    }

    #[tokio::test]
    async fn serve_reports_an_empty_store_without_failing() {
        let backend = Backend::memory(MemStore::new());
        let StoreReply::Workspaces(workspaces) = serve(&backend, &StoreRequest::Workspaces).await
        else {
            panic!("wrong reply variant")
        };
        assert!(workspaces.is_empty());

        let StoreReply::BoxInfo(info) = serve(&backend, &StoreRequest::BoxInfo).await else {
            panic!("wrong reply variant")
        };
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn spawn_answers_with_the_request_seq_and_origin() {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn(Started::detached(demo()), req_rx, rep_tx);
        req_tx
            .send(RequestEnvelope {
                seq: 7,
                origin: Origin::App,
                request: StoreRequest::Workspaces,
            })
            .expect("the worker is alive");
        let envelope = rep_rx.recv().await.expect("the worker answers");
        assert_eq!(envelope.seq, 7);
        assert_eq!(envelope.origin, Origin::App);
        assert!(matches!(envelope.reply, StoreReply::Workspaces(_)));
        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
    }

    #[tokio::test]
    async fn store_state_reports_the_backend_label_and_no_pending_count() {
        let (tx, mut rx, worker) = detached(demo());
        let StoreReply::StoreState {
            label,
            migrations_pending,
        } = round_trip(&tx, &mut rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "memory");
        assert_eq!(
            migrations_pending, None,
            "a memory backend has no schema to migrate"
        );
        drop(tx);
        worker.await.expect("the worker stops with its channel");
    }

    #[tokio::test]
    async fn apply_migrations_without_a_held_store_applies_nothing() {
        let (tx, mut rx, worker) = detached(demo());
        let reply = round_trip(&tx, &mut rx, StoreRequest::ApplyMigrations).await;
        assert!(
            matches!(reply, StoreReply::MigrationsApplied { applied: 0 }),
            "nothing was pending, so nothing was applied: {reply:?}"
        );
        drop(tx);
        worker.await.expect("the worker stops with its channel");
    }

    #[tokio::test]
    async fn every_items_request_publishes_its_scope_to_the_refresher() {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(demo());
        // The receiver the refresher would hold; `Refresher::spawn` calls `subscribe` the same way.
        let scope_rx = started.projects.subscribe();
        started.settings.interval = std::time::Duration::from_secs(3600);
        let worker = spawn(started, req_rx, rep_tx);

        assert!(
            scope_rx.borrow().is_empty(),
            "no scope before the first read"
        );

        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
        };
        let reply = round_trip(
            &req_tx,
            &mut rep_rx,
            StoreRequest::Items {
                scope: scope.clone(),
                filter: ItemFilter::default(),
            },
        )
        .await;
        assert!(matches!(reply, StoreReply::Items(_)));
        assert_eq!(
            *scope_rx.borrow(),
            scope.project_ids,
            "the refresher follows what the user is looking at (plan D9)"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
    }

    #[tokio::test]
    async fn a_failed_connect_event_turns_connecting_into_an_age() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-test", 1)
            .await
            .expect("open a throwaway mirror");

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(Backend::Offline {
            cache: cache.clone(),
            since: None,
        });
        let events_tx = started.events_tx.clone();
        // A DSN would arm the ticker; `detached` leaves it disarmed, which is what keeps this test
        // from dialling anything.
        started.reconnect = None;
        let worker = spawn(started, req_rx, rep_tx);

        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "connecting", "no attempt has answered yet");

        events_tx
            .send(ConnEvent::Failed("no server".to_owned()))
            .await
            .expect("the worker is alive");

        // The request and the event are both ready; `select!` picks at random, so ask until the
        // event has been taken rather than assuming the first round trip loses the race.
        let mut label = String::new();
        for _ in 0..32 {
            let StoreReply::StoreState { label: seen, .. } =
                round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
            else {
                panic!("wrong reply variant")
            };
            label = seen;
            if label != "connecting" {
                break;
            }
        }
        assert!(
            label.starts_with("offline · "),
            "a failed attempt gives up on `connecting`: {label}"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
    }

    /// The mid-session `Online` → `Offline` transition, driven by an injected unreachable store.
    ///
    /// `PgStore::lazy` opens no socket, so the *first* read is what dials - at a DSN nothing
    /// listens on, which is a `StoreError::Unreachable` and therefore exactly the outcome a
    /// server that went away mid-session produces. Restarting Postgres under a live pool is not
    /// something a unit test can do; this drives the same code path without one.
    #[tokio::test]
    async fn an_unreachable_read_drops_an_online_backend_onto_the_mirror() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-swap", 1)
            .await
            .expect("open a throwaway mirror");
        let identity = Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = PgStore::lazy(
            "postgres://nobody:nothing@127.0.0.1:1/none",
            &identity,
            std::time::Duration::from_millis(250),
        )
        .expect("a lazy pool opens no socket");

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(Backend::Online {
            pg,
            cache: cache.clone(),
        });
        // `detached` leaves the ticker disarmed, so nothing re-dials behind the assertions.
        started.reconnect = None;
        let worker = spawn(started, req_rx, rep_tx);

        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "online", "nothing has asked the server yet");

        // The read that notices. It still gets its one reply.
        let reply = round_trip(&req_tx, &mut rep_rx, StoreRequest::Workspaces).await;
        assert!(
            matches!(
                &reply,
                StoreReply::Failed {
                    request: "workspaces",
                    ..
                }
            ),
            "the asking view hears back exactly once, even as the backend swaps: {reply:?}"
        );

        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert!(
            label.starts_with("offline · "),
            "an unreachable read drops the backend onto the mirror: {label}"
        );

        // And the next read answers from the mirror instead of failing again.
        let reply = round_trip(&req_tx, &mut rep_rx, StoreRequest::Workspaces).await;
        assert!(
            matches!(&reply, StoreReply::Workspaces(rows) if rows.is_empty()),
            "an unfilled mirror is empty, not broken: {reply:?}"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
    }

    /// A failure that is *not* a lost connection leaves an online backend alone.
    #[tokio::test]
    async fn lost_the_server_ignores_a_pass_that_failed_for_another_reason() {
        let (health, watcher) = watch::channel(None);
        let mut backend = demo();
        let mut refresher = None;
        let mut seen = Some(watcher.clone());

        health.send_replace(Some(StoreError::Backend("a bad query".to_owned())));
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                lost_the_server(Some(watcher.clone())),
            )
            .await
            .is_err(),
            "only StoreError::Unreachable is the signal"
        );

        health.send_replace(Some(StoreError::Unreachable("gone".to_owned())));
        let err = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            lost_the_server(Some(watcher)),
        )
        .await
        .expect("an unreachable pass resolves it");
        assert!(matches!(err, StoreError::Unreachable(_)));

        // A memory backend has no mirror, so the swap is refused.
        go_offline(&mut backend, &mut refresher, &mut seen, &err);
        assert_eq!(backend.label(), "memory");
    }

    /// The probe is named like every other request, and a build with no runtime says so rather
    /// than dropping the reply (MOD-2 D53).
    #[tokio::test]
    async fn probe_agents_is_named_and_refused_without_a_runtime() {
        assert_eq!(StoreRequest::ProbeAgents.name(), "probe_agents");
        match serve(&demo(), &StoreRequest::ProbeAgents).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, "probe_agents");
                assert_eq!(message, "no agent runtime in this build");
            }
            other => panic!("a probe with no runtime is refused, not served: {other:?}"),
        }
    }

    /// The three install requests are named like every other, and a build with no runtime says so
    /// rather than dropping the reply (MOD-20 D18).
    #[tokio::test]
    async fn the_install_requests_are_named_and_refused_without_a_runtime() {
        let plan = crate::agent_worker::tests::demo_plan(
            AgentId::new(),
            std::path::PathBuf::from("/nowhere"),
            "http://127.0.0.1:1/archive.zip".to_owned(),
        );
        for (request, name) in [
            (
                StoreRequest::InstallPlan {
                    agent_id: AgentId::new(),
                },
                "install_plan",
            ),
            (
                StoreRequest::InstallConfirm {
                    plan: Box::new(plan),
                },
                "install_confirm",
            ),
            (StoreRequest::InstallCancel, "install_cancel"),
        ] {
            assert_eq!(request.name(), name);
            match serve(&demo(), &request).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, name);
                    assert_eq!(message, "no agent runtime in this build");
                }
                other => panic!("an install with no runtime is refused, not served: {other:?}"),
            }
        }
    }

    /// `R-NF-3`, the whole reason the probe is deferred: a probe waits up to `HANDSHAKE_TIMEOUT`
    /// per agent, and the loop must serve everything else while it does.
    #[tokio::test]
    async fn the_loop_answers_other_requests_while_a_probe_is_in_flight() {
        let store = crate::agent_worker::tests::unresolvable_registry().await;
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn_with(
            Started::detached(Backend::memory(store)),
            req_rx,
            rep_tx,
            AgentRuntime::production(),
        );

        for (seq, request) in [
            (1, StoreRequest::ProbeAgents),
            (2, StoreRequest::Workspaces),
        ] {
            req_tx
                .send(RequestEnvelope {
                    seq,
                    origin: Origin::App,
                    request,
                })
                .expect("the worker is alive");
        }

        let first = rep_rx.recv().await.expect("the worker answers");
        assert_eq!(
            first.seq, 2,
            "the loop is free the instant the probe is deferred: {:?}",
            first.reply
        );
        let second = rep_rx.recv().await.expect("the probe answers itself");
        assert_eq!(second.seq, 1);
        assert!(
            matches!(second.reply, StoreReply::Agents(_)),
            "the probe answers with the registry it wrote: {:?}",
            second.reply
        );

        drop(req_tx);
        let _ = worker.await;
    }

    /// `R-NF-3` for the install, and the pin blueprint H-9 exists for: an install streams an
    /// archive for minutes, and the loop must serve everything else while it does.
    ///
    /// The fixture accepts the connection and answers nothing, so the download is provably still
    /// in flight while the assertions run. What the arm did before deferring is the whole subject:
    /// a `box_info()` and an `agents()`, exactly what a probe already awaits there — no client
    /// built, no socket opened, no registry read.
    #[tokio::test]
    async fn the_loop_answers_other_requests_while_an_install_is_in_flight() {
        // A listener that accepts and never replies. Held for the life of the test, so the
        // connection stays open rather than being refused, which is what makes the install stall
        // rather than fail.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral loopback port");
        let addr = listener.local_addr().expect("the bound address");
        let silent = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let store = MemStore::demo();
        let agent_id = AgentId::new();
        let agent = crate::agent_worker::tests::install_row(agent_id, "demo", true);
        htui_core::store::WriteStore::upsert_agent(&store, &agent)
            .await
            .expect("the row lands");
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let root = tmp.path().join("agents");
        let plan = crate::agent_worker::tests::demo_plan(
            agent_id,
            root.clone(),
            format!("http://{addr}/archive.zip"),
        );

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn_with(
            Started::detached(Backend::memory(store)),
            req_rx,
            rep_tx,
            AgentRuntime::production().with_installer(htui_agent::InstallConfig::new(
                format!("http://{addr}"),
                Some(root),
            )),
        );

        for (seq, request) in [
            (
                1,
                StoreRequest::InstallConfirm {
                    plan: Box::new(plan),
                },
            ),
            (2, StoreRequest::Workspaces),
        ] {
            req_tx
                .send(RequestEnvelope {
                    seq,
                    origin: Origin::App,
                    request,
                })
                .expect("the worker is alive");
        }

        let first = rep_rx.recv().await.expect("the worker answers");
        assert_eq!(
            first.seq, 2,
            "the loop is free the instant the install is deferred: {:?}",
            first.reply
        );
        assert!(
            matches!(first.reply, StoreReply::Workspaces(_)),
            "and it is a real answer, not a refusal: {:?}",
            first.reply
        );

        drop(req_tx);
        let _ = worker.await;
        silent.abort();
    }

    /// The four login requests are named like every other, and a build with no runtime says so
    /// rather than dropping the reply (MOD-21 D18).
    #[test]
    fn name_arms_are_stable() {
        assert_eq!(
            StoreRequest::AuthStart {
                agent_id: AgentId::new()
            }
            .name(),
            "auth_start"
        );
        assert_eq!(
            StoreRequest::AuthChoose {
                choice: AuthChoice::Logout
            }
            .name(),
            "auth_choose"
        );
        assert_eq!(
            StoreRequest::AuthOpen {
                url: "https://h.invalid/login".to_owned()
            }
            .name(),
            "auth_open"
        );
        assert_eq!(StoreRequest::AuthCancel.name(), "auth_cancel");
    }

    /// A shell with no agent runtime answers each of the four by name, exactly once, rather than
    /// serving a login from the loop or dropping the reply (MOD-21 D18).
    #[tokio::test]
    async fn try_serve_without_a_runtime_refuses_all_four_by_name() {
        for (request, name) in [
            (
                StoreRequest::AuthStart {
                    agent_id: AgentId::new(),
                },
                "auth_start",
            ),
            (
                StoreRequest::AuthChoose {
                    choice: AuthChoice::Method("m-one".to_owned()),
                },
                "auth_choose",
            ),
            (
                StoreRequest::AuthOpen {
                    url: "https://h.invalid/login".to_owned(),
                },
                "auth_open",
            ),
            (StoreRequest::AuthCancel, "auth_cancel"),
        ] {
            match serve(&demo(), &request).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, name);
                    assert_eq!(message, "no agent runtime in this build");
                }
                other => panic!("a login with no runtime is refused, not served: {other:?}"),
            }
        }
    }

    /// `R-NF-3` for a login, and the reason it is deferred at all: a flow waits on a **human** for
    /// as long as the browser round trip takes, and the loop must serve everything else while it
    /// does.
    ///
    /// The fixture never answers `authenticate`, so the flow is provably still running while the
    /// assertions hold. What the arm did before deferring is the whole subject: a `box_info()` and
    /// an `agents()`, exactly what a probe and an install already await there — no spawn on the
    /// loop, no writer left behind.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_loop_answers_other_requests_while_a_login_waits_for_a_human() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let (store, agent_id) = crate::agent_worker::tests::auth::login_store(
            tmp.path(),
            &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")],
        )
        .await;

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn_with(
            Started::detached(Backend::memory(store)),
            req_rx,
            rep_tx,
            crate::agent_worker::tests::auth::login_runtime(tmp.path()),
        );

        for (seq, request) in [
            (1, StoreRequest::AuthStart { agent_id }),
            (2, StoreRequest::Workspaces),
        ] {
            req_tx
                .send(RequestEnvelope {
                    seq,
                    origin: Origin::App,
                    request,
                })
                .expect("the worker is alive");
        }

        let first = rep_rx.recv().await.expect("the worker answers");
        assert_eq!(
            first.seq, 2,
            "the loop is free the instant the login is deferred: {:?}",
            first.reply
        );
        assert!(
            matches!(first.reply, StoreReply::Workspaces(_)),
            "and it is a real answer, not a refusal: {:?}",
            first.reply
        );

        req_tx
            .send(RequestEnvelope {
                seq: 3,
                origin: Origin::App,
                request: StoreRequest::AuthCancel,
            })
            .expect("the worker is alive");

        drop(req_tx);
        let _ = worker.await;
    }

    /// `go_offline` disarms the `lost_the_server` arm whatever the backend was.
    ///
    /// A health watch that outlived its `Online` backend would keep republishing the same
    /// `Unreachable`, and the arm would resolve on every poll instead of parking - a spin. The
    /// watch therefore goes before the `went_offline` guard, not after it.
    #[tokio::test]
    async fn go_offline_drops_the_health_watch_on_a_backend_that_is_not_online() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-go-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let err = StoreError::Unreachable("gone".to_owned());

        for mut backend in [
            demo(),
            Backend::Offline {
                cache: cache.clone(),
                since: None,
            },
        ] {
            let was = backend.label();
            // The sender stays alive for the whole round, so a watch left behind would be a live
            // one: `seen.is_none()` is the assertion, not a closed-channel accident.
            let (_health, watcher) = watch::channel(Some(err.clone()));
            let mut refresher = None;
            let mut seen = Some(watcher);

            go_offline(&mut backend, &mut refresher, &mut seen, &err);

            assert_eq!(
                backend.label(),
                was,
                "a backend that is not Online stays where it was"
            );
            assert!(
                seen.is_none(),
                "the watch is dropped whatever the backend was: {was}"
            );
        }

        cache.close().await;
    }
}
