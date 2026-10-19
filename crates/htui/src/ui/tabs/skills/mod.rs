//! The Skills tab (MOD-9 PRD D1): a `Skills | Templates` switch.
//!
//! The Skills view (`library`, with its attachments pane in `attach`) is MOD-9 milestone 3: the
//! skill library, any version's body, a line diff between two versions, an editor with a token
//! estimate, and one skill's global, project and phase attachments (plan D82-D84). The Templates
//! view (`templates`) is milestone 1: the scope's prompt templates, their versions, a line diff,
//! and an editor that saves through `parse`. The tab reads both on activation whichever view is
//! shown (blueprint D34, plan D84), so switching views needs no request.

mod agent_help;
mod attach;
mod library;
mod templates;

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::keys::{Act, KeyChord, Stack};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::shell::{Panes, TabLoc};
use crate::ui::tabs::registry::{Tab, TabId};
use crate::ui::tabs::settings::key_line;
use crossterm::event::KeyEvent;

use library::LibraryView;
use templates::TemplatesView;

/// The Skills tab (MOD-9 PRD D1): `Skills | Templates`.
#[derive(Debug, Default)]
pub struct SkillsTab {
    /// Which of the two views is shown.
    view: View,
    /// The Templates view, alive whichever view is shown: its replies land either way.
    templates: TemplatesView,
    /// The Skills view (milestone 3), alive whichever view is shown, for the same reason.
    library: LibraryView,
}

/// The two views of the switch line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum View {
    /// The skill library and its attachments (milestone 3). The tab opens here (D34).
    #[default]
    Skills,
    /// The prompt templates (milestone 1).
    Templates,
}

impl View {
    /// Both views, in switch-line order.
    const ALL: [Self; 2] = [Self::Skills, Self::Templates];

    /// The view's id in a history location (MOD-115 M2 §8).
    const fn id(self) -> &'static str {
        match self {
            Self::Skills => "library",
            Self::Templates => "templates",
        }
    }

    /// The view's name on the switch line, for the header's `back to` chip.
    const fn title(self) -> &'static str {
        match self {
            Self::Skills => "Skills",
            Self::Templates => "Templates",
        }
    }
}

impl SkillsTab {
    /// Identity of the Skills tab.
    pub const ID: TabId = TabId("skills");

    /// A tab on the Skills view, nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The shown view's stack: [`Tab::key_stack`]'s, which always has one.
    fn stack(&self) -> Stack<'static> {
        match self.view {
            View::Skills => self.library.key_stack(),
            View::Templates => self.templates.key_stack(),
        }
    }

    /// Whether the shown view, or `view`, is taking every key: a prompt or an editor is open.
    fn captures(&self, view: View) -> bool {
        [self.view, view].into_iter().any(|view| match view {
            View::Skills => self.library.captures_input(),
            View::Templates => self.templates.captures_input(),
        })
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
        vec![
            StoreRequest::Templates(scope.clone()),
            StoreRequest::Skills(scope.clone()),
        ]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        self.templates.on_scope_change();
        self.library.on_scope_change();
    }

    /// MOD-22 review M-1: a bracketed paste into the active view's open prompt, form or editor.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        let taken = match self.view {
            View::Skills => self.library.on_paste(text),
            View::Templates => self.templates.on_paste(text),
        };
        if taken {
            Handled::Consumed
        } else {
            Handled::Pass
        }
    }

    /// The shown view's stack (MOD-67 M4 D4), always one: the Library's (the attachments pane's
    /// while it is open, else an open agent help's, else its mode's) or the Templates view's (an
    /// open agent help's, else its mode's).
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    /// The shown view's browse stack, whatever prompt or editor is open (MOD-115 review): what
    /// the about panel labels the tab's actions through.
    fn browse_stack(&self) -> Stack<'static> {
        match self.view {
            View::Skills => LibraryView::browse_stack(),
            View::Templates => TemplatesView::browse_stack(),
        }
    }

    /// The view switch is `skills.switch_view` (MOD-67 M4 D5), resolved through the shown view's
    /// stack. Chord equality includes modifiers, so `ctrl-l`, `ctrl-h` or `alt-l` no longer
    /// switch (ANA-26 §2.6 defect 1), and plain `right` no longer does either (MOD-115 M2 D4:
    /// arrows stay in the pane). Every other key goes to the shown view.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A capturing mode answers first: `l` is a letter there (its stack has no `skills`
        // switch either, D5).
        let captured = match self.view {
            View::Skills => self.library.captures_input(),
            View::Templates => self.templates.captures_input(),
        };
        if !captured
            && ctx
                .keys()
                .actions(self.stack(), KeyChord::from_event(key))
                .first()
                == Some(&Act::SkillsSwitchView)
        {
            self.toggle();
            return Handled::Consumed;
        }
        match self.view {
            View::Skills => self.library.on_key(key, ctx),
            View::Templates => self.templates.on_key(key, ctx),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        self.templates.on_reply(reply, ctx);
        self.library.on_reply(reply, ctx);
    }

    /// Both views may have handed off to `$EDITOR`; each ignores an outcome it did not ask for
    /// (blueprint D100, F-N).
    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        self.templates.on_external_edit(outcome.clone(), ctx);
        self.library.on_external_edit(outcome, ctx);
    }

    /// The shown view's keys (MOD-67 M6 T2): its mode's hint through its stack.
    fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        let hint = match self.view {
            View::Skills => self.library.key_hint(ctx.keys()),
            View::Templates => self.templates.key_hint(ctx.keys()),
        };
        key_line(hint, ctx.theme)
    }

    /// MOD-115 M2 D2: the list and the pane, the 32-cell list both views drew by default.
    fn panes(&self) -> Panes {
        Panes::ListDetail(library::LAYOUT)
    }

    /// MOD-115 M2 §8: the shown view (`library` or `templates`, with its title) and its row.
    fn location(&self) -> TabLoc {
        let cursor = match self.view {
            View::Skills => self.library.cursor(),
            View::Templates => self.templates.cursor(),
        };
        TabLoc {
            subtab: Some(self.view.id()),
            subtab_title: Some(self.view.title().to_owned()),
            cursor: Some(cursor.to_string()),
        }
    }

    /// MOD-115 M2 §8: back to a [`location`](Tab::location): its view shown and its row under
    /// the cursor. Nothing while either view is taking keys: a move would leave a prompt or an
    /// editor behind.
    fn restore(&mut self, loc: &TabLoc, _ctx: &mut Ctx<'_>) -> bool {
        let target = View::ALL
            .into_iter()
            .find(|view| loc.subtab == Some(view.id()));
        if self.captures(target.unwrap_or(self.view)) {
            return false;
        }
        if let Some(view) = target {
            self.view = view;
        }
        let row = loc
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.parse().ok())
            .is_some_and(|index| match self.view {
                View::Skills => self.library.restore_cursor(index),
                View::Templates => self.templates.restore_cursor(index),
            });
        target.is_some() || row
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
