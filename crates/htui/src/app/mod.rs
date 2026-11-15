//! The shell: state, actions, the update function and the view registrations.

pub mod action;
mod pane;
pub mod state;
pub mod update;

pub use action::{
    Action, Handled, HistoryAction, LayoutAction, LoggingAction, OverlayAction, PaneAction,
    RevealKind, RevealTarget, TabAction,
};
// Only tests outside `app` read it (the item form's and the Backlog's claim tests, R1 nit a).
#[cfg(test)]
pub(crate) use pane::MIN_PANE;
pub use state::{App, Ctx, EDITOR_NEEDS_A_TAB, Emit, TopBarState};

use crate::keys::Act;
use crate::ui::overlay::{
    AbortConfirm, ConceptsSearch, LeaveDialog, MigrationPrompt, QueueOverlay, WaitingList,
};
use crate::ui::tabs::settings::{
    AgentsSection, BoxesSection, ConnectionSection, HierarchySection, KindsSection, LoggingSection,
    PersonasSection, ProjectTypesSection, PromptSection, QdrantSection, QueueSection,
    SecretsSection,
};
use crate::ui::tabs::{
    BacklogTab, ChatTab, LogsTab, RequirementsTab, SettingsTab, SkillsTab, WorkspacesTab,
};

/// Registers every tab and every overlay factory, and names the Workspaces tab as the tab an
/// empty store lands on.
///
/// This is the whole registration surface: MOD-2's Chat tab and MOD-13's filter overlay are one
/// line each here plus their own file, with no change to the event loop (plan D5).
///
/// In this order:
///
/// 1. Workspaces, Backlog, Skills, Requirements and Settings are registered. Registration order
///    is tab-strip order and `1`..`9` order, so Workspaces is tab `1` and Backlog `2` (MOD-116 M1
///    D11). Requirements sits before Settings, `R-TUI-1`'s order (MOD-39 PRD D2). Chat follows
///    them and, last, Logs (MOD-122 D4).
/// 2. Global `w` is **offered** ([`App::offer`] with [`Act::Workspaces`], MOD-67 D5) as the focus
///    of the Workspaces tab rather than mapped by the shell, because it names a view, and the
///    shell must not know which views exist (MOD-116 M1 D10; it replaced the workspace switcher).
/// 3. The Workspaces tab is named as the empty tab ([`App::empty_tab`]). Nothing opens here: the
///    first workspace list either opens the last workspace, else the first (MOD-116 M1 D12), or
///    is empty, in which case `App::on_app_reply` lands on the Workspaces tab, which says there
///    are no workspaces and how to create one (blueprint D, "Startup").
/// 4. The migration prompt's factory goes in and it is named as the migration overlay. It has no
///    key binding on purpose: it is opened by the shell when a `StoreState` reply reports a
///    pending schema, never by the user (`R-STO-5`, plan D11).
/// 5. The Chat tab is named as the replay tab (MOD-2 D39); the Backlog answers a missed `Enter`
///    itself (`backlog.replay`, MOD-67 M5 D6).
/// 6. The Backlog and Requirements tabs are named as the tabs that reveal items and requirements
///    (MOD-64 D235).
/// 7. The concepts search's factory goes in and global `Ctrl+F` is offered as [`Act::Find`] to
///    open it (MOD-64 D236), a chord so no tab's letters and no text field's input can take it.
/// 8. The waiting list's factory goes in and global `Ctrl+W` is offered as [`Act::Waiting`] to
///    open it (MOD-69 D7), a chord for `Ctrl+F`'s reason.
/// 9. The queue overlay's factory goes in and global `Ctrl+Q` is offered as [`Act::Queue`] to
///    open it (MOD-12 M3 D8), a chord for the same reason.
/// 10. The leave dialog's factory goes in and it is named as the leave overlay (MOD-115 M2 D6,
///     `R-TUI-18`; it replaced MOD-57 M2 E4's quit question): `global.quit` (`q`, `ctrl-c`)
///     opens it on every screen, and its `leave.confirm` (`Enter`, a second `ctrl-c`) leaves.
/// 11. `App::start` is told to ask for the Qdrant snapshot, so an old keyring Qdrant URL is said
///     once on the status line (MOD-158 D10).
/// 12. Last, the shell lands on the Backlog ([`App::land_on`], no history place), so a restart
///     still opens on the tab it always did (MOD-116 M1 D11).
///
/// Calling this twice would register every tab twice; the shell calls it exactly once, between
/// [`App::new`] and [`App::start`].
pub fn register_all(app: &mut App) {
    // MOD-116 M1 D11: first, so it is tab `1`; every other tab's digit moved up by one.
    app.register_tab(Box::new(WorkspacesTab::new()));
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
        // Last (MOD-10 milestone 4, D1): appending moves no existing section's line.
        Box::new(SecretsSection::new()),
        // Last (MOD-12 milestone 2, D8): appending moves no existing section's line.
        Box::new(QueueSection::new()),
        // Last (MOD-146 D8): appending moves no existing section's line.
        Box::new(ProjectTypesSection::new()),
        // Last (MOD-122 D5): appending moves no existing section's line.
        Box::new(LoggingSection::new(app.logging.control().clone())),
    ])));
    app.register_tab(Box::new(ChatTab::new()));
    // Last (MOD-122 D4): `7` since MOD-116 M1 D11. The buffer is the one `run` installed.
    app.register_tab(Box::new(LogsTab::new(app.logging.buffer().clone())));

    // MOD-116 M1 D10: `w` focuses the Workspaces tab (the switcher it opened is gone).
    app.offer(
        Act::Workspaces,
        Action::Tab(TabAction::Focus(WorkspacesTab::ID)),
    );

    // MOD-64 D236: the concepts search, global `Ctrl+F`. A chord, so no tab's letters and no text
    // field's input can take it (every text widget passes chords, blueprint F7).
    app.overlay_factories
        .register(ConceptsSearch::ID, || Box::new(ConceptsSearch::new()));
    app.offer(
        Act::Find,
        Action::Overlay(OverlayAction::Open(ConceptsSearch::ID)),
    );

    // MOD-69 D7: the waiting-on-you list, global `Ctrl+W`. A chord, for `Ctrl+F`'s reason.
    app.overlay_factories
        .register(WaitingList::ID, || Box::new(WaitingList::new()));
    app.offer(
        Act::Waiting,
        Action::Overlay(OverlayAction::Open(WaitingList::ID)),
    );

    // MOD-12 M3 D8: the queue overlay, global `Ctrl+Q`. A chord, for `Ctrl+F`'s reason.
    app.overlay_factories
        .register(QueueOverlay::ID, || Box::new(QueueOverlay::new()));
    app.offer(
        Act::Queue,
        Action::Overlay(OverlayAction::Open(QueueOverlay::ID)),
    );

    // MOD-116 M1 D11: an empty first workspace list lands on the Workspaces tab.
    app.empty_tab = Some(WorkspacesTab::ID);

    app.overlay_factories
        .register(MigrationPrompt::ID, || Box::new(MigrationPrompt::new()));
    app.migration_overlay = Some(MigrationPrompt::ID);

    // MOD-115 M2 D6: the leave dialog, an id for the migration prompt's reason.
    app.overlay_factories
        .register(LeaveDialog::ID, || Box::new(LeaveDialog::new()));
    app.leave_overlay = Some(LeaveDialog::ID);
    // MOD-57 M2 R1 M-2: the abort question, for the same reason.
    app.overlay_factories
        .register(AbortConfirm::ID, || Box::new(AbortConfirm::new()));
    app.abort_overlay = Some(AbortConfirm::ID);

    app.replay_tab = Some(ChatTab::ID);
    // MOD-64 D235: which tab selects a revealed item or requirement. Ids, not views, for the reason
    // `replay_tab` is one.
    app.reveal_tabs = vec![
        (RevealKind::Item, BacklogTab::ID),
        (RevealKind::Requirement, RequirementsTab::ID),
    ];
    // MOD-158 D10 (blueprint P-6): `App::start` asks whether this box's keyring holds the old
    // Qdrant URL, so the status line can say once that Settings > Qdrant offers it.
    app.qdrant_offer_check = true;
    // MOD-116 M1 D11: a restart still opens on the Backlog, with no history place recorded.
    app.land_on(BacklogTab::ID);
}
