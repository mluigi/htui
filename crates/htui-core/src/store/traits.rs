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
use uuid::Uuid;

use crate::model::{
    Agent, AgentBox, AgentId, BoxId, BoxProbe, BoxRecord, ChatRunSpec, Claim, CommandRun, Document,
    DocumentHead, DocumentId, GateOutcome, Item, ItemFilter, ItemId, ItemKind, ItemKindId,
    ItemKindPatch, ItemPatch, ItemRevision, ItemSummary, LinkGraph, NewCommandRun, NewDocument,
    NewItem, NewItemKind, NewNote, NewProject, NewRepo, NewRun, NewRunStep, NewStepGraph,
    NewWorkspace, Note, PhaseId, PhasePatch, Project, ProjectId, ProjectPatch, PromptScope, Repo,
    RepoBoxPath, RepoId, RepoPatch, ResolvedInput, Run, RunId, RunStatus, RunStep, RunStepCommit,
    RunStepTree, RunSummary, Scope, SessionEvent, Status, StepGraph, StepGraphId, StepGraphPatch,
    StepGraphPhase, StepId, StepOutcome, StepStatus, UpstreamEntry, Workspace, WorkspaceBoxPath,
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

    // ---- ANA-2 §8: the run tables (MOD-4 milestone 1, plan D1) -------------------------------
    //
    // All five read a table the cache mirrors, which is §8's test for a trait method rather than
    // an inherent one: `run`, `run_step` and `run_step_commit` are mirrored already, and
    // `run_step_tree` becomes mirrored in this milestone.

    /// One `run` row, or `None`. Mirrored (`cache_migrations/0001_mirror.sql:103`), so every
    /// backend answers it.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn run(&self, id: RunId) -> Result<Option<Run>>;

    /// Every `run_step` of a run in `(position, attempt, fanout_index)` order — the judge
    /// (`fanout_index = -1`) sorts before its candidates. Empty for an unknown run: a list read is
    /// total, like [`documents`](ReadStore::documents).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>>;

    /// The step's `run_step_tree` rows in `repo_id` order; empty for an unknown step.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn step_trees(&self, step: StepId) -> Result<Vec<RunStepTree>>;

    /// The step's `run_step_commit` rows in `repo_id` order; empty for an unknown step.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn step_commits(&self, step: StepId) -> Result<Vec<RunStepCommit>>;

    /// ANA-2 §4.2's resolver, which [`documents_of_kinds`](ReadStore::documents_of_kinds) is not
    /// (plan D2): per requested kind, the latest document of the item whose producing step is not
    /// a fan-out loser (`selected IS NOT FALSE`), preferring one produced by a step of `run`;
    /// hand-written documents rank after any run's output (`ORDER BY (s.run_id = $run) DESC
    /// NULLS LAST, d.version DESC`). One entry per kind in `kinds` order, a missing kind carried
    /// as `document: None`. An empty `kinds` means every kind the item has, in kind byte order.
    ///
    /// Total, like the two list reads above: an unknown item resolves every requested kind to
    /// `None` rather than refusing, because a caller that asks for inputs of an item that is gone
    /// has the same problem either way and the step's own failure names the missing kinds.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> Result<Vec<ResolvedInput>>;
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
    ///
    /// An illegal `(from, to)` is [`StoreError::Constraint`](crate::store::StoreError::Constraint)
    /// without an update ([`legal_move`], ANA-2 §4.3); a missing row is
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) first (plan D14). A stale
    /// `from` on a legal pair is still `Ok(false)`.
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

    /// Writes one box probe (MOD-7 D10): the hardware columns, `probed_tags`, `htui_version`,
    /// `last_probed_at` and `probe_spec_digest`, and replaces this box's `box_tool` set, in one
    /// transaction. A narrow machine writer (MOD-2 D74): never `hostname`, `declared_tags`, `quirks`,
    /// `settings`, `machine_fingerprint`, `edit_version` or `last_seen_at`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "box"` when no
    /// row has `probe.box_id`; [`StoreError::Constraint`](crate::store::StoreError::Constraint)
    /// when a tool name repeats or `spec_digest` is not 64 lowercase hex. Either way nothing is
    /// written.
    async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()>;

    /// Every box of this user with its tools and recorded spec digest (MOD-7 D10, D18): boxes by
    /// id, tools by name byte order (`COLLATE "C"`), so every store answers byte for byte.
    ///
    /// # Errors
    ///
    /// Whatever the backend's read fails with.
    async fn boxes(&self) -> Result<Vec<BoxRecord>>;

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
    /// [`StepStatus`] share - `done`, `failed`, `cancelled` - because a
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

    // ---- ANA-2 §8: graph runs (MOD-4 milestone 1) --------------------------------------------
    //
    // In §8's order. Seven are transactions on every backend, `MemStore` included (plan M1 D6,
    // M2 D7): `create_run`, `claim_run`, `select_fanout`, `write_document`, `promote_step`,
    // `finish_run` and `close_out`. Every writer
    // that stamps a clock takes the instant from the caller (blueprint F-S); `updated_at` stays
    // the trigger's and is never set by hand.

    /// Inserts a `kind = 'graph'` run at `queued` with its snapshot and moves the item to
    /// `queued` under the §4.3 law, in one transaction (plan D6).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "item" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the item's status
    /// cannot move to `queued` (only `open` and `failed` can), when the item is not in
    /// `new.project_id` ([`item_not_in_project`] — the run is filed under that id and
    /// [`delete_project`](WriteStore::delete_project) counts by it), when `id` already exists, or
    /// on any foreign key.
    async fn create_run(&self, new: NewRun) -> Result<Run>;

    /// ANA-2 §4.7's admission, one transaction, answered as a [`Claim`] (plan D83).
    ///
    /// The decision order: `NotFound` for the run, then the box; [`Claim::NotClaimable`] when the
    /// run is not `queued` or its `target_box_id` is not `box_id`; [`Claim::SlotFull`] when the
    /// box already runs its limit of **`running`** runs
    /// ([`BoxSettings::max_concurrent_items`](crate::model::BoxSettings::max_concurrent_items),
    /// else `app_setting`, else
    /// [`DEFAULT_MAX_CONCURRENT_ITEMS`](crate::model::DEFAULT_MAX_CONCURRENT_ITEMS)); then
    /// [`Claim::Overlaps`] naming the first **non-terminal** run on the box, in
    /// `(queued_at, id)` order, that §4.7's predicate
    /// [`overlaps`](crate::model::overlap::overlaps) says it collides with: rule L (either run is
    /// `local` in a shared repo), rule I (either is not isolated there), rule P (both are
    /// isolated but their path prefixes intersect, an empty list meaning the whole repo). Only
    /// runs whose `repo_scope` shares a repo with this one are compared. Each side's scope is
    /// [`scope_of`](crate::model::overlap::scope_of) over its `graph_snapshot`, so a run written
    /// before milestone 5 (no `scope`) reads conservatively: any shared repo overlaps, as
    /// [`OverlapRule::NotIsolated`](crate::model::OverlapRule::NotIsolated).
    ///
    /// On [`Claim::Admitted`] the run moves `queued -> running` with `executing_box_id = box_id`,
    /// `started_at = at`, `lease_box_id = box_id`, `lease_owner = owner`,
    /// `lease_expires_at = lease_until`, and the item `queued -> in_progress`. Every other answer
    /// writes nothing.
    ///
    /// The two predicates range over two different sets, and `awaiting_approval` is where they
    /// part: a parked run consumes no compute and so holds no slot, but it still owns its trees
    /// and its unmerged branch and so still refuses an overlapping scope (§4.7, invariant 6).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown run (`"run"`)
    /// or box (`"box"`), the run looked up first.
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Claim>;

    /// ANA-2 §4.9's heartbeat: `UPDATE run SET lease_expires_at = until WHERE id = run AND
    /// lease_owner = owner`. `Ok(false)` = zero rows = abandon; the run exists but is not ours.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`.
    async fn refresh_lease(&self, run: RunId, owner: Uuid, until: DateTime<Utc>) -> Result<bool>;

    /// ANA-2 §4.9's sweep: every `running` run whose `executing_box_id` is `box_id` and whose
    /// lease is `NULL` or expired at `now` becomes ours (`lease_owner = owner`,
    /// `lease_expires_at = lease_until`). Returns the adopted rows in `queued_at` order, ties
    /// broken by `id` so the order is total and the same on every backend; empty when nothing was
    /// abandoned. A box that does not exist adopts nothing (`Ok(vec![])`).
    ///
    /// **Never** a run whose `lease_owner` is `owner` (plan D88): a process whose heartbeat
    /// stalled past its TTL must not adopt its own live walk and run it twice under one owner.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn adopt_runs(
        &self,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Vec<Run>>;

    /// Plan D87: the lease of a run that is ours or free, taken before a command's first write on
    /// a parked or running run. `UPDATE run SET lease_owner = owner, lease_box_id = box_id,
    /// lease_expires_at = until WHERE id = run AND status IN ('running','awaiting_approval') AND
    /// executing_box_id = box_id AND (lease_owner = owner OR lease_owner IS NULL OR
    /// lease_expires_at IS NULL OR lease_expires_at <= now)`.
    ///
    /// `Ok(false)` = zero rows: another owner holds a live lease, or the run is not takeable here
    /// (not `running`/`awaiting_approval`, or executing on another box). Nothing is written then.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`, told
    /// apart from "not takeable" by one follow-up read.
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<bool>;

    /// Plan D139: gives a lease back. `UPDATE run SET lease_owner = NULL, lease_expires_at = now
    /// WHERE id = run AND lease_owner = owner`. `Ok(false)` = zero rows = not ours, and nothing
    /// is written.
    ///
    /// The owner is cleared, not only the expiry. Plan D88 keeps [`adopt_runs`] off a run whose
    /// `lease_owner` is the sweeper, so a lease released with [`refresh_lease`] stayed out of its
    /// own process's sweep for ever. A released run is free to every sweep, this process's
    /// included. A heartbeat that hung and commits after the release matches no row, so it
    /// cannot bring the lease back.
    ///
    /// `lease_box_id` is left as it was. [`claim_run`], [`take_lease`] and [`adopt_runs`] all
    /// select on `executing_box_id`, never on `lease_box_id`, and each of them overwrites it.
    /// So it only names the box that last held the lease.
    ///
    /// [`adopt_runs`]: WriteStore::adopt_runs
    /// [`refresh_lease`]: WriteStore::refresh_lease
    /// [`claim_run`]: WriteStore::claim_run
    /// [`take_lease`]: WriteStore::take_lease
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`, told
    /// apart from "not ours" by one follow-up read.
    async fn release_lease(&self, run: RunId, owner: Uuid, now: DateTime<Utc>) -> Result<bool>;

    /// Inserts a step at `pending`. `fanout_index = -1` is the judge and is accepted.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on an unknown run or agent
    /// (foreign key, like [`append_events`](WriteStore::append_events)), on a duplicate id, or on
    /// a repeat of `(run_id, position, attempt, fanout_index)`.
    async fn create_step(&self, new: NewRunStep) -> Result<RunStep>;

    /// §4.3 compare-and-set on `run.status`; `started_at = COALESCE(started_at, at)` when `to` is
    /// `running`, `finished_at = COALESCE(finished_at, at)` when `to` is terminal. Same contract
    /// as [`transition`](WriteStore::transition): `Ok(false)` on a stale `from`, `Constraint` on
    /// an illegal pair without an update, `NotFound` first (plan D14).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a pair outside
    /// [`RunStatus::can_move_to`].
    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool>;

    /// The `run_step` twin of [`transition_run`](WriteStore::transition_run).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a pair outside
    /// [`StepStatus::can_move_to`].
    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool>;

    /// Writes the settle columns of [`StepOutcome`]; never `status`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`.
    async fn finish_step(&self, step: StepId, outcome: StepOutcome) -> Result<()>;

    /// Plan D89, ANA-2 §4.9's interrupted step: a compare-and-set `running -> failed` with
    /// `gate_note = note` and `finished_at = COALESCE(finished_at, at)`, `gate_outcome` left
    /// untouched (`NULL`: a crash is not a gate answer). `Ok(false)` when the step is not
    /// `running`, with nothing written.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`.
    async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool>;

    /// The four `R-ORCH-2` answers, a compare-and-set on `awaiting_approval`:
    /// `Approved | Skipped -> done`, `Rejected -> failed`, `Retried -> superseded`, with
    /// `gate_outcome`, `gate_note` and `finished_at = COALESCE(finished_at, at)`. `Ok(false)` when
    /// the step is not awaiting.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`.
    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool>;

    /// ANA-2 §4.5's bookkeeping, one transaction (plan D6): among the steps of
    /// `(run, position, attempt)` with `fanout_index >= 0`, the winner becomes
    /// `selected = true, status = done`; every other candidate becomes `selected = false` and,
    /// when its status is `pending`, `awaiting_approval` or `done`, `superseded` (a `failed` or
    /// `cancelled` loser keeps its status); the judge row (`fanout_index = -1`), when there is
    /// one, becomes `done` with `gate_note = reason` — but only from `pending`, `running` or
    /// `awaiting_approval`. A judge that already reached an outcome is left alone entirely, status
    /// and `gate_note` both: §4.5's judge-failure path (`docs/ANA-2.md:858-862`) parks the run at
    /// `awaiting_approval` with the failure reason in the judge's `gate_note` and has *a human*
    /// pick through this method, so settling it here would be the `failed -> done` §4.3 rejects,
    /// written over the reason the pick was made from. "Nothing is lost" is that sentence.
    ///
    /// §4.5 selects among *settled* candidates, so a candidate still `running` is not a case the
    /// slot is expected to hold; one that is gets `selected = false` and keeps `running`, exactly
    /// as a `failed` loser keeps `failed`. Nothing here refuses it — a live loser is the
    /// orchestrator's to cancel (§4.4), not this write's.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }` for
    /// `winner`; [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `winner` is
    /// not a candidate of that `(run, position, attempt)` or is not `awaiting_approval | done`.
    /// Either refusal leaves every row untouched.
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> Result<()>;

    /// §4.4's loop half: `pending | awaiting_approval | done -> superseded`, no other status.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) from any other status (the
    /// law's table, not a stale compare-and-set).
    async fn supersede_step(&self, step: StepId) -> Result<()>;

    /// Upserts `run_step_tree` rows on the table's `(run_step_id, repo_id)` key; an empty slice
    /// checks the step and writes nothing.
    ///
    /// Also writes `run_step.isolation_path` (ANA-2 `:903`, plan D33): the `path` of the batch's
    /// row whose repo `is_primary`, else of its lowest `repo_id` — the order
    /// [`step_trees`](ReadStore::step_trees) answers in. An empty batch names no tree, so the
    /// column is left as it was rather than cleared. One step has many trees and one
    /// `isolation_path`, and this is where that choice is made, so the two can never disagree.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when a row's
    /// `run_step_id` is not `step` or names an unknown repo.
    async fn upsert_step_tree(&self, step: StepId, trees: &[RunStepTree]) -> Result<()>;

    /// `R-ORCH-11`'s two hashes, upserted on `(run_step_id, repo_id)`; same refusals as
    /// [`upsert_step_tree`](WriteStore::upsert_step_tree).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when a row's
    /// `run_step_id` is not `step` or names an unknown repo.
    async fn record_commits(&self, step: StepId, commits: &[RunStepCommit]) -> Result<()>;

    /// Records one `command_run` row (ANA-2 §4.2, `docs/ANA-2.md:501-506`; plan D31): this
    /// milestone's `verify_command` runs, later MOD-11's queue.
    ///
    /// Every column is the caller's, `queued_at` included, so the row is the durable input of a
    /// later `verify_failure` render (`docs/ANA-5.md:335`) rather than a second reading of a
    /// second clock. Nothing is allocated server-side, so the stored row is the argument and is
    /// handed straight back.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`,
    /// checked before the box;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) with
    /// [`references_no_row`] for an unknown `box_id` and with [`already_exists`] for an `id` the
    /// store already holds. Both sentences are `MemStore`'s; Postgres answers the same variant in
    /// its own constraint's words.
    async fn record_command_run(&self, new: NewCommandRun) -> Result<CommandRun>;

    /// A step's `command_run` rows in `(queued_at, id)` order.
    ///
    /// A read on [`WriteStore`] rather than [`ReadStore`], by milestone 1's precedent for
    /// [`repos`](WriteStore::repos) and [`phases`](WriteStore::phases): `command_run` is not a
    /// mirrored table — [`MIRRORED_TABLES`](crate::store::MIRRORED_TABLES) does not list it — so a
    /// `ReadStore` placement would put it where the conformance suite, which is written against
    /// `WriteStore` alone, could never reach it.
    ///
    /// Total, like [`step_trees`](ReadStore::step_trees): a step with no rows and a step id
    /// nothing has both answer `Ok(vec![])`.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn command_runs(&self, step: StepId) -> Result<Vec<CommandRun>>;

    /// Inserts the document at `max(version) + 1` for `(item, kind)` under the item's row lock
    /// (plan D6), so two writers cannot allocate the same version.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "item" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a duplicate id or an
    /// unknown `produced_by_step_id` / `created_by`.
    async fn write_document(&self, new: NewDocument) -> Result<Document>;

    /// §4.8's promotion, one transaction: the step
    /// `failed | awaiting_approval -> awaiting_approval` with `promoted_at = at`; its run
    /// `running -> awaiting_approval` (already `awaiting_approval` is fine); its item
    /// `in_progress -> awaiting_approval` (already `awaiting_approval` is fine).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the step's status is
    /// any other, or its run is terminal.
    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> Result<()>;

    /// `status = failed, failure = failure, finished_at = COALESCE(finished_at, at)` from any
    /// non-terminal status (the law allows `queued | running | awaiting_approval -> failed`).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the run is already
    /// terminal.
    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> Result<()>;

    /// Ends a graph run and mirrors the item in the same transaction (ANA-2 §4.3 verdict 3 and
    /// the propagation rule at `:660-664`; milestone 2 plan D7, blueprint R-1 option (b)).
    ///
    /// `to` must be terminal (`done`, `failed`, `cancelled`). The run moves `<current> -> to`
    /// under [`legal_move`], `finished_at = COALESCE(finished_at, at)`, and `failure` is written
    /// when `to = failed`. Then, **only when no other run of the item is non-terminal**, the item
    /// moves:
    ///
    /// | `to`        | item, from -> to                                                 |
    /// |-------------|------------------------------------------------------------------|
    /// | `done`      | `in_progress -> done`                                            |
    /// | `failed`    | `in_progress -> failed`, `awaiting_approval -> failed`            |
    /// | `cancelled` | `queued -> open`, `in_progress -> open`, `awaiting_approval -> open` |
    ///
    /// An item at any other status is left alone (plan D17: the row may not have passed through
    /// `can_move_to`), and an item that still has a live run is held where it is — the second leg
    /// of the conformance case. A chat run (`item_id IS NULL`) moves only the run.
    ///
    /// [`transition_run`](WriteStore::transition_run) and [`fail_run`](WriteStore::fail_run) are
    /// unchanged and remain the run-only writers the chat path uses.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) with
    /// [`finish_run_needs_a_terminal_status`] when `to` is not terminal, with
    /// [`failure_disagrees_with_status`] when `failure.is_some() != (to == Failed)`, and with
    /// [`illegal_move`]'s sentence when the run is already terminal.
    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()>;

    /// `R-TUI-9`'s three effects, one transaction (plan D6): the summary document at its next
    /// version, the commits upserted, the item moved to `closed` under the law with `closed_at`
    /// set. Refused while any run of the item is active.
    ///
    /// # Errors
    /// Its own refusals, plus - because it performs their work - every refusal of
    /// [`record_commits`](WriteStore::record_commits) and
    /// [`write_document`](WriteStore::write_document). In full:
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "item" }`, or
    /// `{ entity: "run_step" }` for a commit row naming a step that does not exist;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when a run of the item is
    /// `queued | running | awaiting_approval`, when `summary.kind != "summary"`, when
    /// `summary.item_id != item`, when the item's status cannot move to `closed` (only `blocked`,
    /// `failed` and `done` can), when a commit row names an unknown repo, or when the summary
    /// duplicates a document id or names an unknown `created_by` / `produced_by_step_id`. Any
    /// refusal writes nothing: every one of these is decided before the first write.
    async fn close_out(
        &self,
        item: ItemId,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document>;

    /// Inserts an `item_note`; the refusal notes of ANA-2 invariant 7.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on an unknown item,
    /// author, box or step (foreign keys) or a duplicate id.
    async fn add_note(&self, note: NewNote) -> Result<Note>;
}

/// The `run_step.status` a terminal [`RunStatus`] closes a chat step with, or `None` when the
/// status is still live (plan D4).
///
/// Shared by every [`WriteStore`] implementation so the two backends cannot disagree about which
/// three values are terminal; the text is the same in both tables' `CHECK` lists.
#[must_use]
pub fn chat_step_status(status: RunStatus) -> Option<StepStatus> {
    match status {
        RunStatus::Done => Some(StepStatus::Done),
        RunStatus::Failed => Some(StepStatus::Failed),
        RunStatus::Cancelled => Some(StepStatus::Cancelled),
        RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval => None,
    }
}

/// The three ANA-2 §4.3 tables behind one name, so `MemStore` and `PgStore` call the same rule
/// (plan D15; the precedent is [`chat_step_status`]).
pub trait TransitionLaw: Copy + std::fmt::Display {
    /// `"item"`, `"run"` or `"run_step"`: the `entity` a refusal names.
    const ENTITY: &'static str;

    /// The table: whether `self -> to` is a sanctioned move.
    fn is_legal_move(self, to: Self) -> bool;
}

impl TransitionLaw for Status {
    const ENTITY: &'static str = "item";

    fn is_legal_move(self, to: Self) -> bool {
        self.can_move_to(to)
    }
}

impl TransitionLaw for RunStatus {
    const ENTITY: &'static str = "run";

    fn is_legal_move(self, to: Self) -> bool {
        self.can_move_to(to)
    }
}

impl TransitionLaw for StepStatus {
    const ENTITY: &'static str = "run_step";

    fn is_legal_move(self, to: Self) -> bool {
        self.can_move_to(to)
    }
}

/// `Ok(())` when `from -> to` is in the table, else the
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) every compare-and-set returns
/// **before** touching the row.
///
/// Precedence (plan D14): the caller looks the row up first, so a missing row is
/// [`StoreError::NotFound`](crate::store::StoreError::NotFound) even when the pair is also
/// illegal — telling a caller its pair is wrong when the real problem is that its id is would be
/// the wrong sentence, and `pg_criteria` already pins the other order.
///
/// The order every compare-and-set honours, on both stores: row lookup → `NotFound`; this call →
/// `Constraint` with no write; `from` mismatch → `Ok(false)` with no write; update → `Ok(true)`.
///
/// # Errors
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) with [`illegal_move`]'s
/// sentence when the pair is outside the table.
pub fn legal_move<T: TransitionLaw>(from: T, to: T) -> Result<()> {
    if from.is_legal_move(to) {
        Ok(())
    } else {
        Err(crate::store::StoreError::Constraint(illegal_move(
            T::ENTITY,
            from,
            to,
        )))
    }
}

/// The refusal text of an illegal move, one sentence for the three tables.
#[must_use]
pub fn illegal_move(
    entity: &'static str,
    from: impl std::fmt::Display,
    to: impl std::fmt::Display,
) -> String {
    format!("{entity}.status `{from}` cannot move to `{to}` (ANA-2 §4.3)")
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

/// §5.1: a run is filed under `run.project_id`, which `delete_project` counts by — so the item it
/// runs must live in that same project. The schema cannot say it: the two columns are independent
/// foreign keys.
#[must_use]
pub fn item_not_in_project(item: ItemId, project: ProjectId) -> String {
    format!("item {item} is not in project {project}")
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

// ---- MOD-4: the refusals of ANA-2 §8's writers ------------------------------------------------
//
// Same reason as the five above: a rule the schema cannot express is spelled once here, so
// `MemStore` and `PgStore` refuse the same input with the same sentence. The FK refusals reuse
// `references_no_row`, which is the sentence `MemStore` already gave `run.project_id` and
// `session_event.run_step_id` before this milestone gave it a name.

/// A foreign key that names no row, in the sentence both stores give it.
#[must_use]
pub fn references_no_row(column: &str, id: impl std::fmt::Display, table: &str) -> String {
    format!("{column} `{id}` references no {table}")
}

/// A client-minted id that is already stored: Postgres's duplicate key, in words.
#[must_use]
pub fn already_exists(entity: &str, id: impl std::fmt::Display) -> String {
    format!("{entity} `{id}` already exists")
}

/// `UNIQUE (run_id, position, attempt, fanout_index)` (§5.8), in words.
#[must_use]
pub fn step_slot_is_taken(run: RunId, position: i32, attempt: i32, fanout_index: i32) -> String {
    format!("run {run} already has a step at ({position}, {attempt}, {fanout_index})")
}

/// `R-TUI-9`: close-out is refused while any run of the item is still active, and the refusal
/// names the run that is in the way rather than saying only that one is.
#[must_use]
pub fn item_has_a_live_run(item: ItemId, run: RunId, status: RunStatus) -> String {
    format!("item {item} has a live run {run} (`{status}`)")
}

/// `R-TUI-9`: the document close-out writes is the item's summary, not any other kind.
#[must_use]
pub fn close_out_needs_a_summary(kind: &str) -> String {
    format!("close_out writes a `summary` document, not a `{kind}`")
}

/// `R-TUI-9`: the summary must name the item being closed.
#[must_use]
pub fn summary_names_another_item(item: ItemId, named: ItemId) -> String {
    format!("the summary of {item} cannot name item {named}")
}

/// §4.5: `select_fanout` takes a winner from the candidates of one `(run, position, attempt)`.
#[must_use]
pub fn not_a_fanout_candidate(winner: StepId, run: RunId, position: i32, attempt: i32) -> String {
    format!("run_step {winner} is not a candidate of run {run} at ({position}, {attempt})")
}

/// §4.5: only a settled candidate can win — one the gate has answered or that finished on its own.
#[must_use]
pub fn winner_is_not_settled(winner: StepId, status: StepStatus) -> String {
    format!("run_step {winner} is `{status}`; a fan-out winner is `awaiting_approval` or `done`")
}

/// §4.6 / `R-ORCH-11`: a batch of tree or commit rows belongs to the step it is written for.
#[must_use]
pub fn row_names_another_step(table: &str, row: StepId, step: StepId) -> String {
    format!("{table}.run_step_id `{row}` is not the step being written (`{step}`)")
}

/// §4.8: promotion takes a step the run stopped on, not one that is still moving or already done.
#[must_use]
pub fn step_is_not_promotable(step: StepId, status: StepStatus) -> String {
    format!("run_step {step} is `{status}`; only `failed` and `awaiting_approval` are promotable")
}

/// §4.8: a terminal run has nothing left to stop on, so nothing below it can be promoted.
#[must_use]
pub fn run_is_terminal(run: RunId, status: RunStatus) -> String {
    format!("run {run} is terminal (`{status}`)")
}

/// M2 D7: where [`finish_run`](WriteStore::finish_run) takes the item, or `None` to leave it.
///
/// ANA-2 §4.3's verdict table, as one function rather than one per backend: `MemStore` matches on
/// it and `PgStore` binds its answer, so the mapping cannot be spelled two ways the way the refusal
/// sentences could before they were named. `to` is the run's terminal status and `item` the item's
/// *current* one; the caller has already established that no other run of the item is active.
///
/// `None` is "leave it alone", which is plan D17 rather than an omission: MOD-2's chat path and the
/// offline upload path insert `run` and `item` rows outside §4.3, so an item at a status this table
/// does not list has not necessarily passed through [`Status::can_move_to`] and must not be forced
/// through the law on its way out.
#[must_use]
pub const fn finish_run_item_mirror(to: RunStatus, item: Status) -> Option<Status> {
    match (to, item) {
        (RunStatus::Done, Status::InProgress) => Some(Status::Done),
        (RunStatus::Failed, Status::InProgress | Status::AwaitingApproval) => Some(Status::Failed),
        (RunStatus::Cancelled, Status::Queued | Status::InProgress | Status::AwaitingApproval) => {
            Some(Status::Open)
        }
        _ => None,
    }
}

/// M2 D7: [`finish_run`](WriteStore::finish_run) was asked to leave a run non-terminal.
///
/// It is a separate sentence from [`illegal_move`] on purpose: `running -> awaiting_approval` is a
/// *sanctioned* pair, so the law has nothing to say about it — what refuses it here is that this
/// writer ends runs, and `transition_run` is the one that moves them.
#[must_use]
pub fn finish_run_needs_a_terminal_status(run: RunId, to: RunStatus) -> String {
    format!("run {run}: `finish_run` moves to a terminal status, not `{to}` (ANA-2 §4.3)")
}

/// M2 D7: `finish_run`'s `failure` and `to` disagree — text on a non-failure, or none on a failure.
///
/// D12's failure strings are `run.failure` text, so a terminal `failed` that carried none would
/// leave the column NULL and force the second write the composite exists to avoid; and a `done`
/// that carried one would file a reason under a run that has none.
#[must_use]
pub fn failure_disagrees_with_status(run: RunId, to: RunStatus, has_failure: bool) -> String {
    if has_failure {
        format!("run {run}: a failure text is refused on a move to `{to}`")
    } else {
        format!("run {run}: a move to `failed` needs a failure text")
    }
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
/// `workspace_links` and `workspace_box_paths` only. `phase_agents` is `0` on `MemStore`, which
/// holds no such table, and `0` on the demo database, which seeds none; `run_step_commits`,
/// `run_step_trees` and `command_runs` are counted on both since MOD-4.
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
    /// `run_step_tree` rows, the table `0003_orchestration.sql` adds (ANA-2 §4.6).
    pub run_step_trees: u64,
    /// `command_run` rows, which cascade from `run_step` (`0001_init.sql:539`).
    ///
    /// Held here since MOD-15 against a table nothing wrote, because the alternative was a field
    /// added later by whoever first noticed the delete took rows it never named (review L1).
    /// MOD-4 milestone 3's [`record_command_run`](WriteStore::record_command_run) is the first
    /// writer, so the count is a real one on both stores now.
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
