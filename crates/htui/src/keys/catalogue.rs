//! The action catalogue: every named action, its context, defaults and help (ANA-26 §7.2).
//!
//! MOD-67 M1 (plan D4, D12) fills the global, overlay and shared contexts. Global and overlay
//! rows reproduce today's `Keymap::default_global` bindings, plus `f1` for help (ANA-26 §6.5).
//! Each shared-context row (`list`, `pane`, `confirm`, `form`, `common`) takes its defaults from
//! today's match arms and cites the arm it mirrors. No view consumes a shared context until M3,
//! so those rows change no behaviour yet. M3-M5 append each view's context in its own block.
//!
//! Defaults are strict spec strings (blueprint B4): lower-case key names (`tab`, `esc`, `pgdn`,
//! `f1`), the character itself for a character (`G`, `]`), and `ctrl-x` for a ctrl chord. This
//! module does not parse them; `keys::Keys::defaults` does, and its tests pin that every one
//! parses. `ctrl-c` is never a default: it is fixed (`keys::chord::CTRL_C`).
//!
//! `in_capture` is a property of an action (blueprint B3). It means "offered while a text field
//! captures", so every default and every user chord of the action must be non-printable
//! (`KeyChord::is_printable`; M2's validator). The global layer is **not** marked. Under a
//! capturing mode (M3) the global layer is filtered chord by chord, and printable chords drop
//! out. So `global.help` (`?`, `f1`) reaches help through `f1` while a field types `?`, and
//! `global.quit` (`q`) is unreachable there. `ctrl-c` quits regardless.

use Context::{Common, Confirm, Form, Global, List, Overlay, Pane};

/// A key context: a TOML table of `keys.toml` and a layer of a context stack (ANA-26 §7.2-§7.3).
/// M1 has the global, overlay and shared contexts; M3-M5 append view contexts (`SettingsAgents`,
/// `BacklogRuns`, ...), each in its own block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// `[global]`: reachable from every screen, checked last.
    Global,
    /// `[overlay]`: every overlay's own keys, ahead of the modal swallow.
    Overlay,
    /// `[list]`: moving a cursor through rows.
    List,
    /// `[pane]`: scrolling a read-only pane and cycling its sub-tabs.
    Pane,
    /// `[confirm]`: answering a yes/no question.
    Confirm,
    /// `[form]`: moving between a form's fields and saving it; reachable while a field captures.
    Form,
    /// `[common]`: verbs every view that offers them shares (edit, new, delete, ...).
    Common,
}

impl Context {
    /// The TOML table name: `global`, `overlay`, `list`, `pane`, `confirm`, `form`, `common`.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Overlay => "overlay",
            Self::List => "list",
            Self::Pane => "pane",
            Self::Confirm => "confirm",
            Self::Form => "form",
            Self::Common => "common",
        }
    }

    /// The `?` box heading: `Global`, `Overlay`, `List`, `Pane`, `Confirm`, `Form`, `Common`.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Overlay => "Overlay",
            Self::List => "List",
            Self::Pane => "Pane",
            Self::Confirm => "Confirm",
            Self::Form => "Form",
            Self::Common => "Common",
        }
    }
}

/// A named action: one per meaning, not per letter (ANA-26 §6.2). Views (M3-M5) match on it
/// instead of `KeyCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act {
    /// `global.quit`: leave htui.
    Quit,
    /// `global.next_tab`: the tab to the right.
    NextTab,
    /// `global.prev_tab`: the tab to the left.
    PrevTab,
    /// `global.select_tab_1`: the first tab.
    SelectTab1,
    /// `global.select_tab_2`: the second tab.
    SelectTab2,
    /// `global.select_tab_3`: the third tab.
    SelectTab3,
    /// `global.select_tab_4`: the fourth tab.
    SelectTab4,
    /// `global.select_tab_5`: the fifth tab.
    SelectTab5,
    /// `global.select_tab_6`: the sixth tab.
    SelectTab6,
    /// `global.select_tab_7`: the seventh tab.
    SelectTab7,
    /// `global.select_tab_8`: the eighth tab.
    SelectTab8,
    /// `global.select_tab_9`: the ninth tab.
    SelectTab9,
    /// `global.help`: toggle the `?` box.
    Help,
    /// `global.workspaces`: open the workspace switcher (offered by `register_all`).
    Workspaces,
    /// `global.find`: open the concepts search (offered by `register_all`).
    Find,
    /// `global.waiting`: open the waiting list (offered by `register_all`).
    Waiting,
    /// `overlay.close`: close the top overlay.
    OverlayClose,
    /// `list.down`: the next row.
    ListDown,
    /// `list.up`: the previous row.
    ListUp,
    /// `list.top`: the first row.
    ListTop,
    /// `list.bottom`: the last row.
    ListBottom,
    /// `list.fold`: fold or unfold the row under the cursor.
    ListFold,
    /// `pane.scroll_down`: scroll the pane one line down.
    PaneScrollDown,
    /// `pane.scroll_up`: scroll the pane one line up.
    PaneScrollUp,
    /// `pane.page_down`: scroll the pane one page down.
    PanePageDown,
    /// `pane.page_up`: scroll the pane one page up.
    PanePageUp,
    /// `pane.next_subtab`: the next sub-tab of the pane.
    PaneNextSubtab,
    /// `pane.prev_subtab`: the previous sub-tab of the pane.
    PanePrevSubtab,
    /// `confirm.yes`: answer yes.
    ConfirmYes,
    /// `confirm.no`: answer no.
    ConfirmNo,
    /// `form.next_field`: focus the next field.
    FormNextField,
    /// `form.prev_field`: focus the previous field.
    FormPrevField,
    /// `form.save`: save the form.
    FormSave,
    /// `form.external_editor`: edit the focused field in `$EDITOR`.
    FormExternalEditor,
    /// `common.edit`: edit the selected item.
    Edit,
    /// `common.new`: create an item.
    New,
    /// `common.delete`: delete the selected item.
    Delete,
    /// `common.clear`: clear the selected value.
    Clear,
    /// `common.reload`: reload from the source.
    Reload,
    /// `common.back`: leave the view's inner level.
    Back,
    /// `common.dismiss`: clear the notice.
    Dismiss,
}

impl Act {
    /// The tab index of `select_tab_1`..`_9` (0..=8); `None` for every other action.
    #[must_use]
    pub const fn tab_index(self) -> Option<usize> {
        Some(match self {
            Self::SelectTab1 => 0,
            Self::SelectTab2 => 1,
            Self::SelectTab3 => 2,
            Self::SelectTab4 => 3,
            Self::SelectTab5 => 4,
            Self::SelectTab6 => 5,
            Self::SelectTab7 => 6,
            Self::SelectTab8 => 7,
            Self::SelectTab9 => 8,
            _ => return None,
        })
    }

    /// This action's catalogue row. `None` only if a variant was added without a row, which
    /// the test `every_act_is_listed_and_has_a_row` forbids.
    #[must_use]
    pub fn spec(self) -> Option<&'static ActionSpec> {
        CATALOGUE.iter().find(|row| row.act == self)
    }
}

/// One catalogue row: the single source of truth for an action's name, defaults and help
/// (ANA-26 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    /// The action.
    pub act: Act,
    /// Its context (TOML table).
    pub context: Context,
    /// Its key in that table, e.g. `"next_tab"`.
    pub name: &'static str,
    /// Default chords as strict spec strings, parsed by `keys::Keys::defaults`; `[]` is unbound.
    pub defaults: &'static [&'static str],
    /// The `?` box and status-line label, e.g. `"next tab"`.
    pub help: &'static str,
    /// Offered while a text field captures, so every chord must be non-printable (ANA-26 §6.4).
    pub in_capture: bool,
}

/// A row that is not offered while a text field captures.
const fn row(
    act: Act,
    context: Context,
    name: &'static str,
    defaults: &'static [&'static str],
    help: &'static str,
) -> ActionSpec {
    ActionSpec {
        act,
        context,
        name,
        defaults,
        help,
        in_capture: false,
    }
}

/// A row that stays offered while a text field captures (`in_capture`, blueprint B3).
const fn capture_row(
    act: Act,
    context: Context,
    name: &'static str,
    defaults: &'static [&'static str],
    help: &'static str,
) -> ActionSpec {
    ActionSpec {
        in_capture: true,
        ..row(act, context, name, defaults, help)
    }
}

/// Every action, grouped by context, in display order (the status line and the `?` box follow
/// it). M3-M5 append their contexts' blocks at the end.
pub static CATALOGUE: &[ActionSpec] = &[
    // [global]: today's `keymap.rs` `default_global` rows and `app/mod.rs` `register_all`
    // labels, in status-line order. `ctrl-c` is fixed (`CTRL_C`) and never listed.
    row(Act::Quit, Global, "quit", &["q"], "quit"),
    row(Act::NextTab, Global, "next_tab", &["tab"], "next tab"),
    row(
        Act::PrevTab,
        Global,
        "prev_tab",
        &["backtab"],
        "previous tab",
    ),
    row(
        Act::SelectTab1,
        Global,
        "select_tab_1",
        &["1"],
        "select tab",
    ),
    row(
        Act::SelectTab2,
        Global,
        "select_tab_2",
        &["2"],
        "select tab",
    ),
    row(
        Act::SelectTab3,
        Global,
        "select_tab_3",
        &["3"],
        "select tab",
    ),
    row(
        Act::SelectTab4,
        Global,
        "select_tab_4",
        &["4"],
        "select tab",
    ),
    row(
        Act::SelectTab5,
        Global,
        "select_tab_5",
        &["5"],
        "select tab",
    ),
    row(
        Act::SelectTab6,
        Global,
        "select_tab_6",
        &["6"],
        "select tab",
    ),
    row(
        Act::SelectTab7,
        Global,
        "select_tab_7",
        &["7"],
        "select tab",
    ),
    row(
        Act::SelectTab8,
        Global,
        "select_tab_8",
        &["8"],
        "select tab",
    ),
    row(
        Act::SelectTab9,
        Global,
        "select_tab_9",
        &["9"],
        "select tab",
    ),
    // `f1` is new (ANA-26 §6.5, D12); not `in_capture`, `?` is printable (B3).
    row(Act::Help, Global, "help", &["?", "f1"], "help"),
    // Offered by `register_all` (D5): `app/mod.rs` workspace switcher, concepts search and
    // waiting list (MOD-69 D7).
    row(Act::Workspaces, Global, "workspaces", &["w"], "workspaces"),
    row(Act::Find, Global, "find", &["ctrl-f"], "find"),
    row(Act::Waiting, Global, "waiting", &["ctrl-w"], "waiting"),
    // [overlay]: `keymap.rs`'s wildcard overlay row. In capture because the concepts field
    // returns `Pass` on `Esc` and this layer closes it (`concepts_search.rs:334-335`).
    capture_row(Act::OverlayClose, Overlay, "close", &["esc"], "close"),
    // [list]
    // backlog/mod.rs:655, requirements/mod.rs:430, connection.rs:811, kinds.rs:1492,
    // qdrant.rs:404, workspace_switcher.rs:163, waiting_list.rs:274. agents.rs:2420 and
    // hierarchy.rs:1211 take `j` only (ANA §6.6 adds `down`).
    row(Act::ListDown, List, "down", &["j", "down"], "down"),
    // backlog/mod.rs:656, requirements/mod.rs:431, connection.rs:815, kinds.rs:1496,
    // boxes.rs:699, library.rs:689.
    row(Act::ListUp, List, "up", &["k", "up"], "up"),
    // backlog/mod.rs:657, requirements/mod.rs:432. kinds.rs:1476 binds `g` to open graph (M3).
    row(Act::ListTop, List, "top", &["g", "home"], "top"),
    // backlog/mod.rs:658, requirements/mod.rs:433.
    row(Act::ListBottom, List, "bottom", &["G", "end"], "bottom"),
    // backlog/mod.rs:684-689 (fold, else the detail pane), requirements/mod.rs:434. Elsewhere
    // `Enter` is a view verb (connection.rs:807, workspace_switcher.rs:171).
    row(Act::ListFold, List, "fold", &["enter"], "fold"),
    // [pane]
    // backlog/detail/mod.rs:447 (`Scroll::on_key`), requirements/mod.rs:435, library.rs:708,
    // templates.rs:528. runs.rs:1434 `J` is next step (a view verb, ANA §6.2).
    row(
        Act::PaneScrollDown,
        Pane,
        "scroll_down",
        &["J"],
        "scroll down",
    ),
    // backlog/detail/mod.rs:448.
    row(Act::PaneScrollUp, Pane, "scroll_up", &["K"], "scroll up"),
    // backlog/detail/mod.rs:449, graph.rs:697, divergence.rs:261.
    row(Act::PanePageDown, Pane, "page_down", &["pgdn"], "page down"),
    // backlog/detail/mod.rs:450.
    row(Act::PanePageUp, Pane, "page_up", &["pgup"], "page up"),
    // backlog/mod.rs:659. settings/mod.rs:340 binds the same chords to sections (M3).
    row(
        Act::PaneNextSubtab,
        Pane,
        "next_subtab",
        &["l", "]", "right"],
        "next sub-tab",
    ),
    // backlog/mod.rs:660. settings/mod.rs:344 (sections).
    row(
        Act::PanePrevSubtab,
        Pane,
        "prev_subtab",
        &["h", "[", "left"],
        "previous sub-tab",
    ),
    // [confirm]
    // connection.rs:483, qdrant.rs:214, kinds.rs:715, hierarchy.rs:769, personas.rs:785,
    // boxes.rs:545, runs.rs:583, detail/requirements.rs:306. migration_prompt.rs:92 also
    // takes `Y`.
    row(Act::ConfirmYes, Confirm, "yes", &["y"], "yes"),
    // connection.rs:487, qdrant.rs:218, kinds.rs:726, agents.rs:1161, boxes.rs:546,
    // personas.rs:791, runs.rs:587, detail/requirements.rs:317, hierarchy.rs:761.
    // migration_prompt.rs:97 also takes `N`.
    row(Act::ConfirmNo, Confirm, "no", &["n", "esc"], "no"),
    // [form]: all in capture; every chord is named or ctrl.
    // item_form.rs:613, requirements/forms.rs:306, compose.rs:340. The Settings and attach
    // forms also take `Down` (agents.rs:2170, attach.rs:488), added per view in M3/M4.
    capture_row(
        Act::FormNextField,
        Form,
        "next_field",
        &["tab"],
        "next field",
    ),
    // item_form.rs:617, requirements/forms.rs:310, compose.rs:344.
    capture_row(
        Act::FormPrevField,
        Form,
        "prev_field",
        &["backtab"],
        "previous field",
    ),
    // text_area.rs:194, item_form.rs:275, requirements/mod.rs:233, attach.rs:477,
    // library.rs:939. Every site also accepts `ctrl-S` (blueprint F-7, M4).
    capture_row(Act::FormSave, Form, "save", &["ctrl-s"], "save"),
    // item_form.rs:281 (label at :63), library.rs:1021, templates.rs:689. Same `E` alias.
    capture_row(
        Act::FormExternalEditor,
        Form,
        "external_editor",
        &["ctrl-e"],
        "$EDITOR",
    ),
    // [common]
    // connection.rs:779, qdrant.rs:382, kinds.rs:1468, hierarchy.rs:1241, prompt.rs:849,
    // agents.rs:2401, backlog/mod.rs:680. boxes.rs:707 `e` edits quirks (a view verb).
    row(Act::Edit, Common, "edit", &["e"], "edit"),
    // kinds.rs:1452, hierarchy.rs:1233, agents.rs:2395, library.rs:694, templates.rs:527.
    // backlog/mod.rs:679 uses `N`.
    row(Act::New, Common, "new", &["n"], "new"),
    // hierarchy.rs:1278, kinds.rs:1484.
    row(Act::Delete, Common, "delete", &["d"], "delete"),
    // connection.rs:785, qdrant.rs:388. detail/requirements.rs:481 `c` is the cite picker.
    row(Act::Clear, Common, "clear", &["c"], "clear"),
    // connection.rs:822, qdrant.rs:414, kinds.rs:1505, hierarchy.rs:1288, prompt.rs:869,
    // boxes.rs:733, requirements/mod.rs:449, library.rs:690, templates.rs:523, attach.rs:398.
    // agents.rs:2495 `r` probes (a view verb).
    row(Act::Reload, Common, "reload", &["r"], "reload"),
    // divergence.rs:245, library.rs:895, chat/mod.rs:233. personas.rs:841 also takes `Enter`.
    row(Act::Back, Common, "back", &["esc"], "back"),
    // connection.rs:828, boxes.rs:739, hierarchy.rs:1294, kinds.rs:1511, prompt.rs:875.
    // Shares `Esc` with `back`: `STATE_GUARDED`.
    row(Act::Dismiss, Common, "dismiss", &["esc"], "dismiss"),
];

/// Default pairs that share a chord in one context because a view accepts at most one of them
/// in any state and declines the other (ANA-26 §7.4 step 7, "state-guarded"). It starts M2's
/// reviewed allow-list.
pub static STATE_GUARDED: &[(Act, Act)] = &[(Act::Back, Act::Dismiss)];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{Act, CATALOGUE, Context, STATE_GUARDED};

    /// Every [`Act`], in declaration order. [`position`] keeps it complete.
    const ALL: &[Act] = &[
        Act::Quit,
        Act::NextTab,
        Act::PrevTab,
        Act::SelectTab1,
        Act::SelectTab2,
        Act::SelectTab3,
        Act::SelectTab4,
        Act::SelectTab5,
        Act::SelectTab6,
        Act::SelectTab7,
        Act::SelectTab8,
        Act::SelectTab9,
        Act::Help,
        Act::Workspaces,
        Act::Find,
        Act::Waiting,
        Act::OverlayClose,
        Act::ListDown,
        Act::ListUp,
        Act::ListTop,
        Act::ListBottom,
        Act::ListFold,
        Act::PaneScrollDown,
        Act::PaneScrollUp,
        Act::PanePageDown,
        Act::PanePageUp,
        Act::PaneNextSubtab,
        Act::PanePrevSubtab,
        Act::ConfirmYes,
        Act::ConfirmNo,
        Act::FormNextField,
        Act::FormPrevField,
        Act::FormSave,
        Act::FormExternalEditor,
        Act::Edit,
        Act::New,
        Act::Delete,
        Act::Clear,
        Act::Reload,
        Act::Back,
        Act::Dismiss,
    ];

    /// `act`'s index in [`ALL`]. No wildcard arm: a new variant fails to compile here until it is
    /// listed, and then in [`ALL`] too, or `every_act_is_listed_and_has_a_row` fails.
    const fn position(act: Act) -> usize {
        match act {
            Act::Quit => 0,
            Act::NextTab => 1,
            Act::PrevTab => 2,
            Act::SelectTab1 => 3,
            Act::SelectTab2 => 4,
            Act::SelectTab3 => 5,
            Act::SelectTab4 => 6,
            Act::SelectTab5 => 7,
            Act::SelectTab6 => 8,
            Act::SelectTab7 => 9,
            Act::SelectTab8 => 10,
            Act::SelectTab9 => 11,
            Act::Help => 12,
            Act::Workspaces => 13,
            Act::Find => 14,
            Act::Waiting => 15,
            Act::OverlayClose => 16,
            Act::ListDown => 17,
            Act::ListUp => 18,
            Act::ListTop => 19,
            Act::ListBottom => 20,
            Act::ListFold => 21,
            Act::PaneScrollDown => 22,
            Act::PaneScrollUp => 23,
            Act::PanePageDown => 24,
            Act::PanePageUp => 25,
            Act::PaneNextSubtab => 26,
            Act::PanePrevSubtab => 27,
            Act::ConfirmYes => 28,
            Act::ConfirmNo => 29,
            Act::FormNextField => 30,
            Act::FormPrevField => 31,
            Act::FormSave => 32,
            Act::FormExternalEditor => 33,
            Act::Edit => 34,
            Act::New => 35,
            Act::Delete => 36,
            Act::Clear => 37,
            Act::Reload => 38,
            Act::Back => 39,
            Act::Dismiss => 40,
        }
    }

    #[test]
    fn every_act_is_listed_and_has_a_row() {
        for (index, act) in ALL.iter().enumerate() {
            assert_eq!(position(*act), index, "{act:?} is out of place in ALL");
            assert!(act.spec().is_some(), "{act:?} has no catalogue row");
        }
        assert_eq!(
            ALL.len(),
            CATALOGUE.len(),
            "a row's act is missing from ALL"
        );
    }

    #[test]
    fn global_help_is_question_mark_and_f1() {
        let help = Act::Help.spec().expect("help has a row");
        assert_eq!(help.defaults, ["?", "f1"]);
    }

    #[test]
    fn every_act_has_exactly_one_row() {
        assert_eq!(CATALOGUE.len(), 41);
        let acts: HashSet<Act> = CATALOGUE.iter().map(|row| row.act).collect();
        assert_eq!(acts.len(), CATALOGUE.len(), "an act has two rows");
        for row in CATALOGUE {
            let found = row.act.spec().expect("every act has a row");
            assert!(std::ptr::eq(found, row), "{:?}", row.act);
        }
    }

    #[test]
    fn names_are_unique_per_context() {
        let mut seen = HashSet::new();
        for row in CATALOGUE {
            assert!(
                seen.insert((row.context, row.name)),
                "[{}] {} is named twice",
                row.context.table(),
                row.name
            );
        }
    }

    #[test]
    fn help_is_never_empty() {
        for row in CATALOGUE {
            assert!(!row.help.is_empty(), "{:?}", row.act);
        }
    }

    #[test]
    fn overlay_close_keeps_a_chord() {
        let close = Act::OverlayClose.spec().expect("overlay.close has a row");
        assert!(!close.defaults.is_empty());
    }

    #[test]
    fn no_default_spells_ctrl_c() {
        for row in CATALOGUE {
            for spec in row.defaults {
                let folded: String = spec
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .to_lowercase();
                assert!(
                    !["ctrl-c", "ctrl+c", "control-c"].contains(&folded.as_str()),
                    "{:?} binds {spec:?}",
                    row.act
                );
            }
        }
    }

    #[test]
    fn no_spec_is_shared_in_a_context_unless_state_guarded() {
        let guarded = |a: Act, b: Act| {
            STATE_GUARDED
                .iter()
                .any(|&pair| pair == (a, b) || pair == (b, a))
        };
        for (i, first) in CATALOGUE.iter().enumerate() {
            for second in &CATALOGUE[i + 1..] {
                if first.context != second.context || guarded(first.act, second.act) {
                    continue;
                }
                for spec in first.defaults {
                    assert!(
                        !second.defaults.contains(spec),
                        "{:?} and {:?} share {spec:?}",
                        first.act,
                        second.act
                    );
                }
            }
        }
    }

    #[test]
    fn the_global_block_is_in_status_line_order() {
        let names: Vec<&str> = CATALOGUE
            .iter()
            .filter(|row| row.context == Context::Global)
            .map(|row| row.name)
            .collect();
        assert_eq!(
            names,
            [
                "quit",
                "next_tab",
                "prev_tab",
                "select_tab_1",
                "select_tab_2",
                "select_tab_3",
                "select_tab_4",
                "select_tab_5",
                "select_tab_6",
                "select_tab_7",
                "select_tab_8",
                "select_tab_9",
                "help",
                "workspaces",
                "find",
                "waiting",
            ]
        );
    }

    #[test]
    fn context_tables_and_headings() {
        let table = [
            (Context::Global, "global", "Global"),
            (Context::Overlay, "overlay", "Overlay"),
            (Context::List, "list", "List"),
            (Context::Pane, "pane", "Pane"),
            (Context::Confirm, "confirm", "Confirm"),
            (Context::Form, "form", "Form"),
            (Context::Common, "common", "Common"),
        ];
        for (context, name, heading) in table {
            assert_eq!(context.table(), name);
            assert_eq!(context.heading(), heading);
        }
    }

    #[test]
    fn tab_index_maps_the_nine_digits() {
        let digits = [
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
        for (index, act) in digits.into_iter().enumerate() {
            assert_eq!(act.tab_index(), Some(index), "{act:?}");
            assert_eq!(
                act.spec().expect("a row").defaults,
                [format!("{}", index + 1)]
            );
        }
        assert_eq!(Act::Quit.tab_index(), None);
        assert_eq!(Act::NextTab.tab_index(), None);
    }

    #[test]
    fn only_the_form_and_overlay_close_are_in_capture() {
        let captured: HashSet<Act> = CATALOGUE
            .iter()
            .filter(|row| row.in_capture)
            .map(|row| row.act)
            .collect();
        assert_eq!(
            captured,
            HashSet::from([
                Act::FormNextField,
                Act::FormPrevField,
                Act::FormSave,
                Act::FormExternalEditor,
                Act::OverlayClose,
            ])
        );
    }
}
