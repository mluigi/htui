//! The Backlog's key hints: the list's keys after the focused pane's in the hint row.

use super::render::{buffer_of, split, text};
use super::*;

// ---- MOD-67 M5 Q1 (a), PA-5: the list hint row --------------------------------------------

/// Q1 (a), PA-5, MOD-115 M4 Q2: the list keys are the focused list pane's own hints, on its
/// bottom border; the detail, unfocused, draws none. `Tab::key_hint` keeps the shell's row for
/// the activity pane (C8): on the Body pane, which has no keys of its own, the list's alone.
#[tokio::test]
async fn the_list_hint_is_on_the_focused_list_panes_border() {
    let bench = Bench::new().await;
    let tab = bench.tab();
    let hint = tab.list_hint(Keys::compiled()).expect("nothing captures");
    assert_eq!(hint, DEFAULT_LIST_HINT);
    assert_eq!(
        row_at(&tab, &bench, 100).as_deref(),
        Some(DEFAULT_LIST_HINT)
    );
    let buffer = buffer_of(&tab, &bench.ctx());
    let [list, detail] = split();
    let bottom = DEFAULT_SIZE.1 - 1;
    assert_eq!(
        text(&buffer, list.x, bottom, list.width),
        format!("╰─{DEFAULT_LIST_HINT}──╯")
    );
    assert_eq!(
        text(&buffer, detail.x, bottom, detail.width),
        format!("╰{}╯", "─".repeat(usize::from(detail.width) - 2)),
        "the unfocused detail draws no hint"
    );
}

/// MOD-115 M4 Q2: with the detail focused its own keys go on its border, the list's after them
/// only when both fit the pane (`Tab::key_hint`'s join at the detail's width); the list draws
/// none.
#[tokio::test]
async fn the_focused_detail_carries_its_own_keys() {
    let bench = Bench::new().await;
    let Selection::Item(item) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    tab.detail.on_item_change(Some(item));
    assert!(tab.detail.select_id(RunsTab::ID));
    let [list, detail] = split();
    let room = detail.width - 4;
    let wanted = row_at(&tab, &bench, room).expect("the Runs pane has keys");
    assert!(
        !wanted.contains(DEFAULT_LIST_HINT),
        "{wanted}: too wide to join"
    );
    let buffer = buffer_of(&tab, &bench.ctx().with_focus(PaneId::Detail));
    let bottom = DEFAULT_SIZE.1 - 1;
    let row = text(&buffer, detail.x, bottom, detail.width);
    assert!(
        row.starts_with(&format!("╰─{wanted}")),
        "{row} for {wanted}"
    );
    let row = text(&buffer, list.x, bottom, list.width);
    assert!(!row.contains("move"), "the list draws no hint: {row}");
}

/// MOD-115 M4 Q2: the open filter form's keys go on the Filter pane's border, the bottom row
/// of the list column; the detail draws none.
#[tokio::test]
async fn the_filter_forms_keys_are_on_its_pane() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let form = FilterForm::open(&tab.filter, &bench.projects);
    let keys = form.key_hint(Keys::compiled());
    tab.form = Some(form);
    let buffer = buffer_of(&tab, &bench.ctx());
    let [list, detail] = split();
    let bottom = DEFAULT_SIZE.1 - 1;
    let row = text(&buffer, list.x, bottom, list.width);
    assert!(row.starts_with(&format!("╰─{keys}")), "{row}");
    let row = text(&buffer, detail.x, bottom, detail.width);
    assert!(!row.contains("apply"), "{row}");
}

/// MOD-67 M6 T2: browsing a pane with keys of its own, the pane's lead and the list's follow
/// after ` · ` when both fit the row; when they do not, the pane's stand alone.
#[tokio::test]
async fn the_pane_hint_leads_and_the_list_keys_follow_when_they_fit() {
    let bench = Bench::new().await;
    let Selection::Item(item) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    tab.detail.on_item_change(Some(item));
    assert!(tab.detail.select_id(RunsTab::ID));
    let pane = tab
        .detail
        .key_hint(Keys::compiled(), &Theme::default())
        .expect("the Runs pane has keys")
        .to_string();
    let joined = format!("{pane} \u{b7} {DEFAULT_LIST_HINT}");
    let fits = u16::try_from(cell_width(&joined)).expect("a narrow row");
    assert!(fits <= 100, "{joined:?} fits 100 columns");
    assert_eq!(row_at(&tab, &bench, 100).as_deref(), Some(joined.as_str()));
    assert_eq!(row_at(&tab, &bench, fits).as_deref(), Some(joined.as_str()));
    assert_eq!(
        row_at(&tab, &bench, fits - 1).as_deref(),
        Some(pane.as_str())
    );
    assert_eq!(row_at(&tab, &bench, 30).as_deref(), Some(pane.as_str()));
}

/// MOD-67 M6 T2: a capturing pane's own keys stand alone (the list's do not apply while it
/// takes every key), and an open form's are the row.
#[tokio::test]
async fn a_capturing_pane_shows_only_its_own_keys() {
    let bench = Bench::new().await;
    let Selection::Item(item) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    tab.detail.on_item_change(Some(item));
    assert!(tab.detail.select_id(NotesTab::ID));
    press(&mut tab, &bench, KeyCode::Char('a'));
    tab.on_reply(&StoreReply::NoteForm { item }, &mut bench.ctx());
    assert!(tab.detail.captures_input(), "the note area is open");
    let pane = tab
        .detail
        .key_hint(Keys::compiled(), &Theme::default())
        .expect("the area has keys")
        .to_string();
    assert_eq!(row_at(&tab, &bench, 100), Some(pane.clone()));
    assert!(!pane.contains(DEFAULT_LIST_HINT), "{pane}");

    let mut tab = bench.tab();
    let form = FilterForm::open(&tab.filter, &bench.projects);
    let keys = form.key_hint(Keys::compiled());
    tab.form = Some(form);
    assert_eq!(row_at(&tab, &bench, 100), Some(keys), "the filter form's");
}

/// Q1 (a): a hint for keys a field is typing would lie, so the row is not drawn while the
/// filter form, the item form or a sub-tab captures.
#[tokio::test]
async fn the_list_hint_is_hidden_while_a_form_or_pane_captures() {
    let bench = Bench::new().await;

    let mut tab = bench.tab();
    tab.form = Some(FilterForm::open(&tab.filter, &bench.projects));
    assert_eq!(tab.list_hint(Keys::compiled()), None, "the filter form");
    let frame = drawn(&tab, &bench);
    assert!(!frame.contains(DEFAULT_LIST_HINT), "filter form:\n{frame}");

    let mut tab = bench.tab();
    let backend = htui_store::Backend::memory(MemStore::demo());
    let request = StoreRequest::ItemForm {
        project: htui_core::fixtures::ids::PROJECT_HTUI,
        item: None,
    };
    let context = match item_writes::serve(&backend, &request).await {
        Ok(StoreReply::ItemForm(context)) => *context,
        other => panic!("the form read answered {other:?}"),
    };
    tab.item_form = Some(ItemForm::open_new(context, &bench.projects, None));
    assert_eq!(tab.list_hint(Keys::compiled()), None, "the item form");
    let frame = drawn(&tab, &bench);
    assert!(!frame.contains(DEFAULT_LIST_HINT), "item form:\n{frame}");

    let capturing = Rc::new(Cell::new(true));
    let mut detail = DetailRegistry::new();
    detail.register(Box::new(CapturingProbe {
        capturing: Rc::clone(&capturing),
        seen: Rc::default(),
    }));
    let tab = BacklogTab {
        detail,
        ..bench.tab()
    };
    assert_eq!(tab.list_hint(Keys::compiled()), None, "a capturing pane");
    let frame = drawn(&tab, &bench);
    assert!(
        !frame.contains(DEFAULT_LIST_HINT),
        "capturing pane:\n{frame}"
    );
    capturing.set(false);
    assert_eq!(
        tab.list_hint(Keys::compiled()).as_deref(),
        Some(DEFAULT_LIST_HINT),
        "back once the pane stops capturing"
    );
}

/// Q1 (a): the hint is built from the keys in force, so `[backlog] filter = "/"` reads
/// `/ filter`.
#[tokio::test]
async fn the_list_hint_names_a_rebound_key() {
    let bench = Bench::new().await;
    let keys =
        crate::keys::load_str("version = 1\n[backlog]\nfilter = \"/\"\n").expect("the keys load");
    assert_eq!(
        bench.tab().list_hint(&keys).as_deref(),
        Some("j/k move \u{b7} h/l sub-tab \u{b7} / filter \u{b7} N new \u{b7} e edit")
    );
}
