//! The modal frame over [`layout::centered`](crate::ui::layout::centered) (MOD-115), the one
//! overlay rule (MOD-115 M4 C3: what a modal leaves visible is faded with
//! [`Theme::backdrop`](crate::ui::Theme::backdrop)), and the two kinds of box built on it: a
//! question ([`Ask`], C7) and the help box ([`help`]).

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::app::Handled;
use crate::keys::{KeyChord, Stack};
use crate::ui::kit::list::ellipsize;
use crate::ui::kit::text::wrapped;
use crate::ui::layout::centered;
use crate::ui::text_field::TextField;
use crate::ui::{Theme, cells};

/// Columns a modal's chrome takes around its content: the left border, the right border and one
/// column of right padding.
pub const CHROME: u16 = 3;

/// Rows a modal's chrome takes: the top and bottom border.
const CHROME_ROWS: u16 = 2;

/// Columns a `Fit` modal leaves free on the screen (two each side).
const MARGIN_COLS: u16 = 4;

/// Columns of the bottom border a hint never takes: the two corners and the blank either side.
const HINT_CHROME: u16 = 4;

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

    /// A hint on the bottom border: whole when the border holds it, else cut ending in `…`.
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

    /// Fades `area` outside `box_area` with [`Theme::backdrop`] (MOD-115 M4 C3), and clears and
    /// draws the modal over `area`, returning the content rect: the box less its borders and
    /// its right padding column. The content starts at the box's first inner column, so a row
    /// that wants a left margin brings its own. `area` is what the modal is centred over: the
    /// whole frame for an overlay, a section's rect for the Hierarchy path picker.
    pub fn render(self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect {
        let (width, height) = self.dimensions(area);
        let box_area = centered(area, width, height);
        backdrop(frame.buffer_mut(), area, box_area, theme);
        frame.render_widget(Clear, box_area);
        let mut block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.accent)
            .title(Span::styled(format!(" {} ", self.title), theme.title));
        if let Some(hint) = self.hint {
            // The one cut rule for every modal's border hint (MOD-115 M4 E6): whole when the
            // border holds it between its corners with a blank either side, else cut there
            // ending in `…`, so a narrow screen never cuts the keys mid-word with no mark.
            let room = box_area.width.saturating_sub(HINT_CHROME);
            let hint = ellipsize(Line::raw(hint), room);
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

/// Restyles every cell of `area` (within the buffer) outside `box_area` with the backdrop role:
/// ratatui's `Cell::set_style` adds DIM and takes BOLD away, and leaves the cell's colours and
/// text as they were.
fn backdrop(buffer: &mut Buffer, area: Rect, box_area: Rect, theme: &Theme) {
    let area = area.intersection(buffer.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if !box_area.contains(Position::new(x, y)) {
                buffer[(x, y)].set_style(theme.backdrop);
            }
        }
    }
}

/// The content width an [`Ask`] grows to unless told otherwise.
const ASK_WIDTH: u16 = 60;

/// A question in a modal (MOD-115 M4 E3): what is asked, optionally a typed answer, the keys on
/// the bottom border.
///
/// The box fits its content (`ModalSize::Fit`, so it keeps the margin and fades the rest): as
/// wide as its widest line, its input, its title and its hint (and one rule cell) ask for, at
/// most [`Ask::width`]; as tall as the question wrapped to that width, then, with an input, one
/// blank row, the input row and the error under it.
#[derive(Debug, Clone)]
pub struct Ask<'a> {
    title: &'a str,
    lines: Vec<Line<'a>>,
    input: Option<(&'a str, &'a TextField)>,
    error: Option<Cow<'a, str>>,
    hint: Option<&'a str>,
    width: u16,
}

impl<'a> Ask<'a> {
    /// A question titled `title`, with no lines yet; its content grows to 60 cells.
    #[must_use]
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            lines: Vec::new(),
            input: None,
            error: None,
            hint: None,
            width: ASK_WIDTH,
        }
    }

    /// A line of the question, wrapped to the content width. A line that fits is drawn as
    /// given; a wrapped one keeps its line style (patched with its first span's), since its
    /// words move between rows.
    #[must_use]
    pub fn line(mut self, line: impl Into<Line<'a>>) -> Self {
        self.lines.push(line.into());
        self
    }

    /// A typed answer: `prompt` in the title role, then the field's own line with its cursor.
    #[must_use]
    pub fn input(mut self, prompt: &'a str, field: &'a TextField) -> Self {
        self.input = Some((prompt, field));
        self
    }

    /// What is wrong with the answer, in the error role, wrapped, under the input.
    #[must_use]
    pub fn error(mut self, text: impl Into<Cow<'a, str>>) -> Self {
        self.error = Some(text.into());
        self
    }

    /// The keys, on the bottom border.
    #[must_use]
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// The widest the content grows, in cells.
    #[must_use]
    pub fn width(mut self, content: u16) -> Self {
        self.width = content;
        self
    }

    /// The rows the content needs at `content_width`: the question wrapped, then, with an
    /// input, a blank row and the input row; the error's rows last.
    #[must_use]
    pub fn rows(&self, content_width: u16) -> u16 {
        let question: usize = self
            .lines
            .iter()
            .map(|line| wrap_line(line, content_width).len())
            .sum();
        let input = match self.input {
            Some(_) if question > 0 => 2,
            Some(_) => 1,
            None => 0,
        };
        let error = self
            .error
            .as_deref()
            .map_or(0, |error| wrapped(error, usize::from(content_width)).len());
        u16::try_from(question + input + error).unwrap_or(u16::MAX)
    }

    /// The content width the box asks for: the widest of its lines, its input (prompt, text and
    /// the cursor cell), its title and its hint, each with a rule cell to spare on the border,
    /// at most [`Ask::width`].
    fn wanted_width(&self) -> u16 {
        let widest = self
            .lines
            .iter()
            .map(|line| line_width(line))
            .chain(self.input.map(|(prompt, field)| {
                cells::cell_width(prompt) + cells::cell_width(field.text().unwrap_or("")) + 1
            }))
            .chain(self.error.as_deref().map(cells::cell_width))
            .chain([cells::cell_width(self.title) + 1])
            .chain(self.hint.map(|hint| cells::cell_width(hint) + 1))
            .max()
            .unwrap_or(1);
        u16::try_from(widest)
            .unwrap_or(u16::MAX)
            .min(self.width)
            .max(1)
    }

    /// Draws the question centred over `area` (fading the rest), returning the content rect.
    pub fn render(self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect {
        // What the screen leaves the content: the `Fit` clamp, so the rows are counted at the
        // width they are drawn at.
        let room = area
            .width
            .saturating_sub(MARGIN_COLS)
            .saturating_sub(CHROME);
        let content_w = self.wanted_width().min(room);
        let content_h = self.rows(content_w);
        let mut modal = Modal::new(self.title).size(ModalSize::Fit {
            content_w,
            content_h,
        });
        if let Some(hint) = self.hint {
            modal = modal.hint(hint);
        }
        let content = modal.render(frame, area, theme);

        let mut question: Vec<Line<'_>> = Vec::new();
        for line in &self.lines {
            question.extend(wrap_line(line, content.width));
        }
        // The input row, then the error's rows: what the question gives way to on a screen too
        // short for all of them.
        let mut tail: Vec<Line<'_>> = Vec::new();
        if let Some((prompt, field)) = self.input {
            let prompt_w = u16::try_from(cells::cell_width(prompt)).unwrap_or(u16::MAX);
            let mut spans = vec![Span::styled(prompt, theme.title)];
            spans.extend(
                field
                    .line(content.width.saturating_sub(prompt_w), true, theme)
                    .spans,
            );
            tail.push(Line::from(spans));
        }
        if let Some(error) = self.error.as_deref() {
            tail.extend(
                wrapped(error, usize::from(content.width))
                    .into_iter()
                    .map(|row| Line::styled(row, theme.error)),
            );
        }
        let gap = self.input.is_some() && !question.is_empty();
        let rows = fit_rows(question, gap, tail, content);
        let buffer = frame.buffer_mut();
        for (line, y) in rows.iter().zip(content.y..content.bottom()) {
            buffer.set_line(content.x, y, line, content.width);
        }
        content
    }
}

/// The rows an [`Ask`] draws in `content`: the `question`, a blank row when `gap`, then the
/// `tail` (the input row and the error). When they do not all fit, the blank row goes first,
/// then the question is cut, its last row shown ending in `…`; on a screen with room for less
/// than the tail, the tail's first rows (the input first) are what stays.
fn fit_rows<'l>(
    mut question: Vec<Line<'l>>,
    gap: bool,
    mut tail: Vec<Line<'l>>,
    content: Rect,
) -> Vec<Line<'l>> {
    let height = usize::from(content.height);
    let mut rows = Vec::with_capacity(height);
    if question.len() + usize::from(gap) + tail.len() <= height {
        rows.append(&mut question);
        if gap {
            rows.push(Line::default());
        }
        rows.append(&mut tail);
        return rows;
    }
    tail.truncate(height);
    let room = height - tail.len();
    if question.len() > room {
        question.truncate(room);
        if let Some(last) = question.last_mut() {
            *last = ellipsised(last, content.width);
        }
    }
    rows.append(&mut question);
    rows.append(&mut tail);
    rows
}

/// `line` as one row of at most `width` cells ending in `…`: the mark that the rows after it
/// were cut. Drawn in its line style patched with its first span's, like a wrapped row.
fn ellipsised<'l>(line: &Line<'l>, width: u16) -> Line<'l> {
    let style = line
        .spans
        .first()
        .map_or(line.style, |span| line.style.patch(span.style));
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let text = format!("{}\u{2026}", text.trim_end());
    Line::styled(cells::clip(&text, usize::from(width)), style)
}

/// The cells `line` takes, measured as `ratatui` draws them.
fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| cells::cell_width(&span.content))
        .sum()
}

/// `line` as rows of at most `width` cells: itself when it fits, else its text wrapped
/// ([`wrapped`]) in its line style patched with its first span's.
fn wrap_line<'l>(line: &Line<'l>, width: u16) -> Vec<Line<'l>> {
    if line_width(line) <= usize::from(width) {
        return vec![line.clone()];
    }
    let style = line
        .spans
        .first()
        .map_or(line.style, |span| line.style.patch(span.style));
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    wrapped(&text, usize::from(width))
        .into_iter()
        .map(|row| Line::styled(row, style))
        .collect()
}

/// The help box (MOD-115 M4 E3, E7): a `Fixed` modal of `content_w` plus the chrome by the rows
/// plus the borders, clamped to `area`, titled `title`, with `rows` drawn as given from the
/// content's top. Returns the content rect.
pub fn help(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    rows: &[Line<'_>],
    content_w: u16,
    theme: &Theme,
) -> Rect {
    let h = u16::try_from(rows.len())
        .unwrap_or(u16::MAX)
        .saturating_add(CHROME_ROWS);
    let content = Modal::new(title)
        .size(ModalSize::Fixed {
            w: content_w.saturating_add(CHROME),
            h,
        })
        .render(frame, area, theme);
    let buffer = frame.buffer_mut();
    for (line, y) in rows.iter().zip(content.y..content.bottom()) {
        buffer.set_line(content.x, y, line, content.width);
    }
    content
}

/// What a modal mode answers for a `chord` none of its own acts took (MOD-67 M3 PA-5): `Pass`
/// for the chords `stack`'s global layer lets through (CONTROL, ALT, function keys), so `ctrl-c`
/// quits and `F1` opens help; `Consumed` for every other, so a `q` or a `Tab` goes nowhere.
///
/// One copy for every modal mode, so the pass rule cannot drift between them. Moved from the
/// Settings tab (MOD-115 M3 D2), whose sections and every other tab's modes share it.
#[must_use]
pub fn modal_rest(stack: Stack<'_>, chord: KeyChord) -> Handled {
    if stack.passes(chord) {
        Handled::Pass
    } else {
        Handled::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
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

    /// MOD-115 M4 E6 (finding F3): every modal cuts its border hint by one rule. A hint the
    /// box holds (between the corners, a blank either side) is drawn whole; a wider one is cut
    /// at that width ending in `…`, never mid-word with no mark.
    #[test]
    fn a_hint_wider_than_the_box_is_cut_with_an_ellipsis() {
        // A 20-cell box holds 16 cells of hint.
        let fits = "abcdefghij klmnop";
        let modal = Modal::new("T")
            .size(ModalSize::Fixed { w: 20, h: 4 })
            .hint(&fits[..16]);
        let (_, buffer) = draw(modal, 40, 10, &Theme::default());
        let bottom = row(&buffer, box_of(&buffer).bottom() - 1);
        assert!(bottom.contains(" abcdefghij klmno "), "{bottom}");
        assert!(!bottom.contains('\u{2026}'), "{bottom}");

        let modal = Modal::new("T")
            .size(ModalSize::Fixed { w: 20, h: 4 })
            .hint(fits);
        let (_, buffer) = draw(modal, 40, 10, &Theme::default());
        let bottom = row(&buffer, box_of(&buffer).bottom() - 1);
        assert!(bottom.contains(" abcdefghij klmn\u{2026} "), "{bottom}");

        // An `Ask` shares the rule: its box is clamped to the screen, its hint cut to fit it.
        let (_, buffer) = draw_ask(
            Ask::new("T")
                .line("q")
                .hint("y apply \u{b7} n/Esc stay offline"),
            20,
            8,
            &Theme::default(),
        );
        let bottom = row(&buffer, box_of(&buffer).bottom() - 1);
        assert!(bottom.contains(" y apply \u{b7} n\u{2026} "), "{bottom}");
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

    /// Draws `modal` over `area` of a `width` x `height` screen pre-filled with a red, bold `#`,
    /// so the backdrop's effect on a cell's own style shows.
    fn draw_over_styled(
        modal: Modal<'_>,
        (width, height): (u16, u16),
        area: Rect,
        theme: &Theme,
    ) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                let screen = frame.area();
                let page = ratatui::style::Style::new()
                    .fg(ratatui::style::Color::Red)
                    .add_modifier(Modifier::BOLD);
                for y in 0..screen.height {
                    for x in 0..screen.width {
                        frame.buffer_mut()[(x, y)].set_symbol("#").set_style(page);
                    }
                }
                modal.render(frame, area, theme);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    /// MOD-115 M4 C3: every cell a modal leaves visible in the area it is centred over fades:
    /// DIM, never BOLD, its own colour kept; in both themes.
    #[test]
    fn the_backdrop_outside_the_box_is_dimmed_and_never_bold() {
        for theme in [Theme::default(), Theme::monochrome()] {
            let area = Rect::new(0, 0, 40, 12);
            let modal = Modal::new("Dim").size(ModalSize::Fixed { w: 20, h: 5 });
            let buffer = draw_over_styled(modal, (40, 12), area, &theme);
            let outer = centered(area, 20, 5);
            for y in 0..12 {
                for x in 0..40 {
                    if outer.contains(Position::new(x, y)) {
                        continue;
                    }
                    let cell = &buffer[(x, y)];
                    assert_eq!(cell.symbol(), "#", "({x},{y}) keeps the page's text");
                    assert!(cell.modifier.contains(Modifier::DIM), "({x},{y}) {cell:?}");
                    assert!(
                        !cell.modifier.contains(Modifier::BOLD),
                        "({x},{y}) {cell:?}"
                    );
                    assert_eq!(
                        cell.fg,
                        ratatui::style::Color::Red,
                        "({x},{y}) keeps its colour"
                    );
                }
            }
        }
    }

    #[test]
    fn the_box_itself_is_not_dimmed() {
        for theme in [Theme::default(), Theme::monochrome()] {
            let area = Rect::new(0, 0, 40, 12);
            let modal = Modal::new("Box").size(ModalSize::Fixed { w: 20, h: 5 });
            let buffer = draw_over_styled(modal, (40, 12), area, &theme);
            let outer = centered(area, 20, 5);
            for y in outer.y..outer.bottom() {
                for x in outer.x..outer.right() {
                    let cell = &buffer[(x, y)];
                    assert!(!cell.modifier.contains(Modifier::DIM), "({x},{y}) {cell:?}");
                }
            }
        }
    }

    /// The Hierarchy path picker centres over a section, not the screen: only that section
    /// fades, the rest of the screen is drawn as it was.
    #[test]
    fn a_modal_over_a_smaller_area_dims_only_that_area() {
        let theme = Theme::default();
        let area = Rect::new(10, 2, 30, 10);
        let modal = Modal::new("Pick").size(ModalSize::Fixed { w: 12, h: 4 });
        let buffer = draw_over_styled(modal, (60, 16), area, &theme);
        let outer = centered(area, 12, 4);
        for y in 0..16 {
            for x in 0..60 {
                let at = Position::new(x, y);
                if outer.contains(at) {
                    continue;
                }
                let cell = &buffer[(x, y)];
                if area.contains(at) {
                    assert!(cell.modifier.contains(Modifier::DIM), "({x},{y}) {cell:?}");
                    assert!(
                        !cell.modifier.contains(Modifier::BOLD),
                        "({x},{y}) {cell:?}"
                    );
                } else {
                    assert!(!cell.modifier.contains(Modifier::DIM), "({x},{y}) {cell:?}");
                    assert!(cell.modifier.contains(Modifier::BOLD), "({x},{y}) {cell:?}");
                }
            }
        }
    }

    /// The text of `rect`'s row `y` in `buffer`.
    fn text_in(buffer: &Buffer, rect: Rect, y: u16) -> String {
        (rect.x..rect.right())
            .map(|x| buffer[(x, y)].symbol().to_owned())
            .collect()
    }

    /// Draws `ask` over the whole of a `width` x `height` screen pre-filled with `#`; returns
    /// the content rect and the buffer.
    fn draw_ask(ask: Ask<'_>, width: u16, height: u16, theme: &Theme) -> (Rect, Buffer) {
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
                content = ask.render(frame, area, theme);
            })
            .expect("draw");
        (content, terminal.backend().buffer().clone())
    }

    #[test]
    fn an_ask_draws_its_lines_then_the_input_and_the_hint_on_the_border() {
        let theme = Theme::default();
        let field = TextField::with_text("FEAT-1");
        let ask = Ask::new("Close out")
            .line("Type the item's key to close it.")
            .input("key ", &field)
            .hint("enter close  esc keep");
        let (content, buffer) = draw_ask(ask, 80, 24, &theme);
        let outer = box_of(&buffer);
        assert!(row(&buffer, outer.y).contains(" Close out "));
        assert_eq!(
            text_in(&buffer, content, content.y).trim_end(),
            "Type the item's key to close it."
        );
        assert_eq!(
            text_in(&buffer, content, content.y + 1).trim(),
            "",
            "one blank row before the input"
        );
        let input = text_in(&buffer, content, content.y + 2);
        assert!(input.starts_with("key FEAT-1"), "{input:?}");
        assert!(
            buffer[(content.x, content.y + 2)]
                .modifier
                .contains(Modifier::BOLD),
            "the prompt is in the title role"
        );
        assert_eq!(content.height, 3, "the question, the blank, the input");
        assert!(row(&buffer, outer.bottom() - 1).contains(" enter close  esc keep "));
    }

    #[test]
    fn an_ask_without_an_input_has_no_blank_row() {
        let ask = Ask::new("Cancel")
            .line("Cancel the run?")
            .line("Its steps stop.");
        assert_eq!(ask.rows(40), 2);
        let field = TextField::new();
        let typed = Ask::new("Cancel")
            .line("Cancel the run?")
            .input("> ", &field);
        assert_eq!(typed.rows(40), 3);
    }

    #[test]
    fn the_input_row_keeps_the_cursor_visible() {
        let theme = Theme::monochrome();
        let field = TextField::with_text(&"x".repeat(200));
        let ask = Ask::new("Type")
            .line("A long answer.")
            .input("answer ", &field);
        let (content, buffer) = draw_ask(ask, 50, 12, &theme);
        let y = content.y + 2;
        let cursor = (content.x..content.right())
            .filter(|&x| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
            .count();
        assert_eq!(cursor, 1, "{:?}", text_in(&buffer, content, y));
    }

    #[test]
    fn an_error_goes_under_the_input_in_the_error_role() {
        let theme = Theme::default();
        let field = TextField::with_text("FEAT-2");
        let ask = Ask::new("Close out")
            .line("Type the item's key.")
            .input("key ", &field)
            .error("That is not this item's key.");
        let (content, buffer) = draw_ask(ask, 80, 24, &theme);
        let y = content.y + 3;
        assert_eq!(
            text_in(&buffer, content, y).trim_end(),
            "That is not this item's key."
        );
        assert_eq!(
            buffer[(content.x, y)].fg,
            theme.error.fg.unwrap_or_default()
        );
        assert_eq!(content.height, 4);
    }

    #[test]
    fn a_long_question_wraps_and_the_box_grows() {
        let theme = Theme::default();
        let question = "word ".repeat(30);
        let ask = Ask::new("Long").line(question.trim_end()).width(40);
        assert_eq!(ask.rows(40), 4, "149 cells in rows of 40");
        let (content, buffer) = draw_ask(ask, 80, 24, &theme);
        assert_eq!(content.width, 40);
        assert_eq!(content.height, 4);
        let outer = box_of(&buffer);
        assert_eq!(outer.height, 4 + 2);
        let shown: String = (content.y..content.bottom())
            .map(|y| text_in(&buffer, content, y).trim_end().to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(shown, question.trim_end(), "no word is lost");
    }

    #[test]
    fn an_ask_fits_its_content_and_keeps_the_margin() {
        let theme = Theme::default();
        let area = Rect::new(0, 0, 80, 24);
        let ask = Ask::new("Abort").line("Abort the edit?").hint("y abort");
        let (_, buffer) = draw_ask(ask, 80, 24, &theme);
        // "Abort the edit?" is 15 cells, wider than the hint (7 + 1) and the title.
        assert_eq!(box_of(&buffer), centered(area, 15 + CHROME, 1 + 2));
        let ask = Ask::new("Abort")
            .line("Abort?")
            .hint("y abort  any other key keeps it");
        let (content, _) = draw_ask(ask, 80, 24, &theme);
        assert_eq!(content.width, 31 + 1, "the hint and a cell of rule fit");
        // A screen too small for the question: the box keeps the margin and the text wraps.
        let ask = Ask::new("Abort").line("word ".repeat(40).trim_end().to_owned());
        let (content, buffer) = draw_ask(ask, 30, 10, &theme);
        let outer = box_of(&buffer);
        assert!(outer.x >= 2 && outer.right() <= 28, "{outer:?}");
        assert!(outer.y >= 1 && outer.bottom() <= 9, "{outer:?}");
        assert_eq!(content.width, 30 - 4 - CHROME);
    }

    /// On a screen too short for the whole question, the input and the error stay on screen:
    /// the blank row goes first, then the question is cut, its last row ending in `…`.
    #[test]
    fn a_short_screen_cuts_the_question_and_keeps_the_input_and_the_error() {
        let theme = Theme::default();
        let field = TextField::with_text("FEAT-1");
        let question = "word ".repeat(60);
        let ask = Ask::new("Close out")
            .line(question.trim_end().to_owned())
            .input("key ", &field)
            .error("That is not this item's key.");
        let (content, buffer) = draw_ask(ask, 60, 8, &theme);
        assert_eq!(content.height, 4);
        let rows: Vec<String> = (content.y..content.bottom())
            .map(|y| text_in(&buffer, content, y).trim_end().to_owned())
            .collect();
        assert!(rows[0].starts_with("word word"), "{rows:?}");
        assert!(rows[1].ends_with('…'), "the cut is marked: {rows:?}");
        assert!(rows[2].starts_with("key FEAT-1"), "{rows:?}");
        assert_eq!(rows[3], "That is not this item's key.");
        assert_eq!(
            buffer[(content.x, content.bottom() - 1)].fg,
            theme.error.fg.unwrap_or_default()
        );
        // Whole question and the input fit once the blank row goes: nothing is cut.
        let field = TextField::with_text("x");
        let ask = Ask::new("Q").line("one").line("two").input("> ", &field);
        let (content, buffer) = draw_ask(ask, 40, 7, &theme);
        assert_eq!(content.height, 3);
        let rows: Vec<String> = (content.y..content.bottom())
            .map(|y| text_in(&buffer, content, y).trim_end().to_owned())
            .collect();
        assert_eq!(rows[0], "one");
        assert_eq!(rows[1], "two");
        assert!(rows[2].starts_with("> x"), "{rows:?}");
        // Room for the input alone: the input it is.
        let field = TextField::with_text("FEAT-1");
        let ask = Ask::new("Q")
            .line("A question.")
            .input("key ", &field)
            .error("bad");
        let (content, buffer) = draw_ask(ask, 40, 5, &theme);
        assert_eq!(content.height, 1);
        assert!(
            text_in(&buffer, content, content.y).starts_with("key FEAT-1"),
            "{:?}",
            text_in(&buffer, content, content.y)
        );
    }

    #[test]
    fn an_ask_under_monochrome_names_no_colour() {
        let theme = Theme::monochrome();
        let field = TextField::with_text("x");
        let ask = Ask::new("Mono")
            .line("Sure?")
            .input("> ", &field)
            .error("no")
            .hint("y yes");
        let (_, buffer) = draw_ask(ask, 40, 12, &theme);
        let outer = box_of(&buffer);
        for y in outer.y..outer.bottom() {
            for x in outer.x..outer.right() {
                let cell = &buffer[(x, y)];
                assert_eq!(cell.fg, ratatui::style::Color::Reset, "({x},{y})");
                assert_eq!(cell.bg, ratatui::style::Color::Reset, "({x},{y})");
            }
        }
    }

    #[test]
    fn the_help_box_is_as_tall_as_its_rows_and_titled() {
        let theme = Theme::default();
        let area = Rect::new(0, 0, 80, 24);
        let rows = [
            Line::raw("? help"),
            Line::raw("q quit"),
            Line::raw("tab next"),
        ];
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        let mut content = Rect::default();
        terminal
            .draw(|frame| content = help(frame, frame.area(), "Keys", &rows, 30, &theme))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let outer = centered(area, 30 + CHROME, 3 + 2);
        assert!(row(&buffer, outer.y).contains(" Keys "));
        assert_eq!(content, Rect::new(outer.x + 1, outer.y + 1, 30, 3));
        for (i, line) in ["? help", "q quit", "tab next"].iter().enumerate() {
            let y = content.y + u16::try_from(i).expect("small");
            assert_eq!(text_in(&buffer, content, y).trim_end(), *line);
        }
    }

    #[test]
    fn a_help_box_taller_than_the_screen_clamps() {
        let theme = Theme::default();
        let rows: Vec<Line<'_>> = (0..40).map(|i| Line::raw(format!("row {i}"))).collect();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("terminal");
        let mut content = Rect::default();
        terminal
            .draw(|frame| content = help(frame, frame.area(), "Keys", &rows, 200, &theme))
            .expect("draw");
        assert_eq!(content, Rect::new(1, 1, 60 - CHROME, 12 - 2));
        let buffer = terminal.backend().buffer().clone();
        assert_eq!(text_in(&buffer, content, 1).trim_end(), "row 0");
        assert_eq!(text_in(&buffer, content, 10).trim_end(), "row 9");
    }

    /// MOD-67 M3 PA-5: a modal mode passes on what its global layer lets through, so `ctrl-c`
    /// quits and `F1` opens help, and swallows every other chord none of its acts took.
    #[test]
    fn a_modal_rests_on_what_its_global_layer_passes() {
        use crossterm::event::{KeyCode, KeyModifiers};

        use crate::keys::views;

        let stack = views::CONNECTION_CONFIRM;
        let chord = |code, mods| KeyChord::new(code, mods);
        assert_eq!(
            modal_rest(stack, chord(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Handled::Pass
        );
        assert_eq!(
            modal_rest(stack, chord(KeyCode::F(1), KeyModifiers::NONE)),
            Handled::Pass
        );
        assert_eq!(
            modal_rest(stack, chord(KeyCode::Char('q'), KeyModifiers::NONE)),
            Handled::Consumed
        );
        assert_eq!(
            modal_rest(stack, chord(KeyCode::Tab, KeyModifiers::NONE)),
            Handled::Consumed
        );
    }
}
