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

/// The read-only half of the suite (plan D96), bound to the same fixture.
///
/// A second binding rather than more cases in the first: `run_all` is generic over `WriteStore`,
/// and the point of `READ_CASES` is that its harness is generic over `ReadStore`, so MOD-2
/// milestone 9's `CacheStore` can be a target too. Running it here is what makes `MemStore` the
/// reference those two are compared against.
#[tokio::test]
async fn mem_store_read_conformance() {
    conformance::run_all_reads(|| async { MemStore::demo() }).await;
}

#[tokio::test]
async fn demo_store_loads_the_fixture() {
    let store = MemStore::demo();
    assert_eq!(
        store.item_count(),
        13,
        "the §G fixture holds thirteen items"
    );
    assert_eq!(
        conformance::CASES.len(),
        68,
        "B.9's fifteen cases, MOD-2's five store-seam cases (plan D3), milestone 7's two quota \
         cases (plans D67 and D74), milestone 9's `set_step_prompt`, MOD-15 milestone 1's \
         twelve, one per entity group (plan D12), milestone 2's seed case (plan D7), MOD-4 \
         milestone 1's eleven for the run seam (plan D12), MOD-4 milestone 2's one for \
         `finish_run` (plan D7), MOD-4 milestone 3's one for `record_command_run` (plan D31), \
         MOD-4 milestone 5's four: the isolation and path rules (plan D83), `take_lease` \
         (plan D87), `interrupt_step` (plan D89) and `release_lease` (plan D139), MOD-7 \
         milestone 1's three for the box probe writer (plan D10), MOD-38's two close-out cases \
         (plan D4) and ten requirement cases (plan D9-D14)"
    );
    assert_eq!(
        conformance::READ_CASES.len(),
        14,
        "milestone 9's four `ReadStore` additions, the upstream walk taking three cases (D96), \
         MOD-4 milestone 1's three for ANA-2 §8's mirrored reads (plan D12), and MOD-38's five \
         requirement reads (plan D12)"
    );
}
