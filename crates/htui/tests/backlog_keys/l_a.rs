//! Lane L-A of MOD-67 M5: the Backlog tab, its filter, item form and divergence, Notes and Docs.
//! Each case drives the real shell (`register_all`) over the demo fixture, scoped to `Platform`,
//! so a key reaches the tab, the pane and the shell's stack exactly as at run time (D12).

use htui::agent_worker::AgentRuntime;
use htui::app::{Action, register_all};
use htui::keys::{Keys, load_str};
use htui::testkit::Harness;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{ItemId, WorkspaceSummary};
use htui_core::store::{MemStore, ReadStore as _};

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
    shell_over(MemStore::demo(), keys).await
}

/// [`shell`] over `store`, so a test can read what landed.
async fn shell_over(store: MemStore, keys: Option<Keys>) -> Harness {
    let mut harness =
        Harness::over(store).with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
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

/// Rows down from `ANA-1` to htui `FEAT-1`, the item Notes and Docs have data for.
const TO_FEAT_1: usize = 3;

/// Sub-tabs right of Body to Docs; Notes is one further.
const TO_DOCS: usize = 3;

/// Presses each of `chords`, then serves what they asked for.
async fn presses(harness: &mut Harness, chords: &[&str]) {
    for chord in chords {
        harness.key(chord);
    }
    harness.drive_to_end().await;
}

/// Types `text` key by key, a space as `space`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        match c {
            ' ' => harness.key("space"),
            c => harness.key(&c.to_string()),
        }
    }
}

/// `item`'s head version in `store`.
async fn version(store: &MemStore, item: ItemId) -> i32 {
    store
        .item(item)
        .await
        .expect("the memory store never fails")
        .expect("the demo item exists")
        .version
}

/// `FEAT-1`'s note and document counts in `store`.
async fn written(store: &MemStore) -> (usize, usize) {
    let notes = store.notes(ids::HTUI_FEAT_1).await.expect("notes").len();
    let documents = store
        .documents(ids::HTUI_FEAT_1)
        .await
        .expect("documents")
        .len();
    (notes, documents)
}

/// D8, D12: `[form] save = "f2"` saves the item form, a note and a document (`form.save` resolves
/// after the widget, through `ITEM_FORM`, `NOTES_COMPOSE` and `DOCS_COMPOSE`); the old `Ctrl+S`
/// saves nothing, types nothing and leaves each open; each hint names `F2 save`.
#[tokio::test]
async fn a_rebound_save_saves_the_item_form_a_note_and_a_document() {
    let store = MemStore::demo();
    let mut harness = shell_over(store.clone(), Some(keys("[form]\nsave = \"f2\"\n"))).await;

    press(&mut harness, "e").await;
    type_text(&mut harness, "Z");
    press(&mut harness, "ctrl-s").await;
    let frame = harness.render();
    assert!(frame.contains(" Edit ANA-1 "), "still open:\n{frame}");
    assert!(
        frame.contains("Tab field · F2 save · Esc cancel"),
        "{frame}"
    );
    assert!(!frame.contains("Zs"), "`ctrl-s` typed nothing:\n{frame}");
    assert_eq!(
        version(&store, ids::HTUI_ANA_1).await,
        1,
        "`ctrl-s` saved nothing"
    );
    press(&mut harness, "f2").await;
    assert_eq!(
        version(&store, ids::HTUI_ANA_1).await,
        2,
        "`F2` saved the edit"
    );

    for _ in 0..TO_FEAT_1 {
        press(&mut harness, "j").await;
    }
    for _ in 0..=TO_DOCS {
        press(&mut harness, "l").await;
    }
    let (notes, documents) = written(&store).await;
    press(&mut harness, "a").await;
    harness.paste("A note by hand.");
    press(&mut harness, "ctrl-s").await;
    let frame = harness.render();
    assert!(frame.contains(" New note "), "still open:\n{frame}");
    assert!(
        frame.contains("A note by hand.") && !frame.contains("A note by hand.s"),
        "`ctrl-s` typed nothing into the note:\n{frame}"
    );
    assert!(
        frame.contains("Ctrl+e $EDITOR · F2 save · Esc cancel"),
        "{frame}"
    );
    assert_eq!(
        written(&store).await,
        (notes, documents),
        "`ctrl-s` wrote nothing"
    );
    press(&mut harness, "f2").await;
    assert_eq!(
        written(&store).await,
        (notes + 1, documents),
        "`F2` added the note"
    );

    press(&mut harness, "h").await;
    press(&mut harness, "a").await;
    type_text(&mut harness, "summary");
    let frame = harness.render();
    assert!(
        frame.contains("Tab field · F2 save · Esc cancel"),
        "{frame}"
    );
    presses(&mut harness, &["tab"]).await;
    type_text(&mut harness, "Summary of FEAT-1");
    presses(&mut harness, &["tab"]).await;
    harness.paste("The summary.");
    press(&mut harness, "ctrl-s").await;
    let frame = harness.render();
    assert!(frame.contains(" New document "), "still open:\n{frame}");
    assert!(
        frame.contains("The summary.") && !frame.contains("The summary.s"),
        "`ctrl-s` typed nothing into the body:\n{frame}"
    );
    assert!(
        frame.contains("Ctrl+e $EDITOR · F2 save · Esc cancel"),
        "{frame}"
    );
    assert_eq!(
        written(&store).await,
        (notes + 1, documents),
        "`ctrl-s` wrote nothing"
    );
    press(&mut harness, "f2").await;
    assert_eq!(
        written(&store).await,
        (notes + 1, documents + 1),
        "`F2` wrote the document"
    );
}

/// D12: `[form] external_editor = "f3"` hands the item form's Body and a note's text to
/// `$EDITOR`; the old `Ctrl+E` asks for nothing.
#[tokio::test]
async fn a_rebound_external_editor_hands_the_body_out() {
    let rebound = || Some(keys("[form]\nexternal_editor = \"f3\"\n"));
    let mut harness = shell(rebound()).await;
    press(&mut harness, "e").await;
    presses(&mut harness, &["tab"; 5]).await;
    press(&mut harness, "ctrl-e").await;
    assert!(
        harness.app().take_external_edit().is_none(),
        "`ctrl-e` is inert"
    );
    press(&mut harness, "f3").await;
    let (_, edit) = harness
        .app()
        .take_external_edit()
        .expect("`F3` asked for the editor");
    assert_eq!(edit.stem, "ANA-1-body");

    let mut harness = shell(rebound()).await;
    for _ in 0..TO_FEAT_1 {
        press(&mut harness, "j").await;
    }
    for _ in 0..=TO_DOCS {
        press(&mut harness, "l").await;
    }
    press(&mut harness, "a").await;
    harness.paste("A note by hand.");
    press(&mut harness, "ctrl-e").await;
    assert!(
        harness.app().take_external_edit().is_none(),
        "`ctrl-e` is inert"
    );
    press(&mut harness, "f3").await;
    let (_, edit) = harness
        .app()
        .take_external_edit()
        .expect("`F3` asked for the editor");
    assert_eq!(edit.text, "A note by hand.");
}

/// The shell's hint row (MOD-67 M6 T2): the frame's second-to-last row, trailing blanks dropped.
fn hint_row(frame: &str) -> String {
    let rows: Vec<&str> = frame.lines().collect();
    rows[rows.len() - 2].trim_end().to_owned()
}

/// The filter form's four rows, as drawn in the list pane: borders and padding off (its keys are
/// the shell's hint row since MOD-67 M6 T2).
fn filter_rows(frame: &str) -> Vec<String> {
    let lines: Vec<&str> = frame.lines().collect();
    let title = lines
        .iter()
        .position(|line| line.contains("┌ Filter "))
        .unwrap_or_else(|| panic!("the filter form is open:\n{frame}"));
    lines[title + 1..=title + 4]
        .iter()
        .map(|line| {
            let left = line.split("││").next().unwrap_or_default();
            left.trim_start_matches('│')
                .trim_end_matches(['│', ' '])
                .to_owned()
        })
        .collect()
}

/// The focused row's label (the row drawn with `>`).
fn focused(harness: &mut Harness) -> String {
    let rows = filter_rows(&harness.render());
    rows.iter()
        .find_map(|row| row.strip_prefix("> "))
        .and_then(|row| row.split_whitespace().next())
        .unwrap_or_else(|| panic!("a focused row in {rows:#?}"))
        .to_owned()
}

/// D5, borderline keys (blueprint §1): an option row's value keys are the form's own, plain keys
/// only (`alt-l`, `alt-x` change nothing); `x` is `backlog.filter_form.clear`; `j`, `Down` and
/// `Tab` move the row. On the Tags row `j` and `x` are typed, and `Down`/`Tab` move on.
#[tokio::test]
async fn the_filter_form_keeps_its_value_keys_and_x_clears() {
    let mut harness = shell(None).await;
    press(&mut harness, "f").await;
    let status = |harness: &mut Harness| filter_rows(&harness.render())[0].clone();
    assert_eq!(status(&mut harness), "> status  ‹[ ] open›  any");

    press(&mut harness, "l").await;
    let queued = status(&mut harness);
    assert_eq!(
        queued, "> status  ‹[ ] queued›  any",
        "`l` moves the option"
    );
    press(&mut harness, "right").await;
    let third = status(&mut harness);
    assert_ne!(third, queued, "`Right` moves it on");
    for chord in ["alt-l", "alt-h", "alt-space"] {
        press(&mut harness, chord).await;
        assert_eq!(status(&mut harness), third, "`{chord}` changed nothing");
    }
    press(&mut harness, "space").await;
    let ticked = status(&mut harness);
    assert!(ticked.contains("‹[x] "), "Space ticks: {ticked}");
    press(&mut harness, "alt-x").await;
    assert_eq!(status(&mut harness), ticked, "`alt-x` cleared nothing");
    press(&mut harness, "x").await;
    let cleared = status(&mut harness);
    assert!(
        cleared.contains("‹[ ] ") && cleared.ends_with("any"),
        "`x` clears: {cleared}"
    );

    press(&mut harness, "j").await;
    assert_eq!(focused(&mut harness), "project", "`j`");
    press(&mut harness, "k").await;
    press(&mut harness, "down").await;
    assert_eq!(focused(&mut harness), "project", "`Down`");
    press(&mut harness, "tab").await;
    assert_eq!(focused(&mut harness), "tags", "`Tab`");

    press(&mut harness, "j").await;
    press(&mut harness, "x").await;
    let frame = harness.render();
    let rows = filter_rows(&frame);
    assert_eq!(rows[2], "> tags    jx", "typed, not moved or cleared");
    assert_eq!(
        hint_row(&frame),
        "comma list · Tab row · Enter apply · Esc cancel"
    );
    press(&mut harness, "down").await;
    assert_eq!(focused(&mut harness), "ready", "`Down` moves on");
    press(&mut harness, "backtab").await;
    assert_eq!(focused(&mut harness), "tags", "`BackTab` comes back");
    press(&mut harness, "tab").await;
    assert_eq!(focused(&mut harness), "ready", "`Tab` moves on");
}

/// The lane's rebinding test (D12): `[backlog.docs] new_version = "V"` moves the Docs pane's
/// new-version key, so the pane-first Backlog hands `V` to Docs, the old `v` opens nothing, and
/// the Docs hint names the new key.
#[tokio::test]
async fn a_rebound_new_version_key_opens_the_form_and_v_is_inert() {
    let mut harness = shell(Some(keys("[backlog.docs]\nnew_version = \"V\"\n"))).await;
    for _ in 0..TO_FEAT_1 {
        press(&mut harness, "j").await;
    }
    for _ in 0..TO_DOCS {
        press(&mut harness, "l").await;
    }
    let frame = harness.render();
    assert!(
        frame.contains("J/K move · a new · V new version"),
        "the Docs hint:\n{frame}"
    );
    press(&mut harness, "v").await;
    let frame = harness.render();
    assert!(
        !frame.contains(" New version of "),
        "`v` is inert:\n{frame}"
    );
    press(&mut harness, "V").await;
    let frame = harness.render();
    assert!(
        frame.contains(" New version of plan (from v2) "),
        "`V` opens the form on the cursor's plan:\n{frame}"
    );
}
