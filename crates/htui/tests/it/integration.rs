//! What [`register_all`] wires together (blueprint E, T6).
//!
//! T4's `tests/backlog.rs` and T5's `tests/shell.rs` each build their own view by hand, so neither
//! of them proves that the shell a user actually starts has the Backlog tab in front, the
//! Workspaces tab reachable and the two of them talking to the same scope. That is this file.
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::register_all;
use htui::testkit::Harness;
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
        "`register_all` lands on the Backlog (MOD-116 M1 D11), so it is the tab a user lands on"
    );
    assert_eq!(
        harness.app().tabs.len(),
        7,
        "Workspaces, Backlog, Skills, Requirements, Settings, Chat and Logs, in that order \
         (MOD-2 milestone 3 registered Chat, MOD-39 plan T3 put Requirements third, MOD-122 D4 \
         put Logs last, MOD-116 M1 D11 put Workspaces first)"
    );
    assert!(
        harness.app().overlays.is_empty(),
        "the startup workspace was entered from the first reply, so no overlay was ever opened"
    );
    // At 100 columns the seven tab chips and `? keys` leave no room for the working count and the
    // store, so the header is read at 160 and the snapshot below keeps the 100-column frame.
    harness.resize(160, 30);
    let header = top_bar(&harness.render());
    harness.resize(100, 30);
    assert!(
        header.starts_with("Graphics") && header.contains("0 working · 0 waiting · memory"),
        "the header reads the startup scope: {header}"
    );
    assert!(
        frame.contains(
            " \u{b9}Workspaces  \u{b2}Backlog  \u{b3}Skills  \u{2074}Requirements  \u{2075}Settings"
        ),
        "the tab chips are in registration order: {frame}"
    );
    // MOD-122 D4: Logs registers last; MOD-116 M1 D11 moved every digit up by one.
    assert!(
        header.contains("\u{2076}Chat  \u{2077}Logs"),
        "Chat takes 6 and Logs 7: {header}"
    );
    assert!(
        harness.app().keys_line().contains("w workspaces"),
        "the global `w` binding is offered"
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

    harness.key("3");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("skills")
    );

    harness.key("2");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("backlog"),
        "`2` is the Backlog tab now that Workspaces is registered first"
    );
}

/// MOD-116 M1 D11: the Workspaces tab is tab `1` and the Backlog tab `2`.
#[tokio::test]
async fn the_workspaces_tab_is_tab_one_and_backlog_is_two() {
    let mut harness = demo_shell().await;
    harness.key("1");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("workspaces")
    );
    harness.key("2");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("backlog")
    );
}

/// MOD-116 M1 D10: `w` focuses the Workspaces tab, which lists every workspace; no overlay opens.
#[tokio::test]
async fn w_jumps_to_the_workspaces_tab() {
    let mut harness = demo_shell().await;

    harness.key("w");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty(), "`w` opens no overlay");
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("workspaces")
    );
    let frame = harness.render();
    assert!(
        frame.contains("Graphics") && frame.contains("Platform"),
        "the tab lists every workspace: {frame}"
    );
}

/// MOD-116 M1 D9: `o` on the tab `w` reached enters the workspace under the cursor; the focus
/// stays on the tab, and the Backlog reads its rows for the new scope.
#[tokio::test]
async fn o_on_the_tab_reached_by_w_changes_the_scope() {
    let mut harness = demo_shell().await;

    harness.key("w");
    harness.drive_to_end().await;
    // Graphics, overview, Vulkan Tutorials, Platform.
    for _ in 0..3 {
        harness.key("j");
    }
    harness.key("o");
    harness.drive_to_end().await;

    assert!(harness.app().overlays.is_empty());
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("workspaces"),
        "the focus stays on the tab"
    );
    harness.key("2");
    harness.drive_to_end().await;
    assert!(
        harness.render().contains("agy"),
        "the Backlog tab re-read its rows for the new scope"
    );
}

/// MOD-116 M1 D11: an empty store lands on the Workspaces tab, which says how to make one.
#[tokio::test]
async fn an_empty_store_lands_on_the_workspaces_tab() {
    let mut harness = Harness::empty();
    register_all(harness.app());
    harness.drive_to_end().await;
    let frame = harness.render();

    assert!(harness.app().overlays.is_empty(), "no overlay opened");
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("workspaces"),
        "nothing entered a workspace, so the empty first reply landed on the Workspaces tab"
    );
    assert!(
        frame.contains("No workspaces yet."),
        "an empty store is an empty list, not a blank shell: {frame}"
    );
}
