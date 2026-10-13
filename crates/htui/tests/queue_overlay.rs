//! The queue overlay (MOD-12 M3 D8, D9), end to end through a `Harness` over the memory backend:
//! global `Ctrl+Q` opens it on this box's queue, `J`/`K`, `P` and `Q` write through the store
//! worker and the overlay re-reads, and every third shell refresh tick re-reads it while it is
//! open.
//!
//! The memory backend is `htui --demo`'s: its header reads `demo: nothing is admitted` and its
//! runtime never admits, so every entry keeps its place between keys. The agent runtime keeps a
//! Backlog selection's prompt preview off the status line (`tests/keys.rs`).
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::agent_worker::AgentRuntime;
use htui::app::{Action, register_all};
use htui::keys::{Keys, load_str};
use htui::testkit::Harness;
use htui::ui::overlay::QueueOverlay;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::ItemId;
use htui_core::store::MemStore;

/// The demo shell over `store`, settled.
async fn shell(store: &MemStore) -> Harness {
    settled(Harness::over(store.clone())).await
}

/// `harness` with the agent runtime and every view, settled.
async fn settled(harness: Harness) -> Harness {
    let mut harness = harness.with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness
}

/// The demo `store` with `items` queued on its box, in that call order.
async fn queued(items: &[ItemId]) -> MemStore {
    let store = MemStore::demo();
    for item in items {
        store
            .queue_item(*item, ids::BOX, ids::USER, Utc::now())
            .await
            .expect("the item queues");
    }
    store
}

/// The shell over `store` with the queue overlay open and read.
async fn opened(store: &MemStore) -> Harness {
    open(shell(store).await).await
}

/// The shell over `store`, run with `keys`, with the queue overlay open and read.
async fn opened_with(store: &MemStore, keys: Keys) -> Harness {
    open(settled(Harness::over(store.clone()).with_keys(keys)).await).await
}

/// `harness` with the queue overlay opened by `Ctrl+Q` and read.
async fn open(mut harness: Harness) -> Harness {
    harness.key("ctrl-q");
    harness.drive_to_end().await;
    assert_eq!(
        harness.app().overlays.top().map(|top| top.id()),
        Some(QueueOverlay::ID),
        "`Ctrl+Q` opened it"
    );
    harness
}

/// The lines inside the overlay's box: from its title row to its bottom border, cut at the box's
/// edges, so the Backlog drawn around it is not read.
fn queue_box(frame: &str) -> Vec<String> {
    let lines: Vec<&str> = frame.lines().collect();
    let (top, column, width) = lines
        .iter()
        .enumerate()
        .find_map(|(index, line)| {
            let byte = line.find("┌ Queue ")?;
            let column = line[..byte].chars().count();
            let width = line[byte..].chars().position(|c| c == '┐')? + 1;
            Some((index, column, width))
        })
        .expect("the queue box is drawn");
    lines[top..]
        .iter()
        .map(|line| line.chars().skip(column).take(width).collect::<String>())
        .take_while(|line| !line.starts_with('└'))
        .collect()
}

/// The overlay's item keys, top to bottom, among `keys`.
fn row_order(harness: &mut Harness, keys: &[&str]) -> Vec<String> {
    queue_box(&harness.render())
        .iter()
        .filter_map(|line| {
            let cells = line.trim_start_matches('│').trim_start_matches("> ");
            keys.iter()
                .find(|key| cells.trim_start().starts_with(&format!("{key} ")))
                .map(|key| (*key).to_owned())
        })
        .collect()
}

/// The key of the row the cursor is on, among `keys`.
fn cursor_row(harness: &mut Harness, keys: &[&str]) -> Option<String> {
    queue_box(&harness.render()).iter().find_map(|line| {
        let cells = line.trim_start_matches('│').strip_prefix("> ")?;
        keys.iter()
            .find(|key| cells.starts_with(&format!("{key} ")))
            .map(|key| (*key).to_owned())
    })
}

/// The queue's items in store order.
async fn store_order(store: &MemStore) -> Vec<ItemId> {
    store
        .queue_entries(ids::BOX)
        .await
        .expect("the queue reads")
        .into_iter()
        .map(|entry| entry.item_id)
        .collect()
}

#[tokio::test]
async fn ctrl_q_opens_the_queue_overlay_over_the_memory_backend() {
    let store = queued(&[ids::HTUI_ANA_2, ids::HTUI_FEAT_2]).await;
    let mut harness = opened(&store).await;
    let frame = harness.render();
    let inside = queue_box(&frame).join("\n");
    assert!(inside.contains("ANA-2 "), "{frame}");
    assert!(inside.contains("FEAT-2 "), "{frame}");
    assert!(inside.contains("demo: nothing is admitted"), "{frame}");
    assert!(inside.contains("Esc close"), "{frame}");
    insta::assert_snapshot!("opened", inside);

    // A lower-case `q` is swallowed by the modal overlay: it never quits from inside it.
    harness.key("q");
    assert!(!harness.app().should_quit);
    harness.key("esc");
    assert!(harness.app().overlays.is_empty(), "`Esc` closes it");
}

#[tokio::test]
async fn a_reorder_round_trips_through_the_store() {
    let store = queued(&[ids::HTUI_ANA_2, ids::HTUI_FEAT_2]).await;
    let before = store_order(&store).await;
    assert_eq!(before.len(), 2);
    let mut harness = opened(&store).await;
    let keys = ["ANA-2", "FEAT-2"];
    let shown = row_order(&mut harness, &keys);
    assert_eq!(shown.len(), 2, "both rows are drawn: {shown:?}");

    // The second row moves up.
    harness.key("j");
    harness.key("K");
    harness.drive_to_end().await;

    let after = store_order(&store).await;
    assert_eq!(after, [before[1], before[0]], "the store's order swapped");
    let positions: Vec<Option<i32>> = store
        .queue_entries(ids::BOX)
        .await
        .expect("the queue reads")
        .into_iter()
        .map(|entry| entry.position)
        .collect();
    assert_eq!(positions, [Some(1), Some(2)], "the move materialised 1..n");

    let reordered = row_order(&mut harness, &keys);
    assert_eq!(reordered, [shown[1].clone(), shown[0].clone()]);
    let frame = harness.render();
    assert!(
        queue_box(&frame)
            .iter()
            .any(|line| line.starts_with(&format!("│> {} ", shown[1]))),
        "the cursor stayed on the moved item: {frame}"
    );
}

#[tokio::test]
async fn p_resumes_and_pauses_from_the_overlay() {
    let store = queued(&[ids::HTUI_ANA_2]).await;
    assert_eq!(store.open_batch_of(ids::BOX).await.expect("reads"), None);
    let mut harness = opened(&store).await;
    let header = |harness: &mut Harness| queue_box(&harness.render())[1].clone();
    assert!(
        header(&mut harness).contains("queue: paused · demo: nothing is admitted"),
        "{}",
        harness.render()
    );

    harness.key("P");
    harness.drive_to_end().await;
    assert!(
        store
            .open_batch_of(ids::BOX)
            .await
            .expect("reads")
            .is_some()
    );
    assert!(
        header(&mut harness).contains("queue: running · demo: nothing is admitted"),
        "{}",
        harness.render()
    );

    // M3 review R1 L4: once a batch closed, the header says why it is paused.
    harness.key("P");
    harness.drive_to_end().await;
    assert_eq!(store.open_batch_of(ids::BOX).await.expect("reads"), None);
    assert!(
        header(&mut harness).contains("queue: paused (by a user) · demo: nothing is admitted"),
        "{}",
        harness.render()
    );
}

#[tokio::test]
async fn capital_q_dequeues_from_the_overlay() {
    let store = queued(&[ids::HTUI_ANA_2, ids::HTUI_FEAT_2]).await;
    let mut harness = opened(&store).await;
    let keys = ["ANA-2", "FEAT-2"];
    let shown = row_order(&mut harness, &keys);
    let first = store_order(&store).await[0];

    harness.key("Q");
    harness.drive_to_end().await;
    let left = store_order(&store).await;
    assert!(!left.contains(&first), "the cursor row left the queue");
    assert_eq!(left.len(), 1);
    assert_eq!(row_order(&mut harness, &keys), [shown[1].clone()]);
}

#[tokio::test]
async fn the_overlay_re_reads_on_the_refresh_tick() {
    let store = queued(&[ids::HTUI_ANA_2]).await;
    let mut harness = opened(&store).await;
    let keys = ["ANA-2", "TOOL-1"];
    assert_eq!(row_order(&mut harness, &keys), ["ANA-2"]);

    // Queued behind the overlay's back: nothing re-reads until the tick.
    store
        .queue_item(ids::HTUI_TOOL_1, ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the item queues");
    harness.drive_to_end().await;
    assert_eq!(row_order(&mut harness, &keys), ["ANA-2"]);

    // One shell refresh (`update.rs`'s private `TICKS_PER_REFRESH` is 4) is not enough: the
    // overlay re-reads every third refresh (M3 review R1 M2).
    let refresh = |harness: &mut Harness| {
        for _ in 0..4 {
            harness.app().update(Action::Tick);
        }
    };
    refresh(&mut harness);
    refresh(&mut harness);
    harness.drive_to_end().await;
    assert_eq!(row_order(&mut harness, &keys), ["ANA-2"]);
    refresh(&mut harness);
    harness.drive_to_end().await;
    let rows = row_order(&mut harness, &keys);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.contains(&"TOOL-1".to_owned()), "{rows:?}");
}

/// MOD-12 M3 review R1 M1: the overlay shows a running batch; the runner closes it as stalled
/// (L4) before the next read; `P` meant pause, and the pause names the batch the box showed, so
/// it is refused rather than read as a resume: no batch opens, and the box re-reads the queue.
#[tokio::test]
async fn p_over_a_batch_the_runner_closed_is_refused_and_opens_nothing() {
    let store = queued(&[ids::HTUI_ANA_2]).await;
    let seen = store
        .open_batch(ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the batch opens")
        .id;
    let mut harness = opened(&store).await;
    let header = |harness: &mut Harness| queue_box(&harness.render())[1].clone();
    assert!(
        header(&mut harness).contains("queue: running"),
        "{}",
        harness.render()
    );

    // The runner's stall close (L4), behind the overlay's back: admit read ANA-2's entry.
    store
        .close_drained_batch(seen, &[ids::HTUI_ANA_2], Utc::now())
        .await
        .expect("the store answers")
        .expect("the stalled batch closed");

    harness.key("P");
    harness.drive_to_end().await;
    assert_eq!(
        store.open_batch_of(ids::BOX).await.expect("reads"),
        None,
        "no batch opened"
    );
    let last = store
        .last_closed_batch(ids::BOX)
        .await
        .expect("reads")
        .expect("a batch closed");
    assert_eq!(last.id, seen, "the runner's close is the last one");
    assert_eq!(
        harness.app().status.as_deref(),
        Some("queue already paused: its batch closed before P reached it")
    );
    assert!(
        header(&mut harness).contains("queue: paused (the last batch drained: nothing admissible)"),
        "the box re-read the queue, and says why it is paused (L4): {}",
        harness.render()
    );
}

/// MOD-67 M6 X-S (PA-5): `[queue] pause = "X"`; the runner closes the batch the box showed, and
/// the refused pause names `X`, the key that pauses in this box, not the default `P`.
#[tokio::test]
async fn a_rebound_pause_key_is_named_after_a_stale_pause() {
    let store = queued(&[ids::HTUI_ANA_2]).await;
    let seen = store
        .open_batch(ids::BOX, ids::USER, Utc::now())
        .await
        .expect("the batch opens")
        .id;
    let keys = load_str("[queue]\npause = \"X\"\n").expect("the keys load");
    let mut harness = opened_with(&store, keys).await;

    // The runner's stall close (L4), behind the overlay's back.
    store
        .close_drained_batch(seen, &[ids::HTUI_ANA_2], Utc::now())
        .await
        .expect("the store answers")
        .expect("the stalled batch closed");

    harness.key("X");
    harness.drive_to_end().await;
    assert_eq!(
        store.open_batch_of(ids::BOX).await.expect("reads"),
        None,
        "no batch opened"
    );
    assert_eq!(
        harness.app().status.as_deref(),
        Some("queue already paused: its batch closed before X reached it")
    );
}

/// MOD-67 M6: the lane's rebinding test, through the store. `[queue] move_down = "D"` and
/// `move_up = "U"`: `J` is inert, `D` moves the cursor row down in the store, and the hint names
/// the new chords.
#[tokio::test]
async fn a_rebound_reorder_key_round_trips_through_the_store() {
    let store = queued(&[ids::HTUI_ANA_2, ids::HTUI_FEAT_2]).await;
    let before = store_order(&store).await;
    let keys = load_str("[queue]\nmove_down = \"D\"\nmove_up = \"U\"\n").expect("the keys load");
    let mut harness = opened_with(&store, keys).await;

    harness.key("J");
    harness.drive_to_end().await;
    assert_eq!(store_order(&store).await, before, "`J` moved nothing");

    harness.key("D");
    harness.drive_to_end().await;
    assert_eq!(
        store_order(&store).await,
        [before[1], before[0]],
        "`D` moved the first entry down"
    );
    let frame = harness.render();
    let inside = queue_box(&frame).join("\n");
    assert!(inside.contains("D/U reorder"), "{frame}");
    assert!(!inside.contains("J/K reorder"), "{frame}");
}

/// MOD-67 M6 (D2, D8): over the queue the `?` box renders the queue's stack, one line per layer.
#[tokio::test]
async fn the_help_box_lists_the_queue_keys() {
    let store = queued(&[ids::HTUI_ANA_2]).await;
    let mut harness = opened(&store).await;
    harness.key("?");
    assert!(harness.app().help_visible, "`?` opens the box");
    let frame = harness.render();
    assert!(
        frame
            .lines()
            .any(|line| line.contains("Queue: J move down")),
        "{frame}"
    );
    for line in [
        "List: j/Down down · k/Up up",
        "Overlay: Esc close",
        "Global: Ctrl+c quit · ?/F1 help",
    ] {
        assert!(frame.contains(line), "`{line}` is in the box: {frame}");
    }
    harness.key("?");
    assert!(!harness.app().help_visible, "`?` again closes it");
}

/// MOD-67 M6 (D2): chord equality includes the modifiers, so `Ctrl+J` moves no cursor and
/// `Ctrl+P` or `Alt+P` pauses nothing.
#[tokio::test]
async fn a_ctrl_chord_does_not_steer_the_queue() {
    let store = queued(&[ids::HTUI_ANA_2, ids::HTUI_FEAT_2]).await;
    let mut harness = opened(&store).await;
    let keys = ["ANA-2", "FEAT-2"];
    let first = cursor_row(&mut harness, &keys);
    assert!(first.is_some(), "{}", harness.render());

    for chord in ["ctrl-j", "ctrl-P", "alt-P"] {
        harness.key(chord);
        harness.drive_to_end().await;
    }
    assert_eq!(cursor_row(&mut harness, &keys), first, "the cursor stayed");
    assert_eq!(
        store.open_batch_of(ids::BOX).await.expect("reads"),
        None,
        "the queue is still paused"
    );
    assert!(!harness.app().overlays.is_empty(), "the overlay is open");
}
