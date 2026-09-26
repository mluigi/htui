//! One small multi-line text area (MOD-7 milestone 2, PRD D3): the quirks editor's widget, the
//! multi-line sibling of [`TextField`](crate::ui::TextField). Hard lines only (no soft wrap),
//! counted in `char`s. `ctrl-s` submits, because `Enter` breaks the line and no terminal mode
//! that reports `Ctrl+Enter` is enabled (plan OQ-16). No history, selection, undo, mask,
//! bracketed paste or `Zeroizing`: what it holds is not secret.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::{FieldOutcome, Theme};

/// Every modifier that makes a key a chord; `SHIFT` is how a terminal reports a capital.
const CHORD: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::META)
    .union(KeyModifiers::HYPER);

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
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
        }
    }
}

/// Never the text: sections derive `Debug`. Prints `line_count` and `len` only.
impl core::fmt::Debug for TextArea {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextArea")
            .field("line_count", &self.line_count())
            .field("len", &self.len())
            .finish()
    }
}

impl TextArea {
    /// An empty area: one empty line, cursor at `(0, 0)`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `text` split into lines on `\n`, after `\r\n` and a lone `\r` become `\n` (D59); cursor at
    /// the end of the last line.
    #[must_use]
    pub fn with_text(text: &str) -> Self {
        let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<String> = normalised.split('\n').map(str::to_owned).collect();
        let row = lines.len() - 1;
        let col = lines[row].chars().count();
        Self { lines, row, col }
    }

    /// Feeds one key.
    ///
    /// Any chord (`CONTROL`, `ALT`, `SUPER`, `META`, `HYPER`) passes, except `ctrl-s`, which
    /// submits; so `ctrl-c` reaches the caller. `Char` inserts unless it is a control char (which
    /// is swallowed); `Enter` splits the line at the cursor; `Backspace` and `Delete` join lines
    /// at a line's edge; the arrows, `Home` and `End` move; `Esc` cancels; everything else passes.
    pub fn on_key(&mut self, key: KeyEvent) -> FieldOutcome {
        let chord = key.modifiers.intersection(CHORD);
        if !chord.is_empty() {
            return if chord == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('s' | 'S'))
            {
                FieldOutcome::Submit
            } else {
                FieldOutcome::Pass
            };
        }
        match key.code {
            KeyCode::Char(c) => {
                if !c.is_control() {
                    let byte = self.byte_of(self.col);
                    self.lines[self.row].insert(byte, c);
                    self.col += 1;
                }
            }
            KeyCode::Enter => {
                let byte = self.byte_of(self.col);
                let tail = self.lines[self.row].split_off(byte);
                self.lines.insert(self.row + 1, tail);
                self.row += 1;
                self.col = 0;
            }
            KeyCode::Backspace => {
                if self.col > 0 {
                    let byte = self.byte_of(self.col - 1);
                    self.lines[self.row].remove(byte);
                    self.col -= 1;
                } else if self.row > 0 {
                    let line = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = self.row_len(self.row);
                    self.lines[self.row].push_str(&line);
                }
            }
            KeyCode::Delete => {
                if self.col < self.row_len(self.row) {
                    let byte = self.byte_of(self.col);
                    self.lines[self.row].remove(byte);
                } else if self.row + 1 < self.lines.len() {
                    let next = self.lines.remove(self.row + 1);
                    self.lines[self.row].push_str(&next);
                }
            }
            KeyCode::Left => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.row_len(self.row);
                }
            }
            KeyCode::Right => {
                if self.col < self.row_len(self.row) {
                    self.col += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
            }
            KeyCode::Up => {
                if self.row > 0 {
                    self.row -= 1;
                    self.col = self.col.min(self.row_len(self.row));
                }
            }
            KeyCode::Down => {
                if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = self.col.min(self.row_len(self.row));
                }
            }
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.row_len(self.row),
            KeyCode::Esc => return FieldOutcome::Cancel,
            _ => return FieldOutcome::Pass,
        }
        FieldOutcome::Consumed
    }

    /// The lines joined by `\n`.
    #[must_use]
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Whether the area is one empty line.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// How many chars, the `\n` between lines included (so `len() == text().chars().count()`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.chars().count())
            .sum::<usize>()
            + (self.lines.len() - 1)
    }

    /// How many lines (at least 1).
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The cursor as `(row, col)`.
    #[must_use]
    pub const fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// At most `height` lines, each at most `width` cells, for the caller to place (and pad) in
    /// its own `Rect`.
    ///
    /// Nothing when either is 0. The window ends at the cursor row (D61). The cursor row is drawn
    /// as [`TextField::line`](crate::ui::TextField::line) draws an unmasked field: a window of
    /// chars ending at the cursor, a leading dim `…` when its start is clipped, the cursor cell in
    /// `theme.selected` while `focused`. Every other row is drawn from column 0 and, when it has
    /// more than `width` chars, cut to its first `width - 1` and a trailing dim `…`. Computed on
    /// every call, so a resize needs no event.
    #[must_use]
    pub fn lines(
        &self,
        width: u16,
        height: u16,
        focused: bool,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let width = usize::from(width);
        let height = usize::from(height);
        let top = self.row.saturating_sub(height - 1);
        let bottom = (top + height).min(self.lines.len());
        (top..bottom)
            .map(|row| {
                if row == self.row {
                    self.cursor_line(width, focused, theme)
                } else {
                    clipped_line(&self.lines[row], width, theme)
                }
            })
            .collect()
    }

    /// The cursor row, `TextField::line`'s algorithm (`text_field.rs`) without the mask (D44:
    /// copied, not extracted).
    fn cursor_line(&self, budget: usize, focused: bool, theme: &Theme) -> Line<'static> {
        let glyphs: Vec<char> = self.lines[self.row].chars().collect();
        let cursor = self.col;

        // `budget` cells hold `['…'] + before + cursor cell + after`, so the cursor is always on
        // screen; below two cells there is room for the cursor and nothing else.
        let (start, ellipsis) = if budget < 2 {
            (cursor, false)
        } else if cursor < budget {
            (0, false)
        } else {
            (cursor + 2 - budget, true)
        };
        let before: String = glyphs[start.min(glyphs.len())..cursor.min(glyphs.len())]
            .iter()
            .collect();
        let at = glyphs.get(cursor).copied().unwrap_or(' ');
        let room = budget
            .saturating_sub(usize::from(ellipsis))
            .saturating_sub(before.chars().count())
            .saturating_sub(1);
        let after: String = glyphs
            .iter()
            .skip(cursor.saturating_add(1))
            .take(room)
            .collect();

        let cursor_style: Style = if focused { theme.selected } else { theme.base };
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(4);
        if ellipsis {
            spans.push(Span::styled("…", theme.dim));
        }
        if !before.is_empty() {
            spans.push(Span::styled(before, theme.base));
        }
        spans.push(Span::styled(at.to_string(), cursor_style));
        if !after.is_empty() {
            spans.push(Span::styled(after, theme.base));
        }
        Line::from(spans)
    }

    /// How many chars `lines[row]` holds.
    fn row_len(&self, row: usize) -> usize {
        self.lines[row].chars().count()
    }

    /// Byte offset of char `index` in the cursor's line, or the line's length past the end.
    fn byte_of(&self, index: usize) -> usize {
        let line = &self.lines[self.row];
        line.char_indices()
            .nth(index)
            .map_or(line.len(), |(byte, _)| byte)
    }
}

/// A row other than the cursor's: from column 0, cut to `width - 1` chars and a dim `…` when it
/// is longer than `width`.
fn clipped_line(line: &str, width: usize, theme: &Theme) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(2);
    if line.chars().count() > width {
        let head: String = line.chars().take(width - 1).collect();
        if !head.is_empty() {
            spans.push(Span::styled(head, theme.base));
        }
        spans.push(Span::styled("…", theme.dim));
    } else if !line.is_empty() {
        spans.push(Span::styled(line.to_owned(), theme.base));
    }
    Line::from(spans)
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
