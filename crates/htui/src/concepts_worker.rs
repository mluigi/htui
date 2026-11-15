//! The concepts index behind the TUI's search (MOD-64 D230-D232, D237, D238).
//!
//! `SearchConcepts` loads an embedding model and calls Qdrant; `IndexConcepts` reads every item of
//! the scope; `ProbeQdrant` (MOD-160 D9) asks Qdrant, which may take its 3 s bound when it is
//! down. None may run in the store worker's serial loop (`R-NF-3`): [`ConceptsRuntime`] serves
//! them on tasks of their own, the way `AgentRuntime::preview` serves a preview, and each task
//! sends its own reply. The index is an object-safe [`ConceptIndex`]: [`QdrantIndex`] in
//! production, `MemIndex` in tests.

use std::collections::{HashMap, HashSet};
use std::mem::Discriminant;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock, Weak};
use std::time::Duration;

use chrono::TimeDelta;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use htui_core::model::{BoxId, IndexedCollection, IndexerLease, LeaseTurn, Scope, WorkspaceId};
use htui_core::qdrant::{QdrantUrl, QdrantUrlError, StoredQdrantUrl};
use htui_core::store::{IndexerLeaseStore as _, MemStore, StoreError, take_or_request_pass};
use htui_orch::recover::LeaseTimes;
use htui_store::embed::{DENSE_DIM, RtenEmbedder};
use htui_store::model;
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::secret::{KeyHost, QdrantEnv, QdrantKeyError};
use htui_store::vector::{
    self, ExistingCollection, Hit, Probe, QdrantStore, SearchQuery, VectorStore as _,
};
use htui_store::vector_sync::{
    DeleteGuard, Indexer, SyncReport, Unguarded, workspace_search_scope,
};
use htui_store::{Backend, PgStore, secret};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest};

/// What both requests answer when nothing serves them: the test harness without a runtime (D231).
pub const NOT_AVAILABLE: &str = "concepts search is not available";

/// The three `StoreRequest::name`s this module serves, in that order (D241, MOD-160 D9).
pub const REQUEST_NAMES: [&str; 3] = ["search_concepts", "index_concepts", "probe_qdrant"];

/// One concepts answer, its error inside (D232, blueprint D242): a Qdrant failure is the overlay's
/// to show and never becomes `StoreReply::Failed`, which the shell also puts on the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum ConceptsReply {
    /// Answer to `SearchConcepts`. `query` and `scope_row_of` are the request's own, so a list is
    /// matched to the search it answers.
    Hits {
        /// The search this answers.
        query: SearchQuery,
        /// The request's `scope_row_of` (MOD-147 M4 R1 M1): a one-member workspace's "all
        /// projects" search and its member's alone send the same query and differ only here.
        scope_row_of: Option<WorkspaceId>,
        /// The hits, best first, or why there are none.
        outcome: Result<Vec<Hit>, String>,
    },
    /// Answer to `IndexConcepts`: what the run did, or why it stopped.
    Indexed(Result<IndexOutcome, String>),
}

/// What a `Ctrl+R` run did (MOD-159 M1 plan D12): it indexes only under its workspace
/// collection's lease, so a lease another process holds is an answer, not an error.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexOutcome {
    /// The lease was free or this runtime's: the run synced.
    Synced(SyncReport),
    /// Another process holds the lease and indexes this collection; nothing was synced here and
    /// no pass request could be recorded (it let go and was taken again at every attempt).
    HeldBy(IndexerLease),
    /// Another process holds the lease live and was asked for a pass (MOD-159 M3 plan D4, D6):
    /// the holder's index job, or the next job to take the lease (a `Ctrl+R` press or
    /// `--index-items` never claims it), reconciles the collection in full at its next beat;
    /// nothing was synced here.
    Requested(IndexerLease),
}

/// `Ctrl+R`'s answer on a box with no `box` row of its own (MOD-159 M1 plan D12): a lease names
/// the box that holds it.
pub const NO_BOX_IDENTITY: &str = "knowledge index unavailable: this box has no identity";

/// Which collections a search reads (MOD-159 M1 plan D9). The global collection is read in
/// both cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchIn {
    /// These workspaces' collections, plus the global one: the TUI's and the CLI's searches.
    Workspaces(Vec<WorkspaceId>),
    /// Every workspace collection the index holds, plus the global one: MCP `search_concepts`,
    /// whose project filter alone decides what it sees.
    Everywhere,
}

/// D9: several collections' hits as one list: best score first, a hit two collections hold (a
/// project in two workspaces) kept once, at its best score, and cut to `limit`. Ties keep the
/// order of `lists`.
#[must_use]
pub fn merge_hits(lists: Vec<Vec<Hit>>, limit: u64) -> Vec<Hit> {
    let mut hits: Vec<Hit> = lists.into_iter().flatten().collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen = HashSet::new();
    hits.retain(|hit| {
        seen.insert((
            hit.point_type,
            hit.owner,
            hit.document.clone(),
            hit.snippet.clone(),
        ))
    });
    hits.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    hits
}

/// D9, MOD-159 M1 R2: each searched collection's answer (a missing one has none) as one. A
/// collection that failed (a stale one, another embedder's) is logged and skipped while another
/// answers, so one bad `htui_ws_*` does not fail every search. A failure is the answer only when
/// no workspace collection answered: the global one, empty on most boxes, never stands in for a
/// workspace (`concepts::combine`'s rule for the CLI).
fn partial_answer(
    answers: Vec<(String, Result<Vec<Hit>, String>)>,
    limit: u64,
) -> Result<Vec<Hit>, String> {
    let mut lists = Vec::with_capacity(answers.len());
    let mut failed = Vec::new();
    let mut a_workspace_answered = false;
    for (collection, answer) in answers {
        match answer {
            Ok(hits) => {
                a_workspace_answered |= matches!(
                    IndexedCollection::parse(&collection),
                    Some(IndexedCollection::Workspace(_))
                );
                lists.push(hits);
            }
            Err(err) => failed.push((collection, err)),
        }
    }
    if !a_workspace_answered && let Some((_, first)) = failed.first() {
        return Err(first.clone());
    }
    for (collection, err) in failed {
        tracing::warn!(collection, %err, "concepts search skipped a collection that failed");
    }
    Ok(merge_hits(lists, limit))
}

/// [`DeleteGuard`] made object-safe for [`ConceptIndex::sync`] (MOD-159 M1 plan D6): asked
/// before each project's one delete batch; `false` stops the sync there, nothing deleted.
/// [`Unguarded`] always answers `true`; [`LeaseRenewal`] renews a collection's lease.
pub trait DeleteGuardDyn: Send + Sync {
    /// Whether the sync may delete now.
    fn before_delete(&self) -> BoxFuture<'_, Result<bool, StoreError>>;
}

impl DeleteGuardDyn for Unguarded {
    fn before_delete(&self) -> BoxFuture<'_, Result<bool, StoreError>> {
        Box::pin(async { Ok(true) })
    }
}

/// A [`DeleteGuardDyn`] as the [`DeleteGuard`] `Indexer::sync` takes.
struct Guarded<'a>(&'a dyn DeleteGuardDyn);

impl DeleteGuard for Guarded<'_> {
    async fn before_delete(&self) -> Result<bool, StoreError> {
        self.0.before_delete().await
    }
}

/// D6's guard under a lease: renews `collection`'s lease for `owner` before each delete batch,
/// and stops the sync when the renewal touches nothing (another owner took it).
#[derive(Debug)]
pub struct LeaseRenewal {
    /// The store the lease lives in.
    pub backend: Backend,
    /// The collection whose lease is renewed.
    pub collection: IndexedCollection,
    /// The holder's token.
    pub owner: Uuid,
    /// The TTL each renewal stamps.
    pub ttl: TimeDelta,
}

impl DeleteGuardDyn for LeaseRenewal {
    fn before_delete(&self) -> BoxFuture<'_, Result<bool, StoreError>> {
        Box::pin(
            self.backend
                .renew_indexer_lease(&self.collection, self.owner, self.ttl),
        )
    }
}

/// The index seam (D230): object-safe, where `VectorStore` (`async fn`s) is not. Errors are display
/// strings: they are only ever shown.
pub trait ConceptIndex: Send + Sync {
    /// Hybrid search of the collections `within` names (MOD-159 M1 plan D9), scoped to
    /// `query.projects`: a missing collection adds nothing, and the hits are [`merge_hits`]'
    /// merge.
    fn search(
        &self,
        within: SearchIn,
        query: SearchQuery,
    ) -> BoxFuture<'_, Result<Vec<Hit>, String>>;
    /// `Indexer::sync` of every project of `scope` into `collection`, reading through `backend`,
    /// asking `guard` before each delete batch (D6). Takes no lease: the caller holds it.
    fn sync(
        &self,
        backend: Backend,
        collection: IndexedCollection,
        scope: Scope,
        guard: Box<dyn DeleteGuardDyn>,
    ) -> BoxFuture<'_, Result<SyncReport, String>>;
    /// MOD-158 D6: the store the `qdrant_url` row is read through from now on (`None`: offline,
    /// `--demo`, or not connected yet). An index that reads no settings ignores it.
    fn publish(&self, _store: Option<Arc<dyn UrlRow>>) {}
    /// MOD-160 D5: whether `search_concepts` is offered to a session opened now. Read once per
    /// session at `McpHost::open`, on whatever thread opens it (the store loop for a chat), so it
    /// awaits nothing. An index that reads no settings is always offered.
    fn offered(&self) -> bool {
        true
    }
    /// MOD-160 D5: this process wrote or cleared the `qdrant_url` row; the offer is read again.
    fn url_changed(&self) {}
    /// MOD-160 D9: a model-free look at the Qdrant this index searches, for Settings › Qdrant and
    /// the TUI's launch notice. `None`: nothing was probed (offline, or an index that reads no
    /// settings).
    fn check(&self) -> BoxFuture<'_, Option<QdrantCheck>> {
        Box::pin(async { None })
    }
}

/// "Postgres is not connected" for the Qdrant URL (MOD-158 D6): no store is published (offline,
/// `--demo`, or before the first connect) and no `HTUI_QDRANT_URL` override is set.
pub const URL_OFFLINE: &str = "the Qdrant URL cannot be read: Postgres is not connected";

/// D3's sentence for an override that is not a Qdrant URL. Never the value: it may hold a password.
pub(crate) fn env_url_unusable(why: QdrantUrlError) -> String {
    format!(
        "{} is not a usable Qdrant URL: {why}",
        htui_core::secret::QDRANT_URL_ENV
    )
}

/// The `HTUI_QDRANT_URL` override as a `Debug` may show it: the parsed URL, or a placeholder for
/// one that does not parse (a refused URL may hold a password, so its text is never shown).
pub(crate) fn shown_env_url(env: &QdrantEnv) -> Option<String> {
    env.url.as_deref().map(|text| {
        QdrantUrl::parse(text)
            .map_or_else(|_| "<not a usable URL>".to_owned(), |url| url.to_string())
    })
}

/// The keyring read of the Qdrant API key: [`secret::get_qdrant_api_key`] in production, a
/// closure in a test, so no case reads a keyring.
pub type KeyringRead = fn() -> htui_core::store::Result<Option<String>>;

/// MOD-158 D6: what the `qdrant_url` row is read through: the store this process is connected to.
pub trait UrlRow: Send + Sync {
    /// The row, malformed or not; `None` when there is none.
    fn qdrant_url(&self) -> BoxFuture<'_, Result<Option<StoredQdrantUrl>, StoreError>>;
}

impl UrlRow for PgStore {
    fn qdrant_url(&self) -> BoxFuture<'_, Result<Option<StoredQdrantUrl>, StoreError>> {
        Box::pin(Self::qdrant_url(self))
    }
}

/// The tests' store, and `--demo`'s should it ever publish one.
impl UrlRow for MemStore {
    fn qdrant_url(&self) -> BoxFuture<'_, Result<Option<StoredQdrantUrl>, StoreError>> {
        Box::pin(Self::qdrant_url(self))
    }
}

/// MOD-158 D4 for this process, owned (the borrowed form is [`KeyHost`]).
#[derive(Clone)]
pub enum KeySource {
    /// The TUI and the CLI verbs: this box's keyring, read per request.
    Interactive,
    /// `htui worker`: its `--qdrant-key-stdin` line when given, else the keyring, else the systemd
    /// credential `htui-qdrant-key`.
    Headless {
        /// The line read at worker start, exactly when `--qdrant-key-stdin` was given.
        stdin: Option<Zeroizing<String>>,
        /// `$CREDENTIALS_DIRECTORY`, as the worker read it at start.
        credentials_dir: Option<PathBuf>,
    },
}

impl KeySource {
    /// The borrowed form `secret::qdrant_api_key` takes.
    fn host(&self) -> KeyHost<'_> {
        match self {
            Self::Interactive => KeyHost::Interactive,
            Self::Headless {
                stdin,
                credentials_dir,
            } => KeyHost::Headless {
                stdin: stdin.as_deref().map(String::as_str),
                credentials_dir: credentials_dir.as_deref(),
            },
        }
    }
}

impl core::fmt::Debug for KeySource {
    /// [`KeyHost`]'s: whether stdin is a source, never what it holds.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&self.host(), f)
    }
}

/// Why no Qdrant settings resolved (MOD-158 D3, D4). [`Self::sentence`] is what a search, an index
/// run or a log line says.
#[derive(Debug, Clone)]
pub enum Unresolved {
    /// No override and no row: `NO_QDRANT_URL`.
    NoUrl,
    /// No store published and no override: [`URL_OFFLINE`].
    Offline,
    /// The override or the row is not a usable URL: its sentence, which names where it came from.
    Unusable(String),
    /// The row could not be read.
    Read(StoreError),
    /// The key could not be read.
    Key(QdrantKeyError),
}

impl Unresolved {
    /// The sentence shown for it.
    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            Self::NoUrl => crate::concepts::NO_QDRANT_URL.to_owned(),
            Self::Offline => URL_OFFLINE.to_owned(),
            Self::Unusable(sentence) => sentence.clone(),
            Self::Read(err) => format!("the Qdrant URL setting could not be read: {err}"),
            Self::Key(err) => err.to_string(),
        }
    }
}

impl core::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.sentence())
    }
}

/// D3 alone: the override (an invalid one is an error, never a fallback); else the row, through
/// [`StoredQdrantUrl::usable`] (a malformed row is never "no URL"); else [`Unresolved::NoUrl`].
/// `row` is `None` when no store is published: [`Unresolved::Offline`], unless the override is set.
async fn resolve_url(env: &QdrantEnv, row: Option<&dyn UrlRow>) -> Result<QdrantUrl, Unresolved> {
    if let Some(text) = &env.url {
        return QdrantUrl::parse(text).map_err(|why| Unresolved::Unusable(env_url_unusable(why)));
    }
    let Some(row) = row else {
        return Err(Unresolved::Offline);
    };
    match row.qdrant_url().await {
        Err(err) => Err(Unresolved::Read(err)),
        Ok(None) => Err(Unresolved::NoUrl),
        Ok(Some(stored)) => stored.usable().cloned().map_err(|err| match err {
            StoreError::Backend(sentence) => Unresolved::Unusable(sentence),
            other => Unresolved::Unusable(other.to_string()),
        }),
    }
}

/// D4: `HTUI_QDRANT_API_KEY`, then `key`'s sources. A source that may touch the keyring or a file
/// is read on the blocking pool (`QdrantSnapshot::fetch`'s rule), never on an async worker.
async fn resolve_key(
    env: &QdrantEnv,
    key: &KeySource,
    keyring: KeyringRead,
) -> Result<Option<Zeroizing<String>>, Unresolved> {
    let env_key = env.api_key.clone();
    let key = key.clone();
    let read = move || {
        secret::qdrant_api_key(env_key.as_deref().map(String::as_str), key.host(), &keyring)
    };
    tokio::task::spawn_blocking(read)
        .await
        .map_err(|err| {
            Unresolved::Key(QdrantKeyError::Keyring(format!(
                "the key read stopped: {err}"
            )))
        })?
        .map_err(Unresolved::Key)
}

/// MOD-158 D3, D4, the one resolution every Qdrant user goes through (the TUI's index, the worker
/// job, `--search-items`, `--index-items`): the URL first, so a box without one reads no key.
pub(crate) async fn resolve_settings(
    env: &QdrantEnv,
    row: Option<&dyn UrlRow>,
    key: &KeySource,
    keyring: KeyringRead,
) -> Result<QdrantSettings, Unresolved> {
    let url = resolve_url(env, row).await?;
    let api_key = resolve_key(env, key, keyring).await?;
    Ok(QdrantSettings { url, api_key })
}

/// MOD-160 D5: how old a known offer may be before [`QdrantSource::offered`] asks again.
pub(crate) const OFFER_STALE_AFTER: Duration = Duration::from_secs(30);

/// MOD-160 D5: whether a Qdrant URL resolves, as last read. Shared with the spawned reads (`Arc`),
/// so a read needs no `&QdrantSource`.
#[derive(Debug, Default)]
struct Configured {
    /// `None` until first known; `tokio::time::Instant`, so a paused clock ages it.
    known: Mutex<Option<(bool, tokio::time::Instant)>>,
    /// One staleness re-read at a time (`offered()`'s single flight). A drop guard in the task
    /// clears it, so an aborted task cannot leave it set.
    reading: AtomicBool,
    /// Bumped by `publish` and `reread`. A read records only if no bump happened since it began,
    /// so a read of the old store never overwrites the new store's answer.
    generation: AtomicU64,
}

impl Configured {
    /// Known at once, with no read.
    fn known(configured: bool) -> Self {
        Self {
            known: Mutex::new(Some((configured, tokio::time::Instant::now()))),
            ..Self::default()
        }
    }

    /// The generation a read starting now belongs to.
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// A new store or a new row: every read begun before is out of date. Under the `known` lock,
    /// so no read can check the old generation and then record after the bump.
    fn bump(&self) -> u64 {
        let _known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Records `configured` for a read begun under `generation`, unless a bump came since.
    fn record(&self, generation: u64, configured: bool) {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        if self.generation.load(Ordering::SeqCst) == generation {
            *known = Some((configured, tokio::time::Instant::now()));
        }
    }
}

/// Clears [`Configured::reading`] when the staleness read ends, however it ends.
struct Reading(Arc<Configured>);

impl Drop for Reading {
    fn drop(&mut self) {
        self.0.reading.store(false, Ordering::SeqCst);
    }
}

/// MOD-158 D6: one process's source of Qdrant settings. The slot holds the current store (`None`
/// offline, under `--demo`, or before a connect); every read takes an `Arc` clone and drops the
/// lock before any `.await`, and a publish is a pointer swap with no `.await` (`R-NF-3`).
///
/// MOD-160 D5: it also caches whether a URL resolves ([`Self::offered`]), refreshed by spawned
/// reads and by every resolution already in flight, never awaited by a reader.
pub struct QdrantSource {
    /// `HTUI_QDRANT_URL` and `HTUI_QDRANT_API_KEY`, as this process read them.
    env: QdrantEnv,
    /// The store the `qdrant_url` row is read through.
    store: RwLock<Option<Arc<dyn UrlRow>>>,
    /// Where the key comes from after the environment.
    key: RwLock<KeySource>,
    /// The keyring read of the key.
    keyring: KeyringRead,
    /// Whether a URL resolves, as last read (D5).
    configured: Arc<Configured>,
}

impl core::fmt::Debug for QdrantSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Not `QdrantEnv`'s own `Debug`: that prints the override unvalidated.
        f.debug_struct("QdrantSource")
            .field("env_url", &shown_env_url(&self.env))
            .field("env_key", &self.env.api_key.as_ref().map(|_| "<redacted>"))
            .field("published", &self.current().is_some())
            .field(
                "key",
                &*self.key.read().unwrap_or_else(PoisonError::into_inner),
            )
            .finish_non_exhaustive()
    }
}

impl QdrantSource {
    /// A source over `env` and `key`, nothing published, reading the keyring through
    /// [`secret::get_qdrant_api_key`]. An override in force is logged once, here, at `info` (D3);
    /// an unusable one at `warn`, without its value.
    #[must_use]
    pub fn new(env: QdrantEnv, key: KeySource) -> Self {
        if let Some(text) = &env.url {
            // Never the text of an invalid one: a refused URL may hold a password.
            match QdrantUrl::parse(text) {
                Ok(url) => tracing::info!(
                    %url,
                    "{} overrides the qdrant_url setting in this process",
                    htui_core::secret::QDRANT_URL_ENV
                ),
                // A `warn`: every search and sync will refuse until it is fixed (MOD-158 R1).
                Err(why) => tracing::warn!("{}", env_url_unusable(why)),
            }
        }
        // D5: the override wins over every row, so its verdict is known without a read.
        let configured = env.url.as_deref().map_or_else(Configured::default, |text| {
            Configured::known(QdrantUrl::parse(text).is_ok())
        });
        Self {
            env,
            store: RwLock::new(None),
            key: RwLock::new(key),
            keyring: secret::get_qdrant_api_key,
            configured: Arc::new(configured),
        }
    }

    /// The same source, reading the key's keyring through `keyring` (a test's closure).
    #[must_use]
    pub fn with_keyring(self, keyring: KeyringRead) -> Self {
        Self { keyring, ..self }
    }

    /// This process's environment, the interactive key: what a production entry point builds
    /// (blueprint P-13: never a test).
    #[must_use]
    pub fn from_process() -> Self {
        Self::new(QdrantEnv::from_process(), KeySource::Interactive)
    }

    /// The environment this source was built over.
    #[must_use]
    pub const fn env(&self) -> &QdrantEnv {
        &self.env
    }

    /// The store the row is read through from now on, then the offer (MOD-160 D5): `None` records
    /// at once with no task (the override's verdict, else `false`: offline, `--demo`); `Some`
    /// starts one read of the new store.
    pub fn publish(&self, store: Option<Arc<dyn UrlRow>>) {
        let read = store.is_some();
        *self.store.write().unwrap_or_else(PoisonError::into_inner) = store;
        let generation = self.configured.bump();
        if read {
            self.spawn_read(true);
        } else {
            let verdict = self
                .env
                .url
                .as_deref()
                .is_some_and(|text| QdrantUrl::parse(text).is_ok());
            self.configured.record(generation, verdict);
        }
    }

    /// MOD-160 D5: this process wrote or cleared the URL row: the offer is read again, now.
    pub fn reread(&self) {
        self.configured.bump();
        self.spawn_read(true);
    }

    /// MOD-160 D5: whether a Qdrant URL resolves, as last read; never awaited. Unknown answers
    /// `false` and starts a read. Older than `OFFER_STALE_AFTER`, it answers the old value and
    /// starts at most one read.
    #[must_use]
    pub fn offered(&self) -> bool {
        let known = *self
            .configured
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match known {
            None => {
                self.spawn_read(false);
                false
            }
            Some((offered, at)) => {
                if at.elapsed() >= OFFER_STALE_AFTER {
                    self.spawn_read(false);
                }
                offered
            }
        }
    }

    /// The read behind `publish`, `reread` and `offered`, on a task of its own: the URL alone
    /// (never the key), over the current store, recorded under the generation it began in. With
    /// no runtime it starts nothing and the flag stays as it was. Unforced, it is the single
    /// flight's: none starts while another runs.
    fn spawn_read(&self, forced: bool) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let guard = if forced {
            None
        } else if self
            .configured
            .reading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(Reading(Arc::clone(&self.configured)))
        } else {
            return;
        };
        let generation = self.configured.generation();
        let env = QdrantEnv {
            url: self.env.url.clone(),
            api_key: None,
        };
        let store = self.current();
        let configured = Arc::clone(&self.configured);
        runtime.spawn(async move {
            let _guard = guard;
            let resolved = resolve_url(&env, store.as_deref()).await;
            if let Err(why) = &resolved {
                tracing::debug!(%why, "search_concepts is not offered");
            }
            configured.record(generation, resolved.is_ok());
        });
    }

    /// Where the key comes from from now on (`htui worker`: [`KeySource::Headless`]).
    pub fn set_key_source(&self, key: KeySource) {
        *self.key.write().unwrap_or_else(PoisonError::into_inner) = key;
    }

    /// The published store, its lock already dropped.
    fn current(&self) -> Option<Arc<dyn UrlRow>> {
        self.store
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// D3 alone: the worker's `search_concepts` gate (D7, Q3).
    ///
    /// # Errors
    /// [`Unresolved`]: no URL, offline, an unusable override or row, or an unreadable row.
    pub async fn url(&self) -> Result<QdrantUrl, Unresolved> {
        let generation = self.configured.generation();
        let store = self.current();
        let url = resolve_url(&self.env, store.as_deref()).await;
        self.configured.record(generation, url.is_ok());
        url
    }

    /// D3 and D4, read now: a URL or key changed since the last call applies to this one.
    ///
    /// # Errors
    /// [`Unresolved`].
    ///
    /// MOD-160 D5: the URL's verdict is recorded as soon as it is known, before the key: a URL
    /// that resolves is configured even when the key cannot be read.
    pub async fn settings(&self) -> Result<QdrantSettings, Unresolved> {
        let url = self.url().await?;
        let api_key = self.api_key().await?;
        Ok(QdrantSettings { url, api_key })
    }

    /// D4 alone, read now: the key, `None` when no source holds one. The index job reads it only
    /// when a collection is due (MOD-159 M1 R2), since an interactive key may prompt a locked
    /// keyring.
    ///
    /// # Errors
    /// [`Unresolved::Key`].
    pub async fn api_key(&self) -> Result<Option<Zeroizing<String>>, Unresolved> {
        let key = self
            .key
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        resolve_key(&self.env, &key, self.keyring).await
    }
}

/// How the embedding model is loaded: [`load_model`] in production, a counter in a test.
type Loader = Arc<dyn Fn() -> BoxFuture<'static, Result<RtenEmbedder, String>> + Send + Sync>;

/// The pinned files (fetched or adopted, on this task), then `RtenEmbedder::load` off the async
/// workers (D238, MOD-68 D7), on a thread of its own (`concepts::apart`): `htui worker` loads its
/// index job's model here too (H-26), and a load must not hold its exit (MOD-41 D19).
fn load_model() -> BoxFuture<'static, Result<RtenEmbedder, String>> {
    Box::pin(async {
        let files = model::ensure_model().await.map_err(|e| e.to_string())?;
        crate::concepts::apart("htui-model", move || RtenEmbedder::load(&files))
            .await
            .ok_or_else(|| "the embedding model's loader stopped".to_owned())?
            .map_err(|e| e.to_string())
    })
}

/// The production index (D238, amended at review 3 and by MOD-158 D6): settings resolved per
/// request through its [`QdrantSource`] (the `qdrant_url` row of the published store, or the
/// override, and the key), so a URL or key changed in Settings > Qdrant applies to the next search;
/// the Qdrant connection reused while the URL and key stay the same; the embedding model loaded
/// once, on the blocking pool, and shared.
pub struct QdrantIndex {
    /// The model's load: in flight, done, or failed. One at a time (review 2): a failed one is
    /// replaced by the next search's, a done one answers every later search (D238).
    load: Mutex<Option<Load>>,
    /// What loads the model.
    loader: Loader,
    /// Each collection's last connection, for the settings it was made with (review 3), and how
    /// (D4).
    connections: Connections<QdrantStore<RtenEmbedder>>,
    /// Where the settings come from (MOD-158 D6).
    source: Arc<QdrantSource>,
}

/// How a cached store was made (MOD-160 D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Made {
    /// `QdrantStore::connect`: the collection ensured (payload indexes, stamp, MOD-144 rebuild).
    Ensured,
    /// `ExistingCollection::open` (D2): read-only; never serves a sync.
    ReadOnly,
}

/// One cached connection per collection name (MOD-159 M1 plan D9), each kept for the URL and key
/// it was made with (review 3).
///
/// `QdrantStore::connect` is a round trip and five waited payload-index writes, too much to pay per
/// search. The lock is never held across an `await`: two requests that miss together both connect,
/// and the later `put` wins, which costs one spare connection and nothing else.
///
/// MOD-160 D4: the slot records how its store was made. A search opens an existing collection
/// read-only and never creates one (D2), so its store has none of what `ensure_collection` does; a
/// sync relies on that, so it takes only an ensured store and otherwise connects.
struct Connections<T> {
    /// Per collection name: the settings, how the connection was made, and the connection.
    slots: Mutex<HashMap<String, Cached<T>>>,
}

/// What [`Connections`] holds.
struct Cached<T> {
    /// The settings the connection was made with.
    settings: QdrantSettings,
    /// How (D4).
    made: Made,
    /// The connection.
    connection: Arc<T>,
}

/// Whether `a` and `b` name the same URL and key.
fn same_settings(a: &QdrantSettings, b: &QdrantSettings) -> bool {
    a.url == b.url && a.api_key.as_deref() == b.api_key.as_deref()
}

impl<T> Default for Connections<T> {
    fn default() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> Connections<T> {
    /// The cached connection to the collection `name` for `settings`' URL and key that `need`
    /// accepts: `ReadOnly` takes either kind, `Ensured` only an ensured one (D4).
    fn get(&self, name: &str, settings: &QdrantSettings, need: Made) -> Option<Arc<T>> {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots
            .get(name)
            .filter(|cached| same_settings(&cached.settings, settings))
            .filter(|cached| need == Made::ReadOnly || cached.made == Made::Ensured)
            .map(|cached| Arc::clone(&cached.connection))
    }

    /// Caches `connection` as the collection `name`'s for `settings`. A `ReadOnly` put never
    /// replaces an `Ensured` connection for the same settings: a search racing an index run keeps
    /// the better one.
    fn put(&self, name: &str, settings: QdrantSettings, made: Made, connection: Arc<T>) {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let keeps_better = made == Made::ReadOnly
            && slots.get(name).is_some_and(|cached| {
                cached.made == Made::Ensured && same_settings(&cached.settings, &settings)
            });
        if !keeps_better {
            slots.insert(
                name.to_owned(),
                Cached {
                    settings,
                    made,
                    connection,
                },
            );
        }
    }

    /// The settings stopped resolving (no URL, offline, an unusable setting): no connection, and
    /// no key it was made with, is kept for them, in any collection (MOD-158 R1, B-L1).
    fn clear(&self) {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Every connection made as `made`, in any collection, goes; the others stay.
    fn clear_made(&self, made: Made) {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, cached| cached.made != made);
    }

    /// [`QdrantSource::settings`]' answer, the cache cleared when it is not settings.
    fn resolved(
        &self,
        settings: Result<QdrantSettings, Unresolved>,
    ) -> Result<QdrantSettings, Unresolved> {
        if settings.is_err() {
            self.clear();
        }
        settings
    }

    /// A call on `connection` failed: the next request to its collection reconnects. A
    /// connection another request has cached since is left alone, and so is every other
    /// collection's.
    fn forget(&self, connection: &Arc<T>) {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, cached| !Arc::ptr_eq(&cached.connection, connection));
    }
}

/// One load of the model, awaited by every search that overlaps it.
type Load = Shared<BoxFuture<'static, Result<RtenEmbedder, String>>>;

impl core::fmt::Debug for QdrantIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QdrantIndex").finish_non_exhaustive()
    }
}

impl Default for QdrantIndex {
    /// Over this process's environment and the interactive key (a production entry point).
    fn default() -> Self {
        Self::with_loader(load_model).with_source(Arc::new(QdrantSource::from_process()))
    }
}

impl QdrantIndex {
    /// An index with no model loaded yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An index whose model comes from `loader` (review 2: the seam a test counts loads through).
    /// Its source reads no environment (blueprint P-13): a test that needs one calls
    /// [`Self::with_source`].
    pub(crate) fn with_loader(
        loader: impl Fn() -> BoxFuture<'static, Result<RtenEmbedder, String>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            load: Mutex::new(None),
            loader: Arc::new(loader),
            connections: Connections::default(),
            source: Arc::new(QdrantSource::new(
                QdrantEnv::default(),
                KeySource::Interactive,
            )),
        }
    }

    /// The same index, its settings read through `source`.
    #[must_use]
    pub fn with_source(self, source: Arc<QdrantSource>) -> Self {
        Self { source, ..self }
    }

    /// Where its settings come from: `htui worker` publishes its store and key source here.
    #[must_use]
    pub const fn source(&self) -> &Arc<QdrantSource> {
        &self.source
    }

    /// The model, loading it on first use (F9, blueprint D245). `concepts::IndexJob::spawn` takes
    /// the worker's from here too, so its index job and `search_concepts` share one (H-26).
    pub(crate) async fn embedder(&self) -> Result<RtenEmbedder, String> {
        self.load().await
    }

    /// The load to await: the one in flight or done, else a new one (review 2).
    ///
    /// A new load is driven by a task of its own, so a superseded search aborted mid-load cancels
    /// only its wait, never the load (F9), and a load nobody awaits any more still finishes, so a
    /// failure is seen by the next search, which starts exactly one more. `tokio::sync::OnceCell`
    /// cannot say that: on a failed init it hands the init to the next queued waiter, so every
    /// search that overlapped a failing load retried it, one download after another.
    fn load(&self) -> Load {
        let mut slot = self.load.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(load) = slot.as_ref()
            && !matches!(load.peek(), Some(Err(_)))
        {
            return load.clone();
        }
        let load = (self.loader)().shared();
        tokio::spawn(load.clone().map(drop));
        *slot = Some(load.clone());
        load
    }

    /// `collection` at `settings`, over the shared model: the cached connection when it was made
    /// with the same URL and key, else a new one, cached (review 3). The worker's index job opens
    /// its stores here, so a changed URL or key reconnects them too (MOD-158 D7). It creates the
    /// collection when there is none, so only a sync connects (MOD-160 D2), and only an ensured
    /// store serves it (D4): a read-only one cached by a search is replaced.
    pub(crate) async fn connect(
        &self,
        settings: QdrantSettings,
        collection: &IndexedCollection,
    ) -> Result<Arc<QdrantStore<RtenEmbedder>>, String> {
        let name = collection.name();
        if let Some(store) = self.connections.get(&name, &settings, Made::Ensured) {
            return Ok(store);
        }
        let embedder = self.embedder().await?;
        let store = QdrantStore::connect(&settings, embedder, collection)
            .await
            .map(Arc::new)
            .map_err(refusal)?;
        self.connections
            .put(&name, settings, Made::Ensured, Arc::clone(&store));
        Ok(store)
    }

    /// MOD-160 D2, a search's store of the collection `name`: Qdrant and the collection first,
    /// then the embedder check without the model (D3), then the shared model, then the store,
    /// cached read-only (D4). `Ok(None)` when the collection does not exist: an empty index, with
    /// nothing created, cached or loaded.
    async fn open_read_only(
        &self,
        settings: QdrantSettings,
        name: &str,
    ) -> Result<Option<Arc<QdrantStore<RtenEmbedder>>>, String> {
        let Some(found) = ExistingCollection::find_in(&settings, name)
            .await
            .map_err(refusal)?
        else {
            return Ok(None);
        };
        found
            .check(&model::identity(), DENSE_DIM)
            .await
            .map_err(refusal)?;
        let embedder = self.embedder().await?;
        let store = found.open(embedder).await.map(Arc::new).map_err(refusal)?;
        self.connections
            .put(name, settings, Made::ReadOnly, Arc::clone(&store));
        Ok(Some(store))
    }

    /// D9: `query` over each of `collections` in turn (each read-only, as one search was before
    /// MOD-159), answered by [`partial_answer`]. A missing collection adds nothing; a failure
    /// drops the connection it used.
    async fn search_collections(
        &self,
        settings: &QdrantSettings,
        names: &[String],
        query: &SearchQuery,
    ) -> Result<Vec<Hit>, String> {
        let mut answers = Vec::with_capacity(names.len());
        for name in names {
            let store = match self.connections.get(name, settings, Made::ReadOnly) {
                Some(store) => store,
                None => match self.open_read_only(settings.clone(), name).await {
                    Ok(Some(store)) => store,
                    Ok(None) => continue,
                    Err(err) => {
                        answers.push((name.clone(), Err(err)));
                        continue;
                    }
                },
            };
            let hits = store.search(query).await.map_err(|e| {
                self.connections.forget(&store);
                e.to_string()
            });
            answers.push((name.clone(), hits));
        }
        partial_answer(answers, query.limit)
    }

    /// The worker's index job's `forget` (MOD-158 R1, B-L1): `Some(store)` after a failed sync on
    /// it, `None` when the settings did not resolve.
    pub(crate) fn forget(&self, store: Option<&Arc<QdrantStore<RtenEmbedder>>>) {
        match store {
            Some(store) => self.connections.forget(store),
            None => self.connections.clear(),
        }
    }

    /// The index job's key stopped resolving (MOD-159 M1 R2): the ensured connections a sync made
    /// go, the read-only ones a search made stay.
    pub(crate) fn forget_synced(&self) {
        self.connections.clear_made(Made::Ensured);
    }

    /// The settings, read now; a cached connection is dropped when they do not resolve.
    async fn settings(&self) -> Result<QdrantSettings, String> {
        self.connections
            .resolved(self.source.settings().await)
            .map_err(|why| why.sentence())
    }
}

/// A failed Qdrant call as the error a user reads ([`crate::concepts::qdrant_refusal`]).
fn refusal(err: StoreError) -> String {
    format!("{:#}", crate::concepts::qdrant_refusal(err))
}

// Settings before the model, in both: a box with no URL fails in microseconds without downloading
// anything. A search then asks Qdrant and the collection before the model too, and never creates
// one (MOD-160 D2); a sync connects, which does. Any failure drops the connection it used, so the
// next request reconnects: telling a dropped connection from a refused query would mean parsing the
// client's errors, and a spare reconnect after a refusal costs what every request cost before the
// cache (review 3).
impl ConceptIndex for QdrantIndex {
    fn search(
        &self,
        within: SearchIn,
        query: SearchQuery,
    ) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
        Box::pin(async move {
            let settings = self.settings().await?;
            let mut names: Vec<String> = match within {
                SearchIn::Workspaces(workspaces) => workspaces
                    .into_iter()
                    .map(|ws| IndexedCollection::Workspace(ws).name())
                    .collect(),
                SearchIn::Everywhere => vector::workspace_collections(&settings)
                    .await
                    .map_err(refusal)?
                    .iter()
                    .map(IndexedCollection::name)
                    .collect(),
            };
            names.push(IndexedCollection::Global.name());
            self.search_collections(&settings, &names, &query).await
        })
    }

    fn sync(
        &self,
        backend: Backend,
        collection: IndexedCollection,
        scope: Scope,
        guard: Box<dyn DeleteGuardDyn>,
    ) -> BoxFuture<'_, Result<SyncReport, String>> {
        Box::pin(async move {
            let settings = self.settings().await?;
            let store = self.connect(settings, &collection).await?;
            Indexer::sync(&backend, &scope, &*store, &Guarded(&*guard))
                .await
                .map_err(|e| {
                    self.connections.forget(&store);
                    e.to_string()
                })
        })
    }

    fn publish(&self, store: Option<Arc<dyn UrlRow>>) {
        self.source.publish(store);
    }

    fn offered(&self) -> bool {
        self.source.offered()
    }

    fn url_changed(&self) {
        self.source.reread();
    }

    /// MOD-159 M1 plan D11: probes `htui_global`, the one collection every box shares; a
    /// missing one is `NoCollection`, as an empty Qdrant is.
    fn check(&self) -> BoxFuture<'_, Option<QdrantCheck>> {
        Box::pin(async move { check_qdrant(&self.source, &IndexedCollection::Global.name()).await })
    }
}

/// The process's one production index (MOD-11 T7, H-26): [`ConceptsRuntime::production`] and
/// `mcp_search::production` both search it, so the TUI's concepts search and the MCP tool
/// `search_concepts` share one embedding model (133 MB, MOD-68) and one cached Qdrant connection.
/// Built on first use; building it loads nothing.
///
/// Production only: its settings come from this process's environment (`HTUI_QDRANT_*`,
/// [`QdrantSource::from_process`]), which an hr sandbox sets and a host does not, so no test may
/// search or sync through it (blueprint P-13, MOD-158 R1 B-L7). Tests build a [`QdrantIndex`] with
/// [`QdrantIndex::with_source`] over a `QdrantEnv` literal, or a `MemIndex`.
#[must_use]
pub fn shared_index() -> Arc<dyn ConceptIndex> {
    shared_qdrant()
}

/// [`shared_index`] as itself: `concepts::IndexJob::spawn` takes the worker's model from it, so
/// `htui worker`'s index job and its `search_concepts` load the model once (H-26).
#[must_use]
pub fn shared_qdrant() -> Arc<QdrantIndex> {
    static SHARED: OnceLock<Arc<QdrantIndex>> = OnceLock::new();
    Arc::clone(SHARED.get_or_init(|| Arc::new(QdrantIndex::new())))
}

/// MOD-160 D9: what one model-free look at this process's Qdrant says. The worker's start line,
/// Settings › Qdrant and the TUI's launch notice (both through `StoreRequest::ProbeQdrant`) all
/// read it, so every surface says the same states in the same words. No variant carries the API
/// key: each sentence is an [`Unresolved`] sentence or a [`Probe`]'s, and neither ever holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QdrantCheck {
    /// No URL, or an unusable one: [`Unresolved`]'s sentence.
    NotConfigured(String),
    /// Unreachable, or an embedder mismatch: the probe's sentence.
    Unavailable(String),
    /// The `qdrant_url` row or this box's key could not be read, so Qdrant was never contacted:
    /// [`Unresolved`]'s sentence (MOD-160 R1: never said to be unreachable).
    Unreadable(String),
    /// Refused while this box has a key: the sentence.
    KeyRefused(String),
    /// Refused, and no key from any source.
    NoKey(String),
    /// Answered without any key (Q4): an info line, not a warning.
    Keyless,
    /// Answered with a key, or nothing to say.
    Ready,
}

impl QdrantCheck {
    /// The verdict on `resolved` settings and the `probe` made with them: `None` when nothing was
    /// probed (offline, or no probe). An empty Qdrant (`NoCollection`) is no warning: the index is
    /// built by the next sync.
    pub(crate) fn of(
        resolved: Result<&QdrantSettings, &Unresolved>,
        probe: Option<&Probe>,
    ) -> Option<Self> {
        let settings = match resolved {
            Err(why @ (Unresolved::NoUrl | Unresolved::Unusable(_))) => {
                return Some(Self::NotConfigured(why.sentence()));
            }
            Err(Unresolved::Offline) => return None,
            Err(why @ (Unresolved::Read(_) | Unresolved::Key(_))) => {
                return Some(Self::Unreadable(why.sentence()));
            }
            Ok(settings) => settings,
        };
        let keyed = settings.api_key.is_some();
        Some(match probe? {
            Probe::Ready | Probe::NoCollection if keyed => Self::Ready,
            Probe::Ready | Probe::NoCollection => Self::Keyless,
            Probe::Unreachable(sentence) | Probe::Mismatch(sentence) => {
                Self::Unavailable(sentence.clone())
            }
            Probe::Refused(sentence) if keyed => Self::KeyRefused(sentence.clone()),
            Probe::Refused(sentence) => Self::NoKey(sentence.clone()),
        })
    }

    /// The warning state in words; `None` for [`Self::Keyless`] and [`Self::Ready`].
    #[must_use]
    pub const fn state(&self) -> Option<&'static str> {
        match self {
            Self::NotConfigured(_) => Some("no Qdrant is configured"),
            Self::Unavailable(_) => Some("Qdrant is unreachable or refuses the collection"),
            Self::Unreadable(_) => Some("this box cannot read the Qdrant settings"),
            Self::KeyRefused(_) => Some("the Qdrant refused this box's API key"),
            Self::NoKey(_) => Some("this box lacks the Qdrant API key"),
            Self::Keyless | Self::Ready => None,
        }
    }

    /// `"{state}: {cause}"` for the five warnings; `None` for [`Self::Keyless`] and
    /// [`Self::Ready`].
    #[must_use]
    pub fn warning(&self) -> Option<String> {
        let (Self::NotConfigured(cause)
        | Self::Unavailable(cause)
        | Self::Unreadable(cause)
        | Self::KeyRefused(cause)
        | Self::NoKey(cause)) = self
        else {
            return None;
        };
        self.state().map(|state| format!("{state}: {cause}"))
    }

    /// The info line (Q4), for [`Self::Keyless`] only.
    #[must_use]
    pub const fn info(&self) -> Option<&'static str> {
        match self {
            Self::Keyless => Some("this Qdrant answers without an API key"),
            _ => None,
        }
    }
}

/// MOD-160 D9: `source`'s settings, then [`vector::probe`] of `collection` when they resolve. Loads
/// no model, creates nothing, and is bounded by `vector::PROBE_TIMEOUT`. `None` when nothing was
/// probed (offline).
pub(crate) async fn check_qdrant(source: &QdrantSource, collection: &str) -> Option<QdrantCheck> {
    let resolved = source.settings().await;
    let probe = match &resolved {
        Ok(settings) => Some(vector::probe(settings, collection).await),
        Err(_) => None,
    };
    QdrantCheck::of(resolved.as_ref(), probe.as_ref())
}

/// A [`ConceptIndex`] over `MemVectorStore`s (D230): ranks by shared terms, needs no model. One
/// fake per collection name (MOD-159 M1 plan D9), made by the first sync or [`MemIndex::store`]
/// as `connect` creates a Qdrant collection. It can be told to fail every call, and to wait
/// before answering, for the error and supersede cases.
#[cfg(any(test, feature = "testkit"))]
#[derive(Debug, Default)]
pub struct MemIndex {
    /// The fakes the calls go to, by collection name.
    stores: Mutex<HashMap<String, Arc<vector::MemVectorStore>>>,
    /// What every call answers instead, when set.
    failure: Option<String>,
    /// The collections, by name, whose search fails (a stale or mismatched collection).
    failing_in: HashSet<String>,
    /// How long every call sleeps first, when set.
    delay: Option<Duration>,
    /// What a probe answers (MOD-160 D9); `None`: nothing probed.
    check: Option<QdrantCheck>,
}

#[cfg(any(test, feature = "testkit"))]
impl MemIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every call answers `Err(message)` (after the delay, if any).
    #[must_use]
    pub fn failing(self, message: impl Into<String>) -> Self {
        Self {
            failure: Some(message.into()),
            ..self
        }
    }

    /// A search of `collection` fails, with a message naming it; the other collections answer.
    #[must_use]
    pub fn failing_in(mut self, collection: &IndexedCollection) -> Self {
        self.failing_in.insert(collection.name());
        self
    }

    /// Every call sleeps `by` first (`tokio::time::sleep`, so a paused clock advances it).
    #[must_use]
    pub fn delayed(self, by: Duration) -> Self {
        Self {
            delay: Some(by),
            ..self
        }
    }

    /// A probe answers `check` (after the delay; the failure does not apply to it).
    #[must_use]
    pub fn with_check(self, check: QdrantCheck) -> Self {
        Self {
            check: Some(check),
            ..self
        }
    }

    /// `collection`'s fake, made empty when it has none yet: for a test's setup and assertions.
    #[must_use]
    pub fn store(&self, collection: &IndexedCollection) -> Arc<vector::MemVectorStore> {
        self.named(&collection.name())
    }

    /// The fake of the collection `name`, made when missing.
    fn named(&self, name: &str) -> Arc<vector::MemVectorStore> {
        let mut stores = self.stores.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(stores.entry(name.to_owned()).or_default())
    }

    /// The fake of the collection `name`, `None` when it was never made (a missing collection).
    fn existing(&self, name: &str) -> Option<Arc<vector::MemVectorStore>> {
        self.stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    /// Every workspace collection made so far, by name: what Qdrant would list (D9).
    fn workspace_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .filter(|name| {
                matches!(
                    IndexedCollection::parse(name),
                    Some(IndexedCollection::Workspace(_))
                )
            })
            .cloned()
            .collect();
        names.sort_unstable();
        names
    }

    /// Indexes `scope` through `backend` into `collection` directly, as a test's setup.
    ///
    /// # Panics
    /// If the sync fails, which `MemVectorStore` never does over a memory backend.
    pub async fn seed(
        &self,
        collection: &IndexedCollection,
        backend: &Backend,
        scope: &Scope,
    ) -> SyncReport {
        Indexer::sync(backend, scope, &*self.store(collection), &Unguarded)
            .await
            .expect("a memory index over a memory backend never fails")
    }

    /// The configured wait, then the configured failure, if any.
    async fn pause(&self) -> Result<(), String> {
        if let Some(by) = self.delay {
            tokio::time::sleep(by).await;
        }
        self.failure.clone().map_or(Ok(()), Err)
    }
}

#[cfg(any(test, feature = "testkit"))]
impl ConceptIndex for MemIndex {
    fn search(
        &self,
        within: SearchIn,
        query: SearchQuery,
    ) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
        Box::pin(async move {
            self.pause().await?;
            let mut names: Vec<String> = match within {
                SearchIn::Workspaces(workspaces) => workspaces
                    .into_iter()
                    .map(|ws| IndexedCollection::Workspace(ws).name())
                    .collect(),
                SearchIn::Everywhere => self.workspace_names(),
            };
            names.push(IndexedCollection::Global.name());
            let mut answers = Vec::with_capacity(names.len());
            for name in names {
                if let Some(store) = self.existing(&name) {
                    let answer = if self.failing_in.contains(&name) {
                        Err(format!("qdrant: {name}: planted failure"))
                    } else {
                        store.search(&query).await.map_err(|e| e.to_string())
                    };
                    answers.push((name, answer));
                }
            }
            partial_answer(answers, query.limit)
        })
    }

    fn sync(
        &self,
        backend: Backend,
        collection: IndexedCollection,
        scope: Scope,
        guard: Box<dyn DeleteGuardDyn>,
    ) -> BoxFuture<'_, Result<SyncReport, String>> {
        Box::pin(async move {
            self.pause().await?;
            let store = self.store(&collection);
            Indexer::sync(&backend, &scope, &*store, &Guarded(&*guard))
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn check(&self) -> BoxFuture<'_, Option<QdrantCheck>> {
        Box::pin(async move {
            // The wait applies; a configured failure is a search's or a sync's, not a probe's.
            let _ = self.pause().await;
            self.check.clone()
        })
    }
}

/// The box `backend` speaks for, the holder a lease names (MOD-159 M1 plan D12): Postgres's
/// registered box online, the memory store's own (the demo has one); `None` when there is none.
async fn this_box(backend: &Backend) -> Option<BoxId> {
    if let Some(pg) = backend.writable() {
        return Some(pg.this_box());
    }
    backend
        .box_info()
        .await
        .ok()
        .flatten()
        .map(|info| info.box_id)
}

/// `Ctrl+R` under `scope`'s workspace collection lease (MOD-159 M1 plan D12, R2). A lease the
/// runtime's token (`job_owner`) holds, its index job's, renews: the sync runs under it and it is
/// kept. Otherwise the press goes through [`take_or_request_pass`] under the runtime's press token
/// (`press_owner`, never the job's, MOD-159 M3 plan D7), so the job, beating meanwhile, finds the
/// lease held instead of sharing it: a lease free or already the presses' is synced under and let
/// go of after the sync (a press aborted half-way, superseded or quit, leaves it to the next press,
/// which takes it back as its own, or to the TUI's exit, which lets it go); a live holder is asked
/// for a pass ([`IndexOutcome::Requested`]); one that could not be asked is
/// [`IndexOutcome::HeldBy`]. Neither syncs anything. The sync renews the lease before each delete
/// batch ([`LeaseRenewal`]).
async fn index_under_lease(
    index: &dyn ConceptIndex,
    backend: Backend,
    scope: Scope,
    job_owner: Uuid,
    press_owner: Uuid,
) -> Result<IndexOutcome, String> {
    let collection = IndexedCollection::Workspace(scope.workspace_id);
    let Some(box_id) = this_box(&backend).await else {
        return Err(NO_BOX_IDENTITY.to_owned());
    };
    let ttl = LeaseTimes::from_app(&backend.app_settings().await.unwrap_or_default()).ttl;
    let the_jobs = backend
        .renew_indexer_lease(&collection, job_owner, ttl)
        .await
        .map_err(|e| e.to_string())?;
    let owner = if the_jobs {
        job_owner
    } else {
        match take_or_request_pass(&backend, &collection, box_id, press_owner, ttl)
            .await
            .map_err(|e| e.to_string())?
        {
            LeaseTurn::Requested(lease) => return Ok(IndexOutcome::Requested(lease)),
            LeaseTurn::Held(lease) => return Ok(IndexOutcome::HeldBy(lease)),
            // Free, or a superseded press's left under the same token: the press holds it now.
            LeaseTurn::Ours => press_owner,
        }
    };
    let guard = LeaseRenewal {
        backend: backend.clone(),
        collection,
        owner,
        ttl,
    };
    let synced = index
        .sync(backend.clone(), collection, scope, Box::new(guard))
        .await;
    if !the_jobs {
        // This one lease, compare-and-set on the press's token: nothing of the job's.
        if let Err(err) = backend.release_indexer_lease(&collection, owner).await {
            tracing::warn!(%err, %collection, "the Ctrl+R lease was not let go; it lapses");
        }
    }
    synced.map(IndexOutcome::Synced)
}

/// What [`ConceptsRuntime::serve`] decided (the `RunServed` shape, blueprint D243).
#[derive(Debug)]
pub enum ConceptsServed {
    /// Answer with this reply, now.
    Reply(StoreReply),
    /// A task of the runtime answers the request, exactly once.
    Deferred,
}

/// Serves `SearchConcepts`, `IndexConcepts` and `ProbeQdrant` on tasks of their own, inside the
/// store worker's loop beside `AgentRuntime` and `RunRuntime` (D231). One task per
/// `(origin, request kind)`: a newer request of the same kind from the same view aborts the older
/// one, whose answer the shell would drop anyway. A search never aborts an index run: they are
/// different kinds.
pub struct ConceptsRuntime {
    /// The index every task searches and writes.
    index: Arc<dyn ConceptIndex>,
    /// The task of each `(origin, request kind)`, keyed as `App::latest` is.
    tasks: HashMap<(Origin, Discriminant<StoreRequest>), JoinHandle<()>>,
    /// What [`Self::publish`] last handed the index (MOD-158 R1, B-L2).
    published: Published,
    /// This process's indexer-lease token (MOD-159 M1 plan D5), its index job's: `Ctrl+R` renews
    /// a lease the job holds under it, and takes a free one under [`Self::press_owner`] (R2).
    owner: Uuid,
    /// Every `Ctrl+R` press's indexer-lease token (MOD-159 M3 plan D7), one per runtime and never
    /// [`Self::owner`]: a press superseded half-way leaves its lease under it, so the next press
    /// takes it back as its own, and [`Self::stop_index_job`] lets go of what it still holds.
    press_owner: Uuid,
    /// The tasks [`Self::shutdown`] aborted, awaited by [`Self::stop_index_job`] before it lets go
    /// of the presses' leases, so none is taken again after the release.
    aborted: Vec<JoinHandle<()>>,
    /// The TUI's index job ([`Self::with_index_job`]), `None` when this runtime runs none.
    job: Option<JobSlot>,
}

/// The TUI's index job (MOD-159 M1 plan D8): the index it runs over, the job over the writable
/// store last published, if any, and the stops of the jobs before it.
struct JobSlot {
    /// The process's shared index, so the job and the overlay's search load one model.
    index: Arc<QdrantIndex>,
    /// The job running now.
    running: Option<crate::concepts::IndexJob<PgStore>>,
    /// Every earlier job's stop, chained, until it is done (MOD-159 M1 R2): kept across a `None`
    /// publish, so a job started after going offline and back still waits for the release of the
    /// one before, which lets go of every lease of the same token.
    stopping: Option<Stopping>,
}

/// Earlier jobs' stops, awaited by the next job's first beat and by
/// [`ConceptsRuntime::stop_index_job`].
type Stopping = Shared<BoxFuture<'static, ()>>;

/// How long the TUI's [`crate::concepts::IndexJob::stop`] waits for its leases to be let go
/// (MOD-159 M1 R2): the store loop's whole shutdown has `crate::SHUTDOWN` (5 s), so less than
/// `htui worker`'s [`crate::concepts::WORKER_RELEASE_WAIT`]; leases not let go lapse in the TTL.
pub const TUI_RELEASE_WAIT: Duration = Duration::from_secs(1);

/// What [`ConceptsRuntime::publish`] last handed the index, so an unchanged backend is not handed
/// over again on every iteration of the store loop.
enum Published {
    /// Nothing yet: the first publish always goes through.
    Never,
    /// No writable store (offline, the mirror, memory).
    Nothing,
    /// The store over this pool, known by its connect options' allocation. Weak, so a pool that is
    /// gone is not kept, nor its password; and while the weak lives the address is not reused, so a
    /// new pool never passes for the old one.
    Pool(Weak<dyn std::any::Any + Send + Sync>),
}

impl Published {
    /// What publishing `store` now would hand over.
    fn of(store: Option<&PgStore>) -> Self {
        store.map_or(Self::Nothing, |pg| {
            Self::Pool(Self::weak(pg.pool().connect_options()))
        })
    }

    /// The options' allocation, typed away (htui names no `sqlx` type outside its tests).
    fn weak<T: Send + Sync + 'static>(options: Arc<T>) -> Weak<dyn std::any::Any + Send + Sync> {
        let options: Arc<dyn std::any::Any + Send + Sync> = options;
        Arc::downgrade(&options)
    }

    /// Whether `self` and `other` hand the index the same thing.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nothing, Self::Nothing) => true,
            (Self::Pool(a), Self::Pool(b)) => Weak::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl core::fmt::Debug for ConceptsRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConceptsRuntime")
            .field("tasks", &self.tasks.len())
            .finish_non_exhaustive()
    }
}

impl ConceptsRuntime {
    /// A runtime over `index`.
    #[must_use]
    pub fn new(index: Arc<dyn ConceptIndex>) -> Self {
        Self {
            index,
            tasks: HashMap::new(),
            published: Published::Never,
            owner: Uuid::now_v7(),
            press_owner: Uuid::now_v7(),
            aborted: Vec::new(),
            job: None,
        }
    }

    /// The same runtime, also running the TUI's index job (MOD-159 M1 plan D8) over the process's
    /// shared index: [`Self::publish`] starts it over each writable store it is handed, with this
    /// runtime's token (so `Ctrl+R` renews a lease the job holds), and stops it, letting go of its
    /// leases, when the store goes; [`Self::stop_index_job`] stops it at exit. Only
    /// `store_worker::spawn_hosted` asks for it: `--demo` and the memory backend have no writable
    /// store, so no job runs there, and the test harness builds runtimes without one.
    #[must_use]
    pub fn with_index_job(self) -> Self {
        self.with_index_job_over(shared_qdrant())
    }

    /// [`Self::with_index_job`] over `index`: a test's, whose settings come from a `QdrantEnv`
    /// literal (blueprint P-13).
    #[must_use]
    pub(crate) fn with_index_job_over(self, index: Arc<QdrantIndex>) -> Self {
        Self {
            job: Some(JobSlot {
                index,
                running: None,
                stopping: None,
            }),
            ..self
        }
    }

    /// The indexer-lease token `Ctrl+R` takes leases with (MOD-159 M1 plan D5, D12).
    #[must_use]
    pub const fn owner(&self) -> Uuid {
        self.owner
    }

    /// The indexer-lease token every `Ctrl+R` press takes a lease with when the index job does not
    /// hold it (MOD-159 M3 plan D7).
    #[must_use]
    pub const fn press_owner(&self) -> Uuid {
        self.press_owner
    }

    /// Over [`shared_index`]: what `store_worker::spawn_with_runtimes` builds (D231).
    ///
    /// Production only: the shared index resolves its settings over this process's environment
    /// (`HTUI_QDRANT_*`), so a test that searched through it would behave differently in an hr
    /// sandbox, which exports `HTUI_QDRANT_URL`, than on the host. Tests build
    /// [`Self::new`] over an index of their own (blueprint P-13, MOD-158 R1 B-L7).
    #[must_use]
    pub fn production() -> Self {
        Self::new(shared_index())
    }

    /// MOD-158 D6: `backend`'s writable store, or `None`, becomes what the index reads the
    /// `qdrant_url` row through. The store worker calls it at the top of every iteration, so every
    /// swap (connect, `SetDsn`, a lost server) reaches the index (blueprint P-2). Only a change is
    /// handed over: the same pool, or still no store, is skipped (MOD-158 R1, B-L2). Awaits
    /// nothing.
    pub fn publish(&mut self, backend: &Backend) {
        let store = backend.writable();
        let now = Published::of(store);
        if now.same(&self.published) {
            return;
        }
        self.published = now;
        self.index
            .publish(store.map(|pg| Arc::new(pg.clone()) as Arc<dyn UrlRow>));
        // MOD-159 M1 plan D8: the job follows the writable store. The old one stops on a task of
        // its own (this awaits nothing), after every stop before it; the next job's first beat
        // waits for them all, so no release under the same token can free a lease it has just
        // taken. The chain outlives a `None` publish (R2), for the job after going back online.
        if let Some(job) = &mut self.job {
            if let Some(old) = job.running.take() {
                let before = job.stopping.take();
                let stop = tokio::spawn(async move {
                    if let Some(before) = before {
                        before.await;
                    }
                    old.stop(TUI_RELEASE_WAIT).await;
                });
                job.stopping = Some(
                    async move {
                        let _ = stop.await;
                    }
                    .boxed()
                    .shared(),
                );
            }
            job.running = store.map(|pg| {
                crate::concepts::IndexJob::spawn_over(
                    pg.clone(),
                    Arc::clone(&job.index),
                    self.owner,
                    job.stopping.clone().map(FutureExt::boxed),
                )
            });
        }
    }

    /// MOD-159 M1 plan D8, M3 plan D7: at the TUI's exit, side by side, lets go of the `Ctrl+R`
    /// presses' leases over `backend` (after the tasks [`Self::shutdown`] aborted are gone or half
    /// of [`TUI_RELEASE_WAIT`] has passed, all within it; a failure, a timeout or an offline
    /// backend leaves them to lapse) and stops the TUI's index job, if one runs, letting go of its
    /// leases (waited for at most [`TUI_RELEASE_WAIT`]), then waits as long again at most for an earlier job's stop
    /// still in flight. The two tokens differ, so the two legs touch different rows. The store
    /// loop calls it once, when the UI is gone, after [`Self::shutdown`].
    pub async fn stop_index_job(&mut self, backend: &Backend) {
        let (aborted, press_owner) = (std::mem::take(&mut self.aborted), self.press_owner);
        let presses = async move {
            // MOD-159 M3 R1: the aborted tasks have half the wait, so one that does not end still
            // leaves the release the rest; the leg as a whole stays within the wait.
            let deadline = tokio::time::Instant::now() + TUI_RELEASE_WAIT;
            let gone = async {
                for task in aborted {
                    let _ = task.await;
                }
            };
            if tokio::time::timeout(TUI_RELEASE_WAIT / 2, gone)
                .await
                .is_err()
            {
                tracing::debug!("an aborted concepts task did not end in time");
            }
            let release = backend.release_indexer_leases(press_owner);
            match tokio::time::timeout_at(deadline, release).await {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => {
                    tracing::debug!(%err, "the Ctrl+R leases were not let go; they lapse");
                }
                Err(_) => tracing::debug!("the Ctrl+R leases were not let go in time; they lapse"),
            }
        };
        let job = async {
            let Some(job) = self.job.as_mut() else {
                return;
            };
            if let Some(running) = job.running.take() {
                running.stop(TUI_RELEASE_WAIT).await;
            }
            if let Some(stopping) = job.stopping.take() {
                let _ = tokio::time::timeout(TUI_RELEASE_WAIT, stopping).await;
            }
        };
        tokio::join!(presses, job);
    }

    /// MOD-160 D5: the loop wrote or cleared the `qdrant_url` row (or tried): the index reads its
    /// offer again. Awaits nothing.
    pub fn url_changed(&self) {
        self.index.url_changed();
    }

    /// Spawns the request's task and answers `Deferred`, or answers now. Awaits nothing.
    ///
    /// Not `async`: the two things it does are a `match` and a `tokio::spawn`, so the worker's
    /// `select!` arm returns having awaited nothing at all (`R-NF-3`). The backend clone is a
    /// snapshot, not the backend: it cannot perform the swap the worker owns (the preview's rule).
    pub fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
    ) -> ConceptsServed {
        self.tasks.retain(|_, task| !task.is_finished());
        let key = (
            envelope.origin.clone(),
            std::mem::discriminant(&envelope.request),
        );
        let (seq, origin) = (envelope.seq, envelope.origin.clone());
        let index = Arc::clone(&self.index);
        let replies = replies.clone();
        let task = match &envelope.request {
            StoreRequest::SearchConcepts {
                query,
                scope_row_of,
                workspace,
            } => {
                // D233: no projects is no search; the index would say `[]` too. MOD-147 M4 D8: a
                // workspace's scope row still is something to search. MOD-159 M1 D9: the nil
                // (startup) workspace has no collection to search.
                if (query.projects.is_empty() && scope_row_of.is_none())
                    || query.limit == 0
                    || workspace.as_uuid().is_nil()
                {
                    return ConceptsServed::Reply(StoreReply::Concepts(Box::new(
                        ConceptsReply::Hits {
                            query: query.clone(),
                            scope_row_of: *scope_row_of,
                            outcome: Ok(Vec::new()),
                        },
                    )));
                }
                let (backend, query, scope_row_of, workspace) =
                    (backend.clone(), query.clone(), *scope_row_of, *workspace);
                tokio::spawn(async move {
                    // MOD-147 M4 D8: widened here, off the loop; the reply keeps the request's
                    // own query and scope row (R1 M1), so the overlay matches it to its search.
                    let searched = match scope_row_of {
                        Some(workspace_id) => {
                            let members = Scope {
                                workspace_id,
                                project_ids: query.projects.clone(),
                            };
                            SearchQuery {
                                projects: workspace_search_scope(&backend, &members)
                                    .await
                                    .project_ids,
                                ..query.clone()
                            }
                        }
                        None => query.clone(),
                    };
                    let outcome = index
                        .search(SearchIn::Workspaces(vec![workspace]), searched)
                        .await;
                    let reply = ConceptsReply::Hits {
                        query,
                        scope_row_of,
                        outcome,
                    };
                    let _ = replies.send(ReplyEnvelope {
                        seq,
                        origin,
                        reply: StoreReply::Concepts(Box::new(reply)),
                    });
                })
            }
            StoreRequest::IndexConcepts {
                scope,
                scope_row_of,
            } => {
                // MOD-159 M1 D12: the nil (startup) workspace has no collection either.
                if (scope.project_ids.is_empty() && scope_row_of.is_none())
                    || scope.workspace_id.as_uuid().is_nil()
                {
                    return ConceptsServed::Reply(StoreReply::Concepts(Box::new(
                        ConceptsReply::Indexed(Ok(IndexOutcome::Synced(SyncReport::default()))),
                    )));
                }
                let (backend, scope, scope_row_of, owner, press_owner) = (
                    backend.clone(),
                    scope.clone(),
                    *scope_row_of,
                    self.owner,
                    self.press_owner,
                );
                // MOD-159 M3 R1: every press shares one token, so the press this one supersedes
                // must be gone before this one takes: its release, still in flight, would let go
                // of the lease this press has just taken. Aborted before the spawn, awaited first.
                let superseded = self.tasks.remove(&key);
                if let Some(superseded) = &superseded {
                    superseded.abort();
                }
                tokio::spawn(async move {
                    if let Some(superseded) = superseded {
                        let _ = superseded.await;
                    }
                    // MOD-147 M4 D8: the scope row joins off the loop, as a search's does.
                    let scope = match scope_row_of {
                        Some(workspace_id) => {
                            let asked = Scope {
                                workspace_id,
                                ..scope
                            };
                            workspace_search_scope(&backend, &asked).await
                        }
                        None => scope,
                    };
                    let outcome =
                        index_under_lease(&*index, backend, scope, owner, press_owner).await;
                    let _ = replies.send(ReplyEnvelope {
                        seq,
                        origin,
                        reply: StoreReply::Concepts(Box::new(ConceptsReply::Indexed(outcome))),
                    });
                })
            }
            // MOD-160 D9: a model-free look at Qdrant, bounded by the probe's 3 s, on its own task
            // like the two above. Its answer is never `Failed`: "nothing probed" is `None`.
            StoreRequest::ProbeQdrant => tokio::spawn(async move {
                let check = index.check().await;
                let _ = replies.send(ReplyEnvelope {
                    seq,
                    origin,
                    reply: StoreReply::QdrantProbe(check),
                });
            }),
            // The loop routes only the three above here; the two runtimes' rule for anything else.
            other => {
                return ConceptsServed::Reply(StoreReply::Failed {
                    request: other.name(),
                    message: "not a concepts request".to_owned(),
                });
            }
        };
        // The previous request of this kind from this view is work whose answer the staleness
        // index is already committed to dropping. Aborting a search is safe anywhere; aborting an
        // index run half-way leaves what the next run rebuilds.
        if let Some(superseded) = self.tasks.insert(key, task) {
            superseded.abort();
        }
        ConceptsServed::Deferred
    }

    /// Harness only (`RunRuntime::settle`'s shape): awaits every task, each within `limit`, and
    /// answers how many did not finish (they are aborted).
    pub async fn settle(&mut self, limit: Duration) -> usize {
        let mut stuck = 0;
        for task in std::mem::take(&mut self.tasks).into_values() {
            let abort = task.abort_handle();
            if tokio::time::timeout(limit, task).await.is_err() {
                abort.abort();
                stuck += 1;
            }
        }
        stuck
    }

    /// The UI is gone: every task is aborted. An index run stopped half-way is safe: the next run
    /// rebuilds what it did not reach. The aborted tasks are kept for [`Self::stop_index_job`],
    /// which awaits them before it lets go of the presses' leases (MOD-159 M3 plan D7).
    pub fn shutdown(&mut self) {
        for task in std::mem::take(&mut self.tasks).into_values() {
            task.abort();
            self.aborted.push(task);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concepts::{self, DECISION_RESOLUTIONS};
    use crate::store_worker;
    use crate::ui::overlay::OverlayId;
    use htui_core::fixtures::ids;
    use htui_core::model::LeaseTake;
    use htui_core::model::document::GOALS_KIND;
    use htui_core::model::{DocumentId, NewOwnerDocument, OwnerDocument};
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
    use htui_store::vector::{Owner, PointType};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PROBE: Origin = Origin::Overlay(OverlayId("probe"));

    /// The demo store and its `platform` workspace, the one FEAT-1 is in.
    async fn platform() -> (Backend, Scope) {
        let backend = Backend::memory(MemStore::demo());
        let StoreReply::Workspaces(workspaces) =
            store_worker::serve(&backend, &StoreRequest::Workspaces).await
        else {
            panic!("workspaces answered with the wrong variant")
        };
        let platform = workspaces
            .iter()
            .find(|w| w.slug == "platform")
            .expect("the demo fixture holds the `platform` workspace");
        let scope = Scope::from_workspace(platform);
        (backend, scope)
    }

    /// FEAT-1's title: words the seeded fake is sure to match.
    async fn feat_1_title(backend: &Backend) -> String {
        backend
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("the memory store never fails")
            .expect("FEAT-1 is in the fixture")
            .title
    }

    fn envelope(seq: u64, request: StoreRequest) -> RequestEnvelope {
        RequestEnvelope {
            seq,
            origin: PROBE,
            request,
        }
    }

    /// Serves one request, awaits the runtime's tasks and drains every reply: a direct answer
    /// first, addressed as the task's would be.
    async fn serve_one(
        runtime: &mut ConceptsRuntime,
        backend: &Backend,
        envelope: &RequestEnvelope,
    ) -> Vec<ReplyEnvelope> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut out = Vec::new();
        if let ConceptsServed::Reply(reply) = runtime.serve(backend, &tx, envelope) {
            out.push(ReplyEnvelope {
                seq: envelope.seq,
                origin: envelope.origin.clone(),
                reply,
            });
        }
        assert_eq!(runtime.settle(Duration::from_secs(10)).await, 0);
        while let Ok(reply) = rx.try_recv() {
            out.push(reply);
        }
        out
    }

    fn concepts_reply(reply: &StoreReply) -> &ConceptsReply {
        match reply {
            StoreReply::Concepts(concepts) => concepts,
            other => panic!("expected a concepts reply, got {other:?}"),
        }
    }

    fn hits(reply: &StoreReply) -> (&SearchQuery, &Result<Vec<Hit>, String>) {
        match concepts_reply(reply) {
            ConceptsReply::Hits { query, outcome, .. } => (query, outcome),
            other @ ConceptsReply::Indexed(_) => panic!("expected hits, got {other:?}"),
        }
    }

    /// The `scope_row_of` a search's reply echoes (MOD-147 M4 R1 M1).
    fn echoed_scope_row(reply: &StoreReply) -> Option<WorkspaceId> {
        match concepts_reply(reply) {
            ConceptsReply::Hits { scope_row_of, .. } => *scope_row_of,
            other @ ConceptsReply::Indexed(_) => panic!("expected hits, got {other:?}"),
        }
    }

    fn outcome(reply: &StoreReply) -> &Result<IndexOutcome, String> {
        match concepts_reply(reply) {
            ConceptsReply::Indexed(outcome) => outcome,
            other @ ConceptsReply::Hits { .. } => panic!("expected an index report, got {other:?}"),
        }
    }

    /// An index run's report, or why it stopped; a lease held elsewhere is a panic.
    fn indexed(reply: &StoreReply) -> Result<SyncReport, String> {
        match outcome(reply) {
            Ok(IndexOutcome::Synced(report)) => Ok(*report),
            Ok(IndexOutcome::HeldBy(lease)) => panic!("expected a sync, held by {lease:?}"),
            Ok(IndexOutcome::Requested(lease)) => panic!("expected a sync, asked {lease:?}"),
            Err(message) => Err(message.clone()),
        }
    }

    /// The demo's `platform` workspace collection, the one its searches and index runs use.
    const PLATFORM: IndexedCollection = IndexedCollection::Workspace(ids::WORKSPACE_PLATFORM);

    async fn seeded(backend: &Backend, scope: &Scope, index: MemIndex) -> Arc<MemIndex> {
        index
            .seed(
                &IndexedCollection::Workspace(scope.workspace_id),
                backend,
                scope,
            )
            .await;
        Arc::new(index)
    }

    #[tokio::test]
    async fn a_search_answers_the_fake_s_hits_at_its_address() {
        let (backend, scope) = platform().await;
        let index = seeded(&backend, &scope, MemIndex::new()).await;
        let mut runtime = ConceptsRuntime::new(index.clone());
        let query = concepts::query(
            &feat_1_title(&backend).await,
            scope.project_ids.clone(),
            false,
            10,
        );
        let (tx, mut rx) = mpsc::unbounded_channel();

        let served = runtime.serve(
            &backend,
            &tx,
            &envelope(
                7,
                StoreRequest::SearchConcepts {
                    query: query.clone(),
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        );
        assert!(matches!(served, ConceptsServed::Deferred), "{served:?}");
        assert_eq!(runtime.settle(Duration::from_secs(10)).await, 0);

        let reply = rx.try_recv().expect("the task answered");
        assert!(rx.try_recv().is_err(), "exactly once");
        assert_eq!((reply.seq, &reply.origin), (7, &PROBE));
        let (echoed, outcome) = hits(&reply.reply);
        assert_eq!(echoed, &query);
        let expected = index
            .store(&PLATFORM)
            .search(&query)
            .await
            .expect("the fake");
        assert!(!expected.is_empty(), "FEAT-1's own title finds it");
        assert_eq!(outcome.as_ref().expect("no failure"), &expected);
    }

    #[tokio::test]
    async fn decisions_narrow_the_hits_to_closed_items() {
        let (backend, scope) = platform().await;
        let index = seeded(&backend, &scope, MemIndex::new()).await;
        let mut runtime = ConceptsRuntime::new(index.clone());
        let decided = index
            .store(&PLATFORM)
            .points()
            .into_iter()
            .find(|p| {
                p.subject
                    .resolution()
                    .is_some_and(|r| DECISION_RESOLUTIONS.contains(&r))
            })
            .expect("the demo fixture closes an item as a decision");
        let text = format!("{} {}", feat_1_title(&backend).await, decided.text);

        let all = concepts::query(&text, scope.project_ids.clone(), false, 50);
        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                1,
                StoreRequest::SearchConcepts {
                    query: all,
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        )
        .await;
        let everything = hits(&replies[0].reply).1.clone().expect("no failure");
        assert!(
            everything.iter().any(|hit| hit.resolution.is_none()),
            "without the toggle an open item is found too"
        );

        let decisions = concepts::query(&text, scope.project_ids.clone(), true, 50);
        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                2,
                StoreRequest::SearchConcepts {
                    query: decisions,
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        )
        .await;
        let narrowed = hits(&replies[0].reply).1.clone().expect("no failure");
        assert!(!narrowed.is_empty(), "the decided item's own text finds it");
        for hit in &narrowed {
            assert!(
                hit.resolution
                    .is_some_and(|r| DECISION_RESOLUTIONS.contains(&r)),
                "{hit:?}"
            );
            assert_ne!(hit.point_type, PointType::Requirement, "{hit:?}");
        }
    }

    #[tokio::test]
    async fn a_search_with_no_projects_answers_empty_without_the_index() {
        let (backend, _) = platform().await;
        let mut runtime = ConceptsRuntime::new(Arc::new(MemIndex::new().failing("boom")));
        let (tx, _rx) = mpsc::unbounded_channel();
        let query = concepts::query("anything", Vec::new(), false, 10);

        let served = runtime.serve(
            &backend,
            &tx,
            &envelope(
                1,
                StoreRequest::SearchConcepts {
                    query: query.clone(),
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        );
        let ConceptsServed::Reply(reply) = served else {
            panic!("answered at once, got {served:?}")
        };
        let (echoed, outcome) = hits(&reply);
        assert_eq!(echoed, &query);
        assert_eq!(
            outcome,
            &Ok(Vec::new()),
            "the failing index was never called"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_search_from_the_same_origin_aborts_the_first() {
        let (backend, scope) = platform().await;
        let index = seeded(
            &backend,
            &scope,
            MemIndex::new().delayed(Duration::from_secs(1)),
        )
        .await;
        let mut runtime = ConceptsRuntime::new(index);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let title = feat_1_title(&backend).await;
        let query = concepts::query(&title, scope.project_ids.clone(), false, 10);

        for seq in [1, 2] {
            let served = runtime.serve(
                &backend,
                &tx,
                &envelope(
                    seq,
                    StoreRequest::SearchConcepts {
                        query: query.clone(),
                        scope_row_of: None,
                        workspace: ids::WORKSPACE_PLATFORM,
                    },
                ),
            );
            assert!(matches!(served, ConceptsServed::Deferred), "{served:?}");
        }
        assert_eq!(runtime.settle(Duration::from_secs(10)).await, 0);
        // Long past the first one's delay: had it not been aborted, it would have answered.
        tokio::time::sleep(Duration::from_secs(60)).await;

        let seqs: Vec<u64> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|reply| reply.seq)
            .collect();
        assert_eq!(seqs, vec![2]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_search_does_not_abort_an_index_run() {
        let (backend, scope) = platform().await;
        let index = Arc::new(MemIndex::new().delayed(Duration::from_secs(1)));
        let mut runtime = ConceptsRuntime::new(index);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        let first = runtime.serve(
            &backend,
            &tx,
            &envelope(
                1,
                StoreRequest::IndexConcepts {
                    scope: scope.clone(),
                    scope_row_of: None,
                },
            ),
        );
        let second = runtime.serve(
            &backend,
            &tx,
            &envelope(
                2,
                StoreRequest::SearchConcepts {
                    query,
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        );
        assert!(matches!(first, ConceptsServed::Deferred), "{first:?}");
        assert!(matches!(second, ConceptsServed::Deferred), "{second:?}");
        assert_eq!(runtime.settle(Duration::from_secs(10)).await, 0);

        let mut seqs: Vec<u64> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|reply| reply.seq)
            .collect();
        seqs.sort_unstable();
        assert_eq!(seqs, vec![1, 2]);
    }

    #[tokio::test]
    async fn an_index_run_reports_its_counts() {
        let (backend, scope) = platform().await;
        let index = Arc::new(MemIndex::new());
        let mut runtime = ConceptsRuntime::new(index.clone());
        let request = StoreRequest::IndexConcepts {
            scope: scope.clone(),
            scope_row_of: None,
        };

        let replies = serve_one(&mut runtime, &backend, &envelope(1, request.clone())).await;
        assert_eq!(replies.len(), 1);
        let report = *indexed(&replies[0].reply).as_ref().expect("no failure");
        assert!(report.items_rebuilt > 0, "{report:?}");
        assert_eq!(
            report.points_upserted,
            index.store(&PLATFORM).points().len()
        );

        let replies = serve_one(&mut runtime, &backend, &envelope(2, request)).await;
        let again = *indexed(&replies[0].reply).as_ref().expect("no failure");
        assert_eq!(again.items_rebuilt, 0, "nothing changed since: {again:?}");
    }

    /// The demo store with a `platform` goals document that says `zircon`, the members-only
    /// `platform` scope, and the document.
    async fn platform_with_goals() -> (Backend, Scope, OwnerDocument) {
        let mem = MemStore::demo();
        let goals = mem
            .write_owner_document(NewOwnerDocument {
                id: DocumentId::new(),
                project_id: ids::PROJECT_SCOPE_PLATFORM,
                kind: GOALS_KIND.into(),
                title: "Goals".into(),
                body: "## Aim\none zircon search for the platform".into(),
                created_by: ids::USER,
                created_at: chrono::Utc::now(),
            })
            .await
            .expect("the memory store writes it");
        let backend = Backend::memory(mem);
        let StoreReply::Workspaces(workspaces) =
            store_worker::serve(&backend, &StoreRequest::Workspaces).await
        else {
            panic!("workspaces answered with the wrong variant")
        };
        let platform = workspaces
            .iter()
            .find(|w| w.slug == "platform")
            .expect("the demo fixture holds the `platform` workspace");
        (backend, Scope::from_workspace(platform), goals)
    }

    /// MOD-147 M4 D8: `scope_row_of` widens a search to the workspace's scope row off the loop,
    /// so its goals document is found; without it the members alone are searched. The reply
    /// carries the request's own query either way, and no projects with a scope row still search.
    #[tokio::test]
    async fn a_search_with_the_scope_row_finds_the_workspaces_own_document() {
        let (backend, scope, goals) = platform_with_goals().await;
        let wide = scope.with_scope_row(ids::PROJECT_SCOPE_PLATFORM);
        let index = seeded(&backend, &wide, MemIndex::new()).await;
        let mut runtime = ConceptsRuntime::new(index);
        let finds_goals = |reply: &StoreReply| {
            hits(reply)
                .1
                .as_ref()
                .expect("no failure")
                .iter()
                .any(|hit| hit.owner == Owner::Document(goals.id))
        };

        for (seq, projects) in [(1, scope.project_ids.clone()), (2, Vec::new())] {
            let query = concepts::query("zircon", projects, false, 10);
            let replies = serve_one(
                &mut runtime,
                &backend,
                &envelope(
                    seq,
                    StoreRequest::SearchConcepts {
                        query: query.clone(),
                        scope_row_of: Some(scope.workspace_id),
                        workspace: ids::WORKSPACE_PLATFORM,
                    },
                ),
            )
            .await;
            assert_eq!(replies.len(), 1);
            assert_eq!(hits(&replies[0].reply).0, &query, "the request's own query");
            assert_eq!(
                echoed_scope_row(&replies[0].reply),
                Some(scope.workspace_id),
                "and its own scope row (MOD-147 M4 R1 M1)"
            );
            assert!(finds_goals(&replies[0].reply), "{:?}", replies[0].reply);
        }

        let query = concepts::query("zircon", scope.project_ids.clone(), false, 10);
        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                3,
                StoreRequest::SearchConcepts {
                    query: query.clone(),
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        )
        .await;
        assert_eq!(hits(&replies[0].reply).0, &query);
        assert_eq!(echoed_scope_row(&replies[0].reply), None);
        assert!(!finds_goals(&replies[0].reply), "{:?}", replies[0].reply);

        // MOD-147 M4 R1 M1: a search answered without a task echoes its scope row too.
        let none = concepts::query("zircon", scope.project_ids.clone(), false, 0);
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime.serve(
            &backend,
            &tx,
            &envelope(
                4,
                StoreRequest::SearchConcepts {
                    query: none,
                    scope_row_of: Some(scope.workspace_id),
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        );
        let ConceptsServed::Reply(reply) = served else {
            panic!("a limit of 0 is answered at once: {served:?}")
        };
        assert_eq!(echoed_scope_row(&reply), Some(scope.workspace_id));
    }

    /// MOD-147 M4 D8: an index run with `scope_row_of` indexes the scope row's points too, and one
    /// without it the members alone.
    #[tokio::test]
    async fn an_index_run_with_the_scope_row_reports_its_points() {
        let (backend, scope, goals) = platform_with_goals().await;
        let of_the_row = |index: &MemIndex| {
            index
                .store(&PLATFORM)
                .points()
                .into_iter()
                .filter(|p| p.project_id == ids::PROJECT_SCOPE_PLATFORM)
                .collect::<Vec<_>>()
        };

        let index = Arc::new(MemIndex::new());
        let mut runtime = ConceptsRuntime::new(index.clone());
        let request = StoreRequest::IndexConcepts {
            scope: scope.clone(),
            scope_row_of: Some(scope.workspace_id),
        };
        let replies = serve_one(&mut runtime, &backend, &envelope(1, request)).await;
        let report = *indexed(&replies[0].reply).as_ref().expect("no failure");
        assert_eq!(
            report.points_upserted,
            index.store(&PLATFORM).points().len()
        );
        let row = of_the_row(&index);
        assert!(
            row.iter()
                .any(|p| p.subject.owner() == Owner::Document(goals.id)),
            "{row:?}"
        );

        let members = Arc::new(MemIndex::new());
        let mut runtime = ConceptsRuntime::new(members.clone());
        let request = StoreRequest::IndexConcepts {
            scope,
            scope_row_of: None,
        };
        serve_one(&mut runtime, &backend, &envelope(1, request)).await;
        assert!(!members.store(&PLATFORM).points().is_empty());
        assert!(of_the_row(&members).is_empty());
    }

    #[tokio::test]
    async fn index_and_search_errors_are_concepts_replies_never_failed() {
        let (backend, scope) = platform().await;
        let message = "qdrant: query: refused";
        let mut runtime = ConceptsRuntime::new(Arc::new(MemIndex::new().failing(message)));
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                1,
                StoreRequest::SearchConcepts {
                    query,
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        )
        .await;
        assert_eq!(hits(&replies[0].reply).1, &Err(message.to_owned()));

        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                2,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        )
        .await;
        assert_eq!(indexed(&replies[0].reply), Err(message.to_owned()));
        assert!(
            !matches!(replies[0].reply, StoreReply::Failed { .. }),
            "never the status line's variant"
        );
    }

    #[tokio::test]
    async fn without_a_runtime_both_requests_answer_not_available() {
        let (backend, scope) = platform().await;
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        let reply = store_worker::serve(
            &backend,
            &StoreRequest::SearchConcepts {
                query: query.clone(),
                scope_row_of: None,
                workspace: ids::WORKSPACE_PLATFORM,
            },
        )
        .await;
        let (echoed, outcome) = hits(&reply);
        assert_eq!(echoed, &query);
        assert_eq!(outcome, &Err(NOT_AVAILABLE.to_owned()));
        // MOD-147 M4 R1 M1: the scope row is echoed as asked.
        let reply = store_worker::serve(
            &backend,
            &StoreRequest::SearchConcepts {
                query: query.clone(),
                scope_row_of: Some(scope.workspace_id),
                workspace: ids::WORKSPACE_PLATFORM,
            },
        )
        .await;
        assert_eq!(echoed_scope_row(&reply), Some(scope.workspace_id));

        let reply = store_worker::serve(
            &backend,
            &StoreRequest::IndexConcepts {
                scope,
                scope_row_of: None,
            },
        )
        .await;
        assert_eq!(indexed(&reply), Err(NOT_AVAILABLE.to_owned()));
    }

    #[tokio::test]
    async fn request_names_match_the_name_arms() {
        let (_, scope) = platform().await;
        let search = StoreRequest::SearchConcepts {
            query: concepts::query("x", Vec::new(), false, 1),
            scope_row_of: None,
            workspace: ids::WORKSPACE_PLATFORM,
        };
        assert_eq!(REQUEST_NAMES[0], search.name());
        assert_eq!(
            REQUEST_NAMES[1],
            StoreRequest::IndexConcepts {
                scope,
                scope_row_of: None,
            }
            .name()
        );
        assert_eq!(REQUEST_NAMES[2], StoreRequest::ProbeQdrant.name());
        assert!(
            !StoreRequest::ProbeQdrant.is_qdrant(),
            "the probe is served off the loop, never by `qdrant_settings_info` (P-7)"
        );
    }

    /// MOD-160 D9: a probe is a task of the runtime, answered in its own variant and never
    /// `Failed`; an index that cannot look answers "nothing probed".
    #[tokio::test]
    async fn a_probe_is_a_concepts_reply_task_and_never_failed() {
        let (backend, _) = platform().await;
        let check = QdrantCheck::NoKey("qdrant: access refused".to_owned());
        let mut runtime = ConceptsRuntime::new(Arc::new(MemIndex::new().with_check(check.clone())));
        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(7, StoreRequest::ProbeQdrant),
        )
        .await;
        let [reply] = replies.as_slice() else {
            panic!("one answer: {replies:?}")
        };
        assert_eq!((reply.seq, &reply.origin), (7, &PROBE));
        assert!(
            matches!(&reply.reply, StoreReply::QdrantProbe(Some(said)) if *said == check),
            "{:?}",
            reply.reply
        );

        let mut runtime = ConceptsRuntime::new(Arc::new(MemIndex::new()));
        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(8, StoreRequest::ProbeQdrant),
        )
        .await;
        assert!(
            matches!(
                replies.as_slice(),
                [ReplyEnvelope {
                    seq: 8,
                    reply: StoreReply::QdrantProbe(None),
                    ..
                }]
            ),
            "{replies:?}"
        );
    }

    /// MOD-160 D9: the production index's probe of a closed port answers "unavailable", within
    /// the probe's bound, and loads no model.
    #[tokio::test]
    async fn a_qdrant_index_probe_of_a_closed_port_is_unavailable() {
        let loads = Arc::new(AtomicUsize::new(0));
        let index = QdrantIndex::with_loader(counting(&loads)).with_source(Arc::new(source(
            Some("http://127.0.0.1:1"),
            None,
            no_key,
        )));
        let check = tokio::time::timeout(Duration::from_secs(10), index.check())
            .await
            .expect("the probe is bounded");
        assert!(
            matches!(check, Some(QdrantCheck::Unavailable(_))),
            "{check:?}"
        );
        assert_eq!(loads.load(Ordering::SeqCst), 0, "no model was loaded");
        assert!(nothing_cached(&index), "a probe caches nothing");
    }

    /// A source over `env_url` and `env_key`, its keyring answering `keyring`, nothing published.
    fn source(env_url: Option<&str>, env_key: Option<&str>, keyring: KeyringRead) -> QdrantSource {
        QdrantSource::new(
            QdrantEnv::new(env_url.map(str::to_owned), env_key.map(str::to_owned)),
            KeySource::Interactive,
        )
        .with_keyring(keyring)
    }

    /// A keyring holding no key.
    fn no_key() -> htui_core::store::Result<Option<String>> {
        Ok(None)
    }

    fn url(text: &str) -> QdrantUrl {
        QdrantUrl::parse(text).expect("a valid URL")
    }

    /// Sets the row of `store` to `text`, over whatever it holds.
    async fn set_row(store: &MemStore, text: &str) {
        let token = store
            .qdrant_url()
            .await
            .expect("the memory store never fails")
            .map(|row| row.token);
        let outcome = store
            .set_qdrant_url(&url(text), token)
            .await
            .expect("the memory store never fails");
        assert!(
            matches!(outcome, htui_core::store::CasOutcome::Applied(_)),
            "{outcome:?}"
        );
    }

    /// MOD-158 D6: no store published (offline, `--demo`, before a connect) and no override.
    #[tokio::test]
    async fn no_published_store_reads_as_postgres_not_connected() {
        let source = source(None, None, no_key);
        let why = source
            .url()
            .await
            .expect_err("nothing to read the row through");
        assert!(matches!(why, Unresolved::Offline), "{why:?}");
        assert_eq!(why.sentence(), URL_OFFLINE);
        let why = source.settings().await.expect_err("no URL, no settings");
        assert_eq!(why.sentence(), URL_OFFLINE);

        source.publish(Some(Arc::new(MemStore::new())));
        source.publish(None);
        assert!(
            matches!(source.url().await, Err(Unresolved::Offline)),
            "a withdrawn store is offline again"
        );
    }

    /// MOD-158 D3: `HTUI_QDRANT_URL` needs no store, and wins over the row when there is one.
    #[tokio::test]
    async fn the_env_url_wins_and_needs_no_store() {
        let source = source(Some(" http://env:6334 "), None, no_key);
        assert_eq!(
            source.url().await.expect("the override"),
            url("http://env:6334")
        );
        let store = MemStore::new();
        set_row(&store, "http://row:6334").await;
        source.publish(Some(Arc::new(store)));
        let settings = source.settings().await.expect("the override");
        assert_eq!(settings.url, url("http://env:6334"));
        assert!(settings.api_key.is_none());
    }

    /// MOD-158 D3: an override that is not a Qdrant URL is an error naming the variable, never a
    /// fall-through to the row, and never echoes the value (it may hold a password).
    #[tokio::test]
    async fn an_invalid_env_url_is_an_error_never_a_fallback() {
        let source = source(Some("http://user:SENTINEL@env:6334"), None, no_key);
        let store = MemStore::new();
        set_row(&store, "http://row:6334").await;
        source.publish(Some(Arc::new(store)));
        let why = source.url().await.expect_err("an invalid override");
        let Unresolved::Unusable(sentence) = &why else {
            panic!("{why:?}")
        };
        assert!(
            sentence.starts_with("HTUI_QDRANT_URL is not a usable Qdrant URL: "),
            "{sentence}"
        );
        assert!(!sentence.contains("SENTINEL"), "{sentence}");
        assert!(!sentence.contains("row"), "{sentence}");
        assert!(source.settings().await.is_err());
    }

    /// MOD-158 D3: `Debug` prints the override only once it parses; a refused one may hold a
    /// password and is never echoed, and the key never is.
    #[test]
    fn the_debug_never_prints_an_unusable_override() {
        let shown = format!(
            "{:?}",
            source(
                Some("http://user:SENTINEL@env:6334"),
                Some("KEY-SENTINEL"),
                no_key
            )
        );
        assert!(!shown.contains("SENTINEL"), "{shown}");
        assert!(shown.contains("<not a usable URL>"), "{shown}");
        let shown = format!("{:?}", source(Some(" http://env:6334 "), None, no_key));
        assert!(shown.contains("http://env:6334"), "{shown}");
        let shown = format!("{:?}", source(None, None, no_key));
        assert!(!shown.contains("usable"), "{shown}");
    }

    /// MOD-158 D6: the row is read per call, so one set between two reads is read by the second.
    #[tokio::test]
    async fn a_row_set_between_two_reads_is_read_by_the_second() {
        let source = source(None, None, no_key);
        let store = MemStore::new();
        source.publish(Some(Arc::new(store.clone())));
        let why = source.url().await.expect_err("no row yet");
        assert!(matches!(why, Unresolved::NoUrl), "{why:?}");
        assert_eq!(why.sentence(), concepts::NO_QDRANT_URL);

        set_row(&store, "http://first:6334").await;
        assert_eq!(
            source.url().await.expect("the row"),
            url("http://first:6334")
        );
        set_row(&store, "http://second:6334").await;
        let settings = source.settings().await.expect("the row");
        assert_eq!(settings.url, url("http://second:6334"));
    }

    /// MOD-158 D1, P-3: a malformed row is an error naming the key, never "no URL".
    #[tokio::test]
    async fn a_malformed_row_is_an_error_naming_the_key_not_no_url() {
        let source = source(None, None, no_key);
        let store = MemStore::new();
        store.set_app_setting(htui_core::qdrant::QDRANT_URL_KEY, serde_json::json!(42));
        source.publish(Some(Arc::new(store)));
        let why = source.url().await.expect_err("a malformed row");
        let Unresolved::Unusable(sentence) = &why else {
            panic!("{why:?}")
        };
        assert!(sentence.contains("`qdrant_url`"), "{sentence}");
        assert!(sentence.contains("htui --set-qdrant-url"), "{sentence}");
    }

    /// A row that cannot be read is said as such, not as "no URL".
    #[tokio::test]
    async fn an_unreadable_row_is_a_read_error() {
        struct Broken;
        impl UrlRow for Broken {
            fn qdrant_url(&self) -> BoxFuture<'_, Result<Option<StoredQdrantUrl>, StoreError>> {
                Box::pin(async { Err(StoreError::Unreachable("refused".into())) })
            }
        }
        let source = source(None, None, no_key);
        source.publish(Some(Arc::new(Broken)));
        let why = source.url().await.expect_err("an unreadable row");
        assert!(matches!(why, Unresolved::Read(_)), "{why:?}");
        assert!(why.sentence().contains("could not be read"), "{why}");
    }

    /// MOD-158 D4: `HTUI_QDRANT_API_KEY` wins over the keyring; without it the keyring's key is
    /// read, and a keyring that fails is an error for an interactive process.
    #[tokio::test]
    async fn the_env_key_wins_over_the_keyring() {
        fn ring() -> htui_core::store::Result<Option<String>> {
            Ok(Some("ring-key".to_owned()))
        }
        fn broken() -> htui_core::store::Result<Option<String>> {
            Err(StoreError::Backend("no Secret Service".into()))
        }
        let key = |settings: &QdrantSettings| settings.api_key.as_deref().cloned();

        let env = source(Some("http://env:6334"), Some("env-key"), ring);
        let settings = env.settings().await.expect("settings");
        assert_eq!(key(&settings).as_deref(), Some("env-key"));
        let env = source(Some("http://env:6334"), Some("env-key"), broken);
        assert!(env.settings().await.is_ok(), "the keyring is not read");

        let keyring = source(Some("http://env:6334"), None, ring);
        let settings = keyring.settings().await.expect("settings");
        assert_eq!(key(&settings).as_deref(), Some("ring-key"));
        let broken = source(Some("http://env:6334"), None, broken);
        let why = broken.settings().await.expect_err("an unreadable keyring");
        assert!(matches!(why, Unresolved::Key(_)), "{why:?}");
        assert!(
            source(None, None, ring).settings().await.is_err(),
            "no URL: the key alone is nothing"
        );
    }

    /// MOD-158 D4: `htui worker`'s source: its stdin line, and nothing after it, when given.
    #[tokio::test]
    async fn a_headless_key_line_is_the_key() {
        fn ring() -> htui_core::store::Result<Option<String>> {
            Ok(Some("ring-key".to_owned()))
        }
        let source = source(Some("http://env:6334"), None, ring);
        source.set_key_source(KeySource::Headless {
            stdin: Some(Zeroizing::new("line-key".to_owned())),
            credentials_dir: None,
        });
        let settings = source.settings().await.expect("settings");
        assert_eq!(
            settings.api_key.as_deref().map(String::as_str),
            Some("line-key")
        );
        let shown = format!("{source:?}");
        assert!(!shown.contains("line-key"), "{shown}");
    }

    /// A `qdrant_url` row that counts its reads, waits `delay` (paused clock) before each answer,
    /// and answers a stored URL while `present`, else no row.
    struct CountingRow {
        /// The row behind it, set to a usable URL.
        store: MemStore,
        /// Whether the row is there.
        present: AtomicBool,
        /// How many reads began.
        reads: AtomicUsize,
        /// How long each read waits.
        delay: Duration,
    }

    impl CountingRow {
        async fn new(present: bool, delay: Duration) -> Arc<Self> {
            let store = MemStore::new();
            set_row(&store, "http://row:6334").await;
            Arc::new(Self {
                store,
                present: AtomicBool::new(present),
                reads: AtomicUsize::new(0),
                delay,
            })
        }

        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }

        fn set_present(&self, present: bool) {
            self.present.store(present, Ordering::SeqCst);
        }
    }

    impl UrlRow for CountingRow {
        fn qdrant_url(&self) -> BoxFuture<'_, Result<Option<StoredQdrantUrl>, StoreError>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                tokio::time::sleep(self.delay).await;
                if self.present.load(Ordering::SeqCst) {
                    UrlRow::qdrant_url(&self.store).await
                } else {
                    Ok(None)
                }
            })
        }
    }

    /// Lets every spawned read that is due finish (a paused clock advances on its own).
    async fn settle_reads() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    /// The flag as last recorded, without starting a read.
    fn known(source: &QdrantSource) -> Option<bool> {
        source
            .configured
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .map(|(offered, _)| offered)
    }

    /// A current-thread runtime on a paused clock: what is published before `block_on` is
    /// published with no runtime, so it starts no read.
    fn paused_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("a runtime")
    }

    /// MOD-160 D5: unknown is "not offered", and asks once however often it is asked.
    #[test]
    fn an_unknown_offer_is_false_and_starts_one_read() {
        let runtime = paused_runtime();
        let row = runtime.block_on(CountingRow::new(true, Duration::from_secs(1)));
        let source = source(None, None, no_key);
        source.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        assert_eq!(known(&source), None, "no runtime, no read");
        runtime.block_on(async {
            for _ in 0..3 {
                assert!(!source.offered(), "unknown is not offered");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert_eq!(row.reads(), 1, "one read for every unknown answer");
            assert!(source.offered(), "the row resolves");
            assert_eq!(row.reads(), 1, "a fresh answer starts nothing");
        });
    }

    /// MOD-160 D5: a publish starts the read, so the first session after it knows.
    #[tokio::test(start_paused = true)]
    async fn a_url_row_is_offered_after_the_read() {
        let source = source(None, None, no_key);
        let row = CountingRow::new(true, Duration::ZERO).await;
        source.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        settle_reads().await;
        assert_eq!(known(&source), Some(true));
        assert!(source.offered());
        assert_eq!(row.reads(), 1);
    }

    /// MOD-160 D5: offline and no row are not offered; a `None` publish needs no read.
    #[tokio::test(start_paused = true)]
    async fn no_url_and_offline_are_not_offered() {
        let source = source(None, None, no_key);
        source.publish(None);
        assert_eq!(
            known(&source),
            Some(false),
            "recorded at once, with no task"
        );
        assert!(!source.offered());

        let row = CountingRow::new(false, Duration::ZERO).await;
        source.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        settle_reads().await;
        for _ in 0..3 {
            assert!(!source.offered(), "no row is not offered");
        }
        settle_reads().await;
        assert_eq!(row.reads(), 1, "the publish's read, and no other");
    }

    /// MOD-160 D5: `HTUI_QDRANT_URL` wins over every row, so its verdict is known at once.
    #[tokio::test(start_paused = true)]
    async fn an_override_is_known_without_a_read() {
        let usable = source(Some("http://env:6334"), None, no_key);
        assert_eq!(known(&usable), Some(true), "known before any read");
        assert!(usable.offered());
        let row = CountingRow::new(false, Duration::ZERO).await;
        usable.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        settle_reads().await;
        assert!(usable.offered(), "the row is never read over the override");
        assert_eq!(row.reads(), 0);

        let unusable = source(Some("http://u:SENTINEL@env:6334"), None, no_key);
        assert_eq!(known(&unusable), Some(false));
        unusable.publish(None);
        assert!(
            !unusable.offered(),
            "offline changes nothing for an override"
        );
    }

    /// MOD-160 D5: an answer older than 30 s is still answered, and read again once, however many
    /// sessions open meanwhile.
    #[tokio::test(start_paused = true)]
    async fn a_stale_answer_is_used_and_read_again_once() {
        let source = source(None, None, no_key);
        let row = CountingRow::new(true, Duration::from_secs(1)).await;
        source.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(source.offered());
        assert_eq!(row.reads(), 1);

        row.set_present(false);
        tokio::time::sleep(OFFER_STALE_AFTER + Duration::from_secs(1)).await;
        for _ in 0..5 {
            assert!(source.offered(), "the old answer, while the read runs");
            tokio::task::yield_now().await;
        }
        assert_eq!(row.reads(), 2, "one re-read");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!source.offered(), "the re-read's answer");
        assert_eq!(row.reads(), 2, "a fresh answer starts nothing");
    }

    /// MOD-160 D5: a slow read of the old store lands after the new store's answer, and is dropped.
    #[tokio::test(start_paused = true)]
    async fn a_read_of_the_old_store_never_overwrites_the_new_one() {
        let source = source(None, None, no_key);
        let old = CountingRow::new(true, Duration::from_secs(5)).await;
        let new = CountingRow::new(false, Duration::ZERO).await;
        source.publish(Some(Arc::clone(&old) as Arc<dyn UrlRow>));
        tokio::task::yield_now().await;
        source.publish(Some(Arc::clone(&new) as Arc<dyn UrlRow>));
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!((old.reads(), new.reads()), (1, 1));
        assert_eq!(known(&source), Some(false), "the new store's answer stands");
    }

    /// MOD-160 D5: an in-process URL write or clear re-reads at once.
    #[tokio::test(start_paused = true)]
    async fn a_reread_after_a_write_picks_up_a_cleared_row() {
        let source = source(None, None, no_key);
        let row = CountingRow::new(true, Duration::ZERO).await;
        source.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        settle_reads().await;
        assert!(source.offered());

        row.set_present(false);
        source.reread();
        settle_reads().await;
        assert_eq!(row.reads(), 2);
        assert!(!source.offered(), "the cleared row");
    }

    /// MOD-160 D5: every settings resolution records the flag, a search's included, before the
    /// key and whatever Qdrant answers.
    #[test]
    fn a_search_records_the_offer() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let row = runtime.block_on(async {
            let row = CountingRow::new(true, Duration::ZERO).await;
            set_row(&row.store, "http://127.0.0.1:1").await;
            row
        });
        fn broken() -> htui_core::store::Result<Option<String>> {
            Err(StoreError::Backend("no Secret Service".into()))
        }
        let source = Arc::new(source(None, None, no_key));
        source.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        let index = QdrantIndex::with_loader(|| Box::pin(std::future::pending()))
            .with_source(Arc::clone(&source));
        runtime.block_on(async {
            let why = index
                .search(SearchIn::Workspaces(vec![WorkspaceId::new()]), any_query())
                .await
                .expect_err("port 1");
            assert!(why.starts_with("cannot reach Qdrant"), "{why}");
        });
        assert_eq!(known(&source), Some(true), "recorded by the search itself");
        assert_eq!(row.reads(), 1);

        // A URL that resolves is configured even when the key cannot be read.
        let keyless = Arc::new(
            QdrantSource::new(QdrantEnv::default(), KeySource::Interactive).with_keyring(broken),
        );
        keyless.publish(Some(Arc::clone(&row) as Arc<dyn UrlRow>));
        runtime.block_on(async {
            assert!(keyless.settings().await.is_err(), "the key cannot be read");
        });
        assert_eq!(known(&keyless), Some(true));
    }

    /// MOD-160 D5: the runtime hands a URL write to its index.
    #[test]
    fn the_runtime_forwards_a_url_change() {
        let index = Arc::new(Recording::default());
        let runtime = ConceptsRuntime::new(index.clone());
        runtime.url_changed();
        runtime.url_changed();
        assert_eq!(index.1.load(Ordering::SeqCst), 2);
    }

    /// MOD-160 D5: the production index answers the offer from its source.
    #[tokio::test(start_paused = true)]
    async fn the_qdrant_index_offers_from_its_source() {
        let index = QdrantIndex::with_loader(|| Box::pin(std::future::pending()))
            .with_source(Arc::new(source(None, None, no_key)));
        ConceptIndex::publish(&index, None);
        assert!(!ConceptIndex::offered(&index));
        let index = QdrantIndex::with_loader(|| Box::pin(std::future::pending()))
            .with_source(Arc::new(source(Some("http://env:6334"), None, no_key)));
        assert!(ConceptIndex::offered(&index));
        assert!(
            ConceptIndex::offered(&MemIndex::new()),
            "an index without settings is offered"
        );
    }

    /// An index that records what is published to it, and counts the URL changes it is told.
    #[derive(Default)]
    struct Recording(Mutex<Vec<bool>>, AtomicUsize);

    impl ConceptIndex for Recording {
        fn search(&self, _: SearchIn, _: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn sync(
            &self,
            _: Backend,
            _: IndexedCollection,
            _: Scope,
            _: Box<dyn DeleteGuardDyn>,
        ) -> BoxFuture<'_, Result<SyncReport, String>> {
            Box::pin(async { Ok(SyncReport::default()) })
        }
        fn publish(&self, store: Option<Arc<dyn UrlRow>>) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(store.is_some());
        }
        fn url_changed(&self) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// MOD-158 D6: only a writable Postgres is published; memory and the mirror are `None`.
    #[tokio::test]
    async fn the_runtime_publishes_the_writable_store_only() {
        let index = Arc::new(Recording::default());
        let mut runtime = ConceptsRuntime::new(index.clone());
        runtime.publish(&Backend::memory(MemStore::demo()));

        let identity = htui_store::Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = PgStore::lazy(
            "postgres://nobody:nothing@127.0.0.1:1/none",
            &identity,
            Duration::from_millis(250),
        )
        .expect("a lazy pool opens no socket");
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "concepts-publish", 1)
            .await
            .expect("open a throwaway mirror");
        let mut backend = Backend::Online {
            pg,
            cache: cache.clone(),
        };
        runtime.publish(&backend);
        assert!(backend.went_offline());
        runtime.publish(&backend);
        assert_eq!(
            *index.0.lock().unwrap_or_else(PoisonError::into_inner),
            [false, true, false]
        );
        cache.close().await;
    }

    /// MOD-158 R1 (B-L2): the store loop publishes at the top of every iteration, but the index
    /// is handed a store only when the backend's writable store changed: a repeat is skipped, while
    /// a lost server, a reconnect and another server (a new pool, as `SetDsn` then `go_online`
    /// make) are each published.
    #[tokio::test]
    async fn the_runtime_publishes_only_a_changed_store() {
        let index = Arc::new(Recording::default());
        let mut runtime = ConceptsRuntime::new(index.clone());
        let published = || {
            index
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        };
        let memory = Backend::memory(MemStore::demo());
        runtime.publish(&memory);
        runtime.publish(&memory);
        assert_eq!(published(), [false], "the first publish, then nothing new");

        let identity = htui_store::Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let lazy = || {
            PgStore::lazy(
                "postgres://nobody:nothing@127.0.0.1:1/none",
                &identity,
                Duration::from_millis(250),
            )
            .expect("a lazy pool opens no socket")
        };
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "concepts-publish-once", 1)
            .await
            .expect("open a throwaway mirror");
        let mut backend = Backend::Online {
            pg: lazy(),
            cache: cache.clone(),
        };
        runtime.publish(&backend);
        runtime.publish(&backend);
        assert_eq!(published(), [false, true], "online once");
        assert!(backend.went_offline());
        runtime.publish(&backend);
        runtime.publish(&backend);
        assert_eq!(published(), [false, true, false], "offline once");

        let first = lazy();
        let online = Backend::Online {
            pg: first.clone(),
            cache: cache.clone(),
        };
        runtime.publish(&online);
        runtime.publish(&Backend::Online {
            pg: first,
            cache: cache.clone(),
        });
        assert_eq!(
            published(),
            [false, true, false, true],
            "a clone of the same pool is the same store"
        );
        runtime.publish(&Backend::Online {
            pg: lazy(),
            cache: cache.clone(),
        });
        assert_eq!(
            published(),
            [false, true, false, true, true],
            "another pool is published"
        );
        cache.close().await;
    }

    /// MOD-158 D8: a published store without a row is "no URL", naming both places to set one,
    /// read before the model and without any keyring.
    #[tokio::test]
    async fn qdrant_index_without_a_stored_url_names_where_to_set_it() {
        let (backend, scope) = platform().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let settings = Arc::new(source(None, None, no_key));
        settings.publish(Some(Arc::new(MemStore::new())));
        let index = QdrantIndex::with_loader({
            let attempts = Arc::clone(&attempts);
            move || {
                attempts.fetch_add(1, Ordering::SeqCst);
                load_model()
            }
        })
        .with_source(settings);
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        let searched = index
            .search(SearchIn::Workspaces(vec![scope.workspace_id]), query)
            .await
            .expect_err("no URL is stored");
        for place in ["Settings > Qdrant", "htui --set-qdrant-url"] {
            assert!(searched.contains(place), "{searched}");
        }
        let synced = index
            .sync(
                backend,
                IndexedCollection::Workspace(scope.workspace_id),
                scope,
                Box::new(Unguarded),
            )
            .await
            .expect_err("no URL is stored");
        assert_eq!(synced, searched);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            0,
            "the settings are read before the model: nothing was loaded"
        );
    }

    /// MOD-158 D6: the index reads through what is published to it.
    #[tokio::test]
    async fn the_index_reads_the_store_published_to_it() {
        let index = QdrantIndex::with_loader(|| Box::pin(std::future::pending()))
            .with_source(Arc::new(source(None, None, no_key)));
        let query = concepts::query(
            "anything",
            vec![htui_core::model::ProjectId::new()],
            false,
            1,
        );
        assert_eq!(
            index
                .search(SearchIn::Everywhere, query.clone())
                .await
                .expect_err("offline"),
            URL_OFFLINE
        );
        ConceptIndex::publish(&index, Some(Arc::new(MemStore::new())));
        assert_eq!(
            index
                .search(SearchIn::Everywhere, query)
                .await
                .expect_err("no row"),
            concepts::NO_QDRANT_URL
        );
    }

    /// A loader that counts its calls and never finishes: a test that sees a load fails its count.
    fn counting(
        loads: &Arc<AtomicUsize>,
    ) -> impl Fn() -> BoxFuture<'static, Result<RtenEmbedder, String>> + Send + Sync + 'static {
        let loads = Arc::clone(loads);
        move || {
            loads.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::pending())
        }
    }

    /// A query over one project, so the runtime would not answer it empty itself.
    fn any_query() -> SearchQuery {
        concepts::query(
            "anything",
            vec![htui_core::model::ProjectId::new()],
            false,
            10,
        )
    }

    /// Whether the index caches no connection.
    fn nothing_cached(index: &QdrantIndex) -> bool {
        index
            .connections
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    /// MOD-160 D2: a search asks Qdrant before the model, so a Qdrant that is down costs no
    /// 133 MB download and caches nothing.
    #[tokio::test]
    async fn a_search_against_an_unreachable_qdrant_loads_no_model() {
        let loads = Arc::new(AtomicUsize::new(0));
        let index = QdrantIndex::with_loader(counting(&loads)).with_source(Arc::new(source(
            Some("http://127.0.0.1:1"),
            None,
            no_key,
        )));

        let why = index
            .search(SearchIn::Workspaces(vec![WorkspaceId::new()]), any_query())
            .await
            .expect_err("nothing listens on port 1");
        assert!(why.starts_with("cannot reach Qdrant"), "{why}");
        assert_eq!(loads.load(Ordering::SeqCst), 0, "no model was loaded");
        assert!(nothing_cached(&index), "nothing is cached");
    }

    /// `HTUI_TEST_QDRANT_URL` (never `HTUI_QDRANT_*`, blueprint P-13) and this case's turn at it
    /// (TOOL-16: the live cases of one binary run one at a time), or the skip line.
    async fn test_qdrant() -> Option<(htui_store::testkit::QdrantTurn, String)> {
        let Ok(url) = std::env::var("HTUI_TEST_QDRANT_URL") else {
            eprintln!("skipped: HTUI_TEST_QDRANT_URL not set");
            return None;
        };
        Some((htui_store::testkit::qdrant_turn().await, url))
    }

    /// A collection name no other test uses.
    fn fresh_collection() -> String {
        format!("htui_test_{}", Uuid::now_v7().simple())
    }

    /// MOD-160 D2 (live): a TUI search of a collection that does not exist answers no hits, creates
    /// nothing and loads no model.
    #[tokio::test]
    async fn live_qdrant_a_tui_search_against_an_empty_qdrant_creates_no_collection_and_loads_nothing()
     {
        let Some((_turn, url)) = test_qdrant().await else {
            return;
        };
        let fresh = fresh_collection();
        let loads = Arc::new(AtomicUsize::new(0));
        let index = QdrantIndex::with_loader(counting(&loads)).with_source(Arc::new(source(
            Some(&url),
            None,
            no_key,
        )));

        // The per-collection read itself: the global collection a full search adds may exist in
        // a shared test Qdrant, and would load the model.
        let hits = index
            .search_collections(
                &settings(&url, None),
                std::slice::from_ref(&fresh),
                &any_query(),
            )
            .await
            .expect("an empty index is not a failure");
        assert!(hits.is_empty(), "{hits:?}");
        let found = ExistingCollection::find_in(&settings(&url, None), &fresh)
            .await
            .expect("Qdrant answers");
        assert!(found.is_none(), "the search created the collection");
        assert_eq!(loads.load(Ordering::SeqCst), 0, "no model was loaded");
        assert!(nothing_cached(&index), "nothing is cached");
    }

    /// MOD-160 D3 (live): a collection another embedder wrote is refused before the model loads.
    #[tokio::test]
    async fn live_qdrant_a_tui_search_of_another_embedders_collection_is_refused_before_the_model()
    {
        let Some((_turn, url)) = test_qdrant().await else {
            return;
        };
        // A fresh workspace's collection, searched before the global one.
        let workspace = WorkspaceId::new();
        let fresh = IndexedCollection::Workspace(workspace).name();
        let created = htui_store::testkit::qdrant_connect_to(
            &settings(&url, None),
            htui_store::embed::HashEmbedder::new(DENSE_DIM),
            &fresh,
        )
        .await
        .expect("Qdrant answers");
        let loads = Arc::new(AtomicUsize::new(0));
        let index = QdrantIndex::with_loader(counting(&loads)).with_source(Arc::new(source(
            Some(&url),
            None,
            no_key,
        )));

        let searched = index
            .search(SearchIn::Workspaces(vec![workspace]), any_query())
            .await;
        created.drop_collection().await.expect("drop");
        let why = searched.expect_err("another embedder's collection");
        assert!(why.contains(vector::EMBEDDER_MISMATCH), "{why}");
        assert_eq!(loads.load(Ordering::SeqCst), 0, "no model was loaded");
    }

    /// MOD-160 D9: every warning state, the info line, and what says nothing.
    #[test]
    fn qdrant_check_names_each_state() {
        let keyless = settings("http://qdrant:6334", None);
        let keyed = settings("http://qdrant:6334", Some("key"));
        let unusable = Unresolved::Unusable("the `qdrant_url` setting is not a usable URL".into());
        let read = Unresolved::Read(StoreError::Unreachable("refused".into()));
        let key = Unresolved::Key(QdrantKeyError::Keyring("no Secret Service".into()));
        let check = QdrantCheck::of;

        assert_eq!(
            check(Err(&Unresolved::NoUrl), None),
            Some(QdrantCheck::NotConfigured(
                concepts::NO_QDRANT_URL.to_owned()
            ))
        );
        assert_eq!(
            check(Err(&unusable), None),
            Some(QdrantCheck::NotConfigured(unusable.sentence()))
        );
        assert_eq!(
            check(Err(&Unresolved::Offline), None),
            None,
            "nothing probed"
        );
        // R1 (review gate): the row or this box's keyring unreadable never contacted Qdrant, so it
        // is never said to be unreachable.
        for why in [&read, &key] {
            let unreadable = check(Err(why), None).expect("a warning");
            assert_eq!(unreadable, QdrantCheck::Unreadable(why.sentence()));
            assert!(
                !unreadable
                    .state()
                    .unwrap_or_default()
                    .contains("unreachable"),
                "{unreadable:?}"
            );
        }

        for probe in [Probe::Ready, Probe::NoCollection] {
            assert_eq!(
                check(Ok(&keyless), Some(&probe)),
                Some(QdrantCheck::Keyless)
            );
            assert_eq!(check(Ok(&keyed), Some(&probe)), Some(QdrantCheck::Ready));
        }
        for probe in [
            Probe::Unreachable("cannot reach".into()),
            Probe::Mismatch("qdrant: embedder mismatch: other".into()),
        ] {
            let (Probe::Unreachable(sentence) | Probe::Mismatch(sentence)) = &probe else {
                unreachable!()
            };
            for settings in [&keyless, &keyed] {
                assert_eq!(
                    check(Ok(settings), Some(&probe)),
                    Some(QdrantCheck::Unavailable(sentence.clone()))
                );
            }
        }
        let refused = Probe::Refused("qdrant: access refused".into());
        assert_eq!(
            check(Ok(&keyed), Some(&refused)),
            Some(QdrantCheck::KeyRefused("qdrant: access refused".into()))
        );
        assert_eq!(
            check(Ok(&keyless), Some(&refused)),
            Some(QdrantCheck::NoKey("qdrant: access refused".into()))
        );
        assert_eq!(check(Ok(&keyed), None), None, "nothing probed");

        let cases = [
            (
                QdrantCheck::NotConfigured("why".into()),
                "no Qdrant is configured",
            ),
            (
                QdrantCheck::Unavailable("why".into()),
                "Qdrant is unreachable or refuses the collection",
            ),
            (
                QdrantCheck::Unreadable("why".into()),
                "this box cannot read the Qdrant settings",
            ),
            (
                QdrantCheck::KeyRefused("why".into()),
                "the Qdrant refused this box's API key",
            ),
            (
                QdrantCheck::NoKey("why".into()),
                "this box lacks the Qdrant API key",
            ),
        ];
        for (check, state) in cases {
            assert_eq!(check.state(), Some(state), "{check:?}");
            assert_eq!(check.warning(), Some(format!("{state}: why")), "{check:?}");
            assert_eq!(check.info(), None, "{check:?}");
        }
        assert_eq!(
            QdrantCheck::Keyless.info(),
            Some("this Qdrant answers without an API key")
        );
        for quiet in [QdrantCheck::Keyless, QdrantCheck::Ready] {
            assert_eq!(quiet.state(), None, "{quiet:?}");
            assert_eq!(quiet.warning(), None, "{quiet:?}");
        }
        assert_eq!(QdrantCheck::Ready.info(), None);
    }

    /// MOD-160 D9: a check of a Qdrant that is down says so, and never says the key.
    #[tokio::test]
    async fn a_check_never_says_the_key() {
        let source = source(Some("http://127.0.0.1:1"), Some("KEY-SENTINEL"), no_key);
        let check = check_qdrant(&source, &IndexedCollection::Global.name())
            .await
            .expect("settings resolve, so Qdrant is probed");
        assert!(matches!(check, QdrantCheck::Unavailable(_)), "{check:?}");
        let said = [
            format!("{check:?}"),
            check.warning().unwrap_or_default(),
            check.state().unwrap_or_default().to_owned(),
        ];
        for line in said {
            assert!(!line.contains("KEY-SENTINEL"), "{line}");
        }
        assert!(
            check_qdrant(&source_offline(), &IndexedCollection::Global.name())
                .await
                .is_none(),
            "offline: nothing probed"
        );
    }

    /// No override, nothing published: offline.
    fn source_offline() -> QdrantSource {
        source(None, None, no_key)
    }

    /// MOD-11 T7, H-26: the TUI's concepts runtime and the MCP adapter search one index, so the
    /// process loads one embedding model.
    #[test]
    fn the_production_runtime_searches_the_process_wide_index() {
        let address = |index: &Arc<dyn ConceptIndex>| Arc::as_ptr(index).cast::<()>();
        let shared = shared_index();
        assert_eq!(address(&shared), address(&shared_index()));
        assert_eq!(
            Arc::as_ptr(&shared_qdrant()).cast::<()>(),
            address(&shared),
            "the worker's index job takes its model from the same index"
        );
        assert_eq!(
            address(&ConceptsRuntime::production().index),
            address(&shared)
        );
    }

    /// The collection the connection-cache cases cache under.
    const NAME: &str = "htui_global";

    fn settings(url: &str, key: Option<&str>) -> QdrantSettings {
        QdrantSettings::new(
            QdrantUrl::parse(url).expect("a valid URL"),
            key.map(str::to_owned),
        )
    }

    /// Review 3: a connection is reused for the URL and key it was made with, and only for them.
    #[test]
    fn a_connection_is_reused_for_its_own_settings_until_it_is_forgotten() {
        let connections = Connections::<u32>::default();
        let made_for = settings("http://qdrant:6334", Some("key"));
        assert!(
            connections.get(NAME, &made_for, Made::ReadOnly).is_none(),
            "nothing connected yet"
        );

        let first = Arc::new(1);
        connections.put(NAME, made_for.clone(), Made::Ensured, Arc::clone(&first));
        let reused = connections
            .get(NAME, &made_for, Made::Ensured)
            .expect("the same settings reuse it");
        assert!(Arc::ptr_eq(&reused, &first));
        for moved in [
            settings("http://qdrant:6334", Some("other")),
            settings("http://qdrant:6334", None),
            settings("http://elsewhere:6334", Some("key")),
        ] {
            for need in [Made::Ensured, Made::ReadOnly] {
                assert!(connections.get(NAME, &moved, need).is_none(), "{moved:?}");
            }
        }

        // An error on a connection another request has already replaced leaves the new one.
        let second = Arc::new(2);
        connections.put(NAME, made_for.clone(), Made::Ensured, Arc::clone(&second));
        connections.forget(&first);
        let kept = connections
            .get(NAME, &made_for, Made::Ensured)
            .expect("the replacement stays");
        assert!(Arc::ptr_eq(&kept, &second));

        connections.forget(&second);
        assert!(
            connections.get(NAME, &made_for, Made::ReadOnly).is_none(),
            "after an error the next request reconnects"
        );
    }

    /// MOD-160 D4: a store a search opened read-only (no payload indexes, no stamp, no MOD-144
    /// rebuild) serves later searches but never a sync, and never displaces an ensured one.
    #[test]
    fn a_read_only_connection_never_serves_a_sync() {
        let connections = Connections::<u32>::default();
        let made_for = settings("http://qdrant:6334", None);

        let read_only = Arc::new(1);
        connections.put(
            NAME,
            made_for.clone(),
            Made::ReadOnly,
            Arc::clone(&read_only),
        );
        let searched = connections
            .get(NAME, &made_for, Made::ReadOnly)
            .expect("a search takes it");
        assert!(Arc::ptr_eq(&searched, &read_only));
        assert!(
            connections.get(NAME, &made_for, Made::Ensured).is_none(),
            "a sync connects instead"
        );

        let ensured = Arc::new(2);
        connections.put(NAME, made_for.clone(), Made::Ensured, Arc::clone(&ensured));
        for need in [Made::Ensured, Made::ReadOnly] {
            let served = connections
                .get(NAME, &made_for, need)
                .expect("an ensured store");
            assert!(Arc::ptr_eq(&served, &ensured), "{need:?}");
        }

        // A search that opened read-only while an index run connected keeps the better one.
        connections.put(NAME, made_for.clone(), Made::ReadOnly, Arc::new(3));
        let kept = connections
            .get(NAME, &made_for, Made::Ensured)
            .expect("the ensured store stays");
        assert!(Arc::ptr_eq(&kept, &ensured));

        // A read-only store for other settings replaces it: the slot holds one connection.
        let elsewhere = settings("http://elsewhere:6334", None);
        connections.put(NAME, elsewhere.clone(), Made::ReadOnly, Arc::new(4));
        assert!(connections.get(NAME, &made_for, Made::ReadOnly).is_none());
        assert!(connections.get(NAME, &elsewhere, Made::ReadOnly).is_some());
    }

    /// MOD-158 R1 (B-L6): an unusable `HTUI_QDRANT_URL` is a `warn` line, never its value; a
    /// usable one stays the `info` line D3 asks for.
    #[test]
    fn an_unusable_env_url_is_a_warning_without_its_value() {
        #[derive(Clone, Default)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let logged = |url: &str| {
            let buffer = Buffer::default();
            let subscriber = tracing_subscriber::fmt()
                .with_writer({
                    let buffer = buffer.clone();
                    move || buffer.clone()
                })
                .with_ansi(false)
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                drop(QdrantSource::new(
                    QdrantEnv::new(Some(url.to_owned()), None),
                    KeySource::Interactive,
                ));
            });
            let bytes = buffer
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            String::from_utf8(bytes).expect("utf-8 logs")
        };
        let bad = logged("http://u:SENTINEL@q:6334");
        assert!(bad.contains(" WARN "), "{bad}");
        assert!(!bad.contains("SENTINEL"), "{bad}");
        let good = logged("http://q:6334");
        assert!(good.contains(" INFO "), "{good}");
    }

    /// MOD-158 R1 (B-L1): settings that stop resolving drop the cached connection, and the key it
    /// was made with; settings that resolve keep it.
    #[test]
    fn unresolved_settings_drop_the_cached_connection() {
        let connections = Connections::<u32>::default();
        let made_for = settings("http://qdrant:6334", Some("key"));
        connections.put(NAME, made_for.clone(), Made::Ensured, Arc::new(1));
        assert!(connections.resolved(Ok(made_for.clone())).is_ok());
        assert!(
            connections.get(NAME, &made_for, Made::Ensured).is_some(),
            "resolving keeps it"
        );
        for (why, made) in [
            (Unresolved::NoUrl, Made::Ensured),
            (Unresolved::Offline, Made::ReadOnly),
        ] {
            connections.put(NAME, made_for.clone(), made, Arc::new(2));
            assert!(connections.resolved(Err(why)).is_err());
            assert!(
                connections
                    .slots
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_empty(),
                "nothing cached once the settings stop resolving"
            );
        }
    }

    /// Review 2: every search that overlaps a load waits on that one load, superseded ones
    /// included, and a failed load is retried once, by the next search, not once per waiter.
    #[tokio::test(start_paused = true)]
    async fn superseded_searches_share_one_failing_load_and_the_next_search_retries_once() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let index = Arc::new(QdrantIndex::with_loader({
            let attempts = Arc::clone(&attempts);
            move || {
                attempts.fetch_add(1, Ordering::SeqCst);
                Box::pin(async {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    Err("no network".to_owned())
                })
            }
        }));

        // Five searches, each superseded (aborted) while the model loads, then one that waits.
        for _ in 0..5 {
            let index = Arc::clone(&index);
            let search = tokio::spawn(async move { index.embedder().await.map(drop) });
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            search.abort();
        }
        let waited = index.embedder().await.map(drop);
        assert_eq!(waited, Err("no network".to_owned()));
        // Long past any load a waiter could still start.
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "one load for every search that overlapped it"
        );

        let again = index.embedder().await.map(drop);
        assert_eq!(again, Err("no network".to_owned()));
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "the search after the failure retried, once"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_aborts_every_task() {
        let (backend, scope) = platform().await;
        let index = Arc::new(MemIndex::new().delayed(Duration::from_secs(3600)));
        let mut runtime = ConceptsRuntime::new(index);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        runtime.serve(
            &backend,
            &tx,
            &envelope(
                1,
                StoreRequest::SearchConcepts {
                    query,
                    scope_row_of: None,
                    workspace: ids::WORKSPACE_PLATFORM,
                },
            ),
        );
        runtime.serve(
            &backend,
            &tx,
            &envelope(
                2,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        );
        runtime.shutdown();
        assert_eq!(
            runtime.settle(Duration::from_millis(1)).await,
            0,
            "nothing left"
        );
        tokio::time::sleep(Duration::from_secs(7200)).await;
        assert!(rx.try_recv().is_err(), "no aborted task answered");
    }

    // MOD-159 M1 T4 (plan D9, D12): per-collection searches and the `Ctrl+R` lease.

    /// A hit of `project` with `snippet` and `score`, owned by a fresh item.
    fn hit(project_key: &str, snippet: &str, score: f32) -> Hit {
        Hit {
            point_type: PointType::Item,
            owner: Owner::Item(htui_core::model::ItemId::new()),
            key: project_key.to_owned(),
            document: None,
            resolution: None,
            state: None,
            score,
            snippet: snippet.to_owned(),
        }
    }

    /// D9: hits merge best first, a hit two collections hold is kept once (its best), and the
    /// merged list is cut to the limit.
    #[test]
    fn merged_hits_are_best_first_deduplicated_and_cut_to_the_limit() {
        let shared = hit("SHARED-1", "in two workspaces", 0.5);
        let lower_copy = Hit {
            score: 0.25,
            ..shared.clone()
        };
        let a = vec![hit("A-1", "a one", 0.9), shared.clone()];
        let b = vec![
            hit("B-1", "b one", 0.7),
            lower_copy,
            hit("B-2", "b two", 0.1),
        ];

        let merged = merge_hits(vec![a.clone(), b.clone()], 10);
        let keys: Vec<&str> = merged.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(keys, ["A-1", "B-1", "SHARED-1", "B-2"]);
        assert!(
            (merged[2].score - 0.5).abs() < f32::EPSILON,
            "the best copy is kept"
        );

        let cut = merge_hits(vec![a, b], 2);
        let keys: Vec<&str> = cut.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(keys, ["A-1", "B-1"]);
        assert!(merge_hits(Vec::new(), 10).is_empty());
    }

    /// A point of `project`'s item `key`, saying `words`.
    fn point(project: htui_core::model::ProjectId, key: &str, words: &str) -> vector::ConceptPoint {
        vector::ConceptPoint {
            subject: vector::Subject::Item {
                id: htui_core::model::ItemId::new(),
                kind_id: htui_core::model::ItemKindId::new(),
                status: htui_core::model::Status::Open,
                resolution: None,
            },
            key: key.to_owned(),
            project_id: project,
            updated_at: chrono::Utc::now(),
            document: None,
            text: format!("{key} {words}"),
        }
    }

    /// D9: each workspace's collection is searched only when that workspace is named; the global
    /// one always; `Everywhere` searches every collection and finds a project two workspaces
    /// share once.
    #[tokio::test]
    async fn a_search_reads_the_named_workspaces_collections_and_the_global_one() {
        use htui_core::model::ProjectId;
        const WORDS: &str = "lighthouse beacon";
        let (ws_a, ws_b) = (WorkspaceId::new(), WorkspaceId::new());
        let (pa, pb, shared) = (ProjectId::new(), ProjectId::new(), ProjectId::new());
        let index = MemIndex::new();
        let both = point(shared, "SHARED-1", WORDS);
        index
            .store(&IndexedCollection::Workspace(ws_a))
            .upsert(vec![point(pa, "A-1", WORDS), both.clone()])
            .await
            .expect("the fake upserts");
        index
            .store(&IndexedCollection::Workspace(ws_b))
            .upsert(vec![point(pb, "B-1", WORDS), both])
            .await
            .expect("the fake upserts");
        index
            .store(&IndexedCollection::Global)
            .upsert(vec![point(pa, "G-1", WORDS)])
            .await
            .expect("the fake upserts");
        let query = concepts::query(WORDS, vec![pa, pb, shared], false, 10);
        let keys = |hits: Result<Vec<Hit>, String>| {
            let mut keys: Vec<String> = hits
                .expect("the fake answers")
                .into_iter()
                .map(|h| h.key)
                .collect();
            keys.sort_unstable();
            keys
        };

        let only_a = index
            .search(SearchIn::Workspaces(vec![ws_a]), query.clone())
            .await;
        assert_eq!(keys(only_a), ["A-1", "G-1", "SHARED-1"]);
        let only_b = index
            .search(SearchIn::Workspaces(vec![ws_b]), query.clone())
            .await;
        assert_eq!(keys(only_b), ["B-1", "G-1", "SHARED-1"]);
        let neither = index
            .search(
                SearchIn::Workspaces(vec![WorkspaceId::new()]),
                query.clone(),
            )
            .await;
        assert_eq!(keys(neither), ["G-1"], "a missing collection adds nothing");
        let everywhere = index.search(SearchIn::Everywhere, query.clone()).await;
        assert_eq!(keys(everywhere), ["A-1", "B-1", "G-1", "SHARED-1"]);
        let named_both = index
            .search(SearchIn::Workspaces(vec![ws_a, ws_b]), query)
            .await;
        assert_eq!(keys(named_both), ["A-1", "B-1", "G-1", "SHARED-1"]);
    }

    /// D9: the overlay's search reads its workspace's collection: a point indexed into another
    /// workspace's is not found.
    #[tokio::test]
    async fn the_overlays_search_reads_its_own_workspaces_collection() {
        let (backend, scope) = platform().await;
        let elsewhere = IndexedCollection::Workspace(WorkspaceId::new());
        let index = Arc::new(MemIndex::new());
        index.seed(&elsewhere, &backend, &scope).await;
        let mut runtime = ConceptsRuntime::new(index.clone());
        let query = concepts::query(
            &feat_1_title(&backend).await,
            scope.project_ids.clone(),
            false,
            10,
        );
        let request = StoreRequest::SearchConcepts {
            query: query.clone(),
            scope_row_of: None,
            workspace: scope.workspace_id,
        };
        let replies = serve_one(&mut runtime, &backend, &envelope(1, request.clone())).await;
        assert_eq!(hits(&replies[0].reply).1, &Ok(Vec::new()));

        index
            .seed(
                &IndexedCollection::Workspace(scope.workspace_id),
                &backend,
                &scope,
            )
            .await;
        let replies = serve_one(&mut runtime, &backend, &envelope(2, request)).await;
        assert!(
            !hits(&replies[0].reply)
                .1
                .as_ref()
                .expect("no failure")
                .is_empty(),
            "its own collection answers"
        );
    }

    /// D9: a nil workspace (the startup one) is no search.
    #[tokio::test]
    async fn a_search_in_the_nil_workspace_answers_empty_without_the_index() {
        let (backend, scope) = platform().await;
        let mut runtime = ConceptsRuntime::new(Arc::new(MemIndex::new().failing("boom")));
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime.serve(
            &backend,
            &tx,
            &envelope(
                1,
                StoreRequest::SearchConcepts {
                    query: concepts::query("anything", scope.project_ids, false, 10),
                    scope_row_of: None,
                    workspace: WorkspaceId::from_uuid(Uuid::nil()),
                },
            ),
        );
        let ConceptsServed::Reply(reply) = served else {
            panic!("answered at once, got {served:?}")
        };
        assert_eq!(hits(&reply).1, &Ok(Vec::new()));
    }

    /// D12, MOD-159 M3 D4: another owner holds the workspace's lease live: the press asks it for
    /// a pass (its next claim finds the request), the reply names it, and nothing is synced.
    #[tokio::test]
    async fn a_ctrl_r_while_another_owner_holds_the_lease_asks_the_holder_and_syncs_nothing() {
        let (backend, scope) = platform().await;
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let other_box = BoxId::new();
        let other_owner = Uuid::now_v7();
        let taken = backend
            .take_indexer_lease(&collection, other_box, other_owner, TimeDelta::seconds(120))
            .await
            .expect("the memory store takes it");
        assert_eq!(taken, LeaseTake::Taken);
        let index = Arc::new(MemIndex::new());
        let mut runtime = ConceptsRuntime::new(index.clone());

        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                1,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        )
        .await;
        let Ok(IndexOutcome::Requested(lease)) = outcome(&replies[0].reply) else {
            panic!("the other owner was asked: {:?}", replies[0].reply)
        };
        assert_eq!(lease.box_id, Some(other_box));
        assert!(lease.live);
        assert!(
            backend
                .claim_indexer_pass(&collection, other_owner)
                .await
                .expect("the memory store claims it"),
            "the holder finds the request"
        );
        assert!(
            index.store(&collection).points().is_empty(),
            "nothing was synced"
        );
    }

    /// D12: a free lease is taken for the run, the run syncs, and the lease is let go after:
    /// no longer live, and another owner takes it.
    #[tokio::test]
    async fn a_ctrl_r_on_a_free_lease_syncs_then_lets_the_lease_go() {
        let (backend, scope) = platform().await;
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let index = Arc::new(MemIndex::new());
        let mut runtime = ConceptsRuntime::new(index.clone());

        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                1,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        )
        .await;
        let Ok(IndexOutcome::Synced(report)) = outcome(&replies[0].reply) else {
            panic!("synced: {:?}", replies[0].reply)
        };
        assert!(report.points_upserted > 0, "{report:?}");
        assert_eq!(
            report.points_upserted,
            index.store(&collection).points().len()
        );
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("the run took it");
        assert!(!lease.live, "let go after the run: {lease:?}");
        let other = backend
            .take_indexer_lease(
                &collection,
                BoxId::new(),
                Uuid::now_v7(),
                TimeDelta::seconds(120),
            )
            .await
            .expect("the memory store takes it");
        assert_eq!(other, LeaseTake::Taken);
    }

    /// D12: a lease this runtime already holds (its own index job's) is used and kept.
    #[tokio::test]
    async fn a_ctrl_r_under_this_runtimes_own_lease_syncs_and_keeps_it() {
        let (backend, scope) = platform().await;
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let index = Arc::new(MemIndex::new());
        let mut runtime = ConceptsRuntime::new(index.clone());
        let box_id = backend
            .box_info()
            .await
            .expect("the memory store reads it")
            .expect("the demo has a box")
            .box_id;
        backend
            .take_indexer_lease(
                &collection,
                box_id,
                runtime.owner(),
                TimeDelta::seconds(120),
            )
            .await
            .expect("the memory store takes it");

        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                1,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        )
        .await;
        assert!(
            matches!(outcome(&replies[0].reply), Ok(IndexOutcome::Synced(_))),
            "{:?}",
            replies[0].reply
        );
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("held");
        assert!(lease.live, "still held: {lease:?}");
        assert_eq!(lease.box_id, Some(box_id));
        // MOD-159 M1 R2: still the job's: its next renewal finds it.
        assert!(
            backend
                .renew_indexer_lease(&collection, runtime.owner(), TimeDelta::seconds(120))
                .await
                .expect("the memory store renews it"),
            "the job's lease is renewable by the runtime's token after the press"
        );
    }

    /// MOD-159 M1 R2: a press on a free lease takes it under a token of its own, never the
    /// runtime's: the runtime's index job, beating meanwhile, finds it held and leaves it, and the
    /// press's release afterwards cannot free anything of the job's.
    #[tokio::test(start_paused = true)]
    async fn a_ctrl_r_on_a_free_lease_holds_it_under_a_token_of_its_own() {
        let (backend, scope) = platform().await;
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let index = Arc::new(MemIndex::new().delayed(Duration::from_secs(10)));
        let mut runtime = ConceptsRuntime::new(index.clone());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let served = runtime.serve(
            &backend,
            &tx,
            &envelope(
                1,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        );
        assert!(matches!(served, ConceptsServed::Deferred), "{served:?}");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("the press took it");
        assert!(lease.live, "{lease:?}");
        assert!(
            !backend
                .renew_indexer_lease(&collection, runtime.owner(), TimeDelta::seconds(120))
                .await
                .expect("the memory store answers"),
            "the runtime's token does not hold the press's lease"
        );
        assert_eq!(runtime.settle(Duration::from_secs(60)).await, 0);
        let reply = rx.try_recv().expect("the press answered");
        assert!(
            matches!(outcome(&reply.reply), Ok(IndexOutcome::Synced(_))),
            "{:?}",
            reply.reply
        );
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("taken once");
        assert!(!lease.live, "the press let it go: {lease:?}");
    }

    /// MOD-159 M3 D7: a press the TUI's exit stopped half-way lets its lease go at
    /// `stop_index_job` (no index job runs here), so another owner takes it at once rather than
    /// after the TTL.
    #[tokio::test(start_paused = true)]
    async fn a_ctrl_r_quit_mid_sync_lets_its_lease_go() {
        let (backend, scope) = platform().await;
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let index = Arc::new(MemIndex::new().delayed(Duration::from_secs(3600)));
        let mut runtime = ConceptsRuntime::new(index);
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime.serve(
            &backend,
            &tx,
            &envelope(
                1,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        );
        assert!(matches!(served, ConceptsServed::Deferred), "{served:?}");
        tokio::time::sleep(Duration::from_secs(5)).await;
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("the press took it");
        assert!(lease.live, "held mid-sync: {lease:?}");
        assert!(
            backend
                .renew_indexer_lease(&collection, runtime.press_owner(), TimeDelta::seconds(120))
                .await
                .expect("the memory store answers"),
            "the press token holds it"
        );

        runtime.shutdown();
        runtime.stop_index_job(&backend).await;
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("taken once");
        assert!(!lease.live, "let go at exit: {lease:?}");
        let other = backend
            .take_indexer_lease(
                &collection,
                BoxId::new(),
                Uuid::now_v7(),
                TimeDelta::seconds(120),
            )
            .await
            .expect("the memory store takes it");
        assert_eq!(other, LeaseTake::Taken);
    }

    /// MOD-159 M3 R1: an aborted task that never ends costs the presses' release only its share
    /// of the wait: their lease is still let go at the exit, within `TUI_RELEASE_WAIT`.
    #[tokio::test(start_paused = true)]
    async fn a_stuck_aborted_task_still_leaves_the_release_its_share() {
        let (backend, scope) = platform().await;
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let mut runtime = ConceptsRuntime::new(Arc::new(MemIndex::new()));
        let taken = backend
            .take_indexer_lease(
                &collection,
                BoxId::new(),
                runtime.press_owner(),
                TimeDelta::seconds(120),
            )
            .await
            .expect("the memory store takes it");
        assert_eq!(taken, LeaseTake::Taken);
        runtime
            .aborted
            .push(tokio::spawn(std::future::pending::<()>()));

        let started = tokio::time::Instant::now();
        runtime.stop_index_job(&backend).await;
        assert!(
            started.elapsed() <= TUI_RELEASE_WAIT,
            "{:?}",
            started.elapsed()
        );
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("taken once");
        assert!(!lease.live, "let go despite the stuck task: {lease:?}");
    }

    /// MOD-159 M3 D7: a press superseded half-way leaves its lease under the runtime's one press
    /// token, so the next press takes it back as its own and syncs (never asking its own box for a
    /// pass), then lets it go.
    #[tokio::test(start_paused = true)]
    async fn a_ctrl_r_after_a_superseded_one_syncs_under_the_same_token() {
        let (backend, scope) = platform().await;
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let index = Arc::new(MemIndex::new().delayed(Duration::from_secs(10)));
        let mut runtime = ConceptsRuntime::new(index);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let press = |seq| {
            envelope(
                seq,
                StoreRequest::IndexConcepts {
                    scope: scope.clone(),
                    scope_row_of: None,
                },
            )
        };
        let first = runtime.serve(&backend, &tx, &press(1));
        assert!(matches!(first, ConceptsServed::Deferred), "{first:?}");
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(
            backend
                .indexer_lease(&collection)
                .await
                .expect("the memory store reads it")
                .is_some_and(|lease| lease.live),
            "the first press holds it"
        );
        let second = runtime.serve(&backend, &tx, &press(2));
        assert!(matches!(second, ConceptsServed::Deferred), "{second:?}");
        assert_eq!(runtime.settle(Duration::from_secs(60)).await, 0);

        let reply = rx.try_recv().expect("the second press answered");
        assert_eq!(reply.seq, 2, "the superseded press never answers");
        assert!(
            matches!(outcome(&reply.reply), Ok(IndexOutcome::Synced(_))),
            "{:?}",
            reply.reply
        );
        assert!(rx.try_recv().is_err(), "one answer");
        let lease = backend
            .indexer_lease(&collection)
            .await
            .expect("the memory store reads it")
            .expect("taken");
        assert!(!lease.live, "the second press let it go: {lease:?}");
    }

    /// MOD-159 M1 R2: one failing collection (stale, or another embedder's) is skipped and the
    /// others answer, for MCP's `Everywhere` and the TUI's named workspaces alike. A failure is
    /// the answer only when no workspace collection answered: the global one, empty on most
    /// boxes, never stands in for one.
    #[tokio::test]
    async fn a_failing_collection_is_skipped_while_the_others_answer() {
        use htui_core::model::ProjectId;
        const WORDS: &str = "lighthouse beacon";
        let (ws_a, ws_b) = (WorkspaceId::new(), WorkspaceId::new());
        let (pa, pb) = (ProjectId::new(), ProjectId::new());
        let seed = |index: MemIndex| async move {
            for (collection, project, key) in [
                (IndexedCollection::Workspace(ws_a), pa, "A-1"),
                (IndexedCollection::Workspace(ws_b), pb, "B-1"),
                (IndexedCollection::Global, pa, "G-1"),
            ] {
                index
                    .store(&collection)
                    .upsert(vec![point(project, key, WORDS)])
                    .await
                    .expect("the fake upserts");
            }
            index
        };
        let query = concepts::query(WORDS, vec![pa, pb], false, 10);
        let keys = |hits: Result<Vec<Hit>, String>| {
            let mut keys: Vec<String> = hits
                .expect("the others answer")
                .into_iter()
                .map(|h| h.key)
                .collect();
            keys.sort_unstable();
            keys
        };
        let a_fails = seed(MemIndex::new().failing_in(&IndexedCollection::Workspace(ws_a))).await;
        assert_eq!(
            keys(a_fails.search(SearchIn::Everywhere, query.clone()).await),
            ["B-1", "G-1"]
        );
        assert_eq!(
            keys(
                a_fails
                    .search(SearchIn::Workspaces(vec![ws_a, ws_b]), query.clone())
                    .await
            ),
            ["B-1", "G-1"]
        );
        let alone = a_fails
            .search(SearchIn::Workspaces(vec![ws_a]), query.clone())
            .await
            .expect_err("its only workspace failed; the global one does not hide it");
        assert!(alone.contains("planted failure"), "{alone}");

        let global_fails = seed(MemIndex::new().failing_in(&IndexedCollection::Global)).await;
        assert_eq!(
            keys(
                global_fails
                    .search(SearchIn::Everywhere, query.clone())
                    .await
            ),
            ["A-1", "B-1"]
        );
        let every = seed(
            MemIndex::new()
                .failing_in(&IndexedCollection::Workspace(ws_a))
                .failing_in(&IndexedCollection::Workspace(ws_b)),
        )
        .await;
        assert!(
            every.search(SearchIn::Everywhere, query).await.is_err(),
            "every workspace collection failed"
        );
    }

    /// MOD-159 M1 R2 (plan D8): the TUI's index job starts when a writable store is published,
    /// takes leases under the runtime's token, and `stop_index_job` lets go of them. Its model
    /// never loads, so the job holds its first due collection while it waits.
    #[tokio::test]
    async fn the_runtimes_index_job_runs_under_its_token_and_lets_go_at_the_stop() {
        use crate::concepts::IndexSource as _;
        let Some(db) = htui_store::testkit::fresh_db().await else {
            return;
        };
        let index = Arc::new(
            QdrantIndex::with_loader(|| Box::pin(std::future::pending()))
                .with_source(Arc::new(source(Some("http://127.0.0.1:1"), None, no_key))),
        );
        let mut runtime =
            ConceptsRuntime::new(Arc::new(Recording::default())).with_index_job_over(index);
        let first = db
            .store
            .index_scopes()
            .await
            .expect("the scopes")
            .first()
            .map_or(IndexedCollection::Global, |scope| {
                IndexedCollection::Workspace(scope.workspace_id)
            });
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "concepts-index-job", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        runtime.publish(&backend);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !db
            .store
            .indexer_lease(&first)
            .await
            .expect("the lease reads")
            .is_some_and(|lease| lease.live)
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the job never took {first}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            db.store
                .renew_indexer_lease(&first, runtime.owner(), TimeDelta::seconds(120))
                .await
                .expect("the lease renews"),
            "taken under the runtime's token"
        );
        runtime.stop_index_job(&backend).await;
        let lease = db
            .store
            .indexer_lease(&first)
            .await
            .expect("the lease reads")
            .expect("taken once");
        assert!(!lease.live, "let go at the stop: {lease:?}");
        cache.close().await;
        db.drop_db().await;
    }

    /// D12: a backend with no box of its own cannot take a lease: the run says so.
    #[tokio::test]
    async fn a_ctrl_r_on_a_box_without_identity_is_an_error() {
        let backend = Backend::memory(MemStore::new());
        let mut runtime = ConceptsRuntime::new(Arc::new(MemIndex::new()));
        let scope = Scope {
            workspace_id: WorkspaceId::new(),
            project_ids: vec![htui_core::model::ProjectId::new()],
        };
        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(
                1,
                StoreRequest::IndexConcepts {
                    scope,
                    scope_row_of: None,
                },
            ),
        )
        .await;
        assert_eq!(indexed(&replies[0].reply), Err(NO_BOX_IDENTITY.to_owned()));
    }
}
