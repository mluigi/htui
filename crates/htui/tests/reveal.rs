//! `Action::Reveal` (MOD-64 D235), through a `Harness` over the demo world: the shell focuses the
//! tab registered for the target's kind and the tab selects it, now or when its list lands.
//!
//! The Harness enters the Graphics workspace at startup; TOOL-1 and every requirement are on
//! Platform's `htui`, so each case moves there first (the `tests/requirements.rs` walk). The agent
//! runtime is installed because a Backlog selection sends a `PromptPreview`, which `settle` would
//! answer `Failed` onto the status line. A selection is read from the detail pane's title,
//! `┌ KEY `. `R-STO-1`, not `R-ENT-1`, which the Requirements tab lands on by itself.
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::{Action, RevealTarget, register_all};
use htui::testkit::Harness;
use htui::ui::tabs::backlog::not_in_this_backlog;
use htui::ui::tabs::registry::CLOSE_THE_FIELD_FIRST;
use htui::ui::tabs::requirements::not_in_these_requirements;
use htui::ui::tabs::{BacklogTab, RequirementsTab};
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{ItemId, RequirementId};

/// The demo shell, moved to Platform with the switcher's last key pressed and nothing served
/// since: the scope change has cleared the Backlog and its `Items` read is still queued.
async fn platform_unserved() -> Harness {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness.key("w");
    harness.drive_to_end().await;
    harness.key("j");
    harness.key("enter");
    harness
}

/// The demo shell in Platform, on the Backlog, everything served.
async fn platform() -> Harness {
    let mut harness = platform_unserved().await;
    harness.drive_to_end().await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    harness
}

/// Emits the reveal as the search overlay would, and serves what it asked for.
async fn reveal(harness: &mut Harness, target: RevealTarget) {
    harness.app().update(Action::Reveal(target));
    harness.drive_to_end().await;
}

/// `htui` TOOL-1.
fn tool_1() -> RevealTarget {
    RevealTarget::Item {
        id: ids::HTUI_TOOL_1,
        key: "TOOL-1".to_owned(),
    }
}

/// `htui` R-STO-1.
fn r_sto_1() -> RevealTarget {
    RevealTarget::Requirement {
        id: ids::REQ_STO_1,
        key: "R-STO-1".to_owned(),
    }
}

/// The detail pane's title for `key`.
fn titled(key: &str) -> String {
    format!("\u{250c} {key} ")
}

/// The Requirements tab's notice: the rows between the content's bottom border and the hint,
/// joined (the `tests/requirements.rs` reading).
fn notice(frame: &str) -> String {
    let lines: Vec<&str> = frame.lines().collect();
    let hint = lines.len() - 2;
    let bottom = lines[..hint]
        .iter()
        .rposition(|line| line.starts_with('\u{2514}'))
        .unwrap_or_else(|| panic!("no bottom border above the hint:\n{frame}"));
    lines[bottom + 1..hint]
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Types `text` one key at a time: a space is `space`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        match c {
            ' ' => harness.key("space"),
            c => harness.key(&c.to_string()),
        }
    }
}

#[tokio::test]
async fn revealing_a_loaded_item_focuses_the_backlog_and_selects_it() {
    let mut harness = platform().await;
    harness.key("3");
    harness.drive_to_end().await;
    assert_eq!(harness.app().tabs.active_id(), Some(RequirementsTab::ID));

    reveal(&mut harness, tool_1()).await;
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    let frame = harness.render();
    assert!(frame.contains(&titled("TOOL-1")), "{frame}");
    assert!(
        frame.contains("\u{2502}CI matrix for the three OSes"),
        "the Body pane shows TOOL-1: {frame}"
    );
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn revealing_an_item_under_a_folded_project_unfolds_it() {
    let mut harness = platform().await;
    harness.key("g");
    harness.drive_to_end().await;
    harness.key("enter");
    harness.drive_to_end().await;
    let folded = harness.render();
    assert!(folded.contains("\u{25b8} htui"), "{folded}");

    reveal(&mut harness, tool_1()).await;
    let frame = harness.render();
    assert!(frame.contains("\u{25be} htui"), "{frame}");
    assert!(frame.contains(&titled("TOOL-1")), "{frame}");
}

#[tokio::test]
async fn revealing_before_the_backlog_has_loaded_selects_on_arrival() {
    let mut harness = platform_unserved().await;
    reveal(&mut harness, tool_1()).await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    let frame = harness.render();
    assert!(frame.contains(&titled("TOOL-1")), "{frame}");
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn revealing_an_unknown_item_says_so_and_keeps_the_cursor() {
    let mut harness = platform().await;
    let before = harness.render();
    let title_row = |frame: &str| frame.lines().nth(2).unwrap_or_default().to_owned();
    assert!(title_row(&before).contains(&titled("ANA-1")), "{before}");

    reveal(
        &mut harness,
        RevealTarget::Item {
            id: ItemId::new(),
            key: "NOPE-1".to_owned(),
        },
    )
    .await;
    assert_eq!(harness.app().status, Some(not_in_this_backlog("NOPE-1")));
    let after = harness.render();
    assert_eq!(title_row(&after), title_row(&before), "{after}");
}

#[tokio::test]
async fn revealing_a_requirement_focuses_the_requirements_tab_and_opens_it() {
    let mut harness = platform().await;
    reveal(&mut harness, r_sto_1()).await;
    assert_eq!(harness.app().tabs.active_id(), Some(RequirementsTab::ID));
    let frame = harness.render();
    assert!(frame.contains(&titled("R-STO-1")), "{frame}");
    assert!(
        frame.contains("Postgres is the source of truth;"),
        "the detail body, not the truncated tree row: {frame}"
    );
    assert!(!frame.contains("select a requirement"), "{frame}");
}

#[tokio::test]
async fn revealing_a_requirement_the_filter_hides_clears_the_filter() {
    let mut harness = platform().await;
    harness.key("3");
    harness.drive_to_end().await;
    harness.key("/");
    type_text(&mut harness, "priority");
    harness.key("enter");
    harness.drive_to_end().await;
    let filtered = harness.render();
    assert!(filtered.contains("\u{b7} /priority"), "{filtered}");
    assert!(!filtered.contains("R-STO-1"), "{filtered}");

    reveal(&mut harness, r_sto_1()).await;
    let frame = harness.render();
    assert!(frame.contains(" Requirements (3) \u{2500}"), "{frame}");
    // The tree title's `· /priority`; the hint row has its own `· / filter` (MOD-67 M4).
    assert!(
        !frame.contains("\u{b7} /priority"),
        "the filter is gone: {frame}"
    );
    assert!(frame.contains(&titled("R-STO-1")), "{frame}");
}

#[tokio::test]
async fn revealing_an_unknown_requirement_says_so_on_the_tab() {
    let mut harness = platform().await;
    harness.key("3");
    harness.drive_to_end().await;

    reveal(
        &mut harness,
        RevealTarget::Requirement {
            id: RequirementId::new(),
            key: "R-NOPE-1".to_owned(),
        },
    )
    .await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains(&not_in_these_requirements("R-NOPE-1")),
        "{frame}"
    );
    assert!(
        frame.contains(&titled("R-ENT-1")),
        "the cursor stays: {frame}"
    );
}

#[tokio::test]
async fn a_reveal_over_an_open_form_keeps_the_form() {
    let mut harness = platform().await;
    harness.key("3");
    harness.drive_to_end().await;
    // Up from R-ENT-1 onto its area, ENT.
    harness.key("k");
    harness.drive_to_end().await;
    harness.key("n");
    type_text(&mut harness, "Keys are short.");
    let open = harness.render();
    assert!(open.contains(" New requirement in ENT "), "{open}");

    reveal(&mut harness, r_sto_1()).await;
    let frame = harness.render();
    assert!(notice(&frame).contains(CLOSE_THE_FIELD_FIRST), "{frame}");
    assert!(frame.contains(" New requirement in ENT "), "{frame}");
    assert!(
        frame.contains("Keys are short."),
        "the text is kept: {frame}"
    );
}

/// MOD-64 review 7: by the time a tab refuses a reveal the search overlay has closed, and `Ctrl+F`
/// opens an empty one, so the sentence asks for a new search rather than for the same hit.
#[test]
fn the_refusal_asks_for_a_new_search_not_the_same_hit() {
    assert!(
        CLOSE_THE_FIELD_FIRST.ends_with("then search again"),
        "{CLOSE_THE_FIELD_FIRST}"
    );
}

// ---- MOD-69 plan D8: a reveal to a run's step (blueprint §4.6) ------------------------------

/// `htui` FEAT-1's `RUN_1` at `step`.
fn feat_1_at(step: htui_core::model::StepId) -> RevealTarget {
    RevealTarget::Step {
        item: ids::HTUI_FEAT_1,
        key: "FEAT-1".to_owned(),
        run: Some(ids::RUN_1),
        step: Some(step),
    }
}

/// The Runs pane's cursor line: the detail pane's row that starts with the pane's `▸`, past the
/// list's right border and the detail's left one.
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

/// The Runs pane is the one drawn: its run grid's header.
fn shows_runs(frame: &str) -> bool {
    frame.contains("\u{2502}kind   status    box")
}

#[tokio::test]
async fn revealing_a_step_lands_on_the_runs_pane_with_that_step_selected() {
    let mut harness = platform().await;
    reveal(&mut harness, feat_1_at(ids::STEP_REVIEW)).await;
    assert_eq!(harness.app().tabs.active_id(), Some(BacklogTab::ID));
    let frame = harness.render();
    assert!(frame.contains(&titled("FEAT-1")), "{frame}");
    assert!(shows_runs(&frame), "the Runs sub-tab is open: {frame}");
    let line = cursor_line(&frame);
    assert!(line.contains("review"), "{line}\n{frame}");
    assert!(!line.contains("prd"), "{line}\n{frame}");
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn revealing_a_step_of_the_already_selected_item_moves_the_cursor() {
    let mut harness = platform().await;
    reveal(
        &mut harness,
        RevealTarget::Item {
            id: ids::HTUI_FEAT_1,
            key: "FEAT-1".to_owned(),
        },
    )
    .await;
    harness.key("l");
    harness.drive_to_end().await;
    let before = harness.render();
    assert!(shows_runs(&before), "l lands on Runs: {before}");
    assert!(cursor_line(&before).contains("prd"), "{before}");

    reveal(&mut harness, feat_1_at(ids::STEP_IMPL)).await;
    let frame = harness.render();
    assert!(frame.contains(&titled("FEAT-1")), "{frame}");
    assert!(shows_runs(&frame), "{frame}");
    let line = cursor_line(&frame);
    assert!(line.contains("implement"), "{line}\n{frame}");
    assert_eq!(harness.app().status, None);
}

#[tokio::test]
async fn revealing_a_step_before_the_list_lands_selects_it_on_arrival() {
    let mut harness = platform_unserved().await;
    reveal(&mut harness, feat_1_at(ids::STEP_REVIEW)).await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    let frame = harness.render();
    assert!(frame.contains(&titled("FEAT-1")), "{frame}");
    assert!(shows_runs(&frame), "{frame}");
    let line = cursor_line(&frame);
    assert!(line.contains("review"), "{line}\n{frame}");
    assert!(!line.contains("prd"), "{line}\n{frame}");
    assert_eq!(harness.app().status, None);
}
