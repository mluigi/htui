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
    ItemFilter, ItemId, ItemPatch, ItemRevision, ItemSummary, LinkGraph, NewItem, Note, Project,
    ProjectId, PromptScope, RunId, RunStatus, RunSummary, Scope, SessionEvent, Status, StepId,
    UpstreamEntry,
};
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
    // links, notes, templates, box ...
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
