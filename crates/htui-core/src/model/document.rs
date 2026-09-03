//! Documents produced for an item (`docs/ANA-9.md` §5.5). Documents are append-only
//! (`R-ENT-12`): a new body is a new version.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{DocumentId, ItemId, StepId, UserId};

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
