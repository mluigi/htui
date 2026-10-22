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
//! `r` edits the permission rules one per line (D17), `d` asks before deleting (a bound persona is
//! refused by the store, D14), and `I` hands a path to the worker, which reads the files (D20);
//! anything refused, skipped or dropped opens a report. Personas are not mirrored, so offline the
//! read is refused and the section offers no key but navigation.
//!
//! It draws through the kit (MOD-115 M3 L-kinds): the registry is a [`list`] with the cursor
//! row's detail as note rows, the fields form and the import path are [`Form`]s whose focused row
//! carries the marker, the outcome is [`notice`] lines and an empty registry is an [`empty`] state
//! with a next step. Every hint and every sentence that names a key reads it from the stack in
//! force (MD-A), and the acts resolve from [`Ctx::pressed`].

use core::cell::Cell;

use chrono::{DateTime, Utc};
use htui_core::model::persona::{
    format_rules, list_of, new_persona_refusal, parse_rules, persona_patch_refusal,
};
use htui_core::model::{
    NewPersona, Persona, PersonaDefault, PersonaId, PersonaPatch, PersonaPermission, PersonaTools,
    Scope,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::persona_import::PersonaOutcome;
use crate::persona_settings::{IMPORT_NAME, PersonaWrite, READ_NAME, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::{cell_width, graphemes};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form, Value};
use crate::ui::kit::list::{self, RowKind};
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection, changed_elsewhere,
    is_error, yes_or_no,
};
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};
use crossterm::event::KeyEvent;

/// The Browse keys over a readable registry with rows: `j/k select · n new · e edit · b body ·
/// r rules · d delete · I import` with the default keys.
pub const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "select"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::PersonasBody, "body"),
    Hint::One(Act::PersonasRules, "rules"),
    Hint::One(Act::Delete, "delete"),
    Hint::One(Act::PersonasImport, "import"),
];

/// The Browse keys over an empty, readable registry: `n new · I import`.
pub const HINT_EMPTY: HintSpec = &[
    Hint::One(Act::New, "new"),
    Hint::One(Act::PersonasImport, "import"),
];

/// The Browse keys while the registry is unavailable: none but navigation.
pub const HINT_UNAVAILABLE: HintSpec = &[];

/// The create form's keys: `Tab/Shift+Tab field · Enter body · Esc cancel` with the compiled
/// keys; the field's submit and cancel through the form's `[field]` layer (MOD-115 M3 C6).
pub const HINT_FORM_NEW: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::One(Act::FieldSubmit, "body"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The edit form's keys: `Tab/Shift+Tab field · Enter save · Esc cancel` with the compiled keys.
pub const HINT_FORM_EDIT: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The new persona's body editor keys: `Ctrl+s create · Esc back to the fields`.
pub const HINT_BODY_NEW: HintSpec = &[
    Hint::One(Act::FormSave, "create"),
    Hint::One(Act::FieldCancel, "back to the fields"),
];

/// The body (edit) and rules editors' keys: `Ctrl+s save · Esc cancel · Enter breaks the line`.
/// The line break is `field.submit`'s chord: the shell turns it into the canonical `Enter`, which
/// the `TextArea` reads as a newline (MOD-115 M3 C6).
pub const HINT_EDITOR: HintSpec = &[
    Hint::One(Act::FormSave, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::One(Act::FieldSubmit, "breaks the line"),
];

/// The delete question's keys: `y delete · n/Esc stop`.
pub const HINT_DELETING: HintSpec = &[
    Hint::One(Act::ConfirmYes, "delete"),
    Hint::All(Act::ConfirmNo, "stop"),
];

/// The import path's keys, the field's own through `CAPTURE`'s `[field]` layer: `Enter import ·
/// Esc cancel` with the compiled keys.
pub const HINT_IMPORT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "import"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The import report's keys: `j/k scroll · Esc close`.
pub const HINT_REPORT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "scroll"),
    Hint::One(Act::Back, "close"),
];

/// `Enter` on an empty import path.
pub const ENTER_A_PATH: &str = "type a path to a persona `.md` file or a directory of them";

/// While the worker walks an import.
pub const IMPORTING: &str = "importing\u{2026}";

/// An import whose path held no candidate file.
pub const NOTHING_IMPORTED: &str = "nothing to import at that path";

/// The rules editor's help, wrapped to the pane (D17).
pub const RULES_HELP: &str = "one rule per line: reject_once|reject_always kind=\u{2026} name=\u{2026} path=\u{2026} command=\u{2026} # reason \u{b7} quote a value holding a space, \", #, = or \\ \u{b7} kind: read edit delete move search execute think fetch switch_mode other";

/// Before the first read answered.
pub const NOT_READ: &str = "personas not read yet";

/// The next step of [`NOT_READ`].
const NOT_READ_NEXT: &str = "they appear once the store answers";

/// The next step of [`UNAVAILABLE`]: the registry has no read-only copy to fall back on.
const UNAVAILABLE_NEXT: &str = "personas are not mirrored: they return once the store answers";

/// The opening of a refused read, drawn `{UNAVAILABLE}: {message}`.
pub const UNAVAILABLE: &str = "personas unavailable";

/// A readable, empty registry.
pub const NO_PERSONAS: &str = "no personas yet";

/// `e`, `b`, `r` or `d` with no row under the cursor.
pub const NO_ROW: &str = "no persona is selected";

/// A save that would write what the row already holds.
pub const UNCHANGED: &str = "nothing changed; nothing was written";

/// The first cancel over an edited body or rules text: `cancel` is `field.cancel`'s label
/// through `PERSONAS_EDITOR` (`Esc` with the compiled keys, MOD-115 M3 C5), and with none bound
/// the sentence says a cancel.
#[must_use]
pub fn unsaved(cancel: Option<&str>) -> String {
    match cancel {
        Some(cancel) => format!("unsaved changes \u{2014} {cancel} again discards"),
        None => "unsaved changes \u{2014} cancel again to discard".to_owned(),
    }
}

/// The `command-run (y/n)` field holds neither.
pub const COMMAND_RUN_IS_Y_OR_N: &str = "`command-run (y/n)` is y or n";

/// The `permission-default` field holds something else.
pub const DEFAULT_IS_ASK_OR_DENY: &str = "`permission-default` is blank, ask or deny";

/// A body or rules save whose token was spent: the editor stays, and `save`, `FormSave`'s label
/// through `PERSONAS_EDITOR`, retries (MOD-67 M6 D4); with none bound the sentence says a save.
#[must_use]
pub fn changed_elsewhere_save(save: Option<&str>) -> String {
    match save {
        Some(save) => format!(
            "changed elsewhere since you opened it \u{2014} reloaded; {save} retries against the \
             current row"
        ),
        None => "changed elsewhere since you opened it \u{2014} reloaded; save again to retry \
                 against the current row"
            .to_owned(),
    }
}

/// What a spent token says when fields the user changed were changed elsewhere too, before their
/// labels (copied from `agents.rs`). `retry` is `field.submit`'s label through `PERSONAS_FORM`
/// (`Enter` with the compiled keys, MOD-115 M3 D-5); with none bound the sentence says a save.
/// Opens like `changed_elsewhere`, so `is_error` draws it the same.
#[must_use]
pub fn changed_on_both_sides(retry: Option<&str>) -> String {
    match retry {
        Some(retry) => format!(
            "changed elsewhere \u{2014} reloaded; {retry} retries \u{b7} also changed elsewhere: "
        ),
        None => "changed elsewhere \u{2014} reloaded; save again to retry \u{b7} also changed \
                 elsewhere: "
            .to_owned(),
    }
}

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
    /// An import report that landed over an editor (R1 M-1): it opens on the next return to
    /// Browse, so no refusal, skip or dropped entry goes unnamed (OQ-7, R-13).
    pending_report: Option<Vec<PersonaOutcome>>,
    /// The list's or the import report's `(lines, first line drawn)` as of the last render, for
    /// the Settings pane's scrollbar (MOD-115 M3 D-11), in the pane's terms: the notice rows under
    /// it count as lines, since the pane measures the bar against its whole inner rect. `None`
    /// while neither is drawn.
    scroll: Cell<Option<ScrollSpan>>,
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
    /// `r`: the rules `TextArea`, one rule per line (D17).
    Rules(RulesEditor),
    /// `d`: the y/n question, then the delete in flight.
    Deleting {
        /// The row the question names.
        id: PersonaId,
        /// Its name, for the question and the notice.
        name: String,
        /// Asking, or sent.
        stage: DeleteStage,
    },
    /// `I`: the path field.
    ImportPath {
        /// The path, one line.
        field: TextField,
    },
    /// An import's per-file report, scrolled by line.
    Report {
        /// One row per file.
        report: Vec<PersonaOutcome>,
        /// The first entry line drawn.
        top: usize,
        /// The largest `top` the last frame could draw: `j` stops there.
        max_top: Cell<usize>,
    },
}

/// Where the delete question is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteStage {
    /// The question is on screen.
    Asking,
    /// `y` was pressed; the delete is in flight.
    InFlight,
}

/// The rules editor. `Debug` hand-written: the texts are printed as lengths only (H-7).
struct RulesEditor {
    /// The row.
    id: PersonaId,
    /// Its name, for the header.
    name: String,
    /// The token.
    expected: DateTime<Utc>,
    /// The row's `permission.default`, carried into the permission patch; retaken on `Stale`.
    default: Option<PersonaDefault>,
    /// The widget.
    area: TextArea,
    /// The text it opened on: the "unchanged" and warn-once baseline.
    original: String,
    /// The first `Esc` over an edited text armed the second.
    esc_armed: bool,
    /// The text in flight.
    sent: Option<String>,
}

impl core::fmt::Debug for RulesEditor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RulesEditor")
            .field("id", &self.id)
            .field("expected", &self.expected)
            .field("default", &self.default)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("esc_armed", &self.esc_armed)
            .field("sent_len", &self.sent.as_ref().map(String::len))
            .finish()
    }
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

    /// The write in flight, unless it is the import: an import walk can be long, and the keys
    /// typed while it runs may open an editor (whose save is still refused until it lands).
    fn own_write(&self) -> Option<&'static str> {
        self.busy.filter(|busy| *busy != IMPORT_NAME)
    }

    /// Sends one write and holds the guard until its answer.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// The stack of the current mode (MOD-67 D3): the only place a mode maps to its keys;
    /// `key_stack`, `on_key` and the hint all read it.
    fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::PERSONAS_BROWSE,
            Mode::Editing(_) => views::PERSONAS_FORM,
            Mode::Body(_) | Mode::Rules(_) => views::PERSONAS_EDITOR,
            Mode::Deleting { .. } => views::PERSONAS_DELETE,
            Mode::ImportPath { .. } => views::CAPTURE,
            Mode::Report { .. } => views::PERSONAS_REPORT,
        }
    }

    /// One key in Browse (B-12), through `PERSONAS_BROWSE`: `list.down/up`, `common.new/edit/
    /// delete` and `settings.personas.body/rules/import`. Every other chord, a global act among
    /// them, passes; chord equality includes modifiers, so `ctrl-d` is not `d`.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::PERSONAS_BROWSE, chord) {
            match act {
                Act::ListDown => {
                    if self.cursor + 1 < self.personas.len() {
                        self.cursor += 1;
                    }
                    return Handled::Consumed;
                }
                Act::ListUp => {
                    self.cursor = self.cursor.saturating_sub(1);
                    return Handled::Consumed;
                }
                Act::New
                | Act::Edit
                | Act::PersonasBody
                | Act::PersonasRules
                | Act::Delete
                | Act::PersonasImport => {
                    self.open(act);
                    return Handled::Consumed;
                }
                _ => continue, // a global act (or `settings.*`, which the tab took first)
            }
        }
        Handled::Pass
    }

    /// One of Browse's writing keys: `new`, `edit`, `body`, `rules`, `delete` or `import`. Every
    /// refusal (unavailable, busy, no row) is accept-and-say, never a fall-through.
    fn open(&mut self, act: Act) {
        if self.unavailable.is_some() {
            return;
        }
        // `delete` and `import` send at once; the editors send only on their own save, which the
        // import in flight still refuses.
        let busy = if matches!(act, Act::Delete | Act::PersonasImport) {
            self.busy
        } else {
            self.own_write()
        };
        if let Some(busy) = busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        if act == Act::New {
            self.mode = Mode::Editing(Editor::create());
            self.notice = None;
            return;
        }
        if act == Act::PersonasImport {
            self.mode = Mode::ImportPath {
                field: TextField::new(),
            };
            self.notice = None;
            return;
        }
        let Some(row) = self.selected() else {
            self.notice = Some(Notice::Error(NO_ROW.to_owned()));
            return;
        };
        let (mode, notice) = match act {
            Act::Edit => (Mode::Editing(Editor::edit(row)), None),
            Act::PersonasBody => (Mode::Body(BodyEditor::edit(row)), None),
            Act::PersonasRules => (Mode::Rules(RulesEditor::edit(row)), None),
            Act::Delete => (
                Mode::Deleting {
                    id: row.id,
                    name: row.name.clone(),
                    stage: DeleteStage::Asking,
                },
                Some(Notice::Info(delete_question(&row.name))),
            ),
            // `on_key` hands over only the six writing acts; any other opens nothing rather than
            // the delete question.
            _ => return,
        };
        self.mode = mode;
        self.notice = notice;
    }

    /// One key while the fields form is open: the focused field first, then the form's own
    /// navigation through `PERSONAS_FORM`; everything else is swallowed but the chords a modal
    /// mode passes (CONTROL, ALT, function keys).
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
            // The chord the user pressed (MOD-115 M3 MD-A): a `[field]` chord the field passed is
            // that row's, never re-read as another act of its canonical key.
            FieldOutcome::Pass => form_navigation(
                ctx.keys(),
                ctx.pressed(&key),
                &mut editor.focus,
                editor.fields.len(),
            ),
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

    /// One key while the body editor is open: the `TextArea` first, then `form.save` through
    /// `PERSONAS_EDITOR`, the only save: the widget passes `Ctrl+s` like every chord (MOD-67 M4
    /// D6).
    fn on_body_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let cancel = ctx.keys().label(views::PERSONAS_EDITOR, Act::FieldCancel);
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
            // Unreachable since MOD-67 M4 D6: a `TextArea` never submits.
            FieldOutcome::Submit => {
                self.save_body(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.cancel_body(cancel.as_deref());
                Handled::Consumed
            }
            FieldOutcome::Pass => match editor_act(ctx.keys(), ctx.pressed(&key)) {
                Ok(()) => {
                    self.save_body(ctx);
                    Handled::Consumed
                }
                Err(handled) => handled,
            },
        }
    }

    /// `Esc` in the body editor; `cancel` is the key that discards on a second press, as the
    /// warning names it.
    fn cancel_body(&mut self, cancel: Option<&str>) {
        if let Some(busy) = self.own_write() {
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
            self.notice = Some(Notice::Info(unsaved(cancel)));
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

    /// One key while the rules editor is open: as the body editor.
    fn on_rules_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let own = self.own_write();
        let cancel = ctx.keys().label(views::PERSONAS_EDITOR, Act::FieldCancel);
        let Mode::Rules(editor) = &mut self.mode else {
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
            // Unreachable since MOD-67 M4 D6: a `TextArea` never submits.
            FieldOutcome::Submit => {
                self.save_rules(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                if let Some(busy) = own {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                } else if editor.area.text() == editor.original || editor.esc_armed {
                    self.mode = Mode::Browse;
                    self.notice = None;
                } else {
                    editor.esc_armed = true;
                    self.notice = Some(Notice::Info(unsaved(cancel.as_deref())));
                }
                Handled::Consumed
            }
            FieldOutcome::Pass => match editor_act(ctx.keys(), ctx.pressed(&key)) {
                Ok(()) => {
                    self.save_rules(ctx);
                    Handled::Consumed
                }
                Err(handled) => handled,
            },
        }
    }

    /// `Ctrl+S` in the rules editor: the rule lines (D17), then the store's rule over the
    /// permission patch (I-8), then one `UpdatePersona` carrying the permission alone, its
    /// `default` the row's.
    fn save_rules(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Mode::Rules(editor) = &mut self.mode else {
            return;
        };
        let text = editor.area.text().to_owned();
        let rules = match parse_rules(&text) {
            Ok(rules) => rules,
            Err(error) => {
                self.notice = Some(Notice::Error(error.to_string()));
                return;
            }
        };
        if parse_rules(&editor.original).is_ok_and(|original| original == rules) {
            self.mode = Mode::Browse;
            self.notice = Some(Notice::Info(UNCHANGED.to_owned()));
            return;
        }
        let patch = PersonaPatch {
            permission: Some(PersonaPermission {
                default: editor.default,
                rules,
            }),
            ..PersonaPatch::default()
        };
        if let Some(sentence) = persona_patch_refusal(&patch) {
            self.notice = Some(Notice::Error(sentence));
            return;
        }
        let request = StoreRequest::UpdatePersona {
            id: editor.id,
            expected: editor.expected,
            patch,
        };
        editor.sent = Some(text);
        self.notice = None;
        self.send(request, ctx);
    }

    /// One key over the delete question, through `PERSONAS_DELETE`: `confirm.yes` sends,
    /// `confirm.no` closes; once sent, both are swallowed until the answer. A chord a modal mode
    /// passes (CONTROL, ALT, function keys) goes to the shell; everything else is swallowed.
    fn on_delete_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::PERSONAS_DELETE;
        let chord = ctx.pressed(&key);
        let Mode::Deleting { id, stage, .. } = &mut self.mode else {
            return Handled::Pass;
        };
        // The narrowest candidate decides: a global act is the pass rule's.
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ConfirmYes | Act::ConfirmNo) if *stage == DeleteStage::InFlight => {
                Handled::Consumed
            }
            Some(Act::ConfirmYes) => {
                *stage = DeleteStage::InFlight;
                let request = StoreRequest::DeletePersona { id: *id };
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

    /// One key in the import path field (`views::CAPTURE`): the field, then the pass rule.
    fn on_import_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let Mode::ImportPath { field } = &mut self.mode else {
            return Handled::Pass;
        };
        match field.on_key(key) {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                let path = field.text().unwrap_or_default().to_owned();
                if path.trim().is_empty() {
                    self.notice = Some(Notice::Error(ENTER_A_PATH.to_owned()));
                } else if let Some(busy) = self.busy {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                } else {
                    self.mode = Mode::Browse;
                    self.notice = Some(Notice::Info(IMPORTING.to_owned()));
                    self.send(StoreRequest::ImportPersonas { path }, ctx);
                }
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => modal_rest(views::CAPTURE, ctx.pressed(&key)),
        }
    }

    /// One key over the import report, through `PERSONAS_REPORT`: `list.down`/`up` scroll a
    /// line, `common.back` (`Esc`, and `Enter` as a view default) closes. The report stays modal
    /// (L-D Q8): a chord a modal mode passes goes to the shell, everything else is swallowed.
    fn on_report_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::PERSONAS_REPORT;
        let chord = ctx.pressed(&key);
        let Mode::Report { top, max_top, .. } = &mut self.mode else {
            return Handled::Pass;
        };
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ListDown) => *top = (*top + 1).min(max_top.get()),
            Some(Act::ListUp) => *top = top.saturating_sub(1),
            Some(Act::Back) => self.mode = Mode::Browse,
            _ => return modal_rest(stack, chord),
        }
        Handled::Consumed
    }

    /// An import's answer (D20): a clean one is one notice, anything refused, skipped or dropped
    /// opens the report over Browse, or is counted over an editor opened while the walk ran and
    /// kept until that editor closes (R1 M-1).
    fn land_import(&mut self, report: &[PersonaOutcome]) {
        let (imported, refused, skipped, dropped) = counts(report);
        if report.is_empty() {
            self.notice = Some(Notice::Info(NOTHING_IMPORTED.to_owned()));
            return;
        }
        if refused + skipped + dropped == 0 {
            let names: Vec<&str> = report
                .iter()
                .filter_map(|outcome| match outcome {
                    PersonaOutcome::Imported { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            self.notice = Some(Notice::Info(format!("imported {}", names.join(", "))));
            return;
        }
        if matches!(self.mode, Mode::Browse) {
            self.mode = Mode::Report {
                report: report.to_vec(),
                top: 0,
                max_top: Cell::new(usize::MAX),
            };
            self.notice = None;
        } else {
            self.pending_report = Some(report.to_vec());
            self.notice = Some(Notice::Error(format!(
                "imported {imported}{DOT}refused {refused}{DOT}skipped {skipped}{DOT}dropped \
                 {dropped} \u{2014} the report opens when the editor closes"
            )));
        }
    }

    /// Opens a report kept by [`Self::land_import`] once the section is back in Browse.
    fn open_pending_report(&mut self) {
        if matches!(self.mode, Mode::Browse)
            && let Some(report) = self.pending_report.take()
        {
            self.mode = Mode::Report {
                report,
                top: 0,
                max_top: Cell::new(usize::MAX),
            };
        }
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
            })
            | Mode::Rules(RulesEditor { id: open, .. }) => *open == id,
            _ => false,
        }
    }

    /// What one persona write did (D21), once the rows are the re-read's.
    fn on_written(&mut self, outcome: &PersonaWrite, keys: &Keys) {
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
                if self.editing(*id) {
                    // Keys typed while a body or rules save was in flight: the editor stays over
                    // them, its token and baseline at what was saved (the skills rule,
                    // `library.rs`); anything else closes.
                    let stays = match &mut self.mode {
                        Mode::Body(editor) => {
                            if let BodyTarget::Edit { expected, .. } = &mut editor.target
                                && let Some(at) = updated_at
                            {
                                *expected = at;
                            }
                            let stays =
                                keep_unsent(&mut editor.original, &mut editor.sent, &editor.area);
                            editor.esc_armed &= !stays;
                            stays
                        }
                        Mode::Rules(editor) => {
                            if let Some(at) = updated_at {
                                editor.expected = at;
                            }
                            let stays =
                                keep_unsent(&mut editor.original, &mut editor.sent, &editor.area);
                            editor.esc_armed &= !stays;
                            stays
                        }
                        _ => false,
                    };
                    if !stays {
                        self.mode = Mode::Browse;
                    }
                }
                self.notice = Some(Notice::Info(format!(
                    "saved persona `{}`",
                    name.escape_debug()
                )));
            }
            PersonaWrite::Deleted { id } => {
                let notice = match &self.mode {
                    Mode::Deleting {
                        id: asked, name, ..
                    } if asked == id => {
                        format!("deleted persona `{}`", name.escape_debug())
                    }
                    _ => "deleted persona".to_owned(),
                };
                if matches!(&self.mode, Mode::Deleting { id: asked, .. } if asked == id) {
                    self.mode = Mode::Browse;
                }
                self.notice = Some(Notice::Info(notice));
            }
            PersonaWrite::Stale { id } => self.on_stale(*id, keys),
            PersonaWrite::Gone { id } => {
                if self.editing(*id)
                    || matches!(&self.mode, Mode::Deleting { id: asked, .. } if asked == id)
                {
                    self.mode = Mode::Browse;
                    self.notice = Some(Notice::Error(DELETED_ELSEWHERE.to_owned()));
                } else {
                    self.notice = Some(Notice::Error(GONE_CLOSED.to_owned()));
                }
            }
        }
    }

    /// A spent token: rebase an open editor of `id` onto the re-read's row. The body and rules
    /// editors' sentence names their save key as `keys` binds it (MOD-67 M6 D4).
    fn on_stale(&mut self, id: PersonaId, keys: &Keys) {
        let save = keys.label(views::PERSONAS_EDITOR, Act::FormSave);
        // The fields form retries on `field.submit` (MOD-115 M3 D-5).
        let retry = keys.label(views::PERSONAS_FORM, Act::FieldSubmit);
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
                    changed_elsewhere(retry.as_deref())
                } else {
                    clash_notice(&clashes, retry.as_deref())
                }
            }
            Mode::Body(editor) => {
                if editor.area.text() == editor.original {
                    editor.area = at_end(&row.body);
                    // The area's text, as `BodyEditor::edit` takes it (R1 L-4).
                    editor.original = editor.area.text().to_owned();
                }
                if let BodyTarget::Edit { expected, .. } = &mut editor.target {
                    *expected = row.updated_at;
                }
                editor.sent = None;
                editor.esc_armed = false;
                changed_elsewhere_save(save.as_deref())
            }
            Mode::Rules(editor) => {
                let rules = format_rules(&row.permission.rules);
                if editor.area.text() == editor.original {
                    editor.area = at_end(&rules);
                    editor.original = rules;
                }
                editor.expected = row.updated_at;
                editor.default = row.permission.default;
                editor.sent = None;
                editor.esc_armed = false;
                changed_elsewhere_save(save.as_deref())
            }
            Mode::Browse
            | Mode::Deleting { .. }
            | Mode::ImportPath { .. }
            | Mode::Report { .. } => {
                return;
            }
        };
        self.notice = Some(Notice::Error(notice));
    }

    /// The pane above the notice while an editor or the report is open (the list is
    /// [`render_list`](Self::render_list)'s).
    fn body_lines(&self, width: u16, height: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        match &self.mode {
            Mode::Browse | Mode::Deleting { .. } | Mode::ImportPath { .. } => Vec::new(),
            Mode::Editing(editor) => {
                let mut lines = vec![Line::styled(editor.header(), theme.base), Line::default()];
                lines.extend(editor.form().lines(width, ctx.accent(), theme));
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
            Mode::Rules(editor) => {
                let mut lines = vec![Line::styled(
                    format!("rules of `{}`", editor.name),
                    theme.base,
                )];
                lines.extend(
                    wrapped(RULES_HELP, usize::from(width).max(1))
                        .into_iter()
                        .map(|line| Line::styled(line, theme.dim)),
                );
                let used = u16::try_from(lines.len()).unwrap_or(u16::MAX);
                lines.extend(
                    editor
                        .area
                        .lines(width, height.saturating_sub(used), true, theme),
                );
                lines
            }
            Mode::Report {
                report,
                top,
                max_top,
            } => report_lines(report, *top, max_top, width, theme),
        }
    }

    /// The registry as a kit list: one row per persona, the cursor row in the band with its
    /// detail under it as note rows, scrolled so the cursor row and its detail are on screen; or
    /// the empty state that says why there is none and what comes next.
    fn render_list(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        if let Some(why) = &self.unavailable {
            let mut lines: Vec<Line<'static>> = wrapped(
                &format!("{UNAVAILABLE}: {why}"),
                usize::from(area.width).max(1),
            )
            .into_iter()
            .map(|line| notice(&line, Tone::Error, theme))
            .collect();
            // The next step in the empty state's own role.
            lines.extend(empty::lines("", NextStep(UNAVAILABLE_NEXT), theme).pop());
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }
        if !self.read {
            empty::render(frame, area, NOT_READ, NextStep(NOT_READ_NEXT), theme);
            return;
        }
        if self.personas.is_empty() {
            let next = adds_one(ctx.keys());
            empty::render(frame, area, NO_PERSONAS, NextStep(&next), theme);
            return;
        }
        // The list's own width: the gutter takes one cell.
        let width = usize::from(area.width.saturating_sub(1));
        let mut rows = Vec::new();
        let mut cursor_end = 0;
        for (index, row) in self.personas.iter().enumerate() {
            rows.push(list::Row {
                line: Line::styled(row_line(row, width), theme.base),
                kind: RowKind::Item,
            });
            if index == self.cursor {
                let detail = detail_line(row);
                for line in wrapped(&detail, width.saturating_sub(4).max(1)) {
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

    /// The notice row's lines: the import path as a one-row kit form while it is open, then the
    /// last outcome as kit notice lines, wrapped.
    fn notice_lines(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let mut lines = Vec::new();
        if let Mode::ImportPath { field } = &self.mode {
            lines.extend(
                Form::new()
                    .field(form::Field::new("path", Value::Text(field)))
                    .focus(0)
                    .editing(true)
                    .lines(width, ctx.accent(), theme),
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
            wrapped(said.text(), usize::from(width).max(1))
                .into_iter()
                .map(|line| notice(&line, tone, theme)),
        );
        lines
    }

    /// The key line for the mode, rendered through the mode's stack with the keys in force: the
    /// shell's hint row (MOD-67 M6 T2, [`SettingsSection::key_hint`]).
    fn hint_text(&self, keys: &Keys) -> String {
        keys.hint(self.stack(), self.hint_spec())
    }

    /// The hint spec for the mode.
    fn hint_spec(&self) -> HintSpec {
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
            Mode::Body(_) | Mode::Rules(_) => HINT_EDITOR,
            Mode::Deleting { .. } => HINT_DELETING,
            Mode::ImportPath { .. } => HINT_IMPORT,
            Mode::Report { .. } => HINT_REPORT,
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

    /// The fields as a kit form (MOD-115 M3 D3): one row per field in tab order, the focused one
    /// carrying the marker and the cursor, the labels in one shared column (the widest,
    /// `permission-default`, and one).
    fn form(&self) -> Form<'_> {
        self.fields
            .iter()
            .fold(Form::new(), |form, field| {
                form.field(form::Field::new(field.label, Value::Text(&field.input)))
            })
            .focus(self.focus)
            .editing(true)
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

    /// `b` over one row's body, the cursor at the end. The baseline is the area's text, not the
    /// row's: the `TextArea` turns a lone `\r` into a line break, and an untouched body must
    /// still read as unchanged (R1 L-4).
    fn edit(row: &Persona) -> Self {
        let area = at_end(&row.body);
        Self {
            target: BodyTarget::Edit {
                id: row.id,
                name: row.name.clone(),
                expected: row.updated_at,
            },
            original: area.text().to_owned(),
            area,
            esc_armed: false,
            sent: None,
        }
    }
}

impl RulesEditor {
    /// `r` over one row's rules, one per line (D17), the cursor at the end.
    fn edit(row: &Persona) -> Self {
        let original = format_rules(&row.permission.rules);
        Self {
            id: row.id,
            name: row.name.clone(),
            expected: row.updated_at,
            default: row.permission.default,
            area: at_end(&original),
            original,
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
/// drops its segment. Everything is measured in display cells (MOD-54's [`cell_width`]) and the
/// description is cut at a grapheme boundary, so a wide glyph never pushes the tail off the line.
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
        .saturating_sub(cell_width(&row.name))
        .saturating_sub(cell_width(&tail))
        .saturating_sub(cell_width(DOT));
    let description = if cell_width(&row.description) <= room {
        row.description.clone()
    } else if room == 0 {
        return format!("{}{tail}", row.name);
    } else {
        // One cell is the `…`'s; a wide glyph that would pass the edge is left out whole.
        let mut cut = String::new();
        let mut used = 0;
        for grapheme in graphemes(&row.description) {
            let cells = cell_width(grapheme);
            if used + cells > room - 1 {
                break;
            }
            used += cells;
            cut.push_str(grapheme);
        }
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

/// What a form does with a chord its focused field passed on, through `PERSONAS_FORM`:
/// `form.next_field` (`Tab`, and `Down` as a view default) and `form.prev_field` (`BackTab`,
/// `Up`) move the focus with a wrap; then [`modal_rest`].
fn form_navigation(keys: &Keys, chord: KeyChord, focus: &mut usize, fields: usize) -> Handled {
    let stack = views::PERSONAS_FORM;
    let len = fields.max(1);
    match keys.actions(stack, chord).first() {
        Some(Act::FormNextField) => *focus = (*focus + 1) % len,
        Some(Act::FormPrevField) => *focus = (*focus + len - 1) % len,
        _ => return modal_rest(stack, chord),
    }
    Handled::Consumed
}

/// What the body and rules editors do with a key their `TextArea` passed on: `Ok` for
/// `form.save` through `PERSONAS_EDITOR` (the editors' only save: the widget passes `Ctrl+s` like
/// every chord, MOD-67 M4 D6), else `Err` with [`modal_rest`]'s answer.
fn editor_act(keys: &Keys, chord: KeyChord) -> Result<(), Handled> {
    let stack = views::PERSONAS_EDITOR;
    if keys.actions(stack, chord).first() == Some(&Act::FormSave) {
        return Ok(());
    }
    Err(modal_rest(stack, chord))
}

/// [`changed_on_both_sides`] with the clashing labels in form order, as many as fit
/// [`NOTE_WIDTH`], then ` +N more` (a private copy of `agents.rs`' `clash_notice`).
fn clash_notice(clashes: &[&str], retry: Option<&str>) -> String {
    let mut notice = changed_on_both_sides(retry);
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

/// A body or rules save landed: `true` when keys typed while it was in flight changed the text,
/// so the editor stays with the saved text as its baseline; `false` closes it.
fn keep_unsent(original: &mut String, sent: &mut Option<String>, area: &TextArea) -> bool {
    match sent.take() {
        Some(text) if text == area.text() => false,
        Some(text) => {
            *original = text;
            true
        }
        None => area.text() != original.as_str(),
    }
}

/// An import report's counts: imported, refused, skipped, and imported with entries dropped.
fn counts(report: &[PersonaOutcome]) -> (usize, usize, usize, usize) {
    report
        .iter()
        .fold((0, 0, 0, 0), |(i, r, s, d), outcome| match outcome {
            PersonaOutcome::Imported { dropped, .. } => {
                (i + 1, r, s, d + usize::from(!dropped.is_empty()))
            }
            PersonaOutcome::Refused { .. } => (i, r + 1, s, d),
            PersonaOutcome::Skipped { .. } => (i, r, s + 1, d),
        })
}

/// The report: its counts, then one entry per outcome wrapped with a 2-space continuation
/// indent, scrolled from `top` (clamped so the last line stays, which `max_top` remembers for
/// `j`).
fn report_lines(
    report: &[PersonaOutcome],
    top: usize,
    max_top: &Cell<usize>,
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let width = usize::from(width).max(3);
    let (imported, refused, skipped, _) = counts(report);
    let mut entries: Vec<Line<'static>> = Vec::new();
    for outcome in report {
        let (text, style) = match outcome {
            PersonaOutcome::Imported {
                name,
                path,
                dropped,
            } => {
                let mut text = format!("+ {name} ({path})");
                if !dropped.is_empty() {
                    let tools: Vec<String> =
                        dropped.iter().map(|tool| format!("`{tool}`")).collect();
                    text.push_str(&format!(
                        " \u{2014} dropped from `tools`: {} \u{2014} `allow` keeps built-in tools \
                         only",
                        tools.join(", ")
                    ));
                }
                (text, theme.base)
            }
            PersonaOutcome::Refused { path, message } => {
                (format!("! {path} \u{2014} {message}"), theme.error)
            }
            PersonaOutcome::Skipped { path, reason } => {
                (format!("\u{b7} {path} \u{2014} {reason}"), theme.dim)
            }
        };
        let mut first = true;
        let mut rest = String::new();
        for line in wrapped(&text, width) {
            if first {
                entries.push(Line::styled(line, style));
                first = false;
            } else {
                rest.push_str(&line);
                rest.push(' ');
            }
        }
        for line in wrapped(&rest, width - 2) {
            entries.push(Line::styled(format!("  {line}"), style));
        }
    }
    // A pager's rule: the entries scroll until the last line is the first one drawn.
    max_top.set(entries.len().saturating_sub(1));
    let top = top.min(max_top.get());
    let mut lines = vec![Line::styled(
        format!("imported {imported}{DOT}refused {refused}{DOT}skipped {skipped}"),
        theme.base,
    )];
    lines.extend(entries.into_iter().skip(top));
    lines
}

/// The delete question (D14): the store refuses a bound persona, and says which phases hold it.
fn delete_question(name: &str) -> String {
    format!(
        "delete persona `{}`? a persona bound to a phase is refused.",
        name.escape_debug()
    )
}

/// The next step of [`NO_PERSONAS`]: the keys in force that add one (MD-A).
fn adds_one(keys: &Keys) -> String {
    let stack = views::PERSONAS_BROWSE;
    let steps: Vec<String> = [
        keys.label(stack, Act::New)
            .map(|key| format!("{key} adds one")),
        keys.label(stack, Act::PersonasImport)
            .map(|key| format!("{key} imports a file")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if steps.is_empty() {
        "no key in force adds one".to_owned()
    } else {
        steps.join(DOT)
    }
}

/// The refusal of a write key while a write is in flight.
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// A write in flight in words (MOD-115 M3: no request name on screen), by request name.
fn doing(busy: &str) -> &'static str {
    match busy {
        "personas" => "reading the personas",
        "create_persona" => "adding the persona",
        "update_persona" => "saving the persona",
        "delete_persona" => "deleting the persona",
        "import_personas" => "importing personas",
        _ => "writing",
    }
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

    /// The current mode's stack (MOD-67 D4).
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
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
                editor.esc_armed = false;
                if matches!(self.notice, Some(Notice::Info(_))) {
                    self.notice = None;
                }
                Handled::Consumed
            }
            Mode::Rules(editor) => {
                editor.area.on_paste(text);
                editor.esc_armed = false;
                if matches!(self.notice, Some(Notice::Info(_))) {
                    self.notice = None;
                }
                Handled::Consumed
            }
            Mode::ImportPath { field } => {
                field.on_paste(text);
                Handled::Consumed
            }
            Mode::Browse | Mode::Deleting { .. } | Mode::Report { .. } => Handled::Pass,
        }
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let handled = match self.mode {
            Mode::Browse => self.on_browse_key(key, ctx),
            Mode::Editing(_) => self.on_editor_key(key, ctx),
            Mode::Body(_) => self.on_body_key(key, ctx),
            Mode::Rules(_) => self.on_rules_key(key, ctx),
            Mode::Deleting { .. } => self.on_delete_key(key, ctx),
            Mode::ImportPath { .. } => self.on_import_key(key, ctx),
            Mode::Report { .. } => self.on_report_key(key, ctx),
        };
        self.open_pending_report();
        handled
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
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
                self.on_written(outcome, ctx.keys());
            }
            StoreReply::PersonaImports(imports) => {
                if let Ok(rows) = &imports.personas {
                    self.personas.clone_from(rows);
                    self.read = true;
                    self.unavailable = None;
                    self.clamp();
                }
                if self.busy == Some(IMPORT_NAME) {
                    self.busy = None;
                    self.land_import(&imports.report);
                    if let Err(why) = &imports.personas {
                        // R1 L-2: the report landed; the list is the one before the import.
                        let why = format!("the list was not re-read: {why}");
                        self.notice = Some(Notice::Error(match self.notice.take() {
                            Some(notice) => format!("{}{DOT}{why}", notice.text()),
                            None => why,
                        }));
                    }
                }
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
                // The store's sentence, bare (B-11); every editor stays open over its text, and the
                // delete question closes.
                self.busy = None;
                if matches!(self.mode, Mode::Deleting { .. }) {
                    self.mode = Mode::Browse;
                }
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
        self.open_pending_report();
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let notice = self.notice_lines(area.width, ctx);
        // MOD-67 M6 T2: the keys row went to the shell's hint row; the pane gains it.
        let [pane, notice_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(notice.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        self.scroll.set(None);
        match &self.mode {
            Mode::Browse | Mode::Deleting { .. } | Mode::ImportPath { .. } => {
                self.render_list(frame, pane, ctx);
            }
            Mode::Editing(_) | Mode::Body(_) | Mode::Rules(_) | Mode::Report { .. } => {
                frame.render_widget(
                    Paragraph::new(self.body_lines(pane.width, pane.height, ctx)),
                    pane,
                );
            }
        }
        // The report scrolls with `j`/`k`: its counts line and its entries, from the clamped top.
        if let Mode::Report {
            report,
            top,
            max_top,
        } = &self.mode
        {
            let entries = if report.is_empty() {
                0
            } else {
                max_top.get() + 1
            };
            self.scroll.set(Some(ScrollSpan::new(
                1 + entries,
                (*top).min(max_top.get()),
            )));
        }
        // The pane's bar is measured against the rows the list or report was drawn into and
        // sits beside them (RV1 M2's span): the notice rows under them do not scroll.
        if let Some(span) = self.scroll.get() {
            self.scroll.set(Some(span.rows(pane)));
        }
        if !notice.is_empty() {
            frame.render_widget(Paragraph::new(notice), notice_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-67 M6 D4: the stale body/rules notice names the save key, and says a save unbound.
    #[test]
    fn changed_elsewhere_save_names_the_save_key_or_none() {
        assert_eq!(
            changed_elsewhere_save(Some("Ctrl+s")),
            "changed elsewhere since you opened it \u{2014} reloaded; Ctrl+s retries against the \
             current row"
        );
        assert_eq!(
            changed_elsewhere_save(None),
            "changed elsewhere since you opened it \u{2014} reloaded; save again to retry against \
             the current row"
        );
    }

    /// MOD-115 M3: a write in flight reads in words: every request name has its own phrase, and
    /// the refusal never prints the name itself.
    #[test]
    fn a_write_in_flight_reads_in_words() {
        for name in REQUEST_NAMES {
            assert_ne!(doing(name), doing("unknown"), "`{name}` has its own phrase");
            assert!(!in_flight(name).contains('_'), "{}", in_flight(name));
        }
    }
}
