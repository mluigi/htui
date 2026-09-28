//! The Skills tab's Skills view (MOD-9 milestone 3, T3): the library, the version browser, the
//! line diff, the two-field name form, the editor, the save and the `$EDITOR` handoff, through a
//! `Harness` over the demo world.
//!
//! The Harness enters the Graphics workspace at startup, so the scope is one project,
//! `vulkan-tutorials`. The demo attaches its skills to `htui` and `agy`, both in the **Platform**
//! workspace, so the library starts with no attachment in scope and one test attaches `tests` to
//! `vulkan-tutorials` to give the activation column something to draw. The library itself is
//! global, so both demo skills are always on screen. A direct write goes into the same `MemStore`
//! through a clone: `MemStore` shares its state across clones. Every read and write **the view**
//! makes goes through `StoreRequest`, so "nothing was sent" is checked on the store: a request
//! that was sent is served by `settle` and would have written a row.
//!
//! The six `templates__*.snap` files are the coupling milestone 1 left behind: this view shares
//! the tab with the Templates view, the switch line, the strip text and the shell's frame. They
//! are read here and pinned, so a change to the shared row is a failing test rather than a silent
//! diff in a file nobody was reading.
#![cfg(feature = "testkit")]

use htui::app::{Action, register_all};
use htui::editor::{ExternalEdit, ExternalEditOutcome};
use htui::store_worker::StoreRequest;
use htui::testkit::Harness;
use htui::ui::tabs::{SkillsTab, Tab};
use htui_core::fixtures::ids;
use htui_core::model::{
    Activation, NewSkill, NewSkillBinding, NewSkillVersion, Scope, SkillBindingId, SkillId,
    SkillVersion, WorkspaceSummary,
};
use htui_core::store::{CasOutcome, MemStore, WriteStore as _};

/// The shell over `store`, every view registered, on the Skills tab's **Skills** view. The tab
/// opens there (D34), so — unlike the templates helper — there is no second `l`.
async fn open_over(store: MemStore) -> Harness {
    let mut harness = Harness::over(store);
    register_all(harness.app());
    harness.settle().await;
    harness.key("2");
    harness.settle().await;
    harness
}

/// The demo world on the Skills view.
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

/// Moves the cursor onto `name`: to the top, then down until the body pane is titled with it.
///
/// The library is `skill.name` byte order, so the fixture's two rows are `rust-style` then
/// `tests`; the loop is here so a fixture that grows a third row does not silently move this
/// helper's callers onto the wrong one.
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
    panic!("`{name}` is not in the library:\n{}", harness.render());
}

/// Every library entry, as another session would read it.
async fn library(store: &MemStore) -> Vec<htui_core::model::SkillEntry> {
    store
        .skill_library()
        .await
        .unwrap_or_else(|err| panic!("the read failed: {err}"))
}

/// The head version of `name`.
async fn head(store: &MemStore, name: &str) -> Option<SkillVersion> {
    library(store)
        .await
        .into_iter()
        .find(|entry| entry.skill.name == name)
        .and_then(|entry| entry.versions.last().cloned())
}

/// The `skill.id` of `name`.
async fn id_of(store: &MemStore, name: &str) -> SkillId {
    library(store)
        .await
        .into_iter()
        .find(|entry| entry.skill.name == name)
        .map(|entry| entry.skill.id)
        .unwrap_or_else(|| panic!("`{name}` is in the library"))
}

/// Appends `body` to `skill_id` straight into the shared store, as another session would, over
/// `expected` (`None` for a skill nothing has appended to yet).
async fn append(store: &MemStore, skill_id: SkillId, body: &str, expected: Option<i32>) {
    let outcome = store
        .add_skill_version(
            NewSkillVersion {
                skill_id,
                body: body.to_owned(),
                source: serde_json::json!({}),
                created_by: ids::USER,
            },
            expected,
        )
        .await
        .expect("the direct write");
    assert!(
        matches!(outcome, CasOutcome::Applied(_)),
        "the direct append over {expected:?} was not applied"
    );
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

/// The demo's second workspace, as a scope change would ask for it.
async fn platform(store: &MemStore) -> WorkspaceSummary {
    store
        .workspaces()
        .await
        .unwrap_or_else(|err| panic!("the workspace read failed: {err}"))
        .into_iter()
        .find(|workspace| workspace.slug == "platform")
        .expect("the demo fixture holds the `platform` workspace")
}

/// A skill row as `render_browse` draws it, so the assertions are the layout rather than a count
/// of spaces someone typed by hand.
fn row(name: &str, head: i32, activation: &str, tokens: &str) -> String {
    format!("  {name:<26} v{head:<3} {activation:<5} {tokens}")
}

// --- snapshots ---------------------------------------------------------------------------------

/// D84 / D108: the list carries the head, the activation and the **token estimate computed in
/// `render`** from bytes the snapshot already holds — no store round trip, so `R-NF-3` holds. The
/// pane title names the estimator's own id, so one id never carries two arithmetics.
#[tokio::test]
async fn the_skills_view_lists_the_library_with_versions_and_token_estimates() {
    let store = MemStore::demo();
    // The demo attaches its skills to `htui` and `agy`, both in the Platform workspace, so the
    // Graphics scope has nothing attached and the activation column would be two em dashes. One
    // project-level row here is what gives it a value to draw.
    let attached = store
        .set_skill_binding(
            NewSkillBinding {
                id: SkillBindingId::new(),
                skill_id: ids::SKILL_TESTS,
                project_id: Some(ids::PROJECT_VULKAN),
                phase_id: None,
                pinned_version: None,
                position: 0,
                activation: Activation::Always,
                globs: Vec::new(),
                languages: Vec::new(),
            },
            None,
        )
        .await
        .expect("the direct write");
    assert!(
        matches!(attached, CasOutcome::Applied(_)),
        "the bind applies"
    );

    let mut harness = open_over(store).await;
    select(&mut harness, "rust-style");
    let frame = harness.render();
    assert!(
        frame.contains("\u{250c} rust-style v2 (head v2) ~23 tok (chars-v2) "),
        "the body pane names the head, the estimate and the estimator's own id: {frame}"
    );
    assert!(
        frame.contains(&row("rust-style", 2, "\u{2014}", "~23")),
        "a skill the scope attaches nowhere: {frame}"
    );
    assert!(
        frame.contains(&row("tests", 1, "always", "~15")),
        "and an attached one, with its activation: {frame}"
    );
    assert!(
        frame.contains("Prefer `expect` with a reason."),
        "the body: {frame}"
    );
    insta::assert_snapshot!("browse", frame);
}

/// `diff::unified` over two stored versions, with the two labels the Templates view uses.
#[tokio::test]
async fn any_two_skill_versions_diff() {
    let mut harness = open().await;
    select(&mut harness, "rust-style");
    harness.key(",");
    harness.key("b");
    harness.key(".");
    harness.key("d");
    let frame = harness.render();
    assert!(
        frame.contains("diff v1 \u{2192} v2"),
        "the pane title: {frame}"
    );
    assert!(
        frame.contains("--- rust-style v1") && frame.contains("+++ rust-style v2"),
        "the two labels: {frame}"
    );
    assert!(
        // The pane is 52 columns, so the added line wraps: the head of it is the claim.
        frame.contains("+Prefer `expect` with a reason. One error enum per"),
        "the added line: {frame}"
    );
    insta::assert_snapshot!("diff_two_versions", frame);
}

/// A skill body is markdown, so there are no placeholders to list; the right-hand pane says what a
/// save does instead. The Templates view's analogue is `templates__edit_help`.
#[tokio::test]
async fn the_editor_lists_its_help() {
    let mut harness = open().await;
    select(&mut harness, "rust-style");
    harness.key("E");
    let frame = harness.render();
    assert!(
        frame.contains(" rust-style \u{b7} editing from v2, saves v3 "),
        "the editor's title: {frame}"
    );
    assert!(frame.contains("appends a version"), "the help: {frame}");
    assert!(hint(&frame).ends_with("L1:C1"), "{}", hint(&frame));
    insta::assert_snapshot!("editor_help", frame);
}

/// The form has **two** fields (`R-SKL-1`: the library is `name` + `description` + versioned
/// body), and `Tab` moves between them. `h`/`l` are letters there, not the view switch (H-32).
#[tokio::test]
async fn the_name_form_takes_a_name_and_a_description() {
    let mut harness = open().await;
    harness.key("n");
    type_text(&mut harness, "house-rules");
    harness.key("tab");
    type_text(&mut harness, "the ones we always follow");
    let frame = harness.render();
    assert_eq!(
        harness.app().tabs.active_id(),
        Some(SkillsTab::ID),
        "H-32: `Tab` moves the form's cursor, it does not switch tabs"
    );
    assert!(frame.contains(" name: house-rules"), "the name: {frame}");
    assert!(
        frame.contains(" description: the ones we always follow"),
        "the description: {frame}"
    );
    assert_eq!(
        hint(&frame),
        " Tab next field  Enter confirm  Esc cancel",
        "the form's own hint: {frame}"
    );
    insta::assert_snapshot!("naming", frame);
}

/// D101: a spent head version answers `SkillsStale`, the draft is kept and **both** tokens move to
/// the head the user has just been told about, so the next `Ctrl+S` is a deliberate append.
#[tokio::test]
async fn a_save_over_a_moved_head_keeps_the_draft() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "rust-style");
    harness.key("E");
    type_text(&mut harness, "DRAFT ");
    append(&store, ids::SKILL_RUST_STYLE, "saved elsewhere\n", Some(2)).await;
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains(
            "this skill changed elsewhere and is now at v3; the draft is kept, and Ctrl+S appends \
             to v4"
        ),
        "the stale notice: {frame}"
    );
    assert!(
        frame.contains("DRAFT Prefer `expect`"),
        "the draft is kept: {frame}"
    );
    assert!(
        frame.contains("saves v4"),
        "the head token moved to v3: {frame}"
    );
    assert_eq!(
        head(&store, "rust-style").await.map(|row| row.body),
        Some("saved elsewhere\n".to_owned()),
        "the stale save wrote nothing"
    );
    insta::assert_snapshot!("stale", frame);
}

// --- asserts -----------------------------------------------------------------------------------

/// D78 / OQ-18: one `Ctrl+S` upserts the row and appends the body as the next version, so the
/// head moves and the view comes back to Browse.
#[tokio::test]
async fn a_save_appends_a_version_and_moves_the_head() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "rust-style");
    harness.key("E");
    type_text(&mut harness, "MARKER");
    harness.key("ctrl-s");
    harness.settle().await;

    let row = head(&store, "rust-style").await.expect("a head");
    assert_eq!(row.version, 3, "the head's plus one");
    assert!(
        row.body.starts_with("MARKERPrefer `expect`"),
        "the body that was sent: {:?}",
        row.body
    );
    let frame = harness.render();
    assert!(notice(&frame).contains("saved v3"), "{frame}");
    assert!(
        !hint(&frame).contains("Ctrl+S save"),
        "the editor closed on an unchanged draft: {frame}"
    );
}

/// `n` creates: a name no row holds, an empty body, and a save that lands as **v1** — not v2,
/// because the head token of a skill that does not exist is `None`. The form's second field is
/// `skill.description`, not part of the body.
#[tokio::test]
async fn a_new_skill_starts_at_version_one() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    harness.key("n");
    type_text(&mut harness, "house-rules");
    harness.key("tab");
    type_text(&mut harness, "the ones we always follow");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains(" house-rules \u{b7} new, saves v1 "),
        "{frame}"
    );
    type_text(&mut harness, "No `unwrap` off the test path.");
    harness.key("ctrl-s");
    harness.settle().await;

    let saved = head(&store, "house-rules").await.expect("the new skill");
    assert_eq!(saved.version, 1);
    assert_eq!(saved.body, "No `unwrap` off the test path.");
    let entry = library(&store)
        .await
        .into_iter()
        .find(|entry| entry.skill.name == "house-rules")
        .expect("the new entry");
    assert_eq!(
        entry.skill.description, "the ones we always follow",
        "the form's second field is the row's description"
    );
    let frame = harness.render();
    assert!(notice(&frame).contains("saved v1"), "{frame}");
    assert!(
        frame.contains(&row("house-rules", 1, "\u{2014}", "~12")),
        "and the library grew a row: {frame}"
    );
}

/// D77 / OQ-20: the name rule is the writer's, so the view refuses with the same sentence and
/// **sends nothing**. The store is the only thing that could tell the two apart, and it holds no
/// row.
#[tokio::test]
async fn a_refused_name_sends_no_request() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    harness.key("n");
    type_text(&mut harness, "House Rules");
    harness.key("enter");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("must be 1-64 characters of `[a-z0-9-]`"),
        "the writer's own sentence: {frame}"
    );
    assert!(!frame.contains("saves v1"), "no editor opened: {frame}");
    assert!(
        hint(&frame).contains("Tab next field"),
        "the form stays open, so a typo is one `Backspace` away: {frame}"
    );
    assert!(
        library(&store)
            .await
            .iter()
            .all(|entry| entry.skill.name == "rust-style" || entry.skill.name == "tests"),
        "nothing was sent, so the library is the fixture's two rows"
    );
}

/// A name the fixture already holds is refused before the editor opens, and the notice names the
/// key that reaches it.
#[tokio::test]
async fn n_refuses_a_name_the_library_holds() {
    let mut harness = open().await;
    harness.key("n");
    type_text(&mut harness, "rust-style");
    harness.key("enter");
    let frame = harness.render();
    assert!(notice(&frame).contains("`rust-style` exists"), "{frame}");
    assert!(!frame.contains("saves v1"), "no editor opened: {frame}");
}

/// `E` opens the editor on the shown version; `Ctrl+E` hands the draft to `$EDITOR`; the return
/// opens the editor on the text and **nothing is sent** — there is no `parse` here, because a
/// skill body is markdown and there is no byte offset to point at.
#[tokio::test]
async fn the_editor_hands_off_to_a_fake_editor_and_returns_edited() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "rust-style");
    harness.key("E");
    harness.key("ctrl-e");
    let Some((tab, edit)) = harness.app().take_external_edit() else {
        panic!("`Ctrl+E` asked for the editor");
    };
    assert_eq!(tab, SkillsTab::ID);
    assert_eq!(
        edit,
        ExternalEdit {
            text: "Prefer `expect` with a reason. One error enum per crate.".to_owned(),
            stem: "rust-style".to_owned(),
        },
        "the draft travels as it is, named for the library key"
    );
    harness.app().finish_external_edit(
        SkillsTab::ID,
        ExternalEditOutcome::Edited("Prefer `expect` with a reason.\n".to_owned()),
    );
    harness.settle().await;

    let frame = harness.render();
    assert!(notice(&frame).contains("edited in $EDITOR"), "{frame}");
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "the editor is open on the returned text: {frame}"
    );
    assert_eq!(
        head(&store, "rust-style").await.map(|row| row.version),
        Some(2),
        "edited, not saved"
    );

    harness.key("ctrl-s");
    harness.settle().await;
    let row = head(&store, "rust-style").await.expect("a head");
    assert_eq!(row.version, 3);
    assert_eq!(row.body, "Prefer `expect` with a reason.\n");
}

/// A failed `$EDITOR` leaves the draft where it was and says why.
#[tokio::test]
async fn a_failed_external_edit_keeps_the_draft() {
    let mut harness = open().await;
    select(&mut harness, "rust-style");
    harness.key("E");
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
        frame.contains("DRAFT Prefer `expect`"),
        "the draft: {frame}"
    );
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "in the editor: {frame}"
    );
}

/// `on_scope_change` is `settings/prompt.rs`'s: the library, the editor, the pending handoff and
/// the write in flight all belong to the workspace that was left, and only the notice survives —
/// because the scope change is often the consequence of what it reports.
#[tokio::test]
async fn a_scope_change_drops_the_editor_and_keeps_the_notice() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "rust-style");
    harness.key("b");
    assert!(
        notice(&harness.render()).contains("base v2"),
        "the notice to keep"
    );
    let workspace = platform(&store).await;
    harness.app().update(Action::SetScope { workspace });
    harness.settle().await;

    let frame = harness.render();
    assert!(
        notice(&frame).contains("base v2"),
        "the notice survives the scope change: {frame}"
    );
    assert!(
        !hint(&frame).contains("Ctrl+S save"),
        "the editor went with the workspace it belonged to: {frame}"
    );
    assert!(
        frame.contains(&row("tests", 1, "always", "~15")),
        "and the library was re-read for the new scope, where the demo's attachments apply: {frame}"
    );
}

/// The list is 46 columns: a name longer than its 26-column field is cut with `…`, so the head,
/// the activation and the estimate stay on the row. A name that fits is whole.
#[tokio::test]
async fn a_long_name_is_cut_so_the_version_and_estimate_stay() {
    let store = MemStore::demo();
    let long = "a-very-long-skill-name-beyond-the-field";
    let saved = store
        .upsert_skill(
            NewSkill {
                id: SkillId::new(),
                name: long.to_owned(),
                description: "cut".to_owned(),
                created_by: ids::USER,
            },
            None,
        )
        .await
        .expect("the direct write");
    assert!(matches!(saved, CasOutcome::Applied(_)), "{long}");
    let id = id_of(&store, long).await;
    append(&store, id, "body\n", None).await;
    let mut harness = open_over(store).await;
    harness.key("r");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("  a-very-long-skill-name-be\u{2026} v1"),
        "the long name is cut to its field: {frame}"
    );
    assert!(
        frame.contains(&row("tests", 1, "\u{2014}", "~15")),
        "a name that fits is whole: {frame}"
    );
}

// --- contracts ---------------------------------------------------------------------------------

/// F-12 / D109: the tab asks for **two** requests of two different variants. The staleness index
/// keeps only the newest request of a kind, so one variant per project would leave all but one
/// project undrawn — the failure `Catalogue(Scope)`'s own doc describes, and which two requests of
/// two variants is not.
#[test]
fn the_tab_asks_for_templates_and_skills() {
    let scope = Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    };
    let names: Vec<&str> = SkillsTab::new()
        .wants_requests(&scope)
        .iter()
        .map(StoreRequest::name)
        .collect();
    assert_eq!(names, vec!["templates", "skills"]);
}

/// The coupling milestone 1 left behind: one tab, one switch line, one strip, and the six
/// Templates snapshots are its evidence. `library.rs` has its own list widths (D106, H-27) and
/// `skills/mod.rs` does not touch the switch line (H-28), so both the list title and the body
/// pane these files show are still the Templates view's.
///
/// This reads the files rather than the shell, so it fails even when the Templates suite is not
/// run — which is the point: a change to the shared row is a red test here, not a surprise in a
/// diff nobody was reading.
#[test]
fn the_six_template_snapshots_do_not_move() {
    let snapshots = [
        ("browse", include_str!("snapshots/templates__browse.snap")),
        (
            "changed_elsewhere",
            include_str!("snapshots/templates__changed_elsewhere.snap"),
        ),
        (
            "diff_two_versions",
            include_str!("snapshots/templates__diff_two_versions.snap"),
        ),
        (
            "edit_help",
            include_str!("snapshots/templates__edit_help.snap"),
        ),
        (
            "missing_item_confirm",
            include_str!("snapshots/templates__missing_item_confirm.snap"),
        ),
        (
            "unknown_placeholder_cursor",
            include_str!("snapshots/templates__unknown_placeholder_cursor.snap"),
        ),
    ];
    assert_eq!(snapshots.len(), 6, "milestone 1 shipped six");
    for (name, text) in snapshots {
        assert!(
            text.contains(" Skills \u{2502} Templates"),
            "templates__{name}.snap no longer shows the tab's switch line: the two views and the \
             line between them are milestone 1's, and the Skills view is not allowed to move them."
        );
        assert!(
            text.contains(" 1 Backlog  2 Skills  3 Settings  4 Chat"),
            "templates__{name}.snap no longer shows the shell's strip text"
        );
        assert!(
            !text.contains("\u{250c} Skills "),
            "templates__{name}.snap is drawing the Skills list: `library.rs` has its own widths \
             (D106) and must not have widened the Templates one"
        );
    }
    // The two that are in Browse rather than in the editor also pin the list's own title, which is
    // what a widened `LIST_WIDTH` would move.
    for (name, text) in [
        ("browse", include_str!("snapshots/templates__browse.snap")),
        (
            "diff_two_versions",
            include_str!("snapshots/templates__diff_two_versions.snap"),
        ),
    ] {
        assert!(
            text.contains("\u{250c} Templates \u{2500}"),
            "templates__{name}.snap no longer shows the Templates list title"
        );
    }
}

/// The strip text is the shell's, not this tab's, and `the_strip_text_is_unchanged` in
/// `tests/templates.rs` is the pin. Checked here too, because the Skills view is the one this
/// milestone puts in front of the strip.
#[tokio::test]
async fn the_strip_text_is_unchanged() {
    let mut harness = open().await;
    let frame = harness.render();
    assert!(
        frame.contains(" 1 Backlog  2 Skills  3 Settings  4 Chat"),
        "{frame}"
    );
}
