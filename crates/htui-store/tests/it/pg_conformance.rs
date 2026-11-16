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
/// MOD-13 milestone 5's hand-written round trip (plan D12) makes it 135, MOD-37 milestone 5's
/// `run_step.opening` case (R-48) makes it 136, MOD-11 T1's six fenced-write cases (plan D13, B-4)
/// make it 142, and MOD-11 T8's four command-queue cases (plan D14) make it 146, and MOD-69's two waiting-list
/// cases (plan D2, D4) make it 148, MOD-70 T0's nine follow-up cases (plan D1-D5, D9) make it 157
/// and MOD-78's `lease_holds` case (plan D1, D2) makes it 158.
/// and MOD-10 M4's secret-column case (plan D7) makes it 159,
/// and MOD-12 M2's five queue-setting cases (plan D9) make it 164,
/// and MOD-125 M1's `create_run_records_its_workspace` (plan D3) makes it 165,
/// and MOD-125 M1's four stop-verb cases (plan D4, D7) make it 169,
/// and MOD-125 M1's four claim-gate cases (plan D5) make it 173,
/// and MOD-125 M1's settlement case (plan D6) makes it 174,
/// and MOD-125 M1's deleted-workspace case (blueprint F16) makes it 175,
/// and MOD-137 T3's eighteen graph v2 cases (plan D3-D11, blueprint §T3.6) make it 193,
/// and MOD-137 review round 1's kept-node slot case (plan D10) makes it 194,
/// and MOD-146's two project type cases (plan D7) make it 196,
/// and MOD-163 M1's ten memory cases (plan D4, D6, D8) make it 206,
/// and MOD-138 M1's four walk-validator cases (plan D9) make it 210,
/// and MOD-138 M1's five per-row default-edge cases (plan D7) make it 215,
/// and three for D12's usable graphs (plan D12, blueprint A3/A4) make it 218,
/// and MOD-138 M1 review R1's kept-kind-graph case (A7) makes it 219,
/// and MOD-144's nine item-status cases (plan "The rules", blueprint §T1.5) make it 228,
/// and MOD-144 R1's `block_if_idle` case (M1) makes it 229,
/// and MOD-138 M2's closed-`if`-set case (plan D4, W4) makes it 230,
/// and MOD-138 M2's three `record_branch` cases (plan D5, Q-A) make it 233,
/// and MOD-138 M3's W5 mixed-`asks_you` case (plan D8, Q-3) makes it 234,
/// and MOD-147 M1's fourteen scope-row and owner-document cases (plan D1-D9, A1, A2) make it 248,
/// and MOD-147 M2's seven key, reach, prefix, graph, run-workspace and upstream cases (plan D2-D9)
/// make it 255,
/// and MOD-139 M1's nine divided fan-out record cases (plan D1, D4-D6) make it 264,
/// and MOD-139 M1 F1's cross-run parent case makes it 265,
/// and MOD-139 M2's nine admission, abandonment and part-input cases (plan D9, D12) make it 274,
/// and MOD-141 M2's three judge and deadline cases (plan D1, D2) make it 277,
/// and MOD-150 M1's seven link-kind cases (plan D2-D7) make it 284,
/// and MOD-125 M2's three `pending_stops` cases (plan D1), its M-3 case (plan D9) and its L-3 case
/// (plan D10) make it 289,
/// and MOD-125 M2 R1's cancel-and-stop case (review M-1) makes it 290,
/// and MOD-152 T1's three command-log cases (plan D1, D5) make it 293,
/// and MOD-125 M3's five resume and stopped-run cancel cases (plan D1, D2) make it 298,
/// and MOD-164's six capture-write and two usage-tally cases (plan D4, D5) make it 306,
/// and MOD-164 F2's person-amended run entry (M1) and evidence (L4) cases make it 308,
/// and MOD-150 M2's four close-out link cases (plan D2, D3) make it 312,
/// and MOD-150 M3's five document link cases (plan D1-D8) make it 317,
/// and MOD-149's seven key, nesting, refinement and coverage cases (plan D2-D9) make it 324,
/// and MOD-125 M4's `workspace_live_runs` case (plan D4) makes it 325,
/// and MOD-133 M1's seven integration cases (plan D2, D4, D6, D7) make it 332,
/// and MOD-130 M1 T1's seeded local target case (D2, P2) makes it 333,
/// and MOD-130 M1 T2's four create, edit and bind target cases (P4-P6, P9) make it 337,
/// and MOD-130 M1 T3's item target spec field case (D11) makes it 338,
/// and MOD-130 M1 T4's five level, default, retire and delete cases (D3, D5, D11, D14) make it
/// 343,
/// and MOD-130 M2 T2's three recorded-target cases (D7, P7-P9) make it 346,
/// and MOD-133 M2's three landing-state, refused-waiting-row and retry cases (E1-E5) make it 349,
/// and MOD-130 M3 T1's driving-box case (D8, P6) makes it 350,
/// and MOD-130 M3 T2's three driver-keyed claim cases (D8, D9, P4) make it 353,
/// and MOD-133 M3's five pushed `pull_request` record cases (plan D5) make it 358,
/// and MOD-127 M1's four workspace and item-kind attachment cases (D2, D3, D6) make it 362,
/// and MOD-130 M4 T1's Runs-read target case (D7, P2) makes it 363,
/// and MOD-127 M2 T3's six MCP server and attachment cases (D2-D4) make it 369.
const EXPECTED_CASES: usize = 369;

#[test]
fn case_list_matches_mem_store() {
    assert_eq!(
        htui_core::store::conformance::CASES.len(),
        EXPECTED_CASES,
        "every conformance case must run against PgStore too (369 since MOD-78's `lease_holds` \
         case, MOD-10 M4's secret-column case, MOD-12 M2's five queue-setting cases and \
         MOD-125 M1's run-workspace and four stop-verb and four claim-gate cases and its \
         settlement and deleted-workspace cases, MOD-137's eighteen graph v2 cases and its \
         review round's kept-node slot case, MOD-146's two project type cases, MOD-163 M1's \
         ten memory cases, MOD-138 M1's four walk-validator and five per-row default-edge \
         cases, three for D12's usable graphs and the review round's kept-kind-graph case, and \
         MOD-144's nine item-status cases and its R1 `block_if_idle` case, and MOD-138 M2's \
         closed-`if`-set case and three `record_branch` cases, MOD-138 M3's W5 \
         mixed-`asks_you` case, MOD-147 M1's fourteen scope-row and owner-document cases, \
         MOD-147 M2's seven key, reach, prefix, graph, run-workspace and upstream cases, and \
         MOD-139 M1's nine divided fan-out record cases and its F1 cross-run parent case, and \
         MOD-139 M2's nine admission, abandonment and part-input cases, and MOD-141 M2's \
         three judge and deadline cases, and MOD-150 M1's seven link-kind cases, and MOD-125 M2's \
         three `pending_stops` cases, its M-3 case and its L-3 case and its R1 cancel-and-stop \
         case, and MOD-152 T1's three command-log cases, and MOD-125 M3's five resume and \
         stopped-run cancel cases, and MOD-164's six capture-write and two usage-tally cases and its F2 person-amended and evidence cases, and MOD-150 M2's four close-out link cases, and MOD-150 M3's five document link cases, and MOD-149's seven key, nesting, \
             refinement and coverage cases, and MOD-125 M4's `workspace_live_runs` case, and MOD-133 M1's seven integration \
             cases, and MOD-130 M1 T1's seeded local target case, and MOD-130 \
             M1 T2's four target cases, and MOD-130 M1 T3's item target case, and MOD-130 M1 \
             T4's five level, default, retire and delete cases, and MOD-130 M2 T2's three \
             recorded-target cases, and MOD-133 M2's three landing, refusal and retry cases, and \
             MOD-130 M3 T1's driving-box case, and MOD-130 M3 T2's three driver-keyed claim \
             cases, and MOD-133 M3's five pushed `pull_request` record cases, and MOD-127 M1's four \
             attachment level cases, and MOD-130 M4 T1's one Runs-read target case, and MOD-127 \
             M2 T3's six MCP server and attachment cases)"
    );
}

/// How many tests `CASES` is dealt across (TOOL-16). One test running all 290 cases was the
/// longest test of the crate, so it set the wall time of every parallel run; dealt round-robin
/// across four, the cases run side by side, each still on its own database.
#[cfg(feature = "demo")]
const SHARDS: usize = 4;

/// Runs every `SHARDS`-th case of `CASES`, starting at `shard`.
#[cfg(feature = "demo")]
async fn pg_store_conformance_shard(shard: usize) {
    use htui_core::store::conformance;

    for name in conformance::CASES.iter().skip(shard).step_by(SHARDS) {
        // Boxed (MOD-137): `demo_db`'s create, connect and `load_demo` future is otherwise inline
        // in this test's future, on the 2 MiB test thread's stack, under every case's poll.
        let Some(db) = Box::pin(common::demo_db()).await else {
            return; // `common` printed the skip line already (plan D13).
        };
        println!("case {name}");
        conformance::run_case(name, &db.store).await;
        db.drop_db().await;
    }
}

#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_conformance_shard_0() {
    pg_store_conformance_shard(0).await;
}

#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_conformance_shard_1() {
    pg_store_conformance_shard(1).await;
}

#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_conformance_shard_2() {
    pg_store_conformance_shard(2).await;
}

#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_conformance_shard_3() {
    pg_store_conformance_shard(3).await;
}

/// MOD-2 milestone 9's read suite (plan D96's `READ_CASES`) run against `PgStore`.
///
/// A second loop rather than six more entries in `CASES`: `run_case` is bound to `WriteStore` and
/// eleven of its cases write, so the bound could not be relaxed without splitting every one of
/// them. `READ_CASES` is additive, and running it here is one half of what makes ANA-5 §12
/// criterion 7's "the same diamond read through `PgStore` and through `CacheStore` renders
/// byte-identically" a test — `cache.rs::the_mirror_passes_the_read_cases` is the other.
///
/// One database per case for the write cases' reason, even though no read case writes:
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

/// MOD-122 M2's worker log suite (`WORKER_LOG_CASES`, blueprint F1) run against `PgStore`.
///
/// A third loop for `pg_store_read_conformance`'s reason: `run_worker_log_case` is bound to
/// `WriteStore + WorkerLogStore`, a narrow trait only the two stores implement. Every case
/// writes, so each gets its own database, boxed as `pg_store_conformance`'s are (MOD-137).
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_worker_log_conformance() {
    use htui_core::store::conformance;

    assert_eq!(
        conformance::WORKER_LOG_CASES.len(),
        5,
        "MOD-122 M2's five worker log cases, as `mem_store.rs` pins them"
    );
    for name in conformance::WORKER_LOG_CASES {
        let Some(db) = Box::pin(common::demo_db()).await else {
            return; // `common` printed the skip line already (plan D13).
        };
        println!("worker log case {name}");
        conformance::run_worker_log_case(name, &db.store).await;
        db.drop_db().await;
    }
}

/// MOD-122 M3's activity suite (`ACTIVITY_CASES`, blueprint F1) run against `PgStore`.
///
/// A fourth loop for the worker log loop's reason: `run_activity_case` is bound to
/// `WriteStore + ActivityStore`. Every case writes, so each gets its own database. On Postgres
/// `run_park_kinds` is what pins the plpgsql park rule of `0036_activity_event.sql` against
/// `park_kind` (F1), and every case first asserts that `load_demo` wrote no event (D13).
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_activity_conformance() {
    use htui_core::store::conformance;

    assert_eq!(
        conformance::ACTIVITY_CASES.len(),
        12,
        "MOD-122 M3's twelve activity cases, as `mem_store.rs` pins them"
    );
    for name in conformance::ACTIVITY_CASES {
        let Some(db) = Box::pin(common::demo_db()).await else {
            return; // `common` printed the skip line already (plan D13).
        };
        println!("activity case {name}");
        conformance::run_activity_case(name, &db.store).await;
        db.drop_db().await;
    }
}

/// MOD-159 M1's indexer lease suite (`INDEXER_LEASE_CASES`, plan D2, D3) run against `PgStore`.
///
/// A fourth loop for `pg_store_worker_log_conformance`'s reason: `run_indexer_lease_case` is bound
/// to `WriteStore + IndexerLeaseStore`, a narrow trait only the two stores implement. Every case
/// writes, so each gets its own database, boxed as `pg_store_conformance`'s are (MOD-137).
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_indexer_lease_conformance() {
    use htui_core::store::conformance;

    assert_eq!(
        conformance::INDEXER_LEASE_CASES.len(),
        15,
        "MOD-159 M1's ten indexer lease cases and M3's five pass-request cases, as `mem_store.rs` \
         pins them"
    );
    for name in conformance::INDEXER_LEASE_CASES {
        let Some(db) = Box::pin(common::demo_db()).await else {
            return; // `common` printed the skip line already (plan D13).
        };
        println!("indexer lease case {name}");
        conformance::run_indexer_lease_case(name, &db.store).await;
        db.drop_db().await;
    }
}

/// MOD-159 M2's change read suite (`INDEX_CHANGES_CASES`, plan D3, D4) run against `PgStore`.
///
/// A fifth loop for `pg_store_indexer_lease_conformance`'s reason: `run_index_changes_case` is
/// bound to `WriteStore + IndexChangesStore`, a narrow trait only the two stores implement. Every
/// case writes, so each gets its own database, boxed as `pg_store_conformance`'s are (MOD-137).
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_store_index_changes_conformance() {
    use htui_core::store::conformance;

    assert_eq!(
        conformance::INDEX_CHANGES_CASES.len(),
        7,
        "MOD-159 M2's seven change-read cases, as `mem_store.rs` pins them"
    );
    for name in conformance::INDEX_CHANGES_CASES {
        let Some(db) = Box::pin(common::demo_db()).await else {
            return; // `common` printed the skip line already (plan D13).
        };
        println!("index changes case {name}");
        conformance::run_index_changes_case(name, &db.store).await;
        db.drop_db().await;
    }
}

/// MOD-159 M1 plan D3 (review R1): eight takes of one collection at once, by distinct owners on
/// one shared box or boxes of their own, have exactly one winner, on a collection with no row
/// (the `INSERT ... ON CONFLICT` race) and on one lapsed under another owner (the `UPDATE` race,
/// where `TAKE_ATTEMPTS` re-reads). The seven others are `Held`, naming the winner's box as live,
/// and only the winner's renewal holds. Postgres only: `MemStore` takes under one lock, so there
/// is no race to lose, and it is not a conformance case.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn pg_store_indexer_lease_concurrent_takes_have_one_winner() {
    use chrono::TimeDelta;
    use futures::future::join_all;
    use htui_core::model::{BoxId, IndexedCollection, LeaseTake, WorkspaceId};
    use htui_core::store::IndexerLeaseStore;
    use uuid::Uuid;

    const TAKERS: usize = 8;
    const ROUNDS: usize = 5;
    let Some(db) = Box::pin(common::fresh_db()).await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let store = &db.store;
    let ttl = TimeDelta::minutes(5);
    let shared = BoxId::new();
    for round in 0..ROUNDS {
        for lapsed in [false, true] {
            let collection = IndexedCollection::Workspace(WorkspaceId::new());
            let case = format!("round {round}, lapsed: {lapsed}");
            if lapsed {
                assert_eq!(
                    store
                        .take_indexer_lease(
                            &collection,
                            BoxId::new(),
                            Uuid::now_v7(),
                            TimeDelta::zero()
                        )
                        .await
                        .expect("the earlier owner's take"),
                    LeaseTake::Taken,
                    "{case}"
                );
            }
            let takers: Vec<(BoxId, Uuid)> = (0..TAKERS)
                .map(|i| {
                    let box_id = if i % 2 == 0 { shared } else { BoxId::new() };
                    (box_id, Uuid::now_v7())
                })
                .collect();
            let answers =
                join_all(takers.iter().map(|&(box_id, owner)| {
                    store.take_indexer_lease(&collection, box_id, owner, ttl)
                }))
                .await;
            let answers: Vec<LeaseTake> = answers
                .into_iter()
                .map(|answer| answer.expect("a concurrent take"))
                .collect();
            let winners: Vec<usize> = (0..TAKERS)
                .filter(|&i| answers[i] == LeaseTake::Taken)
                .collect();
            assert_eq!(winners.len(), 1, "{case}: one winner: {answers:?}");
            let winner = winners[0];
            for (i, answer) in answers.iter().enumerate().filter(|&(i, _)| i != winner) {
                let LeaseTake::Held(held) = answer else {
                    panic!("{case}: taker {i} lost and is Held, got {answer:?}");
                };
                assert_eq!(
                    held.box_id,
                    Some(takers[winner].0),
                    "{case}: taker {i}'s Held names the winner's box"
                );
                assert!(held.live, "{case}: the winner holds it live: {held:?}");
                assert_eq!(held.collection, collection.name(), "{case}");
            }
            for (i, &(_, owner)) in takers.iter().enumerate() {
                assert_eq!(
                    store
                        .renew_indexer_lease(&collection, owner, ttl)
                        .await
                        .expect("a renewal"),
                    i == winner,
                    "{case}: only the winner's renewal holds (taker {i})"
                );
            }
        }
    }
    db.drop_db().await;
}

/// MOD-133 D7, Postgres only: the two readiness sites the conformance suite does not reach, the
/// inherent `ready_items` and `queue_rows`' open-blocker list, release a `blocked_by` target whose
/// latest graph run is `done` only when every `integration` record of that run is `landed` or
/// `pr_merged`. The fixture's `agy` FIX-1 is blocked by `agy` ANA-1, whose latest graph run
/// (`RUN_7`) is `done`: ready with no record, held by a `pending`, `refused`, `pr_open` or
/// `pr_closed` one, released again once it is `landed` or `pr_merged`. `items(ready)` agrees.
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_ready_items_and_queue_rows_wait_for_landed_integrations() {
    use chrono::{SubsecRound as _, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{ItemFilter, Scope, TIMESTAMPTZ_DIGITS};
    use htui_core::store::ReadStore as _;

    let Some(db) = Box::pin(common::demo_db()).await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let pg = &db.store;
    let at = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
    let scope = Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
    };
    pg.queue_item(ids::AGY_FIX_1, ids::BOX, None, ids::USER, at)
        .await
        .expect("queue agy FIX-1");
    let blocker: String = sqlx::query_scalar("SELECT key FROM item WHERE id = $1")
        .bind(ids::AGY_ANA_1.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read agy ANA-1's key");
    let repo: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO repo (project_id, name, remote_url) \
         VALUES ($1, 'landed', 'git@example.invalid:landed.git') RETURNING id",
    )
    .bind(ids::PROJECT_AGY.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("plant a repo");

    let observe = async |case: &str| -> (bool, bool, Vec<String>) {
        let ready = pg
            .ready_items(&scope, ids::BOX)
            .await
            .expect(case)
            .iter()
            .any(|row| row.id == ids::AGY_FIX_1);
        let listed = pg
            .items(
                &scope,
                &ItemFilter {
                    ready: Some(true),
                    ..ItemFilter::default()
                },
            )
            .await
            .expect(case)
            .iter()
            .any(|row| row.id == ids::AGY_FIX_1);
        let blockers = pg
            .queue_rows(ids::BOX)
            .await
            .expect(case)
            .into_iter()
            .find(|row| row.entry.item_id == ids::AGY_FIX_1)
            .unwrap_or_else(|| panic!("{case}: agy FIX-1 is on the box's queue"))
            .open_blockers;
        (ready, listed, blockers)
    };

    assert_eq!(
        observe("no record").await,
        (true, true, Vec::new()),
        "a `done` blocker run with no record (`local`) releases, as before MOD-133"
    );
    sqlx::query(
        "INSERT INTO integration (run_id, repo_id, project_id, mode, remote, target_branch) \
         VALUES ($1, $2, $3, 'push', 'git@example.invalid:landed.git', 'main')",
    )
    .bind(ids::RUN_7.as_uuid())
    .bind(repo)
    .bind(ids::PROJECT_AGY.as_uuid())
    .execute(&db.pool)
    .await
    .expect("a pending record of RUN_7");
    for (state, refusal, released) in [
        ("pending", None, false),
        (
            "refused",
            Some("non_fast_forward: main on the remote has moved"),
            false,
        ),
        ("pr_open", None, false),
        ("pr_closed", None, false),
        ("landed", None, true),
        ("pr_merged", None, true),
    ] {
        sqlx::query("UPDATE integration SET state = $2, refusal = $3 WHERE run_id = $1")
            .bind(ids::RUN_7.as_uuid())
            .bind(state)
            .bind(refusal)
            .execute(&db.pool)
            .await
            .unwrap_or_else(|err| panic!("move the record to {state}: {err}"));
        let expected = if released {
            (true, true, Vec::new())
        } else {
            (false, false, vec![blocker.clone()])
        };
        assert_eq!(
            observe(state).await,
            expected,
            "a `{state}` record {} its dependent",
            if released { "releases" } else { "holds" }
        );
    }

    db.drop_db().await;
}

/// MOD-133 M2 E3, Postgres only: the two landing carriers the conformance suite does not reach,
/// the inherent `queue_rows` and `upstream_summaries`, carry the landing of the item's latest
/// graph run. The fixture's `agy` ANA-1 has a `done` latest graph run (`RUN_7`) and `agy` FIX-1
/// is `blocked_by` it: no record reads `done`, a `pending` one `landing`, a `refused` one beside
/// it `landing refused` (refused wins), every record landed `done` again.
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn pg_queue_rows_and_upstream_carry_the_latest_runs_landing() {
    use chrono::{SubsecRound as _, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{ItemState, PromptScope, RunLanding, TIMESTAMPTZ_DIGITS};
    use htui_core::store::ReadStore as _;

    let Some(db) = Box::pin(common::demo_db()).await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let pg = &db.store;
    let at = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
    pg.queue_item(ids::AGY_ANA_1, ids::BOX, None, ids::USER, at)
        .await
        .expect("queue agy ANA-1");
    let mut repos = Vec::new();
    for name in ["landing-a", "landing-b"] {
        let repo: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO repo (project_id, name, remote_url) \
             VALUES ($1, $2, 'git@example.invalid:landing.git') RETURNING id",
        )
        .bind(ids::PROJECT_AGY.as_uuid())
        .bind(name)
        .fetch_one(&db.pool)
        .await
        .expect("plant a repo");
        repos.push(repo);
    }
    let scope = PromptScope {
        workspace: Some(ids::WORKSPACE_PLATFORM),
        project: ids::PROJECT_AGY,
    };

    let observe = async |case: &str| -> [(Option<RunLanding>, ItemState); 2] {
        let row = pg
            .queue_rows(ids::BOX)
            .await
            .expect(case)
            .into_iter()
            .find(|row| row.entry.item_id == ids::AGY_ANA_1)
            .unwrap_or_else(|| panic!("{case}: agy ANA-1 is on the box's queue"));
        let entry = pg
            .upstream_summaries(ids::AGY_FIX_1, 1, &scope)
            .await
            .expect(case)
            .into_iter()
            .find(|entry| entry.item_id == ids::AGY_ANA_1)
            .unwrap_or_else(|| panic!("{case}: agy ANA-1 is upstream of agy FIX-1"));
        [
            (row.latest_run_landing, row.state()),
            (entry.latest_run_landing, entry.state()),
        ]
    };

    assert_eq!(
        observe("no record").await,
        [(None, ItemState::Done); 2],
        "a `done` run with no record (`local`) reads `done`"
    );
    for (states, landing, state) in [
        (
            ["pending", "pending"],
            Some(RunLanding::Pending),
            ItemState::Landing,
        ),
        (
            ["pending", "refused"],
            Some(RunLanding::Refused),
            ItemState::LandRefused,
        ),
        (
            ["landed", "refused"],
            Some(RunLanding::Refused),
            ItemState::LandRefused,
        ),
        (["landed", "pr_merged"], None, ItemState::Done),
    ] {
        sqlx::query("DELETE FROM integration WHERE run_id = $1")
            .bind(ids::RUN_7.as_uuid())
            .execute(&db.pool)
            .await
            .expect("clear RUN_7's records");
        for (repo, record) in repos.iter().zip(states) {
            sqlx::query(
                "INSERT INTO integration \
                     (run_id, repo_id, project_id, mode, remote, target_branch, state, refusal) \
                 VALUES ($1, $2, $3, 'push', 'git@example.invalid:landing.git', 'main', $4, \
                         CASE WHEN $4 = 'refused' THEN 'non_fast_forward: main moved' END)",
            )
            .bind(ids::RUN_7.as_uuid())
            .bind(repo)
            .bind(ids::PROJECT_AGY.as_uuid())
            .bind(record)
            .execute(&db.pool)
            .await
            .unwrap_or_else(|err| panic!("a {record} record of RUN_7: {err}"));
        }
        assert_eq!(
            observe(&format!("{states:?}")).await,
            [(landing, state); 2],
            "records {states:?}"
        );
    }

    db.drop_db().await;
}

/// MOD-133 D6, Postgres only: eight `take_integrations` at once, by distinct owners of the box
/// the run executed on, lease each `pending` record to exactly one of them (the compare-and-set
/// re-checks the lease after a row-lock wait), and only that owner settles it. `MemStore` takes
/// under one lock, so there is no race to lose, and it is not a conformance case.
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn pg_concurrent_integration_takes_lease_each_record_once() {
    use chrono::TimeDelta;
    use futures::future::join_all;
    use htui_core::fixtures::ids;
    use htui_core::model::{LandOutcome, RepoId, RunId};
    use htui_core::store::WriteStore as _;
    use uuid::Uuid;

    const TAKERS: usize = 8;
    const RECORDS: usize = 3;
    let Some(db) = Box::pin(common::demo_db()).await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let pg = &db.store;
    sqlx::query("UPDATE run SET executing_box_id = $2 WHERE id = $1")
        .bind(ids::RUN_7.as_uuid())
        .bind(ids::BOX.as_uuid())
        .execute(&db.pool)
        .await
        .expect("RUN_7 executed on the fixture box");
    let mut planted: Vec<(RunId, RepoId)> = Vec::new();
    for index in 0..RECORDS {
        let repo: Uuid = sqlx::query_scalar(
            "INSERT INTO repo (project_id, name, remote_url) \
             VALUES ($1, $2, 'git@example.invalid:raced.git') RETURNING id",
        )
        .bind(ids::PROJECT_AGY.as_uuid())
        .bind(format!("raced-{index}"))
        .fetch_one(&db.pool)
        .await
        .expect("plant a repo");
        sqlx::query(
            "INSERT INTO integration (run_id, repo_id, project_id, mode, remote, target_branch) \
             VALUES ($1, $2, $3, 'push', 'git@example.invalid:raced.git', 'main')",
        )
        .bind(ids::RUN_7.as_uuid())
        .bind(repo)
        .bind(ids::PROJECT_AGY.as_uuid())
        .execute(&db.pool)
        .await
        .expect("plant a pending record");
        planted.push((ids::RUN_7, RepoId(repo)));
    }
    planted.sort_unstable();

    let owners: Vec<Uuid> = (0..TAKERS).map(|_| Uuid::now_v7()).collect();
    let answers = join_all(
        owners
            .iter()
            .map(|&owner| pg.take_integrations(ids::BOX, owner, TimeDelta::minutes(5), None)),
    )
    .await;
    let mut taken: Vec<((RunId, RepoId), Uuid)> = Vec::new();
    for (answer, owner) in answers.into_iter().zip(&owners) {
        for row in answer.expect("a concurrent take") {
            taken.push(((row.run_id, row.repo_id), *owner));
        }
    }
    taken.sort_unstable();
    assert_eq!(
        taken.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        planted,
        "every record is leased, and to one owner only: {taken:?}"
    );
    let landed = LandOutcome::Landed {
        hash: "4444444444444444444444444444444444444444".to_owned(),
    };
    for ((run, repo), holder) in &taken {
        for owner in &owners {
            if owner != holder {
                assert!(
                    !pg.settle_integration(*run, *repo, *owner, &landed)
                        .await
                        .expect("a settle"),
                    "only the lease's holder settles {repo}"
                );
            }
        }
        assert!(
            pg.settle_integration(*run, *repo, *holder, &landed)
                .await
                .expect("a settle"),
            "the holder settles {repo}"
        );
    }

    db.drop_db().await;
}

/// MOD-133 fix round F1: plants `count` `pending` push records of `RUN_7`, which it marks as
/// executed on the fixture box, each on a repo of its own, and returns their keys in order.
#[cfg(feature = "demo")]
async fn plant_box_integrations(
    db: &common::TestDb,
    tag: &str,
    count: usize,
) -> Vec<(htui_core::model::RunId, htui_core::model::RepoId)> {
    use htui_core::fixtures::ids;
    use htui_core::model::RepoId;
    use uuid::Uuid;

    sqlx::query("UPDATE run SET executing_box_id = $2 WHERE id = $1")
        .bind(ids::RUN_7.as_uuid())
        .bind(ids::BOX.as_uuid())
        .execute(&db.pool)
        .await
        .expect("RUN_7 executed on the fixture box");
    let mut planted = Vec::new();
    for index in 0..count {
        let repo: Uuid = sqlx::query_scalar(
            "INSERT INTO repo (project_id, name, remote_url) \
             VALUES ($1, $2, 'git@example.invalid:raced.git') RETURNING id",
        )
        .bind(ids::PROJECT_AGY.as_uuid())
        .bind(format!("{tag}-{index}"))
        .fetch_one(&db.pool)
        .await
        .expect("plant a repo");
        sqlx::query(
            "INSERT INTO integration (run_id, repo_id, project_id, mode, remote, target_branch) \
             VALUES ($1, $2, $3, 'push', 'git@example.invalid:raced.git', 'main')",
        )
        .bind(ids::RUN_7.as_uuid())
        .bind(repo)
        .bind(ids::PROJECT_AGY.as_uuid())
        .execute(&db.pool)
        .await
        .expect("plant a pending record");
        planted.push((ids::RUN_7, RepoId(repo)));
    }
    planted.sort_unstable();
    planted
}

/// MOD-133 fix round F1 (review L1), Postgres only: the walk's take (filtered to its run) and the
/// sweep's take (every run on the box), by different owners at once over two records of one run,
/// both answer `Ok` - neither is killed as a deadlock victim (`40P01`) - and together lease each
/// record exactly once. Repeated, because a deadlock needs the two scans to interleave.
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pg_walk_and_sweep_takes_of_one_run_never_deadlock() {
    use chrono::TimeDelta;
    use htui_core::fixtures::ids;
    use htui_core::store::WriteStore as _;
    use uuid::Uuid;

    const ROUNDS: usize = 40;
    let Some(db) = Box::pin(common::demo_db()).await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let pg = &db.store;
    let planted = plant_box_integrations(&db, "walked", 2).await;
    for round in 0..ROUNDS {
        sqlx::query(
            "UPDATE integration \
                SET lease_box_id = NULL, lease_owner = NULL, lease_expires_at = NULL \
              WHERE run_id = $1",
        )
        .bind(ids::RUN_7.as_uuid())
        .execute(&db.pool)
        .await
        .expect("free every lease");
        let (walk_owner, sweep_owner) = (Uuid::now_v7(), Uuid::now_v7());
        let (walk, sweep) = tokio::join!(
            pg.take_integrations(
                ids::BOX,
                walk_owner,
                TimeDelta::minutes(5),
                Some(ids::RUN_7)
            ),
            pg.take_integrations(ids::BOX, sweep_owner, TimeDelta::minutes(5), None),
        );
        let walk = walk.unwrap_or_else(|err| panic!("round {round}: the walk's take: {err}"));
        let sweep = sweep.unwrap_or_else(|err| panic!("round {round}: the sweep's take: {err}"));
        let mut taken: Vec<_> = walk
            .iter()
            .chain(&sweep)
            .map(|row| (row.run_id, row.repo_id))
            .collect();
        taken.sort_unstable();
        assert_eq!(
            taken, planted,
            "round {round}: every record is leased, to one owner only"
        );
    }

    db.drop_db().await;
}

/// MOD-133 fix round F1 (review L1), Postgres only: a take never waits on a row lock. With one
/// record held `FOR UPDATE` by another transaction, a take answers at once with the other record
/// only; once the lock is gone, the next take leases the held one.
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pg_integration_take_skips_a_record_another_transaction_holds() {
    use chrono::TimeDelta;
    use htui_core::fixtures::ids;
    use htui_core::store::WriteStore as _;
    use std::time::Duration;
    use uuid::Uuid;

    let Some(db) = Box::pin(common::demo_db()).await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let pg = &db.store;
    let planted = plant_box_integrations(&db, "held", 2).await;
    let (held, free) = (planted[0], planted[1]);
    let mut tx = db.pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM integration WHERE run_id = $1 AND repo_id = $2 FOR UPDATE")
        .bind(held.0.as_uuid())
        .bind(held.1.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("hold one record");

    let first = Uuid::now_v7();
    let taken = tokio::time::timeout(
        Duration::from_secs(10),
        pg.take_integrations(ids::BOX, first, TimeDelta::minutes(5), None),
    )
    .await
    .expect("the take does not wait on the held row")
    .expect("a take");
    assert_eq!(
        taken
            .iter()
            .map(|row| (row.run_id, row.repo_id))
            .collect::<Vec<_>>(),
        vec![free],
        "the held record is skipped, the free one leased"
    );

    tx.rollback().await.expect("release the lock");
    let second = Uuid::now_v7();
    let taken = pg
        .take_integrations(ids::BOX, second, TimeDelta::minutes(5), None)
        .await
        .expect("a take");
    assert_eq!(
        taken
            .iter()
            .map(|row| (row.run_id, row.repo_id))
            .collect::<Vec<_>>(),
        vec![held],
        "the released record is leased; the other stays with its first owner"
    );

    db.drop_db().await;
}

/// MOD-130 M3 T2 (P4, fact-check C24), Postgres only: a claim of a child-box run reads the child
/// box row `FOR SHARE`, so the tag check cannot race a re-probe or an `edit_box` of the box the
/// work runs on. The claim cannot be paused after it takes that lock, so the conflict is shown
/// from the other side: with the child's row held by an uncommitted tag edit, the driver's claim
/// waits on the child row and, once the edit commits, decides on the edited tags. A local run
/// takes no such lock: its claim answers at once while the child is held.
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn claim_run_holds_the_target_box_against_a_re_probe() {
    use chrono::{SubsecRound as _, TimeDelta, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        BoxId, Claim, GraphSnapshot, Isolation, NewChildBox, NewRun, NewTarget, ResolvedTarget,
        RunId, RunMode, SnapshotGraph, SnapshotSettings, TIMESTAMPTZ_DIGITS, TargetId, TargetKind,
        TargetSource,
    };
    use htui_core::store::WriteStore as _;
    use std::time::Duration;
    use uuid::Uuid;

    let Some(db) = Box::pin(common::demo_db()).await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let pg = &db.store;
    let at = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
    let lease = TimeDelta::minutes(5);
    let owner = Uuid::now_v7();

    let docker = pg
        .create_target(NewTarget {
            id: TargetId::new(),
            name: "raced docker".to_owned(),
            kind: TargetKind::Docker,
            locator: serde_json::json!({}),
        })
        .await
        .expect("create the docker target");
    let child = BoxId::new();
    pg.attach_child_box(NewChildBox {
        id: child,
        parent: ids::BOX,
        hostname: "raced-child".to_owned(),
        target: docker.id,
    })
    .await
    .expect("attach the child box");
    sqlx::query("UPDATE item SET required_tags = '{gpu}' WHERE id = $1")
        .bind(ids::HTUI_ANA_2.as_uuid())
        .execute(&db.pool)
        .await
        .expect("ANA-2 requires gpu");

    let new_run = |item, on, driver, target| NewRun {
        workspace_id: None,
        id: RunId::new(),
        project_id: ids::PROJECT_HTUI,
        item_id: item,
        mode: RunMode::Manual,
        target_box_id: on,
        executing_box_id: driver,
        started_by: ids::USER,
        graph_snapshot: GraphSnapshot {
            v: GraphSnapshot::V,
            graph: SnapshotGraph {
                id: ids::GRAPH_HTUI_FEAT,
                name: "feature".to_owned(),
                is_override: false,
                scope: None,
                revision: None,
            },
            topology: "sha256:pg_conformance".to_owned(),
            mode: RunMode::Manual,
            phases: Vec::new(),
            settings: SnapshotSettings {
                default_isolation: Isolation::Worktree,
                per_token_cap_run: None,
                per_token_cap_batch: None,
                max_fan_out: 4,
                max_agents_per_run: 6,
                max_parallel_sessions: SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS,
                max_plan_revisions: SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS,
                max_child_runs: SnapshotSettings::DEFAULT_MAX_CHILD_RUNS,
                runtime_changes: htui_core::model::RuntimeChanges::Ask,
            },
            scope: None,
            personas: Vec::new(),
            memory: None,
            capture: None,
            source: None,
            edges: Vec::new(),
            integrate: std::collections::BTreeMap::new(),
            toolbox: None,
        },
        repo_scope: Vec::new(),
        queued_at: at,
        batch_id: None,
        target,
    };
    let driven = pg
        .create_run(new_run(
            ids::HTUI_ANA_2,
            child,
            Some(ids::BOX),
            Some(ResolvedTarget {
                target: docker.id,
                source: TargetSource::Item,
            }),
        ))
        .await
        .expect("queue the child-box run")
        .id;
    let local = pg
        .create_run(new_run(ids::HTUI_CLEAN_1, ids::BOX, None, None))
        .await
        .expect("queue the local run")
        .id;

    // The holder: an uncommitted tag edit of the child, the row lock `edit_box` takes.
    let mut holder = db.pool.begin().await.expect("begin the holder");
    sqlx::query("UPDATE box SET declared_tags = '{gpu}' WHERE id = $1")
        .bind(child.as_uuid())
        .execute(&mut *holder)
        .await
        .expect("hold the child row");

    let claimed = tokio::time::timeout(
        Duration::from_secs(10),
        pg.claim_run(local, ids::BOX, owner, at, lease),
    )
    .await
    .expect("a local run's claim does not wait on the child row")
    .expect("claim the local run");
    assert_eq!(claimed, Claim::Admitted, "the local run is admitted");

    let release = async {
        for _ in 0..400 {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                  WHERE datname = current_database() AND pid <> pg_backend_pid() \
                    AND wait_event_type = 'Lock' AND strpos(query, $1) > 0",
            )
            .bind("FROM box WHERE id = $1 FOR SHARE")
            .fetch_one(&db.pool)
            .await
            .expect("read pg_stat_activity");
            if waiting > 0 {
                holder.commit().await.expect("commit the tag edit");
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the claim never waited on the child row");
    };
    let (claimed, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(pg.claim_run(driven, ids::BOX, owner, at, lease), release)
    })
    .await
    .expect("the claim answers once the edit commits");
    assert_eq!(
        claimed.expect("claim the child-box run"),
        Claim::Admitted,
        "the claim waited for the edit and read the child's committed `gpu`"
    );

    db.drop_db().await;
}
