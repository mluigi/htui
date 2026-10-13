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

/// Steps right of Body to the Reqs sub-tab, registered after Prompt.
const TO_REQS: usize = 6;

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
pub(super) async fn backlog() -> Harness {
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
        frame.contains("┌ FEAT-1")
            && frame.contains("depth 2/3")
            && frame.contains("+/- depth · Enter re-root"),
        "the Graph on FEAT-1, its keys label-built in the hint row:\n{frame}"
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

/// On htui `FEAT-1`'s Reqs sub-tab, its one citation (`R-STO-1 addresses`) under the cursor.
pub(super) async fn on_feat_1_reqs(harness: &mut Harness) {
    down(harness, TO_FEAT_1).await;
    sub_tab(harness, TO_REQS);
    let frame = harness.render();
    assert!(
        frame.contains("┌ FEAT-1") && frame.contains("▸ R-STO-1 addresses v1"),
        "the Reqs on FEAT-1, its citation under the cursor:\n{frame}"
    );
}

/// D7, D12: the uncite question resolves through `REQS_CONFIRM`, whose `confirm.yes` is `y`
/// alone. `alt-y` (which the CONTROL-only pass let through to the mode) sends no
/// `UnciteRequirement` and leaves the question up; `y` sends one.
#[tokio::test]
async fn alt_y_does_not_uncite() {
    let mut harness = backlog().await;
    on_feat_1_reqs(&mut harness).await;
    harness.key("u");
    let asking = harness.render();
    assert!(
        asking.contains("uncite R-STO-1 (addresses)?"),
        "`u` asks first:\n{asking}"
    );
    assert_eq!(harness.queued(), 0, "nothing in flight");

    harness.key("alt-y");
    assert_eq!(harness.queued(), 0, "`alt-y` sends no UnciteRequirement");
    let frame = harness.render();
    assert!(
        frame.contains("uncite R-STO-1 (addresses)?"),
        "the question stays:\n{frame}"
    );

    harness.key("y");
    assert_eq!(harness.queued(), 1, "`y` sends the UnciteRequirement");
}

/// Main §1 item 3: the picker is a modal on `REQS_PICK`, whose global layer passes `F1`, so `F1`
/// opens the help box from it (the picker swallowed it); a second `F1` closes the box and the
/// picker is still open on the candidate it was on.
#[tokio::test]
async fn f1_opens_help_from_the_reqs_picker_and_it_stays() {
    let mut harness = backlog().await;
    on_feat_1_reqs(&mut harness).await;
    harness.key("c");
    harness.drive_to_end().await;
    harness.key("j");
    let frame = harness.render();
    assert!(
        frame.contains("▸ R-ENT-2"),
        "the picker, its cursor on R-ENT-2:\n{frame}"
    );

    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` opened the box");
    harness.key("f1");
    assert!(!harness.app().help_visible, "`F1` closed it");
    let frame = harness.render();
    assert!(
        frame.contains("▸ R-ENT-2") && frame.contains("Enter pick"),
        "the picker is still open on R-ENT-2:\n{frame}"
    );
}

/// D7: the picker's cursor moves on `backlog.reqs.next_row`/`prev_row` (`J`/`K`, the pane's
/// cursor verbs) and on `list.down`/`up` (`j`/`k`, `Down`/`Up`): two acts, one move.
#[tokio::test]
async fn the_picker_moves_on_j_and_j_capital() {
    let mut harness = backlog().await;
    on_feat_1_reqs(&mut harness).await;
    harness.key("c");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("▸ R-ENT-1"),
        "the picker opens on its first candidate:\n{frame}"
    );
    for (down, up) in [("J", "K"), ("j", "k"), ("down", "up")] {
        harness.key(down);
        let frame = harness.render();
        assert!(
            frame.contains("▸ R-ENT-2"),
            "`{down}` moved the picker down:\n{frame}"
        );
        harness.key(up);
        let frame = harness.render();
        assert!(
            frame.contains("▸ R-ENT-1"),
            "`{up}` moved it back:\n{frame}"
        );
    }
}

/// The lane's rebinding test (D8): `[backlog.reqs] cite = "C"` moves the picker off `c`. `C`
/// passes the unconverted Backlog tab and the converted one alike (no `backlog`, `list` or `pane`
/// act binds it), so the pane resolves it; `c` opens nothing, and the browse hint names `C`.
#[tokio::test]
async fn a_rebound_cite_key_opens_the_picker_and_c_is_inert() {
    let keys =
        htui::keys::load_str("version = 1\n[backlog.reqs]\ncite = \"C\"\n").expect("the keys load");
    let mut harness = backlog_with(keys).await;
    on_feat_1_reqs(&mut harness).await;
    let frame = harness.render();
    assert!(
        frame.contains("J/K move · r reconfirm · C cite · u uncite"),
        "the browse hint names the rebound key:\n{frame}"
    );

    harness.key("c");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        !frame.contains("Enter pick") && !frame.contains("▸ R-ENT-1"),
        "`c` opened nothing:\n{frame}"
    );
    assert_eq!(harness.app().status, None, "and said nothing");

    harness.key("C");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("▸ R-ENT-1") && frame.contains("j/k move · Enter pick · Esc back"),
        "`C` opened the picker:\n{frame}"
    );
}
