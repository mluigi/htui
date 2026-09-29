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
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
use htui_store::embed::{DENSE_DIM, HashEmbedder};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Owner, PointType, QdrantStore, SearchQuery, VectorStore as _};
use htui_store::vector_sync::Indexer;

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
        assert!(hit.resolution.is_some(), "{hit:?}");
    }

    // A resolution the demo's closed items do not have finds none of them.
    let rejected = SearchQuery {
        resolutions: vec![Resolution::Rejected],
        ..decisions
    };
    assert!(store.search(&rejected).await.expect("search").is_empty());
    store.drop_collection().await.expect("drop");
}
