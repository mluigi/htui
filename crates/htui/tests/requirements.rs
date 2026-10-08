//! The Requirements tab (MOD-39 T3, PRD D2, plan P10-P11), through a `Harness` over the demo world.
//!
//! The Harness enters the Graphics workspace at startup; every requirement of the demo is on `htui`,
//! so each case switches to Platform (`htui`, `agy`) first, as `skills.rs`' attachment cases do.
//! `htui` has a spec owned by the demo's user, areas `ENT` and `STO`, and `R-ENT-1` (v2, amended by
//! `ANA-2`, so `ANA-1`'s v1 citation is suspect), `R-ENT-2` and `R-STO-1`; `agy` has no spec.
//!
//! A direct write goes into the same `MemStore` through a clone (`MemStore` shares its state across
//! clones). Every request the tab makes goes through `StoreRequest`, so "nothing was sent" is
//! checked on the store: a sent write is served by `settle` and would have changed a row.
//!
//! `MemStore` stamps a write with the wall clock, so a frame that shows a revision written here
//! goes through [`STAMP_FILTER`] before it is compared.
#![cfg(feature = "testkit")]

use chrono::TimeDelta;
use htui::app::register_all;
use htui::requirements::not_the_maintainer;
use htui::testkit::Harness;
use htui::ui::tabs::RequirementsTab;
use htui_core::fixtures::{self, ids};
use htui_core::model::{
    AppUser, Priority, RequirementFilter, RequirementPatch, RequirementState, RequirementUpdate,
    UserId,
};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

/// The detail pane's `%m-%d %H:%M` stamps: a revision written by a case carries today's.
const STAMP_FILTER: (&str, &str) = (r"\d{2}-\d{2} \d{2}:\d{2}", "MM-DD hh:mm");

/// The shell over `store` in the Platform workspace (`htui`, `agy`), on the Requirements tab.
async fn open_platform_over(store: MemStore) -> Harness {
    open_platform_in(Harness::over(store)).await
}

/// `harness` moved to the Platform workspace, on the Requirements tab.
async fn open_platform_in(mut harness: Harness) -> Harness {
    register_all(harness.app());
    harness.settle().await;
    harness.key("w");
    harness.settle().await;
    harness.key("j");
    harness.key("enter");
    harness.settle().await;
    assert_eq!(harness.app().top_bar.workspace, "Platform");
    harness.key("3");
    harness.settle().await;
    assert_eq!(harness.app().tabs.active_id(), Some(RequirementsTab::ID));
    harness
}

/// The demo world on the Requirements tab.
async fn open() -> Harness {
    open_platform_over(MemStore::demo()).await
}

/// The demo with a second user created a day before the fixture's, so `MemStore::this_user` (the
/// earliest row) is the stranger and `htui`'s spec owner is not.
fn stranger_first() -> MemStore {
    let mut data = fixtures::demo_data();
    let at = fixtures::demo_at(0, 0) - TimeDelta::days(1);
    data.users.push(AppUser {
        id: UserId::new(),
        name: "stranger".to_owned(),
        email: None,
        created_at: at,
        updated_at: at,
    });
    MemStore::from_demo(data)
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

/// Moves the tree's cursor onto the requirement `key`: to the top, then down until the detail pane
/// is titled with it.
fn select(harness: &mut Harness, key: &str) {
    harness.key("g");
    let title = format!("\u{250c} {key} ");
    for _ in 0..20 {
        if harness.render().contains(&title) {
            return;
        }
        harness.key("j");
    }
    panic!("`{key}` is not in the tree:\n{}", harness.render());
}

/// The status line: the frame's last line.
fn status(frame: &str) -> String {
    frame.lines().last().unwrap_or_default().to_owned()
}

/// The notice: the rows between the content's bottom border and the hint, joined (it wraps).
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

/// Every requirement of `htui`, every state.
async fn htui_requirements(store: &MemStore) -> Vec<htui_core::model::Requirement> {
    store
        .requirements(ids::PROJECT_HTUI, &RequirementFilter::default())
        .await
        .expect("the memory store never fails")
}

#[tokio::test]
async fn the_tab_is_third_in_the_strip() {
    let mut harness = open().await;
    let frame = harness.render();
    assert!(
        frame.contains(" 1 Backlog  2 Skills  3 Requirements  4 Settings  5 Chat"),
        "{frame}"
    );
}

#[tokio::test]
async fn the_tab_shows_the_tree_and_r_ent_1_detail() {
    let mut harness = open().await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{250c} R-ENT-1 "),
        "R-ENT-1 is selected: {frame}"
    );
    assert!(frame.contains("ANA-1"), "coverage lists ANA-1: {frame}");
    assert!(
        frame.contains("! suspect"),
        "ANA-1's v1 citation is suspect: {frame}"
    );
    assert!(
        frame.contains("by ANA-2"),
        "v2 was amended by ANA-2: {frame}"
    );
    insta::assert_snapshot!("requirements_tree", frame);
}

/// The PRD metric "ANA-1's citation shows a suspect marker" on a standard 80x24 terminal: the
/// detail pane is 42 columns there and draws without wrapping, so the marker must not be last.
#[tokio::test]
async fn the_suspect_marker_fits_an_80_column_terminal() {
    let mut harness = open_platform_in(Harness::over(MemStore::demo()).size(80, 24)).await;
    let frame = harness.render();
    let row = frame
        .lines()
        .find(|line| line.contains("ANA-1  addresses"))
        .unwrap_or_else(|| panic!("ANA-1's citation is drawn:\n{frame}"));
    assert!(row.contains("! suspect"), "{frame}");
}

#[tokio::test]
async fn slash_filters_over_key_and_body() {
    let mut harness = open().await;
    harness.key("/");
    type_text(&mut harness, "STO");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("R-STO-1"), "{frame}");
    assert!(
        !frame.contains("R-ENT-2"),
        "R-ENT-2 matches neither key nor body: {frame}"
    );
    assert!(
        frame.contains("\u{250c} R-STO-1 "),
        "Enter re-selects onto a visible row: {frame}"
    );
    insta::assert_snapshot!("requirements_filter", frame);
}

#[tokio::test]
async fn n_mints_the_next_key_in_the_area() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    harness.key("n");
    assert!(
        harness.render().contains(" New requirement in ENT "),
        "{}",
        harness.render()
    );
    type_text(&mut harness, "Every item has a title.");
    harness.key("tab");
    type_text(&mut harness, "Titles are what people read.");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("minted R-ENT-3"), "{frame}");
    assert!(
        frame.contains("\u{250c} R-ENT-3 "),
        "the minted row is selected: {frame}"
    );
    let minted = htui_requirements(&store)
        .await
        .into_iter()
        .find(|row| row.key == "R-ENT-3")
        .expect("R-ENT-3 is in the store");
    assert_eq!(minted.body, "Every item has a title.");
    assert_eq!(minted.rationale, "Titles are what people read.");
    assert_eq!(minted.priority, Priority::Must);
    insta::with_settings!({ filters => vec![STAMP_FILTER] }, {
        insta::assert_snapshot!("requirements_minted", frame);
    });
}

#[tokio::test]
async fn e_amends_with_a_deciding_item() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    harness.key("e");
    assert!(
        harness.render().contains(" Amend R-ENT-1 (v2) "),
        "{}",
        harness.render()
    );
    harness.key("end");
    type_text(&mut harness, " Keys never change.");
    harness.key("tab");
    harness.key("tab");
    harness.key("tab");
    type_text(&mut harness, "ANA-2");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("amended R-ENT-1 to v3"), "{frame}");
    assert!(frame.contains("must \u{b7} active \u{b7} v3"), "{frame}");
    assert!(frame.contains("! suspect"), "ANA-1 stays suspect: {frame}");
    let amended = store
        .requirement(ids::REQ_ENT_1)
        .await
        .expect("the memory store never fails")
        .expect("R-ENT-1 exists");
    assert_eq!(amended.version, 3);
    assert_eq!(
        amended.body,
        "Every item has a stable key of the form PREFIX-N. Keys never change."
    );
    insta::with_settings!({ filters => vec![STAMP_FILTER] }, {
        insta::assert_snapshot!("requirements_amended", frame);
    });
}

#[tokio::test]
async fn shift_w_withdraws_after_the_key_is_typed_back() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    select(&mut harness, "R-ENT-2");
    harness.key("W");
    assert!(harness.render().contains(" Withdraw R-ENT-2 "));
    type_text(&mut harness, "ANA-2");
    harness.key("enter");
    // A wrong key is refused and the field clears.
    type_text(&mut harness, "R-ENT-1");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("that is not the requirement's key"),
        "{frame}"
    );
    type_text(&mut harness, "R-ENT-2");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("withdrew R-ENT-2"), "{frame}");
    assert!(
        frame.contains("R-ENT-2"),
        "a withdrawn row stays listed: {frame}"
    );
    assert!(
        frame.contains("later \u{b7} withdrawn \u{b7} v2"),
        "{frame}"
    );
    let withdrawn = store
        .requirement(ids::REQ_ENT_2)
        .await
        .expect("the memory store never fails")
        .expect("R-ENT-2 exists");
    assert_eq!(withdrawn.state, RequirementState::Withdrawn);
    insta::with_settings!({ filters => vec![STAMP_FILTER] }, {
        insta::assert_snapshot!("requirements_withdrawn", frame);
    });
}

#[tokio::test]
async fn a_creates_an_area() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    harness.key("a");
    assert!(harness.render().contains(" New area in htui "));
    type_text(&mut harness, "UI");
    harness.key("tab");
    type_text(&mut harness, "User interface");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("added area UI"), "{frame}");
    assert!(frame.contains("UI User interface (0)"), "{frame}");
    let areas = store
        .requirement_areas(ids::PROJECT_HTUI)
        .await
        .expect("the memory store never fails");
    assert!(
        areas
            .iter()
            .any(|area| area.code == "UI" && area.position == 2)
    );
    insta::assert_snapshot!("requirements_new_area", frame);
}

#[tokio::test]
async fn a_non_maintainer_is_answered_on_the_status_line() {
    let store = stranger_first();
    let before = htui_requirements(&store).await;
    let mut harness = open_platform_over(store.clone()).await;
    harness.key("n");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        status(&frame).contains(&not_the_maintainer("htui")),
        "{frame}"
    );
    assert!(
        frame.contains("read-only"),
        "the htui header says so: {frame}"
    );
    assert!(
        !frame.contains("New requirement"),
        "no form opened: {frame}"
    );
    assert_eq!(
        htui_requirements(&store).await,
        before,
        "nothing was written"
    );
    insta::assert_snapshot!("requirements_read_only", frame);
}

#[tokio::test]
async fn a_stale_amend_keeps_the_form() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    harness.key("e");
    harness.key("end");
    type_text(&mut harness, " Mine.");
    harness.key("tab");
    harness.key("tab");
    harness.key("tab");
    type_text(&mut harness, "ANA-2");

    // Someone else amends R-ENT-1 while the form is open.
    let patch = RequirementPatch {
        body: Some("Theirs.".to_owned()),
        rationale: None,
        priority: None,
        author_id: ids::USER,
        box_id: None,
        reason: "amended".to_owned(),
    };
    let update = store
        .amend_requirement(ids::REQ_ENT_1, 2, patch, ids::HTUI_ANA_2)
        .await
        .expect("the direct amend applies");
    assert!(matches!(update, RequirementUpdate::Updated(_)));

    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("now v3"), "{frame}");
    assert!(
        frame.contains(" Amend R-ENT-1 (v3) "),
        "the form stays open: {frame}"
    );
    assert!(frame.contains("Mine."), "the text is kept: {frame}");

    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("amended R-ENT-1 to v4"), "{frame}");
    let head = store
        .requirement(ids::REQ_ENT_1)
        .await
        .expect("the memory store never fails")
        .expect("R-ENT-1 exists");
    assert_eq!(head.version, 4);
    assert!(head.body.ends_with("Mine."), "{}", head.body);
}

/// The hint row: the frame's line above the status line.
fn hint(frame: &str) -> String {
    let lines: Vec<&str> = frame.lines().collect();
    lines[lines.len() - 2].to_owned()
}

/// The demo on the Requirements tab with `file` as the keys file (MOD-67 M4).
async fn open_with_keys(file: &str) -> Harness {
    let keys = htui::keys::load_str(file).expect("the keys load");
    open_platform_in(Harness::over(MemStore::demo()).with_keys(keys)).await
}

/// MOD-67 M4 (D6, PA-3): a rebound `form.save` saves a mint from the body, the form hint names
/// it, and `ctrl-s` (a chord, which never reaches the `TextArea`) no longer saves.
#[tokio::test]
async fn a_rebound_save_saves_a_requirements_draft() {
    let mut harness = open_with_keys("version = 1\n[form]\nsave = \"f2\"\n").await;
    harness.key("n");
    type_text(&mut harness, "Every item has a title.");
    let frame = harness.render();
    assert_eq!(
        hint(&frame),
        " Tab field \u{b7} F2 save \u{b7} Esc cancel",
        "{frame}"
    );
    harness.key("ctrl-s");
    assert_eq!(harness.queued(), 0, "`ctrl-s` sends nothing");
    assert!(
        harness.render().contains("Every item has a title."),
        "the draft is kept"
    );
    harness.key("f2");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("minted R-ENT-3"), "{frame}");
}

/// MOD-67 M4 (D10, risk 6): Priority's value keys stay the field's, and a chord never reaches it.
#[tokio::test]
async fn the_priority_field_keeps_its_value_keys_and_ignores_chords() {
    let mut harness = open().await;
    harness.key("n");
    harness.key("tab");
    harness.key("tab");
    let frame = harness.render();
    assert!(frame.contains("[must]"), "{frame}");
    assert!(hint(&frame).ends_with(" \u{b7} m/l priority"), "{frame}");
    harness.key("l");
    assert!(harness.render().contains("[later]"), "`l` is later");
    harness.key("m");
    assert!(harness.render().contains("[must]"), "`m` is must");
    harness.key("space");
    assert!(harness.render().contains("[later]"), "`space` toggles");
    harness.key("left");
    assert!(harness.render().contains("[must]"), "`left` toggles");
    harness.key("right");
    assert!(harness.render().contains("[later]"), "`right` toggles");
    harness.key("right");
    assert!(harness.render().contains("[must]"), "`right` toggles back");
    harness.key("ctrl-l");
    assert!(harness.render().contains("[must]"), "`ctrl-l` is not `l`");
    harness.key("l");
    harness.key("alt-m");
    let frame = harness.render();
    assert!(frame.contains("[later]"), "`alt-m` is not `m`: {frame}");
    assert!(
        frame.contains(" New requirement in ENT "),
        "the form is still open: {frame}"
    );
}

/// MOD-67 M4 (D5; L-C §2.3 (b), review low 2): an ALT chord from a Requirements form passes to
/// the shell, which resolves it through the form's stack: with `[global] quit` also on `alt-q`,
/// `alt-q` quits from the body (a `TextArea`) and from Priority, where a plain `q` stays the
/// form's.
#[tokio::test]
async fn an_alt_chord_from_a_form_reaches_the_shell() {
    for field_tabs in [0, 2] {
        let mut harness =
            open_with_keys("version = 1\n[global]\nquit = [\"q\", \"alt-q\"]\n").await;
        harness.key("n");
        for _ in 0..field_tabs {
            harness.key("tab");
        }
        let before = harness.render();
        assert!(
            before.contains(" New requirement in ENT "),
            "the form is open: {before}"
        );
        harness.key("q");
        assert!(!harness.app().should_quit, "`q` stays the form's");
        harness.key("alt-q");
        assert!(
            harness.app().should_quit,
            "`alt-q` passed to the shell from field {field_tabs}"
        );
    }
}

/// MOD-67 M4 (§1 item 4, pinned on purpose): `form.next_field`/`prev_field` move the focus,
/// `Down`/`Up` included (the `requirements` view defaults), on every Requirements form; the
/// `TextArea` fields keep `Up`/`Down` for themselves.
#[tokio::test]
async fn tab_and_down_move_the_form_focus() {
    let mut harness = open().await;
    harness.key("a");
    type_text(&mut harness, "AB");
    harness.key("tab");
    type_text(&mut harness, "T1");
    harness.key("backtab");
    type_text(&mut harness, "C");
    harness.key("down");
    type_text(&mut harness, "2");
    harness.key("up");
    type_text(&mut harness, "D");
    let frame = harness.render();
    assert!(frame.contains("ABCD"), "the code took A, B, C, D: {frame}");
    assert!(frame.contains("T12"), "the title took T, 1, 2: {frame}");
    harness.key("esc");

    // The mint form: Body → Rationale → Priority, and `Down` on Priority wraps to the body.
    harness.key("n");
    type_text(&mut harness, "b");
    harness.key("tab");
    type_text(&mut harness, "r");
    harness.key("tab");
    harness.key("l");
    assert!(harness.render().contains("[later]"), "on Priority");
    harness.key("down");
    type_text(&mut harness, "x");
    // The body's `TextArea` keeps `Up` (a line up); `Shift+Tab` wraps back to Priority.
    harness.key("up");
    harness.key("backtab");
    harness.key("m");
    let frame = harness.render();
    assert!(
        frame.contains("bx"),
        "`down` on Priority moved to the body: {frame}"
    );
    assert!(
        frame.contains("[must]"),
        "`backtab` on the body moved to Priority: {frame}"
    );
    harness.key("esc");

    // The amend form: `Down` on Priority moves to the deciding item, `Up` back.
    harness.key("e");
    harness.key("tab");
    harness.key("tab");
    harness.key("down");
    type_text(&mut harness, "ANA-9");
    harness.key("up");
    harness.key("l");
    let frame = harness.render();
    assert!(
        frame.contains("ANA-9"),
        "the deciding item took it: {frame}"
    );
    assert!(
        frame.contains("[later]"),
        "`up` moved back to Priority: {frame}"
    );
}

/// MOD-67 M4 (D5): `F1` opens the `?` box over a form and closes it; the form keeps its text.
#[tokio::test]
async fn f1_opens_help_from_a_form_and_the_form_stays() {
    let mut harness = open().await;
    harness.key("n");
    type_text(&mut harness, "hello");
    harness.key("f1");
    assert!(harness.app().help_visible, "`F1` opens the box");
    let frame = harness.render();
    assert!(frame.contains("Requirements: "), "{frame}");
    assert!(frame.contains("Form: "), "{frame}");
    assert!(frame.contains("F1 closes this box"), "{frame}");
    harness.key("f1");
    assert!(!harness.app().help_visible, "`F1` closes it");
    let frame = harness.render();
    assert!(frame.contains(" New requirement in ENT "), "{frame}");
    assert!(frame.contains("hello"), "{frame}");
}

/// MOD-67 M4: while a write is in flight a form swallows plain keys, `form.save` says the write is
/// in flight, and `ctrl-c` still quits.
#[tokio::test]
async fn a_write_in_flight_swallows_keys_but_save_says_so() {
    let mut harness = open().await;
    harness.key("n");
    type_text(&mut harness, "Body");
    harness.key("ctrl-s");
    assert_eq!(harness.queued(), 1, "the mint is in flight");
    harness.key("x");
    let frame = harness.render();
    assert!(!frame.contains("Bodyx"), "`x` was swallowed: {frame}");
    harness.key("ctrl-s");
    let frame = harness.render();
    assert!(notice(&frame).contains("is still in flight"), "{frame}");
    assert_eq!(harness.queued(), 1, "nothing more was sent");
    harness.key("ctrl-c");
    assert!(harness.app().should_quit, "`ctrl-c` still quits");
}

/// MOD-67 M4 (D11): browsing the tree, the status line is today's and the `?` box renders
/// `REQUIREMENTS_BROWSE`, one line per layer.
#[tokio::test]
async fn the_requirements_tree_status_line_and_help_box() {
    let mut harness = open().await;
    let frame = harness.render();
    assert_eq!(
        status(&frame),
        "q quit \u{b7} Tab next tab \u{b7} Shift+Tab previous tab \u{b7} 1 select tab \u{b7} ? help \
         \u{b7} w workspaces \u{b7} Ctrl+f find",
        "{frame}"
    );
    assert_eq!(
        hint(&frame),
        " j/k move \u{b7} Enter fold \u{b7} / filter \u{b7} a area \u{b7} n new \u{b7} e amend \
         \u{b7} W withdraw \u{b7} r reload",
        "{frame}"
    );
    harness.key("?");
    assert!(harness.app().help_visible);
    let frame = harness.render();
    for line in [
        "Requirements: a new area \u{b7} e amend \u{b7} W withdraw \u{b7} / filter",
        "Common: n new \u{b7} r reload \u{b7} Esc back",
        "List: j/Down down \u{b7} k/Up up \u{b7} g/Home top \u{b7} G/End bottom \u{b7} Enter fold",
        "Pane: J scroll down \u{b7} K scroll up \u{b7} PgDn page down \u{b7} PgUp page up",
        "Global: ",
        "?/F1 closes this box",
    ] {
        assert!(frame.contains(line), "`{line}` is in the box: {frame}");
    }
    assert!(
        !frame.contains("Requirements: j/k"),
        "no legacy tab line: {frame}"
    );
}

/// MOD-67 M4: the filter's stack (`CAPTURE`) offers no `form.save`, so `ctrl-s` there sends
/// nothing, and a chord never types into the field.
#[tokio::test]
async fn ctrl_s_on_the_filter_saves_nothing_and_types_nothing() {
    let mut harness = open().await;
    harness.key("/");
    type_text(&mut harness, "ST");
    harness.key("ctrl-s");
    assert_eq!(harness.queued(), 0, "nothing was sent");
    let frame = harness.render();
    assert!(hint(&frame).starts_with(" /ST"), "{frame}");
    assert!(!hint(&frame).starts_with(" /STs"), "{frame}");
    assert!(
        hint(&frame).ends_with("  Enter apply \u{b7} Esc clear"),
        "{frame}"
    );
}

/// MOD-67 M4 (lane rebinding test): `[requirements] amend = "E"` moves the amend to `E`; `e`
/// opens nothing and the browse hint names the new chord.
#[tokio::test]
async fn a_rebound_amend_amends_and_e_is_inert() {
    let mut harness = open_with_keys("version = 1\n[requirements]\namend = \"E\"\n").await;
    let frame = harness.render();
    assert!(hint(&frame).contains("E amend"), "{frame}");
    assert!(!hint(&frame).contains("e amend"), "{frame}");
    harness.key("e");
    let frame = harness.render();
    assert!(
        !frame.contains(" Amend R-ENT-1"),
        "`e` opens nothing: {frame}"
    );
    harness.key("E");
    let frame = harness.render();
    assert!(frame.contains(" Amend R-ENT-1 (v2) "), "{frame}");
}
