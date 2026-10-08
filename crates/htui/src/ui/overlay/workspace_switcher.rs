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
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};
use crate::ui::layout::centered;
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crossterm::event::KeyEvent;

/// Marker in front of the row the cursor is on. Selection is also styled, but the marker is what
/// makes it visible in a snapshot, which records symbols and not styles.
const CURSOR: &str = "> ";

/// The marker's width, in front of every other row, so names stay in one column.
const NO_CURSOR: &str = "  ";

/// The line under the list (MOD-67 D9): `list.down`/`list.up` and `switcher.switch` are handled
/// here, `overlay.close` by the shell.
const HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::SwitcherSwitch, "switch"),
    Hint::One(Act::OverlayClose, "close"),
];

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

    /// The box's contents: one line per workspace, a blank line, then `hint` (the rendered
    /// [`HINT`]).
    fn lines(&self, hint: &str, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = if self.workspaces.is_empty() {
            vec![Line::styled(
                format!("{NO_CURSOR}{}", self.empty_text()),
                theme.dim,
            )]
        } else {
            let column = self
                .workspaces
                .iter()
                .map(|w| cell_width(&w.name))
                .max()
                .unwrap_or(0);
            self.workspaces
                .iter()
                .enumerate()
                .map(|(index, workspace)| self.row(index, workspace, column, theme))
                .collect()
        };
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("{NO_CURSOR}{hint}"), theme.dim));
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
        // `fit`, not `pad`: `column` is the widest name, so nothing is cut, but `fit` clips first
        // and clipping flattens a control to the blank cell `cell_width` counted it as (D3).
        let name = format!("{marker}{}{GAP}", cells::fit(&workspace.name, column));
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

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::SWITCHER)
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Every workspace, not just the one in scope: switching out of the scope is the point.
        vec![StoreRequest::Workspaces]
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::SWITCHER, chord) {
            match act {
                Act::ListDown => {
                    self.down();
                    return Handled::Consumed;
                }
                Act::ListUp => {
                    self.up();
                    return Handled::Consumed;
                }
                Act::SwitcherSwitch => return self.enter(ctx),
                _ => continue, // `overlay.close` or `global.help`: the shell's
            }
        }
        // The shell's overlay step closes on `overlay.close`; the rest is swallowed by
        // `is_modal`, so a stray key never reaches the tab underneath.
        Handled::Pass
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
        let lines = self.lines(&ctx.keys().hint(views::SWITCHER, HINT), ctx.theme);
        let widest = widest(&lines);
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

/// The widest of `lines`, in cells. Measured as the rows were padded: `Line::width` skips the
/// halfwidth sound mark rule, so a name the column pads by cells would overrun a box sized by it
/// (MOD-60 B3).
fn widest(lines: &[Line<'_>]) -> usize {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| cell_width(&span.content))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use htui_core::model::WorkspaceId;

    use super::*;
    use crate::keys::Keys;

    /// The hint rendered with the compiled keys.
    fn hint() -> String {
        Keys::compiled().hint(views::SWITCHER, HINT)
    }

    fn workspace(name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: WorkspaceId::default(),
            slug: "w".to_owned(),
            name: name.to_owned(),
            projects: Vec::new(),
        }
    }

    /// MOD-60: the name column is measured and padded in cells, so a CJK name's project count
    /// lines up with an ASCII one's.
    #[test]
    fn a_wide_workspace_name_keeps_the_counts_in_one_column() {
        let switcher = WorkspaceSwitcher {
            workspaces: vec![workspace("Platform"), workspace("\u{5e73}\u{53f0}")],
            selected: 0,
            loaded: true,
        };
        let lines = switcher.lines(&hint(), &Theme::default());
        let first: Vec<usize> = lines[..2]
            .iter()
            .map(|line| cell_width(&line.spans[0].content))
            .collect();
        assert_eq!(first[0], first[1], "{lines:?}");
    }

    /// MOD-60 D3: `cell_width` counts a control as one cell but ratatui draws nothing for it, so
    /// the name is flattened before it is padded. Measured where ratatui stops drawing each row:
    /// every row ends with the same label, so equal ends mean one count column.
    #[test]
    fn a_control_char_in_a_workspace_name_keeps_the_counts_in_one_column() {
        let switcher = WorkspaceSwitcher {
            workspaces: vec![
                workspace("Platform"),
                workspace("a\tb"),
                workspace("a\u{1}b"),
            ],
            selected: 0,
            loaded: true,
        };
        let lines = switcher.lines(&hint(), &Theme::default());
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 80, 3));
        let ends: Vec<u16> = (0u16..3)
            .map(|y| buffer.set_line(0, y, &lines[usize::from(y)], 80).0)
            .collect();
        assert_eq!(ends, vec![ends[0]; 3], "{lines:?}");
    }

    /// MOD-60 B3: the box is as wide as the widest row in cells. `ｶﾞ` is two cells and one to
    /// `Line::width`, so a box sized that way is narrower than the row padded in cells.
    #[test]
    fn a_halfwidth_workspace_name_fits_inside_the_box() {
        let name = "\u{ff76}\u{ff9e}".repeat(20);
        let switcher = WorkspaceSwitcher {
            workspaces: vec![workspace(&name)],
            selected: 0,
            loaded: true,
        };
        let lines = switcher.lines(&hint(), &Theme::default());
        let row: String = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(
            cell_width(&row),
            2 + 40 + 2 + cell_width(&projects_label(0))
        );
        assert_eq!(widest(&lines), cell_width(&row), "{row:?}");
    }
}
