//! One small multi-line text area (MOD-7 milestone 2, PRD D3): the quirks editor's widget, the
//! multi-line sibling of [`TextField`](crate::ui::TextField). Hard lines only (no soft wrap),
//! counted in `char`s. `ctrl-s` submits, because `Enter` breaks the line and no terminal mode
//! that reports `Ctrl+Enter` is enabled (plan OQ-16). No history, selection, undo, mask,
//! bracketed paste or `Zeroizing`: what it holds is not secret.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};

use crate::ui::{FieldOutcome, Theme};

/// A multi-line buffer with a `(row, col)` cursor, counted in `char`s.
#[derive(Clone)]
pub struct TextArea {
    /// The lines, never empty: an empty area is one empty line. No line holds `\n` or `\r`.
    lines: Vec<String>,
    /// Line index, `0..lines.len()`.
    row: usize,
    /// Char index into `lines[row]`, `0..=chars`.
    col: usize,
}

impl Default for TextArea {
    /// One empty line, cursor at `(0, 0)`.
    fn default() -> Self {
        todo!()
    }
}

/// Never the text: sections derive `Debug`. Prints `line_count` and `len` only.
impl core::fmt::Debug for TextArea {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        todo!()
    }
}

impl TextArea {
    /// An empty area: one empty line, cursor at `(0, 0)`.
    #[must_use]
    pub fn new() -> Self {
        todo!()
    }

    /// `text` split into lines on `\n`, after `\r\n` and a lone `\r` become `\n` (D59); cursor at
    /// the end of the last line.
    #[must_use]
    pub fn with_text(_text: &str) -> Self {
        todo!()
    }

    /// Feeds one key.
    pub fn on_key(&mut self, _key: KeyEvent) -> FieldOutcome {
        todo!()
    }

    /// The lines joined by `\n`.
    #[must_use]
    pub fn text(&self) -> String {
        todo!()
    }

    /// Whether the area is one empty line.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// How many chars, the `\n` between lines included (so `len() == text().chars().count()`).
    #[must_use]
    pub fn len(&self) -> usize {
        todo!()
    }

    /// How many lines (at least 1).
    #[must_use]
    pub fn line_count(&self) -> usize {
        todo!()
    }

    /// The cursor as `(row, col)`.
    #[must_use]
    pub const fn cursor(&self) -> (usize, usize) {
        todo!()
    }

    /// At most `height` lines, each at most `width` cells.
    #[must_use]
    pub fn lines(
        &self,
        _width: u16,
        _height: u16,
        _focused: bool,
        _theme: &Theme,
    ) -> Vec<Line<'static>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::TextField;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn chord(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// An area holding `text` with the cursor put at `(row, col)` directly.
    fn at(text: &str, row: usize, col: usize) -> TextArea {
        let mut area = TextArea::with_text(text);
        area.row = row;
        area.col = col;
        area
    }

    /// A rendered line's text, span contents concatenated.
    fn plain(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn enter_splits_the_line_at_the_cursor() {
        let mut area = TextArea::with_text("abcd");
        assert_eq!(area.on_key(key(KeyCode::Left)), FieldOutcome::Consumed);
        assert_eq!(area.on_key(key(KeyCode::Left)), FieldOutcome::Consumed);
        assert_eq!(area.on_key(key(KeyCode::Enter)), FieldOutcome::Consumed);
        assert_eq!(area.text(), "ab\ncd");
        assert_eq!(area.cursor(), (1, 0));
        assert_eq!(area.line_count(), 2);

        // `SHIFT` is how a terminal reports a capital; it does not turn `Enter` into anything else.
        let mut shifted = TextArea::with_text("xy");
        assert_eq!(
            shifted.on_key(chord(KeyCode::Enter, KeyModifiers::SHIFT)),
            FieldOutcome::Consumed
        );
        assert_eq!(shifted.text(), "xy\n");
        assert_eq!(shifted.cursor(), (1, 0));
    }

    #[test]
    fn backspace_at_column_zero_joins_the_previous_line() {
        let mut area = at("ab\ncd", 1, 0);
        assert_eq!(area.on_key(key(KeyCode::Backspace)), FieldOutcome::Consumed);
        assert_eq!(area.text(), "abcd");
        assert_eq!(area.cursor(), (0, 2));

        // At `(0, 0)` there is nothing before the cursor.
        let mut start = at("ab\ncd", 0, 0);
        assert_eq!(
            start.on_key(key(KeyCode::Backspace)),
            FieldOutcome::Consumed
        );
        assert_eq!(start.text(), "ab\ncd");
        assert_eq!(start.cursor(), (0, 0));
    }

    #[test]
    fn delete_at_the_end_joins_the_next_line() {
        let mut area = at("ab\ncd", 0, 2);
        assert_eq!(area.on_key(key(KeyCode::Delete)), FieldOutcome::Consumed);
        assert_eq!(area.text(), "abcd");
        assert_eq!(area.cursor(), (0, 2));

        // At the end of the last line there is nothing after the cursor.
        let mut end = TextArea::with_text("ab\ncd");
        assert_eq!(end.on_key(key(KeyCode::Delete)), FieldOutcome::Consumed);
        assert_eq!(end.text(), "ab\ncd");
        assert_eq!(end.cursor(), (1, 2));

        // Before the end of a line it removes the char at the cursor.
        let mut middle = at("ab\ncd", 1, 0);
        middle.on_key(key(KeyCode::Delete));
        assert_eq!(middle.text(), "ab\nd");
        assert_eq!(middle.cursor(), (1, 0));
    }

    #[test]
    fn up_and_down_keep_the_column_clamped() {
        let mut area = at("abcdef\nxy\nlonger", 0, 5);
        assert_eq!(area.on_key(key(KeyCode::Down)), FieldOutcome::Consumed);
        assert_eq!(area.cursor(), (1, 2), "clamped to `xy`");
        assert_eq!(area.on_key(key(KeyCode::Down)), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor(),
            (2, 2),
            "no goal column: the clamp of the last move sticks"
        );
        assert_eq!(area.on_key(key(KeyCode::Down)), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor(),
            (2, 2),
            "`Down` on the last line moves nothing"
        );

        let mut top = at("abcdef\nxy\nlonger", 0, 5);
        assert_eq!(top.on_key(key(KeyCode::Up)), FieldOutcome::Consumed);
        assert_eq!(top.cursor(), (0, 5), "`Up` on the first line moves nothing");
        assert_eq!(top.text(), "abcdef\nxy\nlonger");
    }

    #[test]
    fn left_and_right_cross_line_ends() {
        let mut area = at("abc\nde", 1, 0);
        assert_eq!(area.on_key(key(KeyCode::Left)), FieldOutcome::Consumed);
        assert_eq!(area.cursor(), (0, 3), "to the end of the previous line");
        assert_eq!(area.on_key(key(KeyCode::Right)), FieldOutcome::Consumed);
        assert_eq!(area.cursor(), (1, 0), "to column 0 of the next line");

        let mut start = at("abc\nde", 0, 0);
        assert_eq!(start.on_key(key(KeyCode::Left)), FieldOutcome::Consumed);
        assert_eq!(start.cursor(), (0, 0));

        let mut end = TextArea::with_text("abc\nde");
        assert_eq!(end.cursor(), (1, 2));
        assert_eq!(end.on_key(key(KeyCode::Right)), FieldOutcome::Consumed);
        assert_eq!(end.cursor(), (1, 2));

        // `Home` and `End` stay on the current line.
        let mut line = at("abc\nde", 1, 1);
        assert_eq!(line.on_key(key(KeyCode::Home)), FieldOutcome::Consumed);
        assert_eq!(line.cursor(), (1, 0));
        assert_eq!(line.on_key(key(KeyCode::End)), FieldOutcome::Consumed);
        assert_eq!(line.cursor(), (1, 2));
    }

    #[test]
    fn ctrl_s_submits_and_esc_cancels() {
        let mut area = TextArea::with_text("a\nb");
        assert_eq!(
            area.on_key(chord(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            FieldOutcome::Submit
        );
        assert_eq!(
            area.on_key(chord(
                KeyCode::Char('S'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            FieldOutcome::Submit,
            "`SHIFT` on top of `CONTROL` still submits"
        );
        assert_eq!(area.on_key(key(KeyCode::Esc)), FieldOutcome::Cancel);
        assert_eq!(area.text(), "a\nb");
        assert_eq!(area.cursor(), (1, 1));
    }

    #[test]
    fn other_chords_tab_and_function_keys_pass() {
        let mut area = TextArea::with_text("ab");
        for event in [
            chord(KeyCode::Char('c'), KeyModifiers::CONTROL),
            chord(KeyCode::Char('x'), KeyModifiers::ALT),
            chord(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
            chord(KeyCode::Char('s'), KeyModifiers::SUPER),
            key(KeyCode::Tab),
            key(KeyCode::BackTab),
            key(KeyCode::F(5)),
            key(KeyCode::PageUp),
            key(KeyCode::PageDown),
        ] {
            assert_eq!(area.on_key(event), FieldOutcome::Pass, "{event:?}");
        }
        assert_eq!(area.text(), "ab");
        assert_eq!(area.cursor(), (0, 2));

        // `SHIFT` alone is a capital, not a chord.
        assert_eq!(
            area.on_key(chord(KeyCode::Char('C'), KeyModifiers::SHIFT)),
            FieldOutcome::Consumed
        );
        assert_eq!(area.text(), "abC");
    }

    #[test]
    fn a_control_char_is_swallowed_not_inserted() {
        let mut area = TextArea::with_text("ab");
        assert_eq!(
            area.on_key(key(KeyCode::Char('\u{7}'))),
            FieldOutcome::Consumed
        );
        assert_eq!(area.text(), "ab");
        assert_eq!(area.cursor(), (0, 2));
    }

    #[test]
    fn text_joins_lines_with_newline_and_with_text_round_trips() {
        let area = TextArea::with_text("a\n\nb");
        assert_eq!(area.text(), "a\n\nb");
        assert_eq!(area.line_count(), 3);
        assert_eq!(area.len(), area.text().chars().count());
        assert_eq!(area.cursor(), (2, 1), "cursor at the end of the last line");

        assert_eq!(TextArea::with_text("a\r\nb\rc").text(), "a\nb\nc");
        assert!(TextArea::with_text("").is_empty());
        assert!(!TextArea::with_text("\n").is_empty());
        assert_eq!(TextArea::new().line_count(), 1);
        assert!(TextArea::new().is_empty());
        assert_eq!(TextArea::new().len(), 0);
        assert_eq!(TextArea::default().cursor(), (0, 0));
    }

    #[test]
    fn debug_never_prints_the_text() {
        let printed = format!("{:?}", TextArea::with_text("secret-ish\nnote"));
        assert!(printed.contains("line_count"), "{printed}");
        assert!(printed.contains("len"), "{printed}");
        assert!(!printed.contains("secret-ish"), "{printed}");
        assert!(!printed.contains("note"), "{printed}");
    }

    #[test]
    fn the_window_keeps_the_cursor_row_visible() {
        let text = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let area = TextArea::with_text(&text);
        assert_eq!(area.cursor().0, 9);
        let drawn: Vec<String> = area
            .lines(20, 3, false, &Theme::default())
            .iter()
            .map(|line| plain(line).trim_end().to_owned())
            .collect();
        assert_eq!(drawn, ["line7", "line8", "line9"]);

        // A window taller than the text draws every line and no more.
        assert_eq!(area.lines(20, 30, false, &Theme::default()).len(), 10);
        // A zero-sized window draws nothing.
        assert!(area.lines(0, 3, false, &Theme::default()).is_empty());
        assert!(area.lines(20, 0, false, &Theme::default()).is_empty());
    }

    #[test]
    fn a_long_other_row_is_clipped_with_a_trailing_ellipsis() {
        let theme = Theme::default();
        let area = TextArea::with_text("abcdefghijklmnop\nx");
        let drawn = area.lines(10, 2, false, &theme);
        assert_eq!(plain(&drawn[0]), "abcdefghi…");
        let last = drawn[0].spans.last().expect("a span");
        assert_eq!(last.content, "…");
        assert_eq!(last.style, theme.dim);
    }

    #[test]
    fn a_long_cursor_line_is_windowed_like_a_text_field() {
        let theme = Theme::default();
        let text = "abcdefghijklmnopqrstuvwxyz0123";
        assert_eq!(text.chars().count(), 30);
        let area = TextArea::with_text(text);
        let drawn = area.lines(10, 1, true, &theme);
        assert_eq!(drawn.len(), 1);
        let rendered = plain(&drawn[0]);
        assert!(rendered.starts_with('…'), "{rendered:?}");
        let cursor_cell = drawn[0].spans.last().expect("a span");
        assert_eq!(cursor_cell.content, " ");
        assert_eq!(cursor_cell.style, theme.selected);
        assert_eq!(drawn[0], TextField::with_text(text).line(10, true, &theme));
    }

    #[test]
    fn a_multi_byte_char_counts_as_one() {
        let mut area = TextArea::with_text("aé");
        assert_eq!(area.len(), 2);
        assert_eq!(area.cursor(), (0, 2));
        assert_eq!(area.on_key(key(KeyCode::Backspace)), FieldOutcome::Consumed);
        assert_eq!(area.text(), "a");
        assert_eq!(area.cursor(), (0, 1));
    }
}
