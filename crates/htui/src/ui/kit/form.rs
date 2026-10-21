//! Form rows (MOD-115 M3 D3): a shared label column, one value per row (text, toggle, choice,
//! secret, read-only), the focus marker (`kit::list`'s gutter and band), the focused row's help
//! under it and an error under the row it concerns. Rendering only: the arrow-changes-value
//! behaviour is MOD-123.
//!
//! A row reads `[gutter 1][label column L][gap 2][value]`. `L` is the widest label and one cell
//! (or [`Form::label_width`]), counted over every field, those scrolled off included, so the
//! column never jumps; it is capped at a third of the width, and a longer label is cut with `…`
//! (MOD-60). The focused row's gutter is [`GUTTER`] in the caller's accent (`ctx.accent()`).
//! While browsing, the row goes through [`selection_band`] as a list row does; while editing,
//! only the label column is banded, so the field's own cursor (`theme.cursor`, reversed) stays
//! visible in monochrome, where a reversed band would hide a reversed cursor.
//!
//! Under the focused row come its help lines (`theme.dim`), then its error lines
//! (`theme.error`); an unfocused row with an error shows only the error. Both are wrapped to the
//! value column and indented to it, so help sits on the line under the field it is about (D3,
//! the audit's item 3).

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::kit::list::{GUTTER, ellipsize, selection_band};
use crate::ui::kit::text::wrapped;
use crate::ui::text_field::TextField;
use crate::ui::{Theme, cells};

/// The blank cells between the label column and the value.
const GAP: u16 = 2;

/// The toggle's glyphs and words.
const ON: &str = "\u{25cf} on";
const OFF: &str = "\u{25cb} off";

/// A stored secret: the dots stand for it, never its length.
const DOTS: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// What a form row shows in its value column.
#[derive(Debug, Clone)]
pub enum Value<'a> {
    /// A text field: its own [`TextField::line`], the cursor shown only while the row is
    /// focused and the form is [`editing`](Form::editing).
    Text(&'a TextField),
    /// `● on` (base) or `○ off` (dim).
    Toggle(bool),
    /// `‹ value ›`: the marks dim, the value base. The caller passes a `kit::label` word, never an
    /// enum's own name.
    Choice(&'a str),
    /// `•••••••• stored` (the dots base, the word dim) when stored, else `not stored` (dim).
    Secret(bool),
    /// Plain text, wrapped under the value column ([`wrapped`]).
    ReadOnly(Cow<'a, str>),
    /// Pre-rendered rows, one per line, each cut with `…` to the value column: a `TextArea`'s
    /// lines, a styled value.
    Lines(Vec<Line<'static>>),
}

/// One labelled row of a [`Form`].
#[derive(Debug, Clone)]
pub struct Field<'a> {
    label: &'a str,
    value: Value<'a>,
    help: Option<Cow<'a, str>>,
    error: Option<Cow<'a, str>>,
    style: Option<Style>,
}

impl<'a> Field<'a> {
    /// A row labelled `label` showing `value`, with no help, error or style of its own.
    #[must_use]
    pub fn new(label: &'a str, value: Value<'a>) -> Self {
        Self {
            label,
            value,
            help: None,
            error: None,
            style: None,
        }
    }

    /// Help shown under the row while it is focused.
    #[must_use]
    pub fn help(mut self, text: impl Into<Cow<'a, str>>) -> Self {
        self.help = Some(text.into());
        self
    }

    /// An error shown under the row whatever has focus.
    #[must_use]
    pub fn error(mut self, text: impl Into<Cow<'a, str>>) -> Self {
        self.error = Some(text.into());
        self
    }

    /// The whole row's base style (a dimmed or withdrawn row); the value's own styles and the
    /// band sit on top of it.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }
}

/// One focusable item of a form.
#[derive(Debug, Clone)]
enum Item<'a> {
    Field(Field<'a>),
    Heading(Cow<'a, str>),
    Blank,
}

/// A form under construction: fields, headings and blanks, one of them focused.
#[derive(Debug, Clone, Default)]
pub struct Form<'a> {
    items: Vec<Item<'a>>,
    focus: Option<usize>,
    editing: bool,
    label_width: Option<u16>,
}

impl<'a> Form<'a> {
    /// An empty form, nothing focused, browsing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a field.
    #[must_use]
    pub fn field(mut self, field: Field<'a>) -> Self {
        self.items.push(Item::Field(field));
        self
    }

    /// Adds a group row in `theme.title`. It is focusable and counts as an item.
    #[must_use]
    pub fn heading(mut self, text: impl Into<Cow<'a, str>>) -> Self {
        self.items.push(Item::Heading(text.into()));
        self
    }

    /// Adds an empty row. It is focusable and counts as an item.
    #[must_use]
    pub fn blank(mut self) -> Self {
        self.items.push(Item::Blank);
        self
    }

    /// Focuses `item`, an index over the fields, headings and blanks in the order added.
    #[must_use]
    pub fn focus(mut self, item: usize) -> Self {
        self.focus = Some(item);
        self
    }

    /// Whether the focused row's value is being typed into: only its label is banded, and a
    /// [`Value::Text`] shows its cursor.
    #[must_use]
    pub fn editing(mut self, on: bool) -> Self {
        self.editing = on;
        self
    }

    /// Sets the label column to `cells`, so two forms (or a form and a key-value list) line up.
    #[must_use]
    pub fn label_width(mut self, cells: u16) -> Self {
        self.label_width = Some(cells);
        self
    }

    /// The label column's cells in a form `width` wide: [`label_width`](Self::label_width), else
    /// the widest label and one, capped at `width / 3`, at least 1.
    #[must_use]
    pub fn label_column(&self, width: u16) -> u16 {
        let wanted = self.label_width.unwrap_or_else(|| {
            let widest = self
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Field(field) => Some(cells::cell_width(field.label)),
                    Item::Heading(_) | Item::Blank => None,
                })
                .max()
                .unwrap_or(0);
            u16::try_from(widest + 1).unwrap_or(u16::MAX)
        });
        wanted.min(width / 3).max(1)
    }

    /// The form as lines `width` cells wide, one block of rows per item.
    #[must_use]
    pub fn lines(&self, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>> {
        self.blocks(width, accent, theme).concat()
    }

    /// The first line of the focused item, to scroll it into view; `None` when nothing (or
    /// nothing that exists) is focused.
    #[must_use]
    pub fn focus_line(&self, width: u16) -> Option<usize> {
        let focus = self.focus.filter(|focus| *focus < self.items.len())?;
        // Line counts do not depend on styles, so any theme measures them.
        let blocks = self.blocks(width, Style::new(), &Theme::default());
        Some(blocks[..focus].iter().map(Vec::len).sum())
    }

    /// Draws the form's lines from `offset` into `area`, one per row.
    pub fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        offset: usize,
        accent: Style,
        theme: &Theme,
    ) {
        let lines = self.lines(area.width, accent, theme);
        let buf = frame.buffer_mut();
        for (y, line) in (area.y..area.bottom()).zip(lines.iter().skip(offset)) {
            buf.set_line(area.x, y, line, area.width);
        }
    }

    /// Each item's rows.
    fn blocks(&self, width: u16, accent: Style, theme: &Theme) -> Vec<Vec<Line<'static>>> {
        let label = self.label_column(width);
        let value = width.saturating_sub(1 + label + GAP);
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let focused = self.focus == Some(index);
                let gutter = if focused {
                    Span::styled(GUTTER, accent)
                } else {
                    Span::raw(" ")
                };
                let content = width.saturating_sub(1);
                match item {
                    Item::Heading(text) => {
                        let line = Line::from(Span::styled(text.to_string(), theme.title));
                        vec![marked(gutter, line, focused, content, theme)]
                    }
                    Item::Blank if focused => {
                        vec![marked(gutter, Line::default(), true, content, theme)]
                    }
                    Item::Blank => vec![Line::default()],
                    Item::Field(field) => {
                        let rows = Rows {
                            label,
                            value,
                            focused,
                            editing: focused && self.editing,
                        };
                        rows.field(field, gutter, theme)
                    }
                }
            })
            .collect()
    }
}

/// `line` behind `gutter`: banded to `width` when `focused`, else cut to it.
fn marked(
    gutter: Span<'static>,
    line: Line<'_>,
    focused: bool,
    width: u16,
    theme: &Theme,
) -> Line<'static> {
    let line = if focused {
        selection_band(line, width, theme)
    } else {
        ellipsize(line, width)
    };
    let mut spans = vec![gutter];
    spans.extend(line.spans);
    Line::from(spans).style(line.style)
}

/// The geometry and focus state one field's rows are laid out with.
struct Rows {
    label: u16,
    value: u16,
    focused: bool,
    editing: bool,
}

impl Rows {
    /// `field`'s rows: its value rows (the first one labelled), then its help while focused,
    /// then its error.
    fn field(&self, field: &Field<'_>, gutter: Span<'static>, theme: &Theme) -> Vec<Line<'static>> {
        let label = usize::from(self.label);
        let gap = Span::raw(" ".repeat(usize::from(GAP)));
        let mut lines: Vec<Line<'static>> = self
            .values(&field.value, theme)
            .into_iter()
            .enumerate()
            .map(|(row, value)| {
                let text = if row == 0 {
                    cells::fit(field.label, label)
                } else {
                    " ".repeat(label)
                };
                let name = Line::from(Span::styled(text, theme.base));
                let mut line = if self.focused && !self.editing {
                    let mut spans = name.spans;
                    spans.push(gap.clone());
                    spans.extend(value.spans);
                    let banded = selection_band(
                        Line::from(spans).style(value.style),
                        self.label + GAP + self.value,
                        theme,
                    );
                    let mut out = vec![gutter.clone()];
                    out.extend(banded.spans);
                    Line::from(out).style(banded.style)
                } else {
                    let name = if self.focused {
                        selection_band(name, self.label, theme)
                    } else {
                        name
                    };
                    let mut out = vec![gutter.clone()];
                    out.extend(name.spans);
                    out.push(gap.clone());
                    out.extend(value.spans);
                    Line::from(out).style(value.style)
                };
                if let Some(style) = field.style {
                    line = line.patch_style(style);
                }
                line
            })
            .collect();
        let notes = self
            .focused
            .then_some(field.help.as_deref().map(|help| (help, theme.dim)))
            .flatten()
            .into_iter()
            .chain(field.error.as_deref().map(|error| (error, theme.error)));
        let indent = " ".repeat(1 + label + usize::from(GAP));
        for (text, style) in notes {
            for row in wrapped(text, usize::from(self.value)) {
                lines.push(Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::styled(row, style),
                ]));
            }
        }
        lines
    }

    /// `value`'s rows, each at most the value column wide; at least one.
    fn values(&self, value: &Value<'_>, theme: &Theme) -> Vec<Line<'static>> {
        let width = self.value;
        let rows = match value {
            Value::Text(field) => vec![field.line(width, self.editing, theme)],
            Value::Toggle(true) => vec![Line::from(Span::styled(ON, theme.base))],
            Value::Toggle(false) => vec![Line::from(Span::styled(OFF, theme.dim))],
            Value::Choice(word) => vec![Line::from(vec![
                Span::styled("\u{2039} ", theme.dim),
                Span::styled((*word).to_owned(), theme.base),
                Span::styled(" \u{203a}", theme.dim),
            ])],
            Value::Secret(true) => vec![Line::from(vec![
                Span::styled(DOTS, theme.base),
                Span::styled(" stored", theme.dim),
            ])],
            Value::Secret(false) => vec![Line::from(Span::styled("not stored", theme.dim))],
            Value::ReadOnly(text) => wrapped(text, usize::from(width))
                .into_iter()
                .map(|row| Line::from(Span::styled(row, theme.base)))
                .collect(),
            Value::Lines(lines) => lines.clone(),
        };
        if rows.is_empty() {
            return vec![Line::default()];
        }
        rows.into_iter().map(|row| ellipsize(row, width)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::list::GUTTER;
    use crate::ui::kit::text::wrapped;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    fn accent() -> Style {
        Style::new().fg(Color::Magenta)
    }

    /// Each line's text, its trailing blanks dropped.
    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    /// The style of the span holding `text` in `line`.
    fn style_of(line: &Line<'_>, text: &str) -> Style {
        line.spans
            .iter()
            .find(|span| span.content.contains(text))
            .unwrap_or_else(|| panic!("no span holds {text:?} in {line:?}"))
            .style
    }

    fn draw(form: &Form<'_>, width: u16, height: u16, offset: usize, theme: &Theme) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| form.render(frame, frame.area(), offset, accent(), theme))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn each_value_kind_reads_as_its_words() {
        let theme = Theme::default();
        let form = Form::new()
            .field(Field::new("a", Value::Toggle(true)))
            .field(Field::new("b", Value::Toggle(false)))
            .field(Field::new("c", Value::Choice("later")))
            .field(Field::new("d", Value::Secret(true)))
            .field(Field::new("e", Value::Secret(false)));
        let lines = form.lines(40, accent(), &theme);
        assert_eq!(
            texts(&lines),
            [
                " a   ● on",
                " b   ○ off",
                " c   ‹ later ›",
                " d   •••••••• stored",
                " e   not stored",
            ]
        );
        assert_eq!(style_of(&lines[0], "● on"), theme.base);
        assert_eq!(style_of(&lines[1], "○ off"), theme.dim);
        assert_eq!(style_of(&lines[2], "‹"), theme.dim, "the marks are dim");
        assert_eq!(style_of(&lines[2], "later"), theme.base, "the word is not");
        assert_eq!(style_of(&lines[3], "••••••••"), theme.base);
        assert_eq!(style_of(&lines[3], "stored"), theme.dim);
        assert_eq!(style_of(&lines[4], "not stored"), theme.dim);
    }

    #[test]
    fn a_text_value_is_the_fields_own_line_and_shows_no_cursor_unless_editing() {
        let theme = Theme::default();
        let field = TextField::with_text("htui");
        let form = Form::new().field(Field::new("name", Value::Text(&field)));
        let lines = form.lines(30, accent(), &theme);
        assert_eq!(texts(&lines), [" name   htui"]);
        assert!(
            lines[0]
                .spans
                .iter()
                .all(|span| !span.style.add_modifier.contains(Modifier::REVERSED)),
            "{lines:?}"
        );
    }

    #[test]
    fn a_read_only_value_wraps_under_its_own_column_and_lines_are_cut() {
        let theme = Theme::default();
        // Width 20, label column 3: the value column is 14 cells, from cell 6.
        let form = Form::new()
            .field(Field::new(
                "ab",
                Value::ReadOnly("one two three four five six".into()),
            ))
            .field(Field::new(
                "cd",
                Value::Lines(vec![
                    Line::from("first"),
                    Line::from("a very long second line here"),
                ]),
            ))
            .field(Field::new("ef", Value::ReadOnly("".into())));
        assert_eq!(
            texts(&form.lines(20, accent(), &theme)),
            [
                " ab   one two three",
                "      four five six",
                " cd   first",
                "      a very long s…",
                " ef",
            ]
        );
        // Focused while browsing, every row of the value carries the marker and the band.
        let lines = form.clone().focus(0).lines(20, accent(), &theme);
        for line in &lines[..2] {
            assert_eq!(line.spans[0].content, GUTTER);
            assert_eq!(line.width(), 20, "the band runs the full width");
        }
        assert_eq!(lines[2].spans[0].content, " ");
    }

    #[test]
    fn the_label_column_is_shared_and_a_long_label_is_cut_at_a_third() {
        let theme = Theme::default();
        let form = Form::new()
            .field(Field::new("name", Value::Toggle(true)))
            .field(Field::new("a much longer label here", Value::Toggle(false)));
        assert_eq!(form.label_column(60), 20, "25 wanted, a third of 60 given");
        assert_eq!(form.label_column(90), 25, "the longest label and one cell");
        assert_eq!(
            texts(&form.lines(60, accent(), &theme)),
            [
                format!(" name{}  ● on", " ".repeat(16)),
                " a much longer label…  ○ off".to_owned(),
            ]
        );
        assert_eq!(form.clone().label_width(14).label_column(60), 14);
        assert_eq!(form.clone().label_width(30).label_column(60), 20, "capped");
        assert_eq!(Form::new().heading("x").label_column(60), 1, "at least 1");
        // The column counts every field, those scrolled off included.
        let buffer = draw(&form, 60, 1, 1, &theme);
        assert_eq!(row(&buffer, 0), " a much longer label…  ○ off");
        let buffer = draw(&form.clone().label_width(8), 60, 2, 0, &theme);
        assert_eq!(row(&buffer, 0), " name      ● on");
        assert_eq!(row(&buffer, 1), " a much …  ○ off");
    }

    #[test]
    fn the_focused_row_is_banded_while_browsing() {
        let theme = Theme::default();
        let field = TextField::with_text("htui");
        let form = Form::new()
            .field(Field::new("name", Value::Text(&field)))
            .field(Field::new("mode", Value::Toggle(true)))
            .focus(1);
        let lines = form.lines(30, accent(), &theme);
        assert_eq!(lines[0].spans[0].content, " ");
        assert_eq!(lines[1].spans[0].content, GUTTER);
        assert_eq!(lines[1].spans[0].style, accent());
        assert_eq!(lines[1].width(), 30);
        for span in &lines[1].spans[1..] {
            assert_eq!(span.style.bg, theme.selected.bg, "{span:?}");
        }
    }

    #[test]
    fn while_editing_only_the_label_is_banded_and_the_cursor_shows_in_monochrome() {
        let theme = Theme::monochrome();
        let field = TextField::with_text("htui");
        let form = Form::new()
            .field(Field::new("name", Value::Text(&field)))
            .field(Field::new("mode", Value::Toggle(true)))
            .focus(0)
            .editing(true);
        // Label column 5 (cells 1..=5), the gap 6..=7, `htui` 8..=11, the cursor at 12.
        let buffer = draw(&form, 30, 2, 0, &theme);
        assert_eq!(buffer[(0, 0)].symbol(), GUTTER);
        let reversed = |x: u16| buffer[(x, 0)].modifier.contains(Modifier::REVERSED);
        assert!((1..=5).all(reversed), "the label column is banded");
        assert!(!(6..=11).any(reversed), "the gap and the text are not");
        assert!(reversed(12), "the cursor is visible");
        assert!(!(13..30).any(reversed));
        assert_eq!(row(&buffer, 0), "▌name   htui");
        // Browsing the same row bands it whole, and the field shows no cursor.
        let buffer = draw(&form.clone().editing(false), 30, 2, 0, &theme);
        assert!((1..30).all(|x| buffer[(x, 0)].modifier.contains(Modifier::REVERSED)));
    }

    #[test]
    fn help_shows_under_the_focused_row_only_and_an_error_under_its_own_row_always() {
        let theme = Theme::default();
        let help = "a short name, lowercase letters and digits only";
        let form = Form::new()
            .field(
                Field::new("name", Value::Toggle(true))
                    .help(help)
                    .error("taken"),
            )
            .field(Field::new("slug", Value::Toggle(false)).error("must be on"));
        // Width 30, label column 5: help and errors wrap to 22 cells, indented by 8.
        let indent = " ".repeat(8);
        let help_rows = wrapped(help, 22);
        assert!(help_rows.len() > 1, "the help wraps: {help_rows:?}");
        let mut expected = vec!["▌name   ● on".to_owned()];
        expected.extend(help_rows.iter().map(|row| format!("{indent}{row}")));
        expected.extend([
            format!("{indent}taken"),
            " slug   ○ off".to_owned(),
            format!("{indent}must be on"),
        ]);
        let focused = form.clone().focus(0).lines(30, accent(), &theme);
        assert_eq!(texts(&focused), expected);
        assert_eq!(style_of(&focused[1], help_rows[0].as_str()), theme.dim);
        let error_row = &focused[1 + help_rows.len()];
        assert_eq!(style_of(error_row, "taken"), theme.error);
        let mut unfocused = [
            " name   ● on".to_owned(),
            format!("{indent}taken"),
            " slug   ○ off".to_owned(),
            format!("{indent}must be on"),
        ];
        assert_eq!(texts(&form.lines(30, accent(), &theme)), unfocused);
        unfocused[2] = "▌slug   ○ off".to_owned();
        assert_eq!(
            texts(&form.clone().focus(1).lines(30, accent(), &theme)),
            unfocused,
            "the other row focused: `name` keeps its error and loses its help"
        );
    }

    #[test]
    fn headings_and_blanks_are_focusable_and_numbered_with_the_fields() {
        let theme = Theme::default();
        let form = Form::new()
            .heading("project htui")
            .field(Field::new("a", Value::Toggle(true)))
            .blank()
            .field(Field::new("b", Value::Toggle(false)));
        let lines = form.lines(30, accent(), &theme);
        assert_eq!(
            texts(&lines),
            [" project htui", " a   ● on", "", " b   ○ off"]
        );
        assert_eq!(style_of(&lines[0], "project htui"), theme.title);
        let heading = form.clone().focus(0).lines(30, accent(), &theme);
        assert_eq!(heading[0].spans[0].content, GUTTER);
        assert_eq!(heading[0].width(), 30);
        let blank = form.clone().focus(2).lines(30, accent(), &theme);
        assert_eq!(blank[2].spans[0].content, GUTTER);
        assert_eq!(blank[1].spans[0].content, " ");
        let last = form.clone().focus(3).lines(30, accent(), &theme);
        assert_eq!(last[3].spans[0].content, GUTTER);
    }

    #[test]
    fn focus_line_is_the_first_line_of_the_focused_row() {
        // Field `a` takes two rows of value and one of error; then a heading, a blank, `b`.
        let form = Form::new()
            .field(
                Field::new("a", Value::ReadOnly("one two three four five six".into()))
                    .help("some help for a")
                    .error("bad"),
            )
            .heading("group")
            .blank()
            .field(Field::new("b", Value::Toggle(true)));
        assert_eq!(form.focus_line(20), None, "nothing focused");
        assert_eq!(form.clone().focus(0).focus_line(20), Some(0));
        assert_eq!(form.clone().focus(1).focus_line(20), Some(3));
        assert_eq!(form.clone().focus(3).focus_line(20), Some(5));
        assert_eq!(form.clone().focus(9).focus_line(20), None, "past the end");
        let lines = form.clone().focus(3).lines(20, accent(), &Theme::default());
        assert_eq!(lines[5].spans[0].content, GUTTER);
    }

    #[test]
    fn render_draws_from_the_offset_into_the_area() {
        let theme = Theme::default();
        let form = Form::new()
            .field(Field::new("a", Value::Toggle(true)))
            .field(Field::new("b", Value::Toggle(false)))
            .field(Field::new("c", Value::Choice("must")));
        let buffer = draw(&form, 20, 2, 1, &theme);
        assert_eq!(row(&buffer, 0), " b   ○ off");
        assert_eq!(row(&buffer, 1), " c   ‹ must ›");
        let buffer = draw(&form, 20, 2, 9, &theme);
        assert_eq!(row(&buffer, 0), "", "an offset past the end draws nothing");
    }

    #[test]
    fn a_field_style_is_the_whole_rows_base() {
        let theme = Theme::default();
        let form = Form::new().field(Field::new("old", Value::Toggle(true)).style(theme.dim));
        let buffer = draw(&form, 20, 1, 0, &theme);
        assert_eq!(buffer[(1, 0)].fg, Color::Indexed(244), "the label");
        assert_eq!(buffer[(7, 0)].symbol(), "●");
        assert_eq!(buffer[(7, 0)].fg, Color::Indexed(244), "the value");
    }

    #[test]
    fn monochrome_names_no_colour_and_keeps_every_role_readable() {
        let theme = Theme::monochrome();
        let field = TextField::with_text("x");
        let form = Form::new()
            .heading("group")
            .field(
                Field::new("a", Value::Text(&field))
                    .help("help")
                    .error("bad"),
            )
            .field(Field::new("b", Value::Toggle(false)))
            .field(Field::new("c", Value::Choice("later")))
            .field(Field::new("d", Value::Secret(true)))
            .focus(1);
        let accent = theme.tab_accent("settings");
        let lines = form.lines(30, accent, &theme);
        for line in &lines {
            for span in &line.spans {
                for colour in [span.style.fg, span.style.bg] {
                    assert!(matches!(colour, None | Some(Color::Reset)), "{span:?}");
                }
            }
        }
        assert!(
            style_of(&lines[2], "help")
                .add_modifier
                .contains(Modifier::DIM)
        );
        assert!(
            style_of(&lines[3], "bad")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of(&lines[4], "○ off")
                .add_modifier
                .contains(Modifier::DIM)
        );
        assert!(
            style_of(&lines[0], "group")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of(&lines[1], "a")
                .add_modifier
                .contains(Modifier::REVERSED),
            "the browsing band"
        );
        assert!(
            lines[1].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD),
            "the gutter"
        );
    }
}
