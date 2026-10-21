//! The queue section of the Settings tab (MOD-12 milestone 2, D8-D10): the spend caps of each
//! scope project, the three box-wide `app_setting` keys and this box's concurrency limit.
//!
//! It holds **no store handle** (`R-NF-3`): it names one read ([`StoreRequest::QueueSettings`]),
//! is handed the snapshot that comes back, and every write leaves through `ctx.request` for
//! [`crate::queue_settings::serve`] to carry out. What is on screen is always the last snapshot the
//! worker assembled; no row is patched in locally.
//!
//! Money is typed and shown in USD and stored as micros ([`parse_usd`](htui_core::model::parse_usd), [`format_usd`](htui_core::model::format_usd)); the
//! window is typed `HH:MM-HH:MM` ([`parse_window`](htui_core::model::parse_window)). Every bound is the key's own validator
//! ([`QueueSetting::validate`](htui_core::model::QueueSetting::validate)), checked here before a request is sent and again by the store, and
//! its sentence is shown verbatim. Empty clears: a cap becomes unbounded, the box limit inherits
//! the app default.
//!
//! On the kit (MOD-115 M3 L-prompt): the rows are one `kit::form` of read-only values under the
//! `project …`, `all boxes` and `this box (…)` headings, the focused row marked by the form's
//! gutter in the tab's accent; an open editor is the row it edits, with its unit and what the row
//! holds now on the line under it. Every key a hint or a sentence names is the key in force,
//! resolved from `Ctx::pressed` (MD-A).

use htui_core::model::{
    BoxId, DEFAULT_MAX_CONCURRENT_ITEMS, ProjectId, QueueSetting, Scope, format_usd, format_window,
    parse_usd, parse_window,
};
use htui_core::store::{QueueTarget, QueueToken};
use std::cell::Cell;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use serde_json::{Value, json};

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::queue_settings::{QueueProjectEntry, QueueSettingsSnapshot, READ_NAME, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Field, Form};
use crate::ui::kit::list;
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection, changed_elsewhere,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

/// The help line under the rows (plan risk row 2): a cap guards only what an agent reports.
pub const UNKNOWN_COST: &str =
    "a run whose agent reports no USD cost is never capped (unknown is unbounded)";

/// Browse's keys, with something to browse: `j/k · e edit · r reload` by default (`Enter` edits
/// too, through the queue's view default on `common.edit`; the hint names the first chord).
pub const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::Reload, "reload"),
];

/// Browse's keys with nothing read, or the read refused.
const HINT_NO_SNAPSHOT: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// An open editor's keys: the field's own submit and cancel through `CAPTURE`'s `[field]` layer
/// (MOD-67 D13; MOD-115 M3 MD-A, C6), `Enter save · Esc cancel` with the compiled keys.
pub const HINT_EDITING: HintSpec = &[
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::Text("empty clears"),
];

/// What the rows pane says before any settings have arrived.
pub const NOT_READ: &str = "queue settings not read yet";

/// What the rows pane says when the read itself was refused, before the refusal's sentence.
pub const UNAVAILABLE: &str = "queue settings unavailable";

/// The next step of [`NOT_READ`]: the reload key in force, or what reads the settings without one.
fn reads_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "the queue settings are read again when the section opens".to_owned(),
        |key| format!("{key} reads the queue settings again"),
    )
}

/// The next step of [`UNAVAILABLE`]: the reload key in force, or when the read is asked again.
fn asks_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "they are asked for again when the section opens".to_owned(),
        |key| format!("{key} asks again once the store answers"),
    )
}

/// The label column: today's, the widest label (`max concurrent`) wide.
const LABEL_WIDTH: u16 = 14;

/// What `e` says on a group header. `edit` is `Edit`'s first label through `QUEUE_BROWSE`
/// (MOD-67 M6 D4); with none bound the sentence names no key.
#[must_use]
pub fn not_a_value_row(edit: Option<&str>) -> String {
    match edit {
        Some(edit) => format!("`{edit}` edits a value row"),
        None => "only a value row is edited".to_owned(),
    }
}

/// What an empty field over a key that holds nothing says: there is nothing to clear.
pub const NOTHING_SET: &str = "nothing is set here";

/// D10: the window row's suffix; R-ORCH-13 (enforcing it) is `later`.
pub const WINDOW_NOT_ENFORCED: &str = "stored, not enforced";

/// A cap that holds nothing.
const UNBOUNDED: &str = "unbounded";

/// A minimum that holds nothing.
const NONE: &str = "none";

/// A window that holds nothing.
const NOT_SET: &str = "not set";

/// The `app_setting` group's line.
const APP_HEADER: &str = "all boxes";

/// The sentence for concurrency text that is not a whole number.
const WHOLE_NUMBER: &str = "type a whole number, like 3";

/// One line of the section, by index into the snapshot. Headers are rows, as `PromptSection`'s
/// are, so a row's index is its line's index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The `project {slug}` line.
    ProjectHeader {
        /// Index into `snapshot.projects`.
        p: usize,
    },
    /// One of a project's two caps.
    Project {
        /// Index into `snapshot.projects`.
        p: usize,
        /// [`QueueSetting::PerTokenCapRun`] or [`QueueSetting::PerTokenCapBatch`].
        key: QueueSetting,
    },
    /// The `all boxes` line.
    AppHeader,
    /// `snapshot.app[i]`.
    App {
        /// Index into `snapshot.app`.
        i: usize,
    },
    /// The `this box ({hostname})` line.
    BoxHeader,
    /// This box's `max_concurrent_items`.
    Box,
}

/// Where an editor writes: the target and the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// One `app_setting` row.
    App(QueueSetting),
    /// One cap of one project.
    Project(ProjectId, QueueSetting),
    /// This box's limit.
    Box(BoxId),
}

impl Target {
    /// The key written.
    const fn key(self) -> QueueSetting {
        match self {
            Self::App(key) | Self::Project(_, key) => key,
            Self::Box(_) => QueueSetting::MaxConcurrentItems,
        }
    }

    /// The store's target.
    const fn queue_target(self) -> QueueTarget {
        match self {
            Self::App(_) => QueueTarget::App,
            Self::Project(id, _) => QueueTarget::Project(id),
            Self::Box(id) => QueueTarget::Box(id),
        }
    }
}

/// The open editor: the target, its one field, and the token it opened on.
struct Editor {
    /// Where this writes.
    target: Target,
    /// The buffer; never printed by `Debug`.
    input: TextField,
    /// The token the write presents.
    expected: QueueToken,
}

/// The target and the token, never the buffer (`PromptSection`'s H-5).
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("target", &self.target)
            .field("expected", &self.expected)
            .finish()
    }
}

/// What the section is doing.
#[derive(Default)]
enum Mode {
    /// The rows and the cursor.
    #[default]
    Browse,
    /// One row being typed into.
    Editing(Editor),
}

/// The mode and the editor's target, never the buffer.
impl core::fmt::Debug for Mode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Browse => f.write_str("Browse"),
            Self::Editing(editor) => f.debug_tuple("Editing").field(editor).finish(),
        }
    }
}

/// The last outcome, and whether the user has to act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// One line of report.
    Info(String),
    /// One line drawn in `theme.error`.
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

/// What a reloaded snapshot does to an open editor's token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reload {
    /// The row the editor opened on is gone from the reload.
    Gone,
    /// The token as it is now (`Stamp(None)` for an app row that no longer exists).
    Token(QueueToken),
}

/// `Settings > Queue` (MOD-12 M2 D8): the spend caps, the minimum, both concurrency levels and
/// the stored window.
#[derive(Debug, Default)]
pub struct QueueSection {
    /// The last settings the worker assembled, or `None` before the first reply.
    snapshot: Option<QueueSettingsSnapshot>,
    /// `Some(message)` after the read itself was refused.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](QueueSection::rows).
    cursor: usize,
    /// Browsing, or typing into one row.
    mode: Mode,
    /// The write in flight, by [`StoreRequest::name`]; a second is refused until the reply, and
    /// only a write's reply closes the editor (`PromptSection`'s H-3, H-4).
    busy: Option<&'static str>,
    /// The last outcome, on the notice row under the pane.
    notice: Option<Notice>,
    /// `(total rows, offset)` as of the last render, for the Settings pane's scrollbar (MOD-115
    /// M3 D-11); `None` while nothing scrolls.
    scroll: Cell<Option<ScrollSpan>>,
}

/// One item of the rows' form, in [`rows`](QueueSection::rows) order: built once per frame and
/// folded into a [`Form`] as often as the scroll needs to measure it.
#[derive(Clone)]
enum Entry<'a> {
    /// A group line (`project …`, `all boxes`, `this box (…)`).
    Heading(String),
    /// A value row, or the open editor's field.
    Field(Field<'a>),
    /// A row the snapshot no longer has: kept, so an item's index is still its row's.
    Blank,
}

/// `entries` as one form, `focus` focused, on today's label column.
fn form_of<'a>(entries: &[Entry<'a>], focus: usize, editing: bool) -> Form<'a> {
    entries.iter().fold(
        Form::new()
            .label_width(LABEL_WIDTH)
            .focus(focus)
            .editing(editing),
        |form, entry| match entry {
            Entry::Heading(text) => form.heading(text.clone()),
            Entry::Field(field) => form.field(field.clone()),
            Entry::Blank => form.blank(),
        },
    )
}

impl QueueSection {
    /// Stable identity.
    pub const ID: SectionId = SectionId("queue");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The flat list the cursor indexes, derived from the snapshot on demand.
    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for p in 0..snapshot.projects.len() {
            rows.push(Row::ProjectHeader { p });
            rows.extend(
                QueueSetting::PROJECT_KEYS
                    .into_iter()
                    .map(|key| Row::Project { p, key }),
            );
        }
        rows.push(Row::AppHeader);
        rows.extend((0..snapshot.app.len()).map(|i| Row::App { i }));
        if snapshot.this_box.is_some() {
            rows.push(Row::BoxHeader);
            rows.push(Row::Box);
        }
        rows
    }

    /// Moves the cursor one row, stopping at either end.
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

    /// Puts the cursor back inside the list after a reply replaced it.
    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// The row under the cursor.
    fn selected(&self) -> Option<Row> {
        self.rows().get(self.cursor).copied()
    }

    /// One project of the snapshot.
    fn project(&self, p: usize) -> Option<&QueueProjectEntry> {
        self.snapshot.as_ref()?.projects.get(p)
    }

    /// A value row's target, what it stores, and its token; `None` on a header.
    fn opening(&self, row: Row) -> Option<(Target, Option<&Value>, QueueToken)> {
        let snapshot = self.snapshot.as_ref()?;
        match row {
            Row::ProjectHeader { .. } | Row::AppHeader | Row::BoxHeader => None,
            Row::Project { p, key } => {
                let entry = snapshot.projects.get(p)?;
                Some((
                    Target::Project(entry.project.id, key),
                    entry.cap(key),
                    entry.token(),
                ))
            }
            Row::App { i } => {
                let entry = snapshot.app.get(i)?;
                Some((Target::App(entry.key), entry.value.as_ref(), entry.token))
            }
            Row::Box => {
                let entry = snapshot.this_box.as_ref()?;
                Some((Target::Box(entry.id), entry.value.as_ref(), entry.token))
            }
        }
    }

    /// What the target stores **now**: `None` when it is gone from the snapshot, `Some(None)` when
    /// it holds nothing.
    fn stored(&self, target: Target) -> Option<Option<&Value>> {
        let snapshot = self.snapshot.as_ref()?;
        match target {
            Target::App(key) => snapshot.app_entry(key).map(|entry| entry.value.as_ref()),
            Target::Project(id, key) => snapshot
                .projects
                .iter()
                .find(|entry| entry.project.id == id)
                .map(|entry| entry.cap(key)),
            Target::Box(id) => snapshot
                .this_box
                .as_ref()
                .filter(|entry| entry.id == id)
                .map(|entry| entry.value.as_ref()),
        }
    }

    /// `e`: the row under the cursor, prefilled with its value as typed.
    fn open_edit(&mut self, row: Row, keys: &Keys) {
        let Some((target, stored, expected)) = self.opening(row) else {
            self.say(&not_a_value_row(
                keys.label(views::QUEUE_BROWSE, Act::Edit).as_deref(),
            ));
            return;
        };
        let text = stored.map_or_else(String::new, |value| typed(target.key(), value));
        self.notice = None;
        self.mode = Mode::Editing(Editor {
            target,
            input: TextField::with_text(&text),
            expected,
        });
    }

    /// Whether the key that opens an editor is refused right now.
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return true;
        }
        self.snapshot.is_none() || self.unavailable.is_some()
    }

    /// Sends one write and remembers its name until the reply.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// One key while an editor is open: the field answers first. What it passes on is swallowed,
    /// except the chords [`views::CAPTURE`] passes (CONTROL, ALT, function keys: MOD-67 D5), so
    /// `ctrl-c` still quits and `F1` opens help.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Editing(editor) => editor.input.on_key(key),
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
            // The chord the user pressed (MD-A): a `[field]` chord the field passed is that row's,
            // never re-read as another act of its canonical key.
            FieldOutcome::Pass => modal_rest(views::CAPTURE, ctx.pressed(&key)),
        }
    }

    /// `Enter` in the editor: clear on an empty field, otherwise parse, validate and set. A parse
    /// or validator refusal keeps the editor open and sends nothing.
    fn submit(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return;
        }
        let Mode::Editing(editor) = &self.mode else {
            return;
        };
        let (target, expected) = (editor.target, editor.expected);
        let text = editor.input.text().unwrap_or_default().trim().to_owned();
        let Some(stored) = self.stored(target) else {
            self.mode = Mode::Browse;
            self.say(DELETED_ELSEWHERE);
            return;
        };

        if text.is_empty() {
            if stored.is_none() {
                self.say(NOTHING_SET);
                return;
            }
            self.notice = None;
            self.send(
                StoreRequest::ClearQueueSetting {
                    scope: ctx.scope.clone(),
                    target: target.queue_target(),
                    key: target.key(),
                    expected,
                },
                ctx,
            );
            return;
        }

        let key = target.key();
        match parse(key, &text).and_then(|value| key.validate(&value).map(|()| value)) {
            Ok(value) => {
                self.notice = None;
                self.send(
                    StoreRequest::SetQueueSetting {
                        scope: ctx.scope.clone(),
                        target: target.queue_target(),
                        key,
                        value,
                        expected,
                    },
                    ctx,
                );
            }
            Err(sentence) => self.refuse(sentence),
        }
    }

    /// Fresh settings: adopted whole. Only a write's reply closes the editor.
    fn on_settings(&mut self, snapshot: &QueueSettingsSnapshot) {
        let write = self.busy.take();
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        self.follow_editor();
        if write.is_some() {
            self.notice = None;
            self.mode = Mode::Browse;
        }
    }

    /// A compare-and-set miss: adopt the reload, keep the editor's text, retake its token, and
    /// leave the retry to the next `Enter`.
    /// The sentence names the retry key as the editor's stack binds it (MD-A, D-5).
    fn on_stale(&mut self, snapshot: &QueueSettingsSnapshot, keys: &Keys) {
        self.busy = None;
        self.unavailable = None;
        let reloaded = match &self.mode {
            Mode::Editing(editor) => Some(reload(snapshot, editor.target)),
            Mode::Browse => None,
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        self.follow_editor();
        match reloaded {
            None => self.say(CHANGED_ELSEWHERE_CLOSED),
            Some(Reload::Gone) => {
                self.mode = Mode::Browse;
                self.say(DELETED_ELSEWHERE);
            }
            Some(Reload::Token(token)) => {
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.expected = token;
                }
                let retry = keys.label(views::CAPTURE, Act::FieldSubmit);
                self.say(&changed_elsewhere(retry.as_deref()));
            }
        }
    }

    /// The open editor's row in [`rows`](QueueSection::rows), or `None` with no editor or with
    /// its row gone from the snapshot.
    fn editing_row(&self) -> Option<usize> {
        let Mode::Editing(editor) = &self.mode else {
            return None;
        };
        self.rows().into_iter().position(|row| {
            self.opening(row)
                .is_some_and(|(target, _, _)| target == editor.target)
        })
    }

    /// Puts the cursor on the open editor's row after a reload moved the rows under it, so the
    /// row being typed into is the one marked.
    fn follow_editor(&mut self) {
        if let Some(row) = self.editing_row() {
            self.cursor = row;
        }
    }

    /// The rows as form items, in [`rows`](QueueSection::rows) order, and the item focused (§2.2's
    /// mapping): each value read-only under its label, the open editor on the row it edits with
    /// its unit and what the row holds now as its help. When that row is gone from the snapshot
    /// the field is drawn after the last row, so nobody types into a field they cannot see.
    fn entries(&self, snapshot: &QueueSettingsSnapshot) -> (Vec<Entry<'_>>, usize) {
        let editor = match &self.mode {
            Mode::Editing(editor) => Some(editor),
            Mode::Browse => None,
        };
        let rows = self.rows();
        let editing = self.editing_row();
        let focus = match editor {
            Some(_) => editing.unwrap_or(rows.len()),
            None => self.cursor,
        };
        let mut entries: Vec<Entry<'_>> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| match *row {
                Row::ProjectHeader { p } => self.project(p).map_or(Entry::Blank, |entry| {
                    Entry::Heading(format!("project {}", entry.project.slug))
                }),
                Row::AppHeader => Entry::Heading(APP_HEADER.to_owned()),
                Row::BoxHeader => snapshot.this_box.as_ref().map_or(Entry::Blank, |entry| {
                    Entry::Heading(format!("this box ({})", entry.hostname))
                }),
                Row::Project { .. } | Row::App { .. } | Row::Box => {
                    let Some((target, stored, _)) = self.opening(*row) else {
                        return Entry::Blank;
                    };
                    let key = target.key();
                    let shown = shown(target, stored, snapshot);
                    Entry::Field(match editor.filter(|_| editing == Some(index)) {
                        Some(editor) => Field::new(label(key), form::Value::Text(&editor.input))
                            .help(format!("{} \u{b7} now {shown}", unit(key))),
                        None => Field::new(label(key), form::Value::ReadOnly(shown.into())),
                    })
                }
            })
            .collect();
        if let Some(editor) = editor
            && editing.is_none()
        {
            let key = editor.target.key();
            entries.push(Entry::Field(
                Field::new(label(key), form::Value::Text(&editor.input)).help(unit(key)),
            ));
        }
        (entries, focus)
    }

    /// The one line under the rows: the last outcome as a kit notice, in `theme.error` when it is
    /// one to act on (a compare-and-set miss, a refusal). The keys this mode binds are the
    /// Settings pane's hints ([`SettingsSection::key_hint`]); blank without one. Any mode: an
    /// editor's outcome is drawn here too.
    fn notice_line(&self, theme: &Theme) -> Line<'static> {
        let Some(notice) = &self.notice else {
            return Line::default();
        };
        let tone = match notice {
            Notice::Error(_) => Tone::Error,
            Notice::Info(_) => Tone::Info,
        };
        self::notice(notice.text(), tone, theme)
    }

    /// The stack of the current mode (MOD-67 D3, D4): the one place a mode maps to its keys, read
    /// by `key_stack`, the key handler and the hint.
    fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::QUEUE_BROWSE,
            Mode::Editing(_) => views::CAPTURE,
        }
    }

    /// The mode's keys for the shell's hint row, through the mode's stack (MOD-67 D9), plus a
    /// write in flight.
    fn hint_text(&self, bound: &Keys) -> String {
        let spec = match self.mode {
            Mode::Editing(_) => HINT_EDITING,
            Mode::Browse if self.unavailable.is_some() || self.snapshot.is_none() => {
                HINT_NO_SNAPSHOT
            }
            Mode::Browse => HINT_BROWSE,
        };
        let keys = bound.hint(self.stack(), spec);
        match self.busy {
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {}\u{2026}", doing(busy))
            }
            _ => keys,
        }
    }

    /// Reports an outcome, classified by the rule the sections share.
    fn say(&mut self, text: &str) {
        self.notice = Some(if super::is_error(text) {
            Notice::Error(text.to_owned())
        } else {
            Notice::Info(text.to_owned())
        });
    }

    /// Reports a refusal.
    fn refuse(&mut self, text: String) {
        self.notice = Some(Notice::Error(text));
    }
}

impl SettingsSection for QueueSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Queue"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::QueueSettings(scope.clone())]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // The rows belong to the workspace that was left; the notice survives, as
        // `PromptSection`'s does.
        self.snapshot = None;
        self.mode = Mode::Browse;
        self.busy = None;
        self.cursor = 0;
    }

    fn captures_input(&self) -> bool {
        matches!(self.mode, Mode::Editing(_))
    }

    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        editor.input.on_paste(text);
        Handled::Consumed
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
        // Browse (MOD-67 D6, skeleton (c)): the first candidate this state accepts. `edit` is `e`
        // and `Enter` here (the queue's view default, D12); a global act or a declined one is the
        // shell's.
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::QUEUE_BROWSE, chord) {
            match act {
                Act::Edit => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_edit(row, ctx.keys());
                    }
                }
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                Act::Reload => {
                    for request in self.wants_requests(ctx.scope) {
                        ctx.request(request);
                    }
                }
                Act::Dismiss if self.notice.is_some() => self.notice = None,
                _ => continue, // a global act, or one this state declines
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::QueueSettings(snapshot) => self.on_settings(snapshot),
            StoreReply::QueueSettingsStale(snapshot) => self.on_stale(snapshot, ctx.keys()),
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            // A refused write: the editor stays open over its text for a fix and a second `Enter`.
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                self.busy = None;
                self.refuse(message.clone());
            }
            _ => {}
        }
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let showing = self.unavailable.is_none() && self.snapshot.is_some();
        let help = if showing {
            wrapped(UNKNOWN_COST, usize::from(area.width).max(1))
        } else {
            Vec::new()
        };
        let [rows, help_area, hint] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(u16::try_from(help.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);
        self.scroll.set(None);
        let reload = ctx.keys().label(views::QUEUE_BROWSE, Act::Reload);
        match (&self.unavailable, &self.snapshot) {
            (Some(why), _) => {
                let mut lines: Vec<Line<'static>> = wrapped(
                    &format!("{UNAVAILABLE}: {why}"),
                    usize::from(rows.width).max(1),
                )
                .into_iter()
                .map(|line| notice(&line, Tone::Error, theme))
                .collect();
                // The next step in the empty state's own role (MOD-83).
                let next = asks_again(reload.as_deref());
                lines.extend(empty::lines("", NextStep(&next), theme).pop());
                frame.render_widget(Paragraph::new(lines), rows);
            }
            (None, None) => empty::render(
                frame,
                rows,
                NOT_READ,
                NextStep(&reads_again(reload.as_deref())),
                theme,
            ),
            (None, Some(snapshot)) => {
                let width = rows.width;
                let (entries, focus) = self.entries(snapshot);
                let editing = matches!(self.mode, Mode::Editing(_));
                let form = form_of(&entries, focus, editing);
                let total = form.lines(width, ctx.accent(), theme).len();
                // Scrolled so the focused row and its help are on screen, the row itself first
                // when both do not fit.
                let start = form.focus_line(width).unwrap_or(0);
                let through = entries.get(..=focus).unwrap_or(&entries);
                let end = form_of(through, focus, editing)
                    .lines(width, ctx.accent(), theme)
                    .len()
                    .saturating_sub(1);
                let offset =
                    list::scroll_into_view(end, 0, usize::from(rows.height), total).min(start);
                form.render(frame, rows, offset, ctx.accent(), theme);
                // The pane's bar is measured against the whole inner rect (D-11): the help and
                // notice lines under the rows are added, so it shows exactly when a row is hidden.
                let below = usize::from(area.height.saturating_sub(rows.height));
                self.scroll
                    .set(Some(ScrollSpan::new(total + below, offset)));
            }
        }
        if !help.is_empty() {
            let help: Vec<Line<'static>> = help
                .into_iter()
                .map(|line| Line::styled(line, theme.dim))
                .collect();
            frame.render_widget(Paragraph::new(help), help_area);
        }
        frame.render_widget(Paragraph::new(self.notice_line(theme)), hint);
    }
}

/// A row's label.
const fn label(key: QueueSetting) -> &'static str {
    match key {
        QueueSetting::PerTokenCapRun => "run cap",
        QueueSetting::PerTokenCapBatch => "batch cap",
        QueueSetting::MinBudgetForNewAttempt => "min budget",
        QueueSetting::MaxConcurrentItems => "max concurrent",
        QueueSetting::SchedulerWindow => "window",
    }
}

/// What the editor's label says the field takes.
const fn unit(key: QueueSetting) -> &'static str {
    match key {
        QueueSetting::PerTokenCapRun
        | QueueSetting::PerTokenCapBatch
        | QueueSetting::MinBudgetForNewAttempt => "USD",
        QueueSetting::MaxConcurrentItems => "runs",
        QueueSetting::SchedulerWindow => "HH:MM-HH:MM",
    }
}

/// What a value row shows (D8).
fn shown(target: Target, stored: Option<&Value>, snapshot: &QueueSettingsSnapshot) -> String {
    let key = target.key();
    match (target, stored) {
        (_, Some(value)) if key.is_money() => {
            value.as_i64().map_or_else(|| value.to_string(), format_usd)
        }
        (Target::Project(..), None) => UNBOUNDED.to_owned(),
        (Target::App(QueueSetting::MinBudgetForNewAttempt), None) => NONE.to_owned(),
        (Target::App(QueueSetting::SchedulerWindow), stored) => {
            let window = match stored {
                Some(value) => format_window(value).unwrap_or_else(|| value.to_string()),
                None => NOT_SET.to_owned(),
            };
            format!("{window} \u{b7} {WINDOW_NOT_ENFORCED}")
        }
        (Target::App(_), None) => format!("{DEFAULT_MAX_CONCURRENT_ITEMS} (default)"),
        (Target::Box(_), None) => {
            let effective = snapshot
                .this_box
                .as_ref()
                .map_or_else(|| snapshot.app_limit(), |entry| entry.effective);
            format!("inherit ({effective})")
        }
        (_, Some(value)) => value.to_string(),
    }
}

/// A stored value as the editor is prefilled with it: dollars without the `$`, the window as
/// `HH:MM-HH:MM`, anything else as its JSON.
fn typed(key: QueueSetting, value: &Value) -> String {
    if key.is_money()
        && let Some(micros) = value.as_i64()
    {
        let dollars = format_usd(micros);
        return dollars.strip_prefix('$').unwrap_or(&dollars).to_owned();
    }
    if key == QueueSetting::SchedulerWindow
        && let Some(window) = format_window(value)
    {
        return window;
    }
    value.to_string()
}

/// Typed text as the JSON the key stores; the validator runs after.
fn parse(key: QueueSetting, text: &str) -> Result<Value, String> {
    match key {
        QueueSetting::PerTokenCapRun
        | QueueSetting::PerTokenCapBatch
        | QueueSetting::MinBudgetForNewAttempt => parse_usd(text).map(|micros| json!(micros)),
        // `u64`, so a limit past `u32::MAX` reaches the validator and its sentence.
        QueueSetting::MaxConcurrentItems => text
            .parse::<u64>()
            .map(|limit| json!(limit))
            .map_err(|_| WHOLE_NUMBER.to_owned()),
        QueueSetting::SchedulerWindow => parse_window(text),
    }
}

/// What a second write is told while the first is still out.
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// What a write in flight is doing, in words (MOD-83): never its request name.
fn doing(busy: &str) -> &'static str {
    if busy == REQUEST_NAMES[1] {
        "saving the setting"
    } else if busy == REQUEST_NAMES[2] {
        "clearing the setting"
    } else {
        "writing"
    }
}

/// The token an open editor retries against after a reload.
fn reload(snapshot: &QueueSettingsSnapshot, target: Target) -> Reload {
    match target {
        Target::App(key) => snapshot
            .app_entry(key)
            .map_or(Reload::Gone, |entry| Reload::Token(entry.token)),
        Target::Project(id, _) => snapshot
            .projects
            .iter()
            .find(|entry| entry.project.id == id)
            .map_or(Reload::Gone, |entry| Reload::Token(entry.token())),
        Target::Box(id) => snapshot
            .this_box
            .as_ref()
            .filter(|entry| entry.id == id)
            .map_or(Reload::Gone, |entry| Reload::Token(entry.token)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-67 M6 D4: the header refusal names the edit key, and none unbound.
    #[test]
    fn not_a_value_row_names_the_edit_key_or_none() {
        assert_eq!(not_a_value_row(Some("e")), "`e` edits a value row");
        assert_eq!(not_a_value_row(None), "only a value row is edited");
    }

    /// The editor's `Debug` names the target and the token, never the buffer.
    #[test]
    fn an_editor_never_prints_its_buffer() {
        let section = QueueSection {
            mode: Mode::Editing(Editor {
                target: Target::App(QueueSetting::MinBudgetForNewAttempt),
                input: TextField::with_text("424242"),
                expected: QueueToken::Stamp(None),
            }),
            ..QueueSection::new()
        };

        let printed = format!("{section:?}");

        assert!(!printed.contains("424242"), "{printed}");
        assert!(printed.contains("MinBudgetForNewAttempt"), "{printed}");
    }

    /// The prefill is the text `parse` reads back to the same JSON.
    #[test]
    fn the_prefill_parses_back_to_what_is_stored() {
        for (key, value) in [
            (QueueSetting::PerTokenCapBatch, json!(1_500_000)),
            (QueueSetting::MinBudgetForNewAttempt, json!(1)),
            (QueueSetting::MaxConcurrentItems, json!(3)),
            (
                QueueSetting::SchedulerWindow,
                json!({"start": "22:00", "end": "06:00"}),
            ),
        ] {
            assert_eq!(parse(key, &typed(key, &value)), Ok(value), "{key}");
        }
        assert_eq!(
            typed(QueueSetting::PerTokenCapRun, &json!(1_500_000)),
            "1.50"
        );
        assert_eq!(
            parse(QueueSetting::MaxConcurrentItems, "-1"),
            Err(WHOLE_NUMBER.to_owned())
        );
    }
}
