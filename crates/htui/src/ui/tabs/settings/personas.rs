//! The persona registry section of the Settings tab (MOD-26 milestone 2, D22): the list of
//! `persona` rows, and the keys that create, edit, re-body and re-rule them.
//!
//! A section like `Agents`: the registry is global, so the read is unscoped and a scope change
//! moves nothing. Every write is one request served by [`crate::persona_settings`] in the store
//! loop (`R-NF-3`) and answered by one self-naming [`StoreReply::PersonaWritten`]; a plain
//! [`StoreReply::Personas`] replaces the rows and never closes an editor or moves its token.
//!
//! `n` opens the seven one-line fields (the frontmatter spellings), and `Enter` there moves on to
//! the body, because a persona needs one: `Ctrl+S` in the body creates the row, with an id this
//! section mints (B-13). `e` edits the fields of the selected row and sends only what changed;
//! `b` edits its body. Every refusal shown before anything is sent is the store's own sentence
//! (I-8): the form runs the store's rule over a one-field patch, so no second grammar can drift.
//! Personas are not mirrored, so offline the read is refused and the section offers no key but
//! navigation.

use chrono::{DateTime, Utc};
use htui_core::model::persona::{list_of, new_persona_refusal, persona_patch_refusal};
use htui_core::model::{
    NewPersona, Persona, PersonaDefault, PersonaId, PersonaPatch, PersonaPermission, PersonaTools,
    Scope,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::persona_settings::{PersonaWrite, READ_NAME, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    is_error, wrapped, yes_or_no,
};
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The Browse keys over a readable registry with rows.
pub const HINT_BROWSE: &str = "j/k select \u{b7} n new \u{b7} e edit \u{b7} b body \u{b7} r rules \u{b7} d delete \u{b7} I import";

/// The Browse keys over an empty, readable registry.
pub const HINT_EMPTY: &str = "n new \u{b7} I import";

/// The Browse keys while the registry is unavailable: none but navigation.
pub const HINT_UNAVAILABLE: &str = "";

/// The create form's keys.
pub const HINT_FORM_NEW: &str = "Tab/Shift+Tab field \u{b7} Enter body \u{b7} Esc cancel";

/// The edit form's keys.
pub const HINT_FORM_EDIT: &str = "Tab/Shift+Tab field \u{b7} Enter save \u{b7} Esc cancel";

/// The new persona's body editor keys.
pub const HINT_BODY_NEW: &str = "Ctrl+S create \u{b7} Esc back to the fields";

/// The body (edit) and rules editors' keys.
pub const HINT_EDITOR: &str = "Ctrl+S save \u{b7} Esc cancel \u{b7} Enter breaks the line";

/// Before the first read answered.
pub const NOT_READ: &str = "personas not read yet";

/// The opening of a refused read, drawn `{UNAVAILABLE}: {message}`.
pub const UNAVAILABLE: &str = "personas unavailable";

/// A readable, empty registry.
pub const NO_PERSONAS: &str = "no personas yet";

/// `e`, `b`, `r` or `d` with no row under the cursor.
pub const NO_ROW: &str = "no persona is selected";

/// A save that would write what the row already holds.
pub const UNCHANGED: &str = "nothing changed; nothing was written";

/// The first `Esc` over an edited body or rules text.
pub const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";

/// The `command-run (y/n)` field holds neither.
pub const COMMAND_RUN_IS_Y_OR_N: &str = "`command-run (y/n)` is y or n";

/// The `permission-default` field holds something else.
pub const DEFAULT_IS_ASK_OR_DENY: &str = "`permission-default` is blank, ask or deny";

/// A body or rules save whose token was spent: the editor stays, and `Ctrl+S` retries.
pub const CHANGED_ELSEWHERE_SAVE: &str = "changed elsewhere since you opened it \u{2014} reloaded; Ctrl+S retries against the current row";

/// What a spent token says when fields the user changed were changed elsewhere too, before their
/// labels (copied from `agents.rs`, which stays untouched). Opens like [`CHANGED_ELSEWHERE`], so
/// [`is_error`] draws it the same.
pub const CHANGED_ON_BOTH_SIDES: &str =
    "changed elsewhere \u{2014} reloaded; Enter retries \u{b7} also changed elsewhere: ";

/// A write answered `Gone` with no editor of that row open.
pub const GONE_CLOSED: &str = "deleted elsewhere; nothing was written";

/// The form's labels, in tab order: the frontmatter spellings (D22).
pub const FIELD_LABELS: [&str; 7] = [
    "name",
    "description",
    "tools",
    "disallowed-tools",
    "deny-kinds",
    "command-run (y/n)",
    "permission-default",
];

const NAME: &str = FIELD_LABELS[0];
const DESCRIPTION: &str = FIELD_LABELS[1];
const TOOLS: &str = FIELD_LABELS[2];
const DISALLOWED: &str = FIELD_LABELS[3];
const DENY_KINDS: &str = FIELD_LABELS[4];
const COMMAND_RUN: &str = FIELD_LABELS[5];
const DEFAULT: &str = FIELD_LABELS[6];

/// How many lines `PgUp`/`PgDn` move in the body and rules editors.
const EDITOR_PAGE: u16 = 10;

/// The widest note line a clash notice may take: the Settings pane's border costs two of 100.
const NOTE_WIDTH: usize = 98;

/// The separator of the Browse line's segments.
const DOT: &str = " \u{b7} ";

/// The persona registry, with the keys that edit it (MOD-26 M2 D22).
#[derive(Debug, Default)]
pub struct PersonasSection {
    /// The registry by name, as the last `Personas`/`PersonaWritten`/`PersonaImports` answered.
    personas: Vec<Persona>,
    /// Whether any read has answered yet: before it, the pane says [`NOT_READ`].
    read: bool,
    /// `Some(message)` after `Failed { request: "personas" }`.
    unavailable: Option<String>,
    /// Index into `personas`; no wrap (a held `j` must not aim `d` at a row nobody looked at).
    cursor: usize,
    /// What the section is doing.
    mode: Mode,
    /// The write in flight, by request name; one at a time.
    busy: Option<&'static str>,
    /// The last outcome, drawn in `theme.error` when `Notice::Error`.
    notice: Option<Notice>,
}

/// What the section is doing.
#[derive(Debug, Default)]
enum Mode {
    /// The list and the cursor; captures nothing.
    #[default]
    Browse,
    /// `n` or `e`: the seven one-line fields.
    Editing(Editor),
    /// `b`, or `n`'s second step: the body `TextArea`.
    Body(BodyEditor),
}

/// The fields form. `Debug` hand-written: labels and focus only, never typed text (H-7).
struct Editor {
    /// What `Enter` does.
    target: Target,
    /// The inputs, labelled from [`FIELD_LABELS`].
    fields: Vec<Field>,
    /// Index into `fields`.
    focus: usize,
}

impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let target = match &self.target {
            Target::Create { .. } => "create",
            Target::Edit { .. } => "edit",
        };
        f.debug_struct("Editor")
            .field("target", &target)
            .field(
                "fields",
                &self
                    .fields
                    .iter()
                    .map(|field| field.label)
                    .collect::<Vec<_>>(),
            )
            .field("focus", &self.focus)
            .finish()
    }
}

/// What an open form writes.
enum Target {
    /// `n`: a new row; `body` keeps the body editor's text across `Esc` back to the fields.
    Create {
        /// The body typed so far.
        body: String,
    },
    /// `e`: one row under CAS; `opened` is the row the form prefilled from (rebased on `Stale`).
    Edit {
        /// The row.
        id: PersonaId,
        /// The token: the row's `updated_at` as a registry reply answered it.
        expected: DateTime<Utc>,
        /// The row the form prefilled from: the "unchanged" baseline. Boxed: a row is large.
        opened: Box<Persona>,
    },
}

/// One labelled input of the form.
struct Field {
    /// One of [`FIELD_LABELS`].
    label: &'static str,
    /// The buffer.
    input: TextField,
}

/// The body editor. `Debug` hand-written: the texts are printed as lengths only (H-7).
struct BodyEditor {
    /// What `Ctrl+S` writes.
    target: BodyTarget,
    /// The widget.
    area: TextArea,
    /// The text it opened on: the "unchanged" and warn-once baseline.
    original: String,
    /// The first `Esc` over an edited text armed the second.
    esc_armed: bool,
    /// The body in flight, for the skills rule on `Updated`.
    sent: Option<String>,
}

impl core::fmt::Debug for BodyEditor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BodyEditor")
            .field("target", &self.target)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("esc_armed", &self.esc_armed)
            .field("sent_len", &self.sent.as_ref().map(String::len))
            .finish()
    }
}

/// What a body editor writes.
#[derive(Debug)]
enum BodyTarget {
    /// `n`'s second step: the form it came from, kept whole for `Esc`.
    Create {
        /// The fields form, boxed: it is the largest thing a mode holds.
        form: Box<Editor>,
    },
    /// `b`: one row's body under CAS.
    Edit {
        /// The row.
        id: PersonaId,
        /// Its name, for the header.
        name: String,
        /// The token.
        expected: DateTime<Utc>,
    },
}

/// One line on the notice row.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// One line of report.
    Info(String),
    /// One line the user has to act on, drawn in `theme.error`.
    Error(String),
}

impl Notice {
    /// The sentence.
    fn text(&self) -> &str {
        match self {
            Self::Info(text) | Self::Error(text) => text,
        }
    }
}

/// A local refusal: the field it names and the sentence.
#[derive(Debug)]
struct Refusal {
    /// One of [`FIELD_LABELS`].
    field: &'static str,
    /// The sentence: the store's own where the store has one (I-8).
    reason: String,
}

/// The form parsed: every field as the row would hold it.
#[derive(Debug)]
struct Draft {
    name: String,
    description: String,
    tools: PersonaTools,
    default: Option<PersonaDefault>,
}

impl PersonasSection {
    /// Identity of the personas section.
    pub const ID: SectionId = SectionId("personas");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The row under the cursor.
    fn selected(&self) -> Option<&Persona> {
        self.personas.get(self.cursor)
    }

    /// Puts the cursor back inside the list after a read replaced the rows.
    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.personas.len().saturating_sub(1));
    }

    /// Sends one write and holds the guard until its answer.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// One key in Browse (B-12): `j k ↓ ↑ n e b r d I`; everything else passes.
    fn on_browse_key(&mut self, key: KeyEvent) -> Handled {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Handled::Pass;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.cursor + 1 < self.personas.len() {
                    self.cursor += 1;
                }
                Handled::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                Handled::Consumed
            }
            KeyCode::Char(c @ ('n' | 'e' | 'b')) => {
                if self.unavailable.is_some() {
                    return Handled::Consumed;
                }
                if let Some(busy) = self.busy {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                    return Handled::Consumed;
                }
                if c == 'n' {
                    self.mode = Mode::Editing(Editor::create());
                    self.notice = None;
                    return Handled::Consumed;
                }
                let Some(row) = self.selected() else {
                    self.notice = Some(Notice::Error(NO_ROW.to_owned()));
                    return Handled::Consumed;
                };
                self.mode = if c == 'e' {
                    Mode::Editing(Editor::edit(row))
                } else {
                    Mode::Body(BodyEditor::edit(row))
                };
                self.notice = None;
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    /// One key while the fields form is open: the focused field first, then the form's own
    /// navigation; everything else is swallowed but `CONTROL` chords.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        let outcome = match editor.fields.get_mut(editor.focus) {
            Some(field) => field.input.on_key(key),
            None => FieldOutcome::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit_form(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => form_navigation(key, &mut editor.focus, editor.fields.len()),
        }
    }

    /// `Enter` in the form: a create moves on to the body; an edit sends the changed fields.
    fn submit_form(&mut self, ctx: &Ctx<'_>) {
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        let draft = match editor.draft() {
            Ok(draft) => draft,
            Err(refusal) => {
                editor.focus_on(refusal.field);
                self.notice = Some(Notice::Error(refusal.reason));
                return;
            }
        };
        match &editor.target {
            Target::Create { .. } => {
                let Mode::Editing(form) = core::mem::take(&mut self.mode) else {
                    return;
                };
                self.mode = Mode::Body(BodyEditor::create(form));
                self.notice = None;
            }
            Target::Edit {
                id,
                expected,
                opened,
            } => {
                if let Some(busy) = self.busy {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                    return;
                }
                let patch = draft.patch_over(opened);
                if patch == PersonaPatch::default() {
                    self.mode = Mode::Browse;
                    self.notice = Some(Notice::Info(UNCHANGED.to_owned()));
                    return;
                }
                let request = StoreRequest::UpdatePersona {
                    id: *id,
                    expected: *expected,
                    patch,
                };
                self.notice = None;
                self.send(request, ctx);
            }
        }
    }

    /// One key while the body editor is open.
    fn on_body_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let Mode::Body(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        match editor.area.on_key(key, EDITOR_PAGE) {
            FieldOutcome::Consumed => {
                editor.esc_armed = false;
                if matches!(self.notice, Some(Notice::Info(_))) {
                    self.notice = None;
                }
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                self.save_body(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.cancel_body();
                Handled::Consumed
            }
            FieldOutcome::Pass if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
            FieldOutcome::Pass => Handled::Consumed,
        }
    }

    /// `Esc` in the body editor.
    fn cancel_body(&mut self) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Mode::Body(editor) = &mut self.mode else {
            return;
        };
        if matches!(editor.target, BodyTarget::Create { .. }) {
            let Mode::Body(BodyEditor {
                target: BodyTarget::Create { mut form },
                area,
                ..
            }) = core::mem::take(&mut self.mode)
            else {
                return;
            };
            // Nothing typed is lost: the body rides on the form until it comes back.
            form.target = Target::Create {
                body: area.into_text(),
            };
            self.mode = Mode::Editing(*form);
            self.notice = None;
            return;
        }
        if editor.area.text() == editor.original || editor.esc_armed {
            self.mode = Mode::Browse;
            self.notice = None;
        } else {
            editor.esc_armed = true;
            self.notice = Some(Notice::Info(UNSAVED.to_owned()));
        }
    }

    /// `Ctrl+S` in the body editor: a create sends the whole row, an edit the body alone.
    fn save_body(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Mode::Body(editor) = &mut self.mode else {
            return;
        };
        let body = editor.area.text().to_owned();
        let request = match &mut editor.target {
            BodyTarget::Create { form } => {
                let draft = match form.draft() {
                    Ok(draft) => draft,
                    Err(refusal) => {
                        self.notice = Some(Notice::Error(refusal.reason));
                        return;
                    }
                };
                let new = NewPersona {
                    id: PersonaId::new(),
                    name: draft.name,
                    description: draft.description,
                    body,
                    tools: draft.tools,
                    permission: PersonaPermission {
                        default: draft.default,
                        rules: Vec::new(),
                    },
                };
                if let Some(sentence) = new_persona_refusal(&new) {
                    self.notice = Some(Notice::Error(sentence));
                    return;
                }
                StoreRequest::CreatePersona { new }
            }
            BodyTarget::Edit { id, expected, .. } => {
                if body == editor.original {
                    self.mode = Mode::Browse;
                    self.notice = Some(Notice::Info(UNCHANGED.to_owned()));
                    return;
                }
                let patch = PersonaPatch {
                    body: Some(body.clone()),
                    ..PersonaPatch::default()
                };
                if let Some(sentence) = persona_patch_refusal(&patch) {
                    self.notice = Some(Notice::Error(sentence));
                    return;
                }
                let request = StoreRequest::UpdatePersona {
                    id: *id,
                    expected: *expected,
                    patch,
                };
                editor.sent = Some(body);
                request
            }
        };
        self.notice = None;
        self.send(request, ctx);
    }

    /// Whether an editor of `id` is open (the form, the body editor).
    fn editing(&self, id: PersonaId) -> bool {
        match &self.mode {
            Mode::Editing(Editor {
                target: Target::Edit { id: open, .. },
                ..
            })
            | Mode::Body(BodyEditor {
                target: BodyTarget::Edit { id: open, .. },
                ..
            }) => *open == id,
            _ => false,
        }
    }

    /// What one persona write did (D21), once the rows are the re-read's.
    fn on_written(&mut self, outcome: &PersonaWrite) {
        match outcome {
            PersonaWrite::Created { id, name } => {
                if matches!(
                    self.mode,
                    Mode::Editing(Editor {
                        target: Target::Create { .. },
                        ..
                    }) | Mode::Body(BodyEditor {
                        target: BodyTarget::Create { .. },
                        ..
                    })
                ) {
                    self.mode = Mode::Browse;
                }
                if let Some(index) = self.personas.iter().position(|row| row.id == *id) {
                    self.cursor = index;
                }
                self.notice = Some(Notice::Info(format!(
                    "created persona `{}`",
                    name.escape_debug()
                )));
            }
            PersonaWrite::Updated { id, name } => {
                let updated_at = self
                    .personas
                    .iter()
                    .find(|row| row.id == *id)
                    .map(|row| row.updated_at);
                match &mut self.mode {
                    Mode::Editing(Editor {
                        target: Target::Edit { id: open, .. },
                        ..
                    }) if open == id => self.mode = Mode::Browse,
                    Mode::Body(editor) if matches!(editor.target, BodyTarget::Edit { id: open, .. } if open == *id) =>
                    {
                        if editor.sent.as_deref() == Some(editor.area.text()) {
                            self.mode = Mode::Browse;
                        } else {
                            // Keys typed while the save was in flight: the editor stays over them,
                            // the token and the baseline at what was saved (the skills rule).
                            if let (BodyTarget::Edit { expected, .. }, Some(at)) =
                                (&mut editor.target, updated_at)
                            {
                                *expected = at;
                            }
                            if let Some(sent) = editor.sent.take() {
                                editor.original = sent;
                            }
                            editor.esc_armed = false;
                        }
                    }
                    _ => {}
                }
                self.notice = Some(Notice::Info(format!(
                    "saved persona `{}`",
                    name.escape_debug()
                )));
            }
            PersonaWrite::Deleted { .. } => {
                self.notice = Some(Notice::Info("deleted persona".to_owned()));
            }
            PersonaWrite::Stale { id } => self.on_stale(*id),
            PersonaWrite::Gone { id } => {
                if self.editing(*id) {
                    self.mode = Mode::Browse;
                    self.notice = Some(Notice::Error(DELETED_ELSEWHERE.to_owned()));
                } else {
                    self.notice = Some(Notice::Error(GONE_CLOSED.to_owned()));
                }
            }
        }
    }

    /// A spent token: rebase an open editor of `id` onto the re-read's row.
    fn on_stale(&mut self, id: PersonaId) {
        if !self.editing(id) {
            self.notice = Some(Notice::Error(CHANGED_ELSEWHERE_CLOSED.to_owned()));
            return;
        }
        let Some(row) = self.personas.iter().find(|row| row.id == id).cloned() else {
            self.mode = Mode::Browse;
            self.notice = Some(Notice::Error(DELETED_ELSEWHERE.to_owned()));
            return;
        };
        let notice = match &mut self.mode {
            Mode::Editing(editor) => {
                let clashes = editor.rebase(&row);
                if clashes.is_empty() {
                    CHANGED_ELSEWHERE.to_owned()
                } else {
                    clash_notice(&clashes)
                }
            }
            Mode::Body(editor) => {
                if editor.area.text() == editor.original {
                    editor.area = at_end(&row.body);
                    editor.original.clone_from(&row.body);
                }
                if let BodyTarget::Edit { expected, .. } = &mut editor.target {
                    *expected = row.updated_at;
                }
                editor.sent = None;
                editor.esc_armed = false;
                CHANGED_ELSEWHERE_SAVE.to_owned()
            }
            Mode::Browse => return,
        };
        self.notice = Some(Notice::Error(notice));
    }

    /// The pane above the notice: the list, or the open editor.
    fn body_lines(&self, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
        match &self.mode {
            Mode::Browse => self.list_lines(width, height, theme),
            Mode::Editing(editor) => {
                let mut lines = vec![Line::styled(editor.header(), theme.base), Line::default()];
                lines.extend(editor.lines(width, theme));
                lines
            }
            Mode::Body(editor) => {
                let header = match &editor.target {
                    BodyTarget::Create { form } => {
                        format!("body of new persona `{}`", form.text(NAME).trim())
                    }
                    BodyTarget::Edit { name, .. } => format!("body of `{name}`"),
                };
                let mut lines = vec![Line::styled(header, theme.base)];
                lines.extend(
                    editor
                        .area
                        .lines(width, height.saturating_sub(1), true, theme),
                );
                lines
            }
        }
    }

    /// The registry: one line per persona, the cursor row accented with its detail under it,
    /// scrolled so the cursor row and its detail are on screen.
    fn list_lines(&self, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
        if let Some(why) = &self.unavailable {
            return wrapped(&format!("{UNAVAILABLE}: {why}"), usize::from(width).max(1))
                .into_iter()
                .map(|line| Line::styled(line, theme.error))
                .collect();
        }
        if !self.read {
            return vec![Line::styled(NOT_READ, theme.dim)];
        }
        if self.personas.is_empty() {
            return vec![Line::styled(NO_PERSONAS, theme.dim)];
        }
        let width = usize::from(width);
        let mut lines = Vec::new();
        let mut cursor_end = 0;
        for (index, row) in self.personas.iter().enumerate() {
            if index == self.cursor {
                lines.push(Line::styled(row_line(row, width), theme.accent));
                let detail = detail_line(row);
                for line in wrapped(&detail, width.saturating_sub(4).max(1)) {
                    lines.push(Line::styled(format!("    {line}"), theme.dim));
                }
                cursor_end = lines.len();
            } else {
                lines.push(Line::styled(row_line(row, width), theme.base));
            }
        }
        let skip = cursor_end.saturating_sub(usize::from(height));
        lines.into_iter().skip(skip).collect()
    }

    /// The notice row's lines, wrapped.
    fn notice_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let Some(notice) = &self.notice else {
            return Vec::new();
        };
        let style = match notice {
            Notice::Error(_) => theme.error,
            Notice::Info(text) if is_error(text) => theme.error,
            Notice::Info(_) => theme.dim,
        };
        wrapped(notice.text(), usize::from(width).max(1))
            .into_iter()
            .map(|line| Line::styled(line, style))
            .collect()
    }

    /// The key line for the mode.
    fn hint(&self) -> &'static str {
        match &self.mode {
            Mode::Browse if self.unavailable.is_some() => HINT_UNAVAILABLE,
            Mode::Browse if self.read && self.personas.is_empty() => HINT_EMPTY,
            Mode::Browse => HINT_BROWSE,
            Mode::Editing(Editor {
                target: Target::Create { .. },
                ..
            }) => HINT_FORM_NEW,
            Mode::Editing(_) => HINT_FORM_EDIT,
            Mode::Body(BodyEditor {
                target: BodyTarget::Create { .. },
                ..
            }) => HINT_BODY_NEW,
            Mode::Body(_) => HINT_EDITOR,
        }
    }
}

impl Editor {
    /// The create form: everything blank but `command-run`, which defaults to `y`.
    fn create() -> Self {
        let texts = ["", "", "", "", "", "y", ""].map(str::to_owned);
        Self {
            target: Target::Create {
                body: String::new(),
            },
            fields: fields(texts),
            focus: 0,
        }
    }

    /// The edit form over one row, prefilled.
    fn edit(row: &Persona) -> Self {
        Self {
            fields: fields(prefill(row)),
            target: Target::Edit {
                id: row.id,
                expected: row.updated_at,
                opened: Box::new(row.clone()),
            },
            focus: 0,
        }
    }

    /// The text of the field labelled `label`.
    fn text(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|field| field.label == label)
            .map_or("", Field::text)
    }

    /// Moves the focus to the field a refusal names.
    fn focus_on(&mut self, label: &str) {
        if let Some(index) = self.fields.iter().position(|field| field.label == label) {
            self.focus = index;
        }
    }

    /// The form parsed, field by field in tab order; the first refusal wins. The store's rules
    /// run over a one-field patch each, so every sentence is the store's own (I-8, B-3).
    fn draft(&self) -> Result<Draft, Refusal> {
        let name = self.text(NAME).trim().to_owned();
        let description = self.text(DESCRIPTION).to_owned();
        let allow = list_of(self.text(TOOLS));
        let deny = list_of(self.text(DISALLOWED));
        let deny_kinds = list_of(self.text(DENY_KINDS));
        let checks = [
            (
                NAME,
                PersonaPatch {
                    name: Some(name.clone()),
                    ..PersonaPatch::default()
                },
            ),
            (
                DESCRIPTION,
                PersonaPatch {
                    description: Some(description.clone()),
                    ..PersonaPatch::default()
                },
            ),
            (
                TOOLS,
                tools_patch(PersonaTools {
                    allow: allow.clone(),
                    ..PersonaTools::default()
                }),
            ),
            (
                DISALLOWED,
                tools_patch(PersonaTools {
                    deny: deny.clone(),
                    ..PersonaTools::default()
                }),
            ),
            (
                DENY_KINDS,
                tools_patch(PersonaTools {
                    deny_kinds: deny_kinds.clone(),
                    ..PersonaTools::default()
                }),
            ),
        ];
        for (field, patch) in checks {
            if let Some(reason) = persona_patch_refusal(&patch) {
                return Err(Refusal { field, reason });
            }
        }
        let command_run = yes_or_no(self.text(COMMAND_RUN)).ok_or_else(|| Refusal {
            field: COMMAND_RUN,
            reason: COMMAND_RUN_IS_Y_OR_N.to_owned(),
        })?;
        let default = match self.text(DEFAULT).trim().to_ascii_lowercase().as_str() {
            "" => None,
            "ask" => Some(PersonaDefault::Ask),
            "deny" => Some(PersonaDefault::Deny),
            _ => {
                return Err(Refusal {
                    field: DEFAULT,
                    reason: DEFAULT_IS_ASK_OR_DENY.to_owned(),
                });
            }
        };
        Ok(Draft {
            name,
            description,
            tools: PersonaTools {
                allow,
                deny,
                deny_kinds,
                command_run,
            },
            default,
        })
    }

    /// A spent token under an open edit form (agents' rule): untouched fields take the row's
    /// text, changed ones keep theirs, and the labels of fields changed on both sides come back.
    fn rebase(&mut self, current: &Persona) -> Vec<&'static str> {
        let Target::Edit {
            expected, opened, ..
        } = &mut self.target
        else {
            return Vec::new();
        };
        let before = prefill(opened);
        let after = prefill(current);
        let mut clashes = Vec::new();
        for ((field, old), new) in self.fields.iter_mut().zip(before).zip(after) {
            if field.text() == old {
                if new != old {
                    field.input = TextField::with_text(&new);
                }
            } else if new != old && field.text() != new {
                clashes.push(field.label);
            }
        }
        *expected = current.updated_at;
        **opened = current.clone();
        clashes
    }

    /// The pane's first line.
    fn header(&self) -> String {
        match &self.target {
            Target::Create { .. } => "new persona".to_owned(),
            Target::Edit { opened, .. } => format!("edit persona `{}`", opened.name),
        }
    }

    /// One line per field, the focused label accented and the focused field carrying the cursor
    /// (agents' `Editor::lines`); the label column is the widest label, `permission-default`.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let label_width = FIELD_LABELS
            .iter()
            .map(|label| label.chars().count())
            .max()
            .unwrap_or(0);
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let focused = index == self.focus;
                let style = if focused { theme.accent } else { theme.dim };
                let mut spans = vec![Span::styled(
                    format!("{:<label_width$}: ", field.label),
                    style,
                )];
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

impl Draft {
    /// The edit's patch: only the fields that differ from `opened` (D22).
    fn patch_over(self, opened: &Persona) -> PersonaPatch {
        let permission = (self.default != opened.permission.default).then(|| PersonaPermission {
            default: self.default,
            rules: opened.permission.rules.clone(),
        });
        PersonaPatch {
            name: (self.name != opened.name).then_some(self.name),
            description: (self.description != opened.description).then_some(self.description),
            body: None,
            tools: (self.tools != opened.tools).then_some(self.tools),
            permission,
        }
    }
}

impl BodyEditor {
    /// `n`'s second step, over the body the form kept.
    fn create(mut form: Editor) -> Self {
        let body = match &mut form.target {
            Target::Create { body } => core::mem::take(body),
            Target::Edit { .. } => String::new(),
        };
        Self {
            area: at_end(&body),
            original: body,
            target: BodyTarget::Create {
                form: Box::new(form),
            },
            esc_armed: false,
            sent: None,
        }
    }

    /// `b` over one row's body, the cursor at the end.
    fn edit(row: &Persona) -> Self {
        Self {
            target: BodyTarget::Edit {
                id: row.id,
                name: row.name.clone(),
                expected: row.updated_at,
            },
            area: at_end(&row.body),
            original: row.body.clone(),
            esc_armed: false,
            sent: None,
        }
    }
}

impl Field {
    /// What was typed. Never masked here.
    fn text(&self) -> &str {
        self.input.text().unwrap_or_default()
    }
}

/// The seven fields over `texts`, in [`FIELD_LABELS`] order.
fn fields(texts: [String; 7]) -> Vec<Field> {
    FIELD_LABELS
        .iter()
        .zip(texts)
        .map(|(label, text)| Field {
            label,
            input: TextField::with_text(&text),
        })
        .collect()
}

/// The form's text for a row, in [`FIELD_LABELS`] order: what `e` prefills and what a `Stale`
/// rebase compares against. Lists print with `", "`, which [`list_of`] reads back (B-3).
fn prefill(row: &Persona) -> [String; 7] {
    [
        row.name.clone(),
        row.description.clone(),
        row.tools.allow.join(", "),
        row.tools.deny.join(", "),
        row.tools.deny_kinds.join(", "),
        if row.tools.command_run { "y" } else { "n" }.to_owned(),
        match row.permission.default {
            None => "",
            Some(PersonaDefault::Ask) => "ask",
            Some(PersonaDefault::Deny) => "deny",
        }
        .to_owned(),
    ]
}

/// A one-field `tools` patch, for the store's rule over that list alone.
fn tools_patch(tools: PersonaTools) -> PersonaPatch {
    PersonaPatch {
        tools: Some(tools),
        ..PersonaPatch::default()
    }
}

/// A `TextArea` over `text` with the cursor at the end.
fn at_end(text: &str) -> TextArea {
    let mut area = TextArea::with_text(text);
    area.set_cursor(usize::MAX);
    area
}

/// One Browse line: `{name} · {description} · deny {kinds} · allow {n} · rules {n}`, the
/// description cut with `…` to fit; the name and the tail are never cut, and an empty description
/// drops its segment.
fn row_line(row: &Persona, width: usize) -> String {
    let deny = if row.tools.deny_kinds.is_empty() {
        "none".to_owned()
    } else {
        row.tools.deny_kinds.join(",")
    };
    let tail = format!(
        "{DOT}deny {deny}{DOT}allow {}{DOT}rules {}",
        row.tools.allow.len(),
        row.permission.rules.len()
    );
    if row.description.is_empty() {
        return format!("{}{tail}", row.name);
    }
    let room = width
        .saturating_sub(row.name.chars().count())
        .saturating_sub(tail.chars().count())
        .saturating_sub(DOT.chars().count());
    let description = if row.description.chars().count() <= room {
        row.description.clone()
    } else if room == 0 {
        return format!("{}{tail}", row.name);
    } else {
        let mut cut: String = row.description.chars().take(room - 1).collect();
        cut.push('\u{2026}');
        cut
    };
    format!("{}{DOT}{description}{tail}", row.name)
}

/// The detail under the cursor row.
fn detail_line(row: &Persona) -> String {
    let allow = if row.tools.allow.is_empty() {
        "all".to_owned()
    } else {
        row.tools.allow.join(", ")
    };
    let deny = if row.tools.deny.is_empty() {
        "none".to_owned()
    } else {
        row.tools.deny.join(", ")
    };
    let default = match row.permission.default {
        None => "inherit",
        Some(PersonaDefault::Ask) => "ask",
        Some(PersonaDefault::Deny) => "deny",
    };
    format!(
        "allow {allow}{DOT}disallowed {deny}{DOT}command-run {}{DOT}default {default}",
        if row.tools.command_run { "y" } else { "n" }
    )
}

/// What a form does with a key its focused field passed on: `Tab`/`Down` and `BackTab`/`Up` move
/// the focus with a wrap, a `CONTROL` chord passes so `ctrl-c` still quits, and everything else
/// is swallowed rather than offered to the shell.
fn form_navigation(key: KeyEvent, focus: &mut usize, fields: usize) -> Handled {
    let len = fields.max(1);
    match key.code {
        KeyCode::Tab | KeyCode::Down => {
            *focus = (*focus + 1) % len;
            Handled::Consumed
        }
        KeyCode::BackTab | KeyCode::Up => {
            *focus = (*focus + len - 1) % len;
            Handled::Consumed
        }
        _ if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
        _ => Handled::Consumed,
    }
}

/// [`CHANGED_ON_BOTH_SIDES`] with the clashing labels in form order, as many as fit
/// [`NOTE_WIDTH`], then ` +N more` (a private copy of `agents.rs`' `clash_notice`).
fn clash_notice(clashes: &[&str]) -> String {
    let mut notice = CHANGED_ON_BOTH_SIDES.to_owned();
    let mut shown = 0;
    for (index, label) in clashes.iter().enumerate() {
        let separator = if index == 0 { "" } else { ", " };
        let after = clashes.len() - index - 1;
        let owed = if after == 0 {
            String::new()
        } else {
            format!(" +{after} more")
        };
        let width =
            notice.chars().count() + separator.len() + label.chars().count() + owed.chars().count();
        if width > NOTE_WIDTH {
            break;
        }
        notice.push_str(separator);
        notice.push_str(label);
        shown += 1;
    }
    let hidden = clashes.len() - shown;
    if hidden > 0 {
        notice.push_str(&format!(" +{hidden} more"));
    }
    notice
}

/// The refusal of a write key while a write is in flight.
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

impl SettingsSection for PersonasSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Personas"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped: `persona` is global, so the read does not change with the workspace.
        vec![StoreRequest::Personas]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    /// Every mode but Browse (R-10): `h` and `l` are letters in every editor.
    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        match &mut self.mode {
            Mode::Editing(editor) => {
                if let Some(field) = editor.fields.get_mut(editor.focus) {
                    field.input.on_paste(text);
                }
                Handled::Consumed
            }
            Mode::Body(editor) => {
                editor.area.on_paste(text);
                Handled::Consumed
            }
            Mode::Browse => Handled::Pass,
        }
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::Browse => self.on_browse_key(key),
            Mode::Editing(_) => self.on_editor_key(key, ctx),
            Mode::Body(_) => self.on_body_key(key, ctx),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Personas(rows) => {
                self.personas.clone_from(rows);
                self.read = true;
                self.unavailable = None;
                self.clamp();
            }
            StoreReply::PersonaWritten { personas, outcome } => {
                self.personas.clone_from(personas);
                self.read = true;
                self.unavailable = None;
                self.busy = None;
                self.clamp();
                self.on_written(outcome);
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.personas.clear();
                self.read = true;
                self.clamp();
                self.unavailable = Some(message.clone());
            }
            StoreReply::Failed { request, message }
                if REQUEST_NAMES[1..].contains(request) && self.busy == Some(*request) =>
            {
                // The store's sentence, bare (B-11); every editor stays open over its text.
                self.busy = None;
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let notice = self.notice_lines(area.width, ctx.theme);
        let [pane, notice_area, hint_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(notice.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(self.body_lines(pane.width, pane.height, ctx.theme)),
            pane,
        );
        if !notice.is_empty() {
            frame.render_widget(Paragraph::new(notice), notice_area);
        }
        let hint: Style = ctx.theme.dim;
        frame.render_widget(Paragraph::new(Line::styled(self.hint(), hint)), hint_area);
    }
}
