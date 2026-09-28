//! The Skills tab's attachments matrix (MOD-9 milestone 3, T4): one row per level, one column per
//! skill, the activation form, the language expansion, the repo picker and the unbind, through a
//! `Harness` over the demo world.
//!
//! The Harness enters the Graphics workspace at startup, so the scope is one project,
//! `vulkan-tutorials`, and the level list is the global row, the project row and that project's
//! fifteen seeded phases — which is where the matrix reads the catalogue from. The demo attaches
//! its skills to `htui` and `agy`, both in the **Platform** workspace, so a scope-level test plants
//! the rows it needs with a direct write: `MemStore` shares its state across clones, and every
//! read and write **the view** makes goes through `StoreRequest`, so "nothing was sent" is checked
//! on the store — a request that was sent is served by `settle` and would have written a row.
#![cfg(feature = "testkit")]

use chrono::{DateTime, Utc};
use htui::app::register_all;
use htui::testkit::Harness;
use htui_core::fixtures::ids;
use htui_core::model::{
    Activation, NewRepo, NewSkillBinding, PhaseId, ProjectId, RepoId, SkillAttachmentRow,
    SkillBindingId, SkillId,
};
use htui_core::prompt::glob;
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

/// Opens the attachments matrix and lets the catalogue read land. The matrix is a **mode** of the
/// Skills view rather than a third name on the switch line, because that line is milestone 1's and
/// byte-identical in the six `templates__*.snap` files (H-15).
async fn matrix(mut harness: Harness) -> Harness {
    harness.key("m");
    harness.settle().await;
    harness
}

/// The demo world with the matrix open.
async fn open_matrix() -> Harness {
    matrix(open().await).await
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

/// One matrix line as `render` draws it, so the assertions are the layout rather than a count of
/// spaces someone typed by hand. Trailing blanks are trimmed because the test backend trims them.
fn row(label: &str, cells: &[&str]) -> String {
    let mut line = format!("  {label:<18}");
    for cell in cells {
        line.push_str(&format!("{cell:<13}"));
    }
    line.trim_end().to_owned()
}

/// The two demo skills' columns, in the header's order: `skill.name` byte order.
const SKILLS: [&str; 2] = ["rust-style", "tests"];

/// The label column's header, which `render` draws once above the level rows.
const LEVEL_HEADER: &str = "level";

/// An empty cell for both columns.
const NONE: [&str; 2] = ["\u{b7}", "\u{b7}"];

/// One matrix line with the two demo columns' cells spelled out.
fn row_text(label: &str, first: &str, second: &str) -> String {
    row(label, &[first, second])
}

/// Walks the level cursor down until the label column reads `label`, so a fixture that grows or
/// loses a phase does not silently move this helper's callers onto the wrong row.
fn go_to(harness: &mut Harness, label: &str) {
    for _ in 0..40 {
        if harness.render().contains(&row(label, &NONE)) {
            return;
        }
        harness.key("j");
    }
    panic!("`{label}` is not a level of this scope:\n{}", harness.render());
}

/// The attachments of the Graphics scope, as another session would read them.
async fn attachments(store: &MemStore) -> Vec<SkillAttachmentRow> {
    store
        .skill_attachments(&[ids::PROJECT_VULKAN])
        .await
        .unwrap_or_else(|err| panic!("the attachment read failed: {err}"))
}

/// The one attachment of the Graphics scope, or `None`.
async fn only(store: &MemStore) -> Option<SkillAttachmentRow> {
    let mut rows = attachments(store).await;
    assert!(rows.len() <= 1, "the test planted one row: {rows:?}");
    rows.pop()
}

/// Writes one attachment straight into the shared store, as another session would.
async fn bind(
    store: &MemStore,
    skill_id: SkillId,
    project: Option<ProjectId>,
    phase: Option<PhaseId>,
    position: i32,
    expected: Option<DateTime<Utc>>,
) {
    let outcome = store
        .set_skill_binding(
            NewSkillBinding {
                id: SkillBindingId::new(),
                skill_id,
                project_id: project,
                phase_id: phase,
                pinned_version: None,
                position,
                activation: Activation::Always,
                globs: Vec::new(),
                languages: Vec::new(),
            },
            expected,
        )
        .await
        .expect("the direct write");
    assert!(
        matches!(outcome, CasOutcome::Applied(_)),
        "the direct bind over {expected:?} was not applied"
    );
}

// --- snapshots ---------------------------------------------------------------------------------

/// D82: one row per level, the global row above the projects and the phases, and one column per
/// skill — packed with `format!` into a bordered block, not a `ratatui::widgets::Table`, because
/// the three `Table` users all have a row cursor and this one selects a cell. The switch line and
/// the strip text are the shell's and do not move (H-15, H-28).
#[tokio::test]
async fn the_matrix_shows_a_global_row_above_the_projects_and_the_phases() {
    let mut harness = open_matrix().await;
    let frame = harness.render();

    assert!(
        frame.contains("\u{250c} Attachments \u{2500}"),
        "the list's own title, so a widened `LIST_WIDTH` would be a diff here: {frame}"
    );
    assert!(
        frame.contains(" Skills \u{2502} Templates"),
        "H-15: the switch line is milestone 1's and byte-identical in the six `templates__*.snap` \
         files, which is why the matrix is a mode and not a third name: {frame}"
    );
    assert!(
        frame.contains(" 1 Backlog  2 Skills  3 Settings  4 Chat"),
        "and the strip text is the shell's: {frame}"
    );
    let header = row(LEVEL_HEADER, &["rust-style", "tests"]);
    assert!(frame.contains(&header), "the skill columns: {frame}");
    // The whole level list, in the order the rows are drawn: the global row first, then the
    // scope's project, then that project's phases by graph name and phase position. The seed is
    // fifteen phases across five graphs (`seed::KINDS`), and the list is the read's own order, so
    // this is the matrix's whole shape in one assertion.
    let levels = [
        "global",
        "vulkan-tutorials",
        "analysis/research",
        "analysis/verdict",
        "bug/reproduce",
        "bug/fix",
        "bug/review",
        "feature/prd",
        "feature/plan",
        "feature/implement",
        "feature/review",
        "refactor/plan",
        "refactor/implement",
        "refactor/review",
        "tooling/plan",
        "tooling/implement",
        "tooling/review",
    ];
    let mut previous = 0;
    for label in levels {
        let at = frame
            .find(&row(label, &NONE))
            .unwrap_or_else(|| panic!("`{label}` is a row of the matrix:\n{frame}"));
        assert!(
            at > previous,
            "`{label}` is drawn after the level above it: {at} is not after {previous}"
        );
        previous = at;
    }
    insta::assert_snapshot!("overview", frame);
}

/// D83: the form shows the **effective** globs — the typed globs unioned with every named
/// language's expansion — before the save, and the save writes exactly that union, with
/// `languages` keeping what was typed. `shell` is the three-pattern language (F-10: `rust` is one).
#[tokio::test]
async fn the_language_map_expands_into_the_effective_globs_shown_before_the_save() {
    let store = MemStore::demo();
    let mut harness = open_matrix().await;

    harness.key("e");
    type_text(&mut harness, "docs/**");
    harness.key("tab");
    type_text(&mut harness, "shell");
    let frame = harness.render();
    assert!(
        frame.contains(" effective globs: "),
        "the preview block, which is what makes the expansion visible before the write: {frame}"
    );
    for glob in ["docs/**", "**/*.sh", "**/*.bash", "**/*.zsh"] {
        assert!(
            frame.contains(glob),
            "`{glob}` is one of the four the union is made of, in the union's order: {frame}"
        );
    }
    let preview = frame.find("docs/**").expect("the typed glob");
    let sh = frame.find("**/*.sh").expect("the first expansion");
    let bash = frame.find("**/*.bash").expect("the second");
    let zsh = frame.find("**/*.zsh").expect("the third");
    assert!(
        preview < sh && sh < bash && bash < zsh,
        "H-33: the preview is in `effective_globs`' own order — typed first, then each language's \
         patterns in the order it wrote them: {frame}"
    );
    insta::assert_snapshot!("effective_globs", frame);

    harness.key("ctrl-s");
    harness.settle().await;
    let row = only(&store).await.expect("the row the form wrote");
    assert_eq!(
        row.globs,
        ["docs/**", "**/*.sh", "**/*.bash", "**/*.zsh"],
        "the store receives the union the form previewed, so the two cannot disagree"
    );
    assert_eq!(
        row.languages,
        ["shell"],
        "and `languages` keeps what was typed, so a later map change never moves a saved attachment"
    );
}

// --- asserts -----------------------------------------------------------------------------------

/// D78: `a` writes one row at the selected level and the reply re-reads the whole scope, so the
/// cell is drawn from the store rather than patched in locally.
#[tokio::test]
async fn attaching_at_a_level_writes_one_row_and_the_matrix_re_reads() {
    let store = MemStore::demo();
    let mut harness = matrix(open_over(store.clone()).await).await;
    go_to(&mut harness, "vulkan-tutorials");

    harness.key("a");
    harness.settle().await;

    let row = only(&store).await.expect("the new row");
    assert_eq!(row.skill_id, ids::SKILL_RUST_STYLE, "the first column");
    assert_eq!(row.project_id, Some(ids::PROJECT_VULKAN));
    assert_eq!(row.phase_id, None, "a project row joins no phase");
    assert_eq!(row.activation, Activation::Always, "the default");
    assert_eq!(row.pinned_version, None, "and it follows the latest");
    assert_eq!(row.position, 0);
    let frame = harness.render();
    assert!(
        notice(&frame).contains("attached rust-style at vulkan-tutorials"),
        "the notice names the skill and the level the user was looking at: {frame}"
    );
    assert!(
        frame.contains(&row_text("vulkan-tutorials", "v2", "\u{b7}")),
        "the cell is the version in force — `rust-style`'s head — and the other column is still \
         empty: {frame}"
    );
}

/// `p` cycles `latest → v1 → v2 → latest`, and the cell shows which version is in force.
#[tokio::test]
async fn a_pin_follows_latest_until_it_is_set_and_cleared() {
    let store = MemStore::demo();
    let mut harness = matrix(open_over(store.clone()).await).await;
    go_to(&mut harness, "vulkan-tutorials");
    harness.key("a");
    harness.settle().await;
    assert_eq!(only(&store).await.expect("the row").pinned_version, None);

    harness.key("p");
    assert!(
        hint(&harness.render()).contains("Ctrl+S save"),
        "`p` opens the form over the cell and cycles the pin there, because a cycle applied to the \
         cell itself would walk straight into the `glob`-needs-a-glob refusal: {}",
        hint(&harness.render())
    );
    harness.key("ctrl-s");
    harness.settle().await;
    let row = only(&store).await.expect("the row");
    assert_eq!(row.pinned_version, Some(1), "the first `p` pins v1");
    assert!(
        harness.render().contains(&row_text("vulkan-tutorials", "v1", "\u{b7}")),
        "and the cell says v1, not the head: {}",
        harness.render()
    );

    harness.key("p");
    harness.key("ctrl-s");
    harness.settle().await;
    assert_eq!(
        only(&store).await.expect("the row").pinned_version,
        None,
        "the second `p` is back to latest"
    );
}

/// D78: `activation = glob` with no glob is refused, in the writer's own sentence, and **nothing
/// is sent** — the store is the only thing that could tell the two apart.
#[tokio::test]
async fn activating_glob_without_globs_is_refused_before_it_is_sent() {
    let store = MemStore::demo();
    let mut harness = open_matrix().await;
    // The `tests` column on the global row: attached nowhere, so the form opens on the defaults.
    harness.key("right");
    harness.key("e");
    harness.key("A");
    assert!(
        harness.render().contains(" activation: glob "),
        "`A` cycles `always → glob → off` in the form: {}",
        harness.render()
    );

    harness.key("ctrl-s");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        notice(&frame).contains(
            "skill_binding.globs must name at least one glob when activation is `glob` \
             (skill_binding_glob_needs_globs)"
        ),
        "the writer's own sentence reaches the view, which is what D100 is for: {frame}"
    );
    assert!(
        attachments(&store).await.is_empty(),
        "nothing was sent, so the store holds no row"
    );
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "and the form stays open, so a glob is one `Tab` and a few keys away: {frame}"
    );
}

/// D78 / ANA-22 §6 item 6: a repo-qualified glob is refused on a **global** row, which no CHECK
/// can hold and which therefore exists only in the writer — and, by D100, in the view too.
#[tokio::test]
async fn a_qualified_glob_is_refused_on_a_global_row() {
    let store = MemStore::demo();
    let mut harness = open_matrix().await;
    harness.key("e");
    type_text(&mut harness, "tutorials:**/*.rs");
    harness.key("ctrl-s");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        notice(&frame).contains(
            "skill_binding.globs `tutorials:**/*.rs` names a repo, and a global attachment applies \
             to every project"
        ),
        "the writer's sentence, from the row the user was on: {frame}"
    );
    assert!(
        attachments(&store).await.is_empty(),
        "nothing was sent"
    );
}

/// D100: the three patterns the plan names are refused **before** the request, each with
/// `GlobError`'s own `Display` — the very string the writer would have put in its `Constraint`, so
/// the view's notice and the store's refusal cannot drift about the same bytes.
#[tokio::test]
async fn a_glob_the_matcher_cannot_compile_is_refused_before_it_is_sent() {
    let store = MemStore::demo();
    for pattern in ["src/**x/*.rs", "a/{b,{c,d}}/x.rs", "x{,.txt}"] {
        let mut harness = open_matrix().await;
        harness.key("e");
        type_text(&mut harness, pattern);
        harness.key("ctrl-s");
        harness.settle().await;

        let why = glob::compile(pattern)
            .expect_err("the plan names three the matcher refuses")
            .to_string();
        let frame = harness.render();
        assert!(
            notice(&frame).contains(&why),
            "`{pattern}` is refused with the matcher's own sentence, `{why}`: {frame}"
        );
        assert!(
            attachments(&store).await.is_empty(),
            "`{pattern}` was refused before the request, so nothing was sent"
        );
    }
}

/// OQ-19: `x` sends `RemoveSkillBinding` with the row's own `updated_at`, and the reply's
/// re-read shows the cell back to `·` — the row is gone, not an attachment reading as `off`.
#[tokio::test]
async fn unbinding_removes_the_row_and_the_matrix_shows_it_gone() {
    let store = MemStore::demo();
    bind(
        &store,
        ids::SKILL_TESTS,
        Some(ids::PROJECT_VULKAN),
        None,
        0,
        None,
    )
    .await;
    let mut harness = matrix(open_over(store.clone()).await).await;
    go_to(&mut harness, "vulkan-tutorials");
    harness.key("right");
    let frame = harness.render();
    assert!(
        frame.contains(&row_text("vulkan-tutorials", "\u{b7}", "v1")),
        "the `tests` cell holds the row: {frame}"
    );

    harness.key("x");
    harness.settle().await;

    assert!(
        attachments(&store).await.is_empty(),
        "the row is gone rather than written `activation = off`"
    );
    let frame = harness.render();
    assert!(
        notice(&frame).contains("detached tests from vulkan-tutorials"),
        "the notice names both: {frame}"
    );
    assert!(
        frame.contains(&row("vulkan-tutorials", &NONE)),
        "and the cell is back to `·`: {frame}"
    );
    insta::assert_snapshot!("unbound", frame);
}

/// D78 / R-32: an unbind is as safe as every other write. A spent `updated_at` answers
/// `SkillsStale`, the row another writer changed survives, and the form is dropped so what the
/// user sees is the store and not the draft that lost the race.
#[tokio::test]
async fn a_spent_token_leaves_the_row_as_it_is_and_says_so() {
    let store = MemStore::demo();
    bind(
        &store,
        ids::SKILL_TESTS,
        Some(ids::PROJECT_VULKAN),
        None,
        0,
        None,
    )
    .await;
    let token = only(&store).await.expect("the row").updated_at;
    // Another session moves the row, so the token the form will carry is spent.
    bind(
        &store,
        ids::SKILL_TESTS,
        Some(ids::PROJECT_VULKAN),
        None,
        7,
        Some(token),
    )
    .await;

    let mut harness = matrix(open_over(store.clone()).await).await;
    go_to(&mut harness, "vulkan-tutorials");
    harness.key("right");
    harness.key("e");
    // The form opened on the row the *read* answered, which is the one the other session wrote.
    // Move it again, so the token this form holds is the one just spent.
    bind(
        &store,
        ids::SKILL_TESTS,
        Some(ids::PROJECT_VULKAN),
        None,
        9,
        Some(token),
    )
    .await;
    harness.key("ctrl-s");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        notice(&frame).contains("this attachment changed elsewhere; it is unchanged"),
        "the spent token's own sentence: {frame}"
    );
    assert_eq!(
        only(&store).await.expect("the surviving row").position,
        9,
        "the row is as the writer that won left it"
    );
    assert!(
        !hint(&frame).contains("Ctrl+S save"),
        "and the form is gone, so the frame is the store rather than a draft that lost: {frame}"
    );
}

/// The picker writes the `<repo>:` qualifier from the **project's own repos**, read from the
/// hierarchy. The demo fixture holds no `repo` row at all, so the test creates one the way a user
/// would have.
#[tokio::test]
async fn a_repo_picker_writes_the_qualifier_from_the_project_s_repos() {
    let store = MemStore::demo();
    store
        .create_repo(NewRepo {
            id: RepoId::new(),
            project_id: ids::PROJECT_VULKAN,
            name: "tutorials".to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: true,
        })
        .await
        .expect("the direct write");
    let mut harness = matrix(open_over(store.clone()).await).await;
    go_to(&mut harness, "vulkan-tutorials");
    harness.key("e");
    type_text(&mut harness, "**/*.rs");
    harness.key("R");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("every repo") && frame.contains("tutorials"),
        "the picker lists the project's repos under the bare option: {frame}"
    );

    harness.key("j");
    harness.key("enter");
    let frame = harness.render();
    assert!(
        frame.contains("tutorials:**/*.rs"),
        "the qualifier the picker wrote is prepended to the typed glob, so the globs the form \
         shows are the globs the store receives: {frame}"
    );
    assert!(
        !frame.contains(" every repo: yes "),
        "and the qualifier line now names the repo: {frame}"
    );

    harness.key("ctrl-s");
    harness.settle().await;
    assert_eq!(
        only(&store).await.expect("the row").globs,
        ["tutorials:**/*.rs"],
        "and the store received it"
    );
}

/// A global attachment cannot name a repo, so the picker is refused there before it is even asked
/// for — the same rule the writer enforces on a qualified glob, said the same way.
#[tokio::test]
async fn the_repo_picker_is_refused_on_a_global_row() {
    let store = MemStore::demo();
    let mut harness = open_matrix().await;
    harness.key("e");
    harness.key("R");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("a global attachment cannot name a repo"),
        "the refusal names the rule: {frame}"
    );
    assert!(
        !frame.contains("every repo"),
        "and no picker was opened: {frame}"
    );
}

/// `Esc` closes the matrix and hands the tab back to the library view, which is where `m` came
/// from.
#[tokio::test]
async fn esc_closes_the_matrix_and_the_library_is_there() {
    let mut harness = open_matrix().await;
    assert!(
        harness.render().contains("\u{250c} Attachments "),
        "the matrix is showing"
    );

    harness.key("esc");
    let frame = harness.render();
    assert!(
        !frame.contains("\u{250c} Attachments "),
        "and after `Esc` it is not: {frame}"
    );
    assert!(
        frame.contains("\u{250c} Skills \u{2500}"),
        "the library view is back, with its own widths (D106, H-27): {frame}"
    );

    harness.key("m");
    let frame = harness.render();
    assert!(
        frame.contains("\u{250c} Attachments "),
        "and `m` opens it again: {frame}"
    );
}

/// The form is a mode of the matrix, so the tab's own `h`/`l` are letters inside it rather than
/// the view switch — the per-view `captures_input` trade the Templates view already makes (H-32).
#[tokio::test]
async fn an_open_form_keeps_the_views_switch_off() {
    let mut harness = open_matrix().await;
    harness.key("e");
    type_text(&mut harness, "hello");

    harness.key("l");
    let frame = harness.render();
    assert!(
        frame.contains("\u{250c} Attachments "),
        "`l` did not switch the view out from under the form: {frame}"
    );
    assert!(
        frame.contains("hello"),
        "and it typed an `l` where it was aimed: {frame}"
    );
}
