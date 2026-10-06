//! Key bindings as data (plan D5).
//!
//! A binding is a `(scope, chord) -> Action` row in a table, so MOD-13's `n`/`e` and MOD-4's
//! approve/reject are `Keymap::bind` calls rather than `match` arms in the event loop. Resolution
//! is scoped: the overlay on top wins over the active tab, which wins over the global table
//! (blueprint C.4, C.6). `KeyChord` lives in `crate::keys::chord` since MOD-67 M1 and is
//! re-exported here.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::action::{Action, OverlayAction, TabAction};
use crate::ui::overlay::OverlayId;
use crate::ui::tabs::TabId;

/// Moved to `crate::keys::chord` (MOD-67 D2); re-exported so every `crate::keymap::KeyChord`
/// and `htui::keymap::KeyChord` path keeps compiling until M6.
pub use crate::keys::chord::KeyChord;

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

    /// The MOD-1 table: `q`, `Tab`, `Shift+Tab`, `1`..`9`, `?`, and `Esc` for every overlay, plus
    /// `ctrl-c`, which quits from anywhere (MOD-52).
    ///
    /// Raw mode clears `ISIG`, so `ctrl-c` raises no `SIGINT` and arrives as a key like any other.
    /// It is bound twice: globally, and on the overlay wildcard so a modal overlay (the startup
    /// switcher, the migration prompt) does not swallow it. Every capturing section and text field
    /// passes `CONTROL` chords on, so it quits from inside a half-typed field too, where `q` is a
    /// letter.
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
        for scope in [KeyScope::Global, KeyScope::Overlay(OverlayId::ANY)] {
            map.bind(Binding {
                scope,
                key: KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                action: Action::Quit,
                help: "quit",
            });
        }
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

    /// MOD-52: `ctrl-c` quits globally and from every overlay, while `q` stays global only.
    #[test]
    fn ctrl_c_quits_globally_and_from_any_overlay() {
        let map = Keymap::default_global();
        assert!(matches!(
            map.resolve(&KeyScope::Global, chord("ctrl-c")),
            Some(Action::Quit)
        ));
        assert!(matches!(
            map.resolve(
                &KeyScope::Overlay(OverlayId("workspace_switcher")),
                chord("ctrl-c")
            ),
            Some(Action::Quit)
        ));
        assert!(
            map.resolve(
                &KeyScope::Overlay(OverlayId("workspace_switcher")),
                chord("q")
            )
            .is_none(),
            "`q` is still not an overlay key"
        );
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
