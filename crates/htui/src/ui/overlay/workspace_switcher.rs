//! The workspace switcher: the only way the scope ever changes (plan D10, blueprint E T5).
//!
//! The overlay asks for [`StoreRequest::Workspaces`] when it is pushed, lists what comes back with
//! each workspace's project count, and turns `Enter` into an
//! [`Action::SetScope`] that the shell applies. It holds no store
//! handle and no channel: data arrives through `on_reply`, effects leave through `Ctx` (`R-NF-3`).
//!
//! Creating a workspace is MOD-15, so an empty store renders "no workspaces" rather than an editor.

use htui_core::model::{Scope, WorkspaceSummary};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::layout::centered;
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crossterm::event::{KeyCode, KeyEvent};

/// Marker in front of the row the cursor is on. Selection is also styled, but the marker is what
/// makes it visible in a snapshot, which records symbols and not styles.
const CURSOR: &str = "> ";

/// The marker's width, in front of every other row, so names stay in one column.
const NO_CURSOR: &str = "  ";

/// The line under the list. `Esc` is the wildcard overlay binding, `j`/`k`/`Enter` are handled here.
const HINT: &str = "j/k move · Enter switch · Esc close";

/// Gap between a workspace's name and its project count.
const GAP: &str = "  ";

/// Left border, right border and one column of right padding.
const CHROME: u16 = 3;

/// Lists the workspaces and enters the selected one.
#[derive(Debug, Default)]
pub struct WorkspaceSwitcher {
    /// What the last [`StoreReply::Workspaces`] carried, in the store's order (by name).
    workspaces: Vec<WorkspaceSummary>,
    /// Index of the highlighted row; `0` while the list is empty.
    selected: usize,
    /// Whether a reply has landed yet: "still reading" and "nothing there" are different screens.
    loaded: bool,
}

impl WorkspaceSwitcher {
    /// Identity of the switcher. T6 registers the factory and the global `w` binding under it.
    pub const ID: OverlayId = OverlayId("workspace_switcher");

    /// A switcher with an empty list; the list arrives with the first reply.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the cursor down one row. The list does not wrap: a switcher is short and a wrapping
    /// cursor makes "am I at the end?" unanswerable without counting.
    fn down(&mut self) {
        if self.selected + 1 < self.workspaces.len() {
            self.selected += 1;
        }
    }

    /// Moves the cursor up one row, stopping at the first.
    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Enters the selected workspace: the scope change is T3's `SetScope` arm, this only emits it.
    ///
    /// The close is emitted too even though entering a workspace already clears every overlay
    /// (`App::set_scope`), so the switcher does not depend on that for its own lifetime.
    fn enter(&self, ctx: &Ctx<'_>) -> Handled {
        if let Some(workspace) = self.workspaces.get(self.selected) {
            ctx.emit(Action::SetScope {
                workspace: workspace.clone(),
            });
            ctx.emit(Action::Overlay(OverlayAction::Close));
        }
        Handled::Consumed
    }

    /// The box's contents: one line per workspace, a blank line, then the hint.
    fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = if self.workspaces.is_empty() {
            vec![Line::styled(
                format!("{NO_CURSOR}{}", self.empty_text()),
                theme.dim,
            )]
        } else {
            let column = self
                .workspaces
                .iter()
                .map(|w| w.name.chars().count())
                .max()
                .unwrap_or(0);
            self.workspaces
                .iter()
                .enumerate()
                .map(|(index, workspace)| self.row(index, workspace, column, theme))
                .collect()
        };
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("{NO_CURSOR}{HINT}"), theme.dim));
        lines
    }

    /// What an empty list says. Never a blank box (plan D11's rule, applied to an overlay).
    fn empty_text(&self) -> &'static str {
        if self.loaded {
            "no workspaces — `N` in Settings > Hierarchy creates one"
        } else {
            "reading the store"
        }
    }

    /// One workspace row: `> Platform  2 projects`, the name padded to the widest one.
    fn row(
        &self,
        index: usize,
        workspace: &WorkspaceSummary,
        column: usize,
        theme: &Theme,
    ) -> Line<'static> {
        let selected = index == self.selected;
        let marker = if selected { CURSOR } else { NO_CURSOR };
        let padding = " ".repeat(column.saturating_sub(workspace.name.chars().count()));
        let name = format!("{marker}{}{padding}{GAP}", workspace.name);
        let style = if selected { theme.accent } else { theme.base };
        Line::from(vec![
            Span::styled(name, style),
            Span::styled(projects_label(workspace.projects.len()), theme.dim),
        ])
    }
}

impl Overlay for WorkspaceSwitcher {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Workspaces"
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Every workspace, not just the one in scope: switching out of the scope is the point.
        vec![StoreRequest::Workspaces]
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.down();
                Handled::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.up();
                Handled::Consumed
            }
            KeyCode::Enter => self.enter(ctx),
            // `Esc` falls through to the wildcard overlay binding; the rest is swallowed by
            // `is_modal`, so a stray key never reaches the tab underneath.
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        let StoreReply::Workspaces(workspaces) = reply else {
            return;
        };
        // The cursor opens on the workspace the shell is already inside, so `Enter` on an
        // untouched switcher is a no-op rather than a jump to whatever sorts first.
        self.selected = workspaces
            .iter()
            .position(|w| w.workspace_id == ctx.scope.workspace_id)
            .unwrap_or(0);
        self.workspaces = workspaces.clone();
        self.loaded = true;
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let lines = self.lines(ctx.theme);
        let widest = lines.iter().map(Line::width).max().unwrap_or(0);
        let width = u16::try_from(widest)
            .unwrap_or(u16::MAX)
            .saturating_add(CHROME);
        let height = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2);
        let box_area = centered(area, width, height);

        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(Span::styled(format!(" {} ", self.title()), ctx.theme.title)),
            ),
            box_area,
        );
    }
}

/// `no projects` / `1 project` / `n projects`, so a row never reads `1 projects`.
fn projects_label(count: usize) -> String {
    match count {
        0 => "no projects".to_owned(),
        1 => "1 project".to_owned(),
        n => format!("{n} projects"),
    }
}
