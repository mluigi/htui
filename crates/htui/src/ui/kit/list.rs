//! List parts: the `▌` gutter and selection band, ellipsis, group rows and the "hidden" note
//! (MOD-115).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

use crate::ui::Theme;

/// The marker in front of the selected row.
pub const GUTTER: &str = "▌";

/// What a row is, so the renderer knows whether it can be selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A selectable item.
    Item,
    /// A group header, folded or open.
    Group {
        /// Whether the group's children are hidden.
        folded: bool,
    },
    /// A note row (such as "3 hidden"); never selected.
    Note,
}

/// One row of a list.
#[derive(Debug, Clone)]
pub struct Row<'a> {
    /// The row's content.
    pub line: Line<'a>,
    /// What kind of row it is.
    pub kind: RowKind,
}

/// Draws `rows` from `offset` into `area`, the `selected` one in the band. Stub: draws nothing.
pub fn render(
    _frame: &mut Frame<'_>,
    _area: Rect,
    _rows: &[Row<'_>],
    _selected: Option<usize>,
    _offset: usize,
    _accent: Style,
    _theme: &Theme,
) {
}

/// `offset` moved the least that brings `selected` into a viewport of `height` rows over
/// `total`. Stub: keeps `offset` where `selected` fits, else follows it.
#[must_use]
pub fn scroll_into_view(selected: usize, offset: usize, height: usize, total: usize) -> usize {
    let last = total.saturating_sub(height);
    if selected < offset {
        selected
    } else if height > 0 && selected >= offset + height {
        (selected + 1 - height).min(last)
    } else {
        offset.min(last)
    }
}
