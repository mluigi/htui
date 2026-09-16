//! `Settings > Hierarchy`, from the worker side out (MOD-15 milestone 3, D14).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, which is how
//! `try_serve`'s reads are already exercised without a runtime: one request in, one reply out, no
//! channels and no shell. The section half lives below the divider and goes through `Harness`.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::hierarchy::{HierarchySnapshot, MirrorAfterDelete, REQUEST_NAMES};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::{ProjectId, ProjectPatch, RepoId, RepoPatch, WorkspaceId, WorkspacePatch};
use htui_core::store::{DeleteTarget, MemStore, ReadStore};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};

/// The demo world behind a memory backend: a `Writer::Memory`, a `this_user` and a `this_box`.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The tree a reply carries, or a panic naming what came back instead.
#[track_caller]
fn tree(reply: StoreReply) -> HierarchySnapshot {
    match reply {
        StoreReply::Hierarchy(Some(tree)) => *tree,
        other => panic!("expected a tree: {other:?}"),
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

/// The tree of one workspace, as the section will render it: the workspace row, this box, the
/// projects in position order, and no root path (the fixture seeds none).
#[tokio::test]
async fn a_hierarchy_read_returns_the_demo_tree() {
    let reply = serve(&demo(), &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await;

    let StoreReply::Hierarchy(Some(tree)) = reply else {
        panic!("a workspace that exists answers a tree: {reply:?}");
    };
    assert_eq!(tree.workspace.slug, "graphics");
    assert_eq!(tree.workspace.id, ids::WORKSPACE_GRAPHICS);
    assert_eq!(
        tree.this_box,
        Some(ids::BOX),
        "identity is resolved worker-side, never carried by the request"
    );
    assert!(
        tree.root_path.is_none(),
        "the fixture seeds no `workspace_box_path` row"
    );

    let projects: Vec<(&str, i32)> = tree
        .projects
        .iter()
        .map(|entry| (entry.project.slug.as_str(), entry.link.position))
        .collect();
    assert_eq!(projects, vec![("vulkan-tutorials", 0)]);
    assert!(
        tree.projects[0].repos.is_empty(),
        "the fixture seeds no repo"
    );
}

/// The startup scope is the nil workspace, and it has to be an answer rather than a failure.
#[tokio::test]
async fn a_hierarchy_read_of_nil_is_none() {
    let reply = serve(&demo(), &StoreRequest::Hierarchy(WorkspaceId::default())).await;
    assert!(
        matches!(reply, StoreReply::Hierarchy(None)),
        "a workspace that does not exist answers `None`, not a failure: {reply:?}"
    );
}

/// `StoreRequest::name` and `hierarchy::REQUEST_NAMES` are the same twelve strings in the same
/// order: the section matches a `Failed` reply by name, and the two lists drifting apart would
/// make a refusal land on no one.
#[test]
fn hierarchy_names_are_stable() {
    let requests = [
        StoreRequest::Hierarchy(WorkspaceId::default()),
        StoreRequest::CreateWorkspace {
            slug: String::new(),
            name: String::new(),
            description: String::new(),
        },
        StoreRequest::UpdateWorkspace {
            id: WorkspaceId::default(),
            expected: Utc::now(),
            patch: WorkspacePatch::default(),
        },
        StoreRequest::SetWorkspaceRoot {
            id: WorkspaceId::default(),
            path: String::new(),
        },
        StoreRequest::CreateProject {
            workspace: WorkspaceId::default(),
            slug: String::new(),
            name: String::new(),
            description: String::new(),
        },
        StoreRequest::UpdateProject {
            id: ProjectId::default(),
            expected: Utc::now(),
            patch: ProjectPatch::default(),
        },
        StoreRequest::CreateRepo {
            project: ProjectId::default(),
            name: String::new(),
            remote_url: None,
            default_branch: String::new(),
            is_primary: false,
        },
        StoreRequest::UpdateRepo {
            project: ProjectId::default(),
            id: RepoId::default(),
            expected: Utc::now(),
            patch: RepoPatch::default(),
        },
        StoreRequest::SetRepoPath {
            project: ProjectId::default(),
            repo: RepoId::default(),
            path: String::new(),
        },
        StoreRequest::DeleteReach(DeleteTarget::Workspace(WorkspaceId::default())),
        StoreRequest::DeleteWorkspace(WorkspaceId::default()),
        StoreRequest::DeleteProject(ProjectId::default()),
    ];

    assert_eq!(requests.len(), REQUEST_NAMES.len());
    for (request, expected) in requests.iter().zip(REQUEST_NAMES) {
        assert_eq!(request.name(), expected);
    }
}

/// `N` creates a workspace **and enters it**: the reply is the new workspace's own tree, which is
/// what the section turns into a `SetScope` (D11). `created_by` is the worker's.
#[tokio::test]
async fn create_workspace_answers_its_own_tree() {
    let backend = demo();
    let tree = tree(
        serve(
            &backend,
            &StoreRequest::CreateWorkspace {
                slug: "ops".to_owned(),
                name: "Ops".to_owned(),
                description: "Operations".to_owned(),
            },
        )
        .await,
    );

    assert_eq!(tree.workspace.slug, "ops");
    assert_eq!(
        tree.workspace.created_by,
        ids::USER,
        "the worker fills in `created_by`; the request never carries a `UserId`"
    );
    assert!(tree.projects.is_empty(), "a new workspace has no projects");
}

/// A project is linked at the end of the workspace, and the catalogue M2 seeds with it is visible
/// through the counts a delete would report.
#[tokio::test]
async fn create_project_links_at_the_end() {
    let backend = demo();
    let tree = tree(
        serve(
            &backend,
            &StoreRequest::CreateProject {
                workspace: ids::WORKSPACE_GRAPHICS,
                slug: "renderer".to_owned(),
                name: "Renderer".to_owned(),
                description: String::new(),
            },
        )
        .await,
    );

    let positions: Vec<(&str, i32)> = tree
        .projects
        .iter()
        .map(|entry| (entry.project.slug.as_str(), entry.link.position))
        .collect();
    assert_eq!(
        positions,
        vec![("vulkan-tutorials", 0), ("renderer", 1)],
        "the new link takes `position = links.len()`"
    );

    let created = tree.projects[1].project.id;
    let reply = serve(
        &backend,
        &StoreRequest::DeleteReach(DeleteTarget::Project(created)),
    )
    .await;
    let StoreReply::DeleteReach(Some(reach)) = reply else {
        panic!("a project that exists has a reach: {reply:?}");
    };
    // M2 D4's catalogue, seeded in `create_project`'s own transaction: five kinds, their five
    // graphs, the fifteen phases and the ten `DEFAULT_TEMPLATES`, plus the link just made.
    assert_eq!(reach.item_kinds, 5);
    assert_eq!(reach.step_graphs, 5);
    assert_eq!(reach.phases, 15);
    assert_eq!(reach.prompt_templates, 10);
    assert_eq!(reach.workspace_links, 1);
}

/// `p` is the only writer of `is_primary` (D12), and the store demotes the old primary in the same
/// transaction, so the invariant holds without this path ever unsetting a flag.
#[tokio::test]
async fn create_repo_then_p_moves_the_primary() {
    let backend = demo();
    for (name, is_primary) in [("alpha", true), ("beta", false)] {
        let reply = serve(
            &backend,
            &StoreRequest::CreateRepo {
                project: ids::PROJECT_VULKAN,
                name: name.to_owned(),
                remote_url: Some(format!("https://git.invalid/{name}.git")),
                default_branch: "main".to_owned(),
                is_primary,
            },
        )
        .await;
        let _ = tree(reply);
    }

    let before = tree(serve(&backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await);
    let repos = &before.projects[0].repos;
    assert_eq!(
        repos
            .iter()
            .map(|entry| entry.repo.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"],
        "repos are ordered by name"
    );
    assert!(repos[0].repo.is_primary && !repos[1].repo.is_primary);

    let beta = &repos[1].repo;
    let after = tree(
        serve(
            &backend,
            &StoreRequest::UpdateRepo {
                project: ids::PROJECT_VULKAN,
                id: beta.id,
                expected: beta.updated_at,
                patch: RepoPatch {
                    is_primary: Some(true),
                    ..RepoPatch::default()
                },
            },
        )
        .await,
    );

    let primaries: Vec<&str> = after.projects[0]
        .repos
        .iter()
        .filter(|entry| entry.repo.is_primary)
        .map(|entry| entry.repo.name.as_str())
        .collect();
    assert_eq!(
        primaries,
        vec!["beta"],
        "the flag moved rather than doubled"
    );
}

/// A CAS miss answers the tree as it is **now** (D7): the editor reloads against it and retries by
/// hand, and nothing is written under the token it edited from.
#[tokio::test]
async fn a_stale_update_answers_the_current_tree() {
    let backend = demo();
    let opened = tree(serve(&backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await);
    let expected = opened.workspace.updated_at;

    let first = serve(
        &backend,
        &StoreRequest::UpdateWorkspace {
            id: ids::WORKSPACE_GRAPHICS,
            expected,
            patch: WorkspacePatch {
                name: Some("Graphics, renamed".to_owned()),
                ..WorkspacePatch::default()
            },
        },
    )
    .await;
    assert_eq!(tree(first).workspace.name, "Graphics, renamed");

    // The same token a second time: someone else got there first, as far as this editor knows.
    let second = serve(
        &backend,
        &StoreRequest::UpdateWorkspace {
            id: ids::WORKSPACE_GRAPHICS,
            expected,
            patch: WorkspacePatch {
                name: Some("Graphics, again".to_owned()),
                ..WorkspacePatch::default()
            },
        },
    )
    .await;
    let StoreReply::HierarchyStale(current) = second else {
        panic!("a stale token answers the current tree: {second:?}");
    };
    assert_eq!(
        current.workspace.name, "Graphics, renamed",
        "the second write did not land"
    );
    assert_ne!(current.workspace.updated_at, expected);
}

/// The guard of D8 through the worker: a real directory is stored canonical, and neither refusal
/// names anything but what was typed. The refusal reaches the caller as `Failed` under the
/// request's own name, with `StoreError::Constraint`'s prefix in front of the sentence.
#[tokio::test]
async fn a_root_is_stored_canonical_and_a_link_is_refused() {
    let backend = demo();
    let dir = tempfile::tempdir().expect("a throwaway directory");
    let canonical = dir.path().canonicalize().expect("the directory resolves");

    let stored = tree(
        serve(
            &backend,
            &StoreRequest::SetWorkspaceRoot {
                id: ids::WORKSPACE_GRAPHICS,
                path: dir.path().display().to_string(),
            },
        )
        .await,
    );
    let row = stored.root_path.expect("this box's row is in the tree");
    assert_eq!(row.root_path, canonical.display().to_string());
    assert_eq!(row.box_id, ids::BOX, "the worker fills in the box");

    let missing = dir.path().join("gone");
    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::SetWorkspaceRoot {
                id: ids::WORKSPACE_GRAPHICS,
                path: missing.display().to_string(),
            },
        )
        .await,
    );
    assert_eq!(request, "set_workspace_root");
    assert!(
        message.contains("does not exist on this box"),
        "the refusal is the guard's sentence: {message}"
    );
    assert!(
        message.contains("constraint violated"),
        "`StoreError::Constraint` renders its own prefix: {message}"
    );

    #[cfg(unix)]
    {
        let link = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("nowhere-at-all"), &link)
            .expect("the link is created");
        let (request, message) = refusal(
            serve(
                &backend,
                &StoreRequest::SetRepoPath {
                    project: ids::PROJECT_VULKAN,
                    repo: RepoId::default(),
                    path: link.display().to_string(),
                },
            )
            .await,
        );
        assert_eq!(request, "set_repo_path");
        assert!(
            message.contains("is a link to nothing"),
            "the guard answers before the store is touched: {message}"
        );
        assert!(
            !message.contains("nowhere-at-all"),
            "a refusal never names the link's target: {message}"
        );
    }
}

/// The report equals the act (PRD D13): what the warning pane showed is what the delete took. On a
/// memory backend there is no mirror to rebuild.
#[tokio::test]
async fn delete_reach_equals_deleted_reach() {
    let backend = demo();
    let reply = serve(
        &backend,
        &StoreRequest::DeleteReach(DeleteTarget::Project(ids::PROJECT_VULKAN)),
    )
    .await;
    let StoreReply::DeleteReach(Some(counted)) = reply else {
        panic!("a project that exists has a reach: {reply:?}");
    };

    let reply = serve(&backend, &StoreRequest::DeleteProject(ids::PROJECT_VULKAN)).await;
    let StoreReply::Deleted {
        target,
        reach,
        mirror,
    } = reply
    else {
        panic!("a delete answers what it took: {reply:?}");
    };
    assert_eq!(target, DeleteTarget::Project(ids::PROJECT_VULKAN));
    assert_eq!(reach, counted, "the numbers shown are the numbers removed");
    assert_eq!(mirror, MirrorAfterDelete::NoMirror);

    let after = tree(serve(&backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await);
    assert!(
        after.projects.is_empty(),
        "the workspace outlived its only project"
    );

    let gone = serve(
        &backend,
        &StoreRequest::DeleteReach(DeleteTarget::Project(ids::PROJECT_VULKAN)),
    )
    .await;
    assert!(
        matches!(gone, StoreReply::DeleteReach(None)),
        "a target that is already gone answers `None`: {gone:?}"
    );
}

/// A workspace delete takes its links and its box paths and **not** its projects (M1 D4), and it
/// needs no rebuild: the two tables it touches are full-table replaced on every refresh pass.
#[tokio::test]
async fn delete_workspace_keeps_its_projects() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());

    let reply = serve(
        &backend,
        &StoreRequest::DeleteWorkspace(ids::WORKSPACE_GRAPHICS),
    )
    .await;
    let StoreReply::Deleted {
        target,
        reach,
        mirror,
    } = reply
    else {
        panic!("a delete answers what it took: {reply:?}");
    };
    assert_eq!(target, DeleteTarget::Workspace(ids::WORKSPACE_GRAPHICS));
    assert_eq!(reach.workspace_links, 1);
    assert_eq!(reach.items, 0, "a workspace delete reaches no item");
    assert_eq!(mirror, MirrorAfterDelete::NotNeeded);

    let after = serve(&backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await;
    assert!(
        matches!(after, StoreReply::Hierarchy(None)),
        "the workspace is gone: {after:?}"
    );
    assert!(
        store
            .project(ids::PROJECT_VULKAN)
            .await
            .expect("the store answers")
            .is_some(),
        "its project survives and stays reachable from other workspaces"
    );
}

/// Offline, every one of the twelve is refused by its own name with the one sentence MOD-25
/// coined: `Backend::writer()` is `None`, so `serve` never reaches the seam.
#[tokio::test]
async fn offline_refuses_every_hierarchy_request_by_name() {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "hierarchy-offline", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let backend = Backend::Offline {
        cache,
        since: Some(Utc::now()),
    };

    for (request, name) in hierarchy_requests().into_iter().zip(REQUEST_NAMES) {
        let (refused, message) = refusal(serve(&backend, &request).await);
        assert_eq!(refused, name);
        assert!(
            message.contains(DATABASE_UNREACHABLE),
            "`{name}` is refused with MOD-25's sentence: {message}"
        );
    }
}

/// One of each of the twelve, in `REQUEST_NAMES` order. The ids are nil where the request never
/// reaches a store.
fn hierarchy_requests() -> Vec<StoreRequest> {
    vec![
        StoreRequest::Hierarchy(WorkspaceId::default()),
        StoreRequest::CreateWorkspace {
            slug: "ops".to_owned(),
            name: "Ops".to_owned(),
            description: String::new(),
        },
        StoreRequest::UpdateWorkspace {
            id: WorkspaceId::default(),
            expected: Utc::now(),
            patch: WorkspacePatch::default(),
        },
        StoreRequest::SetWorkspaceRoot {
            id: WorkspaceId::default(),
            path: "/srv/htui".to_owned(),
        },
        StoreRequest::CreateProject {
            workspace: WorkspaceId::default(),
            slug: "renderer".to_owned(),
            name: "Renderer".to_owned(),
            description: String::new(),
        },
        StoreRequest::UpdateProject {
            id: ProjectId::default(),
            expected: Utc::now(),
            patch: ProjectPatch::default(),
        },
        StoreRequest::CreateRepo {
            project: ProjectId::default(),
            name: "alpha".to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: false,
        },
        StoreRequest::UpdateRepo {
            project: ProjectId::default(),
            id: RepoId::default(),
            expected: Utc::now(),
            patch: RepoPatch::default(),
        },
        StoreRequest::SetRepoPath {
            project: ProjectId::default(),
            repo: RepoId::default(),
            path: "/srv/htui".to_owned(),
        },
        StoreRequest::DeleteReach(DeleteTarget::Workspace(WorkspaceId::default())),
        StoreRequest::DeleteWorkspace(WorkspaceId::default()),
        StoreRequest::DeleteProject(ProjectId::default()),
    ]
}
