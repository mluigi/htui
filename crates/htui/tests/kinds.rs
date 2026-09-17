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
    StepGraphId, StepGraphPatch, WorkspaceId,
};
use htui_core::seed::PHASES_PER_PROJECT;
use htui_core::store::{MemStore, ReadStore, StoreError, WriteStore};
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
