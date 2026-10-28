//! A titled rule (MOD-115 M4): the one-row header of a part drawn inside a pane: an in-pane
//! editor, a compose area (C6, D-14). Not a pane: the part lives in its host pane's rect, so a
//! box would nest a second frame there and take the in-pane editor's columns.
//!
//! The row reads `title` flush left (clipped with `…`), then `─` to the edge: the row a
//! `Block` with only its top border and a title drew, cell for cell. Focused, the title and the
//! rule take `theme.title`; otherwise `theme.dim`. The row is cleared first, so nothing the host
//! drew there shows through.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Clear;

use crate::ui::{Theme, cells};

/// The rule glyph.
const RULE: &str = "\u{2500}";

/// Draws the titled rule on the first row of `area` and returns the rows under it. An empty
/// `area` draws nothing and is returned as it is.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    focused: bool,
    theme: &Theme,
) -> Rect {
    if area.is_empty() {
        return area;
    }
    let style = if focused { theme.title } else { theme.dim };
    let rule = Rect { height: 1, ..area };
    frame.render_widget(Clear, rule);
    let width = usize::from(area.width);
    let title = cells::clip(title, width);
    let used = cells::cell_width(&title);
    let text = format!("{title}{}", RULE.repeat(width.saturating_sub(used)));
    frame
        .buffer_mut()
        .set_stringn(area.x, area.y, text, width, style);
    Rect {
        y: area.y.saturating_add(1),
        height: area.height - 1,
        ..area
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::widgets::{Block, Borders};

    /// `title`'s rule drawn over `area` of a `width` x `height` screen pre-filled with a red `#`:
    /// the rows under it and the buffer.
    fn draw(
        (width, height): (u16, u16),
        area: Rect,
        title: &str,
        focused: bool,
        theme: &Theme,
    ) -> (Rect, Buffer) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let mut under = Rect::default();
        terminal
            .draw(|frame| {
                let screen = frame.area();
                for y in 0..screen.height {
                    for x in 0..screen.width {
                        frame.buffer_mut()[(x, y)]
                            .set_symbol("#")
                            .set_style(Style::new().bg(Color::Red));
                    }
                }
                under = render(frame, area, title, focused, theme);
            })
            .expect("draw");
        (under, terminal.backend().buffer().clone())
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn the_rule_reads_as_the_title_then_rules() {
        let area = Rect::new(0, 0, 20, 3);
        let (_, buffer) = draw((20, 3), area, "Notes", true, &Theme::default());
        assert_eq!(row(&buffer, 0), format!("Notes{}", "─".repeat(15)));
        // Cell for cell the row a top-bordered, titled `Block` drew.
        let mut block = Buffer::empty(area);
        ratatui::widgets::Widget::render(
            Block::new().borders(Borders::TOP).title("Notes"),
            area,
            &mut block,
        );
        assert_eq!(row(&buffer, 0), row(&block, 0));
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        let area = Rect::new(0, 0, 8, 2);
        let (_, buffer) = draw((8, 2), area, "A very long title", false, &Theme::default());
        let top = row(&buffer, 0);
        assert!(top.starts_with("A very"), "{top}");
        assert!(top.ends_with('…'), "{top}");
        assert_eq!(cells::cell_width(&top), 8);
    }

    #[test]
    fn focused_is_title_else_dim() {
        let theme = Theme::monochrome();
        let area = Rect::new(0, 0, 20, 2);
        let (_, focused) = draw((20, 2), area, "edit", true, &theme);
        let (_, idle) = draw((20, 2), area, "edit", false, &theme);
        for x in [0, 3, 4, 19] {
            let on = &focused[(x, 0)];
            assert!(on.modifier.contains(Modifier::BOLD), "focused ({x}) {on:?}");
            assert!(!on.modifier.contains(Modifier::DIM), "focused ({x}) {on:?}");
            let off = &idle[(x, 0)];
            assert!(off.modifier.contains(Modifier::DIM), "idle ({x}) {off:?}");
            assert!(!off.modifier.contains(Modifier::BOLD), "idle ({x}) {off:?}");
            for cell in [on, off] {
                assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset), "({x})");
            }
        }
        let theme = Theme::default();
        let (_, coloured) = draw((20, 2), area, "edit", false, &theme);
        assert_eq!(coloured[(0, 0)].fg, theme.dim.fg.unwrap_or_default());
        assert_eq!(coloured[(10, 0)].fg, theme.dim.fg.unwrap_or_default());
    }

    #[test]
    fn the_row_is_cleared_first() {
        let area = Rect::new(2, 1, 10, 3);
        let (_, buffer) = draw((14, 4), area, "a", true, &Theme::default());
        for x in area.x..area.right() {
            let cell = &buffer[(x, 1)];
            assert_ne!(cell.symbol(), "#", "({x})");
            assert_eq!(cell.bg, Color::Reset, "({x}) nothing shows through");
        }
        assert_eq!(buffer[(1, 1)].symbol(), "#", "only the area's row");
        assert_eq!(buffer[(2, 2)].symbol(), "#", "only its first row");
    }

    #[test]
    fn it_returns_the_rows_under_it() {
        let theme = Theme::default();
        let (under, _) = draw((30, 10), Rect::new(2, 3, 20, 5), "t", true, &theme);
        assert_eq!(under, Rect::new(2, 4, 20, 4));
        let (under, _) = draw((30, 10), Rect::new(2, 3, 20, 1), "t", true, &theme);
        assert_eq!(under, Rect::new(2, 4, 20, 0));
        let (under, buffer) = draw((30, 10), Rect::new(2, 3, 0, 0), "t", true, &theme);
        assert_eq!(under, Rect::new(2, 3, 0, 0));
        assert_eq!(buffer[(2, 3)].symbol(), "#", "an empty area draws nothing");
    }
}
