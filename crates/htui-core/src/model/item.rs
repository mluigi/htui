//! Items, their filters, their edits and their revision log (`docs/ANA-9.md` §5.5, §4.1, §4.2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{BoxId, ItemId, ItemKindId, ProjectId, StepGraphId, UserId};

str_enum!(
    /// `item.status` (§5.5).
    Status {
        /// Not started.
        Open => "open",
        /// Queued for a run.
        Queued => "queued",
        /// A run is working on it.
        InProgress => "in_progress",
        /// Stopped at a gate, waiting for a human.
        AwaitingApproval => "awaiting_approval",
        /// Blocked by another item.
        Blocked => "blocked",
        /// Finished successfully.
        Done => "done",
        /// Finished unsuccessfully.
        Failed => "failed",
        /// Closed without completion.
        Closed => "closed",
    }
);

impl Status {
    /// Whether the item is finished for the readiness rule (§7.4): a `blocked_by` edge to a
    /// terminal item no longer blocks.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Closed)
    }

    /// ANA-2 §4.3's `item` table (`docs/ANA-2.md:565-589`): whether the orchestrator or close-out
    /// may move an item from `self` to `to`. `closed` reaches nothing.
    ///
    /// Every legal pair is a row of that table; a pair outside it is a bug, not a race, and
    /// [`WriteStore::transition`](crate::store::WriteStore::transition) refuses it before it
    /// reaches the row. Note that `is_terminal` and "reaches nothing" are **not** the same set
    /// here: `done` is terminal for the readiness rule and still reaches `closed` and `open`.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Open => matches!(to, Self::Queued | Self::Blocked),
            Self::Queued => matches!(to, Self::InProgress | Self::Open | Self::Blocked),
            Self::InProgress => matches!(
                to,
                Self::AwaitingApproval | Self::Done | Self::Failed | Self::Blocked | Self::Open
            ),
            Self::AwaitingApproval => matches!(to, Self::InProgress | Self::Failed | Self::Open),
            Self::Blocked => matches!(to, Self::Open | Self::Closed),
            Self::Failed => matches!(to, Self::Queued | Self::Closed),
            Self::Done => matches!(to, Self::Closed | Self::Open),
            Self::Closed => false,
        }
    }
}

/// A row of `item` (§5.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// `item.id`.
    pub id: ItemId,
    /// `item.project_id`.
    pub project_id: ProjectId,
    /// `item.kind_id`.
    pub kind_id: ItemKindId,
    /// `item.key_prefix`, copied from the kind at mint time and never rewritten (§4.1).
    pub key_prefix: String,
    /// `item.key_number`, from the per-`(project, prefix)` counter (§4.1).
    pub key_number: i32,
    /// `item.key`, the generated column `key_prefix || '-' || key_number`. Never settable.
    pub key: String,
    /// `item.title`.
    pub title: String,
    /// `item.body`.
    pub body: String,
    /// `item.status`; moved by its own compare-and-set, never by an edit (§4.2).
    pub status: Status,
    /// `item.priority`; higher runs first (`R-ORCH-6`).
    pub priority: i16,
    /// `item.required_tags`: capabilities the executing box must have (`R-ORCH-10`).
    pub required_tags: Vec<String>,
    /// `item.touched_paths`: declared overlap set, repo-relative globs (`R-ORCH-9`).
    pub touched_paths: Vec<String>,
    /// `item.step_graph_id`; `None` means the kind's default graph.
    pub step_graph_id: Option<StepGraphId>,
    /// `item.version`, the divergence counter of §4.2.
    pub version: i32,
    /// `item.created_by`.
    pub created_by: UserId,
    /// `item.created_at`.
    pub created_at: DateTime<Utc>,
    /// `item.updated_at`.
    pub updated_at: DateTime<Utc>,
    /// `item.closed_at`.
    pub closed_at: Option<DateTime<Utc>>,
}

impl Item {
    /// The list projection of this item.
    #[must_use]
    pub fn summary(&self) -> ItemSummary {
        ItemSummary {
            id: self.id,
            project_id: self.project_id,
            kind_id: self.kind_id,
            key: self.key.clone(),
            key_prefix: self.key_prefix.clone(),
            key_number: self.key_number,
            title: self.title.clone(),
            status: self.status,
            priority: self.priority,
            required_tags: self.required_tags.clone(),
            updated_at: self.updated_at,
            touched_paths: self.touched_paths.clone(),
        }
    }
}

/// List projection: what a Backlog row and its grouping need, nothing else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemSummary {
    /// `item.id`.
    pub id: ItemId,
    /// `item.project_id`.
    pub project_id: ProjectId,
    /// `item.kind_id`.
    pub kind_id: ItemKindId,
    /// `item.key`.
    pub key: String,
    /// `item.key_prefix`, the grouping key of the Backlog list.
    pub key_prefix: String,
    /// `item.key_number`, so rows sort numerically rather than by text.
    pub key_number: i32,
    /// `item.title`.
    pub title: String,
    /// `item.status`.
    pub status: Status,
    /// `item.priority`.
    pub priority: i16,
    /// `item.required_tags`.
    pub required_tags: Vec<String>,
    /// `item.updated_at`.
    pub updated_at: DateTime<Utc>,
    /// `item.touched_paths` (ANA-2 §4.7): the declared overlap set the admission resolves to a
    /// `run.repo_scope`. Appended, never inserted (plan D10): the SQL projections bind
    /// positionally.
    pub touched_paths: Vec<String>,
}

/// Filter passed to [`crate::store::ReadStore::items`]. Every field is a conjunct; `None` means
/// "do not filter on this".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemFilter {
    /// Keep items whose status is in this list. `None` = any status.
    pub statuses: Option<Vec<Status>>,
    /// Keep items in these projects. `None` = every project in the [`crate::model::Scope`].
    pub project_ids: Option<Vec<ProjectId>>,
    /// Keep items whose `required_tags` contain all of these.
    pub tags: Option<Vec<String>>,
    /// Keep only ready items: `status == open` and no live `blocked_by` edge to a non-terminal
    /// item (§7.4, store-side half). The capability half is expressed through `tags` by the
    /// caller; MOD-4 owns matching against a real box.
    pub ready: Option<bool>,
    /// Case-insensitive substring match on `key` and `title`.
    pub text: Option<String>,
}

/// Arguments of [`crate::store::WriteStore::mint_item`] (§7.1).
///
/// No `key_prefix` / `key_number`: §4.1 copies the prefix from the kind at mint time and the
/// counter supplies the number. The importer variant (explicit number) belongs to MOD-8.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewItem {
    /// `item.id`, minted by the caller as UUIDv7 (§3).
    pub id: ItemId,
    /// `item.project_id`.
    pub project_id: ProjectId,
    /// `item.kind_id`; supplies `key_prefix` and must belong to `project_id`.
    pub kind_id: ItemKindId,
    /// `item.title`.
    pub title: String,
    /// `item.body`.
    pub body: String,
    /// `item.required_tags`.
    pub required_tags: Vec<String>,
    /// `item.touched_paths`.
    pub touched_paths: Vec<String>,
    /// `item.priority`.
    pub priority: i16,
    /// `item.step_graph_id`; `None` means the kind's default graph.
    pub step_graph_id: Option<StepGraphId>,
    /// `item.created_by`, also the author of revision 1.
    pub created_by: UserId,
    /// `item_revision.box_id` of the box that minted the item.
    pub box_id: Option<BoxId>,
}

/// Edit passed to [`crate::store::WriteStore::update_item`]: exactly the columns `item.version`
/// covers (§4.2), plus the authorship of the revision the update writes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemPatch {
    /// New `item.title`.
    pub title: Option<String>,
    /// New `item.body`.
    pub body: Option<String>,
    /// New `item.kind_id`.
    pub kind_id: Option<ItemKindId>,
    /// New `item.required_tags`.
    pub required_tags: Option<Vec<String>>,
    /// New `item.priority`.
    pub priority: Option<i16>,
    /// New `item.touched_paths`.
    pub touched_paths: Option<Vec<String>>,
    /// New `item.step_graph_id`; the outer `Option` is "change it", the inner one is the value,
    /// so clearing the override back to the kind default is expressible.
    pub step_graph_id: Option<Option<StepGraphId>>,
    /// `item_revision.author_id` of the revision this edit writes.
    pub author_id: UserId,
    /// `item_revision.box_id` of the box the edit was made on.
    pub box_id: Option<BoxId>,
    /// `item_revision.reason`, e.g. `edited` or `divergence_resolution`.
    pub reason: String,
}

/// A row of `item_revision` (§5.5): the snapshot written on every version-bumping edit, and the
/// ancestor a divergence is rendered against (§4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemRevision {
    /// `item_revision.item_id`.
    pub item_id: ItemId,
    /// `item_revision.version`; version 1 is written at creation, so an ancestor always exists.
    pub version: i32,
    /// `item_revision.title`.
    pub title: String,
    /// `item_revision.body`.
    pub body: String,
    /// `item_revision.required_tags`.
    pub required_tags: Vec<String>,
    /// `item_revision.author_id`.
    pub author_id: UserId,
    /// `item_revision.box_id`.
    pub box_id: Option<BoxId>,
    /// `item_revision.reason`.
    pub reason: String,
    /// `item_revision.created_at`.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::Status;

    /// Every row of ANA-2 §4.3's item transition table (`docs/ANA-2.md:565-589`), transcribed
    /// from the document rather than from [`Status::can_move_to`], so a pair added to or dropped
    /// from the `match` fails here.
    const SANCTIONED: &[(Status, Status)] = &[
        // `open`
        (Status::Open, Status::Queued),
        (Status::Open, Status::Blocked),
        // `queued`
        (Status::Queued, Status::InProgress),
        (Status::Queued, Status::Open),
        (Status::Queued, Status::Blocked),
        // `in_progress`
        (Status::InProgress, Status::AwaitingApproval),
        (Status::InProgress, Status::Done),
        (Status::InProgress, Status::Failed),
        (Status::InProgress, Status::Blocked),
        (Status::InProgress, Status::Open),
        // `awaiting_approval`
        (Status::AwaitingApproval, Status::InProgress),
        (Status::AwaitingApproval, Status::Failed),
        (Status::AwaitingApproval, Status::Open),
        // `blocked`
        (Status::Blocked, Status::Open),
        (Status::Blocked, Status::AwaitingApproval),
        (Status::Blocked, Status::Closed),
        // `failed`
        (Status::Failed, Status::Queued),
        (Status::Failed, Status::Closed),
        // `done`
        (Status::Done, Status::Closed),
        (Status::Done, Status::Open),
        // `closed` reaches nothing.
    ];

    #[test]
    fn the_item_status_table_sanctions_exactly_the_ana_2_pairs() {
        for &from in Status::ALL {
            for &to in Status::ALL {
                let sanctioned = SANCTIONED.contains(&(from, to));
                assert_eq!(
                    from.can_move_to(to),
                    sanctioned,
                    "item.status `{from}` -> `{to}`: the table says {sanctioned}"
                );
            }
        }
    }

    #[test]
    fn a_closed_item_reaches_no_other_status() {
        for &to in Status::ALL {
            assert!(
                !Status::Closed.can_move_to(to),
                "item.status `closed` is terminal, so it cannot reach `{to}`"
            );
        }
    }
}
