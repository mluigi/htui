//! A workspace's or a project's owner documents pane (MOD-147 M3 D9, D10): every version of the
//! owner's documents, one version's body, and a new document or a new version written through
//! `$EDITOR`, opened from a workspace or project row of Settings › Hierarchy.
//!
//! Self-contained, as the memory pane ([`memory`]) is: the Hierarchy section holds one
//! [`OwnerDocumentsPane`] in its `Mode::Documents` and hands it every key, paste, reply, `$EDITOR`
//! outcome and frame while it is open. It holds no store handle (`R-NF-3`): the read and the
//! write leave through `ctx.request` for [`crate::owner_documents::serve`].
//!
//! The list is one row per version, `kind · vN · title · date`, ordered by kind and newest
//! version first. A workspace's pane lists `goals` first ([`GOALS_KIND`], D8); with none written
//! yet its row reads [`GOALS_NONE_YET`] and `n` there pre-fills the kind. `Enter` shows the
//! selected version's body, read-only and scrolling; `Esc` goes back. `n` opens the form (kind and
//! title), whose `Enter` hands an empty body to `$EDITOR`; `v` opens it with the row's kind fixed
//! and its title, and hands that version's body. An edited body is written as the kind's next
//! version; an unchanged one writes nothing and says [`NO_CHANGES`].
//!
//! A write that did not land keeps its draft (review M2, the Backlog compose's rule): kind, title
//! and the edited text. A refusal (offline, a constraint, no scope row) says so at once; any other
//! failure may follow a COMMIT whose answer was lost, so the pane re-reads the list and reports
//! the write saved when the kind's newest version is newer than before it, else keeps the draft.
//! While a draft is kept, `Enter`, `n` and `v` reopen `$EDITOR` on its text, and closing it
//! untouched is the retry. A list read offline says no write is taken (`writable`), and `n`/`v`
//! refuse before `$EDITOR` opens.
//!
//! Every hint and every sentence that names a key reads it from the stack in force, and every act
//! resolves from [`Ctx::pressed`].
//!
//! [`memory`]: crate::ui::tabs::settings::memory
//! [`OwnerDocumentsPane`]: crate::ui::tabs::settings::owner_documents::OwnerDocumentsPane
//! [`GOALS_KIND`]: htui_core::model::document::GOALS_KIND
//! [`GOALS_NONE_YET`]: crate::ui::tabs::settings::owner_documents::GOALS_NONE_YET
//! [`NO_CHANGES`]: crate::editor::NO_CHANGES
//! [`Ctx::pressed`]: crate::app::Ctx::pressed

use core::cell::Cell;
use core::cmp::Reverse;

use crossterm::event::KeyEvent;
use htui_core::model::document::GOALS_KIND;
use htui_core::model::hand_written as rules;
use htui_core::model::{DocumentId, OwnerDocument};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled};
use crate::editor::{
    ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG, strip_added_newline, without_controls,
};
use crate::hand_written::HandText;
use crate::hand_written::answered_from_the_mirror;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::owner_documents::{
    DocumentOwner, READ_NAME, WRITE_NAME, offline_refusal, write_refused,
};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form, Value};
use crate::ui::kit::list::{self, RowKind};
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::markdown;
use crate::ui::tabs::backlog::detail::compose::{could_not_check, may_have_landed, not_written};
use crate::ui::{FieldOutcome, TextField, Theme};

pub use crate::ui::tabs::settings::memory::PaneKey;

/// A workspace pane's `goals` row while no goals document exists (D8).
pub const GOALS_NONE_YET: &str = "goals \u{2014} none yet";

/// Before the first read answered.
pub const NOT_READ: &str = "documents not read yet";

/// A readable owner with no document.
pub const NO_DOCUMENTS: &str = "no documents yet";

/// `v` or `Enter` with no version under the cursor.
pub const NO_ROW: &str = "no document version is selected";

/// The opening of a refused read, drawn `{UNAVAILABLE}: {message}`.
pub const UNAVAILABLE: &str = "documents unavailable";

/// A write key while a write is in flight.
pub const STILL_SAVING: &str = "still saving the document \u{2014} wait for the store's answer";

/// A write key while `$EDITOR` holds a body the pane handed out.
pub const EDITOR_OPEN: &str = "the editor is still open on a document";

/// The next step of [`NOT_READ`].
const NOT_READ_NEXT: &str = "the documents appear once the store answers";

/// The separator of a row's segments.
const DOT: &str = " \u{b7} ";

/// The form's rows, in tab order.
const KIND_ROW: usize = 0;
/// The title row.
const TITLE_ROW: usize = 1;

/// The Browse keys over a list with rows: `j/k select · Enter view · n new · v new version · Esc
/// back` with the default keys.
pub const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "select"),
    Hint::One(Act::DocumentsView, "view"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::DocumentsNewVersion, "new version"),
    Hint::One(Act::Back, "back"),
];

/// The Browse keys over an empty, readable list: `n new · Esc back`.
pub const HINT_EMPTY: HintSpec = &[Hint::One(Act::New, "new"), Hint::One(Act::Back, "back")];

/// The Browse keys while the documents cannot be read: `Esc back`.
pub const HINT_UNAVAILABLE: HintSpec = &[Hint::One(Act::Back, "back")];

/// One version's body: `j/k scroll · Esc back`.
pub const HINT_VIEW: HintSpec = &[
    Hint::Pair(Act::PaneScrollDown, Act::PaneScrollUp, "scroll"),
    Hint::One(Act::Back, "back"),
];

/// The form with both rows open: `Tab/Shift+Tab field · Enter $EDITOR · Esc cancel`.
pub const HINT_FORM: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::One(Act::FieldSubmit, "$EDITOR"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The form with its kind fixed (`v`): `Enter $EDITOR · Esc cancel`.
pub const HINT_FORM_FIXED: HintSpec = &[
    Hint::One(Act::FieldSubmit, "$EDITOR"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// One row of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// A workspace's `goals` before one exists (D8).
    GoalsNoneYet,
    /// Index into the sorted documents.
    Version(usize),
}

/// What the pane is doing.
#[derive(Debug, Default)]
enum Mode {
    /// The list and the cursor.
    #[default]
    Browse,
    /// `Enter`: one version's body, read-only, from `offset`.
    Viewing {
        /// The version shown.
        id: DocumentId,
        /// The first body row drawn.
        offset: usize,
    },
    /// `n` / `v`: kind and title, then `$EDITOR`.
    Form(DocumentForm),
}

/// The form: the kind (typed, or fixed by `v`), the title, and the body `$EDITOR` opens on.
/// `Debug` hand-written: never the typed text or the body.
struct DocumentForm {
    kind: TextField,
    /// `v`: the row's kind, which the form does not let change.
    fixed: Option<String>,
    title: TextField,
    body: String,
    /// [`KIND_ROW`] or [`TITLE_ROW`].
    focus: usize,
}

impl core::fmt::Debug for DocumentForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DocumentForm")
            .field("fixed", &self.fixed)
            .field("body_len", &self.body.len())
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl DocumentForm {
    /// `n`: an empty form, the kind pre-filled with `kind` (the `goals` row's, D8).
    fn new_document(kind: &str) -> Self {
        Self {
            kind: TextField::with_text(kind),
            fixed: None,
            title: TextField::new(),
            body: String::new(),
            focus: if kind.is_empty() { KIND_ROW } else { TITLE_ROW },
        }
    }

    /// `v`: a new version of `document`, its kind fixed, its title and body handed on.
    fn new_version(document: &OwnerDocument) -> Self {
        Self {
            kind: TextField::with_text(&document.kind),
            fixed: Some(document.kind.clone()),
            title: TextField::with_text(&document.title),
            body: document.body.clone(),
            focus: TITLE_ROW,
        }
    }

    fn kind_text(&self) -> &str {
        match &self.fixed {
            Some(kind) => kind,
            None => self.kind.text().unwrap_or_default(),
        }
    }

    fn title_text(&self) -> &str {
        self.title.text().unwrap_or_default()
    }

    fn focused(&mut self) -> &mut TextField {
        if self.focus == KIND_ROW && self.fixed.is_none() {
            &mut self.kind
        } else {
            &mut self.title
        }
    }

    /// The pane's first line.
    fn header(&self) -> String {
        match &self.fixed {
            Some(kind) => format!("new version of `{kind}`"),
            None => "new document".to_owned(),
        }
    }

    /// The two rows as a kit form.
    fn form(&self) -> Form<'_> {
        let kind = match &self.fixed {
            Some(kind) => Value::ReadOnly(kind.as_str().into()),
            None => Value::Text(&self.kind),
        };
        Form::new()
            .field(form::Field::new("kind", kind))
            .field(form::Field::new("title", Value::Text(&self.title)))
            .focus(self.focus)
            .editing(true)
    }
}

/// What `$EDITOR` was handed, kept until its outcome comes back (D10), and, once edited, the
/// text a write sent or a failed write kept (review M2). `Debug` hand-written: lengths only.
struct Draft {
    kind: String,
    title: String,
    /// The text `$EDITOR` holds: the form's body, or a kept draft's edited text.
    handed: String,
    /// The text that counts as no change: the form's body (empty for `n`, the version's for
    /// `v`). A kept draft's `handed` differs from it, so closing `$EDITOR` untouched retries.
    base: String,
}

impl Draft {
    /// The draft holds text no write has stored yet.
    fn is_kept_text(&self) -> bool {
        self.handed != self.base
    }
}

impl core::fmt::Debug for Draft {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Draft")
            .field("kind", &self.kind)
            .field("handed_len", &self.handed.len())
            .field("base_len", &self.base.len())
            .finish_non_exhaustive()
    }
}

/// A write in flight (review M2): its draft, and the kind's newest version before it, which an
/// ambiguous failure's re-read compares against.
#[derive(Debug)]
struct Sent {
    draft: Draft,
    before: i32,
}

/// An ambiguous write failure whose re-read is in flight (review M2).
#[derive(Debug)]
struct Hedged {
    sent: Sent,
    /// The failure's message.
    why: String,
    /// The failure was an `Unreachable`, so the re-read is the mirror's, which cannot hold a
    /// write whose answer was lost: not finding it there proves nothing.
    mirror: bool,
}

/// One line on the notice row.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// A report.
    Info(String),
    /// One the user has to act on, drawn in `theme.error`.
    Error(String),
}

/// What a form key asks for once the form's borrow ends.
enum FormStep {
    Done(Handled),
    Submit,
    Cancel,
}

/// One owner's documents, with the keys that write them (MOD-147 M3 D9).
#[derive(Debug)]
pub struct OwnerDocumentsPane {
    /// Whose documents.
    owner: DocumentOwner,
    /// Its slug, for the title and the editor's file name.
    slug: String,
    /// The last list a reply carried, in display order; `None` until one did.
    documents: Option<Vec<OwnerDocument>>,
    /// `Failed { owner_documents }`: the read was refused.
    refused: Option<String>,
    /// Index into [`rows`](Self::rows); no wrap.
    cursor: usize,
    /// What the pane is doing.
    mode: Mode,
    /// The last list's `writable`: `false` offline, where `n` and `v` refuse (review M2).
    writable: bool,
    /// The write in flight.
    sent: Option<Sent>,
    /// An ambiguous write failure, until the re-read settles it.
    hedged: Option<Hedged>,
    /// A draft whose write did not land: `Enter`, `n` and `v` reopen `$EDITOR` on it.
    kept: Option<Draft>,
    /// The version a landed write minted: the next list's cursor goes to it.
    follow: Option<(String, i32)>,
    /// What `$EDITOR` holds (D10).
    pending: Option<Draft>,
    /// The last outcome, drawn under the pane.
    notice: Option<Notice>,
    /// The list's or the body's `(lines, first line drawn)` as of the last render.
    scroll: Cell<Option<ScrollSpan>>,
    /// The body view's `(width, height)` as of the last render, for its scroll clamp.
    body_area: Cell<(u16, u16)>,
}

impl OwnerDocumentsPane {
    /// Opens on `owner` (`slug` for the title) and answers the read to send:
    /// `OwnerDocuments(owner)`.
    #[must_use]
    pub fn open(owner: DocumentOwner, slug: String) -> (Self, StoreRequest) {
        let pane = Self {
            owner,
            slug,
            documents: None,
            refused: None,
            cursor: 0,
            mode: Mode::Browse,
            writable: true,
            sent: None,
            hedged: None,
            kept: None,
            follow: None,
            pending: None,
            notice: None,
            scroll: Cell::new(None),
            body_area: Cell::new((0, 0)),
        };
        let read = pane.read_request();
        (pane, read)
    }

    /// Whose documents the pane lists.
    #[must_use]
    pub const fn owner(&self) -> DocumentOwner {
        self.owner
    }

    /// The read the host re-sends on activation.
    #[must_use]
    pub fn read_request(&self) -> StoreRequest {
        StoreRequest::OwnerDocuments(self.owner)
    }

    /// The stack of the current mode: `key_stack`, `on_key` and the hint all read it.
    #[must_use]
    pub fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::DOCUMENTS_BROWSE,
            Mode::Viewing { .. } => views::DOCUMENTS_VIEW,
            Mode::Form(_) => views::DOCUMENTS_FORM,
        }
    }

    /// The mode's keys for the host's hint row, through the mode's stack.
    #[must_use]
    pub fn hint_text(&self, keys: &Keys) -> String {
        keys.hint(self.stack(), self.hint_spec())
    }

    fn hint_spec(&self) -> HintSpec {
        match &self.mode {
            Mode::Browse if self.refused.is_some() => HINT_UNAVAILABLE,
            Mode::Browse if self.rows().is_empty() => HINT_EMPTY,
            Mode::Browse => HINT_BROWSE,
            Mode::Viewing { .. } => HINT_VIEW,
            Mode::Form(form) if form.fixed.is_some() => HINT_FORM_FIXED,
            Mode::Form(_) => HINT_FORM,
        }
    }

    /// The scroll as of the last render.
    #[must_use]
    pub fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    fn versions(&self) -> &[OwnerDocument] {
        self.documents.as_deref().unwrap_or_default()
    }

    /// The rows on screen: a workspace's `goals` placeholder first while it has none (D8), then
    /// every version.
    fn rows(&self) -> Vec<Row> {
        let Some(documents) = &self.documents else {
            return Vec::new();
        };
        let placeholder = matches!(self.owner, DocumentOwner::Workspace(_))
            && !documents.iter().any(|row| row.kind == GOALS_KIND);
        placeholder
            .then_some(Row::GoalsNoneYet)
            .into_iter()
            .chain((0..documents.len()).map(Row::Version))
            .collect()
    }

    fn selected(&self) -> Option<Row> {
        self.rows().get(self.cursor).copied()
    }

    fn selected_version(&self) -> Option<&OwnerDocument> {
        match self.selected()? {
            Row::Version(index) => self.versions().get(index),
            Row::GoalsNoneYet => None,
        }
    }

    /// The list in display order: a workspace's `goals` first (D8), then kind byte order, the
    /// newest version first.
    fn ordered(&self, documents: &[OwnerDocument]) -> Vec<OwnerDocument> {
        let goals_first = matches!(self.owner, DocumentOwner::Workspace(_));
        let mut documents = documents.to_vec();
        documents.sort_by(|left, right| {
            order_key(left, goals_first).cmp(&order_key(right, goals_first))
        });
        documents
    }

    /// A write is in flight, or its ambiguous failure is being checked.
    const fn busy(&self) -> bool {
        self.sent.is_some() || self.hedged.is_some()
    }

    /// The newest version of `kind` in the list, `0` with none.
    fn newest(&self, kind: &str) -> i32 {
        self.versions()
            .iter()
            .filter(|row| row.kind == kind)
            .map(|row| row.version)
            .max()
            .unwrap_or(0)
    }

    /// Why a write key is refused now, if it is.
    fn write_refusal(&self) -> Option<String> {
        if let Some(why) = &self.refused {
            return Some(format!("{UNAVAILABLE}: {why}"));
        }
        if self.documents.is_none() {
            return Some(NOT_READ.to_owned());
        }
        if !self.writable {
            return Some(format!("not saved: {}", offline_refusal()));
        }
        if self.busy() {
            return Some(STILL_SAVING.to_owned());
        }
        self.pending.as_ref().map(|_| EDITOR_OPEN.to_owned())
    }

    /// One key (the host hands every key while the pane is open).
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> PaneKey {
        let handled = match self.mode {
            Mode::Browse => return self.on_browse_key(key, ctx),
            Mode::Viewing { .. } => self.on_view_key(key, ctx),
            Mode::Form(_) => self.on_form_key(key, ctx),
        };
        PaneKey::Handled(handled)
    }

    /// One key in Browse, through `DOCUMENTS_BROWSE`.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> PaneKey {
        let stack = views::DOCUMENTS_BROWSE;
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
                // Review M2: a kept draft is what these keys reopen.
                Act::DocumentsView | Act::New | Act::DocumentsNewVersion if self.kept.is_some() => {
                    self.reopen(ctx);
                }
                Act::DocumentsView => match self.selected_version() {
                    Some(document) => {
                        self.mode = Mode::Viewing {
                            id: document.id,
                            offset: 0,
                        };
                        self.notice = None;
                    }
                    None => self.notice = Some(Notice::Error(NO_ROW.to_owned())),
                },
                Act::New | Act::DocumentsNewVersion => self.open_form(act),
                _ => continue,
            }
            return PaneKey::Handled(Handled::Consumed);
        }
        PaneKey::Handled(modal_rest(stack, chord))
    }

    /// `n` or `v`: refused in place while a write is in flight, the editor is open, or there is
    /// no list to write to.
    fn open_form(&mut self, act: Act) {
        if let Some(why) = self.write_refusal() {
            self.notice = Some(Notice::Error(why));
            return;
        }
        let form = match (act, self.selected()) {
            // D8: on the `goals` row, both pre-fill the kind.
            (_, Some(Row::GoalsNoneYet)) => DocumentForm::new_document(GOALS_KIND),
            (Act::New, _) => DocumentForm::new_document(""),
            (_, Some(Row::Version(index))) => match self.versions().get(index) {
                Some(document) => DocumentForm::new_version(document),
                None => return,
            },
            (_, None) => {
                self.notice = Some(Notice::Error(NO_ROW.to_owned()));
                return;
            }
        };
        self.mode = Mode::Form(form);
        self.notice = None;
    }

    /// Review M2: `$EDITOR` again on the kept draft's text, refused in place as `n` is.
    fn reopen(&mut self, ctx: &Ctx<'_>) {
        if let Some(why) = self.write_refusal() {
            self.notice = Some(Notice::Error(why));
            return;
        }
        let Some(draft) = self.kept.take() else {
            return;
        };
        ctx.emit(Action::EditExternally(ExternalEdit {
            text: draft.handed.clone(),
            stem: format!("{}-{}", self.slug, draft.kind),
        }));
        self.pending = Some(draft);
        self.notice = None;
    }

    /// One key over a version's body, through `DOCUMENTS_VIEW`.
    fn on_view_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::DOCUMENTS_VIEW;
        let chord = ctx.pressed(&key);
        let max = self.max_offset();
        let (_, height) = self.body_area.get();
        let page = usize::from(height.saturating_sub(1)).max(1);
        let Mode::Viewing { offset, .. } = &mut self.mode else {
            return Handled::Pass;
        };
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::PaneScrollDown) => *offset = (*offset + 1).min(max),
            Some(Act::PaneScrollUp) => *offset = offset.saturating_sub(1),
            Some(Act::PanePageDown) => *offset = (*offset + page).min(max),
            Some(Act::PanePageUp) => *offset = offset.saturating_sub(page),
            Some(Act::Back) => {
                self.mode = Mode::Browse;
                self.notice = None;
            }
            _ => return modal_rest(stack, chord),
        }
        Handled::Consumed
    }

    /// The document the body view shows, if it is still listed.
    fn viewed(&self) -> Option<&OwnerDocument> {
        let Mode::Viewing { id, .. } = &self.mode else {
            return None;
        };
        self.versions().iter().find(|row| row.id == *id)
    }

    /// The furthest the body view scrolls, as of the last render.
    fn max_offset(&self) -> usize {
        let (width, height) = self.body_area.get();
        self.viewed().map_or(0, |document| {
            markdown::row_count(&document.body, width).saturating_sub(usize::from(height))
        })
    }

    /// One key in the form, through `DOCUMENTS_FORM`: the focused field first, then the form's
    /// moves.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::DOCUMENTS_FORM;
        let chord = ctx.pressed(&key);
        let step = {
            let Mode::Form(form) = &mut self.mode else {
                return Handled::Pass;
            };
            match form.focused().on_key(key) {
                FieldOutcome::Consumed => FormStep::Done(Handled::Consumed),
                FieldOutcome::Submit => FormStep::Submit,
                FieldOutcome::Cancel => FormStep::Cancel,
                FieldOutcome::Pass => match ctx.keys().actions(stack, chord).first() {
                    Some(Act::FormNextField | Act::FormPrevField) => {
                        if form.fixed.is_none() {
                            form.focus = if form.focus == KIND_ROW {
                                TITLE_ROW
                            } else {
                                KIND_ROW
                            };
                        }
                        FormStep::Done(Handled::Consumed)
                    }
                    _ => FormStep::Done(modal_rest(stack, chord)),
                },
            }
        };
        match step {
            FormStep::Done(handled) => handled,
            FormStep::Submit => {
                self.submit_form(ctx);
                Handled::Consumed
            }
            FormStep::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
        }
    }

    /// `Enter` in the form: the kind and title are checked as the store checks them, then the
    /// body goes to `$EDITOR` (D10). A refusal stays in the form, and nothing is handed out.
    fn submit_form(&mut self, ctx: &Ctx<'_>) {
        let Mode::Form(form) = &mut self.mode else {
            return;
        };
        let kind = match rules::document_kind(form.kind_text()) {
            Ok(kind) => kind,
            Err(error) => {
                form.focus = KIND_ROW;
                self.notice = Some(Notice::Error(format!("kind: {error}")));
                return;
            }
        };
        let title = match rules::document_title(form.title_text()) {
            Ok(title) => title,
            Err(error) => {
                form.focus = TITLE_ROW;
                self.notice = Some(Notice::Error(format!("title: {error}")));
                return;
            }
        };
        let Mode::Form(form) = core::mem::take(&mut self.mode) else {
            return;
        };
        ctx.emit(Action::EditExternally(ExternalEdit {
            text: form.body.clone(),
            stem: format!("{}-{kind}", self.slug),
        }));
        self.pending = Some(Draft {
            kind,
            title,
            handed: form.body.clone(),
            base: form.body,
        });
        self.notice = None;
    }

    /// D10: what `$EDITOR` did with the body the form handed it. `Edited` writes the kind's next
    /// version; `Unchanged` writes nothing and says so; `Failed` says its sentence. Ignored unless
    /// the pane handed a body out.
    ///
    /// Review M2: a reopened kept draft closed untouched is the retry, and writes its text (a
    /// `quick` return keeps it, the editor did not wait); a refused body or an editor that failed
    /// keeps the edited text rather than dropping it.
    pub fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        let Some(draft) = self.pending.take() else {
            return;
        };
        let reopen = reopen_label(ctx.keys());
        match outcome {
            ExternalEditOutcome::Edited(returned) => {
                let returned = without_controls(&returned);
                let text = strip_added_newline(&draft.handed, &returned).to_owned();
                if text == draft.base {
                    self.notice = Some(Notice::Info(NO_CHANGES.to_owned()));
                    return;
                }
                let error = rules::document_body(&text).err();
                let draft = Draft {
                    handed: text,
                    ..draft
                };
                match error {
                    Some(error) => self.keep(
                        draft,
                        &format!("body: {error}; nothing was written"),
                        reopen.as_deref(),
                    ),
                    None => self.send(draft, ctx),
                }
            }
            ExternalEditOutcome::Unchanged { quick } => {
                if draft.is_kept_text() && !quick {
                    self.send(draft, ctx);
                    return;
                }
                let wait = if quick { WAIT_FLAG } else { "" };
                let said = format!("{NO_CHANGES}{wait}");
                if draft.is_kept_text() {
                    self.keep(draft, &said, reopen.as_deref());
                } else {
                    self.notice = Some(Notice::Info(said));
                }
            }
            ExternalEditOutcome::Failed(message) => {
                if draft.is_kept_text() {
                    self.keep(draft, &message, reopen.as_deref());
                } else {
                    self.notice = Some(Notice::Error(message));
                }
            }
        }
    }

    /// Sends `draft` as the kind's next version, noting the kind's newest version before it.
    fn send(&mut self, draft: Draft, ctx: &Ctx<'_>) {
        let before = self.newest(&draft.kind);
        self.notice = None;
        ctx.request(StoreRequest::WriteOwnerDocument {
            owner: self.owner,
            kind: draft.kind.clone(),
            title: draft.title.clone(),
            body: HandText::new(draft.handed.clone()),
        });
        self.sent = Some(Sent { draft, before });
    }

    /// Review M2: keeps `draft` for `Enter`, `n` or `v`, saying `head` and how to reopen it.
    fn keep(&mut self, draft: Draft, head: &str, reopen: Option<&str>) {
        self.notice = Some(Notice::Error(kept_sentence(head, reopen)));
        self.kept = Some(draft);
    }

    /// Review M2: the re-read after an ambiguous failure. The kind's newest version past the one
    /// before the write means it landed; otherwise the draft is kept, and, read from the mirror,
    /// the sentence says it could not be checked.
    fn settle(&mut self, hedged: Hedged, keys: &Keys) {
        let Hedged { sent, why, mirror } = hedged;
        let kind = sent.draft.kind.clone();
        let newest = self.newest(&kind);
        if newest > sent.before {
            self.notice = Some(Notice::Info(format!("saved {kind}{DOT}v{newest}")));
            self.follow = Some((kind, newest));
            return;
        }
        let head = if mirror {
            format!(
                "{}; saving it again may write it twice",
                could_not_check(&why, "list", None)
            )
        } else {
            not_written(&why, None)
        };
        self.keep(sent.draft, &head, reopen_label(keys).as_deref());
    }

    /// A bracketed paste into the form's focused field; dropped anywhere else.
    pub fn on_paste(&mut self, text: &str) -> Handled {
        if let Mode::Form(form) = &mut self.mode {
            form.focused().on_paste(text);
        }
        Handled::Consumed
    }

    /// One reply. A reply for another owner is ignored; a write's `Failed` lands only on the
    /// write in flight. Review M2: a refused write keeps its draft; any other failure re-reads
    /// the list, and that list settles it.
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::OwnerDocuments {
                owner,
                documents,
                writable,
            } if *owner == self.owner => {
                let prior = self
                    .selected_version()
                    .map(|row| (row.kind.clone(), row.version));
                self.documents = Some(self.ordered(documents));
                self.refused = None;
                self.writable = *writable;
                if let Some(hedged) = self.hedged.take() {
                    self.settle(hedged, ctx.keys());
                }
                let keep = self.follow.take().or(prior);
                self.reselect(keep);
            }
            StoreReply::OwnerDocumentWritten {
                owner,
                kind,
                version,
            } if *owner == self.owner => {
                self.sent = None;
                self.hedged = None;
                self.follow = Some((kind.clone(), *version));
                self.notice = Some(Notice::Info(format!("saved {kind}{DOT}v{version}")));
                ctx.request(self.read_request());
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                match self.hedged.take() {
                    // The check itself failed: the list stays as it was, the draft is kept.
                    Some(Hedged { sent, why, .. }) => {
                        let head = format!(
                            "{}; saving it again may write it twice",
                            could_not_check(&why, "list", None)
                        );
                        self.keep(sent.draft, &head, reopen_label(ctx.keys()).as_deref());
                    }
                    None => self.refused = Some(message.clone()),
                }
            }
            StoreReply::Failed { request, message } if *request == WRITE_NAME => {
                let Some(sent) = self.sent.take() else {
                    return;
                };
                if write_refused(message) {
                    self.keep(
                        sent.draft,
                        &format!("not saved: {message}"),
                        reopen_label(ctx.keys()).as_deref(),
                    );
                } else {
                    self.notice = Some(Notice::Info(may_have_landed(message, "list")));
                    self.hedged = Some(Hedged {
                        sent,
                        why: message.clone(),
                        mirror: answered_from_the_mirror(message),
                    });
                    ctx.request(self.read_request());
                }
            }
            _ => {}
        }
    }

    /// After a new list: the cursor follows `keep` (kind and version) to its row, else stays
    /// where it was, clamped.
    fn reselect(&mut self, keep: Option<(String, i32)>) {
        let rows = self.rows();
        if let Some((kind, version)) = keep
            && let Some(at) = rows.iter().position(|row| match row {
                Row::Version(index) => self
                    .versions()
                    .get(*index)
                    .is_some_and(|row| row.kind == kind && row.version == version),
                Row::GoalsNoneYet => false,
            })
        {
            self.cursor = at;
        }
        self.cursor = self.cursor.min(rows.len().saturating_sub(1));
        if matches!(self.mode, Mode::Viewing { .. }) && self.viewed().is_none() {
            self.mode = Mode::Browse;
        }
    }

    /// Draws the pane: its title, the list, the body or the form, and the notice.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let notice = self.notice_lines(area.width, theme);
        let [title, pane, notice_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(u16::try_from(notice.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("documents{DOT}{}", self.slug),
                theme.title,
            )),
            title,
        );
        self.scroll.set(None);
        match &self.mode {
            Mode::Browse => self.render_list(frame, pane, ctx),
            Mode::Viewing { offset, .. } => self.render_body(frame, pane, *offset, theme),
            Mode::Form(form) => {
                let mut lines = vec![Line::styled(form.header(), theme.base), Line::default()];
                lines.extend(form.form().lines(pane.width, ctx.accent(), theme));
                frame.render_widget(Paragraph::new(lines), pane);
            }
        }
        if self.pending.is_some() {
            // D10: an in-pane editor draws over the pane it was asked from.
            ctx.claim_editor_area(pane);
        }
        if !notice.is_empty() {
            frame.render_widget(Paragraph::new(notice), notice_area);
        }
    }

    /// One version's heading, then its body as Markdown from `offset`.
    fn render_body(&self, frame: &mut Frame<'_>, area: Rect, offset: usize, theme: &Theme) {
        let Some(document) = self.viewed() else {
            return;
        };
        let [heading, body] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(area);
        frame.render_widget(
            Paragraph::new(Line::styled(row_line(document), theme.base)),
            heading,
        );
        self.body_area.set((body.width, body.height));
        let lines = markdown::lines(&document.body, body.width, theme);
        let total = lines.len();
        let offset = offset.min(total.saturating_sub(usize::from(body.height)));
        frame.render_widget(
            Paragraph::new(lines.into_iter().skip(offset).collect::<Vec<_>>()),
            body,
        );
        self.scroll
            .set(Some(ScrollSpan::new(total, offset).rows(body)));
    }

    /// The versions as a kit list, or the state that says why there is none.
    fn render_list(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
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
        if self.documents.is_none() {
            empty::render(frame, area, NOT_READ, NextStep(NOT_READ_NEXT), theme);
            return;
        }
        let rows = self.rows();
        if rows.is_empty() {
            let next = adds_one(ctx.keys());
            empty::render(frame, area, NO_DOCUMENTS, NextStep(&next), theme);
            return;
        }
        let lines: Vec<list::Row> = rows
            .iter()
            .map(|row| {
                let (text, style) = match row {
                    Row::GoalsNoneYet => (GOALS_NONE_YET.to_owned(), theme.dim),
                    Row::Version(index) => (
                        self.versions()
                            .get(*index)
                            .map(row_line)
                            .unwrap_or_default(),
                        theme.base,
                    ),
                };
                list::Row {
                    line: Line::styled(text, style),
                    kind: RowKind::Item,
                }
            })
            .collect();
        let total = lines.len();
        let offset = list::scroll_into_view(self.cursor, 0, usize::from(area.height), total);
        list::render(
            frame,
            area,
            &lines,
            Some(self.cursor),
            offset,
            ctx.accent(),
            theme,
        );
        self.scroll.set(Some(ScrollSpan::new(total, offset)));
    }

    /// The notice row's lines, wrapped.
    fn notice_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let Some(said) = &self.notice else {
            return Vec::new();
        };
        let (text, tone) = match said {
            Notice::Info(text) => (text, Tone::Info),
            Notice::Error(text) => (text, Tone::Error),
        };
        wrapped(text, usize::from(width).max(1))
            .into_iter()
            .map(|line| notice(&line, tone, theme))
            .collect()
    }
}

/// One list row: `kind · vN · title · date` (D9).
fn row_line(document: &OwnerDocument) -> String {
    format!(
        "{}{DOT}v{}{DOT}{}{DOT}{}",
        document.kind,
        document.version,
        document.title,
        document.created_at.format("%Y-%m-%d")
    )
}

/// The list order's key: a workspace's `goals` first (D8), then kind byte order (`str` order is
/// byte order), the newest version first.
fn order_key(row: &OwnerDocument, goals_first: bool) -> (bool, &str, Reverse<i32>) {
    (
        !(goals_first && row.kind == GOALS_KIND),
        row.kind.as_str(),
        Reverse(row.version),
    )
}

/// The key in force that reopens a kept draft: `n`'s label under the defaults.
fn reopen_label(keys: &Keys) -> Option<String> {
    keys.label(views::DOCUMENTS_BROWSE, Act::New)
}

/// Review M2: `head`, then that the text is kept and the key that reopens `$EDITOR` on it.
fn kept_sentence(head: &str, reopen: Option<&str>) -> String {
    match reopen {
        Some(key) => format!("{head}; your text is kept and {key} reopens $EDITOR on it"),
        None => format!("{head}; your text is kept"),
    }
}

/// The next step of [`NO_DOCUMENTS`]: the key in force that adds one.
fn adds_one(keys: &Keys) -> String {
    keys.label(views::DOCUMENTS_BROWSE, Act::New).map_or_else(
        || "no key in force adds one".to_owned(),
        |key| format!("{key} adds one"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone as _, Utc};
    use htui_core::fixtures::ids;
    use uuid::Uuid;

    fn document(n: u128, kind: &str, version: i32) -> OwnerDocument {
        OwnerDocument {
            id: DocumentId::from_uuid(Uuid::from_u128(n)),
            project_id: ids::PROJECT_SCOPE_GRAPHICS,
            kind: kind.to_owned(),
            version,
            title: format!("{kind} {version}"),
            body: String::new(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: Utc
                .with_ymd_and_hms(2026, 11, 2, 9, 0, 0)
                .single()
                .expect("an instant"),
        }
    }

    fn order(owner: DocumentOwner) -> Vec<(String, i32)> {
        let (pane, _) = OwnerDocumentsPane::open(owner, "x".to_owned());
        pane.ordered(&[
            document(1, "brief", 1),
            document(2, "goals", 1),
            document(3, "brief", 2),
            document(4, "goals", 2),
            document(5, "adr", 1),
        ])
        .into_iter()
        .map(|row| (row.kind, row.version))
        .collect()
    }

    fn pairs(rows: &[(&str, i32)]) -> Vec<(String, i32)> {
        rows.iter()
            .map(|(kind, version)| ((*kind).to_owned(), *version))
            .collect()
    }

    #[test]
    fn a_workspace_lists_goals_first_then_kinds_newest_first() {
        assert_eq!(
            order(DocumentOwner::Workspace(ids::WORKSPACE_GRAPHICS)),
            pairs(&[
                ("goals", 2),
                ("goals", 1),
                ("adr", 1),
                ("brief", 2),
                ("brief", 1)
            ])
        );
    }

    #[test]
    fn a_project_lists_kinds_in_byte_order_newest_first() {
        assert_eq!(
            order(DocumentOwner::Project(ids::PROJECT_VULKAN)),
            pairs(&[
                ("adr", 1),
                ("brief", 2),
                ("brief", 1),
                ("goals", 2),
                ("goals", 1)
            ])
        );
    }

    #[test]
    fn a_row_reads_kind_version_title_and_date() {
        assert_eq!(
            row_line(&document(1, "goals", 3)),
            "goals \u{b7} v3 \u{b7} goals 3 \u{b7} 2026-11-02"
        );
    }
}
