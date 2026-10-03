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
//!
//! **MOD-9 milestone 3** adds the skill readers to `WriteStore` beside their writers (plan D75),
//! as MOD-15 did; the bound-skill read the prompt uses stays inherent.
//!
//! **MOD-23** adds one narrow writer, [`WriteStore::set_agent_box_enabled`], the per-box switch
//! (plan D242).
//!
//! **MOD-42** (plan D1-D5, D12-D14) adds nine writer methods over two tables, `step_permission`
//! and `run_command`: the permission and control relay. Neither table is mirrored (plan OQ-4), so
//! all nine are online and writer-only, by the `command_runs` precedent; `RelayStore` and
//! `WorkerStore` forward seven of them, and `relay_view` / `answer_permission` stay here alone.

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::model::link::{ItemLink, LinkKind, ProposeLink, WithdrawLink};
use crate::model::skill::validate_name;
use crate::model::skill_glob::{SkillGlob, canonical_globs};
use crate::model::skill_language;
use crate::model::{
    Activation, Agent, AgentBox, AgentId, AnswerOutcome, Attachment, BindingChange, BoxEdit, BoxId,
    BoxProbe, BoxRecord, BoxRow, CancelRequest, ChatRunSpec, CitationKind, Claim, CommandRun,
    CoverageRow, Document, DocumentHead, DocumentId, GateOutcome, Item, ItemCitation, ItemFilter,
    ItemId, ItemKind, ItemKindId, ItemKindPatch, ItemPatch, ItemRequirement, ItemRevision,
    ItemSummary, LinkGraph, NewCommandRun, NewDocument, NewItem, NewItemKind, NewNote, NewPersona,
    NewProject, NewPromptTemplate, NewRepo, NewRequirement, NewRequirementArea, NewRun, NewRunStep,
    NewSkill, NewSkillVersion, NewStepGraph, NewWorkspace, Note, OpenPermission, PermissionChoice,
    PermissionId, PermissionStatus, Persona, PersonaId, PersonaPatch, PhaseAgent, PhaseId,
    PhasePatch, Project, ProjectId, ProjectPatch, PromptScope, PromptTemplate, RelaySessionId,
    RelayView, Repo, RepoBoxPath, RepoId, RepoPatch, Requirement, RequirementArea,
    RequirementAreaId, RequirementFilter, RequirementId, RequirementPatch, RequirementRevision,
    RequirementSpec, RequirementUpdate, Resolution, ResolvedInput, Run, RunCommand, RunCommandId,
    RunCommandStatus, RunId, RunStatus, RunStep, RunStepCommit, RunStepTree, RunSummary, Scope,
    SessionEvent, Skill, SkillBinding, SkillBindingKey, SkillId, SkillPatch, SkillVersion, Status,
    StepGraph, StepGraphId, StepGraphPatch, StepGraphPhase, StepId, StepOutcome, StepPermission,
    StepStatus, ToolCallCount, UpstreamEntry, UserId, Workspace, WorkspaceBoxPath, WorkspaceId,
    WorkspacePatch, WorkspaceProject,
};
use crate::prompt::settings::{Rungs, SettingKey};
use crate::prompt::template::{TemplateRole, parse};
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

    // ---- ANA-11 §5.1: requirements (MOD-38) --------------------------------------------------

    /// The project's spec header, or `None` when it has none (mirrored, plan D12).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn requirement_spec(&self, project: ProjectId) -> Result<Option<RequirementSpec>>;

    /// The project's areas in `(position, code)` order, code by bytes; empty for an unknown
    /// project.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn requirement_areas(&self, project: ProjectId) -> Result<Vec<RequirementArea>>;

    /// The project's requirements matching `filter` (plan D14), in `(area_code, number)` order,
    /// area_code by bytes. The text filter is a literal substring of `key` or `body`, case-folded
    /// per backend as [`items`](Self::items)' is ([`RequirementFilter::text`]).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn requirements(
        &self,
        project: ProjectId,
        filter: &RequirementFilter,
    ) -> Result<Vec<Requirement>>;

    /// One requirement, or `None`.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn requirement(&self, id: RequirementId) -> Result<Option<Requirement>>;

    /// The requirement's revisions in `version` order; `Some(vec![])` for an unknown id on
    /// `MemStore`/`PgStore`, and `None` = not cached (the mirror holds no revisions, plan D12).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn requirement_revisions(
        &self,
        id: RequirementId,
    ) -> Result<Option<Vec<RequirementRevision>>>;

    /// The item's live citations with `suspect` derived (plan D11), in
    /// `(requirement.area_code, requirement.number, kind)` order, all by bytes; empty for an
    /// unknown item.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn item_requirements(&self, item: ItemId) -> Result<Vec<ItemCitation>>;

    /// The requirement's live citations with each citing item's status and resolution, in
    /// `(item.key_prefix, item.key_number, item.id, kind)` order, text by bytes; empty for an
    /// unknown id.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn requirement_coverage(&self, requirement: RequirementId) -> Result<Vec<CoverageRow>>;

    // ---- MOD-72: per-step tool-call counts (ANA-12 §3.2) ------------------------------------

    /// How many `tool_call` rows each step of the item's runs recorded, per `payload.tool_kind`
    /// (MOD-72 plan D1-D3): one row per `(step, tool_kind)` with `calls >= 1`, in
    /// [`ToolCallCount::sort_canonical`](crate::model::ToolCallCount::sort_canonical) order. A
    /// `tool_kind` that is missing or not a JSON string counts as `"other"`; a `tool_result` is
    /// half of a call and is not counted. Empty for an unknown item, or one whose steps recorded
    /// no call.
    ///
    /// On `ReadStore` because `session_event` is mirrored (`cache_migrations/0001_mirror.sql:125-128`):
    /// offline the mirror answers from its last-N-steps window, and a step outside it has no row,
    /// which reads as "no calls" (plan D2).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn tool_call_counts(&self, item: ItemId) -> Result<Vec<ToolCallCount>>;
}

/// Everything a write path needs.
///
/// Implemented by the stores that write where they read — `PgStore`, which needs a connection,
/// and `MemStore` — and by `htui-store`'s `Writer`, which holds one of the two. The read-only
/// mirror does not implement it, so an offline write is a compile error. (Between MOD-2 milestone
/// 4 and MOD-25 an offline sink implemented it too; MOD-25 removed it.)
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
    /// `from` on a legal pair is still `Ok(false)`. `to = closed` is refused from every status
    /// (MOD-38 PRD D1); use [`close_out`](WriteStore::close_out).
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool>;

    /// Appends session events under `fence`, skipping any `(run_step_id, seq)` already stored, and
    /// answers how many rows were actually inserted (`docs/ANA-4.md` §4.1, `docs/ANA-9.md` §4.3,
    /// MOD-40 plan D1).
    ///
    /// One statement: either every new row lands or none does, so a batch naming a step that does
    /// not exist, or a step whose run does not carry `fence`'s lease, writes nothing at all.
    /// Idempotence is the primary key's: a row already stored is skipped and not counted. That is
    /// what makes the recorder's re-offer of a batch whose answer it never got safe, and why such a
    /// replay may answer less than it offered; a **fresh** batch that answers less has met a second
    /// writer, which the recorder reports (MOD-40 plan D3).
    ///
    /// # Errors
    ///
    /// In this order: [`StoreError::Constraint`](crate::store::StoreError::Constraint) when an
    /// event names a `run_step` that does not exist;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) when a named step's run has a
    /// `lease_owner` other than `fence`'s, even if every row is already stored;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `kind` / `role`
    /// outside the §4.3 `CHECK` lists.
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> Result<usize>;

    /// Writes `run_step.usage`, and `run_step.prompt_digest` when `prompt_digest` is `Some`
    /// (`docs/ANA-4.md` §4.1; the digest parameter survives to milestone 9, plan D15(b)).
    ///
    /// `None` leaves the stored digest as it is rather than clearing it: the recorder computes the
    /// digest once, at the prompt, and every later usage write for the same step passes `None`.
    ///
    /// Written only while the step's run carries `fence`'s lease (MOD-40 plan D1).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) when the step does not exist;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) when it does and its run's
    /// `lease_owner` is not `fence`'s.
    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()>;

    /// Creates or edits one `agent` row, keyed by `agent.id` (`docs/ANA-4.md` §4.1, §5.7), as a
    /// compare-and-set on `agent.updated_at` (MOD-40 plan D5, `docs/ANA-16.md` C6) — the
    /// [`set_setting`](WriteStore::set_setting) `App` rung's shape.
    ///
    /// `expected: None` is "I expect no row": every column is inserted as given, `created_at` and
    /// `updated_at` included (the migration's trigger is `BEFORE UPDATE` only). An id that is
    /// already stored is [`CasOutcome::Stale`] with the stored row, and nothing is written — so
    /// an agent seeded or created by another process is never overwritten by a create.
    ///
    /// `Some(t)` is the `updated_at` of the row the caller read and edited. Every column but the
    /// two stamps is written where the stored `updated_at` is still `t`; `created_at` is never
    /// rewritten and `updated_at` becomes the store's clock. A token that no longer matches is
    /// `Stale` with the row as it is now, and nothing is written.
    ///
    /// `Applied` carries the row as stored; its `updated_at` is the next token. Take tokens from
    /// a row the store answered (this outcome, or a registry read), never from a struct the caller
    /// built: Postgres keeps microseconds (MOD-40 blueprint F-17).
    ///
    /// Order, the same on every store: the id and the token first (`Stale`, `NotFound`), then the
    /// name, then the write. A stale edit is `Stale` even when it would also take another agent's
    /// name.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "agent"` for
    /// `Some(_)` on an id no row has; [`StoreError::Constraint`](crate::store::StoreError::Constraint)
    /// when the write would give this id a name another id holds (`agent.name` is `UNIQUE`).
    async fn upsert_agent(
        &self,
        agent: &Agent,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Agent>>;

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
    /// Since MOD-23 (D242) the update writes `enabled` as `row.enabled && !user_off`: a row the
    /// human switched off on this box stays off whatever the probe proposes. The insert is
    /// unchanged, since a fresh row has `user_off = false`.
    /// [`set_agent_box_enabled`](WriteStore::set_agent_box_enabled) is the switch.
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
    /// back to `NULL`, and that is when these two parameters become `Option`s across the five
    /// implementations and `store::conformance`. Widening them before there is a caller would be
    /// five signatures changed to express a case no code can reach.
    ///
    /// # Newest wins (MOD-40 plan D4)
    ///
    /// The write lands only when `quota_at` is at least the stored one: `quota_at IS NULL OR
    /// quota_at <= $quota_at`. Two chats on one box latch the same row from two processes, and a
    /// report that arrives late must not overwrite a newer allowance with an older one. `<=`, not
    /// `<`: a second latch of the same instant rewrites the document, which is how one session
    /// refreshes the spend under an unchanged `observed_at`. Callers pass microseconds, as the
    /// recorder does (`stamp`), so the comparison means the same on every store.
    ///
    /// Answers `true` when the pair was written and `false` when an equal-or-newer `quota_at`
    /// was already stored and nothing was written. `false` is not an error: the latch is
    /// best-effort, and "somebody newer got there first" is the ordering working.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "agent_box"` and
    /// id `"<agent_id>/<box_id>"` when no row has that key, whatever `quota_at` is.
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> Result<bool>;

    /// Switches this agent on or off **on one box** (MOD-23 D242): writes `agent_box.user_off` and
    /// re-derives `agent_box.enabled`, and nothing else. It never writes `probe`, `version`,
    /// `path`, `probed_at`, `quota` or `quota_at`.
    ///
    /// The probe proposes `enabled` and the human vetoes it. `false` sets `user_off` and
    /// `enabled = false`, and while `user_off` holds,
    /// [`upsert_agent_box`](WriteStore::upsert_agent_box) cannot turn `enabled` back on. `true`
    /// clears `user_off` and sets `enabled` to the stored probe's verdict: `true` when the row
    /// holds no probe document, else whether its `status` is `ready` (a document without a
    /// `status` is not ready).
    ///
    /// An absent row is inserted bare: `enabled` as switched, every probe column `NULL`, so a
    /// reader treats it as never probed. No compare-and-set: the switch is an absolute set, and a
    /// token on `agent_box.updated_at` would be spent by every probe. The last of two concurrent
    /// switches wins, and both see it on their re-read.
    ///
    /// The only writer of `user_off` (MOD-2 D74's single-writer shape).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the agent or the box
    /// does not exist; nothing is written.
    async fn set_agent_box_enabled(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        enabled: bool,
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
    /// when a tool name repeats or `spec_digest` is not 64 lowercase hex. `NotFound` for an
    /// unknown box wins over any `Constraint` the same probe would also hit. Either way nothing is
    /// written.
    async fn record_box_probe(&self, probe: &BoxProbe) -> Result<()>;

    /// Every box of this user with its tools and recorded spec digest (MOD-7 D10, D18): boxes by
    /// id, tools by name byte order (`COLLATE "C"`), so every store answers byte for byte.
    ///
    /// # Errors
    ///
    /// Whatever the backend's read fails with.
    async fn boxes(&self) -> Result<Vec<BoxRecord>>;

    /// The box editors' compare-and-set (MOD-7 milestone 2, D41; MOD-41 plan D10): writes the
    /// columns `edit` names, the `executor` key of `box.settings` when `edit.executor` is `Some`,
    /// and `edit_version + 1`, where the row is this user's and its `edit_version` is `expected`,
    /// in one statement. A narrow human writer (MOD-2 D74): never `hostname`, the probe columns,
    /// `htui_version`, any other `settings` key, `machine_fingerprint`, `probe_spec_digest`,
    /// `last_seen_at` or `box_tool`. Registration and the probe never write `edit_version`, so
    /// neither can stale an open editor.
    ///
    /// **Every human writer of `box` is this compare-and-set** (MOD-40 plan D6, `docs/ANA-16.md`
    /// C7). It is `box.settings`' only writer, which admission reads under `claim_run`'s row lock:
    /// key by key (`executor` only), under the same `edit_version` guard, and every other key
    /// (`max_concurrent_items`, `command_limits`, unknown ones) survives. A second, unguarded
    /// `UPDATE box SET settings` would let two editors overwrite each other silently.
    ///
    /// Answers [`CasOutcome::Applied`] with the row as written, or [`CasOutcome::Stale`] with the
    /// row as it is now when `expected` is spent (nothing is written).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "box"` for an
    /// unknown id **or a box of another `app_user`** (the reach of [`boxes`](WriteStore::boxes));
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) carrying
    /// [`canonical_declared_tags`](crate::model::canonical_declared_tags)'s sentence for a tag it
    /// refuses, [`EXECUTOR_MUST_BE_KNOWN`] for an
    /// [`Executor::Other`](crate::model::Executor::Other), or [`BOX_SETTINGS_NOT_AN_OBJECT`] for an
    /// executor asked of a blob that is not a JSON object. Precedence: `NotFound`, then `Stale`,
    /// then `Constraint` (tags, executor, blob), and a refusal writes nothing.
    async fn edit_box(&self, id: BoxId, expected: i32, edit: BoxEdit)
    -> Result<CasOutcome<BoxRow>>;

    /// The stored `app_setting` row keyed
    /// [`BOX_PROBE_SPEC_KEY`](crate::model::BOX_PROBE_SPEC_KEY) and its compare-and-set token
    /// (MOD-51 D2, MOD-7 D52), **unvalidated**: whether the probe would accept the value is
    /// `htui-agent`'s question (`htui_agent::box_probe::spec::effective`), not the store's.
    ///
    /// `None` when there is no row; a row always answers `value: Some`.
    ///
    /// # Errors
    ///
    /// Whatever the backend's read fails with.
    async fn box_probe_spec(&self) -> Result<Option<StoredSetting>>;

    /// Sets, replaces or clears the `box_probe_spec` overlay, a compare-and-set on the row's
    /// `updated_at` (MOD-51 D2, `docs/ANA-16.md` C7). The narrow typed writer beside the `App` rung
    /// of [`set_setting`](WriteStore::set_setting), whose keys are the closed `SettingKey` enum.
    ///
    /// - `overlay: Some(v)`, `expected: None` ("I expect no row"): inserts. A row already there is
    ///   [`CasOutcome::Stale`] carrying it, never an overwrite.
    /// - `overlay: Some(v)`, `expected: Some(t)`: replaces the value where `updated_at = t`.
    /// - `overlay: None`, `expected: Some(t)`: deletes the row where `updated_at = t`, answering
    ///   `Applied(None)`.
    ///
    /// `Applied(Some(row))` carries the value as written and the store's new token. A miss is
    /// `Stale` with the row as it is now, and **`Stale(None)` when the row is gone**. That differs
    /// from `set_setting`'s `NotFound` on purpose: a row deleted under an open editor is a miss
    /// like a spent token (MOD-7 D48), and the editor retries with `expected: None`. Nothing is
    /// written on a miss.
    ///
    /// The store checks only what it can know without the probe. Whether the keys, tools and
    /// patterns mean anything is checked by the caller first (`htui::box_settings::serve`, through
    /// `htui_agent::box_probe::spec::check`).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) carrying
    /// [`BOX_PROBE_SPEC_NOT_AN_OBJECT`] for a `Some(v)` that is not a JSON object, or
    /// [`BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN`] for `None` with `expected: None`. Both are decided
    /// before any read, so they win over `Stale`, and a refusal writes nothing.
    async fn set_box_probe_spec(
        &self,
        overlay: Option<Value>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredSetting>>>;

    /// Mints the `run` / `run_step` pair of a free-standing chat, both `ON CONFLICT (id) DO
    /// NOTHING` (MOD-2 plan D4).
    ///
    /// Both rows are written `status = 'running'` with `finished_at NULL`, which
    /// [`finish_chat_run`](WriteStore::finish_chat_run) closes. The ids are the spec's, minted
    /// client-side, so a start retried after an answer that never arrived converges on one pair of
    /// rows rather than two.
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
    /// The same digest `htui_agent`'s `Recorder::record_prompt_digesting` will later recompute
    /// over the digest text ANA-5 supplies (MOD-33 D271) and hand to
    /// [`set_step_usage`](WriteStore::set_step_usage), so the column's two writers agree by
    /// construction rather than by ordering. The digest text is the sent text with each undigested
    /// span's value replaced by its stand-in; where there is none, as for a chat, it is the same
    /// text.
    ///
    /// Not folded into `set_step_usage`: that one is the **chat** path's digest writer (plan D97),
    /// whose prompt has no template, no sections and no trim record to write.
    ///
    /// Written only while the step's run carries `fence`'s lease (MOD-41 plan D1, as
    /// [`set_step_usage`](WriteStore::set_step_usage) since MOD-40 plan D1).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "run_step"` when
    /// the step does not exist; [`StoreError::Fenced`](crate::store::StoreError::Fenced) when it
    /// does and its run's `lease_owner` is not `fence`'s.
    async fn set_step_prompt(
        &self,
        fence: StepFence,
        step: StepId,
        digest: &str,
        trim: &Value,
    ) -> Result<()>;

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

    /// Inserts this box's checkout path for a repo **only where none exists** (MOD-7 milestone 4,
    /// D104): `Ok(true)` when the row was written, `Ok(false)` when a row for
    /// `(repo_id, box_id)` already existed, which is then left exactly as it was.
    ///
    /// The compare-and-set path inference needs, keyed on the row's **absence**, which no
    /// reconnect changes (the PRD's box-writer constraint). A manual
    /// [`upsert_repo_box_path`](Self::upsert_repo_box_path) racing it either lands first, so this
    /// answers `false`, or overwrites the inferred row, so the manual path wins. `updated_at` is the
    /// store's: the value passed in is ignored on both backends.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row, and nothing is written.
    async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool>;

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
    /// taken `(graph_id, position)` or `(graph_id, name)`, a `graph_id` that names no row, or a
    /// `persona_id` that names no row (`references_no_row("step_graph_phase.persona_id", id,
    /// "persona")`), checked after the position and name clashes (MOD-26 D5).
    async fn create_phase(&self, phase: &StepGraphPhase) -> Result<StepGraphPhase>;

    /// Inserts `agents` as `phase`'s candidate rows, all or nothing; an empty slice writes nothing
    /// and checks nothing (MOD-37 R-6). Insert-only: `override_graph` writes to fresh phases. The
    /// rows may come in any order; the readers sort by `position`.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a row whose `phase_id`
    /// is not `phase` ([`row_names_another_phase`]), a `phase` or an `agent_id` that names no
    /// row, or a `(phase_id, position)` already taken, by a stored row or by another row of the
    /// batch.
    async fn create_phase_agents(&self, phase: PhaseId, agents: &[PhaseAgent]) -> Result<()>;

    /// Edits the six columns of [`PhasePatch`] under CAS; `token_budget` is
    /// [`set_setting`](Self::set_setting)'s on the `Phase` rung and is not here (D8).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a reserved name, a
    /// `(graph_id, position)` / `(graph_id, name)` collision, or a `persona_id` that names no row
    /// (`references_no_row("step_graph_phase.persona_id", id, "persona")`), checked after the
    /// position and name clashes (MOD-26 D5).
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

    // prompt_template (MOD-9 milestone 1, plan D1-D4)

    /// Appends version `head + 1` of `(new.project_id, new.name)` iff the head version is
    /// `expected` (`None`: the name has no row yet), as one compare-and-set. Rows are never
    /// updated or deleted (PRD D5): `step_graph_phase.template_version`, a run snapshot and
    /// `trim_record.template` refer to versions by number. The reads stay inherent
    /// (`MemStore::prompt_templates`), because `prompt_template` is not mirrored.
    ///
    /// Order, the same on every store (plan D4, blueprint D18): the token first, so a spent token
    /// answers `Stale` even for bad input; then the name and the body (`parse` in the role
    /// `TemplateRole::of_name(name)`); then the project and `created_by`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "prompt_template" }`
    /// when `expected` is `Some` and the name has no row;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for an invalid name, a body
    /// `parse` refuses, or a project or `created_by` that names no row. Nothing is written.
    async fn append_prompt_template(
        &self,
        new: NewPromptTemplate,
        expected: Option<i32>,
    ) -> Result<CasOutcome<PromptTemplate>>;

    // skill, skill_version, skill_binding (MOD-9 milestone 3, plan D75-D79)
    //
    // Readers sit beside the writers, as MOD-15's do, so the conformance suite can read back what
    // it wrote on both stores; the not-mirrored rule of this file's header is about `ReadStore`
    // and the mirror, which these do not touch. Every writer is a compare-and-set, and the
    // precedence is `append_prompt_template`'s: the token first, then `NotFound`, then
    // `Constraint` (D75). The refusal sentences are the pure helpers below (D88).

    /// Every skill, ordered by `name` bytes (`COLLATE "C"`).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn skills(&self) -> Result<Vec<Skill>>;

    /// One skill's versions, ascending; empty for an unknown skill.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn skill_versions(&self, skill: SkillId) -> Result<Vec<SkillVersion>>;

    /// `None`: the global attachments; `Some(p)`: `p`'s project and phase attachments. Ordered by
    /// `(skill_id, phase_id)` bytes, `NULL` phase first (D91).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn skill_bindings(&self, project: Option<ProjectId>) -> Result<Vec<SkillBinding>>;

    /// Inserts the skill and its version 1 together (D75, D77, OQ-16).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for
    /// [`new_skill_refusal`]'s sentences, a taken name (`already_exists("skill", name)`), a taken
    /// id, or a `created_by` that names no user. Nothing is written.
    async fn create_skill(&self, new: NewSkill) -> Result<(Skill, SkillVersion)>;

    /// Renames and/or re-describes under CAS on `skill.updated_at` (D76, OQ-17). Order: `NotFound`
    /// (unknown id), then `Stale` (spent token, even for bad input), then `Constraint`.
    ///
    /// # Errors
    /// `NotFound { entity: "skill" }`; `Constraint` for [`skill_patch_refusal`]'s sentences or a
    /// taken name (`already_exists("skill", name)`).
    async fn update_skill(
        &self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
    ) -> Result<CasOutcome<Skill>>;

    /// Appends version `expected + 1` iff the head is `expected` (`0`: "the skill has no version",
    /// a hand-written or imported row). Order (D89): a head other than `expected` is `Stale(head)`;
    /// then `NotFound { entity: "skill" }`; then, with no version at all and `expected != 0`,
    /// `NotFound { entity: "skill_version", id: skill_version_key(skill, expected) }`; then
    /// `Constraint`. `skill.updated_at` is not touched.
    ///
    /// # Errors
    /// As above; `Constraint` for [`skill_body_refusal`]'s sentences or an unknown `created_by`.
    async fn add_skill_version(
        &self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
    ) -> Result<CasOutcome<SkillVersion>>;

    /// Attaches, changes or detaches the one row at `key` (D78). `expected: None` is "I expect no
    /// row"; `Some(t)` is that row's `updated_at`. Order (D90): the row at `key` is read and a
    /// token that does not match it answers `Stale(row or None)`; then `NotFound` for the skill,
    /// the project, the phase (`entity`: `"skill"`, `"project"`, `"step_graph_phase"`); then
    /// `Detach` deletes (no row: `Applied(None)`); then [`check_attachment`]'s `Constraint`s; then
    /// the write. `Applied` carries the row as stored (`None` after a detach).
    ///
    /// # Errors
    /// As above.
    async fn set_skill_binding(
        &self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> Result<CasOutcome<Option<SkillBinding>>>;

    // persona (MOD-26 milestone 1, plan D4; milestone 2, D14)
    //
    // A global registry like `skill`, read and written here for the skill block's reason
    // (above). `step_graph_phase.persona_id` is `ON DELETE RESTRICT`, and `delete_persona` refuses a
    // bound persona first with [`persona_is_bound`]'s sentence, so the constraint is never what the
    // caller reads. The refusal sentences are `model::persona`'s pure helpers, re-exported below
    // (plan D3), so both stores word them once.

    /// Every persona, ordered by `name` bytes (`COLLATE "C"`).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn personas(&self) -> Result<Vec<Persona>>;

    /// Inserts one persona; both stamps are the store's clock (plan D4). Order:
    /// [`new_persona_refusal`]'s sentences, then a taken id (`already_exists("persona", id)`),
    /// then a taken name (`already_exists("persona", name)`).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) as above. Nothing is
    /// written.
    async fn create_persona(&self, new: NewPersona) -> Result<Persona>;

    /// Edits a persona under CAS on `persona.updated_at` (plan D4), `update_skill`'s order:
    /// `NotFound { entity: "persona" }` for an unknown id, then `Stale(current)` for a spent token
    /// (even with bad input), then `Constraint` for [`persona_patch_refusal`]'s sentences or a
    /// taken name (`already_exists("persona", name)`). An all-`None` patch still stamps
    /// `updated_at`. A started run never sees the edit: it reads its snapshot (I-3).
    ///
    /// # Errors
    /// As above.
    async fn update_persona(
        &self,
        id: PersonaId,
        expected: DateTime<Utc>,
        patch: PersonaPatch,
    ) -> Result<CasOutcome<Persona>>;

    /// Deletes a persona no phase binds (MOD-26 milestone 2 D14, I-9). No compare-and-set token,
    /// as [`delete_item_kind`](WriteStore::delete_item_kind). Order: `NotFound { entity:
    /// "persona" }` for an unknown id; then [`persona_is_bound`]'s sentence while any
    /// `step_graph_phase` names it, override graphs included; then the delete. A started run never
    /// needs the row: it reads its snapshot (M1 I-3).
    ///
    /// # Errors
    /// As above. A refusal writes nothing.
    async fn delete_persona(&self, id: PersonaId) -> Result<()>;

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
    /// run is not `queued` or its `target_box_id` is not `box_id`; [`Claim::MissingTags`] when the
    /// run's item requires a tag in neither the box's `probed_tags` nor its `declared_tags`
    /// (`R-ORCH-10`, MOD-7 milestone 3 D80, D81: exact bytes, the missing tags in byte order and
    /// deduplicated; a run with no item has none); [`Claim::SlotFull`] when the box already runs
    /// its limit of **`running`** runs
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
    /// `started_at = at`, `lease_box_id = box_id`, `lease_owner = owner`, and `lease_expires_at`
    /// the **store's** clock plus `ttl` (MOD-40 plan D10: Postgres's `clock_timestamp()`), and the
    /// item `queued -> in_progress`. On [`Claim::MissingTags`] the run moves `queued -> failed`
    /// with `failure` = [`missing_tags_failure`](crate::model::missing_tags_failure) of the list
    /// and `finished_at = at`, and its item `queued -> blocked`, in the same transaction;
    /// `executing_box_id`, `started_at` and the lease stay unset, so no slot is taken. Every other
    /// answer writes nothing.
    ///
    /// `at` is the caller's clock, like every other stamp of the run's timeline; only the lease is
    /// the store's, because only the lease is compared by another process (MOD-40 blueprint B25).
    ///
    /// The two predicates range over two different sets, and `awaiting_approval` is where they
    /// part: a parked run consumes no compute and so holds no slot, but it still owns its trees
    /// and its unmerged branch and so still refuses an overlapping scope (§4.7, invariant 6).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` outside
    /// [`lease_ttl_micros`]'s range, before anything is read;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown run (`"run"`)
    /// or box (`"box"`), the run looked up first.
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> Result<Claim>;

    /// ANA-2 §4.9's heartbeat: `UPDATE run SET lease_expires_at = <store now> + ttl WHERE id = run
    /// AND lease_owner = owner`. `Ok(false)` = zero rows = abandon; the run exists but is not
    /// ours. The expiry is the store's clock (MOD-40 plan D10) and is not returned: the caller
    /// fences on its own clock, from the instant it sent the refresh (plan OQ-2).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` out of range;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`.
    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool>;

    /// ANA-2 §4.9's sweep: every `running` run of `kind = 'graph'` whose `executing_box_id` is
    /// `box_id` and whose lease is `NULL` or expired **by the store's clock** becomes ours
    /// (`lease_owner = owner`, `lease_expires_at` = the store's clock plus `ttl`); never a chat
    /// run (MOD-24 D3b). Returns the adopted rows in `queued_at` order, ties broken by `id` so the
    /// order is total and the same on every backend; empty when nothing was abandoned. A box that
    /// does not exist adopts nothing (`Ok(vec![])`).
    ///
    /// **Never** a run whose `lease_owner` is `owner` (plan D88): a process whose heartbeat
    /// stalled past its TTL must not adopt its own live walk and run it twice under one owner.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` out of range;
    /// the backend's own failures.
    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>>;

    /// Plan D87: the lease of a run that is ours or free, taken before a command's first write on
    /// a parked or running run. `UPDATE run SET lease_owner = owner, lease_box_id = box_id,
    /// lease_expires_at = <store now> + ttl WHERE id = run AND status IN
    /// ('running','awaiting_approval') AND executing_box_id = box_id AND (lease_owner = owner OR
    /// lease_owner IS NULL OR lease_expires_at IS NULL OR lease_expires_at <= <store now>)`.
    ///
    /// `Ok(false)` = zero rows: another owner holds a lease live by the store's clock, or the run
    /// is not takeable here (not `running`/`awaiting_approval`, or executing on another box).
    /// Nothing is written then.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` out of range;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`, told
    /// apart from "not takeable" by one follow-up read.
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool>;

    /// Plan D139: gives a lease back. `UPDATE run SET lease_owner = NULL, lease_expires_at =
    /// <store now> WHERE id = run AND lease_owner = owner`. `Ok(false)` = zero rows = not ours,
    /// and nothing is written.
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
    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool>;

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

    /// Writes the settle columns of [`StepOutcome`]; never `status`. Only while the step's run
    /// carries `fence`'s lease (MOD-40 plan D1).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) when the step exists and its run's
    /// `lease_owner` is not `fence`'s.
    async fn finish_step(&self, fence: StepFence, step: StepId, outcome: StepOutcome)
    -> Result<()>;

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
    /// Written only while the step's run carries `fence`'s lease (MOD-41 plan D1, as
    /// [`set_step_usage`](WriteStore::set_step_usage) since MOD-40 plan D1).
    ///
    /// # Errors
    /// In this order: [`StoreError::NotFound`](crate::store::StoreError::NotFound)
    /// `{ entity: "run_step" }`; [`StoreError::Fenced`](crate::store::StoreError::Fenced) when the
    /// step's run has a `lease_owner` other than `fence`'s;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when a row's
    /// `run_step_id` is not `step` or names an unknown repo.
    async fn upsert_step_tree(
        &self,
        fence: StepFence,
        step: StepId,
        trees: &[RunStepTree],
    ) -> Result<()>;

    /// `R-ORCH-11`'s two hashes, upserted on `(run_step_id, repo_id)`; same fence and refusals as
    /// [`upsert_step_tree`](WriteStore::upsert_step_tree).
    ///
    /// # Errors
    /// In this order: [`StoreError::NotFound`](crate::store::StoreError::NotFound)
    /// `{ entity: "run_step" }`; [`StoreError::Fenced`](crate::store::StoreError::Fenced) when the
    /// step's run has a `lease_owner` other than `fence`'s;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when a row's
    /// `run_step_id` is not `step` or names an unknown repo.
    async fn record_commits(
        &self,
        fence: StepFence,
        step: StepId,
        commits: &[RunStepCommit],
    ) -> Result<()>;

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

    /// MOD-37 R-5, ANA-2 §4.2's "done + skipped" cell: the step `running -> done` with
    /// `gate_outcome = 'skipped'`, `gate_note = note` when `note` is `Some` (kept otherwise) and
    /// the first `finished_at` (`COALESCE(finished_at, at)`). One statement, under `fence`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`, then
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) when the step's run does not carry
    /// `fence`'s lease. `Ok(false)`: the step is not `running`, and nothing is written.
    async fn pass_step(
        &self,
        fence: StepFence,
        step: StepId,
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<bool>;

    /// MOD-37 R-5, ANA-2 §4.2's park, one transaction and [`promote_step`](Self::promote_step)'s
    /// shape: the step and its run `running -> awaiting_approval`, its item
    /// `in_progress -> awaiting_approval`. An item at any other status is left alone and the answer
    /// is still [`ParkOutcome::Parked`] (plan D17); a chat run has no item. No instant moves:
    /// `running -> awaiting_approval` stamps nothing, and `updated_at` is the store's.
    ///
    /// Decided before the first write, in this order: the step exists, its run carries `fence`'s
    /// lease, the step is `running` ([`ParkOutcome::StepMoved`]), the run is `running`
    /// ([`ParkOutcome::RunMoved`]). A refusal writes nothing.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced).
    async fn park_step(&self, fence: StepFence, step: StepId) -> Result<ParkOutcome>;

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
    /// version, the commits upserted, and the item set to `closed` with `resolution` and
    /// `closed_at`. Close-out is the **only** way into `closed` (MOD-38 PRD D1; it amends ANA-2
    /// §4.3); its law is [`Resolution::closes_from`], not [`legal_move`], so an `open` item closes
    /// as one of the four non-success resolutions (ANA-11 §4.2). Refused while any run of the item
    /// is active. Guard order: NotFound, live run, summary kind, summary item, `closes_from`,
    /// commit steps.
    ///
    /// # Errors
    /// Its own refusals, plus - because it performs their work - every refusal of
    /// [`record_commits`](WriteStore::record_commits) and
    /// [`write_document`](WriteStore::write_document). In full:
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "item" }`, or
    /// `{ entity: "run_step" }` for a commit row naming a step that does not exist;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when a run of the item is
    /// `queued | running | awaiting_approval`, when `summary.kind != "summary"`, when
    /// `summary.item_id != item`, when `!resolution.closes_from(status)`
    /// ([`resolution_not_closable`]), when a commit row names an unknown repo, or when the summary
    /// duplicates a document id or names an unknown `created_by` / `produced_by_step_id`. Any
    /// refusal writes nothing: every one of these is decided before the first write.
    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document>;

    /// Inserts an `item_note`; the refusal notes of ANA-2 invariant 7.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on an unknown item,
    /// author, box or step (foreign keys) or a duplicate id.
    async fn add_note(&self, note: NewNote) -> Result<Note>;

    // ---- ANA-11 §5.1: requirements and citations (MOD-38) ------------------------------------

    /// Compare-and-set write of the spec header (plan D9): `expected_version: None` inserts
    /// version 1 when the project has no header, `Some(v)` updates the header at `v` to `v + 1`.
    /// A token that does not match the stored row (including `None` when a row exists) is
    /// `Ok(Stale(row as it is))`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound)
    /// `{ entity: "requirement_spec" }` for `Some(_)` with no header;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for an unknown project or
    /// owner.
    async fn set_requirement_spec(
        &self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
    ) -> Result<CasOutcome<RequirementSpec>>;

    /// Inserts one `requirement_area`.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a code outside
    /// `^[A-Z][A-Z0-9]{1,15}$` ([`invalid_area_code`]), a code the project already has, a
    /// duplicate id or an unknown project.
    async fn create_requirement_area(&self, new: NewRequirementArea) -> Result<RequirementArea>;

    /// Mints the area's next number, the requirement and its revision 1 (`reason = "created"`)
    /// in one statement; a refused mint consumes no number (plan D8).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound)
    /// `{ entity: "requirement_area" }` for an unknown area;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a duplicate id or a
    /// `created_by` / `box_id` that names no row.
    async fn mint_requirement(
        &self,
        area: RequirementAreaId,
        new: NewRequirement,
    ) -> Result<Requirement>;

    /// Compare-and-set amend, one transaction (PRD D3): the row at `version + 1`, its revision
    /// with `amended_by_item_id = amended_by`, and `amended_by`'s `amends` citation upserted at the
    /// new version (a tombstone revived). Checked in the order NotFound, divergence, Constraint.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "requirement" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a withdrawn
    /// requirement ([`requirement_withdrawn`]) or an `amended_by` / author / box that names no
    /// row.
    async fn amend_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        amended_by: ItemId,
    ) -> Result<RequirementUpdate>;

    /// [`amend_requirement`](WriteStore::amend_requirement) for `state = withdrawn`: revision
    /// `reason = "withdrawn"`, and `withdrawn_by`'s `withdraws` citation at the new version.
    ///
    /// # Errors
    /// As [`amend_requirement`](WriteStore::amend_requirement), an already-withdrawn requirement
    /// included.
    async fn withdraw_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        withdrawn_by: ItemId,
        author_id: UserId,
        box_id: Option<BoxId>,
    ) -> Result<RequirementUpdate>;

    /// Upserts a live citation stamped at the requirement's current version, reviving a tombstone
    /// and overwriting `proposed_by_step_id` (plan D10).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `"item"` then
    /// `"requirement"`; [`StoreError::Constraint`](crate::store::StoreError::Constraint) for
    /// `addresses`/`reserves` of a withdrawn requirement ([`withdrawn_requirement_cited`]) or an
    /// unknown step.
    async fn cite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        proposed_by: Option<StepId>,
    ) -> Result<ItemRequirement>;

    /// Tombstones a live citation (`deleted_at = now`).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound)
    /// `{ entity: "item_requirement", id: citation_key(..) }` ([`citation_key`]) when no live row
    /// matches.
    async fn uncite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<()>;

    /// Re-stamps a live citation at the requirement's current version, clearing `suspect`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound)
    /// `{ entity: "item_requirement", id: citation_key(..) }` ([`citation_key`]) when no live row
    /// matches; then [`StoreError::Constraint`](crate::store::StoreError::Constraint) for
    /// `addresses`/`reserves` of a withdrawn requirement ([`withdrawn_requirement_cited`]), as
    /// [`cite`](Self::cite) refuses it (plan D10).
    async fn reconfirm(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> Result<ItemRequirement>;

    // -- MOD-42: the permission and control relay (plan D1-D5, D12-D14)

    /// D1, D5, B-9: parks one stage-3 request. In one transaction: the step and its run's lease
    /// (`NotFound { entity: "run_step" }`; `Constraint` when the step is not `open.run_id`'s;
    /// `Fenced { step }` when `run.lease_owner` is not `open.owner`), then every `pending` or
    /// `answered` row of the same step from **another session** moves to `stale` (D5), then the
    /// insert. Answers `open.id`. `created_at` is the store's clock (I-4).
    ///
    /// # Errors
    /// The three above; `Constraint` for a repeated id or a repeated `(session, request_id)`.
    async fn open_permission(&self, open: OpenPermission) -> Result<PermissionId>;

    /// One row by id; `None` for an id no row has.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn permission(&self, id: PermissionId) -> Result<Option<StepPermission>>;

    /// D4: compare-and-set `answered → applied`, only while `run.lease_owner = owner` (B-9: owner
    /// only, as a step fence). `Some(choice)` = applied now; `None` = not `answered`, not
    /// `owner`'s row, or the run's lease is not `owner`'s. `resolved_at` is the store's clock.
    ///
    /// # Errors
    /// `NotFound { entity: "step_permission" }` for an unknown id, told apart by one re-read.
    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> Result<Option<PermissionChoice>>;

    /// D5, I-7: every `pending` or `answered` row of `session` moves to `to` (`Cancelled` or
    /// `Stale`), `resolved_at` the store's clock. Answers how many moved.
    ///
    /// # Errors
    /// `Constraint` for any other `to`, before anything is written.
    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> Result<u64>;

    /// D12: one `pending` cancel per run. Writes `(id, run, 'cancel', user, box_id)` unless a
    /// pending cancel exists. Never reads the run's status: the caller decides (B-20).
    ///
    /// # Errors
    /// `NotFound { entity: "run" }`; `Constraint` for an unknown user or box (B-7).
    async fn request_cancel(
        &self,
        run: RunId,
        user: UserId,
        box_id: BoxId,
    ) -> Result<CancelRequest>;

    /// D13, B-4: the `pending` commands this process applies, `(issued_at, id)` order: runs whose
    /// `lease_owner = owner`; runs executing on `box_id` whose lease is free or expired by the
    /// store's clock and whose status is `running` or `awaiting_approval`; and runs executing on
    /// `box_id` that are already terminal (the caller refuses those with their status).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn pending_commands(&self, owner: Uuid, box_id: BoxId) -> Result<Vec<RunCommand>>;

    /// D12, D13: compare-and-set `pending → to` (`Applied` or `Refused`), with `resolution`,
    /// `resolved_at` the store's clock. `Ok(false)` = not pending any more (I-3).
    ///
    /// # Errors
    /// `Constraint` for `to = Pending`, before anything is read; `NotFound { entity:
    /// "run_command" }` for an unknown id.
    async fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> Result<bool>;

    /// D14: the item's `pending` requests whose owner holds the run's lease live by the store's
    /// clock, and its non-terminal runs with a `pending` cancel. A read on `WriteStore` by the
    /// `command_runs` precedent: neither table is mirrored (OQ-4).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn relay_view(&self, item: ItemId) -> Result<RelayView>;

    /// D3: compare-and-set `pending → answered`, only while `option_id` is one of the row's
    /// options and the row's `owner` holds the run's lease live by the store's clock. Never
    /// touches `run`, the lease or `session_event` (I-1). A loser is `Refused` with the actual
    /// state, decided by one re-read in this order: status (not `pending`), then `NotOffered`,
    /// then `ExecutorGone`.
    ///
    /// # Errors
    /// `NotFound { entity: "step_permission" }`; `Constraint` for an unknown user or box.
    async fn answer_permission(
        &self,
        id: PermissionId,
        option_id: &str,
        user: UserId,
        box_id: BoxId,
    ) -> Result<AnswerOutcome>;

    // -- MOD-11: agent writes (plan D13, B-4..B-6) ------------------------------------------

    /// D13: [`write_document`](Self::write_document) for a step's own item under its fence. One
    /// transaction; the step's and its run's rows `FOR SHARE` first, then the item `FOR UPDATE`
    /// (step → run → item, the `park_step` order). Several calls write several versions ("newest
    /// wins").
    ///
    /// # Errors
    /// `Constraint(document_needs_a_step())` when `produced_by_step_id` is `None`, then
    /// `Constraint(step_document_refusal(..))` for a NUL (both before any read);
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound)
    /// `{ entity: "run_step" }`; [`StoreError::Fenced`](crate::store::StoreError::Fenced)
    /// `{ step }`; `Constraint(step_writes_own_item(..))` when the run's `item_id` is not
    /// `new.item_id`; then `write_document`'s own errors.
    async fn write_step_document(&self, fence: StepFence, new: NewDocument) -> Result<Document>;

    /// D13: [`add_note`](Self::add_note) with `via_step_id` required, on the step's own item,
    /// under its fence. Same order of refusals as
    /// [`write_step_document`](Self::write_step_document) (`note_needs_a_step()` first).
    ///
    /// # Errors
    /// `Constraint(note_needs_a_step())`; `Constraint(step_note_refusal(..))` for a NUL in the
    /// body; `NotFound { entity: "run_step" }`; `Fenced { step }`;
    /// `Constraint(step_writes_own_item(..))`; then `add_note`'s own errors.
    async fn add_step_note(&self, fence: StepFence, note: NewNote) -> Result<Note>;

    /// D13, B-6: upserts a live `item_link` proposed by `link.step`; revives a tombstone with the
    /// new proposer, keeps a live row's proposer. `updated_at` is the trigger's (Pg) / the clock's
    /// (Mem).
    ///
    /// # Errors
    /// `Constraint(self_link(..))` when `from == to` (before any read); `NotFound { run_step }`;
    /// `Fenced`; `Constraint(step_writes_own_item(..))` when `from` is not the run's item;
    /// `NotFound { entity: "item" }` for `to`; `Constraint(link_outside_project(..))` when `to` is
    /// in another project than the run.
    async fn propose_link(&self, fence: StepFence, link: ProposeLink) -> Result<ItemLink>;

    /// D13, B-5: tombstones the live link `(from, to, kind)` when its `proposed_by_step_id` is a
    /// step of `link.step`'s run. Answers the tombstoned row.
    ///
    /// # Errors
    /// `NotFound { run_step }`; `Fenced`; `Constraint(step_writes_own_item(..))`; then
    /// `NotFound { entity: "item_link", id: link_key(..) }` when no live row matches, else
    /// `Constraint(link_not_proposed_by_run(..))`.
    async fn withdraw_link(&self, fence: StepFence, link: WithdrawLink) -> Result<ItemLink>;

    /// B-4: the item of `project` whose `key` is `key`; `None` when there is none, a key holding
    /// a NUL included (answered before any read: no key holds one, and Postgres would fail the
    /// parameter, `22021`). A read on `WriteStore` by the `command_runs` precedent:
    /// `WorkerStore`'s reads come from here.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn item_by_key(&self, project: ProjectId, key: &str) -> Result<Option<ItemId>>;
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

/// MOD-9 D4: a template name [`PromptTemplate::name_is_valid`] refuses, in the sentence both
/// stores give it.
#[must_use]
pub fn invalid_template_name(name: &str) -> String {
    format!(
        "prompt_template.name `{}` must be non-empty, single-line, trimmed and free of NUL",
        name.escape_debug()
    )
}

/// MOD-9 D17: the `NotFound` id of a `(project, name)` pair, so both stores spell it alike.
#[must_use]
pub fn prompt_template_key(project: ProjectId, name: &str) -> String {
    format!("{project}/{name}")
}

/// MOD-9 D4, D17: why a template may not be saved, or `None` when it may. The name rule first,
/// then a body with U+0000 (which `parse` accepts and Postgres `text` cannot hold), then [`parse`]
/// in the name's role; that last sentence is [`TemplateError`](crate::prompt::TemplateError)'s
/// `Display`.
#[must_use]
pub fn prompt_template_refusal(name: &str, body: &str) -> Option<String> {
    if !PromptTemplate::name_is_valid(name) {
        return Some(invalid_template_name(name));
    }
    if body.contains('\0') {
        return Some("prompt_template.body must not contain a NUL character".to_owned());
    }
    parse(TemplateRole::of_name(name), body)
        .err()
        .map(|err| err.to_string())
}

// ---- MOD-26: the persona writers' refusals (plan D3) live in `model::persona`, where the
// persona-file reader needs them too; re-exported so the store's refusal vocabulary is one list.
pub use crate::model::persona::{
    BLANK_PERSONA_BODY, MODEL_REFUSED, PersonaNotInSnapshot, RULE_MATCHES_EVERYTHING,
    allow_names_an_mcp_tool, invalid_persona_name, kind_not_narrowable, new_persona_refusal,
    not_a_tool_name, persona_patch_refusal, persona_refusal, rule_kind_unknown,
};

// ---- MOD-9 milestone 3: the skill writers' refusals (plan D71-D79, blueprint D88) -------------
//
// Every rule and sentence of the four skill writers lives here, so `MemStore` and `PgStore` only
// look facts up and refuse the same input with the same sentence (R-32).

/// MOD-41 plan D10: [`WriteStore::edit_box`]'s refusal of an
/// [`Executor::Other`](crate::model::Executor::Other), so the editor writes only the two values
/// this build knows.
pub const EXECUTOR_MUST_BE_KNOWN: &str = "executor must be tui or worker";

/// MOD-41 plan D10: [`WriteStore::edit_box`]'s refusal to write `executor` into a `box.settings`
/// blob that is not a JSON object (Postgres' `jsonb_set` errors on a scalar or an array).
pub const BOX_SETTINGS_NOT_AN_OBJECT: &str = "box.settings is not a JSON object";

/// MOD-51 D2: [`WriteStore::set_box_probe_spec`]'s refusal of an overlay that is not a JSON
/// object. The probe would ignore one too; the store refuses it without the probe's help.
pub const BOX_PROBE_SPEC_NOT_AN_OBJECT: &str = "box_probe_spec is not a JSON object";

/// MOD-51 D2: [`WriteStore::set_box_probe_spec`]'s refusal of a clear with no token: a delete is
/// a compare-and-set on the row it deletes, and "expect no row" has nothing to delete.
pub const BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN: &str =
    "box_probe_spec clear needs the updated_at of the row it clears";

/// MOD-9 D71: a name [`validate_name`] refuses.
#[must_use]
pub fn invalid_skill_name(name: &str) -> String {
    format!(
        "skill.name `{}` must be 1-64 of a-z, 0-9 and single inner hyphens",
        name.escape_debug()
    )
}

/// MOD-9 D77 (OQ-20): a blank body renders an empty `<skill>` block that costs tokens and says
/// nothing.
pub const BLANK_SKILL_BODY: &str = "a skill needs text";

/// A `text` column Postgres cannot hold (`22021`), refused by rule on both stores.
#[must_use]
pub fn has_nul(column: &str) -> String {
    format!("{column} must not contain a NUL character")
}

/// D77: why a body may not be stored — blank first, then a NUL (`skill_version.body`).
#[must_use]
pub fn skill_body_refusal(body: &str) -> Option<String> {
    if body.trim().is_empty() {
        return Some(BLANK_SKILL_BODY.to_owned());
    }
    body.contains('\0').then(|| has_nul("skill_version.body"))
}

/// D71, D77: `create_skill`'s input, in order: name, description NUL (`skill.description`), body.
#[must_use]
pub fn new_skill_refusal(name: &str, description: &str, body: &str) -> Option<String> {
    if !validate_name(name) {
        return Some(invalid_skill_name(name));
    }
    if description.contains('\0') {
        return Some(has_nul("skill.description"));
    }
    skill_body_refusal(body)
}

/// D76: `update_skill`'s input: the name (if any), then the description's NUL (if any).
#[must_use]
pub fn skill_patch_refusal(patch: &SkillPatch) -> Option<String> {
    if let Some(name) = &patch.name
        && !validate_name(name)
    {
        return Some(invalid_skill_name(name));
    }
    patch
        .description
        .as_deref()
        .is_some_and(|description| description.contains('\0'))
        .then(|| has_nul("skill.description"))
}

/// The `NotFound` id of a missing version: `"{skill}/v{version}"`.
#[must_use]
pub fn skill_version_key(skill: SkillId, version: i32) -> String {
    format!("{skill}/v{version}")
}

/// D78: a phase key with no project.
#[must_use]
pub fn phase_attachment_needs_a_project(phase: PhaseId) -> String {
    format!("a phase attachment needs its project: step_graph_phase {phase} was given none")
}

/// D78: the phase's graph belongs to another project ([`graph_not_in_project`]'s shape).
#[must_use]
pub fn phase_not_in_project(phase: PhaseId, project: ProjectId) -> String {
    format!("step_graph_phase {phase} is not in project {project}")
}

/// D78: a pin to a version the skill does not have.
#[must_use]
pub fn pin_names_no_version(skill: &str, version: i32) -> String {
    format!("skill_binding.pinned_version `{version}` names no version of skill `{skill}`")
}

/// D78: `position < 0`.
#[must_use]
pub fn negative_position(position: i32) -> String {
    format!("skill_binding.position `{position}` must be 0 or more")
}

/// D78: a qualified glob on a global row.
#[must_use]
pub fn global_glob_names_a_repo(glob: &str, repo: &str) -> String {
    format!("glob `{glob}` names repo `{repo}`, but a global attachment belongs to no project")
}

/// D79 (OQ-18), verbatim.
#[must_use]
pub fn glob_names_unknown_repo(glob: &str, repo: &str, project_slug: &str) -> String {
    format!("glob `{glob}` names repo `{repo}`, which project `{project_slug}` does not have")
}

/// D78: `activation = glob` with no effective glob (the DB's glob-needs-globs check backs it).
pub const GLOB_NEEDS_GLOBS: &str =
    "an attachment with activation `glob` needs at least one glob or language";

/// What a store looked up before [`check_attachment`] (D88): the facts the rule needs, so the
/// rule itself is pure and has one definition.
#[derive(Debug, Clone, Copy)]
pub struct BindingFacts<'a> {
    /// The key being written.
    pub key: SkillBindingKey,
    /// The project owning `key.phase`'s graph (`None` when `key.phase` is `None`).
    pub phase_project: Option<ProjectId>,
    /// `skill.name`, for the pin sentence.
    pub skill_name: &'a str,
    /// Every version number the skill has.
    pub versions: &'a [i32],
    /// `key.project`'s repo names (empty for a global key).
    pub repos: &'a [String],
    /// `key.project`'s slug (empty for a global key), for D79's sentence.
    pub project_slug: &'a str,
}

/// The columns [`check_attachment`] derives: what is stored beside the attachment's own three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAttachment {
    /// `canonical_globs(attachment.globs, attachment.languages)`.
    pub globs: Vec<String>,
    /// `skill_language::normalise(attachment.languages)`.
    pub languages: Vec<String>,
}

/// D78's `Constraint` chain, in order, first refusal wins:
/// 1. `key.phase` without `key.project` → [`phase_attachment_needs_a_project`];
/// 2. `phase_project != key.project` → [`phase_not_in_project`];
/// 3. `pinned_version: Some(n)` not in `versions` → [`pin_names_no_version`];
/// 4. `position < 0` → [`negative_position`];
/// 5. [`canonical_globs`] → the
///    [`GlobError`](crate::model::GlobError)'s `Display` (languages, then globs);
/// 6. per canonical glob with a qualifier: global key → [`global_glob_names_a_repo`]; repo not in
///    `repos` → [`glob_names_unknown_repo`];
/// 7. `Glob` with no canonical glob → [`GLOB_NEEDS_GLOBS`].
///
/// # Errors
/// The sentence.
pub fn check_attachment(
    facts: &BindingFacts<'_>,
    attachment: &Attachment,
) -> core::result::Result<StoredAttachment, String> {
    let key = facts.key;
    if let Some(phase) = key.phase {
        let Some(project) = key.project else {
            return Err(phase_attachment_needs_a_project(phase));
        };
        if facts.phase_project != Some(project) {
            return Err(phase_not_in_project(phase, project));
        }
    }
    if let Some(pin) = attachment.pinned_version
        && !facts.versions.contains(&pin)
    {
        return Err(pin_names_no_version(facts.skill_name, pin));
    }
    if attachment.position < 0 {
        return Err(negative_position(attachment.position));
    }
    let globs =
        canonical_globs(&attachment.globs, &attachment.languages).map_err(|err| err.to_string())?;
    for glob in &globs {
        let parsed = SkillGlob::parse(glob).map_err(|err| err.to_string())?;
        let Some(repo) = parsed.repo else {
            continue;
        };
        if key.project.is_none() {
            return Err(global_glob_names_a_repo(glob, &repo));
        }
        if !facts.repos.contains(&repo) {
            return Err(glob_names_unknown_repo(glob, &repo, facts.project_slug));
        }
    }
    if attachment.activation == Activation::Glob && globs.is_empty() {
        return Err(GLOB_NEEDS_GLOBS.to_owned());
    }
    Ok(StoredAttachment {
        globs,
        languages: skill_language::normalise(&attachment.languages),
    })
}

/// D6: the holder count, in the sentence the refusal carries.
///
/// The `item.kind_id` FK would refuse the delete on Postgres anyway (`0001_init.sql:313`, no
/// cascade), but with a constraint name where the PRD asks for "names what holds it".
#[must_use]
pub fn item_kind_is_held(prefix: &str, items: u64) -> String {
    format!("item_kind {prefix} is held by {items} items")
}

/// How many phases [`persona_is_bound`] names before it counts the rest.
const BOUND_PHASES_NAMED: usize = 5;

/// MOD-26 milestone 2 D14 (I-9): `delete_persona`'s refusal while phases bind the persona. Each
/// holder is `(project slug, graph name, phase name)`, named `` `<slug>/<graph>/<phase>` ``: a
/// persona is global and a graph name is unique only per project (`0001_init.sql:221`). Sorted by
/// the triple's bytes and deduplicated **here**, so both stores word it the same whatever order
/// they read in; at most five, then "and n more".
#[must_use]
pub fn persona_is_bound(name: &str, holders: &[(String, String, String)]) -> String {
    let mut holders: Vec<&(String, String, String)> = holders.iter().collect();
    holders.sort_unstable();
    holders.dedup();
    let named: Vec<String> = holders
        .iter()
        .take(BOUND_PHASES_NAMED)
        .map(|(project, graph, phase)| {
            format!("`{}`", format!("{project}/{graph}/{phase}").escape_debug())
        })
        .collect();
    let more = holders.len().saturating_sub(BOUND_PHASES_NAMED);
    let more = if more == 0 {
        String::new()
    } else {
        format!(" and {more} more")
    };
    let (noun, them) = if holders.len() == 1 {
        ("phase", "it")
    } else {
        ("phases", "them")
    };
    format!(
        "persona `{}` is bound to {} {noun} ({}{more}); clear {them} in Settings \u{203a} Kinds \
         first",
        name.escape_debug(),
        holders.len(),
        named.join(", ")
    )
}

// ---- MOD-4: the refusals of ANA-2 §8's writers ------------------------------------------------
//
// Same reason as the five above: a rule the schema cannot express is spelled once here, so
// `MemStore` and `PgStore` refuse the same input with the same sentence. The FK refusals reuse
// `references_no_row`, which is the sentence `MemStore` already gave `run.project_id` and
// `session_event.run_step_id` before this milestone gave it a name.

/// The longest lease TTL a store accepts: 365 days, the bound `htui-orch`'s `LeaseTimes::from_app`
/// already clamps `lease_ttl_seconds` to, so `now + ttl` never overflows an instant and the µs
/// count Postgres multiplies stays below 2^53, where a `float8` product is exact (MOD-40 blueprint
/// P-13, B27).
pub const MAX_LEASE_TTL: TimeDelta = TimeDelta::seconds(365 * 24 * 60 * 60);

/// MOD-40 plan D10 (blueprint B27): a lease TTL as the whole microseconds both stores add to their
/// own clock, or the one refusal both give.
///
/// Sub-microsecond parts are dropped (`TIMESTAMPTZ` has none), so Postgres, which binds the
/// count, and `MemStore`, which adds `TimeDelta::microseconds(count)`, add the same span.
///
/// # Errors
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) naming the TTL when it is
/// negative or longer than [`MAX_LEASE_TTL`]. Checked before any row is read, so an out-of-range
/// TTL on an unknown run is this refusal, not `NotFound`.
pub fn lease_ttl_micros(ttl: TimeDelta) -> Result<i64> {
    match ttl.num_microseconds() {
        Some(micros) if ttl >= TimeDelta::zero() && ttl <= MAX_LEASE_TTL => Ok(micros),
        _ => Err(crate::store::StoreError::Constraint(format!(
            "lease ttl {ttl} is outside 0 ..= {MAX_LEASE_TTL}"
        ))),
    }
}

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

// ---- MOD-38: ANA-11's refusals ----

/// ANA-11 §4.2: the close-out law refuses this pair (T3).
#[must_use]
pub fn resolution_not_closable(item: ItemId, status: Status, resolution: Resolution) -> String {
    format!("item {item} is `{status}`; it cannot close as `{resolution}` (ANA-11 §4.2)")
}

/// Plan D10: a withdrawn requirement is not amended or withdrawn again (T6).
#[must_use]
pub fn requirement_withdrawn(key: &str) -> String {
    format!("requirement {key} is withdrawn")
}

/// Plan D10: a withdrawn requirement takes no new `addresses` / `reserves` citation (T6).
#[must_use]
pub fn withdrawn_requirement_cited(key: &str, kind: CitationKind) -> String {
    format!("requirement {key} is withdrawn; it takes no new `{kind}` citation")
}

/// The `requirement_area.code` CHECK, in words; both stores check it before the insert (T6).
#[must_use]
pub fn invalid_area_code(code: &str) -> String {
    format!("requirement_area.code `{code}` is not `^[A-Z][A-Z0-9]{{1,15}}$`")
}

/// The `id` of an `item_requirement` `NotFound`: its primary key, `item/requirement/kind` (T6).
#[must_use]
pub fn citation_key(item: ItemId, requirement: RequirementId, kind: CitationKind) -> String {
    format!("{item}/{requirement}/{kind}")
}

// ---- MOD-11: the agent writes' refusals (plan D13, B-4..B-6) ----

/// D13: [`WriteStore::write_step_document`] writes a document a step produced, so it names one.
#[must_use]
pub fn document_needs_a_step() -> String {
    "an agent document names the step that wrote it (produced_by_step_id)".to_owned()
}

/// D13: [`WriteStore::add_step_note`] writes a note a step wrote, so it names one.
#[must_use]
pub fn note_needs_a_step() -> String {
    "an agent note names the step that wrote it (via_step_id)".to_owned()
}

/// D13: the first of an agent document's `text` columns (`kind`, `title`, `body`) that holds a
/// NUL ([`has_nul`]); [`WriteStore::write_step_document`] refuses it on both stores before any
/// read, where Postgres alone would fail it as a backend error (`22021`).
#[must_use]
pub fn step_document_refusal(new: &NewDocument) -> Option<String> {
    [
        ("document.kind", &new.kind),
        ("document.title", &new.title),
        ("document.body", &new.body),
    ]
    .into_iter()
    .find(|(_, text)| text.contains('\0'))
    .map(|(column, _)| has_nul(column))
}

/// D13: [`step_document_refusal`] for [`WriteStore::add_step_note`]'s `item_note.body`.
#[must_use]
pub fn step_note_refusal(note: &NewNote) -> Option<String> {
    note.body.contains('\0').then(|| has_nul("item_note.body"))
}

/// D13: a step writes only on its own run's item (PRD OQ-4).
#[must_use]
pub fn step_writes_own_item(step: StepId, item: ItemId) -> String {
    format!("step {step} may write only on its run's own item, not {item}")
}

/// D13: `item_link`'s `CHECK (from_item_id <> to_item_id)`, in words, decided before any read.
#[must_use]
pub fn self_link(item: ItemId) -> String {
    format!("an item cannot link to itself ({item})")
}

/// D13: an agent links its item only to items of its run's project.
#[must_use]
pub fn link_outside_project(to: ItemId) -> String {
    format!("item {to} is outside the run's project")
}

/// The `id` of an `item_link` `NotFound`: its primary key, `from-kind->to` (D13).
#[must_use]
pub fn link_key(from: ItemId, to: ItemId, kind: LinkKind) -> String {
    format!("{from}-{kind}->{to}")
}

/// D13, B-6: an agent tombstones only a link its own run proposed (PRD OQ-4).
#[must_use]
pub fn link_not_proposed_by_run(key: &str) -> String {
    format!("link {key} was not proposed by this run")
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

/// MOD-37 R-6: a batch of `phase_agent` rows belongs to the phase it is written for.
#[must_use]
pub fn row_names_another_phase(table: &str, row: PhaseId, phase: PhaseId) -> String {
    format!("{table}.phase_id `{row}` is not the phase being written (`{phase}`)")
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
/// `None` is "leave it alone", which is plan D17 rather than an omission: MOD-2's chat path, and
/// before MOD-25 the offline upload path, insert `run` and `item` rows outside §4.3, so an item at
/// a status this table does not list has not necessarily passed through [`Status::can_move_to`]
/// and must not be forced through the law on its way out.
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

/// Which lease a step write is made under (MOD-40 plan D1, PRD D1).
///
/// [`WriteStore::append_events`], [`WriteStore::set_step_usage`], [`WriteStore::finish_step`]
/// (MOD-40), [`WriteStore::set_step_prompt`], [`WriteStore::upsert_step_tree`] and
/// [`WriteStore::record_commits`] (MOD-41 plan D1), [`WriteStore::pass_step`] and
/// [`WriteStore::park_step`] (MOD-37 R-5), [`WriteStore::write_step_document`],
/// [`WriteStore::add_step_note`], [`WriteStore::propose_link`] and [`WriteStore::withdraw_link`]
/// (MOD-11 plan D13) take one and write only while the step's run carries
/// exactly that lease: `run.lease_owner IS NOT DISTINCT FROM` [`StepFence::owner`]. A process
/// whose run another process adopted ([`WriteStore::adopt_runs`], [`WriteStore::take_lease`])
/// still holds its old `Lease`, and the store answers it with
/// [`StoreError::Fenced`](crate::store::StoreError::Fenced) and writes nothing.
///
/// No `Default`: every caller says which one it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepFence {
    /// The walk's own lease: the `owner` it passed to `claim_run`, `take_lease` or `adopt_runs`
    /// (the engine's `parts.owner`). Writes only while `run.lease_owner = owner`.
    Lease(Uuid),
    /// No lease: a chat run, whose `lease_owner` is `NULL`, or a promoted step continued by a chat
    /// after its park released the lease. Refused on a run whose lease names an owner.
    Unleased,
}

impl StepFence {
    /// The `run.lease_owner` this fence writes under: the owner, or `None` for `NULL`.
    #[must_use]
    pub const fn owner(self) -> Option<Uuid> {
        match self {
            Self::Lease(owner) => Some(owner),
            Self::Unleased => None,
        }
    }
}

/// What [`WriteStore::park_step`] found (MOD-37 R-5). A refusal writes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkOutcome {
    /// The step and its run are `awaiting_approval`, and its item too when it was `in_progress`.
    Parked,
    /// The step was not `running`: another writer moved it first.
    StepMoved,
    /// The step was `running` and its run was not: another writer moved the run first.
    RunMoved,
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
/// `workspace_links` and `workspace_box_paths` only. `phase_agents` is counted on both stores
/// since MOD-37 R-6 gave `MemStore` the table, and is `0` on the demo fixture, which seeds none;
/// `run_step_commits`, `run_step_trees` and `command_runs` are counted on both since MOD-4.
/// MOD-38's six requirement fields are counted on both stores.
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
    /// `requirement_spec` rows (0 or 1), MOD-38.
    pub requirement_specs: u64,
    /// `requirement_area` rows.
    pub requirement_areas: u64,
    /// `requirement_key_counter` rows, which cascade from `requirement_area`.
    pub requirement_key_counters: u64,
    /// `requirement` rows.
    pub requirements: u64,
    /// `requirement_revision` rows of the project's requirements.
    pub requirement_revisions: u64,
    /// `item_requirement` rows whose item **or** requirement is in the project, tombstones
    /// included: a cross-project citation goes with either end, as `links` does.
    pub item_requirements: u64,
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

/// One setting as stored, with the token a later [`WriteStore::set_setting`],
/// [`WriteStore::clear_setting`] or [`WriteStore::set_box_probe_spec`] must present.
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
