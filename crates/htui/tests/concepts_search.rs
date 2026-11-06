//! The concepts search overlay (MOD-64 D233, D234, D236), end to end through a `Harness` over the
//! demo world and a `ConceptsRuntime` over `MemIndex` (no Qdrant, no model).
//!
//! The Harness enters the Graphics workspace at startup; the indexed rows are Platform's, so each
//! case moves there first (the `tests/reveal.rs` walk). The agent runtime is installed because a
//! Backlog selection sends a `PromptPreview`, which `settle` would answer `Failed` onto the status
//! line. Expected hit lines are computed from the fake index, never hard-coded: the prefix of
//! the overlay's own row (`hit_row`, MOD-115 M4 D-19) that fits the box's hit cells, in the
//! order the index ranks them.
#![cfg(feature = "testkit")]

use std::sync::Arc;

use htui::agent_worker::AgentRuntime;
use htui::app::register_all;
use htui::concepts;
use htui::concepts_worker::{ConceptsRuntime, MemIndex, NOT_AVAILABLE};
use htui::store_worker::{self, StoreReply, StoreRequest};
use htui::testkit::Harness;
use htui::ui::overlay::ConceptsSearch;
use htui::ui::overlay::concepts_search::{
    INDEXING, LOADING, changed, hit_row, idle, index_report, unavailable_status,
};
use htui::ui::tabs::{BacklogTab, ChatTab, RequirementsTab};
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::Scope;
use htui_core::store::MemStore;
use htui_store::Backend;
use htui_store::vector::{Hit, PointType};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// Whether the Backlog's detail pane is on `key` (MOD-115 M4 B1): its title on the rounded
/// top border is `KEY · title` (`╭─ KEY ·`), or the bare `Detail` with no item selected.
fn on_item(frame: &str, key: &str) -> bool {
    frame.contains(&format!("\u{256d}\u{2500} {key} "))
        || frame.contains(&format!("\u{2500} {key} \u{b7}"))
}

/// Cells a hit line's text keeps on a 100-column frame: the box's 93-cell content less the
/// list's gutter is 92, and a longer row ends in an ellipsis after its 91st.
const HIT_CELLS: usize = 91;

/// Cells a one-row line keeps on a 100-column frame: the 93-cell content, a longer line ending in
/// an ellipsis after its 92nd.
const INNER_CELLS: usize = 92;

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
    in_platform(harness).await
}

/// `harness` with every tab and overlay registered, moved to Platform, on the Backlog.
async fn in_platform(mut harness: Harness) -> Harness {
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

/// The best score among `hits`: what each hit's score word is relative to.
fn best(hits: &[Hit]) -> f32 {
    hits.iter().map(|hit| hit.score).fold(0.0, f32::max)
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
async fn enter_searches_and_lists_hits_in_rank_order() {
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
    let best = best(&hits);
    for hit in &hits {
        let line = clip(&hit_row(hit, best), HIT_CELLS);
        let at = rest
            .find(&line)
            .unwrap_or_else(|| panic!("`{line}` is not in the frame, in order:\n{frame}"));
        rest = &rest[at + line.len()..];
    }
    insta::assert_snapshot!("hits", frame);

    // MOD-115 M4 E6: at 80 columns every hit is still listed, cut at the box's edge.
    harness.resize(80, 30);
    let frame = harness.render();
    for hit in &hits {
        assert!(
            frame.contains(&clip(&hit_row(hit, best), 40)),
            "{}: {frame}",
            hit.key
        );
    }
    insta::assert_snapshot!("hits_80", frame);
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
    assert!(on_item(&frame, &hit.key), "{frame}");
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
    assert!(on_item(&frame, "ANA-1"), "{frame}");
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
    // MOD-115 M3 L-req: the Requirements detail is a kit pane, titled `╭─ KEY `.
    assert!(
        frame.contains(&format!("\u{256d}\u{2500} {} ", hit.key)),
        "{frame}"
    );
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
    let stale = changed(Some("Enter"));
    assert!(!frame.contains(&stale), "{frame}");

    harness.key("ctrl-d");
    let frame = harness.render();
    assert!(
        frame.contains("scope all projects · decisions on"),
        "{frame}"
    );
    assert!(frame.contains(&stale), "{frame}");

    harness.key("ctrl-p");
    let frame = harness.render();
    assert!(frame.contains("scope htui · decisions on"), "{frame}");
    assert!(frame.contains(&stale), "{frame}");
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
    let best = best(&hits);
    for hit in &hits {
        assert!(
            frame.contains(&clip(&hit_row(hit, best), HIT_CELLS)),
            "{frame}"
        );
    }
    insta::assert_snapshot!("decisions_project", frame);
}

/// MOD-160 D7: a failing search says the one sentence in the box, the why key in force named on
/// its status row and border; `Ctrl+E` opens the cause with where it is fixed, and closes it. The
/// shell's status line stays empty throughout.
#[tokio::test]
async fn a_failing_search_says_unavailable_and_ctrl_e_shows_why() {
    let failing = Arc::new(MemIndex::new().failing("qdrant: query: connection refused"));
    let mut harness = open(Some(failing)).await;
    search(&mut harness, MIXED).await;
    assert!(search_is_open(&mut harness));
    assert_eq!(harness.app().status, None);
    let frame = harness.render();
    assert!(
        frame.contains(&unavailable_status(Some("Ctrl+e"))),
        "{frame}"
    );
    assert!(frame.contains("Ctrl+e why"), "the border names it: {frame}");
    // R-3: the row stays whole on the border at 100 columns; `move` gives way to `why`, since a
    // failed search has no hits to move through, and `close` still ends the row.
    assert!(
        frame.contains("Ctrl+r index · Ctrl+e why · Esc close"),
        "the border row is whole: {frame}"
    );
    assert!(!frame.contains("↑/↓ move"), "move gives way: {frame}");
    assert!(!frame.contains("connection refused"), "{frame}");
    insta::assert_snapshot!("error", frame);

    harness.key("ctrl-e");
    let frame = harness.render();
    assert!(
        frame.contains("qdrant: query: connection refused \u{2014} Settings \u{203a} Qdrant"),
        "{frame}"
    );
    assert!(search_is_open(&mut harness));
    assert_eq!(harness.app().status, None);
    insta::assert_snapshot!("error_why", frame);

    harness.key("ctrl-e");
    let frame = harness.render();
    assert!(!frame.contains("connection refused"), "closed: {frame}");
}

/// MOD-160 D7: `[concepts] why = "f6"` moves the why key: the status row and the border name
/// `F6`, `Ctrl+E` does nothing (and types nothing), and `F6` opens the cause.
#[tokio::test]
async fn a_rebound_why_key_moves_the_hint_and_acts() {
    let keys =
        htui::keys::load_str("version = 1\n[concepts]\nwhy = \"f6\"\n").expect("the keys load");
    let failing = Arc::new(MemIndex::new().failing("qdrant: query: connection refused"));
    let mut harness = in_platform(
        Harness::demo()
            .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
            .with_concepts_runtime(ConceptsRuntime::new(failing))
            .with_keys(keys),
    )
    .await;
    search(&mut harness, MIXED).await;
    let frame = harness.render();
    assert!(frame.contains(&unavailable_status(Some("F6"))), "{frame}");
    assert!(
        frame.contains("F6 why"),
        "the hint names the new chord: {frame}"
    );
    assert!(!frame.contains("Ctrl+e"), "and not the old one: {frame}");

    harness.key("ctrl-e");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(search_is_open(&mut harness));
    assert!(
        !frame.contains("connection refused"),
        "`Ctrl+E` opened nothing: {frame}"
    );
    assert!(
        !frame.contains(&format!("query {MIXED}e")),
        "and typed nothing: {frame}"
    );

    harness.key("f6");
    let frame = harness.render();
    assert!(
        frame.contains("qdrant: query: connection refused \u{2014} Settings \u{203a} Qdrant"),
        "`F6` opened the cause: {frame}"
    );
    assert_eq!(harness.app().status, None);
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
        frame.contains(&clip(&index_report(&report), INNER_CELLS)),
        "{frame}"
    );
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn without_a_concepts_runtime_the_search_says_not_available() {
    let mut harness = open(None).await;
    search(&mut harness, MIXED).await;
    let frame = harness.render();
    // MOD-160 D7: the sentence on the status row, the harness's own cause behind the why key.
    assert!(
        frame.contains(&unavailable_status(Some("Ctrl+e"))),
        "{frame}"
    );
    assert!(!frame.contains(NOT_AVAILABLE), "{frame}");
    harness.key("ctrl-e");
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
    assert!(frame.contains(&idle(Some("Enter"))), "{frame}");
    assert!(
        !frame.contains(&clip(&index_report(&report), INNER_CELLS)),
        "the closed box's index report reached the reopened one:\n{frame}"
    );
    assert!(!frame.contains(INDEXING), "{frame}");
}

#[tokio::test]
async fn ctrl_c_still_quits_from_the_search() {
    let mut harness = open(Some(seeded().await)).await;
    harness.key("ctrl-f");
    // MOD-115 M2 D6: the leave dialog opens over the search, and a second `ctrl-c` leaves.
    harness.key("ctrl-c");
    assert!(!harness.app().should_quit);
    assert_eq!(
        harness.app().overlays.top().map(|overlay| overlay.id()),
        Some(htui::ui::overlay::LeaveDialog::ID)
    );
    harness.key("ctrl-c");
    assert!(harness.app().should_quit);
}

#[tokio::test]
async fn the_empty_box() {
    let mut harness = open(Some(seeded().await)).await;
    harness.key("ctrl-f");
    let frame = harness.render();
    assert!(frame.contains(&idle(Some("Enter"))), "{frame}");
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

/// The keys line the status row listed before MOD-115 (nothing draws it any more).
fn status_line(harness: &mut Harness) -> String {
    harness.app().keys_line()
}

/// MOD-67 M3 (D7, D8, L-E Q7): the search's field captures `?`, so the status line, the `?` box
/// and its closer name only `F1`.
#[tokio::test]
async fn the_status_line_and_help_box_over_the_search_name_f1() {
    let mut harness = open(Some(seeded().await)).await;
    harness.key("ctrl-f");
    assert_eq!(status_line(&mut harness), "Ctrl+c quit · F1 help");
    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` opens the box");
    let frame = harness.render();
    for line in [
        "Search concepts: Ctrl+d decisions only",
        "Overlay: Esc close",
        "Global: Ctrl+c quit · F1 help",
        "F1 closes this box",
    ] {
        assert!(frame.contains(line), "`{line}` is in the box: {frame}");
    }
    assert!(!frame.contains("?/F1"), "`?` is query text here: {frame}");
}

/// MOD-67 M3: the lane's rebinding test. `[concepts] reindex = "f5"` moves the re-index to `F5`:
/// `Ctrl+R` does nothing (and types nothing), `F5` runs it, and the hint names `F5`.
#[tokio::test]
async fn a_rebound_reindex_acts_and_the_old_chord_is_inert() {
    let keys =
        htui::keys::load_str("version = 1\n[concepts]\nreindex = \"f5\"\n").expect("the keys load");
    let mut harness = in_platform(
        Harness::demo()
            .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
            .with_concepts_runtime(ConceptsRuntime::new(Arc::new(MemIndex::new())))
            .with_keys(keys),
    )
    .await;

    let (backend, scope) = platform_scope().await;
    let report = clip(
        &index_report(&MemIndex::new().seed(&backend, &scope).await),
        INNER_CELLS,
    );

    harness.key("ctrl-f");
    harness.key("x");
    harness.key("ctrl-r");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(search_is_open(&mut harness));
    assert!(!frame.contains(&report), "`Ctrl+R` ran nothing: {frame}");
    assert!(!frame.contains(INDEXING), "{frame}");
    assert!(
        frame.contains("query x") && !frame.contains("query xr"),
        "and typed nothing: {frame}"
    );
    assert!(
        frame.contains("F5 index"),
        "the hint names the new chord: {frame}"
    );
    assert!(!frame.contains("Ctrl+r"), "and not the old one: {frame}");

    harness.key("f5");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains(&report), "`F5` re-indexed: {frame}");
    assert_eq!(harness.app().status, None);
}

/// MOD-67 M6 D4: a search that found nothing names the re-index key in force.
#[tokio::test]
async fn a_rebound_reindex_key_is_named_with_no_matches() {
    let keys = htui::keys::load_str("version = 1\n[concepts]\nreindex = \"ctrl-o\"\n")
        .expect("the keys load");
    let mut harness = in_platform(
        Harness::demo()
            .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
            .with_concepts_runtime(ConceptsRuntime::new(Arc::new(MemIndex::new())))
            .with_keys(keys),
    )
    .await;
    harness.key("ctrl-f");
    for pressed in ["q", "u", "x"] {
        harness.key(pressed);
    }
    harness.key("enter");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("no matches \u{2014} Ctrl+o indexes this scope if the index is empty"),
        "{frame}"
    );
}
