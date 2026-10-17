//! The rounded pane: title, sub-tab chips, bottom-border hints, focus and a scrollbar (MOD-115).
//!
//! A focused pane's border takes the tab accent, any other the faint border role. The title sits
//! on the top border after the corner, the sub-tab chips flush with the right corner, the hints
//! on the bottom border two cells in; none of them ever overwrites a corner. When the pane is
//! narrow the title is clipped first, then whole chips are dropped from the right, then the hints
//! are clipped (always with an ellipsis, via `cells::clip_line`).
//!
//! Every style is a [`Theme`] role, so the monochrome theme keeps focus visible: a focused border
//! is bold, an unfocused one dim.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use crate::ui::{Theme, cells};

/// The scrollbar's thumb: heavy, so it reads over the light vertical border it replaces.
const THUMB: &str = "\u{2503}";

/// Whether a border cell is a horizontal rule, the only cell [`paint_bottom_hints`] may overwrite.
fn is_rule(symbol: &str) -> bool {
    matches!(symbol, "\u{2500}" | "\u{2501}")
}

/// Whether a pane has focus: the focused border takes the tab accent, the other the faint one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The pane keys go to.
    Focused,
    /// Any other pane.
    Unfocused,
}

/// A pane under construction; [`Pane::render`] draws it.
#[derive(Debug, Clone)]
pub struct Pane<'a> {
    title: &'a str,
    sub_tabs: Vec<&'a str>,
    active: usize,
    hints: Option<Line<'a>>,
    focus: Focus,
    accent: Option<Style>,
    scrollbar: Option<(usize, usize)>,
}

impl<'a> Pane<'a> {
    /// A pane titled `title`, unfocused, with no chips, hints or scrollbar.
    #[must_use]
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            sub_tabs: Vec::new(),
            active: 0,
            hints: None,
            focus: Focus::Unfocused,
            accent: None,
            scrollbar: None,
        }
    }

    /// Sub-tab chips on the top border, `active` being the current one.
    #[must_use]
    pub fn sub_tabs(mut self, titles: &[&'a str], active: usize) -> Self {
        self.sub_tabs = titles.to_vec();
        self.active = active;
        self
    }

    /// Hints on the bottom border, drawn only while the pane is focused.
    #[must_use]
    pub fn hints(mut self, hints: Line<'a>) -> Self {
        self.hints = Some(hints);
        self
    }

    /// Sets the focus.
    #[must_use]
    pub fn focus(mut self, focus: Focus) -> Self {
        self.focus = focus;
        self
    }

    /// The focused border's style (the tab accent); without it the theme's focus accent.
    #[must_use]
    pub fn accent(mut self, accent: Style) -> Self {
        self.accent = Some(accent);
        self
    }

    /// A scrollbar for `total` rows scrolled to `offset`.
    #[must_use]
    pub fn scrollbar(mut self, total: usize, offset: usize) -> Self {
        self.scrollbar = Some((total, offset));
        self
    }

    /// Draws the pane into `area` and returns the inner rect for its content. The scrollbar, if
    /// any, is measured against that rect's height and replaces part of the right border.
    pub fn render(self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect {
        let focused = self.focus == Focus::Focused;
        let border = if focused {
            self.accent.unwrap_or(theme.accent)
        } else {
            theme.border_faint
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if area.width < 2 || area.height == 0 {
            return inner;
        }
        let buf = frame.buffer_mut();

        let chips = sub_tab_rects(area, &self.sub_tabs);
        let mut limit = area.right() - 1;
        for (index, (rect, title)) in chips.iter().zip(&self.sub_tabs).enumerate() {
            if rect.width == 0 {
                continue;
            }
            limit = limit.min(rect.x);
            let label_style = if index == self.active {
                theme.active_tab
            } else {
                theme.dim
            };
            let chip = Line::from(vec![
                Span::raw(" "),
                Span::styled((*title).to_owned(), label_style),
                Span::raw(" "),
            ]);
            buf.set_line(rect.x, rect.y, &chip, rect.width);
        }

        if !self.title.is_empty() {
            let start = area.x + 2;
            let room = usize::from(limit.saturating_sub(1).saturating_sub(start));
            if room > 2 {
                let text = cells::clip(&format!(" {} ", self.title), room);
                let style = if focused {
                    border.add_modifier(Modifier::BOLD)
                } else {
                    theme.dim
                };
                buf.set_stringn(start, area.y, text, room, style);
            }
        }

        if focused
            && let Some(hints) = self.hints
            && area.height >= 2
        {
            let width = area.width.saturating_sub(4);
            if width > 0 {
                let line = cells::clip_line(hints, width);
                line.render(Rect::new(area.x + 2, area.bottom() - 1, width, 1), buf);
            }
        }

        if let Some((total, offset)) = self.scrollbar {
            paint_scrollbar(buf, area, inner, total, offset, border);
        }
        inner
    }
}

/// The thumb on the right border for `total` rows scrolled to `offset`; nothing when they all fit.
fn paint_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    inner: Rect,
    total: usize,
    offset: usize,
    style: Style,
) {
    let viewport = usize::from(inner.height);
    if viewport == 0 || total <= viewport || area.width < 2 {
        return;
    }
    let offset = clamp_offset(offset, total, viewport);
    let length = (viewport * viewport / total).clamp(1, viewport);
    let travel = viewport - length;
    let max = total - viewport;
    let top = (offset * travel + max / 2) / max;
    let x = area.right() - 1;
    for row in top..top + length {
        let y = inner.y + u16::try_from(row).unwrap_or(u16::MAX);
        buf[(x, y)].set_symbol(THUMB).set_style(style);
    }
}

/// Where each sub-tab chip sits on the top border of a pane drawn into `area`, one rect per title.
/// A chip is ` title ` (two cells wider than the title); chips are adjacent and end flush with the
/// right corner, so the block starts at `area.right() - 1 - width`. A chip that does not fit
/// between the corners is dropped whole, from the right, and gets a zero-width rect no click hits.
#[must_use]
pub fn sub_tab_rects(area: Rect, titles: &[&str]) -> Vec<Rect> {
    let widths: Vec<u16> = titles
        .iter()
        .map(|title| u16::try_from(cells::cell_width(title) + 2).unwrap_or(u16::MAX))
        .collect();
    let avail = u32::from(area.width.saturating_sub(2));
    let mut shown = widths.len();
    let mut total: u32 = widths.iter().map(|w| u32::from(*w)).sum();
    while shown > 0 && total > avail {
        shown -= 1;
        total -= u32::from(widths[shown]);
    }
    let corner = area.right().saturating_sub(1);
    let gone = Rect::new(corner, area.y, 0, 1);
    let mut x = corner.saturating_sub(u16::try_from(total).unwrap_or(0));
    widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            if index >= shown {
                return gone;
            }
            let rect = Rect::new(x, area.y, *width, 1);
            x += width;
            rect
        })
        .collect()
}

/// Draws `hints` over the horizontal border glyphs of `row`, the last row of a body a tab has
/// already framed (the M1 adapter until tabs hand their hints to [`Pane::hints`]). They start two
/// cells in and are clipped to `row.width - 4`; a cell that is not a plain horizontal rule (a
/// corner, text) is left alone, so a row that is not a border simply loses the hint. `style` is
/// the hint line's base style.
pub fn paint_bottom_hints(buf: &mut Buffer, row: Rect, hints: Line<'_>, style: Style) {
    let width = row.width.saturating_sub(4);
    let row = row.intersection(buf.area);
    if width == 0 || row.is_empty() {
        return;
    }
    let line = cells::clip_line(hints.patch_style(style), width);
    let shown = u16::try_from(line.width()).unwrap_or(width).min(width);
    let origin = Rect::new(row.x + 2, row.y, shown, 1).intersection(buf.area);
    if origin.is_empty() {
        return;
    }
    let mut scratch = Buffer::empty(origin);
    line.render(origin, &mut scratch);
    let mut x = origin.x;
    while x < origin.right() {
        let cells_wide = cells::cell_width(scratch[(x, origin.y)].symbol()).max(1);
        let end = (x + u16::try_from(cells_wide).unwrap_or(1)).min(origin.right());
        if (x..end).all(|col| is_rule(buf[(col, origin.y)].symbol())) {
            for col in x..end {
                buf[(col, origin.y)] = scratch[(col, origin.y)].clone();
            }
        }
        x = end;
    }
}

/// `offset` clamped so a scrolled list stops once its last row is in view.
#[must_use]
pub fn clamp_offset(offset: usize, total: usize, viewport: usize) -> usize {
    offset.min(total.saturating_sub(viewport))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// `pane` drawn into the whole of a `width` by `height` terminal: the buffer and the inner rect.
    fn draw(width: u16, height: u16, theme: &Theme, pane: Pane<'_>) -> (Buffer, Rect) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        let mut inner = Rect::default();
        terminal
            .draw(|frame| inner = pane.render(frame, frame.area(), theme))
            .expect("draw");
        (terminal.backend().buffer().clone(), inner)
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    fn accent() -> Style {
        Style::new().fg(Color::Magenta)
    }

    #[test]
    fn the_title_sits_on_the_top_border_after_the_corner() {
        let (buffer, inner) = draw(20, 5, &Theme::default(), Pane::new("Items"));
        assert_eq!(row(&buffer, 0), format!("╭─ Items {}╮", "─".repeat(10)));
        assert_eq!(row(&buffer, 4), format!("╰{}╯", "─".repeat(18)));
        assert_eq!(
            inner,
            Rect::new(1, 1, 18, 3),
            "the content goes inside the border"
        );
    }

    #[test]
    fn a_long_title_is_clipped_with_an_ellipsis_and_never_reaches_the_corner() {
        let (buffer, _) = draw(12, 3, &Theme::default(), Pane::new("A very long title"));
        let top = row(&buffer, 0);
        assert!(top.starts_with("╭─ "), "{top}");
        assert!(top.contains('…'), "{top}");
        assert!(
            top.ends_with("─╮"),
            "the corner and its rule are untouched: {top}"
        );
    }

    #[test]
    fn sub_tab_chips_end_flush_with_the_right_corner() {
        let area = Rect::new(2, 1, 30, 5);
        let rects = sub_tab_rects(area, &["Body", "Notes"]);
        // " Body " is 6 cells, " Notes " is 7: together 13, starting at right - 1 - 13.
        assert_eq!(area.right() - 1 - 13, 18);
        assert_eq!(rects, vec![Rect::new(18, 1, 6, 1), Rect::new(24, 1, 7, 1)]);
        assert_eq!(rects[1].right(), area.right() - 1);
    }

    #[test]
    fn the_chips_are_drawn_where_the_rects_say_and_the_active_one_is_styled() {
        let theme = Theme::default();
        let titles = ["Body", "Notes"];
        let (buffer, _) = draw(
            30,
            4,
            &theme,
            Pane::new("Item").sub_tabs(&titles, 1).focus(Focus::Focused),
        );
        let rects = sub_tab_rects(Rect::new(0, 0, 30, 4), &titles);
        let text = |rect: Rect| -> String {
            (rect.x..rect.right())
                .map(|x| buffer[(x, 0)].symbol())
                .collect()
        };
        assert_eq!(text(rects[0]), " Body ");
        assert_eq!(text(rects[1]), " Notes ");
        let active = &buffer[(rects[1].x + 1, 0)];
        let idle = &buffer[(rects[0].x + 1, 0)];
        assert_eq!(active.style().fg, theme.active_tab.fg);
        assert!(
            active
                .modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert_eq!(idle.style().fg, theme.dim.fg);
        assert_eq!(row(&buffer, 0).chars().last(), Some('╮'));
    }

    #[test]
    fn the_title_is_clipped_before_a_chip_is_dropped() {
        let titles = ["Body", "Notes"];
        let (buffer, _) = draw(
            24,
            3,
            &Theme::default(),
            Pane::new("Backlog items").sub_tabs(&titles, 0),
        );
        let top = row(&buffer, 0);
        assert!(top.contains(" Body  Notes "), "both chips survive: {top}");
        assert!(top.contains('…') || top.contains("Backlog"), "{top}");
        assert!(top.starts_with("╭─ "), "{top}");
    }

    #[test]
    fn chips_that_do_not_fit_are_dropped_whole_from_the_right() {
        let area = Rect::new(0, 0, 14, 3);
        let rects = sub_tab_rects(area, &["Body", "Notes", "Graph"]);
        // 12 cells between the corners: " Body " (6) fits, " Notes " (7) does not.
        assert_eq!(rects[0], Rect::new(7, 0, 6, 1));
        assert_eq!(rects[1].width, 0);
        assert_eq!(rects[2].width, 0);
        let (buffer, _) = draw(
            14,
            3,
            &Theme::default(),
            Pane::new("").sub_tabs(&["Body", "Notes", "Graph"], 0),
        );
        let top = row(&buffer, 0);
        assert!(
            top.contains("Body") && !top.contains("Notes") && !top.contains("Graph"),
            "{top}"
        );
        assert!(top.ends_with('╮'), "{top}");
        assert!(
            sub_tab_rects(Rect::new(0, 0, 2, 3), &["Body"])[0].width == 0,
            "no room at all"
        );
    }

    #[test]
    fn hints_start_two_cells_in_and_only_while_focused() {
        let hints = || Line::from("enter open");
        let (focused, _) = draw(
            24,
            4,
            &Theme::default(),
            Pane::new("T").hints(hints()).focus(Focus::Focused),
        );
        assert_eq!(row(&focused, 3), format!("╰─enter open{}╯", "─".repeat(11)));
        let (unfocused, _) = draw(24, 4, &Theme::default(), Pane::new("T").hints(hints()));
        assert_eq!(row(&unfocused, 3), format!("╰{}╯", "─".repeat(22)));
    }

    #[test]
    fn hints_are_trimmed_to_the_width_and_never_touch_a_corner() {
        let hints = Line::from("enter open · e edit · d delete · ? help");
        let (buffer, _) = draw(
            20,
            3,
            &Theme::default(),
            Pane::new("T").hints(hints).focus(Focus::Focused),
        );
        let bottom = row(&buffer, 2);
        assert!(bottom.starts_with("╰─"), "{bottom}");
        assert!(bottom.ends_with("─╯"), "{bottom}");
        assert!(
            bottom.contains('…'),
            "a clipped hint line ends in an ellipsis: {bottom}"
        );
        assert_eq!(bottom.chars().count(), 20);
        // Too narrow for any hint: nothing is drawn, the corners stay.
        let (tiny, _) = draw(
            4,
            3,
            &Theme::default(),
            Pane::new("T").hints(Line::from("x")).focus(Focus::Focused),
        );
        assert_eq!(row(&tiny, 2), "╰──╯");
    }

    #[test]
    fn a_focused_border_takes_the_accent_and_an_unfocused_one_the_faint_role() {
        let theme = Theme::default();
        let (focused, _) = draw(
            10,
            3,
            &theme,
            Pane::new("").focus(Focus::Focused).accent(accent()),
        );
        let (unfocused, _) = draw(10, 3, &theme, Pane::new("").accent(accent()));
        for buffer in [&focused, &unfocused] {
            assert_eq!(buffer[(0, 0)].symbol(), "╭");
        }
        for (x, y) in [(0, 0), (9, 0), (0, 1), (5, 2), (9, 2)] {
            assert_eq!(focused[(x, y)].fg, Color::Magenta, "({x},{y})");
            assert_eq!(unfocused[(x, y)].fg, Color::Indexed(240), "({x},{y})");
        }
        let (default_accent, _) = draw(10, 3, &theme, Pane::new("").focus(Focus::Focused));
        assert_eq!(
            default_accent[(0, 0)].fg,
            Color::Cyan,
            "no accent given: the focus accent"
        );
    }

    /// The thumb's rows, top to bottom, on the right border of a 6-row pane (4 inner rows).
    fn thumb_rows(total: usize, offset: usize) -> Vec<u16> {
        let (buffer, _) = draw(
            10,
            6,
            &Theme::default(),
            Pane::new("").scrollbar(total, offset),
        );
        (1..5)
            .filter(|&y| buffer[(9, y)].symbol() == THUMB)
            .collect()
    }

    #[test]
    fn the_scrollbar_thumb_moves_down_and_stops_at_the_end() {
        assert_eq!(thumb_rows(8, 0), vec![1, 2], "at the top");
        assert_eq!(
            thumb_rows(8, 4),
            vec![3, 4],
            "the last row in view: the thumb is at the bottom"
        );
        assert_eq!(
            thumb_rows(8, 40),
            vec![3, 4],
            "scrolled past the end is the end"
        );
        let middle = thumb_rows(8, 2);
        assert_eq!(middle.len(), 2);
        assert!(middle[0] > 1 && middle[1] < 4, "{middle:?}");
    }

    #[test]
    fn no_scrollbar_when_everything_fits() {
        assert!(thumb_rows(4, 0).is_empty());
        assert!(thumb_rows(0, 0).is_empty());
        let (buffer, _) = draw(10, 6, &Theme::default(), Pane::new("").scrollbar(4, 0));
        assert_eq!(buffer[(9, 2)].symbol(), "│");
    }

    #[test]
    fn clamp_offset_stops_at_the_last_full_page() {
        assert_eq!(clamp_offset(0, 10, 4), 0);
        assert_eq!(clamp_offset(6, 10, 4), 6);
        assert_eq!(clamp_offset(9, 10, 4), 6);
        assert_eq!(
            clamp_offset(3, 2, 4),
            0,
            "a list shorter than the viewport never scrolls"
        );
    }

    #[test]
    fn monochrome_keeps_focus_visible_without_colour() {
        let theme = Theme::monochrome();
        let accent = theme.tab_accent("backlog");
        let (focused, _) = draw(
            10,
            3,
            &theme,
            Pane::new("T").focus(Focus::Focused).accent(accent),
        );
        let (unfocused, _) = draw(10, 3, &theme, Pane::new("T").accent(accent));
        let (bare, _) = draw(10, 3, &theme, Pane::new("T").focus(Focus::Focused));
        assert!(focused[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(
            bare[(0, 0)].modifier.contains(Modifier::BOLD),
            "even with no accent given"
        );
        assert!(unfocused[(0, 0)].modifier.contains(Modifier::DIM));
        assert!(!unfocused[(0, 0)].modifier.contains(Modifier::BOLD));
        for buffer in [&focused, &unfocused, &bare] {
            for y in 0..3 {
                for x in 0..10 {
                    assert_eq!(buffer[(x, y)].fg, Color::Reset, "({x},{y})");
                    assert_eq!(buffer[(x, y)].bg, Color::Reset, "({x},{y})");
                }
            }
        }
    }

    fn bordered_row(text: &str) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(
            0,
            0,
            u16::try_from(text.chars().count()).unwrap(),
            1,
        ));
        buffer.set_string(0, 0, text, Style::new());
        buffer
    }

    #[test]
    fn bottom_hints_overwrite_only_horizontal_rules() {
        let mut buffer = bordered_row(&format!("╰{}╯", "─".repeat(16)));
        paint_bottom_hints(
            &mut buffer,
            Rect::new(0, 0, 18, 1),
            Line::from("a b"),
            Style::new(),
        );
        assert_eq!(row(&buffer, 0), format!("╰─a b{}╯", "─".repeat(12)));
        // Anything that is not a plain rule is kept, the hint cell dropped.
        let mut text = bordered_row("│ some body text │");
        paint_bottom_hints(
            &mut text,
            Rect::new(0, 0, 18, 1),
            Line::from("a b"),
            Style::new(),
        );
        assert_eq!(row(&text, 0), "│ some body text │");
        let mut mixed = bordered_row(&format!("╰──x{}╯", "─".repeat(13)));
        paint_bottom_hints(
            &mut mixed,
            Rect::new(0, 0, 18, 1),
            Line::from("abcd"),
            Style::new(),
        );
        assert_eq!(row(&mixed, 0), format!("╰─axcd{}╯", "─".repeat(11)));
    }

    #[test]
    fn bottom_hints_are_clipped_and_leave_the_corners_alone() {
        let mut buffer = bordered_row(&format!("╰{}╯", "─".repeat(10)));
        paint_bottom_hints(
            &mut buffer,
            Rect::new(0, 0, 12, 1),
            Line::from("enter open and more"),
            Style::new(),
        );
        let text = row(&buffer, 0);
        assert!(
            text.starts_with("╰─") && text.ends_with("─╯") && text.contains('…'),
            "{text}"
        );
        assert_eq!(text.chars().count(), 12);
    }
}
