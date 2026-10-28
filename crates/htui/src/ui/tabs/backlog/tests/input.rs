//! The Backlog's keyboard, paste and mouse routing: what captures, what passes.

use super::*;

/// MOD-4 plan OQ-7, blueprint D201: while a sub-tab captures, the list's own keys reach it
/// and the list cursor does not move; once it stops, they are the list's again.
#[tokio::test]
async fn a_capturing_sub_tab_gets_the_navigation_keys() {
    let store = MemStore::demo();
    let workspace = store
        .workspaces()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|workspace| workspace.slug == "platform")
        .expect("the demo holds `platform`");
    let scope = Scope::from_workspace(&workspace);
    let projects = store.projects(&scope).await.expect("the projects");
    let items = store
        .items(&scope, &ItemFilter::default())
        .await
        .expect("the items");
    let rows = list::rows(&items, &projects, &[]);
    let first = rows
        .iter()
        .copied()
        .find(|row| matches!(row, Selection::Item(_)))
        .expect("the scope has an item");

    let capturing = Rc::new(Cell::new(true));
    let seen = Rc::default();
    let mut detail = DetailRegistry::new();
    detail.register(Box::new(CapturingProbe {
        capturing: Rc::clone(&capturing),
        seen: Rc::clone(&seen),
    }));
    let mut tab = BacklogTab {
        items,
        folded: Vec::new(),
        selected: Some(first),
        detail,
        pending_reveal: None,
        refreshes: 0,
        filter: BacklogFilter::default(),
        shown: BacklogFilter::default(),
        form: None,
        item_form: None,
        opening: None,
        queue_intent: None,
        queue_writing: None,
        detail_drawn: Cell::new(true),
    };

    let (top_bar, keys, theme, emit) = (
        TopBarState::default(),
        Keys::compiled(),
        Theme::default(),
        Emit::default(),
    );
    let mut ctx = Ctx::new(
        &scope,
        &projects,
        &top_bar,
        keys,
        &theme,
        Origin::Tab(BacklogTab::ID),
        &emit,
    );
    let keys = [
        KeyCode::Char('j'),
        KeyCode::Char('G'),
        KeyCode::Char('l'),
        KeyCode::Char('['),
        KeyCode::Enter,
    ];
    for code in keys {
        assert_eq!(
            tab.on_key(KeyEvent::from(code), &mut ctx),
            Handled::Consumed,
            "{code:?} is the capturing sub-tab's"
        );
    }
    assert_eq!(*seen.borrow(), keys, "every key reached the sub-tab");
    assert_eq!(
        tab.selected,
        Some(first),
        "and the list cursor did not move"
    );
    assert!(emit.is_empty(), "so no detail read went out");

    capturing.set(false);
    tab.on_key(KeyEvent::from(KeyCode::Char('j')), &mut ctx);
    assert_ne!(
        tab.selected,
        Some(first),
        "a sub-tab that stopped capturing gives `j` back to the list"
    );
    // MOD-67 M5 D5: the pane is offered every key first, and passed `j` to the list.
    assert_eq!(
        seen.borrow().len(),
        keys.len() + 1,
        "offered first, and passed"
    );
    assert_eq!(seen.borrow().last(), Some(&KeyCode::Char('j')));
}

/// MOD-14 D8, `open graph`: `m` on the list selects the Graph sub-tab and nothing else.
#[tokio::test]
async fn m_on_the_list_opens_the_graph() {
    let (scope, projects, items, first) = platform().await;
    let mut tab = BacklogTab {
        items,
        folded: Vec::new(),
        selected: Some(first),
        detail: BacklogTab::new().detail,
        pending_reveal: None,
        refreshes: 0,
        filter: BacklogFilter::default(),
        shown: BacklogFilter::default(),
        form: None,
        item_form: None,
        opening: None,
        queue_intent: None,
        queue_writing: None,
        detail_drawn: Cell::new(true),
    };
    let (top_bar, keys, theme, emit) = (
        TopBarState::default(),
        Keys::compiled(),
        Theme::default(),
        Emit::default(),
    );
    let mut ctx = Ctx::new(
        &scope,
        &projects,
        &top_bar,
        keys,
        &theme,
        Origin::Tab(BacklogTab::ID),
        &emit,
    );
    assert_ne!(tab.detail.active_id(), Some(GraphTab::ID));
    assert_eq!(
        tab.on_key(KeyEvent::from(KeyCode::Char('m')), &mut ctx),
        Handled::Consumed
    );
    assert_eq!(tab.detail.active_id(), Some(GraphTab::ID));
    assert_eq!(tab.selected, Some(first), "the list cursor did not move");
    assert!(emit.is_empty(), "so no detail read went out");
}

/// MOD-14 D8: `m` is below the capture guard, so it is a capturing sub-tab's letter.
#[tokio::test]
async fn m_while_a_sub_tab_captures_is_the_sub_tab_s() {
    let (scope, projects, items, first) = platform().await;
    let capturing = Rc::new(Cell::new(true));
    let seen: Rc<RefCell<Vec<KeyCode>>> = Rc::default();
    let mut detail = DetailRegistry::new();
    detail.register(Box::new(CapturingProbe {
        capturing: Rc::clone(&capturing),
        seen: Rc::clone(&seen),
    }));
    detail.register(Box::new(GraphTab::new()));
    let mut tab = BacklogTab {
        items,
        folded: Vec::new(),
        selected: Some(first),
        detail,
        pending_reveal: None,
        refreshes: 0,
        filter: BacklogFilter::default(),
        shown: BacklogFilter::default(),
        form: None,
        item_form: None,
        opening: None,
        queue_intent: None,
        queue_writing: None,
        detail_drawn: Cell::new(true),
    };
    let (top_bar, keys, theme, emit) = (
        TopBarState::default(),
        Keys::compiled(),
        Theme::default(),
        Emit::default(),
    );
    let mut ctx = Ctx::new(
        &scope,
        &projects,
        &top_bar,
        keys,
        &theme,
        Origin::Tab(BacklogTab::ID),
        &emit,
    );
    let m = KeyEvent::from(KeyCode::Char('m'));
    assert_eq!(tab.on_key(m, &mut ctx), Handled::Consumed);
    assert_eq!(*seen.borrow(), [KeyCode::Char('m')], "the probe got it");
    assert_eq!(tab.detail.active_id(), Some(DetailId("probe")));

    capturing.set(false);
    assert_eq!(tab.on_key(m, &mut ctx), Handled::Consumed);
    assert_eq!(tab.detail.active_id(), Some(GraphTab::ID));
    // MOD-67 M5 D5: a pane in browse is offered every key first; the probe passed `m`.
    assert_eq!(
        seen.borrow().len(),
        2,
        "the probe was offered `m`, and passed it"
    );
}

// ---- MOD-71: the mouse reaches the detail pane (plan D1, D4) ----------------------------

/// MOD-71 D1, D4: the detail pane gets the mouse only while nothing is typed over it — the
/// filter form and the item form each take it away — and only while its sub-tab wants it.
#[tokio::test]
async fn the_detail_gets_the_mouse_only_with_no_form_open() {
    let bench = Bench::new().await;
    let (probe, wants, seen, _lost) = mouse_probe("mouse", true);
    let mut detail = DetailRegistry::new();
    detail.register(probe);
    let mut tab = BacklogTab {
        detail,
        ..bench.tab()
    };
    assert!(tab.wants_mouse());
    assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Consumed);
    assert_eq!(seen.borrow().len(), 1);

    press(&mut tab, &bench, KeyCode::Char('f'));
    assert!(tab.form.is_some(), "the filter form is open");
    assert!(!tab.wants_mouse(), "the filter form takes the mouse away");
    assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
    assert_eq!(seen.borrow().len(), 1);
    press(&mut tab, &bench, KeyCode::Esc);
    assert!(tab.form.is_none(), "Esc closes it");
    assert!(tab.wants_mouse(), "and the mouse is wanted again");

    let _ = bench.actions();
    let store = MemStore::demo();
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
    assert!(!tab.wants_mouse(), "the item form takes the mouse away");
    assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
    assert_eq!(seen.borrow().len(), 1);
    press(&mut tab, &bench, KeyCode::Esc);
    assert!(tab.item_form.is_none(), "Esc closes it");
    assert!(tab.wants_mouse(), "and the mouse is wanted again");

    wants.set(false);
    assert!(!tab.wants_mouse(), "the sub-tab's own answer counts");
    assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
    assert_eq!(seen.borrow().len(), 1);
}

/// MOD-71 D1, D4: a hidden sub-tab is not on screen to be clicked, so only the active one is
/// asked.
#[tokio::test]
async fn only_the_active_sub_tab_is_asked_for_the_mouse() {
    let bench = Bench::new().await;
    let (a, _a_wants, a_seen, _a_lost) = mouse_probe("a", false);
    let (b, _b_wants, b_seen, _b_lost) = mouse_probe("b", true);
    let mut registry = DetailRegistry::new();
    registry.register(a);
    registry.register(b);
    assert!(!registry.wants_mouse());
    assert_eq!(registry.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
    assert!(a_seen.borrow().is_empty() && b_seen.borrow().is_empty());

    assert!(registry.select(1));
    assert!(registry.wants_mouse());
    assert_eq!(
        registry.on_mouse(click(), &mut bench.ctx()),
        Handled::Consumed
    );
    assert_eq!(
        *b_seen.borrow(),
        vec![MouseEventKind::Down(MouseButton::Left)]
    );
    assert!(a_seen.borrow().is_empty());
}

/// MOD-74 D3, H-12: a lost capture reaches every sub-tab, the inactive one holding a gesture
/// included, and no form gates it: a form opening is itself one of the losses.
#[tokio::test]
async fn a_lost_capture_reaches_every_sub_tab() {
    let bench = Bench::new().await;
    let (a, _a_wants, _a_seen, a_lost) = mouse_probe("a", false);
    let (b, _b_wants, _b_seen, b_lost) = mouse_probe("b", true);
    let mut detail = DetailRegistry::new();
    detail.register(a);
    detail.register(b);
    let mut tab = BacklogTab {
        detail,
        ..bench.tab()
    };
    press(&mut tab, &bench, KeyCode::Char('f'));
    assert!(tab.form.is_some(), "the filter form is open");
    assert_eq!(tab.detail.active_id(), Some(DetailId("a")), "b is inactive");
    tab.on_mouse_lost();
    assert_eq!(a_lost.get(), 1, "the active sub-tab is told");
    assert_eq!(b_lost.get(), 1, "and so is the inactive one");
}

/// MOD-13 review L4: a chord never acts as its letter in the open form. MOD-67 M5: what passes
/// is `BACKLOG_FILTER`'s modal layer's rule (D5), so an `Alt` chord reaches the shell and a
/// `Super`, `Meta` or `Hyper` one is swallowed, as in every M3/M4 modal mode.
#[tokio::test]
async fn an_alt_chord_passes_through_the_open_form() {
    let bench = Bench::new().await;
    let mut tab = BacklogTab {
        filter: done_only(),
        ..bench.tab()
    };
    press(&mut tab, &bench, KeyCode::Char('f'));
    for (modifier, handled) in [
        (KeyModifiers::ALT, Handled::Pass),
        (KeyModifiers::SUPER, Handled::Consumed),
        (KeyModifiers::META, Handled::Consumed),
        (KeyModifiers::HYPER, Handled::Consumed),
    ] {
        assert_eq!(
            tab.on_key(
                KeyEvent::new(KeyCode::Char('x'), modifier),
                &mut bench.ctx()
            ),
            handled,
            "{modifier:?}"
        );
    }
    press(&mut tab, &bench, KeyCode::Enter);
    assert_eq!(tab.filter, done_only(), "no chord cleared the draft");
}

/// MOD-52: `ctrl-c` quits from inside the form, as it does from every text field.
#[tokio::test]
async fn ctrl_c_passes_through_the_open_form() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    press(&mut tab, &bench, KeyCode::Char('f'));
    assert_eq!(
        tab.on_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &mut bench.ctx()
        ),
        Handled::Pass
    );
    assert!(tab.form.is_some());
}

/// MOD-22 review M-1: a paste reaches the open form's tag field, not the detail pane.
#[tokio::test]
async fn a_paste_while_the_form_is_open_goes_to_its_tag_field() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    press(&mut tab, &bench, KeyCode::Char('f'));
    press(&mut tab, &bench, KeyCode::Down);
    press(&mut tab, &bench, KeyCode::Down);
    assert_eq!(
        tab.on_paste("rust, gpu", &mut bench.ctx()),
        Handled::Consumed
    );
    press(&mut tab, &bench, KeyCode::Enter);
    assert_eq!(tab.filter.tags, ["gpu", "rust"]);
}
