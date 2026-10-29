//! The workspace switcher: the only way the scope ever changes (plan D10, blueprint E T5).
//!
//! The overlay asks for [`StoreRequest::Workspaces`] when it is pushed, lists what comes back with
//! each workspace's project count, and turns `Enter` into an
//! [`Action::SetScope`] that the shell applies. It holds no store
//! handle and no channel: data arrives through `on_reply`, effects leave through `Ctx` (`R-NF-3`).
//!
//! Creating a workspace is MOD-15, so an empty store renders "no workspaces" rather than an editor.
//!
//! Drawn on the MOD-115 kit (M4 E6): [`Modal`] is the frame, `kit::list` the rows (the `▌` gutter
//! and the selection band mark the cursor, MOD-85), `kit::empty` the reading and empty states, and
//! the keys sit on the modal's bottom border.

use htui_core::model::{Scope, WorkspaceSummary};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::label::plural;
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::modal::{Modal, ModalSize};
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crossterm::event::KeyEvent;

/// The keys on the bottom border (MOD-67 D9): `list.down`/`list.up` and `switcher.switch` are
/// handled here, `overlay.close` by the shell.
const HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::SwitcherSwitch, "switch"),
    Hint::One(Act::OverlayClose, "close"),
];

/// Gap between a workspace's name and its project count.
const GAP: &str = "  ";

/// What the box says before the first reply.
const READING: &str = "reading the store";

/// The reading state's next step.
const READING_NEXT: &str = "the list appears once the store answers";

/// What a reply with no workspace says.
const EMPTY: &str = "no workspaces";

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

    /// One kit row per workspace: `Platform  2 projects`, the name padded to the widest one; the
    /// kit's gutter and band mark the cursor's.
    fn rows(&self, theme: &Theme) -> Vec<Row<'static>> {
        let column = self
            .workspaces
            .iter()
            .map(|w| cell_width(&w.name))
            .max()
            .unwrap_or(0);
        self.workspaces
            .iter()
            .map(|workspace| row(workspace, column, theme))
            .collect()
    }

    /// What the box says with no row, and its next step. Never a blank box (plan D11's rule,
    /// applied to an overlay). The key that creates a workspace is the one `keys` binds where it
    /// acts, Settings › Hierarchy (`HIERARCHY_BROWSE`, MOD-67 M6 D4); with none bound the step
    /// names only the place.
    fn empty_state(&self, keys: &Keys) -> (&'static str, String) {
        if !self.loaded {
            return (READING, READING_NEXT.to_owned());
        }
        let next = match keys.label(views::HIERARCHY_BROWSE, Act::HierarchyNewWorkspace) {
            Some(new) => format!("`{new}` in Settings \u{203a} Hierarchy creates one"),
            None => "create one in Settings \u{203a} Hierarchy".to_owned(),
        };
        (EMPTY, next)
    }
}

/// One workspace row: `Platform  2 projects`, the name padded to `column`.
fn row(workspace: &WorkspaceSummary, column: usize, theme: &Theme) -> Row<'static> {
    // `fit`, not `pad`: `column` is the widest name, so nothing is cut, but `fit` clips first
    // and clipping flattens a control to the blank cell `cell_width` counted it as (D3).
    let name = format!("{}{GAP}", cells::fit(&workspace.name, column));
    Row {
        line: Line::from(vec![
            Span::styled(name, theme.base),
            Span::styled(projects_label(workspace.projects.len()), theme.dim),
        ]),
        kind: RowKind::Item,
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
        let theme = ctx.theme;
        let hint = ctx.keys().hint(views::SWITCHER, HINT);
        let rows = self.rows(theme);
        let (what, next) = self.empty_state(ctx.keys());

        // The natural size: every row whole (the gutter's cell and the row), or the two-line
        // empty state; the hint whole on the border.
        let (widest, height) = if rows.is_empty() {
            (cell_width(what).max(cell_width(&next)), 2)
        } else {
            (1 + widest(&rows), rows.len())
        };
        let content_w = u16::try_from(widest.max(cell_width(&hint) + 1)).unwrap_or(u16::MAX);
        let content = Modal::new(self.title())
            .size(ModalSize::Fit {
                content_w,
                content_h: u16::try_from(height).unwrap_or(u16::MAX),
            })
            .hint(&hint)
            .render(frame, area, theme);

        if rows.is_empty() {
            empty::render(frame, content, what, NextStep(&next), theme);
            return;
        }
        let offset =
            list::scroll_into_view(self.selected, 0, usize::from(content.height), rows.len());
        list::render(
            frame,
            content,
            &rows,
            Some(self.selected),
            offset,
            theme.accent,
            theme,
        );
    }
}

/// `no projects` / `1 project` / `n projects`, so a row never reads `1 projects`.
fn projects_label(count: usize) -> String {
    match count {
        0 => "no projects".to_owned(),
        n => plural(n, "project", "projects"),
    }
}

/// The widest of `rows`, in cells. Measured as the rows were padded: `Line::width` skips the
/// halfwidth sound mark rule, so a name the column pads by cells would overrun a box sized by it
/// (MOD-60 B3).
fn widest(rows: &[Row<'_>]) -> usize {
    rows.iter()
        .map(|row| {
            row.line
                .spans
                .iter()
                .map(|span| cell_width(&span.content))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use htui_core::model::{ProjectId, ProjectRef, WorkspaceId};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Modifier;

    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::store_worker::Origin;
    use crate::ui::kit::list::GUTTER;

    fn workspace(name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: WorkspaceId::default(),
            slug: "w".to_owned(),
            name: name.to_owned(),
            projects: Vec::new(),
        }
    }

    /// A workspace named `name` holding `projects` projects.
    fn holding(name: &str, projects: usize) -> WorkspaceSummary {
        WorkspaceSummary {
            projects: (0..projects)
                .map(|position| ProjectRef {
                    project_id: ProjectId::default(),
                    slug: format!("p{position}"),
                    name: format!("p{position}"),
                    position: i32::try_from(position).expect("few"),
                })
                .collect(),
            ..workspace(name)
        }
    }

    /// A loaded switcher over `workspaces`, its cursor on the first.
    fn loaded(workspaces: Vec<WorkspaceSummary>) -> WorkspaceSwitcher {
        WorkspaceSwitcher {
            workspaces,
            selected: 0,
            loaded: true,
        }
    }

    /// `switcher` drawn over a blank `width`×`height` frame under `keys` and `theme`.
    fn drawn(
        switcher: &WorkspaceSwitcher,
        keys: &Keys,
        theme: &Theme,
        (width, height): (u16, u16),
    ) -> Buffer {
        let scope = Scope {
            workspace_id: WorkspaceId::default(),
            project_ids: Vec::new(),
        };
        let (top_bar, emit) = (TopBarState::default(), Emit::default());
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            keys,
            theme,
            Origin::Overlay(WorkspaceSwitcher::ID),
            &emit,
        );
        let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        term.draw(|frame| switcher.render(frame, frame.area(), &ctx))
            .expect("the switcher draws");
        term.backend().buffer().clone()
    }

    /// The buffer's rows, trailing blanks cut.
    fn lines(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    /// The `(x, y)` where `text` starts in `buffer`.
    fn find(buffer: &Buffer, text: &str) -> (u16, u16) {
        for (y, line) in lines(buffer).iter().enumerate() {
            if let Some(byte) = line.find(text) {
                let x = line[..byte].chars().count();
                return (
                    u16::try_from(x).expect("narrow"),
                    u16::try_from(y).expect("short"),
                );
            }
        }
        panic!("`{text}` is not drawn: {:#?}", lines(buffer));
    }

    /// MOD-67 M6 D4, MOD-115 M4 E6: the empty list says what is missing and names the key that
    /// creates a workspace where it acts (`HIERARCHY_BROWSE`), only the place when it is unbound.
    #[test]
    fn the_empty_switcher_names_the_hierarchy_key() {
        let switcher = loaded(Vec::new());
        assert_eq!(
            switcher.empty_state(Keys::compiled()),
            (
                "no workspaces",
                "`N` in Settings \u{203a} Hierarchy creates one".to_owned()
            )
        );
        let keys =
            crate::keys::load_str("version = 1\n[settings.hierarchy]\nnew_workspace = \"W\"\n")
                .expect("loads");
        assert_eq!(
            switcher.empty_state(&keys).1,
            "`W` in Settings \u{203a} Hierarchy creates one"
        );
        let unbound =
            crate::keys::load_str("version = 1\n[settings.hierarchy]\nnew_workspace = []\n")
                .expect("loads");
        assert_eq!(
            switcher.empty_state(&unbound).1,
            "create one in Settings \u{203a} Hierarchy"
        );
        let reading = WorkspaceSwitcher::new();
        assert_eq!(
            reading.empty_state(Keys::compiled()),
            (
                "reading the store",
                "the list appears once the store answers".to_owned()
            )
        );
    }

    /// The empty state is drawn through `kit::empty`: both lines, inside the box.
    #[test]
    fn the_empty_box_draws_what_is_missing_and_the_next_step() {
        let buffer = drawn(
            &loaded(Vec::new()),
            Keys::compiled(),
            &Theme::default(),
            (100, 30),
        );
        let (_, what) = find(&buffer, "no workspaces");
        let (_, next) = find(&buffer, "`N` in Settings \u{203a} Hierarchy creates one");
        assert_eq!(next, what + 1, "{:#?}", lines(&buffer));
        let buffer = drawn(
            &WorkspaceSwitcher::new(),
            Keys::compiled(),
            &Theme::default(),
            (100, 30),
        );
        find(&buffer, "reading the store");
        find(&buffer, "the list appears once the store answers");
    }

    /// `no projects` / `1 project` / `n projects`, through `kit::label::plural`.
    #[test]
    fn a_row_counts_its_projects_in_words() {
        assert_eq!(projects_label(0), "no projects");
        assert_eq!(projects_label(1), "1 project");
        assert_eq!(projects_label(3), "3 projects");
        let buffer = drawn(
            &loaded(vec![holding("Graphics", 1), holding("Platform", 2)]),
            Keys::compiled(),
            &Theme::default(),
            (100, 30),
        );
        find(&buffer, "Graphics  1 project");
        find(&buffer, "Platform  2 projects");
    }

    /// MOD-115 M4 E6 (MOD-85): the cursor row is the kit's gutter and band, so it reads under
    /// `NO_COLOR` (the band is reversed); the other rows have neither.
    #[test]
    fn the_cursor_row_is_the_band_and_reads_without_colour() {
        let theme = Theme::monochrome();
        let switcher = WorkspaceSwitcher {
            selected: 1,
            ..loaded(vec![holding("Graphics", 1), holding("Platform", 2)])
        };
        let buffer = drawn(&switcher, Keys::compiled(), &theme, (100, 30));
        let (x, y) = find(&buffer, "Platform");
        let row = lines(&buffer)[usize::from(y)].clone();
        // Finding F2: the gutter alone marks it, no `> ` text marker beside it.
        assert!(row.contains(&format!("{GUTTER}Platform")), "{row}");
        assert!(
            buffer[(x, y)].modifier.contains(Modifier::REVERSED),
            "the band: {:?}",
            buffer[(x, y)]
        );
        let (x, y) = find(&buffer, "Graphics");
        assert!(!lines(&buffer)[usize::from(y)].contains(GUTTER));
        assert!(!buffer[(x, y)].modifier.contains(Modifier::REVERSED));
        for cell in &buffer.content {
            assert_eq!(
                (cell.fg, cell.bg),
                (ratatui::style::Color::Reset, ratatui::style::Color::Reset)
            );
        }
    }

    /// MOD-115 M4 E6: the keys ride the modal's bottom border, so the box holds only its rows.
    #[test]
    fn the_keys_ride_the_bottom_border() {
        let buffer = drawn(
            &loaded(vec![holding("Graphics", 1), holding("Platform", 2)]),
            Keys::compiled(),
            &Theme::default(),
            (100, 30),
        );
        let (_, top) = find(&buffer, "\u{256d} Workspaces ");
        let (_, first) = find(&buffer, "Graphics");
        let (_, bottom) = find(&buffer, " j/k move \u{b7} Enter switch \u{b7} Esc close ");
        assert_eq!(first, top + 1);
        assert_eq!(
            bottom,
            top + 3,
            "two rows, then the border: {:#?}",
            lines(&buffer)
        );
        assert!(lines(&buffer)[usize::from(bottom)].contains('\u{2570}'));
    }

    /// On a screen narrower than the hint, the border's hint is cut with an ellipsis.
    #[test]
    fn a_hint_wider_than_the_box_is_cut_with_an_ellipsis() {
        let buffer = drawn(
            &loaded(vec![holding("Graphics", 1)]),
            Keys::compiled(),
            &Theme::default(),
            (30, 10),
        );
        let (_, bottom) = find(&buffer, "j/k move");
        let row = lines(&buffer)[usize::from(bottom)].clone();
        assert!(row.contains('\u{2026}'), "{row}");
    }

    /// MOD-60: the name column is measured and padded in cells, so a CJK name's project count
    /// lines up with an ASCII one's.
    #[test]
    fn a_wide_workspace_name_keeps_the_counts_in_one_column() {
        let switcher = loaded(vec![workspace("Platform"), workspace("\u{5e73}\u{53f0}")]);
        let rows = switcher.rows(&Theme::default());
        let first: Vec<usize> = rows
            .iter()
            .map(|row| cell_width(&row.line.spans[0].content))
            .collect();
        assert_eq!(first[0], first[1], "{rows:?}");
    }

    /// MOD-60 D3: `cell_width` counts a control as one cell but ratatui draws nothing for it, so
    /// the name is flattened before it is padded. Measured where ratatui stops drawing each row:
    /// every row ends with the same label, so equal ends mean one count column.
    #[test]
    fn a_control_char_in_a_workspace_name_keeps_the_counts_in_one_column() {
        let switcher = loaded(vec![
            workspace("Platform"),
            workspace("a\tb"),
            workspace("a\u{1}b"),
        ]);
        let rows = switcher.rows(&Theme::default());
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 3));
        let ends: Vec<u16> = (0u16..3)
            .map(|y| buffer.set_line(0, y, &rows[usize::from(y)].line, 80).0)
            .collect();
        assert_eq!(ends, vec![ends[0]; 3], "{rows:?}");
    }

    /// MOD-60 B3: the box is as wide as the widest row in cells. `ｶﾞ` is two cells and one to
    /// `Line::width`, so a box sized that way is narrower than the row padded in cells.
    #[test]
    fn a_halfwidth_workspace_name_fits_inside_the_box() {
        let name = "\u{ff76}\u{ff9e}".repeat(20);
        let switcher = loaded(vec![workspace(&name)]);
        let rows = switcher.rows(&Theme::default());
        let row: String = rows[0]
            .line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(cell_width(&row), 40 + 2 + cell_width(&projects_label(0)));
        assert_eq!(widest(&rows), cell_width(&row), "{row:?}");
        // The whole row is drawn: its count is not cut at the box's edge.
        let buffer = drawn(&switcher, Keys::compiled(), &Theme::default(), (100, 30));
        find(&buffer, "  no projects");
    }
}
