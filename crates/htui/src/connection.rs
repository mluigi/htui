//! The connection section's read, and the names of the three writes behind it (MOD-15 milestone 6,
//! D4, D9, D10).
//!
//! [`snapshot`] is the read; [`serve`] is the [`try_serve`](crate::store_worker) face of it, which
//! has no loop and therefore no memory of the last dial and no `--offline` flag — the worker's own
//! `ConnectionInfo` arm hands both in (blueprint flag K). The three **writers are not here**: they
//! rewire the worker's loop state (`backend`, `refresher`, `health`, `held`, `reconnect`) and live
//! beside it, exactly as `StoreState` and `ApplyMigrations` do.
//!
//! Nothing in this module can produce a DSN. The keyring read answers a `String` that never leaves
//! the function it was read in: it goes into [`htui_store::Dsn::parse`] and what comes out is a
//! summary with no password in it, because the type the summary is built from has no getter for
//! one. A stored text that does not parse fails **closed** — `dsn_summary: None` — rather than
//! being rendered raw (blueprint B-7).

use chrono::{DateTime, Utc};
use htui_core::store::{Result, StoreError};
use htui_store::{Backend, ConnectContext, Dsn, secret};
use zeroize::Zeroizing;

use crate::store_worker::{StoreReply, StoreRequest};

/// The connection as the section shows it. Never the DSN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSnapshot {
    /// [`Backend::label`] at the time of the read: `memory`, `online`, `connecting` or
    /// `offline · <age>`. The section renders this string and never parses it (D15).
    pub label: String,
    /// What the keyring answered, including the answer "nothing at all" (see [`DsnState`]).
    pub dsn_state: DsnState,
    /// [`Dsn::summary`] of the stored DSN; `None` when nothing is stored **and** when the stored
    /// text does not pass [`Dsn::parse`] — one `--set-dsn` may have written it raw (B-7).
    pub dsn_summary: Option<String>,
    /// The mirror's `cache_meta` and nothing else; `None` on [`Backend::Memory`].
    pub mirror: Option<MirrorInfo>,
    /// The last dial this session: `None` before the first one, after a `SetDsn` has forgotten the
    /// previous server's, and always from [`serve`], which has no loop to remember it (flag K).
    pub last_attempt: Option<Attempt>,
    /// `--offline` (D12): the DSN is stored and the mirror re-opened, but nothing dials until the
    /// next launch. `false` whenever the read has no context to ask.
    pub offline: bool,
}

/// What the keyring said when the snapshot was taken.
///
/// Four states rather than the `Option<bool>` this started as, because a keyring has **two** ways
/// of not holding a DSN and they are not the same fact. An entry that is not there is a first
/// launch. A store that cannot be opened — a locked collection, a session with no secret service,
/// which is what most CI images and a minimal window manager are — knows nothing either way, and
/// reporting it as [`Self::NotStored`] would tell a user with a perfectly good stored DSN that
/// they have none, and invite them to retype a credential into a store that cannot hold it.
///
/// [`htui_store::secret::get_dsn`] keeps the distinction at the seam: it maps only `NoEntry` and a
/// blank entry to `Ok(None)`, and every other failure stays an `Err`. This enum is where that
/// `Err` stops being fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DsnState {
    /// [`Backend::Memory`]: no keyring is consulted at all, so there is nothing to report (D10).
    NotApplicable,
    /// A DSN is in the keyring. [`ConnectionSnapshot::dsn_summary`] carries its redacted summary,
    /// or `None` when this build's parser refuses the stored text (B-7).
    Stored,
    /// The keyring answered, and it holds nothing: a first launch, or a `--clear-dsn`.
    NotStored,
    /// The keyring could not be read, and the seam's own sentence says why.
    ///
    /// Never a DSN: the text is [`htui_store::secret`]'s `cannot read the keyring entry
    /// (<service>/<user>): <platform>`, produced on the path where nothing was read.
    Unreadable(String),
}

impl DsnState {
    /// Whether there is a stored DSN to replace or to clear.
    #[must_use]
    pub const fn is_stored(&self) -> bool {
        matches!(self, Self::Stored)
    }
}

/// `CacheMeta` as the Mirror row reads it.
///
/// A copy rather than the store type so the section cannot reach a pool through it: this crate's
/// render side holds no store handle (`R-NF-3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorInfo {
    /// `sha256(host:port/dbname)` of the server this mirror belongs to, and the directory name
    /// under `<config_root>/cache/`. Credentials do not change it (`identity.rs:132-143`).
    pub db_fingerprint: String,
    /// The Postgres migration version the mirror was built from.
    pub schema_version: i64,
    /// When the file was created or last recreated. A `RebuildCache` keeps it (D14).
    pub built_at: DateTime<Utc>,
    /// When the last cursor-at-zero pass finished; `None` until one has, and again after a
    /// `RebuildCache`.
    pub last_full_refresh_at: Option<DateTime<Utc>>,
}

/// One dial and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// When the worker learned the outcome, not when the dial started.
    pub at: DateTime<Utc>,
    /// How it ended.
    pub outcome: AttemptOutcome,
}

/// How a dial ended: `ConnEvent` without its `PgStore` (B-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// Connected, schema up to date.
    Online,
    /// Connected, with `n` embedded migrations not applied; the store is held aside.
    MigrationsPending(usize),
    /// The dial failed. The text is `ConnEvent::Failed`'s — sqlx's connection error, which is what
    /// the worker already logs — and never the DSN.
    Failed(String),
}

/// The read (D4).
///
/// The keyring is platform FFI and blocks, so it goes through `spawn_blocking` (`R-NF-3`,
/// ANA-10 §4.9 (7)); on [`Backend::Memory`] it is not consulted at all, which is what keeps a
/// `--demo` snapshot and every `Harness` test out of the developer's credential store (D10).
///
/// `attempt` and `context` are the worker loop's memory. [`serve`] has neither and passes `None`
/// for both, so the same read is available to a build with no loop — with two fields honestly
/// empty rather than guessed at (flag K, open item O-4).
///
/// A keyring that **refuses to answer** is a [`DsnState::Unreadable`] row rather than an error:
/// the other three rows — the label, the mirror, the last dial — are all still known, and
/// replacing them with one sentence because the keyring could not be opened would lose more than
/// it reports. It is not flattened into [`DsnState::NotStored`] anywhere: see [`DsnState`].
///
/// # Errors
///
/// [`StoreError::Backend`] when the keyring's blocking task fails to *join* — a failure of this
/// process, not of the keyring — and whatever `CacheStore::meta` reports. Neither a *missing* DSN
/// nor an unreadable keyring is an error.
pub async fn snapshot(
    backend: &Backend,
    attempt: Option<&Attempt>,
    context: Option<&ConnectContext>,
) -> Result<ConnectionSnapshot> {
    let label = backend.label();
    let last_attempt = attempt.cloned();

    if matches!(backend, Backend::Memory(_)) {
        return Ok(ConnectionSnapshot {
            label,
            dsn_state: DsnState::NotApplicable,
            dsn_summary: None,
            mirror: None,
            last_attempt,
            offline: false,
        });
    }

    let read = tokio::task::spawn_blocking(secret::get_dsn)
        .await
        .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))?;
    // The one place a raw stored text meets the newtype, and the only reader of it: `summary`
    // carries no password because `PgConnectOptions` has no getter for one.
    let (dsn_state, dsn_summary) = match read.map(|stored| stored.map(Zeroizing::new)) {
        Ok(None) => (DsnState::NotStored, None),
        Ok(Some(text)) => (
            DsnState::Stored,
            Dsn::parse(&text).ok().map(|dsn| dsn.summary()),
        ),
        // The seam's sentence without `StoreError`'s own prefix: this is rendered in a row, and
        // `store backend error: ` is the shell's wording for a store failure, which the section
        // already argues this is not (see `serve` below).
        Err(err) => (DsnState::Unreadable(seam_sentence(&err)), None),
    };

    let mirror = match backend.cache() {
        Some(cache) => {
            let meta = cache.meta().await?;
            Some(MirrorInfo {
                db_fingerprint: meta.db_fingerprint,
                schema_version: meta.schema_version,
                built_at: meta.built_at,
                last_full_refresh_at: meta.last_full_refresh_at,
            })
        }
        None => None,
    };

    Ok(ConnectionSnapshot {
        label,
        dsn_state,
        dsn_summary,
        mirror,
        last_attempt,
        offline: context.is_some_and(|context| context.offline),
    })
}

/// The seam's own sentence, without the `StoreError` wrapper the shell puts on a store failure.
///
/// [`StoreError::Backend`]'s `Display` is `store backend error: {0}`, which is the status line's
/// vocabulary for a store that failed. A keyring the section is *reporting on* is a row, so the
/// row shows what `htui_store::secret` wrote and nothing else. Every other variant is rendered
/// whole, because only `Backend` has a prefix worth dropping.
fn seam_sentence(err: &StoreError) -> String {
    match err {
        StoreError::Backend(message) => message.clone(),
        other => other.to_string(),
    }
}

/// `try_serve`'s arm: the read without the loop's memory, and a refusal for each writer (D9).
///
/// The three writers answer [`StoreReply::Failed`] rather than an `Err`, which is the thirteen
/// runtime variants' shipped shape (`store_worker.rs:901-916`, "no agent runtime in this build")
/// and what D9 names: an `Err` would reach the section through `StoreError`'s `Display` and arrive
/// prefixed `store backend error: `, which is a store failure's sentence and this is not one.
/// Every one of the three needs at least two pieces of loop state that a free function over
/// `&Backend` cannot reach; that is a fact about the build, not a fault in the request.
///
/// # Errors
///
/// [`snapshot`]'s, and [`StoreError::Backend`] for a request that is not one of this module's —
/// which `try_serve` never sends here, so it names what it was given rather than panicking.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::ConnectionInfo => {
            Ok(StoreReply::Connection(snapshot(backend, None, None).await?))
        }
        StoreRequest::SetDsn(_) | StoreRequest::ClearDsn | StoreRequest::RebuildCache => {
            Ok(StoreReply::Failed {
                request: request.name(),
                message: NO_WORKER.to_owned(),
            })
        }
        // `try_serve` routes exactly this module's four variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a connection request: {}",
            other.name()
        ))),
    }
}

/// What a writer is refused with when there is no worker loop to rewire (D9).
///
/// A different sentence from [`DEMO_SESSION`] because they are different situations: this one is a
/// build — the test harness, `store_worker::serve` — that has no loop at all, and the tests tell
/// them apart by text (B-4).
pub const NO_WORKER: &str = "no connection worker in this build";

/// What a writer is refused with on [`Backend::Memory`] (D10, B-4).
pub const DEMO_SESSION: &str = "a demo session has no DSN to change";

/// Every request this module answers, in `StoreRequest` order.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a fifth
/// request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 4] = ["connection_info", "set_dsn", "clear_dsn", "rebuild_cache"];

/// The **read**'s name, the one of the four a `Failed` is treated differently for: a refused read
/// leaves the section with no snapshot at all, where a refused write leaves the rows it had.
///
/// Named rather than reached for as `REQUEST_NAMES[0]`, so a section's `Failed` arms say which
/// request they mean instead of relying on the order of the array above.
pub const READ_NAME: &str = REQUEST_NAMES[0];
