//! Editing and deleting workspaces and setting their default target on the Workspaces tab
//! (MOD-116 M3 T4, D5, D9, D10, D12), through a `Harness` over the demo world.
//!
//! As in `workspaces_create`, every case gives the tab a temp home (`WorkspacesTab::new()
//! .with_home(..)`, D3) and the demo's two workspaces folder rows at existing folders under it
//! (`<home>/graphics`, `<home>/platform`), so no folder lands under the real `~/workspaces` and
//! the frames, which show paths under `~`, do not depend on where the temp dir is. A workspace a
//! case deletes is made in the store first (`Scratch`): the demo's Graphics holds a stopped run
//! and Platform live ones, and the store refuses deleting either.
#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use chrono::Utc;
use htui::testkit::Harness;
use htui::ui::tabs::WorkspacesTab;
use htui_core::fixtures::ids;
use htui_core::model::{
    NewTarget, NewWorkspace, TargetId, TargetKind, WorkspaceBoxPath, WorkspaceId, WorkspacePatch,
};
use htui_core::store::{CasOutcome, MemStore, ReadStore as _, WriteStore as _};
use tempfile::TempDir;

/// A temp home, canonical, so what the worker answers (canonical) is under it as typed.
fn temp_home() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a throwaway home");
    let home = dir.path().canonicalize().expect("the home resolves");
    (dir, home)
}

/// A folder row for `workspace` at `<home>/<name>`, made on disk.
async fn folder_row(store: &MemStore, home: &Path, workspace: WorkspaceId, name: &str) {
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

/// The demo with a folder row for both workspaces at an existing folder under `home`.
async fn demo(home: &Path) -> MemStore {
    let store = MemStore::demo();
    folder_row(&store, home, ids::WORKSPACE_GRAPHICS, "graphics").await;
    folder_row(&store, home, ids::WORKSPACE_PLATFORM, "platform").await;
    store
}

/// [`demo`] plus a third workspace, `Scratch` (no project, no run), its folder `<home>/scratch`.
async fn with_scratch(home: &Path) -> (MemStore, WorkspaceId) {
    let store = demo(home).await;
    let scratch = store
        .create_workspace(NewWorkspace {
            id: WorkspaceId::new(),
            slug: "scratch".to_owned(),
            name: "Scratch".to_owned(),
            description: String::new(),
            created_by: ids::USER,
        })
        .await
        .expect("a workspace is created");
    folder_row(&store, home, scratch.id, "scratch").await;
    (store, scratch.id)
}

/// The demo world on the Workspaces tab alone, 100 x 30, Graphics open.
async fn open_over(store: MemStore, home: &Path) -> Harness {
    let mut harness = Harness::over(store)
        .size(100, 30)
        .with_tab(Box::new(WorkspacesTab::new().with_home(Some(home))));
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

/// `n` backspaces.
fn erase(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.key("backspace");
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

/// The detail pane's words, one space between each (a long value wraps there).
fn detail_text(frame: &str) -> String {
    let words: Vec<&str> = frame
        .lines()
        .filter_map(|line| {
            line.split_once("\u{2502}\u{2502}")
                .map(|(_, detail)| detail)
        })
        .flat_map(|detail| detail.trim_end_matches('\u{2502}').split_whitespace())
        .collect();
    words.join(" ")
}

/// The list's selected row (the cursor's `▌` in the list's first column).
fn selected(frame: &str) -> String {
    frame
        .lines()
        .find(|line| line.starts_with("\u{2502}\u{258c}"))
        .unwrap_or_else(|| panic!("no row is selected:\n{frame}"))
        .to_owned()
}

/// `j` until the selected row holds `needle`, then the overview it asks for is read.
async fn select(harness: &mut Harness, needle: &str) {
    for _ in 0..12 {
        if selected(&harness.render()).contains(needle) {
            harness.settle().await;
            return;
        }
        harness.key("j");
    }
    panic!("no row holds `{needle}`:\n{}", harness.render());
}

/// The workspace `id` as the store holds it.
async fn workspace(store: &MemStore, id: WorkspaceId) -> htui_core::model::Workspace {
    store
        .workspace(id)
        .await
        .expect("a memory read")
        .expect("the workspace is there")
}

/// The target `workspace`'s scope row names, and the row's token.
async fn scope_target(
    store: &MemStore,
    workspace: WorkspaceId,
) -> (Option<TargetId>, chrono::DateTime<Utc>) {
    let id = store
        .scope_project(workspace)
        .await
        .expect("a memory read")
        .expect("a scope row");
    let row = store
        .project(id)
        .await
        .expect("a memory read")
        .expect("the scope row");
    (row.target_id, row.updated_at)
}

/// D5: `e` on Platform opens its form prefilled; a rename is saved, listed, and the header reads
/// it once Platform is opened.
#[tokio::test]
async fn e_opens_the_workspace_prefilled_and_saves_a_rename() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    select(&mut harness, "Platform").await;
    harness.key("e");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("Edit Platform"), "{frame}");
    assert!(row_with(&frame, "name").contains("Platform"), "{frame}");
    assert!(row_with(&frame, "slug").contains("platform"), "{frame}");
    assert!(row_with(&frame, "folder").contains("~/platform"), "{frame}");
    assert!(
        detail_text(&frame).contains("the old folder and what it holds stay where they are"),
        "{frame}"
    );
    insta::assert_snapshot!("edit_form_100", frame);

    type_text(&mut harness, " Core");
    // The slug does not follow the name on an edit.
    let frame = harness.render();
    assert!(
        !row_with(&frame, "slug").contains("platform-core"),
        "{frame}"
    );
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !frame.contains("Edit Platform"),
        "the form closed:\n{frame}"
    );
    assert!(row_with(&frame, "\u{25be} Platform Core").contains("Platform Core"));
    let row = workspace(&store, ids::WORKSPACE_PLATFORM).await;
    assert_eq!(
        (row.name.as_str(), row.slug.as_str()),
        ("Platform Core", "platform")
    );
    assert_eq!(harness.app().status, None);

    harness.key("o");
    harness.settle().await;
    assert_eq!(harness.app().top_bar.workspace, "Platform Core");
}

/// D11: a rename of the open workspace reaches the header with no `o`.
#[tokio::test]
async fn renaming_the_open_workspace_moves_the_header() {
    let (_dir, home) = temp_home();
    let mut harness = open_over(demo(&home).await, &home).await;
    harness.key("e");
    harness.settle().await;
    type_text(&mut harness, " Lab");
    harness.key("enter");
    harness.settle().await;
    assert_eq!(harness.app().top_bar.workspace, "Graphics Lab");
    assert_eq!(harness.app().scope.workspace_id, ids::WORKSPACE_GRAPHICS);
    let frame = harness.render();
    assert!(frame.contains("Graphics Lab"), "{frame}");
}

/// D5: a new folder is made and recorded; the old one and what it holds stay where they are.
#[tokio::test]
async fn editing_the_folder_points_at_it_and_moves_nothing() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    std::fs::write(home.join("graphics").join("keep.txt"), "kept").expect("a file to keep");
    let mut harness = open_over(store, &home).await;
    harness.key("e");
    harness.settle().await;
    harness.key("tab");
    harness.key("tab");
    erase(&mut harness, "~/graphics".len());
    type_text(&mut harness, "~/elsewhere");
    harness.key("enter");
    harness.settle().await;

    assert!(home.join("elsewhere").is_dir(), "the new folder is made");
    assert!(
        home.join("graphics").join("keep.txt").is_file(),
        "the old folder keeps what it holds"
    );
    assert!(!home.join("elsewhere").join("keep.txt").exists());
    let frame = harness.render();
    assert!(
        !frame.contains("Edit Graphics"),
        "the form closed:\n{frame}"
    );
    assert!(
        row_with(&frame, "folder on this box").contains("~/elsewhere"),
        "{frame}"
    );
}

/// D5: the row changed between `e` and `Enter`: the form keeps its text and says so, and a
/// second `Enter` saves over it.
#[tokio::test]
async fn an_edit_changed_elsewhere_keeps_the_text_and_retries() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    harness.key("e");
    harness.settle().await;
    type_text(&mut harness, " Lab");

    let before = workspace(&store, ids::WORKSPACE_GRAPHICS).await;
    let moved = store
        .update_workspace(
            ids::WORKSPACE_GRAPHICS,
            before.updated_at,
            WorkspacePatch {
                description: Some("edited elsewhere".to_owned()),
                ..WorkspacePatch::default()
            },
        )
        .await
        .expect("a memory write");
    assert!(matches!(moved, CasOutcome::Applied(_)), "{moved:?}");

    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("Edit Graphics"), "the form stays:\n{frame}");
    assert!(frame.contains("changed elsewhere"), "{frame}");
    assert!(row_with(&frame, "name").contains("Graphics Lab"), "{frame}");
    assert_eq!(harness.app().status, None);
    assert_eq!(
        workspace(&store, ids::WORKSPACE_GRAPHICS).await.name,
        "Graphics"
    );

    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !frame.contains("Edit Graphics"),
        "the form closed:\n{frame}"
    );
    assert_eq!(
        workspace(&store, ids::WORKSPACE_GRAPHICS).await.name,
        "Graphics Lab"
    );
}

/// D10: `d` counts, warns with the store's counts, asks for the slug, then deletes; the folder
/// stays on disk.
#[tokio::test]
async fn d_counts_warns_and_deletes_after_the_typed_slug() {
    let (_dir, home) = temp_home();
    let (store, scratch) = with_scratch(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    select(&mut harness, "Scratch").await;
    harness.key("d");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("This deletes workspace `scratch`"),
        "{frame}"
    );
    assert!(frame.contains("Nothing here can be undone."), "{frame}");
    assert!(
        frame.contains("Its folder `~/scratch` stays on disk."),
        "{frame}"
    );
    insta::assert_snapshot!("delete_warn_100", frame);

    harness.key("y");
    let frame = harness.render();
    assert!(frame.contains("Type `scratch` to confirm:"), "{frame}");
    insta::assert_snapshot!("delete_typed_100", frame);

    type_text(&mut harness, "scratch");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(!frame.contains("Scratch"), "no longer listed:\n{frame}");
    assert!(!frame.contains("This deletes"), "the box closed:\n{frame}");
    assert!(home.join("scratch").is_dir(), "its folder stays on disk");
    assert_eq!(store.workspace(scratch).await.expect("a memory read"), None);
    assert_eq!(harness.app().status, None);
    assert_eq!(harness.app().scope.workspace_id, ids::WORKSPACE_GRAPHICS);
}

/// D10: a wrong slug clears the field and says so; nothing is deleted.
#[tokio::test]
async fn a_wrong_slug_deletes_nothing() {
    let (_dir, home) = temp_home();
    let (store, scratch) = with_scratch(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    select(&mut harness, "Scratch").await;
    harness.key("d");
    harness.settle().await;
    harness.key("y");
    type_text(&mut harness, "nope");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("that is not the slug; nothing was deleted"),
        "{frame}"
    );
    assert!(
        !row_with(&frame, "Type `scratch`").contains("nope"),
        "{frame}"
    );
    harness.key("esc");
    harness.settle().await;
    let frame = harness.render();
    assert!(!frame.contains("This deletes"), "{frame}");
    assert!(store.workspace(scratch).await.expect("a read").is_some());
}

/// D10: the store refuses deleting a workspace whose runs would lose their resume (the demo's
/// Graphics holds a stopped run; its member projects' live runs alone do not refuse, the scope
/// row's would): its sentence in the box, back at the warning, nothing deleted and nothing on the
/// status line.
#[tokio::test]
async fn a_workspace_with_live_runs_is_refused_in_the_box() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    harness.key("d");
    harness.settle().await;
    harness.key("y");
    type_text(&mut harness, "graphics");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("This deletes workspace `graphics`"),
        "{frame}"
    );
    assert!(frame.contains("stopped run"), "{frame}");
    assert!(
        !frame.contains("Type `graphics`"),
        "back at the warning:\n{frame}"
    );
    assert_eq!(harness.app().status, None);
    assert!(
        store
            .workspace(ids::WORKSPACE_GRAPHICS)
            .await
            .expect("a read")
            .is_some()
    );
    harness.key("esc");
    assert!(!harness.render().contains("This deletes"));
}

/// D10: deleting the open workspace opens the first one left.
#[tokio::test]
async fn deleting_the_open_workspace_opens_the_first_left() {
    let (_dir, home) = temp_home();
    let (store, scratch) = with_scratch(&home).await;
    let mut harness = open_over(store, &home).await;
    select(&mut harness, "Scratch").await;
    harness.key("o");
    harness.settle().await;
    assert_eq!(harness.app().scope.workspace_id, scratch);
    harness.key("d");
    harness.settle().await;
    harness.key("y");
    type_text(&mut harness, "scratch");
    harness.key("enter");
    harness.settle().await;
    assert_eq!(harness.app().scope.workspace_id, ids::WORKSPACE_GRAPHICS);
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    let frame = harness.render();
    assert!(
        row_with(&frame, "\u{25be} Graphics").contains("\u{25cf} open"),
        "{frame}"
    );
}

/// D12: `e` and `d` on a project row say where projects are edited.
#[tokio::test]
async fn e_and_d_on_a_project_row_say_where_projects_are_edited() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    harness.key("j");
    harness.key("j");
    harness.settle().await;
    let frame = harness.render();
    assert!(!selected(&frame).contains("overview"), "{frame}");
    for key in ["e", "d"] {
        harness.key(key);
        harness.settle().await;
        let frame = harness.render();
        assert!(
            frame.contains("projects are edited in Settings \u{203a} Hierarchy until MOD-123"),
            "{key}:\n{frame}"
        );
        harness.key("esc");
        assert!(!harness.render().contains("until MOD-123"));
    }
    assert_eq!(store.workspaces().await.expect("listed").len(), 2);
}

/// D9: `T` lists the Settings default and the live targets; choosing one sets the workspace's,
/// and `inherit` clears it again.
#[tokio::test]
async fn t_lists_the_targets_and_sets_the_workspace_default() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    let frame = harness.render();
    assert!(
        detail_text(&frame).contains("default target inherits the Settings default (this box)"),
        "{frame}"
    );
    harness.key("T");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("Default target of Graphics"), "{frame}");
    assert!(
        row_with(&frame, "inherit the Settings default (this box)").contains("\u{b7} current"),
        "{frame}"
    );
    assert!(row_with(&frame, "this box  local").len() > 1, "{frame}");
    insta::assert_snapshot!("target_picker_100", frame);

    harness.key("j");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !frame.contains("Default target of"),
        "the picker closed:\n{frame}"
    );
    let line = row_with(&frame, "default target");
    assert!(
        line.contains("this box") && !line.contains("inherits"),
        "{frame}"
    );
    let (target, _) = scope_target(&store, ids::WORKSPACE_GRAPHICS).await;
    assert_eq!(target, Some(htui_core::seed::LOCAL_TARGET_ID));

    harness.key("T");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        row_with(&frame, "this box  local").contains("\u{b7} current"),
        "{frame}"
    );
    harness.key("k");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        detail_text(&frame).contains("default target inherits the Settings default (this box)"),
        "{frame}"
    );
    assert_eq!(scope_target(&store, ids::WORKSPACE_GRAPHICS).await.0, None);
    assert_eq!(harness.app().status, None);
}

/// D9: a kind this build does not run is listed dim, and choosing it is told, not written.
#[tokio::test]
async fn a_planned_target_cannot_be_chosen() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    store
        .create_target(NewTarget {
            id: TargetId::new(),
            name: "builder".to_owned(),
            kind: TargetKind::Docker,
            locator: serde_json::json!({}),
        })
        .await
        .expect("a docker target");
    let mut harness = open_over(store.clone(), &home).await;
    harness.key("T");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        row_with(&frame, "builder  docker").contains("planned \u{b7} MOD-44"),
        "{frame}"
    );
    harness.key("j");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("builder runs once MOD-44 lands"), "{frame}");
    assert_eq!(scope_target(&store, ids::WORKSPACE_GRAPHICS).await.0, None);
    // The told box closes onto the picker, which still offers the rest.
    harness.key("esc");
    let frame = harness.render();
    assert!(frame.contains("Default target of Graphics"), "{frame}");
}

/// D9: the scope row changed while the picker was up: nothing is written, and it is said.
#[tokio::test]
async fn a_target_changed_elsewhere_is_refused_and_said() {
    let (_dir, home) = temp_home();
    let store = demo(&home).await;
    let mut harness = open_over(store.clone(), &home).await;
    harness.key("T");
    harness.settle().await;

    let scope = store
        .scope_project(ids::WORKSPACE_GRAPHICS)
        .await
        .expect("a read")
        .expect("a scope row");
    let (_, token) = scope_target(&store, ids::WORKSPACE_GRAPHICS).await;
    let moved = store
        .set_project_target(scope, token, Some(htui_core::seed::LOCAL_TARGET_ID))
        .await
        .expect("a memory write");
    assert!(matches!(moved, CasOutcome::Applied(_)), "{moved:?}");
    let moved = store
        .set_project_target(scope, workspace_token(&store, scope).await, None)
        .await
        .expect("a memory write");
    assert!(matches!(moved, CasOutcome::Applied(_)), "{moved:?}");

    harness.key("j");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("the default target of Graphics changed meanwhile; press T again"),
        "{frame}"
    );
    assert_eq!(scope_target(&store, ids::WORKSPACE_GRAPHICS).await.0, None);
    assert_eq!(harness.app().status, None);
}

/// The scope row `scope`'s token now.
async fn workspace_token(
    store: &MemStore,
    scope: htui_core::model::ProjectId,
) -> chrono::DateTime<Utc> {
    store
        .project(scope)
        .await
        .expect("a read")
        .expect("the scope row")
        .updated_at
}
