//! The Qdrant section of the Settings tab: the Qdrant URL and API key this box keeps in its
//! keyring, and the keys that change them (CLEAN-8, MOD-10 M4).
//!
//! It holds no store handle (`R-NF-3`): it names one read ([`StoreRequest::QdrantInfo`]), is
//! handed the [`QdrantSnapshot`] that comes back, and every write leaves through `ctx.request`.
//! The key is typed into a masked field, read once by `take`, and travels [`Redacted`]; no
//! `Debug` here prints a field or a notice.
//!
//! The rows are one kit form (MOD-115 M3 D3): an open field is its own row right under the row it
//! replaces, its guide or refusal under it; the clear question is the error line of the focused
//! row; the last outcome is a kit notice line under the form. Every sentence that names a key
//! reads it from the keys in force (MD-A, C5), and the acts resolve from [`Ctx::pressed`].

use std::borrow::Cow;
use std::cell::Cell;

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use crate::secrets_settings::{DEMO_SESSION, Redacted};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form, Value};
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{SectionId, SettingsSection, answers, asked};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

const NOT_READ: &str = "not read yet";
const STORED: &str = "stored";
const CLEARED: &str = "the Qdrant settings are gone from the keyring";
const EMPTY_KEY: &str = "nothing typed; the stored API key is unchanged";
/// CLEAN-8 #9: both rows in a demo session, which has no keyring.
const DEMO_ROW: &str = "n/a in a demo session";
const UNREADABLE: &str = "the keyring could not be read";
/// The read itself was refused; the seam's sentence follows it.
const UNAVAILABLE: &str = "Qdrant settings are unavailable";
/// The key field's guide. Not "Enter API key": that named a verb with a key's name (C5, D-15).
const KEY_GUIDE: &str = "type the API key, or leave it blank if there is none";
/// The labels of the two rows and of the field that replaces each.
const URL_LABEL: &str = "URL";
/// The key row's label.
const KEY_LABEL: &str = "API key";
/// The open URL field's label, under the URL row.
const NEW_URL_LABEL: &str = "new URL";
/// The open key field's label, under the key row.
const NEW_KEY_LABEL: &str = "new key";
/// One label column for every row and field, so opening a field never moves the values.
const LABEL_WIDTH: u16 = 8;

/// What the open URL field says when no URL is stored. `submit` is `field.submit`'s label through
/// `CAPTURE` (MD-A, C5); with none bound the sentence says what to do without a key.
fn no_url_yet(submit: Option<&str>) -> String {
    match submit {
        Some(key) => format!("no Qdrant URL is stored; type one and press {key}"),
        None => "no Qdrant URL is stored; type one to store it".to_owned(),
    }
}

/// What the open URL field says over a stored one; `submit` as [`no_url_yet`]'s.
fn replaces_url(submit: Option<&str>) -> String {
    match submit {
        Some(key) => format!("{key} replaces the stored Qdrant URL"),
        None => "storing replaces the stored Qdrant URL".to_owned(),
    }
}

/// What the open URL field says over a keyring that could not be read; `submit` as
/// [`no_url_yet`]'s.
fn unreadable_guide(submit: Option<&str>) -> String {
    match submit {
        Some(key) => {
            format!("the keyring could not be read; {key} tries to store a Qdrant URL in it")
        }
        None => "the keyring could not be read; storing tries to put a Qdrant URL in it".to_owned(),
    }
}

/// `c` with nothing stored (MOD-83): what is the case and the key that changes it, rather than a
/// claim that the settings are unavailable. `edit` is `Edit`'s label through `QDRANT_BROWSE`.
fn nothing_to_clear(edit: Option<&str>) -> String {
    let said = "no Qdrant URL or API key is stored, so there is nothing to clear";
    match edit {
        Some(key) => format!("{said}; {key} stores one"),
        None => said.to_owned(),
    }
}

/// `c` over an unreadable row (MOD-83): the row's own words and the key that reads it again,
/// never [`nothing_to_clear`], since nobody knows whether anything is stored. `reload` is
/// `Reload`'s label through `QDRANT_BROWSE`.
fn unreadable_refusal(why: String, reload: Option<&str>) -> String {
    match reload {
        Some(key) => format!("{UNREADABLE}: {why}; {key} reads it again"),
        None => format!("{UNREADABLE}: {why}"),
    }
}

/// The next step of [`UNAVAILABLE`] (MOD-83): the reload key in force, or what happens without
/// one.
fn asks_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "it is asked again when the section opens".to_owned(),
        |key| format!("{key} asks again once the store answers"),
    )
}
/// The clear question; the answers are appended as the keys bind them (MOD-67 M6 PA-6).
const CONFIRM_CLEAR: &str = "Remove the Qdrant settings from the keyring?";

// The hint rows (MOD-67 M3 D9): rendered through the mode's stack, so a rebound key shows its
// new chord. The editors' keys are `field.submit` and `field.cancel` through `CAPTURE` (MOD-115
// M3 C6): `Enter store \u{b7} Esc cancel` under the compiled keys.
const HINT_BROWSE: HintSpec = &[
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::Clear, "clear all"),
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
const HINT_NO_SNAPSHOT: HintSpec = &[
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
const HINT_EDITING: HintSpec = &[
    Hint::One(Act::FieldSubmit, "store"),
    Hint::One(Act::FieldCancel, "cancel"),
];
const HINT_EDITING_KEY: HintSpec = &[
    Hint::One(Act::FieldSubmit, "store"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::Text("typed text is never shown"),
];
const HINT_CONFIRM: HintSpec = &[
    Hint::One(Act::ConfirmYes, "confirm"),
    Hint::All(Act::ConfirmNo, "cancel"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Url,
    Key,
}
impl Row {
    const ALL: [Self; 2] = [Self::Url, Self::Key];
}
struct Editor {
    input: TextField,
}

#[derive(Default)]
enum Mode {
    #[default]
    Browse,
    ConfirmClear,
    EditingUrl(Editor),
    EditingKey(Editor),
}

#[derive(Clone, PartialEq, Eq)]
enum Notice {
    Info(String),
    Error(String),
}

impl Notice {
    fn text(&self) -> &str {
        match self {
            Self::Info(text) | Self::Error(text) => text,
        }
    }
    fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// Qdrant section of Settings tab
pub struct QdrantSection {
    mode: Mode,
    notice: Option<Notice>,
    snapshot: Option<QdrantSnapshot>,
    unavailable: Option<String>,
    busy: Option<&'static str>,
    /// An `r` re-read is out. The worker answers in order, and a newer read supersedes an older one
    /// at the app's staleness gate, so the next snapshot is that read's answer, never a write's.
    read_out: bool,
    opened_for_empty: bool,
    cursor: usize,
    /// The form's `(lines, first line drawn)` as of the last render, for the Settings pane's
    /// scrollbar (MOD-115 M3 D-11), the notice rows under it counted; `None` while the refusal
    /// stands in for the rows.
    scroll: Cell<Option<ScrollSpan>>,
}

impl std::fmt::Debug for QdrantSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantSection").finish_non_exhaustive()
    }
}

impl Default for QdrantSection {
    fn default() -> Self {
        Self::new()
    }
}

impl QdrantSection {
    /// The ID for the qdrant settings section
    pub const ID: SectionId = SectionId("qdrant");

    /// Create a new QdrantSection
    pub fn new() -> Self {
        Self {
            mode: Mode::default(),
            notice: None,
            snapshot: None,
            unavailable: None,
            busy: None,
            read_out: false,
            opened_for_empty: false,
            cursor: 0,
            scroll: Cell::new(None),
        }
    }

    fn state(&self) -> Option<&QdrantState> {
        self.snapshot.as_ref().map(|s| &s.url_state)
    }

    /// CLEAN-8 #9: the snapshot is a demo session's, which has no keyring to edit.
    fn demo(&self) -> bool {
        self.state() == Some(&QdrantState::NotApplicable)
    }

    fn row(&self) -> Row {
        Row::ALL[self.cursor]
    }

    fn move_cursor(&mut self, down: bool) {
        // Moving the cursor is not a write, so a write in flight does not stop it.
        if self.snapshot.is_none() || self.unavailable.is_some() {
            return;
        }
        if down {
            self.cursor = (self.cursor + 1) % Row::ALL.len();
        } else {
            self.cursor = (self.cursor + Row::ALL.len() - 1) % Row::ALL.len();
        }
    }

    /// Whether a key that opens an editor or a question is refused right now. A write in flight
    /// says so, as the Connection section's does, and so does a demo session (CLEAN-8 #9), as the
    /// Secrets section's does; `r` is deliberately not on this path.
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return true;
        }
        if self.demo() {
            self.refuse(DEMO_SESSION.to_owned());
            return true;
        }
        self.unavailable.is_some() || self.snapshot.is_none()
    }

    fn open_edit(&mut self) {
        match self.row() {
            Row::Url => {
                self.mode = Mode::EditingUrl(Editor {
                    input: TextField::new(),
                });
            }
            Row::Key => {
                self.mode = Mode::EditingKey(Editor {
                    input: TextField::masked(),
                });
            }
        }
    }

    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let (outcome, url_to_submit, key_to_submit) = match &mut self.mode {
            Mode::EditingUrl(editor) => {
                let outcome = editor.input.on_key(key);
                let mut url = None;
                if let FieldOutcome::Submit = outcome {
                    url = Some(editor.input.text().unwrap_or("").trim().to_owned());
                }
                (outcome, url, None)
            }
            Mode::EditingKey(editor) => {
                let outcome = editor.input.on_key(key);
                let mut key_text = None;
                if let FieldOutcome::Submit = outcome {
                    // MOD-10 M4 blueprint A-6: the field is masked, so `text()` is `None`; `take`
                    // is its only read. The typed key is wiped here and travels redacted.
                    let raw = zeroize::Zeroizing::new(editor.input.take());
                    key_text = Some(Redacted::new(raw.trim().to_owned()));
                }
                (outcome, None, key_text)
            }
            _ => return Handled::Pass,
        };

        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                if let Some(url) = url_to_submit {
                    if url.is_empty() {
                        self.mode = Mode::Browse;
                        self.say("nothing typed; the stored URL is unchanged");
                        return Handled::Consumed;
                    }
                    self.mode = Mode::Browse;
                    self.send(StoreRequest::SetQdrantUrl(url), ctx);
                }
                if let Some(key_text) = key_to_submit {
                    self.mode = Mode::Browse;
                    // MOD-10 M4 R1 L-5: as the URL row, an empty submit changes nothing; removing
                    // the key stays on `c`, behind its question.
                    if key_text.is_empty() {
                        self.say(EMPTY_KEY);
                        return Handled::Consumed;
                    }
                    self.send(StoreRequest::SetQdrantApiKey(key_text), ctx);
                }
                Handled::Consumed
            }
            // The field passed it (`Tab`, arrows, a CONTROL chord …): the editor keeps it unless
            // the modal global layer admits it (MOD-67 D5, PA-5). `Tab` stays here rather than
            // switching tabs (ANA-26 §2.6 defect 2); `ctrl-c` and `F1` reach the shell.
            // The chord the user pressed (MOD-115 M3 MD-A): a `[field]` chord the field passed is
            // the field's, never re-read as another act of its canonical key.
            FieldOutcome::Pass => modal_rest(views::CAPTURE, ctx.pressed(&key)),
        }
    }

    /// The clear question (MOD-67 M3 §6.4): `confirm.yes` clears, `confirm.no` goes back, a chord
    /// the modal global layer admits passes, and everything else is swallowed.
    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::QDRANT_CONFIRM;
        let chord = ctx.pressed(&key);
        // The first candidate decides (MOD-67 M3 §6.4): a global act there is the pass rule's.
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ConfirmYes) => {
                self.mode = Mode::Browse;
                self.send(StoreRequest::ClearQdrantSettings, ctx);
                return Handled::Consumed;
            }
            Some(Act::ConfirmNo) => {
                self.mode = Mode::Browse;
                return Handled::Consumed;
            }
            _ => {}
        }
        modal_rest(stack, chord)
    }

    /// The stack of the current mode (MOD-67 D4): the one place a mode maps to its keys;
    /// `key_stack`, the key handlers and the hint all read it.
    fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::QDRANT_BROWSE,
            Mode::ConfirmClear => views::QDRANT_CONFIRM,
            Mode::EditingUrl(_) | Mode::EditingKey(_) => views::CAPTURE,
        }
    }

    /// One key in Browse (MOD-67 M3 §6.2): whole chords through `QDRANT_BROWSE`, so `ctrl-e`
    /// edits nothing and `ctrl-r` reads nothing (defect 1).
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::QDRANT_BROWSE, chord) {
            match act {
                Act::Edit => {
                    if !self.blocked() {
                        self.open_edit();
                    }
                    return Handled::Consumed;
                }
                Act::Clear => {
                    if !self.blocked() {
                        if matches!(self.state(), Some(QdrantState::Stored))
                            || matches!(
                                self.snapshot.as_ref().map(|s| &s.key_state),
                                Some(QdrantState::Stored)
                            )
                        {
                            self.notice = None;
                            self.mode = Mode::ConfirmClear;
                        } else if let Some(why) = self.unreadable_why() {
                            // An unreadable row is not an empty one (Connection's rule): its own
                            // words, never a claim that nothing is stored.
                            let reload = ctx.keys().label(views::QDRANT_BROWSE, Act::Reload);
                            self.refuse(unreadable_refusal(why, reload.as_deref()));
                        } else {
                            let edit = ctx.keys().label(views::QDRANT_BROWSE, Act::Edit);
                            self.refuse(nothing_to_clear(edit.as_deref()));
                        }
                    }
                    return Handled::Consumed;
                }
                Act::ListDown => {
                    self.move_cursor(true);
                    return Handled::Consumed;
                }
                Act::ListUp => {
                    self.move_cursor(false);
                    return Handled::Consumed;
                }
                // A re-read, never refused: it is how the unavailable state recovers (MOD-63). It
                // sets no `busy`, like the Connection section's `r`.
                Act::Reload => {
                    self.read_out = true;
                    ctx.request(StoreRequest::QdrantInfo);
                    return Handled::Consumed;
                }
                // A global act: the shell resolves the same stack and applies it.
                _ => continue,
            }
        }
        Handled::Pass
    }

    /// Why the first unreadable row, URL then key, could not be read: `None` when both are known.
    fn unreadable_why(&self) -> Option<String> {
        let snapshot = self.snapshot.as_ref()?;
        [&snapshot.url_state, &snapshot.key_state]
            .into_iter()
            .find_map(|state| match state {
                QdrantState::Unreadable(why) => Some(why.clone()),
                _ => None,
            })
    }

    /// Sends one write and remembers its name until the reply, so `on_snapshot` can say what it
    /// did and a failure lands on this section (the Connection section's `send`).
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        self.notice = None;
        ctx.request(request);
    }

    /// The guide under the open URL field: what storing there does, named with the submit key in
    /// force (C5).
    fn url_guide(&self, keys: &Keys) -> String {
        let submit = keys.label(views::CAPTURE, Act::FieldSubmit);
        match self.state() {
            Some(QdrantState::Stored) => replaces_url(submit.as_deref()),
            Some(QdrantState::Unreadable(_)) => unreadable_guide(submit.as_deref()),
            _ => no_url_yet(submit.as_deref()),
        }
    }

    /// The URL row's value: the stored summary, or the state the keyring is in.
    fn url_value(&self) -> Value<'static> {
        let text = match self.state() {
            Some(QdrantState::Stored) => self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.url_summary.as_ref())
                .map_or_else(
                    || STORED.to_owned(),
                    |summary| format!("{STORED} \u{b7} {summary}"),
                ),
            Some(QdrantState::NotStored) => return Value::Secret(false),
            Some(QdrantState::Unreadable(why)) => format!("{UNREADABLE}: {why}"),
            Some(QdrantState::NotApplicable) => DEMO_ROW.to_owned(),
            None => NOT_READ.to_owned(),
        };
        Value::ReadOnly(Cow::Owned(text))
    }

    /// The key row's value: a secret, stored or not, and never a placeholder that reads as data.
    fn key_value(&self) -> Value<'static> {
        match self.snapshot.as_ref().map(|snapshot| &snapshot.key_state) {
            Some(QdrantState::Stored) => Value::Secret(true),
            Some(QdrantState::NotStored) => Value::Secret(false),
            Some(QdrantState::Unreadable(why)) => {
                Value::ReadOnly(Cow::Owned(format!("{UNREADABLE}: {why}")))
            }
            Some(QdrantState::NotApplicable) => Value::ReadOnly(Cow::Borrowed(DEMO_ROW)),
            None => Value::ReadOnly(Cow::Borrowed(NOT_READ)),
        }
    }

    /// The open field's row: `label`, the field, and its guide or what was refused under it.
    fn field_row<'a>(
        &'a self,
        label: &'a str,
        input: &'a TextField,
        guide: String,
    ) -> form::Field<'a> {
        let field = form::Field::new(label, Value::Text(input));
        match &self.notice {
            Some(Notice::Error(text)) => field.error(text.as_str()),
            Some(Notice::Info(text)) => field.help(text.as_str()),
            None => field.help(guide),
        }
    }

    /// The two rows as one kit form (MOD-115 M3 D3): the cursor row focused while browsing; an
    /// open field on its own row right under the row it replaces (so what is stored stays on
    /// screen); the clear question under the focused row.
    fn form(&self, keys: &Keys) -> Form<'_> {
        let mut url = form::Field::new(URL_LABEL, self.url_value());
        let mut key = form::Field::new(KEY_LABEL, self.key_value());
        if matches!(self.mode, Mode::ConfirmClear) {
            let question = asked(CONFIRM_CLEAR, &answers(keys, views::QDRANT_CONFIRM));
            match self.row() {
                Row::Url => url = url.error(question),
                Row::Key => key = key.error(question),
            }
        }
        let mut form = Form::new().label_width(LABEL_WIDTH).field(url);
        let focus = match &self.mode {
            Mode::EditingUrl(editor) => {
                form =
                    form.field(self.field_row(NEW_URL_LABEL, &editor.input, self.url_guide(keys)));
                1
            }
            Mode::EditingKey(_) => 2,
            Mode::Browse | Mode::ConfirmClear => self.cursor,
        };
        form = form.field(key);
        if let Mode::EditingKey(editor) = &self.mode {
            form = form.field(self.field_row(NEW_KEY_LABEL, &editor.input, KEY_GUIDE.to_owned()));
        }
        form.focus(focus).editing(matches!(
            self.mode,
            Mode::EditingUrl(_) | Mode::EditingKey(_)
        ))
    }

    /// The rows under the form in Browse: the last outcome, as kit notice lines in `theme.error`
    /// when it is one to act on. The keys this mode binds are on the pane's border (MOD-67 M6
    /// T2, [`SettingsSection::key_hint`]); the editors and the question draw theirs in the form.
    fn notice_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let (Some(said), Mode::Browse) = (&self.notice, &self.mode) else {
            return Vec::new();
        };
        let tone = if said.is_error() {
            Tone::Error
        } else {
            Tone::Info
        };
        wrapped(said.text(), usize::from(width).max(1))
            .into_iter()
            .map(|line| notice(&line, tone, theme))
            .collect()
    }

    fn hint_text(&self, keys: &Keys) -> String {
        let spec = match self.mode {
            Mode::EditingUrl(_) => HINT_EDITING,
            Mode::EditingKey(_) => HINT_EDITING_KEY,
            Mode::ConfirmClear => HINT_CONFIRM,
            Mode::Browse => {
                if self.unavailable.is_some() || self.snapshot.is_none() {
                    HINT_NO_SNAPSHOT
                } else {
                    HINT_BROWSE
                }
            }
        };
        let keys = keys.hint(self.stack(), spec);
        match self.busy {
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {}\u{2026}", doing(busy))
            }
            _ => keys,
        }
    }

    fn say(&mut self, text: &str) {
        self.notice = Some(if super::is_error(text) {
            Notice::Error(text.to_owned())
        } else {
            Notice::Info(text.to_owned())
        });
    }

    fn refuse(&mut self, text: String) {
        self.notice = Some(Notice::Error(text));
    }

    /// A fresh snapshot, and the write that asked for it, if any, says what it did.
    ///
    /// A `Qdrant` reply names no request, so attribution is by order. A snapshot that lands while
    /// an `r` re-read is out answers that read and leaves `busy` alone, so a reload sent just before
    /// a write can never report the write as stored or cleared before it has run. A write sent
    /// before the `r` answers first, so its notice waits for the read's snapshot and is then true.
    fn on_snapshot(&mut self, snapshot: &QdrantSnapshot) {
        let write = if std::mem::take(&mut self.read_out) {
            None
        } else {
            self.busy.take()
        };
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());

        match write {
            Some("set_qdrant_url") | Some("set_qdrant_api_key") => {
                self.mode = Mode::Browse;
                self.say(STORED);
            }
            Some("clear_qdrant_settings") => {
                self.mode = Mode::Browse;
                self.say(CLEARED);
            }
            _ => {}
        }

        if write.is_none()
            && snapshot.url_state == QdrantState::NotStored
            && !self.opened_for_empty
            && matches!(self.mode, Mode::Browse)
        {
            // No notice: the open field's own guide says there is no URL, with the key in force.
            self.opened_for_empty = true;
            self.open_edit();
        }
    }
}

/// What a second write is told while the first is still out, in words (no request name).
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// What the request `busy` is doing, as the user reads it.
fn doing(busy: &str) -> &'static str {
    match busy {
        "set_qdrant_url" => "storing the Qdrant URL",
        "set_qdrant_api_key" => "storing the API key",
        "clear_qdrant_settings" => "clearing the Qdrant settings",
        "qdrant_info" => "reading the Qdrant settings",
        _ => "writing",
    }
}

impl SettingsSection for QdrantSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Qdrant"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::QdrantInfo]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        self.mode = Mode::Browse;
        self.opened_for_empty = false;
        self.notice = None;
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// MOD-22 review M-1: a bracketed paste into the open URL or key field, whole; the masked key
    /// within its reservation or refused by name.
    fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        let (Mode::EditingUrl(editor) | Mode::EditingKey(editor)) = &mut self.mode else {
            return Handled::Pass;
        };
        if !editor.input.on_paste(text) {
            ctx.emit(crate::app::Action::Error(
                crate::ui::text_field::PASTE_DOES_NOT_FIT.to_owned(),
            ));
        }
        Handled::Consumed
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::EditingUrl(_) | Mode::EditingKey(_) => self.on_editor_key(key, ctx),
            Mode::ConfirmClear => self.on_confirm_key(key, ctx),
            Mode::Browse => self.on_browse_key(key, ctx),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Qdrant(snapshot) = reply {
            self.on_snapshot(snapshot);
        } else if let StoreReply::Failed { request, message } = reply {
            if self.busy == Some(*request) {
                self.busy = None;
                self.refuse(message.clone());
            } else if *request == "qdrant_info" {
                self.read_out = false;
                self.unavailable = Some(message.clone());
            }
        }
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    /// The form over the rows, the notice under it; the refusal and its next step stand in for
    /// the rows while the read is refused.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let said = self.notice_lines(area.width, theme);
        let [rows, notice_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(u16::try_from(said.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        self.scroll.set(None);
        match &self.unavailable {
            // The refusal wins the rows even with a snapshot behind it (Connection's rule): what
            // is on screen would otherwise be a keyring nothing has confirmed since the outage.
            Some(why) => {
                let mut lines: Vec<Line<'static>> = wrapped(
                    &format!("{UNAVAILABLE}: {why}"),
                    usize::from(rows.width).max(1),
                )
                .into_iter()
                .map(|line| notice(&line, Tone::Error, theme))
                .collect();
                // MOD-83: the next step, in the empty state's own role.
                let reload = ctx.keys().label(views::QDRANT_BROWSE, Act::Reload);
                let next = asks_again(reload.as_deref());
                lines.extend(empty::lines("", NextStep(&next), theme).pop());
                frame.render_widget(Paragraph::new(lines), rows);
            }
            None => {
                let mut laid = self
                    .form(ctx.keys())
                    .layout(rows.width, ctx.accent(), theme);
                // The form's own measure of the focused row (RV2 M1), not a guess.
                let offset = laid.offset(rows.height);
                let total = laid.lines.len();
                let shown: Vec<Line<'static>> = laid.lines.drain(offset.min(total)..).collect();
                frame.render_widget(Paragraph::new(shown), rows);
                // Beside the rows the form was drawn into (RV1 M2): the notice under them does
                // not scroll.
                self.scroll
                    .set(Some(ScrollSpan::new(total, offset).rows(rows)));
            }
        }
        if !said.is_empty() {
            frame.render_widget(Paragraph::new(said), notice_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-67 M6 T2: in Browse the rows under the form carry the notice alone, as kit notice
    /// lines, and the keys are the section's [`SettingsSection::key_hint`], the pane's border.
    #[test]
    fn the_notice_line_carries_the_notice_alone() {
        let notice = "\u{6f22}".repeat(10);
        let section = QdrantSection {
            notice: Some(Notice::Error(notice.clone())),
            ..QdrantSection::new()
        };
        let theme = Theme::default();
        let lines = section.notice_lines(100, &theme);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].to_string(), notice);
        assert_eq!(lines[0].style, theme.error);
        assert!(QdrantSection::new().notice_lines(100, &theme).is_empty());
        let keys = section
            .key_hint(Keys::compiled(), &theme)
            .expect("the defaults bind the keys");
        assert_eq!(keys.to_string(), section.hint_text(Keys::compiled()));
    }

    /// MD-A, C5: the guides with their key unbound, and a write in flight in words.
    #[test]
    fn the_qdrant_sentences_without_their_keys() {
        assert_eq!(
            no_url_yet(Some("Enter")),
            "no Qdrant URL is stored; type one and press Enter"
        );
        assert_eq!(
            no_url_yet(None),
            "no Qdrant URL is stored; type one to store it"
        );
        assert_eq!(replaces_url(None), "storing replaces the stored Qdrant URL");
        assert!(unreadable_guide(None).ends_with("storing tries to put a Qdrant URL in it"));
        assert_eq!(
            nothing_to_clear(None),
            "no Qdrant URL or API key is stored, so there is nothing to clear"
        );
        for name in [
            "set_qdrant_url",
            "set_qdrant_api_key",
            "clear_qdrant_settings",
            "qdrant_info",
        ] {
            assert_ne!(doing(name), doing("unknown"), "`{name}` has its own phrase");
            assert!(!in_flight(name).contains('_'), "{}", in_flight(name));
        }
    }
}
