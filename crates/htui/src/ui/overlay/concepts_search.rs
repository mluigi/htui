//! The concepts search (MOD-64 D233, D234, D236): a modal overlay over the concepts index.
//!
//! A query field, a project scope (`Ctrl+P`: every project of the workspace, then each one), a
//! decisions toggle (`Ctrl+D`) and the last hits, printed as `htui --search-items` prints them
//! (`concepts::format_hit`, D239). `Enter` searches when the query or a toggle changed since the
//! last search, and otherwise opens the highlighted hit through `Action::Reveal` (D234, D235).
//! `Ctrl+R` re-indexes the current scope (D237).
//!
//! Like every view it holds no store handle and no channel: the search and the index run leave
//! through `Ctx::request` as `StoreRequest::SearchConcepts` / `IndexConcepts`, which the worker
//! serves on tasks of its own (`R-NF-3`), and come back through `on_reply` as
//! `StoreReply::Concepts`, with the error inside (D232): a missing or unreachable Qdrant is a line
//! in this box and nothing else.

use crossterm::event::KeyEvent;
use htui_core::model::{ProjectId, ProjectRef, Scope};
use htui_store::vector::{Hit, Owner, SearchQuery};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{Action, Ctx, Handled, OverlayAction, RevealTarget};
use crate::concepts;
use crate::concepts_worker::ConceptsReply;
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::{cell_width, graphemes};
use crate::ui::layout::centered;
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crate::ui::{FieldOutcome, TextField, Theme};

/// The box's title.
const TITLE: &str = "Search concepts";

/// In front of the query field.
const QUERY_LABEL: &str = "query ";

/// Marker in front of the highlighted hit (the switcher's).
const CURSOR: &str = "> ";

/// The marker's width, in front of every other hit.
const NO_CURSOR: &str = "  ";

/// Rows of hits: the CLI's default limit, so the box shows every hit a search asks for.
const HIT_ROWS: u16 = 10;

/// Two borders and the sixteen rows below them.
const BOX_HEIGHT: u16 = 18;

/// Columns left free around the box.
const MARGIN: u16 = 4;

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

/// Before the first search.
pub const IDLE: &str = "type a query; Enter searches";

/// A workspace with no project: nothing is sent (D233).
pub const NO_PROJECTS: &str = "this workspace has no projects";

/// `Enter` on an empty query.
pub const EMPTY_QUERY: &str = "type a query first";

/// A search that found nothing.
pub const NO_MATCHES: &str = "no matches — Ctrl+R indexes this scope if the index is empty";

/// After the list, when the query or a toggle moved since it was searched.
pub const CHANGED: &str = "changed — Enter searches again";

/// The keys, under everything (MOD-67 D9). `Enter` is the field's own (D13); the short `scope`
/// and `index` labels keep the row inside the box at 100 columns (L-E Q5, maintainer 2026-10-08).
const HINT: HintSpec = &[
    Hint::Text("Enter search/open"),
    Hint::Pair(Act::ConceptsUp, Act::ConceptsDown, "move"),
    Hint::One(Act::ConceptsDecisions, "decisions"),
    Hint::One(Act::ConceptsProject, "scope"),
    Hint::One(Act::ConceptsReindex, "index"),
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
    fn status(&self, scope: &Scope, theme: &Theme) -> (String, Style) {
        if let Some(notice) = self.notice {
            return (notice.to_owned(), theme.dim);
        }
        if let Some(error) = &self.error {
            return (error.clone(), theme.error);
        }
        if self.searching {
            let text = if self.answered { SEARCHING } else { LOADING };
            return (text.to_owned(), theme.dim);
        }
        if self.sent.is_none() {
            return (IDLE.to_owned(), theme.dim);
        }
        let mut text = if self.hits.is_empty() {
            NO_MATCHES.to_owned()
        } else {
            format!("{} hit(s)", self.hits.len())
        };
        if self.changed(scope) {
            text.push_str(" · ");
            text.push_str(CHANGED);
        }
        (text, theme.dim)
    }

    /// The `Ctrl+R` row.
    fn index_line(&self, theme: &Theme) -> (String, Style) {
        match &self.index {
            IndexLine::Idle => (String::new(), theme.dim),
            IndexLine::Running => (INDEXING.to_owned(), theme.dim),
            IndexLine::Done(line) => (line.clone(), theme.dim),
            IndexLine::Failed(message) => (message.clone(), theme.error),
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
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::CONCEPTS_QUERY, chord) {
            match act {
                Act::ConceptsDecisions => {
                    self.decisions = !self.decisions;
                    return Handled::Consumed;
                }
                Act::ConceptsProject => {
                    self.cycle_project(ctx.projects);
                    return Handled::Consumed;
                }
                Act::ConceptsReindex => {
                    self.reindex(ctx);
                    return Handled::Consumed;
                }
                Act::ConceptsUp => {
                    self.cursor = self.cursor.saturating_sub(1);
                    return Handled::Consumed;
                }
                Act::ConceptsDown => {
                    if self.cursor + 1 < self.hits.len() {
                        self.cursor += 1;
                    }
                    return Handled::Consumed;
                }
                _ => break, // `overlay.close` or `global.help`: the shell's
            }
        }
        // `F1` and `Ctrl+C` are the shell's (D261); `Tab` and the rest are swallowed by
        // `is_modal`.
        Handled::Pass
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
                self.index = IndexLine::Done(concepts::report_line(report));
            }
            ConceptsReply::Indexed(Err(message)) => {
                self.index = IndexLine::Failed(message.clone());
            }
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let width = area.width.saturating_sub(MARGIN).min(MAX_WIDTH);
        let box_area = centered(area, width, BOX_HEIGHT);
        let block = Block::new()
            .borders(Borders::ALL)
            .title(Span::styled(format!(" {TITLE} "), theme.title));
        let inner = block.inner(box_area);
        frame.render_widget(Clear, box_area);
        frame.render_widget(block, box_area);

        let [header, query, _blank, hits, status, index, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(HIT_ROWS),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        let cells = usize::from(inner.width);
        let row = |text: &str, style: Style| Paragraph::new(Line::styled(clip(text, cells), style));

        frame.render_widget(row(&self.header(ctx.projects), theme.dim), header);

        let label = u16::try_from(cell_width(QUERY_LABEL)).unwrap_or(u16::MAX);
        let mut spans = vec![Span::styled(QUERY_LABEL, theme.dim)];
        spans.extend(
            self.field
                .line(inner.width.saturating_sub(label), true, theme)
                .spans,
        );
        frame.render_widget(Paragraph::new(Line::from(spans)), query);

        let hit_cells = cells.saturating_sub(cell_width(CURSOR));
        let mut lines: Vec<Line<'static>> = self
            .hits
            .iter()
            .enumerate()
            .map(|(at, hit)| {
                let selected = at == self.cursor;
                let marker = if selected { CURSOR } else { NO_CURSOR };
                let style = if selected { theme.accent } else { theme.base };
                Line::styled(
                    format!("{marker}{}", clip(&concepts::format_hit(hit), hit_cells)),
                    style,
                )
            })
            .collect();
        let (mut status_text, status_style) = self.status(ctx.scope, theme);
        let (mut index_text, index_style) = self.index_line(theme);
        if self.hits.is_empty() {
            // Review 8: an error too wide for its row is drawn whole in the empty hit rows, search
            // first, and its own row is left blank rather than repeating the start of it.
            let status_is_error = self.notice.is_none() && self.error.is_some();
            let index_is_error = matches!(self.index, IndexLine::Failed(_));
            for (is_error, text, style) in [
                (status_is_error, &mut status_text, status_style),
                (index_is_error, &mut index_text, index_style),
            ] {
                if is_error && cell_width(text) > cells {
                    lines.extend(
                        wrap(text, cells)
                            .into_iter()
                            .map(|wrapped| Line::styled(wrapped, style)),
                    );
                    text.clear();
                }
            }
        }
        frame.render_widget(Paragraph::new(lines), hits);

        frame.render_widget(row(&status_text, status_style), status);
        frame.render_widget(row(&index_text, index_style), index);
        let keys = ctx.keys().hint(views::CONCEPTS_QUERY, HINT);
        frame.render_widget(row(&keys, theme.dim), hint);
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

/// `text` cut to at most `cells` display cells, at a grapheme boundary (MOD-54's measure): a wide
/// glyph that would pass the edge is left out whole, never split.
fn clip(text: &str, cells: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for grapheme in graphemes(text) {
        let width = cell_width(grapheme);
        if used + width > cells {
            break;
        }
        used += width;
        out.push_str(grapheme);
    }
    out
}

/// `text` broken into rows of at most `cells` display cells, at grapheme boundaries (MOD-54's
/// measure, [`clip`]'s rule): a wide glyph that would pass the edge starts the next row whole,
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
    use crate::keymap::Keymap;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectRef, WorkspaceId};
    use htui_store::vector::{Owner, PointType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Everything a `Ctx` borrows, and the `Emit` that makes "nothing was sent" an assertion.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
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
                keymap: Keymap::new(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keymap,
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

        /// The overlay drawn over a blank 100×30 frame, one line per row.
        fn render(&self, search: &ConceptsSearch) -> String {
            let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
            term.draw(|frame| search.render(frame, frame.area(), &self.ctx()))
                .expect("the overlay draws");
            let buffer = term.backend().buffer();
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
            frame.contains(&format!("{CURSOR}{}", concepts::format_hit(&item_hit()))),
            "{frame}"
        );
    }

    #[test]
    fn a_toggle_after_hits_makes_enter_search_again() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        searched(&bench, &mut search, "tui", vec![item_hit()]);
        assert!(!bench.render(&search).contains(CHANGED));

        assert_eq!(bench.ctrl(&mut search, 'd'), Handled::Consumed);
        assert!(bench.render(&search).contains(CHANGED), "the list is stale");
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
        assert!(
            bench
                .render(&search)
                .contains("cannot reach Qdrant: refused")
        );

        bench.code(&mut search, KeyCode::Enter);
        assert_eq!(searches(&bench), sent, "the same search, again");
    }

    #[test]
    fn the_first_search_says_it_loads_the_model_later_ones_say_searching() {
        let bench = Bench::platform();
        let mut search = ConceptsSearch::new();
        assert!(bench.render(&search).contains(IDLE));
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
        assert!(bench.render(&search).contains(NO_MATCHES));

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

        let report = htui_store::vector_sync::SyncReport {
            items_rebuilt: 3,
            ..Default::default()
        };
        bench.reply(&mut search, ConceptsReply::Indexed(Ok(report)));
        // Wider than the box, which clips it like every other row.
        let line = clip(&concepts::report_line(&report), 94);
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
        let expected = clip(&concepts::format_hit(&long), 92);
        assert!(frame.contains(&expected), "{frame}");
        // 45 cells of key, place and score, then 23 wide glyphs: a 24th would pass the 92nd cell.
        assert_eq!(
            cell_width(&expected),
            91,
            "whole wide glyphs, none split at the edge"
        );
        assert_eq!(clip("abc", 2), "ab");
        assert_eq!(
            clip("a一b", 2),
            "a",
            "a wide glyph that would pass the edge stops the line"
        );
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
        bench.typed(&mut search, "tui");
        bench.code(&mut search, KeyCode::Enter);
        let sent = searches(&bench);
        bench.reply(
            &mut search,
            ConceptsReply::Hits {
                query: sent[0].clone(),
                outcome: Err(long.clone()),
            },
        );
        let frame = bench.render(&search);
        let lines: Vec<&str> = frame.lines().collect();
        let first = lines
            .iter()
            .position(|line| line.contains(&rows[0]))
            .unwrap_or_else(|| panic!("the error's first row:\n{frame}"));
        assert!(lines[first + 1].contains(&rows[1]), "{frame}");
        assert_eq!(
            frame.matches("cannot reach Qdrant").count(),
            1,
            "drawn once: {frame}"
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
}
