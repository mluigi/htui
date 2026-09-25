//! Line diffs as the TUI draws them (MOD-9 D12): `similar`'s unified diff, and the one gutter
//! style the chat transcript and the Skills tab's Templates view share.
//!
//! Not `htui_agent::acp::fs::unified_diff`: that one prints `a/<path>`/`b/<path>` headers for a
//! file an agent proposed to write, and the Skills tab has no business reaching into an ACP module
//! for a version-to-version diff (plan D12).

use ratatui::style::Style;
use ratatui::text::Line;

use crate::ui::Theme;

/// What [`lines`] draws for two identical texts, whose unified diff is empty.
pub const NO_DIFFERENCES: &str = "no differences";

/// `similar`'s unified diff of `old` → `new`: three lines of context and `---`/`+++` headers
/// carrying the two labels, `acp::fs`'s shape. Empty when the texts are identical: `similar` writes
/// the headers with the first hunk, and there is none.
#[must_use]
pub fn unified(old: &str, new: &str, old_label: &str, new_label: &str) -> String {
    let _ = (old, new, old_label, new_label);
    todo!("MOD-9 T5: the unified diff")
}

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

/// One line per line of `diff`, styled by [`diff_style`]; an empty diff is the single dim line
/// [`NO_DIFFERENCES`].
#[must_use]
pub fn lines(diff: &str, theme: &Theme) -> Vec<Line<'static>> {
    let _ = (diff, theme);
    todo!("MOD-9 T5: the styled diff lines")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_texts_render_no_differences() {
        let theme = Theme::default();
        let text = "one\ntwo\n";
        let diff = unified(text, text, "v1", "v2");
        assert_eq!(diff, "", "no hunk, so no header either");
        let lines = lines(&diff, &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].to_string(), NO_DIFFERENCES);
        assert_eq!(lines[0].style, theme.dim);
    }

    #[test]
    fn an_added_line_is_accented() {
        let theme = Theme::default();
        let diff = unified("one\ntwo\n", "one\nadded\ntwo\n", "v1", "v2");
        assert!(
            diff.starts_with("--- v1\n+++ v2\n@@ "),
            "headers first: {diff}"
        );
        let lines = lines(&diff, &theme);
        let added = lines
            .iter()
            .find(|line| line.to_string() == "+added")
            .unwrap_or_else(|| panic!("no `+added` line in {diff}"));
        assert_eq!(added.style, theme.accent);
        let header = lines
            .iter()
            .find(|line| line.to_string() == "+++ v2")
            .expect("the new header");
        assert_eq!(header.style, theme.dim, "a header is not an added line");
        let context = lines
            .iter()
            .find(|line| line.to_string() == " one")
            .expect("a context line");
        assert_eq!(context.style, theme.dim);
    }
}
