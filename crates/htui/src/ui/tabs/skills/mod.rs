//! The Skills tab (MOD-9 PRD D1): a `Skills | Templates` switch.
//!
//! The Templates view (`templates`) is MOD-9 milestone 1: the scope's prompt templates, their
//! versions, a line diff, and an editor that saves through `parse`. The Skills view is milestone
//! 3's and says so. The tab reads the templates on activation whichever view is shown (blueprint
//! D34), so switching views needs no request.

mod templates;

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::registry::{Tab, TabId};
use crossterm::event::{KeyCode, KeyEvent};

use templates::TemplatesView;

/// What the Skills view says until milestone 3 fills it (plan D14; the stub's MOD-12 attribution
/// was wrong, PRD "Record corrections").
const SKILLS_LATER: &str = "Skills are edited here from MOD-9 milestone 3.";

/// The Skills tab (MOD-9 PRD D1): `Skills | Templates`. The Skills view is milestone 3's.
#[derive(Debug, Default)]
pub struct SkillsTab {
    /// Which of the two views is shown.
    view: View,
    /// The Templates view, alive whichever view is shown: its replies land either way.
    templates: TemplatesView,
}

/// The two views of the switch line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum View {
    /// Milestone 3's; one line until then. The tab opens here (D34).
    #[default]
    Skills,
    /// The prompt templates (milestone 1).
    Templates,
}

impl SkillsTab {
    /// Identity of the Skills tab.
    pub const ID: TabId = TabId("skills");

    /// A tab on the Skills view, nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The other view.
    fn toggle(&mut self) {
        self.view = match self.view {
            View::Skills => View::Templates,
            View::Templates => View::Skills,
        };
    }
}

impl Tab for SkillsTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Skills"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::Templates(scope.clone())]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        self.templates.on_scope_change();
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // An open editor or name prompt owns every key it uses: `l` is a letter there, not a view
        // switch (the chat composer's and the Settings sections' rule).
        if self.view == View::Templates && self.templates.captures_input() {
            return self.templates.on_key(key, ctx);
        }
        match key.code {
            KeyCode::Char('h' | 'l' | '[' | ']') | KeyCode::Left | KeyCode::Right => {
                self.toggle();
                Handled::Consumed
            }
            _ if self.view == View::Templates => self.templates.on_key(key, ctx),
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        self.templates.on_reply(reply, ctx);
    }

    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        self.templates.on_external_edit(outcome, ctx);
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let [switch, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        let style = |view: View| {
            if self.view == view {
                ctx.theme.title
            } else {
                ctx.theme.dim
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Skills ", style(View::Skills)),
                Span::styled("\u{2502}", ctx.theme.dim),
                Span::styled(" Templates ", style(View::Templates)),
            ])),
            switch,
        );
        match self.view {
            View::Skills => frame.render_widget(
                Paragraph::new(Line::styled(format!(" {SKILLS_LATER}"), ctx.theme.dim)),
                body,
            ),
            View::Templates => self.templates.render(frame, body, ctx),
        }
    }
}
