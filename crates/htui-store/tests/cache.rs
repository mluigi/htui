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

use chrono::{SubsecRound as _, TimeDelta, Utc};
use htui_core::fixtures::{self, ids};
use htui_core::model::{
    CitationKind, DocumentId, EventKind, EventRole, ItemFilter, LinkGraph, NewDocument, ProjectId,
    PromptScope, Resolution, RunId, Scope, SessionEvent, Status, StepId, UserId, WorkspaceSummary,
};
use htui_core::store::{MemStore, ReadStore as _, StoreError, WriteStore as _};
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
        "requirement_spec",
        "requirement_area",
        "requirement",
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
    let live_citations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM item_requirement WHERE deleted_at IS NULL")
            .fetch_one(&db.pool)
            .await
            .expect("count live citations");
    assert_eq!(
        mirror_count(cache.pool(), "item_requirement").await,
        live_citations,
        "a tombstoned citation is a deletion in the mirror too (MOD-38 plan D12)"
    );

    assert_eq!(
        report.tombstones, 2,
        "the fixture has one tombstoned link and one tombstoned citation"
    );
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
            .document(DocumentId::new())
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
            statuses: Some(vec![Status::Open]),
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

    for (record, expected) in [
        (
            json!({ "estimated_after": 35_988, "sections": [{ "trimmed": false }, { "trimmed": true }] }),
            (Some(35_988), true),
        ),
        (json!({ "estimated_after": "34000" }), (None, false)),
        (json!({ "estimated_after": 35_988.5 }), (None, false)),
        (json!({ "estimated_after": 35_988.0_f64 }), (None, false)),
        (
            json!({ "estimated_after": 3_000_000_000_i64 }),
            (None, false),
        ),
        (
            json!({ "sections": { "a": { "trimmed": true } } }),
            (None, false),
        ),
        (json!({ "sections": { "trimmed": true } }), (None, false)),
        (
            json!({ "sections": [[{ "trimmed": true }]] }),
            (None, false),
        ),
        (
            json!({ "sections": [5, { "trimmed": true }] }),
            (None, true),
        ),
        (
            json!({ "sections": [{ "trimmed": "nope" }] }),
            (None, false),
        ),
        (json!({ "sections": [{ "trimmed": 1 }] }), (None, false)),
        (
            json!({ "sections": [{ "trimmed": [true] }] }),
            (None, false),
        ),
        (json!({ "sections": [5, "x"] }), (None, false)),
        (json!({}), (None, false)),
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
            Some(expected),
            "and both project {record} as {expected:?}"
        );
        assert_eq!(
            mirrored,
            Some(htui_core::model::prompt_summary(Some(&record))),
            "and that is what `prompt_summary` answers too"
        );
    }

    teardown(db, &[&cache]).await;
}

/// Every column `0003_orchestration.sql` adds reaches the mirror, and the ones a projection
/// carries read back through it (MOD-4 milestone 1, blueprint §3.8).
///
/// The fixture leaves all of them at their defaults — `touched_paths` empty on every item, the
/// verify and promotion columns NULL on every step, no scope and no lease on any run — so an
/// equality against `MemStore` proves nothing about them. This writes a distinct value into each
/// one on Postgres, refreshes, and reads it back: the projected ones (`ItemSummary.touched_paths`
/// and the six `RunStepSummary` fields) through the mirror's own reads, and the three `run`
/// columns no read reaches until the five `ReadStore` run reads land, straight off the table.
#[tokio::test]
async fn the_0003_columns_reach_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let scope = platform_scope().await;
    let lease_at = Utc::now().trunc_subsecs(6);
    let repo_scope = uuid::Uuid::from_u128(0x0003_0003_0003_0003_0003_0003_0003_0003);

    sqlx::query("UPDATE item SET touched_paths = $2 WHERE id = $1")
        .bind(ids::HTUI_FEAT_1.as_uuid())
        .bind(vec!["core:src/**".to_owned(), "docs:*.md".to_owned()])
        .execute(&db.pool)
        .await
        .expect("declare an overlap set");
    sqlx::query(
        "UPDATE run_step SET verify_outcome = 'fail', verify_exit_code = 7, promoted_at = $2, \
                             exit_code = 3, selected = true, usage = '{\"in\":1}'::jsonb \
          WHERE id = $1",
    )
    .bind(ids::STEP_IMPL.as_uuid())
    .bind(lease_at)
    .execute(&db.pool)
    .await
    .expect("record a verification and a promotion");
    sqlx::query(
        "UPDATE run SET repo_scope = $2::uuid[], lease_box_id = $3, lease_owner = gen_random_uuid(), \
                        lease_expires_at = $4 \
          WHERE id = $1",
    )
    .bind(ids::RUN_1.as_uuid())
    .bind(vec![repo_scope])
    .bind(db.store.this_box().as_uuid())
    .bind(lease_at)
    .execute(&db.pool)
    .await
    .expect("scope the run and take a lease on it");

    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let filter = ItemFilter::default();
    let mirrored_items = cache.items(&scope, &filter).await.expect("cache items");
    assert_eq!(
        mirrored_items
            .iter()
            .find(|row| row.id == ids::HTUI_FEAT_1)
            .map(|row| row.touched_paths.clone()),
        Some(vec!["core:src/**".to_owned(), "docs:*.md".to_owned()]),
        "item.touched_paths is mirrored and projected, in order"
    );
    assert_eq!(
        mirrored_items,
        db.store.items(&scope, &filter).await.expect("pg items"),
        "and the two backends project the whole list alike"
    );

    let mirrored_runs = cache.runs(ids::HTUI_FEAT_1).await.expect("cache runs");
    let step = mirrored_runs
        .iter()
        .flat_map(|run| run.steps.iter())
        .find(|step| step.id == ids::STEP_IMPL)
        .expect("the implement step is mirrored");
    assert_eq!(
        (
            step.verify_outcome,
            step.promoted_at,
            step.exit_code,
            step.selected,
            step.usage.clone(),
            step.agent_name.as_deref(),
        ),
        (
            Some(htui_core::model::VerifyOutcome::Fail),
            Some(lease_at),
            Some(3),
            Some(true),
            Some(serde_json::json!({ "in": 1 })),
            Some("claude"),
        ),
        "the six RunStepSummary fields ANA-2 added come off the mirror, `agent_name` through the \
         `LEFT JOIN agent` that stands in for `MemStore`'s agent map"
    );
    assert_eq!(
        mirrored_runs,
        db.store.runs(ids::HTUI_FEAT_1).await.expect("pg runs"),
        "and the two backends project the whole list alike"
    );

    // `CacheStore::run` decodes all three: `repos_col` on the JSON array and `opt_ts_col` on the
    // lease stamp. The conformance read case reaches them only with an empty scope and a NULL
    // lease - "a run seeded before 0003 declares no scope" - which every decoder answers alike, so
    // this is the one place the two projections are compared with values in them.
    let mirrored_run = cache.run(ids::RUN_1).await.expect("cache run");
    assert_eq!(
        mirrored_run,
        db.store.run(ids::RUN_1).await.expect("pg run"),
        "the mirror's run row equals Postgres's, scope and lease included"
    );
    let mirrored_run = mirrored_run.expect("RUN_1 is mirrored");
    assert_eq!(
        (
            mirrored_run.repo_scope,
            mirrored_run.lease_box_id,
            mirrored_run.lease_expires_at,
        ),
        (
            vec![htui_core::model::RepoId::from(repo_scope)],
            Some(db.store.this_box()),
            Some(lease_at),
        ),
        "and it decodes to the values written, not to the empty defaults an unscoped run has"
    );

    // The storage shape underneath that read, which no projection can show: `lease_owner` is
    // deliberately not mirrored at all (plan D7).
    let row =
        sqlx::query("SELECT repo_scope, lease_box_id, lease_expires_at FROM run WHERE id = ?")
            .bind(ids::RUN_1.to_string())
            .fetch_one(cache.pool())
            .await
            .expect("the mirrored run row");
    assert_eq!(
        (
            row.try_get::<String, _>("repo_scope").expect("repo_scope"),
            row.try_get::<Option<String>, _>("lease_box_id")
                .expect("lease_box_id"),
            row.try_get::<Option<i64>, _>("lease_expires_at")
                .expect("lease_expires_at"),
        ),
        (
            format!("[\"{repo_scope}\"]"),
            Some(db.store.this_box().to_string()),
            Some(lease_at.timestamp_micros()),
        ),
        "the scope arrives as a JSON array of uuids and the lease as epoch microseconds"
    );
    assert!(
        sqlx::query("SELECT lease_owner FROM run LIMIT 1")
            .fetch_optional(cache.pool())
            .await
            .is_err(),
        "`lease_owner` is not a mirror column: a liveness token for a process that is not running"
    );

    teardown(db, &[&cache]).await;
}

/// `run_step_tree` is the seventeenth mirrored table, and — having no `updated_at` of its own —
/// it rides its parent step's, exactly as `run_step_commit` does (ANA-2 §4.6, plan D9).
///
/// **This is the milestone's gate on the mirror carrying the two step-child tables at all** (T1
/// audit A-6): `trees_and_commits_read_back` is a totality case over a fixture with no
/// `run_step_tree` and no `run_step_commit` row, and both of its row-content twins are
/// `WriteStore` cases a read-only `CacheStore` cannot run. So this test asserts rows, not
/// emptiness — a mirror that refreshes nothing must fail it.
///
/// Blueprint F-F is the trap it exists for: `run_pass`'s table match used to end in a `_ =>`
/// arm, so a `run_step_tree` added to the cursor list alone would have been handed to
/// `refresh_run_step_commit`, which compiles, refreshes nothing, and leaves every read empty.
///
/// The passes run with **zero overlap** so the middle leg is deterministic: the §4.4 visibility
/// window would otherwise re-fetch a step whose `updated_at` is merely close to the high-water
/// mark, and "the tree moved but its step did not" would prove nothing.
#[tokio::test]
async fn run_step_tree_refreshes_off_its_parent_step() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let sharp = RefreshSettings {
        overlap: std::time::Duration::ZERO,
        ..settings(&db, 20)
    };

    // The fixture has no `repo` row (blueprint F-P), and both tables key on one.
    let repo: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO repo (project_id, name, is_primary) VALUES ($1, 'core', true) RETURNING id",
    )
    .bind(ids::PROJECT_HTUI.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("a repo for the trees to key on");
    sqlx::query(
        "INSERT INTO run_step_tree (run_step_id, repo_id, mode, path, base_ref, dirty) \
         VALUES ($1, $2, 'worktree', '/w/htui-feat-1', 'a1b2c3', false)",
    )
    .bind(ids::STEP_IMPL.as_uuid())
    .bind(repo)
    .execute(&db.pool)
    .await
    .expect("isolate the implement step");
    sqlx::query(
        "INSERT INTO run_step_commit (run_step_id, repo_id, before_hash, after_hash) \
         VALUES ($1, $2, 'a1b2c3', 'd4e5f6')",
    )
    .bind(ids::STEP_IMPL.as_uuid())
    .bind(repo)
    .execute(&db.pool)
    .await
    .expect("record what the step committed");

    run_pass(&db.pool, &cache, &all_projects(), &sharp)
        .await
        .expect("the first pass");

    let mirrored = cache.step_trees(ids::STEP_IMPL).await.expect("cache trees");
    assert_eq!(
        mirrored,
        db.store.step_trees(ids::STEP_IMPL).await.expect("pg trees"),
        "the tree reaches the mirror and reads back as Postgres reads it"
    );
    assert_eq!(
        mirrored
            .iter()
            .map(|tree| (
                tree.mode,
                tree.path.as_str(),
                tree.base_ref.as_str(),
                tree.dirty
            ))
            .collect::<Vec<_>>(),
        vec![(
            htui_core::model::Isolation::Worktree,
            "/w/htui-feat-1",
            "a1b2c3",
            false
        )],
        "and it is a row, not an empty answer that would pass against a mirror of nothing"
    );
    assert_eq!(
        cache
            .step_commits(ids::STEP_IMPL)
            .await
            .expect("cache commits"),
        db.store
            .step_commits(ids::STEP_IMPL)
            .await
            .expect("pg commits"),
        "`run_step_commit` is mirrored on the same terms and has never had a row to prove it"
    );
    assert!(
        cache
            .step_trees(ids::STEP_PLAN)
            .await
            .expect("cache trees")
            .is_empty(),
        "a step with no tree is empty, not the previous step's answer"
    );

    // The tree alone moves: its parent step's `updated_at` does not, so the pass does not see it.
    sqlx::query(
        "UPDATE run_step_tree SET dirty = true, base_ref = '999999' WHERE run_step_id = $1",
    )
    .bind(ids::STEP_IMPL.as_uuid())
    .execute(&db.pool)
    .await
    .expect("dirty the tree behind the cursor's back");
    run_pass(&db.pool, &cache, &all_projects(), &sharp)
        .await
        .expect("the blind pass");
    assert_eq!(
        cache
            .step_trees(ids::STEP_IMPL)
            .await
            .expect("cache trees")
            .first()
            .map(|tree| (tree.base_ref.clone(), tree.dirty)),
        Some(("a1b2c3".to_owned(), false)),
        "a tree has no timestamp of its own, so nothing about it alone advances a cursor"
    );

    // Touch the step: the `BEFORE UPDATE` trigger moves `updated_at`, and the tree rides it.
    sqlx::query("UPDATE run_step SET model = 'opus-2' WHERE id = $1")
        .bind(ids::STEP_IMPL.as_uuid())
        .execute(&db.pool)
        .await
        .expect("touch the parent step");
    run_pass(&db.pool, &cache, &all_projects(), &sharp)
        .await
        .expect("the riding pass");
    assert_eq!(
        cache
            .step_trees(ids::STEP_IMPL)
            .await
            .expect("cache trees")
            .first()
            .map(|tree| (tree.base_ref.clone(), tree.dirty)),
        Some(("999999".to_owned(), true)),
        "once the step moves, the tree is re-upserted whole"
    );

    teardown(db, &[&cache]).await;
}

/// `resolve_inputs`' three-armed `CASE` rank is the mirror's, not just Postgres's (T2 audit).
///
/// Blueprint §0.1's A-6 hole, in its second form. `the_mirror_passes_the_read_cases` is the only
/// harness a read-only `CacheStore` can run, and in every `resolve_inputs` leg of
/// `resolve_inputs_prefers_this_run_and_skips_losers` the expected document is *simultaneously* the
/// top-ranked row and the highest-version eligible one, so `ORDER BY d.version DESC` alone answers
/// each of them: deleting the whole `CASE WHEN s.id IS NULL THEN 2 WHEN s.run_id = ? THEN 0 ELSE 1
/// END` from `cache/read.rs` leaves that case green. The one test that does make the rank bite -
/// `write_document_allocates_its_version` - is a `WriteStore` case, and a `CacheStore` is read-only.
///
/// So the fixture's `research` ladder is extended here, in Postgres, with two rows that make each
/// arm decide something:
///
/// - **v4, hand-written** (`produced_by_step_id IS NULL`, rank 2). It is a higher version than the
///   winner, so rank 2 must lose to rank 0 and to rank 1.
/// - **v5, produced by `STEP_PLAN`** (rank 1 from `RUN_3`'s seat, rank 0 from `RUN_1`'s). `RUN_1` is
///   the run `STEP_PLAN` belongs to and its `selected` is `NULL`, so the row is eligible from both
///   seats and only the rank tells them apart.
///
/// From `RUN_3` the answer is still the fixture's v2 - rank 0 beats a higher-versioned rank 1 and
/// two rank 2s - and from `RUN_1` it is v5, rank 0 there. Both are asserted equal to Postgres's own
/// answer as well as by id, so a mirror that ranks differently *or* a Postgres that does fails.
#[tokio::test]
async fn the_mirror_ranks_resolve_inputs_the_way_postgres_does() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let insert = |version: i32, step: Option<StepId>| {
        let pool = db.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO document (item_id, kind, version, title, body, \
                                       produced_by_step_id, created_by) \
                 VALUES ($1, 'research', $2, $3, 'body', $4, $5) RETURNING id",
            )
            .bind(ids::HTUI_ANA_1.as_uuid())
            .bind(version)
            .bind(format!("research v{version}"))
            .bind(step.map(StepId::as_uuid))
            .bind(ids::USER.as_uuid())
            .fetch_one(&pool)
            .await
            .map(|row| row.get::<uuid::Uuid, _>("id"))
            .expect("a research row the rank has to judge")
        }
    };
    let hand_written_v4 = insert(4, None).await;
    let other_runs_v5 = insert(5, Some(ids::STEP_PLAN)).await;

    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the pass that carries the two new documents");

    let asked = ["research".to_owned()];
    let winner = |seat: RunId| {
        let (cache, db_store, asked) = (&cache, &db.store, &asked);
        async move {
            let mirrored = cache
                .resolve_inputs(ids::HTUI_ANA_1, seat, asked)
                .await
                .expect("the mirror resolves");
            assert_eq!(
                mirrored,
                db_store
                    .resolve_inputs(ids::HTUI_ANA_1, seat, asked)
                    .await
                    .expect("Postgres resolves"),
                "the two engines rank the same ladder the same way from {seat}"
            );
            mirrored
                .first()
                .and_then(|row| row.document.as_ref())
                .map(|document| document.id.as_uuid())
        }
    };

    assert_eq!(
        winner(ids::RUN_3).await,
        Some(ids::DOC_ANA_1_RESEARCH_V2.as_uuid()),
        "from RUN_3's seat its own selected output outranks v5's other run and v4's hand, \
         both of which are the higher version"
    );
    assert_ne!(
        winner(ids::RUN_3).await,
        Some(hand_written_v4),
        "and rank 2 never wins on version alone"
    );
    assert_eq!(
        winner(ids::RUN_1).await,
        Some(other_runs_v5),
        "from RUN_1's seat v5 is the rank-0 row, so the arms are told apart and not merely ordered"
    );

    teardown(db, &[&cache]).await;
}

/// The §7.3 walk never renders the root as its own upstream entry, even when the graph loops back
/// to it (T68, F-52 review, H3).
///
/// `MemStore` seeds `seen` with the root and so cannot emit it; the two SQL backends have a
/// recursive CTE whose anchor term starts at the root's *neighbours* and whose `best` aggregate has
/// nothing that excludes the root, so a cycle that returns to it within `hops` emitted it as an
/// ordinary entry. `migrations/0001_init.sql` forbids a self-loop only
/// (`CHECK (from_item_id <> to_item_id)`), so the two-edge cycle this builds is storable — and the
/// assembler would then feed an item its own summary as upstream context.
///
/// All three backends are asserted here rather than in `store::conformance::READ_CASES`, which is
/// where a read-only case belongs: `READ_CASES` reads the fixture and writes nothing, and the
/// fixture graph is acyclic. Giving it a cycle means editing `htui_core::fixtures`, whose edge
/// count, ready list and `links_hops_*` sets are pinned across three crates. The mirror harness
/// already has a Postgres handle, a mirror and the fixture data, so the cycle is built here and
/// `MemStore::from_demo` gets the same edge appended.
#[tokio::test]
async fn a_cycle_never_renders_the_root_as_its_own_upstream() {
    use htui_core::model::{ItemLink, LinkKind};

    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    // The fixture already has `agy:FIX-1 --blocked_by--> agy:ANA-1`; this closes the two-cycle, so
    // the root is reachable from itself at depth 2 and the walk's ceiling is 2.
    let back_edge = ItemLink {
        from_item_id: ids::AGY_ANA_1,
        to_item_id: ids::AGY_FIX_1,
        kind: LinkKind::BlockedBy,
        proposed_by_step_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    sqlx::query(
        "INSERT INTO item_link (from_item_id, to_item_id, kind, created_at, updated_at) \
         VALUES ($1, $2, 'blocked_by', $3, $4)",
    )
    .bind(back_edge.from_item_id.as_uuid())
    .bind(back_edge.to_item_id.as_uuid())
    .bind(back_edge.created_at)
    .bind(back_edge.updated_at)
    .execute(&db.pool)
    .await
    .expect("the back-edge lands in Postgres");

    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("a pass mirrors the back-edge");

    let mut data = fixtures::demo_data();
    data.links.push(back_edge);
    let mem = MemStore::from_demo(data);

    let scope = PromptScope::from_scope(&platform_scope().await, ids::PROJECT_AGY);
    let pg = db
        .store
        .upstream_summaries(ids::AGY_FIX_1, 2, &scope)
        .await
        .expect("pg upstream_summaries");
    let mirrored = cache
        .upstream_summaries(ids::AGY_FIX_1, 2, &scope)
        .await
        .expect("cache upstream_summaries");
    let reference = mem
        .upstream_summaries(ids::AGY_FIX_1, 2, &scope)
        .await
        .expect("mem upstream_summaries");

    for (backend, entries) in [("pg", &pg), ("mirror", &mirrored), ("mem", &reference)] {
        assert!(
            !entries.iter().any(|entry| entry.item_id == ids::AGY_FIX_1),
            "{backend}: the root is the step's own item and is never an entry, got {entries:?}"
        );
        assert!(
            entries.iter().any(|entry| entry.item_id == ids::AGY_ANA_1),
            "{backend}: the cycle's other arm is still walked, got {entries:?}"
        );
    }
    assert_eq!(pg, reference, "Postgres agrees with the reference store");
    assert_eq!(mirrored, reference, "the mirror agrees with it too");

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

/// MOD-38 T5 (blueprint §6.3): `item.resolution` reaches the mirror, and so do the four
/// requirement tables' names.
///
/// `FIX-1` is the fixture's closed item, backfilled to `done`; `agy` `FIX-1` is closed on Postgres
/// between two passes as `withdrawn`, which is the value only `close_out` can write and the
/// status-derived default would never produce.
#[tokio::test]
async fn a_closed_item_mirrors_its_resolution() {
    assert_eq!(
        htui_store::cache::MIRRORED_TABLES.len(),
        21,
        "MOD-38 adds requirement_spec, requirement_area, requirement and item_requirement",
    );
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let settings = settings(&db, 20);

    run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the first pass");
    let fix = cache
        .item(ids::HTUI_FIX_1)
        .await
        .expect("read the mirror")
        .expect("FIX-1 is mirrored");
    assert_eq!(fix.status, Status::Closed);
    assert_eq!(
        fix.resolution,
        Some(Resolution::Done),
        "the fixture's closed item"
    );
    let open = cache
        .item(ids::AGY_FIX_1)
        .await
        .expect("read the mirror")
        .expect("agy FIX-1 is mirrored");
    assert_eq!(open.resolution, None, "an open item has no resolution");

    db.store
        .close_out(
            ids::AGY_FIX_1,
            Resolution::Withdrawn,
            NewDocument {
                id: DocumentId::new(),
                item_id: ids::AGY_FIX_1,
                kind: "summary".to_owned(),
                title: "Withdrawn".to_owned(),
                body: String::new(),
                produced_by_step_id: None,
                created_by: ids::USER,
                created_at: Utc::now(),
            },
            &[],
        )
        .await
        .expect("close agy FIX-1 out as withdrawn");
    run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the pass that sees the close-out");

    let closed = cache
        .item(ids::AGY_FIX_1)
        .await
        .expect("read the mirror")
        .expect("agy FIX-1 is still mirrored");
    assert_eq!(closed.status, Status::Closed);
    assert_eq!(closed.resolution, Some(Resolution::Withdrawn));
    let raw: Option<String> = sqlx::query_scalar("SELECT resolution FROM item WHERE id = ?")
        .bind(ids::AGY_FIX_1.to_string())
        .fetch_one(cache.pool())
        .await
        .expect("read the mirrored column");
    assert_eq!(
        raw.as_deref(),
        Some("withdrawn"),
        "stored as its Postgres text"
    );

    teardown(db, &[&cache]).await;
}

/// MOD-38 T9 (blueprint §6.3): an `uncite` is a tombstone on Postgres and a deletion in the
/// mirror, as `item_link`'s is (plan D12).
///
/// The tombstone is written in SQL, the way [`a_tombstoned_link_disappears_from_the_mirror`]
/// writes its own: what is under test is the refresh arm, not `PgStore::uncite`, and the
/// `BEFORE UPDATE` trigger bumps `updated_at` either way.
#[tokio::test]
async fn the_mirror_drops_a_tombstoned_citation() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let settings = settings(&db, 20);

    run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the first pass");
    assert_eq!(
        cache
            .item_requirements(ids::HTUI_FIX_1)
            .await
            .expect("item_requirements")
            .iter()
            .map(|row| (row.requirement.id, row.kind))
            .collect::<Vec<_>>(),
        vec![(ids::REQ_STO_1, CitationKind::Addresses)],
        "the live citation is mirrored first",
    );

    sqlx::query(
        "UPDATE item_requirement SET deleted_at = now() \
          WHERE item_id = $1 AND requirement_id = $2 AND kind = 'addresses'",
    )
    .bind(ids::HTUI_FIX_1.as_uuid())
    .bind(ids::REQ_STO_1.as_uuid())
    .execute(&db.pool)
    .await
    .expect("tombstone the citation");

    let report = run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the pass that sees the tombstone");
    assert!(report.tombstones >= 1);

    assert_eq!(
        cache
            .item_requirements(ids::HTUI_FIX_1)
            .await
            .expect("item_requirements"),
        Vec::new(),
        "the tombstoned citation is gone from the item's reads",
    );
    assert!(
        !cache
            .requirement_coverage(ids::REQ_STO_1)
            .await
            .expect("requirement_coverage")
            .iter()
            .any(|row| row.item.id == ids::HTUI_FIX_1),
        "and from the requirement's coverage",
    );
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM item_requirement WHERE item_id = ? AND requirement_id = ?",
    )
    .bind(ids::HTUI_FIX_1.to_string())
    .bind(ids::REQ_STO_1.to_string())
    .fetch_one(cache.pool())
    .await
    .expect("count the mirrored row");
    assert_eq!(remaining, 0, "the row itself is deleted, not flagged");

    teardown(db, &[&cache]).await;
}

/// MOD-38 T9 (blueprint §6.3): plan D11's `suspect` is computed on the mirror's read, so a newer
/// requirement version that reaches the mirror makes an unchanged citation suspect offline, and a
/// re-stamped citation clears it again.
///
/// The amend and the re-stamp are written in SQL: the two refresh arms under test are
/// `requirement`'s and `item_requirement`'s, and both ride the `updated_at` the trigger bumps.
#[tokio::test]
async fn a_suspect_citation_reads_offline() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let settings = settings(&db, 20);

    run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the first pass");
    let before = cache
        .item_requirements(ids::HTUI_FEAT_1)
        .await
        .expect("item_requirements");
    assert_eq!(
        before
            .iter()
            .map(|row| (row.requirement.id, row.requirement_version, row.suspect))
            .collect::<Vec<_>>(),
        vec![(ids::REQ_STO_1, 1, false)],
        "FEAT-1 cites R-STO-1 at its current version",
    );

    sqlx::query("UPDATE requirement SET version = version + 1, body = $2 WHERE id = $1")
        .bind(ids::REQ_STO_1.as_uuid())
        .bind("Postgres is the source of truth; the cache mirrors it.")
        .execute(&db.pool)
        .await
        .expect("amend R-STO-1 on Postgres");
    run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the pass that sees the amend");

    let after = cache
        .item_requirements(ids::HTUI_FEAT_1)
        .await
        .expect("item_requirements");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].requirement.version, 2, "the requirement moved on");
    assert_eq!(after[0].requirement_version, 1, "the stamp did not");
    assert!(after[0].suspect, "so the citation is suspect offline");
    assert!(
        cache
            .requirement_coverage(ids::REQ_STO_1)
            .await
            .expect("requirement_coverage")
            .iter()
            .all(|row| row.suspect),
        "every citation of R-STO-1 is stamped at v1, so coverage is all suspect",
    );

    sqlx::query(
        "UPDATE item_requirement SET requirement_version = 2 \
          WHERE item_id = $1 AND requirement_id = $2 AND kind = 'addresses'",
    )
    .bind(ids::HTUI_FEAT_1.as_uuid())
    .bind(ids::REQ_STO_1.as_uuid())
    .execute(&db.pool)
    .await
    .expect("re-stamp the citation on Postgres");
    run_pass(&db.pool, &cache, &all_projects(), &settings)
        .await
        .expect("the pass that sees the re-stamp");

    let reconfirmed = cache
        .item_requirements(ids::HTUI_FEAT_1)
        .await
        .expect("item_requirements");
    assert_eq!(
        reconfirmed
            .iter()
            .map(|row| (row.requirement_version, row.suspect))
            .collect::<Vec<_>>(),
        vec![(2, false)],
        "a re-stamped citation is not suspect",
    );

    teardown(db, &[&cache]).await;
}

#[tokio::test]
async fn steps_beyond_n_lose_their_events() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    // The fixture only gives `STEP_PLAN` events; the newest finished step needs some too, or
    // "the newest keeps its events" has nothing to assert on.
    //
    // `refresh_transcripts` orders by `finished_at DESC` **within one project**, and MOD-4's
    // fixture growth (blueprint F-P) put `RUN_3` on `HTUI_ANA_1`, which is `PROJECT_HTUI` too:
    // its two candidates finish at hours 15 and 16 of day 1, after `STEP_REVIEW`'s 12. So the
    // project's newest finished step is now the fan-out loser, and naming `STEP_REVIEW` here
    // would assert that a step outside the window keeps its events.
    let newest = ids::STEP_R3_RESEARCH_B;
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
