//! One small multi-line editor, shared by the Skills tab's Templates editor (MOD-9 D7; PRD D2)
//! and the Settings → Boxes quirks editor (MOD-7 milestone 2, D44; PRD D3): the multi-line
//! sibling of [`TextField`](crate::ui::TextField).
//!
//! Insert, delete, newline, arrows, Home/End, PgUp/PgDn and a byte-offset cursor, so a `parse`
//! error lands on its byte. Hard lines only: no soft wrap, undo, selection, history, mask,
//! bracketed paste or `Zeroizing` (MOD-9 PRD risk row 5; what it holds is not secret, though its
//! `Debug` still prints lengths only). `ctrl-s` submits, because `Enter` breaks the line and no
//! terminal mode that reports `Ctrl+Enter` is enabled (MOD-7 plan OQ-16); every other chord
//! passes to the caller. [`TextArea::with_text`] turns `\r\n` and a lone `\r` into `\n` (MOD-7
//! D59), so the text it hands back is what an editor opened on compares against.
//!
//! Width is counted in `char`s, as `TextField` counts it (no `unicode-width`): a wide char (CJK,
//! most emoji) takes two cells but counts as one, so a line of them can overrun its column, and
//! the cursor steps by code point, not by grapheme. The exceptions: a `\t` draws as spaces to the
//! next tab stop (every four columns) and any other control char as a one-column stand-in, because
//! `ratatui` drops control chars when it draws, and a body from `$EDITOR` can hold them.
//!
//! The viewport (`top`, `left`) lives in `Cell`s (MOD-9 D19): [`TextArea::lines`] scrolls it to
//! keep the cursor in view and remembers where it left it, and it does so through `&self`, because
//! a tab draws from `Tab::render(&self, ..)`.

use core::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};

use crate::ui::{FieldOutcome, Theme};

/// Columns between tab stops, for a `\t` that came back from `$EDITOR` (typing one is out of scope,
/// MOD-9 PRD risk row 5).
const TAB_STOP: usize = 4;

/// Every modifier that makes a key a chord; `SHIFT` is how a terminal reports a capital.
const CHORD: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::META)
    .union(KeyModifiers::HYPER);

/// A multi-line buffer with a byte-offset cursor. Never `Debug`s its text.
#[derive(Clone, Default)]
pub struct TextArea {
    /// What was typed; lines are split on `\n`. Never printed by [`Debug`](core::fmt::Debug).
    text: String,
    /// Byte offset into `text`, always on a char boundary.
    cursor: usize,
    /// The char column `Up`/`Down` aim for; cleared by every horizontal move and edit.
    goal_col: Option<usize>,
    /// First visible line (D19): moved by [`lines`](TextArea::lines) to keep the cursor in view,
    /// remembered between frames so the view does not jump. `Cell` because `Tab::render` is
    /// `&self`.
    top: Cell<usize>,
    /// First visible drawn column, as `top`.
    left: Cell<usize>,
}

/// Lengths and the cursor, never the text: a template body and a box's quirks are user text, and
/// every request or section that might carry a view derives `Debug` (`TextField`'s rule).
impl core::fmt::Debug for TextArea {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextArea")
            .field("len", &self.len())
            .field("line_count", &self.line_count())
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl TextArea {
    /// An empty buffer: one empty line, cursor at byte 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer holding `text` with `\r\n` and a lone `\r` turned into `\n` (MOD-7 D59), cursor at
    /// byte 0 and the viewport at the top left. [`set_cursor`](TextArea::set_cursor)`(usize::MAX)`
    /// puts the cursor at the end instead.
    #[must_use]
    pub fn with_text(text: &str) -> Self {
        let text = if text.contains('\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.to_owned()
        };
        Self {
            text,
            ..Self::default()
        }
    }

    /// The whole buffer, lines joined by `\n`.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Moves the buffer out.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }

    /// Whether the buffer is one empty line.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// How many chars, the `\n` between lines included (so `len() == text().chars().count()`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// How many lines the buffer holds (at least 1): one more than its `\n`s.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.text.bytes().filter(|&b| b == b'\n').count() + 1
    }

    /// The cursor, as a byte offset into [`text`](TextArea::text).
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Puts the cursor on `byte`: clamped to the buffer's length, then floored to a char boundary,
    /// so a byte offset from `parse` inside a multi-byte char lands on that char.
    pub fn set_cursor(&mut self, byte: usize) {
        let mut byte = byte.min(self.text.len());
        while !self.text.is_char_boundary(byte) {
            byte -= 1;
        }
        self.cursor = byte;
        self.goal_col = None;
    }

    /// The cursor as a 0-based line and a 0-based **char** column.
    #[must_use]
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let line = before.bytes().filter(|&b| b == b'\n').count();
        let col = before[line_start(before, before.len())..].chars().count();
        (line, col)
    }

    /// Feeds one key; `page` is how many lines `PageUp`/`PageDown` move (at least one).
    ///
    /// Any chord (`CONTROL`, `ALT`, `SUPER`, `META`, `HYPER`) passes, except `ctrl-s`, which
    /// submits; so `ctrl-c` and `ctrl-e` reach the caller. `SHIFT` alone is a capital, not a
    /// chord. `Char` inserts unless it is a control char, which is swallowed; `Enter` inserts
    /// `\n`; `Backspace`/`Delete` join lines at their ends; `Left`/`Right` cross them; `Up`/`Down`
    /// and `PageUp`/`PageDown` keep the goal column; `Home`/`End` stay on the line; `Esc` cancels
    /// and everything else passes. Edits happen in place: no key builds a new `String`.
    pub fn on_key(&mut self, key: KeyEvent, page: u16) -> FieldOutcome {
        let chord = key.modifiers.intersection(CHORD);
        if !chord.is_empty() {
            return if chord == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('s' | 'S'))
            {
                FieldOutcome::Submit
            } else {
                FieldOutcome::Pass
            };
        }
        let page = usize::from(page.max(1));
        match key.code {
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.insert(c);
                }
            }
            KeyCode::Enter => self.insert('\n'),
            KeyCode::Backspace => {
                if let Some(previous) = self.previous_boundary() {
                    self.text.remove(previous);
                    self.cursor = previous;
                }
                self.goal_col = None;
            }
            KeyCode::Delete => {
                if self.cursor < self.text.len() {
                    self.text.remove(self.cursor);
                }
                self.goal_col = None;
            }
            KeyCode::Left => {
                self.cursor = self.previous_boundary().unwrap_or(self.cursor);
                self.goal_col = None;
            }
            KeyCode::Right => {
                self.cursor = self.next_boundary().unwrap_or(self.cursor);
                self.goal_col = None;
            }
            KeyCode::Home => {
                self.cursor = line_start(&self.text, self.cursor);
                self.goal_col = None;
            }
            KeyCode::End => {
                self.cursor = line_end(&self.text, self.cursor);
                self.goal_col = None;
            }
            KeyCode::Up => self.move_lines(false, 1),
            KeyCode::Down => self.move_lines(true, 1),
            KeyCode::PageUp => self.move_lines(false, page),
            KeyCode::PageDown => self.move_lines(true, page),
            KeyCode::Esc => return FieldOutcome::Cancel,
            _ => return FieldOutcome::Pass,
        }
        FieldOutcome::Consumed
    }

    /// At most `height` lines, each the `width`-column window of its line that keeps the cursor in
    /// view, for the caller to place in its own `Rect`. A line is drawn with its tabs as spaces and
    /// its other control chars as one-column stand-ins; one char is otherwise one cell, as
    /// `TextField` takes it, so a CJK or emoji line can overrun `width` cells.
    ///
    /// The viewport moves only as far as it must to show the cursor, and is remembered (D19), so a
    /// cursor moving inside the window does not scroll it; every line shares its horizontal
    /// offset, and a line longer than the window is cut at its edge. Every line is `theme.base`;
    /// while `focused`, the cursor cell (a space past the end of its line) is `theme.selected`.
    /// No room, no lines.
    #[must_use]
    pub fn lines(
        &self,
        width: u16,
        height: u16,
        focused: bool,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let (width, height) = (usize::from(width), usize::from(height));
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let (line, _) = self.cursor_line_col();
        // The drawn column, not the char column: the viewport and the highlight both count a tab
        // as the cells it draws as.
        let col = drawn(&self.text[line_start(&self.text, self.cursor)..self.cursor]).count();
        let top = follow(self.top.get(), line, height);
        let left = follow(self.left.get(), col, width);
        self.top.set(top);
        self.left.set(left);

        self.text
            .split('\n')
            .enumerate()
            .skip(top)
            .take(height)
            .map(|(index, text)| {
                let mut chars = drawn(text).skip(left);
                if !(focused && index == line) {
                    let window: String = chars.take(width).collect();
                    return Line::from(Span::styled(window, theme.base));
                }
                let before: String = chars.by_ref().take(col - left).collect();
                let at = chars.next().unwrap_or(' ');
                let after: String = chars.take(left + width - col - 1).collect();
                let mut spans = Vec::with_capacity(3);
                if !before.is_empty() {
                    spans.push(Span::styled(before, theme.base));
                }
                spans.push(Span::styled(at.to_string(), theme.selected));
                if !after.is_empty() {
                    spans.push(Span::styled(after, theme.base));
                }
                Line::from(spans)
            })
            .collect()
    }

    /// Inserts one char at the cursor and steps over it.
    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.goal_col = None;
    }

    /// The char boundary before the cursor, if any.
    fn previous_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(byte, _)| byte)
    }

    /// The char boundary after the cursor, if any.
    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..]
            .chars()
            .next()
            .map(|c| self.cursor + c.len_utf8())
    }

    /// Moves `count` lines down (or up), clamped to the buffer, aiming for the goal column.
    fn move_lines(&mut self, down: bool, count: usize) {
        let (line, col) = self.cursor_line_col();
        let goal = self.goal_col.unwrap_or(col);
        let target = if down {
            line.saturating_add(count).min(self.line_count() - 1)
        } else {
            line.saturating_sub(count)
        };
        let start = if target == 0 {
            0
        } else {
            self.text
                .match_indices('\n')
                .nth(target - 1)
                .map_or(self.text.len(), |(byte, _)| byte + 1)
        };
        let end = line_end(&self.text, start);
        self.cursor = self.text[start..end]
            .char_indices()
            .nth(goal)
            .map_or(end, |(byte, _)| start + byte);
        self.goal_col = Some(goal);
    }
}

/// The byte where the line holding byte `at` starts.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |newline| newline + 1)
}

/// The byte where the line holding byte `at` ends (its `\n`, or the buffer's end).
fn line_end(text: &str, at: usize) -> usize {
    text[at..]
        .find('\n')
        .map_or(text.len(), |newline| at + newline)
}

/// `line` as drawn, one char per column: a `\t` is spaces to the next [`TAB_STOP`], any other
/// control char its stand-in (a Control Pictures glyph for C0 and `DEL`, `U+FFFD` for C1). A
/// prefix of a line draws as a prefix of its cells, so the cursor's column is the count of the
/// text before it.
fn drawn(line: &str) -> impl Iterator<Item = char> + '_ {
    let mut col = 0;
    line.chars().flat_map(move |c| {
        let (cell, count) = match c {
            '\t' => (' ', TAB_STOP - col % TAB_STOP),
            '\0'..='\u{1f}' => (
                char::from_u32(0x2400 + u32::from(c)).unwrap_or('\u{fffd}'),
                1,
            ),
            '\u{7f}' => ('\u{2421}', 1),
            c if c.is_control() => ('\u{fffd}', 1),
            c => (c, 1),
        };
        col += count;
        core::iter::repeat_n(cell, count)
    })
}

/// The first visible index of a `span`-wide window that was at `first` and must now show `at`.
const fn follow(first: usize, at: usize, span: usize) -> usize {
    if at < first {
        at
    } else if at >= first + span {
        at + 1 - span
    } else {
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn chord(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn press(area: &mut TextArea, code: KeyCode) -> FieldOutcome {
        area.on_key(key(code), 10)
    }

    fn typed(area: &mut TextArea, text: &str) {
        for c in text.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            assert_eq!(press(area, code), FieldOutcome::Consumed);
        }
    }

    /// An area holding `text` with the cursor at the end, as the Boxes quirks editor opens it.
    fn at_end(text: &str) -> TextArea {
        let mut area = TextArea::with_text(text);
        area.set_cursor(usize::MAX);
        area
    }

    /// An area holding `text` with the cursor put on line `row`, char column `col`.
    fn at(text: &str, row: usize, col: usize) -> TextArea {
        let mut area = TextArea::with_text(text);
        let start: usize = text.split('\n').take(row).map(|line| line.len() + 1).sum();
        let line = text.split('\n').nth(row).expect("the row exists");
        let byte = line.char_indices().nth(col).map_or(line.len(), |(b, _)| b);
        area.set_cursor(start + byte);
        assert_eq!(area.cursor_line_col(), (row, col));
        area
    }

    /// One drawn line's text, spans joined.
    fn plain(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn window(area: &TextArea, width: u16, height: u16) -> Vec<String> {
        area.lines(width, height, true, &Theme::default())
            .iter()
            .map(plain)
            .collect()
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut area = TextArea::new();
        typed(&mut area, "ac");
        assert_eq!(area.text(), "ac");
        assert_eq!(area.cursor(), 2);

        press(&mut area, KeyCode::Left);
        typed(&mut area, "b");
        assert_eq!(area.text(), "abc", "`b` lands before `c`");
        assert_eq!(area.cursor(), 2);

        // The cursor is a byte offset: stepping over `é` moves it by two.
        let mut wide = TextArea::with_text("é");
        assert_eq!(wide.cursor(), 0, "`with_text` starts at the top");
        typed(&mut wide, "a");
        press(&mut wide, KeyCode::Right);
        typed(&mut wide, "z");
        assert_eq!(wide.text(), "aéz");
        assert_eq!(wide.cursor(), 4);

        // `SHIFT` is a capital, not a chord.
        assert_eq!(
            wide.on_key(chord(KeyCode::Char('N'), KeyModifiers::SHIFT), 10),
            FieldOutcome::Consumed
        );
        assert_eq!(wide.text(), "aézN");

        // A control char is swallowed, not inserted and not passed.
        assert_eq!(
            press(&mut wide, KeyCode::Char('\u{7}')),
            FieldOutcome::Consumed
        );
        assert_eq!(wide.into_text(), "aézN");
    }

    #[test]
    fn enter_splits_the_line_and_backspace_joins_it_again() {
        let mut area = TextArea::with_text("headtail");
        area.set_cursor(4);
        assert_eq!(press(&mut area, KeyCode::Enter), FieldOutcome::Consumed);
        assert_eq!(area.text(), "head\ntail");
        assert_eq!(area.cursor_line_col(), (1, 0));

        assert_eq!(press(&mut area, KeyCode::Backspace), FieldOutcome::Consumed);
        assert_eq!(area.text(), "headtail");
        assert_eq!(area.cursor(), 4);
        assert_eq!(area.cursor_line_col(), (0, 4));

        // At the very start there is nothing to remove.
        area.set_cursor(0);
        assert_eq!(press(&mut area, KeyCode::Backspace), FieldOutcome::Consumed);
        assert_eq!(area.text(), "headtail");
        assert_eq!(area.cursor(), 0);
    }

    #[test]
    fn enter_splits_the_line_at_the_cursor() {
        let mut area = at_end("abcd");
        assert_eq!(press(&mut area, KeyCode::Left), FieldOutcome::Consumed);
        assert_eq!(press(&mut area, KeyCode::Left), FieldOutcome::Consumed);
        assert_eq!(press(&mut area, KeyCode::Enter), FieldOutcome::Consumed);
        assert_eq!(area.text(), "ab\ncd");
        assert_eq!(area.cursor_line_col(), (1, 0));
        assert_eq!(area.line_count(), 2);

        // `SHIFT` is how a terminal reports a capital; it does not turn `Enter` into anything else.
        let mut shifted = at_end("xy");
        assert_eq!(
            shifted.on_key(chord(KeyCode::Enter, KeyModifiers::SHIFT), 10),
            FieldOutcome::Consumed
        );
        assert_eq!(shifted.text(), "xy\n");
        assert_eq!(shifted.cursor_line_col(), (1, 0));
    }

    #[test]
    fn backspace_at_column_zero_joins_the_previous_line() {
        let mut area = at("ab\ncd", 1, 0);
        assert_eq!(press(&mut area, KeyCode::Backspace), FieldOutcome::Consumed);
        assert_eq!(area.text(), "abcd");
        assert_eq!(area.cursor_line_col(), (0, 2));

        // At `(0, 0)` there is nothing before the cursor.
        let mut start = at("ab\ncd", 0, 0);
        assert_eq!(
            press(&mut start, KeyCode::Backspace),
            FieldOutcome::Consumed
        );
        assert_eq!(start.text(), "ab\ncd");
        assert_eq!(start.cursor_line_col(), (0, 0));
    }

    #[test]
    fn delete_at_line_end_joins_the_next_line() {
        let mut area = TextArea::with_text("ab\ncd");
        press(&mut area, KeyCode::End);
        assert_eq!(area.cursor(), 2);
        assert_eq!(press(&mut area, KeyCode::Delete), FieldOutcome::Consumed);
        assert_eq!(area.text(), "abcd");
        assert_eq!(area.cursor(), 2, "the cursor stays put");

        // A multi-byte char goes whole.
        let mut wide = TextArea::with_text("éx");
        press(&mut wide, KeyCode::Delete);
        assert_eq!(wide.text(), "x");

        // At the very end there is nothing to remove.
        let mut end = TextArea::with_text("ab");
        end.set_cursor(2);
        press(&mut end, KeyCode::Delete);
        assert_eq!(end.text(), "ab");
    }

    #[test]
    fn delete_at_the_end_joins_the_next_line() {
        let mut area = at("ab\ncd", 0, 2);
        assert_eq!(press(&mut area, KeyCode::Delete), FieldOutcome::Consumed);
        assert_eq!(area.text(), "abcd");
        assert_eq!(area.cursor_line_col(), (0, 2));

        // At the end of the last line there is nothing after the cursor.
        let mut end = at_end("ab\ncd");
        assert_eq!(press(&mut end, KeyCode::Delete), FieldOutcome::Consumed);
        assert_eq!(end.text(), "ab\ncd");
        assert_eq!(end.cursor_line_col(), (1, 2));

        // Before the end of a line it removes the char at the cursor.
        let mut middle = at("ab\ncd", 1, 0);
        press(&mut middle, KeyCode::Delete);
        assert_eq!(middle.text(), "ab\nd");
        assert_eq!(middle.cursor_line_col(), (1, 0));
    }

    #[test]
    fn left_and_right_cross_line_boundaries() {
        let mut area = TextArea::with_text("ab\ncd");
        area.set_cursor(3);
        assert_eq!(area.cursor_line_col(), (1, 0));

        assert_eq!(press(&mut area, KeyCode::Left), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor_line_col(),
            (0, 2),
            "to the end of the line above"
        );
        assert_eq!(press(&mut area, KeyCode::Right), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor_line_col(),
            (1, 0),
            "and back to the start of the next"
        );

        // Both ends saturate.
        area.set_cursor(0);
        press(&mut area, KeyCode::Left);
        assert_eq!(area.cursor(), 0);
        area.set_cursor(5);
        press(&mut area, KeyCode::Right);
        assert_eq!(area.cursor(), 5);
    }

    #[test]
    fn left_and_right_cross_line_ends() {
        let mut area = at("abc\nde", 1, 0);
        assert_eq!(press(&mut area, KeyCode::Left), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor_line_col(),
            (0, 3),
            "to the end of the previous line"
        );
        assert_eq!(press(&mut area, KeyCode::Right), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor_line_col(),
            (1, 0),
            "to column 0 of the next line"
        );

        let mut start = at("abc\nde", 0, 0);
        assert_eq!(press(&mut start, KeyCode::Left), FieldOutcome::Consumed);
        assert_eq!(start.cursor_line_col(), (0, 0));

        let mut end = at_end("abc\nde");
        assert_eq!(end.cursor_line_col(), (1, 2));
        assert_eq!(press(&mut end, KeyCode::Right), FieldOutcome::Consumed);
        assert_eq!(end.cursor_line_col(), (1, 2));

        // `Home` and `End` stay on the current line.
        let mut line = at("abc\nde", 1, 1);
        assert_eq!(press(&mut line, KeyCode::Home), FieldOutcome::Consumed);
        assert_eq!(line.cursor_line_col(), (1, 0));
        assert_eq!(press(&mut line, KeyCode::End), FieldOutcome::Consumed);
        assert_eq!(line.cursor_line_col(), (1, 2));
    }

    #[test]
    fn up_and_down_keep_the_goal_column_through_a_short_line() {
        let mut area = TextArea::with_text("abcdef\nxy\nghijkl");
        area.set_cursor(5);
        assert_eq!(area.cursor_line_col(), (0, 5));

        press(&mut area, KeyCode::Down);
        assert_eq!(
            area.cursor_line_col(),
            (1, 2),
            "clamped to the short line's end"
        );
        press(&mut area, KeyCode::Down);
        assert_eq!(
            area.cursor_line_col(),
            (2, 5),
            "and the goal column comes back"
        );
        press(&mut area, KeyCode::Up);
        press(&mut area, KeyCode::Up);
        assert_eq!(area.cursor_line_col(), (0, 5));

        // A horizontal move forgets the goal.
        press(&mut area, KeyCode::Down);
        press(&mut area, KeyCode::Left);
        assert_eq!(area.cursor_line_col(), (1, 1));
        press(&mut area, KeyCode::Down);
        assert_eq!(area.cursor_line_col(), (2, 1));

        // Past the first and last lines, nothing moves.
        press(&mut area, KeyCode::Down);
        assert_eq!(area.cursor_line_col(), (2, 1));
        area.set_cursor(1);
        press(&mut area, KeyCode::Up);
        assert_eq!(area.cursor_line_col(), (0, 1));

        // Columns are chars: `é` is one column.
        let mut wide = TextArea::with_text("éé\nabc");
        press(&mut wide, KeyCode::End);
        press(&mut wide, KeyCode::Down);
        assert_eq!(wide.cursor_line_col(), (1, 2));
        assert_eq!(wide.cursor(), 7);
    }

    #[test]
    fn up_and_down_keep_the_column_clamped() {
        let mut area = at("abcdef\nxy\nlonger", 0, 5);
        assert_eq!(press(&mut area, KeyCode::Down), FieldOutcome::Consumed);
        assert_eq!(area.cursor_line_col(), (1, 2), "clamped to `xy`");
        assert_eq!(press(&mut area, KeyCode::Down), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor_line_col(),
            (2, 5),
            "the goal column comes back past the short line"
        );
        assert_eq!(press(&mut area, KeyCode::Down), FieldOutcome::Consumed);
        assert_eq!(
            area.cursor_line_col(),
            (2, 5),
            "`Down` on the last line moves nothing"
        );

        let mut top = at("abcdef\nxy\nlonger", 0, 5);
        assert_eq!(press(&mut top, KeyCode::Up), FieldOutcome::Consumed);
        assert_eq!(
            top.cursor_line_col(),
            (0, 5),
            "`Up` on the first line moves nothing"
        );
        assert_eq!(top.text(), "abcdef\nxy\nlonger");
    }

    #[test]
    fn home_and_end_stay_on_the_line() {
        let mut area = TextArea::with_text("ab\ncde\nf");
        area.set_cursor(4);
        assert_eq!(press(&mut area, KeyCode::Home), FieldOutcome::Consumed);
        assert_eq!(area.cursor(), 3);
        assert_eq!(area.cursor_line_col(), (1, 0));
        assert_eq!(press(&mut area, KeyCode::End), FieldOutcome::Consumed);
        assert_eq!(area.cursor(), 6);
        assert_eq!(area.cursor_line_col(), (1, 3));

        // Pressed again, each stays where it is.
        press(&mut area, KeyCode::End);
        assert_eq!(area.cursor(), 6);
        press(&mut area, KeyCode::Home);
        press(&mut area, KeyCode::Home);
        assert_eq!(area.cursor(), 3);
    }

    #[test]
    fn page_down_moves_by_the_page_and_clamps() {
        let text: Vec<String> = (0..10).map(|n| format!("line{n}")).collect();
        let mut area = TextArea::with_text(&text.join("\n"));
        area.set_cursor(2);

        assert_eq!(
            area.on_key(key(KeyCode::PageDown), 3),
            FieldOutcome::Consumed
        );
        assert_eq!(
            area.cursor_line_col(),
            (3, 2),
            "three lines down, same column"
        );
        area.on_key(key(KeyCode::PageDown), 3);
        area.on_key(key(KeyCode::PageDown), 3);
        assert_eq!(area.cursor_line_col(), (9, 2));
        area.on_key(key(KeyCode::PageDown), 3);
        assert_eq!(area.cursor_line_col(), (9, 2), "clamped to the last line");

        area.on_key(key(KeyCode::PageUp), 4);
        assert_eq!(area.cursor_line_col(), (5, 2));
        area.on_key(key(KeyCode::PageUp), 40);
        assert_eq!(area.cursor_line_col(), (0, 2), "clamped to the first line");

        // A zero page still moves one line.
        area.on_key(key(KeyCode::PageDown), 0);
        assert_eq!(area.cursor_line_col(), (1, 2));
    }

    #[test]
    fn set_cursor_floors_inside_a_multibyte_char() {
        let mut area = TextArea::with_text("é{{x");
        area.set_cursor(1);
        assert_eq!(area.cursor(), 0, "byte 1 is inside `é`");
        area.set_cursor(2);
        assert_eq!(area.cursor(), 2, "byte 2 is the `{{`");
    }

    #[test]
    fn set_cursor_past_the_end_clamps_to_len() {
        let mut area = TextArea::with_text("ab\ncé");
        area.set_cursor(99);
        assert_eq!(area.cursor(), area.text().len());
        assert_eq!(area.cursor_line_col(), (1, 2));
    }

    #[test]
    fn cursor_line_col_counts_chars_not_bytes() {
        let mut area = TextArea::with_text("ééé\nàbc");
        area.set_cursor(4);
        assert_eq!(area.cursor_line_col(), (0, 2));
        area.set_cursor(9);
        assert_eq!(
            area.cursor_line_col(),
            (1, 1),
            "`à` is two bytes and one column"
        );
        assert_eq!(TextArea::new().cursor_line_col(), (0, 0));
    }

    #[test]
    fn control_chords_tab_and_function_keys_pass() {
        let mut area = TextArea::with_text("ab");
        for modifier in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::META,
            KeyModifiers::HYPER,
        ] {
            assert_eq!(
                area.on_key(chord(KeyCode::Char('e'), modifier), 10),
                FieldOutcome::Pass,
                "`{modifier:?}` is a chord, not a character"
            );
            assert_eq!(
                area.on_key(chord(KeyCode::Enter, modifier), 10),
                FieldOutcome::Pass
            );
        }
        for code in [
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::F(2),
            KeyCode::Insert,
        ] {
            assert_eq!(press(&mut area, code), FieldOutcome::Pass, "{code:?}");
        }
        assert_eq!(area.text(), "ab", "none of those typed anything");
        assert_eq!(area.cursor(), 0);
    }

    #[test]
    fn ctrl_s_submits_and_esc_cancels() {
        let mut area = at_end("a\nb");
        assert_eq!(
            area.on_key(chord(KeyCode::Char('s'), KeyModifiers::CONTROL), 10),
            FieldOutcome::Submit
        );
        assert_eq!(
            area.on_key(
                chord(
                    KeyCode::Char('S'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                10
            ),
            FieldOutcome::Submit,
            "`SHIFT` on top of `CONTROL` still submits"
        );
        assert_eq!(press(&mut area, KeyCode::Esc), FieldOutcome::Cancel);
        assert_eq!(area.text(), "a\nb");
        assert_eq!(area.cursor_line_col(), (1, 1));
    }

    #[test]
    fn esc_cancels() {
        let mut area = TextArea::with_text("ab");
        assert_eq!(press(&mut area, KeyCode::Esc), FieldOutcome::Cancel);
        assert_eq!(area.text(), "ab");
    }

    #[test]
    fn other_chords_tab_and_function_keys_pass() {
        let mut area = at_end("ab");
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
        ] {
            assert_eq!(area.on_key(event, 10), FieldOutcome::Pass, "{event:?}");
        }
        assert_eq!(area.text(), "ab");
        assert_eq!(area.cursor_line_col(), (0, 2));

        // `SHIFT` alone is a capital, not a chord.
        assert_eq!(
            area.on_key(chord(KeyCode::Char('C'), KeyModifiers::SHIFT), 10),
            FieldOutcome::Consumed
        );
        assert_eq!(area.text(), "abC");
    }

    #[test]
    fn a_control_char_is_swallowed_not_inserted() {
        let mut area = at_end("ab");
        assert_eq!(
            press(&mut area, KeyCode::Char('\u{7}')),
            FieldOutcome::Consumed
        );
        assert_eq!(area.text(), "ab");
        assert_eq!(area.cursor_line_col(), (0, 2));
    }

    #[test]
    fn text_joins_lines_with_newline_and_with_text_round_trips() {
        let area = TextArea::with_text("a\n\nb");
        assert_eq!(area.text(), "a\n\nb");
        assert_eq!(area.line_count(), 3);
        assert_eq!(area.len(), area.text().chars().count());
        assert_eq!(area.cursor_line_col(), (0, 0), "cursor at the start");
        assert_eq!(at_end("a\n\nb").cursor_line_col(), (2, 1));

        assert_eq!(TextArea::with_text("a\r\nb\rc").text(), "a\nb\nc");
        assert!(TextArea::with_text("").is_empty());
        assert!(!TextArea::with_text("\n").is_empty());
        assert_eq!(TextArea::new().line_count(), 1);
        assert!(TextArea::new().is_empty());
        assert_eq!(TextArea::new().len(), 0);
        assert_eq!(TextArea::default().cursor(), 0);
        assert_eq!(TextArea::default().cursor_line_col(), (0, 0));
    }

    #[test]
    fn debug_prints_lengths_not_text() {
        let mut area = TextArea::with_text("secret\nbody");
        area.set_cursor(3);
        let rendered = format!("{area:?}");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(!rendered.contains("body"), "{rendered}");
        assert!(rendered.contains("len: 11"), "{rendered}");
        assert!(rendered.contains("line_count: 2"), "{rendered}");
        assert!(rendered.contains("cursor: 3"), "{rendered}");
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
    fn the_viewport_scrolls_to_keep_the_cursor_visible() {
        let text: Vec<String> = (0..10).map(|n| format!("line{n}")).collect();
        let mut area = TextArea::with_text(&text.join("\n"));
        assert_eq!(window(&area, 5, 3), ["line0", "line1", "line2"]);

        area.on_key(key(KeyCode::PageDown), 3);
        assert_eq!(area.cursor_line_col(), (3, 0));
        let first = window(&area, 5, 3);
        assert_eq!(first, ["line1", "line2", "line3"], "scrolled just enough");
        assert_eq!(
            window(&area, 5, 3),
            first,
            "no key between, same window (the `Cell` memory)"
        );

        // Moving up inside the window does not jump it back to the top.
        press(&mut area, KeyCode::Up);
        assert_eq!(window(&area, 5, 3), ["line1", "line2", "line3"]);
        press(&mut area, KeyCode::Up);
        press(&mut area, KeyCode::Up);
        assert_eq!(window(&area, 5, 3), ["line0", "line1", "line2"]);

        // Horizontally too: `End` on a five-char line in a three-wide window shows the tail and
        // the cursor's trailing cell.
        press(&mut area, KeyCode::End);
        assert_eq!(area.cursor_line_col(), (0, 5));
        assert_eq!(window(&area, 3, 1), ["e0 "]);

        // The cursor cell carries `selected` only while focused.
        let theme = Theme::default();
        let focused = area.lines(3, 1, true, &theme);
        let cell = focused[0].spans.last().expect("a cursor span");
        assert_eq!(cell.content, " ");
        assert!(cell.style.add_modifier.contains(Modifier::REVERSED));
        let unfocused = area.lines(3, 1, false, &theme);
        assert!(
            unfocused[0]
                .spans
                .iter()
                .all(|span| !span.style.add_modifier.contains(Modifier::REVERSED))
        );

        // No room, no lines.
        assert!(area.lines(0, 3, true, &theme).is_empty());
        assert!(area.lines(3, 0, true, &theme).is_empty());
    }

    #[test]
    fn the_window_keeps_the_cursor_row_visible() {
        let text = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let area = at_end(&text);
        assert_eq!(area.cursor_line_col().0, 9);
        let drawn: Vec<String> = area
            .lines(20, 3, false, &Theme::default())
            .iter()
            .map(|line| plain(line).trim_end().to_owned())
            .collect();
        assert_eq!(drawn, ["line7", "line8", "line9"]);

        // A window taller than the text draws every line and no more (a fresh area: this one
        // remembers its viewport at `line7`, D19).
        assert_eq!(
            at_end(&text).lines(20, 30, false, &Theme::default()).len(),
            10
        );
        // A zero-sized window draws nothing.
        assert!(area.lines(0, 3, false, &Theme::default()).is_empty());
        assert!(area.lines(20, 0, false, &Theme::default()).is_empty());
    }

    #[test]
    fn a_long_other_row_is_cut_at_the_window_edge() {
        let theme = Theme::default();
        let area = at_end("abcdefghijklmnop\nx");
        let drawn = area.lines(10, 2, false, &theme);
        assert_eq!(plain(&drawn[0]), "abcdefghij", "cut, with no `…`");
        assert!(drawn[0].spans.iter().all(|span| span.style == theme.base));
    }

    #[test]
    fn a_long_cursor_line_scrolls_to_show_the_cursor() {
        let theme = Theme::default();
        let text = "abcdefghijklmnopqrstuvwxyz0123";
        assert_eq!(text.chars().count(), 30);
        let area = at_end(text);
        let drawn = area.lines(10, 1, true, &theme);
        assert_eq!(drawn.len(), 1);
        let rendered = plain(&drawn[0]);
        assert_eq!(rendered, "vwxyz0123 ", "the tail and the cursor cell");
        let cursor_cell = drawn[0].spans.last().expect("a span");
        assert_eq!(cursor_cell.content, " ");
        assert_eq!(cursor_cell.style, theme.selected);
    }

    #[test]
    fn a_wide_char_line_is_windowed_by_chars_not_cells() {
        // The known limitation shared with `TextField`: the window counts chars, so a 10-wide
        // window over twelve CJK chars draws up to 10 chars (about 19 cells), not 10 cells. Every
        // line shares the cursor line's horizontal offset.
        let theme = Theme::default();
        let text = "\u{4e00}".repeat(12);
        let area = at_end(&format!("{text}\n{text}"));
        let drawn = area.lines(10, 2, true, &theme);
        let other = plain(&drawn[0]);
        assert_eq!(other, "\u{4e00}".repeat(9), "{other:?}");
        let cursor_row = plain(&drawn[1]);
        assert_eq!(cursor_row.chars().count(), 10, "{cursor_row:?}");
        assert_eq!(cursor_row, format!("{} ", "\u{4e00}".repeat(9)));
    }

    #[test]
    fn a_multi_byte_char_counts_as_one() {
        let mut area = at_end("aé");
        assert_eq!(area.len(), 2);
        assert_eq!(area.cursor_line_col(), (0, 2));
        assert_eq!(press(&mut area, KeyCode::Backspace), FieldOutcome::Consumed);
        assert_eq!(area.text(), "a");
        assert_eq!(area.cursor_line_col(), (0, 1));
    }

    /// A `\t` from `$EDITOR` draws as spaces to the next tab stop; `ratatui` would drop it and
    /// shift the rest of the line left of where the cursor is counted.
    #[test]
    fn a_tab_draws_as_spaces_to_the_next_stop() {
        let mut area = TextArea::with_text("a\tb\n\tc\nabcd\te");
        assert_eq!(TAB_STOP, 4, "the expectations below are for a stop of four");
        assert_eq!(window(&area, 10, 3), ["a   b", "    c", "abcd    e"]);

        // The cursor is drawn where the text is: on `b`, after the tab's three cells.
        area.set_cursor(2);
        let theme = Theme::default();
        let drawn = area.lines(10, 1, true, &theme);
        let cursor: Vec<&str> = drawn[0]
            .spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(cursor, ["b"]);
        assert_eq!(plain(&drawn[0]), "a   b");

        // On the tab itself, the cursor is the tab's first cell.
        area.set_cursor(1);
        let drawn = area.lines(10, 1, true, &theme);
        assert_eq!(drawn[0].spans[1].content, " ");
        assert!(
            drawn[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::REVERSED)
        );
        assert_eq!(plain(&drawn[0]), "a   b");

        // The viewport follows the drawn column: `b` is column 4, so a three-wide window starts
        // at column 2 and shows `b` last.
        area.set_cursor(2);
        assert_eq!(window(&area, 3, 1), ["  b"]);

        // The hint's column is still in chars (D20): `b` is the third char.
        assert_eq!(area.cursor_line_col(), (0, 2));
    }

    /// Any other control char draws as a visible stand-in, one column wide.
    #[test]
    fn other_control_chars_draw_visibly() {
        let mut area = TextArea::with_text("a\u{1}b\u{7f}\u{9b}c");
        area.set_cursor(2);
        assert_eq!(window(&area, 10, 1), ["a\u{2401}b\u{2421}\u{fffd}c"]);
        let drawn = area.lines(10, 1, true, &Theme::default());
        assert_eq!(drawn[0].spans[1].content, "b", "the cursor is on `b`");
    }

    /// D19: `lines` scrolls through `&self`, because `Tab::render` is `&self` (F-C).
    #[test]
    fn lines_takes_shared_self() {
        fn render(area: &TextArea) -> Vec<Line<'static>> {
            area.lines(4, 2, true, &Theme::default())
        }
        let area = at_end("a\nb\nc\nd");
        let drawn: Vec<String> = render(&area).iter().map(plain).collect();
        assert_eq!(drawn, ["c", "d "]);
    }
}
