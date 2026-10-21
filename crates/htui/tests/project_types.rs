//! `Settings > Project types` (MOD-146 D8): the read-only `project_type` registry, driven through
//! a [`SectionBench`] with replies built from the memory store's six built-ins, and the frames a
//! user sees through a [`Harness`].
#![cfg(feature = "testkit")]

use htui::app::Action;
use htui::store_worker::{StoreReply, StoreRequest};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::project_types::{self, UNAVAILABLE};
use htui::ui::tabs::settings::{ProjectTypesSection, SettingsSection, SettingsTab};
use htui_core::model::{ProjectType, Scope};
use htui_core::store::{MemStore, WriteStore as _};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};

/// The browse hint with the default keys.
const BROWSE_KEYS: &str = "j/k select";

/// The registry the memory store answers: the six built-ins in slug order.
async fn demo_rows() -> Vec<ProjectType> {
    MemStore::demo()
        .project_types()
        .await
        .expect("the memory store never fails")
}

/// The scope of the demo fixture's first workspace.
async fn demo_scope() -> Scope {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    Scope::from_workspace(workspaces.first().expect("the fixture has a workspace"))
}

/// A bench and a section `rows` have been delivered to.
async fn bench_with(rows: Vec<ProjectType>) -> (SectionBench, ProjectTypesSection) {
    let bench = SectionBench::new().await;
    let mut section = ProjectTypesSection::new();
    bench.reply(&mut section, &StoreReply::ProjectTypes(rows));
    let _ = bench.drained();
    (bench, section)
}

/// A bench handing the section `file`'s keys instead of the compiled defaults, the registry
/// delivered.
async fn bench_with_keys(file: &str) -> (SectionBench, ProjectTypesSection) {
    let keys = htui::keys::load_str(file).expect("the key file loads");
    let bench = SectionBench::new().await.with_keys(keys);
    let mut section = ProjectTypesSection::new();
    bench.reply(&mut section, &StoreReply::ProjectTypes(demo_rows().await));
    let _ = bench.drained();
    (bench, section)
}

/// The store requests the section emitted since the last drain.
fn requests(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// The section drawn at 100x30, as text.
fn frame(bench: &SectionBench, section: &ProjectTypesSection) -> String {
    bench.render_section(section, 100)
}

/// The frame's lines, trailing blanks cut.
fn lines(frame: &str) -> Vec<String> {
    frame
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect()
}

/// The last line that holds anything.
fn last_line(frame: &str) -> String {
    lines(frame)
        .into_iter()
        .rfind(|line| !line.is_empty())
        .unwrap_or_default()
}

/// The frame's words joined by single spaces, so a wrapped sentence reads as one.
fn flat(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the frame holds `sentence`, wrapped or not.
fn shows(frame: &str, sentence: &str) -> bool {
    flat(frame).contains(&flat(sentence))
}

/// D8: one line per type, the cursor row's description under it, `coding` on Software and
/// `not selectable` on Workspace.
#[tokio::test]
async fn the_registry_lists_one_line_per_type() {
    let (bench, section) = bench_with(demo_rows().await).await;
    let frame = frame(&bench, &section);
    let rows = lines(&frame);
    assert_eq!(rows[0], "Documents \u{b7} docs", "{frame}");
    assert_eq!(
        rows[1], "    Documents outlined, drafted and reviewed.",
        "{frame}"
    );
    assert_eq!(rows[2], "General \u{b7} general", "{frame}");
    assert!(
        rows.contains(&"Software \u{b7} software \u{b7} coding".to_owned()),
        "{frame}"
    );
    assert!(
        rows.contains(&"Workspace \u{b7} workspace \u{b7} not selectable".to_owned()),
        "{frame}"
    );
    assert!(bench.key_hint(&section).contains(BROWSE_KEYS), "{frame}");
    insta::assert_snapshot!("browse", frame);
}

/// D8: `j`/`k` move the cursor through the shared list acts, and stop at the ends.
#[tokio::test]
async fn j_and_k_move_the_cursor_and_stop_at_the_ends() {
    let (bench, mut section) = bench_with(demo_rows().await).await;
    bench.key(&mut section, "k");
    let top = lines(&frame(&bench, &section));
    assert_eq!(top[0], "Documents \u{b7} docs");
    assert_eq!(top[1], "    Documents outlined, drafted and reviewed.");

    for _ in 0..7 {
        bench.key(&mut section, "j");
    }
    let bottom = frame(&bench, &section);
    let rows = lines(&bottom);
    assert_eq!(
        rows[0], "Documents \u{b7} docs",
        "no description under it: {bottom}"
    );
    assert_eq!(rows[1], "General \u{b7} general", "{bottom}");
    assert_eq!(
        last_line(&bottom),
        "    A workspace's own items; not offered for new projects.",
        "the cursor is on Workspace, the last row: {bottom}"
    );
    assert!(requests(&bench).is_empty(), "a move reads nothing");
}

/// D8: the read is unscoped, and a scope change moves nothing.
#[tokio::test]
async fn wants_requests_is_the_unscoped_read() {
    let (bench, mut section) = bench_with(demo_rows().await).await;
    let scope = demo_scope().await;
    let wanted = section.wants_requests(&scope);
    assert_eq!(wanted.len(), 1, "{wanted:?}");
    assert!(
        matches!(wanted[0], StoreRequest::ProjectTypes),
        "{wanted:?}"
    );
    bench.key(&mut section, "j");
    let before = frame(&bench, &section);
    section.on_scope_change(&scope);
    assert_eq!(frame(&bench, &section), before);
}

/// A refused read says "unavailable" with the store's sentence, and offers no key.
#[tokio::test]
async fn a_refused_read_says_unavailable_and_offers_no_keys() {
    let (bench, mut section) = bench_with(demo_rows().await).await;
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "project_types",
            message: "x".to_owned(),
        },
    );
    let frame = frame(&bench, &section);
    assert!(shows(&frame, &format!("{UNAVAILABLE}: x")), "{frame}");
    assert!(shows(&frame, "project types unavailable: x"), "{frame}");
    assert!(!bench.key_hint(&section).contains("j/k"), "{frame}");
}

/// `project_type` is not mirrored: offline the read is refused, the section says so and offers
/// no key.
#[tokio::test]
async fn offline_the_section_says_unavailable_and_offers_no_keys() {
    // `App::start` issues `ConnectionInfo`, which over a non-`Memory` backend reaches the keyring.
    let _keyring = htui_store::testkit::mock_keyring().await;
    // The mirror outlives the harness: dropping the directory deletes it mid-test.
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(
        root.path(),
        "project-types-section",
        PgStore::schema_version(),
    )
    .await
    .expect("a fresh mirror");
    let mut harness = Harness::over_backend(Backend::Offline {
        cache,
        since: Some(chrono::Utc::now()),
    })
    .with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
        ProjectTypesSection::new(),
    )])))
    // `offline · 3s` would age between the render and the next tick.
    .with_store_state("offline \u{b7} 0s", None);
    harness.settle().await;

    let frame = harness.render();
    assert!(frame.contains(" Project types "), "{frame}");
    assert!(
        shows(&frame, "project types unavailable: store unreachable:"),
        "{frame}"
    );
    assert!(shows(&frame, DATABASE_UNREACHABLE), "{frame}");
    assert!(!frame.contains(BROWSE_KEYS), "no keys are offered: {frame}");

    // The first line is the shell's header: the refused read's flash gives way on the first key,
    // so the section's rows are what must not move.
    let pane = |frame: &str| -> Vec<String> {
        let lines: Vec<String> = frame.lines().map(str::to_owned).collect();
        lines[1..].to_vec()
    };
    for key in ["j", "k"] {
        harness.key(key);
        harness.settle().await;
        assert_eq!(
            pane(&harness.render()),
            pane(&frame),
            "`{key}` changes nothing on screen"
        );
    }
    insta::assert_snapshot!("offline", frame);
}

/// D8: the product registers `Project types` after `Queue`, so ten `l` reach it; the strip
/// scrolls to keep its title on screen (MOD-146 F-1 (A)).
#[tokio::test]
async fn the_product_registers_project_types_after_queue() {
    let mut harness = Harness::demo();
    htui::app::register_all(harness.app());
    harness.settle().await;
    // Settings is the fourth tab.
    harness.key("4");
    harness.settle().await;
    for _ in 0..10 {
        harness.key("l");
    }
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains(" Queue  Project types "),
        "Project types follows Queue: {frame}"
    );
    assert!(
        frame.contains("Software \u{b7} software \u{b7} coding"),
        "{frame}"
    );
    assert!(
        frame.contains(BROWSE_KEYS),
        "the active section is Project types: {frame}"
    );
}

/// The section's read name is the request's own, so a `Failed` reply reaches it.
#[test]
fn the_read_is_named_project_types() {
    assert_eq!(StoreRequest::ProjectTypes.name(), project_types::READ_NAME);
}

/// D8: the keys go through keys.toml. A `[settings.project_types]` override moves the cursor on
/// its new chord, and the old one no longer does.
#[tokio::test]
async fn a_view_override_moves_the_cursor_on_its_new_chord_only() {
    let (bench, mut section) =
        bench_with_keys("version = 2\n[settings.project_types]\ndown = [\"J\"]\n").await;
    let before = frame(&bench, &section);
    bench.key(&mut section, "j");
    assert_eq!(frame(&bench, &section), before, "`j` is no longer down");
    bench.key(&mut section, "J");
    let moved = lines(&frame(&bench, &section));
    assert_eq!(moved[0], "Documents \u{b7} docs", "{moved:?}");
    assert_eq!(moved[1], "General \u{b7} general", "{moved:?}");
    assert_eq!(moved[2], "    Tasks carried out and reviewed.", "{moved:?}");
    assert!(
        bench.key_hint(&section).contains("J/k select"),
        "the hint names the new chord: {}",
        bench.key_hint(&section)
    );
}
