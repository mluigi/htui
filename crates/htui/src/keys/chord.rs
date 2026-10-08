//! One normalised keystroke, and the strict parser a key file is read with (ANA-26 §7.1).
//!
//! `KeyChord` moved here from `keymap.rs` (MOD-67 M1, plan D2); `crate::keymap::KeyChord`
//! re-exports it until M6. `parse` is the harness's lenient reader and is unchanged.
//! `parse_strict` is the key file's reader (ANA-26 §7.1); on Unix it also refuses the chords a
//! terminal in the legacy encoding never delivers as themselves (MOD-67 M2 D6).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One normalised keystroke.
///
/// Normalisation (see [`KeyChord::new`]) is what makes `Shift+Tab` typed on Windows Terminal and
/// `"shift-tab"` written in a table the same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// The key itself.
    pub code: KeyCode,
    /// Modifiers still meaningful after normalisation.
    pub mods: KeyModifiers,
}

impl KeyChord {
    /// A normalised chord.
    ///
    /// Three rules, all forced by how terminals report keys: `Shift+Tab` is `BackTab` without a
    /// shift flag; a `Char` already carries its shift in the character itself (`J`, `?`), so
    /// the flag is dropped there; and CONTROL with an ASCII capital is the lower-case letter,
    /// since a kitty-protocol terminal reports ctrl-shift-d as `D` and the legacy encoding as
    /// `d` (MOD-67 M3 PA-6). `parse_strict` still refuses `ctrl-D` (it checks before
    /// normalising).
    #[must_use]
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let (mut code, mut mods) = match code {
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => (KeyCode::BackTab, mods),
            other => (other, mods),
        };
        if matches!(code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        if mods.contains(KeyModifiers::CONTROL)
            && let KeyCode::Char(c) = code
            && c.is_ascii_uppercase()
        {
            code = KeyCode::Char(c.to_ascii_lowercase());
        }
        Self { code, mods }
    }

    /// Whether a capturing or confirming mode lets this chord through to the global layer
    /// (MOD-67 D5): CONTROL or ALT is held, or the key is a function key. `ctrl-c`, `ctrl-f`,
    /// `alt-x`, `F1` pass; `q`, `?`, `Tab`, `Enter`, `Esc` and arrows do not.
    #[must_use]
    pub fn passes_modal(&self) -> bool {
        self.mods
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            || matches!(self.code, KeyCode::F(_))
    }

    /// The chord a terminal event carries.
    #[must_use]
    pub fn from_event(event: KeyEvent) -> Self {
        Self::new(event.code, event.modifiers)
    }

    /// The event this chord would arrive as: the test harness feeds keys this way.
    #[must_use]
    pub fn to_event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.mods)
    }

    /// Parses a binding spec such as `"q"`, `"?"`, `"Enter"`, `"shift-tab"`, `"ctrl-c"`, `"f5"`.
    ///
    /// Modifier names are `ctrl`, `alt` and `shift`, joined to the key by `-` or `+`, in any case.
    /// Returns `None` for an unknown modifier or key name, never panics.
    #[must_use]
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        let mut chars = spec.chars();
        match (chars.next(), chars.next()) {
            (None, _) => return None,
            // A one-character spec is that character, even when it is `-` or `+`.
            (Some(c), None) => return Some(Self::new(KeyCode::Char(c), KeyModifiers::NONE)),
            _ => {}
        }
        let parts: Vec<&str> = spec.split(['-', '+']).filter(|p| !p.is_empty()).collect();
        let (key, modifiers) = parts.split_last()?;
        let mut mods = KeyModifiers::NONE;
        for name in modifiers {
            match name.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
                "alt" | "meta" => mods |= KeyModifiers::ALT,
                "shift" => mods |= KeyModifiers::SHIFT,
                _ => return None,
            }
        }
        Some(Self::new(key_code(key)?, mods))
    }

    /// Help-line rendering of the chord: `q`, `Shift+Tab`, `Ctrl+c`, `Esc`.
    #[must_use]
    pub fn label(&self) -> String {
        let mut out = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            out.push_str("Ctrl+");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            out.push_str("Alt+");
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            out.push_str("Shift+");
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("Space"),
            KeyCode::Char(c) => out.push(c),
            KeyCode::Enter => out.push_str("Enter"),
            KeyCode::Esc => out.push_str("Esc"),
            KeyCode::Tab => out.push_str("Tab"),
            KeyCode::BackTab => out.push_str("Shift+Tab"),
            KeyCode::Backspace => out.push_str("Backspace"),
            KeyCode::Delete => out.push_str("Del"),
            KeyCode::Insert => out.push_str("Ins"),
            KeyCode::Home => out.push_str("Home"),
            KeyCode::End => out.push_str("End"),
            KeyCode::PageUp => out.push_str("PgUp"),
            KeyCode::PageDown => out.push_str("PgDn"),
            KeyCode::Up => out.push_str("Up"),
            KeyCode::Down => out.push_str("Down"),
            KeyCode::Left => out.push_str("Left"),
            KeyCode::Right => out.push_str("Right"),
            KeyCode::F(n) => out.push_str(&format!("F{n}")),
            other => out.push_str(&format!("{other:?}")),
        }
        out
    }
}

/// `ctrl-c`: always quits (ANA-26 §6.4). `App::on_key` checks it before any overlay or view, and
/// no catalogue action may bind it.
pub const CTRL_C: KeyChord = KeyChord {
    code: KeyCode::Char('c'),
    mods: KeyModifiers::CONTROL,
};

impl KeyChord {
    /// Parses a spec from a key file (ANA-26 §7.1, MOD-67 D11): `parse`'s grammar, made strict.
    ///
    /// Parts are trimmed, the key is split off at the last `-`/`+` that is not the final
    /// character (so `ctrl--` is `ctrl` + `-`), and the only modifiers are `ctrl`, `alt` and
    /// `shift`. `shift-` with a letter is refused with the capital to write, `shift-` with another
    /// character is refused outright, `ctrl-` with a capital (or with `shift-` and a letter) is
    /// refused with the lower case, and a ctrl chord the terminal delivers as another key
    /// (`ctrl-i` is `Tab`) with what arrives; that last refusal also covers `ctrl-I`,
    /// `ctrl-shift-i` and their `m` twins, ahead of the capital one. Last, on Unix only, a chord
    /// the terminal never delivers as itself is refused (MOD-67 M2 D6; see `unix_drops`).
    /// `ctrl-c` is not refused here: that is the loader's rule.
    ///
    /// # Errors
    ///
    /// A [`ChordError`] naming the first rule the spec breaks.
    pub fn parse_strict(spec: &str) -> Result<Self, ChordError> {
        let written = spec.trim();
        let Some((last, _)) = written.char_indices().next_back() else {
            return Err(ChordError::Empty);
        };
        // Split at the last separator that is not the final character; a one-character spec has
        // `last == 0` and so no modifier part.
        let (modifier_part, key_part) = match written[..last].rfind(['-', '+']) {
            Some(i) => (Some(&written[..i]), written[i + 1..].trim()),
            None => (None, written),
        };
        let mut mods = KeyModifiers::NONE;
        for piece in modifier_part
            .into_iter()
            .flat_map(|part| part.split(['-', '+']))
        {
            let piece = piece.trim();
            let (flag, name) = match piece.to_ascii_lowercase().as_str() {
                "ctrl" => (KeyModifiers::CONTROL, "ctrl"),
                "alt" => (KeyModifiers::ALT, "alt"),
                "shift" => (KeyModifiers::SHIFT, "shift"),
                _ => return Err(ChordError::UnknownModifier(piece.to_owned())),
            };
            if mods.contains(flag) {
                return Err(ChordError::DuplicateModifier(name.to_owned()));
            }
            mods |= flag;
        }
        let code = key_code(key_part).ok_or_else(|| ChordError::UnknownKey(key_part.to_owned()))?;
        if let KeyCode::Char(c) = code {
            refuse_char(written, c, mods)?;
        }
        let chord = Self::new(code, mods);
        // `cfg!`, not `#[cfg]`: `unix_drops` stays used (and tested) on every target.
        if cfg!(unix) && unix_drops(chord) {
            return Err(ChordError::NotDelivered {
                written: written.to_owned(),
            });
        }
        Ok(chord)
    }

    /// The canonical spelling `parse_strict` reads back: modifiers in the order `ctrl-`, `alt-`,
    /// `shift-` (`shift-` only on a named key), then the key: the character itself, `space`, or a
    /// lower-case name (`enter`, `esc`, `tab`, `backtab`, `backspace`, `delete`, `insert`, `home`,
    /// `end`, `pgup`, `pgdn`, `up`, `down`, `left`, `right`, `f1`..`f12`).
    /// Error suggestions use it, and M2's `--print-keys` will too.
    #[must_use]
    pub fn spec(&self) -> String {
        let mut out = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            out.push_str("ctrl-");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            out.push_str("alt-");
        }
        if self.mods.contains(KeyModifiers::SHIFT) && !matches!(self.code, KeyCode::Char(_)) {
            out.push_str("shift-");
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("space"),
            KeyCode::Char(c) => out.push(c),
            KeyCode::Enter => out.push_str("enter"),
            KeyCode::Esc => out.push_str("esc"),
            KeyCode::Tab => out.push_str("tab"),
            KeyCode::BackTab => out.push_str("backtab"),
            KeyCode::Backspace => out.push_str("backspace"),
            KeyCode::Delete => out.push_str("delete"),
            KeyCode::Insert => out.push_str("insert"),
            KeyCode::Home => out.push_str("home"),
            KeyCode::End => out.push_str("end"),
            KeyCode::PageUp => out.push_str("pgup"),
            KeyCode::PageDown => out.push_str("pgdn"),
            KeyCode::Up => out.push_str("up"),
            KeyCode::Down => out.push_str("down"),
            KeyCode::Left => out.push_str("left"),
            KeyCode::Right => out.push_str("right"),
            KeyCode::F(n) => out.push_str(&format!("f{n}")),
            other => out.push_str(&format!("{other:?}").to_ascii_lowercase()),
        }
        out
    }

    /// Whether a text field would take this chord as typed text: a `Char` with neither CONTROL
    /// nor ALT (ANA-26 §6.4). `Space` is printable; `F1`, `Tab` and `ctrl-f` are not.
    #[must_use]
    pub fn is_printable(&self) -> bool {
        matches!(self.code, KeyCode::Char(_))
            && !self
                .mods
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    }
}

/// `parse_strict`'s refusals for a character key, in D11's order: a shift, a ctrl capital, then a
/// ctrl chord the legacy encoding delivers as another key.
///
/// One exception runs first: `ctrl-I`, `ctrl-shift-i`, `ctrl-M` and `ctrl-shift-m` are refused
/// by what arrives (`Tab`, `Enter`), since suggesting `ctrl-i` or `ctrl-m` would name a chord
/// this function refuses too.
fn refuse_char(written: &str, c: char, mods: KeyModifiers) -> Result<(), ChordError> {
    let spelled = |mods: KeyModifiers, c: char| {
        KeyChord {
            code: KeyCode::Char(c),
            mods,
        }
        .spec()
    };
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let lowered = c.to_ascii_lowercase();
    if ctrl
        && matches!(lowered, 'i' | 'm')
        && let Some(arrives_as) = legacy_arrival(lowered, mods.difference(KeyModifiers::SHIFT))
    {
        return Err(ChordError::Indistinguishable {
            written: written.to_owned(),
            arrives_as,
        });
    }
    if mods.contains(KeyModifiers::SHIFT) {
        let unshifted = mods.difference(KeyModifiers::SHIFT);
        return Err(if ctrl && c.is_ascii_alphabetic() {
            ChordError::CtrlCapital {
                suggestion: spelled(unshifted, c.to_ascii_lowercase()),
            }
        } else if c.is_ascii_alphabetic() {
            ChordError::ShiftedLetter {
                suggestion: spelled(unshifted, c.to_ascii_uppercase()),
            }
        } else {
            ChordError::ShiftedCharacter {
                written: written.to_owned(),
            }
        });
    }
    if !ctrl {
        return Ok(());
    }
    if c.is_ascii_uppercase() {
        return Err(ChordError::CtrlCapital {
            suggestion: spelled(mods, c.to_ascii_lowercase()),
        });
    }
    match legacy_arrival(c, mods) {
        Some(arrives_as) => Err(ChordError::Indistinguishable {
            written: written.to_owned(),
            arrives_as,
        }),
        None => Ok(()),
    }
}

/// What a ctrl chord of `c` arrives as when its legacy encoding is another key's byte, or `None`
/// when it arrives as itself. `mods` holds CONTROL and no SHIFT.
///
/// crossterm 0.29 `event/sys/unix/parse.rs:92-118`: `0x00` is ctrl-space (so `ctrl-2` and
/// `ctrl-@`), `0x1B` is Esc (`ctrl-3`, `ctrl-[`), `0x1C..=0x1F` are ctrl-4..7 (`ctrl-\`,
/// `ctrl-]`, `ctrl-^`, `ctrl-_`, `ctrl-/`) and `0x7F` is Backspace (`ctrl-8`, `ctrl-?`).
/// `ctrl-h` and `ctrl-j` arrive as themselves in raw mode (D11).
fn legacy_arrival(c: char, mods: KeyModifiers) -> Option<KeyChord> {
    let without_ctrl = mods.difference(KeyModifiers::CONTROL);
    Some(match c {
        'i' => KeyChord::new(KeyCode::Tab, without_ctrl),
        'm' => KeyChord::new(KeyCode::Enter, without_ctrl),
        '[' => KeyChord::new(KeyCode::Esc, without_ctrl),
        '\\' => KeyChord::new(KeyCode::Char('4'), mods),
        ']' => KeyChord::new(KeyCode::Char('5'), mods),
        '^' => KeyChord::new(KeyCode::Char('6'), mods),
        '_' => KeyChord::new(KeyCode::Char('7'), mods),
        '2' | '@' => KeyChord::new(KeyCode::Char(' '), mods),
        '3' => KeyChord::new(KeyCode::Esc, without_ctrl),
        '8' | '?' => KeyChord::new(KeyCode::Backspace, without_ctrl),
        '/' => KeyChord::new(KeyCode::Char('7'), mods),
        _ => return None,
    })
}

/// Whether a Unix terminal in the legacy encoding never delivers `chord` as itself (MOD-67 M2
/// D6): ctrl with a character other than `a`-`z`, space and `4`-`7` arrives as another key or
/// not at all, and ctrl/shift on `Enter`, `Tab`, `Backspace` or `Esc` arrive without that
/// modifier (`shift-tab` is `BackTab` and stays deliverable; ctrl-backtab is not). htui never
/// pushes keyboard-enhancement flags, so nothing restores them. Windows delivers all of these.
fn unix_drops(chord: KeyChord) -> bool {
    let ctrl = chord.mods.contains(KeyModifiers::CONTROL);
    match chord.code {
        KeyCode::Char(c) => {
            ctrl && !(c.is_ascii_lowercase() || c == ' ' || ('4'..='7').contains(&c))
        }
        KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::Esc => chord
            .mods
            .intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        KeyCode::BackTab => ctrl,
        _ => false,
    }
}

/// Why a spec is not a chord a terminal can deliver (ANA-26 §7.1, §7.5; `NotDelivered`, MOD-67
/// M2 D6, only on Unix). Every message is the tail of a `keys.toml:LINE: [context] name =
/// "spec": …` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    /// Nothing but whitespace.
    Empty,
    /// A modifier other than `ctrl`, `alt` or `shift`; carries the part as written.
    UnknownModifier(String),
    /// The same modifier twice; carries its lower-case name.
    DuplicateModifier(String),
    /// Not one character and not a key name; carries the key part as written.
    UnknownKey(String),
    /// `shift-` with a letter (and no `ctrl-`): a terminal sends the capital.
    ShiftedLetter {
        /// The spec to write instead, e.g. `A` or `alt-X`.
        suggestion: String,
    },
    /// `shift-` with a non-letter character (`shift-1`): only the character it types arrives,
    /// and that depends on the keyboard layout.
    ShiftedCharacter {
        /// The spec as written (trimmed).
        written: String,
    },
    /// `ctrl-` with a capital, or with `shift-` and a letter: a terminal sends ctrl chords in
    /// lower case.
    CtrlCapital {
        /// The spec to write instead, e.g. `ctrl-c`.
        suggestion: String,
    },
    /// A ctrl chord the legacy encoding delivers as another key (crossterm 0.29,
    /// `event/sys/unix/parse.rs:92-116`).
    Indistinguishable {
        /// The spec as written (trimmed).
        written: String,
        /// What actually arrives.
        arrives_as: KeyChord,
    },
    /// A chord a Unix terminal in the legacy encoding never delivers as itself (MOD-67 M2 D6):
    /// ctrl with a character other than `a`-`z`, space and `4`-`7`, or ctrl/shift on `Enter`,
    /// `Tab`, `Backspace` or `Esc`. Produced only on Unix; Windows delivers these chords.
    NotDelivered {
        /// The spec as written (trimmed).
        written: String,
    },
}

impl std::fmt::Display for ChordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("an empty chord"),
            Self::UnknownModifier(m) => {
                write!(f, r#""{m}" is not a modifier: write ctrl, alt or shift"#)
            }
            Self::DuplicateModifier(m) => write!(f, r#""{m}" is written twice"#),
            Self::UnknownKey(k) => write!(
                f,
                r#""{k}" is not a key: write one character or a key name such as "enter" or "f5""#
            ),
            Self::ShiftedLetter { suggestion } => {
                write!(f, r#"write a shifted letter as "{suggestion}""#)
            }
            Self::ShiftedCharacter { written } => write!(
                f,
                r#""{written}": write the character the shifted key types instead"#
            ),
            Self::CtrlCapital { suggestion } => write!(
                f,
                r#"write "{suggestion}": a terminal sends ctrl with the lower-case letter"#
            ),
            Self::Indistinguishable {
                written,
                arrives_as,
            } => {
                let label = arrives_as.label();
                write!(
                    f,
                    r#""{written}" arrives as {label}: a terminal cannot tell the two apart"#
                )
            }
            Self::NotDelivered { written } => write!(
                f,
                r#""{written}" never reaches htui: a Unix terminal sends it as another key or not at all"#
            ),
        }
    }
}

impl std::error::Error for ChordError {}

/// The name of a key, as written in a binding spec.
fn key_code(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(c));
    }
    let lower = name.to_ascii_lowercase();
    if let Some(digits) = lower.strip_prefix('f')
        && let Ok(n) = digits.parse::<u8>()
        && (1..=12).contains(&n)
    {
        return Some(KeyCode::F(n));
    }
    Some(match lower.as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(spec: &str) -> KeyChord {
        KeyChord::parse(spec).expect("test specs parse")
    }

    fn strict(spec: &str) -> Result<KeyChord, ChordError> {
        KeyChord::parse_strict(spec)
    }

    #[test]
    fn parse_reads_plain_keys_named_keys_and_modifiers() {
        assert_eq!(
            chord("q"),
            KeyChord::new(KeyCode::Char('q'), KeyModifiers::NONE)
        );
        assert_eq!(
            chord("?"),
            KeyChord::new(KeyCode::Char('?'), KeyModifiers::NONE)
        );
        assert_eq!(
            chord("Enter"),
            KeyChord::new(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(
            chord("ctrl-c"),
            KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            chord("f5"),
            KeyChord::new(KeyCode::F(5), KeyModifiers::NONE)
        );
        assert_eq!(
            chord("-"),
            KeyChord::new(KeyCode::Char('-'), KeyModifiers::NONE)
        );
        assert_eq!(KeyChord::parse(""), None);
        assert_eq!(KeyChord::parse("hyper-x"), None);
        assert_eq!(KeyChord::parse("wat"), None);
    }

    #[test]
    fn shift_tab_normalises_to_backtab_however_it_is_written() {
        let parsed = chord("shift-tab");
        assert_eq!(parsed.code, KeyCode::BackTab);
        assert!(!parsed.mods.contains(KeyModifiers::SHIFT));
        assert_eq!(parsed, chord("backtab"));
        assert_eq!(
            parsed,
            KeyChord::from_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
        );
        assert_eq!(parsed.label(), "Shift+Tab");
    }

    #[test]
    fn a_shifted_character_keeps_the_character_and_drops_the_flag() {
        let typed = KeyChord::from_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT));
        assert_eq!(typed, chord("?"));
    }

    /// Blueprint §2.5 test 4's canonical specs, which both parsers must read alike.
    const CANONICAL: [&str; 25] = [
        "q",
        "?",
        "enter",
        "esc",
        "tab",
        "backtab",
        "shift-tab",
        "space",
        "G",
        "f1",
        "f5",
        "pgdn",
        "pgup",
        "home",
        "end",
        "down",
        "up",
        "left",
        "right",
        "ctrl-f",
        "ctrl-w",
        "ctrl-s",
        "ctrl-e",
        "ctrl-c",
        "-",
    ];

    #[test]
    fn strict_reads_every_canonical_spec_as_parse_does() {
        for spec in CANONICAL {
            assert_eq!(strict(spec), Ok(chord(spec)), "{spec}");
        }
    }

    #[test]
    fn parts_are_trimmed() {
        for spec in [" ctrl - c ", "ctrl -c", "ctrl- c"] {
            assert_eq!(strict(spec), Ok(chord("ctrl-c")), "{spec:?}");
        }
    }

    /// The grammar reads `ctrl--`, `ctrl++` and `ctrl-+`: the key is split off at the last
    /// separator that is not the final character. A Unix terminal never sends ctrl with `-` or
    /// `+` (MOD-67 M2 D6), so there they reach `NotDelivered`, which proves the parse got past
    /// the grammar; elsewhere they are the ctrl chords.
    #[test]
    fn ctrl_minus_and_ctrl_plus_are_writable() {
        for (spec, written, c) in [
            ("ctrl--", "ctrl--", '-'),
            ("ctrl++", "ctrl++", '+'),
            ("ctrl-+", "ctrl-+", '+'),
            (" ctrl-+ ", "ctrl-+", '+'),
        ] {
            let expected = if cfg!(unix) {
                Err(ChordError::NotDelivered {
                    written: written.to_owned(),
                })
            } else {
                Ok(KeyChord::new(KeyCode::Char(c), KeyModifiers::CONTROL))
            };
            assert_eq!(strict(spec), expected, "{spec:?}");
        }
        assert_eq!(
            strict("-"),
            Ok(KeyChord::new(KeyCode::Char('-'), KeyModifiers::NONE))
        );
    }

    #[test]
    fn shift_tab_is_backtab() {
        let parsed = strict("shift-tab").expect("shift-tab parses");
        assert_eq!(parsed.code, KeyCode::BackTab);
        assert!(!parsed.mods.contains(KeyModifiers::SHIFT));
        assert_eq!(Ok(parsed), strict("backtab"));
    }

    #[test]
    fn a_shifted_letter_is_refused_with_its_capital() {
        let refused = ChordError::ShiftedLetter {
            suggestion: "A".to_owned(),
        };
        assert_eq!(strict("shift-a"), Err(refused.clone()));
        assert_eq!(refused.to_string(), r#"write a shifted letter as "A""#);
        assert_eq!(strict("shift-A"), Err(refused));
        assert_eq!(
            strict("alt-shift-x"),
            Err(ChordError::ShiftedLetter {
                suggestion: "alt-X".to_owned()
            })
        );
    }

    #[test]
    fn ctrl_with_a_capital_is_refused_with_the_lower_case() {
        let refused = ChordError::CtrlCapital {
            suggestion: "ctrl-c".to_owned(),
        };
        assert_eq!(strict("ctrl-C"), Err(refused.clone()));
        assert_eq!(
            refused.to_string(),
            r#"write "ctrl-c": a terminal sends ctrl with the lower-case letter"#
        );
        assert_eq!(strict("ctrl-shift-c"), Err(refused));
    }

    #[test]
    fn chords_the_terminal_cannot_tell_apart_are_refused_by_what_arrives() {
        let table = [
            ("ctrl-i", "Tab"),
            ("ctrl-m", "Enter"),
            ("ctrl-[", "Esc"),
            ("ctrl-\\", "Ctrl+4"),
            ("ctrl-]", "Ctrl+5"),
            ("ctrl-^", "Ctrl+6"),
            ("ctrl-_", "Ctrl+7"),
        ];
        for (spec, label) in table {
            match strict(spec) {
                Err(ChordError::Indistinguishable {
                    written,
                    arrives_as,
                }) => {
                    assert_eq!(written, spec);
                    assert_eq!(arrives_as.label(), label, "{spec}");
                }
                other => panic!("{spec}: expected Indistinguishable, got {other:?}"),
            }
        }
        assert_eq!(
            strict("ctrl-i").expect_err("ctrl-i is refused").to_string(),
            r#""ctrl-i" arrives as Tab: a terminal cannot tell the two apart"#
        );
    }

    #[test]
    fn digits_and_symbols_with_a_legacy_byte_are_refused_by_what_arrives() {
        let table = [
            ("ctrl-2", "Ctrl+Space"),
            ("ctrl-@", "Ctrl+Space"),
            ("ctrl-3", "Esc"),
            ("ctrl-8", "Backspace"),
            ("ctrl-?", "Backspace"),
            ("ctrl-/", "Ctrl+7"),
        ];
        for (spec, label) in table {
            match strict(spec) {
                Err(ChordError::Indistinguishable {
                    written,
                    arrives_as,
                }) => {
                    assert_eq!(written, spec);
                    assert_eq!(arrives_as.label(), label, "{spec}");
                }
                other => panic!("{spec}: expected Indistinguishable, got {other:?}"),
            }
        }
        assert_eq!(
            strict("ctrl-2").expect_err("ctrl-2 is refused").to_string(),
            r#""ctrl-2" arrives as Ctrl+Space: a terminal cannot tell the two apart"#
        );
    }

    #[test]
    fn unix_drops_ctrl_characters_outside_letters_space_and_4_to_7() {
        let ctrl = KeyModifiers::CONTROL;
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        let dropped = ['1', '0', '9', '.', ',', ';', '\'', '=', '-', '+', '\u{e9}']
            .map(|c| (c, ctrl))
            .into_iter()
            .chain([('1', ctrl_alt)]);
        for (c, mods) in dropped {
            let chord = KeyChord::new(KeyCode::Char(c), mods);
            assert!(unix_drops(chord), "{chord:?}");
        }
        let delivered = ['a', 'z', 'h', 'j', ' ', '4', '5', '6', '7']
            .map(|c| (c, ctrl))
            .into_iter()
            .chain([
                ('x', ctrl_alt),
                ('1', KeyModifiers::ALT),
                ('1', KeyModifiers::NONE),
                ('q', KeyModifiers::NONE),
            ]);
        for (c, mods) in delivered {
            let chord = KeyChord::new(KeyCode::Char(c), mods);
            assert!(!unix_drops(chord), "{chord:?}");
        }
    }

    #[test]
    fn unix_drops_ctrl_and_shift_on_enter_tab_backspace_and_esc() {
        let ctrl = KeyModifiers::CONTROL;
        let shift = KeyModifiers::SHIFT;
        let alt = KeyModifiers::ALT;
        for (code, mods) in [
            (KeyCode::Enter, ctrl),
            (KeyCode::Enter, shift),
            (KeyCode::Enter, alt | shift),
            (KeyCode::Tab, ctrl),
            // ctrl-backtab
            (KeyCode::Tab, ctrl | shift),
            (KeyCode::Backspace, ctrl),
            (KeyCode::Backspace, shift),
            (KeyCode::Esc, ctrl),
            (KeyCode::Esc, shift),
        ] {
            let chord = KeyChord::new(code, mods);
            assert!(unix_drops(chord), "{chord:?}");
        }
        for (code, mods) in [
            (KeyCode::Enter, KeyModifiers::NONE),
            (KeyCode::Tab, KeyModifiers::NONE),
            (KeyCode::Backspace, KeyModifiers::NONE),
            (KeyCode::Esc, KeyModifiers::NONE),
            // backtab
            (KeyCode::Tab, shift),
            (KeyCode::Enter, alt),
            (KeyCode::Backspace, alt),
            (KeyCode::Up, shift),
            (KeyCode::Up, ctrl),
            (KeyCode::F(5), shift),
            (KeyCode::Delete, ctrl),
        ] {
            let chord = KeyChord::new(code, mods);
            assert!(!unix_drops(chord), "{chord:?}");
        }
    }

    /// The specs a Unix terminal never sends as themselves, with the trimmed spec each reports.
    const NOT_DELIVERED: [(&str, &str); 7] = [
        ("ctrl-1", "ctrl-1"),
        ("shift-enter", "shift-enter"),
        ("ctrl-tab", "ctrl-tab"),
        ("ctrl-shift-tab", "ctrl-shift-tab"),
        ("ctrl-backspace", "ctrl-backspace"),
        ("shift-esc", "shift-esc"),
        (" ctrl - . ", "ctrl - ."),
    ];

    #[cfg(unix)]
    #[test]
    fn a_chord_a_unix_terminal_never_sends_is_refused_there() {
        for (spec, written) in NOT_DELIVERED {
            assert_eq!(
                strict(spec),
                Err(ChordError::NotDelivered {
                    written: written.to_owned()
                }),
                "{spec:?}"
            );
        }
        assert_eq!(
            strict("ctrl-1").expect_err("ctrl-1 is refused").to_string(),
            r#""ctrl-1" never reaches htui: a Unix terminal sends it as another key or not at all"#
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn a_chord_a_unix_terminal_never_sends_parses_elsewhere() {
        for (spec, _) in NOT_DELIVERED {
            assert!(strict(spec).is_ok(), "{spec:?}");
        }
        assert_eq!(
            strict("shift-enter"),
            Ok(KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT))
        );
    }

    #[test]
    fn the_older_refusals_still_win_over_the_unix_rule() {
        assert_eq!(
            strict("ctrl-A"),
            Err(ChordError::CtrlCapital {
                suggestion: "ctrl-a".to_owned()
            })
        );
        for spec in ["shift-1", "ctrl-shift-1"] {
            assert_eq!(
                strict(spec),
                Err(ChordError::ShiftedCharacter {
                    written: spec.to_owned()
                }),
                "{spec}"
            );
        }
        match strict("ctrl-i") {
            Err(ChordError::Indistinguishable { arrives_as, .. }) => {
                assert_eq!(arrives_as.label(), "Tab");
            }
            other => panic!("ctrl-i: expected Indistinguishable, got {other:?}"),
        }
    }

    #[test]
    fn a_capital_or_shifted_ctrl_i_or_m_is_refused_by_what_arrives() {
        for (spec, label) in [
            ("ctrl-I", "Tab"),
            ("ctrl-shift-i", "Tab"),
            ("ctrl-M", "Enter"),
            ("ctrl-shift-m", "Enter"),
        ] {
            let refused = strict(spec).expect_err("the spec is refused");
            assert_eq!(
                refused.to_string(),
                format!(r#""{spec}" arrives as {label}: a terminal cannot tell the two apart"#),
                "{spec}"
            );
            match refused {
                ChordError::Indistinguishable {
                    written,
                    arrives_as,
                } => {
                    assert_eq!(written, spec);
                    assert_eq!(arrives_as.label(), label, "{spec}");
                }
                other => panic!("{spec}: expected Indistinguishable, got {other:?}"),
            }
        }
    }

    #[test]
    fn ctrl_h_and_ctrl_j_are_accepted() {
        for c in ['h', 'j'] {
            assert_eq!(
                strict(&format!("ctrl-{c}")),
                Ok(KeyChord::new(KeyCode::Char(c), KeyModifiers::CONTROL))
            );
        }
    }

    #[test]
    fn unknown_empty_and_repeated_parts_are_refused() {
        assert_eq!(strict(""), Err(ChordError::Empty));
        assert_eq!(ChordError::Empty.to_string(), "an empty chord");
        assert_eq!(strict("   "), Err(ChordError::Empty));
        assert_eq!(
            strict("hyper-x"),
            Err(ChordError::UnknownModifier("hyper".to_owned()))
        );
        assert_eq!(
            strict("control-x"),
            Err(ChordError::UnknownModifier("control".to_owned()))
        );
        assert_eq!(
            strict("ctrl-wat"),
            Err(ChordError::UnknownKey("wat".to_owned()))
        );
        assert_eq!(
            strict("ctrl-ctrl-x"),
            Err(ChordError::DuplicateModifier("ctrl".to_owned()))
        );
        assert_eq!(
            strict("-x"),
            Err(ChordError::UnknownModifier(String::new()))
        );
        // The messages M2 prints after `keys.toml:LINE: [context] name = "spec": `.
        for (spec, message) in [
            (
                "hyper-x",
                r#""hyper" is not a modifier: write ctrl, alt or shift"#,
            ),
            ("ctrl-ctrl-x", r#""ctrl" is written twice"#),
            (
                "ctrl-wat",
                r#""wat" is not a key: write one character or a key name such as "enter" or "f5""#,
            ),
        ] {
            let refused = strict(spec).expect_err("the spec is refused");
            assert_eq!(refused.to_string(), message, "{spec}");
        }
    }

    #[test]
    fn shift_on_a_non_letter_is_refused() {
        assert_eq!(
            strict("shift-1"),
            Err(ChordError::ShiftedCharacter {
                written: "shift-1".to_owned()
            })
        );
        assert_eq!(
            strict("shift-1")
                .expect_err("shift-1 is refused")
                .to_string(),
            r#""shift-1": write the character the shifted key types instead"#
        );
    }

    #[test]
    fn a_shifted_named_key_keeps_its_shift() {
        let parsed = strict("shift-up").expect("shift-up parses");
        assert_eq!(parsed.mods, KeyModifiers::SHIFT);
        assert_eq!(parsed.label(), "Shift+Up");
    }

    #[test]
    fn the_ctrl_c_constant_is_the_parsed_chord() {
        assert_eq!(CTRL_C, chord("ctrl-c"));
        assert_eq!(Ok(CTRL_C), strict("ctrl-c"));
    }

    #[test]
    fn spec_round_trips_through_the_strict_parser() {
        for spec in CANONICAL.into_iter().chain(["alt-x", "shift-up", "f12"]) {
            let parsed = strict(spec).expect("canonical specs parse");
            assert_eq!(strict(&parsed.spec()), Ok(parsed), "{spec}");
        }
    }

    /// MOD-67 M3 PA-6: a kitty-protocol terminal reports ctrl-shift-d as `D` with CONTROL; the
    /// chord is `ctrl-d`. The key file still cannot write `ctrl-D`.
    #[test]
    fn ctrl_with_a_capital_is_the_lower_case_chord() {
        let event = KeyEvent::new(KeyCode::Char('D'), KeyModifiers::CONTROL);
        assert_eq!(KeyChord::from_event(event), chord("ctrl-d"));
        assert_eq!(
            KeyChord::new(
                KeyCode::Char('X'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            ),
            chord("ctrl-alt-x")
        );
        assert_eq!(
            KeyChord::new(KeyCode::Char('X'), KeyModifiers::ALT),
            KeyChord {
                code: KeyCode::Char('X'),
                mods: KeyModifiers::ALT
            }
        );
        assert_eq!(
            strict("ctrl-D"),
            Err(ChordError::CtrlCapital {
                suggestion: "ctrl-d".to_owned()
            })
        );
    }

    #[test]
    fn passes_modal_is_ctrl_alt_or_a_function_key() {
        for spec in ["ctrl-c", "ctrl-f", "alt-x", "f1", "shift-f5"] {
            assert!(chord(spec).passes_modal(), "{spec}");
        }
        for spec in [
            "q", "?", "tab", "backtab", "enter", "esc", "down", "shift-up",
        ] {
            assert!(!chord(spec).passes_modal(), "{spec}");
        }
    }

    #[test]
    fn printable_means_a_character_without_ctrl_or_alt() {
        for spec in ["q", "?", "space", "G"] {
            assert!(chord(spec).is_printable(), "{spec}");
        }
        for spec in ["ctrl-q", "alt-q", "f1", "tab", "esc"] {
            assert!(!chord(spec).is_printable(), "{spec}");
        }
    }
}
