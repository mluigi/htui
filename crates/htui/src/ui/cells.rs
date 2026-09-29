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
}
