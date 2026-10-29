//! MOD-115 M4 L-forms: the Backlog's item form and divergence view on the kit, through the
//! harness. The form's rows are `kit::form` rows (the gutter and band mark the focused field, the
//! pickers read `‹ word ›`), the divergence view's `theirs`/`mine` words are not focus colours,
//! and both read at 80 columns (MOD-81).
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::testkit::Harness;
use htui::ui::tabs::backlog::BacklogTab;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{ItemPatch, WorkspaceSummary};
use htui_core::store::{MemStore, UpdateOutcome, WriteStore as _};

/// The focus gutter `kit::list` and `kit::form` draw.
const GUTTER: &str = "\u{258c}";

/// The workspace of the demo fixture with this slug.
async fn workspace(slug: &str) -> WorkspaceSummary {
    MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .unwrap_or_else(|| panic!("the demo fixture holds the `{slug}` workspace"))
}

/// A settled Backlog tab over `store`, scoped to `Platform`, on a `width` x `height` terminal.
async fn backlog(
    store: MemStore,
    width: u16,
    height: u16,
    keys: Option<htui::keys::Keys>,
) -> Harness {
    let mut harness = Harness::over(store).size(width, height);
    if let Some(keys) = keys {
        harness = harness.with_keys(keys);
    }
    let mut harness = harness
        .with_tab(Box::new(BacklogTab::new()))
        .with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    harness.drive_to_end().await;
    harness.app().update(Action::SetScope {
        workspace: workspace("platform").await,
    });
    harness.drive_to_end().await;
    harness
}

/// Presses each of `keys`, then serves what they asked for.
async fn keys(harness: &mut Harness, keys: &[&str]) {
    for key in keys {
        harness.key(key);
    }
    harness.drive_to_end().await;
}

/// Types `text`, a space as `space`.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        if c == ' ' {
            harness.key("space");
        } else {
            harness.key(&c.to_string());
        }
    }
}

/// The frame line holding the item form's `label` row: the gutter or a blank, then the label.
fn form_row<'a>(frame: &'a str, label: &str) -> Option<&'a str> {
    frame.lines().find(|line| {
        line.contains(&format!("\u{2502}{GUTTER}{label} "))
            || line.contains(&format!("\u{2502} {label} "))
    })
}

/// Whether the item form's `label` row carries the focus gutter.
fn focused(frame: &str, label: &str) -> bool {
    form_row(frame, label).is_some_and(|line| line.contains(&format!("\u{2502}{GUTTER}{label} ")))
}

/// `patch` written on `ANA-1` at version 1, as another process would: the head moves to 2.
async fn their_write(store: &MemStore, patch: ItemPatch) {
    let outcome = store
        .update_item(ids::HTUI_ANA_1, 1, patch)
        .await
        .expect("the memory store never fails");
    assert!(
        matches!(outcome, UpdateOutcome::Updated(_)),
        "the other write is at the head: {outcome:?}"
    );
}

/// Their retitle and body, as another process writes them.
fn theirs() -> ItemPatch {
    ItemPatch {
        title: Some("Theirs".to_owned()),
        body: Some("Their body.".to_owned()),
        author_id: ids::USER,
        box_id: Some(ids::BOX),
        reason: "edited".to_owned(),
        ..ItemPatch::default()
    }
}

/// `e`, their write, ` mine` after the title, `Mine first. ` before the body and `Ctrl+S`: the
/// stale save opens the divergence view, title and body in conflict.
async fn diverged(width: u16, height: u16) -> Harness {
    let store = MemStore::demo();
    let mut harness = backlog(store.clone(), width, height, None).await;
    keys(&mut harness, &["e"]).await;
    their_write(&store, theirs()).await;
    type_text(&mut harness, " mine");
    keys(&mut harness, &["tab"; 5]).await;
    type_text(&mut harness, "Mine first. ");
    keys(&mut harness, &["ctrl-s"]).await;
    harness
}

/// MOD-115 M4 L-forms: the gutter is on the focused field and moves with the focus; nothing is
/// marked with `> `.
#[tokio::test]
async fn the_gutter_marks_the_focused_field_and_moves_with_it() {
    let mut harness = backlog(MemStore::demo(), 100, 30, None).await;
    keys(&mut harness, &["N"]).await;
    let frame = harness.render();
    assert!(
        focused(&frame, "title"),
        "the form opens on Title:\n{frame}"
    );
    for label in ["project", "kind", "priority", "tags", "graph"] {
        assert!(!focused(&frame, label), "{label}:\n{frame}");
    }
    assert!(!frame.contains("\u{2502}> "), "no `> ` marker:\n{frame}");

    keys(&mut harness, &["tab"]).await;
    let frame = harness.render();
    assert!(focused(&frame, "priority"), "{frame}");
    assert!(!focused(&frame, "title"), "{frame}");
}

/// MOD-115 M4 L-forms (MOD-85): the focused row's gutter is drawn in the Backlog's accent and
/// its label in the selection band, as a list row's is.
#[tokio::test]
async fn the_focused_row_is_the_accent_gutter_and_the_band() {
    let mut harness = backlog(MemStore::demo(), 100, 30, None).await;
    keys(&mut harness, &["N"]).await;
    let buffer = harness.render_buffer();
    let theme = htui::ui::Theme::default();
    let (x, y) = (0..buffer.area.height)
        .find_map(|y| {
            (0..buffer.area.width.saturating_sub(5)).find_map(|x| {
                let text: String = (x..x + 6).map(|x| buffer[(x, y)].symbol()).collect();
                (text == format!("{GUTTER}title")).then_some((x, y))
            })
        })
        .expect("the focused title row");
    assert_eq!(
        buffer[(x, y)].fg,
        theme
            .tab_accent(BacklogTab::ID.0)
            .fg
            .expect("the Backlog's accent has a colour"),
        "the gutter is the tab's accent"
    );
    assert_eq!(
        buffer[(x + 1, y)].bg,
        theme.selected.bg.expect("the band has a colour"),
        "the label is banded"
    );
}

/// MOD-115 M4 L-forms (MD-A): the form pane's keys are the ones in force.
#[tokio::test]
async fn a_rebound_field_key_is_the_one_the_form_pane_names() {
    let file = htui::keys::load_str("version = 1\n[form]\nnext_field = \"f3\"\nsave = \"f2\"\n")
        .expect("the key file loads");
    let mut harness = backlog(MemStore::demo(), 100, 30, Some(file)).await;
    keys(&mut harness, &["N"]).await;
    let frame = harness.render();
    assert!(frame.contains("F3 field \u{b7} F2 save"), "{frame}");
    keys(&mut harness, &["f3"]).await;
    assert!(focused(&harness.render(), "priority"), "F3 moves on");
}

/// MOD-81: the new item form at 80 columns: every row, the pickers' words and the pane's keys.
#[tokio::test]
async fn the_item_form_at_80_columns() {
    let mut harness = backlog(MemStore::demo(), 80, 24, None).await;
    keys(&mut harness, &["N"]).await;
    type_text(&mut harness, "Fresh item");
    let frame = harness.render();
    for wanted in [
        " New item \u{b7} htui ",
        "\u{2039} htui \u{203a}",
        "\u{2039} ANA analysis \u{203a}",
        "\u{2039} kind default \u{203a}",
        "paths (one per line)",
        "Tab field \u{b7} Ctrl+s save",
    ] {
        assert!(frame.contains(wanted), "{wanted:?} in\n{frame}");
    }
    assert!(
        form_row(&frame, "title").is_some_and(|line| line.contains("Fresh item")),
        "{frame}"
    );
    insta::assert_snapshot!("item_form_80", frame);
}

/// MOD-81: the divergence view at 80 columns: the header, every moved field with its state, both
/// diffs and the keys.
#[tokio::test]
async fn the_divergence_view_at_80_columns() {
    let mut harness = diverged(80, 24).await;
    let frame = harness.render();
    for wanted in [
        "ANA-1  v1 \u{2192} theirs v2",
        "--- ancestor v1",
        "+++ theirs v2",
        "+++ mine",
        "t theirs wins \u{b7} m mine wins \u{b7} Esc back",
    ] {
        assert!(frame.contains(wanted), "{wanted:?} in\n{frame}");
    }
    for field in ["title", "body"] {
        assert!(
            frame
                .lines()
                .any(|line| line.contains(field) && line.contains("conflict")),
            "{field} conflicts:\n{frame}"
        );
    }
    insta::assert_snapshot!("divergence_80", frame);
}
