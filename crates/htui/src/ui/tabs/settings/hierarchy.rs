//! The hierarchy section of the Settings tab: the workspace, its projects, their repos and this
//! box's paths, editable in place (MOD-15 milestone 3, D11/D12/D13; `R-ENT-1..4`, `R-BOX-4`).
//!
//! It holds **no store handle, no `UserId` and no `BoxId`** (`R-NF-3`): it names one read
//! ([`StoreRequest::Hierarchy`]), is handed the tree that comes back, and every write leaves
//! through `ctx.request` for [`crate::hierarchy::serve`] to fill the identity columns in. What is
//! on screen is always the last snapshot the worker assembled — no row is ever patched in locally,
//! so there is exactly one source of truth.
//!
//! Three modes, and the mode is what [`captures_input`](SettingsSection::captures_input) is derived
//! from rather than a flag of its own (D2): in `Browse` the tab still cycles on `h`/`l` and the
//! global table still owns `q`, `?`, `Tab` and the digits; while an editor or a delete
//! confirmation is open those letters are text and are swallowed, with one carve-out — a chord
//! carrying `CONTROL` always passes, so `ctrl-c` quits from inside a half-typed slug.

use chrono::{DateTime, Utc};
use htui_core::model::{
    ProjectId, ProjectPatch, RepoId, RepoPatch, Scope, WorkspaceId, WorkspacePatch,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled};
use crate::hierarchy::{HierarchySnapshot, ProjectEntry, REQUEST_NAMES, RepoEntry};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{SectionId, SettingsSection, message};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the rows pane says when the scope is a workspace that does not exist — the nil startup
/// scope, or the last workspace after it was deleted (D9's "stranded shell" row).
const NO_WORKSPACE: &str = "no workspace — `N` creates one";

/// What the rows pane says when the read itself was refused: the tree is one `ReadStore` call and
/// an offline box has no mirror of it.
const UNAVAILABLE: &str = "hierarchy needs Postgres";

/// Browse's keys.
const HINT_BROWSE: &str = "j/k \u{b7} N workspace \u{b7} n project/repo \u{b7} e edit \u{b7} p primary \u{b7} b path \u{b7} d delete \u{b7} r reload";

/// Browse's keys with nothing read: only the two that do not need a tree.
const HINT_NO_WORKSPACE: &str = "N workspace · r reload";

/// Browse's keys with the read refused: nothing here can be created against a store that did not
/// answer, so the only offer is to ask again.
const HINT_UNAVAILABLE: &str = "r reload";

/// An open editor's keys.
const HINT_EDITING: &str = "Tab/Shift+Tab field · Enter save · Esc cancel";

/// What a CAS miss says while an editor is open (D7, PRD D8): the text is kept, the token is not,
/// and the retry is the user's.
const CHANGED_ELSEWHERE: &str =
    "changed elsewhere since you opened it — reloaded; Enter retries against the current row";

/// What a CAS miss says when the row the editor opened on is gone from the reloaded tree.
const DELETED_ELSEWHERE: &str = "deleted elsewhere while you were editing";

/// What a CAS miss says with no editor open — `p` is the one write that has none (D12).
const RELOADED: &str = "reloaded; press p again";

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
/// `UpdateRepo` and `SetRepoPath` carry the repo's **project** because the seam has no `repo(id)`
/// reader: the worker re-reads the tree the reply renders, and it needs the project to find the
/// workspace (plan open item O-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorKind {
    /// `N`: a workspace the shell then enters (D11).
    NewWorkspace,
    /// `e` on the workspace row.
    EditWorkspace(WorkspaceId),
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
    /// `b` on the workspace row.
    WorkspaceRoot(WorkspaceId),
    /// `b` on a repo row.
    RepoPath {
        /// The repo's project.
        project: ProjectId,
        /// The repo.
        id: RepoId,
    },
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
    /// Index into `fields`.
    focus: usize,
    /// The `updated_at` of the row this opened on, for the three `Edit*` kinds (M1 D3).
    expected: Option<DateTime<Utc>>,
}

/// What the section is doing. `Browse` is not a mode in the modal sense: it captures nothing.
#[derive(Debug, Default)]
enum Mode {
    /// The rows, the cursor and the tab's own `h`/`l`.
    #[default]
    Browse,
    /// One row being typed into.
    Editing(Editor),
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
    /// The last outcome, one line on the hint row.
    notice: Option<String>,
}

impl HierarchySection {
    /// Identity of the hierarchy section.
    pub const ID: SectionId = SectionId("hierarchy");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// One line per row, in [`rows`](HierarchySection::rows) order (D13).
    fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        self.rows()
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let text = match row {
                    Row::Workspace => format!(
                        "{} ({}) — root on this box: {}",
                        snapshot.workspace.name,
                        snapshot.workspace.slug,
                        snapshot
                            .root_path
                            .as_ref()
                            .map_or("unset", |path| path.root_path.as_str())
                    ),
                    Row::Project { index } => snapshot
                        .projects
                        .get(index)
                        .map_or_else(String::new, |entry| {
                            format!("  {}  {}", entry.project.slug, entry.project.name)
                        }),
                    Row::Repo { project, index } => snapshot
                        .projects
                        .get(project)
                        .and_then(|entry| entry.repos.get(index))
                        .map_or_else(String::new, |entry| {
                            format!(
                                "    {}{}  {}  {}  {}",
                                if entry.repo.is_primary { "*" } else { " " },
                                entry.repo.name,
                                entry.repo.default_branch,
                                entry.repo.remote_url.as_deref().unwrap_or("—"),
                                entry
                                    .local_path
                                    .as_ref()
                                    .map_or("unset", |path| path.local_path.as_str())
                            )
                        }),
                };
                let style = if index == self.cursor {
                    theme.accent
                } else {
                    theme.base
                };
                Line::styled(text, style)
            })
            .collect()
    }

    /// The pane under the rows: the open editor, or nothing at all in Browse.
    ///
    /// Takes the width because the delete warning is a sentence rather than a row and has to wrap
    /// inside the pane it is measured for (the layout below needs the height first).
    fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        match &self.mode {
            Mode::Browse => Vec::new(),
            Mode::Editing(editor) => editor.lines(width, theme),
        }
    }

    /// The one line under the pane: the keys this mode binds, then the last outcome.
    ///
    /// Two spans rather than one string: a CAS miss is reported here and D7 asks for it in
    /// `theme.error`, because "someone else wrote to this row" is the one notice a user has to act
    /// on rather than read.
    fn hint(&self, theme: &Theme) -> Line<'static> {
        let keys = self.hint_text();
        let Some(notice) = &self.notice else {
            return Line::styled(keys, theme.dim);
        };
        let style = if is_error(notice) {
            theme.error
        } else {
            theme.dim
        };
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(notice.clone(), style),
        ])
    }

    /// The keys half of the hint line, plus what a write in flight adds to it.
    fn hint_text(&self) -> String {
        let keys = match &self.mode {
            Mode::Browse => {
                if self.unavailable.is_some() {
                    HINT_UNAVAILABLE
                } else if self.snapshot.is_none() {
                    HINT_NO_WORKSPACE
                } else {
                    HINT_BROWSE
                }
            }
            Mode::Editing(_) => HINT_EDITING,
        };
        match self.busy {
            // Only in Browse: an editor's own hint says what `Enter` is for, and a write in flight
            // is why `Enter` is not answering.
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {busy} in flight")
            }
            _ => keys.to_owned(),
        }
    }

    /// Whether a key that opens an editor is refused right now, with the notice that says why.
    ///
    /// Two refusals, in this order: a write in flight (a second one of the same kind would lose a
    /// reply), then a tree that is not there to edit. `r` is deliberately **not** on this path —
    /// re-reading is how a section that lost a reply recovers.
    fn refuse(&mut self, key: char) -> bool {
        if let Some(busy) = self.busy {
            self.notice = Some(format!("`{busy}` is still in flight"));
            return true;
        }
        if self.unavailable.is_some() || (self.snapshot.is_none() && key != 'N') {
            self.notice = Some(NO_WORKSPACE.to_owned());
            return true;
        }
        false
    }

    /// Opens an editor, clearing whatever the last one said.
    fn open(&mut self, kind: EditorKind, fields: Vec<Field>, expected: Option<DateTime<Utc>>) {
        self.notice = None;
        self.mode = Mode::Editing(Editor {
            kind,
            fields,
            focus: 0,
            expected,
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
                        Field::optional("remote_url", ""),
                        Field::required("default_branch", "main"),
                        Field::required("primary (y/n)", primary),
                    ],
                    None,
                );
            }
        }
    }

    /// `e`: the row under the cursor, prefilled, with its `updated_at` as the CAS token.
    fn open_edit(&mut self, row: Row) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match row {
            Row::Workspace => {
                let workspace = &snapshot.workspace;
                let (kind, expected) = (
                    EditorKind::EditWorkspace(workspace.id),
                    Some(workspace.updated_at),
                );
                let fields = vec![
                    Field::required("slug", &workspace.slug),
                    Field::required("name", &workspace.name),
                    Field::optional("description", &workspace.description),
                ];
                self.open(kind, fields, expected);
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
                    Field::optional("remote_url", repo.repo.remote_url.as_deref().unwrap_or("")),
                    Field::required("default_branch", &repo.repo.default_branch),
                ];
                self.open(
                    EditorKind::EditRepo {
                        project: owner.project.id,
                        id: repo.repo.id,
                    },
                    fields,
                    Some(repo.repo.updated_at),
                );
            }
        }
    }

    /// `b`: this box's path for the workspace or for a repo, prefilled with what is stored.
    fn open_path(&mut self, row: Row) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match row {
            Row::Workspace => {
                let stored = snapshot
                    .root_path
                    .as_ref()
                    .map_or("", |path| path.root_path.as_str());
                let fields = vec![Field::required("path", stored)];
                self.open(
                    EditorKind::WorkspaceRoot(snapshot.workspace.id),
                    fields,
                    None,
                );
            }
            Row::Project { .. } => {
                self.notice = Some("`b` wants the workspace or a repo row".to_owned());
            }
            Row::Repo { project, index } => {
                let Some((owner, repo)) = snapshot
                    .projects
                    .get(project)
                    .and_then(|owner| repo_at(owner, index))
                else {
                    return;
                };
                let stored = repo
                    .local_path
                    .as_ref()
                    .map_or("", |path| path.local_path.as_str());
                let fields = vec![Field::required("path", stored)];
                self.open(
                    EditorKind::RepoPath {
                        project: owner,
                        id: repo.repo.id,
                    },
                    fields,
                    None,
                );
            }
        }
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
                self.notice = Some("`p` wants a repo row".to_owned());
            }
        }
    }

    /// Sends one write and remembers its name until the reply.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// One key while an editor is open (D13).
    ///
    /// The focused field answers first, so `l`, `q` and the digits are letters here; what it passes
    /// on is the form's own navigation, and everything left over is swallowed rather than offered
    /// to the shell — with `CONTROL` chords excepted, so `ctrl-c` still quits.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Editing(editor) => match editor.fields.get_mut(editor.focus) {
                Some(field) => field.input.on_key(key),
                None => FieldOutcome::Pass,
            },
            Mode::Browse => return Handled::Pass,
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
                let Mode::Editing(editor) = &mut self.mode else {
                    return Handled::Pass;
                };
                let len = editor.fields.len().max(1);
                match key.code {
                    KeyCode::Tab | KeyCode::Down => {
                        editor.focus = (editor.focus + 1) % len;
                        Handled::Consumed
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        editor.focus = (editor.focus + len - 1) % len;
                        Handled::Consumed
                    }
                    _ if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
                    _ => Handled::Consumed,
                }
            }
        }
    }

    /// `Enter` in an editor: the required fields, then one request per [`EditorKind`].
    ///
    /// The editor **stays open** until the reply lands, so a refusal (a duplicate slug, a refused
    /// path) leaves the text where it was and a second `Enter` retries it.
    fn submit(&mut self, ctx: &mut Ctx<'_>) {
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
            EditorKind::NewWorkspace => StoreRequest::CreateWorkspace {
                slug: editor.text(0),
                name: editor.text(1),
                description: editor.text(2),
            },
            EditorKind::EditWorkspace(id) => {
                let Some(expected) = expected else {
                    self.notice = Some(DELETED_ELSEWHERE.to_owned());
                    return;
                };
                StoreRequest::UpdateWorkspace {
                    id,
                    expected,
                    patch: WorkspacePatch {
                        slug: Some(editor.text(0)),
                        name: Some(editor.text(1)),
                        description: Some(editor.text(2)),
                    },
                }
            }
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
                    },
                }
            }
            EditorKind::NewRepo(project) => {
                let Some(is_primary) = yes_or_no(&editor.text(3)) else {
                    self.notice = Some("`primary (y/n)` is y or n".to_owned());
                    return;
                };
                StoreRequest::CreateRepo {
                    project,
                    name: editor.text(0),
                    remote_url: some_text(editor.text(1)),
                    default_branch: editor.text(2),
                    is_primary,
                }
            }
            EditorKind::EditRepo { project, id } => {
                let Some(expected) = expected else {
                    self.notice = Some(DELETED_ELSEWHERE.to_owned());
                    return;
                };
                StoreRequest::UpdateRepo {
                    project,
                    id,
                    expected,
                    patch: RepoPatch {
                        name: Some(editor.text(0)),
                        remote_url: Some(some_text(editor.text(1))),
                        default_branch: Some(editor.text(2)),
                        // Never the flag: `p` is its only writer (D12).
                        is_primary: None,
                    },
                }
            }
            EditorKind::WorkspaceRoot(id) => StoreRequest::SetWorkspaceRoot {
                id,
                path: editor.text(0),
            },
            EditorKind::RepoPath { project, id } => StoreRequest::SetRepoPath {
                project,
                repo: id,
                path: editor.text(0),
            },
        };
        self.notice = None;
        self.send(request, ctx);
    }

    /// A fresh tree: rows, cursor, whatever the write that asked for it owes the user, and the
    /// scope follow of D11.
    fn on_tree(&mut self, snapshot: &HierarchySnapshot, ctx: &Ctx<'_>) {
        let write = self.busy.take();
        self.unavailable = None;
        if let Some(name) = write {
            self.notice = self.written(name, snapshot);
            self.mode = Mode::Browse;
        }
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();

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
    }

    /// What a write that applied has to say, if anything.
    ///
    /// Only the two path writes do: the guard canonicalises what was typed (D8), and a path stored
    /// under a name the user did not type is a surprise worth one line.
    fn written(&self, request: &'static str, snapshot: &HierarchySnapshot) -> Option<String> {
        let Mode::Editing(editor) = &self.mode else {
            return None;
        };
        let stored = match editor.kind {
            EditorKind::WorkspaceRoot(_) if request == "set_workspace_root" => snapshot
                .root_path
                .as_ref()
                .map(|path| path.root_path.clone())?,
            EditorKind::RepoPath { id, .. } if request == "set_repo_path" => snapshot
                .projects
                .iter()
                .flat_map(|entry| entry.repos.iter())
                .find(|repo| repo.repo.id == id)
                .and_then(|repo| repo.local_path.as_ref())
                .map(|path| path.local_path.clone())?,
            _ => return None,
        };
        (stored != editor.text(0)).then(|| format!("stored as `{stored}`"))
    }

    /// A CAS miss (D7): the tree is replaced, the editor keeps its text and takes the current row's
    /// token, and the retry is a second `Enter` rather than an automatic write.
    fn on_stale(&mut self, snapshot: &HierarchySnapshot) {
        self.busy = None;
        let reloaded = match &self.mode {
            Mode::Editing(editor) => Some(reload(snapshot, editor.kind)),
            Mode::Browse => None,
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        match reloaded {
            // No editor: `p` is the only write that gets here, and its retry is the key again.
            None => self.notice = Some(RELOADED.to_owned()),
            Some(Reload::Gone) => {
                self.mode = Mode::Browse;
                self.notice = Some(DELETED_ELSEWHERE.to_owned());
            }
            Some(Reload::Token(token)) => {
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.expected = Some(token);
                }
                self.notice = Some(CHANGED_ELSEWHERE.to_owned());
            }
            Some(Reload::Keep) => self.notice = Some(CHANGED_ELSEWHERE.to_owned()),
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
        vec![StoreRequest::Hierarchy(scope.workspace_id)]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // An editor open across a workspace switch is dropped on purpose (D5): its CAS token
        // belongs to the other workspace. The notice survives, because the scope change is often
        // the *consequence* of what it is reporting.
        self.snapshot = None;
        self.mode = Mode::Browse;
        self.busy = None;
        self.cursor = 0;
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
        // Browse. `j`, `k`, `N`, `n`, `e`, `p`, `b`, `d` and `r` are free: the global table binds
        // `q`, `?`, the digits, `ctrl-c` and `-`, and the tab consumes `h`/`l`/`[`/`]`/arrows
        // before a section is offered the key.
        match key.code {
            KeyCode::Char('j') => {
                self.move_cursor(true);
                Handled::Consumed
            }
            KeyCode::Char('k') => {
                self.move_cursor(false);
                Handled::Consumed
            }
            KeyCode::Char('N') => {
                if !self.refuse('N') {
                    self.open(
                        EditorKind::NewWorkspace,
                        vec![
                            Field::required("slug", ""),
                            Field::required("name", ""),
                            Field::optional("description", ""),
                        ],
                        None,
                    );
                }
                Handled::Consumed
            }
            KeyCode::Char('n') => {
                if !self.refuse('n')
                    && let Some(row) = self.selected()
                {
                    self.open_new_child(row);
                }
                Handled::Consumed
            }
            KeyCode::Char('e') => {
                if !self.refuse('e')
                    && let Some(row) = self.selected()
                {
                    self.open_edit(row);
                }
                Handled::Consumed
            }
            KeyCode::Char('p') => {
                if !self.refuse('p')
                    && let Some(row) = self.selected()
                {
                    self.move_primary(row, ctx);
                }
                Handled::Consumed
            }
            KeyCode::Char('b') => {
                if !self.refuse('b')
                    && let Some(row) = self.selected()
                {
                    self.open_path(row);
                }
                Handled::Consumed
            }
            // Allowed while `busy`: re-reading is how a section that lost a reply recovers, and a
            // read cannot lose a write's reply — the staleness index is keyed by request kind.
            KeyCode::Char('r') => {
                ctx.request(StoreRequest::Hierarchy(ctx.scope.workspace_id));
                Handled::Consumed
            }
            // Only when there is something to clear: a section that swallowed every `Esc` would
            // take the one the shell uses to close an overlay over it.
            KeyCode::Esc if self.notice.is_some() => {
                self.notice = None;
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Hierarchy(Some(snapshot)) => self.on_tree(snapshot, ctx),
            StoreReply::Hierarchy(None) => {
                self.busy = None;
                self.snapshot = None;
                self.mode = Mode::Browse;
                self.clamp_cursor();
            }
            StoreReply::HierarchyStale(snapshot) => self.on_stale(snapshot),
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
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let pane = self.pane(area.width, ctx.theme);
        let [rows, pane_area, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        if self.unavailable.is_some() {
            message(frame, rows, UNAVAILABLE, ctx.theme);
        } else if self.snapshot.is_none() {
            message(frame, rows, NO_WORKSPACE, ctx.theme);
        } else {
            frame.render_widget(Paragraph::new(self.lines(ctx.theme)), rows);
        }
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), pane_area);
        }
        frame.render_widget(Paragraph::new(self.hint(ctx.theme)), hint);
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

    /// One line per field, the focused label accented and the focused field carrying the cursor.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let label_width = self
            .fields
            .iter()
            .map(|field| field.label.chars().count())
            .max()
            .unwrap_or(0);
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let focused = index == self.focus;
                let padding = " ".repeat(label_width - field.label.chars().count());
                let style = if focused { theme.accent } else { theme.dim };
                let mut spans = vec![Span::styled(format!("{}{padding}: ", field.label), style)];
                let room = usize::from(width).saturating_sub(label_width + 2);
                spans.extend(
                    field
                        .input
                        .line(u16::try_from(room).unwrap_or(u16::MAX), focused, theme)
                        .spans,
                );
                Line::from(spans)
            })
            .collect()
    }
}

/// What a reloaded tree does to an open editor's CAS token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reload {
    /// The editor's row is not in the reloaded tree.
    Gone,
    /// The row's `updated_at` as it is now.
    Token(DateTime<Utc>),
    /// The editor has no CAS token to refresh (a create, or a path write).
    Keep,
}

/// The token an open editor should retry against after a reload.
fn reload(snapshot: &HierarchySnapshot, kind: EditorKind) -> Reload {
    match kind {
        EditorKind::NewWorkspace | EditorKind::NewProject => Reload::Keep,
        EditorKind::EditWorkspace(id) | EditorKind::WorkspaceRoot(id) => {
            if snapshot.workspace.id == id {
                Reload::Token(snapshot.workspace.updated_at)
            } else {
                Reload::Gone
            }
        }
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
        EditorKind::EditRepo { id, .. } | EditorKind::RepoPath { id, .. } => snapshot
            .projects
            .iter()
            .flat_map(|entry| entry.repos.iter())
            .find(|entry| entry.repo.id == id)
            .map_or(Reload::Gone, |entry| Reload::Token(entry.repo.updated_at)),
    }
}

/// The repo at `index` of a project entry, with the project it belongs to.
fn repo_at(owner: &ProjectEntry, index: usize) -> Option<(ProjectId, &RepoEntry)> {
    owner.repos.get(index).map(|repo| (owner.project.id, repo))
}

/// Whether a notice is one the user has to act on rather than read (D7): both come from a row that
/// moved under an open editor.
fn is_error(notice: &str) -> bool {
    notice == CHANGED_ELSEWHERE || notice == DELETED_ELSEWHERE
}

/// An optional column: an empty field clears it rather than storing `""`.
fn some_text(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

/// The `y`/`n` field of the repo editor; anything else is refused rather than guessed at.
fn yes_or_no(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}
