//! MOD-11 D12, B-3: the concept index `search_concepts` queries, adapted from the binary's
//! `ConceptIndex` to `htui_mcp::search::ConceptSearch`.
//!
//! `htui-mcp` has no `htui-store` dependency, so it speaks its own plain types; [`McpSearch`]
//! maps them to `SearchQuery` and back from `Hit` by hand. The query is scoped to the one project
//! the tool names (the session's, I-1) and never asks for resolutions. [`production`] searches the
//! process-wide index the TUI's concepts runtime searches too ([`shared_index`], H-26).
//!
//! [`shared_index`]: crate::concepts_worker::shared_index

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use htui_mcp::search::{ConceptHit, ConceptQuery, ConceptSearch, ConceptType, OwnerKind};
use htui_store::vector::{Hit, Owner, PointType, SearchQuery};

use crate::concepts_worker::{ConceptIndex, shared_index};

/// `search_concepts`' index: a [`ConceptIndex`] seen as a `ConceptSearch`.
pub struct McpSearch(Arc<dyn ConceptIndex>);

impl McpSearch {
    /// The adapter over `index`.
    #[must_use]
    pub fn new(index: Arc<dyn ConceptIndex>) -> Self {
        Self(index)
    }
}

impl core::fmt::Debug for McpSearch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("McpSearch").finish_non_exhaustive()
    }
}

impl ConceptSearch for McpSearch {
    fn search(
        &self,
        query: ConceptQuery,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ConceptHit>, String>> + Send + '_>> {
        let query = SearchQuery {
            text: query.text,
            projects: vec![query.project],
            types: query.types.into_iter().map(point_type).collect(),
            statuses: query.statuses,
            resolutions: Vec::new(),
            limit: query.limit,
        };
        Box::pin(async move {
            let hits = self.0.search(query).await?;
            Ok(hits.into_iter().map(concept_hit).collect())
        })
    }
}

/// The index's name for a tool's type.
const fn point_type(t: ConceptType) -> PointType {
    match t {
        ConceptType::Item => PointType::Item,
        ConceptType::Document => PointType::Document,
        ConceptType::Requirement => PointType::Requirement,
    }
}

/// A hit as the tool returns it: field by field, nothing looked up.
fn concept_hit(hit: Hit) -> ConceptHit {
    ConceptHit {
        point_type: match hit.point_type {
            PointType::Item => ConceptType::Item,
            PointType::Document => ConceptType::Document,
            PointType::Requirement => ConceptType::Requirement,
        },
        owner_kind: match hit.owner {
            Owner::Item(_) => OwnerKind::Item,
            Owner::Requirement(_) => OwnerKind::Requirement,
        },
        key: hit.key,
        document_kind: hit.document.map(|(_, kind)| kind),
        resolution: hit.resolution.map(|r| r.as_str().to_owned()),
        state: hit.state.map(|s| s.as_str().to_owned()),
        score: hit.score,
        snippet: hit.snippet,
    }
}

/// The production concept search for `htui_mcp::McpHost::with_search`: an [`McpSearch`] over the
/// process-wide index. Always `Some` in this build; a box without Qdrant gets `search unavailable:
/// <why>` from the tool, the way the TUI's search overlay shows it (R-STO-8).
#[must_use]
pub fn production() -> Option<Arc<dyn ConceptSearch>> {
    Some(Arc::new(McpSearch::new(shared_index())))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{DateTime, Utc};
    use htui_core::model::{
        DocumentId, ItemId, ItemKindId, Priority, ProjectId, RequirementId, RequirementState,
        Resolution, Status,
    };
    use htui_mcp::search::{ConceptHit, ConceptQuery, ConceptSearch, ConceptType, OwnerKind};
    use htui_store::vector::{ConceptPoint, DocumentRef, Subject, VectorStore as _};

    use super::{McpSearch, production};
    use crate::concepts_worker::MemIndex;

    /// The words every seeded point carries, so one query finds them all.
    const WORDS: &str = "scoped concept search";

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-03T10:00:00Z")
            .expect("a timestamp")
            .with_timezone(&Utc)
    }

    /// An item point of `project`, closed as `resolution` when given.
    fn item(project: ProjectId, key: &str, resolution: Option<Resolution>) -> ConceptPoint {
        ConceptPoint {
            subject: Subject::Item {
                id: ItemId::new(),
                kind_id: ItemKindId::new(),
                status: if resolution.is_some() {
                    Status::Closed
                } else {
                    Status::Open
                },
                resolution,
            },
            key: key.to_owned(),
            project_id: project,
            updated_at: at(),
            document: None,
            text: format!("{key} {WORDS}"),
        }
    }

    /// A section of `kind` of a document of an open item of `project`.
    fn document(project: ProjectId, key: &str, kind: &str) -> ConceptPoint {
        ConceptPoint {
            document: Some(DocumentRef {
                id: DocumentId::new(),
                kind: kind.to_owned(),
                version: 1,
                chunk: 0,
            }),
            ..item(project, key, None)
        }
    }

    /// A requirement point of `project`.
    fn requirement(project: ProjectId, key: &str) -> ConceptPoint {
        ConceptPoint {
            subject: Subject::Requirement {
                id: RequirementId::new(),
                area_code: "STO".to_owned(),
                priority: Priority::Must,
                state: RequirementState::Active,
                version: 1,
            },
            key: key.to_owned(),
            project_id: project,
            updated_at: at(),
            document: None,
            text: format!("{key} {WORDS}"),
        }
    }

    async fn seeded(points: Vec<ConceptPoint>) -> McpSearch {
        let index = MemIndex::new();
        index
            .store()
            .upsert(points)
            .await
            .expect("the fake upserts");
        McpSearch::new(Arc::new(index))
    }

    fn query(project: ProjectId) -> ConceptQuery {
        ConceptQuery {
            text: WORDS.to_owned(),
            project,
            types: Vec::new(),
            statuses: Vec::new(),
            limit: 20,
        }
    }

    #[tokio::test]
    async fn a_hit_in_another_project_is_never_returned() {
        let (ours, theirs) = (ProjectId::new(), ProjectId::new());
        let search = seeded(vec![
            item(ours, "OURS-1", None),
            requirement(ours, "R-OURS-1"),
            item(theirs, "THEIRS-1", None),
            document(theirs, "THEIRS-1", "plan"),
            requirement(theirs, "R-THEIRS-1"),
        ])
        .await;

        let hits = search.search(query(ours)).await.expect("the fake answers");
        let mut keys: Vec<&str> = hits.iter().map(|hit| hit.key.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["OURS-1", "R-OURS-1"]);

        let hits = search
            .search(query(theirs))
            .await
            .expect("the fake answers");
        assert_eq!(hits.len(), 3);
        assert!(
            hits.iter().all(|hit| hit.key.contains("THEIRS")),
            "{hits:?}"
        );
    }

    #[tokio::test]
    async fn hits_map_every_field_by_hand() {
        let project = ProjectId::new();
        let search = seeded(vec![
            item(project, "HTUI-1", Some(Resolution::Done)),
            document(project, "HTUI-2", "plan"),
            requirement(project, "R-STO-8"),
        ])
        .await;

        let mut hits = search
            .search(query(project))
            .await
            .expect("the fake answers");
        hits.sort_by(|a, b| a.key.cmp(&b.key));
        let score = |key: &str| {
            hits.iter()
                .find(|hit| hit.key == key)
                .map(|hit| hit.score)
                .expect("a hit")
        };
        assert_eq!(
            hits,
            [
                ConceptHit {
                    point_type: ConceptType::Item,
                    owner_kind: OwnerKind::Item,
                    key: "HTUI-1".to_owned(),
                    document_kind: None,
                    resolution: Some("done".to_owned()),
                    state: None,
                    score: score("HTUI-1"),
                    snippet: format!("HTUI-1 {WORDS}"),
                },
                ConceptHit {
                    point_type: ConceptType::Document,
                    owner_kind: OwnerKind::Item,
                    key: "HTUI-2".to_owned(),
                    document_kind: Some("plan".to_owned()),
                    resolution: None,
                    state: None,
                    score: score("HTUI-2"),
                    snippet: format!("HTUI-2 {WORDS}"),
                },
                ConceptHit {
                    point_type: ConceptType::Requirement,
                    owner_kind: OwnerKind::Requirement,
                    key: "R-STO-8".to_owned(),
                    document_kind: None,
                    resolution: None,
                    state: Some("active".to_owned()),
                    score: score("R-STO-8"),
                    snippet: format!("R-STO-8 {WORDS}"),
                },
            ]
        );
        assert!(hits.iter().all(|hit| hit.score > 0.0), "{hits:?}");

        // The query's types, statuses and limit reach the index.
        let requirements = search
            .search(ConceptQuery {
                types: vec![ConceptType::Requirement],
                ..query(project)
            })
            .await
            .expect("the fake answers");
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].key, "R-STO-8");
        let closed = search
            .search(ConceptQuery {
                statuses: vec![Status::Closed],
                ..query(project)
            })
            .await
            .expect("the fake answers");
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].key, "HTUI-1");
        let one = search
            .search(ConceptQuery {
                limit: 1,
                ..query(project)
            })
            .await
            .expect("the fake answers");
        assert_eq!(one.len(), 1);
    }

    #[tokio::test]
    async fn an_index_failure_is_its_cause() {
        let search = McpSearch::new(Arc::new(MemIndex::new().failing("qdrant: query: refused")));
        assert_eq!(
            search.search(query(ProjectId::new())).await,
            Err("qdrant: query: refused".to_owned())
        );
    }

    #[test]
    fn production_offers_the_shared_index() {
        assert!(production().is_some(), "T7 attaches the concept index");
    }
}
