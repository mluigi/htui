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
//! [`SettingKey::spec`](htui_core::prompt::SettingKey::spec), so an eleventh key added by MOD-4 or
//! MOD-12 appears here without this file being touched. The one exception is the **effective**
//! column, which needs the reader's own per-key resolver and is therefore an exhaustive
//! `match key` with no wildcard: an eleventh key is a compile error naming the missing arm rather
//! than a silent blank (blueprint flag G, O-4).
//!
//! The section parses **shape** and nothing else (D11). Every bound — `min`, `max`, `not_above`,
//! the phase narrowing — belongs to
//! [`validate`](htui_core::prompt::validate), and its sentence is shown verbatim. A range checked
//! twice is a range that can disagree with itself.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use htui_core::model::Scope;
use htui_core::prompt::settings::{
    resolve_budget, resolve_excerpt_caps, resolve_hops, resolve_max_skill_tokens,
};
use htui_core::prompt::{Budget, BudgetSource, SettingKey, SettingKind};
use htui_core::store::SettingRung;
use serde_json::Value;

use crate::app::{Ctx, Handled};
use crate::prompt_settings::{AppEntry, ProjectEntry, READ_NAME, REQUEST_NAMES, SettingsSnapshot};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    message, wrapped,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the rows pane says before any settings have arrived.
const NOT_READ: &str = "settings not read yet";

/// What the rows pane says when the read itself was refused.
const UNAVAILABLE: &str = "settings unavailable";

/// What a rung that holds no value of its own shows.
///
/// One word for both rungs: the kinds section's `inherit` would be wrong on `App`, which has no
/// rung above it — only the compiled table below (B-7).
const UNSET: &str = "unset";

/// Browse's keys, with something to browse.
const HINT_BROWSE: &str = "j/k \u{b7} e edit \u{b7} r reload";

/// Browse's keys with nothing read, or the read refused: the only offer is to ask again.
const HINT_NO_SNAPSHOT: &str = "r reload";

/// An open editor's keys. One field, so there is no `Tab`.
const HINT_EDITING: &str = "Enter save \u{b7} Esc cancel \u{b7} empty clears";

/// What `e` says on a group header (B-1).
const NOT_A_VALUE_ROW: &str = "`e` edits a value row";

/// What an empty field over a rung that holds nothing says: there is nothing to clear, so there is
/// no request to send (D10).
const NOTHING_SET: &str = "nothing is set on this rung";

/// The `App` group's line.
const APP_HEADER: &str = "app";

/// The prefix of a project group's line (B-1).
const PROJECT_HEADER: &str = "project";

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
    /// As the row prints it (`5000`, `0.1 (1000 bp)`).
    text: String,
    /// The same as a number, for D13's comparison; `None` for the one fraction key, whose stored
    /// unit and resolved unit are not the same thing.
    number: Option<i64>,
    /// Which rung the reader took it from.
    source: BudgetSource,
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
    /// The last outcome, one line on the hint row.
    notice: Option<Notice>,
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
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return true;
        }
        self.snapshot.is_none()
    }

    /// `e`: the row under the cursor, prefilled with what the rung stores.
    ///
    /// Empty when the rung holds nothing, which is the same field an `Enter` reads as "clear"
    /// — and on an unset rung that is not a write at all (D10).
    fn open_edit(&mut self, row: Row) {
        let Some(opening) = self.target_of(row) else {
            self.say(NOT_A_VALUE_ROW);
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
    /// on is swallowed rather than offered to the shell — with `CONTROL` chords excepted, so
    /// `ctrl-c` still quits.
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
            FieldOutcome::Pass if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
            FieldOutcome::Pass => Handled::Consumed,
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
    /// write.
    fn on_stale(&mut self, snapshot: &SettingsSnapshot) {
        self.busy = None;
        let reloaded = match &self.mode {
            Mode::Editing(editor) => Some(reload(snapshot, editor.target)),
            Mode::Browse => None,
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
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
                self.say(CHANGED_ELSEWHERE);
            }
        }
    }

    /// One line per row, in [`rows`](PromptSection::rows) order (D9).
    fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        // Ten entries, rebuilt per frame rather than cached: the snapshot is replaced whole on
        // every reply, and a cache beside it would be the second source of truth D3 refuses.
        let app = snapshot.app_map();
        let mut lines = Vec::new();
        for (index, row) in self.rows().into_iter().enumerate() {
            let style = if index == self.cursor {
                theme.selected
            } else {
                theme.base
            };
            match row {
                // `&'static str` rather than an owned copy: `Line<'static>` borrows a constant
                // happily, and the group header is drawn on every frame.
                Row::AppHeader => lines.push(Line::styled(APP_HEADER, style)),
                Row::ProjectHeader { p } => {
                    if let Some(entry) = self.project(p) {
                        lines.push(Line::styled(
                            format!("{PROJECT_HEADER} {}", entry.project.name),
                            style,
                        ));
                    }
                }
                Row::App { .. } | Row::Project { .. } => {
                    if let Some((key, stored, effective)) = self.value_at(row, &app) {
                        lines.push(Line::styled(value_line(key, stored, &effective), style));
                    }
                }
            }
        }
        lines
    }

    /// The pane under the rows: the registry's own row for the selected key (B-6).
    ///
    /// The kinds section's Browse pane is empty because it has nothing row-specific to say; this
    /// one does — the doc line, the range in its unit, the rungs the key accepts, and the two
    /// things the reader would do to the stored number that the row alone cannot show.
    fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        // A header selected has nothing row-specific to say, so the pane is the field alone — and
        // in Browse, nothing at all.
        if let Some(snapshot) = &self.snapshot
            && let Some(row) = self.selected()
            && let Some((key, stored, effective)) = self.value_at(row, &snapshot.app_map())
        {
            let spec = key.spec();
            let room = usize::from(width).max(1);
            lines.extend(
                wrapped(spec.doc, room)
                    .into_iter()
                    .map(|line| Line::styled(line, theme.dim)),
            );
            lines.push(Line::styled(
                format!(
                    "range {}..={} {} \u{b7} rungs {}",
                    spec.min, spec.max, spec.unit, spec.rungs
                ),
                theme.dim,
            ));
            // D13: the `not_above` rule is one-directional by decision, so a peer lowered under
            // this row's stored value leaves the row describing a number the reader will not use.
            // Derived from the spec and the resolver's own answer, so no key is named here.
            if let Some(peer) = spec.not_above
                && let Some(held) = stored.and_then(Value::as_i64)
                && let Some(used) = effective.number
                && used < held
            {
                lines.push(Line::styled(clamp_line(peer, used), theme.error));
            }
            for note in &effective.notes {
                lines.push(Line::styled(note.clone(), theme.error));
            }
        }
        // The field goes under the registry's own lines, not over them: what the range and the
        // rungs say is what an `Enter` will be judged against. Drawn whatever the cursor is on, so
        // a reload that moved it can never leave someone typing into a field they cannot see.
        if let Mode::Editing(editor) = &self.mode {
            let label = format!("{}: ", editor.target.key);
            let room = usize::from(width).saturating_sub(label.chars().count());
            let mut spans = vec![Span::styled(label, theme.accent)];
            spans.extend(
                editor
                    .input
                    .line(u16::try_from(room).unwrap_or(u16::MAX), true, theme)
                    .spans,
            );
            lines.push(Line::from(spans));
        }
        lines
    }

    /// The one line under the pane: the keys this mode binds, then the last outcome.
    ///
    /// Two spans rather than one string: a compare-and-set miss is reported here and D14 asks for
    /// it in `theme.error`, because "someone else wrote to this row" is the one notice a user has
    /// to act on rather than read.
    fn hint(&self, width: u16, theme: &Theme) -> Line<'static> {
        let keys = self.hint_text();
        let Some(notice) = &self.notice else {
            return Line::styled(keys, theme.dim);
        };
        let style = if notice.is_error() {
            theme.error
        } else {
            theme.dim
        };
        let text = notice.text().to_owned();
        // The outcome wins the line when both do not fit: the keys are on screen every other
        // frame, and this is the only place the outcome appears.
        let room = usize::from(width);
        if keys.chars().count() + text.chars().count() + 3 > room {
            return Line::styled(text, style);
        }
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(text, style),
        ])
    }

    /// The keys half of the hint line, plus what a write in flight adds to it.
    fn hint_text(&self) -> String {
        let keys = match self.mode {
            Mode::Editing(_) => HINT_EDITING,
            Mode::Browse => {
                if self.unavailable.is_some() || self.snapshot.is_none() {
                    HINT_NO_SNAPSHOT
                } else {
                    HINT_BROWSE
                }
            }
        };
        match self.busy {
            // Only in Browse: an editor's own hint says what `Enter` is for, and a write in flight
            // is why `Enter` is not answering.
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {busy} in flight")
            }
            _ => keys.to_owned(),
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

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
        // Browse. `j`, `k`, `e`, `r` are free: the global table binds `q`, `?`, the digits,
        // `ctrl-c` and `-`, and the tab consumes `h`/`l`/`[`/`]`/arrows before a section is
        // offered the key.
        match key.code {
            KeyCode::Char('e') => {
                if !self.blocked()
                    && let Some(row) = self.selected()
                {
                    self.open_edit(row);
                }
                Handled::Consumed
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_cursor(true);
                Handled::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_cursor(false);
                Handled::Consumed
            }
            // Allowed whatever else is going on: re-reading is how a section that lost a reply
            // recovers, and a read cannot lose a write's reply — the staleness index is keyed by
            // request kind. What it *can* do is be mistaken for one; that trade is argued where
            // the mistake is made, in `on_settings`.
            KeyCode::Char('r') => {
                ctx.request(StoreRequest::PromptSettings(ctx.scope.clone()));
                Handled::Consumed
            }
            // Only when there is something to clear: a section that swallowed every `Esc` would
            // take the one the shell uses to close an overlay over it (H-13).
            KeyCode::Esc if self.notice.is_some() => {
                self.notice = None;
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::PromptSettings(snapshot) => self.on_settings(snapshot),
            StoreReply::PromptSettingsStale(snapshot) => self.on_stale(snapshot),
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

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let pane = self.pane(area.width, ctx.theme);
        let [rows, pane_area, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        match (&self.unavailable, &self.snapshot) {
            // The refusal wins the pane even with a tree behind it: what is on screen would
            // otherwise be settings nothing has confirmed since the outage started.
            (Some(why), _) => frame.render_widget(
                Paragraph::new(Line::styled(
                    format!("{UNAVAILABLE}: {why}"),
                    ctx.theme.error,
                ))
                .wrap(Wrap { trim: true }),
                rows,
            ),
            (None, None) => message(frame, rows, NOT_READ, ctx.theme),
            // There is no empty case below this one: the `App` group is always the registry's ten.
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
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), pane_area);
        }
        frame.render_widget(Paragraph::new(self.hint(area.width, ctx.theme)), hint);
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

/// What the reader would use for `key`, over the snapshot's own `app_map` and — on a project row —
/// that project's blob (D6).
///
/// **Exhaustive on purpose** (blueprint flag G, O-4): the four resolvers return four shapes, so an
/// eleventh key cannot be resolved generically. Without a wildcard the compiler names the missing
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
    let present = present_source(project_value, app.contains_key(key.key()));
    let mut notes = Vec::new();
    let (text, number, source) = match key {
        // The one key that records its own provenance: the reader's answer is the label (D5).
        SettingKey::TokenBudget => {
            let budget = resolve_budget(None, project, app);
            (
                budget.tokens.to_string(),
                Some(budget.tokens),
                budget.source,
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
/// calling this per row walked the ten keys thirteen times a frame.
static KEY_WIDTH: LazyLock<usize> = LazyLock::new(|| {
    SettingKey::ALL
        .iter()
        .map(|key| key.key().chars().count())
        .max()
        .unwrap_or(0)
});

/// One value row: the key, what the rung stores, what the reader would use, which rung answered,
/// and the unit the registry counts in (D9).
fn value_line(key: SettingKey, stored: Option<&Value>, effective: &Effective) -> String {
    let held = stored.map_or_else(|| UNSET.to_owned(), Value::to_string);
    // `key.key()` rather than `key`: [`SettingKey`]'s own `Display` writes the string straight out
    // and so ignores the formatter's width, where a `&str` honours it.
    format!(
        "  {:<width$}  {held} | {} ({})   {}",
        key.key(),
        effective.text,
        effective.source.as_str(),
        key.spec().unit,
        width = *KEY_WIDTH,
    )
}

/// The reserve as what it is and what it means (D12): the fraction the reader resolved, and the
/// basis points its range is written in.
///
/// Showing only the float hides the unit the refusal is written in; showing only the basis points
/// would invite typing `1000`, which rounds to 10 000 000 bp and is refused.
fn fraction_text(budget: &Budget) -> String {
    format!("{} ({} bp)", budget.reserve(), budget.reserve_bp)
}

/// D11's sentence for an integer key: shape, and never a bound.
fn integer_sentence(key: SettingKey) -> String {
    format!("`{key}` is a whole number, or empty to clear")
}

/// D11's sentence for the one fraction key.
fn fraction_sentence(key: SettingKey) -> String {
    format!("`{key}` is a decimal fraction, or empty to clear")
}

/// What a second write is told while the first is still out.
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
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
}
