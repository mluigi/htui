//! The hierarchy section of the Settings tab: the workspace, its projects, their repos and this
//! box's paths, editable in place (MOD-15 milestone 3, D11/D12/D13; `R-ENT-1..4`, `R-BOX-4`). The
//! workspace row heads the tree read only since MOD-116 M3 D17: `e`, `b` and `d` on it name the
//! Workspaces tab's keys, where workspaces are created, edited and deleted.
//!
//! It holds **no store handle, no `UserId` and no `BoxId`** (`R-NF-3`): it names one read
//! ([`StoreRequest::Hierarchy`]), is handed the tree that comes back, and every write leaves
//! through `ctx.request` for [`crate::hierarchy::serve`] to fill the identity columns in. What is
//! on screen is always the last snapshot the worker assembled — no row is ever patched in locally,
//! so there is exactly one source of truth.
//!
//! Six modes — browsing, an editor, a delete confirmation, a directory picker (MOD-49), a
//! project's memory pane (MOD-163 D20, [`MemoryPane`], delegated as the picker is), or a
//! workspace's or project's owner documents pane (MOD-147 M3 D9, [`OwnerDocumentsPane`],
//! delegated the same way) — and
//! the mode is what [`captures_input`](SettingsSection::captures_input) is derived from rather than
//! a flag of its own (D2): in `Browse` the tab still cycles on `h`/`l` and the global table still
//! owns `q`, `?`, `Tab` and the digits; while an editor, a delete confirmation or the picker is
//! open those letters are text (or the picker's keys (`[picker]`)) and are swallowed, with one
//! carve-out — a chord the mode's modal global layer admits (`CONTROL`, `ALT`, a function key)
//! passes, so `ctrl-c` quits from inside a half-typed slug and `F1` opens help.
//!
//! Every mode resolves its keys through its own stack in [`views`] (MOD-67 M3), so a chord
//! matches with its modifiers (`ctrl-d` is not `d`) and a rebound key acts under its new chord;
//! the hint row is rendered from the same stack. Acts resolve from [`Ctx::pressed`], the chord the
//! user pressed (MOD-115 M3 MD-A).
//!
//! It draws through the kit (MOD-115 M3): the tree is a `kit::list` (gutter, band), an editor is a
//! `kit::form`, the notice a `kit::text` notice line and the empty states `kit::empty` with a next
//! step; the picker is a `kit::modal` ([`PathPicker`]).

use std::cell::Cell;
use std::path::Path;

use chrono::{DateTime, Utc};
use htui_core::model::{IntegrateMode, ProjectId, ProjectPatch, RepoId, RepoPatch, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use htui_core::store::{DeleteReach, DeleteTarget};

use crate::app::{Action, Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::hierarchy::{
    HierarchySnapshot, InferOutcome, InferReport, MirrorAfterDelete, ProjectEntry, REQUEST_NAMES,
    RepoEntry, reach_parts, reach_totals, workspace_root,
};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::memory::REQUEST_NAMES as MEMORY_REQUEST_NAMES;
use crate::owner_documents::{DocumentOwner, REQUEST_NAMES as DOCUMENT_REQUEST_NAMES};
use crate::store_worker::{LIST_DIR, StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form, Value};
use crate::ui::kit::label::plural;
use crate::ui::kit::list::{self, RowKind};
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::path_picker::{PathPicker, PickerOutcome, start_dir};
use crate::ui::tabs::settings::memory::{MemoryPane, PaneKey};
use crate::ui::tabs::settings::owner_documents::OwnerDocumentsPane;
use crate::ui::tabs::settings::{SectionId, SettingsSection, changed_elsewhere};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

/// What the rows pane says when the scope is a workspace that does not exist — the nil startup
/// scope, or the last workspace after it was deleted (D9's "stranded shell" row). The empty
/// state's first line; [`creates_one`] is its next step (MOD-115 M3).
const NO_WORKSPACE: &str = "no workspace";

/// The tab where workspaces are created, edited and deleted (MOD-116 M3 D17): this section keeps
/// the workspace row as the tree's head, read only, and its notices send the person there.
const WORKSPACES_TAB: &str = "the Workspaces tab";

/// The next step of [`NO_WORKSPACE`]. `new` is the label of `workspaces.new` through
/// [`views::WORKSPACES_BROWSE`] (MOD-116 M3 D17; MOD-67 M6 D4): `` `N` on the Workspaces tab
/// creates one`` under the defaults; unbound, no key is named and the step is the one left, a
/// workspace made elsewhere.
fn creates_one(new: Option<&str>) -> String {
    match new {
        Some(new) => format!("`{new}` on {WORKSPACES_TAB} creates one"),
        None => "a workspace created elsewhere shows up here".to_owned(),
    }
}

/// The refusal a key gets with no workspace: `` no workspace — `N` on the Workspaces tab creates
/// one`` under the defaults, and no key named when it is unbound.
fn no_workspace(new: Option<&str>) -> String {
    match new {
        Some(_) => format!("{NO_WORKSPACE} \u{2014} {}", creates_one(new)),
        None => NO_WORKSPACE.to_owned(),
    }
}

/// What the rows pane says when the read itself was refused: the tree is one `ReadStore` call and
/// an offline box has no mirror of it.
const UNAVAILABLE: &str = "hierarchy needs Postgres";

/// The next step of [`UNAVAILABLE`] (MOD-83's example): where a DSN is stored, then `reload`, the
/// label of `common.reload` through the browse stack, asking again; unbound, the first half alone.
fn unavailable_next(reload: Option<&str>) -> String {
    const CONNECTION: &str = "Settings \u{203a} Connection stores a DSN";
    match reload {
        Some(reload) => format!("{CONNECTION} \u{b7} {reload} asks again once the store answers"),
        None => CONNECTION.to_owned(),
    }
}

/// Browse's keys.
const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::New, "project/repo"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::HierarchyPrimary, "primary"),
    Hint::One(Act::HierarchyChoosePath, "path"),
    Hint::One(Act::HierarchyInfer, "infer"),
    // MOD-147 M3 D9: before `memory`, so it shows in the cut-off hint line.
    Hint::One(Act::HierarchyDocuments, "documents"),
    Hint::One(Act::HierarchyMemory, "memory"),
    Hint::One(Act::Delete, "delete"),
    Hint::One(Act::Reload, "reload"),
];

/// Browse's keys with nothing read: only the one that does not need a tree. A workspace is
/// created on the Workspaces tab (MOD-116 M3 D17), whose key the empty state names.
const HINT_NO_WORKSPACE: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// Browse's keys with the read refused: nothing here can be created against a store that did not
/// answer, so the only offer is to ask again.
const HINT_UNAVAILABLE: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// An open editor's keys: the form's own, then the field's `field.submit` and `field.cancel`
/// through the editor's stack (D13; MOD-115 M3 C6), `Enter save · Esc cancel` by default.
const HINT_EDITING: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The repo editor's keys on its `integrate` row (MOD-133 M2 E8): [`HINT_EDITING`] with the
/// choice keys first.
const HINT_EDITING_CHOICE: HintSpec = &[
    Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "mode"),
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The repo editor's `integrate` row's label (MOD-133 M2 E8).
const INTEGRATE_LABEL: &str = "integrate";

/// The `integrate` row's help: what it is, and what `default` means.
const INTEGRATE_HELP: &str =
    "how a finished run reaches the remote; default is the project's `integrate` setting";

/// The index of the repo editor's `remote url` field, which `push` needs non-empty.
const REMOTE_URL_FIELD: usize = 1;

/// The picker's section hint (MOD-49 blueprint D17): the popup carries the full key list; the
/// way out goes through the mode's stack (MOD-67 M6), so it drops while the go-to field is open,
/// where `Esc` closes only the field.
const HINT_PICKING: HintSpec = &[
    Hint::Text("choosing a directory"),
    Hint::One(Act::Back, "cancel"),
];

/// The workspace row's words for the folder (MOD-147 M3 D12), read only here since MOD-116 M3
/// D17: the Workspaces tab's edit form sets it.
const FOLDER_ON_THIS_BOX: &str = "folder on this box";

/// What a folder with no `workspace_box_path` row for this box adds after it (D12).
const DEFAULT_FOLDER: &str = " (default)";

/// MOD-147 M3 D12: the tree's workspace folder on this box as the section writes it — the stored
/// root, else `<home>/workspaces/<slug>` with ` (default)`, a path under `home` written `~/…`;
/// `unset` when there is neither (no row, and no home or a slug that is not one plain name).
/// Computed here rather than carried by the tree: [`snapshot`](crate::hierarchy::snapshot) has
/// no home, and the section's is the worker's `dirs::home_dir()` anyway.
fn folder_on_this_box(snapshot: &HierarchySnapshot, home: Option<&str>) -> String {
    let home = home.map(Path::new).filter(|home| home.parent().is_some());
    let Some(folder) = workspace_root(snapshot, home) else {
        return "unset".to_owned();
    };
    let shown = match home.and_then(|home| folder.strip_prefix(home).ok()) {
        Some(rest) => under_home(rest),
        None => folder.display().to_string(),
    };
    if snapshot.folder_path.is_some() {
        shown
    } else {
        format!("{shown}{DEFAULT_FOLDER}")
    }
}

/// `rest`, a path under the home, written `~/…` with its components joined by `/` rather than
/// the native separator (review M1); `~` alone for the home itself.
fn under_home(rest: &Path) -> String {
    let parts: Vec<_> = rest
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect();
    if parts.is_empty() {
        "~".to_owned()
    } else {
        format!("~/{}", parts.join("/"))
    }
}

/// What a CAS miss says when the row the editor opened on is gone from the reloaded tree.
const DELETED_ELSEWHERE: &str = "deleted elsewhere while you were editing";

/// `settings.hierarchy.memory` on a row that is not a project's (MOD-163 D20).
pub const PICK_A_PROJECT: &str = "memory belongs to a project: put the cursor on a project row";

/// The memory pane closed because its project left the tree.
const MEMORY_PROJECT_GONE: &str = "the project is gone; its memory pane closed";

/// `settings.hierarchy.documents` on a row that is neither the workspace's nor a project's
/// (MOD-147 M3 D9).
pub const PICK_AN_OWNER: &str =
    "documents belong to the workspace or a project: put the cursor on one of their rows";

/// The documents pane closed because its owner left the tree.
const DOCUMENTS_OWNER_GONE: &str = "the owner is gone; its documents pane closed";

/// What a CAS miss says with no editor open — `p` is the one write that has none (D12). `primary`
/// is the label of `settings.hierarchy.primary` (MOD-67 M6 D4): `reloaded; press p again` under
/// the defaults.
fn reloaded(primary: Option<&str>) -> String {
    match primary {
        Some(primary) => format!("reloaded; press {primary} again"),
        None => "reloaded".to_owned(),
    }
}

/// The hint line while `delete_reach` is being counted: every chord that stops it (MOD-67 M3,
/// blueprint L-C Q7: the arm always stopped on `n` too).
const HINT_COUNTING: HintSpec = &[
    Hint::Text("counting rows\u{2026}"),
    Hint::All(Act::ConfirmNo, "stop"),
];

/// The hint line of the first confirmation.
const HINT_WARN: HintSpec = &[
    Hint::One(Act::ConfirmYes, "continue"),
    Hint::All(Act::ConfirmNo, "stop"),
];

/// The hint line of the second, typed confirmation: the field's own keys through
/// [`views::CAPTURE`] (D13; MOD-115 M3 C6), `Enter confirm · Esc stop` by default.
const HINT_TYPED: HintSpec = &[
    Hint::One(Act::FieldSubmit, "confirm"),
    Hint::One(Act::FieldCancel, "stop"),
];

/// The hint line while the delete is in flight.
const HINT_DELETING: HintSpec = &[Hint::Text("deleting\u{2026}")];

/// The second line of every warning: the one sentence PRD D13 asks to be in front of a user before
/// anything is removed. It names no key: the hint row under it does, from the live keys, in every
/// stage (MOD-67 M3 R1), so a rebound `confirm.yes` cannot leave this line saying `y`.
const NOT_UNDONE: &str = "Nothing here can be undone.";

/// What a second confirmation that did not match says. The stage stays where it was.
const WRONG_SLUG: &str = "that is not the slug; nothing was deleted";

/// How many repo names one list of an inference notice shows before `+N more` (blueprint D137).
const NAMES_SHOWN: usize = 3;

/// What follows the primary repo's name on its row (MOD-83: a word, where a bare `*` stood).
const PRIMARY: &str = " \u{b7} primary";

/// One row of the flat list the section draws and the cursor indexes.
///
/// Positions rather than ids: the list is rebuilt from the snapshot on every reply, and an index
/// that outlives its tree is caught by the clamp, where a stale id would silently select nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The workspace line.
    Workspace,
    /// One project of the workspace.
    Project {
        /// Index into `snapshot.projects`.
        index: usize,
    },
    /// One repo of a project.
    Repo {
        /// Index into `snapshot.projects`.
        project: usize,
        /// Index into that project's `repos`.
        index: usize,
    },
}

/// Which row an open editor writes back to, and what it needs to address it.
///
/// `EditRepo` carries the repo's **project** because the seam has no `repo(id)` reader: the worker
/// re-reads the tree the reply renders, and it needs the project to find the workspace (plan open
/// item O-2). [`PathTarget::RepoPath`] carries it for the same reason (MOD-49).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorKind {
    /// `n` on the workspace row.
    NewProject,
    /// `e` on a project row.
    EditProject(ProjectId),
    /// `n` on a project or repo row.
    NewRepo(ProjectId),
    /// `e` on a repo row. Never carries `is_primary`: `p` is that column's only writer (D12).
    EditRepo {
        /// The repo's project.
        project: ProjectId,
        /// The repo.
        id: RepoId,
    },
}

/// Which row `b` writes a path for (MOD-49, blueprint D7), out of [`EditorKind`] now that no
/// editor types it. A repo's checkout alone since MOD-116 M3 D17: the workspace's folder is set on
/// the Workspaces tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathTarget {
    /// `b` on a repo row: this box's checkout.
    RepoPath {
        /// The repo's project.
        project: ProjectId,
        /// The repo.
        id: RepoId,
    },
}

impl PathTarget {
    /// The unchanged write (MOD-49 plan summary): the worker's `canonical` guard decides what is
    /// stored, so the chosen path goes out as navigated.
    fn request(self, path: String) -> StoreRequest {
        match self {
            Self::RepoPath { project, id } => StoreRequest::SetRepoPath {
                project,
                repo: id,
                path,
            },
        }
    }
}

/// One labelled input of an editor.
#[derive(Debug)]
struct Field {
    /// What the pane prints in front of it.
    label: &'static str,
    /// The buffer; its `Debug` never prints the text.
    input: TextField,
    /// Whether `Enter` refuses while it is empty.
    required: bool,
}

/// The open editor: which row, its fields, which one has focus, and the CAS token it opened on.
#[derive(Debug)]
struct Editor {
    /// The row this writes back to.
    kind: EditorKind,
    /// The inputs, in tab order.
    fields: Vec<Field>,
    /// Index into `fields`, or `fields.len()` for the `integrate` row.
    focus: usize,
    /// The `updated_at` of the row this opened on, for the three `Edit*` kinds (M1 D3).
    expected: Option<DateTime<Utc>>,
    /// MOD-133 M2 E8: the repo's mode, `Some` only for [`EditorKind::EditRepo`] (a new repo is
    /// created with the project's default). The inner `None` is that default.
    integrate: Option<Option<IntegrateMode>>,
    /// The project's default as [`IntegrateMode::project_default`] reads it, `None` when it reads
    /// no mode (runs then refuse at start): what the row's `default` stands for.
    inherited: Option<IntegrateMode>,
    /// The word the row shows for the default, `default (local)`.
    default_word: String,
}

/// What one key on an editor's non-field row comes to, decided while the editor is borrowed and
/// acted on after (MOD-133 M2 E8; `memory.rs`'s `FormStep`).
enum EditorStep {
    /// Handled, or not, in place.
    Done(Handled),
    /// The field's submit, on the `integrate` row.
    Submit,
    /// The field's cancel, on the `integrate` row.
    Cancel,
}

/// What the section is doing. `Browse` is not a mode in the modal sense: it captures nothing.
#[derive(Debug, Default)]
enum Mode {
    /// The rows, the cursor and the tab's own `h`/`l`.
    #[default]
    Browse,
    /// One row being typed into.
    Editing(Editor),
    /// One project being deleted, behind PRD D13's two confirmations. A workspace is deleted on
    /// the Workspaces tab (MOD-116 M3 D17), so a project is all this mode can name (R1b B10): no
    /// workspace delete's reply, which goes to that tab, is ever waited on here.
    Deleting {
        /// The project `DeleteReach` and `DeleteProject` name.
        target: ProjectId,
        /// The slug the second confirmation asks to be typed.
        slug: String,
        /// How far the confirmation has got.
        stage: DeleteStage,
    },
    /// `b`: choosing a directory (MOD-49). Stays open while the write is in flight (blueprint D8).
    Picking {
        /// The row the choice is written to.
        target: PathTarget,
        /// The popup.
        picker: PathPicker,
        /// The path sent, until the reply: what `written` compares the stored path with.
        chosen: Option<String>,
    },
    /// MOD-163 D20: a project's memory pane, delegated as `Picking` is.
    Memory {
        /// The project the pane lists.
        project: ProjectId,
        /// The pane. Boxed: it is the largest thing a mode holds.
        pane: Box<MemoryPane>,
    },
    /// MOD-147 M3 D9: a workspace's or a project's owner documents pane, delegated as `Memory`
    /// is.
    Documents {
        /// Whose documents the pane lists.
        owner: DocumentOwner,
        /// The pane. Boxed, as the memory pane is.
        pane: Box<OwnerDocumentsPane>,
    },
}

/// The stages of a delete, each carrying the counts the pane shows (flag J).
///
/// The reach travels with the stage rather than beside it because both panes print it: a warning
/// that said "5 kinds" and a prompt that had lost them would be two different claims about one act.
#[derive(Debug)]
enum DeleteStage {
    /// `delete_reach` is in flight.
    Counting,
    /// The counts are on screen and `y` is the first confirmation.
    Warn(Box<DeleteReach>),
    /// The slug is being typed — the second confirmation, and the one that deletes.
    Typed {
        /// What the warning listed.
        reach: Box<DeleteReach>,
        /// What has been typed so far.
        field: TextField,
    },
    /// The delete is in flight.
    InFlight(Box<DeleteReach>),
}

/// The workspace tree of the scope, with the keys that edit it.
#[derive(Debug, Default)]
pub struct HierarchySection {
    /// The last tree the worker assembled, or `None` for a scope with no workspace.
    snapshot: Option<HierarchySnapshot>,
    /// `Some(message)` after `Failed { request: "hierarchy" }`: the read itself was refused.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](HierarchySection::rows).
    cursor: usize,
    /// Browsing, or typing.
    mode: Mode,
    /// The write in flight, by [`StoreRequest::name`]. A second one is refused until the reply, as
    /// the agent section's `probing` refuses a second probe: the staleness index keeps only the
    /// newest request of a kind, so two writes of one kind racing would lose a reply.
    busy: Option<&'static str>,
    /// The last outcome, on the notice row under the pane.
    notice: Option<String>,
    /// What an inference in flight puts in front of its report: the `stored as …` notice of the
    /// write it follows, or `None` for `i`. Held apart from `notice`, which a key refused during
    /// the walk overwrites with "still in flight" (R-57).
    carried: Option<String>,
    /// The home directory, read once by [`new`](HierarchySection::new) (MOD-49 blueprint D9):
    /// the picker's last start directory before `/` (P7), and what the folder line writes `~`.
    home: Option<String>,
    /// The tree's `(lines, first line drawn)` as of the last render, for the Settings pane's
    /// scrollbar (MOD-115 M3 D-11), in the pane's terms: the rows under the tree (an open editor
    /// or warning, the notice line) count as lines, since the pane measures the bar against its
    /// whole inner rect. `None` while no tree is drawn.
    scroll: Cell<Option<ScrollSpan>>,
}

impl HierarchySection {
    /// Identity of the hierarchy section.
    pub const ID: SectionId = SectionId("hierarchy");

    /// A section with nothing read yet. Reads the home directory once, here, so opening the picker
    /// touches no environment (MOD-49 blueprint D9): `dirs::home_dir()`, the one the worker puts
    /// the default workspace folder under (MOD-147 M3 D12), which is not `$HOME` on Windows.
    #[must_use]
    pub fn new() -> Self {
        Self {
            home: dirs::home_dir().and_then(|home| home.to_str().map(str::to_owned)),
            ..Self::default()
        }
    }

    /// The same section with the home fixed, so a test's start directory doesn't depend on the box
    /// running it (MOD-49 blueprint D9).
    #[must_use]
    pub fn with_home(mut self, home: Option<&str>) -> Self {
        self.home = home.map(str::to_owned);
        self
    }

    /// The flat list the cursor indexes: the workspace, then each project with its repos under it.
    ///
    /// Derived on demand rather than cached beside the snapshot, so the two cannot disagree about
    /// what is on screen.
    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut rows = vec![Row::Workspace];
        for (index, entry) in snapshot.projects.iter().enumerate() {
            rows.push(Row::Project { index });
            for repo in 0..entry.repos.len() {
                rows.push(Row::Repo {
                    project: index,
                    index: repo,
                });
            }
        }
        rows
    }

    /// The row under the cursor, or `None` while there is no tree.
    fn selected(&self) -> Option<Row> {
        self.rows().get(self.cursor).copied()
    }

    /// Moves the cursor one row and stops at the end it reaches.
    ///
    /// No wrap, for the reason the agent table gives: `d` acts on the row the cursor is on, and a
    /// held `j` that wrapped to the top would aim a delete at a row the user never looked at.
    fn move_cursor(&mut self, down: bool) {
        let Some(last) = self.rows().len().checked_sub(1) else {
            self.cursor = 0;
            return;
        };
        self.cursor = if down {
            self.cursor.saturating_add(1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
    }

    /// Puts the cursor back inside the list after a reply replaced the tree.
    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// One list row per tree row, in [`rows`](HierarchySection::rows) order (D13): the workspace
    /// and its projects head their rows as the kit's group rows, the repos are items (MOD-115 M3).
    /// The primary repo says so in a word after its name (MOD-83), never a bare `*`.
    fn list_rows(&self, theme: &Theme) -> Vec<list::Row<'static>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let group = |text: String| list::Row {
            line: Line::styled(text, theme.title),
            kind: RowKind::Group { folded: false },
        };
        self.rows()
            .into_iter()
            .map(|row| match row {
                Row::Workspace => group(format!(
                    "{} ({}) \u{2014} {FOLDER_ON_THIS_BOX}: {}",
                    snapshot.workspace.name,
                    snapshot.workspace.slug,
                    folder_on_this_box(snapshot, self.home.as_deref()),
                )),
                Row::Project { index } => group(
                    snapshot
                        .projects
                        .get(index)
                        .map_or_else(String::new, |entry| {
                            format!("  {}  {}", entry.project.slug, entry.project.name)
                        }),
                ),
                Row::Repo { project, index } => list::Row {
                    line: Line::styled(
                        snapshot
                            .projects
                            .get(project)
                            .and_then(|entry| entry.repos.get(index))
                            .map_or_else(String::new, |entry| {
                                format!(
                                    "    {}{}  {}  {}  {}",
                                    entry.repo.name,
                                    if entry.repo.is_primary { PRIMARY } else { "" },
                                    entry.repo.default_branch,
                                    entry.repo.remote_url.as_deref().unwrap_or("\u{2014}"),
                                    entry
                                        .local_path
                                        .as_ref()
                                        .map_or("unset", |path| path.local_path.as_str())
                                )
                            }),
                        theme.base,
                    ),
                    kind: RowKind::Item,
                },
            })
            .collect()
    }

    /// The pane under the rows: the open editor as a kit form, the delete's warning, or nothing at
    /// all in Browse.
    ///
    /// Takes the width because the delete warning is a sentence rather than a row and has to wrap
    /// inside the pane it is measured for (the layout below needs the height first).
    fn pane(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        match &self.mode {
            Mode::Browse => Vec::new(),
            Mode::Editing(editor) => editor.form().lines(width, ctx.accent(), theme),
            // MOD-49 P2: the popup draws over the section in `render`, so the pane stays empty;
            // the memory pane draws over the whole section too (MOD-163 D20).
            Mode::Picking { .. } | Mode::Memory { .. } | Mode::Documents { .. } => Vec::new(),
            Mode::Deleting { slug, stage, .. } => delete_pane(slug, stage, width, theme),
        }
    }

    /// The one line under the pane: the last outcome as a kit notice, an error when it is one to
    /// act on rather than read (a compare-and-set miss). The keys this mode binds are on the
    /// Settings pane's bottom border (MOD-67 M6 T2, [`SettingsSection::key_hint`]), so the outcome
    /// has the line to itself; blank without one.
    /// Any mode: an editor's outcome is drawn here too.
    fn notice_line(&self, theme: &Theme) -> Line<'static> {
        let Some(text) = &self.notice else {
            return Line::default();
        };
        let tone = if super::is_error(text) {
            Tone::Error
        } else {
            Tone::Info
        };
        notice(text, tone, theme)
    }

    /// The mode's keys for the shell's hint row, plus what a write in flight adds to them.
    /// Rendered through the mode's own stack, so a rebound key shows its new chord.
    fn hint_text(&self, keys: &Keys) -> String {
        if let Mode::Memory { pane, .. } = &self.mode {
            return pane.hint_text(keys);
        }
        if let Mode::Documents { pane, .. } = &self.mode {
            return pane.hint_text(keys);
        }
        let spec = match &self.mode {
            Mode::Browse => {
                if self.unavailable.is_some() {
                    HINT_UNAVAILABLE
                } else if self.snapshot.is_none() {
                    HINT_NO_WORKSPACE
                } else {
                    HINT_BROWSE
                }
            }
            Mode::Editing(editor) if editor.on_integrate() => HINT_EDITING_CHOICE,
            Mode::Editing(_) => HINT_EDITING,
            Mode::Picking { .. } => HINT_PICKING,
            // Answered above; the pane names its own keys.
            Mode::Memory { .. } | Mode::Documents { .. } => HINT_BROWSE,
            Mode::Deleting { stage, .. } => match stage {
                DeleteStage::Counting => HINT_COUNTING,
                DeleteStage::Warn(_) => HINT_WARN,
                DeleteStage::Typed { .. } => HINT_TYPED,
                DeleteStage::InFlight(_) => HINT_DELETING,
            },
        };
        let keys = keys.hint(self.stack(), spec);
        match self.busy {
            // Only in Browse: an editor's own hint says what `Enter` is for, and a write in flight
            // is why `Enter` is not answering.
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {}\u{2026}", doing(busy))
            }
            _ => keys,
        }
    }

    /// The stack of the current mode (MOD-67 M3), as `key_stack`, the delete and the picker read
    /// it; the browse and editor handlers name their own mode's constant, which is this one
    /// whenever they run. The typed confirmation and the delete in flight are a field or a
    /// widget and nothing else ([`views::CAPTURE`]); the picker is on
    /// [`views::HIERARCHY_PICKER`], its go-to field on [`views::CAPTURE`] (MOD-67 M6).
    fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::HIERARCHY_BROWSE,
            Mode::Editing(_) => views::HIERARCHY_EDITOR,
            Mode::Picking { picker, .. } if picker.goto_open() => views::CAPTURE,
            Mode::Picking { .. } => views::HIERARCHY_PICKER,
            Mode::Memory { pane, .. } => pane.stack(),
            Mode::Documents { pane, .. } => pane.stack(),
            Mode::Deleting { stage, .. } => match stage {
                DeleteStage::Counting => views::HIERARCHY_DELETE_COUNTING,
                DeleteStage::Warn(_) => views::HIERARCHY_DELETE_WARN,
                DeleteStage::Typed { .. } | DeleteStage::InFlight(_) => views::CAPTURE,
            },
        }
    }

    /// One key in Browse, through `views::HIERARCHY_BROWSE` (MOD-67 M3), resolved from the chord the
    /// user pressed ([`Ctx::pressed`], MOD-115 M3 MD-A). A chord matches with its
    /// modifiers, so `ctrl-d` is not `d` (ANA-26 §2.6 defect 1). The tab took `settings.*` before
    /// offering the key; a global act, or one this state declines, is the shell's (`Pass`).
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::HIERARCHY_BROWSE, chord) {
            match act {
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                Act::New => {
                    if !self.refuse(ctx.keys())
                        && let Some(row) = self.selected()
                    {
                        self.open_new_child(row);
                    }
                }
                Act::Edit => {
                    if !self.refuse(ctx.keys())
                        && let Some(row) = self.selected()
                    {
                        self.open_edit(row, ctx.keys());
                    }
                }
                Act::HierarchyPrimary => {
                    if !self.refuse(ctx.keys())
                        && let Some(row) = self.selected()
                    {
                        self.move_primary(row, ctx);
                    }
                }
                Act::HierarchyChoosePath => {
                    if !self.refuse(ctx.keys())
                        && let Some(row) = self.selected()
                    {
                        self.open_path(row, ctx);
                    }
                }
                // MOD-7 milestone 4 (D114): infer this box's repo paths under the workspace root.
                // A write, so it is refused while another is in flight, as the other writes are.
                Act::HierarchyInfer => {
                    if !self.refuse(ctx.keys())
                        && let Some(snapshot) = &self.snapshot
                    {
                        let request = StoreRequest::InferRepoPaths(snapshot.workspace.id);
                        self.notice = None;
                        self.carried = None;
                        self.send(request, ctx);
                    }
                }
                Act::Delete => {
                    if !self.refuse(ctx.keys())
                        && let Some(row) = self.selected()
                    {
                        self.begin_delete(row, ctx);
                    }
                }
                // MOD-163 D20: a project row opens its memory pane. A read, so not `busy`; refused
                // like the other keys while a write is in flight or the tree is not there.
                Act::HierarchyMemory => {
                    if !self.refuse(ctx.keys()) {
                        match self.selected() {
                            Some(Row::Project { index }) => self.open_memory(index, ctx),
                            _ => self.notice = Some(PICK_A_PROJECT.to_owned()),
                        }
                    }
                }
                // MOD-147 M3 D9: the workspace row or a project row opens its owner documents; a
                // read, so refused only as the memory pane's key is.
                Act::HierarchyDocuments => {
                    if !self.refuse(ctx.keys()) {
                        match self.selected() {
                            Some(row @ (Row::Workspace | Row::Project { .. })) => {
                                self.open_documents(row, ctx);
                            }
                            _ => self.notice = Some(PICK_AN_OWNER.to_owned()),
                        }
                    }
                }
                // Allowed while `busy`: re-reading is how a section that lost a reply recovers,
                // and a read cannot lose a write's reply — the staleness index is keyed by request
                // kind.
                Act::Reload => ctx.request(StoreRequest::Hierarchy(ctx.scope.workspace_id)),
                // Only when there is something to clear: a section that swallowed every `Esc`
                // would take the one the shell uses to close an overlay over it.
                Act::Dismiss if self.notice.is_some() => self.notice = None,
                // A global act, or one this state declines.
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// Whether a key that opens an editor is refused right now, with the notice that says why.
    ///
    /// Two refusals, in this order: a write in flight (a second one of the same kind would lose a
    /// reply), then a tree that is not there to edit. `r` is deliberately **not** on this path —
    /// re-reading is how a section that lost a reply recovers. `keys` name the key that creates a
    /// workspace in the no-workspace refusal (MOD-67 M6 D4): the Workspaces tab's `new` since
    /// MOD-116 M3 D17, so with no tree every key here is refused.
    fn refuse(&mut self, keys: &Keys) -> bool {
        if self.in_flight() {
            return true;
        }
        // The read was refused, so there is nothing to edit and nothing to create against either,
        // and a notice offering the Workspaces tab's `N` would name a key that does not work.
        if let Some(why) = &self.unavailable {
            self.notice = Some(format!("{UNAVAILABLE}: {why}"));
            return true;
        }
        if self.snapshot.is_none() {
            let new = keys.label(views::WORKSPACES_BROWSE, Act::WorkspacesNew);
            self.notice = Some(no_workspace(new.as_deref()));
            return true;
        }
        false
    }

    /// Whether a write is already in flight, with the notice that says which one.
    ///
    /// The one refusal both ends of the section owe: a second write of a kind would have the
    /// staleness index drop the first one's reply, and the first is the one about the write that
    /// actually landed.
    fn in_flight(&mut self) -> bool {
        let Some(busy) = self.busy else {
            return false;
        };
        self.notice = Some(in_flight(busy));
        true
    }

    /// `settings.hierarchy.memory` on project `index`: the pane opens on it and its read goes out
    /// (MOD-163 D20).
    fn open_memory(&mut self, index: usize, ctx: &Ctx<'_>) {
        let Some(entry) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.projects.get(index))
        else {
            return;
        };
        let project = entry.project.id;
        let (pane, read) = MemoryPane::open(project, entry.project.slug.clone());
        ctx.request(read);
        self.notice = None;
        self.mode = Mode::Memory {
            project,
            pane: Box::new(pane),
        };
    }

    /// `settings.hierarchy.documents` on the workspace row or a project row: the pane opens on
    /// its owner and its read goes out (MOD-147 M3 D9).
    fn open_documents(&mut self, row: Row, ctx: &Ctx<'_>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let (owner, slug) = match row {
            Row::Workspace => (
                DocumentOwner::Workspace(snapshot.workspace.id),
                snapshot.workspace.slug.clone(),
            ),
            Row::Project { index } => match snapshot.projects.get(index) {
                Some(entry) => (
                    DocumentOwner::Project(entry.project.id),
                    entry.project.slug.clone(),
                ),
                None => return,
            },
            Row::Repo { .. } => return,
        };
        let (pane, read) = OwnerDocumentsPane::open(owner, slug);
        ctx.request(read);
        self.notice = None;
        self.mode = Mode::Documents {
            owner,
            pane: Box::new(pane),
        };
    }

    /// Opens an editor, clearing whatever the last one said.
    fn open(&mut self, kind: EditorKind, fields: Vec<Field>, expected: Option<DateTime<Utc>>) {
        self.notice = None;
        self.mode = Mode::Editing(Editor {
            kind,
            fields,
            focus: 0,
            expected,
            integrate: None,
            inherited: None,
            default_word: String::new(),
        });
    }

    /// `n`: a project under the workspace row, a repo under a project or a repo row.
    fn open_new_child(&mut self, row: Row) {
        match row {
            Row::Workspace => self.open(
                EditorKind::NewProject,
                vec![
                    Field::required("slug", ""),
                    Field::required("name", ""),
                    Field::optional("description", ""),
                ],
                None,
            ),
            Row::Project { index } | Row::Repo { project: index, .. } => {
                let Some(entry) = self.snapshot.as_ref().and_then(|s| s.projects.get(index)) else {
                    return;
                };
                // A project's first repo is offered as the primary one, because a project with
                // repos and no primary is reachable by the schema and wanted by nothing (D12).
                let primary = if entry.repos.is_empty() { "y" } else { "n" };
                let project = entry.project.id;
                self.open(
                    EditorKind::NewRepo(project),
                    vec![
                        Field::required("name", ""),
                        Field::optional("remote url", ""),
                        Field::required("default branch", "main"),
                        Field::required("primary (y/n)", primary),
                    ],
                    None,
                );
            }
        }
    }

    /// `e`: the row under the cursor, prefilled, with its `updated_at` as the CAS token. The
    /// workspace row names the Workspaces tab's `edit` instead (MOD-116 M3 D17).
    fn open_edit(&mut self, row: Row, keys: &Keys) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match row {
            Row::Workspace => {
                let edit = keys.label(views::WORKSPACES_BROWSE, Act::Edit);
                self.notice = Some(match edit {
                    Some(edit) => format!("`{edit}` on {WORKSPACES_TAB} edits the workspace"),
                    None => format!("edit the workspace on {WORKSPACES_TAB}"),
                });
            }
            Row::Project { index } => {
                let Some(entry) = snapshot.projects.get(index) else {
                    return;
                };
                let project = &entry.project;
                let fields = vec![
                    Field::required("slug", &project.slug),
                    Field::required("name", &project.name),
                    Field::optional("description", &project.description),
                ];
                self.open(
                    EditorKind::EditProject(project.id),
                    fields,
                    Some(project.updated_at),
                );
            }
            Row::Repo { project, index } => {
                let Some(entry) = snapshot
                    .projects
                    .get(project)
                    .and_then(|owner| owner.repos.get(index).map(|repo| (owner, repo)))
                else {
                    return;
                };
                let (owner, repo) = entry;
                let fields = vec![
                    Field::required("name", &repo.repo.name),
                    Field::optional("remote url", repo.repo.remote_url.as_deref().unwrap_or("")),
                    Field::required("default branch", &repo.repo.default_branch),
                ];
                // MOD-133 M2 E8: the mode row, holding the stored column; `None` is the project's
                // default, named by what it reads as now.
                let integrate = repo.repo.integrate;
                let inherited = IntegrateMode::project_default(&owner.project.settings).ok();
                let default_word = inherited.map_or_else(
                    || "default (unreadable)".to_owned(),
                    |mode| format!("default ({mode})"),
                );
                self.open(
                    EditorKind::EditRepo {
                        project: owner.project.id,
                        id: repo.repo.id,
                    },
                    fields,
                    Some(repo.repo.updated_at),
                );
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.integrate = Some(integrate);
                    editor.inherited = inherited;
                    editor.default_word = default_word;
                }
            }
        }
    }

    /// `b`: the directory picker on this box's path for a repo (MOD-49 P1, P7). The listing goes
    /// out through `ctx.request`, not `send`: it is a read and marks nothing busy (P11). The
    /// workspace row names the Workspaces tab's `edit`, whose form sets the folder (MOD-116 M3
    /// D17).
    fn open_path(&mut self, row: Row, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let workspace_root = snapshot
            .folder_path
            .as_ref()
            .map(|path| path.folder_path.as_str());
        let (target, title, stored, fallback) = match row {
            Row::Workspace => {
                let edit = ctx.keys().label(views::WORKSPACES_BROWSE, Act::Edit);
                self.notice = Some(match edit {
                    Some(edit) => format!("`{edit}` on {WORKSPACES_TAB} sets its folder"),
                    None => format!("set the workspace's folder on {WORKSPACES_TAB}"),
                });
                return;
            }
            Row::Project { .. } => {
                let choose = ctx
                    .keys()
                    .label(views::HIERARCHY_BROWSE, Act::HierarchyChoosePath);
                self.notice = Some(match choose {
                    Some(choose) => format!("`{choose}` wants a repo row"),
                    None => "pick a path on a repo row".to_owned(),
                });
                return;
            }
            Row::Repo { project, index } => {
                let Some((owner, repo)) = snapshot
                    .projects
                    .get(project)
                    .and_then(|owner| repo_at(owner, index))
                else {
                    return;
                };
                (
                    PathTarget::RepoPath {
                        project: owner,
                        id: repo.repo.id,
                    },
                    format!("Checkout of `{}` on this box", repo.repo.name),
                    repo.local_path
                        .as_ref()
                        .map(|path| path.local_path.as_str()),
                    workspace_root,
                )
            }
        };
        let start = start_dir(stored, fallback, self.home.as_deref());
        let (picker, request) = PathPicker::open(title, start);
        self.notice = None;
        self.mode = Mode::Picking {
            target,
            picker,
            chosen: None,
        };
        ctx.request(request);
    }

    /// `p`: move the primary flag to the repo under the cursor (D12).
    ///
    /// The one write with no editor behind it, so the one that can answer `HierarchyStale` in
    /// Browse.
    fn move_primary(&mut self, row: Row, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match row {
            Row::Repo { project, index } => {
                let Some((owner, repo)) = snapshot
                    .projects
                    .get(project)
                    .and_then(|owner| repo_at(owner, index))
                else {
                    return;
                };
                let request = StoreRequest::UpdateRepo {
                    project: owner,
                    id: repo.repo.id,
                    expected: repo.repo.updated_at,
                    patch: RepoPatch {
                        is_primary: Some(true),
                        ..RepoPatch::default()
                    },
                };
                self.notice = None;
                self.send(request, ctx);
            }
            Row::Workspace | Row::Project { .. } => {
                let primary = ctx
                    .keys()
                    .label(views::HIERARCHY_BROWSE, Act::HierarchyPrimary);
                self.notice = Some(match primary {
                    Some(primary) => format!("`{primary}` wants a repo row"),
                    None => "only a repo row can be made primary".to_owned(),
                });
            }
        }
    }

    /// `d`: the first confirmation of PRD D13, which is a count rather than a question.
    ///
    /// The counting is a request of its own so the numbers on screen are the store's and not this
    /// section's arithmetic; `delete_reach` and `delete_*` share one implementation, so what is
    /// shown is what will go (M1 D4). The workspace row names the Workspaces tab's `delete`
    /// instead (MOD-116 M3 D17).
    fn begin_delete(&mut self, row: Row, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let (target, slug) = match row {
            Row::Workspace => {
                let delete = ctx.keys().label(views::WORKSPACES_BROWSE, Act::Delete);
                self.notice = Some(match delete {
                    Some(delete) => format!("`{delete}` on {WORKSPACES_TAB} deletes it"),
                    None => format!("delete the workspace on {WORKSPACES_TAB}"),
                });
                return;
            }
            Row::Project { index } => {
                let Some(entry) = snapshot.projects.get(index) else {
                    return;
                };
                (entry.project.id, entry.project.slug.clone())
            }
            // A repo is removed by deleting its project or not at all: `delete_repo` is not on the
            // seam and this milestone adds no method to it.
            Row::Repo { .. } => {
                self.notice = Some("repos are not deleted here".to_owned());
                return;
            }
        };
        self.notice = None;
        self.mode = Mode::Deleting {
            target,
            slug,
            stage: DeleteStage::Counting,
        };
        self.send(
            StoreRequest::DeleteReach(DeleteTarget::Project(target)),
            ctx,
        );
    }

    /// One key while a delete is being confirmed (D9, flag B).
    ///
    /// Modal over the shell as well as over the tree: a key that is not listed is **swallowed**,
    /// because the second confirmation is a typed slug and a `q` in the middle of one must not quit
    /// the application. A chord the stage's modal global layer admits (`CONTROL`, `ALT`, a function
    /// key) is the carve-out, so `ctrl-c` still quits and `F1` opens help (MOD-67 D5).
    ///
    /// The count and the warning answer through their stacks (`confirm.no`, and `confirm.yes` at
    /// the warning); the typed slug is its field's, which sees every key first (D13).
    fn on_deleting_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = self.stack();
        let chord = ctx.pressed(&key);
        let answer = ctx.keys().actions(stack, chord).first().copied();
        let Mode::Deleting {
            target,
            slug,
            stage,
        } = &mut self.mode
        else {
            return Handled::Pass;
        };
        let target = *target;
        match stage {
            DeleteStage::Counting | DeleteStage::Warn(_) => match answer {
                Some(Act::ConfirmYes) => {
                    if let DeleteStage::Warn(reach) = stage {
                        *stage = DeleteStage::Typed {
                            reach: reach.clone(),
                            field: TextField::new(),
                        };
                    }
                    return Handled::Consumed;
                }
                Some(Act::ConfirmNo) => {
                    self.mode = Mode::Browse;
                    return Handled::Consumed;
                }
                // A global act, or nothing.
                _ => return modal_rest(stack, chord),
            },
            DeleteStage::Typed { reach, field } => match field.on_key(key) {
                FieldOutcome::Submit => {
                    if field.text() == Some(slug.as_str()) {
                        *stage = DeleteStage::InFlight(reach.clone());
                        self.notice = None;
                        self.send(StoreRequest::DeleteProject(target), ctx);
                    } else {
                        field.clear();
                        self.notice = Some(WRONG_SLUG.to_owned());
                    }
                }
                FieldOutcome::Cancel => {
                    self.mode = Mode::Browse;
                    self.notice = None;
                }
                // Typed: `n` is a letter of a slug here, not an answer.
                FieldOutcome::Consumed => {}
                FieldOutcome::Pass => return modal_rest(stack, chord),
            },
            // Nothing to answer: the rows are already going.
            DeleteStage::InFlight(_) => return modal_rest(stack, chord),
        }
        Handled::Consumed
    }

    /// What the delete just took, and what the mirror did about it (D9, D10, flag I).
    fn deleted(&mut self, slug: &str, reach: &DeleteReach, mirror: &MirrorAfterDelete) {
        let (rows, tables) = reach_totals(reach);
        let mirror = match mirror {
            MirrorAfterDelete::Rebuilt => "mirror rebuilt".to_owned(),
            MirrorAfterDelete::NoMirror => "no mirror".to_owned(),
            MirrorAfterDelete::Failed(err) => format!("mirror not rebuilt: {err}"),
        };
        let rows = plural(usize::try_from(rows).unwrap_or(usize::MAX), "row", "rows");
        let tables = plural(tables, "table", "tables");
        self.notice = Some(format!(
            "deleted `{slug}`: {rows} across {tables}; {mirror}"
        ));
        self.mode = Mode::Browse;
        self.busy = None;
    }

    /// The slug of the delete being confirmed, for the notice that reports it afterwards.
    fn deleting_slug(&self) -> Option<String> {
        match &self.mode {
            Mode::Deleting { slug, .. } => Some(slug.clone()),
            Mode::Browse
            | Mode::Editing(_)
            | Mode::Picking { .. }
            | Mode::Memory { .. }
            | Mode::Documents { .. } => None,
        }
    }

    /// Sends one write and remembers its name until the reply.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// One key while an editor is open (D13).
    ///
    /// The focused field answers first, so `l`, `q` and the digits are letters here (D13); what it
    /// passes on is the form's own navigation (`form.next_field`/`prev_field` through
    /// `views::HIERARCHY_EDITOR`, `Down`/`Up` included), and everything left over is swallowed
    /// rather than offered to the shell — with the chords the modal global layer admits excepted,
    /// so `ctrl-c` still quits and `F1` opens help (MOD-67 D5).
    ///
    /// On the repo editor's `integrate` row (MOD-133 M2 E8) no field sees the key: the choice keys
    /// cycle the mode, and the field's submit and cancel act on the form, as `memory.rs`'s kind
    /// row does. Every candidate act of the chord is tried in stack order.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::HIERARCHY_EDITOR;
        let outcome = match &mut self.mode {
            Mode::Editing(editor) => match editor.fields.get_mut(editor.focus) {
                Some(field) => field.input.on_key(key),
                None => FieldOutcome::Pass,
            },
            Mode::Browse
            | Mode::Deleting { .. }
            | Mode::Picking { .. }
            | Mode::Memory { .. }
            | Mode::Documents { .. } => {
                return Handled::Pass;
            }
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => {
                let chord = ctx.pressed(&key);
                let Mode::Editing(editor) = &mut self.mode else {
                    return Handled::Pass;
                };
                let len = editor.rows().max(1);
                let on_choice = editor.on_integrate();
                let mut step = EditorStep::Done(modal_rest(stack, chord));
                for act in ctx.keys().actions(stack, chord) {
                    match act {
                        Act::FormNextField => editor.focus = (editor.focus + 1) % len,
                        Act::FormPrevField => editor.focus = (editor.focus + len - 1) % len,
                        Act::ChoicePrev if on_choice => editor.cycle_integrate(false),
                        Act::ChoiceNext if on_choice => editor.cycle_integrate(true),
                        Act::FieldSubmit if on_choice => {
                            step = EditorStep::Submit;
                            break;
                        }
                        Act::FieldCancel if on_choice => {
                            step = EditorStep::Cancel;
                            break;
                        }
                        // A global act, or one this row does not take.
                        _ => continue,
                    }
                    step = EditorStep::Done(Handled::Consumed);
                    break;
                }
                match step {
                    EditorStep::Done(handled) => handled,
                    EditorStep::Submit => {
                        self.submit(ctx);
                        Handled::Consumed
                    }
                    EditorStep::Cancel => {
                        self.mode = Mode::Browse;
                        self.notice = None;
                        Handled::Consumed
                    }
                }
            }
        }
    }

    /// One key while the picker is open (MOD-49 P8, blueprint H-4, D15; MOD-67 M6 D3).
    ///
    /// Over the go-to field a chord [`views::CAPTURE`] passes (`CONTROL`, `ALT`, a function key)
    /// is the shell's, so `ctrl-c` still quits and `F1` opens help; otherwise the picker resolves
    /// the key through the mode's stack and what it does not take is `modal_rest`'s: `q` can't
    /// quit mid-pick, `F1` and `ctrl-f` pass. A choice is a write: refused while another is in
    /// flight, and the picker stays open until its reply (D8).
    fn on_picker_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = self.stack();
        let chord = ctx.pressed(&key);
        let Mode::Picking { picker, .. } = &mut self.mode else {
            return Handled::Pass;
        };
        if picker.goto_open() && stack.passes(chord) {
            return Handled::Pass;
        }
        let Some(outcome) = picker.on_key(key, chord, ctx.keys(), stack) else {
            return modal_rest(stack, chord);
        };
        match outcome {
            PickerOutcome::Stay => {}
            PickerOutcome::Request(request) => ctx.request(request),
            PickerOutcome::Chosen(path) => {
                if self.in_flight() {
                    return Handled::Consumed;
                }
                let Mode::Picking { target, chosen, .. } = &mut self.mode else {
                    return Handled::Consumed;
                };
                let request = target.request(path.clone());
                *chosen = Some(path);
                self.notice = None;
                self.send(request, ctx);
            }
            PickerOutcome::Cancelled => {
                self.mode = Mode::Browse;
                self.notice = None;
            }
        }
        Handled::Consumed
    }

    /// `Enter` in an editor: the required fields, then one request per [`EditorKind`].
    ///
    /// The editor **stays open** until the reply lands, so a refusal (a duplicate slug, say)
    /// leaves the text where it was and a second `Enter` retries it. Which is why the first
    /// statement is the same refusal the Browse keys get: the editor being open is not a reply, and
    /// a second `Enter` before one arrives would re-send a write that already landed (D6).
    fn submit(&mut self, ctx: &mut Ctx<'_>) {
        if self.in_flight() {
            return;
        }
        let Mode::Editing(editor) = &self.mode else {
            return;
        };
        if let Some(missing) = editor
            .fields
            .iter()
            .find(|field| field.required && field.text().is_empty())
        {
            self.notice = Some(format!("`{}` is required", missing.label));
            return;
        }
        let expected = editor.expected;
        let request = match editor.kind {
            EditorKind::NewProject => {
                let Some(snapshot) = &self.snapshot else {
                    return;
                };
                StoreRequest::CreateProject {
                    workspace: snapshot.workspace.id,
                    slug: editor.text(0),
                    name: editor.text(1),
                    description: editor.text(2),
                }
            }
            EditorKind::EditProject(id) => {
                let Some(expected) = expected else {
                    self.notice = Some(DELETED_ELSEWHERE.to_owned());
                    return;
                };
                StoreRequest::UpdateProject {
                    id,
                    expected,
                    patch: ProjectPatch {
                        slug: Some(editor.text(0)),
                        name: Some(editor.text(1)),
                        description: Some(editor.text(2)),
                        secret: None,
                    },
                }
            }
            EditorKind::NewRepo(project) => {
                let Some(is_primary) = super::yes_or_no(&editor.text(3)) else {
                    self.notice = Some("`primary (y/n)` is y or n".to_owned());
                    return;
                };
                StoreRequest::CreateRepo {
                    project,
                    name: editor.text(0),
                    remote_url: super::some_text(editor.text(1)),
                    default_branch: editor.text(2),
                    is_primary,
                }
            }
            EditorKind::EditRepo { project, id } => {
                let Some(expected) = expected else {
                    self.notice = Some(DELETED_ELSEWHERE.to_owned());
                    return;
                };
                // MOD-133 M2 E8: the form is the guard; the stores keep no remote check.
                if let Some(refusal) = editor.integrate_refusal() {
                    self.notice = Some(refusal);
                    return;
                }
                StoreRequest::UpdateRepo {
                    project,
                    id,
                    expected,
                    patch: RepoPatch {
                        name: Some(editor.text(0)),
                        remote_url: Some(super::some_text(editor.text(1))),
                        default_branch: Some(editor.text(2)),
                        // Never the flag: `p` is its only writer (D12).
                        is_primary: None,
                        // MOD-133 M2 E8: always sent; `Some(None)` is the project's default.
                        integrate: Some(editor.integrate.flatten()),
                    },
                }
            }
        };
        self.notice = None;
        self.send(request, ctx);
    }

    /// A fresh tree: rows, cursor, whatever the write that asked for it owes the user, and the
    /// scope follow of D11.
    fn on_tree(&mut self, snapshot: &HierarchySnapshot, ctx: &Ctx<'_>) {
        let write = self.busy.take();
        self.unavailable = None;
        // MOD-7 milestone 4 (D114, D136): a write that can change what inference reads — a
        // repo's name or remote — is followed by one inference. Captured here, before the editor
        // is closed below; `p` has none, so it is never followed, and a repo path
        // (`set_repo_path`) isn't either. The root's write left with the workspace row's verbs
        // (MOD-116 M3 D17).
        let follow = write.is_some_and(|name| matches!(name, "create_repo" | "update_repo"))
            && match &self.mode {
                Mode::Editing(editor) => {
                    matches!(
                        editor.kind,
                        EditorKind::NewRepo(_) | EditorKind::EditRepo { .. }
                    )
                }
                Mode::Picking { .. }
                | Mode::Browse
                | Mode::Deleting { .. }
                | Mode::Memory { .. }
                | Mode::Documents { .. } => false,
            };
        if let Some(name) = write {
            self.notice = self.written(name, snapshot);
            // An **editor** or the picker only (MOD-49 D8). A reply carries no correlation, so a
            // read that lands while a delete is being counted would otherwise close a
            // confirmation nobody answered, and the `Deleted` that followed would report a slug the
            // section had already forgotten. The delete flow ends through `Deleted`,
            // `DeleteReach(None)`, a refusal or `Esc`.
            if matches!(self.mode, Mode::Editing(_) | Mode::Picking { .. }) {
                self.mode = Mode::Browse;
            }
        }
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        // MOD-163 D20: a memory pane whose project left the tree closes.
        if let Mode::Memory { project, .. } = &self.mode
            && !snapshot
                .projects
                .iter()
                .any(|entry| entry.project.id == *project)
        {
            self.mode = Mode::Browse;
            self.notice = Some(MEMORY_PROJECT_GONE.to_owned());
        }
        // MOD-147 M3 D9: a documents pane whose owner left the tree closes.
        if let Mode::Documents { owner, .. } = &self.mode {
            let present = match owner {
                DocumentOwner::Workspace(id) => snapshot.workspace.id == *id,
                DocumentOwner::Project(id) => snapshot
                    .projects
                    .iter()
                    .any(|entry| entry.project.id == *id),
            };
            if !present {
                self.mode = Mode::Browse;
                self.notice = Some(DOCUMENTS_OWNER_GONE.to_owned());
            }
        }

        // D11: the tree the section reads is what decides the scope, so a project created or
        // deleted here reaches the Backlog without either tab knowing the other exists. The loop
        // ends because `set_scope` re-issues the read and the second reply matches.
        let projects: Vec<ProjectId> = snapshot
            .projects
            .iter()
            .map(|entry| entry.project.id)
            .collect();
        if snapshot.workspace.id != ctx.scope.workspace_id || projects != ctx.scope.project_ids {
            ctx.emit(Action::SetScope {
                workspace: snapshot.summary(),
            });
        }

        // Only with a root on this box: without one the answer could only be "no root". The
        // `stored as …` notice set above stays on screen until the inference answers, and is
        // carried in front of its report.
        if follow && snapshot.folder_path.is_some() {
            self.carried = self.notice.clone();
            self.send(StoreRequest::InferRepoPaths(snapshot.workspace.id), ctx);
        }
    }

    /// What a write that applied has to say, if anything.
    ///
    /// Only the path write does: the guard canonicalises what was chosen (D8), and a path stored
    /// under a name the user did not navigate to is a surprise worth one line. It comes from the
    /// picker (MOD-49 blueprint D7), which holds the path it sent in `chosen`.
    fn written(&self, request: &'static str, snapshot: &HierarchySnapshot) -> Option<String> {
        let Mode::Picking {
            target,
            chosen: Some(chosen),
            ..
        } = &self.mode
        else {
            return None;
        };
        let stored = match *target {
            PathTarget::RepoPath { id, .. } if request == "set_repo_path" => snapshot
                .projects
                .iter()
                .flat_map(|entry| entry.repos.iter())
                .find(|repo| repo.repo.id == id)
                .and_then(|repo| repo.local_path.as_ref())
                .map(|path| path.local_path.clone())?,
            _ => return None,
        };
        (stored != *chosen).then(|| format!("stored as `{stored}`"))
    }

    /// A CAS miss (D7): the tree is replaced, the editor keeps its text and takes the current row's
    /// token, and the retry is a second `Enter` rather than an automatic write. `keys` name the
    /// key a `p` retries on (MOD-67 M6 D4), and the key an editor retries on: `field.submit`
    /// through the editor's stack (MOD-115 M3 D-5), or the picker's `choose`.
    fn on_stale(&mut self, snapshot: &HierarchySnapshot, keys: &Keys) {
        let retry = match &self.mode {
            Mode::Picking { .. } => keys.label(views::HIERARCHY_PICKER, Act::PickerChoose),
            _ => keys.label(views::HIERARCHY_EDITOR, Act::FieldSubmit),
        };
        let retry = changed_elsewhere(retry.as_deref());
        self.busy = None;
        let refreshed = match &self.mode {
            Mode::Editing(editor) => Some(reload(snapshot, editor.kind)),
            // MOD-49 blueprint D19: unreachable — path writes are upserts and never answer
            // `HierarchyStale` — and harmless if it ever were: the picker keeps its place.
            Mode::Picking { .. } => Some(Reload::Keep),
            // `p` is the one write with no editor behind it, and a delete has no CAS token at all.
            // The memory and documents panes write no tree, so a tree's CAS miss is never theirs.
            Mode::Browse | Mode::Deleting { .. } | Mode::Memory { .. } | Mode::Documents { .. } => {
                None
            }
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        match refreshed {
            // No editor: `p` is the only write that gets here, and its retry is the key again.
            None => {
                let primary = keys.label(views::HIERARCHY_BROWSE, Act::HierarchyPrimary);
                self.notice = Some(reloaded(primary.as_deref()));
            }
            Some(Reload::Gone) => {
                self.mode = Mode::Browse;
                self.notice = Some(DELETED_ELSEWHERE.to_owned());
            }
            Some(Reload::Token(token)) => {
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.expected = Some(token);
                }
                self.notice = Some(retry);
            }
            Some(Reload::Keep) => self.notice = Some(retry),
        }
    }
}

impl SettingsSection for HierarchySection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Hierarchy"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        let mut out = vec![StoreRequest::Hierarchy(scope.workspace_id)];
        // MOD-163 D20: an open memory pane reads its list again on activation.
        if let Mode::Memory { pane, .. } = &self.mode {
            out.push(pane.read_request());
        }
        // MOD-147 M3 D9: so does an open documents pane.
        if let Mode::Documents { pane, .. } = &self.mode {
            out.push(pane.read_request());
        }
        out
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // An editor open across a workspace switch is dropped on purpose (D5): its CAS token
        // belongs to the other workspace. The notice survives, because the scope change is often
        // the *consequence* of what it is reporting.
        self.snapshot = None;
        self.mode = Mode::Browse;
        self.busy = None;
        self.carried = None;
        self.cursor = 0;
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    /// MOD-22 review M-1: a bracketed paste into the open form's focused field, or into the
    /// typed-slug confirmation. The warning before it is not a field: a paste there is dropped, so
    /// its `y` advances nothing.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        match &mut self.mode {
            Mode::Editing(editor) => {
                if let Some(field) = editor.fields.get_mut(editor.focus) {
                    field.input.on_paste(text);
                }
                Handled::Consumed
            }
            Mode::Deleting {
                stage: DeleteStage::Typed { field, .. },
                ..
            } => {
                field.on_paste(text);
                Handled::Consumed
            }
            // MOD-49 P6: a paste into the picker's go-to field, opening it when it is closed.
            Mode::Picking { picker, .. } => {
                picker.on_paste(text);
                Handled::Consumed
            }
            Mode::Memory { pane, .. } => pane.on_paste(text),
            Mode::Documents { pane, .. } => pane.on_paste(text),
            Mode::Browse | Mode::Deleting { .. } => Handled::Pass,
        }
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // MOD-163 D20: the memory pane takes every key while it is open; its `back` closes it.
        if let Mode::Memory { pane, .. } = &mut self.mode {
            return match pane.on_key(key, ctx) {
                PaneKey::Handled(handled) => handled,
                PaneKey::Close => {
                    self.mode = Mode::Browse;
                    Handled::Consumed
                }
            };
        }
        // MOD-147 M3 D9: so does the documents pane.
        if let Mode::Documents { pane, .. } = &mut self.mode {
            return match pane.on_key(key, ctx) {
                PaneKey::Handled(handled) => handled,
                PaneKey::Close => {
                    self.mode = Mode::Browse;
                    Handled::Consumed
                }
            };
        }
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
        if matches!(self.mode, Mode::Picking { .. }) {
            return self.on_picker_key(key, ctx);
        }
        if matches!(self.mode, Mode::Deleting { .. }) {
            return self.on_deleting_key(key, ctx);
        }
        self.on_browse_key(key, ctx)
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            // MOD-163 D21: the memory pane's replies, and its requests' refusals, are the pane's;
            // with no pane open they land nowhere.
            StoreReply::Memory { .. }
            | StoreReply::MemoryWritten { .. }
            | StoreReply::MemoryStale { .. }
            | StoreReply::MemorySubjectTaken { .. } => {
                if let Mode::Memory { pane, .. } = &mut self.mode {
                    pane.on_reply(reply, ctx);
                }
            }
            StoreReply::Failed { request, .. } if MEMORY_REQUEST_NAMES.contains(request) => {
                if let Mode::Memory { pane, .. } = &mut self.mode {
                    pane.on_reply(reply, ctx);
                }
            }
            // MOD-147 M3 D9: the documents pane's, likewise.
            StoreReply::OwnerDocuments { .. } | StoreReply::OwnerDocumentWritten { .. } => {
                if let Mode::Documents { pane, .. } = &mut self.mode {
                    pane.on_reply(reply, ctx);
                }
            }
            StoreReply::Failed { request, .. } if DOCUMENT_REQUEST_NAMES.contains(request) => {
                if let Mode::Documents { pane, .. } = &mut self.mode {
                    pane.on_reply(reply, ctx);
                }
            }
            StoreReply::Hierarchy(Some(snapshot)) => self.on_tree(snapshot, ctx),
            // MOD-147 D6: a join is a tree like any other; a prefix it shares with the workspace
            // is one notice line, never a refusal.
            StoreReply::HierarchyJoined {
                tree,
                project,
                shared_prefixes,
            } => {
                self.on_tree(tree, ctx);
                if !shared_prefixes.is_empty() {
                    self.notice = Some(joined_notice(
                        project,
                        &tree.workspace.slug,
                        shared_prefixes,
                    ));
                }
            }
            // A read that answered, even with nothing in it, is the end of an outage: leaving
            // `unavailable` set would say "hierarchy needs Postgres" over a store that just spoke.
            StoreReply::Hierarchy(None) => {
                self.busy = None;
                self.unavailable = None;
                self.snapshot = None;
                self.mode = Mode::Browse;
                self.clamp_cursor();
            }
            StoreReply::HierarchyStale(snapshot) => self.on_stale(snapshot, ctx.keys()),
            // MOD-7 milestone 4 (D117): a tree like any other, then what the pass did. A notice a
            // write left (`stored as …`, a follow-up's cause) is kept in front of the report; a
            // refusal shown during the walk is not, since the walk it waited for is over.
            StoreReply::RepoPathsInferred { tree, report } => {
                let before = self.carried.take();
                self.on_tree(tree, ctx);
                let choose = ctx
                    .keys()
                    .label(views::HIERARCHY_BROWSE, Act::HierarchyChoosePath);
                let edit = ctx.keys().label(views::WORKSPACES_BROWSE, Act::Edit);
                let infer = ctx
                    .keys()
                    .label(views::HIERARCHY_BROWSE, Act::HierarchyInfer);
                let report =
                    inferred_notice(report, choose.as_deref(), edit.as_deref(), infer.as_deref());
                self.notice = Some(match before {
                    Some(before) => format!("{before} \u{b7} {report}"),
                    None => report,
                });
            }
            StoreReply::DeleteReach(Some(reach)) => {
                self.busy = None;
                if let Mode::Deleting { stage, .. } = &mut self.mode
                    && matches!(stage, DeleteStage::Counting)
                {
                    *stage = DeleteStage::Warn(reach.clone());
                }
            }
            // Nothing to count and nothing to delete: someone else got there first.
            StoreReply::DeleteReach(None) => {
                self.busy = None;
                self.mode = Mode::Browse;
                self.notice = Some("already gone".to_owned());
                ctx.request(StoreRequest::Hierarchy(ctx.scope.workspace_id));
            }
            // The fresh tree is what moves the scope: D11's arm emits `SetScope` because the
            // project list changed, and that is how the Backlog stops listing it. A project is the
            // one thing deleted here: a workspace is deleted on the Workspaces tab, and its reply
            // goes there (MOD-116 M3 D17).
            StoreReply::Deleted {
                target: DeleteTarget::Project(_),
                reach,
                mirror,
            } => {
                let slug = self.deleting_slug().unwrap_or_default();
                self.deleted(&slug, reach, mirror);
                ctx.request(StoreRequest::Hierarchy(ctx.scope.workspace_id));
            }
            // MOD-10 M4 (blueprint A-1): the Secrets section's scope write. The tree is adopted so
            // the tokens stay current; nothing else here is this section's — no `busy`, no
            // editor, no notice, no scope follow.
            // R1 L-3: the Secrets section's own tree read, adopted the same way.
            StoreReply::SecretScopeWritten { tree, .. } | StoreReply::SecretsTree(Some(tree)) => {
                if tree.workspace.id == ctx.scope.workspace_id {
                    self.snapshot = Some((**tree).clone());
                    self.clamp_cursor();
                }
            }
            // MOD-49: a listing, or its refusal, for the open picker (blueprint D2, B-3). The
            // shell has already put a refusal on the status line too (D16).
            StoreReply::DirListing(_)
            | StoreReply::Failed {
                request: LIST_DIR, ..
            } => {
                if let Mode::Picking { picker, .. } = &mut self.mode {
                    picker.on_reply(reply);
                }
            }
            // The read itself was refused: saying so beats an empty tree that reads as "nothing
            // here yet" (the agent section's rule, one section across).
            StoreReply::Failed { request, message } if *request == "hierarchy" => {
                self.busy = None;
                self.unavailable = Some(message.clone());
            }
            // Every other refusal of this section's own: the shell has already put
            // `{request}: {message}` on the status line, so all this owes is a state the next key
            // can start from — with the editor left open over its text.
            StoreReply::Failed { request, .. } if REQUEST_NAMES.contains(request) => {
                self.busy = None;
                if *request == "infer_repo_paths" {
                    self.carried = None;
                }
                // MOD-49 D8, D21: a refused path write leaves the picker open for another choice,
                // and forgets what it sent so a later tree isn't labelled with it.
                if let Mode::Picking { chosen, .. } = &mut self.mode {
                    *chosen = None;
                }
                // A delete that was refused must not leave `deleting…` or `counting rows…` on
                // screen: neither stage has anything left to wait for. `InFlight` goes back to the
                // counts the user already saw, where `y` retries and `Esc` stops; `Counting` has no
                // counts to go back to, so it leaves the tree, where `d` starts again.
                if let Mode::Deleting { stage, .. } = &mut self.mode {
                    match stage {
                        DeleteStage::InFlight(reach) => *stage = DeleteStage::Warn(reach.clone()),
                        DeleteStage::Counting => self.mode = Mode::Browse,
                        DeleteStage::Warn(_) | DeleteStage::Typed { .. } => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    /// MOD-147 M3 D10: the documents pane asked for the edit; with no pane open the outcome
    /// lands nowhere.
    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        match &mut self.mode {
            Mode::Documents { pane, .. } => pane.on_external_edit(outcome, ctx),
            _ => tracing::debug!("no documents pane to take the $EDITOR outcome"),
        }
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        if let Mode::Memory { pane, .. } = &self.mode {
            return pane.scroll();
        }
        if let Mode::Documents { pane, .. } = &self.mode {
            return pane.scroll();
        }
        self.scroll.get()
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // MOD-163 D20: the memory pane draws over the whole section.
        if let Mode::Memory { pane, .. } = &self.mode {
            pane.render(frame, area, ctx);
            return;
        }
        // MOD-147 M3 D9: so does the documents pane.
        if let Mode::Documents { pane, .. } = &self.mode {
            pane.render(frame, area, ctx);
            return;
        }
        let theme = ctx.theme;
        let pane = self.pane(area.width, ctx);
        let [rows, pane_area, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        self.scroll.set(None);
        if self.unavailable.is_some() {
            // The refusal in the error role, then its next step in the empty state's own.
            let reload = ctx.keys().label(views::HIERARCHY_BROWSE, Act::Reload);
            let next = unavailable_next(reload.as_deref());
            let mut lines: Vec<Line<'static>> =
                wrapped(UNAVAILABLE, usize::from(rows.width).max(1))
                    .into_iter()
                    .map(|line| notice(&line, Tone::Error, theme))
                    .collect();
            lines.extend(empty::lines("", NextStep(&next), theme).pop());
            frame.render_widget(Paragraph::new(lines), rows);
        } else if self.snapshot.is_none() {
            let new = ctx
                .keys()
                .label(views::WORKSPACES_BROWSE, Act::WorkspacesNew);
            empty::render(
                frame,
                rows,
                NO_WORKSPACE,
                NextStep(&creates_one(new.as_deref())),
                theme,
            );
        } else {
            // Scrolled so the cursor is on screen: a workspace with several projects and their
            // repos is longer than the pane, and a selection nobody can see acts blind.
            let lines = self.list_rows(theme);
            let total = lines.len();
            let offset = list::scroll_into_view(self.cursor, 0, usize::from(rows.height), total);
            list::render(
                frame,
                rows,
                &lines,
                Some(self.cursor),
                offset,
                ctx.accent(),
                theme,
            );
            // The pane's bar is measured against the rows the tree was drawn into and sits
            // beside them (RV1 M2's span): the rows under the tree do not scroll.
            self.scroll
                .set(Some(ScrollSpan::new(total, offset).rows(rows)));
        }
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), pane_area);
        }
        frame.render_widget(Paragraph::new(self.notice_line(theme)), hint);
        // MOD-49 P2: the popup draws over the whole section, not in the pane.
        if let Mode::Picking { picker, .. } = &self.mode {
            // Always the browse stack: the popup's hint stays while the go-to field is open.
            picker.render(frame, area, theme, ctx.keys(), views::HIERARCHY_PICKER);
        }
    }
}

impl Field {
    /// A field `Enter` refuses while it is empty.
    fn required(label: &'static str, text: &str) -> Self {
        Self {
            label,
            input: TextField::with_text(text),
            required: true,
        }
    }

    /// A field that may stay empty: an empty `description` is `""` and an empty `remote_url` is
    /// `None`.
    fn optional(label: &'static str, text: &str) -> Self {
        Self {
            label,
            input: TextField::with_text(text),
            required: false,
        }
    }

    /// What was typed. Never masked here, so [`TextField::text`] always answers.
    fn text(&self) -> &str {
        self.input.text().unwrap_or_default()
    }
}

impl Editor {
    /// The text of field `index`, or `""` when the editor has no such field.
    fn text(&self, index: usize) -> String {
        self.fields
            .get(index)
            .map_or_else(String::new, |field| field.text().to_owned())
    }

    /// The rows focus cycles over: the fields, then the `integrate` row when there is one.
    fn rows(&self) -> usize {
        self.fields.len() + usize::from(self.integrate.is_some())
    }

    /// Whether the focus is on the `integrate` row (MOD-133 M2 E8).
    fn on_integrate(&self) -> bool {
        self.integrate.is_some() && self.focus == self.fields.len()
    }

    /// One step of the `integrate` choice: `default → local → push → pull_request → default`
    /// forward, the reverse back.
    fn cycle_integrate(&mut self, forward: bool) {
        const ORDER: [Option<IntegrateMode>; 4] = [
            None,
            Some(IntegrateMode::Local),
            Some(IntegrateMode::Push),
            Some(IntegrateMode::PullRequest),
        ];
        let Some(choice) = &mut self.integrate else {
            return;
        };
        let at = ORDER.iter().position(|mode| mode == choice).unwrap_or(0);
        let next = if forward {
            (at + 1) % ORDER.len()
        } else {
            (at + ORDER.len() - 1) % ORDER.len()
        };
        *choice = ORDER[next];
    }

    /// Whether the `remote url` field holds a URL.
    fn has_remote(&self) -> bool {
        !self.text(REMOTE_URL_FIELD).trim().is_empty()
    }

    /// Why the chosen mode cannot be saved, or `None`; the inherited default never refuses here
    /// (runs refuse at start, M1 D3).
    fn integrate_refusal(&self) -> Option<String> {
        self.integrate
            .flatten()
            .and_then(|mode| mode.selectable_refusal(self.has_remote()))
    }

    /// The `integrate` row (MOD-133 M2 E8): the choice, its refusal as the row's error, and as
    /// help either the default's refusal (help only) or what the row is.
    fn integrate_field(&self, choice: Option<IntegrateMode>) -> form::Field<'_> {
        let word: &str = match choice {
            Some(mode) => mode.as_str(),
            None => &self.default_word,
        };
        let field = form::Field::new(INTEGRATE_LABEL, Value::Choice(word));
        if let Some(refusal) = self.integrate_refusal() {
            return field.error(refusal);
        }
        let inherited = choice
            .is_none()
            .then_some(self.inherited)
            .flatten()
            .and_then(|mode| mode.selectable_refusal(self.has_remote()));
        field.help(inherited.unwrap_or_else(|| INTEGRATE_HELP.to_owned()))
    }

    /// The fields as a kit form (MOD-115 M3 D3): one row per field in tab order, the focused one
    /// carrying the marker and the cursor, the labels in one shared column; the repo editor's
    /// `integrate` choice last (MOD-133 M2 E8).
    fn form(&self) -> Form<'_> {
        let form = self.fields.iter().fold(Form::new(), |form, field| {
            form.field(form::Field::new(field.label, Value::Text(&field.input)))
        });
        let form = match self.integrate {
            Some(choice) => form.field(self.integrate_field(choice)),
            None => form,
        };
        form.focus(self.focus).editing(true)
    }
}

/// What a reloaded tree does to an open editor's CAS token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reload {
    /// The editor's row is not in the reloaded tree.
    Gone,
    /// The row's `updated_at` as it is now.
    Token(DateTime<Utc>),
    /// Nothing to refresh: a create has no CAS token, and neither has the picker (MOD-49 D19).
    Keep,
}

/// The token an open editor should retry against after a reload.
fn reload(snapshot: &HierarchySnapshot, kind: EditorKind) -> Reload {
    match kind {
        EditorKind::NewProject => Reload::Keep,
        EditorKind::EditProject(id) => snapshot
            .projects
            .iter()
            .find(|entry| entry.project.id == id)
            .map_or(Reload::Gone, |entry| {
                Reload::Token(entry.project.updated_at)
            }),
        EditorKind::NewRepo(project) => snapshot
            .projects
            .iter()
            .find(|entry| entry.project.id == project)
            .map_or(Reload::Gone, |_| Reload::Keep),
        EditorKind::EditRepo { id, .. } => snapshot
            .projects
            .iter()
            .flat_map(|entry| entry.repos.iter())
            .find(|entry| entry.repo.id == id)
            .map_or(Reload::Gone, |entry| Reload::Token(entry.repo.updated_at)),
    }
}

/// MOD-147 D6: the notice a join that shares prefixes with its workspace leaves: "`api` shares
/// prefix FEAT with workspace `shop`: keys show as `api:FEAT-1` and `@shop:FEAT-1`" ("prefixes
/// FEAT, FIX" for several; the example uses the first).
#[must_use]
pub fn joined_notice(project: &str, workspace: &str, prefixes: &[String]) -> String {
    let first = prefixes.first().map_or("", String::as_str);
    let noun = if prefixes.len() == 1 {
        "prefix"
    } else {
        "prefixes"
    };
    format!(
        "`{project}` shares {noun} {} with workspace `{workspace}`: keys show as \
         `{project}:{first}-1` and `@{workspace}:{first}-1`",
        prefixes.join(", ")
    )
}

/// The notice an inference leaves (MOD-7 milestone 4, plan D117, blueprint D137).
///
/// Three sentences for the answers that inferred nothing by construction — no root here, a
/// truncated scan, a workspace without a repo — and otherwise `inferred {n} of {m}`, where `m`
/// counts every repo that had no row before the pass, followed by what was not inferred. A refused
/// repo is named, not explained: its sentence may name a path, and the notice is one line.
///
/// `choose` is the label of `settings.hierarchy.choose_path` (MOD-67 M6 D4), the key the notice
/// sends the user to on a repo row (`b` under the defaults), `edit` the Workspaces tab's `edit`,
/// whose form sets the workspace's folder (MOD-116 M3 D17), and `infer` the label of
/// `settings.hierarchy.infer`, which runs the pass once the folder is set: that edit runs none
/// (R1b B9). Unbound, the notice names no key; `infer` unbound drops its step.
pub(crate) fn inferred_notice(
    report: &InferReport,
    choose: Option<&str>,
    edit: Option<&str>,
    infer: Option<&str>,
) -> String {
    if report.root.is_none() {
        let set = match edit {
            Some(edit) => format!("{edit} on {WORKSPACES_TAB} sets its folder"),
            None => format!("set its folder on {WORKSPACES_TAB}"),
        };
        let then = infer.map_or_else(String::new, |infer| format!(", then {infer} here"));
        return format!("no root on this box for this workspace \u{2014} {set}{then}");
    }
    if report.truncated {
        return format!(
            "the scan stopped at {} directories; nothing inferred",
            htui_orch::infer::MAX_DIRS
        );
    }
    if report.repos.is_empty() {
        return "no repo in this workspace to infer".to_owned();
    }

    let mut inferred = 0_usize;
    let mut already = 0_usize;
    let mut unmatched = Vec::new();
    let mut ambiguous = Vec::new();
    let mut refused = Vec::new();
    for line in &report.repos {
        match &line.outcome {
            InferOutcome::Inferred { .. } => inferred += 1,
            InferOutcome::AlreadySet => already += 1,
            InferOutcome::NoMatch => unmatched.push(line.name.clone()),
            InferOutcome::Ambiguous { candidates } => {
                ambiguous.push(format!("{} ({candidates})", line.name));
            }
            InferOutcome::Refused(_) => refused.push(line.name.clone()),
            // Only in a truncated report, which answered above.
            InferOutcome::ScanTruncated => {}
        }
    }

    let total = report.repos.len() - already;
    let mut parts = vec![format!("inferred {inferred} of {total}")];
    if already > 0 {
        parts.push(format!("{already} already set"));
    }
    for (label, names) in [
        ("no checkout", &unmatched),
        ("ambiguous", &ambiguous),
        ("refused", &refused),
    ] {
        if !names.is_empty() {
            parts.push(format!("{label}: {}", capped(names)));
        }
    }
    let mut notice = parts.join(" \u{b7} ");
    if !(unmatched.is_empty() && ambiguous.is_empty() && refused.is_empty()) {
        match choose {
            Some(choose) => {
                notice.push_str(&format!(" \u{2014} {choose} on a repo sets it by hand"))
            }
            None => notice.push_str(" \u{2014} set it by hand on a repo row"),
        }
    }
    notice
}

/// At most [`NAMES_SHOWN`] names, comma-separated, then `+{rest} more`.
fn capped(names: &[String]) -> String {
    let shown = names
        .iter()
        .take(NAMES_SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    match names.len().saturating_sub(NAMES_SHOWN) {
        0 => shown,
        rest => format!("{shown} +{rest} more"),
    }
}

/// What a second write is told while the first is still out (D6).
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// A request in flight in words (MOD-115 M3: no request name on screen), by
/// [`StoreRequest::name`] ([`REQUEST_NAMES`]).
fn doing(busy: &str) -> &'static str {
    match busy {
        "hierarchy" => "reading the tree",
        "create_project" => "adding the project",
        "update_project" => "saving the project",
        "create_repo" => "adding the repo",
        "update_repo" => "saving the repo",
        "set_repo_path" => "saving the checkout",
        "delete_reach" => "counting rows",
        "delete_project" => "deleting the project",
        "infer_repo_paths" => "inferring the repo paths",
        _ => "writing",
    }
}

/// The repo at `index` of a project entry, with the project it belongs to.
fn repo_at(owner: &ProjectEntry, index: usize) -> Option<(ProjectId, &RepoEntry)> {
    owner.repos.get(index).map(|repo| (owner.project.id, repo))
}

/// The pane of a project's delete, every warning line in `theme.error` (D9).
///
/// The counts are the store's own, in [`DeleteReach`]'s field order and with the zeros left out:
/// the pane lists what a delete takes, and "0 runs" is not something being taken.
fn delete_pane(slug: &str, stage: &DeleteStage, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let reach = match stage {
        DeleteStage::Counting => {
            return vec![notice("counting rows\u{2026}", Tone::Info, theme)];
        }
        DeleteStage::InFlight(_) => {
            return vec![notice(
                &format!("deleting `{slug}`\u{2026}"),
                Tone::Info,
                theme,
            )];
        }
        DeleteStage::Warn(reach) | DeleteStage::Typed { reach, .. } => reach,
    };

    let parts = reach_parts(reach);
    let taken = if parts.is_empty() {
        "no other rows".to_owned()
    } else {
        parts.join(", ")
    };
    let headline =
        format!("This deletes project `{slug}` and its entire history. Gone for good: {taken}.");

    let room = usize::from(width).max(1);
    let mut lines: Vec<Line<'static>> = wrapped(&headline, room)
        .into_iter()
        .chain(wrapped(NOT_UNDONE, room))
        .map(|line| notice(&line, Tone::Error, theme))
        .collect();
    if let DeleteStage::Typed { field, .. } = stage {
        let prompt = format!("Type `{slug}` to confirm: ");
        let used = cell_width(&prompt);
        let mut spans = vec![Span::styled(prompt, theme.error)];
        spans.extend(
            field
                .line(
                    u16::try_from(room.saturating_sub(used)).unwrap_or(u16::MAX),
                    true,
                    theme,
                )
                .spans,
        );
        lines.push(Line::from(spans));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-147 M3 review M1: the section's home is the one the worker resolves the default folder
    /// under (`dirs::home_dir()`), not `$HOME`, which Windows does not set.
    #[test]
    fn the_home_is_the_workers_home_dir() {
        let worker = dirs::home_dir().and_then(|home| home.to_str().map(str::to_owned));
        assert_eq!(HierarchySection::new().home, worker);
    }

    /// MOD-147 M3 review M1: the part under the home is written with `/` whatever the native
    /// separator, so `~/workspaces/graphics` reads the same on every box.
    #[test]
    fn the_part_under_home_is_joined_with_slashes() {
        let rest = Path::new("workspaces").join("graphics");
        assert_eq!(under_home(&rest), "~/workspaces/graphics");
        assert_eq!(under_home(Path::new("")), "~");
    }

    /// MOD-60: the confirm field's room is what the prompt leaves in **cells**. A CJK slug is
    /// twice as wide as its `char` count, and measuring it by `char` handed the field 8 cells
    /// (one per CJK `char`) the line does not have.
    #[test]
    fn a_wide_slug_leaves_the_confirm_field_its_room() {
        let width = 40;
        let stage = DeleteStage::Typed {
            reach: Box::default(),
            field: TextField::with_text(&"x".repeat(60)),
        };

        let lines = delete_pane(&"\u{6f22}".repeat(8), &stage, width, &Theme::default());

        let last = lines.last().expect("the confirm line");
        let prompt = format!("Type `{}` to confirm: ", "\u{6f22}".repeat(8));
        assert_eq!(last.spans[0].content, prompt);
        // The prompt is 36 cells, so the field gets the 4 left: `…`, two `x`, the cursor cell.
        let field: usize = last.spans[1..]
            .iter()
            .map(|span| cell_width(&span.content))
            .sum();
        assert_eq!(field, usize::from(width) - cell_width(&prompt), "{last:?}");
        assert_eq!(cell_width(&prompt) + field, usize::from(width), "{last:?}");
    }

    /// MOD-67 M6 T2: the row under the pane carries the notice alone, and the keys are the
    /// section's [`SettingsSection::key_hint`], the shell's row.
    #[test]
    fn the_notice_row_carries_the_notice_alone() {
        let notice = changed_elsewhere(Some("Enter"));
        let section = HierarchySection {
            notice: Some(notice.clone()),
            ..HierarchySection::new()
        };
        let theme = Theme::default();
        let line = section.notice_line(&theme);
        assert_eq!(line.spans.len(), 1, "{line:?}");
        assert_eq!(line.spans[0].content, notice);
        assert_eq!(line.style, theme.error);
        let keys = section
            .key_hint(Keys::compiled(), &theme)
            .expect("the defaults bind the keys");
        assert_eq!(keys.to_string(), section.hint_text(Keys::compiled()));
    }

    /// MOD-67 M6 D3: the picker resolves its keys through `views::HIERARCHY_PICKER`; its go-to
    /// field is a text field and nothing else, so the section is on `views::CAPTURE` while it is
    /// open.
    #[test]
    fn the_picker_mode_reports_the_picker_stack() {
        let (picker, _) = PathPicker::open("Root", "/srv".to_owned());
        let mut section = HierarchySection {
            mode: Mode::Picking {
                target: PathTarget::RepoPath {
                    project: ProjectId::new(),
                    id: RepoId::new(),
                },
                picker,
                chosen: None,
            },
            ..HierarchySection::new()
        };
        assert_eq!(section.key_stack(), Some(views::HIERARCHY_PICKER));

        if let Mode::Picking { picker, .. } = &mut section.mode {
            picker.on_paste("/opt");
        }
        assert_eq!(section.key_stack(), Some(views::CAPTURE));
    }

    /// MOD-115 M3: a write in flight reads in words. Every request name the section sends has its
    /// own phrase, and neither the refusal nor the Browse hint prints the name itself. The four the
    /// workspace row's verbs sent are the Workspaces tab's now (MOD-116 M3 D17): still served, but
    /// never in flight here.
    #[test]
    fn a_write_in_flight_reads_in_words() {
        const NOT_SENT_HERE: [&str; 4] = [
            "create_workspace",
            "update_workspace",
            "set_workspace_root",
            "delete_workspace",
        ];
        for name in REQUEST_NAMES {
            if NOT_SENT_HERE.contains(&name) {
                assert_eq!(doing(name), doing("unknown"), "`{name}` is not sent here");
                continue;
            }
            assert_ne!(doing(name), doing("unknown"), "`{name}` has its own phrase");
            assert!(!in_flight(name).contains('_'), "{}", in_flight(name));
        }
        let section = HierarchySection {
            busy: Some(REQUEST_NAMES[7]),
            ..HierarchySection::new()
        };
        let hint = section.hint_text(Keys::compiled());
        assert!(hint.ends_with(" \u{b7} saving the repo\u{2026}"), "{hint}");
    }

    /// MOD-115 M3 C6: the typed confirmation's keys are the field's through `views::CAPTURE`,
    /// so a rebound `field.submit` is the one it names.
    #[test]
    fn the_typed_hint_names_the_field_keys() {
        let keys = crate::keys::load_str("version = 2\n[field]\nsubmit = [\"ctrl-t\"]\n")
            .expect("the key file loads");
        assert_eq!(
            Keys::compiled().hint(views::CAPTURE, HINT_TYPED),
            "Enter confirm \u{b7} Esc stop"
        );
        assert_eq!(
            keys.hint(views::CAPTURE, HINT_TYPED),
            "Ctrl+t confirm \u{b7} Esc stop"
        );
    }

    /// MOD-67 M6 D4: each key-naming sentence names the label it is handed, and none when the
    /// act is unbound.
    #[test]
    fn the_prose_names_the_keys_in_force() {
        assert_eq!(
            no_workspace(Some("W")),
            "no workspace \u{2014} `W` on the Workspaces tab creates one"
        );
        assert_eq!(no_workspace(None), "no workspace");
        assert_eq!(
            creates_one(Some("W")),
            "`W` on the Workspaces tab creates one"
        );
        assert_eq!(
            creates_one(None),
            "a workspace created elsewhere shows up here"
        );
        assert_eq!(
            unavailable_next(Some("R")),
            "Settings \u{203a} Connection stores a DSN \u{b7} R asks again once the store answers"
        );
        assert_eq!(
            unavailable_next(None),
            "Settings \u{203a} Connection stores a DSN"
        );
        assert_eq!(reloaded(Some("P")), "reloaded; press P again");
        assert_eq!(reloaded(None), "reloaded");

        let unmatched = InferReport {
            root: Some("/work".to_owned()),
            truncated: false,
            repos: vec![crate::hierarchy::RepoInference {
                repo: RepoId::new(),
                name: "docs".to_owned(),
                outcome: InferOutcome::NoMatch,
            }],
        };
        assert_eq!(
            inferred_notice(&unmatched, Some("B"), Some("E"), Some("I")),
            "inferred 0 of 1 \u{b7} no checkout: docs \u{2014} B on a repo sets it by hand"
        );
        assert_eq!(
            inferred_notice(&unmatched, None, None, None),
            "inferred 0 of 1 \u{b7} no checkout: docs \u{2014} set it by hand on a repo row"
        );

        let rootless = InferReport {
            root: None,
            truncated: false,
            repos: Vec::new(),
        };
        // R1b B9: the folder set, `infer` here runs the pass (the tab's edit runs none).
        assert_eq!(
            inferred_notice(&rootless, Some("B"), Some("E"), Some("I")),
            "no root on this box for this workspace \u{2014} E on the Workspaces tab sets its \
             folder, then I here"
        );
        assert_eq!(
            inferred_notice(&rootless, None, None, Some("I")),
            "no root on this box for this workspace \u{2014} set its folder on the Workspaces tab, \
             then I here"
        );
        assert_eq!(
            inferred_notice(&rootless, Some("B"), Some("E"), None),
            "no root on this box for this workspace \u{2014} E on the Workspaces tab sets its folder"
        );
        assert_eq!(
            inferred_notice(&rootless, None, None, None),
            "no root on this box for this workspace \u{2014} set its folder on the Workspaces tab"
        );
    }
}
