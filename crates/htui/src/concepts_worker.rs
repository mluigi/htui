//! The concepts index behind the TUI's search (MOD-64 D230-D232, D237, D238).
//!
//! `SearchConcepts` loads an embedding model and calls Qdrant; `IndexConcepts` reads every item of
//! the scope. Neither may run in the store worker's serial loop (`R-NF-3`): [`ConceptsRuntime`]
//! serves both on tasks of their own, the way `AgentRuntime::preview` serves a preview, and each
//! task sends its own reply. The index is an object-safe [`ConceptIndex`]: [`QdrantIndex`] in
//! production, `MemIndex` in tests.

use std::collections::HashMap;
use std::mem::Discriminant;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock, Weak};
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use htui_core::model::Scope;
use htui_core::qdrant::{QdrantUrl, QdrantUrlError, StoredQdrantUrl};
use htui_core::store::{MemStore, StoreError};
use htui_store::embed::RtenEmbedder;
use htui_store::model;
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::secret::{KeyHost, QdrantEnv, QdrantKeyError};
use htui_store::vector::{Hit, QdrantStore, SearchQuery, VectorStore as _};
use htui_store::vector_sync::{Indexer, SyncReport};
use htui_store::{Backend, PgStore, secret};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest};

/// What both requests answer when nothing serves them: the test harness without a runtime (D231).
pub const NOT_AVAILABLE: &str = "concepts search is not available";

/// The two `StoreRequest::name`s this module serves, in that order (D241).
pub const REQUEST_NAMES: [&str; 2] = ["search_concepts", "index_concepts"];

/// One concepts answer, its error inside (D232, blueprint D242): a Qdrant failure is the overlay's
/// to show and never becomes `StoreReply::Failed`, which the shell also puts on the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum ConceptsReply {
    /// Answer to `SearchConcepts`. `query` is the request's own, so a list is matched to the
    /// search it answers.
    Hits {
        /// The search this answers.
        query: SearchQuery,
        /// The hits, best first, or why there are none.
        outcome: Result<Vec<Hit>, String>,
    },
    /// Answer to `IndexConcepts`: what the run did, or why it stopped.
    Indexed(Result<SyncReport, String>),
}

/// The index seam (D230): object-safe, where `VectorStore` (`async fn`s) is not. Errors are display
/// strings: they are only ever shown.
pub trait ConceptIndex: Send + Sync {
    /// Hybrid search, scoped to `query.projects`.
    fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>>;
    /// `Indexer::sync` of every project of `scope`, reading through `backend`.
    fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>>;
    /// MOD-158 D6: the store the `qdrant_url` row is read through from now on (`None`: offline,
    /// `--demo`, or not connected yet). An index that reads no settings ignores it.
    fn publish(&self, _store: Option<Arc<dyn UrlRow>>) {}
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
    /// No override and no row: [`crate::concepts::NO_QDRANT_URL`].
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

/// MOD-158 D6: one process's source of Qdrant settings. The slot holds the current store (`None`
/// offline, under `--demo`, or before a connect); every read takes an `Arc` clone and drops the
/// lock before any `.await`, and a publish is a pointer swap with no `.await` (`R-NF-3`).
pub struct QdrantSource {
    /// `HTUI_QDRANT_URL` and `HTUI_QDRANT_API_KEY`, as this process read them.
    env: QdrantEnv,
    /// The store the `qdrant_url` row is read through.
    store: RwLock<Option<Arc<dyn UrlRow>>>,
    /// Where the key comes from after the environment.
    key: RwLock<KeySource>,
    /// The keyring read of the key.
    keyring: KeyringRead,
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
        Self {
            env,
            store: RwLock::new(None),
            key: RwLock::new(key),
            keyring: secret::get_qdrant_api_key,
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

    /// The store the row is read through from now on.
    pub fn publish(&self, store: Option<Arc<dyn UrlRow>>) {
        *self.store.write().unwrap_or_else(PoisonError::into_inner) = store;
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
        let store = self.current();
        resolve_url(&self.env, store.as_deref()).await
    }

    /// D3 and D4, read now: a URL or key changed since the last call applies to this one.
    ///
    /// # Errors
    /// [`Unresolved`].
    pub async fn settings(&self) -> Result<QdrantSettings, Unresolved> {
        let store = self.current();
        let key = self
            .key
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        resolve_settings(&self.env, store.as_deref(), &key, self.keyring).await
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
    /// The last connection made, for the settings it was made with (review 3).
    connections: Connections<QdrantStore<RtenEmbedder>>,
    /// Where the settings come from (MOD-158 D6).
    source: Arc<QdrantSource>,
}

/// One cached connection, keyed by the URL and key it was made with (review 3).
///
/// `QdrantStore::connect` is a round trip and five waited payload-index writes, too much to pay per
/// search. The lock is never held across an `await`: two requests that miss together both connect,
/// and the later `put` wins, which costs one spare connection and nothing else.
struct Connections<T> {
    /// The settings and the connection made with them.
    slot: Mutex<Option<(QdrantSettings, Arc<T>)>>,
}

impl<T> Default for Connections<T> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }
}

impl<T> Connections<T> {
    /// The cached connection, when it was made with `settings`' URL and key.
    fn get(&self, settings: &QdrantSettings) -> Option<Arc<T>> {
        let slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.as_ref()
            .filter(|(made_for, _)| {
                made_for.url == settings.url
                    && made_for.api_key.as_deref() == settings.api_key.as_deref()
            })
            .map(|(_, connection)| Arc::clone(connection))
    }

    /// Caches `connection` as the one for `settings`.
    fn put(&self, settings: QdrantSettings, connection: Arc<T>) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some((settings, connection));
    }

    /// The settings stopped resolving (no URL, offline, an unusable setting): no connection, and
    /// no key it was made with, is kept for them (MOD-158 R1, B-L1).
    fn clear(&self) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
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

    /// A call on `connection` failed: the next request reconnects. A connection another request
    /// has cached since is left alone.
    fn forget(&self, connection: &Arc<T>) {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if slot
            .as_ref()
            .is_some_and(|(_, cached)| Arc::ptr_eq(cached, connection))
        {
            *slot = None;
        }
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

    /// The model, loading it on first use (F9, blueprint D245). `concepts::spawn_index_job` takes
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

    /// Qdrant at `settings`, over the shared model: the cached connection when it was made with
    /// the same URL and key, else a new one, cached (review 3). The worker's index job opens its
    /// store here, so a changed URL or key reconnects it too (MOD-158 D7).
    pub(crate) async fn connect(
        &self,
        settings: QdrantSettings,
    ) -> Result<Arc<QdrantStore<RtenEmbedder>>, String> {
        if let Some(store) = self.connections.get(&settings) {
            return Ok(store);
        }
        let embedder = self.embedder().await?;
        let store = QdrantStore::connect(&settings, embedder)
            .await
            .map(Arc::new)
            .map_err(|e| format!("{:#}", crate::concepts::qdrant_refusal(e)))?;
        self.connections.put(settings, Arc::clone(&store));
        Ok(store)
    }

    /// The worker's index job's `forget` (MOD-158 R1, B-L1): `Some(store)` after a failed sync on
    /// it, `None` when the settings did not resolve.
    pub(crate) fn forget(&self, store: Option<&Arc<QdrantStore<RtenEmbedder>>>) {
        match store {
            Some(store) => self.connections.forget(store),
            None => self.connections.clear(),
        }
    }

    /// The settings, read now; a cached connection is dropped when they do not resolve.
    async fn settings(&self) -> Result<QdrantSettings, String> {
        self.connections
            .resolved(self.source.settings().await)
            .map_err(|why| why.sentence())
    }
}

// Settings before the model, in both: a box with no URL fails in microseconds without downloading
// anything. Any failure drops the connection it used, so the next request reconnects: telling a
// dropped connection from a refused query would mean parsing the client's errors, and a spare
// reconnect after a refusal costs what every request cost before the cache (review 3).
impl ConceptIndex for QdrantIndex {
    fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
        Box::pin(async move {
            let settings = self.settings().await?;
            let store = self.connect(settings).await?;
            store.search(&query).await.map_err(|e| {
                self.connections.forget(&store);
                e.to_string()
            })
        })
    }

    fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>> {
        Box::pin(async move {
            let settings = self.settings().await?;
            let store = self.connect(settings).await?;
            Indexer::sync(&backend, &scope, &*store).await.map_err(|e| {
                self.connections.forget(&store);
                e.to_string()
            })
        })
    }

    fn publish(&self, store: Option<Arc<dyn UrlRow>>) {
        self.source.publish(store);
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

/// [`shared_index`] as itself: `concepts::spawn_index_job` takes the worker's model from it, so
/// `htui worker`'s index job and its `search_concepts` load the model once (H-26).
#[must_use]
pub fn shared_qdrant() -> Arc<QdrantIndex> {
    static SHARED: OnceLock<Arc<QdrantIndex>> = OnceLock::new();
    Arc::clone(SHARED.get_or_init(|| Arc::new(QdrantIndex::new())))
}

/// A [`ConceptIndex`] over `MemVectorStore` (D230): ranks by shared terms, needs no model. It can be
/// told to fail every call, and to wait before answering, for the error and supersede cases.
#[cfg(any(test, feature = "testkit"))]
#[derive(Debug, Default)]
pub struct MemIndex {
    /// The fake the calls go to.
    store: htui_store::vector::MemVectorStore,
    /// What every call answers instead, when set.
    failure: Option<String>,
    /// How long every call sleeps first, when set.
    delay: Option<Duration>,
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

    /// Every call sleeps `by` first (`tokio::time::sleep`, so a paused clock advances it).
    #[must_use]
    pub fn delayed(self, by: Duration) -> Self {
        Self {
            delay: Some(by),
            ..self
        }
    }

    /// The fake store, for assertions.
    #[must_use]
    pub const fn store(&self) -> &htui_store::vector::MemVectorStore {
        &self.store
    }

    /// Indexes `scope` through `backend` directly, as a test's setup.
    ///
    /// # Panics
    /// If the sync fails, which `MemVectorStore` never does over a memory backend.
    pub async fn seed(&self, backend: &Backend, scope: &Scope) -> SyncReport {
        Indexer::sync(backend, scope, &self.store)
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
    fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
        Box::pin(async move {
            self.pause().await?;
            self.store.search(&query).await.map_err(|e| e.to_string())
        })
    }

    fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>> {
        Box::pin(async move {
            self.pause().await?;
            Indexer::sync(&backend, &scope, &self.store)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// What [`ConceptsRuntime::serve`] decided (the `RunServed` shape, blueprint D243).
#[derive(Debug)]
pub enum ConceptsServed {
    /// Answer with this reply, now.
    Reply(StoreReply),
    /// A task of the runtime answers the request, exactly once.
    Deferred,
}

/// Serves `SearchConcepts` and `IndexConcepts` on tasks of their own, inside the store worker's
/// loop beside `AgentRuntime` and `RunRuntime` (D231). One task per `(origin, request kind)`: a
/// newer request of the same kind from the same view aborts the older one, whose answer the shell
/// would drop anyway. A search never aborts an index run: they are different kinds.
pub struct ConceptsRuntime {
    /// The index every task searches and writes.
    index: Arc<dyn ConceptIndex>,
    /// The task of each `(origin, request kind)`, keyed as `App::latest` is.
    tasks: HashMap<(Origin, Discriminant<StoreRequest>), JoinHandle<()>>,
    /// What [`Self::publish`] last handed the index (MOD-158 R1, B-L2).
    published: Published,
}

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
        }
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
            StoreRequest::SearchConcepts(query) => {
                // D233: no projects is no search; the index would say `[]` too.
                if query.projects.is_empty() || query.limit == 0 {
                    return ConceptsServed::Reply(StoreReply::Concepts(Box::new(
                        ConceptsReply::Hits {
                            query: query.clone(),
                            outcome: Ok(Vec::new()),
                        },
                    )));
                }
                let query = query.clone();
                tokio::spawn(async move {
                    let outcome = index.search(query.clone()).await;
                    let reply = ConceptsReply::Hits { query, outcome };
                    let _ = replies.send(ReplyEnvelope {
                        seq,
                        origin,
                        reply: StoreReply::Concepts(Box::new(reply)),
                    });
                })
            }
            StoreRequest::IndexConcepts { scope } => {
                if scope.project_ids.is_empty() {
                    return ConceptsServed::Reply(StoreReply::Concepts(Box::new(
                        ConceptsReply::Indexed(Ok(SyncReport::default())),
                    )));
                }
                let (backend, scope) = (backend.clone(), scope.clone());
                tokio::spawn(async move {
                    let outcome = index.sync(backend, scope).await;
                    let _ = replies.send(ReplyEnvelope {
                        seq,
                        origin,
                        reply: StoreReply::Concepts(Box::new(ConceptsReply::Indexed(outcome))),
                    });
                })
            }
            // The loop routes only the two above here; the two runtimes' rule for anything else.
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
    /// rebuilds what it did not reach.
    pub fn shutdown(&mut self) {
        for task in std::mem::take(&mut self.tasks).into_values() {
            task.abort();
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
    use htui_core::store::{MemStore, ReadStore as _};
    use htui_store::vector::PointType;
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
            ConceptsReply::Hits { query, outcome } => (query, outcome),
            other @ ConceptsReply::Indexed(_) => panic!("expected hits, got {other:?}"),
        }
    }

    fn indexed(reply: &StoreReply) -> &Result<SyncReport, String> {
        match concepts_reply(reply) {
            ConceptsReply::Indexed(outcome) => outcome,
            other @ ConceptsReply::Hits { .. } => panic!("expected an index report, got {other:?}"),
        }
    }

    async fn seeded(backend: &Backend, scope: &Scope, index: MemIndex) -> Arc<MemIndex> {
        index.seed(backend, scope).await;
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
            &envelope(7, StoreRequest::SearchConcepts(query.clone())),
        );
        assert!(matches!(served, ConceptsServed::Deferred), "{served:?}");
        assert_eq!(runtime.settle(Duration::from_secs(10)).await, 0);

        let reply = rx.try_recv().expect("the task answered");
        assert!(rx.try_recv().is_err(), "exactly once");
        assert_eq!((reply.seq, &reply.origin), (7, &PROBE));
        let (echoed, outcome) = hits(&reply.reply);
        assert_eq!(echoed, &query);
        let expected = index.store().search(&query).await.expect("the fake");
        assert!(!expected.is_empty(), "FEAT-1's own title finds it");
        assert_eq!(outcome.as_ref().expect("no failure"), &expected);
    }

    #[tokio::test]
    async fn decisions_narrow_the_hits_to_closed_items() {
        let (backend, scope) = platform().await;
        let index = seeded(&backend, &scope, MemIndex::new()).await;
        let mut runtime = ConceptsRuntime::new(index.clone());
        let decided = index
            .store()
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
            &envelope(1, StoreRequest::SearchConcepts(all)),
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
            &envelope(2, StoreRequest::SearchConcepts(decisions)),
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
            &envelope(1, StoreRequest::SearchConcepts(query.clone())),
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
                &envelope(seq, StoreRequest::SearchConcepts(query.clone())),
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
                },
            ),
        );
        let second = runtime.serve(
            &backend,
            &tx,
            &envelope(2, StoreRequest::SearchConcepts(query)),
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
        };

        let replies = serve_one(&mut runtime, &backend, &envelope(1, request.clone())).await;
        assert_eq!(replies.len(), 1);
        let report = *indexed(&replies[0].reply).as_ref().expect("no failure");
        assert!(report.items_rebuilt > 0, "{report:?}");
        assert_eq!(report.points_upserted, index.store().points().len());

        let replies = serve_one(&mut runtime, &backend, &envelope(2, request)).await;
        let again = *indexed(&replies[0].reply).as_ref().expect("no failure");
        assert_eq!(again.items_rebuilt, 0, "nothing changed since: {again:?}");
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
            &envelope(1, StoreRequest::SearchConcepts(query)),
        )
        .await;
        assert_eq!(hits(&replies[0].reply).1, &Err(message.to_owned()));

        let replies = serve_one(
            &mut runtime,
            &backend,
            &envelope(2, StoreRequest::IndexConcepts { scope }),
        )
        .await;
        assert_eq!(indexed(&replies[0].reply), &Err(message.to_owned()));
        assert!(
            !matches!(replies[0].reply, StoreReply::Failed { .. }),
            "never the status line's variant"
        );
    }

    #[tokio::test]
    async fn without_a_runtime_both_requests_answer_not_available() {
        let (backend, scope) = platform().await;
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        let reply =
            store_worker::serve(&backend, &StoreRequest::SearchConcepts(query.clone())).await;
        let (echoed, outcome) = hits(&reply);
        assert_eq!(echoed, &query);
        assert_eq!(outcome, &Err(NOT_AVAILABLE.to_owned()));

        let reply = store_worker::serve(&backend, &StoreRequest::IndexConcepts { scope }).await;
        assert_eq!(indexed(&reply), &Err(NOT_AVAILABLE.to_owned()));
    }

    #[tokio::test]
    async fn request_names_match_the_name_arms() {
        let (_, scope) = platform().await;
        let search = StoreRequest::SearchConcepts(concepts::query("x", Vec::new(), false, 1));
        assert_eq!(REQUEST_NAMES[0], search.name());
        assert_eq!(
            REQUEST_NAMES[1],
            StoreRequest::IndexConcepts { scope }.name()
        );
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

    /// An index that records what is published to it.
    #[derive(Default)]
    struct Recording(Mutex<Vec<bool>>);

    impl ConceptIndex for Recording {
        fn search(&self, _: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn sync(&self, _: Backend, _: Scope) -> BoxFuture<'_, Result<SyncReport, String>> {
            Box::pin(async { Ok(SyncReport::default()) })
        }
        fn publish(&self, store: Option<Arc<dyn UrlRow>>) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(store.is_some());
        }
    }

    /// MOD-158 D6: only a writable Postgres is published; memory and the mirror are `None`.
    #[tokio::test]
    async fn the_runtime_publishes_the_writable_store_only() {
        let index = Arc::new(Recording::default());
        let mut runtime = ConceptsRuntime::new(index.clone());
        runtime.publish(&Backend::memory(MemStore::demo()));

        let identity = htui_store::Identity {
            box_id: htui_core::model::BoxId::new(),
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
            box_id: htui_core::model::BoxId::new(),
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

        let searched = index.search(query).await.expect_err("no URL is stored");
        for place in ["Settings > Qdrant", "htui --set-qdrant-url"] {
            assert!(searched.contains(place), "{searched}");
        }
        let synced = index
            .sync(backend, scope)
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
            index.search(query.clone()).await.expect_err("offline"),
            URL_OFFLINE
        );
        ConceptIndex::publish(&index, Some(Arc::new(MemStore::new())));
        assert_eq!(
            index.search(query).await.expect_err("no row"),
            concepts::NO_QDRANT_URL
        );
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
            connections.get(&made_for).is_none(),
            "nothing connected yet"
        );

        let first = Arc::new(1);
        connections.put(made_for.clone(), Arc::clone(&first));
        let reused = connections
            .get(&made_for)
            .expect("the same settings reuse it");
        assert!(Arc::ptr_eq(&reused, &first));
        for moved in [
            settings("http://qdrant:6334", Some("other")),
            settings("http://qdrant:6334", None),
            settings("http://elsewhere:6334", Some("key")),
        ] {
            assert!(connections.get(&moved).is_none(), "{moved:?}");
        }

        // An error on a connection another request has already replaced leaves the new one.
        let second = Arc::new(2);
        connections.put(made_for.clone(), Arc::clone(&second));
        connections.forget(&first);
        let kept = connections.get(&made_for).expect("the replacement stays");
        assert!(Arc::ptr_eq(&kept, &second));

        connections.forget(&second);
        assert!(
            connections.get(&made_for).is_none(),
            "after an error the next request reconnects"
        );
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
        connections.put(made_for.clone(), Arc::new(1));
        assert!(connections.resolved(Ok(made_for.clone())).is_ok());
        assert!(connections.get(&made_for).is_some(), "resolving keeps it");
        for why in [Unresolved::NoUrl, Unresolved::Offline] {
            connections.put(made_for.clone(), Arc::new(2));
            assert!(connections.resolved(Err(why)).is_err());
            assert!(
                connections
                    .slot
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_none(),
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
            &envelope(1, StoreRequest::SearchConcepts(query)),
        );
        runtime.serve(
            &backend,
            &tx,
            &envelope(2, StoreRequest::IndexConcepts { scope }),
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
}
