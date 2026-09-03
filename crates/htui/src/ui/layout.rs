//! The frame split every screen shares: top bar, tab strip, body, status line.

use ratatui::layout::{Constraint, Flex, Layout, Rect};

/// The four fixed regions of a frame. Tabs only ever draw inside [`Chrome::body`]; overlays get
/// the whole frame and centre themselves with [`centered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chrome {
    /// One line: `workspace · box · store · N runs`.
    pub top_bar: Rect,
    /// One line: the registered tabs in registration order.
    pub tab_strip: Rect,
    /// Everything left over: the active tab.
    pub body: Rect,
    /// One line: the last error, or the global help line.
    pub status: Rect,
}

/// Splits a frame into its four regions.
#[must_use]
pub fn chrome(area: Rect) -> Chrome {
    let [top_bar, tab_strip, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    Chrome {
        top_bar,
        tab_strip,
        body,
        status,
    }
}

/// A `width` x `height` rectangle centred inside `area`, clamped to it.
///
/// Overlays render over the whole frame (`Overlay::render` takes the frame area), so this is how
/// they place their own box without each one re-deriving the arithmetic.
#[must_use]
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [horizontal] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [centre] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(horizontal);
    centre
}
