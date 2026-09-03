//! The store seam of `docs/ANA-9.md` §6.1, quoted verbatim.
//!
//! Method names, parameter names, parameter order and the return types are copied from §6.1, not
//! paraphrased (plan V2). `PgStore: WriteStore`, `CacheStore: ReadStore` only,
//! [`MemStore`](crate::store::MemStore)`: WriteStore`. Nothing is added to these two traits in
//! MOD-1.

use crate::model::{
    DocumentHead, Item, ItemFilter, ItemId, ItemPatch, ItemRevision, ItemSummary, LinkGraph,
    NewItem, Note, RunSummary, Scope, SessionEvent, Status, StepId,
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
    // runs, steps, events, links, notes, documents, skills, templates, box, agents ...
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
