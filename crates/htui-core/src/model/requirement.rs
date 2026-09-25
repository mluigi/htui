//! Requirements, their areas, the spec header and citations (`docs/ANA-11.md` §4, §5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{
    BoxId, ItemId, ProjectId, RequirementAreaId, RequirementId, StepId, UserId,
};
use crate::model::item::{ItemSummary, Resolution};
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
    /// `requirement.number`, per area, never reused.
    pub number: i32,
    /// `requirement.key`, generated: `R-<area_code>-<number>`.
    pub key: String,
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
