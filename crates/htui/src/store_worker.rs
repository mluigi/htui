//! The only task that owns a [`Backend`] (plan D4).
//!
//! The UI side holds two unbounded channels and nothing else: `R-NF-3` ("no store handle on the
//! render side") is a fact about this module's ownership, not a convention. [`serve`] is the pure
//! request -> reply function; [`spawn`] is a loop around it, and the test harness calls it inline
//! so snapshots need no sleeps (blueprint C.1, C.8).

use htui_core::model::{
    BoxInfo, DocumentHead, Item, ItemFilter, ItemId, ItemSummary, LinkGraph, Note, RunSummary,
    Scope, WorkspaceSummary,
};
use htui_core::store::{Backend, ReadStore};
use tokio::sync::mpsc;

use crate::ui::overlay::OverlayId;
use crate::ui::tabs::TabId;

/// Monotonic request counter, minted by `App::dispatch` (blueprint C.2).
pub type Seq = u64;

/// Who asked, and therefore who the reply is addressed to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The shell itself: top-bar reads and the startup workspace list.
    App,
    /// A registered tab.
    Tab(TabId),
    /// An open overlay. A reply to a popped overlay is dropped.
    Overlay(OverlayId),
}

/// Everything the UI can ask the store for in MOD-1.
///
/// MOD-2 adds `StepEvents(StepId)` here and one arm in [`serve`]; the event loop does not change.
#[derive(Debug, Clone)]
pub enum StoreRequest {
    /// Every workspace with its projects (switcher, startup scope).
    Workspaces,
    /// This box's row for the top bar. No probe: that is MOD-7.
    BoxInfo,
    /// How many runs of the scope are active (top bar).
    ActiveRuns {
        /// The workspace scope to count in.
        scope: Scope,
    },
    /// The Backlog list.
    Items {
        /// The workspace scope to read.
        scope: Scope,
        /// Conjunctive filter; `ItemFilter::default()` means "everything in scope".
        filter: ItemFilter,
    },
    /// One item with its body.
    Item(ItemId),
    /// The link neighbourhood of an item.
    Links {
        /// Root of the traversal.
        id: ItemId,
        /// How many hops to follow.
        hops: u8,
    },
    /// The item's documents without bodies.
    Documents(ItemId),
    /// The item's notes.
    Notes(ItemId),
    /// The item's runs with their steps.
    Runs(ItemId),
}

impl StoreRequest {
    /// Stable name of the request, used in [`StoreReply::Failed`] and in logs.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Workspaces => "workspaces",
            Self::BoxInfo => "box_info",
            Self::ActiveRuns { .. } => "active_runs",
            Self::Items { .. } => "items",
            Self::Item(_) => "item",
            Self::Links { .. } => "links",
            Self::Documents(_) => "documents",
            Self::Notes(_) => "notes",
            Self::Runs(_) => "runs",
        }
    }
}

/// The answer to exactly one [`StoreRequest`].
#[derive(Debug, Clone)]
pub enum StoreReply {
    /// Answer to [`StoreRequest::Workspaces`], ordered by name.
    Workspaces(Vec<WorkspaceSummary>),
    /// Answer to [`StoreRequest::BoxInfo`]; `None` when no box row is registered.
    BoxInfo(Option<BoxInfo>),
    /// Answer to [`StoreRequest::ActiveRuns`].
    ActiveRuns(usize),
    /// Answer to [`StoreRequest::Items`].
    Items(Vec<ItemSummary>),
    /// Answer to [`StoreRequest::Item`]; boxed because `Item` dwarfs every other variant.
    Item(Box<Option<Item>>),
    /// Answer to [`StoreRequest::Links`].
    Links(LinkGraph),
    /// Answer to [`StoreRequest::Documents`].
    Documents(Vec<DocumentHead>),
    /// Answer to [`StoreRequest::Notes`].
    Notes(Vec<Note>),
    /// Answer to [`StoreRequest::Runs`].
    Runs(Vec<RunSummary>),
    /// The store failed. `request` is [`StoreRequest::name`].
    Failed {
        /// Which request failed.
        request: &'static str,
        /// The `StoreError`, rendered through `Display`.
        message: String,
    },
}

/// A request on its way to the worker.
#[derive(Debug)]
pub struct RequestEnvelope {
    /// Staleness stamp (blueprint C.2).
    pub seq: Seq,
    /// Who asked.
    pub origin: Origin,
    /// What was asked.
    pub request: StoreRequest,
}

/// A reply on its way back to the UI.
#[derive(Debug, Clone)]
pub struct ReplyEnvelope {
    /// The `seq` of the request this answers.
    pub seq: Seq,
    /// Who asked, and therefore who receives it.
    pub origin: Origin,
    /// The answer.
    pub reply: StoreReply,
}

/// Serves one request. Pure: no channels, no state, no logging.
///
/// A `StoreError` becomes [`StoreReply::Failed`] rather than a panic or a dropped reply, so the
/// asking view always hears back exactly once.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> StoreReply {
    let name = request.name();
    match request {
        StoreRequest::Workspaces => match backend.workspaces().await {
            Ok(rows) => StoreReply::Workspaces(rows),
            Err(err) => failed(name, &err),
        },
        StoreRequest::BoxInfo => match backend.box_info().await {
            Ok(info) => StoreReply::BoxInfo(info),
            Err(err) => failed(name, &err),
        },
        StoreRequest::ActiveRuns { scope } => match backend.active_runs(scope).await {
            Ok(count) => StoreReply::ActiveRuns(count),
            Err(err) => failed(name, &err),
        },
        StoreRequest::Items { scope, filter } => match backend.items(scope, filter).await {
            Ok(rows) => StoreReply::Items(rows),
            Err(err) => failed(name, &err),
        },
        StoreRequest::Item(id) => match backend.item(*id).await {
            Ok(item) => StoreReply::Item(Box::new(item)),
            Err(err) => failed(name, &err),
        },
        StoreRequest::Links { id, hops } => match backend.links(*id, *hops).await {
            Ok(graph) => StoreReply::Links(graph),
            Err(err) => failed(name, &err),
        },
        StoreRequest::Documents(id) => match backend.documents(*id).await {
            Ok(rows) => StoreReply::Documents(rows),
            Err(err) => failed(name, &err),
        },
        StoreRequest::Notes(id) => match backend.notes(*id).await {
            Ok(rows) => StoreReply::Notes(rows),
            Err(err) => failed(name, &err),
        },
        StoreRequest::Runs(id) => match backend.runs(*id).await {
            Ok(rows) => StoreReply::Runs(rows),
            Err(err) => failed(name, &err),
        },
    }
}

/// Renders a store error into the reply the asking view receives.
fn failed(request: &'static str, err: &htui_core::store::StoreError) -> StoreReply {
    StoreReply::Failed {
        request,
        message: err.to_string(),
    }
}

/// Spawns the worker. The `Backend` moves in and no other task can reach it afterwards (D4).
pub fn spawn(
    backend: Backend,
    mut rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(envelope) = rx.recv().await {
            let reply = serve(&backend, &envelope.request).await;
            let answer = ReplyEnvelope {
                seq: envelope.seq,
                origin: envelope.origin,
                reply,
            };
            if tx.send(answer).is_err() {
                // The UI is gone; there is nobody left to answer.
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::fixtures::ids;
    use htui_core::store::MemStore;

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    async fn platform_scope(backend: &Backend) -> Scope {
        let StoreReply::Workspaces(workspaces) = serve(backend, &StoreRequest::Workspaces).await
        else {
            panic!("workspaces answered with the wrong variant")
        };
        let platform = workspaces
            .iter()
            .find(|w| w.slug == "platform")
            .expect("the demo fixture holds the `platform` workspace");
        Scope::from_workspace(platform)
    }

    #[tokio::test]
    async fn serve_workspaces_lists_the_demo_hierarchy() {
        let backend = demo();
        let StoreReply::Workspaces(workspaces) = serve(&backend, &StoreRequest::Workspaces).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].name, "Graphics");
        assert_eq!(workspaces[1].name, "Platform");
        assert_eq!(workspaces[1].projects.len(), 2);
    }

    #[tokio::test]
    async fn serve_box_info_answers_this_box() {
        let backend = demo();
        let StoreReply::BoxInfo(info) = serve(&backend, &StoreRequest::BoxInfo).await else {
            panic!("wrong reply variant")
        };
        assert_eq!(
            info.expect("the demo fixture registers this box").hostname,
            "DESKTOP-HTUI"
        );
    }

    #[tokio::test]
    async fn serve_active_runs_counts_the_queued_run() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let StoreReply::ActiveRuns(count) =
            serve(&backend, &StoreRequest::ActiveRuns { scope }).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(count, 1, "RUN_2 is the fixture's only active run");
    }

    #[tokio::test]
    async fn serve_items_returns_the_scope_in_store_order() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let StoreReply::Items(items) = serve(
            &backend,
            &StoreRequest::Items {
                scope,
                filter: ItemFilter::default(),
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(items.len(), 11, "eight htui items plus three agy items");
        assert_eq!(items[0].key, "ANA-1");
    }

    #[tokio::test]
    async fn serve_item_returns_the_body() {
        let backend = demo();
        let StoreReply::Item(item) = serve(&backend, &StoreRequest::Item(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        let item = item.expect("FEAT-1 is in the fixture");
        assert_eq!(item.key, "FEAT-1");
        assert!(!item.body.is_empty());
    }

    #[tokio::test]
    async fn serve_links_walks_one_hop() {
        let backend = demo();
        let StoreReply::Links(graph) = serve(
            &backend,
            &StoreRequest::Links {
                id: ids::HTUI_FEAT_2,
                hops: 1,
            },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(graph.root, ids::HTUI_FEAT_2);
        assert_eq!(
            graph.nodes.len(),
            3,
            "FEAT-2, the FEAT-1 it is blocked by, and the cross-project agy FEAT-1"
        );
    }

    #[tokio::test]
    async fn serve_documents_notes_and_runs_answer_their_variants() {
        let backend = demo();
        let StoreReply::Documents(docs) =
            serve(&backend, &StoreRequest::Documents(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(docs.len(), 3, "prd v1 plus plan v1 and v2");

        let StoreReply::Notes(notes) =
            serve(&backend, &StoreRequest::Notes(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(notes.len(), 2);

        let StoreReply::Runs(runs) = serve(&backend, &StoreRequest::Runs(ids::HTUI_FEAT_1)).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].steps.len(), 4);
    }

    #[tokio::test]
    async fn serve_reports_an_empty_store_without_failing() {
        let backend = Backend::memory(MemStore::new());
        let StoreReply::Workspaces(workspaces) = serve(&backend, &StoreRequest::Workspaces).await
        else {
            panic!("wrong reply variant")
        };
        assert!(workspaces.is_empty());

        let StoreReply::BoxInfo(info) = serve(&backend, &StoreRequest::BoxInfo).await else {
            panic!("wrong reply variant")
        };
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn spawn_answers_with_the_request_seq_and_origin() {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let worker = spawn(demo(), req_rx, rep_tx);
        req_tx
            .send(RequestEnvelope {
                seq: 7,
                origin: Origin::App,
                request: StoreRequest::Workspaces,
            })
            .expect("the worker is alive");
        let envelope = rep_rx.recv().await.expect("the worker answers");
        assert_eq!(envelope.seq, 7);
        assert_eq!(envelope.origin, Origin::App);
        assert!(matches!(envelope.reply, StoreReply::Workspaces(_)));
        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
    }
}
