//! `Settings > Qdrant`, the section half (MOD-115 M3 L-sec, D-13): the rows, the two editors and
//! the clear question, drawn through a [`SectionBench`] and through the shell at 80 columns.
//!
//! Every snapshot the section is fed is its own reply built as a literal, so no case here reaches
//! the keyring (`tests/connection.rs`'s rule): the bench has no store, and the one shell case runs
//! `--demo`, which answers `QdrantState::NotApplicable` without opening one. The worker half of
//! the section (the keyring calls, the demo refusals) is `tests/settings.rs`'s.
//!
//! No case types a real key: the one the key editor takes is a fixture, and the frame shows its
//! dots and count only.
#![cfg(feature = "testkit")]

use htui::app::Handled;
use htui::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use htui::store_worker::StoreReply;
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::{QdrantSection, SettingsSection, SettingsTab};

/// The URL summary a stored snapshot carries.
const SUMMARY: &str = "https://qdrant.example:6334";

/// The key the key editor cases type: a fixture, never shown, so `contains` finding it means the
/// section drew it.
const TYPED_KEY: &str = "qk-fixture-0123456789";

/// The sentence a box with no unlocked collection answers.
const UNREADABLE_WHY: &str =
    "cannot read the keyring entry (htui/qdrant-url): Secret Service: no result found";

/// The URL and the key both stored.
fn stored() -> QdrantSnapshot {
    QdrantSnapshot {
        url_state: QdrantState::Stored,
        key_state: QdrantState::Stored,
        url_summary: Some(SUMMARY.to_owned()),
    }
}

/// A keyring with neither.
fn empty() -> QdrantSnapshot {
    QdrantSnapshot {
        url_state: QdrantState::NotStored,
        key_state: QdrantState::NotStored,
        url_summary: None,
    }
}

/// A keyring that answers nothing at all: not empty, unreadable.
fn unreadable() -> QdrantSnapshot {
    QdrantSnapshot {
        url_state: QdrantState::Unreadable(UNREADABLE_WHY.to_owned()),
        key_state: QdrantState::Unreadable(UNREADABLE_WHY.to_owned()),
        url_summary: None,
    }
}

/// A bench and a section with `snapshot` already delivered as the read's reply.
async fn bench_with(snapshot: QdrantSnapshot) -> (SectionBench, QdrantSection) {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(snapshot));
    let _ = bench.drained();
    (bench, section)
}

/// Types `text` one key at a time.
fn typed(bench: &SectionBench, section: &mut QdrantSection, text: &str) {
    for c in text.chars() {
        assert_eq!(bench.key(section, &c.to_string()), Handled::Consumed);
    }
}

/// The section drawn into the bench's 100x30 buffer.
fn frame_of(bench: &SectionBench, section: &QdrantSection) -> String {
    bench.render_section(section, 100)
}

/// The frame as one line, so a sentence is found wherever it wrapped.
fn flattened(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Both rows of a stored keyring: the URL by its summary, the key by its presence only.
#[tokio::test]
async fn the_rows_show_what_is_stored_and_never_the_key() {
    let (bench, section) = bench_with(stored()).await;
    let frame = frame_of(&bench, &section);
    assert!(frame.contains(SUMMARY), "{frame}");
    assert!(!section.captures_input(), "a stored URL opens nothing");
    insta::assert_snapshot!("stored", frame);
}

/// No URL stored: the URL editor opens itself, once, over the sentence that says so.
#[tokio::test]
async fn an_empty_keyring_opens_the_url_editor() {
    let (bench, section) = bench_with(empty()).await;
    assert!(section.captures_input(), "the URL editor opened itself");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains("no Qdrant URL is stored"), "{frame}");
    insta::assert_snapshot!("empty", frame_of(&bench, &section));
}

/// A keyring that could not be read is never shown as an empty one, and opens no editor.
#[tokio::test]
async fn an_unreadable_keyring_says_so_and_opens_nothing() {
    let (bench, section) = bench_with(unreadable()).await;
    assert!(
        !section.captures_input(),
        "no field over a keyring nobody can read"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains("the keyring could not be read"), "{frame}");
    assert!(!frame.contains("not stored"), "{frame}");
    insta::assert_snapshot!("unreadable", frame_of(&bench, &section));
}

/// `e` on the URL row over a stored URL: a fresh field that replaces it.
#[tokio::test]
async fn e_opens_the_url_editor_over_a_stored_url() {
    let (bench, mut section) = bench_with(stored()).await;
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    typed(&bench, &mut section, "https://q.example:6334");
    let frame = frame_of(&bench, &section);
    assert!(frame.contains("https://q.example:6334"), "{frame}");
    assert!(
        flattened(&frame).contains("replaces the stored Qdrant URL"),
        "{frame}"
    );
    insta::assert_snapshot!("editor", frame);
}

/// `e` on the key row: a masked field, its dots and count only.
#[tokio::test]
async fn e_opens_the_masked_key_editor() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    typed(&bench, &mut section, TYPED_KEY);
    let frame = frame_of(&bench, &section);
    assert!(!frame.contains(TYPED_KEY), "the key is never drawn");
    assert!(!frame.contains("qk-"), "not even a fragment of it");
    assert!(frame.contains("(21)"), "the count beside the dots: {frame}");
    insta::assert_snapshot!("editor_key", frame);
}

/// `c` asks before it clears both rows.
#[tokio::test]
async fn c_asks_before_it_clears() {
    let (bench, mut section) = bench_with(stored()).await;
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty(), "the question sends nothing");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Remove the Qdrant settings from the keyring? y / n"),
        "{frame}"
    );
    insta::assert_snapshot!("confirm", frame_of(&bench, &section));
}

/// MOD-81: the section at 80 columns, through the shell (`--demo`: no keyring is read).
#[tokio::test]
async fn the_section_at_80_columns() {
    let mut harness = Harness::demo()
        .size(80, 24)
        .with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
            QdrantSection::new(),
        )])));
    harness.settle().await;
    let frame = harness.render();
    assert!(frame.contains("n/a in a demo session"), "{frame}");
    insta::assert_snapshot!("demo_80", frame);
}

/// The section asks for its own read and is called Qdrant.
#[tokio::test]
async fn the_section_names_itself() {
    let section = QdrantSection::new();
    assert_eq!(section.id(), QdrantSection::ID);
    assert_eq!(section.title(), "Qdrant");
}
