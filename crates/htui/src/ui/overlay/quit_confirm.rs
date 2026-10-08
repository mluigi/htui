//! MOD-57 M2 E4 (PRD D5): the question `global.quit` asks while an in-pane editor is alive.
//! (T2 fills in the question, the keys and the box.)

use crossterm::event::KeyEvent;
use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::{Ctx, Handled};
use crate::keys::{Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::overlay::registry::{Overlay, OverlayId};

/// Asks before `global.quit` drops an open edit.
#[derive(Debug, Default)]
pub struct QuitConfirm;

impl QuitConfirm {
    /// Identity of the confirm; `register_all` registers the factory under it and names it as
    /// `App::quit_overlay`.
    pub const ID: OverlayId = OverlayId("quit_confirm");

    /// The confirm.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Overlay for QuitConfirm {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Quit"
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::QUIT_CONFIRM)
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }

    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        // The stub's keys are all swallowed by `is_modal`.
        Handled::Pass
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
}
