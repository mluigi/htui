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
use htui::ui::Theme;
use htui::ui::tabs::settings::{QdrantSection, SettingsSection, SettingsTab};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{Terminal, TerminalOptions, Viewport, backend::TestBackend};

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

// ---- MOD-115 M3 (lane L-sec): the section on the kit ----------------------------------------------

/// `field.submit` and `field.cancel` moved (MOD-115 M3 C6): every field's `Enter` and `Esc`.
const FIELD_REBOUND: &str = "version = 2\n[field]\nsubmit = [\"ctrl-t\"]\ncancel = [\"ctrl-o\"]\n";

/// A bench over `keys`, `snapshot` already delivered.
async fn bench_with_keys(file: &str, snapshot: QdrantSnapshot) -> (SectionBench, QdrantSection) {
    let keys = htui::keys::load_str(file).expect("the key file loads");
    let bench = SectionBench::new().await.with_keys(keys);
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(snapshot));
    let _ = bench.drained();
    (bench, section)
}

/// One section drawn into a 100x30 buffer: a marker's colour is a style, not text.
fn drawn(bench: &SectionBench, section: &QdrantSection) -> Buffer {
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::with_options(
        TestBackend::new(100, 30),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("a test terminal");
    let ctx = bench.ctx();
    terminal
        .draw(|frame| section.render(frame, frame.area(), &ctx))
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// Row `y` of `buffer` as text, its trailing blanks dropped.
fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Both editors' hints read as before under the compiled keys and name the rebound chords once
/// `[field]` moves them (MD-A, C6).
#[tokio::test]
async fn every_field_hint_names_the_field_keys_in_force() {
    let (bench, mut section) = bench_with_keys(FIELD_REBOUND, stored()).await;
    let compiled = SectionBench::new().await;

    bench.key(&mut section, "e");
    assert_eq!(compiled.key_hint(&section), "Enter store \u{b7} Esc cancel");
    assert_eq!(
        bench.key_hint(&section),
        "Ctrl+t store \u{b7} Ctrl+o cancel"
    );
    bench.key(&mut section, "esc");

    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    assert_eq!(
        compiled.key_hint(&section),
        "Enter store \u{b7} Esc cancel \u{b7} typed text is never shown"
    );
    assert_eq!(
        bench.key_hint(&section),
        "Ctrl+t store \u{b7} Ctrl+o cancel \u{b7} typed text is never shown"
    );
}

/// The guides under the URL field name the submit key in force (C5, D-15): an empty keyring, a
/// stored URL and an unreadable keyring.
#[tokio::test]
async fn the_guides_name_the_submit_key_in_force() {
    let (bench, section) = bench_with_keys(FIELD_REBOUND, empty()).await;
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("no Qdrant URL is stored; type one and press Ctrl+t"),
        "{frame}"
    );

    let (bench, mut section) = bench_with_keys(FIELD_REBOUND, stored()).await;
    bench.key(&mut section, "e");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Ctrl+t replaces the stored Qdrant URL"),
        "{frame}"
    );
    assert!(!frame.contains("Enter"), "{frame}");

    let (bench, mut section) = bench_with_keys(FIELD_REBOUND, unreadable()).await;
    bench.key(&mut section, "e");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("the keyring could not be read; Ctrl+t tries to store a Qdrant URL in it"),
        "{frame}"
    );
}

/// `Enter API key` named a verb with a key's name (C5): the key field's guide is a sentence.
#[tokio::test]
async fn the_key_guide_is_a_sentence() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "j");
    bench.key(&mut section, "e");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("type the API key, or leave it blank if there is none"),
        "{frame}"
    );
    assert!(!frame.contains("Enter API key"), "{frame}");
}

/// The field is its own row right under the row it replaces (D3, audit item 3), behind the
/// form's marker, with what is stored still on screen; no label is drawn in the accent: only the
/// focused row's gutter carries the tab's (MOD-85).
#[tokio::test]
async fn the_field_is_under_the_row_it_replaces() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "e");
    typed(&bench, &mut section, "https://q.example");
    let buffer = drawn(&bench, &section);
    let url_row = (0..buffer.area.height)
        .find(|y| row(&buffer, *y).contains("https://q.example"))
        .expect("the field is drawn");
    let text = row(&buffer, url_row);
    assert!(text.starts_with("\u{258c}new URL"), "{text}");
    let above = row(&buffer, url_row - 1);
    assert!(
        above.starts_with(" URL") && above.contains(SUMMARY),
        "the row it replaces is right above: {above}"
    );
    assert!(
        !(0..buffer.area.height).any(|y| row(&buffer, y).contains("URL:")),
        "the old field under the rows is gone"
    );
    let theme = Theme::default();
    let accent = theme.tab_accent("settings");
    assert_eq!(buffer[(0, url_row)].fg, accent.fg.unwrap_or(Color::Reset));
    for y in 0..buffer.area.height {
        for x in 1..buffer.area.width {
            assert_ne!(
                buffer[(x, y)].style().fg,
                theme.accent.fg,
                "no label in the accent: {:?} at ({x}, {y})",
                row(&buffer, y)
            );
        }
    }
}

/// The key row is a secret value: dots and a word, never a placeholder that looks like data.
#[tokio::test]
async fn a_stored_key_reads_as_a_secret() {
    let (bench, section) = bench_with(stored()).await;
    let frame = frame_of(&bench, &section);
    assert!(
        frame.contains("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022} stored"),
        "{frame}"
    );
    assert!(!frame.contains("<redacted>"), "{frame}");
}

/// MOD-83: an unreadable keyring names why, so the user knows where to look.
#[tokio::test]
async fn an_unreadable_keyring_names_the_reason() {
    let (bench, section) = bench_with(unreadable()).await;
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(&format!("the keyring could not be read: {UNREADABLE_WHY}")),
        "{frame}"
    );
}

/// MOD-83: a refused read says so, and what brings the rows back.
#[tokio::test]
async fn a_refused_read_says_unavailable_and_what_to_do() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "qdrant_info",
            message: "the keyring is locked".to_owned(),
        },
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Qdrant settings are unavailable: the keyring is locked"),
        "{frame}"
    );
    assert!(
        frame.contains("r asks again once the store answers"),
        "{frame}"
    );
}

/// MOD-83: `c` with nothing stored says so and names the key that stores one, rather than calling
/// the settings unavailable.
#[tokio::test]
async fn c_with_nothing_stored_names_the_way_forward() {
    let (bench, mut section) = bench_with(empty()).await;
    bench.key(&mut section, "esc");
    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty());
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(
            "no Qdrant URL or API key is stored, so there is nothing to clear; e stores one"
        ),
        "{frame}"
    );
    assert!(!frame.contains("unavailable"), "{frame}");
}

/// A write in flight reads in words, never as a store request's name.
#[tokio::test]
async fn a_write_in_flight_reads_in_words() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = bench.drained();
    let hint = bench.key_hint(&section);
    assert!(
        hint.ends_with("\u{b7} clearing the Qdrant settings\u{2026}"),
        "{hint}"
    );
    bench.key(&mut section, "e");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("still clearing the Qdrant settings \u{2014} wait for the store's answer"),
        "{frame}"
    );
    assert!(!frame.contains('_'), "no request name: {frame}");
}

/// The clear question is the focused row's error line, under the row it is about.
#[tokio::test]
async fn the_clear_question_sits_under_its_row() {
    let (bench, mut section) = bench_with(stored()).await;
    bench.key(&mut section, "c");
    let buffer = drawn(&bench, &section);
    let url = (0..buffer.area.height)
        .find(|y| row(&buffer, *y).starts_with("\u{258c}URL"))
        .expect("the URL row is focused");
    assert!(
        row(&buffer, url + 1).contains("Remove the Qdrant settings from the keyring? y / n"),
        "{:?}",
        row(&buffer, url + 1)
    );
}
