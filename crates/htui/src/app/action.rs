//! Everything that can happen, as data (plan D5).
//!
//! A view never mutates the shell: it emits an [`Action`] through
//! [`Ctx::emit`](crate::app::Ctx::emit) and `App::update` is the single place that applies it.

use htui_core::model::{StepId, WorkspaceSummary};

use crate::store_worker::{ReplyEnvelope, StoreRequest};
use crate::ui::overlay::OverlayId;
use crate::ui::tabs::TabId;
use crate::ui::tabs::settings::SectionId;

/// One state change of the shell.
#[derive(Debug, Clone)]
pub enum Action {
    /// Leave the application; the event loop stops after the current iteration.
    Quit,
    /// Move around the tab registry.
    Tab(TabAction),
    /// Open or close an overlay.
    Overlay(OverlayAction),
    /// Ask the store. Stamped with the emitting view's [`Origin`](crate::store_worker::Origin)
    /// when the emit queue is drained, so a view cannot address someone else's reply.
    Store(StoreRequest),
    /// A reply came back from the worker.
    Reply(ReplyEnvelope),
    /// Enter a workspace: the only way the scope ever changes (plan D10). Emitted by T5's
    /// switcher and by the shell itself at startup.
    SetScope {
        /// The workspace to enter, with its projects in `workspace_project.position` order.
        workspace: WorkspaceSummary,
    },
    /// Reopen a past step read-only (MOD-2 D39, `R-HIS-2`). Emitted by the Runs pane.
    ///
    /// The emitting view names the step and nothing else: which tab replays it is the shell's
    /// registration (`App::replay_tab`), so the Backlog tab never has to know that a Chat tab
    /// exists.
    Replay {
        /// The step whose persisted rows are to be replayed.
        step_id: StepId,
    },
    /// Show or hide the key help.
    ToggleHelp,
    /// Put a message on the status line: what a `StoreReply::Failed` becomes.
    Error(String),
    /// The 250 ms timer fired.
    Tick,
}

/// Movement inside the tab registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAction {
    /// Next tab, wrapping.
    Next,
    /// Previous tab, wrapping.
    Prev,
    /// The tab at this registration index; ignored when out of range.
    Select(usize),
    /// The tab with this id; ignored when it is not registered.
    Focus(TabId),
    /// Focus `tab` and, within it, `section` (MOD-15 M6 D7): the shell's redirect to the
    /// connection section when no DSN is stored.
    ///
    /// Either id being unregistered is a no-op, as [`Focus`](TabAction::Focus)'s is — a build
    /// that dropped a view must not kill the shell, and must not silently land somewhere else.
    FocusSection(TabId, SectionId),
}

/// Opening and closing overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    /// Build the overlay registered under this id and push it. A missing factory is ignored.
    Open(OverlayId),
    /// Close the topmost overlay.
    Close,
    /// Close every overlay: what a scope change does.
    CloseAll,
}

/// Whether a view took the key.
///
/// Two states are enough because a view's side effects travel through
/// [`Ctx::emit`](crate::app::Ctx::emit): one key may produce several actions and the return value
/// only answers "did you take it?" (blueprint C.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// The key was used; propagation stops here.
    Consumed,
    /// The key was ignored; the next stop in the chain gets it.
    Pass,
}
