//! The Backlog's item writes: the item form, edits, mints, divergence and `$EDITOR` outcomes.

use super::*;

// ---- MOD-13 milestone 2: new and edit ----------------------------------------------------

/// Blueprint E4: a terminal's `N` may or may not carry SHIFT; both read the form.
#[tokio::test]
async fn n_reads_the_item_form_on_the_selected_project_with_and_without_shift() {
    use htui_core::fixtures::ids::PROJECT_HTUI;
    let bench = Bench::new().await;
    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
        let mut tab = bench.tab();
        assert_eq!(
            tab.on_key(
                KeyEvent::new(KeyCode::Char('N'), modifiers),
                &mut bench.ctx()
            ),
            Handled::Consumed
        );
        let requests = sent(&bench);
        assert!(
            matches!(
                requests.as_slice(),
                [StoreRequest::ItemForm { project, item: None }] if *project == PROJECT_HTUI
            ),
            "{modifiers:?}: {requests:?}"
        );
    }
}

/// D7: `e` reads the selected item's form; with nothing selected it sends nothing.
#[tokio::test]
async fn e_reads_the_selected_item_and_nothing_without_a_selection() {
    use htui_core::fixtures::ids::PROJECT_HTUI;
    let bench = Bench::new().await;
    let Selection::Item(first) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    assert_eq!(
        press(&mut tab, &bench, KeyCode::Char('e')),
        Handled::Consumed
    );
    let requests = sent(&bench);
    assert!(
        matches!(
            requests.as_slice(),
            [StoreRequest::ItemForm { project, item: Some(id) }]
                if *project == PROJECT_HTUI && *id == first
        ),
        "{requests:?}"
    );

    let mut empty = BacklogTab::new();
    assert_eq!(
        press(&mut empty, &bench, KeyCode::Char('e')),
        Handled::Consumed
    );
    assert!(sent(&bench).is_empty());
    assert!(empty.opening.is_none());
}

/// D8: the reply opens the form in the detail pane, and it captures the list's and the
/// shell's letters.
#[tokio::test]
async fn the_item_form_reply_opens_the_form_and_it_captures() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    let before = drawn(&tab, &bench);
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
    for code in [KeyCode::Char('j'), KeyCode::Char('q'), KeyCode::Char('3')] {
        assert_eq!(press(&mut tab, &bench, code), Handled::Consumed, "{code:?}");
    }
    assert_eq!(tab.selected, Some(bench.first), "the cursor did not move");
    assert!(sent(&bench).is_empty(), "nothing was read");
    let frame = drawn(&tab, &bench);
    assert!(frame.contains(" New item \u{b7} htui "), "{frame}");
    assert_ne!(frame, before);

    press(&mut tab, &bench, KeyCode::Esc);
    assert!(tab.item_form.is_none(), "Esc closes it");
    assert_eq!(
        drawn(&tab, &bench),
        before,
        "and the pane is the detail again"
    );
}

/// Blueprint E8: a form opens only on the answer to the read `N`/`e` sent.
#[tokio::test]
async fn an_item_form_reply_nobody_asked_for_opens_nothing() {
    use htui_core::fixtures::ids::{PROJECT_AGY, PROJECT_HTUI};
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    let unasked = served(
        &store,
        &StoreRequest::ItemForm {
            project: PROJECT_HTUI,
            item: None,
        },
    )
    .await;
    tab.on_reply(&unasked, &mut bench.ctx());
    assert!(tab.item_form.is_none(), "no `N`, no form");

    press(&mut tab, &bench, KeyCode::Char('N'));
    let _ = sent(&bench);
    let other = served(
        &store,
        &StoreRequest::ItemForm {
            project: PROJECT_AGY,
            item: None,
        },
    )
    .await;
    tab.on_reply(&other, &mut bench.ctx());
    assert!(
        tab.item_form.is_none(),
        "another project's answer opens nothing"
    );
    assert!(errors(&bench.actions()).is_empty());
}

/// MOD-22 review M-1: a paste reaches the open item form's focused field.
#[tokio::test]
async fn a_paste_goes_to_the_open_item_form() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
    assert_eq!(tab.on_paste("Pasted", &mut bench.ctx()), Handled::Consumed);
    let request = saved(&mut tab, &bench);
    assert!(
        matches!(&request, StoreRequest::MintItem { spec, .. } if spec.title == "Pasted"),
        "{request:?}"
    );
}

#[tokio::test]
async fn ctrl_s_on_a_blank_title_refuses_in_the_form_and_sends_nothing() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
    assert_eq!(tab.on_key(ctrl('s'), &mut bench.ctx()), Handled::Consumed);
    let actions = bench.actions();
    assert!(
        actions.is_empty(),
        "nothing sent, nothing reported: {actions:?}"
    );
    assert_eq!(
        notice(&tab),
        Some(htui_core::model::SpecError::BlankTitle.to_string())
    );
}

#[tokio::test]
async fn an_unchanged_edit_says_nothing_to_save_and_sends_nothing() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
    assert_eq!(tab.on_key(ctrl('s'), &mut bench.ctx()), Handled::Consumed);
    assert!(bench.actions().is_empty());
    assert_eq!(
        notice(&tab).as_deref(),
        Some(htui_core::model::item_spec::NOTHING_TO_SAVE)
    );
}

/// D5, D6: only the title, at the version the form's read carried.
#[tokio::test]
async fn a_changed_title_sends_edit_item_with_only_the_title_at_the_read_version() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let Selection::Item(first) = bench.first else {
        panic!("the first row is an item")
    };
    let title = format!(
        "{} v2",
        store
            .item(first)
            .await
            .expect("read")
            .expect("the item")
            .title
    );
    let mut tab = bench.tab();
    let request = an_edit_saved(&mut tab, &bench, &store).await;
    assert!(
        matches!(
            &request,
            StoreRequest::EditItem {
                id,
                expected_version: 1,
                changes,
                reason: htui_core::model::EditReason::Edited,
            } if *id == first && *changes == htui_core::model::SpecChanges {
                    title: Some(title.clone()),
                    ..htui_core::model::SpecChanges::default()
                }
        ),
        "{request:?}"
    );
}

/// D9, A5: the form closes, the item's seven reads go out again and the list re-reads,
/// with the cursor where it was.
#[tokio::test]
async fn an_applied_edit_closes_the_form_and_re_reads_the_item_keeping_the_selection() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let Selection::Item(first) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    let request = an_edit_saved(&mut tab, &bench, &store).await;
    let reply = served(&store, &request).await;
    assert!(matches!(reply, StoreReply::ItemWritten { .. }), "{reply:?}");
    tab.on_reply(&reply, &mut bench.ctx());
    assert!(tab.item_form.is_none(), "the form closed");
    assert_eq!(tab.selected, Some(bench.first));

    let requests = sent(&bench);
    let names: Vec<&str> = requests.iter().map(StoreRequest::name).collect();
    assert_eq!(requests.len(), 8, "{names:?}");
    let per_item = requests
        .iter()
        .filter(|request| match request {
            StoreRequest::Item(id)
            | StoreRequest::Runs(id)
            | StoreRequest::Documents(id)
            | StoreRequest::Notes(id)
            | StoreRequest::ItemRequirements(id)
            | StoreRequest::Links { id, .. }
            | StoreRequest::PromptPreview { item: id, .. } => *id == first,
            _ => false,
        })
        .count();
    assert_eq!(per_item, 7, "{names:?}");
    assert_eq!(
        items_reads(&requests.into_iter().map(Action::Store).collect::<Vec<_>>()).len(),
        1
    );
}

/// Review M1: an `e` whose reply lands after the cursor moved opens nothing, so the form
/// cannot edit an item the header no longer names.
#[tokio::test]
async fn an_edit_reply_after_the_cursor_moved_opens_nothing() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    press(&mut tab, &bench, KeyCode::Char('e'));
    let requests = sent(&bench);
    let [read @ StoreRequest::ItemForm { .. }] = requests.as_slice() else {
        panic!("one form read: {requests:?}")
    };
    press(&mut tab, &bench, KeyCode::Char('j'));
    assert_ne!(tab.selected, Some(bench.first), "the cursor moved");
    let _ = sent(&bench);

    let reply = served(&store, read).await;
    tab.on_reply(&reply, &mut bench.ctx());
    assert!(tab.item_form.is_none(), "no form for the old item");
    assert!(tab.opening.is_none(), "the opening is spent");
    assert!(bench.actions().is_empty(), "nothing read, nothing reported");
}

/// Review M1: an applied edit re-reads the item only while it is still selected; once a list
/// moved the cursor off it, the list re-read alone follows, so the Body cannot show the
/// edited item under the new selection's header.
#[tokio::test]
async fn an_applied_edit_after_the_selection_moved_reads_only_the_list() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let Selection::Item(first) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    let request = an_edit_saved(&mut tab, &bench, &store).await;
    let others: Vec<ItemSummary> = bench
        .items
        .iter()
        .filter(|item| item.id != first)
        .cloned()
        .collect();
    tab.on_reply(&StoreReply::Items(others), &mut bench.ctx());
    assert_ne!(tab.selected, Some(bench.first), "the list moved the cursor");
    let _ = sent(&bench);

    let reply = served(&store, &request).await;
    assert!(matches!(reply, StoreReply::ItemWritten { .. }), "{reply:?}");
    tab.on_reply(&reply, &mut bench.ctx());
    assert!(tab.item_form.is_none(), "the form closed");
    let requests = sent(&bench);
    let names: Vec<&str> = requests.iter().map(StoreRequest::name).collect();
    assert_eq!(per_item_reads(&requests, first), 0, "{names:?}");
    assert_eq!(
        items_reads(&requests.into_iter().map(Action::Store).collect::<Vec<_>>()).len(),
        1,
        "{names:?}"
    );
}

/// D9, A5: a minted item is never in the loaded rows, so the reveal clears the filter and
/// the next list selects it.
#[tokio::test]
async fn an_applied_mint_closes_the_form_clears_the_filter_and_reveals_it() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = BacklogTab {
        filter: done_only(),
        ..bench.tab()
    };
    let request = a_mint_saved(&mut tab, &bench, &store).await;
    let reply = served(&store, &request).await;
    let StoreReply::ItemWritten { item: minted, .. } = reply else {
        panic!("a mint answers ItemWritten, not {reply:?}")
    };
    tab.on_reply(&reply, &mut bench.ctx());
    assert!(tab.item_form.is_none(), "the form closed");
    assert!(tab.filter.is_empty(), "the filter is cleared");
    assert_eq!(
        tab.pending_reveal,
        Some(PendingReveal {
            id: minted,
            key: "ANA-3".to_owned(),
            focus: None,
        })
    );
    let actions = bench.actions();
    assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);
    assert_eq!(actions.len(), 1, "{actions:?}");

    let list = served(&store, &tab.filter.to_request(&bench.scope)).await;
    tab.on_reply(&list, &mut bench.ctx());
    assert_eq!(tab.selected, Some(Selection::Item(minted)));
    assert!(errors(&bench.actions()).is_empty());
}

// ---- MOD-13 milestone 3: the divergence view -------------------------------------------

/// Milestone 3 D3, D5: a stale edit opens the view; `Esc` returns to the form with its text
/// and its token, and the next save is the same stale one.
#[tokio::test]
async fn a_diverged_edit_opens_the_view_and_esc_keeps_the_text_and_token() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    in_the_view(&mut tab, &bench, &store).await;
    assert!(errors(&bench.actions()).is_empty());

    assert_eq!(press(&mut tab, &bench, KeyCode::Esc), Handled::Consumed);
    assert!(view(&tab).is_none(), "the view closed");
    assert!(tab.item_form.is_some(), "the form stays");
    assert_eq!(
        notice(&tab),
        Some(divergence::still_behind(2, Some("Ctrl+s")))
    );

    let again = saved(&mut tab, &bench);
    assert!(
        matches!(
            &again,
            StoreRequest::EditItem {
                expected_version: 1,
                changes,
                reason: htui_core::model::EditReason::Edited,
                ..
            } if changes.title.as_deref().is_some_and(|title| title.ends_with(" mine"))
        ),
        "the token never moved and the text is kept: {again:?}"
    );
}

/// D1: a divergence opens the view only on the form whose save it answers.
#[tokio::test]
async fn a_divergence_for_another_item_or_token_is_dropped() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    let Selection::Item(first) = bench.first else {
        panic!("the first row is an item")
    };
    open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
    store
        .update_item(
            first,
            1,
            htui_core::model::ItemPatch {
                title: Some("Theirs".to_owned()),
                author_id: htui_core::fixtures::ids::USER,
                reason: "elsewhere".to_owned(),
                ..htui_core::model::ItemPatch::default()
            },
        )
        .await
        .expect("their edit");
    type_into(&mut tab, &bench, " mine");
    let request = saved(&mut tab, &bench);
    let StoreReply::ItemDiverged(real) = served(&store, &request).await else {
        panic!("a divergence")
    };
    let mut late = real.as_ref().clone();
    late.ancestor.version = 7;
    let mut other = real.as_ref().clone();
    other.head.id = ItemId::new();
    for stray in [late, other] {
        tab.on_reply(&StoreReply::ItemDiverged(Box::new(stray)), &mut bench.ctx());
        assert!(view(&tab).is_none());
        assert_eq!(
            tab.item_form.as_ref().and_then(ItemForm::busy),
            Some(Busy::Editing)
        );
    }
    tab.on_reply(&StoreReply::ItemDiverged(real), &mut bench.ctx());
    assert!(view(&tab).is_some(), "the real one still opens it");
}

/// D5, D6: `m` rebases the form on the head, and Ctrl+S lands a resolution there.
#[tokio::test]
async fn m_rebases_and_ctrl_s_sends_a_resolution_at_the_head() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    in_the_view(&mut tab, &bench, &store).await;
    assert_eq!(
        press(&mut tab, &bench, KeyCode::Char('m')),
        Handled::Consumed
    );
    assert!(view(&tab).is_none());
    assert_eq!(
        notice(&tab),
        Some(divergence::rebased_on(2, Some("Ctrl+s")))
    );
    let request = saved(&mut tab, &bench);
    assert!(
        matches!(
            &request,
            StoreRequest::EditItem {
                expected_version: 2,
                changes,
                reason: htui_core::model::EditReason::DivergenceResolution,
                ..
            } if changes.title.as_deref().is_some_and(|title| title.ends_with(" mine"))
                && *changes == htui_core::model::SpecChanges {
                    title: changes.title.clone(),
                    ..htui_core::model::SpecChanges::default()
                }
        ),
        "{request:?}"
    );
    let reply = served(&store, &request).await;
    assert!(
        matches!(
            &reply,
            StoreReply::ItemWritten {
                outcome: ItemWrite::Edited { version: 3, .. },
                ..
            }
        ),
        "{reply:?}"
    );
    tab.on_reply(&reply, &mut bench.ctx());
    assert!(tab.item_form.is_none(), "the form closed");
}

/// D8: a resolution that is itself stale re-opens the view, its ancestor the rebased head.
#[tokio::test]
async fn a_second_divergence_after_a_rebase_reopens_the_view() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    let (first, _) = in_the_view(&mut tab, &bench, &store).await;
    press(&mut tab, &bench, KeyCode::Char('m'));
    store
        .update_item(
            first,
            2,
            htui_core::model::ItemPatch {
                priority: Some(42),
                author_id: htui_core::fixtures::ids::USER,
                reason: "elsewhere again".to_owned(),
                ..htui_core::model::ItemPatch::default()
            },
        )
        .await
        .expect("their second edit");
    let request = saved(&mut tab, &bench);
    let reply = served(&store, &request).await;
    let StoreReply::ItemDiverged(divergence) = &reply else {
        panic!("a second divergence: {reply:?}")
    };
    assert_eq!(divergence.ancestor.version, 2);
    assert_eq!(
        tab.item_form.as_ref().and_then(ItemForm::token),
        Some(2),
        "the reply answers this token"
    );
    tab.on_reply(&reply, &mut bench.ctx());
    let open = view(&tab).expect("the view re-opened");
    assert_eq!((open.ancestor_version(), open.head_version()), (2, 3));
}

/// D4: the view draws over the whole tab area, the list pane included.
#[tokio::test]
async fn the_view_takes_the_whole_tab_area() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    let (_, key) = in_the_view(&mut tab, &bench, &store).await;
    let frame = drawn(&tab, &bench);
    assert!(
        frame.contains(&divergence::header(&key, 1, 2)),
        "the header in\n{frame}"
    );
    let other = &bench.items[1].key;
    assert_ne!(*other, key);
    assert!(!frame.contains(other.as_str()), "no list row in\n{frame}");
    let slug = &bench.projects[0].slug;
    assert!(
        !frame.contains(&format!("{slug} ")),
        "no list header in\n{frame}"
    );

    press(&mut tab, &bench, KeyCode::Esc);
    let frame = drawn(&tab, &bench);
    assert!(
        frame.contains(&format!(" Edit {key} (v1) ")),
        "the form again in\n{frame}"
    );
    assert!(frame.contains(slug.as_str()), "the list again in\n{frame}");
}

#[tokio::test]
async fn a_scope_change_closes_the_open_view() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    in_the_view(&mut tab, &bench, &store).await;
    tab.on_scope_change(&bench.scope);
    assert!(tab.item_form.is_none());
}

/// A reveal would drop the half-resolved edit, as it would the form.
#[tokio::test]
async fn a_reveal_while_the_view_is_open_asks_to_close_it_first() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    in_the_view(&mut tab, &bench, &store).await;
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
    assert!(view(&tab).is_some(), "the view stays");
    assert_eq!(tab.selected, Some(bench.first));
}

/// §10.1, §10.2: a mint `Failed` that may follow a COMMIT keeps the text, hedges and re-reads
/// the whole list, so the filter cannot hide the item the notice asks to look for.
#[tokio::test]
async fn a_store_failed_mint_keeps_the_text_hedges_and_re_reads_unfiltered() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = BacklogTab {
        filter: done_only(),
        ..bench.tab()
    };
    let _ = a_mint_saved(&mut tab, &bench, &store).await;
    let why = htui_core::store::StoreError::Backend("the server went away".to_owned()).to_string();
    tab.on_reply(
        &StoreReply::Failed {
            request: item_writes::MINT_NAME,
            message: why.clone(),
        },
        &mut bench.ctx(),
    );
    let form = tab.item_form.as_ref().expect("the form stays");
    assert_eq!(form.busy(), None);
    assert_eq!(
        notice(&tab),
        Some(mint_may_have_landed(&why, Some("Ctrl+s")))
    );
    assert!(tab.filter.is_empty(), "the filter is cleared");
    let actions = bench.actions();
    assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);
    assert_eq!(
        actions.len(),
        1,
        "one read, no error of the tab's own: {actions:?}"
    );

    let retry = saved(&mut tab, &bench);
    assert!(
        matches!(&retry, StoreRequest::MintItem { spec, .. } if spec.title == "Fresh item"),
        "the text is kept: {retry:?}"
    );
}

/// §10.1: a refusal given before the insert wrote nothing; it is said plainly, nothing is
/// re-read and the filter stays.
#[tokio::test]
async fn a_refused_mint_keeps_the_text_and_says_why_without_hedging_or_re_reading() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = BacklogTab {
        filter: done_only(),
        ..bench.tab()
    };
    let _ = a_mint_saved(&mut tab, &bench, &store).await;
    let why = htui_core::store::StoreError::Constraint("that kind is gone".to_owned()).to_string();
    tab.on_reply(
        &StoreReply::Failed {
            request: item_writes::MINT_NAME,
            message: why.clone(),
        },
        &mut bench.ctx(),
    );
    assert_eq!(notice(&tab), Some(why));
    assert_eq!(tab.item_form.as_ref().and_then(ItemForm::busy), None);
    assert_eq!(tab.filter, done_only(), "the filter stays");
    assert!(
        bench.actions().is_empty(),
        "nothing re-read, nothing reported"
    );
}

/// Review L4: a store-failed mint that lands after a scope change closed its form keeps its
/// hedge, on the status line, as the one error the tab raises of its own; nothing is re-read.
#[tokio::test]
async fn a_store_failed_mint_with_its_form_gone_is_hedged_on_the_status_line() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    let _ = a_mint_saved(&mut tab, &bench, &store).await;
    tab.on_scope_change(&bench.scope);
    let why = htui_core::store::StoreError::Backend("the server went away".to_owned()).to_string();
    tab.on_reply(
        &StoreReply::Failed {
            request: item_writes::MINT_NAME,
            message: why.clone(),
        },
        &mut bench.ctx(),
    );
    let actions = bench.actions();
    let [Action::Error(sentence)] = actions.as_slice() else {
        panic!("one status-line hedge, nothing re-read: {actions:?}");
    };
    assert!(sentence.starts_with(&why), "{sentence:?}");
    assert!(sentence.contains("may have been written"), "{sentence:?}");
    // Round 2: nothing is re-read and no form is open, so neither is promised.
    assert!(!sentence.contains("re-read"), "{sentence:?}");
    assert!(!sentence.contains("Ctrl+s"), "{sentence:?}");
    assert_eq!(*sentence, mint_may_have_landed_in_the_old_scope(&why));
    assert!(tab.item_form.is_none());
}

/// Review L4: a refused mint wrote nothing, and `App` already reports its `Failed`, so with
/// its form gone the tab adds nothing.
#[tokio::test]
async fn a_refused_mint_with_its_form_gone_adds_nothing() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    let _ = a_mint_saved(&mut tab, &bench, &store).await;
    tab.on_scope_change(&bench.scope);
    let why = htui_core::store::StoreError::Constraint("that kind is gone".to_owned()).to_string();
    tab.on_reply(
        &StoreReply::Failed {
            request: item_writes::MINT_NAME,
            message: why,
        },
        &mut bench.ctx(),
    );
    assert!(bench.actions().is_empty());
}

/// D2, A8: a refused form read opens nothing, and the tab adds no error to the one `App`
/// already reports.
#[tokio::test]
async fn a_failed_item_form_read_opens_nothing_and_adds_no_error() {
    use htui_core::fixtures::ids::PROJECT_HTUI;
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    press(&mut tab, &bench, KeyCode::Char('N'));
    let _ = sent(&bench);
    tab.on_reply(
        &StoreReply::Failed {
            request: item_writes::FORM_NAME,
            message: format!("store unreachable: {}", htui_store::DATABASE_UNREACHABLE),
        },
        &mut bench.ctx(),
    );
    assert!(tab.item_form.is_none());
    assert!(tab.opening.is_none(), "the opening is forgotten");
    assert!(
        bench.actions().is_empty(),
        "no Action::Error of the tab's own"
    );

    let late = served(
        &store,
        &StoreRequest::ItemForm {
            project: PROJECT_HTUI,
            item: None,
        },
    )
    .await;
    tab.on_reply(&late, &mut bench.ctx());
    assert!(tab.item_form.is_none(), "a late answer opens nothing");
}

/// A7, E8: a scope change closes the form and forgets a read in flight.
#[tokio::test]
async fn a_scope_change_closes_the_item_form_and_forgets_the_opening() {
    use htui_core::fixtures::ids::PROJECT_HTUI;
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
    tab.on_scope_change(&bench.scope);
    assert!(tab.item_form.is_none(), "the form closed");

    let mut tab = bench.tab();
    press(&mut tab, &bench, KeyCode::Char('N'));
    let _ = sent(&bench);
    tab.on_scope_change(&bench.scope);
    assert!(tab.opening.is_none());
    let late = served(
        &store,
        &StoreRequest::ItemForm {
            project: PROJECT_HTUI,
            item: None,
        },
    )
    .await;
    tab.on_reply(&late, &mut bench.ctx());
    assert!(
        tab.item_form.is_none(),
        "the old scope's answer opens nothing"
    );
}

/// Review M2, E7: the project picker's reload re-targets the open form; the typed text stays.
#[tokio::test]
async fn the_project_picker_reloads_and_the_reply_retargets_keeping_the_text() {
    use htui_core::fixtures::ids::PROJECT_AGY;
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    a_project_moved(&mut tab, &bench, &store).await;

    let reply = served(
        &store,
        &StoreRequest::ItemForm {
            project: PROJECT_AGY,
            item: None,
        },
    )
    .await;
    tab.on_reply(&reply, &mut bench.ctx());
    let form = tab.item_form.as_ref().expect("the form stays");
    assert_eq!(form.busy(), None);
    assert_eq!(form.project(), PROJECT_AGY);
    assert!(bench.actions().is_empty(), "nothing else is read");

    let request = saved(&mut tab, &bench);
    assert!(
        matches!(
            &request,
            StoreRequest::MintItem { project, spec }
                if *project == PROJECT_AGY && spec.title == "Fresh item"
        ),
        "the text is kept: {request:?}"
    );
}

/// Review M2, E7: a refused reload settles the form with the reason; `Esc` then closes it.
#[tokio::test]
async fn a_refused_reload_settles_the_form_with_a_notice_and_esc_closes_it() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    a_project_moved(&mut tab, &bench, &store).await;

    let why = format!("store unreachable: {}", htui_store::DATABASE_UNREACHABLE);
    tab.on_reply(
        &StoreReply::Failed {
            request: item_writes::FORM_NAME,
            message: why.clone(),
        },
        &mut bench.ctx(),
    );
    let form = tab.item_form.as_ref().expect("the form stays");
    assert_eq!(form.busy(), None);
    assert_eq!(notice(&tab), Some(why));
    assert!(
        bench.actions().is_empty(),
        "no Action::Error of the tab's own"
    );

    assert_eq!(press(&mut tab, &bench, KeyCode::Esc), Handled::Consumed);
    assert!(tab.item_form.is_none(), "Esc closes it");
}

/// A reveal would drop the half-typed item, as it would a half-edited filter.
#[tokio::test]
async fn a_reveal_while_the_item_form_is_open_asks_to_close_it_first() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
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
    assert!(tab.item_form.is_some());
    assert_eq!(tab.selected, Some(bench.first));
}

/// MOD-52: `ctrl-c` quits from inside the item form, as from every text field.
#[tokio::test]
async fn ctrl_c_passes_through_the_open_item_form() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
    assert_eq!(tab.on_key(ctrl('c'), &mut bench.ctx()), Handled::Pass);
    assert!(tab.item_form.is_some());
}

// ---- MOD-13 milestone 4: the $EDITOR round-trip ------------------------------------------

/// D3: Ctrl+E on the body reaches the loop as an `EditExternally`, and nothing is read.
#[tokio::test]
async fn ctrl_e_on_the_body_emits_an_external_edit_for_the_loop() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let Selection::Item(first) = bench.first else {
        panic!("the first row is an item")
    };
    let item = store.item(first).await.expect("read").expect("the item");
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
    to_body(&mut tab, &bench);
    assert_eq!(tab.on_key(ctrl('e'), &mut bench.ctx()), Handled::Consumed);
    let actions = bench.actions();
    assert!(
        matches!(
            actions.as_slice(),
            [Action::EditExternally(edit)]
                if edit.stem == format!("{}-body", item.key) && edit.text == item.body
        ),
        "{actions:?}"
    );
}

/// D1: on a one-line field Ctrl+E is consumed, so it never reaches a global binding.
#[tokio::test]
async fn ctrl_e_on_the_title_is_consumed_and_emits_nothing() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
    assert_eq!(tab.on_key(ctrl('e'), &mut bench.ctx()), Handled::Consumed);
    assert!(bench.actions().is_empty());
}

/// D4: the outcome reaches the open form, and Ctrl+S sends the body alone at the read version.
#[tokio::test]
async fn the_outcome_reaches_the_open_form_and_ctrl_s_saves_the_body() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
    to_body(&mut tab, &bench);
    assert_eq!(tab.on_key(ctrl('e'), &mut bench.ctx()), Handled::Consumed);
    assert_eq!(bench.actions().len(), 1, "the handoff");
    tab.on_external_edit(
        ExternalEditOutcome::Edited("New body.".to_owned()),
        &mut bench.ctx(),
    );
    assert_eq!(
        notice(&tab).as_deref(),
        Some(crate::editor::edited(Some("Ctrl+s")).as_str())
    );
    let request = saved(&mut tab, &bench);
    assert!(
        matches!(
            &request,
            StoreRequest::EditItem {
                expected_version: 1,
                changes,
                reason: htui_core::model::EditReason::Edited,
                ..
            } if *changes == htui_core::model::SpecChanges {
                    body: Some("New body.".to_owned()),
                    ..htui_core::model::SpecChanges::default()
                }
        ),
        "{request:?}"
    );
}

/// D4, milestone 5 D7: with no form open the outcome reaches the active sub-tab; Body asked
/// for nothing and drops it.
#[tokio::test]
async fn an_outcome_with_no_form_open_is_dropped() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    tab.on_external_edit(
        ExternalEditOutcome::Edited("x".to_owned()),
        &mut bench.ctx(),
    );
    assert!(tab.item_form.is_none());
    assert!(bench.actions().is_empty());
}

// ---- MOD-13 milestone 5: $EDITOR outcomes to the detail sub-tab --------------------------

/// Milestone 5 D7: with no item form, the outcome goes to the active sub-tab.
#[tokio::test]
async fn an_outcome_with_no_item_form_reaches_the_active_sub_tab() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let probe = OutcomeProbe::default();
    let seen = Rc::clone(&probe.seen);
    tab.detail = only(probe);
    tab.on_external_edit(
        ExternalEditOutcome::Edited("x".to_owned()),
        &mut bench.ctx(),
    );
    assert_eq!(
        *seen.borrow(),
        [ExternalEditOutcome::Edited("x".to_owned())]
    );
    assert!(bench.actions().is_empty());
}

/// Milestone 5 D7: an open item form takes the outcome; the sub-tab sees nothing.
#[tokio::test]
async fn an_open_item_form_takes_the_outcome_not_the_sub_tab() {
    let bench = Bench::new().await;
    let store = MemStore::demo();
    let mut tab = bench.tab();
    open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
    let probe = OutcomeProbe::default();
    let seen = Rc::clone(&probe.seen);
    tab.detail = only(probe);
    to_body(&mut tab, &bench);
    assert_eq!(tab.on_key(ctrl('e'), &mut bench.ctx()), Handled::Consumed);
    tab.on_external_edit(
        ExternalEditOutcome::Edited("New body.".to_owned()),
        &mut bench.ctx(),
    );
    assert_eq!(
        notice(&tab).as_deref(),
        Some(crate::editor::edited(Some("Ctrl+s")).as_str())
    );
    assert!(seen.borrow().is_empty());
}

/// Milestone 5 E4: a compose read answered while a form captures above the detail opens
/// nothing; with the form closed, the same reply reaches the sub-tab.
#[tokio::test]
async fn a_compose_read_answered_under_an_open_form_opens_nothing() {
    let bench = Bench::new().await;
    let Selection::Item(item) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    tab.detail = only(ComposeProbe::default());
    press(&mut tab, &bench, KeyCode::Char('f'));
    assert!(tab.form.is_some(), "the filter form is open");
    tab.on_reply(&StoreReply::NoteForm { item }, &mut bench.ctx());
    assert!(
        !tab.detail.captures_input(),
        "the guard held the reply back"
    );

    press(&mut tab, &bench, KeyCode::Esc);
    assert!(tab.form.is_none(), "Esc closed the filter form");
    tab.on_reply(&StoreReply::NoteForm { item }, &mut bench.ctx());
    assert!(tab.detail.captures_input());
}

/// Milestone 5 E4: `a` in Notes, then `l` before its `NoteForm` answers. The reply reaches
/// only the active sub-tab, so back on Notes nothing captures (and `e` could not have opened
/// the item form over a hidden area).
#[tokio::test]
async fn a_compose_read_answered_after_the_strip_moved_opens_nothing() {
    let bench = Bench::new().await;
    let Selection::Item(item) = bench.first else {
        panic!("the first row is an item")
    };
    let mut tab = bench.tab();
    tab.detail.on_item_change(Some(item));
    assert!(tab.detail.select_id(NotesTab::ID));
    assert_eq!(
        press(&mut tab, &bench, KeyCode::Char('a')),
        Handled::Consumed
    );
    assert!(
        bench.actions().iter().any(|action| matches!(
            action,
            Action::Store(StoreRequest::NoteForm { item: asked }) if *asked == item
        )),
        "`a` sent the NoteForm read"
    );
    press(&mut tab, &bench, KeyCode::Char('l'));
    assert_ne!(
        tab.detail.active_id(),
        Some(NotesTab::ID),
        "the strip moved"
    );

    tab.on_reply(&StoreReply::NoteForm { item }, &mut bench.ctx());
    assert!(tab.detail.select_id(NotesTab::ID));
    assert!(
        !tab.detail.captures_input(),
        "no area opened on the hidden Notes pane"
    );
}
