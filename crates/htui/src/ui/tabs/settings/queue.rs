//! The queue section of the Settings tab (MOD-12 milestone 2, D8-D10): the spend caps of each
//! scope project, the three box-wide `app_setting` keys and this box's concurrency limit.
//!
//! It holds **no store handle** (`R-NF-3`): it names one read ([`StoreRequest::QueueSettings`]),
//! is handed the snapshot that comes back, and every write leaves through `ctx.request` for
//! [`crate::queue_settings::serve`] to carry out. What is on screen is always the last snapshot the
//! worker assembled; no row is patched in locally.
//!
//! Money is typed and shown in USD and stored as micros ([`parse_usd`], [`format_usd`]); the
//! window is typed `HH:MM-HH:MM` ([`parse_window`]). Every bound is the key's own validator
//! ([`QueueSetting::validate`]), checked here before a request is sent and again by the store, and
//! its sentence is shown verbatim. Empty clears: a cap becomes unbounded, the box limit inherits
//! the app default.

use htui_core::model::{
    BoxId, DEFAULT_MAX_CONCURRENT_ITEMS, ProjectId, QueueSetting, Scope, format_usd, format_window,
    parse_usd, parse_window,
};
use htui_core::store::{QueueTarget, QueueToken};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use serde_json::{Value, json};

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::queue_settings::{QueueProjectEntry, QueueSettingsSnapshot, READ_NAME, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    message, modal_rest, wrapped,
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

/// An open editor's keys: the field's own (MOD-67 D13).
pub const HINT_EDITING: HintSpec = &[
    Hint::Text("Enter save"),
    Hint::Text("Esc cancel"),
    Hint::Text("empty clears"),
];

/// What the rows pane says before any settings have arrived.
pub const NOT_READ: &str = "queue settings not read yet";

/// What the rows pane says when the read itself was refused, before the refusal's sentence.
pub const UNAVAILABLE: &str = "queue settings unavailable";

/// What `e` says on a group header.
pub const NOT_A_VALUE_ROW: &str = "`e` edits a value row";

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
    /// The last outcome, one line on the hint row.
    notice: Option<Notice>,
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
    fn open_edit(&mut self, row: Row) {
        let Some((target, stored, expected)) = self.opening(row) else {
            self.say(NOT_A_VALUE_ROW);
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
            FieldOutcome::Pass => modal_rest(views::CAPTURE, KeyChord::from_event(key)),
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
        if write.is_some() {
            self.notice = None;
            self.mode = Mode::Browse;
        }
    }

    /// A compare-and-set miss: adopt the reload, keep the editor's text, retake its token, and
    /// leave the retry to the next `Enter`.
    fn on_stale(&mut self, snapshot: &QueueSettingsSnapshot) {
        self.busy = None;
        self.unavailable = None;
        let reloaded = match &self.mode {
            Mode::Editing(editor) => Some(reload(snapshot, editor.target)),
            Mode::Browse => None,
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
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
                self.say(CHANGED_ELSEWHERE);
            }
        }
    }

    /// One line per row, in [`rows`](QueueSection::rows) order.
    fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for (index, row) in self.rows().into_iter().enumerate() {
            let style = if index == self.cursor {
                theme.selected
            } else {
                theme.base
            };
            let text = match row {
                Row::ProjectHeader { p } => self
                    .project(p)
                    .map(|entry| format!("project {}", entry.project.slug)),
                Row::AppHeader => Some(APP_HEADER.to_owned()),
                Row::BoxHeader => snapshot
                    .this_box
                    .as_ref()
                    .map(|entry| format!("this box ({})", entry.hostname)),
                Row::Project { .. } | Row::App { .. } | Row::Box => {
                    self.opening(row).map(|(target, stored, _)| {
                        value_line(label(target.key()), &shown(target, stored, snapshot))
                    })
                }
            };
            if let Some(text) = text {
                lines.push(Line::styled(text, style));
            }
        }
        lines
    }

    /// The open editor's field, one line; nothing in Browse.
    fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let Mode::Editing(editor) = &self.mode else {
            return Vec::new();
        };
        let key = editor.target.key();
        let label = format!("{} ({}): ", label(key), unit(key));
        let room = usize::from(width).saturating_sub(cell_width(&label));
        let mut spans = vec![Span::styled(label, theme.accent)];
        spans.extend(
            editor
                .input
                .line(u16::try_from(room).unwrap_or(u16::MAX), true, theme)
                .spans,
        );
        vec![Line::from(spans)]
    }

    /// The keys this mode binds, then the last outcome; the outcome wins when both do not fit.
    fn hint(&self, bound: &Keys, width: u16, theme: &Theme) -> Line<'static> {
        let keys = self.hint_text(bound);
        let Some(notice) = &self.notice else {
            return Line::styled(keys, theme.dim);
        };
        let style = match notice {
            Notice::Error(_) => theme.error,
            Notice::Info(_) => theme.dim,
        };
        let text = notice.text().to_owned();
        if cell_width(&keys) + cell_width(&text) + 3 > usize::from(width) {
            return Line::styled(text, style);
        }
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(text, style),
        ])
    }

    /// The stack of the current mode (MOD-67 D3, D4): the one place a mode maps to its keys, read
    /// by `key_stack`, the key handler and the hint.
    fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::QUEUE_BROWSE,
            Mode::Editing(_) => views::CAPTURE,
        }
    }

    /// The keys half of the hint line, through the mode's stack (MOD-67 D9), plus a write in
    /// flight.
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
                format!("{keys} \u{b7} {busy} in flight")
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
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::QUEUE_BROWSE, chord) {
            match act {
                Act::Edit => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_edit(row);
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

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::QueueSettings(snapshot) => self.on_settings(snapshot),
            StoreReply::QueueSettingsStale(snapshot) => self.on_stale(snapshot),
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

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let showing = self.unavailable.is_none() && self.snapshot.is_some();
        let help = if showing {
            wrapped(UNKNOWN_COST, usize::from(area.width).max(1))
        } else {
            Vec::new()
        };
        let pane = self.pane(area.width, ctx.theme);
        let [rows, help_area, pane_area, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(help.len()).unwrap_or(u16::MAX)),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        match (&self.unavailable, &self.snapshot) {
            (Some(why), _) => frame.render_widget(
                Paragraph::new(Line::styled(
                    format!("{UNAVAILABLE}: {why}"),
                    ctx.theme.error,
                ))
                .wrap(Wrap { trim: true }),
                rows,
            ),
            (None, None) => message(frame, rows, NOT_READ, ctx.theme),
            (None, Some(_)) => {
                let height = usize::from(rows.height);
                let offset = self.cursor.saturating_sub(height.saturating_sub(1));
                frame.render_widget(
                    Paragraph::new(self.lines(ctx.theme))
                        .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
                    rows,
                );
            }
        }
        if !help.is_empty() {
            let help: Vec<Line<'static>> = help
                .into_iter()
                .map(|line| Line::styled(line, ctx.theme.dim))
                .collect();
            frame.render_widget(Paragraph::new(help), help_area);
        }
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), pane_area);
        }
        frame.render_widget(
            Paragraph::new(self.hint(ctx.keys(), area.width, ctx.theme)),
            hint,
        );
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

/// One value row: the label, padded so the values line up, then the value.
fn value_line(label: &str, value: &str) -> String {
    format!("  {label:<14}  {value}")
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
    format!("`{busy}` is still in flight")
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
