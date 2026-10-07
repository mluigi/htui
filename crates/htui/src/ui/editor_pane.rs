//! The in-pane editor's widget: a VT screen copied into the frame (MOD-57 M1, plan P2, P10).
//!
//! The child draws into a `vt100` grid; this module copies that grid into ratatui's buffer cell
//! by cell, under a one-row top rule that carries the title (MOD-57 B14). The grid is the truth:
//! a wide cell is drawn with its continuation left blank (in the wide cell's style, which the
//! terminal painted there, so a shrunk coloured wide cell is repainted), and every drawn cell is
//! pinned to the grid's width with `CellDiffOption::ForcedWidth`, so ratatui's diff never
//! re-measures a VS16 emoji or a halfwidth sound mark (which `vt100` counts per code point and
//! ratatui per string) and never skips the cell behind it (plan "Widths"). Every covered cell is
//! reset first, so a smaller screen leaves nothing stale. The terminal's real cursor is the only
//! cursor drawn (MOD-57 B9); nothing here keeps state, logs or formats the screen's contents.

use std::num::NonZeroU16;

use ratatui::Frame;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};

use crate::ui::Theme;
use crate::ui::cells::clip;

/// The diff width of a narrow grid cell.
const NARROW: NonZeroU16 = NonZeroU16::MIN;
/// The diff width of a wide grid cell (its continuation is the second column).
const WIDE: NonZeroU16 = NonZeroU16::new(2).unwrap();

/// Draws an in-pane editor over `area`: a top rule titled `title` (focused: `theme.title`, else
/// `theme.dim`), then the VT screen cell by cell in the rest. When `focused` and the child shows
/// its cursor, the terminal's real cursor is placed on it (`Frame::set_cursor_position`).
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    screen: &vt100::Screen,
    title: &str,
    focused: bool,
    theme: &Theme,
) {
    if area.is_empty() {
        return;
    }
    let style = if focused { theme.title } else { theme.dim };
    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(style)
        .title(Line::styled(clip(title, usize::from(area.width)), style));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    draw_screen(frame.buffer_mut(), inner, screen);
    if focused && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < inner.height && col < inner.width {
            frame.set_cursor_position((inner.x + col, inner.y + row));
        }
    }
}

/// The grid into `area`: every covered cell is reset, then holds the grid cell at its offset, or
/// stays blank when the grid has none there, when it is a wide cell's continuation (blank in the
/// wide cell's style) or when a wide cell would not fit in the last column. A blank grid cell
/// keeps its style. A drawn cell's diff width is the grid's (`ForcedWidth`).
fn draw_screen(buf: &mut Buffer, area: Rect, screen: &vt100::Screen) {
    let area = area.intersection(buf.area);
    for y in 0..area.height {
        for x in 0..area.width {
            let target = &mut buf[(area.x + x, area.y + y)];
            target.reset();
            let Some(cell) = screen.cell(y, x) else {
                continue;
            };
            if cell.is_wide_continuation() {
                // Blank, but in the wide cell's style: the terminal painted this column with it,
                // so the next frame's diff sees a change when a default blank replaces it.
                if let Some(wide) = x.checked_sub(1).and_then(|left| screen.cell(y, left)) {
                    target.set_style(style_of(wide));
                }
                continue;
            }
            if cell.is_wide() && x + 1 == area.width {
                continue;
            }
            target.set_style(style_of(cell));
            if !cell.has_contents() {
                continue;
            }
            let width = if cell.is_wide() { WIDE } else { NARROW };
            target
                .set_symbol(cell.contents())
                .set_diff_option(CellDiffOption::ForcedWidth(width));
        }
    }
}

/// A cell's colours and attributes.
fn style_of(cell: &vt100::Cell) -> Style {
    let mut modifier = Modifier::empty();
    for (on, flag) in [
        (cell.bold(), Modifier::BOLD),
        (cell.dim(), Modifier::DIM),
        (cell.italic(), Modifier::ITALIC),
        (cell.underline(), Modifier::UNDERLINED),
        (cell.inverse(), Modifier::REVERSED),
    ] {
        modifier.set(flag, on);
    }
    Style::new()
        .fg(color(cell.fgcolor()))
        .bg(color(cell.bgcolor()))
        .add_modifier(modifier)
}

/// `Default` → `Color::Reset`, `Idx(i)` → `Color::Indexed(i)`, `Rgb(r, g, b)` → `Color::Rgb`.
fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Cell;
    use ratatui::layout::Position;

    use super::*;
    use crate::ui::cells::cell_width;

    const FOCUSED: &str = " nvim · Ctrl+4 to htui ";
    const UNFOCUSED: &str = " nvim · htui has the keys ";
    /// Test 1's screen: SGR red, a CJK pair, a VS16 emoji and a halfwidth sound mark.
    const MIXED: &[u8] = "hello \x1b[31mred\x1b[0m\r\n中文X\r\n❤\u{fe0f}Y ｱﾞZ".as_bytes();

    fn parser(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(bytes);
        parser
    }

    /// One draw of `screen` over `area` into `term`.
    fn draw(
        term: &mut Terminal<TestBackend>,
        area: Rect,
        screen: &vt100::Screen,
        title: &str,
        focused: bool,
    ) {
        term.draw(|frame| render(frame, area, screen, title, focused, &Theme::default()))
            .expect("the pane draws");
    }

    /// The buffer as a terminal shows it, one trimmed line per row: a cell covered by the wide
    /// cell before it (its `ForcedWidth`, else its symbol's width) is not printed.
    fn text(buffer: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            let mut x = 0;
            while x < buffer.area.width {
                let cell = &buffer[(buffer.area.x + x, buffer.area.y + y)];
                line.push_str(cell.symbol());
                x += match cell.diff_option {
                    CellDiffOption::ForcedWidth(width) => width.get(),
                    _ => u16::try_from(cell_width(cell.symbol()).max(1)).unwrap_or(1),
                };
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// The backend's cells `x..x + len` of row `y`, one symbol each: what reached the terminal.
    fn cells(term: &Terminal<TestBackend>, x: u16, y: u16, len: u16) -> String {
        let buffer = term.backend().buffer();
        (x..x + len).map(|x| buffer[(x, y)].symbol()).collect()
    }

    fn styled(style: Style) -> Cell {
        let mut cell = Cell::EMPTY;
        cell.set_style(style);
        cell
    }

    #[test]
    fn the_screen_is_copied_cell_by_cell() {
        let parser = parser(6, 20, MIXED);
        let mut term = Terminal::new(TestBackend::new(22, 7)).expect("a test backend");
        draw(
            &mut term,
            Rect::new(0, 0, 22, 7),
            parser.screen(),
            FOCUSED,
            true,
        );
        let buffer = term.backend().buffer().clone();
        insta::assert_snapshot!("the_focused_frame", text(&buffer));

        // `red` is on the first screen row, the second frame row.
        for x in 6..9 {
            assert_eq!(buffer[(x, 1)].fg, Color::Indexed(1), "`red` at column {x}");
        }
        assert_eq!(buffer[(5, 1)].fg, Color::Reset, "the space before `red`");
        assert_eq!(buffer[(0, 2)].symbol(), "中");
        assert_eq!(
            buffer[(0, 2)].diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(2).expect("2")),
            "a wide cell is two grid columns wide"
        );
        assert_eq!(buffer[(1, 2)], Cell::EMPTY, "the continuation stays blank");
        assert_eq!(
            buffer[(0, 1)].diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(1).expect("1")),
            "a narrow cell is one grid column wide"
        );
        assert_eq!(buffer[(0, 3)].symbol(), "❤\u{fe0f}");
        assert_eq!(
            buffer[(0, 3)].diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(1).expect("1")),
            "a VS16 emoji is as wide as the VT grid says, not as unicode-width says"
        );
        assert_eq!(buffer[(1, 3)].symbol(), "Y");
    }

    #[test]
    fn a_vs16_emoji_or_sound_mark_leaves_no_stale_cell_across_two_frames() {
        let area = Rect::new(0, 0, 10, 3);
        for (first, second) in [
            ("❤\u{fe0f}Y", "abcd"),
            ("ｱﾞZ", "abcd"),
            ("abcd", "❤\u{fe0f}Y"),
            ("abcd", "ｱﾞZ"),
        ] {
            let mut term = Terminal::new(TestBackend::new(10, 3)).expect("a test backend");
            for bytes in [first, second] {
                let parser = parser(2, 10, bytes.as_bytes());
                draw(&mut term, area, parser.screen(), FOCUSED, false);
                // Each grid cell's own symbol, as the VT grid holds it (blank past the text).
                let expected: String = (0..4)
                    .map(|col| {
                        parser.screen().cell(0, col).map_or(" ", |cell| {
                            if cell.has_contents() {
                                cell.contents()
                            } else {
                                " "
                            }
                        })
                    })
                    .collect();
                assert_eq!(
                    cells(&term, 0, 1, 4),
                    expected,
                    "{first:?} then {second:?}: the terminal holds the grid after {bytes:?}"
                );
            }
        }
    }

    #[test]
    fn the_cursor_is_real_and_only_when_focused() {
        let area = Rect::new(2, 1, 20, 6);
        let mut term = Terminal::new(TestBackend::new(30, 10)).expect("a test backend");
        let shown = parser(5, 20, b"ab\r\ncd");
        assert_eq!(shown.screen().cursor_position(), (1, 2));

        draw(&mut term, area, shown.screen(), FOCUSED, true);
        assert!(term.backend().cursor_visible(), "focused: the cursor shows");
        assert_eq!(
            term.get_cursor_position().expect("a position"),
            Position::new(2 + 2, 1 + 1 + 1),
            "on the screen's cursor, below the rule"
        );

        draw(&mut term, area, shown.screen(), UNFOCUSED, false);
        assert!(!term.backend().cursor_visible(), "unfocused: hidden");

        let hidden = parser(5, 20, b"ab\x1b[?25l");
        draw(&mut term, area, hidden.screen(), FOCUSED, true);
        assert!(!term.backend().cursor_visible(), "the child hid it");

        // A 24x80 screen with its cursor at row 10, column 50, in a 20x5 rect: clipped away.
        let far = parser(24, 80, b"\x1b[11;51H");
        assert_eq!(far.screen().cursor_position(), (10, 50));
        draw(
            &mut term,
            Rect::new(0, 0, 20, 5),
            far.screen(),
            FOCUSED,
            true,
        );
        assert!(!term.backend().cursor_visible(), "outside the rect: hidden");
    }

    #[test]
    fn a_rect_smaller_or_larger_than_the_screen_draws_without_panic() {
        // `中` covers columns 8 and 9 of the first row; row 23 has text at column 79.
        let parser = parser(24, 80, "abcdefgh中\x1b[24;80Hz".as_bytes());
        let screen = parser.screen();
        let mut term = Terminal::new(TestBackend::new(100, 40)).expect("a test backend");
        for area in [
            Rect::new(0, 0, 10, 3),
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 1),
            Rect::new(1, 1, 90, 30),
            Rect::new(0, 0, 9, 4),
        ] {
            let drawn = term.draw(|frame| {
                // Junk under the pane: every covered cell must be reset.
                let full = frame.area();
                for y in full.top()..full.bottom() {
                    for x in full.left()..full.right() {
                        frame.buffer_mut()[(x, y)].set_symbol("#");
                    }
                }
                render(frame, area, screen, FOCUSED, true, &Theme::default());
            });
            // The frame's own buffer: the backend never sees a continuation, which the terminal
            // fills with the wide cell before it.
            let buffer = drawn.expect("the pane draws").buffer.clone();
            if area.height < 2 {
                continue;
            }
            let inner = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
            for y in inner.top()..inner.bottom() {
                for x in inner.left()..inner.right() {
                    let (row, col) = (y - inner.y, x - inner.x);
                    let symbol = buffer[(x, y)].symbol();
                    assert_ne!(symbol, "#", "{area:?}: ({x}, {y}) was reset");
                    if row >= 24 || col >= 80 {
                        assert_eq!(symbol, " ", "{area:?}: beyond the screen is blank");
                    }
                }
            }
            if area.width == 9 {
                assert_eq!(
                    buffer[(8, inner.y)],
                    Cell::EMPTY,
                    "a wide character in the last column is blank"
                );
                assert_eq!(buffer[(7, inner.y)].symbol(), "h");
            }
            if area.width == 90 {
                assert_eq!(buffer[(inner.x + 79, inner.y + 23)].symbol(), "z");
                assert_eq!(buffer[(inner.x + 8, inner.y)].symbol(), "中");
            }
        }
    }

    #[test]
    fn the_title_style_follows_focus() {
        let theme = Theme::default();
        let parser = parser(6, 20, MIXED);
        let area = Rect::new(0, 0, 22, 7);
        let mut term = Terminal::new(TestBackend::new(22, 7)).expect("a test backend");

        draw(&mut term, area, parser.screen(), FOCUSED, true);
        let buffer = term.backend().buffer();
        assert_eq!(buffer[(1, 0)].symbol(), "n", "the title starts the rule");
        for x in [1, 21] {
            let cell = &buffer[(x, 0)];
            let expected = styled(theme.title);
            assert_eq!(
                (cell.fg, cell.modifier),
                (expected.fg, expected.modifier),
                "x {x}"
            );
        }

        draw(&mut term, area, parser.screen(), UNFOCUSED, false);
        let buffer = term.backend().buffer().clone();
        insta::assert_snapshot!("the_unfocused_frame", text(&buffer));
        for x in [1, 21] {
            let cell = &buffer[(x, 0)];
            let expected = styled(theme.dim);
            assert_eq!(
                (cell.fg, cell.modifier),
                (expected.fg, expected.modifier),
                "x {x}"
            );
        }

        // The rule past the title takes the same style.
        let mut wide = Terminal::new(TestBackend::new(40, 3)).expect("a test backend");
        for (focused, style) in [(true, theme.title), (false, theme.dim)] {
            draw(
                &mut wide,
                Rect::new(0, 0, 40, 3),
                parser.screen(),
                FOCUSED,
                focused,
            );
            let cell = &wide.backend().buffer()[(39, 0)];
            let expected = styled(style);
            assert_eq!(cell.symbol(), "─", "the rule");
            assert_eq!((cell.fg, cell.modifier), (expected.fg, expected.modifier));
        }

        // A title wider than the rect is clipped, never wrapped or overflowed.
        let mut narrow = Terminal::new(TestBackend::new(10, 3)).expect("a test backend");
        draw(
            &mut narrow,
            Rect::new(0, 0, 10, 3),
            parser.screen(),
            FOCUSED,
            true,
        );
        assert_eq!(cells(&narrow, 0, 0, 10), " nvim · C…");
    }

    #[test]
    fn a_blank_cell_keeps_its_background() {
        // `EL` with a blue background: the rest of the row is blank, yet blue (a status line).
        let parser = parser(2, 10, b"ab\x1b[44m\x1b[K\x1b[0m");
        assert!(!parser.screen().cell(0, 5).expect("a cell").has_contents());
        let mut term = Terminal::new(TestBackend::new(10, 3)).expect("a test backend");
        draw(
            &mut term,
            Rect::new(0, 0, 10, 3),
            parser.screen(),
            FOCUSED,
            true,
        );
        let buffer = term.backend().buffer();
        for x in 2..10 {
            assert_eq!(buffer[(x, 1)].symbol(), " ", "column {x} is blank");
            assert_eq!(
                buffer[(x, 1)].bg,
                Color::Indexed(4),
                "column {x} stays blue"
            );
        }
        assert_eq!(
            buffer[(1, 1)].bg,
            Color::Reset,
            "`b` was written before the SGR"
        );
        assert_eq!(buffer[(0, 2)].bg, Color::Reset, "the next row is untouched");
    }

    #[test]
    fn a_shrunk_wide_cell_repaints_its_coloured_trailing_column() {
        // The terminal paints both columns of a wide cell with its background (or reverse
        // video); when a narrow cell and a default blank replace it, the diff must send the
        // blank, or the right half stays coloured (ratatui #2585).
        let area = Rect::new(0, 0, 4, 1);
        for first in ["\x1b[44m中\x1b[0m", "\x1b[7m中\x1b[0m"] {
            let mut prev = Buffer::empty(area);
            draw_screen(&mut prev, area, parser(1, 4, first.as_bytes()).screen());
            let painted = Buffer::empty(area).diff(&prev);
            assert!(
                painted.iter().all(|&(x, _, _)| x != 1),
                "{first:?}: the continuation is never sent, the wide cell covers it"
            );

            let mut next = Buffer::empty(area);
            draw_screen(&mut next, area, parser(1, 4, b"a").screen());
            let sent: Vec<(u16, u16, String)> = prev
                .diff(&next)
                .into_iter()
                .map(|(x, y, cell)| (x, y, cell.symbol().to_owned()))
                .collect();
            assert_eq!(
                sent,
                [(0, 0, "a".to_owned()), (1, 0, " ".to_owned())],
                "{first:?} then `a`"
            );
        }
    }

    #[test]
    fn colours_and_attributes_map() {
        for (vt, expected) in [
            (vt100::Color::Default, Color::Reset),
            (vt100::Color::Idx(0), Color::Indexed(0)),
            (vt100::Color::Idx(200), Color::Indexed(200)),
            (vt100::Color::Rgb(1, 2, 3), Color::Rgb(1, 2, 3)),
        ] {
            assert_eq!(color(vt), expected, "{vt:?}");
        }

        let parser = parser(
            1,
            20,
            b"\x1b[1mb\x1b[0m\x1b[2md\x1b[0m\x1b[3mi\x1b[0m\x1b[4mu\x1b[0m\x1b[7mr\x1b[0m\
              \x1b[38;2;1;2;3;48;5;200mc\x1b[0mp",
        );
        let screen = parser.screen();
        let at = |col| style_of(screen.cell(0, col).expect("a cell"));
        for (col, modifier) in [
            (0, Modifier::BOLD),
            (1, Modifier::DIM),
            (2, Modifier::ITALIC),
            (3, Modifier::UNDERLINED),
            (4, Modifier::REVERSED),
        ] {
            let style = at(col);
            assert_eq!(style.add_modifier, modifier, "column {col}");
            assert_eq!(
                (style.fg, style.bg),
                (Some(Color::Reset), Some(Color::Reset))
            );
        }
        let coloured = at(5);
        assert_eq!(coloured.fg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(coloured.bg, Some(Color::Indexed(200)));
        assert_eq!(coloured.add_modifier, Modifier::empty());
        let plain = at(6);
        assert_eq!(
            (plain.fg, plain.bg, plain.add_modifier),
            (Some(Color::Reset), Some(Color::Reset), Modifier::empty())
        );
    }
}
