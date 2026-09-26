//! The Skills view of the Skills tab (MOD-9 milestone 3, plan D82): not built yet.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::{Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::store_worker::StoreReply;
use crossterm::event::KeyEvent;

/// The Skills view: nothing yet.
#[derive(Debug, Default)]
pub(super) struct LibraryView;

impl LibraryView {
    /// Never.
    pub(super) fn captures_input(&self) -> bool {
        false
    }

    /// Nothing to reset.
    pub(super) fn on_scope_change(&mut self) {}

    /// Every key passes.
    pub(super) fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }

    /// Ignored.
    pub(super) fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    /// Ignored.
    pub(super) fn on_external_edit(&mut self, _outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {}

    /// Draws nothing.
    pub(super) fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
}
