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
const EXPECTED_CASES: usize = 20;

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
