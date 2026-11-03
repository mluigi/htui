//! The Backlog's queue toggles (MOD-115 M4 T1): `Q` and `P`, the `QueueState` read each waits
//! on, and the status line a queue write's answer sets.

use htui_core::model::ItemState;

use crate::app::{Action, Ctx};
use crate::keys::Act;
use crate::store_worker::{QueueView, QueueWrite, StoreRequest};
use crate::ui::queue_status::queue_sentence;
use crate::ui::tabs::backlog::{BacklogTab, QueueIntent};

/// MOD-12 D9: what `Q` says with the cursor on a project header.
pub(super) const NO_ITEM_TO_QUEUE: &str = "select an item to queue";

impl BacklogTab {
    /// MOD-12 D9: `Q`. A finished item (MOD-144: state `done` or `closed`) is refused here and
    /// nothing is sent; any other reads the queue first, and the reply decides between queue and
    /// dequeue.
    pub(super) fn toggle_queued(&mut self, ctx: &Ctx<'_>) {
        let Some(item) = self.item() else {
            ctx.emit(Action::Error(NO_ITEM_TO_QUEUE.to_owned()));
            return;
        };
        let state = item.state();
        if matches!(state, ItemState::Done | ItemState::Closed) {
            ctx.emit(Action::Error(format!(
                "{} is {}: nothing to queue",
                item.key,
                state.word()
            )));
            return;
        }
        self.queue_intent = Some(QueueIntent::Item {
            id: item.id,
            key: item.key.clone(),
        });
        ctx.request(StoreRequest::QueueState);
    }

    /// MOD-12 D9: the queue as it is now decides the write the pending toggle sends, and spends
    /// the toggle: one intent is at most one write (review NIT-intent). A read nobody here asked
    /// for (no intent) is ignored.
    pub(super) fn on_queue(&mut self, view: &QueueView, ctx: &Ctx<'_>) {
        let Some(intent) = self.queue_intent.take() else {
            return;
        };
        let request = match intent {
            QueueIntent::Item { id, key } => {
                let request = if view.entries.contains(&id) {
                    StoreRequest::DequeueItem { item: id }
                } else {
                    StoreRequest::QueueItem {
                        item: id,
                        workspace: ctx.scope.workspace_id,
                    }
                };
                self.queue_writing = Some((id, key));
                request
            }
            // M3 review R1 M1: the write carries the state this read saw, so a queue that
            // changed between the read and the write refuses it rather than flipping the other way.
            QueueIntent::Pause => match view.open_batch {
                Some(batch) => StoreRequest::PauseQueue {
                    expect: Some(batch),
                },
                None => StoreRequest::ResumeQueue {
                    expect_paused: true,
                },
            },
        };
        ctx.request(request);
    }

    /// MOD-12 D9: a queue write landed; the status line says what it did. A toggle pressed since
    /// is left for its own read (review NIT-intent).
    pub(super) fn on_queue_written(&mut self, write: &QueueWrite, view: &QueueView, ctx: &Ctx<'_>) {
        let key = match write {
            QueueWrite::Queued { item } | QueueWrite::Dequeued { item, .. } => {
                match self.queue_writing.take_if(|(id, _)| id == item) {
                    Some((_, key)) => key,
                    None => self
                        .items
                        .iter()
                        .find(|row| row.id == *item)
                        .map_or_else(|| item.to_string(), |row| row.key.clone()),
                }
            }
            QueueWrite::Resumed { .. }
            | QueueWrite::Paused { .. }
            | QueueWrite::Moved { .. }
            | QueueWrite::Stale { .. } => String::new(),
        };
        let pause = ctx
            .keys()
            .label(self.detail.key_stack(), Act::BacklogPauseQueue);
        ctx.emit(Action::Error(queue_sentence(
            write,
            &key,
            view,
            pause.as_deref(),
        )));
    }
}
