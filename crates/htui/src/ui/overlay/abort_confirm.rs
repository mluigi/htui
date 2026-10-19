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
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::layout::centered;
use crate::ui::overlay::registry::{Overlay, OverlayId};

/// What the box asks.
const QUESTION: &str = "Abort the edit and lose it?";

/// The line under the question: the abort key, generated, then the fixed rest.
const HINT: HintSpec = &[
    Hint::One(Act::EditorAbort, "again aborts"),
    Hint::Text("any other key keeps editing"),
];

/// Left border, right border and one column of right padding.
const CHROME: u16 = 3;

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

    /// The box's contents: the question, a blank line, then `hint` (the rendered [`HINT`]).
    fn lines(hint: String, theme: &Theme) -> Vec<Line<'static>> {
        vec![
            Line::styled(QUESTION, theme.base),
            Line::raw(""),
            Line::styled(hint, theme.dim),
        ]
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

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let lines = Self::lines(ctx.keys().hint(views::ABORT_CONFIRM, HINT), ctx.theme);
        let widest = lines.iter().map(Line::width).max().unwrap_or(0);
        let width = u16::try_from(widest)
            .unwrap_or(u16::MAX)
            .saturating_add(CHROME);
        let height = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2);
        let box_area = centered(area, width, height);

        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(Span::styled(format!(" {} ", self.title()), ctx.theme.title)),
            ),
            box_area,
        );
    }
}
