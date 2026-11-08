//! Documents produced for an item (`docs/ANA-9.md` §5.5). Documents are append-only
//! (`R-ENT-12`): a new body is a new version.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{DocumentId, ItemId, ProjectId, StepId, UserId};

/// A row of `document` (§5.5), body included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// `document.id`.
    pub id: DocumentId,
    /// `document.item_id`.
    pub item_id: ItemId,
    /// `document.kind`: an open set of phase output kinds plus `summary`.
    pub kind: String,
    /// `document.version`.
    pub version: i32,
    /// `document.title`.
    pub title: String,
    /// `document.body`.
    pub body: String,
    /// `document.produced_by_step_id`; `None` means written by hand.
    pub produced_by_step_id: Option<StepId>,
    /// `document.created_by`.
    pub created_by: UserId,
    /// `document.created_at`.
    pub created_at: DateTime<Utc>,
}

/// A `document` row a project — or a workspace's scope row — owns: `item_id IS NULL` (MOD-147 D2,
/// ANA-35 §7.5). Body included; versions are per `(project, kind)`. [`Document`] stays item-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerDocument {
    /// `document.id`.
    pub id: DocumentId,
    /// `document.project_id`: the owner.
    pub project_id: ProjectId,
    /// `document.kind`, e.g. `goals`.
    pub kind: String,
    /// `document.version`, per `(project, kind)` among the owner's documents.
    pub version: i32,
    /// `document.title`.
    pub title: String,
    /// `document.body`.
    pub body: String,
    /// `document.produced_by_step_id`; `None` for every row M1 writes.
    pub produced_by_step_id: Option<StepId>,
    /// `document.created_by`.
    pub created_by: UserId,
    /// `document.created_at`.
    pub created_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::write_owner_document`]: everything but `version`, which
/// the store allocates under the owner's row lock, and `produced_by_step_id`, which is always `NULL`
/// (a person writes an owner document, ANA-35 §7.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewOwnerDocument {
    /// `document.id`, minted client-side as a UUIDv7.
    pub id: DocumentId,
    /// `document.project_id`: the owner.
    pub project_id: ProjectId,
    /// `document.kind`.
    pub kind: String,
    /// `document.title`.
    pub title: String,
    /// `document.body`.
    pub body: String,
    /// `document.created_by`.
    pub created_by: UserId,
    /// `document.created_at`.
    pub created_at: DateTime<Utc>,
}

/// A `document` row without its body: what the Documents sub-tab lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentHead {
    /// `document.id`.
    pub id: DocumentId,
    /// `document.item_id`.
    pub item_id: ItemId,
    /// `document.kind`.
    pub kind: String,
    /// `document.version`.
    pub version: i32,
    /// `document.title`.
    pub title: String,
    /// `document.produced_by_step_id`.
    pub produced_by_step_id: Option<StepId>,
    /// `document.created_by`.
    pub created_by: UserId,
    /// `document.created_at`.
    pub created_at: DateTime<Utc>,
}

impl Document {
    /// This document without its body.
    #[must_use]
    pub fn head(&self) -> DocumentHead {
        DocumentHead {
            id: self.id,
            item_id: self.item_id,
            kind: self.kind.clone(),
            version: self.version,
            title: self.title.clone(),
            produced_by_step_id: self.produced_by_step_id,
            created_by: self.created_by,
            created_at: self.created_at,
        }
    }
}

/// Arguments of [`crate::store::WriteStore::write_document`]: everything but `version`, which the
/// store allocates under the item's row lock (ANA-2 §4.2, plan D6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewDocument {
    /// `document.id`.
    pub id: DocumentId,
    /// `document.item_id`.
    pub item_id: ItemId,
    /// `document.kind`.
    pub kind: String,
    /// `document.title`.
    pub title: String,
    /// `document.body`.
    pub body: String,
    /// `document.produced_by_step_id`; `None` means written by hand.
    pub produced_by_step_id: Option<StepId>,
    /// `document.created_by`.
    pub created_by: UserId,
    /// `document.created_at`.
    pub created_at: DateTime<Utc>,
}

/// One requested kind of [`crate::store::ReadStore::resolve_inputs`], present or missing.
///
/// A `Vec<Document>` could not report a kind the item has no eligible row for, and ANA-2 §4.2
/// needs that fact: a missing input kind fails the step (blueprint F-O).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedInput {
    /// The requested `document.kind`.
    pub kind: String,
    /// `None` = the item has no eligible document of this kind.
    pub document: Option<Document>,
}
