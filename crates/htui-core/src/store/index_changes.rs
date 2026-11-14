//! The index change read (MOD-159 M2, plan D3): one narrow trait beside [`WriteStore`], as
//! [`IndexerLeaseStore`] is, so the five `ReadStore` implementors do not each grow a read only the
//! indexer's incremental pass makes, and [`ItemFilter`] (serialised, read by the mirror) stays as
//! it is.
//!
//! Two implementors: `MemStore` (`mem.rs`) and `htui-store`'s `PgStore`. Every method is declared
//! `fn … -> impl Future<Output = Result<T>> + Send`, the form of `store/indexer_lease.rs`, and an
//! implementor writes `async fn`.
//!
//! The instant a pass starts from is the store's own clock ([`IndexChangesStore::index_clock`]),
//! never the caller's, and a row counts when its stamp is strictly after it, as the cache cursor's
//! rows do. Item and requirement stamps are the store's; a document's `created_at` is its
//! writer's, so a writer whose clock lags the store's by more than the caller's overlap is left to
//! the full reconcile.
//!
//! [`WriteStore`]: crate::store::WriteStore
//! [`IndexerLeaseStore`]: crate::store::IndexerLeaseStore
//! [`ItemFilter`]: crate::model::ItemFilter

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::model::{IndexChanges, ProjectId};
use crate::store::error::Result;

/// What the indexer's incremental pass reads (MOD-159 M2 plan D3).
pub trait IndexChangesStore: Send + Sync {
    /// The store's clock now (Postgres `clock_timestamp()`, `MemStore`'s clock).
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn index_clock(&self) -> impl Future<Output = Result<DateTime<Utc>>> + Send;

    /// The rows of `projects` written strictly after `since`, by project: items whose
    /// `updated_at` is after it or that gained a document version created after it (listed under
    /// the item's current project), requirements whose `updated_at` is after it, and owner
    /// documents created after it. A project with no such row has no entry; an empty `projects`
    /// answers empty.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn index_changes(
        &self,
        projects: &[ProjectId],
        since: DateTime<Utc>,
    ) -> impl Future<Output = Result<IndexChanges>> + Send;
}
