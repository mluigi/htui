//! MOD-92 T3: `htui status` and `htui waiting` end to end over **Postgres**, through the seam
//! `state_cmd::answer` + `state_cmd::emit` (the keyring is never read: its fake is process-wide).
//!
//! What only this file can prove: the verbs answer over the read-only session (`PgStore::
//! connect_read_only`), write nothing (no `box` row, no seed, no `updated_at` moved), scope to the
//! first workspace, `--project` and `--cwd` as the plan says, compute the waiting rows with the
//! TUI's own classifier, and answer an unreachable server or a pending schema with a typed document
//! and exit 3 or 2. Every case runs over `testkit::demo_db()` (fixed ids and stamps) but the two
//! that need no database; each prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL`
//! unset, and panics instead when `CI` is set, like every other Postgres-backed suite.
//!
//! `--search-items` runs through its own seam, `concepts::search_answer_with` (review R1 M4), with
//! `HashEmbedder` behind a counting loader, so no model is downloaded and the cases can say when
//! the model is loaded at all. The cases that need an index are gated on `HTUI_TEST_QDRANT_URL`
//! too and own a throwaway collection.
#![cfg(feature = "testkit")]

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use htui::cli::StateArgs;
use htui::concepts::{self, SearchContext, SearchOptions};
use htui::state_cmd::{self, Answer, Context, Kind, Outcome, StateExit, Verb};
use htui_core::fixtures::ids;
use htui_core::model::{BoxId, ItemFilter, NewRepo, RepoBoxPath, RepoId, Scope};
use htui_core::store::{ReadStore as _, WriteStore as _};
use htui_store::embed::{DENSE_DIM, HashEmbedder};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::QdrantStore;
use htui_store::{Identity, testkit};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(5);
const UNREACHABLE: &str = "postgres://nobody:nothing@127.0.0.1:1/none";

const UUID: (&str, &str) = (
    r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
    "[uuid]",
);
const STAMP: (&str, &str) = (r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "[timestamp]");
const AGE: (&str, &str) = (r#""age_seconds": \d+"#, r#""age_seconds": "[age]""#);
const MESSAGE: (&str, &str) = (r#""message": "[^"]*""#, r#""message": "[message]""#);

/// `testkit::fixture_box_store`'s identity: the fixture's registered box.
fn fixture_identity() -> Identity {
    Identity {
        box_id: ids::BOX,
        hostname: "DESKTOP-HTUI".into(),
    }
}

/// A box no `box` row names: `connect_headless` would register it.
fn unregistered_identity() -> Identity {
    Identity {
        box_id: BoxId::new(),
        hostname: "HTUI-TEST".into(),
    }
}

/// A fixed instant, so ages are deterministic where nothing re-stamps.
fn now() -> DateTime<Utc> {
    htui_core::fixtures::demo_at(30, 0)
}

/// What one verb printed and how it ended.
struct Ran {
    answer: Answer,
    exit: Result<(), StateExit>,
    document: Value,
}

impl Ran {
    fn code(&self) -> u8 {
        self.answer.exit_code()
    }

    fn pretty(&self) -> String {
        serde_json::to_string_pretty(&self.document).expect("a document prints")
    }

    fn error_code(&self) -> &str {
        self.document["error"]["code"].as_str().unwrap_or_default()
    }
}

/// `answer`, then `emit --json` into a buffer: exactly what `state_cmd::run` writes to stdout.
async fn ran(
    verb: Verb,
    args: StateArgs,
    dsn: &str,
    identity: Option<&Identity>,
    timeout: Duration,
) -> Ran {
    let ctx = Context {
        dsn,
        identity,
        now: now(),
        connect_timeout: timeout,
    };
    let answer = state_cmd::answer(verb, &args, &ctx).await;
    let mut out = Vec::new();
    let exit = state_cmd::emit(&mut out, &answer, true);
    let text = String::from_utf8(out).expect("UTF-8");
    assert_eq!(text.matches('\n').count(), 1, "one line: {text}");
    let document = serde_json::from_str(&text).expect("one JSON document");
    assert_eq!(document, state_cmd::document(&answer));
    Ran {
        answer,
        exit,
        document,
    }
}

fn json() -> StateArgs {
    StateArgs {
        json: true,
        ..StateArgs::default()
    }
}

fn project(slug: &str) -> StateArgs {
    StateArgs {
        project: Some(slug.into()),
        ..json()
    }
}

fn cwd(path: PathBuf) -> StateArgs {
    StateArgs {
        cwd: Some(path),
        ..json()
    }
}

/// Parks demo `RUN_2` at its `prd` gate, as `PgStore::park_step` leaves a run (the trigger stamps
/// both rows' `updated_at`).
async fn seed_gate(db: &testkit::TestDb) {
    sqlx::query("UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1")
        .bind(ids::STEP_R2_PRD.as_uuid())
        .execute(&db.pool)
        .await
        .expect("park the step");
    sqlx::query("UPDATE run SET status = 'awaiting_approval' WHERE id = $1")
        .bind(ids::RUN_2.as_uuid())
        .execute(&db.pool)
        .await
        .expect("park the run");
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[tokio::test]
async fn status_defaults_to_the_first_workspace() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let identity = fixture_identity();
    let run = ran(Verb::Status, json(), &db.url, Some(&identity), WAIT).await;
    assert_eq!(run.code(), 0, "{}", run.pretty());
    assert!(run.exit.is_ok());
    // `workspaces()` orders by name: Graphics before Platform.
    assert_eq!(run.document["workspace"]["name"], "Graphics");
    assert_eq!(run.document["box"]["registered"], true);
    assert_eq!(run.document["box"]["hostname"], "DESKTOP-HTUI");
    insta::with_settings!({ filters => vec![STAMP, AGE] }, {
        insta::assert_snapshot!("status_default_workspace", run.pretty());
    });
    db.drop_db().await;
}

#[tokio::test]
async fn waiting_matches_the_tuis_classifier() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    seed_gate(&db).await;
    let identity = fixture_identity();
    let run = ran(
        Verb::Waiting,
        project("htui"),
        &db.url,
        Some(&identity),
        WAIT,
    )
    .await;
    assert_eq!(run.code(), 0, "{}", run.pretty());

    // Parity with `serve_waiting`: the same three reads on the read-write store, the same
    // classifier, the same scope.
    let workspaces = db.store.workspaces().await.expect("workspaces");
    let ws = workspaces
        .iter()
        .find(|ws| {
            ws.projects
                .iter()
                .any(|p| p.project_id == ids::PROJECT_HTUI)
        })
        .expect("a workspace lists htui");
    let scope = Scope {
        workspace_id: ws.workspace_id,
        project_ids: vec![ids::PROJECT_HTUI],
    };
    let active = db.store.active_runs(&scope).await.expect("active");
    let candidates = db
        .store
        .waiting_candidates(&scope)
        .await
        .expect("candidates");
    let permissions = db
        .store
        .open_permissions(&scope)
        .await
        .expect("permissions");
    let view = htui_worker::waiting(&scope, active, &candidates, Some(&permissions));
    let Outcome::Waiting(body) = &run.answer.outcome else {
        panic!("a waiting answer: {:?}", run.answer);
    };
    assert_eq!(body.working, view.working);
    let ours: Vec<_> = body
        .rows
        .iter()
        .map(|row| {
            (
                row.reason,
                row.item_id,
                row.run_id,
                row.step_id,
                row.since.clone(),
            )
        })
        .collect();
    let tuis: Vec<_> = view
        .rows
        .iter()
        .map(|row| {
            (
                state_cmd::ReasonDoc::from(row.reason),
                row.item,
                row.run,
                row.step,
                rfc3339(row.since),
            )
        })
        .collect();
    assert_eq!(ours, tuis);

    let gate = body
        .rows
        .iter()
        .find(|row| row.step_id == Some(ids::STEP_R2_PRD))
        .expect("the parked gate is a row");
    assert_eq!(gate.reason, state_cmd::ReasonDoc::Gate);
    let (stamp,): (DateTime<Utc>,) =
        sqlx::query_as("SELECT updated_at FROM run_step WHERE id = $1")
            .bind(ids::STEP_R2_PRD.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("the step's stamp");
    assert_eq!(gate.since, rfc3339(stamp));
    insta::with_settings!({ filters => vec![STAMP, AGE] }, {
        insta::assert_snapshot!("waiting_project_htui", run.pretty());
    });
    db.drop_db().await;
}

#[tokio::test]
async fn cwd_scopes_to_the_registered_checkout() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let checkout = tempfile::tempdir().expect("a checkout");
    let sibling = tempfile::tempdir().expect("a sibling");
    let inside = checkout.path().join("a").join("b");
    std::fs::create_dir_all(&inside).expect("mkdir");
    let repo = db
        .store
        .create_repo(NewRepo {
            id: RepoId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "mod-92-cwd".into(),
            remote_url: None,
            default_branch: "main".into(),
            is_primary: false,
        })
        .await
        .expect("a repo");
    let canonical = std::fs::canonicalize(checkout.path()).expect("canonical");
    db.store
        .upsert_repo_box_path(&RepoBoxPath {
            repo_id: repo.id,
            box_id: ids::BOX,
            local_path: canonical.display().to_string(),
            updated_at: Utc::now(),
        })
        .await
        .expect("this box's checkout");
    let identity = fixture_identity();

    let run = ran(
        Verb::Status,
        cwd(inside.clone()),
        &db.url,
        Some(&identity),
        WAIT,
    )
    .await;
    assert_eq!(run.code(), 0, "{}", run.pretty());
    let slugs: Vec<_> = run.document["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .map(|p| p["slug"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(slugs, ["htui"]);

    let elsewhere = ran(
        Verb::Waiting,
        cwd(sibling.path().to_path_buf()),
        &db.url,
        Some(&identity),
        WAIT,
    )
    .await;
    assert_eq!(elsewhere.code(), 2, "{}", elsewhere.pretty());
    assert_eq!(elsewhere.document["store"], "refused");
    assert_eq!(elsewhere.error_code(), "cwd_unmatched");

    let no_box = ran(Verb::Status, cwd(inside), &db.url, None, WAIT).await;
    assert_eq!(no_box.code(), 2, "{}", no_box.pretty());
    assert_eq!(no_box.error_code(), "no_box");

    let missing = ran(
        Verb::Status,
        cwd(checkout.path().join("not-there")),
        &db.url,
        Some(&identity),
        WAIT,
    )
    .await;
    assert_eq!(missing.code(), 2, "{}", missing.pretty());
    assert_eq!(missing.error_code(), "cwd_unresolved");
    db.drop_db().await;
}

#[tokio::test]
async fn an_unknown_project_is_refused() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let identity = fixture_identity();
    let run = ran(
        Verb::Waiting,
        project("nope"),
        &db.url,
        Some(&identity),
        WAIT,
    )
    .await;
    assert_eq!(run.code(), 2);
    assert_eq!(run.document["store"], "refused");
    assert_eq!(run.error_code(), "unknown_project");
    assert!(
        matches!(run.exit, Err(StateExit::Refused(_))),
        "{:?}",
        run.exit
    );
    insta::assert_snapshot!("unknown_project", run.pretty());
    db.drop_db().await;
}

/// Needs no database: never skips.
#[tokio::test]
async fn an_unreachable_store_is_offline() {
    let timeout = Duration::from_secs(2);
    for verb in [Verb::Status, Verb::Waiting] {
        let run = ran(verb, json(), UNREACHABLE, None, timeout).await;
        assert_eq!(run.code(), 3, "{}", run.pretty());
        assert_eq!(run.document["store"], "offline");
        assert_eq!(run.error_code(), "unreachable");
        assert!(
            matches!(run.exit, Err(StateExit::Offline(_))),
            "{:?}",
            run.exit
        );
        if verb == Verb::Status {
            insta::with_settings!({ filters => vec![MESSAGE] }, {
                insta::assert_snapshot!("offline", run.pretty());
            });
        }
    }
}

#[tokio::test]
async fn a_pending_schema_is_refused() {
    let Some(db) = testkit::bare_db().await else {
        return;
    };
    let run = ran(Verb::Status, json(), &db.url, None, WAIT).await;
    assert_eq!(run.code(), 2, "{}", run.pretty());
    assert_eq!(run.document["store"], "refused");
    assert_eq!(run.error_code(), "migrations_pending");
    db.drop_db().await;
}

/// Row counts and the newest stamps of every table a write would touch.
async fn footprint(db: &testkit::TestDb) -> Vec<(String, i64, Option<String>)> {
    let mut rows = Vec::new();
    for (table, stamp) in [
        ("box", "GREATEST(MAX(updated_at), MAX(last_seen_at))"),
        ("app_user", "MAX(updated_at)"),
        ("item", "MAX(updated_at)"),
        ("run", "MAX(updated_at)"),
        ("run_step", "MAX(updated_at)"),
        ("app_setting", "MAX(updated_at)"),
    ] {
        let sql = format!("SELECT COUNT(*)::bigint, ({stamp})::text FROM {table}");
        let (count, newest): (i64, Option<String>) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .fetch_one(&db.pool)
            .await
            .expect("a footprint read");
        rows.push((table.to_owned(), count, newest));
    }
    rows
}

#[tokio::test]
async fn the_verbs_write_nothing() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let before = footprint(&db).await;
    let identity = unregistered_identity();
    let status = ran(Verb::Status, json(), &db.url, Some(&identity), WAIT).await;
    assert_eq!(status.code(), 0, "{}", status.pretty());
    let waiting = ran(Verb::Waiting, json(), &db.url, Some(&identity), WAIT).await;
    assert_eq!(waiting.code(), 0, "{}", waiting.pretty());
    assert_eq!(footprint(&db).await, before);
    db.drop_db().await;
}

#[tokio::test]
async fn the_read_only_session_is_what_answers() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let identity = unregistered_identity();
    let run = ran(Verb::Status, json(), &db.url, Some(&identity), WAIT).await;
    assert_eq!(run.code(), 0, "{}", run.pretty());
    // `connect_headless` would have registered this box; the read-only session reports it as
    // unregistered, under this machine's hostname, and leaves no row.
    assert_eq!(run.document["box"]["registered"], false);
    assert_eq!(run.document["box"]["hostname"], "HTUI-TEST");
    let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM box WHERE id = $1")
        .bind(identity.box_id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("a count");
    assert_eq!(rows, 0);
    insta::with_settings!({ filters => vec![UUID] }, {
        insta::assert_snapshot!("status_unregistered_box", run.pretty());
    });
    db.drop_db().await;
}

// --- `--search-items` (review R1 M4) ---

/// Nothing listens on port 1: a Qdrant that does not answer.
const QDRANT_DOWN: &str = "http://127.0.0.1:1";

fn search_options(query: &str) -> SearchOptions {
    SearchOptions {
        query: query.into(),
        project: None,
        decisions: false,
        limit: concepts::DEFAULT_LIMIT,
        cwd: None,
        json: true,
    }
}

/// A model loader that counts its calls and hands out `HashEmbedder`.
#[derive(Clone, Default)]
struct Loads(Arc<AtomicUsize>);

impl Loads {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }

    fn loader(&self) -> impl FnOnce() -> std::future::Ready<anyhow::Result<HashEmbedder>> + use<> {
        let loads = Arc::clone(&self.0);
        move || {
            loads.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(HashEmbedder::new(DENSE_DIM)))
        }
    }
}

/// `search_answer_with`, then `emit --json` into a buffer: what `search_items` writes to stdout.
async fn searched(
    settings: &QdrantSettings,
    collection: &str,
    dsn: &str,
    identity: Option<&Identity>,
    options: &SearchOptions,
    loads: &Loads,
) -> Ran {
    let ctx = SearchContext {
        settings,
        collection,
        dsn,
        identity,
        connect_timeout: WAIT,
    };
    let answer = concepts::search_answer_with(&ctx, options, loads.loader()).await;
    let mut out = Vec::new();
    let exit = state_cmd::emit(&mut out, &answer, true);
    let text = String::from_utf8(out).expect("UTF-8");
    assert_eq!(text.matches('\n').count(), 1, "one line: {text}");
    let document = serde_json::from_str(&text).expect("one JSON document");
    assert_eq!(document, state_cmd::document(&answer));
    Ran {
        answer,
        exit,
        document,
    }
}

fn qdrant(url: &str) -> QdrantSettings {
    QdrantSettings::new(url.to_owned(), None).expect("a valid URL")
}

/// The test Qdrant's URL, or the skip line.
fn test_qdrant() -> Option<String> {
    let url = std::env::var("HTUI_TEST_QDRANT_URL").ok();
    if url.is_none() {
        eprintln!("skipped: HTUI_TEST_QDRANT_URL not set");
    }
    url
}

fn fresh_collection() -> String {
    format!("htui_test_{}", uuid::Uuid::now_v7().simple())
}

/// Needs no database: never skips. A box with no Qdrant URL is refused before Postgres is asked.
#[tokio::test]
async fn a_search_without_a_qdrant_url_is_refused() {
    let failure =
        concepts::qdrant_settings(None, Some("a-key".into())).expect_err("no URL, no settings");
    let answer = Answer {
        kind: Kind::Search,
        outcome: Outcome::Failed(failure),
    };
    assert_eq!(answer.exit_code(), 2);
    let document = state_cmd::document(&answer);
    assert_eq!(document["store"], "refused");
    assert_eq!(document["error"]["code"], "no_qdrant_url");
    let unusable = concepts::qdrant_settings(Some("localhost:6334".into()), None)
        .expect_err("a URL without a scheme");
    assert_eq!(unusable.error.code, state_cmd::ErrorCode::QdrantUrlUnusable);
}

/// Qdrant down: Postgres answered (`store` online), the index is `offline`/3, and the model was
/// never loaded (review R1 M3).
#[tokio::test]
async fn a_search_with_qdrant_down_is_offline_and_loads_no_model() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let loads = Loads::default();
    let run = searched(
        &qdrant(QDRANT_DOWN),
        "htui_concepts_v2",
        &db.url,
        None,
        &search_options("closed items"),
        &loads,
    )
    .await;
    assert_eq!(run.code(), 3, "{}", run.pretty());
    assert_eq!(run.document["store"], "online");
    assert_eq!(run.document["index"], "offline");
    assert_eq!(run.error_code(), "qdrant_unreachable");
    assert!(
        matches!(run.exit, Err(StateExit::Offline(_))),
        "{:?}",
        run.exit
    );
    assert_eq!(
        loads.count(),
        0,
        "no model for a Qdrant that does not answer"
    );
    db.drop_db().await;
}

/// The scope refusals come before Qdrant and the model: an unknown `--project`, and a `--cwd`
/// in no registered checkout.
#[tokio::test]
async fn a_search_scope_refusal_comes_before_the_index() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let loads = Loads::default();
    let unknown = SearchOptions {
        project: Some("nope".into()),
        ..search_options("anything")
    };
    let run = searched(&qdrant(QDRANT_DOWN), "c", &db.url, None, &unknown, &loads).await;
    assert_eq!(run.code(), 2, "{}", run.pretty());
    assert_eq!(run.document["store"], "refused");
    assert_eq!(run.error_code(), "unknown_project");

    let elsewhere = tempfile::tempdir().expect("a directory");
    let unmatched = SearchOptions {
        cwd: Some(elsewhere.path().to_path_buf()),
        ..search_options("anything")
    };
    let identity = fixture_identity();
    let run = searched(
        &qdrant(QDRANT_DOWN),
        "c",
        &db.url,
        Some(&identity),
        &unmatched,
        &loads,
    )
    .await;
    assert_eq!(run.code(), 2, "{}", run.pretty());
    assert_eq!(run.error_code(), "cwd_unmatched");
    assert_eq!(loads.count(), 0);
    db.drop_db().await;
}

/// No collection: `empty`/0 with `hits: []`, nothing created, and no model loaded (review R1 M3).
#[tokio::test]
async fn a_search_without_a_collection_is_empty_and_loads_no_model() {
    let Some(url) = test_qdrant() else {
        return;
    };
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let loads = Loads::default();
    let name = fresh_collection();
    let run = searched(
        &qdrant(&url),
        &name,
        &db.url,
        None,
        &search_options("closed items"),
        &loads,
    )
    .await;
    assert_eq!(run.code(), 0, "{}", run.pretty());
    assert_eq!(run.document["index"], "empty");
    assert_eq!(run.document["hits"], serde_json::json!([]));
    assert_eq!(loads.count(), 0, "no model for a missing index");
    let exists = QdrantStore::open_existing_to(&qdrant(&url), HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers")
        .is_some();
    assert!(!exists, "the search created no collection");
    db.drop_db().await;
}

/// Over an index of the demo: the model loads once, `--project` and `--cwd` narrow the search to
/// one project, and a model that fails, at load or on the query, is `model_unavailable`/1.
#[tokio::test]
async fn a_search_over_an_index_answers_in_scope() {
    let Some(url) = test_qdrant() else {
        return;
    };
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let name = fresh_collection();
    let index = QdrantStore::connect_to(&qdrant(&url), HashEmbedder::new(DENSE_DIM), &name)
        .await
        .expect("Qdrant answers");
    concepts::index_cycle(&db.store, &index).await;
    let workspaces = db.store.workspaces().await.expect("workspaces");
    let keys_of = |project| {
        let workspace_id = workspaces
            .iter()
            .find(|ws| ws.projects.iter().any(|p| p.project_id == project))
            .expect("a workspace lists the project")
            .workspace_id;
        let store = &db.store;
        async move {
            store
                .items(
                    &Scope {
                        workspace_id,
                        project_ids: vec![project],
                    },
                    &ItemFilter::default(),
                )
                .await
                .expect("items")
                .into_iter()
                .map(|item| item.key)
                .collect::<Vec<_>>()
        }
    };
    // Keys are per project: one only htui holds, so agy's scope cannot find it.
    let agy_keys = keys_of(ids::PROJECT_AGY).await;
    let key = keys_of(ids::PROJECT_HTUI)
        .await
        .into_iter()
        .find(|key| !agy_keys.contains(key))
        .expect("a key only htui holds");
    let keys = |run: &Ran| -> Vec<String> {
        run.document["hits"]
            .as_array()
            .map(|hits| {
                hits.iter()
                    .map(|hit| hit["key"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let settings = qdrant(&url);
    let loads = Loads::default();

    let everywhere = searched(
        &settings,
        &name,
        &db.url,
        None,
        &search_options(&key),
        &loads,
    )
    .await;
    assert_eq!(everywhere.code(), 0, "{}", everywhere.pretty());
    assert_eq!(everywhere.document["index"], "online");
    assert!(keys(&everywhere).contains(&key), "{}", everywhere.pretty());
    assert_eq!(loads.count(), 1, "one load per search");

    let htui = SearchOptions {
        project: Some("htui".into()),
        ..search_options(&key)
    };
    let run = searched(&settings, &name, &db.url, None, &htui, &loads).await;
    assert!(keys(&run).contains(&key), "{}", run.pretty());
    let agy = SearchOptions {
        project: Some("agy".into()),
        ..search_options(&key)
    };
    let run = searched(&settings, &name, &db.url, None, &agy, &loads).await;
    assert_eq!(run.code(), 0, "{}", run.pretty());
    assert!(!keys(&run).contains(&key), "agy's scope: {}", run.pretty());

    // `--cwd` inside htui's checkout on this box is `--project htui`.
    let checkout = tempfile::tempdir().expect("a checkout");
    let repo = db
        .store
        .create_repo(NewRepo {
            id: RepoId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "r1-search-cwd".into(),
            remote_url: None,
            default_branch: "main".into(),
            is_primary: false,
        })
        .await
        .expect("a repo");
    db.store
        .upsert_repo_box_path(&RepoBoxPath {
            repo_id: repo.id,
            box_id: ids::BOX,
            local_path: std::fs::canonicalize(checkout.path())
                .expect("canonical")
                .display()
                .to_string(),
            updated_at: Utc::now(),
        })
        .await
        .expect("this box's checkout");
    let identity = fixture_identity();
    let in_checkout = SearchOptions {
        cwd: Some(checkout.path().to_path_buf()),
        ..search_options(&key)
    };
    let run = searched(
        &settings,
        &name,
        &db.url,
        Some(&identity),
        &in_checkout,
        &loads,
    )
    .await;
    assert_eq!(run.code(), 0, "{}", run.pretty());
    assert!(keys(&run).contains(&key), "{}", run.pretty());

    // A model that cannot load, and one that fails on the query: the model's failure, not Qdrant's.
    let ctx = SearchContext {
        settings: &settings,
        collection: &name,
        dsn: &db.url,
        identity: None,
        connect_timeout: WAIT,
    };
    let no_model = concepts::search_answer_with(&ctx, &search_options(&key), || {
        std::future::ready(Err::<HashEmbedder, _>(anyhow::anyhow!("no model here")))
    })
    .await;
    let broken = concepts::search_answer_with(&ctx, &search_options(&key), || {
        std::future::ready(Ok(Broken))
    })
    .await;
    index.drop_collection().await.expect("drop");
    for answer in [no_model, broken] {
        assert_eq!(answer.exit_code(), 1, "{answer:?}");
        let document = state_cmd::document(&answer);
        assert_eq!(document["store"], "online");
        assert_eq!(document["index"], "error");
        assert_eq!(document["error"]["code"], "model_unavailable", "{document}");
    }
    db.drop_db().await;
}

/// `HashEmbedder`'s identity and width, and an error for every query.
#[derive(Debug, Clone)]
struct Broken;

impl htui_store::embed::DenseEmbedder for Broken {
    fn dim(&self) -> usize {
        DENSE_DIM
    }

    fn identity(&self) -> htui_store::embed::EmbedderIdentity {
        htui_store::embed::EmbedderIdentity::hash(DENSE_DIM)
    }

    async fn embed(&self, _: Vec<String>) -> Result<Vec<Vec<f32>>, htui_core::store::StoreError> {
        Err(htui_core::store::StoreError::Backend(
            "the model's session failed".into(),
        ))
    }
}
