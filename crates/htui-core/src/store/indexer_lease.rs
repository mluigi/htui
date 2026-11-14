//! The indexer lease store (MOD-159 M1, plan D3): one narrow trait beside [`WriteStore`], as
//! [`WorkerLogStore`] is, so the five `WriteStore` implementors do not each grow four methods for
//! one coordination table.
//!
//! Each Qdrant collection ([`IndexedCollection`]) has at most one `indexer_lease` row, and its
//! holder alone indexes that collection. Two implementors: `MemStore` (`mem.rs`) and `htui-store`'s
//! `PgStore`. Every method is declared `fn … -> impl Future<Output = Result<T>> + Send`, the form
//! of `store/worker_log.rs`, and an implementor writes `async fn`.
//!
//! Expiry is judged, and a new one stamped, by the store's own clock plus a TTL, as the run lease
//! is (MOD-40 plan D10); a TTL is refused by [`lease_ttl_micros`]'s rule before anything is read.
//! The owner token is written, compared and cleared but never read back: [`IndexerLease`] leaves it
//! out, as `Run` leaves out `lease_owner`.
//!
//! [`WriteStore`]: crate::store::WriteStore
//! [`WorkerLogStore`]: crate::store::WorkerLogStore
//! [`lease_ttl_micros`]: crate::store::lease_ttl_micros

use std::future::Future;

use chrono::TimeDelta;
use uuid::Uuid;

use crate::model::{BoxId, IndexedCollection, IndexerLease, LeaseTake};
use crate::store::error::Result;

/// `indexer_lease` rows (MOD-159 M1 plan D2, D3).
pub trait IndexerLeaseStore: Send + Sync {
    /// Takes `collection`'s lease for `owner` on `box_id` until the store's now plus `ttl` when it
    /// is free (no row, no owner), lapsed, or already `owner`'s; the first take of a collection
    /// creates its row. [`LeaseTake::Taken`] for a lease that was free, released, or lapsed under
    /// another owner; [`LeaseTake::Ours`] when `owner` holds it, or held it last and no one has
    /// taken it since, even if it lapsed (its expiry moves); and [`LeaseTake::Held`] naming the
    /// holder when another owner holds it live, in which case nothing is written.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a TTL out of range,
    /// before anything is read; whatever the backend reports.
    fn take_indexer_lease(
        &self,
        collection: &IndexedCollection,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> impl Future<Output = Result<LeaseTake>> + Send;

    /// Moves `collection`'s expiry to the store's now plus `ttl` when `owner` holds it, or held it
    /// last and no one has taken it since, even if it lapsed (so the lease is live again). `false`
    /// (zero rows: another owner took it, or it was released, or never taken) means abandon.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a TTL out of range,
    /// before anything is read; whatever the backend reports.
    fn renew_indexer_lease(
        &self,
        collection: &IndexedCollection,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Frees every lease `owner` holds (a clean exit): each loses its owner and expires now, and
    /// keeps its box for display. Returns the count freed; another owner's rows are untouched.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn release_indexer_leases(&self, owner: Uuid) -> impl Future<Output = Result<u64>> + Send;

    /// Frees `collection`'s lease when `owner` holds it, or held it last and no one has taken it
    /// since: it loses its owner, expires now, and keeps its box for display, as
    /// [`release_indexer_leases`](Self::release_indexer_leases) frees each. Answers whether a row
    /// changed; `owner`'s other leases and another owner's are untouched.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn release_indexer_lease(
        &self,
        collection: &IndexedCollection,
        owner: Uuid,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// `collection`'s lease as people see it, `None` when it was never taken.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn indexer_lease(
        &self,
        collection: &IndexedCollection,
    ) -> impl Future<Output = Result<Option<IndexerLease>>> + Send;
}
