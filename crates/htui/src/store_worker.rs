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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use htui_agent::auth::loopback::{ListenerReply, RedirectUrl};
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::driver::{AgentSessionRef, DriverCaps, PermissionAnswer, PermissionRequestId};
use htui_agent::event::{DriverEnvelope, StopReason};
use htui_agent::probe::ProbeStatus;
use htui_core::model::{
    AgentId, AgentSummary, AnswerOutcome, BindingChange, BoxEdit, BoxId, BoxInfo, CitationKind,
    Document, DocumentHead, DocumentId, EditReason, Item, ItemFilter, ItemId, ItemKindId,
    ItemKindPatch, ItemSpec, ItemSummary, LinkGraph, NewPersona, Note, PermissionId, Persona,
    PersonaId, PersonaPatch, PhaseId, PhasePatch, Priority, ProjectId, ProjectPatch, RelayView,
    RepoId, RepoPatch, RequirementAreaId, RequirementId, RunSummary, Scope, SessionEvent,
    SkillBindingKey, SkillId, SkillPatch, SpecChanges, StepGraphId, StepGraphPatch, StepId,
    ToolCallCount, WaitingPermission, WorkspaceId, WorkspacePatch, WorkspaceSummary,
};
use htui_core::prompt::SettingKey;
use htui_core::root_path::{DirListing, RootRefusal, list_dirs};
use htui_core::secret::SecretSource;
use htui_core::store::{
    DeleteReach, DeleteTarget, ReadStore, Result as StoreResult, SettingRung, StoreError,
    WriteStore,
};
use htui_store::cache::refresh::{RefreshSettings, Refresher};
use htui_store::vector::SearchQuery;
use htui_store::{
    Backend, ConnEvent, DATABASE_UNREACHABLE, Dsn, PgStore, Started, Writer, connect,
};
use htui_worker::WaitingView;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use crate::agent_settings::{self, AgentDraft, AgentWrite};
use crate::agent_worker::{AgentRuntime, Served};
use crate::box_settings::{self, BoxesSnapshot};
use crate::catalogue::{self, CatalogueSnapshot};
use crate::concepts_worker::{self, ConceptsReply, ConceptsRuntime, ConceptsServed};
use crate::connection::{self, Attempt, AttemptOutcome, ConnectionSnapshot};
use crate::hand_written::{self, DocumentFormContext, HandText};
use crate::hierarchy::{self, HierarchySnapshot, InferReport, MirrorAfterDelete};
use crate::item_writes::{self, ItemDivergence, ItemFormContext, ItemWrite};
use crate::persona_import::PersonaImports;
use crate::persona_settings::{self, PersonaWrite};
use crate::prompt_settings::{self, SettingsSnapshot};
use crate::requirements::{
    self, ItemCitations, RequirementDetail, RequirementText, RequirementWrite, RequirementsSnapshot,
};
use crate::run_worker::{LiveChats, RunRuntime, RunServed, TuiRuns as _};
use crate::skill_import::SkillImports;
use crate::skills::{self, SkillWrite, SkillsSnapshot, StaleWhat};
use crate::templates::{self, TemplateBody, TemplatesSnapshot};
use crate::ui::overlay::OverlayId;
use crate::ui::tabs::TabId;

/// Monotonic request counter, minted by `App::dispatch` (blueprint C.2).
pub type Seq = u64;

/// The address of a reply nobody asked for: `App::dispatch` counts up from 0 and never reaches
/// it, so the freshness gate drops it after `observe_reply` (MOD-7 D13).
pub const UNSOLICITED: Seq = Seq::MAX;

/// [`StoreRequest::name`] of the preview, named once so the deferred task's `Failed` replies carry
/// the same string the request does (`preview::run_preview` cannot call `name()` — it no longer has
/// the request).
pub const PROMPT_PREVIEW: &str = "prompt_preview";

/// [`StoreRequest::name`] of [`StoreRequest::ListDir`] (MOD-49 plan P3, blueprint D2). Not in
/// `hierarchy::REQUEST_NAMES`: a listing reads this box's filesystem and no store, so it is not one
/// of the thirteen offline refusals, and the Hierarchy section matches its `Failed` by this name.
pub const LIST_DIR: &str = "list_dir";

/// The most entries one [`StoreReply::DirListing`] carries; the rest is its `more` (MOD-49 P4).
pub const LIST_CAP: usize = 1000;

/// How long one listing may take before it is answered as refused (MOD-49 review M-1): a hung
/// NFS, sshfs or autofs mount never returns from `stat` or `read_dir`, and the picker must hear
/// back rather than read `reading…` forever.
pub const LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What [`serve`] answers an orchestrator command with when no `RunRuntime` serves it (blueprint
/// D183): the test harness without one.
pub const NO_RUN_RUNTIME: &str = "no run runtime in this build";

/// What a promotion is answered with, after its engine writes, by a test harness that has a run
/// runtime and no chat runtime to bind the promoted step to (blueprint D181). The store loop always
/// holds both.
pub const PROMOTION_NEEDS_CHAT: &str = "promotion needs the chat runtime";

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
    /// This box's row for the top bar.
    BoxInfo,
    /// MOD-69 plan D6: the waiting-on-you list and the top bar's two counts, re-read on the
    /// shell's refresh tick and on a scope change.
    Waiting {
        /// The workspace scope to read.
        scope: Scope,
    },
    /// The Backlog list.
    Items {
        /// The workspace scope to read.
        scope: Scope,
        /// Conjunctive filter; `ItemFilter::default()` means "everything in scope".
        filter: ItemFilter,
        /// MOD-13 D2: ANA-9 §7.4 for *this* box. The worker adds `ready: Some(true)` and keeps the
        /// rows whose `required_tags ⊆ probed_tags ∪ declared_tags` of `Backend::box_info()`;
        /// `None` (unregistered box) = no tags, as §7.4's `LEFT JOIN … COALESCE`.
        ready_here: bool,
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
    /// Stream a live chat's frames to this request's address from now on (MOD-4 plan D165,
    /// blueprint D185).
    ///
    /// The Chat tab sends it when a promotion answers (T7). A promotion's address is an `Orch`
    /// request's, and any later `Orch` request from the tab — a second promotion the run runtime
    /// refuses included — supersedes it in the shell's staleness index (`App::is_fresh`), which
    /// would drop every frame after it. Nothing else from the tab supersedes this one while the
    /// chat is live. Answered by the stream itself: no reply of its own, and none for a chat that
    /// is already over. Served before the promotion's bind has opened the chat, it is kept for
    /// the bind (T7), so the chat's first frame already goes to this request's address.
    ChatFollow {
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
    /// Probe this box, then its agents, now (MOD-7 D11): the registration probe's path without the
    /// "needs a probe" decision. Served by the agent runtime's own task; answered once with
    /// [`StoreReply::BoxProbed`], or refused with [`StoreReply::Failed`] before anything spawns
    /// (offline: `REGISTRY_ON_SERVER_ONLY`; a claim held). The `Settings > Boxes` section binds it
    /// to `p`, on this box only (MOD-7 milestone 2, D49).
    ProbeBox,
    /// Every box of this user, this box marked, and the effective probe spec (MOD-7 milestone 2,
    /// D45): answered with [`StoreReply::Boxes`]. Served in the loop, like the catalogue reads.
    Boxes,
    /// A declared-tags or quirks edit, compare-and-set on `box.edit_version` (MOD-7 D41, D46).
    /// Answered with [`StoreReply::Boxes`] when it applied and [`StoreReply::BoxesStale`] when the
    /// token was spent or the box is gone; a refused tag list is [`StoreReply::Failed`].
    EditBox {
        /// The box, as a snapshot listed it.
        box_id: BoxId,
        /// The `edit_version` the editor opened on.
        expected: i32,
        /// Only the edited field is `Some`.
        edit: BoxEdit,
    },
    /// Set, replace or clear the `app_setting.box_probe_spec` overlay (MOD-51 D2–D4), a
    /// compare-and-set on the row's `updated_at`. `overlay: Some` is checked by
    /// `htui_agent::box_probe::spec::check` before anything is written, and a refusal is
    /// [`StoreReply::Failed`] carrying the probe's own fault sentence; `overlay: None` clears the
    /// row and needs `expected`. Answered with [`StoreReply::Boxes`] when it applied and
    /// [`StoreReply::BoxesStale`] when the token was spent or the row is gone. The overlay is a
    /// tool list, not a secret, so `Debug` may print it (the rule above).
    SetProbeSpec {
        /// The overlay to store, or `None` to clear the row.
        overlay: Option<Value>,
        /// The `updated_at` of the row the editor opened on (`SpecView.stored`); `None` expects
        /// no row.
        expected: Option<DateTime<Utc>>,
    },
    /// Create one `agent` row from the Settings form (MOD-23 D239). The worker mints the id and
    /// the clock, builds `launch` as `{command, args, env: {}}`, and copies `settings` from
    /// `settings_from` (OQ-5) or starts from `{}`. Served in the loop by
    /// [`agent_settings::serve`]; answered with [`StoreReply::AgentWritten`] (`Created`), or with
    /// [`StoreReply::Failed`] for a refused name or field or a taken name. Offline:
    /// `REGISTRY_ON_SERVER_ONLY`.
    CreateAgent {
        /// `agent.name`, as typed; checked again by the worker (D234).
        name: String,
        /// The parsed form; checked again by the worker (D247).
        draft: AgentDraft,
        /// The row whose `settings` document the new row starts from; `None`, or a row that is
        /// gone by the time the worker reads, starts from `{}`.
        settings_from: Option<AgentId>,
    },
    /// Edit one `agent` row as a compare-and-set on `updated_at` (MOD-40 D5, MOD-23 D239). `name`,
    /// `settings` and every `launch` key but `command` and `args` are kept. Answered with
    /// [`StoreReply::AgentWritten`] (`Edited`, `Stale` or `Gone`), or with [`StoreReply::Failed`]
    /// for a refused field. Offline: `REGISTRY_ON_SERVER_ONLY`.
    EditAgent {
        /// The row.
        agent_id: AgentId,
        /// The `updated_at` of the row as a registry read answered it, never a built one (MOD-40
        /// blueprint F-17).
        expected: DateTime<Utc>,
        /// The parsed form; checked again by the worker (D247).
        draft: AgentDraft,
    },
    /// Switch one agent on or off on **this** box (MOD-23 D242): `agent_box.user_off`, through
    /// `WriteStore::set_agent_box_enabled`. Answered with [`StoreReply::AgentWritten`]
    /// (`Switched`), or with [`StoreReply::Failed`] before this box is registered. Offline:
    /// `REGISTRY_ON_SERVER_ONLY`.
    SetAgentOnBox {
        /// The agent.
        agent_id: AgentId,
        /// `false` switches it off here; `true` returns it to the probe's verdict.
        enabled: bool,
    },
    /// Set this box's manual tool paths for one agent (MOD-66 D7): `agent_box.probe.manual`, by a
    /// probe of the row over them. Served by the **agent runtime's own task**, because the probe
    /// may spawn tier 2. Answered exactly once, with [`StoreReply::AgentWritten`] (`ToolPaths`),
    /// or with [`StoreReply::Failed`] before anything is spawned. The write always lands: no
    /// manual-row rule can swallow it (D9). An empty map clears every manual path. Offline:
    /// `REGISTRY_ON_SERVER_ONLY`.
    SetToolPaths {
        /// The agent.
        agent_id: AgentId,
        /// `${tool}` name → an absolute path to a file on this box, for names the row's
        /// `discovery.tools` declares. Paths, not secrets (`R-SEC-2`).
        paths: BTreeMap<String, String>,
    },
    /// The persona registry by name (MOD-26 M2 D21). Served by [`persona_settings::serve`]
    /// through the writer: personas are not mirrored, so offline it is refused with
    /// `DATABASE_UNREACHABLE`. Answered with [`StoreReply::Personas`].
    Personas,
    /// Create one persona from the Settings form (D21); the section mints the id (B-13).
    /// Answered with [`StoreReply::PersonaWritten`] (`Created`), or with [`StoreReply::Failed`]
    /// carrying the store's sentence byte for byte (I-8, B-11).
    CreatePersona {
        /// The row to insert; checked again by the store.
        new: NewPersona,
    },
    /// Edit one persona under compare-and-set on `updated_at` (D21): only the fields the editor
    /// changed are `Some`. Answered with [`StoreReply::PersonaWritten`] (`Updated`, `Stale` or
    /// `Gone`), or `Failed` with the store's sentence.
    UpdatePersona {
        /// The row.
        id: PersonaId,
        /// `updated_at` as a registry reply answered it, never a built one (MOD-40 F-17).
        expected: DateTime<Utc>,
        /// The changed fields.
        patch: PersonaPatch,
    },
    /// Delete one persona no phase binds (D14, D21). Answered with
    /// [`StoreReply::PersonaWritten`] (`Deleted` or `Gone`), or `Failed` carrying
    /// `persona_is_bound`'s sentence.
    DeletePersona {
        /// The row.
        id: PersonaId,
    },
    /// Import one frontmatter `.md` file or the depth-0 `*.md` of a directory (D20, OQ-9). The
    /// **worker** reads the filesystem (`R-NF-3`, I-11). Answered with
    /// [`StoreReply::PersonaImports`].
    ImportPersonas {
        /// The path exactly as typed (one line, never split).
        path: String,
    },
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
    /// Relay the address the browser could not open to the running login's own loopback
    /// listener (MOD-22 D263, D269).
    ///
    /// Served **inside the live flow's task**, never on the loop: one plain `GET` to the port the
    /// flow's link advertised, while the flow keeps serving its wire, its stderr and `x`.
    /// Answered exactly once at its own `seq`: [`AuthFrame::Delivered`] with what the listener
    /// said, or [`StoreReply::Failed`] naming the rule the paste broke or why nothing answered.
    /// The flow then ends through MOD-21's own path. The address carries an authorization code,
    /// so it travels as a [`RedirectUrl`], whose `Debug` is `RedirectUrl(<redacted>)`; the rule it
    /// is held to is the credential rule of `htui_agent::auth::loopback`.
    AuthDeliver {
        /// The pasted address.
        url: RedirectUrl,
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

    // The thirteen hierarchy requests of `Settings > Hierarchy` (MOD-15 milestone 3, D5/D6). Each
    // carries **only what the user typed**: `created_by` and the box a path belongs to are filled
    // in by [`crate::hierarchy::serve`] from `Backend` identity, so no view holds a `UserId` or a
    // `BoxId`. All thirteen are served through one or-ed arm of [`try_serve`].
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
    /// Infer this box's checkout paths for every repo of a workspace that has none here (MOD-7
    /// milestone 4, PRD D5, plan D114): the workspace root on this box is walked, and each repo
    /// with exactly one matching checkout gets a canonical row, inserted only where none exists,
    /// so a manual path is never replaced. Answers [`StoreReply::RepoPathsInferred`].
    InferRepoPaths(WorkspaceId),

    // MOD-49 (plan P3): not one of the thirteen above — no store, no box row, no writer.
    /// The directories in `path` on **this** box, for the path picker (MOD-49 P1, P3, P4). A read:
    /// the section that sends it marks nothing busy (P11). Answers [`StoreReply::DirListing`], or
    /// `Failed { request: "list_dir" }` carrying the guard's sentence about the path as typed.
    ListDir {
        /// The directory to list, as the picker navigated to it (never canonicalised, P5).
        path: String,
        /// Whether `.`-prefixed entries are listed (blueprint D1).
        show_hidden: bool,
    },

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
    /// CAS on `step_graph_phase.updated_at` (M1 D3) over [`PhasePatch`]'s six columns.
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
    /// Every scope project's prompt templates, every version (MOD-9 D5).
    Templates(Scope),
    /// Append version `expected + 1` of `(project, name)` iff `expected` is its head (`None`: a new
    /// name). The worker fills `created_by` (`this_user`); the view never holds a `UserId`.
    /// Answered with [`StoreReply::TemplateSaved`] when it applied (MOD-59),
    /// [`StoreReply::TemplatesStale`] when the token was spent.
    SaveTemplate {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The project the template belongs to.
        project: ProjectId,
        /// The template's name; its role is `TemplateRole::of_name(name)`.
        name: String,
        /// The whole new body, which the store refuses if `parse` does. Its `Debug` is its
        /// length.
        body: TemplateBody,
        /// The head version the editor opened on: the CAS token, `None` for a name with no row.
        expected: Option<i32>,
    },
    /// The Skills view's library, global attachments, and each scope project's attachments,
    /// graphs and repo names (MOD-9 D81). Answered with [`StoreReply::Skills`].
    Skills(Scope),
    /// `create_skill`: the row and its version 1 together (D75, D77). The worker mints the id and
    /// fills `created_by`; the view never holds a `UserId`. Answered with
    /// [`StoreReply::SkillWritten`] when it applied (MOD-59); a refusal (name, blank body, taken
    /// name) is [`StoreReply::Failed`].
    CreateSkill {
        /// The scope the reply re-reads.
        scope: Scope,
        /// `skill.name`.
        name: String,
        /// `skill.description`.
        description: String,
        /// Version 1's body. Its `Debug` is its length.
        body: TemplateBody,
    },
    /// `update_skill` under CAS on `skill.updated_at` (D76): answered with
    /// [`StoreReply::SkillWritten`] when it applied (MOD-59) and [`StoreReply::SkillsStale`] when
    /// the token was spent or the skill is gone.
    EditSkill {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which skill.
        skill: SkillId,
        /// `skill.updated_at` the form opened on: the CAS token.
        expected: DateTime<Utc>,
        /// What changed.
        patch: SkillPatch,
    },
    /// `add_skill_version`: append version `expected + 1` iff `expected` is the head (D75, D89).
    /// The worker fills `created_by`. Answered with [`StoreReply::SkillWritten`] when it applied
    /// (MOD-59), [`StoreReply::SkillsStale`] when the head moved or the skill is gone.
    SaveSkillVersion {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which skill.
        skill: SkillId,
        /// The head version the editor opened on: the CAS token (`0`: the skill has none).
        expected: i32,
        /// The new body. Its `Debug` is its length.
        body: TemplateBody,
    },
    /// `set_skill_binding`: attach, change or detach the one row at `key` (D78, D90). Answered
    /// with [`StoreReply::SkillWritten`] when it applied (MOD-59), [`StoreReply::SkillsStale`]
    /// when the row at the key is not the one the form opened on, or its skill, project or phase
    /// is gone.
    SetSkillBinding {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The attachment's key.
        key: SkillBindingKey,
        /// The row's `updated_at` the form opened on; `None` for "no row at the key".
        expected: Option<DateTime<Utc>>,
        /// Attach or detach.
        change: BindingChange,
    },
    /// Imports `SKILL.md` and rules files from the paths named, each a file or a directory (MOD-9
    /// milestone 4, import plan D97). The **worker** reads the filesystem: `R-NF-3` keeps the walk
    /// off the UI task, and the view types a path and nothing else. Writes `skill` and
    /// `skill_version` rows only, never an attachment. Answered with [`StoreReply::SkillImports`].
    ImportSkills {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The paths as the maintainer typed them, in that order. A path is never split: one line
        /// of the import form is one path, so a path containing a space is one path.
        paths: Vec<String>,
    },
    /// The connection as the Settings > Connection section shows it (MOD-15 M6, D4): backend
    /// label, whether a DSN is stored (never the DSN), the mirror's `cache_meta`, the last dial.
    ConnectionInfo,
    /// Store `dsn` in the keyring, open its mirror and start dialling it, without a restart
    /// (D11). Carries the redacting newtype, never a `String` (M5 review L-9).
    SetDsn(Dsn),
    /// Remove the keyring entry and stop dialling. The live connection, if any, is kept until
    /// quit (D13).
    ClearDsn,
    /// `CacheStore::rebuild()` on the current mirror: the mirrored tables
    /// (`htui_store::cache::MIRRORED_TABLES`) and the cursor go, the file and its `cache_meta`
    /// stay (D14).
    RebuildCache,
    /// Request to fetch Qdrant connection info.
    QdrantInfo,
    /// Request to set the Qdrant connection string.
    SetQdrantUrl(String),
    /// Request to set the Qdrant API key.
    SetQdrantApiKey(zeroize::Zeroizing<String>),
    /// Request to clear the Qdrant connection string.
    ClearQdrantSettings,
    /// MOD-64 D231: a concepts search, served by `concepts_worker::ConceptsRuntime` on a task of its
    /// own (`R-NF-3`). Answered with [`StoreReply::Concepts`], never `Failed` (D232).
    SearchConcepts(SearchQuery),
    /// MOD-64 D237: `Indexer::sync` over `scope`'s projects, on the runtime's task. Answered with
    /// [`StoreReply::Concepts`].
    IndexConcepts {
        /// The workspace and the projects to index.
        scope: Scope,
    },
    /// Plan D154: one orchestrator command or read, served by `run_worker::RunRuntime` on its own
    /// task (`R-NF-3`). Answered once at this `seq`, possibly hours later (R-41).
    Orch(crate::run_worker::OrchRequest),
    /// Plan D172: a subscription. Answered at once with `RunFrame { kind: Subscribed }`, then with a
    /// frame at **this** `seq` per change to the item's runs (blueprint §0a point 3).
    RunStream {
        /// The item whose runs the subscriber shows.
        item: ItemId,
    },
    /// Plan D173: one document with its body.
    Document(DocumentId),
    /// Blueprint D182: every action's enabling verdict for the item, from the engine's own guards.
    RunActions(ItemId),
    /// MOD-42 plan D14: the item's pending permission requests and pending cancels. Served from
    /// the writer; offline, an **empty** view — never `Failed` or `Unreachable` (OQ-4).
    RelayView {
        /// The item the Runs pane shows.
        item: ItemId,
    },
    /// MOD-72 plan D4: per-step tool-call counts of the item's runs, for the Runs flow's chips.
    /// An ordinary read, so the mirror answers it offline (plan D2).
    ToolCalls {
        /// The item the Runs pane shows.
        item: ItemId,
    },
    /// MOD-42 plan D14: one answer to a pending request (D3). Refused offline with
    /// `DATABASE_UNREACHABLE`.
    AnswerPermission {
        /// The request.
        permission: PermissionId,
        /// The chosen option's id.
        option_id: String,
    },
    /// MOD-39 plan P2: the scope's specs, areas and requirements, answered with
    /// [`StoreReply::Requirements`].
    Requirements(Scope),
    /// One requirement with its coverage and revisions (plan P7).
    RequirementDetail(RequirementId),
    /// One item's citations and what it could cite (plan P8), answered with
    /// [`StoreReply::ItemCitations`].
    ItemRequirements(ItemId),
    /// `create_requirement_area`, gated (PRD D1). The worker picks `position` (last + 1). Answered
    /// with [`StoreReply::RequirementWritten`] when it applied (MOD-59).
    CreateRequirementArea {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The project the area belongs to.
        project: ProjectId,
        /// The area code as typed; the worker trims it.
        code: String,
        /// The title as typed; the worker trims it.
        title: String,
    },
    /// `mint_requirement`, gated. The worker mints the id and fills `created_by` and `box_id`.
    /// Answered with [`StoreReply::RequirementWritten`] when it applied (MOD-59).
    MintRequirement {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The area's project: the one the gate checks (MOD-39 blueprint F-7).
        project: ProjectId,
        /// The area the requirement is minted in; must be one of `project`'s.
        area: RequirementAreaId,
        /// The body.
        body: RequirementText,
        /// The rationale.
        rationale: RequirementText,
        /// The priority.
        priority: Priority,
    },
    /// `amend_requirement` at `expected_version`, gated; `deciding` is the typed item key (plan
    /// P6). Answered with [`StoreReply::RequirementWritten`] when it applied (MOD-59),
    /// [`StoreReply::RequirementsStale`] on `Diverged`.
    AmendRequirement {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The requirement amended.
        id: RequirementId,
        /// The version the form opened on: the compare-and-set token.
        expected_version: i32,
        /// The new body.
        body: RequirementText,
        /// The new rationale.
        rationale: RequirementText,
        /// The new priority.
        priority: Priority,
        /// The deciding item's key, as typed.
        deciding: String,
    },
    /// `withdraw_requirement` at `expected_version`, gated; `deciding` as above. Answered with
    /// [`StoreReply::RequirementWritten`] when it applied (MOD-59),
    /// [`StoreReply::RequirementsStale`] on `Diverged`.
    WithdrawRequirement {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The requirement withdrawn.
        id: RequirementId,
        /// The version the form opened on: the compare-and-set token.
        expected_version: i32,
        /// The deciding item's key, as typed.
        deciding: String,
    },
    /// `cite(item, requirement, kind, None)`: a human citation. Not gated (plan P5); `addresses`
    /// or `reserves` of a requirement of the item's project only (PRD D4).
    CiteRequirement {
        /// The citing item.
        item: ItemId,
        /// The cited requirement.
        requirement: RequirementId,
        /// The citation kind.
        kind: CitationKind,
    },
    /// `uncite`; `amends`/`withdraws` refused (plan P9).
    UnciteRequirement {
        /// The citing item.
        item: ItemId,
        /// The cited requirement.
        requirement: RequirementId,
        /// The citation kind.
        kind: CitationKind,
    },
    /// `reconfirm`: re-stamp at the current version.
    ReconfirmCitation {
        /// The citing item.
        item: ItemId,
        /// The cited requirement.
        requirement: RequirementId,
        /// The citation kind.
        kind: CitationKind,
    },
    /// MOD-13 milestone 2 D3: the read that opens the Backlog's item form; answered with
    /// [`StoreReply::ItemForm`]. Refused offline (D2). For an edit, `project` must be the item's.
    ItemForm {
        /// The project the form writes in: its kinds, graphs and repos are read.
        project: ProjectId,
        /// The item an edit form opens on; `None` for a new item.
        item: Option<ItemId>,
    },
    /// §7.1 through the shared validator (D4); the worker mints the id and fills `created_by` and
    /// `box_id`. Answered with [`StoreReply::ItemWritten`].
    MintItem {
        /// The project the item is minted in.
        project: ProjectId,
        /// The spec columns; `body` and `touched_paths` print as lengths (E6).
        spec: ItemSpec,
    },
    /// §7.2 at `expected_version`, only the changed columns (D5), with its reason (milestone 3
    /// D6). Answered with [`StoreReply::ItemWritten`] or [`StoreReply::ItemDiverged`].
    EditItem {
        /// The item edited.
        id: ItemId,
        /// The compare-and-set token: the `version` the form's text came from (D6).
        expected_version: i32,
        /// The changed columns; `body` and `touched_paths` print as lengths (E6).
        changes: SpecChanges,
        /// The revision's reason: `Edited` until the form is rebased on a divergence's head, then
        /// `DivergenceResolution` (milestone 3 D6).
        reason: EditReason,
    },
    /// MOD-13 milestone 5 D1-D3: the read that opens the Notes compose area; answered with
    /// [`StoreReply::NoteForm`]. Refused offline before anything is read (D2).
    NoteForm {
        /// The item the note is for.
        item: ItemId,
    },
    /// A hand-written note (`R-ENT-11`); the worker fills id, author, box and clock. Answered with
    /// [`StoreReply::NoteAdded`].
    AddNote {
        /// The item.
        item: ItemId,
        /// The text; prints as its length (D1).
        body: HandText,
    },
    /// The read that opens the Docs form: `kind` is `None` for `a`, the row's kind for `v`, whose
    /// version the next step reads (`resolve_inputs`, MOD-73 review M1) prefills the form (D9).
    /// Answered with [`StoreReply::DocumentForm`].
    DocumentForm {
        /// The item the document is for.
        item: ItemId,
        /// The kind whose next-step input prefills the form (`v`); `None` for a new document (`a`).
        kind: Option<String>,
    },
    /// A hand-written document at the store's next version of `kind` (`R-ENT-12`, D5: never a
    /// compare-and-set). Answered with [`StoreReply::DocumentWritten`].
    WriteDocument {
        /// The item.
        item: ItemId,
        /// The kind, as typed; the worker trims it (D4).
        kind: String,
        /// The title, as typed; plain in `Debug` (D1).
        title: String,
        /// The text; prints as its length (D1).
        body: HandText,
    },
}

impl StoreRequest {
    /// Stable name of the request, used in [`StoreReply::Failed`] and in logs.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Workspaces => "workspaces",
            Self::BoxInfo => "box_info",
            Self::Waiting { .. } => "waiting",
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
            Self::ChatFollow { .. } => "chat_follow",
            Self::ProbeAgents => "probe_agents",
            Self::ProbeBox => "probe_box",
            // The three of `box_settings::REQUEST_NAMES`, in that order (MOD-7 milestone 2, D46;
            // MOD-51 D4).
            Self::Boxes => "boxes",
            Self::EditBox { .. } => "edit_box",
            Self::SetProbeSpec { .. } => "set_probe_spec",
            // The three of `agent_settings::REQUEST_NAMES`, in that order (MOD-23 D241).
            Self::CreateAgent { .. } => "create_agent",
            Self::EditAgent { .. } => "edit_agent",
            Self::SetAgentOnBox { .. } => "set_agent_on_box",
            // Served by the agent runtime, not `agent_settings::serve` (MOD-66 D7).
            Self::SetToolPaths { .. } => agent_settings::SET_TOOL_PATHS,
            // The five of `persona_settings::REQUEST_NAMES`, in that order (MOD-26 M2 D21).
            Self::Personas => "personas",
            Self::CreatePersona { .. } => "create_persona",
            Self::UpdatePersona { .. } => "update_persona",
            Self::DeletePersona { .. } => "delete_persona",
            Self::ImportPersonas { .. } => "import_personas",
            Self::InstallPlan { .. } => "install_plan",
            Self::InstallConfirm { .. } => "install_confirm",
            Self::InstallCancel => "install_cancel",
            Self::AuthStart { .. } => "auth_start",
            Self::AuthChoose { .. } => "auth_choose",
            Self::AuthOpen { .. } => "auth_open",
            Self::AuthDeliver { .. } => "auth_deliver",
            Self::AuthCancel => "auth_cancel",
            Self::StoreState => "store_state",
            Self::ApplyMigrations => "apply_migrations",
            // The thirteen of `hierarchy::REQUEST_NAMES`, in that order. String literals, because
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
            Self::InferRepoPaths(..) => "infer_repo_paths",
            // MOD-49: outside `hierarchy::REQUEST_NAMES` on purpose (blueprint D2).
            Self::ListDir { .. } => LIST_DIR,
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
            // The two of `templates::REQUEST_NAMES`, in that order (MOD-9 D5).
            Self::Templates(..) => "templates",
            Self::SaveTemplate { .. } => "save_template",
            // The six of `skills::REQUEST_NAMES`, in that order (MOD-9 D81, import plan D97).
            Self::Skills(..) => "skills",
            Self::CreateSkill { .. } => "create_skill",
            Self::EditSkill { .. } => "edit_skill",
            Self::SaveSkillVersion { .. } => "save_skill_version",
            Self::SetSkillBinding { .. } => "set_skill_binding",
            Self::ImportSkills { .. } => "import_skills",
            // The four of `connection::REQUEST_NAMES`, in that order (MOD-15 M6 D4).
            Self::ConnectionInfo => "connection_info",
            Self::SetDsn(_) => "set_dsn",
            Self::ClearDsn => "clear_dsn",
            Self::RebuildCache => "rebuild_cache",
            Self::QdrantInfo => "qdrant_info",
            Self::SetQdrantUrl(_) => "set_qdrant_url",
            Self::SetQdrantApiKey(_) => "set_qdrant_api_key",
            Self::ClearQdrantSettings => "clear_qdrant_settings",
            // The two of `concepts_worker::REQUEST_NAMES`, in that order (MOD-64 D241).
            Self::SearchConcepts(_) => "search_concepts",
            Self::IndexConcepts { .. } => "index_concepts",
            // Blueprint D209: one name per verb, `run_worker::ORCH_NAMES`.
            Self::Orch(request) => request.name(),
            Self::RunStream { .. } => "run_stream",
            Self::Document(_) => "document",
            Self::RunActions(_) => "run_actions",
            // MOD-42 plan D14.
            Self::RelayView { .. } => "relay_view",
            Self::AnswerPermission { .. } => "answer_permission",
            // MOD-72 plan D4.
            Self::ToolCalls { .. } => "tool_calls",
            // The ten of `requirements::REQUEST_NAMES`, in that order (MOD-39 plan P2).
            Self::Requirements(..) => "requirements",
            Self::RequirementDetail(..) => "requirement_detail",
            Self::ItemRequirements(..) => "item_requirements",
            Self::CreateRequirementArea { .. } => "create_requirement_area",
            Self::MintRequirement { .. } => "mint_requirement",
            Self::AmendRequirement { .. } => "amend_requirement",
            Self::WithdrawRequirement { .. } => "withdraw_requirement",
            Self::CiteRequirement { .. } => "cite_requirement",
            Self::UnciteRequirement { .. } => "uncite_requirement",
            Self::ReconfirmCitation { .. } => "reconfirm_citation",
            // The three of `item_writes::REQUEST_NAMES`, in that order (MOD-13 milestone 2).
            Self::ItemForm { .. } => "item_form",
            Self::MintItem { .. } => "mint_item",
            Self::EditItem { .. } => "edit_item",
            // The four of `hand_written::REQUEST_NAMES`, in that order (MOD-13 milestone 5 D1).
            Self::NoteForm { .. } => "note_form",
            Self::AddNote { .. } => "add_note",
            Self::DocumentForm { .. } => "document_form",
            Self::WriteDocument { .. } => "write_document",
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
    /// Answer to [`StoreRequest::Waiting`].
    Waiting {
        /// The asked scope: a reply for a scope the shell has since left - another workspace, or
        /// the same one with another project set - is dropped, as `observe_reply` runs before the
        /// freshness gate (MOD-69 review M1, R2).
        scope: Scope,
        /// The list and the two counts.
        view: WaitingView,
    },
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
        /// MOD-11 D18: the session carries `htui`'s permission-prompt port, so it announces and
        /// answers permission requests although `caps` — the row's, which the interlock reads —
        /// says it cannot (a CLI session).
        prompts: bool,
    },
    /// Answer to [`StoreRequest::StoreState`].
    StoreState {
        /// `Backend::label()`: `memory`, `connecting`, `online` or `offline · <age>`.
        label: String,
        /// How many migrations are pending; `None` when the question does not apply — a memory
        /// backend, an offline one, or a connected one whose schema is up to date.
        migrations_pending: Option<usize>,
        /// The database's target version when this build is below it (MOD-40 plan D9, PRD D3):
        /// `PgStore::below_target` of an `Online` backend, `None` for every other. The shell
        /// says it once on the status line; the session runs regardless.
        below_target: Option<String>,
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
    DeleteReach(Option<Box<DeleteReach>>),
    /// Answer to [`StoreRequest::DeleteWorkspace`] / [`StoreRequest::DeleteProject`]: what was
    /// removed, and what the mirror did about it (D10).
    Deleted {
        /// What was deleted.
        target: DeleteTarget,
        /// The rows it took, per table — the same counts the warning pane showed.
        reach: Box<DeleteReach>,
        /// What the worker did to the mirror afterwards.
        mirror: MirrorAfterDelete,
    },
    /// Answer to [`StoreRequest::InferRepoPaths`]: the tree as it is now and what the pass did,
    /// per repo (plan D116). One reply carries both, so the section never patches a row locally.
    /// No URL travels here: the report names repos and canonical paths only.
    RepoPathsInferred {
        /// The workspace re-read after the writes.
        tree: Box<HierarchySnapshot>,
        /// Per repo, what happened and why.
        report: InferReport,
    },
    /// Answer to [`StoreRequest::ListDir`] (MOD-49 P3): directories and links to directories,
    /// names only, never a link's target (`R-BOX-4`).
    DirListing(DirListing),
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
    /// The scope's prompt templates, freshly read: the answer to [`StoreRequest::Templates`] (MOD-9
    /// D5). A read answer only: a save that applied answers [`StoreReply::TemplateSaved`]
    /// (MOD-59).
    Templates(Box<TemplatesSnapshot>),
    /// A template save missed its CAS token (PRD D5): the templates as they are now, for the editor
    /// to reload against. The editor keeps its typed text and retries only by hand.
    TemplatesStale(Box<TemplatesSnapshot>),
    /// The answer to a [`StoreRequest::SaveTemplate`] that applied (MOD-59 D1): the templates
    /// re-read after it and the row the store appended. Self-naming: the Templates view lands its
    /// save on this variant alone, and a plain [`StoreReply::Templates`] never closes its editor or
    /// moves its token.
    TemplateSaved {
        /// The templates as they are now; or, when only the re-read failed, its `StoreError`
        /// rendered through `Display`. The version was appended either way (D5).
        snapshot: Result<Box<TemplatesSnapshot>, String>,
        /// The template's project, as stored.
        project: ProjectId,
        /// The template's name, as stored.
        name: String,
        /// The version the save appended.
        version: i32,
    },
    /// The Skills view's snapshot, freshly read: the answer to [`StoreRequest::Skills`] (MOD-9
    /// D81). A read answer only: a skill write that applied answers [`StoreReply::SkillWritten`]
    /// (MOD-59).
    Skills(Box<SkillsSnapshot>),
    /// A skill write missed its token, or its skill, project or phase is gone (D81, D97): the
    /// snapshot as it is now and which write it answers. The draft keeps its text and retries only
    /// by hand.
    SkillsStale {
        /// The snapshot as it is now.
        snapshot: Box<SkillsSnapshot>,
        /// Which write went stale.
        what: StaleWhat,
    },
    /// The answer to every skill write that applied (MOD-59 D1): the snapshot re-read after it,
    /// and what the write did. Self-naming: the Skills view lands a write on this variant alone,
    /// and a plain [`StoreReply::Skills`] never closes its editor, form or question.
    SkillWritten {
        /// The snapshot as it is now; or, when only the re-read failed, its `StoreError` rendered
        /// through `Display`. The write landed either way (D5).
        snapshot: Result<Box<SkillsSnapshot>, String>,
        /// What the write did.
        outcome: SkillWrite,
    },
    /// The library after an import, and what happened to every file it touched
    /// ([`StoreRequest::ImportSkills`], MOD-9 import plan D97). One variant and not two: a file
    /// that lost its token is a row of `report` while the rest of the batch lands, so there is no
    /// `SkillsStale` shape to answer.
    SkillImports(Box<SkillImports>),
    /// `ConnectionInfo`, and every connection writer's success (D4): the section re-renders from
    /// it and never patches a field of its own into what it already had.
    Connection(ConnectionSnapshot),
    /// Reply to QdrantInfo, SetQdrantDsn, ClearQdrantSettings requests.
    Qdrant(crate::qdrant_settings_info::QdrantSnapshot),
    /// Answer to [`StoreRequest::SearchConcepts`] and [`StoreRequest::IndexConcepts`] (MOD-64 D232):
    /// the outcome carries its own error, so a Qdrant failure stays in the search overlay and never
    /// reaches the status line the way a [`StoreReply::Failed`] does.
    Concepts(Box<ConceptsReply>),
    /// Answer to [`StoreRequest::Orch`], once, at its `seq`.
    Orch(crate::run_worker::OrchReply),
    /// One frame of a [`StoreRequest::RunStream`] subscription, at the subscribing `seq`.
    RunStream(crate::run_worker::RunFrame),
    /// Answer to [`StoreRequest::Document`]; `None` when no row has that id.
    Document(Box<Option<Document>>),
    /// Answer to [`StoreRequest::RunActions`].
    RunActions(Box<crate::run_worker::ItemActions>),
    /// Answer to [`StoreRequest::RelayView`] for `item` (MOD-42 blueprint B-13).
    RelayView {
        /// The item asked about.
        item: ItemId,
        /// What it holds.
        view: Box<RelayView>,
    },
    /// Answer to [`StoreRequest::ToolCalls`] for `item`, in `ToolCallCount::sort_canonical` order.
    ToolCalls {
        /// The item asked about.
        item: ItemId,
        /// One row per `(step, tool_kind)` with at least one call.
        counts: Vec<ToolCallCount>,
    },
    /// [`StoreRequest::AnswerPermission`] won its compare-and-set; a refusal is
    /// [`StoreReply::Failed`] with the refusal's sentence (MOD-42 blueprint B-13).
    PermissionAnswered {
        /// The answered request.
        permission: PermissionId,
    },
    /// What one box probe did (MOD-7 D13): one per box probe, at the requester's address or
    /// [`UNSOLICITED`].
    BoxProbed(crate::agent_worker::BoxProbeReport),
    /// This user's boxes, freshly read: the answer to [`StoreRequest::Boxes`] and to every box
    /// or probe spec write that applied (MOD-7 milestone 2, D46; MOD-51 D4).
    Boxes(Box<BoxesSnapshot>),
    /// A box edit or a probe spec write missed its token, or its row is gone (D46, D48; MOD-51
    /// D4): the boxes as they are now, for the editor to reload against. The editor keeps its
    /// typed text and retries only on save.
    BoxesStale(Box<BoxesSnapshot>),
    /// The answer to every agent registry write (MOD-23 D240) and to `SetToolPaths` (MOD-66 D7):
    /// the registry re-read after the write, and what the write did. Self-naming (MOD-59): the
    /// Settings section lands a write on this variant alone, and a plain [`StoreReply::Agents`]
    /// never closes its form or moves its token.
    AgentWritten {
        /// The registry as it is now, ordered by name, whatever the outcome.
        agents: Vec<AgentSummary>,
        /// What the write did.
        outcome: AgentWrite,
    },
    /// The persona registry by name: the answer to [`StoreRequest::Personas`] (MOD-26 M2 D21). A
    /// read answer only: it never closes an editor or moves its token.
    Personas(Vec<Persona>),
    /// The answer to every persona write (D21; self-naming, MOD-59): the registry re-read after
    /// the write, and what the write did.
    PersonaWritten {
        /// The registry as it is now, by name, whatever the outcome.
        personas: Vec<Persona>,
        /// What the write did.
        outcome: PersonaWrite,
    },
    /// The registry after an import, and what happened to every file (D20). Boxed: the report
    /// can be long.
    PersonaImports(Box<PersonaImports>),
    /// The scope's requirements, freshly read: the answer to [`StoreRequest::Requirements`] (MOD-39
    /// plan P3). A read answer only: a tab write that applied answers
    /// [`StoreReply::RequirementWritten`] (MOD-59).
    Requirements(Box<RequirementsSnapshot>),
    /// An amend or withdraw missed its version (plan P3): the snapshot as it is now. The form keeps
    /// its text and retries only by hand.
    RequirementsStale(Box<RequirementsSnapshot>),
    /// The answer to every tab write that applied (MOD-59 D1): the scope re-read after it, and
    /// what the write did. Self-naming: the Requirements tab lands a write on this variant alone,
    /// and a plain [`StoreReply::Requirements`] never closes its form.
    RequirementWritten {
        /// The requirements as they are now; or, when only the re-read failed, its `StoreError`
        /// rendered through `Display`. The write landed either way (D5).
        snapshot: Result<Box<RequirementsSnapshot>, String>,
        /// What the write did.
        outcome: RequirementWrite,
    },
    /// Answer to [`StoreRequest::RequirementDetail`].
    RequirementDetail(Box<RequirementDetail>),
    /// Answer to [`StoreRequest::ItemRequirements`] and to every citation write that applied.
    ItemCitations(Box<ItemCitations>),
    /// Answer to [`StoreRequest::ItemForm`]; boxed, it carries a whole `Item`.
    ItemForm(Box<ItemFormContext>),
    /// An item write that applied (self-naming, MOD-59): the tab lands a write on this alone.
    ItemWritten {
        /// The item written.
        item: ItemId,
        /// What the write did.
        outcome: ItemWrite,
    },
    /// An edit that missed its version: nothing was written; both sides and the fresh catalogue
    /// (milestone 3 D7).
    ItemDiverged(Box<ItemDivergence>),
    /// Answer to [`StoreRequest::NoteForm`]: the item exists and the store takes a write.
    NoteForm {
        /// The item.
        item: ItemId,
    },
    /// A note that landed (self-naming, MOD-59): the Notes pane closes its area on this alone.
    NoteAdded {
        /// The item the note landed on.
        item: ItemId,
    },
    /// Answer to [`StoreRequest::DocumentForm`]; boxed, it carries a whole `Document`.
    DocumentForm(Box<DocumentFormContext>),
    /// A document that landed (self-naming): `version` is the one the store allocated (D5).
    DocumentWritten {
        /// The item the document landed on.
        item: ItemId,
        /// The kind as stored (trimmed).
        kind: String,
        /// The version the store allocated.
        version: i32,
    },
    /// The store failed. `request` is [`StoreRequest::name`].
    Failed {
        /// Which request failed.
        request: &'static str,
        /// The `StoreError`, rendered through `Display`.
        message: String,
    },
}

/// What one self-naming write did, before the re-read that answers it (MOD-59 review L3): the
/// skills, templates and requirements writers each hand their `answer` one of these. Only
/// [`WriteOutcome::answer`] turns it into a reply, so the three keep one rule: an applied write
/// answers its self-naming reply whatever the re-read came to (D5), and a stale one answers its
/// stale reply, whose failed re-read stays an error (D3).
///
/// Not [`htui_core::store::CasOutcome`]: its `Stale` carries the row as it is now, of the applied
/// row's own type, where a stale write here carries only which write missed and the re-read is
/// the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome<T, S = ()> {
    /// The write applied: what the store says it wrote.
    Applied(T),
    /// The write missed its token, or its row is gone: which write, when the reply says.
    Stale(S),
}

impl<T, S> WriteOutcome<T, S> {
    /// The reply, given the re-read after the write: `written` for an applied write, handed the
    /// re-read whatever it came to; `stale` for a stale one, handed the snapshot, or the re-read's
    /// error in its place.
    ///
    /// # Errors
    /// The re-read's, after a stale write only.
    pub fn answer<R>(
        self,
        reread: StoreResult<R>,
        written: impl FnOnce(StoreResult<R>, T) -> StoreReply,
        stale: impl FnOnce(R, S) -> StoreReply,
    ) -> StoreResult<StoreReply> {
        match self {
            Self::Applied(outcome) => Ok(written(reread, outcome)),
            Self::Stale(what) => Ok(stale(reread?, what)),
        }
    }
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
/// advertised, a sentence the agent itself wrote to its own stderr, a link it printed, a status the
/// probe decided, or what a loopback listener answered a delivered paste, with the pasted `code`
/// and `state` blanked out of it (MOD-22 D268). `htui` never reads the credential a login leaves
/// behind — it asks the probe whether one exists.
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
    /// What the login's loopback listener answered a delivered paste (MOD-22 D268).
    ///
    /// An **answer**, not an outcome: a `4xx` or `5xx` is reported here too, and the adapter
    /// decides what it means. The flow keeps running either way, and ends with its own terminal
    /// frame. Carries a status, a reason, a host and an excerpt with the pasted `code` and `state`
    /// blanked. Not a terminal frame.
    Delivered(ListenerReply),
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

/// MOD-69 review L4: the open permissions, or `None` (unknown, as offline) when their read fails
/// for any reason but [`StoreError::Unreachable`], which propagates so `go_offline` still fires.
/// A broken permission read then costs the list its permission rows, not the whole reply.
fn permissions_or_unknown(
    read: StoreResult<Vec<WaitingPermission>>,
) -> StoreResult<Option<Vec<WaitingPermission>>> {
    match read {
        Ok(permissions) => Ok(Some(permissions)),
        Err(err @ StoreError::Unreachable(_)) => Err(err),
        Err(err) => {
            tracing::warn!(%err, "the waiting list's permission read failed; listing none");
            Ok(None)
        }
    }
}

/// [`StoreRequest::Waiting`]. MOD-69 plan D1, D4, D6: one candidate read, plus the open
/// permissions online; the classification is the engine's own guards (`htui_worker::waiting`). An
/// `Unreachable` from either read propagates, so `go_offline` still drops the backend (blueprint
/// H-12); any other failure of the permission read only leaves them unknown (review L4).
///
/// `read_permissions` is the writer's permission read. [`try_serve`] passes
/// [`WriteStore::open_permissions`]; it is a parameter only so a test can make it fail with
/// something other than `Unreachable`, which no [`Backend`] can be made to do (review R2).
async fn serve_waiting(
    backend: &Backend,
    scope: &Scope,
    read_permissions: impl AsyncFnOnce(&Writer, &Scope) -> StoreResult<Vec<WaitingPermission>>,
) -> StoreResult<StoreReply> {
    let active = backend.active_runs(scope).await?;
    let candidates = backend.waiting_candidates(scope).await?;
    let writer = backend.writer();
    let permissions = match &writer {
        Some(writer) => permissions_or_unknown(read_permissions(writer, scope).await)?,
        None => None,
    };
    Ok(StoreReply::Waiting {
        scope: scope.clone(),
        // Review L3 with L4: no writer is offline; a writer with unknown permissions is a failed
        // read online, which the overlay must not call offline.
        view: WaitingView {
            offline: writer.is_none(),
            ..htui_worker::waiting(scope, active, &candidates, permissions.as_deref())
        },
    })
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
        StoreRequest::Waiting { scope } => {
            serve_waiting(backend, scope, async |writer, scope| {
                WriteStore::open_permissions(writer, scope).await
            })
            .await?
        }
        StoreRequest::Items {
            scope,
            filter,
            ready_here,
        } => StoreReply::Items(read_items(backend, scope, filter, *ready_here).await?),
        StoreRequest::Item(id) => StoreReply::Item(Box::new(backend.item(*id).await?)),
        StoreRequest::Links { id, hops } => StoreReply::Links(backend.links(*id, *hops).await?),
        StoreRequest::Documents(id) => StoreReply::Documents(backend.documents(*id).await?),
        StoreRequest::Notes(id) => StoreReply::Notes(backend.notes(*id).await?),
        StoreRequest::Runs(id) => StoreReply::Runs(backend.runs(*id).await?),
        // MOD-72 D4: an ordinary read, so an `Unreachable` drops an `Online` backend onto the
        // mirror, which answers from its window (plan D2) - unlike `RelayView`, which is the
        // writer's.
        StoreRequest::ToolCalls { item } => StoreReply::ToolCalls {
            item: *item,
            counts: backend.tool_call_counts(*item).await?,
        },
        StoreRequest::Agents => StoreReply::Agents(backend.agents().await?),
        // Served through the ordinary read path on purpose: an `Unreachable` from it drops an
        // `Online` backend onto the mirror exactly as any other read does, and the replay then
        // answers from whatever the mirror kept.
        StoreRequest::StepEvents(step) => StoreReply::StepEvents {
            step_id: *step,
            events: backend.step_events(*step).await?,
        },
        // The five chat requests need the worker loop's own state (the live sessions), and the
        // two probes, the preview, the three install requests, MOD-21's four login ones,
        // MOD-22's delivery and MOD-66's tool-paths write need the runtime that owns their tasks,
        // so all seventeen are served ahead of this function, exactly as `ApplyMigrations` is. One
        // of them that reaches here at all belongs to a caller with no runtime — the test harness
        // without one — and saying so is more use than a panic.
        StoreRequest::PromptPreview { .. }
        | StoreRequest::ChatStart { .. }
        | StoreRequest::ChatSend { .. }
        | StoreRequest::ChatAnswer { .. }
        | StoreRequest::ChatCancel { .. }
        | StoreRequest::ChatFollow { .. }
        | StoreRequest::ProbeAgents
        | StoreRequest::ProbeBox
        | StoreRequest::InstallPlan { .. }
        | StoreRequest::InstallConfirm { .. }
        | StoreRequest::InstallCancel
        | StoreRequest::AuthStart { .. }
        | StoreRequest::AuthChoose { .. }
        | StoreRequest::AuthOpen { .. }
        | StoreRequest::AuthDeliver { .. }
        | StoreRequest::AuthCancel
        | StoreRequest::SetToolPaths { .. } => StoreReply::Failed {
            request: request.name(),
            message: "no agent runtime in this build".to_owned(),
        },
        // The thirteen hierarchy requests, or-ed rather than guarded: this `match` has no wildcard,
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
        | StoreRequest::DeleteProject(..)
        | StoreRequest::InferRepoPaths(..) => hierarchy::serve(backend, request).await?,
        // The nine catalogue requests, or-ed for the same reason the thirteen above are: a guard
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
        // The three prompt settings requests, or-ed for the same reason the twenty-two above are:
        // a guard does not count towards exhaustivity in a wildcard-free `match`, so `_ if …`
        // would be an E0004 here (MOD-15 M3 plan F-12, M5 plan F-13).
        StoreRequest::PromptSettings(..)
        | StoreRequest::SetSetting { .. }
        | StoreRequest::ClearSetting { .. } => prompt_settings::serve(backend, request).await?,
        // The two template requests, or-ed for the reason the arms above are: a guard does not count
        // towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12).
        StoreRequest::Templates(..) | StoreRequest::SaveTemplate { .. } => {
            templates::serve(backend, request).await?
        }
        // The six skill requests, or-ed for the reason the arms above are: a guard does not count
        // towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-9 D81, import
        // plan D97).
        StoreRequest::Skills(..)
        | StoreRequest::CreateSkill { .. }
        | StoreRequest::EditSkill { .. }
        | StoreRequest::SaveSkillVersion { .. }
        | StoreRequest::SetSkillBinding { .. }
        | StoreRequest::ImportSkills { .. } => skills::serve(backend, request).await?,
        // The four connection requests, or-ed for the same reason the twenty-five above are: a
        // guard does not count towards exhaustivity in a wildcard-free `match`, so `_ if …` would
        // be an E0004 here (MOD-15 M3 plan F-12, M6 plan D9). Only the read is answered: the three
        // writers need `reconnect`, `refresher` and the connect context, none of which a function
        // over `&Backend` can reach, so they are refused here and served by the loop below.
        StoreRequest::ConnectionInfo
        | StoreRequest::SetDsn(_)
        | StoreRequest::ClearDsn
        | StoreRequest::RebuildCache => connection::serve(backend, request).await?,
        // The three box requests, or-ed for the same reason the twenty-nine above are: a guard
        // does not count towards exhaustivity in a wildcard-free `match`, so `_ if …` would be an
        // E0004 here (MOD-15 M3 plan F-12, MOD-7 milestone 2 D46, MOD-51 D4).
        StoreRequest::Boxes | StoreRequest::EditBox { .. } | StoreRequest::SetProbeSpec { .. } => {
            box_settings::serve(backend, request).await?
        }
        // The three agent registry writes, or-ed for the reason the arms above are: a guard does
        // not count towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-23
        // D241). Served here, in the loop: one statement and one read each (`R-NF-3`).
        StoreRequest::CreateAgent { .. }
        | StoreRequest::EditAgent { .. }
        | StoreRequest::SetAgentOnBox { .. } => agent_settings::serve(backend, request).await?,
        // The five persona requests, or-ed for the reason the arms above are: a guard does not
        // count towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-26 M2
        // D21). Served here, in the loop: the import's file reads included (`R-NF-3`, I-11).
        StoreRequest::Personas
        | StoreRequest::CreatePersona { .. }
        | StoreRequest::UpdatePersona { .. }
        | StoreRequest::DeletePersona { .. }
        | StoreRequest::ImportPersonas { .. } => persona_settings::serve(backend, request).await?,
        // The ten requirement requests, or-ed for the reason the arms above are: a guard does not
        // count towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-39 plan
        // P1).
        StoreRequest::Requirements(..)
        | StoreRequest::RequirementDetail(..)
        | StoreRequest::ItemRequirements(..)
        | StoreRequest::CreateRequirementArea { .. }
        | StoreRequest::MintRequirement { .. }
        | StoreRequest::AmendRequirement { .. }
        | StoreRequest::WithdrawRequirement { .. }
        | StoreRequest::CiteRequirement { .. }
        | StoreRequest::UnciteRequirement { .. }
        | StoreRequest::ReconfirmCitation { .. } => requirements::serve(backend, request).await?,
        // The three item requests, or-ed for the reason the arms above are: a guard does not
        // count towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-13
        // milestone 2 D1).
        StoreRequest::ItemForm { .. }
        | StoreRequest::MintItem { .. }
        | StoreRequest::EditItem { .. } => item_writes::serve(backend, request).await?,
        // The four hand-written requests, or-ed for the reason the arms above are (MOD-13
        // milestone 5 D1).
        StoreRequest::NoteForm { .. }
        | StoreRequest::AddNote { .. }
        | StoreRequest::DocumentForm { .. }
        | StoreRequest::WriteDocument { .. } => hand_written::serve(backend, request).await?,
        StoreRequest::StoreState => StoreReply::StoreState {
            label: backend.label(),
            migrations_pending: None,
            below_target: backend
                .writable()
                .and_then(PgStore::below_target)
                .map(str::to_owned),
        },
        StoreRequest::ApplyMigrations => StoreReply::MigrationsApplied { applied: 0 },
        // MOD-49 (plan P3): this box's filesystem under `spawn_blocking`; no store is read, so it
        // answers offline too, and a refusal is a `Constraint` that never drops the backend. The
        // spawned loop answers it from its own task before reaching here (review M-1); this arm
        // is `serve`'s, for the harness and `--demo`.
        StoreRequest::ListDir { path, show_hidden } => list_dir(path, *show_hidden).await?,
        StoreRequest::QdrantInfo
        | StoreRequest::SetQdrantUrl(_)
        | StoreRequest::SetQdrantApiKey(_)
        | StoreRequest::ClearQdrantSettings => StoreReply::Failed {
            request: request.name(),
            message: "handled in worker loop".to_owned(),
        },
        // MOD-64 D231: the loop serves both through the concepts runtime; one that reaches here
        // belongs to a caller with none (the harness default), and is answered in the overlay's
        // own reply (D232).
        StoreRequest::SearchConcepts(query) => {
            StoreReply::Concepts(Box::new(ConceptsReply::Hits {
                query: query.clone(),
                outcome: Err(concepts_worker::NOT_AVAILABLE.to_owned()),
            }))
        }
        StoreRequest::IndexConcepts { .. } => StoreReply::Concepts(Box::new(
            ConceptsReply::Indexed(Err(concepts_worker::NOT_AVAILABLE.to_owned())),
        )),
        // Blueprint D183 (F-C, F-P): the three orchestrator reads need no runtime. A shell with
        // none — the test harness — gets a subscription acknowledgement, the verdicts with no live
        // chat, and the document, so no view renders a status-line error for asking.
        StoreRequest::Document(id) => StoreReply::Document(Box::new(backend.document(*id).await?)),
        StoreRequest::RunStream { item } => {
            StoreReply::RunStream(crate::run_worker::RunFrame::subscribed(*item))
        }
        StoreRequest::RunActions(item) => StoreReply::RunActions(Box::new(
            crate::run_worker::actions(backend, *item, &LiveChats::default()).await?,
        )),
        // MOD-42 D14: a display read; offline (no writer) the view is empty, never `Failed` or
        // `Unreachable` (OQ-4): a `Failed` lands on the status line (`app/update.rs`) and an
        // `Unreachable` drops the backend through `go_offline`.
        StoreRequest::RelayView { item } => StoreReply::RelayView {
            item: *item,
            view: Box::new(match backend.writer() {
                Some(writer) => WriteStore::relay_view(&writer, *item).await?,
                None => RelayView::default(),
            }),
        },
        // MOD-42 D3, D14: refused offline before anything is sent (the `requirements.rs`
        // pattern); a lost compare-and-set is the refusal's sentence on the status line (B-13).
        StoreRequest::AnswerPermission {
            permission,
            option_id,
        } => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            let user = backend.this_user().await?;
            let box_id = backend
                .box_info()
                .await?
                .map(|info| info.box_id)
                .ok_or_else(|| StoreError::NotFound {
                    entity: "box",
                    id: "this box".to_owned(),
                })?;
            match WriteStore::answer_permission(&writer, *permission, option_id, user, box_id)
                .await?
            {
                AnswerOutcome::Answered => StoreReply::PermissionAnswered {
                    permission: *permission,
                },
                AnswerOutcome::Refused(why) => StoreReply::Failed {
                    request: request.name(),
                    message: why.to_string(),
                },
            }
        }
        // A command needs the runtime that owns its task, and the loop serves every one of them
        // ahead of this function; one that reaches here belongs to a caller with no runtime.
        StoreRequest::Orch(_) => StoreReply::Failed {
            request: request.name(),
            message: NO_RUN_RUNTIME.to_owned(),
        },
    })
}

/// MOD-13 D2: `Items`, with this box's readiness composed in. Not `Backend::ready_items`, which
/// refuses offline (MOD-25: an offline box still browses).
///
/// The box row is read first, so a refused `box_info` costs no item read; it answers `Failed`
/// like every other arm.
async fn read_items(
    backend: &Backend,
    scope: &Scope,
    filter: &ItemFilter,
    ready_here: bool,
) -> StoreResult<Vec<ItemSummary>> {
    if !ready_here {
        return backend.items(scope, filter).await;
    }
    let info = backend.box_info().await?;
    let filter = ItemFilter {
        ready: Some(true),
        ..filter.clone()
    };
    Ok(runnable_here(
        backend.items(scope, &filter).await?,
        info.as_ref(),
    ))
}

/// The rows `info` has every required tag for, in the store's order; `None` has no tags
/// (MOD-13 D2, the capability half of `MemStore::ready_items`).
fn runnable_here(rows: Vec<ItemSummary>, info: Option<&BoxInfo>) -> Vec<ItemSummary> {
    rows.into_iter()
        .filter(|row| {
            row.required_tags.iter().all(|tag| {
                info.is_some_and(|b| b.probed_tags.contains(tag) || b.declared_tags.contains(tag))
            })
        })
        .collect()
}

/// `ListDir` (MOD-49 P3): [`list_dirs`] off the async task, as `hierarchy::canonical` wraps
/// `canonical_root`, bounded by [`LIST_TIMEOUT`] (review M-1). A refusal becomes
/// [`StoreError::Constraint`], so it reads
/// ``list_dir: constraint violated: `/x` does not exist on this box``.
async fn list_dir(path: &str, show_hidden: bool) -> StoreResult<StoreReply> {
    list_dir_within(path, show_hidden, LIST_TIMEOUT, list_dirs).await
}

/// [`list_dir`] over a chosen lister and bound, so a test can hang one (MOD-49 review M-1).
///
/// A listing that hasn't answered within `within` becomes [`StoreError::Constraint`] naming the
/// path as typed. Its blocking thread is left to finish on its own: a thread stuck in the kernel
/// can't be cancelled, only stopped being waited for.
async fn list_dir_within<F>(
    path: &str,
    show_hidden: bool,
    within: std::time::Duration,
    lister: F,
) -> StoreResult<StoreReply>
where
    F: FnOnce(&std::path::Path, bool, usize) -> Result<DirListing, RootRefusal> + Send + 'static,
{
    let typed = path.to_owned();
    let walk = tokio::task::spawn_blocking(move || {
        lister(std::path::Path::new(&typed), show_hidden, LIST_CAP)
    });
    let listing = tokio::time::timeout(within, walk)
        .await
        .map_err(|_| {
            StoreError::Constraint(format!(
                "`{path}` did not answer within {} s on this box",
                within.as_secs_f32()
            ))
        })?
        .map_err(|err| StoreError::Backend(err.to_string()))?
        .map_err(|refusal| StoreError::Constraint(refusal.to_string()))?;
    Ok(StoreReply::DirListing(listing))
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
/// - A **box heartbeat** every `Started::box_heartbeat` ([`connect::BOX_HEARTBEAT`]) while the
///   backend is `Online`: `PgStore::touch_box` on a spawned task, one at a time, its failure only
///   logged (MOD-40 plan D7).
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
    rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    runtime: AgentRuntime,
) -> tokio::task::JoinHandle<()> {
    let runs = crate::run_worker::production_for(&started.backend);
    spawn_with_runtimes(started, rx, tx, runtime, runs)
}

/// [`spawn_with`] hosting htui's MCP tools (MOD-11 D11): with `tools`, the chat runtime and the
/// run runtime open their leases on the one host, and the loop hands it the current writable
/// backend at the top of every iteration (B-2, `host_the_backend`), so a session opened after a
/// `SetDsn` writes to the new server.
///
/// With `secrets` (MOD-10 D15), the chat runtime and the run runtime resolve provider projects'
/// secrets through the one source, so a provider's login latch and cool-down are the process's,
/// not one runtime's. `None` for both is [`spawn_with`] exactly.
pub fn spawn_hosted(
    started: Started,
    rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    runtime: AgentRuntime,
    tools: Option<Arc<htui_mcp::McpHost<Backend>>>,
    secrets: Option<Arc<dyn SecretSource>>,
) -> tokio::task::JoinHandle<()> {
    let mut runtime = runtime;
    let mut runs = crate::run_worker::production_for(&started.backend);
    if let Some(source) = &secrets {
        runtime = runtime.with_secret_source(Arc::clone(source));
        runs = runs.with_secret_source(Arc::clone(source));
    }
    if let Some(host) = &tools {
        (runtime, runs) = host_the_runtimes(runtime, runs, host);
    }
    spawn_with_concepts(
        started,
        rx,
        tx,
        runtime,
        runs,
        ConceptsRuntime::production(),
        tools,
    )
}

/// Both runtimes open their leases on `host` (MOD-11 D11). The run runtime gets it through
/// [`SharedToolHost`], so its shutdown leaves the host to the loop (T6 ADV-2).
fn host_the_runtimes(
    runtime: AgentRuntime,
    runs: RunRuntime,
    host: &Arc<htui_mcp::McpHost<Backend>>,
) -> (AgentRuntime, RunRuntime) {
    (
        runtime.with_tool_host(Arc::clone(host) as Arc<dyn htui_orch::tools::ToolHost>),
        runs.with_tool_host(Arc::new(SharedToolHost(Arc::clone(host)))),
    )
}

/// The run runtime's view of the tool host the loop shares with the chat runtime (MOD-11 T6
/// ADV-2): `open` is the host's; `close` is a no-op.
///
/// `RunRuntime::shutdown` closes its tool host once its walks are done (B-19), which is right in
/// `htui worker`, where it owns the host alone. In the TUI the loop joins that shutdown with the
/// chat runtime's, so with no walk left the run runtime would close the host while every chat is
/// still inside its cancel window, ending the sessions of agents that have not stopped yet (H-25).
/// The loop closes the host itself, once, after both shutdowns.
#[derive(Debug)]
struct SharedToolHost(Arc<htui_mcp::McpHost<Backend>>);

impl htui_orch::tools::ToolHost for SharedToolHost {
    fn open(
        &self,
        scope: htui_orch::tools::ToolScope,
    ) -> Result<htui_orch::tools::ToolLease, htui_orch::tools::ToolHostError> {
        self.0.open(scope)
    }

    fn close(&self) {}
}

/// The steps a chat of this process is live on (blueprint D206): the runtime's chats whose
/// session task is still running (`AgentRuntime::live_steps`).
pub(crate) fn live_chats(runtime: &AgentRuntime) -> LiveChats {
    LiveChats::of(runtime.live_steps())
}

/// What the loop does with a [`RunServed`] that is not a plain reply (blueprint D181): the
/// runtime's event channel carries only `Attach`.
///
/// A promotion's engine writes are done and `Orch(Promoted)` has answered the request, so the chat
/// runtime binds a session to the promoted step (MOD-4 plan D165). Its task answers the same
/// address with `ChatAccepted` and every frame after it; a refusal is answered here, once.
async fn on_run_served(
    served: RunServed,
    runtime: &mut AgentRuntime,
    backend: &Backend,
    tx: &mpsc::UnboundedSender<ReplyEnvelope>,
) {
    match served {
        RunServed::Attach {
            addr,
            promoted,
            ended,
        } => {
            match runtime
                .attach_promoted(backend, tx, addr.clone(), *promoted)
                .await
            {
                Served::Start { step_id, task } => {
                    runtime.attach(step_id, tokio::spawn(ended.after(task)));
                }
                Served::Reply(reply) => {
                    let _ = tx.send(ReplyEnvelope {
                        seq: addr.seq,
                        origin: addr.origin,
                        reply,
                    });
                }
                Served::Deferred => {}
            }
        }
        other @ (RunServed::Reply(_) | RunServed::Deferred) => {
            debug_assert!(false, "the run runtime's event channel carries only Attach");
            tracing::error!(?other, "a run event that is not an attach was dropped");
        }
    }
}

/// [`spawn_with`] over a chosen [`RunRuntime`] too (plan D153): the orchestrator's runtime lives
/// inside this loop beside the chat one, for the same reason.
///
/// Beyond [`spawn_with`]'s three sources, the loop owns the run runtime's event receiver and a
/// sweep ticker of `runs.sweep_every()` (D190), guarded on a writer; it sweeps at start when the
/// backend has one and after every `Online` swap. A command-poll ticker of
/// `htui_worker::worker::COMMAND_POLL` (MOD-42 plan D13) sits beside the sweeper, guarded the
/// same way.
pub fn spawn_with_runtimes(
    started: Started,
    rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    runtime: AgentRuntime,
    runs: RunRuntime,
) -> tokio::task::JoinHandle<()> {
    // MOD-64 D231: the production concepts runtime, so no public signature moves.
    spawn_with_concepts(
        started,
        rx,
        tx,
        runtime,
        runs,
        ConceptsRuntime::production(),
        None,
    )
}

/// [`spawn_with_runtimes`] over a chosen [`ConceptsRuntime`] too: crate-private, for a test that
/// needs the loop over a fake index (MOD-64 review 5).
pub(crate) fn spawn_with_concepts(
    started: Started,
    mut rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    mut runtime: AgentRuntime,
    mut runs: RunRuntime,
    mut concepts: ConceptsRuntime,
    tools: Option<Arc<htui_mcp::McpHost<Backend>>>,
) -> tokio::task::JoinHandle<()> {
    let Started {
        mut backend,
        mut events,
        events_tx,
        projects,
        settings,
        box_heartbeat,
        // `mut` since MOD-15 M6: a `SetDsn` replaces the closure so every later tick dials the
        // **new** server, and a `ClearDsn` drops it so none dials a credential the user just
        // deleted (D11 step 7, D13).
        mut reconnect,
        // What `connect::apply_dsn` needs (D20); `None` from `detached`, which is what makes
        // `SetDsn` refuse under `--demo` and in a test that did not ask for one.
        connect,
    } = started;

    tokio::spawn(async move {
        // How many migrations are waiting, and the store that is waiting to apply them.
        let mut pending: Option<usize> = None;
        let mut held: Option<PgStore> = None;
        let mut refresher: Option<Refresher> = None;
        // The refresher's last pass outcome, for as long as there is a refresher.
        let mut health: Option<watch::Receiver<Option<StoreError>>> = None;
        // The last dial's outcome, for the connection section's Status row (MOD-15 M6, B-5).
        // Forgotten by a `SetDsn`, whose swap makes the previous server's outcome a fact about a
        // database this session has left.
        let mut last_attempt: Option<Attempt> = None;
        // Which server the spawned dials are for (ruling O-1, hazard H-6). Every dial captures the
        // generation it was spawned under and drops its `ConnEvent` when `SetDsn` has moved it:
        // otherwise the old server's `PgStore` is installed over the **new** mirror, and this
        // database's reads are answered from another database's cache (PRD `:373`).
        let dials = Arc::new(AtomicU64::new(0));
        // `interval_at`, not `interval`: the first tick of an `interval` completes immediately,
        // which would re-dial in the same breath as `start`'s own attempt.
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + connect::RECONNECT,
            connect::RECONNECT,
        );
        // A dial that overran its slot must not then be followed by a burst of catch-up dials.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // D181, D190: the run runtime's events and its sweep ticker are loop locals, like the two
        // above, so a handler may borrow `runs` (H-4).
        let mut run_events = runs.take_events();
        let mut sweep_every = runs.sweep_every();
        let mut sweeper = sweep_ticker(sweep_every);
        // MOD-42 plan D13: the command poll's ticker, beside the sweeper; first tick at once.
        let mut commands = tokio::time::interval(htui_worker::worker::COMMAND_POLL);
        commands.set_missed_tick_behavior(MissedTickBehavior::Delay);
        if backend.writer().is_some() {
            runs.sweep(&backend, &tx);
        }
        // MOD-40 plan D7 (C4): the box heartbeat's ticker, first beat one period out and `Delay`
        // on a missed one like the two above, and the beat in flight, if any (blueprint B14).
        let mut box_beat =
            tokio::time::interval_at(tokio::time::Instant::now() + box_heartbeat, box_heartbeat);
        box_beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut beat: Option<tokio::task::JoinHandle<()>> = None;

        loop {
            // MOD-11 B-2 (H-10): the backend this iteration serves is the one a tool session
            // opened from now on writes to — robust to every site below that swaps or mutates it.
            if let Some(host) = &tools {
                host_the_backend(host, &backend);
            }
            // The first sweep reads `lease_ttl_seconds`; the ticker follows it.
            if runs.sweep_every() != sweep_every {
                sweep_every = runs.sweep_every();
                sweeper = sweep_ticker(sweep_every);
            }
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
                            below_target: backend
                                .writable()
                                .and_then(PgStore::below_target)
                                .map(str::to_owned),
                        },
                        StoreRequest::ApplyMigrations if held.is_some() => {
                            // `if held.is_some()` above, so this cannot be the `None` arm; the
                            // store has to be moved out to be applied (`&mut self`).
                            match held.take() {
                                Some(mut pg) => {
                                    // What `box.toml` said: this store connected to a pending
                                    // schema, so nothing has registered it yet (MOD-7 D3).
                                    let presented = pg.identity().clone();
                                    match pg.apply_migrations().await {
                                        Ok(()) => {
                                            let applied = pending.take().unwrap_or(0);
                                            tracing::info!(applied, "schema migrations applied");
                                            // MOD-7 D3: this bootstrap registered the box, so
                                            // a minted id is written back here, as
                                            // `try_connect` does over an up-to-date schema.
                                            // F1: and remembered for the session's later dials,
                                            // which present it while `box.toml` cannot be
                                            // rewritten instead of minting one row per tick.
                                            if let Some(ctx) = connect.as_ref() {
                                                if let Err(err) = connect::persist_registration(
                                                    &ctx.config_root, &presented, &pg,
                                                ) {
                                                    tracing::warn!(
                                                        %err,
                                                        "the registered box id was not written back to box.toml"
                                                    );
                                                }
                                                ctx.registered.record(&presented, &pg);
                                            }
                                            go_online(
                                                &mut backend, pg, &mut refresher, &mut health,
                                                &projects, settings,
                                            ).await;
                                            // MOD-7 D11: the registration probe, then D190's sweep.
                                            runtime.on_online(&backend, &tx);
                                            runs.sweep(&backend, &tx);
                                            StoreReply::MigrationsApplied { applied }
                                        }
                                        Err(err) => {
                                            // Still pending, still held: `y` can be answered
                                            // again.
                                            held = Some(pg);
                                            failed("apply_migrations", &err)
                                        }
                                    }
                                }
                                None => StoreReply::MigrationsApplied { applied: 0 },
                            }
                        }
                        // The four connection requests, kept by the loop for the reason
                        // `ApplyMigrations` above is kept: the read needs this loop's memory of the
                        // last dial and of `--offline`, and each writer rewires at least two of
                        // `backend`, `refresher`, `health`, `held`, `reconnect` and `dials`, none
                        // of which `try_serve`'s `&Backend` can reach (MOD-15 M6 D9, flag K).
                        StoreRequest::ConnectionInfo => {
                            connection::snapshot(&backend, last_attempt.as_ref(), connect.as_ref())
                                .await
                                .map_or_else(
                                    |err| failed("connection_info", &err),
                                    StoreReply::Connection,
                                )
                        }
                        // D11's eight steps, in this order and no other. Everything that can fail
                        // happens **before** the swap at (6); nothing after it may fail, because a
                        // half-applied swap is a mirror handle for one database answering another
                        // database's reads.
                        StoreRequest::SetDsn(dsn) => 'set: {
                            // (0) A demo session has no keyring and no mirror (D10).
                            if matches!(backend, Backend::Memory(_)) {
                                break 'set StoreReply::Failed {
                                    request: "set_dsn",
                                    message: connection::DEMO_SESSION.to_owned(),
                                };
                            }
                            // (0') No context is `Started::detached` outside `--demo`: there is no
                            // config root to open a mirror under and no timeout to dial with.
                            let Some(ctx) = connect.as_ref() else {
                                break 'set StoreReply::Failed {
                                    request: "set_dsn",
                                    message: connection::NO_WORKER.to_owned(),
                                };
                            };
                            // (1) The DSN parsed on the UI side of the seam - it is a `Dsn`, so it
                            // parsed. (2) the keyring write and (3) the new mirror are
                            // `apply_dsn`'s, in that order: a mirror opened for a DSN that was
                            // never stored is a lie the next launch inherits. A failure in either
                            // leaves `backend`, `refresher`, `health`, `held` and `reconnect`
                            // exactly as they were.
                            let applied = match connect::apply_dsn(dsn.clone(), ctx).await {
                                Ok(applied) => applied,
                                Err(err) => break 'set failed("set_dsn", &err),
                            };
                            // (4) The refresher mirrored the **old** server and the health watch
                            // was armed for it. Aborted before the close below, so its last write
                            // is not in flight while the pool goes.
                            if let Some(previous) = refresher.take() {
                                previous.abort();
                            }
                            health = None;
                            // (5) The old mirror's pool, closed rather than dropped: dropping the
                            // last handle closes it asynchronously, which is the race
                            // `CacheStore::close` exists to avoid (`cache/mod.rs:216-224`).
                            if let Some(old) = backend.cache() {
                                old.close().await;
                            }
                            // (6) Nothing from here on can fail. The old `PgStore`, and the one
                            // held aside for a migration prompt, both belong to the old server;
                            // their pools close when the old backend drops. The generation moves
                            // here, so every dial spawned before this point discards its event.
                            held = None;
                            pending = None;
                            dials.fetch_add(1, Ordering::SeqCst);
                            // Every walk goes back to the server it was claimed on, and the run
                            // runtime forgets the old server's parts and queue (MOD-4 T6).
                            runs.forget_server();
                            backend = Backend::Offline {
                                cache: applied.cache,
                                // `since: None` renders `connecting`, which is what the top bar
                                // should say while the dial is in flight; `--offline` never reads
                                // `connecting`, because nothing is in flight (D12).
                                since: if ctx.offline { Some(Utc::now()) } else { None },
                            };
                            // MOD-11 B-2, T6 ADV-1: the one offline backend the tool host takes
                            // (`host_the_backend` keeps the last writable one): every walk was
                            // preempted above, and the old server's pool must not outlive it here.
                            if let Some(host) = &tools {
                                host.set_host(backend.clone());
                            }
                            // (7) Every later tick dials the new server - unless this session was
                            // started with `--offline`, which is a choice the user typed.
                            reconnect = if ctx.offline {
                                None
                            } else {
                                Some(applied.reconnect)
                            };
                            // (8) One immediate dial, on the channel the ticker uses.
                            if let Some(dial) = reconnect.clone() {
                                spawn_dial(dial, events_tx.clone(), &dials);
                            }
                            // The previous server's outcome is not this one's.
                            last_attempt = None;
                            connection::snapshot(&backend, None, connect.as_ref())
                                .await
                                .map_or_else(|err| failed("set_dsn", &err), StoreReply::Connection)
                        }
                        // D13: the keyring entry goes and the dial is disarmed, because the
                        // closure captured the deleted credential by move. A live `Online` backend
                        // is **not** torn down - the pool is not the secret, and throwing a working
                        // connection and its refresher away serves no security end.
                        StoreRequest::ClearDsn => 'clear: {
                            if matches!(backend, Backend::Memory(_)) {
                                break 'clear StoreReply::Failed {
                                    request: "clear_dsn",
                                    message: connection::DEMO_SESSION.to_owned(),
                                };
                            }
                            if let Err(err) = connect::forget_dsn().await {
                                break 'clear failed("clear_dsn", &err);
                            }
                            reconnect = None;
                            connection::snapshot(&backend, last_attempt.as_ref(), connect.as_ref())
                                .await
                                .map_or_else(
                                    |err| failed("clear_dsn", &err),
                                    StoreReply::Connection,
                                )
                        }
                        // D14: the same `rebuild` the two production callers make
                        // (`hierarchy.rs:339`, `catalogue.rs:171-177`). The refresher, if one is
                        // running, refills from cursor zero on its next pass exactly as it does
                        // after those.
                        StoreRequest::RebuildCache => 'rebuild: {
                            let Some(cache) = backend.cache() else {
                                break 'rebuild StoreReply::Failed {
                                    request: "rebuild_cache",
                                    message: connection::DEMO_SESSION.to_owned(),
                                };
                            };
                            if let Err(err) = cache.rebuild().await {
                                break 'rebuild failed("rebuild_cache", &err);
                            }
                            connection::snapshot(&backend, last_attempt.as_ref(), connect.as_ref())
                                .await
                                .map_or_else(
                                    |err| failed("rebuild_cache", &err),
                                    StoreReply::Connection,
                                )
                        }
                        StoreRequest::QdrantInfo => {
                            StoreReply::Qdrant(crate::qdrant_settings_info::QdrantSnapshot::fetch().await)
                        }
                        StoreRequest::SetQdrantUrl(url) => {
                            let url_str = url.clone();
                            let res = tokio::task::spawn_blocking(move || {
                                htui_store::secret::set_qdrant_url(&url_str)?;
                                Ok::<(), StoreError>(())
                            })
                            .await
                            .unwrap();
                            if let Err(err) = res {
                                failed("set_qdrant_url", &err)
                            } else {
                                StoreReply::Qdrant(crate::qdrant_settings_info::QdrantSnapshot::fetch().await)
                            }
                        }
                        StoreRequest::SetQdrantApiKey(key) => {
                            let k = key.as_str().to_string();
                            let res = tokio::task::spawn_blocking(move || {
                                if k.is_empty() {
                                    htui_store::secret::clear_qdrant_api_key()?;
                                } else {
                                    htui_store::secret::set_qdrant_api_key(&k)?;
                                }
                                Ok::<(), StoreError>(())
                            })
                            .await
                            .unwrap();
                            if let Err(err) = res {
                                failed("set_qdrant_api_key", &err)
                            } else {
                                StoreReply::Qdrant(crate::qdrant_settings_info::QdrantSnapshot::fetch().await)
                            }
                        }
                        StoreRequest::ClearQdrantSettings => {
                            let res = tokio::task::spawn_blocking(|| {
                                htui_store::secret::clear_qdrant_url()?;
                                htui_store::secret::clear_qdrant_api_key()?;
                                Ok::<(), StoreError>(())
                            })
                            .await
                            .unwrap();
                            if let Err(err) = res {
                                failed("clear_qdrant_settings", &err)
                            } else {
                                StoreReply::Qdrant(crate::qdrant_settings_info::QdrantSnapshot::fetch().await)
                            }
                        }
                        // The chat requests need this loop's own state - the live sessions - and
                        // the probes, the tool-paths write, the installs and the logins need the
                        // runtime that owns their tasks, so all of them go to the runtime before
                        // `try_serve`, like `ApplyMigrations` above. An install reads the registry
                        // over the network and streams hundreds of megabytes, and a login waits on
                        // a human in a browser, so `Served::Deferred => continue` is the whole of
                        // `R-NF-3` for both: the arm returns having spawned a task and awaited
                        // nothing longer than `box_info()` (blueprint H-9).
                        StoreRequest::PromptPreview { .. }
                        | StoreRequest::ChatStart { .. }
                        | StoreRequest::ChatSend { .. }
                        | StoreRequest::ChatAnswer { .. }
                        | StoreRequest::ChatCancel { .. }
                        | StoreRequest::ChatFollow { .. }
                        | StoreRequest::ProbeAgents
                        | StoreRequest::ProbeBox
                        | StoreRequest::InstallPlan { .. }
                        | StoreRequest::InstallConfirm { .. }
                        | StoreRequest::InstallCancel
                        | StoreRequest::AuthStart { .. }
                        | StoreRequest::AuthChoose { .. }
                        | StoreRequest::AuthOpen { .. }
                        | StoreRequest::AuthDeliver { .. }
                        | StoreRequest::AuthCancel
                        | StoreRequest::SetToolPaths { .. } => {
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
                        // Plan D153: every orchestrator command is a task of the run runtime, and
                        // so is every `RunActions` read (D215); `Deferred => continue` is the
                        // whole of `R-NF-3` for them. `Document` is a
                        // read and goes to `try_serve` below (D183).
                        StoreRequest::Orch(_)
                        | StoreRequest::RunStream { .. }
                        | StoreRequest::RunActions(_) => {
                            let live = live_chats(&runtime);
                            match runs.serve(&backend, &tx, &envelope, &live).await {
                                RunServed::Reply(reply) => reply.into(),
                                RunServed::Deferred => continue,
                                attach @ RunServed::Attach { .. } => {
                                    on_run_served(attach, &mut runtime, &backend, &tx).await;
                                    continue;
                                }
                            }
                        }
                        // MOD-64 D231: a search loads a model and calls Qdrant, an index run reads
                        // every item: both are tasks of the concepts runtime, and
                        // `Deferred => continue` is the whole of `R-NF-3`.
                        StoreRequest::SearchConcepts(_) | StoreRequest::IndexConcepts { .. } => {
                            match concepts.serve(&backend, &tx, &envelope) {
                                ConceptsServed::Reply(reply) => reply,
                                ConceptsServed::Deferred => continue,
                            }
                        }
                        // MOD-49 review M-1: a listing reads this box's filesystem, and a hung mount
                        // never returns, so it is answered from its own task, like the concepts
                        // arm: `continue` is the whole of `R-NF-3` for it, and `LIST_TIMEOUT`
                        // bounds what the picker waits for. `try_serve` keeps an inline arm for
                        // `serve`'s callers (the harness, `--demo`), which have no loop to stall.
                        StoreRequest::ListDir { path, show_hidden } => {
                            let (path, show_hidden) = (path.clone(), *show_hidden);
                            let (seq, origin, tx) = (envelope.seq, envelope.origin.clone(), tx.clone());
                            tokio::spawn(async move {
                                let reply = match list_dir(&path, show_hidden).await {
                                    Ok(reply) => reply,
                                    Err(err) => failed(LIST_DIR, &err),
                                };
                                let _ = tx.send(ReplyEnvelope { seq, origin, reply });
                            });
                            continue;
                        }
                        other => {
                            let served = try_serve(&backend, other).await;
                            // This read is what noticed the server had gone. The asking view
                            // still hears back exactly once; the next read finds the mirror.
                            // MOD-37 M4 D3 (R-46), review M1: this swap does **not** preempt.
                            // `Unreachable` here includes sqlx's `PoolTimedOut` - local pool
                            // load with the server up - and a walk is not cut for that. Only
                            // the refresher's arm preempts, and `go_offline` aborts the
                            // refresher, so a loss a read notices first leaves the walks on
                            // the pre-M4 path: the heartbeat fence, then adoption.
                            if let Some(err) = lost_the_store(&served) {
                                let _ = go_offline(&mut backend, &mut refresher, &mut health, err);
                            }
                            match served {
                                Ok(reply) => reply,
                                Err(err) => failed(other.name(), &err),
                            }
                        }
                    };

                    let answer = ReplyEnvelope { seq: envelope.seq, origin: envelope.origin, reply };
                    if tx.send(answer).is_err() {
                        break;
                    }
                }

                // The **one** generation check (ruling O-1, hazard H-6, review HIGH-1). It is at
                // consumption rather than at the send for two reasons that between them cover
                // every path an event can take: `connect::start`'s own launch dial never passes
                // through `spawn_dial` at all - it reports under `connect::LAUNCH_GENERATION` -
                // and an event a dial queued in the instant before the `fetch_add` at D11 step (6)
                // is already past any send-side check by the time the generation moves.
                //
                // A `select!` precondition cannot do this job: it is evaluated before the future
                // is polled, so it cannot see the generation that arrives *with* the message, and
                // a stale event left at the head of the queue would block every later event behind
                // it for the rest of the session.
                Some((generation, event)) = events.recv() => {
                    if generation != dials.load(Ordering::SeqCst) {
                        discard_stale(event).await;
                        continue;
                    }
                    match event {
                    ConnEvent::Online(pg) => {
                        pending = None;
                        held = None;
                        go_online(
                            &mut backend, pg, &mut refresher, &mut health, &projects, settings,
                        )
                        .await;
                        last_attempt = Some(Attempt { at: Utc::now(), outcome: AttemptOutcome::Online });
                        tracing::info!(label = backend.label(), "store online");
                        // MOD-7 D11: the registration probe decides on its own task.
                        runtime.on_online(&backend, &tx);
                        // D190: every `Online` sweeps, so a run a dead process left is adopted.
                        runs.sweep(&backend, &tx);
                    }
                    ConnEvent::MigrationsPending(pg, n) => {
                        pending = Some(n);
                        held = Some(pg);
                        // The mirror stays the read path and the top bar stops saying
                        // `connecting`: nothing reads through a schema this binary refuses to
                        // use until the user has answered the prompt.
                        backend.gave_up();
                        last_attempt = Some(Attempt {
                            at: Utc::now(),
                            outcome: AttemptOutcome::MigrationsPending(n),
                        });
                        tracing::warn!(pending = n, "schema has pending migrations");
                    }
                    ConnEvent::Failed(why) => {
                        backend.gave_up();
                        last_attempt = Some(Attempt {
                            at: Utc::now(),
                            outcome: AttemptOutcome::Failed(why.clone()),
                        });
                        tracing::warn!(%why, "connect failed");
                    }
                    }
                }

                Some(served) = run_events.recv() => {
                    on_run_served(served, &mut runtime, &backend, &tx).await;
                }

                _ = sweeper.tick(), if backend.writer().is_some() => {
                    runs.sweep(&backend, &tx);
                }

                // MOD-42 plan D13 (B-10): a cancel a live chat refuses stays pending, silently.
                _ = commands.tick(), if backend.writer().is_some() => {
                    let live = live_chats(&runtime);
                    runs.poll_commands(&backend, &tx, &live);
                }

                // D7: only a backend with a server to beat against, and never a second beat
                // while one is in flight: a touch that waits on a registration's or an editor's
                // row lock is not joined by another every period. Spawned, so the loop never
                // waits on the server (`R-NF-3`).
                _ = box_beat.tick(), if backend.writable().is_some() && beat.is_none() => {
                    if let Some(pg) = backend.writable().cloned() {
                        beat = Some(tokio::spawn(beat_once(pg)));
                    }
                }

                // The beat in flight ends, and the ticker's arm is live again at once: a tick
                // missed while it ran fires now (`Delay`) rather than at the next event.
                _ = in_flight(&mut beat), if beat.is_some() => {
                    beat = None;
                }

                err = lost_the_server(health.clone()) => {
                    // The refresher passes every `interval`, so it usually notices first.
                    // MOD-37 M4 D3 (R-46): the swap preempts every live walk.
                    if go_offline(&mut backend, &mut refresher, &mut health, &err) {
                        runs.preempt_walks();
                    }
                }

                _ = ticker.tick(),
                    if reconnect.is_some() && !backend.is_writable() && held.is_none() =>
                {
                    if let Some(dial) = reconnect.clone() {
                        spawn_dial(dial, events_tx.clone(), &dials);
                    }
                }
            }
        }

        // The UI is gone. Every walk is cancelled — its lease given back, its agent killed by its
        // guard — and every live chat is cancelled, and both are awaited **before** this task
        // returns: dropping a session task at its first await orphans the agent process it
        // spawned (`docs/ANA-4.md` §11 criterion 11). The two run side by side, so a slow walk
        // does not eat the window `lib.rs`'s `SHUTDOWN` leaves the chats.
        tokio::join!(
            runs.shutdown(crate::agent_worker::CANCEL_GRACE),
            runtime.shutdown(crate::agent_worker::CANCEL_GRACE),
        );
        // MOD-11 B-19, T6 ADV-2: the shared tool host goes once both runtimes are down, so no chat
        // loses its session while its agent is still winding down.
        if let Some(host) = &tools {
            htui_orch::tools::ToolHost::close(host.as_ref());
        }
        concepts.shutdown();

        if let Some(refresher) = refresher {
            refresher.abort();
        }
        if let Some(beat) = beat {
            beat.abort();
        }
    })
}

/// Hands `backend` to the shared tool host when it can write (MOD-11 B-2, T6 ADV-1).
///
/// A backend that went offline is **not** handed over: `go_offline` does not preempt a walk, and
/// the walk's `Kit` keeps its own `Online` clone, so the server's last writable backend is still
/// the one its next session's tool writes belong to. Handing over `Offline` would refuse that
/// lease and fail the run (`agent spawn failed`) over a blip the walk's own pool survived. A
/// chat needs a writer to start at all, so it never opens a lease in that window. Moving to
/// another server is the one swap that must reach the host while offline: `SetDsn` preempts
/// every walk and hands the host its `Offline` backend itself, so no session writes to the
/// server the session has left and the old pool is not kept open by the host.
fn host_the_backend(host: &htui_mcp::McpHost<Backend>, backend: &Backend) {
    if backend.writer().is_some() {
        host.set_host(backend.clone());
    }
}

/// The sweep ticker (D190): first tick one period from now, `Delay` on a missed one.
fn sweep_ticker(every: std::time::Duration) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker
}

/// The box heartbeat in flight, to its end: pending when there is none, so the loop's arm over it
/// is inert until a beat is spawned. A beat that panicked or was aborted ends it all the same.
async fn in_flight(beat: &mut Option<tokio::task::JoinHandle<()>>) {
    match beat {
        Some(handle) => drop(handle.await),
        None => std::future::pending().await,
    }
}

/// One box heartbeat (MOD-40 plan D7), on its own task.
///
/// A failure is logged and nothing else. A lost server is the refresher's to report
/// ([`lost_the_server`]); a heartbeat that swapped the backend itself would be a second path to
/// [`go_offline`] racing the first, from a task that does not own the backend (blueprint B14).
async fn beat_once(pg: PgStore) {
    let id = pg.this_box();
    match pg.touch_box(id).await {
        Ok(true) => {}
        Ok(false) => tracing::warn!(box_id = %id, "the box heartbeat found no box row to touch"),
        Err(err) => {
            tracing::debug!(%err, "the box heartbeat failed; the refresher reports a lost server");
        }
    }
}

/// Runs one dial on its own task and reports it under the generation it was spawned in.
///
/// The generation is read when the dial is spawned and sent **with** the outcome; `SetDsn` bumps
/// it at the moment of the swap (D11 step 6), so a dial that was in flight across a swap answers
/// for a server this session has left and the worker's one check drops its [`ConnEvent`] rather
/// than delivering it. Without that, an `Online` from the old server would be installed by
/// [`go_online`] over the **new** mirror — this database's reads answered from another database's
/// cache (ruling O-1, hazard H-6).
///
/// The check below is an **early out**, not the guard: it only saves a queue slot and closes the
/// old pool sooner. The guard that cannot be raced is the one in the loop, because it is the only
/// one a dial finishing during `apply_dsn` cannot slip past (review HIGH-1 gap (b)).
///
/// Both call sites go through here — the reconnect ticker's and `SetDsn`'s own.
fn spawn_dial(
    dial: connect::Reconnect,
    sender: mpsc::Sender<(u64, ConnEvent)>,
    dials: &Arc<AtomicU64>,
) {
    let dials = Arc::clone(dials);
    let generation = dials.load(Ordering::SeqCst);
    tokio::spawn(async move {
        let event = dial().await;
        if dials.load(Ordering::SeqCst) != generation {
            discard_stale(event).await;
            return;
        }
        // The worker is gone if this fails, and there is nobody left to tell.
        let _ = sender.send((generation, event)).await;
    });
}

/// Lets go of a [`ConnEvent`] that answers for a server this session has left.
///
/// Not a plain drop: an `Online` and a `MigrationsPending` each carry a [`PgStore`] holding a pool
/// of up to eight server connections, and dropping the last `PgPool` handle closes it
/// *asynchronously* — the same race `CacheStore::close` exists to avoid (`cache/mod.rs:216-224`),
/// for the same reason D11 step (5) closes the old mirror rather than dropping it.
async fn discard_stale(event: ConnEvent) {
    match event {
        ConnEvent::Online(pg) | ConnEvent::MigrationsPending(pg, _) => {
            tracing::debug!("a dial for a previous DSN connected; closing its pool");
            pg.pool().close().await;
        }
        ConnEvent::Failed(why) => {
            tracing::debug!(%why, "a dial for a previous DSN failed; dropping its outcome");
        }
    }
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
    // Read before the move: `this_box` and the two cache settings are what only a connected server
    // knows (blueprint C.13's `RefreshSettings`).
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
///
/// Whether the backend went `Online → Offline` now ([`Backend::went_offline`]), which the
/// refresher's arm answers by preempting every live walk (MOD-37 M4 D3, R-46). A read's arm
/// discards it (review M1): its `Unreachable` may be pool load, not a lost server.
#[must_use = "an Online → Offline swap the refresher noticed must preempt the live walks (R-46)"]
fn go_offline(
    backend: &mut Backend,
    refresher: &mut Option<Refresher>,
    health: &mut Option<watch::Receiver<Option<StoreError>>>,
    why: &StoreError,
) -> bool {
    if let Some(previous) = refresher.take() {
        previous.abort();
    }
    *health = None;
    if !backend.went_offline() {
        return false;
    }
    tracing::warn!(%why, "store unreachable; falling back to the mirror");
    true
}

/// The [`StoreError::Unreachable`] a served request met, if it met one: its own `Err`, or the
/// re-read a persona import carries beside its report (R1 L-2 keeps the report, so that loss
/// arrives inside an `Ok`; R1 ADV-1). Any other failure is not a loss of the store.
fn lost_the_store(served: &StoreResult<StoreReply>) -> Option<&StoreError> {
    let err = match served {
        Err(err) => err,
        Ok(StoreReply::PersonaImports(imports)) => imports.personas.as_ref().err()?,
        Ok(_) => return None,
    };
    matches!(err, StoreError::Unreachable(_)).then_some(err)
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
    use htui_core::fixtures::{DemoData, demo_data, ids};
    use htui_core::model::{BoxId, NewItem, OsFamily, Status};
    use htui_core::store::MemStore;
    use htui_store::{CacheStore, Identity};
    use htui_worker::{WaitingReason, WaitingRow};

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

    /// MOD-64 D231, D232 (review 5): the loop hands a search to the concepts runtime and answers
    /// the next request while the search is still running; the search's error comes back in its
    /// own reply, never `Failed`. The fake index waits a (paused-clock) second first: had the loop
    /// served it inline, that second would pass before `BoxInfo` was read, and the search would
    /// answer first.
    #[tokio::test(start_paused = true)]
    async fn the_loop_serves_a_search_off_the_loop_and_never_as_failed() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let message = "qdrant: query: refused";
        let index = concepts_worker::MemIndex::new()
            .delayed(std::time::Duration::from_secs(1))
            .failing(message);
        let (tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rx) = mpsc::unbounded_channel();
        let _worker = spawn_with_concepts(
            Started::detached(backend),
            req_rx,
            rep_tx,
            AgentRuntime::production(),
            RunRuntime::production(),
            ConceptsRuntime::new(Arc::new(index)),
            None,
        );
        let query = crate::concepts::query("anything", scope.project_ids, false, 10);
        for (seq, request) in [
            (1, StoreRequest::SearchConcepts(query.clone())),
            (2, StoreRequest::BoxInfo),
        ] {
            tx.send(RequestEnvelope {
                seq,
                origin: Origin::App,
                request,
            })
            .expect("the worker is alive");
        }

        let first = rx.recv().await.expect("the worker answers");
        assert_eq!(first.seq, 2, "BoxInfo is answered while the search runs");
        assert!(
            matches!(first.reply, StoreReply::BoxInfo(_)),
            "{:?}",
            first.reply
        );
        let second = rx.recv().await.expect("the search answers");
        assert_eq!(second.seq, 1);
        let StoreReply::Concepts(concepts) = &second.reply else {
            panic!(
                "the search is answered in its own variant: {:?}",
                second.reply
            )
        };
        let ConceptsReply::Hits {
            query: echoed,
            outcome: Err(error),
        } = concepts.as_ref()
        else {
            panic!("the failing index's search fails: {concepts:?}")
        };
        assert_eq!(echoed, &query);
        assert_eq!(error, message);
    }

    /// MOD-49 review M-1: a listing that hangs (a dead mount) is answered as refused once its
    /// bound passes, naming the path as typed. The fake lister blocks until the test releases it,
    /// so the runtime's shutdown doesn't wait on it.
    #[tokio::test]
    async fn a_listing_that_does_not_answer_in_time_is_refused_by_its_path() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let err = list_dir_within(
            "/mnt/hung",
            false,
            std::time::Duration::from_millis(20),
            move |path, _, _| {
                let _ = held.recv();
                Err(RootRefusal::Missing(path.to_path_buf()))
            },
        )
        .await
        .expect_err("a hung listing is refused");
        let _ = release.send(());

        let StoreReply::Failed { request, message } = failed(LIST_DIR, &err) else {
            panic!("a refusal renders as `Failed`")
        };
        assert_eq!(request, LIST_DIR);
        assert!(
            message.contains("`/mnt/hung`") && message.contains("did not answer"),
            "the refusal names the path as typed and says why: {message}"
        );
    }

    /// The bounded helper answers a listing that is in time exactly as before.
    #[tokio::test]
    async fn a_listing_within_its_bound_is_answered() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        std::fs::create_dir(dir.path().join("alpha")).expect("the directory is created");
        let typed = dir.path().display().to_string();

        let reply = list_dir_within(&typed, false, LIST_TIMEOUT, list_dirs)
            .await
            .expect("a real directory is listed");
        let StoreReply::DirListing(listing) = reply else {
            panic!("a listing answers `DirListing`: {reply:?}")
        };
        assert_eq!(listing.path, typed);
        assert_eq!(listing.entries.len(), 1);
    }

    /// MOD-49 review M-1: the loop answers a listing from its own task, addressed to the request
    /// that asked (its `seq` and `origin`), beside the requests that follow it.
    #[tokio::test]
    async fn the_loop_answers_a_listing_at_its_own_address() {
        let dir = tempfile::tempdir().expect("a throwaway directory");
        let (tx, mut rx, _worker) = detached(demo());
        for (seq, request) in [
            (
                7,
                StoreRequest::ListDir {
                    path: dir.path().display().to_string(),
                    show_hidden: false,
                },
            ),
            (8, StoreRequest::BoxInfo),
        ] {
            tx.send(RequestEnvelope {
                seq,
                origin: Origin::App,
                request,
            })
            .expect("the worker is alive");
        }

        let mut replies = Vec::new();
        for _ in 0..2 {
            replies.push(rx.recv().await.expect("the worker answers"));
        }
        replies.sort_by_key(|reply| reply.seq);
        assert!(
            matches!(
                &replies[0],
                ReplyEnvelope {
                    seq: 7,
                    origin: Origin::App,
                    reply: StoreReply::DirListing(_)
                }
            ),
            "{:?}",
            replies[0]
        );
        assert!(
            matches!(&replies[1].reply, StoreReply::BoxInfo(_)),
            "{:?}",
            replies[1]
        );
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

    /// MOD-69 plan D6, blueprint H-5: over the demo's Platform scope, `RUN_2` is the only active
    /// run and owns no row, `htui` FEAT-2 (blocked, no run) is the one Reopen row, and TOOL-1
    /// (awaiting approval, no run) is a candidate with no row.
    #[tokio::test]
    async fn serve_waiting_reads_the_demo_platform() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        assert_eq!(
            StoreRequest::Waiting {
                scope: scope.clone()
            }
            .name(),
            "waiting"
        );
        let asked = scope.clone();
        let StoreReply::Waiting {
            scope: answered,
            view,
        } = serve(&backend, &StoreRequest::Waiting { scope }).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(
            answered, asked,
            "the reply names the scope it read (review M1)"
        );
        assert_eq!(view.working, 1, "RUN_2 is the fixture's only active run");
        assert!(view.permissions_known, "a Memory backend has a writer");
        assert!(!view.offline, "a Memory backend is not offline");
        assert_eq!(
            view.rows,
            vec![WaitingRow {
                item: ids::HTUI_FEAT_2,
                item_key: "FEAT-2".into(),
                run: None,
                step: None,
                step_label: String::new(),
                reason: WaitingReason::Unblock,
                text: "blocked, no active run: u reopens it".into(),
            }]
        );
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
                ready_here: false,
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(items.len(), 11, "eight htui items plus three agy items");
        assert_eq!(items[0].key, "ANA-1");
    }

    /// The demo plus one open htui item needing `cuda`, a tag the demo box has neither probed nor
    /// declared (MOD-13 blueprint E1: no demo item exercises the capability half on its own). The
    /// mint is `mem.rs`'s `the_eleven_inherent_reads_answer_from_the_fixture`'s. Clones share
    /// state, so the returned store sees what a `Backend` over its clone reads.
    async fn with_cuda(store: MemStore) -> (MemStore, ItemId) {
        let id = store
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
        (store, id)
    }

    /// One `Items` read through `serve`, unwrapped.
    async fn items_of(
        backend: &Backend,
        scope: &Scope,
        filter: ItemFilter,
        ready_here: bool,
    ) -> Vec<ItemSummary> {
        let reply = serve(
            backend,
            &StoreRequest::Items {
                scope: scope.clone(),
                filter,
                ready_here,
            },
        )
        .await;
        let StoreReply::Items(items) = reply else {
            panic!("wrong reply variant: {reply:?}")
        };
        items
    }

    fn ids_of(rows: &[ItemSummary]) -> Vec<ItemId> {
        rows.iter().map(|row| row.id).collect()
    }

    /// MOD-13 D2: `ready_here` is ANA-9 §7.4 for this box, the same rows in the same order as
    /// `MemStore::ready_items` (which `pg_criteria.rs` pins against `PgStore`).
    #[tokio::test]
    async fn ready_here_equals_ready_items_for_this_box() {
        let (store, needs_cuda) = with_cuda(MemStore::demo()).await;
        let backend = Backend::memory(store.clone());
        let scope = platform_scope(&backend).await;

        let open = items_of(
            &backend,
            &scope,
            ItemFilter {
                ready: Some(true),
                ..ItemFilter::default()
            },
            false,
        )
        .await;
        assert!(
            ids_of(&open).contains(&needs_cuda),
            "the store-side half alone keeps the cuda item, so the capability half is exercised"
        );

        let ready = items_of(&backend, &scope, ItemFilter::default(), true).await;
        assert_eq!(
            ready,
            store
                .ready_items(&scope, ids::BOX)
                .await
                .expect("the read is total")
        );
        let ready = ids_of(&ready);
        for id in [ids::HTUI_ANA_2, ids::AGY_FEAT_1, ids::AGY_FIX_1] {
            assert!(ready.contains(&id), "{id:?} is ready on this box");
        }
        assert!(!ready.contains(&needs_cuda), "this box has no `cuda`");
    }

    /// MOD-13 D2: an unregistered box (`box_info()` is `None`) has no tags, as §7.4's
    /// `LEFT JOIN … COALESCE`: only the untagged ready items stay.
    #[tokio::test]
    async fn ready_here_with_an_unregistered_box_keeps_only_untagged_items() {
        let store = MemStore::from_demo(DemoData {
            this_box: None,
            ..demo_data()
        });
        let backend = Backend::memory(store.clone());
        let scope = platform_scope(&backend).await;
        let ready = items_of(&backend, &scope, ItemFilter::default(), true).await;
        assert_eq!(
            ready,
            store
                .ready_items(&scope, BoxId::new())
                .await
                .expect("the read is total")
        );
        assert_eq!(ids_of(&ready), [ids::HTUI_ANA_2, ids::AGY_FIX_1]);
    }

    /// MOD-13 D2: readiness stays one conjunct among the others.
    #[tokio::test]
    async fn ready_here_is_conjunctive_with_projects_tags_and_statuses() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let agy = items_of(
            &backend,
            &scope,
            ItemFilter {
                project_ids: Some(vec![ids::PROJECT_AGY]),
                ..ItemFilter::default()
            },
            true,
        )
        .await;
        assert_eq!(ids_of(&agy), [ids::AGY_FEAT_1, ids::AGY_FIX_1]);
        let rust = items_of(
            &backend,
            &scope,
            ItemFilter {
                tags: Some(vec!["rust".to_owned()]),
                ..ItemFilter::default()
            },
            true,
        )
        .await;
        assert_eq!(ids_of(&rust), [ids::AGY_FEAT_1]);
        let done = items_of(
            &backend,
            &scope,
            ItemFilter {
                statuses: Some(vec![Status::Done]),
                ..ItemFilter::default()
            },
            true,
        )
        .await;
        assert!(done.is_empty(), "a done item is never ready: {done:?}");
    }

    /// MOD-13 D2: without `ready_here` the read is the store's plain `items`, row for row, even
    /// over a store with no box row.
    #[tokio::test]
    async fn ready_here_false_is_the_plain_read() {
        let backend = Backend::memory(MemStore::from_demo(DemoData {
            this_box: None,
            ..demo_data()
        }));
        let scope = platform_scope(&backend).await;
        let items = items_of(&backend, &scope, ItemFilter::default(), false).await;
        assert_eq!(
            items,
            backend
                .items(&scope, &ItemFilter::default())
                .await
                .expect("the memory store never fails")
        );
        assert_eq!(items.len(), 11, "the whole Platform scope");
    }

    /// MOD-13 D2's reason for not calling `Backend::ready_items`: offline, that refuses, while an
    /// `Items` read with `ready_here` answers from the mirror (MOD-25: an offline box still
    /// browses). `seed_mirror` mirrors this box's row but no item, so the answer is an empty
    /// list — an answer, not `Failed`.
    #[tokio::test]
    async fn ready_here_answers_offline_where_ready_items_refuses() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "ready-here-offline", 1)
            .await
            .expect("open a throwaway mirror");
        htui_store::testkit::seed_mirror(&cache, &demo_data())
            .await
            .expect("the mirror is seeded");
        let backend = Backend::Offline { cache, since: None };
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
        };
        assert!(
            backend.ready_items(&scope, ids::BOX).await.is_err(),
            "`ready_items` is refused offline"
        );
        let reply = serve(
            &backend,
            &StoreRequest::Items {
                scope,
                filter: ItemFilter::default(),
                ready_here: true,
            },
        )
        .await;
        assert!(
            matches!(&reply, StoreReply::Items(rows) if rows.is_empty()),
            "answered from the mirror: {reply:?}"
        );
    }

    /// MOD-13 D2: the capability half keeps the store's order, and no box row means no tags.
    #[test]
    fn runnable_here_keeps_order_and_treats_none_as_no_tags() {
        let row = |key: &str, tags: &[&str]| ItemSummary {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            key: key.to_owned(),
            key_prefix: "FEAT".to_owned(),
            key_number: 1,
            title: key.to_owned(),
            status: Status::Open,
            priority: 0,
            required_tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            updated_at: Utc::now(),
            touched_paths: Vec::new(),
        };
        let rows = vec![
            row("C", &["gpu"]),
            row("A", &[]),
            row("B", &["rust", "gpu"]),
            row("D", &["cuda"]),
        ];
        let keys = |rows: Vec<ItemSummary>| -> Vec<String> {
            rows.into_iter().map(|row| row.key).collect()
        };
        let info = BoxInfo {
            box_id: ids::BOX,
            hostname: "DESKTOP-HTUI".to_owned(),
            os_family: OsFamily::Linux,
            probed_tags: vec!["gpu".to_owned()],
            declared_tags: vec!["rust".to_owned()],
            settings: Value::Null,
        };
        assert_eq!(
            keys(runnable_here(rows.clone(), Some(&info))),
            ["C", "A", "B"],
            "probed and declared tags both count; store order is kept"
        );
        assert_eq!(keys(runnable_here(rows, None)), ["A"]);
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

    /// MOD-72 plan D4: the Runs flow's chip read, through the ordinary read path.
    #[tokio::test]
    async fn tool_calls_answers_the_item_s_per_step_counts() {
        let backend = demo();
        let StoreReply::ToolCalls { item, counts } = serve(
            &backend,
            &StoreRequest::ToolCalls {
                item: ids::HTUI_FEAT_1,
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(item, ids::HTUI_FEAT_1, "the reply names the item it read");
        assert_eq!(
            counts,
            vec![ToolCallCount {
                step: ids::STEP_PLAN,
                tool_kind: "read".to_owned(),
                calls: 1,
            }],
            "the fixture's one read on the plan step"
        );
        assert_eq!(StoreRequest::ToolCalls { item }.name(), "tool_calls");

        let StoreReply::ToolCalls { counts, .. } = serve(
            &backend,
            &StoreRequest::ToolCalls {
                item: ids::HTUI_ANA_2,
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert!(counts.is_empty(), "an item with no run has no chips");
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
            below_target,
        } = round_trip(&tx, &mut rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "memory");
        assert_eq!(
            migrations_pending, None,
            "a memory backend has no schema to migrate"
        );
        assert_eq!(below_target, None, "a memory backend has no target");
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
                ready_here: false,
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

        // `LAUNCH_GENERATION`: nothing here ever calls `SetDsn`, so generation zero is current and
        // this injection is delivered (review HIGH-1).
        events_tx
            .send((
                connect::LAUNCH_GENERATION,
                ConnEvent::Failed("no server".to_owned()),
            ))
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

    /// MOD-40 plan D9, blueprint B16: the target a connect read travels inside the `PgStore`, and
    /// the worker copies it into every `StoreState` it answers over that store.
    #[tokio::test(flavor = "multi_thread")]
    async fn store_state_carries_below_target_from_an_online_store() {
        let Some(db) = htui_store::testkit::fresh_db().await else {
            return;
        };
        sqlx::query(
            "INSERT INTO app_setting (key, value) VALUES ($1, $2) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        )
        .bind(htui_store::TARGET_VERSION_KEY)
        .bind(serde_json::json!("99.0.0"))
        .execute(&db.pool)
        .await
        .expect("plant another box's newer target");
        let pg = PgStore::connect(&db.url, &db.identity)
            .await
            .expect("a TUI connects below the target")
            .store;
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "below-target", 1)
            .await
            .expect("open a throwaway mirror");

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(Backend::Online {
            pg,
            cache: cache.clone(),
        });
        started.reconnect = None;
        let worker = spawn(started, req_rx, rep_tx);

        let StoreReply::StoreState {
            label,
            below_target,
            ..
        } = round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "online");
        assert_eq!(
            below_target.as_deref(),
            Some("99.0.0"),
            "the store's target reaches the shell on the reply it re-reads"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
        db.drop_db().await;
    }

    /// MOD-40 plan D7 (C4): an `Online` worker stamps its box's `last_seen_at` every
    /// `Started::box_heartbeat`, against a real server, and moves no editor's token doing it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_box_heartbeat_stamps_last_seen_at_while_online() {
        let Some(db) = htui_store::testkit::fresh_db().await else {
            return;
        };
        let id = db.store.this_box();
        let seen = || async {
            sqlx::query_scalar::<_, DateTime<Utc>>("SELECT last_seen_at FROM box WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_one(&db.pool)
                .await
                .expect("read the registered box row")
        };
        let before = seen().await;
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "heartbeat", 1)
            .await
            .expect("open a throwaway mirror");

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, _rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        });
        started.reconnect = None;
        started.box_heartbeat = std::time::Duration::from_millis(50);
        let worker = spawn(started, req_rx, rep_tx);

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut stamped = seen().await;
        while stamped <= before && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            stamped = seen().await;
        }
        assert!(
            stamped > before,
            "a beat stamped the box while online: {before} -> {stamped}"
        );
        let edit_version: i32 = sqlx::query_scalar("SELECT edit_version FROM box WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read the box's editor token");
        assert_eq!(edit_version, 0, "no editor token moved");

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
        db.drop_db().await;
    }

    /// MOD-40 blueprint B14: a heartbeat against a server that is not there is logged and nothing
    /// else. The refresher owns the swap to `Offline`; a beat never makes it.
    ///
    /// The "server" is a listener that hangs up on every connection, so each beat's `touch_box`
    /// fails at once and is counted. Beats keep coming, one period after the last ended, and the
    /// backend is still `online` after all of them.
    #[tokio::test]
    async fn a_failed_box_heartbeat_leaves_the_backend_online() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a throwaway port");
        let port = listener.local_addr().expect("a bound port").port();
        let dialled = Arc::new(AtomicU64::new(0));
        let hangups = tokio::spawn({
            let dialled = Arc::clone(&dialled);
            async move {
                while let Ok((socket, _)) = listener.accept().await {
                    dialled.fetch_add(1, Ordering::SeqCst);
                    drop(socket);
                }
            }
        });

        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "heartbeat-fails", 1)
            .await
            .expect("open a throwaway mirror");
        let identity = Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = PgStore::lazy(
            &format!("postgres://nobody:nothing@127.0.0.1:{port}/none"),
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
        started.reconnect = None;
        started.box_heartbeat = std::time::Duration::from_millis(20);
        let worker = spawn(started, req_rx, rep_tx);

        // The start-up sweep dials too, once; after it, only beats do (the sweeper's period is
        // the lease's, far past this test). No request is sent meanwhile, so nothing but the
        // beats' own ends wakes the loop to beat again.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let settled = dialled.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let beaten = dialled.load(Ordering::SeqCst);
        assert!(
            beaten >= settled + 3,
            "several failed beats in 300 ms of a 20 ms period: {settled} -> {beaten} dials"
        );
        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(
            label, "online",
            "a failed heartbeat is logged, never a backend swap: the refresher owns that"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        hangups.abort();
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
        assert!(
            !go_offline(&mut backend, &mut refresher, &mut seen, &err),
            "a memory backend never swaps"
        );
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

    /// The box probe is named like every other request, and a build with no runtime says so
    /// rather than dropping the reply (MOD-7 D11).
    #[tokio::test]
    async fn probe_box_is_named_and_refused_without_a_runtime() {
        assert_eq!(StoreRequest::ProbeBox.name(), "probe_box");
        match serve(&demo(), &StoreRequest::ProbeBox).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, "probe_box");
                assert_eq!(message, "no agent runtime in this build");
            }
            other => panic!("a box probe with no runtime is refused, not served: {other:?}"),
        }
    }

    /// `R-NF-3` for the box probe: its version children may each take `version_timeout`, and the
    /// loop must serve everything else while they run. The fake tool sleeps, so the probe is
    /// provably still in flight when `BoxInfo` is answered (H-7: a fake `PATH`, fixed hardware).
    #[cfg(unix)]
    #[tokio::test]
    async fn the_loop_answers_box_info_while_a_box_probe_is_in_flight() {
        use crate::agent_worker::tests::{fake_env, fake_hardware, never_probed, script};

        let tmp = tempfile::tempdir().expect("temp box");
        script(
            tmp.path(),
            "cmake",
            "/bin/sleep 1; echo 'cmake version 3.28.0'",
        );
        let runtime = AgentRuntime::new(htui_agent::registry::DriverFactory::production())
            .with_probe_env(fake_env(tmp.path()), fake_hardware());
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn_with(
            Started::detached(Backend::memory(never_probed().await)),
            req_rx,
            rep_tx,
            runtime,
        );

        for (seq, request) in [(1, StoreRequest::ProbeBox), (2, StoreRequest::BoxInfo)] {
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
            "the loop is free the instant the box probe is deferred: {:?}",
            first.reply
        );
        assert!(matches!(first.reply, StoreReply::BoxInfo(Some(_))));
        let second = rep_rx.recv().await.expect("the box probe answers itself");
        assert_eq!(second.seq, 1);
        let StoreReply::BoxProbed(report) = &second.reply else {
            panic!("the box probe answers BoxProbed: {:?}", second.reply)
        };
        assert_eq!(report.box_failed, None);
        assert_eq!(
            report.tools, 1,
            "the sleeping tool answered within its timeout"
        );

        drop(req_tx);
        let _ = worker.await;
    }

    /// MOD-7 D3 over the loop's other bootstrap: a store that connected to a pending schema
    /// registers only when `ApplyMigrations` runs, and the loop then writes a minted id back to
    /// `box.toml` through `connect::persist_registration`, as `try_connect` does after a connect
    /// over an up-to-date schema.
    ///
    /// The planted row is `htui-store`'s `apply_migrations_then_persist_writes_a_minted_id_back`
    /// one, with the same early return on a machine with no readable identity.
    #[tokio::test]
    async fn apply_migrations_writes_a_minted_box_id_back_to_box_toml() {
        if htui_store::identity::machine_fingerprint().await.is_none() {
            return;
        }
        let Some(db) = htui_store::testkit::bare_db().await else {
            return;
        };
        htui_store::MIGRATOR
            .run(&db.pool)
            .await
            .expect("apply the schema under the store");
        db.store.seed_if_empty().await.expect("seed the app_user");
        sqlx::query(
            "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version, \
                              machine_fingerprint) \
             VALUES ($1, (SELECT id FROM app_user ORDER BY created_at, id LIMIT 1), 'elsewhere', \
                     'linux', '', 'x86_64', '0.0.0', repeat('a', 64))",
        )
        .bind(db.identity.box_id.as_uuid())
        .execute(&db.pool)
        .await
        .expect("plant the other machine's row under the box.toml id");

        let mut started = Started::detached(demo());
        started.connect = Some(connect::ConnectContext {
            config_root: db.config_root.clone(),
            connect_timeout: std::time::Duration::from_secs(5),
            offline: true,
            registered: connect::Registered::default(),
        });
        let events = started.events_tx.clone();
        let (tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn(started, req_rx, rep_tx);
        events
            .send((
                connect::LAUNCH_GENERATION,
                ConnEvent::MigrationsPending(db.store.clone(), 5),
            ))
            .await
            .expect("the worker is listening");
        // The event and the requests race in the loop's `select!`: ask until the store is held.
        loop {
            let StoreReply::StoreState {
                migrations_pending, ..
            } = round_trip(&tx, &mut rep_rx, StoreRequest::StoreState).await
            else {
                panic!("wrong reply variant")
            };
            if migrations_pending == Some(5) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let reply = round_trip(&tx, &mut rep_rx, StoreRequest::ApplyMigrations).await;
        assert!(
            matches!(reply, StoreReply::MigrationsApplied { applied: 5 }),
            "{reply:?}"
        );
        let rewritten =
            htui_store::identity::load_or_mint(&db.config_root).expect("re-read box.toml");
        assert_ne!(
            rewritten.box_id, db.identity.box_id,
            "box.toml holds the id this machine was minted, not the copied one"
        );

        drop(tx);
        let _ = worker.await;
        db.drop_db().await;
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
        WriteStore::upsert_agent(&store, &agent, None)
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
        assert_eq!(
            StoreRequest::AuthDeliver {
                url: RedirectUrl::new("http://127.0.0.1:1/".to_owned())
            }
            .name(),
            "auth_deliver"
        );
        assert_eq!(
            StoreRequest::ChatFollow {
                step_id: StepId::new()
            }
            .name(),
            "chat_follow"
        );
    }

    /// The three box requests are named exactly as `box_settings::REQUEST_NAMES` lists them, so
    /// the section's `Failed` match and the worker cannot drift apart (MOD-7 milestone 2, D46;
    /// MOD-51 D4).
    #[test]
    fn box_requests_are_named_as_box_settings_lists_them() {
        assert_eq!(
            [
                StoreRequest::Boxes.name(),
                StoreRequest::EditBox {
                    box_id: BoxId::new(),
                    expected: 0,
                    edit: BoxEdit::default(),
                }
                .name(),
                StoreRequest::SetProbeSpec {
                    overlay: None,
                    expected: None,
                }
                .name(),
            ],
            box_settings::REQUEST_NAMES
        );
    }

    /// The three agent registry requests are named exactly as `agent_settings::REQUEST_NAMES`
    /// lists them, so the section's `Failed` match and the worker cannot drift apart (MOD-23
    /// D241).
    #[test]
    fn agent_requests_are_named_as_agent_settings_lists_them() {
        let draft = agent_settings::draft_of(&htui_core::model::agent::seed_rows(Utc::now())[0]);
        assert_eq!(
            [
                StoreRequest::CreateAgent {
                    name: "agent-x".to_owned(),
                    draft: draft.clone(),
                    settings_from: None,
                }
                .name(),
                StoreRequest::EditAgent {
                    agent_id: AgentId::new(),
                    expected: Utc::now(),
                    draft,
                }
                .name(),
                StoreRequest::SetAgentOnBox {
                    agent_id: AgentId::new(),
                    enabled: false,
                }
                .name(),
            ],
            agent_settings::REQUEST_NAMES
        );
    }

    /// MOD-66 D7, blueprint B12: the tool-paths write is named by its own const, outside
    /// `agent_settings::REQUEST_NAMES`, and a build with no agent runtime refuses it by name. This
    /// pins `try_serve`'s arm.
    #[tokio::test]
    async fn set_tool_paths_is_named_and_refused_without_a_runtime() {
        let request = StoreRequest::SetToolPaths {
            agent_id: AgentId::new(),
            paths: BTreeMap::new(),
        };
        assert_eq!(request.name(), agent_settings::SET_TOOL_PATHS);
        assert_eq!(agent_settings::SET_TOOL_PATHS, "set_tool_paths");
        match serve(&demo(), &request).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, "set_tool_paths");
                assert_eq!(message, "no agent runtime in this build");
            }
            other => panic!("a tool-paths write with no runtime is refused, not served: {other:?}"),
        }
    }

    /// MOD-66 D7, blueprint H-4 and B13: the store loop's runtime list is a wildcard match, so a
    /// forgotten line would compile and fall through to `try_serve`'s "no agent runtime in this
    /// build". The loop hands the request to the runtime instead, which answers it exactly once
    /// at its address, with its own sentence: "not a chat request" before the handler lands (T2),
    /// the handler's refusal of an unknown agent after it (T3).
    #[tokio::test]
    async fn the_loop_hands_set_tool_paths_to_the_agent_runtime() {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn_with(
            Started::detached(Backend::memory(MemStore::demo())),
            req_rx,
            rep_tx,
            AgentRuntime::new(htui_agent::registry::DriverFactory::new()),
        );
        req_tx
            .send(RequestEnvelope {
                seq: 7,
                origin: Origin::App,
                request: StoreRequest::SetToolPaths {
                    agent_id: AgentId::new(),
                    paths: BTreeMap::new(),
                },
            })
            .expect("the worker is alive");
        drop(req_tx);
        tokio::time::timeout(std::time::Duration::from_secs(5), worker)
            .await
            .expect("the worker stops with its channel")
            .expect("the worker does not panic");

        // The loop has stopped, so everything it answered is queued.
        let mut replies = Vec::new();
        while let Ok(envelope) = rep_rx.try_recv() {
            replies.push(envelope);
        }
        let [envelope] = replies.as_slice() else {
            panic!("exactly one reply: {replies:?}")
        };
        assert_eq!(envelope.seq, 7);
        match &envelope.reply {
            StoreReply::Failed { request, message } => {
                assert_eq!(*request, "set_tool_paths");
                assert_ne!(
                    message, "no agent runtime in this build",
                    "the loop routed the request to try_serve, not to the agent runtime"
                );
                assert!(
                    message.contains("not found"),
                    "the handler's own refusal of an unknown agent (review N2): {message}"
                );
            }
            other => panic!("an unknown agent is refused: {other:?}"),
        }
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

    /// MOD-22 D266: the pasted address carries an authorization code, so neither the request's
    /// `Debug` nor its envelope's prints it.
    #[test]
    fn an_auth_deliver_request_debugs_without_its_url() {
        let pasted = "http://127.0.0.1:39879/?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e";
        let request = StoreRequest::AuthDeliver {
            url: RedirectUrl::new(pasted.to_owned()),
        };
        let envelope = RequestEnvelope {
            seq: 7,
            origin: Origin::App,
            request: request.clone(),
        };
        for shown in [
            format!("{request:?}"),
            format!("{request:#?}"),
            format!("{envelope:?}"),
            format!("{envelope:#?}"),
        ] {
            assert!(shown.contains("RedirectUrl(<redacted>)"), "{shown}");
            assert!(!shown.contains("CODE-SENTINEL-4f1c"), "the code leaked");
            assert!(!shown.contains("STATE-SENTINEL-9a2e"), "the state leaked");
        }
    }

    /// MOD-22's delivery joins MOD-21's four: a build with no agent runtime refuses it by name,
    /// exactly once, rather than dropping it (D283).
    #[tokio::test]
    async fn auth_deliver_without_a_runtime_is_refused_by_name() {
        let request = StoreRequest::AuthDeliver {
            url: RedirectUrl::new("http://127.0.0.1:1/?code=c&state=s".to_owned()),
        };
        match serve(&demo(), &request).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, "auth_deliver");
                assert_eq!(message, "no agent runtime in this build");
            }
            other => panic!("a delivery with no runtime is refused, not served: {other:?}"),
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

    /// Blueprint §8.10, §10.5, D181: a promotion's engine writes are answered `Orch(Promoted)`, and
    /// the runtime's `Attach` event reaches the loop, which binds the Chat tab's runtime to the
    /// promoted step — `ChatAccepted` on that step, both at the request's `seq` and origin.
    #[tokio::test]
    async fn a_promotion_reaches_the_attach_hand_off() {
        use crate::run_worker::tests::{Fixture, start_run, step_at};
        use crate::run_worker::{OrchReply, OrchRequest};
        use htui_agent::conformance::{Script, ScriptEvent};
        use htui_agent::event::{DoneEvent, DriverEvent};

        let fixture = Fixture::new().await;
        // The chat's own transport: the fixture's scripted row is `acp`, and the chat runtime
        // reaches it by row data like the walk does.
        let adapter = htui_agent::fake::FakeAdapter::new();
        adapter.load(Script::one_turn(vec![ScriptEvent::Emit(
            DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            }),
        )]));
        let mut factory = htui_agent::registry::DriverFactory::new();
        factory.register("acp", Box::new(adapter));

        let (requests, requests_rx) = mpsc::unbounded_channel();
        let (replies_tx, mut replies) = mpsc::unbounded_channel();
        let _worker = spawn_with_runtimes(
            Started::detached(Backend::memory(fixture.store.clone())),
            requests_rx,
            replies_tx,
            AgentRuntime::new(factory).with_grace(std::time::Duration::ZERO),
            fixture.runtime(),
        );
        let send = |seq, origin, request| {
            requests
                .send(RequestEnvelope {
                    seq,
                    origin,
                    request,
                })
                .expect("the worker is running");
        };
        let mut seen: Vec<ReplyEnvelope> = Vec::new();
        let mut next_at = async |seq: Seq| loop {
            if let Some(at) = seen.iter().position(|envelope| {
                envelope.seq == seq && !matches!(envelope.reply, StoreReply::RunStream(_))
            }) {
                return seen.remove(at);
            }
            let envelope = tokio::time::timeout(std::time::Duration::from_secs(20), replies.recv())
                .await
                .unwrap_or_else(|_| panic!("no reply at seq {seq}"))
                .expect("the worker is running");
            seen.push(envelope);
        };

        send(1, Origin::App, start_run(ids::HTUI_ANA_2));
        let started = next_at(1).await;
        let StoreReply::Orch(OrchReply::Done(outcome)) = started.reply else {
            panic!("a start answers Done: {:?}", started.reply)
        };
        let htui_orch::CommandOutcome::Started { run, .. } = *outcome else {
            panic!("a start answers Started")
        };
        let step = step_at(&fixture, run, 0).await;

        let chat = Origin::Tab(TabId("chat"));
        send(
            2,
            chat.clone(),
            StoreRequest::Orch(OrchRequest::Command(htui_orch::Command::PromoteStep {
                run,
                step: step.id,
                chat_open: false,
            })),
        );

        let first = next_at(2).await;
        assert_eq!(first.origin, chat);
        assert!(
            matches!(first.reply, StoreReply::Orch(OrchReply::Promoted { step: promoted, run: of, .. })
                if promoted == step.id && of == run),
            "{:?}",
            first.reply
        );
        let second = next_at(2).await;
        assert_eq!(second.origin, chat);
        assert!(
            matches!(second.reply, StoreReply::ChatAccepted { step_id, .. } if step_id == step.id),
            "the chat is bound to the promoted step: {:?}",
            second.reply
        );
    }

    /// MOD-69 plan D4: offline the waiting list reads the mirror and leaves the permissions
    /// unknown (they are not mirrored), rather than failing the whole reply.
    #[tokio::test]
    async fn serve_waiting_offline_leaves_permissions_unknown() {
        // A non-`Memory` backend: nothing here may reach the developer's own OS keyring.
        let _keyring = htui_store::testkit::mock_keyring().await;
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-waiting-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline {
            cache,
            since: Some(Utc::now()),
        };
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        match try_serve(&backend, &StoreRequest::Waiting { scope }).await {
            Ok(StoreReply::Waiting {
                scope: answered,
                view,
            }) => {
                assert_eq!(answered.workspace_id, ids::WORKSPACE_PLATFORM);
                assert!(!view.permissions_known, "offline permissions are unknown");
                assert!(
                    view.offline,
                    "the reply says it was read offline (review L3)"
                );
                assert!(view.rows.is_empty(), "an empty mirror owes nothing");
                assert_eq!(view.working, 0);
            }
            other => panic!("an offline waiting read is a view: {other:?}"),
        }
    }

    /// MOD-69 review R2 (L4): the `Waiting` arm itself, over a backend with a writer whose
    /// permission read fails. Anything but `Unreachable` still answers the candidate rows with the
    /// permissions unknown (and not offline); `Unreachable` fails the reply, so `go_offline` fires.
    #[tokio::test]
    async fn a_waiting_read_survives_a_failing_permission_read_but_not_an_unreachable_one() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let StoreReply::Waiting { view: expected, .. } = serve(
            &backend,
            &StoreRequest::Waiting {
                scope: scope.clone(),
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert!(!expected.rows.is_empty(), "the demo platform owes rows");

        match serve_waiting(&backend, &scope, async |_, _| {
            Err(StoreError::Backend("relation gone".into()))
        })
        .await
        {
            Ok(StoreReply::Waiting {
                scope: answered,
                view,
            }) => {
                assert_eq!(answered, scope);
                assert!(
                    !view.permissions_known,
                    "the failed read leaves them unknown"
                );
                assert!(!view.offline, "a backend with a writer is not offline");
                assert_eq!(view.rows, expected.rows, "the candidate rows survive");
                assert_eq!(view.working, expected.working);
            }
            other => panic!("a failed permission read degrades: {other:?}"),
        }

        assert!(matches!(
            serve_waiting(&backend, &scope, async |_, _| {
                Err(StoreError::Unreachable("down".into()))
            })
            .await,
            Err(StoreError::Unreachable(_))
        ));
    }

    /// MOD-69 review L4: a permission read that fails for any reason but `Unreachable` leaves the
    /// permissions unknown rather than failing the whole `Waiting` reply; `Unreachable` still
    /// propagates, so `go_offline` drops the backend.
    #[test]
    fn a_failing_permission_read_degrades_and_unreachable_propagates() {
        assert_eq!(
            permissions_or_unknown(Ok(Vec::new())).expect("a read"),
            Some(Vec::new())
        );
        assert_eq!(
            permissions_or_unknown(Err(StoreError::Backend("relation gone".into())))
                .expect("degraded, not failed"),
            None,
            "permissions_known = false"
        );
        assert!(matches!(
            permissions_or_unknown(Err(StoreError::Unreachable("down".into()))),
            Err(StoreError::Unreachable(_))
        ));
    }

    /// MOD-42 plan D14, OQ-4: offline the Runs pane's relay read is an **empty** view, never a
    /// `Failed` (the status line) or an `Unreachable` (`go_offline`), and an answer is refused with
    /// `DATABASE_UNREACHABLE` before anything is sent.
    #[tokio::test]
    async fn relay_reads_are_empty_offline_and_answers_are_refused() {
        // A non-`Memory` backend: nothing here may reach the developer's own OS keyring.
        let _keyring = htui_store::testkit::mock_keyring().await;
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-relay-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: Some(Utc::now()),
        };

        let item = ids::HTUI_FEAT_3;
        match try_serve(&backend, &StoreRequest::RelayView { item }).await {
            Ok(StoreReply::RelayView { item: asked, view }) => {
                assert_eq!(asked, item);
                assert_eq!(*view, RelayView::default(), "offline the view is empty");
            }
            other => panic!("an offline relay read is an empty view: {other:?}"),
        }
        assert_eq!(StoreRequest::RelayView { item }.name(), "relay_view");

        let answer = StoreRequest::AnswerPermission {
            permission: PermissionId::new(),
            option_id: "allow".to_owned(),
        };
        assert_eq!(answer.name(), "answer_permission");
        match try_serve(&backend, &answer).await {
            Err(StoreError::Unreachable(message)) => assert_eq!(message, DATABASE_UNREACHABLE),
            other => panic!("an offline answer is refused before anything is sent: {other:?}"),
        }

        cache.close().await;
    }

    /// R1 ADV-1: an import that lost the store keeps its report (`Ok(PersonaImports)`) and still
    /// tells the loop the store is gone, so an `Online` backend drops onto the mirror as it does
    /// for any served request's `Unreachable`; another failure is not a loss.
    #[test]
    fn an_import_that_lost_the_store_still_reports_the_loss() {
        let lost = StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned());
        let other = StoreError::Backend("not a loss".to_owned());
        let import = |personas| {
            Ok(StoreReply::PersonaImports(Box::new(PersonaImports {
                personas,
                report: Vec::new(),
            })))
        };

        assert_eq!(lost_the_store(&import(Err(lost.clone()))), Some(&lost));
        assert_eq!(lost_the_store(&import(Err(other.clone()))), None);
        assert_eq!(lost_the_store(&import(Ok(Vec::new()))), None);
        assert_eq!(lost_the_store(&Err(lost.clone())), Some(&lost));
        assert_eq!(lost_the_store(&Err(other)), None);
        assert_eq!(lost_the_store(&Ok(StoreReply::Personas(Vec::new()))), None);
    }

    /// MOD-37 M4 D3 (R-46), review M1: a read that fails `Unreachable` swaps to the mirror but
    /// preempts no walk - the same error covers sqlx's `PoolTimedOut`, local load with the server
    /// up. A walk's child token held by the case stands in for the walk, which a lazy `Online`
    /// backend could not claim; it is still live after the swap.
    #[tokio::test]
    async fn an_unreachable_read_goes_offline_but_leaves_the_walks() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-swap-preempts", 1)
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
        let backend = Backend::Online {
            pg,
            cache: cache.clone(),
        };
        let runs = crate::run_worker::production_for(&backend);
        let probe = htui_worker::testing::probe(&runs);
        let run = htui_core::model::RunId::new();
        let _walk = probe.walk_child(run);

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(backend);
        started.reconnect = None;
        let worker = spawn_with_runtimes(started, req_rx, rep_tx, AgentRuntime::production(), runs);

        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "online", "nothing has asked the server yet");
        assert!(probe.has_parent(run), "the walk is live before the swap");

        let reply = round_trip(&req_tx, &mut rep_rx, StoreRequest::Workspaces).await;
        assert!(
            matches!(
                &reply,
                StoreReply::Failed {
                    request: "workspaces",
                    ..
                }
            ),
            "{reply:?}"
        );
        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert!(label.starts_with("offline · "), "{label}");
        assert!(
            probe.has_parent(run),
            "a read's swap leaves the walk alone: its run's parent is still live"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
    }

    /// MOD-37 M4 D3 (R-46): the refresher's arm of the swap preempts too. A dial's `Online` over
    /// a lazy store nothing listens behind arms a refresher whose first pass, at once, reports the
    /// server unreachable; nothing else in the case reads through the store.
    #[tokio::test]
    async fn a_refresher_that_loses_the_server_preempts_every_live_walk() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-refresher-preempts", 1)
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
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        let runs = crate::run_worker::production_for(&backend);
        let probe = htui_worker::testing::probe(&runs);
        let run = htui_core::model::RunId::new();
        let _walk = probe.walk_child(run);

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(backend);
        started.reconnect = None;
        let dialled = started.events_tx.clone();
        let worker = spawn_with_runtimes(started, req_rx, rep_tx, AgentRuntime::production(), runs);
        dialled
            .send((0, ConnEvent::Online(pg)))
            .await
            .expect("the worker is alive");

        // `connecting` before the dial lands, `online` until the refresher's pass fails.
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let StoreReply::StoreState { label, .. } =
                    round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
                else {
                    panic!("wrong reply variant")
                };
                if label.starts_with("offline · ") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the refresher reports the lost server");
        assert!(
            !probe.has_parent(run),
            "the swap preempted the walk: its run's parent is cancelled and gone"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
    }

    /// MOD-37 M4 D3: `go_offline` reports the swap, and only the first notice of it is one.
    #[tokio::test]
    async fn go_offline_reports_a_swap_only_from_online() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-go-offline-swap", 1)
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
        let mut backend = Backend::Online {
            pg,
            cache: cache.clone(),
        };
        let err = StoreError::Unreachable("gone".to_owned());
        let mut refresher = None;
        let mut seen = None;

        assert!(
            go_offline(&mut backend, &mut refresher, &mut seen, &err),
            "an Online backend swaps"
        );
        assert!(
            backend.label().starts_with("offline · "),
            "{}",
            backend.label()
        );
        assert!(
            !go_offline(&mut backend, &mut refresher, &mut seen, &err),
            "a second notice of the same drop is no swap"
        );

        cache.close().await;
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

            assert!(
                !go_offline(&mut backend, &mut refresher, &mut seen, &err),
                "{was} is not Online: no swap"
            );

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

    /// A scope for the tool host tests below: no item, no fence, the ACP transport.
    fn tool_scope() -> htui_orch::tools::ToolScope {
        htui_orch::tools::ToolScope {
            run_id: htui_core::model::RunId::new(),
            step_id: StepId::new(),
            project_id: ids::PROJECT_HTUI,
            item_id: None,
            box_id: ids::BOX,
            user: ids::USER,
            fence: htui_core::store::StepFence::Unleased,
            output_kind: None,
            hostname: htui_core::prompt::render::HostnameLine::Omitted,
            command_queue: false,
            cwd: std::env::temp_dir(),
            transport: htui_core::model::Transport::Acp,
        }
    }

    /// MOD-11 T6 ADV-1: a backend that went offline is not handed to the tool host. `go_offline`
    /// does not preempt a walk, which keeps its own `Online` clone across the blip, so the walk's
    /// next session must still open on the server it was claimed on (B-2) rather than answer
    /// `Offline` and fail the run with `agent spawn failed`.
    #[tokio::test]
    async fn an_offline_backend_leaves_the_tool_host_on_the_last_writable_one() {
        use htui_orch::tools::ToolHost as _;

        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-host-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let host = htui_mcp::McpHost::new(demo()).expect("a tool host");

        host_the_backend(
            &host,
            &Backend::Offline {
                cache: cache.clone(),
                since: Some(Utc::now()),
            },
        );
        let lease = host.open(tool_scope()).unwrap_or_else(|err| {
            panic!("a session opened across an offline blip keeps the last writable backend: {err}")
        });

        drop(lease);
        host.close();
        cache.close().await;
    }

    /// MOD-11 T6 ADV-2: the run runtime's shutdown does not close the host it shares with the
    /// chat runtime. The loop joins the two shutdowns, so with no walk left the run runtime is
    /// done while the chats are still inside their cancel window; a chat agent's last tool call
    /// (a `note_add` flushed while it winds down) must still find its session and the listener.
    #[tokio::test]
    async fn the_run_runtime_shutdown_leaves_the_shared_tool_host_open() {
        use htui_orch::tools::ToolHost as _;

        let host = Arc::new(htui_mcp::McpHost::new(demo()).expect("a tool host"));
        let chat = host.open(tool_scope()).expect("a chat's lease");
        let (_runtime, mut runs) =
            host_the_runtimes(AgentRuntime::production(), RunRuntime::production(), &host);

        runs.shutdown(std::time::Duration::ZERO).await;

        assert!(
            host.address().is_some(),
            "the listener outlives the run runtime's shutdown"
        );
        assert!(
            host.client(&chat.spec.env[htui_mcp::ENV_TOKEN]).is_ok(),
            "the chat's session outlives the run runtime's shutdown"
        );
        drop(chat);
        host.close();
    }

    /// MOD-11 T6 ADV-2: the loop closes the shared tool host itself once the UI is gone: its
    /// listener goes and every session it still served ends.
    #[tokio::test]
    async fn the_loop_closes_the_shared_tool_host_after_both_runtimes() {
        use htui_orch::tools::ToolHost as _;

        let host = Arc::new(htui_mcp::McpHost::new(demo()).expect("a tool host"));
        let lease = host.open(tool_scope()).expect("a lease");
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, _rep_rx) = mpsc::unbounded_channel();
        let worker = spawn_hosted(
            Started::detached(demo()),
            req_rx,
            rep_tx,
            AgentRuntime::production(),
            Some(Arc::clone(&host)),
            None,
        );

        drop(req_tx);
        worker.await.expect("the loop ends");

        assert!(host.address().is_none(), "the listener is gone");
        assert!(
            host.client(&lease.spec.env[htui_mcp::ENV_TOKEN]).is_err(),
            "the session ended with the host"
        );
    }
}
