//! `Settings > Kinds`, from the worker side out (MOD-15 milestone 4, D18).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, exactly as
//! `tests/hierarchy.rs` does: one request in, one reply out, no channels and no shell. The section
//! half is milestone 4's task 2 and lands below this one.
#![cfg(feature = "testkit")]

use chrono::{Duration, Utc};
use htui::app::{Action, Handled};
use htui::catalogue::{self, CatalogueSnapshot, GraphEntry, REQUEST_NAMES};
use htui::hierarchy::MirrorAfterDelete;
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{
    AgentsSection, HierarchySection, KindsSection, SettingsSection, SettingsTab,
};
use htui_core::fixtures::ids;
use htui_core::model::{
    ItemKind, ItemKindId, ItemKindPatch, NewProject, PhaseId, PhasePatch, ProjectId, Scope,
    StepGraph, StepGraphId, StepGraphPatch, StepGraphPhase, WorkspaceId,
};
use htui_core::seed::{PHASES_PER_PROJECT, PhaseSeed, phase_row};
use htui_core::store::{
    MemStore, ReadStore, StoreError, WriteStore, item_kind_is_held, reserved_phase_name,
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

/// One read of the demo scope.
async fn demo_catalogue(backend: &Backend) -> CatalogueSnapshot {
    catalogue(serve(backend, &StoreRequest::Catalogue(vulkan_scope())).await)
}

/// The empty scope every name-only request is built against: the nine names are a property of the
/// enum, not of a store.
fn nil_scope() -> Scope {
    Scope {
        workspace_id: WorkspaceId::default(),
        project_ids: Vec::new(),
    }
}

/// One of each of the nine, in [`REQUEST_NAMES`] order. The ids are nil where the request never
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
    ]
}

/// `REQUEST_NAMES` and [`StoreRequest::name`] are one list read from two places (D5): a tenth
/// request cannot be named in one and matched in the other.
#[test]
fn catalogue_names_are_stable() {
    let requests = catalogue_requests();
    assert_eq!(requests.len(), REQUEST_NAMES.len());
    assert_eq!(REQUEST_NAMES.len(), 9);
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
    let phases: usize = entry.graphs.iter().map(|g| g.phases.len()).sum();
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
    let names: Vec<&str> = analysis.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["research", "verdict"], "phases in position order");
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

/// Offline, every one of the nine is refused by its own name with MOD-25's one sentence:
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

/// `catalogue::serve` is reachable only through `try_serve`'s nine or-ed patterns, so a request
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
    assert!(entry.phases.is_empty(), "a graph is created without phases");
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
fn phases(snapshot: &CatalogueSnapshot, graph: StepGraphId) -> &[StepGraphPhase] {
    &snapshot.projects[0]
        .graph(graph)
        .unwrap_or_else(|| panic!("the catalogue still names {graph}"))
        .phases
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
        frame.contains("MOD-4 owns these"),
        "the detail line names its owner: {frame}"
    );
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
            project_id: ids::PROJECT_VULKAN,
            name: "orphan".to_owned(),
            description: String::new(),
            created_at: now,
            updated_at: now,
        },
        phases: Vec::new(),
    });
    bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)));
    let _ = bench.drained();

    let frame = bench.render_section(&section, 100);
    let last = frame
        .lines()
        .filter(|line| !line.trim().is_empty())
        .nth_back(1)
        .expect("the tree has rows above the hint");
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

/// While an editor is open every printable key is text, `l` included, so the tab's own section
/// cycle is off for as long as something is being typed (M3 D2).
#[tokio::test]
async fn l_is_a_letter_while_editing() {
    let (bench, mut section, _) = bench_with_demo().await;

    assert_eq!(bench.key(&mut section, "n"), Handled::Consumed);
    assert!(section.captures_input(), "an open editor takes every key");

    assert_eq!(bench.key(&mut section, "l"), Handled::Consumed);
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("prefix     : l"),
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
        error_text(&bench, &section, 100).contains(&"`create_kind` is still in flight".to_owned()),
        "a second Enter says why it did nothing"
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
            .contains(&"`graph` names no graph in this project".to_owned()),
        "the refusal says what could not be resolved"
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
    assert!(frame.contains("name       : analysis"), "{frame}");
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
        frame.contains("y write \u{b7} n/Esc back to the editor"),
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
        frame.contains("Tab/Shift+Tab field"),
        "the editor is back: {frame}"
    );
    assert!(frame.contains("prefix     : AN"), "with its text: {frame}");
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
        error_text(&bench, &section, 100).contains(&"`update_kind` is still in flight".to_owned())
    );
}
