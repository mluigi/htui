//! MOD-147 M3 D3, D5: the workspace's own group. The `Items` reply names the scope row, which the
//! list draws as the first group (titled with the workspace's name), folds, reveals into and
//! opens a `WS` form on, while `ctx.projects` stays the members alone.

use super::*;
use htui_core::fixtures::ids::{
    KIND_SCOPE_PLATFORM_WS, PROJECT_HTUI, PROJECT_SCOPE_PLATFORM, USER,
};
use htui_core::model::NewItem;

/// `WS-1` minted on Platform's scope row in `store`.
async fn mint_ws_1(store: &MemStore) -> ItemId {
    let item = store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: PROJECT_SCOPE_PLATFORM,
            kind_id: KIND_SCOPE_PLATFORM_WS,
            title: "Spans the workspace".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: USER,
            box_id: Some(htui_core::fixtures::ids::BOX),
        })
        .await
        .expect("mint WS-1 on the scope row");
    assert_eq!(item.key, "WS-1");
    item.id
}

/// The worker's whole-scope `Items` answer over `store`.
async fn items_of(bench: &Bench, store: &MemStore) -> StoreReply {
    served(store, &BacklogFilter::default().to_request(&bench.scope)).await
}

/// A tab settled on the worker's `Items` answer over `store`.
async fn settled(bench: &Bench, store: &MemStore) -> BacklogTab {
    let mut tab = BacklogTab::new();
    tab.on_reply(&items_of(bench, store).await, &mut bench.ctx());
    bench.actions();
    tab
}

/// The index of the one list-pane line holding `text` (left of the `││` between the panes).
fn line_of(frame: &str, text: &str) -> usize {
    let at: Vec<usize> = frame
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            line.split_once("\u{2502}\u{2502}")
                .is_some_and(|(list, _)| list.contains(text))
        })
        .map(|(index, _)| index)
        .collect();
    match at.as_slice() {
        [index] => *index,
        _ => panic!("expected one line holding {text:?}, got {at:?}:\n{frame}"),
    }
}

/// D3: the scope row's group is drawn first, titled with the workspace's name, its item under
/// it; the cursor arrives on that item, the list's first.
#[tokio::test]
async fn the_workspace_group_is_drawn_first_with_its_items() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let ws_1 = mint_ws_1(&store).await;
    let tab = settled(&bench, &store).await;
    let frame = drawn(&tab, &bench);
    let header = line_of(&frame, "\u{25be} Platform (1)");
    let item = line_of(&frame, "WS-1");
    let htui = line_of(&frame, "\u{25be} htui (8)");
    assert!(header < item && item < htui, "{frame}");
    let rows = tab.rows(&bench.ctx());
    assert_eq!(
        rows[..2],
        [
            Selection::Project(PROJECT_SCOPE_PLATFORM),
            Selection::Item(ws_1)
        ]
    );
    assert_eq!(tab.selected, Some(Selection::Item(ws_1)));
}

/// D3: a workspace with no item of its own still draws its group, empty.
#[tokio::test]
async fn an_empty_workspace_group_is_still_drawn() {
    let bench = Bench::new().await;
    let tab = settled(&bench, &MemStore::demo()).await;
    let frame = drawn(&tab, &bench);
    assert!(
        line_of(&frame, "\u{25be} Platform (0)") < line_of(&frame, "\u{25be} htui (8)"),
        "{frame}"
    );
}

/// D3: the workspace header folds and unfolds, and its fold survives an `Items` reply (the
/// retention reads the group projects, not `ctx.projects`).
#[tokio::test]
async fn the_workspace_group_folds_and_keeps_its_fold_over_a_reply() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    mint_ws_1(&store).await;
    let mut tab = settled(&bench, &store).await;
    tab.selected = Some(Selection::Project(PROJECT_SCOPE_PLATFORM));
    assert_eq!(press(&mut tab, &bench, KeyCode::Enter), Handled::Consumed);
    assert!(!drawn(&tab, &bench).contains("Spans the workspace"));

    tab.on_reply(&items_of(&bench, &store).await, &mut bench.ctx());
    let frame = drawn(&tab, &bench);
    assert!(frame.contains("\u{25b8} Platform (1)"), "{frame}");
    assert!(!frame.contains("Spans the workspace"), "{frame}");

    assert_eq!(press(&mut tab, &bench, KeyCode::Enter), Handled::Consumed);
    assert!(drawn(&tab, &bench).contains("Spans the workspace"));
}

/// D3: a reveal of a workspace item unfolds its group and selects a row the list draws, both
/// when the item is loaded and when the re-read brings it.
#[tokio::test]
async fn a_reveal_of_a_workspace_item_selects_a_drawn_row() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let ws_1 = mint_ws_1(&store).await;
    let target = RevealTarget::Item {
        id: ws_1,
        key: "WS-1".to_owned(),
    };

    let mut tab = settled(&bench, &store).await;
    tab.folded.push(PROJECT_SCOPE_PLATFORM);
    tab.selected = Some(Selection::Project(PROJECT_HTUI));
    assert!(tab.reveal(&target, &mut bench.ctx()));
    assert_eq!(tab.selected, Some(Selection::Item(ws_1)));
    assert!(tab.rows(&bench.ctx()).contains(&Selection::Item(ws_1)));
    bench.actions();

    let mut tab = BacklogTab::new();
    assert!(tab.reveal(&target, &mut bench.ctx()));
    let requests = sent(&bench);
    let [read @ StoreRequest::Items { .. }] = requests.as_slice() else {
        panic!("one re-read: {requests:?}")
    };
    tab.on_reply(&served(&store, read).await, &mut bench.ctx());
    assert_eq!(tab.selected, Some(Selection::Item(ws_1)));
    assert!(tab.rows(&bench.ctx()).contains(&Selection::Item(ws_1)));
    assert!(errors(&bench.actions()).is_empty());
}

/// D5: `N` on the workspace header opens a form on the scope row, its kinds the scope row's
/// (`WS`) and its project picker holding that row alone, so it cannot move to a member.
#[tokio::test]
async fn n_on_the_workspace_header_opens_a_ws_form_that_stays_on_the_row() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = settled(&bench, &store).await;
    tab.selected = Some(Selection::Project(PROJECT_SCOPE_PLATFORM));
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
    let form = tab.item_form.as_ref().expect("the form is open");
    assert_eq!(form.project(), PROJECT_SCOPE_PLATFORM);
    assert_eq!(item_form::title(form), "New item \u{b7} @platform");
    let frame = drawn(&tab, &bench);
    assert!(frame.contains("\u{2039} @platform \u{203a}"), "{frame}");
    assert!(frame.contains("\u{2039} WS "), "{frame}");

    for _ in 0..2 {
        press(&mut tab, &bench, KeyCode::BackTab);
    }
    assert_eq!(
        tab.item_form.as_ref().map(ItemForm::focus),
        Some(item_form::Field::Project)
    );
    for code in [KeyCode::Char('l'), KeyCode::Char('h')] {
        assert_eq!(press(&mut tab, &bench, code), Handled::Consumed);
        assert!(sent(&bench).is_empty(), "the picker has nowhere to go");
    }
    let form = tab.item_form.as_ref().expect("the form stays");
    assert_eq!(
        (form.busy(), form.project()),
        (None, PROJECT_SCOPE_PLATFORM)
    );
}

/// D3: a scope change forgets the scope row at once, so the old workspace's group is neither a
/// row nor drawn before the new scope's first reply, nor over it.
#[tokio::test]
async fn a_scope_change_forgets_the_workspace_group() {
    let bench = Bench::new().await;
    let mut tab = settled(&bench, &MemStore::demo()).await;
    assert!(
        tab.rows(&bench.ctx())
            .contains(&Selection::Project(PROJECT_SCOPE_PLATFORM))
    );
    tab.on_scope_change(&bench.scope);
    let rows = tab.rows(&bench.ctx());
    assert!(
        !rows.contains(&Selection::Project(PROJECT_SCOPE_PLATFORM)),
        "{rows:?}"
    );
    let frame = drawn(&tab, &bench);
    assert!(!frame.contains("Platform ("), "{frame}");

    tab.on_reply(&items_reply(bench.items.clone()), &mut bench.ctx());
    let frame = drawn(&tab, &bench);
    assert!(!frame.contains("Platform ("), "{frame}");
}
