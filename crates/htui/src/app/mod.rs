//! The shell: state, actions, the update function and the view registrations.

pub mod action;
pub mod state;
pub mod update;

pub use action::{Action, Handled, OverlayAction, TabAction};
pub use state::{App, Ctx, Emit, TopBarState};

use crate::ui::tabs::{SettingsTab, SkillsTab};

/// Registers every tab and every overlay factory.
///
/// This is the whole registration surface: MOD-2's Chat tab and MOD-13's filter overlay are one
/// line each here plus their own file, with no change to the event loop (plan D5).
///
/// T6 puts `BacklogTab` first, registers T5's workspace switcher factory and binds global `w`.
pub fn register_all(app: &mut App) {
    app.register_tab(Box::new(SkillsTab::new()));
    app.register_tab(Box::new(SettingsTab::new()));
}
