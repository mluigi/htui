//! `Settings > Boxes` (MOD-7 milestone 2, PRD D4, `R-TUI-8`): every box of this user with its
//! profile, tools, probed and declared tags, quirks and last probe; declared tags and quirks
//! edited as a compare-and-set a reconnect cannot stale; the probe on this box only.
//!
//! Holds no store handle and mints no id (`R-NF-3`): every `BoxId` it sends came out of a
//! [`BoxesSnapshot`](crate::box_settings::BoxesSnapshot).

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{SectionId, SettingsSection};
use crossterm::event::KeyEvent;

/// The section.
#[derive(Debug, Default)]
pub struct BoxesSection {
    /// Browse, or one editor.
    mode: Mode,
}

/// Browse, or one editor open over one box.
#[derive(Debug, Default)]
enum Mode {
    /// No editor open.
    #[default]
    Browse,
}

impl BoxesSection {
    /// Stable identity (plan D47, OQ-19).
    pub const ID: SectionId = SectionId("boxes");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SettingsSection for BoxesSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Boxes"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Boxes are the user's, not the workspace's: the same read whatever the scope.
        vec![StoreRequest::Boxes]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
}
