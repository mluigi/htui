//! The rounded pane: title, sub-tab chips, bottom-border hints, focus and a scrollbar (MOD-115).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

use crate::ui::Theme;

/// Whether a pane has focus: the focused border takes the tab accent, the other the faint one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The pane keys go to.
    Focused,
    /// Any other pane.
    Unfocused,
}

/// A pane under construction; [`Pane::render`] draws it.
#[allow(dead_code, reason = "stub: MOD-115 M1 task 1 reads these fields")]
#[derive(Debug, Clone)]
pub struct Pane<'a> {
    title: &'a str,
    sub_tabs: Vec<&'a str>,
    active: usize,
    hints: Option<Line<'a>>,
    focus: Focus,
    accent: Style,
    scrollbar: Option<(usize, usize)>,
}

impl<'a> Pane<'a> {
    /// A pane titled `title`, unfocused, with no chips, hints or scrollbar.
    #[must_use]
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            sub_tabs: Vec::new(),
            active: 0,
            hints: None,
            focus: Focus::Unfocused,
            accent: Style::new(),
            scrollbar: None,
        }
    }

    /// Sub-tab chips on the top border, `active` being the current one.
    #[must_use]
    pub fn sub_tabs(mut self, titles: &[&'a str], active: usize) -> Self {
        self.sub_tabs = titles.to_vec();
        self.active = active;
        self
    }

    /// Hints on the bottom border, drawn only while the pane is focused.
    #[must_use]
    pub fn hints(mut self, hints: Line<'a>) -> Self {
        self.hints = Some(hints);
        self
    }

    /// Sets the focus.
    #[must_use]
    pub fn focus(mut self, focus: Focus) -> Self {
        self.focus = focus;
        self
    }

    /// The focused border's style (the tab accent).
    #[must_use]
    pub fn accent(mut self, accent: Style) -> Self {
        self.accent = accent;
        self
    }

    /// A scrollbar for `total` rows scrolled to `offset`.
    #[must_use]
    pub fn scrollbar(mut self, total: usize, offset: usize) -> Self {
        self.scrollbar = Some((total, offset));
        self
    }

    /// Draws the pane into `area` and returns the inner rect for its content. Stub: draws
    /// nothing and returns `area`.
    pub fn render(self, _frame: &mut Frame<'_>, area: Rect, _theme: &Theme) -> Rect {
        area
    }
}

/// Where each sub-tab chip sits on the top border of a pane drawn into `area` (the rightmost
/// chip ends at `area.right() - 1 - width`). Stub: no rects.
#[must_use]
pub fn sub_tab_rects(_area: Rect, _titles: &[&str]) -> Vec<Rect> {
    Vec::new()
}

/// `offset` clamped so a scrolled list stops once its last row is in view.
#[must_use]
pub fn clamp_offset(offset: usize, total: usize, viewport: usize) -> usize {
    offset.min(total.saturating_sub(viewport))
}
