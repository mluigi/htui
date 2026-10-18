//! Context stacks and the ordered-candidate resolution over them (ANA-26 §7.3).
//!
//! A stack lists the contexts a key reaches, narrowest first (MOD-67 D6). MOD-67 M1 shipped two:
//! [`Stack::BASE`] (the global layer) and [`Stack::OVERLAY`] (the overlay layer, then only
//! `global.help`). MOD-57 adds the editor's two: [`Stack::EDITOR_FOCUSED`] and
//! [`Stack::EDITOR_OPEN`]. A row for an act in a narrower layer shadows that act in every
//! wider layer, bound or not (blueprint B6): that is how M2's narrower overrides and
//! `reject = []` work. [`DECLARED`] lists every stack the key file's validator walks (MOD-67 M2 D8
//! step 5); M3 adds every Settings and overlay mode's stack from [`views`], M4 the
//! Skills and Requirements ones, M5 the Backlog and Chat ones.
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

    /// Whether the stack captures text (MOD-115 M2 B3): it has a `[field]` layer. The shell
    /// translates the `[field]` chords into the widgets' canonical events only then.
    #[must_use]
    pub fn captures_text(self) -> bool {
        self.0.iter().any(|layer| layer.context == Context::Field)
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

    /// `[overlay, global ∩ {quit} (filtered), global ∩ {help}]`: D6 step 2. `Esc` closes and
    /// `?`/`F1` toggle help over a modal overlay, for keys the overlay passed; MOD-115 M2 B4 adds
    /// the quit layer, so `ctrl-c` resolves to `global.quit` over every overlay.
    pub const OVERLAY: Self = Self(&[
        Layer::all(Context::Overlay),
        Layer::only(Context::Global, &[Act::Quit]).with_modal_filter(),
        Layer::only(Context::Global, &[Act::Help]),
    ]);

    /// `[editor ∩ {focus}]`: MOD-57 P5. While the in-pane editor has the keys only the focus
    /// toggle resolves; every other chord, `ctrl-c` included, is written to the editor.
    pub const EDITOR_FOCUSED: Self = Self(&[Layer::only(Context::Editor, &[Act::EditorFocus])]);

    /// `[editor, global]`: MOD-57 M2 (E3). With the editor alive, unfocused and on screen, a key
    /// resolves through the editor's two actions, then every global action; `App` refuses what
    /// nothing resolves, so the asking view never sees it. On any other tab only `editor.focus`
    /// (`EDITOR_FOCUSED`) is checked ahead of the view (E1).
    pub const EDITOR_OPEN: Self = Self(&[Layer::all(Context::Editor), Layer::all(Context::Global)]);
}

/// The phrase of every collision with the editor's keys (MOD-57 M2 E9): `DECLARED`'s entry for
/// `Stack::EDITOR_OPEN` and `validate`'s focus pass.
pub(super) const WHILE_EDITING: &str = "while an edit is open";

/// Every stack the validator walks (ANA-26 §7.3, MOD-67 M2 D8 step 5), each with the phrase its
/// collision errors end with. M3 appends every Settings and overlay mode's stack
/// ([`views`]). M4 appends the Skills and Requirements stacks and the two editor
/// stacks (MOD-57 §6.4, D8.2), the editor stacks right after the overlay's, so an `[editor]`
/// chord is checked against `global.quit` and `global.help`. M5 appends the Backlog and Chat
/// stacks (in strip order), after M4's. M6 appends the queue overlay's and the directory
/// picker's.
pub static DECLARED: &[(&str, Stack<'static>)] = &[
    ("on every screen", Stack::BASE),
    ("over an overlay", Stack::OVERLAY),
    ("in the focused in-pane editor", Stack::EDITOR_FOCUSED),
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
    ("in Skills > Library", views::LIBRARY_BROWSE),
    (
        "in a Library name, description or import prompt",
        views::LIBRARY_PROMPT,
    ),
    ("in the Library rename form", views::LIBRARY_INFO),
    ("in the Library editor", views::LIBRARY_EDITOR),
    ("in the Library import report", views::LIBRARY_REPORT),
    ("in Skills > Library > Attachments", views::ATTACH_BROWSE),
    ("in the Attachments form", views::ATTACH_FORM),
    ("in the Attachments repo picker", views::ATTACH_PICKER),
    ("in the Attachments detach question", views::ATTACH_CONFIRM),
    ("in Skills > Templates", views::TEMPLATES_BROWSE),
    ("in the Templates name prompt", views::TEMPLATES_PROMPT),
    ("in the Templates editor", views::TEMPLATES_EDITOR),
    ("in the agent help prompt", views::HELP_ASKING),
    ("while the agent help waits", views::HELP_WAITING),
    ("in the agent help proposal", views::HELP_PROPOSAL),
    ("in Requirements", views::REQUIREMENTS_BROWSE),
    ("in a Requirements form", views::REQUIREMENTS_FORM),
    (
        "in the Requirements withdraw form",
        views::REQUIREMENTS_WITHDRAW,
    ),
    ("in the quit question", views::QUIT_CONFIRM),
    ("in the abort question", views::ABORT_CONFIRM),
    (WHILE_EDITING, Stack::EDITOR_OPEN),
    // MOD-67 M5: the Backlog and Chat stacks, in strip order, after M4's (blueprint §4.3).
    ("in Backlog > Body", views::BACKLOG_BODY),
    ("in Backlog > Runs", views::RUNS_BROWSE),
    ("in a Runs question", views::RUNS_CONFIRM),
    ("while the Runs close-out counts", views::RUNS_COUNTING),
    ("in the Runs document view", views::RUNS_ARTIFACT),
    ("in Backlog > Graph", views::BACKLOG_GRAPH),
    ("in Backlog > Docs", views::BACKLOG_DOCS),
    ("in the Docs compose area", views::DOCS_COMPOSE),
    ("in Backlog > Notes", views::BACKLOG_NOTES),
    ("in the Notes compose area", views::NOTES_COMPOSE),
    ("in Backlog > Prompt", views::BACKLOG_PROMPT),
    ("in Backlog > Reqs", views::BACKLOG_REQS),
    ("in the Reqs requirement picker", views::REQS_PICK),
    ("in the Reqs citation kind question", views::REQS_KIND),
    ("in the Reqs uncite question", views::REQS_CONFIRM),
    ("in the Backlog filter form", views::BACKLOG_FILTER),
    ("in the item form", views::ITEM_FORM),
    ("in the item divergence view", views::ITEM_DIVERGENCE),
    ("in Chat", views::CHAT_LIVE),
    ("in a Chat replay", views::CHAT_REPLAY),
    // MOD-67 M6: the last two views that matched raw keys (blueprint §4.4), appended so every
    // existing first-reported phrase is unchanged.
    ("in the queue overlay", views::QUEUE_OVERLAY),
    ("in the Hierarchy directory picker", views::HIERARCHY_PICKER),
    // MOD-115 M2 §1.5: the leave dialog, appended so every first-reported phrase is unchanged.
    // Task 6 removes the quit question's entry with `quit_confirm.rs`.
    ("in the leave dialog", views::LEAVE),
];

/// Keys a view matches ahead of the resolver in one declared stack (ANA-26 §6.1, MOD-67 M5 D5):
/// a choice's value keys, a form's own `Enter`/`Esc`, the permission digits. They are not
/// actions, and no key file moves them, so an act bound to one never hears it there; the
/// validator refuses such a binding (M5 R1 M1), on the user's line, naming the stack.
#[derive(Debug, Clone, Copy)]
pub struct Fixed {
    /// The stack, one of [`DECLARED`]'s; its phrase names it in the error.
    pub stack: Stack<'static>,
    /// The keys, spelled as in a key file, matched on plain keys only (SHIFT aside), so the
    /// validator also refuses a named key's SHIFT form (`shift-left`); a character has none.
    pub chords: &'static [&'static str],
    /// The keys shadow only the stack's global layer: the Chat replay resolves its own acts
    /// first and swallows what they left (M5 R1 M1 (a)).
    pub only_global: bool,
    /// Acts the keys shadow by design, on any of them: the tab select, whose digit must not move
    /// the user off a parked request or a replay (MOD-42 D14, D40).
    pub shadowed: &'static [Act],
}

/// The tab select's acts: the digits a parked request or a replay takes are theirs by default.
const TAB_SELECT: &[Act] = &[
    Act::SelectTab1,
    Act::SelectTab2,
    Act::SelectTab3,
    Act::SelectTab4,
    Act::SelectTab5,
    Act::SelectTab6,
    Act::SelectTab7,
    Act::SelectTab8,
    Act::SelectTab9,
];

/// MOD-115 M2 D7: the tab select and the answers. The digits a parked request takes are
/// `answer.option_*` rows now, still fixed until Task 5 removes `FIXED`.
const TAB_SELECT_AND_ANSWERS: &[Act] = &[
    Act::SelectTab1,
    Act::SelectTab2,
    Act::SelectTab3,
    Act::SelectTab4,
    Act::SelectTab5,
    Act::SelectTab6,
    Act::SelectTab7,
    Act::SelectTab8,
    Act::SelectTab9,
    Act::AnswerOption1,
    Act::AnswerOption2,
    Act::AnswerOption3,
    Act::AnswerOption4,
    Act::AnswerOption5,
    Act::AnswerOption6,
    Act::AnswerOption7,
    Act::AnswerOption8,
    Act::AnswerOption9,
];

/// The plain digits a permission request answers on.
const DIGITS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// Every declared stack with keys its view keeps fixed (MOD-67 M5 R1 M1), in [`DECLARED`] order,
/// each citing the arm that matches them. M5's views and the M3/M4 forms' choice keys (MOD-67
/// M6 D5): the Attachments activation, the Requirements priority.
pub static FIXED: &[Fixed] = &[
    // attach.rs `activation_key`: the activation field's plain `Space` cycles it, `Enter`
    // submits and `Esc` cancels (a choice field's value keys, ANA-26 §6.1); library.rs
    // `on_attach_key` also refuses `Esc` while the form's save is in flight.
    Fixed {
        stack: views::ATTACH_FORM,
        chords: &["space", "enter", "esc"],
        only_global: false,
        shadowed: &[Act::ChoiceToggle, Act::FieldSubmit, Act::FieldCancel],
    },
    // requirements/forms.rs `RequirementForm::on_key`: the priority's `m`/`l` choose,
    // `Space`/`Left`/`Right` toggle, `Esc` cancels (MOD-67 M4 D10).
    Fixed {
        stack: views::REQUIREMENTS_FORM,
        chords: &["m", "l", "space", "left", "right", "esc"],
        only_global: false,
        shadowed: &[
            Act::RequirementsPriorityMust,
            Act::RequirementsPriorityLater,
            Act::ChoicePrev,
            Act::ChoiceNext,
            Act::ChoiceToggle,
            Act::FieldLeft,
            Act::FieldRight,
            Act::FieldCancel,
        ],
    },
    // runs.rs `RunsTab::on_key`: while the step under the cursor has a pending request, every
    // plain digit answers it (MOD-42 D14).
    Fixed {
        stack: views::RUNS_BROWSE,
        chords: DIGITS,
        only_global: false,
        shadowed: TAB_SELECT_AND_ANSWERS,
    },
    // runs.rs `close_out_key`: the close-out warning's arrows and `h`/`l` pick the resolution
    // (MOD-39 P13); the cancel question shares the stack.
    Fixed {
        stack: views::RUNS_CONFIRM,
        chords: &["left", "right", "h", "l"],
        only_global: false,
        shadowed: &[Act::ChoicePrev, Act::ChoiceNext],
    },
    // filter.rs `FilterForm::on_key`: an option row's `Enter` applies, `Esc` cancels, `h`/`l` and
    // the arrows move the option, `Space` ticks it.
    Fixed {
        stack: views::BACKLOG_FILTER,
        chords: &["enter", "esc", "h", "l", "left", "right", "space"],
        only_global: false,
        shadowed: &[
            Act::FieldSubmit,
            Act::FieldCancel,
            Act::FieldLeft,
            Act::FieldRight,
            Act::ChoicePrev,
            Act::ChoiceNext,
            Act::ChoiceToggle,
        ],
    },
    // item_form.rs `ItemForm::on_picker_key`: a picker's `h`/`l` and arrows choose, `Enter`
    // moves on, `Esc` cancels.
    Fixed {
        stack: views::ITEM_FORM,
        chords: &["left", "right", "h", "l", "enter", "esc"],
        only_global: false,
        shadowed: &[
            Act::FieldLeft,
            Act::FieldRight,
            Act::FieldSubmit,
            Act::FieldCancel,
            Act::ChoicePrev,
            Act::ChoiceNext,
        ],
    },
    // chat/mod.rs `ChatTab::on_key`: a parked request takes every plain digit.
    Fixed {
        stack: views::CHAT_LIVE,
        chords: DIGITS,
        only_global: false,
        shadowed: TAB_SELECT_AND_ANSWERS,
    },
    // chat/mod.rs `ChatTab::on_key_replay`: what the replay's own acts left of the live tab's
    // keys is swallowed (D40, D41).
    Fixed {
        stack: views::CHAT_REPLAY,
        chords: &[
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "i", "a", "enter",
        ],
        only_global: true,
        shadowed: TAB_SELECT,
    },
];

impl Keys {
    /// The **ordered candidates** for `chord` in `stack` (ANA-26 §7.3): every action whose row in
    /// the first layer that has one binds `chord`, narrowest layer first, catalogue order within
    /// a layer. A row in a narrower layer **shadows** the same act in every wider layer, even
    /// when it is unbound or binds other chords. The caller handles the first candidate it
    /// accepts. MOD-115 M2: [`CTRL_C`](super::CTRL_C) is `global.quit`'s and `leave.confirm`'s
    /// default, so it is their candidate like any chord; until Task 6, `App::on_key` still checks
    /// it before any stack.
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

    use super::{DECLARED, FIXED, Layer, Stack, WHILE_EDITING};
    use crate::keys::{Act, CTRL_C, Context, KeyChord, Keys, load_str, views};

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
        assert_eq!(DECLARED.len(), 81);
        assert_eq!(DECLARED[80], ("in the leave dialog", views::LEAVE));
        assert!(DECLARED.contains(&(WHILE_EDITING, Stack::EDITOR_OPEN)));
        let phrases: HashSet<&str> = DECLARED.iter().map(|(phrase, _)| *phrase).collect();
        assert_eq!(phrases.len(), DECLARED.len(), "a phrase is used twice");
    }

    /// MOD-67 M5 R1 M1: every `FIXED` entry names a declared stack (its phrase is the error's),
    /// once, and its keys parse.
    #[test]
    fn every_fixed_entry_is_a_declared_stack_once() {
        for (index, entry) in FIXED.iter().enumerate() {
            assert!(
                DECLARED.iter().any(|(_, stack)| *stack == entry.stack),
                "FIXED[{index}] is not declared"
            );
            assert!(
                FIXED[index + 1..]
                    .iter()
                    .all(|other| other.stack != entry.stack),
                "FIXED[{index}] is listed twice"
            );
            for written in entry.chords {
                assert!(KeyChord::parse_strict(written).is_ok(), "{written:?}");
            }
            assert!(!entry.only_global || entry.stack.global().is_some());
        }
    }

    /// MOD-67 M6 D5: `FIXED` keeps `DECLARED` order, so a row and chord two entries fix is
    /// reported naming the first declared stack.
    #[test]
    fn fixed_follows_declared_order() {
        let at = |stack: Stack<'static>| {
            DECLARED
                .iter()
                .position(|(_, declared)| *declared == stack)
                .expect("a declared stack")
        };
        for pair in FIXED.windows(2) {
            assert!(
                at(pair[0].stack) < at(pair[1].stack),
                "FIXED is out of DECLARED order at {:?}",
                DECLARED[at(pair[1].stack)].0
            );
        }
    }

    /// MOD-67 M4 R1 L1: `skills.switch_view` is offered only by the Library and Templates
    /// browse stacks, so a global chord on its `h` is refused naming those views, never a Skills
    /// stack no view is ever in.
    #[test]
    fn a_global_chord_on_the_view_switch_names_the_skills_views() {
        let messages: Vec<String> = match load_str("[global]\nhelp = [\"?\", \"f1\", \"h\"]\n") {
            Ok(_) => panic!("`h` is skills.switch_view's"),
            Err(errors) => errors.into_iter().map(|error| error.message).collect(),
        };
        assert_eq!(
            messages,
            [
                r#"[global] help = "h": "h" is already settings.prev_section (default) in Settings"#,
                r#"[global] help = "h": "h" is already skills.switch_view (default) in Skills > Library"#,
                // MOD-67 M5: and the Backlog's sub-tab cycling.
                r#"[global] help = "h": "h" is already pane.prev_subtab (default) in Backlog > Body"#,
            ]
        );
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

    /// MOD-115 M2 §1.4, B4: `ctrl-c` is `global.quit`'s on every screen, over every overlay and
    /// in every field, but the focused in-pane editor still receives it.
    #[test]
    fn ctrl_c_is_quit_on_every_screen_but_the_focused_editor() {
        let keys = Keys::compiled();
        assert_eq!(keys.actions(Stack::BASE, CTRL_C), [Act::Quit]);
        assert_eq!(keys.actions(Stack::OVERLAY, CTRL_C), [Act::Quit]);
        assert_eq!(keys.actions(Stack::EDITOR_FOCUSED, CTRL_C), []);
        assert_eq!(keys.actions(Stack::EDITOR_OPEN, CTRL_C), [Act::Quit]);
        assert_eq!(keys.actions(views::LEAVE, CTRL_C), [Act::LeaveConfirm]);
        // The quit question has no quit layer: Task 6 replaces it with the leave dialog.
        for &(phrase, stack) in DECLARED {
            if [Stack::EDITOR_FOCUSED, views::LEAVE, views::QUIT_CONFIRM].contains(&stack) {
                continue;
            }
            assert_eq!(keys.actions(stack, CTRL_C), [Act::Quit], "{phrase}");
        }
    }

    /// MOD-115 M2 B3: a stack captures text exactly when it has a `[field]` layer.
    #[test]
    fn a_stack_captures_text_when_it_has_a_field_layer() {
        assert!(views::CAPTURE.captures_text());
        assert!(views::ITEM_FORM.captures_text());
        assert!(views::CONCEPTS_QUERY.captures_text());
        assert!(!Stack::BASE.captures_text());
        assert!(!Stack::OVERLAY.captures_text());
        assert!(!views::BACKLOG_BODY.captures_text());
        assert!(!views::RUNS_CONFIRM.captures_text());
        let layers = [Layer::all(Context::Field)];
        assert!(Stack::new(&layers).captures_text());
    }

    /// MOD-115 M2 B4: the overlay stack's quit layer is filtered and sits before the help
    /// layer, so `Stack::global` is still the help layer and `q` is no candidate.
    #[test]
    fn the_overlay_stack_quits_on_ctrl_c_only() {
        let keys = Keys::compiled();
        assert_eq!(overlay(keys, "q"), []);
        assert_eq!(overlay(keys, "ctrl-c"), [Act::Quit]);
        let (index, layer) = Stack::OVERLAY.global().expect("a global layer");
        assert_eq!(index, Stack::OVERLAY.layers().len() - 1);
        assert!(layer.admits(Act::Help) && !layer.admits(Act::Quit));
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

    fn open(keys: &Keys, spec: &str) -> Vec<Act> {
        keys.actions(Stack::EDITOR_OPEN, chord(spec))
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
    fn the_open_editor_stack_is_the_editor_then_every_global() {
        let keys = Keys::compiled();
        assert_eq!(open(keys, "ctrl-4"), [Act::EditorFocus]);
        assert_eq!(open(keys, "ctrl-x"), [Act::EditorAbort]);
        assert_eq!(open(keys, "q"), [Act::Quit]);
        assert_eq!(open(keys, "?"), [Act::Help]);
        assert_eq!(open(keys, "f1"), [Act::Help]);
        assert_eq!(open(keys, "tab"), [Act::NextTab]);
        assert_eq!(open(keys, "shift-tab"), [Act::PrevTab]);
        assert_eq!(open(keys, "1"), [Act::SelectTab1]);
        assert_eq!(open(keys, "w"), [Act::Workspaces]);
        assert_eq!(open(keys, "ctrl-f"), [Act::Find]);
        assert_eq!(open(keys, "ctrl-w"), [Act::Waiting]);
        assert_eq!(open(keys, "ctrl-q"), [Act::Queue]);
        for spec in ["esc", "e", "j", "ctrl-s"] {
            assert_eq!(open(keys, spec), [], "{spec}");
        }
    }

    #[test]
    fn the_editor_labels_come_through_the_stacks() {
        let keys = Keys::compiled();
        let label = |stack, act| keys.label(stack, act);
        assert_eq!(
            label(Stack::EDITOR_OPEN, Act::EditorFocus).as_deref(),
            Some("Ctrl+4")
        );
        assert_eq!(
            label(Stack::EDITOR_OPEN, Act::EditorAbort).as_deref(),
            Some("Ctrl+x")
        );
        assert_eq!(label(Stack::EDITOR_OPEN, Act::Quit).as_deref(), Some("q"));
        assert_eq!(label(Stack::EDITOR_FOCUSED, Act::EditorAbort), None);
    }
}
