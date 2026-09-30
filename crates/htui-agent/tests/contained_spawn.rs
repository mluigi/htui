//! MOD-65: every thread and task `htui_agent::contained` starts runs inside the contain window, and
//! a raw tokio spawn does not.
//!
//! Its own test binary, and one `#[test]` in it, because [`std::panic::set_hook`] is **process**
//! state: a second case running in parallel in this binary would see this one's hook, and this one
//! would see its panics (the precedent is `crates/htui/tests/panic_hook.rs`, for the same reason).
//!
//! It asks [`htui_agent::excerpt::panic_is_contained`] and not
//! `htui::terminal::restores_the_terminal`, because `htui` is not a dependency of this crate, and
//! the one is the negation of the other: what the hook decides is exactly what the window says on
//! the panicking thread.

use std::sync::{Arc, Mutex};

use htui_agent::contained;
use htui_agent::excerpt::panic_is_contained;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::{JoinSet, yield_now};

/// The decision recorded for the panic whose message is `key`, and how many panics carried it.
fn decision(seen: &[(String, bool)], key: &str) -> (Option<bool>, usize) {
    let matching: Vec<bool> = seen
        .iter()
        .filter(|(message, _)| message == key)
        .map(|(_, contained)| *contained)
        .collect();
    (matching.first().copied(), matching.len())
}

#[test]
fn every_contained_spawn_opens_the_window_and_a_raw_one_does_not() {
    // Built before the hook goes in, so a failure here still reports its message. Two workers, so
    // every panic in cases 1-4 happens on a thread other than this one.
    let pool = Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .expect("a two-worker runtime builds");
    // One thread for case 5: the parked task and the raw task must share it, or the raw task would
    // read another worker's flag and `outside` would be false for no reason (H-7).
    let current = Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime builds");

    // What the window said, per panic, keyed by the panic's message: the panics arrive from
    // different threads, so their order means nothing. Replaces the default hook rather than
    // chaining it: the four panics below are all expected. Nothing is asserted while this hook is
    // installed, because a failing assertion's own message would land in the recorder (H-6).
    let decisions: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let recorder = Arc::clone(&decisions);
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-text>".to_owned());
        if let Ok(mut seen) = recorder.lock() {
            seen.push((message, panic_is_contained()));
        }
    }));

    let (blocking_panicked, task_panicked, set_panicked, raw_panicked, values) =
        pool.block_on(async {
            // 1. A blocking thread.
            let blocking = contained::spawn_blocking(|| {
                panic!("contained: blocking");
            })
            .await
            .is_err_and(|error| error.is_panic());

            // 2. A task that panics on its **second** poll: the window has to reopen on every
            //    poll, not be opened once.
            let task = contained::spawn(async {
                yield_now().await;
                panic!("contained: task, second poll");
            })
            .await
            .is_err_and(|error| error.is_panic());

            // 3. A task on a `JoinSet`.
            let mut set = JoinSet::new();
            contained::spawn_in(&mut set, async {
                yield_now().await;
                panic!("contained: set");
            });
            let in_set = matches!(set.join_next().await, Some(Err(ref error)) if error.is_panic());

            // 4. The control: a raw spawn must record `false`, or cases 1-3 prove nothing.
            let raw = tokio::task::spawn_blocking(|| {
                panic!("raw: blocking");
            })
            .await
            .is_err_and(|error| error.is_panic());

            // 6. Values pass through unchanged.
            let blocking_value = contained::spawn_blocking(|| 7_u8).await.ok();
            let task_value = contained::spawn(async {
                yield_now().await;
                "task"
            })
            .await
            .ok();
            let mut numbers = JoinSet::new();
            contained::spawn_in(&mut numbers, async { 11_u32 });
            let set_value = numbers.join_next().await.map(Result::ok);

            (
                blocking,
                task,
                in_set,
                raw,
                (blocking_value, task_value, set_value),
            )
        });

    // 5. No window stays open across a `Pending` (D4): while the contained task is parked, a raw
    //    task on the same thread sees the flag down; when it is polled again, the flag is up.
    let window = current.block_on(async {
        let (entered_tx, entered_rx) = oneshot::channel::<bool>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let parked = contained::spawn(async move {
            let _ = entered_tx.send(panic_is_contained());
            release_rx.await.ok();
            panic_is_contained()
        });
        let raw = tokio::spawn(async move {
            let inside = entered_rx.await.unwrap_or(false);
            let outside = panic_is_contained();
            let _ = release_tx.send(());
            (inside, outside)
        });
        match (raw.await, parked.await) {
            (Ok((inside, outside)), Ok(after)) => Some((inside, outside, after)),
            _ => None,
        }
    });

    let _ = std::panic::take_hook();
    drop(pool);
    drop(current);

    let seen = decisions
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();

    assert!(
        blocking_panicked,
        "case 1: the blocking panic is a panic `JoinError`"
    );
    assert!(
        task_panicked,
        "case 2: the task panic is a panic `JoinError`"
    );
    assert!(
        set_panicked,
        "case 3: the set task panic is a panic `JoinError`"
    );
    assert!(raw_panicked, "case 4: the raw panic is a panic `JoinError`");

    assert_eq!(
        decision(&seen, "contained: blocking"),
        (Some(true), 1),
        "case 1: a contained blocking thread panics inside the window: {seen:?}"
    );
    assert_eq!(
        decision(&seen, "contained: task, second poll"),
        (Some(true), 1),
        "case 2: a contained task panics inside the window on its second poll too: {seen:?}"
    );
    assert_eq!(
        decision(&seen, "contained: set"),
        (Some(true), 1),
        "case 3: a contained set task panics inside the window: {seen:?}"
    );
    assert_eq!(
        decision(&seen, "raw: blocking"),
        (Some(false), 1),
        "case 4: a raw blocking thread panics outside any window, so the recorder can see `false`: \
         {seen:?}"
    );
    assert_eq!(seen.len(), 4, "four panics, four decisions: {seen:?}");

    let (inside, outside, after) =
        window.expect("case 5: both tasks on the current-thread runtime finished");
    assert!(
        inside,
        "case 5: the contained task's first poll is inside the window"
    );
    assert!(
        !outside,
        "case 5: no window stays open across the contained task's `Pending` (D4)"
    );
    assert!(
        after,
        "case 5: the window reopens on the contained task's next poll"
    );

    assert_eq!(
        values,
        (Some(7), Some("task"), Some(Some(11))),
        "case 6: each helper hands back its closure's or future's output unchanged"
    );
}
