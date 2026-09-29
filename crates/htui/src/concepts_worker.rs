//! The concepts index behind the TUI's search (MOD-64 D230-D232, D237, D238).
//!
//! `SearchConcepts` loads an embedding model and calls Qdrant; `IndexConcepts` reads every item of
//! the scope. Neither may run in the store worker's serial loop (`R-NF-3`): [`ConceptsRuntime`]
//! serves both on tasks of their own, the way `AgentRuntime::preview` serves a preview, and each
//! task sends its own reply. The index is an object-safe [`ConceptIndex`]: [`QdrantIndex`] in
//! production, `MemIndex` in tests.

use std::collections::HashMap;
use std::mem::Discriminant;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use htui_core::model::Scope;
use htui_core::store::StoreError;
use htui_store::embed::FastEmbedder;
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Hit, QdrantStore, SearchQuery, VectorStore as _};
use htui_store::vector_sync::{Indexer, SyncReport};
use htui_store::{Backend, secret};
use tokio::sync::{OnceCell, mpsc};
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

/// The production index (D238): settings re-read from the keyring and Qdrant re-connected per
/// request, so a URL changed in Settings > Qdrant applies to the next search; the embedding model
/// loaded once, on the blocking pool, and shared.
#[derive(Debug, Default)]
pub struct QdrantIndex {
    /// Filled by the first load that succeeds; a failed load leaves it empty (D238).
    embedder: Arc<OnceCell<FastEmbedder>>,
}

impl QdrantIndex {
    /// An index with no model loaded yet.
    #[must_use]
    pub fn new() -> Self {
        todo!()
    }

    /// The model, loading it on first use (F9, blueprint D245).
    async fn embedder(&self) -> Result<FastEmbedder, String> {
        todo!()
    }

    /// Qdrant at `settings`, over the shared model.
    async fn connect(&self, settings: &QdrantSettings) -> Result<QdrantStore<FastEmbedder>, String> {
        let _ = settings;
        let _ = self.embedder().await;
        todo!()
    }
}

/// The keyring's URL and key, read on the blocking pool (`QdrantSnapshot::fetch`'s rule), then
/// `concepts::settings_from`.
async fn qdrant_settings() -> Result<QdrantSettings, String> {
    let _ = (secret::get_qdrant_url, StoreError::Backend);
    todo!()
}

impl ConceptIndex for QdrantIndex {
    fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
        Box::pin(async move {
            let settings = qdrant_settings().await?;
            let store = self.connect(&settings).await?;
            let _ = (store, query);
            todo!()
        })
    }

    fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>> {
        Box::pin(async move {
            let _ = (backend, scope, Indexer);
            todo!()
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
        todo!()
    }

    /// Every call answers `Err(message)` (after the delay, if any).
    #[must_use]
    pub fn failing(self, message: impl Into<String>) -> Self {
        let _ = message.into();
        todo!()
    }

    /// Every call sleeps `by` first (`tokio::time::sleep`, so a paused clock advances it).
    #[must_use]
    pub fn delayed(self, by: Duration) -> Self {
        let _ = by;
        todo!()
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
        let _ = (backend, scope);
        todo!()
    }
}

#[cfg(any(test, feature = "testkit"))]
impl ConceptIndex for MemIndex {
    fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>> {
        Box::pin(async move {
            let _ = query;
            todo!()
        })
    }

    fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>> {
        Box::pin(async move {
            let _ = (backend, scope);
            todo!()
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
        let _ = index;
        todo!()
    }

    /// Over [`QdrantIndex`]: what `store_worker::spawn_with_runtimes` builds (D231).
    #[must_use]
    pub fn production() -> Self {
        todo!()
    }

    /// Spawns the request's task and answers `Deferred`, or answers now. Awaits nothing.
    pub fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
    ) -> ConceptsServed {
        let _ = (backend, replies, envelope, ConceptsReply::Indexed);
        todo!()
    }

    /// Harness only (`RunRuntime::settle`'s shape): awaits every task, each within `limit`, and
    /// answers how many did not finish (they are aborted).
    pub async fn settle(&mut self, limit: Duration) -> usize {
        let _ = limit;
        todo!()
    }

    /// The UI is gone: every task is aborted. An index run stopped half-way is safe: the next run
    /// rebuilds what it did not reach.
    pub fn shutdown(&mut self) {
        todo!()
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
        assert_eq!(outcome, &Ok(Vec::new()), "the failing index was never called");
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
        let second = runtime.serve(&backend, &tx, &envelope(2, StoreRequest::SearchConcepts(query)));
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
        assert_eq!(REQUEST_NAMES[1], StoreRequest::IndexConcepts { scope }.name());
    }

    /// The keyring fake is one process-wide slot: run under `--test-threads=1`.
    #[tokio::test]
    async fn qdrant_index_without_a_stored_url_names_where_to_set_it() {
        let _keyring = htui_store::testkit::mock_keyring().await;
        let (backend, scope) = platform().await;
        let index = QdrantIndex::new();
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        let searched = index.search(query).await.expect_err("no URL is stored");
        assert!(searched.contains("Settings > Qdrant"), "{searched}");
        let synced = index
            .sync(backend, scope)
            .await
            .expect_err("no URL is stored");
        assert!(synced.contains("Settings > Qdrant"), "{synced}");
        assert!(
            index.embedder.get().is_none(),
            "the settings are read before the model: nothing was loaded"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_aborts_every_task() {
        let (backend, scope) = platform().await;
        let index = Arc::new(MemIndex::new().delayed(Duration::from_secs(3600)));
        let mut runtime = ConceptsRuntime::new(index);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let query = concepts::query("anything", scope.project_ids.clone(), false, 10);

        runtime.serve(&backend, &tx, &envelope(1, StoreRequest::SearchConcepts(query)));
        runtime.serve(
            &backend,
            &tx,
            &envelope(2, StoreRequest::IndexConcepts { scope }),
        );
        runtime.shutdown();
        assert_eq!(runtime.settle(Duration::from_millis(1)).await, 0, "nothing left");
        tokio::time::sleep(Duration::from_secs(7200)).await;
        assert!(rx.try_recv().is_err(), "no aborted task answered");
    }
}
