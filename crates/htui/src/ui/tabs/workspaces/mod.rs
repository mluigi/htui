//! Workspaces tab (MOD-116 M1 D8, `R-TUI-14`): every workspace, what it is doing, and `o` to
//! open one.
//!
//! The list on the left holds one group row per workspace, in the `Workspaces` reply's order (by
//! name), with its state (`● open`, `◐ background · 1 running`, `◌ stopping…`, `○ stopped · 1
//! held`, D2) derived from the run counts the shell reads once a second into
//! [`TopBarState::workspace_runs`](crate::app::TopBarState) (D5): the tab never polls them itself.
//! Under each group, unless it is folded, an `overview` row and one row per project. The detail
//! pane on the right shows the selected workspace's overview, or the selected project's, from one
//! [`StoreRequest::WorkspaceOverview`] read per event (D7): activation, a cursor move onto another
//! workspace, or a change in that workspace's counts. Never per frame.
//!
//! `o` ([`Act::WorkspacesOpen`]) on any row of a workspace emits [`Action::SetScope`] with it
//! (D9); the shell applies it as it applied the switcher's `Enter`, the focus stays here. A scope
//! change keeps every row: the list is not one workspace's.

mod overview;

use std::collections::BTreeSet;

use core::cell::Cell;
use crossterm::event::KeyEvent;
use htui_core::model::{
    ProjectId, Scope, WorkspaceId, WorkspaceLifecycle, WorkspaceRunCounts, WorkspaceSummary,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::kit::chips::key_line;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::pane::{Focus, Pane};
use crate::ui::kit::scroll::{Scroll, Step, window};
use crate::ui::shell::{PaneId, Panes, TabLayout, TabLoc};
use crate::ui::tabs::registry::{Tab, TabId};
use crate::workspaces::WorkspaceOverview;

/// The Workspaces tab's default layout: the shell's list + detail default.
pub const WORKSPACES_LAYOUT: TabLayout = TabLayout::DEFAULT;

/// [`StoreRequest::WorkspaceOverview`]'s name, whose refusal the detail pane says itself.
const WORKSPACE_OVERVIEW: &str = "workspace_overview";

/// [`StoreRequest::WorkspaceRunCounts`]'s name: the tab asks once on activation, so the list's
/// states are current at once; a refusal (offline) leaves the open workspace reading `open`.
const WORKSPACE_RUN_COUNTS: &str = "workspace_run_counts";

/// The hint row the focused pane carries.
const HINT: HintSpec = &[
    Hint::One(Act::WorkspacesOpen, "open"),
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::ListFold, "fold"),
];

/// The detail pane's sub-tabs (MOD-117 and MOD-118 add theirs).
const SUB_TABS: &[&str] = &["overview"];

/// The `overview` entry's row label under its workspace.
const OVERVIEW_ROW: &str = "overview";

/// The list before the first reply.
const READING: &str = "reading the store";

/// The reading state's next step.
const READING_NEXT: &str = "the workspaces appear once the store answers";

/// What a store with no workspace says.
const EMPTY: &str = "No workspaces yet.";

/// The detail pane while its overview is out.
const READING_OVERVIEW: &str = "reading the overview";

/// The overview's reading state's next step.
const READING_OVERVIEW_NEXT: &str = "it appears once the store answers";

/// A refused overview's sentence, before the store's message.
const NEEDS_THE_DATABASE: &str = "the overview needs the database";

/// A refused overview's next step: the read goes again when the counts move, which they do once
/// the database answers.
const REFUSED_NEXT: &str = "it is read again once the database answers";

/// An overview of a workspace the store no longer holds.
const GONE: &str = "this workspace is no longer in the store";

/// A gone workspace's next step.
const GONE_NEXT: &str = "the list is read again when you come back to this tab";

/// The widest a workspace name is drawn in the list.
const NAME_WIDTH: usize = 24;

/// How a [`TabLoc`] cursor names a workspace's group row.
const WORKSPACE_CURSOR: &str = "workspace";
/// How a [`TabLoc`] cursor names a workspace's `overview` row.
const OVERVIEW_CURSOR: &str = "overview";
/// How a [`TabLoc`] cursor names a project row.
const PROJECT_CURSOR: &str = "project";

/// A selectable row of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A workspace's group row.
    Workspace(WorkspaceId),
    /// A workspace's `overview` row.
    Overview(WorkspaceId),
    /// One of a workspace's projects.
    Project(WorkspaceId, ProjectId),
}

impl Entry {
    /// The workspace the row belongs to.
    const fn workspace(self) -> WorkspaceId {
        match self {
            Self::Workspace(id) | Self::Overview(id) | Self::Project(id, _) => id,
        }
    }

    /// The [`TabLoc`] cursor: `workspace:<id>`, `overview:<id>` or `project:<ws>:<id>`.
    fn cursor(self) -> String {
        match self {
            Self::Workspace(id) => format!("{WORKSPACE_CURSOR}:{id}"),
            Self::Overview(id) => format!("{OVERVIEW_CURSOR}:{id}"),
            Self::Project(ws, id) => format!("{PROJECT_CURSOR}:{ws}:{id}"),
        }
    }

    /// The entry a [`cursor`](Self::cursor) names.
    fn parse(cursor: &str) -> Option<Self> {
        let (kind, rest) = cursor.split_once(':')?;
        match kind {
            WORKSPACE_CURSOR => rest.parse().ok().map(Self::Workspace),
            OVERVIEW_CURSOR => rest.parse().ok().map(Self::Overview),
            PROJECT_CURSOR => {
                let (ws, project) = rest.split_once(':')?;
                Some(Self::Project(ws.parse().ok()?, project.parse().ok()?))
            }
            _ => None,
        }
    }
}

/// What the tab holds for the workspace it last read.
#[derive(Debug, Clone, PartialEq)]
enum Held {
    /// The overview.
    Read(Box<WorkspaceOverview>),
    /// The store holds no such workspace.
    Gone,
    /// The read was refused, with the store's message.
    Refused(String),
}

/// The Workspaces tab (MOD-116 M1 D8).
#[derive(Debug, Default)]
pub struct WorkspacesTab {
    /// What the last [`StoreReply::Workspaces`] carried, in the store's order (by name).
    workspaces: Vec<WorkspaceSummary>,
    /// Whether a reply has landed yet: "still reading" and "nothing there" are different screens.
    loaded: bool,
    /// The workspaces whose group is folded.
    folded: BTreeSet<WorkspaceId>,
    /// The selected row; `None` until the first reply, and while the store holds no workspace.
    cursor: Option<Entry>,
    /// The last overview read, and the workspace it is of.
    overview: Option<(WorkspaceId, Held)>,
    /// The workspace's counts row (`TopBarState::workspace_runs`) when its overview was asked
    /// for: a different row asks again (D7).
    read_against: Option<WorkspaceRunCounts>,
    /// The workspace whose overview is out: one read at a time, as a second one would make the
    /// shell drop the first one's reply.
    in_flight: Option<WorkspaceId>,
    /// The detail pane's scroll.
    scroll: Scroll,
    /// The detail's rows as last drawn, so a scroll is clamped to them.
    detail_rows: Cell<usize>,
}

impl WorkspacesTab {
    /// The tab's id; the theme's amber `workspaces` accent is keyed by it.
    pub const ID: TabId = TabId("workspaces");

    /// The tab before its first reply.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The summary of `workspace`, if listed.
    fn summary(&self, workspace: WorkspaceId) -> Option<&WorkspaceSummary> {
        self.workspaces
            .iter()
            .find(|summary| summary.workspace_id == workspace)
    }

    /// Every selectable row, in list order: a group, then its `overview` and project rows
    /// unless it is folded.
    fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        for summary in &self.workspaces {
            let id = summary.workspace_id;
            entries.push(Entry::Workspace(id));
            if self.folded.contains(&id) {
                continue;
            }
            entries.push(Entry::Overview(id));
            entries.extend(
                projects_of(summary)
                    .iter()
                    .map(|project| Entry::Project(id, project.project_id)),
            );
        }
        entries
    }

    /// Whether `entry` names a row the current list holds (folded groups' rows included).
    fn holds(&self, entry: Entry) -> bool {
        self.summary(entry.workspace())
            .is_some_and(|summary| match entry {
                Entry::Workspace(_) | Entry::Overview(_) => true,
                Entry::Project(_, project) => summary
                    .projects
                    .iter()
                    .any(|held| held.project_id == project),
            })
    }

    /// Moves the cursor to `entry`; another workspace's row resets the detail's scroll.
    fn select(&mut self, entry: Entry) {
        if self.cursor != Some(entry) {
            self.scroll.reset();
        }
        self.cursor = Some(entry);
    }

    /// A list motion over the selectable rows.
    fn move_cursor(&mut self, act: Act) {
        let entries = self.entries();
        let Some(last) = entries.len().checked_sub(1) else {
            return;
        };
        let at = self
            .cursor
            .and_then(|cursor| entries.iter().position(|entry| *entry == cursor))
            .unwrap_or(0);
        let next = match act {
            Act::ListDown => (at + 1).min(last),
            Act::ListUp => at.saturating_sub(1),
            Act::ListBottom => last,
            _ => 0,
        };
        self.select(entries[next]);
    }

    /// `list.fold`: folds or unfolds the group under the cursor; on a child row the group folds
    /// and the cursor goes up to its row.
    fn fold(&mut self) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let workspace = cursor.workspace();
        if !self.folded.remove(&workspace) {
            self.folded.insert(workspace);
            self.select(Entry::Workspace(workspace));
        }
    }

    /// `workspaces.open` (D9): enter the selected row's workspace; the shell's `set_scope` does
    /// the rest (or refuses while an in-pane editor is open).
    fn open(&self, ctx: &Ctx<'_>) {
        if let Some(summary) = self
            .cursor
            .and_then(|entry| self.summary(entry.workspace()))
        {
            ctx.emit(Action::SetScope {
                workspace: summary.clone(),
            });
        }
    }

    /// D7: asks for the selected workspace's overview when none is out and either none is held
    /// for it or its counts row moved since it was asked for.
    fn read_if_due(&mut self, ctx: &Ctx<'_>) {
        if self.in_flight.is_some() {
            return;
        }
        let Some(workspace) = self.cursor.map(Entry::workspace) else {
            return;
        };
        let row = counts_of(ctx, workspace);
        let held = matches!(&self.overview, Some((of, _)) if *of == workspace);
        if held && (row.is_none() || row == self.read_against.as_ref()) {
            return;
        }
        self.read_against = row.cloned();
        self.in_flight = Some(workspace);
        ctx.request(StoreRequest::WorkspaceOverview(workspace));
    }

    /// The list: one group row per workspace and its entries, the cursor on the selected one.
    fn render_list(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focus: Focus,
        hints: Option<&Line<'static>>,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let inner = Pane::inner(area);
        let column = self
            .workspaces
            .iter()
            .map(|summary| cells::cell_width(&summary.name).min(NAME_WIDTH))
            .max()
            .unwrap_or(0);
        let mut rows: Vec<Row<'static>> = Vec::new();
        let mut cursor = None;
        for summary in &self.workspaces {
            let id = summary.workspace_id;
            let folded = self.folded.contains(&id);
            if self.cursor == Some(Entry::Workspace(id)) {
                cursor = Some(rows.len());
            }
            rows.push(group_row(
                summary,
                column,
                folded,
                lifecycle(ctx, id),
                theme,
            ));
            if folded {
                continue;
            }
            if self.cursor == Some(Entry::Overview(id)) {
                cursor = Some(rows.len());
            }
            rows.push(child_row(OVERVIEW_ROW, theme));
            for project in projects_of(summary) {
                if self.cursor == Some(Entry::Project(id, project.project_id)) {
                    cursor = Some(rows.len());
                }
                rows.push(child_row(&project.name, theme));
            }
        }
        let offset = window(cursor.unwrap_or(0), rows.len(), usize::from(inner.height));
        if rows.is_empty() {
            let (what, next) = self.empty_state(ctx);
            empty::render(frame, inner, what, NextStep(&next), theme);
        } else {
            list::render(frame, inner, &rows, cursor, offset, ctx.accent(), theme);
        }
        let mut pane = Pane::new("Workspaces")
            .focus(focus)
            .accent(ctx.accent())
            .scrollbar(rows.len(), offset);
        if let Some(hints) = hints {
            pane = pane.hints(hints.clone());
        }
        pane.render(frame, area, theme);
    }

    /// What the list says with no row, and its next step: reading, or no workspace and how to
    /// create one (the key `Settings › Hierarchy` binds, as the switcher said).
    fn empty_state(&self, ctx: &Ctx<'_>) -> (&'static str, String) {
        if !self.loaded {
            return (READING, READING_NEXT.to_owned());
        }
        let next = match ctx
            .keys()
            .label(views::HIERARCHY_BROWSE, Act::HierarchyNewWorkspace)
        {
            Some(new) => format!("`{new}` in Settings \u{203a} Hierarchy creates one"),
            None => "create one in Settings \u{203a} Hierarchy".to_owned(),
        };
        (EMPTY, next)
    }

    /// The detail: the selected workspace's or project's overview, or the state that says why
    /// there is none.
    fn render_detail(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focus: Focus,
        hints: Option<&Line<'static>>,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let inner = Pane::inner(area);
        let width = usize::from(inner.width);
        let cursor = self.cursor.filter(|entry| self.holds(*entry));
        let title = cursor.map_or_else(|| "Overview".to_owned(), |entry| self.title_of(entry));
        let held = cursor.and_then(|entry| {
            self.overview
                .as_ref()
                .filter(|(of, _)| *of == entry.workspace())
                .map(|(_, held)| held)
        });

        let mut lines: Vec<Line<'static>> = Vec::new();
        match (cursor, held) {
            (None, _) => {
                let (what, next) = self.empty_state(ctx);
                empty::render(frame, inner, what, NextStep(&next), theme);
            }
            (Some(entry), Some(Held::Read(overview))) => {
                let workspace = entry.workspace();
                lines = match entry {
                    Entry::Project(_, project) => overview
                        .tree
                        .projects
                        .iter()
                        .find(|candidate| candidate.project.id == project)
                        .map_or_else(Vec::new, |found| {
                            overview::project_lines(overview, found, width, theme)
                        }),
                    Entry::Workspace(_) | Entry::Overview(_) => {
                        let open = ctx.scope.workspace_id == workspace;
                        let state = WorkspaceLifecycle::of(open, &overview.counts);
                        overview::workspace_lines(overview, state, width, theme)
                    }
                };
            }
            (Some(_), Some(Held::Refused(message))) => {
                let what = format!("{NEEDS_THE_DATABASE}: {message}");
                empty::render_failed(frame, inner, &what, NextStep(REFUSED_NEXT), theme);
            }
            (Some(_), Some(Held::Gone)) => {
                empty::render(frame, inner, GONE, NextStep(GONE_NEXT), theme);
            }
            (Some(_), None) => {
                empty::render(
                    frame,
                    inner,
                    READING_OVERVIEW,
                    NextStep(READING_OVERVIEW_NEXT),
                    theme,
                );
            }
        }
        let total = lines.len();
        self.detail_rows.set(total);
        let offset = self.scroll.skip().min(total.saturating_sub(1));
        if total > 0 {
            frame.render_widget(
                Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
                inner,
            );
        }
        let mut pane = Pane::new(&title)
            .sub_tabs(SUB_TABS, 0)
            .focus(focus)
            .accent(ctx.accent())
            .scrollbar(total, offset);
        if let Some(hints) = hints {
            pane = pane.hints(hints.clone());
        }
        pane.render(frame, area, theme);
    }

    /// The detail pane's title: the workspace's name, or the project's.
    fn title_of(&self, entry: Entry) -> String {
        let Some(summary) = self.summary(entry.workspace()) else {
            return String::new();
        };
        match entry {
            Entry::Workspace(_) | Entry::Overview(_) => summary.name.clone(),
            Entry::Project(_, project) => summary
                .projects
                .iter()
                .find(|held| held.project_id == project)
                .map_or_else(|| summary.name.clone(), |held| held.name.clone()),
        }
    }
}

/// `summary`'s projects in position order.
fn projects_of(summary: &WorkspaceSummary) -> Vec<&htui_core::model::ProjectRef> {
    let mut projects: Vec<_> = summary.projects.iter().collect();
    projects.sort_by_key(|project| project.position);
    projects
}

/// `workspace`'s row of the shell's last counts read, if any.
fn counts_of<'a>(ctx: &Ctx<'a>, workspace: WorkspaceId) -> Option<&'a WorkspaceRunCounts> {
    ctx.top_bar
        .workspace_runs
        .as_deref()
        .and_then(|rows| rows.iter().find(|row| row.workspace_id == workspace))
}

/// `workspace`'s state (D2) from the shell's counts: with none read yet (or offline), the open
/// workspace still reads `open` and the others have no state.
fn lifecycle(ctx: &Ctx<'_>, workspace: WorkspaceId) -> Option<WorkspaceLifecycle> {
    let open = ctx.scope.workspace_id == workspace;
    match counts_of(ctx, workspace) {
        Some(row) => Some(WorkspaceLifecycle::of(open, row)),
        None if open => Some(WorkspaceLifecycle::Open { running: 0 }),
        None => None,
    }
}

/// A workspace's group row: `▾ Platform  ◐ background · 1 running`, the name padded to `column`.
fn group_row(
    summary: &WorkspaceSummary,
    column: usize,
    folded: bool,
    state: Option<WorkspaceLifecycle>,
    theme: &Theme,
) -> Row<'static> {
    let glyph = if folded { "\u{25b8}" } else { "\u{25be}" };
    let mut spans = vec![
        Span::styled(glyph, theme.title),
        Span::styled(" ", theme.title),
        Span::styled(cells::fit(&summary.name, column), theme.title),
    ];
    if let Some(state) = state {
        spans.push(Span::raw("  "));
        spans.extend(overview::state_spans(state, theme));
    }
    Row {
        line: Line::from(spans),
        kind: RowKind::Group { folded },
    }
}

/// An indented row under a group: `overview`, or a project's name.
fn child_row(text: &str, theme: &Theme) -> Row<'static> {
    Row {
        line: Line::from(vec![
            Span::raw("    "),
            Span::styled(text.to_owned(), theme.base),
        ]),
        kind: RowKind::Item,
    }
}

impl Tab for WorkspacesTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Workspaces"
    }

    /// Every workspace, and their counts at once (the reply is the shell's, D5: it sets the top
    /// bar whoever asked).
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::Workspaces, StoreRequest::WorkspaceRunCounts]
    }

    /// The list is every workspace's, not the scope's: a scope change keeps it (only which one
    /// reads `open` moves, and that is derived at render).
    fn on_scope_change(&mut self, _scope: &Scope) {}

    /// Every key resolves through [`views::WORKSPACES_BROWSE`]: a list motion moves the cursor
    /// (or, with the detail focused, scrolls it), `list.fold` folds the group, the pane scroll
    /// moves the detail and `workspaces.open` opens the selected workspace.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        let focus = ctx.focus();
        let rows = self.detail_rows.get();
        for act in ctx.keys().actions(views::WORKSPACES_BROWSE, chord) {
            if Step::by_focus(act, focus, |step| self.scroll.follow(step, rows)) {
                return Handled::Consumed;
            }
            match act {
                Act::ListDown | Act::ListUp | Act::ListTop | Act::ListBottom => {
                    self.move_cursor(act);
                    self.read_if_due(ctx);
                }
                Act::ListFold => self.fold(),
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp => {
                    let _ = self.scroll.apply(act, rows);
                }
                Act::WorkspacesOpen => self.open(ctx),
                // A layout, history or global act: the shell's.
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Workspaces(workspaces) => {
                self.workspaces.clone_from(workspaces);
                self.loaded = true;
                let kept = self.cursor.filter(|entry| self.holds(*entry));
                let landed = kept
                    .or_else(|| {
                        self.summary(ctx.scope.workspace_id)
                            .map(|summary| Entry::Workspace(summary.workspace_id))
                    })
                    .or_else(|| {
                        self.workspaces
                            .first()
                            .map(|summary| Entry::Workspace(summary.workspace_id))
                    });
                match landed {
                    Some(entry) => self.select(entry),
                    None => self.cursor = None,
                }
                self.read_if_due(ctx);
            }
            StoreReply::WorkspaceOverview {
                workspace,
                overview,
            } => {
                if self.in_flight == Some(*workspace) {
                    self.in_flight = None;
                }
                let held = overview
                    .as_ref()
                    .map_or(Held::Gone, |overview| Held::Read(overview.clone()));
                self.overview = Some((*workspace, held));
                self.read_if_due(ctx);
            }
            StoreReply::Failed { request, message } if *request == WORKSPACE_OVERVIEW => {
                if let Some(workspace) = self.in_flight.take() {
                    self.overview = Some((workspace, Held::Refused(message.clone())));
                }
                self.read_if_due(ctx);
            }
            // The shell has set the top bar from it already (D5); a moved row reads again.
            StoreReply::WorkspaceRunCounts(_) => self.read_if_due(ctx),
            _ => {}
        }
    }

    /// The detail pane says a refused overview itself; a refused counts read leaves the states
    /// as they were (the shell's own poll is absorbed too, D5).
    fn absorbs_failure(&self, request: &str) -> bool {
        request == WORKSPACE_OVERVIEW || request == WORKSPACE_RUN_COUNTS
    }

    fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        key_line(ctx.keys().hint(views::WORKSPACES_BROWSE, HINT), ctx.theme)
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let areas = ctx.split(area, WORKSPACES_LAYOUT);
        let hints = key_line(ctx.keys().hint(views::WORKSPACES_BROWSE, HINT), ctx.theme);
        if let Some(list) = areas.list.filter(|rect| !rect.is_empty()) {
            let focus = ctx.pane_focus(PaneId::List);
            self.render_list(frame, list, focus, hints.as_ref(), ctx);
        }
        if !areas.detail.is_empty() {
            let focus = ctx.pane_focus(PaneId::Detail);
            self.render_detail(frame, areas.detail, focus, hints.as_ref(), ctx);
        }
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::WORKSPACES_BROWSE)
    }

    /// D7: once a second while in front, a moved counts row reads the overview again.
    fn on_refresh(&mut self, ctx: &mut Ctx<'_>) {
        self.read_if_due(ctx);
    }

    fn on_activate(&mut self, ctx: &mut Ctx<'_>) {
        self.read_if_due(ctx);
    }

    fn panes(&self) -> Panes {
        Panes::ListDetail(WORKSPACES_LAYOUT)
    }

    /// The selected row: `workspace:<id>`, `overview:<id>` or `project:<ws>:<id>`.
    fn location(&self) -> TabLoc {
        TabLoc {
            cursor: self.cursor.map(Entry::cursor),
            ..TabLoc::default()
        }
    }

    /// Back to a [`location`](Tab::location)'s row: its group unfolds when the row is a child,
    /// and its overview is read when it is not held. Before the list has landed the row is kept
    /// for it; after, a row the list no longer holds is refused.
    fn restore(&mut self, loc: &TabLoc, ctx: &mut Ctx<'_>) -> bool {
        let Some(entry) = loc.cursor.as_deref().and_then(Entry::parse) else {
            return false;
        };
        if self.loaded && !self.holds(entry) {
            return false;
        }
        if !matches!(entry, Entry::Workspace(_)) {
            self.folded.remove(&entry.workspace());
        }
        self.select(entry);
        self.read_if_due(ctx);
        true
    }
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;

    use super::Entry;

    #[test]
    fn an_entry_s_cursor_parses_back() {
        for entry in [
            Entry::Workspace(ids::WORKSPACE_PLATFORM),
            Entry::Overview(ids::WORKSPACE_GRAPHICS),
            Entry::Project(ids::WORKSPACE_PLATFORM, ids::PROJECT_AGY),
        ] {
            assert_eq!(Entry::parse(&entry.cursor()), Some(entry), "{entry:?}");
        }
        assert_eq!(Entry::parse("workspace:not-an-id"), None);
        assert_eq!(Entry::parse("nothing"), None);
        assert_eq!(Entry::parse("project:only-one"), None);
    }
}
