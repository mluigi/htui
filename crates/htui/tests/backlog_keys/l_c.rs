//! Lane L-C of MOD-67 M5: the Body, Graph, Prompt and Reqs sub-tabs resolve their keys through
//! their context stacks (`BACKLOG_BODY`, `BACKLOG_GRAPH`, `BACKLOG_PROMPT`, `BACKLOG_REQS`,
//! `REQS_PICK`, `REQS_KIND`, `REQS_CONFIRM`): the D12 pins and the lane's rebinding test. Helpers
//! stay in this file.

use htui::agent_worker::AgentRuntime;
use htui::app::{Action, RevealKind};
use htui::keys::Keys;
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::model::WorkspaceSummary;
use htui_core::store::MemStore;

/// Steps right of Body to the Graph sub-tab: Body, Runs, **Graph**.
const TO_GRAPH: usize = 2;

/// Steps right of Body to the Prompt sub-tab: Body, Runs, Graph, Docs, Notes, **Prompt**.
const TO_PROMPT: usize = 5;

/// Rows down from the arrival row (htui `ANA-1`) to htui `FEAT-1`, the item every sub-tab has
/// data for (`tests/backlog.rs`).
const TO_FEAT_1: usize = 3;

/// The first row of `FEAT-1`'s body at the detail pane's width (`body.rs`'s in-file test).
const FEAT_1_FIRST_ROW: &str = "Stand up the terminal application: a";

/// The workspace of the demo fixture with this slug.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// A settled Backlog tab over the demo, scoped to `Platform`, with `keys` in force: what
/// `tests/backlog.rs`'s `backlog()` builds, with the keys a case rebinds.
async fn backlog_with(keys: Keys) -> Harness {
    let mut harness = Harness::demo()
        .with_keys(keys)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// [`backlog_with`] the compiled defaults.
async fn backlog() -> Harness {
    backlog_with(Keys::defaults()).await
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

/// PA-1 (main §1 item 2): the Prompt's template keys are `backlog.prompt.next_template`/
/// `prev_template`, matched with their modifiers: `ctrl-n` and `alt-p` (which the unconverted
/// Backlog tab hands the pane) cycle nothing, `n` asks for the next template's preview.
#[tokio::test]
async fn ctrl_n_does_not_cycle_the_prompt_template() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_PROMPT);
    let frame = harness.render();
    assert!(
        frame.contains("n / p"),
        "the Prompt, its preview landed:\n{frame}"
    );
    assert_eq!(harness.queued(), 0, "nothing in flight");

    harness.key("ctrl-n");
    assert_eq!(harness.queued(), 0, "`ctrl-n` sends no PromptPreview");
    harness.key("alt-p");
    assert_eq!(harness.queued(), 0, "`alt-p` sends no PromptPreview");

    harness.key("n");
    assert_eq!(harness.queued(), 1, "`n` asks for the next template");
}

/// Main §1 item 2: Body scrolls on `pane.scroll_down`/`page_down` alone; `alt-J` and `ctrl-pgdn`
/// (which `Scroll::on_key` matched on the code) leave the body on its first row, `J` scrolls.
#[tokio::test]
async fn a_chord_does_not_scroll_the_body() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    let frame = harness.render();
    assert!(
        frame.contains(FEAT_1_FIRST_ROW),
        "Body on FEAT-1, at the top:\n{frame}"
    );

    harness.key("alt-J");
    harness.key("ctrl-pgdn");
    let frame = harness.render();
    assert!(
        frame.contains(FEAT_1_FIRST_ROW),
        "no chord scrolled the body:\n{frame}"
    );

    harness.key("J");
    let frame = harness.render();
    assert!(
        !frame.contains(FEAT_1_FIRST_ROW),
        "`J` scrolled one row:\n{frame}"
    );
}

/// PA-2: on a Graph node `Enter` is `backlog.graph.reroot`, which shadows the Backlog's replay
/// miss: it re-roots the list (the detail border reads the new root) and the status line never
/// says "select a step in the Runs pane". On the new root's own row it is consumed too.
#[tokio::test]
async fn enter_re_roots_the_graph_and_never_says_the_miss() {
    let mut harness = backlog().await;
    // As `register_all` does: the Graph's `Enter` is an `Action::Reveal` (tests/backlog.rs).
    harness.app().reveal_tabs = vec![(RevealKind::Item, BacklogTab::ID)];
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, TO_GRAPH);
    let frame = harness.render();
    assert!(
        frame.contains("┌ FEAT-1") && frame.contains("depth 2/3 · +/- depth · Enter re-root"),
        "the Graph on FEAT-1, its footer label-built:\n{frame}"
    );

    // One `J` from FEAT-1's root row is htui ANA-1 (tests/backlog.rs).
    harness.key("J");
    harness.key("enter");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains("┌ ANA-1"), "re-rooted on ANA-1:\n{frame}");
    assert!(
        frame.contains("depth 2/3"),
        "the Graph is still active:\n{frame}"
    );
    assert_eq!(harness.app().status, None, "no miss on the status line");

    harness.key("enter");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("┌ ANA-1"),
        "the root row stays put:\n{frame}"
    );
    assert_eq!(
        harness.app().status,
        None,
        "the root row's `Enter` says no miss"
    );
}
