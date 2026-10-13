//! Lane L-B of MOD-67 M5: the Runs sub-tab. Its questions, text fields and document view answer
//! through their stacks, `alt-y` no longer answers a question (D7), and browsing draws a hint row
//! built from the keys in force (ANA-26 §8). Helpers stay in this file.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::run_worker::{RunRuntime, StepAuthor};
use htui::testkit::Harness;
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
async fn seeded_store() -> MemStore {
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
async fn driving(store: MemStore, adapter: &FakeAdapter) -> Harness {
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
async fn parked(store: MemStore) -> Harness {
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
