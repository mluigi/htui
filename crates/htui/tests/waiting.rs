//! The waiting-on-you list (MOD-69 plan D5-D8), end to end through a `Harness` over the demo
//! world: the top bar's two counts, `Ctrl+W` from every tab, a list that follows the store on the
//! refresh tick, and `Enter` onto the row's step in the Runs pane.
//!
//! The Harness enters the Graphics workspace at startup, which owes nothing; the demo's waiting
//! rows are Platform's, so most cases move there first (the `tests/reveal.rs` walk). The agent
//! runtime is installed because a Backlog selection sends a `PromptPreview`, which `settle` would
//! answer `Failed` onto the status line. Over the untouched demo, Platform holds one row (`htui`
//! FEAT-2, blocked with no run: Reopen) and one active run (`RUN_2`, queued), blueprint H-5.
#![cfg(feature = "testkit")]

use chrono::{TimeDelta, Utc};
use htui::agent_worker::AgentRuntime;
use htui::app::{Action, register_all};
use htui::testkit::Harness;
use htui::ui::overlay::WaitingList;
use htui::ui::tabs::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{
    Claim, GraphSnapshot, NewRun, NewRunStep, OpenPermission, PermissionId, RelayOption,
    RelayOptionKind, RelaySessionId, RunId, RunMode, StepId, StepStatus,
};
use htui_core::store::{MemStore, ParkOutcome, ReadStore as _, StepFence, WriteStore as _};
use uuid::Uuid;

/// The top bar over Graphics, which owes nothing.
const GRAPHICS: &str = "Graphics · DESKTOP-HTUI · memory · 0 working · 0 waiting";

/// The demo shell over `store`, settled in its startup workspace (Graphics).
async fn graphics(store: &MemStore) -> Harness {
    let mut harness =
        Harness::over(store.clone()).with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    harness
}

/// The demo shell over `store`, moved to Platform, on the Backlog, everything served.
async fn platform(store: &MemStore) -> Harness {
    let mut harness = graphics(store).await;
    harness.key("w");
    harness.drive_to_end().await;
    harness.key("j");
    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    harness
}

/// One shell refresh (`update.rs`'s private `TICKS_PER_REFRESH` is 4), everything served.
async fn refresh(harness: &mut Harness) {
    for _ in 0..4 {
        harness.app().update(Action::Tick);
    }
    harness.drive_to_end().await;
}

/// The frame's first line: the top bar.
fn top_bar(harness: &mut Harness) -> String {
    harness
        .render()
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_owned()
}

/// Whether the waiting list is the top overlay.
fn list_is_open(harness: &mut Harness) -> bool {
    harness.app().overlays.top().map(|top| top.id()) == Some(WaitingList::ID)
}

/// A gate park on `htui` ANA-2, as the engine leaves one: a claimed run whose `prd` step at slot
/// `0.1` is `done` and whose `plan` step at slot `1.1` ran and parked at `awaiting_approval`. Two
/// steps, so a reveal that lands on the run's first step instead of the parked one is caught
/// (review T1). The run carries the demo `RUN_1`'s snapshot, decoded, so the classifier reads the
/// `feature` phases. Answers the run, the parked step and the claim's owner, which still holds the
/// lease (a park keeps it).
async fn park_ana_2(store: &MemStore) -> (RunId, StepId, Uuid) {
    let snapshot = store
        .run(ids::RUN_1)
        .await
        .expect("run")
        .and_then(|run| run.graph_snapshot)
        .expect("RUN_1 carries a snapshot");
    let snapshot: GraphSnapshot = serde_json::from_value(snapshot).expect("RUN_1's snapshot");
    let owner = Uuid::now_v7();
    let at = Utc::now();
    let run = store
        .create_run(NewRun {
            id: RunId::new(),
            project_id: ids::PROJECT_HTUI,
            item_id: ids::HTUI_ANA_2,
            mode: RunMode::Manual,
            target_box_id: ids::BOX,
            started_by: ids::USER,
            graph_snapshot: snapshot,
            repo_scope: Vec::new(),
            queued_at: at,
            batch_id: None,
        })
        .await
        .expect("create_run")
        .id;
    assert_eq!(
        store
            .claim_run(run, ids::BOX, owner, at, TimeDelta::minutes(30))
            .await
            .expect("claim_run"),
        Claim::Admitted
    );
    let running = |position: i32, phase: &'static str| async move {
        let step = store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position,
                attempt: 1,
                fanout_index: 0,
                phase_name: phase.to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: Some("opus".to_owned()),
            })
            .await
            .expect("create_step")
            .id;
        assert!(
            store
                .transition_step(step, StepStatus::Pending, StepStatus::Running, at)
                .await
                .expect("transition_step"),
            "pending -> running"
        );
        step
    };
    let prd = running(0, "prd").await;
    assert!(
        store
            .transition_step(prd, StepStatus::Running, StepStatus::Done, at)
            .await
            .expect("transition_step"),
        "running -> done"
    );
    let step = running(1, "plan").await;
    assert_eq!(
        store
            .park_step(StepFence::Lease(owner), step)
            .await
            .expect("park_step"),
        ParkOutcome::Parked
    );
    (run, step, owner)
}

/// An open `edit: src/lib.rs` permission request on `step`, under the claim's `owner`.
async fn ask_permission(store: &MemStore, run: RunId, step: StepId, owner: Uuid) {
    let id = PermissionId::new();
    assert_eq!(
        store
            .open_permission(OpenPermission {
                id,
                run_id: run,
                run_step_id: step,
                session: RelaySessionId::new(),
                request_id: "req-1".to_owned(),
                tool_call_id: Some("call-1".to_owned()),
                summary: Some("edit: src/lib.rs".to_owned()),
                options: vec![RelayOption {
                    id: "allow-once".to_owned(),
                    label: "Allow once".to_owned(),
                    kind: RelayOptionKind::AllowOnce,
                }],
                owner,
            })
            .await
            .expect("open_permission"),
        id
    );
}

/// The Runs pane's cursor line: the detail pane's row that starts with the pane's `▸`, past the
/// list's right border and the detail's left one (the `tests/reveal.rs` reading).
fn cursor_line(frame: &str) -> String {
    let rows: Vec<&str> = frame
        .lines()
        .filter_map(|line| {
            line.split_once("\u{2502}\u{2502}")
                .map(|(_, detail)| detail)
        })
        .filter(|detail| detail.starts_with('\u{25b8}'))
        .collect();
    match rows.as_slice() {
        [row] => (*row).to_owned(),
        _ => panic!("expected one cursor line in the detail pane, got {rows:?}:\n{frame}"),
    }
}

/// The index of the one frame line holding `columns`, a row of the list read column by column.
fn row_at(frame: &str, columns: &str) -> usize {
    let at: Vec<usize> = frame
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(columns))
        .map(|(index, _)| index)
        .collect();
    match at.as_slice() {
        [index] => *index,
        _ => panic!("expected one row reading {columns:?}, got {at:?}:\n{frame}"),
    }
}

#[tokio::test]
async fn the_top_bar_counts_working_and_waiting() {
    let store = MemStore::demo();
    let mut harness = graphics(&store).await;
    assert_eq!(top_bar(&mut harness), GRAPHICS);

    let mut harness = platform(&store).await;
    assert_eq!(
        top_bar(&mut harness),
        "Platform · DESKTOP-HTUI · memory · 1 working · 1 waiting"
    );
}

/// Plan D5: the parked run moves from working to waiting, and the list grows by its gate row.
#[tokio::test]
async fn a_park_shows_on_the_next_refresh_tick() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    park_ana_2(&store).await;
    refresh(&mut harness).await;
    assert_eq!(
        top_bar(&mut harness),
        "Platform · DESKTOP-HTUI · memory · 1 working · 2 waiting"
    );
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn ctrl_w_opens_the_list_from_every_tab_and_esc_closes_it() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    for tab in ["1", "2", "3", "4", "5"] {
        harness.key(tab);
        harness.drive_to_end().await;
        harness.key("ctrl-w");
        assert!(list_is_open(&mut harness), "Ctrl+W from tab {tab}");
        harness.key("esc");
        assert!(harness.app().overlays.is_empty(), "Esc closes it");
    }
}

/// Plan D6: the open list reads the shell's last reply, so a refresh redraws it with no request
/// of its own and no reopening. Review R1: the row highlighted on open stays highlighted when the
/// park sorts a new row above it, before any key.
#[tokio::test]
async fn the_open_list_follows_a_park_without_reopening() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    harness.key("ctrl-w");
    let before = harness.render();
    assert!(before.contains("> FEAT-2"), "{before}");
    assert!(!before.contains("plan 1.1"), "{before}");

    park_ana_2(&store).await;
    refresh(&mut harness).await;
    assert!(list_is_open(&mut harness), "the same list stays open");
    let after = harness.render();
    assert!(after.contains("  ANA-2"), "{after}");
    assert!(after.contains("plan 1.1"), "{after}");
    assert!(after.contains("> FEAT-2  \u{2014}"), "{after}");
}

/// Plan D8: `Enter` closes the list and lands on the row's step in the Runs pane.
#[tokio::test]
async fn enter_opens_the_rows_step_in_the_runs_pane() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    park_ana_2(&store).await;
    refresh(&mut harness).await;

    harness.key("ctrl-w");
    harness.key("enter");
    harness.drive_to_end().await;
    assert!(harness.app().overlays.is_empty());
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    let frame = harness.render();
    assert!(frame.contains("\u{250c} ANA-2 "), "{frame}");
    // Review T1: the parked `plan`, not the run's first step `prd`.
    let line = cursor_line(&frame);
    assert!(line.contains("plan"), "{line}\n{frame}");
    assert!(!line.contains("prd"), "{line}\n{frame}");
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn the_list_over_graphics_says_nothing_is_waiting() {
    let store = MemStore::demo();
    let mut harness = graphics(&store).await;
    harness.key("ctrl-w");
    let frame = harness.render();
    assert!(frame.contains("nothing is waiting on you"), "{frame}");
    insta::assert_snapshot!("graphics_empty", frame);
}

/// Plan D9: the rows in project, key, run and step order, a run's gate before its permission and
/// the item's Reopen row last of all.
#[tokio::test]
async fn the_list_over_platform_holds_a_gate_a_permission_and_a_reopen() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    let (run, step, owner) = park_ana_2(&store).await;
    ask_permission(&store, run, step, owner).await;
    refresh(&mut harness).await;

    harness.key("ctrl-w");
    let frame = harness.render();
    // Review T2: each row matched on its key, step and reason columns (padded to the widest
    // value of each: `FEAT-2`, `plan 1.1`, `permission`), not on a bare substring.
    let gate = row_at(&frame, "ANA-2   plan 1.1  gate        gate");
    let permission = row_at(&frame, "ANA-2   plan 1.1  permission  edit: src/lib.rs");
    let reopen = row_at(
        &frame,
        "FEAT-2  \u{2014}         unblock     blocked, no active run: u reopens it",
    );
    assert!(gate < permission && permission < reopen, "{frame}");
    insta::assert_snapshot!("platform_mixed", frame);
}
