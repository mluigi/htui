//! The Templates view of the Skills tab (MOD-9 milestone 1; plan D6, D11–D14; blueprint D19, D20,
//! D27): the scope's prompt templates as a tree, any version's body, a line diff between two
//! versions or a version and the compiled default, and an editor that saves through `parse`.
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle
//! and no `UserId`, renders from the last `TemplatesSnapshot` and never patches a row into it.
//! `parse` is the only validator, run on `Ctrl+S` and on an `$EDITOR` return; the store runs it
//! again behind the compare-and-set (plan D4), so a body the view let through cannot land broken.
//!
//! The token a save carries is the head version when the editor opened, not the version shown:
//! editing v1 while v3 is head saves v4 (plan D1, OQ-5).

use core::cell::Cell;

use htui_core::model::{ProjectId, PromptTemplate};
use htui_core::prompt::{Placeholder, TemplateError, TemplateRole, body_of, parse};
use htui_core::store::invalid_template_name;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Action, Ctx, Handled};
use crate::editor::{ExternalEdit, ExternalEditOutcome};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::templates::{READ_NAME, REQUEST_NAMES, TemplateBody, TemplatesSnapshot};
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::settings::wrapped;
use crate::ui::{AreaOutcome, FieldOutcome, TextArea, TextField, Theme, diff};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The save's `StoreRequest::name`, what `busy` holds while it is in flight.
const SAVE_NAME: &str = REQUEST_NAMES[1];

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// The tree's width, borders included: `  {name:<13} v{head:<3} {role}` is at most 28 chars.
const LIST_WIDTH: u16 = 32;

/// The help column's width, borders included: `{{failure_reason}} section required`, the widest
/// placeholder line (a handoff's), is 35 chars.
const HELP_WIDTH: u16 = 38;

/// The pane before the first reply.
const NOT_READ: &str = "templates not read yet";

/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "templates unavailable";

/// A version key on a project header row.
const SELECT_A_TEMPLATE: &str = "select a template";

/// `Ctrl+S` on a phase body that never places `{{item}}` (ANA-5 §4.1: warn, never refuse).
const OMITS_ITEM: &str =
    "this phase template never places {{item}} \u{2014} Ctrl+S again saves anyway";

/// `Esc` over a modified draft, the first time (OQ-6).
const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";

/// A save went out.
const SAVING: &str = "saving\u{2026}";

/// An `$EDITOR` return that `parse` accepts.
const EDITED: &str = "edited in $EDITOR \u{2014} Ctrl+S saves";

/// An `$EDITOR` return that changed nothing.
const NO_CHANGES: &str = "no changes";

/// Appended to [`NO_CHANGES`] when the editor returned within `QUICK_EXIT` (blueprint D24, R-3).
const WAIT_FLAG: &str = " \u{2014} a GUI editor needs its wait flag, e.g. `code --wait`";

/// The hint row in Browse (plan D13).
const BROWSE_HINT: &str = "j/k move  ,/. version  b base  d diff  D default  e edit  E $EDITOR  \
                           n new  r reload  h/l view";

/// The pane's bottom border while its lines overflow it.
const SCROLL_HINT: &str = " J/K PgUp/PgDn scroll ";

/// The hint row while naming a new template.
const NAMING_HINT: &str = "Enter create  Esc cancel";

/// The hint row in the editor, before the cursor's `L{line}:C{col}` (D20).
const EDIT_HINT: &str = "Ctrl+S save  Ctrl+E $EDITOR  Esc cancel";

/// The review phase's wire contract, the one a reviewer's output is parsed by (ANA-5 `:1323-1331`).
const REVIEW_WIRE: &str = "wire: first 3 lines `---` / `verdict: approve|request-changes` / `---`";

/// The judge's wire contract (ANA-5 `:1333-1344`).
const JUDGE_WIRE: &str = "wire: end with one ```json block {winner, reasons}";

/// `TemplatesStale` over an open editor (plan D11). The view's own sentence, not the Settings
/// tab's `CHANGED_ELSEWHERE`: that one names `Enter`, which is not this editor's key.
fn template_changed_elsewhere(head: i32) -> String {
    format!(
        "saved elsewhere since you opened it \u{2014} v{head} is now the latest; your draft is \
         kept and Ctrl+S saves it as v{}",
        head + 1
    )
}

/// The Templates view. Holds no store handle and no `UserId` (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct TemplatesView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<TemplatesSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale tree.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](TemplatesView::rows).
    cursor: usize,
    /// The version shown for the selected name; `None` is the head.
    shown: Option<i32>,
    /// What `d` diffs against, once `b` or `D` chose it; `None` is the shown version's
    /// predecessor.
    base: Option<DiffBase>,
    /// Which pane the Browse layout shows.
    pane: Pane,
    /// The pane's first drawn row (`J`/`K`, `PageDown`/`PageUp`). Back to the top whenever the
    /// pane shows something else: another row, another version, the other pane.
    scroll: Scroll,
    /// The pane's rows at the last draw, what [`scroll`](TemplatesView::scroll) clamps against. A
    /// `Cell` for [`page`](TemplatesView::page)'s reason.
    pane_rows: Cell<usize>,
    /// Browsing, naming a new template, or editing one.
    mode: Mode,
    /// The write in flight, by `StoreRequest::name`. One at a time (`settings/prompt.rs`'s rule):
    /// the staleness index keeps only the newest request of a kind.
    busy: Option<&'static str>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
    /// What `E`/`Ctrl+E` asked `$EDITOR` for, until `on_external_edit`.
    external: Option<Pending>,
    /// The editor's last drawn height: what `PageUp`/`PageDown` move by. A `Cell` because the
    /// height is known only in `render(&self)`.
    page: Cell<u16>,
}

/// One row of the tree, derived from the snapshot on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A project header: its slug.
    Project(ProjectId),
    /// One template name, all its versions.
    Template {
        /// Whose.
        project: ProjectId,
        /// Which.
        name: String,
    },
}

impl Row {
    /// The project the row belongs to.
    fn project(&self) -> ProjectId {
        match self {
            Self::Project(project) | Self::Template { project, .. } => *project,
        }
    }
}

/// What `d` diffs the shown version against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffBase {
    /// One stored version (`b`).
    Version(i32),
    /// The compiled default, `prompt::body_of` (`D`, OQ-7).
    Default,
}

/// The right-hand pane in Browse.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// The shown version's body.
    #[default]
    Body,
    /// The diff from the base to the shown version.
    Diff,
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the tree.
    #[default]
    Browse,
    /// Typing a new template's name (`n`).
    Naming {
        /// The project the new name goes into.
        project: ProjectId,
        /// The name.
        field: TextField,
    },
    /// The in-app editor (`e`, `Enter` on a new name, an `$EDITOR` return).
    Editing(Editor),
}

/// An open editor. Never `Debug`s a body.
struct Editor {
    /// The template's project.
    project: ProjectId,
    /// The template's name; its role is `TemplateRole::of_name(name)`.
    name: String,
    /// The head version when the editor opened: the compare-and-set token (`None`: a new name).
    token: Option<i32>,
    /// The version the draft started from, for the title (`None`: a new name).
    from: Option<i32>,
    /// The draft.
    area: TextArea,
    /// The text the draft started from: `Esc` asks only when the draft differs.
    original: String,
    /// The phase body's missing `{{item}}` was warned about; the next `Ctrl+S` saves anyway.
    confirm_item: bool,
    /// `Esc` warned about unsaved changes; the next one discards.
    esc_armed: bool,
    /// The body the save in flight carries: what tells that save's row from another session's
    /// at the same version, and a draft typed on since from the one that was saved.
    sent: Option<String>,
}

impl Editor {
    /// An editor over `text`, cursor at byte 0.
    fn new(
        project: ProjectId,
        name: String,
        token: Option<i32>,
        from: Option<i32>,
        text: &str,
    ) -> Self {
        Self {
            project,
            name,
            token,
            from,
            area: TextArea::with_text(text),
            original: text.to_owned(),
            confirm_item: false,
            esc_armed: false,
            sent: None,
        }
    }
}

/// Lengths, never the text: `original` and `sent` are the body, as the draft is (`TextArea`'s
/// rule).
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("project", &self.project)
            .field("name", &self.name)
            .field("token", &self.token)
            .field("from", &self.from)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("confirm_item", &self.confirm_item)
            .field("esc_armed", &self.esc_armed)
            .field("sent_len", &self.sent.as_ref().map(String::len))
            .finish()
    }
}

/// An `$EDITOR` handoff in flight.
///
/// The editor travels with it: `Edited` opens it on the returned text, and a `Ctrl+E` handoff
/// that came back with nothing puts it back as it was.
#[derive(Debug)]
struct Pending {
    /// The editor the outcome opens, or returns to.
    editor: Editor,
    /// Whether the handoff came from the in-app editor (`Ctrl+E`) rather than from Browse (`E`).
    resume: bool,
}

/// One line of report.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// Dim.
    Info(String),
    /// `theme.error`: something the user has to act on.
    Error(String),
}

/// Where a `parse` error points: the opening `{{` for the three that have one, and `None` for
/// `MissingRequired`, whose mistake is the body as a whole (the cursor then goes to the end).
fn error_at(err: &TemplateError) -> Option<usize> {
    match err {
        TemplateError::UnknownPlaceholder { at, .. }
        | TemplateError::WrongRole { at, .. }
        | TemplateError::Unterminated { at } => Some(*at),
        TemplateError::MissingRequired { .. } => None,
    }
}

/// A key with no modifier but `SHIFT`, which is how a terminal reports a capital.
fn plain(key: &KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

impl TemplatesView {
    /// Whether an editor or the name prompt is taking every key.
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// The scope changed: the tree, the editor, the pending handoff and the write in flight all
    /// belong to the workspace that was left. The notice survives, as in `settings/prompt.rs`,
    /// because the scope change is often the consequence of what it reports.
    pub(super) fn on_scope_change(&mut self) {
        let notice = self.notice.take();
        *self = Self {
            notice,
            ..Self::default()
        };
    }

    /// A key the tab did not take for the view switch.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::Browse => self.on_browse_key(key, ctx),
            Mode::Naming { .. } => self.on_naming_key(key),
            Mode::Editing(_) => self.on_editor_key(key, ctx),
        }
    }

    /// A reply addressed to the Skills tab.
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Templates(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                self.land_save();
                self.clamp_cursor();
            }
            StoreReply::TemplatesStale(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                if self.busy == Some(SAVE_NAME) {
                    self.busy = None;
                    if let Mode::Editing(editor) = &mut self.mode {
                        editor.sent = None;
                        if let Some(head) = snapshot.head(editor.project, &editor.name) {
                            // The draft stays; the token moves to the head the user has now been
                            // told about, so the next `Ctrl+S` is a deliberate overwrite-by-append.
                            editor.token = Some(head.version);
                            self.notice =
                                Some(Notice::Error(template_changed_elsewhere(head.version)));
                        }
                    }
                }
                self.clamp_cursor();
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                // A refused save leaves the editor over its text: nothing was written.
                self.busy = None;
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.sent = None;
                }
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
    }

    /// The `$EDITOR` handoff came back (plan D11, OQ-3). No handoff pending (the scope changed
    /// meanwhile): ignored.
    pub(super) fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        let Some(Pending { mut editor, resume }) = self.external.take() else {
            return;
        };
        match outcome {
            ExternalEditOutcome::Edited(text) => {
                // The `Ctrl+S` gate minus the send: the cursor goes to the error, else to 0.
                editor.area = TextArea::with_text(&text);
                editor.confirm_item = false;
                editor.esc_armed = false;
                self.notice = Some(match parse(TemplateRole::of_name(&editor.name), &text) {
                    Err(err) => {
                        editor.area.set_cursor(error_at(&err).unwrap_or(text.len()));
                        Notice::Error(err.to_string())
                    }
                    Ok(_) => Notice::Info(EDITED.to_owned()),
                });
                self.mode = Mode::Editing(editor);
            }
            ExternalEditOutcome::Unchanged { quick } => {
                if resume {
                    self.mode = Mode::Editing(editor);
                }
                let wait = if quick { WAIT_FLAG } else { "" };
                self.notice = Some(Notice::Info(format!("{NO_CHANGES}{wait}")));
            }
            ExternalEditOutcome::Failed(message) => {
                if resume {
                    self.mode = Mode::Editing(editor);
                }
                self.notice = Some(Notice::Error(message));
            }
        }
    }

    /// Draws the view below the switch line: the content, the notice row and the hint row.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // The notice wraps rather than clips (at most `NOTICE_LINES`): the stale sentence ends on
        // the version the next `Ctrl+S` writes, which is the one part the user must not lose.
        let (notice, style) = match &self.notice {
            Some(Notice::Info(text)) => (text.as_str(), ctx.theme.dim),
            Some(Notice::Error(text)) => (text.as_str(), ctx.theme.error),
            None => ("", ctx.theme.dim),
        };
        let mut notice = wrapped(notice, usize::from(area.width.saturating_sub(1)).max(1));
        notice.truncate(NOTICE_LINES);
        let notice_height = u16::try_from(notice.len().max(1)).unwrap_or(1);
        let [content, notice_row, hint_row] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(notice_height),
            Constraint::Length(1),
        ])
        .areas(area);
        let hint = match &self.mode {
            Mode::Editing(editor) => {
                self.render_editor(frame, content, editor, ctx.theme);
                let (line, col) = editor.area.cursor_line_col();
                format!("{EDIT_HINT}  L{}:C{}", line + 1, col + 1)
            }
            Mode::Naming { .. } => {
                self.render_browse(frame, content, ctx);
                NAMING_HINT.to_owned()
            }
            Mode::Browse => {
                self.render_browse(frame, content, ctx);
                BROWSE_HINT.to_owned()
            }
        };
        frame.render_widget(
            Paragraph::new(
                notice
                    .into_iter()
                    .map(|line| Line::styled(format!(" {line}"), style))
                    .collect::<Vec<_>>(),
            ),
            notice_row,
        );
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {hint}"), ctx.theme.dim)),
            hint_row,
        );
    }

    // --- keys ----------------------------------------------------------------------------------

    /// Browse (plan D13). Every key here misses the global table: `q`, `Tab`, `Shift+Tab`, the
    /// digits, `?` and `w` are not among them, and the tab took `h`/`l`/`[`/`]`/arrows first.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !plain(&key) {
            return Handled::Pass;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(true),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(false),
            KeyCode::Char('r') => {
                self.notice = None;
                ctx.request(StoreRequest::Templates(ctx.scope.clone()));
            }
            KeyCode::Char('n') => self.open_naming(),
            KeyCode::Char('J' | 'K') | KeyCode::PageDown | KeyCode::PageUp => {
                return self.scroll.on_key(key, self.pane_rows.get());
            }
            KeyCode::Char(c @ (',' | '.' | 'b' | 'd' | 'D' | 'e' | 'E')) => {
                self.notice = None;
                match self.selected_template() {
                    Some((project, name)) => self.on_template_key(c, project, name, ctx),
                    None if !self.rows().is_empty() => {
                        self.notice = Some(Notice::Info(SELECT_A_TEMPLATE.to_owned()));
                    }
                    None => {}
                }
            }
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// A version, diff or edit key with a template under the cursor.
    fn on_template_key(&mut self, key: char, project: ProjectId, name: String, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let versions: Vec<&PromptTemplate> = snapshot
            .projects
            .iter()
            .filter(|entry| entry.project_id == project)
            .flat_map(|entry| entry.templates.iter())
            .filter(|row| row.name == name)
            .collect();
        let (Some(head), Some(shown)) = (
            snapshot.head(project, &name),
            self.shown_row(snapshot, project, &name),
        ) else {
            return;
        };
        let (head, shown_version, shown_body) = (head.version, shown.version, shown.body.clone());
        let index = versions
            .iter()
            .position(|row| row.version == shown_version)
            .unwrap_or(0);
        let has_earlier = versions.iter().any(|row| row.version == shown_version - 1);
        match key {
            ',' | '.' => {
                let index = if key == ',' {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(versions.len().saturating_sub(1))
                };
                let version = versions.get(index).map_or(head, |row| row.version);
                self.shown = (version != head).then_some(version);
                self.scroll.reset();
            }
            'b' => {
                self.base = Some(DiffBase::Version(shown_version));
                self.notice = Some(Notice::Info(format!("base v{shown_version}")));
            }
            'd' => {
                if self.pane == Pane::Diff {
                    self.pane = Pane::Body;
                    self.scroll.reset();
                } else if self.base.is_none() && !has_earlier {
                    self.notice = Some(Notice::Info(format!(
                        "v{shown_version} has no earlier version"
                    )));
                } else {
                    self.pane = Pane::Diff;
                    self.scroll.reset();
                }
            }
            'D' => {
                if body_of(&name).is_some() {
                    self.base = Some(DiffBase::Default);
                    self.pane = Pane::Diff;
                    self.scroll.reset();
                } else {
                    self.notice = Some(Notice::Info(format!("`{name}` has no compiled default")));
                }
            }
            'e' => {
                let editor =
                    Editor::new(project, name, Some(head), Some(shown_version), &shown_body);
                self.mode = Mode::Editing(editor);
            }
            'E' => {
                ctx.emit(Action::EditExternally(ExternalEdit {
                    text: shown_body.clone(),
                    stem: name.clone(),
                }));
                let editor =
                    Editor::new(project, name, Some(head), Some(shown_version), &shown_body);
                self.external = Some(Pending {
                    editor,
                    resume: false,
                });
            }
            _ => {}
        }
    }

    /// `n`: the name prompt, for the project of the row under the cursor.
    fn open_naming(&mut self) {
        let Some(project) = self.rows().get(self.cursor).map(Row::project) else {
            return;
        };
        self.notice = None;
        self.scroll.reset();
        self.mode = Mode::Naming {
            project,
            field: TextField::new(),
        };
    }

    /// The name prompt. A refused name keeps the prompt open, so a typo is one `Backspace` away.
    fn on_naming_key(&mut self, key: KeyEvent) -> Handled {
        let Mode::Naming { project, field } = &mut self.mode else {
            return Handled::Pass;
        };
        match field.on_key(key) {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Pass => Handled::Pass,
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                let (project, name) = (*project, field.text().unwrap_or_default().to_owned());
                self.create(project, name);
                Handled::Consumed
            }
        }
    }

    /// `Enter` on a new name (plan D13): an invalid or taken name is refused; otherwise the editor
    /// opens on the compiled default of that name, or empty, with no token.
    fn create(&mut self, project: ProjectId, name: String) {
        if !PromptTemplate::name_is_valid(&name) {
            self.notice = Some(Notice::Error(invalid_template_name(&name)));
            return;
        }
        if self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.head(project, &name).is_some())
        {
            self.notice = Some(Notice::Error(format!(
                "`{name}` exists \u{2014} select it and press e"
            )));
            return;
        }
        let body = body_of(&name).unwrap_or("");
        self.notice = None;
        self.mode = Mode::Editing(Editor::new(project, name, None, None, body));
    }

    /// The editor (plan D11, D13): `Ctrl+S`, `Ctrl+E` and `Esc` are the view's, `Tab` and
    /// `Shift+Tab` pass so the shell switches tabs with the draft kept, everything else is text.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL;
        match key.code {
            KeyCode::Char('s' | 'S') if chord => {
                self.save(ctx);
                return Handled::Consumed;
            }
            KeyCode::Char('e' | 'E') if chord => {
                self.hand_off(ctx);
                return Handled::Consumed;
            }
            _ => {}
        }
        let busy = self.busy;
        let page = self.page.get();
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        let before = editor.area.text().len();
        match editor.area.on_key(key, page) {
            AreaOutcome::Consumed => {
                if editor.area.text().len() != before {
                    editor.confirm_item = false;
                    editor.esc_armed = false;
                    self.notice = None;
                }
                Handled::Consumed
            }
            AreaOutcome::Cancel => {
                if let Some(busy) = busy {
                    // The save's reply closes the editor or keeps it; leaving now would leave the
                    // reply with no editor to land on and `busy` with nothing to clear it.
                    self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
                } else if editor.esc_armed || editor.area.text() == editor.original {
                    self.mode = Mode::Browse;
                    self.notice = None;
                } else {
                    editor.esc_armed = true;
                    self.notice = Some(Notice::Info(UNSAVED.to_owned()));
                }
                Handled::Consumed
            }
            AreaOutcome::Pass => Handled::Pass,
        }
    }

    /// `Ctrl+S` (plan D11), first match wins: a write in flight; a `parse` refusal, with the
    /// cursor on its byte (or at the end for a missing required placeholder) and nothing sent; a
    /// phase body without `{{item}}`, once; otherwise the save.
    fn save(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
            return;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        let role = TemplateRole::of_name(&editor.name);
        match parse(role, editor.area.text()) {
            Err(err) => {
                let at = error_at(&err).unwrap_or(editor.area.text().len());
                editor.area.set_cursor(at);
                self.notice = Some(Notice::Error(err.to_string()));
            }
            // `omits_item` is true of every judge and handoff body, hence the role guard.
            Ok(parsed)
                if role == TemplateRole::Phase && parsed.omits_item() && !editor.confirm_item =>
            {
                editor.confirm_item = true;
                self.notice = Some(Notice::Info(OMITS_ITEM.to_owned()));
            }
            Ok(_) => {
                let body = editor.area.text().to_owned();
                editor.sent = Some(body.clone());
                self.busy = Some(SAVE_NAME);
                self.notice = Some(Notice::Info(SAVING.to_owned()));
                ctx.request(StoreRequest::SaveTemplate {
                    scope: ctx.scope.clone(),
                    project: editor.project,
                    name: editor.name.clone(),
                    body: TemplateBody::new(body),
                    expected: editor.token,
                });
            }
        }
    }

    /// `Ctrl+E`: the draft goes to `$EDITOR`, and the editor waits in the pending handoff. Not
    /// while a save is in flight, for `Esc`'s reason.
    fn hand_off(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
            return;
        }
        let Mode::Editing(editor) = core::mem::take(&mut self.mode) else {
            return;
        };
        ctx.emit(Action::EditExternally(ExternalEdit {
            text: editor.area.text().to_owned(),
            stem: editor.name.clone(),
        }));
        self.external = Some(Pending {
            editor,
            resume: true,
        });
    }

    // --- state ---------------------------------------------------------------------------------

    /// The flat list the cursor indexes: per project in snapshot order, a header, then one row per
    /// distinct name in byte order (the read's order). Derived, so it cannot disagree with the
    /// snapshot.
    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for entry in &snapshot.projects {
            rows.push(Row::Project(entry.project_id));
            let mut last: Option<&str> = None;
            for row in &entry.templates {
                if last != Some(row.name.as_str()) {
                    rows.push(Row::Template {
                        project: entry.project_id,
                        name: row.name.clone(),
                    });
                    last = Some(row.name.as_str());
                }
            }
        }
        rows
    }

    /// The template under the cursor, if the cursor is on one.
    fn selected_template(&self) -> Option<(ProjectId, String)> {
        match self.rows().into_iter().nth(self.cursor)? {
            Row::Template { project, name } => Some((project, name)),
            Row::Project(_) => None,
        }
    }

    /// The version shown for `(project, name)`: the pinned one, else the head. A pin the snapshot
    /// no longer holds falls back to the head.
    fn shown_row<'a>(
        &self,
        snapshot: &'a TemplatesSnapshot,
        project: ProjectId,
        name: &str,
    ) -> Option<&'a PromptTemplate> {
        self.shown
            .and_then(|version| snapshot.version(project, name, version))
            .or_else(|| snapshot.head(project, name))
    }

    /// `j`/`k`: one row, no wrap; the version, the base and the pane go back to the head's body,
    /// from its top.
    fn move_cursor(&mut self, down: bool) {
        let last = self.rows().len().saturating_sub(1);
        self.cursor = if down {
            (self.cursor + 1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
        self.shown = None;
        self.base = None;
        self.pane = Pane::Body;
        self.scroll.reset();
        self.notice = None;
    }

    /// Keeps the cursor on a row after the tree changed.
    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// Blueprint D27 (F-J): a `Templates` reply is the save's answer only while the save is in
    /// flight **and** it holds a version above the token. A read served before the save (a `Tab`
    /// away and back, `2`, `r`) refreshes the tree and leaves the editor, the token and `busy`
    /// alone.
    ///
    /// "Above the token" is narrowed to the one row an applied save writes: version `token + 1`
    /// with the body that was sent. A read that shows another session's `token + 1` is not the
    /// answer either; the save's own reply is then `TemplatesStale`, which keeps the draft.
    ///
    /// Keys typed while the save was in flight still edit the draft. When they did, the editor
    /// stays open on them with the token at the saved version, so the next `Ctrl+S` appends them.
    fn land_save(&mut self) {
        if self.busy != Some(SAVE_NAME) {
            return;
        }
        let (Some(snapshot), Mode::Editing(editor)) = (&self.snapshot, &self.mode) else {
            return;
        };
        let Some(sent) = editor.sent.as_deref() else {
            return;
        };
        let version = editor.token.unwrap_or(0) + 1;
        if snapshot
            .version(editor.project, &editor.name, version)
            .is_none_or(|row| row.body != sent)
        {
            return;
        }
        let saved = Row::Template {
            project: editor.project,
            name: editor.name.clone(),
        };
        self.busy = None;
        self.shown = None;
        self.base = None;
        self.pane = Pane::Body;
        self.scroll.reset();
        if let Some(index) = self.rows().iter().position(|row| *row == saved) {
            self.cursor = index;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        let sent = editor.sent.take().unwrap_or_default();
        if editor.area.text() == sent {
            self.notice = Some(Notice::Info(format!("saved v{version}")));
            self.mode = Mode::Browse;
        } else {
            editor.token = Some(version);
            editor.from = Some(version);
            editor.original = sent;
            editor.confirm_item = false;
            editor.esc_armed = false;
            self.notice = Some(Notice::Info(format!(
                "saved v{version} \u{2014} later edits kept, Ctrl+S saves them as v{}",
                version + 1
            )));
        }
    }

    // --- frames --------------------------------------------------------------------------------

    /// Browse and Naming: the tree on the left, the pane on the right.
    fn render_browse(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let [list_area, pane_area] =
            Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(1)]).areas(area);

        let list = Block::new().borders(Borders::ALL).title(" Templates ");
        let inner = list.inner(list_area);
        frame.render_widget(list, list_area);
        let lines: Vec<Line<'static>> = self
            .rows()
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let (text, style) = match row {
                    Row::Project(project) => (slug(ctx, *project), theme.title),
                    Row::Template { project, name } => {
                        let head = self
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.head(*project, name))
                            .map_or(0, |row| row.version);
                        let role = TemplateRole::of_name(name).as_str();
                        (format!("  {name:<13} v{head:<3} {role}"), theme.base)
                    }
                };
                let style = if index == self.cursor {
                    theme.selected
                } else {
                    style
                };
                Line::styled(text, style)
            })
            .collect();
        let offset = self
            .cursor
            .saturating_sub(usize::from(inner.height).saturating_sub(1));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
            inner,
        );

        // The body and the diff wrap: a default body's lines run to 150-200 columns, and a change
        // past the pane's edge would be off screen. The row count is a character wrap's, a lower
        // bound on the word wrap's (`Scroll`'s rule), so the clamp never scrolls the pane blank.
        let width = pane_area.width.saturating_sub(2);
        let (title, lines) = self.pane(width, ctx);
        let rows: usize = lines
            .iter()
            .map(|line| line.width().div_ceil(usize::from(width).max(1)).max(1))
            .sum();
        self.pane_rows.set(rows);
        let mut block = Block::new().borders(Borders::ALL).title(title);
        let visible = usize::from(pane_area.height.saturating_sub(2));
        if rows > visible || self.scroll.offset() > 0 {
            block = block.title_bottom(Line::styled(SCROLL_HINT, theme.dim).right_aligned());
        }
        let inner = block.inner(pane_area);
        frame.render_widget(block, pane_area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll.offset(), 0)),
            inner,
        );
    }

    /// The right-hand pane's title and lines: the name prompt, a refusal, the empty states, the
    /// shown body, or the diff. The caller wraps and scrolls them.
    fn pane(&self, width: u16, ctx: &Ctx<'_>) -> (String, Vec<Line<'static>>) {
        let theme = ctx.theme;
        if let Mode::Naming { project, field } = &self.mode {
            let prompt = format!("new template in {}: ", slug(ctx, *project));
            let budget = width.saturating_sub(u16::try_from(prompt.chars().count()).unwrap_or(0));
            let mut spans = vec![Span::styled(prompt, theme.base)];
            spans.extend(field.line(budget, true, theme).spans);
            return (" new template ".to_owned(), vec![Line::from(spans)]);
        }
        let dim = |text: String| (String::new(), vec![Line::styled(text, theme.dim)]);
        if let Some(why) = &self.unavailable {
            return (
                String::new(),
                vec![Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error)],
            );
        }
        let Some(snapshot) = &self.snapshot else {
            return dim(NOT_READ.to_owned());
        };
        let Some((project, name)) = self.selected_template() else {
            return dim(SELECT_A_TEMPLATE.to_owned());
        };
        let (Some(head), Some(shown)) = (
            snapshot.head(project, &name),
            self.shown_row(snapshot, project, &name),
        ) else {
            return dim(SELECT_A_TEMPLATE.to_owned());
        };
        if self.pane == Pane::Body {
            return (
                format!(" {name} v{} (head v{}) ", shown.version, head.version),
                shown
                    .body
                    .lines()
                    .map(|line| Line::styled(line.to_owned(), theme.base))
                    .collect(),
            );
        }
        let base = match self.base {
            Some(DiffBase::Default) => body_of(&name).map(|body| ("default".to_owned(), body)),
            Some(DiffBase::Version(version)) => snapshot
                .version(project, &name, version)
                .map(|row| (format!("v{version}"), row.body.as_str())),
            None => snapshot
                .version(project, &name, shown.version - 1)
                .map(|row| (format!("v{}", row.version), row.body.as_str())),
        };
        let Some((label, text)) = base else {
            return (
                format!(" {name} v{} ", shown.version),
                vec![Line::styled(
                    format!("v{} has no base to diff against", shown.version),
                    theme.dim,
                )],
            );
        };
        let unified = diff::unified(
            text,
            &shown.body,
            &format!("{name} {label}"),
            &format!("{name} v{}", shown.version),
        );
        (
            format!(" diff {label} \u{2192} v{} ", shown.version),
            diff::lines(&unified, theme),
        )
    }

    /// The editor: the draft on the left, the role's placeholders on the right (plan D14).
    fn render_editor(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, theme: &Theme) {
        let [left, right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(HELP_WIDTH)]).areas(area);
        let title = match (editor.token, editor.from) {
            (Some(token), from) => format!(
                " {} \u{b7} editing from v{}, saves v{} ",
                editor.name,
                from.unwrap_or(token),
                token + 1
            ),
            (None, _) => format!(" {} \u{b7} new, saves v1 ", editor.name),
        };
        let block = Block::new().borders(Borders::ALL).title(title);
        let inner = block.inner(left);
        frame.render_widget(block, left);
        self.page.set(inner.height);
        frame.render_widget(
            Paragraph::new(editor.area.lines(inner.width, inner.height, true, theme)),
            inner,
        );

        let role = TemplateRole::of_name(&editor.name);
        let required = Placeholder::required_by(role);
        let mut lines: Vec<Line<'static>> = Placeholder::ALL
            .iter()
            .filter(|placeholder| placeholder.allowed_in(role))
            .map(|placeholder| {
                let kind = if placeholder.is_section() {
                    "section"
                } else {
                    "scalar"
                };
                let need = if required.contains(placeholder) {
                    " required"
                } else {
                    ""
                };
                Line::styled(
                    format!("{{{{{}}}}} {kind}{need}", placeholder.token()),
                    theme.base,
                )
            })
            .collect();
        let wire = match (role, editor.name.as_str()) {
            (TemplateRole::Judge, _) => Some(JUDGE_WIRE),
            (_, "review") => Some(REVIEW_WIRE),
            _ => None,
        };
        if let Some(wire) = wire {
            lines.push(Line::default());
            lines.push(Line::styled(wire, theme.dim));
        }
        let block = Block::new()
            .borders(Borders::ALL)
            .title(format!(" {} placeholders ", role.as_str()));
        let inner = block.inner(right);
        frame.render_widget(block, right);
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

/// Whether a snapshot answers the scope the view is in now: a save's reply that crossed a scope
/// change is fresh to the staleness index (its kind was not re-issued), and its tree belongs to
/// the workspace that was left.
fn in_scope(snapshot: &TemplatesSnapshot, ctx: &Ctx<'_>) -> bool {
    snapshot
        .projects
        .iter()
        .map(|entry| entry.project_id)
        .eq(ctx.scope.project_ids.iter().copied())
}

/// A project header: its slug from the scope's projects, else the id's first eight chars.
fn slug(ctx: &Ctx<'_>, project: ProjectId) -> String {
    ctx.projects
        .iter()
        .find(|entry| entry.project_id == project)
        .map_or_else(
            || project.to_string().chars().take(8).collect(),
            |entry| entry.slug.clone(),
        )
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::Scope;
    use htui_core::store::MemStore;
    use htui_store::Backend;

    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::{Origin, StoreRequest};
    use crate::templates;
    use crate::ui::Theme;
    use crate::ui::tabs::SkillsTab;
    use crossterm::event::{KeyCode, KeyModifiers};

    /// The Harness's startup scope: the Graphics workspace and its one project.
    fn vulkan() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: vec![ids::PROJECT_VULKAN],
        }
    }

    /// One served reply, as the worker would send it.
    async fn serve(backend: &Backend, request: StoreRequest) -> StoreReply {
        templates::serve(backend, &request)
            .await
            .unwrap_or_else(|err| panic!("the template request failed: {err}"))
    }

    /// The requests `emit` holds, drained.
    fn sent(emit: &Emit) -> Vec<StoreRequest> {
        emit.take()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    /// `Ctrl+S` on a body `parse` refuses dispatches nothing: the store would refuse it too, so
    /// only a count of the requests tells the local refusal from a sent-and-refused save.
    #[tokio::test]
    async fn a_refused_save_dispatches_no_request() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        for _ in 0..3 {
            view.on_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut ctx,
            );
        }
        view.on_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            &mut ctx,
        );
        for c in "{{itme}}".chars() {
            view.on_key(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                &mut ctx,
            );
        }
        assert!(sent(&emit).is_empty(), "typing sends nothing");
        view.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut ctx,
        );
        let requests = sent(&emit);
        assert!(requests.is_empty(), "the refused save sent {requests:?}");
        assert_eq!(view.busy, None);
        assert!(
            matches!(&view.notice, Some(Notice::Error(text)) if text.starts_with("unknown prompt placeholder")),
            "{:?}",
            view.notice
        );
    }

    /// The view derives `Debug` down to the open editor, whose `original` and `sent` are the
    /// body: lengths only, as the draft's `TextArea`.
    #[test]
    fn an_open_editor_debug_prints_lengths_not_text() {
        let mut editor = Editor::new(
            ids::PROJECT_VULKAN,
            "implement".to_owned(),
            Some(1),
            Some(1),
            "secret original",
        );
        editor.sent = Some("secret sent".to_owned());
        let view = TemplatesView {
            mode: Mode::Editing(editor),
            ..TemplatesView::default()
        };
        let shown = format!("{view:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("original_len: 15"), "{shown}");
        assert!(shown.contains("sent_len: Some(11)"), "{shown}");
    }

    /// D27 (F-J): a `Templates` read served while a save is in flight — `Tab` away and back, `2`,
    /// or `r` — must not be taken for the save's answer. `settle` serves in queue order, save
    /// first, so only a direct drive can put a read's reply ahead of the save's.
    #[tokio::test]
    async fn a_read_reply_does_not_close_the_editor_mid_save() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = TemplatesView::default();
        let untouched = serve(&backend, StoreRequest::Templates(scope.clone())).await;
        view.on_reply(&untouched, &mut ctx);

        // The tree is `[vulkan, fix, handoff, implement, …]`: three `j`s reach `implement`.
        for _ in 0..3 {
            view.on_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut ctx,
            );
        }
        view.on_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            &mut ctx,
        );
        view.on_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut ctx,
        );
        view.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut ctx,
        );
        let sent: Vec<StoreRequest> = emit
            .take()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect();
        let [save] = sent.as_slice() else {
            panic!("exactly the save was sent: {sent:?}");
        };
        let StoreRequest::SaveTemplate { name, expected, .. } = save else {
            panic!("not a save: {save:?}");
        };
        assert_eq!((name.as_str(), *expected), ("implement", Some(1)));
        assert_eq!(view.busy, Some("save_template"));

        // A read at the token's own head: the save has not landed in it.
        view.on_reply(&untouched, &mut ctx);
        assert!(
            matches!(&view.mode, Mode::Editing(editor) if editor.name == "implement"),
            "a read whose head is the token is not the save's answer: {:?}",
            view.mode
        );
        assert_eq!(
            view.busy,
            Some("save_template"),
            "the save is still in flight"
        );

        // The save's own answer: the head is token + 1.
        let saved = serve(&backend, save.clone()).await;
        view.on_reply(&saved, &mut ctx);
        assert!(
            matches!(view.mode, Mode::Browse),
            "the head moved past the token, so the save landed: {:?}",
            view.mode
        );
        assert_eq!(view.busy, None);
        assert_eq!(view.notice, Some(Notice::Info("saved v2".to_owned())));
    }
}
