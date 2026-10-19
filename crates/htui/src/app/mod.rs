//! The shell: state, actions, the update function and the view registrations.

pub mod action;
mod pane;
pub mod state;
pub mod update;

pub use action::{
    Action, Handled, HistoryAction, LayoutAction, OverlayAction, PaneAction, RevealKind,
    RevealTarget, TabAction,
};
// Only tests outside `app` read it (the item form's and the Backlog's claim tests, R1 nit a).
#[cfg(test)]
pub(crate) use pane::MIN_PANE;
pub use state::{App, Ctx, EDITOR_NEEDS_A_TAB, Emit, TopBarState};

use crate::keys::Act;
use crate::ui::overlay::{
    AbortConfirm, ConceptsSearch, LeaveDialog, MigrationPrompt, QueueOverlay, WaitingList,
    WorkspaceSwitcher,
};
use crate::ui::tabs::settings::{
    AgentsSection, BoxesSection, ConnectionSection, HierarchySection, KindsSection,
    PersonasSection, PromptSection, QdrantSection, QueueSection, SecretsSection,
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
///    and global `w` is **offered** ([`App::offer`] with [`Act::Workspaces`], MOD-67 D5) rather
///    than mapped by the shell, because it names an overlay, and the shell must not know which
///    views exist.
/// 3. The switcher is named as the startup overlay. Nothing opens here: the first frame is the
///    bare shell, and the first workspace list either picks a startup scope (`--demo`) or is
///    empty, in which case `App::on_app_reply` opens the switcher reading "no workspaces"
///    instead of leaving a blank shell (blueprint D, "Startup").
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
        // Last (MOD-10 milestone 4, D1): appending moves no existing section's line.
        Box::new(SecretsSection::new()),
        // Last (MOD-12 milestone 2, D8): appending moves no existing section's line.
        Box::new(QueueSection::new()),
    ])));
    app.register_tab(Box::new(ChatTab::new()));

    app.overlay_factories
        .register(WorkspaceSwitcher::ID, || Box::new(WorkspaceSwitcher::new()));
    app.offer(
        Act::Workspaces,
        Action::Overlay(OverlayAction::Open(WorkspaceSwitcher::ID)),
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

    app.startup_overlay = Some(WorkspaceSwitcher::ID);

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
}
