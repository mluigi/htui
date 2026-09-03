//! Links between items and the traversal projection the graph view renders (`docs/ANA-9.md`
//! §5.5, §7.3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{ItemId, ProjectId, StepId};
use crate::model::item::Status;

str_enum!(
    /// `item_link.kind` (§5.5): the edge is read `from --kind--> to`, so `blocked_by` means
    /// "`from` is blocked by `to`".
    LinkKind {
        /// `from` cannot start until `to` is terminal.
        BlockedBy => "blocked_by",
        /// `from` was spawned by `to`.
        Origin => "origin",
        /// Untyped relation.
        Relates => "relates",
        /// `from` supersedes `to`.
        Supersedes => "supersedes",
    }
);

/// A row of `item_link` (§5.5). Removal is a tombstone: a live edge has `deleted_at == None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemLink {
    /// `item_link.from_item_id`.
    pub from_item_id: ItemId,
    /// `item_link.to_item_id`.
    pub to_item_id: ItemId,
    /// `item_link.kind`.
    pub kind: LinkKind,
    /// `item_link.proposed_by_step_id`; `None` means a human or the importer created the edge.
    pub proposed_by_step_id: Option<StepId>,
    /// `item_link.created_at`.
    pub created_at: DateTime<Utc>,
    /// `item_link.updated_at`.
    pub updated_at: DateTime<Utc>,
    /// `item_link.deleted_at`: the tombstone the cache cursor rides on.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// One item reached by a traversal, with the hop distance from the root. Not a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkNode {
    /// `item.id`.
    pub item_id: ItemId,
    /// `item.project_id`; a traversal crosses projects (§5.5).
    pub project_id: ProjectId,
    /// `project.slug`, joined so the graph view can label a cross-project node.
    pub project_slug: String,
    /// `item.key`.
    pub key: String,
    /// `item.title`.
    pub title: String,
    /// `item.status`.
    pub status: Status,
    /// Hops from the root; the root itself is `0`.
    pub depth: u8,
}

/// One live edge inside a traversal. Not a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkEdge {
    /// `item_link.from_item_id`.
    pub from_item_id: ItemId,
    /// `item_link.to_item_id`.
    pub to_item_id: ItemId,
    /// `item_link.kind`.
    pub kind: LinkKind,
}

/// Result of [`crate::store::ReadStore::links`]: every item within `hops` of the root, and the
/// live edges between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkGraph {
    /// The item the traversal started from.
    pub root: ItemId,
    /// Reached items, including the root at depth `0`.
    pub nodes: Vec<LinkNode>,
    /// Edges between the reached items.
    pub edges: Vec<LinkEdge>,
}

impl LinkGraph {
    /// The node for `id`, if the traversal reached it.
    #[must_use]
    pub fn node(&self, id: ItemId) -> Option<&LinkNode> {
        self.nodes.iter().find(|node| node.item_id == id)
    }
}
