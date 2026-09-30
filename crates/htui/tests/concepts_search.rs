//! The concepts search overlay (MOD-64 D233, D234, D236), end to end through a `Harness` over the
//! demo world and a `ConceptsRuntime` over `MemIndex` (no Qdrant, no model).
//!
//! The Harness enters the Graphics workspace at startup; the indexed rows are Platform's, so each
//! case moves there first (the `tests/reveal.rs` walk). The agent runtime is installed because a
//! Backlog selection sends a `PromptPreview`, which `settle` would answer `Failed` onto the status
//! line. Expected hit lines are computed from the fake index, never hard-coded: the prefix of
//! `format_hit(hit)` that fits the box's 92 hit cells, in the order the index ranks them.
#![cfg(feature = "testkit")]

use std::sync::Arc;

use htui::agent_worker::AgentRuntime;
use htui::app::register_all;
use htui::concepts::{self, format_hit, report_line};
use htui::concepts_worker::{ConceptsRuntime, MemIndex, NOT_AVAILABLE};
use htui::store_worker::{self, StoreReply, StoreRequest};
use htui::testkit::Harness;
use htui::ui::overlay::ConceptsSearch;
use htui::ui::overlay::concepts_search::{CHANGED, IDLE, INDEXING, LOADING};
use htui::ui::tabs::{BacklogTab, ChatTab, RequirementsTab};
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::Scope;
use htui_core::store::MemStore;
use htui_store::Backend;
use htui_store::vector::{Hit, PointType};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// Cells a hit line gets: the 94-cell inside of the box on a 100-column frame, less the cursor.
const HIT_CELLS: usize = 92;

/// The inside of the box on a 100-column frame.
const INNER_CELLS: usize = 94;

/// A query whose hits are an item, a requirement and document sections of Platform's `htui`.
const MIXED: &str = "postgres store cache";

/// Only `htui` FIX-1 matches it among closed items: the decisions search.
const PANIC: &str = "terminal raw panic";

/// The demo store and its Platform scope.
async fn platform_scope() -> (Backend, Scope) {
    let backend = Backend::memory(MemStore::demo());
    let StoreReply::Workspaces(workspaces) =
        store_worker::serve(&backend, &StoreRequest::Workspaces).await
    else {
        panic!("workspaces answered with the wrong variant")
    };
    let platform = workspaces
        .iter()
        .find(|w| w.slug == "platform")
        .expect("the demo fixture holds `platform`");
    (backend, Scope::from_workspace(platform))
}

/// A fake index holding every Platform point. The demo is deterministic, so the points are the
/// harness's own store's.
async fn seeded() -> Arc<MemIndex> {
    let index = MemIndex::new();
    let (backend, scope) = platform_scope().await;
    index.seed(&backend, &scope).await;
    Arc::new(index)
}

/// The demo shell in Platform, on the Backlog, everything served, with a concepts runtime over
/// `index` when one is given.
async fn open(index: Option<Arc<MemIndex>>) -> Harness {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    if let Some(index) = index {
        harness = harness.with_concepts_runtime(ConceptsRuntime::new(index));
    }
    register_all(harness.app());
    harness.drive_to_end().await;
    harness.key("w");
    harness.drive_to_end().await;
    harness.key("j");
    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    harness
}

/// Types `text` one key at a time: a space is `space`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        match c {
            ' ' => harness.key("space"),
            c => harness.key(&c.to_string()),
        }
    }
}

/// `Ctrl+F`, `text`, `Enter`, and everything served.
async fn search(harness: &mut Harness, text: &str) {
    harness.key("ctrl-f");
    type_text(harness, text);
    harness.key("enter");
    harness.drive_to_end().await;
}

/// What the fake index answers for `text` over `projects`, as the overlay asks it.
async fn expected(
    index: &MemIndex,
    text: &str,
    projects: Vec<htui_core::model::ProjectId>,
    decisions: bool,
) -> Vec<Hit> {
    use htui_store::vector::VectorStore as _;
    index
        .store()
        .search(&concepts::query(
            text,
            projects,
            decisions,
            concepts::DEFAULT_LIMIT,
        ))
        .await
        .expect("the fake never fails")
}

/// `text` cut to at most `cells` display cells at a grapheme boundary: the overlay's clip.
fn clip(text: &str, cells: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let width = grapheme.width();
        if used + width > cells {
            break;
        }
        used += width;
        out.push_str(grapheme);
    }
    out
}

/// Whether the search overlay is the top one.
fn search_is_open(harness: &mut Harness) -> bool {
    harness.app().overlays.top().map(|top| top.id()) == Some(ConceptsSearch::ID)
}

/// Moves the hit cursor to the first hit `wanted` accepts and returns it.
fn cursor_to(harness: &mut Harness, hits: &[Hit], wanted: impl Fn(&Hit) -> bool) -> Hit {
    let at = hits
        .iter()
        .position(wanted)
        .unwrap_or_else(|| panic!("no such hit among {hits:?}"));
    for _ in 0..at {
        harness.key("down");
    }
    hits[at].clone()
}

#[tokio::test]
async fn ctrl_f_opens_the_search_from_every_tab() {
    let mut harness = open(Some(seeded().await)).await;
    for tab in ["1", "2", "3", "4", "5"] {
        harness.key(tab);
        harness.drive_to_end().await;
        harness.key("ctrl-f");
        assert!(search_is_open(&mut harness), "Ctrl+F from tab {tab}");
        harness.key("esc");
        assert!(harness.app().overlays.is_empty(), "Esc closes it");
    }

    // F7: with the Chat composer open, the chord is still the shell's.
    harness.key("5");
    harness.drive_to_end().await;
    assert_eq!(harness.app().tabs.active_id(), Some(ChatTab::ID));
    harness.key("i");
    harness.key("ctrl-f");
    assert!(
        search_is_open(&mut harness),
        "Ctrl+F over the open composer"
    );
}

#[tokio::test]
async fn typed_letters_and_digits_are_query_text() {
    let mut harness = open(Some(seeded().await)).await;
    harness.key("ctrl-f");
    type_text(&mut harness, "wq12?jk");
    assert!(search_is_open(&mut harness), "no key closed or replaced it");
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    assert!(!harness.app().should_quit);
    assert!(!harness.app().help_visible);
    let frame = harness.render();
    assert!(frame.contains("query wq12?jk"), "{frame}");
}

#[tokio::test]
async fn enter_searches_and_lists_hits_as_the_cli_prints_them() {
    let index = seeded().await;
    let mut harness = open(Some(Arc::clone(&index))).await;
    search(&mut harness, MIXED).await;

    let hits = expected(&index, MIXED, platform_scope().await.1.project_ids, false).await;
    assert!(hits.len() > 2, "the query finds several kinds: {hits:?}");
    for kind in [PointType::Item, PointType::Requirement, PointType::Document] {
        assert!(hits.iter().any(|hit| hit.point_type == kind), "{kind:?}");
    }
    let frame = harness.render();
    let mut rest = frame.as_str();
    for hit in &hits {
        let line = clip(&format_hit(hit), HIT_CELLS);
        let at = rest
            .find(&line)
            .unwrap_or_else(|| panic!("`{line}` is not in the frame, in order:\n{frame}"));
        rest = &rest[at + line.len()..];
    }
    insta::assert_snapshot!("hits", frame);
}

#[tokio::test]
async fn enter_on_an_item_hit_reveals_it_in_the_backlog() {
    let index = seeded().await;
    let mut harness = open(Some(Arc::clone(&index))).await;
    harness.key("3");
    harness.drive_to_end().await;
    search(&mut harness, MIXED).await;
    let hits = expected(&index, MIXED, platform_scope().await.1.project_ids, false).await;
    let hit = cursor_to(&mut harness, &hits, |hit| hit.point_type == PointType::Item);

    harness.key("enter");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty());
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    let frame = harness.render();
    assert!(frame.contains(&format!("\u{250c} {} ", hit.key)), "{frame}");
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn enter_on_a_document_hit_reveals_its_owner_item() {
    let index = seeded().await;
    let mut harness = open(Some(Arc::clone(&index))).await;
    search(&mut harness, MIXED).await;
    let hits = expected(&index, MIXED, platform_scope().await.1.project_ids, false).await;
    let hit = cursor_to(&mut harness, &hits, |hit| {
        hit.point_type == PointType::Document
    });
    assert_eq!(hit.owner.item(), Some(ids::HTUI_ANA_1), "{hit:?}");

    harness.key("enter");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty());
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    let frame = harness.render();
    assert!(frame.contains("\u{250c} ANA-1 "), "{frame}");
}

#[tokio::test]
async fn enter_on_a_requirement_hit_opens_it_in_the_requirements_tab() {
    let index = seeded().await;
    let mut harness = open(Some(Arc::clone(&index))).await;
    search(&mut harness, MIXED).await;
    let hits = expected(&index, MIXED, platform_scope().await.1.project_ids, false).await;
    let hit = cursor_to(&mut harness, &hits, |hit| {
        hit.point_type == PointType::Requirement
    });

    harness.key("enter");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty());
    assert_eq!(harness.app().tabs.active_id(), Some(RequirementsTab::ID));
    let frame = harness.render();
    assert!(frame.contains(&format!("\u{250c} {} ", hit.key)), "{frame}");
    assert!(!frame.contains("select a requirement"), "{frame}");
}

#[tokio::test]
async fn ctrl_d_and_ctrl_p_change_the_header_and_mark_the_list_stale() {
    let mut harness = open(Some(seeded().await)).await;
    search(&mut harness, MIXED).await;
    let frame = harness.render();
    assert!(
        frame.contains("scope all projects · decisions off"),
        "{frame}"
    );
    assert!(!frame.contains(CHANGED), "{frame}");

    harness.key("ctrl-d");
    let frame = harness.render();
    assert!(
        frame.contains("scope all projects · decisions on"),
        "{frame}"
    );
    assert!(frame.contains(CHANGED), "{frame}");

    harness.key("ctrl-p");
    let frame = harness.render();
    assert!(frame.contains("scope htui · decisions on"), "{frame}");
    assert!(frame.contains(CHANGED), "{frame}");
}

#[tokio::test]
async fn decisions_in_one_project_list_only_closed_items() {
    let index = seeded().await;
    let mut harness = open(Some(Arc::clone(&index))).await;
    harness.key("ctrl-f");
    harness.key("ctrl-p");
    harness.key("ctrl-d");
    type_text(&mut harness, PANIC);
    harness.key("enter");
    harness.drive_to_end().await;

    let hits = expected(&index, PANIC, vec![ids::PROJECT_HTUI], true).await;
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.resolution.is_some()), "{hits:?}");
    let frame = harness.render();
    assert!(frame.contains("scope htui · decisions on"), "{frame}");
    for hit in &hits {
        assert!(
            frame.contains(&clip(&format_hit(hit), HIT_CELLS)),
            "{frame}"
        );
    }
    insta::assert_snapshot!("decisions_project", frame);
}

#[tokio::test]
async fn a_failing_index_is_an_inline_error_and_the_status_line_stays_empty() {
    let failing = Arc::new(MemIndex::new().failing("qdrant: query: connection refused"));
    let mut harness = open(Some(failing)).await;
    search(&mut harness, MIXED).await;
    assert!(search_is_open(&mut harness));
    assert_eq!(harness.app().status, None);
    let frame = harness.render();
    assert!(
        frame.contains("qdrant: query: connection refused"),
        "{frame}"
    );
    insta::assert_snapshot!("error", frame);
}

#[tokio::test]
async fn ctrl_r_reindexes_and_prints_the_report_line() {
    let mut harness = open(Some(Arc::new(MemIndex::new()))).await;
    harness.key("ctrl-f");
    harness.key("ctrl-r");
    harness.drive_to_end().await;

    let (backend, scope) = platform_scope().await;
    let report = MemIndex::new().seed(&backend, &scope).await;
    let frame = harness.render();
    // The line is wider than the box, which clips it like every other row.
    assert!(
        frame.contains(&clip(&report_line(&report), INNER_CELLS)),
        "{frame}"
    );
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn without_a_concepts_runtime_the_search_says_not_available() {
    let mut harness = open(None).await;
    search(&mut harness, MIXED).await;
    let frame = harness.render();
    assert!(frame.contains(NOT_AVAILABLE), "{frame}");
    assert_eq!(harness.app().status, None);
}

/// D240, end to end. An index run's reply carries no echo, so only the forgotten freshness entry
/// keeps a closed box's `Ctrl+R` report out of a reopened one (a search's `Hits` would be dropped
/// by its query echo alone, and could not tell).
#[tokio::test]
async fn a_reply_to_a_closed_search_is_not_shown_by_a_reopened_one() {
    let mut harness = open(Some(Arc::new(MemIndex::new()))).await;
    harness.key("ctrl-f");
    harness.key("ctrl-r");
    harness.key("esc");
    harness.key("ctrl-f");
    harness.drive_to_end().await;

    let (backend, scope) = platform_scope().await;
    let report = MemIndex::new().seed(&backend, &scope).await;
    let frame = harness.render();
    assert!(search_is_open(&mut harness));
    assert!(frame.contains(IDLE), "{frame}");
    assert!(
        !frame.contains(&clip(&report_line(&report), INNER_CELLS)),
        "the closed box's index report reached the reopened one:\n{frame}"
    );
    assert!(!frame.contains(INDEXING), "{frame}");
}

#[tokio::test]
async fn ctrl_c_still_quits_from_the_search() {
    let mut harness = open(Some(seeded().await)).await;
    harness.key("ctrl-f");
    harness.key("ctrl-c");
    assert!(harness.app().should_quit);
}

#[tokio::test]
async fn the_empty_box() {
    let mut harness = open(Some(seeded().await)).await;
    harness.key("ctrl-f");
    let frame = harness.render();
    assert!(frame.contains(IDLE), "{frame}");
    insta::assert_snapshot!("empty", frame);
}

#[tokio::test]
async fn the_first_search_in_flight_says_it_loads_the_model() {
    let mut harness = open(Some(seeded().await)).await;
    harness.key("ctrl-f");
    type_text(&mut harness, MIXED);
    harness.key("enter");
    let frame = harness.render();
    assert!(frame.contains(LOADING), "{frame}");
    insta::assert_snapshot!("searching", frame);
}
