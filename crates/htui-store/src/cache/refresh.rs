//! The cursor pass that fills the mirror (`docs/ANA-9.md` §6.2, §6.3, blueprint C.13, plan D9).
//!
//! [`Refresher`] is the **only** writer of `cache.sqlite` (§6.3). It never runs on the UI task
//! (`R-NF-3`) and never holds a SQLite transaction across a Postgres await: each `(project, table)`
//! batch fetches from the server first and only then opens its transaction, so a crash mid-pass
//! leaves whole tables consistent and the cursor un-advanced.
//!
//! [`run_pass`] is a free function of `(pool, cache, projects, settings)` so the tests drive one
//! pass without the spawned loop.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{BoxId, ProjectId, UserId};
use htui_core::store::{Result, StoreError};
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row as _, Sqlite, SqlitePool, Transaction};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use super::CacheStore;
use super::read::ts_bind;
use crate::cache::pending::upload_pending;
use crate::error::map_sqlx;

/// `app_setting.cache_refresh_seconds`, seeded by [`crate::PgStore::seed_if_empty`].
const DEFAULT_INTERVAL_SECONDS: u64 = 30;
/// `app_setting.cache_overlap_seconds`, seeded by [`crate::PgStore::seed_if_empty`].
const DEFAULT_OVERLAP_SECONDS: u64 = 300;
/// `project.settings.cached_transcript_steps` (§4.4).
const DEFAULT_TRANSCRIPT_STEPS: i64 = 20;
/// The `project.settings` key that overrides [`RefreshSettings::transcript_steps`] per project.
const TRANSCRIPT_STEPS_KEY: &str = "cached_transcript_steps";

/// Cursor-pass tuning, read from `app_setting` at connect (§4.4, §6.2).
///
/// `this_box` and `this_user` are not tuning; they are here because §6.2 step 2 mirrors the *own*
/// `box` row and step 5 uploads the offline chat buffer under this box and this user, and because
/// both [`Refresher::spawn`] and [`run_pass`] would otherwise need two more arguments each
/// (deviation from blueprint C.13, same components).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshSettings {
    /// `app_setting.cache_refresh_seconds`, default 30 s.
    pub interval: Duration,
    /// `app_setting.cache_overlap_seconds`, default 300 s: the visibility window of §4.4.
    pub overlap: Duration,
    /// `project.settings.cached_transcript_steps`, default 20; the per-project value wins.
    pub transcript_steps: i64,
    /// [`crate::PgStore::this_box`]: whose `box` row is mirrored and who owns an uploaded run.
    pub this_box: BoxId,
    /// [`crate::PgStore::this_user`]: `run.started_by` of an uploaded offline chat.
    pub this_user: UserId,
}

impl Default for RefreshSettings {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECONDS),
            overlap: Duration::from_secs(DEFAULT_OVERLAP_SECONDS),
            transcript_steps: DEFAULT_TRANSCRIPT_STEPS,
            this_box: BoxId::default(),
            this_user: UserId::default(),
        }
    }
}

/// What one pass did; logged at `info` and asserted on in `tests/cache.rs`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    /// Rows upserted, per mirrored table, in pass order.
    pub tables: Vec<(&'static str, u64)>,
    /// `item_link` tombstones removed from the mirror.
    pub tombstones: u64,
    /// `session_event` rows copied.
    pub events: u64,
    /// `session_event` rows trimmed for steps outside the last N.
    pub trimmed: u64,
    /// `pending/*.jsonl` files uploaded.
    pub uploaded: usize,
}

impl PassReport {
    /// Rows upserted into one table, or `0` when the pass did not touch it.
    #[must_use]
    pub fn rows(&self, table: &str) -> u64 {
        self.tables
            .iter()
            .find(|(name, _)| *name == table)
            .map_or(0, |(_, rows)| *rows)
    }

    /// Adds a per-table count, keeping the first-seen (pass) order.
    fn add(&mut self, table: &'static str, rows: u64) {
        if let Some(entry) = self.tables.iter_mut().find(|(name, _)| *name == table) {
            entry.1 += rows;
        } else {
            self.tables.push((table, rows));
        }
    }
}

/// The one task that writes `cache.sqlite` (§6.3).
#[derive(Debug)]
pub struct Refresher {
    handle: JoinHandle<()>,
    wake: Arc<Notify>,
    /// Outcome of the last pass, published for [`Refresher::health`].
    ///
    /// Held here rather than only in the task so the receivers of a live `Refresher` never see a
    /// closed channel; the task holds a clone of the same `Arc`.
    health: Arc<watch::Sender<Option<StoreError>>>,
}

impl Refresher {
    /// Spawns the task: a first pass immediately, then one every `settings.interval`.
    ///
    /// `projects` is the scope the store worker updates on every `Items` request; the task reads
    /// `*projects.borrow()` at the top of each pass, so a scope change is picked up on the next one
    /// without a restart. A failed pass is a `warn!` and the loop continues, because a transient
    /// Postgres error must not kill the mirror. Nothing here ever touches the UI task (`R-NF-3`).
    #[must_use]
    pub fn spawn(
        pool: PgPool,
        cache: CacheStore,
        projects: watch::Receiver<Vec<ProjectId>>,
        settings: RefreshSettings,
    ) -> Self {
        let wake = Arc::new(Notify::new());
        let woken = Arc::clone(&wake);
        let health = Arc::new(watch::Sender::new(None));
        let reported = Arc::clone(&health);
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(settings.interval);
            // A pass that overran its slot must not then run twice back to back.
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    () = woken.notified() => {}
                }
                // Cloned out of the guard on its own line: a `watch::Ref` must never be alive
                // across the `.await` below.
                let scope = projects.borrow().clone();
                let outcome = match run_pass(&pool, &cache, &scope, &settings).await {
                    Ok(report) => {
                        tracing::info!(?report, "cache: refresh pass");
                        None
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cache: refresh pass failed");
                        Some(error)
                    }
                };
                // `send_replace`, so every pass marks the watch changed even when two in a row
                // report the same thing: the store worker wants each outcome, not each change.
                reported.send_replace(outcome);
            }
        });
        Self {
            handle,
            wake,
            health,
        }
    }

    /// The task handle, for tests and for shutdown.
    #[must_use]
    pub const fn handle(&self) -> &JoinHandle<()> {
        &self.handle
    }

    /// The outcome of the last pass: `None` while they succeed, `Some(err)` after one failed.
    ///
    /// The store worker watches this for [`StoreError::Unreachable`], which is its `Online` →
    /// `Offline` signal - the refresher runs every `interval` and is therefore what notices a
    /// server that went away between two UI reads. [`Refresher::handle`]'s `is_finished` cannot
    /// serve instead: a failed pass is a `warn!` and this loop deliberately keeps going, so the
    /// task is still very much alive.
    ///
    /// The receiver stays open for as long as the `Refresher` does, whatever the task is doing.
    #[must_use]
    pub fn health(&self) -> watch::Receiver<Option<StoreError>> {
        self.health.subscribe()
    }

    /// Stops the task. Called when the backend leaves `Online`.
    pub fn abort(&self) {
        self.handle.abort();
    }

    /// Forces a pass now instead of at the next tick.
    pub fn trigger(&self) {
        self.wake.notify_one();
    }
}

/// The ten cursor-driven tables of §6.2, in foreign-key order.
const PROJECT: &str = "project";
const REPO: &str = "repo";
const ITEM_KIND: &str = "item_kind";
const ITEM: &str = "item";
const ITEM_LINK: &str = "item_link";
const ITEM_NOTE: &str = "item_note";
const DOCUMENT: &str = "document";
const RUN: &str = "run";
const RUN_STEP: &str = "run_step";
const RUN_STEP_COMMIT: &str = "run_step_commit";

/// One pass, as a free function so `tests/cache.rs` can run it without a task.
///
/// In order (§6.2 verbatim, with the two gaps of blueprint H.3 and H.4 closed):
///
/// 1. unscoped full replace of `app_user`, `workspace` and `workspace_project` - small tables, and
///    `workspace_project` has no timestamp at all to put a cursor on (H.4);
/// 2. the own `box` row, unscoped, upserted on its primary key;
/// 3. per project, per table in foreign-key order, `ts_col > high_water - overlap`, primary-key
///    upsert, cursor set to `max(ts_col)` of the fetched rows - server time, so clock skew between
///    boxes is irrelevant. `run_step_commit` has no timestamp of its own and rides its parent
///    step's `updated_at` (H.3). An `item_link` row with `deleted_at` set is *deleted* from the
///    mirror instead of upserted and still advances the cursor (§4.4);
/// 4. the last-N **finished** transcripts per project, copied for steps not already mirrored, then
///    trimmed scoped to that project so another project's cached steps survive;
/// 5. the `pending/*.jsonl` upload, once, after the tables (§6.2's last line, §4.3);
/// 6. `cache_meta.last_full_refresh_at`, set only when every cursor was `0` at the top of the pass.
///
/// # Errors
///
/// Whatever either driver reports, through [`map_sqlx`]. A pass that fails part-way leaves the
/// batches it already committed in place; every one of them is idempotent, so the next pass simply
/// redoes the rest.
pub async fn run_pass(
    pool: &PgPool,
    cache: &CacheStore,
    projects: &[ProjectId],
    settings: &RefreshSettings,
) -> Result<PassReport> {
    let mut report = PassReport::default();
    let sqlite = cache.pool();

    replace_app_user(pool, sqlite, &mut report).await?;
    replace_workspace(pool, sqlite, &mut report).await?;
    replace_workspace_project(pool, sqlite, &mut report).await?;
    refresh_box(pool, sqlite, settings.this_box, &mut report).await?;

    // A full pull is one where nothing had been fetched before; an empty scope is not one.
    let mut full = !projects.is_empty();

    for &project in projects {
        for table in [
            PROJECT,
            REPO,
            ITEM_KIND,
            ITEM,
            ITEM_LINK,
            ITEM_NOTE,
            DOCUMENT,
            RUN,
            RUN_STEP,
            RUN_STEP_COMMIT,
        ] {
            let hw = cursor(sqlite, project, table).await?;
            if hw != 0 {
                full = false;
            }
            let since = since(hw, settings.overlap);
            let outcome = match table {
                PROJECT => refresh_project(pool, sqlite, project, since, hw).await?,
                REPO => refresh_repo(pool, sqlite, project, since, hw).await?,
                ITEM_KIND => refresh_item_kind(pool, sqlite, project, since, hw).await?,
                ITEM => refresh_item(pool, sqlite, project, since, hw).await?,
                ITEM_LINK => refresh_item_link(pool, sqlite, project, since, hw).await?,
                ITEM_NOTE => refresh_item_note(pool, sqlite, project, since, hw).await?,
                DOCUMENT => refresh_document(pool, sqlite, project, since, hw).await?,
                RUN => refresh_run(pool, sqlite, project, since, hw).await?,
                RUN_STEP => refresh_run_step(pool, sqlite, project, since, hw).await?,
                _ => refresh_run_step_commit(pool, sqlite, project, since, hw).await?,
            };
            tracing::debug!(
                project = %project,
                table,
                rows = outcome.rows,
                tombstones = outcome.tombstones,
                "cache: table batch"
            );
            report.add(table, outcome.rows);
            report.tombstones += outcome.tombstones;
        }

        let transcripts = refresh_transcripts(pool, sqlite, project, settings).await?;
        report.events += transcripts.copied;
        report.trimmed += transcripts.trimmed;
    }

    report.uploaded =
        upload_pending(pool, cache.dir(), settings.this_box, settings.this_user).await?;

    if full {
        cache.note_full_refresh(Utc::now()).await?;
    }
    Ok(report)
}

/// What one `(project, table)` batch did.
struct Batch {
    /// Rows upserted.
    rows: u64,
    /// Tombstoned rows deleted from the mirror (`item_link` only).
    tombstones: u64,
}

/// The fetch window of §4.4: the high-water mark pulled back by the overlap.
fn since(high_water: i64, overlap: Duration) -> DateTime<Utc> {
    let back = i64::try_from(overlap.as_micros()).unwrap_or(i64::MAX);
    DateTime::from_timestamp_micros(high_water.saturating_sub(back))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// `cache_cursor[project, table]`, or `0` when the pair has never been fetched.
async fn cursor(sqlite: &SqlitePool, project: ProjectId, table: &str) -> Result<i64> {
    let row =
        sqlx::query("SELECT high_water FROM cache_cursor WHERE project_id = ? AND table_name = ?")
            .bind(project.to_string())
            .bind(table)
            .fetch_optional(sqlite)
            .await
            .map_err(map_sqlx)?;
    row.map_or(Ok(0), |row| row.try_get("high_water").map_err(map_sqlx))
}

/// Writes the cursor and commits the batch, so a crash cannot advance it past what landed.
async fn finish(
    mut tx: Transaction<'_, Sqlite>,
    project: ProjectId,
    table: &str,
    high_water: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cache_cursor (project_id, table_name, high_water) VALUES (?, ?, ?) \
         ON CONFLICT (project_id, table_name) DO UPDATE SET high_water = excluded.high_water",
    )
    .bind(project.to_string())
    .bind(table)
    .bind(high_water)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    tx.commit().await.map_err(map_sqlx)
}

/// `INSERT ... VALUES (?, ...) ON CONFLICT (<pk>) DO UPDATE SET <every non-pk column>`.
///
/// Built once per batch and shared by every row of it; `columns` and `table` are module constants,
/// never caller data, which is what [`AssertSqlSafe`] asserts at the call sites.
fn upsert_sql(table: &str, columns: &[&str], pk: usize) -> Arc<str> {
    let values = vec!["?"; columns.len()].join(", ");
    let mut sql = format!(
        "INSERT INTO {table} ({}) VALUES ({values})",
        columns.join(", ")
    );
    sql.push_str(&format!(" ON CONFLICT ({})", columns[..pk].join(", ")));
    if pk == columns.len() {
        sql.push_str(" DO NOTHING");
    } else {
        let sets: Vec<String> = columns[pk..]
            .iter()
            .map(|c| format!("{c} = excluded.{c}"))
            .collect();
        sql.push_str(&format!(" DO UPDATE SET {}", sets.join(", ")));
    }
    sql.into()
}

/// A `JSONB` value as the mirror's TEXT.
fn json_text(value: &Value) -> Result<String> {
    serde_json::to_string(value)
        .map_err(|e| StoreError::Backend(format!("cache: cannot serialise a JSON column: {e}")))
}

/// A nullable `JSONB` value as the mirror's TEXT.
fn opt_json_text(value: Option<&Value>) -> Result<Option<String>> {
    value.map(json_text).transpose()
}

/// A `TEXT[]` value as the mirror's JSON array.
fn strings_text(values: &[String]) -> Result<String> {
    serde_json::to_string(values)
        .map_err(|e| StoreError::Backend(format!("cache: cannot serialise a text[] column: {e}")))
}

// ------------------------------------------------------------------------------------------------
// 1. Unscoped, full replace (blueprint H.4)
// ------------------------------------------------------------------------------------------------

const APP_USER_COLUMNS: &[&str] = &["id", "name", "email", "created_at", "updated_at"];

async fn replace_app_user(
    pool: &PgPool,
    sqlite: &SqlitePool,
    report: &mut PassReport,
) -> Result<()> {
    let rows = sqlx::query!("SELECT id, name, email, created_at, updated_at FROM app_user")
        .fetch_all(pool)
        .await
        .map_err(map_sqlx)?;

    let sql = upsert_sql("app_user", APP_USER_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    sqlx::query("DELETE FROM app_user")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(&row.name)
            .bind(row.email.as_deref())
            .bind(ts_bind(row.created_at))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    tx.commit().await.map_err(map_sqlx)?;
    report.add("app_user", rows.len() as u64);
    Ok(())
}

const WORKSPACE_COLUMNS: &[&str] = &[
    "id",
    "slug",
    "name",
    "description",
    "created_by",
    "created_at",
    "updated_at",
];

async fn replace_workspace(
    pool: &PgPool,
    sqlite: &SqlitePool,
    report: &mut PassReport,
) -> Result<()> {
    let rows = sqlx::query!(
        "SELECT id, slug, name, description, created_by, created_at, updated_at FROM workspace"
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql("workspace", WORKSPACE_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    sqlx::query("DELETE FROM workspace")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(&row.slug)
            .bind(&row.name)
            .bind(&row.description)
            .bind(row.created_by.to_string())
            .bind(ts_bind(row.created_at))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    tx.commit().await.map_err(map_sqlx)?;
    report.add("workspace", rows.len() as u64);
    Ok(())
}

const WORKSPACE_PROJECT_COLUMNS: &[&str] = &["workspace_id", "project_id", "position"];

async fn replace_workspace_project(
    pool: &PgPool,
    sqlite: &SqlitePool,
    report: &mut PassReport,
) -> Result<()> {
    let rows = sqlx::query!("SELECT workspace_id, project_id, position FROM workspace_project")
        .fetch_all(pool)
        .await
        .map_err(map_sqlx)?;

    let sql = upsert_sql("workspace_project", WORKSPACE_PROJECT_COLUMNS, 2);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    sqlx::query("DELETE FROM workspace_project")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.workspace_id.to_string())
            .bind(row.project_id.to_string())
            .bind(i64::from(row.position))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    tx.commit().await.map_err(map_sqlx)?;
    report.add("workspace_project", rows.len() as u64);
    Ok(())
}

// ------------------------------------------------------------------------------------------------
// 2. The own `box` row, unscoped
// ------------------------------------------------------------------------------------------------

const BOX_COLUMNS: &[&str] = &[
    "id",
    "user_id",
    "hostname",
    "os_family",
    "os_version",
    "arch",
    "cpu",
    "ram_mb",
    "gpu_present",
    "gpu_vendor",
    "htui_version",
    "probed_tags",
    "declared_tags",
    "quirks",
    "settings",
    "registered_at",
    "last_seen_at",
    "last_probed_at",
    "updated_at",
];

async fn refresh_box(
    pool: &PgPool,
    sqlite: &SqlitePool,
    this_box: BoxId,
    report: &mut PassReport,
) -> Result<()> {
    let rows = sqlx::query!(
        "SELECT id, user_id, hostname, os_family, os_version, arch, cpu, ram_mb, gpu_present, \
                gpu_vendor, htui_version, probed_tags, declared_tags, quirks, settings, \
                registered_at, last_seen_at, last_probed_at, updated_at \
           FROM box WHERE id = $1",
        this_box.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql("box", BOX_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.user_id.to_string())
            .bind(&row.hostname)
            .bind(&row.os_family)
            .bind(&row.os_version)
            .bind(&row.arch)
            .bind(&row.cpu)
            .bind(row.ram_mb.map(i64::from))
            .bind(i64::from(row.gpu_present))
            .bind(row.gpu_vendor.as_deref())
            .bind(&row.htui_version)
            .bind(strings_text(&row.probed_tags)?)
            .bind(strings_text(&row.declared_tags)?)
            .bind(&row.quirks)
            .bind(json_text(&row.settings)?)
            .bind(ts_bind(row.registered_at))
            .bind(ts_bind(row.last_seen_at))
            .bind(row.last_probed_at.map(ts_bind))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    tx.commit().await.map_err(map_sqlx)?;
    report.add("box", rows.len() as u64);
    Ok(())
}

// ------------------------------------------------------------------------------------------------
// 3. The ten cursor-driven tables, per project
// ------------------------------------------------------------------------------------------------

const PROJECT_COLUMNS: &[&str] = &[
    "id",
    "slug",
    "name",
    "description",
    "secret_provider",
    "secret_scope",
    "settings",
    "created_by",
    "created_at",
    "updated_at",
];

async fn refresh_project(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    let rows = sqlx::query!(
        "SELECT id, slug, name, description, secret_provider, secret_scope, settings, created_by, \
                created_at, updated_at \
           FROM project WHERE id = $1 AND updated_at > $2 ORDER BY updated_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(PROJECT, PROJECT_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(&row.slug)
            .bind(&row.name)
            .bind(&row.description)
            .bind(row.secret_provider.as_deref())
            .bind(row.secret_scope.as_deref())
            .bind(json_text(&row.settings)?)
            .bind(row.created_by.to_string())
            .bind(ts_bind(row.created_at))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
    finish(tx, project, PROJECT, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const REPO_COLUMNS: &[&str] = &[
    "id",
    "project_id",
    "name",
    "remote_url",
    "default_branch",
    "is_primary",
    "created_at",
    "updated_at",
];

async fn refresh_repo(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    let rows = sqlx::query!(
        "SELECT id, project_id, name, remote_url, default_branch, is_primary, created_at, \
                updated_at \
           FROM repo WHERE project_id = $1 AND updated_at > $2 ORDER BY updated_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(REPO, REPO_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.project_id.to_string())
            .bind(&row.name)
            .bind(row.remote_url.as_deref())
            .bind(&row.default_branch)
            .bind(i64::from(row.is_primary))
            .bind(ts_bind(row.created_at))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
    finish(tx, project, REPO, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const ITEM_KIND_COLUMNS: &[&str] = &[
    "id",
    "project_id",
    "prefix",
    "name",
    "description",
    "default_graph_id",
    "position",
    "updated_at",
];

async fn refresh_item_kind(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    let rows = sqlx::query!(
        "SELECT id, project_id, prefix, name, description, default_graph_id, position, updated_at \
           FROM item_kind WHERE project_id = $1 AND updated_at > $2 ORDER BY updated_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(ITEM_KIND, ITEM_KIND_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.project_id.to_string())
            .bind(&row.prefix)
            .bind(&row.name)
            .bind(&row.description)
            .bind(row.default_graph_id.to_string())
            .bind(i64::from(row.position))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
    finish(tx, project, ITEM_KIND, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const ITEM_COLUMNS: &[&str] = &[
    "id",
    "project_id",
    "kind_id",
    "key_prefix",
    "key_number",
    "key",
    "title",
    "body",
    "status",
    "priority",
    "required_tags",
    "touched_paths",
    "step_graph_id",
    "version",
    "created_by",
    "created_at",
    "updated_at",
    "closed_at",
];

async fn refresh_item(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    let rows = sqlx::query!(
        r#"SELECT id, project_id, kind_id, key_prefix, key_number, key as "key!", title, body,
                  status, priority, required_tags, touched_paths, step_graph_id, version,
                  created_by, created_at, updated_at, closed_at
             FROM item WHERE project_id = $1 AND updated_at > $2 ORDER BY updated_at"#,
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(ITEM, ITEM_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.project_id.to_string())
            .bind(row.kind_id.to_string())
            .bind(&row.key_prefix)
            .bind(i64::from(row.key_number))
            .bind(&row.key)
            .bind(&row.title)
            .bind(&row.body)
            .bind(&row.status)
            .bind(i64::from(row.priority))
            .bind(strings_text(&row.required_tags)?)
            .bind(strings_text(&row.touched_paths)?)
            .bind(row.step_graph_id.map(|id| id.to_string()))
            .bind(i64::from(row.version))
            .bind(row.created_by.to_string())
            .bind(ts_bind(row.created_at))
            .bind(ts_bind(row.updated_at))
            .bind(row.closed_at.map(ts_bind))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
    finish(tx, project, ITEM, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const ITEM_LINK_COLUMNS: &[&str] = &[
    "from_item_id",
    "to_item_id",
    "kind",
    "proposed_by_step_id",
    "created_at",
    "updated_at",
];

async fn refresh_item_link(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    let rows = sqlx::query!(
        "SELECT from_item_id, to_item_id, kind, proposed_by_step_id, created_at, updated_at, \
                deleted_at \
           FROM item_link \
          WHERE (from_item_id IN (SELECT id FROM item WHERE project_id = $1) \
                 OR to_item_id IN (SELECT id FROM item WHERE project_id = $1)) \
            AND updated_at > $2 \
          ORDER BY updated_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(ITEM_LINK, ITEM_LINK_COLUMNS, 3);
    let mut upserted = 0u64;
    let mut tombstones = 0u64;
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        if row.deleted_at.is_some() {
            // §4.4: "removals are tombstones and ride the same cursor; the mirror drops the row".
            sqlx::query(
                "DELETE FROM item_link WHERE from_item_id = ? AND to_item_id = ? AND kind = ?",
            )
            .bind(row.from_item_id.to_string())
            .bind(row.to_item_id.to_string())
            .bind(&row.kind)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            tombstones += 1;
            continue;
        }
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.from_item_id.to_string())
            .bind(row.to_item_id.to_string())
            .bind(&row.kind)
            .bind(row.proposed_by_step_id.map(|id| id.to_string()))
            .bind(ts_bind(row.created_at))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        upserted += 1;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
    finish(tx, project, ITEM_LINK, high_water).await?;
    Ok(Batch {
        rows: upserted,
        tombstones,
    })
}

const ITEM_NOTE_COLUMNS: &[&str] = &[
    "id",
    "item_id",
    "body",
    "created_by",
    "box_id",
    "via_step_id",
    "created_at",
];

async fn refresh_item_note(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    // Append-only, so the cursor is `created_at` rather than `updated_at` (§6.2).
    let rows = sqlx::query!(
        "SELECT id, item_id, body, created_by, box_id, via_step_id, created_at \
           FROM item_note \
          WHERE item_id IN (SELECT id FROM item WHERE project_id = $1) AND created_at > $2 \
          ORDER BY created_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(ITEM_NOTE, ITEM_NOTE_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.item_id.to_string())
            .bind(&row.body)
            .bind(row.created_by.to_string())
            .bind(row.box_id.map(|id| id.to_string()))
            .bind(row.via_step_id.map(|id| id.to_string()))
            .bind(ts_bind(row.created_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.created_at));
    finish(tx, project, ITEM_NOTE, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const DOCUMENT_COLUMNS: &[&str] = &[
    "id",
    "item_id",
    "kind",
    "version",
    "title",
    "body",
    "produced_by_step_id",
    "created_by",
    "created_at",
];

async fn refresh_document(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    // Append-only, so the cursor is `created_at` rather than `updated_at` (§6.2).
    let rows = sqlx::query!(
        "SELECT id, item_id, kind, version, title, body, produced_by_step_id, created_by, \
                created_at \
           FROM document \
          WHERE item_id IN (SELECT id FROM item WHERE project_id = $1) AND created_at > $2 \
          ORDER BY created_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(DOCUMENT, DOCUMENT_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.item_id.to_string())
            .bind(&row.kind)
            .bind(i64::from(row.version))
            .bind(&row.title)
            .bind(&row.body)
            .bind(row.produced_by_step_id.map(|id| id.to_string()))
            .bind(row.created_by.to_string())
            .bind(ts_bind(row.created_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.created_at));
    finish(tx, project, DOCUMENT, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const RUN_COLUMNS: &[&str] = &[
    "id",
    "project_id",
    "item_id",
    "kind",
    "mode",
    "status",
    "target_box_id",
    "executing_box_id",
    "graph_snapshot",
    "started_by",
    "queued_at",
    "started_at",
    "finished_at",
    "failure",
    "updated_at",
];

async fn refresh_run(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    let rows = sqlx::query!(
        "SELECT id, project_id, item_id, kind, mode, status, target_box_id, executing_box_id, \
                graph_snapshot, started_by, queued_at, started_at, finished_at, failure, \
                updated_at \
           FROM run WHERE project_id = $1 AND updated_at > $2 ORDER BY updated_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(RUN, RUN_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.project_id.to_string())
            .bind(row.item_id.map(|id| id.to_string()))
            .bind(&row.kind)
            .bind(&row.mode)
            .bind(&row.status)
            .bind(row.target_box_id.to_string())
            .bind(row.executing_box_id.map(|id| id.to_string()))
            .bind(opt_json_text(row.graph_snapshot.as_ref())?)
            .bind(row.started_by.to_string())
            .bind(ts_bind(row.queued_at))
            .bind(row.started_at.map(ts_bind))
            .bind(row.finished_at.map(ts_bind))
            .bind(row.failure.as_deref())
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
    finish(tx, project, RUN, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const RUN_STEP_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "position",
    "attempt",
    "fanout_index",
    "phase_name",
    "agent_id",
    "model",
    "status",
    "gate_outcome",
    "gate_note",
    "selected",
    "exit_code",
    "prompt_digest",
    "trim_record",
    "usage",
    "isolation_path",
    "started_at",
    "finished_at",
    "updated_at",
];

async fn refresh_run_step(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    let rows = sqlx::query!(
        "SELECT id, run_id, position, attempt, fanout_index, phase_name, agent_id, model, status, \
                gate_outcome, gate_note, selected, exit_code, prompt_digest, trim_record, usage, \
                isolation_path, started_at, finished_at, updated_at \
           FROM run_step \
          WHERE run_id IN (SELECT id FROM run WHERE project_id = $1) AND updated_at > $2 \
          ORDER BY updated_at",
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(RUN_STEP, RUN_STEP_COLUMNS, 1);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.id.to_string())
            .bind(row.run_id.to_string())
            .bind(i64::from(row.position))
            .bind(i64::from(row.attempt))
            .bind(i64::from(row.fanout_index))
            .bind(&row.phase_name)
            .bind(row.agent_id.map(|id| id.to_string()))
            .bind(row.model.as_deref())
            .bind(&row.status)
            .bind(row.gate_outcome.as_deref())
            .bind(row.gate_note.as_deref())
            .bind(row.selected.map(i64::from))
            .bind(row.exit_code.map(i64::from))
            .bind(row.prompt_digest.as_deref())
            .bind(opt_json_text(row.trim_record.as_ref())?)
            .bind(opt_json_text(row.usage.as_ref())?)
            .bind(row.isolation_path.as_deref())
            .bind(row.started_at.map(ts_bind))
            .bind(row.finished_at.map(ts_bind))
            .bind(ts_bind(row.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
    finish(tx, project, RUN_STEP, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

const RUN_STEP_COMMIT_COLUMNS: &[&str] = &["run_step_id", "repo_id", "before_hash", "after_hash"];

async fn refresh_run_step_commit(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    since: DateTime<Utc>,
    hw: i64,
) -> Result<Batch> {
    // `run_step_commit` has no timestamp of its own, so it rides its parent step's `updated_at`
    // (blueprint H.3): a commit row is only ever written while its step is being updated.
    let rows = sqlx::query!(
        r#"SELECT c.run_step_id, c.repo_id, c.before_hash, c.after_hash, s.updated_at as "ts!"
             FROM run_step_commit c
             JOIN run_step s ON s.id = c.run_step_id
             JOIN run r      ON r.id = s.run_id
            WHERE r.project_id = $1 AND s.updated_at > $2
            ORDER BY s.updated_at"#,
        project.as_uuid(),
        since,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let sql = upsert_sql(RUN_STEP_COMMIT, RUN_STEP_COMMIT_COLUMNS, 2);
    let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
    for row in &rows {
        sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
            .bind(row.run_step_id.to_string())
            .bind(row.repo_id.to_string())
            .bind(&row.before_hash)
            .bind(row.after_hash.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    }
    let high_water = rows.last().map_or(hw, |row| ts_bind(row.ts));
    finish(tx, project, RUN_STEP_COMMIT, high_water).await?;
    Ok(Batch {
        rows: rows.len() as u64,
        tombstones: 0,
    })
}

// ------------------------------------------------------------------------------------------------
// 4. Last-N transcripts
// ------------------------------------------------------------------------------------------------

/// What the transcript half of one project's pass did.
struct Transcripts {
    /// `session_event` rows copied.
    copied: u64,
    /// `session_event` rows trimmed for steps outside the last N.
    trimmed: u64,
}

const SESSION_EVENT_COLUMNS: &[&str] = &[
    "run_step_id",
    "seq",
    "turn",
    "kind",
    "role",
    "tool_call_id",
    "payload",
    "raw",
    "at",
];

async fn refresh_transcripts(
    pool: &PgPool,
    sqlite: &SqlitePool,
    project: ProjectId,
    settings: &RefreshSettings,
) -> Result<Transcripts> {
    let limit = transcript_steps(sqlite, project, settings.transcript_steps).await?;

    // `finished_at IS NOT NULL`: §6.2 says "last N **finished** run_step ids", and the presence
    // probe below is existence-only. A running step copied mid-flight would be skipped by every
    // later pass and frozen at whatever prefix of its log the first one saw; it becomes a
    // candidate once it has finished and its log can no longer grow.
    let wanted: Vec<Uuid> = sqlx::query_scalar!(
        "SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
          WHERE r.project_id = $1 AND s.finished_at IS NOT NULL \
          ORDER BY s.finished_at DESC, s.updated_at DESC \
          LIMIT $2",
        project.as_uuid(),
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    // Only the steps whose events are not mirrored yet are copied; the rest are already whole,
    // `session_event` being append-only.
    let mut missing: Vec<Uuid> = Vec::new();
    for id in &wanted {
        let present: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM session_event WHERE run_step_id = ? LIMIT 1")
                .bind(id.to_string())
                .fetch_optional(sqlite)
                .await
                .map_err(map_sqlx)?;
        if present.is_none() {
            missing.push(*id);
        }
    }

    let mut copied = 0u64;
    if !missing.is_empty() {
        let rows = sqlx::query!(
            "SELECT run_step_id, seq, turn, kind, role, tool_call_id, payload, raw, at \
               FROM session_event WHERE run_step_id = ANY($1) ORDER BY run_step_id, seq",
            &missing[..],
        )
        .fetch_all(pool)
        .await
        .map_err(map_sqlx)?;

        let sql = upsert_sql("session_event", SESSION_EVENT_COLUMNS, 2);
        let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
        for row in &rows {
            sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
                .bind(row.run_step_id.to_string())
                .bind(i64::from(row.seq))
                .bind(i64::from(row.turn))
                .bind(&row.kind)
                .bind(&row.role)
                .bind(row.tool_call_id.as_deref())
                .bind(json_text(&row.payload)?)
                .bind(opt_json_text(row.raw.as_ref())?)
                .bind(ts_bind(row.at))
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        }
        tx.commit().await.map_err(map_sqlx)?;
        copied = rows.len() as u64;
    }

    // Trim, scoped to this project so another project's cached steps survive.
    let mut sql = String::from(
        "DELETE FROM session_event WHERE run_step_id IN \
         (SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id WHERE r.project_id = ?)",
    );
    if !wanted.is_empty() {
        sql.push_str(" AND run_step_id NOT IN (");
        sql.push_str(&vec!["?"; wanted.len()].join(", "));
        sql.push(')');
    }
    let mut query = sqlx::query(AssertSqlSafe(sql)).bind(project.to_string());
    for id in &wanted {
        query = query.bind(id.to_string());
    }
    let trimmed = query
        .execute(sqlite)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

    Ok(Transcripts { copied, trimmed })
}

/// `project.settings.cached_transcript_steps` as an integer, falling back to the pass default.
///
/// Read from the mirror rather than from the server: `project` was refreshed a few statements ago,
/// so the value is current and the pass is one Postgres round trip lighter.
async fn transcript_steps(sqlite: &SqlitePool, project: ProjectId, fallback: i64) -> Result<i64> {
    let settings: Option<String> = sqlx::query_scalar("SELECT settings FROM project WHERE id = ?")
        .bind(project.to_string())
        .fetch_optional(sqlite)
        .await
        .map_err(map_sqlx)?;
    let configured = settings
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get(TRANSCRIPT_STEPS_KEY).and_then(Value::as_i64));
    Ok(configured.filter(|n| *n >= 0).unwrap_or(fallback))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_OVERLAP_SECONDS, RefreshSettings, since, upsert_sql};
    use chrono::{DateTime, Utc};
    use std::time::Duration;

    #[test]
    fn the_upsert_updates_every_non_key_column() {
        assert_eq!(
            &*upsert_sql("t", &["a", "b", "c"], 1),
            "INSERT INTO t (a, b, c) VALUES (?, ?, ?) ON CONFLICT (a) \
             DO UPDATE SET b = excluded.b, c = excluded.c"
        );
    }

    #[test]
    fn an_all_key_table_upserts_to_nothing() {
        assert_eq!(
            &*upsert_sql("t", &["a", "b"], 2),
            "INSERT INTO t (a, b) VALUES (?, ?) ON CONFLICT (a, b) DO NOTHING"
        );
    }

    #[test]
    fn a_zero_cursor_reaches_back_before_the_epoch() {
        let overlap = Duration::from_secs(DEFAULT_OVERLAP_SECONDS);
        assert!(since(0, overlap) < DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"));
    }

    #[test]
    fn the_defaults_are_the_seeded_app_settings() {
        let settings = RefreshSettings::default();
        assert_eq!(settings.interval, Duration::from_secs(30));
        assert_eq!(settings.overlap, Duration::from_secs(300));
        assert_eq!(settings.transcript_steps, 20);
    }
}
