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

use chrono::DateTime;
use htui::app::Action;
use htui::keys::{Stack, load_str, views};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{
    AgentsSection, HierarchySection, KindsSection, SettingsSection, SettingsTab,
};
use htui_core::fixtures::ids;
use htui_core::model::predicate::IfPredicate;
use htui_core::model::{
    AgentId, CommandQueue, EdgeKind, Gate, GraphBody, Isolation, ItemId, NodeBody, PhaseAgent,
    PhaseId, RunMode, StepGraphEdge, StepGraphId,
};
use htui_core::store::{
    CasOutcome, MemStore, WorkerStore, WriteStore, loop_edge_runs_forward, reserved_phase_name,
};
use htui_store::Backend;

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

/// A new step on an empty graph starts with no template. While the scope has templates to choose,
/// saving it asks for one to be chosen; it never sends the user off to create a template that
/// already exists (that sentence is for a scope with none).
#[tokio::test]
async fn a_step_saved_without_a_template_asks_for_one() {
    let mut harness = kinds_over(MemStore::demo()).await;
    harness.key("N");
    typed(&mut harness, "solo");
    harness.key("enter");
    harness.settle().await;
    cursor_to(&mut harness, "solo \u{b7} graph, no kind");
    harness.key("enter");
    harness.settle().await;

    harness.key("a");
    typed(&mut harness, "solo");
    harness.key("enter");
    harness.settle().await;
    let frame = flat(&harness.render());
    assert!(
        frame.contains("`template` is required; choose one"),
        "{frame}"
    );
    assert!(!frame.contains("add one in Skills"), "{frame}");
    assert!(!frame.contains("saved:"), "nothing written: {frame}");

    // Choosing one on the template row saves.
    harness.key("tab");
    harness.key("l");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("saved: adding solo"), "{frame}");
}

// -------------------------------------------------------------------------------------------
// The section bench
// -------------------------------------------------------------------------------------------

/// The graph every bench case edits: vulkan-tutorials' `analysis`, `research -next-> verdict`.
const ANALYSIS: StepGraphId = ids::GRAPH_VULKAN_ANA;

/// The words a frame shows joined by single spaces, so a wrapped sentence reads as one.
fn flat(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The id of step `name` of `body`.
#[track_caller]
fn step(body: &GraphBody, name: &str) -> PhaseId {
    body.nodes
        .iter()
        .find(|node| node.phase.name == name)
        .unwrap_or_else(|| panic!("no step {name}: {body:?}"))
        .phase
        .id
}

/// `body` with what every write stamps (the revision, `updated_at`) set aside, so two bodies
/// compare on what an edit changes.
fn shape(body: &GraphBody) -> GraphBody {
    let epoch = DateTime::UNIX_EPOCH;
    let mut body = body.clone();
    body.graph.revision = 0;
    body.graph.updated_at = epoch;
    for node in &mut body.nodes {
        node.phase.updated_at = epoch;
    }
    body
}

/// Drains the section's store requests, serves each with the store worker's own `serve` over
/// `backend` and hands the reply back, until the section asks nothing more. The requests served,
/// in order.
async fn bench_round(
    bench: &SectionBench,
    section: &mut KindsSection,
    backend: &Backend,
) -> Vec<StoreRequest> {
    let mut served = Vec::new();
    loop {
        let requests: Vec<StoreRequest> = bench
            .drained()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect();
        if requests.is_empty() {
            return served;
        }
        for request in requests {
            let reply = serve(backend, &request).await;
            bench.reply(section, &reply);
            served.push(request);
        }
    }
}

/// A Kinds section over `store` (the bench's scope's catalogue delivered), with the graph editor
/// open on [`ANALYSIS`] from its kind row and its agents and templates read.
async fn analysis_bench(
    store: &MemStore,
    keys: Option<htui::keys::Keys>,
) -> (SectionBench, KindsSection, Backend) {
    let backend = Backend::memory(store.clone());
    let bench = match keys {
        Some(keys) => SectionBench::new().await.with_keys(keys),
        None => SectionBench::new().await,
    };
    let mut section = KindsSection::new();
    let scope = bench.ctx().scope.clone();
    let catalogue = serve(&backend, &StoreRequest::Catalogue(scope)).await;
    bench.reply(&mut section, &catalogue);
    let _ = bench_round(&bench, &mut section, &backend).await;
    // The project row, then `ANA analysis`: Enter opens its graph.
    bench.key(&mut section, "j");
    bench.key(&mut section, "enter");
    let read = bench_round(&bench, &mut section, &backend).await;
    assert!(
        matches!(
            read.as_slice(),
            [StoreRequest::Agents, StoreRequest::Templates(_)]
        ),
        "the editor opened and read its lookups: {read:?}"
    );
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));
    (bench, section, backend)
}

/// Every chord in order.
fn press(bench: &SectionBench, section: &mut KindsSection, chords: &[&str]) {
    for chord in chords {
        bench.key(section, chord);
    }
}

/// Every character of `text` as its own key (a blank is `space`).
fn type_in(bench: &SectionBench, section: &mut KindsSection, text: &str) {
    for character in text.chars() {
        if character == ' ' {
            bench.key(section, "space");
        } else {
            bench.key(section, &character.to_string());
        }
    }
}

/// Selects step `name` on the canvas with the editor's direction keys, `e`'s form title the
/// probe (as [`select`]).
#[track_caller]
fn bench_select(bench: &SectionBench, section: &mut KindsSection, name: &str) {
    let title = format!("step {name}");
    for chord in ["h", "k", "l", "j"].iter().cycle().take(24) {
        bench.key(section, "e");
        let frame = bench.render_section(section, 100);
        bench.key(section, "esc");
        if frame.contains(&title) {
            return;
        }
        bench.key(section, chord);
    }
    panic!(
        "no direction selects `{name}`: {}",
        bench.render_section(section, 100)
    );
}

/// The section's current key stack, as a name for messages.
fn stack_of(section: &KindsSection) -> Option<Stack<'static>> {
    section.key_stack()
}

/// The (agent, model) pairs the node form's candidate rows cycle over, in its order: agents as
/// the store answers them, each agent's models in order, a model-less agent with its default.
async fn agent_pairs(backend: &Backend) -> Vec<(AgentId, String)> {
    let StoreReply::Agents(agents) = serve(backend, &StoreRequest::Agents).await else {
        panic!("the agents read answers");
    };
    agents
        .iter()
        .flat_map(|summary| {
            let agent = &summary.agent;
            if agent.models.is_empty() {
                vec![(agent.id, agent.default_model.clone().unwrap_or_default())]
            } else {
                agent
                    .models
                    .iter()
                    .map(|model| (agent.id, model.clone()))
                    .collect()
            }
        })
        .collect()
}

/// `phase`'s candidate rows for `pairs`, in order.
fn candidates(phase: PhaseId, pairs: &[&(AgentId, String)]) -> Vec<PhaseAgent> {
    pairs
        .iter()
        .enumerate()
        .map(|(at, (agent_id, model))| PhaseAgent {
            phase_id: phase,
            position: i32::try_from(at).expect("a small index"),
            agent_id: *agent_id,
            model: model.clone(),
        })
        .collect()
}

/// The option after `current` in `options`, wrapping (the form's choice rows).
fn after<T: Copy + PartialEq>(options: &[T], current: T) -> T {
    let at = options
        .iter()
        .position(|option| *option == current)
        .expect("the current option is listed");
    options[(at + 1) % options.len()]
}

/// One node-form case: what the keys do from the form's first row, and how the stored node
/// differs from the seed's for it.
struct NodeCase {
    /// The column, for messages.
    column: &'static str,
    /// The chords after `e`; `"SAVE"` saves and lets the store answer, mid-case.
    chords: Vec<String>,
    /// The seed node, changed as the case expects it stored.
    expect: Box<dyn Fn(&mut NodeBody)>,
}

/// `n` tabs, then `chords`.
fn at_row(n: usize, chords: &[&str]) -> Vec<String> {
    core::iter::repeat_n("tab", n)
        .chain(chords.iter().copied())
        .map(str::to_owned)
        .collect()
}

/// `n` tabs, the row's text cleared, then `text` typed.
fn retyped(n: usize, text: &str) -> Vec<String> {
    let mut chords = at_row(n, &["end"]);
    chords.extend(core::iter::repeat_n("backspace".to_owned(), 40));
    chords.extend(text.chars().map(|character| {
        if character == ' ' {
            "space".to_owned()
        } else {
            character.to_string()
        }
    }));
    chords
}

/// PRD metric "every M1 node field editable": each column of the step form, edited in the form,
/// saved through the store, and read back from `graph_body`; every other column of the graph
/// unchanged.
#[tokio::test]
async fn every_m1_node_field_round_trips() {
    let seed = body_of(&MemStore::demo(), ANALYSIS).await;
    let research = step(&seed, "research");
    let node = |body: &GraphBody| {
        body.nodes
            .iter()
            .find(|node| node.phase.id == research)
            .cloned()
            .expect("research")
    };
    let seeded = node(&seed).phase;
    let pairs = agent_pairs(&Backend::memory(MemStore::demo())).await;
    assert!(pairs.len() >= 2, "two candidates to reorder: {pairs:?}");
    let (first, second) = (pairs[0].clone(), pairs[1].clone());
    let personas = {
        let StoreReply::Catalogue(snapshot) = serve(
            &Backend::memory(MemStore::demo()),
            &StoreRequest::Catalogue(SectionBench::new().await.ctx().scope.clone()),
        )
        .await
        else {
            panic!("the catalogue reads");
        };
        snapshot.personas.clone()
    };
    let isolations: Vec<Option<Isolation>> = core::iter::once(None)
        .chain(Isolation::ALL.iter().copied().map(Some))
        .collect();

    let cases = vec![
        NodeCase {
            column: "name",
            chords: retyped(0, "inquiry"),
            expect: Box::new(|node| "inquiry".clone_into(&mut node.phase.name)),
        },
        NodeCase {
            column: "template",
            // research's next in name order.
            chords: at_row(1, &["l"]),
            expect: Box::new(|node| {
                "review".clone_into(&mut node.phase.template_name);
                node.phase.template_version = None;
            }),
        },
        NodeCase {
            column: "version",
            chords: at_row(2, &["l"]),
            expect: Box::new(|node| node.phase.template_version = Some(1)),
        },
        NodeCase {
            column: "persona",
            chords: at_row(3, &["l"]),
            expect: Box::new({
                let first = personas.first().map(|persona| persona.id);
                move |node| node.phase.persona_id = first
            }),
        },
        NodeCase {
            column: "gate",
            chords: at_row(4, &["l"]),
            expect: Box::new(move |node| node.phase.gate = after(Gate::ALL, seeded.gate)),
        },
        NodeCase {
            column: "hard gate",
            chords: at_row(5, &["l"]),
            expect: Box::new(move |node| node.phase.gate_hard = !seeded.gate_hard),
        },
        NodeCase {
            column: "retry limit",
            chords: retyped(6, "4"),
            expect: Box::new(|node| node.phase.retry_limit = 4),
        },
        NodeCase {
            column: "isolation",
            chords: at_row(7, &["l"]),
            expect: Box::new(move |node| {
                node.phase.isolation = after(&isolations, seeded.isolation);
            }),
        },
        NodeCase {
            column: "commands",
            chords: at_row(8, &["l"]),
            expect: Box::new(move |node| {
                node.phase.command_queue = after(CommandQueue::ALL, seeded.command_queue);
            }),
        },
        NodeCase {
            column: "verify",
            chords: retyped(9, "cargo test"),
            expect: Box::new(|node| node.phase.verify_command = Some("cargo test".to_owned())),
        },
        NodeCase {
            column: "token budget",
            chords: retyped(10, "5000"),
            expect: Box::new(|node| node.phase.token_budget = Some(5000)),
        },
        NodeCase {
            column: "inputs",
            chords: retyped(11, "plan, notes"),
            expect: Box::new(|node| {
                node.phase.input_kinds = vec!["plan".to_owned(), "notes".to_owned()];
            }),
        },
        NodeCase {
            column: "output",
            chords: retyped(12, "brief"),
            expect: Box::new(|node| "brief".clone_into(&mut node.phase.output_kind)),
        },
        NodeCase {
            column: "candidates: add",
            chords: at_row(0, &["alt-n"]),
            expect: Box::new({
                let first = first.clone();
                move |node| node.agents = candidates(research, &[&first])
            }),
        },
        NodeCase {
            column: "candidates: reorder",
            // Two candidates, the second cycled to the next pair, then moved up.
            chords: at_row(0, &["alt-n", "alt-n", "l", "alt-up"]),
            expect: Box::new({
                let (first, second) = (first.clone(), second.clone());
                move |node| node.agents = candidates(research, &[&second, &first])
            }),
        },
        NodeCase {
            column: "candidates: remove",
            // Two saved, then the second removed in a second write.
            chords: {
                let mut chords = at_row(0, &["alt-n", "alt-n", "l", "SAVE", "e"]);
                chords.extend(at_row(14, &["alt-d"]));
                chords
            },
            expect: Box::new({
                let first = first.clone();
                move |node| node.agents = candidates(research, &[&first])
            }),
        },
    ];

    for case in cases {
        let store = MemStore::demo();
        let (bench, mut section, backend) = analysis_bench(&store, None).await;
        let before = body_of(&store, ANALYSIS).await;
        bench.key(&mut section, "e");
        assert_eq!(
            stack_of(&section),
            Some(views::GRAPH_EDITOR_NODE_FORM),
            "{}",
            case.column
        );
        for chord in &case.chords {
            if chord == "SAVE" {
                bench.key(&mut section, "enter");
                let _ = bench_round(&bench, &mut section, &backend).await;
            } else {
                bench.key(&mut section, chord);
            }
        }
        bench.key(&mut section, "enter");
        let served = bench_round(&bench, &mut section, &backend).await;
        assert!(
            served
                .iter()
                .any(|request| matches!(request, StoreRequest::ReplaceGraph { .. })),
            "{}: the save went out: {}",
            case.column,
            bench.render_section(&section, 100)
        );
        let stored = body_of(&store, ANALYSIS).await;
        let mut expected = before.clone();
        let at = expected
            .nodes
            .iter()
            .position(|node| node.phase.id == research)
            .expect("research");
        (case.expect)(&mut expected.nodes[at]);
        assert_eq!(shape(&stored), shape(&expected), "{}", case.column);
        assert_ne!(
            shape(&stored),
            shape(&before),
            "{}: the column moved",
            case.column
        );
    }
}

/// The one link `from -> to` of `body`.
#[track_caller]
fn link(body: &GraphBody, from: &str, to: &str) -> StepGraphEdge {
    let (from, to) = (step(body, from), step(body, to));
    let found: Vec<&StepGraphEdge> = body
        .edges
        .iter()
        .filter(|edge| edge.from == from && edge.to == to)
        .collect();
    let [edge] = found.as_slice() else {
        panic!("one link: {found:?}");
    };
    (*edge).clone()
}

/// Opens the link form on `from`'s link `row` (out-links first) from the canvas.
fn edit_link(bench: &SectionBench, section: &mut KindsSection, from: &str, row: usize) {
    bench_select(bench, section, from);
    bench.key(section, "i");
    for _ in 0..row {
        bench.key(section, "j");
    }
    bench.key(section, "e");
    assert_eq!(stack_of(section), Some(views::GRAPH_EDITOR_EDGE_FORM));
}

/// Saves the open form, lets the store answer and checks it applied.
async fn saved(bench: &SectionBench, section: &mut KindsSection, backend: &Backend) {
    bench.key(section, "enter");
    let served = bench_round(bench, section, backend).await;
    assert!(
        served
            .iter()
            .any(|request| matches!(request, StoreRequest::ReplaceGraph { .. })),
        "a write went out: {}",
        bench.render_section(section, 100)
    );
    let frame = bench.render_section(section, 100);
    assert!(frame.contains("saved: "), "it applied: {frame}");
}

/// PRD metric "every edge field editable": the relation (every kind, each on a pair it is valid
/// on), the `if` predicate and the loop limit (set, changed, cleared on `asks you`), each saved and
/// read back; the other links unchanged.
#[tokio::test]
async fn every_edge_field_round_trips() {
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    let seed = body_of(&store, ANALYSIS).await;
    let forward = link(&seed, "research", "verdict");
    assert_eq!(forward.kind, EdgeKind::Next);
    let check = |body: &GraphBody, from: &str, to: &str, kind, predicate: Option<&str>, limit| {
        let edge = link(body, from, to);
        assert_eq!(
            (
                edge.kind,
                edge.predicate.map(|predicate| predicate.0.to_string()),
                edge.loop_limit
            ),
            (kind, predicate.map(str::to_owned), limit),
            "{from} -> {to}"
        );
    };

    // `if` with its predicate.
    edit_link(&bench, &mut section, "research", 0);
    press(&bench, &mut section, &["l", "tab"]);
    type_in(&bench, &mut section, r#"{"verify":"failed"}"#);
    saved(&bench, &mut section, &backend).await;
    let body = body_of(&store, ANALYSIS).await;
    check(
        &body,
        "research",
        "verdict",
        EdgeKind::If,
        Some(r#"{"verify":"failed"}"#),
        None,
    );
    assert_eq!(
        link(&body, "research", "verdict").id,
        forward.id,
        "the same link"
    );

    // `asks you`, a loop limit set, changed, cleared.
    edit_link(&bench, &mut section, "research", 0);
    press(&bench, &mut section, &["l", "tab"]);
    type_in(&bench, &mut section, "2");
    saved(&bench, &mut section, &backend).await;
    check(
        &body_of(&store, ANALYSIS).await,
        "research",
        "verdict",
        EdgeKind::AsksYou,
        None,
        Some(2),
    );
    edit_link(&bench, &mut section, "research", 0);
    press(&bench, &mut section, &["tab", "backspace"]);
    type_in(&bench, &mut section, "5");
    saved(&bench, &mut section, &backend).await;
    check(
        &body_of(&store, ANALYSIS).await,
        "research",
        "verdict",
        EdgeKind::AsksYou,
        None,
        Some(5),
    );
    edit_link(&bench, &mut section, "research", 0);
    press(&bench, &mut section, &["tab", "backspace"]);
    saved(&bench, &mut section, &backend).await;
    check(
        &body_of(&store, ANALYSIS).await,
        "research",
        "verdict",
        EdgeKind::AsksYou,
        None,
        None,
    );

    // Back to `next`.
    edit_link(&bench, &mut section, "research", 0);
    press(&bench, &mut section, &["h", "h"]);
    saved(&bench, &mut section, &backend).await;
    check(
        &body_of(&store, ANALYSIS).await,
        "research",
        "verdict",
        EdgeKind::Next,
        None,
        None,
    );

    // A back link: `changes requested` at most 3, then `fails` at most 1.
    bench_select(&bench, &mut section, "verdict");
    press(&bench, &mut section, &["c", "k", "enter"]);
    assert!(
        bench
            .render_section(&section, 100)
            .contains("link verdict \u{2192} research"),
        "{}",
        bench.render_section(&section, 100)
    );
    press(&bench, &mut section, &["l", "l", "l", "tab"]);
    type_in(&bench, &mut section, "3");
    saved(&bench, &mut section, &backend).await;
    let body = body_of(&store, ANALYSIS).await;
    check(
        &body,
        "verdict",
        "research",
        EdgeKind::ChangesRequested,
        None,
        Some(3),
    );
    check(&body, "research", "verdict", EdgeKind::Next, None, None);
    edit_link(&bench, &mut section, "verdict", 0);
    press(&bench, &mut section, &["l", "tab", "backspace"]);
    type_in(&bench, &mut section, "1");
    saved(&bench, &mut section, &backend).await;
    let body = body_of(&store, ANALYSIS).await;
    check(&body, "verdict", "research", EdgeKind::Fails, None, Some(1));
    check(&body, "research", "verdict", EdgeKind::Next, None, None);
    assert_eq!(body.edges.len(), 2, "{:?}", body.edges);
    assert_eq!(shape(&body).nodes, shape(&seed).nodes, "no step moved");
}

/// Renames the selected step by appending `suffix`: one write, answered.
async fn rename(bench: &SectionBench, section: &mut KindsSection, backend: &Backend, suffix: &str) {
    bench.key(section, "e");
    bench.key(section, "end");
    type_in(bench, section, suffix);
    saved(bench, section, backend).await;
}

/// D4, D6: undo sends the previous body as a new write, redo the next; every one moves the
/// revision on.
#[tokio::test]
async fn undo_and_redo_across_three_edits() {
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    let seed = body_of(&store, ANALYSIS).await;
    for suffix in ["1", "2", "3"] {
        rename(&bench, &mut section, &backend, suffix).await;
    }
    let third = body_of(&store, ANALYSIS).await;
    assert_eq!(step(&third, "research123"), step(&seed, "research"));
    assert_eq!(third.graph.revision, seed.graph.revision + 3);

    for _ in 0..3 {
        bench.key(&mut section, "u");
        let served = bench_round(&bench, &mut section, &backend).await;
        assert_eq!(served.len(), 1, "one write per undo: {served:?}");
    }
    let undone = body_of(&store, ANALYSIS).await;
    assert_eq!(shape(&undone), shape(&seed), "the seed again");
    assert_eq!(undone.graph.revision, seed.graph.revision + 6);
    bench.key(&mut section, "u");
    assert!(bench_round(&bench, &mut section, &backend).await.is_empty());
    assert!(
        bench
            .render_section(&section, 100)
            .contains("nothing to undo"),
        "{}",
        bench.render_section(&section, 100)
    );

    for _ in 0..3 {
        bench.key(&mut section, "U");
        let served = bench_round(&bench, &mut section, &backend).await;
        assert_eq!(served.len(), 1, "one write per redo: {served:?}");
    }
    let redone = body_of(&store, ANALYSIS).await;
    assert_eq!(shape(&redone), shape(&third), "after edit 3 again");
    assert_eq!(redone.graph.revision, seed.graph.revision + 9);
}

/// D4: a write that meets another writer's revision adopts the stored body, says so, and the
/// undo history goes with it.
#[tokio::test]
async fn a_stale_reply_clears_the_stacks() {
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    rename(&bench, &mut section, &backend, "1").await;
    rename(&bench, &mut section, &backend, "2").await;

    // Another writer, straight to the store.
    let mut foreign = body_of(&store, ANALYSIS).await;
    let verdict = foreign
        .nodes
        .iter_mut()
        .find(|node| node.phase.name == "verdict")
        .expect("verdict");
    "decide".clone_into(&mut verdict.phase.name);
    "decide".clone_into(&mut verdict.phase.output_kind);
    let revision = foreign.graph.revision;
    let CasOutcome::Applied(foreign) = store
        .replace_graph(ANALYSIS, revision, foreign)
        .await
        .expect("the foreign write lands")
    else {
        panic!("the foreign write applies");
    };

    bench.key(&mut section, "e");
    type_in(&bench, &mut section, "3");
    bench.key(&mut section, "enter");
    let served = bench_round(&bench, &mut section, &backend).await;
    assert_eq!(served.len(), 1, "{served:?}");
    let frame = bench.render_section(&section, 100);
    assert!(
        flat(&frame).contains("changed elsewhere, reloaded; undo history cleared"),
        "{frame}"
    );
    assert!(
        frame.contains("decide"),
        "the stored body on screen: {frame}"
    );
    assert_eq!(shape(&body_of(&store, ANALYSIS).await), shape(&foreign));
    assert_eq!(
        body_of(&store, ANALYSIS).await.graph.revision,
        foreign.graph.revision
    );

    bench.key(&mut section, "u");
    assert!(bench_round(&bench, &mut section, &backend).await.is_empty());
    assert!(
        bench
            .render_section(&section, 100)
            .contains("nothing to undo"),
        "{}",
        bench.render_section(&section, 100)
    );
    assert_eq!(shape(&body_of(&store, ANALYSIS).await), shape(&foreign));
}

/// D5: a body the store would refuse never leaves; the editor shows the store's own sentence.
#[tokio::test]
async fn a_refused_body_never_sends_a_request() {
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    let seed = body_of(&store, ANALYSIS).await;
    let refused = |bench: &SectionBench, section: &mut KindsSection, sentence: &str| {
        bench.key(section, "enter");
        let asked: Vec<Action> = bench.drained();
        assert!(
            !asked
                .iter()
                .any(|action| matches!(action, Action::Store(StoreRequest::ReplaceGraph { .. }))),
            "nothing sent: {asked:?}"
        );
        let frame = bench.render_section(section, 100);
        assert!(
            flat(&frame).contains(&flat(sentence)),
            "`{sentence}`: {frame}"
        );
        bench.key(section, "esc");
    };

    // A reserved name.
    bench.key(&mut section, "e");
    press(&bench, &mut section, &["end"]);
    press(&bench, &mut section, &["backspace"; 12]);
    type_in(&bench, &mut section, "judge");
    refused(&bench, &mut section, &reserved_phase_name("judge"));

    // A forward `fails`.
    bench_select(&bench, &mut section, "research");
    press(&bench, &mut section, &["c", "j", "enter"]);
    assert_eq!(stack_of(&section), Some(views::GRAPH_EDITOR_EDGE_FORM));
    press(&bench, &mut section, &["l", "l", "l", "l", "tab"]);
    type_in(&bench, &mut section, "1");
    refused(
        &bench,
        &mut section,
        &loop_edge_runs_forward(EdgeKind::Fails, "research", "verdict", ANALYSIS),
    );

    // `if` outside the closed set.
    edit_link(&bench, &mut section, "research", 0);
    press(&bench, &mut section, &["l", "tab"]);
    type_in(&bench, &mut section, r#"{"bogus":1}"#);
    let why = IfPredicate::parse(&serde_json::json!({"bogus": 1}))
        .expect_err("outside the closed set")
        .to_string();
    refused(&bench, &mut section, &why);

    assert!(bench_round(&bench, &mut section, &backend).await.is_empty());
    let stored = body_of(&store, ANALYSIS).await;
    assert_eq!(
        stored.graph.revision, seed.graph.revision,
        "nothing written"
    );
    assert_eq!(shape(&stored), shape(&seed));
}

/// D12, PRD "0 hardcoded keys": a key file that rebinds the editor's `add` and the links list's
/// `delete` drives the same add and link delete, and the old chords do nothing. With
/// `tests/no_hardcoded_keys.rs` this covers the metric.
#[tokio::test]
async fn the_editor_keys_are_catalogue_keys() {
    let keys =
        load_str("[graph_editor]\nadd = [\"A\"]\n\n[graph_editor.links]\ndelete = [\"D\"]\n")
            .expect("the key file loads");
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, Some(keys)).await;

    bench.key(&mut section, "a");
    assert_eq!(
        stack_of(&section),
        Some(views::GRAPH_EDITOR),
        "`a` adds nothing now"
    );
    bench.key(&mut section, "A");
    assert_eq!(stack_of(&section), Some(views::GRAPH_EDITOR_NODE_FORM));
    type_in(&bench, &mut section, "probe");
    saved(&bench, &mut section, &backend).await;
    let body = body_of(&store, ANALYSIS).await;
    let probe = link(&body, "probe", "verdict");
    assert_eq!(link(&body, "research", "probe").kind, EdgeKind::Next);

    // probe is selected: its links list, `→ verdict · next` first.
    bench.key(&mut section, "i");
    assert_eq!(stack_of(&section), Some(views::GRAPH_EDITOR_LINKS));
    for chord in ["x", "d"] {
        bench.key(&mut section, chord);
        assert!(
            bench_round(&bench, &mut section, &backend).await.is_empty(),
            "`{chord}` deletes nothing now"
        );
    }
    bench.key(&mut section, "D");
    let served = bench_round(&bench, &mut section, &backend).await;
    assert!(
        matches!(served.as_slice(), [StoreRequest::ReplaceGraph { .. }, ..]),
        "{served:?}"
    );
    let body = body_of(&store, ANALYSIS).await;
    assert!(
        body.edges.iter().all(|edge| edge.id != probe.id),
        "the link is gone: {:?}",
        body.edges
    );
    assert_eq!(body.edges.len(), 1, "{:?}", body.edges);
}
