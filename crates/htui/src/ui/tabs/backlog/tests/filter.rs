//! The Backlog's filter form: opening, applying, clearing and the reads it sends.

use super::*;

// ---- MOD-13 milestone 1: the filter ------------------------------------------------------

/// MOD-13 D1: `f` opens the form, and while it is open the list's letters and the global
/// ones (`q` would quit) are the form's.
#[tokio::test]
async fn f_opens_the_filter_form_and_it_captures() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    assert_eq!(
        press(&mut tab, &bench, KeyCode::Char('f')),
        Handled::Consumed
    );
    assert!(tab.form.is_some(), "the form is open");
    for code in [
        KeyCode::Char('j'),
        KeyCode::Char('q'),
        KeyCode::Char('3'),
        KeyCode::Tab,
    ] {
        assert_eq!(
            press(&mut tab, &bench, code),
            Handled::Consumed,
            "{code:?} is the form's"
        );
    }
    assert_eq!(
        tab.selected,
        Some(bench.first),
        "the list cursor did not move"
    );
    assert!(bench.actions().is_empty(), "and nothing was read");
}

/// MOD-13 D2: applying sends exactly one `Items`, the same request kind the unfiltered list
/// uses, so the staleness index supersedes it.
#[tokio::test]
async fn applying_the_form_sends_one_filtered_items_read() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    press(&mut tab, &bench, KeyCode::Char('f'));
    for _ in 0..5 {
        press(&mut tab, &bench, KeyCode::Char('l'));
    }
    press(&mut tab, &bench, KeyCode::Char(' '));
    assert!(bench.actions().is_empty(), "nothing is read before `Enter`");
    assert_eq!(press(&mut tab, &bench, KeyCode::Enter), Handled::Consumed);

    let actions = bench.actions();
    assert_eq!(
        items_reads(&actions),
        [(
            ItemFilter {
                statuses: Some(vec![Status::Done]),
                ..ItemFilter::default()
            },
            false
        )],
        "{actions:?}"
    );
    assert_eq!(actions.len(), 1, "and nothing else: {actions:?}");
    assert!(tab.form.is_none(), "the form closed");
    assert_eq!(tab.filter, done_only());
}

/// MOD-13 D3: a refused tag list goes to the status line and the form stays open to fix it.
#[tokio::test]
async fn a_refused_tag_list_keeps_the_form_open_and_reports_it() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    press(&mut tab, &bench, KeyCode::Char('f'));
    press(&mut tab, &bench, KeyCode::Down);
    press(&mut tab, &bench, KeyCode::Down);
    tab.on_key(
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT),
        &mut bench.ctx(),
    );
    press(&mut tab, &bench, KeyCode::Enter);

    let actions = bench.actions();
    assert!(
        matches!(actions.as_slice(), [Action::Error(_)]),
        "one refusal and no read: {actions:?}"
    );
    assert!(tab.form.is_some(), "the form stays open");
    assert!(tab.filter.is_empty(), "and the filter is untouched");
}

/// MOD-13 D5: `F` clears an active filter and reads the whole scope again.
#[tokio::test]
async fn shift_f_clears_the_filter_and_re_reads() {
    let bench = Bench::new().await;
    let mut tab = BacklogTab {
        filter: done_only(),
        ..bench.tab()
    };
    assert_eq!(
        press(&mut tab, &bench, KeyCode::Char('F')),
        Handled::Consumed
    );
    assert!(tab.filter.is_empty());
    let actions = bench.actions();
    assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);
    assert_eq!(actions.len(), 1, "{actions:?}");
}

#[tokio::test]
async fn shift_f_without_a_filter_sends_nothing() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    assert_eq!(
        press(&mut tab, &bench, KeyCode::Char('F')),
        Handled::Consumed
    );
    assert!(bench.actions().is_empty());
}

/// MOD-13 D2: a tab activation or refresh reads with the active filter, not the whole scope.
#[tokio::test]
async fn wants_requests_carries_the_active_filter() {
    let bench = Bench::new().await;
    let tab = BacklogTab {
        filter: BacklogFilter {
            tags: vec!["rust".to_owned()],
            ready_here: true,
            ..BacklogFilter::default()
        },
        ..bench.tab()
    };
    let requests: Vec<Action> = tab
        .wants_requests(&bench.scope)
        .into_iter()
        .map(Action::Store)
        .collect();
    assert_eq!(
        items_reads(&requests),
        [(
            ItemFilter {
                tags: Some(vec!["rust".to_owned()]),
                ..ItemFilter::default()
            },
            true
        )]
    );
    assert_eq!(requests.len(), 1);
    assert!(
        BacklogTab::new()
            .wants_requests(&bench.scope)
            .iter()
            .all(|request| matches!(
                request,
                StoreRequest::Items { filter, ready_here: false, .. }
                    if *filter == ItemFilter::default()
            )),
        "no filter is the pre-MOD-13 read (D5)"
    );
}

/// MOD-13 blueprint §6: a filtered reply must not decide a reveal sent before it.
#[tokio::test]
async fn applying_a_filter_disarms_a_pending_reveal() {
    let bench = Bench::new().await;
    let mut tab = BacklogTab::new();
    let target = RevealTarget::Item {
        id: ItemId::default(),
        key: "GONE-1".to_owned(),
    };
    assert!(tab.reveal(&target, &mut bench.ctx()));
    assert!(tab.pending_reveal.is_some());
    let _ = bench.actions();

    press(&mut tab, &bench, KeyCode::Char('f'));
    press(&mut tab, &bench, KeyCode::Char(' '));
    press(&mut tab, &bench, KeyCode::Enter);
    assert!(tab.pending_reveal.is_none(), "the filter disarmed it");
    let _ = bench.actions();

    tab.on_reply(&StoreReply::Items(Vec::new()), &mut bench.ctx());
    assert!(
        !bench
            .actions()
            .iter()
            .any(|action| matches!(action, Action::Error(_))),
        "the filtered list reports nothing"
    );
}

/// MOD-13 review N5: applying no filter right after a reveal miss keeps the reveal armed. The
/// unfiltered reply it waits for can still decide it.
#[tokio::test]
async fn applying_no_filter_keeps_a_pending_reveal() {
    let bench = Bench::new().await;
    let mut tab = BacklogTab::new();
    let target = RevealTarget::Item {
        id: htui_core::fixtures::ids::HTUI_ANA_2,
        key: "ANA-2".to_owned(),
    };
    assert!(tab.reveal(&target, &mut bench.ctx()));
    press(&mut tab, &bench, KeyCode::Char('f'));
    press(&mut tab, &bench, KeyCode::Enter);
    assert!(tab.pending_reveal.is_some(), "still armed");

    tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
    assert_eq!(
        tab.selected,
        Some(Selection::Item(htui_core::fixtures::ids::HTUI_ANA_2)),
        "the reply decided it"
    );
}

/// MOD-13 D6: a scope change keeps the filter minus the projects the new scope lacks, and
/// closes the form, whose project list belonged to the old scope.
#[tokio::test]
async fn a_scope_change_drops_the_projects_it_lacks_and_closes_the_form() {
    use htui_core::fixtures::ids::{PROJECT_AGY, PROJECT_HTUI};
    let bench = Bench::new().await;
    let htui_only = Scope {
        workspace_id: bench.scope.workspace_id,
        project_ids: vec![PROJECT_HTUI],
    };
    let mut tab = BacklogTab {
        filter: BacklogFilter {
            projects: vec![PROJECT_HTUI, PROJECT_AGY],
            ..done_only()
        },
        ..bench.tab()
    };
    press(&mut tab, &bench, KeyCode::Char('f'));
    tab.on_scope_change(&htui_only);
    assert_eq!(tab.filter.projects, [PROJECT_HTUI]);
    assert_eq!(tab.filter.statuses, done_only().statuses, "status is kept");
    assert!(tab.form.is_none(), "the form closed");

    let mut agy = BacklogTab {
        filter: BacklogFilter {
            projects: vec![PROJECT_AGY],
            ..BacklogFilter::default()
        },
        ..bench.tab()
    };
    agy.on_scope_change(&htui_only);
    assert!(
        agy.filter.is_empty(),
        "a filter naming only a gone project is no filter, not an empty list"
    );
}

/// MOD-13 review M1: the title names the filter the rows were read under. A refused read
/// leaves the old rows under the old title, and the filter rolls back to it, so the next `f`
/// opens on what is on screen.
#[tokio::test]
async fn a_refused_filtered_read_keeps_the_title_and_rolls_the_filter_back() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    tab.apply(done_only(), &bench.ctx());
    let pending = drawn(&tab, &bench);
    assert!(
        pending.contains("Backlog (11) ") && !pending.contains("status: done"),
        "the reply has not arrived, so the title is the whole list's:\n{pending}"
    );

    tab.on_reply(
        &StoreReply::Failed {
            request: ITEMS_READ,
            message: "the server went away".to_owned(),
        },
        &mut bench.ctx(),
    );
    let refused = drawn(&tab, &bench);
    assert!(
        refused.contains("Backlog (11) ") && !refused.contains("status: done"),
        "{refused}"
    );
    assert_eq!(tab.filter, tab.shown, "the filter is what is shown");
    assert!(tab.filter.is_empty());
}

/// MOD-13 review M1: the reply to a filtered read brings its filter into the list (MOD-115 M4
/// E4: the summary band).
#[tokio::test]
async fn a_filtered_reply_names_its_filter_in_the_title() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    tab.apply(done_only(), &bench.ctx());
    tab.on_reply(
        &StoreReply::Items(with_status(&bench, Status::Done)),
        &mut bench.ctx(),
    );
    assert_eq!(tab.shown, done_only());
    let frame = drawn(&tab, &bench);
    // MOD-115 M4 E4: the count in the title, the filter in the summary band under it.
    assert!(frame.contains("Backlog (2) "), "{frame}");
    assert!(frame.contains(" / status: done "), "{frame}");
}
