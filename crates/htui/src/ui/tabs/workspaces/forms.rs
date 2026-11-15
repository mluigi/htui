//! The Workspaces tab's two creation forms (MOD-116 M3 D2-D7, `R-TUI-14`): a new workspace (name,
//! slug, folder, description) and a new project in a workspace (name, slug, description, where its
//! repository comes from, and the folder or the URL), each with a preview of what it writes.
//!
//! In both the slug follows the name until the slug field is edited (D2, [`slug_of`]); the
//! workspace's folder follows the slug until the folder field is edited; a project's
//! destination (`<workspace folder>/projects/<slug>`) always follows its slug and is not a field.
//!
//! Paths are shown under `~` and sent absolute (D3): [`shown`] writes a path under the tab's home
//! as `~/…`, [`expanded`] turns a typed leading `~` back into it, and a path that is neither
//! absolute nor under `~` is refused on the form. The forms read no disk (`R-NF-3`): the worker
//! decides what a folder is.
//!
//! Keys go through [`views::WORKSPACES_FORM`]: the focused text field first (so `q`, `l` and the
//! digits are letters), `h`/`l` on the source choice, `Tab`/`Down` and `Shift+Tab`/`Up` move
//! ([`Act::FormNextField`]/[`Act::FormPrevField`]), `Enter` anywhere submits and `Esc` closes. While
//! the request is out (`busy`) every key but `Esc` is swallowed.

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use htui_core::model::{WORKSPACES_DIR, WorkspaceId, project_folder, slug_of, workspace_folder};
use htui_orch::infer::redact_remote;
use ratatui::style::Style;
use ratatui::text::Line;

use crate::app::Ctx;
use crate::keys::translate::FieldEvent;
use crate::keys::{Act, KeyChord, Keys, views};
use crate::store_worker::StoreRequest;
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::kit::text::wrapped;
use crate::ui::{FieldOutcome, TextField, Theme};
use crate::workspace_setup::ProjectSource;

/// What a relative typed folder is told (D3).
pub(super) const NOT_ABSOLUTE: &str = "the folder must be an absolute path or start with ~";

/// The workspace preview's folder line.
const FOLDER_NOTE: &str = "created now; an existing folder is used as it is";

/// The workspace preview's `projects/` line.
const PROJECTS_NOTE: &str = "projects cloned from a URL or started here";

/// The workspace preview's components line.
const COMPONENTS_NOTE: &str =
    "the compose and env files land here once components arrive (MOD-117)";

/// The workspace preview's last line (PRD Q4).
const STARTS_STOPPED: &str = "A new workspace starts stopped.";

/// The workspace preview with no folder to show.
const TYPE_A_FOLDER: &str = "type the folder: an absolute path, or one under ~";

/// What a project's destination reads while the workspace has no folder on this box, or the
/// slug is not one plain name.
const NO_DESTINATION: &str = "<workspace folder>/projects/<slug>";

/// `path` as the tab shows it (D3): `~` or `~/…` under `home`, else as it is.
pub(super) fn shown(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `typed` as an absolute path (D3): `~` and `~/x` under `home`, an absolute path as it is;
/// `None` for anything else (a relative path, `~` with no home, nothing typed).
pub(super) fn expanded(typed: &str, home: Option<&Path>) -> Option<PathBuf> {
    let typed = typed.trim();
    if typed == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = typed.strip_prefix("~/") {
        return home.map(|home| home.join(rest));
    }
    let path = PathBuf::from(typed);
    (!typed.is_empty() && path.is_absolute()).then_some(path)
}

/// What one key did to a form.
#[derive(Debug, Clone)]
pub(super) enum FormOutcome {
    /// The form took it; nothing for the tab to do.
    Stay,
    /// Send this; the form is busy until its reply.
    Submit(Box<StoreRequest>),
    /// The form refused to submit; it says why itself, under the fields.
    Refused,
    /// `Esc`: close the form.
    Cancel,
    /// A chord the modal layer lets through (`ctrl-c`, `F1`): the shell's.
    Pass,
}

/// The new-workspace form (D4).
#[derive(Debug)]
pub(super) struct WorkspaceForm {
    /// `workspace.name`.
    name: TextField,
    /// `workspace.slug`; follows the name until edited.
    slug: TextField,
    /// The folder as shown (`~/…`); follows the slug until edited.
    folder: TextField,
    /// `workspace.description`.
    description: TextField,
    /// Index into name, slug, folder, description.
    focus: usize,
    /// Whether the slug was edited: it no longer follows the name.
    slug_edited: bool,
    /// Whether the folder was edited: it no longer follows the slug.
    folder_edited: bool,
    /// The tab's home directory, for `~`.
    home: Option<PathBuf>,
    /// Whole seconds since the request went out; `None` while none is.
    busy: Option<u64>,
    /// The last refusal, under the form.
    error: Option<String>,
}

/// The workspace form's fields in focus order.
const WORKSPACE_FIELDS: usize = 4;

impl WorkspaceForm {
    /// An empty form over `home`.
    pub(super) fn new(home: Option<&Path>) -> Self {
        Self {
            name: TextField::new(),
            slug: TextField::new(),
            folder: TextField::new(),
            description: TextField::new(),
            focus: 0,
            slug_edited: false,
            folder_edited: false,
            home: home.map(Path::to_path_buf),
            busy: None,
            error: None,
        }
    }

    /// The focused text field.
    const fn field_mut(&mut self) -> &mut TextField {
        match self.focus {
            0 => &mut self.name,
            1 => &mut self.slug,
            2 => &mut self.folder,
            _ => &mut self.description,
        }
    }

    /// One key (`key` as delivered, `pressed` as the user pressed it).
    pub(super) fn on_key(&mut self, key: KeyEvent, pressed: KeyChord, keys: &Keys) -> FormOutcome {
        if self.busy.is_some() {
            return busy_key(&key, pressed);
        }
        let before = text(self.field_mut()).to_owned();
        match self.field_mut().on_key(key) {
            FieldOutcome::Consumed => {
                if text(self.field_mut()) != before {
                    self.edited();
                }
                FormOutcome::Stay
            }
            FieldOutcome::Submit => self.submit(),
            FieldOutcome::Cancel => FormOutcome::Cancel,
            FieldOutcome::Pass => move_focus(&mut self.focus, WORKSPACE_FIELDS, pressed, keys),
        }
    }

    /// A bracketed paste into the focused field; dropped while busy.
    pub(super) fn on_paste(&mut self, pasted: &str) {
        if self.busy.is_some() {
            return;
        }
        let before = text(self.field_mut()).to_owned();
        let _ = self.field_mut().on_paste(pasted);
        if text(self.field_mut()) != before {
            self.edited();
        }
    }

    /// The focused field changed: what follows it is derived again (D2).
    fn edited(&mut self) {
        self.error = None;
        match self.focus {
            0 if !self.slug_edited => self.slug = TextField::with_text(&slug_of(text(&self.name))),
            1 => self.slug_edited = true,
            2 => self.folder_edited = true,
            _ => return,
        }
        if !self.folder_edited {
            self.folder = TextField::with_text(&self.default_folder());
        }
    }

    /// `~/workspaces/<slug>`, or `""` with no home or a slug that is not one plain name.
    fn default_folder(&self) -> String {
        self.home
            .as_deref()
            .and_then(|home| workspace_folder(text(&self.slug).trim(), None, home))
            .map_or_else(String::new, |folder| shown(&folder, self.home.as_deref()))
    }

    /// `Enter`: name, slug and folder required, the folder absolute or under `~`.
    fn submit(&mut self) -> FormOutcome {
        let missing = [
            ("name", &self.name),
            ("slug", &self.slug),
            ("folder", &self.folder),
        ]
        .into_iter()
        .find(|(_, field)| text(field).trim().is_empty())
        .map(|(label, _)| format!("`{label}` is required"));
        if let Some(missing) = missing {
            return self.refuse(missing);
        }
        let Some(folder) = expanded(text(&self.folder), self.home.as_deref()) else {
            return self.refuse(NOT_ABSOLUTE.to_owned());
        };
        self.error = None;
        self.busy = Some(0);
        FormOutcome::Submit(Box::new(StoreRequest::NewWorkspace {
            slug: text(&self.slug).trim().to_owned(),
            name: text(&self.name).trim().to_owned(),
            description: text(&self.description).to_owned(),
            folder: folder.display().to_string(),
        }))
    }

    /// Says `why` under the form.
    fn refuse(&mut self, why: String) -> FormOutcome {
        self.error = Some(why);
        FormOutcome::Refused
    }

    /// The reply landed refused: the text stays, `Enter` retries (D13).
    pub(super) fn refused(&mut self, why: &str) {
        self.busy = None;
        self.error = Some(why.to_owned());
    }

    /// Whether the request is out.
    pub(super) const fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    /// One second more while busy (the tab's `on_refresh`).
    pub(super) fn tick(&mut self) {
        if let Some(seconds) = &mut self.busy {
            *seconds += 1;
        }
    }

    /// The form, its error or busy line, then the preview (dim).
    pub(super) fn lines(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let form = Form::new()
            .field(Field::new("name", Value::Text(&self.name)))
            .field(Field::new("slug", Value::Text(&self.slug)))
            .field(
                Field::new("folder", Value::Text(&self.folder))
                    .help("an absolute path, or one under ~"),
            )
            .field(Field::new("description", Value::Text(&self.description)))
            .focus(self.focus)
            .editing(self.busy.is_none());
        let busy = self
            .busy
            .map(|seconds| counted("creating the workspace\u{2026}".to_owned(), seconds));
        assemble(
            form.lines(width, ctx.accent(), ctx.theme),
            self.error.as_deref(),
            busy,
            &self.preview(),
            width,
            ctx.theme,
        )
    }

    /// The preview's lines, as a small tree (leading blanks indent): the folder and its
    /// `projects/` each with what it is, the components line, then the state a new workspace
    /// starts in.
    fn preview(&self) -> Vec<String> {
        let folder = text(&self.folder).trim().trim_end_matches('/');
        let mut lines = Vec::new();
        if folder.is_empty() {
            // Nothing to follow yet: say where the folder goes once there is a slug.
            let follows = self
                .home
                .as_deref()
                .filter(|_| !self.folder_edited)
                .map(|home| shown(&home.join(WORKSPACES_DIR), Some(home)));
            lines.push(follows.map_or_else(
                || TYPE_A_FOLDER.to_owned(),
                |base| format!("the folder follows the slug: {base}/<slug>"),
            ));
        } else {
            lines.push(format!("{folder}/"));
            lines.push(format!("  {FOLDER_NOTE}"));
            lines.push(format!("  {}/", htui_core::model::PROJECTS_DIR));
            lines.push(format!("    {PROJECTS_NOTE}"));
            lines.push(COMPONENTS_NOTE.to_owned());
        }
        lines.push(String::new());
        lines.push(STARTS_STOPPED.to_owned());
        lines
    }

    /// The slug typed, for tests.
    #[cfg(test)]
    fn slug(&self) -> &str {
        text(&self.slug)
    }

    /// The folder typed, for tests.
    #[cfg(test)]
    fn folder(&self) -> &str {
        text(&self.folder)
    }
}

/// Where a new project's repository comes from (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceKind {
    /// A checkout on this box, registered where it is.
    Existing,
    /// A git URL, cloned into the workspace folder.
    Url,
    /// A new repository with one empty commit.
    NewRepo,
}

impl SourceKind {
    /// Every source, in the choice's order.
    const ALL: [Self; 3] = [Self::Existing, Self::Url, Self::NewRepo];

    /// The choice's word.
    const fn word(self) -> &'static str {
        match self {
            Self::Existing => "existing folder",
            Self::Url => "git URL",
            Self::NewRepo => "new repo",
        }
    }

    /// The next (or previous) source, with wrap.
    fn step(self, forward: bool) -> Self {
        let at = Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        }]
    }
}

/// The project form's fields, in focus order (`Location` absent for a new repo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectField {
    /// `project.name`.
    Name,
    /// `project.slug`.
    Slug,
    /// `project.description`.
    Description,
    /// The source choice.
    Source,
    /// The folder path or the URL.
    Location,
}

/// The new-project form (D6).
#[derive(Debug)]
pub(super) struct ProjectForm {
    /// The workspace the project joins.
    workspace: WorkspaceId,
    /// Its name, for the title.
    workspace_name: String,
    /// Its folder on this box, when the tab holds it (the overview's).
    workspace_folder: Option<PathBuf>,
    /// `project.name`.
    name: TextField,
    /// `project.slug`; follows the name until edited.
    slug: TextField,
    /// `project.description`.
    description: TextField,
    /// Where the repository comes from.
    source: SourceKind,
    /// The folder path (`Existing`) or the URL (`Url`).
    location: TextField,
    /// Index into [`fields`](Self::fields).
    focus: usize,
    /// Whether the slug was edited: it no longer follows the name.
    slug_edited: bool,
    /// The tab's home directory, for `~`.
    home: Option<PathBuf>,
    /// Whole seconds since the request went out; `None` while none is.
    busy: Option<u64>,
    /// The last refusal, under the form.
    error: Option<String>,
}

impl ProjectForm {
    /// An empty form for a project in `workspace` (`name`), whose folder is `folder` when known.
    pub(super) fn new(
        workspace: WorkspaceId,
        name: &str,
        folder: Option<PathBuf>,
        home: Option<&Path>,
    ) -> Self {
        Self {
            workspace,
            workspace_name: name.to_owned(),
            workspace_folder: folder,
            name: TextField::new(),
            slug: TextField::new(),
            description: TextField::new(),
            source: SourceKind::Existing,
            location: TextField::new(),
            focus: 0,
            slug_edited: false,
            home: home.map(Path::to_path_buf),
            busy: None,
            error: None,
        }
    }

    /// The workspace the project joins.
    pub(super) const fn workspace(&self) -> WorkspaceId {
        self.workspace
    }

    /// The workspace's name.
    pub(super) fn workspace_name(&self) -> &str {
        &self.workspace_name
    }

    /// The slug typed.
    pub(super) fn slug(&self) -> &str {
        text(&self.slug).trim()
    }

    /// The fields in focus order.
    fn fields(&self) -> &'static [ProjectField] {
        use ProjectField::{Description, Location, Name, Slug, Source};
        if self.source == SourceKind::NewRepo {
            &[Name, Slug, Description, Source]
        } else {
            &[Name, Slug, Description, Source, Location]
        }
    }

    /// The focused field.
    fn focused(&self) -> ProjectField {
        self.fields()
            .get(self.focus)
            .copied()
            .unwrap_or(ProjectField::Name)
    }

    /// The focused text field; `None` on the source choice.
    const fn text_field(&mut self, field: ProjectField) -> Option<&mut TextField> {
        match field {
            ProjectField::Name => Some(&mut self.name),
            ProjectField::Slug => Some(&mut self.slug),
            ProjectField::Description => Some(&mut self.description),
            ProjectField::Location => Some(&mut self.location),
            ProjectField::Source => None,
        }
    }

    /// One key (`key` as delivered, `pressed` as the user pressed it).
    pub(super) fn on_key(&mut self, key: KeyEvent, pressed: KeyChord, keys: &Keys) -> FormOutcome {
        if self.busy.is_some() {
            return busy_key(&key, pressed);
        }
        let focused = self.focused();
        let len = self.fields().len();
        let Some(field) = self.text_field(focused) else {
            // The source choice: `Enter` submits, `Esc` closes, `h`/`l` cycle it.
            return match FieldEvent::of(&key) {
                Some(FieldEvent::Submit) => self.submit(),
                Some(FieldEvent::Cancel) => FormOutcome::Cancel,
                _ => {
                    let acts = keys.actions(views::WORKSPACES_FORM, pressed);
                    match acts.iter().find_map(|act| match act {
                        Act::ChoicePrev => Some(false),
                        Act::ChoiceNext => Some(true),
                        _ => None,
                    }) {
                        Some(forward) => {
                            self.source = self.source.step(forward);
                            self.error = None;
                            FormOutcome::Stay
                        }
                        None => move_focus(&mut self.focus, len, pressed, keys),
                    }
                }
            };
        };
        let before = text(field).to_owned();
        match field.on_key(key) {
            FieldOutcome::Consumed => {
                if text(field) != before {
                    self.edited(focused);
                }
                FormOutcome::Stay
            }
            FieldOutcome::Submit => self.submit(),
            FieldOutcome::Cancel => FormOutcome::Cancel,
            FieldOutcome::Pass => move_focus(&mut self.focus, len, pressed, keys),
        }
    }

    /// A bracketed paste into the focused text field; dropped on the choice and while busy.
    pub(super) fn on_paste(&mut self, pasted: &str) {
        if self.busy.is_some() {
            return;
        }
        let focused = self.focused();
        let Some(field) = self.text_field(focused) else {
            return;
        };
        let before = text(field).to_owned();
        let _ = field.on_paste(pasted);
        if text(field) != before {
            self.edited(focused);
        }
    }

    /// `field` changed: the slug follows the name until it is edited itself (D2).
    fn edited(&mut self, field: ProjectField) {
        self.error = None;
        match field {
            ProjectField::Name if !self.slug_edited => {
                self.slug = TextField::with_text(&slug_of(text(&self.name)));
            }
            ProjectField::Slug => self.slug_edited = true,
            _ => {}
        }
    }

    /// Where a clone or a new repository goes: `<workspace folder>/projects/<slug>`, when the
    /// workspace's folder is known and the slug is one plain name.
    fn destination(&self) -> Option<PathBuf> {
        self.workspace_folder
            .as_deref()
            .and_then(|folder| project_folder(folder, self.slug()))
    }

    /// [`destination`](Self::destination) as shown.
    fn destination_shown(&self) -> String {
        self.destination().map_or_else(
            || NO_DESTINATION.to_owned(),
            |dest| shown(&dest, self.home.as_deref()),
        )
    }

    /// `Enter`: name and slug required, the folder absolute or under `~`, the URL non-empty.
    fn submit(&mut self) -> FormOutcome {
        let mut required = vec![("name", &self.name), ("slug", &self.slug)];
        match self.source {
            SourceKind::Existing => required.push(("folder", &self.location)),
            SourceKind::Url => required.push(("URL", &self.location)),
            SourceKind::NewRepo => {}
        }
        let missing = required
            .into_iter()
            .find(|(_, field)| text(field).trim().is_empty())
            .map(|(label, _)| format!("`{label}` is required"));
        if let Some(missing) = missing {
            return self.refuse(missing);
        }
        let source = match self.source {
            SourceKind::Existing => match expanded(text(&self.location), self.home.as_deref()) {
                Some(path) => ProjectSource::Existing {
                    path: path.display().to_string(),
                },
                None => return self.refuse(NOT_ABSOLUTE.to_owned()),
            },
            SourceKind::Url => ProjectSource::Clone {
                url: text(&self.location).trim().to_owned(),
            },
            SourceKind::NewRepo => ProjectSource::NewRepo,
        };
        self.error = None;
        self.busy = Some(0);
        FormOutcome::Submit(Box::new(StoreRequest::NewProject {
            workspace: self.workspace,
            slug: self.slug().to_owned(),
            name: text(&self.name).trim().to_owned(),
            description: text(&self.description).to_owned(),
            source,
        }))
    }

    /// Says `why` under the form.
    fn refuse(&mut self, why: String) -> FormOutcome {
        self.error = Some(why);
        FormOutcome::Refused
    }

    /// The reply landed refused: the text stays, `Enter` retries (D13).
    pub(super) fn refused(&mut self, why: &str) {
        self.busy = None;
        self.error = Some(why.to_owned());
    }

    /// Whether the request is out.
    pub(super) const fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    /// One second more while busy (the tab's `on_refresh`).
    pub(super) fn tick(&mut self) {
        if let Some(seconds) = &mut self.busy {
            *seconds += 1;
        }
    }

    /// The form, its error or busy line, then the preview (dim).
    pub(super) fn lines(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let mut form = Form::new()
            .field(Field::new("name", Value::Text(&self.name)))
            .field(Field::new("slug", Value::Text(&self.slug)))
            .field(Field::new("description", Value::Text(&self.description)))
            .field(Field::new("source", Value::Choice(self.source.word())));
        match self.source {
            SourceKind::Existing => {
                form = form.field(
                    Field::new("folder", Value::Text(&self.location))
                        .help("an absolute path, or one under ~"),
                );
            }
            SourceKind::Url => {
                form = form.field(Field::new("URL", Value::Text(&self.location)));
            }
            SourceKind::NewRepo => {}
        }
        let form = form
            .focus(self.focus)
            .editing(self.busy.is_none() && self.focused() != ProjectField::Source);
        let busy = self.busy.map(|seconds| counted(self.busy_line(), seconds));
        assemble(
            form.lines(width, ctx.accent(), ctx.theme),
            self.error.as_deref(),
            busy,
            &self.preview(),
            width,
            ctx.theme,
        )
    }

    /// D7: what the request out is doing.
    fn busy_line(&self) -> String {
        match self.source {
            SourceKind::Existing => {
                let path = expanded(text(&self.location), self.home.as_deref()).map_or_else(
                    || text(&self.location).trim().to_owned(),
                    |path| shown(&path, self.home.as_deref()),
                );
                format!("registering {path}\u{2026}")
            }
            SourceKind::Url => format!(
                "cloning {} into {}\u{2026}",
                redact_remote(text(&self.location).trim()),
                self.destination_shown()
            ),
            SourceKind::NewRepo => {
                format!(
                    "creating {} with git init\u{2026}",
                    self.destination_shown()
                )
            }
        }
    }

    /// The preview's sentence for the chosen source (D6).
    fn preview(&self) -> Vec<String> {
        let sentence = match self.source {
            SourceKind::Existing => {
                let typed = text(&self.location).trim();
                let path = if typed.is_empty() {
                    "the folder"
                } else {
                    typed
                };
                format!(
                    "registers {path} as it is; htui writes nothing in it; plain folders come \
                     with MOD-145"
                )
            }
            SourceKind::Url => format!(
                "clones it into {} with this machine's git (its ssh agent and credential helper; \
                 htui never prompts)",
                self.destination_shown()
            ),
            SourceKind::NewRepo => format!(
                "creates {} with git init and one empty commit on main",
                self.destination_shown()
            ),
        };
        vec![sentence]
    }

    /// The name typed, for tests.
    #[cfg(test)]
    fn name(&self) -> &str {
        text(&self.name)
    }
}

/// What a text field holds (the forms mask nothing).
fn text(field: &TextField) -> &str {
    field.text().unwrap_or_default()
}

/// A key while the request is out: `Esc` closes, what the modal layer passes is the shell's,
/// every other key is swallowed (D7).
fn busy_key(key: &KeyEvent, pressed: KeyChord) -> FormOutcome {
    if FieldEvent::of(key) == Some(FieldEvent::Cancel) {
        FormOutcome::Cancel
    } else if views::WORKSPACES_FORM.passes(pressed) {
        FormOutcome::Pass
    } else {
        FormOutcome::Stay
    }
}

/// A key no field took: `form.next_field`/`prev_field` move `focus` over `len` fields; anything
/// else is swallowed unless the modal layer passes it.
fn move_focus(focus: &mut usize, len: usize, pressed: KeyChord, keys: &Keys) -> FormOutcome {
    let stack = views::WORKSPACES_FORM;
    let len = len.max(1);
    for act in keys.actions(stack, pressed) {
        match act {
            Act::FormNextField => *focus = (*focus + 1) % len,
            Act::FormPrevField => *focus = (*focus + len - 1) % len,
            _ => continue,
        }
        return FormOutcome::Stay;
    }
    if stack.passes(pressed) {
        FormOutcome::Pass
    } else {
        FormOutcome::Stay
    }
}

/// `line` with ` · <n> s` once a second has passed (D7).
fn counted(line: String, seconds: u64) -> String {
    if seconds == 0 {
        line
    } else {
        format!("{line} \u{b7} {seconds} s")
    }
}

/// A form's lines: the fields, the error (`theme.error`) or the busy line, a blank, then the
/// preview in `theme.dim`, each line wrapped to `width` under its own leading blanks.
fn assemble(
    mut lines: Vec<Line<'static>>,
    error: Option<&str>,
    busy: Option<String>,
    preview: &[String],
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let mut push = |text: &str, style: Style| {
        let body = text.trim_start_matches(' ');
        if body.is_empty() {
            lines.push(Line::default());
            return;
        }
        let indent = (text.len() - body.len()).min(width / 2);
        for row in wrapped(body, width - indent) {
            lines.push(Line::styled(format!("{}{row}", " ".repeat(indent)), style));
        }
    };
    push("", theme.dim);
    if let Some(error) = error {
        push(error, theme.error);
    }
    if let Some(busy) = &busy {
        push(busy, theme.base);
    }
    if error.is_some() || busy.is_some() {
        push("", theme.dim);
    }
    for line in preview {
        push(line, theme.dim);
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use htui_core::fixtures::ids;

    use super::{
        FormOutcome, NOT_ABSOLUTE, ProjectForm, SourceKind, WorkspaceForm, expanded, shown,
    };
    use crate::keys::{KeyChord, Keys};
    use crate::store_worker::StoreRequest;
    use crate::workspace_setup::ProjectSource;

    const HOME: &str = "/home/u";

    /// `chord` (a `KeyChord::parse` spec) as the shell delivers it.
    fn chord(spec: &str) -> (crossterm::event::KeyEvent, KeyChord) {
        let parsed = KeyChord::parse(spec).unwrap_or_else(|| panic!("`{spec}` is a chord"));
        (parsed.to_event(), parsed)
    }

    /// One chord per character of `text`.
    fn chords(text: &str) -> Vec<String> {
        text.chars()
            .map(|c| {
                if c == ' ' {
                    "space".to_owned()
                } else {
                    c.to_string()
                }
            })
            .collect()
    }

    fn workspace_key(form: &mut WorkspaceForm, spec: &str) -> FormOutcome {
        let (key, pressed) = chord(spec);
        form.on_key(key, pressed, Keys::compiled())
    }

    fn project_key(form: &mut ProjectForm, spec: &str) -> FormOutcome {
        let (key, pressed) = chord(spec);
        form.on_key(key, pressed, Keys::compiled())
    }

    fn type_into_workspace(form: &mut WorkspaceForm, text: &str) {
        for spec in chords(text) {
            assert!(stays(&workspace_key(form, &spec)), "{spec}");
        }
    }

    fn type_into_project(form: &mut ProjectForm, text: &str) {
        for spec in chords(text) {
            assert!(stays(&project_key(form, &spec)), "{spec}");
        }
    }

    fn stays(outcome: &FormOutcome) -> bool {
        matches!(outcome, FormOutcome::Stay)
    }

    /// `Enter` on `form`, refused: what the form says under its fields.
    fn refused(form: &mut WorkspaceForm) -> Option<String> {
        assert!(matches!(workspace_key(form, "enter"), FormOutcome::Refused));
        form.error.clone()
    }

    #[test]
    fn the_slug_and_folder_follow_the_name_until_edited() {
        let mut form = WorkspaceForm::new(Some(Path::new(HOME)));
        type_into_workspace(&mut form, "Mobile App");
        assert_eq!(form.slug(), "mobile-app");
        assert_eq!(form.folder(), "~/workspaces/mobile-app");
        // The slug edited: the folder follows it, the name no longer moves it.
        assert!(stays(&workspace_key(&mut form, "tab")));
        type_into_workspace(&mut form, "2");
        assert_eq!(form.slug(), "mobile-app2");
        assert_eq!(form.folder(), "~/workspaces/mobile-app2");
        assert!(stays(&workspace_key(&mut form, "shift-tab")));
        type_into_workspace(&mut form, "s");
        assert_eq!(form.slug(), "mobile-app2");
        // Q4 and the digits are letters in a field.
        type_into_workspace(&mut form, "q1");
        assert_eq!(form.slug(), "mobile-app2");
    }

    #[test]
    fn an_edited_folder_stops_following() {
        let mut form = WorkspaceForm::new(Some(Path::new(HOME)));
        type_into_workspace(&mut form, "Web");
        assert_eq!(form.folder(), "~/workspaces/web");
        let _ = workspace_key(&mut form, "down");
        let _ = workspace_key(&mut form, "down");
        type_into_workspace(&mut form, "x");
        assert_eq!(form.folder(), "~/workspaces/webx");
        let _ = workspace_key(&mut form, "up");
        let _ = workspace_key(&mut form, "up");
        type_into_workspace(&mut form, " Site");
        assert_eq!(form.slug(), "web-site");
        assert_eq!(
            form.folder(),
            "~/workspaces/webx",
            "the folder no longer follows"
        );
    }

    #[test]
    fn with_no_home_the_folder_is_left_to_type() {
        let mut form = WorkspaceForm::new(None);
        type_into_workspace(&mut form, "Web");
        assert_eq!(form.slug(), "web");
        assert_eq!(form.folder(), "");
        assert!(
            form.preview()[0].contains("type the folder"),
            "{:?}",
            form.preview()
        );
        assert_eq!(refused(&mut form).as_deref(), Some("`folder` is required"));
    }

    #[test]
    fn a_relative_folder_is_refused_on_the_form() {
        let mut form = WorkspaceForm::new(Some(Path::new(HOME)));
        assert_eq!(refused(&mut form).as_deref(), Some("`name` is required"));
        type_into_workspace(&mut form, "Web");
        let _ = workspace_key(&mut form, "tab");
        let _ = workspace_key(&mut form, "tab");
        for _ in 0.."~/workspaces/web".len() {
            let _ = workspace_key(&mut form, "backspace");
        }
        type_into_workspace(&mut form, "rel/web");
        assert_eq!(refused(&mut form).as_deref(), Some(NOT_ABSOLUTE));
        assert_eq!(form.error.as_deref(), Some(NOT_ABSOLUTE));
        assert!(!form.is_busy());
    }

    #[test]
    fn tilde_paths_expand_with_the_tab_s_home() {
        let home = Path::new(HOME);
        assert_eq!(expanded("~", Some(home)), Some(PathBuf::from(HOME)));
        assert_eq!(
            expanded(" ~/workspaces/x ", Some(home)),
            Some(PathBuf::from("/home/u/workspaces/x"))
        );
        assert_eq!(
            expanded("/srv/x", Some(home)),
            Some(PathBuf::from("/srv/x"))
        );
        assert_eq!(expanded("/srv/x", None), Some(PathBuf::from("/srv/x")));
        assert_eq!(expanded("~/x", None), None);
        assert_eq!(expanded("x/y", Some(home)), None);
        assert_eq!(expanded("", Some(home)), None);
        assert_eq!(
            shown(Path::new("/home/u/workspaces/x"), Some(home)),
            "~/workspaces/x"
        );
        assert_eq!(shown(home, Some(home)), "~");
        assert_eq!(shown(Path::new("/srv/x"), Some(home)), "/srv/x");
        assert_eq!(shown(Path::new("/home/u/x"), None), "/home/u/x");

        // The default folder is sent absolute, under the tab's home.
        let mut form = WorkspaceForm::new(Some(home));
        type_into_workspace(&mut form, "Mobile App");
        let FormOutcome::Submit(request) = workspace_key(&mut form, "enter") else {
            panic!("the form submits");
        };
        let StoreRequest::NewWorkspace {
            slug, name, folder, ..
        } = *request
        else {
            panic!("a new workspace");
        };
        assert_eq!(
            (slug.as_str(), name.as_str(), folder.as_str()),
            ("mobile-app", "Mobile App", "/home/u/workspaces/mobile-app")
        );
        assert!(form.is_busy());
        assert!(stays(&workspace_key(&mut form, "x")), "busy swallows");
        assert!(matches!(
            workspace_key(&mut form, "esc"),
            FormOutcome::Cancel
        ));
    }

    #[test]
    fn the_project_location_follows_the_slug() {
        let mut form = ProjectForm::new(
            ids::WORKSPACE_GRAPHICS,
            "Graphics",
            Some(PathBuf::from("/home/u/graphics")),
            Some(Path::new(HOME)),
        );
        type_into_project(&mut form, "Web Site");
        assert_eq!(form.name(), "Web Site");
        assert_eq!(form.slug(), "web-site");
        assert_eq!(form.destination_shown(), "~/graphics/projects/web-site");
        let _ = project_key(&mut form, "tab");
        type_into_project(&mut form, "s");
        assert_eq!(form.destination_shown(), "~/graphics/projects/web-sites");
        // The slug edited, the name no longer moves it.
        let _ = project_key(&mut form, "shift-tab");
        type_into_project(&mut form, "!");
        assert_eq!(form.slug(), "web-sites");

        // A workspace with no known folder still says where.
        let form = ProjectForm::new(ids::WORKSPACE_GRAPHICS, "Graphics", None, None);
        assert_eq!(form.destination_shown(), super::NO_DESTINATION);
    }

    #[test]
    fn each_source_previews_what_it_does() {
        let mut form = ProjectForm::new(
            ids::WORKSPACE_GRAPHICS,
            "Graphics",
            Some(PathBuf::from("/home/u/graphics")),
            Some(Path::new(HOME)),
        );
        type_into_project(&mut form, "App");
        for _ in 0..3 {
            let _ = project_key(&mut form, "tab");
        }
        assert_eq!(form.source, SourceKind::Existing);
        assert_eq!(
            form.preview(),
            [
                "registers the folder as it is; htui writes nothing in it; plain folders come with \
              MOD-145"
            ]
        );
        assert!(stays(&project_key(&mut form, "l")));
        assert_eq!(form.source, SourceKind::Url);
        assert_eq!(
            form.preview(),
            [
                "clones it into ~/graphics/projects/app with this machine's git (its ssh agent and \
              credential helper; htui never prompts)"
            ]
        );
        let _ = project_key(&mut form, "l");
        assert_eq!(form.source, SourceKind::NewRepo);
        assert_eq!(
            form.preview(),
            ["creates ~/graphics/projects/app with git init and one empty commit on main"]
        );
        assert_eq!(form.fields().len(), 4, "a new repo has no location");
        let _ = project_key(&mut form, "h");
        let _ = project_key(&mut form, "h");
        assert_eq!(form.source, SourceKind::Existing);
        let _ = project_key(&mut form, "tab");
        type_into_project(&mut form, "~/src/app");
        assert_eq!(
            form.preview(),
            [
                "registers ~/src/app as it is; htui writes nothing in it; plain folders come with \
              MOD-145"
            ]
        );
        let FormOutcome::Submit(request) = project_key(&mut form, "enter") else {
            panic!("the form submits");
        };
        let StoreRequest::NewProject { source, slug, .. } = *request else {
            panic!("a new project");
        };
        assert_eq!(slug, "app");
        assert_eq!(
            source,
            ProjectSource::Existing {
                path: "/home/u/src/app".to_owned()
            }
        );
        assert_eq!(form.busy_line(), "registering ~/src/app\u{2026}");
    }

    #[test]
    fn a_url_s_busy_line_drops_its_userinfo_and_counts_seconds() {
        let mut form = ProjectForm::new(
            ids::WORKSPACE_GRAPHICS,
            "Graphics",
            Some(PathBuf::from("/home/u/graphics")),
            Some(Path::new(HOME)),
        );
        type_into_project(&mut form, "App");
        for _ in 0..3 {
            let _ = project_key(&mut form, "tab");
        }
        let _ = project_key(&mut form, "l");
        let _ = project_key(&mut form, "tab");
        form.on_paste("https://me:secret@example.com/app.git");
        assert!(matches!(
            project_key(&mut form, "enter"),
            FormOutcome::Submit(request) if matches!(
                *request,
                StoreRequest::NewProject {
                    source: ProjectSource::Clone { .. },
                    ..
                }
            )
        ));
        form.tick();
        form.tick();
        assert_eq!(
            super::counted(form.busy_line(), 2),
            "cloning https://example.com/app.git into ~/graphics/projects/app\u{2026} \u{b7} 2 s"
        );
        form.refused("git clone: fatal");
        assert!(!form.is_busy());
        assert_eq!(form.error.as_deref(), Some("git clone: fatal"));
    }
}
