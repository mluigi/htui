//! The worker's index job against a real Postgres and a real Qdrant (MOD-41 plan D19, blueprint
//! §16).
//!
//! Gated on `HTUI_TEST_QDRANT_URL` (the gRPC endpoint, as `htui-store/tests/qdrant_live.rs`) and
//! `HTUI_TEST_DATABASE_URL`; with either unset the case prints the skip line and passes. It lives
//! here rather than beside `qdrant_live.rs` because the job is `htui` code, which an `htui-store`
//! test cannot reach. The collection is a throwaway one and vectors come from `HashEmbedder`, so
//! no model is downloaded and the user's `concepts` collection is never touched.

use std::time::Duration;

use htui_core::fixtures::ids;
use htui_core::model::{ItemId, NewItem};
use htui_core::store::{ReadStore as _, WriteStore as _};
use htui_store::embed::{DENSE_DIM, HashEmbedder};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::testkit;
use htui_store::vector::{QdrantStore, SearchQuery, VectorStore as _};

const ENV_URL: &str = "HTUI_TEST_QDRANT_URL";

async fn throwaway() -> Option<QdrantStore<HashEmbedder>> {
    let Ok(url) = std::env::var(ENV_URL) else {
        eprintln!("skipped: {ENV_URL} not set");
        return None;
    };
    let settings = QdrantSettings::new(url, None).expect("valid URL");
    let name = format!("htui_test_{}", uuid::Uuid::now_v7().simple());
    Some(
        QdrantStore::connect_to(&settings, HashEmbedder::new(DENSE_DIM), &name)
            .await
            .expect("Qdrant answers"),
    )
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

/// A new item in Postgres is in the index after the job's next cycle, and not before it.
#[tokio::test]
async fn the_worker_job_indexes_a_new_item() {
    let Some(store) = throwaway().await else {
        return;
    };
    let Some(db) = testkit::demo_db().await else {
        store.drop_collection().await.expect("drop");
        return;
    };

    let pause = htui::concepts::index_cycle(&db.store, &store).await;
    assert_eq!(
        pause,
        Duration::from_secs(15 * 60),
        "no `concepts_sync_minutes` row: the default interval"
    );
    assert!(
        !store
            .indexed(ids::PROJECT_HTUI)
            .await
            .expect("indexed")
            .is_empty(),
        "the start cycle indexed the demo's htui items"
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
            body: "Minted after the first cycle; only the next one can index it.".to_owned(),
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
        "{} is not indexed before the next cycle",
        item.key
    );

    htui::concepts::index_cycle(&db.store, &store).await;
    assert!(
        found(&store, &item.key).await,
        "{} is indexed after one cycle",
        item.key
    );
    assert!(
        db.store.item(item.id).await.expect("read").is_some(),
        "the job only read Postgres"
    );

    store.drop_collection().await.expect("drop");
    db.drop_db().await;
}
