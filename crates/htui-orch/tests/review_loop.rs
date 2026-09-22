//! `gate::review_loop` against a `graph_snapshot` whose phases are not a dense, sorted array.
//!
//! Every snapshot the walk mints today *is* dense and sorted, because `graph::resolve` renumbers
//! the phases it reads (`crates/htui-orch/src/graph.rs`). Nothing else promises it: the column is
//! `ck_run_graph_snapshot`-checked for non-null and nothing more, `NewRun.graph_snapshot` is taken
//! as given by both stores, and milestone 5's recovery sweep adopts runs this process did not
//! create. So "the phase at index `p` has `position == p`" is an assumption about a value that
//! arrives from outside, and these three cases are what it costs to stop assuming it.
//!
//! The loop is driven directly rather than through the walk on purpose: `Engine` mints its snapshot
//! through `graph::resolve` and therefore cannot produce one of these shapes, which is exactly why
//! the hazard was invisible to the conformance suite.

use chrono::Utc;
use htui_core::fixtures::{demo_data, ids};
use htui_core::model::{
    GraphSnapshot, ItemId, NewRun, NewRunStep, Run, RunId, RunMode, SnapshotPhase, StepId,
    StepStatus,
};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
use htui_orch::fake::TestClock;
use htui_orch::gate::{GateContext, LoopOutcome, LoopStop, review_loop};

/// The item every case here queues its run on: seeded `open`, so `create_run` accepts it.
const ITEM: ItemId = ids::HTUI_ANA_2;

/// The seeded `feature` snapshot — `prd, plan, implement, review` at positions 0 to 3.
fn feature_snapshot() -> GraphSnapshot {
    let run = demo_data()
        .runs
        .into_iter()
        .find(|row| row.id == ids::RUN_1)
        .expect("the fixture holds RUN_1");
    serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
        .expect("the fixture snapshot is a `GraphSnapshot`")
}

/// One phase of the seeded snapshot, by name.
fn phase(snapshot: &GraphSnapshot, name: &str) -> SnapshotPhase {
    snapshot
        .phases
        .iter()
        .find(|phase| phase.name == name)
        .expect("the `feature` snapshot names prd, plan, implement and review")
        .clone()
}

/// A `queued` run on [`ITEM`] carrying `snapshot` verbatim — the shape a restored or hand-authored
/// row can have, and the one `graph::resolve` would never mint.
async fn run_with(store: &MemStore, snapshot: GraphSnapshot) -> Run {
    let item = store
        .item(ITEM)
        .await
        .expect("MemStore never fails a read")
        .expect("the fixture holds the item");
    store
        .create_run(NewRun {
            id: RunId::new(),
            project_id: item.project_id,
            item_id: item.id,
            mode: RunMode::Manual,
            target_box_id: ids::BOX,
            started_by: item.created_by,
            graph_snapshot: snapshot,
            repo_scope: Vec::new(),
            queued_at: Utc::now(),
        })
        .await
        .expect("the item is `open`, so it queues")
}

/// A step at `(position, attempt 1, 0)` moved to `status` through the legal path.
async fn step_at(
    store: &MemStore,
    run: RunId,
    position: i32,
    name: &str,
    status: StepStatus,
) -> StepId {
    let now = Utc::now();
    let step = store
        .create_step(NewRunStep {
            id: StepId::new(),
            run_id: run,
            position,
            attempt: 1,
            fanout_index: 0,
            phase_name: name.to_owned(),
            agent_id: None,
            model: None,
        })
        .await
        .expect("the position is free");
    if status != StepStatus::Pending {
        store
            .transition_step(step.id, StepStatus::Pending, StepStatus::Running, now)
            .await
            .expect("`pending -> running` is the walk's own move");
        if status != StepStatus::Running {
            store
                .transition_step(step.id, StepStatus::Running, status, now)
                .await
                .expect("the caller names a status reachable from `running`");
        }
    }
    step.id
}

/// The step row, re-read.
async fn row(store: &MemStore, run: RunId, step: StepId) -> htui_core::model::RunStep {
    store
        .run_steps(run)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|candidate| candidate.id == step)
        .expect("the case created it")
}

/// A snapshot with a **gap**: `implement` at position 4 and `review` at position 5, four phases in
/// the array.
///
/// `loop_target` answers 4, which is a legal index into nothing: the array holds four entries. Read
/// positionally this panics with an out-of-bounds index, which is a walk that dies rather than a
/// run that refuses — and ANA-2 invariant 7 wants a row a human can read either way.
#[tokio::test]
async fn a_sparse_snapshot_does_not_index_past_the_phase_array() {
    let store = MemStore::demo();
    let clock = TestClock::new();
    let mut snapshot = feature_snapshot();
    snapshot.phases[2].position = 4;
    snapshot.phases[3].position = 5;

    let run = run_with(&store, snapshot.clone()).await;
    let implement = step_at(&store, run.id, 4, "implement", StepStatus::Done).await;
    let review = step_at(&store, run.id, 5, "review", StepStatus::Failed).await;

    let ctx = GateContext {
        store: &store,
        clock: &clock,
        run: &run,
        snapshot: &snapshot,
        user: store.this_user().expect("the fixture seeds one `app_user`"),
        box_id: ids::BOX,
    };
    let outcome = review_loop(&ctx, &row(&store, run.id, review).await)
        .await
        .expect("the loop reads rows and writes rows; neither refuses here");

    assert_eq!(
        outcome,
        LoopOutcome::Resumed {
            position: 4,
            attempt: 2
        },
        "the loop target is a `position`, and the phase carrying it is the one whose budget counts"
    );
    assert_eq!(
        row(&store, run.id, implement).await.status,
        StepStatus::Superseded
    );
    assert_eq!(
        row(&store, run.id, review).await.status,
        StepStatus::Cancelled,
        "`failed -> superseded` is illegal, so the rejecting review is retired by its one legal move"
    );
}

/// A snapshot whose phases are **unsorted**: the array is `review, implement, plan, prd` and every
/// `position` is the seeded one.
///
/// Read positionally, index 2 is `plan` — a different phase with a different name and a different
/// `retry_limit`. The two are told apart by giving them different budgets: `implement` may not be
/// attempted again and `plan` may, so a loop that read the wrong row would resume where this one
/// escalates, and the note a human reads would name the wrong phase.
#[tokio::test]
async fn an_unsorted_snapshot_picks_the_phase_whose_position_matches() {
    let store = MemStore::demo();
    let clock = TestClock::new();
    let mut snapshot = feature_snapshot();
    let mut implement = phase(&snapshot, "implement");
    implement.retry_limit = 0;
    let mut plan = phase(&snapshot, "plan");
    plan.retry_limit = 1;
    snapshot.phases = vec![
        phase(&snapshot, "review"),
        implement,
        plan,
        phase(&snapshot, "prd"),
    ];

    let run = run_with(&store, snapshot.clone()).await;
    step_at(&store, run.id, 2, "implement", StepStatus::Done).await;
    let review = step_at(&store, run.id, 3, "review", StepStatus::Failed).await;

    let ctx = GateContext {
        store: &store,
        clock: &clock,
        run: &run,
        snapshot: &snapshot,
        user: store.this_user().expect("the fixture seeds one `app_user`"),
        box_id: ids::BOX,
    };
    let outcome = review_loop(&ctx, &row(&store, run.id, review).await)
        .await
        .expect("the loop reads rows and writes rows; neither refuses here");

    assert_eq!(
        outcome,
        LoopOutcome::Escalated {
            attempts: 1,
            reason: LoopStop::Exhausted
        },
        "`implement` is the phase at position 2 and its budget is spent; `plan`'s is not, and \
         `plan` is what sits at index 2 of this array"
    );
    let notes: Vec<String> = store
        .notes(ITEM)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .map(|note| note.body)
        .collect();
    assert!(
        notes.iter().any(|body| body.contains("phase `implement`")),
        "§4.4's escalation note names the phase whose budget ran out: {notes:?}"
    );
}

/// A sparse snapshot with **no** loop target at all: `prd` at 0 and `review` at 2, and nothing at 1.
///
/// `loop_target` refuses this one before any lookup — no earlier `implement`, and the immediately
/// preceding position names no phase — so the run ends `failed` with `no_loop_target` and the loop
/// answers [`LoopOutcome::NoTarget`]. Pinned here beside the other two because "sparse" must mean a
/// named refusal in every branch that meets it, not a panic in one and a refusal in another.
#[tokio::test]
async fn a_sparse_snapshot_with_no_predecessor_refuses_by_name() {
    let store = MemStore::demo();
    let clock = TestClock::new();
    let mut snapshot = feature_snapshot();
    let mut review = phase(&snapshot, "review");
    review.position = 2;
    snapshot.phases = vec![phase(&snapshot, "prd"), review];

    let run = run_with(&store, snapshot.clone()).await;
    let step = step_at(&store, run.id, 2, "review", StepStatus::Failed).await;

    let ctx = GateContext {
        store: &store,
        clock: &clock,
        run: &run,
        snapshot: &snapshot,
        user: store.this_user().expect("the fixture seeds one `app_user`"),
        box_id: ids::BOX,
    };
    let outcome = review_loop(&ctx, &row(&store, run.id, step).await)
        .await
        .expect("a terminal review is a refusal, not an error");

    assert_eq!(outcome, LoopOutcome::NoTarget);
    assert_eq!(
        store
            .run(run.id)
            .await
            .expect("MemStore never fails a read")
            .expect("the case created it")
            .failure
            .as_deref(),
        Some("no_loop_target"),
        "plan D5's terminal review, on a snapshot with a hole in it"
    );
}
