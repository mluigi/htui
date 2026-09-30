//! One small multi-line editor, shared by the Skills tab's Templates editor (MOD-9 D7; PRD D2)
//! and the Settings → Boxes quirks editor (MOD-7 milestone 2, D44; PRD D3): the multi-line
//! sibling of [`TextField`](crate::ui::TextField).
//!
//! Insert, delete, newline, arrows, Home/End, PgUp/PgDn and a byte-offset cursor, so a `parse`
//! error lands on its byte. **The public cursor stays a byte offset** (MOD-54 D7): `set_cursor` is
//! fed one at `skills/templates.rs:395` and `:698`, and that position has to land on the offending
//! token. What changed is the *stepping* -- `Left`/`Right`/`Backspace`/`Delete` move by **grapheme
//! cluster**, `set_cursor` floors to a cluster start, and `cursor_line_col` counts clusters -- so
//! none of them can split a combining sequence (MOD-54 D8, D13). Hard lines only: no soft wrap, undo, selection, history, mask or
//! `Zeroizing` (MOD-9 PRD risk row 5; what it holds is not secret, though its `Debug` still prints
//! lengths only). A bracketed paste arrives whole through [`TextArea::on_paste`] (MOD-22 review
//! M-1), its line breaks kept. `ctrl-s` submits, because `Enter` breaks the line and no
//! terminal mode that reports `Ctrl+Enter` is enabled (MOD-7 plan OQ-16); every other chord
//! passes to the caller. [`TextArea::with_text`] turns `\r\n` and a lone `\r` into `\n` (MOD-7
//! D59), so the text it hands back is what an editor opened on compares against.
//!
//! **Everything drawn is counted in display cells** (MOD-54 D9, D11), through
//! [`crate::ui::cells`], the same module [`TextField`](crate::ui::TextField) counts by: a wide char
//! (CJK, most emoji) takes two cells and a combining mark none, and the cursor cell is sized from
//! that measurement rather than assumed to be one. The exceptions: a `\t` draws as spaces to the
//! next tab stop (every four columns) counted from the accumulated cell column, and any control
//! char as a one-cell stand-in, because `ratatui` drops control chars when it draws and a body from
//! `$EDITOR` can hold them. A line is never drawn wider than it was given, and a grapheme that
//! would straddle an edge is dropped whole rather than split (MOD-54 D10).
//!
//! The viewport (`top`, `left`) lives in `Cell`s (MOD-9 D19): [`TextArea::lines`] scrolls it to
//! keep the cursor in view and remembers where it left it, and it does so through `&self`, because
//! a tab draws from `Tab::render(&self, ..)`.

use core::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::ui::cells::{cell_width, graphemes};
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
    /// Byte offset into `text`, always on a grapheme-cluster boundary (MOD-54 D8).
    cursor: usize,
    /// The **cell** column `Up`/`Down` aim for (MOD-54 D12); cleared by every horizontal move and
    /// edit. A cell column because "stay in the same visual column" is what a goal column means,
    /// and because that is the column [`TextArea::lines`] already measures.
    goal_col: Option<usize>,
    /// First visible line (D19): moved by [`lines`](TextArea::lines) to keep the cursor in view,
    /// remembered between frames so the view does not jump. `Cell` because `Tab::render` is
    /// `&self`.
    top: Cell<usize>,
    /// First visible **cell** column, as `top` (MOD-54 D11). Always re-derived and re-snapped to a
    /// cluster start on the cursor's line, so it can never leave half a grapheme on screen.
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
    ///
    /// **A char count, not a grapheme count**, unlike
    /// [`TextField::len`](crate::ui::TextField::len): this one is a buffer size, not a count of
    /// what a person typed (MOD-54 D7).
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

    /// Puts the cursor on `byte`: clamped to the buffer's length, then floored to a **grapheme
    /// cluster** start, so a byte offset from `parse` landing inside an emoji ZWJ sequence lands
    /// on that sequence rather than in the middle of it (MOD-54 D8).
    ///
    /// A cursor inside a cluster has no drawn cell to sit on and no defined `Left`/`Right`, which
    /// is a state every other method here would then have to cope with.
    pub fn set_cursor(&mut self, byte: usize) {
        let mut byte = byte.min(self.text.len());
        while !self.text.is_char_boundary(byte) {
            byte -= 1;
        }
        // `unicode-segmentation` 1.13 has no `is_grapheme_boundary`, so the floor is the end of the
        // last cluster that ends at or before `byte` -- which is `byte` itself when `byte` is
        // already a boundary, and the containing cluster's start when it is not.
        byte = self
            .text
            .grapheme_indices(true)
            .take_while(|(at, cluster)| at + cluster.len() <= byte)
            .map(|(at, cluster)| at + cluster.len())
            .last()
            .unwrap_or(0);
        self.cursor = byte;
        self.goal_col = None;
    }

    /// The cursor as a 0-based line and a 0-based **grapheme** column (MOD-54 D13).
    ///
    /// A text position, not a screen cell: it sits beside `L{line}`, which is a line number, and it
    /// has to agree with what `Left`/`Right` actually step over. A cell column would be neither.
    #[must_use]
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let line = before.bytes().filter(|&b| b == b'\n').count();
        let col = graphemes(&before[line_start(before, before.len())..]).count();
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
                    self.text.replace_range(previous..self.cursor, "");
                    self.cursor = previous;
                }
                self.goal_col = None;
            }
            KeyCode::Delete => {
                if self.cursor < self.text.len() {
                    let end = self.next_boundary().unwrap_or(self.text.len());
                    self.text.replace_range(self.cursor..end, "");
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

    /// At most `height` lines, each the `width`-**cell** window of its line that keeps the cursor
    /// in view, for the caller to place in its own `Rect`. A line is drawn with its tabs as spaces
    /// and its other control chars as one-cell stand-ins; every other grapheme is drawn at its own
    /// measured width.
    ///
    /// The viewport moves only as far as it must to show the cursor, and is remembered (D19), so a
    /// cursor moving inside the window does not scroll it; every line shares its horizontal offset.
    /// Two invariants hold (MOD-54 D10): **no line is ever drawn wider than `width` cells**, and a
    /// grapheme that would straddle either edge is dropped whole rather than split. Every line is
    /// `theme.base`; while `focused`, the cursor cell is `theme.selected`. No room, no lines.
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
        let row_start = line_start(&self.text, self.cursor);
        let row_end = line_end(&self.text, self.cursor);
        // The drawn column, not the char or cluster column: the viewport and the highlight both
        // count a tab as the cells it draws as, and a wide cluster as the two it takes.
        let col: usize = drawn(&self.text[row_start..self.cursor])
            .iter()
            .map(|(_, _, cells)| cells)
            .sum();
        let row = drawn(&self.text[row_start..row_end]);
        let row_starts = start_columns(&row);
        let at_width = row
            .get(row_starts.partition_point(|&c| c < col))
            .map_or(1, |(_, _, cells)| *cells);

        let top = follow(self.top.get(), line, height);
        let left = self.left_column(&row_starts, col, at_width, width);
        self.top.set(top);
        self.left.set(left);

        self.text
            .split('\n')
            .enumerate()
            .skip(top)
            .take(height)
            .map(|(index, text)| {
                let cells = drawn(text);
                let starts = start_columns(&cells);
                // A row whose clusters are wider than the offset it is given starts at the next
                // one, so nothing is ever half visible in the window's first column (D11).
                let from = starts.partition_point(|&c| c < left).min(cells.len());

                if !(focused && index == line) {
                    let window = take_cells(&cells, from, width);
                    return Line::from(Span::styled(window, theme.base));
                }

                let at_index = starts.partition_point(|&c| c < col).min(cells.len());
                let before: String = cells[from.min(at_index)..at_index]
                    .iter()
                    .map(|(_, item, _)| item.as_str())
                    .collect();
                let used = col.saturating_sub(starts[from]);
                // A cluster of no width, or one wider than the room left, still has to leave a
                // visible cursor: `ratatui` filters a zero-width string out of what it writes and
                // stops the row at one that does not fit (D10, D14).
                let (at, at_cells) = match cells.get(at_index) {
                    Some((_, item, cells)) if *cells <= width.saturating_sub(used) => {
                        (item.clone(), *cells)
                    }
                    _ => (" ".to_owned(), 1),
                };
                let after = take_cells(
                    &cells,
                    (at_index + 1).min(cells.len()),
                    width.saturating_sub(used + at_cells),
                );

                let mut spans = Vec::with_capacity(3);
                if !before.is_empty() {
                    spans.push(Span::styled(before, theme.base));
                }
                spans.push(Span::styled(at, theme.selected));
                if !after.is_empty() {
                    spans.push(Span::styled(after, theme.base));
                }
                Line::from(spans)
            })
            .collect()
    }

    /// A bracketed paste (MOD-22 review M-1), inserted at the cursor as typing it would: its line
    /// breaks (`\r\n`, a lone `\r`, `\n`) become `\n`, every other control character is dropped.
    pub fn on_paste(&mut self, text: &str) {
        // Review R2-L4: normalised once, inserted once and re-segmented once, so a large paste is
        // linear rather than one whole-buffer segmentation per character.
        let mut kept = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    chars.next_if_eq(&'\n');
                    kept.push('\n');
                }
                '\n' => kept.push('\n'),
                c if c.is_control() => {}
                c => kept.push(c),
            }
        }
        self.text.insert_str(self.cursor, &kept);
        self.cursor += kept.len();
        self.step_out_of_cluster();
        self.goal_col = None;
    }

    /// Inserts one char at the cursor and steps over the whole cluster it joined.
    ///
    /// A combining mark, a ZWJ or a variation selector merges into the cluster **before** it, which
    /// would leave the cursor stranded between the base and the mark -- a state nothing else here
    /// handles. So it advances on to the end of the cluster that now contains it (D8).
    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.step_out_of_cluster();
        self.goal_col = None;
    }

    /// After an insertion: if what was inserted merged into a cluster, the cursor is now *inside*
    /// it; step on to its end rather than stranding it between a base and its mark. `at < cursor`
    /// matters: a cluster that merely *starts* at the cursor is not one containing it, and
    /// treating it as one walks the cursor off the end of every plain insertion.
    fn step_out_of_cluster(&mut self) {
        if let Some(end) = self
            .text
            .grapheme_indices(true)
            .find(|(at, cluster)| *at < self.cursor && at + cluster.len() > self.cursor)
            .map(|(at, cluster)| at + cluster.len())
        {
            self.cursor = end;
        }
    }

    /// The grapheme boundary before the cursor, if any.
    fn previous_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(byte, _)| byte)
    }

    /// The grapheme boundary after the cursor, if any.
    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map(|cluster| self.cursor + cluster.len())
    }

    /// The first visible cell column that shows the cursor without splitting a grapheme (D11).
    ///
    /// The remembered value is snapped **down** to a cluster start on the *current* cursor line
    /// first, so a stale column from a line that has since changed can only move the window by
    /// whole cells, never leave half a cluster on screen (R-2). Then, in order: scroll left if the
    /// cursor has gone off the front; keep the remembered column if the cursor still fits after it;
    /// otherwise take the **smallest** cluster start that brings the cursor's end within `width` --
    /// the least scrolling that shows it.
    fn left_column(&self, cols: &[usize], col: usize, at_width: usize, width: usize) -> usize {
        let from = cols
            .iter()
            .copied()
            .take_while(|&c| c <= self.left.get())
            .last()
            .unwrap_or(0);
        if col < from {
            return col;
        }
        if col - from + at_width <= width {
            return from;
        }
        let floor = col + at_width - width;
        cols.iter()
            .copied()
            .find(|&c| c >= floor)
            .unwrap_or(col)
            .min(col)
    }

    /// Moves `count` lines down (or up), clamped to the buffer, aiming for the goal **cell** column.
    ///
    /// A cell column because "stay in the same visual column" is what a goal column means, and a
    /// code point index does not. It lands on the last cluster that *starts* at or before the goal,
    /// so the byte is always a grapheme boundary -- which is what the stepping in `on_key` assumes
    /// (D8, D12).
    fn move_lines(&mut self, down: bool, count: usize) {
        let (line, _) = self.cursor_line_col();
        let here = line_start(&self.text, self.cursor);
        let col: usize = drawn(&self.text[here..self.cursor])
            .iter()
            .map(|(_, _, cells)| cells)
            .sum();
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

        let mut at = 0;
        let mut cursor = end;
        let mut placed = false;
        for (byte, _, cells) in drawn(&self.text[start..end]) {
            if at > goal {
                break;
            }
            cursor = start + byte;
            at += cells;
            placed = true;
        }
        if placed && at <= goal {
            // The goal is past the end of this line, so it clamps to the line end rather than to
            // its last cluster -- which is what a goal column has always done on a short line.
            cursor = end;
        }
        self.cursor = cursor;
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

/// A control char as something visible: a Control Pictures glyph for C0 and `DEL`, `U+FFFD` for
/// C1.
///
/// Every one of these is width 1 under `unicode-width`'s non-CJK `width()`, which is what keeps
/// `ratatui`'s `debug_assert!` on unfiltered control chars quiet (`cell_width.rs:34-38`): the
/// widgets never hand a raw control char to the renderer, and this is why.
fn stand_in(c: char) -> String {
    match c {
        '\0'..='\u{1f}' => char::from_u32(0x2400 + u32::from(c))
            .map_or_else(|| "\u{fffd}".to_owned(), |glyph| glyph.to_string()),
        '\u{7f}' => "\u{2421}".to_owned(),
        _ => "\u{fffd}".to_owned(),
    }
}

/// `line` as drawn: one entry per drawn **cell** -- the byte where its cluster starts, the string
/// in it, and how many cells it takes.
///
/// A `\t` is spaces to the next [`TAB_STOP`] counted from the **accumulated cell column**, so a wide
/// grapheme before a tab moves the stop. It yields one one-cell entry per cell rather than a single
/// n-cell entry, because `a_tab_draws_as_spaces_to_the_next_stop` pins the cursor landing on a
/// one-cell span. Every other cluster is itself with its measured width, floored at one so a
/// zero-width cluster still occupies a cell and the cursor has somewhere to sit (D14).
fn drawn(line: &str) -> Vec<(usize, String, usize)> {
    let mut out: Vec<(usize, String, usize)> = Vec::new();
    let mut col = 0;
    for (at, cluster) in line.grapheme_indices(true) {
        if cluster == "\t" {
            let cells = TAB_STOP - col % TAB_STOP;
            out.extend(core::iter::repeat_n((at, " ".to_owned(), 1), cells));
            col += cells;
        } else if let Some(control) = cluster.chars().find(|c| c.is_control()) {
            out.push((at, stand_in(control), 1));
            col += 1;
        } else if cell_width(cluster) == 0 {
            // A cluster of no width still has to occupy a cell, or the cursor has nowhere to sit
            // and `ratatui` -- which filters every zero-width string out of what it writes --
            // would drop the character and shift the rest of the row left (D14).
            out.push((at, "\u{fffd}".to_owned(), 1));
            col += 1;
        } else {
            let cells = cell_width(cluster);
            out.push((at, cluster.to_owned(), cells));
            col += cells;
        }
    }
    out
}

/// The cell column at which each drawn cell starts, with a final entry for the end of the row.
fn start_columns(cells: &[(usize, String, usize)]) -> Vec<usize> {
    let mut cols = Vec::with_capacity(cells.len() + 1);
    cols.push(0);
    for (_, _, cells) in cells {
        let last = *cols.last().expect("seeded with the start of the row");
        cols.push(last + cells);
    }
    cols
}

/// Whole cells from `from` while they fit in `room`; the first that would overrun ends the window,
/// rather than being split (D10).
fn take_cells(cells: &[(usize, String, usize)], from: usize, room: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for (_, item, cells) in cells.iter().skip(from) {
        if used + cells > room {
            break;
        }
        used += cells;
        out.push_str(item);
    }
    out
}

/// The first visible line of a `span`-tall window that was at `first` and must now show `at`.
///
/// Lines are counted in lines, so this stays a plain index comparison. The horizontal counterpart
/// is [`TextArea::left_column`], which is in cells (MOD-54 D11).
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

    /// MOD-22 review M-1: a paste keeps its lines, every spelling of a break becoming `\n`, and
    /// drops the other control characters.
    #[test]
    fn a_paste_keeps_its_lines_and_drops_other_controls() {
        let mut area = TextArea::with_text("<>");
        area.set_cursor(1);
        area.on_paste("a\r\nb\rc\nd\u{7}");
        assert_eq!(area.text(), "<a\nb\nc\nd>");
        assert_eq!(area.cursor(), "<a\nb\nc\nd".len(), "after the paste");
    }

    /// Review R2-L4: a large paste lands whole, lines and all, with the cursor after it — in one
    /// insertion, not one re-segmentation of the whole buffer per character.
    #[test]
    fn a_large_paste_lands_whole() {
        let line = "a line of pasted text, é and 漢字 included\r\n";
        let paste = line.repeat(50 * 1024 / line.len() + 1);
        assert!(paste.len() >= 50 * 1024);
        let mut area = TextArea::with_text("before|after");
        area.set_cursor("before|".len());
        area.on_paste(&paste);
        let landed = paste.replace("\r\n", "\n");
        assert_eq!(area.text(), format!("before|{landed}after"));
        assert_eq!(area.cursor(), "before|".len() + landed.len());
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

    /// The inverted form of the test that used to name this bug (MOD-54 D19). It was rewritten
    /// rather than deleted: its doc comment was the only place in the repo that said out loud
    /// that the window counted chars instead of cells.
    #[test]
    fn a_wide_char_line_is_windowed_by_cells() {
        // A ten-cell window over twelve CJK clusters shows five of them, not ten, and every line
        // shares the cursor line's horizontal offset.
        let theme = Theme::default();
        let text = "\u{4e00}".repeat(12);
        let area = at_end(&format!("{text}\n{text}"));
        let drawn = area.lines(10, 2, true, &theme);
        // Every line shares the cursor line's horizontal offset, and the cursor sits at the end,
        // so the window starts at cell 16 -- the first cluster start that brings the cursor's cell
        // within ten. Four clusters are left on either side, where a char-counting window would
        // have shown nine and run to nineteen cells.
        let other = plain(&drawn[0]);
        assert_eq!(other, "\u{4e00}".repeat(4), "{other:?}");
        assert_eq!(cell_width(&other), 8);
        let cursor_row = plain(&drawn[1]);
        assert_eq!(cursor_row, format!("{} ", "\u{4e00}".repeat(4)));
        assert_eq!(cell_width(&cursor_row), 9);
        assert!(
            cell_width(&cursor_row) <= 10 && cell_width(&other) <= 10,
            "and neither row overruns the window"
        );
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

    // ---- MOD-54 ---------------------------------------------------------------------------------

    /// D8: a `parse` byte can land inside an emoji ZWJ sequence, and a cursor there has no drawn
    /// cell and no defined `Left`/`Right`, so it floors to the cluster's start.
    #[test]
    fn set_cursor_floors_inside_a_grapheme() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let mut area = TextArea::with_text(&format!("{family}x"));
        for byte in 1..family.len() {
            area.set_cursor(byte);
            assert_eq!(area.cursor(), 0, "byte {byte} is inside the family emoji");
        }
        area.set_cursor(family.len());
        assert_eq!(
            area.cursor(),
            family.len(),
            "and the `x` past it is its own byte"
        );
    }

    /// D8: `Left`/`Right` step by cluster, so the cursor cannot land between a base and its mark.
    #[test]
    fn left_and_right_step_by_grapheme() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let mut area = TextArea::with_text(&format!("a\u{301}{family}c"));
        area.set_cursor(usize::MAX);
        assert_eq!(
            area.cursor_line_col(),
            (0, 3),
            "three clusters, not six code points"
        );
        press(&mut area, KeyCode::Left);
        assert_eq!(
            area.cursor_line_col(),
            (0, 2),
            "past the family emoji whole"
        );
        press(&mut area, KeyCode::Left);
        assert_eq!(
            area.cursor_line_col(),
            (0, 1),
            "and to the end of the mark's cluster"
        );
    }

    /// D8: `Backspace` removes a whole cluster rather than orphaning the mark.
    #[test]
    fn backspace_removes_a_whole_grapheme() {
        let mut area = at_end("a\u{301}b");
        press(&mut area, KeyCode::Backspace);
        assert_eq!(area.text(), "a\u{301}", "not a base and a stray mark");
    }

    /// D8's `insert`: a typed mark merges into the cluster before it, and the cursor advances on
    /// to the end of the cluster that now holds it rather than stranding itself between the two.
    #[test]
    fn a_typed_combining_mark_joins_the_cluster_before_it() {
        let mut area = TextArea::new();
        for c in ['a', '\u{301}', 'b'] {
            assert_eq!(press(&mut area, KeyCode::Char(c)), FieldOutcome::Consumed);
        }
        assert_eq!(area.text(), "a\u{301}b");
        assert_eq!(
            area.cursor_line_col(),
            (0, 2),
            "past `b` at the end, and in particular not stranded at 1 between the base and its mark"
        );
    }

    /// D5: `ratatui` writes the style onto a wide grapheme's first cell and then *resets* the
    /// continuation, so the cursor lands on the first.
    #[test]
    fn a_wide_cursor_cell_highlights_its_first_cell() {
        let theme = Theme::default();
        let mut area = TextArea::with_text("\u{4e00}x");
        area.set_cursor(0);
        let drawn = area.lines(6, 1, true, &theme);
        let cursor = drawn[0]
            .spans
            .iter()
            .find(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .expect("a cursor span");
        assert_eq!(cursor.content, "\u{4e00}");
    }

    /// D10: a grapheme that would straddle the window's edge is dropped whole, never split.
    #[test]
    fn a_wide_grapheme_that_does_not_fit_is_dropped_not_split() {
        let theme = Theme::default();
        let area = at_end(&format!("{}\n1", "\u{4e00}".repeat(12)));
        let drawn = area.lines(10, 2, true, &theme);
        for row in &drawn {
            assert!(
                cell_width(&plain(row)) <= 10,
                "no row overruns: {:?}",
                plain(row)
            );
        }
        assert_eq!(
            plain(&drawn[1]),
            "1 ",
            "the ASCII row is whole, with the cursor cell"
        );
    }

    /// D11: `left` is a cell column, re-derived on every draw, and never starts mid-cluster.
    #[test]
    fn a_viewport_never_starts_mid_grapheme() {
        let theme = Theme::default();
        let area = at_end(&"\u{4e00}".repeat(12));
        for width in 1..12u16 {
            let row = plain(&area.lines(width, 1, true, &theme)[0]);
            assert!(cell_width(&row) <= usize::from(width), "{row:?} at {width}");
            assert!(
                row.chars().all(|c| c == '\u{4e00}' || c == ' '),
                "only whole clusters and the cursor cell, at width {width}: {row:?}"
            );
        }
    }

    /// D12: the goal column is a **cell** column, so `Up`/`Down` hold the same visual column
    /// across lines of different widths -- cell 6 is the fourth CJK cluster, not the seventh.
    #[test]
    fn up_and_down_keep_the_cell_goal_column() {
        let mut area = at(
            "abcdefgh\n\u{4e00}\u{4e00}\u{4e00}\u{4e00}\u{4e00}\u{4e00}",
            0,
            6,
        );
        assert_eq!(area.cursor_line_col(), (0, 6));
        press(&mut area, KeyCode::Down);
        assert_eq!(
            area.cursor_line_col(),
            (1, 3),
            "the same cell, not the same index"
        );
    }

    /// D14: a tab stop is counted from the accumulated **cell** column, so a wide grapheme before
    /// a tab moves the stop. Six cells of `漢` would leave four; two cells leaves two.
    #[test]
    fn a_wide_char_before_a_tab_moves_the_stop() {
        let area = at_end("\u{6f22}\tx");
        let row = plain(&area.lines(10, 1, true, &Theme::default())[0]);
        assert_eq!(
            row, "\u{6f22}  x ",
            "two cells of text, then a two-space tab, then the cursor cell"
        );
        assert_eq!(
            cell_width(&row),
            6,
            "two for the kanji, two for the tab, one for `x`, one for the cursor"
        );
        // The hint counts clusters, not cells: the tab is one column, so `x` is the third. The
        // two-space tab above is a *cell* fact and does not move this number.
        assert_eq!(
            area.cursor_line_col(),
            (0, 3),
            "at the end, past all three clusters"
        );
        let mut on_x = TextArea::with_text("\u{6f22}\tx");
        on_x.set_cursor("\u{6f22}\t".len());
        assert_eq!(
            on_x.cursor_line_col(),
            (0, 2),
            "and `x` itself is the third cluster"
        );
    }

    /// D15: a family emoji is one cluster measured as a string, so a ten-cell window holds five.
    #[test]
    fn an_emoji_zwj_sequence_is_one_cluster_two_cells() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let area = at_end(&family.repeat(5));
        let row = plain(&area.lines(10, 1, true, &Theme::default())[0]);
        // Four clusters and the cursor's cell: five clusters would be ten cells with nothing left
        // for the cursor to sit in, and the cursor stays on screen (D10).
        assert_eq!(
            row,
            format!("{} ", family.repeat(4)),
            "four clusters and the cursor cell"
        );
        assert_eq!(cell_width(&row), 9);
    }

    /// D15: a variation selector is measured with its base.
    #[test]
    fn a_variation_selector_is_measured_with_its_base() {
        assert_eq!(cell_width("\u{2328}\u{fe0f}"), 2);
        assert_eq!(cell_width("\u{2328}"), 1);
        let theme = Theme::default();
        assert_eq!(
            plain(&at_end("\u{2328}").lines(6, 1, true, &theme)[0]).trim_end(),
            "\u{2328}"
        );
        assert_eq!(
            plain(&at_end("\u{2328}\u{fe0f}").lines(6, 1, true, &theme)[0]).trim_end(),
            "\u{2328}\u{fe0f}"
        );
    }

    /// D14: a zero-width cluster under the cursor draws a replacement glyph, because `ratatui`
    /// filters a zero-width string out of what it writes and the cursor would vanish.
    #[test]
    fn a_zero_width_cluster_at_the_cursor_draws_a_replacement_glyph() {
        let mut area = TextArea::with_text("\u{301}a");
        area.set_cursor(0);
        let drawn = area.lines(4, 1, true, &Theme::default());
        let cursor = drawn[0]
            .spans
            .iter()
            .find(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .expect("a cursor span");
        assert_eq!(cursor.content, "\u{fffd}");
    }

    /// The invariant the whole change exists to hold: no row is ever drawn wider than it was
    /// given, over a mix of everything the widget can hold.
    #[test]
    fn a_wide_grapheme_at_the_row_edge_does_not_overrun() {
        let theme = Theme::default();
        for filler in [
            "abcdefghij",
            "\u{6f22}\u{6f22}\u{6f22}\u{6f22}\u{6f22}",
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
            "a\tb\tc",
            "a\u{1}b\u{7f}c",
            "e\u{301}".repeat(4).as_str(),
        ] {
            for width in 1..12u16 {
                let area = at_end(&format!("{filler}\n{filler}"));
                for row in area.lines(width, 2, true, &theme) {
                    assert!(
                        cell_width(&plain(&row)) <= usize::from(width),
                        "{filler:?} at width {width} drew {:?}",
                        plain(&row)
                    );
                }
            }
        }
    }

    /// D13, renamed: the column is a grapheme column, so it agrees with what `Left` steps over.
    #[test]
    fn cursor_line_col_counts_graphemes_not_bytes() {
        let mut area = TextArea::with_text("\u{e9}\u{e9}\u{e9}\n\u{e0}bc");
        area.set_cursor(4);
        assert_eq!(
            area.cursor_line_col(),
            (0, 2),
            "unchanged for single-cluster text"
        );
        area.set_cursor(9);
        assert_eq!(area.cursor_line_col(), (1, 1));

        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let mut emoji = TextArea::with_text(&format!("{family}x"));
        emoji.set_cursor(family.len());
        assert_eq!(
            emoji.cursor_line_col(),
            (0, 1),
            "a family emoji is one column"
        );
    }

    /// D17c: today's exact expectations, inlined, so a change to the windowing arithmetic is a
    /// diff someone can read rather than a rewritten test that looks the same.
    #[test]
    fn ascii_rendering_is_unchanged() {
        let text: Vec<String> = (0..10).map(|n| format!("line{n}")).collect();
        let text = text.join("\n");
        let mut area = TextArea::with_text(&text);
        area.on_key(key(KeyCode::PageDown), 3);
        assert_eq!(window(&area, 5, 3), ["line1", "line2", "line3"]);
        for _ in 0..3 {
            press(&mut area, KeyCode::Up);
        }
        assert_eq!(window(&area, 5, 3), ["line0", "line1", "line2"]);
        press(&mut area, KeyCode::End);
        assert_eq!(window(&area, 3, 1), ["e0 "]);

        assert_eq!(
            plain(
                &at_end("abcdefghijklmnopqrstuvwxyz0123").lines(10, 1, true, &Theme::default())[0]
            ),
            "vwxyz0123 "
        );
        assert_eq!(
            plain(&at_end("abcdefghijklmnop\nx").lines(10, 2, false, &Theme::default())[0]),
            "abcdefghij"
        );
        assert_eq!(
            window(&TextArea::with_text("a\tb\n\tc\nabcd\te"), 10, 3),
            ["a   b", "    c", "abcd    e"]
        );
        assert_eq!(
            window(&TextArea::with_text("a\u{1}b\u{7f}\u{9b}c"), 10, 1),
            ["a\u{2401}b\u{2421}\u{fffd}c"]
        );
        assert_eq!(window(&at_end("a\nb\nc\nd"), 4, 2), ["c", "d "]);

        let mut wide = TextArea::with_text("\u{e9}\u{e9}\nabc");
        press(&mut wide, KeyCode::End);
        press(&mut wide, KeyCode::Down);
        assert_eq!(wide.cursor_line_col(), (1, 2));
        assert_eq!(wide.cursor(), 7);
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
