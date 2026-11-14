//! The index job against a real Postgres and a real Qdrant (MOD-41 plan D19, blueprint §16,
//! MOD-159 M1 plan D7).
//!
//! Gated on `HTUI_TEST_QDRANT_URL` (the gRPC endpoint, as `htui-store/tests/qdrant_live.rs`) and
//! `HTUI_TEST_DATABASE_URL`; with either unset the case prints the skip line and passes. It lives
//! here rather than beside `qdrant_live.rs` because the job is `htui` code, which an `htui-store`
//! test cannot reach. Each case's collections carry a throwaway prefix before the per-workspace
//! names the job writes (`htui_ws_<uuid>`, `htui_global`), vectors come from `HashEmbedder`, so no
//! model is downloaded and no real index is ever touched; the case drops what it created.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use htui::concepts::{self, JobIndex, JobState};
use htui::concepts_worker::{KeySource, QdrantSource};
use htui_core::fixtures::ids;
use htui_core::model::{IndexedCollection, ItemId, NewItem};
use htui_core::qdrant::QdrantUrl;
use htui_core::store::{IndexerLeaseStore as _, ReadStore as _, WriteStore as _};
use htui_store::embed::{DENSE_DIM, HashEmbedder};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::secret::QdrantEnv;
use htui_store::testkit;
use htui_store::vector::{self, LEGACY_COLLECTION, QdrantStore, SearchQuery, VectorStore as _};

const ENV_URL: &str = "HTUI_TEST_QDRANT_URL";

/// The demo's Platform workspace's collection: the one the htui project is indexed into.
const PLATFORM: IndexedCollection = IndexedCollection::Workspace(ids::WORKSPACE_PLATFORM);

/// The job's index over the test Qdrant: every collection name behind this case's prefix, every
/// open's URL and collection recorded, and the legacy collection the prefixed one.
struct Prefixed {
    prefix: String,
    opened: Mutex<Vec<(String, IndexedCollection)>>,
    stores: Mutex<Vec<(IndexedCollection, Arc<QdrantStore<HashEmbedder>>)>>,
}

impl Prefixed {
    fn new() -> Self {
        Self {
            prefix: format!("htui_test_{}_", uuid::Uuid::now_v7().simple()),
            opened: Mutex::new(Vec::new()),
            stores: Mutex::new(Vec::new()),
        }
    }

    fn name(&self, collection: &IndexedCollection) -> String {
        format!("{}{collection}", self.prefix)
    }

    fn legacy(&self) -> String {
        format!("{}{LEGACY_COLLECTION}", self.prefix)
    }

    fn urls(&self) -> Vec<String> {
        let opened = self.opened.lock().expect("not poisoned");
        opened.iter().map(|(url, _)| url.clone()).collect()
    }

    fn store(&self, collection: IndexedCollection) -> Arc<QdrantStore<HashEmbedder>> {
        let stores = self.stores.lock().expect("not poisoned");
        stores
            .iter()
            .rev()
            .find(|(opened, _)| *opened == collection)
            .map(|(_, store)| Arc::clone(store))
            .unwrap_or_else(|| panic!("{collection} was opened"))
    }

    /// Drops every collection this case's prefix names that was created.
    async fn drop_all(&self, settings: &QdrantSettings) {
        let mut names: Vec<String> = {
            let opened = self.opened.lock().expect("not poisoned");
            opened.iter().map(|(_, c)| self.name(c)).collect()
        };
        names.push(self.legacy());
        names.sort();
        names.dedup();
        for name in names {
            vector::drop_collection_named(settings, &name)
                .await
                .expect("drop");
        }
    }
}

impl JobIndex for Prefixed {
    type Store = QdrantStore<HashEmbedder>;

    async fn open(
        &self,
        settings: QdrantSettings,
        collection: &IndexedCollection,
    ) -> Result<Arc<Self::Store>, String> {
        self.opened
            .lock()
            .expect("not poisoned")
            .push((settings.url.to_string(), *collection));
        let store = testkit::qdrant_connect_to(
            &settings,
            HashEmbedder::new(DENSE_DIM),
            &self.name(collection),
        )
        .await
        .map(Arc::new)
        .map_err(|err| err.to_string())?;
        self.stores
            .lock()
            .expect("not poisoned")
            .push((*collection, Arc::clone(&store)));
        Ok(store)
    }

    fn forget(&self, _: Option<&Arc<Self::Store>>) {}

    fn forget_synced(&self) {}

    async fn drop_legacy(&self, settings: &QdrantSettings) -> Result<bool, String> {
        vector::drop_collection_named(settings, &self.legacy())
            .await
            .map_err(|err| err.to_string())
    }
}

/// The test Qdrant's URL and this case's turn at it (TOOL-16), or the skip line.
async fn test_qdrant() -> Option<(testkit::QdrantTurn, String)> {
    let Ok(url) = std::env::var(ENV_URL) else {
        eprintln!("skipped: {ENV_URL} not set");
        return None;
    };
    Some((testkit::qdrant_turn().await, url))
}

/// The settings source a job reads: `url` as the override when given (a literal, never the
/// process's environment, blueprint P-13), no key, the row read through `store`.
fn source(url: Option<&str>, store: &htui_store::PgStore) -> QdrantSource {
    let source = QdrantSource::new(
        QdrantEnv::new(url.map(str::to_owned), None),
        KeySource::Interactive,
    )
    .with_keyring(|| Ok(None));
    source.publish(Some(Arc::new(store.clone())));
    source
}

/// Whether a search of the htui project for `key` finds the point of that key.
async fn found(store: &QdrantStore<HashEmbedder>, key: &str) -> bool {
    let hits = store
        .search(&SearchQuery {
            text: key.to_owned(),
            projects: vec![ids::PROJECT_HTUI],
            types: Vec::new(),
            statuses: Vec::new(),
            resolutions: Vec::new(),
            limit: 10,
        })
        .await
        .expect("search");
    hits.iter().any(|hit| hit.key == key)
}

/// MOD-159 M1 D7: the job's first beat takes Platform's lease and indexes the htui project into
/// Platform's own collection; a new item in Postgres is in the index after the next reconcile,
/// and not before it.
#[tokio::test]
async fn the_worker_job_indexes_a_new_item_into_its_workspace_collection() {
    let Some((_turn, url)) = test_qdrant().await else {
        return;
    };
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let settings = source(Some(&url), &db.store);
    let index = Prefixed::new();
    let owner = uuid::Uuid::now_v7();

    let pause = concepts::index_beat(
        &db.store,
        &settings,
        &index,
        owner,
        &mut JobState::default(),
    )
    .await;
    assert_eq!(
        pause,
        Duration::from_secs(40),
        "no lease rows in `app_setting`: the default beat"
    );
    let lease = db
        .store
        .indexer_lease(&PLATFORM)
        .await
        .expect("read the lease")
        .expect("the beat took Platform's lease");
    assert!(lease.live, "{lease:?}");
    let store = index.store(PLATFORM);
    assert!(
        !store
            .indexed(ids::PROJECT_HTUI)
            .await
            .expect("indexed")
            .is_empty(),
        "the first beat indexed the demo's htui items into Platform's collection"
    );

    let kind = db
        .store
        .item_kinds(ids::PROJECT_HTUI)
        .await
        .expect("kinds")
        .into_iter()
        .next()
        .expect("the demo's htui project has a kind");
    let item = db
        .store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: kind.id,
            title: "A lighthouse keeps the index job honest".to_owned(),
            body: "Minted after the first beat; only the next reconcile can index it.".to_owned(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("mint the new item");
    assert!(
        !found(&store, &item.key).await,
        "{} is not indexed before the next reconcile",
        item.key
    );

    // A job that never reconciled (a restart) does so at once, under its own lease (`Ours`).
    concepts::index_beat(
        &db.store,
        &settings,
        &index,
        owner,
        &mut JobState::default(),
    )
    .await;
    assert!(
        found(&index.store(PLATFORM), &item.key).await,
        "{} is indexed after one reconcile",
        item.key
    );
    assert!(
        db.store.item(item.id).await.expect("read").is_some(),
        "the job only read Postgres"
    );
    assert_eq!(
        db.store
            .release_indexer_leases(owner)
            .await
            .expect("release"),
        3,
        "Platform's, Graphics' and the global collection's"
    );

    index
        .drop_all(&QdrantSettings::new(
            QdrantUrl::parse(&url).expect("URL"),
            None,
        ))
        .await;
    db.drop_db().await;
}

/// MOD-158 D6, D7: the job reads the `qdrant_url` row through the store published to its source,
/// every beat: with no row it takes no lease and opens nothing; once the row names the test
/// Qdrant, the next beat opens that Qdrant and indexes the demo into it.
#[tokio::test]
async fn the_worker_job_follows_the_url_row() {
    let Some((_turn, url)) = test_qdrant().await else {
        return;
    };
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let settings = source(None, &db.store);
    let index = Prefixed::new();
    let owner = uuid::Uuid::now_v7();
    let mut state = JobState::default();

    concepts::index_beat(&db.store, &settings, &index, owner, &mut state).await;
    assert!(index.urls().is_empty(), "no row: nothing opened");
    assert_eq!(
        db.store.indexer_lease(&PLATFORM).await.expect("read"),
        None,
        "no row: no lease taken"
    );

    let row = QdrantUrl::parse(&url).expect("a valid URL");
    db.store
        .set_qdrant_url(&row, None)
        .await
        .expect("set the row");
    concepts::index_beat(&db.store, &settings, &index, owner, &mut state).await;
    assert!(!index.urls().is_empty(), "the next beat opened");
    assert!(
        index.urls().iter().all(|opened| opened == row.as_str()),
        "the row's Qdrant: {:?}",
        index.urls()
    );
    let indexed = index
        .store(PLATFORM)
        .indexed(ids::PROJECT_HTUI)
        .await
        .expect("indexed");
    index.drop_all(&QdrantSettings::new(row, None)).await;
    assert!(
        !indexed.is_empty(),
        "the beat indexed the demo's htui items"
    );
    db.drop_db().await;
}

/// MOD-159 M1 D10: the global collection's holder creates it and drops the legacy collection on
/// its first reconcile, once: a later beat leaves a legacy collection made since alone.
#[tokio::test]
async fn the_global_holder_drops_the_legacy_collection_once() {
    let Some((_turn, url)) = test_qdrant().await else {
        return;
    };
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let qdrant = QdrantSettings::new(QdrantUrl::parse(&url).expect("URL"), None);
    let settings = source(Some(&url), &db.store);
    let index = Prefixed::new();
    let legacy = |index: &Prefixed| {
        let name = index.legacy();
        let qdrant = qdrant.clone();
        async move {
            QdrantStore::open_existing_to(&qdrant, HashEmbedder::new(DENSE_DIM), &name)
                .await
                .expect("Qdrant answers")
                .is_some()
        }
    };
    testkit::qdrant_connect_to(&qdrant, HashEmbedder::new(DENSE_DIM), &index.legacy())
        .await
        .expect("plant the legacy collection");
    assert!(legacy(&index).await);
    let owner = uuid::Uuid::now_v7();
    let mut state = JobState::default();

    concepts::index_beat(&db.store, &settings, &index, owner, &mut state).await;
    assert!(!legacy(&index).await, "the global holder dropped it");
    assert!(
        QdrantStore::open_existing_to(
            &qdrant,
            HashEmbedder::new(DENSE_DIM),
            &index.name(&IndexedCollection::Global)
        )
        .await
        .expect("Qdrant answers")
        .is_some(),
        "and created the global collection"
    );

    testkit::qdrant_connect_to(&qdrant, HashEmbedder::new(DENSE_DIM), &index.legacy())
        .await
        .expect("plant it again");
    concepts::index_beat(&db.store, &settings, &index, owner, &mut state).await;
    assert!(
        legacy(&index).await,
        "never looked for again in this process"
    );

    index.drop_all(&qdrant).await;
    db.drop_db().await;
}
