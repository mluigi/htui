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
//! to 8 do not reopen this file. The registry read that goes with them, `agents()`, is **not**
//! here: `agent` and `agent_box` are not mirrored (`docs/ANA-9.md` §4.4), so it is inherent on
//! `MemStore` / `PgStore` and dispatched by `Backend`, following the `workspaces` / `box_info` /
//! `active_runs` / `projects` precedent.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{
    Agent, AgentBox, ChatRunSpec, DocumentHead, Item, ItemFilter, ItemId, ItemPatch, ItemRevision,
    ItemSummary, LinkGraph, NewItem, Note, RunId, RunStatus, RunSummary, Scope, SessionEvent,
    Status, StepId,
};
use crate::store::error::Result;

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
}

/// Everything a write path needs. Only a backend that can reach Postgres implements it, so an
/// offline write is a compile error rather than a runtime flag.
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
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the agent or the box
    /// does not exist.
    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()>;

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
    // links, notes, documents, skills, templates, box ...
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
