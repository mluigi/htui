//! `Settings > Hierarchy`, from the worker side out (MOD-15 milestone 3, D14).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, which is how
//! `try_serve`'s reads are already exercised without a runtime: one request in, one reply out, no
//! channels and no shell. The section half lives below the divider and goes through `Harness`.
#![cfg(feature = "testkit")]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::Utc;
use htui::app::Action;
use htui::hierarchy::{
    HierarchySnapshot, InferOutcome, InferReport, MirrorAfterDelete, REQUEST_NAMES, RepoInference,
};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{AgentsSection, HierarchySection, SettingsSection, SettingsTab};
use htui_core::fixtures::ids;
use htui_core::model::{
    NewProject, ProjectId, ProjectPatch, RepoBoxPath, RepoId, RepoPatch, WorkspaceBoxPath,
    WorkspaceId, WorkspacePatch,
};
use htui_core::store::{DeleteReach, DeleteTarget, MemStore, ReadStore, WriteStore};
use htui_orch::infer::{MAX_DIRS, MatchedBy};
use htui_orch::isolate::git::testkit::repo_with_one_commit;
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{Terminal, TerminalOptions, Viewport};

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

/// `StoreRequest::name` and `hierarchy::REQUEST_NAMES` are the same thirteen strings in the same
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
        StoreRequest::InferRepoPaths(WorkspaceId::default()),
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

/// A canonical path that is not UTF-8 is refused rather than stored mangled: `to_string_lossy`
/// would put a string naming nothing on disk into `root_path`. The typed path is valid UTF-8 — it
/// arrived as a `String` — so only what a link resolves to can get here, and the refusal names what
/// was typed and never the target (`R-BOX-4`).
#[cfg(unix)]
#[tokio::test]
async fn a_canonical_path_that_is_not_utf8_is_refused() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let dir = tempfile::tempdir().expect("a throwaway directory");
    let target = dir.path().join(OsStr::from_bytes(b"not\xffutf8"));
    fs::create_dir(&target).expect("a directory whose name is not UTF-8");
    let link = dir.path().join("checkout");
    std::os::unix::fs::symlink(&target, &link).expect("the link is created");

    let (request, message) = refusal(
        serve(
            &demo(),
            &StoreRequest::SetWorkspaceRoot {
                id: ids::WORKSPACE_GRAPHICS,
                path: link.display().to_string(),
            },
        )
        .await,
    );
    assert_eq!(request, "set_workspace_root");
    assert!(
        message.contains("is not valid UTF-8"),
        "the refusal says what is wrong with it: {message}"
    );
    assert!(
        message.contains("checkout"),
        "and names the path as typed: {message}"
    );
    assert!(
        !message.contains("utf8"),
        "a refusal never names a link's target: {message}"
    );
}

/// A box with no row is refused **before** the path is stat'ed: the box refusal is the one that
/// tells the user what is actually wrong, and a `spawn_blocking` on a cold mount is not worth
/// paying to reach a refusal that was already decided.
#[tokio::test]
async fn a_box_with_no_row_is_refused_before_the_path_is_read() {
    let backend = Backend::memory(MemStore::new());
    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::SetWorkspaceRoot {
                id: WorkspaceId::default(),
                path: "/definitely/not/here".to_owned(),
            },
        )
        .await,
    );
    assert_eq!(request, "set_workspace_root");
    assert!(
        message.contains("box `(this box)` not found"),
        "the box is refused, not the path: {message}"
    );
}

/// The workspace a write's reply re-reads is resolved **before** the write: a project that is
/// linked to nothing has no tree to answer with, and a read that failed after the fact would report
/// `Failed` for a write that applied.
#[tokio::test]
async fn an_unlinked_project_is_refused_before_anything_is_written() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let orphan = store
        .create_project(NewProject {
            id: ProjectId::new(),
            slug: "orphan".to_owned(),
            name: "Orphan".to_owned(),
            description: String::new(),
            created_by: ids::USER,
        })
        .await
        .expect("the store creates it");

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::UpdateProject {
                id: orphan.id,
                expected: orphan.updated_at,
                patch: ProjectPatch {
                    name: Some("Renamed".to_owned()),
                    ..ProjectPatch::default()
                },
            },
        )
        .await,
    );
    assert_eq!(request, "update_project");
    assert!(message.contains("workspace_project"), "{message}");
    assert_eq!(
        store
            .project(orphan.id)
            .await
            .expect("the store answers")
            .expect("the project is there")
            .name,
        "Orphan",
        "the rename did not apply behind a refusal that says it did not"
    );

    let (request, _) = refusal(
        serve(
            &backend,
            &StoreRequest::CreateRepo {
                project: orphan.id,
                name: "alpha".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            },
        )
        .await,
    );
    assert_eq!(request, "create_repo");
    assert!(
        store
            .repos(orphan.id)
            .await
            .expect("the store answers")
            .is_empty(),
        "and no repo was left behind either"
    );
}

/// A link's position is one past the highest, not the count: after a middle project is deleted the
/// count collides with a position that is still in use, and `workspace_project` has no unique on it.
#[tokio::test]
async fn a_link_takes_one_past_the_highest_position() {
    let backend = demo();
    let second = tree(
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
    assert_eq!(second.projects[1].link.position, 1);

    // The first one goes, leaving a hole at `0` and `renderer` at `1`.
    let _ = serve(&backend, &StoreRequest::DeleteProject(ids::PROJECT_VULKAN)).await;
    let third = tree(
        serve(
            &backend,
            &StoreRequest::CreateProject {
                workspace: ids::WORKSPACE_GRAPHICS,
                slug: "shaders".to_owned(),
                name: "Shaders".to_owned(),
                description: String::new(),
            },
        )
        .await,
    );

    let positions: Vec<(&str, i32)> = third
        .projects
        .iter()
        .map(|entry| (entry.project.slug.as_str(), entry.link.position))
        .collect();
    assert_eq!(
        positions,
        vec![("renderer", 1), ("shaders", 2)],
        "the new link is one past the highest, so no two links share a position"
    );
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

/// Offline, every one of the thirteen is refused by its own name with the one sentence MOD-25
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

/// One of each of the thirteen, in `REQUEST_NAMES` order. The ids are nil where the request never
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
        StoreRequest::InferRepoPaths(WorkspaceId::default()),
    ]
}

// ---- inference (MOD-7 milestone 4, T4; plan D114, D116, D124) ----
//
// Every checkout is a real repository built by `repo_with_one_commit` with `gix` alone, and its
// remote is a `[remote "origin"]` section appended to `.git/config` by hand (D131): no `git` binary.

/// The remote every `core` in this file is registered with.
const CORE_REMOTE: &str = "https://example.com/o/core.git";

/// The same repository as [`CORE_REMOTE`], spelled the way a clone over SSH records it.
const CORE_CLONE_REMOTE: &str = "git@example.com:o/core";

/// A repo named `name` in `vulkan-tutorials`, created through the worker; its id.
async fn create_repo(backend: &Backend, name: &str, remote: Option<&str>) -> RepoId {
    let created = tree(
        serve(
            backend,
            &StoreRequest::CreateRepo {
                project: ids::PROJECT_VULKAN,
                name: name.to_owned(),
                remote_url: remote.map(str::to_owned),
                default_branch: "main".to_owned(),
                is_primary: false,
            },
        )
        .await,
    );
    created.projects[0]
        .repos
        .iter()
        .find(|entry| entry.repo.name == name)
        .expect("the new repo is in the tree")
        .repo
        .id
}

/// A real repository at `root/rel`, with `remote` as its `origin` when given.
fn checkout(root: &Path, rel: &str, remote: Option<&str>) -> PathBuf {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).expect("the checkout's directory is created");
    repo_with_one_commit(&dir);
    if let Some(url) = remote {
        let mut config = fs::OpenOptions::new()
            .append(true)
            .open(dir.join(".git").join("config"))
            .expect("the repository's config opens");
        write!(
            config,
            "[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
        )
        .expect("the remote is written");
    }
    dir
}

/// `Graphics`' root on this box, through the worker (and so through its guard).
async fn set_root(backend: &Backend, dir: &Path) -> HierarchySnapshot {
    tree(
        serve(
            backend,
            &StoreRequest::SetWorkspaceRoot {
                id: ids::WORKSPACE_GRAPHICS,
                path: dir.display().to_string(),
            },
        )
        .await,
    )
}

/// The tree and the report an inference answers, or a panic naming what came back instead.
#[track_caller]
fn inferred(reply: StoreReply) -> (HierarchySnapshot, InferReport) {
    match reply {
        StoreReply::RepoPathsInferred { tree, report } => (*tree, report),
        other => panic!("expected an inference: {other:?}"),
    }
}

/// One `InferRepoPaths` over `Graphics`.
async fn infer(backend: &Backend) -> (HierarchySnapshot, InferReport) {
    inferred(
        serve(
            backend,
            &StoreRequest::InferRepoPaths(ids::WORKSPACE_GRAPHICS),
        )
        .await,
    )
}

/// What the report says about the repo named `name`.
#[track_caller]
fn outcome<'a>(report: &'a InferReport, name: &str) -> &'a InferOutcome {
    &report
        .repos
        .iter()
        .find(|line| line.name == name)
        .unwrap_or_else(|| panic!("`{name}` is in the report: {report:?}"))
        .outcome
}

/// This box's row for the repo named `name`, as the tree shows it.
#[track_caller]
fn local_path(tree: &HierarchySnapshot, name: &str) -> Option<RepoBoxPath> {
    tree.projects
        .iter()
        .flat_map(|entry| entry.repos.iter())
        .find(|entry| entry.repo.name == name)
        .unwrap_or_else(|| panic!("`{name}` is in the tree"))
        .local_path
        .clone()
}

/// `path`, canonical, as the worker stores it.
fn canonical(path: &Path) -> String {
    path.canonicalize()
        .expect("the path resolves")
        .into_os_string()
        .into_string()
        .expect("a tempdir path is UTF-8")
}

/// The first failing test of T4: a remote match under the root is stored canonical, under this
/// box, and the report names the root it walked.
#[tokio::test]
async fn inference_writes_one_canonical_row_by_remote() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    let core = checkout(root.path(), "src/core", Some(CORE_CLONE_REMOTE));
    create_repo(&backend, "core", Some(CORE_REMOTE)).await;
    set_root(&backend, root.path()).await;

    let (tree, report) = infer(&backend).await;

    assert_eq!(report.root, Some(canonical(root.path())));
    assert!(!report.truncated);
    assert_eq!(
        outcome(&report, "core"),
        &InferOutcome::Inferred {
            path: canonical(&core),
            by: MatchedBy::Remote,
        }
    );
    let row = local_path(&tree, "core").expect("the reply's tree carries the new row");
    assert_eq!(row.local_path, canonical(&core));
    assert_eq!(row.box_id, ids::BOX, "the worker fills in the box");
}

/// A manual row is never replaced (PRD D5): the repo is `AlreadySet`, and the tree still shows the
/// path that was typed, not the checkout the walk would have found.
#[tokio::test]
async fn inference_leaves_a_manual_row_and_reports_it_already_set() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    checkout(root.path(), "docs", None);
    let elsewhere = root.path().join("elsewhere");
    fs::create_dir(&elsewhere).expect("the manual path exists");
    let docs = create_repo(&backend, "docs", None).await;
    set_root(&backend, root.path()).await;
    let _ = tree(
        serve(
            &backend,
            &StoreRequest::SetRepoPath {
                project: ids::PROJECT_VULKAN,
                repo: docs,
                path: elsewhere.display().to_string(),
            },
        )
        .await,
    );

    let (tree, report) = infer(&backend).await;

    assert_eq!(outcome(&report, "docs"), &InferOutcome::AlreadySet);
    assert_eq!(
        local_path(&tree, "docs").map(|row| row.local_path),
        Some(canonical(&elsewhere)),
        "the manual row is untouched"
    );
}

/// Two clones of one remote are two candidates, and ambiguity writes nothing (PRD D5).
#[tokio::test]
async fn two_clones_are_ambiguous_and_write_nothing() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    checkout(root.path(), "one/core", Some(CORE_CLONE_REMOTE));
    checkout(root.path(), "two/core", Some(CORE_REMOTE));
    create_repo(&backend, "core", Some(CORE_REMOTE)).await;
    set_root(&backend, root.path()).await;

    let (tree, report) = infer(&backend).await;

    assert_eq!(
        outcome(&report, "core"),
        &InferOutcome::Ambiguous { candidates: 2 }
    );
    assert_eq!(local_path(&tree, "core"), None, "nothing was written");
}

/// A root with nothing under it matches nothing.
#[tokio::test]
async fn no_checkout_reports_no_match() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    create_repo(&backend, "web", Some("https://example.com/o/web.git")).await;
    set_root(&backend, root.path()).await;

    let (tree, report) = infer(&backend).await;

    assert_eq!(outcome(&report, "web"), &InferOutcome::NoMatch);
    assert_eq!(local_path(&tree, "web"), None);
}

/// A repo with no remote is matched on its name, the second rung (plan D112).
#[tokio::test]
async fn a_name_match_is_inferred_for_a_repo_without_a_remote() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    let tools = checkout(root.path(), "tools", None);
    create_repo(&backend, "tools", None).await;
    set_root(&backend, root.path()).await;

    let (tree, report) = infer(&backend).await;

    assert_eq!(
        outcome(&report, "tools"),
        &InferOutcome::Inferred {
            path: canonical(&tools),
            by: MatchedBy::Name,
        }
    );
    assert_eq!(
        local_path(&tree, "tools").map(|row| row.local_path),
        Some(canonical(&tools))
    );
}

/// A root set through a link is stored as its target (F-102), and every path the walk finds is
/// under that target: a link's name never reaches a row.
#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_root_yields_paths_under_its_target() {
    let backend = demo();
    let dir = tempfile::tempdir().expect("a throwaway directory");
    let real = dir.path().join("real");
    fs::create_dir(&real).expect("the real root");
    checkout(&real, "core", Some(CORE_CLONE_REMOTE));
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("the link is created");
    create_repo(&backend, "core", Some(CORE_REMOTE)).await;

    let stored = set_root(&backend, &link).await;
    assert_eq!(
        stored.root_path.map(|row| row.root_path),
        Some(canonical(&real)),
        "the root is stored as the link's target"
    );

    let (_, report) = infer(&backend).await;
    let InferOutcome::Inferred { path, .. } = outcome(&report, "core") else {
        panic!("the checkout under the target is found: {report:?}");
    };
    assert!(
        path.starts_with(&canonical(&real)),
        "the path is under the target: {path}"
    );
    assert!(
        !Path::new(path)
            .components()
            .any(|part| part.as_os_str() == "link"),
        "the path never walks the link: {path}"
    );
}

/// A legacy row stored as a link (before `SetRepoPath` canonicalised, F-102) still holds the
/// checkout it points at: the walk yields the target, so the held set has to know the row by its
/// target too, or a second repo with the same remote is inferred onto a checkout already owned.
#[cfg(unix)]
#[tokio::test]
async fn a_legacy_link_row_holds_the_checkout_it_points_at() {
    let backend = demo();
    let dir = tempfile::tempdir().expect("a throwaway directory");
    let root = dir.path().join("root");
    fs::create_dir(&root).expect("the root");
    let core = checkout(&root, "core", Some(CORE_CLONE_REMOTE));
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&core, &link).expect("the link is created");
    let owner = create_repo(&backend, "core", Some(CORE_REMOTE)).await;
    create_repo(&backend, "fork", Some(CORE_REMOTE)).await;
    set_root(&backend, &root).await;
    // Straight to the store, past the worker's guard: the row as an older build stored it.
    backend
        .writer()
        .expect("a memory backend writes")
        .upsert_repo_box_path(&RepoBoxPath {
            repo_id: owner,
            box_id: ids::BOX,
            local_path: link.display().to_string(),
            updated_at: Utc::now(),
        })
        .await
        .expect("the legacy row is written");

    let (tree, report) = infer(&backend).await;

    assert_eq!(outcome(&report, "core"), &InferOutcome::AlreadySet);
    assert_eq!(
        outcome(&report, "fork"),
        &InferOutcome::NoMatch,
        "the only checkout is held by `core`'s row"
    );
    assert_eq!(local_path(&tree, "fork"), None, "nothing was written");
}

/// No root on this box: nothing is walked and nothing is reported.
#[tokio::test]
async fn no_root_reports_none_and_walks_nothing() {
    let backend = demo();
    create_repo(&backend, "core", Some(CORE_REMOTE)).await;

    let (_, report) = infer(&backend).await;

    assert_eq!(
        report,
        InferReport {
            root: None,
            truncated: false,
            repos: vec![],
        }
    );
}

/// Inference is idempotent: the second pass finds the row the first wrote and changes nothing.
#[tokio::test]
async fn a_second_pass_changes_nothing() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    checkout(root.path(), "src/core", Some(CORE_CLONE_REMOTE));
    create_repo(&backend, "core", Some(CORE_REMOTE)).await;
    set_root(&backend, root.path()).await;

    let (first, _) = infer(&backend).await;
    let (second, report) = infer(&backend).await;

    assert_eq!(outcome(&report, "core"), &InferOutcome::AlreadySet);
    assert_eq!(second, first, "the tree is the one the first pass answered");
}

/// A scan cut short at `MAX_DIRS` infers nothing, not even the match it did see: an unseen second
/// clone would turn it into a wrong single match (plan D113).
#[tokio::test]
async fn a_scan_that_hits_the_cap_infers_nothing() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    checkout(root.path(), "core", Some(CORE_CLONE_REMOTE));
    for n in 0..MAX_DIRS {
        fs::create_dir(root.path().join(format!("d{n:05}"))).expect("an empty sibling");
    }
    create_repo(&backend, "core", Some(CORE_REMOTE)).await;
    set_root(&backend, root.path()).await;

    let (tree, report) = infer(&backend).await;

    assert!(report.truncated, "the walk stopped at the cap");
    assert_eq!(outcome(&report, "core"), &InferOutcome::ScanTruncated);
    assert_eq!(local_path(&tree, "core"), None, "nothing was written");
}

/// D134, R-55: the pass is sequential and its held set grows with each write, so of two
/// same-named repos without a remote only the first in tree order takes the one checkout.
#[tokio::test]
async fn a_path_this_pass_writes_is_held_for_the_next_repo() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    let tools = checkout(root.path(), "tools", None);
    let first = create_repo(&backend, "tools", None).await;
    let created = tree(
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
    let renderer = created.projects[1].project.id;
    let _ = tree(
        serve(
            &backend,
            &StoreRequest::CreateRepo {
                project: renderer,
                name: "tools".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: false,
            },
        )
        .await,
    );
    set_root(&backend, root.path()).await;

    let (tree, report) = infer(&backend).await;

    let outcomes: Vec<(RepoId, &InferOutcome)> = report
        .repos
        .iter()
        .map(|line| (line.repo, &line.outcome))
        .collect();
    assert_eq!(outcomes.len(), 2, "both repos are reported: {report:?}");
    assert_eq!(outcomes[0].0, first, "the report is in tree order");
    assert_eq!(
        outcomes[0].1,
        &InferOutcome::Inferred {
            path: canonical(&tools),
            by: MatchedBy::Name,
        }
    );
    assert_eq!(
        outcomes[1].1,
        &InferOutcome::NoMatch,
        "the checkout the first repo took is held"
    );
    let rows: Vec<String> = tree
        .projects
        .iter()
        .flat_map(|entry| entry.repos.iter())
        .filter_map(|entry| entry.local_path.as_ref())
        .map(|row| row.local_path.clone())
        .collect();
    assert_eq!(rows, vec![canonical(&tools)], "exactly one row carries it");
}

/// D134: a checkout another repo already owns on this box is never a candidate, even for a repo
/// whose name matches it.
#[tokio::test]
async fn a_checkout_held_by_another_repo_is_not_a_candidate() {
    let backend = demo();
    let root = tempfile::tempdir().expect("a throwaway workspace root");
    let tools = checkout(root.path(), "tools", None);
    let owner = create_repo(&backend, "tools2", None).await;
    let _ = tree(
        serve(
            &backend,
            &StoreRequest::SetRepoPath {
                project: ids::PROJECT_VULKAN,
                repo: owner,
                path: canonical(&tools),
            },
        )
        .await,
    );
    create_repo(&backend, "tools", None).await;
    set_root(&backend, root.path()).await;

    let (tree, report) = infer(&backend).await;

    assert_eq!(outcome(&report, "tools2"), &InferOutcome::AlreadySet);
    assert_eq!(outcome(&report, "tools"), &InferOutcome::NoMatch);
    assert_eq!(local_path(&tree, "tools"), None, "no row for `tools`");
}

// -------------------------------------------------------------------------------------------
// ---- section (T3) ----
//
// The same tree from the other end: a `Harness` for the frames a user sees and a `SectionBench`
// for the keys and replies a frame cannot show (a request that was emitted, a scope that moved).
// -------------------------------------------------------------------------------------------

/// A settled Settings tab over `store`, both sections registered and the strip already cycled onto
/// `Hierarchy` — the product's own registration order (D4), so `l` is what reaches this section.
async fn hierarchy_over(store: MemStore) -> Harness {
    let mut harness = Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
    ])));
    harness.settle().await;
    harness.key("l");
    harness.settle().await;
    harness
}

/// The demo tree as the section draws it: the workspace line with its root unset, its one project
/// and no repos.
#[tokio::test]
async fn the_demo_tree_renders() {
    let mut harness = hierarchy_over(MemStore::demo()).await;
    insta::assert_snapshot!("demo", harness.render());
}

/// Browse is not a mode: `captures_input` is false there, so the global table still owns `q`
/// (the risk row "a capturing section swallows `q` forever").
#[tokio::test]
async fn q_quits_from_browse() {
    let mut harness = hierarchy_over(MemStore::demo()).await;
    harness.key("q");
    harness.settle().await;
    assert!(
        harness.app().should_quit,
        "Browse binds no `q`, so the global binding takes it"
    );
}

/// Types one key per char, as a user would: the field is the only thing that sees them.
fn type_into(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(&c.to_string());
    }
}

/// The same, one section wide.
fn type_at(bench: &SectionBench, section: &mut dyn SettingsSection, text: &str) {
    for c in text.chars() {
        bench.key(section, &c.to_string());
    }
}

/// One section drawn into a `width`x30 buffer.
///
/// [`SectionBench::render_section`] answers text, and the two things below are *styles*: a warning
/// that is not in `theme.error` reads as a row of the tree, and a snapshot records symbols only.
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

/// The demo tree behind a `SectionBench`, as the worker assembles it.
async fn demo_tree(backend: &Backend, workspace: WorkspaceId) -> HierarchySnapshot {
    tree(serve(backend, &StoreRequest::Hierarchy(workspace)).await)
}

/// `n` on a project row opens the repo editor, and from there `l` is a letter: the tab's own
/// section cycle is off for as long as something is being typed (D2).
#[tokio::test]
async fn n_on_a_project_opens_the_repo_editor() {
    let mut harness = hierarchy_over(MemStore::demo()).await;
    harness.key("j");
    harness.key("n");
    harness.settle().await;
    type_into(&mut harness, "l");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        !frame.contains("transport"),
        "`l` typed into the name field instead of cycling to Agents: {frame}"
    );
    insta::assert_snapshot!("editor_repo", frame);
}

/// A CAS miss keeps the editor and its text, moves the token to the row as it is now, and waits for
/// a second `Enter` (D7, PRD D8): retyping is the cost the PRD said not to pay.
#[tokio::test]
async fn a_stale_reply_keeps_the_typed_text() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(opened.clone()))),
    );
    let _ = bench.drained();

    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "-2");

    // Someone else wrote to the row this editor opened on.
    let current = tree(
        serve(
            &backend,
            &StoreRequest::UpdateWorkspace {
                id: ids::WORKSPACE_GRAPHICS,
                expected: opened.workspace.updated_at,
                patch: WorkspacePatch {
                    name: Some("Graphics, renamed".to_owned()),
                    ..WorkspacePatch::default()
                },
            },
        )
        .await,
    );
    bench.reply(
        &mut section,
        &StoreReply::HierarchyStale(Box::new(current.clone())),
    );
    insta::assert_snapshot!("stale", bench.render_section(&section, 100));
    let flagged = error_text(&bench, &section, 100);
    assert_eq!(
        flagged.len(),
        1,
        "the CAS miss is the one line in `theme.error`: {flagged:?}"
    );
    assert!(
        flagged[0].starts_with("changed elsewhere since you opened it"),
        "{flagged:?}"
    );

    bench.key(&mut section, "enter");
    let emitted = bench.drained();
    let [
        Action::Store(StoreRequest::UpdateWorkspace {
            expected, patch, ..
        }),
    ] = emitted.as_slice()
    else {
        panic!("`Enter` retries by hand, once: {emitted:?}");
    };
    assert_eq!(
        *expected, current.workspace.updated_at,
        "the retry carries the reloaded row's token, not the one the editor opened on"
    );
    assert_eq!(
        patch.slug.as_deref(),
        Some("graphics-2"),
        "and the text that was typed survived the reload"
    );
}

/// The nil startup scope: a pane that names the key that fixes it rather than an empty box. The
/// shell after its **last** workspace was deleted is the other half, and it is
/// [`deleting_the_last_workspace_leaves_the_pane_empty`] — a scope that never had a tree and a
/// scope whose tree was just taken reach this pane by two different routes.
#[tokio::test]
async fn no_workspace_says_so() {
    let mut harness = hierarchy_over(MemStore::new()).await;
    insta::assert_snapshot!("no_workspace", harness.render());
}

/// The last workspace, deleted: the tree it took goes with it, so the pane reads "no workspace"
/// rather than offering `e`/`n`/`b`/`d` over rows whose ids are gone (blueprint §9.6).
#[tokio::test]
async fn deleting_the_last_workspace_leaves_the_pane_empty() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));
    let _ = bench.drained();

    // `d` on the workspace row, through both confirmations.
    bench.key(&mut section, "d");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::DeleteReach(Some(Box::new(DeleteReach {
            workspace_links: 1,
            ..DeleteReach::default()
        }))),
    );
    bench.key(&mut section, "y");
    type_at(&bench, &mut section, "graphics");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Deleted {
            target: DeleteTarget::Workspace(ids::WORKSPACE_GRAPHICS),
            reach: Box::new(DeleteReach {
                workspace_links: 1,
                ..DeleteReach::default()
            }),
            mirror: MirrorAfterDelete::NotNeeded,
        },
    );
    let asked = bench.drained();
    assert!(
        matches!(asked.as_slice(), [Action::Store(StoreRequest::Workspaces)]),
        "the workspace the shell was inside is gone, so the list is what decides where it lands: {asked:?}"
    );

    // Nothing left to enter.
    bench.reply(&mut section, &StoreReply::Workspaces(Vec::new()));
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("no workspace \u{2014} `N` creates one"),
        "an empty list leaves the pane naming the key that fixes it: {frame}"
    );
    assert!(
        frame.contains("deleted `graphics`"),
        "and the notice still reports what was taken: {frame}"
    );
    assert!(
        !frame.contains("vulkan-tutorials"),
        "the deleted tree is not still on screen: {frame}"
    );

    // And the keys that would write against the deleted ids are refused.
    bench.key(&mut section, "e");
    bench.key(&mut section, "d");
    assert!(
        bench.drained().is_empty(),
        "a deleted tree is not a tree to write to"
    );
}

/// `Enter` while the first write is still in flight is refused, as the Browse keys are (D6): the
/// staleness index keeps only the newest request of a kind, so a second send would have the first
/// reply — the one about the write that landed — dropped.
#[tokio::test]
async fn a_second_enter_does_not_resend_the_write() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));
    let _ = bench.drained();

    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "-2");
    bench.key(&mut section, "enter");
    bench.key(&mut section, "enter");

    let emitted = bench.drained();
    let [Action::Store(StoreRequest::UpdateWorkspace { .. })] = emitted.as_slice() else {
        panic!("two `Enter`s are one write: {emitted:?}");
    };
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("`update_workspace` is still in flight"),
        "and the second one says why it did nothing: {frame}"
    );
}

/// `r` then `d`: the read's tree lands while the delete is being counted, and it must not take the
/// confirmation with it — a reply carries no correlation, so `on_tree` closes an editor and never a
/// delete.
#[tokio::test]
async fn a_tree_that_lands_mid_delete_keeps_the_confirmation() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(opened.clone()))),
    );
    let _ = bench.drained();

    bench.key(&mut section, "d");
    let asked = bench.drained();
    assert!(
        matches!(
            asked.as_slice(),
            [Action::Store(StoreRequest::DeleteReach(_))]
        ),
        "`d` counts first: {asked:?}"
    );

    // The read `r` asked for, answered after `d` was pressed.
    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("counting rows"),
        "a read is not the answer to a delete: {frame}"
    );

    bench.reply(
        &mut section,
        &StoreReply::DeleteReach(Some(Box::new(DeleteReach {
            workspace_links: 1,
            ..DeleteReach::default()
        }))),
    );
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("This deletes workspace `graphics`"),
        "and the counts still reach the warning they were asked for: {frame}"
    );
}

/// A `delete_reach` that was refused leaves the pane counting forever unless the stage is left:
/// `Counting` binds nothing but `Esc`, so a refusal puts the tree back by itself.
#[tokio::test]
async fn a_refused_count_leaves_the_confirmation() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));
    let _ = bench.drained();

    bench.key(&mut section, "d");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "delete_reach",
            message: "connection closed".to_owned(),
        },
    );

    let frame = bench.render_section(&section, 100);
    assert!(
        !frame.contains("counting rows"),
        "a refused count does not go on counting: {frame}"
    );
    assert!(
        frame.contains("vulkan-tutorials"),
        "the tree is back, where `d` can be pressed again: {frame}"
    );
}

/// An outage that ended: the read that answers clears "hierarchy needs Postgres", and while it is
/// up the keys it refuses say *that* rather than offering `N` against a store that did not answer.
#[tokio::test]
async fn a_refused_read_is_echoed_and_then_cleared() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "hierarchy",
            message: "connection refused".to_owned(),
        },
    );

    bench.key(&mut section, "N");
    assert!(
        bench.drained().is_empty(),
        "`N` is refused too while the store did not answer"
    );
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("connection refused"),
        "the refusal echoes what the store said, not `N` creates one: {frame}"
    );
    assert!(
        !frame.contains("`N` creates one"),
        "`N` is not on offer here: {frame}"
    );

    // The store answers again, and the workspace is simply not there. (`Esc` clears the notice the
    // refusal left; what is under test is the pane, which is not a notice.)
    bench.reply(&mut section, &StoreReply::Hierarchy(None));
    bench.key(&mut section, "esc");
    let frame = bench.render_section(&section, 100);
    assert!(
        !frame.contains("hierarchy needs Postgres"),
        "a read that answered is not an outage: {frame}"
    );
    assert!(
        frame.contains("no workspace \u{2014} `N` creates one"),
        "{frame}"
    );
}

/// Offline the tree is one refused read: `Backend::writer()` is `None`, so the section says what is
/// missing instead of rendering a workspace that is not there (E-16).
#[tokio::test]
async fn offline_says_it_needs_postgres() {
    // `App::start` issues `ConnectionInfo` since MOD-15 M6, and over a non-`Memory` backend that
    // read reaches `secret::get_dsn` - the developer's own OS keyring without this guard.
    let _keyring = htui_store::testkit::mock_keyring().await;
    // The mirror outlives the harness: dropping the directory deletes it mid-test.
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "hierarchy-section", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let mut harness = Harness::over_backend(Backend::Offline {
        cache,
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
    ])))
    // `offline · 3s` would age between the render and the next tick.
    .with_store_state("offline \u{b7} 0s", None);
    harness.settle().await;
    harness.key("l");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("hierarchy needs Postgres"),
        "a refused read says what is missing: {frame}"
    );
    insta::assert_snapshot!("offline", frame);
}

/// `p` is the only writer of `is_primary` and it only ever sets it (D12): the store demotes the old
/// primary in the same transaction, so no key in this section can leave a project without one.
#[tokio::test]
async fn p_moves_the_primary() {
    let backend = demo();
    for (name, is_primary) in [("alpha", true), ("beta", false)] {
        let reply = serve(
            &backend,
            &StoreRequest::CreateRepo {
                project: ids::PROJECT_VULKAN,
                name: name.to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary,
            },
        )
        .await;
        let _ = tree(reply);
    }
    let with_repos = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    let beta = with_repos.projects[0].repos[1].repo.clone();

    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(with_repos))),
    );
    let _ = bench.drained();

    // Workspace, project, `alpha`, `beta`.
    for _ in 0..3 {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "p");

    let emitted = bench.drained();
    let [
        Action::Store(StoreRequest::UpdateRepo {
            project,
            id,
            expected,
            patch,
        }),
    ] = emitted.as_slice()
    else {
        panic!("`p` is one request: {emitted:?}");
    };
    assert_eq!(*project, ids::PROJECT_VULKAN);
    assert_eq!(*id, beta.id);
    assert_eq!(*expected, beta.updated_at, "the row's own CAS token");
    assert_eq!(patch.is_primary, Some(true));
    assert_eq!(
        (&patch.name, &patch.remote_url, &patch.default_branch),
        (&None, &None, &None),
        "`p` writes one column and the editor never writes this one"
    );
}

/// `b` on the workspace row sends what was typed, untouched: the guard runs on the worker, so the
/// section never stats a path and no test path ever reaches a frame (D8, D14).
#[tokio::test]
async fn b_on_the_workspace_row_sets_the_root() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));
    let _ = bench.drained();

    bench.key(&mut section, "b");
    type_at(&bench, &mut section, "/srv/htui");
    bench.key(&mut section, "enter");

    let emitted = bench.drained();
    let [Action::Store(StoreRequest::SetWorkspaceRoot { id, path })] = emitted.as_slice() else {
        panic!("`b` then `Enter` is one request: {emitted:?}");
    };
    assert_eq!(*id, ids::WORKSPACE_GRAPHICS);
    assert_eq!(path, "/srv/htui", "the path is the user's, verbatim");
}

/// A tree from a workspace the shell is not inside moves the scope (D11): `N` creates a workspace
/// *and enters it*, and the same arm is what makes a deleted project leave the Backlog.
#[tokio::test]
async fn a_reply_from_another_workspace_moves_the_scope() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let elsewhere = demo_tree(&backend, ids::WORKSPACE_PLATFORM).await;

    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(elsewhere))),
    );

    let emitted = bench.drained();
    let [Action::SetScope { workspace }] = emitted.as_slice() else {
        panic!("a tree from elsewhere is one `SetScope`: {emitted:?}");
    };
    assert_eq!(workspace.workspace_id, ids::WORKSPACE_PLATFORM);
    assert_eq!(
        workspace
            .projects
            .iter()
            .map(|project| project.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["htui", "agy"],
        "the summary carries the tree's own projects, in position order"
    );
}

/// PRD D13's two confirmations: the counts before the act, then the slug typed by hand. What the
/// warning listed is what the delete then reports having taken (D9).
#[tokio::test]
async fn d_on_a_project_warns_with_counts_then_asks_for_the_slug() {
    let mut harness = hierarchy_over(MemStore::demo()).await;
    harness.key("j");
    harness.key("d");
    harness.settle().await;
    insta::assert_snapshot!("delete_warn", harness.render());

    harness.key("y");
    harness.settle().await;
    insta::assert_snapshot!("delete_typed", harness.render());

    type_into(&mut harness, "vulkan-tutorials");
    harness.key("enter");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("deleted `vulkan-tutorials`"),
        "the notice reports what was taken: {frame}"
    );
    assert!(
        frame.contains("no mirror"),
        "a memory backend has no mirror to rebuild (D10): {frame}"
    );
    assert!(
        harness.app().projects.is_empty(),
        "the tree that came back moved the scope, so the Backlog loses the project too"
    );
    assert_eq!(
        harness.app().top_bar.workspace,
        "Graphics",
        "the workspace outlived its project"
    );
}

/// The second confirmation is the one that counts: anything but the slug deletes nothing, and the
/// stage stays where it was. While it is up, `q` is a letter (flag B, E-9).
#[tokio::test]
async fn a_wrong_slug_deletes_nothing() {
    let mut harness = hierarchy_over(MemStore::demo()).await;
    harness.key("j");
    harness.key("d");
    harness.settle().await;
    harness.key("y");

    harness.key("q");
    assert!(
        !harness.app().should_quit,
        "a typed confirmation swallows what it does not bind"
    );
    harness.key("enter");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("that is not the slug; nothing was deleted"),
        "{frame}"
    );
    assert_eq!(
        harness.app().projects.len(),
        1,
        "nothing was deleted and the scope did not move"
    );

    harness.key("esc");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !frame.contains("to confirm"),
        "`Esc` leaves the confirmation: {frame}"
    );
    assert!(
        frame.contains("vulkan-tutorials  Vulkan Tutorials"),
        "and the tree is still there: {frame}"
    );
}

// ---- section: inference (MOD-7 milestone 4, T4; plan D114, D117; blueprint D136-D138) ----

/// `tree` with a root on this box at `path`: a synthetic row, since the section never stats one.
fn with_root(mut tree: HierarchySnapshot, path: &str) -> HierarchySnapshot {
    tree.root_path = Some(WorkspaceBoxPath {
        workspace_id: tree.workspace.id,
        box_id: ids::BOX,
        root_path: path.to_owned(),
        updated_at: tree.workspace.updated_at,
    });
    tree
}

/// The `InferRepoPaths` requests among `actions`, by workspace.
fn inferences(actions: &[Action]) -> Vec<WorkspaceId> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Store(StoreRequest::InferRepoPaths(ws)) => Some(*ws),
            _ => None,
        })
        .collect()
}

/// `vulkan-tutorials` with `alpha` (primary) and `beta`, as `p_moves_the_primary` builds it.
async fn tree_with_two_repos(backend: &Backend) -> HierarchySnapshot {
    for (name, is_primary) in [("alpha", true), ("beta", false)] {
        let reply = serve(
            backend,
            &StoreRequest::CreateRepo {
                project: ids::PROJECT_VULKAN,
                name: name.to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary,
            },
        )
        .await;
        let _ = tree(reply);
    }
    demo_tree(backend, ids::WORKSPACE_GRAPHICS).await
}

/// `i` asks the worker to infer the scope's workspace, once (D114).
#[tokio::test]
async fn i_sends_infer_repo_paths_for_the_scope() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));
    let _ = bench.drained();

    bench.key(&mut section, "i");

    let emitted = bench.drained();
    assert!(
        matches!(
            emitted.as_slice(),
            [Action::Store(StoreRequest::InferRepoPaths(ws))] if *ws == ids::WORKSPACE_GRAPHICS
        ),
        "`i` is one request for the scope's workspace: {emitted:?}"
    );
}

/// `i` is a write, so it is refused while another write is in flight, as the other write keys are.
#[tokio::test]
async fn i_is_refused_while_a_write_is_in_flight() {
    let backend = demo();
    let with_repos = tree_with_two_repos(&backend).await;
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(with_repos))),
    );
    let _ = bench.drained();

    for _ in 0..3 {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "p");
    let _ = bench.drained();
    bench.key(&mut section, "i");

    assert!(
        bench.drained().is_empty(),
        "no second request while `p` is in flight"
    );
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("`update_repo` is still in flight"),
        "and the refusal names the write it is waiting for: {frame}"
    );
}

/// A root written through the editor, with a root in the fresh tree, is followed by exactly one
/// inference (D136).
#[tokio::test]
async fn a_root_write_that_applied_is_followed_by_one_inference() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(opened.clone()))),
    );
    let _ = bench.drained();

    bench.key(&mut section, "b");
    type_at(&bench, &mut section, "/srv/htui");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(with_root(opened, "/srv/htui")))),
    );

    let emitted = bench.drained();
    assert!(
        matches!(
            emitted.as_slice(),
            [Action::Store(StoreRequest::InferRepoPaths(ws))] if *ws == ids::WORKSPACE_GRAPHICS
        ),
        "the applied root write is followed by one inference: {emitted:?}"
    );
}

/// A new repo on a workspace with no root here has nothing to walk, so it is not followed (D136).
#[tokio::test]
async fn a_new_repo_on_a_workspace_without_a_root_is_not_followed() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(opened.clone()))),
    );
    let _ = bench.drained();

    bench.key(&mut section, "j");
    bench.key(&mut section, "n");
    type_at(&bench, &mut section, "core");
    bench.key(&mut section, "enter");
    let emitted = bench.drained();
    assert!(
        matches!(
            emitted.as_slice(),
            [Action::Store(StoreRequest::CreateRepo { .. })]
        ),
        "`Enter` creates the repo: {emitted:?}"
    );

    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));

    let emitted = bench.drained();
    assert!(
        inferences(&emitted).is_empty(),
        "no root on this box, no inference: {emitted:?}"
    );
}

/// A new repo on a workspace with a root here is followed by exactly one inference (D136).
#[tokio::test]
async fn a_new_repo_on_a_workspace_with_a_root_is_followed_by_one_inference() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = with_root(
        demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await,
        "/srv/htui",
    );
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(opened.clone()))),
    );
    let _ = bench.drained();

    bench.key(&mut section, "j");
    bench.key(&mut section, "n");
    type_at(&bench, &mut section, "core");
    bench.key(&mut section, "enter");
    let emitted = bench.drained();
    assert!(
        matches!(
            emitted.as_slice(),
            [Action::Store(StoreRequest::CreateRepo { .. })]
        ),
        "`Enter` creates the repo: {emitted:?}"
    );

    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));

    let emitted = bench.drained();
    assert_eq!(
        inferences(&emitted),
        vec![ids::WORKSPACE_GRAPHICS],
        "the applied `create_repo` is followed by one inference: {emitted:?}"
    );
}

/// A repo edited through `e`, with a root here, is followed by exactly one inference (D136).
#[tokio::test]
async fn an_edited_repo_on_a_workspace_with_a_root_is_followed_by_one_inference() {
    let backend = demo();
    let with_repos = with_root(tree_with_two_repos(&backend).await, "/srv/htui");
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(with_repos.clone()))),
    );
    let _ = bench.drained();

    for _ in 0..3 {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "e");
    bench.key(&mut section, "enter");
    let emitted = bench.drained();
    assert!(
        matches!(
            emitted.as_slice(),
            [Action::Store(StoreRequest::UpdateRepo { .. })]
        ),
        "`Enter` updates the repo: {emitted:?}"
    );

    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(with_repos))),
    );

    let emitted = bench.drained();
    assert_eq!(
        inferences(&emitted),
        vec![ids::WORKSPACE_GRAPHICS],
        "the applied `update_repo` is followed by one inference: {emitted:?}"
    );
}

/// `p` changes no column inference reads, so it is never followed, root or not (P-8, D136).
#[tokio::test]
async fn p_is_not_followed_by_an_inference() {
    let backend = demo();
    let with_repos = with_root(tree_with_two_repos(&backend).await, "/srv/htui");
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(with_repos.clone()))),
    );
    let _ = bench.drained();

    for _ in 0..3 {
        bench.key(&mut section, "j");
    }
    bench.key(&mut section, "p");
    let _ = bench.drained();
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(with_repos))),
    );

    let emitted = bench.drained();
    assert!(
        inferences(&emitted).is_empty(),
        "`p` is not followed by an inference: {emitted:?}"
    );
}

/// An inference reply is a tree plus a notice built from its report (D117). Rendered from a
/// synthetic reply with fixed paths, so the frame does not move with a tempdir (H-10, D138).
#[tokio::test]
async fn an_inference_reply_renders_the_tree_and_the_report() {
    let backend = demo();
    let core = create_repo(&backend, "core", Some(CORE_REMOTE)).await;
    let docs = create_repo(&backend, "docs", None).await;
    let mut tree = with_root(
        demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await,
        "/srv/graphics",
    );
    for entry in &mut tree.projects[0].repos {
        if entry.repo.id == core {
            entry.local_path = Some(RepoBoxPath {
                repo_id: core,
                box_id: ids::BOX,
                local_path: "/srv/graphics/core".to_owned(),
                updated_at: entry.repo.updated_at,
            });
        }
    }
    let report = InferReport {
        root: Some("/srv/graphics".to_owned()),
        truncated: false,
        repos: vec![
            RepoInference {
                repo: core,
                name: "core".to_owned(),
                outcome: InferOutcome::Inferred {
                    path: "/srv/graphics/core".to_owned(),
                    by: MatchedBy::Remote,
                },
            },
            RepoInference {
                repo: docs,
                name: "docs".to_owned(),
                outcome: InferOutcome::NoMatch,
            },
        ],
    };

    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    bench.reply(
        &mut section,
        &StoreReply::RepoPathsInferred {
            tree: Box::new(tree),
            report,
        },
    );

    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(
            "inferred 1 of 2 \u{b7} no checkout: docs \u{2014} b on a repo sets it by hand"
        ),
        "the notice reports what was and was not inferred: {frame}"
    );
    insta::assert_snapshot!("inferred", frame);
}

/// The report is prefixed by the follow-up's cause, the `stored as …` of the root write, and never
/// by a refusal shown while the walk ran (R-57): once the reply lands nothing is in flight.
#[tokio::test]
async fn an_inference_report_keeps_its_cause_and_drops_a_refusal_shown_during_the_walk() {
    let bench = SectionBench::new().await;
    let mut section = HierarchySection::new();
    let backend = demo();
    let opened = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    let rooted = with_root(opened.clone(), "/srv/htui");
    let report = || InferReport {
        root: Some("/srv/htui".to_owned()),
        truncated: false,
        repos: vec![],
    };
    bench.reply(&mut section, &StoreReply::Hierarchy(Some(Box::new(opened))));
    let _ = bench.drained();

    // The follow-up: typed with a trailing slash, so the stored root reads differently.
    bench.key(&mut section, "b");
    type_at(&bench, &mut section, "/srv/htui/");
    bench.key(&mut section, "enter");
    bench.reply(
        &mut section,
        &StoreReply::Hierarchy(Some(Box::new(rooted.clone()))),
    );
    assert_eq!(inferences(&bench.drained()), vec![ids::WORKSPACE_GRAPHICS]);
    bench.key(&mut section, "e");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("`infer_repo_paths` is still in flight"),
        "`e` is refused during the walk: {frame}"
    );
    bench.reply(
        &mut section,
        &StoreReply::RepoPathsInferred {
            tree: Box::new(rooted.clone()),
            report: report(),
        },
    );
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("stored as `/srv/htui` \u{b7} no repo in this workspace to infer"),
        "the follow-up's cause stays in front of the report: {frame}"
    );
    assert!(
        !frame.contains("in flight"),
        "nothing is in flight: {frame}"
    );

    // `i` carries no cause, and the refusal it provoked is not one.
    bench.key(&mut section, "i");
    let _ = bench.drained();
    bench.key(&mut section, "b");
    bench.reply(
        &mut section,
        &StoreReply::RepoPathsInferred {
            tree: Box::new(rooted),
            report: report(),
        },
    );
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("no repo in this workspace to infer"),
        "the report is shown: {frame}"
    );
    assert!(
        !frame.contains("in flight") && !frame.contains("stored as"),
        "and nothing in front of it: {frame}"
    );
}

/// One report line for the notice cases.
fn line(name: &str, outcome: InferOutcome) -> RepoInference {
    RepoInference {
        repo: RepoId::new(),
        name: name.to_owned(),
        outcome,
    }
}

/// The notice grammar of D137, byte-exact, for every row of blueprint §6.5: no root, a truncated
/// scan, an empty workspace, and the counted sentence with each name list capped at three.
#[tokio::test]
async fn the_report_notice_names_what_was_not_inferred() {
    let inferred = || InferOutcome::Inferred {
        path: "/srv/graphics/core".to_owned(),
        by: MatchedBy::Remote,
    };
    let cases = [
        (
            InferReport {
                root: None,
                truncated: false,
                repos: vec![],
            },
            "no root on this box for this workspace \u{2014} b on the workspace row sets it"
                .to_owned(),
        ),
        (
            InferReport {
                root: Some("/srv/graphics".to_owned()),
                truncated: true,
                repos: vec![line("core", InferOutcome::ScanTruncated)],
            },
            format!("the scan stopped at {MAX_DIRS} directories; nothing inferred"),
        ),
        (
            InferReport {
                root: Some("/srv/graphics".to_owned()),
                truncated: false,
                repos: vec![],
            },
            "no repo in this workspace to infer".to_owned(),
        ),
        (
            InferReport {
                root: Some("/srv/graphics".to_owned()),
                truncated: false,
                repos: vec![
                    line("core", inferred()),
                    line("api", InferOutcome::AlreadySet),
                    line("docs", InferOutcome::NoMatch),
                    line("web", InferOutcome::Ambiguous { candidates: 2 }),
                ],
            },
            "inferred 1 of 3 \u{b7} 1 already set \u{b7} no checkout: docs \u{b7} ambiguous: web (2) \u{2014} b on a repo sets it by hand"
                .to_owned(),
        ),
        (
            InferReport {
                root: Some("/srv/graphics".to_owned()),
                truncated: false,
                repos: vec![
                    line("core", inferred()),
                    line("a1", InferOutcome::NoMatch),
                    line("a2", InferOutcome::NoMatch),
                    line("a3", InferOutcome::NoMatch),
                    line("a4", InferOutcome::NoMatch),
                    line("a5", InferOutcome::NoMatch),
                    line("gfx", InferOutcome::Refused("refused".to_owned())),
                ],
            },
            "inferred 1 of 7 \u{b7} no checkout: a1, a2, a3 +2 more \u{b7} refused: gfx \u{2014} b on a repo sets it by hand"
                .to_owned(),
        ),
        (
            InferReport {
                root: Some("/srv/graphics".to_owned()),
                truncated: false,
                repos: vec![line("core", inferred()), line("docs", inferred())],
            },
            "inferred 2 of 2".to_owned(),
        ),
    ];

    let backend = demo();
    let tree = demo_tree(&backend, ids::WORKSPACE_GRAPHICS).await;
    for (report, expected) in cases {
        let bench = SectionBench::new().await;
        let mut section = HierarchySection::new();
        bench.reply(
            &mut section,
            &StoreReply::RepoPathsInferred {
                tree: Box::new(tree.clone()),
                report: report.clone(),
            },
        );
        let frame = bench.render_section(&section, 250);
        let hint = frame
            .lines()
            .find(|row| row.contains("j/k"))
            .unwrap_or_else(|| panic!("the hint row is drawn: {frame}"));
        assert!(
            hint.trim_end().ends_with(&format!("\u{b7} {expected}")),
            "{report:?} reads `{expected}`: {hint}"
        );
    }
}
