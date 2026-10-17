//! The modal frame over [`layout::centered`](crate::ui::layout::centered) (MOD-115).

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::ui::Theme;

/// Rows a modal's chrome takes around its content.
pub const CHROME: u16 = 3;

/// How big a modal is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalSize {
    /// As big as its content, clamped to the screen.
    Fit {
        /// Content width in cells.
        content_w: u16,
        /// Content height in rows.
        content_h: u16,
    },
    /// An exact size.
    Fixed {
        /// Width in cells.
        w: u16,
        /// Height in rows.
        h: u16,
    },
}

/// A modal under construction; [`Modal::render`] draws it.
#[allow(dead_code, reason = "stub: MOD-115 M1 task 3 reads these fields")]
#[derive(Debug, Clone)]
pub struct Modal<'a> {
    title: &'a str,
    size: ModalSize,
    hint: Option<&'a str>,
}

impl<'a> Modal<'a> {
    /// A modal titled `title`, fitting a one-cell content.
    #[must_use]
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            size: ModalSize::Fit {
                content_w: 1,
                content_h: 1,
            },
            hint: None,
        }
    }

    /// Sets the size.
    #[must_use]
    pub fn size(mut self, size: ModalSize) -> Self {
        self.size = size;
        self
    }

    /// A hint on the bottom border.
    #[must_use]
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Clears and draws the modal over `area`, returning the content rect. Stub: draws nothing
    /// and returns `area`.
    pub fn render(self, _frame: &mut Frame<'_>, area: Rect, _theme: &Theme) -> Rect {
        area
    }
}
