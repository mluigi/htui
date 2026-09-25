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

    /// ANA-2 §4.3's `item` table (`docs/ANA-2.md:565-589`): whether the orchestrator may move an
    /// item from `self` to `to`. `closed` reaches nothing.
    ///
    /// Every legal pair is a row of that table; a pair outside it is a bug, not a race, and
    /// [`WriteStore::transition`](crate::store::WriteStore::transition) refuses it before it
    /// reaches the row. Note that `is_terminal` and "reaches nothing" are **not** the same set
    /// here: `done` is terminal for the readiness rule and still reaches `open`.
    ///
    /// `closed` is reached only by close-out (PRD D1, [`Resolution::closes_from`]).
    ///
    /// `blocked → awaiting_approval` is MOD-4 plan D161's one deviation from
    /// `docs/ANA-2.md:583-584`. `Unblock` uses it to let an escalated item follow its parked run
    /// back (R-4).
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
            Self::Blocked => matches!(to, Self::Open | Self::AwaitingApproval),
            Self::Failed => matches!(to, Self::Queued),
            Self::Done => matches!(to, Self::Open),
            Self::Closed => false,
        }
    }
}

str_enum!(
    /// `item.resolution` (ANA-11 §4.2): why a `closed` item closed. Set by
    /// [`WriteStore::close_out`](crate::store::WriteStore::close_out) only; `None` on every item
    /// that is not `closed` (`chk_item_resolution_iff_closed`).
    Resolution {
        /// The work was done.
        Done => "done",
        /// An analysis reached its verdict.
        Concluded => "concluded",
        /// Decided against.
        Rejected => "rejected",
        /// Dropped without a decision.
        Withdrawn => "withdrawn",
        /// Replaced by another item.
        Superseded => "superseded",
        /// The same as another item.
        Duplicate => "duplicate",
    }
);

impl Resolution {
    /// ANA-11 §4.2's close-out law (plan D4): `done` and `concluded` close only a `done` item;
    /// the other four close an `open`, `blocked`, `failed` or `done` one. Everything else,
    /// `closed` included, is refused.
    #[must_use]
    pub const fn closes_from(self, status: Status) -> bool {
        match self {
            Self::Done | Self::Concluded => matches!(status, Status::Done),
            Self::Rejected | Self::Withdrawn | Self::Superseded | Self::Duplicate => matches!(
                status,
                Status::Open | Status::Blocked | Status::Failed | Status::Done
            ),
        }
    }

    /// PRD D2 / plan D6: the resolution the Runs pane closes with until MOD-39's picker. `done`
    /// closes as `Done`; `blocked` and `failed` close as `Withdrawn`; nothing else is offered.
    #[must_use]
    pub const fn default_for(status: Status) -> Option<Self> {
        match status {
            Status::Done => Some(Self::Done),
            Status::Blocked | Status::Failed => Some(Self::Withdrawn),
            _ => None,
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
    /// `item.resolution` (ANA-11 §4.2): `Some` exactly when `status` is `closed`.
    #[serde(default)]
    pub resolution: Option<Resolution>,
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
    use super::{Resolution, Status};

    /// Every row of ANA-2 §4.3's item transition table (`docs/ANA-2.md:565-589`), transcribed
    /// from the document rather than from [`Status::can_move_to`], so a pair added to or dropped
    /// from the `match` fails here. The one row not in the document is `blocked →
    /// awaiting_approval`, MOD-4 plan D161's deviation. `→ closed` is not a transition since
    /// MOD-38 (PRD D1); [`CLOSE_OUT_SANCTIONED`] holds the close-out law instead.
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
        // `failed`
        (Status::Failed, Status::Queued),
        // `done`
        (Status::Done, Status::Open),
        // `closed` reaches nothing.
    ];

    /// ANA-11 §4.2's close-out law, transcribed from the document rather than from
    /// [`Resolution::closes_from`].
    const CLOSE_OUT_SANCTIONED: &[(Status, Resolution)] = &[
        (Status::Open, Resolution::Rejected),
        (Status::Open, Resolution::Withdrawn),
        (Status::Open, Resolution::Superseded),
        (Status::Open, Resolution::Duplicate),
        (Status::Blocked, Resolution::Rejected),
        (Status::Blocked, Resolution::Withdrawn),
        (Status::Blocked, Resolution::Superseded),
        (Status::Blocked, Resolution::Duplicate),
        (Status::Failed, Resolution::Rejected),
        (Status::Failed, Resolution::Withdrawn),
        (Status::Failed, Resolution::Superseded),
        (Status::Failed, Resolution::Duplicate),
        (Status::Done, Resolution::Done),
        (Status::Done, Resolution::Concluded),
        (Status::Done, Resolution::Rejected),
        (Status::Done, Resolution::Withdrawn),
        (Status::Done, Resolution::Superseded),
        (Status::Done, Resolution::Duplicate),
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

    #[test]
    fn nothing_transitions_into_closed() {
        for &from in Status::ALL {
            assert!(
                !from.can_move_to(Status::Closed),
                "item.status `{from}` -> `closed` is close-out's, not a transition (PRD D1)"
            );
        }
    }

    #[test]
    fn the_close_out_law_sanctions_exactly_the_ana_11_pairs() {
        assert_eq!(CLOSE_OUT_SANCTIONED.len(), 18);
        for &status in Status::ALL {
            for &resolution in Resolution::ALL {
                let sanctioned = CLOSE_OUT_SANCTIONED.contains(&(status, resolution));
                assert_eq!(
                    resolution.closes_from(status),
                    sanctioned,
                    "close-out of `{status}` as `{resolution}`: the law says {sanctioned}"
                );
            }
        }
    }

    #[test]
    fn the_default_resolution_is_a_sanctioned_close_out() {
        let defaults: Vec<(Status, Option<Resolution>)> = Status::ALL
            .iter()
            .map(|&status| (status, Resolution::default_for(status)))
            .collect();
        for &(status, default) in &defaults {
            if let Some(resolution) = default {
                assert!(
                    resolution.closes_from(status),
                    "the default for `{status}` is `{resolution}`, which cannot close it"
                );
            }
        }
        assert_eq!(
            defaults,
            [
                (Status::Open, None),
                (Status::Queued, None),
                (Status::InProgress, None),
                (Status::AwaitingApproval, None),
                (Status::Blocked, Some(Resolution::Withdrawn)),
                (Status::Done, Some(Resolution::Done)),
                (Status::Failed, Some(Resolution::Withdrawn)),
                (Status::Closed, None),
            ]
        );
    }
}
