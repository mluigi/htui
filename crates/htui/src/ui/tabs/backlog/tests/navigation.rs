//! The Backlog's selection: reveals, the cursor over filtered rows, the panes it claims, focus
//! motion and the location it saves and restores.

use super::*;

/// MOD-64 review 6, Requirements' D251 rule: a refused `Items` read disarms a pending reveal,
/// so a later list neither jumps to nor reports an item the user has since moved on from.
#[tokio::test]
async fn a_refused_items_read_disarms_a_pending_reveal() {
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
    let read = StoreRequest::Items {
        scope: scope.clone(),
        filter: ItemFilter::default(),
        ready_here: false,
    };
    assert_eq!(ITEMS_READ, read.name());
    let mut tab = BacklogTab::new();
    let target = RevealTarget::Item {
        id: ItemId::default(),
        key: "GONE-1".to_owned(),
    };
    assert!(tab.reveal(&target, &mut ctx));
    assert!(
        tab.pending_reveal.is_some(),
        "not loaded: the next list decides"
    );
    let _ = emit.take();

    tab.on_reply(
        &StoreReply::Failed {
            request: "items",
            message: "the server went away".to_owned(),
        },
        &mut ctx,
    );
    assert!(tab.pending_reveal.is_none(), "the refusal disarms it");

    tab.on_reply(&StoreReply::Items(Vec::new()), &mut ctx);
    let errors: Vec<Action> = emit
        .take()
        .into_iter()
        .filter(|action| matches!(action, Action::Error(_)))
        .collect();
    assert!(
        errors.is_empty(),
        "a later list reports nothing: {errors:?}"
    );
}

/// MOD-14 D8: `select_id` activates a registered sub-tab by id and leaves an unknown id alone.
#[test]
fn select_id_finds_a_registered_sub_tab() {
    let mut detail = BacklogTab::new().detail;
    assert!(detail.select_id(GraphTab::ID));
    assert_eq!(detail.active_id(), Some(GraphTab::ID));
    assert!(!detail.select_id(DetailId("nope")));
    assert_eq!(
        detail.active_id(),
        Some(GraphTab::ID),
        "an unknown id changes nothing"
    );
}

/// MOD-13 D4: a reveal of an item the filter hides clears the filter and re-reads, so D251's
/// "not in this backlog" only fires for an item the workspace does not hold.
#[tokio::test]
async fn a_reveal_of_a_hidden_item_clears_the_filter() {
    let bench = Bench::new().await;
    let done: Vec<ItemSummary> = bench
        .items
        .iter()
        .filter(|item| item.status == Status::Done)
        .cloned()
        .collect();
    assert!(
        done.iter()
            .all(|item| item.id != htui_core::fixtures::ids::HTUI_ANA_2)
    );
    let mut tab = BacklogTab {
        items: done,
        filter: done_only(),
        ..bench.tab()
    };
    let target = RevealTarget::Item {
        id: htui_core::fixtures::ids::HTUI_ANA_2,
        key: "ANA-2".to_owned(),
    };
    assert!(tab.reveal(&target, &mut bench.ctx()));
    assert!(tab.filter.is_empty(), "the filter is cleared");
    assert!(tab.pending_reveal.is_some(), "the next list decides");
    let actions = bench.actions();
    assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);

    tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
    assert_eq!(
        tab.selected,
        Some(Selection::Item(htui_core::fixtures::ids::HTUI_ANA_2))
    );
    assert!(
        !bench
            .actions()
            .iter()
            .any(|action| matches!(action, Action::Error(_))),
        "the whole list holds it, so nothing is reported"
    );
}

/// A reveal would drop the half-edited form, as it would a half-typed note.
#[tokio::test]
async fn a_reveal_while_the_form_is_open_asks_to_close_it_first() {
    let bench = Bench::new().await;
    let mut tab = BacklogTab {
        filter: done_only(),
        ..bench.tab()
    };
    press(&mut tab, &bench, KeyCode::Char('f'));
    let target = RevealTarget::Item {
        id: htui_core::fixtures::ids::HTUI_ANA_2,
        key: "ANA-2".to_owned(),
    };
    assert!(tab.reveal(&target, &mut bench.ctx()));
    let actions = bench.actions();
    assert!(
        matches!(actions.as_slice(), [Action::Error(sentence)] if sentence == CLOSE_THE_FIELD_FIRST),
        "{actions:?}"
    );
    assert!(tab.form.is_some());
    assert_eq!(tab.filter, done_only(), "the filter stays");
    assert!(tab.pending_reveal.is_none());
}

// ---- MOD-69 plan D8: a reveal to a run's step (blueprint §4.3) ----------------------------

#[tokio::test]
async fn revealing_a_step_of_a_loaded_item_selects_it_opens_runs_and_reads_once() {
    use htui_core::fixtures::ids;
    let bench = Bench::new().await;
    let mut tab = BacklogTab {
        selected: None,
        ..bench.tab()
    };
    assert!(tab.reveal(&feat_1_review(), &mut bench.ctx()));
    assert_eq!(tab.selected, Some(Selection::Item(ids::HTUI_FEAT_1)));
    assert_eq!(tab.detail.active_id(), Some(RunsTab::ID));
    assert_eq!(tab.pending_reveal, None);
    let actions = bench.actions();
    assert_eq!(
        runs_reads(&actions),
        [ids::HTUI_FEAT_1],
        "the selection's own read carries the focus: {actions:?}"
    );
}

#[tokio::test]
async fn revealing_a_step_of_the_selected_item_re_reads_its_runs() {
    use htui_core::fixtures::ids;
    let bench = Bench::new().await;
    let mut tab = BacklogTab {
        selected: Some(Selection::Item(ids::HTUI_FEAT_1)),
        ..bench.tab()
    };
    assert!(tab.reveal(&feat_1_review(), &mut bench.ctx()));
    assert_eq!(tab.selected, Some(Selection::Item(ids::HTUI_FEAT_1)));
    assert_eq!(tab.detail.active_id(), Some(RunsTab::ID));
    let actions = bench.actions();
    assert_eq!(
        runs_reads(&actions),
        [ids::HTUI_FEAT_1],
        "`go` short-circuited, so the reveal asks for the rows: {actions:?}"
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Store(StoreRequest::Item(_)))),
        "and nothing else of the item is re-read: {actions:?}"
    );
}

#[tokio::test]
async fn revealing_a_step_of_an_unloaded_item_carries_the_focus_to_the_items_reply() {
    use htui_core::fixtures::ids;
    let bench = Bench::new().await;
    let others: Vec<ItemSummary> = bench
        .items
        .iter()
        .filter(|item| item.id != ids::HTUI_FEAT_1)
        .cloned()
        .collect();
    let mut tab = BacklogTab {
        items: others,
        selected: None,
        ..bench.tab()
    };
    assert!(tab.reveal(&feat_1_review(), &mut bench.ctx()));
    assert_eq!(
        tab.pending_reveal,
        Some(PendingReveal {
            id: ids::HTUI_FEAT_1,
            key: "FEAT-1".to_owned(),
            focus: Some((Some(ids::RUN_1), Some(ids::STEP_REVIEW))),
        })
    );
    let actions = bench.actions();
    assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);
    assert!(runs_reads(&actions).is_empty(), "{actions:?}");

    tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
    assert_eq!(tab.selected, Some(Selection::Item(ids::HTUI_FEAT_1)));
    assert_eq!(tab.detail.active_id(), Some(RunsTab::ID));
    assert_eq!(tab.pending_reveal, None);
    let actions = bench.actions();
    assert_eq!(runs_reads(&actions), [ids::HTUI_FEAT_1], "{actions:?}");
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Error(_))),
        "{actions:?}"
    );
}

#[tokio::test]
async fn a_step_reveal_while_a_field_is_open_asks_to_close_it_first() {
    let bench = Bench::new().await;
    let mut tab = BacklogTab {
        selected: None,
        ..bench.tab()
    };
    press(&mut tab, &bench, KeyCode::Char('f'));
    assert!(tab.reveal(&feat_1_review(), &mut bench.ctx()));
    let actions = bench.actions();
    assert!(
        matches!(actions.as_slice(), [Action::Error(sentence)] if sentence == CLOSE_THE_FIELD_FIRST),
        "{actions:?}"
    );
    assert!(tab.form.is_some());
    assert_eq!(tab.selected, None, "the cursor did not move");
    assert_ne!(tab.detail.active_id(), Some(RunsTab::ID));
    assert!(tab.pending_reveal.is_none());
}

/// A Reopen row names no run: the item and its Runs pane are the target, and only the
/// selection's own read goes out (none at all when the item was already selected).
#[tokio::test]
async fn a_step_reveal_with_no_run_opens_runs_without_asking_twice() {
    use htui_core::fixtures::ids;
    let bench = Bench::new().await;
    let reopen = RevealTarget::Step {
        item: ids::HTUI_FEAT_1,
        key: "FEAT-1".to_owned(),
        run: None,
        step: None,
    };
    let mut tab = BacklogTab {
        selected: None,
        ..bench.tab()
    };
    assert!(tab.reveal(&reopen, &mut bench.ctx()));
    assert_eq!(tab.selected, Some(Selection::Item(ids::HTUI_FEAT_1)));
    assert_eq!(tab.detail.active_id(), Some(RunsTab::ID));
    let actions = bench.actions();
    assert_eq!(runs_reads(&actions), [ids::HTUI_FEAT_1], "{actions:?}");

    assert!(tab.reveal(&reopen, &mut bench.ctx()));
    let actions = bench.actions();
    assert!(runs_reads(&actions).is_empty(), "{actions:?}");
}

/// A filtered reply without the cursor's row moves the cursor to its first item, which
/// re-reads that item's detail.
#[tokio::test]
async fn a_filtered_reply_moves_the_cursor_to_its_first_item() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let Selection::Item(first) = bench.first else {
        panic!("the first row is an item")
    };
    let others: Vec<ItemSummary> = bench
        .items
        .iter()
        .filter(|item| item.id != first)
        .cloned()
        .collect();
    let expected = list::rows(&others, &bench.projects, &[])
        .into_iter()
        .find(|row| matches!(row, Selection::Item(_)))
        .expect("another item");
    tab.on_reply(&StoreReply::Items(others), &mut bench.ctx());
    assert_eq!(tab.selected, Some(expected));
    let Selection::Item(id) = expected else {
        unreachable!()
    };
    assert!(
        bench
            .actions()
            .iter()
            .any(|action| matches!(action, Action::Store(StoreRequest::Item(read)) if *read == id)),
        "the new row's detail is read"
    );
}

/// MOD-13 review M2: a filter nothing matches draws no row, so there is no row to select,
/// fold or step over; `F` then brings the whole list back with nothing folded.
#[tokio::test]
async fn a_filter_nothing_matches_leaves_no_row_to_move_or_fold() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let cuda = BacklogFilter {
        tags: vec!["cuda".to_owned()],
        ..BacklogFilter::default()
    };
    tab.apply(cuda, &bench.ctx());
    tab.on_reply(&StoreReply::Items(Vec::new()), &mut bench.ctx());
    assert_eq!(tab.selected, None, "no row, no cursor");
    assert!(drawn(&tab, &bench).contains(list::NO_MATCH));
    let _ = bench.actions();

    for code in [
        KeyCode::Enter,
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('G'),
        KeyCode::Char('g'),
    ] {
        press(&mut tab, &bench, code);
        assert_eq!(tab.selected, None, "{code:?} found no row");
    }
    assert!(tab.folded.is_empty(), "`Enter` folded nothing");
    // MOD-67 M5 D6: the tab answers the missed `Enter` itself (the legacy shell row did).
    let actions = bench.actions();
    assert!(
        matches!(
            actions.as_slice(),
            [Action::Error(miss)] if miss == "select a step in the Runs pane (J/K) to replay it"
        ),
        "and nothing was read: {actions:?}"
    );

    press(&mut tab, &bench, KeyCode::Char('F'));
    tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
    assert!(tab.folded.is_empty(), "no project comes back folded");
    assert_eq!(tab.selected, Some(bench.first));
}

/// MOD-13 review L5: a fold belongs to the scope's project, not to the rows one read holds,
/// so a filter that hides the project's items does not forget it.
#[tokio::test]
async fn a_fold_survives_a_filter_that_hides_its_project() {
    use htui_core::fixtures::ids::{PROJECT_AGY, PROJECT_HTUI};
    let bench = Bench::new().await;
    let mut tab = BacklogTab {
        folded: vec![PROJECT_AGY],
        ..bench.tab()
    };
    let htui_only = BacklogFilter {
        projects: vec![PROJECT_HTUI],
        ..BacklogFilter::default()
    };
    tab.apply(htui_only, &bench.ctx());
    let htui: Vec<ItemSummary> = bench
        .items
        .iter()
        .filter(|item| item.project_id == PROJECT_HTUI)
        .cloned()
        .collect();
    tab.on_reply(&StoreReply::Items(htui), &mut bench.ctx());
    assert_eq!(tab.folded, [PROJECT_AGY]);

    press(&mut tab, &bench, KeyCode::Char('F'));
    tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
    assert_eq!(tab.folded, [PROJECT_AGY], "still folded after `F`");
}

// ---- MOD-115 M2 T7: the panes, motion by focus, the location ---------------------------

/// MOD-115 M2 D2: the Backlog is a list + detail tab, its default today's 55/45 split, and
/// its render claims exactly the rects the tab drew before the shell kept layouts.
#[tokio::test]
async fn the_backlog_claims_a_list_and_a_detail_at_today_s_split() {
    assert_eq!(BacklogTab::new().panes(), Panes::ListDetail(BACKLOG_LAYOUT));
    assert_eq!(BACKLOG_LAYOUT.list, ListSize::Percent(55));
    assert_eq!(BACKLOG_LAYOUT.preset, Preset::Ide);

    let bench = Bench::new().await;
    let claim = Cell::new(None);
    let ctx = bench.ctx().with_pane_claim(&claim);
    let _ = drawn_through(&bench.tab(), &ctx);
    let (region, areas) = claim.get().expect("the render claims its panes");
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(region);
    assert_eq!(region, Rect::new(0, 0, DEFAULT_SIZE.0, DEFAULT_SIZE.1));
    assert_eq!((areas.list, areas.detail), (Some(left), right));
    assert_eq!(areas.activity, None);
}

/// MOD-115 M2 D2: a layout that hides the list gives the detail the whole body; the
/// divergence view still takes the whole area and claims nothing.
#[tokio::test]
async fn a_hidden_list_gives_the_detail_the_whole_body() {
    let bench = Bench::new().await;
    let key = bench
        .items
        .iter()
        .find(|item| Selection::Item(item.id) == bench.first)
        .map(|item| item.key.clone())
        .expect("the first row is an item");
    let focus = TabLayout {
        preset: Preset::Focus,
        ..BACKLOG_LAYOUT
    };
    let frame = drawn_through(&bench.tab(), &bench.ctx().with_layout(focus));
    let top = frame.lines().next().expect("a first row");
    assert!(
        top.starts_with(&format!("\u{250c} {key} ")),
        "the detail starts at the left edge:\n{frame}"
    );
    let both = drawn_through(&bench.tab(), &bench.ctx());
    assert!(
        !both
            .lines()
            .next()
            .unwrap_or("")
            .starts_with(&format!("\u{250c} {key} ")),
        "the list is on the left by default:\n{both}"
    );
}

/// MOD-115 M2 §7: with the list focused, motion moves the list as before; the pane is
/// offered the key first and takes no step.
#[tokio::test]
async fn with_the_list_focused_motion_moves_the_list() {
    let bench = Bench::new().await;
    let (mut tab, steps, keys) = stepping(&bench);
    let mut ctx = bench.ctx().with_focus(PaneId::List);
    assert_eq!(
        tab.on_key(KeyEvent::from(KeyCode::Char('j')), &mut ctx),
        Handled::Consumed
    );
    assert_ne!(tab.selected, Some(bench.first), "the cursor moved");
    assert!(steps.borrow().is_empty(), "the pane took no step");
    assert_eq!(*keys.borrow(), [KeyCode::Char('j')], "offered first");
}

/// MOD-115 M2 §7: with the detail focused, `list.down`/`up`/`top`/`bottom` step the detail
/// pane and the list cursor stays, so no detail read goes out.
#[tokio::test]
async fn with_the_detail_focused_motion_steps_the_detail_pane() {
    let bench = Bench::new().await;
    let (mut tab, steps, _) = stepping(&bench);
    let mut ctx = bench.ctx().with_focus(PaneId::Detail);
    for code in [
        KeyCode::Char('j'),
        KeyCode::Up,
        KeyCode::Char('g'),
        KeyCode::End,
    ] {
        assert_eq!(
            tab.on_key(KeyEvent::from(code), &mut ctx),
            Handled::Consumed,
            "{code:?}"
        );
    }
    assert_eq!(
        *steps.borrow(),
        [Step::Down, Step::Up, Step::Top, Step::Bottom]
    );
    assert_eq!(tab.selected, Some(bench.first), "the list cursor stayed");
    assert!(bench.actions().is_empty(), "so nothing was read");
}

/// MOD-115 M2 §7: a sub-tab that takes no step (the trait's default) still keeps the list
/// cursor where it is while the detail is focused.
#[tokio::test]
async fn a_pane_that_takes_no_step_still_keeps_the_list_cursor() {
    let bench = Bench::new().await;
    let mut detail = DetailRegistry::new();
    detail.register(Box::new(CapturingProbe {
        capturing: Rc::new(Cell::new(false)),
        seen: Rc::default(),
    }));
    let mut tab = BacklogTab {
        detail,
        ..bench.tab()
    };
    let mut ctx = bench.ctx().with_focus(PaneId::Detail);
    assert_eq!(
        tab.on_key(KeyEvent::from(KeyCode::Char('j')), &mut ctx),
        Handled::Consumed
    );
    assert_eq!(tab.selected, Some(bench.first));
}

/// MOD-115 M2 §7: with the activity pane focused, motion is consumed and nothing moves.
#[tokio::test]
async fn with_the_activity_focused_motion_moves_nothing() {
    let bench = Bench::new().await;
    let (mut tab, steps, _) = stepping(&bench);
    let mut ctx = bench.ctx().with_focus(PaneId::Activity);
    for code in [KeyCode::Char('j'), KeyCode::Char('k'), KeyCode::Char('G')] {
        assert_eq!(
            tab.on_key(KeyEvent::from(code), &mut ctx),
            Handled::Consumed,
            "{code:?}"
        );
    }
    assert!(steps.borrow().is_empty(), "the detail took no step");
    assert_eq!(tab.selected, Some(bench.first), "the list cursor stayed");
    assert!(bench.actions().is_empty());
}

/// MOD-115 M2 §7: `Enter` on a project header folds it with the list focused; with the
/// detail focused it skips the fold and reaches the pane (the Runs pane replays there).
#[tokio::test]
async fn enter_with_the_detail_focused_reaches_the_pane_not_the_fold() {
    let bench = Bench::new().await;
    let project = first_project(&bench);
    let (mut tab, _, keys) = stepping(&bench);
    tab.selected = Some(Selection::Project(project));
    let mut ctx = bench.ctx().with_focus(PaneId::Detail);
    let _ = tab.on_key(KeyEvent::from(KeyCode::Enter), &mut ctx);
    assert!(tab.folded.is_empty(), "no fold with the detail focused");
    assert_eq!(*keys.borrow(), [KeyCode::Enter], "the pane was offered it");

    let mut ctx = bench.ctx().with_focus(PaneId::List);
    assert_eq!(
        tab.on_key(KeyEvent::from(KeyCode::Enter), &mut ctx),
        Handled::Consumed
    );
    assert_eq!(tab.folded, [project], "the list's fold");
}

/// MOD-115 M2 §7: a view verb acts whatever pane is focused: `f` opens the filter with the
/// detail focused, and `l` cycles the sub-tab with the activity pane focused.
#[tokio::test]
async fn a_view_verb_acts_whatever_pane_is_focused() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let before = tab.detail.active_id();
    let mut ctx = bench.ctx().with_focus(PaneId::Activity);
    let _ = tab.on_key(KeyEvent::from(KeyCode::Char('l')), &mut ctx);
    assert_ne!(tab.detail.active_id(), before, "the sub-tab cycled");
    let mut ctx = bench.ctx().with_focus(PaneId::Detail);
    let _ = tab.on_key(KeyEvent::from(KeyCode::Char('f')), &mut ctx);
    assert!(tab.form.is_some(), "the filter opened");
}

/// MOD-115 M2 D5, §8: the location names the active sub-tab (id and title) and the item
/// under the cursor; restoring it on a tab moved since selects the sub-tab, unfolds the
/// item's project and selects the item, reading its detail.
#[tokio::test]
async fn the_location_names_the_sub_tab_and_the_item_and_restore_returns_to_them() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    assert!(tab.detail.select_id(RunsTab::ID));
    let here = tab.location();
    assert_eq!(here.subtab, Some("runs"));
    assert_eq!(here.subtab_title.as_deref(), Some("Runs"));
    assert!(here.cursor.is_some(), "the item under the cursor");

    let mut moved = bench.tab();
    let _ = moved.on_key(KeyEvent::from(KeyCode::Char('j')), &mut bench.ctx());
    assert_ne!(moved.selected, Some(bench.first));
    let project = first_project(&bench);
    moved.folded.push(project);
    let _ = bench.actions();

    assert!(moved.restore(&here, &mut bench.ctx()));
    assert_eq!(moved.selected, Some(bench.first), "back on the item");
    assert_eq!(moved.detail.active_id(), Some(RunsTab::ID), "on Runs");
    assert!(!moved.folded.contains(&project), "its project unfolded");
    let Selection::Item(id) = bench.first else {
        unreachable!("the first row is an item")
    };
    assert!(
        bench
            .actions()
            .iter()
            .any(|action| matches!(action, Action::Store(StoreRequest::Item(read)) if *read == id)),
        "its detail is read"
    );
    assert_eq!(moved.location(), here, "and the location reads the same");
}

/// MOD-115 M2 §8: a project header is a location too.
#[tokio::test]
async fn a_project_header_is_a_location_too() {
    let bench = Bench::new().await;
    let project = first_project(&bench);
    let mut tab = bench.tab();
    tab.selected = Some(Selection::Project(project));
    let here = tab.location();
    let mut moved = bench.tab();
    assert!(moved.restore(&here, &mut bench.ctx()));
    assert_eq!(moved.selected, Some(Selection::Project(project)));
}

/// MOD-115 M2 §8: an item the list no longer holds is read again, as a reveal reads it
/// (the filter goes, the reveal is armed).
#[tokio::test]
async fn restoring_an_item_the_list_does_not_hold_reads_it_again() {
    let bench = Bench::new().await;
    let here = bench.tab().location();
    let mut empty = BacklogTab::new();
    assert!(empty.restore(&here, &mut bench.ctx()));
    assert!(empty.pending_reveal.is_some(), "the reveal is armed");
    assert_eq!(items_reads(&bench.actions()).len(), 1, "one list read");
}

/// MOD-115 M2 §8: an open form keeps the tab where it is, and a location that names no
/// sub-tab or item restores nothing.
#[tokio::test]
async fn restore_moves_nothing_while_a_form_is_open_or_with_nothing_named() {
    let bench = Bench::new().await;
    let here = {
        let mut tab = bench.tab();
        tab.detail.select_id(RunsTab::ID);
        tab.location()
    };
    let mut tab = bench.tab();
    let _ = tab.on_key(KeyEvent::from(KeyCode::Char('j')), &mut bench.ctx());
    let at = tab.selected;
    tab.form = Some(FilterForm::open(&tab.filter, &bench.projects));
    assert!(!tab.restore(&here, &mut bench.ctx()));
    assert_eq!(tab.selected, at);
    assert_ne!(tab.detail.active_id(), Some(RunsTab::ID));

    let mut tab = bench.tab();
    assert!(!tab.restore(&TabLoc::default(), &mut bench.ctx()));
    assert_eq!(tab.selected, Some(bench.first));
}
