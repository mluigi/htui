//! What [`register_all`] wires together (blueprint E, T6).
//!
//! T4's `tests/backlog.rs` and T5's `tests/shell.rs` each build their own view by hand, so neither
//! of them proves that the shell a user actually starts has the Backlog tab in front, the
//! workspace switcher reachable and the two of them talking to the same scope. That is this file.
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::register_all;
use htui::testkit::Harness;
use htui::ui::overlay::WorkspaceSwitcher;
use htui_agent::registry::DriverFactory;

/// The registered shell over the demo fixture, settled into its startup workspace.
///
/// It carries an agent runtime because `store_worker::spawn` does (`AgentRuntime::production`):
/// since MOD-2 milestone 9 the Backlog tab's sixth read is the prompt preview, which the runtime
/// owns, and a harness without one would photograph a shell no user starts — the point of this
/// file being that it renders the shell a user *does*. The empty factory is enough: a preview
/// spawns no process.
async fn demo_shell() -> Harness {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness
}

/// The top bar, i.e. the first line of a rendered frame.
fn top_bar(frame: &str) -> String {
    frame.lines().next().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn the_demo_shell_starts_inside_the_first_workspace_on_the_backlog_tab() {
    let mut harness = demo_shell().await;
    let frame = harness.render();

    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("backlog"),
        "Backlog is registered first, so it is the tab a user lands on"
    );
    assert_eq!(
        harness.app().tabs.len(),
        4,
        "Backlog, Skills, Settings and Chat, in that order (MOD-2 milestone 3 registered the last)"
    );
    assert!(
        harness.app().overlays.is_empty(),
        "the startup workspace was entered from the first reply, so no switcher was ever opened"
    );
    assert_eq!(
        top_bar(&frame),
        "Graphics · DESKTOP-HTUI · memory · 0 runs",
        "the top bar reads the startup scope"
    );
    assert!(
        frame.contains(" 1 Backlog  2 Skills  3 Settings"),
        "the tab strip is in registration order: {frame}"
    );
    assert!(
        frame.contains("w workspaces"),
        "the status line advertises the global `w` binding: {frame}"
    );
    insta::assert_snapshot!("demo_shell", frame);
}

#[tokio::test]
async fn the_backlog_tab_shows_the_scope_and_the_digits_reach_the_other_tabs() {
    let mut harness = demo_shell().await;
    assert!(
        harness.render().contains("Vulkan Tutorials"),
        "the active tab was activated, so its rows were requested and answered"
    );

    harness.key("2");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("skills")
    );

    harness.key("1");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("backlog"),
        "`1` is the Backlog tab now that it is registered first"
    );
}

#[tokio::test]
async fn w_opens_the_workspace_switcher_and_esc_closes_it_again() {
    let mut harness = demo_shell().await;

    harness.key("w");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().overlays.len(),
        1,
        "`w` opens exactly one switcher"
    );
    let frame = harness.render();
    assert!(
        frame.contains("Workspaces") && frame.contains("Graphics") && frame.contains("Platform"),
        "the factory registered under the switcher's id built a loaded switcher: {frame}"
    );

    harness.key("esc");
    assert!(
        harness.app().overlays.is_empty(),
        "`Esc` closes it, leaving the Backlog tab where it was"
    );
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("backlog")
    );
}

#[tokio::test]
async fn the_switcher_reached_by_w_still_changes_the_scope() {
    let mut harness = demo_shell().await;

    harness.key("w");
    harness.drive_to_end().await;
    harness.key("j");
    harness.key("enter");
    harness.drive_to_end().await;

    assert!(harness.app().overlays.is_empty());
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert!(
        harness.render().contains("agy"),
        "the Backlog tab re-read its rows for the new scope"
    );
}

#[tokio::test]
async fn an_empty_store_starts_with_the_switcher_open_over_no_workspaces() {
    let mut harness = Harness::empty();
    register_all(harness.app());
    harness.drive_to_end().await;
    let frame = harness.render();

    assert_eq!(
        harness.app().overlays.top().map(|o| o.id()),
        Some(WorkspaceSwitcher::ID),
        "nothing entered a workspace, so the empty first reply opened the startup switcher"
    );
    assert!(
        frame.contains("no workspaces"),
        "an empty store is an empty list, not a blank shell: {frame}"
    );
}
