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

use chrono::Utc;
use htui_core::model::agent::seed_rows;
use htui_core::model::{BoxId, OsFamily, UserId};
use htui_core::store::{Result, StoreError};
use sqlx::migrate::Migrate as _;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

use crate::MIGRATOR;
use crate::error::{checksum_drift, map_migrate, map_sqlx, schema_is_newer};
use crate::identity::{Fingerprint, Identity};

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

/// How long [`PgStore::connect`] waits for its first connection.
///
/// [`PgStore::connect_with`] takes it as an argument so a test that dials a port nothing listens
/// on does not have to sit out the full ten seconds.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The `app_setting` defaults ANA-9 §5.10 seeds (plan D5, D8, D9).
const SEEDED_SETTINGS: [(&str, i32); 2] = [
    ("cache_refresh_seconds", 30),
    ("cache_overlap_seconds", 300),
];

/// The `htui` version this build records (plan D5): inserted at first registration, rewritten by
/// the probe writer, compared by `BoxRecord::needs_probe`.
pub const HTUI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What [`PgStore::register_box`] found (plan D2, D3, blueprint D19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// The id was new: one row inserted.
    New,
    /// The id was known and belongs to this machine; `renamed_from` is the old hostname on a rename.
    Known {
        /// The hostname the row carried before this registration, when it changed.
        renamed_from: Option<String>,
    },
    /// `box.toml` was carried from another machine or another user: `previous` is left untouched
    /// and this machine registered as `minted`.
    Copied {
        /// The row the copied `box.toml` named.
        previous: BoxId,
        /// The id this machine now has.
        minted: BoxId,
    },
}

impl Registration {
    /// The id this box has after registering `presented`.
    #[must_use]
    pub const fn box_id(&self, presented: BoxId) -> BoxId {
        let _ = presented;
        todo!()
    }
}

/// The Postgres store: the only `WriteStore` in the product (ANA-9 §6.1).
#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
    identity: Identity,
    this_box: BoxId,
    this_user: UserId,
    registration: Option<Registration>,
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
        Self::connect_with(dsn, identity, CONNECT_TIMEOUT).await
    }

    /// [`PgStore::connect`] with a caller-chosen acquire timeout.
    ///
    /// Only the wait differs. A test that dials a port nothing listens on gets its refusal from
    /// the OS immediately on some platforms and waits out the whole timeout on others (a dropped
    /// SYN, a firewall), so the knob is what keeps such a case fast everywhere rather than an
    /// assumption about the loopback.
    ///
    /// # Errors
    ///
    /// The same as [`PgStore::connect`].
    pub async fn connect_with(
        dsn: &str,
        identity: &Identity,
        connect_timeout: Duration,
    ) -> Result<Connected> {
        let options = PgConnectOptions::from_str(dsn).map_err(map_sqlx)?;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(connect_timeout)
            .connect_with(options)
            .await
            .map_err(map_sqlx)?;

        let migrations = schema_state(&pool).await?;
        let mut store = Self {
            pool,
            identity: identity.clone(),
            this_box: BoxId::default(),
            this_user: UserId::default(),
            registration: None,
        };
        if migrations == MigrationState::UpToDate {
            store.bootstrap().await?;
        }
        Ok(Connected { store, migrations })
    }

    /// A store over a pool that has **never** connected, for tests that need an `Online`
    /// [`Backend`](crate::Backend) without a server (feature `demo`).
    ///
    /// `connect_lazy_with` opens no socket: the first query is what dials, so against a DSN
    /// nothing listens on every read fails with [`StoreError::Unreachable`]. That is how the
    /// `htui` store worker's swap test injects a mid-session drop - restarting Postgres under a
    /// live pool is not something a unit test can do. `this_box` and `this_user` are nil: nothing
    /// seeded them, and nothing that uses this store may write.
    ///
    /// `acquire_timeout` is how long one failing read waits before it gives up.
    ///
    /// # Errors
    ///
    /// [`StoreError::Backend`] when `dsn` is not a Postgres connection string. Never a connection
    /// error: there is no connection.
    #[cfg(feature = "demo")]
    pub fn lazy(dsn: &str, identity: &Identity, acquire_timeout: Duration) -> Result<Self> {
        let options = PgConnectOptions::from_str(dsn).map_err(map_sqlx)?;
        Ok(Self {
            pool: PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(acquire_timeout)
                .connect_lazy_with(options),
            identity: identity.clone(),
            this_box: BoxId::default(),
            this_user: UserId::default(),
            registration: None,
        })
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
    /// [`seed_if_empty_as`](PgStore::seed_if_empty_as) with
    /// [`identity::os_user_name`](crate::identity::os_user_name) - the one definition
    /// [`CacheStore::this_user`](crate::CacheStore::this_user) reads back offline (MOD-2 D33).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn seed_if_empty(&self) -> Result<UserId> {
        self.seed_if_empty_as(&crate::identity::os_user_name())
            .await
    }

    /// [`seed_if_empty`](PgStore::seed_if_empty) under a caller-supplied `app_user.name`.
    ///
    /// Idempotent: every insert is conditional. Seeds one `app_user`, the ten `capability_tag`
    /// rows with `seeded = true`, the `app_setting` defaults `cache_refresh_seconds` = 30 and
    /// `cache_overlap_seconds` = 300, and the `agent` rows of `docs/ANA-4.md` §5.3 as MOD-2's plan
    /// amends them.
    ///
    /// MOD-6 deliberately seeded **no** agent, because `agent.launch` is `JSONB NOT NULL` and its
    /// shape was still ANA-4's to settle, so a guess would have needed migrating away (MOD-6 plan
    /// D5, blueprint H.2). ANA-4 settled it and assigned the seed to MOD-2, which is this: the
    /// rows come from [`seed_rows`], one source shared with the demo fixture.
    ///
    /// The agent insert is a **name-keyed top-up** (MOD-2 D88): every row of [`seed_rows`] is
    /// offered on every pass and `ON CONFLICT (name) DO NOTHING` decides. It used to be guarded on
    /// the `agent` table being empty as well, so that a maintainer's edit could not be undone —
    /// but `ON CONFLICT (name) DO NOTHING` already protects an existing row, edits included, while
    /// the emptiness guard also refused a row that had never been seeded *at all*. Every box that
    /// has ever launched has a non-empty table, so a row a later milestone adds would have reached
    /// none of them, and the feature would be verifiable only on a fresh database. The cost,
    /// accepted at CONFIRM: a row that was **deleted** comes back on the next connect. MOD-23's
    /// model for retiring an agent is `enabled = false`, not deletion, and that survives.
    ///
    /// **`R-USR-2`, one row, whatever the OS user is called.** The `app_user` insert is guarded by
    /// `WHERE NOT EXISTS (SELECT 1 FROM app_user)` rather than by `ON CONFLICT (name)`: a second
    /// box whose `USERNAME` differs would otherwise seed a *second* user and every row it wrote
    /// would be attributed to it. For the same reason the returned id is the oldest row's, not the
    /// one matching `name` - the name is what a first, empty database is stamped with, and never a
    /// lookup key.
    ///
    /// The name is an argument rather than always the env-derived one so a test can exercise a
    /// second box without mutating the process environment.
    ///
    /// **`SHARE ROW EXCLUSIVE` on `app_user` is the transaction's first statement.** Under READ
    /// COMMITTED - the default, and what this pool uses - `WHERE NOT EXISTS` is evaluated against
    /// a snapshot taken when the statement starts, so two first-ever connects that overlap both
    /// see an empty table and both insert; the differing names slip past `ON CONFLICT (name)` and
    /// `R-USR-2` is broken. The lock conflicts with itself and not with the readers, so the second
    /// connect waits for the first to commit and then sees the row it seeded. It is taken before
    /// any other statement so two seeds can never hold half of it each and deadlock.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn seed_if_empty_as(&self, name: &str) -> Result<UserId> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        sqlx::query("LOCK TABLE app_user IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        sqlx::query!(
            "INSERT INTO app_user (id, name) SELECT $1, $2 \
             WHERE NOT EXISTS (SELECT 1 FROM app_user) ON CONFLICT (name) DO NOTHING",
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

        // A **name**-keyed top-up (MOD-2 D88), not an emptiness check: `ON CONFLICT (name) DO
        // NOTHING` inserts only the names the table lacks, so a box that was seeded by an older
        // build gains a row a later milestone added, and a row that is already there is not
        // written over — which is what the empty-table guard was protecting. It is inside the
        // transaction that holds the `app_user` lock, so two first connects cannot both insert.
        for agent in seed_rows(Utc::now()) {
            sqlx::query!(
                "INSERT INTO agent (id, name, transport, launch, models, default_model, billing, \
                                    enabled, settings, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 ON CONFLICT (name) DO NOTHING",
                agent.id.as_uuid(),
                agent.name,
                agent.transport.as_str(),
                &agent.launch,
                &agent.models[..],
                agent.default_model.as_deref(),
                agent.billing.as_str(),
                agent.enabled,
                &agent.settings,
                agent.created_at,
                agent.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        // The oldest row, not `WHERE name = $1`: this box's OS user name is how an empty database
        // is stamped, not how the single `app_user` of `R-USR-2` is found again.
        let row = sqlx::query!(
            r#"SELECT id as "id: UserId" FROM app_user ORDER BY created_at, id LIMIT 1"#,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.id)
    }

    /// Registers this box by its `box.toml` id, checked by the machine fingerprint (PRD D1-D5).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn register_box(
        &self,
        identity: &Identity,
        fingerprint: Option<&Fingerprint>,
    ) -> Result<Registration> {
        let _ = (identity, fingerprint, this_os_family());
        todo!("MOD-7 T1 (c)")
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

    /// What registration answered at the last bootstrap; `None` until one has run.
    #[must_use]
    pub const fn registration(&self) -> Option<&Registration> {
        self.registration.as_ref()
    }

    /// Seeds, registers this box and adopts the database's id for it (plan D5, D6).
    ///
    /// No file I/O: persisting an adopted id back to `box.toml` is the caller's job, because only
    /// the caller knows which config root it read the identity from.
    async fn bootstrap(&mut self) -> Result<()> {
        self.this_user = self.seed_if_empty().await?;
        let fingerprint = crate::identity::machine_fingerprint().await;
        let registration = self
            .register_box(&self.identity, fingerprint.as_ref())
            .await?;
        let id = registration.box_id(self.identity.box_id);
        self.identity.box_id = id;
        self.this_box = id;
        self.registration = Some(registration);
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
