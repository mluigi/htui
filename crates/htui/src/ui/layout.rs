//! The frame split every screen shares: the one-row header, the optional about panel, the body.

use ratatui::layout::{Constraint, Flex, Layout, Rect};

/// The rows the about panel takes: two border rows and two content rows.
pub const ABOUT_HEIGHT: u16 = 4;

/// The fewest body rows a frame keeps while it still shows the about panel; a frame too short for
/// the header, the panel and this many body rows hides the panel and keeps the body.
const MIN_BODY_WITH_ABOUT: u16 = 1;

/// The regions of a frame. Tabs only ever draw inside [`Chrome::body`]; overlays get the whole
/// frame and centre themselves with [`centered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chrome {
    /// One line: workspace and tab chips on the left, the live state and key chips on the right.
    pub header: Rect,
    /// [`ABOUT_HEIGHT`] rows under the header: what the tab is for. `None` when it is switched off
    /// or the frame is too short to spare it.
    pub about: Option<Rect>,
    /// Everything left over: the active tab. Its last row is where the key hints are painted.
    pub body: Rect,
}

/// Splits a frame into header and body: no about panel.
#[must_use]
pub fn chrome(area: Rect) -> Chrome {
    chrome_with(area, false)
}

/// Splits a frame into header, the about panel when `about` is asked for and the frame has room
/// for it (the header, the four panel rows and [`MIN_BODY_WITH_ABOUT`] body row), and body.
#[must_use]
pub fn chrome_with(area: Rect, about: bool) -> Chrome {
    let [header, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    if about && rest.height >= ABOUT_HEIGHT + MIN_BODY_WITH_ABOUT {
        let [panel, body] =
            Layout::vertical([Constraint::Length(ABOUT_HEIGHT), Constraint::Min(0)]).areas(rest);
        return Chrome {
            header,
            about: Some(panel),
            body,
        };
    }
    Chrome {
        header,
        about: None,
        body: rest,
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

    /// MOD-115: one header row, then the body; the five-row chrome is gone.
    #[test]
    fn the_chrome_is_a_header_row_over_the_body() {
        let chrome = chrome(Rect::new(0, 0, 100, 30));
        assert_eq!(chrome.header, Rect::new(0, 0, 100, 1));
        assert_eq!(chrome.about, None);
        assert_eq!(chrome.body, Rect::new(0, 1, 100, 29));
    }

    #[test]
    fn the_about_panel_takes_four_rows_under_the_header() {
        let chrome = chrome_with(Rect::new(0, 0, 100, 30), true);
        assert_eq!(chrome.header, Rect::new(0, 0, 100, 1));
        assert_eq!(chrome.about, Some(Rect::new(0, 1, 100, 4)));
        assert_eq!(chrome.body, Rect::new(0, 5, 100, 25));
        let off = chrome_with(Rect::new(0, 0, 100, 30), false);
        assert_eq!(off.about, None);
        assert_eq!(off.body, Rect::new(0, 1, 100, 29));
    }

    #[test]
    fn a_short_frame_drops_the_about_panel_and_keeps_the_body() {
        // header 1 + panel 4 + body 1 is the smallest frame that shows the panel.
        let fits = chrome_with(Rect::new(0, 0, 40, 6), true);
        assert_eq!(fits.about, Some(Rect::new(0, 1, 40, 4)));
        assert_eq!(fits.body, Rect::new(0, 5, 40, 1));
        let short = chrome_with(Rect::new(0, 0, 40, 5), true);
        assert_eq!(short.about, None);
        assert_eq!(short.body, Rect::new(0, 1, 40, 4));
        let tiny = chrome_with(Rect::new(0, 0, 40, 1), true);
        assert_eq!(tiny.header, Rect::new(0, 0, 40, 1));
        assert_eq!(tiny.body.height, 0);
    }
}
