//! The leave dialog over the demo world (MOD-116 M2 T3, D9; `R-TUI-18`): every live workspace
//! listed, and `S` stopping them gracefully before leaving.
//!
//! The Harness enters `Graphics` at startup; `Platform` has `RUN_2` queued, `RUN_4` running and
//! `RUN_5` parked, and no stop row. The dialog reads the shell's run counts
//! (`TopBarState::workspace_runs`), asked once a second; while its stops settle it asks for them
//! itself on each of the shell's refreshes (four ticks). A direct write goes into the same
//! `MemStore` through a clone.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::app::Action;
use htui::testkit::Harness;
use htui::ui::overlay::{LeaveDialog, Overlay};
use htui_core::fixtures::ids;
use htui_core::model::{RunStatus, StopMode, WorkspaceId};
use htui_core::store::{MemStore, WriteStore as _};

/// The demo world with the leave dialog registered, the startup workspace entered and the
/// counts read once.
async fn demo(store: MemStore) -> Harness {
    let mut harness = Harness::over(store).size(100, 30).with_leave_dialog();
    harness.settle().await;
    refresh(&mut harness).await;
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    harness
}

/// The shell's once-a-second refresh, served: four ticks, then every request answered.
async fn refresh(harness: &mut Harness) {
    for _ in 0..4 {
        harness.app().update(Action::Tick);
    }
    harness.settle().await;
}

/// The overlay on top, if any.
fn top(harness: &mut Harness) -> Option<htui::ui::overlay::OverlayId> {
    harness.app().overlays.top().map(Overlay::id)
}

/// `workspace`'s stop row's mode, if any.
async fn stop_of(store: &MemStore, workspace: WorkspaceId) -> Option<StopMode> {
    store
        .workspace_stop(workspace)
        .await
        .expect("the stop row is read")
        .map(|row| row.mode)
}

/// `q` opens the dialog, `S` writes the stops; both served.
async fn stop_first(harness: &mut Harness) {
    harness.key("q");
    harness.settle().await;
    assert_eq!(top(harness), Some(LeaveDialog::ID));
    harness.key("S");
    harness.settle().await;
}

/// Platform runs in the background, so it has its row; Graphics, open but idle in the demo
/// (`0 working · 0 waiting`), has none.
#[tokio::test]
async fn the_leave_dialog_lists_platform_not_the_idle_graphics() {
    let mut harness = demo(MemStore::demo()).await;
    harness.key("q");
    harness.settle().await;
    assert_eq!(top(&mut harness), Some(LeaveDialog::ID));
    let frame = harness.render();
    assert!(
        frame.contains("\u{25d0} 1 working \u{b7} \u{26a0} 1 waiting \u{b7} 1 queued   Platform"),
        "{frame}"
    );
    assert!(!frame.contains("waiting   Graphics"), "{frame}");
    assert!(frame.contains("S  stop first"), "{frame}");
    insta::assert_snapshot!("two_workspaces", frame);
}

#[tokio::test]
async fn s_stops_every_live_workspace_then_leaves_when_they_settle() {
    let store = MemStore::demo();
    let mut harness = demo(store.clone()).await;
    stop_first(&mut harness).await;
    assert_eq!(
        stop_of(&store, ids::WORKSPACE_PLATFORM).await,
        Some(StopMode::Graceful)
    );
    assert!(!harness.app().should_quit, "RUN_4 still runs");
    let frame = harness.render();
    assert!(
        frame.contains("waiting for 1 running run to stop"),
        "{frame}"
    );

    refresh(&mut harness).await;
    assert!(!harness.app().should_quit, "RUN_4 still runs");

    store
        .finish_run(ids::RUN_4, RunStatus::Done, None, Utc::now())
        .await
        .expect("RUN_4 finishes");
    refresh(&mut harness).await;
    assert!(harness.app().should_quit, "every stop has settled");
}

#[tokio::test]
async fn s_leaves_after_the_bound_when_a_run_never_stops() {
    let store = MemStore::demo();
    let mut harness = demo(store.clone()).await;
    stop_first(&mut harness).await;
    for second in 1..20 {
        refresh(&mut harness).await;
        assert!(!harness.app().should_quit, "second {second}");
    }
    refresh(&mut harness).await;
    assert!(harness.app().should_quit, "20 s waited");
    assert_eq!(
        stop_of(&store, ids::WORKSPACE_PLATFORM).await,
        Some(StopMode::Graceful),
        "the stop stays"
    );
}

#[tokio::test]
async fn a_second_ctrl_c_while_settling_leaves_at_once() {
    let mut harness = demo(MemStore::demo()).await;
    stop_first(&mut harness).await;
    assert!(!harness.app().should_quit);
    harness.key("ctrl-c");
    assert!(harness.app().should_quit);
}

#[tokio::test]
async fn esc_while_settling_stays_and_the_stop_stands() {
    let store = MemStore::demo();
    let mut harness = demo(store.clone()).await;
    stop_first(&mut harness).await;
    harness.key("Esc");
    harness.settle().await;
    assert_eq!(top(&mut harness), None, "the dialog is closed");
    for _ in 0..20 {
        refresh(&mut harness).await;
    }
    assert!(!harness.app().should_quit, "no dialog is left to leave");
    assert_eq!(
        stop_of(&store, ids::WORKSPACE_PLATFORM).await,
        Some(StopMode::Graceful)
    );
}
