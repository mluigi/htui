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
//!
//! For panes a tab still frames itself, the shell paints over them (MOD-115 M2 D3):
//! [`paint_focus`] restyles a drawn border for the shell's focus, [`paint_bottom_hints`] and
//! [`paint_bottom_note`] put the hints (left) and a hidden pane's note (right) on its bottom rule.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
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
    note: Option<Line<'a>>,
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
            note: None,
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

    /// A note on the bottom border, right-aligned and drawn focused or not: what a hidden pane
    /// says about bringing it back (`R-TUI-12`). It takes only the rule the hints left.
    #[must_use]
    pub fn note(mut self, note: Line<'a>) -> Self {
        self.note = Some(note);
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

        if let Some(note) = self.note
            && area.height >= 2
        {
            let row = Rect::new(area.x, area.bottom() - 1, area.width, 1);
            paint_bottom_note(buf, row, note, theme.dim);
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
/// cells in and are clipped to `row.width - 4`. The hint is drawn only over the run of plain
/// horizontal rule that starts there: where the tab drew text of its own on the border (a scroll
/// hint as a bottom title) or a corner stands, the hint ends with an ellipsis in the last rule
/// cell before it, or whole if it fits in the run. A row whose first cell there is not a rule
/// gets nothing, so a row that is not a border simply loses the hint. `style` is the hint
/// line's base style.
pub fn paint_bottom_hints(buf: &mut Buffer, row: Rect, hints: Line<'_>, style: Style) {
    let width = row.width.saturating_sub(4);
    let row = row.intersection(buf.area);
    if width == 0 || row.is_empty() {
        return;
    }
    let start = row.x + 2;
    let run = (start..start.saturating_add(width))
        .take_while(|x| *x < row.right() && is_rule(buf[(*x, row.y)].symbol()))
        .count();
    let room = u16::try_from(run).unwrap_or(width).min(width);
    if room == 0 {
        return;
    }
    let line = cells::clip_line(hints.patch_style(style), room);
    let shown = u16::try_from(line.width()).unwrap_or(room).min(room);
    let origin = Rect::new(start, row.y, shown, 1).intersection(buf.area);
    if origin.is_empty() {
        return;
    }
    let mut scratch = Buffer::empty(origin);
    line.render(origin, &mut scratch);
    for col in origin.x..origin.right() {
        buf[(col, origin.y)] = scratch[(col, origin.y)].clone();
    }
}

/// Whether `symbol` is one box-drawing glyph (U+2500..=U+257F), the only cell [`paint_focus`]
/// restyles.
fn is_box_drawing(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if ('\u{2500}'..='\u{257F}').contains(&c))
}

/// Restyles the border of a pane a tab already drew into `area` for `focus` (the shell's focus,
/// MOD-115 M2 D3): focused, the `accent` (the tab accent; bold in monochrome), else the faint
/// border role (dim in monochrome). Only the box-drawing cells on `area`'s edge change, so the
/// title, the chips, the hints and the note keep their style, and so does the pane's content.
pub fn paint_focus(buf: &mut Buffer, area: Rect, focus: Focus, accent: Style, theme: &Theme) {
    if area.is_empty() {
        return;
    }
    let style = match focus {
        Focus::Focused => accent,
        Focus::Unfocused => theme.border_faint,
    };
    let (right, bottom) = (area.right() - 1, area.bottom() - 1);
    let edge = (area.x..=right)
        .flat_map(|x| [(x, area.y), (x, bottom)])
        .chain((area.y..=bottom).flat_map(|y| [(area.x, y), (right, y)]));
    for (x, y) in edge {
        if !buf.area.contains(Position::new(x, y)) {
            continue;
        }
        let cell = &mut buf[(x, y)];
        if is_box_drawing(cell.symbol()) {
            // Replace the colour and the modifiers rather than patch them, so a dim border
            // that gains focus is bold alone; the background is the pane's own.
            cell.fg = Color::Reset;
            cell.modifier = Modifier::empty();
            cell.set_style(style);
        }
    }
}

/// Draws `note` over the horizontal border glyphs of `row`, right-aligned: the twin of
/// [`paint_bottom_hints`], ending two cells before the right corner. It is clipped to
/// `row.width - 4` and drawn only over the run of plain rule that ends there, so it stops (with
/// an ellipsis) at the first cell that is not one, a hint or a corner. `style` is the note's
/// base style.
pub fn paint_bottom_note(buf: &mut Buffer, row: Rect, note: Line<'_>, style: Style) {
    let width = row.width.saturating_sub(4);
    let row = row.intersection(buf.area);
    if width == 0 || row.is_empty() {
        return;
    }
    // One past the note's last cell: the corner and one rule cell stay to its right.
    let end = row.right().saturating_sub(2);
    let run = (1..=width)
        .take_while(|k| end >= row.x + k && is_rule(buf[(end - k, row.y)].symbol()))
        .count();
    let room = u16::try_from(run).unwrap_or(width).min(width);
    if room == 0 {
        return;
    }
    let line = cells::clip_line(note.patch_style(style), room);
    let shown = u16::try_from(line.width()).unwrap_or(room).min(room);
    let origin = Rect::new(end - shown, row.y, shown, 1).intersection(buf.area);
    if origin.is_empty() {
        return;
    }
    let mut scratch = Buffer::empty(origin);
    line.render(origin, &mut scratch);
    for col in origin.x..origin.right() {
        buf[(col, origin.y)] = scratch[(col, origin.y)].clone();
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
        // The hint runs up to the first cell that is not a rule and ends there with an
        // ellipsis, instead of skipping that cell and going on past it.
        assert_eq!(row(&mixed, 0), format!("╰─…x{}╯", "─".repeat(13)));
        let mut room = bordered_row(&format!("╰──x{}╯", "─".repeat(13)));
        paint_bottom_hints(
            &mut room,
            Rect::new(0, 0, 18, 1),
            Line::from("a"),
            Style::new(),
        );
        assert_eq!(
            row(&room, 0),
            format!("╰─ax{}╯", "─".repeat(13)),
            "a hint that fits before the text is drawn whole, no ellipsis"
        );
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

    /// A 12 by 4 plain-bordered block in `border`, its title in yellow, a yellow hint on the
    /// bottom border and two green rule glyphs inside.
    fn framed(border: Style) -> Buffer {
        let area = Rect::new(0, 0, 12, 4);
        let mut buffer = Buffer::empty(area);
        Block::bordered()
            .border_style(border)
            .title(Line::styled(" Items ", Style::new().fg(Color::Yellow)))
            .render(area, &mut buffer);
        paint_bottom_hints(
            &mut buffer,
            Rect::new(0, 3, 12, 1),
            Line::from("a b"),
            Style::new().fg(Color::Yellow),
        );
        buffer.set_string(2, 1, "──", Style::new().fg(Color::Green));
        buffer
    }

    /// The edge cells of a 12 by 4 area that are still border glyphs in [`framed`].
    const EDGE: [(u16, u16); 8] = [
        (0, 0),
        (11, 0),
        (9, 0),
        (0, 1),
        (11, 2),
        (0, 3),
        (8, 3),
        (11, 3),
    ];

    #[test]
    fn paint_focus_gives_a_focused_border_the_accent_and_leaves_the_text_alone() {
        let theme = Theme::default();
        let mut buffer = framed(Style::new().fg(Color::White));
        paint_focus(
            &mut buffer,
            Rect::new(0, 0, 12, 4),
            Focus::Focused,
            accent(),
            &theme,
        );
        for (x, y) in EDGE {
            assert_eq!(buffer[(x, y)].fg, Color::Magenta, "({x},{y})");
        }
        assert_eq!(buffer[(2, 0)].symbol(), "I");
        assert_eq!(
            buffer[(2, 0)].fg,
            Color::Yellow,
            "the title keeps its style"
        );
        assert_eq!(buffer[(2, 3)].symbol(), "a");
        assert_eq!(buffer[(2, 3)].fg, Color::Yellow, "the hint keeps its style");
        assert_eq!(buffer[(2, 1)].symbol(), "─");
        assert_eq!(
            buffer[(2, 1)].fg,
            Color::Green,
            "the content is not the border"
        );
    }

    #[test]
    fn paint_focus_gives_an_unfocused_border_the_faint_role() {
        let theme = Theme::default();
        let mut buffer = framed(accent());
        paint_focus(
            &mut buffer,
            Rect::new(0, 0, 12, 4),
            Focus::Unfocused,
            accent(),
            &theme,
        );
        for (x, y) in EDGE {
            assert_eq!(buffer[(x, y)].fg, Color::Indexed(240), "({x},{y})");
        }
        assert_eq!(buffer[(2, 0)].fg, Color::Yellow);
        assert_eq!(buffer[(2, 1)].fg, Color::Green);
    }

    #[test]
    fn paint_focus_in_monochrome_is_bold_or_dim_never_both_and_never_coloured() {
        let theme = Theme::monochrome();
        let area = Rect::new(0, 0, 12, 4);
        let mut buffer = framed(theme.border_faint);
        paint_focus(
            &mut buffer,
            area,
            Focus::Focused,
            theme.tab_accent("backlog"),
            &theme,
        );
        for (x, y) in EDGE {
            let cell = &buffer[(x, y)];
            assert!(cell.modifier.contains(Modifier::BOLD), "({x},{y})");
            assert!(!cell.modifier.contains(Modifier::DIM), "({x},{y})");
            assert_eq!(
                (cell.fg, cell.bg),
                (Color::Reset, Color::Reset),
                "({x},{y})"
            );
        }
        paint_focus(
            &mut buffer,
            area,
            Focus::Unfocused,
            theme.tab_accent("backlog"),
            &theme,
        );
        for (x, y) in EDGE {
            let cell = &buffer[(x, y)];
            assert!(cell.modifier.contains(Modifier::DIM), "({x},{y})");
            assert!(!cell.modifier.contains(Modifier::BOLD), "({x},{y})");
            assert_eq!(
                (cell.fg, cell.bg),
                (Color::Reset, Color::Reset),
                "({x},{y})"
            );
        }
    }

    #[test]
    fn paint_focus_restyles_a_rounded_pane_and_its_scrollbar() {
        let theme = Theme::default();
        let (mut buffer, _) = draw(10, 6, &theme, Pane::new("").scrollbar(8, 0));
        paint_focus(
            &mut buffer,
            Rect::new(0, 0, 10, 6),
            Focus::Focused,
            accent(),
            &theme,
        );
        assert_eq!(buffer[(0, 0)].symbol(), "╭");
        assert_eq!(buffer[(0, 0)].fg, Color::Magenta);
        assert_eq!(buffer[(9, 1)].symbol(), THUMB);
        assert_eq!(buffer[(9, 1)].fg, Color::Magenta);
    }

    #[test]
    fn paint_focus_outside_the_buffer_or_on_nothing_does_not_panic() {
        let theme = Theme::default();
        let mut buffer = framed(Style::new());
        paint_focus(
            &mut buffer,
            Rect::new(6, 2, 20, 10),
            Focus::Focused,
            accent(),
            &theme,
        );
        assert_eq!(
            buffer[(11, 2)].fg,
            Color::Magenta,
            "the clipped edge's own cells"
        );
        paint_focus(
            &mut buffer,
            Rect::default(),
            Focus::Focused,
            accent(),
            &theme,
        );
        paint_focus(
            &mut buffer,
            Rect::new(40, 40, 3, 3),
            Focus::Focused,
            accent(),
            &theme,
        );
    }

    #[test]
    fn a_bottom_note_ends_two_cells_before_the_corner() {
        let mut buffer = bordered_row(&format!("╰{}╯", "─".repeat(16)));
        paint_bottom_note(
            &mut buffer,
            Rect::new(0, 0, 18, 1),
            Line::from("a b"),
            Style::new().fg(Color::Yellow),
        );
        assert_eq!(row(&buffer, 0), format!("╰{}a b─╯", "─".repeat(12)));
        assert_eq!(buffer[(13, 0)].fg, Color::Yellow, "the note's style");
    }

    #[test]
    fn a_bottom_note_takes_only_the_rule_the_hints_left() {
        let text = format!("╰─abcdef{}╯", "─".repeat(6));
        let mut clipped = bordered_row(&text);
        paint_bottom_note(
            &mut clipped,
            Rect::new(0, 0, 15, 1),
            Line::from("123456789"),
            Style::new(),
        );
        assert_eq!(row(&clipped, 0), "╰─abcdef1234…─╯");
        let mut fits = bordered_row(&text);
        paint_bottom_note(
            &mut fits,
            Rect::new(0, 0, 15, 1),
            Line::from("ab"),
            Style::new(),
        );
        assert_eq!(row(&fits, 0), "╰─abcdef───ab─╯");
    }

    #[test]
    fn a_bottom_note_skips_a_row_that_is_not_a_border_or_has_no_room() {
        let mut text = bordered_row("│ some body text │");
        paint_bottom_note(
            &mut text,
            Rect::new(0, 0, 18, 1),
            Line::from("z unzoom"),
            Style::new(),
        );
        assert_eq!(row(&text, 0), "│ some body text │");
        let mut tiny = bordered_row("╰──╯");
        paint_bottom_note(
            &mut tiny,
            Rect::new(0, 0, 4, 1),
            Line::from("z"),
            Style::new(),
        );
        assert_eq!(row(&tiny, 0), "╰──╯");
        let mut long = bordered_row(&format!("╰{}╯", "─".repeat(10)));
        paint_bottom_note(
            &mut long,
            Rect::new(0, 0, 12, 1),
            Line::from("F2 show list · p layouts"),
            Style::new(),
        );
        let shown = row(&long, 0);
        assert!(
            shown.starts_with("╰─") && shown.ends_with("─╯") && shown.contains('…'),
            "{shown}"
        );
        assert_eq!(shown.chars().count(), 12);
    }

    #[test]
    fn a_panes_note_sits_right_aligned_on_the_bottom_border_focused_or_not() {
        let theme = Theme::default();
        let (unfocused, _) = draw(24, 4, &theme, Pane::new("T").note(Line::from("z unzoom")));
        assert_eq!(row(&unfocused, 3), format!("╰{}z unzoom─╯", "─".repeat(13)));
        let (focused, _) = draw(
            30,
            4,
            &theme,
            Pane::new("T")
                .hints(Line::from("enter open"))
                .note(Line::from("z unzoom"))
                .focus(Focus::Focused),
        );
        assert_eq!(
            row(&focused, 3),
            format!("╰─enter open{}z unzoom─╯", "─".repeat(8))
        );
    }
}
