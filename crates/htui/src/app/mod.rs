//! The shell: state, actions, the update function and the view registrations.

pub mod action;
pub mod state;
pub mod update;

pub use action::{Action, Handled, OverlayAction, RevealKind, RevealTarget, TabAction};
pub use state::{App, Ctx, EDITOR_NEEDS_A_TAB, Emit, TopBarState};

use crossterm::event::{KeyCode, KeyModifiers};

use crate::keymap::{Binding, KeyChord, KeyScope};
use crate::ui::overlay::{ConceptsSearch, MigrationPrompt, WaitingList, WorkspaceSwitcher};
use crate::ui::tabs::settings::{
    AgentsSection, BoxesSection, ConnectionSection, HierarchySection, KindsSection,
    PersonasSection, PromptSection, QdrantSection,
};
use crate::ui::tabs::{BacklogTab, ChatTab, RequirementsTab, SettingsTab, SkillsTab};

/// Registers every tab and every overlay factory, and names the workspace switcher as the
/// startup overlay.
///
/// This is the whole registration surface: MOD-2's Chat tab and MOD-13's filter overlay are one
/// line each here plus their own file, with no change to the event loop (plan D5).
///
/// Three things happen, in this order:
///
/// 1. Backlog, Skills, Requirements and Settings are registered. Registration order is tab-strip
///    order and `1`..`9` order, so Backlog first is what makes it the tab a user lands on.
///    Requirements sits before Settings, `R-TUI-1`'s order (MOD-39 PRD D2).
/// 2. The switcher's factory goes in under its own [`OverlayId`](crate::ui::overlay::OverlayId)
///    and global `w` is bound to opening it. The binding is added here rather than in
///    [`Keymap::default_global`](crate::keymap::Keymap::default_global) because it names an
///    overlay, and the key table must not know which views exist.
/// 3. The switcher is named as the startup overlay. Nothing opens here: the first frame is the
///    bare shell, and the first workspace list either picks a startup scope (`--demo`) or is
///    empty, in which case `App::on_app_reply` opens the switcher reading "no workspaces"
///    instead of leaving a blank shell (blueprint D, "Startup").
/// 4. The migration prompt's factory goes in and it is named as the migration overlay. It has no
///    key binding on purpose: it is opened by the shell when a `StoreState` reply reports a
///    pending schema, never by the user (`R-STO-5`, plan D11).
/// 5. The Chat tab is named as the replay tab and the Backlog tab's `Enter` is bound to the
///    refusal that says how to reach a replay (MOD-2 D39). Both name a concrete view, so both
///    belong here for the same reason the `w` binding does.
/// 6. The Backlog and Requirements tabs are named as the tabs that reveal items and requirements
///    (MOD-64 D235).
/// 7. The concepts search's factory goes in and global `Ctrl+F` is bound to opening it (MOD-64
///    D236), a chord so no tab's letters and no text field's input can take it.
/// 8. The Backlog tab's `m` is bound to its `open graph` help text (MOD-14 D8). The tab's own
///    arm selects the Graph sub-tab; the binding only puts `m open graph` on the help line.
/// 9. The Backlog tab's `f` and `F` are bound to their `filter` and `clear filter` help texts
///    (MOD-13 D1), for the same reason: the tab's own arms open and clear the filter.
/// 10. The Backlog tab's `N` and `e` are bound to their `new item` and `edit item` help texts
///     (MOD-13 milestone 2 D7), for the same reason: the tab's own arms read the item form.
/// 11. The waiting list's factory goes in and global `Ctrl+W` is bound to opening it (MOD-69 D7),
///     a chord for `Ctrl+F`'s reason.
///
/// Calling this twice would stack a second switcher; the shell calls it exactly once, between
/// [`App::new`] and [`App::start`].
pub fn register_all(app: &mut App) {
    app.register_tab(Box::new(BacklogTab::new()));
    app.register_tab(Box::new(SkillsTab::new()));
    app.register_tab(Box::new(RequirementsTab::new()));
    app.register_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
        Box::new(KindsSection::new()),
        Box::new(PromptSection::new()),
        // Last (MOD-15 M6 D19): registration order is strip order, and appending moves no
        // existing section's line.
        Box::new(ConnectionSection::new()),
        Box::new(QdrantSection::new()),
        // Last (MOD-7 milestone 2, D47): appending moves no existing section's line.
        Box::new(BoxesSection::new()),
        // Last (MOD-26 milestone 2, D22): appending moves no existing section's line.
        Box::new(PersonasSection::new()),
    ])));
    app.register_tab(Box::new(ChatTab::new()));

    app.overlay_factories
        .register(WorkspaceSwitcher::ID, || Box::new(WorkspaceSwitcher::new()));
    app.keymap.bind(Binding {
        scope: KeyScope::Global,
        key: KeyChord::new(KeyCode::Char('w'), KeyModifiers::NONE),
        action: Action::Overlay(OverlayAction::Open(WorkspaceSwitcher::ID)),
        help: "workspaces",
    });

    // MOD-64 D236: the concepts search, global `Ctrl+F`. A chord, so no tab's letters and no text
    // field's input can take it (every text widget passes chords, blueprint F7).
    app.overlay_factories
        .register(ConceptsSearch::ID, || Box::new(ConceptsSearch::new()));
    app.keymap.bind(Binding {
        scope: KeyScope::Global,
        key: KeyChord::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        action: Action::Overlay(OverlayAction::Open(ConceptsSearch::ID)),
        help: "find",
    });

    // MOD-69 D7: the waiting-on-you list, global `Ctrl+W`. A chord, for `Ctrl+F`'s reason; MOD-67
    // makes it a named action.
    app.overlay_factories
        .register(WaitingList::ID, || Box::new(WaitingList::new()));
    app.keymap.bind(Binding {
        scope: KeyScope::Global,
        key: KeyChord::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        action: Action::Overlay(OverlayAction::Open(WaitingList::ID)),
        help: "waiting",
    });

    app.startup_overlay = Some(WorkspaceSwitcher::ID);

    app.overlay_factories
        .register(MigrationPrompt::ID, || Box::new(MigrationPrompt::new()));
    app.migration_overlay = Some(MigrationPrompt::ID);

    app.replay_tab = Some(ChatTab::ID);
    // MOD-64 D235: which tab selects a revealed item or requirement. Ids, not views, for the reason
    // `replay_tab` is one.
    app.reveal_tabs = vec![
        (RevealKind::Item, BacklogTab::ID),
        (RevealKind::Requirement, RequirementsTab::ID),
    ];
    // The Runs pane consumes `Enter` when it has a step under the cursor and emits
    // `Action::Replay` with it; the payload is the selection, which a static binding cannot
    // carry. This row is therefore the *miss*: it fires only when no pane took the key, and then
    // it says what to press instead. It also puts `Enter replay step` on the Backlog tab's help
    // line, which is the other half of what a binding is for.
    app.keymap.bind(Binding {
        scope: KeyScope::Tab(BacklogTab::ID),
        key: KeyChord::new(KeyCode::Enter, KeyModifiers::NONE),
        action: Action::Error("select a step in the Runs pane (J/K) to replay it".to_owned()),
        help: "replay step",
    });
    // MOD-14 D8: `m` opens the Graph sub-tab. The Backlog's own arm does it (no `Action` selects a
    // sub-tab) and always consumes the key, so this row never fires: it is the help box's half, as
    // the `Enter` row is. Its action is a no-op focus of the tab that is already active.
    app.keymap.bind(Binding {
        scope: KeyScope::Tab(BacklogTab::ID),
        key: KeyChord::new(KeyCode::Char('m'), KeyModifiers::NONE),
        action: Action::Tab(TabAction::Focus(BacklogTab::ID)),
        help: "open graph",
    });
    // MOD-13 D1: `f` opens the filter form and `F` clears the filter, in the Backlog's own arms,
    // which always consume the key; these rows are the help box's half, as the `m` row is.
    for (key, help) in [('f', "filter"), ('F', "clear filter")] {
        app.keymap.bind(Binding {
            scope: KeyScope::Tab(BacklogTab::ID),
            key: KeyChord::new(KeyCode::Char(key), KeyModifiers::NONE),
            action: Action::Tab(TabAction::Focus(BacklogTab::ID)),
            help,
        });
    }
    // MOD-13 milestone 2 D7: `N` opens the new-item form and `e` the edit form, in the Backlog's
    // own arms, which always consume the key; these rows are the help box's half, as the `f` row
    // is.
    for (key, help) in [('N', "new item"), ('e', "edit item")] {
        app.keymap.bind(Binding {
            scope: KeyScope::Tab(BacklogTab::ID),
            key: KeyChord::new(KeyCode::Char(key), KeyModifiers::NONE),
            action: Action::Tab(TabAction::Focus(BacklogTab::ID)),
            help,
        });
    }
    // MOD-12 D9: `Q` toggles the cursor item's queue membership and `P` this box's queue, in the
    // Backlog's own arms, which always consume the key; these rows are the help box's half.
    for (key, help) in [('Q', "queue / dequeue"), ('P', "pause / resume queue")] {
        app.keymap.bind(Binding {
            scope: KeyScope::Tab(BacklogTab::ID),
            key: KeyChord::new(KeyCode::Char(key), KeyModifiers::NONE),
            action: Action::Tab(TabAction::Focus(BacklogTab::ID)),
            help,
        });
    }
}
