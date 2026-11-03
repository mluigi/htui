//! MOD-57 M2 R1 M-2: the question `editor.abort` asks on the asking tab.
//!
//! A tab switch lands on the asking tab unfocused, so nano's habitual `ctrl-x` (`editor.abort`'s
//! default) would kill the edit with no question. So while an editor is alive the abort opens
//! this box instead (`App::request_abort`), and only `editor.abort` **again** aborts
//! ([`Action::AbortEdit`]), on a press: a held abort key's repeats are consumed and leave the
//! question up. A `y` cannot confirm it: every other key closes the box, keeps the
//! edit as it was and goes nowhere else, so nano's `ctrl-x`, `y` keeps the edit. `global.quit`
//! (`ctrl-c`) is passed to the shell, whose quit layer opens the leave dialog over the question
//! (MOD-115 M2 B4); `Esc` there comes back to it. A paste under it
//! is dropped (it reaches neither the editor nor a tab) and the question stays. An editor that
//! exits under it takes it along (`App::on_pane_event`), and so does a tab move off the asking
//! tab (`App::update_tab`).
//!
//! Fixed text, shaped like [`LeaveDialog`](super::LeaveDialog): the factory takes no argument.

use crossterm::event::{KeyEvent, KeyEventKind};
use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::modal::Ask;
use crate::ui::overlay::registry::{Overlay, OverlayId};

/// What the box asks.
const QUESTION: &str = "Abort the edit and lose it?";

/// The keys on the box's bottom border: the abort key, generated, then the fixed rest (prose, not
/// a key: every other key keeps editing).
const HINT: HintSpec = &[
    Hint::One(Act::EditorAbort, "again aborts"),
    Hint::Text("any other key keeps editing"),
];

/// Asks before `editor.abort` drops an open edit.
#[derive(Debug, Default)]
pub struct AbortConfirm;

impl AbortConfirm {
    /// Identity of the confirm; `register_all` registers the factory under it and names it as
    /// `App::abort_overlay`.
    pub const ID: OverlayId = OverlayId("abort_confirm");

    /// The confirm.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Overlay for AbortConfirm {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Abort"
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::ABORT_CONFIRM)
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        let acts = ctx.keys().actions(views::ABORT_CONFIRM, chord);
        if acts.first() == Some(&Act::Quit) {
            // MOD-115 M2 B4: the shell's quit layer opens the leave dialog.
            return Handled::Pass;
        }
        if acts.contains(&Act::EditorAbort) {
            // M2 R2 LOW-1: a held abort key repeats the press that opened this box; only a fresh
            // press confirms, and a repeat leaves the question up.
            if key.kind == KeyEventKind::Press {
                // `App::abort_editor` closes this box with the edit.
                ctx.emit(Action::AbortEdit);
            }
        } else {
            ctx.emit(Action::Overlay(OverlayAction::Close));
        }
        Handled::Consumed
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    /// A kit question (MOD-115 M4 E7, C7): the question in the box, [`HINT`] on its bottom
    /// border.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let hint = ctx.keys().hint(views::ABORT_CONFIRM, HINT);
        Ask::new(self.title())
            .line(Line::styled(QUESTION, ctx.theme.base))
            .hint(&hint)
            .render(frame, area, ctx.theme);
    }
}
