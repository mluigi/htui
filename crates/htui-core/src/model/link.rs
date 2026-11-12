//! Links between items and the traversal projection the graph view renders (`docs/ANA-9.md`
//! §5.5, §7.3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{ItemId, ProjectId, StepId};
use crate::model::item::{ItemState, Status};
use crate::model::run::RunStatus;

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
        /// `from` is a part of `to`, its parent (MOD-150, ANA-35 §7.6): at most one live parent
        /// per `from`, no cycles, a chain at most `MAX_PARENT_DEPTH` items deep. Walks upstream,
        /// never gates readiness.
        Parent => "parent",
        /// `from` is the same as `to`, its canonical item: at most one live canonical per `from`.
        Duplicates => "duplicates",
        /// `from` carries out `to`'s decision or specification.
        Implements => "implements",
    }
);

impl LinkKind {
    /// MOD-150 D6: whether the upstream prompt walk
    /// ([`ReadStore::upstream_summaries`](crate::store::ReadStore::upstream_summaries)) follows
    /// this kind from `from` to `to`: `blocked_by`, `origin` and `parent`. The SQL backends spell
    /// the same list literally (`pg/read.rs`, `cache/read.rs`).
    #[must_use]
    pub const fn walks_upstream(self) -> bool {
        matches!(self, Self::BlockedBy | Self::Origin | Self::Parent)
    }
}

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

/// Arguments of `WriteStore::propose_link` (MOD-11 D13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposeLink {
    /// `item_link.from_item_id`: the proposing step's own item.
    pub from: ItemId,
    /// `item_link.to_item_id`: an item within the run's reach (MOD-147 D3).
    pub to: ItemId,
    /// `item_link.kind`. `parent` and `duplicates` are single-valued, and a `parent` is checked
    /// for cycles and depth (MOD-150 D4).
    pub kind: LinkKind,
    /// The proposing step, written to `item_link.proposed_by_step_id`.
    pub step: StepId,
}

/// Arguments of `WriteStore::withdraw_link` (MOD-11 D13, B-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawLink {
    /// `item_link.from_item_id`: the withdrawing step's own item.
    pub from: ItemId,
    /// `item_link.to_item_id`.
    pub to: ItemId,
    /// `item_link.kind`.
    pub kind: LinkKind,
    /// The withdrawing step; the link must have been proposed by a step of its run.
    pub step: StepId,
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
    /// MOD-144: the status of the item's latest non-cancelled graph run; `None` without one.
    /// Appended, never inserted.
    #[serde(default)]
    pub latest_run_status: Option<RunStatus>,
}

impl LinkNode {
    /// The item's derived display state (MOD-144).
    #[must_use]
    pub const fn state(&self) -> ItemState {
        ItemState::of(self.status, self.latest_run_status)
    }
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

/// One upstream item of the prompt's upstream section (`docs/ANA-5.md` §4.3, amending §7.3): an
/// item reached from the step's item over `blocked_by`, `origin` and `parent` edges (MOD-150 D6),
/// at its **minimum** depth, classified against the active scope. Not a table.
///
/// `in_scope` and `summary` are separate facts on purpose. "Outside the active workspace" is the
/// `R-PRM-2` stub and "in the workspace but nobody has written a summary yet" is a state the agent
/// can act on, so collapsing the two into one `NULL` would lose the difference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamEntry {
    /// `item.id`.
    pub item_id: ItemId,
    /// `project.slug` + `:` + `item.key`, the key the render and the sort both use.
    pub qualified_key: String,
    /// `item.title`, raw; the render collapses its whitespace and truncates it.
    pub title: String,
    /// `item.status`, the stored status (MOD-144: `open`, `blocked` or `closed`).
    pub status: Status,
    /// Hops from the step's item, `1` or `2`, minimised over every path (the diamond dedup).
    pub depth: u8,
    /// Whether the item's project is inside the scope; `false` is the `R-PRM-2` stub.
    pub in_scope: bool,
    /// The latest `document.kind = 'summary'` body, `Some` only when the item is in scope.
    pub summary: Option<String>,
    /// MOD-144: the status of the item's latest non-cancelled graph run; `None` without one.
    /// Appended, never inserted.
    #[serde(default)]
    pub latest_run_status: Option<RunStatus>,
}

impl UpstreamEntry {
    /// The item's derived display state (MOD-144): what the prompt's upstream section prints.
    #[must_use]
    pub const fn state(&self) -> ItemState {
        ItemState::of(self.status, self.latest_run_status)
    }

    /// `docs/ANA-5.md` §4.7 rule 2: sort by `(depth, qualified_key bytes, item_id)`.
    ///
    /// `str`'s `Ord` is byte order in Rust, which is the point: Postgres orders text by the
    /// database collation while the SQLite mirror orders by byte value, so this is the one sort
    /// every backend runs after its own `ORDER BY`. Without it the same item on the same box would
    /// digest differently online and offline.
    pub fn sort_canonical(entries: &mut [Self]) {
        entries.sort_by(|a, b| {
            a.depth.cmp(&b.depth).then_with(|| {
                a.qualified_key
                    .as_bytes()
                    .cmp(b.qualified_key.as_bytes())
                    .then_with(|| a.item_id.cmp(&b.item_id))
            })
        });
    }

    /// Whether this entry renders as a summary block: in scope, with a summary document.
    #[must_use]
    pub const fn is_summary(&self) -> bool {
        self.in_scope && self.summary.is_some()
    }

    /// Whether this entry renders as the `- no summary yet` one-liner: in scope, but nobody has
    /// written its summary. An out-of-scope entry is neither — it is the `R-PRM-2` stub.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.in_scope && self.summary.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn entry(key: &str, depth: u8, id: u128) -> UpstreamEntry {
        UpstreamEntry {
            item_id: ItemId::from_uuid(Uuid::from_u128(id)),
            qualified_key: key.to_owned(),
            title: format!("{key} title"),
            status: Status::Open,
            depth,
            in_scope: true,
            summary: Some(format!("{key} summary")),
            latest_run_status: Some(RunStatus::Done),
        }
    }

    fn keys(entries: &[UpstreamEntry]) -> Vec<(&str, u8, Uuid)> {
        entries
            .iter()
            .map(|e| (e.qualified_key.as_str(), e.depth, e.item_id.as_uuid()))
            .collect()
    }

    /// §4.7 rule 2: `(depth, qualified_key bytes, item_id)`, in that order. Depth outranks the
    /// key, and the id breaks a key tie so two items sharing a key can never swap places.
    #[test]
    fn sort_canonical_is_depth_then_key_bytes_then_id() {
        let mut entries = vec![
            entry("htui:MOD-7", 2, 9),
            entry("auth:MOD-1", 1, 4),
            entry("htui:ANA-9", 1, 7),
            entry("auth:MOD-1", 1, 2),
            entry("auth:MOD-1", 2, 1),
        ];

        UpstreamEntry::sort_canonical(&mut entries);

        assert_eq!(
            keys(&entries),
            vec![
                ("auth:MOD-1", 1, Uuid::from_u128(2)),
                ("auth:MOD-1", 1, Uuid::from_u128(4)),
                ("htui:ANA-9", 1, Uuid::from_u128(7)),
                ("auth:MOD-1", 2, Uuid::from_u128(1)),
                ("htui:MOD-7", 2, Uuid::from_u128(9)),
            ]
        );
    }

    /// Byte order, not collation order: Postgres orders text by the database collation and the
    /// SQLite mirror by byte value, so without this re-sort the same graph would digest
    /// differently online and offline (ANA-5 §4.3 `:675-681`). An ICU collation reads these as
    /// `a:x`, `Å:x`, `Z:x`.
    #[test]
    fn a_non_ascii_key_sorts_by_bytes() {
        let mut entries = vec![
            entry("a:x", 1, 1),
            entry("Ångström:x", 1, 2),
            entry("Z:x", 1, 3),
        ];

        UpstreamEntry::sort_canonical(&mut entries);

        assert_eq!(
            entries
                .iter()
                .map(|e| e.qualified_key.as_str())
                .collect::<Vec<_>>(),
            vec!["Z:x", "a:x", "Ångström:x"]
        );
    }

    /// MOD-150 D6: the upstream walk follows exactly `blocked_by`, `origin` and `parent`, in
    /// `LinkKind::ALL`'s order.
    #[test]
    fn walks_upstream_is_blocked_by_origin_and_parent() {
        let walked: Vec<LinkKind> = LinkKind::ALL
            .iter()
            .copied()
            .filter(|kind| kind.walks_upstream())
            .collect();
        assert_eq!(
            walked,
            vec![LinkKind::BlockedBy, LinkKind::Origin, LinkKind::Parent]
        );
    }

    /// ANA-5 §4.3 step 5's three render states. "Outside the workspace" and "nobody has written a
    /// summary yet" are separable facts, which is the whole reason `in_scope` is a column of its
    /// own rather than a `NULL` summary.
    #[test]
    fn classification_separates_scope_from_summary() {
        let summary = entry("htui:ANA-9", 1, 1);
        assert!(summary.is_summary());
        assert!(!summary.is_pending());

        let pending = UpstreamEntry {
            summary: None,
            ..entry("htui:MOD-7", 1, 2)
        };
        assert!(!pending.is_summary());
        assert!(pending.is_pending());

        let stub = UpstreamEntry {
            in_scope: false,
            ..entry("auth:MOD-1", 1, 3)
        };
        assert!(!stub.is_summary(), "out of scope is never a summary block");
        assert!(!stub.is_pending(), "out of scope is a stub, not a pending");
    }
}
