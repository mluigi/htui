//! Lane L-A of MOD-67 M5: the Backlog tab, its filter, item form and divergence, Notes and Docs.
//! Each case drives the real shell (`register_all`) over the demo fixture, scoped to `Platform`,
//! so a key reaches the tab, the pane and the shell's stack exactly as at run time (D12).

use htui::agent_worker::AgentRuntime;
use htui::app::{Action, register_all};
use htui::keys::{Keys, load_str};
use htui::testkit::Harness;
use htui_agent::registry::DriverFactory;
use htui_core::model::WorkspaceSummary;
use htui_core::store::MemStore;

/// The `?` box's closing line.
const CLOSER: &str = "?/F1 closes this box";

/// The D6 miss under the default keys.
const DEFAULT_MISS: &str = "select a step in the Runs pane (J/K) to replay it";

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

/// A key file's text as `Keys`, `version = 1` prepended.
fn keys(table: &str) -> Keys {
    load_str(&format!("version = 1\n\n{table}")).expect("the key file loads")
}

/// The registered demo shell under `keys` (the defaults with `None`), scoped to `Platform`: the
/// cursor on htui `ANA-1`, Body active. The agent runtime keeps prompt previews off the status
/// line (`tests/backlog.rs`).
async fn shell(keys: Option<Keys>) -> Harness {
    let mut harness = Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    if let Some(keys) = keys {
        harness = harness.with_keys(keys);
    }
    register_all(harness.app());
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// Presses `chord`, then serves what it asked for.
async fn press(harness: &mut Harness, chord: &str) {
    harness.key(chord);
    harness.drive_to_end().await;
}

/// D12: chord equality. A CONTROL or ALT chord on a list or sub-tab letter is no act in
/// `BACKLOG_BODY`, so it neither moves the cursor nor cycles the strip; the plain letters still do.
#[tokio::test]
async fn ctrl_l_alt_l_and_alt_j_do_not_move_the_backlog() {
    let mut harness = shell(None).await;
    let before = harness.render();
    assert!(before.contains("┌ ANA-1"), "ANA-1 on arrival:\n{before}");
    for chord in ["ctrl-l", "alt-l", "alt-]", "alt-j", "ctrl-j"] {
        press(&mut harness, chord).await;
        assert_eq!(harness.render(), before, "`{chord}` changed nothing");
        assert_eq!(harness.app().status, None, "`{chord}` said nothing");
    }

    press(&mut harness, "l").await;
    let runs = harness.render();
    assert_ne!(runs, before, "`l` cycles the strip");
    assert!(runs.contains("┌ ANA-1"), "and keeps the row:\n{runs}");
    press(&mut harness, "h").await;
    press(&mut harness, "j").await;
    assert!(
        harness.render().contains("┌ ANA-2"),
        "`j` moves the row:\n{}",
        harness.render()
    );
}

/// D6, D12: the miss is the tab's `backlog.replay`, and it names the Runs step keys through the
/// Runs browse stack, so a rebound step key is the one it names.
#[tokio::test]
async fn the_replay_miss_names_a_rebound_step_key() {
    let mut harness = shell(Some(keys("[backlog.runs]\nnext_step = \"n\"\n"))).await;
    press(&mut harness, "enter").await;
    assert_eq!(
        harness.app().status.as_deref(),
        Some("select a step in the Runs pane (n/K) to replay it")
    );
}

/// D6, PA-2: on a project header `Enter` folds before any pane sees it, and says nothing; on an
/// item row with Body active the fold declines, Body declines, and the tab answers the miss.
#[tokio::test]
async fn enter_on_a_header_still_folds_and_on_body_misses() {
    let mut harness = shell(None).await;
    press(&mut harness, "k").await;
    press(&mut harness, "enter").await;
    let folded = harness.render();
    assert!(!folded.contains("TUI scaffold"), "folded:\n{folded}");
    assert_eq!(harness.app().status, None, "a fold says nothing");

    press(&mut harness, "enter").await;
    press(&mut harness, "j").await;
    assert!(harness.render().contains("┌ ANA-1"));
    press(&mut harness, "enter").await;
    assert_eq!(harness.app().status.as_deref(), Some(DEFAULT_MISS));
}

/// D12, PA-3: the `?` box lists the focused pane's stack, one line per layer: the pane's
/// scrolling, the Backlog's own verbs (no legacy `Keymap` line), `e` under `Common`, the list's
/// moves and fold, then the global keys.
#[tokio::test]
async fn the_backlog_help_box_lists_the_tab_and_the_list() {
    let mut harness = shell(None).await;
    harness.key("?");
    let frame = harness.render();
    for wanted in [
        "Pane: J scroll down",
        "Backlog: Enter replay step · m open graph · f filter · F clear filter · N new item",
        "Q queue / dequeue · P pause / resume queue",
        "Common: e edit",
        "List: ",
        "Enter fold",
        "Global: ",
        CLOSER,
    ] {
        assert!(frame.contains(wanted), "{wanted:?} in the box:\n{frame}");
    }
    assert!(
        !frame.contains("e edit item"),
        "the legacy line is gone:\n{frame}"
    );
}

/// D1, D12: `[backlog] filter` moves the filter key on every sub-tab; the old `f` is inert.
#[tokio::test]
async fn a_backlog_table_moves_the_filter_key() {
    let mut harness = shell(Some(keys("[backlog]\nfilter = \"/\"\n"))).await;
    press(&mut harness, "f").await;
    assert!(
        !harness.render().contains(" Filter "),
        "`f` no longer opens the form:\n{}",
        harness.render()
    );
    press(&mut harness, "/").await;
    assert!(
        harness.render().contains(" Filter "),
        "`/` opens it:\n{}",
        harness.render()
    );
}

/// The status line of a capturing mode under `register_all` (D9).
const FILTERED: &str = "Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue";

/// The status line: the frame's last row, trailing blanks dropped.
fn status_line(harness: &mut Harness) -> String {
    harness
        .render()
        .lines()
        .last()
        .unwrap_or_default()
        .trim_end()
        .to_owned()
}

/// User-visible item 3: `F1` opens help from the item form (it swallowed function keys), the box
/// lists the form's own layer, and a second `F1` closes it over the form, text kept.
#[tokio::test]
async fn f1_opens_help_from_the_item_form_and_the_form_stays() {
    let mut harness = shell(None).await;
    press(&mut harness, "N").await;
    for c in ["F", "r", "e", "s", "h"] {
        harness.key(c);
    }
    assert!(
        harness.render().contains("> title    Fresh"),
        "{}",
        harness.render()
    );

    press(&mut harness, "f1").await;
    let help = harness.render();
    assert!(help.contains("Form: "), "the form's layer:\n{help}");
    assert!(help.contains("F1 closes this box"), "{help}");

    press(&mut harness, "f1").await;
    let frame = harness.render();
    assert!(!frame.contains("F1 closes this box"), "closed:\n{frame}");
    assert!(frame.contains(" New item "), "the form stays:\n{frame}");
    assert!(
        frame.contains("> title    Fresh"),
        "with its text:\n{frame}"
    );
}

/// D9: a capturing Backlog form reports its modal stack, so the status line is the filtered one:
/// `Ctrl+c quit · F1 help`, then the chords `register_all` offers (`Ctrl+f`, `Ctrl+w`, `Ctrl+q`),
/// and none of the browse keys (`q`, `Tab`, digits, `?`, `w`).
#[tokio::test]
async fn a_capturing_backlog_form_shows_the_filtered_status_line() {
    let mut harness = shell(None).await;
    let browsing = status_line(&mut harness);
    assert!(browsing.starts_with("q quit · Tab next tab"), "{browsing}");
    press(&mut harness, "N").await;
    assert!(harness.render().contains(" New item "));
    assert_eq!(status_line(&mut harness), FILTERED, "item form");

    press(&mut harness, "esc").await;
    press(&mut harness, "f").await;
    assert!(harness.render().contains(" Filter "));
    assert_eq!(status_line(&mut harness), FILTERED, "filter form");
}
