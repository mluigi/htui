//! MOD-141 milestone 1, T8: the step-graph editor end to end (PRD metrics).
//!
//! Two drivers. The reference graph is built through the whole shell ([`Harness`]): every key
//! goes through `App::on_key`, every `ReplaceGraph` really travels the store worker
//! (`store_worker::serve` over the memory backend) and its reply comes back through the shell's
//! fan-out. The table-driven cases use a [`SectionBench`] and [`bench_round`], the `tests/kinds.rs`
//! worker-plus-section pattern without a shell. `MemStore` clones share their state, so every
//! assertion reads the store the requests were served from.
#![cfg(feature = "testkit")]

use std::collections::BTreeMap;

use htui::testkit::Harness;
use htui::ui::Theme;
use htui::ui::tabs::settings::{AgentsSection, HierarchySection, KindsSection, SettingsTab};
use htui_core::fixtures::ids;
use htui_core::model::{EdgeKind, GraphBody, ItemId, RunMode, StepGraphEdge, StepGraphId};
use htui_core::store::{MemStore, WorkerStore, WriteStore};

// -------------------------------------------------------------------------------------------
// The shell driver
// -------------------------------------------------------------------------------------------

/// The settled Settings tab over `store`, its three sections registered in the product's order
/// and the strip cycled onto `Kinds` (`tests/kinds.rs::kinds_over`).
async fn kinds_over(store: MemStore) -> Harness {
    let mut harness = Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
        Box::new(KindsSection::new()),
    ])));
    harness.settle().await;
    harness.key("l");
    harness.key("l");
    harness.settle().await;
    harness
}

/// Every character of `text` as its own key (a blank is `space`).
fn typed(harness: &mut Harness, text: &str) {
    for character in text.chars() {
        if character == ' ' {
            harness.key("space");
        } else {
            harness.key(&character.to_string());
        }
    }
}

/// `chord` pressed `times` times.
fn repeat(harness: &mut Harness, chord: &str, times: usize) {
    for _ in 0..times {
        harness.key(chord);
    }
}

/// The rows of the frame drawn in the theme's selection band, as text.
fn selected_rows(harness: &mut Harness) -> Vec<String> {
    let band = Theme::default()
        .selected
        .bg
        .expect("selected has a background");
    let buffer = harness.render_buffer();
    (0..buffer.area.height)
        .filter(|y| (0..buffer.area.width).any(|x| buffer[(x, *y)].bg == band))
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

/// Takes the Kinds cursor to the top, then presses `j` until the cursor's row reads `needle`.
#[track_caller]
fn cursor_to(harness: &mut Harness, needle: &str) {
    repeat(harness, "k", 96);
    for _ in 0..96 {
        if selected_rows(harness)
            .iter()
            .any(|row| row.contains(needle))
        {
            return;
        }
        harness.key("j");
    }
    panic!("no row reads `{needle}`: {}", harness.render());
}

/// Selects step `name` on the canvas with the editor's own direction keys. `e` opens the
/// selected step's form, whose title names it, so it is the probe; `Esc` cancels it.
#[track_caller]
fn select(harness: &mut Harness, name: &str) {
    let title = format!("step {name}");
    for chord in ["h", "k", "l", "j"].iter().cycle().take(24) {
        harness.key("e");
        let frame = harness.render();
        harness.key("esc");
        if frame.contains(&title) {
            return;
        }
        harness.key(chord);
    }
    panic!("no direction selects `{name}`: {}", harness.render());
}

/// The stored body of graph `graph`.
async fn body_of(store: &MemStore, graph: StepGraphId) -> GraphBody {
    store
        .graph_body(graph)
        .await
        .expect("a memory read never fails")
        .expect("the graph is stored")
}

/// One link by step name: `(from, to, kind, predicate JSON, loop limit)`.
type NamedEdge = (String, String, EdgeKind, Option<String>, Option<i32>);

/// `body`'s edges by step name, sorted by their ends.
fn edges_by_name(body: &GraphBody) -> Vec<NamedEdge> {
    let name = |id| {
        body.nodes
            .iter()
            .find(|node| node.phase.id == id)
            .map_or_else(|| "?".to_owned(), |node| node.phase.name.clone())
    };
    let mut edges: Vec<_> = body
        .edges
        .iter()
        .map(|edge: &StepGraphEdge| {
            (
                name(edge.from),
                name(edge.to),
                edge.kind,
                edge.predicate
                    .as_ref()
                    .map(|predicate| predicate.0.to_string()),
                edge.loop_limit,
            )
        })
        .collect();
    edges.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    edges
}

/// PRD metric "0 out-of-TUI edits for the reference graph": `implement -> review`, review loops
/// back to implement at most 3 times, an `if {"verify":"failed"}` branch to a fix step. Built
/// with keys only over the whole shell, saved through the store worker, put under a kind with
/// the existing kind editor, and resolved by the orchestrator's own resolver.
#[tokio::test]
async fn the_reference_graph_is_built_in_the_tui_and_resolves() {
    let store = MemStore::demo();
    let mut harness = kinds_over(store.clone()).await;

    // 1. `N` on the project row creates graph `reference`; Enter on its row opens the editor.
    harness.key("N");
    typed(&mut harness, "reference");
    harness.key("enter");
    harness.settle().await;
    cursor_to(&mut harness, "reference \u{b7} graph, no kind");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("reference \u{b7} project graph \u{b7} revision 0"),
        "the editor opens on the empty graph: {frame}"
    );

    // 2. `a`: `implement`, its template chosen on the template row.
    harness.key("a");
    typed(&mut harness, "implement");
    harness.key("tab");
    // Templates in name order: fix, handoff, implement.
    repeat(&mut harness, "l", 3);
    // A candidate agent: the first agent and model the store's registry answers.
    harness.key("alt-n");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("saved: adding implement"), "{frame}");

    // 3. `a` with implement selected: `implement -next-> review`.
    harness.key("a");
    typed(&mut harness, "review");
    harness.key("alt-n");
    harness.key("enter");
    harness.settle().await;
    // 4. `a` with review selected: `review -next-> fix`.
    harness.key("a");
    typed(&mut harness, "fix");
    harness.key("alt-n");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("saved: adding fix"), "{frame}");

    // 5. fix's links list: its one link in, `← review · next`, deleted.
    harness.key("i");
    let frame = harness.render();
    assert!(frame.contains("\u{2190} review \u{b7} next"), "{frame}");
    harness.key("x");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("saved: removing the link review \u{2192} fix"),
        "{frame}"
    );
    harness.key("esc");

    // 6. implement `c` fix: `if {"verify":"failed"}`.
    select(&mut harness, "implement");
    harness.key("c");
    // fix lost its only link in, so it is a second root beside implement.
    harness.key("l");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains("link implement \u{2192} fix"),
        "the pick lands on fix: {frame}"
    );
    harness.key("l");
    harness.key("tab");
    typed(&mut harness, r#"{"verify":"failed"}"#);
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("saved: linking implement \u{2192} fix"),
        "{frame}"
    );

    // 7. review `c` implement: `changes requested`, at most 3 times round.
    select(&mut harness, "review");
    harness.key("c");
    harness.key("k");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains("link review \u{2192} implement"),
        "the pick lands on implement: {frame}"
    );
    repeat(&mut harness, "l", 3);
    harness.key("tab");
    typed(&mut harness, "3");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("saved: linking review \u{2192} implement"),
        "{frame}"
    );

    // 11. The final editor frame.
    insta::assert_snapshot!("reference_graph", frame);

    // 8. `Esc` closes the editor; `e` on kind FEAT puts it on `reference` (the existing kind
    // editor: prefix, name, description, graph).
    harness.key("esc");
    cursor_to(&mut harness, "FEAT  feature");
    harness.key("e");
    repeat(&mut harness, "tab", 3);
    repeat(&mut harness, "backspace", "feature".len());
    typed(&mut harness, "reference");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("FEAT  feature \u{b7} graph reference"),
        "the kind now runs the reference graph: {frame}"
    );

    // 9. The stored body: three steps and exactly the three links.
    let item: ItemId = ids::VULKAN_FEAT_1;
    let resolved = store
        .resolve_graph(item)
        .await
        .expect("a memory read never fails")
        .expect("the item resolves a graph");
    assert_eq!(resolved.graph.name, "reference");
    let body = body_of(&store, resolved.graph.id).await;
    assert_eq!(body.nodes.len(), 3, "{body:?}");
    for node in &body.nodes {
        assert_eq!(node.phase.template_name, "implement", "{}", node.phase.name);
        assert_eq!(node.agents.len(), 1, "{}: one candidate", node.phase.name);
    }
    assert_eq!(
        edges_by_name(&body),
        vec![
            (
                "implement".to_owned(),
                "fix".to_owned(),
                EdgeKind::If,
                Some(r#"{"verify":"failed"}"#.to_owned()),
                None,
            ),
            (
                "implement".to_owned(),
                "review".to_owned(),
                EdgeKind::Next,
                None,
                None,
            ),
            (
                "review".to_owned(),
                "implement".to_owned(),
                EdgeKind::ChangesRequested,
                None,
                Some(3),
            ),
        ]
    );
    assert_eq!(
        htui_core::store::graph_body_walk_refusal(resolved.graph.id, &body),
        None,
        "the store's walk rules take it"
    );

    // 10. The item resolves through its kind, phases in position order, and the orchestrator's
    // resolver freezes it into a run snapshot.
    let phases: Vec<&str> = resolved
        .phases
        .iter()
        .map(|phase| phase.phase.name.as_str())
        .collect();
    assert_eq!(phases, ["implement", "review", "fix"]);
    let item = WorkerStore::item(&store, item)
        .await
        .expect("a memory read never fails")
        .expect("the demo item");
    let snapshot = htui_orch::graph::resolve(
        &store,
        &store,
        &item,
        RunMode::Manual,
        &BTreeMap::new(),
        None,
        ids::BOX,
    )
    .await
    .unwrap_or_else(|refusal| panic!("the reference graph resolves: {refusal}"))
    .snapshot;
    let frozen: Vec<&str> = snapshot
        .phases
        .iter()
        .map(|phase| phase.name.as_str())
        .collect();
    assert_eq!(frozen, ["implement", "review", "fix"]);
    assert_eq!(snapshot.edges.len(), 3, "{:?}", snapshot.edges);
}
