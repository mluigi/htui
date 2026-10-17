//! The about panel (`F9`): what the current tab is for and its main actions (MOD-115).

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::keys::{Act, Keys, Stack};
use crate::ui::Theme;
use crate::ui::tabs::registry::TabId;

/// One tab's about text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct About {
    /// What the tab is for, in a sentence.
    pub purpose: &'static str,
    /// Its main actions, each with the word shown beside its key; an act with no label in the
    /// current keys is skipped.
    pub actions: &'static [(Act, &'static str)],
}

/// The about text of `tab`, if it has one. Stub: none.
#[must_use]
pub fn about_for(_tab: TabId) -> Option<&'static About> {
    None
}

/// Draws `about` into `area`, labels resolved through `keys` for `stack`. Stub: draws nothing.
pub fn render(
    _frame: &mut Frame<'_>,
    _area: Rect,
    _about: &About,
    _keys: &Keys,
    _stack: Stack,
    _theme: &Theme,
) {
}
