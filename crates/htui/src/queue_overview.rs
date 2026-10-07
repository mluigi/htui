//! The queue overlay's read (MOD-12 M3 D7): one `QueueOverview` composed from the runner's own
//! reads (`ready_items`, `batch_cancelled_items`, `batch_spend`, `batch_budget`, `admission_limit`),
//! so the overlay says what `admit` would do. Classification is `htui_core::model::classify_entry`.

use htui_core::model::QueueOverview;
use htui_core::store::{Result, StoreError};
use htui_store::Backend;

/// [`StoreRequest::QueueOverview`](crate::store_worker::StoreRequest::QueueOverview)'s answer.
///
/// # Errors
/// `NotFound` "this box" before registration; offline, `Unreachable(DATABASE_UNREACHABLE)` from
/// `queue_rows`, the first queue read; otherwise whatever a read reports.
pub async fn overview(backend: &Backend) -> Result<QueueOverview> {
    let info = backend
        .box_info()
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "box",
            id: "this box".to_owned(),
        })?;
    // Not composed yet (MOD-12 M3 T4): the next commit reads the queue.
    Ok(QueueOverview {
        box_id: info.box_id,
        batch: None,
        slots_used: 0,
        slots_limit: 0,
        rows: Vec::new(),
        demo: false,
    })
}
