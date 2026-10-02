//! The two measurements both text widgets draw by: how many terminal **cells** a string occupies,
//! and where its **grapheme cluster** boundaries are (MOD-54 D2).
//!
//! One module, private, so [`TextField`](crate::ui::TextField) and
//! [`TextArea`](crate::ui::TextArea) cannot disagree with each other — and so that neither can
//! disagree with `ratatui`, which is the thing actually drawing the result. Every cell count in
//! `ui/` comes from [`cell_width`]; a widget that counts columns any other way drifts from the
//! renderer by exactly the difference, and the cursor highlight lands on the wrong cell.
//!
//! **Two rules a reader must not break.** First, a cluster's width is the width of the cluster's
//! **own string**: `UnicodeWidthStr::width`, never a sum of `UnicodeWidthChar::width` over the code
//! points. A family emoji is one cluster of five code points; summed per `char` it is 6, and
//! measured as a string it is 2. The crate documents the same asymmetry for `"\r\n"` and for emoji
//! modifier and presentation sequences. Second, a per-`char` sum is *also* simply wrong for control
//! characters: `UnicodeWidthChar::width('\u{1}')` is `None` while `UnicodeWidthStr::width("\u{1}")`
//! is `1`, so the natural `.unwrap_or(0)` undercounts every C0 control and `DEL` by one — and a body
//! that came back from `$EDITOR` can hold those.
//!
//! The non-CJK `width()` is used, never `width_cjk()`. East Asian **Ambiguous** characters — `…`,
//! `•`, `U+FFFD`, every box-drawing char — are 1 cell under `width()` and 2 under `width_cjk()`,
//! and `…` and `•` are two of the glyphs `TextField` draws itself. `ratatui` uses `width()`.
//!
//! Every clip, pad, fit and wrap in `ui/` goes through the operations below (MOD-60 D1), so a row
//! is never measured one way and drawn another: [`clip`], [`pad`], [`pad_left`], [`fit`],
//! [`wrap`] and [`clip_spans`], all cutting at [`ELLIPSIS`].

use std::borrow::Cow;

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr;

/// The halfwidth katakana voiced sound mark, which `unicode-width` reports as zero-width
/// `Grapheme_Extend` while terminals draw it as an independent halfwidth cell.
const VOICED_SOUND_MARK: char = '\u{FF9E}';
/// Its semi-voiced counterpart, `U+FF9F`.
const SEMI_VOICED_SOUND_MARK: char = '\u{FF9F}';

/// How many terminal cells `s` occupies.
///
/// `ratatui` measures a cell the same way and then adds one per halfwidth sound mark, because
/// `unicode-width` calls those two marks zero-width while terminals draw them as cells
/// (`ratatui-core-0.1.2/src/buffer/cell_width.rs:34-46`, citing Ruby reline #832 and Microsoft
/// Terminal #18087). Replicating that is the point of OQ-1: if this function disagreed with the
/// renderer, every halfwidth-katakana line would be measured a cell short of how it is drawn.
///
/// The marks are counted **inside the string being measured**. `ratatui` counts over a whole line
/// unconditionally, which gives `"あﾞ"` 3; because both marks are `Grapheme_Extend` they always
/// attach to the preceding cluster, so counting per cluster gives the same answer. The test below
/// pins `"あﾞ"` at 3 so that equivalence is tested rather than assumed.
#[must_use]
pub(crate) fn cell_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
        + s.chars()
            .filter(|c| matches!(*c, VOICED_SOUND_MARK | SEMI_VOICED_SOUND_MARK))
            .count()
}

/// `s` split into UAX #29 **extended** grapheme clusters — the boundaries a person perceives as one
/// character, and the boundaries `Left`, `Right`, `Backspace` and `Delete` must step by.
///
/// Extended, not legacy: legacy boundaries separate `"👨‍👩‍👧"` into three code points and split the
/// combining sequence in `"a\u{301}b"`, which is the defect this item exists to remove. That is also
/// what `unicode-segmentation` recommends.
pub(crate) fn graphemes(s: &str) -> impl Iterator<Item = &str> + '_ {
    s.graphemes(true)
}

/// The one cut mark (MOD-60 D2): East Asian Ambiguous, so 1 cell under `width()` (see the module
/// doc).
pub(crate) const ELLIPSIS: char = '\u{2026}';

/// `text` with every control cluster drawn as one blank cell (MOD-60 D3).
///
/// `cell_width("\u{1}")` is 1, but `ratatui` skips a control grapheme when it draws, so an
/// unflattened control is measured one way and drawn another. Per **cluster**, not per `char`:
/// `"\r\n"` is one cluster and becomes one space. UAX #29 always isolates controls (GB4/GB5), so
/// a cluster holding a control is nothing but control. Borrowed when there is nothing to rewrite.
fn flatten(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        graphemes(text)
            .map(|cluster| {
                if cluster.chars().any(char::is_control) {
                    " "
                } else {
                    cluster
                }
            })
            .collect(),
    )
}

/// `text` in at most `width` cells, cut at a grapheme boundary with [`ELLIPSIS`] as its last cell
/// when anything was cut (MOD-60 D1, D2). A control character draws as one blank cell (D3) —
/// also when nothing is cut. Width 0 is `""`: a lone `…` would be a cell over. A wide cluster that
/// would straddle the cut is dropped, so a cut result may be one cell short; never one long.
#[allow(
    dead_code,
    reason = "MOD-60 T0 lands before its callers (T1-T4); T5 deletes this attribute"
)]
#[must_use]
pub(crate) fn clip(text: &str, width: usize) -> String {
    let flat = flatten(text);
    if cell_width(&flat) <= width {
        return flat.into_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = head(&flat, width - 1);
    out.push(ELLIPSIS);
    out
}

/// The leading clusters of `text` that fit in `room` cells, stopping at the **first** that does
/// not — a narrower cluster behind it is not looked for, so nothing is reordered.
fn head(text: &str, room: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for cluster in graphemes(text) {
        let cells = cell_width(cluster);
        if used + cells > room {
            break;
        }
        used += cells;
        out.push_str(cluster);
    }
    out
}

/// `text` followed by spaces up to `width` cells (MOD-60 D1). Never clips, never rewrites:
/// a `text` already `width` or wider comes back unchanged.
#[allow(
    dead_code,
    reason = "MOD-60 T0 lands before its callers (T1-T4); T5 deletes this attribute"
)]
#[must_use]
pub(crate) fn pad(text: &str, width: usize) -> String {
    let mut out = text.to_owned();
    out.push_str(&" ".repeat(width.saturating_sub(cell_width(text))));
    out
}

/// Spaces then `text`, right-aligned in `width` cells — `{:>N}` measured in cells (MOD-60 D1).
/// Never clips.
#[allow(
    dead_code,
    reason = "MOD-60 T0 lands before its callers (T1-T4); T5 deletes this attribute"
)]
#[must_use]
pub(crate) fn pad_left(text: &str, width: usize) -> String {
    let mut out = " ".repeat(width.saturating_sub(cell_width(text)));
    out.push_str(text);
    out
}

/// Exactly `width` cells: [`clip`] then [`pad`], so a straddling wide cluster's lost cell is
/// padded back and the next column stays put (MOD-60 D1, R-3).
#[allow(
    dead_code,
    reason = "MOD-60 T0 lands before its callers (T1-T4); T5 deletes this attribute"
)]
#[must_use]
pub(crate) fn fit(text: &str, width: usize) -> String {
    pad(&clip(text, width), width)
}

/// `line` in rows of at most `width` cells (MOD-60 D4, from `divergence::wrap_row`): broken at a
/// space where one fits, inside a word by grapheme only when the word alone is wider; a cluster
/// wider than the whole width sits alone on its row (the one overflow). An empty line is one
/// empty row; leading spaces are kept; a space that does not fit is the break. Width 0 reads as 1.
/// Control clusters draw as one blank cell (D3); only U+0020 splits words.
#[must_use]
pub(crate) fn wrap(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = vec![String::new()];
    for (at, raw) in line.split(' ').enumerate() {
        // Flattened before either push path, so the fits-after-a-space one draws a control as
        // a blank cell too (MOD-60 B8). Widths do not move: a control cluster is 1 cell either
        // way.
        let word = flatten(raw);
        // Re-measured rather than kept as a running sum: it measures what is drawn.
        let used = rows.last().map_or(0, |row| cell_width(row));
        if at > 0 {
            if used + 1 + cell_width(&word) <= width {
                if let Some(row) = rows.last_mut() {
                    row.push(' ');
                    row.push_str(&word);
                }
                continue;
            }
            if word.is_empty() {
                // A space that does not fit is the break itself.
                continue;
            }
            if used > 0 {
                rows.push(String::new());
            }
        }
        for cluster in graphemes(&word) {
            let cells = cell_width(cluster);
            if rows
                .last()
                .is_some_and(|row| !row.is_empty() && cell_width(row) + cells > width)
            {
                rows.push(String::new());
            }
            if let Some(row) = rows.last_mut() {
                row.push_str(cluster);
            }
        }
    }
    rows
}

/// `spans` cut from the end to at most `width` cells, every kept span keeping its style; when
/// anything was cut, [`ELLIPSIS`] ends the line in the style of the span the cut fell in (B1).
/// Controls flattened per span (D3). No padding. Width 0 is no spans.
#[allow(
    dead_code,
    reason = "MOD-60 T0 lands before its callers (T1-T4); T5 deletes this attribute"
)]
#[must_use]
pub(crate) fn clip_spans(spans: &[Span<'_>], width: usize) -> Vec<Span<'static>> {
    let flat: Vec<(Cow<'_, str>, Style)> = spans
        .iter()
        .map(|span| (flatten(&span.content), span.style))
        .collect();
    if flat.iter().map(|(text, _)| cell_width(text)).sum::<usize>() <= width {
        return flat
            .into_iter()
            .map(|(text, style)| Span::styled(text.into_owned(), style))
            .collect();
    }
    if width == 0 {
        return Vec::new();
    }
    let mut room = width - 1;
    let mut out = Vec::new();
    for (text, style) in flat {
        let cells = cell_width(&text);
        if cells <= room {
            room -= cells;
            out.push(Span::styled(text.into_owned(), style));
            continue;
        }
        // The cut span: the first not kept whole. Its head plus the mark, which may be the mark
        // alone when the cut lands on a span boundary (B1).
        let mut cut = head(&text, room);
        cut.push(ELLIPSIS);
        out.push(Span::styled(cut, style));
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    #[test]
    fn ascii_and_the_widgets_own_glyphs_are_one_cell() {
        assert_eq!(cell_width(""), 0);
        for one in [
            "a", "…", "•", "\u{2500}", "\u{FFFD}", "\u{2400}", "\u{2421}",
        ] {
            assert_eq!(cell_width(one), 1, "{one:?}");
        }
    }

    #[test]
    fn a_wide_grapheme_is_two_cells() {
        assert_eq!(cell_width("一"), 2);
        assert_eq!(
            cell_width("⌨️"),
            2,
            "VS16 flips the base to emoji presentation"
        );
        assert_eq!(cell_width("⌨"), 1, "and the bare base is not");
    }

    #[test]
    fn a_zero_width_cluster_is_zero_cells() {
        assert_eq!(cell_width("\u{301}"), 0, "a lone combining mark");
        assert_eq!(cell_width("\u{200d}"), 0, "a lone ZWJ");
    }

    /// OQ-1, answered at the CONFIRM gate: replicate `ratatui`'s halfwidth sound mark adjustment,
    /// so the widget's arithmetic and the renderer's agree by construction.
    #[test]
    fn a_halfwidth_sound_mark_adds_a_cell() {
        assert_eq!(
            UnicodeWidthStr::width("ｶﾞ"),
            1,
            "unicode-width alone says one"
        );
        assert_eq!(
            cell_width("ｶﾞ"),
            2,
            "and a terminal draws two, as ratatui counts it"
        );
        assert_eq!(
            cell_width("あ"),
            2,
            "a fullwidth kana, untouched by the rule"
        );
    }

    /// The adjustment is unconditional in `ratatui` — it counts the mark wherever it appears, so
    /// `"あﾞ"` is 3 and not 2. Counting inside the cluster gives the same answer because the mark is
    /// `Grapheme_Extend`; this pins the equivalence.
    #[test]
    fn the_sound_mark_count_is_the_same_whole_or_per_cluster() {
        assert_eq!(cell_width("あﾞ"), 3);
        assert_eq!(
            "あ"
                .graphemes(true)
                .chain("ﾞ".graphemes(true))
                .map(cell_width)
                .sum::<usize>(),
            cell_width("あﾞ"),
        );
    }

    /// D2b, and the trap R-5 names. `width_cjk` is available without declaring anything —
    /// `unicode-width`'s `cjk` feature is default-on and `ratatui` does not disable it — and it
    /// doubles every East Asian Ambiguous character, three of which this widget draws itself.
    #[test]
    fn the_non_cjk_width_is_the_one_we_use() {
        for ambiguous in ["…", "•", "\u{fffd}", "\u{2500}", "\u{2502}"] {
            assert_eq!(cell_width(ambiguous), 1, "{ambiguous:?}");
            assert_eq!(
                UnicodeWidthStr::width_cjk(ambiguous),
                2,
                "{ambiguous:?} under width_cjk"
            );
        }
    }

    #[test]
    fn a_cluster_is_split_on_user_perceived_boundaries() {
        assert_eq!(
            graphemes("a\u{301}").count(),
            1,
            "base + combining mark is one"
        );
        assert_eq!(
            graphemes("👨‍👩‍👧").count(),
            1,
            "a family emoji is one, not three"
        );
        assert_eq!(graphemes("a\r\nb").collect::<Vec<_>>(), ["a", "\r\n", "b"]);
        assert_eq!(
            graphemes("\u{1}").count(),
            1,
            "a C0 control is its own cluster"
        );
        assert_eq!(graphemes("").count(), 0);
    }

    /// D15's rule, as a test rather than a comment: measure the string, never sum per `char`.
    #[test]
    fn a_cluster_is_measured_as_its_own_string() {
        let family = "👨‍👩‍👧";
        let summed: usize = family
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        assert_eq!(cell_width(family), 2);
        assert_eq!(summed, 6, "which is why the per-char sum is not an option");
    }

    /// The same asymmetry, for the control characters a `$EDITOR` body can hold: the `char` API has
    /// no answer, so summing per `char` with `unwrap_or(0)` loses a cell each.
    #[test]
    fn a_control_char_is_wider_than_a_char_sum_says() {
        for control in ["\u{1}", "\u{7f}", "\t", "\r\n"] {
            let summed: usize = control
                .chars()
                .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                .sum();
            assert_eq!(cell_width(control), 1, "{control:?}");
            assert_ne!(summed, cell_width(control), "{control:?} sums to {summed}");
        }
    }

    /// The widgets rebuild a drawn `String` per cluster, and `ratatui` re-splits what it is given.
    /// That only round-trips if clustering is the same operation over the concatenation.
    #[test]
    fn clusters_rejoin_into_the_string_they_came_from() {
        for s in ["👨‍👩‍👧", "a\tb", "a\u{1}b", "漢字\t字", "", "\r\n", "e\u{301}x"] {
            assert_eq!(graphemes(s).collect::<String>(), s, "{s:?}");
        }
    }

    // -----------------------------------------------------------------------------------------
    // The shared operations (MOD-60 D1-D4, D12): one table per op.
    // -----------------------------------------------------------------------------------------

    use ratatui::style::{Color, Modifier};

    /// Three wide CJK characters: 6 cells.
    const CJK: &str = "\u{6f22}\u{5b57}\u{6587}";
    /// A ZWJ family: one cluster of five code points, 2 cells.
    const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    /// A base and a combining mark: one cluster, 1 cell.
    const COMBINING: &str = "e\u{301}";
    /// A halfwidth katakana and its voiced sound mark: one cluster, 2 cells.
    const SOUND: &str = "\u{ff76}\u{ff9e}";

    /// Every input the clip and fit tables use, for the properties over all widths.
    fn table_inputs() -> Vec<String> {
        vec![
            "abcdef".to_owned(),
            CJK.to_owned(),
            FAMILY.repeat(2),
            COMBINING.repeat(3),
            SOUND.repeat(2),
            "a\u{1}b".to_owned(),
            "a\r\nb".to_owned(),
            "ab\ncd".to_owned(),
            "\t".to_owned(),
            "ab".to_owned(),
            "abcd".to_owned(),
            "abcde".to_owned(),
            "a\nb".to_owned(),
            "\u{2014}".to_owned(),
            "abc".to_owned(),
            String::new(),
        ]
    }

    #[test]
    fn the_ellipsis_is_one_cell() {
        assert_eq!(cell_width(&ELLIPSIS.to_string()), 1);
    }

    #[test]
    fn clip_cuts_by_cells_and_marks_the_cut() {
        let family2 = FAMILY.repeat(2);
        let combining3 = COMBINING.repeat(3);
        let sound2 = SOUND.repeat(2);
        let cases: Vec<(&str, usize, String, usize)> = vec![
            ("abcdef", 6, "abcdef".to_owned(), 6),
            ("abcdef", 5, "abcd\u{2026}".to_owned(), 5),
            ("abcdef", 1, "\u{2026}".to_owned(), 1),
            ("abcdef", 0, String::new(), 0),
            (CJK, 6, CJK.to_owned(), 6),
            (CJK, 5, "\u{6f22}\u{5b57}\u{2026}".to_owned(), 5),
            (CJK, 4, "\u{6f22}\u{2026}".to_owned(), 3),
            (CJK, 3, "\u{6f22}\u{2026}".to_owned(), 3),
            (CJK, 2, "\u{2026}".to_owned(), 1),
            (CJK, 1, "\u{2026}".to_owned(), 1),
            (CJK, 0, String::new(), 0),
            (&family2, 4, family2.clone(), 4),
            (&family2, 3, format!("{FAMILY}\u{2026}"), 3),
            (&family2, 2, "\u{2026}".to_owned(), 1),
            (&combining3, 3, combining3.clone(), 3),
            (&combining3, 2, "e\u{301}\u{2026}".to_owned(), 2),
            (&combining3, 1, "\u{2026}".to_owned(), 1),
            (&sound2, 4, sound2.clone(), 4),
            (&sound2, 3, format!("{SOUND}\u{2026}"), 3),
            (&sound2, 2, "\u{2026}".to_owned(), 1),
            ("a\u{1}b", 3, "a b".to_owned(), 3),
            ("a\r\nb", 3, "a b".to_owned(), 3),
            ("ab\ncd", 3, "ab\u{2026}".to_owned(), 3),
            ("\t", 1, " ".to_owned(), 1),
        ];
        for (text, width, want, cells) in cases {
            let out = clip(text, width);
            assert_eq!(out, want, "{text:?} at {width}");
            assert_eq!(cell_width(&out), cells, "{out:?} at {width}");
        }
    }

    #[test]
    fn clip_is_never_wider_than_its_width() {
        for input in table_inputs() {
            for w in 0..=8 {
                let out = clip(&input, w);
                assert!(cell_width(&out) <= w, "{out:?} against {w}");
                if cell_width(&input) <= w {
                    assert_eq!(out, flatten(&input), "{input:?} fits {w}");
                }
            }
        }
    }

    #[test]
    fn pad_fills_to_the_width_and_never_clips() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("\u{6f22}", 4), "\u{6f22}  ");
        assert_eq!(pad(FAMILY, 3), format!("{FAMILY} "));
        assert_eq!(pad(SOUND, 3), format!("{SOUND} "));
        assert_eq!(pad(COMBINING, 2), "e\u{301} ");
        assert_eq!(pad("abcdef", 4), "abcdef", "never clips");
        assert_eq!(pad("", 0), "");
        assert_eq!(pad("a\u{1}", 3), "a\u{1} ", "no rewrite; measured 2");
    }

    #[test]
    fn pad_left_right_aligns_by_cells() {
        assert_eq!(pad_left("ab", 4), "  ab");
        assert_eq!(pad_left("\u{6f22}", 3), " \u{6f22}");
        assert_eq!(pad_left("abc", 2), "abc", "never clips");
    }

    #[test]
    fn fit_is_exactly_its_width() {
        // Ported from `runs::fit_pads_cuts_and_flattens`.
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcd", 4), "abcd");
        assert_eq!(fit("abcde", 4), "abc\u{2026}");
        assert_eq!(fit("a\nb", 3), "a b");
        assert_eq!(
            fit("\u{2014}", 2),
            "\u{2014} ",
            "counted in chars, not bytes"
        );
        assert_eq!(fit("abc", 0), "");
        // The straddling wide cluster's lost cell is padded back (R-3).
        assert_eq!(fit(CJK, 4), "\u{6f22}\u{2026} ");
        assert_eq!(fit(CJK, 5), "\u{6f22}\u{5b57}\u{2026}");
        assert_eq!(fit(&FAMILY.repeat(2), 2), "\u{2026} ");
        assert_eq!(fit(&SOUND.repeat(2), 2), "\u{2026} ");
        for input in table_inputs() {
            for w in 0..=8 {
                let out = fit(&input, w);
                assert_eq!(cell_width(&out), w, "{out:?} against {w}");
            }
        }
    }

    #[test]
    fn wrap_breaks_at_spaces_and_inside_only_a_wider_word() {
        // Ported from `runs::wrap_line_breaks_at_spaces_and_inside_only_a_wider_word`.
        assert_eq!(wrap("", 5), [""]);
        assert_eq!(wrap("ab cd ef", 5), ["ab cd", "ef"]);
        assert_eq!(wrap("  ab", 5), ["  ab"], "an indent is kept");
        assert_eq!(wrap("abcdefgh ij", 3), ["abc", "def", "gh", "ij"]);
        assert_eq!(
            wrap("abc ", 3),
            ["abc"],
            "a space that does not fit is the break"
        );
        assert!(
            wrap(&"word ".repeat(40), 7)
                .iter()
                .all(|row| cell_width(row) <= 7)
        );
    }

    #[test]
    fn no_wrapped_row_is_wider_than_its_width() {
        // Ported from `divergence::no_wrapped_row_is_wider_than_its_column`.
        for line in [
            format!("+{}\u{7d42}", "\u{6f22}".repeat(45)),
            format!("+{} end", "\u{1f642}".repeat(30)),
            " ascii then \u{6f22}\u{5b57} and a\u{301} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"
                .to_owned(),
            "\u{1}\u{7f}x".to_owned(),
        ] {
            for width in [1, 2, 3, 7, 20, 48] {
                let rows = wrap(&line, width);
                // A wide cluster alone on a row is the one overflow a 1-cell column allows.
                let room = width.max(2);
                for row in &rows {
                    let cells = cell_width(row);
                    assert!(cells <= room, "{row:?} is {cells} cells at {width}");
                }
                let kept: String = rows.concat().chars().filter(|c| *c != ' ').collect();
                let wanted: String = line
                    .chars()
                    .filter(|c| *c != ' ' && !c.is_control())
                    .collect();
                assert_eq!(kept, wanted, "nothing dropped from {line:?} at {width}");
            }
        }
    }

    #[test]
    fn wrap_keeps_a_cluster_whole() {
        let han = "\u{6f22}";
        assert_eq!(
            wrap(&han.repeat(5), 4),
            [han.repeat(2), han.repeat(2), han.to_owned()]
        );
        assert_eq!(
            wrap(&han.repeat(3), 1),
            [han, han, han],
            "a cluster wider than the width sits alone: the one overflow"
        );
        assert_eq!(
            wrap(&FAMILY.repeat(3), 4),
            [FAMILY.repeat(2), FAMILY.to_owned()]
        );
        assert_eq!(wrap(FAMILY, 1), [FAMILY]);
        assert_eq!(
            wrap(&COMBINING.repeat(3), 2),
            [COMBINING.repeat(2), COMBINING.to_owned()]
        );
        assert_eq!(
            wrap(&SOUND.repeat(3), 4),
            [SOUND.repeat(2), SOUND.to_owned()]
        );
        assert_eq!(wrap("ab", 0), ["a", "b"], "width 0 reads as 1");
    }

    #[test]
    fn wrap_draws_a_control_char_as_a_blank_cell() {
        assert_eq!(wrap("a\u{1}b c", 80), ["a b c"]);
        assert_eq!(
            wrap("x \u{1}y", 80),
            ["x  y"],
            "flattened on the fits-after-a-space path too (B8)"
        );
        assert_eq!(wrap("a\tb", 80), ["a b"], "a tab is not a break");
    }

    fn style_a() -> Style {
        Style::new().fg(Color::Red)
    }

    fn style_b() -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }

    fn spans(parts: &[(&str, Style)]) -> Vec<Span<'static>> {
        parts
            .iter()
            .map(|(text, style)| Span::styled((*text).to_owned(), *style))
            .collect()
    }

    #[test]
    fn clip_spans_cuts_from_the_end_and_keeps_styles() {
        let (a, b) = (style_a(), style_b());
        let input = spans(&[("ab", a), ("cd", b)]);
        for w in [4, 5, 9] {
            assert_eq!(clip_spans(&input, w), input, "fits at {w}");
        }
        assert_eq!(clip_spans(&input, 2), spans(&[("a\u{2026}", a)]));
        assert_eq!(clip_spans(&input, 1), spans(&[("\u{2026}", a)]));
        assert_eq!(clip_spans(&input, 0), Vec::<Span<'static>>::new());
        assert_eq!(
            clip_spans(&spans(&[("a\nb", a)]), 3),
            spans(&[("a b", a)]),
            "flattened, and it fits"
        );
        let with_empty = spans(&[("", a), ("ab", b)]);
        assert_eq!(clip_spans(&with_empty, 2), with_empty, "empty span kept");
    }

    #[test]
    fn clip_spans_puts_the_ellipsis_in_the_style_of_the_span_it_cuts() {
        let (a, b) = (style_a(), style_b());
        let input = spans(&[("ab", a), ("cd", b)]);
        assert_eq!(
            clip_spans(&input, 3),
            spans(&[("ab", a), ("\u{2026}", b)]),
            "a cut on a span boundary marks the first dropped span (B1)"
        );
    }

    #[test]
    fn clip_spans_drops_a_wide_cluster_that_would_straddle_the_cut() {
        let (a, b) = (style_a(), style_b());
        let input = spans(&[("\u{6f22}", a), ("\u{5b57}\u{6587}", b)]);
        let five = clip_spans(&input, 5);
        assert_eq!(five, spans(&[("\u{6f22}", a), ("\u{5b57}\u{2026}", b)]));
        let four = clip_spans(&input, 4);
        assert_eq!(four, spans(&[("\u{6f22}", a), ("\u{2026}", b)]));
        for (out, w) in [(five, 5), (four, 4)] {
            let cells: usize = out.iter().map(|span| cell_width(&span.content)).sum();
            assert!(cells <= w, "{out:?} against {w}");
        }
    }
}
