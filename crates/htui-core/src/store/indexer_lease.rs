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
//! A pass request (MOD-159 M3 plan D1, D2) is the row's `pass_requested_at`, stamped by the store
//! clock: [`IndexerLeaseStore::request_indexer_pass`] stamps it only under a live holder, a take
//! and a release keep it, and only the holder's [`IndexerLeaseStore::claim_indexer_pass`] clears
//! it. The holder claims before its pass reads anything (plan D3), so a request stamped during
//! the pass stays for the next claim. [`take_or_request_pass`] is the requesters' one rule.
//!
//! [`WriteStore`]: crate::store::WriteStore
//! [`WorkerLogStore`]: crate::store::WorkerLogStore
//! [`lease_ttl_micros`]: crate::store::lease_ttl_micros

use std::future::Future;

use chrono::TimeDelta;
use uuid::Uuid;

use crate::model::{BoxId, IndexedCollection, IndexerLease, LeaseTake, LeaseTurn};
use crate::store::error::Result;

/// How often [`take_or_request_pass`] tries a take and then a request (MOD-159 M3 plan D4) before
/// it answers [`LeaseTurn::Held`] with the holder its last take saw.
pub const PASS_REQUEST_ATTEMPTS: usize = 3;

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

    /// Stamps a pass request (the store's now) on `collection`'s row when someone holds it live;
    /// answers that holder (as [`indexer_lease`](Self::indexer_lease) shows it, `live` true).
    /// `None` (no row, released, lapsed): nothing written. A request already pending is stamped
    /// again.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn request_indexer_pass(
        &self,
        collection: &IndexedCollection,
    ) -> impl Future<Output = Result<Option<IndexerLease>>> + Send;

    /// Clears `collection`'s pass request when `owner` holds the row (expiry not judged, as
    /// [`renew_indexer_lease`](Self::renew_indexer_lease)); answers whether one was pending
    /// (zero rows: none pending, or the row is not `owner`'s). A request stamped after this call
    /// stays for the next claim.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn claim_indexer_pass(
        &self,
        collection: &IndexedCollection,
        owner: Uuid,
    ) -> impl Future<Output = Result<bool>> + Send;
}

/// Takes `collection`'s lease for `owner` on `box_id`, or asks its live holder for a pass
/// (MOD-159 M3 plan D4), the rule both requesters (`--index-items` and `Ctrl+R`) share.
///
/// A take that answers [`LeaseTake::Taken`] or [`LeaseTake::Ours`] is [`LeaseTurn::Ours`], so a
/// free or lapsed lease is taken, never requested. A [`LeaseTake::Held`] is followed by
/// [`IndexerLeaseStore::request_indexer_pass`]: a holder answered is [`LeaseTurn::Requested`];
/// `None` (the holder let go in between) takes again, up to [`PASS_REQUEST_ATTEMPTS`] times, after
/// which the answer is [`LeaseTurn::Held`] with the holder the last take saw and nothing recorded.
///
/// # Errors
///
/// The first error of a take or a request.
pub async fn take_or_request_pass<L: IndexerLeaseStore>(
    leases: &L,
    collection: &IndexedCollection,
    box_id: BoxId,
    owner: Uuid,
    ttl: TimeDelta,
) -> Result<LeaseTurn> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let held = match leases
            .take_indexer_lease(collection, box_id, owner, ttl)
            .await?
        {
            LeaseTake::Taken | LeaseTake::Ours => return Ok(LeaseTurn::Ours),
            LeaseTake::Held(held) => held,
        };
        if let Some(holder) = leases.request_indexer_pass(collection).await? {
            return Ok(LeaseTurn::Requested(holder));
        }
        if attempt >= PASS_REQUEST_ATTEMPTS {
            return Ok(LeaseTurn::Held(held));
        }
    }
}
