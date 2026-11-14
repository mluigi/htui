//! The knowledge index's collections and the lease that names their one indexer (MOD-159 M1,
//! plan D1, D3; `0036_indexer_lease.sql`).
//!
//! Each workspace has its own Qdrant collection and there is one global one
//! ([`IndexedCollection`]). Exactly one htui process indexes a collection: the holder of its
//! `indexer_lease` row. [`IndexerLease`] is that row as people see it (which box holds it, until
//! when) and never carries the holder's owner token, as `Run` keeps `lease_owner` out.
//! [`LeaseTake`] is what a take answers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::ids::{BoxId, WorkspaceId};

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

#[cfg(test)]
mod tests {
    use super::*;

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
