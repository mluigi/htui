//! The auto-mode queue (MOD-12 PRD D1-D4; `0016_auto_queue.sql`): one item's membership of a
//! box's queue, the batch a queue activation is, and the pure rules the runner composes
//! (plan D4-D6). Neither table is mirrored, so every store read of them is inherent.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::box_::{BoxSettings, DEFAULT_MAX_CONCURRENT_ITEMS};
use crate::model::ids::{BatchId, BoxId, ItemId, ProjectId, RunId, StepId, UserId};
use crate::model::item::{ItemSummary, Status};
use crate::model::quota::CapError;
use crate::model::run::{RunMode, RunStatus};

str_enum!(
    /// `queue_batch.closed_reason` (MOD-12 D2, D3).
    BatchClose {
        /// `P` closed it: no new admission, running runs untouched.
        Paused => "paused",
        /// Nothing was admissible and no auto run of the batch was live: the queue emptied
        /// (M1 D3) or stalled (M3 D4, L4).
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

/// `R-ORCH-9`'s three rungs, as `claim_run` reads them (`PgStore::claim_run` in
/// `htui-store`'s `pg/write.rs`, `State::max_concurrent_items` in `store/mem.rs`): the box's
/// `settings.max_concurrent_items`, else `app_setting`'s, else
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

/// `app_setting.min_budget_for_new_attempt` (OQ-6): unseeded; USD micros.
pub const MIN_BUDGET_FOR_NEW_ATTEMPT: &str = "min_budget_for_new_attempt";

/// OQ-6's reading of [`MIN_BUDGET_FOR_NEW_ATTEMPT`]: a positive integer, else `0`. A stray `0`, a
/// negative, a string or a float all read as `0`, as `htui-orch`'s `min_budget` reads it, so the
/// runner (MOD-12 M2 D4) and the walk agree on one number.
#[must_use]
pub fn min_budget_micros(app: &BTreeMap<String, Value>) -> i64 {
    app.get(MIN_BUDGET_FOR_NEW_ATTEMPT)
        .and_then(Value::as_i64)
        .filter(|micros| *micros > 0)
        .unwrap_or(0)
}

/// Why [`batch_budget`] admits no new attempt in a batch (MOD-12 M2 D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchStop {
    /// The batch has spent its project's `per_token_cap_batch` or more (rule 2's batch twin).
    CapReached {
        /// The batch's spend, USD micros.
        spent: i64,
        /// The cap compared against, USD micros.
        cap: i64,
    },
    /// What is left is below `min_budget_for_new_attempt` (rule 5's batch twin).
    Budget {
        /// `cap - spent`, USD micros.
        remaining: i64,
        /// The minimum a new attempt needs, USD micros.
        min: i64,
    },
}

impl core::fmt::Display for BatchStop {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CapReached { spent, cap } => {
                write!(f, "batch cap reached ({spent} of {cap} micros)")
            }
            Self::Budget { remaining, min } => {
                write!(f, "batch budget: {remaining} micros left, {min} required")
            }
        }
    }
}

/// MOD-12 M2 D3: the one admission rule the runner, the walk and (through its figures) the
/// recorder share, mirroring `select::walk`'s rule 2 and rule 5 exactly. Either figure unknown is
/// unbounded (`Ok(None)`, OQ-6). `spent >= cap` is [`BatchStop::CapReached`] (equality reaches it);
/// `cap - spent < min` is [`BatchStop::Budget`] (exactly the minimum is enough); otherwise
/// `Ok(Some(cap - spent))`.
///
/// # Errors
/// The [`BatchStop`] that refuses the attempt.
pub fn batch_budget(
    spent: Option<i64>,
    cap: Option<i64>,
    min: i64,
) -> Result<Option<i64>, BatchStop> {
    let (Some(spent), Some(cap)) = (spent, cap) else {
        return Ok(None);
    };
    if spent >= cap {
        return Err(BatchStop::CapReached { spent, cap });
    }
    let remaining = cap.saturating_sub(spent);
    if remaining < min {
        return Err(BatchStop::Budget { remaining, min });
    }
    Ok(Some(remaining))
}

/// MOD-12 M3 D3: which way [`moved_order`] (and the stores' `move_queue_entry`) moves an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMove {
    /// One place towards the head of the queue.
    Up,
    /// One place towards its tail.
    Down,
}

/// MOD-12 M3 D3: `order` with `item` swapped with its neighbour `to`; `None` when `item` is not
/// in `order`, or is already at that end. Both stores call it, so Mem and Pg move alike.
#[must_use]
pub fn moved_order(order: &[ItemId], item: ItemId, to: QueueMove) -> Option<Vec<ItemId>> {
    let at = order.iter().position(|id| *id == item)?;
    let other = match to {
        QueueMove::Up => at.checked_sub(1)?,
        QueueMove::Down => Some(at + 1).filter(|next| *next < order.len())?,
    };
    let mut moved = order.to_vec();
    moved.swap(at, other);
    Some(moved)
}

/// MOD-12 M3 D5, D6: the item's latest graph run, as [`QueueRow`] carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRunFact {
    /// `run.id`.
    pub id: RunId,
    /// `run.status`.
    pub status: RunStatus,
    /// `run.mode`.
    pub mode: RunMode,
    /// `run.target_box_id`.
    pub target_box_id: BoxId,
    /// That box's `hostname`; `None` when no box row answers (Mem only; Pg's FK guarantees one).
    pub target_hostname: Option<String>,
    /// `run.failure`.
    pub failure: Option<String>,
    /// The run's first step `awaiting_approval` by `(position, attempt, fanout_index)`: a parked
    /// gate. `None` for a judge park, which parks no step (`engine.rs` `park_selection`).
    pub parked_step: Option<StepId>,
}

/// MOD-12 M3 D5, D6: one queue entry with what the overlay classifies it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRow {
    /// The `queue_entry` row (with its item's project).
    pub entry: QueueEntry,
    /// `item.key`, e.g. `FIX-3`.
    pub key: String,
    /// `item.title`.
    pub title: String,
    /// `item.status`.
    pub status: Status,
    /// `item.priority` (D2's tie-break; never written by a move).
    pub priority: i16,
    /// `item.created_at` (D2's second tie-break).
    pub created_at: DateTime<Utc>,
    /// The item's latest `kind = 'graph'` run by `(queued_at, id)`.
    pub latest_run: Option<QueueRunFact>,
    /// The body of the item's latest note by `(created_at, id)`, verbatim.
    pub latest_note: Option<String>,
    /// The keys of its live `blocked_by` targets that are not `done`/`closed`, in byte order.
    pub open_blockers: Vec<String>,
}

/// MOD-12 M3 D5: why a ready entry is not admitted this batch: the runner's own three refusals
/// in `admit` (M2 D4, review R1 L3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// [`batch_budget`] refused it.
    Budget(BatchStop),
    /// Its project's caps do not parse ([`ProjectCaps::from_settings`](crate::model::ProjectCaps)).
    BadCap(CapError),
    /// Its project read as absent (a delete racing the read).
    ProjectGone,
}

/// MOD-12 M3 D5: why an entry waits without needing a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    /// Open `blocked_by` targets, by key.
    BlockedBy(Vec<String>),
    /// A run of it was cancelled under the open batch (review H1).
    CancelledInBatch,
    /// No batch is open.
    Paused,
    /// Not ready for a reason the list above does not name: the item's status.
    NotReady(Status),
}

/// MOD-12 M3 D5 (PRD hypothesis): why an entry needs a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Escalation {
    /// A hard gate parked a step of the run.
    HardGateParked {
        /// The parked run.
        run: RunId,
        /// The parked step.
        step: StepId,
    },
    /// A fan-out awaits a human selection: the run parked with no step parked.
    JudgeUndecided {
        /// The parked run.
        run: RunId,
        /// The item's latest note (the fan-out's selection note).
        note: Option<String>,
    },
    /// The review loop gave up: item `blocked`, run `awaiting_approval`.
    ReviewLoopExhausted {
        /// The parked run.
        run: RunId,
        /// The item's latest note.
        note: Option<String>,
    },
    /// The item is `blocked` with no parked run (a walk refusal, or by hand).
    Blocked {
        /// Its latest run, if any (for `Enter`).
        run: Option<RunId>,
        /// The item's latest note.
        note: Option<String>,
    },
    /// The item is `failed`.
    Failed {
        /// Its latest run, if any (for `Enter`).
        run: Option<RunId>,
        /// That run's `failure`.
        failure: Option<String>,
    },
    /// This box lacks tags the item requires (`R-ORCH-10`).
    MissingTags(Vec<String>),
}

/// MOD-12 M3 D5: what one entry is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryState {
    /// A `queued` or `running` run targeted at this box.
    Running {
        /// The run.
        run: RunId,
        /// `Queued` (admitted, not claimed yet) or `Running`.
        status: RunStatus,
    },
    /// A `queued` or `running` run targeted at another box (MOD-43 owns choosing; M3 reports).
    Elsewhere {
        /// The run.
        run: RunId,
        /// The target box's hostname.
        hostname: Option<String>,
        /// `Queued` or `Running`.
        status: RunStatus,
    },
    /// Ready and admissible: the runner admits it at the next free slot.
    ///
    /// M3 review R1 L3, a known limit: the overlay reads only stored state, so an entry whose
    /// admission the runner's enqueue refused without changing any (the refusal never closes the
    /// batch, by design) still reads `Next`, and keeps the batch open, until the refusal's cause
    /// changes. The runner keeps no record of that refusal for the overview to read; surfacing it
    /// needs runner shared state, left to a follow-up (noted in ANA-2 §4.10 at close-out).
    Next,
    /// Ready, but this batch will not admit it.
    Held(Hold),
    /// Not ready; nothing for a person to do.
    Waiting(Wait),
    /// A person is needed.
    Escalated(Escalation),
}

impl EntryState {
    /// Whether the overlay draws the row in the warning style.
    #[must_use]
    pub const fn is_escalated(&self) -> bool {
        matches!(self, Self::Escalated(_))
    }

    /// The run and step `Enter` reveals (M3 D8): `(None, None)` reveals the item.
    #[must_use]
    pub fn reveal(&self) -> (Option<RunId>, Option<StepId>) {
        match self {
            Self::Running { run, .. }
            | Self::Elsewhere { run, .. }
            | Self::Escalated(
                Escalation::JudgeUndecided { run, .. }
                | Escalation::ReviewLoopExhausted { run, .. },
            ) => (Some(*run), None),
            Self::Escalated(Escalation::HardGateParked { run, step }) => (Some(*run), Some(*step)),
            Self::Escalated(Escalation::Blocked { run, .. } | Escalation::Failed { run, .. }) => {
                (*run, None)
            }
            Self::Next
            | Self::Held(_)
            | Self::Waiting(_)
            | Self::Escalated(Escalation::MissingTags(_)) => (None, None),
        }
    }
}

/// MOD-12 M3 D5: the per-box, per-batch facts [`classify_entry`] reads, composed live by the
/// serve module from the runner's own reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveFacts {
    /// No batch is open.
    pub paused: bool,
    /// `ready_items` over the rows' projects on this box.
    pub ready: HashSet<ItemId>,
    /// `batch_cancelled_items` of the open batch; empty when paused.
    pub cancelled: HashSet<ItemId>,
    /// `missing_tags` of each `open` row that is not ready; empty lists are left out.
    pub missing_tags: HashMap<ItemId, Vec<String>>,
    /// The batch's [`Hold`] per project; empty when paused.
    pub holds: HashMap<ProjectId, Hold>,
}

/// MOD-12 M3 D5: the open batch as the overlay's header shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchFigures {
    /// `queue_batch.id`.
    pub id: BatchId,
    /// `queue_batch.opened_at`.
    pub opened_at: DateTime<Utc>,
    /// `batch_spend`, USD micros; `None` when no step reported a cost.
    pub spent: Option<i64>,
}

/// MOD-12 M3 D5, D7: one box's queue for the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueOverview {
    /// This box.
    pub box_id: BoxId,
    /// The open batch; `None` is paused.
    pub batch: Option<BatchFigures>,
    /// M3 review R1 L4: how this box's last batch closed (`last_closed_batch`), read only while
    /// none is open, so the header says why the queue is paused; `None` while a batch is open or
    /// before any closed. A stall and an emptied queue both close [`BatchClose::Drained`].
    pub last_close: Option<BatchClose>,
    /// `running` runs on the box plus `queued` runs targeted at it ([`free_slots`]' inputs).
    pub slots_used: usize,
    /// [`admission_limit`].
    pub slots_limit: u32,
    /// The rows in D2 order, each classified.
    pub rows: Vec<(QueueRow, EntryState)>,
    /// The memory backend (`htui --demo`), whose runtime never admits (M1 review L3).
    pub demo: bool,
}

/// MOD-12 M3 D5: what `row` is doing on box `here`, given `live`. First match wins:
///
/// 1. a `queued`/`running` latest run targeted here is [`EntryState::Running`], 2. one targeted
///    elsewhere is [`EntryState::Elsewhere`]; a live run beats everything below;
/// 3. a `failed` item is [`Escalation::Failed`];
/// 4. a `blocked` item whose latest run is parked is [`Escalation::ReviewLoopExhausted`],
/// 5. any other `blocked` item is [`Escalation::Blocked`];
/// 6. an `awaiting_approval` item whose parked run parked a step is
///    [`Escalation::HardGateParked`], 7. one that parked no step is [`Escalation::JudgeUndecided`];
/// 8. tags this box lacks are [`Escalation::MissingTags`];
/// 9. open blockers are [`Wait::BlockedBy`];
/// 10. a run cancelled in the open batch is [`Wait::CancelledInBatch`];
/// 11. an item `ready_items` does not hold is [`Wait::NotReady`];
/// 12. a closed batch is [`Wait::Paused`];
/// 13. a [`Hold`] on the item's project is [`EntryState::Held`];
/// 14. otherwise [`EntryState::Next`].
///
/// So [`EntryState::Held`] is reached only by a ready, uncancelled entry of an open batch: the
/// complement of the runner's `admissible`.
#[must_use]
pub fn classify_entry(row: &QueueRow, here: BoxId, live: &LiveFacts) -> EntryState {
    let item = row.entry.item_id;
    let latest = row.latest_run.as_ref();
    if let Some(run) =
        latest.filter(|run| matches!(run.status, RunStatus::Queued | RunStatus::Running))
    {
        return if run.target_box_id == here {
            EntryState::Running {
                run: run.id,
                status: run.status,
            }
        } else {
            EntryState::Elsewhere {
                run: run.id,
                hostname: run.target_hostname.clone(),
                status: run.status,
            }
        };
    }
    let parked = latest.filter(|run| run.status == RunStatus::AwaitingApproval);
    let note = || row.latest_note.clone();
    let escalation = match (row.status, parked) {
        (Status::Failed, _) => Some(Escalation::Failed {
            run: latest.map(|run| run.id),
            failure: latest.and_then(|run| run.failure.clone()),
        }),
        (Status::Blocked, Some(run)) => Some(Escalation::ReviewLoopExhausted {
            run: run.id,
            note: note(),
        }),
        (Status::Blocked, None) => Some(Escalation::Blocked {
            run: latest.map(|run| run.id),
            note: note(),
        }),
        (Status::AwaitingApproval, Some(run)) => Some(match run.parked_step {
            Some(step) => Escalation::HardGateParked { run: run.id, step },
            None => Escalation::JudgeUndecided {
                run: run.id,
                note: note(),
            },
        }),
        _ => live
            .missing_tags
            .get(&item)
            .filter(|tags| !tags.is_empty())
            .map(|tags| Escalation::MissingTags(tags.clone())),
    };
    if let Some(escalation) = escalation {
        return EntryState::Escalated(escalation);
    }
    if !row.open_blockers.is_empty() {
        return EntryState::Waiting(Wait::BlockedBy(row.open_blockers.clone()));
    }
    if live.cancelled.contains(&item) {
        return EntryState::Waiting(Wait::CancelledInBatch);
    }
    if !live.ready.contains(&item) {
        return EntryState::Waiting(Wait::NotReady(row.status));
    }
    if live.paused {
        return EntryState::Waiting(Wait::Paused);
    }
    live.holds
        .get(&row.entry.project_id)
        .map_or(EntryState::Next, |hold| EntryState::Held(hold.clone()))
}

/// The first line of a note or failure: the overlay draws one row per entry, and the Runs pane
/// holds the whole text.
fn note_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// `head`, then ` (last note: …)` when there is a note.
fn with_note(
    f: &mut core::fmt::Formatter<'_>,
    head: &str,
    note: Option<&str>,
) -> core::fmt::Result {
    match note {
        Some(note) => write!(f, "{head} (last note: {})", note_line(note)),
        None => f.write_str(head),
    }
}

impl core::fmt::Display for Hold {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Budget(stop) => write!(f, "{stop}"),
            Self::BadCap(error) => write!(f, "{error}"),
            Self::ProjectGone => f.write_str("its project is gone"),
        }
    }
}

impl core::fmt::Display for Wait {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BlockedBy(keys) => write!(f, "waiting on {}", keys.join(", ")),
            Self::CancelledInBatch => {
                f.write_str("cancelled in this batch; the next batch runs it")
            }
            Self::Paused => f.write_str("queue paused"),
            Self::NotReady(status) => write!(f, "not ready: item is {}", status.as_str()),
        }
    }
}

impl core::fmt::Display for Escalation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::HardGateParked { .. } => f.write_str("hard gate parked"),
            Self::JudgeUndecided { note, .. } => with_note(f, "judge undecided", note.as_deref()),
            Self::ReviewLoopExhausted { note, .. } => {
                with_note(f, "review loop exhausted", note.as_deref())
            }
            Self::Blocked { note, .. } => with_note(f, "blocked", note.as_deref()),
            Self::Failed {
                failure: Some(failure),
                ..
            } => write!(f, "failed: {}", note_line(failure)),
            Self::Failed { failure: None, .. } => f.write_str("failed"),
            Self::MissingTags(tags) => write!(f, "missing tags: {}", tags.join(", ")),
        }
    }
}

impl core::fmt::Display for EntryState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Running {
                status: RunStatus::Queued,
                ..
            } => f.write_str("admitted, waiting to be claimed"),
            Self::Running { .. } => f.write_str("running here"),
            Self::Elsewhere {
                hostname, status, ..
            } => {
                let host = hostname.as_deref().unwrap_or("another box");
                if *status == RunStatus::Queued {
                    write!(f, "queued for {host}")
                } else {
                    write!(f, "running on {host}")
                }
            }
            // M3 review R1 L3: also what an entry reads whose enqueue was refused with no
            // state change (see `Next`): the overview cannot tell the two apart.
            Self::Next => f.write_str("next to run"),
            Self::Held(hold) => write!(f, "held: {hold}"),
            Self::Waiting(wait) => write!(f, "{wait}"),
            Self::Escalated(escalation) => write!(f, "{escalation}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::ItemKindId;
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

    #[test]
    fn batch_budget_is_unbounded_when_either_figure_is_unknown() {
        assert_eq!(batch_budget(None, Some(1_000), 200), Ok(None));
        assert_eq!(batch_budget(Some(900), None, 200), Ok(None));
        assert_eq!(batch_budget(None, None, 200), Ok(None));
    }

    #[test]
    fn batch_budget_is_reached_at_equality() {
        assert_eq!(
            batch_budget(Some(500), Some(500), 0),
            Err(BatchStop::CapReached {
                spent: 500,
                cap: 500
            })
        );
        assert_eq!(
            batch_budget(Some(600), Some(500), 0),
            Err(BatchStop::CapReached {
                spent: 600,
                cap: 500
            })
        );
    }

    #[test]
    fn batch_budget_refuses_a_remainder_below_the_minimum() {
        assert_eq!(
            batch_budget(Some(900), Some(1_000), 200),
            Err(BatchStop::Budget {
                remaining: 100,
                min: 200
            })
        );
    }

    #[test]
    fn batch_budget_admits_exactly_the_minimum() {
        assert_eq!(batch_budget(Some(800), Some(1_000), 200), Ok(Some(200)));
        assert_eq!(
            batch_budget(Some(0), Some(0), 0),
            Err(BatchStop::CapReached { spent: 0, cap: 0 }),
            "0 is a real cap"
        );
    }

    #[test]
    fn batch_stop_names_the_batch() {
        assert_eq!(
            BatchStop::CapReached {
                spent: 500,
                cap: 500
            }
            .to_string(),
            "batch cap reached (500 of 500 micros)"
        );
        assert_eq!(
            BatchStop::Budget {
                remaining: 100,
                min: 200
            }
            .to_string(),
            "batch budget: 100 micros left, 200 required"
        );
    }

    #[test]
    fn min_budget_micros_reads_a_positive_integer_else_zero() {
        let app = |value: Value| BTreeMap::from([(MIN_BUDGET_FOR_NEW_ATTEMPT.to_owned(), value)]);
        assert_eq!(min_budget_micros(&app(json!(500))), 500);
        assert_eq!(min_budget_micros(&BTreeMap::new()), 0, "absent");
        for stray in [json!(0), json!(-5), json!("500"), json!(1.5)] {
            assert_eq!(min_budget_micros(&app(stray.clone())), 0, "{stray}");
        }
    }

    // MOD-12 M3 T1: moved_order, classify_entry and the overlay's sentences.

    const HERE_HOST: &str = "here";

    fn here() -> BoxId {
        BoxId::from_uuid(uuid::Uuid::from_u128(1))
    }

    fn there() -> BoxId {
        BoxId::from_uuid(uuid::Uuid::from_u128(2))
    }

    fn row(status: Status) -> QueueRow {
        QueueRow {
            entry: entry(ItemId::new(), None),
            key: "FIX-1".to_owned(),
            title: "t".to_owned(),
            status,
            priority: 0,
            created_at: DateTime::UNIX_EPOCH,
            latest_run: None,
            latest_note: None,
            open_blockers: Vec::new(),
        }
    }

    fn run(status: RunStatus, target: BoxId) -> QueueRunFact {
        QueueRunFact {
            id: RunId::new(),
            status,
            mode: RunMode::Auto,
            target_box_id: target,
            target_hostname: Some(if target == here() { HERE_HOST } else { "b" }.to_owned()),
            failure: None,
            parked_step: None,
        }
    }

    /// The row's item ready, nothing else.
    fn facts(row: &QueueRow) -> LiveFacts {
        LiveFacts {
            ready: HashSet::from([row.entry.item_id]),
            ..LiveFacts::default()
        }
    }

    fn budget_hold() -> Hold {
        Hold::Budget(BatchStop::CapReached {
            spent: 600,
            cap: 500,
        })
    }

    fn cap_error() -> CapError {
        CapError {
            key: crate::model::quota::PER_TOKEN_CAP_BATCH,
            found: "\"x\"".to_owned(),
        }
    }

    #[test]
    fn moved_order_swaps_with_the_neighbour() {
        let (a, b, c) = (ItemId::new(), ItemId::new(), ItemId::new());
        assert_eq!(
            moved_order(&[a, b, c], b, QueueMove::Up),
            Some(vec![b, a, c])
        );
        assert_eq!(
            moved_order(&[a, b, c], b, QueueMove::Down),
            Some(vec![a, c, b])
        );
    }

    #[test]
    fn moved_order_is_none_at_an_end_or_off_the_list() {
        let (a, b, c) = (ItemId::new(), ItemId::new(), ItemId::new());
        assert_eq!(moved_order(&[a, b, c], a, QueueMove::Up), None, "head up");
        assert_eq!(
            moved_order(&[a, b, c], c, QueueMove::Down),
            None,
            "tail down"
        );
        assert_eq!(moved_order(&[a, b, c], ItemId::new(), QueueMove::Up), None);
        assert_eq!(
            moved_order(&[a, b, c], ItemId::new(), QueueMove::Down),
            None
        );
        assert_eq!(moved_order(&[], a, QueueMove::Down), None, "an empty queue");
    }

    #[test]
    fn a_live_run_here_reads_running_and_a_queued_one_admitted() {
        let mut item = row(Status::InProgress);
        let running = run(RunStatus::Running, here());
        item.latest_run = Some(running.clone());
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Running {
                run: running.id,
                status: RunStatus::Running
            }
        );
        assert_eq!(state.to_string(), "running here");

        let queued = run(RunStatus::Queued, here());
        item.status = Status::Queued;
        item.latest_run = Some(queued.clone());
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Running {
                run: queued.id,
                status: RunStatus::Queued
            }
        );
        assert_eq!(state.to_string(), "admitted, waiting to be claimed");
    }

    #[test]
    fn a_live_run_elsewhere_beats_everything() {
        let mut item = row(Status::Failed);
        item.open_blockers = vec!["FEAT-1".to_owned()];
        let mut live = facts(&item);
        live.missing_tags
            .insert(item.entry.item_id, vec!["gpu".to_owned()]);
        live.holds.insert(item.entry.project_id, budget_hold());
        live.cancelled.insert(item.entry.item_id);

        let running = run(RunStatus::Running, there());
        item.latest_run = Some(running.clone());
        let state = classify_entry(&item, here(), &live);
        assert_eq!(
            state,
            EntryState::Elsewhere {
                run: running.id,
                hostname: Some("b".to_owned()),
                status: RunStatus::Running
            }
        );
        assert_eq!(state.to_string(), "running on b");

        let queued = run(RunStatus::Queued, there());
        item.latest_run = Some(queued.clone());
        let state = classify_entry(&item, here(), &live);
        assert_eq!(
            state,
            EntryState::Elsewhere {
                run: queued.id,
                hostname: Some("b".to_owned()),
                status: RunStatus::Queued
            }
        );
        assert_eq!(state.to_string(), "queued for b");

        let mut nameless = run(RunStatus::Running, there());
        nameless.target_hostname = None;
        item.latest_run = Some(nameless);
        assert_eq!(
            classify_entry(&item, here(), &live).to_string(),
            "running on another box"
        );
    }

    #[test]
    fn a_failed_item_reads_failed_even_with_an_open_blocker() {
        let mut item = row(Status::Failed);
        item.open_blockers = vec!["FEAT-1".to_owned()];
        let mut failed = run(RunStatus::Failed, here());
        failed.failure = Some("boom\nat line 3".to_owned());
        item.latest_run = Some(failed.clone());
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::Failed {
                run: Some(failed.id),
                failure: Some("boom\nat line 3".to_owned())
            })
        );
        assert!(state.is_escalated());
        assert_eq!(state.to_string(), "failed: boom");

        item.latest_run = None;
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::Failed {
                run: None,
                failure: None
            })
        );
        assert_eq!(state.to_string(), "failed");
    }

    #[test]
    fn a_blocked_item_with_a_parked_run_reads_review_loop_exhausted() {
        let mut item = row(Status::Blocked);
        let parked = run(RunStatus::AwaitingApproval, here());
        item.latest_run = Some(parked.clone());
        item.latest_note = Some("review loop exhausted after 3 attempts".to_owned());
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::ReviewLoopExhausted {
                run: parked.id,
                note: Some("review loop exhausted after 3 attempts".to_owned())
            })
        );
        assert_eq!(
            state.to_string(),
            "review loop exhausted (last note: review loop exhausted after 3 attempts)"
        );

        item.latest_note = None;
        assert_eq!(
            classify_entry(&item, here(), &facts(&item)).to_string(),
            "review loop exhausted"
        );
    }

    #[test]
    fn a_blocked_item_without_a_parked_run_reads_blocked() {
        let mut item = row(Status::Blocked);
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::Blocked {
                run: None,
                note: None
            })
        );
        assert_eq!(state.to_string(), "blocked");

        let done = run(RunStatus::Done, here());
        item.latest_run = Some(done.clone());
        item.latest_note = Some("no_candidate_agent: phase `research`".to_owned());
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::Blocked {
                run: Some(done.id),
                note: Some("no_candidate_agent: phase `research`".to_owned())
            })
        );
        assert_eq!(
            state.to_string(),
            "blocked (last note: no_candidate_agent: phase `research`)"
        );
    }

    #[test]
    fn awaiting_approval_with_a_parked_step_reads_hard_gate_parked() {
        let mut item = row(Status::AwaitingApproval);
        let mut parked = run(RunStatus::AwaitingApproval, here());
        let step = StepId::new();
        parked.parked_step = Some(step);
        item.latest_run = Some(parked.clone());
        item.latest_note = Some("ignored".to_owned());
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::HardGateParked {
                run: parked.id,
                step
            })
        );
        assert_eq!(state.to_string(), "hard gate parked");
    }

    #[test]
    fn awaiting_approval_without_a_parked_step_reads_judge_undecided() {
        let mut item = row(Status::AwaitingApproval);
        let parked = run(RunStatus::AwaitingApproval, here());
        item.latest_run = Some(parked.clone());
        let note = "fan-out `p` attempt 2 awaits selection: 3 candidates";
        item.latest_note = Some(note.to_owned());
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::JudgeUndecided {
                run: parked.id,
                note: Some(note.to_owned())
            })
        );
        assert_eq!(
            state.to_string(),
            format!("judge undecided (last note: {note})")
        );

        item.latest_note = None;
        assert_eq!(
            classify_entry(&item, here(), &facts(&item)).to_string(),
            "judge undecided"
        );
    }

    #[test]
    fn awaiting_approval_without_a_parked_run_is_not_ready() {
        let mut item = row(Status::AwaitingApproval);
        item.latest_run = Some(run(RunStatus::Done, here()));
        let state = classify_entry(&item, here(), &LiveFacts::default());
        assert_eq!(
            state,
            EntryState::Waiting(Wait::NotReady(Status::AwaitingApproval))
        );
        assert!(!state.is_escalated());
        assert_eq!(state.to_string(), "not ready: item is awaiting_approval");
    }

    #[test]
    fn the_status_escalations_beat_missing_tags() {
        let tagged = |item: &QueueRow| {
            let mut live = facts(item);
            live.missing_tags
                .insert(item.entry.item_id, vec!["gpu".to_owned()]);
            live
        };

        let mut failed = row(Status::Failed);
        failed.latest_run = Some(run(RunStatus::Failed, here()));
        assert!(matches!(
            classify_entry(&failed, here(), &tagged(&failed)),
            EntryState::Escalated(Escalation::Failed { .. })
        ));

        let mut exhausted = row(Status::Blocked);
        exhausted.latest_run = Some(run(RunStatus::AwaitingApproval, here()));
        assert!(matches!(
            classify_entry(&exhausted, here(), &tagged(&exhausted)),
            EntryState::Escalated(Escalation::ReviewLoopExhausted { .. })
        ));

        let blocked = row(Status::Blocked);
        assert!(matches!(
            classify_entry(&blocked, here(), &tagged(&blocked)),
            EntryState::Escalated(Escalation::Blocked { .. })
        ));

        let mut gate = row(Status::AwaitingApproval);
        let mut parked = run(RunStatus::AwaitingApproval, here());
        parked.parked_step = Some(StepId::new());
        gate.latest_run = Some(parked);
        assert!(matches!(
            classify_entry(&gate, here(), &tagged(&gate)),
            EntryState::Escalated(Escalation::HardGateParked { .. })
        ));

        let mut judge = row(Status::AwaitingApproval);
        judge.latest_run = Some(run(RunStatus::AwaitingApproval, here()));
        assert!(matches!(
            classify_entry(&judge, here(), &tagged(&judge)),
            EntryState::Escalated(Escalation::JudgeUndecided { .. })
        ));
    }

    #[test]
    fn missing_tags_beat_an_open_blocker() {
        let mut item = row(Status::Open);
        item.open_blockers = vec!["FEAT-1".to_owned()];
        let mut live = LiveFacts::default();
        live.missing_tags.insert(
            item.entry.item_id,
            vec!["cuda".to_owned(), "gpu".to_owned()],
        );
        let state = classify_entry(&item, here(), &live);
        assert_eq!(
            state,
            EntryState::Escalated(Escalation::MissingTags(vec![
                "cuda".to_owned(),
                "gpu".to_owned()
            ]))
        );
        assert_eq!(state.to_string(), "missing tags: cuda, gpu");

        live.missing_tags.insert(item.entry.item_id, Vec::new());
        assert_eq!(
            classify_entry(&item, here(), &live),
            EntryState::Waiting(Wait::BlockedBy(vec!["FEAT-1".to_owned()])),
            "an empty tag list is no escalation"
        );
    }

    #[test]
    fn an_open_blocker_reads_waiting_on_its_keys() {
        let mut item = row(Status::Open);
        item.open_blockers = vec!["FEAT-1".to_owned(), "FEAT-2".to_owned()];
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(
            state,
            EntryState::Waiting(Wait::BlockedBy(vec![
                "FEAT-1".to_owned(),
                "FEAT-2".to_owned()
            ]))
        );
        assert_eq!(state.to_string(), "waiting on FEAT-1, FEAT-2");
    }

    #[test]
    fn cancelled_in_batch_reads_waiting() {
        let item = row(Status::Open);
        let mut live = facts(&item);
        live.cancelled.insert(item.entry.item_id);
        live.holds.insert(item.entry.project_id, budget_hold());
        let state = classify_entry(&item, here(), &live);
        assert_eq!(state, EntryState::Waiting(Wait::CancelledInBatch));
        assert_eq!(
            state.to_string(),
            "cancelled in this batch; the next batch runs it"
        );
    }

    #[test]
    fn a_paused_queue_reads_paused_for_ready_entries_only() {
        let item = row(Status::Open);
        let mut live = facts(&item);
        live.paused = true;
        let state = classify_entry(&item, here(), &live);
        assert_eq!(state, EntryState::Waiting(Wait::Paused));
        assert_eq!(state.to_string(), "queue paused");

        live.ready.clear();
        assert_eq!(
            classify_entry(&item, here(), &live),
            EntryState::Waiting(Wait::NotReady(Status::Open))
        );
    }

    #[test]
    fn held_only_for_ready_entries_of_an_open_batch() {
        let item = row(Status::Open);
        let mut live = facts(&item);
        live.holds.insert(item.entry.project_id, budget_hold());
        let state = classify_entry(&item, here(), &live);
        assert_eq!(state, EntryState::Held(budget_hold()));
        assert!(!state.is_escalated());
        assert_eq!(
            state.to_string(),
            "held: batch cap reached (600 of 500 micros)"
        );

        let budget = Hold::Budget(BatchStop::Budget {
            remaining: 100,
            min: 200,
        });
        assert_eq!(
            budget.to_string(),
            "batch budget: 100 micros left, 200 required"
        );

        let mut not_ready = live.clone();
        not_ready.ready.clear();
        assert_eq!(
            classify_entry(&item, here(), &not_ready),
            EntryState::Waiting(Wait::NotReady(Status::Open))
        );

        live.holds
            .insert(item.entry.project_id, Hold::BadCap(cap_error()));
        let state = classify_entry(&item, here(), &live);
        assert_eq!(state, EntryState::Held(Hold::BadCap(cap_error())));
        assert_eq!(state.to_string(), format!("held: {}", cap_error()));

        live.holds.insert(item.entry.project_id, Hold::ProjectGone);
        let state = classify_entry(&item, here(), &live);
        assert_eq!(state, EntryState::Held(Hold::ProjectGone));
        assert_eq!(state.to_string(), "held: its project is gone");

        let mut other = live.clone();
        other.holds.clear();
        other.holds.insert(ProjectId::new(), Hold::ProjectGone);
        assert_eq!(
            classify_entry(&item, here(), &other),
            EntryState::Next,
            "another project's hold is not this entry's"
        );
    }

    #[test]
    fn a_ready_admissible_entry_reads_next() {
        let item = row(Status::Open);
        let state = classify_entry(&item, here(), &facts(&item));
        assert_eq!(state, EntryState::Next);
        assert!(!state.is_escalated());
        assert_eq!(state.to_string(), "next to run");
    }

    #[test]
    fn a_settled_live_run_does_not_shadow_the_item() {
        let mut item = row(Status::Open);
        item.latest_run = Some(run(RunStatus::Cancelled, there()));
        assert_eq!(
            classify_entry(&item, here(), &facts(&item)),
            EntryState::Next,
            "a cancelled run elsewhere is not live"
        );

        item.status = Status::InProgress;
        assert_eq!(
            classify_entry(&item, here(), &LiveFacts::default()).to_string(),
            "not ready: item is in_progress"
        );
    }

    #[test]
    fn a_multi_line_note_shows_its_first_line() {
        let mut item = row(Status::Blocked);
        item.latest_note = Some("first\nsecond".to_owned());
        assert_eq!(
            classify_entry(&item, here(), &facts(&item)).to_string(),
            "blocked (last note: first)"
        );
    }

    #[test]
    fn reveal_names_the_run_and_the_parked_step() {
        let (run, step) = (RunId::new(), StepId::new());
        assert_eq!(
            EntryState::Escalated(Escalation::HardGateParked { run, step }).reveal(),
            (Some(run), Some(step))
        );
        assert_eq!(EntryState::Next.reveal(), (None, None));
        assert_eq!(
            EntryState::Escalated(Escalation::Failed {
                run: Some(run),
                failure: None
            })
            .reveal(),
            (Some(run), None)
        );
        assert_eq!(
            EntryState::Escalated(Escalation::Failed {
                run: None,
                failure: None
            })
            .reveal(),
            (None, None)
        );
        assert_eq!(
            EntryState::Escalated(Escalation::Blocked {
                run: Some(run),
                note: None
            })
            .reveal(),
            (Some(run), None)
        );
        assert_eq!(
            EntryState::Running {
                run,
                status: RunStatus::Running
            }
            .reveal(),
            (Some(run), None)
        );
        assert_eq!(
            EntryState::Elsewhere {
                run,
                hostname: None,
                status: RunStatus::Queued
            }
            .reveal(),
            (Some(run), None)
        );
        for parked in [
            Escalation::JudgeUndecided { run, note: None },
            Escalation::ReviewLoopExhausted { run, note: None },
        ] {
            assert_eq!(EntryState::Escalated(parked).reveal(), (Some(run), None));
        }
        for idle in [
            EntryState::Held(Hold::ProjectGone),
            EntryState::Waiting(Wait::Paused),
            EntryState::Escalated(Escalation::MissingTags(vec!["gpu".to_owned()])),
        ] {
            assert_eq!(idle.reveal(), (None, None), "{idle}");
        }
    }
}
