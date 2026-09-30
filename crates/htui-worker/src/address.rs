//! Addressing without tabs (MOD-41 plan D7): where the runtime's answers and frames go, what it
//! serves and answers, and who is subscribed to which item's runs.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use htui_core::model::{ItemId, ProjectId, RunId, StepId};
use htui_orch::Opening;

use crate::runtime::Tag;
use crate::views::{FrameKind, ItemActions, OrchReply, OrchRequest, RunFrame};

/// Where the run runtime's answers and frames go (MOD-41 plan D7). The TUI's is its
/// `TuiReplies`, over the store loop's reply channel; the worker's is [`Unaddressed`].
pub trait ReplySink: Clone + Send + Sync + 'static {
    /// One request's address.
    type Addr: Clone + Send + Sync + core::fmt::Debug + 'static;
    /// Who a subscription belongs to: a later subscription of the same subscriber replaces it.
    type Subscriber: Clone + Eq + core::hash::Hash + Send + Sync + core::fmt::Debug + 'static;
    /// The subscriber `addr` belongs to.
    fn subscriber(addr: &Self::Addr) -> Self::Subscriber;
    /// One reply to `to`. Never fails: a gone receiver drops it, as a closed reply channel always
    /// has.
    fn send(&self, to: &Self::Addr, reply: RunReply);
}

/// One request the run runtime serves (MOD-41 plan D7): the payload of
/// `StoreRequest::{Orch, RunStream, RunActions}`, without the TUI's envelope.
#[derive(Debug, Clone)]
pub enum RunRequest {
    /// A command or an orchestrator read.
    Orch(OrchRequest),
    /// Follow an item's runs.
    Stream {
        /// The item.
        item: ItemId,
    },
    /// Every verdict for an item.
    Actions(ItemId),
}

impl RunRequest {
    /// Exactly the TUI's `StoreRequest::name` for the same request: the
    /// [`ORCH_NAMES`](crate::ORCH_NAMES) entry, `run_stream` or `run_actions`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Orch(request) => request.name(),
            Self::Stream { .. } => "run_stream",
            Self::Actions(_) => "run_actions",
        }
    }
}

/// One answer of the run runtime (MOD-41 plan D7), one-to-one with
/// `StoreReply::{Orch, RunStream, RunActions, Failed}`.
#[derive(Debug, Clone)]
pub enum RunReply {
    /// A command's or read's outcome.
    Orch(OrchReply),
    /// A frame of a subscribed item.
    Frame(RunFrame),
    /// Every verdict for an item.
    Actions(Box<ItemActions>),
    /// The request failed with the sentence.
    Failed {
        /// The request's name.
        request: &'static str,
        /// Why.
        message: String,
    },
}

/// The worker's sink: it serves no request, so nothing is addressed (MOD-41 plan D7). An
/// `Attach` cannot be built for it, since its address type has no value.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unaddressed;

impl ReplySink for Unaddressed {
    type Addr = core::convert::Infallible;
    type Subscriber = core::convert::Infallible;

    fn subscriber(addr: &Self::Addr) -> Self::Subscriber {
        match *addr {}
    }

    fn send(&self, to: &Self::Addr, _reply: RunReply) {
        match *to {}
    }
}

/// What [`RunRuntime::serve_request`](crate::RunRuntime::serve_request) (and the runtime's event
/// channel) decided about one request.
///
/// MOD-41 plan D7: `A` is the sink's address ([`ReplySink::Addr`]); the TUI's is its `ReplyAddr`.
#[derive(Debug)]
pub enum RunServed<A> {
    /// Answer with this reply, now.
    Reply(RunReply),
    /// A task this runtime owns answers the request, exactly once.
    Deferred,
    /// D165/D181: a promotion's engine writes are done; the loop hands `promoted` to
    /// `AgentRuntime::attach_promoted` (T7) and answers at `addr`.
    Attach {
        /// The promotion request's address.
        addr: A,
        /// What the chat binds to.
        promoted: Box<Promoted>,
        /// What the chat's end publishes: the loop wraps the session task in it (D212).
        ended: ChatEnd,
    },
}

/// Blueprint D212: what a promoted chat's end publishes for its item.
///
/// The Runs pane greys the run's verbs while a step of it is chatted with, refuses a greyed key
/// itself, and reads its verdicts again only on a frame of its item. Ending a chat writes nothing
/// the walk publishes, so without a frame of its own the verbs would stay greyed after `Esc Esc`.
/// The session task is wrapped in [`after`](Self::after), which publishes `Changed` for the item
/// and run once the task has returned.
#[derive(Debug, Clone)]
pub struct ChatEnd {
    pub(crate) publisher: Arc<dyn Publish>,
    pub(crate) tag: Arc<Tag>,
}

impl ChatEnd {
    /// `task`, then a `Changed` frame for the promoted step's item. The session's command
    /// receiver dies with `task`, so by the time the frame is out
    /// `AgentRuntime::live_steps` no longer names the step and the verdicts the pane reads next
    /// are the chat-free ones.
    ///
    /// MOD-41 plan D7: any task, so the runtime names no chat type of the TUI's. The future owns
    /// `self` and `task`, so it is `'static`, and `Send` whenever `task` is.
    pub async fn after<F>(self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        task.await;
        if let Some(item) = self.tag.item.get() {
            self.publisher.publish(&RunFrame {
                item: *item,
                run: self.tag.run.get().copied(),
                kind: FrameKind::Changed,
            });
        }
    }
}

/// A promoted step, as the Chat tab's runtime binds to it (D165, D191).
#[derive(Debug, Clone)]
pub struct Promoted {
    /// The step's run.
    pub run: RunId,
    /// The promoted step.
    pub step: StepId,
    /// `run.project_id`.
    pub project: ProjectId,
    /// How its chat opens.
    pub opening: Opening,
}

/// Plan D172, blueprint §0a point 3: who is subscribed to which item's runs, and at which address.
///
/// One subscription per subscriber ([`ReplySink::Subscriber`], the TUI's origin): a later
/// `RunStream` from the same subscriber replaces the earlier one, whose `seq` `App::latest` already
/// treats as stale. Every frame for `item` goes to each subscriber of it at **its** subscription's
/// address, the only one `App::is_fresh` passes.
pub(crate) struct Publisher<P: ReplySink>(Arc<StdMutex<Subscribers<P>>>);

struct Subscribers<P: ReplySink> {
    subs: HashMap<P::Subscriber, (P::Addr, ItemId)>,
    sink: Option<P>,
}

impl<P: ReplySink> Clone for Publisher<P> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<P: ReplySink> Default for Publisher<P> {
    fn default() -> Self {
        Self(Arc::new(StdMutex::new(Subscribers {
            subs: HashMap::new(),
            sink: None,
        })))
    }
}

/// MOD-41 blueprint F-9: hand-written, so no `P: Debug` is asked of a sink. It never waits for the
/// map, as the derived `Mutex` one never did.
impl<P: ReplySink> core::fmt::Debug for Publisher<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Publisher");
        match self.0.try_lock() {
            Ok(state) => debug
                .field("subscribers", &state.subs.len())
                .field("wired", &state.sink.is_some())
                .finish(),
            Err(_) => debug.finish_non_exhaustive(),
        }
    }
}

impl<P: ReplySink> Publisher<P> {
    fn lock(&self) -> std::sync::MutexGuard<'_, Subscribers<P>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The sink frames go out on: the loop's reply channel.
    ///
    /// MOD-41 blueprint F-10: the last wire wins. Every caller wires the one sink of its runtime
    /// (the loop's channel, or a test's one channel per runtime), so replacing it unconditionally
    /// is the same as keeping the first live one.
    pub(crate) fn wire(&self, sink: &P) {
        self.lock().sink = Some(sink.clone());
    }

    /// `addr`'s subscriber now follows `item`, at `addr`.
    pub(crate) fn subscribe(&self, addr: P::Addr, item: ItemId) {
        self.lock().subs.insert(P::subscriber(&addr), (addr, item));
    }
}

/// MOD-41 blueprint F-9: what [`ChatEnd`] and [`ProgressSink`] publish through, so neither names
/// the sink's type.
pub(crate) trait Publish: Send + Sync + core::fmt::Debug {
    /// One frame to every subscriber of its item.
    fn publish(&self, frame: &RunFrame);
}

impl<P: ReplySink> Publish for Publisher<P> {
    fn publish(&self, frame: &RunFrame) {
        let state = self.lock();
        let Some(sink) = state.sink.as_ref() else {
            return;
        };
        for (addr, item) in state.subs.values() {
            if *item != frame.item {
                continue;
            }
            sink.send(addr, RunReply::Frame(frame.clone()));
        }
    }
}
