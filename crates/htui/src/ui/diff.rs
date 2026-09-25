//! Line diffs as the TUI draws them (MOD-9 D12): the one gutter style the chat transcript and the
//! Skills tab's Templates view share.

use ratatui::style::Style;

use crate::ui::Theme;

/// `+` accents, `-` errors, the `---`/`+++` headers and everything else dim — the two gutters a
/// reader looks for. Moved here from the chat transcript (D12), byte for byte, so its frames do not
/// move.
#[must_use]
pub fn diff_style(line: &str, theme: &Theme) -> Style {
    if line.starts_with("+++") || line.starts_with("---") {
        theme.dim
    } else if line.starts_with('+') {
        theme.accent
    } else if line.starts_with('-') {
        theme.error
    } else {
        theme.dim
    }
}
