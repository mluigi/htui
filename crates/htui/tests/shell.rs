//! Shell-level tests for the workspace switcher overlay (blueprint E, T5).
//!
//! Everything here goes through [`htui::testkit::Harness`] and the public `Overlay` contract: no
//! T3 file is touched and none of T6's registration has to exist yet, because the harness pushes
//! the overlay by hand (blueprint C.8).
#![cfg(feature = "testkit")]

use htui::app::register_all;
use htui::testkit::Harness;
use htui::ui::overlay::workspace_switcher::WorkspaceSwitcher;

/// The demo shell with the switcher open over it, settled and ready to render.
async fn open_over_demo() -> Harness {
    let mut harness = Harness::demo().with_overlay(Box::new(WorkspaceSwitcher::new()));
    register_all(harness.app());
    harness.settle().await;
    harness
}

/// The top bar, i.e. the first line of a rendered frame.
fn top_bar(frame: &str) -> String {
    frame.lines().next().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn the_switcher_lists_every_workspace_with_its_project_count() {
    let mut harness = open_over_demo().await;
    let frame = harness.render();

    assert!(frame.contains("Graphics"), "the first workspace is listed");
    assert!(frame.contains("Platform"), "the second workspace is listed");
    assert!(
        frame.contains("1 project") && frame.contains("2 projects"),
        "each row carries its project count: {frame}"
    );
    insta::assert_snapshot!("switcher_open", frame);
}

#[tokio::test]
async fn the_switcher_opens_on_the_workspace_the_shell_is_already_inside() {
    let mut harness = open_over_demo().await;
    let frame = harness.render();

    assert_eq!(
        harness.app().top_bar.workspace,
        "Graphics",
        "startup entered the first workspace"
    );
    assert!(
        frame.contains("> Graphics") && !frame.contains("> Platform"),
        "the cursor starts on the scope's own workspace: {frame}"
    );
}

#[tokio::test]
async fn j_and_k_move_the_cursor_and_stop_at_the_ends() {
    let mut harness = open_over_demo().await;

    harness.key("j");
    let down = harness.render();
    assert!(
        down.contains("> Platform") && !down.contains("> Graphics"),
        "`j` moves onto the second workspace: {down}"
    );

    // The list does not wrap: `j` at the bottom and `k` at the top are no-ops.
    harness.key("j");
    assert_eq!(harness.render(), down, "`j` stops at the last row");

    harness.key("k");
    harness.key("k");
    let up = harness.render();
    assert!(
        up.contains("> Graphics") && !up.contains("> Platform"),
        "`k` walks back to the first workspace and stops there: {up}"
    );
}

#[tokio::test]
async fn enter_switches_the_scope_and_the_top_bar_follows() {
    let mut harness = open_over_demo().await;
    assert_eq!(
        top_bar(&harness.render()),
        "Graphics · DESKTOP-HTUI · memory · 0 runs",
        "the fixture's only active run is in the other workspace"
    );

    harness.key("j");
    harness.key("enter");
    harness.settle().await;

    assert!(
        harness.app().overlays.is_empty(),
        "entering a workspace closes the switcher"
    );
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(
        harness.app().scope.project_ids.len(),
        2,
        "the scope carries both projects of `Platform`, ordered by position"
    );
    assert_eq!(
        harness
            .app()
            .projects
            .iter()
            .map(|p| p.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["htui", "agy"],
        "`workspace_project.position` decides the order"
    );

    let frame = harness.render();
    assert_eq!(
        top_bar(&frame),
        "Platform · DESKTOP-HTUI · memory · 1 run",
        "the top bar reads the new workspace and its active-run count"
    );
    insta::assert_snapshot!("after_switch", frame);
}

#[tokio::test]
async fn esc_closes_the_switcher_and_leaves_the_scope_alone() {
    let mut harness = open_over_demo().await;
    harness.key("j");
    harness.key("esc");
    harness.settle().await;

    assert!(
        harness.app().overlays.is_empty(),
        "`Esc` closes the overlay"
    );
    assert_eq!(
        harness.app().top_bar.workspace,
        "Graphics",
        "a cursor move that was never confirmed changes nothing"
    );
    assert!(
        !harness.render().contains("Workspaces"),
        "the box is gone from the frame"
    );
}

#[tokio::test]
async fn an_empty_store_shows_no_workspaces() {
    let mut harness = Harness::empty().with_overlay(Box::new(WorkspaceSwitcher::new()));
    register_all(harness.app());
    harness.settle().await;
    let frame = harness.render();

    assert!(
        frame.contains("no workspaces"),
        "an empty store is an empty list, not a blank box: {frame}"
    );
    harness.key("enter");
    assert!(
        !harness.app().overlays.is_empty(),
        "`Enter` on an empty list does nothing"
    );
    insta::assert_snapshot!("switcher_empty", frame);
}
