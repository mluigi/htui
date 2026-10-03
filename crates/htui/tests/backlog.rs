//! Backlog tab tests (blueprint §E, T4).
//!
//! Everything runs against `Harness::demo()` and the tab's own public surface, so no T3 or T5
//! file is touched and the T6 registration does not have to exist yet. Frames are 100x30, the
//! size the whole snapshot suite is pinned to (plan risk row).
#![cfg(feature = "testkit")]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use crossterm::event::{MouseButton, MouseEventKind};
use htui::agent_worker::AgentRuntime;
use htui::app::{Action, RevealKind};
use htui::editor::{ExternalEdit, ExternalEditOutcome};
use htui::keymap::KeyScope;
use htui::requirements::decision_citation_stays;
use htui::run_worker::{self, LiveChats, RunRuntime, StepAuthor};
use htui::store_worker::StoreRequest;
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::event::{DoneEvent, DriverEvent, StopReason};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::{demo_at, edit_agent, ids};
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, CitationKind, Claim, DocumentHead, DocumentId,
    EXECUTOR_GONE, GraphSnapshot, Isolation, Item, ItemFilter, ItemId, ItemPatch, NewDocument,
    NewItem, NewRun, NewRunStep, Note, OpenPermission, PermissionId, PermissionStatus, RelayOption,
    RelayOptionKind, RelaySessionId, Resolution, RunId, RunMode, RunStatus, RunStep, Scope,
    SnapshotGraph, SnapshotPhase, SnapshotSettings, Status, StepId, Transport, WorkspaceSummary,
};
use htui_core::store::{MAX_LEASE_TTL, MemStore, ReadStore as _, UpdateOutcome, WriteStore as _};
use htui_orch::Clock;
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};
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
// The Runs pane's flow view (MOD-28 plan D1-D15).
// ---------------------------------------------------------------------------------------------

/// MOD-28 D1, D10: `v` on `ANA-1`'s fan-out run draws its two candidates side by side, the winner
/// checked and the loser superseded; a second `v` is back on the list.
#[tokio::test]
async fn v_draws_the_fan_out_run_as_a_flow() {
    let mut harness = backlog().await;
    sub_tab(&mut harness, 1);
    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("0.1/0 done \u{2713}") && frame.contains("0.1/1 superseded"),
        "both candidates are drawn:\n{frame}"
    );
    assert!(!frame.contains("kind   status"), "no list header:\n{frame}");
    insta::assert_snapshot!("runs_flow_fanout", frame);

    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("kind   status"),
        "the list is back:\n{frame}"
    );
}

/// MOD-28 D11: a modal footer is drawn under the flow exactly as under the list.
#[tokio::test]
async fn the_reject_note_renders_under_the_flow() {
    let mut harness = parked().await;
    harness.key("v");
    harness.key("x");
    type_text(&mut harness, "needs work");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("reject with a note:") && frame.contains("research"),
        "the note is under the parked step's node:\n{frame}"
    );
    insta::assert_snapshot!("runs_flow_reject_note", frame);
}

/// The smallest snapshot `create_run` accepts (`htui-agent/tests/relay.rs`).
fn bare_snapshot() -> GraphSnapshot {
    GraphSnapshot {
        v: GraphSnapshot::V,
        graph: SnapshotGraph {
            id: ids::GRAPH_HTUI_FEAT,
            name: "feature".to_owned(),
            is_override: false,
        },
        topology: "sha256:flow".to_owned(),
        mode: RunMode::Manual,
        phases: Vec::new(),
        settings: SnapshotSettings {
            default_isolation: Isolation::Worktree,
            per_token_cap_run: None,
            per_token_cap_batch: None,
            max_fan_out: 4,
            max_agents_per_run: 8,
        },
        scope: None,
        personas: Vec::new(),
    }
}

/// MOD-28 D5: a queued run with no step yet shows its run line and says so.
#[tokio::test]
async fn a_run_with_no_steps_says_so_in_the_flow() {
    let store = MemStore::demo();
    store
        .create_run(NewRun {
            id: RunId::new(),
            project_id: ids::PROJECT_HTUI,
            item_id: ids::HTUI_ANA_2,
            mode: RunMode::Manual,
            target_box_id: ids::BOX,
            started_by: ids::USER,
            graph_snapshot: bare_snapshot(),
            repo_scope: Vec::new(),
            queued_at: demo_at(2, 8),
        })
        .await
        .expect("the run is queued");
    // No run runtime: nothing claims the run, so it never gets a step.
    let mut harness = polled(store).await;
    down(&mut harness, TO_ANA_2).await;
    sub_tab(&mut harness, 1);
    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("No steps yet.") && frame.contains("queued"),
        "the run line, then the empty canvas:\n{frame}"
    );
}

/// MOD-28 D7, ANA-12 invariant 2: an action key in the flow is answered as in the list, through
/// the real worker (blueprint E14).
#[tokio::test]
async fn an_action_key_in_flow_answers_as_in_the_list() {
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

    harness.key("v");
    harness.key("a");
    harness.drive().await;
    assert_eq!(harness.app().status.as_deref(), Some(sentence.as_str()));
}

/// MOD-72 D1, D5, D6: `FEAT-1`'s plan step recorded the fixture's one tool call, a `read`, so its
/// node's third line is `⚒ read×1`; no other node draws a chip.
#[tokio::test]
async fn the_flow_draws_the_plan_step_s_tool_call_as_a_chip() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, 1);
    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{2692} read\u{d7}1"),
        "the plan step's chip:\n{frame}"
    );
    assert_eq!(
        frame.matches('\u{2692}').count(),
        1,
        "only the plan step made a call:\n{frame}"
    );
    insta::assert_snapshot!("runs_flow_tool_chips", frame);
}

/// The cell `needle` starts at in a frame, `(column, row)`. Every glyph in a Backlog frame is one
/// cell wide (box drawing, `…`, `✓`, `·`), so a char count is a column.
fn cell_of(frame: &str, needle: &str) -> (u16, u16) {
    frame
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let byte = line.find(needle)?;
            Some((
                u16::try_from(line[..byte].chars().count()).expect("the column fits"),
                u16::try_from(row).expect("the row fits"),
            ))
        })
        .unwrap_or_else(|| panic!("`{needle}` is drawn:\n{frame}"))
}

/// The list row of the slot `slot` in a frame: the line that names it.
fn listed<'a>(frame: &'a str, slot: &str) -> &'a str {
    frame
        .lines()
        .find(|line| line.contains(slot))
        .unwrap_or_else(|| panic!("`{slot}` is listed:\n{frame}"))
}

/// MOD-71 D1, D6, ANA-12 invariant 2: in the flow, a click on `ANA-1`'s losing candidate moves the
/// shared cursor to it, so `a` answers for that step, and the list shows the cursor there.
#[tokio::test]
async fn a_click_in_the_flow_moves_the_cursor_an_action_key_reads() {
    let mut harness = backlog().await;
    sub_tab(&mut harness, 1);
    assert!(
        !harness.app().wants_mouse(),
        "the list keeps the terminal's selection"
    );
    harness.key("v");
    harness.drive_to_end().await;
    assert!(harness.app().wants_mouse(), "the flow wants the mouse");
    let (column, row) = cell_of(&harness.render(), "0.1/1 superseded");
    harness.mouse(MouseEventKind::Down(MouseButton::Left), column, row);
    harness.mouse(MouseEventKind::Up(MouseButton::Left), column, row);

    let verdicts = run_worker::actions(
        &Backend::memory(MemStore::demo()),
        ids::HTUI_ANA_1,
        &LiveChats::default(),
    )
    .await
    .expect("the verdicts read");
    let sentence = verdicts.steps[&ids::STEP_R3_RESEARCH_B]
        .approve
        .clone()
        .expect_err("a step of a finished run cannot be approved");
    harness.key("a");
    harness.drive().await;
    assert_eq!(harness.app().status.as_deref(), Some(sentence.as_str()));

    harness.key("v");
    harness.drive_to_end().await;
    assert!(!harness.app().wants_mouse());
    let frame = harness.render();
    assert!(
        listed(&frame, "0.1/1").contains('\u{25b8}'),
        "the list's cursor is on the clicked step:\n{frame}"
    );
}

/// MOD-71 D5, D9: a drag on empty canvas moves the drawn nodes by the drag, and the wheel redraws
/// them at another zoom; neither moves the cursor.
#[tokio::test]
async fn a_drag_pans_and_the_wheel_zooms_the_flow() {
    let mut harness = backlog().await;
    sub_tab(&mut harness, 1);
    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    let (column, row) = cell_of(&frame, "0.1/0 done");
    let below = frame
        .lines()
        .nth(usize::from(row + 8))
        .and_then(|line| line.chars().nth(usize::from(column)));
    assert_eq!(below, Some(' '), "a blank canvas cell:\n{frame}");

    harness.mouse(MouseEventKind::Down(MouseButton::Left), column, row + 8);
    harness.mouse(MouseEventKind::Drag(MouseButton::Left), column + 2, row + 9);
    harness.mouse(MouseEventKind::Up(MouseButton::Left), column + 2, row + 9);
    let panned = harness.render();
    assert_eq!(
        cell_of(&panned, "0.1/0 done"),
        (column + 2, row + 1),
        "the nodes moved by the drag:\n{panned}"
    );

    harness.mouse(MouseEventKind::ScrollDown, column + 2, row + 9);
    let zoomed = harness.render();
    assert_ne!(zoomed, panned, "the wheel redrew the flow");

    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        listed(&frame, "0.1/0").contains('\u{25b8}'),
        "the cursor stayed on the first candidate:\n{frame}"
    );
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

// ---------------------------------------------------------------------------------------------
// Filters (MOD-13 milestone 1, plan D1-D6, blueprint §4).
// ---------------------------------------------------------------------------------------------

/// [`backlog`] over `store` instead of the plain demo.
async fn backlog_over(store: MemStore) -> Harness {
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

/// Presses each of `keys`, then serves what they asked for.
async fn keys(harness: &mut Harness, keys: &[&str]) {
    for key in keys {
        harness.key(key);
    }
    harness.drive_to_end().await;
}

/// `f`, then `l` five times to `done` on the Status row, ticked with `space`.
const TICK_DONE: [&str; 7] = ["f", "l", "l", "l", "l", "l", "space"];

/// From the Status row, down to Tags and type `rust`.
const TYPE_RUST: [&str; 6] = ["down", "down", "r", "u", "s", "t"];

/// The list pane's title row: the frame line holding `Backlog (`.
fn list_title(frame: &str) -> &str {
    frame
        .lines()
        .find(|line| line.contains("Backlog ("))
        .expect("the list pane has a title")
}

/// D1, D2: a status filter reads only that status.
#[tokio::test]
async fn filtering_by_status_done_lists_only_done_items() {
    let mut harness = backlog().await;
    keys(&mut harness, &TICK_DONE).await;
    keys(&mut harness, &["enter"]).await;
    let frame = harness.render();
    assert!(
        list_title(&frame).contains("Backlog (2) · status:done "),
        "{frame}"
    );
    assert!(frame.contains("Data model, box registry"), "{frame}");
    assert!(frame.contains("Prompt assembly survey"), "{frame}");
    assert!(!frame.contains("TUI scaffold"), "{frame}");
    assert_eq!(harness.app().status, None);
}

/// D1: a project filter reads only that project's items.
#[tokio::test]
async fn filtering_by_one_project_lists_only_its_items() {
    let mut harness = backlog().await;
    keys(&mut harness, &["f", "j", "space", "enter"]).await;
    let frame = harness.render();
    assert!(
        list_title(&frame).contains("Backlog (8) · project:htui "),
        "{frame}"
    );
    assert!(!frame.contains("ACP transport upgrade"), "{frame}");
}

/// D3: the capability filter keeps the items whose required tags hold `rust`.
#[tokio::test]
async fn filtering_by_the_rust_tag_lists_only_rust_items() {
    let mut harness = backlog().await;
    keys(&mut harness, &["f"]).await;
    keys(&mut harness, &TYPE_RUST).await;
    keys(&mut harness, &["enter"]).await;
    let frame = harness.render();
    assert!(
        list_title(&frame).contains("Backlog (4) · tags:rust "),
        "htui FEAT-1/2/3 and agy FEAT-1:\n{frame}"
    );
    assert!(
        frame.contains("ACP transport upgrade"),
        "agy FEAT-1:\n{frame}"
    );
    assert!(
        !frame.contains("Data model, box registry"),
        "untagged htui ANA-1 is gone:\n{frame}"
    );
}

/// D2: "ready here" is `MemStore::ready_items` for this box. The demo plus an open item needing
/// `cuda`, which the store-side half alone would keep (blueprint E1).
#[tokio::test]
async fn ready_here_lists_what_this_box_can_start() {
    let store = MemStore::demo();
    store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            title: "needs a GPU toolchain".to_owned(),
            body: String::new(),
            required_tags: vec!["cuda".to_owned()],
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("the mint lands");
    let platform = Scope::from_workspace(&workspace("platform").await);
    let ready = store
        .ready_items(&platform, ids::BOX)
        .await
        .expect("the read is total");
    assert_eq!(ready.len(), 3, "htui ANA-2, agy FEAT-1 and agy FIX-1");

    let mut harness = backlog_over(store).await;
    assert!(
        harness.render().contains("needs a GPU toolchain"),
        "unfiltered, the cuda item is listed"
    );
    keys(
        &mut harness,
        &["f", "down", "down", "down", "space", "enter"],
    )
    .await;
    let frame = harness.render();
    assert!(
        list_title(&frame).contains(&format!("Backlog ({}) · ready here ", ready.len())),
        "{frame}"
    );
    for item in &ready {
        let head: String = item.title.chars().take(20).collect();
        assert!(frame.contains(&head), "{} is listed:\n{frame}", item.key);
    }
    assert!(
        !frame.contains("needs a GPU toolchain"),
        "this box has no `cuda`:\n{frame}"
    );
    assert_eq!(harness.app().status, None);
}

/// D5: `F` restores the whole list and its plain title.
#[tokio::test]
async fn shift_f_restores_the_whole_list() {
    let mut harness = backlog().await;
    keys(&mut harness, &TICK_DONE).await;
    keys(&mut harness, &["enter"]).await;
    assert!(list_title(&harness.render()).contains("Backlog (2) · "));
    keys(&mut harness, &["F"]).await;
    let frame = harness.render();
    let title = list_title(&frame);
    assert!(title.contains("Backlog (11) "), "{frame}");
    assert!(
        !title.contains('·'),
        "no summary without a filter:\n{frame}"
    );
    assert!(frame.contains("TUI scaffold"), "{frame}");
}

/// Blueprint E4: `f` and `F` are on the Backlog's help line, next to `m`.
#[tokio::test]
async fn f_and_shift_f_are_on_the_backlog_help_line() {
    let mut harness = Harness::demo();
    htui::app::register_all(harness.app());
    let help = harness
        .app()
        .keymap
        .help_line(&KeyScope::Tab(BacklogTab::ID));
    assert!(help.contains("f filter"), "{help}");
    assert!(help.contains("F clear filter"), "{help}");
}

/// D1: the open form at the bottom of the list pane, `done` ticked and `rust` typed, nothing
/// applied yet: the title is still the whole list's.
#[tokio::test]
async fn the_filter_form_renders_in_the_list_pane() {
    let mut harness = backlog().await;
    keys(&mut harness, &TICK_DONE).await;
    keys(&mut harness, &TYPE_RUST).await;
    let frame = harness.render();
    assert!(list_title(&frame).contains("Backlog (11) "), "{frame}");
    assert!(frame.contains("Filter"), "{frame}");
    assert!(frame.contains("[x] done"), "{frame}");
    assert!(frame.contains("> tags    rust"), "{frame}");
    insta::assert_snapshot!("filter_form", frame);
}

/// D5: a filtered list names its filter in the title; the cursor lands on its first item.
#[tokio::test]
async fn the_filtered_list_names_its_filter() {
    let mut harness = backlog().await;
    keys(&mut harness, &TICK_DONE).await;
    keys(&mut harness, &["enter"]).await;
    let frame = harness.render();
    assert!(frame.contains("┌ Backlog (2) · status:done"), "{frame}");
    assert!(frame.contains("┌ ANA-1"), "{frame}");
    insta::assert_snapshot!("filtered_list", frame);
}

/// D5, blueprint E3: a filter nothing matches says so, even though the project headers remain.
#[tokio::test]
async fn a_filter_nothing_matches_says_so() {
    let mut harness = backlog().await;
    keys(&mut harness, &TICK_DONE).await;
    keys(&mut harness, &TYPE_RUST).await;
    keys(&mut harness, &["enter"]).await;
    let frame = harness.render();
    assert!(frame.contains("No items match the filter."), "{frame}");
    insta::assert_snapshot!("filter_no_match", frame);
}

// MOD-42 plan D14: answering a relayed permission request from the Runs pane.
// ---------------------------------------------------------------------------------------------

/// The demo with `FEAT-3`'s queued run claimed on the demo box by `owner` for
/// [`MAX_LEASE_TTL`], and one request parked on its `prd` step, as a walk on another process
/// would have opened it.
async fn relayed_store(owner: uuid::Uuid) -> (MemStore, PermissionId) {
    let store = MemStore::demo();
    assert_eq!(
        store
            .claim_run(ids::RUN_2, ids::BOX, owner, demo_at(2, 9), MAX_LEASE_TTL)
            .await
            .expect("the claim reads"),
        Claim::Admitted,
        "`RUN_2` was queued on the demo box"
    );
    let id = store
        .open_permission(OpenPermission {
            id: PermissionId::new(),
            run_id: ids::RUN_2,
            run_step_id: ids::STEP_R2_PRD,
            session: RelaySessionId::new(),
            request_id: "req-1".to_owned(),
            tool_call_id: Some("call-1".to_owned()),
            summary: Some("execute: cargo test".to_owned()),
            options: vec![
                RelayOption {
                    id: "allow".to_owned(),
                    label: "Allow once".to_owned(),
                    kind: RelayOptionKind::AllowOnce,
                },
                RelayOption {
                    id: "reject".to_owned(),
                    label: "Reject once".to_owned(),
                    kind: RelayOptionKind::RejectOnce,
                },
            ],
            owner,
        })
        .await
        .expect("the owner parks the request");
    (store, id)
}

/// The Backlog over `store`, on `FEAT-3`'s Runs pane.
async fn on_feat_3_runs(store: MemStore) -> Harness {
    let mut harness = polled(store).await;
    down(&mut harness, TO_FEAT_3).await;
    sub_tab(&mut harness, 1);
    harness.drive().await;
    harness
}

/// D14: the step's pending request is drawn under it: the scrubbed summary and the strip.
#[tokio::test]
async fn runs_pane_shows_a_pending_permission() {
    let (store, _) = relayed_store(uuid::Uuid::new_v4()).await;
    let mut harness = on_feat_3_runs(store).await;
    let frame = harness.render();
    let pane = detail_pane(&frame);
    assert!(
        pane.contains("asks: execute: cargo test"),
        "the summary is shown:\n{frame}"
    );
    assert!(
        pane.contains("[1] Allow once  [2] Reject once"),
        "and the strip:\n{frame}"
    );
    assert_eq!(harness.app().status, None);
    insta::assert_snapshot!("runs_pending_permission", frame);
}

/// D3, D14: a digit answers through the store, and the re-read drops the answered request.
#[tokio::test]
async fn a_digit_on_the_runs_pane_answers_through_the_store() {
    let (store, id) = relayed_store(uuid::Uuid::new_v4()).await;
    let mut harness = on_feat_3_runs(store.clone()).await;
    assert!(harness.render().contains("[1] Allow once"));

    harness.key("1");
    harness.drive().await;
    let rows = store.relay_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].status, PermissionStatus::Answered);
    assert_eq!(rows[0].option_id.as_deref(), Some("allow"));
    assert_eq!(rows[0].answered_by, Some(ids::USER));
    assert_eq!(rows[0].answered_box, Some(ids::BOX));
    assert_eq!(harness.app().status, None, "nothing failed");
    let frame = harness.render();
    assert!(
        !frame.contains("[1] Allow once") && !frame.contains("asks:"),
        "the re-read dropped the strip:\n{frame}"
    );
}

/// D3, B-13: an answer the store refuses puts the refusal's sentence on the status line.
#[tokio::test]
async fn a_refused_answer_lands_on_the_status_line() {
    let owner = uuid::Uuid::new_v4();
    let (store, _) = relayed_store(owner).await;
    let mut harness = on_feat_3_runs(store.clone()).await;
    assert!(
        harness.render().contains("[1] Allow once"),
        "the strip is up"
    );

    assert!(
        store
            .release_lease(ids::RUN_2, owner)
            .await
            .expect("the run exists"),
        "the executor lets go of the run"
    );
    harness.key("1");
    harness.drive().await;
    assert_eq!(
        harness.app().status,
        Some(format!("answer_permission: {EXECUTOR_GONE}")),
        "the refusal's own sentence, under the request's name"
    );
    assert_eq!(store.relay_rows()[0].status, PermissionStatus::Pending);
}

/// D14, OQ-4: offline the relay read is an empty view, so the Runs pane's every refresh raises
/// nothing; an answer, the one write, is refused onto the status line.
///
/// The mirror holds no item (`seed_mirror` writes the hierarchy and the registry only), so the
/// read is dispatched as the pane sends it, the `testkit.rs` pattern for the Runs pane's reads.
#[tokio::test]
async fn offline_the_runs_pane_asks_for_no_error() {
    // `App::start` issues `ConnectionInfo`, which over a non-`Memory` backend reaches the OS
    // keyring without this guard.
    let _keyring = htui_store::testkit::mock_keyring().await;
    // The mirror outlives the harness: dropping the directory deletes it mid-test.
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "runs-relay-offline", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    htui_store::testkit::seed_mirror(&cache, &htui_core::fixtures::demo_data())
        .await
        .expect("the mirror is seeded");
    let mut harness = Harness::over_backend(Backend::Offline {
        cache,
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(BacklogTab::new()))
    // `offline · 3s` would age between the render and the next tick.
    .with_store_state("offline \u{b7} 0s", None);
    harness.drive().await;
    assert_eq!(harness.app().status, None, "the shell started clean");

    harness.app().update(Action::Store(StoreRequest::RelayView {
        item: ids::HTUI_FEAT_3,
    }));
    harness.drive().await;
    assert_eq!(
        harness.app().status,
        None,
        "the empty relay view raised nothing"
    );

    harness
        .app()
        .update(Action::Store(StoreRequest::AnswerPermission {
            permission: PermissionId::new(),
            option_id: "allow".to_owned(),
        }));
    harness.drive().await;
    let status = harness.app().status.clone().unwrap_or_default();
    assert!(
        status.starts_with("answer_permission: ") && status.ends_with(DATABASE_UNREACHABLE),
        "an answer offline is refused before anything is sent: {status:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// New and edit (MOD-13 milestone 2, plan D1-D11, blueprint §5).
// ---------------------------------------------------------------------------------------------

/// From the item form's Title, `Tab` four times to Paths: Priority, Tags, Graph, Paths.
const TITLE_TO_PATHS: [&str; 4] = ["tab", "tab", "tab", "tab"];

/// The arrival row's title, the one `e` opens on.
const ANA_1_TITLE: &str = "Data model, box registry and sync topology";

/// A title-only patch, as another process would write it.
fn retitled(title: &str) -> ItemPatch {
    ItemPatch {
        title: Some(title.to_owned()),
        author_id: ids::USER,
        box_id: Some(ids::BOX),
        reason: "edited".to_owned(),
        ..ItemPatch::default()
    }
}

/// D1, A5: `N`, a title and `Ctrl+S` mint the item; the list re-reads, selects it and the Body
/// shows it. List selection is style-only, so the detail pane is what proves the selection.
#[tokio::test]
async fn n_mints_an_item_and_reveals_it() {
    let mut harness = backlog().await;
    keys(&mut harness, &["N"]).await;
    type_text(&mut harness, "Fresh item");
    keys(&mut harness, &["ctrl-s"]).await;
    let detail = detail_pane(&harness.render());
    assert!(detail.contains("ANA-3"), "the minted key:\n{detail}");
    assert!(detail.contains("Fresh item"), "its title:\n{detail}");
    assert!(
        !detail.contains("New item"),
        "the form closed on the write:\n{detail}"
    );
    assert_eq!(harness.app().status, None, "a mint raises nothing");
}

/// D5, D9 (blueprint E3): `e`, a changed title and `Ctrl+S` land version 2, and the Body shows the
/// re-read head. The new title is 45 characters, so the Body wraps it inside the 43-column pane.
#[tokio::test]
async fn e_edits_the_title_and_the_body_shows_version_2() {
    let store = MemStore::demo();
    let mut harness = backlog_over(store.clone()).await;
    keys(&mut harness, &["e"]).await;
    type_text(&mut harness, " v2");
    keys(&mut harness, &["ctrl-s"]).await;
    let head = store
        .item(ids::HTUI_ANA_1)
        .await
        .expect("the memory store never fails")
        .expect("the demo item exists");
    assert_eq!(head.title, format!("{ANA_1_TITLE} v2"));
    assert_eq!(head.version, 2);
    let detail = detail_pane(&harness.render());
    // The pane's rows, border and padding dropped, joined: the wrapped title reads whole again.
    let text = detail
        .lines()
        .map(|line| line.trim_end_matches('\u{2502}').trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains(&format!("{ANA_1_TITLE} v2")),
        "the new title:\n{detail}"
    );
    assert!(detail.contains("version 2"), "the new version:\n{detail}");
    assert!(
        !detail.contains(" Edit "),
        "the form closed on the write:\n{detail}"
    );
    assert_eq!(harness.app().status, None, "an edit raises nothing");
}

/// D4: a touched path naming no repo of the project is refused by the form, by name, before
/// anything is sent; nothing is written and the status line stays clean.
#[tokio::test]
async fn an_unknown_repo_in_touched_paths_is_refused_by_name() {
    let store = MemStore::demo();
    let before = store.item_count();
    let mut harness = backlog_over(store.clone()).await;
    keys(&mut harness, &["N"]).await;
    type_text(&mut harness, "Fresh item");
    keys(&mut harness, &TITLE_TO_PATHS).await;
    type_text(&mut harness, "nope:src");
    keys(&mut harness, &["ctrl-s"]).await;
    let frame = harness.render();
    assert!(frame.contains("`nope`"), "the refusal names it:\n{frame}");
    assert!(frame.contains("New item"), "the form stays open:\n{frame}");
    assert_eq!(store.item_count(), before, "nothing was written");
    assert_eq!(harness.app().status, None, "a form-side refusal");
}

/// A11: the demo's items in the mirror's `item` table, with its encodings (TEXT uuids, JSON
/// arrays, microsecond stamps, `0001_mirror.sql`'s `item` plus `0004`'s `resolution`).
async fn seed_mirror_items(cache: &CacheStore, items: &[Item]) {
    for item in items {
        sqlx::query(
            "INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, key, title, body, \
                               status, priority, required_tags, touched_paths, step_graph_id, \
                               version, created_by, created_at, updated_at, closed_at, resolution) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item.id.to_string())
        .bind(item.project_id.to_string())
        .bind(item.kind_id.to_string())
        .bind(&item.key_prefix)
        .bind(i64::from(item.key_number))
        .bind(&item.key)
        .bind(&item.title)
        .bind(&item.body)
        .bind(item.status.as_str())
        .bind(i64::from(item.priority))
        .bind(serde_json::to_string(&item.required_tags).expect("tags serialise"))
        .bind(serde_json::to_string(&item.touched_paths).expect("paths serialise"))
        .bind(item.step_graph_id.map(|id| id.to_string()))
        .bind(i64::from(item.version))
        .bind(item.created_by.to_string())
        .bind(item.created_at.timestamp_micros())
        .bind(item.updated_at.timestamp_micros())
        .bind(item.closed_at.map(|at| at.timestamp_micros()))
        .bind(item.resolution.map(Resolution::as_str))
        .execute(cache.pool())
        .await
        .expect("a mirror item row");
    }
}

/// A Backlog scoped to `Platform` over an offline mirror under `root`, seeded with the demo and
/// its items. The caller holds the keyring guard and the temp dir.
async fn offline_backlog(root: &std::path::Path) -> (Harness, CacheStore) {
    let cache = CacheStore::open(root, "backlog-items-offline", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let demo = htui_core::fixtures::demo_data();
    htui_store::testkit::seed_mirror(&cache, &demo)
        .await
        .expect("the mirror is seeded");
    seed_mirror_items(&cache, &demo.items).await;
    let mut harness = Harness::over_backend(Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(BacklogTab::new()))
    .with_store_state("offline \u{b7} 0s", None);
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    (harness, cache)
}

/// D2, A11 (blueprint E11): offline, `N` and `e` are refused by the worker before anything is
/// read. The status line carries the read-only sentence under `item_form`, no form opens, and
/// the mirror is untouched.
///
/// With item rows in the mirror the first `Items` reply selects `ANA-1` and sends its detail
/// reads, so the status line is not clean on arrival; it is cleared before each key instead.
#[tokio::test]
async fn offline_n_and_e_are_refused_with_the_read_only_notice() {
    let _keyring = htui_store::testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("a throwaway config root");
    let (mut harness, cache) = offline_backlog(root.path()).await;
    assert!(
        harness.render().contains("┌ ANA-1"),
        "the mirror's items are listed and the first is selected"
    );
    let refused = format!("item_form: store unreachable: {DATABASE_UNREACHABLE}");

    harness.app().status = None;
    keys(&mut harness, &["N"]).await;
    assert_eq!(harness.app().status, Some(refused.clone()), "`N` offline");
    let frame = harness.render();
    assert!(!frame.contains("New item"), "no form opened:\n{frame}");

    harness.app().status = None;
    keys(&mut harness, &["e"]).await;
    assert_eq!(harness.app().status, Some(refused), "`e` offline");
    let frame = harness.render();
    assert!(!frame.contains(" Edit "), "no form opened:\n{frame}");

    let ana_1 = cache
        .item(ids::HTUI_ANA_1)
        .await
        .expect("the mirror answers")
        .expect("the seeded item");
    assert_eq!(ana_1.version, 1, "nothing was written");
    let platform = Scope::from_workspace(&workspace("platform").await);
    let items = cache
        .items(&platform, &ItemFilter::default())
        .await
        .expect("the mirror answers");
    assert_eq!(items.len(), 11, "nothing was minted");
}

/// D8: `N` and `e` are on the Backlog's help line.
#[tokio::test]
async fn n_and_e_are_on_the_backlog_help_line() {
    let mut harness = Harness::demo();
    htui::app::register_all(harness.app());
    let help = harness
        .app()
        .keymap
        .help_line(&KeyScope::Tab(BacklogTab::ID));
    assert!(help.contains("N new item"), "{help}");
    assert!(help.contains("e edit item"), "{help}");
}

/// D8: the new form in the detail pane, a title typed: the project picker, the hinted kind, the
/// kind-default graph.
#[tokio::test]
async fn the_new_item_form_renders_in_the_detail_pane() {
    let mut harness = backlog().await;
    keys(&mut harness, &["N"]).await;
    type_text(&mut harness, "Fresh item");
    let frame = harness.render();
    assert!(frame.contains(" New item \u{b7} htui "), "{frame}");
    assert!(frame.contains("> title"), "the focus is on Title:\n{frame}");
    assert!(frame.contains("\u{2039}ANA "), "the hinted kind:\n{frame}");
    assert!(frame.contains("kind default"), "{frame}");
    insta::assert_snapshot!("item_form_new", frame);
}

/// D8: the edit form opens on the item at its version, with no project picker.
#[tokio::test]
async fn the_edit_item_form_renders_in_the_detail_pane() {
    let mut harness = backlog().await;
    keys(&mut harness, &["e"]).await;
    let frame = harness.render();
    assert!(frame.contains(" Edit ANA-1 (v1) "), "{frame}");
    assert!(
        !frame.contains("project  \u{2039}"),
        "an edit has no project row:\n{frame}"
    );
    insta::assert_snapshot!("item_form_edit", frame);
}

// ---------------------------------------------------------------------------------------------
// Divergence (MOD-13 milestone 3, plan D1-D9).
// ---------------------------------------------------------------------------------------------

/// From the item form's Title, `Tab` five times to Body: Priority, Tags, Graph, Paths, Body.
const TITLE_TO_BODY: [&str; 5] = ["tab"; 5];

/// `ANA-1`'s head as `(title, version)`.
async fn ana_1_head(store: &MemStore) -> (String, i32) {
    let head = store
        .item(ids::HTUI_ANA_1)
        .await
        .expect("the memory store never fails")
        .expect("the demo item exists");
    (head.title, head.version)
}

/// `patch` written at version 1 through a held clone, as another process would: the head moves
/// to version 2 under the open form.
async fn their_write(store: &MemStore, patch: ItemPatch) {
    let outcome = store
        .update_item(ids::HTUI_ANA_1, 1, patch)
        .await
        .expect("the memory store never fails");
    assert!(
        matches!(outcome, UpdateOutcome::Updated(_)),
        "the other write is at the head: {outcome:?}"
    );
}

/// The state the divergence view gives `field`, read off that field's own row: the block title,
/// the column headers and the hint line name `theirs` and `mine` whatever the rows say.
fn divergence_state<'a>(frame: &'a str, field: &str) -> &'a str {
    frame
        .lines()
        .find_map(|line| {
            let mut words = line.trim_start_matches('\u{2502}').split_whitespace();
            if words.next() == Some(field) {
                words.next()
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("a {field:?} row in the view:\n{frame}"))
}

/// `e`, their write, ` mine` typed after the title and `Ctrl+S`: the stale save opens the view.
async fn diverged_on(store: &MemStore, theirs: ItemPatch) -> Harness {
    let mut harness = backlog_over(store.clone()).await;
    keys(&mut harness, &["e"]).await;
    their_write(store, theirs).await;
    type_text(&mut harness, " mine");
    keys(&mut harness, &["ctrl-s"]).await;
    harness
}

/// D1, D3-D5, D9 (rewrites milestone 2's stale-edit case): a stale `Ctrl+S` opens the three-way
/// view and writes nothing; `m` rebases the form on the head with my title; `Ctrl+S` then lands
/// it at version 3 and the Body shows the re-read head (E4: `version 3`).
#[tokio::test]
async fn a_stale_edit_opens_the_view_and_m_lands_mine_at_version_3() {
    let store = MemStore::demo();
    let mut harness = diverged_on(&store, retitled("Theirs")).await;
    let frame = harness.render();
    for wanted in [
        "ANA-1  v1 \u{2192} theirs v2",
        "Theirs",
        "sync topology mine",
    ] {
        assert!(frame.contains(wanted), "{wanted:?} in the view:\n{frame}");
    }
    assert_eq!(
        divergence_state(&frame, "title"),
        "conflict",
        "both sides retitled:\n{frame}"
    );
    assert_eq!(
        ana_1_head(&store).await,
        ("Theirs".to_owned(), 2),
        "the stale save wrote nothing"
    );

    keys(&mut harness, &["m"]).await;
    let frame = harness.render();
    assert!(
        frame.contains(" Edit ANA-1 (v2) "),
        "the form is rebased on the head:\n{frame}"
    );
    assert!(frame.contains("rebased on v2"), "and says so:\n{frame}");

    keys(&mut harness, &["ctrl-s"]).await;
    assert_eq!(
        ana_1_head(&store).await,
        (format!("{ANA_1_TITLE} mine"), 3),
        "mine lands on top of theirs"
    );
    let detail = detail_pane(&harness.render());
    assert!(detail.contains("version 3"), "the new version:\n{detail}");
    assert!(
        !detail.contains(" Edit "),
        "the form closed on the write:\n{detail}"
    );
    assert_eq!(harness.app().status, None, "a resolution raises nothing");
}

/// D2 end to end: their priority and my title do not conflict, so `m` keeps both.
#[tokio::test]
async fn a_resolution_keeps_their_priority_and_my_title() {
    let store = MemStore::demo();
    let theirs = ItemPatch {
        priority: Some(9),
        author_id: ids::USER,
        box_id: Some(ids::BOX),
        reason: "edited".to_owned(),
        ..ItemPatch::default()
    };
    let mut harness = diverged_on(&store, theirs).await;
    let frame = harness.render();
    assert_eq!(
        divergence_state(&frame, "priority"),
        "theirs",
        "only they moved the priority:\n{frame}"
    );
    assert_eq!(
        divergence_state(&frame, "title"),
        "mine",
        "only I moved the title:\n{frame}"
    );

    keys(&mut harness, &["m", "ctrl-s"]).await;
    let head = store
        .item(ids::HTUI_ANA_1)
        .await
        .expect("the memory store never fails")
        .expect("the demo item exists");
    assert_eq!(head.priority, 9, "their priority");
    assert_eq!(head.title, format!("{ANA_1_TITLE} mine"), "my title");
    assert_eq!(head.version, 3);
    assert_eq!(harness.app().status, None, "a resolution raises nothing");
}

/// D5: `Esc` in the view returns to the form with its token and its text; a second `Ctrl+S`
/// compares again and still writes nothing, and the form then closes as ever.
#[tokio::test]
async fn esc_from_the_view_keeps_the_text_and_the_head() {
    let store = MemStore::demo();
    let mut harness = diverged_on(&store, retitled("Theirs")).await;

    keys(&mut harness, &["esc"]).await;
    let frame = harness.render();
    assert!(
        frame.contains(" Edit ANA-1 (v1) "),
        "the form keeps its token:\n{frame}"
    );
    assert!(frame.contains("still behind v2"), "and says so:\n{frame}");
    // The title row scrolls inside its `TextField`, so only the typed tail is sure to show.
    assert!(frame.contains("mine"), "the typed text is kept:\n{frame}");
    assert_eq!(ana_1_head(&store).await, ("Theirs".to_owned(), 2));

    keys(&mut harness, &["ctrl-s"]).await;
    let frame = harness.render();
    assert!(frame.contains("theirs v2"), "the view is back:\n{frame}");
    assert_eq!(
        ana_1_head(&store).await,
        ("Theirs".to_owned(), 2),
        "the second save wrote nothing either"
    );

    keys(&mut harness, &["esc", "esc"]).await;
    let frame = harness.render();
    assert!(!frame.contains(" Edit "), "the form is closed:\n{frame}");
    assert_eq!(ana_1_head(&store).await, ("Theirs".to_owned(), 2));
}

/// D4: the view over the whole tab, list pane included: the header, the field rows with their
/// states, the body's two diffs side by side and the hint. Paths are the same on all three
/// sides, so there is no `Tab body/paths`.
#[tokio::test]
async fn the_divergence_view_renders_over_the_whole_tab() {
    let store = MemStore::demo();
    let mut harness = backlog_over(store.clone()).await;
    keys(&mut harness, &["e"]).await;
    their_write(
        &store,
        ItemPatch {
            body: Some("Their body.".to_owned()),
            ..retitled("Theirs")
        },
    )
    .await;
    type_text(&mut harness, " mine");
    keys(&mut harness, &TITLE_TO_BODY).await;
    // The body's cursor starts at byte 0, so this prepends.
    type_text(&mut harness, "Mine first. ");
    keys(&mut harness, &["ctrl-s"]).await;
    let frame = harness.render();
    for wanted in [
        "ANA-1  v1 \u{2192} theirs v2",
        "--- ancestor v1",
        "+++ theirs v2",
        "+++ mine",
        "t theirs wins  m mine wins  Esc back",
    ] {
        assert!(frame.contains(wanted), "{wanted:?} in the view:\n{frame}");
    }
    let conflicts = frame
        .lines()
        .filter(|line| line.contains("conflict"))
        .collect::<Vec<_>>();
    assert!(
        conflicts.iter().any(|line| line.contains("title"))
            && conflicts.iter().any(|line| line.contains("body")),
        "title and body conflict:\n{frame}"
    );
    assert!(
        !frame.contains("Tab body/paths"),
        "paths are the same:\n{frame}"
    );
    insta::assert_snapshot!("item_divergence", frame);
}

// ---------------------------------------------------------------------------------------------
// $EDITOR round-trip (MOD-13 milestone 4, plan D1-D8).
// ---------------------------------------------------------------------------------------------

/// `ANA-1`'s head row.
async fn ana_1(store: &MemStore) -> Item {
    store
        .item(ids::HTUI_ANA_1)
        .await
        .expect("the memory store never fails")
        .expect("the demo item exists")
}

/// D3, D4: `e`, Body, `Ctrl+E` asks the loop for the editor with ANA-1's body; the text that
/// comes back lands in the form (minus the editor's final newline, D5) and `Ctrl+S` saves it as
/// version 2.
#[tokio::test]
async fn ctrl_e_hands_the_body_out_and_ctrl_s_saves_what_came_back() {
    let store = MemStore::demo();
    let mut harness = backlog_over(store.clone()).await;
    keys(&mut harness, &["e"]).await;
    keys(&mut harness, &TITLE_TO_BODY).await;
    keys(&mut harness, &["ctrl-e"]).await;
    assert_eq!(
        harness.app().take_external_edit(),
        Some((
            BacklogTab::ID,
            ExternalEdit {
                text: ana_1(&store).await.body,
                stem: "ANA-1-body".to_owned(),
            }
        ))
    );
    assert!(harness.app().take_external_edit().is_none(), "asked once");

    harness.app().finish_external_edit(
        BacklogTab::ID,
        ExternalEditOutcome::Edited("New body.\n".to_owned()),
    );
    let frame = harness.render();
    assert!(frame.contains("edited in $EDITOR"), "the notice:\n{frame}");
    assert!(
        frame.contains(" Edit ANA-1 (v1) "),
        "the form, unsaved:\n{frame}"
    );

    keys(&mut harness, &["ctrl-s"]).await;
    let head = ana_1(&store).await;
    assert_eq!(head.body, "New body.", "D5 dropped the editor's newline");
    assert_eq!(head.version, 2);
    let detail = detail_pane(&harness.render());
    assert!(detail.contains("version 2"), "the new version:\n{detail}");
    assert!(
        !detail.contains(" Edit "),
        "the form closed on the write:\n{detail}"
    );
    assert_eq!(
        harness.app().status,
        None,
        "an external edit raises nothing"
    );
}

/// The PRD risk (blueprint E2). Two batches of app-origin polls (`StoreState`, `ActiveRuns`) are
/// served, one on each side of the editor's outcome: they are addressed to the shell, not the
/// tab, so this pins only that they cannot reach or disturb the open form (nothing Backlog-bound
/// can be in flight while an idle form is open). The real asserts are the rest: another writer
/// moves the head while the editor is open, the editor's text and the form's token survive, and
/// `Ctrl+S` is stale and opens the three-way view with the editor's body as mine.
#[tokio::test]
async fn a_reply_and_a_concurrent_write_around_the_editor_end_in_the_divergence_view() {
    let store = MemStore::demo();
    let mut harness = backlog_over(store.clone()).await;
    keys(&mut harness, &["e"]).await;
    keys(&mut harness, &TITLE_TO_BODY).await;
    // One shell refresh (`update.rs`'s private `TICKS_PER_REFRESH` is 4): `StoreState` and
    // `ActiveRuns` are queued, not served.
    for _ in 0..4 {
        harness.app().update(Action::Tick);
    }
    // `key`, not `keys`: nothing is served yet.
    harness.key("ctrl-e");
    let (tab, edit) = harness
        .app()
        .take_external_edit()
        .expect("`Ctrl+E` asked for the editor");
    their_write(&store, retitled("Theirs")).await;
    // The queued replies land before the outcome (the plan's order)...
    assert!(harness.queued() > 0, "the refresh's polls are in flight");
    harness.settle().await;
    assert_eq!(harness.queued(), 0, "and served before the outcome");
    for _ in 0..4 {
        harness.app().update(Action::Tick);
    }
    harness.app().finish_external_edit(
        tab,
        ExternalEditOutcome::Edited(format!("{}\nWritten in the editor.\n", edit.text)),
    );
    // ...and these after it (the loop's: `finish_external_edit`, then the next `select!`).
    assert!(
        harness.queued() > 0,
        "the second refresh's polls are in flight"
    );
    harness.settle().await;
    assert_eq!(harness.queued(), 0, "and served after the outcome");
    let frame = harness.render();
    assert!(frame.contains("edited in $EDITOR"), "the notice:\n{frame}");
    assert!(
        frame.contains(" Edit ANA-1 (v1) "),
        "the token did not move:\n{frame}"
    );

    keys(&mut harness, &["ctrl-s"]).await;
    let frame = harness.render();
    for wanted in ["ANA-1  v1 \u{2192} theirs v2", "Written in the editor."] {
        assert!(frame.contains(wanted), "{wanted:?} in the view:\n{frame}");
    }
    assert_eq!(divergence_state(&frame, "title"), "theirs", "{frame}");
    assert_eq!(divergence_state(&frame, "body"), "mine", "{frame}");
    assert_eq!(
        ana_1_head(&store).await,
        ("Theirs".to_owned(), 2),
        "the stale save wrote nothing"
    );

    keys(&mut harness, &["m"]).await;
    keys(&mut harness, &["ctrl-s"]).await;
    let head = ana_1(&store).await;
    assert_eq!(head.title, "Theirs");
    assert_eq!(head.body, format!("{}\nWritten in the editor.", edit.text));
    assert_eq!(head.version, 3);
    assert_eq!(harness.app().status, None, "a resolution raises nothing");
}

/// E3: a real (fake) editor through the public `editor::run`, with the `ExternalEdit` the app
/// asked for. It appends a line and saves with a final newline, as an editor does; D5 drops that
/// newline, so the stored body ends on the appended line. No terminal is driven:
/// `run_suspended`'s leave/enter is pinned by `editor.rs`'s `suspension` tests.
#[cfg(unix)]
#[tokio::test]
async fn a_fake_editor_appends_a_line_and_ctrl_s_saves_it() {
    use htui::editor::EditorCommand;
    use std::os::unix::fs::PermissionsExt as _;

    let store = MemStore::demo();
    let mut harness = backlog_over(store.clone()).await;
    keys(&mut harness, &["e"]).await;
    keys(&mut harness, &TITLE_TO_BODY).await;
    keys(&mut harness, &["ctrl-e"]).await;
    let (tab, edit) = harness
        .app()
        .take_external_edit()
        .expect("`Ctrl+E` asked for the editor");

    let dir = tempfile::TempDir::new().expect("a temp dir");
    let path = dir.path().join("append");
    // R-13: `fs::write` closes the handle at once, so a fork elsewhere cannot hold it open for
    // long.
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf '\\nAppended by the editor.\\n' >> \"$1\"\n",
    )
    .expect("the script is written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("the script is executable");
    let cmd = EditorCommand::resolve(move |key: &str| {
        (key == "VISUAL").then(|| format!("'{}'", path.display()))
    });
    let outcome = htui::editor::run(&cmd, &edit.text, &edit.stem).await;
    assert_eq!(
        outcome,
        ExternalEditOutcome::Edited(format!("{}\nAppended by the editor.\n", edit.text))
    );

    harness.app().finish_external_edit(tab, outcome);
    keys(&mut harness, &["ctrl-s"]).await;
    let head = ana_1(&store).await;
    assert_eq!(
        head.body,
        format!("{}\nAppended by the editor.", edit.text),
        "D5 dropped the editor's final newline"
    );
    assert_eq!(head.version, 2);
    assert_eq!(
        harness.app().status,
        None,
        "an external edit raises nothing"
    );
}

/// D6: offline no form opens, so `Ctrl+E` asks for no editor; with no form the chord reaches the
/// detail pane, and nothing there or globally claims it.
#[tokio::test]
async fn offline_ctrl_e_asks_for_no_editor() {
    let _keyring = htui_store::testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("a throwaway config root");
    let (mut harness, _cache) = offline_backlog(root.path()).await;
    let refused = format!("item_form: store unreachable: {DATABASE_UNREACHABLE}");

    harness.app().status = None;
    keys(&mut harness, &["e"]).await;
    assert_eq!(harness.app().status, Some(refused), "`e` offline");
    let frame = harness.render();
    assert!(!frame.contains(" Edit "), "no form opened:\n{frame}");

    keys(&mut harness, &["ctrl-e"]).await;
    assert!(harness.app().take_external_edit().is_none());
}

// ---------------------------------------------------------------------------------------------
// Notes and documents (MOD-13 milestone 5, plan D1-D12).
// ---------------------------------------------------------------------------------------------

/// Sub-tabs right of Body to Docs (V22).
const TO_DOCS: usize = 3;

/// Sub-tabs right of Body to Notes (V22).
const TO_NOTES: usize = 4;

/// [`backlog_over`] on htui `FEAT-1`, `pane` sub-tabs right of Body.
async fn on_feat_1(store: MemStore, pane: usize) -> Harness {
    let mut harness = backlog_over(store).await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, pane);
    harness.drive_to_end().await;
    harness
}

/// `item`'s notes, oldest first.
async fn notes_of(store: &MemStore, item: ItemId) -> Vec<Note> {
    store
        .notes(item)
        .await
        .expect("the memory store never fails")
}

/// `item`'s documents, by kind then version.
async fn documents_of(store: &MemStore, item: ItemId) -> Vec<DocumentHead> {
    store
        .documents(item)
        .await
        .expect("the memory store never fails")
}

/// The detail pane's rows, borders and padding trimmed, joined by spaces: a wrapped notice reads
/// whole.
fn detail_text(frame: &str) -> String {
    detail_pane(frame)
        .lines()
        .map(|line| line.trim_end_matches(['\u{2502}', ' ']).trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// D1, D4, D8: `a` opens the compose area, the pasted text is the note, and Ctrl+S lands it by
/// hand (no step) as this user on this box; the thread shows it one row per line.
#[tokio::test]
async fn a_types_a_note_and_ctrl_s_adds_it_to_the_thread() {
    let store = MemStore::demo();
    let mut harness = on_feat_1(store.clone(), TO_NOTES).await;
    keys(&mut harness, &["a"]).await;
    harness.paste("Written by hand.\nSecond line.");
    keys(&mut harness, &["ctrl-s"]).await;

    let notes = notes_of(&store, ids::HTUI_FEAT_1).await;
    let note = notes.last().expect("a note landed");
    assert_eq!(note.body, "Written by hand.\nSecond line.");
    assert_eq!(note.via_step_id, None);
    assert_eq!(note.created_by, ids::USER);
    assert_eq!(note.box_id, Some(ids::BOX));

    let detail = detail_pane(&harness.render());
    let rows: Vec<&str> = detail.lines().map(str::trim).collect();
    let first = rows
        .iter()
        .position(|row| row.starts_with("Written by hand."))
        .unwrap_or_else(|| panic!("the first line:\n{detail}"));
    assert!(
        rows[first + 1].starts_with("Second line."),
        "the second line on its own row:\n{detail}"
    );
    assert!(!detail.contains(" New note "), "the area closed:\n{detail}");
    assert_eq!(harness.app().status, None);
}

/// D9: `v` on the cursor's `plan` opens the form prefilled from v2, and Ctrl+S lands v3 by hand.
#[tokio::test]
async fn v_on_plan_writes_plan_v3_by_hand() {
    let store = MemStore::demo();
    let mut harness = on_feat_1(store.clone(), TO_DOCS).await;
    keys(&mut harness, &["v"]).await;
    let frame = harness.render();
    for wanted in [
        " New version of plan (from v2) ",
        "Plan: TUI scaffold (revised)",
    ] {
        assert!(frame.contains(wanted), "{wanted:?} in the form:\n{frame}");
    }

    harness.paste("Edited by hand.\n");
    keys(&mut harness, &["ctrl-s"]).await;
    let documents = documents_of(&store, ids::HTUI_FEAT_1).await;
    let v3 = documents
        .iter()
        .find(|document| document.kind == "plan" && document.version == 3)
        .expect("plan v3 landed");
    assert_eq!(v3.produced_by_step_id, None);
    let body = store
        .document(v3.id)
        .await
        .expect("the memory store never fails")
        .expect("plan v3")
        .body;
    assert!(body.starts_with("Edited by hand.\n# Plan"), "{body:?}");

    let frame = harness.render();
    let detail = detail_pane(&frame);
    assert!(detail.contains("v3  hand"), "the new row:\n{detail}");
    let text = detail_text(&frame);
    assert!(text.contains("saved as plan v3"), "the notice: {text}");
    assert_eq!(harness.app().status, None);
}

/// Maintainer answer (blueprint §6 Q1): `v`, then Ctrl+S with nothing edited, says there is
/// nothing to save, keeps the form open and writes nothing.
#[tokio::test]
async fn v_then_ctrl_s_with_no_edit_saves_nothing() {
    let store = MemStore::demo();
    let before = documents_of(&store, ids::HTUI_FEAT_1).await.len();
    let mut harness = on_feat_1(store.clone(), TO_DOCS).await;
    keys(&mut harness, &["v"]).await;
    keys(&mut harness, &["ctrl-s"]).await;

    let frame = harness.render();
    assert!(
        frame.contains(" New version of plan (from v2) "),
        "the form stays open:\n{frame}"
    );
    let text = detail_text(&frame);
    assert!(text.contains("nothing to save"), "the notice: {text}");
    assert_eq!(
        documents_of(&store, ids::HTUI_FEAT_1).await.len(),
        before,
        "nothing was written"
    );
    assert_eq!(harness.app().status, None);
}

/// D4, D9: `a` with a typed kind, a title and a body lands that kind's first version.
#[tokio::test]
async fn a_with_kind_summary_lands_summary_v1() {
    let store = MemStore::demo();
    let mut harness = on_feat_1(store.clone(), TO_DOCS).await;
    keys(&mut harness, &["a"]).await;
    type_text(&mut harness, "summary");
    keys(&mut harness, &["tab"]).await;
    type_text(&mut harness, "Summary of FEAT-1");
    keys(&mut harness, &["tab"]).await;
    harness.paste("The summary.");
    keys(&mut harness, &["ctrl-s"]).await;

    let documents = documents_of(&store, ids::HTUI_FEAT_1).await;
    let summary = documents
        .iter()
        .find(|document| document.kind == "summary")
        .expect("summary landed");
    assert_eq!(summary.version, 1);
    assert_eq!(summary.title, "Summary of FEAT-1");
    assert_eq!(summary.produced_by_step_id, None);

    let frame = harness.render();
    let text = detail_text(&frame);
    assert!(text.contains("saved as summary v1"), "the notice: {text}");
    assert!(
        detail_pane(&frame)
            .lines()
            .any(|row| row.trim_start().starts_with("summary")),
        "the table lists it:\n{frame}"
    );
    assert_eq!(harness.app().status, None);
}

/// D5: a version another writer lands between `v` and Ctrl+S is kept, and the notice names it.
#[tokio::test]
async fn a_version_written_meanwhile_is_named_in_the_notice() {
    let store = MemStore::demo();
    let mut harness = on_feat_1(store.clone(), TO_DOCS).await;
    keys(&mut harness, &["v"]).await;
    let theirs = store
        .write_document(NewDocument {
            id: DocumentId::new(),
            item_id: ids::HTUI_FEAT_1,
            kind: "plan".to_owned(),
            title: "Theirs".to_owned(),
            body: "Theirs.".to_owned(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: Utc::now(),
        })
        .await
        .expect("their version lands");
    assert_eq!(theirs.version, 3);

    harness.paste("Mine.\n");
    keys(&mut harness, &["ctrl-s"]).await;
    let frame = harness.render();
    let text = detail_text(&frame);
    assert!(
        text.contains(
            "saved as plan v4 \u{2014} v3 was written after you opened v2; both are kept"
        ),
        "the notice: {text}"
    );
    let plans: Vec<i32> = documents_of(&store, ids::HTUI_FEAT_1)
        .await
        .into_iter()
        .filter(|document| document.kind == "plan")
        .map(|document| document.version)
        .collect();
    assert_eq!(plans, [1, 2, 3, 4], "both are kept");
    let detail = detail_pane(&frame);
    for row in ["v3  hand", "v4  hand"] {
        assert!(detail.contains(row), "{row:?} listed:\n{detail}");
    }
}

/// D2: offline, `a` is refused by the worker before anything is read, in either pane. The status
/// line carries the sentence under the form read's name, no area opens, and the mirror is
/// untouched.
#[tokio::test]
async fn offline_a_opens_no_compose_in_either_pane() {
    let _keyring = htui_store::testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("a throwaway config root");
    let (mut harness, cache) = offline_backlog(root.path()).await;
    assert!(
        harness.render().contains("\u{250c} ANA-1"),
        "ANA-1 is selected"
    );
    let notes = cache
        .notes(ids::HTUI_ANA_1)
        .await
        .expect("the mirror")
        .len();
    let documents = cache
        .documents(ids::HTUI_ANA_1)
        .await
        .expect("the mirror")
        .len();

    sub_tab(&mut harness, TO_DOCS);
    harness.drive_to_end().await;
    harness.app().status = None;
    keys(&mut harness, &["a"]).await;
    assert_eq!(
        harness.app().status,
        Some(format!(
            "document_form: store unreachable: {DATABASE_UNREACHABLE}"
        )),
        "`a` on Docs offline"
    );
    let frame = harness.render();
    assert!(!frame.contains(" New document "), "no form:\n{frame}");

    sub_tab(&mut harness, 1);
    harness.drive_to_end().await;
    harness.app().status = None;
    keys(&mut harness, &["a"]).await;
    assert_eq!(
        harness.app().status,
        Some(format!(
            "note_form: store unreachable: {DATABASE_UNREACHABLE}"
        )),
        "`a` on Notes offline"
    );
    let frame = harness.render();
    assert!(!frame.contains(" New note "), "no area:\n{frame}");

    assert_eq!(
        cache
            .notes(ids::HTUI_ANA_1)
            .await
            .expect("the mirror")
            .len(),
        notes
    );
    assert_eq!(
        cache
            .documents(ids::HTUI_ANA_1)
            .await
            .expect("the mirror")
            .len(),
        documents
    );
}

/// D7: Ctrl+E in the Notes area hands the (empty) text out under `FEAT-1-note`; what comes back
/// lands in the area (minus the editor's final newline, D5) and Ctrl+S adds it. Mirrors
/// [`ctrl_e_hands_the_body_out_and_ctrl_s_saves_what_came_back`].
#[tokio::test]
async fn ctrl_e_hands_the_note_to_the_editor_and_ctrl_s_adds_it() {
    let store = MemStore::demo();
    let mut harness = on_feat_1(store.clone(), TO_NOTES).await;
    keys(&mut harness, &["a"]).await;
    keys(&mut harness, &["ctrl-e"]).await;
    assert_eq!(
        harness.app().take_external_edit(),
        Some((
            BacklogTab::ID,
            ExternalEdit {
                text: String::new(),
                stem: "FEAT-1-note".to_owned(),
            }
        ))
    );
    assert!(harness.app().take_external_edit().is_none(), "asked once");

    harness.app().finish_external_edit(
        BacklogTab::ID,
        ExternalEditOutcome::Edited("From the editor.\n".to_owned()),
    );
    let frame = harness.render();
    assert!(frame.contains("edited in $EDITOR"), "the notice:\n{frame}");
    assert!(frame.contains(" New note "), "the area, unsaved:\n{frame}");

    keys(&mut harness, &["ctrl-s"]).await;
    let notes = notes_of(&store, ids::HTUI_FEAT_1).await;
    assert_eq!(
        notes.last().map(|note| note.body.as_str()),
        Some("From the editor."),
        "D5 dropped the editor's newline"
    );
    assert_eq!(
        harness.app().status,
        None,
        "an external edit raises nothing"
    );
}

#[tokio::test]
async fn the_note_compose_renders_in_the_notes_pane() {
    let mut harness = on_feat_1(MemStore::demo(), TO_NOTES).await;
    keys(&mut harness, &["a"]).await;
    harness.paste("A hand-written note.\nIts second line.");
    let frame = harness.render();
    insta::assert_snapshot!("note_compose", frame);
}

#[tokio::test]
async fn the_document_form_renders_in_the_docs_pane() {
    let mut harness = on_feat_1(MemStore::demo(), TO_DOCS).await;
    keys(&mut harness, &["v"]).await;
    let frame = harness.render();
    insta::assert_snapshot!("document_form", frame);
}
