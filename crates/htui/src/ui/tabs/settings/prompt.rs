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

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use htui_core::model::Scope;
use htui_core::prompt::settings::{
    resolve_budget, resolve_excerpt_caps, resolve_hops, resolve_max_skill_tokens,
};
use htui_core::prompt::{Budget, BudgetSource, SettingKey};
use serde_json::Value;

use crate::app::{Ctx, Handled};
use crate::prompt_settings::{AppEntry, ProjectEntry, REQUEST_NAMES, SettingsSnapshot};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::settings::{SectionId, SettingsSection, message};
use crossterm::event::{KeyCode, KeyEvent};

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
                Row::AppHeader => lines.push(Line::styled(APP_HEADER.to_owned(), style)),
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
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let app = snapshot.app_map();
        let Some(row) = self.selected() else {
            return Vec::new();
        };
        let Some((key, stored, effective)) = self.value_at(row, &app) else {
            return Vec::new();
        };
        let spec = key.spec();
        let room = usize::from(width).max(1);
        let mut lines: Vec<Line<'static>> = wrapped(spec.doc, room)
            .into_iter()
            .map(|line| Line::styled(line, theme.dim))
            .collect();
        lines.push(Line::styled(
            format!(
                "range {}..={} {} \u{b7} rungs {}",
                spec.min, spec.max, spec.unit, spec.rungs
            ),
            theme.dim,
        ));
        // D13: the `not_above` rule is one-directional by decision, so a peer lowered under this
        // row's stored value leaves the row describing a number the reader will not use. Derived
        // from the spec and the resolver's own answer, so no key is named here.
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
        lines
    }

    /// The one line under the pane: the keys this mode binds.
    fn hint(&self, theme: &Theme) -> Line<'static> {
        let keys = if self.unavailable.is_some() || self.snapshot.is_none() {
            HINT_NO_SNAPSHOT
        } else {
            HINT_BROWSE
        };
        Line::styled(keys.to_owned(), theme.dim)
    }

    /// Fresh settings: the tree is replaced whole and the cursor put back inside it (D3).
    fn on_settings(&mut self, snapshot: &SettingsSnapshot) {
        // A read that answered is the end of an outage: leaving `unavailable` set would say the
        // settings are unavailable over a store that just spoke (H-14).
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
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
        self.cursor = 0;
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
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
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::PromptSettings(snapshot) => self.on_settings(snapshot),
            // The read itself was refused: saying so beats an empty tree that reads as "nothing
            // here yet" (the agent section's rule, one section across).
            StoreReply::Failed { request, message } if *request == REQUEST_NAMES[0] => {
                self.unavailable = Some(message.clone());
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
        frame.render_widget(Paragraph::new(self.hint(ctx.theme)), hint);
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
fn key_width() -> usize {
    SettingKey::ALL
        .iter()
        .map(|key| key.key().chars().count())
        .max()
        .unwrap_or(0)
}

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
        width = key_width(),
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

/// D13's line: what the reader will use instead of what this row holds, and which peer decided it.
fn clamp_line(peer: SettingKey, used: i64) -> String {
    format!("clamped to {peer} = {used}")
}

/// One sentence broken into lines of at most `width` chars, on spaces.
///
/// Copied from the kinds section rather than promoted beside [`is_error`](super::is_error): two
/// copies of three lines are cheaper than a third shared helper, and a third copy is what would
/// justify the promotion (O-5).
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let extra = if line.is_empty() {
            word.chars().count()
        } else {
            word.chars().count() + 1
        };
        if !line.is_empty() && line.chars().count() + extra > width {
            lines.push(core::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// B-3: the presence rule and the reader's own `source` are the same fact for the four
    /// combinations of project-holds × app-holds, so the label a row carries cannot drift from the
    /// rung the assembler actually used.
    ///
    /// Asserted through `token_budget`, the one key that both rules describe: `resolve_budget`
    /// records its provenance and the presence rule derives it. The other nine have no resolver
    /// that records one, which is why the rule exists at all.
    /// The blob a project holds when it stores `key` — built from the registry, because no key is
    /// spelled in this file (acceptance line 4).
    fn blob(key: SettingKey, value: Value) -> Value {
        let mut map = serde_json::Map::new();
        map.insert(key.key().to_owned(), value);
        Value::Object(map)
    }

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
