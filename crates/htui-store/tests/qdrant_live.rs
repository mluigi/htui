//! The concepts index against a real Qdrant (MOD-34, `R-STO-8`).
//!
//! Gated on `HTUI_TEST_QDRANT_URL` (the gRPC endpoint, e.g. `http://localhost:6334` from
//! `docker compose up -d qdrant`); unset, every case prints the skip line and passes, as the
//! Postgres suites do with `HTUI_TEST_DATABASE_URL`. Each case owns a throwaway collection and
//! drops it. Vectors come from `HashEmbedder`, so no model is downloaded: what is under test is
//! the collection layout, the payload filters, hybrid fusion and the indexer's bookkeeping, not
//! relevance.
#![cfg(feature = "test-support")]

use htui_core::fixtures::ids;
use htui_core::model::{
    ItemFilter, RequirementFilter, RequirementState, Resolution, Scope, Status,
};
use htui_core::store::StoreError;
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
use htui_store::embed::{DENSE_DIM, DenseEmbedder, EmbedderIdentity, HashEmbedder};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{
    DENSE, EMBEDDER_KEY, ExistingCollection, Owner, PAYLOAD_SCHEMA, PAYLOAD_SCHEMA_KEY, PointType,
    Probe, QdrantStore, SPARSE, SearchQuery, VectorStore as _, is_embedder_mismatch, probe_with,
};
use htui_store::vector_sync::Indexer;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    CountPointsBuilder, CreateCollectionBuilder, Distance, Modifier, SparseVectorParamsBuilder,
    SparseVectorsConfigBuilder, UpdateCollectionBuilder, VectorParamsBuilder, VectorsConfigBuilder,
};

const ENV_URL: &str = "HTUI_TEST_QDRANT_URL";

/// The test Qdrant's URL, or the skip line.
fn qdrant_url() -> Option<String> {
    let url = std::env::var(ENV_URL).ok();
    if url.is_none() {
        eprintln!("skipped: {ENV_URL} not set");
    }
    url
}

fn fresh_name() -> String {
    format!("htui_test_{}", uuid::Uuid::now_v7().simple())
}

async fn connect<E: DenseEmbedder>(
    url: &str,
    embedder: E,
    name: &str,
) -> Result<QdrantStore<E>, StoreError> {
    let settings = QdrantSettings::new(
        htui_core::qdrant::QdrantUrl::parse(url).expect("valid URL"),
        None,
    );
    QdrantStore::connect_to(&settings, embedder, name).await
}

async fn throwaway() -> Option<QdrantStore<HashEmbedder>> {
    let url = qdrant_url()?;
    Some(
        connect(&url, HashEmbedder::new(DENSE_DIM), &fresh_name())
            .await
            .expect("Qdrant answers"),
    )
}

fn scope() -> Scope {
    Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
    }
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_owned(),
        projects: vec![ids::PROJECT_HTUI],
        types: Vec::new(),
        statuses: Vec::new(),
        resolutions: Vec::new(),
        limit: 5,
    }
}

#[tokio::test]
async fn sync_then_search_finds_an_item_by_its_exact_key() {
    let Some(store) = throwaway().await else {
        return;
    };
    let read = MemStore::demo();
    let first = Indexer::sync(&read, &scope(), &store).await.expect("sync");
    assert!(first.points_upserted > 0);

    let second = Indexer::sync(&read, &scope(), &store)
        .await
        .expect("resync");
    assert_eq!(
        second.points_upserted, 0,
        "an unchanged store writes nothing"
    );
    assert_eq!(second.points_deleted, 0);

    let item = read
        .items(
            &Scope {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_ids: vec![ids::PROJECT_HTUI],
            },
            &ItemFilter::default(),
        )
        .await
        .expect("items")
        .remove(0);
    let hits = store.search(&query(&item.key)).await.expect("search");
    assert!(
        hits.iter().any(|h| h.key == item.key),
        "{} not among {hits:?}",
        item.key
    );
    assert!(hits.iter().all(|h| !h.snippet.is_empty()));
    store.drop_collection().await.expect("drop");
}

#[tokio::test]
async fn searches_are_scoped_to_projects_and_filtered_by_type_and_status() {
    let Some(store) = throwaway().await else {
        return;
    };
    let read = MemStore::demo();
    Indexer::sync(&read, &scope(), &store).await.expect("sync");

    let agy = store.indexed(ids::PROJECT_AGY).await.expect("indexed");
    assert!(!agy.is_empty());
    let htui_hits = store.search(&query("the")).await.expect("search");
    let agy_owners: Vec<Owner> = agy.iter().map(|p| p.owner).collect();
    assert!(htui_hits.iter().all(|h| !agy_owners.contains(&h.owner)));

    let docs_only = SearchQuery {
        types: vec![PointType::Document],
        ..query("the plan")
    };
    let doc_hits = store.search(&docs_only).await.expect("search");
    assert!(!doc_hits.is_empty(), "the demo documents mention a plan");
    for hit in doc_hits {
        assert_eq!(hit.point_type, PointType::Document);
        assert!(hit.document.is_some());
    }

    let nothing = SearchQuery {
        projects: Vec::new(),
        ..query("the")
    };
    assert!(store.search(&nothing).await.expect("search").is_empty());

    // The status filter: an item's own key finds it under its status and not under another.
    let item = read
        .item(
            agy_owners
                .iter()
                .find_map(|o| o.item())
                .expect("an agy item"),
        )
        .await
        .expect("item")
        .expect("exists");
    let own = SearchQuery {
        projects: vec![ids::PROJECT_AGY],
        statuses: vec![item.status],
        ..query(&item.key)
    };
    assert!(
        store
            .search(&own)
            .await
            .expect("search")
            .iter()
            .any(|h| h.owner == Owner::Item(item.id))
    );
    let other = Status::ALL
        .iter()
        .copied()
        .find(|s| *s != item.status)
        .expect("more than one status");
    let elsewhere = SearchQuery {
        statuses: vec![other],
        ..own
    };
    assert!(
        store
            .search(&elsewhere)
            .await
            .expect("search")
            .iter()
            .all(|h| h.owner != Owner::Item(item.id))
    );
    store.drop_collection().await.expect("drop");
}

#[tokio::test]
async fn deleted_points_leave_the_index() {
    let Some(store) = throwaway().await else {
        return;
    };
    let read = MemStore::demo();
    Indexer::sync(&read, &scope(), &store).await.expect("sync");
    let before = store.indexed(ids::PROJECT_HTUI).await.expect("indexed");
    let gone = before
        .iter()
        .find(|p| p.point_type == PointType::Item)
        .expect("an item point")
        .id;
    store.delete(vec![gone]).await.expect("delete");
    let after = store.indexed(ids::PROJECT_HTUI).await.expect("indexed");
    assert_eq!(after.len(), before.len() - 1);
    assert!(after.iter().all(|p| p.id != gone));

    // The next sync puts it back.
    let report = Indexer::sync(&read, &scope(), &store).await.expect("sync");
    assert_eq!(report.items_rebuilt, 1);
    assert_eq!(
        store
            .indexed(ids::PROJECT_HTUI)
            .await
            .expect("indexed")
            .len(),
        before.len()
    );
    store.drop_collection().await.expect("drop");
}

#[tokio::test]
async fn a_requirement_is_found_by_its_exact_key() {
    let Some(store) = throwaway().await else {
        return;
    };
    let read = MemStore::demo();
    let report = Indexer::sync(&read, &scope(), &store).await.expect("sync");
    let rows = read
        .requirements(ids::PROJECT_HTUI, &RequirementFilter::default())
        .await
        .expect("requirements");
    assert_eq!(report.requirements_rebuilt, rows.len());
    let row = &rows[0];
    // Among the hits, not necessarily first: `HashEmbedder`'s dense arm is noise, and fusion can
    // tie a sibling key (`R-ENT-2` shares the `ent` term) with it.
    let own = |hits: &[htui_store::vector::Hit]| {
        hits.iter()
            .find(|h| h.owner == Owner::Requirement(row.id))
            .cloned()
    };
    let hits = store.search(&query(&row.key)).await.expect("search");
    let hit = own(&hits).unwrap_or_else(|| panic!("{} not among {hits:?}", row.key));
    assert_eq!(hit.point_type, PointType::Requirement);
    assert_eq!(hit.key, row.key);
    assert_eq!(hit.state, Some(row.state));

    // Withdrawn: rebuilt with its new state, and nothing else moves.
    read.withdraw_requirement(
        row.id,
        row.version,
        ids::HTUI_FEAT_3,
        ids::USER,
        Some(ids::BOX),
    )
    .await
    .expect("withdraw");
    let again = Indexer::sync(&read, &scope(), &store)
        .await
        .expect("resync");
    assert_eq!(again.requirements_rebuilt, 1);
    assert_eq!(again.items_rebuilt, 0);
    assert_eq!(again.points_upserted, 1);
    let hits = store.search(&query(&row.key)).await.expect("search");
    let hit = own(&hits).expect("a withdrawn requirement stays searchable");
    assert_eq!(hit.state, Some(RequirementState::Withdrawn));
    store.drop_collection().await.expect("drop");
}

#[tokio::test]
async fn a_resolution_filter_keeps_decisions_and_leaves_requirements_out() {
    let Some(store) = throwaway().await else {
        return;
    };
    let read = MemStore::demo();
    Indexer::sync(&read, &scope(), &store).await.expect("sync");
    let fix = read
        .item(ids::HTUI_FIX_1)
        .await
        .expect("item")
        .expect("exists");
    assert_eq!(fix.resolution, Some(Resolution::Done));

    let decisions = SearchQuery {
        resolutions: vec![
            Resolution::Done,
            Resolution::Concluded,
            Resolution::Rejected,
        ],
        limit: 100,
        ..query(&format!("{} {}", fix.key, fix.title))
    };
    let hits = store.search(&decisions).await.expect("search");
    assert!(
        hits.iter().any(|h| h.owner == Owner::Item(fix.id)),
        "{} among {hits:?}",
        fix.key
    );
    for hit in &hits {
        assert_ne!(hit.point_type, PointType::Requirement);
        assert!(
            hit.resolution
                .is_some_and(|r| decisions.resolutions.contains(&r)),
            "{hit:?}"
        );
    }

    // A resolution the demo's closed items do not have finds none of them.
    let rejected = SearchQuery {
        resolutions: vec![Resolution::Rejected],
        ..decisions
    };
    assert!(store.search(&rejected).await.expect("search").is_empty());
    store.drop_collection().await.expect("drop");
}

// MOD-68 D8: the embedder's identity in the collection's metadata.

/// A client of its own, to look at and set up collections behind `QdrantStore`'s back.
fn raw(url: &str) -> Qdrant {
    Qdrant::from_url(url)
        .skip_compatibility_check()
        .build()
        .expect("client")
}

/// What the collection's `embedder` metadata says, if anything.
async fn stored_identity(client: &Qdrant, name: &str) -> Option<EmbedderIdentity> {
    let info = client
        .collection_info(name)
        .await
        .expect("collection info")
        .result
        .expect("a result");
    let value = info
        .config
        .expect("a config")
        .metadata
        .remove(EMBEDDER_KEY)?;
    Some(serde_json::from_value(serde_json::Value::from(value)).expect("an identity"))
}

/// A collection laid out as `create_collection` makes one, `dense` `dense_size` wide, with no
/// metadata: what every pre-MOD-68 collection looks like.
async fn create_bare(client: &Qdrant, name: &str, dense_size: u64) {
    create_unstamped(client, name, dense_size, None).await;
}

/// As [`create_bare`], holding `metadata` but no embedder.
async fn create_unstamped(
    client: &Qdrant,
    name: &str,
    dense_size: u64,
    metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
) {
    let mut dense = VectorsConfigBuilder::default();
    dense.add_named_vector_params(
        DENSE,
        VectorParamsBuilder::new(dense_size, Distance::Cosine),
    );
    let mut sparse = SparseVectorsConfigBuilder::default();
    sparse.add_named_vector_params(
        SPARSE,
        SparseVectorParamsBuilder::default().modifier(Modifier::Idf),
    );
    let mut create = CreateCollectionBuilder::new(name)
        .vectors_config(dense)
        .sparse_vectors_config(sparse);
    if let Some(metadata) = metadata {
        create = create.metadata(metadata);
    }
    client
        .create_collection(create)
        .await
        .expect("create a bare collection");
}

/// `HashEmbedder`'s vectors under another model's name.
#[derive(Debug, Clone)]
struct Renamed {
    inner: HashEmbedder,
    identity: EmbedderIdentity,
}

impl DenseEmbedder for Renamed {
    fn dim(&self) -> usize {
        self.inner.dim()
    }

    fn identity(&self) -> EmbedderIdentity {
        self.identity.clone()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError> {
        self.inner.embed(texts).await
    }
}

#[tokio::test]
async fn a_new_collection_records_the_embedder() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let store = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let client = raw(&url);
    let stored = stored_identity(&client, &name).await;
    store.drop_collection().await.expect("drop");
    assert_eq!(stored, Some(EmbedderIdentity::hash(DENSE_DIM)));
}

/// The stamp records the embedder only (MOD-144 R1): the collection was filled by an htui that
/// recorded no embedder, so its payload layout is unknown, and the next connect rebuilds it once
/// under this layout, keeping the embedder.
#[tokio::test]
async fn an_unrecorded_collection_is_stamped_once() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    create_bare(&client, &name, DENSE_DIM as u64).await;
    let first = connect(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let stamped = stored_identity(&client, &name).await;
    let stamped_payload = stored_payload_schema(&client, &name).await;
    let second = connect(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let after = stored_identity(&client, &name).await;
    let after_payload = stored_payload_schema(&client, &name).await;
    client.delete_collection(&name).await.expect("drop");
    first.expect("an unrecorded collection is accepted");
    assert_eq!(stamped, Some(EmbedderIdentity::hash(DENSE_DIM)));
    assert_eq!(stamped_payload, None, "the stamp records no payload layout");
    second.expect("a stamped collection is accepted");
    assert_eq!(after, stamped);
    assert_eq!(
        after_payload,
        Some(serde_json::json!(PAYLOAD_SCHEMA)),
        "the next connect rebuilt it under this payload layout"
    );
}

/// Qdrant merges an update's metadata into the collection's (observed on 1.19.1, over gRPC here
/// and over REST): the stamp adds its key and leaves the others.
#[tokio::test]
async fn stamping_keeps_the_collections_other_metadata() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    let other = std::collections::HashMap::from([("other".to_owned(), serde_json::json!("kept"))]);
    create_unstamped(&client, &name, DENSE_DIM as u64, Some(other)).await;
    let stamped = connect(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let identity = stored_identity(&client, &name).await;
    let info = client
        .collection_info(&name)
        .await
        .expect("collection info")
        .result
        .expect("a result");
    client.delete_collection(&name).await.expect("drop");
    stamped.expect("an unrecorded collection is accepted");
    assert_eq!(identity, Some(EmbedderIdentity::hash(DENSE_DIM)));
    let kept = info
        .config
        .expect("a config")
        .metadata
        .remove("other")
        .map(serde_json::Value::from);
    assert_eq!(kept, Some(serde_json::json!("kept")));
}

#[tokio::test]
async fn another_identity_is_refused_naming_both() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let other = Renamed {
        inner: HashEmbedder::new(DENSE_DIM),
        identity: EmbedderIdentity {
            model: "test/other".into(),
            ..EmbedderIdentity::hash(DENSE_DIM)
        },
    };
    let refused = connect(&url, other, &name).await;
    let client = raw(&url);
    let stored = stored_identity(&client, &name).await;
    client.delete_collection(&name).await.expect("drop");
    let err = refused.expect_err("another embedder is refused");
    assert!(is_embedder_mismatch(&err), "{err}");
    let err = err.to_string();
    assert!(err.contains("embedder mismatch"), "{err}");
    assert!(err.contains("hash/384"), "{err}");
    assert!(err.contains("test/other/384"), "{err}");
    assert!(err.contains(&format!("collection `{name}`")), "{err}");
    assert!(err.contains("htui --index-items"), "{err}");
    assert_eq!(
        stored,
        Some(EmbedderIdentity::hash(DENSE_DIM)),
        "left as it was"
    );
}

#[tokio::test]
async fn another_dense_width_is_refused_and_not_stamped() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    create_bare(&client, &name, 8).await;
    let refused = connect(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let stored = stored_identity(&client, &name).await;
    client.delete_collection(&name).await.expect("drop");
    let err = refused.expect_err("another width is refused");
    assert!(is_embedder_mismatch(&err), "{err}");
    let err = err.to_string();
    assert!(err.contains("8-wide"), "{err}");
    assert!(err.contains("384-wide"), "{err}");
    assert_eq!(stored, None, "a wrong-width collection is never stamped");
}

// MOD-92 G8: `open_existing`, the read-only open of `--search-items --json`.

async fn open_existing<E: DenseEmbedder>(
    url: &str,
    embedder: E,
    name: &str,
) -> Result<Option<QdrantStore<E>>, StoreError> {
    let settings = QdrantSettings::new(
        htui_core::qdrant::QdrantUrl::parse(url).expect("valid URL"),
        None,
    );
    QdrantStore::open_existing_to(&settings, embedder, name).await
}

#[tokio::test]
async fn open_existing_on_a_missing_collection_is_none_and_creates_nothing() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let opened = open_existing(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let exists = raw(&url)
        .collection_exists(&name)
        .await
        .expect("collection exists");
    if exists {
        raw(&url).delete_collection(&name).await.expect("drop");
    }
    assert!(opened.is_none(), "a missing collection is reported");
    assert!(!exists, "and not created");
}

#[tokio::test]
async fn open_existing_searches_what_connect_built() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let built = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let read = MemStore::demo();
    Indexer::sync(&read, &scope(), &built).await.expect("sync");
    let item = read
        .items(
            &Scope {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_ids: vec![ids::PROJECT_HTUI],
            },
            &ItemFilter::default(),
        )
        .await
        .expect("items")
        .remove(0);

    let opened = open_existing(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let hits = match &opened {
        Ok(Some(store)) => Some(store.search(&query(&item.key)).await),
        _ => None,
    };
    built.drop_collection().await.expect("drop");
    let opened = opened.expect("Qdrant answers");
    assert!(opened.is_some(), "an existing collection opens");
    let hits = hits.expect("opened").expect("search");
    assert!(
        hits.iter().any(|h| h.key == item.key),
        "{} not among {hits:?}",
        item.key
    );
}

#[tokio::test]
async fn open_existing_refuses_another_identity() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let other = Renamed {
        inner: HashEmbedder::new(DENSE_DIM),
        identity: EmbedderIdentity {
            model: "test/other".into(),
            ..EmbedderIdentity::hash(DENSE_DIM)
        },
    };
    let refused = open_existing(&url, other, &name).await;
    raw(&url).delete_collection(&name).await.expect("drop");
    let err = refused.expect_err("another embedder is refused");
    assert!(is_embedder_mismatch(&err), "{err}");
}

#[tokio::test]
async fn open_existing_refuses_another_width() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    create_bare(&client, &name, DENSE_DIM as u64 + 1).await;
    let refused = open_existing(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    client.delete_collection(&name).await.expect("drop");
    let err = refused.expect_err("another width is refused");
    assert!(is_embedder_mismatch(&err), "{err}");
}

/// Review R1 M3: whether the collection exists is answered before any embedder exists, so
/// `--search-items` never loads the model for a missing index; the embedder check follows on
/// `open`, as `open_existing` runs it.
#[tokio::test]
async fn an_existing_collection_is_found_before_the_embedder_and_checked_after() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let settings = QdrantSettings::new(
        htui_core::qdrant::QdrantUrl::parse(&url).expect("valid URL"),
        None,
    );
    let missing = ExistingCollection::find_in(&settings, &fresh_name())
        .await
        .expect("Qdrant answers");
    assert!(missing.is_none(), "a missing collection, and no embedder");

    let name = fresh_name();
    let client = raw(&url);
    create_bare(&client, &name, DENSE_DIM as u64 + 1).await;
    let found = ExistingCollection::find_in(&settings, &name).await;
    let opened = match found {
        Ok(Some(found)) => Some(found.open(HashEmbedder::new(DENSE_DIM)).await),
        _ => None,
    };
    client.delete_collection(&name).await.expect("drop");
    let err = opened
        .expect("found")
        .expect_err("another width is refused on open");
    assert!(is_embedder_mismatch(&err), "{err}");
}

/// `connect` would stamp an unstamped collection and build its payload indexes; `open_existing`
/// writes neither.
#[tokio::test]
async fn open_existing_leaves_an_unstamped_collection_unstamped() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    create_unstamped(&client, &name, DENSE_DIM as u64, None).await;
    let opened = open_existing(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let stored = stored_identity(&client, &name).await;
    let indexes = client
        .collection_info(&name)
        .await
        .expect("collection info")
        .result
        .expect("a result")
        .payload_schema
        .len();
    client.delete_collection(&name).await.expect("drop");
    assert!(
        opened.expect("an unstamped collection opens").is_some(),
        "it exists"
    );
    assert_eq!(stored, None, "no stamp is written");
    assert_eq!(indexes, 0, "no payload index is created");
}

// MOD-144 Decision B: the payload layout's version in the collection's metadata.

/// What the collection's `payload_schema` metadata says, if anything.
async fn stored_payload_schema(client: &Qdrant, name: &str) -> Option<serde_json::Value> {
    let info = client
        .collection_info(name)
        .await
        .expect("collection info")
        .result
        .expect("a result");
    info.config
        .expect("a config")
        .metadata
        .remove(PAYLOAD_SCHEMA_KEY)
        .map(serde_json::Value::from)
}

/// How many points the collection holds, counted exactly.
async fn point_count(client: &Qdrant, name: &str) -> u64 {
    client
        .count(CountPointsBuilder::new(name).exact(true))
        .await
        .expect("count")
        .result
        .expect("a result")
        .count
}

/// Records `payload_schema` as `version`, behind `QdrantStore`'s back: Qdrant merges it into the
/// collection's metadata, so the embedder stays.
async fn stamp_payload_schema(client: &Qdrant, name: &str, version: u64) {
    client
        .update_collection(UpdateCollectionBuilder::new(name).metadata(
            std::collections::HashMap::from([(
                PAYLOAD_SCHEMA_KEY.to_owned(),
                serde_json::json!(version),
            )]),
        ))
        .await
        .expect("stamp an old payload schema");
}

#[tokio::test]
async fn a_new_collection_records_the_payload_schema() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let store = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let client = raw(&url);
    let payload = stored_payload_schema(&client, &name).await;
    let identity = stored_identity(&client, &name).await;
    store.drop_collection().await.expect("drop");
    assert_eq!(payload, Some(serde_json::json!(PAYLOAD_SCHEMA)));
    assert_eq!(identity, Some(EmbedderIdentity::hash(DENSE_DIM)));
}

/// A collection filled under an older payload layout (MOD-144: the eight-value `status`) is
/// dropped and created again on `connect`, empty, stamped with this layout and this embedder;
/// the next `Indexer::sync` refills it.
#[tokio::test]
async fn an_old_payload_schema_drops_and_recreates_the_collection() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    let first = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let read = MemStore::demo();
    Indexer::sync(&read, &scope(), &first).await.expect("sync");
    let filled = point_count(&client, &name).await;
    stamp_payload_schema(&client, &name, PAYLOAD_SCHEMA - 1).await;
    let old = stored_payload_schema(&client, &name).await;

    let second = connect(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let emptied = point_count(&client, &name).await;
    let payload = stored_payload_schema(&client, &name).await;
    let identity = stored_identity(&client, &name).await;
    let indexes = client
        .collection_info(&name)
        .await
        .expect("collection info")
        .result
        .expect("a result")
        .payload_schema
        .len();
    let refilled = match &second {
        Ok(store) => Some(Indexer::sync(&read, &scope(), store).await),
        Err(_) => None,
    };
    let after = point_count(&client, &name).await;
    client.delete_collection(&name).await.expect("drop");

    assert!(filled > 0, "control: the first sync wrote points");
    assert_eq!(
        old,
        Some(serde_json::json!(PAYLOAD_SCHEMA - 1)),
        "control: the old stamp is recorded"
    );
    second.expect("an old payload schema is rebuilt, not refused");
    assert_eq!(emptied, 0, "the rebuilt collection starts empty");
    assert_eq!(payload, Some(serde_json::json!(PAYLOAD_SCHEMA)));
    assert_eq!(
        identity,
        Some(EmbedderIdentity::hash(DENSE_DIM)),
        "the embedder is kept"
    );
    assert_eq!(indexes, 5, "the payload indexes are built again");
    refilled.expect("connected").expect("sync");
    assert_eq!(after, filled, "the next sync refills it");
}

/// A payload layout newer than this htui's (MOD-144 R1, M2) was written by a newer htui: `connect`
/// refuses it as an `embedder mismatch` naming both versions, and neither drops nor stamps it.
#[tokio::test]
async fn a_newer_payload_schema_is_refused_and_kept() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    let built = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    Indexer::sync(&MemStore::demo(), &scope(), &built)
        .await
        .expect("sync");
    let filled = point_count(&client, &name).await;
    stamp_payload_schema(&client, &name, PAYLOAD_SCHEMA + 1).await;

    let refused = connect(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let kept = point_count(&client, &name).await;
    let payload = stored_payload_schema(&client, &name).await;
    client.delete_collection(&name).await.expect("drop");

    let Err(err) = refused else {
        panic!("a newer payload schema is refused")
    };
    assert!(is_embedder_mismatch(&err), "{err}");
    assert!(
        err.to_string().contains(&format!(
            "written by a newer htui (payload schema {} > {PAYLOAD_SCHEMA})",
            PAYLOAD_SCHEMA + 1
        )),
        "{err}"
    );
    assert!(filled > 0, "control: the sync wrote points");
    assert_eq!(kept, filled, "nothing was dropped");
    assert_eq!(
        payload,
        Some(serde_json::json!(PAYLOAD_SCHEMA + 1)),
        "nothing was stamped"
    );
}

/// The read-only open writes nothing: under an older payload layout it searches the collection
/// as it is, and neither drops it nor stamps it.
#[tokio::test]
async fn open_existing_never_drops_an_old_payload_schema() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    let built = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    Indexer::sync(&MemStore::demo(), &scope(), &built)
        .await
        .expect("sync");
    let filled = point_count(&client, &name).await;
    stamp_payload_schema(&client, &name, PAYLOAD_SCHEMA - 1).await;

    let opened = open_existing(&url, HashEmbedder::new(DENSE_DIM), &name).await;
    let kept = point_count(&client, &name).await;
    let payload = stored_payload_schema(&client, &name).await;
    client.delete_collection(&name).await.expect("drop");

    assert!(
        opened.expect("Qdrant answers").is_some(),
        "an old payload schema still opens"
    );
    assert!(filled > 0, "control: the sync wrote points");
    assert_eq!(kept, filled, "nothing was dropped");
    assert_eq!(
        payload,
        Some(serde_json::json!(PAYLOAD_SCHEMA - 1)),
        "nothing was stamped"
    );
}

// MOD-160 D3: `ExistingCollection::check`, `open`'s embedder check before any embedder exists.

fn settings(url: &str) -> QdrantSettings {
    QdrantSettings::new(
        htui_core::qdrant::QdrantUrl::parse(url).expect("valid URL"),
        None,
    )
}

/// `check` on the collection `name` against `mine` of width `dim`; `None` when no such collection
/// is found. It never panics, so a caller drops its collection before it asserts.
async fn check(
    url: &str,
    name: &str,
    mine: &EmbedderIdentity,
    dim: usize,
) -> Option<Result<(), StoreError>> {
    match ExistingCollection::find_in(&settings(url), name).await {
        Ok(Some(found)) => Some(found.check(mine, dim).await),
        Ok(None) => None,
        Err(err) => Some(Err(err)),
    }
}

#[tokio::test]
async fn check_refuses_another_width_before_any_embedder() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let built = connect(&url, HashEmbedder::new(8), &name)
        .await
        .expect("Qdrant answers");
    let checked = check(&url, &name, &EmbedderIdentity::hash(DENSE_DIM), DENSE_DIM).await;
    built.drop_collection().await.expect("drop");
    let err = checked
        .expect("the collection exists")
        .expect_err("another width is refused");
    assert!(is_embedder_mismatch(&err), "{err}");
    let err = err.to_string();
    assert!(err.contains("8-wide"), "{err}");
    assert!(err.contains(&format!("{DENSE_DIM}-wide")), "{err}");
}

#[tokio::test]
async fn check_refuses_another_embedder() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let built = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let other = EmbedderIdentity {
        model: "test/other".into(),
        ..EmbedderIdentity::hash(DENSE_DIM)
    };
    let checked = check(&url, &name, &other, DENSE_DIM).await;
    let client = raw(&url);
    let stored = stored_identity(&client, &name).await;
    built.drop_collection().await.expect("drop");
    let err = checked
        .expect("the collection exists")
        .expect_err("another embedder is refused");
    assert!(is_embedder_mismatch(&err), "{err}");
    let err = err.to_string();
    assert!(err.contains("hash/384"), "{err}");
    assert!(err.contains("test/other/384"), "{err}");
    assert_eq!(
        stored,
        Some(EmbedderIdentity::hash(DENSE_DIM)),
        "left as it was"
    );
}

/// `connect` would stamp it and build its payload indexes; `check` reads only.
#[tokio::test]
async fn check_passes_an_unstamped_collection_and_writes_nothing() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    let other = std::collections::HashMap::from([("other".to_owned(), serde_json::json!("kept"))]);
    create_unstamped(&client, &name, DENSE_DIM as u64, Some(other)).await;
    let checked = check(&url, &name, &EmbedderIdentity::hash(DENSE_DIM), DENSE_DIM).await;
    let identity = stored_identity(&client, &name).await;
    let payload = stored_payload_schema(&client, &name).await;
    let info = client
        .collection_info(&name)
        .await
        .expect("collection info")
        .result
        .expect("a result");
    client.delete_collection(&name).await.expect("drop");
    checked
        .expect("the collection exists")
        .expect("an unstamped collection of the right width passes");
    assert_eq!(identity, None, "no stamp is written");
    assert_eq!(payload, None, "no payload layout is written");
    assert_eq!(info.payload_schema.len(), 0, "no payload index is created");
    let kept = info
        .config
        .expect("a config")
        .metadata
        .remove("other")
        .map(serde_json::Value::from);
    assert_eq!(
        kept,
        Some(serde_json::json!("kept")),
        "the metadata is unchanged"
    );
}

/// `connect` would drop and recreate it (MOD-144 Decision B); `check` passes it and drops nothing.
#[tokio::test]
async fn check_passes_an_older_payload_stamp_and_drops_nothing() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let client = raw(&url);
    let built = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    Indexer::sync(&MemStore::demo(), &scope(), &built)
        .await
        .expect("sync");
    let filled = point_count(&client, &name).await;
    stamp_payload_schema(&client, &name, PAYLOAD_SCHEMA - 1).await;

    let checked = check(&url, &name, &EmbedderIdentity::hash(DENSE_DIM), DENSE_DIM).await;
    let kept = point_count(&client, &name).await;
    let payload = stored_payload_schema(&client, &name).await;
    client.delete_collection(&name).await.expect("drop");

    checked
        .expect("the collection exists")
        .expect("an older payload stamp passes read-only");
    assert!(filled > 0, "control: the sync wrote points");
    assert_eq!(kept, filled, "nothing was dropped");
    assert_eq!(
        payload,
        Some(serde_json::json!(PAYLOAD_SCHEMA - 1)),
        "nothing was stamped"
    );
}

// MOD-160 D9: `probe`, the model-free look at Qdrant behind every warning.

#[tokio::test]
async fn probe_of_a_missing_collection_is_no_collection_and_creates_none() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let probed = probe_with(
        &settings(&url),
        &name,
        &EmbedderIdentity::hash(DENSE_DIM),
        DENSE_DIM,
    )
    .await;
    let exists = raw(&url)
        .collection_exists(&name)
        .await
        .expect("collection exists");
    if exists {
        raw(&url).delete_collection(&name).await.expect("drop");
    }
    assert_eq!(probed, Probe::NoCollection);
    assert!(!exists, "the probe created nothing");
}

#[tokio::test]
async fn probe_of_a_matching_collection_is_ready() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let built = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let probed = probe_with(
        &settings(&url),
        &name,
        &HashEmbedder::new(DENSE_DIM).identity(),
        DENSE_DIM,
    )
    .await;
    built.drop_collection().await.expect("drop");
    assert_eq!(probed, Probe::Ready);
}

/// A key no request header can carry, against a Qdrant that answers: qdrant-client's interceptor
/// would echo it ("Malformed API key or token: <the key>"); the probe refuses it unechoed.
#[tokio::test]
async fn probe_with_a_malformed_key_is_refused_and_never_echoes_it() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let settings = QdrantSettings::new(
        htui_core::qdrant::QdrantUrl::parse(&url).expect("valid URL"),
        Some("SENTINEL-KEY-\u{1}".to_owned()),
    );
    let probed = probe_with(
        &settings,
        &fresh_name(),
        &EmbedderIdentity::hash(DENSE_DIM),
        DENSE_DIM,
    )
    .await;
    let Probe::Refused(sentence) = &probed else {
        panic!("a malformed key is refused: {probed:?}");
    };
    assert!(!sentence.contains("SENTINEL"), "{sentence}");
}

/// The pinned model's identity against a `HashEmbedder` collection: a mismatch, told without
/// loading (or downloading) the model.
#[cfg(feature = "local-embed")]
#[tokio::test]
async fn probe_of_another_embedders_collection_is_mismatch() {
    let Some(url) = qdrant_url() else {
        return;
    };
    let name = fresh_name();
    let built = connect(&url, HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    let probed = htui_store::vector::probe(&settings(&url), &name).await;
    built.drop_collection().await.expect("drop");
    let Probe::Mismatch(sentence) = &probed else {
        panic!("another embedder's collection is a mismatch: {probed:?}");
    };
    assert!(sentence.contains("hash/384"), "{sentence}");
    assert!(
        sentence.contains(&format!("collection `{name}`")),
        "{sentence}"
    );
}
