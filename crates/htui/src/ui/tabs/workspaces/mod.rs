//! Workspaces tab (MOD-116 M1 D8, `R-TUI-14`): every workspace, what it is doing, and `o` to
//! open one.
//!
//! The list on the left holds one group row per workspace, in the `Workspaces` reply's order (by
//! name), with its state (`● open`, `◐ background · 1 running`, `◌ stopping…`, `○ stopped · 1
//! held`, D2) derived from the run counts the shell reads once a second into
//! [`TopBarState::workspace_runs`](crate::app::TopBarState) (D5): the tab never polls them itself.
//! Under each group, unless it is folded, an `overview` row and one row per project. The detail
//! pane on the right shows the selected workspace's overview, or the selected project's, from one
//! [`StoreRequest::WorkspaceOverview`] read per event (D7): activation (always), a cursor move
//! onto another workspace, a change in that workspace's counts, or a list whose projects for it
//! differ from the overview's. Never per frame. A refused read is not held: the next refresh
//! asks again (R1 F3).
//!
//! `o` ([`Act::WorkspacesOpen`]) on any row of a workspace emits [`Action::SetScope`] with it
//! (D9); the shell applies it as it applied the switcher's `Enter`, the focus stays here. A scope
//! change keeps every row: the list is not one workspace's. A cursor the user has not moved
//! follows the open workspace: it lands on its row on every list and every scope change, and is
//! not placed while there is no scope yet (R1 F1: the tab's first list can land before the
//! shell's startup scope).
//!
//! Offline the shell's counts are the last ones read, not current (D8 over D5): the rows stay,
//! but no workspace other than the open one shows a state (R1 F6).
//!
//! MOD-116 M2 (`R-ORCH-16`, `R-TUI-14`): `s` ([`Act::WorkspacesStop`]) and `r`
//! ([`Act::WorkspacesResume`]) on any row of a workspace read its overview afresh and ask from its
//! stop report (D6): stop its live runs gracefully, force a graceful stop that still has a running
//! run, or resume it; each question lists the runs and what the answer does to each. A `y` (or
//! `Enter`) sends the write with the stop row the question was decided on, so a stop that changed
//! meanwhile writes nothing and is said (D3). Whatever there is nothing to ask about is told in the
//! same box. The detail lists the workspace's runs and how each resumes (D5); the open workspace's
//! state carries its stop (`● open · 1 running · stopping…`, D7); the hint row offers what the
//! selected workspace's state allows (D8). Offline the detail shows no state the counts cannot
//! vouch for, and no Runs section (D7).
//!
//! MOD-116 M3 (`R-TUI-14`, `R-ENT-2`): `N` ([`Act::WorkspacesNew`]) opens a chooser over the tab,
//! `workspace` or `project in <name>` for the selected row's workspace (D1), or the workspace form
//! at once when none is listed. Each form takes the detail pane's place (the list's, when the
//! layout hides the detail) and previews what it writes; paths are shown under `~` and sent
//! absolute (D3), and the worker (`crate::workspace_setup`) does every disk step. A new
//! workspace is listed stopped and not opened, unless the shell's scope names none the tab lists
//! (D4); a new project is listed and selected, and the open workspace's scope follows the list
//! (D11). A clone may take minutes: the form counts the seconds, `Esc` closes it while the
//! request finishes, and the outcome is told then (D7, D13).
//!
//! MOD-116 M3 T4: `e` ([`Act::Edit`]) on a workspace's group or `overview` row opens its form
//! prefilled from a fresh overview (D5); `d` ([`Act::Delete`]) counts, warns and asks for the
//! typed slug, as Settings › Hierarchy does, and leaves the folder on disk (D10); `T`
//! ([`Act::WorkspacesTarget`]) on any row picks the workspace's default target among the live
//! targets, or the Settings default it inherits (D9). On a project row `e` and `d` say where
//! projects are edited (D12).

mod chooser;
mod delete;
mod forms;
mod overview;
mod question;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use core::cell::Cell;
use crossterm::event::KeyEvent;
use htui_core::model::{
    ProjectId, ProjectRef, RunStatus, Scope, StopMode, TargetId, WorkspaceId, WorkspaceLifecycle,
    WorkspaceRunCounts, WorkspaceStop, WorkspaceSummary,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, Stack, views};
use crate::stop_report::{ResumeWrite, StopTarget, StopWrite};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::kit::chips::key_line;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::modal::{Ask, modal_rest};
use crate::ui::kit::pane::{self, Focus, Pane};
use crate::ui::kit::scroll::{Scroll, Step, window};
use crate::ui::shell::{PaneId, Panes, TabLayout, TabLoc};
use crate::ui::tabs::registry::{Tab, TabId};
use crate::ui::tabs::settings::changed_elsewhere;
use crate::workspace_setup::{self, EditOutcome, TargetChoices, TargetWrite};
use crate::workspaces::WorkspaceOverview;
use chooser::{ChoiceRow, Chooser, ChooserOutcome};
use delete::{DELETE_REACH, DELETE_TITLE, DELETE_WORKSPACE, DeleteOutcome, Deleting};
use forms::{FormOutcome, ProjectForm, WorkspaceForm, shown};

/// The Workspaces tab's default layout: the shell's list + detail default.
pub const WORKSPACES_LAYOUT: TabLayout = TabLayout::DEFAULT;

/// [`StoreRequest::WorkspaceOverview`]'s name, whose refusal the detail pane says itself.
const WORKSPACE_OVERVIEW: &str = "workspace_overview";

/// [`StoreRequest::WorkspaceRunCounts`]'s name: the tab asks once on activation, so the list's
/// states are current at once; a refusal (offline) leaves the open workspace reading `open`.
const WORKSPACE_RUN_COUNTS: &str = "workspace_run_counts";

/// [`StoreRequest::StopWorkspaces`]'s name, whose refusal the tab tells in its box (D6).
const STOP_WORKSPACES: &str = "stop_workspaces";

/// [`StoreRequest::ResumeWorkspace`]'s name, whose refusal the tab tells in its box (D6).
const RESUME_WORKSPACE: &str = "resume_workspace";

/// [`StoreRequest::NewWorkspace`]'s name (MOD-116 M3 D15).
const NEW_WORKSPACE: &str = workspace_setup::REQUEST_NAMES[0];

/// [`StoreRequest::EditWorkspace`]'s name (MOD-116 M3 D15).
const EDIT_WORKSPACE: &str = workspace_setup::REQUEST_NAMES[1];

/// [`StoreRequest::NewProject`]'s name (MOD-116 M3 D15).
const NEW_PROJECT: &str = workspace_setup::REQUEST_NAMES[2];

/// [`StoreRequest::TargetChoices`]'s name (MOD-116 M3 D15).
const TARGET_CHOICES: &str = workspace_setup::REQUEST_NAMES[3];

/// The chooser's title (MOD-116 M3 D1).
const NEW_TITLE: &str = "New";

/// The workspace form's title, and its told boxes'.
const NEW_WORKSPACE_TITLE: &str = "New workspace";

/// A told box about a project being created (D7, D13).
const NEW_PROJECT_TITLE: &str = "New project";

/// `N` → project while one is out (D7: the shell's staleness gate would drop the first reply).
const STILL_CREATING: &str = "a project is still being created";

/// `Esc` while a project is out (D7).
const TOLD_WHEN_IT_LANDS: &str = "the project is still being created; you are told when it lands";

/// A told box about an edit (D5).
const EDIT_TITLE: &str = "Edit workspace";

/// A told box about the default target (D9).
const TARGET_TITLE: &str = "Default target";

/// `e` or `d` on a project row (D12).
const PROJECTS_IN_SETTINGS: &str =
    "projects are edited in Settings \u{203a} Hierarchy until MOD-123";

/// The chooser's keys on its border (D1).
const CHOOSER_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::ConfirmYes, "choose"),
    Hint::One(Act::ConfirmNo, "close"),
];

/// The workspace form's keys on its pane's border.
const WORKSPACE_FORM_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "create"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::One(Act::FormNextField, "field"),
];

/// The edit form's keys on its pane's border (D5).
const EDIT_FORM_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::One(Act::FormNextField, "field"),
];

/// The project form's keys on its pane's border: the workspace form's.
const PROJECT_FORM_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "create"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::One(Act::FormNextField, "field"),
];

/// The project form's keys on the source choice (R1b B1): its own keys first, so the pane's cut
/// keeps the keys that act on the focused field (as Requirements' priority field does), written
/// as a choice's are (`←/→ source` under the defaults, whose second chords are `h`/`l`).
const SOURCE_FORM_HINT: HintSpec = &[
    Hint::Glyphs(Act::ChoicePrev, Act::ChoiceNext, "source"),
    Hint::One(Act::FieldSubmit, "create"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::One(Act::FormNextField, "field"),
];

/// A form's keys while its request is out (D7, R1b B5): every key but `Esc` is swallowed, and
/// `Esc` only closes the form (nothing out is cancelled).
const BUSY_FORM_HINT: HintSpec = &[Hint::One(Act::FieldCancel, "close")];

/// The hint row of a workspace with nothing to stop or resume (M1's).
const HINT_IDLE: HintSpec = &[
    Hint::One(Act::WorkspacesOpen, "open"),
    Hint::One(Act::WorkspacesNew, "new"),
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::ListFold, "fold"),
];

/// MOD-116 M2 D8: a workspace with live runs and no stop row.
const HINT_STOP: HintSpec = &[
    Hint::One(Act::WorkspacesOpen, "open"),
    Hint::One(Act::WorkspacesStop, "stop"),
    Hint::One(Act::WorkspacesNew, "new"),
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::ListFold, "fold"),
];

/// MOD-116 M2 D8: a graceful stop with a run still running.
const HINT_FORCE: HintSpec = &[
    Hint::One(Act::WorkspacesOpen, "open"),
    Hint::One(Act::WorkspacesStop, "force stop"),
    Hint::One(Act::WorkspacesResume, "resume"),
    Hint::One(Act::WorkspacesNew, "new"),
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::ListFold, "fold"),
];

/// MOD-116 M2 D8: any other stop row.
const HINT_RESUME: HintSpec = &[
    Hint::One(Act::WorkspacesOpen, "open"),
    Hint::One(Act::WorkspacesResume, "resume"),
    Hint::One(Act::WorkspacesNew, "new"),
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::ListFold, "fold"),
];

/// The graceful stop question's keys (D6).
const STOP_HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "stop"),
    Hint::One(Act::ConfirmNo, "cancel"),
];

/// The forced stop question's keys: no writes nothing, and the graceful stop goes on (D6).
const FORCE_HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "stop"),
    Hint::One(Act::ConfirmNo, "keep the graceful stop"),
];

/// The resume question's keys (D6).
const RESUME_HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "resume"),
    Hint::One(Act::ConfirmNo, "cancel"),
];

/// A told box's one answer (D6).
const TOLD_HINT: HintSpec = &[Hint::One(Act::ConfirmNo, "close")];

/// The widest a question's content grows (D6).
const QUESTION_WIDTH: u16 = 72;

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

/// Which key waits on a fresh overview read (MOD-116 M2 D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    /// `workspaces.stop`.
    Stop,
    /// `workspaces.resume`.
    Resume,
    /// `common.edit` (MOD-116 M3 D5): the form opens on the fresh overview.
    Edit,
}

/// What the tab asks, or tells, in a kit `Ask` over its body (MOD-116 M2 D6).
#[derive(Debug, Clone, PartialEq)]
enum Question {
    /// Stop `workspace`'s runs in `mode`, decided on the stop row `decided_on`.
    Stop {
        /// The workspace.
        workspace: WorkspaceId,
        /// Its name, for the title.
        name: String,
        /// The mode a yes writes.
        mode: StopMode,
        /// The stop row the question was built from.
        decided_on: Option<WorkspaceStop>,
        /// The first line, a blank one, and each run with what the stop does to it.
        lines: Vec<Line<'static>>,
    },
    /// Resume `workspace`, decided on the stop row `decided_on`.
    Resume {
        /// The workspace.
        workspace: WorkspaceId,
        /// Its name, for the title.
        name: String,
        /// The stop row the question was built from.
        decided_on: Option<WorkspaceStop>,
        /// The first line, a blank one, and each run with how it resumes.
        lines: Vec<Line<'static>>,
    },
    /// Nothing to ask: why, with one answer that closes it.
    Told {
        /// The box's title.
        title: String,
        /// What it says.
        text: String,
    },
}

/// What `N`'s chooser offers (MOD-116 M3 D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewWhat {
    /// A new workspace.
    Workspace,
    /// A new project in this workspace.
    Project(WorkspaceId),
}

/// What a row of `T`'s picker answers (MOD-116 M3 D9).
#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetPick {
    /// The scope row names no target: the Settings default is inherited.
    Inherit,
    /// The scope row names this target.
    Target(TargetId),
    /// A kind this build does not run: told, never written.
    Planned {
        /// The target's name.
        name: String,
        /// The handoff item that makes its kind runnable.
        owner: String,
    },
}

/// What the tab is doing besides browsing (MOD-116 M3).
#[derive(Debug, Default)]
enum Mode {
    /// The list and the detail.
    #[default]
    Browse,
    /// `N`'s chooser over the tab.
    Choosing(Chooser<NewWhat>),
    /// The new-workspace form in the detail pane.
    Workspace(Box<WorkspaceForm>),
    /// The new-project form in the detail pane.
    Project(Box<ProjectForm>),
    /// T4 (D5): a workspace's edit form in the detail pane.
    Editing(Box<WorkspaceForm>),
    /// T4 (D10): a workspace being deleted, in a box over the tab.
    Deleting(Box<Deleting>),
    /// T4 (D9): `T`'s picker over the tab, for this workspace, with its scope row's token.
    Targeting(
        Chooser<TargetPick>,
        WorkspaceId,
        Option<(ProjectId, DateTime<Utc>)>,
    ),
}

impl Mode {
    /// Whether a form is up.
    const fn has_form(&self) -> bool {
        matches!(
            self,
            Self::Workspace(_) | Self::Project(_) | Self::Editing(_)
        )
    }
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
    /// The overview is read again whatever the counts say (R1 F2): set on activation and by a
    /// list whose projects differ from the held overview's, cleared when a read goes out.
    stale: bool,
    /// Whether the user put the cursor where it is (a list motion, a fold, a restored place); a
    /// cursor the tab placed follows the open workspace (R1 F1).
    user_placed: bool,
    /// The detail pane's scroll.
    scroll: Scroll,
    /// The detail's rows as last drawn, so a scroll is clamped to them.
    detail_rows: Cell<usize>,
    /// MOD-116 M2 D6: the workspace whose `s` or `r` waits on its fresh overview; a cursor moved
    /// onto another workspace drops it.
    pending: Option<(WorkspaceId, Asked)>,
    /// MOD-116 M2 D6: the question or told box over the tab; it holds the keys while up.
    question: Option<Question>,
    /// MOD-116 M3 D3: the home directory paths are shown under (`~`), read once.
    home: Option<PathBuf>,
    /// MOD-116 M3: the chooser or a form, else browsing.
    mode: Mode,
    /// MOD-116 M3 D7: the slug of the `NewProject` out; one at a time.
    creating: Option<String>,
    /// MOD-116 M3 D13: the row a create lands the cursor on once the list holds it.
    land_on: Option<Entry>,
    /// MOD-116 M3 D9: the workspace whose `TargetChoices` is out; one at a time.
    asking_targets: Option<WorkspaceId>,
    /// MOD-116 M3 D10: the open workspace just deleted: the next list opens the first one left.
    reopen: Option<WorkspaceId>,
}

impl WorkspacesTab {
    /// The tab's id; the theme's amber `workspaces` accent is keyed by it.
    pub const ID: TabId = TabId("workspaces");

    /// The tab before its first reply, its paths shown under this user's home (D3).
    #[must_use]
    pub fn new() -> Self {
        Self {
            home: dirs::home_dir(),
            ..Self::default()
        }
    }

    /// This tab with `home` as the directory `~` names (MOD-116 M3 D3): a test's temp home, so
    /// a default folder lands under it and never under the real `~/workspaces`.
    #[must_use]
    pub fn with_home(mut self, home: Option<&Path>) -> Self {
        self.home = home.map(Path::to_path_buf);
        self
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

    /// Moves the cursor to `entry`; another workspace's row resets the detail's scroll and drops
    /// a pending ask about the one it leaves (D6).
    fn select(&mut self, entry: Entry) {
        if self.cursor != Some(entry) {
            self.scroll.reset();
        }
        if self
            .pending
            .is_some_and(|(workspace, _)| workspace != entry.workspace())
        {
            self.pending = None;
        }
        self.cursor = Some(entry);
    }

    /// Lands a cursor the user has not placed on the open workspace's row, or the first
    /// workspace's when the list does not hold it; none while there is no scope yet (R1 F1).
    fn land(&mut self, scope: &Scope) {
        if scope.workspace_id == WorkspaceId::default() {
            self.cursor = None;
            return;
        }
        let landed = self
            .summary(scope.workspace_id)
            .or_else(|| self.workspaces.first())
            .map(|summary| Entry::Workspace(summary.workspace_id));
        match landed {
            Some(entry) => self.select(entry),
            None => self.cursor = None,
        }
    }

    /// Whether the held overview's projects differ from what the list holds for its workspace:
    /// a project added or removed since the read (R1 F2).
    fn overview_lags(&self) -> bool {
        let Some((of, Held::Read(overview))) = &self.overview else {
            return false;
        };
        self.summary(*of).is_some_and(|summary| {
            let listed: BTreeSet<ProjectId> = summary
                .projects
                .iter()
                .map(|project| project.project_id)
                .collect();
            let read: BTreeSet<ProjectId> = overview
                .tree
                .projects
                .iter()
                .map(|entry| entry.project.id)
                .collect();
            listed != read
        })
    }

    /// A list motion over the selectable rows.
    fn move_cursor(&mut self, act: Act) {
        let entries = self.entries();
        let Some(last) = entries.len().checked_sub(1) else {
            return;
        };
        self.user_placed = true;
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
            self.user_placed = true;
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

    /// MOD-116 M2 D6: `s` or `r` on a row of a listed workspace: the question waits on a fresh
    /// overview of it, the one in flight or one asked now whatever the counts say.
    fn ask(&mut self, asked: Asked, ctx: &Ctx<'_>) {
        let Some(workspace) = self
            .cursor
            .filter(|entry| self.holds(*entry))
            .map(Entry::workspace)
        else {
            return;
        };
        self.pending = Some((workspace, asked));
        if self.in_flight != Some(workspace) {
            self.stale = true;
            self.read_if_due(ctx);
        }
    }

    /// MOD-116 M2 D6: the question `asked` raises over `overview`, from its stop report: what is
    /// live and what stop row there is decide which one, or what is told instead.
    fn question_of(
        asked: Asked,
        workspace: WorkspaceId,
        overview: &WorkspaceOverview,
        ctx: &Ctx<'_>,
    ) -> Question {
        let theme = ctx.theme;
        let width = usize::from(QUESTION_WIDTH);
        let name = overview.tree.workspace.name.clone();
        let report = &overview.report;
        let first = |text: &'static str| vec![Line::styled(text, theme.base), Line::default()];
        let told = |title: &str, text: String| Question::Told {
            title: title.to_owned(),
            text,
        };
        match asked {
            Asked::Stop => match report.stop.as_ref().map(|row| row.mode) {
                None if !report.lines.iter().any(question::is_live) => {
                    told(question::STOP_TOLD_TITLE, question::nothing_running(&name))
                }
                None => {
                    let mut lines = first(question::GRACEFUL_FIRST);
                    lines.extend(question::stop_lines(
                        report,
                        StopMode::Graceful,
                        width,
                        theme,
                    ));
                    Question::Stop {
                        workspace,
                        name,
                        mode: StopMode::Graceful,
                        decided_on: None,
                        lines,
                    }
                }
                Some(StopMode::Graceful)
                    if report
                        .lines
                        .iter()
                        .any(|line| line.status == RunStatus::Running) =>
                {
                    let mut lines = first(question::FORCED_FIRST);
                    lines.extend(question::stop_lines(report, StopMode::Forced, width, theme));
                    Question::Stop {
                        workspace,
                        name,
                        mode: StopMode::Forced,
                        decided_on: report.stop.clone(),
                        lines,
                    }
                }
                Some(StopMode::Graceful) => {
                    told(question::STOP_TOLD_TITLE, question::settled(&name))
                }
                Some(mode) => told(
                    question::STOP_TOLD_TITLE,
                    question::already_stopped(&name, mode),
                ),
            },
            // Never raised: `on_reply` opens the edit form on the fresh overview instead.
            Asked::Edit => told(EDIT_TITLE, format!("{name} is edited in its form")),
            Asked::Resume => match &report.stop {
                None => told(question::RESUME_TOLD_TITLE, question::not_stopped(&name)),
                Some(_) => {
                    let mut lines = first(question::RESUME_FIRST);
                    lines.extend(question::resume_lines(report, width, theme));
                    Question::Resume {
                        workspace,
                        name,
                        decided_on: report.stop.clone(),
                        lines,
                    }
                }
            },
        }
    }

    /// Puts up a told box.
    fn tell(&mut self, title: &str, text: String) {
        self.question = Some(Question::Told {
            title: title.to_owned(),
            text,
        });
    }

    /// `workspace`'s name as listed, for a told box.
    fn name_of(&self, workspace: WorkspaceId) -> String {
        self.summary(workspace).map_or_else(
            || "this workspace".to_owned(),
            |summary| summary.name.clone(),
        )
    }

    /// D6: a stop or a resume was written: the states move at once (the counts and the
    /// overview are read again).
    fn written(&mut self, ctx: &Ctx<'_>) {
        ctx.request(StoreRequest::WorkspaceRunCounts);
        self.stale = true;
        self.read_if_due(ctx);
    }

    /// D3/D6: a write refused because the stop row moved while asking.
    fn changed(&mut self, workspace: WorkspaceId, stop: bool, ctx: &Ctx<'_>) {
        let act = if stop {
            Act::WorkspacesStop
        } else {
            Act::WorkspacesResume
        };
        let key = ctx.keys().label(views::WORKSPACES_BROWSE, act);
        let title = if stop {
            question::STOP_TOLD_TITLE
        } else {
            question::RESUME_TOLD_TITLE
        };
        let text = question::changed(&self.name_of(workspace), stop, key.as_deref());
        self.tell(title, text);
    }

    /// D6: a key while the question is up, through [`views::WORKSPACES_CONFIRM`]: `confirm.yes`
    /// sends the write the question was decided on and closes it (a told box just closes),
    /// `confirm.no` closes it, and every other chord is `modal_rest`'s.
    fn question_key(&mut self, question: Question, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::WORKSPACES_CONFIRM;
        let chord = ctx.pressed(&key);
        let act = ctx.keys().actions(stack, chord).first().copied();
        match (act, question) {
            (
                Some(Act::ConfirmYes),
                Question::Stop {
                    workspace,
                    mode,
                    decided_on,
                    ..
                },
            ) => ctx.request(StoreRequest::StopWorkspaces {
                mode,
                targets: vec![StopTarget {
                    workspace,
                    decided_on,
                }],
            }),
            (
                Some(Act::ConfirmYes),
                Question::Resume {
                    workspace,
                    decided_on,
                    ..
                },
            ) => ctx.request(StoreRequest::ResumeWorkspace {
                workspace,
                decided_on,
            }),
            (Some(Act::ConfirmYes | Act::ConfirmNo), _) => {}
            (_, question) => {
                self.question = Some(question);
                return modal_rest(stack, chord);
            }
        }
        Handled::Consumed
    }

    /// D8: the selected workspace's counts row, as the hint row reads it: none offline.
    fn selected_counts<'a>(&self, ctx: &Ctx<'a>) -> Option<&'a WorkspaceRunCounts> {
        self.cursor
            .and_then(|entry| counts_of(ctx, entry.workspace()))
            .filter(|_| !ctx.top_bar.offline())
    }

    /// The browse hint row (D8), or none while a question, the chooser or a form is up (its box
    /// or its pane carries its keys).
    fn browse_hints(&self, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        if self.question.is_some() || !matches!(self.mode, Mode::Browse) {
            return None;
        }
        let spec = hint_for(self.selected_counts(ctx));
        key_line(ctx.keys().hint(views::WORKSPACES_BROWSE, spec), ctx.theme)
    }

    /// D6: the question or told box over `area`, its keys on its border.
    fn render_question(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let Some(question) = &self.question else {
            return;
        };
        let told;
        let (title, lines, spec) = match question {
            Question::Stop {
                name, mode, lines, ..
            } => match mode {
                StopMode::Graceful => (question::stop_title(name), lines, STOP_HINT),
                StopMode::Forced => (question::force_title(name), lines, FORCE_HINT),
            },
            Question::Resume { name, lines, .. } => {
                (question::resume_title(name), lines, RESUME_HINT)
            }
            Question::Told { title, text } => {
                told = vec![Line::styled(text.clone(), ctx.theme.base)];
                (title.clone(), &told, TOLD_HINT)
            }
        };
        let hint = ctx.keys().hint(views::WORKSPACES_CONFIRM, spec);
        let mut ask = Ask::new(&title).width(QUESTION_WIDTH).hint(&hint);
        for line in lines {
            ask = ask.line(line.clone());
        }
        let _ = ask.render(frame, area, ctx.theme);
    }

    /// D7: asks for the selected workspace's overview when none is out, there is a scope, and
    /// either none is held for it (a refusal is not held, R1 F3), it is [`stale`](Self::stale),
    /// or its counts row moved since it was asked for.
    fn read_if_due(&mut self, ctx: &Ctx<'_>) {
        if self.in_flight.is_some() || ctx.scope.workspace_id == WorkspaceId::default() {
            return;
        }
        let Some(workspace) = self.cursor.map(Entry::workspace) else {
            return;
        };
        let row = counts_of(ctx, workspace);
        let held =
            matches!(&self.overview, Some((of, Held::Read(_) | Held::Gone)) if *of == workspace);
        if held && !self.stale && (row.is_none() || row == self.read_against.as_ref()) {
            return;
        }
        self.stale = false;
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
                counts_of(ctx, id).filter(|_| !ctx.top_bar.offline()),
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

    /// What the list says with no row, and its next step: reading, or no workspace and the key
    /// that creates one here (MOD-116 M3 D1).
    fn empty_state(&self, ctx: &Ctx<'_>) -> (&'static str, String) {
        if !self.loaded {
            return (READING, READING_NEXT.to_owned());
        }
        let next = match ctx
            .keys()
            .label(views::WORKSPACES_BROWSE, Act::WorkspacesNew)
        {
            Some(new) => format!("`{new}` creates one"),
            None => "a workspace created elsewhere shows up here".to_owned(),
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

        let reading = |frame: &mut Frame<'_>| {
            empty::render(
                frame,
                inner,
                READING_OVERVIEW,
                NextStep(READING_OVERVIEW_NEXT),
                theme,
            );
        };
        let mut lines: Vec<Line<'static>> = Vec::new();
        match (cursor, held) {
            (None, _) if self.workspaces.is_empty() => {
                let (what, next) = self.empty_state(ctx);
                empty::render(frame, inner, what, NextStep(&next), theme);
            }
            // R1 F1: listed, but no scope to land the cursor by yet.
            (None, _) => reading(frame),
            (Some(Entry::Project(_, project)), Some(Held::Read(overview))) => {
                // R1 F2: a project the list holds and the overview does not is still being read.
                match overview
                    .tree
                    .projects
                    .iter()
                    .find(|candidate| candidate.project.id == project)
                {
                    Some(found) => lines = overview::project_lines(overview, found, width, theme),
                    None => reading(frame),
                }
            }
            (Some(entry), Some(Held::Read(overview))) => {
                // MOD-116 M2 D7: offline the overview's counts are the last read's: a workspace
                // that is not open shows no state, the open one `● open` alone, and neither
                // its runs.
                let open = ctx.scope.workspace_id == entry.workspace();
                let current = !ctx.top_bar.offline();
                let state = match (current, open) {
                    (true, _) => Some(WorkspaceLifecycle::of(open, &overview.counts)),
                    (false, true) => Some(WorkspaceLifecycle::Open { running: 0 }),
                    (false, false) => None,
                };
                let overview = self.under_home(overview);
                lines = overview::workspace_lines(&overview, state, current, width, theme);
            }
            (Some(_), Some(Held::Refused(message))) => {
                let what = format!("{NEEDS_THE_DATABASE}: {message}");
                empty::render_failed(frame, inner, &what, NextStep(REFUSED_NEXT), theme);
            }
            (Some(_), Some(Held::Gone)) => {
                empty::render(frame, inner, GONE, NextStep(GONE_NEXT), theme);
            }
            (Some(_), None) => reading(frame),
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

    /// MOD-116 M3 D3: `overview` with its folder shown under `~` when it is under the home.
    fn under_home<'o>(&self, overview: &'o WorkspaceOverview) -> Cow<'o, WorkspaceOverview> {
        match (&overview.folder, self.home.as_deref()) {
            (Some(folder), Some(home)) if folder.starts_with(home) => {
                let mut under = overview.clone();
                under.folder = Some(PathBuf::from(shown(folder, Some(home))));
                Cow::Owned(under)
            }
            _ => Cow::Borrowed(overview),
        }
    }

    /// MOD-116 M3 D3: `text` (a worker's sentence) with every path under the home written `~/…`.
    fn under_home_text(&self, text: &str) -> String {
        match self.home.as_deref() {
            Some(home) => {
                let home = home.display().to_string();
                text.replace(&format!("{}/", home.trim_end_matches('/')), "~/")
            }
            None => text.to_owned(),
        }
    }

    /// MOD-116 M3 D1: `N`: the chooser, `workspace` then `project in <name>` for the selected
    /// row's workspace; with no workspace listed, the workspace form at once.
    fn begin_new(&mut self) {
        if self.workspaces.is_empty() {
            self.mode = Mode::Workspace(Box::new(WorkspaceForm::new(self.home.as_deref())));
            return;
        }
        let mut rows = vec![ChoiceRow {
            label: "workspace".to_owned(),
            note: None,
            enabled: true,
            value: NewWhat::Workspace,
        }];
        if let Some(summary) = self
            .cursor
            .filter(|entry| self.holds(*entry))
            .and_then(|entry| self.summary(entry.workspace()))
        {
            rows.push(ChoiceRow {
                label: format!("project in {}", summary.name),
                note: None,
                enabled: true,
                value: NewWhat::Project(summary.workspace_id),
            });
        }
        self.mode = Mode::Choosing(Chooser::new(NEW_TITLE, rows, 0));
    }

    /// The chooser's answer: the form it names; a second project while one is out is told (D7).
    fn chosen(&mut self, what: NewWhat) {
        self.mode = match what {
            NewWhat::Workspace => {
                Mode::Workspace(Box::new(WorkspaceForm::new(self.home.as_deref())))
            }
            NewWhat::Project(_) if self.creating.is_some() => {
                self.tell(NEW_PROJECT_TITLE, STILL_CREATING.to_owned());
                Mode::Browse
            }
            NewWhat::Project(workspace) => {
                let folder = match &self.overview {
                    Some((of, Held::Read(overview))) if *of == workspace => overview.folder.clone(),
                    _ => None,
                };
                Mode::Project(Box::new(ProjectForm::new(
                    workspace,
                    &self.name_of(workspace),
                    folder,
                    self.home.as_deref(),
                )))
            }
        };
    }

    /// MOD-116 M3: a key while the chooser ([`views::WORKSPACES_PICK`]) or a form
    /// ([`views::WORKSPACES_FORM`]) is up; `None` while browsing.
    fn mode_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Option<Handled> {
        let pressed = ctx.pressed(&key);
        let keys = ctx.keys();
        let outcome = match &mut self.mode {
            Mode::Browse => return None,
            Mode::Choosing(chooser) => {
                let stack = views::WORKSPACES_PICK;
                let act = keys.actions(stack, pressed).first().copied();
                let Some(act @ (Act::ListDown | Act::ListUp | Act::ConfirmYes | Act::ConfirmNo)) =
                    act
                else {
                    return Some(modal_rest(stack, pressed));
                };
                match chooser.on_act(act) {
                    ChooserOutcome::Stay | ChooserOutcome::Disabled(_) => {}
                    ChooserOutcome::Chosen(what) => self.chosen(what),
                    ChooserOutcome::Closed => self.mode = Mode::Browse,
                }
                return Some(Handled::Consumed);
            }
            Mode::Targeting(chooser, workspace, scope_row) => {
                let stack = views::WORKSPACES_PICK;
                let act = keys.actions(stack, pressed).first().copied();
                let Some(act @ (Act::ListDown | Act::ListUp | Act::ConfirmYes | Act::ConfirmNo)) =
                    act
                else {
                    return Some(modal_rest(stack, pressed));
                };
                let (workspace, scope_row) = (*workspace, *scope_row);
                match chooser.on_act(act) {
                    ChooserOutcome::Stay => {}
                    ChooserOutcome::Disabled(index) => {
                        if let Some(TargetPick::Planned { name, owner }) =
                            chooser.row(index).map(|row| row.value.clone())
                        {
                            self.tell(TARGET_TITLE, format!("{name} runs once {owner} lands"));
                        }
                    }
                    ChooserOutcome::Chosen(pick) => {
                        self.mode = Mode::Browse;
                        self.set_target(workspace, scope_row, pick, ctx);
                    }
                    ChooserOutcome::Closed => self.mode = Mode::Browse,
                }
                return Some(Handled::Consumed);
            }
            Mode::Deleting(deleting) => {
                let outcome = deleting.on_key(key, pressed, keys);
                return Some(self.delete_outcome(outcome, ctx));
            }
            Mode::Workspace(form) | Mode::Editing(form) => form.on_key(key, pressed, keys),
            Mode::Project(form) => form.on_key(key, pressed, keys),
        };
        Some(self.form_outcome(outcome, ctx))
    }

    /// MOD-116 M3 D5: `e` on a workspace's group or `overview` row: its form, prefilled from the
    /// held overview when it is current, else once a fresh one lands; on a project row, where
    /// projects are edited (D12).
    fn begin_edit(&mut self, ctx: &Ctx<'_>) {
        let Some(entry) = self.cursor.filter(|entry| self.holds(*entry)) else {
            return;
        };
        if let Entry::Project(..) = entry {
            self.tell(EDIT_TITLE, PROJECTS_IN_SETTINGS.to_owned());
            return;
        }
        let workspace = entry.workspace();
        let current = !self.stale && self.in_flight != Some(workspace);
        match &self.overview {
            Some((of, Held::Read(overview))) if *of == workspace && current => {
                let form = WorkspaceForm::edit(
                    &overview.tree.workspace,
                    overview.folder.as_deref(),
                    self.home.as_deref(),
                );
                self.mode = Mode::Editing(Box::new(form));
            }
            _ => self.ask(Asked::Edit, ctx),
        }
    }

    /// MOD-116 M3 D5: the fresh overview `asked` waited on has landed: the edit form opens on
    /// it, unless something else is up meanwhile.
    fn open_edit(&mut self, overview: &WorkspaceOverview) {
        if matches!(self.mode, Mode::Browse) && self.question.is_none() {
            let form = WorkspaceForm::edit(
                &overview.tree.workspace,
                overview.folder.as_deref(),
                self.home.as_deref(),
            );
            self.mode = Mode::Editing(Box::new(form));
        }
    }

    /// MOD-116 M3 D5: the edit was written: the form closes, the list is read again (the shell
    /// takes a rename of the open workspace from it, D11) and so is the overview.
    fn edited(&mut self, workspace: WorkspaceId, ctx: &Ctx<'_>) {
        if matches!(&self.mode, Mode::Editing(form) if form.editing() == Some(workspace)) {
            self.mode = Mode::Browse;
        }
        ctx.request(StoreRequest::Workspaces);
        self.stale = true;
        self.read_if_due(ctx);
    }

    /// MOD-116 M3 D5: the row changed since the form opened: the text stays, the form takes the
    /// row's new token and says so, and the next `Enter` saves over it.
    fn edit_stale(
        &mut self,
        workspace: WorkspaceId,
        current: &htui_core::model::Workspace,
        ctx: &Ctx<'_>,
    ) {
        let retry = ctx.keys().label(views::WORKSPACES_FORM, Act::FieldSubmit);
        match &mut self.mode {
            Mode::Editing(form) if form.editing() == Some(workspace) && form.is_busy() => {
                form.retoken(current);
                form.refused(&changed_elsewhere(retry.as_deref()));
            }
            _ => {
                let text = format!(
                    "{} changed elsewhere meanwhile; nothing was saved",
                    current.name
                );
                self.tell(EDIT_TITLE, text);
            }
        }
    }

    /// MOD-116 M3 D10: `d` on a workspace's group or `overview` row: the count goes out and the
    /// box says what it would take; on a project row, where projects are edited (D12).
    fn begin_delete(&mut self, ctx: &Ctx<'_>) {
        let Some(entry) = self.cursor.filter(|entry| self.holds(*entry)) else {
            return;
        };
        if let Entry::Project(..) = entry {
            self.tell(DELETE_TITLE, PROJECTS_IN_SETTINGS.to_owned());
            return;
        }
        let workspace = entry.workspace();
        let Some(slug) = self.summary(workspace).map(|summary| summary.slug.clone()) else {
            return;
        };
        let folder = match &self.overview {
            Some((of, Held::Read(overview))) if *of == workspace => overview
                .folder
                .as_deref()
                .map(|folder| shown(folder, self.home.as_deref())),
            _ => None,
        };
        let (deleting, request) = Deleting::begin(workspace, slug, folder);
        self.mode = Mode::Deleting(Box::new(deleting));
        ctx.request(request);
    }

    /// MOD-116 M3 D10: what a delete's key or reply does to the tab.
    fn delete_outcome(&mut self, outcome: DeleteOutcome, ctx: &Ctx<'_>) -> Handled {
        match outcome {
            DeleteOutcome::Stay => {}
            DeleteOutcome::Pass => return Handled::Pass,
            DeleteOutcome::Send(request) => ctx.request(request),
            DeleteOutcome::Closed => self.mode = Mode::Browse,
            DeleteOutcome::Told(text) => {
                self.mode = Mode::Browse;
                self.tell(DELETE_TITLE, text);
                ctx.request(StoreRequest::Workspaces);
            }
            DeleteOutcome::Deleted { workspace } => {
                self.mode = Mode::Browse;
                if workspace == ctx.scope.workspace_id {
                    self.reopen = Some(workspace);
                }
                ctx.request(StoreRequest::Workspaces);
                ctx.request(StoreRequest::WorkspaceRunCounts);
            }
        }
        Handled::Consumed
    }

    /// MOD-116 M3 D9: `T` on any row of a workspace reads what its default target may be; the
    /// picker opens on the reply. One read out at a time.
    fn ask_targets(&mut self, ctx: &Ctx<'_>) {
        if self.asking_targets.is_some() {
            return;
        }
        let Some(workspace) = self
            .cursor
            .filter(|entry| self.holds(*entry))
            .map(Entry::workspace)
        else {
            return;
        };
        self.asking_targets = Some(workspace);
        ctx.request(StoreRequest::TargetChoices(workspace));
    }

    /// MOD-116 M3 D9: the picker over `choices`: the Settings default first, then every live
    /// target with its kind; a kind this build does not run is dim and says which item runs it.
    /// The current choice is selected and marked.
    fn open_targets(&mut self, workspace: WorkspaceId, choices: &TargetChoices) {
        const CURRENT: &str = "\u{b7} current";
        let mut rows = vec![ChoiceRow {
            label: format!("inherit the Settings default ({})", choices.default_name),
            note: choices.current.is_none().then(|| CURRENT.to_owned()),
            enabled: true,
            value: TargetPick::Inherit,
        }];
        let mut selected = 0;
        for target in &choices.targets {
            let current = choices.current == Some(target.id);
            if current {
                selected = rows.len();
            }
            let mut notes = Vec::new();
            if let Some(owner) = target.planned {
                notes.push(format!("planned \u{b7} {owner}"));
            }
            if current {
                notes.push(CURRENT.to_owned());
            }
            rows.push(ChoiceRow {
                label: format!("{}  {}", target.name, target.kind.as_str()),
                note: (!notes.is_empty()).then(|| notes.join(" ")),
                enabled: target.planned.is_none(),
                value: target
                    .planned
                    .map_or(TargetPick::Target(target.id), |owner| TargetPick::Planned {
                        name: target.name.clone(),
                        owner: owner.to_owned(),
                    }),
            });
        }
        let title = format!("Default target of {}", self.name_of(workspace));
        self.mode = Mode::Targeting(
            Chooser::new(title, rows, selected),
            workspace,
            choices.scope_row,
        );
    }

    /// MOD-116 M3 D9: the picker's choice is written on the scope row it was read with; a
    /// workspace with no scope row is told.
    fn set_target(
        &mut self,
        workspace: WorkspaceId,
        scope_row: Option<(ProjectId, DateTime<Utc>)>,
        pick: TargetPick,
        ctx: &Ctx<'_>,
    ) {
        let target = match pick {
            TargetPick::Inherit => None,
            TargetPick::Target(id) => Some(id),
            TargetPick::Planned { .. } => return,
        };
        let Some((_, expected)) = scope_row else {
            let text = format!(
                "{} has no scope row, so its default target cannot be set",
                self.name_of(workspace)
            );
            self.tell(TARGET_TITLE, text);
            return;
        };
        ctx.request(StoreRequest::SetWorkspaceTarget {
            workspace,
            expected,
            target,
        });
    }

    /// MOD-116 M3 D9: the scope row moved while the picker was up: nothing was written.
    fn target_stale(&mut self, workspace: WorkspaceId, ctx: &Ctx<'_>) {
        let again = ctx
            .keys()
            .label(views::WORKSPACES_BROWSE, Act::WorkspacesTarget)
            .map_or_else(
                || "ask again".to_owned(),
                |key| format!("press {key} again"),
            );
        let text = format!(
            "the default target of {} changed meanwhile; {again}",
            self.name_of(workspace)
        );
        self.tell(TARGET_TITLE, text);
    }

    /// What a form's key does to the tab: a submit is sent (a project's slug held while it is
    /// out, D7); `Esc` closes the form, and a project still out is told it is (D7).
    fn form_outcome(&mut self, outcome: FormOutcome, ctx: &Ctx<'_>) -> Handled {
        match outcome {
            FormOutcome::Stay | FormOutcome::Refused => {}
            FormOutcome::Submit(request) => {
                if let StoreRequest::NewProject { slug, .. } = request.as_ref() {
                    self.creating = Some(slug.clone());
                }
                ctx.request(*request);
            }
            FormOutcome::Cancel => {
                let out = matches!(&self.mode, Mode::Project(form) if form.is_busy());
                self.mode = Mode::Browse;
                if out {
                    self.tell(NEW_PROJECT_TITLE, TOLD_WHEN_IT_LANDS.to_owned());
                }
            }
            FormOutcome::Pass => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// MOD-116 M3 D4: the workspace exists: the form closes, the list is read again and the
    /// cursor lands on its group. It is not opened, unless the shell's scope names no workspace
    /// the tab lists (a first start), so the shell is never left without one.
    fn workspace_created(&mut self, workspace: &WorkspaceSummary, ctx: &Ctx<'_>) {
        // R1b B4: only the form waiting on it; one opened since the first was closed stays.
        if matches!(&self.mode, Mode::Workspace(form) if form.is_busy()) {
            self.mode = Mode::Browse;
        }
        self.land_on = Some(Entry::Workspace(workspace.workspace_id));
        ctx.request(StoreRequest::Workspaces);
        // Its state (`○ stopped`, PRD Q4) reads from the counts, which list it from now on.
        ctx.request(StoreRequest::WorkspaceRunCounts);
        if self.summary(ctx.scope.workspace_id).is_none() {
            ctx.emit(Action::SetScope {
                workspace: workspace.clone(),
            });
        }
    }

    /// MOD-116 M3 D6, D13: the project exists: its form closes (or, closed already, the outcome
    /// is told), the list and the overview are read again and the cursor lands on its row.
    fn project_created(
        &mut self,
        workspace: WorkspaceId,
        project: &ProjectRef,
        repo_path: &str,
        ctx: &Ctx<'_>,
    ) {
        self.creating = None;
        let its_form = matches!(
            &self.mode,
            Mode::Project(form)
                if form.workspace() == workspace && form.slug() == project.slug && form.is_busy()
        );
        if its_form {
            self.mode = Mode::Browse;
        } else {
            let text = format!(
                "Project `{}` is ready in {}: {}",
                project.slug,
                self.name_of(workspace),
                shown(Path::new(repo_path), self.home.as_deref())
            );
            self.tell(NEW_PROJECT_TITLE, text);
        }
        self.land_on = Some(Entry::Project(workspace, project.project_id));
        self.stale = true;
        ctx.request(StoreRequest::Workspaces);
    }

    /// MOD-116 M3 D13: a create or an edit refused (a sentence) or failed (the store's message):
    /// under the form while it is open and waiting, its text kept, else in the box. A refused
    /// target read or write (D9) is told.
    fn setup_refused(&mut self, request: &str, why: &str) {
        let why = &self.under_home_text(why);
        match request {
            NEW_PROJECT => {
                let slug = self.creating.take();
                if let Mode::Project(form) = &mut self.mode
                    && form.is_busy()
                {
                    form.refused(why);
                    return;
                }
                let text = match slug {
                    Some(slug) => format!("Project `{slug}` was not created: {why}"),
                    None => format!("The project was not created: {why}"),
                };
                self.tell(NEW_PROJECT_TITLE, text);
            }
            NEW_WORKSPACE => {
                if let Mode::Workspace(form) = &mut self.mode
                    && form.is_busy()
                {
                    form.refused(why);
                    return;
                }
                self.tell(
                    NEW_WORKSPACE_TITLE,
                    format!("The workspace was not created: {why}"),
                );
            }
            EDIT_WORKSPACE => {
                if let Mode::Editing(form) = &mut self.mode
                    && form.is_busy()
                {
                    form.refused(why);
                    return;
                }
                self.tell(EDIT_TITLE, format!("The workspace was not saved: {why}"));
            }
            TARGET_CHOICES => {
                self.asking_targets = None;
                self.tell(TARGET_TITLE, format!("The targets cannot be read: {why}"));
            }
            _ => self.tell(
                TARGET_TITLE,
                format!("The default target was not set: {why}"),
            ),
        }
    }

    /// MOD-116 M3 D11 (Hierarchy's D11, mirrored): the open workspace's listed projects differ
    /// from the scope's, so the scope follows the list (a project created here reaches the
    /// Backlog). The shell refuses it while an in-pane editor is open.
    fn follow_scope(&self, ctx: &Ctx<'_>) {
        let Some(summary) = self.summary(ctx.scope.workspace_id) else {
            return;
        };
        if Scope::from_workspace(summary).project_ids != ctx.scope.project_ids {
            ctx.emit(Action::SetScope {
                workspace: summary.clone(),
            });
        }
    }

    /// MOD-116 M3: the open form's keys, through [`views::WORKSPACES_FORM`]: the source choice's
    /// own first while it has the focus, `Esc close` alone while the request is out (R1b B1, B5);
    /// none without a form or while a question is up.
    fn form_hints(&self, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        let spec = match (&self.question, &self.mode) {
            (None, Mode::Workspace(form) | Mode::Editing(form)) if form.is_busy() => BUSY_FORM_HINT,
            (None, Mode::Project(form)) if form.is_busy() => BUSY_FORM_HINT,
            (None, Mode::Workspace(_)) => WORKSPACE_FORM_HINT,
            (None, Mode::Editing(_)) => EDIT_FORM_HINT,
            (None, Mode::Project(form)) if form.on_source() => SOURCE_FORM_HINT,
            (None, Mode::Project(_)) => PROJECT_FORM_HINT,
            _ => return None,
        };
        key_line(ctx.keys().hint(views::WORKSPACES_FORM, spec), ctx.theme)
    }

    /// MOD-116 M3: the open form in `area`, titled for what it creates, `hints` on the border.
    fn render_form(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focus: Focus,
        hints: Option<&Line<'static>>,
        ctx: &Ctx<'_>,
    ) {
        let inner = Pane::inner(area);
        let (title, lines) = match &self.mode {
            Mode::Workspace(form) | Mode::Editing(form) => {
                (form.title(), form.lines(inner.width, ctx))
            }
            Mode::Project(form) => (
                format!("New project in {}", form.workspace_name()),
                form.lines(inner.width, ctx),
            ),
            Mode::Browse | Mode::Choosing(_) | Mode::Deleting(_) | Mode::Targeting(..) => return,
        };
        frame.render_widget(Paragraph::new(lines), inner);
        let mut pane = Pane::new(&title).focus(focus).accent(ctx.accent());
        if let Some(hints) = hints {
            pane = pane.hints(hints.clone());
        }
        pane.render(frame, area, ctx.theme);
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
fn projects_of(summary: &WorkspaceSummary) -> Vec<&ProjectRef> {
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

/// `workspace`'s state (D2) from the shell's counts: with none read yet, or offline (the counts
/// held are the last ones read, not current: D8 over D5, R1 F6), the open workspace still reads
/// `open` and the others have no state.
fn lifecycle(ctx: &Ctx<'_>, workspace: WorkspaceId) -> Option<WorkspaceLifecycle> {
    let open = ctx.scope.workspace_id == workspace;
    let row = counts_of(ctx, workspace).filter(|_| !ctx.top_bar.offline());
    match row {
        Some(row) => Some(WorkspaceLifecycle::of(open, row)),
        None if open => Some(WorkspaceLifecycle::Open { running: 0 }),
        None => None,
    }
}

/// MOD-116 M2 D8: the hint row `row` (the selected workspace's counts, `None` offline) offers:
/// stop while runs are live and nothing stops them, force stop and resume while a graceful stop
/// still has a run running, resume under any other stop, M1's otherwise.
fn hint_for(row: Option<&WorkspaceRunCounts>) -> HintSpec {
    let Some(row) = row else {
        return HINT_IDLE;
    };
    match &row.stop {
        None if row.live() > 0 => HINT_STOP,
        None => HINT_IDLE,
        Some(stop) if stop.mode == StopMode::Graceful && row.running > 0 => HINT_FORCE,
        Some(_) => HINT_RESUME,
    }
}

/// A workspace's group row: `▾ Platform  ◐ background · 1 running`, the name padded to `column`;
/// `counts` (none offline) carries the open workspace's stop (D7).
fn group_row(
    summary: &WorkspaceSummary,
    column: usize,
    folded: bool,
    state: Option<WorkspaceLifecycle>,
    counts: Option<&WorkspaceRunCounts>,
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
        spans.extend(overview::state_spans(state, counts, theme));
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
    /// reads `open` moves, and that is derived at render). A cursor the user has not moved lands
    /// on the newly open workspace (R1 F1); its overview is read on the activation that follows.
    fn on_scope_change(&mut self, scope: &Scope) {
        if self.loaded && !self.user_placed {
            self.land(scope);
        }
    }

    /// Every key resolves through [`views::WORKSPACES_BROWSE`]: a list motion moves the cursor
    /// (or, with the detail focused, scrolls it), `list.fold` folds the group, the pane scroll
    /// moves the detail, `workspaces.open` opens the selected workspace, and `workspaces.stop`
    /// and `workspaces.resume` ask (D6). While a question is up, through
    /// [`views::WORKSPACES_CONFIRM`].
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if let Some(question) = self.question.take() {
            return self.question_key(question, key, ctx);
        }
        if let Some(handled) = self.mode_key(key, ctx) {
            return handled;
        }
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
                Act::WorkspacesStop => self.ask(Asked::Stop, ctx),
                Act::WorkspacesResume => self.ask(Asked::Resume, ctx),
                Act::WorkspacesNew => self.begin_new(),
                Act::Edit => self.begin_edit(ctx),
                Act::Delete => self.begin_delete(ctx),
                Act::WorkspacesTarget => self.ask_targets(ctx),
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
                if self.overview_lags() {
                    self.stale = true;
                }
                // The user's row is kept while the list holds it; any other cursor follows the
                // open workspace (R1 F1).
                if !(self.user_placed && self.cursor.is_some_and(|entry| self.holds(entry))) {
                    self.user_placed = false;
                    self.land(ctx.scope);
                }
                // MOD-116 M3 D13: a row just created is selected once the list holds it.
                if let Some(entry) = self.land_on.filter(|entry| self.holds(*entry)) {
                    self.land_on = None;
                    self.folded.remove(&entry.workspace());
                    self.user_placed = true;
                    self.select(entry);
                }
                // MOD-116 M3 D10: the open workspace was deleted here: the first one left is
                // opened (none leaves the empty state).
                if let Some(gone) = self.reopen.take()
                    && ctx.scope.workspace_id == gone
                    && let Some(first) = self
                        .workspaces
                        .iter()
                        .find(|summary| summary.workspace_id != gone)
                {
                    ctx.emit(Action::SetScope {
                        workspace: first.clone(),
                    });
                }
                self.follow_scope(ctx);
                self.read_if_due(ctx);
            }
            StoreReply::WorkspaceOverview {
                workspace,
                overview,
            } => {
                if self.in_flight == Some(*workspace) {
                    self.in_flight = None;
                }
                // D6: the pending ask's fresh overview raises its question; R1 F6: a workspace
                // gone meanwhile is told as gone.
                if let Some((_, asked)) = self.pending.take_if(|(pending, _)| pending == workspace)
                {
                    match overview {
                        // MOD-116 M3 D5: the edit form opens on it.
                        Some(read) if asked == Asked::Edit => self.open_edit(read),
                        Some(read) => {
                            self.question = Some(Self::question_of(asked, *workspace, read, ctx));
                        }
                        None => {
                            let title = match asked {
                                Asked::Stop => question::STOP_TOLD_TITLE,
                                Asked::Resume => question::RESUME_TOLD_TITLE,
                                Asked::Edit => EDIT_TITLE,
                            };
                            let text = format!("{} no longer exists", self.name_of(*workspace));
                            self.tell(title, text);
                        }
                    }
                }
                let held = overview
                    .as_ref()
                    .map_or(Held::Gone, |overview| Held::Read(overview.clone()));
                self.overview = Some((*workspace, held));
                self.read_if_due(ctx);
            }
            StoreReply::StopsWritten { outcomes, .. } => {
                for (workspace, outcome) in outcomes {
                    match outcome {
                        StopWrite::Written => self.written(ctx),
                        StopWrite::Already(mode) => {
                            let text = question::already_stopped(&self.name_of(*workspace), *mode);
                            self.tell(question::STOP_TOLD_TITLE, text);
                        }
                        StopWrite::Changed => self.changed(*workspace, true, ctx),
                        StopWrite::Failed(message) => self.tell(
                            question::STOP_TOLD_TITLE,
                            question::needs_the_database(true, message),
                        ),
                    }
                }
            }
            StoreReply::ResumeWritten { workspace, outcome } => match outcome {
                ResumeWrite::Resumed => self.written(ctx),
                ResumeWrite::NotStopped => {
                    let text = question::not_stopped(&self.name_of(*workspace));
                    self.tell(question::RESUME_TOLD_TITLE, text);
                }
                ResumeWrite::Changed => self.changed(*workspace, false, ctx),
            },
            StoreReply::Failed { request, message } if *request == STOP_WORKSPACES => {
                self.tell(
                    question::STOP_TOLD_TITLE,
                    question::needs_the_database(true, message),
                );
            }
            StoreReply::Failed { request, message } if *request == RESUME_WORKSPACE => {
                self.tell(
                    question::RESUME_TOLD_TITLE,
                    question::needs_the_database(false, message),
                );
            }
            StoreReply::Failed { request, message } if *request == WORKSPACE_OVERVIEW => {
                if let Some(workspace) = self.in_flight.take() {
                    // D6: the pending ask's read was refused: said in the box.
                    if let Some((_, asked)) =
                        self.pending.take_if(|(pending, _)| *pending == workspace)
                    {
                        match asked {
                            // MOD-116 M3 D5: the edit's own sentence.
                            Asked::Edit => self
                                .tell(EDIT_TITLE, format!("Editing needs the database: {message}")),
                            Asked::Stop => self.tell(
                                question::STOP_TOLD_TITLE,
                                question::needs_the_database(true, message),
                            ),
                            Asked::Resume => self.tell(
                                question::RESUME_TOLD_TITLE,
                                question::needs_the_database(false, message),
                            ),
                        }
                    }
                    self.overview = Some((workspace, Held::Refused(message.clone())));
                    // R1 F3: the refused workspace is asked again on the next refresh, not at
                    // once, which would ask on every refusal; another one now selected is read.
                    if self.cursor.map(Entry::workspace) != Some(workspace) {
                        self.read_if_due(ctx);
                    }
                }
            }
            // The shell has set the top bar from it already (D5); a moved row reads again.
            StoreReply::WorkspaceRunCounts(_) => self.read_if_due(ctx),
            StoreReply::WorkspaceCreated { workspace, .. } => {
                self.workspace_created(workspace, ctx)
            }
            StoreReply::ProjectCreated {
                workspace,
                project,
                repo_path,
            } => self.project_created(*workspace, project, repo_path, ctx),
            StoreReply::WorkspaceEdited { workspace, outcome } => match outcome {
                EditOutcome::Applied { .. } => self.edited(*workspace, ctx),
                EditOutcome::Stale(current) => self.edit_stale(*workspace, current, ctx),
            },
            StoreReply::TargetChoices { workspace, choices } => {
                if self
                    .asking_targets
                    .take_if(|asked| asked == workspace)
                    .is_some()
                    && matches!(self.mode, Mode::Browse)
                {
                    self.open_targets(*workspace, choices);
                }
            }
            StoreReply::WorkspaceTargetSet { workspace, outcome } => match outcome {
                // The overview's `default target` line follows.
                TargetWrite::Applied => {
                    self.stale = true;
                    self.read_if_due(ctx);
                }
                TargetWrite::Stale => self.target_stale(*workspace, ctx),
            },
            StoreReply::SetupRefused { request, reason }
                if workspace_setup::REQUEST_NAMES.contains(request) =>
            {
                self.setup_refused(request, reason);
            }
            StoreReply::Failed { request, message }
                if workspace_setup::REQUEST_NAMES.contains(request) =>
            {
                self.setup_refused(request, message);
            }
            // MOD-116 M3 D10: the delete's count, its write, and their refusals.
            StoreReply::DeleteReach(_) | StoreReply::Deleted { .. } => {
                if let Mode::Deleting(deleting) = &mut self.mode {
                    let outcome = deleting.on_reply(reply);
                    let _ = self.delete_outcome(outcome, ctx);
                }
            }
            StoreReply::Failed { request, .. }
                if *request == DELETE_REACH || *request == DELETE_WORKSPACE =>
            {
                if let Mode::Deleting(deleting) = &mut self.mode {
                    let outcome = deleting.on_reply(reply);
                    let _ = self.delete_outcome(outcome, ctx);
                }
            }
            _ => {}
        }
    }

    /// The detail pane says a refused overview itself; a refused counts read leaves the states
    /// as they were (the shell's own poll is absorbed too, D5); a refused stop or resume is told
    /// in the tab's box (MOD-116 M2 D6).
    ///
    /// MOD-116 M3 D13: every request of `workspace_setup::REQUEST_NAMES` too: a refused create is
    /// said under its form, or in the box once the form is closed; D10: a refused count or delete
    /// is said in the delete's box.
    fn absorbs_failure(&self, request: &str) -> bool {
        [
            WORKSPACE_OVERVIEW,
            WORKSPACE_RUN_COUNTS,
            STOP_WORKSPACES,
            RESUME_WORKSPACE,
            DELETE_REACH,
            DELETE_WORKSPACE,
        ]
        .contains(&request)
            || workspace_setup::REQUEST_NAMES.contains(&request)
    }

    /// A bracketed paste: into the focused field of an open form (MOD-116 M3), else dropped.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        if self.question.is_some() {
            return Handled::Pass;
        }
        match &mut self.mode {
            Mode::Workspace(form) | Mode::Editing(form) => form.on_paste(text),
            Mode::Project(form) => form.on_paste(text),
            Mode::Browse | Mode::Choosing(_) | Mode::Deleting(_) | Mode::Targeting(..) => {
                return Handled::Pass;
            }
        }
        Handled::Consumed
    }

    /// D8's hint row; none while a question, the chooser or a form is up (its box or its pane
    /// carries its keys).
    fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        self.browse_hints(ctx)
    }

    /// MOD-116 M3: a form takes the detail pane's place, or the list's when the layout hides the
    /// detail; the chooser is a box over the tab, under any question.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let areas = ctx.split(area, WORKSPACES_LAYOUT);
        let form = self.mode.has_form();
        let detail = !areas.detail.is_empty();
        // R1b B1: a form holds the keys, so its pane is drawn as the focused one and carries
        // its keys on its own border (the list beside it is not focused); browsing, the keys go
        // on whichever of the tab's panes is focused.
        let activity = areas.activity.is_some_and(|rect| !rect.is_empty());
        let (list_focus, detail_focus, hints) = if form {
            let holder = pane::holder(ctx, activity);
            if detail {
                (Focus::Unfocused, holder, self.form_hints(ctx))
            } else {
                (holder, Focus::Unfocused, self.form_hints(ctx))
            }
        } else {
            (
                ctx.pane_focus(PaneId::List),
                ctx.pane_focus(PaneId::Detail),
                self.browse_hints(ctx),
            )
        };
        if let Some(list) = areas.list.filter(|rect| !rect.is_empty()) {
            if form && !detail {
                self.render_form(frame, list, list_focus, hints.as_ref(), ctx);
            } else {
                self.render_list(frame, list, list_focus, hints.as_ref(), ctx);
            }
        }
        if detail {
            if form {
                self.render_form(frame, areas.detail, detail_focus, hints.as_ref(), ctx);
            } else {
                self.render_detail(frame, areas.detail, detail_focus, hints.as_ref(), ctx);
            }
        }
        match &self.mode {
            Mode::Choosing(chooser) => {
                let hint = ctx.keys().hint(views::WORKSPACES_PICK, CHOOSER_HINT);
                chooser.render(frame, area, &hint, ctx);
            }
            Mode::Targeting(chooser, ..) => {
                let hint = ctx.keys().hint(views::WORKSPACES_PICK, CHOOSER_HINT);
                chooser.render(frame, area, &hint, ctx);
            }
            Mode::Deleting(deleting) => deleting.render(frame, area, ctx),
            Mode::Browse | Mode::Workspace(_) | Mode::Project(_) | Mode::Editing(_) => {}
        }
        self.render_question(frame, area, ctx);
    }

    /// [`views::WORKSPACES_CONFIRM`] while a question is up (D6); MOD-116 M3:
    /// [`views::WORKSPACES_PICK`] while the chooser or the target picker is,
    /// [`views::WORKSPACES_FORM`] while a form is, the delete's own stage's while one is up;
    /// else the browse stack.
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(match (&self.question, &self.mode) {
            (Some(_), _) => views::WORKSPACES_CONFIRM,
            (None, Mode::Browse) => views::WORKSPACES_BROWSE,
            (None, Mode::Choosing(_) | Mode::Targeting(..)) => views::WORKSPACES_PICK,
            (None, Mode::Workspace(_) | Mode::Project(_) | Mode::Editing(_)) => {
                views::WORKSPACES_FORM
            }
            (None, Mode::Deleting(deleting)) => deleting.stack(),
        })
    }

    /// The about panel labels the browse keys whatever is up.
    fn browse_stack(&self) -> Stack<'static> {
        views::WORKSPACES_BROWSE
    }

    /// D7: once a second while in front, a moved counts row reads the overview again, and a
    /// refused one is asked again (R1 F3). MOD-116 M3 D7: a busy form counts one second more.
    fn on_refresh(&mut self, ctx: &mut Ctx<'_>) {
        match &mut self.mode {
            Mode::Workspace(form) | Mode::Editing(form) => form.tick(),
            Mode::Project(form) => form.tick(),
            Mode::Browse | Mode::Choosing(_) | Mode::Deleting(_) | Mode::Targeting(..) => {}
        }
        self.read_if_due(ctx);
    }

    /// D7: every activation reads the overview again (R1 F2): item counts, goals, the
    /// description and repos edited elsewhere move no run count.
    fn on_activate(&mut self, ctx: &mut Ctx<'_>) {
        self.stale = true;
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
        self.user_placed = true;
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
