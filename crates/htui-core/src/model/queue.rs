//! The auto-mode queue (MOD-12 PRD D1-D4; `0016_auto_queue.sql`): one item's membership of a
//! box's queue, the batch a queue activation is, and the pure rules the runner composes
//! (plan D4-D6). Neither table is mirrored, so every store read of them is inherent.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::box_::{BoxSettings, DEFAULT_MAX_CONCURRENT_ITEMS};
use crate::model::ids::{BatchId, BoxId, ItemId, ProjectId, UserId};
use crate::model::item::ItemSummary;

str_enum!(
    /// `queue_batch.closed_reason` (MOD-12 D2, D3).
    BatchClose {
        /// `P` closed it: no new admission, running runs untouched.
        Paused => "paused",
        /// No entry was left and no auto run of the batch was live.
        Drained => "drained",
    }
);

/// One `queue_entry` row, with its item's project (joined, not a column): what the runner needs
/// to build `ready_items`' scope (MOD-12 D5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// `queue_entry.item_id`.
    pub item_id: ItemId,
    /// `item.project_id` of that item.
    pub project_id: ProjectId,
    /// `queue_entry.box_id`.
    pub box_id: BoxId,
    /// `queue_entry.position`; `None` sorts last (D4). Milestone 1 writes only `None`.
    pub position: Option<i32>,
    /// `queue_entry.queued_at`.
    pub queued_at: DateTime<Utc>,
    /// `queue_entry.queued_by`.
    pub queued_by: UserId,
}

/// One `queue_batch` row (MOD-12 D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueBatch {
    /// `queue_batch.id`.
    pub id: BatchId,
    /// `queue_batch.box_id`.
    pub box_id: BoxId,
    /// `queue_batch.opened_at`.
    pub opened_at: DateTime<Utc>,
    /// `queue_batch.opened_by`.
    pub opened_by: UserId,
    /// `queue_batch.closed_at`; `None` while the queue runs.
    pub closed_at: Option<DateTime<Utc>>,
    /// `queue_batch.closed_reason`; `None` exactly when `closed_at` is.
    pub closed_reason: Option<BatchClose>,
}

/// MOD-12 D4, D5: the queued items `ready` holds, in queue order: an explicit `position` first,
/// ascending, then `ready`'s own order (`ready_items` is `priority DESC, created_at, id`). An entry
/// whose item `ready` does not hold is not ready and is left out.
#[must_use]
pub fn admission_order(entries: &[QueueEntry], ready: &[ItemSummary]) -> Vec<ItemId> {
    let rank: HashMap<ItemId, usize> = ready
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id, index))
        .collect();
    let mut queued: Vec<(Option<i32>, usize, ItemId)> = entries
        .iter()
        .filter_map(|entry| {
            rank.get(&entry.item_id)
                .map(|&index| (entry.position, index, entry.item_id))
        })
        .collect();
    queued.sort_by_key(|&(position, index, _)| (position.is_none(), position, index));
    queued.into_iter().map(|(_, _, item)| item).collect()
}

/// `R-ORCH-9`'s three rungs, as `claim_run` reads them (`pg/write.rs:4375-4389`,
/// `mem.rs:4349-4361`): the box's `settings.max_concurrent_items`, else `app_setting`'s, else
/// [`DEFAULT_MAX_CONCURRENT_ITEMS`]. A blob that does not decode falls through.
#[must_use]
pub fn admission_limit(box_settings: &Value, app: &BTreeMap<String, Value>) -> u32 {
    serde_json::from_value::<BoxSettings>(box_settings.clone())
        .ok()
        .and_then(|settings| settings.max_concurrent_items)
        .or_else(|| {
            app.get("max_concurrent_items")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
        })
        .unwrap_or(DEFAULT_MAX_CONCURRENT_ITEMS)
}

/// MOD-12 D6: the runs the runner may still create: the limit less every `running` run on the
/// box and every `queued` run targeted at it. Never below zero.
#[must_use]
pub fn free_slots(limit: u32, running: usize, queued: usize) -> usize {
    usize::try_from(limit)
        .unwrap_or(usize::MAX)
        .saturating_sub(running.saturating_add(queued))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::ItemKindId;
    use crate::model::item::Status;
    use serde_json::json;

    fn entry(item: ItemId, position: Option<i32>) -> QueueEntry {
        QueueEntry {
            item_id: item,
            project_id: ProjectId::default(),
            box_id: BoxId::default(),
            position,
            queued_at: DateTime::UNIX_EPOCH,
            queued_by: UserId::default(),
        }
    }

    fn ready(item: ItemId) -> ItemSummary {
        ItemSummary {
            id: item,
            project_id: ProjectId::default(),
            kind_id: ItemKindId::default(),
            key: "ANA-1".to_owned(),
            key_prefix: "ANA".to_owned(),
            key_number: 1,
            title: "t".to_owned(),
            status: Status::Open,
            priority: 0,
            required_tags: Vec::new(),
            updated_at: DateTime::UNIX_EPOCH,
            touched_paths: Vec::new(),
        }
    }

    #[test]
    fn batch_close_matches_check_list() {
        let texts: Vec<&str> = BatchClose::ALL.iter().map(|close| close.as_str()).collect();
        assert_eq!(
            texts,
            ["paused", "drained"],
            "0016's chk_queue_batch_closed_reason"
        );
    }

    #[test]
    fn admission_order_puts_positions_first_then_ready_order() {
        let (x, y, z) = (ItemId::new(), ItemId::new(), ItemId::new());
        let entries = [entry(x, None), entry(y, Some(2)), entry(z, Some(1))];
        let order = admission_order(&entries, &[ready(x), ready(y), ready(z)]);
        assert_eq!(
            order,
            [z, y, x],
            "positions ascending, then the unpositioned"
        );

        let (a, b) = (ItemId::new(), ItemId::new());
        let order = admission_order(&[entry(a, None), entry(b, None)], &[ready(b), ready(a)]);
        assert_eq!(order, [b, a], "without positions, ready_items' own order");
    }

    #[test]
    fn admission_order_drops_entries_that_are_not_ready() {
        let (queued_ready, queued_waiting, ready_unqueued) =
            (ItemId::new(), ItemId::new(), ItemId::new());
        let entries = [entry(queued_waiting, Some(1)), entry(queued_ready, None)];
        let order = admission_order(&entries, &[ready(ready_unqueued), ready(queued_ready)]);
        assert_eq!(order, [queued_ready]);
    }

    #[test]
    fn admission_limit_walks_box_then_app_then_default() {
        let app = BTreeMap::from([("max_concurrent_items".to_owned(), json!(5))]);
        assert_eq!(
            admission_limit(&json!({"max_concurrent_items": 3}), &app),
            3
        );
        assert_eq!(
            admission_limit(&json!({}), &app),
            5,
            "absent key: the app rung"
        );
        assert_eq!(
            admission_limit(&json!({"max_concurrent_items": "three"}), &app),
            5,
            "an undecodable blob falls through"
        );
        assert_eq!(admission_limit(&json!([1, 2]), &app), 5);
        assert_eq!(
            admission_limit(&json!({}), &BTreeMap::new()),
            DEFAULT_MAX_CONCURRENT_ITEMS
        );
        let bad_app = BTreeMap::from([("max_concurrent_items".to_owned(), json!("x"))]);
        assert_eq!(
            admission_limit(&json!({}), &bad_app),
            DEFAULT_MAX_CONCURRENT_ITEMS
        );
    }

    #[test]
    fn free_slots_counts_running_and_queued_and_never_underflows() {
        assert_eq!(free_slots(2, 1, 0), 1);
        assert_eq!(free_slots(2, 1, 1), 0);
        assert_eq!(free_slots(1, 3, 0), 0);
    }
}
