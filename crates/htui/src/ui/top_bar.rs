//! The one-line header: `workspace · box · store · N working · M waiting` (`R-TUI-1`,
//! `R-TUI-11`).
//!
//! Every field comes from [`TopBarState`], which only [`App::observe_reply`](crate::app::App)
//! writes, so the header cannot disagree with what the store answered.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::TopBarState;
use crate::ui::Theme;

/// Separator between the header's fields.
const SEP: &str = " · ";

/// Draws the top bar into a one-line area.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &TopBarState, theme: &Theme) {
    let workspace = if state.workspace.is_empty() {
        "no workspace"
    } else {
        &state.workspace
    };
    let box_name = if state.box_name.is_empty() {
        "no box"
    } else {
        &state.box_name
    };
    let (working, waiting) = state
        .waiting
        .as_ref()
        .map_or((0, 0), |view| (view.working, view.waiting()));
    // MOD-69 plan T3.3: the waiting part in `accent` when a person is owed something (`Theme` has
    // no warning style); no plural rule, so "1 working · 1 waiting" reads as written.
    let waiting_style = if waiting > 0 {
        theme.accent
    } else {
        theme.base
    };
    let line = Line::from(vec![
        Span::styled(workspace.to_owned(), theme.title),
        Span::styled(SEP, theme.dim),
        Span::styled(box_name.to_owned(), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(state.store.clone(), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(format!("{working} working"), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(format!("{waiting} waiting"), waiting_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::ItemId;
    use htui_worker::{WaitingReason, WaitingRow, WaitingView};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    /// The top bar of `state`, drawn 80 cells wide.
    fn draw(state: &TopBarState, theme: &Theme) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).expect("a test terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), state, theme))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row(key: &str) -> WaitingRow {
        WaitingRow {
            item: ItemId::new(),
            item_key: key.to_owned(),
            run: None,
            step: None,
            step_label: String::new(),
            reason: WaitingReason::Unblock,
            text: "blocked".to_owned(),
        }
    }

    fn line(buffer: &Buffer) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    /// MOD-69 plan T3.3: `M waiting` is accented exactly when a person is owed something.
    #[test]
    fn the_waiting_count_is_accented_only_when_non_zero() {
        let theme = Theme::default();
        let mut state = TopBarState {
            workspace: "Platform".to_owned(),
            box_name: "DESKTOP-HTUI".to_owned(),
            store: "memory".to_owned(),
            waiting: Some(WaitingView {
                working: 1,
                rows: Vec::new(),
                permissions_known: true,
                offline: false,
            }),
        };
        let idle = draw(&state, &theme);
        let text = line(&idle);
        assert_eq!(
            text,
            "Platform · DESKTOP-HTUI · memory · 1 working · 0 waiting"
        );
        let at = u16::try_from(text.chars().count() - "0 waiting".len()).expect("fits");
        assert_ne!(
            idle[(at, 0)].style().fg,
            theme.accent.fg,
            "nothing owed, no accent"
        );

        state.waiting = Some(WaitingView {
            working: 0,
            rows: vec![row("FEAT-2"), row("FEAT-3")],
            permissions_known: true,
            offline: false,
        });
        let owed = draw(&state, &theme);
        let text = line(&owed);
        assert_eq!(
            text,
            "Platform · DESKTOP-HTUI · memory · 0 working · 2 waiting"
        );
        let at = u16::try_from(text.chars().count() - "2 waiting".len()).expect("fits");
        assert_eq!(owed[(at, 0)].symbol(), "2");
        assert_eq!(owed[(at, 0)].style().fg, theme.accent.fg);
        let byte = text.find("0 working").expect("working");
        let working = u16::try_from(text[..byte].chars().count()).expect("fits");
        assert_ne!(
            owed[(working, 0)].style().fg,
            theme.accent.fg,
            "working is never accented"
        );
    }

    /// MOD-69 blueprint A-6: before the first reply the bar reads zero for both counts.
    #[test]
    fn before_the_first_reply_both_counts_read_zero() {
        let state = TopBarState::default();
        assert_eq!(
            line(&draw(&state, &Theme::default())),
            "no workspace · no box ·  · 0 working · 0 waiting"
        );
    }
}
