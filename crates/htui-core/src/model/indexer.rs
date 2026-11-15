//! The knowledge index's collections and the lease that names their one indexer (MOD-159 M1,
//! plan D1, D3; `0038_indexer_lease.sql`), what changed since a pass (MOD-159 M2, plan D2, D3),
//! and a pass request (MOD-159 M3, plan D2, D4).
//!
//! Each workspace has its own Qdrant collection and there is one global one
//! ([`IndexedCollection`]). Exactly one htui process indexes a collection: the holder of its
//! `indexer_lease` row. [`IndexerLease`] is that row as people see it (which box holds it, until
//! when) and never carries the holder's owner token, as `Run` keeps `lease_owner` out.
//! [`LeaseTake`] is what a take answers, and [`LeaseTurn`] what a take that asks a live holder
//! for a pass answers ([`take_or_request_pass`](crate::store::take_or_request_pass)).
//!
//! Between full reconciles the indexer runs incremental passes: [`IndexCursor`] is where the last
//! pass of a collection began, on the store's clock, and [`IndexChanges`] is what the store
//! answers when asked which rows were written since then
//! ([`IndexChangesStore`](crate::store::IndexChangesStore)).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::ids::{BoxId, DocumentId, ItemId, ProjectId, RequirementId, WorkspaceId};

/// The prefix of a workspace's collection name; the workspace's UUID follows it in simple form.
const WORKSPACE_PREFIX: &str = "htui_ws_";

/// The global collection's name.
const GLOBAL_NAME: &str = "htui_global";

/// One Qdrant collection the indexer fills (plan D1): a workspace's, named by the workspace's
/// UUID (never its slug, which can be renamed), or the global one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndexedCollection {
    /// `htui_ws_<uuid>`: the workspace's items, requirements and documents.
    Workspace(WorkspaceId),
    /// `htui_global`: what belongs to no workspace (empty until MOD-167/168).
    Global,
}

impl IndexedCollection {
    /// The Qdrant collection name, and the `indexer_lease.collection` key: `htui_ws_` and the
    /// workspace UUID in simple form (32 lowercase hex digits, no hyphens), or `htui_global`.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Workspace(workspace) => {
                format!("{WORKSPACE_PREFIX}{}", workspace.as_uuid().simple())
            }
            Self::Global => GLOBAL_NAME.to_owned(),
        }
    }

    /// The collection `name` names, the inverse of [`IndexedCollection::name`]: `None` for any
    /// other text, including a workspace UUID that is hyphenated or upper case (the table's
    /// `CHECK` refuses both).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        if name == GLOBAL_NAME {
            return Some(Self::Global);
        }
        let hex = name.strip_prefix(WORKSPACE_PREFIX)?;
        if hex.len() != 32
            || !hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        Uuid::try_parse(hex)
            .ok()
            .map(|uuid| Self::Workspace(WorkspaceId::from_uuid(uuid)))
    }
}

impl core::fmt::Display for IndexedCollection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.name())
    }
}

/// One collection's `indexer_lease` row as people see it (plan D3): the box that holds or last
/// held it, and until when. The holder's owner token is deliberately not a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerLease {
    /// `indexer_lease.collection`: an [`IndexedCollection::name`].
    pub collection: String,
    /// `indexer_lease.lease_box_id`: the holder's box. No foreign key, so the box may be gone.
    pub box_id: Option<BoxId>,
    /// The hostname of `box_id`'s row, `None` when there is no such box (display only).
    pub box_name: Option<String>,
    /// `indexer_lease.lease_expires_at`, on the store's clock.
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether someone holds it now: an owner is set and `expires_at` is after the store's now.
    pub live: bool,
}

/// What a take of an indexer lease answers (plan D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseTake {
    /// The lease was free, released, or lapsed under another owner, and is now the taker's.
    Taken,
    /// The taker holds it, or held it last and no one has taken it since, even if it lapsed; the
    /// expiry moved.
    Ours,
    /// Another owner holds it live; nothing was written.
    Held(IndexerLease),
}

/// What [`take_or_request_pass`](crate::store::take_or_request_pass) answers (MOD-159 M3 plan
/// D4): the lease is the caller's to index under, or a live holder was asked for a pass, or no
/// request could be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseTurn {
    /// The lease was free, released, lapsed or already the caller's, and is now the caller's
    /// ([`LeaseTake::Taken`] or [`LeaseTake::Ours`]).
    Ours,
    /// Another owner holds it live and its row now carries a pass request; the holder as
    /// `indexer_lease` shows it.
    Requested(IndexerLease),
    /// The holder the last take saw (possibly lapsed, so `live` may be `false`), and nothing was
    /// recorded (each request found it let go, and each take found it held again).
    Held(IndexerLease),
}

/// Where a collection's last completed pass began (MOD-159 M2 plan D2): the store-clock instant
/// read before that pass read the changes, kept in process. One field per change source; a later
/// source (MOD-112's transcripts) adds its own beside `rows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexCursor {
    /// The store's clock when the last pass began: items, requirements and documents written
    /// after it (less the overlap) are the next pass's.
    pub rows: DateTime<Utc>,
}

impl IndexCursor {
    /// A cursor whose rows source stands at `rows`.
    #[must_use]
    pub const fn at(rows: DateTime<Utc>) -> Self {
        Self { rows }
    }
}

/// The rows written strictly after an instant, by project (MOD-159 M2 plan D3): what
/// [`IndexChangesStore::index_changes`](crate::store::IndexChangesStore::index_changes) answers.
/// Only a project with at least one change has an entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexChanges {
    /// Each changed project's changes.
    pub projects: BTreeMap<ProjectId, ProjectChanges>,
}

impl IndexChanges {
    /// Whether nothing changed: no project has an entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

/// One project's changed rows (MOD-159 M2 plan D3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectChanges {
    /// Items of the project (its current one) whose `updated_at` is after the instant, or that
    /// gained a document version created after it.
    pub items: BTreeSet<ItemId>,
    /// Requirements of the project whose `updated_at` is after the instant.
    pub requirements: BTreeSet<RequirementId>,
    /// The project's owner documents (no item) created after the instant.
    pub owner_documents: BTreeSet<DocumentId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_changes_are_empty_until_a_project_has_an_entry() {
        assert!(IndexChanges::default().is_empty());
        let mut changes = IndexChanges::default();
        changes.projects.insert(
            ProjectId::new(),
            ProjectChanges {
                items: BTreeSet::from([ItemId::new()]),
                ..ProjectChanges::default()
            },
        );
        assert!(!changes.is_empty());
    }

    #[test]
    fn a_workspace_collection_is_named_by_its_uuid_in_simple_form() {
        let uuid = Uuid::parse_str("0192f3a4-5b6c-7d8e-9f01-23456789abcd").expect("uuid");
        let collection = IndexedCollection::Workspace(WorkspaceId::from_uuid(uuid));
        assert_eq!(
            collection.name(),
            "htui_ws_0192f3a45b6c7d8e9f0123456789abcd"
        );
        assert_eq!(collection.to_string(), collection.name());
        assert_eq!(IndexedCollection::Global.name(), "htui_global");
    }

    #[test]
    fn an_indexed_collection_name_parses_back_to_its_collection() {
        for collection in [
            IndexedCollection::Workspace(WorkspaceId::new()),
            IndexedCollection::Workspace(WorkspaceId::from_uuid(Uuid::nil())),
            IndexedCollection::Global,
        ] {
            assert_eq!(
                IndexedCollection::parse(&collection.name()),
                Some(collection),
                "{collection}"
            );
        }
    }

    #[test]
    fn text_that_names_no_indexed_collection_parses_to_none() {
        for garbage in [
            "",
            "htui_concepts_v2",
            "htui_global_",
            "htui_ws_",
            "htui_ws_0192f3a45b6c7d8e9f0123456789abc",
            "htui_ws_0192f3a45b6c7d8e9f0123456789abcde",
            "htui_ws_0192F3A45B6C7D8E9F0123456789ABCD",
            "htui_ws_0192f3a4-5b6c-7d8e-9f01-23456789abcd",
            "htui_ws_0192f3a45b6c7d8e9f0123456789abcg",
            "htui_ws_+192f3a45b6c7d8e9f0123456789abcd",
            "xhtui_global",
        ] {
            assert_eq!(IndexedCollection::parse(garbage), None, "`{garbage}`");
        }
    }
}
