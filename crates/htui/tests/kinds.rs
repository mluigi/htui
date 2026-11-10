//! `Settings > Kinds`, from the worker side out (MOD-15 milestone 4, D18).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, exactly as
//! `tests/hierarchy.rs` does: one request in, one reply out, no channels and no shell. The section
//! half is milestone 4's task 2 and lands below this one.
#![cfg(feature = "testkit")]

use chrono::{Duration, Utc};
use htui::app::{Action, Handled};
use htui::catalogue::{self, CatalogueSnapshot, GraphEntry, PersonaSummary, REQUEST_NAMES};
use htui::hierarchy::MirrorAfterDelete;
use htui::keys::views;
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::graph_editor::GraphEditor;
use htui::ui::kit::pane::Pane;
use htui::ui::tabs::settings::{
    AgentsSection, HierarchySection, KindsSection, SettingsSection, SettingsTab,
};
use htui_core::fixtures::ids;
use htui_core::model::{
    EdgeId, EdgeKind, GraphBody, GraphScope, Isolation, ItemKind, ItemKindId, ItemKindPatch,
    NewProject, NewStepGraph, NodeBody, PersonaId, PhaseAgent, PhaseId, PhasePart, PhasePatch,
    ProjectId, Scope, Split, StepGraph, StepGraphEdge, StepGraphId, StepGraphPatch, StepGraphPhase,
    WorkspaceId,
};
use htui_core::seed::{PHASES_PER_PROJECT, PhaseSeed, phase_row};
use htui_core::store::{
    CasOutcome, MemStore, ReadStore, StoreError, WriteStore, item_kind_is_held,
    palette_phase_has_edges, reserved_phase_name,
};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{Terminal, TerminalOptions, Viewport};

/// The demo world behind a memory backend: a `Writer::Memory` and the seeded catalogue M2 gave
/// every project (F-9), so no worker test here writes a row by hand.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The scope the section renders in the demo world: the Graphics workspace and its one project.
fn vulkan_scope() -> Scope {
    Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    }
}

/// The catalogue a reply carries, or a panic naming what came back instead.
#[track_caller]
fn catalogue(reply: StoreReply) -> CatalogueSnapshot {
    match reply {
        StoreReply::Catalogue(snapshot) => *snapshot,
        other => panic!("expected a catalogue: {other:?}"),
    }
}

/// The `Failed` reply's `(request, message)`, or a panic naming what came back instead.
#[track_caller]
fn refusal(reply: StoreReply) -> (&'static str, String) {
    match reply {
        StoreReply::Failed { request, message } => (request, message),
        other => panic!("expected a refusal: {other:?}"),
    }
}

/// `ReplaceGraph`'s answer (MOD-141 D13): the write's outcome with the body, and the scope's
/// catalogue re-read after it, or a panic naming what came back instead.
#[track_caller]
fn replaced(reply: StoreReply) -> (CasOutcome<Box<GraphBody>>, CatalogueSnapshot) {
    match reply {
        StoreReply::GraphReplaced { outcome, catalogue } => (outcome, *catalogue),
        other => panic!("expected a replaced graph: {other:?}"),
    }
}

/// One read of the demo scope.
async fn demo_catalogue(backend: &Backend) -> CatalogueSnapshot {
    catalogue(serve(backend, &StoreRequest::Catalogue(vulkan_scope())).await)
}

/// The empty scope every name-only request is built against: the twelve names are a property of the
/// enum, not of a store.
fn nil_scope() -> Scope {
    Scope {
        workspace_id: WorkspaceId::default(),
        project_ids: Vec::new(),
    }
}

/// One of each of the twelve, in [`REQUEST_NAMES`] order. The ids are nil where the request never
/// reaches a store.
fn catalogue_requests() -> Vec<StoreRequest> {
    vec![
        StoreRequest::Catalogue(nil_scope()),
        StoreRequest::CreateKind {
            scope: nil_scope(),
            project: ProjectId::default(),
            prefix: String::new(),
            name: String::new(),
            description: String::new(),
            graph: StepGraphId::default(),
            position: 0,
        },
        StoreRequest::UpdateKind {
            scope: nil_scope(),
            id: ItemKindId::default(),
            expected: Utc::now(),
            patch: ItemKindPatch::default(),
        },
        StoreRequest::DeleteKind {
            scope: nil_scope(),
            id: ItemKindId::default(),
        },
        StoreRequest::CreateGraph {
            scope: nil_scope(),
            project: ProjectId::default(),
            name: String::new(),
            description: String::new(),
        },
        StoreRequest::UpdateGraph {
            scope: nil_scope(),
            id: StepGraphId::default(),
            expected: Utc::now(),
            patch: StepGraphPatch::default(),
        },
        StoreRequest::CreatePhase {
            scope: nil_scope(),
            graph: StepGraphId::default(),
            name: String::new(),
            position: 0,
            template_name: String::new(),
            gate_hard: false,
            input_kinds: Vec::new(),
        },
        StoreRequest::UpdatePhase {
            scope: nil_scope(),
            id: PhaseId::default(),
            expected: Utc::now(),
            patch: PhasePatch::default(),
        },
        StoreRequest::SetPhaseBudget {
            scope: nil_scope(),
            phase: PhaseId::default(),
            expected: Utc::now(),
            budget: None,
        },
        StoreRequest::ReplaceGraph {
            scope: nil_scope(),
            graph: StepGraphId::default(),
            expected_revision: 0,
            body: Box::new(GraphBody {
                graph: StepGraph {
                    id: StepGraphId::default(),
                    scope: GraphScope::Project(ProjectId::default()),
                    name: String::new(),
                    description: String::new(),
                    is_override: false,
                    revision: 0,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                nodes: Vec::new(),
                edges: Vec::new(),
            }),
        },
        StoreRequest::DeletePhase {
            scope: nil_scope(),
            phase: PhaseId::default(),
        },
        StoreRequest::GraphBody(StepGraphId::default()),
    ]
}

/// `REQUEST_NAMES` and [`StoreRequest::name`] are one list read from two places (D5): a thirteenth
/// request cannot be named in one and matched in the other.
#[test]
fn catalogue_names_are_stable() {
    let requests = catalogue_requests();
    assert_eq!(requests.len(), REQUEST_NAMES.len());
    assert_eq!(REQUEST_NAMES.len(), 12);
    assert_eq!(
        REQUEST_NAMES[9..],
        ["replace_graph", "delete_phase", "graph_body"],
        "the two MOD-137 verbs follow the nine (D12), then MOD-141's read (D13)"
    );
    for (request, name) in requests.iter().zip(REQUEST_NAMES) {
        assert_eq!(request.name(), name);
    }
}

/// The seed reads back whole through one request: five kinds in position order, five graphs, and
/// the fifteen phases `PHASES_PER_PROJECT` counts (D3, F-9).
#[tokio::test]
async fn the_demo_catalogue_reads_back_the_seed() {
    let snapshot = demo_catalogue(&demo()).await;

    assert_eq!(snapshot.projects.len(), 1, "the scope holds one project");
    let entry = &snapshot.projects[0];
    assert_eq!(entry.project.slug, "vulkan-tutorials");

    let prefixes: Vec<&str> = entry.kinds.iter().map(|k| k.prefix.as_str()).collect();
    assert_eq!(prefixes, ["ANA", "FEAT", "FIX", "CLEAN", "TOOL"]);

    assert_eq!(entry.graphs.len(), 5);
    let phases: usize = entry.graphs.iter().map(|g| g.nodes.len()).sum();
    assert_eq!(phases, PHASES_PER_PROJECT);

    for kind in &entry.kinds {
        let graph = entry
            .graph(kind.default_graph_id)
            .unwrap_or_else(|| panic!("`{}` points at a graph of its project", kind.prefix));
        assert_eq!(
            graph.graph.name, kind.name,
            "the seeded graph carries the kind's name"
        );
    }

    let analysis = entry
        .graph(ids::GRAPH_VULKAN_ANA)
        .expect("the analysis graph");
    let names: Vec<&str> = analysis
        .nodes
        .iter()
        .map(|n| n.phase.name.as_str())
        .collect();
    assert_eq!(names, ["research", "verdict"], "phases in position order");
}

/// The same read carries the global persona registry by name, id and name only: the phase editor's
/// `persona` field resolves against it and a bound phase's line names its persona from it (MOD-26
/// M2 D23). By name bytes, as `personas()` answers.
#[tokio::test]
async fn the_catalogue_carries_the_personas_by_name() {
    let snapshot = demo_catalogue(&demo()).await;

    assert_eq!(
        snapshot.personas,
        [
            PersonaSummary {
                id: ids::PERSONA_ARCHITECT,
                name: "architect".to_owned(),
            },
            PersonaSummary {
                id: ids::PERSONA_REVIEWER,
                name: "reviewer".to_owned(),
            },
        ]
    );
}

/// Two projects in the scope answer two catalogues in **scope order**, which is the whole reason
/// the read is one request rather than one per project (D2, F-22).
#[tokio::test]
async fn a_two_project_scope_answers_both_in_scope_order() {
    let store = MemStore::demo();
    let second = store
        .create_project(NewProject {
            id: ProjectId::new(),
            slug: "second".to_owned(),
            name: "Second".to_owned(),
            description: String::new(),
            created_by: ids::USER,
        })
        .await
        .expect("a second project");
    let backend = Backend::memory(store);
    let scope = Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![second.id, ids::PROJECT_VULKAN],
    };

    let snapshot = catalogue(serve(&backend, &StoreRequest::Catalogue(scope)).await);

    let slugs: Vec<&str> = snapshot
        .projects
        .iter()
        .map(|p| p.project.slug.as_str())
        .collect();
    assert_eq!(slugs, ["second", "vulkan-tutorials"]);
    assert_eq!(
        snapshot.projects[0].kinds.len(),
        5,
        "a created project arrives seeded (M2)"
    );
}

/// A scope id that names no project is skipped rather than failing the read: a torn read is not a
/// state to render (D3, H-15).
#[tokio::test]
async fn an_unknown_project_id_is_skipped() {
    let scope = Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ProjectId::new(), ids::PROJECT_VULKAN],
    };

    let snapshot = catalogue(serve(&demo(), &StoreRequest::Catalogue(scope)).await);

    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.projects[0].project.slug, "vulkan-tutorials");
}

/// Offline, every one of the twelve is refused by its own name with MOD-25's one sentence:
/// `Backend::writer()` is `None`, so `serve` never reaches the seam — the read included (D7).
#[tokio::test]
async fn offline_refuses_every_catalogue_request_by_name() {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "catalogue-offline", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let backend = Backend::Offline {
        cache,
        since: Some(Utc::now()),
    };

    for (request, name) in catalogue_requests().into_iter().zip(REQUEST_NAMES) {
        let (refused, message) = refusal(serve(&backend, &request).await);
        assert_eq!(refused, name);
        assert!(
            message.contains(DATABASE_UNREACHABLE),
            "`{name}` is refused with MOD-25's sentence: {message}"
        );
    }
}

/// `catalogue::serve` is reachable only through `try_serve`'s twelve or-ed patterns, so a request
/// from anywhere else is told which one it sent rather than panicking.
#[tokio::test]
async fn serve_refuses_a_foreign_request_by_name() {
    let err = catalogue::serve(&demo(), &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS))
        .await
        .expect_err("a hierarchy request is not this module's");

    assert_eq!(
        err,
        StoreError::Backend("not a catalogue request: hierarchy".to_owned())
    );
}

/// The kind of a snapshot's one project, by id.
#[track_caller]
fn kind(snapshot: &CatalogueSnapshot, id: ItemKindId) -> &ItemKind {
    snapshot.projects[0]
        .kinds
        .iter()
        .find(|k| k.id == id)
        .unwrap_or_else(|| panic!("the catalogue still names {id}"))
}

/// A create lands and the reply is the tree as it is now, not the row that was written (D5, D7).
#[tokio::test]
async fn create_kind_lands_and_rereads() {
    let backend = demo();

    let snapshot = catalogue(
        serve(
            &backend,
            &StoreRequest::CreateKind {
                scope: vulkan_scope(),
                project: ids::PROJECT_VULKAN,
                prefix: "DOC".to_owned(),
                name: "docs".to_owned(),
                description: String::new(),
                graph: ids::GRAPH_VULKAN_TOOL,
                position: 5,
            },
        )
        .await,
    );

    let kinds = &snapshot.projects[0].kinds;
    assert_eq!(kinds.len(), 6);
    let created = kinds.last().expect("the new kind sorts last by position");
    assert_eq!(created.prefix, "DOC");
    assert_eq!(created.name, "docs");
    assert_eq!(created.default_graph_id, ids::GRAPH_VULKAN_TOOL);
    assert_eq!(created.position, 5);
}

/// An edit against the current token applies and moves it (D8).
#[tokio::test]
async fn update_kind_applies_against_the_current_token() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let expected = kind(&before, ids::KIND_VULKAN_FEAT).updated_at;

    let snapshot = catalogue(
        serve(
            &backend,
            &StoreRequest::UpdateKind {
                scope: vulkan_scope(),
                id: ids::KIND_VULKAN_FEAT,
                expected,
                patch: ItemKindPatch {
                    name: Some("features".to_owned()),
                    ..ItemKindPatch::default()
                },
            },
        )
        .await,
    );

    let after = kind(&snapshot, ids::KIND_VULKAN_FEAT);
    assert_eq!(after.name, "features");
    assert_ne!(after.updated_at, expected, "the CAS token moved");
}

/// PRD D12's half the seam owns: renaming a prefix changes what the *next* mint spells and leaves
/// every key already minted under the old one exactly as it is (F-17).
#[tokio::test]
async fn a_prefix_rename_leaves_existing_keys_alone() {
    let store = MemStore::demo();
    let backend = Backend::memory(store);
    let before = demo_catalogue(&backend).await;
    let expected = kind(&before, ids::KIND_VULKAN_FEAT).updated_at;

    let snapshot = catalogue(
        serve(
            &backend,
            &StoreRequest::UpdateKind {
                scope: vulkan_scope(),
                id: ids::KIND_VULKAN_FEAT,
                expected,
                patch: ItemKindPatch {
                    prefix: Some("FT".to_owned()),
                    ..ItemKindPatch::default()
                },
            },
        )
        .await,
    );

    assert_eq!(kind(&snapshot, ids::KIND_VULKAN_FEAT).prefix, "FT");
    let item = backend
        .item(ids::VULKAN_FEAT_1)
        .await
        .expect("the store answers")
        .expect("the fixture's `FEAT-1`");
    assert_eq!(item.key_prefix, "FEAT");
    assert_eq!(item.key, "FEAT-1");
}

/// A token from before someone else's write answers `CatalogueStale` with the tree as it is now:
/// the write did not happen and the editor retries only on a second `Enter` (D8, `R-ENT-10`).
#[tokio::test]
async fn update_kind_with_a_stale_token_answers_stale() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let stored = kind(&before, ids::KIND_VULKAN_FEAT);
    let expected = stored.updated_at - Duration::seconds(1);

    let reply = serve(
        &backend,
        &StoreRequest::UpdateKind {
            scope: vulkan_scope(),
            id: ids::KIND_VULKAN_FEAT,
            expected,
            patch: ItemKindPatch {
                name: Some("features".to_owned()),
                ..ItemKindPatch::default()
            },
        },
    )
    .await;

    let StoreReply::CatalogueStale(snapshot) = reply else {
        panic!("a missed token answers stale: {reply:?}");
    };
    assert_eq!(
        kind(&snapshot, ids::KIND_VULKAN_FEAT).name,
        "feature",
        "the refused write left the row alone"
    );
}

/// A graph is created empty and edited under CAS, and a second edit with the first token is stale
/// (D5, D8).
#[tokio::test]
async fn create_graph_and_update_graph() {
    let backend = demo();

    let created = catalogue(
        serve(
            &backend,
            &StoreRequest::CreateGraph {
                scope: vulkan_scope(),
                project: ids::PROJECT_VULKAN,
                name: "docs".to_owned(),
                description: String::new(),
            },
        )
        .await,
    );
    assert_eq!(created.projects[0].graphs.len(), 6);
    let entry = created.projects[0]
        .graphs
        .iter()
        .find(|g| g.graph.name == "docs")
        .expect("the new graph");
    assert!(entry.nodes.is_empty(), "a graph is created without phases");
    let id = entry.graph.id;
    let expected = entry.graph.updated_at;

    let renamed = catalogue(
        serve(
            &backend,
            &StoreRequest::UpdateGraph {
                scope: vulkan_scope(),
                id,
                expected,
                patch: StepGraphPatch {
                    name: Some("documentation".to_owned()),
                    description: None,
                },
            },
        )
        .await,
    );
    assert_eq!(
        renamed.projects[0]
            .graph(id)
            .expect("the renamed graph")
            .graph
            .name,
        "documentation"
    );

    let reply = serve(
        &backend,
        &StoreRequest::UpdateGraph {
            scope: vulkan_scope(),
            id,
            expected,
            patch: StepGraphPatch {
                name: Some("docs again".to_owned()),
                description: None,
            },
        },
    )
    .await;
    assert!(
        matches!(reply, StoreReply::CatalogueStale(_)),
        "the first token is spent: {reply:?}"
    );
}

/// One graph's phases, in position order.
#[track_caller]
fn phases(snapshot: &CatalogueSnapshot, graph: StepGraphId) -> Vec<StepGraphPhase> {
    snapshot.projects[0]
        .graph(graph)
        .unwrap_or_else(|| panic!("the catalogue still names {graph}"))
        .nodes
        .iter()
        .map(|node| node.phase.clone())
        .collect()
}

/// A created phase is the seeder's own row with exactly the request's five columns written over
/// it: the eight frozen ones come from `seed::phase_row` and from nowhere else (D9, D17b, H-2).
#[tokio::test]
async fn create_phase_is_the_seeders_row_plus_five_columns() {
    let backend = demo();

    let snapshot = catalogue(
        serve(
            &backend,
            &StoreRequest::CreatePhase {
                scope: vulkan_scope(),
                graph: ids::GRAPH_VULKAN_ANA,
                name: "triage".to_owned(),
                position: 7,
                template_name: "triage".to_owned(),
                gate_hard: true,
                input_kinds: vec!["verdict".to_owned()],
            },
        )
        .await,
    );

    let rows = phases(&snapshot, ids::GRAPH_VULKAN_ANA);
    assert_eq!(rows.len(), 3);
    let created = rows.last().expect("the new phase sorts last by position");
    assert_eq!(created.name, "triage");
    assert_eq!(
        created.output_kind, "triage",
        "a created phase takes the seeder's rule: output_kind is the name (D17b)"
    );
    assert_eq!(created.template_name, "triage");
    assert_eq!(created.position, 7);
    assert!(created.gate_hard);
    assert_eq!(created.input_kinds, ["verdict"]);

    let frozen = phase_row(
        PhaseId::new(),
        ids::GRAPH_VULKAN_ANA,
        7,
        &PhaseSeed {
            name: "",
            input_kinds: &[],
            gate_hard: true,
        },
        Utc::now(),
    );
    assert_eq!(created.fan_out, frozen.fan_out);
    assert_eq!(created.gate, frozen.gate);
    assert_eq!(created.retry_limit, frozen.retry_limit);
    assert_eq!(created.isolation, frozen.isolation);
    assert_eq!(created.command_queue, frozen.command_queue);
    assert_eq!(created.verify_command, frozen.verify_command);
    assert_eq!(created.template_version, frozen.template_version);
    assert_eq!(created.token_budget, frozen.token_budget);
    assert_eq!(
        created.token_budget, None,
        "a phase is born inheriting (B-3)"
    );
}

/// `judge` and `handoff` are template roles, not phase names: the store refuses them, so nothing
/// in the app re-checks it (F-8).
#[tokio::test]
async fn create_phase_with_a_reserved_name_is_refused() {
    let (request, message) = refusal(
        serve(
            &demo(),
            &StoreRequest::CreatePhase {
                scope: vulkan_scope(),
                graph: ids::GRAPH_VULKAN_ANA,
                name: "judge".to_owned(),
                position: 7,
                template_name: "judge".to_owned(),
                gate_hard: false,
                input_kinds: Vec::new(),
            },
        )
        .await,
    );

    assert_eq!(request, "create_phase");
    assert!(
        message.contains(&reserved_phase_name("judge")),
        "the seam's own sentence reaches the section: {message}"
    );
}

/// `input_kinds` is replaced whole, empty list included (D13).
#[tokio::test]
async fn update_phase_replaces_input_kinds_whole() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let verdict = &phases(&before, ids::GRAPH_VULKAN_ANA)[1];
    assert_eq!(verdict.input_kinds, ["research"], "the seeded list");
    let expected = verdict.updated_at;
    let id = verdict.id;

    let snapshot = catalogue(
        serve(
            &backend,
            &StoreRequest::UpdatePhase {
                scope: vulkan_scope(),
                id,
                expected,
                patch: PhasePatch {
                    input_kinds: Some(Vec::new()),
                    ..PhasePatch::default()
                },
            },
        )
        .await,
    );

    let after = &phases(&snapshot, ids::GRAPH_VULKAN_ANA)[1];
    assert!(after.input_kinds.is_empty());
    assert_ne!(after.updated_at, expected, "the CAS token moved");
}

/// `token_budget` rides the `Phase` rung and nothing else (D6, D16): `Some` sets, `None` clears so
/// the rung below answers, a number the column cannot hold is the store's own refusal (B-1), and a
/// spent token is a CAS miss (B-2).
#[tokio::test]
async fn set_phase_budget_sets_then_clears() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let seeded = &phases(&before, ids::GRAPH_VULKAN_ANA)[0];
    let phase = seeded.id;
    assert_eq!(seeded.token_budget, None, "the seed inherits");

    let set = catalogue(
        serve(
            &backend,
            &StoreRequest::SetPhaseBudget {
                scope: vulkan_scope(),
                phase,
                expected: seeded.updated_at,
                budget: Some(60_000),
            },
        )
        .await,
    );
    let after_set = &phases(&set, ids::GRAPH_VULKAN_ANA)[0];
    assert_eq!(after_set.token_budget, Some(60_000));

    let stale = serve(
        &backend,
        &StoreRequest::SetPhaseBudget {
            scope: vulkan_scope(),
            phase,
            expected: seeded.updated_at,
            budget: Some(90_000),
        },
    )
    .await;
    assert!(
        matches!(stale, StoreReply::CatalogueStale(_)),
        "the token the set spent does not write twice: {stale:?}"
    );

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::SetPhaseBudget {
                scope: vulkan_scope(),
                phase,
                expected: after_set.updated_at,
                budget: Some(i64::MAX),
            },
        )
        .await,
    );
    assert_eq!(request, "set_phase_budget");
    // The blueprint (B-1, H-5) expected the column's `does not fit … which is INTEGER` sentence
    // here. `validate` runs first and the `Phase` rung's range is already `1..=i32::MAX`, so what
    // actually reaches the section is the range refusal; the INTEGER cast below it is defence in
    // depth a validated value never reaches. Either way the store is the one validator (M1 D7) and
    // the section re-implements no bound.
    assert!(
        message.contains("is outside 1..=2147483647 tokens"),
        "the store's own sentence reaches the section: {message}"
    );

    let cleared = catalogue(
        serve(
            &backend,
            &StoreRequest::SetPhaseBudget {
                scope: vulkan_scope(),
                phase,
                expected: after_set.updated_at,
                budget: None,
            },
        )
        .await,
    );
    assert_eq!(
        phases(&cleared, ids::GRAPH_VULKAN_ANA)[0].token_budget,
        None,
        "clearing lets the project rung answer rather than writing a guessed default"
    );
}

/// A kind any item holds cannot be deleted at all, so the destructive case PRD D13's typed slug
/// exists for does not arise here: the seam refuses by name and says what holds it (D11, F-7).
#[tokio::test]
async fn delete_kind_of_a_held_kind_is_refused_with_the_seam_sentence() {
    let backend = demo();

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::DeleteKind {
                scope: vulkan_scope(),
                id: ids::KIND_VULKAN_FEAT,
            },
        )
        .await,
    );

    assert_eq!(request, "delete_kind");
    assert!(
        message.contains(&item_kind_is_held("FEAT", 1)),
        "the sentence names what holds it: {message}"
    );
    assert_eq!(
        demo_catalogue(&backend).await.projects[0].kinds.len(),
        5,
        "the refused delete took nothing"
    );
}

/// A kind that is gone is reported with what the worker did to the mirror afterwards (D12): the
/// Backlog must not keep offering a kind no row answers for, and a memory backend has no mirror to
/// rebuild.
#[tokio::test]
async fn delete_kind_of_an_unreferenced_kind_reports_the_mirror() {
    let backend = demo();

    let reply = serve(
        &backend,
        &StoreRequest::DeleteKind {
            scope: vulkan_scope(),
            id: ids::KIND_VULKAN_ANA,
        },
    )
    .await;

    let StoreReply::KindDeleted { mirror, catalogue } = reply else {
        panic!("a delete that happened answers KindDeleted: {reply:?}");
    };
    assert_eq!(mirror, MirrorAfterDelete::NoMirror);
    let kinds = &catalogue.projects[0].kinds;
    assert_eq!(kinds.len(), 4);
    assert!(
        kinds.iter().all(|k| k.prefix != "ANA"),
        "the catalogue in the reply is the tree without it"
    );
    assert_eq!(
        catalogue.projects[0].graphs.len(),
        5,
        "the kind's graph survives it; nothing here deletes a graph (D5)"
    );
}

// ---- MOD-137 T4: edges in the catalogue, `ReplaceGraph` and `DeletePhase` ----

/// The entry of one of the demo project's graphs, or a panic naming the id.
#[track_caller]
fn graph_entry(snapshot: &CatalogueSnapshot, graph: StepGraphId) -> &GraphEntry {
    snapshot.projects[0]
        .graph(graph)
        .unwrap_or_else(|| panic!("the catalogue names graph {graph}"))
}

/// The phase of `entry` called `name`, or a panic.
#[track_caller]
fn phase_named<'a>(entry: &'a GraphEntry, name: &str) -> &'a StepGraphPhase {
    entry
        .nodes
        .iter()
        .map(|node| &node.phase)
        .find(|phase| phase.name == name)
        .unwrap_or_else(|| panic!("graph `{}` has a phase `{name}`", entry.graph.name))
}

/// The body an editor would send for `entry` as the catalogue holds it: the entry itself, which
/// is the graph's whole body (MOD-137 R1b, review C-M2).
fn body_of(entry: &GraphEntry) -> GraphBody {
    entry.clone()
}

/// The analysis graph with `verdict` renamed `decide` and an `asks_you decide → research` edge
/// (`loop_limit 1`) added, as a `ReplaceGraph` on the revision the catalogue carries.
fn rename_verdict_request(entry: &GraphEntry) -> StoreRequest {
    let mut body = body_of(entry);
    let research = phase_named(entry, "research").id;
    let verdict = phase_named(entry, "verdict").id;
    for node in &mut body.nodes {
        if node.phase.id == verdict {
            node.phase.name = "decide".to_owned();
        }
    }
    body.edges.push(StepGraphEdge {
        id: EdgeId::new(),
        graph_id: entry.graph.id,
        from: verdict,
        to: research,
        kind: EdgeKind::AsksYou,
        predicate: None,
        loop_limit: Some(1),
    });
    StoreRequest::ReplaceGraph {
        scope: vulkan_scope(),
        graph: entry.graph.id,
        expected_revision: entry.graph.revision,
        body: Box::new(body),
    }
}

/// Every graph entry carries its seeded edges (MOD-137 D9, D12): one `next` in analysis, the bug
/// graph's `review → fix` loop, fourteen in the project, every graph on revision 0.
#[tokio::test]
async fn the_demo_catalogue_carries_seeded_edges() {
    let snapshot = demo_catalogue(&demo()).await;

    let analysis = graph_entry(&snapshot, ids::GRAPH_VULKAN_ANA);
    assert_eq!(analysis.edges.len(), 1);
    assert_eq!(analysis.edges[0].kind, EdgeKind::Next);
    assert_eq!(analysis.edges[0].from, phase_named(analysis, "research").id);
    assert_eq!(analysis.edges[0].to, phase_named(analysis, "verdict").id);

    let bug = graph_entry(&snapshot, ids::GRAPH_VULKAN_FIX);
    let review = phase_named(bug, "review").id;
    let fix = phase_named(bug, "fix");
    let loops: Vec<&StepGraphEdge> = bug
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::ChangesRequested)
        .collect();
    assert_eq!(loops.len(), 1, "one review loop in the bug graph");
    assert_eq!((loops[0].from, loops[0].to), (review, fix.id));
    assert_eq!(loops[0].loop_limit, Some(fix.retry_limit));

    let graphs = &snapshot.projects[0].graphs;
    let edges: usize = graphs.iter().map(|entry| entry.edges.len()).sum();
    assert_eq!(edges, 14, "ten `next` and four review loops");
    for entry in graphs {
        assert_eq!(
            entry.graph.revision, 0,
            "`{}` is unedited",
            entry.graph.name
        );
        assert!(
            entry
                .edges
                .iter()
                .all(|edge| edge.graph_id == entry.graph.id),
            "`{}` carries only its own edges",
            entry.graph.name
        );
    }
}

/// The editor's one write lands whole and the reply carries the body as stored beside the catalogue
/// re-read: the renamed phase, the new edge, and the revision moved by one (MOD-137 D10, D12,
/// MOD-141 D13).
#[tokio::test]
async fn replace_graph_round_trips_through_the_catalogue() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let entry = graph_entry(&before, ids::GRAPH_VULKAN_ANA);
    let verdict = phase_named(entry, "verdict").id;
    let research = phase_named(entry, "research").id;

    let (outcome, snapshot) = replaced(serve(&backend, &rename_verdict_request(entry)).await);

    let after = graph_entry(&snapshot, ids::GRAPH_VULKAN_ANA);
    let CasOutcome::Applied(body) = outcome else {
        panic!("a fresh revision applies: {outcome:?}");
    };
    assert_eq!(body.graph.revision, 1);
    assert_eq!(*body, *after, "the reply's body is the one stored");
    assert_eq!(after.graph.revision, 1);
    let names: Vec<&str> = after.nodes.iter().map(|n| n.phase.name.as_str()).collect();
    assert_eq!(names, ["research", "decide"]);
    assert_eq!(
        phase_named(after, "decide").id,
        verdict,
        "a kept node keeps its id"
    );
    let asks: Vec<&StepGraphEdge> = after
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::AsksYou)
        .collect();
    assert_eq!(asks.len(), 1);
    assert_eq!((asks[0].from, asks[0].to), (verdict, research));
    assert_eq!(asks[0].loop_limit, Some(1));
    assert_eq!(after.edges.len(), 2, "the seeded `next` survives");
}

/// MOD-137 R1b (review C-M2): the catalogue carries each graph's whole body, so the editor's
/// `ReplaceGraph` built from an entry keeps the candidates and parts it did not touch rather than
/// wiping them.
#[tokio::test]
async fn replace_graph_from_the_catalogue_keeps_candidates_and_parts() {
    let backend = demo();
    let writer = backend.writer().expect("a memory backend writes");
    let seeded = demo_catalogue(&backend).await;
    let entry = graph_entry(&seeded, ids::GRAPH_VULKAN_ANA);
    let research = phase_named(entry, "research").id;
    let verdict = phase_named(entry, "verdict").id;
    let candidate = PhaseAgent {
        phase_id: research,
        position: 0,
        agent_id: ids::AGENT_CLAUDE,
        model: "sonnet".to_owned(),
    };
    writer
        .create_phase_agents(research, std::slice::from_ref(&candidate))
        .await
        .expect("research takes a candidate");
    let part = PhasePart {
        phase_id: verdict,
        position: 0,
        key: "api".to_owned(),
        title: "The API".to_owned(),
        brief: String::new(),
        owned_paths: vec!["src/api/**".to_owned()],
        acceptance: None,
        agent_id: None,
        model: None,
        persona_id: None,
    };
    let mut divided = writer
        .graph_body(ids::GRAPH_VULKAN_ANA)
        .await
        .expect("a memory read")
        .expect("the analysis graph is there");
    for node in &mut divided.nodes {
        if node.phase.id == verdict {
            node.phase.split = Split::Divide;
            node.parts = vec![part.clone()];
        }
    }
    let revision = divided.graph.revision;
    let outcome = writer
        .replace_graph(ids::GRAPH_VULKAN_ANA, revision, divided)
        .await
        .expect("verdict divides into one part");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");

    // The editor opens the graph from the catalogue and renames `verdict`.
    let opened = demo_catalogue(&backend).await;
    let (_, snapshot) = replaced(
        serve(
            &backend,
            &rename_verdict_request(graph_entry(&opened, ids::GRAPH_VULKAN_ANA)),
        )
        .await,
    );

    let after = graph_entry(&snapshot, ids::GRAPH_VULKAN_ANA);
    assert_eq!(after.graph.revision, 3, "candidates 1, divide 2, rename 3");
    assert_eq!(phase_named(after, "decide").id, verdict);
    let stored = writer
        .graph_body(ids::GRAPH_VULKAN_ANA)
        .await
        .expect("a memory read")
        .expect("the analysis graph is there");
    let node = |id: PhaseId| {
        stored
            .nodes
            .iter()
            .find(|node| node.phase.id == id)
            .expect("the node is kept")
    };
    assert_eq!(
        node(research).agents,
        [candidate],
        "research keeps its candidate"
    );
    assert_eq!(node(verdict).parts, [part], "and the renamed node its part");
}

/// The same request twice: the second carries a spent revision and answers `GraphReplaced` with
/// `Stale` and the body as it is now, beside the catalogue as the first left it (D5, D8, MOD-141
/// D13).
#[tokio::test]
async fn a_spent_revision_answers_graph_replaced_stale() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let request = rename_verdict_request(graph_entry(&before, ids::GRAPH_VULKAN_ANA));

    let (applied, first) = replaced(serve(&backend, &request).await);
    assert!(matches!(applied, CasOutcome::Applied(_)), "{applied:?}");
    let reply = serve(&backend, &request).await;

    let StoreReply::GraphReplaced {
        outcome: CasOutcome::Stale(current),
        catalogue: snapshot,
    } = reply
    else {
        panic!("a spent revision answers stale: {reply:?}");
    };
    assert_eq!(
        current.graph.revision, 1,
        "the body as the first write left it"
    );
    assert_eq!(*snapshot, first, "the refused write changed nothing");
    assert_eq!(
        graph_entry(&snapshot, ids::GRAPH_VULKAN_ANA).graph.revision,
        1
    );
}

/// A store refusal of the body answers `Failed` by the request's own name with the seam's
/// sentence (MOD-137 §1.5): here a palette node that keeps an edge.
#[tokio::test]
async fn a_refused_replace_graph_answers_failed_by_name() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let entry = graph_entry(&before, ids::GRAPH_VULKAN_ANA);
    let mut body = body_of(entry);
    let research = phase_named(entry, "research").id;
    for node in &mut body.nodes {
        if node.phase.id == research {
            node.phase.palette = true;
        }
    }
    let edge = body.edges[0].id;

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::ReplaceGraph {
                scope: vulkan_scope(),
                graph: entry.graph.id,
                expected_revision: entry.graph.revision,
                body: Box::new(body),
            },
        )
        .await,
    );

    assert_eq!(request, "replace_graph");
    assert!(
        message.contains(&palette_phase_has_edges("research", edge)),
        "the sentence names the palette node and its edge: {message}"
    );
    assert_eq!(
        graph_entry(&demo_catalogue(&backend).await, ids::GRAPH_VULKAN_ANA)
            .graph
            .revision,
        0,
        "the refused write bumped nothing"
    );
}

/// A phase deleted goes with its edges, the graph's revision moves, and the reply is the catalogue
/// re-read (MOD-137 D11, D12).
#[tokio::test]
async fn delete_phase_rereads_the_catalogue() {
    let backend = demo();
    let before = demo_catalogue(&backend).await;
    let verdict = phase_named(graph_entry(&before, ids::GRAPH_VULKAN_ANA), "verdict").id;

    let snapshot = catalogue(
        serve(
            &backend,
            &StoreRequest::DeletePhase {
                scope: vulkan_scope(),
                phase: verdict,
            },
        )
        .await,
    );

    let after = graph_entry(&snapshot, ids::GRAPH_VULKAN_ANA);
    let names: Vec<&str> = after.nodes.iter().map(|n| n.phase.name.as_str()).collect();
    assert_eq!(names, ["research"]);
    assert!(after.edges.is_empty(), "the phase's edge went with it");
    assert_eq!(after.graph.revision, 1);
}

/// MOD-141 D13: `GraphBody` reads one graph by id whatever its scope, the editor's open-by-id
/// (the Toolbox hook): a project graph from the seed, then a workspace and a library graph made
/// here; an id no graph has answers `None`.
#[tokio::test]
async fn graph_body_reads_one_graph_of_each_scope() {
    let backend = demo();
    let writer = backend.writer().expect("a memory backend writes");
    let workspace = StepGraphId::new();
    let library = StepGraphId::new();
    for (id, scope, name) in [
        (
            workspace,
            GraphScope::Workspace(ids::WORKSPACE_GRAPHICS),
            "ws-flow",
        ),
        (library, GraphScope::Library, "lib-flow"),
    ] {
        writer
            .create_step_graph(NewStepGraph {
                id,
                scope,
                name: name.to_owned(),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("the graph is created");
    }

    for (id, scope) in [
        (
            ids::GRAPH_VULKAN_ANA,
            GraphScope::Project(ids::PROJECT_VULKAN),
        ),
        (workspace, GraphScope::Workspace(ids::WORKSPACE_GRAPHICS)),
        (library, GraphScope::Library),
    ] {
        let reply = serve(&backend, &StoreRequest::GraphBody(id)).await;
        let StoreReply::GraphBody(Some(body)) = reply else {
            panic!("graph {id} answers its body: {reply:?}");
        };
        assert_eq!(body.graph.id, id);
        assert_eq!(body.graph.scope, scope);
    }

    let reply = serve(&backend, &StoreRequest::GraphBody(StepGraphId::new())).await;
    assert!(
        matches!(reply, StoreReply::GraphBody(None)),
        "an unknown id answers none: {reply:?}"
    );
}

/// MOD-141 D13: a library graph is written through the same `ReplaceGraph` as a project one, its
/// body read by `GraphBody`; the reply's body is the library graph as stored. The scope's catalogue
/// does not list it, and need not: the editor draws from the body.
#[tokio::test]
async fn replace_graph_on_a_library_graph_answers_its_body() {
    let backend = demo();
    let writer = backend.writer().expect("a memory backend writes");
    let library = StepGraphId::new();
    writer
        .create_step_graph(NewStepGraph {
            id: library,
            scope: GraphScope::Library,
            name: "lib-flow".to_owned(),
            description: String::new(),
            is_override: false,
        })
        .await
        .expect("the library graph is created");
    let reply = serve(&backend, &StoreRequest::GraphBody(library)).await;
    let StoreReply::GraphBody(Some(mut body)) = reply else {
        panic!("the library graph answers its body: {reply:?}");
    };
    body.nodes.push(NodeBody {
        phase: phase_row(
            PhaseId::new(),
            library,
            0,
            &PhaseSeed {
                name: "research",
                input_kinds: &[],
                gate_hard: false,
            },
            Utc::now(),
        ),
        agents: Vec::new(),
        parts: Vec::new(),
    });

    let (outcome, snapshot) = replaced(
        serve(
            &backend,
            &StoreRequest::ReplaceGraph {
                scope: vulkan_scope(),
                graph: library,
                expected_revision: body.graph.revision,
                body,
            },
        )
        .await,
    );

    let CasOutcome::Applied(stored) = outcome else {
        panic!("a fresh revision applies: {outcome:?}");
    };
    assert_eq!(stored.graph.scope, GraphScope::Library);
    assert_eq!(stored.graph.revision, 1);
    let names: Vec<&str> = stored.nodes.iter().map(|n| n.phase.name.as_str()).collect();
    assert_eq!(names, ["research"]);
    assert!(
        snapshot.projects[0].graph(library).is_none(),
        "the project's catalogue does not list a library graph"
    );
}

// -------------------------------------------------------------------------------------------
// ---- section (T2) ----
//
// The same catalogue from the other end: a `Harness` for the frames a user sees and a
// `SectionBench` for the keys and replies a frame cannot show (a request that was emitted, a
// scope that moved).
// -------------------------------------------------------------------------------------------

/// A settled Settings tab over `store`, all three sections registered and the strip already cycled
/// onto `Kinds` — the product's own registration order (D17), so two `l` are what reach it.
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

/// The scope a [`SectionBench`] issues against: the demo fixture's first workspace, exactly as the
/// bench builds it (the field itself is private).
async fn bench_scope() -> Scope {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    Scope::from_workspace(workspaces.first().expect("the fixture has a workspace"))
}

/// A bench, a section and the demo catalogue already delivered to it.
async fn bench_with_demo() -> (SectionBench, KindsSection, CatalogueSnapshot) {
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let snapshot = demo_catalogue(&demo()).await;
    bench.reply(
        &mut section,
        &StoreReply::Catalogue(Box::new(snapshot.clone())),
    );
    let _ = bench.drained();
    (bench, section, snapshot)
}

/// One section drawn into a `width`x30 buffer.
///
/// [`SectionBench::render_section`] answers text, and the thing below is a *style*: a refusal that
/// is not in `theme.error` reads as a row of the tree, and a snapshot records symbols only.
fn drawn(bench: &SectionBench, section: &dyn SettingsSection, width: u16) -> Buffer {
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
        .draw(|frame| section.render(frame, frame.area(), &ctx))
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// What the section drew in the theme's error colour, one entry per row that has any.
fn error_text(bench: &SectionBench, section: &dyn SettingsSection, width: u16) -> Vec<String> {
    let error = Theme::default().error.fg.unwrap_or(Color::Reset);
    let buffer = drawn(bench, section, width);
    (0..buffer.area.height)
        .filter_map(|y| {
            let text: String = (0..width)
                .filter(|x| buffer[(*x, y)].fg == error)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            let text = text.trim().to_owned();
            (!text.is_empty()).then_some(text)
        })
        .collect()
}

/// The demo catalogue as the section draws it (D4): the project, then each kind with the phases of
/// its default graph under it, and no `Graph` row at all — the seed is one graph per kind, so every
/// graph is spoken for.
#[tokio::test]
async fn the_demo_catalogue_renders_the_tree() {
    let mut harness = kinds_over(MemStore::demo()).await;
    let frame = harness.render();
    assert!(
        frame.contains(" Agents  Hierarchy  Kinds "),
        "the strip carries the third section: {frame}"
    );
    assert!(
        !frame.contains("graph, no kind"),
        "every seeded graph has a kind pointing at it: {frame}"
    );
    insta::assert_snapshot!("demo", frame);
}

/// Browse is not a mode: `captures_input` is false there, so the global table still owns `q`.
#[tokio::test]
async fn q_quits_from_browse() {
    let mut harness = kinds_over(MemStore::demo()).await;
    harness.key("q");
    harness.settle().await;
    assert!(
        harness.app().should_quit,
        "Browse binds no `q`, so the global binding takes it"
    );
}

/// An empty store answers an empty catalogue: the pane says the scope holds no project rather than
/// drawing a blank box.
#[tokio::test]
async fn no_workspace_says_so() {
    let mut harness = kinds_over(MemStore::new()).await;
    let frame = harness.render();
    assert!(frame.contains("no project in scope"), "{frame}");
    insta::assert_snapshot!("no_workspace", frame);
}

/// Offline the catalogue is one refused read — the read goes through `Backend::writer()` like every
/// write (D7) — so the section says what is missing, in `theme.error`, instead of an empty tree.
#[tokio::test]
async fn offline_is_unavailable_with_the_worker_sentence() {
    // `App::start` issues `ConnectionInfo` since MOD-15 M6, and over a non-`Memory` backend that
    // read reaches `secret::get_dsn` - the developer's own OS keyring without this guard.
    let _keyring = htui_store::testkit::mock_keyring().await;
    // The mirror outlives the harness: dropping the directory deletes it mid-test.
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "kinds-section", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let mut harness = Harness::over_backend(Backend::Offline {
        cache,
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
        Box::new(KindsSection::new()),
    ])))
    // `offline · 3s` would age between the render and the next tick.
    .with_store_state("offline \u{b7} 0s", None);
    harness.settle().await;
    harness.key("l");
    harness.key("l");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("catalogue unavailable"),
        "a refused read says what is missing: {frame}"
    );
    assert!(frame.contains(DATABASE_UNREACHABLE), "{frame}");
    insta::assert_snapshot!("offline", frame);
}

/// The selected phase prints MOD-4's columns under it, dimmed and unselectable, and says who owns
/// them (D15, B-11): editing a column nothing reads would invent MOD-4's semantics a milestone
/// early, hiding it would pretend the column is not there.
#[tokio::test]
async fn the_selected_phase_shows_the_read_only_line() {
    let (bench, mut section, _) = bench_with_demo().await;
    // project → the ANA kind → its first phase.
    bench.key(&mut section, "j");
    bench.key(&mut section, "j");

    let frame = bench.render_section(&section, 100);
    assert!(
        !frame.contains("MOD-4 owns these"),
        "the detail line has no owner tail any more (MOD-141 D14): {frame}"
    );
    assert!(frame.contains("template: latest"), "{frame}");
    assert!(
        error_text(&bench, &section, 100).is_empty(),
        "read-only is dim, not an error"
    );
    insta::assert_snapshot!("phase_detail", frame);
}

/// A kind whose `default_graph_id` names no graph of its project renders `graph missing` and
/// contributes no phase rows; the graph nothing points at is then listed on its own (D4).
#[tokio::test]
async fn a_kind_whose_graph_is_missing_says_so() {
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let mut snapshot = demo_catalogue(&demo()).await;
    snapshot.projects[0].kinds[0].default_graph_id = StepGraphId::new();
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();

    let frame = bench.render_section(&section, 100);
    let lines: Vec<&str> = frame.lines().collect();
    assert_eq!(
        lines[1].trim(),
        "ANA  analysis · graph missing",
        "the kind says what it cannot resolve: {frame}"
    );
    assert!(
        lines[2].trim().starts_with("FEAT"),
        "and contributes no phase rows: {frame}"
    );
    assert!(
        error_text(&bench, &section, 100).contains(&"graph missing".to_owned()),
        "the tail is in `theme.error`: {frame}"
    );
    assert!(
        frame.contains("analysis · graph, no kind"),
        "the graph nobody points at is listed on its own: {frame}"
    );
}

/// A graph no kind names is listed after the kinds with its own phases under it — without it a
/// graph created here would be invisible the moment no kind pointed at it (D4).
#[tokio::test]
async fn an_unreferenced_graph_is_listed_after_the_kinds() {
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let mut snapshot = demo_catalogue(&demo()).await;
    let now = Utc::now();
    snapshot.projects[0].graphs.push(GraphEntry {
        graph: StepGraph {
            id: StepGraphId::new(),
            scope: GraphScope::Project(ids::PROJECT_VULKAN),
            name: "orphan".to_owned(),
            description: String::new(),
            is_override: false,
            revision: 0,
            created_at: now,
            updated_at: now,
        },
        nodes: Vec::new(),
        edges: Vec::new(),
    });
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();

    let frame = bench.render_section(&section, 100);
    let last = frame
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .expect("the tree has rows");
    assert_eq!(
        last.trim(),
        "orphan · graph, no kind",
        "the unreferenced graph is the last row: {frame}"
    );
}

/// `r` re-reads the scope and `Esc` clears whatever the last reply said (T2 keys).
#[tokio::test]
async fn r_reloads_and_esc_clears_the_notice() {
    let (bench, mut section, _) = bench_with_demo().await;

    bench.key(&mut section, "r");
    let asked = bench.drained();
    let [Action::Store(StoreRequest::Catalogue(scope))] = asked.as_slice() else {
        panic!("`r` asks for the scope's catalogue, once: {asked:?}");
    };
    assert_eq!(*scope, bench_scope().await);

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "update_kind",
            message: "nope".to_owned(),
        },
    );
    assert!(bench.render_section(&section, 100).contains("nope"));

    bench.key(&mut section, "esc");
    assert!(!bench.render_section(&section, 100).contains("nope"));
}

/// A scope change drops what belongs to the other workspace — the editor included, because its
/// compare-and-set token is the other workspace's — and keeps the notice, which is often the
/// *consequence* of the switch (B-12).
#[tokio::test]
async fn a_scope_change_drops_the_editor_and_keeps_the_notice() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    assert!(section.captures_input());
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "update_kind",
            message: "nope".to_owned(),
        },
    );
    let other = Scope {
        workspace_id: WorkspaceId::new(),
        project_ids: vec![ProjectId::new()],
    };

    section.on_scope_change(&other);

    assert!(!section.captures_input(), "the editor did not survive it");
    let wanted = section.wants_requests(&other);
    let [StoreRequest::Catalogue(scope)] = wanted.as_slice() else {
        panic!("the read the section wants is the new scope's, whole (D2): {wanted:?}");
    };
    assert_eq!(*scope, other);
    let frame = bench.render_section(&section, 100);
    assert!(!frame.contains("vulkan-tutorials"), "{frame}");
    assert!(frame.contains("nope"), "the notice survives: {frame}");
}

/// Types one key per char into a section, as a user would: the focused field is the only thing that
/// sees them.
fn type_at(bench: &SectionBench, section: &mut dyn SettingsSection, text: &str) {
    for c in text.chars() {
        bench.key(section, &c.to_string());
    }
}

/// A frame line with the kit's one-cell gutter (`▌` on the focused row, blank elsewhere) taken
/// off the front.
fn past_gutter(line: &str) -> &str {
    let mut chars = line.chars();
    chars.next();
    chars.as_str()
}

/// The index of the editor row labelled `label` (MOD-115 M3: a `kit::form` row reads
/// `[gutter][label column][two blanks][value]`), or `None` when no row carries it.
fn field_line(frame: &str, label: &str) -> Option<usize> {
    frame.lines().position(|line| {
        past_gutter(line)
            .strip_prefix(label)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with("  "))
    })
}

/// What the editor row labelled `label` shows, trimmed; `""` for a blank field.
fn field_value(frame: &str, label: &str) -> Option<String> {
    let line = frame.lines().nth(field_line(frame, label)?)?;
    Some(past_gutter(line)[label.len()..].trim().to_owned())
}

/// The frame's words joined by single spaces, so a wrapped sentence reads as one.
fn flat(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// While an editor is open every printable key is text, `l` included, so the tab's own section
/// cycle is off for as long as something is being typed (M3 D2).
#[tokio::test]
async fn l_is_a_letter_while_editing() {
    let (bench, mut section, _) = bench_with_demo().await;

    assert_eq!(bench.key(&mut section, "n"), Handled::Consumed);
    assert!(section.captures_input(), "an open editor takes every key");

    assert_eq!(bench.key(&mut section, "l"), Handled::Consumed);
    let frame = bench.render_section(&section, 100);
    assert_eq!(
        field_value(&frame, "prefix").as_deref(),
        Some("l"),
        "`l` went into the field, not to the strip: {frame}"
    );
    assert!(bench.drained().is_empty(), "typing asks the store nothing");
}

/// `n` on a kind row opens a new kind for that kind's project, prefilled with the first graph and
/// the position after the last one (B-7, B-8), and `Enter` sends exactly one create (D5).
#[tokio::test]
async fn n_on_a_kind_opens_the_kind_editor_and_enter_creates() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");

    bench.key(&mut section, "n");
    type_at(&bench, &mut section, "DOC");
    bench.key(&mut section, "tab");
    type_at(&bench, &mut section, "docs");
    // description, then the prefilled `graph` and `position`.
    bench.key(&mut section, "tab");
    bench.key(&mut section, "tab");
    bench.key(&mut section, "tab");
    bench.key(&mut section, "enter");

    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::CreateKind {
            scope,
            project,
            prefix,
            name,
            description,
            graph,
            position,
        }),
    ] = asked.as_slice()
    else {
        panic!("`Enter` creates once: {asked:?}");
    };
    assert_eq!(*scope, bench_scope().await, "every write carries the scope");
    assert_eq!(*project, ids::PROJECT_VULKAN);
    assert_eq!(prefix, "DOC");
    assert_eq!(name, "docs");
    assert_eq!(description, "");
    assert_eq!(*graph, ids::GRAPH_VULKAN_ANA, "the first graph by name");
    assert_eq!(*position, 5, "after the five seeded kinds");

    // One write of a kind at a time: the staleness index would drop the first reply.
    bench.key(&mut section, "enter");
    assert!(bench.drained().is_empty());
    assert!(
        error_text(&bench, &section, 100)
            .contains(&"still adding the kind \u{2014} wait for the store's answer".to_owned()),
        "a second Enter says why it did nothing, in words"
    );
    assert!(
        !error_text(&bench, &section, 100)
            .iter()
            .any(|text| text.contains("create_kind")),
        "no request name is printed (MOD-115 M3)"
    );
}

/// A required field that is empty is refused here, before the store is asked anything.
#[tokio::test]
async fn a_required_field_is_refused_before_any_request() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "n");

    bench.key(&mut section, "enter");

    assert!(bench.drained().is_empty());
    assert!(
        error_text(&bench, &section, 100).contains(&"`prefix` is required".to_owned()),
        "the refusal names the field"
    );
}

/// The kind editor's `graph` field is a graph **name**, resolved among the project's graphs at
/// submit; a name that matches none is refused rather than guessed at (B-7).
#[tokio::test]
async fn an_unknown_graph_name_is_refused() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "n");
    type_at(&bench, &mut section, "DOC");
    bench.key(&mut section, "tab");
    type_at(&bench, &mut section, "docs");
    bench.key(&mut section, "tab");
    bench.key(&mut section, "tab");
    // `graph`: clear the prefill, then a name no graph carries.
    for _ in 0.."analysis".len() {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "nope");

    bench.key(&mut section, "enter");

    assert!(bench.drained().is_empty());
    assert!(
        error_text(&bench, &section, 100)
            .contains(&"`graph` names no graph this project can use".to_owned()),
        "the refusal says what could not be resolved"
    );
}

/// MOD-138 D12: the kind editor's `graph` field resolves a library graph's name too (after the
/// project's own), the store takes it, and the kind's line names it with its scope.
#[tokio::test]
async fn a_kind_can_take_a_library_graph() {
    let store = MemStore::demo();
    let library = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            scope: GraphScope::Library,
            name: "libflow".to_owned(),
            description: String::new(),
            is_override: false,
        })
        .await
        .expect("a library graph");
    let backend = Backend::memory(store);
    let snapshot = demo_catalogue(&backend).await;
    assert_eq!(
        snapshot.projects[0]
            .shared
            .iter()
            .map(|graph| graph.id)
            .collect::<Vec<_>>(),
        vec![library.id],
        "the catalogue carries the graphs the project may use beside its own"
    );

    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Catalogue(Box::new(snapshot.clone())),
    );
    let _ = bench.drained();
    bench.key(&mut section, "n");
    type_at(&bench, &mut section, "DOC");
    bench.key(&mut section, "tab");
    type_at(&bench, &mut section, "docs");
    bench.key(&mut section, "tab");
    bench.key(&mut section, "tab");
    for _ in 0.."analysis".len() {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "libflow");
    bench.key(&mut section, "enter");

    let asked = bench.drained();
    let [request @ Action::Store(StoreRequest::CreateKind { graph, .. })] = asked.as_slice() else {
        panic!("`Enter` creates once: {asked:?}");
    };
    assert_eq!(*graph, library.id, "the library graph, by name");
    let Action::Store(request) = request else {
        unreachable!("matched above")
    };
    let reread = catalogue(serve(&backend, request).await);
    let created = reread.projects[0]
        .kinds
        .iter()
        .find(|kind| kind.prefix == "DOC")
        .expect("the kind landed");
    assert_eq!(created.default_graph_id, library.id);

    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(reread)));
    let _ = bench.drained();
    let frame = bench.render_section(&section, 100);
    assert!(
        frame
            .lines()
            .any(|line| line.trim() == "DOC  docs \u{b7} graph libflow \u{b7} library"),
        "the kind's line names the library graph: {frame}"
    );

    // `g` on that kind: the library graph is not this section's to edit, and is not "missing".
    let row = frame
        .lines()
        .position(|line| line.trim().starts_with("DOC  docs"))
        .expect("the kind's row");
    for _ in 0..row {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "g");
    assert!(bench.drained().is_empty(), "no editor, no request");
    assert!(
        error_text(&bench, &section, 100)
            .contains(&"graph libflow \u{b7} library is not edited here".to_owned()),
        "the refusal names the shared graph: {}",
        bench.render_section(&section, 100)
    );
}

/// MOD-138 D12: a graph name is unique within one scope only, so the project may own a graph
/// named like the library graph its kind is on. The `graph` field then reads `libflow · library`,
/// an untouched `e` + `Enter` keeps the library graph rather than switching the kind to the
/// project's own, and the bare name still picks the project's graph first.
#[tokio::test]
async fn an_untouched_kind_edit_keeps_a_shared_graph_the_project_shadows_by_name() {
    let store = MemStore::demo();
    let library = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            scope: GraphScope::Library,
            name: "libflow".to_owned(),
            description: String::new(),
            is_override: false,
        })
        .await
        .expect("a library graph");
    store
        .create_item_kind(htui_core::model::NewItemKind {
            id: ItemKindId::new(),
            project_id: ids::PROJECT_VULKAN,
            prefix: "DOC".to_owned(),
            name: "docs".to_owned(),
            description: String::new(),
            default_graph_id: library.id,
            position: 99,
        })
        .await
        .expect("a kind on the library graph");
    let own = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            scope: GraphScope::Project(ids::PROJECT_VULKAN),
            name: "libflow".to_owned(),
            description: String::new(),
            is_override: false,
        })
        .await
        .expect("the project's own graph of the same name");
    let backend = Backend::memory(store);
    let snapshot = demo_catalogue(&backend).await;

    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();
    let frame = bench.render_section(&section, 100);
    let row = frame
        .lines()
        .position(|line| line.trim() == "DOC  docs \u{b7} graph libflow \u{b7} library")
        .unwrap_or_else(|| panic!("the kind's row names the library graph: {frame}"));
    let edit = |section: &mut KindsSection| {
        for _ in 0..row {
            bench.key(section, "j");
        }
        bench.key(section, "e");
    };

    edit(&mut section);
    let open = bench.render_section(&section, 100);
    assert!(
        open.lines()
            .any(|line| line.trim() == "graph         libflow \u{b7} library"),
        "the graph field names the scope: {open}"
    );
    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [Action::Store(StoreRequest::UpdateKind { patch, .. })] = asked.as_slice() else {
        panic!("`Enter` updates once: {asked:?}");
    };
    assert_eq!(
        patch.default_graph_id,
        Some(library.id),
        "an untouched field keeps the library graph"
    );

    // Typing the bare name over the field picks the project's own graph.
    let mut section = KindsSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Catalogue(Box::new(demo_catalogue(&backend).await)),
    );
    let _ = bench.drained();
    edit(&mut section);
    for _ in 0..3 {
        bench.key(&mut section, "tab");
    }
    for _ in 0.."libflow \u{b7} library".chars().count() {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "libflow");
    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [Action::Store(StoreRequest::UpdateKind { patch, .. })] = asked.as_slice() else {
        panic!("`Enter` updates once: {asked:?}");
    };
    assert_eq!(
        patch.default_graph_id,
        Some(own.id),
        "the bare name is the project's own graph"
    );
}

/// MOD-138 D12: a kind keeps its workspace graph after the project leaves that workspace, and the
/// kind still resolves. `usable_graphs` no longer lists the graph, yet the catalogue carries its row
/// in `kept`, so the kind's line names it with its scope rather than `graph missing`, and the edit
/// form opens on that label. A rename sends the kept graph untouched, and it lands: a kind's graph
/// is checked against the usable-graphs rule only when an update changes it (A7).
#[tokio::test]
async fn a_kept_workspace_graph_reads_with_its_scope_not_as_missing() {
    let store = MemStore::demo();
    let shared = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            scope: GraphScope::Workspace(ids::WORKSPACE_GRAPHICS),
            name: "wsflow".to_owned(),
            description: String::new(),
            is_override: false,
        })
        .await
        .expect("a workspace graph");
    store
        .create_item_kind(htui_core::model::NewItemKind {
            id: ItemKindId::new(),
            project_id: ids::PROJECT_VULKAN,
            prefix: "DOC".to_owned(),
            name: "docs".to_owned(),
            description: String::new(),
            default_graph_id: shared.id,
            position: 99,
        })
        .await
        .expect("a kind on the workspace graph");
    store
        .remove_workspace_project(ids::WORKSPACE_GRAPHICS, ids::PROJECT_VULKAN)
        .await
        .expect("the project leaves the workspace");
    let backend = Backend::memory(store);
    let snapshot = demo_catalogue(&backend).await;
    let project = &snapshot.projects[0];
    assert!(
        !project.shared.iter().any(|graph| graph.id == shared.id),
        "the project may no longer pick it"
    );
    assert_eq!(
        project
            .kept
            .iter()
            .map(|graph| graph.id)
            .collect::<Vec<_>>(),
        vec![shared.id],
        "the catalogue carries the graph its kind kept"
    );

    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();
    let frame = bench.render_section(&section, 100);
    let row = frame
        .lines()
        .position(|line| line.trim() == "DOC  docs \u{b7} graph wsflow \u{b7} workspace")
        .unwrap_or_else(|| panic!("the kind's line names the kept graph: {frame}"));
    assert!(
        !error_text(&bench, &section, 100).contains(&"graph missing".to_owned()),
        "a kept graph is not missing: {frame}"
    );

    for _ in 0..row {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "e");
    let open = bench.render_section(&section, 100);
    assert!(
        open.lines()
            .any(|line| line.trim() == "graph         wsflow \u{b7} workspace"),
        "the graph field names the kept graph: {open}"
    );
    bench.key(&mut section, "tab");
    for _ in 0.."docs".len() {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "guides");
    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [Action::Store(request @ StoreRequest::UpdateKind { patch, .. })] = asked.as_slice() else {
        panic!("`Enter` updates once: {asked:?}");
    };
    assert_eq!(
        patch.default_graph_id,
        Some(shared.id),
        "an untouched field keeps the kept graph"
    );
    assert_eq!(patch.name.as_deref(), Some("guides"), "the rename");
    let reread = catalogue(serve(&backend, request).await);
    let renamed = reread.projects[0]
        .kinds
        .iter()
        .find(|kind| kind.prefix == "DOC")
        .expect("the kind is still there");
    assert_eq!(renamed.name, "guides", "the rename lands");
    assert_eq!(
        renamed.default_graph_id, shared.id,
        "an unchanged graph is not re-checked, so the kind keeps it"
    );
}

/// `e` on a kind sends the whole patch against the row's token, changed columns or not (B-5): one
/// habit across both sections, and compare-and-set is what makes it safe.
#[tokio::test]
async fn e_on_a_kind_then_enter_sends_the_whole_patch_with_the_token() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    let stored = snapshot.projects[0]
        .kinds
        .iter()
        .find(|k| k.id == ids::KIND_VULKAN_ANA)
        .expect("the ANA kind")
        .clone();
    bench.key(&mut section, "j");

    bench.key(&mut section, "e");
    bench.key(&mut section, "tab");
    bench.key(&mut section, "space");
    type_at(&bench, &mut section, "2");
    bench.key(&mut section, "enter");

    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::UpdateKind {
            id,
            expected,
            patch,
            ..
        }),
    ] = asked.as_slice()
    else {
        panic!("`Enter` edits once: {asked:?}");
    };
    assert_eq!(*id, ids::KIND_VULKAN_ANA);
    assert_eq!(*expected, stored.updated_at, "the row's own token (D8)");
    assert_eq!(patch.prefix.as_deref(), Some("ANA"));
    assert_eq!(patch.name.as_deref(), Some("analysis 2"));
    assert_eq!(
        patch.description.as_deref(),
        Some(stored.description.as_str()),
        "an untouched column still goes out, prefilled (B-5)"
    );
    assert_eq!(patch.default_graph_id, Some(ids::GRAPH_VULKAN_ANA));
    assert_eq!(patch.position, Some(0));
}

/// D19: `g` reaches the owning graph's editor from a kind row and from a phase row, because a graph
/// every kind points at has no `Graph` row of its own — without this key the five seeded graphs
/// would have no editable name or description at all (H-6).
#[tokio::test]
async fn g_opens_the_owning_graphs_editor_from_a_kind_and_from_a_phase() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    let graph = snapshot.projects[0]
        .graph(ids::GRAPH_VULKAN_ANA)
        .expect("the analysis graph")
        .graph
        .clone();

    // On the kind row.
    bench.key(&mut section, "j");
    bench.key(&mut section, "g");
    let frame = bench.render_section(&section, 100);
    assert_eq!(
        field_value(&frame, "name").as_deref(),
        Some("analysis"),
        "{frame}"
    );
    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::UpdateGraph {
            id,
            expected,
            patch,
            ..
        }),
    ] = asked.as_slice()
    else {
        panic!("`g` opens the editor `e` opens on a graph row: {asked:?}");
    };
    assert_eq!(*id, ids::GRAPH_VULKAN_ANA);
    assert_eq!(*expected, graph.updated_at);
    assert_eq!(patch.name.as_deref(), Some("analysis"));

    // And on a phase row, where it means the graph the phase belongs to.
    bench.reply(
        &mut section,
        &StoreReply::Catalogue(Box::new(snapshot.clone())),
    );
    let _ = bench.drained();
    bench.key(&mut section, "j");
    bench.key(&mut section, "j");
    bench.key(&mut section, "g");
    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [Action::Store(StoreRequest::UpdateGraph { id, .. })] = asked.as_slice() else {
        panic!("a phase row's `g` is its graph: {asked:?}");
    };
    assert_eq!(*id, ids::GRAPH_VULKAN_ANA);
}

/// `N` opens a graph for the row's project, and a project row is read-only here: the hierarchy
/// section owns projects, this one owns what is inside them (B-13).
#[tokio::test]
async fn n_upper_creates_a_graph_and_a_project_row_points_at_hierarchy() {
    let (bench, mut section, _) = bench_with_demo().await;

    bench.key(&mut section, "e");
    assert!(bench.drained().is_empty());
    assert!(
        error_text(&bench, &section, 100).contains(&"projects are edited in Hierarchy".to_owned()),
        "a project row says where it is edited"
    );

    bench.key(&mut section, "N");
    type_at(&bench, &mut section, "docs");
    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::CreateGraph {
            project,
            name,
            description,
            ..
        }),
    ] = asked.as_slice()
    else {
        panic!("`N` creates a graph once: {asked:?}");
    };
    assert_eq!(*project, ids::PROJECT_VULKAN);
    assert_eq!(name, "docs");
    assert_eq!(description, "");
}

/// A compare-and-set miss keeps the editor and its text, moves the token to the row as it is now,
/// and waits for a second `Enter` (D8, PRD D8): retyping is the cost the PRD said not to pay.
#[tokio::test]
async fn a_stale_reply_keeps_the_text_and_retakes_the_token() {
    let backend = demo();
    let (bench, mut section, snapshot) = bench_with_demo().await;
    let opened = snapshot.projects[0]
        .kinds
        .iter()
        .find(|k| k.id == ids::KIND_VULKAN_ANA)
        .expect("the ANA kind")
        .clone();

    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    bench.key(&mut section, "tab");
    type_at(&bench, &mut section, "x");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    // Someone else wrote to the row this editor opened on.
    let current = catalogue(
        serve(
            &backend,
            &StoreRequest::UpdateKind {
                scope: vulkan_scope(),
                id: ids::KIND_VULKAN_ANA,
                expected: opened.updated_at,
                patch: ItemKindPatch {
                    name: Some("analysis, renamed".to_owned()),
                    ..ItemKindPatch::default()
                },
            },
        )
        .await,
    );
    bench.reply(
        &mut section,
        &StoreReply::CatalogueStale(Box::new(current.clone())),
    );

    insta::assert_snapshot!("stale", bench.render_section(&section, 100));
    let flagged = error_text(&bench, &section, 100);
    assert!(
        flagged
            .iter()
            .any(|line| line.starts_with("changed elsewhere since you opened it")),
        "the miss is reported in `theme.error`: {flagged:?}"
    );

    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::UpdateKind {
            expected, patch, ..
        }),
    ] = asked.as_slice()
    else {
        panic!("`Enter` retries by hand, once: {asked:?}");
    };
    let reloaded = current.projects[0]
        .kinds
        .iter()
        .find(|k| k.id == ids::KIND_VULKAN_ANA)
        .expect("the ANA kind")
        .updated_at;
    assert_eq!(
        *expected, reloaded,
        "the retry carries the reloaded row's token, not the one the editor opened on"
    );
    assert_eq!(
        patch.name.as_deref(),
        Some("analysisx"),
        "and the text that was typed survived the reload"
    );
}

/// The same miss with the row gone: there is nothing to retry against, so the editor closes and
/// says why.
#[tokio::test]
async fn a_stale_reply_with_the_row_gone_closes_the_editor() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let mut without = snapshot.clone();
    without.projects[0]
        .kinds
        .retain(|k| k.id != ids::KIND_VULKAN_ANA);
    bench.reply(&mut section, &StoreReply::CatalogueStale(Box::new(without)));

    assert!(!section.captures_input(), "the editor is gone");
    assert!(
        error_text(&bench, &section, 100)
            .contains(&"deleted elsewhere \u{2014} the editor was closed".to_owned()),
        "and it says why"
    );
}

/// A miss that outlives the editor it was raised from says what the ordinary sentence cannot.
///
/// `Enter retries against the current row` names a row that is not on screen: the editor went with
/// the scope change (B-12), or a `Catalogue` that answered something else was taken for this
/// write's reply and closed it (H-9). Either way the write did **not** apply, and the notice is the
/// only thing that can say so and say how to get back to it.
#[tokio::test]
async fn a_stale_reply_with_no_editor_open_says_nothing_was_written() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    bench.key(&mut section, "tab");
    type_at(&bench, &mut section, "x");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    // The workspace changed under the write: the editor and its token went with it.
    section.on_scope_change(&nil_scope());
    assert!(!section.captures_input(), "nothing is being typed into");

    bench.reply(
        &mut section,
        &StoreReply::CatalogueStale(Box::new(snapshot.clone())),
    );

    let flagged = error_text(&bench, &section, 100);
    assert!(
        flagged.iter().any(|line| line
            == "changed elsewhere; nothing was written \u{2014} reopen the editor and retry"),
        "the miss says the write did not apply, and how to resume: {flagged:?}"
    );
}

/// MOD-141 D13: `ReplaceGraph`'s own reply redraws the tree the way `Catalogue` did, and its
/// `Stale` twin with no editor open says nothing was written, as `CatalogueStale` did.
#[tokio::test]
async fn a_graph_replaced_reply_redraws_the_tree_like_a_catalogue() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    assert!(!bench.render_section(&section, 100).contains("decide"));
    let mut renamed = snapshot.clone();
    let entry = renamed.projects[0]
        .graphs
        .iter_mut()
        .find(|entry| entry.graph.id == ids::GRAPH_VULKAN_ANA)
        .expect("the demo carries the analysis graph");
    for node in &mut entry.nodes {
        if node.phase.name == "verdict" {
            node.phase.name = "decide".to_owned();
        }
    }
    let body = Box::new(entry.clone());

    bench.reply(
        &mut section,
        &StoreReply::GraphReplaced {
            outcome: CasOutcome::Applied(body.clone()),
            catalogue: Box::new(renamed.clone()),
        },
    );

    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("decide"), "the tree is the reply's: {frame}");
    let flagged = error_text(&bench, &section, 100);
    assert!(
        flagged.is_empty(),
        "an applied write sets no notice: {flagged:?}"
    );

    bench.reply(
        &mut section,
        &StoreReply::GraphReplaced {
            outcome: CasOutcome::Stale(body),
            catalogue: Box::new(renamed),
        },
    );

    let flagged = error_text(&bench, &section, 100);
    assert!(
        flagged.iter().any(|line| line
            == "changed elsewhere; nothing was written \u{2014} reopen the editor and retry"),
        "the miss says the write did not apply, and how to resume: {flagged:?}"
    );
}

/// A prefix change is a warning before it is a write (PRD D12, D10): the old keys and their counter
/// survive, and the next item minted under the kind spells the new one. Nothing is sent until that
/// sentence has been on screen.
#[tokio::test]
async fn a_prefix_change_asks_before_it_writes() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    for _ in 0..3 {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "AN");

    bench.key(&mut section, "enter");

    assert!(
        bench.drained().is_empty(),
        "the warning is read before the write"
    );
    assert!(section.captures_input(), "and it is modal while it is up");
    let frame = bench.render_section(&section, 100);
    assert!(
        bench
            .key_hint(&section)
            .contains("y write \u{b7} n/Esc back to the editor"),
        "the hint names both answers: {frame}"
    );
    assert!(
        error_text(&bench, &section, 100).contains(
            &"items keyed ANA-* keep their keys and their counter; the next item minted under this \
              kind is AN-1."
                .to_owned()
        ),
        "the warning states PRD D12's semantics, in `theme.error`: {frame}"
    );
    insta::assert_snapshot!("prefix_warn", frame);
}

/// `n` on the warning is a way back, not a cancel: the editor comes back with its text, and a
/// second `Enter` asks again (D10).
#[tokio::test]
async fn n_on_the_prefix_warning_returns_to_the_editor_with_its_text() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    for _ in 0..3 {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "AN");
    bench.key(&mut section, "enter");

    bench.key(&mut section, "n");

    let frame = bench.render_section(&section, 100);
    assert!(
        bench.key_hint(&section).contains("Tab/Shift+Tab field"),
        "the editor is back: {frame}"
    );
    assert_eq!(
        field_value(&frame, "prefix").as_deref(),
        Some("AN"),
        "with its text: {frame}"
    );
    assert!(bench.drained().is_empty());

    bench.key(&mut section, "enter");
    bench.key(&mut section, "y");
    let asked = bench.drained();
    let [Action::Store(StoreRequest::UpdateKind { patch, .. })] = asked.as_slice() else {
        panic!("`y` writes once: {asked:?}");
    };
    assert_eq!(patch.prefix.as_deref(), Some("AN"));
}

/// `y` writes once, and the editor stays open until the reply so a compare-and-set miss can
/// re-take the token (D8, D10).
#[tokio::test]
async fn y_on_the_prefix_warning_writes_once() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    for _ in 0..3 {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "AN");
    bench.key(&mut section, "enter");
    assert!(
        bench.drained().is_empty(),
        "`Enter` only raised the warning"
    );

    bench.key(&mut section, "y");

    let asked = bench.drained();
    let [Action::Store(StoreRequest::UpdateKind { id, patch, .. })] = asked.as_slice() else {
        panic!("`y` is the only key that writes: {asked:?}");
    };
    assert_eq!(*id, ids::KIND_VULKAN_ANA);
    assert_eq!(patch.prefix.as_deref(), Some("AN"));
    assert!(section.captures_input(), "the editor is still open");

    bench.key(&mut section, "enter");
    assert!(bench.drained().is_empty(), "one write of a kind at a time");
    assert!(
        error_text(&bench, &section, 100)
            .contains(&"still saving the kind \u{2014} wait for the store's answer".to_owned())
    );
}

/// MOD-141 D14: `n` on a graph or a phase row points at the graph editor, where steps are added,
/// naming the key in force; nothing is sent and no form opens.
#[tokio::test]
async fn n_on_a_graph_or_phase_points_at_the_graph_editor() {
    let (bench, mut section) = bench_with_unreferenced_analysis().await;
    for row in ["analysis \u{b7} graph, no kind", ". verdict "] {
        cursor_to(&bench, &mut section, row);
        bench.key(&mut section, "n");
        assert!(bench.drained().is_empty(), "{row}");
        assert!(!section.captures_input(), "{row}: no form opens");
        assert!(
            error_text(&bench, &section, 100).contains(
                &"steps are added and deleted in the graph editor: Enter opens it".to_owned()
            ),
            "{row}"
        );
        bench.key(&mut section, "esc");
    }
}

// -------------------------------------------------------------------------------------------
// The phase editor's `persona` field (MOD-26 M2 D23, OQ-11): typed like every other field, blank
// for none, resolved against the catalogue's persona list before anything is sent.
// -------------------------------------------------------------------------------------------

/// A bench over the demo world with vulkan `analysis/research` bound to `persona` (or left unbound
/// for `None`), the catalogue read back through the worker and delivered to a fresh section.
async fn bench_with_bound(
    persona: Option<PersonaId>,
) -> (SectionBench, KindsSection, CatalogueSnapshot) {
    let store = MemStore::demo();
    let research = store
        .phases(ids::GRAPH_VULKAN_ANA)
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|phase| phase.name == "research")
        .expect("the analysis graph's `research`");
    let outcome = store
        .update_phase(
            research.id,
            research.updated_at,
            PhasePatch {
                persona: Some(persona),
                ..PhasePatch::default()
            },
        )
        .await
        .expect("the binding is written");
    assert!(
        matches!(outcome, CasOutcome::Applied(_)),
        "against the row's own token: {outcome:?}"
    );
    let snapshot = catalogue(
        serve(
            &Backend::memory(store),
            &StoreRequest::Catalogue(vulkan_scope()),
        )
        .await,
    );
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Catalogue(Box::new(snapshot.clone())),
    );
    let _ = bench.drained();
    (bench, section, snapshot)
}

/// A binding to an id the loaded list does not know — a persona created after the catalogue was
/// read — prints as the id (D23). Editing it is the graph editor's (MOD-141 D14).
#[tokio::test]
async fn an_id_the_list_does_not_know_prints_as_the_id() {
    let unknown = PersonaId::new();
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let mut snapshot = demo_catalogue(&demo()).await;
    snapshot.projects[0]
        .graphs
        .iter_mut()
        .find(|entry| entry.graph.id == ids::GRAPH_VULKAN_ANA)
        .expect("the analysis graph")
        .nodes[0]
        .phase
        .persona_id = Some(unknown);
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();

    let frame = bench.render_section(&section, 160);
    let line = frame
        .lines()
        .find(|line| line.contains(". research "))
        .unwrap_or_else(|| panic!("the research row: {frame}"));
    assert!(
        line.trim_end()
            .ends_with(&format!(" \u{b7} persona {unknown}")),
        "an id the list does not know prints as the id: {line}"
    );
}

/// A bound phase's row ends with its persona's name; an unbound one's is exactly what it was
/// (D23, I-12).
#[tokio::test]
async fn a_bound_phase_line_names_its_persona() {
    let (bench, section, _) = bench_with_bound(Some(ids::PERSONA_REVIEWER)).await;

    let frame = bench.render_section(&section, 100);
    let research = frame
        .lines()
        .find(|line| line.contains(". research "))
        .unwrap_or_else(|| panic!("the research row: {frame}"));
    assert!(
        research
            .trim_end()
            .ends_with(" \u{b7} budget inherited \u{b7} persona reviewer"),
        "the bound row names its persona last: {research}"
    );
    let verdict = frame
        .lines()
        .find(|line| line.contains(". verdict "))
        .unwrap_or_else(|| panic!("the verdict row: {frame}"));
    assert!(
        !verdict.contains("persona"),
        "an unbound row says nothing of personas: {verdict}"
    );
    insta::assert_snapshot!("phase_persona", frame);
}

/// `d` on a kind asks once and writes nothing until `y` (D11).
///
/// One confirmation and not PRD D13's two: a kind any item holds cannot be deleted at all, so the
/// destructive case the typed slug exists for does not arise here.
#[tokio::test]
async fn d_on_a_kind_asks_once() {
    let (bench, mut section, _) = bench_with_demo().await;
    // project, ANA, its two phases, FEAT.
    for _ in 0..4 {
        bench.key(&mut section, "j");
    }

    bench.key(&mut section, "d");

    assert!(bench.drained().is_empty(), "the question is not a write");
    assert!(section.captures_input(), "and it is modal while it is up");
    let frame = bench.render_section(&section, 100);
    assert!(
        error_text(&bench, &section, 100).contains(
            &"delete kind feature (FEAT)? a kind any item uses is refused. y delete \u{b7} n/Esc \
              stop"
                .to_owned()
        ),
        "the question names the kind and what the store will refuse: {frame}"
    );
    insta::assert_snapshot!("delete_ask", frame);
}

/// `y` sends the delete, and a kind an item holds comes back refused in the seam's own words
/// (D11, F-7) — the section neither counts the items nor phrases the refusal.
#[tokio::test]
async fn y_sends_delete_kind_and_a_refusal_is_the_seams_sentence() {
    let (bench, mut section, _) = bench_with_demo().await;
    for _ in 0..4 {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "d");

    bench.key(&mut section, "y");

    let asked = bench.drained();
    let [Action::Store(StoreRequest::DeleteKind { scope, id })] = asked.as_slice() else {
        panic!("`y` deletes once: {asked:?}");
    };
    assert_eq!(*scope, bench_scope().await);
    assert_eq!(*id, ids::KIND_VULKAN_FEAT);
    let waiting = bench.render_section(&section, 100);
    assert!(
        waiting.contains("deleting the kind\u{2026}"),
        "the delete in flight reads in words: {waiting}"
    );
    assert!(!waiting.contains("delete_kind"), "{waiting}");

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "delete_kind",
            message: item_kind_is_held("FEAT", 1),
        },
    );

    assert!(!section.captures_input(), "the question has been answered");
    assert!(
        error_text(&bench, &section, 100).contains(&"item_kind FEAT is held by 1 items".to_owned()),
        "the seam's sentence reaches the user untouched"
    );
}

/// A kind that is gone reports what happened to the mirror with it (D12): the Backlog must not
/// keep offering a kind no row answers for.
#[tokio::test]
async fn a_kind_delete_reports_the_mirror() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "d");
    bench.key(&mut section, "y");
    let _ = bench.drained();

    let mut after = snapshot.clone();
    after.projects[0]
        .kinds
        .retain(|k| k.id != ids::KIND_VULKAN_ANA);
    bench.reply(
        &mut section,
        &StoreReply::KindDeleted {
            mirror: MirrorAfterDelete::Rebuilt,
            catalogue: Box::new(after),
        },
    );

    assert!(!section.captures_input(), "back to the tree");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("deleted kind `ANA`; mirror rebuilt"),
        "the notice says what went and what was rebuilt: {frame}"
    );
    assert!(
        !frame.contains("ANA  analysis"),
        "the kind is gone: {frame}"
    );
    assert!(
        frame.contains("analysis \u{b7} graph, no kind"),
        "and its graph is now the one nothing points at (D4, D5): {frame}"
    );
}

/// `d` elsewhere says where deletes live: graphs have no seam method at all (D5), steps are deleted
/// in the graph editor (MOD-141 D14), and projects belong to the hierarchy section (B-13).
#[tokio::test]
async fn d_on_a_phase_or_graph_is_refused() {
    let (bench, mut section) = bench_with_unreferenced_analysis().await;

    bench.key(&mut section, "d");
    assert!(bench.drained().is_empty());
    assert!(
        error_text(&bench, &section, 100).contains(&"projects are edited in Hierarchy".to_owned())
    );

    cursor_to(&bench, &mut section, "analysis \u{b7} graph, no kind");
    bench.key(&mut section, "d");
    assert!(bench.drained().is_empty());
    assert!(
        error_text(&bench, &section, 100).contains(&"graphs are not deleted here".to_owned()),
        "a graph row says so in the section's own words"
    );

    cursor_to(&bench, &mut section, ". verdict ");
    bench.key(&mut section, "d");
    assert!(bench.drained().is_empty());
    assert!(!section.captures_input(), "no question is asked");
    assert!(
        error_text(&bench, &section, 100).contains(
            &"steps are added and deleted in the graph editor: Enter opens it".to_owned()
        ),
        "a phase row points at the graph editor"
    );
}

/// MOD-80 review L1, MOD-115 M3: the rows a section drew in the kit's selection band, as
/// `(y, text)`: the `▌` gutter in the tab's accent at column 0, then the row black on cyan over
/// the whole width so no span's own colour shows through the cursor.
fn selected_rows(buffer: &Buffer) -> Vec<(u16, String)> {
    let selected = Theme::default().selected;
    let accent = Theme::default().tab_accent("settings");
    let (fg, bg) = (
        selected.fg.expect("selected has a foreground"),
        selected.bg.expect("selected has a background"),
    );
    (0..buffer.area.height)
        .filter(|y| buffer[(1, *y)].bg == bg)
        .map(|y| {
            let gutter = &buffer[(0, y)];
            assert_eq!(gutter.symbol(), "\u{258c}", "the gutter marks row {y}");
            assert_eq!(gutter.fg, accent.fg.unwrap_or(Color::Reset), "row {y}");
            let text: String = (1..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned();
            for x in 1..buffer.area.width {
                let cell = &buffer[(x, y)];
                assert_eq!((cell.fg, cell.bg), (fg, bg), "{text:?} at x = {x}");
            }
            (y, text)
        })
        .collect()
}

/// MOD-80 review L1: the Kinds cursor row is drawn in `theme.selected`, as Connection's is, and a
/// kind whose graph is missing is one block too, its `graph missing` tail included.
#[tokio::test]
async fn the_cursor_row_is_drawn_selected() {
    let (bench, mut section, _) = bench_with_demo().await;
    let on_project = selected_rows(&drawn(&bench, &section, 100));
    assert_eq!(on_project.len(), 1, "{on_project:?}");
    bench.key(&mut section, "j");
    let on_kind = selected_rows(&drawn(&bench, &section, 100));
    assert_eq!(on_kind.len(), 1, "{on_kind:?}");
    assert_eq!(
        on_kind[0].0,
        on_project[0].0 + 1,
        "the cursor moved one row"
    );
    assert!(on_kind[0].1.trim_start().starts_with("ANA"), "{on_kind:?}");

    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let mut snapshot = demo_catalogue(&demo()).await;
    snapshot.projects[0].kinds[0].default_graph_id = StepGraphId::new();
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();
    bench.key(&mut section, "j");
    let missing = selected_rows(&drawn(&bench, &section, 100));
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0].1.trim(), "ANA  analysis \u{b7} graph missing");
}

// -------------------------------------------------------------------------------------------
// MOD-67 M3 (lane L-C): Browse dispatches through `views::KINDS_BROWSE`, so a chord matches with
// its modifiers, and every mode resolves through its own stack.
// -------------------------------------------------------------------------------------------

/// ANA-26 §2.6 defect 1 (MOD-67 D14): `ctrl-d` on a kind is not `d`. It used to open the delete
/// question, because Browse matched `key.code` alone; the other browse letters with `CONTROL`
/// held open or send nothing either.
#[tokio::test]
async fn ctrl_d_in_browse_opens_no_delete_question() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");

    assert_eq!(bench.key(&mut section, "ctrl-d"), Handled::Pass);
    assert!(!section.captures_input(), "no question was opened");
    assert!(
        !bench
            .render_section(&section, 100)
            .contains("a kind any item uses is refused"),
        "no question on screen"
    );

    for chord in ["ctrl-n", "ctrl-g", "ctrl-o", "ctrl-e", "ctrl-r"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Pass, "{chord}");
        assert!(!section.captures_input(), "{chord} opened nothing");
        assert!(bench.drained().is_empty(), "{chord} asked nothing");
    }
}

/// MOD-67 D5: an `ALT` chord is not an answer. `alt-y` at the delete question used to send the
/// delete (only `CONTROL` was carved out); now it is swallowed and the question stays.
#[tokio::test]
async fn alt_y_does_not_answer_the_delete_question() {
    let (bench, mut section, _) = bench_with_demo().await;
    for _ in 0..4 {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "d");

    bench.key(&mut section, "alt-y");
    assert!(bench.drained().is_empty(), "`alt-y` deleted nothing");
    assert!(section.captures_input(), "the question is still up");
    assert_eq!(bench.key(&mut section, "q"), Handled::Consumed, "and modal");
    assert_eq!(
        bench.key(&mut section, "ctrl-c"),
        Handled::Pass,
        "`ctrl-c` passes"
    );
}

/// MOD-67 M3 `VIEW_DEFAULTS`: the editor moves its focus on `Down` and `Up` as on `Tab` and
/// `Shift+Tab`.
#[tokio::test]
async fn the_editor_moves_focus_on_down_and_up() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "n");

    assert_eq!(bench.key(&mut section, "down"), Handled::Consumed);
    type_at(&bench, &mut section, "docs");
    assert_eq!(bench.key(&mut section, "up"), Handled::Consumed);
    type_at(&bench, &mut section, "DOC");

    let frame = bench.render_section(&section, 100);
    assert_eq!(
        field_value(&frame, "prefix").as_deref(),
        Some("DOC"),
        "{frame}"
    );
    assert_eq!(
        field_value(&frame, "name").as_deref(),
        Some("docs"),
        "{frame}"
    );
}

/// MOD-67 D14, a second rebinding: `[settings.kinds] graph = "G"` opens the graph editor on `G`,
/// leaves `g` inert, and the hint row follows.
#[tokio::test]
async fn a_rebound_graph_acts_on_its_new_chord_and_the_hint_follows() {
    let keys = htui::keys::load_str("version = 1\n[settings.kinds]\ngraph = \"G\"\n")
        .expect("the key file loads");
    let (bench, mut section, _) = bench_with_demo().await;
    let bench = bench.with_keys(keys);
    bench.key(&mut section, "j");

    assert!(
        bench
            .key_hint(&section)
            .contains("e edit \u{b7} G graph \u{b7} Enter graph editor \u{b7} d delete kind"),
        "{}",
        bench.key_hint(&section)
    );
    assert_eq!(bench.key(&mut section, "g"), Handled::Pass);
    assert!(!section.captures_input(), "`g` is unbound now");
    assert_eq!(bench.key(&mut section, "G"), Handled::Consumed);
    assert_eq!(
        field_value(&bench.render_section(&section, 100), "name").as_deref(),
        Some("analysis"),
        "`G` opened the kind's graph"
    );
}

// -------------------------------------------------------------------------------------------
// MOD-115 M3 (lane L-kinds): the section on the kit. The tree is a `kit::list`, the editor a
// `kit::form`, the notices `kit::text` lines and the empty states `kit::empty` with a next
// step; every hint and every sentence that names a key reads it from the keys in force.
// -------------------------------------------------------------------------------------------

/// `field.submit` and `field.cancel` moved (MOD-115 M3 C6): the editor's `Enter` and `Esc`.
const FIELD_REBOUND: &str = "version = 2\n[field]\nsubmit = [\"ctrl-t\"]\ncancel = [\"ctrl-o\"]\n";

/// `keys` loaded, or a panic naming the file.
fn rebound(file: &str) -> htui::keys::Keys {
    htui::keys::load_str(file).expect("the key file loads")
}

/// One section drawn into a `width`x`height` buffer.
fn drawn_at(
    bench: &SectionBench,
    section: &dyn SettingsSection,
    width: u16,
    height: u16,
) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut terminal = Terminal::with_options(
        TestBackend::new(width, height),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("a test terminal");
    let ctx = bench.ctx();
    terminal
        .draw(|frame| section.render(frame, frame.area(), &ctx))
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// The editor's hint names the field's own keys through the editor's stack (MD-A, C6): `Enter
/// save · Esc cancel` under the compiled keys, the rebound chords once `[field]` moves them.
#[tokio::test]
async fn the_editor_hint_names_the_field_keys_in_force() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    let hint = bench.key_hint(&section);
    assert!(
        hint.contains("Tab/Shift+Tab field \u{b7} Enter save \u{b7} Esc cancel"),
        "{hint}"
    );

    let bench = bench.with_keys(rebound(FIELD_REBOUND));
    let hint = bench.key_hint(&section);
    assert!(hint.contains("Ctrl+t save \u{b7} Ctrl+o cancel"), "{hint}");
    assert!(!hint.contains("Enter"), "no literal key is left: {hint}");
}

/// A compare-and-set miss under an open editor names the key that retries as the editor's stack
/// binds it (MD-A, D-5): `Enter` under the compiled keys, the rebound chord otherwise.
#[tokio::test]
async fn a_stale_reply_names_the_retry_key_in_force() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    let bench = bench.with_keys(rebound(FIELD_REBOUND));
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::CatalogueStale(Box::new(snapshot)),
    );

    let flagged = error_text(&bench, &section, 100);
    assert!(
        flagged.iter().any(|line| line
            == "changed elsewhere since you opened it \u{2014} reloaded; Ctrl+t retries against \
                the current row"),
        "{flagged:?}"
    );
}

/// MOD-83: a phase row reads its gate, inputs and budget as words, not as column values.
#[tokio::test]
async fn a_phase_row_reads_its_gate_inputs_and_budget_in_words() {
    let (bench, section, _) = bench_with_demo().await;
    let frame = bench.render_section(&section, 100);
    let row = |name: &str| -> String {
        frame
            .lines()
            .find(|line| line.contains(&format!(". {name} ")))
            .unwrap_or_else(|| panic!("the {name} row: {frame}"))
            .trim()
            .to_owned()
    };
    assert_eq!(
        row("research"),
        "0. research \u{b7} template research \u{b7} soft gate \u{b7} no inputs \u{b7} budget \
         inherited"
    );
    assert_eq!(
        row("verdict"),
        "1. verdict \u{b7} template verdict \u{b7} hard gate \u{b7} inputs research \u{b7} \
         budget inherited"
    );
    assert!(
        frame.contains("inputs plan, review"),
        "a list of inputs reads as one: {frame}"
    );
    // `budget inherited` (MOD-115 M3 L-kinds deviation from §5.4's `inherits the budget`): three
    // cells shorter, so a bound phase's persona still fits a 100-column row. The raw column read
    // `budget inherit` followed by a separator.
    for raw in ["gate soft", "gate hard", "in none", "budget inherit "] {
        assert!(!frame.contains(raw), "`{raw}` is gone: {frame}");
    }
}

/// MOD-83: the read-only line under the selected phase names its columns in words and shows the
/// gate, isolation and command queue through their labels (D15 kept: MOD-4 still owns them).
#[tokio::test]
async fn the_read_only_line_names_its_columns_in_words() {
    let (bench, mut section, mut snapshot) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "j");
    let frame = bench.render_section(&section, 100);
    assert!(
        flat(&frame).contains(
            "fan-out 1 \u{b7} gate: always \u{b7} isolation: project default \u{b7} commands: \
             queued when fanned out \u{b7} verify: none \u{b7} retries 1 \u{b7} output: research \
             \u{b7} template: latest"
        ),
        "{frame}"
    );
    for raw in [
        "fan_out",
        "command_queue",
        "verify_command",
        "retry_limit",
        "output_kind",
        "template_version",
    ] {
        assert!(!frame.contains(raw), "`{raw}` is not on screen: {frame}");
    }

    let phase = &mut snapshot.projects[0]
        .graphs
        .iter_mut()
        .find(|entry| entry.graph.id == ids::GRAPH_VULKAN_ANA)
        .expect("the analysis graph")
        .nodes
        .get_mut(0)
        .expect("research")
        .phase;
    phase.isolation = Some(Isolation::Local);
    phase.template_version = Some(3);
    phase.verify_command = Some("cargo test".to_owned());
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let frame = flat(&bench.render_section(&section, 100));
    for words in [
        "isolation: none (local tree)",
        "verify: cargo test",
        "template: v3",
    ] {
        assert!(frame.contains(words), "`{words}`: {frame}");
    }
}

/// MOD-85: in the editor the accent marks the focused row's gutter and the tree's cursor, and
/// nothing else: no label is drawn in it.
#[tokio::test]
async fn the_editor_marks_its_focused_row_and_no_label_takes_the_accent() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    bench.key(&mut section, "tab");
    let frame = bench.render_section(&section, 100);
    let name = u16::try_from(field_line(&frame, "name").expect("a `name` row")).expect("a row");
    let accent = bench
        .ctx()
        .accent()
        .fg
        .expect("the Settings accent has a colour");
    let buffer = drawn(&bench, &section, 100);
    let accented: Vec<(u16, u16)> = (0..buffer.area.height)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .filter(|(x, y)| buffer[(*x, *y)].fg == accent)
        .collect();
    assert!(
        accented
            .iter()
            .all(|(x, y)| *x == 0 && buffer[(*x, *y)].symbol() == "\u{258c}"),
        "only gutters take the accent: {accented:?}"
    );
    assert!(
        accented.contains(&(0, name)),
        "the focused field is marked: {accented:?} {frame}"
    );
    assert_eq!(
        accented.len(),
        2,
        "that field and the tree's cursor: {accented:?}"
    );
}

/// The empty states say what is missing and what to do next (MOD-115 audit item 2).
#[tokio::test]
async fn the_empty_states_carry_a_next_step() {
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("no workspace: nothing to list"), "{frame}");
    assert!(frame.contains("r reads the catalogue again"), "{frame}");

    let mut snapshot = demo_catalogue(&demo()).await;
    snapshot.projects.clear();
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("no project in scope"), "{frame}");
    assert!(
        frame.contains("Settings \u{203a} Hierarchy adds a project"),
        "{frame}"
    );

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "catalogue",
            message: "store unreachable".to_owned(),
        },
    );
    let flagged = error_text(&bench, &section, 100);
    assert!(
        flagged.contains(&"catalogue unavailable: store unreachable".to_owned()),
        "the refusal is an error: {flagged:?}"
    );
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("r asks again once the store answers"),
        "{frame}"
    );
}

/// The section reports its scroll for the Settings pane's scrollbar (MOD-115 M3 D-11): the
/// tree's rows and the first one drawn, the cursor kept on screen.
#[tokio::test]
async fn the_section_reports_its_scroll_for_the_pane() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    assert_eq!(
        section.scroll().map(|span| (span.total, span.offset)),
        None,
        "nothing is drawn yet"
    );
    let rows = 1 + snapshot.projects[0]
        .kinds
        .iter()
        .map(|kind| {
            1 + snapshot.projects[0]
                .graph(kind.default_graph_id)
                .map_or(0, |entry| entry.nodes.len())
        })
        .sum::<usize>();

    // The tree's own rows (RV1 M2): the notice line under it is not padded in.
    let _ = drawn_at(&bench, &section, 100, 10);
    assert_eq!(
        section.scroll().map(|span| (span.total, span.offset)),
        Some((rows, 0))
    );
    let tree = section
        .scroll()
        .and_then(|span| span.rows)
        .expect("the tree's rows");
    assert_eq!(tree.height, 9, "the rows above the notice line");

    // The last kind, `TOOL`, with its three phases under it: a kind row has no detail line.
    let tool = rows - 4;
    for _ in 0..tool {
        bench.key(&mut section, "j");
    }
    let buffer = drawn_at(&bench, &section, 100, 10);
    // Nine rows above the notice line; the cursor's is the last of them.
    assert_eq!(
        section.scroll().map(|span| (span.total, span.offset)),
        Some((rows, tool - 8))
    );
    let last: String = (0..100).map(|x| buffer[(x, 8)].symbol()).collect();
    assert!(
        last.starts_with('\u{258c}') && last.contains("TOOL"),
        "the cursor is on screen: {last}"
    );
}

/// The demo tree's line count: the project row, then each kind's row and its graph's phases.
fn tree_lines(snapshot: &CatalogueSnapshot) -> usize {
    1 + snapshot.projects[0]
        .kinds
        .iter()
        .map(|kind| {
            1 + snapshot.projects[0]
                .graph(kind.default_graph_id)
                .map_or(0, |entry| entry.nodes.len())
        })
        .sum::<usize>()
}

/// One section drawn inside a kit pane as the Settings tab draws it (MOD-115 M3 C3, D-11): the
/// section into the pane's inner rect first, then the frame with the section's scroll as its bar.
fn framed_at(
    bench: &SectionBench,
    section: &dyn SettingsSection,
    width: u16,
    height: u16,
) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut terminal = Terminal::with_options(
        TestBackend::new(width, height),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("a test terminal");
    let ctx = bench.ctx();
    let theme = Theme::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            section.render(frame, Pane::inner(area), &ctx);
            let mut pane = Pane::new(section.title());
            if let Some(span) = section.scroll() {
                pane = pane.scroll(span);
            }
            let _ = pane.render(frame, area, &theme);
        })
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// The rows of the right border the scrollbar's thumb (`┃`) is drawn on.
fn thumb_rows(buffer: &Buffer) -> Vec<u16> {
    let x = buffer.area.width - 1;
    (0..buffer.area.height)
        .filter(|y| buffer[(x, *y)].symbol() == "\u{2503}")
        .collect()
}

/// MOD-115 M3 D-11, RV1 M2: the pane measures its bar against the rows the section says it
/// scrolls, the tree's. A thumb shows exactly when a tree row is hidden: the notice line, and an
/// open editor, are rows under the tree it cannot use.
#[tokio::test]
async fn the_pane_draws_a_thumb_exactly_when_a_tree_row_is_hidden() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    let rows = tree_lines(&snapshot);
    // The borders, the tree, the notice line: every tree row fits.
    let height = u16::try_from(rows + 3).expect("a small tree");
    let fits = framed_at(&bench, &section, 100, height);
    assert!(thumb_rows(&fits).is_empty(), "nothing is hidden, no bar");

    // One row less: the last phase is off screen, and the bar says so.
    let cut = framed_at(&bench, &section, 100, height - 1);
    assert!(!thumb_rows(&cut).is_empty(), "a hidden row shows a thumb");

    // An editor under the tree, opened on the last phase (`N`, a new graph: MOD-141 D14 took the
    // phase editor into the graph editor): the tree scrolls to its end, the thumb with it.
    for _ in 1..rows {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "N");
    assert!(section.captures_input(), "the graph form is open");
    let editing = framed_at(&bench, &section, 100, height);
    let thumb = thumb_rows(&editing);
    assert!(!thumb.is_empty(), "the form hides tree rows");
    // RV1 M2: beside the tree's rows only, never the editor's under them.
    let tree = section
        .scroll()
        .and_then(|span| span.rows)
        .expect("the tree's rows");
    assert!(
        tree.bottom() < height - 1,
        "the editor takes rows: {tree:?}"
    );
    assert_eq!(
        thumb.last(),
        Some(&(tree.bottom() - 1)),
        "scrolled to the end, the thumb sits beside the tree's last row: {thumb:?}"
    );
    assert!(
        thumb.iter().all(|y| (tree.y..tree.bottom()).contains(y)),
        "{thumb:?}"
    );
}

/// MOD-81: the section at 80 columns, through the shell.
#[tokio::test]
async fn the_section_at_80_columns() {
    let mut harness = Harness::over(MemStore::demo())
        .size(80, 24)
        .with_tab(Box::new(SettingsTab::with_sections(vec![
            Box::new(AgentsSection::new()),
            Box::new(HierarchySection::new()),
            Box::new(KindsSection::new()),
        ])));
    harness.settle().await;
    harness.key("l");
    harness.key("l");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("soft gate"), "{frame}");
    assert!(
        frame.contains('\u{2503}'),
        "the tree overflows the pane, so the pane shows a thumb: {frame}"
    );
    insta::assert_snapshot!("demo_80", frame);
}

// -------------------------------------------------------------------------------------------
// MOD-141 T7: the graph editor, hosted over the whole section (D2, D3, D14)
// -------------------------------------------------------------------------------------------

/// The store requests the section emitted since the last drain, in order.
fn store_requests(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// Presses `j` until the cursor's row reads `needle`.
#[track_caller]
fn cursor_to(bench: &SectionBench, section: &mut KindsSection, needle: &str) {
    for _ in 0..64 {
        let rows = selected_rows(&drawn(bench, section, 100));
        if rows.iter().any(|(_, text)| text.contains(needle)) {
            return;
        }
        bench.key(section, "j");
    }
    panic!(
        "no row reads `{needle}`: {}",
        bench.render_section(section, 100)
    );
}

/// The cursor's row, as drawn.
#[track_caller]
fn cursor_row(bench: &SectionBench, section: &KindsSection) -> String {
    let rows = selected_rows(&drawn(bench, section, 100));
    let [(_, text)] = rows.as_slice() else {
        panic!("one cursor row: {rows:?}");
    };
    text.clone()
}

/// Answers the editor's two reads (`Agents`, `Templates`) from the demo world, as the worker would.
async fn answer_lookups(bench: &SectionBench, section: &mut KindsSection) {
    let backend = demo();
    for request in store_requests(bench) {
        if matches!(request, StoreRequest::Agents | StoreRequest::Templates(_)) {
            let reply = serve(&backend, &request).await;
            bench.reply(section, &reply);
        }
    }
    let _ = bench.drained();
}

/// The demo catalogue with the ANA kind pointed away from its graph, so `analysis` is listed on
/// its own `Graph` row, its phases under it, at the end of the tree.
async fn bench_with_unreferenced_analysis() -> (SectionBench, KindsSection) {
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    let mut snapshot = demo_catalogue(&demo()).await;
    snapshot.projects[0].kinds[0].default_graph_id = StepGraphId::new();
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();
    (bench, section)
}

/// The editor's header names the graph, its scope and its revision.
const ANALYSIS_HEADER: &str = "analysis \u{b7} project graph \u{b7} revision";

/// MOD-141 D3: `Enter` on a graph row opens the editor over the section; it asks for the agents
/// and the templates its step form picks from, and the section's keys are the editor's.
#[tokio::test]
async fn enter_on_a_graph_row_opens_the_editor_and_asks_for_agents_and_templates() {
    let (bench, mut section) = bench_with_unreferenced_analysis().await;
    cursor_to(&bench, &mut section, "analysis \u{b7} graph, no kind");

    assert_eq!(bench.key(&mut section, "enter"), Handled::Consumed);

    let asked = store_requests(&bench);
    let scope = bench_scope().await;
    assert!(
        matches!(
            asked.as_slice(),
            [StoreRequest::Agents, StoreRequest::Templates(asked_scope)] if *asked_scope == scope
        ),
        "{asked:?}"
    );
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));
    assert!(
        section.captures_input(),
        "the tab's `h`/`l` are the editor's"
    );
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(ANALYSIS_HEADER), "{frame}");
    let wanted = section.wants_requests(&scope);
    assert!(
        matches!(
            wanted.as_slice(),
            [
                StoreRequest::Catalogue(_),
                StoreRequest::Agents,
                StoreRequest::Templates(_)
            ]
        ),
        "an open editor reads again on activation: {wanted:?}"
    );
}

/// MOD-141 D3: `Enter` on a phase row opens the editor on that step: the editor's own `Enter`
/// then edits it.
#[tokio::test]
async fn enter_on_a_phase_row_opens_the_editor_on_that_step() {
    let (bench, mut section, _) = bench_with_demo().await;
    cursor_to(&bench, &mut section, ". verdict ");

    bench.key(&mut section, "enter");
    answer_lookups(&bench, &mut section).await;
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));

    bench.key(&mut section, "enter");
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR_NODE_FORM));
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("step verdict"), "{frame}");
    assert!(
        store_requests(&bench).is_empty(),
        "opening a form writes nothing"
    );
}

/// MOD-141 D14: `e` on a phase row opens the editor with that step's form already open; `Esc`
/// cancels the form back to the canvas, and the editor stays.
#[tokio::test]
async fn e_on_a_phase_row_opens_its_step_form_in_the_editor() {
    let (bench, mut section, _) = bench_with_demo().await;
    cursor_to(&bench, &mut section, ". verdict ");

    bench.key(&mut section, "e");

    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR_NODE_FORM));
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("step verdict"), "{frame}");
    assert!(frame.contains(ANALYSIS_HEADER), "{frame}");
    let asked = store_requests(&bench);
    assert!(
        matches!(
            asked.as_slice(),
            [StoreRequest::Agents, StoreRequest::Templates(_)]
        ),
        "only the editor's reads: {asked:?}"
    );

    bench.key(&mut section, "esc");
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));
}

/// MOD-141 D3: `Enter` on a kind opens its default graph when that is one of the project's own;
/// a shared graph is refused in the words `g` uses, and the project row points at a graph row.
#[tokio::test]
async fn enter_on_a_kind_opens_its_project_graph_and_refuses_a_shared_one() {
    let (bench, mut section, snapshot) = bench_with_demo().await;

    bench.key(&mut section, "enter");
    assert_eq!(section.key_stack(), Some(views::KINDS_BROWSE));
    assert!(store_requests(&bench).is_empty());
    assert!(
        error_text(&bench, &section, 100)
            .contains(&"select a graph or a step: Enter opens the graph editor".to_owned()),
        "{:?}",
        error_text(&bench, &section, 100)
    );

    cursor_to(&bench, &mut section, "ANA  analysis");
    bench.key(&mut section, "enter");
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(ANALYSIS_HEADER), "{frame}");

    // The same kind, pointed at a library graph: the section edits none of those.
    let mut shared = snapshot;
    let now = Utc::now();
    let library = StepGraph {
        id: StepGraphId::new(),
        scope: GraphScope::Library,
        name: "triage".to_owned(),
        description: String::new(),
        is_override: false,
        revision: 0,
        created_at: now,
        updated_at: now,
    };
    shared.projects[0].kinds[0].default_graph_id = library.id;
    shared.projects[0].shared.push(library);
    let bench = SectionBench::new().await;
    let mut section = KindsSection::new();
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(shared)));
    let _ = bench.drained();
    cursor_to(&bench, &mut section, "ANA  analysis");
    bench.key(&mut section, "enter");
    assert_eq!(section.key_stack(), Some(views::KINDS_BROWSE));
    assert!(store_requests(&bench).is_empty());
    assert!(
        error_text(&bench, &section, 100)
            .contains(&"graph triage \u{b7} library is not edited here".to_owned()),
        "{:?}",
        error_text(&bench, &section, 100)
    );
}

/// MOD-141 D2: while the editor is open the Settings tab's `h`/`l` are the editor's (it captures
/// input), so the sections do not cycle under it.
#[tokio::test]
async fn while_the_editor_is_open_h_and_l_do_not_cycle_sections() {
    let mut harness = kinds_over(MemStore::demo()).await;
    harness.key("j");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains(ANALYSIS_HEADER), "{frame}");

    for chord in ["l", "h", "l"] {
        harness.key(chord);
        harness.settle().await;
        let frame = harness.render();
        assert!(
            frame.contains(ANALYSIS_HEADER),
            "`{chord}` moved inside the editor, not the strip: {frame}"
        );
    }
}

/// MOD-141 D2: closing the editor returns to Browse with the cursor on the first row that shows
/// the graph: its kind, or its own row when no kind points at it.
#[tokio::test]
async fn closing_the_editor_returns_to_browse_with_the_cursor_on_the_graph() {
    let (bench, mut section, _) = bench_with_demo().await;
    cursor_to(&bench, &mut section, ". verdict ");
    bench.key(&mut section, "enter");
    assert!(section.captures_input());

    assert_eq!(bench.key(&mut section, "esc"), Handled::Consumed);

    assert!(!section.captures_input());
    assert_eq!(section.key_stack(), Some(views::KINDS_BROWSE));
    assert!(
        cursor_row(&bench, &section).contains("ANA  analysis"),
        "{}",
        bench.render_section(&section, 100)
    );

    let (bench, mut section) = bench_with_unreferenced_analysis().await;
    cursor_to(&bench, &mut section, ". verdict ");
    bench.key(&mut section, "enter");
    bench.key(&mut section, "esc");
    assert!(
        cursor_row(&bench, &section).contains("analysis \u{b7} graph, no kind"),
        "{}",
        bench.render_section(&section, 100)
    );
}

/// MOD-141 D13, D2: the editor's write is answered with the scope's catalogue; the section adopts
/// it quietly (the tree behind the editor moves) and the editor takes the outcome.
#[tokio::test]
async fn a_graph_replaced_reply_updates_the_tree_and_the_editor() {
    let backend = demo();
    let (bench, mut section, _) = bench_with_demo().await;
    cursor_to(&bench, &mut section, ". verdict ");
    bench.key(&mut section, "enter");
    answer_lookups(&bench, &mut section).await;
    bench.key(&mut section, "enter");
    for _ in 0.."verdict".len() {
        bench.key(&mut section, "backspace");
    }
    type_at(&bench, &mut section, "decide");
    bench.key(&mut section, "enter");
    let asked = store_requests(&bench);
    let [write @ StoreRequest::ReplaceGraph { .. }] = asked.as_slice() else {
        panic!("the step form's save is one `replace_graph`: {asked:?}");
    };

    let reply = serve(&backend, write).await;
    assert!(
        matches!(
            &reply,
            StoreReply::GraphReplaced {
                outcome: CasOutcome::Applied(_),
                ..
            }
        ),
        "{reply:?}"
    );
    bench.reply(&mut section, &reply);

    assert!(store_requests(&bench).is_empty(), "nothing more is sent");
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("decide"),
        "the editor shows the stored body: {frame}"
    );
    assert!(
        section.absorbs_failure("replace_graph"),
        "the editor's notice says a refused write"
    );

    bench.key(&mut section, "esc");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(". decide "),
        "the tree is the reply's: {frame}"
    );
    assert!(
        error_text(&bench, &section, 100).is_empty(),
        "an applied write leaves no notice: {frame}"
    );
    assert!(!section.absorbs_failure("replace_graph"));
}

/// MOD-141 D2: a scope change drops the editor with the tree it was opened from (`R-NF-3`'s
/// rule for every editor: its token belongs to the other workspace).
#[tokio::test]
async fn a_scope_change_closes_the_editor() {
    let (bench, mut section, _) = bench_with_demo().await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "enter");
    assert_eq!(section.key_stack(), Some(views::GRAPH_EDITOR));
    let other = Scope {
        workspace_id: WorkspaceId::new(),
        project_ids: vec![ProjectId::new()],
    };

    section.on_scope_change(&other);

    assert!(!section.captures_input());
    assert_eq!(section.key_stack(), Some(views::KINDS_BROWSE));
    let wanted = section.wants_requests(&other);
    assert!(
        matches!(wanted.as_slice(), [StoreRequest::Catalogue(scope)] if *scope == other),
        "{wanted:?}"
    );
}

/// MOD-141 D2: the hint row is the editor's own while it is open, and Browse's names the key that
/// opens it.
#[tokio::test]
async fn the_hint_row_is_the_editors_while_it_is_open() {
    let (bench, mut section, snapshot) = bench_with_demo().await;
    let browse = bench.key_hint(&section);
    assert!(browse.contains("Enter graph editor"), "{browse}");
    assert!(browse.contains("n kind "), "{browse}");

    bench.key(&mut section, "j");
    bench.key(&mut section, "enter");

    let entry = snapshot.projects[0]
        .graph(ids::GRAPH_VULKAN_ANA)
        .expect("the analysis graph")
        .clone();
    let (editor, _) = GraphEditor::open(entry, snapshot.personas.clone(), &bench_scope().await);
    assert_eq!(
        bench.key_hint(&section),
        editor.hint_text(htui::keys::Keys::compiled())
    );
}

/// MOD-141 D11, C5: the Settings tab hands the mouse to its active section's gesture. Kinds wants it
/// only while the graph editor is open: a press on the canvas is the editor's, a press off it is
/// not, and closing the editor gives the mouse back.
#[tokio::test]
async fn the_settings_tab_hands_the_mouse_to_the_open_editor_only() {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use htui::keys::KeyChord;
    use htui::ui::tabs::Tab;

    let press = |column, row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let key = |chord: &str| {
        KeyChord::parse(chord)
            .unwrap_or_else(|| panic!("`{chord}`"))
            .to_event()
    };
    let draw = |tab: &SettingsTab, ctx: &htui::app::Ctx<'_>| {
        let area = Rect::new(0, 0, 100, 30);
        let mut terminal = Terminal::with_options(
            TestBackend::new(100, 30),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .expect("a test terminal");
        terminal
            .draw(|frame| tab.render(frame, frame.area(), ctx))
            .expect("the tab draws");
    };
    let bench = SectionBench::new().await;
    let mut ctx = bench.ctx();
    let mut tab = SettingsTab::with_sections(vec![Box::new(KindsSection::new())]);
    tab.on_reply(
        &StoreReply::Catalogue(Box::new(demo_catalogue(&demo()).await)),
        &mut ctx,
    );
    draw(&tab, &ctx);
    assert!(!tab.wants_mouse(), "Browse has no gesture of its own");
    assert_eq!(tab.on_mouse(press(50, 15), &mut ctx), Handled::Pass);

    tab.on_key(key("j"), &mut ctx);
    tab.on_key(key("enter"), &mut ctx);
    assert!(
        tab.wants_mouse(),
        "the open editor's canvas takes the mouse"
    );
    draw(&tab, &ctx);
    assert_eq!(
        tab.on_mouse(press(50, 15), &mut ctx),
        Handled::Consumed,
        "a press on the canvas is the editor's"
    );
    assert_eq!(
        tab.on_mouse(press(0, 0), &mut ctx),
        Handled::Pass,
        "a press on the pane's border is not"
    );
    tab.on_mouse_lost();

    tab.on_key(key("esc"), &mut ctx);
    assert!(!tab.wants_mouse(), "closed, the mouse goes back");
}
