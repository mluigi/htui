//! `Settings > Hierarchy`, from the worker side out (MOD-15 milestone 3, D14).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, which is how
//! `try_serve`'s reads are already exercised without a runtime: one request in, one reply out, no
//! channels and no shell. The section half lives below the divider and goes through `Harness`.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::app::Action;
use htui::hierarchy::{HierarchySnapshot, MirrorAfterDelete, REQUEST_NAMES};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{AgentsSection, HierarchySection, SettingsSection, SettingsTab};
use htui_core::fixtures::ids;
use htui_core::model::{
    NewProject, ProjectId, ProjectPatch, RepoId, RepoPatch, WorkspaceId, WorkspacePatch,
};
use htui_core::store::{DeleteReach, DeleteTarget, MemStore, ReadStore, WriteStore};
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
    std::fs::create_dir(&target).expect("a directory whose name is not UTF-8");
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
