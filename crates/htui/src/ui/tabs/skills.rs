//! The Skills tab: a stub until MOD-12 fills it.

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::registry::{Tab, TabId};
use crossterm::event::KeyEvent;

/// Placeholder screen: the tab exists so the strip, the `1`..`9` bindings and the registry are
/// exercised by more than one entry before MOD-12 lands.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkillsTab;

impl SkillsTab {
    /// Identity of the Skills tab.
    pub const ID: TabId = TabId("skills");

    /// A new stub.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Tab for SkillsTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Skills"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let body = Text::from(vec![Line::styled(
            "Skill registry and capability tags arrive with MOD-12.",
            ctx.theme.dim,
        )]);
        frame.render_widget(
            Paragraph::new(body).block(Block::new().borders(Borders::ALL).title(" Skills ")),
            area,
        );
    }
}
