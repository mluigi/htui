//! MOD-57 M2 E4 (PRD D5): the question `global.quit` asks while an in-pane editor is alive.
//!
//! Quitting drops `App`, and the editor with it: whatever the user typed and did not save is
//! gone. So while an editor is alive `global.quit` does not quit; `App::request_quit` opens this
//! box instead, and only `y` (`confirm.yes`) quits. `n` and `Esc` (`confirm.no`, then
//! `overlay.close`) close it and leave the edit as it was. `ctrl-c` never reaches it: the shell's
//! hard quit (C3) runs before any overlay sees a key.
//!
//! The question is fixed text (MOD-57 M2 PD-M2-1): the factory signature is
//! `Fn() -> Box<dyn Overlay>`, so the shell cannot hand the box the editor's value, and the
//! shell never names a concrete overlay. Shaped exactly like
//! [`MigrationPrompt`](super::MigrationPrompt): a centred, modal box with its own hint line.

use crossterm::event::KeyEvent;
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

/// What the box asks (PD-M2-1: fixed; the factory takes no argument).
const QUESTION: &str = "An edit is still open in the editor. Quit and lose it?";

/// The line under the question: `confirm.yes` and `confirm.no` here, `overlay.close` (the same
/// effect as `no`) by the shell.
const HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "quit"),
    Hint::Pair(Act::ConfirmNo, Act::OverlayClose, "keep editing"),
];

/// Left border, right border and one column of right padding.
const CHROME: u16 = 3;

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

    /// The box's contents: the question, a blank line, then `hint` (the rendered [`HINT`]).
    fn lines(hint: String, theme: &Theme) -> Vec<Line<'static>> {
        vec![
            Line::styled(QUESTION, theme.base),
            Line::raw(""),
            Line::styled(hint, theme.dim),
        ]
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

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // `y` is `confirm.yes`; `n` and `Esc` are `confirm.no` (`Esc` is `overlay.close` too,
        // `SHADOWING`: `confirm.no` resolves first).
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::QUIT_CONFIRM, chord) {
            match act {
                Act::ConfirmYes => {
                    // No `Close`: the loop stops, `App` drops, and the editor with it.
                    ctx.emit(Action::Quit);
                    return Handled::Consumed;
                }
                Act::ConfirmNo => {
                    // The edit stays as it was.
                    ctx.emit(Action::Overlay(OverlayAction::Close));
                    return Handled::Consumed;
                }
                _ => continue, // `overlay.close` or `global.help`: the shell's
            }
        }
        // Everything else is swallowed by `is_modal`: `q` opens no second question.
        Handled::Pass
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let lines = Self::lines(ctx.keys().hint(views::QUIT_CONFIRM, HINT), ctx.theme);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keys::{Keys, load_str};
    use crate::store_worker::Origin;
    use crate::ui::cells::cell_width;
    use htui_core::model::{ProjectRef, WorkspaceId};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Everything a `Ctx` borrows; `keys` replaces the compiled defaults when set.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        theme: Theme,
        keys: Option<Keys>,
        emit: Emit,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: Vec::new(),
                },
                projects: Vec::new(),
                top_bar: TopBarState::default(),
                theme: Theme::default(),
                keys: None,
                emit: Emit::default(),
            }
        }

        /// The bench with a loaded key file in force.
        fn with_keys(mut self, keys: Keys) -> Self {
            self.keys = Some(keys);
            self
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                self.keys.as_ref().unwrap_or(Keys::compiled()),
                &self.theme,
                Origin::Overlay(QuitConfirm::ID),
                &self.emit,
            )
        }

        /// Sends one chord (a strict key-file spec) to the confirm.
        fn key(&self, confirm: &mut QuitConfirm, spec: &str) -> Handled {
            let chord = KeyChord::parse_strict(spec).expect("a valid spec");
            confirm.on_key(chord.to_event(), &mut self.ctx())
        }

        /// The confirm drawn over a blank 100×30 frame, one line per row.
        fn render(&self, confirm: &QuitConfirm) -> String {
            let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
            term.draw(|frame| confirm.render(frame, frame.area(), &self.ctx()))
                .expect("the overlay draws");
            let buffer = term.backend().buffer();
            let mut out = String::new();
            for y in 0..buffer.area.height {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    x += u16::try_from(cell_width(symbol).max(1)).unwrap_or(1);
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out
        }
    }

    #[test]
    fn y_quits() {
        let bench = Bench::new();
        let mut confirm = QuitConfirm::new();

        assert_eq!(bench.key(&mut confirm, "y"), Handled::Consumed);
        let emitted = bench.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Quit]),
            "y quits and nothing else: {emitted:?}"
        );
    }

    #[test]
    fn n_and_esc_keep_the_edit() {
        let bench = Bench::new();
        let mut confirm = QuitConfirm::new();

        for spec in ["n", "esc"] {
            assert_eq!(bench.key(&mut confirm, spec), Handled::Consumed, "{spec}");
            let emitted = bench.emit.take();
            assert!(
                matches!(emitted.as_slice(), [Action::Overlay(OverlayAction::Close)]),
                "{spec} closes the question and nothing else: {emitted:?}"
            );
        }
    }

    #[test]
    fn every_other_key_is_left_to_the_modal_swallow() {
        let bench = Bench::new();
        let mut confirm = QuitConfirm::new();

        for spec in ["q", "Y", "enter", "ctrl-y", "?"] {
            assert_eq!(bench.key(&mut confirm, spec), Handled::Pass, "{spec}");
            assert!(bench.emit.is_empty(), "{spec} emits nothing");
        }
    }

    #[test]
    fn the_hint_follows_the_keys() {
        let keys = load_str("[confirm]\nyes = [\"f9\"]\n").expect("the file loads");
        let bench = Bench::new().with_keys(keys);

        let frame = bench.render(&QuitConfirm::new());
        assert!(frame.contains("F9 quit · n/Esc keep editing"), "{frame}");
    }

    #[test]
    fn the_box() {
        let bench = Bench::new();
        insta::assert_snapshot!(bench.render(&QuitConfirm::new()));
    }
}
