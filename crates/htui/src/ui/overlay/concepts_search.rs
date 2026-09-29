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

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use htui_core::model::{ProjectId, Scope};
use htui_store::vector::{Hit, SearchQuery};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::TextField;
use crate::ui::overlay::registry::{Overlay, OverlayId};

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

/// The keys, under everything.
const HINT: &str =
    "Enter search/open  Up/Dn move  Ctrl+D decisions  Ctrl+P project  Ctrl+R re-index  Esc close";

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

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Nothing on open: the first search loads a model, so only `Enter` asks (D234).
        Vec::new()
    }

    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        todo!()
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        todo!()
    }

    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {
        todo!()
    }
}

/// `text` cut to at most `cells` display cells, at a grapheme boundary (MOD-54's measure): a wide
/// glyph that would pass the edge is left out whole, never split.
fn clip(_text: &str, _cells: usize) -> String {
    todo!()
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
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
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
        assert!(
            bench
                .render(&search)
                .contains(&concepts::report_line(&report))
        );

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
        assert_eq!(
            cell_width(&expected),
            92,
            "whole wide glyphs up to the edge"
        );
        assert_eq!(clip("abc", 2), "ab");
        assert_eq!(
            clip("a一b", 2),
            "a",
            "a wide glyph that would pass the edge stops the line"
        );
    }

    #[test]
    fn hit_rows_is_the_cli_default_limit() {
        assert_eq!(u64::from(HIT_ROWS), concepts::DEFAULT_LIMIT);
    }
}
