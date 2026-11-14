//! The shell's activity panel (MOD-122 M3 D8, D9, D10): one per `App`, since events are global,
//! not a tab's. It holds the newest [`CAPACITY`] events by id and the pinned needs-you events the
//! last read returned, filters them in the panel (D5), draws them in the activity pane and turns
//! `activity.open` into an [`ActivityJump`] the shell carries out.
//!
//! Reads overlap: after the first (`after: None`, the newest page) each re-reads from the highest
//! id held minus [`OVERLAP`], so an event whose transaction committed late still lands, and a
//! repeated id is never doubled (the Logs tab's workers' merge, M2 D10). `pinned` is replaced
//! wholesale by every reply. Every key resolves through [`views::ACTIVITY_BROWSE`].

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use htui_core::model::activity::{
    ACTIVITY_PAGE, ActivityClass, ActivityEvent, ActivityKind, ActivityPage,
};
use htui_core::model::{Scope, StepId, WorkspaceSummary};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::app::{Ctx, RevealTarget};
use crate::keys::{Act, Hint, HintSpec, Keys, views};
use crate::ui::Theme;
use crate::ui::kit::chips::key_line;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::pane::{Focus, Pane};
use crate::ui::kit::scroll::window;
use crate::ui::kit::text::STAMP;

/// D8 (blueprint F7): the panel holds at most this many events, one store page.
pub const CAPACITY: usize = ACTIVITY_PAGE as usize;

/// D8: a later read re-reads from the highest id held minus this many (M2 D10's late-commit
/// overlap).
pub const OVERLAP: i64 = 200;

/// D8: which events the panel lists. Session state, not saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivityFilter {
    /// Every event the panel holds.
    #[default]
    All,
    /// The pinned events only: what waits on you now.
    NeedsYou,
    /// `step_failed` and `run_failed`.
    Failures,
    /// D5: the events of the current workspace.
    ThisWorkspace,
}

impl ActivityFilter {
    /// `activity.filter`'s cycle: all workspaces → needs you → failures → this workspace → all.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::NeedsYou,
            Self::NeedsYou => Self::Failures,
            Self::Failures => Self::ThisWorkspace,
            Self::ThisWorkspace => Self::All,
        }
    }

    /// The filter's words in the panel's title.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "all workspaces",
            Self::NeedsYou => "needs you",
            Self::Failures => "failures",
            Self::ThisWorkspace => "this workspace",
        }
    }
}

/// What Enter asks the shell to do (D9).
#[derive(Debug, Clone, PartialEq)]
pub enum ActivityJump {
    /// Open the chat's transcript (`Action::Replay`).
    Replay {
        /// The chat's step.
        step: StepId,
    },
    /// Reveal the item's step, entering `workspace` first when it is `Some`.
    Reveal {
        /// The workspace the item lives in, when it is not the current one.
        workspace: Option<WorkspaceSummary>,
        /// What to reveal.
        target: RevealTarget,
    },
    /// The row's item or run is gone: nothing moves, the status line says so.
    Gone {
        /// The row's key (`MOD-7`, or `chat`).
        key: String,
    },
}

/// The shell's activity panel (D8).
#[derive(Debug, Default)]
pub struct ActivityPanel {
    /// The events held, by id: at most [`CAPACITY`], the oldest dropped first.
    events: BTreeMap<i64, ActivityEvent>,
    /// The last reply's pinned events, newest first.
    pinned: Vec<ActivityEvent>,
    /// The highest id ever merged: the next read's cursor ([`after`](Self::after)).
    max_id: Option<i64>,
    /// The filter: session state, not saved.
    filter: ActivityFilter,
    /// The selected row's index, when [`anchor`](Self::anchor) is not listed.
    cursor: usize,
    /// The selected event's id: the cursor stays on it when rows arrive above.
    anchor: Option<i64>,
    /// Whether a read is out.
    in_flight: bool,
    /// Every workspace, for the `@slug` and the jump's workspace (D5).
    workspaces: Vec<WorkspaceSummary>,
    /// How a row's time is printed.
    zone: Zone,
}

/// How a row's time is printed: in the machine's zone; in UTC under test and in the `testkit`
/// harness (review L5), so no snapshot moves with the machine's.
#[derive(Debug, Clone, Copy, Default)]
enum Zone {
    /// The local zone.
    #[default]
    Local,
    /// UTC, "today" being `today`'s date or, with none, the wall clock's.
    #[cfg(any(test, feature = "testkit"))]
    Utc { today: Option<NaiveDate> },
}

impl ActivityPanel {
    /// An empty panel, before its first read.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Review L5: the panel prints its rows' times in UTC, so a harness frame reads the same on
    /// every machine.
    #[cfg(any(test, feature = "testkit"))]
    pub(crate) fn print_in_utc(&mut self) {
        self.zone = Zone::Utc { today: None };
    }

    /// `None` until the first reply; then `max_id − OVERLAP`.
    #[must_use]
    pub fn after(&self) -> Option<i64> {
        self.max_id.map(|max| max.saturating_sub(OVERLAP).max(0))
    }

    /// Marks a read as gone out; `false` while one is in flight.
    pub fn begin_request(&mut self) -> bool {
        !std::mem::replace(&mut self.in_flight, true)
    }

    /// One reply: the events merge in by id (a re-read one is not doubled, a late lower one
    /// lands in order), the oldest beyond [`CAPACITY`] drop, and `pinned` and `workspaces` are
    /// replaced wholesale. The next read may go.
    pub fn merge(&mut self, page: ActivityPage, workspaces: Vec<WorkspaceSummary>) {
        self.in_flight = false;
        for event in page.events {
            self.max_id = Some(self.max_id.map_or(event.id, |max| max.max(event.id)));
            self.events.insert(event.id, event);
        }
        while self.events.len() > CAPACITY {
            self.events.pop_first();
        }
        self.pinned = page.pinned;
        self.workspaces = workspaces;
    }

    /// A refused read: the next may go, the rows stay.
    pub fn failed(&mut self) {
        self.in_flight = false;
    }

    /// The rows under the filter: pinned first, then the rest, newest first; a pinned event is
    /// not repeated below.
    #[must_use]
    pub fn rows(&self, scope: &Scope) -> Vec<&ActivityEvent> {
        let (mut rows, pinned) = self.pinned_rows(scope);
        rows.extend(
            self.events
                .values()
                .rev()
                .filter(|event| !pinned.contains(&event.id))
                .filter(|event| self.passes(event, scope, false)),
        );
        rows
    }

    /// The pinned rows under the current filter: the title's count.
    #[must_use]
    pub fn pinned_count(&self, scope: &Scope) -> usize {
        self.pinned_rows(scope).0.len()
    }

    /// One of the panel's acts: the list moves, the filter's cycle, or Enter's jump (D9). Any
    /// other act is not the panel's: `None`.
    pub fn on_act(&mut self, act: Act, scope: &Scope) -> Option<ActivityJump> {
        let ids: Vec<i64> = self.rows(scope).iter().map(|event| event.id).collect();
        let last = ids.len().saturating_sub(1);
        let at = self.at(&ids);
        match act {
            Act::ActivityOpen => {
                let id = *ids.get(at?)?;
                let event = self.event(id)?;
                return Some(self.jump(event, scope));
            }
            Act::ActivityFilter => self.filter = self.filter.next(),
            Act::ListDown => self.put(&ids, at.map_or(0, |at| (at + 1).min(last))),
            Act::ListUp => self.put(&ids, at.map_or(0, |at| at.saturating_sub(1))),
            Act::ListTop => self.put(&ids, 0),
            Act::ListBottom => self.put(&ids, last),
            _ => {}
        }
        None
    }

    /// Draws the panel into `area`: the title with the pinned count and the filter, the rows
    /// (pinned under a `needs you` divider), or the empty state that names a next step; the
    /// hints on the bottom border while `focused`.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>, focused: bool) {
        let theme = ctx.theme;
        let scope = ctx.scope;
        let rows = self.rows(scope);
        let pinned = self.pinned_count(scope);
        let title = if pinned > 0 {
            format!(
                "Activity \u{b7} \u{26a0} {pinned} needs you \u{b7} {}",
                self.filter.label()
            )
        } else {
            format!("Activity \u{b7} {}", self.filter.label())
        };
        let inner = Pane::inner(area);
        // The list: the events, under a `needs you` divider when pinned rows lead and a `recent`
        // one after them when more follow. Review L1: the window is placed from the selection
        // and the dividers first, and only the rows in it are built (the panel holds hundreds).
        let lead = usize::from(pinned > 0);
        let recent = pinned > 0 && rows.len() > pinned;
        let total = rows.len() + lead + usize::from(recent);
        let slot = |index: usize| index + lead + usize::from(recent && index >= pinned);
        let at = self.at(&rows.iter().map(|event| event.id).collect::<Vec<_>>());
        let height = usize::from(inner.height);
        let offset = window(at.map_or(0, slot), total, height);
        let shown = offset..offset.saturating_add(height).min(total);
        let selected = at.map(slot).and_then(|at| at.checked_sub(offset));
        let mut list_rows: Vec<Row<'static>> = Vec::with_capacity(shown.len());
        for (index, event) in rows.iter().enumerate() {
            let at = slot(index);
            if lead == 1 && index == 0 && shown.contains(&0) {
                list_rows.push(divider(NEEDS_YOU_DIVIDER, inner.width, theme));
            }
            if recent && index == pinned && shown.contains(&(at - 1)) {
                list_rows.push(divider(RECENT_DIVIDER, inner.width, theme));
            }
            if at >= shown.end {
                break;
            }
            if shown.contains(&at) {
                list_rows.push(Row {
                    line: self.line(event, scope, theme),
                    kind: RowKind::Item,
                });
            }
        }
        let focus = if focused {
            Focus::Focused
        } else {
            Focus::Unfocused
        };
        let mut pane = Pane::new(&title)
            .focus(focus)
            .accent(ctx.accent())
            .scrollbar(total, offset);
        if focused && let Some(hints) = self.key_hint(ctx.keys(), theme) {
            pane = pane.hints(hints);
        }
        let inner = pane.render(frame, area, theme);
        if total == 0 {
            let (what, next) = self.empty_state(ctx.keys());
            empty::render(frame, inner, &what, NextStep(&next), theme);
        } else {
            list::render(frame, inner, &list_rows, selected, 0, ctx.accent(), theme);
        }
    }

    /// The hint row the focused pane carries, through [`views::ACTIVITY_BROWSE`]; `None` when
    /// no key of it is bound.
    #[must_use]
    pub fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(keys.hint(views::ACTIVITY_BROWSE, HINT), theme)
    }

    /// The pinned events under the filter, newest first, and the ids of every pinned event.
    fn pinned_rows(&self, scope: &Scope) -> (Vec<&ActivityEvent>, HashSet<i64>) {
        let mut rows: Vec<&ActivityEvent> = self
            .pinned
            .iter()
            .filter(|event| self.passes(event, scope, true))
            .collect();
        rows.sort_by_key(|event| std::cmp::Reverse(event.id));
        (rows, self.pinned.iter().map(|event| event.id).collect())
    }

    /// Whether `event` (pinned or not) is listed under the filter.
    fn passes(&self, event: &ActivityEvent, scope: &Scope, pinned: bool) -> bool {
        match self.filter {
            ActivityFilter::All => true,
            ActivityFilter::NeedsYou => pinned,
            ActivityFilter::Failures => event.kind.class() == ActivityClass::Failure,
            ActivityFilter::ThisWorkspace => in_workspace(event, scope),
        }
    }

    /// The selected row among `ids`: the anchored event while it is listed, else the cursor's
    /// index, kept on the list; `None` on an empty list.
    fn at(&self, ids: &[i64]) -> Option<usize> {
        if ids.is_empty() {
            return None;
        }
        self.anchor
            .and_then(|anchor| ids.iter().position(|&id| id == anchor))
            .or(Some(self.cursor.min(ids.len() - 1)))
    }

    /// Selects row `index` of `ids`, anchoring its event.
    fn put(&mut self, ids: &[i64], index: usize) {
        self.cursor = index;
        self.anchor = ids.get(index).copied();
    }

    /// The held event `id`, pinned or not.
    fn event(&self, id: i64) -> Option<&ActivityEvent> {
        self.pinned
            .iter()
            .find(|event| event.id == id)
            .or_else(|| self.events.get(&id))
    }

    /// D9: what Enter on `event` does. A chat replays its step; an item reveals its step, in
    /// the workspace D5 resolves when it is not the current one (none resolved: the reveal's own
    /// miss sentence says so); a row whose chat step, item or run went is [`ActivityJump::Gone`].
    /// An unblock has no run, so only its item counts.
    fn jump(&self, event: &ActivityEvent, scope: &Scope) -> ActivityJump {
        let gone = || ActivityJump::Gone {
            key: row_key(event),
        };
        if event.chat {
            return event
                .step
                .map_or_else(gone, |step| ActivityJump::Replay { step });
        }
        let (Some(item), Some(key)) = (event.item, event.item_key.clone()) else {
            return gone();
        };
        if event.kind != ActivityKind::Unblock && event.run.is_none() {
            return gone();
        }
        let workspace = if in_workspace(event, scope) {
            None
        } else {
            self.workspace_of(event).cloned()
        };
        ActivityJump::Reveal {
            workspace,
            target: RevealTarget::Step {
                item,
                key,
                run: event.run,
                step: event.step,
            },
        }
    }

    /// D5: `event`'s workspace: the one it names, else the first by name whose projects hold
    /// its project.
    fn workspace_of(&self, event: &ActivityEvent) -> Option<&WorkspaceSummary> {
        event
            .workspace
            .and_then(|id| self.workspaces.iter().find(|ws| ws.workspace_id == id))
            .or_else(|| {
                let project = event.project?;
                self.workspaces
                    .iter()
                    .filter(|ws| ws.projects.iter().any(|p| p.project_id == project))
                    .min_by(|a, b| a.name.cmp(&b.name))
            })
    }

    /// One row (D8): glyph and word, `HH:MM`, `@slug` outside the current workspace, the key or
    /// `chat`, the step, the kind's label (a plain kind's word already says it) and the text.
    fn line(&self, event: &ActivityEvent, scope: &Scope, theme: &Theme) -> Line<'static> {
        let (mark, style) = mark(event.kind, theme);
        let mut spans = vec![
            Span::styled(mark, style),
            Span::styled(format!(" {}", self.clock(event.at)), theme.dim),
        ];
        if !in_workspace(event, scope) {
            let slug = self
                .workspace_of(event)
                .map_or(UNKNOWN_WORKSPACE, |ws| ws.slug.as_str());
            spans.push(Span::styled(format!(" @{slug}"), theme.dim));
        }
        spans.push(Span::styled(format!(" {}", row_key(event)), theme.base));
        if !event.chat && event.item.is_none() {
            spans.push(Span::styled(format!(" {DELETED}"), theme.dim));
        }
        if let Some(step) = &event.step_name {
            spans.push(Span::styled(format!(" {step}"), theme.dim));
        }
        let mut words = String::new();
        if event.kind.class() != ActivityClass::Plain {
            words.push_str(event.kind.label());
        }
        let text = event.text.replace(['\n', '\r'], " ");
        if !text.is_empty() {
            if !words.is_empty() {
                words.push(':');
            }
            words.push(' ');
            words.push_str(text.trim());
        }
        if !words.is_empty() {
            spans.push(Span::styled(format!(" {}", words.trim_start()), theme.base));
        }
        Line::from(spans)
    }

    /// `at` in the panel's zone: `HH:MM` today, `MM-DD HH:MM` on any other day (review L4, the
    /// Runs table's [`STAMP`]).
    fn clock(&self, at: DateTime<Utc>) -> String {
        match self.zone {
            Zone::Local => stamp(
                at.with_timezone(&chrono::Local).naive_local(),
                chrono::Local::now().date_naive(),
            ),
            #[cfg(any(test, feature = "testkit"))]
            Zone::Utc { today } => stamp(
                at.naive_utc(),
                today.unwrap_or_else(|| Utc::now().date_naive()),
            ),
        }
    }

    /// What the pane says with no row, and its next step: nothing held yet, or a filter that
    /// hides everything, naming the key that moves it on.
    fn empty_state(&self, keys: &Keys) -> (String, String) {
        if self.events.is_empty() && self.pinned.is_empty() {
            return (NO_EVENTS.to_owned(), NO_EVENTS_NEXT.to_owned());
        }
        let next = self.filter.next().label();
        let step = keys
            .label(views::ACTIVITY_BROWSE, Act::ActivityFilter)
            .map_or_else(
                || format!("{UNBOUND_FILTER} {next}"),
                |key| format!("{key} shows {next}"),
            );
        (format!("No events under {}.", self.filter.label()), step)
    }
}

/// The hint row the focused pane carries.
const HINT: HintSpec = &[
    Hint::One(Act::ActivityOpen, "open"),
    Hint::One(Act::ActivityFilter, "filter"),
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
];

/// The divider over the pinned rows (D8).
const NEEDS_YOU_DIVIDER: &str = "needs you";

/// The divider over the rest, when pinned rows lead.
const RECENT_DIVIDER: &str = "recent";

/// The `@slug` of an event no workspace resolves (D5).
const UNKNOWN_WORKSPACE: &str = "?";

/// After the key of a row whose item is gone.
const DELETED: &str = "(deleted)";

/// The empty panel and its next step (D8).
const NO_EVENTS: &str = "No activity yet.";
const NO_EVENTS_NEXT: &str = "events appear as runs start, wait and fail";

/// The next step under a filter that hides everything while `activity.filter` has no key.
const UNBOUND_FILTER: &str = "bind activity.filter to show";

/// A row's time (D8, review L4): `HH:MM` on `today`, else `MM-DD HH:MM`.
fn stamp(at: NaiveDateTime, today: NaiveDate) -> String {
    if at.date() == today {
        at.format("%H:%M").to_string()
    } else {
        at.format(STAMP).to_string()
    }
}

/// D5: whether `event` is the current workspace's: it names the workspace (a run started there,
/// or the workspace's own scope row), or its project is in the scope.
fn in_workspace(event: &ActivityEvent, scope: &Scope) -> bool {
    event.workspace == Some(scope.workspace_id)
        || event.project.is_some_and(|project| scope.contains(project))
}

/// The row's key: the item's, `chat`, else the project's slug.
fn row_key(event: &ActivityEvent) -> String {
    match &event.item_key {
        Some(key) => key.clone(),
        None if event.chat => "chat".to_owned(),
        None => event.project_slug.clone(),
    }
}

/// D8: a kind's glyph and word, never colour alone, and the role they are drawn in.
fn mark(kind: ActivityKind, theme: &Theme) -> (&'static str, Style) {
    match (kind.class(), kind) {
        (ActivityClass::NeedsYou, _) => ("\u{26a0} needs you", theme.warning),
        (ActivityClass::Failure, _) => ("\u{2717} failed", theme.error),
        (_, ActivityKind::RunStarted) => ("\u{25d0} started", theme.running),
        (_, ActivityKind::CloseOut) => ("\u{2713} ready to close", theme.added),
        _ => ("\u{25cf} finished", theme.base),
    }
}

/// A divider row: the word, then a rule to `width`, dim, never selected.
fn divider(word: &str, width: u16, theme: &Theme) -> Row<'static> {
    let rule = "\u{2500}".repeat(usize::from(width).saturating_sub(word.len() + 2));
    Row {
        line: Line::styled(format!("{word} {rule}"), theme.dim),
        kind: RowKind::Note,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use htui_core::model::{ItemId, ProjectId, ProjectRef, RunId, WorkspaceId};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use uuid::Uuid;

    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::store_worker::Origin;

    /// The current workspace, `Graphics`, holding project 1.
    const HERE: u128 = 1;
    /// `Audio`, holding project 2.
    const AUDIO: u128 = 2;
    /// `Beta` and `Alpha` both hold project 3; `Alpha` comes first by name.
    const BETA: u128 = 3;
    const ALPHA: u128 = 4;

    fn workspace_id(n: u128) -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(n))
    }

    fn project_id(n: u128) -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(100 + n))
    }

    fn summary(n: u128, slug: &str, name: &str, projects: &[u128]) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: workspace_id(n),
            slug: slug.to_owned(),
            name: name.to_owned(),
            projects: projects
                .iter()
                .zip(0..)
                .map(|(&p, position)| ProjectRef {
                    project_id: project_id(p),
                    slug: format!("p{p}"),
                    name: format!("Project {p}"),
                    position,
                })
                .collect(),
        }
    }

    /// Every workspace, `Beta` listed before `Alpha` so the by-name pick is the panel's.
    fn workspaces() -> Vec<WorkspaceSummary> {
        vec![
            summary(HERE, "gfx", "Graphics", &[1]),
            summary(AUDIO, "aud", "Audio", &[2]),
            summary(BETA, "beta", "Beta", &[3]),
            summary(ALPHA, "alpha", "Alpha", &[3]),
        ]
    }

    fn scope() -> Scope {
        Scope::from_workspace(&summary(HERE, "gfx", "Graphics", &[1]))
    }

    fn at(minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 14, 9, 0, 0)
            .single()
            .expect("a valid time")
            + chrono::TimeDelta::minutes(minute)
    }

    /// Event `id` of `kind` on `MOD-<id>` in the current workspace's project, at minute `id`.
    fn event(id: i64, kind: ActivityKind) -> ActivityEvent {
        let n = u128::try_from(id).expect("positive");
        ActivityEvent {
            id,
            at: at(id % 60),
            kind,
            workspace: Some(workspace_id(HERE)),
            project: Some(project_id(1)),
            project_slug: "p1".to_owned(),
            item: Some(ItemId::from_uuid(Uuid::from_u128(1_000 + n))),
            item_key: Some(format!("MOD-{id}")),
            run: Some(RunId::from_uuid(Uuid::from_u128(2_000 + n))),
            chat: false,
            step: Some(StepId::from_uuid(Uuid::from_u128(3_000 + n))),
            step_name: Some("research".to_owned()),
            permission: None,
            text: String::new(),
        }
    }

    /// `event` in workspace `ws` (or none) on project `project`.
    fn elsewhere(id: i64, ws: Option<u128>, project: u128) -> ActivityEvent {
        ActivityEvent {
            workspace: ws.map(workspace_id),
            project: Some(project_id(project)),
            project_slug: format!("p{project}"),
            ..event(id, ActivityKind::RunStarted)
        }
    }

    fn page(events: Vec<ActivityEvent>, pinned: Vec<ActivityEvent>) -> ActivityPage {
        ActivityPage { events, pinned }
    }

    /// A panel that prints times in UTC, so no snapshot moves with the machine's zone, on the
    /// day of [`at`]'s events.
    fn fresh() -> ActivityPanel {
        ActivityPanel {
            zone: Zone::Utc {
                today: Some(at(0).date_naive()),
            },
            ..ActivityPanel::new()
        }
    }

    fn ids(rows: &[&ActivityEvent]) -> Vec<i64> {
        rows.iter().map(|event| event.id).collect()
    }

    fn plain(range: std::ops::RangeInclusive<i64>) -> Vec<ActivityEvent> {
        range
            .map(|id| event(id, ActivityKind::RunStarted))
            .collect()
    }

    // ---- merge ----

    #[test]
    fn the_ring_holds_capacity_and_drops_the_oldest() {
        assert_eq!(CAPACITY, 500);
        let mut panel = fresh();
        panel.merge(page(plain(1..=505), Vec::new()), workspaces());
        let rows = ids(&panel.rows(&scope()));
        assert_eq!(rows.len(), CAPACITY);
        assert_eq!(rows.first(), Some(&505));
        assert_eq!(rows.last(), Some(&6));
    }

    #[test]
    fn a_repeated_id_is_not_doubled_and_a_late_lower_id_lands_in_order() {
        let mut panel = fresh();
        panel.merge(page(plain(1..=3), Vec::new()), workspaces());
        panel.merge(page(plain(2..=4), Vec::new()), workspaces());
        assert_eq!(ids(&panel.rows(&scope())), [4, 3, 2, 1]);
        let mut late = fresh();
        late.merge(
            page(
                vec![
                    event(10, ActivityKind::RunStarted),
                    event(12, ActivityKind::RunStarted),
                ],
                Vec::new(),
            ),
            workspaces(),
        );
        late.merge(
            page(vec![event(11, ActivityKind::RunStarted)], Vec::new()),
            workspaces(),
        );
        assert_eq!(ids(&late.rows(&scope())), [12, 11, 10]);
    }

    #[test]
    fn pinned_is_replaced_wholesale() {
        let mut panel = fresh();
        panel.merge(
            page(
                Vec::new(),
                vec![
                    event(2, ActivityKind::Gate),
                    event(1, ActivityKind::Permission),
                ],
            ),
            workspaces(),
        );
        assert_eq!(ids(&panel.rows(&scope())), [2, 1]);
        panel.merge(
            page(Vec::new(), vec![event(3, ActivityKind::Selection)]),
            workspaces(),
        );
        assert_eq!(ids(&panel.rows(&scope())), [3]);
        panel.merge(page(Vec::new(), Vec::new()), workspaces());
        assert!(panel.rows(&scope()).is_empty());
    }

    #[test]
    fn after_is_none_until_a_reply_then_the_overlap_below_the_highest_id() {
        let mut panel = fresh();
        assert_eq!(panel.after(), None);
        panel.merge(page(plain(440..=450), Vec::new()), workspaces());
        assert_eq!(panel.after(), Some(250));
        // A late lower id does not lower the cursor.
        panel.merge(page(plain(300..=300), Vec::new()), workspaces());
        assert_eq!(panel.after(), Some(250));
        let mut low = fresh();
        low.merge(page(plain(1..=5), Vec::new()), workspaces());
        assert_eq!(low.after(), Some(0));
    }

    #[test]
    fn one_read_in_flight_and_a_failure_keeps_the_rows() {
        let mut panel = fresh();
        assert!(panel.begin_request());
        assert!(!panel.begin_request());
        panel.merge(page(plain(1..=2), Vec::new()), workspaces());
        assert!(panel.begin_request());
        panel.failed();
        assert_eq!(ids(&panel.rows(&scope())), [2, 1]);
        assert!(panel.begin_request());
    }

    // ---- rows and filters ----

    /// Pinned 7 (gate) and 5 (permission, also in the history), failures 6 (step) and 4 (run),
    /// 3 started in Audio, 2 finished in a workspace's own scope row, 1 started here.
    fn mixed() -> ActivityPanel {
        let mut own_row = elsewhere(2, Some(HERE), 9);
        own_row.kind = ActivityKind::RunFinished;
        let mut panel = fresh();
        panel.merge(
            page(
                vec![
                    event(1, ActivityKind::RunStarted),
                    own_row,
                    elsewhere(3, Some(AUDIO), 2),
                    event(4, ActivityKind::RunFailed),
                    event(5, ActivityKind::Permission),
                    event(6, ActivityKind::StepFailed),
                ],
                vec![
                    event(7, ActivityKind::Gate),
                    event(5, ActivityKind::Permission),
                ],
            ),
            workspaces(),
        );
        panel
    }

    #[test]
    fn pinned_come_first_newest_first_and_are_not_repeated_below() {
        let panel = mixed();
        assert_eq!(ids(&panel.rows(&scope())), [7, 5, 6, 4, 3, 2, 1]);
        assert_eq!(panel.pinned_count(&scope()), 2);
    }

    #[test]
    fn each_filter_keeps_its_rows() {
        let mut panel = mixed();
        let scope = scope();
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push((
                panel.filter,
                ids(&panel.rows(&scope)),
                panel.pinned_count(&scope),
            ));
            assert_eq!(panel.on_act(Act::ActivityFilter, &scope), None);
        }
        assert_eq!(
            seen,
            [
                (ActivityFilter::All, vec![7, 5, 6, 4, 3, 2, 1], 2),
                (ActivityFilter::NeedsYou, vec![7, 5], 2),
                (ActivityFilter::Failures, vec![6, 4], 0),
                // D5: 2 is the workspace's own scope row (its project is in no scope list).
                (ActivityFilter::ThisWorkspace, vec![7, 5, 6, 4, 2, 1], 2),
            ]
        );
        assert_eq!(panel.filter, ActivityFilter::All, "the cycle wraps");
        let labels: Vec<&str> = [
            ActivityFilter::All,
            ActivityFilter::NeedsYou,
            ActivityFilter::Failures,
            ActivityFilter::ThisWorkspace,
        ]
        .map(ActivityFilter::label)
        .to_vec();
        assert_eq!(
            labels,
            ["all workspaces", "needs you", "failures", "this workspace"]
        );
        assert_eq!(ActivityFilter::ThisWorkspace.next(), ActivityFilter::All);
    }

    #[test]
    fn a_project_in_scope_counts_as_this_workspace_whatever_its_run_started_from() {
        let mut panel = fresh();
        panel.merge(
            page(
                vec![elsewhere(1, Some(AUDIO), 1), elsewhere(2, None, 1)],
                Vec::new(),
            ),
            workspaces(),
        );
        panel.filter = ActivityFilter::ThisWorkspace;
        assert_eq!(ids(&panel.rows(&scope())), [2, 1]);
    }

    // ---- on_act ----

    #[test]
    fn enter_on_an_item_in_scope_reveals_its_step_here() {
        let mut panel = fresh();
        let row = event(1, ActivityKind::Gate);
        panel.merge(page(vec![row.clone()], Vec::new()), workspaces());
        assert_eq!(
            panel.on_act(Act::ActivityOpen, &scope()),
            Some(ActivityJump::Reveal {
                workspace: None,
                target: RevealTarget::Step {
                    item: row.item.expect("an item"),
                    key: "MOD-1".to_owned(),
                    run: row.run,
                    step: row.step,
                },
            })
        );
    }

    #[test]
    fn enter_out_of_scope_names_the_workspace_by_id_then_first_by_name() {
        let scope = scope();
        let open = |row: ActivityEvent| {
            let mut panel = fresh();
            panel.merge(page(vec![row], Vec::new()), workspaces());
            match panel.on_act(Act::ActivityOpen, &scope) {
                Some(ActivityJump::Reveal { workspace, .. }) => workspace.map(|ws| ws.slug),
                other => panic!("not a reveal: {other:?}"),
            }
        };
        assert_eq!(open(elsewhere(1, Some(AUDIO), 2)), Some("aud".to_owned()));
        assert_eq!(open(elsewhere(1, Some(BETA), 3)), Some("beta".to_owned()));
        assert_eq!(open(elsewhere(1, None, 3)), Some("alpha".to_owned()));
        assert_eq!(open(elsewhere(1, None, 8)), None, "no workspace holds it");
    }

    #[test]
    fn enter_on_a_chat_replays_and_on_a_deleted_row_says_so() {
        let scope = scope();
        let open = |row: ActivityEvent| {
            let mut panel = fresh();
            panel.merge(page(vec![row], Vec::new()), workspaces());
            panel.on_act(Act::ActivityOpen, &scope)
        };
        let mut chat = event(1, ActivityKind::Permission);
        chat.chat = true;
        chat.item = None;
        chat.item_key = None;
        let step = chat.step.expect("a step");
        assert_eq!(open(chat.clone()), Some(ActivityJump::Replay { step }));
        chat.step = None;
        assert_eq!(
            open(chat),
            Some(ActivityJump::Gone {
                key: "chat".to_owned()
            })
        );
        let mut deleted = event(2, ActivityKind::StepFailed);
        deleted.item = None;
        deleted.run = None;
        deleted.step = None;
        assert_eq!(
            open(deleted),
            Some(ActivityJump::Gone {
                key: "MOD-2".to_owned()
            })
        );
        let mut run_gone = event(3, ActivityKind::RunFailed);
        run_gone.run = None;
        assert_eq!(
            open(run_gone),
            Some(ActivityJump::Gone {
                key: "MOD-3".to_owned()
            })
        );
        // An unblock has no run: its item is all it needs.
        let mut unblock = event(4, ActivityKind::Unblock);
        unblock.run = None;
        unblock.step = None;
        assert!(matches!(
            open(unblock),
            Some(ActivityJump::Reveal {
                workspace: None,
                ..
            })
        ));
        let mut empty = fresh();
        assert_eq!(empty.on_act(Act::ActivityOpen, &scope), None);
    }

    #[test]
    fn the_cursor_moves_and_stays_on_its_event_when_rows_arrive_above() {
        let scope = scope();
        let mut panel = fresh();
        panel.merge(page(plain(1..=5), Vec::new()), workspaces());
        let opened = |panel: &mut ActivityPanel| match panel.on_act(Act::ActivityOpen, &scope) {
            Some(ActivityJump::Reveal {
                target: RevealTarget::Step { key, .. },
                ..
            }) => key,
            other => panic!("not a reveal: {other:?}"),
        };
        assert_eq!(opened(&mut panel), "MOD-5");
        panel.on_act(Act::ListDown, &scope);
        panel.on_act(Act::ListDown, &scope);
        assert_eq!(opened(&mut panel), "MOD-3");
        panel.on_act(Act::ListUp, &scope);
        assert_eq!(opened(&mut panel), "MOD-4");
        panel.on_act(Act::ListBottom, &scope);
        assert_eq!(opened(&mut panel), "MOD-1");
        panel.on_act(Act::ListDown, &scope);
        assert_eq!(opened(&mut panel), "MOD-1", "the bottom holds");
        panel.on_act(Act::ListTop, &scope);
        assert_eq!(opened(&mut panel), "MOD-5");
        panel.on_act(Act::ListUp, &scope);
        assert_eq!(opened(&mut panel), "MOD-5", "the top holds");
        panel.on_act(Act::ListDown, &scope);
        assert_eq!(opened(&mut panel), "MOD-4");
        panel.merge(
            page(plain(6..=8), vec![event(9, ActivityKind::Gate)]),
            workspaces(),
        );
        assert_eq!(
            opened(&mut panel),
            "MOD-4",
            "new rows above keep the cursor's event"
        );
    }

    // ---- drawing ----

    /// The panel drawn into a `width` x `height` frame, with the compiled keys.
    fn draw(panel: &ActivityPanel, width: u16, height: u16, focused: bool) -> Buffer {
        let scope = scope();
        let top_bar = TopBarState::default();
        let theme = Theme::default();
        let emit = Emit::default();
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            Keys::compiled(),
            &theme,
            Origin::App,
            &emit,
        );
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| panel.render(frame, frame.area(), &ctx, focused))
            .expect("drawn");
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &Buffer) -> String {
        let area = buffer.area;
        let mut out = String::new();
        for y in area.top()..area.bottom() {
            let mut line = String::new();
            for x in area.left()..area.right() {
                line.push_str(buffer[(x, y)].symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    fn frame(panel: &ActivityPanel, width: u16, height: u16) -> String {
        text(&draw(panel, width, height, true))
    }

    #[test]
    fn an_empty_panel_says_where_events_come_from() {
        let frame = frame(&fresh(), 60, 8);
        assert!(
            frame.contains("events appear as runs start, wait and fail"),
            "{frame}"
        );
        insta::assert_snapshot!("empty", frame);
    }

    #[test]
    fn a_filter_that_hides_everything_names_its_key() {
        let mut panel = fresh();
        panel.merge(page(plain(1..=2), Vec::new()), workspaces());
        panel.on_act(Act::ActivityFilter, &scope());
        let frame = frame(&panel, 60, 8);
        assert!(frame.contains("f shows failures"), "{frame}");
        insta::assert_snapshot!("empty_under_a_filter", frame);
    }

    #[test]
    fn pinned_rows_lead_under_their_divider_and_the_title_counts_them() {
        let mut panel = mixed();
        let mut gate = event(7, ActivityKind::Gate);
        gate.text = "approve the plan".to_owned();
        panel.pinned[0] = gate;
        let frame = frame(&panel, 90, 14);
        assert!(
            frame.contains("Activity \u{b7} \u{26a0} 2 needs you \u{b7} all workspaces"),
            "{frame}"
        );
        assert!(frame.contains("needs you"), "{frame}");
        assert!(frame.contains("\u{2717} failed"), "{frame}");
        assert!(frame.contains("\u{25d0} started"), "{frame}");
        assert!(frame.contains("\u{25cf} finished"), "{frame}");
        assert!(frame.contains("gate: approve the plan"), "{frame}");
        insta::assert_snapshot!("pinned_and_history", frame);
        panel.filter = ActivityFilter::Failures;
        let failures = text(&draw(&panel, 90, 14, true));
        assert!(
            failures.contains("Activity \u{b7} failures"),
            "no count at 0: {failures}"
        );
    }

    #[test]
    fn another_workspace_shows_its_slug() {
        let mut panel = fresh();
        let mut close = elsewhere(3, None, 3);
        close.kind = ActivityKind::CloseOut;
        panel.merge(
            page(
                vec![
                    event(1, ActivityKind::RunStarted),
                    elsewhere(2, Some(AUDIO), 2),
                    close,
                    elsewhere(4, None, 8),
                ],
                Vec::new(),
            ),
            workspaces(),
        );
        let frame = frame(&panel, 80, 8);
        assert!(frame.contains("@aud MOD-2"), "{frame}");
        assert!(frame.contains("@alpha MOD-3"), "{frame}");
        assert!(frame.contains("@? MOD-4"), "{frame}");
        assert!(frame.contains("\u{2713} ready to close"), "{frame}");
        assert!(
            !frame.contains("@gfx"),
            "the current workspace has no slug: {frame}"
        );
        insta::assert_snapshot!("other_workspaces", frame);
    }

    #[test]
    fn a_removed_items_row_still_reads() {
        let mut panel = fresh();
        let mut gone = event(1, ActivityKind::StepFailed);
        gone.item = None;
        gone.run = None;
        gone.step = None;
        gone.text = "exit 1".to_owned();
        let mut chat = event(2, ActivityKind::RunStarted);
        chat.chat = true;
        chat.item = None;
        chat.item_key = None;
        panel.merge(page(vec![gone, chat], Vec::new()), workspaces());
        let frame = frame(&panel, 80, 6);
        assert!(frame.contains("MOD-1 (deleted)"), "{frame}");
        assert!(frame.contains("chat research"), "{frame}");
        insta::assert_snapshot!("removed_item", frame);
    }

    #[test]
    fn a_narrow_pane_cuts_rows_with_an_ellipsis() {
        let mut panel = fresh();
        let mut long = event(1, ActivityKind::StepFailed);
        long.text = "the agent exited before it wrote the plan".to_owned();
        panel.merge(page(vec![long], Vec::new()), workspaces());
        let frame = frame(&panel, 30, 6);
        assert!(frame.contains('\u{2026}'), "{frame}");
        insta::assert_snapshot!("narrow", frame);
    }

    #[test]
    fn only_the_focused_panel_carries_its_hints_and_accent() {
        let mut panel = fresh();
        panel.merge(page(plain(1..=2), Vec::new()), workspaces());
        let focused = draw(&panel, 60, 6, true);
        let unfocused = draw(&panel, 60, 6, false);
        assert_ne!(
            focused[(0, 0)].style(),
            unfocused[(0, 0)].style(),
            "the border shows focus"
        );
        let (focused, unfocused) = (text(&focused), text(&unfocused));
        assert!(focused.contains("open"), "{focused}");
        assert!(!unfocused.contains("open"), "{unfocused}");
        insta::assert_snapshot!("focused", focused);
        insta::assert_snapshot!("unfocused", unfocused);
    }

    /// Review L4: a row from before today carries its date, as the Runs table's do; today's
    /// keep the bare time.
    #[test]
    fn a_row_from_before_today_carries_its_date() {
        let mut panel = fresh();
        let mut yesterday = event(2, ActivityKind::RunStarted);
        yesterday.at = at(0) - chrono::TimeDelta::minutes(9 * 60 + 1);
        panel.merge(
            page(
                vec![event(1, ActivityKind::RunStarted), yesterday],
                Vec::new(),
            ),
            workspaces(),
        );
        let frame = frame(&panel, 60, 6);
        assert!(frame.contains("started 11-13 23:59 MOD-2"), "{frame}");
        assert!(frame.contains("started 09:01 MOD-1"), "{frame}");
    }

    /// Review L1: only the rows in view are built, the window placed from the selection and the
    /// dividers: at the bottom of a full ring the oldest row is selected and the dividers are
    /// off screen; at the top both lead, with the scrollbar's length the whole list.
    #[test]
    fn a_long_list_draws_the_window_around_the_selection() {
        let mut panel = fresh();
        panel.merge(
            page(
                plain(1..=CAPACITY as i64),
                vec![
                    event(900, ActivityKind::Gate),
                    event(901, ActivityKind::Gate),
                ],
            ),
            workspaces(),
        );
        panel.on_act(Act::ListBottom, &scope());
        let bottom = frame(&panel, 60, 8);
        let rows: Vec<&str> = bottom.lines().collect();
        assert!(rows[6].contains("MOD-1 research"), "{bottom}");
        assert!(
            rows[6].starts_with("\u{2502}\u{258c}"),
            "selected:\n{bottom}"
        );
        assert!(rows[5].contains("MOD-2 research"), "{bottom}");
        assert!(!bottom.contains("needs you \u{2500}"), "{bottom}");
        assert!(!bottom.contains("recent \u{2500}"), "{bottom}");

        panel.on_act(Act::ListTop, &scope());
        let top = frame(&panel, 60, 8);
        let rows: Vec<&str> = top.lines().collect();
        assert!(rows[1].contains("needs you \u{2500}"), "{top}");
        assert!(rows[2].contains("MOD-901"), "{top}");
        assert!(rows[2].starts_with("\u{2502}\u{258c}"), "selected:\n{top}");
        assert!(rows[3].contains("MOD-900"), "{top}");
        assert!(rows[4].contains("recent \u{2500}"), "{top}");
        assert!(rows[5].contains(&format!("MOD-{CAPACITY} ")), "{top}");

        // The `recent` divider on the window's last row, the event after it off screen.
        panel.on_act(Act::ListDown, &scope());
        let edge = frame(&panel, 60, 5);
        let rows: Vec<&str> = edge.lines().collect();
        assert!(rows[1].contains("MOD-901"), "{edge}");
        assert!(rows[2].contains("MOD-900"), "{edge}");
        assert!(rows[3].contains("recent \u{2500}"), "{edge}");
    }

    #[test]
    fn the_hint_row_resolves_through_the_panel_stack() {
        let hint = fresh()
            .key_hint(Keys::compiled(), &Theme::default())
            .expect("bound keys");
        let words: String = hint
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(
            words.contains("open") && words.contains("filter"),
            "{words}"
        );
    }
}
