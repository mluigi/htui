//! The `[field]` translation (MOD-115 M2 D7, blueprint §3): while the active stack captures text
//! ([`Stack::captures_text`]), a chord bound to a `[field]` row becomes that row's canonical
//! editing event, so `TextField`, `TextArea` and `Composer` keep matching their own literals and
//! every editing key is still rebindable.
//!
//! Rules, in order ([`translate`]):
//! 1. a stack that captures no text: the key, untouched;
//! 2. a printable chord: the key, untouched (typed text is the one exception to MD1);
//! 3. `Esc`: the key, untouched. `Esc` is never configurable (MD7): `field.cancel` keeps it
//!    whatever the key file says, so the translation never drops it;
//! 4. the first candidate is a `[field]` act: that act's canonical event, with no modifier
//!    (`field.cancel = ["ctrl-o"]` makes `ctrl-o` the widgets' `Esc`);
//! 5. a plain canonical key its `[field]` row did not claim first: swallowed when nothing in the
//!    stack binds it, else the key, untouched, for the view or the shell to resolve (D-13: a
//!    focused widget still reads such a key literally, e.g. `left` kept by `choice.prev` after
//!    `field.left` moved off it);
//! 6. anything else: the key, untouched.
//!
//! MOD-115 M2 Task 5 writes it; Task 6 calls it from `App::on_key`. A site that needs submit or
//! cancel semantics after the translation decodes the event with [`FieldEvent::of`], never with
//! `Keys`: the event is canonical by then.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Act, KeyChord, Keys, Stack};

/// One canonical editing event of a text widget: a `[field]` row's meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldEvent {
    /// `field.submit`: `Enter`.
    Submit,
    /// `field.cancel`: `Esc`.
    Cancel,
    /// `field.left`: the cursor one grapheme left.
    Left,
    /// `field.right`: the cursor one grapheme right.
    Right,
    /// `field.home`: the line start.
    Home,
    /// `field.end`: the line end.
    End,
    /// `field.delete_back`: `Backspace`.
    DeleteBack,
    /// `field.delete_forward`: `Delete`.
    DeleteForward,
    /// `field.line_up`: a `TextArea`'s line up.
    LineUp,
    /// `field.line_down`: a `TextArea`'s line down.
    LineDown,
    /// `field.page_up`: a `TextArea`'s page up.
    PageUp,
    /// `field.page_down`: a `TextArea`'s page down.
    PageDown,
}

impl FieldEvent {
    /// Every event, in catalogue order.
    pub const ALL: [Self; 12] = [
        Self::Submit,
        Self::Cancel,
        Self::Left,
        Self::Right,
        Self::Home,
        Self::End,
        Self::DeleteBack,
        Self::DeleteForward,
        Self::LineUp,
        Self::LineDown,
        Self::PageUp,
        Self::PageDown,
    ];

    /// The `[field]` row this event is.
    #[must_use]
    pub const fn act(self) -> Act {
        match self {
            Self::Submit => Act::FieldSubmit,
            Self::Cancel => Act::FieldCancel,
            Self::Left => Act::FieldLeft,
            Self::Right => Act::FieldRight,
            Self::Home => Act::FieldHome,
            Self::End => Act::FieldEnd,
            Self::DeleteBack => Act::FieldDeleteBack,
            Self::DeleteForward => Act::FieldDeleteForward,
            Self::LineUp => Act::FieldLineUp,
            Self::LineDown => Act::FieldLineDown,
            Self::PageUp => Act::FieldPageUp,
            Self::PageDown => Act::FieldPageDown,
        }
    }

    /// The event a `[field]` act is, or `None` for any other act.
    #[must_use]
    pub fn from_act(act: Act) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.act() == act)
    }

    /// The key the widgets match for this event: their own literals.
    #[must_use]
    pub const fn canonical(self) -> KeyCode {
        match self {
            Self::Submit => KeyCode::Enter,
            Self::Cancel => KeyCode::Esc,
            Self::Left => KeyCode::Left,
            Self::Right => KeyCode::Right,
            Self::Home => KeyCode::Home,
            Self::End => KeyCode::End,
            Self::DeleteBack => KeyCode::Backspace,
            Self::DeleteForward => KeyCode::Delete,
            Self::LineUp => KeyCode::Up,
            Self::LineDown => KeyCode::Down,
            Self::PageUp => KeyCode::PageUp,
            Self::PageDown => KeyCode::PageDown,
        }
    }

    /// The event a *plain* canonical key is (no modifier but `SHIFT`, as the widgets read it), or
    /// `None`: `Esc` is `Cancel`, `ctrl-g` and `alt-enter` are nothing.
    #[must_use]
    pub fn of(key: &KeyEvent) -> Option<Self> {
        if !(key.modifiers - KeyModifiers::SHIFT).is_empty() {
            return None;
        }
        Self::ALL
            .into_iter()
            .find(|event| event.canonical() == key.code)
    }
}

/// The key the view sees while `stack` is active (the rules in the module doc): `Some(key)`
/// delivered, possibly as a `[field]` row's canonical event, or `None`, swallowed.
#[must_use]
pub fn translate(keys: &Keys, stack: Stack<'_>, key: KeyEvent) -> Option<KeyEvent> {
    if !stack.captures_text() {
        return Some(key);
    }
    let chord = KeyChord::from_event(key);
    if chord.is_printable() || key.code == KeyCode::Esc {
        return Some(key);
    }
    let candidates = keys.actions(stack, chord);
    if let Some(event) = candidates.first().copied().and_then(FieldEvent::from_act) {
        return Some(KeyEvent {
            code: event.canonical(),
            modifiers: KeyModifiers::NONE,
            kind: key.kind,
            state: key.state,
        });
    }
    if FieldEvent::of(&key).is_some() && candidates.is_empty() {
        return None;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::{FieldEvent, translate};
    use crate::keys::{Act, CTRL_C, Context, KeyChord, Keys, Stack, load_str, views};

    fn key(spec: &str) -> KeyEvent {
        KeyChord::parse_strict(spec)
            .expect("a valid spec")
            .to_event()
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn every_event_round_trips_through_its_act_and_its_canonical_key() {
        for event in FieldEvent::ALL {
            assert_eq!(FieldEvent::from_act(event.act()), Some(event));
            assert_eq!(FieldEvent::of(&plain(event.canonical())), Some(event));
            assert_eq!(
                event.act().spec().map(|spec| spec.context),
                Some(Context::Field),
                "{event:?}"
            );
        }
        assert_eq!(FieldEvent::from_act(Act::Quit), None);
    }

    /// The compiled `[field]` rows are the widgets' literals: translating them changes nothing.
    #[test]
    fn the_compiled_field_rows_are_the_canonical_keys() {
        let keys = Keys::compiled();
        for event in FieldEvent::ALL {
            let chord = KeyChord::new(event.canonical(), KeyModifiers::NONE);
            assert_eq!(
                keys.chords(Context::Field, event.act()),
                [chord],
                "{event:?}"
            );
        }
    }

    #[test]
    fn of_reads_only_a_plain_canonical_key() {
        assert_eq!(FieldEvent::of(&key("esc")), Some(FieldEvent::Cancel));
        assert_eq!(FieldEvent::of(&key("enter")), Some(FieldEvent::Submit));
        assert_eq!(
            FieldEvent::of(&key("shift-left")),
            Some(FieldEvent::Left),
            "SHIFT is how a terminal reports a capital"
        );
        for spec in ["ctrl-g", "alt-enter", "ctrl-left", "a", "tab", "f1"] {
            assert_eq!(FieldEvent::of(&key(spec)), None, "{spec}");
        }
    }

    /// Rule 1: a stack that captures no text is never translated.
    #[test]
    fn a_stack_that_captures_no_text_is_untouched() {
        let keys = Keys::defaults().with_chords(Context::Field, Act::FieldSubmit, &["ctrl-t"]);
        for stack in [Stack::BASE, views::BACKLOG_BODY, views::RUNS_CONFIRM] {
            assert!(!stack.captures_text());
            for spec in ["ctrl-t", "enter", "esc", "left", "x"] {
                assert_eq!(
                    translate(&keys, stack, key(spec)),
                    Some(key(spec)),
                    "{spec}"
                );
            }
        }
    }

    /// Rule 2: typed text is the one exception to "every key is a binding" (MD1).
    #[test]
    fn a_printable_key_is_text() {
        let keys = Keys::compiled();
        for spec in ["a", "Z", "space", "1", "?", "j", "l", "m"] {
            assert_eq!(
                translate(keys, views::REQUIREMENTS_FORM, key(spec)),
                Some(key(spec)),
                "{spec}"
            );
        }
    }

    /// Rule 3 (MD7): `Esc` always reaches the widget as itself, even from keys that moved
    /// `field.cancel` off it (only code can: a key file cannot list `esc`).
    #[test]
    fn esc_is_never_dropped() {
        let moved = Keys::defaults().with_chords(Context::Field, Act::FieldCancel, &["ctrl-g"]);
        for keys in [Keys::compiled(), &moved] {
            for stack in [views::CAPTURE, views::ITEM_FORM, views::CONCEPTS_QUERY] {
                assert_eq!(translate(keys, stack, key("esc")), Some(key("esc")));
            }
        }
    }

    /// Rule 4: a rebound `[field]` chord arrives as the widgets' canonical event, without its
    /// modifier, keeping the event's kind; a key file's `cancel` adds to `Esc`, never replaces it.
    #[test]
    fn a_rebound_field_chord_becomes_the_canonical_event() {
        let keys = load_str("version = 2\n[field]\ncancel = [\"ctrl-o\"]\nsubmit = [\"ctrl-t\"]\n")
            .expect("the file loads");
        assert_eq!(
            translate(&keys, views::CAPTURE, key("ctrl-o")),
            Some(plain(KeyCode::Esc))
        );
        assert_eq!(
            translate(&keys, views::CAPTURE, key("esc")),
            Some(plain(KeyCode::Esc))
        );
        assert_eq!(
            translate(&keys, views::ITEM_FORM, key("ctrl-t")),
            Some(plain(KeyCode::Enter))
        );
        let release = KeyEvent {
            kind: KeyEventKind::Release,
            ..key("ctrl-o")
        };
        let translated = translate(&keys, views::CAPTURE, release).expect("delivered");
        assert_eq!(translated.kind, KeyEventKind::Release);
        assert_eq!(translated.code, KeyCode::Esc);
    }

    /// Rule 5: a canonical key its row gave up is swallowed where nothing else binds it, and
    /// delivered unchanged where another act of the stack does (D-13).
    #[test]
    fn a_canonical_key_its_row_gave_up_is_swallowed_or_left_to_the_stack() {
        let keys = load_str("version = 2\n[field]\nsubmit = [\"ctrl-t\"]\nleft = [\"ctrl-b\"]\n")
            .expect("the file loads");
        assert_eq!(translate(&keys, views::CAPTURE, key("enter")), None);
        assert_eq!(translate(&keys, views::CAPTURE, key("left")), None);
        assert_eq!(
            translate(&keys, views::CAPTURE, key("ctrl-b")),
            Some(plain(KeyCode::Left))
        );
        // The item form's `left` is still `choice.prev`'s: the view resolves it.
        assert_eq!(
            keys.actions(views::ITEM_FORM, KeyChord::from_event(key("left"))),
            [Act::ChoicePrev]
        );
        assert_eq!(
            translate(&keys, views::ITEM_FORM, key("left")),
            Some(key("left"))
        );
    }

    /// The concepts search's `up`/`down` are the hit cursor's first (SHADOWING), so they reach
    /// the view unchanged; its `Esc` closes whatever `field.cancel` is bound to.
    #[test]
    fn the_concepts_search_keeps_its_arrows_and_its_esc() {
        let keys = load_str("version = 2\n[field]\ncancel = [\"ctrl-o\"]\n").expect("loads");
        for spec in ["up", "down", "esc"] {
            assert_eq!(
                translate(&keys, views::CONCEPTS_QUERY, key(spec)),
                Some(key(spec)),
                "{spec}"
            );
        }
        assert_eq!(
            translate(&keys, views::CONCEPTS_QUERY, key("ctrl-o")),
            Some(plain(KeyCode::Esc))
        );
    }

    /// Rule 6: `ctrl-c`, function keys and other chords no `[field]` row binds pass unchanged,
    /// so `ctrl-c` reaches the modal layer and `global.quit`.
    #[test]
    fn a_chord_no_field_row_binds_is_untouched() {
        let keys = Keys::compiled();
        let ctrl_c = CTRL_C.to_event();
        assert_eq!(translate(keys, views::CAPTURE, ctrl_c), Some(ctrl_c));
        assert_eq!(
            keys.actions(views::CAPTURE, CTRL_C),
            [Act::Quit],
            "ctrl-c is quit in a field"
        );
        for spec in ["f1", "tab", "backtab", "ctrl-s", "alt-x"] {
            assert_eq!(
                translate(keys, views::ITEM_FORM, key(spec)),
                Some(key(spec)),
                "{spec}"
            );
        }
    }
}
