//! Context stacks and the ordered-candidate resolution over them (ANA-26 §7.3).
//!
//! A stack lists the contexts a key reaches, narrowest first (MOD-67 D6). M1 ships two:
//! [`Stack::BASE`] (the global layer) and [`Stack::OVERLAY`] (the overlay layer, then only
//! `global.help`). A row for an act in a narrower layer shadows that act in every wider layer,
//! bound or not (blueprint B6): that is how M2's narrower overrides and `reject = []` work.
//! [`DECLARED`] lists every stack the key file's validator walks (MOD-67 M2 D8 step 5).

use super::{Act, Context, KeyChord, Keys};

/// One layer of a stack: a context, optionally narrowed to some of its actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    context: Context,
    only: Option<&'static [Act]>,
}

impl Layer {
    /// Every action of `context`.
    #[must_use]
    pub const fn all(context: Context) -> Self {
        Self {
            context,
            only: None,
        }
    }

    /// Only `acts` of `context`: how the overlay stack lets `global.help` through and nothing
    /// else of the global layer (D6 step 2).
    #[must_use]
    pub const fn only(context: Context, acts: &'static [Act]) -> Self {
        Self {
            context,
            only: Some(acts),
        }
    }

    /// The context.
    #[must_use]
    pub const fn context(self) -> Context {
        self.context
    }

    /// Whether this layer offers `act`.
    #[must_use]
    pub fn admits(self, act: Act) -> bool {
        self.only.is_none_or(|acts| acts.contains(&act))
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
}

/// Every stack the validator walks (ANA-26 §7.3, MOD-67 M2 D8 step 5), each with the phrase its
/// collision errors end with. M3-M5 append each view mode's stack.
pub static DECLARED: &[(&str, Stack<'static>)] = &[
    ("on every screen", Stack::BASE),
    ("over an overlay", Stack::OVERLAY),
];

impl Keys {
    /// The **ordered candidates** for `chord` in `stack` (ANA-26 §7.3): every action whose row in
    /// the first layer that has one binds `chord`, narrowest layer first, catalogue order within
    /// a layer. A row in a narrower layer **shadows** the same act in every wider layer, even
    /// when it is unbound or binds other chords. The caller handles the first candidate it
    /// accepts. [`CTRL_C`](super::CTRL_C) is never a candidate: no catalogue default binds it and
    /// M2's loader refuses it, and `App::on_key` checks it before any stack regardless.
    #[must_use]
    pub fn actions(&self, stack: Stack<'_>, chord: KeyChord) -> Vec<Act> {
        let mut seen: Vec<Act> = Vec::new();
        let mut out = Vec::new();
        for layer in stack.layers() {
            let mut here = Vec::new();
            let rows = self
                .rows
                .iter()
                .filter(|row| row.context == layer.context && layer.admits(row.act));
            for row in rows {
                if seen.contains(&row.act) {
                    continue; // shadowed by a narrower layer
                }
                here.push(row.act);
                if row.chords.contains(&chord) {
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
    use super::{DECLARED, Layer, Stack};
    use crate::keys::{Act, CTRL_C, Context, KeyChord, Keys};

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
        assert_eq!(base(keys, "z"), []);
        assert_eq!(base(keys, "esc"), []);
    }

    #[test]
    fn declared_walks_every_screen_then_over_an_overlay() {
        assert_eq!(
            DECLARED,
            [
                ("on every screen", Stack::BASE),
                ("over an overlay", Stack::OVERLAY)
            ]
        );
    }

    #[test]
    fn ctrl_c_is_no_action_in_either_stack() {
        let keys = Keys::compiled();
        assert_eq!(keys.actions(Stack::BASE, CTRL_C), []);
        assert_eq!(keys.actions(Stack::OVERLAY, CTRL_C), []);
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
}
