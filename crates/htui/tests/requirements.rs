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
    let mut harness = Harness::over(store);
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
