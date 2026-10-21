//! The filter band over a list (MOD-115 M3 D3): `/ query`, a marker while it captures the
//! keys, and `n of m`. Requirements adopts it in M3; the Backlog's filter form in M4.
//!
//! One row, `width` cells, laid out left to right:
//! - typing: [`GUTTER`] in the caller's accent (the keys go to the field), `/ ` in `theme.key`,
//!   then the field's own line with its cursor;
//! - applied: ` / ` dim, then the query in `theme.base`;
//! - either way `{shown} of {total}` in `theme.dim`, against the right edge.
//!
//! The count gives way last: the field first, then the slash, and only a count wider than the
//! whole row is cut, ending with `…` so it never reads as another number (`3 of 1` for
//! `3 of 12`). An applied query that is empty is no band at all, and the caller does not reserve
//! its row.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::kit::list::GUTTER;
use crate::ui::text_field::TextField;
use crate::ui::{Theme, cells};

/// The cells in front of the query: the marker (or a blank), the slash and a blank.
const PREFIX: usize = 3;

/// What the band shows.
#[derive(Debug, Clone, Copy)]
pub enum Filter<'a> {
    /// The query is being typed: the band captures the keys.
    Typing {
        /// The field being typed into.
        field: &'a TextField,
        /// The rows the query keeps.
        shown: usize,
        /// The rows in all.
        total: usize,
    },
    /// A query is applied and the keys are the list's again.
    Applied {
        /// The query as applied.
        query: &'a str,
        /// The rows the query keeps.
        shown: usize,
        /// The rows in all.
        total: usize,
    },
}

/// The band as one line of exactly `width` cells, the count cut with `…` when it alone is wider;
/// an empty applied query is an empty line. `accent` is the typing marker's style
/// (`ctx.accent()`).
#[must_use]
pub fn line(filter: &Filter<'_>, width: u16, accent: Style, theme: &Theme) -> Line<'static> {
    let (shown, total) = match *filter {
        Filter::Applied { query: "", .. } => return Line::default(),
        Filter::Typing { shown, total, .. } | Filter::Applied { shown, total, .. } => {
            (shown, total)
        }
    };
    let width = usize::from(width);
    let count = cells::clip(&format!("{shown} of {total}"), width);
    let count_width = cells::cell_width(&count);
    let mut spans: Vec<Span<'static>> = Vec::new();
    // The prefix, one blank before the count, and the count: what is left is the query's.
    if width >= PREFIX + 1 + count_width {
        let room = width - PREFIX - 1 - count_width;
        match *filter {
            Filter::Typing { field, .. } => {
                spans.push(Span::styled(GUTTER, accent));
                spans.push(Span::styled("/ ", theme.key));
                if room > 0 {
                    let room = u16::try_from(room).unwrap_or(u16::MAX);
                    spans.extend(field.line(room, true, theme).spans);
                }
            }
            Filter::Applied { query, .. } => {
                spans.push(Span::styled(" / ", theme.dim));
                spans.push(Span::styled(cells::clip(query, room), theme.base));
            }
        }
    }
    let used: usize = spans
        .iter()
        .map(|span| cells::cell_width(&span.content))
        .sum();
    let pad = width.saturating_sub(used + count_width);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.push(Span::styled(count, theme.dim));
    Line::from(spans)
}

/// Draws the band into the first row of `area`.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    filter: &Filter<'_>,
    accent: Style,
    theme: &Theme,
) {
    if area.is_empty() {
        return;
    }
    let band = line(filter, area.width, accent, theme);
    frame
        .buffer_mut()
        .set_line(area.x, area.y, &band, area.width);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};

    fn accent() -> Style {
        Style::new().fg(Color::Yellow)
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn style_of(line: &Line<'_>, needle: &str) -> Style {
        line.spans
            .iter()
            .find(|span| span.content.contains(needle))
            .unwrap_or_else(|| panic!("no span holds {needle:?} in {line:?}"))
            .style
    }

    #[test]
    fn typing_reads_the_marker_the_slash_the_field_and_the_count() {
        let theme = Theme::default();
        let field = TextField::with_text("bug");
        let typing = Filter::Typing {
            field: &field,
            shown: 3,
            total: 12,
        };
        let band = line(&typing, 30, accent(), &theme);
        // `▌/ bug`, the cursor cell after it, then `3 of 12` against the right edge.
        assert_eq!(text(&band), format!("▌/ bug{}3 of 12", " ".repeat(17)));
        assert_eq!(band.width(), 30);
        assert_eq!(style_of(&band, "▌"), accent());
        assert_eq!(style_of(&band, "/"), theme.key);
        assert_eq!(style_of(&band, "3 of 12"), theme.dim);
        let cursor = band
            .spans
            .iter()
            .find(|span| span.style == theme.cursor)
            .expect("the field's cursor");
        assert_eq!(cursor.content, " ", "the cursor sits after the text");
    }

    #[test]
    fn applied_reads_the_query_and_the_count_without_a_marker() {
        let theme = Theme::default();
        let applied = Filter::Applied {
            query: "bug",
            shown: 3,
            total: 12,
        };
        let band = line(&applied, 30, accent(), &theme);
        assert_eq!(text(&band), format!(" / bug{}3 of 12", " ".repeat(17)));
        assert!(!text(&band).contains('▌'), "the marker is only for typing");
        assert_eq!(style_of(&band, "/"), theme.dim);
        assert_eq!(style_of(&band, "bug"), theme.base);
        assert_eq!(style_of(&band, "3 of 12"), theme.dim);
        assert!(band.spans.iter().all(|span| span.style != theme.cursor));
    }

    #[test]
    fn the_count_stays_right_aligned_and_whole_while_the_field_gives_way() {
        let theme = Theme::default();
        let field = TextField::with_text("a long query text");
        let typing = Filter::Typing {
            field: &field,
            shown: 3,
            total: 12,
        };
        let applied = Filter::Applied {
            query: "a long query text",
            shown: 3,
            total: 12,
        };
        for filter in [typing, applied] {
            // 20 cells: 3 for `▌/ `, 9 of query, one blank, 7 of count.
            let band = line(&filter, 20, accent(), &theme);
            assert_eq!(band.width(), 20, "{band:?}");
            assert!(text(&band).ends_with(" 3 of 12"), "{band:?}");
            assert!(text(&band).contains('…'), "the query is clipped: {band:?}");
            // No room for any of the query: the slash and the count.
            let band = line(&filter, 11, accent(), &theme);
            assert_eq!(
                text(&band).trim_start_matches('▌'),
                match filter {
                    Filter::Typing { .. } => "/  3 of 12",
                    Filter::Applied { .. } => " /  3 of 12",
                }
            );
            // No room for the slash either: the count alone, against the right edge.
            assert_eq!(text(&line(&filter, 9, accent(), &theme)), "  3 of 12");
            assert_eq!(text(&line(&filter, 7, accent(), &theme)), "3 of 12");
        }
        let applied = line(&applied, 20, accent(), &theme);
        assert_eq!(text(&applied), " / a long q… 3 of 12");
    }

    /// RV1 L1: a count wider than the row is cut with `…`, never to a believable wrong number
    /// (`3 of 1` for `3 of 12`), and the band is never wider than the row.
    #[test]
    fn a_count_wider_than_the_row_ends_with_an_ellipsis() {
        let theme = Theme::default();
        let applied = Filter::Applied {
            query: "bug",
            shown: 3,
            total: 12,
        };
        for width in 0..7u16 {
            let band = line(&applied, width, accent(), &theme);
            assert_eq!(band.width(), usize::from(width), "{width}: {band:?}");
            let shown = text(&band);
            assert!(width == 0 || shown.ends_with('…'), "{width}: {shown}");
            assert!(!shown.contains("of 1"), "{width}: {shown}");
        }
        assert_eq!(text(&line(&applied, 6, accent(), &theme)), "3 of …");
        assert_eq!(
            style_of(&line(&applied, 6, accent(), &theme), "…"),
            theme.dim
        );
        let mut terminal = Terminal::new(TestBackend::new(6, 1)).expect("a test terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), &applied, accent(), &theme))
            .expect("draw");
        let row: String = (0..6)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
            .collect();
        assert_eq!(row, "3 of …");
    }

    #[test]
    fn an_empty_applied_query_is_no_band() {
        let theme = Theme::default();
        let empty = Filter::Applied {
            query: "",
            shown: 12,
            total: 12,
        };
        assert_eq!(line(&empty, 30, accent(), &theme).width(), 0);
        let field = TextField::new();
        let typing = Filter::Typing {
            field: &field,
            shown: 12,
            total: 12,
        };
        assert!(
            text(&line(&typing, 30, accent(), &theme)).starts_with("▌/ "),
            "an empty field being typed into is still a band"
        );
    }

    #[test]
    fn render_draws_the_band_into_the_rows_first_line() {
        let theme = Theme::default();
        let applied = Filter::Applied {
            query: "bug",
            shown: 1,
            total: 2,
        };
        let mut terminal = Terminal::new(TestBackend::new(16, 2)).expect("a test terminal");
        terminal
            .draw(|frame| render(frame, Rect::new(0, 1, 16, 1), &applied, accent(), &theme))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..16).map(|x| buffer[(x, y)].symbol()).collect() };
        assert_eq!(row(0).trim(), "");
        assert_eq!(row(1), " / bug    1 of 2");
    }

    #[test]
    fn monochrome_keeps_the_marker_and_the_count_readable_without_colour() {
        let theme = Theme::monochrome();
        let field = TextField::with_text("x");
        let typing = Filter::Typing {
            field: &field,
            shown: 1,
            total: 4,
        };
        let band = line(&typing, 24, theme.tab_accent("requirements"), &theme);
        for span in &band.spans {
            for colour in [span.style.fg, span.style.bg] {
                assert!(matches!(colour, None | Some(Color::Reset)), "{span:?}");
            }
        }
        assert!(style_of(&band, "▌").add_modifier.contains(Modifier::BOLD));
        assert!(
            style_of(&band, "1 of 4")
                .add_modifier
                .contains(Modifier::DIM)
        );
        assert!(
            band.spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::REVERSED)),
            "the cursor"
        );
    }
}
