//! Backlog tab tests (blueprint §E, T4).
//!
//! Everything runs against `Harness::demo()` and the tab's own public surface, so no T3 or T5
//! file is touched and the T6 registration does not have to exist yet. Frames are 100x30, the
//! size the whole snapshot suite is pinned to (plan risk row).
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::model::WorkspaceSummary;
use htui_core::store::MemStore;

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
