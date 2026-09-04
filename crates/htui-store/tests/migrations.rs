//! ANA-9 §11 criterion 1: the schema applies on a clean Postgres, a second run is a no-op, and a
//! database whose schema this binary does not know is refused rather than repaired (`R-STO-5`).
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes, so the suite is green on a box without a server (plan D13).
//! `box_toml_mint_is_stable_across_two_reads` needs no server and therefore never skips.

mod common;

use std::collections::BTreeSet;

use htui_core::model::{BoxId, OsFamily};
use htui_core::store::StoreError;
use htui_store::{MIGRATOR, MigrationState, PgStore, identity};
use sqlx::Row as _;

/// The 32 tables of blueprint B.1, in creation order.
const TABLES: &[&str] = &[
    "app_user",
    "capability_tag",
    "box",
    "box_tool",
    "agent",
    "agent_box",
    "workspace",
    "project",
    "workspace_project",
    "workspace_box_path",
    "repo",
    "repo_box_path",
    "step_graph",
    "step_graph_phase",
    "phase_agent",
    "prompt_template",
    "item_kind",
    "item_key_counter",
    "item",
    "item_revision",
    "item_link",
    "item_note",
    "document",
    "skill",
    "skill_version",
    "skill_binding",
    "run",
    "run_step",
    "run_step_commit",
    "session_event",
    "command_run",
    "app_setting",
];

#[tokio::test]
async fn migrations_apply_on_a_clean_database() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY 1")
        .fetch_all(&db.pool)
        .await
        .expect("read _sqlx_migrations");
    let embedded: Vec<i64> = MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| m.version)
        .collect();
    assert_eq!(applied, embedded, "every embedded migration is applied");

    let present: BTreeSet<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read information_schema.tables")
    .into_iter()
    .collect();

    for table in TABLES {
        assert!(present.contains(*table), "table `{table}` was created");
    }
    assert_eq!(
        TABLES.len(),
        32,
        "blueprint B.1 lists 32 tables; ANA-9 §3's prose count of 30 is wrong (H.1)"
    );
    // `_sqlx_migrations` is the only extra table sqlx adds.
    assert_eq!(
        present.len(),
        TABLES.len() + 1,
        "the migration creates the 32 tables of B.1 and nothing else, got {present:?}"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn a_second_run_is_a_no_op() {
    let Some(mut db) = common::fresh_db().await else {
        return;
    };

    let before: i64 = common::count(&db.pool, "_sqlx_migrations").await;
    db.store
        .apply_migrations()
        .await
        .expect("a second apply_migrations succeeds");
    let after: i64 = common::count(&db.pool, "_sqlx_migrations").await;

    assert_eq!(before, after, "re-running the migrator applies nothing new");
    db.drop_db().await;
}

#[tokio::test]
async fn connect_reports_up_to_date_after_apply() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let connected = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("connect to the migrated database");
    assert_eq!(connected.migrations, MigrationState::UpToDate);

    db.drop_db().await;
}

#[tokio::test]
async fn connect_reports_pending_on_a_bare_database() {
    let Some(db) = common::bare_db().await else {
        return;
    };

    assert_eq!(
        db.migrations_at_connect,
        MigrationState::Pending(1),
        "one embedded migration, none applied"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn a_newer_applied_version_is_refused() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (9999, 'from a newer htui', true, '\\x00'::bytea, 0)",
    )
    .execute(&db.pool)
    .await
    .expect("plant a newer applied migration");

    let err = PgStore::connect(&db.url, &db.identity)
        .await
        .expect_err("a newer schema is refused, not adopted");
    match &err {
        StoreError::Backend(text) => assert!(
            text.contains("schema is newer"),
            "the refusal names the reason, got {text}"
        ),
        other => panic!("expected StoreError::Backend, got {other:?}"),
    }

    db.drop_db().await;
}

#[tokio::test]
async fn a_checksum_mismatch_is_refused() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\xdeadbeef'::bytea WHERE version = 1")
        .execute(&db.pool)
        .await
        .expect("corrupt the recorded checksum");

    let err = PgStore::connect(&db.url, &db.identity)
        .await
        .expect_err("a modified migration is refused");
    match &err {
        StoreError::Backend(text) => assert!(
            text.contains("checksum"),
            "the refusal names the reason, got {text}"
        ),
        other => panic!("expected StoreError::Backend, got {other:?}"),
    }

    db.drop_db().await;
}

#[tokio::test]
async fn the_trigger_bumps_updated_at_on_update_only() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    // The smallest FK chain that reaches `item`: project -> step_graph -> item_kind -> item.
    let user = db.store.this_user().as_uuid();
    let project = uuid::Uuid::now_v7();
    let graph = uuid::Uuid::now_v7();
    let kind = uuid::Uuid::now_v7();
    let item = uuid::Uuid::now_v7();
    let stamp = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
        .expect("a literal RFC-3339 timestamp")
        .with_timezone(&chrono::Utc);

    sqlx::query("INSERT INTO project (id, slug, name, created_by) VALUES ($1, 'trg', 'Trg', $2)")
        .bind(project)
        .bind(user)
        .execute(&db.pool)
        .await
        .expect("insert project");
    sqlx::query("INSERT INTO step_graph (id, project_id, name) VALUES ($1, $2, 'default')")
        .bind(graph)
        .bind(project)
        .execute(&db.pool)
        .await
        .expect("insert step_graph");
    sqlx::query(
        "INSERT INTO item_kind (id, project_id, prefix, name, default_graph_id) \
         VALUES ($1, $2, 'TRG', 'trigger', $3)",
    )
    .bind(kind)
    .bind(project)
    .bind(graph)
    .execute(&db.pool)
    .await
    .expect("insert item_kind");
    sqlx::query(
        "INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, created_by, \
         created_at, updated_at) VALUES ($1, $2, $3, 'TRG', 1, 'before', $4, $5, $5)",
    )
    .bind(item)
    .bind(project)
    .bind(kind)
    .bind(user)
    .bind(stamp)
    .execute(&db.pool)
    .await
    .expect("insert item");

    let inserted: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM item WHERE id = $1")
            .bind(item)
            .fetch_one(&db.pool)
            .await
            .expect("read updated_at");
    assert_eq!(
        inserted, stamp,
        "the BEFORE UPDATE trigger leaves an explicit INSERT timestamp alone"
    );

    sqlx::query("UPDATE item SET title = 'after' WHERE id = $1")
        .bind(item)
        .execute(&db.pool)
        .await
        .expect("update item");
    let updated: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM item WHERE id = $1")
            .bind(item)
            .fetch_one(&db.pool)
            .await
            .expect("read updated_at");
    assert!(
        updated > inserted,
        "every UPDATE gets clock_timestamp(), which is what the cache cursor rides on"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn seed_is_idempotent() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    // `apply_migrations` already seeded once; this is the second and third pass.
    let first = db.store.seed_if_empty().await.expect("seed once more");
    let second = db.store.seed_if_empty().await.expect("and again");
    assert_eq!(first, second, "seeding twice yields the same app_user");
    assert_eq!(
        first,
        db.store.this_user(),
        "the store points at the seeded user"
    );

    assert_eq!(common::count(&db.pool, "app_user").await, 1, "one app_user");
    assert_eq!(
        common::count(&db.pool, "capability_tag").await,
        10,
        "the ten seeded capability tags of §5.10"
    );
    assert_eq!(
        common::count(&db.pool, "app_setting").await,
        2,
        "cache_refresh_seconds and cache_overlap_seconds"
    );
    assert_eq!(
        common::count(&db.pool, "agent").await,
        0,
        "no agent rows: agent.launch is ANA-4's shape (plan D5, blueprint H.2)"
    );

    let seeded: bool = sqlx::query_scalar("SELECT bool_and(seeded) FROM capability_tag")
        .fetch_one(&db.pool)
        .await
        .expect("read capability_tag.seeded");
    assert!(seeded, "every seeded tag is marked as such");

    db.drop_db().await;
}

/// `R-USR-2`: one `app_user`, however many boxes and whatever their OS user is called.
///
/// The seed name is injected rather than pushed through `USERNAME` / `USER` so the case does not
/// mutate the process environment other tests read.
#[tokio::test]
async fn a_second_box_under_another_os_user_name_seeds_no_second_app_user() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let seeded = db.store.this_user();

    // Another box: another `box.toml` identity, another hostname, another OS user name.
    let elsewhere = identity::Identity {
        box_id: BoxId::new(),
        hostname: format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple()),
    };
    let second = PgStore::connect(&db.url, &elsewhere)
        .await
        .expect("a second box connects to the same database")
        .store;
    let adopted = second
        .seed_if_empty_as("another-os-user")
        .await
        .expect("seed from the second box");

    assert_eq!(
        common::count(&db.pool, "app_user").await,
        1,
        "a differently named OS user must not seed a second app_user (R-USR-2)"
    );
    assert_eq!(
        adopted, seeded,
        "the second box adopts the row the first one seeded"
    );
    assert_eq!(
        second.this_user(),
        seeded,
        "and both stores answer the same app_user"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn register_box_upserts_and_adopts() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let hostname = format!("HTUI-TEST-{}", uuid::Uuid::now_v7().simple());
    let first = identity::Identity {
        box_id: BoxId::new(),
        hostname: hostname.clone(),
    };
    let adopted = db
        .store
        .register_box(&first)
        .await
        .expect("register a new box");
    assert_eq!(adopted, first.box_id, "a first registration keeps its id");

    let second = identity::Identity {
        box_id: BoxId::new(),
        hostname: hostname.clone(),
    };
    let again = db
        .store
        .register_box(&second)
        .await
        .expect("register the same hostname under another id");
    assert_eq!(
        again, first.box_id,
        "the hostname already has a row, so its id wins (D6 adopt-DB-id)"
    );

    assert_eq!(
        db.store.identity().box_id,
        db.store.this_box(),
        "the store carries the adopted id, for the caller to write back"
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM box WHERE hostname = $1")
        .bind(&hostname)
        .fetch_one(&db.pool)
        .await
        .expect("count box rows");
    assert_eq!(rows, 1, "the upsert never adds a second row for a hostname");

    let family: String = sqlx::query("SELECT os_family, htui_version FROM box WHERE id = $1")
        .bind(adopted.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read the box row")
        .get("os_family");
    assert!(
        OsFamily::ALL.iter().any(|f| f.as_str() == family),
        "os_family is one of the three CHECK values, got {family}"
    );

    // The caller-side half of D6: `box.toml` is rewritten to the id the database handed back.
    let root = tempfile::tempdir().expect("a temp config root");
    identity::store(
        root.path(),
        &identity::Identity {
            box_id: again,
            hostname,
        },
    )
    .expect("write box.toml");
    assert_eq!(
        identity::load_or_mint(root.path())
            .expect("read box.toml")
            .box_id,
        again,
        "the adopted id is what the next launch reads"
    );

    db.drop_db().await;
}

#[cfg(feature = "demo")]
#[tokio::test]
async fn load_demo_round_trips_a_count_per_table() {
    let Some(mut db) = common::fresh_db().await else {
        return;
    };

    // `fresh_db` already seeded one `app_user` and registered this box, so the assertion is on the
    // delta the fixture adds, table by table - which is what "a count per table" has to mean once
    // seeding runs first (§5.10 before §G).
    let tables = [
        "app_user",
        "box",
        "workspace",
        "project",
        "workspace_project",
        "agent",
        "step_graph",
        "step_graph_phase",
        "prompt_template",
        "item_kind",
        "item_key_counter",
        "item",
        "item_revision",
        "item_link",
        "item_note",
        "document",
        "run",
        "run_step",
        "session_event",
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(common::count(&db.pool, table).await);
    }

    let data = htui_core::fixtures::demo_data();
    db.store.load_demo(&data).await.expect("load the fixture");

    let expected: [(&str, usize); 19] = [
        ("app_user", data.users.len()),
        ("box", data.boxes.len()),
        ("workspace", data.workspaces.len()),
        ("project", data.projects.len()),
        ("workspace_project", data.workspace_projects.len()),
        ("agent", data.agents.len()),
        ("step_graph", data.graphs.len()),
        ("step_graph_phase", data.phases.len()),
        ("prompt_template", data.templates.len()),
        ("item_kind", data.kinds.len()),
        ("item_key_counter", data.item_key_counter.len()),
        ("item", data.items.len()),
        ("item_revision", data.revisions.len()),
        ("item_link", data.links.len()),
        ("item_note", data.notes.len()),
        ("document", data.documents.len()),
        ("run", data.runs.len()),
        ("run_step", data.steps.len()),
        ("session_event", data.events.len()),
    ];

    for (i, (table, len)) in expected.iter().enumerate() {
        let after = common::count(&db.pool, table).await;
        assert_eq!(
            after - before[i],
            i64::try_from(*len).expect("a fixture vector fits in i64"),
            "`{table}` holds one row per DemoData entry"
        );
    }

    assert_eq!(
        db.store.this_box(),
        data.this_box.expect("the fixture names a box"),
        "load_demo points the store at the fixture's box, as MemStore::from_demo does"
    );
    assert_eq!(
        db.store.this_user(),
        data.users.first().expect("the fixture has a user").id,
        "and at the fixture's author"
    );

    // The generated column is computed, never inserted (blueprint C.5).
    let mismatched: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM item WHERE key <> key_prefix || '-' || key_number::text",
    )
    .fetch_one(&db.pool)
    .await
    .expect("check the generated key column");
    assert_eq!(mismatched, 0, "item.key is GENERATED ALWAYS");

    db.drop_db().await;
}

#[test]
fn box_toml_mint_is_stable_across_two_reads() {
    let root = tempfile::tempdir().expect("a temp config root");

    let first = identity::load_or_mint(root.path()).expect("mint box.toml");
    let second = identity::load_or_mint(root.path()).expect("read it back");

    assert_eq!(
        first, second,
        "the box identity is minted once and never re-minted (D6)"
    );
    assert!(
        root.path().join("box.toml").is_file(),
        "minting writes the file"
    );
    assert!(
        !first.hostname.is_empty(),
        "the hostname is recorded alongside the id"
    );
}

#[test]
fn db_fingerprint_is_stable_and_credential_free() {
    let a = identity::db_fingerprint("postgres://alice:secret@db.example:5433/htui");
    let b = identity::db_fingerprint("postgres://bob:other@db.example:5433/htui");
    let c = identity::db_fingerprint("postgres://alice:secret@db.example:5433/other");

    assert_eq!(a, b, "only host, port and dbname are hashed (ANA-9 §4.4)");
    assert_ne!(
        a, c,
        "a different database gets a different cache directory"
    );
    assert_eq!(a.len(), 64, "lowercase hex sha256");
    assert!(
        a.chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_uppercase())
    );
    assert!(
        !identity::db_fingerprint("not a dsn").is_empty(),
        "a DSN that does not parse still gets a stable directory instead of a panic"
    );
}

/// Blueprint H.8 and A.6: the derives `htui-core`'s `sqlx` feature adds are a run-time contract,
/// not a compile-time one - a missing `#[sqlx(type_name = "text")]` compiles and then fails to
/// decode. T2 leans on both, so T1 proves them.
#[cfg(feature = "demo")]
#[tokio::test]
async fn the_core_sqlx_derives_decode_against_text_and_uuid_columns() {
    use htui_core::model::{ItemId, Status};

    let Some(db) = common::demo_db().await else {
        return;
    };

    let statuses: Vec<Status> =
        sqlx::query_scalar("SELECT status FROM item ORDER BY key_prefix, key_number")
            .fetch_all(&db.pool)
            .await
            .expect("a str_enum! enum decodes from a TEXT column (H.8)");
    assert!(!statuses.is_empty(), "the fixture has items");

    let ids: Vec<ItemId> = sqlx::query_scalar("SELECT id FROM item")
        .fetch_all(&db.pool)
        .await
        .expect("a transparent ID newtype decodes from a UUID column (A.6)");
    assert_eq!(ids.len(), statuses.len());

    db.drop_db().await;
}

/// `PgStore::connect` takes its identity from the caller and does no file I/O of its own, so a test
/// never writes to the user's real config directory.
#[tokio::test]
async fn connect_writes_no_files_of_its_own() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let mut entries: Vec<String> = std::fs::read_dir(&db.config_root)
        .expect("the throwaway config root exists")
        .map(|e| {
            e.expect("a readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![identity::BOX_FILE.to_owned()],
        "only the box.toml the harness minted; connect wrote nothing"
    );
    assert_eq!(
        db.store.this_box(),
        db.identity.box_id,
        "a first registration keeps the caller's id"
    );

    let root = db.config_root.clone();
    db.drop_db().await;
    assert!(!root.exists(), "drop_db removes the throwaway config root");
}
