//! Backlog › Docs and Notes, and their compose area, on the kit (MOD-115 M4 L-docs): the
//! Documents table as `kit::list` rows under a header line (the kind a base word, MOD-85 E9),
//! the empty states through `kit::empty` with a next step, the compose area's rule through
//! `kit::rule` and a document's rows through `kit::form`, and the 80-column frames (MOD-81).
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::model::WorkspaceSummary;
use htui_core::store::MemStore;
use ratatui::style::Modifier;

/// Rows down from the arrival row (htui `ANA-1`) to htui `FEAT-1`, which has documents and notes.
const TO_FEAT_1: usize = 3;

/// Rows down to htui `ANA-2`, which has neither.
const TO_ANA_2: usize = 1;

/// Sub-tabs right of Body to Docs.
const TO_DOCS: usize = 3;

/// Sub-tabs right of Body to Notes.
const TO_NOTES: usize = 4;

/// The rule glyph and the list's gutter.
const RULE: char = '\u{2500}';
const GUTTER: &str = "\u{258c}";

/// The demo workspace with this slug, read out of a throw-away store.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// A settled Backlog scoped to `Platform` on a `width` x `height` terminal (`tests/backlog.rs`'
/// `backlog_at`); the agent runtime keeps prompt previews off the status line.
async fn backlog_at(width: u16, height: u16) -> Harness {
    let mut harness = Harness::demo()
        .size(width, height)
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// [`backlog_at`] `rows` down the list and `pane` sub-tabs right of Body.
async fn on(width: u16, height: u16, rows: usize, pane: usize) -> Harness {
    let mut harness = backlog_at(width, height).await;
    for _ in 0..rows {
        harness.key("j");
        harness.drive_to_end().await;
    }
    for _ in 0..pane {
        harness.key("l");
    }
    harness.drive_to_end().await;
    harness
}

/// Presses each of `keys`, serving what each asks for.
async fn keys(harness: &mut Harness, keys: &[&str]) {
    for key in keys {
        harness.key(key);
        harness.drive_to_end().await;
    }
}

/// The detail pane's rows of `frame`: everything right of the list pane's last `│` on each row.
fn detail_rows(frame: &str) -> Vec<String> {
    frame
        .lines()
        .map(|row| {
            let chars: Vec<char> = row.chars().collect();
            // The two panes meet at `││`, `╮╭` or `╯╰`: the detail starts after the second.
            let at = chars
                .windows(2)
                .position(|pair| {
                    matches!(
                        pair,
                        ['\u{2502}', '\u{2502}']
                            | ['\u{256e}', '\u{256d}']
                            | ['\u{256f}', '\u{2570}']
                    )
                })
                .map_or(0, |at| at + 1);
            chars[at..].iter().collect()
        })
        .collect()
}

/// MOD-81, MOD-115 M4 L-docs: Docs and Notes at 80 columns. The table's header and rows keep the
/// kind, version and provenance columns, the kind's as wide as the widest kind listed so the
/// titles keep their room, the cursor behind the gutter; the thread keeps each note's stamp and
/// its lines.
#[tokio::test]
async fn the_documents_and_the_notes_at_80_columns() {
    let mut harness = on(80, 24, TO_FEAT_1, TO_DOCS).await;
    let frame = harness.render();
    let detail = detail_rows(&frame).join("\n");
    assert!(detail.contains(" kind ver by   title"), "{frame}");
    assert!(
        detail.contains(&format!("{GUTTER}plan v1  step Plan")),
        "the cursor's row behind the gutter:\n{frame}"
    );
    assert!(detail.contains(" prd  v1  hand PRD"), "{frame}");
    insta::assert_snapshot!("documents_80", frame);

    let mut harness = on(80, 24, TO_FEAT_1, TO_NOTES).await;
    let frame = harness.render();
    let detail = detail_rows(&frame).join("\n");
    assert!(detail.contains("09-02 09:00"), "a note's stamp:\n{frame}");
    assert!(detail.contains("Skeleton only"), "its body:\n{frame}");
    insta::assert_snapshot!("notes_80", frame);
}

/// MOD-115 M4 L-docs: an item with no documents or no notes says so and names the key that
/// writes one (`kit::empty`), at the default 100 columns.
#[tokio::test]
async fn the_empty_docs_and_notes_name_the_key_that_writes_one() {
    for (pane, what) in [
        (TO_DOCS, "No documents for this item."),
        (TO_NOTES, "No notes for this item."),
    ] {
        let mut harness = on(100, 30, TO_ANA_2, pane).await;
        let frame = harness.render();
        assert!(frame.contains(what), "{frame}");
        assert!(frame.contains("a writes one"), "{frame}");
    }
}

/// MOD-115 M4 L-docs: `v` opens the compose area on the kit: its rule is the title then `─` to
/// the pane's edge, bold while the detail holds the keyboard; the fixed kind and the title are
/// form rows and the body's heading has the focus, behind the gutter, never a `> ` marker.
#[tokio::test]
async fn the_document_form_is_a_kit_rule_over_a_kit_form() {
    let mut harness = on(100, 30, TO_FEAT_1, TO_DOCS).await;
    keys(&mut harness, &["v"]).await;
    let frame = harness.render();
    let rows = detail_rows(&frame);
    let rule = rows
        .iter()
        .position(|row| row.contains(" New version of plan (from v2) "))
        .unwrap_or_else(|| panic!("the rule:\n{frame}"));
    assert!(
        rows[rule].contains(&format!("(from v2) {RULE}{RULE}")),
        "{frame}"
    );
    let below: Vec<&str> = rows[rule + 1..rule + 4]
        .iter()
        .map(|row| row.trim_start_matches('\u{2502}'))
        .collect();
    assert!(below[0].starts_with(" kind    plan"), "{frame}");
    assert!(
        below[1].starts_with(" title   Plan: TUI scaffold"),
        "{frame}"
    );
    assert!(below[2].starts_with(&format!("{GUTTER}body")), "{frame}");
    assert!(!frame.contains("> body"), "{frame}");

    let buffer = harness.render_buffer();
    let y = u16::try_from(rule).expect("on screen");
    let x = (0..buffer.area.width)
        .find(|x| buffer[(*x, y)].symbol() == "N")
        .expect("the rule's title");
    assert!(
        buffer[(x, y)].modifier.contains(Modifier::BOLD),
        "the rule is in the title role while the area has the keys: {:?}",
        buffer[(x, y)]
    );
}
