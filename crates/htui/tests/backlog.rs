//! Backlog tab tests (blueprint §E, T4).
//!
//! Everything runs against `Harness::demo()` and the tab's own public surface, so no T3 or T5
//! file is touched and the T6 registration does not have to exist yet. Frames are 100x30, the
//! size the whole snapshot suite is pinned to (plan risk row).
#![cfg(feature = "testkit")]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use htui::agent_worker::AgentRuntime;
use htui::app::{Action, RevealKind};
use htui::keymap::KeyScope;
use htui::requirements::decision_citation_stays;
use htui::run_worker::{self, LiveChats, RunRuntime, StepAuthor};
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::event::{DoneEvent, DriverEvent, StopReason};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::{demo_at, edit_agent, ids};
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, CitationKind, DocumentId, ItemId, NewDocument, NewRunStep,
    Resolution, RunStatus, RunStep, SnapshotPhase, Status, StepId, Transport, WorkspaceSummary,
};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
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
        harness.render().contains("R-STO-1 addresses v1"),
        "h from Body wraps around to Reqs, the seventh since MOD-39 (blueprint F-5)"
    );
    harness.key("h");
    assert!(
        harness.render().contains("digest"),
        "a second h lands on Prompt, the sixth since MOD-2 milestone 9"
    );
    harness.key("l");
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

/// MOD-14 D8: `m` opens the Graph from the Backlog's own arm; its binding is the help box's half,
/// next to the `Enter` row it mirrors.
#[tokio::test]
async fn m_is_on_the_backlog_help_line() {
    let mut harness = Harness::demo();
    htui::app::register_all(harness.app());
    let help = harness
        .app()
        .keymap
        .help_line(&KeyScope::Tab(BacklogTab::ID));
    assert!(help.contains("m open graph"), "{help}");
    assert!(help.contains("Enter replay step"), "{help}");
}

// ---------------------------------------------------------------------------------------------
// The Reqs sub-tab (MOD-39 PRD D3, D4; plan P12; blueprint §5).
// ---------------------------------------------------------------------------------------------

/// Steps right of Body to the Reqs sub-tab, registered after Prompt.
const TO_REQS: usize = 6;

/// Rows down to htui `CLEAN-1`, the item that cites no requirement.
const TO_CLEAN_1: usize = 2;

/// On htui `FEAT-1`'s Reqs sub-tab, `c` opens the picker over htui's active requirements but
/// `R-STO-1`, which `FEAT-1` already addresses, and `j` puts its cursor on `R-ENT-2`, the second:
/// the harness and the frame at the Pick stage.
async fn picking_r_ent_2() -> (Harness, String) {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_REQS);
    harness.key("c");
    harness.drive_to_end().await;
    harness.key("j");
    let frame = harness.render();
    (harness, frame)
}

/// [`picking_r_ent_2`], then `Enter` picks it and `a` cites it as `addresses`.
async fn cited_r_ent_2() -> Harness {
    let (mut harness, _) = picking_r_ent_2().await;
    harness.key("enter");
    harness.key("a");
    harness.drive_to_end().await;
    harness
}

/// PRD D3: the arrival row `ANA-1` cites `R-ENT-1` at v1, and `ANA-2` amended it to v2 since, so
/// the citation is the demo's one suspect citation.
#[tokio::test]
async fn the_reqs_sub_tab_shows_ana_1_s_suspect_citation() {
    let mut harness = backlog().await;
    sub_tab(&mut harness, TO_REQS);
    let frame = harness.render();
    assert!(frame.contains("┌ ANA-1"), "the arrival row:\n{frame}");
    assert!(
        frame.contains("R-ENT-1 addresses v1 ! suspect"),
        "the citation, its stamp and the marker:\n{frame}"
    );
    assert!(
        frame.contains("Every item has a stable key"),
        "the requirement's first line under it:\n{frame}"
    );
    insta::assert_snapshot!("detail_reqs", frame);
}

/// Plan P9: `r` re-stamps the suspect citation at the requirement's current version.
#[tokio::test]
async fn r_reconfirms_the_suspect_citation() {
    let mut harness = backlog().await;
    sub_tab(&mut harness, TO_REQS);
    harness.key("r");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert_eq!(harness.app().status, None, "the re-confirm applied");
    assert!(
        frame.contains("R-ENT-1 addresses v2"),
        "stamped at v2:\n{frame}"
    );
    assert!(
        !frame.contains("! suspect"),
        "and no longer suspect:\n{frame}"
    );
    insta::assert_snapshot!("detail_reqs_reconfirmed", frame);
}

/// PRD D4: `c` offers the active requirements of the item's own project that it does not cite
/// yet, `Enter` picks one and `a` cites it as `addresses`; the list cursor never moves while the
/// picker captures, and lands on the new citation once it is made.
#[tokio::test]
async fn c_cites_a_requirement_of_the_item_s_project() {
    let (mut harness, picker) = picking_r_ent_2().await;
    assert!(
        picker.contains("┌ FEAT-1"),
        "the list cursor stayed:\n{picker}"
    );
    assert!(
        picker.contains("▸ R-ENT-2 later An item may carry"),
        "the picker's cursor is on R-ENT-2:\n{picker}"
    );
    assert!(
        !picker.contains("R-STO-1 must"),
        "R-STO-1, already cited, is not offered:\n{picker}"
    );
    insta::assert_snapshot!("detail_reqs_cite_picker", picker);

    harness.key("enter");
    harness.key("a");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert_eq!(harness.app().status, None, "the cite applied");
    assert!(
        frame.contains("▸ R-ENT-2 addresses v1"),
        "the new citation, under the cursor:\n{frame}"
    );
    assert!(
        frame.contains("R-STO-1 addresses v1"),
        "next to the one FEAT-1 already had:\n{frame}"
    );
}

/// Plan P9: `u` asks, `y` uncites; the row is gone.
#[tokio::test]
async fn u_then_y_uncites() {
    let mut harness = cited_r_ent_2().await;
    assert!(
        harness.render().contains("▸ R-ENT-2 addresses v1"),
        "the new citation is under the cursor"
    );
    harness.key("u");
    let asking = harness.render();
    assert!(
        asking.contains("uncite R-ENT-2 (addresses)?"),
        "`u` asks first:\n{asking}"
    );
    harness.key("y");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert_eq!(harness.app().status, None, "the uncite applied");
    assert!(!frame.contains("R-ENT-2"), "the citation is gone:\n{frame}");
    assert!(
        frame.contains("R-STO-1 addresses v1"),
        "and the other one stays:\n{frame}"
    );
}

/// Plan P9: `ANA-2`'s `amends` citation records a decision, so `u` answers the status line and
/// asks nothing.
#[tokio::test]
async fn u_on_an_amends_citation_is_answered_on_the_status_line() {
    let mut harness = backlog().await;
    down(&mut harness, TO_ANA_2).await;
    sub_tab(&mut harness, TO_REQS);
    harness.key("u");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().status,
        Some(decision_citation_stays(CitationKind::Amends))
    );
    let frame = harness.render();
    assert!(
        frame.contains("R-ENT-1 amends v2"),
        "the citation stays:\n{frame}"
    );
    assert!(!frame.contains("uncite R-"), "and nothing asks:\n{frame}");
}

/// Plan D11: `CLEAN-1` cites nothing, and the pane says so rather than going blank.
#[tokio::test]
async fn the_reqs_sub_tab_says_so_when_it_has_nothing() {
    let mut harness = backlog().await;
    down(&mut harness, TO_CLEAN_1).await;
    sub_tab(&mut harness, TO_REQS);
    let frame = harness.render();
    assert!(frame.contains("┌ CLEAN-1"), "the empty case:\n{frame}");
    assert!(
        frame.contains("No requirements cited."),
        "a message, never a blank pane:\n{frame}"
    );
    insta::assert_snapshot!("empty_reqs", frame);
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
    assert!(
        harness.render().contains("research"),
        "`Esc` is back on the list"
    );
}

/// D167: `C` on a `done` item with no live run counts what the close-out writes, then asks for
/// the item's key typed back.
#[tokio::test]
async fn the_close_out_counts_then_asks_for_the_key() {
    let adapter = FakeAdapter::new();
    let mut harness = driving(MemStore::demo(), &adapter).await;
    sub_tab(&mut harness, 1);
    assert!(
        harness.render().contains("┌ ANA-1"),
        "the arrival row is htui `ANA-1`"
    );
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
    assert!(
        typed.contains("┌ ANA-1"),
        "`A`, `N` and `A` are typed, not bound"
    );
    insta::assert_snapshot!("runs_closeout_typed", typed);
    assert_eq!(harness.app().status, None);
}

/// MOD-39 plan P13 through the whole tab: the engine's preview starts an `open` item's picker on
/// `withdrawn`; `l`/`→` and `h`/`←`, which cycle the sub-tabs elsewhere, reach the Runs pane's
/// picker while it captures input; and the key typed back closes the item with the resolution
/// picked, not the default.
#[tokio::test]
async fn the_close_out_picker_takes_h_and_l_and_lands_the_pick() {
    let adapter = FakeAdapter::new();
    let store = MemStore::demo();
    let mut harness = driving(store.clone(), &adapter).await;
    for _ in 0..TO_ANA_2 {
        harness.key("j");
        harness.drive().await;
    }
    sub_tab(&mut harness, 1);
    harness.key("C");
    harness.drive().await;
    let warn = harness.render();
    assert!(
        warn.contains("close ANA-2 as withdrawn"),
        "an `open` item's picker starts on `withdrawn`:\n{warn}"
    );

    for (key, picked) in [
        ("l", "superseded"),
        ("right", "duplicate"),
        ("h", "superseded"),
        ("left", "withdrawn"),
        ("left", "rejected"),
    ] {
        harness.key(key);
        harness.drive().await;
        let frame = harness.render();
        assert!(
            frame.contains(&format!("close ANA-2 as {picked}")),
            "`{key}` picks `{picked}` and stays on the Runs pane:\n{frame}"
        );
    }

    harness.key("y");
    type_text(&mut harness, "ANA-2");
    harness.key("enter");
    harness.drive().await;
    assert_eq!(harness.app().status, None, "the close-out was accepted");
    let closed = store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("the memory store never fails")
        .expect("the item exists");
    assert_eq!(
        (closed.status, closed.resolution),
        (Status::Closed, Some(Resolution::Rejected))
    );
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

// ---------------------------------------------------------------------------------------------
// The Graph sub-tab (MOD-14 plan D2-D8, blueprint §3 T3).
// ---------------------------------------------------------------------------------------------

/// Steps right of Body to the Graph sub-tab.
const TO_GRAPH: usize = 2;

/// Rows down from the arrival row to htui `FEAT-2`.
const TO_FEAT_2: usize = 4;

/// `backlog()` with the Backlog named as the tab that reveals items, as `register_all` does: the
/// Graph's `Enter` is an `Action::Reveal`, which reveals nothing without this (blueprint §6 D-1).
async fn revealing_backlog() -> Harness {
    let mut harness = backlog().await;
    harness.app().reveal_tabs = vec![(RevealKind::Item, BacklogTab::ID)];
    harness
}

/// D6: `Enter` on a graph node moves the list cursor to it; the Graph stays the active sub-tab
/// and now shows the new item's neighbourhood.
#[tokio::test]
async fn enter_on_a_graph_node_re_roots_the_backlog() {
    let mut harness = revealing_backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_GRAPH);
    // One `J` from FEAT-1's root row is htui ANA-1 (blueprint §1.3).
    harness.key("J");
    harness.key("enter");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains("┌ ANA-1"), "re-rooted on ANA-1:\n{frame}");
    assert!(
        frame.contains("agy:ANA-1"),
        "only an htui root labels agy ANA-1 so:\n{frame}"
    );
    assert!(
        frame.contains("depth 2/3"),
        "the Graph is still active:\n{frame}"
    );
    assert_eq!(harness.app().status, None);
    insta::assert_snapshot!("graph_re_rooted", frame);

    harness.key("j");
    harness.drive_to_end().await;
    assert!(
        harness.render().contains("┌ ANA-2"),
        "the list cursor was on htui ANA-1, not agy ANA-1"
    );
}

/// D6 through `BacklogTab::reveal`: a node of the workspace's other project is revealed, and its
/// folded group unfolds.
#[tokio::test]
async fn enter_reveals_a_node_of_the_other_project_and_unfolds_its_group() {
    let mut harness = revealing_backlog().await;
    harness.key("G");
    harness.drive_to_end().await;
    for _ in 0..3 {
        harness.key("k");
        harness.drive_to_end().await;
    }
    harness.key("enter");
    harness.drive_to_end().await;
    assert!(
        !harness.render().contains("ACP transport upgrade"),
        "the agy group is folded"
    );

    harness.key("g");
    harness.drive_to_end().await;
    // From the htui header: one row to the arrival row, then on to FEAT-2.
    down(&mut harness, 1 + TO_FEAT_2).await;
    assert!(harness.render().contains("┌ FEAT-2"));
    sub_tab(&mut harness, TO_GRAPH);
    // One `J` from FEAT-2's root row is agy FEAT-1: `agy` < `htui` breaks the key tie.
    harness.key("J");
    harness.key("enter");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains("┌ FEAT-1"), "re-rooted on FEAT-1:\n{frame}");
    assert!(
        frame.contains("htui:FEAT-2"),
        "an agy root labels htui FEAT-2:\n{frame}"
    );
    assert!(frame.contains("▾ agy (3)"), "the group unfolded:\n{frame}");
    assert!(
        frame.contains("ACP transport upgrade"),
        "and shows its rows:\n{frame}"
    );

    harness.key("j");
    harness.drive_to_end().await;
    assert!(
        harness.render().contains("┌ FIX-1"),
        "the list cursor was on agy FEAT-1"
    );
}

/// D7: a node outside the workspace is refused on the status line, and the list stays put.
#[tokio::test]
async fn enter_on_a_node_outside_the_workspace_refuses_on_the_status_line() {
    let mut harness = revealing_backlog().await;
    harness.key("G");
    harness.drive_to_end().await;
    assert!(harness.render().contains("┌ FIX-1"), "agy FIX-1");
    sub_tab(&mut harness, TO_GRAPH);
    // Three `J` from agy FIX-1's root row is vulkan-tutorials FEAT-1 (blueprint §1.4).
    for _ in 0..3 {
        harness.key("J");
    }
    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().status.as_deref(),
        Some("vulkan-tutorials:FEAT-1 is outside this workspace")
    );
    let frame = harness.render();
    assert!(frame.contains("┌ FIX-1"), "the list did not move:\n{frame}");
    insta::assert_snapshot!("graph_outside_workspace", frame);
}

/// Review L7: under the shell's real registration, whose Backlog `Enter` row is the `replay step`
/// miss, `Enter` on the Graph's root row is consumed and nothing reaches the status line. On Body
/// the same key does fall through to the miss, which is what gives the first half its meaning.
#[tokio::test]
async fn enter_on_the_graph_root_row_is_consumed_under_the_real_registration() {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    htui::app::register_all(harness.app());
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    down(&mut harness, TO_FEAT_1).await;
    harness.key("m");
    let frame = harness.render();
    assert!(
        frame.contains("┌ FEAT-1") && frame.contains("▸ FEAT-1 in_progress"),
        "the Graph, its cursor on the root row:\n{frame}"
    );
    assert_eq!(harness.app().status, None);

    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(harness.app().status, None, "the Graph consumed `Enter`");

    sub_tab_back(&mut harness, TO_GRAPH);
    harness.key("enter");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().status.as_deref(),
        Some("select a step in the Runs pane (J/K) to replay it"),
        "from Body, the miss answers"
    );
}

/// Cycles `n` sub-tabs to the left.
fn sub_tab_back(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.key("h");
    }
}

/// D8: `m` lands on the Graph from whichever sub-tab is showing.
#[tokio::test]
async fn m_opens_the_graph_from_any_sub_tab() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    assert!(!harness.render().contains("depth 2/3"), "Body is showing");
    harness.key("m");
    assert!(harness.render().contains("depth 2/3"), "from Body");

    // Back one to Runs.
    harness.key("h");
    assert!(!harness.render().contains("depth 2/3"), "Runs is showing");
    harness.key("m");
    assert!(harness.render().contains("depth 2/3"), "from Runs");
}

/// D2: `+` / `-` change the view depth locally, clamp at 1 and 3, and the depth survives a new
/// selection.
#[tokio::test]
async fn plus_and_minus_change_the_graph_depth() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    harness.key("m");
    harness.key("-");
    let depth_1 = harness.render();
    assert!(depth_1.contains("depth 1/3"), "{depth_1}");
    insta::assert_snapshot!("graph_depth_1", depth_1);

    harness.key("+");
    harness.key("+");
    let depth_3 = harness.render();
    assert!(depth_3.contains("depth 3/3"), "{depth_3}");
    insta::assert_snapshot!("graph_depth_3", depth_3);

    harness.key("+");
    assert!(harness.render().contains("depth 3/3"), "`+` stops at 3");

    harness.key("j");
    harness.drive_to_end().await;
    let feat_2 = harness.render();
    assert!(feat_2.contains("┌ FEAT-2"), "{feat_2}");
    assert!(
        feat_2.contains("depth 3/3"),
        "the depth is a preference, not per item:\n{feat_2}"
    );
}

// ---------------------------------------------------------------------------------------------
// The Runs poll (MOD-41 plan D16, OQ-3, blueprint §12).
// ---------------------------------------------------------------------------------------------

/// Rows down to htui `FEAT-3`, the item whose run (`RUN_2`) is the fixture's only active one.
const TO_FEAT_3: usize = 5;

/// 250 ms ticks between two `Runs` polls: a refresh is every fourth tick and a poll every fifth
/// refresh, so 5 s (D16).
const TICKS_PER_POLL: usize = 20;

/// A settled Backlog tab over `store`, scoped to `Platform`, with no run runtime: nothing but the
/// poll can tell the pane that a run moved, because no process here walks it and so no
/// `RunStream` frame is ever sent (the store worker only acknowledges the subscription).
async fn polled(store: MemStore) -> Harness {
    let mut harness = Harness::over(store)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// `n` 250 ms ticks, each followed by a drive that serves what it asked for.
async fn tick(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.app().update(Action::Tick);
        harness.drive().await;
    }
}

/// Columns left of the detail pane's contents at 100x30: the list's 55 (its `LIST_PERCENT`) and
/// the detail pane's own left border.
const DETAIL_INSIDE: usize = 56;

/// The detail pane's half of a frame: every row from inside the pane's left border, so the list's
/// item statuses (`awaiting_approval` is one of them) are not read as a run's.
fn detail_pane(frame: &str) -> String {
    frame
        .lines()
        .map(|line| line.chars().skip(DETAIL_INSIDE).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The run-grid status cell of the only `graph` run on the pane.
fn run_status(frame: &str) -> Option<String> {
    detail_pane(frame).lines().find_map(|line| {
        let mut cells = line.split_whitespace();
        cells.position(|cell| cell == "graph")?;
        cells.next().map(str::to_owned)
    })
}

/// The demo with `RUN_2`, `FEAT-3`'s queued run, moved on to `running` as a claim would.
async fn running_store() -> MemStore {
    let store = MemStore::demo();
    assert!(
        store
            .transition_run(
                ids::RUN_2,
                RunStatus::Queued,
                RunStatus::Running,
                demo_at(2, 9)
            )
            .await
            .expect("the run exists"),
        "`RUN_2` was queued"
    );
    store
}

/// D16: a run another process walks moves in the store; the pane re-reads it on the fifth
/// refresh, and not before.
#[tokio::test]
async fn the_runs_pane_rereads_an_active_run_every_poll() {
    let store = running_store().await;
    let mut harness = polled(store.clone()).await;
    down(&mut harness, TO_FEAT_3).await;
    sub_tab(&mut harness, 1);
    let frame = harness.render();
    assert_eq!(
        run_status(&frame).as_deref(),
        Some("running"),
        "FEAT-3's run shows running:\n{frame}"
    );

    assert!(
        store
            .transition_run(
                ids::RUN_2,
                RunStatus::Running,
                RunStatus::AwaitingApproval,
                demo_at(2, 10),
            )
            .await
            .expect("the run exists"),
        "the other process parks the run"
    );

    tick(&mut harness, TICKS_PER_POLL - 1).await;
    let frame = harness.render();
    assert_eq!(
        run_status(&frame).as_deref(),
        Some("running"),
        "nineteen ticks are four refreshes: no poll yet:\n{frame}"
    );

    tick(&mut harness, 1).await;
    let frame = harness.render();
    assert_eq!(
        run_status(&frame).as_deref(),
        Some("awaiting"),
        "the twentieth tick is the fifth refresh, and the poll shows the park:\n{frame}"
    );
    assert_eq!(harness.app().status, None, "and nothing failed");
}

/// D16: an item whose every run is terminal is not polled. A step written behind the pane's back
/// stays unseen through two poll periods, and a re-selection (which does read) shows it.
#[tokio::test]
async fn the_runs_pane_does_not_poll_a_finished_item() {
    let store = MemStore::demo();
    let mut harness = polled(store.clone()).await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, 1);
    let frame = harness.render();
    assert_eq!(
        run_status(&frame).as_deref(),
        Some("done"),
        "FEAT-1's only run is done:\n{frame}"
    );

    store
        .create_step(NewRunStep {
            id: StepId::new(),
            run_id: ids::RUN_1,
            position: 4,
            attempt: 1,
            fanout_index: 0,
            phase_name: "unpolled".to_owned(),
            agent_id: Some(ids::AGENT_CLAUDE),
            model: Some("sonnet".to_owned()),
        })
        .await
        .expect("the step lands");

    tick(&mut harness, 2 * TICKS_PER_POLL).await;
    let frame = harness.render();
    assert!(
        !frame.contains("unpolled"),
        "forty ticks sent no `Runs` for an item with no active run:\n{frame}"
    );

    harness.key("k");
    harness.drive().await;
    harness.key("j");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("unpolled"),
        "the step is there for a read to find:\n{frame}"
    );
}

/// D16 with MOD-4 D198: the cursor on the second entry stays on it across a poll that changed the
/// rows.
#[tokio::test]
async fn the_runs_pane_poll_keeps_the_cursor() {
    let store = MemStore::demo();
    store
        .create_step(NewRunStep {
            id: StepId::new(),
            run_id: ids::RUN_2,
            position: 1,
            attempt: 1,
            fanout_index: 0,
            phase_name: "plan".to_owned(),
            agent_id: Some(ids::AGENT_CLAUDE),
            model: Some("sonnet".to_owned()),
        })
        .await
        .expect("the second step lands");
    let mut harness = polled(store.clone()).await;
    down(&mut harness, TO_FEAT_3).await;
    sub_tab(&mut harness, 1);
    harness.key("J");
    let cursor_on = |frame: &str| {
        detail_pane(frame)
            .lines()
            .find(|line| line.contains('\u{25b8}'))
            .map(|line| line.contains("plan"))
    };
    let frame = harness.render();
    assert_eq!(
        (run_status(&frame).as_deref(), cursor_on(&frame)),
        (Some("queued"), Some(true)),
        "`J` put the cursor on the second step, `plan`:\n{frame}"
    );

    assert!(
        store
            .transition_run(
                ids::RUN_2,
                RunStatus::Queued,
                RunStatus::Running,
                demo_at(2, 9)
            )
            .await
            .expect("the run exists"),
        "a worker claims the run"
    );
    tick(&mut harness, TICKS_PER_POLL).await;
    let frame = harness.render();
    assert_eq!(
        (run_status(&frame).as_deref(), cursor_on(&frame)),
        (Some("running"), Some(true)),
        "the poll re-read the run and the cursor stayed on `plan`:\n{frame}"
    );
}
