//! Runs the store conformance suite (blueprint B.9) over `MemStore`.
//!
//! The suite itself never names a concrete store: this file is the only place that binds the
//! cases of `docs/ANA-9.md` §4.1 / §4.2 / §7 to `MemStore`, so MOD-6 adds one more file like it
//! for `PgStore` and inherits the same seed data.
#![cfg(feature = "test-support")]

use htui_core::store::MemStore;
use htui_core::store::conformance;

#[tokio::test]
async fn mem_store_conformance() {
    conformance::run_all(|| async { MemStore::demo() }).await;
}

#[tokio::test]
async fn demo_store_loads_the_fixture() {
    let store = MemStore::demo();
    assert_eq!(
        store.item_count(),
        13,
        "the §G fixture holds thirteen items"
    );
    assert_eq!(conformance::CASES.len(), 13, "B.9 names thirteen cases");
}
