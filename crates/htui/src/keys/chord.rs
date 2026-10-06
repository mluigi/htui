//! One normalised keystroke, and the strict parser a key file is read with (ANA-26 §7.1).
//!
//! `KeyChord` moved here from `keymap.rs` (MOD-67 M1, plan D2); `crate::keymap::KeyChord`
//! re-exports it until M6. `parse` is the harness's lenient reader and is unchanged.
//! `parse_strict` is the key file's reader (ANA-26 §7.1).

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
    /// Two rules, both forced by how terminals report keys: `Shift+Tab` is `BackTab` without a
    /// shift flag, and a `Char` already carries its shift in the character itself (`J`, `?`), so
    /// the flag is dropped there.
    #[must_use]
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let (code, mut mods) = match code {
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => (KeyCode::BackTab, mods),
            other => (other, mods),
        };
        if matches!(code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        Self { code, mods }
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
}
