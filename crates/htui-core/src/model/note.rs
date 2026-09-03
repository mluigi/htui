//! Item notes (`docs/ANA-9.md` §5.5). Notes are append-only (`R-ENT-11`), so the row carries
//! `created_at` alone and the cache cursor uses it in place of `updated_at`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{BoxId, ItemId, NoteId, StepId, UserId};

/// A row of `item_note` (§5.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// `item_note.id`.
    pub id: NoteId,
    /// `item_note.item_id`.
    pub item_id: ItemId,
    /// `item_note.body`.
    pub body: String,
    /// `item_note.created_by`.
    pub created_by: UserId,
    /// `item_note.box_id`.
    pub box_id: Option<BoxId>,
    /// `item_note.via_step_id`: set when the note was added through the MCP `note_add` tool.
    pub via_step_id: Option<StepId>,
    /// `item_note.created_at`.
    pub created_at: DateTime<Utc>,
}
