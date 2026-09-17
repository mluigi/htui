//! One single-line text field with a char cursor and an optional mask (MOD-15 milestone 3, D1;
//! PRD D1). The widget MOD-22, MOD-23 and the connection section's DSN entry consume.
//!
//! Width is counted in `char`s, not display columns: no `unicode-width` is declared anywhere in
//! the workspace, and every field this widget opens holds a slug, a name, a branch, a path or a
//! DSN. Multi-line, history, bracketed paste, a reveal toggle and validation are deliberately not
//! built.
//!
//! **`Zeroizing` arrived with milestone 6** (D3), and it covers exactly this much: the buffer is
//! wiped when the field drops, when [`clear`](TextField::clear) is called, and — because
//! [`masked`](TextField::masked) reserves 256 bytes — without a trail of half-typed reallocations
//! behind it. What it does not reach is priced rather than claimed: the terminal's own input
//! buffering, the kernel's, and any allocation a `with_text` field outgrew before this. The one
//! secret this widget ever holds leaves it through [`take`](TextField::take), which **moves** the
//! allocation to a caller that wraps it in `Zeroizing` on the same line.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use zeroize::{Zeroize as _, Zeroizing};

use crate::ui::Theme;

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

/// A single-line buffer with a cursor, counted in `char`s.
#[derive(Clone, Default)]
pub struct TextField {
    /// What was typed. Never printed by [`Debug`](core::fmt::Debug).
    ///
    /// Wiped on drop (D3). For a slug that costs nothing; for a DSN it is the point — and it is
    /// the reason the type is the same for both, since a field that zeroized only when masked
    /// would be one `masked: false` away from not zeroizing at all.
    text: Zeroizing<String>,
    /// Char index, `0..=text.chars().count()`.
    cursor: usize,
    /// Whether [`line`](TextField::line) draws `•` per char and [`text`](TextField::text) refuses.
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

    /// An empty masked field: it draws `•` per char and [`text`](TextField::text) is `None`.
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
        Self {
            masked: true,
            text: Zeroizing::new(String::with_capacity(256)),
            ..Self::default()
        }
    }

    /// An unmasked field holding `text`, cursor at the end — how an editor prefills a row.
    #[must_use]
    pub fn with_text(text: &str) -> Self {
        Self {
            text: Zeroizing::new(text.to_owned()),
            cursor: text.chars().count(),
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
                    let byte = self.byte_of(self.cursor - 1);
                    self.text.remove(byte);
                    self.cursor -= 1;
                }
                FieldOutcome::Consumed
            }
            KeyCode::Delete => {
                if self.cursor < self.len() {
                    let byte = self.byte_of(self.cursor);
                    self.text.remove(byte);
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

    /// How many `char`s are in the buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.text.chars().count()
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

    /// The field as one line `width` cells wide, for the caller to place in its own `Rect`.
    ///
    /// A window of the chars ending at the cursor, with a leading `…` when the start is clipped
    /// and nothing at all when the end is; the cursor cell (a space past the last char) carries
    /// `theme.selected` while `focused`. A masked field appends a dim ` (n)` and the window is
    /// sized around it. Computed on every call — nothing is cached, so a resize needs no event.
    #[must_use]
    pub fn line(&self, width: u16, focused: bool, theme: &Theme) -> Line<'static> {
        let width = usize::from(width);
        let suffix = if self.masked {
            format!(" ({})", self.len())
        } else {
            String::new()
        };
        let budget = width.saturating_sub(suffix.chars().count());
        let glyphs: Vec<char> = if self.masked {
            core::iter::repeat_n('•', self.len()).collect()
        } else {
            self.text.chars().collect()
        };

        // `budget` cells hold `['…'] + before + cursor cell + after`, so the cursor is always on
        // screen; below two cells there is room for the cursor and nothing else.
        let (start, ellipsis) = if budget < 2 {
            (self.cursor, false)
        } else if self.cursor < budget {
            // `cursor < budget` is `cursor + 1 <= budget`: the whole head plus the cursor cell fit.
            (0, false)
        } else {
            (self.cursor + 2 - budget, true)
        };
        let before: String = glyphs[start.min(glyphs.len())..self.cursor.min(glyphs.len())]
            .iter()
            .collect();
        let at = glyphs.get(self.cursor).copied().unwrap_or(' ');
        let room = budget
            .saturating_sub(usize::from(ellipsis))
            .saturating_sub(before.chars().count())
            .saturating_sub(1);
        let after: String = glyphs
            .iter()
            .skip(self.cursor.saturating_add(1))
            .take(room)
            .collect();

        let cursor_style: Style = if focused { theme.selected } else { theme.base };
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(5);
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
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, theme.dim));
        }
        Line::from(spans)
    }

    /// Byte offset of char `index`, or the buffer's length past the end.
    fn byte_of(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(byte, _)| byte)
    }

    /// Inserts one char at the cursor and steps over it.
    fn insert(&mut self, c: char) {
        let byte = self.byte_of(self.cursor);
        self.text.insert(byte, c);
        self.cursor += 1;
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

        // A multi-byte char: the cursor is a char index, so inserting after it is not a byte + 1.
        let mut wide = TextField::with_text("é");
        wide.on_key(key(KeyCode::Home));
        wide.on_key(key(KeyCode::Char('a')));
        assert_eq!(wide.text(), Some("aé"));
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
}
