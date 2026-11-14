//! The activity store (MOD-122 milestone 3, plan D1): one narrow trait of two reads beside
//! [`WriteStore`] and [`ReadStore`], the shape of milestone 2's
//! [`WorkerLogStore`](crate::store::WorkerLogStore).
//!
//! There is no append method: an event is written by the store inside the transition it records
//! (plan D2), by a deferred trigger on Postgres and by `MemStore::write`'s status diff, never by a
//! caller. Two implementors, `MemStore` (`mem.rs`) and `htui-store`'s `PgStore`, whose `Writer`
//! forwards per arm. Every method is declared `fn … -> impl Future<Output = Result<T>> + Send`
//! and an implementor writes `async fn`.
//!
//! [`WriteStore`]: crate::store::WriteStore
//! [`ReadStore`]: crate::store::ReadStore

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::model::ActivityPage;
use crate::store::error::Result;

/// D6: events kept per user; used by T2's tests, T3's TUI sweep and the shipper's sweep.
pub const PER_USER_CAP: u32 = 100_000;

/// `activity_event` reads and the retention sweep (MOD-122 M3 plan D1, D4, D6).
pub trait ActivityStore: Send + Sync {
    /// My events with `id > after`, oldest first, at most `limit`; `None` = the newest `limit`,
    /// still oldest first. `pinned` is every needs-you event of mine still waiting (D4), newest
    /// first, at most [`PINNED_LIMIT`](crate::model::PINNED_LIMIT) (200), whatever `after` says.
    ///
    /// "Still waiting" is read from the subject's current state, never stored: a `gate` while its
    /// step is `awaiting_approval`, a `permission` while its row is `pending`, an `unblock` while
    /// its item is `blocked`, and the three run kinds while the run is `awaiting_approval` with no
    /// step parked; and only the newest needs-you event per subject.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn activity(
        &self,
        after: Option<i64>,
        limit: u32,
    ) -> impl Future<Output = Result<ActivityPage>> + Send;

    /// Deletes events with `at < older_than`, then beyond `per_user_cap` per user (oldest first);
    /// returns the count. Idempotent: two sweepers at once are harmless.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn prune_activity(
        &self,
        older_than: DateTime<Utc>,
        per_user_cap: u32,
    ) -> impl Future<Output = Result<u64>> + Send;
}
