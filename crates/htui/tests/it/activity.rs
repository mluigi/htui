//! The shell's activity panel (MOD-122 M3 D8-D10), end to end through a `Harness` over the demo
//! world with the Backlog laid out `ide_activity`: events a refresh brings, the pinned needs-you
//! rows, the keys the focused pane holds, and `Enter` onto a row's step, its chat or its
//! workspace.
//!
//! What the wire carries (one read in flight, a hidden pane never read, a refused read absorbed,
//! the retention sweep's schedule) is pinned in `app::update`'s `activity_tests`, which read the
//! worker's inbox; the harness keeps it private. The events are seeded with
//! `MemStore::push_activity` from here (blueprint F12: `testkit` does not turn on
//! `htui-core/test-support`, only the dev-dependencies do); a park is a real `park_step`, whose
//! gate event the store's activity writer records.
#![cfg(feature = "testkit")]

use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
use crossterm::event::{MouseButton, MouseEventKind};
use htui::agent_worker::AgentRuntime;
use htui::app::{Action, register_all};
use htui::testkit::Harness;
use htui::ui::shell::{Preset, TabLayout};
use htui::ui::state_file::{UiState, UiStore};
use htui::ui::tabs::ChatTab;
use htui::ui::tabs::backlog::{BACKLOG_LAYOUT, BacklogTab};
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::activity::{ActivityEvent, ActivityKind};
use htui_core::model::{
    Claim, GraphSnapshot, NewRun, NewRunStep, RunId, RunMode, StepId, StepRole, StepStatus,
};
use htui_core::store::{MemStore, ParkOutcome, ReadStore as _, StepFence, WriteStore as _};
use uuid::Uuid;

/// The UI state of a machine whose Backlog shows the activity pane (T6 makes it the default).
fn with_activity() -> UiStore {
    let store = UiStore::memory();
    let mut state = UiState::default();
    state.layouts.insert(
        BacklogTab::ID.0.to_owned(),
        TabLayout {
            preset: Preset::IdeActivity,
            ..BACKLOG_LAYOUT
        },
    );
    store.save(&state).expect("a memory store saves");
    store
}

/// The demo shell over `store`, settled in its startup workspace (Graphics), on the Backlog.
async fn graphics(store: &MemStore) -> Harness {
    let mut harness = Harness::over(store.clone())
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_ui_store(with_activity());
    register_all(harness.app());
    harness.drive_to_end().await;
    assert_eq!(harness.app().top_bar.workspace, "Graphics");
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
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

/// The instant every seeded event is stamped with (review L5): the harness prints the panel's
/// times in UTC, and a day before any run of these tests, so a row reads `11-02 09:41`.
fn seeded_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, 9, 41, 0)
        .single()
        .expect("a valid instant")
}

/// A handle on `store`'s rows whose clock is frozen at [`seeded_at`]: what it writes, the events
/// the store's activity writer records included, is stamped then.
fn seeding(store: &MemStore) -> MemStore {
    store.handle_at(seeded_at())
}

/// The activity pane's title line.
fn activity_title(frame: &str) -> String {
    frame
        .lines()
        .find(|line| line.contains("\u{2500} Activity \u{b7}"))
        .unwrap_or_else(|| panic!("the activity pane:\n{frame}"))
        .to_owned()
}

/// Whether the activity pane is the focused one: it carries the panel's hints.
fn activity_focused(frame: &str) -> bool {
    frame.contains("Enter open \u{b7} f filter")
}

/// Presses `tab` until the activity pane is focused.
fn focus_activity(harness: &mut Harness) {
    for _ in 0..3 {
        if activity_focused(&harness.render()) {
            return;
        }
        harness.key("tab");
    }
    let frame = harness.render();
    assert!(activity_focused(&frame), "the activity pane:\n{frame}");
}

/// Whether the Backlog's detail pane is on `key` (the `tests/waiting.rs` reading).
fn on_item(frame: &str, key: &str) -> bool {
    frame.contains(&format!("\u{256d}\u{2500} {key} "))
        || frame.contains(&format!("\u{2500} {key} \u{b7}"))
}

/// The Runs pane's cursor line: the detail pane's row that starts with the pane's `▸`.
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

/// An event of `kind` on htui `key` (`item`) in Platform, its run `RUN_1`'s `plan` step.
fn event(kind: ActivityKind, key: &str, item: htui_core::model::ItemId) -> ActivityEvent {
    ActivityEvent {
        id: 0,
        at: seeded_at(),
        kind,
        workspace: Some(ids::WORKSPACE_PLATFORM),
        project: Some(ids::PROJECT_HTUI),
        project_slug: "htui".to_owned(),
        item: Some(item),
        item_key: Some(key.to_owned()),
        run: Some(ids::RUN_1),
        chat: false,
        step: Some(ids::STEP_PLAN),
        step_name: Some("plan".to_owned()),
        permission: None,
        text: String::new(),
    }
}

/// A gate park on `htui` ANA-2 (the `tests/waiting.rs` fixture): a claimed run whose `prd` step
/// is `done` and whose `plan` step parked at `awaiting_approval`. Answers the run and the parked
/// step.
async fn park_ana_2(store: &MemStore) -> (RunId, StepId) {
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
            workspace_id: None,
            id: RunId::new(),
            project_id: ids::PROJECT_HTUI,
            item_id: ids::HTUI_ANA_2,
            mode: RunMode::Manual,
            target_box_id: ids::BOX,
            executing_box_id: None,
            started_by: ids::USER,
            graph_snapshot: snapshot,
            repo_scope: Vec::new(),
            queued_at: at,
            batch_id: None,
            target: None,
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
                role: StepRole::Step,
                parent_step_id: None,
                node_key: None,
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
    (run, step)
}

/// D8: an event written to the store shows within one refresh, newest first.
#[tokio::test]
async fn an_event_shows_within_one_refresh_newest_first() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    let before = harness.render();
    assert!(before.contains("No activity yet."), "{before}");

    seeding(&store).push_activity(event(ActivityKind::RunStarted, "ANA-1", ids::HTUI_ANA_1));
    seeding(&store).push_activity(event(ActivityKind::StepFailed, "ANA-2", ids::HTUI_ANA_2));
    refresh(&mut harness).await;
    let after = harness.render();
    let row = |key: &str| {
        after
            .lines()
            .position(|line| line.contains(&format!(" {key} plan")))
            .unwrap_or_else(|| panic!("a row on {key}:\n{after}"))
    };
    assert!(row("ANA-2") < row("ANA-1"), "newest first:\n{after}");
    assert_eq!(harness.app().status, None);
}

/// D4, D8: a parked step is pinned under `needs you` and counted in the title; answering it
/// unpins it on the next refresh. The frame is the Backlog with the panel, pinned and history.
#[tokio::test]
async fn a_park_is_pinned_and_counted_and_answering_it_unpins_it() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    seeding(&store).push_activity(event(ActivityKind::RunFinished, "ANA-1", ids::HTUI_ANA_1));
    let (_, step) = park_ana_2(&seeding(&store)).await;
    refresh(&mut harness).await;
    let pinned = harness.render();
    let title = activity_title(&pinned);
    assert!(
        title.contains("Activity \u{b7} \u{26a0} 1 needs you \u{b7} all workspaces"),
        "{title}"
    );
    assert!(
        pinned.contains("needs you \u{2500}"),
        "the divider:\n{pinned}"
    );
    insta::assert_snapshot!("pinned_and_history", pinned);

    assert!(
        store
            .transition_step(
                step,
                StepStatus::AwaitingApproval,
                StepStatus::Done,
                Utc::now()
            )
            .await
            .expect("transition_step"),
        "the gate is answered"
    );
    refresh(&mut harness).await;
    let answered = harness.render();
    let title = activity_title(&answered);
    assert!(!title.contains("needs you"), "unpinned: {title}");
    assert!(title.contains("Activity \u{b7} all workspaces"), "{title}");
}

/// D9, D10: `Tab` to the activity pane, whose hints it carries; `Enter` on an in-workspace row
/// opens the Backlog's Runs sub-tab on the row's step, focused, and `Backspace` returns to the
/// pane: one history entry.
#[tokio::test]
async fn enter_opens_the_rows_step_and_back_returns_to_the_activity_pane() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    park_ana_2(&seeding(&store)).await;
    refresh(&mut harness).await;
    focus_activity(&mut harness);
    insta::assert_snapshot!("focused", harness.render());

    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    let opened = harness.render();
    assert!(on_item(&opened, "ANA-2"), "{opened}");
    assert!(
        !activity_focused(&opened),
        "the jump left the pane:\n{opened}"
    );
    let line = cursor_line(&opened);
    assert!(line.contains("plan"), "the parked step: {line}\n{opened}");
    assert_eq!(harness.app().status, None);

    harness.key("backspace");
    harness.drive_to_end().await;
    let back = harness.render();
    assert!(
        activity_focused(&back),
        "back on the activity pane:\n{back}"
    );
}

/// D9: `Enter` on another workspace's row enters that workspace, then reveals the step, and the
/// status line names the workspace.
#[tokio::test]
async fn enter_on_another_workspaces_row_enters_it_and_reveals() {
    let store = MemStore::demo();
    let mut harness = graphics(&store).await;
    park_ana_2(&seeding(&store)).await;
    refresh(&mut harness).await;
    let listed = harness.render();
    assert!(listed.contains("@platform ANA-2"), "{listed}");
    focus_activity(&mut harness);

    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    assert_eq!(
        harness.app().status.as_deref(),
        Some("opened workspace Platform for ANA-2")
    );
    let opened = harness.render();
    assert!(on_item(&opened, "ANA-2"), "{opened}");
    let line = cursor_line(&opened);
    assert!(line.contains("plan"), "the parked step: {line}\n{opened}");
}

/// D8, D10: `f` cycles the filter, the title says which, and a filter that hides every row names
/// `f` as the way on.
#[tokio::test]
async fn f_cycles_the_filter_and_the_title_says_which() {
    let store = MemStore::demo();
    let mut harness = graphics(&store).await;
    seeding(&store).push_activity(event(ActivityKind::RunStarted, "ANA-1", ids::HTUI_ANA_1));
    refresh(&mut harness).await;
    focus_activity(&mut harness);
    for filter in ["needs you", "failures", "this workspace", "all workspaces"] {
        harness.key("f");
        let frame = harness.render();
        let title = activity_title(&frame);
        assert!(
            title.contains(&format!("Activity \u{b7} {filter} ")),
            "{title}"
        );
        if filter == "this workspace" {
            assert!(frame.contains("No events under this workspace."), "{frame}");
            assert!(frame.contains("f shows all workspaces"), "{frame}");
        }
    }
}

/// D9: `Enter` on a chat row opens its replay in the Chat tab.
#[tokio::test]
async fn enter_on_a_chat_row_opens_the_replay() {
    let store = MemStore::demo();
    let mut harness = graphics(&store).await;
    seeding(&store).push_activity(ActivityEvent {
        chat: true,
        item: None,
        item_key: None,
        step: Some(ids::STEP_PRD),
        ..event(ActivityKind::StepFailed, "ANA-1", ids::HTUI_ANA_1)
    });
    refresh(&mut harness).await;
    focus_activity(&mut harness);
    let listed = harness.render();
    assert!(listed.contains(" chat plan"), "{listed}");

    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(harness.app().tabs.active_id(), Some(ChatTab::ID));
}

/// D10 (the T7 repair): with the Backlog's item form open and the activity pane focused, a typed
/// key reaches the form, not the panel.
#[tokio::test]
async fn an_open_form_keeps_the_keys_over_the_focused_activity_pane() {
    let store = MemStore::demo();
    let mut harness = platform(&store).await;
    seeding(&store).push_activity(event(ActivityKind::RunStarted, "ANA-1", ids::HTUI_ANA_1));
    refresh(&mut harness).await;
    harness.key("e");
    harness.drive_to_end().await;
    let open = harness.render();
    let row = open
        .lines()
        .position(|line| line.contains("\u{2500} Activity \u{b7}"))
        .unwrap_or_else(|| panic!("the activity pane:\n{open}"));
    let row = u16::try_from(row + 1).expect("fits");
    harness.mouse(MouseEventKind::Down(MouseButton::Left), 4, row);
    harness.mouse(MouseEventKind::Up(MouseButton::Left), 4, row);

    for key in ["Q", "x", "f"] {
        harness.key(key);
    }
    let typed = harness.render();
    assert!(typed.contains("Qxf"), "the form took the keys:\n{typed}");
    let title = activity_title(&typed);
    assert!(
        title.contains("all workspaces"),
        "the filter did not move: {title}"
    );
    harness.key("esc");
    harness.drive_to_end().await;
    let closed = harness.render();
    assert!(
        activity_focused(&closed),
        "the click had focused the activity pane, which has the keys once the form closes:\n{closed}"
    );
}

/// D11: the Backlog's default is `ide_activity`, so a machine with no saved layout opens it with
/// the activity pane below the list and the detail; a layout already saved in `ui-state.toml`
/// (here `ide`) wins over the default and keeps the pane away.
#[tokio::test]
async fn the_backlog_opens_with_the_activity_pane_unless_a_saved_layout_says_otherwise() {
    let mut fresh = Harness::over(MemStore::demo())
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_ui_store(UiStore::memory());
    register_all(fresh.app());
    fresh.drive_to_end().await;
    assert_eq!(fresh.app().tabs.active_id(), Some(BacklogTab::ID));
    let shown = fresh.render();
    assert!(
        activity_title(&shown).contains("all workspaces"),
        "a fresh UI state shows the activity pane:\n{shown}"
    );
    assert!(
        fresh.ui_store().load().state.layouts.is_empty(),
        "the default is not written back"
    );

    let saved = UiStore::memory();
    let mut state = UiState::default();
    state.layouts.insert(
        BacklogTab::ID.0.to_owned(),
        TabLayout {
            preset: Preset::Ide,
            ..BACKLOG_LAYOUT
        },
    );
    saved.save(&state).expect("a memory store saves");
    let mut kept = Harness::over(MemStore::demo())
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_ui_store(saved);
    register_all(kept.app());
    kept.drive_to_end().await;
    assert_eq!(kept.app().tabs.active_id(), Some(BacklogTab::ID));
    let shown = kept.render();
    assert!(
        !shown.contains("\u{2500} Activity"),
        "a saved `ide` layout keeps the activity pane away:\n{shown}"
    );
}
