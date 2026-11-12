//! MOD-141 milestone 1, T8: the step-graph editor end to end (PRD metrics).
//!
//! Two drivers. The reference graph is built through the whole shell ([`Harness`]): every key
//! goes through `App::on_key`, every `ReplaceGraph` really travels the store worker
//! (`store_worker::serve` over the memory backend) and its reply comes back through the shell's
//! fan-out. The table-driven cases use a [`SectionBench`] and [`bench_round`], the `tests/kinds.rs`
//! worker-plus-section pattern without a shell. `MemStore` clones share their state, so every
//! assertion reads the store the requests were served from.
//!
//! MOD-141 milestone 2, T9: the same drivers over the M2 node fields (the deadline, the palette
//! flag, the split and its rows, the default parts through the part form), the compete switch,
//! `htui_orch`'s resolve over the saved body, and the header's limits line.
//!
//! MOD-141 milestone 3, T4: the editor's entry point, [`GraphEditorHost`], driven on its own over
//! a [`SectionBench`]'s context: a graph of every scope opened by id, edited, saved through the
//! store worker and resolved; a stale save; an unknown id. And, through the shell, a kind line
//! that names its workspace and a bare graph name two workspaces share, refused.
#![cfg(feature = "testkit")]

use std::collections::BTreeMap;

use chrono::DateTime;
use htui::app::{Action, Handled};
use htui::keys::{Act, KeyChord, Keys, Stack, load_str, views};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::graph_editor::{EditorKey, GraphEditorHost};
use htui::ui::tabs::settings::{
    AgentsSection, HierarchySection, KindsSection, SettingsSection, SettingsTab,
};
use htui_core::fixtures::ids;
use htui_core::model::predicate::IfPredicate;
use htui_core::model::{
    AgentId, CommandQueue, EdgeId, EdgeKind, Gate, GraphBody, GraphScope, GraphSnapshot, Isolation,
    ItemId, ItemKindId, ItemPatch, NewItemKind, NewStepGraph, NodeBody, NodeRole, PersonaId,
    PhaseAgent, PhaseId, PhasePart, PlanScope, RunMode, SnapshotPart, SnapshotPhase, Split,
    StepGraphEdge, StepGraphId, WorkspaceProject,
};
use htui_core::prompt::SettingKey;
use htui_core::store::{
    CasOutcome, MemStore, SettingRung, UpdateOutcome, WorkerStore, WriteStore,
    loop_edge_runs_forward, reserved_phase_name,
};
use htui_orch::graph::{NotRunnable, ResolveError};
use htui_store::Backend;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use serde_json::json;

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
/// open on [`ANALYSIS`] from its kind row and its agents, templates and settings read.
async fn analysis_bench(
    store: &MemStore,
    keys: Option<Keys>,
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
            [
                StoreRequest::Agents,
                StoreRequest::Templates(_),
                StoreRequest::GraphEditorSettings
            ]
        ),
        "the editor opened and read its lookups and settings: {read:?}"
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

// -------------------------------------------------------------------------------------------
// MOD-141 milestone 2, T9: splits, palette, deadline and the graph header (PRD metrics)
// -------------------------------------------------------------------------------------------

/// M2's step-form rows come after M1's 13 and the candidates or their placeholder (B3, C15):
/// the row `offset` names, counted from the name row of a form opened on `seed`.
fn m2_row(seed: &NodeBody, offset: usize) -> usize {
    13 + seed.agents.len().max(1) + offset
}

// [`m2_row`]'s offsets. The three rows every step shows:
const DEADLINE: usize = 0;
const PALETTE: usize = 1;
const SPLIT: usize = 2;
// A compete step's rows:
const FAN_OUT: usize = 3;
const JUDGE: usize = 4;
// A divide step's rows, its first part (or the parts' placeholder) last:
const PLANNER: usize = 3;
const PLANNER_AGENT: usize = 4;
const INTEGRATE: usize = 5;
const INTEGRATE_AGENT: usize = 6;
const MAX_PARTS: usize = 7;
const PLAN_SCOPE: usize = 8;
const FIRST_PART: usize = 9;

/// Every chord of `chords`, in order.
fn play(bench: &SectionBench, section: &mut KindsSection, chords: &[String]) {
    for chord in chords {
        bench.key(section, chord);
    }
}

/// The seat a form stores for an (agent, model) pair: an empty model is none.
fn seat(pair: &(AgentId, String)) -> (Option<AgentId>, Option<String>) {
    (Some(pair.0), (!pair.1.is_empty()).then(|| pair.1.clone()))
}

/// A default part `key` of `phase` at `position`, nothing else set: what a part form holding
/// only a key keeps.
fn bare_part(phase: PhaseId, position: i32, key: &str) -> PhasePart {
    PhasePart {
        phase_id: phase,
        position,
        key: key.to_owned(),
        title: String::new(),
        brief: String::new(),
        owned_paths: Vec::new(),
        acceptance: None,
        agent_id: None,
        model: None,
        persona_id: None,
    }
}

/// The personas the catalogue hands the editor, in its order.
async fn persona_ids() -> Vec<PersonaId> {
    let StoreReply::Catalogue(snapshot) = serve(
        &Backend::memory(MemStore::demo()),
        &StoreRequest::Catalogue(SectionBench::new().await.ctx().scope.clone()),
    )
    .await
    else {
        panic!("the catalogue reads");
    };
    snapshot.personas.iter().map(|persona| persona.id).collect()
}

/// M2 D5: turns the compete switch on through the prompt section's own `SetSetting` (the `App`
/// rung of `compete_fan_out`), as `Settings > Prompt` would.
async fn compete_on(backend: &Backend) {
    let reply = serve(
        backend,
        &StoreRequest::SetSetting {
            scope: SectionBench::new().await.ctx().scope.clone(),
            rung: SettingRung::App,
            key: SettingKey::CompeteFanOut,
            value: json!(true),
            expected: None,
        },
    )
    .await;
    assert!(
        matches!(reply, StoreReply::PromptSettings(..)),
        "the switch is set on: {reply:?}"
    );
}

/// Changes `name`'s node of [`ANALYSIS`] straight in the store, one `replace_graph`.
async fn store_edit(store: &MemStore, name: &str, edit: impl FnOnce(&mut NodeBody)) {
    let mut body = body_of(store, ANALYSIS).await;
    let node = body
        .nodes
        .iter_mut()
        .find(|node| node.phase.name == name)
        .unwrap_or_else(|| panic!("no step {name}"));
    edit(node);
    let revision = body.graph.revision;
    let outcome = store
        .replace_graph(ANALYSIS, revision, body)
        .await
        .expect("the store takes the edited body");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
}

/// Points the demo item `VULKAN_FEAT_1` at `graph` (`item.step_graph_id`), so resolve reads it.
async fn pin_feat_1(store: &MemStore, graph: StepGraphId) {
    let item = WorkerStore::item(store, ids::VULKAN_FEAT_1)
        .await
        .expect("a memory read never fails")
        .expect("the demo item");
    let outcome = store
        .update_item(
            ids::VULKAN_FEAT_1,
            item.version,
            ItemPatch {
                step_graph_id: Some(Some(graph)),
                author_id: ids::USER,
                reason: "edited".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await
        .expect("a memory write never fails");
    assert!(matches!(outcome, UpdateOutcome::Updated(_)), "{outcome:?}");
}

/// What the orchestrator's own resolver freezes for `VULKAN_FEAT_1` over the store's
/// `app_setting` rows, or its refusal.
async fn resolved(store: &MemStore) -> Result<GraphSnapshot, ResolveError> {
    let item = WorkerStore::item(store, ids::VULKAN_FEAT_1)
        .await
        .expect("a memory read never fails")
        .expect("the demo item");
    let app = store
        .app_settings()
        .await
        .expect("a memory read never fails");
    htui_orch::graph::resolve(store, store, &item, RunMode::Manual, &app, None, ids::BOX)
        .await
        .map(|resolved| resolved.snapshot)
}

/// Gives research and verdict one candidate each, the first (agent, model) pair, one write
/// apiece. The analysis steps have none, and the box's two enabled agents leave the candidate
/// chain's third rung ambiguous, so resolve needs them.
async fn with_candidates(bench: &SectionBench, section: &mut KindsSection, backend: &Backend) {
    for name in ["research", "verdict"] {
        bench_select(bench, section, name);
        press(bench, section, &["e", "alt-n"]);
        saved(bench, section, backend).await;
    }
}

/// The node of `body` with phase id `id`.
#[track_caller]
fn node_of(body: &GraphBody, id: PhaseId) -> &NodeBody {
    body.nodes
        .iter()
        .find(|node| node.phase.id == id)
        .unwrap_or_else(|| panic!("no step {id}: {body:?}"))
}

/// The resolved phase keyed `name`.
#[track_caller]
fn frozen<'a>(snapshot: &'a GraphSnapshot, name: &str) -> &'a SnapshotPhase {
    snapshot
        .phases
        .iter()
        .find(|phase| phase.name == name)
        .unwrap_or_else(|| panic!("no phase {name}: {:?}", snapshot.phases))
}

/// The word the frame's `label` choice row shows between `‹` and `›`.
fn shown_choice(frame: &str, label: &str) -> Option<String> {
    frame.lines().find_map(|line| {
        let rest = &line[line.find(label)? + label.len()..];
        let open = rest.find('\u{2039}')? + '\u{2039}'.len_utf8();
        let close = rest[open..].find('\u{203a}')? + open;
        Some(rest[open..close].trim().to_owned())
    })
}

/// An edit of one node: how a case prepares it, or how it expects it stored.
type NodeEdit = Box<dyn Fn(&mut NodeBody)>;

/// One M2 step-form case: the store's research as `prepare` leaves it, the keys from the form's
/// first row, and how the stored node differs from the prepared one.
struct M2Case {
    /// The column, for messages.
    column: &'static str,
    /// The compete switch is on before the editor opens.
    compete: bool,
    /// Research as the store holds it before the editor opens; `None` keeps the seed's.
    prepare: Option<NodeEdit>,
    /// The chords after `e`; `"SAVE"` saves and lets the store answer, mid-case.
    chords: Vec<String>,
    /// The prepared node, changed as the case expects the mid-case `"SAVE"` to store it; `None`
    /// when the chords hold no `"SAVE"`.
    mid: Option<NodeEdit>,
    /// The prepared node, changed as the case expects it stored.
    expect: NodeEdit,
}

/// `chords` with `then` after them.
fn joined(mut chords: Vec<String>, then: Vec<String>) -> Vec<String> {
    chords.extend(then);
    chords
}

/// PRD metric "every M2 node field round-trips": the deadline, the palette flag, the split and
/// both splits' rows (compete with the switch on; divide's planner, integrate, part cap and plan
/// scope), and the default parts through the part form (add, edit every column, reorder,
/// remove), each edited with keys, saved through the store worker and read back from
/// `graph_body`; every other column of the graph unchanged, a divide step's own columns and
/// parts included when only another column is typed.
#[tokio::test]
async fn every_m2_node_field_round_trips() {
    let seed = body_of(&MemStore::demo(), ANALYSIS).await;
    let research = step(&seed, "research");
    let row = |offset| m2_row(node_of(&seed, research), offset);
    let pairs = agent_pairs(&Backend::memory(MemStore::demo())).await;
    assert!(!pairs.is_empty(), "a pair to choose");
    let first = pairs[0].clone();
    let last = pairs[pairs.len() - 1].clone();
    let personas = persona_ids().await;
    let persona = *personas.first().expect("a seed persona");
    let other_persona = *personas.last().expect("a seed persona");

    let cases = vec![
        M2Case {
            column: "deadline: set",
            compete: false,
            prepare: None,
            chords: retyped(row(DEADLINE), "900"),
            mid: None,
            expect: Box::new(|node| node.phase.deadline_seconds = Some(900)),
        },
        M2Case {
            column: "deadline: cleared",
            compete: false,
            prepare: Some(Box::new(|node| node.phase.deadline_seconds = Some(60))),
            chords: retyped(row(DEADLINE), ""),
            mid: None,
            expect: Box::new(|node| node.phase.deadline_seconds = None),
        },
        M2Case {
            column: "split: divide",
            compete: false,
            prepare: None,
            chords: at_row(row(SPLIT), &["l"]),
            mid: None,
            expect: Box::new(|node| node.phase.split = Split::Divide),
        },
        M2Case {
            column: "divide: planner persona",
            compete: false,
            prepare: None,
            chords: joined(at_row(row(SPLIT), &["l"]), at_row(PLANNER - SPLIT, &["l"])),
            mid: None,
            expect: Box::new(move |node| {
                node.phase.split = Split::Divide;
                node.phase.planner_persona_id = Some(persona);
            }),
        },
        M2Case {
            column: "divide: planner agent",
            compete: false,
            prepare: None,
            chords: joined(
                at_row(row(SPLIT), &["l"]),
                at_row(PLANNER_AGENT - SPLIT, &["l"]),
            ),
            mid: None,
            expect: Box::new({
                let first = first.clone();
                move |node| {
                    node.phase.split = Split::Divide;
                    (node.phase.planner_agent_id, node.phase.planner_model) = seat(&first);
                }
            }),
        },
        M2Case {
            column: "divide: integrate persona",
            compete: false,
            prepare: None,
            chords: joined(
                at_row(row(SPLIT), &["l"]),
                at_row(INTEGRATE - SPLIT, &["l"]),
            ),
            mid: None,
            expect: Box::new(move |node| {
                node.phase.split = Split::Divide;
                node.phase.integrate_persona_id = Some(persona);
            }),
        },
        M2Case {
            column: "divide: integrate agent",
            compete: false,
            prepare: None,
            chords: joined(
                at_row(row(SPLIT), &["l"]),
                at_row(INTEGRATE_AGENT - SPLIT, &["l"]),
            ),
            mid: None,
            expect: Box::new({
                let first = first.clone();
                move |node| {
                    node.phase.split = Split::Divide;
                    (node.phase.integrate_agent_id, node.phase.integrate_model) = seat(&first);
                }
            }),
        },
        M2Case {
            column: "divide: max parts",
            compete: false,
            prepare: None,
            chords: joined(at_row(row(SPLIT), &["l"]), retyped(MAX_PARTS - SPLIT, "3")),
            mid: None,
            expect: Box::new(|node| {
                node.phase.split = Split::Divide;
                node.phase.max_parts = Some(3);
            }),
        },
        M2Case {
            column: "divide: plan scope",
            compete: false,
            prepare: None,
            chords: joined(
                at_row(row(SPLIT), &["l"]),
                at_row(PLAN_SCOPE - SPLIT, &["l"]),
            ),
            mid: None,
            expect: Box::new(|node| {
                node.phase.split = Split::Divide;
                node.phase.plan_scope = Some(PlanScope::Parts);
            }),
        },
        M2Case {
            column: "parts: one, every column through the part form",
            compete: false,
            prepare: None,
            // A candidate first (focus moves to it), so the part's agent is one of the step's.
            chords: {
                let mut chords = at_row(0, &["alt-n"]);
                chords.extend(at_row(row(SPLIT) - 13, &["l"]));
                chords.extend(at_row(FIRST_PART - SPLIT, &["alt-n"]));
                for (at, text) in [
                    "ui",
                    "The UI",
                    "draw it",
                    "src/ui/**, docs/ui.md",
                    "cargo test",
                ]
                .iter()
                .enumerate()
                {
                    if at > 0 {
                        chords.push("tab".to_owned());
                    }
                    chords.extend(text.chars().map(|character| match character {
                        ' ' => "space".to_owned(),
                        other => other.to_string(),
                    }));
                }
                // The agent row: the step's one candidate; the persona row: the first persona.
                chords.extend(at_row(1, &["l", "tab", "l", "enter"]));
                chords
            },
            mid: None,
            expect: Box::new({
                let first = first.clone();
                move |node| {
                    node.agents = candidates(research, &[&first]);
                    node.phase.split = Split::Divide;
                    let (agent_id, model) = seat(&first);
                    node.parts = vec![PhasePart {
                        title: "The UI".to_owned(),
                        brief: "draw it".to_owned(),
                        owned_paths: vec!["src/ui/**".to_owned(), "docs/ui.md".to_owned()],
                        acceptance: Some("cargo test".to_owned()),
                        agent_id,
                        model,
                        persona_id: Some(persona),
                        ..bare_part(research, 0, "ui")
                    }];
                }
            }),
        },
        M2Case {
            column: "parts: a second, then reordered",
            compete: false,
            prepare: None,
            chords: {
                let mut chords = at_row(row(SPLIT), &["l"]);
                chords.extend(at_row(FIRST_PART - SPLIT, &["alt-n", "u", "i", "enter"]));
                chords.extend(
                    ["alt-n", "s", "t", "o", "r", "e", "enter", "alt-up"].map(str::to_owned),
                );
                chords
            },
            mid: None,
            expect: Box::new(move |node| {
                node.phase.split = Split::Divide;
                node.parts = vec![
                    bare_part(research, 0, "store"),
                    bare_part(research, 1, "ui"),
                ];
            }),
        },
        M2Case {
            column: "parts: edited through the part form",
            compete: false,
            prepare: Some(Box::new(move |node| {
                node.phase.split = Split::Divide;
                node.parts = vec![
                    bare_part(research, 0, "ui"),
                    bare_part(research, 1, "store"),
                ];
            })),
            // The second part's title, typed in the form `alt-e` opens.
            chords: at_row(row(FIRST_PART) + 1, &["alt-e", "tab", "D", "B", "enter"]),
            mid: None,
            expect: Box::new(move |node| {
                node.parts[1].title = "DB".to_owned();
            }),
        },
        M2Case {
            column: "parts: remove",
            compete: false,
            prepare: None,
            // Two saved, then the second removed in a second write.
            chords: {
                let mut chords = at_row(row(SPLIT), &["l"]);
                chords.extend(at_row(FIRST_PART - SPLIT, &["alt-n", "u", "i", "enter"]));
                chords.extend(["alt-n", "d", "b", "enter", "SAVE", "e"].map(str::to_owned));
                chords.extend(at_row(row(FIRST_PART) + 1, &["alt-d"]));
                chords
            },
            // The first write stores both parts, so the second removes a stored one.
            mid: Some(Box::new(move |node| {
                node.phase.split = Split::Divide;
                node.parts = vec![bare_part(research, 0, "ui"), bare_part(research, 1, "db")];
            })),
            expect: Box::new(move |node| {
                node.phase.split = Split::Divide;
                node.parts = vec![bare_part(research, 0, "ui")];
            }),
        },
        M2Case {
            column: "split: divide back to none",
            compete: false,
            prepare: Some(Box::new({
                let first = first.clone();
                move |node| {
                    let phase = &mut node.phase;
                    phase.split = Split::Divide;
                    phase.planner_persona_id = Some(persona);
                    (phase.planner_agent_id, phase.planner_model) = seat(&first);
                    phase.integrate_persona_id = Some(persona);
                    (phase.integrate_agent_id, phase.integrate_model) = seat(&first);
                    phase.max_parts = Some(2);
                    phase.plan_scope = Some(PlanScope::Parts);
                    node.parts = vec![bare_part(research, 0, "ui")];
                }
            })),
            chords: at_row(row(SPLIT), &["h"]),
            mid: None,
            // The seven divide columns, the parts and the plan scope go (D7).
            expect: Box::new(|node| {
                let phase = &mut node.phase;
                phase.split = Split::None;
                phase.planner_persona_id = None;
                phase.planner_agent_id = None;
                phase.planner_model = None;
                phase.integrate_persona_id = None;
                phase.integrate_agent_id = None;
                phase.integrate_model = None;
                phase.max_parts = None;
                phase.plan_scope = None;
                node.parts = Vec::new();
            }),
        },
        M2Case {
            column: "divide: kept through an edit of another column",
            compete: false,
            prepare: Some(Box::new({
                let (planner, integrate) = (first.clone(), last.clone());
                move |node| {
                    let phase = &mut node.phase;
                    phase.split = Split::Divide;
                    phase.planner_persona_id = Some(persona);
                    (phase.planner_agent_id, phase.planner_model) = seat(&planner);
                    phase.integrate_persona_id = Some(other_persona);
                    (phase.integrate_agent_id, phase.integrate_model) = seat(&integrate);
                    phase.max_parts = Some(4);
                    phase.plan_scope = Some(PlanScope::Graph);
                    node.parts = vec![
                        PhasePart {
                            title: "The UI".to_owned(),
                            brief: "draw it".to_owned(),
                            owned_paths: vec!["src/ui/**".to_owned()],
                            acceptance: Some("cargo test".to_owned()),
                            persona_id: Some(other_persona),
                            ..bare_part(research, 0, "ui")
                        },
                        bare_part(research, 1, "store"),
                    ];
                }
            })),
            // The deadline is the only column typed: the divide columns and the parts the form
            // opened with are saved as they were.
            chords: retyped(row(DEADLINE), "900"),
            mid: None,
            expect: Box::new(|node| node.phase.deadline_seconds = Some(900)),
        },
        M2Case {
            column: "compete (switch on): fan-out and judge",
            compete: true,
            prepare: None,
            // Back one from none is compete (none, divide, compete), its fan-out set to 2.
            chords: {
                let mut chords = at_row(row(SPLIT), &["h"]);
                chords.extend(retyped(FAN_OUT - SPLIT, "3"));
                chords.extend(at_row(JUDGE - FAN_OUT, &["l"]));
                chords
            },
            mid: None,
            expect: Box::new({
                let first = first.clone();
                move |node| {
                    node.phase.split = Split::Compete;
                    node.phase.fan_out = 3;
                    (node.phase.judge_agent_id, node.phase.judge_model) = seat(&first);
                }
            }),
        },
    ];

    for case in cases {
        let store = MemStore::demo();
        if let Some(prepare) = &case.prepare {
            store_edit(&store, "research", |node| prepare(node)).await;
        }
        if case.compete {
            compete_on(&Backend::memory(store.clone())).await;
        }
        let (bench, mut section, backend) = analysis_bench(&store, None).await;
        let before = body_of(&store, ANALYSIS).await;
        let at = before
            .nodes
            .iter()
            .position(|node| node.phase.id == research)
            .expect("research");
        assert_eq!(
            case.mid.is_some(),
            case.chords.iter().any(|chord| chord == "SAVE"),
            "{}: a mid-case shape is for a mid-case save",
            case.column
        );
        bench.key(&mut section, "e");
        assert_eq!(
            stack_of(&section),
            Some(views::GRAPH_EDITOR_NODE_FORM),
            "{}",
            case.column
        );
        for chord in &case.chords {
            if chord == "SAVE" {
                // Review R1 C-L5: the mid-case save went out and stored what the case says, so
                // what the next write changes is a stored part, not a draft.
                bench.key(&mut section, "enter");
                let served = bench_round(&bench, &mut section, &backend).await;
                let frame = bench.render_section(&section, 100);
                assert!(
                    served
                        .iter()
                        .any(|request| matches!(request, StoreRequest::ReplaceGraph { .. })),
                    "{}: the first save went out: {frame}",
                    case.column
                );
                assert!(frame.contains("saved: "), "{}: {frame}", case.column);
                let mid = case
                    .mid
                    .as_ref()
                    .unwrap_or_else(|| panic!("{}: what the first save stores", case.column));
                let mut expected = before.clone();
                mid(&mut expected.nodes[at]);
                assert_eq!(
                    shape(&body_of(&store, ANALYSIS).await),
                    shape(&expected),
                    "{}: the first save was stored",
                    case.column
                );
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
        (case.expect)(&mut expected.nodes[at]);
        assert_eq!(shape(&stored), shape(&expected), "{}", case.column);
        assert_ne!(
            shape(&stored),
            shape(&before),
            "{}: the column moved",
            case.column
        );
    }

    // Palette, on an unlinked new step: added with it on, then edited off.
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    let before = body_of(&store, ANALYSIS).await;
    bench.key(&mut section, "a");
    type_in(&bench, &mut section, "triage");
    let draft = NodeBody {
        agents: Vec::new(),
        ..node_of(&seed, research).clone()
    };
    play(
        &bench,
        &mut section,
        &at_row(m2_row(&draft, PALETTE), &["l"]),
    );
    saved(&bench, &mut section, &backend).await;
    let added = body_of(&store, ANALYSIS).await;
    let triage = step(&added, "triage");
    assert!(node_of(&added, triage).phase.palette, "palette on");
    assert_eq!(added.edges, before.edges, "a palette step takes no link");

    // The new step is selected: `e` opens it.
    bench.key(&mut section, "e");
    play(
        &bench,
        &mut section,
        &at_row(m2_row(&draft, PALETTE), &["l"]),
    );
    saved(&bench, &mut section, &backend).await;
    let stored = body_of(&store, ANALYSIS).await;
    let mut expected = added.clone();
    expected
        .nodes
        .iter_mut()
        .find(|node| node.phase.id == triage)
        .expect("triage")
        .phase
        .palette = false;
    assert_eq!(shape(&stored), shape(&expected), "palette off");
}

/// PRD metric "compete stays hidden while the switch is off" (M2 D7): a compete step the
/// editor did not make keeps its split, fan-out, judge and deadline byte for byte through an
/// edit, and no step is offered compete. With the switch on, its fan-out and judge edit.
#[tokio::test]
async fn with_the_switch_off_a_compete_node_saves_byte_identical() {
    let store = MemStore::demo();
    let judge = (ids::AGENT_AGY, Some("gemini".to_owned()));
    store_edit(&store, "verdict", |node| {
        node.phase.split = Split::Compete;
        node.phase.fan_out = 2;
        (node.phase.judge_agent_id, node.phase.judge_model) = (Some(judge.0), judge.1.clone());
        node.phase.deadline_seconds = Some(60);
    })
    .await;
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    let before = body_of(&store, ANALYSIS).await;
    let verdict = step(&before, "verdict");

    // The compete rows are shown read-only.
    bench_select(&bench, &mut section, "verdict");
    bench.key(&mut section, "e");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("compete \u{b7} read-only"), "{frame}");
    bench.key(&mut section, "esc");

    // A rename saves every compete column as it was.
    rename(&bench, &mut section, &backend, "2").await;
    let stored = body_of(&store, ANALYSIS).await;
    let mut expected = before.clone();
    expected
        .nodes
        .iter_mut()
        .find(|node| node.phase.id == verdict)
        .expect("verdict")
        .phase
        .name = "verdict2".to_owned();
    assert_eq!(shape(&stored), shape(&expected), "byte for byte");

    // Research's split cycles none and divide only.
    bench_select(&bench, &mut section, "research");
    bench.key(&mut section, "e");
    play(
        &bench,
        &mut section,
        &at_row(
            m2_row(node_of(&before, step(&before, "research")), SPLIT),
            &[],
        ),
    );
    let mut seen = Vec::new();
    for _ in 0..3 {
        bench.key(&mut section, "l");
        let frame = bench.render_section(&section, 100);
        assert!(!frame.contains("\u{2039} compete \u{203a}"), "{frame}");
        seen.push(shown_choice(&frame, "split").unwrap_or_default());
    }
    assert_eq!(seen, ["divide", "none", "divide"]);
    bench.key(&mut section, "esc");
    assert!(bench_round(&bench, &mut section, &backend).await.is_empty());

    // The switch on, a fresh editor: the fan-out and the judge edit.
    compete_on(&backend).await;
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    let renamed = body_of(&store, ANALYSIS).await;
    let pairs = agent_pairs(&backend).await;
    // The judge's choices: the project default, every pair, then the stored judge if no pair is
    // it; `l` steps forward to the first pair.
    let mut options: Vec<(Option<AgentId>, Option<String>)> = vec![(None, None)];
    options.extend(pairs.iter().map(seat));
    let current = (Some(judge.0), judge.1);
    if !options.contains(&current) {
        options.push(current.clone());
    }
    let from = options
        .iter()
        .position(|option| *option == current)
        .expect("listed");
    let presses = (1 + options.len() - from) % options.len();
    bench_select(&bench, &mut section, "verdict2");
    bench.key(&mut section, "e");
    let mut chords = retyped(m2_row(node_of(&renamed, verdict), FAN_OUT), "3");
    chords.extend(at_row(JUDGE - FAN_OUT, &vec!["l"; presses]));
    play(&bench, &mut section, &chords);
    saved(&bench, &mut section, &backend).await;
    let stored = body_of(&store, ANALYSIS).await;
    let mut expected = renamed.clone();
    let node = expected
        .nodes
        .iter_mut()
        .find(|node| node.phase.id == verdict)
        .expect("verdict");
    node.phase.fan_out = 3;
    (node.phase.judge_agent_id, node.phase.judge_model) = seat(&pairs[0]);
    assert_eq!(shape(&stored), shape(&expected), "fan-out and judge edited");
}

/// PRD metric "every saved divide that resolve accepts resolves": a divide step with one part,
/// made with keys and saved, freezes through the orchestrator's own resolver over the same store,
/// as a planner node and an integrate node carrying the part; the canvas marks nothing.
#[tokio::test]
async fn a_divide_step_with_one_part_saves_and_resolves() {
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    with_candidates(&bench, &mut section, &backend).await;
    let before = body_of(&store, ANALYSIS).await;
    let research = node_of(&before, step(&before, "research")).clone();

    bench_select(&bench, &mut section, "research");
    bench.key(&mut section, "e");
    let mut chords = at_row(m2_row(&research, SPLIT), &["l"]);
    chords.extend(at_row(
        FIRST_PART - SPLIT,
        &["alt-n", "u", "i", "tab", "tab", "tab"],
    ));
    chords.extend("src/ui/**".chars().map(|character| character.to_string()));
    chords.push("enter".to_owned());
    play(&bench, &mut section, &chords);
    saved(&bench, &mut section, &backend).await;
    let stored = body_of(&store, ANALYSIS).await;
    let divided = node_of(&stored, research.phase.id);
    assert_eq!(divided.phase.split, Split::Divide);
    let part = PhasePart {
        owned_paths: vec!["src/ui/**".to_owned()],
        ..bare_part(research.phase.id, 0, "ui")
    };
    assert_eq!(divided.parts, core::slice::from_ref(&part));
    let frame = bench.render_section(&section, 100);
    assert!(!frame.contains('!'), "no step is marked: {frame}");

    pin_feat_1(&store, ANALYSIS).await;
    let snapshot = resolved(&store)
        .await
        .unwrap_or_else(|refusal| panic!("the divided graph resolves: {refusal}"));
    let nodes: Vec<(&str, NodeRole)> = snapshot
        .phases
        .iter()
        .map(|phase| (phase.name.as_str(), phase.role))
        .collect();
    assert_eq!(
        nodes,
        [
            ("research:plan", NodeRole::Planner),
            ("research", NodeRole::Integrate),
            ("verdict", NodeRole::Step),
        ]
    );
    let frozen_part = SnapshotPart {
        position: part.position,
        key: part.key,
        title: part.title,
        brief: part.brief,
        owned_paths: part.owned_paths,
        acceptance: part.acceptance,
        agent_id: part.agent_id,
        model: part.model,
        persona_id: part.persona_id,
    };
    for name in ["research:plan", "research"] {
        let divide = frozen(&snapshot, name)
            .divide
            .as_ref()
            .unwrap_or_else(|| panic!("{name} carries the divide block"));
        assert_eq!(
            divide.default_parts,
            core::slice::from_ref(&frozen_part),
            "{name}"
        );
    }
}

/// M2 D11 (M-4): a divide step with no part is saved anyway; its form shows resolve's own
/// `NoParts` sentence, the canvas marks it, and resolve refuses it with that same sentence.
#[tokio::test]
async fn a_part_less_divide_shows_no_parts_and_still_saves() {
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    with_candidates(&bench, &mut section, &backend).await;
    let before = body_of(&store, ANALYSIS).await;
    let research = node_of(&before, step(&before, "research")).clone();

    bench_select(&bench, &mut section, "research");
    bench.key(&mut section, "e");
    play(
        &bench,
        &mut section,
        &at_row(m2_row(&research, SPLIT), &["l"]),
    );
    saved(&bench, &mut section, &backend).await;
    let stored = body_of(&store, ANALYSIS).await;
    let divided = node_of(&stored, research.phase.id);
    assert_eq!(divided.phase.split, Split::Divide);
    assert!(divided.parts.is_empty(), "{:?}", divided.parts);

    let sentence = ResolveError::NotYetRunnable {
        node: "research".to_owned(),
        reason: NotRunnable::NoParts,
    }
    .to_string();
    bench.key(&mut section, "e");
    let frame = bench.render_section(&section, 100);
    assert!(flat(&frame).contains(&format!("! {sentence}")), "{frame}");
    bench.key(&mut section, "esc");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("! research"), "the canvas marks it: {frame}");

    pin_feat_1(&store, ANALYSIS).await;
    let refusal = resolved(&store)
        .await
        .expect_err("a part-less divide does not run yet");
    assert_eq!(refusal.to_string(), sentence);
}

/// M2 D3: a deadline and a judge saved in the editor are what the orchestrator freezes: the
/// phase rung answers first.
#[tokio::test]
async fn a_saved_deadline_and_judge_reach_the_resolved_snapshot() {
    let store = MemStore::demo();
    compete_on(&Backend::memory(store.clone())).await;
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    with_candidates(&bench, &mut section, &backend).await;
    let before = body_of(&store, ANALYSIS).await;
    let pairs = agent_pairs(&backend).await;

    bench_select(&bench, &mut section, "research");
    bench.key(&mut section, "e");
    let research = node_of(&before, step(&before, "research"));
    play(
        &bench,
        &mut section,
        &retyped(m2_row(research, DEADLINE), "900"),
    );
    saved(&bench, &mut section, &backend).await;

    bench_select(&bench, &mut section, "verdict");
    bench.key(&mut section, "e");
    let verdict = node_of(&before, step(&before, "verdict"));
    // Back one from none is compete, its fan-out 2; the judge forward from the project default
    // is the first pair.
    let mut chords = at_row(m2_row(verdict, SPLIT), &["h"]);
    chords.extend(at_row(JUDGE - SPLIT, &["l"]));
    play(&bench, &mut section, &chords);
    saved(&bench, &mut section, &backend).await;

    pin_feat_1(&store, ANALYSIS).await;
    let snapshot = resolved(&store)
        .await
        .unwrap_or_else(|refusal| panic!("the graph resolves: {refusal}"));
    assert_eq!(frozen(&snapshot, "research").deadline_seconds, Some(900));
    let verdict = frozen(&snapshot, "verdict");
    assert_ne!(verdict.deadline_seconds, Some(900), "research's alone");
    assert_eq!(verdict.fan_out, 2);
    let judge = verdict.judge.as_ref().expect("the phase's judge");
    assert_eq!(
        (Some(judge.agent_id), judge.model.clone()),
        seat(&pairs[0]),
        "{judge:?}"
    );
}

/// M2 D10: a step saved with palette on is added with no links and takes nothing over; palette
/// on a linked step is refused in words and nothing is written.
#[tokio::test]
async fn a_palette_step_is_added_with_no_edges() {
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench(&store, None).await;
    let before = body_of(&store, ANALYSIS).await;
    let research = node_of(&before, step(&before, "research")).clone();
    let forward = link(&before, "research", "verdict");

    // `a` with research selected; the draft takes research's template.
    bench.key(&mut section, "a");
    type_in(&bench, &mut section, "triage");
    let draft = NodeBody {
        agents: Vec::new(),
        ..research.clone()
    };
    play(
        &bench,
        &mut section,
        &at_row(m2_row(&draft, PALETTE), &["l"]),
    );
    saved(&bench, &mut section, &backend).await;
    let stored = body_of(&store, ANALYSIS).await;
    let triage = node_of(&stored, step(&stored, "triage"));
    assert!(triage.phase.palette);
    assert!(!triage.phase.template_name.is_empty(), "a template");
    assert!(
        stored
            .edges
            .iter()
            .all(|edge| edge.from != triage.phase.id && edge.to != triage.phase.id),
        "{:?}",
        stored.edges
    );
    assert_eq!(link(&stored, "research", "verdict"), forward, "intact");
    assert_eq!(stored.edges.len(), 1, "{:?}", stored.edges);

    // Palette on research, which has its one link. The new step is selected, in the band under
    // verdict: up twice is research (`bench_select`'s cycle never moves up twice in a row).
    press(&bench, &mut section, &["k", "k"]);
    bench_select(&bench, &mut section, "research");
    bench.key(&mut section, "e");
    play(
        &bench,
        &mut section,
        &at_row(m2_row(&research, PALETTE), &["l", "enter"]),
    );
    assert!(bench_round(&bench, &mut section, &backend).await.is_empty());
    let frame = bench.render_section(&section, 100);
    assert!(
        flat(&frame).contains("a palette step has no links; delete its 1 link first"),
        "{frame}"
    );
    assert_eq!(
        body_of(&store, ANALYSIS).await.graph.revision,
        stored.graph.revision,
        "nothing written"
    );
}

/// The section bench opened on [`ANALYSIS`] as [`analysis_bench`] does, its agents and templates
/// read but its settings read held back.
async fn analysis_bench_unread(store: &MemStore) -> (SectionBench, KindsSection, Backend) {
    let backend = Backend::memory(store.clone());
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let scope = bench.ctx().scope.clone();
    let catalogue = serve(&backend, &StoreRequest::Catalogue(scope)).await;
    bench.reply(&mut section, &catalogue);
    let _ = bench_round(&bench, &mut section, &backend).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "enter");
    let asked: Vec<StoreRequest> = bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect();
    assert!(
        asked
            .iter()
            .any(|request| matches!(request, StoreRequest::GraphEditorSettings)),
        "{asked:?}"
    );
    for request in asked
        .iter()
        .filter(|request| !matches!(request, StoreRequest::GraphEditorSettings))
    {
        let reply = serve(&backend, request).await;
        bench.reply(&mut section, &reply);
    }
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));
    (bench, section, backend)
}

/// The header's second row of a section frame.
fn limits_shown(frame: &str) -> &str {
    frame.lines().nth(1).unwrap_or_default().trim_end()
}

/// M2 D13 (M-1), W6: the header's second row is the run limits resolve freezes, read-only:
/// `limits not read yet` until the settings read answers, then the stored values over the code
/// defaults, clipped to its one row a whole segment at a time.
#[tokio::test]
async fn the_header_shows_the_limits() {
    let defaults = "limits \u{b7} fan-out 4 \u{b7} agents 8 \u{b7} plan revisions 3 \u{b7} \
                    parallel 4 \u{b7} child runs 8 \u{b7} run-time changes ask";
    let store = MemStore::demo();
    let (bench, mut section, backend) = analysis_bench_unread(&store).await;
    let frame = bench.render_section(&section, 120);
    assert_eq!(limits_shown(&frame), "limits not read yet", "{frame}");

    let reply = serve(&backend, &StoreRequest::GraphEditorSettings).await;
    bench.reply(&mut section, &reply);
    let frame = bench.render_section(&section, 120);
    assert_eq!(limits_shown(&frame), defaults, "{frame}");
    assert_eq!(limits_shown(&frame).chars().count(), 99);

    // Narrower than the line: whole segments go and ` · …` says so; the row never wraps.
    let frame = bench.render_section(&section, 60);
    assert_eq!(
        limits_shown(&frame),
        "limits \u{b7} fan-out 4 \u{b7} agents 8 \u{b7} plan revisions 3 \u{b7} \u{2026}",
        "{frame}"
    );
    assert!(!frame.contains("run-time changes"), "{frame}");

    let store = MemStore::demo();
    store.set_app_setting("max_fan_out", json!(2));
    let (bench, section, _) = analysis_bench(&store, None).await;
    let frame = bench.render_section(&section, 120);
    assert_eq!(
        limits_shown(&frame),
        defaults.replace("fan-out 4", "fan-out 2"),
        "{frame}"
    );

    // Review R1 C-L4: every bound set, each to a value no other bound and no default holds, so
    // a field drawn under another's label fails.
    let store = MemStore::demo();
    for (key, value) in [
        ("max_fan_out", json!(2)),
        ("max_agents_per_run", json!(5)),
        ("max_plan_revisions", json!(1)),
        ("max_parallel_sessions", json!(7)),
        ("max_child_runs", json!(6)),
        ("runtime_changes", json!("off")),
    ] {
        store.set_app_setting(key, value);
    }
    let (bench, section, _) = analysis_bench(&store, None).await;
    let frame = bench.render_section(&section, 120);
    assert_eq!(
        limits_shown(&frame),
        "limits \u{b7} fan-out 2 \u{b7} agents 5 \u{b7} plan revisions 1 \u{b7} parallel 7 \
         \u{b7} child runs 6 \u{b7} run-time changes off",
        "{frame}"
    );
}

// -------------------------------------------------------------------------------------------
// The entry point (MOD-141 M3 T4)
// -------------------------------------------------------------------------------------------

/// One chord to the host, written as `KeyChord::parse` reads it.
fn host_key(bench: &SectionBench, host: &mut GraphEditorHost, chord: &str) -> EditorKey {
    let parsed = KeyChord::parse(chord).unwrap_or_else(|| panic!("`{chord}` is not a chord"));
    let mut ctx = bench.ctx();
    host.on_key(parsed.to_event(), &mut ctx)
}

/// Every chord in order, to the host.
fn host_press(bench: &SectionBench, host: &mut GraphEditorHost, chords: &[&str]) {
    for chord in chords {
        host_key(bench, host, chord);
    }
}

/// Every character of `text` as its own key, to the host (a blank is `space`).
fn host_type(bench: &SectionBench, host: &mut GraphEditorHost, text: &str) {
    for character in text.chars() {
        if character == ' ' {
            host_key(bench, host, "space");
        } else {
            host_key(bench, host, &character.to_string());
        }
    }
}

/// Serves `first`, then everything the host asks for until it asks nothing more, each with the
/// store worker's own `serve` over `backend`; every reply must be the host's. The requests served
/// and their replies, in order.
async fn host_round(
    bench: &SectionBench,
    host: &mut GraphEditorHost,
    backend: &Backend,
    first: Vec<StoreRequest>,
) -> Vec<(StoreRequest, StoreReply)> {
    let mut served = Vec::new();
    let mut requests = first;
    loop {
        requests.extend(
            bench
                .drained()
                .into_iter()
                .filter_map(|action| match action {
                    Action::Store(request) => Some(request),
                    _ => None,
                }),
        );
        if requests.is_empty() {
            return served;
        }
        for request in core::mem::take(&mut requests) {
            let reply = serve(backend, &request).await;
            let mut ctx = bench.ctx();
            assert!(
                host.on_reply(&reply, &mut ctx),
                "{request:?} answered {reply:?}, which the host did not take"
            );
            served.push((request, reply));
        }
    }
}

/// The host drawn `width` columns wide, 30 rows high, as text.
fn host_frame(bench: &SectionBench, host: &GraphEditorHost, width: u16) -> String {
    let area = Rect::new(0, 0, width, 30);
    let mut terminal = Terminal::with_options(
        TestBackend::new(width, 30),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("a test terminal");
    let ctx = bench.ctx();
    terminal
        .draw(|frame| host.render(frame, frame.area(), &ctx))
        .expect("the host draws");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Selects step `name` with the editor's direction keys, `e`'s form title the probe (as
/// [`select`]).
#[track_caller]
fn host_select(bench: &SectionBench, host: &mut GraphEditorHost, name: &str) {
    let title = format!("step {name}");
    for chord in ["h", "k", "l", "j"].iter().cycle().take(24) {
        host_key(bench, host, "e");
        let frame = host_frame(bench, host, 100);
        host_key(bench, host, "esc");
        if frame.contains(&title) {
            return;
        }
        host_key(bench, host, chord);
    }
    panic!(
        "no direction selects `{name}`: {}",
        host_frame(bench, host, 100)
    );
}

/// The chord `graph_editor.close` is bound to in the compiled keys.
fn close_chord() -> String {
    Keys::compiled()
        .label(views::GRAPH_EDITOR, Act::GraphEditorClose)
        .expect("close is bound")
}

/// `GraphEditorHost::open_by_id(graph)`, its two reads and then the editor's three served
/// through the store worker: the editor open on `graph`.
async fn opened_by_id(
    bench: &SectionBench,
    backend: &Backend,
    graph: StepGraphId,
) -> GraphEditorHost {
    let (mut host, requests) = GraphEditorHost::open_by_id(graph);
    assert!(
        matches!(
            requests.as_slice(),
            [StoreRequest::GraphBody(asked), StoreRequest::Personas] if *asked == graph
        ),
        "the body and the personas: {requests:?}"
    );
    let served = host_round(bench, &mut host, backend, requests).await;
    let asked: Vec<&StoreRequest> = served.iter().map(|(request, _)| request).collect();
    assert!(
        matches!(
            asked.as_slice(),
            [
                StoreRequest::GraphBody(_),
                StoreRequest::Personas,
                StoreRequest::Agents,
                StoreRequest::Templates(_),
                StoreRequest::GraphEditorSettings,
            ]
        ),
        "the body, then the editor's three reads: {asked:?}"
    );
    assert!(
        host.editor_mut().is_some(),
        "the editor is open: {}",
        host_frame(bench, &host, 100)
    );
    assert_eq!(host.graph(), graph);
    host
}

/// A new graph of `scope` named `name`, holding [`ANALYSIS`]'s steps and links under new ids,
/// written through the store (a created graph has no steps to edit).
async fn analysis_copy(store: &MemStore, scope: GraphScope, name: &str) -> StepGraphId {
    let graph = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            scope,
            name: name.to_owned(),
            description: String::new(),
            is_override: false,
        })
        .await
        .expect("a new graph")
        .id;
    let mut body = body_of(store, graph).await;
    let source = body_of(store, ANALYSIS).await;
    let renamed: Vec<(PhaseId, PhaseId)> = source
        .nodes
        .iter()
        .map(|node| (node.phase.id, PhaseId::new()))
        .collect();
    let new_id = |old: PhaseId| {
        renamed
            .iter()
            .find(|(from, _)| *from == old)
            .map(|(_, to)| *to)
            .expect("every link's ends are steps")
    };
    body.nodes = source
        .nodes
        .iter()
        .map(|node| {
            let mut node = node.clone();
            let id = new_id(node.phase.id);
            node.phase.id = id;
            node.phase.graph_id = graph;
            for agent in &mut node.agents {
                agent.phase_id = id;
            }
            for part in &mut node.parts {
                part.phase_id = id;
            }
            node
        })
        .collect();
    body.edges = source
        .edges
        .iter()
        .map(|edge| StepGraphEdge {
            id: EdgeId::new(),
            graph_id: graph,
            from: new_id(edge.from),
            to: new_id(edge.to),
            ..edge.clone()
        })
        .collect();
    let revision = body.graph.revision;
    let outcome = store
        .replace_graph(graph, revision, body)
        .await
        .expect("the store takes the copy");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
    graph
}

/// Gives every step of `graph` without one the candidate `pair`, one store write: the box's two
/// enabled agents leave the candidate chain's third rung ambiguous, so resolve needs them
/// ([`with_candidates`]).
async fn with_one_candidate(store: &MemStore, graph: StepGraphId, pair: &(AgentId, String)) {
    let mut body = body_of(store, graph).await;
    for node in &mut body.nodes {
        if node.agents.is_empty() {
            node.agents = candidates(node.phase.id, &[pair]);
        }
    }
    let revision = body.graph.revision;
    let outcome = store
        .replace_graph(graph, revision, body)
        .await
        .expect("the store takes the candidates");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
}

/// The one `ReplaceGraph` of `served`: it wrote `graph` on `revision - 1` and its answer applied
/// as `revision`.
#[track_caller]
fn applied_once(served: &[(StoreRequest, StoreReply)], graph: StepGraphId, revision: i64) {
    let writes: Vec<&(StoreRequest, StoreReply)> = served
        .iter()
        .filter(|(request, _)| matches!(request, StoreRequest::ReplaceGraph { .. }))
        .collect();
    let [
        (
            StoreRequest::ReplaceGraph {
                graph: written,
                expected_revision,
                ..
            },
            reply,
        ),
    ] = writes.as_slice()
    else {
        panic!("one write: {served:?}");
    };
    assert_eq!(*written, graph);
    assert_eq!(*expected_revision, revision - 1);
    let StoreReply::GraphReplaced {
        outcome: CasOutcome::Applied(body),
        ..
    } = reply
    else {
        panic!("the write applied: {reply:?}");
    };
    assert_eq!(body.graph.id, graph);
    assert_eq!(body.graph.revision, revision);
}

/// MOD-141 M3 D4 (PRD "every scope editable"): the entry point opens a project, a workspace and
/// a library graph by id, takes a new step and a loop back with a limit, saves each write through
/// the store worker, reads the edit back from `graph_body`, and the saved body resolves through
/// the orchestrator's own resolver for `VULKAN_FEAT_1` (the workspace graph lives in Graphics,
/// the item's workspace).
#[tokio::test]
async fn the_entry_point_edits_every_scope_by_id_and_the_save_resolves() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let bench = SectionBench::new().await;
    let pair = agent_pairs(&backend)
        .await
        .into_iter()
        .next()
        .expect("an agent");
    let workspace = analysis_copy(
        &store,
        GraphScope::Workspace(ids::WORKSPACE_GRAPHICS),
        "shared analysis",
    )
    .await;
    let library = analysis_copy(&store, GraphScope::Library, "library analysis").await;
    let close = close_chord();

    for (graph, scope) in [
        (ANALYSIS, GraphScope::Project(ids::PROJECT_VULKAN)),
        (workspace, GraphScope::Workspace(ids::WORKSPACE_GRAPHICS)),
        (library, GraphScope::Library),
    ] {
        with_one_candidate(&store, graph, &pair).await;
        let before = body_of(&store, graph).await;
        assert_eq!(before.graph.scope, scope);
        let mut host = opened_by_id(&bench, &backend, graph).await;
        let frame = host_frame(&bench, &host, 100);
        assert!(
            frame.contains("research") && frame.contains("verdict"),
            "{scope:?}: {frame}"
        );

        // `a` with research selected: `check` between research and verdict, with a candidate.
        host_select(&bench, &mut host, "research");
        host_key(&bench, &mut host, "a");
        host_type(&bench, &mut host, "check");
        host_press(&bench, &mut host, &["alt-n", "enter"]);
        assert!(host.busy(), "{scope:?}: {}", host_frame(&bench, &host, 100));
        let added = host_round(&bench, &mut host, &backend, Vec::new()).await;
        applied_once(&added, graph, before.graph.revision + 1);
        assert!(!host.busy(), "{scope:?}");
        let frame = host_frame(&bench, &host, 100);
        assert!(frame.contains("saved: adding check"), "{scope:?}: {frame}");

        // `c` from check (selected once added) back to research: `changes requested`, at most 3
        // times round.
        host_press(&bench, &mut host, &["c", "k", "enter"]);
        let frame = host_frame(&bench, &host, 100);
        assert!(
            frame.contains("link check \u{2192} research"),
            "{scope:?}: the pick lands on research: {frame}"
        );
        host_press(&bench, &mut host, &["l", "l", "l", "tab"]);
        host_type(&bench, &mut host, "3");
        host_key(&bench, &mut host, "enter");
        let linked = host_round(&bench, &mut host, &backend, Vec::new()).await;
        applied_once(&linked, graph, before.graph.revision + 2);
        let frame = host_frame(&bench, &host, 100);
        assert!(
            frame.contains("saved: linking check \u{2192} research"),
            "{scope:?}: {frame}"
        );

        // Read back through the worker: the step, both links it took over and the loop.
        let StoreReply::GraphBody(Some(reread)) =
            serve(&backend, &StoreRequest::GraphBody(graph)).await
        else {
            panic!("{scope:?}: the graph reads back");
        };
        assert_eq!(reread.graph.scope, scope);
        assert_eq!(reread.graph.revision, before.graph.revision + 2);
        assert_eq!(reread.nodes.len(), 3, "{scope:?}: {reread:?}");
        assert_eq!(
            node_of(&reread, step(&reread, "check")).agents.len(),
            1,
            "{scope:?}: one candidate"
        );
        assert_eq!(
            edges_by_name(&reread),
            vec![
                (
                    "check".to_owned(),
                    "research".to_owned(),
                    EdgeKind::ChangesRequested,
                    None,
                    Some(3),
                ),
                (
                    "check".to_owned(),
                    "verdict".to_owned(),
                    EdgeKind::Next,
                    None,
                    None,
                ),
                (
                    "research".to_owned(),
                    "check".to_owned(),
                    EdgeKind::Next,
                    None,
                    None,
                ),
            ],
            "{scope:?}"
        );

        // The saved body resolves.
        pin_feat_1(&store, graph).await;
        let snapshot = resolved(&store)
            .await
            .unwrap_or_else(|refusal| panic!("{scope:?}: the saved graph resolves: {refusal}"));
        let mut phases: Vec<&str> = snapshot
            .phases
            .iter()
            .map(|phase| phase.name.as_str())
            .collect();
        phases.sort_unstable();
        assert_eq!(phases, ["check", "research", "verdict"], "{scope:?}");
        assert_eq!(snapshot.edges.len(), 3, "{scope:?}: {:?}", snapshot.edges);

        assert_eq!(host_key(&bench, &mut host, &close), EditorKey::Close);
    }
}

/// MOD-141 M3 D4: another writer moves the graph on while the entry point holds it open; the
/// host's save answers `Stale`, the editor adopts the stored body and says so, its undo history
/// goes, and its next write is made on the adopted revision.
#[tokio::test]
async fn a_stale_save_through_the_entry_point_rebases() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let bench = SectionBench::new().await;
    let mut host = opened_by_id(&bench, &backend, ANALYSIS).await;
    let opened = body_of(&store, ANALYSIS).await;

    // Another writer, straight to the store.
    let mut foreign = opened.clone();
    let verdict = foreign
        .nodes
        .iter_mut()
        .find(|node| node.phase.name == "verdict")
        .expect("verdict");
    "decide".clone_into(&mut verdict.phase.name);
    "decide".clone_into(&mut verdict.phase.output_kind);
    let CasOutcome::Applied(foreign) = store
        .replace_graph(ANALYSIS, opened.graph.revision, foreign)
        .await
        .expect("the foreign write lands")
    else {
        panic!("the foreign write applies");
    };

    host_select(&bench, &mut host, "research");
    host_press(&bench, &mut host, &["e", "end"]);
    host_type(&bench, &mut host, "3");
    host_key(&bench, &mut host, "enter");
    let served = host_round(&bench, &mut host, &backend, Vec::new()).await;
    let [
        (
            StoreRequest::ReplaceGraph {
                expected_revision, ..
            },
            StoreReply::GraphReplaced {
                outcome: CasOutcome::Stale(current),
                ..
            },
        ),
    ] = served.as_slice()
    else {
        panic!("one write, answered stale: {served:?}");
    };
    assert_eq!(*expected_revision, opened.graph.revision);
    assert_eq!(shape(current), shape(&foreign));
    assert!(!host.busy());
    let frame = host_frame(&bench, &host, 100);
    assert!(
        flat(&frame).contains("changed elsewhere, reloaded; undo history cleared"),
        "{frame}"
    );
    assert!(
        frame.contains("decide"),
        "the stored body on screen: {frame}"
    );
    assert!(!frame.contains("research3"), "the edit went: {frame}");
    let stored = body_of(&store, ANALYSIS).await;
    assert_eq!(
        shape(&stored),
        shape(&foreign),
        "nothing of the stale write"
    );
    assert_eq!(stored.graph.revision, foreign.graph.revision);

    // The undo history went with it.
    host_key(&bench, &mut host, "u");
    assert!(
        host_round(&bench, &mut host, &backend, Vec::new())
            .await
            .is_empty()
    );
    assert!(
        host_frame(&bench, &host, 100).contains("nothing to undo"),
        "{}",
        host_frame(&bench, &host, 100)
    );

    // The adopted body is the next write's base: it applies on the foreign revision.
    host_select(&bench, &mut host, "research");
    host_press(&bench, &mut host, &["e", "end"]);
    host_type(&bench, &mut host, "4");
    host_key(&bench, &mut host, "enter");
    let served = host_round(&bench, &mut host, &backend, Vec::new()).await;
    applied_once(&served, ANALYSIS, foreign.graph.revision + 1);
    let stored = body_of(&store, ANALYSIS).await;
    let mut names: Vec<&str> = stored
        .nodes
        .iter()
        .map(|node| node.phase.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["decide", "research4"]);
}

/// MOD-141 M3 D4: an id no graph has opens nothing; the host says so in one line, swallows the
/// editor's keys, sends nothing more, and its close key still closes.
#[tokio::test]
async fn an_unknown_graph_id_opens_nothing() {
    let store = MemStore::demo();
    let backend = Backend::memory(store);
    let bench = SectionBench::new().await;
    let missing = StepGraphId::new();
    let (mut host, requests) = GraphEditorHost::open_by_id(missing);
    let served = host_round(&bench, &mut host, &backend, requests).await;
    assert!(
        matches!(
            served.as_slice(),
            [
                (StoreRequest::GraphBody(_), StoreReply::GraphBody(None)),
                (StoreRequest::Personas, StoreReply::Personas(_)),
            ]
        ),
        "the two reads and nothing after: {served:?}"
    );
    assert!(host.editor_mut().is_none(), "nothing opened");
    assert_eq!(host.graph(), missing);
    assert!(!host.busy());
    assert!(host.wants_requests(bench.ctx().scope).is_empty());
    let frame = host_frame(&bench, &host, 60);
    assert_eq!(frame.lines().next(), Some("graph not found"), "{frame}");

    for chord in ["a", "e", "c", "enter"] {
        assert_eq!(
            host_key(&bench, &mut host, chord),
            EditorKey::Handled(Handled::Consumed),
            "{chord}"
        );
    }
    assert!(
        host_round(&bench, &mut host, &backend, Vec::new())
            .await
            .is_empty(),
        "no key sent anything"
    );
    let hint = host.hint_text(Keys::compiled());
    assert!(
        hint.contains(&close_chord()) && hint.contains("close"),
        "{hint}"
    );
    assert_eq!(
        host_key(&bench, &mut host, &close_chord()),
        EditorKey::Close
    );
}

/// MOD-141 M3 D1, D3 through the shell: with Vulkan linked into Platform and a graph `flow` in
/// both Platform and Graphics, a kind on Platform's names its workspace on its line, and a new
/// kind typed with the bare name `flow` is refused with both qualified labels, nothing written.
#[tokio::test]
async fn a_kind_line_names_its_workspace_and_refuses_an_ambiguous_name() {
    let store = MemStore::demo();
    store
        .upsert_workspace_project(&WorkspaceProject {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_id: ids::PROJECT_VULKAN,
            position: 9,
        })
        .await
        .expect("vulkan joins platform");
    let mut flows = Vec::new();
    for workspace in [ids::WORKSPACE_PLATFORM, ids::WORKSPACE_GRAPHICS] {
        let graph = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Workspace(workspace),
                name: "flow".to_owned(),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("a workspace graph");
        flows.push(graph.id);
    }
    store
        .create_item_kind(NewItemKind {
            id: ItemKindId::new(),
            project_id: ids::PROJECT_VULKAN,
            prefix: "DOC".to_owned(),
            name: "docs".to_owned(),
            description: String::new(),
            default_graph_id: flows[0],
            position: 99,
        })
        .await
        .expect("a kind on Platform's flow");
    let kinds_before = WorkerStore::item_kinds(&store, ids::PROJECT_VULKAN)
        .await
        .expect("a memory read never fails")
        .len();

    let mut harness = kinds_over(store.clone()).await;
    let frame = harness.render();
    assert!(
        frame.contains("DOC  docs \u{b7} graph flow \u{b7} workspace Platform"),
        "the kind's line names the workspace: {frame}"
    );

    // A new kind, its graph field the bare name both workspaces' graphs share.
    repeat(&mut harness, "k", 96);
    harness.key("n");
    typed(&mut harness, "WSF");
    harness.key("tab");
    typed(&mut harness, "wsflow");
    repeat(&mut harness, "tab", 2);
    repeat(&mut harness, "backspace", 40);
    typed(&mut harness, "flow");
    harness.key("enter");
    harness.settle().await;
    let frame = flat(&harness.render());
    // `ambiguous_graph` is the crate's own; the sentence is written out here.
    let graphics = "flow \u{b7} workspace Graphics";
    let platform = "flow \u{b7} workspace Platform";
    let refusals = [
        format!("\"flow\" names 2 graphs; type one of: {platform}, {graphics}"),
        format!("\"flow\" names 2 graphs; type one of: {graphics}, {platform}"),
    ];
    assert!(
        refusals.iter().any(|sentence| frame.contains(sentence)),
        "the refusal lists both labels: {frame}"
    );
    let kinds = WorkerStore::item_kinds(&store, ids::PROJECT_VULKAN)
        .await
        .expect("a memory read never fails");
    assert_eq!(kinds.len(), kinds_before, "nothing is guessed: {kinds:?}");
    assert!(kinds.iter().all(|kind| kind.prefix != "WSF"), "{kinds:?}");
}
