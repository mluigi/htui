//! Context stacks and the ordered-candidate resolution over them (ANA-26 §7.3).
//!
//! A stack lists the contexts a key reaches, narrowest first (MOD-67 D6). MOD-67 M1 shipped two:
//! [`Stack::BASE`] (the global layer) and [`Stack::OVERLAY`] (the overlay layer, then only
//! `global.help`). MOD-57 adds the editor's two: [`Stack::EDITOR_FOCUSED`] and
//! [`Stack::EDITOR_UNFOCUSED`]. A row for an act in a narrower layer shadows that act in every
//! wider layer, bound or not (blueprint B6): that is how M2's narrower overrides and
//! `reject = []` work. [`DECLARED`] lists every stack the key file's validator walks (MOD-67 M2 D8
//! step 5); M3 adds every Settings and overlay mode's stack from [`views`](super::views).
//!
//! M3 adds two layer kinds. A **view** layer ([`Layer::view`], PA-3) offers the mode's own acts
//! plus every shared act a wider layer of the same stack offers, so a view's override and
//! `VIEW_DEFAULTS` rows apply exactly where the mode offers the act. A **modal** layer
//! ([`Layer::modal`], D5) admits only the chords [`KeyChord::passes_modal`] accepts: the global
//! layer of a capturing or confirming mode.

use super::{Act, Context, KeyChord, Keys, views};

/// One layer of a stack: a context, optionally narrowed to some of its actions, optionally a
/// view layer that also admits the shared actions wider layers offer, optionally filtered to
/// the chords a capturing mode lets through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    context: Context,
    only: Option<&'static [Act]>,
    /// `Layer::view`: also admits every shared act (`Context::is_shared`) that a wider layer of
    /// the same stack admits (PA-3). Only `Stack::admits` reads it.
    inherit: bool,
    /// `Layer::modal`/`with_modal_filter`: only chords with `KeyChord::passes_modal` (D5).
    modal: bool,
}

impl Layer {
    /// Every action of `context`.
    #[must_use]
    pub const fn all(context: Context) -> Self {
        Self {
            context,
            only: None,
            inherit: false,
            modal: false,
        }
    }

    /// Only `acts` of `context`: how the overlay stack lets `global.help` through and nothing
    /// else of the global layer (D6 step 2).
    #[must_use]
    pub const fn only(context: Context, acts: &'static [Act]) -> Self {
        Self {
            context,
            only: Some(acts),
            inherit: false,
            modal: false,
        }
    }

    /// A view's layer (MOD-67 M3 PA-3): `own` (the mode's view verbs), plus every shared act a
    /// wider layer of the same stack offers, so D10 override rows and `VIEW_DEFAULTS` rows of
    /// `context` apply in exactly the modes that offer the act. Never a global or overlay act.
    #[must_use]
    pub const fn view(context: Context, own: &'static [Act]) -> Self {
        Self {
            context,
            only: Some(own),
            inherit: true,
            modal: false,
        }
    }

    /// Every action of `context`, chords filtered to [`KeyChord::passes_modal`] (D5): the
    /// global layer of a capturing or confirming mode.
    #[must_use]
    pub const fn modal(context: Context) -> Self {
        Self::all(context).with_modal_filter()
    }

    /// This layer with D5's chord filter: `Layer::only(Context::Global, &[Act::Help])
    /// .with_modal_filter()` is the concepts search's global layer (`?` is text, `F1` is help).
    #[must_use]
    pub const fn with_modal_filter(self) -> Self {
        Self {
            modal: true,
            ..self
        }
    }

    /// The context.
    #[must_use]
    pub const fn context(self) -> Context {
        self.context
    }

    /// Whether the layer's own set offers `act`, ignoring inheritance. Stack-aware code uses
    /// [`Stack::admits`].
    #[must_use]
    pub fn admits(self, act: Act) -> bool {
        self.only.is_none_or(|acts| acts.contains(&act))
    }

    /// Whether `chord` survives this layer's filter: always, unless the layer is modal.
    #[must_use]
    pub fn admits_chord(self, chord: KeyChord) -> bool {
        !self.modal || chord.passes_modal()
    }

    /// Whether the layer carries D5's filter.
    #[must_use]
    pub const fn is_modal(self) -> bool {
        self.modal
    }

    /// Whether this is a view layer ([`Layer::view`]): it inherits the shared acts offered
    /// below it.
    #[cfg(test)]
    pub(crate) const fn inherits(self) -> bool {
        self.inherit
    }
}

/// A context stack, narrowest layer first (ANA-26 §7.3). M1 needs only the two constants below;
/// M3-M5 compose view stacks as slices of [`Layer`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stack<'a>(&'a [Layer]);

impl<'a> Stack<'a> {
    /// A stack over `layers`, narrowest first.
    #[must_use]
    pub const fn new(layers: &'a [Layer]) -> Self {
        Self(layers)
    }

    /// The layers, narrowest first.
    #[must_use]
    pub const fn layers(self) -> &'a [Layer] {
        self.0
    }

    /// Whether layer `index` offers `act`: its own set, or (a view layer) a shared act whose own
    /// context is a wider layer's context and that layer admits it (PA-3). `false` past the end.
    #[must_use]
    pub fn admits(self, index: usize, act: Act) -> bool {
        let Some(&layer) = self.0.get(index) else {
            return false;
        };
        layer.admits(act)
            || (layer.inherit
                && act.spec().is_some_and(|spec| {
                    spec.context.is_shared()
                        && self.0[index + 1..]
                            .iter()
                            .any(|wider| wider.context == spec.context && wider.admits(act))
                }))
    }

    /// Whether a modal view returns `Pass` for a `chord` it did not use (MOD-67 M3 PA-5): the
    /// stack's global layer admits the chord's shape (an unfiltered one always, a modal one
    /// CONTROL, ALT and function keys). `false` if the stack has no global layer.
    #[must_use]
    pub fn passes(self, chord: KeyChord) -> bool {
        self.global()
            .is_some_and(|(_, layer)| layer.admits_chord(chord))
    }

    /// The last layer whose context is `Global`, with its index.
    pub(crate) fn global(self) -> Option<(usize, Layer)> {
        self.0
            .iter()
            .copied()
            .enumerate()
            .rev()
            .find(|(_, layer)| layer.context == Context::Global)
    }

    /// Whether this stack has a layer of `context` that admits `act` (the D10 legality test).
    pub(crate) fn view_admits(self, context: Context, act: Act) -> bool {
        self.0
            .iter()
            .enumerate()
            .any(|(index, layer)| layer.context == context && self.admits(index, act))
    }
}

impl Stack<'static> {
    /// `[global]`: D6 step 6, the status line and the `?` box's global line.
    pub const BASE: Self = Self(&[Layer::all(Context::Global)]);

    /// `[overlay, global ∩ {help}]`: D6 step 2. `Esc` closes and `?`/`F1` toggle help over a
    /// modal overlay, for keys the overlay passed.
    pub const OVERLAY: Self = Self(&[
        Layer::all(Context::Overlay),
        Layer::only(Context::Global, &[Act::Help]),
    ]);

    /// `[editor ∩ {focus}]`: MOD-57 P5. While the in-pane editor has the keys only the focus
    /// toggle resolves; every other chord, `ctrl-c` included, is written to the editor.
    pub const EDITOR_FOCUSED: Self = Self(&[Layer::only(Context::Editor, &[Act::EditorFocus])]);

    /// `[editor, global ∩ {quit, help}]`: MOD-57 P6, the M1 lock. With the editor alive and
    /// unfocused nothing else resolves; `App` refuses every other key on the status line. M2 opens
    /// the rest of htui.
    pub const EDITOR_UNFOCUSED: Self = Self(&[
        Layer::all(Context::Editor),
        Layer::only(Context::Global, &[Act::Quit, Act::Help]),
    ]);
}

/// Every stack the validator walks (ANA-26 §7.3, MOD-67 M2 D8 step 5), each with the phrase its
/// collision errors end with. M3 appends every Settings and overlay mode's stack
/// ([`views`](super::views)); M4 the two editor stacks, right after the overlay's (MOD-57 §6.4,
/// D8.2), so an `[editor]` chord is checked against `global.quit` and `global.help`; M4-M5
/// append their view stacks.
pub static DECLARED: &[(&str, Stack<'static>)] = &[
    ("on every screen", Stack::BASE),
    ("over an overlay", Stack::OVERLAY),
    ("in the focused in-pane editor", Stack::EDITOR_FOCUSED),
    ("in the in-pane editor", Stack::EDITOR_UNFOCUSED),
    ("in Settings", views::SETTINGS_TAB),
    ("while a field captures keys", views::CAPTURE),
    ("in Settings > Agents", views::AGENTS_BROWSE),
    ("in the Agents install question", views::AGENTS_CONSENT),
    ("in the Agents login chooser", views::AGENTS_CHOOSER),
    ("in the Agents form", views::AGENTS_FORM),
    ("in Settings > Hierarchy", views::HIERARCHY_BROWSE),
    ("in the Hierarchy editor", views::HIERARCHY_EDITOR),
    (
        "while Hierarchy counts a delete",
        views::HIERARCHY_DELETE_COUNTING,
    ),
    (
        "in the Hierarchy delete warning",
        views::HIERARCHY_DELETE_WARN,
    ),
    ("in Settings > Kinds", views::KINDS_BROWSE),
    ("in the Kinds editor", views::KINDS_EDITOR),
    ("in a Kinds question", views::KINDS_CONFIRM),
    ("in Settings > Prompt", views::PROMPT_BROWSE),
    ("in Settings > Connection", views::CONNECTION_BROWSE),
    ("in a Connection question", views::CONNECTION_CONFIRM),
    ("in Settings > Qdrant", views::QDRANT_BROWSE),
    ("in the Qdrant clear question", views::QDRANT_CONFIRM),
    ("in Settings > Boxes", views::BOXES_BROWSE),
    (
        "in the Boxes quirks or probe spec editor",
        views::BOXES_EDITOR,
    ),
    ("in the Boxes executor question", views::BOXES_EXECUTOR),
    ("in Settings > Personas", views::PERSONAS_BROWSE),
    ("in the Personas form", views::PERSONAS_FORM),
    (
        "in the Personas body or rules editor",
        views::PERSONAS_EDITOR,
    ),
    ("in the Personas delete question", views::PERSONAS_DELETE),
    ("in the Personas import report", views::PERSONAS_REPORT),
    ("in Settings > Secrets", views::SECRETS_BROWSE),
    ("in a Secrets form", views::SECRETS_FORM),
    ("in a Secrets question", views::SECRETS_CONFIRM),
    ("in Settings > Queue", views::QUEUE_BROWSE),
    ("in the concepts search", views::CONCEPTS_QUERY),
    ("in the workspace switcher", views::SWITCHER),
    ("in the migration prompt", views::MIGRATION),
    ("in the waiting list", views::WAITING_LIST),
];

impl Keys {
    /// The **ordered candidates** for `chord` in `stack` (ANA-26 §7.3): every action whose row in
    /// the first layer that has one binds `chord`, narrowest layer first, catalogue order within
    /// a layer. A row in a narrower layer **shadows** the same act in every wider layer, even
    /// when it is unbound or binds other chords. The caller handles the first candidate it
    /// accepts. [`CTRL_C`](super::CTRL_C) is never a candidate: no catalogue default binds it and
    /// M2's loader refuses it, and `App::on_key` checks it before any stack regardless.
    ///
    /// A row is in a layer when [`Stack::admits`] says the layer offers its act (a view layer
    /// inherits the shared acts offered below it, PA-3). A modal layer's row is a candidate only
    /// for a chord the layer's filter admits (D5); a filtered-out chord still shadows the act.
    #[must_use]
    pub fn actions(&self, stack: Stack<'_>, chord: KeyChord) -> Vec<Act> {
        let mut seen: Vec<Act> = Vec::new();
        let mut out = Vec::new();
        for (index, layer) in stack.layers().iter().enumerate() {
            let mut here = Vec::new();
            let rows = self
                .rows
                .iter()
                .filter(|row| row.context == layer.context && stack.admits(index, row.act));
            for row in rows {
                if seen.contains(&row.act) {
                    continue; // shadowed by a narrower layer
                }
                here.push(row.act);
                if row.chords.contains(&chord) && layer.admits_chord(chord) {
                    out.push(row.act);
                }
            }
            seen.extend(here);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{DECLARED, Layer, Stack};
    use crate::keys::{Act, CTRL_C, Context, KeyChord, Keys, views};

    fn chord(spec: &str) -> KeyChord {
        KeyChord::parse_strict(spec).expect("a valid spec")
    }

    fn base(keys: &Keys, spec: &str) -> Vec<Act> {
        keys.actions(Stack::BASE, chord(spec))
    }

    fn overlay(keys: &Keys, spec: &str) -> Vec<Act> {
        keys.actions(Stack::OVERLAY, chord(spec))
    }

    #[test]
    fn the_base_stack_resolves_quit_tabs_digits_help_and_the_offered_globals() {
        let keys = Keys::compiled();
        assert_eq!(base(keys, "q"), [Act::Quit]);
        assert_eq!(base(keys, "tab"), [Act::NextTab]);
        assert_eq!(base(keys, "shift-tab"), [Act::PrevTab]);
        assert_eq!(base(keys, "3"), [Act::SelectTab3]);
        assert_eq!(base(keys, "?"), [Act::Help]);
        assert_eq!(base(keys, "f1"), [Act::Help]);
        assert_eq!(base(keys, "w"), [Act::Workspaces]);
        assert_eq!(base(keys, "ctrl-f"), [Act::Find]);
        assert_eq!(base(keys, "ctrl-w"), [Act::Waiting]);
        assert_eq!(base(keys, "ctrl-q"), [Act::Queue]);
        assert_eq!(base(keys, "z"), []);
        assert_eq!(base(keys, "esc"), []);
    }

    #[test]
    fn declared_starts_with_base_and_overlay_and_holds_every_view_stack() {
        assert_eq!(DECLARED[0], ("on every screen", Stack::BASE));
        assert_eq!(DECLARED[1], ("over an overlay", Stack::OVERLAY));
        // MOD-67 M4 D8.2 (PA-4): the in-pane editor's two stacks, one phrase each.
        assert_eq!(
            DECLARED[2],
            ("in the focused in-pane editor", Stack::EDITOR_FOCUSED)
        );
        assert_eq!(
            DECLARED[3],
            ("in the in-pane editor", Stack::EDITOR_UNFOCUSED)
        );
        assert_eq!(DECLARED.len(), 38);
        let phrases: HashSet<&str> = DECLARED.iter().map(|(phrase, _)| *phrase).collect();
        assert_eq!(phrases.len(), DECLARED.len(), "a phrase is used twice");
    }

    #[test]
    fn a_modal_layer_admits_only_ctrl_alt_and_function_keys() {
        let keys = Keys::compiled();
        let layers = [Layer::modal(Context::Global)];
        let stack = Stack::new(&layers);
        for spec in ["q", "tab", "?"] {
            assert_eq!(keys.actions(stack, chord(spec)), [], "{spec}");
        }
        assert_eq!(keys.actions(stack, chord("f1")), [Act::Help]);
        assert_eq!(keys.actions(stack, chord("ctrl-f")), [Act::Find]);
        assert!(layers[0].is_modal());
        assert!(!Layer::all(Context::Global).is_modal());
    }

    #[test]
    fn only_and_the_modal_filter_combine() {
        let keys = Keys::compiled();
        let layers = [Layer::only(Context::Global, &[Act::Help]).with_modal_filter()];
        let stack = Stack::new(&layers);
        assert_eq!(keys.actions(stack, chord("?")), []);
        assert_eq!(keys.actions(stack, chord("f1")), [Act::Help]);
        assert_eq!(keys.actions(stack, chord("ctrl-f")), []);
    }

    #[test]
    fn a_view_layer_inherits_only_shared_acts_offered_below() {
        let keys = Keys::defaults()
            .with_chords(Context::SettingsBoxes, Act::Reload, &["f5"])
            .with_chords(Context::SettingsBoxes, Act::Quit, &["f6"]);
        let offered = [
            Layer::view(Context::SettingsBoxes, &[]),
            Layer::only(Context::Common, &[Act::Reload]),
        ];
        let stack = Stack::new(&offered);
        assert_eq!(keys.actions(stack, chord("f5")), [Act::Reload]);
        assert_eq!(keys.actions(stack, chord("r")), []);
        let not_offered = [
            Layer::view(Context::SettingsBoxes, &[]),
            Layer::only(Context::Common, &[Act::Dismiss]),
        ];
        let stack = Stack::new(&not_offered);
        assert_eq!(keys.actions(stack, chord("f5")), []);
        assert_eq!(keys.actions(stack, chord("r")), []);
        let with_global = [
            Layer::view(Context::SettingsBoxes, &[]),
            Layer::all(Context::Global),
        ];
        let stack = Stack::new(&with_global);
        assert!(
            !stack.admits(0, Act::Quit),
            "a global act is never inherited"
        );
        assert_eq!(keys.actions(stack, chord("f6")), []);
        assert_eq!(keys.actions(stack, chord("q")), [Act::Quit]);
    }

    #[test]
    fn passes_follows_the_global_layer() {
        assert!(views::CAPTURE.passes(CTRL_C));
        assert!(!views::CAPTURE.passes(chord("tab")));
        assert!(views::AGENTS_BROWSE.passes(chord("q")));
        let layers = [Layer::all(Context::Confirm)];
        assert!(!Stack::new(&layers).passes(CTRL_C));
    }

    #[test]
    fn ctrl_c_is_no_action_in_either_stack() {
        let keys = Keys::compiled();
        assert_eq!(keys.actions(Stack::BASE, CTRL_C), []);
        assert_eq!(keys.actions(Stack::OVERLAY, CTRL_C), []);
        assert_eq!(keys.actions(Stack::EDITOR_FOCUSED, CTRL_C), []);
        assert_eq!(keys.actions(Stack::EDITOR_UNFOCUSED, CTRL_C), []);
    }

    #[test]
    fn the_overlay_stack_lets_only_help_through_from_global() {
        let keys = Keys::compiled();
        assert_eq!(overlay(keys, "esc"), [Act::OverlayClose]);
        assert_eq!(overlay(keys, "?"), [Act::Help]);
        assert_eq!(overlay(keys, "f1"), [Act::Help]);
        for spec in ["q", "tab", "1", "w"] {
            assert_eq!(overlay(keys, spec), [], "{spec}");
        }
    }

    #[test]
    fn candidates_come_narrowest_first() {
        let layers = [Layer::all(Context::Confirm), Layer::all(Context::Common)];
        let found = Keys::compiled().actions(Stack::new(&layers), chord("esc"));
        assert_eq!(found, [Act::ConfirmNo, Act::Back, Act::Dismiss]);
    }

    #[test]
    fn a_narrower_row_shadows_the_shared_one() {
        let keys = Keys::defaults().with_chords(Context::Form, Act::Reload, &["f5"]);
        let layers = [Layer::all(Context::Form), Layer::all(Context::Common)];
        let stack = Stack::new(&layers);
        assert_eq!(keys.actions(stack, chord("f5")), [Act::Reload]);
        assert_eq!(keys.actions(stack, chord("r")), []);
    }

    #[test]
    fn an_unbound_action_is_never_a_candidate() {
        let keys = Keys::defaults().with_chords(Context::Global, Act::Quit, &[]);
        assert_eq!(base(&keys, "q"), []);
    }

    // MOD-57 M1 (plan P5, P6): the in-pane editor's two stacks.

    fn focused(keys: &Keys, spec: &str) -> Vec<Act> {
        keys.actions(Stack::EDITOR_FOCUSED, chord(spec))
    }

    fn unfocused(keys: &Keys, spec: &str) -> Vec<Act> {
        keys.actions(Stack::EDITOR_UNFOCUSED, chord(spec))
    }

    #[test]
    fn the_focused_editor_stack_resolves_only_the_toggle() {
        let keys = Keys::compiled();
        assert_eq!(focused(keys, "ctrl-4"), [Act::EditorFocus]);
        for spec in ["ctrl-x", "q", "?", "f1", "esc", "tab"] {
            assert_eq!(focused(keys, spec), [], "{spec}");
        }
    }

    #[test]
    fn the_unfocused_editor_stack_is_the_m1_lock() {
        let keys = Keys::compiled();
        assert_eq!(unfocused(keys, "ctrl-4"), [Act::EditorFocus]);
        assert_eq!(unfocused(keys, "ctrl-x"), [Act::EditorAbort]);
        assert_eq!(unfocused(keys, "q"), [Act::Quit]);
        assert_eq!(unfocused(keys, "?"), [Act::Help]);
        assert_eq!(unfocused(keys, "f1"), [Act::Help]);
        for spec in ["tab", "1", "w", "ctrl-f", "ctrl-w", "esc", "e", "ctrl-s"] {
            assert_eq!(unfocused(keys, spec), [], "{spec}");
        }
    }

    #[test]
    fn the_editor_labels_come_through_the_stacks() {
        let keys = Keys::compiled();
        let label = |stack, act| keys.label(stack, act);
        assert_eq!(
            label(Stack::EDITOR_UNFOCUSED, Act::EditorFocus).as_deref(),
            Some("Ctrl+4")
        );
        assert_eq!(
            label(Stack::EDITOR_UNFOCUSED, Act::EditorAbort).as_deref(),
            Some("Ctrl+x")
        );
        assert_eq!(
            label(Stack::EDITOR_UNFOCUSED, Act::Quit).as_deref(),
            Some("q")
        );
        assert_eq!(label(Stack::EDITOR_FOCUSED, Act::EditorAbort), None);
    }
}
