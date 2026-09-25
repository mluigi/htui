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
//! a frame is symbols, not styles.
#![cfg(feature = "testkit")]

use htui::app::register_all;
use htui::editor::{ExternalEdit, ExternalEditOutcome};
use htui::testkit::Harness;
use htui::ui::tabs::SkillsTab;
use htui_core::fixtures::ids;
use htui_core::model::{NewPromptTemplate, PromptTemplate, PromptTemplateId};
use htui_core::prompt::body_of;
use htui_core::store::{CasOutcome, MemStore, WriteStore};

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
    for token in ["{{item_key}}", "{{phase}}", "{{task}}", "{{candidates}}"] {
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
        hint(&frame).ends_with("Ctrl+S save  Ctrl+E $EDITOR  Esc cancel  L1:C1"),
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
        frame.contains("J/K PgUp/PgDn scroll"),
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
        Some("settings"),
        "`Tab` passes to the shell"
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

#[tokio::test]
async fn the_strip_text_is_unchanged() {
    let mut harness = open().await;
    let frame = harness.render();
    assert!(
        frame.contains(" 1 Backlog  2 Skills  3 Settings  4 Chat"),
        "{frame}"
    );
}
