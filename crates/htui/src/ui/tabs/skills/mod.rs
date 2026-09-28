//! The Skills tab (MOD-9 PRD D1): a `Skills | Templates` switch.
//!
//! The Templates view (`templates`) is MOD-9 milestone 1: the scope's prompt templates, their
//! versions, a line diff, and an editor that saves through `parse`. The Skills view (`library`) is
//! milestone 3's: the global skill library, its versions, a line diff, and an editor that saves a
//! markdown body behind two compare-and-set tokens.
//!
//! The tab reads **both** on activation whichever view is shown, so switching views needs no
//! request and no view is left holding a snapshot from a workspace it has left. The switch line
//! and the strip text belong to the shell and are byte-identical to what milestone 1 shipped.

mod library;
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

use library::SkillsView;
use templates::TemplatesView;

/// The Skills tab (MOD-9 PRD D1): `Skills | Templates`.
#[derive(Debug, Default)]
pub struct SkillsTab {
    /// Which of the two views is shown.
    view: View,
    /// The Templates view, alive whichever view is shown: its replies land either way.
    templates: TemplatesView,
    /// The Skills view, alive whichever view is shown, for the same reason.
    library: SkillsView,
}

/// The two views of the switch line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum View {
    /// The skill library (milestone 3). The tab opens here (D34).
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

    /// The shown view's own input guard.
    ///
    /// **Per view, not one flag**: each holds its own `captures_input`, and a global one would
    /// lock the tab's `h`/`l`/`[`/`]` out while the *other* view's editor is open — which cannot
    /// happen, because the other view's editor is not drawn — and would miss the case it is for,
    /// which is that `l` is a letter inside a form, not a view switch.
    fn capturing(&self) -> bool {
        match self.view {
            View::Skills => self.library.captures_input(),
            View::Templates => self.templates.captures_input(),
        }
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
        // Two variants, one each. **Never** one variant per project: the staleness index keeps
        // only the newest request of a kind, so N of one variant would leave all but one undrawn
        // (F-2, the `Catalogue(Scope)` doc). Two of two is fine (D109, F-12).
        vec![
            StoreRequest::Templates(scope.clone()),
            StoreRequest::Skills(scope.clone()),
        ]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        self.templates.on_scope_change();
        self.library.on_scope_change();
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // An open editor or form owns every key it uses: `l` is a letter there, not a view
        // switch (the chat composer's and the Settings sections' rule).
        if self.capturing() {
            return match self.view {
                View::Skills => self.library.on_key(key, ctx),
                View::Templates => self.templates.on_key(key, ctx),
            };
        }
        match key.code {
            KeyCode::Char('h' | 'l' | '[' | ']') | KeyCode::Left | KeyCode::Right => {
                self.toggle();
                Handled::Consumed
            }
            _ => match self.view {
                View::Skills => self.library.on_key(key, ctx),
                View::Templates => self.templates.on_key(key, ctx),
            },
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        self.templates.on_reply(reply, ctx);
        self.library.on_reply(reply, ctx);
    }

    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        self.templates.on_external_edit(outcome.clone(), ctx);
        self.library.on_external_edit(outcome, ctx);
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
            View::Skills => self.library.render(frame, body, ctx),
            View::Templates => self.templates.render(frame, body, ctx),
        }
    }
}
