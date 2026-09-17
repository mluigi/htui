//! `Settings > Kinds`, from the worker side out (MOD-15 milestone 4, D18).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, exactly as
//! `tests/hierarchy.rs` does: one request in, one reply out, no channels and no shell. The section
//! half is milestone 4's task 2 and lands below this one.
#![cfg(feature = "testkit")]

use chrono::{Duration, Utc};
use htui::catalogue::{self, CatalogueSnapshot, REQUEST_NAMES};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::{
    ItemKind, ItemKindId, ItemKindPatch, NewProject, PhaseId, PhasePatch, ProjectId, Scope,
    StepGraphId, StepGraphPatch, StepGraphPhase, WorkspaceId,
};
use htui_core::seed::{PHASES_PER_PROJECT, PhaseSeed, phase_row};
use htui_core::store::{MemStore, ReadStore, StoreError, WriteStore, reserved_phase_name};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};

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
