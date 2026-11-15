//! Requirements, their areas, the spec header and citations (`docs/ANA-11.md` §4, §5).

use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{
    BoxId, ItemId, ProjectId, RequirementAreaId, RequirementId, StepId, UserId,
};
use crate::model::item::{ItemSummary, Resolution, Status};
use crate::model::kind::ItemKind;

str_enum!(
    /// `requirement.priority` (ANA-11 §5).
    Priority {
        /// Required for the product to be what it claims.
        Must => "must",
        /// Wanted, not yet committed to.
        Later => "later",
    }
);

str_enum!(
    /// `requirement.state` (ANA-11 §5).
    RequirementState {
        /// In force.
        Active => "active",
        /// Retired by a deciding item; takes no new `addresses` / `reserves` citation.
        Withdrawn => "withdrawn",
    }
);

str_enum!(
    /// `item_requirement.kind` (ANA-11 §4.3).
    CitationKind {
        /// The item implements the requirement.
        Addresses => "addresses",
        /// The item decided a change to the requirement's text.
        Amends => "amends",
        /// The item decided to retire the requirement.
        Withdraws => "withdraws",
        /// The item claims the requirement for later work.
        Reserves => "reserves",
    }
);

/// A row of `requirement_spec`: one header per project (ANA-11 §4.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementSpec {
    /// `requirement_spec.project_id`, the primary key.
    pub project_id: ProjectId,
    /// `requirement_spec.owner_id`.
    pub owner_id: UserId,
    /// `requirement_spec.preamble`: preamble, out-of-scope and superseded-material prose.
    pub preamble: String,
    /// `requirement_spec.version`: the compare-and-set token (plan D9).
    pub version: i32,
    /// `requirement_spec.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `requirement_area`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementArea {
    /// `requirement_area.id`.
    pub id: RequirementAreaId,
    /// `requirement_area.project_id`.
    pub project_id: ProjectId,
    /// `requirement_area.code`, `^[A-Z][A-Z0-9]{1,15}$`: the `ENT` of `R-ENT-3`.
    pub code: String,
    /// `requirement_area.title`.
    pub title: String,
    /// `requirement_area.description`.
    pub description: String,
    /// `requirement_area.position`: list order within the project.
    pub position: i32,
    /// `requirement_area.updated_at`.
    pub updated_at: DateTime<Utc>,
}

impl RequirementArea {
    /// The `requirement_area.code` CHECK, byte for byte the `item_kind.prefix` one.
    #[must_use]
    pub fn code_is_valid(code: &str) -> bool {
        ItemKind::prefix_is_valid(code)
    }
}

/// Arguments of [`WriteStore::create_requirement_area`](crate::store::WriteStore::create_requirement_area).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRequirementArea {
    /// Client-minted id.
    pub id: RequirementAreaId,
    /// The owning project.
    pub project_id: ProjectId,
    /// The area code.
    pub code: String,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// The list position.
    pub position: i32,
}

/// A row of `requirement`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requirement {
    /// `requirement.id`.
    pub id: RequirementId,
    /// `requirement.project_id`.
    pub project_id: ProjectId,
    /// `requirement.area_id`.
    pub area_id: RequirementAreaId,
    /// `requirement.area_code`, copied at mint.
    pub area_code: String,
    /// `requirement.number`, per area, never reused; `None` on a nested child (MOD-149), which
    /// takes [`child_number`](Self::child_number) instead.
    pub number: Option<i32>,
    /// `requirement.key`, stored at mint (MOD-149 D2): [`Requirement::top_level_key`] for a
    /// top-level row, [`Requirement::child_key`] for a nested one.
    pub key: String,
    /// `requirement.refines_id`: the requirement this one refines (MOD-149), nested under it or
    /// across owners; `None` on a plain top-level row, and after the parent is deleted.
    pub refines_id: Option<RequirementId>,
    /// `requirement.child_number`: the `.n` of a nested child, per parent, never reused; `None`
    /// on a top-level row.
    pub child_number: Option<i32>,
    /// `requirement.body`.
    pub body: String,
    /// `requirement.rationale`.
    pub rationale: String,
    /// `requirement.priority`.
    pub priority: Priority,
    /// `requirement.state`.
    pub state: RequirementState,
    /// `requirement.version`: the compare-and-set token (ANA-9 §4.2).
    pub version: i32,
    /// `requirement.created_by`.
    pub created_by: UserId,
    /// `requirement.created_at`.
    pub created_at: DateTime<Utc>,
    /// `requirement.updated_at`.
    pub updated_at: DateTime<Utc>,
}

impl Requirement {
    /// Plan D11: a citation stamped at `stamp` is suspect when this requirement is newer.
    #[must_use]
    pub const fn makes_suspect(&self, stamp: i32) -> bool {
        self.version > stamp
    }

    /// MOD-149 D2: a top-level key, `W-` in a workspace's scope row, `R-` in a project.
    #[must_use]
    pub fn top_level_key(scope_row: bool, area_code: &str, number: i32) -> String {
        let letter = if scope_row { 'W' } else { 'R' };
        format!("{letter}-{area_code}-{number}")
    }

    /// MOD-149 D2: a nested child's key, `{parent_key}.{child_number}`, at any depth.
    #[must_use]
    pub fn child_key(parent_key: &str, child_number: i32) -> String {
        format!("{parent_key}.{child_number}")
    }

    /// The `chk_requirement_key_shape` CHECK,
    /// `^[RW]-[A-Z][A-Z0-9]*-[1-9][0-9]*(\.[1-9][0-9]*)*$`, without a regex.
    #[must_use]
    pub fn key_is_valid(key: &str) -> bool {
        let Some(rest) = key.strip_prefix("R-").or_else(|| key.strip_prefix("W-")) else {
            return false;
        };
        let Some((code, path)) = rest.split_once('-') else {
            return false;
        };
        let mut code_bytes = code.bytes();
        let code_ok = code_bytes.next().is_some_and(|b| b.is_ascii_uppercase())
            && code_bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        code_ok
            && path.split('.').all(|part| {
                let mut digits = part.bytes();
                digits.next().is_some_and(|b| matches!(b, b'1'..=b'9'))
                    && digits.all(|b| b.is_ascii_digit())
            })
    }

    /// The numbers of the key after its last `-`, split on `.`: `[1, 2]` for `R-ENT-1.2`. A part
    /// that does not parse reads as `i32::MAX`, which a stored key (the CHECK) never holds.
    #[must_use]
    pub fn key_path(&self) -> Vec<i32> {
        self.key_parts().collect()
    }

    /// [`key_path`](Self::key_path), lazily: what [`list_cmp`](Self::list_cmp) compares, so a
    /// sort allocates nothing.
    fn key_parts(&self) -> impl Iterator<Item = i32> + '_ {
        self.key
            .rsplit_once('-')
            .map_or("", |(_, path)| path)
            .split('.')
            .map(|part| part.parse().unwrap_or(i32::MAX))
    }

    /// MOD-149 D6: the list order, `(area_code bytes, key_path, key bytes)`, so a child follows
    /// its parent and `R-ENT-10` follows `R-ENT-9`. The paths compare part by part as they parse.
    #[must_use]
    pub fn list_cmp(&self, other: &Self) -> Ordering {
        self.area_code
            .cmp(&other.area_code)
            .then_with(|| self.key_parts().cmp(other.key_parts()))
            .then_with(|| self.key.cmp(&other.key))
    }

    /// Nested under another requirement (MOD-149): it has a `child_number`.
    #[must_use]
    pub const fn is_nested(&self) -> bool {
        self.child_number.is_some()
    }
}

/// A requirement's coverage (MOD-149 D8), derived on read, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    /// Neither of the others: no live `addresses` citation, or only ones whose items closed as
    /// neither `done` nor `concluded` (`rejected`, `duplicate`, …), which count as nothing. A
    /// split requirement whose every live refinement is `Open`.
    Open,
    /// Not covered, but a live `addresses` citation's item is open or blocked, or closed `done` /
    /// `concluded` at an older version (the citation is suspect). A split requirement with a
    /// live refinement `Covered` or `InProgress` and not every one `Covered`.
    InProgress,
    /// A live, non-suspect `addresses` citation's item is closed `done` or `concluded`. A split
    /// requirement whose every live refinement is `Covered`.
    Covered,
}

impl CoverageState {
    /// The leaf rule: `Covered` when a live `addresses` citation is not suspect and its item is
    /// closed as `done` or `concluded`; else `InProgress` when an `addresses` citation's item is
    /// still open or blocked, or finished it at an older version (suspect); else `Open`. An item
    /// closed any other way (`rejected`, `withdrawn`, …) gave the work up, and other kinds never
    /// count.
    #[must_use]
    pub fn of_citations(rows: &[CoverageRow]) -> Self {
        let addresses = || {
            rows.iter()
                .filter(|row| row.kind == CitationKind::Addresses)
        };
        let finished = |row: &CoverageRow| {
            row.item.status == Status::Closed
                && matches!(
                    row.resolution,
                    Some(Resolution::Done | Resolution::Concluded)
                )
        };
        if addresses().any(|row| !row.suspect && finished(row)) {
            Self::Covered
        } else if addresses().any(|row| row.item.status != Status::Closed || finished(row)) {
            Self::InProgress
        } else {
            Self::Open
        }
    }

    /// The split rule: `None` with no children; `Covered` when every child is; `InProgress`
    /// when any is covered or in progress; else `Open`.
    #[must_use]
    pub fn of_splits(children: impl IntoIterator<Item = Self>) -> Option<Self> {
        let mut any = false;
        let mut all_covered = true;
        let mut any_started = false;
        for child in children {
            any = true;
            all_covered &= child == Self::Covered;
            any_started |= child != Self::Open;
        }
        if !any {
            None
        } else if all_covered {
            Some(Self::Covered)
        } else if any_started {
            Some(Self::InProgress)
        } else {
            Some(Self::Open)
        }
    }
}

/// Arguments of [`WriteStore::mint_requirement`](crate::store::WriteStore::mint_requirement).
/// The area, project, code and number come from the area and its counter, never the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRequirement {
    /// Client-minted id.
    pub id: RequirementId,
    /// The body.
    pub body: String,
    /// The rationale.
    pub rationale: String,
    /// The priority.
    pub priority: Priority,
    /// `requirement.created_by`, and the `author_id` of revision 1.
    pub created_by: UserId,
    /// `box_id` of revision 1.
    pub box_id: Option<BoxId>,
    /// `requirement.refines_id` (MOD-149 D4): `Some` nests the new row under that requirement
    /// when it is in the same project (the area must then be the parent's), or refines it across
    /// owners when it is a workspace requirement the project's workspace holds.
    pub refines_id: Option<RequirementId>,
}

/// The edit of [`WriteStore::amend_requirement`](crate::store::WriteStore::amend_requirement):
/// `None` leaves a column as it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequirementPatch {
    /// New body.
    pub body: Option<String>,
    /// New rationale.
    pub rationale: Option<String>,
    /// New priority.
    pub priority: Option<Priority>,
    /// The revision's `author_id`.
    pub author_id: UserId,
    /// The revision's `box_id`.
    pub box_id: Option<BoxId>,
    /// The revision's `reason`: `"amended"` for an ordinary amend.
    pub reason: String,
}

/// A row of `requirement_revision`: the requirement as it stood at `version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementRevision {
    /// `requirement_revision.requirement_id`.
    pub requirement_id: RequirementId,
    /// `requirement_revision.version`.
    pub version: i32,
    /// `requirement_revision.body`.
    pub body: String,
    /// `requirement_revision.rationale`.
    pub rationale: String,
    /// `requirement_revision.priority`.
    pub priority: Priority,
    /// `requirement_revision.state`.
    pub state: RequirementState,
    /// `requirement_revision.author_id`.
    pub author_id: UserId,
    /// `requirement_revision.box_id`.
    pub box_id: Option<BoxId>,
    /// `requirement_revision.reason`: `created`, `amended`, `withdrawn`, `imported`,
    /// `divergence_resolution` (no CHECK, like `item_revision.reason`).
    pub reason: String,
    /// `requirement_revision.amended_by_item_id`: the deciding item.
    pub amended_by_item_id: Option<ItemId>,
    /// `requirement_revision.created_at`.
    pub created_at: DateTime<Utc>,
}

/// A row of `item_requirement`, tombstone included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemRequirement {
    /// `item_requirement.item_id`.
    pub item_id: ItemId,
    /// `item_requirement.requirement_id`.
    pub requirement_id: RequirementId,
    /// `item_requirement.kind`.
    pub kind: CitationKind,
    /// `item_requirement.requirement_version`: the stamp.
    pub requirement_version: i32,
    /// `item_requirement.proposed_by_step_id`; `None` = a human or the importer.
    pub proposed_by_step_id: Option<StepId>,
    /// `item_requirement.created_at`.
    pub created_at: DateTime<Utc>,
    /// `item_requirement.updated_at`.
    pub updated_at: DateTime<Utc>,
    /// `item_requirement.deleted_at`; `None` on a live citation.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// One live citation of an item, as
/// [`ReadStore::item_requirements`](crate::store::ReadStore::item_requirements) answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemCitation {
    /// The cited requirement as it is now.
    pub requirement: Requirement,
    /// The citation kind.
    pub kind: CitationKind,
    /// The version the citation was stamped at.
    pub requirement_version: i32,
    /// The step that proposed it, if any.
    pub proposed_by_step_id: Option<StepId>,
    /// `requirement.version > requirement_version`, computed on read (plan D11).
    pub suspect: bool,
}

/// One live citation of a requirement, as
/// [`ReadStore::requirement_coverage`](crate::store::ReadStore::requirement_coverage) answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageRow {
    /// The citing item.
    pub item: ItemSummary,
    /// The citation kind.
    pub kind: CitationKind,
    /// `item.resolution` (the summary carries the status).
    pub resolution: Option<Resolution>,
    /// The version the citation was stamped at.
    pub requirement_version: i32,
    /// Plan D11, as [`ItemCitation::suspect`].
    pub suspect: bool,
}

/// The requirement list filter (plan D14). Every field is a conjunct; `None` does not filter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequirementFilter {
    /// `requirement.area_code` in this list.
    pub area_codes: Option<Vec<String>>,
    /// `requirement.state` in this list.
    pub states: Option<Vec<RequirementState>>,
    /// `requirement.priority` in this list.
    pub priorities: Option<Vec<Priority>>,
    /// Literal substring of `key` or `body`, case-folded per backend as [`ItemFilter::text`] is,
    /// so a non-ASCII needle can match differently in `MemStore`, the mirror and Postgres.
    ///
    /// [`ItemFilter::text`]: crate::model::item::ItemFilter::text
    pub text: Option<String>,
}

/// Result of an amend or withdraw (plan D9): the edit landed, or someone else committed first.
#[derive(Debug, Clone, PartialEq)]
pub enum RequirementUpdate {
    /// The compare-and-set matched; this is the new head.
    Updated(Requirement),
    /// The compare-and-set found a different version.
    Diverged {
        /// The row as it is now.
        head: Requirement,
        /// The revision at the version the caller edited from.
        ancestor: RequirementRevision,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::ItemKindId;

    fn requirement(key: &str) -> Requirement {
        let (area_code, path) = key[2..].split_once('-').expect("a key has an area code");
        let nested = path.contains('.');
        Requirement {
            id: RequirementId::new(),
            project_id: ProjectId::new(),
            area_id: RequirementAreaId::new(),
            area_code: area_code.to_owned(),
            number: (!nested).then(|| path.parse().expect("a top-level number")),
            key: key.to_owned(),
            refines_id: None,
            child_number: nested.then(|| {
                path.rsplit('.')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .expect("a child number")
            }),
            body: String::new(),
            rationale: String::new(),
            priority: Priority::Must,
            state: RequirementState::Active,
            version: 1,
            created_by: UserId::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn row(
        kind: CitationKind,
        status: Status,
        resolution: Option<Resolution>,
        suspect: bool,
    ) -> CoverageRow {
        CoverageRow {
            item: ItemSummary {
                id: ItemId::new(),
                project_id: ProjectId::new(),
                kind_id: ItemKindId::new(),
                key: "MOD-1".to_owned(),
                key_prefix: "MOD".to_owned(),
                key_number: 1,
                title: String::new(),
                status,
                priority: 0,
                required_tags: Vec::new(),
                updated_at: Utc::now(),
                touched_paths: Vec::new(),
                latest_run_status: None,
                latest_run_landing: None,
            },
            kind,
            resolution,
            requirement_version: 1,
            suspect,
        }
    }

    #[test]
    fn keys_follow_the_owner_and_nest() {
        assert_eq!(Requirement::top_level_key(false, "ENT", 3), "R-ENT-3");
        assert_eq!(Requirement::top_level_key(true, "ORCH", 1), "W-ORCH-1");
        assert_eq!(Requirement::child_key("W-ORCH-1", 2), "W-ORCH-1.2");
        assert_eq!(Requirement::child_key("R-ENT-1.1", 1), "R-ENT-1.1.1");
    }

    #[test]
    fn key_is_valid_mirrors_the_check() {
        for key in ["R-ENT-1", "W-A1-10.2.3", "R-E-1", "W-ORCH-1.1.1"] {
            assert!(Requirement::key_is_valid(key), "{key} is valid");
        }
        for key in [
            "X-ENT-1",
            "R-ent-1",
            "R-ENT-0",
            "R-ENT-1.0",
            "R-ENT-1.",
            "R-ENT-01",
            "R-1E-1",
            "R--1",
            "R-ENT-",
            "R-ENT",
            "R-ENT-1..2",
            "R-ENT-1a",
            "",
        ] {
            assert!(!Requirement::key_is_valid(key), "{key} is refused");
        }
    }

    #[test]
    fn list_cmp_orders_children_under_their_parent() {
        let sorted = ["R-ENT-1", "R-ENT-1.1", "R-ENT-1.2", "R-ENT-2", "R-ENT-10"];
        for pair in sorted.windows(2) {
            assert_eq!(
                requirement(pair[0]).list_cmp(&requirement(pair[1])),
                Ordering::Less,
                "{} < {}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(
            requirement("R-ENT-1.10").list_cmp(&requirement("R-ENT-1.9")),
            Ordering::Greater
        );
        assert_eq!(
            requirement("R-ENT-9").list_cmp(&requirement("R-STO-1")),
            Ordering::Less,
            "the area decides first"
        );
        assert_eq!(requirement("R-ENT-1.2").key_path(), vec![1, 2]);
        assert!(requirement("R-ENT-1.2").is_nested());
        assert!(!requirement("R-ENT-1").is_nested());
    }

    #[test]
    fn leaf_coverage_reads_addresses() {
        use CitationKind::{Addresses, Reserves};
        assert_eq!(CoverageState::of_citations(&[]), CoverageState::Open);
        assert_eq!(
            CoverageState::of_citations(&[row(Addresses, Status::Open, None, false)]),
            CoverageState::InProgress
        );
        assert_eq!(
            CoverageState::of_citations(&[row(
                Addresses,
                Status::Closed,
                Some(Resolution::Done),
                false
            )]),
            CoverageState::Covered
        );
        assert_eq!(
            CoverageState::of_citations(&[row(
                Addresses,
                Status::Closed,
                Some(Resolution::Concluded),
                false
            )]),
            CoverageState::Covered
        );
        assert_eq!(
            CoverageState::of_citations(&[row(
                Addresses,
                Status::Closed,
                Some(Resolution::Rejected),
                false
            )]),
            CoverageState::Open,
            "an item closed as rejected gave the work up"
        );
        assert_eq!(
            CoverageState::of_citations(&[row(Addresses, Status::Blocked, None, false)]),
            CoverageState::InProgress
        );
        assert_eq!(
            CoverageState::of_citations(&[row(
                Addresses,
                Status::Closed,
                Some(Resolution::Done),
                true
            )]),
            CoverageState::InProgress
        );
        assert_eq!(
            CoverageState::of_citations(&[row(
                Reserves,
                Status::Closed,
                Some(Resolution::Done),
                false
            )]),
            CoverageState::Open
        );
    }

    #[test]
    fn split_coverage_is_all_any_or_open() {
        use CoverageState::{Covered, InProgress, Open};
        assert_eq!(CoverageState::of_splits([]), None);
        assert_eq!(CoverageState::of_splits([Covered, Covered]), Some(Covered));
        assert_eq!(CoverageState::of_splits([Covered, Open]), Some(InProgress));
        assert_eq!(
            CoverageState::of_splits([InProgress, Open]),
            Some(InProgress)
        );
        assert_eq!(CoverageState::of_splits([Open, Open]), Some(Open));
    }
}
