//! Throwaway-database harness and mirror seeder for the tests (MOD-6 blueprint E, plan D13).
//!
//! Every test that needs a server creates its own database, migrates it and drops it again, so the
//! concurrency cases of ANA-9 §11 are deterministic and two test binaries never share state.
//! `HTUI_TEST_DATABASE_URL` is a *maintenance* DSN whose user has `CREATEDB`; when it is unset the
//! helpers print [`SKIP`] and answer `None`, which is what keeps `cargo test --workspace
//! --all-features` green on a box without Postgres.
//!
//! This lived under `tests/common/` until MOD-2 milestone 4 (T21). It moved into the library
//! behind the `test-support` feature because `docs/ANA-4.md` §11 criterion 12 is one sentence with
//! two halves - a **driver** writes the offline buffer, a **server** lands it - and only the `htui`
//! crate depends on both `htui-agent` and this one. A captured fixture as the joint between the
//! two would not fail when the recorder and the uploader drift, which is the whole point of the
//! criterion. `htui-core::fixtures` sets the precedent: test-only data, in the library, behind a
//! feature.

use std::path::PathBuf;
use std::str::FromStr as _;

use htui_core::fixtures::DemoData;
use htui_core::store::Result;
use sqlx::postgres::{PgConnectOptions, PgPool};
use sqlx::{AssertSqlSafe, Connection as _, PgConnection};
use uuid::Uuid;

use crate::cache::CacheStore;
use crate::cache::read::ts_bind;
use crate::error::map_sqlx;
use crate::{Identity, MigrationState, PgStore, identity};

/// The environment variable holding the maintenance DSN (plan D13).
pub const ENV_URL: &str = "HTUI_TEST_DATABASE_URL";

/// Printed, byte for byte, by every helper that finds [`ENV_URL`] unset.
pub const SKIP: &str = "skipped: HTUI_TEST_DATABASE_URL not set";

/// A throwaway database and the store that owns it.
#[derive(Debug)]
pub struct TestDb {
    /// A [`PgStore`] connected to the fresh database.
    pub store: PgStore,
    /// The same pool, for raw assertions and for a second connection in the race tests.
    pub pool: PgPool,
    /// `htui_test_<12 hex>`.
    pub name: String,
    /// The DSN of this database, for a second `PgStore::connect`.
    pub url: String,
    /// The maintenance DSN the database was created through, for [`TestDb::drop_db`].
    pub maint_url: String,
    /// Throwaway config root holding this test's `box.toml`.
    ///
    /// `PgStore::connect` does no file I/O of its own, so this is the only place a test writes an
    /// identity: nothing under `%APPDATA%\htui` (or `~/.config/htui`) is ever created by
    /// `cargo test`. Removed by [`TestDb::drop_db`] and by the `Drop` net.
    pub config_root: PathBuf,
    /// The identity this database's store registered under.
    pub identity: Identity,
    /// What `PgStore::connect` reported before any `apply_migrations` this helper ran.
    pub migrations_at_connect: MigrationState,
    /// Set by [`TestDb::drop_db`] so the `Drop` panic net is a no-op on the normal path.
    dropped: bool,
}

static SWEEP: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// A fresh database with the migrations **not** applied - or `None` when [`ENV_URL`] is unset.
///
/// The migration tests need a database in that state; every other test wants [`fresh_db`].
pub async fn bare_db() -> Option<TestDb> {
    let maint_url = match std::env::var(ENV_URL) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            if std::env::var("CI").is_ok() {
                panic!("{} is not set but CI is running. Database tests must not be skipped in CI.", ENV_URL);
            }
            println!("{SKIP}");
            return None;
        }
    };

    SWEEP
        .get_or_init(|| async {
            let Ok(mut sweep_conn) = PgConnection::connect(&maint_url).await else {
                return;
            };
            let stale: Vec<(String,)> = sqlx::query_as(
                r#"
                SELECT datname 
                FROM pg_database 
                WHERE datname LIKE 'htui_test_%' 
                  AND (pg_stat_file('base/' || oid, true)).modification < now() - interval '1 hour'
                "#,
            )
            .fetch_all(&mut sweep_conn)
            .await
            .unwrap_or_default();

            for (stale_name,) in stale {
                let _ = sqlx::raw_sql(AssertSqlSafe(format!(
                    "DROP DATABASE IF EXISTS \"{stale_name}\" WITH (FORCE)"
                )))
                .execute(&mut sweep_conn)
                .await;
            }
            let _ = sweep_conn.close().await;
        })
        .await;

    // The *tail* of a UUIDv7 is the random half; its head is a millisecond timestamp, which two
    // tests starting in the same millisecond share.
    let hex = Uuid::now_v7().simple().to_string();
    let name = format!("htui_test_{}", &hex[hex.len() - 12..]);
    let mut maint = PgConnection::connect(&maint_url)
        .await
        .expect("connect to HTUI_TEST_DATABASE_URL");
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
        .execute(&mut maint)
        .await
        .expect("CREATE DATABASE");
    maint
        .close()
        .await
        .expect("close the maintenance connection");

    let config_root = std::env::temp_dir().join(format!("htui-test-{name}"));
    std::fs::create_dir_all(&config_root).expect("create the throwaway config root");
    let identity = identity::load_or_mint(&config_root).expect("mint a throwaway box.toml");

    let url = with_database(&maint_url, &name);
    let connected = PgStore::connect(&url, &identity)
        .await
        .expect("PgStore::connect");
    let pool = connected.store.pool().clone();

    Some(TestDb {
        store: connected.store,
        pool,
        name,
        url,
        maint_url,
        config_root,
        identity,
        migrations_at_connect: connected.migrations,
        dropped: false,
    })
}

/// A fresh, migrated, empty database - or `None` when [`ENV_URL`] is unset.
pub async fn fresh_db() -> Option<TestDb> {
    let mut db = bare_db().await?;
    db.store
        .apply_migrations()
        .await
        .expect("apply the embedded migrations");
    Some(db)
}

/// [`fresh_db`] plus `store.load_demo(&fixtures::demo_data())`, with the seeded user removed.
///
/// `apply_migrations` seeds an `app_user` named after this OS user and registers this box under
/// it; the fixture then brings its own, whose `created_at` is the epoch. Two rows is exactly what
/// `R-USR-2` forbids, and `PgStore::seed_if_empty_as` answers with the oldest of them - so a
/// second `PgStore::connect` against such a database would register this hostname under the
/// fixture's user while the row already exists under the seeded one, and collide on `box_pkey`.
/// The seeded pair therefore goes, leaving the fixture's world as the only one. `db.store` needs
/// no reconnect: `load_demo` repoints `this_box` / `this_user` at the fixture itself.
pub async fn demo_db() -> Option<TestDb> {
    let mut db = fresh_db().await?;
    let seeded = db.store.this_user();
    db.store
        .load_demo(&htui_core::fixtures::demo_data())
        .await
        .expect("load the demo fixture");

    sqlx::query("DELETE FROM box WHERE user_id = $1")
        .bind(seeded.as_uuid())
        .execute(&db.pool)
        .await
        .expect("drop the box row of the seeded user");
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(seeded.as_uuid())
        .execute(&db.pool)
        .await
        .expect("drop the seeded app_user");
    Some(db)
}

impl TestDb {
    /// Closes the pool and drops the database. Every test calls this on its last line.
    ///
    /// This is the only cleanup path that reports an error; the `Drop` impl below is the
    /// best-effort net for a test that panicked before reaching this line.
    pub async fn drop_db(mut self) {
        self.dropped = true;
        self.pool.close().await;
        let mut maint = PgConnection::connect(&self.maint_url)
            .await
            .expect("connect to the maintenance database");
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
            self.name
        )))
        .execute(&mut maint)
        .await
        .expect("DROP DATABASE");
        maint
            .close()
            .await
            .expect("close the maintenance connection");
        let _ = std::fs::remove_dir_all(&self.config_root);
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if self.dropped {
            return;
        }
        // A test that failed mid-way never reached `drop_db`. The ambient runtime may already be
        // shutting down, so `tokio::spawn` is not reliable here: use a thread with its own
        // single-threaded runtime, and swallow every error - this is a net, not an assertion.
        let _ = std::fs::remove_dir_all(&self.config_root);
        let maint_url = self.maint_url.clone();
        let name = self.name.clone();
        let cleanup = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async {
                if let Ok(mut conn) = PgConnection::connect(&maint_url).await {
                    let _ = sqlx::raw_sql(AssertSqlSafe(format!(
                        "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
                    )))
                    .execute(&mut conn)
                    .await;
                    let _ = conn.close().await;
                }
            });
        });
        let _ = cleanup.join();
    }
}

/// Rewrites a DSN's database name, keeping scheme, credentials, host, port and query string.
///
/// `PgConnectOptions` has no password getter, so re-serialising it would drop the credentials;
/// rewriting the path segment is both lossless and what `HTUI_TEST_DATABASE_URL` documents.
#[must_use]
pub fn with_database(dsn: &str, database: &str) -> String {
    let (base, query) = match dsn.find('?') {
        Some(i) => (&dsn[..i], &dsn[i..]),
        None => (dsn, ""),
    };
    let authority = base.find("://").map_or(0, |i| i + 3);
    let end = base[authority..]
        .find('/')
        .map_or(base.len(), |i| authority + i);
    format!("{}/{database}{query}", &base[..end])
}

/// Parses a DSN, so a test can assert on the pieces `identity::db_fingerprint` hashes.
#[must_use]
pub fn parse_dsn(dsn: &str) -> PgConnectOptions {
    PgConnectOptions::from_str(dsn).expect("a parseable DSN")
}

/// Writes the six unscoped tables of a [`DemoData`] straight into a mirror, with no server.
///
/// The refresher is the only *production* writer of `cache.sqlite` and it needs a `PgPool`; a test
/// that wants an offline box - one that has synced once and is now alone with its mirror - has no
/// server to sync from. This is that box's history, written with the mirror's own encodings
/// (microsecond stamps, JSON text, `0`/`1` booleans) so a test never hand-rolls a SQLite row and
/// cannot drift from `cache::refresh`'s bind lists by accident.
///
/// Six tables, and deliberately only six: `app_user`, the own `box` row, `workspace`,
/// `workspace_project`, `project` and `agent` - everything an offline chat resolves before it can
/// start (`docs/ANA-4.md` §4.1: the box, the user, the registry row, the project it belongs to).
/// Items, runs and events are the cursor-driven tables and no offline *write* path needs them.
///
/// Only `demo.this_box`'s row is written, because §4.4 says the mirror holds the own `box` row and
/// no other.
///
/// # Errors
///
/// Whatever the driver reports, through [`map_sqlx`], and [`htui_core::store::StoreError::Backend`]
/// when a JSON column cannot be serialised.
pub async fn seed_mirror(cache: &CacheStore, demo: &DemoData) -> Result<()> {
    let mut tx = cache.pool().begin().await.map_err(map_sqlx)?;

    for user in &demo.users {
        sqlx::query(
            "INSERT INTO app_user (id, name, email, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user.id.to_string())
        .bind(&user.name)
        .bind(user.email.as_deref())
        .bind(ts_bind(user.created_at))
        .bind(ts_bind(user.updated_at))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for row in demo
        .boxes
        .iter()
        .filter(|row| demo.this_box.is_some_and(|id| id == row.id))
    {
        sqlx::query(
            "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, cpu, ram_mb, \
                              gpu_present, gpu_vendor, htui_version, probed_tags, declared_tags, \
                              quirks, settings, registered_at, last_seen_at, last_probed_at, \
                              updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.id.to_string())
        .bind(row.user_id.to_string())
        .bind(&row.hostname)
        .bind(row.os_family.as_str())
        .bind(&row.os_version)
        .bind(&row.arch)
        .bind(&row.cpu)
        .bind(row.ram_mb.map(i64::from))
        .bind(i64::from(row.gpu_present))
        .bind(row.gpu_vendor.as_deref())
        .bind(&row.htui_version)
        .bind(strings(&row.probed_tags)?)
        .bind(strings(&row.declared_tags)?)
        .bind(&row.quirks)
        .bind(json(&row.settings)?)
        .bind(ts_bind(row.registered_at))
        .bind(ts_bind(row.last_seen_at))
        .bind(row.last_probed_at.map(ts_bind))
        .bind(ts_bind(row.updated_at))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for workspace in &demo.workspaces {
        sqlx::query(
            "INSERT INTO workspace (id, slug, name, description, created_by, created_at, \
                                    updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(workspace.id.to_string())
        .bind(&workspace.slug)
        .bind(&workspace.name)
        .bind(&workspace.description)
        .bind(workspace.created_by.to_string())
        .bind(ts_bind(workspace.created_at))
        .bind(ts_bind(workspace.updated_at))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for link in &demo.workspace_projects {
        sqlx::query(
            "INSERT INTO workspace_project (workspace_id, project_id, position) VALUES (?, ?, ?)",
        )
        .bind(link.workspace_id.to_string())
        .bind(link.project_id.to_string())
        .bind(i64::from(link.position))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for project in &demo.projects {
        sqlx::query(
            "INSERT INTO project (id, slug, name, description, secret_provider, secret_scope, \
                                  settings, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(project.id.to_string())
        .bind(&project.slug)
        .bind(&project.name)
        .bind(&project.description)
        .bind(project.secret_provider.as_deref())
        .bind(project.secret_scope.as_deref())
        .bind(json(&project.settings)?)
        .bind(project.created_by.to_string())
        .bind(ts_bind(project.created_at))
        .bind(ts_bind(project.updated_at))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for agent in &demo.agents {
        sqlx::query(
            "INSERT INTO agent (id, name, transport, launch, models, default_model, billing, \
                                enabled, settings, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(agent.id.to_string())
        .bind(&agent.name)
        .bind(agent.transport.as_str())
        .bind(json(&agent.launch)?)
        .bind(strings(&agent.models)?)
        .bind(agent.default_model.as_deref())
        .bind(agent.billing.as_str())
        .bind(i64::from(agent.enabled))
        .bind(json(&agent.settings)?)
        .bind(ts_bind(agent.created_at))
        .bind(ts_bind(agent.updated_at))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    tx.commit().await.map_err(map_sqlx)
}

/// A `JSONB` column as the mirror's TEXT, the `cache::refresh` encoding.
fn json(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string(value).map_err(|e| {
        htui_core::store::StoreError::Backend(format!(
            "testkit: cannot serialise a JSON column: {e}"
        ))
    })
}

/// A `TEXT[]` column as the mirror's JSON array, the `cache::refresh` encoding.
fn strings(values: &[String]) -> Result<String> {
    serde_json::to_string(values).map_err(|e| {
        htui_core::store::StoreError::Backend(format!(
            "testkit: cannot serialise a text[] column: {e}"
        ))
    })
}

/// `COUNT(*)` of one table, for the per-table round-trip assertions.
pub async fn count(pool: &PgPool, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
    sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("COUNT(*) FROM {table}: {e}"))
}
