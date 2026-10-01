//! The concepts index behind the TUI's search (MOD-64 D230-D232, D237, D238).
//!
//! `SearchConcepts` loads an embedding model and calls Qdrant; `IndexConcepts` reads every item of
//! the scope. Neither may run in the store worker's serial loop (`R-NF-3`): [`ConceptsRuntime`]
//! serves both on tasks of their own, the way `AgentRuntime::preview` serves a preview, and each
//! task sends its own reply. The index is an object-safe [`ConceptIndex`]: [`QdrantIndex`] in
//! production, `MemIndex` in tests.

use std::collections::HashMap;
use std::mem::Discriminant;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use htui_core::model::Scope;
use htui_core::store::StoreError;
use htui_store::embed::RtenEmbedder;
use htui_store::model;
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Hit, QdrantStore, SearchQuery, VectorStore as _};
use htui_store::vector_sync::{Indexer, SyncReport};
use htui_store::{Backend, secret};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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
}

/// How the embedding model is loaded: [`load_model`] in production, a counter in a test.
type Loader = Arc<dyn Fn() -> BoxFuture<'static, Result<RtenEmbedder, String>> + Send + Sync>;

/// The pinned files (fetched or adopted, on this task), then `RtenEmbedder::load` on the blocking
/// pool (D238, MOD-68 D7).
fn load_model() -> BoxFuture<'static, Result<RtenEmbedder, String>> {
    Box::pin(async {
        let files = model::ensure_model().await.map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || RtenEmbedder::load(&files))
            .await
            .map_err(|e| format!("the embedding model's loader stopped: {e}"))?
            .map_err(|e| e.to_string())
    })
}

/// The production index (D238, amended at review 3): settings re-read from the keyring per
/// request, so a URL changed in Settings > Qdrant applies to the next search; the Qdrant connection
/// reused while the URL and key stay the same; the embedding model loaded once, on the blocking
/// pool, and shared.
pub struct QdrantIndex {
    /// The model's load: in flight, done, or failed. One at a time (review 2): a failed one is
    /// replaced by the next search's, a done one answers every later search (D238).
    load: Mutex<Option<Load>>,
    /// What loads the model.
    loader: Loader,
    /// The last connection made, for the settings it was made with (review 3).
    connections: Connections<QdrantStore<RtenEmbedder>>,
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
    fn default() -> Self {
        Self::with_loader(load_model)
    }
}

impl QdrantIndex {
    /// An index with no model loaded yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An index whose model comes from `loader` (review 2: the seam a test counts loads through).
    fn with_loader(
        loader: impl Fn() -> BoxFuture<'static, Result<RtenEmbedder, String>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            load: Mutex::new(None),
            loader: Arc::new(loader),
            connections: Connections::default(),
        }
    }

    /// The model, loading it on first use (F9, blueprint D245).
    async fn embedder(&self) -> Result<RtenEmbedder, String> {
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
    /// the same URL and key, else a new one, cached (review 3).
    async fn connect(
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
            .map_err(|e| format!("cannot reach Qdrant: {e}"))?;
        self.connections.put(settings, Arc::clone(&store));
        Ok(store)
    }
}

/// The keyring's URL and key, read on the blocking pool (`QdrantSnapshot::fetch`'s rule), then
/// `concepts::settings_from`.
async fn qdrant_settings() -> Result<QdrantSettings, String> {
    let (url, api_key) = tokio::task::spawn_blocking(|| {
        Ok::<_, StoreError>((secret::get_qdrant_url()?, secret::get_qdrant_api_key()?))
    })
    .await
    .map_err(|e| format!("the keyring read stopped: {e}"))?
    .map_err(|e| e.to_string())?;
    crate::concepts::settings_from(url, api_key).map_err(|e| format!("{e:#}"))
}

// Settings before the model, in both: a box with no URL fails in microseconds without downloading
// anything. Any failure drops the connection it used, so the next request reconnects: telling a
// dropped connection from a refused query would mean parsing the client's errors, and a spare
// reconnect after a refusal costs what every request cost before the cache (review 3).
impl ConceptIndex for QdrantIndex {
    fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
        Box::pin(async move {
            let settings = qdrant_settings().await?;
            let store = self.connect(settings).await?;
            store.search(&query).await.map_err(|e| {
                self.connections.forget(&store);
                e.to_string()
            })
        })
    }

    fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>> {
        Box::pin(async move {
            let settings = qdrant_settings().await?;
            let store = self.connect(settings).await?;
            Indexer::sync(&backend, &scope, &*store).await.map_err(|e| {
                self.connections.forget(&store);
                e.to_string()
            })
        })
    }
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
        }
    }

    /// Over [`QdrantIndex`]: what `store_worker::spawn_with_runtimes` builds (D231).
    #[must_use]
    pub fn production() -> Self {
        Self::new(Arc::new(QdrantIndex::new()))
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

    /// The keyring fake is one process-wide slot: run under `--test-threads=1`.
    #[tokio::test]
    async fn qdrant_index_without_a_stored_url_names_where_to_set_it() {
        let _keyring = htui_store::testkit::mock_keyring().await;
        let (backend, scope) = platform().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let index = QdrantIndex::with_loader({
            let attempts = Arc::clone(&attempts);
            move || {
                attempts.fetch_add(1, Ordering::SeqCst);
                load_model()
            }
        });
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        let searched = index.search(query).await.expect_err("no URL is stored");
        assert!(searched.contains("Settings > Qdrant"), "{searched}");
        let synced = index
            .sync(backend, scope)
            .await
            .expect_err("no URL is stored");
        assert!(synced.contains("Settings > Qdrant"), "{synced}");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            0,
            "the settings are read before the model: nothing was loaded"
        );
    }

    fn settings(url: &str, key: Option<&str>) -> QdrantSettings {
        QdrantSettings::new(url.to_owned(), key.map(str::to_owned)).expect("a valid URL")
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
