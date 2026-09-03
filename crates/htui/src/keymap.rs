//! Key bindings as data (plan D5).
//!
//! A binding is a `(scope, chord) -> Action` row in a table, so MOD-13's `n`/`e` and MOD-4's
//! approve/reject are `Keymap::bind` calls rather than `match` arms in the event loop. Resolution
//! is scoped: the overlay on top wins over the active tab, which wins over the global table
//! (blueprint C.4, C.6).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::action::{Action, OverlayAction, TabAction};
use crate::ui::overlay::OverlayId;
use crate::ui::tabs::TabId;

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

/// Where a binding applies. The propagation chain asks the scopes in order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyScope {
    /// Always live, checked last.
    Global,
    /// Live while this tab is active.
    Tab(TabId),
    /// Live while this overlay is on top. [`OverlayId::ANY`] matches every overlay.
    Overlay(OverlayId),
}

/// One row of the key table.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Where it applies.
    pub scope: KeyScope,
    /// What was pressed.
    pub key: KeyChord,
    /// What it does.
    pub action: Action,
    /// Help-line text, e.g. `"quit"`.
    pub help: &'static str,
}

/// The key table. Later bindings shadow earlier ones for the same scope and chord, so a module
/// can rebind a key without removing a row.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The MOD-1 table: `q`, `Tab`, `Shift+Tab`, `1`..`9`, `?`, and `Esc` for every overlay.
    ///
    /// T6 adds global `w` (open the workspace switcher) once T5's overlay exists.
    #[must_use]
    pub fn default_global() -> Self {
        let mut map = Self::new();
        map.bind(Binding {
            scope: KeyScope::Global,
            key: KeyChord::new(KeyCode::Char('q'), KeyModifiers::NONE),
            action: Action::Quit,
            help: "quit",
        });
        map.bind(Binding {
            scope: KeyScope::Global,
            key: KeyChord::new(KeyCode::Tab, KeyModifiers::NONE),
            action: Action::Tab(TabAction::Next),
            help: "next tab",
        });
        map.bind(Binding {
            scope: KeyScope::Global,
            key: KeyChord::new(KeyCode::BackTab, KeyModifiers::NONE),
            action: Action::Tab(TabAction::Prev),
            help: "previous tab",
        });
        for (index, digit) in ('1'..='9').enumerate() {
            map.bind(Binding {
                scope: KeyScope::Global,
                key: KeyChord::new(KeyCode::Char(digit), KeyModifiers::NONE),
                action: Action::Tab(TabAction::Select(index)),
                help: "select tab",
            });
        }
        map.bind(Binding {
            scope: KeyScope::Global,
            key: KeyChord::new(KeyCode::Char('?'), KeyModifiers::NONE),
            action: Action::ToggleHelp,
            help: "help",
        });
        map.bind(Binding {
            scope: KeyScope::Overlay(OverlayId::ANY),
            key: KeyChord::new(KeyCode::Esc, KeyModifiers::NONE),
            action: Action::Overlay(OverlayAction::Close),
            help: "close",
        });
        map
    }

    /// Adds a binding. It shadows any earlier binding of the same scope and chord.
    pub fn bind(&mut self, binding: Binding) {
        self.bindings.push(binding);
    }

    /// The action bound to this chord in this scope, or `None`.
    ///
    /// A [`KeyScope::Overlay`] falls back to [`OverlayId::ANY`], which is how `Esc` closes an
    /// overlay that never registered a binding of its own.
    #[must_use]
    pub fn resolve(&self, scope: &KeyScope, chord: KeyChord) -> Option<&Action> {
        if let Some(action) = self.lookup(scope, chord) {
            return Some(action);
        }
        match scope {
            KeyScope::Overlay(id) if *id != OverlayId::ANY => {
                self.lookup(&KeyScope::Overlay(OverlayId::ANY), chord)
            }
            _ => None,
        }
    }

    /// Exact scope lookup, newest binding first.
    fn lookup(&self, scope: &KeyScope, chord: KeyChord) -> Option<&Action> {
        self.bindings
            .iter()
            .rev()
            .find(|b| b.scope == *scope && b.key == chord)
            .map(|b| &b.action)
    }

    /// Every binding of a scope, newest first, deduplicated by chord.
    #[must_use]
    pub fn bindings(&self, scope: &KeyScope) -> Vec<&Binding> {
        let mut seen: Vec<KeyChord> = Vec::new();
        let mut out: Vec<&Binding> = Vec::new();
        for binding in self.bindings.iter().rev() {
            if binding.scope == *scope && !seen.contains(&binding.key) {
                seen.push(binding.key);
                out.push(binding);
            }
        }
        out.reverse();
        out
    }

    /// The status-line summary of a scope: `q quit · Tab next tab · ? help`.
    ///
    /// Bindings that share a help text (the nine `1`..`9` rows) are collapsed to their first row,
    /// so the line stays one line.
    #[must_use]
    pub fn help_line(&self, scope: &KeyScope) -> String {
        let mut seen_help: Vec<&str> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        for binding in self.bindings(scope) {
            if seen_help.contains(&binding.help) {
                continue;
            }
            seen_help.push(binding.help);
            parts.push(format!("{} {}", binding.key.label(), binding.help));
        }
        parts.join(" · ")
    }
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

    #[test]
    fn the_global_table_binds_quit_tabs_digits_and_help() {
        let map = Keymap::default_global();
        assert!(matches!(
            map.resolve(&KeyScope::Global, chord("q")),
            Some(Action::Quit)
        ));
        assert!(matches!(
            map.resolve(&KeyScope::Global, chord("tab")),
            Some(Action::Tab(TabAction::Next))
        ));
        assert!(matches!(
            map.resolve(&KeyScope::Global, chord("shift-tab")),
            Some(Action::Tab(TabAction::Prev))
        ));
        assert!(matches!(
            map.resolve(&KeyScope::Global, chord("3")),
            Some(Action::Tab(TabAction::Select(2)))
        ));
        assert!(matches!(
            map.resolve(&KeyScope::Global, chord("?")),
            Some(Action::ToggleHelp)
        ));
    }

    #[test]
    fn an_unknown_chord_and_a_foreign_scope_resolve_to_none() {
        let map = Keymap::default_global();
        assert!(map.resolve(&KeyScope::Global, chord("z")).is_none());
        assert!(map.resolve(&KeyScope::Global, chord("esc")).is_none());
        assert!(
            map.resolve(&KeyScope::Tab(TabId("backlog")), chord("q"))
                .is_none(),
            "a global binding never leaks into a tab scope"
        );
    }

    #[test]
    fn every_overlay_inherits_esc_and_can_shadow_it() {
        let switcher = OverlayId("workspace_switcher");
        let mut map = Keymap::default_global();
        assert!(matches!(
            map.resolve(&KeyScope::Overlay(switcher), chord("esc")),
            Some(Action::Overlay(OverlayAction::Close))
        ));
        map.bind(Binding {
            scope: KeyScope::Overlay(switcher),
            key: chord("esc"),
            action: Action::Overlay(OverlayAction::CloseAll),
            help: "close all",
        });
        assert!(
            matches!(
                map.resolve(&KeyScope::Overlay(switcher), chord("esc")),
                Some(Action::Overlay(OverlayAction::CloseAll))
            ),
            "the exact scope wins over the wildcard"
        );
        assert!(
            matches!(
                map.resolve(&KeyScope::Overlay(OverlayId("other")), chord("esc")),
                Some(Action::Overlay(OverlayAction::Close))
            ),
            "and another overlay still inherits the wildcard"
        );
    }

    #[test]
    fn the_newest_binding_of_a_scope_wins() {
        let mut map = Keymap::default_global();
        map.bind(Binding {
            scope: KeyScope::Global,
            key: chord("q"),
            action: Action::ToggleHelp,
            help: "help",
        });
        assert!(matches!(
            map.resolve(&KeyScope::Global, chord("q")),
            Some(Action::ToggleHelp)
        ));
    }

    #[test]
    fn the_help_line_collapses_the_digit_rows() {
        let line = Keymap::default_global().help_line(&KeyScope::Global);
        assert_eq!(
            line,
            "q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help"
        );
    }
}
