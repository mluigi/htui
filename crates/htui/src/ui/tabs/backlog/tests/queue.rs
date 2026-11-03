//! The Backlog's queue toggles: `Q`, `P` and the writes their reads decide.

use super::*;

/// MOD-12 D9 (review NIT-intent): one intent is at most one write, and a write's answer does
/// not wipe a newer intent: `P` pressed while `Q`'s write is in flight still sends its own.
#[tokio::test]
async fn p_pressed_while_q_writes_still_sends_its_write() {
    let bench = Bench::new().await;
    let ana_2 = htui_core::fixtures::ids::HTUI_ANA_2;
    let mut tab = BacklogTab {
        selected: Some(Selection::Item(ana_2)),
        ..bench.tab()
    };
    let paused = |entries: Vec<ItemId>| QueueView {
        entries,
        open_batch: None,
        demo: false,
    };

    press(&mut tab, &bench, KeyCode::Char('Q'));
    assert_eq!(queue_requests(&bench.actions()), ["queue_state"]);
    tab.on_reply(&StoreReply::Queue(paused(Vec::new())), &mut bench.ctx());
    assert_eq!(
        queue_requests(&bench.actions()),
        [format!(
            "queue_item {ana_2} in {}",
            bench.scope.workspace_id
        )]
    );
    tab.on_reply(&StoreReply::Queue(paused(Vec::new())), &mut bench.ctx());
    assert_eq!(
        queue_requests(&bench.actions()),
        Vec::<String>::new(),
        "the intent was spent on the first read: one intent, one write"
    );

    press(&mut tab, &bench, KeyCode::Char('P'));
    assert_eq!(queue_requests(&bench.actions()), ["queue_state"]);
    tab.on_reply(
        &StoreReply::QueueWritten {
            write: QueueWrite::Queued { item: ana_2 },
            view: paused(vec![ana_2]),
        },
        &mut bench.ctx(),
    );
    let actions = bench.actions();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Error(status) if status == "queued ANA-2 (1 in queue, paused)"
        )),
        "Q's write still says what it did: {actions:?}"
    );
    tab.on_reply(&StoreReply::Queue(paused(vec![ana_2])), &mut bench.ctx());
    assert_eq!(
        queue_requests(&bench.actions()),
        ["resume_queue"],
        "P's read still decides P's write"
    );
}

/// MOD-12 M3 review R1 M1: `P`'s write names the state its read saw, the open batch or a
/// paused queue, and a refusal (`Stale`) puts its sentence on the status line.
#[tokio::test]
async fn p_sends_the_state_its_read_saw_and_says_a_refusal() {
    let bench = Bench::new().await;
    let mut tab = bench.tab();
    let batch = htui_core::model::BatchId::new();
    let view = |open_batch| QueueView {
        entries: Vec::new(),
        open_batch,
        demo: false,
    };

    press(&mut tab, &bench, KeyCode::Char('P'));
    bench.actions();
    tab.on_reply(&StoreReply::Queue(view(Some(batch))), &mut bench.ctx());
    let actions = bench.actions();
    assert!(
        matches!(
            actions.as_slice(),
            [Action::Store(StoreRequest::PauseQueue { expect: Some(seen) })] if *seen == batch
        ),
        "{actions:?}"
    );

    press(&mut tab, &bench, KeyCode::Char('P'));
    bench.actions();
    tab.on_reply(&StoreReply::Queue(view(None)), &mut bench.ctx());
    let actions = bench.actions();
    assert!(
        matches!(
            actions.as_slice(),
            [Action::Store(StoreRequest::ResumeQueue {
                expect_paused: true
            })]
        ),
        "{actions:?}"
    );

    tab.on_reply(
        &StoreReply::QueueWritten {
            write: QueueWrite::Stale { pause: true },
            view: view(None),
        },
        &mut bench.ctx(),
    );
    let actions = bench.actions();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Error(status)
                if status == "queue already paused: its batch closed before P reached it"
        )),
        "{actions:?}"
    );
}

/// MOD-12 D9 (review NIT-intent, G3): a refused write spends only its own in-flight key, so
/// `P` pressed while `Q`'s write is refused (review L1's `queued elsewhere`) still sends its
/// own write; a refused read spends the toggle it was for.
#[tokio::test]
async fn a_refused_queue_write_keeps_a_newer_toggle_and_a_refused_read_spends_it() {
    let bench = Bench::new().await;
    let ana_2 = htui_core::fixtures::ids::HTUI_ANA_2;
    let mut tab = BacklogTab {
        selected: Some(Selection::Item(ana_2)),
        ..bench.tab()
    };
    let paused = |entries: Vec<ItemId>| QueueView {
        entries,
        open_batch: None,
        demo: false,
    };
    let refused = |request: &'static str| StoreReply::Failed {
        request,
        message: "the item is queued on another box; dequeue it there".to_owned(),
    };

    press(&mut tab, &bench, KeyCode::Char('Q'));
    tab.on_reply(&StoreReply::Queue(paused(Vec::new())), &mut bench.ctx());
    assert_eq!(
        queue_requests(&bench.actions()),
        [
            "queue_state".to_owned(),
            format!("queue_item {ana_2} in {}", bench.scope.workspace_id)
        ]
    );
    press(&mut tab, &bench, KeyCode::Char('P'));
    assert_eq!(queue_requests(&bench.actions()), ["queue_state"]);
    tab.on_reply(
        &refused(
            StoreRequest::QueueItem {
                item: ana_2,
                workspace: bench.scope.workspace_id,
            }
            .name(),
        ),
        &mut bench.ctx(),
    );
    assert!(
        tab.queue_writing.is_none(),
        "the refused write's key is spent"
    );
    tab.on_reply(&StoreReply::Queue(paused(Vec::new())), &mut bench.ctx());
    assert_eq!(
        queue_requests(&bench.actions()),
        ["resume_queue"],
        "P's read still decides P's write"
    );

    press(&mut tab, &bench, KeyCode::Char('P'));
    assert_eq!(queue_requests(&bench.actions()), ["queue_state"]);
    tab.on_reply(&refused(StoreRequest::QueueState.name()), &mut bench.ctx());
    tab.on_reply(&StoreReply::Queue(paused(Vec::new())), &mut bench.ctx());
    assert_eq!(
        queue_requests(&bench.actions()),
        Vec::<String>::new(),
        "a refused read spent its toggle: a later read writes nothing"
    );
}

/// MOD-12 D9 (review NIT-intent, G3): a write's answer names its item by the key taken when
/// it was sent, so an item gone from the list meanwhile is still named by its key.
#[tokio::test]
async fn a_queue_write_names_its_item_after_the_list_dropped_it() {
    let bench = Bench::new().await;
    let ana_2 = htui_core::fixtures::ids::HTUI_ANA_2;
    let mut tab = BacklogTab {
        selected: Some(Selection::Item(ana_2)),
        ..bench.tab()
    };
    let paused = |entries: Vec<ItemId>| QueueView {
        entries,
        open_batch: None,
        demo: false,
    };

    press(&mut tab, &bench, KeyCode::Char('Q'));
    tab.on_reply(&StoreReply::Queue(paused(Vec::new())), &mut bench.ctx());
    assert_eq!(
        queue_requests(&bench.actions()),
        [
            "queue_state".to_owned(),
            format!("queue_item {ana_2} in {}", bench.scope.workspace_id)
        ]
    );
    tab.on_reply(&StoreReply::Items(Vec::new()), &mut bench.ctx());
    let _ = bench.actions();
    tab.on_reply(
        &StoreReply::QueueWritten {
            write: QueueWrite::Queued { item: ana_2 },
            view: paused(vec![ana_2]),
        },
        &mut bench.ctx(),
    );
    let actions = bench.actions();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Error(status) if status == "queued ANA-2 (1 in queue, paused)"
        )),
        "named by its key, not its id: {actions:?}"
    );
}
