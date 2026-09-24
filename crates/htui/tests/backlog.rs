//! Backlog tab tests (blueprint §E, T4).
//!
//! Everything runs against `Harness::demo()` and the tab's own public surface, so no T3 or T5
//! file is touched and the T6 registration does not have to exist yet. Frames are 100x30, the
//! size the whole snapshot suite is pinned to (plan risk row).
#![cfg(feature = "testkit")]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::run_worker::{self, LiveChats, RunRuntime, StepAuthor};
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::event::{DoneEvent, DriverEvent, StopReason};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::{demo_at, ids};
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, DocumentId, ItemId, NewDocument, RunStep, SnapshotPhase,
    Transport, WorkspaceSummary,
};
use htui_core::store::{MemStore, WriteStore as _};
use htui_orch::Clock;
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use htui_store::Backend;
use serde_json::json;

/// The workspace of the demo fixture with this slug.
///
/// Read out of a throw-away `MemStore::demo()` rather than written by hand, so the row is exactly
/// the one the harness's own store answers with.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// A settled Backlog tab scoped to `Platform`: two projects, eleven items, all eight statuses.
///
/// `Harness::demo()` starts in `Graphics` (workspaces are ordered by name), so the scope is moved
/// the only way it ever moves: an `Action::SetScope` (plan D10).
async fn backlog() -> Harness {
    let mut harness = Harness::demo()
        .with_tab(Box::new(BacklogTab::new()))
        // MOD-2 milestone 9: the sixth read is the prompt preview, which the agent runtime owns
        // and `store_worker::spawn` always has. Without one here every selection would answer
        // `Failed` and take the status line, which is a harness artefact and not a shell state.
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// Moves the selection down `n` rows, serving what each move asks the store for.
async fn down(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.key("j");
        harness.drive_to_end().await;
    }
}

/// Cycles to the sub-tab `n` steps to the right of Body.
fn sub_tab(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.key("l");
    }
}

/// Rows down from the arrival row to htui `FEAT-1`, the item every sub-tab has data for.
///
/// The list is `htui` (header), `ANA-1`, `ANA-2`, `CLEAN-1`, `FEAT-1`, ... and the cursor arrives
/// on `ANA-1`: inside a project the rows are ordered by key prefix and then by number, which is
/// `MemStore`'s own order.
const TO_FEAT_1: usize = 3;

/// Rows down to htui `ANA-2`, the item nothing in the fixture attaches to.
const TO_ANA_2: usize = 1;

#[tokio::test]
async fn the_list_groups_the_two_project_workspace_by_project() {
    let mut harness = backlog().await;
    let frame = harness.render();
    assert!(frame.contains("htui"), "the first project header");
    assert!(frame.contains("agy"), "the second project header");
    assert!(
        frame.contains("awaiting_approval"),
        "every status renders in full"
    );
    insta::assert_snapshot!("list_grouped", frame);
}

#[tokio::test]
async fn enter_folds_and_unfolds_a_project_group() {
    let mut harness = backlog().await;
    harness.key("k");
    harness.drive_to_end().await;
    harness.key("enter");
    harness.drive_to_end().await;
    let folded = harness.render();
    assert!(
        !folded.contains("TUI scaffold"),
        "the folded group hides its items"
    );
    assert!(folded.contains("agy"), "the other group is untouched");
    insta::assert_snapshot!("list_folded", folded);

    harness.key("enter");
    harness.drive_to_end().await;
    assert!(
        harness.render().contains("TUI scaffold"),
        "Enter unfolds it again"
    );
}

#[tokio::test]
async fn j_k_g_and_shift_g_move_the_selection() {
    let mut harness = backlog().await;
    assert!(
        harness.render().contains("┌ ANA-1"),
        "the first item is selected on arrival"
    );

    down(&mut harness, TO_FEAT_1).await;
    assert!(harness.render().contains("┌ FEAT-1"), "three rows down");

    harness.key("k");
    harness.drive_to_end().await;
    assert!(harness.render().contains("┌ CLEAN-1"), "one row back up");

    harness.key("G");
    harness.drive_to_end().await;
    let last = harness.render();
    assert!(
        last.contains("┌ FIX-1"),
        "G lands on the last row: agy FIX-1"
    );
    insta::assert_snapshot!("list_last_row", last);

    harness.key("g");
    harness.drive_to_end().await;
    let first = harness.render();
    assert!(
        first.contains("┌ Detail") && first.contains("No item selected"),
        "g lands on the first project header, which has no detail"
    );
}

#[tokio::test]
async fn the_six_sub_tabs_render_the_selected_item() {
    for (steps, name) in [
        (0, "detail_body"),
        (1, "detail_runs"),
        (2, "detail_graph"),
        (3, "detail_documents"),
        (4, "detail_notes"),
        // The sixth is MOD-2 milestone 9's preview (plan D102), assembled by a task the runtime
        // spawned. Clipped to the pane's 43 columns here; `prompt_preview.rs` renders it wide
        // enough to read and asserts its bytes.
        (5, "detail_prompt"),
    ] {
        let mut harness = backlog().await;
        down(&mut harness, TO_FEAT_1).await;
        sub_tab(&mut harness, steps);
        insta::assert_snapshot!(name, harness.render());
    }
}

#[tokio::test]
async fn every_sub_tab_says_so_when_it_has_nothing() {
    for (steps, name) in [
        (0, "empty_body"),
        (1, "empty_runs"),
        (2, "empty_graph"),
        (3, "empty_documents"),
        (4, "empty_notes"),
        // Not the Prompt sub-tab: htui `ANA-2` has no document and no link, but it still *has* a
        // prompt — that is the whole point of the preview — so "this item has nothing" is not a
        // state it can be in. Its own empty state is the next case.
    ] {
        let mut harness = backlog().await;
        down(&mut harness, TO_ANA_2).await;
        sub_tab(&mut harness, steps);
        let frame = harness.render();
        assert!(
            frame.contains("┌ ANA-2"),
            "the empty cases all sit on htui ANA-2"
        );
        assert!(
            frame.contains("No "),
            "an empty sub-tab renders a message, never a blank pane (plan D11)"
        );
        insta::assert_snapshot!(name, frame);
    }
}

#[tokio::test]
async fn the_prompt_sub_tab_says_so_with_no_item_selected() {
    // The Prompt sub-tab's empty state is not "this item has no rows" — every item has a prompt —
    // but "there is no item": the cursor is on a project header. Plan D11 all the same, a message
    // and never a blank pane.
    let mut harness = backlog().await;
    harness.key("g");
    harness.drive_to_end().await;
    sub_tab(&mut harness, 5);
    let frame = harness.render();
    assert!(
        frame.contains("┌ Detail") && frame.contains("No item selected"),
        "g lands on the first project header:\n{frame}"
    );
    insta::assert_snapshot!("empty_prompt", frame);
}

#[tokio::test]
async fn h_and_l_cycle_the_sub_tabs_both_ways() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    harness.key("h");
    assert!(
        harness.render().contains("digest"),
        "h from Body wraps around to Prompt, the sixth since MOD-2 milestone 9"
    );
    harness.key("]");
    assert!(
        harness
            .render()
            .contains("Stand up the terminal application"),
        "] wraps forward to Body again"
    );
    harness.key("l");
    assert!(harness.render().contains("manual"), "l lands on Runs");
    harness.key("[");
    assert!(
        harness
            .render()
            .contains("Stand up the terminal application"),
        "[ goes back to Body"
    );
}

#[tokio::test]
async fn a_scope_change_clears_the_list_and_the_detail() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    assert!(harness.render().contains("TUI scaffold"));

    harness.app().update(Action::SetScope {
        workspace: workspace("graphics").await,
    });
    harness.drive_to_end().await;

    let frame = harness.render();
    assert!(
        !frame.contains("TUI scaffold"),
        "the other workspace's rows are gone"
    );
    assert!(
        frame.contains("Chapter 12 parity"),
        "the new scope was re-queried"
    );
}

// ---------------------------------------------------------------------------------------------
// The Runs pane driving a run (MOD-4 plan D166-D173, blueprint §9.6, §9.7).
// ---------------------------------------------------------------------------------------------

/// The one instant the engine writes in these cases, so a run's timestamps are byte-stable.
#[derive(Debug)]
struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> DateTime<Utc> {
        demo_at(23, 9)
    }
}

/// D203's author: every step writes one document of its phase's `output_kind`.
#[derive(Debug)]
struct Author;

impl StepAuthor for Author {
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument> {
        Some(NewDocument {
            id: DocumentId::new(),
            item_id: item,
            kind: phase.output_kind.clone(),
            title: format!("{} of attempt {}", phase.output_kind, step.attempt),
            body: "What the step found.\n\nThree sources agree; one does not.".to_owned(),
            produced_by_step_id: Some(step.id),
            created_by: ids::USER,
            created_at: demo_at(23, 9),
        })
    }
}

/// Blueprint F-O: the demo with its own agents disabled and one scripted `acp` row ready on the
/// demo box, so a walk's candidate chain names exactly it.
async fn seeded_store() -> MemStore {
    let store = MemStore::demo();
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        store.upsert_agent(&row).await.expect("the row is disabled");
    }
    let agent = AgentId::new();
    store
        .upsert_agent(&Agent {
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
        })
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

/// htui `ANA-2` on the Runs pane, its run started with `R` and parked at `research`.
async fn parked() -> Harness {
    let adapter = FakeAdapter::new();
    let mut harness = driving(seeded_store().await, &adapter).await;
    for _ in 0..TO_ANA_2 {
        harness.key("j");
        harness.drive().await;
    }
    sub_tab(&mut harness, 1);
    adapter.load(one_turn());
    harness.key("R");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("awaiting") && frame.contains("research"),
        "`R` started a run that parked at `research`:\n{frame}"
    );
    assert_eq!(harness.app().status, None, "and nothing failed");
    harness
}

/// Types `text`, a space as `space`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        if c == ' ' {
            harness.key("space");
        } else {
            harness.key(&c.to_string());
        }
    }
}

/// D168, D201: `x` on a parked step opens a note that takes every letter, including the ones the
/// list moves on.
#[tokio::test]
async fn x_note_letters_do_not_move_the_list() {
    let mut harness = parked().await;
    harness.key("x");
    type_text(&mut harness, "jkgGlh[]q1");
    harness.drive().await;
    let frame = harness.render();
    assert!(frame.contains("┌ ANA-2"), "the list did not move:\n{frame}");
    assert!(
        frame.contains("jkgGlh[]q1"),
        "every letter is in the note, and the Runs pane is still the one shown:\n{frame}"
    );
    assert!(!harness.app().should_quit, "`q` was typed, not obeyed");
}

#[tokio::test]
async fn the_reject_note_renders_under_the_parked_run() {
    let mut harness = parked().await;
    harness.key("x");
    type_text(&mut harness, "needs work");
    harness.drive().await;
    insta::assert_snapshot!("runs_reject_note", harness.render());
}

/// D173: `o` on the parked step opens the document it produced, over the pane, read-only.
#[tokio::test]
async fn o_shows_the_step_s_document_read_only() {
    let mut harness = parked().await;
    harness.key("o");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("What the step found."),
        "the body is shown:\n{frame}"
    );
    insta::assert_snapshot!("runs_artifact", frame);

    harness.key("esc");
    assert!(harness.render().contains("research"), "`Esc` is back on the list");
}

/// D167: `C` on a `done` item with no live run counts what the close-out writes, then asks for
/// the item's key typed back.
#[tokio::test]
async fn the_close_out_counts_then_asks_for_the_key() {
    let adapter = FakeAdapter::new();
    let mut harness = driving(MemStore::demo(), &adapter).await;
    sub_tab(&mut harness, 1);
    assert!(harness.render().contains("┌ ANA-1"), "the arrival row is htui `ANA-1`");
    harness.key("C");
    harness.drive().await;
    let warn = harness.render();
    assert!(warn.contains("close ANA-1"), "the counts line:\n{warn}");
    insta::assert_snapshot!("runs_closeout_warn", warn);

    harness.key("y");
    type_text(&mut harness, "ANA");
    harness.drive().await;
    let typed = harness.render();
    assert!(typed.contains("type ANA-1 to close it: ANA"), "{typed}");
    assert!(typed.contains("┌ ANA-1"), "`A`, `N` and `A` are typed, not bound");
    insta::assert_snapshot!("runs_closeout_typed", typed);
    assert_eq!(harness.app().status, None);
}

/// D168, D182: a refused key shows the engine guard's own sentence and sends nothing: with no run
/// runtime here, an `Orch` request would put `no run runtime in this build` on the status line.
#[tokio::test]
async fn the_runs_pane_greys_a_key_with_the_guard_s_sentence() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, 1);

    let verdicts = run_worker::actions(
        &Backend::memory(MemStore::demo()),
        ids::HTUI_FEAT_1,
        &LiveChats::default(),
    )
    .await
    .expect("the verdicts read");
    let sentence = verdicts.steps[&ids::STEP_PRD]
        .approve
        .clone()
        .expect_err("a `done` step cannot be approved");

    harness.key("a");
    harness.drive().await;
    assert_eq!(harness.app().status.as_deref(), Some(sentence.as_str()));
}
