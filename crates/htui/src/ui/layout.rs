//! The frame split every screen shares: top bar, tab strip, body, hint row, status line.

use ratatui::layout::{Constraint, Flex, Layout, Rect};

/// The five fixed regions of a frame. Tabs only ever draw inside [`Chrome::body`]; overlays get
/// the whole frame and centre themselves with [`centered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chrome {
    /// One line: `workspace · box · store · N working · M waiting`.
    pub top_bar: Rect,
    /// One line: the registered tabs in registration order.
    pub tab_strip: Rect,
    /// Everything left over: the active tab.
    pub body: Rect,
    /// One line: the active tab's keys (MOD-67 M6), blank while an overlay or the in-pane editor
    /// holds them. Always reserved, so the body's height never depends on the mode.
    pub hint: Rect,
    /// One line: the last error, or the global help line.
    pub status: Rect,
}

/// Splits a frame into its five regions.
#[must_use]
pub fn chrome(area: Rect) -> Chrome {
    let [top_bar, tab_strip, body, hint, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    Chrome {
        top_bar,
        tab_strip,
        body,
        hint,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-67 M6 T2: the hint row sits between the body and the status line, one row always, so
    /// the body is one row shorter than before on every frame.
    #[test]
    fn the_chrome_has_one_hint_row_above_the_status_line() {
        let chrome = chrome(Rect::new(0, 0, 100, 30));
        assert_eq!(chrome.top_bar, Rect::new(0, 0, 100, 1));
        assert_eq!(chrome.tab_strip, Rect::new(0, 1, 100, 1));
        assert_eq!(chrome.body, Rect::new(0, 2, 100, 26));
        assert_eq!(chrome.hint, Rect::new(0, 28, 100, 1));
        assert_eq!(chrome.status, Rect::new(0, 29, 100, 1));

        let small = super::chrome(Rect::new(0, 0, 40, 5));
        assert_eq!(
            small.body,
            Rect::new(0, 2, 40, 1),
            "a 5-row frame keeps a one-row body"
        );
        assert_eq!(small.hint, Rect::new(0, 3, 40, 1));
        assert_eq!(small.status, Rect::new(0, 4, 40, 1));
    }
}
