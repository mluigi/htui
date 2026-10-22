//! Plain text for panes (MOD-115 M3 D2, D4): [`wrapped`], the one word-wrap every tab's
//! measured layout uses, and [`notice`], the one-line notice a section shows under its content.
//!
//! [`wrapped`] is here rather than relying on `Paragraph`'s own `Wrap`, because the layout needs
//! the height *before* the pane is drawn and a count that disagreed with the widget's wrapping
//! would clip the last line of a warning — the one line that says nothing can be undone.
//!
//! It was promoted out of the Settings tab, which shared it with the hierarchy, kinds, prompt,
//! connection, qdrant and boxes sections and lent it to the backlog, requirements and skills tabs.
//! M5 O-5 argued a three-line helper was cheaper to copy than to promote; the third copy was the
//! price it named, so it was promoted. The rows are `cells::wrap`'s (MOD-60 D5, B7), so there is
//! one break rule in `ui/`; `split_whitespace` drops the whitespace controls, and any other
//! control stays inside its word for `cells::wrap` to flatten. MOD-115 M3 moved it into the kit,
//! so no tab imports another tab for it.

use ratatui::text::Line;

use crate::ui::{Theme, cells};

/// `text` in rows of at most `width` cells, its whitespace collapsed: words joined by one space,
/// a word wider than `width` broken by grapheme (MOD-60 D5, OQ-1) and the next word joining its
/// last piece when it fits (B7); a control character draws as one blank cell (D3). Text with no
/// words is no rows. Width 0 reads as 1.
#[must_use]
pub fn wrapped(text: &str, width: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    cells::wrap(&words.join(" "), width)
}

/// What a notice line is: something to read, or something the user has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Read and move on: `theme.dim`.
    Info,
    /// Has to be acted on: `theme.error`.
    Error,
}

/// `text` as one notice line in its `tone`'s role, unwrapped (the pane clips it). The kit's
/// notice row a section draws under its content (MOD-115 M3 D4).
#[must_use]
pub fn notice(text: &str, tone: Tone, theme: &Theme) -> Line<'static> {
    let style = match tone {
        Tone::Info => theme.dim,
        Tone::Error => theme.error,
    };
    Line::styled(text.to_owned(), style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::cells::cell_width;
    use ratatui::style::Modifier;

    /// A ZWJ family: one cluster of five code points, 2 cells.
    const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";

    #[test]
    fn ascii_text_wraps_as_it_did() {
        assert_eq!(wrapped("ab cd ef", 5), ["ab cd", "ef"]);
        assert_eq!(
            wrapped("  a \t b\n c ", 80),
            ["a b c"],
            "whitespace collapsed"
        );
        assert!(wrapped("", 9).is_empty());
        assert!(wrapped("   ", 9).is_empty(), "no words is no rows");
    }

    #[test]
    fn a_wide_sentence_wraps_by_cells() {
        let han = "\u{6f22}";
        let rows = wrapped(&han.repeat(10), 7);
        assert_eq!(
            rows,
            [han.repeat(3), han.repeat(3), han.repeat(3), han.to_owned()]
        );
        for row in &rows {
            assert!(cell_width(row) <= 7, "{row:?} against 7");
        }
    }

    #[test]
    fn a_word_wider_than_the_width_breaks_inside() {
        assert_eq!(
            wrapped("see abcdefghijkl now", 5),
            ["see", "abcde", "fghij", "kl", "now"]
        );
        assert_eq!(
            wrapped("abcdefg hi", 5),
            ["abcde", "fg hi"],
            "the next word joins the broken word's last piece (B7)"
        );
    }

    /// Arabic "salam" holds a lam-alef, which `unicode-width` measures as one ligature cell across
    /// two clusters; `ratatui` draws it as two, so the word is 4 cells and two of them need 9.
    #[test]
    fn a_ligature_is_measured_as_drawn() {
        let salam = "\u{633}\u{644}\u{627}\u{645}";
        assert_eq!(wrapped(&format!("{salam} {salam}"), 7), [salam, salam]);
        assert_eq!(
            wrapped(&format!("{salam} {salam}"), 9),
            [format!("{salam} {salam}")]
        );
    }

    #[test]
    fn a_cluster_is_never_split_across_rows() {
        assert_eq!(wrapped(&FAMILY.repeat(4), 3), [FAMILY; 4]);
        assert_eq!(
            wrapped("x\u{1}y", 80),
            ["x y"],
            "a control draws as one blank cell"
        );
    }

    #[test]
    fn a_notice_is_one_unwrapped_line_in_its_tones_role() {
        let theme = Theme::default();
        let long = "word ".repeat(40);
        let info = notice(&long, Tone::Info, &theme);
        assert_eq!(info.to_string(), long, "not wrapped, not trimmed");
        assert_eq!(info.style, theme.dim);
        let error = notice("deleted elsewhere", Tone::Error, &theme);
        assert_eq!(error.to_string(), "deleted elsewhere");
        assert_eq!(error.style, theme.error);
    }

    /// Under the monochrome theme the two tones still differ, by modifier alone.
    #[test]
    fn the_tones_differ_without_colour() {
        let theme = Theme::monochrome();
        let info = notice("x", Tone::Info, &theme).style;
        let error = notice("x", Tone::Error, &theme).style;
        assert_eq!(info.fg, None);
        assert_eq!(error.fg, None);
        assert!(info.add_modifier.contains(Modifier::DIM), "{info:?}");
        assert!(!error.add_modifier.contains(Modifier::DIM), "{error:?}");
        assert_ne!(info, error);
    }
}
