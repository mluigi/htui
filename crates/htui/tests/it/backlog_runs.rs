//! Backlog › Runs on the kit (MOD-115 M4 L-runs), end to end through a `Harness` over the demo
//! world: the steps in words and their marks (E10), the flow's nodes as titled kit panes (D-16),
//! the close-out as a kit question over the pane (C7), and both views at 80 columns (MOD-81).
//! `tests/backlog.rs` keeps the Runs behaviour at the default 100x30.
#![cfg(feature = "testkit")]

use std::sync::Arc;

use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::run_worker::RunRuntime;
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::model::WorkspaceSummary;
use htui_core::store::MemStore;
use htui_orch::fake::{FakeIsolator, FakeVerifier};

/// Rows down from the arrival row (`ANA-1`) to htui `FEAT-1`, whose one run has four steps.
const TO_FEAT_1: usize = 3;

/// The demo fixture's workspace with this slug, as the harness's own store answers it.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// A settled Backlog tab scoped to `Platform` on a `width` x `height` terminal, on `ANA-1`.
async fn backlog_at(width: u16, height: u16) -> Harness {
    let mut harness = Harness::demo()
        .size(width, height)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// A settled Backlog tab as [`backlog_at`] makes, at the default 100x30, with a run runtime over
/// fakes that answers the close-out's preview (no session runs: nothing is started).
async fn driving() -> Harness {
    driving_at(100, 30).await
}

/// [`driving`] on a `width` x `height` terminal.
async fn driving_at(width: u16, height: u16) -> Harness {
    let runtime = RunRuntime::with_parts(
        Arc::new(FakeIsolator::new()),
        Arc::new(FakeVerifier::new()),
        DriverFactory::new(),
    );
    let mut harness = Harness::over(MemStore::demo())
        .size(width, height)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_run_runtime(runtime);
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// `harness` moved `down` rows and onto the Runs sub-tab, settled.
async fn on_runs(harness: &mut Harness, down: usize) {
    for _ in 0..down {
        harness.key("j");
        harness.drive_to_end().await;
    }
    harness.key("l");
    harness.drive_to_end().await;
}

/// MOD-81, E10: `FEAT-1`'s Runs at 80 columns. The phase leads each step, and the run and step
/// statuses read their marks; the Runs grid is the 43 cells of a 100-column detail pane, so the
/// narrower pane shows its left part (the redesign for narrow panes is MOD-119's).
#[tokio::test]
async fn the_runs_list_at_80_columns() {
    let mut harness = backlog_at(80, 24).await;
    on_runs(&mut harness, TO_FEAT_1).await;
    // MOD-144: the pane opens on the demo's stepless rework run `RUN_4`; `J` steps to `RUN_1`'s
    // `prd`.
    harness.key("J");
    harness.drive().await;
    let frame = harness.render();
    for phase in ["\u{25b8} prd", "  plan", "  implement", "  review"] {
        assert!(frame.contains(phase), "`{phase}` leads its step:\n{frame}");
    }
    assert!(frame.contains("graph  \u{25cf} done"), "{frame}");
    assert!(frame.contains("approved"), "{frame}");
    assert!(!frame.contains("0.1"), "no slot id:\n{frame}");
    insta::assert_snapshot!("runs_80", frame);
}

/// MOD-81, D-16, E10: `ANA-1`'s fan-out run as a flow at 80 columns: each candidate a rounded
/// node titled by its place, its phase then its status mark inside.
#[tokio::test]
async fn the_flow_at_80_columns() {
    let mut harness = backlog_at(80, 24).await;
    on_runs(&mut harness, 0).await;
    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{256d}\u{2500} candidate 1 "),
        "the first candidate's node is titled by its place:\n{frame}"
    );
    assert!(frame.contains("\u{25cf} done \u{2713}"), "{frame}");
    assert!(!frame.contains("0.1/"), "no slot id:\n{frame}");
    insta::assert_snapshot!("flow_80", frame);
}

/// C7 end to end: `C` on `ANA-1` (done, no live run) asks in a kit question drawn inside the
/// detail pane, over the run list; its keys are on the question's border and the detail pane's
/// own bottom border carries none while it is up. `n` answers and the hints come back.
#[tokio::test]
async fn the_close_out_asks_inside_the_detail_pane() {
    let mut harness = driving().await;
    on_runs(&mut harness, 0).await;
    harness.key("C");
    harness.drive_to_end().await;
    let frame = harness.render();
    let lines: Vec<&str> = frame.lines().collect();
    let top = lines
        .iter()
        .position(|line| line.contains("\u{256d} Close out "))
        .unwrap_or_else(|| panic!("the close-out question is drawn:\n{frame}"));
    let column = lines[top]
        .find("\u{256d} Close out ")
        .map(|byte| lines[top][..byte].chars().count())
        .expect("the title is on its row");
    assert!(
        column > 56,
        "inside the detail pane, right of the list:\n{frame}"
    );
    assert!(
        frame.contains("\u{2570} \u{2190}/\u{2192} resolution \u{b7} y next \u{b7} n cancel "),
        "the question's keys, whole, on its border:\n{frame}"
    );
    let bottom = lines.last().expect("a frame has rows");
    assert!(
        !bottom.contains("y next") && !bottom.contains("resolution"),
        "the pane draws no hint under the question:\n{frame}"
    );
    assert!(
        frame.contains("kind   status"),
        "the run list stays under the question:\n{frame}"
    );

    harness.key("n");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(!frame.contains("Close out"), "`n` answered:\n{frame}");
}

/// C7 at 80 columns (MOD-81): the close-out's typed stage shows the key as it is typed, whole,
/// on its own row under the question that names it; the question's prompt does not leave the
/// field a few cells beside it.
#[tokio::test]
async fn the_typed_close_out_shows_the_key_typed_at_80_columns() {
    let mut harness = driving_at(80, 24).await;
    on_runs(&mut harness, 0).await;
    for key in ["C", "y"] {
        harness.key(key);
        harness.drive_to_end().await;
    }
    for key in ["A", "N", "A", "-", "1"] {
        harness.key(key);
    }
    harness.drive_to_end().await;
    let frame = harness.render();
    let lines: Vec<&str> = frame.lines().collect();
    let top = lines
        .iter()
        .position(|line| line.contains("\u{256d} Close out "))
        .unwrap_or_else(|| panic!("the close-out question is drawn:\n{frame}"));
    let left = lines[top]
        .find("\u{256d} Close out ")
        .map(|byte| lines[top][..byte].chars().count())
        .expect("the corner is on the row");
    // The modal's content, row by row, between its side borders.
    let inside: Vec<String> = lines[top + 1..]
        .iter()
        .take_while(|line| !line.chars().nth(left).is_some_and(|c| c == '\u{2570}'))
        .map(|line| {
            line.chars()
                .skip(left + 1)
                .take_while(|c| *c != '\u{2502}')
                .collect::<String>()
        })
        .collect();
    assert!(
        inside.iter().any(|row| row.contains("to close it")),
        "the question names the key:\n{frame}"
    );
    assert!(
        inside.iter().any(|row| row.trim() == "ANA-1"),
        "the key typed is whole on its own row:\n{frame}"
    );
    insta::assert_snapshot!("closeout_typed_80", frame);
}
