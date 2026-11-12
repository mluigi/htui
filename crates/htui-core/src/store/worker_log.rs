//! The worker log store (MOD-122 milestone 2, plan D1): one narrow trait beside [`WriteStore`]
//! and [`ReadStore`], not a widening of them, so `Writer`, `Backend` and the test doubles that
//! implement those two do not each grow four methods for one log table.
//!
//! Two implementors: `MemStore` (`mem.rs`) and `htui-store`'s `PgStore`, whose `Writer` forwards
//! per arm. A headless worker writes through `PgStore` directly; the TUI reads through
//! `Backend::worker_logs`. Every method is declared `fn … -> impl Future<Output = Result<T>> +
//! Send`, the form of `store/worker.rs` (MOD-41 plan P-2), and an implementor writes `async fn`.
//!
//! [`WriteStore`]: crate::store::WriteStore
//! [`ReadStore`]: crate::store::ReadStore

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::model::{BoxId, LogShipLevel, NewWorkerLog, WorkerLogLine};
use crate::store::error::Result;

/// `worker_log` rows and the per-box shipping level (MOD-122 M2 plan D1, D4, D6, D7).
pub trait WorkerLogStore: Send + Sync {
    /// One batch from `box_id`, in order; returns the rows written. Each row is stamped with this
    /// user and the box's hostname (`box_name`), and its text goes through
    /// [`NewWorkerLog::stored_text`]. The batch is written whole or not at all.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) (`entity: "box"`) for an
    /// unknown box or another user's, when `lines` is not empty (an empty batch is `Ok(0)` and
    /// reads nothing); whatever the backend reports.
    fn append_worker_logs(
        &self,
        box_id: BoxId,
        lines: &[NewWorkerLog],
    ) -> impl Future<Output = Result<u64>> + Send;

    /// This user's rows with `id > after`, oldest first, at most `limit`; `after: None` is the
    /// newest `limit` rows, still oldest first.
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn worker_logs(
        &self,
        after: Option<i64>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<WorkerLogLine>>> + Send;

    /// Deletes the rows received before `older_than`, then, per box, the oldest rows beyond
    /// `per_box_cap`; returns how many went. Database-wide (`R-USR-2`: one `app_user` per
    /// database), and idempotent, so two workers sweeping together is harmless (plan D4). The cap
    /// counts only rows whose box still exists: a deleted box's error rows stay until the age
    /// cut (plan D6, blueprint F14).
    ///
    /// # Errors
    ///
    /// Whatever the backend reports.
    fn prune_worker_logs(
        &self,
        older_than: DateTime<Utc>,
        per_box_cap: u32,
    ) -> impl Future<Output = Result<u64>> + Send;

    /// Sets (`Some`) or clears (`None`) `box.settings.log_level` on one of this user's boxes;
    /// every other key of the blob survives, and `edit_version` does not move (last write wins,
    /// plan D7). `false` = no such box of this user's.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) with
    /// [`BOX_SETTINGS_NOT_AN_OBJECT`](crate::store::traits::BOX_SETTINGS_NOT_AN_OBJECT) when the
    /// box's settings blob is not a JSON object; whatever the backend reports.
    fn set_box_log_level(
        &self,
        box_id: BoxId,
        level: Option<LogShipLevel>,
    ) -> impl Future<Output = Result<bool>> + Send;
}
