//! List parts: the `▌` gutter and selection band, ellipsis, group rows and the "hidden" note
//! (MOD-115).
//!
//! A list is a slice of [`Row`]s drawn one per terminal row. The first cell of every row is the
//! gutter: [`GUTTER`] in the pane's accent on the selected row, a blank on the others, so the
//! columns behind it never shift when the selection moves. The selected row's content is painted
//! in a band ([`Theme::select`]) that runs the full width, so a short row still reads as selected.
//! Under monochrome the band is reversed and the gutter keeps the accent's emphasis alone.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::Theme;
use crate::ui::cells;

/// The marker in front of the selected row.
pub const GUTTER: &str = "▌";

/// The glyph in front of an open group header.
const OPEN: &str = "▾";

/// The glyph in front of a folded group header.
const FOLDED: &str = "▸";

/// What a row is, so the renderer knows whether it can be selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A selectable item.
    Item,
    /// A group header, folded or open.
    Group {
        /// Whether the group's children are hidden.
        folded: bool,
    },
    /// A note row (such as "3 hidden"); never selected.
    Note,
}

/// One row of a list.
#[derive(Debug, Clone)]
pub struct Row<'a> {
    /// The row's content.
    pub line: Line<'a>,
    /// What kind of row it is.
    pub kind: RowKind,
}

/// Draws `rows` from `offset` into `area`, the `selected` one in the band.
///
/// Column 0 is the gutter; the rest is the row's content, cut at an [`ellipsize`]d edge. A
/// [`RowKind::Note`] is never selected: if `selected` names one it is drawn like any other note,
/// without gutter or band. Rows past the end of `rows`, and an `offset` past the end, draw
/// nothing; an `area` with no room for a gutter and a cell draws nothing.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    rows: &[Row<'_>],
    selected: Option<usize>,
    offset: usize,
    accent: Style,
    theme: &Theme,
) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let content_width = area.width - 1;
    for (slot, (index, row)) in rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(usize::from(area.height))
        .enumerate()
    {
        // `slot < area.height`, so it fits in a `u16`.
        let y = area
            .y
            .saturating_add(u16::try_from(slot).unwrap_or(u16::MAX));
        let on = selected == Some(index) && row.kind != RowKind::Note;
        let gutter = if on {
            Span::styled(GUTTER, accent)
        } else {
            Span::raw(" ")
        };
        frame.render_widget(
            Paragraph::new(Line::from(gutter)),
            Rect::new(area.x, y, 1, 1),
        );
        let line = if on {
            selection_band(row.line.clone(), content_width, theme)
        } else {
            ellipsize(row.line.clone(), content_width)
        };
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(area.x + 1, y, content_width, 1),
        );
    }
}

/// `line` cut to `width` cells by [`ellipsize`], padded with blanks to exactly `width`, and
/// painted with [`Theme::select`], so the band covers the whole row whatever the text's length.
#[must_use]
pub fn selection_band(line: Line<'_>, width: u16, theme: &Theme) -> Line<'static> {
    let mut line = ellipsize(line, width);
    let missing = usize::from(width).saturating_sub(line.width());
    if missing > 0 {
        line.spans.push(Span::raw(" ".repeat(missing)));
    }
    theme.select(line)
}

/// `line` cut to at most `width` cells, the last kept span ending in `…` when anything was cut.
/// Styles are kept; a control character draws as a blank.
#[must_use]
pub fn ellipsize(line: Line<'_>, width: u16) -> Line<'static> {
    cells::clip_line(line, width)
}

/// A group header: `▾ label (count)` when open, `▸ label (count)` when `folded`.
#[must_use]
pub fn group_row(label: &str, count: usize, folded: bool, theme: &Theme) -> Row<'static> {
    let glyph = if folded { FOLDED } else { OPEN };
    Row {
        line: Line::from(vec![
            Span::styled(glyph, theme.title),
            Span::styled(format!(" {label}"), theme.title),
            Span::styled(format!(" ({count})"), theme.dim),
        ]),
        kind: RowKind::Group { folded },
    }
}

/// The note that rows are filtered out: `3 closed hidden · c to show`. `noun` names what is
/// hidden, `key_label` is the chord that shows it, already resolved through
/// [`Keys`](crate::keys::Keys).
#[must_use]
pub fn hidden_note(count: usize, noun: &str, key_label: &str, theme: &Theme) -> Row<'static> {
    Row {
        line: Line::from(vec![
            Span::styled(format!("{count} {noun} hidden · "), theme.dim),
            Span::styled(key_label.to_owned(), theme.key),
            Span::styled(" to show", theme.dim),
        ]),
        kind: RowKind::Note,
    }
}

/// `offset` moved the least that brings `selected` into a viewport of `height` rows over
/// `total`. The list never scrolls past its end, so the last page stays full.
#[must_use]
pub fn scroll_into_view(selected: usize, offset: usize, height: usize, total: usize) -> usize {
    let last = total.saturating_sub(height);
    if selected < offset {
        selected.min(last)
    } else if height > 0 && selected >= offset + height {
        (selected + 1 - height).min(last)
    } else {
        offset.min(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    fn items(texts: &[&'static str]) -> Vec<Row<'static>> {
        texts
            .iter()
            .map(|text| Row {
                line: Line::from(*text),
                kind: RowKind::Item,
            })
            .collect()
    }

    fn draw(
        width: u16,
        height: u16,
        rows: &[Row<'_>],
        selected: Option<usize>,
        offset: usize,
        theme: &Theme,
    ) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    rows,
                    selected,
                    offset,
                    theme.accent,
                    theme,
                );
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn the_selected_row_has_the_gutter_and_a_full_width_band() {
        let theme = Theme::default();
        let rows = items(&["alpha", "b", "gamma"]);
        let buffer = draw(12, 3, &rows, Some(1), 0, &theme);

        assert_eq!(buffer[(0, 1)].symbol(), GUTTER);
        assert_eq!(buffer[(0, 1)].fg, theme.accent.fg.unwrap_or(Color::Reset));
        // The row is only one letter long and the band still runs to the last cell.
        for x in 1..12 {
            assert_eq!(buffer[(x, 1)].bg, theme.selected.bg.unwrap(), "x={x}");
        }
        assert_eq!(buffer[(1, 1)].symbol(), "b");
    }

    #[test]
    fn unselected_rows_have_a_blank_gutter_and_no_band() {
        let theme = Theme::default();
        let rows = items(&["alpha", "b", "gamma"]);
        let buffer = draw(12, 3, &rows, Some(1), 0, &theme);

        for y in [0, 2] {
            assert_eq!(buffer[(0, y)].symbol(), " ", "gutter of row {y}");
            for x in 0..12 {
                assert_eq!(buffer[(x, y)].bg, Color::Reset, "({x},{y})");
            }
        }
        assert_eq!(row_text(&buffer, 0), " alpha      ");
    }

    #[test]
    fn no_selection_draws_no_gutter_mark() {
        let theme = Theme::default();
        let buffer = draw(10, 2, &items(&["one", "two"]), None, 0, &theme);
        assert!(!row_text(&buffer, 0).contains(GUTTER));
        assert!(!row_text(&buffer, 1).contains(GUTTER));
    }

    #[test]
    fn a_long_row_ends_in_an_ellipsis_inside_the_area() {
        let theme = Theme::default();
        let rows = items(&["abcdefghijklmnop"]);
        let buffer = draw(8, 1, &rows, None, 0, &theme);
        assert_eq!(row_text(&buffer, 0), " abcdef…");

        let selected = draw(8, 1, &rows, Some(0), 0, &theme);
        assert_eq!(row_text(&selected, 0), "▌abcdef…");
    }

    #[test]
    fn ellipsize_keeps_a_fitting_line_and_cuts_a_long_one_by_cells() {
        let line = Line::from(vec![Span::raw("ab"), Span::raw("cd")]);
        assert_eq!(ellipsize(line.clone(), 4).to_string(), "abcd");
        assert_eq!(ellipsize(line.clone(), 3).to_string(), "ab…");
        assert_eq!(ellipsize(line, 0).to_string(), "");
        // Wide characters count two cells, so the cut lands on a cluster boundary.
        let wide = ellipsize(Line::from("漢字文"), 5);
        assert_eq!(wide.to_string(), "漢字…");
        assert!(wide.width() <= 5);
    }

    #[test]
    fn the_band_pads_to_the_width_and_overrides_a_spans_own_colour() {
        let theme = Theme::default();
        let line = Line::from(Span::styled("ab", Style::new().fg(Color::Red)));
        let band = selection_band(line, 6, &theme);
        assert_eq!(band.width(), 6);
        for span in &band.spans {
            assert_eq!(span.style.fg, theme.selected.fg, "{span:?}");
            assert_eq!(span.style.bg, theme.selected.bg, "{span:?}");
        }
        let cut = selection_band(Line::from("abcdefgh"), 4, &theme);
        assert_eq!(cut.to_string(), "abc…");
    }

    #[test]
    fn a_group_row_shows_its_label_count_and_fold_state() {
        let theme = Theme::default();
        let open = group_row("Open", 3, false, &theme);
        assert_eq!(open.line.to_string(), "▾ Open (3)");
        assert_eq!(open.kind, RowKind::Group { folded: false });

        let folded = group_row("Open", 3, true, &theme);
        assert_eq!(folded.line.to_string(), "▸ Open (3)");
        assert_eq!(folded.kind, RowKind::Group { folded: true });
    }

    #[test]
    fn a_selected_group_header_gets_the_gutter_and_band() {
        let theme = Theme::default();
        let rows = vec![
            group_row("Open", 2, false, &theme),
            Row {
                line: Line::from("item"),
                kind: RowKind::Item,
            },
        ];
        let buffer = draw(14, 2, &rows, Some(0), 0, &theme);
        assert_eq!(row_text(&buffer, 0), "▌▾ Open (2)   ");
        assert_eq!(buffer[(13, 0)].bg, theme.selected.bg.unwrap());
    }

    #[test]
    fn the_hidden_note_names_the_count_the_noun_and_the_key() {
        let theme = Theme::default();
        let note = hidden_note(3, "closed", "c", &theme);
        assert_eq!(note.kind, RowKind::Note);
        assert_eq!(note.line.to_string(), "3 closed hidden · c to show");
        let key = note
            .line
            .spans
            .iter()
            .find(|span| span.content == "c")
            .expect("the key span");
        assert_eq!(key.style, theme.key);
    }

    #[test]
    fn a_note_is_never_selected() {
        let theme = Theme::default();
        let rows = vec![
            Row {
                line: Line::from("item"),
                kind: RowKind::Item,
            },
            hidden_note(2, "closed", "c", &theme),
        ];
        let buffer = draw(32, 2, &rows, Some(1), 0, &theme);
        assert_eq!(buffer[(0, 1)].symbol(), " ");
        assert_eq!(buffer[(1, 1)].bg, Color::Reset);
        assert!(row_text(&buffer, 1).contains("2 closed hidden"));
    }

    #[test]
    fn the_offset_starts_the_window_and_the_height_ends_it() {
        let theme = Theme::default();
        let rows = items(&["r0", "r1", "r2", "r3", "r4"]);
        let buffer = draw(6, 3, &rows, Some(3), 2, &theme);
        assert_eq!(row_text(&buffer, 0), " r2   ");
        assert_eq!(row_text(&buffer, 1), "▌r3   ");
        assert_eq!(row_text(&buffer, 2), " r4   ");

        let past = draw(6, 3, &rows, None, 9, &theme);
        assert_eq!(row_text(&past, 0), "      ");
    }

    #[test]
    fn an_area_too_small_for_a_gutter_and_a_cell_draws_nothing() {
        let theme = Theme::default();
        let buffer = draw(1, 2, &items(&["x", "y"]), Some(0), 0, &theme);
        assert_eq!(row_text(&buffer, 0), " ");
        let none = draw(5, 0, &items(&["x"]), Some(0), 0, &theme);
        assert_eq!(none.area.height, 0);
    }

    #[test]
    fn scrolling_moves_the_least_that_shows_the_selection() {
        // Already visible: unchanged.
        assert_eq!(scroll_into_view(3, 2, 3, 10), 2);
        // Above the window: the selection becomes the first row.
        assert_eq!(scroll_into_view(1, 2, 3, 10), 1);
        // Below the window: the selection becomes the last row.
        assert_eq!(scroll_into_view(5, 2, 3, 10), 3);
        // The window stops at the end of the list.
        assert_eq!(scroll_into_view(9, 0, 3, 10), 7);
        // A list shorter than the window never scrolls.
        assert_eq!(scroll_into_view(2, 1, 5, 3), 0);
        // No room at all.
        assert_eq!(scroll_into_view(4, 0, 0, 10), 0);
        // An offset left past the end by a shrunk list is pulled back.
        assert_eq!(scroll_into_view(2, 8, 3, 5), 2);
    }

    #[test]
    fn under_monochrome_nothing_names_a_colour_and_the_band_is_reversed() {
        let theme = Theme::monochrome();
        let group = group_row("Open", 1, false, &theme);
        let note = hidden_note(1, "closed", "c", &theme);
        for span in group.line.spans.iter().chain(&note.line.spans) {
            assert_eq!((span.style.fg, span.style.bg), (None, None), "{span:?}");
        }
        let band = selection_band(Line::from("ab"), 5, &theme);
        for span in &band.spans {
            assert_eq!((span.style.fg, span.style.bg), (None, None), "{span:?}");
            assert!(span.style.add_modifier.contains(Modifier::REVERSED));
        }

        let rows = vec![group, note];
        let buffer = draw(30, 2, &rows, Some(0), 0, &theme);
        for cell in &buffer.content {
            assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset));
        }
        assert_eq!(buffer[(0, 0)].symbol(), GUTTER);
        assert!(buffer[(1, 0)].modifier.contains(Modifier::REVERSED));
        assert!(!buffer[(1, 1)].modifier.contains(Modifier::REVERSED));
    }
}
