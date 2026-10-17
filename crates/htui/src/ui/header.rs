//! The one-row header that replaces the top bar and the tab strip (MOD-115): workspace and tab
//! chips on the left, working and waiting counts, store, box and the key chips on the right.
//!
//! Task 0 of M1 declares the surface; `top_bar` keeps drawing the frame until task 5.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::TopBarState;
use crate::keys::Keys;
use crate::ui::Theme;
use crate::ui::tabs::registry::{TabId, TabRegistry};

/// A message that replaces the header's right side for a moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flash {
    /// The text.
    pub text: String,
    /// Its style.
    pub style: Style,
}

/// Where the header's clickable parts landed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderHits {
    /// The workspace name.
    pub workspace: Rect,
    /// Each tab chip.
    pub tabs: Vec<(TabId, Rect)>,
}

/// Draws the header into a one-row `area`. Stub: draws nothing.
pub fn render(
    _frame: &mut Frame<'_>,
    _area: Rect,
    _state: &TopBarState,
    _tabs: &TabRegistry,
    _flash: Option<&Flash>,
    _keys: &Keys,
    _theme: &Theme,
) -> HeaderHits {
    HeaderHits::default()
}
