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
const EXPECTED_CASES: usize = 83;

#[test]
fn case_list_matches_mem_store() {
    assert_eq!(
        htui_core::store::conformance::CASES.len(),
        EXPECTED_CASES,
        "every conformance case must run against PgStore too"
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
