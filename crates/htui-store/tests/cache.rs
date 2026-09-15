//! ANA-9 §11 criteria 4 (mirror half), 6 and 7, plus the §4.4 rebuild rules (blueprint E.4).
//!
//! Every case takes a `demo_db()` and a cache directory **under that database's throwaway config
//! root**, so nothing under `%APPDATA%\htui` (or `~/.config/htui`) is ever created by
//! `cargo test`; `TestDb::drop_db` removes both. With `HTUI_TEST_DATABASE_URL` unset `demo_db()`
//! prints `common::SKIP` and answers `None`, and every case here returns green (plan D13). The
//! `append_pending` cases that reach no server are the exception: they are file I/O only, so they
//! take a plain `tempfile::tempdir()` and run everywhere.
//!
//! `MemStore` is the reference implementation - it is what `store::conformance` pins - so the
//! §11.4 equality criterion is written as `cache.items(..) == mem.items(..)` and friends rather
//! than against a second Postgres read: `PgStore`'s `ReadStore` impl is MOD-6 T2's and lands in a
//! sibling worktree.
#![cfg(feature = "demo")]

use htui_store::testkit as common;

use std::path::Path;

use chrono::{TimeDelta, Utc};
use htui_core::fixtures::{self, ids};
use htui_core::model::{
    EventKind, EventRole, ItemFilter, LinkGraph, ProjectId, PromptScope, RunId, Scope,
    SessionEvent, StepId, UserId, WorkspaceSummary,
};
use htui_core::store::{MemStore, ReadStore as _, StoreError};
use htui_store::cache::pending::{append_pending, seal_orphaned, seal_pending, upload_pending};
use htui_store::cache::refresh::{RefreshSettings, Refresher, run_pass};
use htui_store::{Backend, CacheStore, identity};
use serde_json::json;
use sqlx::{Row as _, SqlitePool};

/// The three demo projects, which is the scope every mirroring case refreshes.
fn all_projects() -> Vec<ProjectId> {
    vec![ids::PROJECT_HTUI, ids::PROJECT_AGY, ids::PROJECT_VULKAN]
}

/// The `Platform` workspace scope: two projects, in `workspace_project.position` order.
async fn platform_scope() -> Scope {
    let summaries: Vec<WorkspaceSummary> = MemStore::demo()
        .workspaces()
        .await
        .expect("MemStore::workspaces");
    let platform = summaries
        .iter()
        .find(|w| w.workspace_id == ids::WORKSPACE_PLATFORM)
        .expect("the fixture has a Platform workspace");
    Scope::from_workspace(platform)
}

/// Settings whose identity halves point at the demo box and the demo user, as `load_demo` leaves
/// [`htui_store::PgStore`].
fn settings(db: &common::TestDb, transcript_steps: i64) -> RefreshSettings {
    RefreshSettings {
        transcript_steps,
        this_box: db.store.this_box(),
        this_user: db.store.this_user(),
        ..RefreshSettings::default()
    }
}

/// Opens a mirror under this test's throwaway config root.
async fn open_cache(db: &common::TestDb) -> CacheStore {
    open_cache_named(db, "fingerprint-under-test").await
}

/// [`open_cache`] under a chosen fingerprint, i.e. a chosen subdirectory.
async fn open_cache_named(db: &common::TestDb, fingerprint: &str) -> CacheStore {
    CacheStore::open(&db.config_root, fingerprint, 1)
        .await
        .expect("open the mirror")
}

/// Closes every mirror this test opened, then drops the database and the config root.
///
/// The close is not optional on Windows: `TestDb::drop_db` removes `config_root` with
/// `remove_dir_all`, which refuses while `cache.sqlite` still has an open handle, and it ignores
/// that error - so a test that skipped this would silently leave `%TEMP%\htui-test-*\cache\`
/// behind on every run.
async fn teardown(db: common::TestDb, caches: &[&CacheStore]) {
    for cache in caches {
        cache.close().await;
    }
    db.drop_db().await;
}

/// `COUNT(*)` of one mirrored table.
async fn mirror_count(pool: &SqlitePool, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) AS n FROM \"{table}\"");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("COUNT(*) FROM {table}: {e}"));
    row.try_get("n").expect("a count column")
}

/// `LinkGraph` with its nodes and edges in a canonical order.
///
/// `MemStore` walks its `links` vector, the mirror walks an index, so the two agree on membership
/// and on depth but not on the order within a depth; the criterion is the graph, not the walk.
fn canonical(mut graph: LinkGraph) -> LinkGraph {
    graph.nodes.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.item_id.cmp(&b.item_id))
    });
    graph.edges.sort_by(|a, b| {
        a.from_item_id
            .cmp(&b.from_item_id)
            .then_with(|| a.to_item_id.cmp(&b.to_item_id))
            .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
    });
    graph
}

/// One synthetic event of a pending buffer.
fn pending_event(step: StepId, seq: i32) -> SessionEvent {
    SessionEvent {
        run_step_id: step,
        seq,
        turn: 0,
        kind: if seq == 0 {
            EventKind::Prompt
        } else {
            EventKind::AssistantText
        },
        role: if seq == 0 {
            EventRole::Htui
        } else {
            EventRole::Agent
        },
        tool_call_id: None,
        payload: json!({ "text": format!("offline line {seq}") }),
        raw: None,
        at: fixtures::demo_at(3, i64::from(seq)),
    }
}

/// Writes `pending/<project>.<run>.jsonl` and returns its path.
fn write_pending(
    dir: &Path,
    project: ProjectId,
    run: RunId,
    lines: &[String],
) -> std::path::PathBuf {
    let path = dir.join("pending").join(format!("{project}.{run}.jsonl"));
    std::fs::write(&path, lines.join("\n")).expect("write the pending buffer");
    path
}

// ------------------------------------------------------------------------------------------------
// A pass mirrors the demo
// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_pass_mirrors_the_demo_projects() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let report = run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    // Table by table, the mirror holds what Postgres holds.
    for table in [
        "app_user",
        "agent",
        "workspace",
        "workspace_project",
        "project",
        "repo",
        "item_kind",
        "item",
        "item_note",
        "document",
        "run",
        "run_step",
        "run_step_commit",
        "session_event",
    ] {
        assert_eq!(
            mirror_count(cache.pool(), table).await,
            common::count(&db.pool, table).await,
            "{table} row count"
        );
    }

    // The two tables 4.4 deliberately mirrors partially.
    assert_eq!(
        mirror_count(cache.pool(), "box").await,
        1,
        "the mirror holds the own box row and no other (4.4)"
    );
    let live_links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM item_link WHERE deleted_at IS NULL")
            .fetch_one(&db.pool)
            .await
            .expect("count live links");
    assert_eq!(
        mirror_count(cache.pool(), "item_link").await,
        live_links,
        "a tombstone is a deletion in the mirror, not a row"
    );

    assert_eq!(report.tombstones, 1, "the fixture has one tombstoned link");
    assert_eq!(report.uploaded, 0, "nothing was buffered offline");
    assert_eq!(
        report.rows("agent"),
        3,
        "the unscoped registry rides the pass beside `app_user` (D32)"
    );
    assert!(report.rows("item") > 0, "items were mirrored");
    assert!(
        cache
            .meta()
            .await
            .expect("meta")
            .last_full_refresh_at
            .is_some(),
        "every cursor was zero, so the pass was a full pull"
    );

    teardown(db, &[&cache]).await;
}

/// `document` and `documents_of_kinds` over the mirror, against the reference store.
///
/// One of the three halves of [`mirror_reads_equal_the_reference_store`]'s milestone-9 additions.
/// Functions rather than three more blocks inline, so the case above stays readable at the length
/// the four new reads take it to; the assertions are that case's, and it is the only caller.
async fn documents_agree(cache: &CacheStore, mem: &MemStore) {
    for item in [
        ids::HTUI_ANA_1,
        ids::HTUI_FEAT_1,
        ids::HTUI_FEAT_3,
        ids::AGY_ANA_1,
        ids::AGY_FIX_1,
    ] {
        for kinds in [
            vec![],
            vec!["prd".to_owned(), "plan".to_owned()],
            vec!["plan".to_owned(), "prd".to_owned(), "missing".to_owned()],
            vec!["summary".to_owned()],
        ] {
            assert_eq!(
                cache
                    .documents_of_kinds(item, &kinds)
                    .await
                    .expect("cache documents_of_kinds"),
                mem.documents_of_kinds(item, &kinds)
                    .await
                    .expect("mem documents_of_kinds"),
                "documents_of_kinds({kinds:?}) of {item}"
            );
        }
        for head in mem.documents(item).await.expect("mem documents") {
            assert_eq!(
                cache.document(head.id).await.expect("cache document"),
                mem.document(head.id).await.expect("mem document"),
                "document {} of {item}, body and all",
                head.id
            );
        }
    }
    assert_eq!(
        cache
            .document(htui_core::model::DocumentId::new())
            .await
            .expect("cache document of an unknown id"),
        None,
        "an id nothing has is None, not a default row"
    );
}

/// The amended §7.3 walk over the mirror, against the reference store: both bounds, every hop
/// count the contract admits.
///
/// `AGY_FIX_1` is the fixture's diamond root (blueprint C.4); the other two roots reach less or
/// nothing, which is an answer the two stores still have to agree on. `hops` runs to `3` because
/// the clamp to `1..=2` is a store's job and all three must clamp alike.
async fn upstream_agrees(cache: &CacheStore, mem: &MemStore, scope: &Scope) {
    for root in [ids::AGY_FIX_1, ids::HTUI_FEAT_2, ids::HTUI_CLEAN_1] {
        for bound in [
            PromptScope::from_scope(scope, ids::PROJECT_AGY),
            PromptScope::project_only(ids::PROJECT_AGY),
            PromptScope::project_only(ids::PROJECT_HTUI),
        ] {
            for hops in [0, 1, 2, 3] {
                assert_eq!(
                    cache
                        .upstream_summaries(root, hops, &bound)
                        .await
                        .expect("cache upstream_summaries"),
                    mem.upstream_summaries(root, hops, &bound)
                        .await
                        .expect("mem upstream_summaries"),
                    "upstream_summaries({root}, {hops}, {bound:?})"
                );
            }
        }
    }
}

/// `project` over the mirror, settings document included, against the reference store.
async fn projects_agree(cache: &CacheStore, mem: &MemStore) {
    for project in all_projects() {
        assert_eq!(
            cache.project(project).await.expect("cache project"),
            mem.project(project).await.expect("mem project"),
            "project {project}, settings document included"
        );
    }
    assert_eq!(
        cache
            .project(ProjectId::new())
            .await
            .expect("cache project of an unknown id"),
        None,
        "an id the mirror does not hold is None, not an error"
    );
}

#[tokio::test]
async fn mirror_reads_equal_the_reference_store() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let mem = MemStore::demo();
    let scope = platform_scope().await;

    for filter in [
        ItemFilter::default(),
        ItemFilter {
            statuses: Some(vec![htui_core::model::Status::Open]),
            ..ItemFilter::default()
        },
        ItemFilter {
            project_ids: Some(vec![ids::PROJECT_AGY]),
            ..ItemFilter::default()
        },
        ItemFilter {
            tags: Some(vec!["rust".to_owned()]),
            ..ItemFilter::default()
        },
        ItemFilter {
            ready: Some(true),
            ..ItemFilter::default()
        },
        ItemFilter {
            text: Some("scaffold".to_owned()),
            ..ItemFilter::default()
        },
        // `text` is a literal substring in every store, so a LIKE wildcard matches nothing.
        ItemFilter {
            text: Some("_".to_owned()),
            ..ItemFilter::default()
        },
        ItemFilter {
            text: Some("%".to_owned()),
            ..ItemFilter::default()
        },
    ] {
        assert_eq!(
            cache.items(&scope, &filter).await.expect("cache items"),
            mem.items(&scope, &filter).await.expect("mem items"),
            "items for {filter:?}"
        );
    }

    for wildcard in ["_", "%", "%aff%", "sc_ffold"] {
        let filter = ItemFilter {
            text: Some(wildcard.to_owned()),
            ..ItemFilter::default()
        };
        assert!(
            cache
                .items(&scope, &filter)
                .await
                .expect("cache items")
                .is_empty(),
            "`{wildcard}` is a literal needle in the mirror too, so nothing matches"
        );
    }

    for item in [
        ids::HTUI_ANA_1,
        ids::HTUI_FEAT_1,
        ids::HTUI_FEAT_2,
        ids::HTUI_FEAT_3,
        ids::HTUI_CLEAN_1,
        ids::AGY_FEAT_1,
    ] {
        assert_eq!(
            cache.item(item).await.expect("cache item"),
            mem.item(item).await.expect("mem item"),
            "item {item}"
        );
        assert_eq!(
            canonical(cache.links(item, 2).await.expect("cache links")),
            canonical(mem.links(item, 2).await.expect("mem links")),
            "links of {item}"
        );
        assert_eq!(
            cache.documents(item).await.expect("cache documents"),
            mem.documents(item).await.expect("mem documents"),
            "documents of {item}"
        );
        assert_eq!(
            cache.notes(item).await.expect("cache notes"),
            mem.notes(item).await.expect("mem notes"),
            "notes of {item}"
        );
        assert_eq!(
            cache.runs(item).await.expect("cache runs"),
            mem.runs(item).await.expect("mem runs"),
            "runs of {item}"
        );
    }

    assert_eq!(
        cache
            .step_events(ids::STEP_PLAN)
            .await
            .expect("cache events"),
        mem.step_events(ids::STEP_PLAN).await.expect("mem events"),
        "the plan step replays identically"
    );
    assert_eq!(
        cache.workspaces().await.expect("cache workspaces"),
        mem.workspaces().await.expect("mem workspaces"),
    );
    assert_eq!(
        cache.projects(&scope).await.expect("cache projects"),
        mem.projects(&scope).await.expect("mem projects"),
    );
    assert_eq!(
        cache.active_runs(&scope).await.expect("cache active_runs"),
        mem.active_runs(&scope).await.expect("mem active_runs"),
    );
    assert_eq!(
        cache.box_info().await.expect("cache box_info"),
        mem.box_info().await.expect("mem box_info"),
    );

    // MOD-2 milestone 9's four additions. Every table they touch is mirrored — `document` body and
    // all, `project.settings`, `item_link`, `workspace_project` — so the mirror owes the same
    // answers here as for everything above; that is ANA-5 §12 criterion 19's second clause.
    documents_agree(&cache, &mem).await;
    upstream_agrees(&cache, &mem, &scope).await;
    projects_agree(&cache, &mem).await;

    teardown(db, &[&cache]).await;
}

/// Plan D106's two `RunStepSummary` fields are derived by three different projections — Rust in
/// `MemStore`, `jsonb` path in Postgres, `json_extract` in SQLite — over an **untyped** column, so
/// the third of them needs a malformed record put through it (T68, F-52).
///
/// `store::conformance::set_step_prompt_writes_digest_and_trim` covers the other two, but
/// `CacheStore` is not a `WriteStore`: the only way a record reaches the mirror is by writing it to
/// Postgres and refreshing, which is what this does. Every one of these made SQLite raise or decode
/// to the wrong type before T68 — and a raising projection fails the whole Runs read, not one row.
#[tokio::test]
async fn the_mirror_projects_a_malformed_trim_record_like_postgres() {
    use htui_core::store::WriteStore as _;

    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    for record in [
        json!({ "estimated_after": 35_988, "sections": [{ "trimmed": false }, { "trimmed": true }] }),
        json!({ "estimated_after": "34000" }),
        json!({ "estimated_after": 35_988.5 }),
        json!({ "estimated_after": 3_000_000_000_i64 }),
        json!({ "sections": { "a": { "trimmed": true } } }),
        json!({ "sections": [{ "trimmed": "nope" }] }),
        json!({ "sections": [{ "trimmed": 1 }] }),
        json!({ "sections": [5, "x"] }),
        json!({}),
    ] {
        db.store
            .set_step_prompt(ids::STEP_IMPL, "dead", &record)
            .await
            .expect("the Postgres write lands");
        run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
            .await
            .expect("a pass re-mirrors the step");

        let figures = |runs: &[htui_core::model::RunSummary]| {
            runs.iter()
                .flat_map(|run| run.steps.iter())
                .find(|step| step.id == ids::STEP_IMPL)
                .map(|step| (step.prompt_tokens, step.trimmed))
        };
        let mirrored = figures(&cache.runs(ids::HTUI_FEAT_1).await.expect("cache runs"));
        assert_eq!(
            mirrored,
            figures(&db.store.runs(ids::HTUI_FEAT_1).await.expect("pg runs")),
            "the mirror and Postgres project {record} alike"
        );
        assert_eq!(
            mirrored,
            Some(htui_core::model::prompt_summary(Some(&record))),
            "and both project it as `prompt_summary` does"
        );
    }

    teardown(db, &[&cache]).await;
}

/// ANA-5 §12 criterion 19's second clause, as a **suite** rather than as pairwise comparisons:
/// `store::conformance::READ_CASES` run over the mirror (plan D96).
///
/// `mirror_reads_equal_the_reference_store` above proves the mirror and `MemStore` agree on the
/// four new reads; `pg_conformance.rs::pg_store_read_conformance` proves Postgres and the suite
/// agree. This one closes the triangle, and with it criterion 7's second clause — the same diamond
/// read through `PgStore` and through `CacheStore` renders the same entries in the same canonical
/// order, which is what the prompt digest is a function of.
///
/// `CacheStore` is the only store in the workspace that implements `ReadStore` and not
/// `WriteStore`, so before `READ_CASES` existed there was no harness it could be a target of at
/// all.
#[tokio::test]
async fn the_mirror_passes_the_read_cases() {
    use htui_core::store::conformance;

    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let refresh = settings(&db, 20);
    let projects = all_projects();

    // `run_all_reads` builds a store per case; the second pass onwards is an incremental no-op
    // against the same cursors, so this costs one full pull and five cheap ones rather than six
    // mirrors. The bindings are references so the futures the closure returns borrow the test's
    // scope and not the closure's own.
    let (pool, cache_ref, projects_ref, refresh_ref) = (&db.pool, &cache, &projects, &refresh);
    conformance::run_all_reads(move || async move {
        run_pass(pool, cache_ref, projects_ref, refresh_ref)
            .await
            .expect("one pass");
        cache_ref.clone()
    })
    .await;

    teardown(db, &[&cache]).await;
}

/// The spawned loop, which is what MOD-6 T4 actually wires up.
///
/// `run_pass` carries every other case here, so this one only pins the three things a free
/// function cannot: the first pass runs without waiting for a tick, a scope pushed through the
/// `watch` channel is picked up on the next pass, and `abort` stops the task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_spawned_refresher_passes_and_follows_the_scope() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    // A scope of one project first, so the widening below is observable.
    let (scope_tx, scope_rx) = tokio::sync::watch::channel(vec![ids::PROJECT_HTUI]);
    let refresher = Refresher::spawn(
        db.pool.clone(),
        cache.clone(),
        scope_rx,
        RefreshSettings {
            // Long enough that only the immediate first pass and `trigger` can fire it.
            interval: std::time::Duration::from_secs(3_600),
            ..settings(&db, 20)
        },
    );

    let htui_items = wait_until(
        || async { mirror_count(cache.pool(), "item").await },
        |n| *n > 0,
    )
    .await
    .expect("the first pass runs immediately, without waiting for a tick");

    scope_tx
        .send(all_projects())
        .expect("the refresher still holds the receiver");
    refresher.trigger();
    let all_items = wait_until(
        || async { mirror_count(cache.pool(), "item").await },
        |n| *n > htui_items,
    )
    .await
    .expect("a widened scope is picked up on the next pass");
    assert!(all_items > htui_items);

    refresher.abort();
    assert!(
        wait_until(|| async { refresher.handle().is_finished() }, |done| *done)
            .await
            .is_some(),
        "abort stops the task",
    );

    teardown(db, &[&cache]).await;
}

/// Polls `probe` every 50 ms for five seconds, returning the first value that satisfies `done`.
///
/// The refresher is a task, so a test that reads after it has to wait for it; a fixed sleep would
/// be either flaky or slow.
async fn wait_until<T, F, Fut>(mut probe: F, done: impl Fn(&T) -> bool) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = T>,
{
    for _ in 0..100 {
        let value = probe().await;
        if done(&value) {
            return Some(value);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    None
}

// ------------------------------------------------------------------------------------------------
// §11.4, mirror half
// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn five_thousand_events_replay_from_the_mirror_in_order() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    // A fresh step of the finished run, so the last-N window certainly holds it.
    let step = StepId::new();
    sqlx::query(
        "INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, status, \
         started_at, finished_at) VALUES ($1, $2, 9, 1, 0, 'replay', 'done', now(), now())",
    )
    .bind(step.as_uuid())
    .bind(ids::RUN_1.as_uuid())
    .execute(&db.pool)
    .await
    .expect("insert the replay step");

    let inserted: Vec<SessionEvent> = (0..5_000).map(|seq| pending_event(step, seq)).collect();
    let mut tx = db.pool.begin().await.expect("begin");
    for event in &inserted {
        sqlx::query(
            "INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id, \
             payload, raw, at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(event.run_step_id.as_uuid())
        .bind(event.seq)
        .bind(event.turn)
        .bind(event.kind.as_str())
        .bind(event.role.as_str())
        .bind(event.tool_call_id.as_deref())
        .bind(&event.payload)
        .bind(event.raw.as_ref())
        .bind(event.at)
        .execute(&mut *tx)
        .await
        .expect("insert an event");
    }
    tx.commit().await.expect("commit");

    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let replayed = cache
        .step_events(step)
        .await
        .expect("replay")
        .expect("the step is cached");
    assert_eq!(replayed.len(), 5_000);
    assert_eq!(replayed, inserted, "same order, same content (11.4)");

    teardown(db, &[&cache]).await;
}

// ------------------------------------------------------------------------------------------------
// §11.6, the overlap window
// ------------------------------------------------------------------------------------------------

/// Criterion 6, both halves: the window catches the row, and without the window it is lost.
///
/// The shape matters. A held transaction alone is not enough to reproduce the gap - its
/// `updated_at` would still be ahead of a cursor that had not moved - so the sequence is the real
/// one of §4.4: the held write stamps `t0`, an *unheld* write on a second row stamps `t1 > t0` and
/// a pass carries the cursor to `t1`, and only then does the held transaction commit. The row is
/// now behind the cursor and exists nowhere in the mirror; `updated_at > t1 - overlap` is the one
/// thing that brings it back.
#[tokio::test]
async fn a_long_transaction_is_caught_by_the_overlap() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let wide = open_cache_named(&db, "wide-overlap").await;
    let narrow = open_cache_named(&db, "no-overlap").await;
    let base = settings(&db, 20);
    let wide_settings = RefreshSettings {
        overlap: std::time::Duration::from_secs(2),
        ..base
    };
    let narrow_settings = RefreshSettings {
        overlap: std::time::Duration::ZERO,
        ..base
    };

    for (cache, settings) in [(&wide, &wide_settings), (&narrow, &narrow_settings)] {
        run_pass(&db.pool, cache, &all_projects(), settings)
            .await
            .expect("the first pass");
    }
    let original = fixtures::demo_data()
        .items
        .iter()
        .find(|i| i.id == ids::HTUI_FEAT_1)
        .expect("the fixture item")
        .title
        .clone();
    assert_eq!(
        mirrored_title(&wide, ids::HTUI_FEAT_1).await,
        original,
        "the baseline pass mirrored the row as it was"
    );

    // A second connection updates the row at `t0` and holds the transaction open.
    let mut held = db.pool.acquire().await.expect("a second connection");
    sqlx::query("BEGIN")
        .execute(&mut *held)
        .await
        .expect("begin");
    sqlx::query("UPDATE item SET title = $1 WHERE id = $2")
        .bind("Edited inside a long transaction")
        .bind(ids::HTUI_FEAT_1.as_uuid())
        .execute(&mut *held)
        .await
        .expect("update inside the held transaction");

    // A committed write at `t1 > t0` on another row of the same table, so the pass below has
    // something to carry the cursor past `t0` with.
    sqlx::query("UPDATE item SET title = title WHERE id = $1")
        .bind(ids::HTUI_FEAT_2.as_uuid())
        .execute(&db.pool)
        .await
        .expect("a committed write after the held one");

    for (cache, settings) in [(&wide, &wide_settings), (&narrow, &narrow_settings)] {
        run_pass(&db.pool, cache, &all_projects(), settings)
            .await
            .expect("the pass that runs while the transaction is open");
    }

    sqlx::query("COMMIT")
        .execute(&mut *held)
        .await
        .expect("commit");
    drop(held);

    for (cache, settings) in [(&wide, &wide_settings), (&narrow, &narrow_settings)] {
        run_pass(&db.pool, cache, &all_projects(), settings)
            .await
            .expect("the pass after the commit");
    }

    assert_eq!(
        mirrored_title(&wide, ids::HTUI_FEAT_1).await,
        "Edited inside a long transaction",
        "the overlap window caught a write committed after the cursor moved past it (11.6)",
    );
    assert_eq!(
        mirrored_title(&narrow, ids::HTUI_FEAT_1).await,
        original,
        "and without the window the same write is lost, which is what the window is for",
    );

    teardown(db, &[&wide, &narrow]).await;
}

/// `item.title` as the mirror holds it.
async fn mirrored_title(cache: &CacheStore, item: htui_core::model::ItemId) -> String {
    cache
        .item(item)
        .await
        .expect("read the mirror")
        .expect("the item is mirrored")
        .title
}

#[tokio::test]
async fn a_tombstoned_link_disappears_from_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let settings = settings(&db, 20);

    run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the first pass");
    assert!(
        cache
            .links(ids::HTUI_FEAT_2, 1)
            .await
            .expect("links")
            .edges
            .iter()
            .any(|e| e.from_item_id == ids::HTUI_FEAT_2 && e.to_item_id == ids::HTUI_FEAT_1),
        "the live edge is mirrored first",
    );

    sqlx::query(
        "UPDATE item_link SET deleted_at = now() WHERE from_item_id = $1 AND to_item_id = $2",
    )
    .bind(ids::HTUI_FEAT_2.as_uuid())
    .bind(ids::HTUI_FEAT_1.as_uuid())
    .execute(&db.pool)
    .await
    .expect("tombstone the edge");

    let report = run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the pass that sees the tombstone");
    assert!(report.tombstones >= 1);

    let graph = cache.links(ids::HTUI_FEAT_2, 1).await.expect("links");
    assert!(
        !graph
            .edges
            .iter()
            .any(|e| e.from_item_id == ids::HTUI_FEAT_2 && e.to_item_id == ids::HTUI_FEAT_1),
        "the tombstoned edge is gone from the mirror",
    );
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM item_link WHERE from_item_id = ? AND to_item_id = ?",
    )
    .bind(ids::HTUI_FEAT_2.to_string())
    .bind(ids::HTUI_FEAT_1.to_string())
    .fetch_one(cache.pool())
    .await
    .expect("count the mirrored row");
    assert_eq!(remaining, 0, "the row itself is deleted, not flagged");

    teardown(db, &[&cache]).await;
}

#[tokio::test]
async fn steps_beyond_n_lose_their_events() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    // The fixture only gives `STEP_PLAN` events; the newest finished step needs some too, or
    // "the newest keeps its events" has nothing to assert on.
    let newest = ids::STEP_REVIEW;
    for seq in 0..3 {
        let event = pending_event(newest, seq);
        sqlx::query(
            "INSERT INTO session_event (run_step_id, seq, turn, kind, role, payload, at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(event.run_step_id.as_uuid())
        .bind(event.seq)
        .bind(event.turn)
        .bind(event.kind.as_str())
        .bind(event.role.as_str())
        .bind(&event.payload)
        .bind(event.at)
        .execute(&db.pool)
        .await
        .expect("insert an event on the newest step");
    }

    let cache = open_cache(&db).await;

    // With room for both, both are cached.
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the wide pass");
    assert!(
        cache
            .step_events(ids::STEP_PLAN)
            .await
            .expect("read")
            .is_some()
    );
    assert!(cache.step_events(newest).await.expect("read").is_some());

    // With room for one, only the newest finished step keeps its events.
    let report = run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 1))
        .await
        .expect("the narrow pass");
    assert!(report.trimmed > 0, "the older step was trimmed");
    assert!(
        cache.step_events(newest).await.expect("read").is_some(),
        "the newest finished step keeps its events",
    );
    assert_eq!(
        cache.step_events(ids::STEP_PLAN).await.expect("read"),
        None,
        "a step outside the last N is `None`, not an empty Vec (6.1)",
    );

    teardown(db, &[&cache]).await;
}

/// §6.2 step 4 selects the last N **finished** steps.
///
/// The presence probe of `refresh_transcripts` is existence-only ("this step already has rows, so
/// skip it"), which is sound for a finished step - `session_event` is append-only - and wrong for a
/// running one: its first partial copy would be frozen for the rest of the session. A running step
/// is therefore not a candidate at all until it has a `finished_at`.
#[tokio::test]
async fn a_running_step_is_mirrored_only_once_it_has_finished() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    // A step of the fixture's finished run that is still going: no `finished_at`, three events.
    let running = StepId::new();
    sqlx::query(
        "INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, status, \
                               started_at, finished_at) \
         VALUES ($1, $2, 99, 1, 0, 'implement', 'running', now(), NULL)",
    )
    .bind(running.as_uuid())
    .bind(ids::RUN_1.as_uuid())
    .execute(&db.pool)
    .await
    .expect("insert a running step");
    for seq in 0..3 {
        let event = pending_event(running, seq);
        sqlx::query(
            "INSERT INTO session_event (run_step_id, seq, turn, kind, role, payload, at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(event.run_step_id.as_uuid())
        .bind(event.seq)
        .bind(event.turn)
        .bind(event.kind.as_str())
        .bind(event.role.as_str())
        .bind(&event.payload)
        .bind(event.at)
        .execute(&db.pool)
        .await
        .expect("insert an event on the running step");
    }

    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the pass that sees the step running");
    assert_eq!(
        cache.step_events(running).await.expect("read"),
        None,
        "a running step is not one of the last N finished steps (6.2)",
    );

    // Two more events land and the step finishes; now the whole log is copied at once.
    for seq in 3..5 {
        let event = pending_event(running, seq);
        sqlx::query(
            "INSERT INTO session_event (run_step_id, seq, turn, kind, role, payload, at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(event.run_step_id.as_uuid())
        .bind(event.seq)
        .bind(event.turn)
        .bind(event.kind.as_str())
        .bind(event.role.as_str())
        .bind(&event.payload)
        .bind(event.at)
        .execute(&db.pool)
        .await
        .expect("insert a later event");
    }
    sqlx::query("UPDATE run_step SET status = 'done', finished_at = now() WHERE id = $1")
        .bind(running.as_uuid())
        .execute(&db.pool)
        .await
        .expect("finish the step");

    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the pass that sees it finished");
    let mirrored = cache
        .step_events(running)
        .await
        .expect("read")
        .expect("a finished step is mirrored");
    assert_eq!(
        mirrored.iter().map(|e| e.seq).collect::<Vec<i32>>(),
        vec![0, 1, 2, 3, 4],
        "every event of the finished step is there, not the three of the first pass",
    );

    teardown(db, &[&cache]).await;
}

// ------------------------------------------------------------------------------------------------
// The mirrored `agent` registry and the offline user (MOD-2 milestone 4, plan D31-D33)
// ------------------------------------------------------------------------------------------------

/// `agent` is unscoped, so the pass replaces it whole and names no cursor (D32).
#[tokio::test]
async fn a_pass_mirrors_the_agent_registry() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let report = run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");
    assert_eq!(report.rows("agent"), 3, "the fixture's registry rows");

    let mirrored = cache.agents().await.expect("read the mirrored registry");
    assert_eq!(
        mirrored
            .iter()
            .map(|row| row.agent.name.as_str())
            .collect::<Vec<&str>>(),
        vec!["agy", "claude", "claude-cli"],
        "ordered by agent.name, as the Postgres read is",
    );
    assert!(
        mirrored.iter().all(|row| row.on_box.is_none()),
        "`agent_box` is not mirrored, so no row claims a probe it does not have (D31)",
    );
    assert_eq!(
        mirrored,
        MemStore::demo()
            .agents()
            .await
            .expect("the reference registry"),
        "column for column, what the reference store answers",
    );

    // A full replace, not an accumulation - and still no `cache_cursor` row anywhere.
    let second = run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("a second pass");
    assert_eq!(second.rows("agent"), 3);
    assert_eq!(
        mirror_count(cache.pool(), "agent").await,
        3,
        "the second pass replaced the rows rather than duplicating them",
    );
    let cursors: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cache_cursor WHERE table_name = 'agent'")
            .fetch_one(cache.pool())
            .await
            .expect("count the agent cursors");
    assert_eq!(
        cursors, 0,
        "an unscoped table is a full replace and rides no cursor (D32)",
    );
    assert_eq!(
        cache.agents().await.expect("read again"),
        mirrored,
        "and answers the same rows",
    );

    teardown(db, &[&cache]).await;
}

/// The registry read an offline chat resolves its driver through (D31).
#[tokio::test]
async fn an_offline_backend_lists_the_mirrored_registry() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let backend = Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    };
    let listed = backend
        .agents()
        .await
        .expect("an offline registry read answers from the mirror");
    assert_eq!(
        listed
            .iter()
            .map(|row| row.agent.name.as_str())
            .collect::<Vec<&str>>(),
        vec!["agy", "claude", "claude-cli"],
    );
    assert!(listed.iter().all(|row| row.on_box.is_none()));
    assert!(
        !backend.is_writable(),
        "listing the registry did not make the server reachable: the re-dial ticker still fires",
    );

    teardown(db, &[&cache]).await;
}

/// `run.started_by` offline: the OS-derived name against the mirrored `app_user`, or `NotFound`
/// (D33). Never an invented author.
#[tokio::test]
async fn this_user_resolves_the_synced_name_and_refuses_an_unsynced_one() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    // This box's OS user, as `PgStore::seed_if_empty` would have named it. `ON CONFLICT` because
    // the fixture user may already carry that name on a maintainer's own machine.
    let os_name = identity::os_user_name();
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING")
        .bind(UserId::new().as_uuid())
        .bind(&os_name)
        .execute(&db.pool)
        .await
        .expect("seed an app_user for this OS user");
    let expected: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM app_user WHERE name = $1 ORDER BY created_at, id LIMIT 1",
    )
    .bind(&os_name)
    .fetch_one(&db.pool)
    .await
    .expect("the row is there");

    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let fixture_name = fixtures::demo_data().users[0].name.clone();
    assert_eq!(
        cache
            .user_named(&fixture_name)
            .await
            .expect("the fixture user is mirrored"),
        ids::USER,
    );
    assert_eq!(
        cache.this_user().await.expect("the OS user is mirrored"),
        UserId::from(expected),
        "the same name the online seed derives (D33)",
    );

    let backend = Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    };
    assert_eq!(
        backend.this_user().await.expect("the offline arm agrees"),
        UserId::from(expected),
    );

    assert!(
        matches!(
            cache.user_named("nobody-this-box-ever-synced").await,
            Err(StoreError::NotFound {
                entity: "app_user",
                ..
            })
        ),
        "a name this box never synced is refused, not invented",
    );

    teardown(db, &[&cache]).await;
}

// ------------------------------------------------------------------------------------------------
// Rebuild rules (§4.4, plan D8)
// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_schema_version_change_rebuilds() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");
    assert!(mirror_count(cache.pool(), "item").await > 0);
    let built_at = cache.meta().await.expect("meta").built_at;
    cache.close().await;
    drop(cache);

    // Same directory, a newer Postgres schema: the file is thrown away, not migrated.
    let reopened = CacheStore::open(&db.config_root, "fingerprint-under-test", 2)
        .await
        .expect("reopen after a schema bump");
    for table in htui_store::cache::MIRRORED_TABLES {
        assert_eq!(
            mirror_count(reopened.pool(), table).await,
            0,
            "{table} is empty after a rebuild"
        );
    }
    assert_eq!(mirror_count(reopened.pool(), "cache_cursor").await, 0);
    let meta = reopened.meta().await.expect("meta");
    assert_eq!(meta.schema_version, 2, "cache_meta is reset, not patched");
    assert_eq!(meta.last_full_refresh_at, None);
    assert!(meta.built_at >= built_at, "the file was rebuilt");

    teardown(db, &[&reopened]).await;
}

/// The same rule as [`a_schema_version_change_rebuilds`], through `db_fingerprint`.
///
/// The recorded fingerprint is rewritten in place rather than the directory being changed: a
/// different directory would start empty anyway and would prove nothing about the check. This is
/// the case that matters - a file that *is* there but belongs to another server (§4.4).
#[tokio::test]
async fn a_fingerprint_change_rebuilds() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");
    assert!(mirror_count(cache.pool(), "item").await > 0);

    sqlx::query("UPDATE cache_meta SET value = 'some-other-server' WHERE key = 'db_fingerprint'")
        .execute(cache.pool())
        .await
        .expect("rewrite the recorded fingerprint");
    cache.close().await;
    drop(cache);

    let path = db
        .config_root
        .join("cache")
        .join("fingerprint-under-test")
        .join(htui_store::cache::CACHE_FILE);
    assert!(path.exists(), "the mirror is on disk before the reopen");

    let reopened = open_cache(&db).await;
    for table in htui_store::cache::MIRRORED_TABLES {
        assert_eq!(
            mirror_count(reopened.pool(), table).await,
            0,
            "{table} is empty after a rebuild"
        );
    }
    assert_eq!(mirror_count(reopened.pool(), "cache_cursor").await, 0);
    assert_eq!(
        reopened.meta().await.expect("meta").db_fingerprint,
        "fingerprint-under-test",
        "cache_meta is reset to the server this mirror now belongs to",
    );

    // A reopen that agrees on both keys keeps everything: the rebuild is a mismatch, not a ritual.
    reopened.close().await;
    drop(reopened);
    let again = open_cache(&db).await;
    assert_eq!(mirror_count(again.pool(), "item").await, 0);
    run_pass(&db.pool, &again, &all_projects(), &settings(&db, 20))
        .await
        .expect("refill");
    let refilled = mirror_count(again.pool(), "item").await;
    assert!(refilled > 0);
    again.close().await;
    drop(again);
    let third = open_cache(&db).await;
    assert_eq!(
        mirror_count(third.pool(), "item").await,
        refilled,
        "same fingerprint and same schema version, same content",
    );

    teardown(db, &[&third]).await;
}

#[tokio::test]
async fn rebuild_clears_rows_but_keeps_the_file() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");
    let before = cache.meta().await.expect("meta");
    assert!(mirror_count(cache.pool(), "item").await > 0);

    cache.rebuild().await.expect("rebuild");

    for table in htui_store::cache::MIRRORED_TABLES {
        assert_eq!(mirror_count(cache.pool(), table).await, 0, "{table}");
    }
    assert_eq!(mirror_count(cache.pool(), "cache_cursor").await, 0);
    let after = cache.meta().await.expect("cache_meta survives a rebuild()");
    assert_eq!(after.schema_version, before.schema_version);
    assert_eq!(after.db_fingerprint, before.db_fingerprint);
    assert_eq!(
        after.built_at, before.built_at,
        "the file was not recreated"
    );
    assert_eq!(
        after.last_full_refresh_at, None,
        "the next pass is a full one"
    );

    teardown(db, &[&cache]).await;
}

#[tokio::test]
async fn a_week_old_full_refresh_clears_the_cursors_without_a_rebuild() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");
    let items = mirror_count(cache.pool(), "item").await;
    assert!(items > 0);

    // Backdate the last full pull past the seven-day rule (plan D8).
    sqlx::query("UPDATE cache_meta SET value = ? WHERE key = 'last_full_refresh_at'")
        .bind((Utc::now() - TimeDelta::days(8)).to_rfc3339())
        .execute(cache.pool())
        .await
        .expect("backdate");
    cache.close().await;
    drop(cache);

    let reopened = open_cache(&db).await;
    assert_eq!(
        mirror_count(reopened.pool(), "item").await,
        items,
        "the rows survive: a stale full refresh is not a rebuild",
    );
    assert_eq!(
        mirror_count(reopened.pool(), "cache_cursor").await,
        0,
        "every cursor is cleared, so the next pass is a full one",
    );

    teardown(db, &[&reopened]).await;
}

// ------------------------------------------------------------------------------------------------
// §11.7, the offline chat buffer
// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn pending_upload_lands_ordered() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let run = RunId::new();
    let step = StepId::new();
    let events: Vec<SessionEvent> = (0..20).map(|seq| pending_event(step, seq)).collect();
    let lines: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).expect("serialise an event"))
        .collect();
    let path = write_pending(cache.dir(), ids::PROJECT_HTUI, run, &lines);

    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("upload");
    assert_eq!(uploaded, 1);
    assert!(
        !path.exists(),
        "the file is removed after the commit (11.7)"
    );

    let run_row =
        sqlx::query("SELECT kind, mode, status, item_id, project_id FROM run WHERE id = $1")
            .bind(run.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("the synthesised run");
    assert_eq!(run_row.get::<String, _>("kind"), "chat");
    assert_eq!(run_row.get::<String, _>("mode"), "manual");
    assert_eq!(run_row.get::<String, _>("status"), "done");
    assert_eq!(run_row.get::<Option<uuid::Uuid>, _>("item_id"), None);
    assert_eq!(
        run_row.get::<uuid::Uuid, _>("project_id"),
        ids::PROJECT_HTUI.as_uuid()
    );

    let phase: String = sqlx::query_scalar("SELECT phase_name FROM run_step WHERE id = $1")
        .bind(step.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("the synthesised step");
    assert_eq!(phase, "chat");

    let seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM session_event WHERE run_step_id = $1 ORDER BY seq")
            .bind(step.as_uuid())
            .fetch_all(&db.pool)
            .await
            .expect("the events");
    assert_eq!(seqs, (0..20).collect::<Vec<i32>>(), "seq order preserved");

    teardown(db, &[&cache]).await;
}

#[tokio::test]
async fn pending_upload_is_idempotent() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let run = RunId::new();
    let step = StepId::new();
    let lines: Vec<String> = (0..20)
        .map(|seq| serde_json::to_string(&pending_event(step, seq)).expect("serialise"))
        .collect();
    write_pending(cache.dir(), ids::PROJECT_HTUI, run, &lines);
    upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("the first upload");

    let runs = common::count(&db.pool, "run").await;
    let steps = common::count(&db.pool, "run_step").await;
    let events = common::count(&db.pool, "session_event").await;

    // The same file again: three `ON CONFLICT DO NOTHING` clauses, and the file still goes.
    let path = write_pending(cache.dir(), ids::PROJECT_HTUI, run, &lines);
    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("the second upload");
    assert_eq!(uploaded, 1);
    assert!(!path.exists());
    assert_eq!(common::count(&db.pool, "run").await, runs);
    assert_eq!(common::count(&db.pool, "run_step").await, steps);
    assert_eq!(common::count(&db.pool, "session_event").await, events);

    teardown(db, &[&cache]).await;
}

/// A buffer the *server* refuses is skipped, not fatal, and does not block the files after it.
///
/// A malformed file never reaches Postgres; this one parses and is rejected on arrival - here by
/// `run.project_id REFERENCES project(id)`, in the field by a project deleted while the box was
/// offline or a `kind` outside its `CHECK`. Propagating that would abort the upload half of every
/// later pass, so one poisoned file would cost the user every chat buffered after it.
#[tokio::test]
async fn a_pending_file_the_server_refuses_does_not_block_the_ones_after_it() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    // A project that does not exist. Its id sorts before every fixture id, which all start at
    // `demo_uuid`'s epoch, so `upload_pending` meets this file first.
    let bogus_project: ProjectId = "00000000-0000-7000-8000-000000000000"
        .parse()
        .expect("a syntactically valid project id");
    let bogus_step = StepId::new();
    let bogus_lines: Vec<String> = (0..3)
        .map(|seq| serde_json::to_string(&pending_event(bogus_step, seq)).expect("serialise"))
        .collect();
    let refused = write_pending(cache.dir(), bogus_project, RunId::new(), &bogus_lines);

    let good_run = RunId::new();
    let good_step = StepId::new();
    let good_lines: Vec<String> = (0..3)
        .map(|seq| serde_json::to_string(&pending_event(good_step, seq)).expect("serialise"))
        .collect();
    let accepted = write_pending(cache.dir(), ids::PROJECT_HTUI, good_run, &good_lines);
    assert!(
        refused.file_name() < accepted.file_name(),
        "the refused file has to be the one `list` reaches first"
    );

    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("a file the server refuses is not a failed pass");

    assert_eq!(uploaded, 1, "only the file that landed is counted");
    assert!(
        refused.exists(),
        "the refused file stays on disk for inspection"
    );
    assert!(
        !accepted.exists(),
        "the file after it still landed and went"
    );

    let seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM session_event WHERE run_step_id = $1 ORDER BY seq")
            .bind(good_step.as_uuid())
            .fetch_all(&db.pool)
            .await
            .expect("the events of the accepted buffer");
    assert_eq!(seqs, vec![0, 1, 2]);
    let refused_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run WHERE project_id = $1")
        .bind(bogus_project.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("count the refused run");
    assert_eq!(refused_rows, 0, "its transaction rolled back whole");

    teardown(db, &[&cache]).await;
}

#[tokio::test]
async fn a_malformed_pending_file_is_left_alone() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let events = common::count(&db.pool, "session_event").await;

    let run = RunId::new();
    let step = StepId::new();
    let mut lines: Vec<String> = (0..3)
        .map(|seq| serde_json::to_string(&pending_event(step, seq)).expect("serialise"))
        .collect();
    lines.insert(1, "{ this is not a session_event }".to_owned());
    let path = write_pending(cache.dir(), ids::PROJECT_HTUI, run, &lines);

    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("upload");
    assert_eq!(uploaded, 0, "a malformed file lands nothing");
    assert!(path.exists(), "and is left in place for inspection");
    assert_eq!(
        common::count(&db.pool, "session_event").await,
        events,
        "not one line of it was inserted",
    );

    teardown(db, &[&cache]).await;
}

// ------------------------------------------------------------------------------------------------
// §11.7, the appender half: `append_pending` writes the name `upload_pending` reads
// ------------------------------------------------------------------------------------------------

/// Reads a buffer file back as one [`SessionEvent`] per line.
fn read_pending(path: &Path) -> Vec<SessionEvent> {
    let text = std::fs::read_to_string(path).expect("read the pending buffer");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every line is a session_event"))
        .collect()
}

/// The path `append_pending` owns while the chat is live, spelled out here rather than asked of
/// the appender (`[H-1]`: the open suffix is what keeps `upload_pending` off a running chat).
fn open_pending_path(dir: &Path, project: ProjectId, run: RunId) -> std::path::PathBuf {
    dir.join("pending")
        .join(format!("{project}.{run}.jsonl.open"))
}

/// The path `seal_pending` renames to, and the only one `upload_pending` looks at.
fn pending_path(dir: &Path, project: ProjectId, run: RunId) -> std::path::PathBuf {
    dir.join("pending").join(format!("{project}.{run}.jsonl"))
}

/// `[H-1]` The `n`th sealed name of one run: what a seal renames to when [`pending_path`] is taken.
fn numbered_pending_path(dir: &Path, project: ProjectId, run: RunId, n: u32) -> std::path::PathBuf {
    dir.join("pending")
        .join(format!("{project}.{run}.{n}.jsonl"))
}

/// Twenty events become twenty lines, in `seq` order, under the two-part name.
#[tokio::test]
async fn pending_append_creates_the_file_in_seq_order() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    let run = RunId::new();
    let step = StepId::new();
    let events: Vec<SessionEvent> = (0..20).map(|seq| pending_event(step, seq)).collect();

    let written = append_pending(root.path(), ids::PROJECT_HTUI, run, &events)
        .await
        .expect("append twenty events");
    assert_eq!(written, 20, "one line per event");

    let path = open_pending_path(root.path(), ids::PROJECT_HTUI, run);
    assert!(
        path.exists(),
        "the appender creates the file on first write, under the open name (H-1)"
    );
    assert!(
        !pending_path(root.path(), ids::PROJECT_HTUI, run).exists(),
        "and not under the name `upload_pending` reads: this chat is still live",
    );
    let back = read_pending(&path);
    assert_eq!(
        back.iter().map(|e| e.seq).collect::<Vec<i32>>(),
        (0..20).collect::<Vec<i32>>(),
        "the lines are in seq order",
    );
    assert_eq!(back, events, "each line is the serde form of its event");
}

/// A second call extends the file; it never rewrites what the first call wrote.
#[tokio::test]
async fn pending_append_extends_without_rewriting() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    let run = RunId::new();
    let step = StepId::new();
    let first: Vec<SessionEvent> = (0..20).map(|seq| pending_event(step, seq)).collect();
    let second: Vec<SessionEvent> = (20..30).map(|seq| pending_event(step, seq)).collect();

    append_pending(root.path(), ids::PROJECT_HTUI, run, &first)
        .await
        .expect("the first append");
    let path = open_pending_path(root.path(), ids::PROJECT_HTUI, run);
    let after_first = std::fs::read_to_string(&path).expect("read the first twenty");

    let written = append_pending(root.path(), ids::PROJECT_HTUI, run, &second)
        .await
        .expect("the second append");
    assert_eq!(written, 10, "only the new events are counted");

    let after_second = std::fs::read_to_string(&path).expect("read all thirty");
    assert!(
        after_second.starts_with(&after_first),
        "the first twenty lines are byte-identical after the second call",
    );
    assert_eq!(
        read_pending(&path)
            .iter()
            .map(|e| e.seq)
            .collect::<Vec<i32>>(),
        (0..30).collect::<Vec<i32>>(),
        "thirty lines, still in seq order",
    );
}

/// An empty slice writes nothing at all: an empty file is one `upload_pending` would only warn at.
#[tokio::test]
async fn pending_append_of_nothing_creates_no_file() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    let run = RunId::new();

    let written = append_pending(root.path(), ids::PROJECT_HTUI, run, &[])
        .await
        .expect("appending no events is not an error");
    assert_eq!(written, 0);
    assert!(
        !open_pending_path(root.path(), ids::PROJECT_HTUI, run).exists(),
        "no events, no file",
    );
}

/// A `pending/` that cannot be written is [`StoreError::Backend`], not a panic.
#[tokio::test]
async fn pending_append_reports_an_unwritable_parent() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    // A plain file where the directory has to be: portable on Windows and unix alike.
    std::fs::write(root.path().join("pending"), b"not a directory").expect("occupy the name");

    let step = StepId::new();
    let events = vec![pending_event(step, 0)];
    let err = append_pending(root.path(), ids::PROJECT_HTUI, RunId::new(), &events)
        .await
        .expect_err("the appender cannot create its file");
    assert!(
        matches!(err, StoreError::Backend(_)),
        "an unwritable parent is a backend error, got {err:?}",
    );
}

/// Criterion 12's first half: what the appender writes, the uploader reads.
#[tokio::test]
async fn pending_upload_lands_a_file_the_appender_wrote() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let run = RunId::new();
    let step = StepId::new();
    let first: Vec<SessionEvent> = (0..20).map(|seq| pending_event(step, seq)).collect();
    let second: Vec<SessionEvent> = (20..30).map(|seq| pending_event(step, seq)).collect();
    append_pending(cache.dir(), ids::PROJECT_HTUI, run, &first)
        .await
        .expect("append the first turn");
    append_pending(cache.dir(), ids::PROJECT_HTUI, run, &second)
        .await
        .expect("append the second turn");
    assert!(
        seal_pending(cache.dir(), ids::PROJECT_HTUI, run)
            .await
            .expect("the chat ends"),
        "the buffer was open, so sealing it is what makes it uploadable (H-1)"
    );
    let path = pending_path(cache.dir(), ids::PROJECT_HTUI, run);
    assert!(path.exists(), "the appender owns this name");

    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("upload the appended buffer");
    assert_eq!(uploaded, 1);
    assert!(
        !path.exists(),
        "the file is removed after the commit (11.7)"
    );

    let run_row = sqlx::query("SELECT kind, mode, status, project_id FROM run WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("the synthesised run");
    assert_eq!(run_row.get::<String, _>("kind"), "chat");
    assert_eq!(run_row.get::<String, _>("mode"), "manual");
    assert_eq!(run_row.get::<String, _>("status"), "done");
    assert_eq!(
        run_row.get::<uuid::Uuid, _>("project_id"),
        ids::PROJECT_HTUI.as_uuid()
    );

    let phase: String = sqlx::query_scalar("SELECT phase_name FROM run_step WHERE id = $1")
        .bind(step.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("the synthesised step");
    assert_eq!(phase, "chat");

    let seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM session_event WHERE run_step_id = $1 ORDER BY seq")
            .bind(step.as_uuid())
            .fetch_all(&db.pool)
            .await
            .expect("the events");
    assert_eq!(
        seqs,
        (0..30).collect::<Vec<i32>>(),
        "both appends landed, in seq order",
    );

    teardown(db, &[&cache]).await;
}

/// Uploading an appended buffer twice is still the three `ON CONFLICT DO NOTHING` clauses.
#[tokio::test]
async fn pending_upload_of_an_appended_file_is_idempotent() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let run = RunId::new();
    let step = StepId::new();
    let events: Vec<SessionEvent> = (0..20).map(|seq| pending_event(step, seq)).collect();
    append_pending(cache.dir(), ids::PROJECT_HTUI, run, &events)
        .await
        .expect("the first append");
    seal_pending(cache.dir(), ids::PROJECT_HTUI, run)
        .await
        .expect("the first seal");
    upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("the first upload");

    let runs = common::count(&db.pool, "run").await;
    let steps = common::count(&db.pool, "run_step").await;
    let session_events = common::count(&db.pool, "session_event").await;

    // The upload deleted the file, so the appender writes the same buffer again from scratch.
    append_pending(cache.dir(), ids::PROJECT_HTUI, run, &events)
        .await
        .expect("the second append");
    seal_pending(cache.dir(), ids::PROJECT_HTUI, run)
        .await
        .expect("the second seal");
    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("the second upload");
    assert_eq!(uploaded, 1);
    assert!(!pending_path(cache.dir(), ids::PROJECT_HTUI, run).exists());
    assert_eq!(common::count(&db.pool, "run").await, runs);
    assert_eq!(common::count(&db.pool, "run_step").await, steps);
    assert_eq!(
        common::count(&db.pool, "session_event").await,
        session_events,
    );

    teardown(db, &[&cache]).await;
}

// ------------------------------------------------------------------------------------------------
// `[H-1]` The seal: a live buffer is invisible to the uploader until its chat ends
// ------------------------------------------------------------------------------------------------

/// Sealing renames the open buffer once, and sealing again is `false` rather than an error.
#[tokio::test]
async fn seal_pending_renames_once() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    let run = RunId::new();
    let step = StepId::new();
    let events: Vec<SessionEvent> = (0..3).map(|seq| pending_event(step, seq)).collect();

    assert!(
        !seal_pending(root.path(), ids::PROJECT_HTUI, run)
            .await
            .expect("sealing nothing is not an error"),
        "a chat that recorded nothing has no buffer to seal",
    );

    append_pending(root.path(), ids::PROJECT_HTUI, run, &events)
        .await
        .expect("append");
    assert!(
        seal_pending(root.path(), ids::PROJECT_HTUI, run)
            .await
            .expect("the seal"),
    );

    let open = open_pending_path(root.path(), ids::PROJECT_HTUI, run);
    let sealed = pending_path(root.path(), ids::PROJECT_HTUI, run);
    assert!(!open.exists(), "the open name is gone");
    assert_eq!(read_pending(&sealed), events, "and the lines are unchanged");

    assert!(
        !seal_pending(root.path(), ids::PROJECT_HTUI, run)
            .await
            .expect("a second seal is not an error"),
        "there is nothing left open to seal",
    );
}

/// `[H-1]` A seal whose sealed name is taken renames to a **second** sealed name and never appends:
/// the file it found is not touched at all.
///
/// Appending was the earlier answer and it had two windows, both of which cost or duplicate rows.
/// A crash between the write and the `remove_file` left the open buffer in place, so the next
/// `seal_orphaned` appended the very same lines a second time and `UsageTotals::from_rows` counted
/// every `usage` row twice (criterion 7). And a second `htui` process that seals this run's buffer,
/// uploads it and deletes it while this one is still writing would take the appended tail down with
/// it. A rename is atomic and has neither window; the extra file uploads on its own pass.
#[tokio::test]
async fn a_seal_onto_a_taken_name_writes_a_second_file_and_never_appends() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    let run = RunId::new();
    let step = StepId::new();

    let first: Vec<SessionEvent> = (0..3).map(|seq| pending_event(step, seq)).collect();
    append_pending(root.path(), ids::PROJECT_HTUI, run, &first)
        .await
        .expect("the chat records");
    seal_pending(root.path(), ids::PROJECT_HTUI, run)
        .await
        .expect("the chat ends");
    let sealed = pending_path(root.path(), ids::PROJECT_HTUI, run);
    let sealed_bytes = std::fs::read(&sealed).expect("the sealed buffer");

    // A flush that landed after the seal, or a buffer a second process sealed under this one's
    // feet: either way the sealed name is taken when the seal comes round.
    let late: Vec<SessionEvent> = (3..5).map(|seq| pending_event(step, seq)).collect();
    append_pending(root.path(), ids::PROJECT_HTUI, run, &late)
        .await
        .expect("the late flush");
    assert!(
        seal_pending(root.path(), ids::PROJECT_HTUI, run)
            .await
            .expect("the second seal"),
    );

    assert_eq!(
        std::fs::read(&sealed).expect("the sealed buffer, again"),
        sealed_bytes,
        "the sealed file the seal found is byte-identical: nothing was appended to it",
    );
    let second = numbered_pending_path(root.path(), ids::PROJECT_HTUI, run, 1);
    assert_eq!(
        read_pending(&second),
        late,
        "the tail became a sealed file of its own",
    );
    assert!(
        !open_pending_path(root.path(), ids::PROJECT_HTUI, run).exists(),
        "and the open name is gone, because the seal was a rename",
    );

    // A third half keeps counting rather than colliding again.
    let later: Vec<SessionEvent> = (5..6).map(|seq| pending_event(step, seq)).collect();
    append_pending(root.path(), ids::PROJECT_HTUI, run, &later)
        .await
        .expect("a later flush");
    seal_pending(root.path(), ids::PROJECT_HTUI, run)
        .await
        .expect("the third seal");
    assert_eq!(
        read_pending(&numbered_pending_path(
            root.path(),
            ids::PROJECT_HTUI,
            run,
            2
        )),
        later,
    );
}

/// `[H-1]` Both halves of a collided seal upload, and the step's log is their union with no `seq`
/// twice.
///
/// Two files, two passes of the same `upload_pending` call: the second finds the `run` and the
/// `run_step` already there (`ON CONFLICT (id) DO NOTHING`) and adds only its own events.
#[tokio::test]
async fn both_halves_of_a_collided_seal_upload_into_one_step() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let run = RunId::new();
    let step = StepId::new();
    let first: Vec<SessionEvent> = (0..3).map(|seq| pending_event(step, seq)).collect();
    let late: Vec<SessionEvent> = (3..5).map(|seq| pending_event(step, seq)).collect();
    for half in [&first, &late] {
        append_pending(cache.dir(), ids::PROJECT_HTUI, run, half)
            .await
            .expect("append a half");
        seal_pending(cache.dir(), ids::PROJECT_HTUI, run)
            .await
            .expect("seal a half");
    }
    assert!(
        numbered_pending_path(cache.dir(), ids::PROJECT_HTUI, run, 1).exists(),
        "the collision made a second sealed file, which is what has to upload too",
    );

    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("upload both halves");
    assert_eq!(uploaded, 2, "each sealed file is a pass of its own");
    assert!(!pending_path(cache.dir(), ids::PROJECT_HTUI, run).exists());
    assert!(!numbered_pending_path(cache.dir(), ids::PROJECT_HTUI, run, 1).exists());

    let seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM session_event WHERE run_step_id = $1 ORDER BY seq")
            .bind(step.as_uuid())
            .fetch_all(&db.pool)
            .await
            .expect("the events");
    assert_eq!(
        seqs,
        (0..5).collect::<Vec<i32>>(),
        "the union of both halves, each seq exactly once",
    );

    teardown(db, &[&cache]).await;
}

/// A buffer that cannot be sealed is a warning, never a failed [`CacheStore::open`] (M-2).
///
/// `seal_orphaned` is best-effort adoption of what a crash left behind; propagating the first
/// `seal_one` error made one unreadable file - a permissions problem here, a sealed file another
/// process holds open on Windows - fail `start()` and take the whole application down with it.
/// `upload_pending` has always treated per-file trouble as warn-and-continue, and this now matches.
#[cfg(unix)]
#[tokio::test]
async fn seal_orphaned_warns_on_a_buffer_it_cannot_seal_and_open_still_succeeds() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().expect("a throwaway cache root");
    let cache = CacheStore::open(root.path(), "degraded-seal", 1)
        .await
        .expect("open a throwaway mirror");
    let step = StepId::new();
    let runs = [RunId::new(), RunId::new()];
    for run in runs {
        append_pending(
            cache.dir(),
            ids::PROJECT_HTUI,
            run,
            &[pending_event(step, 0)],
        )
        .await
        .expect("append");
    }
    let dir = cache.dir().to_path_buf();
    let pending = dir.join("pending");
    cache.close().await;
    drop(cache);

    // The process died here, and `pending/` is no longer writable: every rename in it will fail.
    let restore = std::fs::metadata(&pending)
        .expect("stat pending")
        .permissions();
    std::fs::set_permissions(&pending, std::fs::Permissions::from_mode(0o555))
        .expect("make pending read-only");

    assert_eq!(
        seal_orphaned(&dir).expect("a sweep that can list pending is Ok even when nothing seals"),
        0,
        "nothing was sealed, and that is a warning rather than an error",
    );
    let reopened = CacheStore::open(root.path(), "degraded-seal", 1)
        .await
        .expect("a buffer that cannot be sealed must not fail start()");
    for run in runs {
        assert!(
            open_pending_path(reopened.dir(), ids::PROJECT_HTUI, run).exists(),
            "the buffer is left exactly where it was, for the next sweep",
        );
    }
    reopened.close().await;

    std::fs::set_permissions(&pending, restore).expect("restore pending");
    assert_eq!(
        seal_orphaned(&dir).expect("the sweep that can write"),
        2,
        "both buffers survived the degraded pass and are adopted by the next one",
    );
}

/// The one thing `seal_orphaned` still refuses to shrug off: a `pending/` it cannot list.
///
/// A missing directory is `Ok(0)` - nothing has ever been buffered on this box - but a `pending/`
/// that is there and unreadable is not a per-file problem, and answering `0` would claim a sweep
/// that never happened.
#[tokio::test]
async fn seal_orphaned_reports_a_pending_it_cannot_list() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    assert_eq!(
        seal_orphaned(root.path()).expect("no pending/ at all is not an error"),
        0,
    );

    // A plain file where the directory has to be: portable on Windows and unix alike.
    std::fs::write(root.path().join("pending"), b"not a directory").expect("occupy the name");
    let err = seal_orphaned(root.path()).expect_err("pending/ cannot be listed");
    assert!(
        matches!(err, StoreError::Backend(_)),
        "an unlistable pending/ is a backend error, got {err:?}",
    );
}

/// A buffer a crashed run left open is adopted by the next `CacheStore::open`.
#[tokio::test]
async fn seal_orphaned_adopts_open_buffers_at_open() {
    let root = tempfile::tempdir().expect("a throwaway cache root");
    let cache = CacheStore::open(root.path(), "seal-test", 1)
        .await
        .expect("open a throwaway mirror");
    let run = RunId::new();
    let step = StepId::new();
    let events: Vec<SessionEvent> = (0..3).map(|seq| pending_event(step, seq)).collect();
    append_pending(cache.dir(), ids::PROJECT_HTUI, run, &events)
        .await
        .expect("append");
    cache.close().await;
    drop(cache);

    // The process died here: the buffer is still open and no `finish_chat_run` ever ran.
    let reopened = CacheStore::open(root.path(), "seal-test", 1)
        .await
        .expect("reopen the same mirror");
    assert!(
        !open_pending_path(reopened.dir(), ids::PROJECT_HTUI, run).exists(),
        "no chat of this process can be live, so an open buffer is a crashed one",
    );
    assert_eq!(
        read_pending(&pending_path(reopened.dir(), ids::PROJECT_HTUI, run)),
        events,
        "its rows are complete as far as they got, and now uploadable (R-HIS-1)",
    );
    assert_eq!(
        seal_orphaned(reopened.dir()).expect("a second sweep"),
        0,
        "nothing is left open"
    );
    reopened.close().await;
}

/// The race `[H-1]` exists for: a chat still running when the refresher's pass comes round.
#[tokio::test]
async fn upload_pending_ignores_open_buffers() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let live_run = RunId::new();
    let live_step = StepId::new();
    let live: Vec<SessionEvent> = (0..3).map(|seq| pending_event(live_step, seq)).collect();
    append_pending(cache.dir(), ids::PROJECT_HTUI, live_run, &live)
        .await
        .expect("the live chat appends");

    let done_run = RunId::new();
    let done_step = StepId::new();
    let done: Vec<SessionEvent> = (0..3).map(|seq| pending_event(done_step, seq)).collect();
    append_pending(cache.dir(), ids::PROJECT_HTUI, done_run, &done)
        .await
        .expect("the finished chat appends");
    seal_pending(cache.dir(), ids::PROJECT_HTUI, done_run)
        .await
        .expect("the finished chat seals");

    let uploaded = upload_pending(
        &db.pool,
        cache.dir(),
        db.store.this_box(),
        db.store.this_user(),
    )
    .await
    .expect("upload");
    assert_eq!(uploaded, 1, "only the sealed buffer landed");
    assert!(
        open_pending_path(cache.dir(), ids::PROJECT_HTUI, live_run).exists(),
        "the live chat's buffer is still on disk, still open",
    );
    let live_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM session_event WHERE run_step_id = $1")
            .bind(live_step.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("count the live chat's rows");
    assert_eq!(
        live_rows, 0,
        "and not one of its rows was uploaded mid-flight (H-1)"
    );
    let done_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM session_event WHERE run_step_id = $1")
            .bind(done_step.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("count the finished chat's rows");
    assert_eq!(done_rows, 3, "the chat that ended landed whole");

    teardown(db, &[&cache]).await;
}
