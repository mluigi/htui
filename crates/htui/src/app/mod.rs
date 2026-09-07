//! The shell: state, actions, the update function and the view registrations.

pub mod action;
pub mod state;
pub mod update;

pub use action::{Action, Handled, OverlayAction, TabAction};
pub use state::{App, Ctx, Emit, TopBarState};

use crossterm::event::{KeyCode, KeyModifiers};

use crate::keymap::{Binding, KeyChord, KeyScope};
use crate::ui::overlay::{MigrationPrompt, WorkspaceSwitcher};
use crate::ui::tabs::settings::AgentsSection;
use crate::ui::tabs::{BacklogTab, SettingsTab, SkillsTab};

/// Registers every tab and every overlay factory, and names the workspace switcher as the
/// startup overlay.
///
/// This is the whole registration surface: MOD-2's Chat tab and MOD-13's filter overlay are one
/// line each here plus their own file, with no change to the event loop (plan D5).
///
/// Three things happen, in this order:
///
/// 1. Backlog, Skills and Settings are registered. Registration order is tab-strip order and
///    `1`..`9` order, so Backlog first is what makes it the tab a user lands on.
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
///
/// Calling this twice would stack a second switcher; the shell calls it exactly once, between
/// [`App::new`] and [`App::start`].
pub fn register_all(app: &mut App) {
    app.register_tab(Box::new(BacklogTab::new()));
    app.register_tab(Box::new(SkillsTab::new()));
    app.register_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
        AgentsSection::new(),
    )])));

    app.overlay_factories
        .register(WorkspaceSwitcher::ID, || Box::new(WorkspaceSwitcher::new()));
    app.keymap.bind(Binding {
        scope: KeyScope::Global,
        key: KeyChord::new(KeyCode::Char('w'), KeyModifiers::NONE),
        action: Action::Overlay(OverlayAction::Open(WorkspaceSwitcher::ID)),
        help: "workspaces",
    });

    app.startup_overlay = Some(WorkspaceSwitcher::ID);

    app.overlay_factories
        .register(MigrationPrompt::ID, || Box::new(MigrationPrompt::new()));
    app.migration_overlay = Some(MigrationPrompt::ID);
}
