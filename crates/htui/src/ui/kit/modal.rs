//! The modal frame over [`layout::centered`](crate::ui::layout::centered) (MOD-115).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::ui::Theme;
use crate::ui::layout::centered;

/// Columns a modal's chrome takes around its content: the left border, the right border and one
/// column of right padding.
pub const CHROME: u16 = 3;

/// Rows a modal's chrome takes: the top and bottom border.
const CHROME_ROWS: u16 = 2;

/// Columns a `Fit` modal leaves free on the screen (two each side).
const MARGIN_COLS: u16 = 4;

/// Rows a `Fit` modal leaves free on the screen (one each side).
const MARGIN_ROWS: u16 = 2;

/// How big a modal is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalSize {
    /// As big as its content, clamped to the screen.
    Fit {
        /// Content width in cells.
        content_w: u16,
        /// Content height in rows.
        content_h: u16,
    },
    /// An exact size.
    Fixed {
        /// Width in cells.
        w: u16,
        /// Height in rows.
        h: u16,
    },
}

/// A modal under construction; [`Modal::render`] draws it.
#[derive(Debug, Clone)]
pub struct Modal<'a> {
    title: &'a str,
    size: ModalSize,
    hint: Option<&'a str>,
}

impl<'a> Modal<'a> {
    /// A modal titled `title`, fitting a one-cell content.
    #[must_use]
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            size: ModalSize::Fit {
                content_w: 1,
                content_h: 1,
            },
            hint: None,
        }
    }

    /// Sets the size.
    #[must_use]
    pub fn size(mut self, size: ModalSize) -> Self {
        self.size = size;
        self
    }

    /// A hint on the bottom border.
    #[must_use]
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// The box's width and height inside `area`. `Fit` adds the chrome to the content and keeps
    /// two columns and one row free on every side, the arithmetic the waiting list used;
    /// `Fixed` is taken as given, and `layout::centered` clamps it to `area`.
    fn dimensions(&self, area: Rect) -> (u16, u16) {
        match self.size {
            ModalSize::Fit {
                content_w,
                content_h,
            } => (
                content_w
                    .saturating_add(CHROME)
                    .min(area.width.saturating_sub(MARGIN_COLS)),
                content_h
                    .saturating_add(CHROME_ROWS)
                    .min(area.height.saturating_sub(MARGIN_ROWS)),
            ),
            ModalSize::Fixed { w, h } => (w, h),
        }
    }

    /// Clears and draws the modal over `area`, returning the content rect: the box less its
    /// borders and its right padding column. The content starts at the box's first inner
    /// column, so a row that wants a left margin brings its own.
    pub fn render(self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect {
        let (width, height) = self.dimensions(area);
        let box_area = centered(area, width, height);
        frame.render_widget(Clear, box_area);
        let mut block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.accent)
            .title(Span::styled(format!(" {} ", self.title), theme.title));
        if let Some(hint) = self.hint {
            block = block.title_bottom(Line::styled(format!(" {hint} "), theme.dim));
        }
        frame.render_widget(block, box_area);
        Rect::new(
            box_area.x.saturating_add(1),
            box_area.y.saturating_add(1),
            box_area.width.saturating_sub(CHROME),
            box_area.height.saturating_sub(CHROME_ROWS),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Modifier;

    /// Draws `modal` into a `width` x `height` screen pre-filled with `#`; returns the content
    /// rect and the buffer.
    fn draw(modal: Modal<'_>, width: u16, height: u16, theme: &Theme) -> (Rect, Buffer) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let mut content = Rect::default();
        terminal
            .draw(|frame| {
                let area = frame.area();
                for y in 0..area.height {
                    for x in 0..area.width {
                        frame.buffer_mut()[(x, y)].set_symbol("#");
                    }
                }
                content = modal.render(frame, area, theme);
            })
            .expect("draw");
        (content, terminal.backend().buffer().clone())
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_owned())
            .collect()
    }

    /// The box's outer rect: the bounding box of every cell that is not the `#` fill.
    fn box_of(buffer: &Buffer) -> Rect {
        let mut found: Option<(u16, u16, u16, u16)> = None;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer[(x, y)].symbol() != "#" {
                    let (x0, y0, x1, y1) = found.unwrap_or((x, y, x, y));
                    found = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
                }
            }
        }
        let (x0, y0, x1, y1) = found.expect("the modal drew something");
        Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
    }

    #[test]
    fn a_fit_modal_sits_on_layout_centered() {
        let area = Rect::new(0, 0, 80, 24);
        let modal = Modal::new("Waiting").size(ModalSize::Fit {
            content_w: 20,
            content_h: 5,
        });
        let (_, buffer) = draw(modal, 80, 24, &Theme::default());
        assert_eq!(
            box_of(&buffer),
            centered(area, 20 + CHROME, 5 + 2),
            "content plus chrome, centred"
        );
    }

    #[test]
    fn a_fixed_modal_is_exactly_the_size_asked() {
        let area = Rect::new(0, 0, 80, 24);
        let modal = Modal::new("Fixed").size(ModalSize::Fixed { w: 30, h: 8 });
        let (_, buffer) = draw(modal, 80, 24, &Theme::default());
        assert_eq!(box_of(&buffer), centered(area, 30, 8));
    }

    #[test]
    fn a_fixed_modal_bigger_than_the_screen_clamps_to_it() {
        let modal = Modal::new("Fixed").size(ModalSize::Fixed { w: 200, h: 90 });
        let (_, buffer) = draw(modal, 40, 10, &Theme::default());
        assert_eq!(box_of(&buffer), Rect::new(0, 0, 40, 10));
    }

    #[test]
    fn fit_clamps_to_the_area_less_two_columns_and_one_row_each_side() {
        let modal = Modal::new("Big").size(ModalSize::Fit {
            content_w: 500,
            content_h: 500,
        });
        let (_, buffer) = draw(modal, 40, 12, &Theme::default());
        assert_eq!(box_of(&buffer), Rect::new(2, 1, 36, 10));
    }

    #[test]
    fn fit_on_a_screen_too_small_for_its_margin_collapses_without_panicking() {
        let modal = Modal::new("Tiny").size(ModalSize::Fit {
            content_w: 10,
            content_h: 10,
        });
        let (content, _) = draw(modal, 3, 1, &Theme::default());
        assert_eq!((content.width, content.height), (0, 0));
    }

    #[test]
    fn the_content_rect_is_the_box_less_borders_and_right_padding() {
        let modal = Modal::new("Waiting").size(ModalSize::Fit {
            content_w: 20,
            content_h: 5,
        });
        let (content, buffer) = draw(modal, 80, 24, &Theme::default());
        let outer = box_of(&buffer);
        assert_eq!(content.width, 20);
        assert_eq!(content.height, 5);
        assert_eq!((content.x, content.y), (outer.x + 1, outer.y + 1));
        assert!(content.right() < outer.right(), "a padding column remains");
    }

    #[test]
    fn the_box_clears_what_was_under_it() {
        let modal = Modal::new("Clear").size(ModalSize::Fixed { w: 12, h: 5 });
        let (content, buffer) = draw(modal, 40, 12, &Theme::default());
        for y in content.y..content.bottom() {
            for x in content.x..content.right() {
                assert_eq!(
                    buffer[(x, y)].symbol(),
                    " ",
                    "({x},{y}) still shows the page"
                );
            }
        }
    }

    #[test]
    fn the_title_rides_the_top_border_and_the_hint_the_bottom_one() {
        let modal = Modal::new("Quit?")
            .size(ModalSize::Fixed { w: 30, h: 5 })
            .hint("y yes  n no");
        let (_, buffer) = draw(modal, 60, 12, &Theme::default());
        let outer = box_of(&buffer);
        assert!(row(&buffer, outer.y).contains(" Quit? "));
        assert!(row(&buffer, outer.bottom() - 1).contains(" y yes  n no "));
    }

    #[test]
    fn a_modal_without_a_hint_leaves_the_bottom_border_bare() {
        let modal = Modal::new("Quit?").size(ModalSize::Fixed { w: 30, h: 5 });
        let (_, buffer) = draw(modal, 60, 12, &Theme::default());
        let outer = box_of(&buffer);
        let bottom = row(&buffer, outer.bottom() - 1);
        let inner: String = bottom
            .chars()
            .skip(usize::from(outer.x) + 1)
            .take(usize::from(outer.width) - 2)
            .collect();
        assert!(inner.chars().all(|c| c == '\u{2500}'), "{inner:?}");
    }

    #[test]
    fn the_border_is_rounded_in_the_focus_role() {
        let theme = Theme::default();
        let modal = Modal::new("Round").size(ModalSize::Fixed { w: 20, h: 5 });
        let (_, buffer) = draw(modal, 40, 12, &theme);
        let outer = box_of(&buffer);
        let corner = &buffer[(outer.x, outer.y)];
        assert_eq!(corner.symbol(), "\u{256d}");
        assert_eq!(corner.fg, theme.accent.fg.unwrap_or_default());
    }

    #[test]
    fn under_monochrome_the_frame_names_no_colour_and_the_border_is_bold() {
        let theme = Theme::monochrome();
        let modal = Modal::new("Mono")
            .size(ModalSize::Fixed { w: 20, h: 5 })
            .hint("esc");
        let (_, buffer) = draw(modal, 40, 12, &theme);
        let outer = box_of(&buffer);
        for y in outer.y..outer.bottom() {
            for x in outer.x..outer.right() {
                let cell = &buffer[(x, y)];
                assert_eq!(cell.fg, ratatui::style::Color::Reset, "({x},{y})");
                assert_eq!(cell.bg, ratatui::style::Color::Reset, "({x},{y})");
            }
        }
        assert!(buffer[(outer.x, outer.y)].modifier.contains(Modifier::BOLD));
    }
}
