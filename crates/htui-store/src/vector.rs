//! The concepts index: items, their documents and requirements in Qdrant, searched by meaning and by exact
//! term at once (MOD-34, `R-STO-8`, `docs/ANA-19.md` §3.3, `docs/ANA-20.md` §3).
//!
//! One collection holds every project (ANA-20 §3.4); the tenant is `project_id`, and a point's
//! `type` says what it indexes: an item, a section of one of its documents, or a requirement
//! (MOD-50). Postgres stays the source of truth (`R-STO-1`): every point can be rebuilt from an
//! item, document or requirement row, and [`crate::vector_sync::Indexer`] is what keeps the two in
//! step. Qdrant errors are [`StoreError::Backend`], never [`StoreError::Unreachable`], so
//! nothing that reads the latter as "Postgres went away" can mistake a down vector store for it.
use crate::bm25::{self, SparseVector};
use crate::embed::{DenseEmbedder, EmbedderIdentity};
use crate::qdrant_settings::QdrantSettings;
use chrono::{DateTime, Utc};
use htui_core::model::{
    DocumentId, ItemId, ItemKindId, Priority, ProjectId, RequirementId, RequirementState,
    Resolution, Status,
};
use htui_core::store::StoreError;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    CollectionInfo, Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder,
    DeletePointsBuilder, Distance, FieldType, Filter, Fusion, Modifier, NamedVectors, PointId,
    PointStruct, PrefetchQueryBuilder, Query, QueryPointsBuilder, ScrollPointsBuilder,
    SparseVectorParamsBuilder, SparseVectorsConfigBuilder, UpdateCollectionBuilder,
    UpsertPointsBuilder, Value, VectorParamsBuilder, VectorsConfigBuilder,
    point_id::PointIdOptions, vectors_config,
};
use serde::Deserialize as _;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

/// The collection name. Versioned: a change to the vector layout or the payload gets a new name
/// rather than a migration, since the whole index can be rebuilt from Postgres. `v2` (MOD-50 D222)
/// added `resolution` and requirement points: a `v1` point of an item closed before then would
/// never have been rebuilt, since neither its `updated_at` nor its status moves again.
pub const COLLECTION: &str = "htui_concepts_v2";
/// Name of the dense vector.
pub const DENSE: &str = "dense";
/// Name of the sparse (BM25) vector.
pub const SPARSE: &str = "sparse";
/// The collection metadata key the embedder's identity is stored under (MOD-68 D8).
pub const EMBEDDER_KEY: &str = "embedder";
/// How the message of every refusal of an existing collection begins, before `: ` and what is
/// wrong with which collection and how to rebuild it (MOD-68 D8, review L3). The one spelling
/// [`is_embedder_mismatch`] reads.
pub const EMBEDDER_MISMATCH: &str = "qdrant: embedder mismatch";
/// Longest snippet a point carries in its payload, in characters.
pub const SNIPPET_CHARS: usize = 280;

// Payload keys.
const TYPE: &str = "type";
const ITEM_ID: &str = "item_id";
const KEY: &str = "key";
const PROJECT_ID: &str = "project_id";
const KIND_ID: &str = "kind_id";
const STATUS: &str = "status";
const RESOLUTION: &str = "resolution";
const UPDATED_AT: &str = "updated_at";
const DOCUMENT_ID: &str = "document_id";
const DOC_KIND: &str = "doc_kind";
const DOC_VERSION: &str = "doc_version";
const CHUNK: &str = "chunk";
const SNIPPET: &str = "snippet";
const REQUIREMENT_ID: &str = "requirement_id";
const AREA_CODE: &str = "area_code";
const PRIORITY: &str = "priority";
const STATE: &str = "state";
const VERSION: &str = "version";

/// What a point indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointType {
    /// An item's key, title and body.
    Item,
    /// One section of an item's latest document of some kind.
    Document,
    /// A requirement's key, body and rationale (MOD-50).
    Requirement,
}

impl PointType {
    /// The payload text of this type.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Document => "document",
            Self::Requirement => "requirement",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "item" => Some(Self::Item),
            "document" => Some(Self::Document),
            "requirement" => Some(Self::Requirement),
            _ => None,
        }
    }
}

/// The row a point was built from, with what its payload says about it (MOD-50 D225).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// An item, for its own point and for the points of its documents.
    Item {
        /// `item.id`.
        id: ItemId,
        /// `item.kind_id`.
        kind_id: ItemKindId,
        /// `item.status`.
        status: Status,
        /// `item.resolution`: `Some` exactly when the item is closed, so a search can ask for
        /// decisions (MOD-50 D223).
        resolution: Option<Resolution>,
    },
    /// A requirement row.
    Requirement {
        /// `requirement.id`.
        id: RequirementId,
        /// `requirement.area_code`.
        area_code: String,
        /// `requirement.priority`.
        priority: Priority,
        /// `requirement.state`; a withdrawn requirement stays indexed (MOD-50 D224).
        state: RequirementState,
        /// `requirement.version`, what the indexer compares (MOD-50 D226).
        version: i32,
    },
}

impl Subject {
    /// Which row this is, without the rest of it.
    #[must_use]
    pub const fn owner(&self) -> Owner {
        match self {
            Self::Item { id, .. } => Owner::Item(*id),
            Self::Requirement { id, .. } => Owner::Requirement(*id),
        }
    }

    /// The item's status; `None` for a requirement.
    #[must_use]
    pub const fn status(&self) -> Option<Status> {
        match self {
            Self::Item { status, .. } => Some(*status),
            Self::Requirement { .. } => None,
        }
    }

    /// The item's resolution; `None` for an item that is not closed and for a requirement.
    #[must_use]
    pub const fn resolution(&self) -> Option<Resolution> {
        match self {
            Self::Item { resolution, .. } => *resolution,
            Self::Requirement { .. } => None,
        }
    }

    /// The requirement's state; `None` for an item.
    #[must_use]
    pub const fn state(&self) -> Option<RequirementState> {
        match self {
            Self::Requirement { state, .. } => Some(*state),
            Self::Item { .. } => None,
        }
    }

    /// The requirement's version; `None` for an item.
    #[must_use]
    pub const fn version(&self) -> Option<i32> {
        match self {
            Self::Requirement { version, .. } => Some(*version),
            Self::Item { .. } => None,
        }
    }
}

/// The row a point belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// An item, or the item a document belongs to.
    Item(ItemId),
    /// A requirement.
    Requirement(RequirementId),
}

impl Owner {
    /// The item, when the owner is one.
    #[must_use]
    pub const fn item(self) -> Option<ItemId> {
        match self {
            Self::Item(id) => Some(id),
            Self::Requirement(_) => None,
        }
    }

    /// The requirement, when the owner is one.
    #[must_use]
    pub const fn requirement(self) -> Option<RequirementId> {
        match self {
            Self::Requirement(id) => Some(id),
            Self::Item(_) => None,
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
    /// The item or requirement the point was built from.
    pub subject: Subject,
    /// The item's or the requirement's key, e.g. `MOD-34` or `R-STO-8`.
    pub key: String,
    /// The row's project.
    pub project_id: ProjectId,
    /// The row's `updated_at` when this point was built.
    pub updated_at: DateTime<Utc>,
    /// Set on document points, which only an item has.
    pub document: Option<DocumentRef>,
    /// The text embedded, dense and sparse.
    pub text: String,
}

impl ConceptPoint {
    /// What the point indexes: a requirement, a document section, or an item.
    #[must_use]
    pub const fn point_type(&self) -> PointType {
        match (&self.subject, &self.document) {
            (Subject::Requirement { .. }, _) => PointType::Requirement,
            (Subject::Item { .. }, Some(_)) => PointType::Document,
            (Subject::Item { .. }, None) => PointType::Item,
        }
    }

    /// The point's ID: the item's UUID for an item point; for a document section, a UUID (version
    /// 8) derived from SHA-256 of the document ID and section number; for a requirement, one
    /// derived the same way from its ID. All stable forever.
    #[must_use]
    pub fn id(&self) -> Uuid {
        match (&self.subject, &self.document) {
            (Subject::Requirement { id, .. }, _) => requirement_point_id(*id),
            (Subject::Item { .. }, Some(doc)) => document_point_id(doc.id, doc.chunk),
            (Subject::Item { id, .. }, None) => id.0,
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

/// The ID of requirement `id`'s point. Derived rather than the requirement's own UUID, so it can
/// never meet an item point's (MOD-50 D224).
#[must_use]
pub fn requirement_point_id(id: RequirementId) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"htui-concepts/requirement\0");
    hasher.update(id.0.as_bytes());
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("16 bytes");
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

/// What the index holds for one point, as the indexer needs it to decide what changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPoint {
    /// Point ID.
    pub id: Uuid,
    /// Item, document section or requirement.
    pub point_type: PointType,
    /// The item or requirement it belongs to.
    pub owner: Owner,
    /// The row's `updated_at` when the point was built.
    pub updated_at: DateTime<Utc>,
    /// The item's status when the point was built, on item and document points. Compared on its
    /// own because a status move (`WriteStore::transition`) does not touch `updated_at`.
    pub status: Option<Status>,
    /// The requirement's version when the point was built, on requirement points.
    pub version: Option<i32>,
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
    /// Item statuses to return; empty means all. A requirement has no status, so a non-empty
    /// list leaves requirements out.
    pub statuses: Vec<Status>,
    /// Item resolutions to return; empty means all. Only a closed item and its documents carry
    /// one, so a non-empty list never returns a requirement or an item that is not closed:
    /// `--decisions` asks for `done`, `concluded` and `rejected` (MOD-50 D223).
    pub resolutions: Vec<Resolution>,
    /// Most hits returned.
    pub limit: u64,
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Item, document section or requirement.
    pub point_type: PointType,
    /// The item or requirement.
    pub owner: Owner,
    /// The item's or the requirement's key.
    pub key: String,
    /// The document and its kind, on document hits.
    pub document: Option<(DocumentId, String)>,
    /// The item's resolution, on hits of a closed item and its documents.
    pub resolution: Option<Resolution>,
    /// The requirement's state, on requirement hits.
    pub state: Option<RequirementState>,
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

/// Whether `err` is [`QdrantStore::connect`]'s refusal of a collection holding another width or
/// another embedder's vectors (review L3): Qdrant answered, so it is not "cannot reach", and only
/// rebuilding the collection clears it, so retrying sooner does not help.
///
/// Read from the message, not a variant: `StoreError` is `htui-core`'s, shared by every store and
/// matched across the workspace, and this is the one error of one backend that needs telling
/// apart. [`EMBEDDER_MISMATCH`] is the single spelling both sides use.
#[must_use]
pub fn is_embedder_mismatch(err: &StoreError) -> bool {
    matches!(err, StoreError::Backend(message)
        if message.strip_prefix(EMBEDDER_MISMATCH).is_some_and(|rest| rest.starts_with(": ")))
}

/// Set once the process has warned that its Qdrant keeps no collection metadata, so that warning
/// is said once, not on every connect.
static UNSTAMPED_WARNED: AtomicBool = AtomicBool::new(false);

/// What `ensure_collection` does with a collection that already exists.
#[derive(Debug, PartialEq, Eq)]
enum Existing {
    /// Its `dense` width and its recorded embedder are this one's.
    Matches,
    /// Its `dense` width is right and it records no embedder (every pre-MOD-68 index): record
    /// this one. Safe because fastembed's vectors are the rten embedder's (`docs/ANA-23.md` §5.6).
    Stamp,
}

/// Whether a collection holding `dense_size`-wide `dense` vectors and `stored` as its `embedder`
/// metadata can be used by an embedder `mine` of width `dim` (MOD-68 D8). The width is checked
/// first (A-10): a collection of the wrong width is refused even when it records nothing, so it
/// is never stamped and then failed on every upsert.
fn check_existing(
    collection: &str,
    dense_size: Option<u64>,
    stored: Option<serde_json::Value>,
    mine: &EmbedderIdentity,
    dim: usize,
) -> Result<Existing, StoreError> {
    let refuse = |what: String| {
        StoreError::Backend(format!(
            "{EMBEDDER_MISMATCH}: collection `{collection}` {what}; delete collection \
             `{collection}` and run `htui --index-items` to rebuild it"
        ))
    };
    match dense_size {
        None => return Err(refuse("has no `dense` vector".to_owned())),
        Some(found) if found != dim as u64 => {
            return Err(refuse(format!(
                "holds {found}-wide `dense` vectors, but this htui embeds {dim}-wide ones"
            )));
        }
        Some(_) => {}
    }
    let Some(stored) = stored else {
        return Ok(Existing::Stamp);
    };
    let theirs = match EmbedderIdentity::deserialize(&stored) {
        Ok(id) if id == *mine => return Ok(Existing::Matches),
        Ok(id) => id.to_string(),
        Err(_) => stored.to_string(),
    };
    Err(refuse(format!(
        "holds vectors from {theirs}, but this htui embeds with {mine}"
    )))
}

/// The width of the collection's `dense` vector; `None` when it has no such named vector.
fn dense_size(info: &CollectionInfo) -> Option<u64> {
    let params = info.config.as_ref()?.params.as_ref()?;
    match params.vectors_config.as_ref()?.config.as_ref()? {
        vectors_config::Config::ParamsMap(named) => named.map.get(DENSE).map(|p| p.size),
        vectors_config::Config::Params(_) => None,
    }
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
        (TYPE.to_owned(), Value::from(point.point_type().as_str())),
        (KEY.to_owned(), Value::from(point.key.clone())),
        (
            PROJECT_ID.to_owned(),
            Value::from(point.project_id.0.to_string()),
        ),
        (
            UPDATED_AT.to_owned(),
            Value::from(point.updated_at.to_rfc3339()),
        ),
        (SNIPPET.to_owned(), Value::from(snippet(&point.text))),
    ]);
    match &point.subject {
        Subject::Item {
            id,
            kind_id,
            status,
            resolution,
        } => {
            p.insert(ITEM_ID.to_owned(), Value::from(id.0.to_string()));
            p.insert(KIND_ID.to_owned(), Value::from(kind_id.0.to_string()));
            p.insert(STATUS.to_owned(), Value::from(status.as_str()));
            // Absent, not null, on an item that is not closed: a resolution filter then leaves it
            // out the same way it leaves out a requirement.
            if let Some(r) = resolution {
                p.insert(RESOLUTION.to_owned(), Value::from(r.as_str()));
            }
            // Only an item has documents; a requirement point never writes these keys.
            if let Some(doc) = &point.document {
                p.insert(DOCUMENT_ID.to_owned(), Value::from(doc.id.0.to_string()));
                p.insert(DOC_KIND.to_owned(), Value::from(doc.kind.clone()));
                p.insert(DOC_VERSION.to_owned(), Value::from(i64::from(doc.version)));
                p.insert(CHUNK.to_owned(), Value::from(i64::from(doc.chunk)));
            }
        }
        Subject::Requirement {
            id,
            area_code,
            priority,
            state,
            version,
        } => {
            p.insert(REQUIREMENT_ID.to_owned(), Value::from(id.0.to_string()));
            p.insert(AREA_CODE.to_owned(), Value::from(area_code.clone()));
            p.insert(PRIORITY.to_owned(), Value::from(priority.as_str()));
            p.insert(STATE.to_owned(), Value::from(state.as_str()));
            p.insert(VERSION.to_owned(), Value::from(i64::from(*version)));
        }
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

/// The owner a payload names, by its type: a requirement point names a requirement, every other
/// point an item.
fn owner_of(point_type: PointType, payload: &HashMap<String, Value>) -> Option<Owner> {
    Some(match point_type {
        PointType::Requirement => {
            Owner::Requirement(RequirementId(uuid_of(payload, REQUIREMENT_ID)?))
        }
        PointType::Item | PointType::Document => Owner::Item(ItemId(uuid_of(payload, ITEM_ID)?)),
    })
}

fn parsed<T: FromStr>(payload: &HashMap<String, Value>, key: &str) -> Option<T> {
    text_of(payload, key).and_then(|s| T::from_str(s).ok())
}

fn indexed_point(id: Option<&PointId>, payload: &HashMap<String, Value>) -> Option<IndexedPoint> {
    let point_type = PointType::parse(text_of(payload, TYPE)?)?;
    let (status, version) = match point_type {
        PointType::Requirement => (
            None,
            Some(i32::try_from(payload.get(VERSION)?.as_integer()?).ok()?),
        ),
        PointType::Item | PointType::Document => (Some(parsed(payload, STATUS)?), None),
    };
    Some(IndexedPoint {
        id: point_uuid(id)?,
        point_type,
        owner: owner_of(point_type, payload)?,
        updated_at: DateTime::parse_from_rfc3339(text_of(payload, UPDATED_AT)?)
            .ok()?
            .with_timezone(&Utc),
        status,
        version,
        document_id: uuid_of(payload, DOCUMENT_ID).map(DocumentId),
    })
}

fn hit(payload: &HashMap<String, Value>, score: f32) -> Option<Hit> {
    let document = match (uuid_of(payload, DOCUMENT_ID), text_of(payload, DOC_KIND)) {
        (Some(id), Some(kind)) => Some((DocumentId(id), kind.to_owned())),
        _ => None,
    };
    let point_type = PointType::parse(text_of(payload, TYPE)?)?;
    Some(Hit {
        point_type,
        owner: owner_of(point_type, payload)?,
        key: text_of(payload, KEY)?.to_owned(),
        document,
        resolution: parsed(payload, RESOLUTION),
        state: parsed(payload, STATE),
        score,
        snippet: text_of(payload, SNIPPET).unwrap_or_default().to_owned(),
    })
}

/// The filter a search applies: its projects, and its types, statuses and resolutions when given.
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
    // A point without the key fails the match, so this also leaves out requirements and every
    // item that is not closed (MOD-50 D223).
    if !query.resolutions.is_empty() {
        must.push(Condition::matches(
            RESOLUTION,
            query
                .resolutions
                .iter()
                .map(|r| r.as_str().to_owned())
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
    ///
    /// # Errors
    /// [`StoreError::Backend`] when Qdrant cannot be reached, and `embedder mismatch` when the
    /// collection holds another width or another embedder's vectors (MOD-68 D8); one that records
    /// no embedder is stamped with this one's [`identity`](DenseEmbedder::identity).
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
        if exists {
            self.check_collection().await?;
        } else {
            self.create_collection().await?;
        }
        // Every time, not only on creation: re-creating an existing index is a no-op in Qdrant, and
        // a first run that died between the collection and its indexes is repaired here.
        for field in [TYPE, PROJECT_ID, STATUS, RESOLUTION, ITEM_ID] {
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

    /// Refuses an existing collection of another width or another embedder, and records this
    /// embedder in one that records none (MOD-68 D8).
    async fn check_collection(&self) -> Result<(), StoreError> {
        let info = self
            .client
            .collection_info(&self.collection)
            .await
            .map_err(|e| backend("collection info", e))?
            .result
            .ok_or_else(|| backend("collection info", "no result"))?;
        let stored = info
            .config
            .as_ref()
            .and_then(|c| c.metadata.get(EMBEDDER_KEY))
            .cloned()
            .map(serde_json::Value::from);
        let mine = self.embedder.identity();
        match check_existing(
            &self.collection,
            dense_size(&info),
            stored,
            &mine,
            self.embedder.dim(),
        )? {
            Existing::Matches => {}
            Existing::Stamp => {
                // Qdrant merges an update's metadata into the collection's: the stamp adds its key
                // and leaves any other (observed on 1.19.1 over gRPC and REST; `qdrant_live`'s
                // `stamping_keeps_the_collections_other_metadata`). Two processes stamping at once
                // write the same value.
                self.client
                    .update_collection(
                        UpdateCollectionBuilder::new(&self.collection).metadata(self.metadata()?),
                    )
                    .await
                    .map_err(|e| backend("stamp embedder", e))?;
                if self.stamp_reads_back().await {
                    tracing::info!(
                        collection = %self.collection,
                        identity = %mine,
                        "stamped the collection's embedder"
                    );
                } else if !UNSTAMPED_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!(
                        collection = %self.collection,
                        "this Qdrant does not keep collection metadata: the embedder check is off \
                         for it, and a collection of another model's vectors would be searched \
                         as this one's"
                    );
                }
            }
        }
        Ok(())
    }

    /// Whether the collection now records an embedder. A Qdrant that predates collection
    /// metadata accepts the update and drops it, and a failed read is counted the same way.
    async fn stamp_reads_back(&self) -> bool {
        self.client
            .collection_info(&self.collection)
            .await
            .ok()
            .and_then(|info| info.result?.config)
            .is_some_and(|config| config.metadata.contains_key(EMBEDDER_KEY))
    }

    /// `{EMBEDDER_KEY: identity}`, the collection metadata this embedder writes.
    fn metadata(&self) -> Result<HashMap<String, serde_json::Value>, StoreError> {
        let identity = serde_json::to_value(self.embedder.identity())
            .map_err(|e| backend("embedder identity", e))?;
        Ok(HashMap::from([(EMBEDDER_KEY.to_owned(), identity)]))
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
                    .sparse_vectors_config(sparse)
                    .metadata(self.metadata()?),
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
                point_type: p.point_type(),
                owner: p.subject.owner(),
                updated_at: p.updated_at,
                status: p.subject.status(),
                version: p.subject.version(),
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
            .filter(|p| query.types.is_empty() || query.types.contains(&p.point_type()))
            // A missing field fails a non-empty filter, as Qdrant's `match any` does.
            .filter(|p| {
                query.statuses.is_empty()
                    || p.subject
                        .status()
                        .is_some_and(|s| query.statuses.contains(&s))
            })
            .filter(|p| {
                query.resolutions.is_empty()
                    || p.subject
                        .resolution()
                        .is_some_and(|r| query.resolutions.contains(&r))
            })
            .filter_map(|p| {
                let own = bm25::tokenize(&p.text);
                #[allow(clippy::cast_precision_loss)]
                let score = terms.iter().filter(|t| own.contains(t)).count() as f32;
                (score > 0.0).then(|| Hit {
                    point_type: p.point_type(),
                    owner: p.subject.owner(),
                    key: p.key.clone(),
                    document: p.document.as_ref().map(|d| (d.id, d.kind.clone())),
                    resolution: p.subject.resolution(),
                    state: p.subject.state(),
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

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-25T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn point(document: Option<DocumentRef>) -> ConceptPoint {
        ConceptPoint {
            subject: Subject::Item {
                id: ItemId(Uuid::from_u128(1)),
                kind_id: ItemKindId(Uuid::from_u128(3)),
                status: Status::Closed,
                resolution: Some(Resolution::Rejected),
            },
            key: "MOD-34".into(),
            project_id: ProjectId(Uuid::from_u128(2)),
            updated_at: at(),
            document,
            text: "MOD-34 Qdrant related concepts search".into(),
        }
    }

    fn requirement(state: RequirementState) -> ConceptPoint {
        ConceptPoint {
            subject: Subject::Requirement {
                id: RequirementId(Uuid::from_u128(1)),
                area_code: "STO".into(),
                priority: Priority::Must,
                state,
                version: 3,
            },
            key: "R-STO-8".into(),
            project_id: ProjectId(Uuid::from_u128(2)),
            updated_at: at(),
            document: None,
            text: "R-STO-8 Semantic search over items and requirements".into(),
        }
    }

    fn open_item() -> ConceptPoint {
        ConceptPoint {
            subject: Subject::Item {
                id: ItemId(Uuid::from_u128(5)),
                kind_id: ItemKindId(Uuid::from_u128(3)),
                status: Status::Open,
                resolution: None,
            },
            key: "MOD-99".into(),
            text: "MOD-99 Qdrant search still open".into(),
            ..point(None)
        }
    }

    fn scoped(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.into(),
            projects: vec![ProjectId(Uuid::from_u128(2))],
            types: vec![],
            statuses: vec![],
            resolutions: vec![],
            limit: 10,
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
    fn point_types_follow_the_subject_and_the_document() {
        assert_eq!(point(None).point_type(), PointType::Item);
        assert_eq!(point(Some(doc(0))).point_type(), PointType::Document);
        assert_eq!(
            requirement(RequirementState::Active).point_type(),
            PointType::Requirement
        );
        for t in [PointType::Item, PointType::Document, PointType::Requirement] {
            assert_eq!(PointType::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn requirement_point_ids_are_derived_and_never_the_raw_uuid() {
        let r = requirement(RequirementState::Active);
        let id = RequirementId(Uuid::from_u128(1));
        assert_eq!(r.id(), requirement_point_id(id));
        assert_eq!(r.id().get_version_num(), 8);
        // The item of the same UUID keeps its raw ID, so the two points never meet.
        assert_eq!(point(None).id(), Uuid::from_u128(1));
        assert_ne!(r.id(), point(None).id());
        // Golden: a change here orphans every requirement point already indexed.
        assert_eq!(
            requirement_point_id(id).to_string(),
            "48f650ab-a53f-80b5-bb0a-bca4d0a2b825"
        );
        assert_ne!(
            requirement_point_id(id),
            document_point_id(DocumentId(Uuid::from_u128(1)), 0)
        );
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
        assert_eq!(indexed.document_id, Some(DocumentId(Uuid::from_u128(4))));
        assert_eq!(indexed.status, Some(Status::Closed));
        assert_eq!(indexed.version, None);
        assert_eq!(indexed.owner, Owner::Item(ItemId(Uuid::from_u128(1))));
        let h = hit(&payload, 0.5).unwrap();
        assert_eq!(h.key, "MOD-34");
        assert_eq!(
            h.document,
            Some((DocumentId(Uuid::from_u128(4)), "summary".into()))
        );
        assert_eq!(h.resolution, Some(Resolution::Rejected));
        assert_eq!(h.state, None);
    }

    #[test]
    fn a_requirement_point_never_writes_document_keys() {
        let odd = ConceptPoint {
            document: Some(doc(0)),
            ..requirement(RequirementState::Active)
        };
        let payload = payload(&odd);
        assert!(!payload.contains_key(DOCUMENT_ID));
        assert!(!payload.contains_key(DOC_KIND));
        assert_eq!(hit(&payload, 0.5).unwrap().document, None);
    }

    #[test]
    fn an_open_item_has_no_resolution_key() {
        let payload = payload(&open_item());
        assert!(!payload.contains_key(RESOLUTION));
        assert_eq!(hit(&payload, 0.5).unwrap().resolution, None);
    }

    #[test]
    fn a_requirement_payload_round_trips() {
        let p = requirement(RequirementState::Withdrawn);
        let payload = payload(&p);
        assert_eq!(text_of(&payload, TYPE), Some("requirement"));
        assert_eq!(text_of(&payload, AREA_CODE), Some("STO"));
        assert_eq!(text_of(&payload, PRIORITY), Some("must"));
        assert_eq!(text_of(&payload, STATE), Some("withdrawn"));
        assert!(!payload.contains_key(ITEM_ID));
        assert!(!payload.contains_key(STATUS));
        assert!(!payload.contains_key(RESOLUTION));
        let id = PointId::from(p.id().to_string());
        let indexed = indexed_point(Some(&id), &payload).unwrap();
        assert_eq!(indexed.point_type, PointType::Requirement);
        assert_eq!(
            indexed.owner,
            Owner::Requirement(RequirementId(Uuid::from_u128(1)))
        );
        assert_eq!(indexed.version, Some(3));
        assert_eq!(indexed.status, None);
        let h = hit(&payload, 0.5).unwrap();
        assert_eq!(h.key, "R-STO-8");
        assert_eq!(h.state, Some(RequirementState::Withdrawn));
        assert_eq!(h.resolution, None);
        assert_eq!(h.document, None);
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
        let base = scoped("q");
        assert_eq!(search_filter(&base).must.len(), 1);
        let narrowed = SearchQuery {
            types: vec![PointType::Document],
            statuses: vec![Status::Closed],
            resolutions: vec![Resolution::Done],
            ..base
        };
        assert_eq!(search_filter(&narrowed).must.len(), 4);
    }

    #[tokio::test]
    async fn mem_store_search_is_scoped_and_filtered() {
        let store = MemVectorStore::new();
        store
            .upsert(vec![point(None), point(Some(doc(0)))])
            .await
            .unwrap();
        let q = SearchQuery {
            types: vec![PointType::Item],
            ..scoped("qdrant")
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

    #[tokio::test]
    async fn a_resolution_filter_keeps_closed_items_only() {
        let store = MemVectorStore::new();
        store
            .upsert(vec![
                point(None),
                point(Some(doc(0))),
                open_item(),
                requirement(RequirementState::Active),
            ])
            .await
            .unwrap();
        let everything = store.search(&scoped("search")).await.unwrap();
        assert_eq!(everything.len(), 4);
        let rejected = SearchQuery {
            resolutions: vec![Resolution::Rejected],
            ..scoped("search")
        };
        let hits = store.search(&rejected).await.unwrap();
        assert_eq!(hits.len(), 2, "the item and its document, {hits:?}");
        assert!(
            hits.iter()
                .all(|h| h.resolution == Some(Resolution::Rejected))
        );
        let done = SearchQuery {
            resolutions: vec![Resolution::Done],
            ..scoped("search")
        };
        assert!(store.search(&done).await.unwrap().is_empty());
        let closed = SearchQuery {
            statuses: vec![Status::Closed, Status::Open],
            ..scoped("search")
        };
        let hits = store.search(&closed).await.unwrap();
        assert!(hits.iter().all(|h| h.point_type != PointType::Requirement));
    }

    const C: &str = "htui_concepts_v2";

    fn bge() -> EmbedderIdentity {
        EmbedderIdentity {
            model: "Xenova/bge-small-en-v1.5".into(),
            revision: "ea104dacec62c0de699686887e3f920caeb4f3e3".into(),
            onnx_sha256: "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35".into(),
            dim: 384,
            pooling: "cls".into(),
            normalisation: "l2".into(),
        }
    }

    fn stored(id: &EmbedderIdentity) -> Option<serde_json::Value> {
        Some(serde_json::to_value(id).unwrap())
    }

    fn assert_names_the_remedy(err: &str) {
        assert!(
            err.contains("qdrant: embedder mismatch: collection `"),
            "{err}"
        );
        assert!(err.contains(&format!("collection `{C}`")), "{err}");
        assert!(
            err.contains(&format!(
                "delete collection `{C}` and run `htui --index-items` to rebuild it"
            )),
            "{err}"
        );
    }

    /// Review L3: the front ends tell a refusal from a Qdrant that does not answer.
    #[test]
    fn every_refusal_is_an_embedder_mismatch_and_nothing_else_is() {
        let hash = EmbedderIdentity::hash(384);
        for err in [
            check_existing(C, Some(8), None, &bge(), 384).unwrap_err(),
            check_existing(C, None, None, &bge(), 384).unwrap_err(),
            check_existing(C, Some(384), stored(&hash), &bge(), 384).unwrap_err(),
        ] {
            assert!(is_embedder_mismatch(&err), "{err}");
        }
        for err in [
            backend("collection info", "transport error"),
            StoreError::Backend("qdrant: embedder mismatches nothing".into()),
            StoreError::Unreachable(format!("{EMBEDDER_MISMATCH}: not a backend error")),
        ] {
            assert!(!is_embedder_mismatch(&err), "{err}");
        }
    }

    #[test]
    fn an_equal_identity_matches() {
        assert_eq!(
            check_existing(C, Some(384), stored(&bge()), &bge(), 384).unwrap(),
            Existing::Matches
        );
    }

    #[test]
    fn a_collection_without_an_identity_is_stamped() {
        assert_eq!(
            check_existing(C, Some(384), None, &bge(), 384).unwrap(),
            Existing::Stamp
        );
    }

    #[test]
    fn another_identity_is_refused_naming_both_and_the_remedy() {
        let hash = EmbedderIdentity::hash(384);
        let err = check_existing(C, Some(384), stored(&hash), &bge(), 384)
            .unwrap_err()
            .to_string();
        assert_names_the_remedy(&err);
        assert!(err.contains("holds vectors from hash/384"), "{err}");
        assert!(
            err.contains(&format!("but this htui embeds with {}", bge())),
            "{err}"
        );
    }

    #[test]
    fn another_width_is_refused_before_any_stamp() {
        for stored_id in [None, stored(&bge())] {
            let err = check_existing(C, Some(8), stored_id, &bge(), 384)
                .unwrap_err()
                .to_string();
            assert_names_the_remedy(&err);
            assert!(
                err.contains("holds 8-wide `dense` vectors, but this htui embeds 384-wide ones"),
                "{err}"
            );
        }
    }

    #[test]
    fn a_collection_without_a_dense_vector_is_refused() {
        let err = check_existing(C, None, None, &bge(), 384)
            .unwrap_err()
            .to_string();
        assert_names_the_remedy(&err);
        assert!(err.contains("has no `dense` vector"), "{err}");
    }

    #[test]
    fn an_unreadable_identity_is_refused() {
        let err = check_existing(
            C,
            Some(384),
            Some(serde_json::json!("garbage")),
            &bge(),
            384,
        )
        .unwrap_err()
        .to_string();
        assert_names_the_remedy(&err);
        assert!(err.contains("holds vectors from \"garbage\""), "{err}");
    }

    #[test]
    fn the_identity_round_trips_through_qdrant_metadata() {
        let id = bge();
        let json = serde_json::to_value(&id).unwrap();
        assert_eq!(json["dim"], serde_json::json!(384));
        let back = serde_json::Value::from(Value::from(json));
        assert_eq!(
            serde_json::from_value::<EmbedderIdentity>(back).unwrap(),
            id
        );
    }
}
