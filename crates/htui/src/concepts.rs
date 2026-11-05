//! `--index-items` and `--search-items`: the concepts index from the command line (MOD-34 D3,
//! `R-STO-8`).
//!
//! Both run before any terminal work, like `--set-dsn`, print to the shell and exit. Neither
//! touches the TUI's start path: a missing Qdrant URL, a Qdrant that does not answer or a
//! Postgres that does not answer is one line on stderr and a non-zero exit, and nothing else is
//! affected (`docs/ANA-19.md` §2 invariant 3). The automatic sync belongs to the headless worker
//! (MOD-41) and the agent-facing tool to the MCP server (MOD-11).
//!
//! `--index-items` connects headless (`PgStore::connect_headless`, MOD-40 plan D8): a pending
//! schema or a build below the database's target version is refused, never migrated.
//! `--search-items` reads only (MOD-92): `PgStore::connect_read_only` and
//! `vector::ExistingCollection` (the collection before the model), with the same refusals, and
//! answers through `state_cmd` with exit 3 when Postgres or Qdrant does not answer, 2 on a refusal
//! and 1 on a failure, as lines or (`--json`) one document.
//!
//! The worker's background sync (MOD-41 plan D19, MOD-158 D7) lives here too: [`spawn_index_job`]
//! spawns [`index_job`] at `htui worker` start, which resolves the Qdrant settings every cycle and
//! re-syncs every project at start and then every [`sync_interval`].
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use htui_core::model::{ProjectId, RequirementState, Resolution, Scope};
use htui_core::store::{MemStore, ReadStore, StoreError};
use htui_store::embed::DenseEmbedder;
use htui_store::embed::RtenEmbedder;
use htui_store::model;
use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::secret::QdrantEnv;
use htui_store::vector::{
    COLLECTION, EMBEDDER_MISMATCH, ExistingCollection, Hit, PointType, QdrantStore, SearchQuery,
    VectorStore, is_embedder_mismatch, is_qdrant_refused, is_query_embed_failure,
};
use htui_store::vector_sync::{Indexer, SyncReport};
use htui_store::{HeadlessError, Identity, PgStore, identity, secret};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::concepts_worker::{self, KeySource, QdrantIndex};
use crate::state_cmd::{
    self, Answer, ErrorCode, Failure, IndexAnswer, Kind, Outcome, SearchAnswer, StateExit,
};

/// Options of `--search-items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// The query text.
    pub query: String,
    /// Restrict to one project, by slug.
    pub project: Option<String>,
    /// Decisions only (ANA-11 §4.2): items closed as [`DECISION_RESOLUTIONS`], and their documents.
    pub decisions: bool,
    /// Most hits printed.
    pub limit: u64,
    /// `--cwd`: only the project whose repository, registered on this box, holds this path (MOD-92).
    pub cwd: Option<PathBuf>,
    /// `--json`: one versioned document instead of lines (MOD-92).
    pub json: bool,
}

/// Hits `--search-items` prints when `--limit` is not given.
pub const DEFAULT_LIMIT: u64 = 10;

/// The resolutions `--decisions` keeps (MOD-50 D223, maintainer): an item closed as withdrawn
/// ("dropped without a decision"), duplicate or superseded records no decision in force, and a
/// done item that was never closed out has none yet. Requirements carry no resolution, so they are
/// left out too.
pub const DECISION_RESOLUTIONS: [Resolution; 3] = [
    Resolution::Done,
    Resolution::Concluded,
    Resolution::Rejected,
];

/// The search both front ends send (MOD-64 D239): `text` in `projects`, narrowed to decisions (items
/// closed as one of [`DECISION_RESOLUTIONS`], and their documents) when `decisions` is set, at most
/// `limit` hits. No type or status filter: `--search-items` has none either.
#[must_use]
pub fn query(text: &str, projects: Vec<ProjectId>, decisions: bool, limit: u64) -> SearchQuery {
    SearchQuery {
        text: text.to_owned(),
        projects,
        types: Vec::new(),
        statuses: Vec::new(),
        resolutions: if decisions {
            DECISION_RESOLUTIONS.to_vec()
        } else {
            Vec::new()
        },
        limit,
    }
}

/// The sentence a search or an index run refuses with when no Qdrant URL is set (MOD-158 D3: no
/// `HTUI_QDRANT_URL` override and no `qdrant_url` row).
pub(crate) const NO_QDRANT_URL: &str =
    "no Qdrant URL is set; set one in Settings > Qdrant or with `htui --set-qdrant-url`";

/// The sentence every verb that needs the keyring's DSN refuses with when none is stored.
pub(crate) const NO_DSN: &str = "no Postgres DSN is stored; run `htui --set-dsn` first";

/// `--index-items`' connect: headless, and the collection created on first use, because it writes
/// the index by design. `--search-items` does not come here (MOD-92: it reads only). MOD-158 D8's
/// order: the DSN, the connect, the Qdrant URL (the override or the row) and key, the model, Qdrant.
async fn open() -> anyhow::Result<(PgStore, QdrantStore<RtenEmbedder>)> {
    let Some(dsn) = secret::get_dsn()? else {
        bail!("{NO_DSN}");
    };
    let identity = identity::load_or_mint(&identity::config_root()?)?;
    let pg = PgStore::connect_headless(&dsn, &identity, CONNECT_TIMEOUT, PoolSize::TUI)
        .await
        .map_err(headless_refusal)?;
    let settings = concepts_worker::resolve_settings(
        &QdrantEnv::from_process(),
        Some(&pg),
        &KeySource::Interactive,
        secret::get_qdrant_api_key,
    )
    .await
    .map_err(|why| anyhow::anyhow!("{why}"))?;
    let embedder = load_embedder().await?;
    let store = QdrantStore::connect(&settings, embedder)
        .await
        .map_err(qdrant_refusal)?;
    Ok((pg, store))
}

/// The embedding model, downloaded first if this box has none. Said on stderr only when a download
/// happens: these commands' log goes nowhere without `--log`, and 133 MB is otherwise a silent
/// pause (review L4); stdout stays the command's.
async fn load_embedder() -> anyhow::Result<RtenEmbedder> {
    let files = model::ensure_model_noting(|line| eprintln!("{line}"))
        .await
        .context("cannot load the embedding model")?;
    tokio::task::spawn_blocking(move || RtenEmbedder::load(&files))
        .await
        .context("the embedding model's loader stopped")?
        .context("cannot load the embedding model")
}

/// A failed `QdrantStore::connect` as the error a user reads, at all three sites that connect
/// (review L3): "cannot reach Qdrant" above everything but an embedder mismatch and a refused API
/// key (review R1 M2), which Qdrant answered and which name their own remedy.
pub(crate) fn qdrant_refusal(err: StoreError) -> anyhow::Error {
    if is_embedder_mismatch(&err) || is_qdrant_refused(&err) {
        anyhow::Error::new(err)
    } else {
        anyhow::Error::new(err).context("cannot reach Qdrant")
    }
}

/// A headless connect's refusal as the line a user reads (MOD-40 plan D8's sentences, byte for
/// byte). `--index-items` exits 1 with it, `htui worker` exits 2 (MOD-41 plan D14).
pub fn headless_refusal(err: HeadlessError) -> anyhow::Error {
    match err {
        HeadlessError::MigrationsPending(n) => {
            anyhow::anyhow!("{n} schema migration(s) are pending; start `htui` once to apply them")
        }
        HeadlessError::Store(err) => {
            let context = store_context(&err);
            anyhow::Error::new(err).context(context)
        }
        below @ HeadlessError::BelowTarget { .. } => below.into(),
    }
}

/// The line above a headless connect's store error: only an unreachable server is "cannot
/// reach"; a newer or drifted schema, a partial version or a malformed target was reached and
/// refused this build (MOD-40 plan D8).
fn store_context(err: &StoreError) -> &'static str {
    match err {
        StoreError::Unreachable(_) => "cannot reach Postgres",
        _ => "Postgres refused this htui",
    }
}

/// Every workspace's projects as scopes, narrowed to one project slug when given.
pub(crate) async fn scopes(pg: &PgStore, project: Option<&str>) -> anyhow::Result<Vec<Scope>> {
    let mut scopes = Vec::new();
    let mut found = project.is_none();
    for ws in pg.workspaces().await? {
        let mut scope = Scope::from_workspace(&ws);
        if let Some(slug) = project {
            let wanted: Vec<ProjectId> = ws
                .projects
                .iter()
                .filter(|p| p.slug == slug)
                .map(|p| p.project_id)
                .collect();
            scope.project_ids.retain(|id| wanted.contains(id));
        }
        found |= !scope.project_ids.is_empty();
        if !scope.project_ids.is_empty() {
            scopes.push(scope);
        }
    }
    if !found {
        bail!("no project with slug `{}`", project.unwrap_or_default());
    }
    Ok(scopes)
}

/// `--index-items`: brings the index in step with every project (or one), then reports.
///
/// # Errors
///
/// Missing settings, an unreachable Postgres or Qdrant, a model that cannot load.
pub async fn index_items(project: Option<&str>) -> anyhow::Result<()> {
    let (pg, store) = open().await?;
    let report = sync_all(&pg, &scopes(&pg, project).await?, &store).await?;
    eprintln!("{}", report_line(&report));
    Ok(())
}

/// The `app_setting` key of the worker's index interval (MOD-41 plan D19): a positive JSON
/// integer of minutes.
pub const SYNC_MINUTES_KEY: &str = "concepts_sync_minutes";

/// The interval, in minutes, when [`SYNC_MINUTES_KEY`] is absent, non-positive or not an integer.
pub const DEFAULT_SYNC_MINUTES: u64 = 15;

/// `index_items`' loop over given scopes (MOD-41 plan D19, fact-check F-C4): every scope in
/// order, the reports summed. Taking the scopes rather than a `PgStore` lets the worker job's
/// tests run over `MemStore`.
///
/// # Errors
///
/// The first scope's store or index error; later scopes are not synced.
pub async fn sync_all(
    read: &impl ReadStore,
    scopes: &[Scope],
    vectors: &impl VectorStore,
) -> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();
    for scope in scopes {
        report += Indexer::sync(read, scope, vectors).await?;
    }
    Ok(report)
}

/// The worker's sleep between two syncs, from [`SYNC_MINUTES_KEY`] in `app`, re-read each cycle
/// (the rule of `htui-orch`'s lease times): absent, `null`, a string, a bool, a fraction, zero
/// and a negative all mean [`DEFAULT_SYNC_MINUTES`].
#[must_use]
pub fn sync_interval(app: &BTreeMap<String, Value>) -> Duration {
    let minutes = app
        .get(SYNC_MINUTES_KEY)
        .and_then(Value::as_u64)
        .filter(|minutes| *minutes > 0)
        .unwrap_or(DEFAULT_SYNC_MINUTES);
    Duration::from_secs(minutes.saturating_mul(60))
}

/// What the worker's index job reads besides [`ReadStore`]: both are inherent on the stores, not
/// on a trait (MOD-41 plan D19).
#[allow(async_fn_in_trait, reason = "no dyn IndexSource is formed")]
pub trait IndexSource: ReadStore {
    /// Every workspace's projects as scopes, a workspace without projects left out.
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>>;
    /// The `app_setting` map, [`sync_interval`]'s input.
    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError>;
}

impl IndexSource for PgStore {
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>> {
        scopes(self, None).await
    }

    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError> {
        self.app_settings().await
    }
}

impl IndexSource for MemStore {
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>> {
        Ok(self
            .workspaces()
            .await?
            .iter()
            .map(Scope::from_workspace)
            .filter(|scope| !scope.project_ids.is_empty())
            .collect())
    }

    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError> {
        self.app_settings().await
    }
}

/// One cycle of the worker's index job: the scopes, [`sync_all`] over them, then the settings;
/// answers how long to sleep before the next. A failure is a `warn` and never an `Err`: the next
/// cycle is the retry (MOD-41 plan D19).
pub async fn index_cycle(source: &impl IndexSource, vectors: &impl VectorStore) -> Duration {
    index_cycle_reporting(source, vectors).await.0
}

/// [`index_cycle`], and whether the sync reached the vector store and failed there (the scopes
/// read is Postgres's): [`job_cycle`] then forgets the connection (MOD-158 R1, B-L1).
async fn index_cycle_reporting(
    source: &impl IndexSource,
    vectors: &impl VectorStore,
) -> (Duration, bool) {
    let sync_failed = match source.index_scopes().await {
        Ok(scopes) => match sync_all(source, &scopes, vectors).await {
            Ok(report) => {
                tracing::info!(?report, "concepts index synced");
                false
            }
            Err(err) => {
                tracing::warn!(%err, "concepts index sync failed; next cycle");
                true
            }
        },
        Err(err) => {
            tracing::warn!(err = %format!("{err:#}"), "concepts index scopes failed; next cycle");
            false
        }
    };
    (interval_after(source).await, sync_failed)
}

/// The interval [`index_cycle`] sleeps: `concepts_sync_minutes`, read now, logged when unread.
async fn interval_after(source: &impl IndexSource) -> Duration {
    let app = match source.index_settings().await {
        Ok(app) => app,
        Err(err) => {
            tracing::warn!(%err, "concepts index interval unread; the default applies");
            BTreeMap::new()
        }
    };
    sync_interval(&app)
}

/// The worker's index job: [`index_cycle`] at start, then again after each interval, forever. It
/// never touches the run runtime, and the worker aborts it on shutdown (MOD-41 plan D19).
pub async fn index_loop(source: &impl IndexSource, vectors: &impl VectorStore) -> ! {
    loop {
        let pause = index_cycle(source, vectors).await;
        tokio::time::sleep(pause).await;
    }
}

/// One cycle of the worker's index job (MOD-158 D7): the settings, resolved now through
/// `settings` (the override or the `qdrant_url` row, and the key); then the store, through `open`
/// (in production the index's cached connection, so the model loads only once a URL resolves and
/// a changed URL or key reconnects); then [`index_cycle`]. No URL is one `info` line and the
/// interval; any other unresolved setting and a failed open are [`open_failure_level`]'s line and
/// the interval. Answers how long to sleep before the next cycle.
///
/// `forget` keeps the cached connection honest, as a failed search does (MOD-158 R1, B-L1):
/// `Some(store)` after a sync on `store` failed, so the next cycle reconnects; `None` when the
/// settings do not resolve, so no connection, and no key it was made with, outlives them.
///
/// An embedder mismatch is an `error` once, then `debug` (review L3), and is still re-checked
/// every interval rather than ending the job: its remedy (delete the collection, run `htui
/// --index-items`) happens outside this process. `mismatch` carries that across cycles.
pub async fn job_cycle<S, V, F, Fut, G>(
    source: &S,
    settings: &concepts_worker::QdrantSource,
    open: &F,
    forget: &G,
    mismatch: &mut bool,
) -> Duration
where
    S: IndexSource,
    V: VectorStore,
    F: Fn(QdrantSettings) -> Fut,
    Fut: Future<Output = Result<Arc<V>, String>>,
    G: Fn(Option<&Arc<V>>),
{
    let resolved = settings.settings().await;
    if resolved.is_err() {
        forget(None);
    }
    let (err, mismatched) = match resolved {
        Err(concepts_worker::Unresolved::NoUrl) => {
            tracing::info!("concepts index not synced: {NO_QDRANT_URL}; next cycle");
            return interval_of(source).await;
        }
        Err(why) => (why.sentence(), false),
        Ok(resolved) => match open(resolved).await {
            Ok(store) => {
                *mismatch = false;
                let (pause, sync_failed) = index_cycle_reporting(source, &*store).await;
                if sync_failed {
                    forget(Some(&store));
                }
                return pause;
            }
            Err(err) => {
                let mismatched = names_a_mismatch(&err);
                (err, mismatched)
            }
        },
    };
    match open_failure_level(mismatched, mismatch) {
        tracing::Level::ERROR => tracing::error!(
            %err,
            "concepts index refused by Qdrant; no sync until the collection is rebuilt (checked \
             again every interval, logged once)"
        ),
        tracing::Level::DEBUG => tracing::debug!(%err, "concepts index still refused"),
        _ => tracing::warn!(%err, "concepts index not opened; next cycle"),
    }
    interval_of(source).await
}

/// Whether a failed open's text (`QdrantIndex::connect`'s `{:#}` of [`qdrant_refusal`]) is
/// [`is_embedder_mismatch`]'s refusal: the store error's display wraps its message, so the
/// prefix is looked for inside it.
fn names_a_mismatch(err: &str) -> bool {
    err.contains(&format!("{EMBEDDER_MISMATCH}: "))
}

/// [`sync_interval`] of `source`'s settings, the default when they cannot be read.
async fn interval_of(source: &impl IndexSource) -> Duration {
    sync_interval(&source.index_settings().await.unwrap_or_default())
}

/// The worker's index job: [`job_cycle`] at start, then again after each interval, forever. It
/// never touches the run runtime, and the worker aborts it on shutdown (MOD-41 plan D19).
pub async fn index_job<S, V, F, Fut, G>(
    source: &S,
    settings: &concepts_worker::QdrantSource,
    open: F,
    forget: G,
) -> !
where
    S: IndexSource,
    V: VectorStore,
    F: Fn(QdrantSettings) -> Fut,
    Fut: Future<Output = Result<Arc<V>, String>>,
    G: Fn(Option<&Arc<V>>),
{
    let mut mismatch = false;
    loop {
        let pause = job_cycle(source, settings, &open, &forget, &mut mismatch).await;
        tokio::time::sleep(pause).await;
    }
}

/// The worker's index job over `pg` (MOD-158 D7): always spawned, whatever the setting says; each
/// cycle resolves the URL and key again, so one set later is indexed into without a restart.
#[must_use]
pub fn spawn_index_job(pg: PgStore) -> tokio::task::JoinHandle<()> {
    spawn_index_job_over(pg, concepts_worker::shared_qdrant())
}

/// [`spawn_index_job`] over `models` (MOD-11 H-26): its settings source and its cached connection,
/// so a worker that indexes and serves `search_concepts` loads one model, and resolves both through
/// the store and key source the worker published there.
pub(crate) fn spawn_index_job_over(
    pg: PgStore,
    models: Arc<QdrantIndex>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let settings = Arc::clone(models.source());
        let open = |resolved: QdrantSettings| {
            let models = Arc::clone(&models);
            async move { models.connect(resolved).await }
        };
        let forget = |store: Option<&Arc<_>>| models.forget(store);
        match index_job(&pg, &settings, open, forget).await {}
    })
}

/// The level [`job_cycle`] logs one failed open at: an embedder mismatch is `ERROR` the first time
/// and `DEBUG` after, since it says the same until someone rebuilds the collection; anything else
/// is `WARN` every time (review L3).
fn open_failure_level(mismatch: bool, reported: &mut bool) -> tracing::Level {
    if !mismatch {
        tracing::Level::WARN
    } else if std::mem::replace(reported, true) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::ERROR
    }
}

/// `work` on a named thread of its own, awaited. Not `spawn_blocking`: the runtime waits for its
/// blocking tasks when it is dropped, and a keyring prompt nobody answers or a model load
/// must not hold `htui worker`'s exit (`worker_cmd::read_dsn_apart`'s reason). The thread is
/// left behind on an abort and dies with the process. `None` when the thread cannot be spawned
/// or ends without answering. `concepts_worker`'s model load runs here too.
pub(crate) async fn apart<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (answer, answered) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || drop(answer.send(work())))
        .ok()?;
    answered.await.ok()
}

/// `--search-items`: the best hits, as lines or (`--json`) one document (MOD-92). Read-only:
/// `PgStore::connect_read_only` and `QdrantStore`'s read-only open; a missing collection is an
/// empty index, never created. The embedding model may still be downloaded into the user cache
/// (plan D8), and only when there is a collection to search; its notes go to stderr.
///
/// # Errors
/// [`StateExit`]: 3 when Postgres or Qdrant does not answer, 2 on a refusal (no DSN, schema, no
/// URL or an unusable one, an unreadable key, scope, embedder mismatch, a refused API key), 1 on a
/// model or read failure.
pub async fn search_items(options: &SearchOptions) -> Result<(), StateExit> {
    let answer = match search_inputs(options) {
        Err(failure) => Answer {
            kind: Kind::Search,
            outcome: Outcome::Failed(failure),
        },
        Ok((dsn, identity)) => {
            let env = QdrantEnv::from_process();
            let ctx = SearchContext {
                env: &env,
                keyring_key: secret::get_qdrant_api_key,
                collection: COLLECTION,
                dsn: &dsn,
                identity: identity.as_ref(),
                connect_timeout: CONNECT_TIMEOUT,
            };
            search_answer_with(&ctx, options, load_embedder).await
        }
    };
    state_cmd::emit(&mut std::io::stdout().lock(), &answer, options.json)
}

/// What [`search_answer_with`] needs that [`search_items`] reads from the process, the keyring
/// and `box.toml`: the test seam (review R1 M4), as `state_cmd::Context` is for `status` and
/// `waiting`. The Qdrant URL is not here: it is the `qdrant_url` row, read after the connect
/// (MOD-158 D8), unless `env` overrides it.
#[derive(Clone)]
pub struct SearchContext<'a> {
    /// `HTUI_QDRANT_URL` and `HTUI_QDRANT_API_KEY` (a test's literal, never the process's).
    pub env: &'a QdrantEnv,
    /// The keyring read of the Qdrant API key: `secret::get_qdrant_api_key`, or a test's closure.
    pub keyring_key: concepts_worker::KeyringRead,
    /// The collection searched: [`COLLECTION`], or a test's throwaway one.
    pub collection: &'a str,
    /// The Postgres DSN.
    pub dsn: &'a str,
    /// `box.toml`'s identity; read only for `--cwd`, so `None` otherwise.
    pub identity: Option<&'a Identity>,
    /// How long the Postgres connect may take.
    pub connect_timeout: Duration,
}

impl core::fmt::Debug for SearchContext<'_> {
    /// Everything but the DSN and the API key, which are secrets.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SearchContext")
            .field(
                "qdrant_url_override",
                &concepts_worker::shown_env_url(self.env),
            )
            .field("collection", &self.collection)
            .field("dsn", &"<redacted>")
            .field("identity", &self.identity)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

/// The reads before the connect: the DSN, then `box.toml` (for `--cwd` only). The Qdrant URL is
/// read after the connect (MOD-158 D8: it is a row in the database).
fn search_inputs(
    options: &SearchOptions,
) -> Result<(Zeroizing<String>, Option<Identity>), Failure> {
    let dsn = state_cmd::read_dsn()?;
    let identity = match options.cwd {
        Some(_) => state_cmd::read_identity()?,
        None => None,
    };
    Ok((dsn, identity))
}

/// Why no Qdrant settings resolved, as the search's [`Failure`]: `no_qdrant_url`,
/// `qdrant_url_unusable`, the row read's own failure, or `keyring_unreadable` for the key.
fn unresolved_failure(why: concepts_worker::Unresolved) -> Failure {
    use concepts_worker::Unresolved;
    match why {
        Unresolved::NoUrl => Failure::refused(ErrorCode::NoQdrantUrl, NO_QDRANT_URL),
        Unresolved::Unusable(sentence) => Failure::refused(ErrorCode::QdrantUrlUnusable, sentence),
        Unresolved::Read(err) => Failure::from_read(&err),
        Unresolved::Key(err) => Failure::refused(ErrorCode::KeyringUnreadable, err.to_string()),
        // A search reads through the store it connected, so it never lacks one.
        Unresolved::Offline => Failure::from_read(&StoreError::Unreachable(
            concepts_worker::URL_OFFLINE.to_owned(),
        )),
    }
}

/// `--search-items` over `ctx`, with the model from `load` (review R1 M4). Never fails: a failure
/// is an [`Answer`] too. In MOD-158 D8's order: the read-only connect, the Qdrant URL (the override
/// or the row) and key, the scope (the pool is closed once it is read, review R1 L9), then the
/// index (`search_index`: Qdrant first, the model only when there is a collection to search).
pub async fn search_answer_with<E, F, Fut>(
    ctx: &SearchContext<'_>,
    options: &SearchOptions,
    load: F,
) -> Answer
where
    E: DenseEmbedder,
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<E>>,
{
    let outcome = match search_online(ctx, options, load).await {
        Ok(search) => Outcome::Search(search),
        Err(failure) => Outcome::Failed(failure),
    };
    Answer {
        kind: Kind::Search,
        outcome,
    }
}

/// [`search_answer_with`]'s reads.
async fn search_online<E, F, Fut>(
    ctx: &SearchContext<'_>,
    options: &SearchOptions,
    load: F,
) -> Result<SearchAnswer, Failure>
where
    E: DenseEmbedder,
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<E>>,
{
    let pg = PgStore::connect_read_only(
        ctx.dsn,
        ctx.identity.map(|identity| identity.box_id),
        ctx.connect_timeout,
    )
    .await
    .map_err(Failure::from_connect)?;
    let settings = match concepts_worker::resolve_settings(
        ctx.env,
        Some(&pg),
        &KeySource::Interactive,
        ctx.keyring_key,
    )
    .await
    {
        Ok(settings) => settings,
        Err(why) => {
            pg.close().await;
            return Err(unresolved_failure(why));
        }
    };
    let projects = search_projects(&pg, options, ctx.identity).await;
    pg.close().await;
    let query = query(&options.query, projects?, options.decisions, options.limit);
    Ok(SearchAnswer {
        query: options.query.clone(),
        decisions: options.decisions,
        index: search_index(&settings, ctx.collection, &query, load).await,
    })
}

/// The projects a search reads: `--cwd`'s, `--project`'s, or every workspace's (MOD-92 D6), in
/// scope order.
async fn search_projects(
    pg: &PgStore,
    options: &SearchOptions,
    identity: Option<&Identity>,
) -> Result<Vec<ProjectId>, Failure> {
    if let Some(cwd) = options.cwd.as_deref() {
        let workspaces = pg
            .workspaces()
            .await
            .map_err(|err| Failure::from_read(&err))?;
        return Ok(vec![
            state_cmd::cwd_project(pg, &workspaces, cwd, identity).await?,
        ]);
    }
    let scopes = scopes(pg, options.project.as_deref())
        .await
        .map_err(|err| match err.downcast_ref::<StoreError>() {
            Some(err) => Failure::from_read(err),
            None => Failure::refused(ErrorCode::UnknownProject, err.to_string()),
        })?;
    Ok(projects_of(scopes))
}

/// The scopes' projects, in scope order.
fn projects_of(scopes: Vec<Scope>) -> Vec<ProjectId> {
    scopes.into_iter().flat_map(|s| s.project_ids).collect()
}

/// The collection, then the model, then the search (MOD-92 G8, review R1 M3): a Qdrant that does
/// not answer or refuses, and a missing collection (an empty index, never created), are answered
/// without loading the model; the embedder check runs on the model before anything is searched.
/// Nothing to search (no project, or a zero limit) loads no model either.
async fn search_index<E, F, Fut>(
    settings: &QdrantSettings,
    collection: &str,
    query: &SearchQuery,
    load: F,
) -> IndexAnswer
where
    E: DenseEmbedder,
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<E>>,
{
    let found = match ExistingCollection::find_in(settings, collection).await {
        Ok(Some(found)) => found,
        Ok(None) => return IndexAnswer::Empty,
        Err(err) => return index_refusal(err),
    };
    if query.projects.is_empty() || query.limit == 0 {
        return IndexAnswer::Hits(Vec::new());
    }
    let embedder = match load().await {
        Ok(embedder) => embedder,
        Err(err) => return IndexAnswer::Failed(format!("{err:#}")),
    };
    let store = match found.open(embedder).await {
        Ok(store) => store,
        Err(err) => return index_refusal(err),
    };
    match store.search(query).await {
        Ok(hits) => IndexAnswer::Hits(hits),
        Err(err) => index_refusal(err),
    }
}

/// A failed open or search (review R1 M2): a query the model failed on is the model's failure
/// (`model_unavailable`); a refused API key and an embedder mismatch are refusals
/// (`qdrant_refused`, `embedder_mismatch`); anything else "cannot reach Qdrant"
/// (`qdrant_refusal`'s sentences).
fn index_refusal(err: StoreError) -> IndexAnswer {
    if is_query_embed_failure(&err) {
        return IndexAnswer::Failed(format!(
            "{:#}",
            anyhow::Error::new(err).context("the embedding model failed on the query")
        ));
    }
    let (mismatch, refused) = (is_embedder_mismatch(&err), is_qdrant_refused(&err));
    let sentence = format!("{:#}", qdrant_refusal(err));
    if refused {
        IndexAnswer::AccessRefused(sentence)
    } else if mismatch {
        IndexAnswer::Refused(sentence)
    } else {
        IndexAnswer::Offline(sentence)
    }
}

/// One result line: key, where it matched, score and snippet. Where it matched names a closed
/// item's resolution and a withdrawn requirement (MOD-50 D227): `--decisions` lists `rejected`
/// next to `done`, and a rejected decision must not read as an adopted one.
#[must_use]
pub fn format_hit(hit: &Hit) -> String {
    let place = match (&hit.point_type, &hit.document) {
        (PointType::Document, Some((_, kind))) => format!("{kind} document"),
        (PointType::Requirement, _) => "requirement".to_owned(),
        _ => "item".to_owned(),
    };
    let place = match (hit.resolution, hit.state) {
        (Some(resolution), _) => format!("{place} ({resolution})"),
        (None, Some(RequirementState::Withdrawn)) => format!("{place} (withdrawn)"),
        _ => place,
    };
    // The snippet is already stripped of control characters (`vector::snippet`); the key and the
    // document kind are stripped here, so nothing stored can drive the terminal it is printed on.
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    format!(
        "{:<10} {:<26} {:.3}  {}",
        clean(&hit.key),
        clean(&place),
        hit.score,
        hit.snippet
    )
}

/// What one index run did, as one line (MOD-64 D237, D239): `htui --index-items` prints it on
/// stderr and the search overlay under its hits.
#[must_use]
pub fn report_line(report: &SyncReport) -> String {
    format!(
        "indexed: {} item(s) rebuilt, {} unchanged; {} requirement(s) rebuilt, {} unchanged; \
         {} point(s) written, {} removed",
        report.items_rebuilt,
        report.items_unchanged,
        report.requirements_rebuilt,
        report.requirements_unchanged,
        report.points_upserted,
        report.points_deleted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::{DocumentId, ItemId, RequirementId};
    use htui_store::vector::Owner;
    use uuid::Uuid;

    fn hit(point_type: PointType, document: Option<&str>) -> Hit {
        Hit {
            point_type,
            owner: Owner::Item(ItemId(Uuid::nil())),
            key: "ANA-11".into(),
            document: document.map(|kind| (DocumentId(Uuid::nil()), kind.to_owned())),
            resolution: None,
            state: None,
            score: 0.5,
            snippet: "a decision is a closed item".into(),
        }
    }

    /// MOD-158 T3: `HTUI_QDRANT_URL` is unvalidated, so `Debug` prints it only once it parses;
    /// a refused override may hold a password and is never echoed (nor the key, nor the DSN).
    #[test]
    fn a_search_context_debug_never_prints_an_unusable_override() {
        let shown = |url: &str| {
            let env = QdrantEnv::new(Some(url.to_owned()), Some("KEY-SENTINEL".to_owned()));
            let ctx = SearchContext {
                env: &env,
                keyring_key: || Ok(None),
                collection: "c",
                dsn: "postgres://u:DSN-SENTINEL@db/htui",
                identity: None,
                connect_timeout: Duration::from_secs(1),
            };
            format!("{ctx:?}")
        };
        let refused = shown("http://user:SENTINEL@env:6334");
        assert!(!refused.contains("SENTINEL"), "{refused}");
        assert!(refused.contains("<not a usable URL>"), "{refused}");
        let shown = shown(" http://env:6334 ");
        assert!(shown.contains("http://env:6334"), "{shown}");
        assert!(!shown.contains("SENTINEL"), "{shown}");
    }

    #[test]
    fn only_an_unreachable_server_is_cannot_reach() {
        use htui_core::store::StoreError;
        assert_eq!(
            store_context(&StoreError::Unreachable("refused".into())),
            "cannot reach Postgres"
        );
        assert_eq!(
            store_context(&StoreError::Backend("the schema is newer".into())),
            "Postgres refused this htui"
        );
    }

    /// Review L3: Qdrant answered and refused, so the line says that, not "cannot reach".
    #[test]
    fn an_embedder_mismatch_is_not_cannot_reach_qdrant() {
        use htui_store::vector::EMBEDDER_MISMATCH;
        let mismatch = StoreError::Backend(format!(
            "{EMBEDDER_MISMATCH}: collection `c` holds vectors from hash/384; delete collection \
             `c` and run `htui --index-items` to rebuild it"
        ));
        let text = format!("{:#}", qdrant_refusal(mismatch));
        assert!(text.contains(EMBEDDER_MISMATCH), "{text}");
        assert!(!text.contains("cannot reach"), "{text}");
        assert!(
            names_a_mismatch(&text),
            "the job's open reads it as one: {text}"
        );
        let down = StoreError::Backend("qdrant: collection exists: transport error".into());
        let text = format!("{:#}", qdrant_refusal(down));
        assert!(!names_a_mismatch(&text), "{text}");
        assert!(text.starts_with("cannot reach Qdrant: "), "{text}");
        assert!(
            text.ends_with("qdrant: collection exists: transport error"),
            "{text}"
        );
    }

    /// Review L3: the index job says a mismatch once, at `error`, and a Qdrant that does not
    /// answer every cycle, at `warn`.
    #[test]
    fn a_mismatch_is_an_error_once_then_debug_and_anything_else_a_warning() {
        use tracing::Level;
        let mut reported = false;
        assert_eq!(open_failure_level(false, &mut reported), Level::WARN);
        assert_eq!(open_failure_level(true, &mut reported), Level::ERROR);
        assert_eq!(open_failure_level(true, &mut reported), Level::DEBUG);
        assert_eq!(open_failure_level(false, &mut reported), Level::WARN);
        assert_eq!(open_failure_level(true, &mut reported), Level::DEBUG);
    }

    #[test]
    fn hits_print_key_place_score_and_snippet() {
        let line = format_hit(&hit(PointType::Document, Some("summary")));
        assert!(line.starts_with("ANA-11"));
        assert!(line.contains("summary document"));
        assert!(line.contains("0.500"));
        assert!(line.ends_with("a decision is a closed item"));
    }

    #[test]
    fn an_open_item_prints_as_before() {
        let line = format_hit(&hit(PointType::Item, None));
        assert!(line.contains("item "), "{line}");
        assert!(!line.contains('('), "{line}");
    }

    #[test]
    fn a_closed_item_and_its_documents_name_the_resolution() {
        let rejected = Hit {
            resolution: Some(Resolution::Rejected),
            ..hit(PointType::Item, None)
        };
        assert!(format_hit(&rejected).contains("item (rejected)"));
        let done_doc = Hit {
            resolution: Some(Resolution::Done),
            ..hit(PointType::Document, Some("summary"))
        };
        assert!(format_hit(&done_doc).contains("summary document (done)"));
    }

    #[test]
    fn requirements_print_as_such_and_say_when_withdrawn() {
        let active = Hit {
            owner: Owner::Requirement(RequirementId(Uuid::nil())),
            key: "R-STO-8".into(),
            state: Some(RequirementState::Active),
            ..hit(PointType::Requirement, None)
        };
        let line = format_hit(&active);
        assert!(line.starts_with("R-STO-8"), "{line}");
        assert!(line.contains("requirement "), "{line}");
        assert!(!line.contains('('), "{line}");
        let withdrawn = Hit {
            state: Some(RequirementState::Withdrawn),
            ..active
        };
        assert!(format_hit(&withdrawn).contains("requirement (withdrawn)"));
    }

    #[test]
    fn decisions_are_done_concluded_and_rejected() {
        assert_eq!(
            DECISION_RESOLUTIONS,
            [
                Resolution::Done,
                Resolution::Concluded,
                Resolution::Rejected
            ]
        );
    }

    #[test]
    fn query_narrows_to_decisions_only_when_asked() {
        let projects = vec![ProjectId(Uuid::from_u128(2)), ProjectId(Uuid::from_u128(1))];
        for decisions in [false, true] {
            let q = query("closed items", projects.clone(), decisions, 7);
            assert_eq!(q.text, "closed items");
            assert_eq!(q.projects, projects);
            assert_eq!(q.limit, 7);
            assert!(q.types.is_empty() && q.statuses.is_empty(), "{q:?}");
            let expected = if decisions {
                DECISION_RESOLUTIONS.to_vec()
            } else {
                Vec::new()
            };
            assert_eq!(q.resolutions, expected, "decisions = {decisions}");
        }
    }

    /// MOD-92 D6: without `--project` or `--cwd`, `--search-items` still searches every
    /// workspace's projects, in workspace order, each workspace's by position; `--project`
    /// narrows to one. Through the real scope code over the demo database (review R1 M4).
    #[tokio::test]
    async fn search_projects_default_to_every_workspace_and_project_narrows() {
        let Some(db) = htui_store::testkit::demo_db().await else {
            return;
        };
        let pg = PgStore::connect_read_only(&db.url, None, CONNECT_TIMEOUT)
            .await
            .expect("connect read-only");
        let options = SearchOptions {
            query: "closed items".into(),
            project: None,
            decisions: false,
            limit: DEFAULT_LIMIT,
            cwd: None,
            json: true,
        };
        let workspaces = pg.workspaces().await.expect("workspaces");
        assert!(workspaces.len() >= 2, "{workspaces:?}");
        let expected: Vec<ProjectId> = workspaces
            .iter()
            .flat_map(|ws| Scope::from_workspace(ws).project_ids)
            .collect();
        let every = search_projects(&pg, &options, None)
            .await
            .expect("the default scope");
        assert_eq!(every, expected);
        let one = SearchOptions {
            project: Some("htui".into()),
            ..options.clone()
        };
        assert_eq!(
            search_projects(&pg, &one, None).await.expect("htui"),
            [htui_core::fixtures::ids::PROJECT_HTUI]
        );
        pg.close().await;
        db.drop_db().await;
    }

    /// Review R1 M2: a refused API key is `qdrant_refused` (not "cannot reach"), a query the model
    /// fails on is the model's failure, a mismatch is a refusal, and the rest is offline.
    #[test]
    fn index_refusals_by_cause() {
        use htui_store::vector::{EMBEDDER_MISMATCH, QDRANT_REFUSED, QUERY_EMBED_FAILED};
        let refused = index_refusal(StoreError::Backend(format!(
            "{QDRANT_REFUSED}: collection exists: Invalid api-key; check the Qdrant API key in \
             Settings > Qdrant"
        )));
        let IndexAnswer::AccessRefused(sentence) = &refused else {
            panic!("{refused:?}");
        };
        assert!(!sentence.contains("cannot reach"), "{sentence}");
        assert!(sentence.contains("API key"), "{sentence}");
        assert!(matches!(
            index_refusal(StoreError::Backend(format!(
                "{QUERY_EMBED_FAILED}: backend error: the session failed"
            ))),
            IndexAnswer::Failed(_)
        ));
        assert!(matches!(
            index_refusal(StoreError::Backend(format!(
                "{EMBEDDER_MISMATCH}: collection `c` holds vectors from hash/384"
            ))),
            IndexAnswer::Refused(_)
        ));
        let IndexAnswer::Offline(sentence) = index_refusal(StoreError::Backend(
            "qdrant: collection exists: transport error".into(),
        )) else {
            panic!("a Qdrant that does not answer is offline");
        };
        assert!(sentence.starts_with("cannot reach Qdrant: "), "{sentence}");
    }

    /// `--index-items` and the worker say the same: a refused key is not "cannot reach".
    #[test]
    fn a_refused_api_key_is_not_cannot_reach_qdrant() {
        use htui_store::vector::QDRANT_REFUSED;
        let err = StoreError::Backend(format!("{QDRANT_REFUSED}: collection exists: denied"));
        let text = format!("{:#}", qdrant_refusal(err));
        assert!(text.contains(QDRANT_REFUSED), "{text}");
        assert!(!text.contains("cannot reach"), "{text}");
    }

    #[test]
    fn report_line_is_index_items_wording() {
        let report = SyncReport {
            items_rebuilt: 1,
            items_unchanged: 2,
            requirements_rebuilt: 3,
            requirements_unchanged: 4,
            points_upserted: 5,
            points_deleted: 6,
        };
        assert_eq!(
            report_line(&report),
            "indexed: 1 item(s) rebuilt, 2 unchanged; 3 requirement(s) rebuilt, 4 unchanged; \
             5 point(s) written, 6 removed"
        );
    }

    /// The search runtime shares one loaded model between its tasks (MOD-64 D238), the index job
    /// keeps one across retries, and both move it across tasks; `htui-store`'s own tests build
    /// without `local-embed`, so the check lives here (D258).
    #[test]
    fn rten_embedder_is_clone_send_and_sync() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        shared::<RtenEmbedder>();
    }

    /// The worker's index job (MOD-41 plan D19, blueprint §16) over `MemStore` and
    /// `MemVectorStore`, under paused time.
    mod index_job {
        use super::super::*;
        use htui_core::fixtures::ids;
        use htui_store::vector::{ConceptPoint, IndexedPoint, MemVectorStore};
        use serde_json::json;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use uuid::Uuid;

        const MINUTE: Duration = Duration::from_secs(60);

        /// `MemVectorStore`, counting the syncs that reach it (one `indexed` read of the htui
        /// project per whole sync) and every `indexed` read (a failed sync stops part-way, perhaps
        /// before the htui project), and failing the first `fail` upserts.
        #[derive(Debug, Default)]
        struct Counting {
            inner: MemVectorStore,
            syncs: AtomicUsize,
            reads: AtomicUsize,
            fail: AtomicUsize,
        }

        impl Counting {
            fn failing_once() -> Self {
                Self {
                    fail: AtomicUsize::new(1),
                    ..Self::default()
                }
            }

            fn syncs(&self) -> usize {
                self.syncs.load(Ordering::SeqCst)
            }

            fn reads(&self) -> usize {
                self.reads.load(Ordering::SeqCst)
            }
        }

        impl VectorStore for Counting {
            async fn upsert(&self, points: Vec<ConceptPoint>) -> Result<(), StoreError> {
                let failing = self
                    .fail
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok();
                if failing {
                    return Err(StoreError::Backend(
                        "qdrant: upsert: planted failure".into(),
                    ));
                }
                self.inner.upsert(points).await
            }

            async fn delete(&self, ids: Vec<Uuid>) -> Result<(), StoreError> {
                self.inner.delete(ids).await
            }

            async fn indexed(&self, project: ProjectId) -> Result<Vec<IndexedPoint>, StoreError> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                if project == ids::PROJECT_HTUI {
                    self.syncs.fetch_add(1, Ordering::SeqCst);
                }
                self.inner.indexed(project).await
            }

            async fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, StoreError> {
                self.inner.search(query).await
            }
        }

        /// Runs `checks` beside `index_loop(read, vectors)`; the loop never ends on its own, so
        /// `checks` ending first is also the proof it was still alive.
        async fn beside_the_loop(
            read: &MemStore,
            vectors: &Counting,
            checks: impl Future<Output = ()>,
        ) {
            tokio::select! {
                biased;
                () = checks => {}
                never = index_loop(read, vectors) => match never {},
            }
        }

        #[tokio::test(start_paused = true)]
        async fn the_index_job_syncs_at_start_and_every_interval() {
            let read = MemStore::demo();
            let vectors = Counting::default();
            beside_the_loop(&read, &vectors, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert_eq!(vectors.syncs(), 1, "one sync at start");
                assert!(
                    !vectors.inner.points().is_empty(),
                    "the start sync indexed the demo items"
                );
                tokio::time::sleep(15 * MINUTE - Duration::from_secs(2)).await;
                assert_eq!(vectors.syncs(), 1, "nothing before the default 15 minutes");
                tokio::time::sleep(Duration::from_secs(2)).await;
                assert_eq!(vectors.syncs(), 2, "the second sync after 15 minutes");
                tokio::time::sleep(15 * MINUTE).await;
                assert_eq!(vectors.syncs(), 3, "the third after 30");
            })
            .await;
        }

        #[tokio::test(start_paused = true)]
        async fn a_failing_sync_is_logged_and_retried_next_cycle() {
            let read = MemStore::demo();
            let vectors = Counting::failing_once();
            beside_the_loop(&read, &vectors, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let failed = vectors.reads();
                assert!(failed > 0, "the start sync ran");
                assert!(
                    vectors.inner.points().is_empty(),
                    "and failed at its first upsert"
                );
                tokio::time::sleep(15 * MINUTE - Duration::from_secs(2)).await;
                assert_eq!(vectors.reads(), failed, "no retry before the next cycle");
                tokio::time::sleep(Duration::from_secs(2)).await;
                assert!(vectors.reads() > failed, "the loop outlived the failure");
                assert!(
                    vectors.syncs() >= 1,
                    "the next cycle synced the htui project"
                );
                assert!(
                    !vectors.inner.points().is_empty(),
                    "the next cycle indexed the demo items"
                );
            })
            .await;
        }

        #[tokio::test(start_paused = true)]
        async fn the_interval_reads_concepts_sync_minutes_with_a_15_minute_default() {
            let default = DEFAULT_SYNC_MINUTES * MINUTE.as_secs();
            assert_eq!(default, 15 * 60);
            let with = |value: Value| BTreeMap::from([(SYNC_MINUTES_KEY.to_owned(), value)]);
            assert_eq!(sync_interval(&BTreeMap::new()).as_secs(), default, "absent");
            for value in [
                json!(0),
                json!(-3),
                json!("5"),
                json!(2.5),
                json!(null),
                json!(true),
            ] {
                assert_eq!(
                    sync_interval(&with(value.clone())).as_secs(),
                    default,
                    "{value}"
                );
            }
            assert_eq!(sync_interval(&with(json!(5))), 5 * MINUTE);
            assert_eq!(
                sync_interval(&with(json!(u64::MAX))),
                Duration::from_secs(u64::MAX),
                "a huge value saturates rather than overflowing"
            );

            let read = MemStore::demo();
            let vectors = Counting::default();
            beside_the_loop(&read, &vectors, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert_eq!(vectors.syncs(), 1);
                read.set_app_setting(SYNC_MINUTES_KEY, json!(1));
                tokio::time::sleep(15 * MINUTE).await;
                assert_eq!(
                    vectors.syncs(),
                    2,
                    "the sleep in progress keeps its 15 minutes"
                );
                tokio::time::sleep(MINUTE).await;
                assert_eq!(vectors.syncs(), 3, "the next sleep is the new minute");
            })
            .await;
        }

        /// A pool that has never connected: the job's start must not need it.
        fn unreachable_store() -> PgStore {
            let identity = Identity {
                box_id: htui_core::model::BoxId::new(),
                hostname: "HTUI-TEST".to_owned(),
            };
            PgStore::lazy(
                "postgres://nobody:nothing@127.0.0.1:1/none",
                &identity,
                Duration::from_millis(250),
            )
            .expect("a lazy pool opens no socket")
        }

        /// The job's settings source over `store`, no override, a keyring holding no key.
        fn settings_over(store: &MemStore) -> concepts_worker::QdrantSource {
            let source =
                concepts_worker::QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)
                    .with_keyring(|| Ok(None));
            source.publish(Some(Arc::new(store.clone())));
            source
        }

        /// Sets `store`'s `qdrant_url` row to `text`, over whatever it holds.
        async fn set_row(store: &MemStore, text: &str) {
            let token = store
                .qdrant_url()
                .await
                .expect("the memory store never fails")
                .map(|row| row.token);
            store
                .set_qdrant_url(
                    &htui_core::qdrant::QdrantUrl::parse(text).expect("a valid URL"),
                    token,
                )
                .await
                .expect("the memory store never fails");
        }

        /// `open` for the job: records the URL of every open, and fails the first `fail` of them;
        /// and `forget`: records each call, `true` for a store whose sync failed (it must be the
        /// one opened), `false` for "drop every cached connection".
        #[derive(Default)]
        struct Opens {
            urls: std::sync::Mutex<Vec<String>>,
            fail: AtomicUsize,
            vectors: Arc<Counting>,
            forgotten: std::sync::Mutex<Vec<bool>>,
        }

        impl Opens {
            fn failing_once() -> Self {
                Self {
                    fail: AtomicUsize::new(1),
                    ..Self::default()
                }
            }

            /// Opens that succeed, over a store whose first upsert fails.
            fn syncing_badly_once() -> Self {
                Self {
                    vectors: Arc::new(Counting::failing_once()),
                    ..Self::default()
                }
            }

            fn forget(&self, store: Option<&Arc<Counting>>) {
                if let Some(store) = store {
                    assert!(Arc::ptr_eq(store, &self.vectors), "the store it opened");
                }
                self.forgotten
                    .lock()
                    .expect("not poisoned")
                    .push(store.is_some());
            }

            fn forgotten(&self) -> Vec<bool> {
                self.forgotten.lock().expect("not poisoned").clone()
            }

            fn open(
                &self,
                settings: QdrantSettings,
            ) -> std::future::Ready<Result<Arc<Counting>, String>> {
                self.urls
                    .lock()
                    .expect("not poisoned")
                    .push(settings.url.to_string());
                let failing = self
                    .fail
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok();
                std::future::ready(if failing {
                    Err("cannot reach Qdrant: planted failure".to_owned())
                } else {
                    Ok(Arc::clone(&self.vectors))
                })
            }

            fn urls(&self) -> Vec<String> {
                self.urls.lock().expect("not poisoned").clone()
            }
        }

        /// Runs `checks` beside `index_job(read, settings, opens)`, as `beside_the_loop`.
        async fn beside_the_job(
            read: &MemStore,
            settings: &concepts_worker::QdrantSource,
            opens: &Opens,
            checks: impl Future<Output = ()>,
        ) {
            tokio::select! {
                biased;
                () = checks => {}
                never = index_job(read, settings, |s| opens.open(s), |s| opens.forget(s)) => {
                    match never {}
                }
            }
        }

        /// MOD-158 D7: no URL is one `info` line and the interval; nothing is opened, nothing
        /// loaded.
        #[tokio::test(start_paused = true)]
        async fn no_url_is_one_info_line_then_the_interval() {
            let read = MemStore::demo();
            let settings = settings_over(&read);
            let opens = Opens::default();
            let mut mismatch = false;
            let pause = job_cycle(
                &read,
                &settings,
                &|s| opens.open(s),
                &|s| opens.forget(s),
                &mut mismatch,
            )
            .await;
            assert_eq!(pause, 15 * MINUTE);
            assert!(opens.urls().is_empty(), "never opened without a URL");
            assert_eq!(opens.vectors.reads(), 0);
            assert_eq!(
                opens.forgotten(),
                [false],
                "MOD-158 R1 (B-L1): settings that stop resolving drop the cached connection"
            );
        }

        /// MOD-158 R1 (B-L1): a sync that fails forgets the connection it used, so the next cycle
        /// reconnects, as a failed search already does; a sync that succeeds forgets nothing.
        #[tokio::test(start_paused = true)]
        async fn a_failed_sync_forgets_its_connection() {
            let read = MemStore::demo();
            set_row(&read, "http://qdrant:6334").await;
            let settings = settings_over(&read);
            let opens = Opens::syncing_badly_once();
            let mut mismatch = false;
            let (open, forget) = (
                |s| opens.open(s),
                |s: Option<&Arc<Counting>>| opens.forget(s),
            );
            job_cycle(&read, &settings, &open, &forget, &mut mismatch).await;
            assert!(opens.vectors.inner.points().is_empty(), "the sync failed");
            assert_eq!(opens.forgotten(), [true], "the failed sync's store");
            job_cycle(&read, &settings, &open, &forget, &mut mismatch).await;
            assert!(!opens.vectors.inner.points().is_empty(), "the next synced");
            assert_eq!(opens.forgotten(), [true], "a good sync forgets nothing");
        }

        /// MOD-158 D7: a URL set while the job runs is used by its next cycle, no restart.
        #[tokio::test(start_paused = true)]
        async fn a_url_set_after_the_job_starts_is_used_by_the_next_cycle() {
            let read = MemStore::demo();
            let settings = settings_over(&read);
            let opens = Opens::default();
            beside_the_job(&read, &settings, &opens, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert!(opens.urls().is_empty(), "no URL at start");
                set_row(&read, "http://qdrant:6334").await;
                tokio::time::sleep(15 * MINUTE).await;
                assert_eq!(opens.urls(), ["http://qdrant:6334"]);
                assert_eq!(opens.vectors.syncs(), 1, "the next cycle synced");
            })
            .await;
        }

        /// MOD-158 D7: a changed URL is what the next cycle opens.
        #[tokio::test(start_paused = true)]
        async fn a_changed_url_is_opened_on_the_next_cycle() {
            let read = MemStore::demo();
            set_row(&read, "http://first:6334").await;
            let settings = settings_over(&read);
            let opens = Opens::default();
            beside_the_job(&read, &settings, &opens, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert_eq!(opens.urls(), ["http://first:6334"]);
                set_row(&read, "http://second:6334").await;
                tokio::time::sleep(15 * MINUTE).await;
                assert_eq!(opens.urls(), ["http://first:6334", "http://second:6334"]);
                assert_eq!(opens.vectors.syncs(), 2);
            })
            .await;
        }

        /// MOD-158 D7: a failed open is a log line and the interval; the next cycle opens again.
        #[tokio::test(start_paused = true)]
        async fn a_failed_open_is_retried_next_cycle() {
            let read = MemStore::demo();
            set_row(&read, "http://qdrant:6334").await;
            let settings = settings_over(&read);
            let opens = Opens::failing_once();
            let mut mismatch = false;
            let (open, forget) = (
                |s| opens.open(s),
                |s: Option<&Arc<Counting>>| opens.forget(s),
            );
            let pause = job_cycle(&read, &settings, &open, &forget, &mut mismatch).await;
            assert_eq!(pause, 15 * MINUTE);
            assert_eq!(
                opens.vectors.reads(),
                0,
                "nothing synced after a failed open"
            );
            job_cycle(&read, &settings, &open, &forget, &mut mismatch).await;
            assert_eq!(opens.urls().len(), 2, "opened again");
            assert_eq!(opens.vectors.syncs(), 1);
        }

        /// MOD-158 D7: the job starts whatever the setting says, and runs until it is aborted.
        /// MOD-158 R1 (B-L8): its first cycle really ran: the published row, which says there is
        /// no URL, was read through the index's settings source.
        #[tokio::test]
        async fn the_job_starts_whatever_the_setting_says() {
            /// A row that is not there, counting its reads.
            #[derive(Default)]
            struct NoRow(AtomicUsize);
            impl concepts_worker::UrlRow for NoRow {
                fn qdrant_url(
                    &self,
                ) -> futures::future::BoxFuture<
                    '_,
                    Result<Option<htui_core::qdrant::StoredQdrantUrl>, StoreError>,
                > {
                    self.0.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Ok(None) })
                }
            }

            let source =
                concepts_worker::QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)
                    .with_keyring(|| Ok(None));
            let row = Arc::new(NoRow::default());
            source.publish(Some(Arc::clone(&row) as Arc<dyn concepts_worker::UrlRow>));
            let index = Arc::new(
                QdrantIndex::with_loader(|| Box::pin(std::future::pending()))
                    .with_source(Arc::new(source)),
            );
            let job = spawn_index_job_over(unreachable_store(), index);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while row.0.load(Ordering::SeqCst) == 0 {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the job's first cycle never read the setting"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(!job.is_finished(), "the job runs until it is aborted");
            job.abort();
            assert!(job.await.expect_err("aborted").is_cancelled());
        }

        /// MOD-11 H-26: the index job and every search take the model from one load slot on the
        /// index `search_concepts` searches, so a worker that indexes and serves the tool loads the
        /// 133 MB model once, not twice. The second taker waits on the slot directly (MOD-160
        /// P-1): a search asks Qdrant before the model, and this one's Qdrant is a closed port.
        #[tokio::test]
        async fn the_index_job_and_the_search_share_one_model_load() {
            let loads = Arc::new(AtomicUsize::new(0));
            let settings = concepts_worker::QdrantSource::new(
                QdrantEnv::new(Some("http://127.0.0.1:1".to_owned()), None),
                KeySource::Interactive,
            )
            .with_keyring(|| Ok(None));
            let index = Arc::new(
                QdrantIndex::with_loader({
                    let loads = Arc::clone(&loads);
                    move || {
                        loads.fetch_add(1, Ordering::SeqCst);
                        // A load still in flight: everyone who wants the model waits on this one.
                        Box::pin(std::future::pending())
                    }
                })
                .with_source(Arc::new(settings)),
            );

            let job = spawn_index_job_over(unreachable_store(), Arc::clone(&index));
            tokio::time::timeout(Duration::from_secs(10), async {
                while loads.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the job loads its model through the index");

            let model = index.embedder();
            assert!(
                tokio::time::timeout(Duration::from_millis(500), model)
                    .await
                    .is_err(),
                "a search's model waits on the job's load"
            );
            assert_eq!(loads.load(Ordering::SeqCst), 1, "one model for both");
            job.abort();
        }
    }
}
