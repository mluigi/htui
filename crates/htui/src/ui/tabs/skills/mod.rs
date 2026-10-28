//! The Skills tab (MOD-9 PRD D1): a `Skills | Templates` switch.
//!
//! MOD-115 M4 E5: the tab draws its own kit panes (`draws_chrome`). The switch is no line of
//! its own any more: the pane that leads the shown view (the list while browsing, else what
//! covers the content) carries `Skills` and `Templates` as chips on its top border, the
//! active one naming the view ([`view_chips`]). The pane that holds the keyboard is the
//! focused one ([`holder`], M4 C4), and the hints are drawn on it alone (Q2).
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
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::keys::{Act, KeyChord, Stack};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::kit::chips::key_line;
use crate::ui::kit::pane::{Focus, Pane};
use crate::ui::kit::text::wrapped;
use crate::ui::shell::{PaneId, Panes, TabLoc};
use crate::ui::tabs::registry::{Tab, TabId};
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

/// The two views, the chips the leading pane carries (MOD-115 M4 E5).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum View {
    /// The skill library and its attachments (milestone 3). The tab opens here (D34).
    #[default]
    Skills,
    /// The prompt templates (milestone 1).
    Templates,
}

impl View {
    /// Both views, in chip order.
    const ALL: [Self; 2] = [Self::Skills, Self::Templates];

    /// The view's id in a history location (MOD-115 M2 §8).
    const fn id(self) -> &'static str {
        match self {
            Self::Skills => "library",
            Self::Templates => "templates",
        }
    }

    /// The view's name on its chip, for the header's `back to` chip too.
    const fn title(self) -> &'static str {
        VIEWS[self.index()]
    }

    /// The view's chip among [`VIEWS`].
    const fn index(self) -> usize {
        match self {
            Self::Skills => 0,
            Self::Templates => 1,
        }
    }
}

/// The two views' chips, in switch order (MOD-115 M4 E5).
pub(super) const VIEWS: &[&str] = &["Skills", "Templates"];

/// `pane` with the view chips on its top border, `view` (an index into [`VIEWS`]) the active one:
/// the pane that leads the shown view carries them, its title kept first (M4 C5).
pub(super) fn view_chips(pane: Pane<'_>, view: usize) -> Pane<'_> {
    pane.title_first().sub_tabs(VIEWS, view)
}

/// MOD-115 M4 C4: how the pane that holds the keyboard (a prompt, a form, an editor, a pane drawn
/// over the content) is drawn: focused unless something covers the tab, or the shell's activity
/// pane is on screen (`activity`) and has the focus.
pub(super) fn holder(ctx: &Ctx<'_>, activity: bool) -> Focus {
    let focus = ctx.focus();
    if activity && focus == PaneId::Activity {
        Focus::Unfocused
    } else {
        ctx.pane_focus(focus)
    }
}

/// `pane` drawn as `focus` says, in the tab's accent, with `hints` on its bottom border (drawn
/// only while focused).
pub(super) fn framed<'a>(
    pane: Pane<'a>,
    focus: Focus,
    hints: Option<Line<'a>>,
    ctx: &Ctx<'_>,
) -> Pane<'a> {
    let pane = pane.focus(focus).accent(ctx.accent());
    match hints {
        Some(hints) => pane.hints(hints),
        None => pane,
    }
}

/// The shown view's keys as a pane's hint line, `None` when no key of it is bound.
pub(super) fn hint_line(text: String, ctx: &Ctx<'_>) -> Option<Line<'static>> {
    key_line(text, ctx.theme)
}

/// Hint texts joined by ` \u{b7} `, the empty ones (every key unbound) left out.
pub(super) fn joined(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// Where a refused read is fixed: the store's DSN (MOD-115 M4 E5).
const UNAVAILABLE_NEXT: &str = "Settings \u{203a} Connection stores a DSN";

/// A refused read in `area`: the store's sentence wrapped in the error role, then where to fix
/// it in `kit::empty`'s next-step role (the Requirements tab's shape, MOD-115 M3 RV3).
pub(super) fn unavailable(frame: &mut Frame<'_>, area: Rect, sentence: &str, theme: &Theme) {
    let width = usize::from(area.width).max(1);
    let mut lines: Vec<Line<'static>> = wrapped(sentence, width)
        .into_iter()
        .map(|row| Line::styled(row, theme.error))
        .collect();
    lines.push(Line::styled(UNAVAILABLE_NEXT, theme.accent));
    frame.render_widget(Paragraph::new(lines), area);
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

    /// The shown view's keys (MOD-67 M6 T2): its mode's hint through its stack. Since MOD-115
    /// M4 the panes draw their own hints; this is what the shell draws on its focused activity
    /// pane (C8).
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

    /// MOD-115 M2 §8: the shown view (`library` or `templates`, with its title) and its row:
    /// the row's index and, RV3, what it holds (the skill's id, the template's `project:name`).
    fn location(&self) -> TabLoc {
        let cursor = match self.view {
            View::Skills => self.library.location_cursor(),
            View::Templates => self.templates.location_cursor(),
        };
        TabLoc {
            subtab: Some(self.view.id()),
            subtab_title: Some(self.view.title().to_owned()),
            cursor: Some(cursor),
        }
    }

    /// MOD-115 M2 §8: back to a [`location`](Tab::location): its view shown and the cursor on
    /// the skill or template it held, wherever a create, an import or a delete moved it, else
    /// on its row (clamped). Nothing while either view is taking keys: a move would leave a
    /// prompt or an editor behind.
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
        let row = loc.cursor.as_deref().is_some_and(|cursor| match self.view {
            View::Skills => self.library.restore_cursor(cursor),
            View::Templates => self.templates.restore_cursor(cursor),
        });
        target.is_some() || row
    }

    /// MOD-115 M4 E5: the tab draws its own kit panes; the shell paints nothing over them.
    fn draws_chrome(&self) -> bool {
        true
    }

    /// The shown view over the whole body (MOD-115 M4 E5: no switch line; the view chips are on
    /// the pane that leads it).
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        match self.view {
            View::Skills => self.library.render(frame, area, ctx),
            View::Templates => self.templates.render(frame, area, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Modifier;

    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keys::Keys;
    use crate::store_worker::Origin;
    use crate::ui::Theme;

    /// What a `Ctx` borrows, held by the test.
    struct Bench {
        scope: Scope,
        top_bar: TopBarState,
        theme: Theme,
        emit: Emit,
    }

    impl Bench {
        fn new(theme: Theme) -> Self {
            Self {
                scope: Scope {
                    workspace_id: ids::WORKSPACE_GRAPHICS,
                    project_ids: vec![ids::PROJECT_VULKAN],
                },
                top_bar: TopBarState::default(),
                theme,
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &[],
                &self.top_bar,
                Keys::compiled(),
                &self.theme,
                Origin::Tab(SkillsTab::ID),
                &self.emit,
            )
        }
    }

    /// The tab drawn at 100 x 20, as the buffer and its rows.
    fn drawn(tab: &SkillsTab, ctx: &Ctx<'_>) -> (Buffer, Vec<String>) {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("a test terminal");
        terminal
            .draw(|frame| tab.render(frame, frame.area(), ctx))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect();
        (buffer, rows)
    }

    /// The x where `text` starts on `row`, in cells.
    fn at(row: &str, text: &str) -> Option<u16> {
        let cells: Vec<char> = row.chars().collect();
        (0..cells.len())
            .find(|&x| cells[x..].iter().collect::<String>().starts_with(text))
            .map(|x| u16::try_from(x).expect("a narrow row"))
    }

    /// Whether the top-border cell at `x` is drawn as the active chip: bold and underlined,
    /// which reads without colour too.
    fn active(buffer: &Buffer, x: u16) -> bool {
        buffer[(x, 0)]
            .modifier
            .contains(Modifier::BOLD | Modifier::UNDERLINED)
    }

    /// MOD-115 M4 E5: the tab draws its own kit panes, so the shell paints nothing over them.
    #[test]
    fn the_tab_draws_its_own_chrome() {
        assert!(SkillsTab::new().draws_chrome());
    }

    /// MOD-115 M4 E5: no `Skills │ Templates` line: the list pane's top border, the first row,
    /// carries both views as chips, the shown one active (bold and underlined, so it reads
    /// without colour too).
    #[test]
    fn the_view_chips_lead_the_list_and_no_switch_line_is_drawn() {
        for theme in [Theme::default(), Theme::monochrome()] {
            let bench = Bench::new(theme);
            let tab = SkillsTab::new();
            let (buffer, rows) = drawn(&tab, &bench.ctx());
            assert!(rows[0].starts_with('\u{256d}'), "a kit pane: {rows:#?}");
            let skills = at(&rows[0], " Skills ").expect("the Skills chip");
            let templates = at(&rows[0], " Templates ").expect("the Templates chip");
            assert!(
                skills < templates && templates < 32,
                "on the list: {rows:#?}"
            );
            assert!(
                active(&buffer, skills + 1),
                "the shown view is active: {rows:#?}"
            );
            assert!(!active(&buffer, templates + 1), "only the shown view is");
            assert!(
                rows.iter()
                    .all(|row| !row.contains("Skills \u{2502} Templates")),
                "no switch line: {rows:#?}"
            );
        }
    }

    /// MOD-115 M4 E5: `skills.switch_view` (no key change) moves the active chip.
    #[test]
    fn the_view_switch_moves_the_active_chip() {
        let bench = Bench::new(Theme::default());
        let mut tab = SkillsTab::new();
        let switch = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(tab.on_key(switch, &mut bench.ctx()), Handled::Consumed);
        assert_eq!(tab.view, View::Templates);
        let (buffer, rows) = drawn(&tab, &bench.ctx());
        let templates = at(&rows[0], " Templates ").expect("the Templates chip");
        assert!(active(&buffer, templates + 1), "{rows:#?}");
        let skills = at(&rows[0], " Skills ").expect("the Skills chip");
        assert!(!active(&buffer, skills + 1), "{rows:#?}");
    }
}
