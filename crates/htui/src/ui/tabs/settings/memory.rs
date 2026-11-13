//! A project's memory pane (MOD-163 M3, D20-D22): the entries of one project, and the keys that
//! write, edit, re-body and retire them, opened from a project row of Settings › Hierarchy.
//!
//! Self-contained (MOD-116 re-mounts it): the Hierarchy section holds one [`MemoryPane`] in its
//! `Mode::Memory` and hands it every key, paste, reply and frame while it is open, as it hands
//! the directory picker its own. It holds no store handle (`R-NF-3`): every read and write leaves
//! through `ctx.request` for [`crate::memory::serve`].
//!
//! The list is the project's live entries (`active | suspect`) in `updated_at DESC` order, kind,
//! subject, state and the date the text was last confirmed; `a` lists every state. `n` opens the
//! form (a kind choice and the subject), `Enter` moves on to the body, `Ctrl+S` there writes. `e`
//! edits the selected entry the same way under its version, `b` opens its body alone, `d` asks
//! before retiring it. A subject and a body are checked as the store checks them
//! ([`MemorySubject::new`], [`MemoryBody::new`]) before anything is sent: a refusal stays in the
//! form. A subject another live entry holds comes back as [`StoreReply::MemorySubjectTaken`] and
//! opens the question: edit that entry, or (for a new entry only, MOD-163 O-1) replace it.
//!
//! Every hint and every sentence that names a key reads it from the stack in force, and every act
//! resolves from [`Ctx::pressed`].

use core::cell::Cell;

use crossterm::event::KeyEvent;
use htui_core::model::{
    MemoryBody, MemoryEntry, MemoryEntryId, MemoryKind, MemorySubject, ProjectId,
    memory_subject_key,
};
use htui_core::scrub::MinimalScrubber;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::memory::{MemoryWrite, READ_NAME, is_write};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form, Value};
use crate::ui::kit::list::{self, RowKind};
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::tabs::settings::{changed_elsewhere, is_error};
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};

/// D21's question over a new entry's clash, byte-exact.
pub const CLASH_QUESTION: &str = "An entry with this subject exists: edit it, or replace it?";

/// The question over an amend's clash (MOD-163 O-1): only "edit it" is offered.
pub const CLASH_QUESTION_AMEND: &str = "An entry with this subject exists: edit it?";

/// A `None` read, or a write key offline (`ui/tabs/requirements/detail.rs`'s shape).
pub const MEMORY_NEEDS_THE_DATABASE: &str =
    "memory needs the database \u{b7} Settings \u{203a} Connection";

/// Before the first read answered.
pub const NOT_READ: &str = "memory not read yet";

/// A readable project with no entry in the list.
pub const NO_ENTRIES: &str = "no memory entries yet";

/// `e`, `b` or `d` with no row under the cursor.
pub const NO_ROW: &str = "no memory entry is selected";

/// A save that would write what the entry already holds.
pub const UNCHANGED: &str = "nothing changed; nothing was written";

/// A stale answer whose entry is no longer live: the edit has nothing to land on.
pub const CLOSED_ELSEWHERE: &str = "retired or replaced elsewhere; nothing was written";

/// The opening of a refused read, drawn `{UNAVAILABLE}: {message}`.
pub const UNAVAILABLE: &str = "memory unavailable";

/// The next step of [`NOT_READ`].
const NOT_READ_NEXT: &str = "the entries appear once the store answers";

/// The retire question: `` retire `{subject}`? ``.
#[must_use]
pub fn retire_question(subject: &str) -> String {
    format!("retire `{}`?", quoted(subject))
}

/// A subject as a sentence quotes it: verbatim, but for its control characters, which are
/// escaped (`\n`, `\u{1b}`) so none of them reaches the terminal (review R4-4).
fn quoted(subject: &str) -> String {
    subject
        .chars()
        .map(|c| {
            if c.is_control() {
                c.escape_debug().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// The Browse keys over a readable list with rows: `j/k select · n new · e edit · b body · d
/// retire · a all · Esc back` with the default keys.
pub const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "select"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::MemoryBody, "body"),
    Hint::One(Act::Delete, "retire"),
    Hint::One(Act::MemoryAll, "all"),
    Hint::One(Act::Back, "back"),
];

/// The Browse keys over an empty, readable list: `n new · a all · Esc back`.
pub const HINT_EMPTY: HintSpec = &[
    Hint::One(Act::New, "new"),
    Hint::One(Act::MemoryAll, "all"),
    Hint::One(Act::Back, "back"),
];

/// The Browse keys while the memory cannot be read: `Esc back`.
pub const HINT_UNAVAILABLE: HintSpec = &[Hint::One(Act::Back, "back")];

/// The form's keys: `Tab/Shift+Tab field · ←/→ kind · Enter body · Esc cancel`.
pub const HINT_FORM: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "kind"),
    Hint::One(Act::FieldSubmit, "body"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The body editor's keys: `Ctrl+s save · Esc back · Enter breaks the line`.
pub const HINT_BODY: HintSpec = &[
    Hint::One(Act::FormSave, "save"),
    Hint::One(Act::FieldCancel, "back"),
    Hint::One(Act::FieldSubmit, "breaks the line"),
];

/// The retire question's keys: `y retire · n/Esc stop`.
pub const HINT_RETIRE: HintSpec = &[
    Hint::One(Act::ConfirmYes, "retire"),
    Hint::All(Act::ConfirmNo, "stop"),
];

/// A new entry's subject question: `e edit it · r replace it · Esc back`.
pub const HINT_CLASH: HintSpec = &[
    Hint::One(Act::MemoryClashEdit, "edit it"),
    Hint::One(Act::MemoryClashReplace, "replace it"),
    Hint::One(Act::Back, "back"),
];

/// An amend's subject question (O-1): `e edit it · Esc back`.
pub const HINT_CLASH_AMEND: HintSpec = &[
    Hint::One(Act::MemoryClashEdit, "edit it"),
    Hint::One(Act::Back, "back"),
];

/// How many lines `PgUp`/`PgDn` move in the body editor.
const EDITOR_PAGE: u16 = 10;

/// The form's two rows, in tab order.
const KIND_ROW: usize = 0;
/// The subject row.
const SUBJECT_ROW: usize = 1;

/// The separator of a row's segments.
const DOT: &str = " \u{b7} ";

/// The widest the list's subject column grows, in cells.
const SUBJECT_COLUMN: usize = 48;

/// What one key did to the pane: handled (or passed) inside it, or the pane asks its host to close
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKey {
    /// The pane's answer, as a section's.
    Handled(Handled),
    /// `common.back` in Browse: the host goes back to its own view.
    Close,
}

/// One project's memory entries, with the keys that write them (MOD-163 D20).
#[derive(Debug)]
pub struct MemoryPane {
    /// The project the pane lists.
    project: ProjectId,
    /// Its slug, for the title.
    slug: String,
    /// The last list a reply carried; `None` until one did, or offline.
    entries: Option<Vec<MemoryEntry>>,
    /// Whether a `Memory` reply arrived.
    read: bool,
    /// `Memory { entries: None }`: the backend holds no memory (offline).
    offline: bool,
    /// `Failed { memory_entries }`: the read was refused.
    refused: Option<String>,
    /// Every state, not only the live ones (D20's toggle, off by default).
    all: bool,
    /// Index into `entries`; no wrap.
    cursor: usize,
    /// What the pane is doing.
    mode: Mode,
    /// The write in flight, by request name; one at a time.
    busy: Option<&'static str>,
    /// The last outcome, drawn under the pane.
    notice: Option<Notice>,
    /// The list's `(lines, first line drawn)` as of the last render.
    scroll: Cell<Option<ScrollSpan>>,
}

/// What the pane is doing.
#[derive(Debug, Default)]
enum Mode {
    /// The list and the cursor.
    #[default]
    Browse,
    /// `n` / `e`: the kind choice and the subject; `Enter` moves on to the body.
    Editing(EntryForm),
    /// The body `TextArea`; `Ctrl+S` sends a create or an amend.
    Body(BodyEditor),
    /// `d`: the y/n question, then the retire in flight.
    Retiring {
        /// The entry.
        id: MemoryEntryId,
        /// Its subject, for the question and the notice.
        subject: String,
        /// Its version as the list read it.
        expected: i32,
        /// Asked and answered `y`: the retire is in flight.
        in_flight: bool,
    },
    /// [`StoreReply::MemorySubjectTaken`]: `e` opens `existing`, `r` (a new entry only)
    /// supersedes it with the editor's text, `Esc` goes back to the body.
    Clash {
        /// The live entry holding the subject. Boxed: a row is large.
        existing: Box<MemoryEntry>,
        /// The body editor the write came from, kept whole for `Esc`.
        editor: BodyEditor,
    },
}

/// What an open form or editor writes.
#[derive(Debug, Clone)]
enum Target {
    /// A new entry.
    New,
    /// One entry under compare-and-set.
    Amend {
        /// The entry.
        id: MemoryEntryId,
        /// The token: its version as the list (or a stale answer) read it.
        expected: i32,
        /// What the entry held: the "unchanged" baseline. Boxed: a row is large.
        was: Box<Was>,
    },
}

/// An entry's text as an amend opened on it.
#[derive(Clone)]
struct Was {
    kind: MemoryKind,
    subject: String,
    body: String,
}

impl core::fmt::Debug for Was {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Was")
            .field("kind", &self.kind)
            .field("subject_len", &self.subject.len())
            .field("body_len", &self.body.len())
            .finish()
    }
}

impl Target {
    /// An amend of `entry` at its version.
    fn amend(entry: &MemoryEntry) -> Self {
        Self::Amend {
            id: entry.id,
            expected: entry.version,
            was: Box::new(Was {
                kind: entry.kind,
                subject: entry.subject.clone(),
                body: entry.body.clone(),
            }),
        }
    }

    /// The entry an amend writes, `None` for a new one.
    const fn id(&self) -> Option<MemoryEntryId> {
        match self {
            Self::New => None,
            Self::Amend { id, .. } => Some(*id),
        }
    }
}

/// The form: the kind and the subject, and the body carried to the editor. `Debug`
/// hand-written: never the typed text.
struct EntryForm {
    target: Target,
    kind: MemoryKind,
    subject: TextField,
    /// The body the editor opens on (typed so far, or the entry's).
    body: String,
    /// [`KIND_ROW`] or [`SUBJECT_ROW`].
    focus: usize,
}

impl core::fmt::Debug for EntryForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EntryForm")
            .field("target", &self.target)
            .field("kind", &self.kind)
            .field("body_len", &self.body.len())
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl EntryForm {
    /// `n`: a fact with an empty subject, the focus on the subject.
    fn new_entry() -> Self {
        Self {
            target: Target::New,
            kind: MemoryKind::Fact,
            subject: TextField::new(),
            body: String::new(),
            focus: SUBJECT_ROW,
        }
    }

    /// `e`: the entry's text under its version.
    fn amend(entry: &MemoryEntry) -> Self {
        Self {
            target: Target::amend(entry),
            kind: entry.kind,
            subject: TextField::with_text(&entry.subject),
            body: entry.body.clone(),
            focus: SUBJECT_ROW,
        }
    }

    /// What was typed into the subject.
    fn subject_text(&self) -> &str {
        self.subject.text().unwrap_or_default()
    }

    /// The pane's first line.
    fn header(&self) -> String {
        match &self.target {
            Target::New => "new memory entry".to_owned(),
            Target::Amend { was, .. } => format!("edit `{}`", was.subject),
        }
    }

    /// The two rows as a kit form.
    fn form(&self) -> Form<'_> {
        Form::new()
            .field(form::Field::new("kind", Value::Choice(self.kind.as_str())))
            .field(form::Field::new("subject", Value::Text(&self.subject)))
            .focus(self.focus)
            .editing(self.focus == SUBJECT_ROW)
    }
}

/// The body editor. `Debug` hand-written: lengths only.
struct BodyEditor {
    target: Target,
    kind: MemoryKind,
    /// The checked subject the write carries.
    subject: MemorySubject,
    area: TextArea,
    /// The text it opened on: the warn-once baseline.
    original: String,
    /// The first `Esc` over an edited text armed the second.
    esc_armed: bool,
    /// The form it came from (`n`, `e`), kept for `Esc`; `None` when `b` opened it.
    form: Option<Box<EntryForm>>,
}

impl core::fmt::Debug for BodyEditor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BodyEditor")
            .field("target", &self.target)
            .field("kind", &self.kind)
            .field("subject", &self.subject)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("esc_armed", &self.esc_armed)
            .field("form", &self.form)
            .finish()
    }
}

impl BodyEditor {
    /// The form's second step, over the body it carried.
    fn from_form(mut form: EntryForm, subject: MemorySubject) -> Self {
        let body = core::mem::take(&mut form.body);
        let area = at_end(&body);
        let original = match &form.target {
            Target::New => String::new(),
            Target::Amend { was, .. } => at_end(&was.body).text().to_owned(),
        };
        Self {
            target: form.target.clone(),
            kind: form.kind,
            subject,
            area,
            original,
            esc_armed: false,
            form: Some(Box::new(form)),
        }
    }

    /// `b`: one entry's body under its version, kind and subject unchanged.
    fn of(entry: &MemoryEntry, subject: MemorySubject) -> Self {
        let area = at_end(&entry.body);
        Self {
            target: Target::amend(entry),
            kind: entry.kind,
            subject,
            original: area.text().to_owned(),
            area,
            esc_armed: false,
            form: None,
        }
    }
}

/// One line on the notice row.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// A report.
    Info(String),
    /// One the user has to act on, drawn in `theme.error`.
    Error(String),
}

impl Notice {
    fn text(&self) -> &str {
        match self {
            Self::Info(text) | Self::Error(text) => text,
        }
    }
}

/// What a form key asks for once the form's borrow ends.
enum FormStep {
    Done(Handled),
    Submit,
    Cancel,
}

impl MemoryPane {
    /// Opens on `project` (its `slug` for the title) and answers the read to send:
    /// `MemoryEntries { project, all: false }`.
    #[must_use]
    pub fn open(project: ProjectId, slug: String) -> (Self, StoreRequest) {
        let pane = Self {
            project,
            slug,
            entries: None,
            read: false,
            offline: false,
            refused: None,
            all: false,
            cursor: 0,
            mode: Mode::Browse,
            busy: None,
            notice: None,
            scroll: Cell::new(None),
        };
        let read = pane.read_request();
        (pane, read)
    }

    /// The project the pane lists.
    #[must_use]
    pub const fn project(&self) -> ProjectId {
        self.project
    }

    /// The read the host re-sends on activation.
    #[must_use]
    pub fn read_request(&self) -> StoreRequest {
        StoreRequest::MemoryEntries {
            project: self.project,
            all: self.all,
        }
    }

    /// The stack of the current mode (MOD-67 D3): `key_stack`, `on_key` and the hint all read it.
    #[must_use]
    pub fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::MEMORY_BROWSE,
            Mode::Editing(_) => views::MEMORY_EDIT,
            Mode::Body(_) => views::MEMORY_BODY,
            Mode::Retiring { .. } => views::MEMORY_RETIRE,
            Mode::Clash { editor, .. } => match editor.target {
                Target::New => views::MEMORY_CLASH,
                Target::Amend { .. } => views::MEMORY_CLASH_AMEND,
            },
        }
    }

    /// The mode's keys for the host's hint row, through the mode's stack.
    #[must_use]
    pub fn hint_text(&self, keys: &Keys) -> String {
        keys.hint(self.stack(), self.hint_spec())
    }

    fn hint_spec(&self) -> HintSpec {
        match &self.mode {
            Mode::Browse if self.offline || self.refused.is_some() => HINT_UNAVAILABLE,
            Mode::Browse if self.rows().is_empty() => HINT_EMPTY,
            Mode::Browse => HINT_BROWSE,
            Mode::Editing(_) => HINT_FORM,
            Mode::Body(_) => HINT_BODY,
            Mode::Retiring { .. } => HINT_RETIRE,
            Mode::Clash { editor, .. } => match editor.target {
                Target::New => HINT_CLASH,
                Target::Amend { .. } => HINT_CLASH_AMEND,
            },
        }
    }

    /// The scroll as of the last render.
    #[must_use]
    pub fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    /// The rows on screen: the last list, or none.
    fn rows(&self) -> &[MemoryEntry] {
        self.entries.as_deref().unwrap_or_default()
    }

    fn selected(&self) -> Option<&MemoryEntry> {
        self.rows().get(self.cursor)
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// After a new list: the cursor follows `keep` to its row, else stays where it was, clamped
    /// (review R4-2: the cursor follows ids, not indices).
    fn reselect(&mut self, keep: Option<MemoryEntryId>) {
        if let Some(index) = keep.and_then(|id| self.rows().iter().position(|entry| entry.id == id))
        {
            self.cursor = index;
        }
        self.clamp();
    }

    /// Whether the open form, editor or question is about entry `id`.
    fn targets(&self, id: MemoryEntryId) -> bool {
        match &self.mode {
            Mode::Editing(form) => form.target.id() == Some(id),
            Mode::Body(editor) => editor.target.id() == Some(id),
            Mode::Retiring { id: asked, .. } => *asked == id,
            Mode::Clash { existing, .. } => existing.id == id,
            Mode::Browse => false,
        }
    }

    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// Why a write key is refused now, if it is: the memory cannot be written, or a write is in
    /// flight.
    fn write_refusal(&self) -> Option<String> {
        if self.offline {
            return Some(MEMORY_NEEDS_THE_DATABASE.to_owned());
        }
        if let Some(why) = &self.refused {
            return Some(format!("{UNAVAILABLE}: {why}"));
        }
        if !self.read {
            return Some(NOT_READ.to_owned());
        }
        self.busy.map(in_flight)
    }

    /// One key (the host hands every key while the pane is open).
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> PaneKey {
        let handled = match self.mode {
            Mode::Browse => return self.on_browse_key(key, ctx),
            Mode::Editing(_) => self.on_form_key(key, ctx),
            Mode::Body(_) => self.on_body_key(key, ctx),
            Mode::Retiring { .. } => self.on_retire_key(key, ctx),
            Mode::Clash { .. } => self.on_clash_key(key, ctx),
        };
        PaneKey::Handled(handled)
    }

    /// One key in Browse, through `MEMORY_BROWSE`.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> PaneKey {
        let stack = views::MEMORY_BROWSE;
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(stack, chord) {
            match act {
                Act::ListDown => {
                    if self.cursor + 1 < self.rows().len() {
                        self.cursor += 1;
                    }
                }
                Act::ListUp => self.cursor = self.cursor.saturating_sub(1),
                Act::Back => return PaneKey::Close,
                Act::MemoryAll => {
                    self.all = !self.all;
                    self.notice = None;
                    ctx.request(self.read_request());
                }
                Act::New | Act::Edit | Act::MemoryBody | Act::Delete => self.open_write(act),
                _ => continue,
            }
            return PaneKey::Handled(Handled::Consumed);
        }
        PaneKey::Handled(modal_rest(stack, chord))
    }

    /// One of Browse's writing keys: refused in place (nothing sent) offline, before the read, or
    /// while a write is in flight.
    fn open_write(&mut self, act: Act) {
        if let Some(why) = self.write_refusal() {
            self.notice = Some(Notice::Error(why));
            return;
        }
        if act == Act::New {
            self.mode = Mode::Editing(EntryForm::new_entry());
            self.notice = None;
            return;
        }
        let Some(row) = self.selected() else {
            self.notice = Some(Notice::Error(NO_ROW.to_owned()));
            return;
        };
        if !row.state.is_live() {
            self.notice = Some(Notice::Error(format!(
                "`{}` is {}: only a live entry is written",
                quoted(&row.subject),
                row.state.as_str()
            )));
            return;
        }
        let mode = match act {
            Act::Edit => Mode::Editing(EntryForm::amend(row)),
            Act::MemoryBody => match MemorySubject::new(&row.subject, &scrubber()) {
                Ok(subject) => Mode::Body(BodyEditor::of(row, subject)),
                Err(error) => {
                    self.notice = Some(Notice::Error(format!("subject: {error}")));
                    return;
                }
            },
            // The question is drawn from the mode (review R4-3), not from the notice.
            Act::Delete => Mode::Retiring {
                id: row.id,
                subject: row.subject.clone(),
                expected: row.version,
                in_flight: false,
            },
            _ => return,
        };
        self.mode = mode;
        self.notice = None;
    }

    /// One key in the form, through `MEMORY_EDIT`: the subject field first when it has the focus,
    /// then the form's moves, the kind choice, and the field's submit and cancel on the kind row.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::MEMORY_EDIT;
        let chord = ctx.pressed(&key);
        let step = {
            let Mode::Editing(form) = &mut self.mode else {
                return Handled::Pass;
            };
            let outcome = if form.focus == SUBJECT_ROW {
                form.subject.on_key(key)
            } else {
                FieldOutcome::Pass
            };
            match outcome {
                FieldOutcome::Consumed => FormStep::Done(Handled::Consumed),
                FieldOutcome::Submit => FormStep::Submit,
                FieldOutcome::Cancel => FormStep::Cancel,
                FieldOutcome::Pass => {
                    let mut step = FormStep::Done(modal_rest(stack, chord));
                    for act in ctx.keys().actions(stack, chord) {
                        match act {
                            Act::FormNextField | Act::FormPrevField => {
                                form.focus = if form.focus == KIND_ROW {
                                    SUBJECT_ROW
                                } else {
                                    KIND_ROW
                                };
                            }
                            Act::ChoicePrev if form.focus == KIND_ROW => {
                                form.kind = cycle(form.kind, false);
                            }
                            Act::ChoiceNext if form.focus == KIND_ROW => {
                                form.kind = cycle(form.kind, true);
                            }
                            Act::FieldSubmit if form.focus == KIND_ROW => {
                                step = FormStep::Submit;
                                break;
                            }
                            Act::FieldCancel if form.focus == KIND_ROW => {
                                step = FormStep::Cancel;
                                break;
                            }
                            _ => continue,
                        }
                        step = FormStep::Done(Handled::Consumed);
                        break;
                    }
                    step
                }
            }
        };
        match step {
            FormStep::Done(handled) => handled,
            FormStep::Submit => {
                self.submit_form();
                Handled::Consumed
            }
            FormStep::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
        }
    }

    /// `Enter` in the form: the subject is checked as the store checks it, then the body editor
    /// opens. A refusal stays in the form, and nothing is sent.
    fn submit_form(&mut self) {
        let Mode::Editing(form) = &mut self.mode else {
            return;
        };
        let subject = match MemorySubject::new(form.subject_text(), &scrubber()) {
            Ok(subject) => subject,
            Err(error) => {
                form.focus = SUBJECT_ROW;
                self.notice = Some(Notice::Error(format!("subject: {error}")));
                return;
            }
        };
        let Mode::Editing(form) = core::mem::take(&mut self.mode) else {
            return;
        };
        self.mode = Mode::Body(BodyEditor::from_form(form, subject));
        self.notice = None;
    }

    /// One key in the body editor: the `TextArea` first, then `form.save` through `MEMORY_BODY`.
    fn on_body_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::MEMORY_BODY;
        let cancel = ctx.keys().label(stack, Act::FieldCancel);
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
                self.cancel_body(cancel.as_deref());
                Handled::Consumed
            }
            FieldOutcome::Pass => {
                let chord = ctx.pressed(&key);
                if ctx.keys().actions(stack, chord).first() == Some(&Act::FormSave) {
                    self.save_body(ctx);
                    Handled::Consumed
                } else {
                    modal_rest(stack, chord)
                }
            }
        }
    }

    /// `Esc` in the body editor: back to the form it came from, the text riding on it; opened by
    /// `b`, the first press over an edited text arms a second, which discards.
    fn cancel_body(&mut self, cancel: Option<&str>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Mode::Body(editor) = &mut self.mode else {
            return;
        };
        if editor.form.is_some() {
            let Mode::Body(BodyEditor {
                target,
                area,
                form: Some(mut form),
                ..
            }) = core::mem::take(&mut self.mode)
            else {
                return;
            };
            form.target = target;
            form.body = area.into_text();
            self.mode = Mode::Editing(*form);
            self.notice = None;
            return;
        }
        if editor.area.text() == editor.original || editor.esc_armed {
            self.mode = Mode::Browse;
            self.notice = None;
        } else {
            editor.esc_armed = true;
            self.notice = Some(Notice::Info(unsaved(cancel)));
        }
    }

    /// `Ctrl+S` in the body editor: the body is checked as the store checks it, then a create or
    /// an amend under the editor's version goes out. An amend that changes nothing writes nothing.
    fn save_body(&mut self, ctx: &Ctx<'_>) {
        if let Some(why) = self.write_refusal() {
            self.notice = Some(Notice::Error(why));
            return;
        }
        let (project, all) = (self.project, self.all);
        let Mode::Body(editor) = &mut self.mode else {
            return;
        };
        let body = match MemoryBody::new(editor.area.text(), &scrubber()) {
            Ok(body) => body,
            Err(error) => {
                self.notice = Some(Notice::Error(format!("body: {error}")));
                return;
            }
        };
        let request = match &editor.target {
            Target::New => StoreRequest::CreateMemoryEntry {
                project,
                all,
                kind: editor.kind,
                subject: editor.subject.clone(),
                body,
            },
            Target::Amend { id, expected, was } => {
                if was.kind == editor.kind
                    && was.subject == editor.subject.as_str()
                    && was.body == body.as_str()
                {
                    self.mode = Mode::Browse;
                    self.notice = Some(Notice::Info(UNCHANGED.to_owned()));
                    return;
                }
                StoreRequest::AmendMemoryEntry {
                    project,
                    all,
                    id: *id,
                    expected: *expected,
                    kind: editor.kind,
                    subject: editor.subject.clone(),
                    body,
                }
            }
        };
        self.notice = None;
        self.send(request, ctx);
    }

    /// One key over the retire question, through `MEMORY_RETIRE`; once sent, both answers are
    /// swallowed until the store's.
    fn on_retire_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::MEMORY_RETIRE;
        let chord = ctx.pressed(&key);
        let (project, all) = (self.project, self.all);
        // Taken before the mode is borrowed: a re-read that answered offline while the question
        // was open refuses `y` in place, as every other write key is (blueprint §T4.4).
        let refusal = self.write_refusal();
        let Mode::Retiring {
            id,
            expected,
            in_flight,
            ..
        } = &mut self.mode
        else {
            return Handled::Pass;
        };
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ConfirmYes | Act::ConfirmNo) if *in_flight => Handled::Consumed,
            Some(Act::ConfirmYes) => {
                if let Some(why) = refusal {
                    self.notice = Some(Notice::Error(why));
                    return Handled::Consumed;
                }
                *in_flight = true;
                let request = StoreRequest::RetireMemoryEntry {
                    project,
                    all,
                    id: *id,
                    expected: *expected,
                };
                self.notice = None;
                self.send(request, ctx);
                Handled::Consumed
            }
            Some(Act::ConfirmNo) => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            _ => modal_rest(stack, chord),
        }
    }

    /// One key over the subject question, through `MEMORY_CLASH` (a new entry) or
    /// `MEMORY_CLASH_AMEND` (an amend, which offers no replace, O-1).
    fn on_clash_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = self.stack();
        let chord = ctx.pressed(&key);
        if self.busy.is_some() {
            return match ctx.keys().actions(stack, chord).first() {
                Some(Act::MemoryClashEdit | Act::MemoryClashReplace | Act::Back) => {
                    Handled::Consumed
                }
                _ => modal_rest(stack, chord),
            };
        }
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::MemoryClashEdit) => {
                let Mode::Clash { existing, .. } = core::mem::take(&mut self.mode) else {
                    return Handled::Pass;
                };
                self.mode = Mode::Editing(EntryForm::amend(&existing));
                self.notice = None;
                Handled::Consumed
            }
            Some(Act::MemoryClashReplace) => {
                self.replace(ctx);
                Handled::Consumed
            }
            Some(Act::Back) => {
                let Mode::Clash { editor, .. } = core::mem::take(&mut self.mode) else {
                    return Handled::Pass;
                };
                self.mode = Mode::Body(editor);
                self.notice = None;
                Handled::Consumed
            }
            _ => modal_rest(stack, chord),
        }
    }

    /// `r` over a new entry's clash (M1 D4): the holder is superseded by the editor's text. Never
    /// for an amend (O-1): its stack offers no replace, and this refuses one anyway.
    fn replace(&mut self, ctx: &Ctx<'_>) {
        if let Some(why) = self.write_refusal() {
            self.notice = Some(Notice::Error(why));
            return;
        }
        let (project, all) = (self.project, self.all);
        let Mode::Clash { existing, editor } = &self.mode else {
            return;
        };
        if !matches!(editor.target, Target::New) {
            return;
        }
        let body = match MemoryBody::new(editor.area.text(), &scrubber()) {
            Ok(body) => body,
            Err(error) => {
                self.notice = Some(Notice::Error(format!("body: {error}")));
                return;
            }
        };
        let request = StoreRequest::SupersedeMemoryEntry {
            project,
            all,
            old: existing.id,
            expected: existing.version,
            kind: editor.kind,
            subject: editor.subject.clone(),
            body,
        };
        self.notice = None;
        self.send(request, ctx);
    }

    /// A bracketed paste into the focused field.
    pub fn on_paste(&mut self, text: &str) -> Handled {
        match &mut self.mode {
            Mode::Editing(form) if form.focus == SUBJECT_ROW => {
                form.subject.on_paste(text);
                Handled::Consumed
            }
            Mode::Body(editor) => {
                editor.area.on_paste(text);
                editor.esc_armed = false;
                Handled::Consumed
            }
            // A paste lands nowhere else: dropped, so no letter of it is read as a key.
            _ => Handled::Consumed,
        }
    }

    /// One reply. A reply for another project is ignored; a write's `Failed` lands only on the
    /// write in flight.
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Memory { project, entries } if *project == self.project => {
                let keep = self.selected().map(|entry| entry.id);
                self.read = true;
                self.refused = None;
                match entries {
                    Some(entries) => {
                        self.entries = Some(entries.clone());
                        self.offline = false;
                    }
                    None => {
                        self.entries = None;
                        self.offline = true;
                    }
                }
                self.reselect(keep);
            }
            StoreReply::MemoryWritten {
                project,
                written,
                entries,
            } if *project == self.project => {
                self.busy = None;
                // A retired entry leaves the live list: its subject comes from the question.
                let asked = match core::mem::take(&mut self.mode) {
                    Mode::Retiring { subject, .. } => Some(subject),
                    _ => None,
                };
                match entries {
                    Ok(entries) => {
                        self.entries = Some(entries.clone());
                        self.read = true;
                        self.offline = false;
                        self.refused = None;
                        self.reselect(Some(written.entry()));
                        self.notice = Some(Notice::Info(self.landed(*written, asked)));
                    }
                    Err(why) => {
                        self.notice = Some(Notice::Error(format!(
                            "{}{DOT}the list was not re-read: {why}",
                            self.landed(*written, asked)
                        )));
                    }
                }
            }
            StoreReply::MemoryStale {
                project,
                head,
                entries,
            } if *project == self.project => {
                self.busy = None;
                // The cursor follows the entry the open edit is about, else the one it was on.
                let keep = if self.targets(head.id) {
                    Some(head.id)
                } else {
                    self.selected().map(|entry| entry.id)
                };
                self.entries = Some(entries.clone());
                self.read = true;
                self.reselect(keep);
                self.on_stale(head, ctx.keys());
            }
            StoreReply::MemorySubjectTaken { project, existing } if *project == self.project => {
                self.busy = None;
                match core::mem::take(&mut self.mode) {
                    // A replace met another holder: the question names that one now.
                    Mode::Body(editor) | Mode::Clash { editor, .. } => {
                        self.mode = Mode::Clash {
                            existing: Box::new(existing.clone()),
                            editor,
                        };
                        self.notice = None;
                    }
                    other => self.mode = other,
                }
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.read = true;
                self.refused = Some(message.clone());
            }
            StoreReply::Failed { request, .. }
                if is_write(request) && self.busy == Some(*request) =>
            {
                // The shell put the store's sentence on the status line; the editor stays over
                // its text, and the retire question goes back to asking.
                self.busy = None;
                if let Mode::Retiring { in_flight, .. } = &mut self.mode {
                    *in_flight = false;
                }
            }
            _ => {}
        }
    }

    /// What a landed write says; `asked` is the retire question's subject.
    fn landed(&self, written: MemoryWrite, asked: Option<String>) -> String {
        let subject = self
            .rows()
            .iter()
            .find(|entry| entry.id == written.entry())
            .map(|entry| entry.subject.clone())
            .or(asked)
            .map(|subject| quoted(&subject));
        let verb = match written {
            MemoryWrite::Created { .. } | MemoryWrite::Amended { .. } => "saved",
            MemoryWrite::Retired { .. } => "retired",
            MemoryWrite::Superseded { .. } => "replaced",
        };
        match subject {
            Some(subject) => format!("{verb} `{subject}`"),
            None => verb.to_owned(),
        }
    }

    /// A missed version: an open editor of `head` keeps its text and takes the head's version
    /// while the entry is live; a closed entry ends the edit.
    fn on_stale(&mut self, head: &MemoryEntry, keys: &Keys) {
        let live = head.state.is_live();
        if !self.targets(head.id) {
            self.notice = Some(Notice::Error(
                "changed elsewhere; nothing was written".to_owned(),
            ));
            return;
        }
        if !live {
            match core::mem::take(&mut self.mode) {
                // The holder of the subject closed: the draft goes back to its editor.
                Mode::Clash { editor, .. } => self.mode = Mode::Body(editor),
                _ => self.mode = Mode::Browse,
            }
            self.notice = Some(Notice::Error(CLOSED_ELSEWHERE.to_owned()));
            return;
        }
        // The holder of the subject was renamed elsewhere (review R4-1): the subject is free, so
        // the draft goes back to its editor, whose save writes it as it was meant.
        if let Mode::Clash { editor, .. } = &self.mode
            && memory_subject_key(&head.subject) != memory_subject_key(editor.subject.as_str())
        {
            if let Mode::Clash { editor, .. } = core::mem::take(&mut self.mode) {
                self.mode = Mode::Body(editor);
            }
            let save = keys.label(views::MEMORY_BODY, Act::FormSave);
            self.notice = Some(Notice::Info(subject_free(save.as_deref())));
            return;
        }
        let retry = match &mut self.mode {
            Mode::Editing(form) => {
                rebase(&mut form.target, head);
                keys.label(views::MEMORY_EDIT, Act::FieldSubmit)
            }
            Mode::Body(editor) => {
                rebase(&mut editor.target, head);
                if let Some(form) = &mut editor.form {
                    rebase(&mut form.target, head);
                }
                keys.label(views::MEMORY_BODY, Act::FormSave)
            }
            Mode::Retiring {
                expected,
                in_flight,
                ..
            } => {
                *expected = head.version;
                *in_flight = false;
                keys.label(views::MEMORY_RETIRE, Act::ConfirmYes)
            }
            Mode::Clash { existing, editor } => {
                **existing = head.clone();
                // An amend's question offers no replace (O-1): name the key it does offer.
                match editor.target {
                    Target::New => keys.label(views::MEMORY_CLASH, Act::MemoryClashReplace),
                    Target::Amend { .. } => {
                        keys.label(views::MEMORY_CLASH_AMEND, Act::MemoryClashEdit)
                    }
                }
            }
            Mode::Browse => None,
        };
        self.notice = Some(Notice::Error(changed_elsewhere(retry.as_deref())));
    }

    /// Draws the pane: its title, the list or the open form, editor or question, and the notice.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let notice = self.notice_lines(area.width, theme);
        let [title, pane, notice_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(u16::try_from(notice.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        let mut heading = format!("memory{DOT}{}", self.slug);
        if self.all {
            heading.push_str(" (all states)");
        }
        frame.render_widget(Paragraph::new(Line::styled(heading, theme.title)), title);
        self.scroll.set(None);
        match &self.mode {
            Mode::Browse | Mode::Retiring { .. } => self.render_list(frame, pane, ctx),
            Mode::Editing(form) => {
                let mut lines = vec![Line::styled(form.header(), theme.base), Line::default()];
                lines.extend(form.form().lines(pane.width, ctx.accent(), theme));
                frame.render_widget(Paragraph::new(lines), pane);
            }
            Mode::Body(editor) => {
                frame.render_widget(
                    Paragraph::new(body_lines(editor, pane.width, pane.height, theme)),
                    pane,
                );
            }
            Mode::Clash { existing, editor } => {
                let question = match editor.target {
                    Target::New => CLASH_QUESTION,
                    Target::Amend { .. } => CLASH_QUESTION_AMEND,
                };
                let width = usize::from(pane.width).max(1);
                let mut lines: Vec<Line<'static>> = wrapped(question, width)
                    .into_iter()
                    .map(|line| Line::styled(line, theme.base))
                    .collect();
                lines.push(Line::default());
                lines.push(Line::styled(
                    format!("  {}  {}", existing.kind.as_str(), existing.subject),
                    theme.base,
                ));
                lines.extend(
                    wrapped(&existing.body, width.saturating_sub(4).max(1))
                        .into_iter()
                        .map(|line| Line::styled(format!("    {line}"), theme.dim)),
                );
                frame.render_widget(Paragraph::new(lines), pane);
            }
        }
        if let Some(span) = self.scroll.get() {
            self.scroll.set(Some(span.rows(pane)));
        }
        if !notice.is_empty() {
            frame.render_widget(Paragraph::new(notice), notice_area);
        }
    }

    /// The entries as a kit list: one row per entry (kind, subject, state and the date its text
    /// was confirmed), the cursor row's body under it as note rows; or the state that says why
    /// there is none.
    fn render_list(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        if self.offline {
            empty::render(
                frame,
                area,
                MEMORY_NEEDS_THE_DATABASE,
                NextStep("memory is not mirrored: it returns once the store answers"),
                theme,
            );
            return;
        }
        if let Some(why) = &self.refused {
            let lines: Vec<Line<'static>> = wrapped(
                &format!("{UNAVAILABLE}: {why}"),
                usize::from(area.width).max(1),
            )
            .into_iter()
            .map(|line| notice(&line, Tone::Error, theme))
            .collect();
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }
        if !self.read {
            empty::render(frame, area, NOT_READ, NextStep(NOT_READ_NEXT), theme);
            return;
        }
        if self.rows().is_empty() {
            let next = adds_one(ctx.keys());
            empty::render(frame, area, NO_ENTRIES, NextStep(&next), theme);
            return;
        }
        let width = usize::from(area.width.saturating_sub(1));
        // The subjects share one column, as wide as the widest (capped), so the states line up.
        let column = self
            .rows()
            .iter()
            .map(|entry| cell_width(&entry.subject))
            .max()
            .unwrap_or(0)
            .min(SUBJECT_COLUMN);
        let mut rows = Vec::new();
        let mut cursor_end = 0;
        for (index, entry) in self.rows().iter().enumerate() {
            rows.push(list::Row {
                line: Line::styled(row_line(entry, column), theme.base),
                kind: RowKind::Item,
            });
            if index == self.cursor {
                for line in wrapped(&entry.body, width.saturating_sub(4).max(1)) {
                    rows.push(list::Row {
                        line: Line::styled(format!("    {line}"), theme.dim),
                        kind: RowKind::Note,
                    });
                }
                cursor_end = rows.len() - 1;
            }
        }
        let total = rows.len();
        let offset =
            list::scroll_into_view(cursor_end, 0, usize::from(area.height), total).min(self.cursor);
        list::render(
            frame,
            area,
            &rows,
            Some(self.cursor),
            offset,
            ctx.accent(),
            theme,
        );
        self.scroll.set(Some(ScrollSpan::new(total, offset)));
    }

    /// The notice row's lines, wrapped: the retire question first while it is open, drawn from
    /// the mode so an in-flight clear, a failed write or a stale answer cannot take it away
    /// (review R4-3), then the notice.
    fn notice_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let width = usize::from(width).max(1);
        let mut lines = Vec::new();
        if let Mode::Retiring { subject, .. } = &self.mode {
            lines.extend(
                wrapped(&retire_question(subject), width)
                    .into_iter()
                    .map(|line| notice(&line, Tone::Info, theme)),
            );
        }
        let Some(said) = &self.notice else {
            return lines;
        };
        let tone = match said {
            Notice::Error(_) => Tone::Error,
            Notice::Info(text) if is_error(text) => Tone::Error,
            Notice::Info(_) => Tone::Info,
        };
        lines.extend(
            wrapped(said.text(), width)
                .into_iter()
                .map(|line| notice(&line, tone, theme)),
        );
        lines
    }
}

/// The scrubber a subject and a body are checked with before anything is sent: the store's
/// shape rules, no known secret (D21).
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new(Vec::<String>::new())
}

/// The next or previous kind, with a wrap.
fn cycle(kind: MemoryKind, forward: bool) -> MemoryKind {
    let all = MemoryKind::ALL;
    let at = all.iter().position(|k| *k == kind).unwrap_or(0);
    let next = if forward {
        (at + 1) % all.len()
    } else {
        (at + all.len() - 1) % all.len()
    };
    all[next]
}

/// A missed version under an open edit: the head's version and text become the baseline.
fn rebase(target: &mut Target, head: &MemoryEntry) {
    if let Target::Amend { expected, was, .. } = target {
        *expected = head.version;
        **was = Was {
            kind: head.kind,
            subject: head.subject.clone(),
            body: head.body.clone(),
        };
    }
}

/// One list row: `{kind:<8} {subject:<column}  {state:<10}  {source:<6}  {confirmed_at:%Y-%m-%d}`
/// (D20; MOD-164 D12 adds the source, `person` or `run`), the subject padded in display cells;
/// one wider than `column` pushes its own row's tail only.
fn row_line(entry: &MemoryEntry, column: usize) -> String {
    let pad = column.saturating_sub(cell_width(&entry.subject));
    format!(
        "{:<8} {}{}  {:<10}  {:<6}  {}",
        entry.kind.as_str(),
        entry.subject,
        " ".repeat(pad),
        entry.state.as_str(),
        entry.source.as_str(),
        entry.confirmed_at.format("%Y-%m-%d")
    )
}

/// The body editor's lines: its header, then the area.
fn body_lines(editor: &BodyEditor, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
    let header = match editor.target {
        Target::New => format!(
            "body of new {} `{}`",
            editor.kind.as_str(),
            editor.subject.as_str()
        ),
        Target::Amend { .. } => format!(
            "body of {} `{}`",
            editor.kind.as_str(),
            editor.subject.as_str()
        ),
    };
    let mut lines = vec![Line::styled(header, theme.base)];
    lines.extend(
        editor
            .area
            .lines(width, height.saturating_sub(1), true, theme),
    );
    lines
}

/// A `TextArea` over `text` with the cursor at the end.
fn at_end(text: &str) -> TextArea {
    let mut area = TextArea::with_text(text);
    area.set_cursor(usize::MAX);
    area
}

/// The next step of [`NO_ENTRIES`]: the key in force that adds one.
fn adds_one(keys: &Keys) -> String {
    keys.label(views::MEMORY_BROWSE, Act::New).map_or_else(
        || "no key in force adds one".to_owned(),
        |key| format!("{key} adds one"),
    )
}

/// The first cancel over an edited body: `cancel` is `field.cancel`'s label through
/// `MEMORY_BODY`.
fn unsaved(cancel: Option<&str>) -> String {
    match cancel {
        Some(cancel) => format!("unsaved changes \u{2014} {cancel} again discards"),
        None => "unsaved changes \u{2014} cancel again to discard".to_owned(),
    }
}

/// A clash whose holder changed its subject elsewhere: `save` is `form.save`'s label through
/// `MEMORY_BODY`.
fn subject_free(save: Option<&str>) -> String {
    match save {
        Some(save) => format!("the subject is free now; {save} saves"),
        None => "the subject is free now; save to write it".to_owned(),
    }
}

/// The refusal of a write key while a write is in flight, in words.
fn in_flight(busy: &str) -> String {
    let doing = match busy {
        "create_memory_entry" => "adding the entry",
        "amend_memory_entry" => "saving the entry",
        "retire_memory_entry" => "retiring the entry",
        "supersede_memory_entry" => "replacing the entry",
        _ => "writing",
    };
    format!("still {doing} \u{2014} wait for the store's answer")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::REQUEST_NAMES;

    #[test]
    fn a_write_in_flight_reads_in_words() {
        for name in &REQUEST_NAMES[1..] {
            assert!(!in_flight(name).contains("writing"), "{name}");
            assert!(!in_flight(name).contains('_'), "{}", in_flight(name));
        }
    }

    #[test]
    fn the_kind_cycles_through_the_four_with_a_wrap() {
        let mut kind = MemoryKind::Fact;
        let mut seen = Vec::new();
        for _ in 0..4 {
            kind = cycle(kind, true);
            seen.push(kind);
        }
        assert_eq!(
            seen,
            [
                MemoryKind::Decision,
                MemoryKind::Gotcha,
                MemoryKind::Lesson,
                MemoryKind::Fact
            ]
        );
        assert_eq!(cycle(MemoryKind::Fact, false), MemoryKind::Lesson);
    }
}
