//! Throwaway-database harness for the `htui-store` integration tests (blueprint E, plan D13).
//!
//! Every test that needs a server creates its own database, migrates it and drops it again, so the
//! concurrency cases of ANA-9 §11 are deterministic and two test binaries never share state.
//! `HTUI_TEST_DATABASE_URL` is a *maintenance* DSN whose user has `CREATEDB`; when it is unset the
//! helpers print [`SKIP`] and answer `None`, which is what keeps `cargo test --workspace
//! --all-features` green on a box without Postgres.
#![allow(dead_code)] // each test binary compiles the whole module and uses a subset of it.

use std::path::PathBuf;
use std::str::FromStr as _;

use htui_store::{Identity, MigrationState, PgStore, identity};
use sqlx::postgres::{PgConnectOptions, PgPool};
use sqlx::{AssertSqlSafe, Connection as _, PgConnection};
use uuid::Uuid;

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

/// A fresh database with the migrations **not** applied - or `None` when [`ENV_URL`] is unset.
///
/// The migration tests need a database in that state; every other test wants [`fresh_db`].
pub async fn bare_db() -> Option<TestDb> {
    let maint_url = match std::env::var(ENV_URL) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            println!("{SKIP}");
            return None;
        }
    };

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

/// [`fresh_db`] plus `store.load_demo(&fixtures::demo_data())`.
#[cfg(feature = "demo")]
pub async fn demo_db() -> Option<TestDb> {
    let mut db = fresh_db().await?;
    db.store
        .load_demo(&htui_core::fixtures::demo_data())
        .await
        .expect("load the demo fixture");
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

/// `COUNT(*)` of one table, for the per-table round-trip assertions.
pub async fn count(pool: &PgPool, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
    sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("COUNT(*) FROM {table}: {e}"))
}
