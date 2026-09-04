//! The Postgres store: connect, schema check, seed, box registration (ANA-9 §5.0, §5.10, plan D4-D6).
//!
//! `PgStore` is the only `WriteStore` in the product (ANA-9 §6.1); MOD-6 T2 adds the read and
//! write impls on top of what this module builds.

#[cfg(feature = "demo")]
mod demo;
mod read;
mod rows;
mod write;

use std::str::FromStr as _;
use std::time::Duration;

use htui_core::model::{BoxId, OsFamily, UserId};
use htui_core::store::{Result, StoreError};
use sqlx::migrate::Migrate as _;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

use crate::MIGRATOR;
use crate::error::{checksum_drift, map_migrate, map_sqlx, schema_is_newer};
use crate::identity::Identity;

/// The ten `capability_tag` rows ANA-9 §5.10 seeds.
const SEEDED_TAGS: [&str; 10] = [
    "gpu",
    "vulkan",
    "msvc",
    "mingw",
    "clang",
    "cmake",
    "vcpkg",
    "docker",
    "rust",
    "heavy_build",
];

/// The `app_setting` defaults ANA-9 §5.10 seeds (plan D5, D8, D9).
const SEEDED_SETTINGS: [(&str, i32); 2] = [
    ("cache_refresh_seconds", 30),
    ("cache_overlap_seconds", 300),
];

/// The Postgres store: the only `WriteStore` in the product (ANA-9 §6.1).
#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
    identity: Identity,
    this_box: BoxId,
    this_user: UserId,
}

/// What [`PgStore::connect`] found: a usable store plus the state of its schema.
#[derive(Debug, Clone)]
pub struct Connected {
    /// The store, usable for reads even while migrations are pending.
    pub store: PgStore,
    /// Whether the embedded set is fully applied.
    pub migrations: MigrationState,
}

/// The schema check of ANA-9 §5.0 / `R-STO-5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// Every embedded migration is applied and its checksum matches.
    UpToDate,
    /// This many embedded migrations are not applied yet.
    Pending(usize),
}

impl MigrationState {
    /// How many embedded migrations are waiting; `0` when the schema is up to date.
    #[must_use]
    pub const fn pending(self) -> usize {
        match self {
            Self::UpToDate => 0,
            Self::Pending(n) => n,
        }
    }
}

impl PgStore {
    /// Connects, checks the schema, seeds if empty and registers this box.
    ///
    /// Order, and it matters: open the pool, compare `_sqlx_migrations` against [`MIGRATOR`]
    /// (§5.0), and only then - when the state is [`MigrationState::UpToDate`] - seed and register.
    /// With [`MigrationState::Pending`] the tables may not exist yet, so both are deferred to
    /// [`PgStore::apply_migrations`], which finishes the same three steps.
    ///
    /// `identity` is supplied by the caller rather than read here: this function does no file I/O
    /// at all, so a test can hand it a throwaway box without touching the user's config directory.
    /// Production reads it once with
    /// [`identity::load_or_mint`](crate::identity::load_or_mint) over
    /// [`identity::config_root`](crate::identity::config_root).
    ///
    /// **Adopt-DB-id rule (plan D6)**: when this hostname already has a `box` row under a different
    /// id, the database's id wins and [`PgStore::this_box`] answers it, not `identity.box_id`. The
    /// caller persists that back by comparing the two and, when they differ, writing
    /// [`PgStore::identity`] through [`identity::store`](crate::identity::store) - `PgStore` never
    /// writes `box.toml` itself.
    ///
    /// # Errors
    ///
    /// [`StoreError::Backend`] with `schema is newer than this htui` when an applied version is not
    /// in the embedded set, with `was applied with a different checksum` on drift, and with
    /// `partially applied` on a dirty version. All three refuse rather than repair (plan D4,
    /// `R-STO-5`). Anything the driver reports comes back through
    /// [`map_sqlx`].
    pub async fn connect(dsn: &str, identity: &Identity) -> Result<Connected> {
        let options = PgConnectOptions::from_str(dsn).map_err(map_sqlx)?;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(map_sqlx)?;

        let migrations = schema_state(&pool).await?;
        let mut store = Self {
            pool,
            identity: identity.clone(),
            this_box: BoxId::default(),
            this_user: UserId::default(),
        };
        if migrations == MigrationState::UpToDate {
            store.bootstrap().await?;
        }
        Ok(Connected { store, migrations })
    }

    /// Runs the pending migrations, then seeds and registers as [`PgStore::connect`] would have.
    ///
    /// Takes `&mut self` because seeding and registering are what fill `this_user` and `this_box`,
    /// which a pending schema left empty; the blueprint's `&self` cannot express that.
    ///
    /// # Errors
    ///
    /// The same refusals as [`PgStore::connect`]: `Migrator::run` re-checks the applied set, so the
    /// two paths cannot disagree.
    pub async fn apply_migrations(&mut self) -> Result<()> {
        MIGRATOR.run(&self.pool).await.map_err(map_migrate)?;
        self.bootstrap().await
    }

    /// Seeds ANA-9 §5.10's global part if it is not there, and returns the single `app_user`.
    ///
    /// Idempotent: every insert is `ON CONFLICT DO NOTHING`. Seeds one `app_user` (name from
    /// `USERNAME` / `USER`, fallback `htui`), the ten `capability_tag` rows with `seeded = true`
    /// and the `app_setting` defaults `cache_refresh_seconds` = 30 and `cache_overlap_seconds` =
    /// 300. **No `agent` rows**: `agent.launch` is `JSONB NOT NULL` and its shape is ANA-4's, so
    /// seeding one would be a guess ANA-4 then has to migrate away (plan D5, blueprint H.2).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn seed_if_empty(&self) -> Result<UserId> {
        let name = seed_user_name();
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        sqlx::query!(
            "INSERT INTO app_user (id, name) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING",
            UserId::new().as_uuid(),
            name,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "INSERT INTO capability_tag (tag, seeded) SELECT t, true FROM UNNEST($1::text[]) AS t \
             ON CONFLICT (tag) DO NOTHING",
            &SEEDED_TAGS.map(str::to_owned)[..],
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        for (key, value) in SEEDED_SETTINGS {
            sqlx::query!(
                "INSERT INTO app_setting (key, value) VALUES ($1, to_jsonb($2::integer)) \
                 ON CONFLICT (key) DO NOTHING",
                key,
                value,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        let row = sqlx::query!(
            r#"SELECT id as "id: UserId" FROM app_user WHERE name = $1"#,
            name,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.id)
    }

    /// Upserts this box on `(user_id, hostname)` and bumps `last_seen_at`.
    ///
    /// Writes what `std::env::consts` and `gethostname` know: `os_family` from
    /// `std::env::consts::OS`, `arch` from `std::env::consts::ARCH`, `htui_version` from the crate
    /// version, `os_version` empty. The full probe (`box_tool`, tags, RAM, GPU) is MOD-7's and must
    /// not be attempted here.
    ///
    /// **Adopt-DB-id rule**: the returned id is the row's, which is `identity.box_id` on a first
    /// registration but the *existing* id when this hostname already has a row under a different
    /// one. [`PgStore::connect`] keeps it in [`PgStore::identity`]; writing it back to `box.toml`
    /// through [`identity::store`](crate::identity::store) is the caller's job, since only the
    /// caller knows the config root (plan D6).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn register_box(&self, identity: &Identity) -> Result<BoxId> {
        let row = sqlx::query!(
            r#"
            INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version)
            VALUES ($1, $2, $3, $4, '', $5, $6)
            ON CONFLICT (user_id, hostname) DO UPDATE
                SET last_seen_at = clock_timestamp(),
                    htui_version = EXCLUDED.htui_version,
                    os_family    = EXCLUDED.os_family,
                    arch         = EXCLUDED.arch
            RETURNING id as "id!: BoxId"
            "#,
            identity.box_id.as_uuid(),
            self.this_user.as_uuid(),
            identity.hostname,
            this_os_family().as_str(),
            std::env::consts::ARCH,
            env!("CARGO_PKG_VERSION"),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.id)
    }

    /// The pool, for the refresh task (`cache::refresh`) and for the tests.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The highest version in the embedded set.
    ///
    /// This is the number `cache_meta.schema_version` is compared against (plan D8).
    #[must_use]
    pub fn schema_version() -> i64 {
        MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
    }

    /// `box.id` of this box, as the database knows it after the adopt-DB-id rule.
    #[must_use]
    pub const fn this_box(&self) -> BoxId {
        self.this_box
    }

    /// The identity this store registered under, with `box_id` already adopted from the database.
    ///
    /// This is what a caller writes back to `box.toml` when it differs from what it passed to
    /// [`PgStore::connect`] (plan D6).
    #[must_use]
    pub const fn identity(&self) -> &Identity {
        &self.identity
    }

    /// `app_user.id` of the seeded user.
    #[must_use]
    pub const fn this_user(&self) -> UserId {
        self.this_user
    }

    /// Seeds, registers this box and adopts the database's id for it (plan D5, D6).
    ///
    /// No file I/O: persisting an adopted id back to `box.toml` is the caller's job, because only
    /// the caller knows which config root it read the identity from.
    async fn bootstrap(&mut self) -> Result<()> {
        self.this_user = self.seed_if_empty().await?;
        let registered = self.register_box(&self.identity).await?;
        self.identity.box_id = registered;
        self.this_box = registered;
        Ok(())
    }
}

/// Compares `_sqlx_migrations` against the embedded set (ANA-9 §5.0).
///
/// `list_applied_migrations` takes the table name in sqlx 0.9 and lives on `PgConnection`, not on
/// `PgPool`, so a connection is acquired first (blueprint H.7).
async fn schema_state(pool: &PgPool) -> Result<MigrationState> {
    let table = MIGRATOR.table_name.as_ref();
    let mut conn = pool.acquire().await.map_err(map_sqlx)?;

    conn.ensure_migrations_table(table)
        .await
        .map_err(map_migrate)?;
    if let Some(version) = conn.dirty_version(table).await.map_err(map_migrate)? {
        return Err(StoreError::Backend(format!(
            "migration {version} is partially applied; fix it and remove its `{table}` row"
        )));
    }
    let applied = conn
        .list_applied_migrations(table)
        .await
        .map_err(map_migrate)?;
    drop(conn);

    for entry in &applied {
        if !MIGRATOR.version_exists(entry.version) {
            return Err(StoreError::Backend(schema_is_newer(entry.version)));
        }
    }

    let mut pending = 0usize;
    for embedded in MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
    {
        match applied.iter().find(|a| a.version == embedded.version) {
            Some(entry) if entry.checksum != embedded.checksum => {
                return Err(StoreError::Backend(checksum_drift(embedded.version)));
            }
            Some(_) => {}
            None => pending += 1,
        }
    }

    if pending == 0 {
        Ok(MigrationState::UpToDate)
    } else {
        Ok(MigrationState::Pending(pending))
    }
}

/// `app_user.name` for the seeded user: `USERNAME`, then `USER`, then `htui` (ANA-9 §5.10).
fn seed_user_name() -> String {
    for key in ["USERNAME", "USER"] {
        if let Ok(value) = std::env::var(key)
            && !value.trim().is_empty()
        {
            return value;
        }
    }
    "htui".to_owned()
}

/// `std::env::consts::OS` mapped onto the three values `box.os_family` allows.
fn this_os_family() -> OsFamily {
    match OsFamily::from_str(std::env::consts::OS) {
        Ok(family) => family,
        Err(_) => {
            tracing::warn!(
                os = std::env::consts::OS,
                "unknown OS; recording box.os_family as `linux`"
            );
            OsFamily::Linux
        }
    }
}
