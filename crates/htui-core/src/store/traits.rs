//! The store seam of `docs/ANA-9.md` §6.1, quoted verbatim.
//!
//! Method names, parameter names, parameter order and the return types are copied from §6.1, not
//! paraphrased (plan V2). `PgStore: WriteStore`, `CacheStore: ReadStore` only,
//! [`MemStore`](crate::store::MemStore)`: WriteStore`. Nothing is added to these two traits in
//! MOD-1.
//!
//! **MOD-2 (plan D3)** adds six [`WriteStore`] methods and nothing to [`ReadStore`]: the four of
//! `docs/ANA-4.md` §4.1 that the session recorder writes through, plus the chat-run pair of the
//! MOD-2 PRD. They are the *whole* store seam MOD-2 needs before its milestone 9, so milestones 2
//! to 8 do not reopen this file. Milestone 7 reopens it once, for a seventh:
//! [`WriteStore::set_agent_box_quota`], the two-column latch of `docs/ANA-4.md` §7 that plan D67
//! keeps out of `upsert_agent_box` and plan D74 makes those two columns' only writer. The registry
//! read that goes with them, `agents()`, is **not**
//! here: `agent` and `agent_box` are not mirrored (`docs/ANA-9.md` §4.4), so it is inherent on
//! `MemStore` / `PgStore` and dispatched by `Backend`, following the `workspaces` / `box_info` /
//! `active_runs` / `projects` precedent.
//!
//! **MOD-2 milestone 9** reopens it a second time, and this time [`ReadStore`] grows too: the
//! prompt assembler of `docs/ANA-5.md` needs a document's **body**, the latest document per kind,
//! the amended §7.3 upstream walk and a project's `settings`. All four are reads of tables the
//! cache mirrors, which is ANA-2 §8's test for a trait method rather than an inherent one — and
//! that is what lets `CacheStore` answer them and `store::conformance`'s `READ_CASES` run over the
//! mirror (plan D96). [`WriteStore`] gains one: [`WriteStore::set_step_prompt`], the pre-flight
//! audit row. The prompt inputs that are **not** mirrored — `prompt_template`, `skill`,
//! `skill_version`, `skill_binding`, `box_tool` — stay inherent on `MemStore` / `PgStore` for the
//! same reason `agents()` does.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{
    Agent, AgentBox, AgentId, BoxId, ChatRunSpec, Document, DocumentHead, DocumentId, Item,
    ItemFilter, ItemId, ItemKind, ItemKindId, ItemKindPatch, ItemPatch, ItemRevision, ItemSummary,
    LinkGraph, NewItem, NewItemKind, NewProject, NewRepo, NewStepGraph, NewWorkspace, Note,
    PhaseId, PhasePatch, Project, ProjectId, ProjectPatch, PromptScope, Repo, RepoBoxPath, RepoId,
    RepoPatch, RunId, RunStatus, RunSummary, Scope, SessionEvent, Status, StepGraph, StepGraphId,
    StepGraphPatch, StepGraphPhase, StepId, UpstreamEntry, Workspace, WorkspaceBoxPath,
    WorkspaceId, WorkspacePatch, WorkspaceProject,
};
use crate::prompt::settings::{Rungs, SettingKey};
use crate::store::error::Result;

/// The hop ceiling of [`ReadStore::upstream_summaries`], the amended §7.3 upstream walk
/// (`docs/ANA-5.md` §4.3).
///
/// `R-PRM-1` says "one to two hops", so `2` is the ceiling and `0` is answered without a round
/// trip: the anchor term of the SQL backends' recursive CTE has no depth guard of its own, and
/// unlike [`ReadStore::links`]`(id, 0)` the root is the step's own item and is never an upstream
/// entry.
///
/// It lives beside the trait rather than in a backend because all three backends clamp and they
/// must clamp alike — a caller that did not sanitise `hops` gets the same answer from
/// `MemStore`, `PgStore` and `CacheStore` (T68, F-52 review, L2: the memory backend used to spell
/// it as a literal `2` while `htui-store` had the named constant, so the two could drift without
/// anything failing).
pub const MAX_UPSTREAM_HOPS: u8 = 2;

/// Everything a view can ask for. Implemented by every backend, online or offline.
#[allow(async_fn_in_trait)] // D2 / plan V3: rustc 1.98 warns on async fn in public traits; the
// signatures are ANA-9 §6.1 verbatim and `Backend` is concrete, so
// Send-ness is inferred at the call site instead of specified.
pub trait ReadStore: Send + Sync {
    /// Items of the scope that match the filter.
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>>;
    /// One item with its body.
    async fn item(&self, id: ItemId) -> Result<Option<Item>>;
    /// The link neighbourhood of an item, up to `hops` hops.
    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph>;
    /// The item's documents without their bodies.
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>>;
    /// The item's notes.
    async fn notes(&self, id: ItemId) -> Result<Vec<Note>>;
    /// The item's runs with their steps.
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>>;
    /// The replay log of one step.
    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>>; // None = not cached

    /// One document **with its body**, or `None` when no row has that id (`docs/ANA-5.md` §8).
    ///
    /// [`documents`](ReadStore::documents) answers heads, which is what a list needs; the prompt
    /// assembler needs the text. On `ReadStore` rather than inherent because `document` is
    /// mirrored body and all (`cache_migrations/0001_mirror.sql:97-101`), so every backend can
    /// answer it.
    async fn document(&self, id: DocumentId) -> Result<Option<Document>>;

    /// The **latest version of each `kind`**, in `kinds` order, kinds the item has no row for
    /// omitted. An empty `kinds` means every kind the item has, in kind byte order.
    ///
    /// The order is the caller's because `docs/ANA-5.md` §4.7 rule 3 renders documents in the
    /// phase's `input_kinds` order and the prompt digest is a function of that order; sorting here
    /// would make the store the authority on something the graph owns. Byte order for the empty
    /// case for the reason [`UpstreamEntry::sort_canonical`](crate::model::UpstreamEntry::sort_canonical)
    /// gives: Postgres would otherwise order by collation and the mirror by bytes.
    ///
    /// This is **not** ANA-2 §8's resolver: it prefers no run's output and excludes no loser,
    /// because both need `run_step` rows MOD-4 owns. MOD-4 layers that on top (blueprint P-12).
    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>>;

    /// Upstream items reached over `blocked_by` and `origin` edges, up to `hops` (clamped to
    /// `1..=2`; `0` returns nothing), **one row per item at its minimum depth**, classified
    /// against `scope` (`R-PRM-1`, `R-PRM-2`).
    ///
    /// `docs/ANA-9.md` §7.3 as amended by `docs/ANA-5.md` §4.3, returned in canonical order
    /// ([`UpstreamEntry::sort_canonical`](crate::model::UpstreamEntry::sort_canonical)) so the
    /// three backends hand the assembler the same bytes.
    ///
    /// Directed, unlike [`links`](ReadStore::links): only `to_item_id` is followed, and only those
    /// two link kinds. `in_scope` and `summary.is_some()` are separable facts — an in-scope item
    /// with no summary renders as `no summary yet`, an out-of-scope one as the `R-PRM-2` stub
    /// whose summary is not read at all.
    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>>;

    /// One project row with its `settings`, or `None` when no row has that id
    /// (`docs/ANA-5.md` §8).
    ///
    /// `project.settings` is mirrored (`cache_migrations/0001_mirror.sql:57-61`), which is why
    /// this is a trait method while
    /// [`MemStore::project_settings`](crate::store::MemStore::project_settings) — the column
    /// alone, for the per-run token cap — stays inherent beside it.
    async fn project(&self, id: ProjectId) -> Result<Option<Project>>;
}

/// Everything a write path needs.
///
/// It used to be implemented only by a store that can reach Postgres. Since MOD-2 milestone 4
/// (plan D34) `htui-store`'s offline sink implements it too, writing the `session_event` rows to
/// a JSON-lines buffer and answering
/// [`StoreError::Unreachable`](crate::store::StoreError::Unreachable) for everything the buffer
/// cannot hold — so "an offline write is a compile error" is now narrower and still true where it
/// counts: nothing reaches **Postgres** except through a store that has a connection, and the
/// read-only mirror still does not implement this trait at all.
#[allow(async_fn_in_trait)] // D2 / plan V3, as above.
pub trait WriteStore: ReadStore {
    /// Mints an item: counter upsert, key assembly and revision 1 in one transaction (§7.1, §4.1).
    async fn mint_item(&self, new: NewItem) -> Result<Item>;
    /// Compare-and-set edit on `item.version` (§7.2, §4.2).
    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome>;
    /// Compare-and-set status move; never bumps `version`, never writes a revision (§4.2).
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool>;

    /// Appends session events, skipping any `(run_step_id, seq)` already stored, and answers how
    /// many rows were actually inserted (`docs/ANA-4.md` §4.1, `docs/ANA-9.md` §4.3).
    ///
    /// One statement: either every new row lands or none does, so a batch holding an event for a
    /// step that does not exist writes nothing at all. Idempotence is the primary key's, which is
    /// what makes replaying an offline buffer safe.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when an event names a
    /// `run_step` that does not exist or a `kind` / `role` outside the §4.3 `CHECK` lists.
    async fn append_events(&self, events: &[SessionEvent]) -> Result<usize>;

    /// Writes `run_step.usage`, and `run_step.prompt_digest` when `prompt_digest` is `Some`
    /// (`docs/ANA-4.md` §4.1; the digest parameter survives to milestone 9, plan D15(b)).
    ///
    /// `None` leaves the stored digest as it is rather than clearing it: the recorder computes the
    /// digest once, at the prompt, and every later usage write for the same step passes `None`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) when the step does not exist.
    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()>;

    /// Inserts or updates one `agent` row, keyed by `agent.id` (`docs/ANA-4.md` §4.1, §5.7).
    ///
    /// `agent.created_at` is written on the insert and never rewritten; `updated_at` belongs to
    /// the migration's `BEFORE UPDATE` trigger.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when another id already
    /// holds the name (`agent.name` is `UNIQUE`).
    async fn upsert_agent(&self, agent: &Agent) -> Result<()>;

    /// Inserts or updates one `agent_box` row, keyed by `(agent_id, box_id)` (`docs/ANA-4.md`
    /// §4.1, §5.7).
    ///
    /// `row.quota` and `row.quota_at` are **not part of what this writes** (MOD-2 plan D74). On
    /// the insert they land as `None`; on the update the stored pair is left exactly as it stood.
    /// [`set_agent_box_quota`](WriteStore::set_agent_box_quota) is their only writer, so a probe
    /// that read a row cannot hand a stale allowance back over a latch that landed after it - the
    /// lost update D74 removes.
    ///
    /// An [`AgentBox`] carrying either field is therefore neither an error nor a write. That is
    /// the one sharp edge the design keeps, and `store::conformance`'s
    /// `upsert_agent_box_cannot_write_quota` is what pins it.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the agent or the box
    /// does not exist.
    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()>;

    /// Writes `agent_box.quota` and `quota_at` of one **existing** row and nothing else — never
    /// `probe`, `enabled`, `version` or `path` (MOD-2 plan D67; `docs/ANA-4.md` §7's passive
    /// latch).
    ///
    /// Narrow on purpose: the latch runs inside a chat while a re-probe of the same row may be
    /// running beside it (plan D55/D60), and two writers of one row must not be one statement
    /// wide. No insert: a row that has never been probed has no columns to latch into.
    ///
    /// Since plan D74 this is also the **only** writer of the two columns —
    /// [`upsert_agent_box`](WriteStore::upsert_agent_box) cannot set or clear either — so a latch
    /// cannot be discarded by a re-probe that read the row before it landed.
    ///
    /// # Nothing can clear them yet (review L-6)
    ///
    /// The parameters are `Value` and `DateTime`, not `Option`s, so the single writer of the two
    /// columns can write them and **not** blank them: `Value::Null` would store a JSON null where
    /// `NULL` belongs, which no reader treats as "never latched". No caller needs clearing today —
    /// the latch always has a document, and a row nobody has latched into is `NULL` from its
    /// insert. **MOD-7 is the caller that will**: unregistering an agent from a box, or a
    /// re-registration that must not carry the last box's allowance forward, has to put the pair
    /// back to `NULL`, and that is when these two parameters become `Option`s across the six
    /// implementations and `store::conformance`. Widening them before there is a caller would be
    /// six signatures changed to express a case no code can reach.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "agent_box"` and
    /// id `"<agent_id>/<box_id>"` when no row has that key.
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<()>;

    /// Mints the `run` / `run_step` pair of a free-standing chat, both `ON CONFLICT (id) DO
    /// NOTHING` (MOD-2 plan D4).
    ///
    /// The rows are the ones `htui-store`'s pending-buffer upload writes for the same chat, with
    /// `status = 'running'` and `finished_at NULL` in place of the upload's terminal values, so an
    /// online start followed by a replayed upload - or the reverse - converges on one pair of rows
    /// rather than two.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the project, box,
    /// user or agent the spec names does not exist.
    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()>;

    /// Closes both rows of a chat run: `run.status` / `run_step.status` to the same name, and
    /// `finished_at` on both (MOD-2 plan D4).
    ///
    /// Without this the chat would count towards `active_runs` forever. `status` must be one of
    /// the three terminal values [`RunStatus`] and
    /// [`StepStatus`](crate::model::StepStatus) share - `done`, `failed`, `cancelled` - because a
    /// live status has no `run_step` counterpart to write.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) when the run or the step does
    /// not exist; [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a
    /// non-terminal `status`.
    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> Result<()>;

    /// Writes `run_step.prompt_digest` and `run_step.trim_record`, both, and nothing else
    /// (`docs/ANA-5.md` §4.4): the pre-flight audit of `R-PRM-3` / `R-ORCH-11`, written at stage 3
    /// before a session starts.
    ///
    /// The same digest `htui_agent`'s `Recorder::record_prompt` will later recompute over the same
    /// text and hand to [`set_step_usage`](WriteStore::set_step_usage), so the column's two
    /// writers agree by construction rather than by ordering.
    ///
    /// Not folded into `set_step_usage`: that one is the **chat** path's digest writer (plan D97),
    /// whose prompt has no template, no sections and no trim record to write.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "run_step"` when
    /// the step does not exist.
    async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> Result<()>;

    // ---- MOD-15 milestone 1: the hierarchy (plan D1-D12) -----------------------------------
    //
    // Every edit is a compare-and-set on `updated_at` (D3): the caller passes the token it edited
    // from, the trigger (`clock_timestamp()`) writes the next one, no statement here sets it.
    // `workspace_project` has no `updated_at` and the two path tables are per-box rows with one
    // writer each, so those three are plain upserts. Readers are here rather than on `ReadStore`
    // (D1) so the conformance suite can read back what it wrote on both stores.

    // workspace

    /// Inserts a workspace; the returned row carries the store's clock, not the caller's.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `slug` is taken or
    /// `created_by` names no user.
    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace>;

    /// Edits `slug` / `name` / `description` when the row's `updated_at` still equals `expected`;
    /// `Stale` carries the row as it is now so the editor can reload (D3).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the new `slug`
    /// collides.
    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> Result<CasOutcome<Workspace>>;

    /// One workspace by id, `None` when there is no such row.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>>;

    // workspace links and box paths

    /// Inserts or repositions a workspace-to-project link (PK `(workspace_id, project_id)`, no
    /// `updated_at`, so no CAS).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row.
    async fn upsert_workspace_project(&self, link: &WorkspaceProject) -> Result<()>;

    /// Removes one link; the project survives (D4).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) (`entity: "workspace_project"`)
    /// when no such link exists.
    async fn remove_workspace_project(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> Result<()>;

    /// A workspace's links ordered by `position`, then `project_id` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn workspace_projects(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceProject>>;

    /// Inserts or replaces this box's root path for a workspace (PK `(workspace_id, box_id)`); the
    /// trigger advances `updated_at` on replace.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row.
    async fn upsert_workspace_box_path(&self, path: &WorkspaceBoxPath) -> Result<()>;

    /// Every box's root path for a workspace, ordered by `box_id` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn workspace_box_paths(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceBoxPath>>;

    // project

    /// Inserts a project with `settings = {}` and no secret provider (M1 D9) and seeds it, in
    /// the same transaction, with `htui_core::seed`'s catalogue: five default graphs, their
    /// fifteen phases (ANA-2 §4.1 as amended by PRD D3/D5), five kinds and the ten
    /// `DEFAULT_TEMPLATES` at version 1 (M2 D4). `item_key_counter` is not seeded.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `slug` is taken or
    /// `created_by` names no user; either way nothing is written.
    async fn create_project(&self, new: NewProject) -> Result<Project>;

    /// Edits `slug` / `name` / `description` under CAS; never touches `settings` (that is
    /// [`set_setting`](Self::set_setting)'s).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a `slug` collision.
    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> Result<CasOutcome<Project>>;

    // repo and repo box paths

    /// Inserts a repo. `is_primary: true` clears the project's current primary in the same
    /// transaction so `uq_repo_primary` never trips (D10).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `(project_id, name)`
    /// is taken or `project_id` names no row.
    async fn create_repo(&self, new: NewRepo) -> Result<Repo>;

    /// Edits under CAS; `is_primary: Some(true)` demotes the other primary (its `updated_at`
    /// advances too), `Some(false)` just unsets; `remote_url: Some(None)` clears the URL.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a `name` collision.
    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> Result<CasOutcome<Repo>>;

    /// A project's repos ordered by `name` bytes (`COLLATE "C"`, as `prompt_templates`).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>>;

    /// Inserts or replaces this box's checkout path for a repo (PK `(repo_id, box_id)`).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row.
    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> Result<()>;

    /// Every box's checkout path for a repo, ordered by `box_id` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>>;

    // item_kind

    /// Inserts a kind. The prefix is checked by [`ItemKind::prefix_is_valid`] before the statement
    /// and `default_graph_id` must belong to the same project (D11).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a bad prefix, a taken
    /// `(project_id, prefix)` or `(project_id, name)`, or a graph from another project.
    async fn create_item_kind(&self, new: NewItemKind) -> Result<ItemKind>;

    /// Edits under CAS with the same three rules as [`create_item_kind`](Self::create_item_kind).
    /// Renaming the prefix leaves existing keys (`ANA-2`) and counters alone; the next mint under
    /// the kind starts a counter for the new prefix (PRD D12).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) as for create.
    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> Result<CasOutcome<ItemKind>>;

    /// A project's kinds ordered by `position`, then `prefix` bytes (`position` is not unique).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>>;

    /// Deletes a kind nothing references (D6). `item_key_counter` is keyed by prefix and is never
    /// touched.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint)
    /// `"item_kind FEAT is held by 4 items"` while items reference it;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id.
    async fn delete_item_kind(&self, id: ItemKindId) -> Result<()>;

    // step_graph and phase

    /// Inserts a graph (no phases; [`create_phase`](Self::create_phase) adds them).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `(project_id, name)`
    /// is taken or `project_id` names no row.
    async fn create_step_graph(&self, new: NewStepGraph) -> Result<StepGraph>;

    /// Edits `name` / `description` under CAS.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a `name` collision.
    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> Result<CasOutcome<StepGraph>>;

    /// A project's graphs ordered by `name` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn step_graphs(&self, project: ProjectId) -> Result<Vec<StepGraph>>;

    /// Inserts a whole phase row (D10); `phase.updated_at` is ignored and the store's clock is
    /// returned. `judge` and `handoff` are template roles, not phase names
    /// ([`TemplateRole::of_name`](crate::prompt::template::TemplateRole::of_name)).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a reserved name, a
    /// taken `(graph_id, position)` or `(graph_id, name)`, or a `graph_id` that names no row.
    async fn create_phase(&self, phase: &StepGraphPhase) -> Result<StepGraphPhase>;

    /// Edits the five columns of [`PhasePatch`] under CAS; `token_budget` is
    /// [`set_setting`](Self::set_setting)'s on the `Phase` rung and is not here (D8).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a reserved name or a
    /// `(graph_id, position)` / `(graph_id, name)` collision.
    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> Result<CasOutcome<StepGraphPhase>>;

    /// A graph's phases ordered by `position` (unique per graph).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>>;

    // settings (D7, D8)

    /// Writes one setting on one rung after [`validate`](crate::prompt::settings::validate):
    /// rung, kind, range, `not_above` against the same rung's current peer (or its default).
    /// `App` is an `app_setting` row (`expected: None` = "I expect no row", the insert after a
    /// clear; `Stale` if one exists); `Project` merges one key into `project.settings` under CAS
    /// on `project.updated_at`; `Phase` writes `step_graph_phase.token_budget` under CAS on the
    /// phase's `updated_at`. `Stale` carries the setting as stored now.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) with the key, value and
    /// rule for every validation refusal, and for `expected: None` on `Project` / `Phase`;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown project or
    /// phase.
    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<StoredSetting>>;

    /// Removes one setting from one rung under CAS: `DELETE` on `app_setting` (the returned
    /// `updated_at` is the deleted row's; the next `set_setting` passes `expected: None`),
    /// `settings - key` on the project, `NULL` on the phase. `Applied` always carries
    /// `value: None`.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the key is not
    /// accepted on the rung; [`StoreError::NotFound`](crate::store::StoreError::NotFound) when
    /// the row (or, on `App`, the setting) does not exist.
    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>>;

    /// One setting on one rung with its CAS token. `None` only when the rung's row is absent
    /// (`App`: no such `app_setting`; `Project` / `Phase`: no such id); a present project or
    /// phase without the key answers `Some` with `value: None`.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the key is not
    /// accepted on the rung.
    async fn setting(&self, rung: SettingRung, key: SettingKey) -> Result<Option<StoredSetting>>;

    // deletes (D4)

    /// What a delete would remove, counted without removing; `None` when the target does not
    /// exist. The same counting code feeds [`delete_workspace`](Self::delete_workspace) and
    /// [`delete_project`](Self::delete_project), so the report equals the act (PRD D13).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn delete_reach(&self, target: DeleteTarget) -> Result<Option<DeleteReach>>;

    /// Removes a workspace, its links and its box paths; projects survive
    /// (`0001_init.sql:162,174`).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id.
    async fn delete_workspace(&self, id: WorkspaceId) -> Result<DeleteReach>;

    /// Removes a project and everything PRD D13 lists, in one transaction, and returns the counts
    /// it took. The mirror rebuild is the caller's (D5).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id.
    async fn delete_project(&self, id: ProjectId) -> Result<DeleteReach>;
}

/// The `run_step.status` a terminal [`RunStatus`] closes a chat step with, or `None` when the
/// status is still live (plan D4).
///
/// Shared by every [`WriteStore`] implementation so the two backends cannot disagree about which
/// three values are terminal; the text is the same in both tables' `CHECK` lists.
#[must_use]
pub fn chat_step_status(status: RunStatus) -> Option<crate::model::StepStatus> {
    use crate::model::StepStatus;
    match status {
        RunStatus::Done => Some(StepStatus::Done),
        RunStatus::Failed => Some(StepStatus::Failed),
        RunStatus::Cancelled => Some(StepStatus::Cancelled),
        RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval => None,
    }
}

/// The refusal text a non-terminal [`finish_chat_run`](WriteStore::finish_chat_run) status gets.
#[must_use]
pub fn not_a_terminal_status(status: RunStatus) -> String {
    format!("run.status `{status}` is not a terminal status a chat step can be closed with")
}

/// D11: the `item_kind.prefix` CHECK, in words.
///
/// The five helpers below exist for the reason [`chat_step_status`] does: a rule the schema cannot
/// express is checked in `htui-core` once, so `MemStore` and `PgStore` refuse the same input with
/// the same sentence rather than with a Postgres constraint name on one side and prose on the
/// other.
#[must_use]
pub fn invalid_prefix(prefix: &str) -> String {
    format!("item_kind.prefix `{prefix}` is not `^[A-Z][A-Z0-9]{{1,15}}$`")
}

/// D11: `default_graph_id` must be one of the kind's own project's graphs. The FK is to
/// `step_graph(id)` alone (`0001_init.sql:288`), so nothing below the seam checks this.
#[must_use]
pub fn graph_not_in_project(graph: StepGraphId, project: ProjectId) -> String {
    format!("step_graph {graph} is not in project {project}")
}

/// D11: `judge` and `handoff` are template roles (ANA-5 §4.6), not phase names — a project cannot
/// hold both a `judge` phase template and a `judge` judge template, because `prompt_template` is
/// `UNIQUE (project_id, name, version)`.
#[must_use]
pub fn reserved_phase_name(name: &str) -> String {
    format!("`{name}` is a reserved template name, not a phase name")
}

/// D6: the holder count, in the sentence the refusal carries.
///
/// The `item.kind_id` FK would refuse the delete on Postgres anyway (`0001_init.sql:313`, no
/// cascade), but with a constraint name where the PRD asks for "names what holds it".
#[must_use]
pub fn item_kind_is_held(prefix: &str, items: u64) -> String {
    format!("item_kind {prefix} is held by {items} items")
}

/// D8: the CAS token a rung whose row always exists must be given.
///
/// `expected: None` means "I expect no row", which only the [`SettingRung::App`] rung can mean: a
/// project and a phase exist before the setting does, so `None` there is misuse rather than an
/// insert — and refusing it is what keeps a caller from treating a missing token as a force-write.
/// `entity` is the table the row is in, so the sentence names the row rather than the rung.
///
/// It was private to `store::mem` until the MOD-15 milestone 1 review, which found `PgStore`
/// spelling the same sentence for itself in `pg/write.rs` — byte-identical, and one edit away from
/// not being. The stores wrap it in
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) themselves rather than sharing
/// a signature: `MemStore` has the token to hand back and `PgStore` has a `?` to return through.
#[must_use]
pub fn expected_on_row(key: SettingKey, entity: &str) -> String {
    format!(
        "`{key}` on the {entity} rung needs the row's `updated_at`; `expected: None` is the \
         app_setting insert alone"
    )
}

/// Result of [`WriteStore::update_item`]: either the edit landed, or someone else committed first
/// and the caller gets both sides plus the common ancestor to render (§4.2).
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateOutcome {
    /// The compare-and-set matched; this is the new head.
    Updated(Item),
    /// The compare-and-set found a different version.
    Diverged {
        /// The row as it is now.
        head: Item,
        /// The revision at the version the caller edited from.
        ancestor: ItemRevision,
    },
}

/// Result of a compare-and-set edit (D3). `Applied` carries the row the trigger stamped; `Stale`
/// carries the row as it is now, because the token the caller edited from no longer matches, so
/// the editor can reload and retry (PRD D8).
///
/// A type of its own rather than a reuse of [`UpdateOutcome`], which is typed to `Item` and
/// `ItemRevision` and carries a common ancestor these tables keep no history of.
#[derive(Debug, Clone, PartialEq)]
pub enum CasOutcome<T> {
    /// The token matched; this is the row as the store's clock left it.
    Applied(T),
    /// The token did not match; this is the row as it is now.
    Stale(T),
}

impl<T> CasOutcome<T> {
    /// The row either way, for a caller that wants to render what is stored and does not branch
    /// on which of the two happened.
    pub fn into_inner(self) -> T {
        match self {
            Self::Applied(row) | Self::Stale(row) => row,
        }
    }
}

/// What [`WriteStore::delete_reach`] counts for (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    /// A workspace: its links and its box paths, and no project.
    Workspace(WorkspaceId),
    /// A project: the whole `ON DELETE CASCADE` chain of PRD D13.
    Project(ProjectId),
}

/// Rows a delete removes, per table, in `0001_init.sql`'s cascade order (PRD D13; the fact-check
/// added `phase_agents`, which cascades through `step_graph_phase`).
///
/// One struct rather than a method per table, so a table `0003` adds is a field here and the two
/// callers of the counting code cannot disagree about it. A workspace delete fills
/// `workspace_links` and `workspace_box_paths` only. `phase_agents`, `run_step_commits` and
/// `command_runs` are `0` on `MemStore`, which holds none of the three tables, and `0` on the demo
/// database, which seeds none of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteReach {
    /// `workspace_project` rows.
    pub workspace_links: u64,
    /// `workspace_box_path` rows.
    pub workspace_box_paths: u64,
    /// `item` rows.
    pub items: u64,
    /// `item_key_counter` rows.
    pub item_key_counters: u64,
    /// `item_kind` rows.
    pub item_kinds: u64,
    /// `step_graph` rows.
    pub step_graphs: u64,
    /// `step_graph_phase` rows.
    pub phases: u64,
    /// `phase_agent` rows.
    pub phase_agents: u64,
    /// `prompt_template` rows.
    pub prompt_templates: u64,
    /// `repo` rows.
    pub repos: u64,
    /// `repo_box_path` rows.
    pub repo_box_paths: u64,
    /// `skill_binding` rows.
    pub skill_bindings: u64,
    /// `run` rows.
    pub runs: u64,
    /// `run_step` rows.
    pub run_steps: u64,
    /// `session_event` rows.
    pub session_events: u64,
    /// `run_step_commit` rows.
    pub run_step_commits: u64,
    /// `command_run` rows, which cascade from `run_step` (`0001_init.sql:539`).
    ///
    /// Nothing in this tree writes the table yet — MOD-16's queue is its first writer — so every
    /// count of it is `0` today. It is here anyway, because the alternative is a field added later
    /// by whoever first notices the delete took rows it never named (review L1).
    pub command_runs: u64,
    /// `item_note` rows.
    pub notes: u64,
    /// `item_revision` rows.
    pub revisions: u64,
    /// `item_link` rows, tombstones included.
    pub links: u64,
    /// `document` rows.
    pub documents: u64,
}

/// Where a setting lives (D8): an `app_setting` row, one key of `project.settings`, or
/// `step_graph_phase.token_budget`.
///
/// The PRD writes the phase id as `StepGraphPhaseId`; the newtype in this tree is [`PhaseId`] and
/// the seam uses the tree's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRung {
    /// One `app_setting` row, keyed by [`SettingKey::key`].
    App,
    /// One key of a project's `settings` document, keyed by
    /// [`SettingSpec::project_key`](crate::prompt::settings::SettingSpec::project_key).
    Project(ProjectId),
    /// `step_graph_phase.token_budget` of one phase.
    Phase(PhaseId),
}

impl SettingRung {
    /// The flag [`validate`](crate::prompt::settings::validate) checks against
    /// [`SettingSpec::rungs`](crate::prompt::settings::SettingSpec::rungs).
    #[must_use]
    pub const fn flag(self) -> Rungs {
        match self {
            Self::App => Rungs::APP,
            Self::Project(_) => Rungs::PROJECT,
            Self::Phase(_) => Rungs::PHASE,
        }
    }
}

/// One setting as stored, with the token a later [`WriteStore::set_setting`] or
/// [`WriteStore::clear_setting`] must present.
///
/// `value: None` is "the rung's row exists but holds no value for this key", which is a different
/// fact from the row not existing at all — the first means the rung below answers, the second
/// means there is nothing on this rung to compare against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSetting {
    /// The stored JSON, or `None` when this rung holds no value for the key.
    pub value: Option<Value>,
    /// The rung row's `updated_at`: the CAS token, never written by hand.
    pub updated_at: DateTime<Utc>,
}
