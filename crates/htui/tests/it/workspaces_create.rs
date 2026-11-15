//! Creating workspaces and projects on the Workspaces tab (MOD-116 M3 T3, D1-D7, D11, D13),
//! through a `Harness` over the demo world.
//!
//! Every case gives the tab a temp home (`WorkspacesTab::new().with_home(..)`, D3), so a default
//! folder lands under it and never under the real `~/workspaces`, and the demo's two workspaces
//! get folder rows pointing at existing folders under that home (`<home>/graphics`,
//! `<home>/platform`): the `Harness` serves through `store_worker::serve`, whose worker resolves a
//! workspace with no folder row under the real home, so no case creates a project in one. The
//! tab shows paths under `~`, so the frames do not depend on where the temp dir is.
//!
//! The git cases (`skip_without_git!`) clone from a local bare repository, by a path relative to
//! the clone's working directory (`<home>/graphics/projects`), so its frames are stable too; no
//! case reaches the network.
#![cfg(feature = "testkit")]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use chrono::Utc;
use htui::testkit::Harness;
use htui::ui::tabs::WorkspacesTab;
use htui_core::fixtures::{demo_data, ids};
use htui_core::model::WorkspaceBoxPath;
use htui_core::store::{MemStore, WriteStore as _};
use htui_orch::isolate::git::Cli;
use htui_orch::isolate::git::testkit::repo_with_one_commit;
use htui_orch::skip_without_git;
use tempfile::TempDir;

/// A temp home, canonical, so what the worker answers (canonical) is under it as typed.
fn temp_home() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a throwaway home");
    let home = dir.path().canonicalize().expect("the home resolves");
    (dir, home)
}

/// The demo with a folder row for both workspaces at an existing folder under `home`.
async fn demo(home: &Path) -> MemStore {
    let store = MemStore::demo();
    for (workspace, name) in [
        (ids::WORKSPACE_GRAPHICS, "graphics"),
        (ids::WORKSPACE_PLATFORM, "platform"),
    ] {
        let folder = home.join(name);
        std::fs::create_dir_all(&folder).expect("the folder is made");
        store
            .upsert_workspace_box_path(&WorkspaceBoxPath {
                workspace_id: workspace,
                box_id: ids::BOX,
                folder_path: folder.display().to_string(),
                updated_at: Utc::now(),
            })
            .await
            .expect("a folder row is written");
    }
    store
}

/// The demo's user and box, and no workspace or anything in one.
fn no_workspace() -> MemStore {
    let mut data = demo_data();
    data.workspaces.clear();
    data.workspace_projects.clear();
    data.projects.clear();
    data.item_key_counter.clear();
    data.items.clear();
    data.revisions.clear();
    data.links.clear();
    data.notes.clear();
    data.documents.clear();
    data.runs.clear();
    data.steps.clear();
    data.events.clear();
    data.requirement_specs.clear();
    data.requirement_areas.clear();
    data.requirement_key_counter.clear();
    data.requirements.clear();
    data.requirement_revisions.clear();
    data.item_requirements.clear();
    MemStore::from_demo(data)
}

/// The tab over `home`.
fn tab(home: &Path) -> Box<WorkspacesTab> {
    Box::new(WorkspacesTab::new().with_home(Some(home)))
}

/// The demo world on the Workspaces tab alone, 100 x 30, Graphics open.
async fn open_over(store: MemStore, home: &Path) -> Harness {
    let mut harness = Harness::over(store).size(100, 30).with_tab(tab(home));
    harness.settle().await;
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    harness
}

/// One key per character of `text`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        let chord = if c == ' ' {
            "space".to_owned()
        } else {
            c.to_string()
        };
        harness.key(&chord);
    }
}

/// The frame's row that holds `needle`.
fn row_with(frame: &str, needle: &str) -> String {
    frame
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no row holds `{needle}`:\n{frame}"))
        .to_owned()
}

/// The list's selected row (the cursor's `▌` in the list's first column).
fn selected(frame: &str) -> String {
    frame
        .lines()
        .find(|line| line.starts_with("\u{2502}\u{258c}"))
        .unwrap_or_else(|| panic!("no row is selected:\n{frame}"))
        .to_owned()
}

/// `N`, then the chooser's second row (`project in <name>`): the project form.
fn open_project_form(harness: &mut Harness) {
    harness.key("N");
    harness.key("j");
    harness.key("enter");
}

/// From the project form's name: `App` typed, then the source moved `steps` to the right
/// (`existing folder`, `git URL`, `new repo`).
fn name_and_source(harness: &mut Harness, name: &str, steps: usize) {
    type_text(harness, name);
    for _ in 0..3 {
        harness.key("tab");
    }
    for _ in 0..steps {
        harness.key("l");
    }
}

/// `git <args>` in `cwd`, which must succeed.
async fn git_ok(git: &Cli, cwd: &Path, args: &[&str]) {
    let argv: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let exited = git.run("oracle", cwd, &argv, &[]).await.expect("git runs");
    assert!(exited.ok(), "git {args:?} failed: {}", exited.stderr);
}

/// A bare repository `<home>/remote.git` holding one commit on `main`; the URL a clone into
/// `<home>/graphics/projects/<slug>` reaches it by, relative to that clone's working directory.
async fn bare_remote(git: &Cli, home: &Path) -> &'static str {
    let source = home.join("source");
    std::fs::create_dir(&source).expect("the source dir");
    let _ = repo_with_one_commit(&source);
    let remote = home.join("remote.git");
    let remote_text = remote.display().to_string();
    git_ok(
        git,
        home,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            &remote_text,
        ],
    )
    .await;
    git_ok(
        git,
        &source,
        &[
            "push",
            "--quiet",
            "--",
            &remote_text,
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    "../../remote.git"
}

/// D1: `N` offers a workspace, or a project in the selected row's workspace (Graphics, open).
#[tokio::test]
async fn n_offers_a_workspace_or_a_project_in_the_selected_workspace() {
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    harness.key("N");
    let frame = harness.render();
    assert!(frame.contains(" New "), "{frame}");
    assert!(
        row_with(&frame, "workspace ").contains("workspace"),
        "{frame}"
    );
    assert!(frame.contains("project in Graphics"), "{frame}");
    insta::assert_snapshot!("chooser_100", frame);

    // `n` closes it; `j`, `y` choose the project.
    harness.key("n");
    assert!(!harness.render().contains("project in Graphics"));
    harness.key("N");
    harness.key("j");
    harness.key("y");
    let frame = harness.render();
    assert!(frame.contains("New project in Graphics"), "{frame}");
}

/// D1: with no workspace listed `N` opens the workspace form at once; the empty state names it.
#[tokio::test]
async fn n_on_an_empty_store_opens_the_workspace_form() {
    let (_dir, home) = temp_home();
    let mut harness = Harness::empty().size(100, 30).with_tab(tab(&home));
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("`N` creates one"), "{frame}");
    harness.key("N");
    let frame = harness.render();
    assert!(frame.contains("New workspace"), "{frame}");
    assert!(frame.contains("A new workspace starts stopped."), "{frame}");
    insta::assert_snapshot!("workspace_form_empty_100", frame);
}

/// D2, D3: the slug follows the name, the folder follows the slug, both under `~`.
#[tokio::test]
async fn the_workspace_form_previews_the_folder_that_follows_the_name() {
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    harness.key("N");
    harness.key("enter");
    type_text(&mut harness, "Mobile App");
    let frame = harness.render();
    assert!(row_with(&frame, "slug").contains("mobile-app"), "{frame}");
    assert!(
        row_with(&frame, "folder").contains("~/workspaces/mobile-app"),
        "{frame}"
    );
    assert!(
        frame.contains("~/workspaces/mobile-app/") && frame.contains("projects/"),
        "{frame}"
    );
    insta::assert_snapshot!("workspace_form_100", frame);
}

/// D4: the folder is made under the (temp) home, the workspace is listed stopped and selected,
/// and the open workspace stays open.
#[tokio::test]
async fn creating_a_workspace_makes_its_folder_and_lists_it_stopped_without_opening_it() {
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    harness.key("N");
    harness.key("enter");
    type_text(&mut harness, "Mobile App");
    harness.key("enter");
    harness.settle().await;

    assert!(home.join("workspaces").join("mobile-app").is_dir());
    let frame = harness.render();
    assert!(
        !frame.contains("New workspace"),
        "the form closed:\n{frame}"
    );
    assert!(
        row_with(&frame, "\u{25be} Mobile App").contains("\u{25cb} stopped"),
        "{frame}"
    );
    assert!(selected(&frame).contains("Mobile App"), "{frame}");
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    assert_eq!(harness.app().scope.workspace_id, ids::WORKSPACE_GRAPHICS);
    assert_eq!(harness.app().status, None);
}

/// D4's exception: with no workspace open the new one is opened, so the shell has one. The store
/// holds the demo's user and box and no workspace: `Harness::empty()` has no `app_user`, which
/// the worker refuses a create without.
#[tokio::test]
async fn the_first_workspace_is_opened() {
    let (_dir, home) = temp_home();
    let mut harness = Harness::over(no_workspace())
        .size(100, 30)
        .with_tab(tab(&home));
    harness.settle().await;
    harness.key("N");
    type_text(&mut harness, "Mobile App");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert_eq!(harness.app().top_bar.workspace, "Mobile App", "{frame}");
    assert!(
        row_with(&frame, "\u{25be} Mobile App").contains("\u{25cf} open"),
        "{frame}"
    );
    assert!(home.join("workspaces").join("mobile-app").is_dir());
}

/// D13: a taken slug keeps the form, its text, and the store's message under it; nothing on the
/// status line, and the folder this request made is gone again.
#[tokio::test]
async fn a_taken_slug_keeps_the_form_and_says_so() {
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    harness.key("N");
    harness.key("enter");
    type_text(&mut harness, "Graphics");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("New workspace"), "the form stays:\n{frame}");
    assert!(row_with(&frame, "name").contains("Graphics"), "{frame}");
    assert!(frame.contains("graphics"), "{frame}");
    assert_eq!(harness.app().status, None);
    assert!(!home.join("workspaces").join("graphics").exists());
    let message = frame
        .lines()
        .skip_while(|line| !line.contains("description"))
        .nth(2)
        .unwrap_or_default()
        .to_owned();
    assert!(
        message
            .trim_matches(|c: char| c == '\u{2502}' || c.is_whitespace())
            .len()
            > 3,
        "a message under the form:\n{frame}"
    );
}

/// `Esc` closes the form and nothing is written.
#[tokio::test]
async fn esc_closes_the_form_and_writes_nothing() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    harness.key("N");
    harness.key("enter");
    type_text(&mut harness, "Mobile App");
    harness.key("esc");
    harness.settle().await;
    let frame = harness.render();
    assert!(!frame.contains("New workspace"), "{frame}");
    assert!(!frame.contains("Mobile App"), "{frame}");
    assert_eq!(store.workspaces().await.expect("listed").len(), 2);
    assert!(!home.join("workspaces").exists());
}

/// D6, D11: a new repo in Graphics: listed under it, selected, and the open scope gains it.
#[tokio::test]
async fn a_new_repo_project_is_created_and_selected() {
    let Some(_git) = skip_without_git!() else {
        return;
    };
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    open_project_form(&mut harness);
    // `h` from `existing folder` wraps to `new repo`.
    name_and_source(&mut harness, "Mobile App", 0);
    harness.key("h");
    let frame = harness.render();
    assert!(row_with(&frame, "source").contains("new repo"), "{frame}");
    assert!(
        frame.contains("creates ~/graphics/projects/mobile-app with git"),
        "{frame}"
    );
    insta::assert_snapshot!("project_form_new_repo_100", frame);

    harness.key("enter");
    harness.settle().await;
    let dest = home.join("graphics").join("projects").join("mobile-app");
    assert!(dest.join(".git").is_dir(), "the repository is made");
    let frame = harness.render();
    assert!(
        !frame.contains("New project in"),
        "the form closed:\n{frame}"
    );
    assert!(selected(&frame).contains("Mobile App"), "{frame}");
    let listed = harness.app().scope.project_ids.len();
    assert_eq!(listed, 2, "the open scope gained the project");
    assert_eq!(harness.app().status, None);
}

/// D7: while the clone is out the form says what it does; once served, the project is listed.
#[tokio::test]
async fn a_git_url_project_shows_its_progress_then_lands() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let (_dir, home) = temp_home();
    let url = bare_remote(&git, &home).await;
    let mut harness = open_over(demo(&home).await, &home).await;
    open_project_form(&mut harness);
    name_and_source(&mut harness, "Shaders", 1);
    harness.key("tab");
    type_text(&mut harness, url);
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains("cloning ../../remote.git into") && frame.contains("shaders\u{2026}"),
        "{frame}"
    );
    insta::assert_snapshot!("project_progress_100", frame);

    harness.settle().await;
    let frame = harness.render();
    assert!(!frame.contains("New project in"), "{frame}");
    assert!(selected(&frame).contains("Shaders"), "{frame}");
    assert!(
        home.join("graphics")
            .join("projects")
            .join("shaders")
            .join("f")
            .is_file()
    );
}

/// D6, D13: a folder that is not a repository is refused, naming MOD-145; the form stays.
#[tokio::test]
async fn a_plain_folder_is_refused_naming_mod_145() {
    let (_dir, home) = temp_home();
    std::fs::create_dir(home.join("plain")).expect("a plain folder");
    let mut harness = open_over(demo(&home).await, &home).await;
    open_project_form(&mut harness);
    name_and_source(&mut harness, "Plain", 0);
    harness.key("tab");
    type_text(&mut harness, "~/plain");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("New project in Graphics"), "{frame}");
    assert!(frame.contains("MOD-145"), "{frame}");
    assert!(frame.contains("~/plain"), "{frame}");
    assert_eq!(harness.app().status, None);
    insta::assert_snapshot!("project_refused_100", frame);
}

/// D7, D13: `Esc` while the clone is out closes the form and says so; its outcome is told.
#[tokio::test]
async fn esc_while_cloning_closes_the_form_and_the_outcome_is_told() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let (_dir, home) = temp_home();
    let url = bare_remote(&git, &home).await;
    let mut harness = open_over(demo(&home).await, &home).await;
    open_project_form(&mut harness);
    name_and_source(&mut harness, "Shaders", 1);
    harness.key("tab");
    type_text(&mut harness, url);
    harness.key("enter");
    harness.key("esc");
    let frame = harness.render();
    assert!(!frame.contains("New project in"), "{frame}");
    assert!(frame.contains("still being created"), "{frame}");

    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("Project `shaders` is ready in Graphics"),
        "{frame}"
    );
    assert!(frame.contains("~/graphics/projects/shaders"), "{frame}");
    assert_eq!(harness.app().status, None);
}

/// D7: one project out at a time: a second `N` → project is told.
#[tokio::test]
async fn a_second_project_while_one_is_out_is_refused() {
    let Some(_git) = skip_without_git!() else {
        return;
    };
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    open_project_form(&mut harness);
    name_and_source(&mut harness, "One", 2);
    harness.key("enter");
    harness.key("esc");
    // The told box closes; a second project is refused while the first is out.
    harness.key("esc");
    open_project_form(&mut harness);
    let frame = harness.render();
    assert!(
        frame.contains("a project is still being created"),
        "{frame}"
    );
    assert!(!frame.contains("New project in"), "{frame}");

    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("Project `one` is ready in Graphics"),
        "{frame}"
    );
}

/// The form captures text: digits and `q` are letters, not tab switches or quit.
#[tokio::test]
async fn typing_digits_and_q_in_a_form_does_not_switch_tabs_or_quit() {
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    harness.key("N");
    harness.key("enter");
    type_text(&mut harness, "q1 2");
    assert!(!harness.app().should_quit);
    assert_eq!(harness.app().tabs.active_id(), Some(WorkspacesTab::ID));
    let frame = harness.render();
    assert!(row_with(&frame, "name").contains("q1 2"), "{frame}");
    assert!(row_with(&frame, "slug").contains("q1-2"), "{frame}");
}
