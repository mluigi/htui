//! The `search_concepts` seam (MOD-11 D12, blueprint B-3): what the tool asks of the concept
//! index, in this crate's own plain types.
//!
//! `htui-mcp` has no `htui-store` dependency (D1), so it cannot name `SearchQuery`, `Hit` or
//! `PointType`; the `htui` binary implements [`ConceptSearch`] over its index and maps those types
//! by hand. `None` for the host's search handle leaves the tool unadvertised.

use std::future::Future;
use std::pin::Pin;

use htui_core::model::{ProjectId, Status};

/// The concept index, as `search_concepts` sees it. Object-safe, `Send + Sync`.
pub trait ConceptSearch: Send + Sync + core::fmt::Debug {
    /// The hits for `query`, best first.
    ///
    /// # Errors
    ///
    /// A one-line cause (the index or the embedding model is unavailable); the tool answers
    /// `search unavailable: <cause>` (R-STO-8).
    fn search(
        &self,
        query: ConceptQuery,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ConceptHit>, String>> + Send + '_>>;
}

/// One search, scoped to the session's project by the tool (never by its arguments, I-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptQuery {
    /// The query text.
    pub text: String,
    /// The one project searched: the scope's.
    pub project: ProjectId,
    /// Point types to keep; empty keeps every type.
    pub types: Vec<ConceptType>,
    /// Item statuses to keep; empty keeps every status.
    pub statuses: Vec<Status>,
    /// At most this many hits.
    pub limit: u64,
}

/// What a hit's point is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConceptType {
    /// An item's title and description.
    Item,
    /// A document's body.
    Document,
    /// A requirement.
    Requirement,
}

impl ConceptType {
    /// Every type, in wire order.
    pub const ALL: [Self; 3] = [Self::Item, Self::Document, Self::Requirement];

    /// The wire name: `item`, `document` or `requirement`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Document => "document",
            Self::Requirement => "requirement",
        }
    }

    /// The type a wire name names, or `None`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// What owns a hit's point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    /// An item (its own point, or one of its documents).
    Item,
    /// A requirement.
    Requirement,
}

/// One hit, as the tool returns it: what the index carries, no title or status lookups.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ConceptHit {
    /// The point's type.
    pub point_type: ConceptType,
    /// The point's owner.
    pub owner_kind: OwnerKind,
    /// The owner's key (`HTUI-12`, `R-STO-8`).
    pub key: String,
    /// A document point's kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_kind: Option<String>,
    /// A closed item's resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// A requirement's state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// The similarity score.
    pub score: f32,
    /// The matched text, shortened.
    pub snippet: String,
}

#[cfg(test)]
mod tests {
    use super::ConceptType;

    #[test]
    fn a_concept_type_parses_its_own_wire_name_only() {
        for t in ConceptType::ALL {
            assert_eq!(ConceptType::parse(t.as_str()), Some(t));
            assert_eq!(
                serde_json::to_value(t).expect("serialises"),
                serde_json::json!(t.as_str())
            );
        }
        assert_eq!(ConceptType::parse("Item"), None);
        assert_eq!(ConceptType::parse("note"), None);
    }
}
