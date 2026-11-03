//! What the Backlog draws: the item form's claims, the list and the detail as kit panes, the
//! detail's chips and which pane holds the keyboard (MOD-115 M4 B1).

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

use super::*;
use crate::ui::kit::list::GUTTER;
use crate::ui::kit::pane::Pane;

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
                Pane::inner(right),
                tab.item_form.as_ref().expect("open"),
                bench.ctx().accent(),
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

// ---- MOD-115 M4 B1: the list and the detail as kit panes --------------------------------------

/// `tab` drawn through `ctx` on a `width` x `height` terminal, as its buffer.
pub(super) fn buffer_at(tab: &BacklogTab, ctx: &Ctx<'_>, width: u16, height: u16) -> Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("a test terminal");
    terminal
        .draw(|frame| tab.render(frame, frame.area(), ctx))
        .expect("the frame draws");
    terminal.backend().buffer().clone()
}

/// [`buffer_at`] at the harness's size.
pub(super) fn buffer_of(tab: &BacklogTab, ctx: &Ctx<'_>) -> Buffer {
    let (width, height) = DEFAULT_SIZE;
    buffer_at(tab, ctx, width, height)
}

/// `width` cells of row `y` from `x`, as text.
pub(super) fn text(buffer: &Buffer, x: u16, y: u16, width: u16) -> String {
    (x..x + width).map(|x| buffer[(x, y)].symbol()).collect()
}

/// The x, in cells, where `needle` starts on `row` (every cell drawn here is one char).
fn at(row: &str, needle: &str) -> Option<u16> {
    let cells: Vec<char> = row.chars().collect();
    (0..cells.len())
        .find(|&x| cells[x..].iter().collect::<String>().starts_with(needle))
        .map(|x| u16::try_from(x).expect("a narrow row"))
}

/// The list and the detail panes at the harness's size.
pub(super) fn split() -> [Rect; 2] {
    let (width, height) = DEFAULT_SIZE;
    panes(Rect::new(0, 0, width, height))
}

/// The list pane's rows, as text.
fn list_rows(buffer: &Buffer) -> Vec<String> {
    let [list, _] = split();
    (0..buffer.area.height)
        .map(|y| text(buffer, list.x, y, list.width))
        .collect()
}

/// The settled tab with htui `FEAT-1` (`TUI scaffold`) under the cursor.
fn on_feat_1(bench: &Bench) -> BacklogTab {
    BacklogTab {
        selected: Some(Selection::Item(htui_core::fixtures::ids::HTUI_FEAT_1)),
        ..bench.tab()
    }
}

/// The foreground of the top-left corner of the pane drawn at `(x, y)`.
fn corner(buffer: &Buffer, x: u16, y: u16) -> Option<Color> {
    let cell = &buffer[(x, y)];
    assert_eq!(cell.symbol(), "\u{256d}", "a rounded corner at ({x}, {y})");
    Some(cell.fg)
}

/// MOD-115 M4 E4: the list is a kit pane, rounded and titled with its count alone.
#[tokio::test]
async fn the_list_is_a_kit_pane_titled_with_its_count() {
    let bench = Bench::new().await;
    let rows = list_rows(&buffer_of(&bench.tab(), &bench.ctx()));
    assert!(rows[0].starts_with("╭─ Backlog (11) ─"), "{}", rows[0]);
    assert!(rows[0].ends_with('╮'), "{}", rows[0]);
    assert!(rows[rows.len() - 1].starts_with('╰'), "{rows:#?}");
}

/// MOD-115 M4 E4: a project is `kit::list`'s group row, behind the gutter.
#[tokio::test]
async fn a_project_is_the_kits_group_row() {
    let bench = Bench::new().await;
    let rows = list_rows(&buffer_of(&bench.tab(), &bench.ctx()));
    assert!(rows[1].starts_with("│ ▾ htui (8) "), "{}", rows[1]);
    assert!(
        rows.iter().any(|row| row.starts_with("│ ▾ agy (3) ")),
        "{rows:#?}"
    );
}

/// MOD-115 M4 E4, MOD-81: an item row is ` {key}  {status mark}  {title}` behind the gutter: the
/// status is a glyph and a word, never `in_progress` or `awaiting_approval`.
#[tokio::test]
async fn an_item_row_is_its_key_its_status_mark_and_its_title() {
    let bench = Bench::new().await;
    let rows = list_rows(&buffer_of(&bench.tab(), &bench.ctx()));
    let row = |key: &str| {
        rows.iter()
            .find(|row| row.contains(&format!(" {key} ")))
            .unwrap_or_else(|| panic!("a {key} row in {rows:#?}"))
            .clone()
    };
    assert!(
        row("FEAT-1").starts_with("│  FEAT-1   ◐ running    TUI scaffold"),
        "{}",
        row("FEAT-1")
    );
    assert!(
        row("TOOL-1").starts_with("│  TOOL-1   ⚠ needs you  CI matrix"),
        "{}",
        row("TOOL-1")
    );
    assert!(
        row("CLEAN-1").starts_with("│  CLEAN-1  ✗ failed     Drop the legacy"),
        "{}",
        row("CLEAN-1")
    );
    for raw in ["in_progress", "awaiting_approval"] {
        assert!(
            !rows.iter().any(|row| row.contains(raw)),
            "{raw}: {rows:#?}"
        );
    }
}

/// MOD-81: the status column is as wide as the widest mark (`⚠ needs you`, 11 cells), not the
/// widest raw name (`awaiting_approval`, 17), so every title starts six cells further left.
#[tokio::test]
async fn the_status_column_is_as_wide_as_the_widest_mark() {
    let bench = Bench::new().await;
    let rows = list_rows(&buffer_of(&bench.tab(), &bench.ctx()));
    // The border, the gutter, a blank, `CLEAN-1`, two, `⚠ needs you`, two.
    let title_at = 1 + 1 + 1 + 7 + 2 + 11 + 2;
    for title in ["TUI scaffold", "CI matrix", "Data model", "Session leak"] {
        let row = rows
            .iter()
            .find(|row| row.contains(title))
            .unwrap_or_else(|| panic!("{title} in {rows:#?}"));
        assert_eq!(at(row, title), Some(title_at), "{row}");
    }
}

/// MOD-115 M4 E4, MOD-85: the cursor row is the kit's: the gutter in the tab's accent, then
/// the band across the row; the next row has neither.
#[tokio::test]
async fn the_cursor_row_is_the_gutter_then_the_band() {
    let bench = Bench::new().await;
    let ctx = bench.ctx();
    let buffer = buffer_of(&bench.tab(), &ctx);
    let [list, _] = split();
    // The title, the htui header, then htui ANA-1: the arrival row.
    let gutter = &buffer[(1, 2)];
    assert_eq!(gutter.symbol(), GUTTER, "{gutter:?}");
    assert_eq!(Some(gutter.fg), ctx.accent().fg, "{gutter:?}");
    for x in 2..list.right() - 1 {
        assert_eq!(
            buffer[(x, 2)].bg,
            Color::Cyan,
            "x = {x}: {:?}",
            buffer[(x, 2)]
        );
    }
    assert_eq!(buffer[(1, 3)].symbol(), " ", "the next row has no gutter");
    assert_eq!(buffer[(2, 3)].bg, Color::Reset, "and no band");
}

/// MOD-115 M4 E4: under the monochrome theme the band is reversed and the gutter stays.
#[tokio::test]
async fn the_cursor_row_reads_without_colour() {
    let bench = Bench {
        theme: Theme::monochrome(),
        ..Bench::new().await
    };
    let buffer = buffer_of(&bench.tab(), &bench.ctx());
    let [list, _] = split();
    assert_eq!(buffer[(1, 2)].symbol(), GUTTER);
    for x in 2..list.right() - 1 {
        assert!(
            buffer[(x, 2)].modifier.contains(Modifier::REVERSED),
            "x = {x}: {:?}",
            buffer[(x, 2)]
        );
        assert!(
            !buffer[(x, 3)].modifier.contains(Modifier::REVERSED),
            "x = {x}: {:?}",
            buffer[(x, 3)]
        );
    }
}

/// MOD-115 M4 E4, D-13: an applied filter is the list's summary band, its first row: the
/// filter in words and the count, right-aligned; the title keeps the count alone.
#[tokio::test]
async fn an_applied_filter_is_the_summary_band_over_the_rows() {
    let bench = Bench::new().await;
    let tab = BacklogTab {
        items: with_status(&bench, Status::Done),
        filter: done_only(),
        shown: done_only(),
        ..bench.tab()
    };
    let rows = list_rows(&buffer_of(&tab, &bench.ctx()));
    assert!(rows[0].starts_with("╭─ Backlog (2) ─"), "{}", rows[0]);
    assert!(!rows[0].contains("status"), "{}", rows[0]);
    assert!(rows[1].starts_with("│ / status: done "), "{}", rows[1]);
    assert!(rows[1].ends_with(" 2 items│"), "{}", rows[1]);
    assert!(rows[2].starts_with("│ ▾ htui (1)"), "{}", rows[2]);
}

/// MOD-115 M4 E4, MOD-82: the detail pane is titled `KEY · title`, the title kept first and the
/// chips windowed after it at 100 columns (C5, Q1); the strip row inside the pane is gone.
#[tokio::test]
async fn the_detail_is_titled_with_the_key_and_the_title() {
    let bench = Bench::new().await;
    let buffer = buffer_of(&on_feat_1(&bench), &bench.ctx());
    let [_, detail] = split();
    assert_eq!(detail.width, 45, "the default split at 100 columns");
    assert_eq!(
        text(&buffer, detail.x, 0, detail.width),
        "╭─ FEAT-1 · TUI scaffold ────── Body  Runs ›╮"
    );
    let first = text(&buffer, detail.x, 1, detail.width);
    assert!(!first.contains("Runs"), "no strip row: {first}");

    let header = BacklogTab {
        selected: Some(Selection::Project(first_project(&bench))),
        ..bench.tab()
    };
    let buffer = buffer_of(&header, &bench.ctx());
    let top = text(&buffer, detail.x, 0, detail.width);
    assert!(top.starts_with("╭─ Detail ─"), "{top}");
}

/// MOD-115 M4 C5 (Q1): the chips window around the active one with `‹`/`›` at 100 columns and
/// the title keeps its room; at 72 columns every chip fits beside it.
#[tokio::test]
async fn the_detail_chips_window_around_the_active_one_at_100_columns() {
    let bench = Bench::new().await;
    let mut tab = on_feat_1(&bench);
    assert!(tab.detail.select_id(PromptTab::ID));
    let buffer = buffer_of(&tab, &bench.ctx());
    let [_, detail] = split();
    assert_eq!(
        text(&buffer, detail.x, 0, detail.width),
        "╭─ FEAT-1 · TUI scaffold ────‹ Prompt  Reqs ╮"
    );

    let wide = panes(Rect::new(0, 0, 160, 30))[1];
    assert_eq!(wide.width, 72);
    let buffer = buffer_at(&on_feat_1(&bench), &bench.ctx(), 160, 30);
    assert_eq!(
        text(&buffer, wide.x, 0, wide.width),
        "╭─ FEAT-1 · TUI scaffold  Body  Runs  Graph  Docs  Notes  Prompt  Reqs ╮"
    );
}

/// MOD-80 D5, MOD-115 M4: under the monochrome theme only modifiers mark the active chip.
#[tokio::test]
async fn the_active_chip_is_bold_and_underlined_without_colour() {
    let bench = Bench {
        theme: Theme::monochrome(),
        ..Bench::new().await
    };
    let buffer = buffer_of(&on_feat_1(&bench), &bench.ctx());
    let [_, detail] = split();
    let top = text(&buffer, 0, 0, buffer.area.width);
    let marked = Modifier::BOLD | Modifier::UNDERLINED;
    let body = at(&top, " Body ").expect("the Body chip") + 1;
    let runs = at(&top, " Runs ").expect("the Runs chip") + 1;
    assert!(body > detail.x, "{top}");
    for x in body..body + 4 {
        assert!(buffer[(x, 0)].modifier.contains(marked), "x = {x}");
    }
    for x in runs..runs + 4 {
        assert!(!buffer[(x, 0)].modifier.intersects(marked), "x = {x}");
    }
}

/// MOD-115 M4 C4: browsing, the focused pane is the shell's, in the tab's accent; the other is
/// faint.
#[tokio::test]
async fn browsing_the_focused_pane_is_the_shells() {
    let bench = Bench::new().await;
    let [_, detail] = split();
    let accent = bench.ctx().accent().fg;
    let faint = bench.theme.border_faint.fg;
    let buffer = buffer_of(&bench.tab(), &bench.ctx());
    assert_eq!(corner(&buffer, 0, 0), accent, "the list");
    assert_eq!(corner(&buffer, detail.x, 0), faint, "the detail");
    let buffer = buffer_of(&bench.tab(), &bench.ctx().with_focus(PaneId::Detail));
    assert_eq!(corner(&buffer, 0, 0), faint, "the list");
    assert_eq!(corner(&buffer, detail.x, 0), accent, "the detail");
}

/// MOD-115 M4 C4: the open filter form holds the keyboard, so its pane is the focused one,
/// whichever pane the shell focuses.
#[tokio::test]
async fn the_open_filter_form_is_the_focused_pane() {
    let bench = Bench::new().await;
    let [_, detail] = split();
    let accent = bench.ctx().accent().fg;
    let faint = bench.theme.border_faint.fg;
    let mut tab = bench.tab();
    tab.form = Some(FilterForm::open(&tab.filter, &bench.projects));
    let form_y = DEFAULT_SIZE.1 - crate::ui::tabs::backlog::filter::FORM_HEIGHT;
    for ctx in [bench.ctx(), bench.ctx().with_focus(PaneId::Detail)] {
        let buffer = buffer_of(&tab, &ctx);
        assert_eq!(corner(&buffer, 0, form_y), accent, "the Filter pane");
        assert_eq!(corner(&buffer, 0, 0), faint, "the list");
        assert_eq!(corner(&buffer, detail.x, 0), faint, "the detail");
        assert!(text(&buffer, 0, form_y, 12).starts_with("╭─ Filter "));
    }
}

/// MOD-115 M4 C4: a capturing sub-tab holds the keyboard, so the detail is the focused pane
/// even while the shell focuses the list.
#[tokio::test]
async fn a_capturing_pane_is_the_focused_pane() {
    let bench = Bench::new().await;
    let [_, detail] = split();
    let accent = bench.ctx().accent().fg;
    let faint = bench.theme.border_faint.fg;
    let tab = BacklogTab {
        detail: only(CapturingProbe {
            capturing: Rc::new(Cell::new(true)),
            seen: Rc::default(),
        }),
        ..bench.tab()
    };
    let buffer = buffer_of(&tab, &bench.ctx());
    assert_eq!(corner(&buffer, detail.x, 0), accent, "the detail");
    assert_eq!(corner(&buffer, 0, 0), faint, "the list");
}

/// MOD-115 M3 D1: under an overlay no pane is focused, the one holding the keyboard included.
#[tokio::test]
async fn under_an_overlay_no_pane_is_focused() {
    let bench = Bench::new().await;
    let [_, detail] = split();
    let faint = bench.theme.border_faint.fg;
    let mut tab = bench.tab();
    tab.form = Some(FilterForm::open(&tab.filter, &bench.projects));
    let buffer = buffer_of(&tab, &bench.ctx().with_covered(true));
    let form_y = DEFAULT_SIZE.1 - crate::ui::tabs::backlog::filter::FORM_HEIGHT;
    for (x, y) in [(0, 0), (0, form_y), (detail.x, 0)] {
        assert_eq!(corner(&buffer, x, y), faint, "({x}, {y})");
    }
}

/// The style of a cell, for the monochrome marker proof below.
fn style_at(buffer: &Buffer, x: u16, y: u16) -> Style {
    buffer[(x, y)].style()
}

/// MOD-115 M4 E4: the filter form is a kit form: its focused row is the gutter and the band,
/// legible without colour, and the others have neither.
#[tokio::test]
async fn the_filter_forms_focused_row_reads_without_colour() {
    let bench = Bench {
        theme: Theme::monochrome(),
        ..Bench::new().await
    };
    let mut tab = bench.tab();
    tab.form = Some(FilterForm::open(&tab.filter, &bench.projects));
    let buffer = buffer_of(&tab, &bench.ctx());
    let status = DEFAULT_SIZE.1 - crate::ui::tabs::backlog::filter::FORM_HEIGHT + 1;
    let row = text(&buffer, 0, status, 30);
    assert!(row.starts_with("│▌status    ‹[ ] open›  any"), "{row}");
    assert!(
        style_at(&buffer, 3, status)
            .add_modifier
            .contains(Modifier::REVERSED),
        "the band"
    );
    let project = text(&buffer, 0, status + 1, 30);
    assert!(
        project.starts_with("│ project   ‹[ ] htui›  any"),
        "{project}"
    );
    assert!(
        !style_at(&buffer, 3, status + 1)
            .add_modifier
            .contains(Modifier::REVERSED),
        "no band on the next row"
    );
}
