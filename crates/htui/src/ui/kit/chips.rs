//! Key chips and the colour legend (MOD-115). Labels arrive pre-resolved from
//! [`Keys`](crate::keys::Keys); nothing here hard-codes a chord.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::Theme;

/// Between a chip's word and the next chip.
const GAP: &str = "  ";

/// Between two legend entries.
const LEGEND_GAP: &str = " \u{b7} ";

/// The style of a key chip: the key role, reversed, so a chip reads as a keycap with or without
/// colour (the reverse is a modifier, so it survives `NO_COLOR`).
fn chip_style(theme: &Theme) -> Style {
    theme.key.add_modifier(Modifier::REVERSED)
}

/// One key as a chip: its label padded by a cell on each side, reversed.
#[must_use]
pub fn key_chip(key: &str, theme: &Theme) -> Span<'static> {
    Span::styled(format!(" {key} "), chip_style(theme))
}

/// `(key label, action word)` pairs as one line of chips: each chip, a space, its word in the
/// dim role, two spaces between pairs. A pair with no key label is skipped, so an unbound
/// action never shows a bare word.
#[must_use]
pub fn chip_line(pairs: &[(String, &str)], theme: &Theme) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(pairs.len() * 3);
    for (key, word) in pairs.iter().filter(|(key, _)| !key.is_empty()) {
        if !spans.is_empty() {
            spans.push(Span::raw(GAP));
        }
        spans.push(key_chip(key, theme));
        spans.push(Span::styled(format!(" {word}"), theme.dim));
    }
    Line::from(spans)
}

/// A view's keys as the shell's hint row draws them (MOD-67 M6 T2, every tab's
/// [`Tab::key_hint`](crate::ui::tabs::Tab::key_hint)): `theme.dim`, `None` when no key of the
/// spec is bound. Moved from the Settings tab (MOD-115 M3 D2), which lent it to every tab.
#[must_use]
pub fn key_line(text: String, theme: &Theme) -> Option<Line<'static>> {
    (!text.is_empty()).then(|| Line::styled(text, theme.dim))
}

/// A legend of `(word, style)` entries: each word drawn in its own style, separated by a dim
/// middle dot. The styles are the caller's (a status's, a kind's), so the legend can never
/// disagree with the rows it explains.
#[must_use]
pub fn legend(entries: &[(&str, Style)], theme: &Theme) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(entries.len() * 2);
    for (word, style) in entries {
        if !spans.is_empty() {
            spans.push(Span::styled(LEGEND_GAP, theme.dim));
        }
        spans.push(Span::styled((*word).to_owned(), *style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_key_chip_pads_its_label_by_a_cell_each_side() {
        let chip = key_chip("F9", &Theme::default());
        assert_eq!(chip.content, " F9 ");
    }

    #[test]
    fn a_key_chip_is_reversed_so_it_reads_as_a_keycap() {
        let theme = Theme::default();
        let style = key_chip("?", &theme).style;
        assert!(style.add_modifier.contains(Modifier::REVERSED));
        assert!(style.add_modifier.contains(Modifier::BOLD), "the key role");
    }

    #[test]
    fn a_key_chip_names_no_colour_under_monochrome() {
        let theme = Theme::monochrome();
        let style = key_chip("F9", &theme).style;
        assert_eq!((style.fg, style.bg), (None, None));
        assert!(style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn a_chip_line_pairs_each_key_with_its_word() {
        let theme = Theme::default();
        let line = chip_line(
            &[("F9".to_owned(), "about"), ("?".to_owned(), "keys")],
            &theme,
        );
        assert_eq!(text(&line), " F9  about   ?  keys");
        assert_eq!(line.spans[0].style, chip_style(&theme));
        assert_eq!(line.spans[1].style, theme.dim, "the word is secondary");
    }

    #[test]
    fn a_chip_line_skips_an_unbound_action() {
        let theme = Theme::default();
        let line = chip_line(
            &[
                (String::new(), "ghost"),
                ("?".to_owned(), "keys"),
                (String::new(), "ghost"),
            ],
            &theme,
        );
        assert_eq!(text(&line), " ?  keys");
    }

    #[test]
    fn a_chip_line_of_nothing_is_empty() {
        assert!(chip_line(&[], &Theme::default()).spans.is_empty());
    }

    #[test]
    fn a_chip_line_names_no_colour_under_monochrome() {
        let theme = Theme::monochrome();
        let line = chip_line(&[("F9".to_owned(), "about")], &theme);
        for span in &line.spans {
            assert_eq!((span.style.fg, span.style.bg), (None, None), "{span:?}");
        }
    }

    #[test]
    fn a_legend_draws_each_word_in_its_own_style() {
        let theme = Theme::default();
        let line = legend(&[("ok", theme.added), ("bad", theme.error)], &theme);
        assert_eq!(text(&line), "ok \u{b7} bad");
        assert_eq!(line.spans[0].style, theme.added);
        assert_eq!(line.spans[1].style, theme.dim, "the separator is secondary");
        assert_eq!(line.spans[2].style, theme.error);
    }

    #[test]
    fn a_legend_of_one_entry_has_no_separator() {
        let theme = Theme::default();
        let line = legend(&[("ok", theme.added)], &theme);
        assert_eq!(text(&line), "ok");
    }

    #[test]
    fn a_legend_of_nothing_is_empty() {
        assert!(legend(&[], &Theme::default()).spans.is_empty());
    }

    #[test]
    fn a_key_line_is_the_hint_text_dimmed_and_nothing_when_empty() {
        let theme = Theme::default();
        let line = key_line("j/k move \u{b7} e edit".to_owned(), &theme).expect("a line");
        assert_eq!(line.to_string(), "j/k move \u{b7} e edit");
        assert_eq!(line.style, theme.dim);
        assert_eq!(key_line(String::new(), &theme), None, "no key bound");
        let mono = key_line("x".to_owned(), &Theme::monochrome()).expect("a line");
        assert_eq!(mono.style.fg, None);
        assert!(mono.style.add_modifier.contains(Modifier::DIM));
    }
}
