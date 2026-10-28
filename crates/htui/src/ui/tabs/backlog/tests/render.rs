//! What the Backlog draws: the item form's claims and the detail strip.

use super::*;

/// MOD-57 P2 (PD-3): the open item form claims its body for an in-pane editor, the rect
/// `item_form::render` returns in the detail pane's place; the detail pane claims nothing.
#[tokio::test]
async fn the_item_form_claims_its_body_for_the_editor() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let (width, height) = DEFAULT_SIZE;
    let area = Rect::new(0, 0, width, height);
    let claimed = |tab: &BacklogTab| {
        let cell = Cell::new(None);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("a test terminal");
        terminal
            .draw(|frame| tab.render(frame, area, &bench.ctx().with_editor_area(&cell)))
            .expect("the frame draws");
        (terminal.backend().buffer().clone(), cell.get())
    };
    assert_eq!(claimed(&tab).1, None, "the detail pane claims nothing");

    let backend = htui_store::Backend::memory(MemStore::demo());
    let request = StoreRequest::ItemForm {
        project: htui_core::fixtures::ids::PROJECT_HTUI,
        item: None,
    };
    let context = match item_writes::serve(&backend, &request).await {
        Ok(StoreReply::ItemForm(context)) => *context,
        other => panic!("the form read answered {other:?}"),
    };
    let mut form = ItemForm::open_new(context, &bench.projects, None);
    // Title, Priority, Tags, Graph, Paths, Body.
    for _ in 0..5 {
        let _ = form.on_key(KeyEvent::from(KeyCode::Tab), Keys::compiled());
    }
    form.on_paste("First line.");
    tab.item_form = Some(form);
    let (buffer, claim) = claimed(&tab);
    let claim = claim.expect("the form claims its body");

    let [_, right] = panes(area);
    let mut alone = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("a test terminal");
    let mut body = None;
    alone
        .draw(|frame| {
            body = Some(item_form::render(
                frame,
                right,
                tab.item_form.as_ref().expect("open"),
                &Theme::default(),
            ));
        })
        .expect("the frame draws");
    assert_eq!(Some(claim), body, "the claim is the form's body");
    let row: String = (claim.x..claim.right())
        .map(|x| buffer[(x, claim.y)].symbol())
        .collect();
    assert!(row.starts_with("First line."), "{row:?} at {claim:?}");
}

/// MOD-57 R0 F1: with Paths handed to `$EDITOR` the form claims the paths rect, not its body:
/// three rows, under `MIN_PANE`, so the pane goes over the whole tab body (D1) rather than
/// over a body it is not editing.
#[tokio::test]
async fn the_item_form_claims_its_paths_when_paths_is_handed_off() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let (width, height) = DEFAULT_SIZE;
    let area = Rect::new(0, 0, width, height);
    let backend = htui_store::Backend::memory(MemStore::demo());
    let request = StoreRequest::ItemForm {
        project: htui_core::fixtures::ids::PROJECT_HTUI,
        item: None,
    };
    let context = match item_writes::serve(&backend, &request).await {
        Ok(StoreReply::ItemForm(context)) => *context,
        other => panic!("the form read answered {other:?}"),
    };
    let mut form = ItemForm::open_new(context, &bench.projects, None);
    // Title, Priority, Tags, Graph, Paths.
    for _ in 0..4 {
        let _ = form.on_key(KeyEvent::from(KeyCode::Tab), Keys::compiled());
    }
    form.on_paste("src/**");
    let handed = form.on_key(
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        Keys::compiled(),
    );
    assert!(
        matches!(&handed, ItemFormOutcome::External(edit) if edit.text == "src/**"),
        "{handed:?}"
    );
    tab.item_form = Some(form);

    let cell = Cell::new(None);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("a test terminal");
    terminal
        .draw(|frame| tab.render(frame, area, &bench.ctx().with_editor_area(&cell)))
        .expect("the frame draws");
    let claim = cell.get().expect("the form claims the paths");
    assert_eq!(claim.height, 3, "{claim:?}");
    assert!(claim.height < crate::app::MIN_PANE.height);
    let buffer = terminal.backend().buffer();
    let row: String = (claim.x..claim.right())
        .map(|x| buffer[(x, claim.y)].symbol())
        .collect();
    assert!(row.starts_with("src/**"), "{row:?} at {claim:?}");
}

/// The sub-tab strip fits inside the detail pane at the harness's pinned size, so a longer title, another sub-tab or a narrower pane fails here
/// instead of being re-accepted as a clipped snapshot (MOD-30).
#[test]
fn the_detail_strip_fits_the_detail_pane() {
    let (width, height) = DEFAULT_SIZE;
    let body = chrome(Rect::new(0, 0, width, height)).body;
    let [_, right] = panes(body);
    let inner = detail::frame_block(None).inner(right);
    let tab = BacklogTab::new();
    let strip = detail::strip_line(&tab.detail, &Theme::default());
    assert!(
        strip.width() <= usize::from(inner.width),
        "the strip is {} columns against a {}-column pane",
        strip.width(),
        inner.width
    );
}

/// MOD-80 D5: under the monochrome theme only modifiers can mark the active sub-tab; the
/// separators carry no style at all.
#[test]
fn the_active_sub_tab_is_bold_and_underlined_without_colour() {
    let tab = BacklogTab::new();
    let strip = detail::strip_line(&tab.detail, &Theme::monochrome());
    let marked = ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED;
    let titles: Vec<_> = strip
        .spans
        .iter()
        .filter(|span| !span.content.trim().is_empty())
        .collect();
    assert!(titles.len() > 1, "{strip:?}");
    assert!(
        titles[0].style.add_modifier.contains(marked),
        "the first registered sub-tab is active: {:?}",
        titles[0]
    );
    for span in &titles[1..] {
        assert!(!span.style.add_modifier.intersects(marked), "{span:?}");
    }
    for span in strip
        .spans
        .iter()
        .filter(|span| span.content.trim().is_empty())
    {
        assert_eq!(span.style, ratatui::style::Style::default(), "{span:?}");
    }
}
