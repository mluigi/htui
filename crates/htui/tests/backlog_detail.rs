//! The Backlog's Body, Graph, Prompt and Reqs sub-tabs on the kit (MOD-115 M4 L-detail): the
//! harness cases the lane adds and the four 80-column snapshots (MOD-81).
//!
//! - Body's head is one meta line, `{status mark} · {tags} · priority {p} · version {v}` (E4):
//!   the key and the title are the pane title since B1, and the title keeps a line of its own only
//!   while the border cannot hold it whole.
//! - The Graph reads its link kinds as words and its statuses as marks; its cursor is the kit's
//!   gutter and band.
//! - The Prompt counts its templates in words and names its template keys on the pane's hint
//!   (MOD-82), not in the picker row.
//! - The Reqs cursor is the kit's gutter and band; its empty state names the cite key.
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::model::WorkspaceSummary;
use htui_core::store::MemStore;

/// The workspace of the demo fixture with this slug (`tests/backlog.rs`'s reading).
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// A settled Backlog tab scoped to `Platform` on a `width` x `height` terminal, with the agent
/// runtime the Prompt's preview needs (`tests/backlog.rs`'s `backlog_at`).
async fn backlog_at(width: u16, height: u16) -> Harness {
    let mut harness = Harness::demo()
        .size(width, height)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// [`backlog_at`] the 100x30 every other Backlog snapshot is drawn at.
async fn backlog() -> Harness {
    backlog_at(100, 30).await
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

/// Rows down from the arrival row (htui `ANA-1`) to htui `ANA-2`, which has no body.
const TO_ANA_2: usize = 1;

/// Rows down to htui `CLEAN-1`, which cites no requirement.
const TO_CLEAN_1: usize = 2;

/// Rows down to htui `FEAT-1`, the item every sub-tab has data for.
const TO_FEAT_1: usize = 3;

/// Steps right of Body: Body, Runs, **Graph**.
const TO_GRAPH: usize = 2;

/// Steps right of Body: Body, Runs, Graph, Docs, Notes, **Prompt**.
const TO_PROMPT: usize = 5;

/// Steps right of Body: ... Prompt, **Reqs**.
const TO_REQS: usize = 6;

/// The detail pane's rows of `frame`: what lies right of the list pane's right border.
fn detail(frame: &str) -> Vec<String> {
    frame
        .lines()
        .map(|line| {
            let cut = line
                .find("\u{2502}\u{2502}")
                .or_else(|| line.find("\u{256e}\u{256d}"));
            cut.map_or_else(String::new, |at| line[at + "\u{2502}".len()..].to_owned())
        })
        .collect()
}

/// E4: on FEAT-1, whose `KEY · title` the border holds whole, Body's head is the meta line alone:
/// the status as its mark, the tags, the priority and the version. No title line and no
/// `KEY  PREFIX  status` line repeat the border.
#[tokio::test]
async fn the_body_head_is_the_meta_line_alone() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{2500} FEAT-1 \u{b7} TUI scaffold "),
        "the border carries the key and the title:\n{frame}"
    );
    assert!(
        frame.contains("\u{2502}\u{25d0} running \u{b7} rust \u{b7} priority 2 \u{b7} version 1"),
        "the meta line opens the Body:\n{frame}"
    );
    assert!(
        !frame.contains("\u{2502}TUI scaffold"),
        "no title line repeats the border:\n{frame}"
    );
    assert!(
        !frame.contains("FEAT-1  FEAT") && !frame.contains("in_progress"),
        "no key line and no raw status:\n{frame}"
    );
}

/// Content parity (acceptance 7): ANA-2's `KEY · title` is cut on the 45-column border, so the
/// Body keeps the title whole on its first row, then the meta line.
#[tokio::test]
async fn a_title_the_border_cuts_keeps_its_line_in_the_body() {
    let mut harness = backlog().await;
    down(&mut harness, TO_ANA_2).await;
    let frame = harness.render();
    let rows = detail(&frame);
    assert!(
        rows.iter()
            .any(|row| row.starts_with("\u{2502}Orchestrator step graphs and gates ")),
        "the whole title:\n{frame}"
    );
    assert!(
        frame.contains("\u{2502}\u{25cb} open \u{b7} no tags \u{b7} priority 0 \u{b7} version 1"),
        "then the meta line:\n{frame}"
    );
}

/// An item with no body says so and names the key that writes one (`kit::empty`).
#[tokio::test]
async fn an_empty_body_names_the_edit_key() {
    let mut harness = backlog().await;
    down(&mut harness, TO_ANA_2).await;
    let frame = harness.render();
    assert!(frame.contains("No body for this item."), "{frame}");
    assert!(frame.contains("e writes one"), "{frame}");
}

/// The Graph's edges read as words and its nodes as status marks (MOD-83, D-1); the root row is
/// under the kit's gutter.
#[tokio::test]
async fn the_graph_reads_its_kinds_and_statuses_as_words() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_GRAPH);
    let frame = harness.render();
    assert!(
        frame.contains("\u{258c} FEAT-1 \u{25d0} running TUI scaffold"),
        "the root row under the gutter:\n{frame}"
    );
    assert!(
        frame.contains("\u{2190} blocked by FEAT-2 \u{2298} blocked"),
        "a kind and a status as words:\n{frame}"
    );
    assert!(
        !frame.contains("blocked_by") && !frame.contains("in_progress"),
        "no raw value:\n{frame}"
    );
    assert!(
        !frame.contains("\u{25b8} FEAT-1"),
        "no old cursor:\n{frame}"
    );
}

/// MOD-83, MOD-82: the picker row counts the templates in words and names no key; the template
/// keys are the detail's hint while it is focused.
#[tokio::test]
async fn the_prompt_names_its_template_keys_on_the_pane_hint() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_PROMPT);
    let frame = harness.render();
    assert!(
        frame.contains("picker    8 templates"),
        "the count in words:\n{frame}"
    );
    assert!(
        !frame.contains("(s)") && !frame.contains("n / p"),
        "no `(s)` and no key in the row:\n{frame}"
    );
    // The pane key moves the focus among the panes the last frame drew.
    harness.key("tab");
    let frame = harness.render();
    assert!(
        frame.contains("n/p template"),
        "the keys on the focused detail's border:\n{frame}"
    );
}

/// The Reqs cursor is the kit's gutter (MOD-85); the citation reads as before.
#[tokio::test]
async fn the_reqs_cursor_is_the_kit_gutter() {
    let mut harness = backlog().await;
    sub_tab(&mut harness, TO_REQS);
    let frame = harness.render();
    assert!(
        frame.contains("\u{258c} R-ENT-1 addresses v1 ! suspect"),
        "{frame}"
    );
    assert!(
        !frame.contains("\u{25b8} R-ENT-1"),
        "no old cursor:\n{frame}"
    );
}

/// No citation: the empty state names the key that cites one.
#[tokio::test]
async fn an_item_citing_nothing_names_the_cite_key() {
    let mut harness = backlog().await;
    down(&mut harness, TO_CLEAN_1).await;
    sub_tab(&mut harness, TO_REQS);
    let frame = harness.render();
    assert!(frame.contains("No requirements cited."), "{frame}");
    assert!(frame.contains("c cites one"), "{frame}");
}

/// MOD-81: Body at 80 columns.
#[tokio::test]
async fn body_at_80_columns() {
    let mut harness = backlog_at(80, 24).await;
    down(&mut harness, TO_FEAT_1).await;
    let frame = harness.render();
    // The 34 cells inside the detail wrap the meta line; every part of it stays.
    assert!(
        frame.contains("\u{2502}\u{25d0} running \u{b7} rust \u{b7} priority 2 \u{b7}")
            && frame.contains("\u{2502}version 1 "),
        "{frame}"
    );
    insta::assert_snapshot!("body_80", frame);
}

/// MOD-81: the Graph at 80 columns.
#[tokio::test]
async fn graph_at_80_columns() {
    let mut harness = backlog_at(80, 24).await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_GRAPH);
    let frame = harness.render();
    assert!(frame.contains("depth 2/3"), "{frame}");
    insta::assert_snapshot!("graph_80", frame);
}

/// MOD-81: the Prompt at 80 columns, its rows cut with `…` (D-24).
#[tokio::test]
async fn prompt_at_80_columns() {
    let mut harness = backlog_at(80, 24).await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_PROMPT);
    let frame = harness.render();
    assert!(frame.contains("8 templates"), "{frame}");
    assert!(
        detail(&frame)
            .iter()
            .any(|row| row.ends_with("\u{2026}\u{2502}")),
        "a cut row ends in `…`:\n{frame}"
    );
    insta::assert_snapshot!("prompt_80", frame);
}

/// MOD-81: the Reqs at 80 columns.
#[tokio::test]
async fn reqs_at_80_columns() {
    let mut harness = backlog_at(80, 24).await;
    sub_tab(&mut harness, TO_REQS);
    let frame = harness.render();
    assert!(frame.contains("R-ENT-1 addresses v1"), "{frame}");
    insta::assert_snapshot!("reqs_80", frame);
}
