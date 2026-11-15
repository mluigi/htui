//! Items, their filters, their edits and their revision log (`docs/ANA-9.md` §5.5, §4.1, §4.2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{BoxId, ItemId, ItemKindId, ProjectId, StepGraphId, TargetId, UserId};
use crate::model::integration::IntegrationState;
use crate::model::run::RunStatus;

str_enum!(
    /// `item.status` (MOD-144, ANA-35 §7.1): what a person decided. Progress is the latest
    /// graph run's, derived by [`ItemState::of`].
    Status {
        /// Not blocked and not closed; its runs say how far it got.
        Open => "open",
        /// A human must clear this, with a note (ANA-2).
        Blocked => "blocked",
        /// Closed by close-out, with a resolution.
        Closed => "closed",
    }
);

impl Status {
    /// ANA-35 §7.1's `item` table (MOD-144): whether the orchestrator may move an item from
    /// `self` to `to`. `open ↔ blocked` only; `closed` reaches nothing.
    ///
    /// A pair outside the table is a bug, not a race, and
    /// [`WriteStore::transition`](crate::store::WriteStore::transition) refuses it before it
    /// reaches the row. Progress (`queued`, `running`, `needs you`, `stopped`, `failed`, `done`)
    /// is never stored: it is the latest graph run's ([`ItemState::of`]).
    ///
    /// `closed` is reached only by close-out (PRD D1, [`Resolution::closes_from`]).
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Open, Self::Blocked) | (Self::Blocked, Self::Open)
        )
    }
}

str_enum!(
    /// The display state of an item (MOD-144, ANA-35 §7.1): derived, never stored. `ALL` is the
    /// attention order the Backlog sorts and groups by.
    ItemState {
        /// `open` over a run parked at `awaiting_approval`.
        NeedsYou => "needs_you",
        /// `open` over a `failed` latest graph run.
        Failed => "failed",
        /// `open` over a `done` latest graph run with a `refused` integration record (MOD-133
        /// M2): the work did not reach its remote, and Unblock retries it. Word `landing refused`.
        LandRefused => "land_refused",
        /// `open` over a `running` latest graph run.
        Running => "running",
        /// `open` over a `done` latest graph run with an integration record not yet landed and
        /// none refused (MOD-133 M2): its box is landing the work.
        Landing => "landing",
        /// `open` over a `stopped` latest graph run (MOD-125: live, resumable, holds no slot).
        Stopped => "stopped",
        /// `item.status = 'blocked'`, whatever its runs did.
        Blocked => "blocked",
        /// `open` over a `queued` latest graph run.
        Queued => "queued",
        /// `open` with no non-cancelled graph run.
        Open => "open",
        /// `open` over a `done` latest graph run with every integration record landed, or none
        /// (every `local` run): finished.
        Done => "done",
        /// `item.status = 'closed'`.
        Closed => "closed",
    }
);

impl ItemState {
    /// `closed`, `blocked`, else the latest non-cancelled graph run's state, else `open`:
    /// [`ItemState::of_landing`] with no landing, which is what a reader that holds no
    /// integration records derives (a `done` run reads `done`).
    #[must_use]
    pub const fn of(status: Status, latest: Option<RunStatus>) -> Self {
        Self::of_landing(status, latest, None)
    }

    /// MOD-133 M2 E2: `closed` wins, then `blocked`; over a `done` latest graph run a
    /// [`RunLanding::Refused`] reads `land_refused` and a [`RunLanding::Pending`] `landing`;
    /// everything else is [`ItemState::of`]'s answer, whatever `landing` says.
    #[must_use]
    pub const fn of_landing(
        status: Status,
        latest: Option<RunStatus>,
        landing: Option<RunLanding>,
    ) -> Self {
        match (status, latest, landing) {
            (Status::Closed, _, _) => Self::Closed,
            (Status::Blocked, _, _) => Self::Blocked,
            (Status::Open, Some(RunStatus::AwaitingApproval), _) => Self::NeedsYou,
            (Status::Open, Some(RunStatus::Failed), _) => Self::Failed,
            (Status::Open, Some(RunStatus::Running), _) => Self::Running,
            (Status::Open, Some(RunStatus::Stopped), _) => Self::Stopped,
            (Status::Open, Some(RunStatus::Queued), _) => Self::Queued,
            (Status::Open, Some(RunStatus::Done), Some(RunLanding::Refused)) => Self::LandRefused,
            (Status::Open, Some(RunStatus::Done), Some(RunLanding::Pending)) => Self::Landing,
            (Status::Open, Some(RunStatus::Done), None) => Self::Done,
            // "Latest" skips cancelled runs; a caller passing one gets what a cancel leaves.
            (Status::Open, Some(RunStatus::Cancelled) | None, _) => Self::Open,
        }
    }

    /// The redesign's word: `needs you` and `landing refused` (a space); every other state its
    /// `as_str`.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs you",
            Self::LandRefused => "landing refused",
            other => other.as_str(),
        }
    }

    /// Finished: the latest graph run is `done`, landed or not (`done`, `landing`, `landing
    /// refused`). Whether `done` also requires landed is the close-out law's, MOD-135's; both
    /// stores' `close_out` read `latest == done`, so close-out does not move with the landing.
    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Landing | Self::LandRefused)
    }
}

str_enum!(
    /// MOD-133 M2 E1: the aggregate landing of one run's `integration` records, the one an item
    /// whose latest graph run is `done` reads its state from ([`ItemState::of_landing`]). Named
    /// `RunLanding` because htui-orch already has a `Landing`.
    RunLanding {
        /// A record is not landed and none is refused: the box is landing the work.
        Pending => "pending",
        /// A record is refused: Unblock retries it.
        Refused => "refused",
    }
);

impl RunLanding {
    /// Any `refused` record → [`RunLanding::Refused`]; else any record not
    /// [`IntegrationState::is_landed`] (`pending`, `pr_open`, `pr_closed`; MOD-134 refines the PR
    /// states) → [`RunLanding::Pending`]; else (all landed, or no records) → `None`.
    #[must_use]
    pub fn of(states: impl IntoIterator<Item = IntegrationState>) -> Option<Self> {
        let mut landing = None;
        for state in states {
            if state == IntegrationState::Refused {
                return Some(Self::Refused);
            }
            if !state.is_landed() {
                landing = Some(Self::Pending);
            }
        }
        landing
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
    /// ANA-35 §7.1's close-out law (replaces MOD-38 D1; maintainer decision PD18, 2026-10-27):
    /// `done` and `concluded` close only an `open` item that is `finished` (latest graph run
    /// `done`); `rejected`, `withdrawn`, `superseded` and `duplicate` close an `open` or `blocked`
    /// one whatever its runs did. A `blocked` finished item is unblocked first, then closed as
    /// `done`. `closed` is refused. The live-run refusal is the caller's and comes first.
    #[must_use]
    pub const fn closes_from(self, status: Status, finished: bool) -> bool {
        match self {
            Self::Done | Self::Concluded => matches!(status, Status::Open) && finished,
            Self::Rejected | Self::Withdrawn | Self::Superseded | Self::Duplicate => {
                matches!(status, Status::Open | Status::Blocked)
            }
        }
    }

    /// The resolution the Runs pane's picker starts on (MOD-39 P13, ANA-35 §7.1, PD18).
    /// `finished_output_kind` is the `output_kind` of the last phase of the item's latest graph
    /// run when that run is `done` ([`GraphSnapshot::last_output_kind`](crate::model::GraphSnapshot::last_output_kind),
    /// `Some("")` for an unreadable snapshot), `None` when nothing finished.
    ///
    /// A `blocked` item starts on `withdrawn`, finished or not (PD18: it cannot close `done`).
    /// An `open` one: nothing finished → `withdrawn`; `Some("verdict")` → `concluded`; any other
    /// finished kind → `done`. A `closed` item is refused by the caller first; this answers
    /// `withdrawn` for it.
    #[must_use]
    pub fn default_for(status: Status, finished_output_kind: Option<&str>) -> Self {
        match (status, finished_output_kind) {
            (Status::Open, Some("verdict")) => Self::Concluded,
            (Status::Open, Some(_)) => Self::Done,
            (Status::Open, None) | (Status::Blocked | Status::Closed, _) => Self::Withdrawn,
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
    /// `item.target_id` (MOD-130 M1 D11): the item's execution target, a spec field under
    /// `version`; `None` inherits the workflow's, the project's, then the Settings default (D3).
    pub target_id: Option<TargetId>,
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
    /// The list projection of this item: the item row alone, so `latest_run_status` and
    /// `latest_run_landing` are `None`; a store fills them.
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
            latest_run_status: None,
            latest_run_landing: None,
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
    /// MOD-144: the status of the item's latest `kind = 'graph'` run that is not `cancelled`, by
    /// `(queued_at, id)`; `None` when it has none. Appended, never inserted (plan D10).
    #[serde(default)]
    pub latest_run_status: Option<RunStatus>,
    /// MOD-133 M2 E3: the [`RunLanding`] of the records of **the same run** `latest_run_status`
    /// reads; `None` with no run, a run that wrote no record, or every record landed. Appended,
    /// never inserted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_run_landing: Option<RunLanding>,
}

impl ItemSummary {
    /// The derived display state (MOD-144, MOD-133 M2): [`ItemState::of_landing`] the stored
    /// status, the latest graph run's and its landing.
    #[must_use]
    pub const fn state(&self) -> ItemState {
        ItemState::of_landing(self.status, self.latest_run_status, self.latest_run_landing)
    }
}

/// Filter passed to [`crate::store::ReadStore::items`]. Every field is a conjunct; `None` means
/// "do not filter on this".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemFilter {
    /// Keep items whose stored status is in this list (MOD-11). `None` = any status.
    pub statuses: Option<Vec<Status>>,
    /// Keep items in these projects. `None` = every project in the [`crate::model::Scope`].
    pub project_ids: Option<Vec<ProjectId>>,
    /// Keep items whose `required_tags` contain all of these.
    pub tags: Option<Vec<String>>,
    /// Keep only ready items (§7.4, store-side half; MOD-144): `open`, no non-cancelled graph
    /// run, and every live `blocked_by` target released (closed, or latest graph run `done`).
    /// The capability half is expressed through `tags` by the caller; MOD-4 owns matching
    /// against a real box.
    pub ready: Option<bool>,
    /// Literal substring of `key` or `title`, case-insensitive as far as the backend folds case:
    /// Unicode in `MemStore`, ASCII only in the SQLite mirror, the database's locale in Postgres.
    pub text: Option<String>,
    /// MOD-144: keep items whose derived [`ItemState`] is in this list; `None` = any.
    #[serde(default)]
    pub states: Option<Vec<ItemState>>,
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
    /// `item.target_id` (MOD-130 M1 D11); `None` inherits (D3).
    pub target_id: Option<TargetId>,
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
    /// New `item.target_id` (MOD-130 M1 D11), `step_graph_id`'s double option: `Some(None)` sets
    /// the item back to inherit (D3).
    pub target_id: Option<Option<TargetId>>,
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
    use super::{ItemState, Resolution, RunLanding, Status};
    use crate::model::integration::IntegrationState;
    use crate::model::run::RunStatus;

    /// ANA-35 §7.1's item table (MOD-144), transcribed from the document rather than from
    /// [`Status::can_move_to`]: `open ↔ blocked` only. `→ closed` is close-out's (MOD-38 PRD
    /// D1); [`CLOSE_OUT_SANCTIONED`] holds that law instead.
    const SANCTIONED: &[(Status, Status)] = &[
        (Status::Open, Status::Blocked),
        (Status::Blocked, Status::Open),
    ];

    /// ANA-35 §7.1's close-out law with PD18 (maintainer, 2026-10-27), transcribed from the
    /// document rather than from [`Resolution::closes_from`]: `(status, finished, resolution)`.
    /// The four non-success resolutions close `open` or `blocked` whatever its runs did; `done`
    /// and `concluded` close only an `open` finished item.
    const CLOSE_OUT_SANCTIONED: &[(Status, bool, Resolution)] = &[
        (Status::Open, false, Resolution::Rejected),
        (Status::Open, false, Resolution::Withdrawn),
        (Status::Open, false, Resolution::Superseded),
        (Status::Open, false, Resolution::Duplicate),
        (Status::Open, true, Resolution::Rejected),
        (Status::Open, true, Resolution::Withdrawn),
        (Status::Open, true, Resolution::Superseded),
        (Status::Open, true, Resolution::Duplicate),
        (Status::Open, true, Resolution::Done),
        (Status::Open, true, Resolution::Concluded),
        (Status::Blocked, false, Resolution::Rejected),
        (Status::Blocked, false, Resolution::Withdrawn),
        (Status::Blocked, false, Resolution::Superseded),
        (Status::Blocked, false, Resolution::Duplicate),
        (Status::Blocked, true, Resolution::Rejected),
        (Status::Blocked, true, Resolution::Withdrawn),
        (Status::Blocked, true, Resolution::Superseded),
        (Status::Blocked, true, Resolution::Duplicate),
    ];

    /// ANA-35 §7.1's display table plus Decision A (`stopped`), one row per `(status, latest
    /// graph run)`: 3 × (1 + 7) = 24 rows.
    const DERIVATION: &[(Status, Option<RunStatus>, ItemState)] = &[
        (Status::Open, None, ItemState::Open),
        (Status::Open, Some(RunStatus::Queued), ItemState::Queued),
        (Status::Open, Some(RunStatus::Running), ItemState::Running),
        (
            Status::Open,
            Some(RunStatus::AwaitingApproval),
            ItemState::NeedsYou,
        ),
        (Status::Open, Some(RunStatus::Stopped), ItemState::Stopped),
        (Status::Open, Some(RunStatus::Done), ItemState::Done),
        (Status::Open, Some(RunStatus::Failed), ItemState::Failed),
        (Status::Open, Some(RunStatus::Cancelled), ItemState::Open),
        (Status::Blocked, None, ItemState::Blocked),
        (Status::Blocked, Some(RunStatus::Queued), ItemState::Blocked),
        (
            Status::Blocked,
            Some(RunStatus::Running),
            ItemState::Blocked,
        ),
        (
            Status::Blocked,
            Some(RunStatus::AwaitingApproval),
            ItemState::Blocked,
        ),
        (
            Status::Blocked,
            Some(RunStatus::Stopped),
            ItemState::Blocked,
        ),
        (Status::Blocked, Some(RunStatus::Done), ItemState::Blocked),
        (Status::Blocked, Some(RunStatus::Failed), ItemState::Blocked),
        (
            Status::Blocked,
            Some(RunStatus::Cancelled),
            ItemState::Blocked,
        ),
        (Status::Closed, None, ItemState::Closed),
        (Status::Closed, Some(RunStatus::Queued), ItemState::Closed),
        (Status::Closed, Some(RunStatus::Running), ItemState::Closed),
        (
            Status::Closed,
            Some(RunStatus::AwaitingApproval),
            ItemState::Closed,
        ),
        (Status::Closed, Some(RunStatus::Stopped), ItemState::Closed),
        (Status::Closed, Some(RunStatus::Done), ItemState::Closed),
        (Status::Closed, Some(RunStatus::Failed), ItemState::Closed),
        (
            Status::Closed,
            Some(RunStatus::Cancelled),
            ItemState::Closed,
        ),
    ];

    #[test]
    fn item_state_of_reads_status_then_latest_run() {
        assert_eq!(DERIVATION.len(), 24);
        for &status in Status::ALL {
            let latest = core::iter::once(None).chain(RunStatus::ALL.iter().copied().map(Some));
            for run in latest {
                let row = DERIVATION
                    .iter()
                    .find(|(s, r, _)| *s == status && *r == run)
                    .unwrap_or_else(|| panic!("no table row for ({status}, {run:?})"));
                assert_eq!(
                    ItemState::of(status, run),
                    row.2,
                    "ItemState::of({status}, {run:?})"
                );
            }
        }
    }

    /// MOD-133 M2 E2: the landing moves only an `open` item over a `done` latest graph run:
    /// `refused` reads `landing refused`, `pending` reads `landing`; every other `(status, latest)`
    /// reads exactly what [`ItemState::of`] answers, whatever the landing.
    #[test]
    fn item_state_of_landing_moves_only_an_open_done_item() {
        let landings = [None, Some(RunLanding::Pending), Some(RunLanding::Refused)];
        for &status in Status::ALL {
            let latest = core::iter::once(None).chain(RunStatus::ALL.iter().copied().map(Some));
            for run in latest {
                for landing in landings {
                    let expected = match (status, run, landing) {
                        (Status::Open, Some(RunStatus::Done), Some(RunLanding::Refused)) => {
                            ItemState::LandRefused
                        }
                        (Status::Open, Some(RunStatus::Done), Some(RunLanding::Pending)) => {
                            ItemState::Landing
                        }
                        _ => ItemState::of(status, run),
                    };
                    assert_eq!(
                        ItemState::of_landing(status, run, landing),
                        expected,
                        "ItemState::of_landing({status}, {run:?}, {landing:?})"
                    );
                }
            }
        }
        assert_eq!(
            ItemState::of_landing(Status::Open, Some(RunStatus::Done), None),
            ItemState::Done,
            "all landed, or no records (`local`), is `done`"
        );
        assert_eq!(
            ItemState::of_landing(
                Status::Closed,
                Some(RunStatus::Done),
                Some(RunLanding::Refused)
            ),
            ItemState::Closed,
            "`closed` wins"
        );
        assert_eq!(
            ItemState::of_landing(
                Status::Blocked,
                Some(RunStatus::Done),
                Some(RunLanding::Pending)
            ),
            ItemState::Blocked,
            "then `blocked`"
        );
    }

    /// MOD-133 M2 E1: any `refused` record is `Refused`; else any record not landed is `Pending`;
    /// all landed, or none, is `None`.
    #[test]
    fn run_landing_of_reads_refused_then_unlanded() {
        use IntegrationState::{Landed, Pending, PrClosed, PrMerged, PrOpen, Refused};
        for (states, expected) in [
            (vec![], None),
            (vec![Landed], None),
            (vec![Landed, PrMerged], None),
            (vec![Pending], Some(RunLanding::Pending)),
            (vec![Landed, PrOpen], Some(RunLanding::Pending)),
            (vec![PrClosed, Landed], Some(RunLanding::Pending)),
            (vec![Refused], Some(RunLanding::Refused)),
            (vec![Pending, Refused, Landed], Some(RunLanding::Refused)),
            (vec![Landed, Refused], Some(RunLanding::Refused)),
        ] {
            assert_eq!(RunLanding::of(states.clone()), expected, "{states:?}");
        }
        assert_eq!(
            RunLanding::ALL
                .iter()
                .map(|landing| landing.as_str())
                .collect::<Vec<_>>(),
            ["pending", "refused"]
        );
    }

    #[test]
    fn item_state_all_is_the_attention_order() {
        assert_eq!(
            ItemState::ALL,
            [
                ItemState::NeedsYou,
                ItemState::Failed,
                ItemState::LandRefused,
                ItemState::Running,
                ItemState::Landing,
                ItemState::Stopped,
                ItemState::Blocked,
                ItemState::Queued,
                ItemState::Open,
                ItemState::Done,
                ItemState::Closed,
            ]
        );
    }

    /// B1: `as_str` equals `RunStatus::as_str` where both name a run state, so SQL derives the
    /// state with one `CASE`; the word spells `needs you` and `landing refused` with a space.
    #[test]
    fn item_state_word_spells_needs_you() {
        assert_eq!(ItemState::NeedsYou.as_str(), "needs_you");
        assert_eq!(ItemState::NeedsYou.word(), "needs you");
        assert_eq!(ItemState::LandRefused.as_str(), "land_refused");
        assert_eq!(ItemState::LandRefused.word(), "landing refused");
        assert_eq!(ItemState::Landing.word(), "landing");
        for &state in ItemState::ALL {
            if !matches!(state, ItemState::NeedsYou | ItemState::LandRefused) {
                assert_eq!(state.word(), state.as_str());
            }
        }
        for run in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Stopped,
            RunStatus::Done,
            RunStatus::Failed,
        ] {
            assert_eq!(
                ItemState::of(Status::Open, Some(run)).as_str(),
                run.as_str()
            );
        }
        // MOD-133 M2 E2: the run finished, landed or not (the close-out law is MOD-135's).
        for state in [ItemState::Done, ItemState::Landing, ItemState::LandRefused] {
            assert!(state.is_finished(), "{state}");
        }
        assert_eq!(ItemState::ALL.iter().filter(|s| s.is_finished()).count(), 3);
    }

    #[test]
    fn the_item_status_table_sanctions_exactly_open_and_blocked() {
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
    fn the_close_out_law_sanctions_exactly_the_ana_35_triples() {
        assert_eq!(CLOSE_OUT_SANCTIONED.len(), 18);
        for &status in Status::ALL {
            for finished in [false, true] {
                for &resolution in Resolution::ALL {
                    let sanctioned = CLOSE_OUT_SANCTIONED.contains(&(status, finished, resolution));
                    assert_eq!(
                        resolution.closes_from(status, finished),
                        sanctioned,
                        "close-out of `{status}` (finished {finished}) as `{resolution}`: \
                         the law says {sanctioned}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_default_resolution_is_a_sanctioned_close_out() {
        let open: Vec<(Option<&str>, Resolution)> = [None, Some(""), Some("plan"), Some("verdict")]
            .into_iter()
            .map(|kind| (kind, Resolution::default_for(Status::Open, kind)))
            .collect();
        assert_eq!(
            open,
            [
                (None, Resolution::Withdrawn),
                (Some(""), Resolution::Done),
                (Some("plan"), Resolution::Done),
                (Some("verdict"), Resolution::Concluded),
            ]
        );
        for (kind, resolution) in open {
            assert!(
                resolution.closes_from(Status::Open, kind.is_some()),
                "the default for an open item ({kind:?}) is `{resolution}`, which cannot close it"
            );
        }
        // PD18: a blocked item starts on `withdrawn`, finished or not.
        for kind in [None, Some(""), Some("plan"), Some("verdict")] {
            let resolution = Resolution::default_for(Status::Blocked, kind);
            assert_eq!(resolution, Resolution::Withdrawn, "blocked, {kind:?}");
            assert!(resolution.closes_from(Status::Blocked, kind.is_some()));
        }
    }
}
