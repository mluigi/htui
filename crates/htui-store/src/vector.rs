//! The concepts index: items and their documents in Qdrant, searched by meaning and by exact
//! term at once (MOD-34, `R-STO-8`, `docs/ANA-19.md` §3.3, `docs/ANA-20.md` §3).
//!
//! One collection holds every project (ANA-20 §3.4); the tenant is `project_id`, and a point's
//! `type` says what it indexes. Postgres stays the source of truth (`R-STO-1`): every point can be
//! rebuilt from an item or a document row, and [`crate::vector_sync::Indexer`] is what keeps the
//! two in step. Qdrant errors are [`StoreError::Backend`], never [`StoreError::Unreachable`], so
//! nothing that reads the latter as "Postgres went away" can mistake a down vector store for it.
use crate::bm25::{self, SparseVector};
use crate::embed::DenseEmbedder;
use crate::qdrant_settings::QdrantSettings;
use chrono::{DateTime, Utc};
use htui_core::model::{DocumentId, ItemId, ItemKindId, ProjectId, Status};
use htui_core::store::StoreError;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, DeletePointsBuilder,
    Distance, FieldType, Filter, Fusion, Modifier, NamedVectors, PointId, PointStruct,
    PrefetchQueryBuilder, Query, QueryPointsBuilder, ScrollPointsBuilder,
    SparseVectorParamsBuilder, SparseVectorsConfigBuilder, UpsertPointsBuilder, Value,
    VectorParamsBuilder, VectorsConfigBuilder, point_id::PointIdOptions,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;

/// The collection name. Versioned: a change to the vector layout or the payload gets a new name
/// rather than a migration, since the whole index can be rebuilt from Postgres.
pub const COLLECTION: &str = "htui_concepts_v1";
/// Name of the dense vector.
pub const DENSE: &str = "dense";
/// Name of the sparse (BM25) vector.
pub const SPARSE: &str = "sparse";
/// Longest snippet a point carries in its payload, in characters.
pub const SNIPPET_CHARS: usize = 280;

// Payload keys.
const TYPE: &str = "type";
const ITEM_ID: &str = "item_id";
const KEY: &str = "key";
const PROJECT_ID: &str = "project_id";
const KIND_ID: &str = "kind_id";
const STATUS: &str = "status";
const UPDATED_AT: &str = "updated_at";
const DOCUMENT_ID: &str = "document_id";
const DOC_KIND: &str = "doc_kind";
const DOC_VERSION: &str = "doc_version";
const CHUNK: &str = "chunk";
const SNIPPET: &str = "snippet";

/// What a point indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointType {
    /// An item's key, title and body.
    Item,
    /// One section of an item's latest document of some kind.
    Document,
}

impl PointType {
    /// The payload text of this type. `requirement` is reserved for MOD-38's table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Document => "document",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "item" => Some(Self::Item),
            "document" => Some(Self::Document),
            _ => None,
        }
    }
}

/// Where a document point came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRef {
    /// `document.id`.
    pub id: DocumentId,
    /// `document.kind`, e.g. `summary`.
    pub kind: String,
    /// `document.version`.
    pub version: i32,
    /// Section number within the document, from 0.
    pub chunk: u32,
}

/// One point to write: the text to embed plus what the payload says about it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConceptPoint {
    /// Item or document section.
    pub point_type: PointType,
    /// The item, or the item the document belongs to.
    pub item_id: ItemId,
    /// The item's key, e.g. `MOD-34`.
    pub key: String,
    /// The item's project.
    pub project_id: ProjectId,
    /// The item's kind.
    pub kind_id: ItemKindId,
    /// The item's status, so a search can ask for closed items (decisions) only.
    pub status: Status,
    /// The item's `updated_at` when this point was built.
    pub updated_at: DateTime<Utc>,
    /// Set on document points.
    pub document: Option<DocumentRef>,
    /// The text embedded, dense and sparse.
    pub text: String,
}

impl ConceptPoint {
    /// The point's ID: the item's UUID for an item point; for a document section, a UUID (version
    /// 8) derived from SHA-256 of the document ID and section number. Both stable forever.
    #[must_use]
    pub fn id(&self) -> Uuid {
        match &self.document {
            None => self.item_id.0,
            Some(doc) => document_point_id(doc.id, doc.chunk),
        }
    }
}

/// The ID of section `chunk` of document `id`.
#[must_use]
pub fn document_point_id(id: DocumentId, chunk: u32) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"htui-concepts/document\0");
    hasher.update(id.0.as_bytes());
    hasher.update(chunk.to_le_bytes());
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("16 bytes");
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

/// What the index holds for one point, as the indexer needs it to decide what changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPoint {
    /// Point ID.
    pub id: Uuid,
    /// Item or document section.
    pub point_type: PointType,
    /// The item it belongs to.
    pub item_id: ItemId,
    /// The item's `updated_at` when the point was built.
    pub updated_at: DateTime<Utc>,
    /// The item's status when the point was built. Compared on its own because a status move
    /// (`WriteStore::transition`) does not touch `updated_at`.
    pub status: Status,
    /// Set on document points.
    pub document_id: Option<DocumentId>,
}

/// A search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free text; exact keys such as `MOD-34` match through the sparse vector.
    pub text: String,
    /// Projects to search. Empty finds nothing: a search is always scoped.
    pub projects: Vec<ProjectId>,
    /// Point types to return; empty means all.
    pub types: Vec<PointType>,
    /// Item statuses to return; empty means all. `[Closed]` asks for decisions.
    pub statuses: Vec<Status>,
    /// Most hits returned.
    pub limit: u64,
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Item or document section.
    pub point_type: PointType,
    /// The item.
    pub item_id: ItemId,
    /// The item's key.
    pub key: String,
    /// The document and its kind, on document hits.
    pub document: Option<(DocumentId, String)>,
    /// Fused rank score; higher is better, comparable only within one search.
    pub score: f32,
    /// The start of the indexed text.
    pub snippet: String,
}

/// The index seam. [`QdrantStore`] is the production one; `MemVectorStore` (test-support) is a
/// fake with the same bookkeeping and a naive search.
#[allow(async_fn_in_trait)]
pub trait VectorStore {
    /// Writes points, replacing any with the same ID.
    async fn upsert(&self, points: Vec<ConceptPoint>) -> Result<(), StoreError>;
    /// Removes points by ID; unknown IDs are ignored.
    async fn delete(&self, ids: Vec<Uuid>) -> Result<(), StoreError>;
    /// Every point of one project.
    async fn indexed(&self, project: ProjectId) -> Result<Vec<IndexedPoint>, StoreError>;
    /// Hybrid search.
    async fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, StoreError>;
}

fn backend(context: &str, e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(format!("qdrant: {context}: {e}"))
}

/// The start of `text`, whitespace folded and control characters dropped: item and document
/// bodies are often agent-written, and a snippet ends up printed to a terminal.
fn snippet(text: &str) -> String {
    let flat: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    match flat.char_indices().nth(SNIPPET_CHARS) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}

/// The payload of a point, as Qdrant stores it.
fn payload(point: &ConceptPoint) -> HashMap<String, Value> {
    let mut p = HashMap::from([
        (TYPE.to_owned(), Value::from(point.point_type.as_str())),
        (ITEM_ID.to_owned(), Value::from(point.item_id.0.to_string())),
        (KEY.to_owned(), Value::from(point.key.clone())),
        (
            PROJECT_ID.to_owned(),
            Value::from(point.project_id.0.to_string()),
        ),
        (KIND_ID.to_owned(), Value::from(point.kind_id.0.to_string())),
        (STATUS.to_owned(), Value::from(point.status.as_str())),
        (
            UPDATED_AT.to_owned(),
            Value::from(point.updated_at.to_rfc3339()),
        ),
        (SNIPPET.to_owned(), Value::from(snippet(&point.text))),
    ]);
    if let Some(doc) = &point.document {
        p.insert(DOCUMENT_ID.to_owned(), Value::from(doc.id.0.to_string()));
        p.insert(DOC_KIND.to_owned(), Value::from(doc.kind.clone()));
        p.insert(DOC_VERSION.to_owned(), Value::from(i64::from(doc.version)));
        p.insert(CHUNK.to_owned(), Value::from(i64::from(doc.chunk)));
    }
    p
}

fn text_of<'a>(payload: &'a HashMap<String, Value>, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(Value::as_str).map(String::as_str)
}

fn uuid_of(payload: &HashMap<String, Value>, key: &str) -> Option<Uuid> {
    text_of(payload, key).and_then(|s| Uuid::from_str(s).ok())
}

fn point_uuid(id: Option<&PointId>) -> Option<Uuid> {
    match id?.point_id_options.as_ref()? {
        PointIdOptions::Uuid(s) => Uuid::from_str(s).ok(),
        PointIdOptions::Num(_) => None,
    }
}

fn indexed_point(id: Option<&PointId>, payload: &HashMap<String, Value>) -> Option<IndexedPoint> {
    Some(IndexedPoint {
        id: point_uuid(id)?,
        point_type: PointType::parse(text_of(payload, TYPE)?)?,
        item_id: ItemId(uuid_of(payload, ITEM_ID)?),
        updated_at: DateTime::parse_from_rfc3339(text_of(payload, UPDATED_AT)?)
            .ok()?
            .with_timezone(&Utc),
        status: Status::from_str(text_of(payload, STATUS)?).ok()?,
        document_id: uuid_of(payload, DOCUMENT_ID).map(DocumentId),
    })
}

fn hit(payload: &HashMap<String, Value>, score: f32) -> Option<Hit> {
    let document = match (uuid_of(payload, DOCUMENT_ID), text_of(payload, DOC_KIND)) {
        (Some(id), Some(kind)) => Some((DocumentId(id), kind.to_owned())),
        _ => None,
    };
    Some(Hit {
        point_type: PointType::parse(text_of(payload, TYPE)?)?,
        item_id: ItemId(uuid_of(payload, ITEM_ID)?),
        key: text_of(payload, KEY)?.to_owned(),
        document,
        score,
        snippet: text_of(payload, SNIPPET).unwrap_or_default().to_owned(),
    })
}

/// The filter a search applies: its projects, and its types and statuses when given.
fn search_filter(query: &SearchQuery) -> Filter {
    let mut must = vec![Condition::matches(
        PROJECT_ID,
        query
            .projects
            .iter()
            .map(|p| p.0.to_string())
            .collect::<Vec<_>>(),
    )];
    if !query.types.is_empty() {
        must.push(Condition::matches(
            TYPE,
            query
                .types
                .iter()
                .map(|t| t.as_str().to_owned())
                .collect::<Vec<_>>(),
        ));
    }
    if !query.statuses.is_empty() {
        must.push(Condition::matches(
            STATUS,
            query
                .statuses
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect::<Vec<_>>(),
        ));
    }
    Filter::must(must)
}

fn sparse_pairs(v: &SparseVector) -> Vec<(u32, f32)> {
    v.indices
        .iter()
        .copied()
        .zip(v.values.iter().copied())
        .collect()
}

/// The Qdrant-backed index.
pub struct QdrantStore<E> {
    client: Qdrant,
    embedder: E,
    collection: String,
}

impl<E> std::fmt::Debug for QdrantStore<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantStore")
            .field("collection", &self.collection)
            .finish_non_exhaustive()
    }
}

impl<E: DenseEmbedder> QdrantStore<E> {
    /// Connects to [`COLLECTION`], creating it and its payload indexes on first use.
    pub async fn connect(settings: &QdrantSettings, embedder: E) -> Result<Self, StoreError> {
        Self::connect_to(settings, embedder, COLLECTION).await
    }

    /// As [`connect`](Self::connect), to a named collection (tests use a throwaway one).
    pub async fn connect_to(
        settings: &QdrantSettings,
        embedder: E,
        collection: &str,
    ) -> Result<Self, StoreError> {
        // No compatibility check: it prints to stdout (which `--search-items` owns) and blocks on
        // its own health probe; `ensure_collection` fails fast on a bad server anyway.
        let mut builder = Qdrant::from_url(&settings.url).skip_compatibility_check();
        if let Some(key) = &settings.api_key {
            builder = builder.api_key(key.as_str());
        }
        let client = builder.build().map_err(|e| backend("client", e))?;
        let store = Self {
            client,
            embedder,
            collection: collection.to_owned(),
        };
        store.ensure_collection().await?;
        Ok(store)
    }

    /// Drops the collection. For tests and for a deliberate rebuild.
    pub async fn drop_collection(&self) -> Result<(), StoreError> {
        self.client
            .delete_collection(&self.collection)
            .await
            .map_err(|e| backend("delete collection", e))?;
        Ok(())
    }

    async fn ensure_collection(&self) -> Result<(), StoreError> {
        let exists = self
            .client
            .collection_exists(&self.collection)
            .await
            .map_err(|e| backend("collection exists", e))?;
        if !exists {
            self.create_collection().await?;
        }
        // Every time, not only on creation: re-creating an existing index is a no-op in Qdrant, and
        // a first run that died between the collection and its indexes is repaired here.
        for field in [TYPE, PROJECT_ID, STATUS, ITEM_ID] {
            self.client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(
                        &self.collection,
                        field,
                        FieldType::Keyword,
                    )
                    .wait(true),
                )
                .await
                .map_err(|e| backend("create payload index", e))?;
        }
        Ok(())
    }

    async fn create_collection(&self) -> Result<(), StoreError> {
        let mut dense = VectorsConfigBuilder::default();
        dense.add_named_vector_params(
            DENSE,
            VectorParamsBuilder::new(self.embedder.dim() as u64, Distance::Cosine),
        );
        let mut sparse = SparseVectorsConfigBuilder::default();
        sparse.add_named_vector_params(
            SPARSE,
            SparseVectorParamsBuilder::default().modifier(Modifier::Idf),
        );
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection)
                    .vectors_config(dense)
                    .sparse_vectors_config(sparse),
            )
            .await
            .map_err(|e| backend("create collection", e))?;
        Ok(())
    }
}

impl<E: DenseEmbedder> VectorStore for QdrantStore<E> {
    async fn upsert(&self, points: Vec<ConceptPoint>) -> Result<(), StoreError> {
        if points.is_empty() {
            return Ok(());
        }
        let texts = points.iter().map(|p| p.text.clone()).collect();
        let dense = self.embedder.embed(texts).await?;
        let structs: Vec<PointStruct> = points
            .iter()
            .zip(dense)
            .map(|(point, dense)| {
                let sparse = bm25::document_vector(&point.text);
                let vectors = NamedVectors::default().add_vector(DENSE, dense).add_vector(
                    SPARSE,
                    qdrant_client::qdrant::Vector::from(sparse_pairs(&sparse)),
                );
                PointStruct::new(point.id().to_string(), vectors, payload(point))
            })
            .collect();
        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection, structs).wait(true))
            .await
            .map_err(|e| backend("upsert", e))?;
        Ok(())
    }

    async fn delete(&self, ids: Vec<Uuid>) -> Result<(), StoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<PointId> = ids.iter().map(|id| PointId::from(id.to_string())).collect();
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
                    .points(ids)
                    .wait(true),
            )
            .await
            .map_err(|e| backend("delete", e))?;
        Ok(())
    }

    async fn indexed(&self, project: ProjectId) -> Result<Vec<IndexedPoint>, StoreError> {
        let filter = Filter::must([Condition::matches(PROJECT_ID, project.0.to_string())]);
        let mut out = Vec::new();
        let mut offset: Option<PointId> = None;
        loop {
            let mut request = ScrollPointsBuilder::new(&self.collection)
                .filter(filter.clone())
                .limit(1024)
                .with_payload(true)
                .with_vectors(false);
            if let Some(o) = offset.take() {
                request = request.offset(o);
            }
            let page = self
                .client
                .scroll(request)
                .await
                .map_err(|e| backend("scroll", e))?;
            out.extend(
                page.result
                    .iter()
                    .filter_map(|p| indexed_point(p.id.as_ref(), &p.payload)),
            );
            match page.next_page_offset {
                Some(next) => offset = Some(next),
                None => break,
            }
        }
        Ok(out)
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, StoreError> {
        if query.projects.is_empty() || query.limit == 0 {
            return Ok(Vec::new());
        }
        let dense = self
            .embedder
            .embed(vec![query.text.clone()])
            .await?
            .pop()
            .ok_or_else(|| backend("embed", "no vector for the query"))?;
        let sparse = sparse_pairs(&bm25::query_vector(&query.text));
        let filter = search_filter(query);
        // Each arm fetches more than the final limit so the fusion has overlap to work with.
        let depth = query.limit.saturating_mul(4).max(20);
        let mut request = QueryPointsBuilder::new(&self.collection)
            .add_prefetch(
                PrefetchQueryBuilder::default()
                    .query(Query::new_nearest(dense))
                    .using(DENSE)
                    .filter(filter.clone())
                    .limit(depth),
            )
            .query(Query::new_fusion(Fusion::Rrf))
            .filter(filter.clone())
            .limit(query.limit)
            .with_payload(true);
        if !sparse.is_empty() {
            request = request.add_prefetch(
                PrefetchQueryBuilder::default()
                    .query(Query::from(sparse))
                    .using(SPARSE)
                    .filter(filter)
                    .limit(depth),
            );
        }
        let response = self
            .client
            .query(request)
            .await
            .map_err(|e| backend("query", e))?;
        Ok(response
            .result
            .iter()
            .filter_map(|p| hit(&p.payload, p.score))
            .collect())
    }
}

/// An in-memory [`VectorStore`] for tests. Search ranks by how many query terms a point's text
/// shares, which is enough to prove scoping and filtering, not relevance.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct MemVectorStore {
    points: std::sync::Mutex<std::collections::BTreeMap<Uuid, ConceptPoint>>,
    upserts: std::sync::atomic::AtomicUsize,
}

#[cfg(any(test, feature = "test-support"))]
impl MemVectorStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every point, by ID.
    #[must_use]
    pub fn points(&self) -> Vec<ConceptPoint> {
        self.points
            .lock()
            .expect("poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Points written since creation, counting rewrites.
    #[must_use]
    pub fn upserted(&self) -> usize {
        self.upserts.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl VectorStore for MemVectorStore {
    async fn upsert(&self, points: Vec<ConceptPoint>) -> Result<(), StoreError> {
        self.upserts
            .fetch_add(points.len(), std::sync::atomic::Ordering::SeqCst);
        let mut map = self.points.lock().expect("poisoned");
        for p in points {
            map.insert(p.id(), p);
        }
        Ok(())
    }

    async fn delete(&self, ids: Vec<Uuid>) -> Result<(), StoreError> {
        let mut map = self.points.lock().expect("poisoned");
        for id in ids {
            map.remove(&id);
        }
        Ok(())
    }

    async fn indexed(&self, project: ProjectId) -> Result<Vec<IndexedPoint>, StoreError> {
        Ok(self
            .points
            .lock()
            .expect("poisoned")
            .values()
            .filter(|p| p.project_id == project)
            .map(|p| IndexedPoint {
                id: p.id(),
                point_type: p.point_type,
                item_id: p.item_id,
                updated_at: p.updated_at,
                status: p.status,
                document_id: p.document.as_ref().map(|d| d.id),
            })
            .collect())
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, StoreError> {
        let terms = bm25::tokenize(&query.text);
        let mut hits: Vec<Hit> = self
            .points
            .lock()
            .expect("poisoned")
            .values()
            .filter(|p| query.projects.contains(&p.project_id))
            .filter(|p| query.types.is_empty() || query.types.contains(&p.point_type))
            .filter(|p| query.statuses.is_empty() || query.statuses.contains(&p.status))
            .filter_map(|p| {
                let own = bm25::tokenize(&p.text);
                #[allow(clippy::cast_precision_loss)]
                let score = terms.iter().filter(|t| own.contains(t)).count() as f32;
                (score > 0.0).then(|| Hit {
                    point_type: p.point_type,
                    item_id: p.item_id,
                    key: p.key.clone(),
                    document: p.document.as_ref().map(|d| (d.id, d.kind.clone())),
                    score,
                    snippet: snippet(&p.text),
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(usize::try_from(query.limit).unwrap_or(usize::MAX));
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(document: Option<DocumentRef>) -> ConceptPoint {
        ConceptPoint {
            point_type: if document.is_some() {
                PointType::Document
            } else {
                PointType::Item
            },
            item_id: ItemId(Uuid::from_u128(1)),
            key: "MOD-34".into(),
            project_id: ProjectId(Uuid::from_u128(2)),
            kind_id: ItemKindId(Uuid::from_u128(3)),
            status: Status::Closed,
            updated_at: DateTime::parse_from_rfc3339("2026-09-25T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            document,
            text: "MOD-34 Qdrant related concepts search".into(),
        }
    }

    fn doc(chunk: u32) -> DocumentRef {
        DocumentRef {
            id: DocumentId(Uuid::from_u128(4)),
            kind: "summary".into(),
            version: 2,
            chunk,
        }
    }

    #[test]
    fn item_points_use_the_item_uuid() {
        assert_eq!(point(None).id(), Uuid::from_u128(1));
    }

    #[test]
    fn document_point_ids_are_stable_and_distinct() {
        let a = point(Some(doc(0))).id();
        assert_eq!(a, point(Some(doc(0))).id());
        assert_ne!(a, point(Some(doc(1))).id());
        assert_eq!(a.get_version_num(), 8);
        // Golden: a change here orphans every document point already indexed.
        assert_eq!(a, document_point_id(DocumentId(Uuid::from_u128(4)), 0));
    }

    #[test]
    fn payload_round_trips_to_an_indexed_point_and_a_hit() {
        let p = point(Some(doc(3)));
        let payload = payload(&p);
        assert_eq!(text_of(&payload, TYPE), Some("document"));
        assert_eq!(text_of(&payload, STATUS), Some("closed"));
        assert_eq!(payload.get(CHUNK).and_then(Value::as_integer), Some(3));
        let id = PointId::from(p.id().to_string());
        let indexed = indexed_point(Some(&id), &payload).unwrap();
        assert_eq!(indexed.id, p.id());
        assert_eq!(indexed.updated_at, p.updated_at);
        assert_eq!(indexed.status, Status::Closed);
        assert_eq!(indexed.document_id, Some(DocumentId(Uuid::from_u128(4))));
        let h = hit(&payload, 0.5).unwrap();
        assert_eq!(h.key, "MOD-34");
        assert_eq!(
            h.document,
            Some((DocumentId(Uuid::from_u128(4)), "summary".into()))
        );
    }

    #[test]
    fn snippets_are_flattened_and_capped() {
        assert_eq!(snippet("a\n\n  b"), "a b");
        assert_eq!(snippet("x\u{1b}]52;c;AAAA\u{7}y"), "x]52;c;AAAAy");
        let long = "x".repeat(SNIPPET_CHARS + 10);
        assert_eq!(snippet(&long).chars().count(), SNIPPET_CHARS + 1);
    }

    #[test]
    fn filter_always_scopes_by_project_and_adds_given_sets() {
        let base = SearchQuery {
            text: "q".into(),
            projects: vec![ProjectId(Uuid::from_u128(2))],
            types: vec![],
            statuses: vec![],
            limit: 5,
        };
        assert_eq!(search_filter(&base).must.len(), 1);
        let narrowed = SearchQuery {
            types: vec![PointType::Document],
            statuses: vec![Status::Closed],
            ..base
        };
        assert_eq!(search_filter(&narrowed).must.len(), 3);
    }

    #[tokio::test]
    async fn mem_store_search_is_scoped_and_filtered() {
        let store = MemVectorStore::new();
        store
            .upsert(vec![point(None), point(Some(doc(0)))])
            .await
            .unwrap();
        let q = SearchQuery {
            text: "qdrant".into(),
            projects: vec![ProjectId(Uuid::from_u128(2))],
            types: vec![PointType::Item],
            statuses: vec![],
            limit: 10,
        };
        let hits = store.search(&q).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].point_type, PointType::Item);
        let other = SearchQuery {
            projects: vec![ProjectId(Uuid::from_u128(9))],
            ..q
        };
        assert!(store.search(&other).await.unwrap().is_empty());
    }
}
