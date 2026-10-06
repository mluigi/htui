//! Legacy key rows (MOD-1 plan D5), shrinking.
//!
//! Since MOD-67 M1 the global and overlay keys are named actions in `crate::keys`. What remains
//! are the six Backlog tab rows `register_all` binds: the help box's half of five arms, and the
//! `Enter` miss. M5 moves them to the `backlog` context; M6 deletes this module. `KeyChord` lives
//! in `crate::keys::chord` since MOD-67 M1 and is re-exported here.

use crate::app::action::Action;
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

    /// An empty table (MOD-67 M1, plan D2).
    ///
    /// It held the global and overlay rows until MOD-67. `q`, `Tab`, `Shift+Tab`, `1`..`9`, `?`,
    /// `Esc` and `ctrl-c` are now catalogue actions (`crate::keys::catalogue`), dispatched by
    /// `App::on_key` through the resolver, and `ctrl-c` is checked before anything else. The name
    /// stays because the test benches of M3-M5's files call it; M6 deletes it with `Keymap`.
    #[must_use]
    pub fn default_global() -> Self {
        Self::new()
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
    use crate::app::action::OverlayAction;

    fn chord(spec: &str) -> KeyChord {
        KeyChord::parse(spec).expect("test specs parse")
    }

    #[test]
    fn the_default_table_is_empty_since_the_catalogue_owns_those_keys() {
        let map = Keymap::default_global();
        assert!(map.help_line(&KeyScope::Global).is_empty());
        assert!(map.help_line(&KeyScope::Overlay(OverlayId::ANY)).is_empty());
    }

    #[test]
    fn an_unknown_chord_and_a_foreign_scope_resolve_to_none() {
        let mut map = Keymap::new();
        map.bind(Binding {
            scope: KeyScope::Global,
            key: chord("q"),
            action: Action::Quit,
            help: "quit",
        });
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
        let mut map = Keymap::new();
        map.bind(Binding {
            scope: KeyScope::Overlay(OverlayId::ANY),
            key: chord("esc"),
            action: Action::Overlay(OverlayAction::Close),
            help: "close",
        });
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
        let mut map = Keymap::new();
        map.bind(Binding {
            scope: KeyScope::Global,
            key: chord("q"),
            action: Action::Quit,
            help: "quit",
        });
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
}
