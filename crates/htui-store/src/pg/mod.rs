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
use serde_json::Value;
use sqlx::PgConnection;
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

/// The `app_setting` key holding the **target version** (MOD-40 plan D9, PRD D3): the highest
/// [`HTUI_VERSION`] that has applied migrations to this database, as a JSON string.
///
/// Raised by [`PgStore::apply_migrations`] and never lowered. A TUI below it runs and says so
/// ([`PgStore::below_target`]); a headless process below it refuses
/// ([`HeadlessError::BelowTarget`]). Snake case like every other key; no reader iterates
/// `app_setting`, so the row is invisible to the settings resolvers and the Settings tab.
pub const TARGET_VERSION_KEY: &str = "htui_target_version";

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
        match self {
            Self::New | Self::Known { .. } => presented,
            Self::Copied { minted, .. } => *minted,
        }
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
    below_target: Option<String>,
}

/// What [`PgStore::connect`] found: a usable store plus the state of its schema.
#[derive(Debug, Clone)]
pub struct Connected {
    /// The store, usable for reads even while migrations are pending.
    pub store: PgStore,
    /// Whether the embedded set is fully applied.
    pub migrations: MigrationState,
}

/// Why [`PgStore::connect_headless`] refused (MOD-40 plan D8, `R-STO-5` as amended 2026-09-26: a
/// headless process never migrates; it refuses and reports).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeadlessError {
    /// The store refused as [`PgStore::connect`] would: unreachable, a newer schema, checksum
    /// drift, a partially applied version, or a target that is not a version.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// This many embedded migrations are not applied. A TUI asks; a headless process refuses.
    #[error(
        "{0} schema migration(s) are pending, and a headless process never migrates; start \
         `htui` once to apply them"
    )]
    MigrationsPending(usize),
    /// This build is below the database's target version ([`TARGET_VERSION_KEY`]).
    #[error(
        "this htui is {ours}, below {target}, the version that last migrated this database; \
         upgrade htui on this box"
    )]
    BelowTarget {
        /// [`HTUI_VERSION`].
        ours: String,
        /// The stored target, as `semver` prints it.
        target: String,
    },
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
    /// **Registration (MOD-7 D1-D3)**: the `box` row is keyed on `identity.box_id` and checked by
    /// this machine's [`Fingerprint`] ([`PgStore::register_box`]). [`PgStore::this_box`] answers
    /// the id registration answered, which differs from `box.toml`'s only for a copied file
    /// ([`Registration::Copied`]). The caller persists that back by comparing the two and, when
    /// they differ, writing [`PgStore::identity`] through
    /// [`identity::store`](crate::identity::store) - `PgStore` never writes `box.toml` itself.
    ///
    /// **Target version (MOD-40 plan D9)**: over an up-to-date schema the stored
    /// `htui_target_version` is read, and a build below it is recorded in
    /// [`PgStore::below_target`] and otherwise carries on: a TUI warns, it does not refuse (PRD
    /// D3). A pending schema reads nothing; `apply_migrations` decides.
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
        let pool = open_pool(dsn, connect_timeout).await?;

        let migrations = schema_state(&pool).await?;
        let mut store = Self {
            pool,
            identity: identity.clone(),
            this_box: BoxId::default(),
            this_user: UserId::default(),
            registration: None,
            below_target: None,
        };
        if migrations == MigrationState::UpToDate {
            store.below_target = tui_below_target(&store.pool).await?;
            store.bootstrap().await?;
        }
        Ok(Connected { store, migrations })
    }

    /// Connects for a process with no one to ask (MOD-40 plan D8): refuses rather than migrates.
    ///
    /// Order (blueprint B19), every refusal before any write: open the pool; read the schema
    /// state **without** creating the migrations table (a missing table is every embedded
    /// migration pending; a role that cannot create tables can still connect, F-22); refuse a
    /// dirty, newer or drifted schema as [`PgStore::connect`] does, and a pending one; refuse a
    /// build below the stored target version, and a target that is not a version; then seed and
    /// register exactly as `connect` does, so `this_box` and `this_user` are filled. "Never
    /// migrates" is not "never writes": the bootstrap writes `app_user`, `capability_tag`,
    /// `app_setting` defaults, the seeded agents and this box's row, as `connect` always has.
    ///
    /// # Errors
    ///
    /// [`HeadlessError::Store`] for everything [`PgStore::connect`] refuses, and for a malformed
    /// target; [`HeadlessError::MigrationsPending`]; [`HeadlessError::BelowTarget`].
    pub async fn connect_headless(
        dsn: &str,
        identity: &Identity,
        connect_timeout: Duration,
    ) -> core::result::Result<Self, HeadlessError> {
        let pool = open_pool(dsn, connect_timeout).await?;
        let mut conn = pool.acquire().await.map_err(map_sqlx)?;
        let migrations = if migrations_table_exists(&mut conn).await? {
            applied_state(&mut conn).await?
        } else {
            MigrationState::Pending(embedded_migrations())
        };
        drop(conn);
        if let MigrationState::Pending(n) = migrations {
            return Err(HeadlessError::MigrationsPending(n));
        }
        if let Some(stored) = stored_target(&pool).await? {
            let target = parse_target(&stored).map_err(|text| {
                StoreError::Backend(format!(
                    "app_setting.{TARGET_VERSION_KEY} holds {text}, which is not a version; a \
                     headless process does not guess"
                ))
            })?;
            if this_version() < target {
                return Err(HeadlessError::BelowTarget {
                    ours: HTUI_VERSION.to_owned(),
                    target: target.to_string(),
                });
            }
        }
        let mut store = Self {
            pool,
            identity: identity.clone(),
            this_box: BoxId::default(),
            this_user: UserId::default(),
            registration: None,
            below_target: None,
        };
        store.bootstrap().await?;
        Ok(store)
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
            below_target: None,
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
    ///
    /// Then raises `app_setting.htui_target_version` to this build's [`HTUI_VERSION`] and never
    /// lowers it (MOD-40 plan D9), so a headless process older than the last migrator refuses to
    /// run against the schema it migrated. A target that stays above this build is kept in
    /// [`PgStore::below_target`].
    pub async fn apply_migrations(&mut self) -> Result<()> {
        MIGRATOR.run(&self.pool).await.map_err(map_migrate)?;
        self.bootstrap().await?;
        self.below_target = raise_target(&self.pool).await?;
        Ok(())
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

    /// Registers this box under its `box.toml` id, checked by the machine fingerprint (MOD-7 PRD
    /// D1-D5, blueprint D19, D31).
    ///
    /// The row is keyed on `identity.box_id`, never on the hostname: a rename is the same box, and
    /// two boxes may share a hostname. One transaction, three statements:
    ///
    /// 1. **Insert first**, `ON CONFLICT (id) DO NOTHING`: the only statement that can create the
    ///    row, so two first registrations of one id cannot both insert, and neither fails
    ///    ([`Registration::New`] for the one that inserted).
    /// 2. When it inserted nothing, the row exists: **lock** it (`FOR UPDATE`) and read its owner,
    ///    hostname and fingerprint.
    /// 3. A row under another user, or a stored fingerprint that differs from `fingerprint`, is a
    ///    `box.toml` carried from somewhere else: the row is left untouched and this machine is
    ///    inserted under a freshly minted id with statement 1's text ([`Registration::Copied`]).
    ///    Otherwise it is this box: the display fields and `last_seen_at` are refreshed, a missing
    ///    fingerprint is recorded and a stored one never cleared ([`Registration::Known`]).
    ///
    /// No fingerprint on either side decides by the id alone. `htui_version` is written only by the
    /// insert; it, the probe columns, the tags, `quirks`, `settings` and `edit_version` are never
    /// written by a reconnect (plan D5): the probe writer owns them.
    ///
    /// [`PgStore::connect`] keeps the answered id in [`PgStore::identity`]; writing a minted one
    /// back to `box.toml` through [`identity::store`](crate::identity::store) is the caller's job,
    /// since only the caller knows the config root.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`], and [`StoreError::Backend`] in the
    /// astronomically unlikely case that a freshly minted id already has a row.
    pub async fn register_box(
        &self,
        identity: &Identity,
        fingerprint: Option<&Fingerprint>,
    ) -> Result<Registration> {
        let presented = fingerprint.map(Fingerprint::as_hex);
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        // (a) Insert first.
        let new_row = NewBoxRow {
            user_id: self.this_user,
            identity,
            os_family: this_os_family(),
            fingerprint: presented.as_deref(),
        };
        if new_row.insert(&mut tx, identity.box_id).await? {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(Registration::New);
        }

        // (lock) The row exists, committed, and is now ours until the transaction ends.
        let row = sqlx::query!(
            r#"
            SELECT user_id AS "user_id: UserId", hostname, machine_fingerprint
              FROM box WHERE id = $1 FOR UPDATE
            "#,
            identity.box_id.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let copied = row.user_id != self.this_user
            || matches!(
                (row.machine_fingerprint.as_deref(), presented.as_deref()),
                (Some(stored), Some(this)) if stored != this
            );

        let answer = if copied {
            // Statement (a) again, under a minted id (D31); the copied row is never touched.
            let minted = BoxId::new();
            if !new_row.insert(&mut tx, minted).await? {
                return Err(StoreError::Backend(
                    "a fresh box id already exists".to_owned(),
                ));
            }
            Registration::Copied {
                previous: identity.box_id,
                minted,
            }
        } else {
            // (c) The same machine: display fields and `last_seen_at` only.
            sqlx::query!(
                r#"
                UPDATE box
                   SET hostname = $2, os_family = $3, arch = $4, last_seen_at = clock_timestamp(),
                       machine_fingerprint = COALESCE(machine_fingerprint, $5)
                 WHERE id = $1
                "#,
                identity.box_id.as_uuid(),
                identity.hostname,
                new_row.os_family.as_str(),
                std::env::consts::ARCH,
                new_row.fingerprint,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            Registration::Known {
                renamed_from: (row.hostname != identity.hostname).then_some(row.hostname),
            }
        };

        tx.commit().await.map_err(map_sqlx)?;
        Ok(answer)
    }

    /// The box heartbeat (MOD-40 plan D7, `docs/ANA-16.md` C4): stamps `box.last_seen_at` with the
    /// server's `clock_timestamp()` and answers whether a row had `id`.
    ///
    /// One column, the only one registration refreshes that no reconnect-free session would
    /// otherwise move: never `hostname`, the probe columns, the tags, `quirks`, `settings`,
    /// `machine_fingerprint` or `edit_version`, so a beat cannot stale an open box editor. The
    /// migration's `BEFORE UPDATE` trigger moves `updated_at` with it (`0001_init.sql:575-580`),
    /// which nothing keys on: the editors' token is `edit_version`, and the mirror re-reads the
    /// own box row whole each pass (MOD-40 blueprint F-21).
    ///
    /// The time is the database's, like [`PgStore::register_box`]'s: a box whose clock is off
    /// still reports when the server last heard from it. Nothing reads `last_seen_at` yet (PRD out
    /// of scope: liveness is ANA-2's `DeadWalks`, not this stamp).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn touch_box(&self, id: BoxId) -> Result<bool> {
        let touched = sqlx::query!(
            "UPDATE box SET last_seen_at = clock_timestamp() WHERE id = $1",
            id.as_uuid(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(touched == 1)
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

    /// `box.id` of this box: the id registration answered, which differs from `box.toml`'s only
    /// for a copied file.
    #[must_use]
    pub const fn this_box(&self) -> BoxId {
        self.this_box
    }

    /// The identity this store registered under, with `box_id` the id registration answered, which
    /// differs from `box.toml`'s only for a copied file.
    ///
    /// This is what a caller writes back to `box.toml` when it differs from what it passed to
    /// [`PgStore::connect`] (MOD-7 D3).
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

    /// The database's target version when this build's [`HTUI_VERSION`] is below it (MOD-40
    /// plan D9, blueprint B16): read by [`PgStore::connect`] over an up-to-date schema, and
    /// recomputed by [`PgStore::apply_migrations`], which may raise the target but never lowers
    /// it. `None` when this build is at or above the target, when none is stored, and on a
    /// store whose pending schema has not been applied yet.
    ///
    /// A TUI below the target runs and says so once (PRD D3); a headless process never gets a
    /// store to ask ([`PgStore::connect_headless`] refuses).
    #[must_use]
    pub fn below_target(&self) -> Option<&str> {
        self.below_target.as_deref()
    }

    /// Seeds, reads this machine's fingerprint and registers this box (plan D5, MOD-7 D1-D3).
    ///
    /// No file I/O: persisting a minted id back to `box.toml` is the caller's job, because only
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

/// The values statement (a) of [`PgStore::register_box`] inserts, whichever id they go under.
///
/// One statement text for both inserts, the first registration and the minted `Copied` one
/// (blueprint D31), so the two cannot drift apart and share one `.sqlx` entry.
struct NewBoxRow<'a> {
    user_id: UserId,
    identity: &'a Identity,
    os_family: OsFamily,
    fingerprint: Option<&'a str>,
}

impl NewBoxRow<'_> {
    /// Inserts the row under `id` unless one exists; `true` when it inserted.
    async fn insert(&self, conn: &mut PgConnection, id: BoxId) -> Result<bool> {
        let inserted = sqlx::query!(
            r#"
            INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version,
                             machine_fingerprint)
            VALUES ($1, $2, $3, $4, '', $5, $6, $7)
            ON CONFLICT (id) DO NOTHING
            RETURNING id AS "id!: BoxId"
            "#,
            id.as_uuid(),
            self.user_id.as_uuid(),
            self.identity.hostname,
            self.os_family.as_str(),
            std::env::consts::ARCH,
            HTUI_VERSION,
            self.fingerprint,
        )
        .fetch_optional(conn)
        .await
        .map_err(map_sqlx)?;
        Ok(inserted.is_some())
    }
}

/// The pool both connects open: eight connections, `connect_timeout` to acquire one.
async fn open_pool(dsn: &str, connect_timeout: Duration) -> Result<PgPool> {
    let options = PgConnectOptions::from_str(dsn).map_err(map_sqlx)?;
    PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(connect_timeout)
        .connect_with(options)
        .await
        .map_err(map_sqlx)
}

/// Compares `_sqlx_migrations` against the embedded set (ANA-9 §5.0), creating the empty
/// bookkeeping table first when there is none — the TUI's path, which may go on to migrate.
///
/// `list_applied_migrations` takes the table name in sqlx 0.9 and lives on `PgConnection`, not on
/// `PgPool`, so a connection is acquired first (blueprint H.7).
async fn schema_state(pool: &PgPool) -> Result<MigrationState> {
    let mut conn = pool.acquire().await.map_err(map_sqlx)?;
    conn.ensure_migrations_table(MIGRATOR.table_name.as_ref())
        .await
        .map_err(map_migrate)?;
    applied_state(&mut conn).await
}

/// The read-only half of [`schema_state`] (MOD-40 blueprint B15): refuses a dirty version, a
/// newer applied version and checksum drift, and counts what is pending. Writes nothing, so a
/// headless process may run it under a role that cannot create a table.
async fn applied_state(conn: &mut PgConnection) -> Result<MigrationState> {
    let table = MIGRATOR.table_name.as_ref();
    if let Some(version) = conn.dirty_version(table).await.map_err(map_migrate)? {
        return Err(StoreError::Backend(format!(
            "migration {version} is partially applied; fix it and remove its `{table}` row"
        )));
    }
    let applied = conn
        .list_applied_migrations(table)
        .await
        .map_err(map_migrate)?;

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

/// Whether the migrations table exists at all, without creating it (blueprint B15, F-22).
async fn migrations_table_exists(conn: &mut PgConnection) -> Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT to_regclass($1) IS NOT NULL AS "exists!""#,
        MIGRATOR.table_name.as_ref(),
    )
    .fetch_one(conn)
    .await
    .map_err(map_sqlx)
}

/// How many embedded up-migrations there are: a database with no migrations table has all of
/// them pending.
fn embedded_migrations() -> usize {
    MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .count()
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

/// This build's version. `CARGO_PKG_VERSION` is semver by cargo's own rule, so the parse cannot
/// fail on a build cargo produced.
fn this_version() -> semver::Version {
    semver::Version::parse(HTUI_VERSION).expect("CARGO_PKG_VERSION is a semver version")
}

/// A stored target, parsed: the version, or the stored JSON as text when it is not a string
/// holding one (MOD-40 blueprint B18).
fn parse_target(stored: &Value) -> core::result::Result<semver::Version, String> {
    stored
        .as_str()
        .and_then(|text| semver::Version::parse(text).ok())
        .ok_or_else(|| stored.to_string())
}

/// The stored target document, if any.
///
/// The statement is `stored_setting`'s `App` text byte for byte (`read.rs`), so the two share
/// one `.sqlx` entry.
async fn stored_target(pool: &PgPool) -> Result<Option<Value>> {
    let row = sqlx::query!(
        "SELECT value, updated_at FROM app_setting WHERE key = $1",
        TARGET_VERSION_KEY,
    )
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(row.map(|row| row.value))
}

/// What a TUI makes of the stored target: the target when this build is below it, `None`
/// otherwise. A malformed one is logged and ignored: a TUI runs (B18).
async fn tui_below_target(pool: &PgPool) -> Result<Option<String>> {
    let Some(stored) = stored_target(pool).await? else {
        return Ok(None);
    };
    match parse_target(&stored) {
        Ok(target) if this_version() < target => Ok(Some(target.to_string())),
        Ok(_) => Ok(None),
        Err(text) => {
            tracing::warn!(
                stored = %text,
                "app_setting.{TARGET_VERSION_KEY} is not a version; ignoring it"
            );
            Ok(None)
        }
    }
}

/// Raises the target to this build's version, never lowering it (MOD-40 plan D9, blueprint
/// B17), and answers the target when it is still above this build.
///
/// One transaction, three statements. The insert goes first because a `FOR UPDATE` on a row that
/// does not exist locks nothing: two first appliers would both read "absent" and the second
/// insert would fail on the primary key after its migrations had already run. `ON CONFLICT DO
/// NOTHING` makes the second wait for the first's commit and then do nothing; the locked read
/// that follows sees the committed row, and the comparison and the update run under that lock,
/// so two appliers of different versions leave the higher one whichever commits first.
///
/// A stored value that is not a version is replaced with this build's (B18): the TUI that has
/// just migrated the database is the authority on what it now needs.
async fn raise_target(pool: &PgPool) -> Result<Option<String>> {
    let ours = this_version();
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let inserted = sqlx::query!(
        "INSERT INTO app_setting (key, value) VALUES ($1, to_jsonb($2::text)) \
         ON CONFLICT (key) DO NOTHING",
        TARGET_VERSION_KEY,
        HTUI_VERSION,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    let mut above = None;
    if inserted == 0 {
        let stored = sqlx::query_scalar!(
            "SELECT value FROM app_setting WHERE key = $1 FOR UPDATE",
            TARGET_VERSION_KEY,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let raise = match stored.as_ref().map(parse_target) {
            // Deleted by hand between the two statements: the next apply inserts it.
            None => false,
            Some(Ok(target)) if target > ours => {
                above = Some(target.to_string());
                false
            }
            Some(Ok(target)) => target < ours,
            Some(Err(text)) => {
                tracing::warn!(
                    stored = %text,
                    "app_setting.{TARGET_VERSION_KEY} is not a version; replacing it with this build's"
                );
                true
            }
        };
        if raise {
            sqlx::query!(
                "UPDATE app_setting SET value = to_jsonb($2::text) WHERE key = $1",
                TARGET_VERSION_KEY,
                HTUI_VERSION,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }
    }
    tx.commit().await.map_err(map_sqlx)?;
    Ok(above)
}
