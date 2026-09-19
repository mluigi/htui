//! The per-box SQLite mirror of `docs/ANA-9.md` §4.4 (MOD-6 blueprint C.10, plan D8).
//!
//! [`CacheStore`] is `ReadStore` only: an offline write is a compile error rather than a runtime
//! flag (§6.1). [`refresh`] owns the only writer (§6.3); [`read`] is the reader every offline view
//! goes through.
//!
//! Nothing in this module may run on the UI task (`R-NF-3`).

pub mod read;
pub mod refresh;

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use htui_core::store::{Result, StoreError};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{AssertSqlSafe, Row as _, SqlitePool};

use crate::CACHE_MIGRATOR;
use crate::error::{map_migrate, map_sqlx};

/// The mirror file inside [`CacheStore::dir`].
pub const CACHE_FILE: &str = "cache.sqlite";

/// The seventeen mirrored tables of §4.4, in foreign-key order.
///
/// The order is the one [`refresh::run_pass`] fills them in and the one [`CacheStore::rebuild`]
/// empties them in; `cache_meta` and `cache_cursor` are local and deliberately absent.
///
/// `agent` joined the list in MOD-2 milestone 4 (plan D31): a buffered chat could not resolve a
/// driver without the registry row, and §4.4 was corrected in the same milestone to say so. MOD-25
/// refuses that chat instead, but the row stays mirrored — the Backlog and the agents section read
/// it off the mirror too. `agent_box` is
/// still absent - it is a probe snapshot whose columns milestone 5 changes.
///
/// `run_step_tree` joined in MOD-4 milestone 1 (plan D7, D9): ANA-2 §4.6's isolation trees are
/// what an offline reader needs to say where a step's work went, and like `run_step_commit` the
/// table has no `updated_at` of its own and rides its parent step's.
pub const MIRRORED_TABLES: [&str; 17] = [
    "app_user",
    "agent",
    "box",
    "workspace",
    "workspace_project",
    "project",
    "repo",
    "item_kind",
    "item",
    "item_link",
    "item_note",
    "document",
    "run",
    "run_step",
    "run_step_commit",
    "run_step_tree",
    "session_event",
];

/// A [`CacheMeta::last_full_refresh_at`] older than this clears every cursor on the next
/// [`CacheStore::open`], so the following pass is a full pull (plan D8).
///
/// Seven days. It does **not** rebuild the file: a full pass is idempotent and closes any gap the
/// overlap window of §4.4 did not.
pub const FULL_REFRESH_MAX_AGE: TimeDelta = TimeDelta::days(7);

/// How long a reader waits for the single writer before giving up (§4.4: WAL, one writer).
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// `cache_meta` keys (§4.4).
const KEY_SCHEMA_VERSION: &str = "schema_version";
const KEY_DB_FINGERPRINT: &str = "db_fingerprint";
const KEY_BUILT_AT: &str = "built_at";
const KEY_LAST_FULL_REFRESH_AT: &str = "last_full_refresh_at";

/// The per-box read-only mirror (§4.4). `ReadStore` only: it can never accept a write.
///
/// `Clone` because an [`SqlitePool`] is an `Arc` inside, which is what lets the store worker move
/// the cache from an offline backend into an online one and hand a second handle to
/// [`refresh::Refresher`].
#[derive(Debug, Clone)]
pub struct CacheStore {
    pool: SqlitePool,
    dir: PathBuf,
}

/// What `cache_meta` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMeta {
    /// The Postgres migration version the mirror was built from.
    pub schema_version: i64,
    /// `sha256(host:port/dbname)` of the server this mirror belongs to.
    pub db_fingerprint: String,
    /// When the file was created (or last rebuilt).
    pub built_at: DateTime<Utc>,
    /// When the last full (cursor-at-zero) pass finished; `None` until one has.
    pub last_full_refresh_at: Option<DateTime<Utc>>,
}

impl CacheStore {
    /// Opens `<root>/cache/<fingerprint>/cache.sqlite`, creating and migrating it if needed.
    ///
    /// Creates `<root>/cache/<fingerprint>/` and its `pending/` subdirectory, seals any offline
    /// chat buffer a previous run left open ([`pending::seal_orphaned`], `[H-1]`), then connects
    /// with `create_if_missing`, WAL journalling, a five-second busy timeout and **foreign keys
    /// off**:
    /// the mirror is fed in foreign-key order by one writer, and a partial mirror must not refuse a
    /// row whose parent is not mirrored yet.
    ///
    /// **Rebuild** - delete the file, recreate it, re-migrate and write a fresh `cache_meta` -
    /// when `cache_meta.schema_version` differs from `schema_version`, when
    /// `cache_meta.db_fingerprint` differs from `fingerprint`, or when the file exists but has no
    /// `cache_meta` row. A [`CacheMeta::last_full_refresh_at`] older than
    /// [`FULL_REFRESH_MAX_AGE`] does **not** rebuild: it clears `cache_cursor` so the next pass is
    /// a full one (plan D8).
    ///
    /// `schema_version` is [`crate::PgStore::schema_version`]; `fingerprint` is
    /// [`crate::identity::db_fingerprint`] of the DSN, which is also the directory name
    /// [`crate::identity::cache_dir`] computes.
    ///
    /// # Errors
    ///
    /// [`StoreError::Backend`] when the directory cannot be created, `pending/` cannot be listed
    /// or the file cannot be removed, and whatever the driver or the migrator reports otherwise. A
    /// buffer that cannot be *sealed* is not one of them: [`pending::seal_orphaned`] warns and
    /// carries on, because an unreadable best-effort buffer must not cost the user `start()`.
    pub async fn open(root: &Path, fingerprint: &str, schema_version: i64) -> Result<Self> {
        let dir = root.join("cache").join(fingerprint);
        create_dir(&dir)?;
        let path = dir.join(CACHE_FILE);
        let existed = path.exists();

        let mut pool = connect(&path).await?;
        CACHE_MIGRATOR.run(&pool).await.map_err(map_migrate)?;

        let stored = read_meta(&pool).await?;
        let rebuild = stored.as_ref().is_none_or(|meta| {
            meta.schema_version != schema_version || meta.db_fingerprint != fingerprint
        });

        if rebuild {
            if existed {
                pool.close().await;
                remove_file_set(&path)?;
                pool = connect(&path).await?;
                CACHE_MIGRATOR.run(&pool).await.map_err(map_migrate)?;
            }
            write_fresh_meta(&pool, schema_version, fingerprint).await?;
        } else if stored
            .and_then(|meta| meta.last_full_refresh_at)
            .is_some_and(|at| Utc::now().signed_duration_since(at) > FULL_REFRESH_MAX_AGE)
        {
            tracing::info!("cache: the last full refresh is over a week old; forcing a full pass");
            sqlx::query("DELETE FROM cache_cursor")
                .execute(&pool)
                .await
                .map_err(map_sqlx)?;
        }

        Ok(Self { pool, dir })
    }

    /// Drops every mirrored row and every cursor, keeping the file and `cache_meta`.
    ///
    /// This is `Settings > Rebuild cache`; the next pass refills from zero, which is also why
    /// `last_full_refresh_at` is cleared - nothing has been fully pulled into *this* content yet.
    /// `schema_version`, `db_fingerprint` and `built_at` survive, so [`CacheStore::meta`] keeps
    /// answering and [`CacheStore::open`] does not then rebuild the file on the next launch.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn rebuild(&self) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        for table in MIRRORED_TABLES {
            // `table` is one of the constants above, never caller data.
            sqlx::query(AssertSqlSafe(format!("DELETE FROM {table}")))
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        }
        sqlx::query("DELETE FROM cache_cursor")
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        sqlx::query("DELETE FROM cache_meta WHERE key = ?")
            .bind(KEY_LAST_FULL_REFRESH_AT)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)
    }

    /// The `cache_meta` row set.
    ///
    /// # Errors
    ///
    /// [`StoreError::Backend`] when a required key is missing or unparseable - which is a mirror
    /// [`CacheStore::open`] would have rebuilt, so in practice it cannot be observed.
    pub async fn meta(&self) -> Result<CacheMeta> {
        read_meta(&self.pool).await?.ok_or_else(|| {
            StoreError::Backend("cache: cache_meta is missing or incomplete".to_owned())
        })
    }

    /// `<root>/cache/<fingerprint>`.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Closes every connection to the mirror and waits for them to go.
    ///
    /// Not in blueprint C.10, and needed anyway: on Windows an open handle makes the file
    /// undeletable, so a caller that wants to remove or replace `cache.sqlite` - a test tearing
    /// down its throwaway config root, or the store worker before it hands the directory to
    /// something else - has to close first. Dropping the last [`CacheStore`] closes the pool too,
    /// but asynchronously, which is exactly the race this avoids.
    ///
    /// Every clone of this handle shares the pool, so this closes it for all of them.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// The pool, for [`refresh`] (the only writer) and for the tests.
    #[must_use]
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Records that the pass which just finished was a full pull (plan D8's seven-day rule).
    pub(crate) async fn note_full_refresh(&self, at: DateTime<Utc>) -> Result<()> {
        put_meta(&self.pool, KEY_LAST_FULL_REFRESH_AT, &at.to_rfc3339()).await
    }
}

/// Opens one pool over `path` with the §4.4 connect options.
async fn connect(path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(BUSY_TIMEOUT)
        // One writer feeds the mirror in FK order; a partially mirrored parent must not refuse a
        // child row, and the server already enforced every constraint this file would repeat.
        .foreign_keys(false);
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .map_err(map_sqlx)
}

/// `create_dir_all`, with the path in the error text.
fn create_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .map_err(|e| StoreError::Backend(format!("cannot create {}: {e}", path.display())))
}

/// Removes `cache.sqlite` and the two files WAL mode keeps beside it.
fn remove_file_set(path: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let target = PathBuf::from(name);
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(StoreError::Backend(format!(
                    "cannot remove {}: {e}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

/// Reads `cache_meta`, or `None` when a required key is absent or unparseable.
///
/// "Unparseable" is folded into "absent" on purpose: both mean the file cannot be trusted, and the
/// caller's answer to both is the same rebuild.
async fn read_meta(pool: &SqlitePool) -> Result<Option<CacheMeta>> {
    let rows = sqlx::query("SELECT key, value FROM cache_meta")
        .fetch_all(pool)
        .await
        .map_err(map_sqlx)?;

    let mut schema_version = None;
    let mut db_fingerprint = None;
    let mut built_at = None;
    let mut last_full_refresh_at = None;
    for row in rows {
        let key: String = row.try_get("key").map_err(map_sqlx)?;
        let value: String = row.try_get("value").map_err(map_sqlx)?;
        match key.as_str() {
            KEY_SCHEMA_VERSION => schema_version = value.parse::<i64>().ok(),
            KEY_DB_FINGERPRINT => db_fingerprint = Some(value),
            KEY_BUILT_AT => built_at = parse_rfc3339(&value),
            KEY_LAST_FULL_REFRESH_AT => last_full_refresh_at = parse_rfc3339(&value),
            _ => {}
        }
    }

    Ok(match (schema_version, db_fingerprint, built_at) {
        (Some(schema_version), Some(db_fingerprint), Some(built_at)) => Some(CacheMeta {
            schema_version,
            db_fingerprint,
            built_at,
            last_full_refresh_at,
        }),
        _ => None,
    })
}

/// Empties `cache_meta` and writes the three keys a freshly built file carries.
async fn write_fresh_meta(pool: &SqlitePool, schema_version: i64, fingerprint: &str) -> Result<()> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::query("DELETE FROM cache_meta")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    for (key, value) in [
        (KEY_SCHEMA_VERSION, schema_version.to_string()),
        (KEY_DB_FINGERPRINT, fingerprint.to_owned()),
        (KEY_BUILT_AT, Utc::now().to_rfc3339()),
    ] {
        sqlx::query("INSERT INTO cache_meta (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    tx.commit().await.map_err(map_sqlx)
}

/// Upserts one `cache_meta` key.
async fn put_meta(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO cache_meta (key, value) VALUES (?, ?) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// RFC-3339 text to an instant, or `None`.
fn parse_rfc3339(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}
