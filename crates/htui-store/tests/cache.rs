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
    CitationKind, DocumentId, EventKind, EventRole, ItemFilter, ItemState, LinkGraph,
    MemoryEntryId, NewDocument, ProjectId, PromptScope, Resolution, RunId, Scope, SessionEvent,
    Status, StepId, StepOpening, StepRole, UserId, WorkspaceSummary,
};
use htui_core::model::{NewRunPlan, PlanScope, PlanStatus, RunPlanId};
use htui_core::store::{MemStore, ReadStore as _, StepFence, StoreError, WriteStore as _};
use htui_store::cache::refresh::{RefreshSettings, Refresher, run_pass};
use htui_store::{Backend, CacheStore, ScopeKeys, identity};
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

/// Settings whose `this_box` points at the demo box, as `load_demo` leaves [`htui_store::PgStore`].
fn settings(db: &common::TestDb, transcript_steps: i64) -> RefreshSettings {
    RefreshSettings {
        transcript_steps,
        this_box: db.store.this_box(),
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

/// One synthetic `session_event` row.
fn synthetic_event(step: StepId, seq: i32) -> SessionEvent {
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

// ------------------------------------------------------------------------------------------------
// A pass mirrors the demo
// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_pass_mirrors_the_demo_projects() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    // MOD-147 D1: the two workspaces' scope rows are projects too, and the store worker adds a
    // workspace's to the projects it hands the refresher, so "everything" names all five.
    let mut everything = all_projects();
    everything.extend([ids::PROJECT_SCOPE_PLATFORM, ids::PROJECT_SCOPE_GRAPHICS]);
    let report = run_pass(&db.pool, &cache, &everything, &settings(&db, 20))
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
        // MOD-144: the derived state and the not-ready half. `ItemSummary` equality covers
        // `latest_run_status`, so these are the mirror-against-MemStore drift guard.
        ItemFilter {
            ready: Some(false),
            ..ItemFilter::default()
        },
        ItemFilter {
            states: Some(vec![ItemState::Running, ItemState::Failed]),
            ..ItemFilter::default()
        },
    ]
    .into_iter()
    .chain(ItemState::ALL.iter().map(|state| ItemFilter {
        states: Some(vec![*state]),
        ..ItemFilter::default()
    })) {
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
            .set_step_prompt(StepFence::Unleased, ids::STEP_IMPL, "dead", &record)
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

/// MOD-72 review M2: the mirror's `json_type` guard (blueprint E3) against the `tool_kind` shapes
/// the fixture lacks. `store::conformance::tool_call_counts_group_by_step_and_kind` runs on Mem and
/// Postgres only, and the mirror's read case sees the fixture's one `read`; so this writes the
/// shapes to Postgres, refreshes, and reads them back. A bare `json_extract` would hand back the
/// integer 7, which fails the whole read rather than counting as `other`.
#[tokio::test]
async fn the_mirror_counts_tool_calls_like_postgres() {
    use htui_core::model::ToolCallCount;

    let Some(db) = common::demo_db().await else {
        return;
    };

    // Two finished steps of `FEAT-1`'s run; its plan step keeps the fixture's one `read`.
    let call = EventKind::ToolCall;
    let rows = [
        (ids::STEP_IMPL, call, json!({ "tool_kind": "execute" })),
        // A result names its call's kind too, so a kind-blind count would say 3.
        (
            ids::STEP_IMPL,
            EventKind::ToolResult,
            json!({ "status": "completed", "tool_kind": "execute" }),
        ),
        (ids::STEP_IMPL, call, json!({ "tool_kind": "execute" })),
        (ids::STEP_IMPL, call, json!({ "tool_kind": "read" })),
        (ids::STEP_IMPL, call, json!({ "title": "no kind at all" })),
        (ids::STEP_REVIEW, call, json!({ "tool_kind": "edit" })),
        (ids::STEP_REVIEW, call, json!({ "tool_kind": null })),
        (ids::STEP_REVIEW, call, json!({ "tool_kind": 7 })),
    ];
    for (seq, (step, kind, payload)) in (0..).zip(rows) {
        sqlx::query(
            "INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id, \
                                        payload, at) \
             VALUES ($1, $2, 0, $3, 'agent', $4, $5, $6)",
        )
        .bind(step.as_uuid())
        .bind(seq)
        .bind(kind.as_str())
        .bind(format!("call-{seq}"))
        .bind(&payload)
        .bind(fixtures::demo_at(3, i64::from(seq)))
        .execute(&db.pool)
        .await
        .expect("insert a tool event");
    }

    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let count = |step, tool_kind: &str, calls| ToolCallCount {
        step,
        tool_kind: tool_kind.to_owned(),
        calls,
    };
    let mut expected = vec![
        count(ids::STEP_PLAN, "read", 1),
        count(ids::STEP_IMPL, "execute", 2),
        count(ids::STEP_IMPL, "read", 1),
        count(ids::STEP_IMPL, "other", 1),
        count(ids::STEP_REVIEW, "edit", 1),
        count(ids::STEP_REVIEW, "other", 2),
    ];
    ToolCallCount::sort_canonical(&mut expected);
    let mirrored = cache
        .tool_call_counts(ids::HTUI_FEAT_1)
        .await
        .expect("the mirror reads every shape");
    assert_eq!(
        mirrored,
        db.store
            .tool_call_counts(ids::HTUI_FEAT_1)
            .await
            .expect("pg tool_call_counts"),
        "the mirror and Postgres count alike"
    );
    assert_eq!(mirrored, expected);

    teardown(db, &[&cache]).await;
}

/// MOD-69 plan D2: the mirror's `waiting_candidates` (three runtime reads over `item`, `run` and
/// `run_step`) answers Postgres' rows whole after a pass: a gate park, a selection park on a
/// minted item, a blocked minted item whose run is stopped (MOD-125 D2), FEAT-3 blocked behind its
/// queued run, a blocked `agy` item, and the fixture's two candidates. With that stopped run in
/// the scope, the mirror's `active_runs` answers Postgres' count, which is the scope's runs on the
/// top bar ([`RunStatus::on_top_bar`]): live, less `stopped`.
#[tokio::test]
async fn the_mirror_lists_waiting_candidates_like_postgres() {
    use htui_core::model::{
        Claim, GraphSnapshot, Isolation, ItemId, NewItem, NewRun, NewRunStep, RunMode, RunStatus,
        SnapshotGraph, SnapshotSettings, StepStatus,
    };
    use htui_core::store::ParkOutcome;

    let Some(db) = common::demo_db().await else {
        return;
    };
    let owner = uuid::Uuid::now_v7();
    let at = Utc::now().trunc_subsecs(6);
    let new_run = |item: ItemId| NewRun {
        workspace_id: None,
        id: RunId::new(),
        project_id: ids::PROJECT_HTUI,
        item_id: item,
        mode: RunMode::Manual,
        target_box_id: ids::BOX,
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
            topology: "sha256:cache".to_owned(),
            mode: RunMode::Manual,
            phases: Vec::new(),
            settings: SnapshotSettings {
                default_isolation: Isolation::Worktree,
                per_token_cap_run: None,
                per_token_cap_batch: None,
                max_fan_out: 4,
                max_agents_per_run: 8,
                max_parallel_sessions: SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS,
                max_plan_revisions: SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS,
                max_child_runs: SnapshotSettings::DEFAULT_MAX_CHILD_RUNS,
                runtime_changes: htui_core::model::RuntimeChanges::Ask,
            },
            scope: None,
            personas: Vec::new(),
            memory: None,
            source: None,
            edges: Vec::new(),
        },
        repo_scope: Vec::new(),
        queued_at: at,
        batch_id: None,
    };
    let new_step = |run: RunId, fanout_index: i32| NewRunStep {
        id: StepId::new(),
        run_id: run,
        position: 0,
        attempt: 1,
        fanout_index,
        phase_name: "implement".to_owned(),
        agent_id: Some(ids::AGENT_CLAUDE),
        model: Some("opus".to_owned()),
        role: StepRole::Step,
        parent_step_id: None,
        node_key: None,
    };
    let store = &db.store;
    let claim = |run: RunId| async move {
        assert_eq!(
            store
                .claim_run(run, ids::BOX, owner, at, TimeDelta::minutes(5))
                .await
                .expect("claim_run"),
            Claim::Admitted,
            "the claim is admitted"
        );
    };
    let running = |run: RunId, fanout_index: i32| async move {
        let step = store
            .create_step(new_step(run, fanout_index))
            .await
            .expect("create_step")
            .id;
        assert!(
            store
                .transition_step(step, StepStatus::Pending, StepStatus::Running, at)
                .await
                .expect("transition_step"),
            "pending -> running"
        );
        step
    };

    // A gate park on ANA-2.
    let gate_run = store
        .create_run(new_run(ids::HTUI_ANA_2))
        .await
        .expect("create_run")
        .id;
    claim(gate_run).await;
    let gate_step = running(gate_run, 0).await;
    assert_eq!(
        store
            .park_step(StepFence::Lease(owner), gate_step)
            .await
            .expect("park_step"),
        ParkOutcome::Parked
    );

    // A selection park on a minted item.
    let minted = store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            title: "selection".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("mint_item")
        .id;
    let selection_run = store
        .create_run(new_run(minted))
        .await
        .expect("create_run")
        .id;
    claim(selection_run).await;
    for fanout_index in [0, 1] {
        let step = running(selection_run, fanout_index).await;
        assert!(
            store
                .transition_step(step, StepStatus::Running, StepStatus::Done, at)
                .await
                .expect("transition_step"),
            "running -> done"
        );
    }
    assert!(
        store
            .transition_run(
                selection_run,
                RunStatus::Running,
                RunStatus::AwaitingApproval,
                at
            )
            .await
            .expect("transition_run")
    );
    // MOD-144: the item is not moved; its `needs you` is the run's `awaiting_approval`.

    // A blocked item whose run a workspace stop interrupted: a stopped run is live (MOD-125 D2),
    // so the mirror's run-status list names it as Postgres' does.
    let stopped = store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            title: "stopped".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("mint_item")
        .id;
    let stopped_run = store
        .create_run(new_run(stopped))
        .await
        .expect("create_run")
        .id;
    claim(stopped_run).await;
    let stopped_step = running(stopped_run, 0).await;
    assert!(
        store
            .transition_run(stopped_run, RunStatus::Running, RunStatus::Stopped, at)
            .await
            .expect("transition_run"),
        "running -> stopped"
    );
    assert!(
        store
            .transition_step(stopped_step, StepStatus::Running, StepStatus::Stopped, at)
            .await
            .expect("transition_step"),
        "running -> stopped"
    );
    assert!(
        store
            .transition(stopped, Status::Open, Status::Blocked)
            .await
            .expect("transition")
    );

    // FEAT-3 blocked behind its queued `RUN_2`: a candidate carries its queued run, not only its
    // parked ones (T2's unblock reads the first active run, blueprint H-1).
    assert!(
        store
            .transition(ids::HTUI_FEAT_3, Status::Open, Status::Blocked)
            .await
            .expect("transition")
    );

    // A blocked `agy` item.
    assert!(
        store
            .transition(ids::AGY_FIX_1, Status::Open, Status::Blocked)
            .await
            .expect("transition")
    );

    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let scope = platform_scope().await;
    let mirrored = cache
        .waiting_candidates(&scope)
        .await
        .expect("mirror waiting_candidates");
    assert_eq!(
        mirrored,
        store
            .waiting_candidates(&scope)
            .await
            .expect("pg waiting_candidates"),
        "the mirror and Postgres list the same rows, whole"
    );
    assert_eq!(
        mirrored.iter().map(|row| row.item.id).collect::<Vec<_>>(),
        vec![
            ids::HTUI_ANA_2,
            ids::HTUI_FEAT_2,
            ids::HTUI_FEAT_3,
            minted,
            stopped,
            ids::HTUI_TOOL_1,
            ids::AGY_FIX_1
        ],
        "ANA-2, FEAT-2, FEAT-3, the two minted items and TOOL-1 of htui, then agy's FIX-1"
    );
    assert_eq!(
        mirrored
            .iter()
            .find(|row| row.item.id == stopped)
            .expect("the stopped item is listed")
            .runs
            .iter()
            .map(|(run, steps)| (run.id, run.status, steps.len()))
            .collect::<Vec<_>>(),
        vec![(stopped_run, RunStatus::Stopped, 1)],
        "the stopped item carries its stopped run and the run's stopped step"
    );
    let statuses: Vec<RunStatus> =
        sqlx::query_scalar("SELECT status FROM run WHERE project_id = ANY($1)")
            .bind(
                scope
                    .project_ids
                    .iter()
                    .map(|id| id.as_uuid())
                    .collect::<Vec<_>>(),
            )
            .fetch_all(&db.pool)
            .await
            .expect("read the scope's run statuses");
    assert!(
        statuses.contains(&RunStatus::Stopped),
        "control: the scope holds a stopped run"
    );
    let active = cache.active_runs(&scope).await.expect("mirror active_runs");
    assert_eq!(
        active,
        store.active_runs(&scope).await.expect("pg active_runs"),
        "the mirror and Postgres count the top bar's runs alike"
    );
    assert_eq!(
        active,
        statuses.iter().filter(|status| status.on_top_bar()).count(),
        "the count is the scope's runs on the top bar: the stopped run is not one"
    );
    let feat_3 = mirrored
        .iter()
        .find(|row| row.item.id == ids::HTUI_FEAT_3)
        .expect("FEAT-3 is listed");
    assert_eq!(
        feat_3
            .runs
            .iter()
            .map(|(run, steps)| (run.id, run.status, steps.len()))
            .collect::<Vec<_>>(),
        vec![(ids::RUN_2, RunStatus::Queued, 1)],
        "FEAT-3 carries its queued run and the run's pending step"
    );

    teardown(db, &[&cache]).await;
}

/// MOD-144: the mirror derives an item's state as Postgres does: the latest graph run, `cancelled`
/// skipped, by `(queued_at, id)` (SQLite's correlated subquery against Postgres' `LATERAL`), in
/// `items` (`ready`, `states`), `links`, `upstream_summaries` and `requirement_coverage`. ANA-2
/// gains a `done` run and then a later cancelled one, so a mirror that took the cancel for the
/// latest run would read it `open` (and ready) where Postgres reads it `done`; agy FIX-1 is
/// blocked by hand. With the three demo projects every [`ItemState`] is some item's.
#[tokio::test]
async fn the_mirror_derives_item_state_like_postgres() {
    use htui_core::model::{
        Claim, GraphSnapshot, Isolation, ItemId, NewRun, RunMode, RunStatus, SnapshotGraph,
        SnapshotSettings,
    };

    let Some(db) = common::demo_db().await else {
        return;
    };
    let store = &db.store;
    let at = Utc::now().trunc_subsecs(6);
    let new_run = |id: RunId, queued_at| NewRun {
        workspace_id: None,
        id,
        project_id: ids::PROJECT_HTUI,
        item_id: ids::HTUI_ANA_2,
        mode: RunMode::Manual,
        target_box_id: ids::BOX,
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
            topology: "sha256:cache".to_owned(),
            mode: RunMode::Manual,
            phases: Vec::new(),
            settings: SnapshotSettings {
                default_isolation: Isolation::Worktree,
                per_token_cap_run: None,
                per_token_cap_batch: None,
                max_fan_out: 4,
                max_agents_per_run: 8,
                max_parallel_sessions: SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS,
                max_plan_revisions: SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS,
                max_child_runs: SnapshotSettings::DEFAULT_MAX_CHILD_RUNS,
                runtime_changes: htui_core::model::RuntimeChanges::Ask,
            },
            scope: None,
            personas: Vec::new(),
            memory: None,
            edges: Vec::new(),
            source: None,
        },
        repo_scope: Vec::new(),
        queued_at,
        batch_id: None,
    };
    let (done, cancelled) = (RunId::new(), RunId::new());
    store
        .create_run(new_run(done, at))
        .await
        .expect("create_run");
    assert_eq!(
        store
            .claim_run(
                done,
                ids::BOX,
                uuid::Uuid::now_v7(),
                at,
                TimeDelta::minutes(5)
            )
            .await
            .expect("claim_run"),
        Claim::Admitted
    );
    store
        .finish_run(done, RunStatus::Done, None, at)
        .await
        .expect("finish_run");
    store
        .create_run(new_run(cancelled, at + TimeDelta::seconds(1)))
        .await
        .expect("create_run");
    store
        .finish_run(cancelled, RunStatus::Cancelled, None, at)
        .await
        .expect("finish_run");
    assert!(
        store
            .transition(ids::AGY_FIX_1, Status::Open, Status::Blocked)
            .await
            .expect("transition")
    );

    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");
    let scope = Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: all_projects(),
    };

    let mut filters = vec![
        ItemFilter::default(),
        ItemFilter {
            ready: Some(true),
            ..ItemFilter::default()
        },
        ItemFilter {
            ready: Some(false),
            ..ItemFilter::default()
        },
        ItemFilter {
            states: Some(vec![ItemState::NeedsYou, ItemState::Done]),
            ..ItemFilter::default()
        },
        ItemFilter {
            states: Some(vec![ItemState::Open]),
            statuses: Some(vec![Status::Open]),
            ready: Some(true),
            ..ItemFilter::default()
        },
    ];
    filters.extend(ItemState::ALL.iter().map(|state| ItemFilter {
        states: Some(vec![*state]),
        ..ItemFilter::default()
    }));
    for filter in &filters {
        let mirrored = cache.items(&scope, filter).await.expect("cache items");
        assert_eq!(
            mirrored,
            store.items(&scope, filter).await.expect("pg items"),
            "items for {filter:?}"
        );
        if let Some([state]) = filter.states.as_deref() {
            assert!(
                !mirrored.is_empty(),
                "the demo holds a `{}` item",
                state.word()
            );
            assert!(
                mirrored.iter().all(|row| row.state() == *state),
                "every row of {filter:?} reads `{}`",
                state.word()
            );
        }
    }
    let ana_2 = cache
        .items(&scope, &ItemFilter::default())
        .await
        .expect("cache items")
        .into_iter()
        .find(|row| row.id == ids::HTUI_ANA_2)
        .expect("ANA-2 is listed");
    assert_eq!(
        (ana_2.latest_run_status, ana_2.state()),
        (Some(RunStatus::Done), ItemState::Done),
        "the later cancel is skipped"
    );

    let items: [ItemId; 6] = [
        ids::HTUI_ANA_2,
        ids::HTUI_FEAT_1,
        ids::HTUI_FEAT_2,
        ids::HTUI_TOOL_1,
        ids::AGY_FIX_1,
        ids::VULKAN_FEAT_1,
    ];
    for item in items {
        assert_eq!(
            canonical(cache.links(item, 2).await.expect("cache links")),
            canonical(store.links(item, 2).await.expect("pg links")),
            "links of {item}"
        );
    }
    for root in [ids::AGY_FIX_1, ids::HTUI_FEAT_2, ids::HTUI_ANA_2] {
        for bound in [
            PromptScope::project_only(ids::PROJECT_AGY),
            PromptScope::project_only(ids::PROJECT_HTUI),
        ] {
            assert_eq!(
                cache
                    .upstream_summaries(root, 2, &bound)
                    .await
                    .expect("cache upstream_summaries"),
                store
                    .upstream_summaries(root, 2, &bound)
                    .await
                    .expect("pg upstream_summaries"),
                "upstream_summaries({root}, 2, {bound:?})"
            );
        }
    }
    assert_eq!(
        cache
            .requirement_coverage(ids::REQ_STO_1)
            .await
            .expect("cache requirement_coverage"),
        store
            .requirement_coverage(ids::REQ_STO_1)
            .await
            .expect("pg requirement_coverage"),
        "coverage rows, latest run status included"
    );

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
                             exit_code = 3, selected = true, usage = '{\"in\":1}'::jsonb, \
                             gate_note = 'parked: no tests' \
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
            step.gate_note.as_deref(),
        ),
        (
            Some(htui_core::model::VerifyOutcome::Fail),
            Some(lease_at),
            Some(3),
            Some(true),
            Some(serde_json::json!({ "in": 1 })),
            Some("claude"),
            Some("parked: no tests"),
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

/// MOD-37 milestone 5 (R-48): `run_step.opening` (`cache_migrations/0005_run_step_opening.sql`)
/// reaches the mirror. The fixture leaves it NULL on every step, so an equality against Postgres
/// proves nothing about it: this writes it through `record_opening`, refreshes, and reads the
/// summary back off the mirror.
#[tokio::test]
async fn the_0005_opening_reaches_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    db.store
        .record_opening(ids::STEP_IMPL, StepOpening::ResumeFailed)
        .await
        .expect("record the opening");
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let mirrored_runs = cache.runs(ids::HTUI_FEAT_1).await.expect("cache runs");
    let step = mirrored_runs
        .iter()
        .flat_map(|run| run.steps.iter())
        .find(|step| step.id == ids::STEP_IMPL)
        .expect("the implement step is mirrored");
    assert_eq!(
        step.opening,
        Some(StepOpening::ResumeFailed),
        "`run_step.opening` comes off the mirror"
    );
    assert_eq!(
        mirrored_runs,
        db.store.runs(ids::HTUI_FEAT_1).await.expect("pg runs"),
        "and the two backends project the whole list alike"
    );

    teardown(db, &[&cache]).await;
}

/// MOD-146 plan D6 (blueprint F-5): `project.type` (`cache_migrations/0007_project_type.sql`)
/// reaches the mirror. Every demo project is `software`, which is also the mirror column's
/// default, so an equality against Postgres proves nothing about it: this writes another type by
/// raw SQL (the project trigger bumps `updated_at`), refreshes, and reads it off the mirror.
#[tokio::test]
async fn the_0007_project_type_reaches_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    sqlx::query("UPDATE project SET type = 'research' WHERE id = $1")
        .bind(ids::PROJECT_AGY.as_uuid())
        .execute(&db.pool)
        .await
        .expect("type the project research");
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let mirrored = cache
        .project(ids::PROJECT_AGY)
        .await
        .expect("cache project")
        .expect("the project is mirrored");
    assert_eq!(
        mirrored.project_type, "research",
        "`project.type` comes off the mirror"
    );
    assert_eq!(
        Some(mirrored),
        db.store
            .project(ids::PROJECT_AGY)
            .await
            .expect("pg project"),
        "and the two backends project the row alike"
    );

    teardown(db, &[&cache]).await;
}

/// MOD-147 D11: the offline key qualifier reads a project's kinds off the mirror
/// (`CacheStore::item_kinds`), in the order Postgres lists them (`position`, then `prefix` bytes):
/// a member's seeded five, a scope row's `WS`, nothing for a project the mirror does not hold; and
/// `Backend::item_kinds` answers offline from the mirror.
#[tokio::test]
async fn the_mirror_lists_item_kinds_like_postgres() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let mut projects = all_projects();
    projects.push(ids::PROJECT_SCOPE_PLATFORM);
    run_pass(&db.pool, &cache, &projects, &settings(&db, 20))
        .await
        .expect("one pass");

    let pg = db
        .store
        .item_kinds(ids::PROJECT_HTUI)
        .await
        .expect("pg item_kinds");
    assert_eq!(pg.len(), 5, "htui's seeded kinds");
    let mirrored = cache
        .item_kinds(ids::PROJECT_HTUI)
        .await
        .expect("cache item_kinds");
    assert_eq!(mirrored, pg, "the mirror lists htui's kinds like Postgres");
    let scope_row = cache
        .item_kinds(ids::PROJECT_SCOPE_PLATFORM)
        .await
        .expect("cache item_kinds of the scope row");
    assert_eq!(
        scope_row
            .iter()
            .map(|kind| kind.prefix.as_str())
            .collect::<Vec<_>>(),
        ["WS"],
        "the scope row's one kind"
    );
    assert_eq!(
        scope_row,
        db.store
            .item_kinds(ids::PROJECT_SCOPE_PLATFORM)
            .await
            .expect("pg item_kinds of the scope row"),
        "and Postgres lists it alike"
    );
    assert!(
        cache
            .item_kinds(ProjectId::new())
            .await
            .expect("an unknown project reads")
            .is_empty(),
        "a project the mirror does not hold has no kinds"
    );

    let backend = Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    };
    assert_eq!(
        backend
            .item_kinds(ids::PROJECT_HTUI)
            .await
            .expect("an offline kinds read answers from the mirror"),
        pg,
        "the offline backend reads the mirror"
    );

    teardown(db, &[&cache]).await;
}

/// [`ScopeKeys`] with its projects in id order: Postgres answers them in no particular order.
fn keys_by_id(mut keys: ScopeKeys) -> ScopeKeys {
    keys.projects.sort_by_key(|(id, ..)| id.as_uuid());
    keys
}

/// The `(slug, prefixes)` [`ScopeKeys`] holds for `project`, if any.
fn keys_of(keys: &ScopeKeys, project: ProjectId) -> Option<(&str, Vec<&str>)> {
    keys.projects
        .iter()
        .find(|(id, ..)| *id == project)
        .map(|(_, slug, prefixes)| (slug.as_str(), prefixes.iter().map(String::as_str).collect()))
}

/// MOD-170: the waiting key qualifier's one read. Online it is one Postgres query
/// (`PgStore::waiting_keys`); offline the mirror answers it per project. Both answer alike: the
/// workspace's scope row (flagged, and listed even when not asked by id), each held project's slug
/// and kind prefixes (empty for a project with no kinds), and nothing for a project the store does
/// not hold. Without a workspace no row is flagged and the scope row is listed only when asked.
#[tokio::test]
async fn the_mirror_answers_waiting_keys_like_postgres() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let bare = ProjectId::new();
    sqlx::query("INSERT INTO project (id, slug, name, created_by) VALUES ($1, 'bare', 'Bare', $2)")
        .bind(bare.as_uuid())
        .bind(ids::USER.as_uuid())
        .execute(&db.pool)
        .await
        .expect("insert a project with no kinds");
    let unknown = ProjectId::new();
    let members = platform_scope().await.project_ids;
    let mut asked = members.clone();
    asked.extend([bare, unknown]);

    let wide = db
        .store
        .waiting_keys(Some(ids::WORKSPACE_PLATFORM), &asked)
        .await
        .expect("pg waiting_keys with a workspace");
    assert_eq!(wide.scope_row, Some(ids::PROJECT_SCOPE_PLATFORM));
    assert_eq!(
        wide.projects.len(),
        members.len() + 2,
        "members, bare and the scope row"
    );
    assert_eq!(
        keys_of(&wide, ids::PROJECT_SCOPE_PLATFORM).map(|(_, prefixes)| prefixes),
        Some(vec!["WS"]),
        "the scope row is listed with its one kind though not asked by id"
    );
    for member in &members {
        let pg_prefixes: Vec<String> = db
            .store
            .item_kinds(*member)
            .await
            .expect("pg item_kinds")
            .into_iter()
            .map(|kind| kind.prefix)
            .collect();
        let (slug, prefixes) = keys_of(&wide, *member).expect("a member is listed");
        assert_eq!(
            slug,
            db.store
                .project(*member)
                .await
                .expect("pg project")
                .expect("held")
                .slug
        );
        assert_eq!(prefixes, pg_prefixes, "in kind order, like item_kinds");
        assert!(prefixes.contains(&"FEAT"), "Platform's members share FEAT");
    }
    assert_eq!(
        keys_of(&wide, bare),
        Some(("bare", vec![])),
        "no kinds, empty prefixes"
    );
    assert_eq!(
        keys_of(&wide, unknown),
        None,
        "a project the store does not hold"
    );

    let narrow = db
        .store
        .waiting_keys(None, &asked)
        .await
        .expect("pg waiting_keys without a workspace");
    assert_eq!(narrow.scope_row, None);
    assert_eq!(narrow.projects.len(), members.len() + 1, "members and bare");
    assert_eq!(keys_of(&narrow, ids::PROJECT_SCOPE_PLATFORM), None);

    let by_id = db
        .store
        .waiting_keys(None, &[ids::PROJECT_SCOPE_PLATFORM])
        .await
        .expect("pg waiting_keys of the scope row by id");
    assert_eq!(by_id.scope_row, None, "not flagged without a workspace");
    assert_eq!(
        keys_of(&by_id, ids::PROJECT_SCOPE_PLATFORM),
        Some(("@platform", vec!["WS"]))
    );

    assert_eq!(
        db.store
            .waiting_keys(None, &[])
            .await
            .expect("an empty ask"),
        ScopeKeys::default()
    );

    let cache = open_cache(&db).await;
    let mut mirrored = all_projects();
    mirrored.extend([ids::PROJECT_SCOPE_PLATFORM, bare]);
    run_pass(&db.pool, &cache, &mirrored, &settings(&db, 20))
        .await
        .expect("one pass");
    let offline = Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    };
    // The scope row reached both ways, by id and through the workspace: listed once.
    let both_ask = vec![ids::PROJECT_SCOPE_PLATFORM, members[0]];
    let both = db
        .store
        .waiting_keys(Some(ids::WORKSPACE_PLATFORM), &both_ask)
        .await
        .expect("pg waiting_keys with the scope row asked by id too");
    assert_eq!(both.scope_row, Some(ids::PROJECT_SCOPE_PLATFORM));
    assert_eq!(both.projects.len(), 2, "the scope row once, and the member");
    for (workspace, ask, pg) in [
        (Some(ids::WORKSPACE_PLATFORM), both_ask, both),
        (Some(ids::WORKSPACE_PLATFORM), asked.clone(), wide),
        (None, asked.clone(), narrow),
        (None, vec![ids::PROJECT_SCOPE_PLATFORM], by_id),
    ] {
        assert_eq!(
            keys_by_id(
                offline
                    .waiting_keys(workspace, &ask)
                    .await
                    .expect("offline waiting_keys")
            ),
            keys_by_id(pg),
            "the mirror answers like Postgres for {workspace:?}"
        );
    }

    teardown(db, &[&cache]).await;
}

/// MOD-138 M2 D5 (`cache_migrations/0008_run_step_branch.sql`): `run_step.branch` reaches the
/// mirror. No demo step carries a decision, so an equality against Postgres proves nothing about
/// it: this writes one by raw SQL (the step trigger bumps `updated_at`), refreshes, and reads it off
/// the mirror through both reads that build a `RunStep`: `run_steps` on the done `RUN_1`, and
/// `waiting_candidates` on the queued `RUN_2` once its item, FEAT-3, is blocked. The CHECK does not
/// look at the step's status, so a pending step can carry one here.
#[tokio::test]
async fn the_0008_branch_reaches_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let branch = serde_json::json!({"deploy": true, "docs": false});
    let waiting_branch = serde_json::json!({"ship": false});

    for (step, value) in [
        (ids::STEP_IMPL, &branch),
        (ids::STEP_R2_PRD, &waiting_branch),
    ] {
        sqlx::query("UPDATE run_step SET branch = $2 WHERE id = $1")
            .bind(step.as_uuid())
            .bind(value)
            .execute(&db.pool)
            .await
            .expect("decide the step's branch");
    }
    assert!(
        db.store
            .transition(ids::HTUI_FEAT_3, Status::Open, Status::Blocked)
            .await
            .expect("transition"),
        "FEAT-3 is blocked, so its queued RUN_2 is a waiting candidate's"
    );
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let pg_steps = db.store.run_steps(ids::RUN_1).await.expect("pg run steps");
    let mirrored = cache.run_steps(ids::RUN_1).await.expect("cache run steps");
    let step = mirrored
        .iter()
        .find(|step| step.id == ids::STEP_IMPL)
        .expect("the implement step is mirrored");
    assert_eq!(
        step.branch.as_ref(),
        Some(&branch),
        "`run_step.branch` comes off the mirror"
    );
    assert_eq!(
        mirrored, pg_steps,
        "and the two backends project the steps alike"
    );
    let others = mirrored.iter().filter(|step| step.id != ids::STEP_IMPL);
    assert!(
        others.into_iter().all(|step| step.branch.is_none()),
        "an undecided step mirrors none"
    );

    let scope = platform_scope().await;
    let candidates = cache
        .waiting_candidates(&scope)
        .await
        .expect("mirror waiting_candidates");
    assert_eq!(
        candidates,
        db.store
            .waiting_candidates(&scope)
            .await
            .expect("pg waiting_candidates"),
        "the mirror and Postgres list the same candidates, steps whole"
    );
    let waiting = candidates
        .iter()
        .find(|row| row.item.id == ids::HTUI_FEAT_3)
        .expect("FEAT-3 is a waiting candidate")
        .runs
        .iter()
        .flat_map(|(_, steps)| steps)
        .find(|step| step.id == ids::STEP_R2_PRD)
        .expect("RUN_2's step rides FEAT-3's candidate");
    assert_eq!(
        waiting.branch.as_ref(),
        Some(&waiting_branch),
        "`run_step.branch` comes off the mirror through `waiting_candidates` too"
    );

    teardown(db, &[&cache]).await;
}

/// MOD-139 M1 (`cache_migrations/0010_divided_fanout.sql`): `run_step.role`, `parent_step_id` and
/// `node_key` reach the mirror. No demo step is anything but an ordinary step, so this writes a
/// planner, a part under it and a stage on `RUN_1` through `create_step`, refreshes, and reads the
/// three off the mirror; every fixture step still reads `Step` with no parent and no key.
#[tokio::test]
async fn the_0010_step_role_reaches_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let new = |position: i32, role: StepRole, parent: Option<StepId>, key: Option<&str>| {
        htui_core::model::NewRunStep {
            id: StepId::new(),
            run_id: ids::RUN_1,
            position,
            attempt: 1,
            fanout_index: 0,
            phase_name: "implement".to_owned(),
            agent_id: None,
            model: None,
            role,
            parent_step_id: parent,
            node_key: key.map(str::to_owned),
        }
    };
    let planner = db
        .store
        .create_step(new(50, StepRole::Planner, None, None))
        .await
        .expect("a planner step");
    let part = db
        .store
        .create_step(new(
            51,
            StepRole::Part,
            Some(planner.id),
            Some("implement/api"),
        ))
        .await
        .expect("a part step");
    let stage = db
        .store
        .create_step(new(-1, StepRole::Stage, None, Some("collect")))
        .await
        .expect("a stage step");
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    let pg_steps = db.store.run_steps(ids::RUN_1).await.expect("pg run steps");
    let mirrored = cache.run_steps(ids::RUN_1).await.expect("cache run steps");
    assert_eq!(
        mirrored, pg_steps,
        "the two backends project the steps alike"
    );
    for written in [&planner, &part, &stage] {
        let row = mirrored
            .iter()
            .find(|step| step.id == written.id)
            .expect("the written step is mirrored");
        assert_eq!(
            (row.role, row.parent_step_id, row.node_key.as_deref()),
            (
                written.role,
                written.parent_step_id,
                written.node_key.as_deref()
            ),
            "role, parent and node key come off the mirror"
        );
    }
    assert_eq!(
        (part.role, part.parent_step_id, part.node_key.as_deref()),
        (StepRole::Part, Some(planner.id), Some("implement/api")),
        "Postgres stored what was asked"
    );
    let fixture = mirrored
        .iter()
        .filter(|step| ![planner.id, part.id, stage.id].contains(&step.id));
    assert!(
        fixture.into_iter().all(|step| step.role == StepRole::Step
            && step.parent_step_id.is_none()
            && step.node_key.is_none()),
        "every fixture step is an ordinary step"
    );

    teardown(db, &[&cache]).await;
}

/// MOD-139 M1 (plan D2): `run_plan` is mirrored on its own `updated_at` cursor. A plan written to
/// Postgres through `insert_run_plan` reads back off the mirror, and after its status moves (raw
/// SQL, as M2's admission will; `trg_run_plan_updated_at` moves `updated_at`) the next pass
/// carries the new status and refs.
#[tokio::test]
async fn the_0010_run_plan_reaches_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let leased: bool = sqlx::query_scalar("SELECT lease_owner IS NOT NULL FROM run WHERE id = $1")
        .bind(ids::RUN_1.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("RUN_1's lease");
    assert!(
        !leased,
        "the fixture's RUN_1 is unleased, so StepFence::Unleased passes"
    );
    let planner = db
        .store
        .create_step(htui_core::model::NewRunStep {
            id: StepId::new(),
            run_id: ids::RUN_1,
            position: 60,
            attempt: 1,
            fanout_index: 0,
            phase_name: "implement".to_owned(),
            agent_id: None,
            model: None,
            role: StepRole::Planner,
            parent_step_id: None,
            node_key: None,
        })
        .await
        .expect("a planner step");
    let plan = db
        .store
        .insert_run_plan(
            StepFence::Unleased,
            NewRunPlan {
                id: RunPlanId::new(),
                run_id: ids::RUN_1,
                revision: 0,
                planner_step_id: planner.id,
                scope: PlanScope::Parts,
                base_position: 61,
                body: json!({"parts": [{"key": "api"}]}),
                digest: "sha256:mirror".to_owned(),
            },
        )
        .await
        .expect("a proposed plan");
    assert_eq!(
        (plan.status, plan.base_refs.as_ref()),
        (PlanStatus::Proposed, None),
        "inserted as proposed, with no refs"
    );
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the first pass");
    let pg_plans = db.store.run_plans(ids::RUN_1).await.expect("pg run plans");
    assert_eq!(
        pg_plans,
        vec![plan.clone()],
        "Postgres answers the one plan"
    );
    assert_eq!(
        cache.run_plans(ids::RUN_1).await.expect("cache run plans"),
        pg_plans,
        "the mirror answers the plan as Postgres does"
    );

    let refs = json!({"r": "abc"});
    sqlx::query("UPDATE run_plan SET status = 'admitted', base_refs = $2 WHERE id = $1")
        .bind(plan.id.as_uuid())
        .bind(&refs)
        .execute(&db.pool)
        .await
        .expect("admit the plan");
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the second pass");
    let mirrored = cache.run_plans(ids::RUN_1).await.expect("cache run plans");
    assert_eq!(mirrored.len(), 1, "still the one revision");
    assert_eq!(
        (mirrored[0].status, mirrored[0].base_refs.as_ref()),
        (PlanStatus::Admitted, Some(&refs)),
        "the status move rode updated_at to the mirror"
    );
    assert!(
        mirrored[0].updated_at > plan.updated_at,
        "the trigger moved updated_at"
    );
    assert_eq!(
        mirrored,
        db.store.run_plans(ids::RUN_1).await.expect("pg run plans"),
        "the two backends agree after the move"
    );
    assert!(
        cache
            .run_plans(RunId::new())
            .await
            .expect("an unknown run")
            .is_empty(),
        "an unknown run has no plans"
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

/// `resolve_inputs`' three-armed `CASE` rank and its two arms (MOD-73) are the mirror's, not just
/// Postgres's (T2 audit).
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
/// - **v4, hand-written** (`produced_by_step_id IS NULL`, the hand-written arm). It is newer than
///   `RUN_3`'s own v2 and older than v5, so it wins from one seat and loses from the other.
/// - **v5, produced by `STEP_PLAN`** (rank 1 from `RUN_3`'s seat, rank 0 from `RUN_1`'s). `RUN_1` is
///   the run `STEP_PLAN` belongs to and its `selected` is `NULL`, so the row is eligible from both
///   seats and only the rank tells them apart.
///
/// From `RUN_3` the answer is v4: the step-produced arm picks v2 (rank 0 beats a higher-versioned
/// rank 1), and v4 is newer. From `RUN_1` it is v5, rank 0 there and newer than v4. A mirror that
/// dropped the `CASE` would answer v5 from `RUN_3`; one that dropped the arm partition would answer
/// v2. Both are asserted equal to Postgres's own answer as well as by id, so a mirror that ranks
/// differently *or* a Postgres that does fails.
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
                "INSERT INTO document (project_id, item_id, kind, version, title, body, \
                                       produced_by_step_id, created_by) \
                 VALUES ($1, $2, 'research', $3, $4, 'body', $5, $6) RETURNING id",
            )
            .bind(ids::PROJECT_HTUI.as_uuid())
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
        Some(hand_written_v4),
        "from RUN_3's seat the step-produced pick is its own v2 (rank 0 beats v5's rank 1), and \
         the hand-written v4 is newer, so v4 is read (MOD-73)"
    );
    assert_eq!(
        winner(ids::RUN_1).await,
        Some(other_runs_v5),
        "from RUN_1's seat v5 is the rank-0 row and newer than the hand-written v4, so the arms \
         are told apart and not merely ordered"
    );

    teardown(db, &[&cache]).await;
}

/// MOD-139 D12: the mirror skips a divided part's document the way Postgres does, and keeps a
/// hand-written one beside it (whose `s.role` is `NULL` through the `LEFT JOIN`).
///
/// On `RUN_1` a planner (position 60) and a part under it (61) are written, then a hand-written
/// `notes` v1 and the part's `notes` v2 on the run's item. Both tables are seeded **before** the
/// one pass: `run_pass` commits `document` before `run_step`, and a reader between the two would
/// see the part's document with no step row (the accepted mid-pass lag `CacheStore::resolve_inputs`
/// documents). From `RUN_1`'s seat the part's newer v2 is skipped, so v1 is read on both engines; a
/// mirror without the role test would answer v2.
#[tokio::test]
async fn the_mirror_skips_part_documents_the_way_postgres_does() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let step = |position: i32, role: StepRole, parent: Option<StepId>, key: Option<&str>| {
        htui_core::model::NewRunStep {
            id: StepId::new(),
            run_id: ids::RUN_1,
            position,
            attempt: 1,
            fanout_index: 0,
            phase_name: key.unwrap_or("implement:plan").to_owned(),
            agent_id: None,
            model: None,
            role,
            parent_step_id: parent,
            node_key: key.map(str::to_owned),
        }
    };
    let planner = db
        .store
        .create_step(step(60, StepRole::Planner, None, None))
        .await
        .expect("the planner row")
        .id;
    let part = db
        .store
        .create_step(step(
            61,
            StepRole::Part,
            Some(planner),
            Some("implement/api"),
        ))
        .await
        .expect("the part row")
        .id;

    let insert = |version: i32, step: Option<StepId>| {
        let pool = db.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO document (project_id, item_id, kind, version, title, body, \
                                       produced_by_step_id, created_by) \
                 VALUES ($1, $2, 'notes', $3, $4, 'body', $5, $6) RETURNING id",
            )
            .bind(ids::PROJECT_HTUI.as_uuid())
            .bind(ids::HTUI_FEAT_1.as_uuid())
            .bind(version)
            .bind(format!("notes v{version}"))
            .bind(step.map(StepId::as_uuid))
            .bind(ids::USER.as_uuid())
            .fetch_one(&pool)
            .await
            .map(|row| row.get::<uuid::Uuid, _>("id"))
            .expect("a notes row the resolver has to judge")
        }
    };
    let hand_written = insert(1, None).await;
    insert(2, Some(part)).await;

    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the pass that carries the two steps and the two documents");

    let asked = ["notes".to_owned()];
    let mirrored = cache
        .resolve_inputs(ids::HTUI_FEAT_1, ids::RUN_1, &asked)
        .await
        .expect("the mirror resolves");
    assert_eq!(
        mirrored,
        db.store
            .resolve_inputs(ids::HTUI_FEAT_1, ids::RUN_1, &asked)
            .await
            .expect("Postgres resolves"),
        "the two engines skip the part's document alike"
    );
    assert_eq!(
        mirrored
            .first()
            .and_then(|row| row.document.as_ref())
            .map(|document| document.id.as_uuid()),
        Some(hand_written),
        "the part's newer v2 is no input; the hand-written v1 is read"
    );

    teardown(db, &[&cache]).await;
}

/// Plants one `document` row straight into the mirror, in its encodings: ids as hyphenated text,
/// `created_at` as epoch microseconds. Postgres never sees it.
async fn plant_mirrored_document(
    cache: &CacheStore,
    kind: &str,
    version: i32,
    step: Option<StepId>,
) -> DocumentId {
    let id = DocumentId::new();
    sqlx::query(
        "INSERT INTO document (id, project_id, item_id, kind, version, title, body, \
                               produced_by_step_id, created_by, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'body', ?, ?, 0)",
    )
    .bind(id.as_uuid().to_string())
    .bind(ids::PROJECT_HTUI.as_uuid().to_string())
    .bind(ids::HTUI_ANA_1.as_uuid().to_string())
    .bind(kind)
    .bind(version)
    .bind(format!("{kind} v{version}"))
    .bind(step.map(|step| step.as_uuid().to_string()))
    .bind(ids::USER.as_uuid().to_string())
    .execute(cache.pool())
    .await
    .unwrap_or_else(|e| panic!("plant {kind} v{version} in the mirror: {e}"));
    id
}

/// A document whose producing step the mirror does not hold yet competes in the step-produced
/// arm, never the hand-written one (MOD-73 plan D3, review LOW-1).
///
/// This is the mid-pass shape `CacheStore::resolve_inputs` documents: `run_pass` commits
/// `document` before `run_step`, so a reader between the two sees `produced_by_step_id` naming a
/// step whose `LEFT JOIN` comes back empty. Postgres cannot hold such a row (the column is a
/// foreign key), so the case is mirror-only and the rows are planted straight into SQLite after a
/// refresh. The inner partition is keyed on `d.produced_by_step_id IS NULL`; keyed on the join's
/// `s.id IS NULL` instead, the step-less row would join the hand-written arm and, being the
/// newest there, be read over a real pick. Each assertion below fails under that key:
///
/// - **winner v1 + step-less v2.** Under D3 both sit in the step-produced arm, where v1 ranks 0
///   (its step is `RUN_1`'s) and v2 ranks 2, so v1 is read. Keyed on the join, v2 would be the
///   lone hand-written row and newer than v1, so v2 would be read.
/// - **winner v1 + hand-written v2 + step-less v3.** Under D3 the step-produced arm picks v1, the
///   hand-written arm v2, and v2 is newer. Keyed on the join, v2 and v3 share the hand-written arm,
///   v3 out-versions v2 there, and v3 would be read.
#[tokio::test]
async fn a_document_whose_step_has_not_arrived_is_never_hand_written() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("the pass that mirrors the fixture's steps");

    let not_yet_mirrored = StepId::new();
    let mirrored_steps = |step: StepId| {
        let cache = &cache;
        async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM run_step WHERE id = ?")
                .bind(step.as_uuid().to_string())
                .fetch_one(cache.pool())
                .await
                .expect("count a mirrored step")
        }
    };
    assert_eq!(
        mirrored_steps(ids::STEP_PLAN).await,
        1,
        "the winner's step is mirrored"
    );
    assert_eq!(
        mirrored_steps(not_yet_mirrored).await,
        0,
        "the step-less row's step is not"
    );

    let read = |kind: &'static str| {
        let cache = &cache;
        async move {
            cache
                .resolve_inputs(ids::HTUI_ANA_1, ids::RUN_1, &[kind.to_owned()])
                .await
                .expect("the mirror resolves")
                .first()
                .and_then(|row| row.document.as_ref())
                .map(|document| document.id)
        }
    };

    let kind = "mid-pass-over-a-winner";
    let winner = plant_mirrored_document(&cache, kind, 1, Some(ids::STEP_PLAN)).await;
    plant_mirrored_document(&cache, kind, 2, Some(not_yet_mirrored)).await;
    assert_eq!(
        read(kind).await,
        Some(winner),
        "the step-less v2 ranks 2 in the step-produced arm, below RUN_1's own v1, and is not a \
         hand-written row that out-versions it"
    );

    let kind = "mid-pass-over-a-hand-written-row";
    plant_mirrored_document(&cache, kind, 1, Some(ids::STEP_PLAN)).await;
    let hand_written = plant_mirrored_document(&cache, kind, 2, None).await;
    plant_mirrored_document(&cache, kind, 3, Some(not_yet_mirrored)).await;
    assert_eq!(
        read(kind).await,
        Some(hand_written),
        "the hand-written v2 is newer than the step-produced pick v1; the step-less v3 is not in \
         the hand-written arm, so it never out-versions v2"
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

/// MOD-147 D9: with a workspace scope, the mirror's upstream walk counts the workspace's own
/// scope row as in scope, as Postgres does: a member item `blocked_by` a `WS` item on
/// `@platform` sees it in scope, summary column and all, offline and online alike. The cache's
/// `scope` CTE binds the workspace once more for its new `UNION` arm.
#[tokio::test]
async fn the_mirror_counts_the_scope_row_in_upstream_scope() {
    use htui_core::model::{ItemId, NewItem};

    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let ws = db
        .store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_SCOPE_PLATFORM,
            kind_id: ids::KIND_SCOPE_PLATFORM_WS,
            title: "spans the workspace".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("a WS item mints on the scope row");
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO item_link (from_item_id, to_item_id, kind, created_at, updated_at) \
         VALUES ($1, $2, 'blocked_by', $3, $4)",
    )
    .bind(ids::AGY_FIX_1.as_uuid())
    .bind(ws.id.as_uuid())
    .bind(now)
    .bind(now)
    .execute(&db.pool)
    .await
    .expect("the link lands in Postgres");

    let mut projects = all_projects();
    projects.push(ids::PROJECT_SCOPE_PLATFORM);
    run_pass(&db.pool, &cache, &projects, &settings(&db, 20))
        .await
        .expect("a pass mirrors the scope row's item and the link");

    let scope = PromptScope {
        workspace: Some(ids::WORKSPACE_PLATFORM),
        project: ids::PROJECT_AGY,
    };
    let pg = db
        .store
        .upstream_summaries(ids::AGY_FIX_1, 2, &scope)
        .await
        .expect("pg upstream_summaries");
    let mirrored = cache
        .upstream_summaries(ids::AGY_FIX_1, 2, &scope)
        .await
        .expect("cache upstream_summaries");

    for (backend, entries) in [("pg", &pg), ("mirror", &mirrored)] {
        let entry = entries
            .iter()
            .find(|entry| entry.item_id == ws.id)
            .unwrap_or_else(|| panic!("{backend}: the WS item is upstream, got {entries:?}"));
        assert_eq!(
            (entry.qualified_key.as_str(), entry.depth, entry.in_scope),
            ("@platform:WS-1", 1, true),
            "{backend}: the scope row is in the workspace's scope"
        );
    }
    assert_eq!(mirrored, pg, "the mirror agrees with Postgres");

    let project_only = PromptScope {
        workspace: None,
        project: ids::PROJECT_AGY,
    };
    let narrow = cache
        .upstream_summaries(ids::AGY_FIX_1, 2, &project_only)
        .await
        .expect("cache upstream_summaries, project only");
    assert!(
        narrow
            .iter()
            .any(|entry| entry.item_id == ws.id && !entry.in_scope),
        "without a workspace the scope row is out of scope: {narrow:?}"
    );

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

    let inserted: Vec<SessionEvent> = (0..5_000).map(|seq| synthetic_event(step, seq)).collect();
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
        22,
        "MOD-38 adds requirement_spec, requirement_area, requirement and item_requirement; \
         MOD-139 adds run_plan",
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
        let event = synthetic_event(newest, seq);
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
        let event = synthetic_event(running, seq);
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
        let event = synthetic_event(running, seq);
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

/// MOD-7 blueprint §3.5, R-13: a copied config directory carries a mirror holding the old box's
/// row, and `CacheStore::box_info` reads `ORDER BY id LIMIT 1`. A pass prunes every row but this
/// box's, so the top bar never shows another machine.
#[tokio::test]
async fn a_pass_keeps_only_this_box_in_the_mirror() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;

    let stray = htui_core::model::BoxId::new();
    sqlx::query(
        "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version, \
                          registered_at, last_seen_at, updated_at) \
         VALUES (?, ?, 'another-machine', 'linux', '', 'x86_64', '0.0.0', 0, 0, 0)",
    )
    .bind(stray.as_uuid().to_string())
    .bind(db.store.this_user().as_uuid().to_string())
    .execute(cache.pool())
    .await
    .expect("plant a stray box row in the mirror");
    assert_eq!(
        mirror_count(cache.pool(), "box").await,
        1,
        "only the stray so far"
    );

    run_pass(&db.pool, &cache, &all_projects(), &settings(&db, 20))
        .await
        .expect("one pass");

    assert_eq!(
        mirror_count(cache.pool(), "box").await,
        1,
        "the pass leaves exactly one box row"
    );
    let id: String = sqlx::query_scalar("SELECT id FROM box")
        .fetch_one(cache.pool())
        .await
        .expect("read the mirrored box id");
    assert_eq!(
        uuid::Uuid::parse_str(&id).expect("a uuid"),
        ids::BOX.as_uuid(),
        "and it is this box's"
    );

    teardown(db, &[&cache]).await;
}

/// The registry read the offline arm answers from the mirror (D31).
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

/// MOD-163 plan D7: `memory_entry` and `memory_entry_revision` are not mirrored, so the cache
/// answers both memory reads `None` ("not available here"). Cache-only rather than a `READ_CASES`
/// entry, because `MemStore` and `PgStore` answer `Some`. No database: a plain temp directory.
#[tokio::test]
async fn memory_is_not_mirrored_so_the_cache_answers_none() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let cache = CacheStore::open(dir.path(), "memory-not-mirrored", 1)
        .await
        .expect("open a fresh mirror");
    assert_eq!(
        cache.memory_entries(ids::PROJECT_HTUI, None).await,
        Ok(None)
    );
    assert_eq!(cache.memory_revisions(MemoryEntryId::new()).await, Ok(None));
    cache.close().await;
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

/// MOD-147 (plan D1, D2): a scope row mirrors like any project once the refresher is given it —
/// its row with `owner_workspace_id`, its `WS` kind, an item minted on it and its owner documents —
/// and the mirror answers `scope_project`, `owner_documents` and `items` as Postgres does, while
/// `document(owner)` stays `None` and `workspaces()` still lists no scope row.
#[tokio::test]
async fn a_scope_row_mirrors_its_kinds_items_and_owner_documents() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let cache = open_cache(&db).await;
    let scope = ids::PROJECT_SCOPE_PLATFORM;

    let item = db
        .store
        .mint_item(htui_core::model::NewItem {
            id: htui_core::model::ItemId::new(),
            project_id: scope,
            kind_id: ids::KIND_SCOPE_PLATFORM_WS,
            title: "Spans the workspace".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("mint WS-1 on the scope row");
    assert_eq!(item.key, "WS-1");
    let owner = db
        .store
        .write_owner_document(htui_core::model::NewOwnerDocument {
            id: DocumentId::new(),
            project_id: scope,
            kind: "goals".to_owned(),
            title: "Goals".to_owned(),
            body: "Ship it.".to_owned(),
            created_by: ids::USER,
            created_at: Utc::now(),
        })
        .await
        .expect("write an owner document on the scope row");

    let mut projects = all_projects();
    projects.push(scope);
    run_pass(&db.pool, &cache, &projects, &settings(&db, 20))
        .await
        .expect("one pass over the demo projects and the scope row");

    assert_eq!(
        cache
            .scope_project(ids::WORKSPACE_PLATFORM)
            .await
            .expect("cache scope_project"),
        Some(scope),
        "the mirror names Platform's scope row"
    );
    let mirrored = cache
        .project(scope)
        .await
        .expect("cache project")
        .expect("the scope row is mirrored");
    assert_eq!(mirrored.owner_workspace_id, Some(ids::WORKSPACE_PLATFORM));
    assert_eq!(
        Some(mirrored),
        db.store.project(scope).await.expect("pg project"),
        "the two backends project the scope row alike"
    );
    let mirrored_docs = cache
        .owner_documents(scope)
        .await
        .expect("cache owner_documents");
    assert_eq!(
        mirrored_docs,
        db.store
            .owner_documents(scope)
            .await
            .expect("pg owner_documents"),
        "owner documents come off the mirror as Postgres has them"
    );
    assert_eq!(
        mirrored_docs.iter().map(|doc| doc.id).collect::<Vec<_>>(),
        [owner.id]
    );
    assert_eq!(
        cache.document(owner.id).await.expect("cache document"),
        None,
        "an owner document is never an item document"
    );
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT prefix FROM item_kind WHERE project_id = ?")
            .bind(scope.as_uuid().to_string())
            .fetch_all(cache.pool())
            .await
            .expect("read the mirrored kinds");
    assert_eq!(kinds, ["WS"], "the scope row's one kind is mirrored");
    let scope_items = Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![scope],
    };
    let items = cache
        .items(&scope_items, &ItemFilter::default())
        .await
        .expect("cache items");
    assert_eq!(
        items.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(),
        ["WS-1"],
        "the scope row's item is mirrored"
    );
    let platform = cache
        .workspaces()
        .await
        .expect("cache workspaces")
        .into_iter()
        .find(|ws| ws.workspace_id == ids::WORKSPACE_PLATFORM)
        .expect("Platform is mirrored");
    assert!(
        platform
            .projects
            .iter()
            .all(|project| project.project_id != scope),
        "the mirror's workspaces() lists no scope row: {:?}",
        platform.projects
    );
    let asked = Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY, scope],
    };
    let listed = cache.projects(&asked).await.expect("cache projects");
    assert_eq!(
        listed
            .iter()
            .map(|project| project.project_id)
            .collect::<Vec<_>>(),
        [ids::PROJECT_HTUI, ids::PROJECT_AGY],
        "the mirror's projects() over a scope naming the scope row lists members only"
    );
    assert_eq!(
        listed,
        db.store.projects(&asked).await.expect("pg projects"),
        "the two backends list a scope's projects alike"
    );

    teardown(db, &[&cache]).await;
}
