//! The only task that owns a [`Backend`] (plan D4).
//!
//! The UI side holds two unbounded channels and nothing else: `R-NF-3` ("no store handle on the
//! render side") is a fact about this module's ownership, not a convention. [`serve`] is the pure
//! request -> reply function; [`spawn`] is a loop around it, and the test harness calls it inline
//! so snapshots need no sleeps (blueprint C.1, C.8).
//!
//! **The channels stay unbounded** (MOD-1 D4). A bounded pair would make `App::dispatch` either
//! `.await` on a full queue - on the render task, which `R-NF-3` forbids - or drop requests, and
//! the reply half would let a slow UI stall the one task that owns the store. Backpressure is
//! deferred until `PgStore` read latency has actually been measured against a populated database;
//! until then the queue depth is bounded in practice by the keystrokes a user can produce.

use htui_core::model::{
    BoxInfo, DocumentHead, Item, ItemFilter, ItemId, ItemSummary, LinkGraph, Note, ProjectId,
    RunSummary, Scope, WorkspaceSummary,
};
use htui_core::store::{ReadStore, Result as StoreResult, StoreError};
use htui_store::cache::refresh::{RefreshSettings, Refresher};
use htui_store::{Backend, ConnEvent, PgStore, Started, connect};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

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
    /// The top bar's store field and the pending-migration count (plan D11).
    StoreState,
    /// Apply the pending migrations (`R-STO-5`), after the user answered `y`.
    ApplyMigrations,
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
            Self::StoreState => "store_state",
            Self::ApplyMigrations => "apply_migrations",
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
    /// Answer to [`StoreRequest::StoreState`].
    StoreState {
        /// `Backend::label()`: `memory`, `connecting`, `online` or `offline · <age>`.
        label: String,
        /// How many migrations are pending; `None` when the question does not apply — a memory
        /// backend, an offline one, or a connected one whose schema is up to date.
        migrations_pending: Option<usize>,
    },
    /// Answer to [`StoreRequest::ApplyMigrations`].
    MigrationsApplied {
        /// How many were pending before the run; `0` when there was nothing to do.
        applied: usize,
    },
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
///
/// The two connection-aware requests are answered here as far as a `&Backend` can answer them —
/// [`StoreRequest::StoreState`] without a pending count, [`StoreRequest::ApplyMigrations`] with
/// nothing to apply — because the pending count and the store it belongs to are state of the
/// [`spawn`] loop, which intercepts both before reaching here. This is the answer the test harness
/// and a `--demo` shell get, and both are right: a `MemStore` has no schema to migrate.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> StoreReply {
    match try_serve(backend, request).await {
        Ok(reply) => reply,
        Err(err) => failed(request.name(), &err),
    }
}

/// [`serve`] with the `StoreError` still visible, for the one caller that has to act on it.
///
/// [`spawn`] needs to tell [`StoreError::Unreachable`] from every other failure so it can drop an
/// `Online` backend onto the mirror, and [`StoreReply::Failed`] carries only rendered text - which
/// is what the asking view shows. Widening the reply with a machine-readable field would put a
/// store concept into every view for the benefit of one match in this file.
async fn try_serve(backend: &Backend, request: &StoreRequest) -> StoreResult<StoreReply> {
    Ok(match request {
        StoreRequest::Workspaces => StoreReply::Workspaces(backend.workspaces().await?),
        StoreRequest::BoxInfo => StoreReply::BoxInfo(backend.box_info().await?),
        StoreRequest::ActiveRuns { scope } => {
            StoreReply::ActiveRuns(backend.active_runs(scope).await?)
        }
        StoreRequest::Items { scope, filter } => {
            StoreReply::Items(backend.items(scope, filter).await?)
        }
        StoreRequest::Item(id) => StoreReply::Item(Box::new(backend.item(*id).await?)),
        StoreRequest::Links { id, hops } => StoreReply::Links(backend.links(*id, *hops).await?),
        StoreRequest::Documents(id) => StoreReply::Documents(backend.documents(*id).await?),
        StoreRequest::Notes(id) => StoreReply::Notes(backend.notes(*id).await?),
        StoreRequest::Runs(id) => StoreReply::Runs(backend.runs(*id).await?),
        StoreRequest::StoreState => StoreReply::StoreState {
            label: backend.label(),
            migrations_pending: None,
        },
        StoreRequest::ApplyMigrations => StoreReply::MigrationsApplied { applied: 0 },
    })
}

/// Renders a store error into the reply the asking view receives.
fn failed(request: &'static str, err: &StoreError) -> StoreReply {
    StoreReply::Failed {
        request,
        message: err.to_string(),
    }
}

/// Spawns the worker. The `Backend` moves in and no other task can reach it afterwards (plan D4).
///
/// The loop is a `select!` over three sources — the UI's requests, the connect task's
/// [`ConnEvent`]s and a reconnect ticker — and it owns every backend swap:
///
/// - [`ConnEvent::Online`] replaces `Offline { cache, .. }` with `Online { pg, cache }` (the same
///   mirror, not a re-opened one) and starts the [`Refresher`].
/// - [`ConnEvent::MigrationsPending`] holds the store **aside** instead of swapping: the schema is
///   not one this binary has finished writing, so nothing reads through it and no refresher fills
///   the mirror from it. The count is reported in the next [`StoreReply::StoreState`], which is
///   what opens the migration prompt, and [`StoreRequest::ApplyMigrations`] is what makes the swap
///   happen. (Blueprint D.2 swaps here and applies through `Backend::writable()`;
///   `PgStore::apply_migrations` takes `&mut self`, which a `&PgStore` cannot give — same
///   components, correct ownership.)
/// - [`ConnEvent::Failed`] turns the first `connecting` into `offline · 0s` and nothing else.
/// - An `Online` backend that stops answering goes the other way: a [`StoreError::Unreachable`]
///   from a served request, or the same from the [`Refresher`]'s last pass, swaps `Online` back
///   for `Offline { since: Some(now) }` over the same mirror, aborts the refresher and lets the
///   ticker start dialling again. The request that noticed still gets its
///   [`StoreReply::Failed`]: exactly one reply per request, whatever the backend did.
/// - The ticker re-dials every [`connect::RECONNECT`], and only while there is a DSN to dial, no
///   writable backend and no store waiting for an answer to the migration prompt. Its missed-tick
///   behaviour is `Delay`, not the default `Burst`: a dial that overran its slot - a ten-second
///   acquire timeout inside a thirty-second interval, or a laptop that was suspended - must not
///   be followed by a queue of catch-up dials fired back to back.
///
/// `started` is the bundle [`connect::start`] hands back; `Started::detached` is the `--demo` and
/// test form, whose event channel is never written and whose ticker is disarmed, so the worker
/// behaves exactly as it did in MOD-1 (deviation from blueprint D.2's eight-argument `spawn`: the
/// pieces are the same, passed as the struct that already carries them).
pub fn spawn(
    started: Started,
    mut rx: mpsc::UnboundedReceiver<RequestEnvelope>,
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
) -> tokio::task::JoinHandle<()> {
    let Started {
        mut backend,
        mut events,
        events_tx,
        projects,
        settings,
        reconnect,
    } = started;

    tokio::spawn(async move {
        // How many migrations are waiting, and the store that is waiting to apply them.
        let mut pending: Option<usize> = None;
        let mut held: Option<PgStore> = None;
        let mut refresher: Option<Refresher> = None;
        // The refresher's last pass outcome, for as long as there is a refresher.
        let mut health: Option<watch::Receiver<Option<StoreError>>> = None;
        // `interval_at`, not `interval`: the first tick of an `interval` completes immediately,
        // which would re-dial in the same breath as `start`'s own attempt.
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + connect::RECONNECT,
            connect::RECONNECT,
        );
        // A dial that overran its slot must not then be followed by a burst of catch-up dials.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                envelope = rx.recv() => {
                    // The UI is gone; there is nobody left to answer.
                    let Some(envelope) = envelope else { break };

                    // Publish the scope so the refresher follows what the user is looking at
                    // (plan D9). On every `Items` request, not only on a change: the send is a
                    // pointer swap and a missed update mirrors the wrong project.
                    if let StoreRequest::Items { scope, .. } = &envelope.request {
                        projects.send_replace(scope.project_ids.clone());
                    }

                    let reply = match &envelope.request {
                        StoreRequest::StoreState => StoreReply::StoreState {
                            label: backend.label(),
                            migrations_pending: pending,
                        },
                        StoreRequest::ApplyMigrations if held.is_some() => {
                            // `if held.is_some()` above, so this cannot be the `None` arm; the
                            // store has to be moved out to be applied (`&mut self`).
                            match held.take() {
                                Some(mut pg) => match pg.apply_migrations().await {
                                    Ok(()) => {
                                        let applied = pending.take().unwrap_or(0);
                                        tracing::info!(applied, "schema migrations applied");
                                        go_online(
                                            &mut backend, pg, &mut refresher, &mut health,
                                            &projects, settings,
                                        ).await;
                                        StoreReply::MigrationsApplied { applied }
                                    }
                                    Err(err) => {
                                        // Still pending, still held: `y` can be answered again.
                                        held = Some(pg);
                                        failed("apply_migrations", &err)
                                    }
                                },
                                None => StoreReply::MigrationsApplied { applied: 0 },
                            }
                        }
                        other => match try_serve(&backend, other).await {
                            Ok(reply) => reply,
                            Err(err) => {
                                // This read is what noticed the server had gone. The asking view
                                // still hears back exactly once; the next read finds the mirror.
                                if matches!(err, StoreError::Unreachable(_)) {
                                    go_offline(&mut backend, &mut refresher, &mut health, &err);
                                }
                                failed(other.name(), &err)
                            }
                        },
                    };

                    let answer = ReplyEnvelope { seq: envelope.seq, origin: envelope.origin, reply };
                    if tx.send(answer).is_err() {
                        break;
                    }
                }

                Some(event) = events.recv() => match event {
                    ConnEvent::Online(pg) => {
                        pending = None;
                        held = None;
                        go_online(
                            &mut backend, pg, &mut refresher, &mut health, &projects, settings,
                        )
                        .await;
                        tracing::info!(label = backend.label(), "store online");
                    }
                    ConnEvent::MigrationsPending(pg, n) => {
                        pending = Some(n);
                        held = Some(pg);
                        // The mirror stays the read path and the top bar stops saying
                        // `connecting`: nothing reads through a schema this binary refuses to
                        // use until the user has answered the prompt.
                        backend.gave_up();
                        tracing::warn!(pending = n, "schema has pending migrations");
                    }
                    ConnEvent::Failed(why) => {
                        backend.gave_up();
                        tracing::warn!(%why, "connect failed");
                    }
                },

                err = lost_the_server(health.clone()) => {
                    // The refresher passes every `interval`, so it usually notices first.
                    go_offline(&mut backend, &mut refresher, &mut health, &err);
                }

                _ = ticker.tick(),
                    if reconnect.is_some() && !backend.is_writable() && held.is_none() =>
                {
                    if let Some(dial) = reconnect.clone() {
                        let sender = events_tx.clone();
                        tokio::spawn(async move {
                            let _ = sender.send(dial().await).await;
                        });
                    }
                }
            }
        }

        if let Some(refresher) = refresher {
            refresher.abort();
        }
    })
}

/// Swaps `Offline { cache, .. }` for `Online { pg, cache }` and (re)starts the refresher.
///
/// The mirror is moved across rather than re-opened: it is the file the shell has been reading
/// from since startup, and `CacheStore` is a handle on one pool.
async fn go_online(
    backend: &mut Backend,
    pg: PgStore,
    refresher: &mut Option<Refresher>,
    health: &mut Option<watch::Receiver<Option<StoreError>>>,
    projects: &watch::Sender<Vec<ProjectId>>,
    base: RefreshSettings,
) {
    let Some(cache) = backend.cache().cloned() else {
        tracing::error!("no mirror to go online over; keeping the current backend");
        return;
    };
    // Read before the move: `this_box`, `this_user` and the two cache settings are what only a
    // connected server knows (blueprint C.13's `RefreshSettings`).
    let settings = connect::refresh_settings(&pg, base).await;
    *backend = Backend::Online { pg, cache };
    if let Some(previous) = refresher.take() {
        previous.abort();
    }
    *refresher = spawn_refresher(backend, projects, settings);
    *health = refresher.as_ref().map(Refresher::health);
}

/// The reverse of [`go_online`]: an `Online` backend that lost its server falls back on the mirror.
///
/// Aborts the refresher - there is nothing left to mirror *from*, and its failing passes would
/// otherwise report the same loss every interval - and drops the health watch, which disarms the
/// `select!` arm that reads it. The reconnect ticker re-arms itself, its guard being
/// `!backend.is_writable()`.
///
/// A no-op on every backend that is not `Online`, so a second notice - a read and the refresher
/// racing to report the same drop - does not restart the offline age.
fn go_offline(
    backend: &mut Backend,
    refresher: &mut Option<Refresher>,
    health: &mut Option<watch::Receiver<Option<StoreError>>>,
    why: &StoreError,
) {
    if !backend.went_offline() {
        return;
    }
    if let Some(previous) = refresher.take() {
        previous.abort();
    }
    *health = None;
    tracing::warn!(%why, "store unreachable; falling back to the mirror");
}

/// Resolves with the error when the refresher reports an unreachable server, and never otherwise.
///
/// Takes the receiver by value - `watch::Receiver` is `Clone` and a clone keeps the seen version -
/// so the `select!` arm's body is free to reassign the worker's own `health`.
///
/// Parks forever when there is no refresher and when its sender is gone, which leaves the arm
/// disabled rather than spinning on a closed channel. A pass that succeeded, or that failed for
/// any other reason, is not a signal: the loop keeps waiting.
async fn lost_the_server(health: Option<watch::Receiver<Option<StoreError>>>) -> StoreError {
    let Some(mut passes) = health else {
        return std::future::pending().await;
    };
    loop {
        if passes.changed().await.is_err() {
            return std::future::pending().await;
        }
        let lost = match &*passes.borrow() {
            Some(StoreError::Unreachable(why)) => Some(why.clone()),
            _ => None,
        };
        if let Some(why) = lost {
            return StoreError::Unreachable(why);
        }
    }
}

/// The refresher for an online backend, or `None` for any other (blueprint D.2).
fn spawn_refresher(
    backend: &Backend,
    projects: &watch::Sender<Vec<ProjectId>>,
    settings: RefreshSettings,
) -> Option<Refresher> {
    let pg = backend.writable()?;
    let cache = backend.cache()?;
    Some(Refresher::spawn(
        pg.pool().clone(),
        cache.clone(),
        projects.subscribe(),
        settings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::fixtures::ids;
    use htui_core::model::BoxId;
    use htui_core::store::MemStore;
    use htui_store::{CacheStore, Identity};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// A worker over a backend that never connects, plus the two channel ends the UI holds.
    fn detached(
        backend: Backend,
    ) -> (
        mpsc::UnboundedSender<RequestEnvelope>,
        mpsc::UnboundedReceiver<ReplyEnvelope>,
        tokio::task::JoinHandle<()>,
    ) {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, rep_rx) = mpsc::unbounded_channel();
        let worker = spawn(Started::detached(backend), req_rx, rep_tx);
        (req_tx, rep_rx, worker)
    }

    /// One request through a spawned worker, and its reply.
    async fn round_trip(
        tx: &mpsc::UnboundedSender<RequestEnvelope>,
        rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
        request: StoreRequest,
    ) -> StoreReply {
        tx.send(RequestEnvelope {
            seq: 0,
            origin: Origin::App,
            request,
        })
        .expect("the worker is alive");
        rx.recv().await.expect("the worker answers").reply
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
        let worker = spawn(Started::detached(demo()), req_rx, rep_tx);
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

    #[tokio::test]
    async fn store_state_reports_the_backend_label_and_no_pending_count() {
        let (tx, mut rx, worker) = detached(demo());
        let StoreReply::StoreState {
            label,
            migrations_pending,
        } = round_trip(&tx, &mut rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "memory");
        assert_eq!(
            migrations_pending, None,
            "a memory backend has no schema to migrate"
        );
        drop(tx);
        worker.await.expect("the worker stops with its channel");
    }

    #[tokio::test]
    async fn apply_migrations_without_a_held_store_applies_nothing() {
        let (tx, mut rx, worker) = detached(demo());
        let reply = round_trip(&tx, &mut rx, StoreRequest::ApplyMigrations).await;
        assert!(
            matches!(reply, StoreReply::MigrationsApplied { applied: 0 }),
            "nothing was pending, so nothing was applied: {reply:?}"
        );
        drop(tx);
        worker.await.expect("the worker stops with its channel");
    }

    #[tokio::test]
    async fn every_items_request_publishes_its_scope_to_the_refresher() {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(demo());
        // The receiver the refresher would hold; `Refresher::spawn` calls `subscribe` the same way.
        let scope_rx = started.projects.subscribe();
        started.settings.interval = std::time::Duration::from_secs(3600);
        let worker = spawn(started, req_rx, rep_tx);

        assert!(
            scope_rx.borrow().is_empty(),
            "no scope before the first read"
        );

        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
        };
        let reply = round_trip(
            &req_tx,
            &mut rep_rx,
            StoreRequest::Items {
                scope: scope.clone(),
                filter: ItemFilter::default(),
            },
        )
        .await;
        assert!(matches!(reply, StoreReply::Items(_)));
        assert_eq!(
            *scope_rx.borrow(),
            scope.project_ids,
            "the refresher follows what the user is looking at (plan D9)"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
    }

    #[tokio::test]
    async fn a_failed_connect_event_turns_connecting_into_an_age() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-test", 1)
            .await
            .expect("open a throwaway mirror");

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(Backend::Offline {
            cache: cache.clone(),
            since: None,
        });
        let events_tx = started.events_tx.clone();
        // A DSN would arm the ticker; `detached` leaves it disarmed, which is what keeps this test
        // from dialling anything.
        started.reconnect = None;
        let worker = spawn(started, req_rx, rep_tx);

        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "connecting", "no attempt has answered yet");

        events_tx
            .send(ConnEvent::Failed("no server".to_owned()))
            .await
            .expect("the worker is alive");

        // The request and the event are both ready; `select!` picks at random, so ask until the
        // event has been taken rather than assuming the first round trip loses the race.
        let mut label = String::new();
        for _ in 0..32 {
            let StoreReply::StoreState { label: seen, .. } =
                round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
            else {
                panic!("wrong reply variant")
            };
            label = seen;
            if label != "connecting" {
                break;
            }
        }
        assert!(
            label.starts_with("offline · "),
            "a failed attempt gives up on `connecting`: {label}"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
    }

    /// The mid-session `Online` → `Offline` transition, driven by an injected unreachable store.
    ///
    /// `PgStore::lazy` opens no socket, so the *first* read is what dials - at a DSN nothing
    /// listens on, which is a `StoreError::Unreachable` and therefore exactly the outcome a
    /// server that went away mid-session produces. Restarting Postgres under a live pool is not
    /// something a unit test can do; this drives the same code path without one.
    #[tokio::test]
    async fn an_unreachable_read_drops_an_online_backend_onto_the_mirror() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "worker-swap", 1)
            .await
            .expect("open a throwaway mirror");
        let identity = Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = PgStore::lazy(
            "postgres://nobody:nothing@127.0.0.1:1/none",
            &identity,
            std::time::Duration::from_millis(250),
        )
        .expect("a lazy pool opens no socket");

        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (rep_tx, mut rep_rx) = mpsc::unbounded_channel();
        let mut started = Started::detached(Backend::Online {
            pg,
            cache: cache.clone(),
        });
        // `detached` leaves the ticker disarmed, so nothing re-dials behind the assertions.
        started.reconnect = None;
        let worker = spawn(started, req_rx, rep_tx);

        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(label, "online", "nothing has asked the server yet");

        // The read that notices. It still gets its one reply.
        let reply = round_trip(&req_tx, &mut rep_rx, StoreRequest::Workspaces).await;
        assert!(
            matches!(
                &reply,
                StoreReply::Failed {
                    request: "workspaces",
                    ..
                }
            ),
            "the asking view hears back exactly once, even as the backend swaps: {reply:?}"
        );

        let StoreReply::StoreState { label, .. } =
            round_trip(&req_tx, &mut rep_rx, StoreRequest::StoreState).await
        else {
            panic!("wrong reply variant")
        };
        assert!(
            label.starts_with("offline · "),
            "an unreachable read drops the backend onto the mirror: {label}"
        );

        // And the next read answers from the mirror instead of failing again.
        let reply = round_trip(&req_tx, &mut rep_rx, StoreRequest::Workspaces).await;
        assert!(
            matches!(&reply, StoreReply::Workspaces(rows) if rows.is_empty()),
            "an unfilled mirror is empty, not broken: {reply:?}"
        );

        drop(req_tx);
        worker.await.expect("the worker stops with its channel");
        cache.close().await;
    }

    /// A failure that is *not* a lost connection leaves an online backend alone.
    #[tokio::test]
    async fn lost_the_server_ignores_a_pass_that_failed_for_another_reason() {
        let (health, watcher) = watch::channel(None);
        let mut backend = demo();
        let mut refresher = None;
        let mut seen = Some(watcher.clone());

        health.send_replace(Some(StoreError::Backend("a bad query".to_owned())));
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                lost_the_server(Some(watcher.clone())),
            )
            .await
            .is_err(),
            "only StoreError::Unreachable is the signal"
        );

        health.send_replace(Some(StoreError::Unreachable("gone".to_owned())));
        let err = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            lost_the_server(Some(watcher)),
        )
        .await
        .expect("an unreachable pass resolves it");
        assert!(matches!(err, StoreError::Unreachable(_)));

        // A memory backend has no mirror, so the swap is refused and the watch is left alone.
        go_offline(&mut backend, &mut refresher, &mut seen, &err);
        assert_eq!(backend.label(), "memory");
        assert!(
            seen.is_some(),
            "nothing was swapped, so nothing was dropped"
        );
    }
}
