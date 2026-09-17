//! Startup and reconnect (plan D10, blueprint C.15, ANA-9 §4.4 "refresh sequence on connect").
//!
//! The rule this module exists for is §4.4's first line: **open the cache, render immediately,
//! connect off the UI thread**. [`start`] therefore does only local work — read `box.toml`, read
//! the keyring, open `cache.sqlite` — and hands back a [`Backend::Offline`] plus a channel; the
//! connection itself runs in a spawned task and reports exactly one [`ConnEvent`].
//!
//! The [`crate::cache::refresh::Refresher`] is **not** created here: it needs a [`PgStore`], which
//! does not exist until an event arrives, and a `watch::Receiver` the store worker owns. The
//! worker spawns it on the `Online` transition (blueprint H.14).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use htui_core::model::ProjectId;
use htui_core::store::{Result, StoreError};
use tokio::sync::{mpsc, watch};
use zeroize::Zeroizing;

use crate::backend::Backend;
use crate::cache::CacheStore;
use crate::cache::refresh::RefreshSettings;
use crate::dsn::Dsn;
use crate::error::map_sqlx;
use crate::identity::{self, Identity};
use crate::pg::{Connected, MigrationState, PgStore};
use crate::secret;

/// How often an offline worker retries (plan D10).
pub const RECONNECT: Duration = Duration::from_secs(30);

/// The cache directory name used when there is no DSN to fingerprint.
///
/// A box that has never run `htui --set-dsn` still gets a mirror, so `--offline` and a first
/// launch render the same empty shell instead of failing to open one.
pub const NO_DSN_FINGERPRINT: &str = "offline";

/// What [`ConnEvent::Failed`] says when the keyring holds no DSN.
pub const NO_DSN: &str = "no DSN stored; run `htui --set-dsn`";

/// `app_setting` keys [`refresh_settings`] reads.
const CACHE_REFRESH_SECONDS: &str = "cache_refresh_seconds";
/// The overlap window of ANA-9 §4.4.
const CACHE_OVERLAP_SECONDS: &str = "cache_overlap_seconds";

/// What the connect task reports back, exactly once per attempt.
#[derive(Debug)]
pub enum ConnEvent {
    /// Connected, schema up to date. The worker swaps in [`Backend::Online`] and spawns a
    /// refresher.
    Online(PgStore),
    /// Connected, but `n` embedded migrations are not applied (`R-STO-5`). The store is held aside
    /// until the user answers the migration prompt; nothing reads through a schema this binary has
    /// not finished writing.
    MigrationsPending(PgStore, usize),
    /// The attempt failed: no DSN, unreachable server, refused schema. The text is what the status
    /// line shows and what the log records.
    Failed(String),
}

/// One attempt's future, as the store worker's reconnect ticker spawns it.
pub type ConnFuture = Pin<Box<dyn Future<Output = ConnEvent> + Send>>;

/// Re-runs [`attempt`] with the arguments [`start`] was given.
///
/// A closure rather than the arguments themselves so the store worker never has to hold a DSN:
/// `R-STO-1` keeps the secret in the keyring and in the connect path, not in the shell.
pub type Reconnect = Arc<dyn Fn() -> ConnFuture + Send + Sync>;

/// What [`apply_dsn`] needs from the session [`start`] set up (MOD-15 M6 D20).
///
/// Carried on [`Started`] rather than re-derived in the store worker: calling
/// [`identity::config_root`] there would reach the developer's real configuration directory from
/// every test, and the worker has neither the root nor the timeout today.
#[derive(Debug, Clone)]
pub struct ConnectContext {
    /// The directory `cache/<fingerprint>/` is opened under.
    pub config_root: PathBuf,
    /// Passed to every dial.
    pub connect_timeout: Duration,
    /// `--offline`: the DSN is stored and the mirror re-opened, but nothing dials (D12).
    pub offline: bool,
}

/// What a stored DSN produced, for the worker to install (D1, D11).
///
/// Nothing here is the DSN: the text stayed inside [`apply_dsn`], and what comes back out is a
/// mirror, a closure that holds its own zeroizing copy, a directory name and a redacted summary.
pub struct Applied {
    /// The mirror for the new server, opened and migrated.
    pub cache: CacheStore,
    /// One dial per call, over the new DSN.
    pub reconnect: Reconnect,
    /// [`identity::db_fingerprint`] of the new DSN — the mirror's directory name.
    pub fingerprint: String,
    /// The redacted summary the section shows.
    pub summary: String,
}

impl core::fmt::Debug for Applied {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Applied")
            .field("cache", &self.cache)
            .field("fingerprint", &self.fingerprint)
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

/// How the shell wants to start.
#[derive(Debug, Clone)]
pub struct StartOptions {
    /// Overrides the keyring DSN. `None` in the binary; the tests set it.
    pub dsn: Option<String>,
    /// `--offline`: open the cache, never attempt a connection, no reconnect ticker.
    pub offline: bool,
    /// Where `box.toml` and `cache/` live: [`identity::config_root`] in the binary, a temporary
    /// directory under test, so a test never writes into the user's configuration.
    pub config_root: PathBuf,
    /// How long one attempt waits for a connection, [`crate::pg::CONNECT_TIMEOUT`] by default.
    ///
    /// Threaded down to [`PgStore::connect_with`]. The tests dial a port nothing listens on and
    /// shorten it, so an unreachable-server case answers in a second rather than in ten.
    pub connect_timeout: Duration,
}

impl StartOptions {
    /// Options that read the keyring and connect, rooted at `config_root`.
    #[must_use]
    pub const fn new(config_root: PathBuf) -> Self {
        Self {
            dsn: None,
            offline: false,
            config_root,
            connect_timeout: crate::pg::CONNECT_TIMEOUT,
        }
    }
}

/// Everything the store worker needs to own.
pub struct Started {
    /// The backend to move into the worker: [`Backend::Offline`] over the opened cache, always.
    pub backend: Backend,
    /// One [`ConnEvent`] per attempt. Empty and never written when `offline` is set.
    pub events: mpsc::Receiver<ConnEvent>,
    /// The other end of [`Started::events`], for the worker's own reconnect attempts. Held here so
    /// the receiver never closes while the worker lives.
    pub events_tx: mpsc::Sender<ConnEvent>,
    /// The scope the worker publishes on every `Items` request; the refresher reads it.
    pub projects: watch::Sender<Vec<ProjectId>>,
    /// Cursor-pass tuning the worker starts from. `this_box`, `this_user`, the interval and the
    /// overlap are filled in from the connected server by [`refresh_settings`]; what survives from
    /// here is `transcript_steps` and the fallbacks.
    pub settings: RefreshSettings,
    /// One reconnect attempt, or `None` with `--offline`, with `--demo` and when no DSN is stored.
    pub reconnect: Option<Reconnect>,
    /// What [`apply_dsn`] needs (D20). `None` from [`Started::detached`] and therefore under
    /// `--demo`, which is what makes `SetDsn` refuse there.
    pub connect: Option<ConnectContext>,
}

impl core::fmt::Debug for Started {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Started")
            .field("backend", &self.backend)
            .field("settings", &self.settings)
            .field("reconnect", &self.reconnect.is_some())
            .field("connect", &self.connect)
            .finish_non_exhaustive()
    }
}

impl Started {
    /// A backend that never connects: `--demo` and the tests (blueprint D.4 step 4).
    ///
    /// The event channel is created and immediately left unwritten, and there is no reconnect
    /// closure, so the store worker's two new `select!` arms never fire and it behaves exactly as
    /// it did in MOD-1.
    #[must_use]
    pub fn detached(backend: Backend) -> Self {
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
        let (projects, _) = watch::channel(Vec::new());
        Self {
            backend,
            events,
            events_tx,
            projects,
            settings: RefreshSettings::default(),
            reconnect: None,
            connect: None,
        }
    }
}

/// How many `ConnEvent`s may queue. One attempt is in flight at a time; the slack is for the
/// worker's own reconnects overlapping a `start` that has not been drained yet.
const EVENT_QUEUE: usize = 4;

/// Opens the cache and starts the connect task.
///
/// Never blocks on the network: the identity read, the keyring read, the cache open and the return
/// all happen on the caller's task, and only the connection itself is spawned (ANA-9 §4.4).
///
/// Steps: [`identity::load_or_mint`] → [`secret::get_dsn`] (unless `opts.dsn` overrides it) →
/// [`identity::db_fingerprint`] (or [`NO_DSN_FINGERPRINT`]) → [`CacheStore::open`] with
/// [`PgStore::schema_version`] → [`Backend::Offline`] → spawn [`attempt`] unless `offline` or no
/// DSN.
///
/// With `offline` or without a DSN the backend starts at `since: Some(now)`, so the top bar reads
/// `offline · 0s` rather than a `connecting` that will never resolve.
///
/// # Errors
///
/// [`htui_core::store::StoreError::Backend`] when `box.toml` cannot be read or minted, when the
/// keyring refuses to answer, or when the mirror cannot be opened. A *missing* DSN is not an
/// error: a first launch must still open offline (blueprint C.15).
pub async fn start(opts: StartOptions) -> Result<Started> {
    let root = opts.config_root.clone();
    // Minted here so the failure is reported to the caller rather than swallowed by the spawned
    // attempt, which re-reads the file per try to pick up an adopted id (plan D6).
    identity::load_or_mint(&root)?;
    // Zeroizing from the keyring read onwards (D3): the buffer the DSN lands in is wiped when it
    // goes, and `attempt` gets a fresh plain copy per dial that it consumes and drops.
    let dsn: Option<Zeroizing<String>> = match opts.dsn {
        Some(dsn) => Some(Zeroizing::new(dsn)),
        None => secret::get_dsn()?.map(Zeroizing::new),
    };

    let fingerprint = dsn.as_deref().map_or_else(
        || NO_DSN_FINGERPRINT.to_owned(),
        |text| identity::db_fingerprint(text),
    );
    let cache = CacheStore::open(&root, &fingerprint, PgStore::schema_version()).await?;

    let connecting = !opts.offline && dsn.is_some();
    let backend = Backend::Offline {
        cache,
        since: if connecting { None } else { Some(Utc::now()) },
    };

    let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
    let (projects, _) = watch::channel(Vec::new());
    let timeout = opts.connect_timeout;
    let reconnect: Option<Reconnect> = if connecting {
        Some(reconnect_over(dsn.clone(), root.clone(), timeout))
    } else {
        None
    };

    if connecting {
        let sender = events_tx.clone();
        tokio::spawn(async move {
            let event = attempt(dsn.as_deref().cloned(), root, timeout).await;
            // The worker is gone if this fails, and there is nobody left to tell.
            let _ = sender.send(event).await;
        });
    } else {
        tracing::info!(
            offline = opts.offline,
            "starting from the cache without connecting"
        );
    }

    Ok(Started {
        backend,
        events,
        events_tx,
        projects,
        settings: RefreshSettings::default(),
        reconnect,
        connect: Some(ConnectContext {
            config_root: opts.config_root,
            connect_timeout: timeout,
            offline: opts.offline,
        }),
    })
}

/// One connection attempt, spawned by [`start`] and re-run by the worker's 30 s ticker.
///
/// Never returns an error: every failure is a [`ConnEvent::Failed`] whose text the status line
/// shows, because "no DSN" and "server down" are states the shell renders rather than crashes on.
/// The identity is re-read from `root` per attempt, so an id adopted by an earlier attempt
/// (plan D6) is the one the next one registers under.
pub async fn attempt(dsn: Option<String>, root: PathBuf, connect_timeout: Duration) -> ConnEvent {
    let Some(dsn) = dsn else {
        return ConnEvent::Failed(NO_DSN.to_owned());
    };
    let identity = match identity::load_or_mint(&root) {
        Ok(identity) => identity,
        Err(err) => return ConnEvent::Failed(err.to_string()),
    };
    match try_connect(&dsn, &identity, &root, connect_timeout).await {
        Ok(Connected { store, migrations }) => match migrations {
            MigrationState::UpToDate => ConnEvent::Online(store),
            MigrationState::Pending(n) => ConnEvent::MigrationsPending(store, n),
        },
        Err(err) => ConnEvent::Failed(err.to_string()),
    }
}

/// One dial per call over `dsn`, copied into a plain `String` for the length of the dial only.
///
/// The `Arc` holds the text in a zeroizing buffer, so the closure the store worker keeps for the
/// life of the session does not park a bare `String` on the heap; [`attempt`]'s signature is
/// deliberately unchanged (blueprint flag C), because `--set-dsn` may have stored a DSN that
/// [`Dsn::parse`] would now refuse and M1's startup behaviour must not change.
fn reconnect_over(
    dsn: Option<Zeroizing<String>>,
    root: PathBuf,
    connect_timeout: Duration,
) -> Reconnect {
    Arc::new(move || {
        let dsn = dsn.as_deref().cloned();
        let root = root.clone();
        Box::pin(attempt(dsn, root, connect_timeout)) as ConnFuture
    })
}

/// [`reconnect_over`] for a validated DSN (D20): what the worker arms after a `SetDsn`.
#[must_use]
pub fn reconnect_for(dsn: &Dsn, root: PathBuf, connect_timeout: Duration) -> Reconnect {
    reconnect_over(
        Some(Zeroizing::new(dsn.as_str().to_owned())),
        root,
        connect_timeout,
    )
}

/// Stores `dsn` in the keyring and opens the mirror it names (D11 steps 2–3, 7).
///
/// The keyring write runs on the blocking pool with a clone of the `Dsn` moved into it — platform
/// FFI, and `R-NF-3` binds it even though it is not network I/O (ANA-10 §4.9 (6), (7)). The order
/// is the point: the keyring **first**, because a mirror opened for a DSN that was never stored is
/// a lie the next launch inherits.
///
/// Nothing here touches the running backend; the worker installs the result.
///
/// # Errors
///
/// The keyring's refusal, the blocking task's join failure, or [`CacheStore::open`]'s. On any of
/// them nothing was installed and the caller's backend is untouched — but a keyring write that
/// succeeded before `open` failed **stays written**, so the next launch uses the new DSN and a
/// re-read of the connection reports it as stored. That is the honest state, not a rollback.
pub async fn apply_dsn(dsn: Dsn, ctx: &ConnectContext) -> Result<Applied> {
    let stored = dsn.clone();
    tokio::task::spawn_blocking(move || secret::set_dsn(stored.as_str()))
        .await
        .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))??;

    let fingerprint = dsn.fingerprint();
    let cache = CacheStore::open(&ctx.config_root, &fingerprint, PgStore::schema_version()).await?;
    let reconnect = reconnect_for(&dsn, ctx.config_root.clone(), ctx.connect_timeout);
    let summary = dsn.summary();
    Ok(Applied {
        cache,
        reconnect,
        fingerprint,
        summary,
    })
}

/// Removes the keyring entry (D13). A missing entry is `Ok(())`, as [`secret::clear_dsn`] is.
///
/// # Errors
///
/// The keyring's refusal or the blocking task's join failure.
pub async fn forget_dsn() -> Result<()> {
    tokio::task::spawn_blocking(secret::clear_dsn)
        .await
        .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))?
}

/// Connects and persists an adopted box id (plan D6).
///
/// [`PgStore::connect`] does no file I/O of its own; when this hostname already had a `box` row
/// under another id, the store comes back carrying the database's id and this is where `box.toml`
/// learns about it.
///
/// # Errors
///
/// Whatever [`PgStore::connect`] reports — an unreachable server, a refused schema — and
/// [`htui_core::store::StoreError::Backend`] when the adopted id cannot be written back.
pub async fn try_connect(
    dsn: &str,
    identity: &Identity,
    root: &Path,
    connect_timeout: Duration,
) -> Result<Connected> {
    let connected = PgStore::connect_with(dsn, identity, connect_timeout).await?;
    let adopted = connected.store.identity();
    if adopted.box_id != identity.box_id {
        tracing::info!(
            adopted = %adopted.box_id.as_uuid(),
            minted  = %identity.box_id.as_uuid(),
            "this hostname already had a box row; adopting the database's id"
        );
        identity::store(root, adopted)?;
    }
    Ok(connected)
}

/// Fills in what only a connected server knows: this box, this user and the two cache settings.
///
/// `this_box` / `this_user` come from the store ([`PgStore::this_box`], [`PgStore::this_user`]);
/// the interval and the overlap come from `app_setting`, which [`PgStore::seed_if_empty`] seeds
/// with 30 s and 300 s. A missing, non-numeric or non-positive value keeps `base`'s value, and a
/// failed read is a `warn!` rather than a refusal to mirror at all.
pub async fn refresh_settings(pg: &PgStore, base: RefreshSettings) -> RefreshSettings {
    let mut settings = RefreshSettings {
        this_box: pg.this_box(),
        this_user: pg.this_user(),
        ..base
    };

    let rows = sqlx::query!(
        "SELECT key, value FROM app_setting \
         WHERE key IN ('cache_refresh_seconds', 'cache_overlap_seconds')",
    )
    .fetch_all(pg.pool())
    .await
    .map_err(map_sqlx);

    match rows {
        Ok(rows) => {
            for row in rows {
                let Some(seconds) = row.value.as_i64().filter(|n| *n > 0) else {
                    continue;
                };
                let seconds = Duration::from_secs(seconds.unsigned_abs());
                match row.key.as_str() {
                    CACHE_REFRESH_SECONDS => settings.interval = seconds,
                    CACHE_OVERLAP_SECONDS => settings.overlap = seconds,
                    _ => {}
                }
            }
        }
        Err(error) => tracing::warn!(%error, "cannot read app_setting; using the default tuning"),
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::{NO_DSN, StartOptions, attempt, start};
    use crate::backend::Backend;

    #[tokio::test]
    async fn offline_opens_the_cache_and_never_dials() {
        let root = tempfile::tempdir().expect("temp root");
        let started = start(StartOptions {
            dsn: Some("postgres://nobody@127.0.0.1:1/none".to_owned()),
            offline: true,
            ..StartOptions::new(root.path().to_owned())
        })
        .await
        .expect("start offline");

        assert!(matches!(started.backend, Backend::Offline { .. }));
        assert!(
            started.backend.label().starts_with("offline · "),
            "--offline never reads `connecting`: {}",
            started.backend.label()
        );
        assert!(started.reconnect.is_none(), "no ticker with --offline");
        assert!(
            root.path().join("box.toml").exists(),
            "the identity is minted under the throwaway root, not %APPDATA%"
        );
        if let Some(cache) = started.backend.cache() {
            cache.close().await;
        }
    }

    #[tokio::test]
    async fn no_dsn_is_a_failed_event_and_not_an_error() {
        let root = tempfile::tempdir().expect("temp root");
        match attempt(None, root.path().to_owned(), crate::pg::CONNECT_TIMEOUT).await {
            super::ConnEvent::Failed(why) => assert_eq!(why, NO_DSN),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
