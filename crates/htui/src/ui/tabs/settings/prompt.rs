//! The prompt section of the Settings tab: the registry's keys on the rungs their specs admit,
//! each showing what the rung stores, what the reader would use, and which rung answered
//! (MOD-15 milestone 5, D5/D6/D9/D10/D11/D12/D13; `R-TUI-8`, `R-ENT-10`, `R-PRM-*`).
//!
//! It holds **no store handle, no `UserId` and no `BoxId`** (`R-NF-3`): it names one read
//! ([`StoreRequest::PromptSettings`]), is handed the settings that come back, and every write
//! leaves through `ctx.request` for [`crate::prompt_settings::serve`] to carry out. What is on
//! screen is always the last snapshot the worker assembled — no row is ever patched in locally, so
//! there is exactly one source of truth (D3).
//!
//! Everything a row says comes out of the registry: the key, the label, the unit, the range, the
//! doc line and the rungs it accepts are read from
//! [`SettingKey::ALL`](htui_core::prompt::SettingKey::ALL) and
//! [`SettingKey::spec`](htui_core::prompt::SettingKey::spec), so a key of an existing kind appears
//! here without this file being touched. Two exceptions: the **effective** column needs each key's
//! own resolver and is therefore an exhaustive `match key` with no wildcard, so a new key is a
//! compile error naming the missing arm rather than a silent blank (blueprint flag G, O-4); and a
//! new [`SettingKind`] needs its parse in `submit` and its line in the pane — MOD-33's `Boolean`
//! did.
//!
//! The section parses **shape** and nothing else (D11). Every bound — `min`, `max`, `not_above`,
//! the phase narrowing — belongs to
//! [`validate`](htui_core::prompt::validate), and its sentence is shown verbatim. A range checked
//! twice is a range that can disagree with itself.
//!
//! On the kit (MOD-115 M3 L-prompt): the tree is one `kit::form` of read-only rows under group
//! headings, the focused row marked by the form's gutter in the tab's accent; the registry's doc,
//! range and rungs are the focused row's help on the line under it, and a clamp is an error under
//! the row it concerns. An open editor is the row it edits. The source column reads in words
//! (`kit::label::setting_source`), the reserve in percent beside the fraction it is typed as, and
//! every key a hint or a sentence names is the key in force, resolved from `Ctx::pressed` (MD-A).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use htui_core::model::Scope;
use htui_core::prompt::settings::{
    resolve_allow_env_files, resolve_box_hostname, resolve_budget, resolve_excerpt_caps,
    resolve_hops, resolve_max_skill_tokens,
};
use htui_core::prompt::{Budget, BudgetSource, SettingKey, SettingKind};
use htui_core::store::SettingRung;
use serde_json::Value;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::prompt_settings::{AppEntry, ProjectEntry, READ_NAME, REQUEST_NAMES, SettingsSnapshot};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Field, Form};
use crate::ui::kit::label;
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection, changed_elsewhere,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use chrono::{DateTime, Utc};
use crossterm::event::KeyEvent;

/// What the rows pane says before any settings have arrived.
const NOT_READ: &str = "settings not read yet";

/// What the rows pane says when the read itself was refused.
const UNAVAILABLE: &str = "settings unavailable";

/// The next step of [`NOT_READ`]: the reload key in force, or what reads the settings without one.
fn reads_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "the settings are read again when the section opens".to_owned(),
        |key| format!("{key} reads the settings again"),
    )
}

/// The next step of [`UNAVAILABLE`]: the reload key in force, or when the read is asked again.
fn asks_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "they are asked for again when the section opens".to_owned(),
        |key| format!("{key} asks again once the store answers"),
    )
}

/// What a rung that holds no value of its own shows.
///
/// One word for both rungs: the kinds section's `inherit` would be wrong on `App`, which has no
/// rung above it — only the compiled table below (B-7).
const UNSET: &str = "unset";

/// Browse's keys, with something to browse: `j/k · e edit · r reload` by default.
const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::Reload, "reload"),
];

/// Browse's keys with nothing read, or the read refused: the only offer is to ask again.
const HINT_NO_SNAPSHOT: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// An open editor's keys: the field's own submit and cancel through `CAPTURE`'s `[field]` layer
/// (MOD-67 D13; MOD-115 M3 MD-A, C6), `Enter save · Esc cancel` with the compiled keys. One
/// field, so there is no `Tab`.
const HINT_EDITING: HintSpec = &[
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::Text("empty clears"),
];

/// What `e` says on a group header (B-1). `edit` is `Edit`'s label through `PROMPT_BROWSE`
/// (MOD-67 M6 D4); with none bound the sentence names no key.
fn not_a_value_row(edit: Option<&str>) -> String {
    match edit {
        Some(edit) => format!("`{edit}` edits a value row"),
        None => "only a value row is edited".to_owned(),
    }
}

/// What an empty field over a rung that holds nothing says: there is nothing to clear, so there is
/// no request to send (D10).
const NOTHING_SET: &str = "nothing is set on this rung";

/// The `App` group's line.
const APP_HEADER: &str = "app";

/// The prefix of a project group's line (B-1).
const PROJECT_HEADER: &str = "project";

/// What a [`SettingKind::Boolean`] row's effective column says when the switch is on.
const SWITCH_ON: &str = "on";

/// …and when it is off.
const SWITCH_OFF: &str = "off";

/// The unit column of a [`SettingKind::Fraction`] key: what it is typed as (MOD-83). The registry
/// counts its range in basis points, which the help line keeps.
const FRACTION_UNIT: &str = "fraction";

/// What a [`SettingKind::Boolean`] row's pane says in place of a numeric range.
const SWITCH_RANGE: &str = "on or off";

/// One line of the tree, by index into the snapshot (D9).
///
/// Indices rather than ids, for the kinds section's reason: the list is rebuilt from the snapshot
/// on every reply, and an index that outlives its tree is caught by the clamp, where a stale id
/// would silently select nothing. The two headers are rows of their own (B-1), so a row's index is
/// its line's index and scrolling needs no skip list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The `app` group line.
    AppHeader,
    /// `snapshot.app[i]`.
    App {
        /// Index into `snapshot.app`.
        i: usize,
    },
    /// The `project {name}` group line.
    ProjectHeader {
        /// Index into `snapshot.projects`.
        p: usize,
    },
    /// `snapshot.projects[p].values[v]`.
    Project {
        /// Index into `snapshot.projects`.
        p: usize,
        /// Index into that project's `values`.
        v: usize,
    },
}

/// What an editor writes back to: one rung, one key (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    /// Which rung the write lands on; never `Phase` from here (D8).
    rung: SettingRung,
    /// Which key.
    key: SettingKey,
}

/// The open editor: the target, its one field, and the token it opened on.
struct Editor {
    /// The row this writes back to.
    target: Target,
    /// The buffer; its `Debug` never prints the text.
    input: TextField,
    /// The rung row's `updated_at` when this opened; `None` for an absent `App` row (D4, F-2).
    expected: Option<DateTime<Utc>>,
}

/// The target and the token, never the buffer (H-5).
///
/// Hand-written because [`PromptSection`] derives `Debug` and one `tracing::debug!` of a section is
/// all it takes for a field's text to reach a log. [`TextField`]'s own `Debug` holds the same line.
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("target", &self.target)
            .field("expected", &self.expected)
            .finish()
    }
}

/// What the section is doing. `Browse` is not a mode in the modal sense: it captures nothing.
#[derive(Default)]
enum Mode {
    /// The rows, the cursor and the tab's own `h`/`l`.
    #[default]
    Browse,
    /// One row being typed into.
    Editing(Editor),
}

/// Everything the mode is *about*, never what was typed into it (H-5).
///
/// Hand-written for the same reason [`Editor`]'s is: [`PromptSection`] derives `Debug` through this
/// enum, so a variant that printed its own field's text would need only one `tracing::debug!` of a
/// section to put a typed value in a log.
impl core::fmt::Debug for Mode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Browse => f.write_str("Browse"),
            Self::Editing(editor) => f.debug_tuple("Editing").field(editor).finish(),
        }
    }
}

/// The last outcome, and whether it is one the user has to act on.
///
/// Carried with the text rather than derived from it: a refusal is usually the **seam's** own
/// sentence, and no string rule can tell one of those from a line of good news. The sentences this
/// section shares with the kinds section are classified by the shared
/// [`is_error`](super::is_error), so the two cannot drift apart.
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

    /// Whether it belongs in `theme.error`.
    fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// What a reloaded snapshot does to an open editor's token (D14, B-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reload {
    /// The rung the editor opened on is not in the reloaded snapshot.
    Gone,
    /// The rung row's `updated_at` as it is now.
    Token(DateTime<Utc>),
    /// An `App` row that no longer exists: the next set passes `expected: None`.
    ///
    /// The kinds section's create-style `Keep` would carry the dead token instead, and a set that
    /// carried one over a row that is gone is refused rather than applied (H-1).
    NoRow,
}

/// What `e` on one row would open: where the write lands, what that rung stores now, and the token
/// the write would compare against.
///
/// One named shape rather than the three-tuple the blueprint spells: the same fact, and the caller
/// cannot mix the stored value up with the token.
struct Opening<'a> {
    /// The rung and key a write would carry.
    target: Target,
    /// What the rung stores, or `None` when it holds nothing.
    stored: Option<&'a Value>,
    /// The compare-and-set token, `None` for an absent `App` row (D4, F-2).
    expected: Option<DateTime<Utc>>,
}

/// The reader's answer for one row: what it would use, from which rung, and any note it made.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Effective {
    /// As the row prints it (`5000`, `10% (0.1)`).
    text: String,
    /// The same as a number, for D13's comparison; `None` for the one fraction key, whose stored
    /// unit and resolved unit are not the same thing.
    number: Option<i64>,
    /// Which rung answered, in words ([`label::setting_source`]: `default`, `app setting`).
    source: &'static str,
    /// [`resolve_hops`]'s clamp note, when it made one (blueprint flag B).
    notes: Vec<String>,
}

/// The prompt settings of the scope, with the keys that edit them (D9/D10).
#[derive(Debug, Default)]
pub struct PromptSection {
    /// The last settings the worker assembled, or `None` before the first reply.
    snapshot: Option<SettingsSnapshot>,
    /// `Some(message)` after `Failed { request: "prompt_settings" }`: the read itself was refused.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](PromptSection::rows).
    cursor: usize,
    /// Browsing, or typing into one row.
    mode: Mode,
    /// The write in flight, by [`StoreRequest::name`]. A second one is refused until the reply: the
    /// staleness index keeps only the newest request of a kind, so two writes of one kind racing
    /// would lose the reply about the one that landed (H-4).
    busy: Option<&'static str>,
    /// The last outcome, on the notice row under the pane.
    notice: Option<Notice>,
    /// The form's lines, first line drawn and rows as of the last render, for the Settings
    /// pane's scrollbar (MOD-115 M3 D-11, RV1 M2); `None` while nothing scrolls.
    scroll: Cell<Option<ScrollSpan>>,
}

/// One item of the tree's form, in [`rows`](PromptSection::rows) order: built once per frame
/// and folded into one [`Form`] (RV2 L3).
#[derive(Clone)]
enum Entry<'a> {
    /// A group line (`app`, `project {name}`).
    Heading(String),
    /// A value row, or the open editor's field.
    Field(Field<'a>),
    /// A row the snapshot no longer has: kept, so an item's index is still its row's.
    Blank,
}

/// `entries` as one form, `focus` focused, the label column the registry's widest key wide.
fn form_of<'a>(entries: Vec<Entry<'a>>, focus: usize, editing: bool) -> Form<'a> {
    let width = u16::try_from(*KEY_WIDTH + 1).unwrap_or(u16::MAX);
    entries.into_iter().fold(
        Form::new().label_width(width).focus(focus).editing(editing),
        |form, entry| match entry {
            Entry::Heading(text) => form.heading(text),
            Entry::Field(field) => form.field(field),
            Entry::Blank => form.blank(),
        },
    )
}

impl PromptSection {
    /// Identity of the prompt section.
    pub const ID: SectionId = SectionId("prompt");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The flat list the cursor indexes (D9).
    ///
    /// Derived on demand rather than cached beside the snapshot, so the two cannot disagree about
    /// what is on screen (D3). The project rows come from `values.len()` and never from a
    /// recomputed key list, so the section and the worker cannot disagree about which keys a
    /// project carries or in what order (H-8).
    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut rows = vec![Row::AppHeader];
        rows.extend((0..snapshot.app.len()).map(|i| Row::App { i }));
        for (p, entry) in snapshot.projects.iter().enumerate() {
            rows.push(Row::ProjectHeader { p });
            rows.extend((0..entry.values.len()).map(|v| Row::Project { p, v }));
        }
        rows
    }

    /// Moves the cursor one row and stops at the end it reaches.
    ///
    /// No wrap, for the reason the other two sections give: a key acts on the row the cursor is
    /// on, and a held `j` that wrapped to the top would aim it at a row nobody looked at.
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

    /// The row under the cursor, or `None` while there is no tree.
    fn selected(&self) -> Option<Row> {
        self.rows().get(self.cursor).copied()
    }

    /// One `App` entry of the snapshot.
    fn app(&self, i: usize) -> Option<&AppEntry> {
        self.snapshot.as_ref()?.app.get(i)
    }

    /// One project of the snapshot.
    fn project(&self, p: usize) -> Option<&ProjectEntry> {
        self.snapshot.as_ref()?.projects.get(p)
    }

    /// A value row's key, what its rung stores, and what the reader would use (D6).
    ///
    /// `None` on a header: those carry no value and open no editor.
    fn value_at<'a>(
        &'a self,
        row: Row,
        app: &BTreeMap<String, Value>,
    ) -> Option<(SettingKey, Option<&'a Value>, Effective)> {
        match row {
            Row::AppHeader | Row::ProjectHeader { .. } => None,
            Row::App { i } => {
                let entry = self.app(i)?;
                let effective = effective(entry.key, None, None, app);
                Some((entry.key, entry.value.as_ref(), effective))
            }
            Row::Project { p, v } => {
                let entry = self.project(p)?;
                let value = entry.values.get(v)?;
                // The whole blob, because that is what the reader's own resolvers take (D5/D6);
                // the row's own value is passed beside it for the presence rule (B-3).
                let effective = effective(
                    value.key,
                    Some(&entry.project.settings),
                    value.value.as_ref(),
                    app,
                );
                Some((value.key, value.value.as_ref(), effective))
            }
        }
    }

    /// What an editor opened on `row` would write to, what that rung stores, and the token it
    /// would compare against. `None` on a header.
    ///
    /// On the `App` rung the token is the entry's own and is `None` exactly when there is no row;
    /// on a project it is the **project row's**, held once per project rather than per key, because
    /// the write is a key-level merge into one JSONB column (D4).
    fn target_of(&self, row: Row) -> Option<Opening<'_>> {
        match row {
            Row::AppHeader | Row::ProjectHeader { .. } => None,
            Row::App { i } => {
                let entry = self.app(i)?;
                Some(Opening {
                    target: Target {
                        rung: SettingRung::App,
                        key: entry.key,
                    },
                    stored: entry.value.as_ref(),
                    expected: entry.updated_at,
                })
            }
            Row::Project { p, v } => {
                let entry = self.project(p)?;
                let value = entry.values.get(v)?;
                Some(Opening {
                    target: Target {
                        rung: SettingRung::Project(entry.project.id),
                        key: value.key,
                    },
                    stored: value.value.as_ref(),
                    expected: Some(entry.project.updated_at),
                })
            }
        }
    }

    /// Whether the target's rung holds a value **now**; `None` when the row is gone from the tree.
    ///
    /// Read off the current snapshot rather than remembered on the editor: a reload may have
    /// cleared the rung under it, and an empty field then has nothing left to clear (D10).
    fn holds(&self, target: Target) -> Option<bool> {
        match target.rung {
            SettingRung::App => self
                .snapshot
                .as_ref()?
                .app_entry(target.key)
                .map(|entry| entry.value.is_some()),
            SettingRung::Project(id) => self
                .snapshot
                .as_ref()?
                .projects
                .iter()
                .find(|entry| entry.project.id == id)
                .and_then(|entry| {
                    entry
                        .values
                        .iter()
                        .find(|value| value.key == target.key)
                        .map(|value| value.value.is_some())
                }),
            // Never opened here (D8), so there is nothing to answer about.
            SettingRung::Phase(_) => None,
        }
    }

    /// Whether the key that opens an editor is refused right now, with the notice that says why.
    ///
    /// One write of a kind at a time (M3's rule). `r` is deliberately not on this path —
    /// re-reading is how a section that lost a reply recovers.
    ///
    /// `unavailable` blocks as firmly as a missing tree: `render` hides the rows behind the
    /// refusal and the hint drops to `r reload`, so an `e` that still opened an editor would aim
    /// it at a row nobody can see, over a store whose last word was that it could not answer.
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return true;
        }
        self.snapshot.is_none() || self.unavailable.is_some()
    }

    /// `e`: the row under the cursor, prefilled with what the rung stores.
    ///
    /// Empty when the rung holds nothing, which is the same field an `Enter` reads as "clear"
    /// — and on an unset rung that is not a write at all (D10).
    fn open_edit(&mut self, row: Row, keys: &Keys) {
        let Some(opening) = self.target_of(row) else {
            self.say(&not_a_value_row(
                keys.label(views::PROMPT_BROWSE, Act::Edit).as_deref(),
            ));
            return;
        };
        let editor = Editor {
            target: opening.target,
            input: TextField::with_text(&opening.stored.map_or_else(String::new, Value::to_string)),
            expected: opening.expected,
        };
        self.notice = None;
        self.mode = Mode::Editing(editor);
    }

    /// Sends one write and remembers its name until the reply.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// One key while an editor is open.
    ///
    /// The field answers first, so `l`, `q` and the digits are letters here; everything it passes
    /// on is swallowed rather than offered to the shell — except the chords [`views::CAPTURE`]
    /// passes (CONTROL, ALT, function keys: MOD-67 D5), so `ctrl-c` still quits and `F1` opens
    /// help.
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

    /// `Enter` in the editor: clear on an empty field, otherwise one shape check and a set (D10,
    /// D11).
    ///
    /// Nothing else is checked here. `min`, `max`, `not_above` and the phase narrowing are
    /// [`validate`](htui_core::prompt::validate)'s, and its sentence comes back verbatim in
    /// `Failed`. The editor **stays open** until the reply lands, so a refusal leaves the text
    /// where it was and a second `Enter` retries it — which is why the first statement is the
    /// refusal the Browse keys get.
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
        let Some(holds) = self.holds(target) else {
            self.mode = Mode::Browse;
            self.say(DELETED_ELSEWHERE);
            return;
        };

        if text.is_empty() {
            if !holds {
                self.say(NOTHING_SET);
                return;
            }
            // Unreachable by the `value` ⇔ `updated_at` invariant of an `AppEntry` and by a project
            // row always carrying its project's token — a guard rather than an `expect`.
            let Some(expected) = expected else {
                self.say(NOTHING_SET);
                return;
            };
            self.notice = None;
            self.send(
                StoreRequest::ClearSetting {
                    scope: ctx.scope.clone(),
                    rung: target.rung,
                    key: target.key,
                    expected,
                },
                ctx,
            );
            return;
        }

        let value = match target.key.spec().kind {
            SettingKind::Integer => text
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| integer_sentence(target.key)),
            // `Value::from` on a NaN or an infinity is `Null`, which the seam would refuse as "not
            // a finite JSON number" — a worse sentence than this one, and one round trip later
            // (H-6).
            SettingKind::Fraction => text
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .map(Value::from)
                .ok_or_else(|| fraction_sentence(target.key)),
            SettingKind::Boolean => parse_switch(&text)
                .map(Value::Bool)
                .ok_or_else(|| switch_sentence(target.key)),
        };
        match value {
            Ok(value) => {
                self.notice = None;
                self.send(
                    StoreRequest::SetSetting {
                        scope: ctx.scope.clone(),
                        rung: target.rung,
                        key: target.key,
                        value,
                        expected,
                    },
                    ctx,
                );
            }
            Err(sentence) => self.refuse(sentence),
        }
    }

    /// A compare-and-set miss (D14, PRD D8): the tree is replaced, the editor keeps its text and
    /// takes the current row's token, and the retry is a second `Enter` rather than an automatic
    /// write. The sentence names the retry key as the editor's stack binds it (MD-A, D-5).
    fn on_stale(&mut self, snapshot: &SettingsSnapshot, keys: &Keys) {
        self.busy = None;
        let reloaded = match &self.mode {
            Mode::Editing(editor) => Some(reload(snapshot, editor.target)),
            Mode::Browse => None,
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        self.follow_editor();
        match reloaded {
            // Nothing is open to press `Enter` on, so the sentence has to carry what the editor
            // would otherwise stand for: the write did not apply, and the way back is to reopen.
            None => self.say(CHANGED_ELSEWHERE_CLOSED),
            Some(Reload::Gone) => {
                self.mode = Mode::Browse;
                self.say(DELETED_ELSEWHERE);
            }
            Some(token) => {
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.expected = match token {
                        Reload::Token(token) => Some(token),
                        Reload::Gone | Reload::NoRow => None,
                    };
                }
                let retry = keys.label(views::CAPTURE, Act::FieldSubmit);
                self.say(&changed_elsewhere(retry.as_deref()));
            }
        }
    }

    /// The open editor's row in [`rows`](PromptSection::rows), or `None` with no editor or with
    /// its row gone from the tree.
    fn editing_row(&self) -> Option<usize> {
        let Mode::Editing(editor) = &self.mode else {
            return None;
        };
        self.rows().into_iter().position(|row| {
            self.target_of(row)
                .is_some_and(|opening| opening.target == editor.target)
        })
    }

    /// Puts the cursor on the open editor's row after a reload moved the rows under it, so the
    /// row being typed into is the one marked and the one `Esc` leaves the cursor on.
    fn follow_editor(&mut self) {
        if let Some(row) = self.editing_row() {
            self.cursor = row;
        }
    }

    /// The tree as form items, in [`rows`](PromptSection::rows) order (D9), and the item focused.
    ///
    /// Every value row is read-only and wraps under its column (MOD-81). The focused one carries
    /// the registry's own lines as its help (B-6): the doc, the range in its unit and the rungs
    /// the key accepts; a clamp or a resolver's note is an error under the row it concerns, shown
    /// whatever has focus (D13). An open editor is the row it edits, with what the row holds now
    /// in its help, so a reload under it stays on screen; when that row is gone from the tree the
    /// field is drawn after the last row, so nobody types into a field they cannot see.
    ///
    /// `width` is the form's: a key wider than the label column there (a third of the width,
    /// MOD-60) is cut with `…`, so the focused row's help names it whole.
    fn entries(&self, snapshot: &SettingsSnapshot, width: u16) -> (Vec<Entry<'_>>, usize) {
        let column = usize::from(form_of(Vec::new(), 0, false).label_column(width));
        let help = |key: SettingKey, now: Option<&str>| {
            let cut = cell_width(key.key()) > column;
            help_text(key, now, cut)
        };
        // Ten entries, rebuilt per frame rather than cached: the snapshot is replaced whole on
        // every reply, and a cache beside it would be the second source of truth D3 refuses.
        let app = snapshot.app_map();
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
                Row::AppHeader => Entry::Heading(APP_HEADER.to_owned()),
                Row::ProjectHeader { p } => self.project(p).map_or(Entry::Blank, |entry| {
                    Entry::Heading(format!("{PROJECT_HEADER} {}", entry.project.name))
                }),
                Row::App { .. } | Row::Project { .. } => {
                    let Some((key, stored, effective)) = self.value_at(*row, &app) else {
                        return Entry::Blank;
                    };
                    let shown = value_text(key, stored, &effective);
                    let open = editor.filter(|_| editing == Some(index));
                    let mut field = match open {
                        Some(editor) => Field::new(key.key(), form::Value::Text(&editor.input))
                            .help(help(key, Some(&shown))),
                        None => {
                            let field = Field::new(key.key(), form::Value::ReadOnly(shown.into()));
                            if index == focus {
                                field.help(help(key, None))
                            } else {
                                field
                            }
                        }
                    };
                    if let Some(error) = row_error(key, stored, &effective) {
                        field = field.error(error);
                    }
                    Entry::Field(field)
                }
            })
            .collect();
        if let Some(editor) = editor
            && editing.is_none()
        {
            let key = editor.target.key;
            entries.push(Entry::Field(
                Field::new(key.key(), form::Value::Text(&editor.input)).help(help(key, None)),
            ));
        }
        (entries, focus)
    }

    /// The one line under the tree: the last outcome as a kit notice, in `theme.error` when it is
    /// one to act on rather than read (a compare-and-set miss). The keys this mode binds are the
    /// Settings pane's hints ([`SettingsSection::key_hint`]), so the outcome has the line to
    /// itself; blank without one. Any mode: an editor's outcome is drawn here too.
    fn notice_line(&self, theme: &Theme) -> Line<'static> {
        let Some(notice) = &self.notice else {
            return Line::default();
        };
        let tone = if notice.is_error() {
            Tone::Error
        } else {
            Tone::Info
        };
        self::notice(notice.text(), tone, theme)
    }

    /// The stack of the current mode (MOD-67 D3, D4): the one place a mode maps to its keys, read
    /// by `key_stack`, the key handler and the hint.
    fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::PROMPT_BROWSE,
            Mode::Editing(_) => views::CAPTURE,
        }
    }

    /// The mode's keys for the shell's hint row, through the mode's stack (MOD-67 D9), plus what a
    /// write in flight adds to them.
    fn hint_text(&self, bound: &Keys) -> String {
        let spec = match self.mode {
            Mode::Editing(_) => HINT_EDITING,
            Mode::Browse => {
                if self.unavailable.is_some() || self.snapshot.is_none() {
                    HINT_NO_SNAPSHOT
                } else {
                    HINT_BROWSE
                }
            }
        };
        let keys = bound.hint(self.stack(), spec);
        match self.busy {
            // Only in Browse: an editor's own hint says what `Enter` is for, and a write in flight
            // is why `Enter` is not answering.
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {}\u{2026}", doing(busy))
            }
            _ => keys,
        }
    }

    /// Reports an outcome, classified by the rule the sections share (D14).
    fn say(&mut self, text: &str) {
        self.notice = Some(if super::is_error(text) {
            Notice::Error(text.to_owned())
        } else {
            Notice::Info(text.to_owned())
        });
    }

    /// Reports a refusal: the seam's own sentence, or one of this section's.
    fn refuse(&mut self, text: String) {
        self.notice = Some(Notice::Error(text));
    }

    /// Fresh settings: the tree is replaced whole and the cursor put back inside it (D3).
    ///
    /// Only a reply to a **write** closes an open editor (H-3): `r` sets no `busy`, so a reload
    /// that lands while something is being typed leaves the typing alone.
    ///
    /// `busy` is the whole of the attribution, and a `PromptSettings` carries nothing that says
    /// which request it answers — so a read that lands between a write and its reply is taken for
    /// that reply and closes the editor early. The kinds section argues the same trade in the same
    /// place: `r` is a letter while an editor is open, but a scope change and a tab re-activation
    /// both issue a read on the Browse side. `r` stays allowed anyway, because re-reading is how a
    /// section that lost a reply recovers; the loss is bounded — the write itself has already been
    /// sent — and [`CHANGED_ELSEWHERE_CLOSED`] is what the miss says when it comes back with
    /// nothing open to retry from.
    fn on_settings(&mut self, snapshot: &SettingsSnapshot) {
        let write = self.busy.take();
        // A read that answered is the end of an outage: leaving `unavailable` set would say the
        // settings are unavailable over a store that just spoke (H-14).
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        if write.is_some() {
            self.notice = None;
            self.mode = Mode::Browse;
        }
        self.follow_editor();
    }
}

impl SettingsSection for PromptSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Prompt"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::PromptSettings(scope.clone())]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // The tree and the cursor belong to the workspace that was left. The notice survives,
        // because the scope change is often the *consequence* of what it is reporting.
        self.snapshot = None;
        self.mode = Mode::Browse;
        self.busy = None;
        self.cursor = 0;
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// MOD-22 review M-1: a bracketed paste into the open editor's field.
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
        // Browse (MOD-67 D6, skeleton (c)): the first candidate this state accepts. Section
        // cycling never reaches here (the tab takes `settings.*` first), and a global act or a
        // declined one is the shell's.
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::PROMPT_BROWSE, chord) {
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
                // Allowed whatever else is going on: re-reading is how a section that lost a reply
                // recovers, and a read cannot lose a write's reply — the staleness index is keyed
                // by request kind. What it *can* do is be mistaken for one; that trade is argued
                // where the mistake is made, in `on_settings`.
                Act::Reload => ctx.request(StoreRequest::PromptSettings(ctx.scope.clone())),
                // Only when there is something to clear: a section that swallowed every `Esc`
                // would take the one the shell uses to close an overlay over it (H-13).
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
            StoreReply::PromptSettings(snapshot) => self.on_settings(snapshot),
            StoreReply::PromptSettingsStale(snapshot) => self.on_stale(snapshot, ctx.keys()),
            // The read itself was refused: saying so beats an empty tree that reads as "nothing
            // here yet" (the agent section's rule, one section across).
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            // Every other refusal of this section's own: the shell has already put
            // `{request}: {message}` on the status line, so all this owes is the sentence and a
            // state the next key can start from. The editor stays open over its text — a refused
            // write is retried by fixing what was refused and pressing `Enter` again (B-9), which
            // is also the recovery for H-1's dead token, by way of `Esc`, `r`, `e`.
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
        let [rows, hint] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        self.scroll.set(None);
        let reload = ctx.keys().label(views::PROMPT_BROWSE, Act::Reload);
        match (&self.unavailable, &self.snapshot) {
            // The refusal wins the pane even with a tree behind it: what is on screen would
            // otherwise be settings nothing has confirmed since the outage started.
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
            // There is no empty case below this one: the `App` group is always the registry's ten.
            (None, Some(snapshot)) => {
                let width = rows.width;
                let (entries, focus) = self.entries(snapshot, width);
                let editing = matches!(self.mode, Mode::Editing(_));
                // One layout per frame (RV2 L3), scrolled by the form's own measure of the focused
                // row and what it put under it (RV2 M1): the row itself first when both do not
                // fit, because a cursor nobody can see acts blind.
                let laid = form_of(entries, focus, editing).layout(width, ctx.accent(), theme);
                let offset = laid.offset(rows.height);
                laid.render(frame, rows, offset);
                // The pane's bar is measured against the rows the tree is drawn into and sits
                // beside them (RV1 M2), so the rows under the tree need no padding.
                self.scroll
                    .set(Some(ScrollSpan::new(laid.lines.len(), offset).rows(rows)));
            }
        }
        frame.render_widget(Paragraph::new(self.notice_line(theme)), hint);
    }
}

/// D5's presence rule for every key but `token_budget`: the project rung when it holds a value,
/// else the `app_setting` row when there is one, else the compiled table.
///
/// "Usable" and "present" differ only for a row the store never validated — hand-written SQL — and
/// the `stored | effective` pair is what makes that visible (H-11). The in-module test pins this
/// against [`resolve_budget`]'s own `source` for all four combinations (B-3).
fn present_source(project_value: Option<&Value>, app_holds: bool) -> BudgetSource {
    match (project_value, app_holds) {
        (Some(_), _) => BudgetSource::Project,
        (None, true) => BudgetSource::AppSetting,
        (None, false) => BudgetSource::AppSettingDefault,
    }
}

/// A [`SettingKind::Boolean`] row's effective column, from its resolver's answer `on` and the row's
/// own stored value.
///
/// Project rung only, so there is no `app_setting` rung to name when it is unset (D276); `number`
/// is `None`, the fraction key's precedent, so D13's clamp line never reads a switch. The source is
/// `project` only when the resolver would read the stored value: a non-boolean falls through to
/// the default, so labelling it `project` would name a rung that answered nothing (MOD-33 review L5).
fn switch(on: bool, project_value: Option<&Value>) -> (String, Option<i64>, &'static str) {
    let source = if project_value.and_then(Value::as_bool).is_some() {
        BudgetSource::Project
    } else {
        BudgetSource::AppSettingDefault
    };
    let source = label::setting_source(source);
    let text = if on { SWITCH_ON } else { SWITCH_OFF };
    (text.to_owned(), None, source)
}

/// What the reader would use for `key`, over the snapshot's own `app_map` and — on a project row —
/// that project's blob (D6).
///
/// **Exhaustive on purpose** (blueprint flag G, O-4): the resolvers return different shapes, so a
/// new key cannot be resolved generically. Without a wildcard the compiler names the missing
/// arm; with one it would render a blank the user could not tell from a resolver that answered
/// nothing.
///
/// `project` is the whole blob on a project row and `None` on an `App` row; `project_value` is that
/// row's own stored value, which is what the presence rule reads.
fn effective(
    key: SettingKey,
    project: Option<&Value>,
    project_value: Option<&Value>,
    app: &BTreeMap<String, Value>,
) -> Effective {
    let present = label::setting_source(present_source(project_value, app.contains_key(key.key())));
    let mut notes = Vec::new();
    let (text, number, source) = match key {
        // The two project-only switches (MOD-89 D4, MOD-33 D267): one row shape, each its own
        // resolver.
        SettingKey::AllowEnvFiles => switch(resolve_allow_env_files(project), project_value),
        SettingKey::BoxHostname => switch(resolve_box_hostname(project), project_value),
        // The one key that records its own provenance: the reader's answer is the label (D5).
        SettingKey::TokenBudget => {
            let budget = resolve_budget(None, project, app);
            (
                budget.tokens.to_string(),
                Some(budget.tokens),
                label::setting_source(budget.source),
            )
        }
        SettingKey::UpstreamHops => {
            let hops = resolve_hops(project, app, &mut notes);
            (hops.to_string(), Some(i64::from(hops)), present)
        }
        SettingKey::MaxSkillTokens => {
            let tokens = resolve_max_skill_tokens(app);
            (tokens.to_string(), Some(tokens), present)
        }
        // `resolve_reserve_bp` is private, so the reserve reaches the screen through the one
        // public door that carries it (blueprint flag A) — still the reader's own arithmetic.
        SettingKey::PromptReserveFraction => (
            fraction_text(&resolve_budget(None, None, app)),
            None,
            present,
        ),
        SettingKey::ExcerptFileLineCap => {
            let lines = i64::from(resolve_excerpt_caps(app).0.file_line_cap);
            (lines.to_string(), Some(lines), present)
        }
        // Already `.min(file_line_cap)` inside the resolver, which is what D13's line reports.
        SettingKey::ExcerptHeadLines => {
            let lines = i64::from(resolve_excerpt_caps(app).0.head_lines);
            (lines.to_string(), Some(lines), present)
        }
        SettingKey::ExcerptMaxFileBytes => {
            let bytes =
                i64::try_from(resolve_excerpt_caps(app).0.max_file_bytes).unwrap_or(i64::MAX);
            (bytes.to_string(), Some(bytes), present)
        }
        SettingKey::ExcerptMaxFiles => {
            let files = i64::from(resolve_excerpt_caps(app).0.max_files);
            (files.to_string(), Some(files), present)
        }
        SettingKey::ExcerptMaxScanFiles => {
            let files = i64::from(resolve_excerpt_caps(app).1);
            (files.to_string(), Some(files), present)
        }
        SettingKey::ExcerptProviderDeadlineMs => {
            let ms = i64::try_from(resolve_excerpt_caps(app).2.as_millis()).unwrap_or(i64::MAX);
            (ms.to_string(), Some(ms), present)
        }
    };
    Effective {
        text,
        number,
        source,
        notes,
    }
}

/// The widest key of the registry, so the rows line up whatever the registry holds.
///
/// Computed rather than a constant: a longer key added tomorrow widens the column by itself.
/// Computed **once**: [`SettingKey::ALL`] is compiled in and cannot change between frames, where
/// calling this per row walked every key once per row, every frame.
static KEY_WIDTH: LazyLock<usize> = LazyLock::new(|| {
    SettingKey::ALL
        .iter()
        .map(|key| cell_width(key.key()))
        .max()
        .unwrap_or(0)
});

/// One value row's value column: what the rung stores, what the reader would use, which rung
/// answered in words, and the unit (D9). The key is the row's label. One space before the unit:
/// the form wraps the value under its column, and wrapping collapses runs of blanks.
fn value_text(key: SettingKey, stored: Option<&Value>, effective: &Effective) -> String {
    let held = stored.map_or_else(|| UNSET.to_owned(), Value::to_string);
    format!(
        "{held} | {} ({}) {}",
        effective.text,
        effective.source,
        unit(key)
    )
}

/// The unit column: the registry's own, except a fraction, which is typed as one (MOD-83).
fn unit(key: SettingKey) -> &'static str {
    let spec = key.spec();
    match spec.kind {
        SettingKind::Fraction => FRACTION_UNIT,
        SettingKind::Integer | SettingKind::Boolean => spec.unit,
    }
}

/// The registry's range and rungs for `key` (B-6): two words for a switch, the range in its unit
/// for a number, and for a fraction the range in percent with the basis points the store's
/// refusals are phrased in (D12, maintainer answer 2026-10-21 item 2).
fn range_text(key: SettingKey) -> String {
    let spec = key.spec();
    match spec.kind {
        // `min` 0 and `max` 1 are nominal for a switch; the range it takes is two words.
        SettingKind::Boolean => format!("{SWITCH_RANGE} \u{b7} rungs {}", spec.rungs),
        SettingKind::Integer => format!(
            "range {}..={} {} \u{b7} rungs {}",
            spec.min, spec.max, spec.unit, spec.rungs
        ),
        SettingKind::Fraction => format!(
            "range {}\u{2013}{}% ({}..={} {}) \u{b7} typed as a fraction \u{b7} rungs {}",
            percent(spec.min),
            percent(spec.max),
            spec.min,
            spec.max,
            spec.unit,
            spec.rungs
        ),
    }
}

/// The focused row's help: the key whole when its label is cut (`named`), what the row holds now
/// while its editor is open, then the registry's doc line and [`range_text`].
fn help_text(key: SettingKey, now: Option<&str>, named: bool) -> String {
    let doc = key.spec().doc.trim_end_matches('.');
    named
        .then(|| key.key().to_owned())
        .into_iter()
        .chain(now.map(|now| format!("now {now}")))
        .chain((!doc.is_empty()).then(|| doc.to_owned()))
        .chain([range_text(key)])
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// What the reader does to the stored number that the row alone cannot show (D13 and blueprint
/// flag B): the `not_above` clamp and the resolver's own notes, `None` when there are none.
///
/// D13: the `not_above` rule is one-directional by decision, so a peer lowered under this row's
/// stored value leaves the row describing a number the reader will not use. Derived from the spec
/// and the resolver's own answer, so no key is named here.
fn row_error(key: SettingKey, stored: Option<&Value>, effective: &Effective) -> Option<String> {
    let clamp = key.spec().not_above.and_then(|peer| {
        let held = stored.and_then(Value::as_i64)?;
        let used = effective.number?;
        (used < held).then(|| clamp_line(peer, used))
    });
    let lines: Vec<String> = clamp
        .into_iter()
        .chain(effective.notes.iter().cloned())
        .collect();
    (!lines.is_empty()).then(|| lines.join("; "))
}

/// Basis points as a percentage, `1000` → `10`, `1250` → `12.5`, `5` → `0.05`: exact, so the
/// help never rounds the bound the store will hold the value to.
fn percent(bp: i64) -> String {
    let sign = if bp < 0 { "-" } else { "" };
    let bp = bp.unsigned_abs();
    let (whole, cents) = (bp / 100, bp % 100);
    match cents {
        0 => format!("{sign}{whole}"),
        _ if cents % 10 == 0 => format!("{sign}{whole}.{}", cents / 10),
        _ => format!("{sign}{whole}.{cents:02}"),
    }
}

/// The reserve as what it means and what it is typed as (D12, MOD-83): the percentage of the
/// budget held back, then the fraction the reader resolved, `10% (0.1)`.
///
/// Showing only the float hides what it means; showing only the basis points would invite typing
/// `1000`, which rounds to 10 000 000 bp and is refused. The basis points the refusal is written
/// in are on the help line ([`range_text`]).
fn fraction_text(budget: &Budget) -> String {
    format!(
        "{}% ({})",
        percent(i64::from(budget.reserve_bp)),
        budget.reserve()
    )
}

/// D11's sentence for an integer key: shape, and never a bound.
fn integer_sentence(key: SettingKey) -> String {
    format!("`{key}` is a whole number, or empty to clear")
}

/// D11's sentence for the one fraction key.
fn fraction_sentence(key: SettingKey) -> String {
    format!("`{key}` is a decimal fraction, or empty to clear")
}

/// D276: `on`, `off`, `true`, `false`, ASCII case-insensitive; anything else is not a switch.
fn parse_switch(text: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case(SWITCH_ON) || text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case(SWITCH_OFF) || text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

/// D11's sentence for a switch: shape, never a bound.
fn switch_sentence(key: SettingKey) -> String {
    format!("`{key}` is on or off, or empty to clear")
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

/// What a second write is told while the first is still out.
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// The token an open editor retries against after a reload (D14, B-5).
fn reload(snapshot: &SettingsSnapshot, target: Target) -> Reload {
    match target.rung {
        SettingRung::App => snapshot
            .app_entry(target.key)
            .map_or(Reload::Gone, |entry| {
                entry.updated_at.map_or(Reload::NoRow, Reload::Token)
            }),
        SettingRung::Project(id) => snapshot
            .projects
            .iter()
            .find(|entry| entry.project.id == id)
            .map_or(Reload::Gone, |entry| {
                Reload::Token(entry.project.updated_at)
            }),
        // Never opened here (D8).
        SettingRung::Phase(_) => Reload::Gone,
    }
}

/// D13's line: what the reader will use instead of what this row holds, and which peer decided it.
fn clamp_line(peer: SettingKey, used: i64) -> String {
    format!("clamped to {peer} = {used}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The source column's words for the two rungs a switch can name (MOD-83).
    const PROJECT: &str = label::setting_source(BudgetSource::Project);
    const DEFAULT: &str = label::setting_source(BudgetSource::AppSettingDefault);

    /// MOD-83: no source the column prints is an enum value.
    #[test]
    fn the_source_column_reads_in_words() {
        assert_eq!(PROJECT, "project");
        assert_eq!(DEFAULT, "default");
        assert_eq!(
            label::setting_source(BudgetSource::AppSetting),
            "app setting"
        );
    }

    /// Basis points as an exact percentage, never rounded.
    #[test]
    fn basis_points_read_as_an_exact_percentage() {
        for (bp, shown) in [
            (0, "0"),
            (5, "0.05"),
            (1_000, "10"),
            (1_250, "12.5"),
            (5_000, "50"),
            (-30, "-0.3"),
        ] {
            assert_eq!(percent(bp), shown, "{bp} bp");
        }
    }

    /// Maintainer answer 2026-10-21, item 2: `0.1 (1000 bp)` reads `10% (0.1)`, and the help line
    /// carries the range in percent with the basis points the store's refusals are phrased in.
    #[test]
    fn the_reserve_reads_in_percent_with_the_bp_range_on_the_help_line() {
        let budget = Budget {
            reserve_bp: 1_000,
            ..resolve_budget(None, None, &BTreeMap::new())
        };
        assert_eq!(fraction_text(&budget), "10% (0.1)");
        let key = SettingKey::PromptReserveFraction;
        assert_eq!(unit(key), "fraction");
        let spec = key.spec();
        assert_eq!(
            range_text(key),
            format!(
                "range 0\u{2013}50% ({}..={} bp) \u{b7} typed as a fraction \u{b7} rungs {}",
                spec.min, spec.max, spec.rungs
            )
        );
        assert!(help_text(key, None, false).ends_with(&range_text(key)));
        assert!(
            help_text(key, None, true).starts_with("prompt_reserve_fraction \u{b7} Share"),
            "a cut label is named whole"
        );
        assert!(
            help_text(key, Some("0.1 | 10% (0.1) (app setting) fraction"), false)
                .starts_with("now 0.1 | 10% (0.1) (app setting) fraction \u{b7} ")
        );
    }

    /// MOD-67 M6 D4: the header refusal names the edit key, and none unbound.
    #[test]
    fn not_a_value_row_names_the_edit_key_or_none() {
        assert_eq!(not_a_value_row(Some("e")), "`e` edits a value row");
        assert_eq!(not_a_value_row(None), "only a value row is edited");
    }

    /// The blob a project holds when it stores `key` — built from the registry, because no key is
    /// spelled in this file (acceptance line 4).
    fn blob(key: SettingKey, value: Value) -> Value {
        let mut map = serde_json::Map::new();
        map.insert(key.key().to_owned(), value);
        Value::Object(map)
    }

    /// H-5 reaches the *mode* as well as the editor: [`PromptSection`] derives `Debug` through
    /// [`Mode`], so a variant that printed its own field's text would put it in a log the moment
    /// one `tracing::debug!` names the section. Every value typed here is a number somebody chose
    /// for their own prompt, and milestone 6's masked column is what makes the rule load-bearing.
    #[test]
    fn an_editor_never_prints_its_buffer() {
        let section = PromptSection {
            mode: Mode::Editing(Editor {
                target: Target {
                    rung: SettingRung::App,
                    key: SettingKey::TokenBudget,
                },
                input: TextField::with_text("424242"),
                expected: None,
            }),
            ..PromptSection::new()
        };

        let printed = format!("{section:?}");

        assert!(
            !printed.contains("424242"),
            "what was typed stays out of the line: {printed}"
        );
        assert!(
            printed.contains("Editing") && printed.contains("TokenBudget"),
            "the mode and the target are still legible: {printed}"
        );
    }

    /// B-3: the presence rule and the reader's own `source` are the same fact for the four
    /// combinations of project-holds × app-holds, so the label a row carries cannot drift from the
    /// rung the assembler actually used.
    ///
    /// Asserted through `token_budget`, the one key that both rules describe: `resolve_budget`
    /// records its provenance and the presence rule derives it. The other nine have no resolver
    /// that records one, which is why the rule exists at all.
    #[test]
    fn the_source_label_matches_resolve_budget_for_all_four_combinations() {
        let key = SettingKey::TokenBudget;
        let held = blob(key, json!(4_000));
        let empty = Value::Object(serde_json::Map::new());

        for project in [None, Some(&held), Some(&empty)] {
            for app_holds in [false, true] {
                let mut app = BTreeMap::new();
                if app_holds {
                    app.insert(key.key().to_owned(), json!(7_000));
                }
                let stored = project.and_then(|blob| blob.get(key.key()));

                assert_eq!(
                    present_source(stored, app.contains_key(key.key())),
                    resolve_budget(None, project, &app).source,
                    "project {project:?}, app holds {app_holds}"
                );
            }
        }
    }

    /// MOD-33 D276: the four spellings of a switch, in any ASCII case, and nothing else.
    #[test]
    fn a_switch_parses_on_off_true_false_in_any_case() {
        for (text, on) in [
            ("ON", true),
            ("Off", false),
            ("TRUE", true),
            ("false", false),
        ] {
            assert_eq!(parse_switch(text), Some(on), "`{text}` is a switch");
        }
        for text in ["maybe", "1", "", "yes"] {
            assert_eq!(parse_switch(text), None, "`{text}` is not a switch");
        }
    }

    /// MOD-33 review L5: the switch's row says `project` only when the project stores a JSON
    /// boolean — the one value `resolve_box_hostname` reads — and `default` for anything else,
    /// which the resolver falls through.
    #[test]
    fn the_switch_source_is_project_only_for_a_stored_boolean() {
        let key = SettingKey::BoxHostname;
        let app = BTreeMap::new();
        for (stored, text, source) in [
            (Some(json!(true)), SWITCH_ON, PROJECT),
            (Some(json!(false)), SWITCH_OFF, PROJECT),
            (Some(json!("off")), SWITCH_ON, DEFAULT),
            (Some(json!(1)), SWITCH_ON, DEFAULT),
            (Some(Value::Null), SWITCH_ON, DEFAULT),
            (None, SWITCH_ON, DEFAULT),
        ] {
            let project = stored.as_ref().map_or_else(
                || Value::Object(serde_json::Map::new()),
                |value| blob(key, value.clone()),
            );
            let row = effective(key, Some(&project), project.get(key.key()), &app);
            assert_eq!(row.text, text, "{stored:?}");
            assert_eq!(row.source, source, "{stored:?}");
        }
    }

    /// MOD-89 D4: the env-files switch reads `on` only for a stored JSON `true`, and — the same
    /// L5 rule as `box_hostname` — says `project` only when the project stores a JSON boolean.
    /// Anything else falls through to the compiled default, which is off.
    #[test]
    fn the_env_files_switch_is_off_unless_a_project_stores_true() {
        let key = SettingKey::AllowEnvFiles;
        let app = BTreeMap::new();
        for (stored, text, source) in [
            (Some(json!(true)), SWITCH_ON, PROJECT),
            (Some(json!(false)), SWITCH_OFF, PROJECT),
            (Some(json!("on")), SWITCH_OFF, DEFAULT),
            (Some(json!(1)), SWITCH_OFF, DEFAULT),
            (Some(Value::Null), SWITCH_OFF, DEFAULT),
            (None, SWITCH_OFF, DEFAULT),
        ] {
            let project = stored.as_ref().map_or_else(
                || Value::Object(serde_json::Map::new()),
                |value| blob(key, value.clone()),
            );
            let row = effective(key, Some(&project), project.get(key.key()), &app);
            assert_eq!(row.text, text, "{stored:?}");
            assert_eq!(row.source, source, "{stored:?}");
            assert_eq!(
                row.number, None,
                "D13's clamp arithmetic never reads a switch"
            );
        }
    }

    /// MOD-67 M6 T2: the row under the pane carries the notice alone, and the keys are the
    /// section's [`SettingsSection::key_hint`], the shell's row.
    #[test]
    fn the_notice_row_carries_the_notice_alone() {
        let notice = "\u{6f22}".repeat(10);
        let section = PromptSection {
            notice: Some(Notice::Error(notice.clone())),
            ..PromptSection::new()
        };
        let theme = Theme::default();
        let line = section.notice_line(&theme);
        assert_eq!(line.spans.len(), 1, "{line:?}");
        assert_eq!(line.spans[0].content, notice);
        assert_eq!(line.style, theme.error);
        let keys = section
            .key_hint(Keys::compiled(), &theme)
            .expect("the defaults bind the keys");
        assert_eq!(keys.to_string(), section.hint_text(Keys::compiled()));
    }
}
