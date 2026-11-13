//! MOD-92 D2: the read-only session `htui status`, `htui waiting` and `--search-items` connect
//! over. `PgStore::connect_read_only` keeps `connect_headless`'s refusals, skips its bootstrap (no
//! seed, no box registration), and opens every session with `default_transaction_read_only = on`,
//! so Postgres itself refuses a write whatever code path attempts it.
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes. `an_unreachable_server_is_unreachable_read_only` and
//! `a_pooler_rejecting_the_read_only_parameter_is_a_refusal` need no server and therefore never
//! skip.
#![cfg(feature = "test-support")]

use htui_store::testkit as common;

use std::time::Duration;

use htui_core::fixtures::ids;
use htui_core::model::{BoxId, NewWorkspace, Scope, UserId, WorkspaceId};
use htui_core::store::{ReadStore as _, StoreError, WriteStore as _};
use htui_store::{HTUI_VERSION, HeadlessError, MIGRATOR, PgStore, TARGET_VERSION_KEY};
use serde_json::json;

/// The `connect_timeout` every read-only connect here passes.
const WAIT: Duration = Duration::from_secs(5);

/// `read_only_sql_transaction`: what Postgres answers a write in a read-only transaction.
const READ_ONLY_SQLSTATE: &str = "25006";

/// How many tables the `public` schema holds.
async fn public_tables(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
        .fetch_one(pool)
        .await
        .expect("count the public tables")
}

/// A read-only session answers reads, and carries the caller's box id without registering it.
#[tokio::test]
async fn a_read_only_session_answers_reads() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let store = PgStore::connect_read_only(&db.url, Some(db.identity.box_id), WAIT)
        .await
        .expect("a migrated database connects read-only");
    store.workspaces().await.expect("workspaces answers");
    assert_eq!(store.this_box(), db.identity.box_id, "box.toml's id");
    assert_eq!(store.this_user(), UserId::default(), "no user is resolved");
    assert_eq!(store.registration(), None, "nothing was registered");

    let anonymous = PgStore::connect_read_only(&db.url, None, WAIT)
        .await
        .expect("no box.toml connects too");
    assert_eq!(
        anonymous.this_box(),
        BoxId::default(),
        "nil without box.toml"
    );

    db.drop_db().await;
}

/// The session refuses a raw write: the read-only flag is the session's, not a code path's.
#[tokio::test]
async fn a_raw_insert_is_refused_by_the_session() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let store = PgStore::connect_read_only(&db.url, Some(db.identity.box_id), WAIT)
        .await
        .expect("connect read-only");

    let err = sqlx::query(
        "INSERT INTO workspace (slug, name, created_by) \
         SELECT 'read-only-probe', 'probe', id FROM app_user LIMIT 1",
    )
    .execute(store.pool())
    .await
    .expect_err("a read-only session refuses an INSERT");
    match err {
        sqlx::Error::Database(e) => {
            assert_eq!(e.code().as_deref(), Some(READ_ONLY_SQLSTATE), "{e}");
        }
        other => panic!("expected a database refusal, got {other:?}"),
    }

    db.drop_db().await;
}

/// A `WriteStore` write through the read-only store is refused and writes nothing.
#[tokio::test]
async fn a_store_write_is_refused_and_writes_nothing() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let store = PgStore::connect_read_only(&db.url, Some(db.identity.box_id), WAIT)
        .await
        .expect("connect read-only");
    let before = common::count(&db.pool, "workspace").await;

    let refused = store
        .create_workspace(NewWorkspace {
            id: WorkspaceId::new(),
            slug: "read-only-probe".to_owned(),
            name: "Read-only probe".to_owned(),
            description: String::new(),
            created_by: db.store.this_user(),
        })
        .await
        .expect_err("a read-only store refuses a write");
    match refused {
        StoreError::Backend(m) => assert!(m.starts_with(READ_ONLY_SQLSTATE), "{m}"),
        other => panic!("expected a 25006 Backend refusal, got {other:?}"),
    }
    assert_eq!(
        common::count(&db.pool, "workspace").await,
        before,
        "no workspace row appeared"
    );

    db.drop_db().await;
}

/// No bootstrap: a read-only connect seeds nothing and registers no box, not even an unknown one.
#[tokio::test]
async fn a_read_only_connect_registers_and_seeds_nothing() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let counts = || async {
        (
            common::count(&db.pool, "box").await,
            common::count(&db.pool, "app_user").await,
            common::count(&db.pool, "capability_tag").await,
        )
    };
    let before = counts().await;

    let stranger = BoxId::new();
    PgStore::connect_read_only(&db.url, Some(stranger), WAIT)
        .await
        .expect("an unknown box connects read-only");
    PgStore::connect_read_only(&db.url, None, WAIT)
        .await
        .expect("no box connects read-only");

    assert_eq!(counts().await, before, "box, app_user, capability_tag");
    let stranger_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM box WHERE id = $1")
        .bind(stranger.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("count the stranger's box rows");
    assert_eq!(stranger_rows, 0, "the new id is not registered");

    db.drop_db().await;
}

/// A pending schema is refused, as `connect_headless` refuses it, and nothing is applied.
#[tokio::test]
async fn a_pending_schema_is_refused_read_only() {
    let Some(db) = common::bare_db().await else {
        return;
    };
    let embedded = MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .count();
    let tables = public_tables(&db.pool).await;

    let refused = PgStore::connect_read_only(&db.url, Some(db.identity.box_id), WAIT)
        .await
        .expect_err("a pending schema is refused");
    assert_eq!(refused, HeadlessError::MigrationsPending(embedded));
    assert_eq!(
        common::count(&db.pool, "_sqlx_migrations").await,
        0,
        "nothing applied"
    );
    assert_eq!(public_tables(&db.pool).await, tables, "no table created");

    db.drop_db().await;
}

/// A build below the stored target version is refused, as `connect_headless` refuses it.
#[tokio::test]
async fn a_build_below_the_target_is_refused_read_only() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    sqlx::query(
        "INSERT INTO app_setting (key, value) VALUES ($1, $2) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
    )
    .bind(TARGET_VERSION_KEY)
    .bind(json!("999.0.0"))
    .execute(&db.pool)
    .await
    .expect("plant a target version");

    let refused = PgStore::connect_read_only(&db.url, Some(db.identity.box_id), WAIT)
        .await
        .expect_err("a build below the target is refused");
    assert_eq!(
        refused,
        HeadlessError::BelowTarget {
            ours: HTUI_VERSION.into(),
            target: "999.0.0".into(),
        }
    );

    db.drop_db().await;
}

/// A server that is not there is `Unreachable`, the one refusal the CLI calls "offline".
#[tokio::test]
async fn an_unreachable_server_is_unreachable_read_only() {
    let refused = PgStore::connect_read_only(
        "postgres://nobody:nothing@127.0.0.1:1/none",
        None,
        Duration::from_secs(2),
    )
    .await
    .expect_err("nothing listens on port 1");
    assert!(
        matches!(refused, HeadlessError::Store(StoreError::Unreachable(_))),
        "{refused:?}"
    );
}

/// Every read the CLI verbs make (MOD-92 T3) answers on a read-only session: a hidden
/// `FOR UPDATE` or write in a read path fails here, not in the CLI.
#[tokio::test]
async fn every_cli_read_answers_on_a_read_only_session() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let store = PgStore::connect_read_only(&db.url, Some(ids::BOX), WAIT)
        .await
        .expect("connect read-only");

    let workspaces = store.workspaces().await.expect("workspaces");
    let first = workspaces.first().expect("the fixture has a workspace");
    let scope = Scope::from_workspace(first);
    assert!(
        store.box_info().await.expect("box_info").is_some(),
        "the fixture's box"
    );
    store.active_runs(&scope).await.expect("active_runs");
    store
        .waiting_candidates(&scope)
        .await
        .expect("waiting_candidates");
    store
        .open_permissions(&scope)
        .await
        .expect("open_permissions");
    store.repo_paths(ids::BOX).await.expect("repo_paths");
    let project = first.projects.first().expect("a project").project_id;
    store.repos(project).await.expect("repos");

    db.drop_db().await;
}

/// A one-shot stand-in for a pooler that rejects the read-only startup parameter, as PgBouncer
/// does (`unsupported startup parameter: options`, SQLSTATE `08P01`, then it hangs up): it reads
/// each startup packet and answers it with that `ErrorResponse`. Returns the DSN to dial.
fn startup_rejecting_server() -> String {
    use std::io::{Read as _, Write as _};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let port = listener.local_addr().expect("a local address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).is_err() {
                continue;
            }
            let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let mut rest = vec![0u8; len.saturating_sub(8)];
            if stream.read_exact(&mut rest).is_err() {
                continue;
            }
            let mut fields = Vec::new();
            for (tag, value) in [
                (b'S', "FATAL"),
                (b'V', "FATAL"),
                (b'C', "08P01"),
                (b'M', "unsupported startup parameter: options"),
            ] {
                fields.push(tag);
                fields.extend_from_slice(value.as_bytes());
                fields.push(0);
            }
            fields.push(0);
            let mut message = vec![b'E'];
            let body_len = u32::try_from(fields.len() + 4).expect("a short message");
            message.extend_from_slice(&body_len.to_be_bytes());
            message.extend_from_slice(&fields);
            let _ = stream.write_all(&message);
        }
    });
    format!("postgres://nobody@127.0.0.1:{port}/none?sslmode=disable")
}

/// Review R1 M1: a pooler that rejects the read-only startup parameter (`08P01` while the
/// connection starts) answered, so the read-only connect refuses rather than calling the server
/// unreachable; a script that polls would otherwise read "offline" forever. Only that connect:
/// the read-write connects keep `08P01` as a lost connection.
#[tokio::test]
async fn a_pooler_rejecting_the_read_only_parameter_is_a_refusal() {
    let dsn = startup_rejecting_server();
    let refused = PgStore::connect_read_only(&dsn, None, Duration::from_secs(5))
        .await
        .expect_err("the startup packet is rejected");
    let HeadlessError::Store(StoreError::Backend(message)) = &refused else {
        panic!("expected a Backend refusal, got {refused:?}");
    };
    assert!(message.starts_with("08P01: "), "{message}");
    assert!(
        message.contains("unsupported startup parameter: options"),
        "{message}"
    );
    assert!(
        message.contains("PgBouncer"),
        "names the likely cause: {message}"
    );

    let headless = PgStore::connect_headless(
        &dsn,
        &htui_store::Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".into(),
        },
        Duration::from_secs(5),
        htui_store::pg::PoolSize::WORKER_DEFAULT,
    )
    .await
    .expect_err("the startup packet is rejected");
    assert!(
        matches!(headless, HeadlessError::Store(StoreError::Unreachable(_))),
        "read-write connects keep 08P01 as unreachable: {headless:?}"
    );
}

/// Review R1 L6: a DSN that asks for a writable session (`options=-c
/// default_transaction_read_only=off`) still gets a read-only one: `open_pool` appends `on` after
/// the DSN's own options, and the last setting wins.
#[tokio::test]
async fn a_dsn_asking_for_writes_still_gets_a_read_only_session() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let separator = if db.url.contains('?') { '&' } else { '?' };
    let dsn = format!(
        "{}{separator}options=-c%20default_transaction_read_only%3Doff",
        db.url
    );
    let store = PgStore::connect_read_only(&dsn, Some(db.identity.box_id), WAIT)
        .await
        .expect("connect read-only");
    let setting: String = sqlx::query_scalar("SHOW default_transaction_read_only")
        .fetch_one(store.pool())
        .await
        .expect("read the setting");
    assert_eq!(setting, "on");

    let err = sqlx::query(
        "INSERT INTO workspace (slug, name, created_by) \
         SELECT 'read-only-probe', 'probe', id FROM app_user LIMIT 1",
    )
    .execute(store.pool())
    .await
    .expect_err("a read-only session refuses an INSERT");
    match err {
        sqlx::Error::Database(e) => {
            assert_eq!(e.code().as_deref(), Some(READ_ONLY_SQLSTATE), "{e}");
        }
        other => panic!("expected a database refusal, got {other:?}"),
    }

    db.drop_db().await;
}
