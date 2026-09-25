//! A small multi-line editor (MOD-9 D7; PRD D2): insert, delete, newline, arrows, Home/End,
//! PgUp/PgDn and a byte-offset cursor, so a `parse` error lands on its byte. No wrap, undo,
//! selection or paste (PRD risk row 5). Width in `char`s, as `TextField` (no `unicode-width`),
//! except that a `\t` draws as spaces to the next tab stop (every four columns) and any other
//! control char as a one-column stand-in: `ratatui` drops control chars when it draws, and a body
//! from `$EDITOR` can hold them.
//!
//! The viewport (`top`, `left`) lives in `Cell`s (D19): [`TextArea::lines`] scrolls it to keep the
//! cursor in view and remembers where it left it, and it does so through `&self`, because a tab
//! draws from `Tab::render(&self, ..)`.

use core::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};

use crate::ui::Theme;

/// Columns between tab stops, for a `\t` that came back from `$EDITOR` (typing one is out of scope,
/// PRD risk row 5).
const TAB_STOP: usize = 4;

/// What one key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaOutcome {
    /// Edited or moved: nothing for the caller to do.
    Consumed,
    /// `Esc`: the caller decides what cancelling means.
    Cancel,
    /// Not an editing key (a `CONTROL`/`ALT`/`SUPER`/`META`/`HYPER` chord, `Tab`, `BackTab`,
    /// `F(n)`, …): the caller keeps its own bindings.
    Pass,
}

/// A multi-line buffer with a byte-offset cursor. Never `Debug`s its text.
#[derive(Clone, Default)]
pub struct TextArea {
    /// What was typed. Never printed by [`Debug`](core::fmt::Debug).
    text: String,
    /// Byte offset into `text`, always on a char boundary.
    cursor: usize,
    /// The char column `Up`/`Down` aim for; cleared by every horizontal move and edit.
    goal_col: Option<usize>,
    /// First visible line (D19): moved by [`lines`](TextArea::lines) to keep the cursor in view,
    /// remembered between frames so the view does not jump. `Cell` because `Tab::render` is
    /// `&self`.
    top: Cell<usize>,
    /// First visible char column, as `top`.
    left: Cell<usize>,
}

/// Lengths and the cursor, never the text: a template body is user text, and every request that
/// might carry a view derives `Debug` (`TextField`'s rule).
impl core::fmt::Debug for TextArea {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextArea")
            .field("len", &self.text.len())
            .field("lines", &self.line_count())
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl TextArea {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer holding `text`, cursor at byte 0 and the viewport at the top left.
    #[must_use]
    pub fn with_text(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            ..Self::default()
        }
    }

    /// The whole buffer.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Moves the buffer out.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
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
    /// Every modifier but `SHIFT` passes (`SHIFT` is how a terminal reports a capital). `Char`
    /// inserts unless it is a control char, which is swallowed; `Enter` inserts `\n`;
    /// `Backspace`/`Delete` join lines at their ends; `Left`/`Right` cross them; `Up`/`Down` and
    /// `PageUp`/`PageDown` keep the goal column; `Home`/`End` stay on the line; `Esc` cancels and
    /// everything else passes. Edits happen in place: no key builds a new `String`.
    pub fn on_key(&mut self, key: KeyEvent, page: u16) -> AreaOutcome {
        if key.modifiers.intersects(
            KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER
                | KeyModifiers::META
                | KeyModifiers::HYPER,
        ) {
            return AreaOutcome::Pass;
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
            KeyCode::Esc => return AreaOutcome::Cancel,
            _ => return AreaOutcome::Pass,
        }
        AreaOutcome::Consumed
    }

    /// At most `height` lines, each the `width`-column window of its line that keeps the cursor in
    /// view, for the caller to place in its own `Rect`. A line is drawn with its tabs as spaces and
    /// its other control chars as one-column stand-ins.
    ///
    /// The viewport moves only as far as it must to show the cursor, and is remembered (D19), so a
    /// cursor moving inside the window does not scroll it. Every line is `theme.base`; while
    /// `focused`, the cursor cell (a space past the end of its line) is `theme.selected`. No room,
    /// no lines.
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

    /// How many lines the buffer holds: one more than its `\n`s.
    fn line_count(&self) -> usize {
        self.text.bytes().filter(|&b| b == b'\n').count() + 1
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

    fn press(area: &mut TextArea, code: KeyCode) -> AreaOutcome {
        area.on_key(key(code), 10)
    }

    fn typed(area: &mut TextArea, text: &str) {
        for c in text.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            assert_eq!(press(area, code), AreaOutcome::Consumed);
        }
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
            wide.on_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT), 10),
            AreaOutcome::Consumed
        );
        assert_eq!(wide.text(), "aézN");

        // A control char is swallowed, not inserted and not passed.
        assert_eq!(
            press(&mut wide, KeyCode::Char('\u{7}')),
            AreaOutcome::Consumed
        );
        assert_eq!(wide.into_text(), "aézN");
    }

    #[test]
    fn enter_splits_the_line_and_backspace_joins_it_again() {
        let mut area = TextArea::with_text("headtail");
        area.set_cursor(4);
        assert_eq!(press(&mut area, KeyCode::Enter), AreaOutcome::Consumed);
        assert_eq!(area.text(), "head\ntail");
        assert_eq!(area.cursor_line_col(), (1, 0));

        assert_eq!(press(&mut area, KeyCode::Backspace), AreaOutcome::Consumed);
        assert_eq!(area.text(), "headtail");
        assert_eq!(area.cursor(), 4);
        assert_eq!(area.cursor_line_col(), (0, 4));

        // At the very start there is nothing to remove.
        area.set_cursor(0);
        assert_eq!(press(&mut area, KeyCode::Backspace), AreaOutcome::Consumed);
        assert_eq!(area.text(), "headtail");
        assert_eq!(area.cursor(), 0);
    }

    #[test]
    fn delete_at_line_end_joins_the_next_line() {
        let mut area = TextArea::with_text("ab\ncd");
        press(&mut area, KeyCode::End);
        assert_eq!(area.cursor(), 2);
        assert_eq!(press(&mut area, KeyCode::Delete), AreaOutcome::Consumed);
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
    fn left_and_right_cross_line_boundaries() {
        let mut area = TextArea::with_text("ab\ncd");
        area.set_cursor(3);
        assert_eq!(area.cursor_line_col(), (1, 0));

        assert_eq!(press(&mut area, KeyCode::Left), AreaOutcome::Consumed);
        assert_eq!(
            area.cursor_line_col(),
            (0, 2),
            "to the end of the line above"
        );
        assert_eq!(press(&mut area, KeyCode::Right), AreaOutcome::Consumed);
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
    fn home_and_end_stay_on_the_line() {
        let mut area = TextArea::with_text("ab\ncde\nf");
        area.set_cursor(4);
        assert_eq!(press(&mut area, KeyCode::Home), AreaOutcome::Consumed);
        assert_eq!(area.cursor(), 3);
        assert_eq!(area.cursor_line_col(), (1, 0));
        assert_eq!(press(&mut area, KeyCode::End), AreaOutcome::Consumed);
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
            AreaOutcome::Consumed
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
                area.on_key(KeyEvent::new(KeyCode::Char('s'), modifier), 10),
                AreaOutcome::Pass,
                "`{modifier:?}` is a chord, not a character"
            );
            assert_eq!(
                area.on_key(KeyEvent::new(KeyCode::Enter, modifier), 10),
                AreaOutcome::Pass
            );
        }
        for code in [
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::F(2),
            KeyCode::Insert,
        ] {
            assert_eq!(press(&mut area, code), AreaOutcome::Pass, "{code:?}");
        }
        assert_eq!(area.text(), "ab", "none of those typed anything");
        assert_eq!(area.cursor(), 0);
    }

    #[test]
    fn esc_cancels() {
        let mut area = TextArea::with_text("ab");
        assert_eq!(press(&mut area, KeyCode::Esc), AreaOutcome::Cancel);
        assert_eq!(area.text(), "ab");
    }

    #[test]
    fn debug_prints_lengths_not_text() {
        let mut area = TextArea::with_text("secret\nbody");
        area.set_cursor(3);
        let rendered = format!("{area:?}");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(!rendered.contains("body"), "{rendered}");
        assert!(rendered.contains("len: 11"), "{rendered}");
        assert!(rendered.contains("lines: 2"), "{rendered}");
        assert!(rendered.contains("cursor: 3"), "{rendered}");
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
        let mut area = TextArea::with_text("a\nb\nc\nd");
        area.set_cursor(area.text().len());
        let drawn: Vec<String> = render(&area).iter().map(plain).collect();
        assert_eq!(drawn, ["c", "d "]);
    }
}
