//! The MOD-1 store conformance suite (`htui_core::store::conformance`) run against `PgStore`.
//!
//! The suite is the spec `PgStore` is written to: every case is `MemStore`'s behaviour, asserted
//! through the `WriteStore` trait alone, so the two backends cannot drift (blueprint E.2).
//!
//! Each case gets its **own** database, because a case is free to mint, edit and transition and
//! the next one must still see the untouched fixture. That is twenty `CREATE DATABASE` /
//! `DROP DATABASE` pairs, dropped as the loop goes rather than at the end, so a failure leaves at
//! most one database behind (and `TestDb`'s `Drop` net removes even that one).
//!
//! `conformance::run_all` takes a factory and would fit the loop, but it has nowhere to put the
//! `TestDb` handle a database needs for its drop; `run_case` - which `run_all` itself is written
//! over - lets the loop own the handle and report per case (blueprint E.2, MOD-1 watch item).

use htui_store::testkit as common;

/// The number of cases `crates/htui-core/tests/mem_store.rs` carries, asserted here too so a case
/// added to `CASES` without a Postgres run fails loudly.
/// 96 before MOD-41 and MOD-23's switch case; MOD-41 T1's four fence cases (plan D1) and T7's three
/// executor edit cases (plan D10) make it 104, MOD-51's two `box_probe_spec` cases (plan D7)
/// make it 106, MOD-42 T0's twelve relay cases (plan D1-D5, D12, D13) make it 118, MOD-24's
/// graph-only sweep case (plan D3b) makes it 119, MOD-13 milestone 2's spec-columns case
/// (plan D10) makes it 120, MOD-13 milestone 3's edit-reason case (plan D9) makes it 121,
/// MOD-37's `queued_at`, `phase_agent` and pass/park cases (R-29, R-6, R-5) make it 125,
/// MOD-26 T1's five persona cases (plan D3-D5) make it 130, MOD-26 milestone 2's three delete
/// cases (plan D14) make it 133, MOD-72's tool-call count case (plan D1-D3) makes it 134,
/// MOD-13 milestone 5's hand-written round trip (plan D12) makes it 135, and MOD-37 milestone 5's
/// `run_step.opening` case (R-48) makes it 136.
const EXPECTED_CASES: usize = 136;

#[test]
fn case_list_matches_mem_store() {
    assert_eq!(
        htui_core::store::conformance::CASES.len(),
        EXPECTED_CASES,
        "every conformance case must run against PgStore too (136 since MOD-37 milestone 5's \
         `run_step.opening` case)"
    );
}

#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_conformance() {
    use htui_core::store::conformance;

    for name in conformance::CASES {
        let Some(db) = common::demo_db().await else {
            return; // `common` printed the skip line already (plan D13).
        };
        println!("case {name}");
        conformance::run_case(name, &db.store).await;
        db.drop_db().await;
    }
}

/// MOD-2 milestone 9's read suite (plan D96's `READ_CASES`) run against `PgStore`.
///
/// A second loop rather than six more entries in `CASES`: `run_case` is bound to `WriteStore` and
/// eleven of its cases write, so the bound could not be relaxed without splitting every one of
/// them. `READ_CASES` is additive, and running it here is one half of what makes ANA-5 §12
/// criterion 7's "the same diamond read through `PgStore` and through `CacheStore` renders
/// byte-identically" a test — `cache.rs::the_mirror_passes_the_read_cases` is the other.
///
/// One database per case for `pg_store_conformance`'s reason, even though no read case writes:
/// the loop is the same shape and a read case that grew a write later would not need this file
/// reopened.
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_read_conformance() {
    use htui_core::store::conformance;

    for name in conformance::READ_CASES {
        let Some(db) = common::demo_db().await else {
            return; // `common` printed the skip line already (plan D13).
        };
        println!("read case {name}");
        conformance::run_read_case(name, &db.store).await;
        db.drop_db().await;
    }
}
