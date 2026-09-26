//! The Skills tab's Skills view (MOD-9 milestone 3, T5): the library, the editor, the diff, the
//! token estimate, and the attachments pane with its form and repo picker, through a `Harness` over
//! the demo world.
//!
//! The Harness enters the Graphics workspace at startup, whose one project, `vulkan-tutorials`,
//! holds no attachment: every demo attachment is on `htui` (`fixtures.rs`' `skill_bindings`). The
//! attachment tests therefore switch to Platform first (`htui`, `agy`), as `integration.rs` does
//! (blueprint F-O). The library itself is global, so the library tests stay on Graphics.
//!
//! A direct write goes into the same `MemStore` through a clone: `MemStore` shares its state across
//! clones. Every read and write the view makes goes through `StoreRequest`, so "nothing was sent"
//! is checked on the store: a request that was sent is served by `settle` and would have written a
//! row.
//!
//! The demo estimates (blueprint F-I): `rust-style` v2 is ~42 tokens, v1 ~31, `tests` v1 ~32.
#![cfg(feature = "testkit")]

use chrono::{DateTime, Utc};
use htui::app::register_all;
use htui::editor::{ExternalEdit, ExternalEditOutcome};
use htui::testkit::Harness;
use htui::ui::tabs::SkillsTab;
use htui_core::fixtures::ids;
use htui_core::model::{
    Activation, Attachment, BindingChange, NewRepo, NewSkillVersion, RepoId, SkillBinding,
    SkillBindingKey, SkillVersion,
};
use htui_core::store::{CasOutcome, MemStore, WriteStore};

/// The shell over `store`, every view registered, on the Skills tab's Skills view (the tab opens
/// there, D34).
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

/// The shell over `store` in the Platform workspace (`htui`, `agy`), on the Skills view (F-O).
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
    harness.key("2");
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

/// Moves the list's cursor onto `name`: to the top, then down until the body pane is titled with
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
    panic!("`{name}` is not in the library:\n{}", harness.render());
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

/// The first frame line holding `needle`, trimmed of the pane's borders; panics when none does.
fn line_with(frame: &str, needle: &str) -> String {
    frame
        .lines()
        .find(|line| line.contains(needle))
        .map(|line| line.trim_matches(['\u{2502}', ' ']).to_owned())
        .unwrap_or_else(|| panic!("no line holds `{needle}`:\n{frame}"))
}

/// The `~N tokens` number in `text`, the first one.
fn tokens(text: &str) -> i64 {
    let at = text
        .find(" tokens")
        .unwrap_or_else(|| panic!("no estimate in {text:?}"));
    let head = &text[..at];
    let tilde = head
        .rfind('~')
        .unwrap_or_else(|| panic!("no `~` before the estimate in {text:?}"));
    head[tilde + 1..]
        .parse()
        .unwrap_or_else(|err| panic!("`{}` is not a number: {err}", &head[tilde + 1..]))
}

/// A skill's versions, straight from the store.
async fn versions(store: &MemStore, name: &str) -> Vec<SkillVersion> {
    let skills = store.skills().await.expect("the library read");
    let Some(skill) = skills.iter().find(|skill| skill.name == name) else {
        return Vec::new();
    };
    store
        .skill_versions(skill.id)
        .await
        .expect("the versions read")
}

/// `htui`'s row for `skill` at the project level, straight from the store.
async fn htui_row(store: &MemStore, skill: htui_core::model::SkillId) -> Option<SkillBinding> {
    store
        .skill_bindings(Some(ids::PROJECT_HTUI))
        .await
        .expect("the attachments read")
        .into_iter()
        .find(|row| row.skill_id == skill && row.phase_id.is_none())
}

/// An `Always`, unpinned, position-0 attachment with no globs.
fn always() -> Attachment {
    Attachment {
        pinned_version: None,
        position: 0,
        activation: Activation::Always,
        globs: Vec::new(),
        languages: Vec::new(),
    }
}

/// Writes `change` at `key` straight into the shared store, as another session would.
async fn set_binding(
    store: &MemStore,
    key: SkillBindingKey,
    expected: Option<DateTime<Utc>>,
    change: BindingChange,
) {
    let outcome = store
        .set_skill_binding(key, expected, change)
        .await
        .expect("the direct write");
    assert!(
        matches!(outcome, CasOutcome::Applied(_)),
        "the direct write applied: {outcome:?}"
    );
}

/// Adds repo `name` to `htui`.
async fn add_repo(store: &MemStore, name: &str, is_primary: bool) {
    store
        .create_repo(NewRepo {
            id: RepoId::new(),
            project_id: ids::PROJECT_HTUI,
            name: name.to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary,
        })
        .await
        .expect("the repo write");
}

// --- snapshots ---------------------------------------------------------------------------------

#[tokio::test]
async fn the_library_lists_the_skills_with_body_and_estimate() {
    let mut harness = open().await;
    select(&mut harness, "rust-style");
    let frame = harness.render();
    assert!(frame.contains(" Skills "), "the list block: {frame}");
    assert!(
        frame.contains("  rust-style             v2"),
        "a library row names its head: {frame}"
    );
    assert!(
        frame.contains("  tests                  v1"),
        "every skill is listed: {frame}"
    );
    assert!(
        frame.contains("rust-style v2 (head v2) \u{b7} ~42 tokens"),
        "the body pane's title carries the estimate: {frame}"
    );
    assert!(
        frame.contains("House Rust conventions"),
        "the description heads the body: {frame}"
    );
    assert!(
        frame.contains("Prefer `expect` with a reason. One error enum per crate."),
        "the head's body: {frame}"
    );
    insta::assert_snapshot!("library", frame);
}

#[tokio::test]
async fn the_editor_opens_on_the_shown_version() {
    let mut harness = open().await;
    select(&mut harness, "rust-style");
    harness.key("e");
    harness.key("Z");
    let frame = harness.render();
    assert!(hint(&frame).contains("Ctrl+S save"), "{frame}");
    assert!(
        frame.contains("rust-style \u{b7} editing from v2, saves v3"),
        "the editor's title: {frame}"
    );
    assert!(frame.contains("ZPrefer"), "the typed char: {frame}");
    insta::assert_snapshot!("edit", frame);
}

#[tokio::test]
async fn any_two_versions_diff() {
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
        frame
            .lines()
            .any(|line| line.contains("+Prefer") && line.contains("One error enum per crate.")),
        "the added line: {frame}"
    );
    insta::assert_snapshot!("diff_two_versions", frame);
}

#[tokio::test]
async fn the_attachments_pane_lists_global_projects_and_phases() {
    let mut harness = open_platform_over(MemStore::demo()).await;
    select(&mut harness, "rust-style");
    harness.key("a");
    let frame = harness.render();
    assert!(
        frame.contains("attachments \u{b7} rust-style"),
        "the pane's title: {frame}"
    );
    for label in ["global", "htui", "  analysis \u{203a} research"] {
        assert!(frame.contains(label), "row `{label}`: {frame}");
    }
    assert!(
        line_with(&frame, "htui").contains("always \u{b7} latest \u{b7} pos 1"),
        "the project row's summary: {frame}"
    );
    assert!(
        line_with(&frame, "global").ends_with('\u{2014}'),
        "no global row: {frame}"
    );
    insta::assert_snapshot!("attachments", frame);
}

#[tokio::test]
async fn the_form_shows_the_effective_globs_before_save() {
    let mut harness = open_platform_over(MemStore::demo()).await;
    select(&mut harness, "tests");
    harness.key("a");
    harness.key("j");
    harness.key("enter");
    harness.key("space");
    for _ in 0..3 {
        harness.key("tab");
    }
    type_text(&mut harness, "rust");
    harness.key("tab");
    type_text(&mut harness, "src/**");
    let frame = harness.render();
    assert!(
        frame.contains("edit tests \u{b7} htui"),
        "the form's title: {frame}"
    );
    assert!(
        frame.contains("activation  glob"),
        "`space` cycled the activation: {frame}"
    );
    assert!(
        frame.contains("effective:  src/**, **/*.rs, **/Cargo.toml"),
        "typed first, then the language's globs: {frame}"
    );
    insta::assert_snapshot!("attach_form_effective_globs", frame);
}

#[tokio::test]
async fn ctrl_r_picks_a_repo_of_the_project() {
    let store = MemStore::demo();
    add_repo(&store, "core", true).await;
    add_repo(&store, "web", false).await;
    let mut harness = open_platform_over(store).await;
    select(&mut harness, "tests");
    harness.key("a");
    harness.key("j");
    harness.key("enter");
    harness.key("ctrl-r");
    let frame = harness.render();
    assert!(frame.contains("repos of htui"), "the picker: {frame}");
    assert!(
        frame.contains("core") && frame.contains("web"),
        "the project's repos: {frame}"
    );
    insta::assert_snapshot!("repo_picker", frame);
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains("globs       core:"),
        "the qualifier went in at the globs cursor: {frame}"
    );
    assert!(
        !frame.contains("repos of htui"),
        "back on the form: {frame}"
    );
}

#[tokio::test]
async fn a_save_over_a_moved_head_keeps_the_draft() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "rust-style");
    harness.key("e");
    harness.key("X");
    let elsewhere = store
        .add_skill_version(
            ids::SKILL_RUST_STYLE,
            2,
            NewSkillVersion {
                body: "Saved elsewhere.".to_owned(),
                source: serde_json::json!({}),
                created_by: ids::USER,
            },
        )
        .await
        .expect("the direct write");
    assert!(matches!(elsewhere, CasOutcome::Applied(ref row) if row.version == 3));
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("v3 is now the latest")
            && notice(&frame).contains("saves it as v4"),
        "the stale notice: {frame}"
    );
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "still editing: {frame}"
    );
    assert!(frame.contains("XPrefer"), "the draft is kept: {frame}");
    assert!(
        frame.contains("saves v4"),
        "the token moved to the new head: {frame}"
    );
    let bodies: Vec<String> = versions(&store, "rust-style")
        .await
        .into_iter()
        .map(|row| row.body)
        .collect();
    assert_eq!(
        bodies.last().map(String::as_str),
        Some("Saved elsewhere."),
        "the stale save wrote nothing"
    );
    insta::assert_snapshot!("changed_elsewhere", frame);
}

// --- asserts -----------------------------------------------------------------------------------

#[tokio::test]
async fn a_new_skill_needs_a_valid_name_and_a_body() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    harness.key("n");
    type_text(&mut harness, "Docs");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("skill.name `Docs`"),
        "the name is refused: {frame}"
    );
    for _ in 0..4 {
        harness.key("backspace");
    }
    type_text(&mut harness, "docs-style");
    harness.key("enter");
    type_text(&mut harness, "How docs read");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains("docs-style \u{b7} new, saves v1"),
        "the editor opens for v1: {frame}"
    );
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("a skill needs text"),
        "a blank body is refused: {frame}"
    );
    assert!(
        versions(&store, "docs-style").await.is_empty(),
        "nothing was sent"
    );
    type_text(&mut harness, "Write in the active voice.");
    harness.key("ctrl-s");
    harness.settle().await;
    let saved = versions(&store, "docs-style").await;
    assert_eq!(
        saved
            .iter()
            .map(|row| (row.version, row.body.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "Write in the active voice.")]
    );
    let skills = store.skills().await.expect("the library read");
    let created = skills
        .iter()
        .find(|skill| skill.name == "docs-style")
        .expect("the new skill");
    assert_eq!(created.description, "How docs read");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("created `docs-style` v1"),
        "{frame}"
    );
    assert!(
        frame.contains("\u{250c} docs-style v1 (head v1)"),
        "the cursor is on the new skill: {frame}"
    );
    assert!(
        !hint(&frame).contains("Ctrl+S save"),
        "back in Browse: {frame}"
    );
}

#[tokio::test]
async fn an_existing_name_is_refused_at_naming() {
    let mut harness = open().await;
    harness.key("n");
    type_text(&mut harness, "tests");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("`tests` exists \u{2014} select it and press e"),
        "{frame}"
    );
    assert!(!frame.contains("saves v1"), "no editor opened: {frame}");
}

#[tokio::test]
async fn a_saved_version_moves_the_head_and_the_estimate() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "tests");
    let frame = harness.render();
    assert!(
        frame.contains("tests v1 (head v1) \u{b7} ~32 tokens"),
        "{frame}"
    );
    harness.key("e");
    type_text(&mut harness, "Always name the case. ");
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    let title = line_with(&frame, "tests v2 (head v2)");
    let estimate = tokens(&title);
    assert!(estimate > 32, "the estimate moved up: {title}");
    assert!(
        notice(&frame).contains(&format!("saved v2 \u{b7} ~{estimate} tokens")),
        "the notice repeats the estimate: {frame}"
    );
    assert_eq!(
        versions(&store, "tests")
            .await
            .last()
            .map(|row| (row.version, row.body.clone())),
        Some((
            2,
            "Always name the case. Name the case after the rule it pins.".to_owned()
        ))
    );
}

#[tokio::test]
async fn the_info_form_renames_under_compare_and_set() {
    let store = MemStore::demo();
    let mut harness = open_over(store.clone()).await;
    select(&mut harness, "tests");
    harness.key("i");
    let frame = harness.render();
    assert!(frame.contains("tests \u{b7} rename"), "the form: {frame}");
    for _ in 0..5 {
        harness.key("backspace");
    }
    harness.key("enter");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("skill.name ``"),
        "an empty name is refused before sending: {frame}"
    );
    type_text(&mut harness, "test-rules");
    harness.key("ctrl-s");
    harness.settle().await;
    let names: Vec<String> = store
        .skills()
        .await
        .expect("the library read")
        .into_iter()
        .map(|skill| skill.name)
        .collect();
    assert_eq!(names, ["rust-style", "test-rules"]);
    let frame = harness.render();
    assert!(notice(&frame).contains("saved `test-rules`"), "{frame}");
    assert!(
        frame.contains("\u{250c} test-rules v1 (head v1)"),
        "the pane follows the renamed row: {frame}"
    );
}

#[tokio::test]
async fn the_winning_row_is_starred_per_project() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    select(&mut harness, "rust-style");
    harness.key("a");
    let frame = harness.render();
    assert!(
        line_with(&frame, " htui ").starts_with("* htui"),
        "the project row wins for htui's other phases: {frame}"
    );
    assert!(
        line_with(&frame, "feature \u{203a} implement")
            .starts_with("*   feature \u{203a} implement"),
        "the phase row wins for its phase: {frame}"
    );
    assert!(
        line_with(&frame, "global").starts_with("global"),
        "no star on the empty global row: {frame}"
    );
    assert!(
        line_with(&frame, " agy ").starts_with("agy"),
        "nothing wins for agy: {frame}"
    );

    set_binding(
        &store,
        SkillBindingKey {
            skill: ids::SKILL_RUST_STYLE,
            project: None,
            phase: None,
        },
        None,
        BindingChange::Attach(always()),
    )
    .await;
    harness.key("r");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        line_with(&frame, "global").starts_with("* global"),
        "the global row wins for agy's phases: {frame}"
    );
    assert!(
        line_with(&frame, " htui ").starts_with("* htui"),
        "htui's project row still wins there: {frame}"
    );
    assert!(
        line_with(&frame, " agy ").starts_with("agy"),
        "agy's own row is empty: {frame}"
    );
}

#[tokio::test]
async fn a_glob_row_says_it_fires_from_milestone_5() {
    let store = MemStore::demo();
    let row = htui_row(&store, ids::SKILL_TESTS)
        .await
        .expect("the demo `tests` row");
    set_binding(
        &store,
        SkillBindingKey::of(&row),
        Some(row.updated_at),
        BindingChange::Attach(Attachment {
            activation: Activation::Glob,
            languages: vec!["rust".to_owned()],
            ..always()
        }),
    )
    .await;
    let mut harness = open_platform_over(store).await;
    select(&mut harness, "tests");
    harness.key("a");
    let frame = harness.render();
    assert!(
        line_with(&frame, " htui ").contains(
            "glob \u{b7} latest \u{b7} pos 0 \u{b7} 2 globs \u{b7} fires from milestone 5"
        ),
        "{frame}"
    );
}

#[tokio::test]
async fn detach_asks_first() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    select(&mut harness, "tests");
    harness.key("a");
    harness.key("j");
    harness.key("x");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("detach `tests` from htui?"),
        "{frame}"
    );
    harness.key("n");
    harness.settle().await;
    let frame = harness.render();
    assert!(notice(&frame).contains("kept"), "{frame}");
    assert!(
        htui_row(&store, ids::SKILL_TESTS).await.is_some(),
        "any key but `y` keeps it"
    );
    harness.key("x");
    harness.key("y");
    harness.settle().await;
    assert!(
        htui_row(&store, ids::SKILL_TESTS).await.is_none(),
        "`y` detached it"
    );
    let frame = harness.render();
    assert!(notice(&frame).contains("detached from htui"), "{frame}");
}

#[tokio::test]
async fn an_attachment_saved_in_the_form_lands_and_the_form_closes() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    select(&mut harness, "tests");
    harness.key("a");
    harness.key("j");
    harness.key("enter");
    harness.key("space");
    harness.key("tab");
    harness.key("backspace");
    harness.key("backspace");
    harness.key("backspace");
    harness.key("backspace");
    harness.key("backspace");
    harness.key("backspace");
    type_text(&mut harness, "1");
    harness.key("tab");
    harness.key("backspace");
    type_text(&mut harness, "3");
    harness.key("tab");
    type_text(&mut harness, "Rust");
    harness.key("ctrl-s");
    harness.settle().await;
    let row = htui_row(&store, ids::SKILL_TESTS)
        .await
        .expect("the row is still there");
    assert_eq!(
        (
            row.activation,
            row.pinned_version,
            row.position,
            row.languages.clone(),
            row.globs.clone()
        ),
        (
            Activation::Glob,
            Some(1),
            3,
            vec!["rust".to_owned()],
            vec!["**/*.rs".to_owned(), "**/Cargo.toml".to_owned()]
        )
    );
    let frame = harness.render();
    assert!(notice(&frame).contains("attached to htui"), "{frame}");
    assert!(
        line_with(&frame, " htui ").contains("glob \u{b7} v1 \u{b7} pos 3 \u{b7} 2 globs"),
        "the row shows what landed: {frame}"
    );
    assert!(
        !frame.contains("effective:"),
        "the form closed on landing: {frame}"
    );

    // Reopened, the form shows the languages and no typed glob (D102).
    harness.key("enter");
    let frame = harness.render();
    assert!(frame.contains("languages   rust"), "{frame}");
    assert!(
        frame.contains("effective:  **/*.rs, **/Cargo.toml"),
        "{frame}"
    );
    assert!(
        line_with(&frame, "globs       ") == "globs",
        "the stored globs minus the language's are empty: {frame}"
    );
}

#[tokio::test]
async fn a_glob_naming_no_repo_is_shown_before_save_and_refused_by_the_store() {
    let store = MemStore::demo();
    let mut harness = open_platform_over(store.clone()).await;
    select(&mut harness, "tests");
    harness.key("a");
    harness.key("j");
    harness.key("enter");
    for _ in 0..4 {
        harness.key("tab");
    }
    type_text(&mut harness, "nosuchrepo:**/*.rs");
    let frame = harness.render();
    assert!(
        frame.contains("names repo `nosuchrepo`, which project `htui` does not have"),
        "the store's sentence, before save: {frame}"
    );
    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains("which project `htui` does not have"),
        "the store refused it: {frame}"
    );
    assert!(frame.contains("effective:"), "the form is kept: {frame}");
    assert_eq!(
        htui_row(&store, ids::SKILL_TESTS)
            .await
            .map(|row| row.globs),
        Some(Vec::new()),
        "nothing was written"
    );
}

#[tokio::test]
async fn a_global_attachment_refuses_ctrl_r() {
    let mut harness = open_platform_over(MemStore::demo()).await;
    select(&mut harness, "tests");
    harness.key("a");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains("attach tests \u{b7} global"),
        "a new attachment's form: {frame}"
    );
    harness.key("ctrl-r");
    let frame = harness.render();
    assert!(
        notice(&frame).contains("a global attachment's globs cannot name a repo"),
        "{frame}"
    );
}

#[tokio::test]
async fn external_edit_is_requested_with_the_shown_body() {
    let mut harness = open().await;
    select(&mut harness, "rust-style");
    harness.key(",");
    harness.key("E");
    let Some((tab, edit)) = harness.app().take_external_edit() else {
        panic!("`E` asked for the editor");
    };
    assert_eq!(tab, SkillsTab::ID);
    assert_eq!(
        edit,
        ExternalEdit {
            text: "Prefer `expect` with a reason.".to_owned(),
            stem: "rust-style".to_owned(),
        }
    );
    harness.app().finish_external_edit(
        SkillsTab::ID,
        ExternalEditOutcome::Edited("Prefer `expect`.\n".to_owned()),
    );
    let frame = harness.render();
    assert!(notice(&frame).contains("edited in $EDITOR"), "{frame}");
    assert!(
        frame.contains("rust-style \u{b7} editing from v1, saves v3"),
        "the editor is open on the returned text: {frame}"
    );
}

#[tokio::test]
async fn tab_and_digits_still_switch_tabs_with_a_draft_open() {
    let mut harness = open().await;
    select(&mut harness, "rust-style");
    harness.key("e");
    harness.key("2");
    assert!(
        harness.render().contains("2Prefer"),
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
    assert!(frame.contains("2Prefer"), "the draft survived: {frame}");
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
