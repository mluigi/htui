//! Lane L-B of MOD-67 M5: the Runs sub-tab. Its questions, text fields and document view answer
//! through their stacks, `alt-y` no longer answers a question (D7), and browsing draws a hint row
//! built from the keys in force (ANA-26 §8). Helpers stay in this file.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use htui::agent_worker::AgentRuntime;
use htui::app::{Action, register_all};
use htui::run_worker::{RunRuntime, StepAuthor};
use htui::testkit::Harness;
use htui::ui::tabs::ChatTab;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::event::{DoneEvent, DriverEvent, StopReason};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::{demo_at, edit_agent, ids};
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, DocumentId, ItemId, NewDocument, RunStatus, RunStep,
    SnapshotPhase, Transport, WorkspaceSummary,
};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
use htui_orch::Clock;
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use serde_json::json;

/// The workspace of the demo fixture with this slug, read out of a throw-away store.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// Rows down from the arrival row (htui `ANA-1`) to htui `ANA-2`, which has no run.
const TO_ANA_2: usize = 1;

/// The one instant the engine writes in these cases, so a run's timestamps are byte-stable.
#[derive(Debug)]
struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> DateTime<Utc> {
        demo_at(23, 9)
    }
}

/// Every step writes one document of its phase's `output_kind`.
#[derive(Debug)]
struct Author;

impl StepAuthor for Author {
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument> {
        Some(NewDocument {
            id: DocumentId::new(),
            item_id: item,
            kind: phase.output_kind.clone(),
            title: format!("{} of attempt {}", phase.output_kind, step.attempt),
            body: "What the step found.".to_owned(),
            produced_by_step_id: Some(step.id),
            created_by: ids::USER,
            created_at: demo_at(23, 9),
        })
    }
}

/// The demo with its own agents disabled and one scripted `acp` row ready on the demo box, so a
/// walk's candidate chain names exactly it (`tests/backlog.rs`'s `seeded_store`).
pub(super) async fn seeded_store() -> MemStore {
    let store = MemStore::demo();
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        edit_agent(&store, &row).await.expect("the row is disabled");
    }
    let agent = AgentId::new();
    store
        .upsert_agent(
            &Agent {
                id: agent,
                name: "scripted".to_owned(),
                transport: Transport::Acp,
                billing: Billing::Subscription,
                models: Vec::new(),
                default_model: Some("sonnet".to_owned()),
                launch: json!({ "command": "unused", "args": [] }),
                settings: json!({}),
                enabled: true,
                created_at: demo_at(0, 0),
                updated_at: demo_at(0, 0),
            },
            None,
        )
        .await
        .expect("the scripted row lands");
    store
        .upsert_agent_box(&AgentBox {
            agent_id: agent,
            box_id: ids::BOX,
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(demo_at(0, 0)),
            quota: None,
            quota_at: None,
            updated_at: demo_at(0, 0),
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        })
        .await
        .expect("the agent_box row lands");
    store
}

/// A settled Backlog tab over `store`, scoped to `Platform`, with a run runtime whose sessions
/// each play one `done` turn from `adapter`.
pub(super) async fn driving(store: MemStore, adapter: &FakeAdapter) -> Harness {
    let mut factory = DriverFactory::new();
    factory.register("acp", Box::new(adapter.clone()));
    let runtime = RunRuntime::with_parts(
        Arc::new(FakeIsolator::new()),
        Arc::new(FakeVerifier::new()),
        factory,
    )
    .with_clock(Arc::new(Fixed))
    .with_author(Arc::new(Author));
    let mut harness = Harness::over(store)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()))
        .with_run_runtime(runtime);
    harness.drive().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive().await;
    harness
}

/// One `done` turn: the session the research step's walk runs.
fn one_turn() -> Script {
    Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
        stop_reason: StopReason::EndTurn,
    }))])
}

/// htui `ANA-2` on the Runs pane over `store`, its run started with `R` and parked at `research`.
pub(super) async fn parked(store: MemStore) -> Harness {
    let adapter = FakeAdapter::new();
    let mut harness = driving(store, &adapter).await;
    for _ in 0..TO_ANA_2 {
        harness.key("j");
        harness.drive().await;
    }
    harness.key("l");
    adapter.load(one_turn());
    harness.key("R");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("awaiting") && frame.contains("research"),
        "`R` started a run that parked at `research`:\n{frame}"
    );
    harness
}

/// htui `ANA-1` (a `done` item) on the Runs pane, `C` pressed and the counts in: the close-out
/// warning.
async fn warned() -> Harness {
    let adapter = FakeAdapter::new();
    let mut harness = driving(MemStore::demo(), &adapter).await;
    harness.key("l");
    harness.key("C");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("close ANA-1 as"),
        "`C` counted and warns:\n{frame}"
    );
    harness
}

/// The status of the newest run of `item` in `store`.
async fn run_status(store: &MemStore, item: ItemId) -> RunStatus {
    store
        .runs(item)
        .await
        .expect("the memory store never fails")
        .first()
        .expect("the item has a run")
        .status
}

/// D7, D12: `alt-y` is no act in the cancel question, so it cancels nothing and the question
/// stays; `y` is the one that cancels.
#[tokio::test]
async fn alt_y_does_not_cancel_a_run() {
    let store = seeded_store().await;
    let mut harness = parked(store.clone()).await;
    harness.key("c");
    harness.drive().await;
    assert!(
        harness.render().contains("cancel this run?"),
        "`c` asks first"
    );

    harness.key("alt-y");
    assert_eq!(harness.queued(), 0, "`alt-y` sends nothing");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("cancel this run?"),
        "and the question stays:\n{frame}"
    );
    assert_ne!(
        run_status(&store, ids::HTUI_ANA_2).await,
        RunStatus::Cancelled
    );

    harness.key("y");
    assert_eq!(harness.queued(), 1, "`y` sends the cancel");
    harness.drive().await;
    let frame = harness.render();
    assert!(!frame.contains("cancel this run?"), "answered:\n{frame}");
    assert_eq!(
        run_status(&store, ids::HTUI_ANA_2).await,
        RunStatus::Cancelled
    );
}

/// D7: `alt-y` does not continue the close-out warning; `y` moves on to the typed key.
#[tokio::test]
async fn alt_y_does_not_continue_a_close_out() {
    let mut harness = warned().await;
    harness.key("alt-y");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("close ANA-1 as") && !frame.contains("to close it"),
        "`alt-y` leaves the warning up:\n{frame}"
    );

    harness.key("y");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("type ANA-1 to close it"),
        "`y` asks for the key:\n{frame}"
    );
}

/// The close-out resolution keys are fixed value keys matched on plain keys (blueprint §1):
/// `l` and `Right` cycle the resolution, `alt-l` does not.
#[tokio::test]
async fn the_resolution_keys_stay_fixed_and_plain() {
    let mut harness = warned().await;
    let resolution = |frame: &str| {
        frame
            .lines()
            .find_map(|row| {
                let (_, rest) = row.split_once("close ANA-1 as ")?;
                rest.split_whitespace().next().map(str::to_owned)
            })
            .unwrap_or_else(|| panic!("the warning names a resolution:\n{frame}"))
    };
    let first = resolution(&harness.render());

    harness.key("alt-l");
    harness.drive().await;
    assert_eq!(
        resolution(&harness.render()),
        first,
        "`alt-l` cycles nothing"
    );

    harness.key("l");
    harness.drive().await;
    let second = resolution(&harness.render());
    assert_ne!(second, first, "`l` cycles the resolution");

    harness.key("right");
    harness.drive().await;
    assert_ne!(
        resolution(&harness.render()),
        second,
        "`Right` cycles it too"
    );
    assert!(
        harness.render().contains("┌ ANA-1"),
        "and the Backlog stays on ANA-1's Runs pane"
    );
}

/// What the rejection note reads: the row under `reject with a note:`, in the Runs pane.
fn note(frame: &str) -> String {
    let mut rows = frame.lines();
    rows.find(|row| row.contains("reject with a note:"))
        .unwrap_or_else(|| panic!("the note is open:\n{frame}"));
    let row = rows.next().expect("a row under the prompt");
    let (_, pane) = row
        .rsplit_once("││")
        .unwrap_or_else(|| panic!("the row crosses into the pane: {row:?}"));
    pane.trim_end_matches('│').trim_end().to_owned()
}

/// Blueprint §1 item 3: `F1` opens help from the rejection note (it swallowed it) and closes it
/// again, and the note keeps what was typed. `Tab` is no key of the note's and is swallowed;
/// `2` is a letter there, typed rather than selecting a tab. The box's content follows the
/// pane's stack only on the merged tree (main §7 step 2).
#[tokio::test]
async fn f1_opens_help_from_the_reject_note_and_the_note_stays() {
    let mut harness = parked(seeded_store().await).await;
    harness.key("x");
    harness.key("n");
    harness.key("o");
    assert_eq!(
        note(&harness.render()),
        "no",
        "`x` opened the note and it is typed"
    );

    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` opens help from the note");
    harness.key("f1");
    assert!(!harness.app().help_visible, "`F1` closes it");
    assert_eq!(note(&harness.render()), "no", "the note stays");

    harness.key("tab");
    let frame = harness.render();
    assert_eq!(note(&frame), "no", "`Tab` is swallowed");
    assert!(
        frame.contains("┌ ANA-2"),
        "still on ANA-2's Runs pane:\n{frame}"
    );
    harness.key("2");
    let frame = harness.render();
    assert_eq!(note(&frame), "no2", "`2` is typed, not a tab select");
    assert!(
        frame.contains("┌ ANA-2"),
        "still on ANA-2's Runs pane:\n{frame}"
    );
    assert_eq!(harness.queued(), 0, "and nothing was sent");
}

/// The hints (MOD-67 M6 T2): MOD-115 draws them on the body's bottom border, the frame's last
/// row, so this is that row with the rule and the corners trimmed off.
fn hint_row(frame: &str) -> String {
    frame
        .lines()
        .last()
        .unwrap_or_default()
        .trim_matches([
            '\u{2514}', '\u{2518}', '\u{2500}', '\u{2570}', '\u{256f}', ' ',
        ])
        .to_owned()
}

/// The list's own keys, after a pane's in the shell's hint row when both fit (MOD-67 M6 T2).
const LIST_KEYS: &str = "j/k move · h/l sub-tab · f filter · N new · e edit";

/// ANA-26 §8: browsing the Runs pane, the shell's hint row (MOD-67 M6 T2) leads with the pane's
/// keys, built from the keys in force, one per view: the run list, the flow, and an item with no
/// run; the list's own follow at 100 columns.
#[tokio::test]
async fn the_runs_browse_hint_row() {
    let adapter = FakeAdapter::new();
    let mut harness = driving(MemStore::demo(), &adapter).await;
    harness.key("l");
    harness.drive().await;
    let frame = harness.render();
    assert!(frame.contains("▸"), "ANA-1's runs are listed:\n{frame}");
    assert_eq!(
        hint_row(&frame),
        format!("J/K step · Enter replay · v flow · ? keys · {LIST_KEYS}"),
        "the list's row:\n{frame}"
    );

    harness.key("v");
    harness.drive().await;
    let frame = harness.render();
    assert_eq!(
        hint_row(&frame),
        format!("J/K step · +/- zoom · v list · ? keys · {LIST_KEYS}"),
        "the flow's row:\n{frame}"
    );
    harness.key("v");

    for _ in 0..TO_ANA_2 {
        harness.key("j");
        harness.drive().await;
    }
    let frame = harness.render();
    assert!(
        frame.contains("No runs for this item."),
        "ANA-2 has no run:\n{frame}"
    );
    assert_eq!(
        hint_row(&frame),
        format!("R start run · ? keys · {LIST_KEYS}"),
        "an item with no run:\n{frame}"
    );
}

/// Rows down from the arrival row (htui `ANA-1`) to htui `FEAT-1`, the item with a recorded
/// step (`tests/replay.rs`).
pub(super) const TO_FEAT_1: usize = 3;

/// D6: `Enter` on a step replays it (the registered shell opens the Chat tab, no refusal); on a
/// Runs pane with no step under the cursor the pane declines and the Backlog answers with the
/// miss (the legacy row on this branch, `backlog.replay` after L-A: the same text).
#[tokio::test]
async fn enter_replays_the_step_and_misses_without_one() {
    let mut harness = Harness::demo();
    register_all(harness.app());
    harness.settle().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.settle().await;
    for _ in 0..TO_ANA_2 {
        harness.key("j");
        harness.settle().await;
    }
    harness.key("l");
    harness.settle().await;
    harness.key("enter");
    harness.drive().await;
    assert_eq!(
        harness.app().status.as_deref(),
        Some("select a step in the Runs pane (J/K) to replay it"),
        "no step under the cursor: the miss"
    );
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));

    for _ in TO_ANA_2..TO_FEAT_1 {
        harness.key("j");
        harness.settle().await;
    }
    harness.key("J");
    harness.key("enter");
    harness.drive().await;
    assert_eq!(
        harness.app().tabs.active_id(),
        Some(ChatTab::ID),
        "a step under the cursor replays into the Chat tab"
    );
    assert_eq!(harness.app().status, None, "and no refusal was raised");
}

/// The Runs pane's row the step cursor (`▸`) is on.
fn cursor_row(frame: &str) -> String {
    frame
        .lines()
        .filter_map(|row| row.rsplit_once("││").map(|(_, pane)| pane))
        .find(|pane| pane.contains('▸'))
        .unwrap_or_else(|| panic!("the Runs pane has a step cursor:\n{frame}"))
        .trim_end_matches('│')
        .trim_end()
        .to_owned()
}

/// The lane's rebinding test (MOD-67 M5 D12): `[backlog.runs] next_step = "n"` moves the step
/// cursor to `n`. `n` moves it, `J` no longer does, and the browse row names `n`. `n` passes the
/// unconverted Backlog tab (it intercepts no `n`) and the converted one (no `backlog`, `list` or
/// `pane` act binds it).
#[tokio::test]
async fn a_rebound_next_step_moves_the_cursor_and_j_capital_is_inert() {
    let keys = htui::keys::load_str("version = 1\n[backlog.runs]\nnext_step = \"n\"\n")
        .expect("the keys load");
    let mut harness = Harness::over(MemStore::demo())
        .with_tab(Box::new(BacklogTab::new()))
        .with_keys(keys);
    harness.drive().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive().await;
    for _ in 0..TO_FEAT_1 {
        harness.key("j");
        harness.drive().await;
    }
    harness.key("l");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        cursor_row(&frame).contains("prd"),
        "the cursor starts on FEAT-1's first step:\n{frame}"
    );
    assert!(
        hint_row(&frame).starts_with("n/K step · Enter replay · v flow · ? keys"),
        "the browse row names the rebound key:\n{frame}"
    );

    harness.key("J");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        cursor_row(&frame).contains("prd"),
        "`J` no longer moves the cursor:\n{frame}"
    );

    harness.key("n");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        cursor_row(&frame).contains("plan"),
        "`n` moves it to the next step:\n{frame}"
    );
    assert!(frame.contains("┌ FEAT-1"), "on FEAT-1 still:\n{frame}");
}
