//! One single-line text field with a grapheme cursor and an optional mask (MOD-15 milestone 3, D1;
//! PRD D1; MOD-54 D3). The widget MOD-22, MOD-23 and the connection section's DSN entry consume.
//!
//! Width is counted in **display cells** and the cursor steps by **grapheme cluster**, both through
//! [`crate::ui::cells`] (MOD-54 D1, D2), so a CJK or emoji line occupies the columns it is drawn
//! in rather than one column per code point, and `Left`/`Backspace` cannot split a combining
//! sequence. Every cell count in this file comes from that one module, so it cannot drift from
//! `ratatui`, which is what actually draws the result.
//!
//! Multi-line, history, a reveal toggle and validation are deliberately not built.
//!
//! **Bracketed paste** (MOD-22 review M-1) arrives as one [`on_paste`](TextField::on_paste), never
//! as keys: the shell enables the terminal's bracketed-paste mode, so a paste is one
//! `Event::Paste` that reaches a field only while its view captures input, and is dropped —
//! wiped — anywhere else. Before that mode a paste was replayed as keystrokes, so text pasted
//! before a field was open ran as commands: its digits switched tabs, its `/` opened an unmasked
//! filter and the rest of it (an OAuth `?code=…`) was echoed there in clear. A paste into this
//! single-line widget drops its line breaks and every other control character, and a masked field
//! takes one only whole and only within the capacity it was opened with.
//!
//! **`Zeroizing` arrived with milestone 6** (D3), and it covers exactly this much: the buffer is
//! wiped when the field drops, when [`clear`](TextField::clear) is called, and — because
//! [`masked`](TextField::masked) reserves 256 bytes (or what
//! [`masked_with_capacity`](TextField::masked_with_capacity) was given) — without a trail of
//! half-typed reallocations behind it. What it does not reach is priced rather than claimed: the terminal's own input
//! buffering, the kernel's, and any allocation a `with_text` field outgrew before this. The one
//! secret this widget ever holds leaves it through [`take`](TextField::take), which **moves** the
//! allocation to a caller that wraps it in `Zeroizing` on the same line.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation as _;
use zeroize::{Zeroize as _, Zeroizing};

use crate::ui::Theme;
use crate::ui::cells::{cell_width, graphemes};

/// What a view says when a paste does not fit a masked field's reservation and was refused whole
/// (MOD-22 review M-1).
pub const PASTE_DOES_NOT_FIT: &str =
    "the pasted text is longer than this field holds; nothing was pasted";

/// What one key did to the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldOutcome {
    /// The field edited or moved; nothing for the caller to do.
    Consumed,
    /// `Enter`: the caller reads [`TextField::text`] or [`TextField::take`].
    Submit,
    /// `Esc`: the caller decides what cancelling means.
    Cancel,
    /// Not a field key (`Tab`, `BackTab`, `Up`, `Down`, `F(n)`, any chord but `SHIFT`): the
    /// caller keeps its own bindings.
    Pass,
}

/// A single-line buffer with a cursor, stepped in graphemes.
#[derive(Clone, Default)]
pub struct TextField {
    /// What was typed. Never printed by [`Debug`](core::fmt::Debug).
    ///
    /// Wiped on drop (D3). For a slug that costs nothing; for a DSN it is the point — and it is
    /// the reason the type is the same for both, since a field that zeroized only when masked
    /// would be one `masked: false` away from not zeroizing at all.
    text: Zeroizing<String>,
    /// Grapheme index, `0..=len()`.
    cursor: usize,
    /// Whether [`line`](TextField::line) draws `•` per grapheme and [`text`](TextField::text)
    /// refuses.
    masked: bool,
}

/// Never the text: [`StoreRequest`](crate::store_worker::StoreRequest),
/// [`RequestEnvelope`](crate::store_worker::RequestEnvelope) and every section derive `Debug`, so
/// a field that derived it would put a masked buffer into a log the moment one of them is printed.
impl core::fmt::Debug for TextField {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextField")
            .field("masked", &self.masked)
            .field("len", &self.len())
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl TextField {
    /// An empty, unmasked field.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty masked field: it draws `•` per grapheme and [`text`](TextField::text) is `None`.
    ///
    /// Reserves 256 bytes, so a DSN of ordinary length never reallocates — a `String` that grows
    /// past its capacity copies its bytes into a new allocation and frees the old one **unwiped**,
    /// which would leave a secret behind at every intermediate size on the way to being typed.
    ///
    /// The mask is still a **rendering** guarantee and the reservation a storage one; neither is
    /// the whole story. The other half of the rule lives one crate over: **a DSN must not reach
    /// the worker as a plain `String`** on [`StoreRequest`](crate::store_worker::StoreRequest),
    /// which derives `Debug` and is printed by every test that reports an unexpected reply. It
    /// travels as [`htui_store::Dsn`], whose `Debug` is `Dsn(<redacted>)`.
    #[must_use]
    pub fn masked() -> Self {
        Self::masked_with_capacity(256)
    }

    /// An empty masked field reserving `bytes` up front (MOD-22 D275).
    ///
    /// For a secret longer than a DSN: a pasted redirect is hundreds of characters, and every
    /// time a `String` outgrows its capacity it frees the old allocation **unwiped**. Opened at
    /// the longest text its caller will accept, a field never reallocates on the way to a value
    /// that caller could use.
    #[must_use]
    pub fn masked_with_capacity(bytes: usize) -> Self {
        Self {
            masked: true,
            text: Zeroizing::new(String::with_capacity(bytes)),
            ..Self::default()
        }
    }

    /// An unmasked field holding `text`, cursor at the end — how an editor prefills a row.
    #[must_use]
    pub fn with_text(text: &str) -> Self {
        Self {
            cursor: graphemes(text).count(),
            text: Zeroizing::new(text.to_owned()),
            masked: false,
        }
    }

    /// Feeds one key.
    ///
    /// `Char` inserts unless it carries a chord modifier or is itself a control char (which is
    /// swallowed, not inserted); `Backspace`/`Delete`/`Left`/`Right`/`Home`/`End` edit and move;
    /// `Enter` submits, `Esc` cancels, everything else passes so a form keeps `Tab` and a section
    /// keeps its own letters.
    ///
    /// Every modifier but `SHIFT` passes: `SHIFT` is how a terminal reports a capital, and the
    /// other five — including the `SUPER`/`META`/`HYPER` a kitty-protocol terminal reports and a
    /// plain one never does — mean the key was a chord aimed at something other than this buffer.
    pub fn on_key(&mut self, key: KeyEvent) -> FieldOutcome {
        if key.modifiers.intersects(
            KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER
                | KeyModifiers::META
                | KeyModifiers::HYPER,
        ) {
            return FieldOutcome::Pass;
        }
        match key.code {
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.insert(c);
                }
                FieldOutcome::Consumed
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    // The whole cluster, not its first char: `String::remove` would leave a
                    // stray combining mark or the rest of a ZWJ sequence behind. `replace_range`
                    // shrinks in place, so a masked buffer is not reallocated.
                    let (start, end) = (self.byte_of(self.cursor - 1), self.byte_of(self.cursor));
                    self.text.replace_range(start..end, "");
                    self.cursor -= 1;
                }
                FieldOutcome::Consumed
            }
            KeyCode::Delete => {
                if self.cursor < self.len() {
                    let (start, end) = (self.byte_of(self.cursor), self.byte_of(self.cursor + 1));
                    self.text.replace_range(start..end, "");
                }
                FieldOutcome::Consumed
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                FieldOutcome::Consumed
            }
            KeyCode::Right => {
                self.cursor = self.len().min(self.cursor + 1);
                FieldOutcome::Consumed
            }
            KeyCode::Home => {
                self.cursor = 0;
                FieldOutcome::Consumed
            }
            KeyCode::End => {
                self.cursor = self.len();
                FieldOutcome::Consumed
            }
            KeyCode::Enter => FieldOutcome::Submit,
            KeyCode::Esc => FieldOutcome::Cancel,
            _ => FieldOutcome::Pass,
        }
    }

    /// A bracketed paste (MOD-22 review M-1), inserted at the cursor as one edit; `false` when it
    /// was refused and the field is unchanged.
    ///
    /// One line: line breaks and every other control character are dropped rather than inserted,
    /// so a copied trailing newline is not an `Enter`. A **masked** field takes the paste only
    /// whole and only when it fits the capacity the field was opened with
    /// ([`masked_with_capacity`](TextField::masked_with_capacity)), so its buffer never
    /// reallocates and no prefix of a secret is freed unwiped on the way; one that does not fit is
    /// refused whole (`false`) for the caller to say so. An unmasked field grows as it needs to,
    /// reserving once.
    pub fn on_paste(&mut self, text: &str) -> bool {
        let kept = || text.chars().filter(|c| !c.is_control());
        let adds: usize = kept().map(char::len_utf8).sum();
        if adds == 0 {
            return true;
        }
        if self.masked && self.text.len() + adds > self.text.capacity() {
            return false;
        }
        self.text.reserve(adds);
        let mut at = self.byte_of(self.cursor);
        for c in kept() {
            self.text.insert(at, c);
            at += c.len_utf8();
        }
        self.cursor = self
            .text
            .grapheme_indices(true)
            .take_while(|(byte, _)| *byte < at)
            .count();
        true
    }

    /// What was typed, or `None` while the field is masked.
    ///
    /// A masked buffer leaves only through [`take`](TextField::take), so a caller cannot read a
    /// secret by accident and cannot read one twice.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        if self.masked {
            None
        } else {
            Some(self.text.as_str())
        }
    }

    /// What was typed before the cursor, or `None` while the field is masked: what a key would
    /// insert after.
    #[must_use]
    pub fn before_cursor(&self) -> Option<&str> {
        self.text().map(|text| &text[..self.byte_of(self.cursor)])
    }

    /// Moves the buffer out, leaving the field empty. The only read of a masked field.
    ///
    /// The allocation **moves**: nothing is copied, so a masked caller that wraps the result in
    /// `Zeroizing` on the same line is holding the very bytes that were typed and is the only one
    /// holding them. What it leaves behind is a fresh empty `String`, so a field that is going to
    /// be typed into again wants a new [`masked`](TextField::masked) rather than this one reused.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        core::mem::take(&mut *self.text)
    }

    /// Empties the field, wiping the buffer where it is (D3).
    ///
    /// `String::clear` would set the length to zero and leave every byte of what was typed in the
    /// allocation; this writes over the whole capacity and keeps it.
    pub fn clear(&mut self) {
        self.text.zeroize();
        self.cursor = 0;
    }

    /// How many grapheme clusters are in the buffer.
    ///
    /// The same number the mask draws a `•` for, so the dots and the `(n)` printed beside them
    /// can never disagree (MOD-54 D6). For ASCII that is the code point count, unchanged.
    #[must_use]
    pub fn len(&self) -> usize {
        graphemes(&self.text).count()
    }

    /// Whether nothing has been typed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Whether this field draws `•` instead of what was typed.
    #[must_use]
    pub const fn is_masked(&self) -> bool {
        self.masked
    }

    /// The field as one line of at most `width` **cells**, for the caller to place in its own
    /// `Rect`.
    ///
    /// A window of the graphemes ending at the cursor, counted in display cells, with a leading `…`
    /// when the start is clipped and nothing at all when the end is; the cursor cell carries
    /// `theme.selected` while `focused`. A masked field appends a dim ` (n)` and the window is
    /// sized around it. Computed on every call — nothing is cached, so a resize needs no event.
    ///
    /// Two invariants hold at any width: the drawn line is never wider than `width` cells, and the
    /// cursor is always on screen. The second is why a cluster too wide for the whole window is
    /// replaced by a single styled space rather than drawn and allowed to overrun (MOD-54 D10).
    #[must_use]
    pub fn line(&self, width: u16, focused: bool, theme: &Theme) -> Line<'static> {
        let width = usize::from(width);
        let suffix = if self.masked {
            format!(" ({})", self.len())
        } else {
            String::new()
        };
        let budget = width.saturating_sub(cell_width(&suffix));
        let glyphs: Vec<&str> = if self.masked {
            vec!["•"; self.len()]
        } else {
            graphemes(&self.text).collect()
        };

        // The cell under the cursor: the cluster there, or a space past the end of the buffer. A
        // cluster of no width still needs a cell to sit in, or the cursor has nowhere to go — and
        // `ratatui` filters every zero-width string out of what it writes, so such a cluster
        // would draw as nothing at all (D14). `U+FFFD` is this widget's existing "a character is
        // here with no glyph".
        let at: String = match glyphs.get(self.cursor) {
            Some(g) if cell_width(g) == 0 => "\u{fffd}".to_owned(),
            Some(g) => (*g).to_owned(),
            None => " ".to_owned(),
        };
        let at_width = cell_width(&at);
        let head_width: usize = glyphs[..self.cursor.min(glyphs.len())]
            .iter()
            .map(|g| cell_width(g))
            .sum();

        let cursor_style: Style = if focused { theme.selected } else { theme.base };
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(5);

        // A cluster wider than the whole window cannot be drawn and cannot be split, and
        // `ratatui` would stop the string at it and draw nothing — leaving an invisible cursor. So
        // the cursor takes the one cell it has (D10).
        if at_width > budget {
            spans.push(Span::styled(" ".to_owned(), cursor_style));
            if !suffix.is_empty() {
                spans.push(Span::styled(suffix, theme.dim));
            }
            return Line::from(spans);
        }

        // `budget` cells hold `['…'] + before + the cursor cell + after`, so the cursor is always
        // on screen; below two cells there is room for the cursor and nothing else.
        let (start, ellipsis) = if head_width + at_width <= budget {
            // `head_width + at_width <= budget` is today's `cursor < budget`: the whole head plus
            // the cursor cell fit.
            (0, false)
        } else if budget <= at_width {
            // No cell is left for the ellipsis beside the cursor cell, so the cursor is all that
            // is drawn (and `budget - at_width - 1` below would underflow).
            (self.cursor.min(glyphs.len()), false)
        } else {
            // Otherwise show as much of the tail as fits: the smallest `start` whose shown tail
            // leaves room for the ellipsis and the cursor cell. The scan runs upward and sums the
            // *hidden* head, so `head_width - head` is the width of what is shown.
            let room = budget - at_width - 1;
            let mut head = 0;
            let mut start = 0;
            for g in glyphs.iter().take(self.cursor.min(glyphs.len())) {
                if head_width - head <= room {
                    break;
                }
                head += cell_width(g);
                start += 1;
            }
            (start, true)
        };

        let before: String = glyphs[start.min(glyphs.len())..self.cursor.min(glyphs.len())]
            .iter()
            .map(|g| (*g).to_owned())
            .collect();
        let before_width = cell_width(&before);

        if ellipsis {
            spans.push(Span::styled("…", theme.dim));
        }
        if !before.is_empty() {
            spans.push(Span::styled(before, theme.base));
        }
        spans.push(Span::styled(at, cursor_style));

        // The tail, whole clusters only: the first that would overrun the window ends it.
        let room = budget
            .saturating_sub(usize::from(ellipsis))
            .saturating_sub(before_width)
            .saturating_sub(at_width);
        let mut used = 0;
        let mut after = String::new();
        for g in glyphs.iter().skip(self.cursor + 1) {
            let w = cell_width(g);
            if used + w > room {
                break;
            }
            used += w;
            after.push_str(g);
        }
        if !after.is_empty() {
            spans.push(Span::styled(after, theme.base));
        }
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, theme.dim));
        }
        Line::from(spans)
    }

    /// Byte offset of grapheme `index`, or the buffer's length past the end.
    fn byte_of(&self, index: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .nth(index)
            .map_or(self.text.len(), |(byte, _)| byte)
    }

    /// Inserts one char at the cursor and steps over the whole cluster it joined.
    ///
    /// A combining mark, a ZWJ or a variation selector merges into the cluster **before** it, so
    /// the cluster count does not grow and `cursor += 1` would leave the cursor past the end of
    /// the buffer. The count is recomputed instead: the number of clusters starting before the
    /// byte just past the insertion. That is 1 for `"a"` + `U+0301` (one cluster, cursor at the
    /// end), and 1 for `"ab"` + a mark at 1 (two clusters, cursor on the `b`).
    fn insert(&mut self, c: char) {
        let at = self.byte_of(self.cursor);
        self.text.insert(at, c);
        self.cursor = self
            .text
            .grapheme_indices(true)
            .take_while(|(byte, _)| *byte < at + c.len_utf8())
            .count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;
    use ratatui::widgets::{Paragraph, Widget as _};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// One line drawn at `width`, the way a caller places it.
    fn drawn(field: &TextField, width: u16, focused: bool) -> Buffer {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(field.line(width, focused, &Theme::default())).render(area, &mut buffer);
        buffer
    }

    /// The drawn row as snapshot text, trailing blanks trimmed as `testkit::buffer_text` trims.
    fn drawn_text(field: &TextField, width: u16, focused: bool) -> String {
        let buffer = drawn(field, width, focused);
        (0..width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    /// The widget's own line, spans joined: the string `ratatui` is handed. Unlike the rendered
    /// buffer this keeps a wide cluster as one string, so it can be measured in cells.
    fn line_text(field: &TextField, width: u16, focused: bool) -> String {
        field
            .line(width, focused, &Theme::default())
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn inserts_at_the_cursor() {
        let mut field = TextField::with_text("ac");
        assert_eq!(field.on_key(key(KeyCode::Left)), FieldOutcome::Consumed);
        assert_eq!(
            field.on_key(key(KeyCode::Char('b'))),
            FieldOutcome::Consumed
        );
        assert_eq!(field.text(), Some("abc"), "`b` lands before `c`");
        assert_eq!(field.len(), 3);

        // A multi-byte char: the cursor is a grapheme index, so inserting after it is not a byte + 1.
        let mut wide = TextField::with_text("é");
        wide.on_key(key(KeyCode::Home));
        wide.on_key(key(KeyCode::Char('a')));
        assert_eq!(wide.text(), Some("aé"));
    }

    /// The text before the cursor, sliced at a char boundary; nothing of a masked field.
    #[test]
    fn before_cursor_is_the_text_up_to_the_cursor() {
        let mut field = TextField::with_text("aéc");
        assert_eq!(
            field.before_cursor(),
            Some("aéc"),
            "the cursor starts at the end"
        );
        field.on_key(key(KeyCode::Left));
        assert_eq!(field.before_cursor(), Some("aé"));
        field.on_key(key(KeyCode::Home));
        assert_eq!(field.before_cursor(), Some(""));

        let mut masked = TextField::masked();
        masked.on_key(key(KeyCode::Char('s')));
        assert_eq!(masked.before_cursor(), None, "a masked field is never read");
    }

    #[test]
    fn backspace_and_delete_at_both_ends() {
        let mut field = TextField::with_text("abc");
        field.on_key(key(KeyCode::Backspace));
        assert_eq!(field.text(), Some("ab"));

        field.on_key(key(KeyCode::Home));
        field.on_key(key(KeyCode::Backspace));
        assert_eq!(
            field.text(),
            Some("ab"),
            "backspace at the start does nothing"
        );

        field.on_key(key(KeyCode::Delete));
        assert_eq!(field.text(), Some("b"));

        field.on_key(key(KeyCode::End));
        field.on_key(key(KeyCode::Delete));
        assert_eq!(field.text(), Some("b"), "delete at the end does nothing");
    }

    #[test]
    fn home_and_end_move() {
        let mut field = TextField::with_text("abc");
        field.on_key(key(KeyCode::Home));
        field.on_key(key(KeyCode::Char('x')));
        assert_eq!(field.text(), Some("xabc"));

        field.on_key(key(KeyCode::End));
        field.on_key(key(KeyCode::Char('y')));
        assert_eq!(field.text(), Some("xabcy"));

        // `Left` at the start and `Right` at the end saturate rather than wrap.
        field.on_key(key(KeyCode::Home));
        field.on_key(key(KeyCode::Left));
        field.on_key(key(KeyCode::Char('0')));
        assert_eq!(field.text(), Some("0xabcy"));
        field.on_key(key(KeyCode::End));
        field.on_key(key(KeyCode::Right));
        field.on_key(key(KeyCode::Char('1')));
        assert_eq!(field.text(), Some("0xabcy1"));
    }

    #[test]
    fn a_control_char_is_swallowed() {
        let mut field = TextField::with_text("ab");
        assert_eq!(
            field.on_key(key(KeyCode::Char('\u{7}'))),
            FieldOutcome::Consumed,
            "a control char is eaten, not passed on to the section"
        );
        assert_eq!(field.text(), Some("ab"), "and nothing is inserted");
    }

    #[test]
    fn enter_esc_tab_outcomes() {
        let mut field = TextField::new();
        assert_eq!(field.on_key(key(KeyCode::Enter)), FieldOutcome::Submit);
        assert_eq!(field.on_key(key(KeyCode::Esc)), FieldOutcome::Cancel);
        assert_eq!(field.on_key(key(KeyCode::Tab)), FieldOutcome::Pass);
        assert_eq!(field.on_key(key(KeyCode::BackTab)), FieldOutcome::Pass);
        assert_eq!(field.on_key(key(KeyCode::Up)), FieldOutcome::Pass);
        assert_eq!(field.on_key(key(KeyCode::Down)), FieldOutcome::Pass);
        assert_eq!(field.on_key(key(KeyCode::F(2))), FieldOutcome::Pass);
        assert_eq!(
            field.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            FieldOutcome::Pass,
            "`ctrl-c` still quits the shell while a field has focus"
        );
        assert_eq!(
            field.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            FieldOutcome::Pass
        );
        // The three the kitty protocol reports and a plain terminal never does: a chord is a
        // chord, and `super-w` must not type a `w` into a slug.
        for modifier in [KeyModifiers::SUPER, KeyModifiers::META, KeyModifiers::HYPER] {
            assert_eq!(
                field.on_key(KeyEvent::new(KeyCode::Char('w'), modifier)),
                FieldOutcome::Pass,
                "`{modifier:?}` is a chord, not a character"
            );
        }
        assert!(field.is_empty(), "none of those typed anything");

        // `SHIFT` is how a terminal reports a capital, so it must not pass.
        assert_eq!(
            field.on_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT)),
            FieldOutcome::Consumed
        );
        assert_eq!(field.text(), Some("N"));
    }

    #[test]
    fn the_window_leads_with_an_ellipsis_at_width() {
        let field = TextField::with_text("abcdefghijkl");
        assert_eq!(
            drawn_text(&field, 10, true),
            "…efghijkl",
            "ten cells: `…`, eight chars and the cursor's trailing space"
        );

        let short = TextField::with_text("abc");
        assert_eq!(
            drawn_text(&short, 10, true),
            "abc",
            "nothing clipped, no `…`"
        );
    }

    #[test]
    fn the_cursor_cell_is_selected_only_when_focused() {
        let mut field = TextField::with_text("abcdefghijkl");
        field.on_key(key(KeyCode::Home));
        for _ in 0..5 {
            field.on_key(key(KeyCode::Right));
        }
        assert_eq!(
            drawn_text(&field, 10, true),
            "abcdefghij",
            "the window starts at 0 and the tail is clipped silently"
        );

        let focused = drawn(&field, 10, true);
        assert!(
            focused[(5, 0)].modifier.contains(Modifier::REVERSED),
            "the cursor cell is a style, not a character"
        );
        assert!(!focused[(4, 0)].modifier.contains(Modifier::REVERSED));

        let unfocused = drawn(&field, 10, false);
        assert!(!unfocused[(5, 0)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn a_masked_field_renders_dots_and_a_count() {
        let mut field = TextField::masked();
        for c in "hunter2".chars() {
            field.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            drawn_text(&field, 12, true),
            "•••••••  (7)",
            "seven dots, the cursor's space, then the dim count"
        );

        // The suffix is reserved first: at width 8 a twelve-char secret keeps one dot and the `…`.
        let mut long = TextField::masked();
        for c in "0123456789ab".chars() {
            long.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(drawn_text(&long, 8, true), "…•  (12)");
    }

    #[test]
    fn text_is_none_when_masked() {
        let mut field = TextField::masked();
        field.on_key(key(KeyCode::Char('s')));
        assert!(field.is_masked());
        assert_eq!(
            field.text(),
            None,
            "a masked buffer leaves only through `take`"
        );
        assert_eq!(field.len(), 1);
    }

    #[test]
    fn take_empties_the_field() {
        let mut field = TextField::with_text("secret");
        assert_eq!(field.take(), "secret");
        assert!(field.is_empty());
        assert_eq!(field.len(), 0);
        assert_eq!(field.take(), "", "a second read gets nothing");

        let mut cleared = TextField::with_text("gone");
        cleared.clear();
        assert!(cleared.is_empty());
    }

    /// D3: a masked field reserves its buffer up front, so a DSN typed one key at a time never
    /// reallocates.
    ///
    /// The reservation is not decoration. [`Zeroizing`] wipes the allocation a `String` **holds**;
    /// a `push` that outgrows the current capacity copies the bytes into a new allocation and
    /// frees the old one with the secret still in it, and nothing in safe Rust can go back for it.
    /// 256 bytes is past any DSN a person types, so the wipe on drop covers the whole life of the
    /// buffer rather than its last few characters.
    #[test]
    fn a_masked_field_reserves_its_buffer() {
        let mut field = TextField::masked();
        let reserved = field.text.capacity();
        assert!(
            reserved >= 256,
            "a masked field starts with room for a DSN, not with nothing: {reserved}"
        );

        for c in "postgres://htui:s3cret@db.example.internal:5432/htui?sslmode=require".chars() {
            field.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(field.len(), 68, "a DSN of ordinary length");
        assert_eq!(
            field.text.capacity(),
            reserved,
            "and it fitted in the first allocation, so there is no trail of half-typed copies"
        );

        // The unmasked field makes no such promise and should not pay for one: it holds a slug, a
        // name or a path, and it is not what this reservation exists for.
        assert_eq!(TextField::new().text.capacity(), 0);
    }

    /// MOD-22 D275: a field opened at the longest paste its caller accepts takes a pasted redirect
    /// of several hundred characters in its first allocation. At `masked()`'s 256 bytes the same
    /// paste would have left an unwiped copy of its first 256 and 512 bytes behind.
    #[test]
    fn a_masked_field_opened_with_capacity_takes_a_long_paste_without_reallocating() {
        let mut field = TextField::masked_with_capacity(8192);
        let reserved = field.text.capacity();
        assert!(
            reserved >= 8192,
            "the reservation is what was asked for: {reserved}"
        );
        assert!(field.is_masked());

        let paste = "a".repeat(600);
        for c in paste.chars() {
            field.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(field.len(), 600);
        assert_eq!(
            field.text.capacity(),
            reserved,
            "600 bytes fitted in the first allocation, so nothing was freed unwiped"
        );
        assert_eq!(field.take(), paste);
    }

    /// Review M-1: a paste is one insertion at the cursor, its line breaks and control characters
    /// dropped, and the cursor lands after it.
    #[test]
    fn a_paste_inserts_at_the_cursor_without_line_breaks() {
        let mut field = TextField::with_text("ae");
        field.on_key(key(KeyCode::Left));
        assert!(field.on_paste("b\r\nc\td\u{7}\n"));
        assert_eq!(field.text(), Some("abcde"));
        assert_eq!(field.cursor, 4, "after the pasted `d`, before `e`");

        assert!(field.on_paste("\n\r"), "nothing to insert is not a refusal");
        assert_eq!(field.text(), Some("abcde"));

        let mut marks = TextField::with_text("x");
        assert!(marks.on_paste("a\u{301}\u{1f468}\u{200d}\u{1f469}"));
        assert_eq!(marks.len(), 3);
        assert_eq!(marks.cursor, 3, "the cursor counts clusters, not chars");
    }

    /// Review M-1: a masked field takes a paste within its reservation — drawn as dots — and
    /// refuses one past it whole, so the buffer never reallocates.
    #[test]
    fn a_masked_field_takes_a_paste_that_fits_and_refuses_one_that_does_not() {
        let mut field = TextField::masked_with_capacity(64);
        let reserved = field.text.capacity();
        assert!(field.on_paste("hunter2\n"));
        assert_eq!(drawn_text(&field, 12, true), "•••••••  (7)");
        assert_eq!(field.text(), None, "and it is still unreadable");

        assert!(
            !field.on_paste(&"x".repeat(reserved)),
            "a paste past the reservation is refused"
        );
        assert_eq!(field.len(), 7, "whole: nothing of it went in");
        assert_eq!(field.text.capacity(), reserved);

        assert!(
            field.on_paste(&"y".repeat(reserved - 7)),
            "one that fits exactly"
        );
        assert_eq!(field.text.capacity(), reserved, "and nothing reallocated");
        assert_eq!(field.take().len(), reserved);
    }

    /// D3: `clear` wipes the buffer where it is instead of dropping it.
    ///
    /// `String::clear` would set the length to zero and leave every byte of the secret in the
    /// allocation, which the next `Debug` of a heap dump still reads. `Zeroize` writes the whole
    /// capacity — the initialised bytes and the spare — and keeps the allocation, which is what
    /// the unchanged capacity below pins: a wipe in place, not a free and a new one somewhere else.
    ///
    /// What it cannot assert is the bytes themselves: reading a `String`'s spare capacity needs
    /// `unsafe`, which the workspace forbids. The reachable half is what is asserted; the rest is
    /// `zeroize`'s own contract for `String` and is priced as such in the module doc.
    #[test]
    fn clear_wipes_the_buffer_in_place() {
        let mut field = TextField::masked();
        for c in "hunter2".chars() {
            field.on_key(key(KeyCode::Char(c)));
        }
        let reserved = field.text.capacity();

        field.clear();

        assert!(field.is_empty(), "nothing is left to read");
        assert_eq!(field.len(), 0);
        assert_eq!(field.cursor, 0, "and the cursor came back with it");
        assert_eq!(
            field.text.capacity(),
            reserved,
            "the same allocation, zeroed, rather than a freed one still holding the secret"
        );

        // `take` is the other disposal point, and it moves the buffer out rather than copying it:
        // the field is left empty, and what the caller holds is the very allocation that was
        // typed into, for it to wipe in turn.
        let mut taken = TextField::masked();
        for c in "hunter2".chars() {
            taken.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(taken.take(), "hunter2");
        assert!(taken.is_empty());
        assert_eq!(taken.cursor, 0);
    }

    #[test]
    fn debug_never_prints_the_text() {
        let plain = TextField::with_text("secret");
        let rendered = format!("{plain:?}");
        assert!(
            !rendered.contains("secret"),
            "an unmasked field is printed by `Debug` too: {rendered}"
        );
        assert!(rendered.contains("len: 6"), "{rendered}");

        let mut masked = TextField::masked();
        for c in "secret".chars() {
            masked.on_key(key(KeyCode::Char(c)));
        }
        let rendered = format!("{masked:?}");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(rendered.contains("masked: true"), "{rendered}");
    }

    // ---- MOD-54 ---------------------------------------------------------------------------------

    /// D4: a wide grapheme is two cells, so a field holding two of them is not as long as its
    /// code point count says.
    #[test]
    fn a_wide_grapheme_is_two_cells() {
        let field = TextField::with_text("漢字");
        assert_eq!(
            line_text(&field, 6, true),
            "漢字 ",
            "four cells and the cursor's cell"
        );
        assert!(
            cell_width(&line_text(&field, 6, true)) <= 6,
            "and never past the six cells it was given"
        );
    }

    /// D4: the window is sized in cells. Twelve CJK clusters are twenty-four cells, so a ten-cell
    /// window shows four of them and an ellipsis, not nine of them and a half.
    #[test]
    fn the_window_counts_cells_not_chars() {
        let field = TextField::with_text(&"一".repeat(12));
        let row = line_text(&field, 10, true);
        assert_eq!(row, format!("…{} ", "一".repeat(4)), "{row:?}");
        assert_eq!(cell_width(&row), 10, "exactly the window it was given");
    }

    /// D5: `ratatui` writes the style onto the first cell of a grapheme and then *resets* the
    /// continuation cell, so no `Span` can reverse both. The first cell carries the cursor.
    #[test]
    fn a_wide_cursor_cell_highlights_its_first_cell() {
        let mut field = TextField::with_text("漢字");
        field.on_key(key(KeyCode::Home));
        let buffer = drawn(&field, 6, true);
        assert!(
            buffer[(0, 0)].modifier.contains(Modifier::REVERSED),
            "the cluster's first cell carries the cursor"
        );
        assert!(
            !buffer[(1, 0)].modifier.contains(Modifier::REVERSED),
            "and its continuation cell is not styled, because `ratatui` resets it"
        );
    }

    /// D3: the cursor steps by cluster, so it never lands inside a combining sequence or between
    /// the halves of a ZWJ sequence.
    #[test]
    fn left_and_right_step_by_grapheme() {
        let mut field = TextField::with_text("a\u{301}👨‍👩‍👧c");
        assert_eq!(
            field.len(),
            3,
            "base+mark is one, the family emoji is one, and `c`"
        );
        field.on_key(key(KeyCode::End));
        assert_eq!(field.cursor, 3);

        field.on_key(key(KeyCode::Right));
        assert_eq!(field.cursor, 3, "right at the end does nothing");
        for _ in 0..2 {
            field.on_key(key(KeyCode::Left));
        }
        assert_eq!(
            field.cursor, 1,
            "one step over the family emoji, not into it"
        );
        field.on_key(key(KeyCode::Right));
        assert_eq!(field.cursor, 2, "and back past the whole cluster");
    }

    /// D3: `Backspace` removes a whole cluster, leaving no orphaned mark behind.
    #[test]
    fn backspace_removes_a_whole_grapheme() {
        let mut field = TextField::with_text("a\u{301}b");
        field.on_key(key(KeyCode::Backspace));
        assert_eq!(
            field.text(),
            Some("a\u{301}"),
            "not a base and a stray mark"
        );
    }

    /// Backspace and Delete over a cluster of several chars take all of it: a base and its mark,
    /// and a ZWJ family, leave nothing behind.
    #[test]
    fn backspace_and_delete_remove_a_multi_char_cluster() {
        for cluster in ["a\u{301}", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"] {
            let mut field = TextField::with_text(cluster);
            field.on_key(key(KeyCode::Backspace));
            assert_eq!(field.text(), Some(""), "Backspace over {cluster:?}");
            assert_eq!(field.cursor, 0);

            let mut field = TextField::with_text(&format!("{cluster}x"));
            field.on_key(key(KeyCode::Home));
            field.on_key(key(KeyCode::Delete));
            assert_eq!(field.text(), Some("x"), "Delete over {cluster:?}");
        }
    }

    /// A field whose budget is no wider than the cursor cell, with text before the cursor, draws
    /// only the cursor: there is no room for the ellipsis, and the tail scan must not underflow.
    #[test]
    fn a_budget_no_wider_than_the_cursor_draws_only_the_cursor() {
        let mut field = TextField::with_text("a\u{4e00}");
        field.on_key(key(KeyCode::Left));
        assert_eq!(line_text(&field, 2, true), "\u{4e00}");

        let mut masked = TextField::masked();
        for _ in 0..12 {
            masked.on_key(key(KeyCode::Char('x')));
        }
        // " (12)" takes five of the six cells, leaving one for the cursor at the end.
        assert_eq!(line_text(&masked, 6, true), "  (12)");
    }

    /// D3's `insert`: a typed mark merges into the cluster before it, so the count does not grow
    /// and `cursor += 1` would leave the cursor past the end of the buffer. This is the assertion
    /// that catches the naive port.
    #[test]
    fn a_typed_combining_mark_joins_the_cluster_before_it() {
        let mut field = TextField::new();
        for c in ["a", "\u{301}", "b"] {
            field.on_key(key(KeyCode::Char(c.chars().next().expect("one char"))));
        }
        assert_eq!(field.text(), Some("a\u{301}b"));
        assert_eq!(field.len(), 2, "the mark and its base are one cluster");
        assert_eq!(field.cursor, 2, "and the cursor is after `b`");
    }

    /// D10: a wide grapheme that does not fit is dropped whole, never half-drawn at the edge.
    #[test]
    fn a_wide_grapheme_that_does_not_fit_is_dropped_not_split() {
        let field = TextField::with_text("一x");
        let row = line_text(&field, 3, true);
        assert_eq!(row, "…x ", "the two-cell cluster is dropped, not split");
        assert_eq!(cell_width(&row), 3);
    }

    /// D10's exception: a cluster wider than the whole window still has to leave a visible cursor.
    #[test]
    fn a_wide_cursor_in_a_one_cell_field_draws_a_space() {
        let mut field = TextField::with_text("一");
        field.on_key(key(KeyCode::Home));
        assert_eq!(
            line_text(&field, 1, true),
            " ",
            "the cursor stays visible in a field that cannot hold the cluster"
        );
    }

    /// D14: a zero-width cluster occupies no cell, so the cursor draws `U+FFFD` in one — otherwise
    /// `ratatui` filters it out and the cursor vanishes.
    #[test]
    fn a_zero_width_cluster_at_the_cursor_draws_a_replacement_glyph() {
        let mut field = TextField::with_text("\u{301}a");
        field.on_key(key(KeyCode::Home));
        assert_eq!(
            line_text(&field, 3, true),
            "\u{fffd}a",
            "a cell for a cluster that is no width"
        );
    }

    /// D6: the mask draws one dot per grapheme and prints that same number, so a decomposed and a
    /// precomposed `é` read as the two characters a person typed, not as three code points.
    #[test]
    fn a_masked_field_counts_graphemes_not_code_points() {
        let mut field = TextField::masked();
        for c in ["\u{e9}", "e", "\u{301}"] {
            field.on_key(key(KeyCode::Char(c.chars().next().expect("one char"))));
        }
        assert_eq!(field.len(), 2, "two graphemes, three code points");
        assert_eq!(drawn_text(&field, 12, true), "••  (2)");
    }

    /// D17c: today's exact expectations, inlined. The item's regression net is that a rendering
    /// change for narrow text is a diff someone can read, not a rewritten test that looks the same.
    #[test]
    fn ascii_rendering_is_unchanged() {
        assert_eq!(
            drawn_text(&TextField::with_text("abcdefghijkl"), 10, true),
            "…efghijkl"
        );
        assert_eq!(drawn_text(&TextField::with_text("abc"), 10, true), "abc");

        let mut at_five = TextField::with_text("abcdefghijkl");
        at_five.on_key(key(KeyCode::Home));
        for _ in 0..5 {
            at_five.on_key(key(KeyCode::Right));
        }
        assert_eq!(drawn_text(&at_five, 10, true), "abcdefghij");

        let mut masked = TextField::masked();
        for c in "hunter2".chars() {
            masked.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(drawn_text(&masked, 12, true), "•••••••  (7)");

        let mut long = TextField::masked();
        for c in "0123456789ab".chars() {
            long.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(drawn_text(&long, 8, true), "…•  (12)");
    }
}
