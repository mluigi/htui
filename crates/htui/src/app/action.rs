//! Everything that can happen, as data (plan D5).
//!
//! A view never mutates the shell: it emits an [`Action`] through
//! [`Ctx::emit`](crate::app::Ctx::emit) and `App::update` is the single place that applies it.

use htui_core::model::{ItemId, RequirementId, RunId, StepId, WorkspaceSummary};

use crate::editor::ExternalEdit;
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
    /// Promote a step to a chat (MOD-4 plan D165). Emitted by the Runs pane; the shell focuses the
    /// tab that drives chats (`App::replay_tab`) and asks for the promotion on its behalf, so the
    /// promotion's replies land in that tab.
    Promote {
        /// The step's run.
        run: RunId,
        /// The step to promote.
        step: StepId,
    },
    /// Select an entity in the tab that shows it (MOD-64 D235). Emitted by the concepts search; the
    /// shell focuses the tab registered for the target's kind (`App::reveal_tabs`) and hands it the
    /// target through `Tab::reveal`, so the overlay never names a tab.
    Reveal(RevealTarget),
    /// Hand text to `$VISUAL`/`$EDITOR` (MOD-9 D10). Stamped with the emitting tab in
    /// `App::drain`, which holds it for the event loop; any other origin is refused on the status
    /// line.
    EditExternally(ExternalEdit),
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

/// What `Action::Reveal` selects (MOD-64 D235). The key rides along for the sentence a tab says
/// when the row is not in this workspace (blueprint D248).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevealTarget {
    /// An item; a document hit reveals its owner item.
    Item {
        /// The item.
        id: ItemId,
        /// Its key, e.g. `FEAT-1`.
        key: String,
    },
    /// MOD-69 plan D8: an item, opened on its Runs sub-tab with the cursor on `step` (or on
    /// `run`'s first entry when `step` is `None`; with neither, on whatever the pane selects).
    Step {
        /// The item.
        item: ItemId,
        /// Its key, for the miss sentence (blueprint D248).
        key: String,
        /// The run, when the row has one.
        run: Option<RunId>,
        /// The step, when the row has one.
        step: Option<StepId>,
    },
    /// A requirement.
    Requirement {
        /// The requirement.
        id: RequirementId,
        /// Its key, e.g. `R-STO-8`.
        key: String,
    },
}

impl RevealTarget {
    /// Which registration routes it.
    #[must_use]
    pub const fn kind(&self) -> RevealKind {
        match self {
            Self::Item { .. } | Self::Step { .. } => RevealKind::Item,
            Self::Requirement { .. } => RevealKind::Requirement,
        }
    }

    /// The target's key.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Item { key, .. } | Self::Step { key, .. } | Self::Requirement { key, .. } => key,
        }
    }
}

/// Which kind of entity a tab reveals: the key of `App::reveal_tabs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevealKind {
    /// Items (the Backlog).
    Item,
    /// Requirements (the Requirements tab).
    Requirement,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-69 plan D8: a step reveal is routed to the Backlog, as an item's is, and its miss
    /// sentence names the item's key.
    #[test]
    fn a_step_target_routes_as_an_item_and_keeps_its_key() {
        let target = RevealTarget::Step {
            item: ItemId::new(),
            key: "FEAT-1".to_owned(),
            run: Some(RunId::new()),
            step: Some(StepId::new()),
        };
        assert_eq!(target.kind(), RevealKind::Item);
        assert_eq!(target.key(), "FEAT-1");

        let reopen = RevealTarget::Step {
            item: ItemId::new(),
            key: "FEAT-2".to_owned(),
            run: None,
            step: None,
        };
        assert_eq!(reopen.kind(), RevealKind::Item);
        assert_eq!(reopen.key(), "FEAT-2");
    }
}
