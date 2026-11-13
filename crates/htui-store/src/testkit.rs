//! Throwaway-database harness and mirror seeder for the tests (MOD-6 blueprint E, plan D13).
//!
//! Every test that needs a server creates its own database, migrates it and drops it again, so the
//! concurrency cases of ANA-9 §11 are deterministic and two test binaries never share state.
//! `HTUI_TEST_DATABASE_URL` is a *maintenance* DSN whose user has `CREATEDB`; when it is unset the
//! helpers print [`SKIP`] and answer `None`, which is what keeps `cargo test --workspace
//! --all-features` green on a box without Postgres.
//!
//! This lived under `tests/common/` until MOD-2 milestone 4 (T21). It moved into the library
//! behind the `test-support` feature for `docs/ANA-4.md` §11 criterion 12 - a **driver** wrote the
//! offline buffer, a **server** landed it - which only the `htui` crate, depending on both
//! `htui-agent` and this one, could prove. MOD-25 withdrew that criterion and the buffer has since
//! been removed; the harness stayed. `htui-core::fixtures` sets the precedent: test-only data, in
//! the library, behind a feature.

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

/// The template database of this process's migration set, built or found once (TOOL-16).
static TEMPLATE: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// The advisory-lock key every process takes while it looks for or builds a template, so two
/// test binaries (or two worktrees) on one server never build the same one twice. Any constant
/// does; this one is "htuitmpl" in ASCII.
const TEMPLATE_LOCK: i64 = 0x6874_7569_746d_706c;

/// The maintenance DSN, or `None` (after printing [`SKIP`]) when [`ENV_URL`] is unset.
fn maint_url() -> Option<String> {
    match std::env::var(ENV_URL) {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            if std::env::var("CI").is_ok() {
                panic!(
                    "{} is not set but CI is running. Database tests must not be skipped in CI.",
                    ENV_URL
                );
            }
            println!("{SKIP}");
            None
        }
    }
}

/// Once per process: drops what earlier runs left behind.
///
/// A test database or an unfinished template build older than an hour belongs to a run that
/// died. A template of **another** migration set goes after a week: it belongs to a branch that
/// moved on, and if that branch runs again its first test rebuilds it.
async fn sweep(maint_url: &str) {
    SWEEP
        .get_or_init(|| async {
            let Ok(mut sweep_conn) = PgConnection::connect(maint_url).await else {
                return;
            };
            let stale: Vec<(String,)> = sqlx::query_as(
                r"
                SELECT datname
                FROM pg_database
                WHERE ((datname LIKE 'htui\_test\_%' OR datname LIKE 'htui\_tmplbuild\_%')
                       AND (pg_stat_file('base/' || oid, true)).modification
                           < now() - interval '1 hour')
                   OR (datname LIKE 'htui\_tmpl\_%' AND datname <> $1
                       AND (pg_stat_file('base/' || oid, true)).modification
                           < now() - interval '7 days')
                ",
            )
            .bind(template_name())
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
}

/// `<prefix><12 hex>`. The *tail* of a UUIDv7 is the random half; its head is a millisecond
/// timestamp, which two tests starting in the same millisecond share.
fn unique_name(prefix: &str) -> String {
    let hex = Uuid::now_v7().simple().to_string();
    format!("{prefix}{}", &hex[hex.len() - 12..])
}

/// `htui_tmpl_<16 hex>`: a hash of every embedded migration (version, description, checksum) and
/// of [`crate::pg::HTUI_VERSION`], the target version the template carries. A migration edited in
/// place changes its checksum and therefore the name, so a stale template is never copied.
fn template_name() -> String {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;

    let mut hash = Sha256::new();
    for migration in crate::MIGRATOR.iter() {
        hash.update(migration.version.to_le_bytes());
        hash.update(migration.description.as_bytes());
        hash.update(&migration.checksum[..]);
    }
    hash.update(crate::pg::HTUI_VERSION.as_bytes());
    let digest = hash.finalize();
    let mut name = String::from("htui_tmpl_");
    for byte in &digest[..8] {
        let _ = write!(name, "{byte:02x}");
    }
    name
}

/// The name of a database [`fresh_db`] can copy: every migration applied and the target raised,
/// exactly what [`PgStore::apply_migrations`] leaves **before** its bootstrap. Nothing is seeded
/// and no box is registered, so each copy's own connect seeds and registers its test's identity
/// as a first connect always has.
///
/// Built under a temporary name and renamed when complete, so a template that exists is a
/// finished one; a build that died leaves only an `htui_tmplbuild_` database for [`sweep`].
async fn template(maint_url: &str) -> &'static str {
    TEMPLATE
        .get_or_init(|| async {
            let name = template_name();
            let mut maint = PgConnection::connect(maint_url)
                .await
                .expect("connect to HTUI_TEST_DATABASE_URL");
            sqlx::query("SELECT pg_advisory_lock($1)")
                .bind(TEMPLATE_LOCK)
                .execute(&mut maint)
                .await
                .expect("take the template lock");
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
                    .bind(&name)
                    .fetch_one(&mut maint)
                    .await
                    .expect("look the template up");
            if !exists {
                build_template(&mut maint, maint_url, &name).await;
            }
            sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(TEMPLATE_LOCK)
                .execute(&mut maint)
                .await
                .expect("release the template lock");
            let _ = maint.close().await;
            name
        })
        .await
}

/// Migrates a new database, raises its target and renames it to `name`.
async fn build_template(maint: &mut PgConnection, maint_url: &str, name: &str) {
    let build = unique_name("htui_tmplbuild_");
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE DATABASE \"{build}\"")))
        .execute(&mut *maint)
        .await
        .expect("CREATE DATABASE for the template");

    // The pool `PgStore::connect` opens, so the template is migrated under the session settings a
    // test's own `apply_migrations` would have used. Over a pending schema the connect registers
    // nothing, and the identity is never written anywhere.
    let identity = Identity {
        box_id: htui_core::model::BoxId::new(),
        hostname: "htui-template".to_owned(),
    };
    let connected = PgStore::connect(&with_database(maint_url, &build), &identity)
        .await
        .expect("connect to the template build");
    let pool = connected.store.pool().clone();
    crate::MIGRATOR
        .run(&pool)
        .await
        .expect("apply the embedded migrations to the template");
    crate::pg::raise_target(&pool)
        .await
        .expect("raise the template's target version");
    pool.close().await;
    drop(connected);

    // A closed client's backend can take a moment to exit, and a rename refuses while any session
    // is connected to the database.
    let rename = format!("ALTER DATABASE \"{build}\" RENAME TO \"{name}\"");
    for attempt in 0.. {
        match sqlx::raw_sql(AssertSqlSafe(rename.clone()))
            .execute(&mut *maint)
            .await
        {
            Ok(_) => break,
            Err(_) if attempt < 100 => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => panic!("rename the template build to {name}: {e}"),
        }
    }
}

/// Creates a database (a copy of `template` when given), mints a throwaway identity and connects.
async fn create_db(maint_url: String, template: Option<&str>) -> TestDb {
    let name = unique_name("htui_test_");
    let mut maint = PgConnection::connect(&maint_url)
        .await
        .expect("connect to HTUI_TEST_DATABASE_URL");
    let create = match template {
        Some(template) => format!("CREATE DATABASE \"{name}\" TEMPLATE \"{template}\""),
        None => format!("CREATE DATABASE \"{name}\""),
    };
    sqlx::raw_sql(AssertSqlSafe(create))
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

    TestDb {
        store: connected.store,
        pool,
        name,
        url,
        maint_url,
        config_root,
        identity,
        migrations_at_connect: connected.migrations,
        dropped: false,
    }
}

/// A fresh database with the migrations **not** applied - or `None` when [`ENV_URL`] is unset.
///
/// The migration tests need a database in that state; every other test wants [`fresh_db`].
pub async fn bare_db() -> Option<TestDb> {
    let maint_url = maint_url()?;
    sweep(&maint_url).await;
    Some(create_db(maint_url, None).await)
}

/// A fresh, migrated, empty database - or `None` when [`ENV_URL`] is unset.
///
/// A copy of the [`template`] database, so the migrations run once per process and migration set
/// rather than once per test (TOOL-16). The connect over the copy seeds and registers, as
/// [`PgStore::apply_migrations`]'s bootstrap would have, so the store and the rows are what
/// migrating a bare database gives. The one visible difference: [`TestDb::migrations_at_connect`]
/// is `UpToDate`, never `Pending`; a test about pending migrations starts from [`bare_db`].
pub async fn fresh_db() -> Option<TestDb> {
    let maint_url = maint_url()?;
    sweep(&maint_url).await;
    let template = template(&maint_url).await;
    let db = create_db(maint_url, Some(template)).await;
    assert_eq!(
        db.migrations_at_connect,
        MigrationState::UpToDate,
        "a copy of the template {template} is fully migrated"
    );
    Some(db)
}

/// [`fresh_db`] plus `store.load_demo(&fixtures::demo_data())`, with the seeded user removed.
///
/// `apply_migrations` seeds an `app_user` named after this OS user and registers this box under
/// it; the fixture then brings its own, whose `created_at` is the epoch. Two rows is exactly what
/// `R-USR-2` forbids, and `PgStore::seed_if_empty_as` answers with the oldest of them - so a
/// second connect would find its `box.toml` id under a user that is no longer the oldest and mint
/// a new box (`Registration::Copied`).
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

/// A second process's store on `url`, a [`demo_db`] database, registered as the fixture's box
/// `ids::BOX`: the box `db.store` stands for after `load_demo` repointed it (MOD-24 D2). A
/// `PgStore::connect(url, &db.identity)` would name the minted box `demo_db` deleted.
///
/// The fixture box belongs to the fixture user, the only `app_user` left and therefore the oldest,
/// and its row carries no fingerprint, so registration answers `Known` for any machine. The first
/// connect records this machine's fingerprint, and a later one from the same machine is `Known`
/// again. Registration also refreshes the row's display columns (`os_family`, `arch`). The
/// hostname is kept as the fixture's.
///
/// # Panics
/// When the headless connect fails, or registration answers any box but `ids::BOX`.
pub async fn fixture_box_store(url: &str) -> PgStore {
    let identity = Identity {
        box_id: htui_core::fixtures::ids::BOX,
        hostname: "DESKTOP-HTUI".to_owned(),
    };
    let store = PgStore::connect_headless(
        url,
        &identity,
        crate::pg::CONNECT_TIMEOUT,
        crate::pg::PoolSize::WORKER_DEFAULT,
    )
    .await
    .expect("the fixture box connects headless");
    assert_eq!(
        store.this_box(),
        htui_core::fixtures::ids::BOX,
        "registered as the fixture box, not a copy: {:?}",
        store.registration()
    );
    store
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

/// Serialises the tests that talk to the test Qdrant within one process: every test under
/// `cargo test` (TOOL-16), only the one test under cargo-nextest (TOOL-17).
static QDRANT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One test's turn at the test Qdrant: hold it for the whole test.
pub type QdrantTurn = tokio::sync::MutexGuard<'static, ()>;

/// Waits for this test's turn at the test Qdrant, so the live Qdrant cases of one test binary run
/// one after another while the rest of the binary runs in parallel (TOOL-16).
///
/// Each case owns its collection, so nothing is shared but the server, and that is the point:
/// a dozen collections created and indexed at once make Qdrant's collection and payload-index
/// calls time out (`create collection: ... Timeout expired`), where one at a time they all pass.
/// Before TOOL-15 every test file was its own process and the gate ran single-threaded, so the
/// cases never overlapped.
///
/// The lock only reaches one process. Under `cargo test`, the documented item-run command, a
/// binary's tests share it, so the lock is what keeps them apart. Under cargo-nextest, which the
/// gate runs (TOOL-17), each test is its own process and the lock costs nothing: there the
/// `live-qdrant` test group in `.config/nextest.toml` serialises them instead, and it holds a case
/// only when the case lives in `qdrant_live`/`qdrant_worker` or its name starts with
/// `live_qdrant_`. A live Qdrant case needs both.
pub async fn qdrant_turn() -> QdrantTurn {
    QDRANT.lock().await
}

/// [`QdrantStore::connect_to`](crate::vector::QdrantStore::connect_to), retried when the client's
/// deadline expires (TOOL-16).
///
/// Creating a collection and its payload indexes takes about 0.1 s on an idle Qdrant, but past
/// qdrant-client's default 5 s deadline while the parallel gate saturates the box ("create
/// collection: … Timeout expired"), which a [`qdrant_turn`] cannot help: the load is every other
/// test, not another Qdrant case. `connect_to` is idempotent (an existing collection is checked
/// and its indexes re-asserted), so a retry after a create the server did finish is safe. Any
/// other error, an embedder mismatch included, is returned at once.
///
/// # Errors
/// What the last attempt answered.
pub async fn qdrant_connect_to<E: crate::embed::DenseEmbedder + Clone>(
    settings: &crate::qdrant_settings::QdrantSettings,
    embedder: E,
    collection: &str,
) -> Result<crate::vector::QdrantStore<E>> {
    let mut attempt = 1;
    loop {
        match crate::vector::QdrantStore::connect_to(settings, embedder.clone(), collection).await {
            Err(e) if attempt < 3 && e.to_string().contains("Timeout expired") => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            other => return other,
        }
    }
}

/// Serialises the tests that touch the fake keyring: it is one process-wide slot.
static KEYRING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Holds the fake keyring installed and empty until dropped (MOD-15 M6 D18).
#[derive(Debug)]
pub struct KeyringGuard {
    /// `KEYRING`, held for the length of the test. Underscored because nothing reads it — the
    /// whole of its job is to be released when this guard drops.
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for KeyringGuard {
    fn drop(&mut self) {
        *crate::secret::fake() = None;
    }
}

/// Routes [`crate::secret::get_dsn`], [`crate::secret::set_dsn`] and
/// [`crate::secret::clear_dsn`] to an empty in-process slot until the guard drops (D18).
///
/// **Never the developer's keyring.** A test that forgets the guard and calls `set_dsn` writes to
/// the OS store, which is why every case that can reach the keyring takes one as its first
/// statement. `keyring::mock` is not enough on its own: it keeps the secret in the `Entry` and
/// `secret.rs` opens a fresh `Slot` per call, so a `set_dsn` there is invisible to the next
/// `get_dsn` (blueprint flag L).
///
/// The returned guard also holds a process-wide lock, so two `#[tokio::test]`s that both install
/// the slot run one after the other rather than overwriting each other's DSN.
pub async fn mock_keyring() -> KeyringGuard {
    let lock = KEYRING.lock().await;
    *crate::secret::fake() = Some(crate::secret::Fake::Slot(
        crate::secret::FakeSlots::default(),
    ));
    KeyringGuard { _lock: lock }
}

/// What a keyring that cannot be opened at all answers with.
///
/// The sentence a Linux box with a session bus and **no unlocked collection** produces — a minimal
/// window manager, a container, most CI images. It is `keyring`'s own `NoStorageAccess` text, so a
/// test asserting on it is asserting on what a user would actually see.
pub const BROKEN_KEYRING: &str =
    "Couldn't access platform secure storage: Secret Service: no result found";

/// [`mock_keyring`] for a keyring that answers **nothing**: every read, write and delete fails
/// with [`BROKEN_KEYRING`] until the guard drops.
///
/// Not "empty": an empty keyring is [`mock_keyring`], and the difference between the two is the
/// point. `get_dsn` keeps reporting the failure as an error, and what the callers do with it —
/// [`crate::connect::start`] launches offline anyway, the connection section shows the reason —
/// is what these tests pin.
///
/// Shares `KEYRING` and [`KeyringGuard`] with [`mock_keyring`]: there is one process-wide slot,
/// so the two must not be installable at the same time.
pub async fn mock_keyring_broken() -> KeyringGuard {
    let lock = KEYRING.lock().await;
    *crate::secret::fake() = Some(crate::secret::Fake::Broken(BROKEN_KEYRING.to_owned()));
    KeyringGuard { _lock: lock }
}

/// What the fake keyring holds, for assertions.
///
/// # Panics
///
/// Outside a [`mock_keyring`] guard — reading the real keyring from a test is exactly what this
/// helper exists to make impossible — and under a [`mock_keyring_broken`] one, which holds no DSN
/// to report and answers every call with an error instead.
pub fn fake_qdrant_dsn() -> Option<String> {
    match crate::secret::fake()
        .clone()
        .expect("fake keyring not installed; take a `mock_keyring()` guard first")
    {
        crate::secret::Fake::Slot(slots) => slots.qdrant_url,
        crate::secret::Fake::Broken(_) => {
            panic!("the fake keyring is broken; it holds no DSN to read")
        }
    }
}

/// Returns the stored fake Postgres DSN.
pub fn fake_dsn() -> Option<String> {
    match crate::secret::fake()
        .clone()
        .expect("fake keyring not installed; take a `mock_keyring()` guard first")
    {
        crate::secret::Fake::Slot(slots) => slots.pg,
        crate::secret::Fake::Broken(_) => {
            panic!("the fake keyring is broken; it holds no DSN to read")
        }
    }
}

/// What the fake keyring holds for the Infisical identity, `(client_id, client_secret)`, for
/// assertions.
///
/// # Panics
///
/// Outside a [`mock_keyring`] guard, and under a [`mock_keyring_broken`] one (as
/// [`fake_qdrant_dsn`]).
pub fn fake_machine_identity() -> (Option<String>, Option<String>) {
    match crate::secret::fake()
        .clone()
        .expect("fake keyring not installed; take a `mock_keyring()` guard first")
    {
        crate::secret::Fake::Slot(slots) => {
            (slots.infisical_client_id, slots.infisical_client_secret)
        }
        crate::secret::Fake::Broken(_) => {
            panic!("the fake keyring is broken; it holds no identity to read")
        }
    }
}

/// Makes the installed fake refuse every **store** to `user` (one of the `INFISICAL_*_USER`
/// constants) until the guard drops. Used to prove `set_machine_identity` leaves no half.
///
/// # Panics
///
/// Outside a [`mock_keyring`] guard, and under a [`mock_keyring_broken`] one.
pub fn refuse_fake_store(user: &'static str) {
    match &mut *crate::secret::fake() {
        Some(crate::secret::Fake::Slot(slots)) => slots.refuse_store = Some(user),
        Some(crate::secret::Fake::Broken(_)) => {
            panic!("the fake keyring is broken; it already refuses every store")
        }
        None => panic!("fake keyring not installed; take a `mock_keyring()` guard first"),
    }
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
/// `workspace_project`, `project` and `agent` - everything a chat resolves before it can start
/// (`docs/ANA-4.md` §4.1: the box, the user, the registry row, the project it belongs to). Items,
/// runs and events are the cursor-driven tables, and there is no offline *write* path (MOD-25).
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
                                  settings, created_by, created_at, updated_at, type, \
                                  owner_workspace_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(&project.project_type)
        .bind(project.owner_workspace_id.map(|id| id.to_string()))
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
