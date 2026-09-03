//! The one-line header: `workspace · box · store · N runs` (`R-TUI-1`).
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
    let runs = if state.active_runs == 1 {
        "1 run".to_owned()
    } else {
        format!("{} runs", state.active_runs)
    };
    let line = Line::from(vec![
        Span::styled(workspace.to_owned(), theme.title),
        Span::styled(SEP, theme.dim),
        Span::styled(box_name.to_owned(), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(state.store.clone(), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(runs, theme.base),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}
