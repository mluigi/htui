//! The concepts search (MOD-64 D233, D234, D236): a modal overlay over the concepts index.
//!
//! A query field, a project scope (`Ctrl+P`: every project of the workspace, then each one), a
//! decisions toggle (`Ctrl+D`) and the last hits. `Enter` searches when the query or a toggle
//! changed since the last search, and otherwise opens the highlighted hit through
//! `Action::Reveal` (D234, D235). `Ctrl+R` re-indexes the current scope (D237).
//!
//! Drawn on the MOD-115 kit (M4 E6): [`Modal`] is the frame, `kit::list` the hits (the `▌` gutter
//! and the selection band mark the cursor, MOD-85), and the keys sit on the modal's bottom
//! border. The overlay builds its own rows and index line (M4 C9, D-19): a hit's score reads as a
//! word relative to the best hit (Q4), counts go through `kit::label::plural`, while
//! `concepts::format_hit` and `concepts::report_line` stay the CLI's (`htui --search-items`,
//! `--index-items`).
//!
//! Like every view it holds no store handle and no channel: the search and the index run leave
//! through `Ctx::request` as `StoreRequest::SearchConcepts` / `IndexConcepts`, which the worker
//! serves on tasks of its own (`R-NF-3`), and come back through `on_reply` as
//! `StoreReply::Concepts`, with the error inside (D232): a missing or unreachable Qdrant is a line
//! in this box and nothing else.

use crossterm::event::KeyEvent;
use htui_core::model::{ProjectId, ProjectRef, RequirementState, Scope};
use htui_core::qdrant::KNOWLEDGE_SEARCH_UNAVAILABLE;
use htui_store::vector::{Hit, Owner, PointType, QUERY_EMBED_FAILED, SearchQuery};
use htui_store::vector_sync::SyncReport;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled, OverlayAction, RevealTarget};
use crate::concepts;
use crate::concepts_worker::ConceptsReply;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::{cell_width, graphemes};
use crate::ui::kit::label::{self, plural};
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::modal::{CHROME, Modal, ModalSize};
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crate::ui::{FieldOutcome, TextField, Theme};

/// The box's title.
const TITLE: &str = "Search concepts";

/// In front of the query field.
const QUERY_LABEL: &str = "query ";

/// Rows of hits: the CLI's default limit, so the box shows every hit a search asks for.
const HIT_ROWS: u16 = 10;

/// The content's rows around the hits: the header, the query, a blank, then the status and the
/// index rows under the hits.
const OTHER_ROWS: u16 = 5;

/// The widest the box grows on a wide terminal.
const MAX_WIDTH: u16 = 120;

/// The header's scope when no project is cycled to.
pub const ALL_PROJECTS: &str = "all projects";

/// This overlay's first search: the worker may be loading (or downloading) the model (F10).
pub const LOADING: &str = "loading the embedding model…";

/// Every later search.
pub const SEARCHING: &str = "searching…";

/// While a `Ctrl+R` run is going.
pub const INDEXING: &str = "indexing…";

/// Before the first search: the field's submit key in force names how to search (MOD-115 M4
/// MD-A), `submit` being `FieldSubmit`'s label through `CONCEPTS_QUERY`; with none bound only the
/// first half is said.
#[must_use]
pub fn idle(submit: Option<&str>) -> String {
    match submit {
        Some(submit) => format!("type a query; {submit} searches"),
        None => "type a query".to_owned(),
    }
}

/// A workspace with no project: nothing is sent (D233).
pub const NO_PROJECTS: &str = "this workspace has no projects";

/// `Enter` on an empty query.
pub const EMPTY_QUERY: &str = "type a query first";

/// A search that found nothing. `reindex` is `ConceptsReindex`'s label through `CONCEPTS_QUERY`
/// (MOD-67 M6 D4); with none bound the hint goes.
#[must_use]
pub fn no_matches(reindex: Option<&str>) -> String {
    match reindex {
        Some(reindex) => {
            format!("no matches \u{2014} {reindex} indexes this scope if the index is empty")
        }
        None => "no matches".to_owned(),
    }
}

/// After the list, when the query or a toggle moved since it was searched; `submit` as in
/// [`idle`].
#[must_use]
pub fn changed(submit: Option<&str>) -> String {
    match submit {
        Some(submit) => format!("changed \u{2014} {submit} searches again"),
        None => "changed".to_owned(),
    }
}

/// A failed search's status row (MOD-160 D7): the one sentence every search surface says, then
/// the why key in force, `why` being `ConceptsWhy`'s label through `CONCEPTS_QUERY`; none bound,
/// the sentence alone. The cause is one key away, on the why row.
#[must_use]
pub fn unavailable_status(why: Option<&str>) -> String {
    match why {
        Some(why) => format!("{KNOWLEDGE_SEARCH_UNAVAILABLE} \u{2014} {why} why"),
        None => KNOWLEDGE_SEARCH_UNAVAILABLE.to_owned(),
    }
}

/// Where a missing or unreachable index is fixed (MOD-115 M4 E6, MOD-83).
const QDRANT_PLACE: &str = "Settings \u{203a} Qdrant";

/// Where a store read the index sync made is fixed (MOD-115 M4 RV1-M1).
const CONNECTION_PLACE: &str = "Settings \u{203a} Connection";

/// What to do about an embedding model that could not be fetched: no Settings section fetches it
/// (MOD-115 M4 RV1-M1).
const NETWORK_STEP: &str = "check the network";

/// The keys, on the bottom border (MOD-67 D9). `Enter` is the field's own submit (D13), named
/// through `Keys` (MOD-115 M4 MD-A); the short `scope` and `index` labels and the arrows as the
/// header writes them keep the row on the border at 100 columns (L-E Q5, maintainer 2026-10-08).
const HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "search/open"),
    Hint::Glyphs(Act::ConceptsUp, Act::ConceptsDown, "move"),
    Hint::One(Act::ConceptsDecisions, "decisions"),
    Hint::One(Act::ConceptsProject, "scope"),
    Hint::One(Act::ConceptsReindex, "index"),
    Hint::One(Act::OverlayClose, "close"),
];

/// [`HINT`] while a failed search's cause is there to show (MOD-160 D7): the why key joins it
/// before `close`. A failed search clears the hits, so the cursor's `move` has nothing to move and
/// gives way: the row stays whole on the border at 100 columns (L-E Q5, MOD-160 R-3).
const HINT_WHY: HintSpec = &[
    Hint::One(Act::FieldSubmit, "search/open"),
    Hint::One(Act::ConceptsDecisions, "decisions"),
    Hint::One(Act::ConceptsProject, "scope"),
    Hint::One(Act::ConceptsReindex, "index"),
    Hint::One(Act::ConceptsWhy, "why"),
    Hint::One(Act::OverlayClose, "close"),
];

/// The concepts search (MOD-64): a query field, a project scope, a decisions toggle and the last
/// hits, searched off the UI thread through `StoreRequest::SearchConcepts`.
#[derive(Debug, Default)]
pub struct ConceptsSearch {
    /// The query as typed.
    field: TextField,
    /// `None`: every project of the workspace; `Some`: the one `Ctrl+P` cycled to (D233).
    project: Option<ProjectId>,
    /// `Ctrl+D`: decisions only.
    decisions: bool,
    /// The last search sent, in flight or answered: what tells `Enter`-searches from `Enter`-opens
    /// (D234, blueprint D255). `None` before the first and after a failed one, so `Enter` retries.
    sent: Option<SearchQuery>,
    /// A search is in flight.
    searching: bool,
    /// This overlay has had a search answered: `searching…` rather than the model line (F10).
    answered: bool,
    /// The last answered hits, best first.
    hits: Vec<Hit>,
    /// The highlighted hit.
    cursor: usize,
    /// The last search's own error, drawn in the box (D232).
    error: Option<String>,
    /// The why row is open: the search's cause, with where it is fixed (MOD-160 D7).
    why: bool,
    /// A refusal that sent nothing: no projects, no query.
    notice: Option<&'static str>,
    /// The `Ctrl+R` line (D237).
    index: IndexLine,
}

/// What the `Ctrl+R` row says.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum IndexLine {
    /// No run this overlay.
    #[default]
    Idle,
    /// A run is going.
    Running,
    /// The last run's report line.
    Done(String),
    /// Why the last run stopped.
    Failed(String),
}

impl ConceptsSearch {
    /// Identity of the search; `register_all` registers the factory and `Ctrl+F` under it.
    pub const ID: OverlayId = OverlayId("concepts_search");

    /// An empty search over every project.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The projects a search or an index run covers: the workspace's, or the one cycled to (D233).
    fn projects(&self, scope: &Scope) -> Vec<ProjectId> {
        self.project
            .map_or_else(|| scope.project_ids.clone(), |project| vec![project])
    }

    /// The search the overlay's state asks for now.
    fn current(&self, scope: &Scope) -> SearchQuery {
        concepts::query(
            self.field.text().unwrap_or_default().trim(),
            self.projects(scope),
            self.decisions,
            concepts::DEFAULT_LIMIT,
        )
    }

    /// Whether the query or a toggle moved since the last search was sent (blueprint D255).
    fn changed(&self, scope: &Scope) -> bool {
        self.sent.as_ref() != Some(&self.current(scope))
    }

    /// `Ctrl+P`: every project, then each in the workspace's order, then every project again. A
    /// project no longer in the workspace falls back to every project.
    fn cycle_project(&mut self, projects: &[ProjectRef]) {
        self.project = match self.project {
            None => projects.first().map(|project| project.project_id),
            Some(current) => projects
                .iter()
                .position(|project| project.project_id == current)
                .and_then(|at| projects.get(at + 1))
                .map(|project| project.project_id),
        };
    }

    /// `Ctrl+R`: re-index the current scope (D237). The reply prints the CLI's report line.
    fn reindex(&mut self, ctx: &Ctx<'_>) {
        let project_ids = self.projects(ctx.scope);
        if project_ids.is_empty() {
            self.notice = Some(NO_PROJECTS);
            return;
        }
        self.index = IndexLine::Running;
        ctx.request(StoreRequest::IndexConcepts {
            scope: Scope {
                workspace_id: ctx.scope.workspace_id,
                project_ids,
            },
        });
    }

    /// `Enter` (D234, blueprint D255): search when the state moved since the last search, else
    /// open the highlighted hit — close, then reveal, both applied by the same drain (D235).
    fn enter(&mut self, ctx: &Ctx<'_>) {
        let query = self.current(ctx.scope);
        if self.sent.as_ref() != Some(&query) {
            if query.projects.is_empty() {
                self.notice = Some(NO_PROJECTS);
            } else if query.text.is_empty() {
                self.notice = Some(EMPTY_QUERY);
            } else {
                self.sent = Some(query.clone());
                self.searching = true;
                self.hits.clear();
                self.cursor = 0;
                self.error = None;
                self.why = false;
                self.notice = None;
                ctx.request(StoreRequest::SearchConcepts(query));
            }
            return;
        }
        if self.searching {
            return;
        }
        if let Some(hit) = self.hits.get(self.cursor) {
            ctx.emit(Action::Overlay(OverlayAction::Close));
            ctx.emit(Action::Reveal(target(hit)));
        }
    }

    /// The header: the scope and the decisions toggle.
    fn header(&self, projects: &[ProjectRef]) -> String {
        let scope = self.project.map_or_else(
            || ALL_PROJECTS.to_owned(),
            |id| {
                projects
                    .iter()
                    .find(|project| project.project_id == id)
                    .map_or_else(|| id.to_string(), |project| project.slug.clone())
            },
        );
        let decisions = if self.decisions { "on" } else { "off" };
        format!("scope {scope} · decisions {decisions}")
    }

    /// The search's status row, first match wins (§5.5).
    fn status(&self, scope: &Scope, theme: &Theme, keys: &Keys) -> (String, Style) {
        if let Some(notice) = self.notice {
            return (notice.to_owned(), theme.dim);
        }
        if self.error.is_some() {
            // MOD-160 D7: the sentence, the cause one key away (the why row).
            let why = keys.label(views::CONCEPTS_QUERY, Act::ConceptsWhy);
            return (unavailable_status(why.as_deref()), theme.error);
        }
        if self.searching {
            let text = if self.answered { SEARCHING } else { LOADING };
            return (text.to_owned(), theme.dim);
        }
        let submit = keys.label(views::CONCEPTS_QUERY, Act::FieldSubmit);
        if self.sent.is_none() {
            return (idle(submit.as_deref()), theme.dim);
        }
        let mut text = if self.hits.is_empty() {
            no_matches(
                keys.label(views::CONCEPTS_QUERY, Act::ConceptsReindex)
                    .as_deref(),
            )
        } else {
            plural(self.hits.len(), "hit", "hits")
        };
        if self.changed(scope) {
            text.push_str(" · ");
            text.push_str(&changed(submit.as_deref()));
        }
        (text, theme.dim)
    }

    /// The `Ctrl+R` row.
    fn index_line(&self, theme: &Theme) -> (String, Style) {
        match &self.index {
            IndexLine::Idle => (String::new(), theme.dim),
            IndexLine::Running => (INDEXING.to_owned(), theme.dim),
            IndexLine::Done(line) => (line.clone(), theme.dim),
            IndexLine::Failed(message) => (with_place(message), theme.error),
        }
    }
}

impl Overlay for ConceptsSearch {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        TITLE
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::CONCEPTS_QUERY)
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Nothing on open: the first search loads a model, so only `Enter` asks (D234).
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // The field sees the key first (MOD-67 D13): printable chords, the editing keys, `Enter`
        // and `Esc` are its own. It passes every CONTROL/ALT chord and the rest.
        match self.field.on_key(key) {
            FieldOutcome::Consumed => {
                self.notice = None;
                return Handled::Consumed;
            }
            FieldOutcome::Submit => {
                self.enter(ctx);
                return Handled::Consumed;
            }
            // `Esc`: the shell's `overlay.close` closes the box.
            FieldOutcome::Cancel => return Handled::Pass,
            FieldOutcome::Pass => {}
        }
        // Only the first candidate counts: an own act acts; `overlay.close` and `global.help`
        // (`F1`) are the shell's, `Ctrl+C` too (D261); `Tab` and the rest are swallowed by
        // `is_modal`.
        // The chord as pressed, before the shell's `[field]` translation (MOD-115 M4 MD-A).
        let chord = ctx.pressed(&key);
        match ctx.keys().actions(views::CONCEPTS_QUERY, chord).first() {
            Some(Act::ConceptsDecisions) => self.decisions = !self.decisions,
            Some(Act::ConceptsProject) => self.cycle_project(ctx.projects),
            Some(Act::ConceptsReindex) => self.reindex(ctx),
            Some(Act::ConceptsUp) => self.cursor = self.cursor.saturating_sub(1),
            Some(Act::ConceptsDown) => {
                if self.cursor + 1 < self.hits.len() {
                    self.cursor += 1;
                }
            }
            // MOD-160 D7: the why row opens and closes only while there is a cause.
            Some(Act::ConceptsWhy) => {
                if self.error.is_some() {
                    self.why = !self.why;
                }
            }
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// MOD-22 review M-1: a bracketed paste is query text, as typing it would be.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        self.field.on_paste(text);
        self.notice = None;
        Handled::Consumed
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        let StoreReply::Concepts(reply) = reply else {
            return;
        };
        match &**reply {
            ConceptsReply::Hits { query, outcome } => {
                // A list answers the search it echoes; any other is an older one's.
                if self.sent.as_ref() != Some(query) {
                    return;
                }
                self.searching = false;
                self.answered = true;
                match outcome {
                    Ok(hits) => {
                        self.hits.clone_from(hits);
                        self.cursor = 0;
                        self.error = None;
                        self.why = false;
                    }
                    Err(message) => {
                        self.hits.clear();
                        self.error = Some(message.clone());
                        // So `Enter` retries the same search.
                        self.sent = None;
                    }
                }
            }
            ConceptsReply::Indexed(Ok(report)) => {
                self.index = IndexLine::Done(index_report(report));
            }
            ConceptsReply::Indexed(Err(message)) => {
                self.index = IndexLine::Failed(message.clone());
            }
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let content_w = MAX_WIDTH - CHROME;
        let spec = if self.error.is_some() { HINT_WHY } else { HINT };
        let hint = ctx.keys().hint(views::CONCEPTS_QUERY, spec);
        // `Fit` keeps the margin round the box, so the header row stays uncovered (MOD-82); on a
        // screen too short for every hit row, the hit rows give way.
        let content = Modal::new(TITLE)
            .size(ModalSize::Fit {
                content_w,
                content_h: OTHER_ROWS + HIT_ROWS,
            })
            .hint(&hint)
            .render(frame, area, theme);
        let hit_rows = content.height.saturating_sub(OTHER_ROWS);
        let row_at = |at: u16, height: u16| Rect {
            y: content.y.saturating_add(at).min(content.bottom()),
            height: height.min(content.height.saturating_sub(at)),
            ..content
        };
        let (header, query) = (row_at(0, 1), row_at(1, 1));
        let hits = row_at(3, hit_rows);
        let (status, index) = (row_at(3 + hit_rows, 1), row_at(4 + hit_rows, 1));
        let row = |text: &str, style: Style| {
            Paragraph::new(list::ellipsize(
                Line::styled(text.to_owned(), style),
                content.width,
            ))
        };

        frame.render_widget(row(&self.header(ctx.projects), theme.dim), header);

        let label = u16::try_from(cell_width(QUERY_LABEL)).unwrap_or(u16::MAX);
        let mut spans = vec![Span::styled(QUERY_LABEL, theme.dim)];
        spans.extend(
            self.field
                .line(content.width.saturating_sub(label), true, theme)
                .spans,
        );
        frame.render_widget(Paragraph::new(Line::from(spans)), query);

        let (status_text, status_style) = self.status(ctx.scope, theme, ctx.keys());
        let (mut index_text, index_style) = self.index_line(theme);
        if self.hits.is_empty() {
            let cells = usize::from(content.width);
            let mut lines: Vec<Line<'static>> = Vec::new();
            // MOD-160 D7: the why row, first in the empty hit rows: the search's cause with where
            // it is fixed, whole, wrapped at grapheme boundaries (a failed search clears the hits).
            if let Some(error) = self.error.as_deref().filter(|_| self.why) {
                lines.extend(
                    wrap(&with_place(error), cells)
                        .into_iter()
                        .map(|wrapped| Line::styled(wrapped, theme.error)),
                );
            }
            // Review 8: an index run's error too wide for its row is drawn whole in the empty hit
            // rows, after the why row, and its own row is left blank rather than repeating the
            // start of it. The search's status row is the short sentence, never too wide.
            if matches!(self.index, IndexLine::Failed(_)) && cell_width(&index_text) > cells {
                lines.extend(
                    wrap(&index_text, cells)
                        .into_iter()
                        .map(|wrapped| Line::styled(wrapped, index_style)),
                );
                index_text.clear();
            }
            frame.render_widget(Paragraph::new(lines), hits);
        } else {
            let best = best_score(&self.hits);
            let rows: Vec<Row<'static>> = self
                .hits
                .iter()
                .map(|hit| Row {
                    line: Line::styled(hit_row(hit, best), theme.base),
                    kind: RowKind::Item,
                })
                .collect();
            let offset =
                list::scroll_into_view(self.cursor, 0, usize::from(hits.height), rows.len());
            list::render(
                frame,
                hits,
                &rows,
                Some(self.cursor),
                offset,
                theme.accent,
                theme,
            );
        }

        frame.render_widget(row(&status_text, status_style), status);
        frame.render_widget(row(&index_text, index_style), index);
    }
}

/// What `Enter` on `hit` reveals: its item (a document hit's owner item) or its requirement, with
/// the key the tab's miss sentence names (D235, blueprint D248).
fn target(hit: &Hit) -> RevealTarget {
    match hit.owner {
        Owner::Item(id) => RevealTarget::Item {
            id,
            key: hit.key.clone(),
        },
        Owner::Requirement(id) => RevealTarget::Requirement {
            id,
            key: hit.key.clone(),
        },
    }
}

/// The best score among `hits`, what each hit's word is relative to.
fn best_score(hits: &[Hit]) -> f32 {
    hits.iter().map(|hit| hit.score).fold(0.0, f32::max)
}

/// A hit's score as a word relative to `best`, the top score of its search (MOD-115 M4 Q4): a
/// fused rank score is comparable only within one search, so the number itself says nothing on
/// screen. `best` within 0.1% of the top, then `strong` from 66% of it, `fair` from 33%, else
/// `weak`; a search whose top score is not positive scored every hit alike, so each is `best`.
#[must_use]
pub fn score_word(score: f32, best: f32) -> &'static str {
    if best <= 0.0 || score >= best * 0.999 {
        return "best";
    }
    let ratio = score / best;
    if ratio >= 0.66 {
        "strong"
    } else if ratio >= 0.33 {
        "fair"
    } else {
        "weak"
    }
}

/// One hit as the overlay lists it (MOD-115 M4 D-19): the key, where it matched (a closed item's
/// resolution and a withdrawn requirement named, MOD-50 D227), the score as a word against
/// `best` ([`score_word`]), then the snippet. The CLI's `concepts::format_hit` prints the number.
#[must_use]
pub fn hit_row(hit: &Hit, best: f32) -> String {
    let place = match (&hit.point_type, &hit.document) {
        (PointType::Document, Some((_, kind))) => format!("{kind} document"),
        (PointType::Requirement, _) => "requirement".to_owned(),
        _ => "item".to_owned(),
    };
    let place = match (hit.resolution, hit.state) {
        (Some(resolution), _) => format!("{place} ({})", label::resolution(resolution)),
        (None, Some(RequirementState::Withdrawn)) => format!("{place} (withdrawn)"),
        _ => place,
    };
    // The snippet is already stripped of control characters (`vector::snippet`); the key and the
    // document kind are stripped here, so nothing stored can drive the terminal it is drawn on.
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    format!(
        "{:<10} {:<26} {:<6} {}",
        clean(&hit.key),
        clean(&place),
        score_word(hit.score, best),
        hit.snippet
    )
}

/// What one index run did, as the overlay says it (MOD-115 M4 D-19): `concepts::report_line`'s
/// figures, each count with its noun through `kit::label::plural`.
#[must_use]
pub fn index_report(report: &SyncReport) -> String {
    format!(
        "indexed: {} rebuilt, {} unchanged; {} rebuilt, {} unchanged; {} written, {} removed",
        plural(report.items_rebuilt, "item", "items"),
        report.items_unchanged,
        plural(report.requirements_rebuilt, "requirement", "requirements"),
        report.requirements_unchanged,
        plural(report.points_upserted, "point", "points"),
        report.points_deleted
    )
}

/// `error` with where it is fixed (MOD-115 M4 E6, MOD-83), [`place`]'s, unless it already names a
/// Settings place (`set one in Settings > Qdrant`) or no place fixes it.
#[must_use]
pub fn with_place(error: &str) -> String {
    match place(error) {
        Some(place) => format!("{error} \u{2014} {place}"),
        None => error.to_owned(),
    }
}

/// Where `error` is fixed, by its origin (MOD-115 M4 RV1-M1): the index's errors come from the
/// embedding model, from the Postgres reads its sync makes, and from Qdrant with its keyring
/// settings, and only the last two have a Settings section.
///
/// - The embedding model (`htui_store::model`'s and `embed`'s `embedding model: …`, the query's
///   embedding): nothing in Settings fetches or loads it, so a failed download says
///   [`NETWORK_STEP`] and any other failure names no place.
/// - Qdrant (`qdrant: …`, the URL check), or the keyring that holds its URL and key:
///   [`QDRANT_PLACE`].
/// - The store (`store unreachable: …`, another `store backend error: …`): a read of the index
///   sync, so [`CONNECTION_PLACE`].
/// - Anything else came from the Qdrant connection (`concepts::qdrant_refusal`): [`QDRANT_PLACE`].
fn place(error: &str) -> Option<&'static str> {
    let lower = error.to_lowercase();
    if error.contains("Settings") {
        None
    } else if lower.contains("embedding model")
        || lower.contains("embedding failed")
        || lower.contains("embedding task failed")
        || error.contains(QUERY_EMBED_FAILED)
    {
        lower.contains("cannot download").then_some(NETWORK_STEP)
    } else if lower.contains("qdrant") || lower.contains("keyring") {
        Some(QDRANT_PLACE)
    } else if error.starts_with("store unreachable") || error.starts_with("store backend error") {
        Some(CONNECTION_PLACE)
    } else {
        Some(QDRANT_PLACE)
    }
}

/// `text` broken into rows of at most `cells` display cells, at grapheme boundaries (MOD-54's
/// measure): a wide glyph that would pass the edge starts the next row whole,
/// never split. A cluster wider than a whole row gets a row to itself rather than being lost.
fn wrap(text: &str, cells: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut used = 0;
    for grapheme in graphemes(text) {
        let width = cell_width(grapheme);
        if used + width > cells && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        used += width;
        row.push_str(grapheme);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Emit, OverlayAction, RevealTarget, TopBarState};
    use crate::concepts::{self, DECISION_RESOLUTIONS};
    use crate::concepts_worker::ConceptsReply;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crate::ui::kit::list::GUTTER;
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectRef, WorkspaceId};
    use htui_store::vector::{Owner, PointType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    /// Everything a `Ctx` borrows, and the `Emit` that makes "nothing was sent" an assertion.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keys: Keys,
        theme: Theme,
        emit: Emit,
    }

    impl Bench {
        /// A workspace holding `htui` and `agy`, in that order.
        fn platform() -> Self {
            let projects = vec![
                project(ids::PROJECT_HTUI, "htui", 0),
                project(ids::PROJECT_AGY, "agy", 1),
            ];
            Self::over(projects)
        }

        /// A workspace with no project.
        fn empty() -> Self {
            Self::over(Vec::new())
        }

        fn over(projects: Vec<ProjectRef>) -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: projects.iter().map(|p| p.project_id).collect(),
                },
                projects,
                top_bar: TopBarState::default(),
                keys: Keys::compiled().clone(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keys,
                &self.theme,
                Origin::Overlay(ConceptsSearch::ID),
                &self.emit,
            )
        }

        fn key(&self, search: &mut ConceptsSearch, key: KeyEvent) -> Handled {
            search.on_key(key, &mut self.ctx())
        }

        fn code(&self, search: &mut ConceptsSearch, code: KeyCode) -> Handled {
            self.key(search, KeyEvent::from(code))
        }

        fn ctrl(&self, search: &mut ConceptsSearch, c: char) -> Handled {
            self.key(
                search,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
            )
        }

        fn typed(&self, search: &mut ConceptsSearch, text: &str) {
            for c in text.chars() {
                self.code(search, KeyCode::Char(c));
            }
        }

        fn reply(&self, search: &mut ConceptsSearch, reply: ConceptsReply) {
            search.on_reply(&StoreReply::Concepts(Box::new(reply)), &mut self.ctx());
        }

        /// The overlay drawn over a `width`×`height` frame whose first row reads `HEADER`.
        fn buffer(&self, search: &ConceptsSearch, width: u16, height: u16) -> Buffer {
            let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
            term.draw(|frame| {
                for x in 0..width {
                    frame.buffer_mut()[(x, 0)].set_symbol("H");
                }
                search.render(frame, frame.area(), &self.ctx());
            })
            .expect("the overlay draws");
            term.backend().buffer().clone()
        }

        /// The overlay drawn over a blank 100×30 frame, one line per row.
        fn render(&self, search: &ConceptsSearch) -> String {
            let buffer = &self.buffer(search, 100, 30);
            let mut out = String::new();
            for y in 0..buffer.area.height {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    // A wide glyph's second cell is drawn by the glyph: skip it, so the line
                    // reads as the text it was drawn from.
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    x += u16::try_from(cell_width(symbol).max(1)).unwrap_or(1);
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out
        }
    }

    fn project(project_id: ProjectId, slug: &str, position: i32) -> ProjectRef {
        ProjectRef {
            project_id,
            slug: slug.to_owned(),
            name: slug.to_owned(),
            position,
        }
    }

    fn item_hit() -> Hit {
        Hit {
            point_type: PointType::Item,
            owner: Owner::Item(ids::HTUI_FEAT_1),
            key: "FEAT-1".to_owned(),
            document: None,
            resolution: None,
            state: None,
            score: 2.0,
            snippet: "FEAT-1 TUI scaffold".to_owned(),
        }
    }

    /// Every `SearchConcepts` the overlay emitted since the last call.
    fn searches(bench: &Bench) -> Vec<SearchQuery> {
        bench
            .emit
            .take()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(StoreRequest::SearchConcepts(query)) => Some(query),
                _ => None,
            })
            .collect()
    }

    /// Types `text`, presses `Enter` and answers the one search it sent with `hits`.
    fn searched(bench: &Bench, search: &mut ConceptsSearch, text: &str, hits: Vec<Hit>) {
        bench.typed(search, text);
        bench.code(search, KeyCode::Enter);
        let sent = searches(bench);
        assert_eq!(sent.len(), 1, "one search per Enter: {sent:?}");
        bench.reply(
            search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Ok(hits),
            },
        );
    }

    /// Types `text`, presses `Enter` and fails the one search it sent with `error`; the search
    /// sent.
    fn failed(
        bench: &Bench,
        search: &mut ConceptsSearch,
        text: &str,
        error: &str,
    ) -> Vec<SearchQuery> {
        bench.typed(search, text);
        bench.code(search, KeyCode::Enter);
        let sent = searches(bench);
        assert_eq!(sent.len(), 1, "one search per Enter: {sent:?}");
        bench.reply(
            search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Err(error.to_owned()),
            },
        );
        sent
    }

    /// The row of `frame` that draws `text`.
    fn row_of(frame: &str, text: &str) -> Option<usize> {
        frame.lines().position(|line| line.contains(text))
    }

    /// The box's bottom border, where its keys ride.
    fn border(frame: &str) -> &str {
        frame
            .lines()
            .find(|line| line.contains('\u{2570}'))
            .expect("a rounded box")
    }

    #[test]
    fn an_empty_scope_sends_nothing_and_says_so() {
        let bench = Bench::empty();
        let mut search = ConceptsSearch::new();
        bench.typed(&mut search, "x");
        assert_eq!(bench.code(&mut search, KeyCode::Enter), Handled::Consumed);
        assert!(bench.emit.is_empty(), "no search without a project");
        assert!(bench.render(&search).contains(NO_PROJECTS));

        let mut search = ConceptsSearch::new();
        assert_eq!(bench.ctrl(&mut search, 'r'), Handled::Consumed);
        assert!(bench.emit.is_empty(), "no index run without a project");
        assert!(bench.render(&search).contains(NO_PROJECTS));
    }

    #[test]
    fn an_empty_query_sends_nothing_and_says_so() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.typed(&mut search, "   ");
        bench.code(&mut search, KeyCode::Enter);
        assert!(bench.emit.is_empty());
        assert!(bench.render(&search).contains(EMPTY_QUERY));
    }

    #[test]
    fn enter_searches_once_then_opens() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.typed(&mut search, "tui scaffold");
        bench.code(&mut search, KeyCode::Enter);
        let sent = searches(&bench);
        assert_eq!(
            sent,
            vec![concepts::query(
                "tui scaffold",
                vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
                false,
                concepts::DEFAULT_LIMIT
            )]
        );
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Ok(vec![item_hit()]),
            },
        );

        bench.code(&mut search, KeyCode::Enter);
        let emitted = bench.emit.take();
        assert_eq!(emitted.len(), 2, "{emitted:?}");
        assert!(matches!(emitted[0], Action::Overlay(OverlayAction::Close)));
        match &emitted[1] {
            Action::Reveal(target) => assert_eq!(
                *target,
                RevealTarget::Item {
                    id: ids::HTUI_FEAT_1,
                    key: "FEAT-1".to_owned()
                }
            ),
            other => panic!("expected a reveal, got {other:?}"),
        }
    }

    #[test]
    fn a_requirement_hit_reveals_a_requirement_and_a_document_hit_its_owner_item() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        let requirement = Hit {
            point_type: PointType::Requirement,
            owner: Owner::Requirement(ids::REQ_STO_1),
            key: "R-STO-1".to_owned(),
            ..item_hit()
        };
        let document = Hit {
            point_type: PointType::Document,
            owner: Owner::Item(ids::HTUI_ANA_1),
            key: "ANA-1".to_owned(),
            ..item_hit()
        };
        searched(&bench, &mut search, "postgres", vec![requirement, document]);

        bench.code(&mut search, KeyCode::Enter);
        let emitted = bench.emit.take();
        assert!(matches!(
            &emitted[1],
            Action::Reveal(RevealTarget::Requirement { id, key })
                if *id == ids::REQ_STO_1 && key == "R-STO-1"
        ));

        // A fresh overlay, since the first one asked to be closed.
        let mut search = ConceptsSearch::new();
        let document = Hit {
            point_type: PointType::Document,
            owner: Owner::Item(ids::HTUI_ANA_1),
            key: "ANA-1".to_owned(),
            ..item_hit()
        };
        searched(&bench, &mut search, "postgres", vec![item_hit(), document]);
        bench.code(&mut search, KeyCode::Down);
        bench.code(&mut search, KeyCode::Enter);
        let emitted = bench.emit.take();
        assert!(matches!(
            &emitted[1],
            Action::Reveal(RevealTarget::Item { id, key })
                if *id == ids::HTUI_ANA_1 && key == "ANA-1"
        ));
    }

    #[test]
    fn up_and_down_move_the_cursor_within_the_hits() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        searched(&bench, &mut search, "tui", vec![item_hit(), item_hit()]);
        bench.code(&mut search, KeyCode::Up);
        assert_eq!(search.cursor, 0, "clamped at the top");
        bench.code(&mut search, KeyCode::Down);
        bench.code(&mut search, KeyCode::Down);
        assert_eq!(search.cursor, 1, "clamped at the bottom");
        let frame = bench.render(&search);
        assert!(
            frame.contains(&format!("{GUTTER}{}", hit_row(&item_hit(), 2.0))),
            "the cursor row leads with the kit's gutter: {frame}"
        );
    }

    #[test]
    fn a_toggle_after_hits_makes_enter_search_again() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        searched(&bench, &mut search, "tui", vec![item_hit()]);
        let stale = changed(Some("Enter"));
        assert_eq!(stale, "changed \u{2014} Enter searches again");
        assert!(!bench.render(&search).contains(&stale));

        assert_eq!(bench.ctrl(&mut search, 'd'), Handled::Consumed);
        assert!(bench.render(&search).contains(&stale), "the list is stale");
        bench.code(&mut search, KeyCode::Enter);
        let sent = searches(&bench);
        assert_eq!(sent.len(), 1, "Enter searched rather than opened");
        assert_eq!(sent[0].resolutions, DECISION_RESOLUTIONS.to_vec());
    }

    #[test]
    fn ctrl_p_cycles_all_then_each_project_then_all() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.typed(&mut search, "tui");
        let expected = [
            (ALL_PROJECTS, vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]),
            ("htui", vec![ids::PROJECT_HTUI]),
            ("agy", vec![ids::PROJECT_AGY]),
            (ALL_PROJECTS, vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]),
        ];
        for (index, (header, projects)) in expected.into_iter().enumerate() {
            if index > 0 {
                assert_eq!(bench.ctrl(&mut search, 'p'), Handled::Consumed);
            }
            let frame = bench.render(&search);
            assert!(
                frame.contains(&format!("scope {header} · decisions off")),
                "{frame}"
            );
            // Every turn differs from the search sent last, so `Enter` searches.
            bench.code(&mut search, KeyCode::Enter);
            let sent = searches(&bench);
            assert_eq!(sent.len(), 1, "{header}");
            assert_eq!(sent[0].projects, projects, "{header}");
        }
    }

    /// MOD-67 M3 (PA-6): a kitty-protocol terminal reports ctrl-shift-d as `D`+CONTROL; the
    /// chord folds to `ctrl-d`, so it still toggles. `Shift+Down` is not `concepts.down`
    /// (accepted, L-E §1): it passes, and the modal box swallows it.
    #[test]
    fn a_ctrl_capital_toggles_and_shift_down_does_not_move() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        assert_eq!(bench.ctrl(&mut search, 'D'), Handled::Consumed);
        assert!(search.decisions);
        search.hits = vec![item_hit(), item_hit()];
        let shift_down = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(bench.key(&mut search, shift_down), Handled::Pass);
        assert_eq!(search.cursor, 0);
        assert_eq!(bench.code(&mut search, KeyCode::Down), Handled::Consumed);
        assert_eq!(search.cursor, 1);
    }

    #[test]
    fn a_project_gone_from_the_workspace_falls_back_to_all() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch {
            project: Some(ProjectId::default()),
            ..ConceptsSearch::new()
        };
        bench.ctrl(&mut search, 'p');
        assert_eq!(search.project, None);
    }

    #[test]
    fn a_reply_to_another_query_is_ignored() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.typed(&mut search, "tui");
        bench.code(&mut search, KeyCode::Enter);
        assert_eq!(searches(&bench).len(), 1);
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: concepts::query("other", vec![ids::PROJECT_HTUI], false, 10),
                outcome: Ok(vec![item_hit()]),
            },
        );
        assert!(search.hits.is_empty());
        assert!(search.searching, "still waiting for its own answer");
    }

    #[test]
    fn a_failed_search_lets_enter_retry() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        let sent = failed(&bench, &mut search, "tui", "cannot reach Qdrant: refused");
        let frame = bench.render(&search);
        assert!(
            frame.contains(&unavailable_status(Some("Ctrl+e"))),
            "{frame}"
        );
        bench.ctrl(&mut search, 'e');
        assert!(
            bench
                .render(&search)
                .contains("cannot reach Qdrant: refused")
        );

        bench.code(&mut search, KeyCode::Enter);
        assert_eq!(searches(&bench), sent, "the same search, again");
    }

    /// MOD-160 D7: a failed search says the one sentence on its status row, in the error style,
    /// and names the why key in force; the cause waits behind it.
    #[test]
    fn a_failed_search_says_unavailable_and_names_the_why_key() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        failed(
            &bench,
            &mut search,
            "tui",
            "qdrant: query: connection refused",
        );
        assert_eq!(
            unavailable_status(Some("Ctrl+e")),
            "knowledge search unavailable \u{2014} Ctrl+e why"
        );
        let frame = bench.render(&search);
        assert!(
            frame.contains("knowledge search unavailable \u{2014} Ctrl+e why"),
            "{frame}"
        );
        assert!(
            !frame.contains("connection refused"),
            "the cause waits behind the key: {frame}"
        );
        let (text, style) = search.status(&bench.scope, &bench.theme, &bench.keys);
        assert_eq!(text, unavailable_status(Some("Ctrl+e")));
        assert_eq!(style, bench.theme.error);
    }

    /// MOD-160 D7: the why key opens the cause, with where it is fixed, on the first hit row in
    /// the error style, and closes it again; it types nothing.
    #[test]
    fn the_why_key_toggles_the_cause_with_its_place() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        failed(
            &bench,
            &mut search,
            "tui",
            "qdrant: query: connection refused",
        );
        assert_eq!(bench.ctrl(&mut search, 'e'), Handled::Consumed);
        let cause = "qdrant: query: connection refused \u{2014} Settings \u{203a} Qdrant";
        let frame = bench.render(&search);
        let at = row_of(&frame, cause).unwrap_or_else(|| panic!("the cause:\n{frame}"));
        let query = row_of(&frame, "query tui").expect("the query row");
        assert_eq!(at, query + 2, "the first hit row:\n{frame}");
        assert!(
            frame.contains(&unavailable_status(Some("Ctrl+e"))),
            "the status row keeps the sentence: {frame}"
        );
        assert_eq!(search.field.text(), Some("tui"), "the key typed nothing");
        let buffer = bench.buffer(&search, 100, 30);
        let y = u16::try_from(at).expect("a short frame");
        let x = (0..buffer.area.width)
            .find(|x| buffer[(*x, y)].symbol() == "q")
            .expect("the cause's first cell");
        assert_eq!(buffer[(x, y)].fg, bench.theme.error.fg.expect("a colour"));

        assert_eq!(bench.ctrl(&mut search, 'e'), Handled::Consumed);
        let frame = bench.render(&search);
        assert!(
            !frame.contains("connection refused"),
            "closed again: {frame}"
        );
        assert!(
            frame.contains(&unavailable_status(Some("Ctrl+e"))),
            "{frame}"
        );
    }

    /// MOD-160 D7: with no cause the why key is consumed and changes nothing, before a search and
    /// over hits.
    #[test]
    fn the_why_key_without_a_cause_changes_nothing() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.typed(&mut search, "tui");
        let before = bench.render(&search);
        assert_eq!(bench.ctrl(&mut search, 'e'), Handled::Consumed);
        assert!(!search.why);
        assert_eq!(bench.render(&search), before);
        assert!(bench.emit.is_empty());

        let mut search = ConceptsSearch::new();
        searched(&bench, &mut search, "tui", vec![item_hit()]);
        let before = bench.render(&search);
        assert_eq!(bench.ctrl(&mut search, 'e'), Handled::Consumed);
        assert!(!search.why);
        assert_eq!(bench.render(&search), before);
    }

    /// MOD-160 D7: `Enter` that sends closes the why row, so a retry that fails again starts
    /// closed; a search that answers closes it too.
    #[test]
    fn a_new_search_closes_the_why_row() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        failed(&bench, &mut search, "tui", "qdrant: refused");
        bench.ctrl(&mut search, 'e');
        assert!(search.why);
        bench.code(&mut search, KeyCode::Enter);
        assert!(!search.why, "a retry closes it");
        let sent = searches(&bench);
        assert_eq!(sent.len(), 1);
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Err("qdrant: refused".to_owned()),
            },
        );
        let frame = bench.render(&search);
        assert!(!frame.contains("qdrant: refused"), "{frame}");
        assert!(
            frame.contains(&unavailable_status(Some("Ctrl+e"))),
            "{frame}"
        );

        bench.code(&mut search, KeyCode::Enter);
        let sent = searches(&bench);
        search.why = true;
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Ok(vec![item_hit()]),
            },
        );
        assert!(!search.why, "an answer closes it");
    }

    /// MOD-160 D7, R-3: the border names the why key only while there is a cause, and the row
    /// still fits a 100-column frame whole, `Esc close` last.
    #[test]
    fn the_border_names_why_only_while_there_is_a_cause() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        let frame = bench.render(&search);
        assert!(!border(&frame).contains("why"), "{frame}");

        failed(&bench, &mut search, "tui", "qdrant: refused");
        let frame = bench.render(&search);
        let row = border(&frame);
        assert!(row.contains("Ctrl+e why \u{b7} Esc close"), "{row}");
        assert!(!row.contains('\u{2026}'), "not cut: {row}");

        bench.code(&mut search, KeyCode::Enter);
        let sent = searches(&bench);
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Ok(vec![item_hit()]),
            },
        );
        let frame = bench.render(&search);
        let row = border(&frame);
        assert!(!row.contains("why"), "{row}");
        assert!(row.contains("\u{2191}/\u{2193} move"), "{row}");
    }

    /// MOD-160 D7: with the why key unbound the status row is the sentence alone, the border
    /// does not name it, and no chord opens the cause.
    #[test]
    fn an_unbound_why_key_leaves_the_sentence_alone() {
        let mut bench = Bench::platform();
        bench.keys =
            crate::keys::load_str("version = 1\n[concepts]\nwhy = []\n").expect("the keys load");
        let mut search = ConceptsSearch::new();
        failed(&bench, &mut search, "tui", "qdrant: refused");
        assert_eq!(unavailable_status(None), KNOWLEDGE_SEARCH_UNAVAILABLE);
        let frame = bench.render(&search);
        let status = frame
            .lines()
            .find(|line| line.contains(KNOWLEDGE_SEARCH_UNAVAILABLE))
            .unwrap_or_else(|| panic!("the sentence:\n{frame}"));
        assert!(!status.contains('\u{2014}'), "{status}");
        assert!(!border(&frame).contains("why"), "{frame}");
        bench.ctrl(&mut search, 'e');
        assert!(!bench.render(&search).contains("qdrant: refused"));
    }

    /// MOD-160 D1: an empty index is not unavailable: it still says no matches.
    #[test]
    fn an_empty_index_still_says_no_matches() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        searched(&bench, &mut search, "tui", Vec::new());
        let frame = bench.render(&search);
        assert!(frame.contains(&no_matches(Some("Ctrl+r"))), "{frame}");
        assert!(!frame.contains(KNOWLEDGE_SEARCH_UNAVAILABLE), "{frame}");
        assert!(!border(&frame).contains("why"), "{frame}");
    }

    #[test]
    fn the_first_search_says_it_loads_the_model_later_ones_say_searching() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        assert_eq!(idle(Some("Enter")), "type a query; Enter searches");
        assert_eq!(idle(None), "type a query");
        assert!(bench.render(&search).contains(&idle(Some("Enter"))));
        bench.typed(&mut search, "tui");
        bench.code(&mut search, KeyCode::Enter);
        assert!(bench.render(&search).contains(LOADING));
        let sent = searches(&bench);
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Ok(Vec::new()),
            },
        );
        assert!(bench.render(&search).contains(&no_matches(Some("Ctrl+r"))));
        assert_eq!(no_matches(None), "no matches");

        bench.typed(&mut search, " scaffold");
        bench.code(&mut search, KeyCode::Enter);
        let frame = bench.render(&search);
        assert!(frame.contains(SEARCHING), "{frame}");
        assert!(!frame.contains(LOADING), "{frame}");
    }

    #[test]
    fn ctrl_r_indexes_the_scope_and_prints_the_report() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.ctrl(&mut search, 'p');
        bench.ctrl(&mut search, 'r');
        let emitted = bench.emit.take();
        assert!(
            matches!(
                &emitted[..],
                [Action::Store(StoreRequest::IndexConcepts { scope })]
                    if scope.project_ids == vec![ids::PROJECT_HTUI]
                        && scope.workspace_id == bench.scope.workspace_id
            ),
            "{emitted:?}"
        );
        assert!(bench.render(&search).contains(INDEXING));

        let report = SyncReport {
            items_rebuilt: 3,
            ..Default::default()
        };
        bench.reply(&mut search, ConceptsReply::Indexed(Ok(report)));
        // Wider than the box, which cuts it like every other row.
        let line: String = index_report(&report).chars().take(92).collect();
        assert!(bench.render(&search).contains(&line), "{line}");

        bench.reply(
            &mut search,
            ConceptsReply::Indexed(Err("qdrant: upsert: refused".to_owned())),
        );
        assert!(bench.render(&search).contains("qdrant: upsert: refused"));
    }

    #[test]
    fn other_chords_and_esc_pass_and_letters_are_text() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        assert_eq!(bench.ctrl(&mut search, 'c'), Handled::Pass);
        assert_eq!(bench.code(&mut search, KeyCode::Esc), Handled::Pass);
        for c in ['w', 'q', '1', '?', 'j', 'k'] {
            assert_eq!(bench.code(&mut search, KeyCode::Char(c)), Handled::Consumed);
        }
        assert_eq!(search.field.text(), Some("wq1?jk"));
        assert!(bench.emit.is_empty());
    }

    #[test]
    fn a_long_hit_is_clipped_to_the_box() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        let long = Hit {
            snippet: "一".repeat(80),
            ..item_hit()
        };
        searched(&bench, &mut search, "tui", vec![long.clone()]);
        let frame = bench.render(&search);
        let expected = list::ellipsize(Line::raw(hit_row(&long, 2.0)), 92).to_string();
        assert!(frame.contains(&expected), "{frame}");
        assert!(
            expected.ends_with('\u{2026}'),
            "the cut is marked: {expected}"
        );
        // 45 cells of key, place and score word, then 23 wide glyphs and the ellipsis: a 24th
        // glyph would pass the 91st cell, so it is left out whole, never split.
        assert_eq!(cell_width(&expected), 92, "{expected}");
    }

    /// Review 8: with no hits to draw, an error wider than its row is drawn whole, wrapped across
    /// the hit rows at grapheme boundaries (a wide glyph that would pass the edge starts the next
    /// row), and its own row is left blank rather than repeating the start of it.
    #[test]
    fn a_long_error_wraps_across_the_empty_hit_rows() {
        let bench = Bench::platform();
        // 21 cells, then 60 wide glyphs: 36 of them fill the first row to 93 of its 94 cells.
        let long = format!("cannot reach Qdrant: {} end", "一".repeat(60));
        let rows = [
            format!("cannot reach Qdrant: {}", "一".repeat(36)),
            format!("{} end", "一".repeat(24)),
        ];

        let mut search = ConceptsSearch::new();
        failed(&bench, &mut search, "tui", &long);
        let frame = bench.render(&search);
        assert_eq!(
            frame.matches("cannot reach Qdrant").count(),
            0,
            "behind the why key: {frame}"
        );
        // MOD-160 D7: the why row draws the cause with its place, wrapped the same way.
        bench.ctrl(&mut search, 'e');
        let frame = bench.render(&search);
        let lines: Vec<&str> = frame.lines().collect();
        let first = lines
            .iter()
            .position(|line| line.contains(&rows[0]))
            .unwrap_or_else(|| panic!("the error's first row:\n{frame}"));
        assert!(lines[first + 1].contains(&rows[1]), "{frame}");
        assert!(
            lines[first + 1].contains("end \u{2014} Settings \u{203a} Qdrant"),
            "{frame}"
        );
        assert_eq!(
            frame.matches("cannot reach Qdrant").count(),
            1,
            "drawn once: {frame}"
        );
        assert!(
            frame.contains(&unavailable_status(Some("Ctrl+e"))),
            "{frame}"
        );

        let mut search = ConceptsSearch::new();
        bench.ctrl(&mut search, 'r');
        let _ = bench.emit.take();
        bench.reply(&mut search, ConceptsReply::Indexed(Err(long.clone())));
        let frame = bench.render(&search);
        let lines: Vec<&str> = frame.lines().collect();
        let first = lines
            .iter()
            .position(|line| line.contains(&rows[0]))
            .unwrap_or_else(|| panic!("the index run's error, first row:\n{frame}"));
        assert!(lines[first + 1].contains(&rows[1]), "{frame}");
        assert_eq!(frame.matches("cannot reach Qdrant").count(), 1, "{frame}");
    }

    #[test]
    fn a_short_error_stays_on_its_row() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.ctrl(&mut search, 'r');
        bench.reply(
            &mut search,
            ConceptsReply::Indexed(Err("qdrant: upsert: refused".to_owned())),
        );
        let frame = bench.render(&search);
        let at = frame
            .lines()
            .position(|line| line.contains("qdrant: upsert: refused"))
            .expect("drawn");
        let hint = frame
            .lines()
            .position(|line| line.contains("Esc close"))
            .expect("the hint");
        assert_eq!(at + 1, hint, "the index row, just above the hint:\n{frame}");

        // MOD-160 D7: a search's short cause opens on the first hit row; the sentence keeps the
        // status row, just above the index row.
        let mut search = ConceptsSearch::new();
        failed(&bench, &mut search, "tui", "qdrant: query: refused");
        bench.ctrl(&mut search, 'e');
        let frame = bench.render(&search);
        let cause = row_of(&frame, "qdrant: query: refused").expect("drawn");
        let query = row_of(&frame, "query tui").expect("the query row");
        assert_eq!(cause, query + 2, "the first hit row:\n{frame}");
        let status = row_of(&frame, KNOWLEDGE_SEARCH_UNAVAILABLE).expect("the sentence");
        let hint = row_of(&frame, "Esc close").expect("the hint");
        assert_eq!(status + 2, hint, "the status row:\n{frame}");
    }

    #[test]
    fn wrap_breaks_at_grapheme_boundaries_and_never_splits_a_wide_glyph() {
        assert_eq!(wrap("abcde", 2), ["ab", "cd", "e"]);
        assert_eq!(wrap("a一b", 2), ["a", "一", "b"]);
        assert_eq!(
            wrap("e\u{301}x", 1),
            ["e\u{301}", "x"],
            "a cluster stays whole"
        );
        assert!(wrap("", 4).is_empty());
        assert_eq!(
            wrap("一", 1),
            ["一"],
            "too wide for any row: alone, not lost"
        );
    }

    #[test]
    fn hit_rows_is_the_cli_default_limit() {
        assert_eq!(u64::from(HIT_ROWS), concepts::DEFAULT_LIMIT);
    }

    /// MOD-115 M4 E6 (Q4): a score reads as a word relative to the best hit of its search:
    /// `best` within 0.1% of it, then `strong` from 0.66, `fair` from 0.33, else `weak`.
    #[test]
    fn a_score_reads_as_a_word_relative_to_the_best_hit() {
        assert_eq!(score_word(3.0, 3.0), "best");
        assert_eq!(score_word(2.998, 3.0), "best");
        assert_eq!(score_word(2.0, 3.0), "strong");
        assert_eq!(score_word(0.66 * 3.0, 3.0), "strong");
        assert_eq!(score_word(1.0, 3.0), "fair");
        assert_eq!(score_word(0.33 * 3.0, 3.0), "fair");
        assert_eq!(score_word(0.9, 3.0), "weak");
        assert_eq!(score_word(0.0, 0.0), "best", "a search scored all alike");
    }

    /// MOD-115 M4 E6 (MOD-83, D-19): the overlay builds its own rows: no raw score and no `(s)`
    /// on screen, the CLI's `format_hit` left as it prints.
    #[test]
    fn hits_read_their_scores_in_words_and_count_in_words() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        let scored = |key: &str, score: f32| Hit {
            key: key.to_owned(),
            score,
            ..item_hit()
        };
        searched(
            &bench,
            &mut search,
            "tui",
            vec![
                scored("A-1", 4.0),
                scored("B-2", 3.0),
                scored("C-3", 2.0),
                scored("D-4", 1.0),
            ],
        );
        let frame = bench.render(&search);
        for (key, word) in [
            ("A-1", "best"),
            ("B-2", "strong"),
            ("C-3", "fair"),
            ("D-4", "weak"),
        ] {
            let line = frame
                .lines()
                .find(|line| line.contains(&format!("{key} ")))
                .unwrap_or_else(|| panic!("{key}: {frame}"));
            assert!(line.contains(&format!(" {word} ")), "{key} {word}: {line}");
        }
        for raw in ["4.000", "3.000", "2.000", "1.000", "hit(s)"] {
            assert!(!frame.contains(raw), "`{raw}` on screen: {frame}");
        }
        assert!(frame.contains("4 hits"), "{frame}");
        assert!(
            concepts::format_hit(&item_hit()).contains("2.000"),
            "the CLI's is kept"
        );

        let mut search = ConceptsSearch::new();
        searched(&bench, &mut search, "tui", vec![item_hit()]);
        assert!(bench.render(&search).contains("1 hit"));
    }

    /// The row: key, where it matched (a closed item's resolution through `kit::label`), the
    /// score word, then the snippet.
    #[test]
    fn a_hit_row_names_where_it_matched() {
        let closed = Hit {
            resolution: Some(htui_core::model::Resolution::Rejected),
            ..item_hit()
        };
        assert_eq!(
            hit_row(&closed, 2.0),
            format!(
                "{:<10} {:<26} {:<6} {}",
                "FEAT-1", "item (rejected)", "best", "FEAT-1 TUI scaffold"
            )
        );
        let document = Hit {
            point_type: PointType::Document,
            document: Some((
                htui_core::model::DocumentId::default(),
                "verdict".to_owned(),
            )),
            score: 1.0,
            ..item_hit()
        };
        assert!(hit_row(&document, 2.0).starts_with("FEAT-1     verdict document"));
        assert!(hit_row(&document, 2.0).contains(" fair "));
    }

    /// MOD-115 M4 E6 (MOD-83): the index run's report counts in words.
    #[test]
    fn the_index_report_counts_in_words() {
        let report = SyncReport {
            items_rebuilt: 1,
            items_unchanged: 2,
            requirements_rebuilt: 3,
            requirements_unchanged: 4,
            points_upserted: 5,
            points_deleted: 6,
        };
        assert_eq!(
            index_report(&report),
            "indexed: 1 item rebuilt, 2 unchanged; 3 requirements rebuilt, 4 unchanged; \
             5 points written, 6 removed"
        );
    }

    /// MOD-115 M4 RV1-M1: the place is the error's origin's. Nothing in Settings fetches the
    /// embedding model, so a failed download says to check the network and a model that will not
    /// load names no place; a Postgres read the index sync made is fixed in Settings › Connection;
    /// Qdrant's own errors, the keyring's and the URL's stay on Settings › Qdrant.
    #[test]
    fn an_error_names_the_place_of_its_origin() {
        let download = "store backend error: embedding model: cannot download model.onnx from \
                        https://huggingface.co/x: dns error";
        assert_eq!(
            with_place(download),
            format!("{download} \u{2014} check the network")
        );
        let load = "store backend error: embedding model: cannot load /m/model.onnx: bad header";
        assert_eq!(with_place(load), load, "no place fixes a broken model file");
        assert_eq!(
            with_place("the embedding model's loader stopped"),
            "the embedding model's loader stopped"
        );
        let query = format!("store backend error: {QUERY_EMBED_FAILED}: the session failed");
        assert_eq!(
            with_place(&query),
            query,
            "the model's failure, not Qdrant's"
        );

        assert_eq!(
            with_place("store unreachable: connection refused"),
            "store unreachable: connection refused \u{2014} Settings \u{203a} Connection"
        );
        assert_eq!(
            with_place("store backend error: read.items: pool timed out"),
            "store backend error: read.items: pool timed out \u{2014} Settings \u{203a} Connection"
        );

        for qdrant in [
            "store backend error: qdrant: upsert: refused",
            "store backend error: Qdrant URL must start with http:// or https://",
            "store backend error: cannot read the keyring entry (qdrant_url): locked",
            "the keyring read stopped: cancelled",
        ] {
            assert_eq!(
                with_place(qdrant),
                format!("{qdrant} \u{2014} Settings \u{203a} Qdrant"),
                "{qdrant}"
            );
        }

        // MOD-160 D7: the why row draws the cause with its origin's place.
        let bench = Bench::platform();
        for (error, place) in [
            (
                "store unreachable: connection refused",
                "Settings \u{203a} Connection",
            ),
            (download, "check the network"),
            ("qdrant: query: refused", "Settings \u{203a} Qdrant"),
        ] {
            let mut search = ConceptsSearch::new();
            failed(&bench, &mut search, "tui", error);
            assert!(!bench.render(&search).contains(place), "{error}");
            bench.ctrl(&mut search, 'e');
            let frame = bench.render(&search);
            assert!(frame.contains(place), "{error}: {frame}");
        }
    }

    /// MOD-115 M4 E6 (MOD-83): an error names where it is fixed, once.
    #[test]
    fn an_error_names_where_to_fix_it() {
        assert_eq!(
            with_place("cannot reach Qdrant: refused"),
            "cannot reach Qdrant: refused \u{2014} Settings \u{203a} Qdrant"
        );
        let named = "no Qdrant URL stored; set one in Settings > Qdrant";
        assert_eq!(with_place(named), named, "already named");

        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        bench.typed(&mut search, "tui");
        bench.code(&mut search, KeyCode::Enter);
        let sent = searches(&bench);
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Err("cannot reach Qdrant: refused".to_owned()),
            },
        );
        bench.ctrl(&mut search, 'e');
        assert!(
            bench
                .render(&search)
                .contains("cannot reach Qdrant: refused \u{2014} Settings \u{203a} Qdrant")
        );
        bench.reply(
            &mut search,
            ConceptsReply::Indexed(Err("qdrant: upsert: refused".to_owned())),
        );
        assert!(
            bench
                .render(&search)
                .contains("qdrant: upsert: refused \u{2014} Settings \u{203a} Qdrant")
        );
    }

    /// MOD-115 M4 E6 (MD-A): the field's submit key is named through `Keys`, on the border and
    /// in the idle and stale lines, so a rebound `[field] submit` shows its own chord.
    #[test]
    fn a_rebound_submit_is_named_by_the_hint_and_the_lines() {
        let mut bench = Bench::platform();
        bench.keys = crate::keys::load_str("version = 1\n[field]\nsubmit = \"ctrl-t\"\n")
            .expect("the keys load");
        let search = ConceptsSearch::new();
        let frame = bench.render(&search);
        assert!(frame.contains("Ctrl+t search/open"), "{frame}");
        assert!(frame.contains(&idle(Some("Ctrl+t"))), "{frame}");
        assert!(!frame.contains("Enter"), "{frame}");
        assert_eq!(changed(None), "changed");
    }

    /// MOD-115 M4 E6 (MOD-85): the cursor hit is the kit's gutter and band, legible under
    /// `NO_COLOR`; the other hits have neither.
    #[test]
    fn the_cursor_hit_is_the_band_and_reads_without_colour() {
        let mut bench = Bench::platform();
        bench.theme = Theme::monochrome();
        let mut search = ConceptsSearch::new();
        let second = Hit {
            key: "FEAT-2".to_owned(),
            ..item_hit()
        };
        searched(&bench, &mut search, "tui", vec![item_hit(), second]);
        bench.code(&mut search, KeyCode::Down);
        let buffer = bench.buffer(&search, 100, 30);
        let at = |key: &str| {
            (0..buffer.area.height)
                .find_map(|y| {
                    let line: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol().to_owned())
                        .collect();
                    line.find(&format!("{key} ")).map(|byte| {
                        let x = u16::try_from(line[..byte].chars().count()).expect("narrow");
                        (x, y, line)
                    })
                })
                .unwrap_or_else(|| panic!("{key} is not drawn"))
        };
        let (x, y, line) = at("FEAT-2");
        assert!(line.contains(GUTTER), "{line}");
        assert!(buffer[(x, y)].modifier.contains(Modifier::REVERSED));
        let (x, y, line) = at("FEAT-1");
        assert!(!line.contains(GUTTER), "{line}");
        assert!(!buffer[(x, y)].modifier.contains(Modifier::REVERSED));
        for cell in &buffer.content {
            assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset));
        }
    }

    /// MOD-115 M4 E6 (MOD-82): the box keeps the margin, so the header row stays uncovered at 60
    /// columns, on a tall screen and on a short one; the keys ride its bottom border.
    #[test]
    fn the_box_leaves_the_header_row_and_puts_its_keys_on_the_border() {
        let bench = Bench::platform();
        let search = ConceptsSearch::new();
        for (width, height) in [(60, 20), (60, 16), (100, 30)] {
            let buffer = bench.buffer(&search, width, height);
            for x in 0..width {
                assert_eq!(buffer[(x, 0)].symbol(), "H", "{width}x{height}: ({x}, 0)");
            }
        }
        let frame = bench.render(&search);
        let bottom = frame
            .lines()
            .find(|line| line.contains('\u{2570}'))
            .expect("a rounded box");
        assert!(bottom.contains("Esc close"), "{frame}");
        assert!(
            frame.contains("\u{256d} Search concepts "),
            "the kit's modal: {frame}"
        );
    }
}
