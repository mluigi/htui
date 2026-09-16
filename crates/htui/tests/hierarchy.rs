//! `Settings > Hierarchy`, from the worker side out (MOD-15 milestone 3, D14).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, which is how
//! `try_serve`'s reads are already exercised without a runtime: one request in, one reply out, no
//! channels and no shell. The section half lives below the divider and goes through `Harness`.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::hierarchy::REQUEST_NAMES;
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::{ProjectId, ProjectPatch, RepoId, RepoPatch, WorkspaceId, WorkspacePatch};
use htui_core::store::{DeleteTarget, MemStore};
use htui_store::Backend;

/// The demo world behind a memory backend: a `Writer::Memory`, a `this_user` and a `this_box`.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
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
