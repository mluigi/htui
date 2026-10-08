//! The Skills tab's Templates view (MOD-9 milestone 1, T5): browse, edit, the save gate, the diff
//! and the `$EDITOR` handoff, through a `Harness` over the demo world.
//!
//! The Harness enters the Graphics workspace at startup, so the tree is one project,
//! `vulkan-tutorials`, holding the ten compiled defaults at version 1. A direct write goes into the
//! same `MemStore` through a clone: `MemStore` shares its state across clones. Every read and write
//! the view makes goes through `StoreRequest`, so "nothing was sent" is checked on the store: a
//! request that was sent is served by `settle` and would have written a row.
//!
//! The editor's cursor is observable only through the hint row's `L{line}:C{col}` (blueprint D20):
//! a frame is symbols, not styles. `C` is a **grapheme** column (MOD-54 D13), so it steps by the
//! character a person sees rather than by code point -- unchanged for ASCII, which is all of the
//! fixtures here.
#![cfg(feature = "testkit")]

use std::sync::Arc;
use std::time::Duration;

use htui::agent_worker::AgentRuntime;
use htui::app::register_all;
use htui::editor::{ExternalEdit, ExternalEditOutcome};
use htui::keys::load_str;
use htui::testkit::Harness;
use htui::ui::tabs::SkillsTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::driver::{AgentDriver, DriverCaps};
use htui_agent::event::{DoneEvent, DriverEvent, StopReason, TextChunk};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::{edit_agent, ids};
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, EventKind, NewPromptTemplate, PromptTemplate,
    PromptTemplateId, Scope, Transport,
};
use htui_core::prompt::body_of;
use htui_core::store::{CasOutcome, MemStore, ReadStore as _, WriteStore};
use serde_json::json;

/// The shell over `store`, every view registered, on the Skills tab's Templates view.
async fn open_over(store: MemStore) -> Harness {
    let mut harness = Harness::over(store);
    register_all(harness.app());
    harness.settle().await;
    harness.key("2");
    harness.key("l");
    harness.settle().await;
    harness
}

/// The demo world on the Templates view.
async fn open() -> Harness {
    open_over(MemStore::demo()).await
}

/// The demo world over `store` on the Templates view, with the keys file `toml` (MOD-67 M4: a
/// rebinding test). `toml` follows `version = 1`.
async fn open_with_keys(store: MemStore, toml: &str) -> Harness {
    let keys = load_str(&format!("version = 1\n{toml}"))
        .unwrap_or_else(|errors| panic!("the keys load: {errors:?}"));
    let mut harness = Harness::over(store).with_keys(keys);
    register_all(harness.app());
    harness.settle().await;
    harness.key("2");
    harness.key("l");
    harness.settle().await;
    harness
}

/// Types `text` one key at a time: a space is `space`, a newline `enter`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        match c {
            ' ' => harness.key("space"),
            '\n' => harness.key("enter"),
            c => harness.key(&c.to_string()),
        }
    }
}

/// Moves the tree's cursor onto `name`: to the top, then down until the body pane is titled with
/// it.
fn select(harness: &mut Harness, name: &str) {
    for _ in 0..20 {
        harness.key("k");
    }
    let title = format!("\u{250c} {name} v");
    for _ in 0..20 {
        if harness.render().contains(&title) {
            return;
        }
        harness.key("j");
    }
    panic!("`{name}` is not in the tree:\n{}", harness.render());
}

/// The head of `name` in the Harness's one project.
async fn head(store: &MemStore, name: &str) -> Option<PromptTemplate> {
    store
        .prompt_template(ids::PROJECT_VULKAN, name, None)
        .await
        .unwrap_or_else(|err| panic!("the read failed: {err}"))
}

/// The hint row: the last line of the tab's body, just above the shell's status line.
fn hint(frame: &str) -> String {
    let lines: Vec<&str> = frame.lines().collect();
    lines[lines.len() - 2].to_owned()
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

/// Saves `marker` as a new first line of `implement`, which lands as v2.
async fn save_implement_v2(harness: &mut Harness, marker: &str) {
    select(harness, "implement");
    harness.key("e");
    type_text(harness, marker);
    harness.key("enter");
    harness.key("ctrl-s");
    harness.settle().await;
}

/// Appends `body` to `implement` straight into the shared store, as another session would, over
/// head `expected`.
async fn append_implement(store: &MemStore, body: &str, expected: i32) {
    let outcome = store
        .append_prompt_template(
            NewPromptTemplate {
                id: PromptTemplateId::new(),
                project_id: ids::PROJECT_VULKAN,
                name: "implement".to_owned(),
                body: body.to_owned(),
                created_by: ids::USER,
            },
            Some(expected),
        )
        .await
        .expect("the direct write");
    assert!(
        matches!(outcome, CasOutcome::Applied(ref row) if row.version == expected + 1),
        "v{} was not applied",
        expected + 1
    );
}

// --- snapshots ---------------------------------------------------------------------------------

#[tokio::test]
async fn the_templates_view_lists_the_scope_s_templates() {
    let mut harness = open().await;
    select(&mut harness, "implement");
    let frame = harness.render();
    assert!(
        frame.contains("vulkan-tutorials"),
        "the project header: {frame}"
    );
    assert!(
        frame.contains("  implement     v1   phase"),
        "a template row names its head and its role: {frame}"
    );
    assert!(
        frame.contains("\u{250c} implement v1 (head v1) "),
        "the body pane: {frame}"
    );
    assert!(
        frame.contains("You are running the `implement` phase"),
        "the body: {frame}"
    );
    insta::assert_snapshot!("browse", frame);
}

#[tokio::test]
async fn the_judge_editor_lists_its_placeholders_and_its_wire() {
    let mut harness = open().await;
    select(&mut harness, "judge");
    harness.key("e");
    let frame = harness.render();
    // MOD-9 D48: the judge's closed set gained `{{skills}}`, so its help lists five.
    for token in [
        "{{item_key}}",
        "{{phase}}",
        "{{skills}}",
        "{{task}}",
        "{{candidates}}",
    ] {
        assert!(
            frame.contains(token),
            "`{token}` is a judge placeholder: {frame}"
        );
    }
    assert!(
        !frame.contains("{{item}} "),
        "`item` is not a judge placeholder: {frame}"
    );
    assert!(
        frame.contains("section required"),
        "candidates is required: {frame}"
    );
    assert!(frame.contains("```json"), "the judge wire note: {frame}");
    assert!(hint(&frame).ends_with("L1:C1"), "{}", hint(&frame));
    insta::assert_snapshot!("edit_help", frame);
}

#[tokio::test]
async fn an_unknown_placeholder_puts_the_cursor_on_its_braces() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("e");
    harness.key("down");
    harness.key("down");
    type_text(&mut harness, "{{itme}}");
    assert!(hint(&harness.render()).ends_with("L3:C9"), "after typing");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("unknown prompt placeholder `{{itme}}` at byte"),
        "{frame}"
    );
    assert!(
        hint(&frame).ends_with("L3:C1"),
        "the cursor is on the `{{`: {frame}"
    );
    assert_eq!(
        head(&store, "implement").await.map(|row| row.version),
        Some(1)
    );
    insta::assert_snapshot!("unknown_placeholder_cursor", frame);
}

#[tokio::test]
async fn a_phase_body_without_item_asks_before_saving() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    harness.key("n");
    type_text(&mut harness, "triage");
    harness.key("enter");
    type_text(&mut harness, "Triage {{item_key}}");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("never places {{item}}"),
        "the confirm notice: {frame}"
    );
    assert_eq!(head(&store, "triage").await, None, "nothing was sent");
    insta::assert_snapshot!("missing_item_confirm", frame);
}

#[tokio::test]
async fn any_two_versions_diff() {
    let mut harness = open().await;
    save_implement_v2(&mut harness, "MARKER").await;
    harness.key(",");
    harness.key("b");
    harness.key(".");
    harness.key("d");
    let frame = harness.render();
    assert!(
        frame.contains("diff v1 \u{2192} v2"),
        "the pane title: {frame}"
    );
    assert!(frame.contains("+MARKER"), "the added line: {frame}");
    insta::assert_snapshot!("diff_two_versions", frame);
}

#[tokio::test]
async fn a_save_over_a_moved_head_keeps_the_draft() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("e");
    type_text(&mut harness, "DRAFT ");
    let elsewhere = store
        .append_prompt_template(
            NewPromptTemplate {
                id: PromptTemplateId::new(),
                project_id: ids::PROJECT_VULKAN,
                name: "implement".to_owned(),
                body: "saved elsewhere {{item}}\n".to_owned(),
                created_by: ids::USER,
            },
            Some(1),
        )
        .await
        .expect("the direct write");
    assert!(matches!(elsewhere, CasOutcome::Applied(ref row) if row.version == 2));
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains(
            "saved elsewhere since you opened it \u{2014} v2 is now the latest; your draft is \
             kept and Ctrl+S saves it as v3"
        ),
        "the stale notice: {frame}"
    );
    assert!(
        frame.contains("DRAFT You are running"),
        "the draft is kept: {frame}"
    );
    assert!(
        frame.contains("saves v3"),
        "the token moved to the new head: {frame}"
    );
    assert_eq!(
        head(&store, "implement").await.map(|row| row.body),
        Some("saved elsewhere {{item}}\n".to_owned()),
        "the stale save wrote nothing"
    );
    insta::assert_snapshot!("changed_elsewhere", frame);
}

// --- asserts -----------------------------------------------------------------------------------

#[tokio::test]
async fn ctrl_s_with_missing_required_puts_the_cursor_at_the_end() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "judge");
    harness.key("E");
    assert!(harness.app().take_external_edit().is_some());
    harness.app().finish_external_edit(
        SkillsTab::ID,
        ExternalEditOutcome::Edited("no candidates\nhere\n".to_owned()),
    );
    harness.key("up");
    harness.key("up");
    harness.key("home");
    assert!(hint(&harness.render()).ends_with("L1:C1"));
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("must use candidates"), "{frame}");
    assert!(
        hint(&frame).ends_with("L3:C1"),
        "the end of the body: {frame}"
    );
    assert_eq!(head(&store, "judge").await.map(|row| row.version), Some(1));
}

#[tokio::test]
async fn a_second_ctrl_s_saves_without_item_and_a_new_version_appears() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    harness.key("n");
    type_text(&mut harness, "triage");
    harness.key("enter");
    type_text(&mut harness, "Triage {{item_key}}");
    harness.key("ctrl-s");
    harness.settle().await;
    harness.key("ctrl-s");
    harness.settle().await;
    let row = head(&store, "triage")
        .await
        .expect("the second Ctrl+S saved");
    assert_eq!((row.version, row.body.as_str()), (1, "Triage {{item_key}}"));
    let frame = harness.render();
    assert!(notice(&frame).contains("saved v1"), "{frame}");
    assert!(
        frame.contains("  triage        v1   phase"),
        "the new row: {frame}"
    );
}

#[tokio::test]
async fn judge_and_handoff_bodies_never_get_the_item_warning() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    for name in ["judge", "handoff"] {
        select(&mut harness, name);
        harness.key("e");
        type_text(&mut harness, "x");
        harness.key("ctrl-s");
        harness.settle().await;
        let frame = harness.render();
        assert!(!frame.contains("never places"), "{name}: {frame}");
        assert_eq!(
            head(&store, name).await.map(|row| row.version),
            Some(2),
            "{name} saved on the first Ctrl+S"
        );
    }
}

#[tokio::test]
async fn a_refused_save_sends_no_request() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("e");
    type_text(&mut harness, "{{itme}}");
    harness.key("ctrl-s");
    harness.settle().await;
    assert_eq!(
        head(&store, "implement").await.map(|row| row.version),
        Some(1)
    );
    let frame = harness.render();
    // The store runs the same `parse` behind its compare-and-set, so the head alone cannot tell
    // "nothing was sent" from "sent and refused": a sent save would come back as a `Failed` whose
    // message is `StoreError::Constraint`'s "constraint violated: …", and would leave the cursor
    // where the typing left it (L1:C9). Only the local refusal starts with the parse error and
    // moves the cursor to the `{{`.
    assert!(
        notice(&frame).starts_with("unknown prompt placeholder `{{itme}}` at byte 0"),
        "the parse error, not a store failure: {frame}"
    );
    assert!(
        !notice(&frame).contains("constraint violated"),
        "no `Failed` reply landed: {frame}"
    );
    assert!(
        hint(&frame).ends_with("Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel  L1:C1"),
        "the editor is still open, the cursor on the `{{`: {frame}"
    );
}

#[tokio::test]
async fn a_wrong_role_placeholder_puts_the_cursor_on_its_braces() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("e");
    harness.key("down");
    harness.key("down");
    type_text(&mut harness, "{{candidates}}");
    assert!(hint(&harness.render()).ends_with("L3:C15"), "after typing");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).starts_with("placeholder `{{candidates}}` is not available to a Phase"),
        "{frame}"
    );
    assert!(
        hint(&frame).ends_with("L3:C1"),
        "the cursor is on the `{{`: {frame}"
    );
    assert_eq!(
        head(&store, "implement").await.map(|row| row.version),
        Some(1)
    );
}

#[tokio::test]
async fn an_unterminated_opener_puts_the_cursor_on_its_braces() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("e");
    // Line 2 is empty, so nothing after the opener on its line closes it.
    harness.key("down");
    type_text(&mut harness, "{{item");
    assert!(hint(&harness.render()).ends_with("L2:C7"), "after typing");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).starts_with("unterminated `{{` at byte"),
        "{frame}"
    );
    assert!(
        hint(&frame).ends_with("L2:C1"),
        "the cursor is on the `{{`: {frame}"
    );
    assert_eq!(
        head(&store, "implement").await.map(|row| row.version),
        Some(1)
    );
}

#[tokio::test]
async fn keys_typed_while_a_save_is_in_flight_are_kept() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("e");
    type_text(&mut harness, "A");
    harness.key("ctrl-s");
    // The save is queued, not served: these keys land while it is in flight.
    type_text(&mut harness, "B");
    harness.settle().await;
    let default = body_of("implement").expect("a default");
    assert_eq!(
        head(&store, "implement").await.map(|row| row.body),
        Some(format!("A{default}")),
        "v2 is the body that was sent"
    );
    let frame = harness.render();
    assert!(
        frame.contains("ABYou are running"),
        "the later edit is kept: {frame}"
    );
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "the editor stays open: {frame}"
    );
    assert!(
        notice(&frame).contains("saved v2") && notice(&frame).contains("Ctrl+S saves them as v3"),
        "{frame}"
    );
    assert!(frame.contains("saves v3"), "the token moved to v2: {frame}");
    harness.key("ctrl-s");
    harness.settle().await;
    let row = head(&store, "implement").await.expect("a head");
    assert_eq!((row.version, row.body), (3, format!("AB{default}")));
    let frame = harness.render();
    assert!(notice(&frame).contains("saved v3"), "{frame}");
    assert!(
        !hint(&frame).contains("Ctrl+S save"),
        "back in Browse: {frame}"
    );
}

#[tokio::test]
async fn another_session_s_version_is_not_taken_for_the_save() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("e");
    type_text(&mut harness, "MINE ");
    // `Tab` away and `2` back queues a `Templates` read ahead of the save.
    harness.key("tab");
    harness.key("2");
    harness.key("ctrl-s");
    // Another session appends v2 before either request is served.
    let elsewhere = store
        .append_prompt_template(
            NewPromptTemplate {
                id: PromptTemplateId::new(),
                project_id: ids::PROJECT_VULKAN,
                name: "implement".to_owned(),
                body: "saved elsewhere {{item}}\n".to_owned(),
                created_by: ids::USER,
            },
            Some(1),
        )
        .await
        .expect("the direct write");
    assert!(matches!(elsewhere, CasOutcome::Applied(ref row) if row.version == 2));
    harness.settle().await;
    let frame = harness.render();
    assert!(
        !notice(&frame).contains("saved v2"),
        "the other session's v2 is not our save: {frame}"
    );
    assert!(
        notice(&frame)
            .contains("v2 is now the latest; your draft is kept and Ctrl+S saves it as v3"),
        "the save's own stale reply landed: {frame}"
    );
    assert!(
        frame.contains("MINE You are running"),
        "the draft is kept: {frame}"
    );
    assert!(frame.contains("saves v3"), "{frame}");
    assert_eq!(
        head(&store, "implement").await.map(|row| row.body),
        Some("saved elsewhere {{item}}\n".to_owned()),
        "the stale save wrote nothing"
    );
}

#[tokio::test]
async fn saving_from_v1_while_v2_is_head_writes_v3() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    save_implement_v2(&mut harness, "SECOND").await;
    harness.key(",");
    harness.key("e");
    let frame = harness.render();
    assert!(frame.contains("editing from v1, saves v3"), "{frame}");
    type_text(&mut harness, "THIRD ");
    harness.key("ctrl-s");
    harness.settle().await;
    let row = head(&store, "implement").await.expect("a head");
    assert_eq!(row.version, 3);
    assert_eq!(
        row.body,
        format!("THIRD {}", body_of("implement").expect("a default")),
        "v3 is v1's body with the edit, not v2's"
    );
}

#[tokio::test]
async fn n_creates_a_new_name_at_version_one() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    harness.key("n");
    type_text(&mut harness, "triage");
    harness.key("enter");
    let frame = harness.render();
    assert!(frame.contains("triage \u{b7} new, saves v1"), "{frame}");
    type_text(&mut harness, "Triage {{item}}");
    harness.key("ctrl-s");
    harness.settle().await;
    let row = head(&store, "triage").await.expect("the new name");
    assert_eq!((row.version, row.body.as_str()), (1, "Triage {{item}}"));
    assert_eq!(row.created_by, ids::USER);
}

#[tokio::test]
async fn n_refuses_an_existing_name() {
    let mut harness = open().await;
    harness.key("n");
    type_text(&mut harness, "implement");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("`implement` exists \u{2014} select it and press e"),
        "{frame}"
    );
    assert!(!frame.contains("saves v1"), "no editor opened: {frame}");
}

#[tokio::test]
async fn d_upper_diffs_against_the_compiled_default() {
    let mut harness = open().await;
    save_implement_v2(&mut harness, "MARKER").await;
    harness.key("D");
    let frame = harness.render();
    assert!(frame.contains("diff default \u{2192} v2"), "{frame}");
    assert!(frame.contains("+MARKER"), "{frame}");
    assert!(frame.contains("--- implement default"), "{frame}");
}

/// A default body's lines run to 150-200 columns; the pane is about 66 wide. The body and the diff
/// wrap, so a change past the pane's width is on screen.
#[tokio::test]
async fn a_change_past_column_80_is_visible() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    let default = body_of("implement").expect("a default");
    let mut lines: Vec<String> = default.lines().map(str::to_owned).collect();
    let long = lines
        .iter_mut()
        .find(|line| line.chars().count() >= 80)
        .expect("a line of 80 columns or more");
    long.push_str(" WIDEMARK");
    append_implement(&store, &format!("{}\n", lines.join("\n")), 1).await;
    harness.key("r");
    harness.settle().await;
    select(&mut harness, "implement");
    let frame = harness.render();
    assert!(frame.contains("WIDEMARK"), "the body wraps: {frame}");
    harness.key("d");
    let frame = harness.render();
    assert!(
        frame.contains("diff v1 \u{2192} v2"),
        "the pane title: {frame}"
    );
    assert!(frame.contains("WIDEMARK"), "the diff wraps: {frame}");
}

/// `J`/`K` scroll the pane a row, `PageDown`/`PageUp` ten; a hunk below the fold comes into view.
/// Moving the tree's cursor, or the version with `,`/`.`, starts the pane at its top again.
#[tokio::test]
async fn scrolling_reveals_a_hunk_below_the_fold() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    let body = |changed: bool| {
        let lines: Vec<String> = (1..=60)
            .map(|n| {
                if changed && n % 10 == 0 {
                    format!("line {n} CHANGED{n}")
                } else {
                    format!("line {n}")
                }
            })
            .collect();
        format!("TOPLINE {{{{item}}}}\n{}\n", lines.join("\n"))
    };
    append_implement(&store, &body(false), 1).await;
    append_implement(&store, &body(true), 2).await;
    harness.key("r");
    harness.settle().await;
    select(&mut harness, "implement");
    harness.key("d");
    let frame = harness.render();
    assert!(
        frame.contains("diff v2 \u{2192} v3"),
        "the pane title: {frame}"
    );
    assert!(frame.contains("CHANGED10"), "the first hunk: {frame}");
    assert!(
        !frame.contains("CHANGED60"),
        "the last hunk is below the fold: {frame}"
    );
    assert!(
        frame.contains("J/K scroll · PgUp/PgDn page"),
        "the pane says it scrolls: {frame}"
    );

    let mut presses = 0;
    while !harness.render().contains("CHANGED60") {
        presses += 1;
        assert!(presses <= 10, "PageDown never reached the last hunk");
        harness.key("pagedown");
    }
    assert!(
        !harness.render().contains("--- implement v2"),
        "the header scrolled off"
    );
    for _ in 0..10 {
        harness.key("pageup");
    }
    assert!(
        harness.render().contains("--- implement v2"),
        "back at the top"
    );
    harness.key("J");
    assert!(
        !harness.render().contains("--- implement v2"),
        "`J` is one row"
    );
    harness.key("K");
    assert!(
        harness.render().contains("--- implement v2"),
        "`K` is one row back"
    );

    // `,` shows v2, diffed against v1: the pane starts at its top.
    harness.key("pagedown");
    harness.key(",");
    let frame = harness.render();
    assert!(
        frame.contains("diff v1 \u{2192} v2") && frame.contains("--- implement v1"),
        "`,` resets the scroll: {frame}"
    );

    // Off the row and back: the head's body, from its first line.
    harness.key("pagedown");
    harness.key("j");
    harness.key("k");
    let frame = harness.render();
    assert!(
        frame.contains("\u{2502}TOPLINE {{item}}"),
        "moving resets the scroll: {frame}"
    );
}

/// The list is 32 columns: a name longer than its 13-column field is cut with `…`, so the head and
/// the role stay on the row. A name that fits is shown whole.
#[tokio::test]
async fn a_long_name_is_cut_so_the_version_and_role_stay() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    for name in ["a-very-long-template-name", "thirteen-char"] {
        let outcome = store
            .append_prompt_template(
                NewPromptTemplate {
                    id: PromptTemplateId::new(),
                    project_id: ids::PROJECT_VULKAN,
                    name: name.to_owned(),
                    body: "Do {{item}}\n".to_owned(),
                    created_by: ids::USER,
                },
                None,
            )
            .await
            .expect("the direct write");
        assert!(matches!(outcome, CasOutcome::Applied(_)), "{name}");
    }
    harness.key("r");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{2502}  a-very-long-\u{2026} v1   phase    \u{2502}"),
        "the long name is cut to its field: {frame}"
    );
    assert!(
        frame.contains("\u{2502}  thirteen-char v1   phase    \u{2502}"),
        "a name that fits is whole: {frame}"
    );
}

#[tokio::test]
async fn external_edit_is_requested_with_the_shown_body() {
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("E");
    let Some((tab, edit)) = harness.app().take_external_edit() else {
        panic!("`E` asked for the editor");
    };
    assert_eq!(tab, SkillsTab::ID);
    assert_eq!(
        edit,
        ExternalEdit {
            text: body_of("implement").expect("a default").to_owned(),
            stem: "implement".to_owned(),
        }
    );
}

#[tokio::test]
async fn an_edited_external_result_opens_the_editor_validated() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "implement");
    harness.key("E");
    assert!(harness.app().take_external_edit().is_some());
    harness.app().finish_external_edit(
        SkillsTab::ID,
        ExternalEditOutcome::Edited("x {{bad}}\n".to_owned()),
    );
    harness.settle().await;
    let frame = harness.render();
    assert!(
        hint(&frame).ends_with("L1:C3"),
        "the cursor is on the error: {frame}"
    );
    assert!(
        notice(&frame).contains("unknown prompt placeholder `{{bad}}`"),
        "{frame}"
    );
    assert!(
        frame.contains("x {{bad}}"),
        "the edited text is in the editor: {frame}"
    );
    assert_eq!(
        head(&store, "implement").await.map(|row| row.version),
        Some(1),
        "validated, not sent"
    );

    // A body the store would accept: only the view's not sending it keeps the head at v1.
    harness.key("esc");
    harness.key("esc");
    select(&mut harness, "implement");
    harness.key("E");
    assert!(harness.app().take_external_edit().is_some());
    harness.app().finish_external_edit(
        SkillsTab::ID,
        ExternalEditOutcome::Edited("x {{item}}\n".to_owned()),
    );
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("edited in $EDITOR"), "{frame}");
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "the editor is open on it: {frame}"
    );
    assert_eq!(
        head(&store, "implement").await.map(|row| row.version),
        Some(1),
        "an accepted body is validated, not sent"
    );
}

#[tokio::test]
async fn a_failed_external_result_keeps_the_draft() {
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("e");
    type_text(&mut harness, "DRAFT ");
    harness.key("ctrl-e");
    assert!(harness.app().take_external_edit().is_some());
    harness.app().finish_external_edit(
        SkillsTab::ID,
        ExternalEditOutcome::Failed("`false` exited with 1; nothing was changed".to_owned()),
    );
    let frame = harness.render();
    assert!(notice(&frame).contains("exited with 1"), "{frame}");
    assert!(
        frame.contains("DRAFT You are running"),
        "the draft is back: {frame}"
    );
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "in the editor: {frame}"
    );
}

#[tokio::test]
async fn an_unchanged_quick_return_names_the_wait_flag() {
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("E");
    assert!(harness.app().take_external_edit().is_some());
    harness.app().finish_external_edit(
        SkillsTab::ID,
        ExternalEditOutcome::Unchanged { quick: true },
    );
    let frame = harness.render();
    assert!(notice(&frame).contains("no changes"), "{frame}");
    assert!(notice(&frame).contains("code --wait"), "{frame}");
    assert!(
        !hint(&frame).contains("Ctrl+S save"),
        "back in Browse: {frame}"
    );
}

#[tokio::test]
async fn tab_and_digits_while_editing() {
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("e");
    harness.key("2");
    assert!(
        harness.render().contains("2You are running"),
        "`2` is text in the editor"
    );
    harness.key("tab");
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("requirements"),
        "`Tab` passes to the shell, and Requirements follows Skills"
    );
    harness.key("2");
    harness.settle().await;
    assert_eq!(harness.app().tabs.active_id(), Some(SkillsTab::ID));
    let frame = harness.render();
    assert!(
        frame.contains("2You are running"),
        "the draft survived: {frame}"
    );
    assert!(hint(&frame).contains("Ctrl+S save"), "{frame}");
}

/// MOD-67 M4 D5 through `TEMPLATES_BROWSE`: the Templates view's own stack offers the switch, so
/// `h` shows the Skills view; a CONTROL or ALT `h` is another chord and leaves Templates shown.
#[tokio::test]
async fn h_and_l_switch_from_templates_but_ctrl_h_does_not() {
    let mut harness = open().await;
    let on_templates = |frame: &str| frame.contains("D default");
    let frame = harness.render();
    assert!(on_templates(&frame), "the Templates view: {frame}");
    for key in ["ctrl-h", "alt-h"] {
        harness.key(key);
        harness.settle().await;
        let frame = harness.render();
        assert!(on_templates(&frame), "`{key}` did not switch: {frame}");
    }
    harness.key("h");
    harness.settle().await;
    let frame = harness.render();
    assert!(!on_templates(&frame), "`h` showed the Skills view: {frame}");
    assert!(frame.contains("I import"), "the Library hint: {frame}");
    harness.key("l");
    harness.settle().await;
    let frame = harness.render();
    assert!(on_templates(&frame), "`l` came back: {frame}");
}

/// MOD-67 M4 lane rebinding test: `[skills.templates] diff_default` moves the default diff to
/// `X`; `D` is then inert, and the hint follows.
#[tokio::test]
async fn a_rebound_diff_default_diffs_and_capital_d_is_inert() {
    let mut harness = open_with_keys(
        MemStore::demo(),
        "[skills.templates]\ndiff_default = \"X\"\n",
    )
    .await;
    select(&mut harness, "implement");
    harness.key("D");
    let frame = harness.render();
    assert!(!frame.contains("diff default"), "`D` is inert: {frame}");
    assert!(hint(&frame).contains("X default"), "{frame}");
    assert!(!hint(&frame).contains("D default"), "{frame}");
    harness.key("X");
    let frame = harness.render();
    assert!(
        frame.contains("diff default \u{2192} v1"),
        "`X` diffs against the compiled default: {frame}"
    );
    assert!(
        frame.contains("no differences"),
        "v1 is the default: {frame}"
    );
}

/// MOD-67 M4: a `[skills]` rebind reaches the Templates view (its stack inherits the shared
/// `skills` verbs): `F6` toggles the diff pane, `d` no longer does.
#[tokio::test]
async fn a_shared_skills_rebind_reaches_templates() {
    let mut harness = open_with_keys(MemStore::demo(), "[skills]\ndiff = \"f6\"\n").await;
    save_implement_v2(&mut harness, "MARKER").await;
    let frame = harness.render();
    assert!(hint(&frame).contains("F6 diff"), "{frame}");
    assert!(!hint(&frame).contains("d diff"), "{frame}");
    harness.key("d");
    let frame = harness.render();
    assert!(
        !frame.contains("diff v1 \u{2192} v2"),
        "`d` is inert: {frame}"
    );
    harness.key("f6");
    let frame = harness.render();
    assert!(
        frame.contains("diff v1 \u{2192} v2"),
        "`F6` shows the diff: {frame}"
    );
    assert!(frame.contains("+MARKER"), "{frame}");
    harness.key("f6");
    let frame = harness.render();
    assert!(
        !frame.contains("diff v1 \u{2192} v2"),
        "`F6` toggles back: {frame}"
    );
}

#[tokio::test]
async fn the_strip_text_is_unchanged() {
    let mut harness = open().await;
    let frame = harness.render();
    assert!(
        frame.contains(" 1 Backlog  2 Skills  3 Requirements  4 Settings  5 Chat"),
        "{frame}"
    );
}

// --- MOD-55: agent help over a scripted agent ---------------------------------------------------

/// A registry row the factory reaches by row data alone: `cli` with stream `fake` (`tests/chat.rs`
/// `scripted_row`).
fn scripted_row() -> Agent {
    Agent {
        id: AgentId::new(),
        name: "scripted".to_owned(),
        transport: Transport::Cli,
        billing: Billing::Subscription,
        models: Vec::new(),
        default_model: Some("sonnet".to_owned()),
        launch: json!({ "command": "unused", "args": [] }),
        settings: json!({ "cli": { "stream": "fake", "permission_mode": "ask",
                                   "extra_args": [] } }),
        enabled: true,
        created_at: htui_core::fixtures::demo_at(0, 0),
        updated_at: htui_core::fixtures::demo_at(0, 0),
    }
}

/// Lets the test keep the adapter it loaded a script into.
#[derive(Debug)]
struct SharedAdapter(Arc<FakeAdapter>);

impl htui_agent::registry::TransportBuilder for SharedAdapter {
    fn build(
        &self,
        agent: &Agent,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, htui_agent::error::DriverError> {
        self.0.build(agent, on_box, caps)
    }
}

/// The Templates view over the demo world with an agent runtime whose one enabled agent,
/// `scripted`, plays `script`. The fixture's own agents are disabled, as `tests/chat.rs` does.
async fn open_with_agent(script: Script) -> (Harness, MemStore) {
    let store = MemStore::demo();
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        edit_agent(&store, &row).await.expect("the row is disabled");
    }
    store
        .upsert_agent(&scripted_row(), None)
        .await
        .expect("the scripted row lands");
    let adapter = Arc::new(FakeAdapter::new());
    adapter.load(script);
    let mut factory = DriverFactory::new();
    factory.register("cli/fake", Box::new(SharedAdapter(Arc::clone(&adapter))));
    let mut harness = Harness::over(store.clone())
        .with_agent_runtime(AgentRuntime::new(factory).with_grace(Duration::from_millis(0)));
    register_all(harness.app());
    harness.drive().await;
    harness.key("2");
    harness.key("l");
    harness.drive().await;
    (harness, store)
}

/// The script of TI-1: a short reply whose block is a phase body `parse` accepts.
fn shorter_script() -> Script {
    Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
            text: "Shorter:\n```\n{{item}}\n\nDo the item.\n```\n".to_owned(),
            message_id: Some("m1".to_owned()),
        })),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ])
}

/// The Harness's startup scope.
fn vulkan() -> Scope {
    Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    }
}

/// TI-1: `Ctrl+G`, a request, one turn: the proposal is drawn as a diff against the body sent;
/// accepting it and `Ctrl+S` saves it through `parse` as the next version. The turn was one chat
/// run, recorded as a help (its prompt row carries the template help's sections) and closed.
#[tokio::test]
async fn ctrl_g_proposal_accept_and_save() {
    let (mut harness, store) = open_with_agent(shorter_script()).await;
    let active = store.active_runs(&vulkan()).await.expect("count");
    select(&mut harness, "implement");
    harness.key("e");
    harness.key("ctrl-g");
    harness.drive().await;
    type_text(&mut harness, "shorter");
    harness.key("enter");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains(" proposal from scripted \u{b7} sent \u{2192} proposed "),
        "{frame}"
    );
    assert!(frame.contains("--- sent"), "the diff's header: {frame}");
    assert!(
        frame.contains("-You are running"),
        "a removed line: {frame}"
    );
    assert!(
        hint(&frame).ends_with("Enter accept · n/Esc discard · J/K scroll · PgUp/PgDn page"),
        "{frame}"
    );
    insta::assert_snapshot!("agent_help_proposal", frame);
    // The added line is below the fold: the pane scrolls.
    harness.key("pagedown");
    let scrolled = harness.render();
    assert!(
        scrolled.contains("+Do the item."),
        "the added line: {scrolled}"
    );
    assert!(!scrolled.contains("--- sent"), "the header scrolled off");

    harness.key("enter");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("proposal accepted \u{2014} Ctrl+S saves it"),
        "{frame}"
    );
    assert!(
        frame.contains("{{item}}"),
        "the draft is the proposal: {frame}"
    );
    harness.key("ctrl-s");
    harness.drive().await;
    let row = head(&store, "implement").await.expect("a head");
    assert_eq!(
        (row.version, row.body.as_str()),
        (2, "{{item}}\n\nDo the item.\n")
    );

    let steps = harness.chat_steps();
    let [step] = steps.as_slice() else {
        panic!("one help turn: {steps:?}");
    };
    let log = store
        .step_events(*step)
        .await
        .expect("the log reads")
        .expect("the help step has a log");
    let prompt = log
        .iter()
        .find(|row| row.kind == EventKind::Prompt)
        .expect("a prompt row");
    let sections: Vec<&str> = prompt.payload["sections"]
        .as_array()
        .expect("sections[]")
        .iter()
        .filter_map(|section| section["name"].as_str())
        .collect();
    assert_eq!(sections, ["instruction", "placeholders", "body", "request"]);
    assert_eq!(
        store.active_runs(&vulkan()).await.expect("count"),
        active,
        "the help's run is closed"
    );
}

/// TI-2: the asking panel sits under the locked draft, the placeholder column beside it.
#[tokio::test]
async fn the_asking_panel() {
    let (mut harness, _) = open_with_agent(Script::default()).await;
    select(&mut harness, "implement");
    harness.key("e");
    harness.key("ctrl-g");
    harness.drive().await;
    type_text(&mut harness, "shorter");
    let frame = harness.render();
    assert!(frame.contains(" ask an agent "), "{frame}");
    assert!(frame.contains("ask: shorter"), "{frame}");
    assert!(frame.contains("agent: scripted"), "{frame}");
    assert!(frame.contains(" phase placeholders "), "{frame}");
    assert!(
        hint(&frame).ends_with("Enter ask · Up/Down agent · Esc back"),
        "{frame}"
    );
    insta::assert_snapshot!("agent_help_asking", frame);
}

/// TI-3 (P2): a body holding a credential is refused before anything is sent: the notice names
/// the section and the rule, the help is closed, and no chat run exists.
#[tokio::test]
async fn a_body_holding_a_key_is_not_sent() {
    let (mut harness, store) = open_with_agent(shorter_script()).await;
    let active = store.active_runs(&vulkan()).await.expect("count");
    select(&mut harness, "implement");
    harness.key("e");
    type_text(&mut harness, &format!("ghp_{} ", "A1b2".repeat(9)));
    harness.key("ctrl-g");
    harness.drive().await;
    type_text(&mut harness, "tidy");
    harness.key("enter");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("not sent: the body matches the github_token rule"),
        "{frame}"
    );
    assert!(
        !frame.contains(" ask an agent "),
        "the help closed: {frame}"
    );
    assert!(hint(&frame).contains("Ctrl+S save"), "{frame}");
    assert!(harness.chat_steps().is_empty(), "no session started");
    assert_eq!(store.active_runs(&vulkan()).await.expect("count"), active);
}
