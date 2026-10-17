//! The empty state: what is missing and always a next step (MOD-115).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::ui::Theme;

/// The next step an empty state points at. Required, so no empty state is a dead end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextStep<'a>(pub &'a str);

/// The lines of an empty state for `what`. Stub: none.
#[must_use]
pub fn lines(_what: &str, _next: NextStep<'_>, _theme: &Theme) -> Vec<Line<'static>> {
    Vec::new()
}

/// Draws an empty state into `area`. Stub: draws nothing.
pub fn render(
    _frame: &mut Frame<'_>,
    _area: Rect,
    _what: &str,
    _next: NextStep<'_>,
    _theme: &Theme,
) {
}
