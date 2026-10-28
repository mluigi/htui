//! The status-line sentence a queue write says (MOD-12 D9), shared by the Backlog tab and the queue
//! overlay (M3 review R1 N2). It speaks of the store worker's [`QueueWrite`] and [`QueueView`], so
//! it lives beside the views, not in the kit, which names no domain type (MOD-115 M4 T0, D-23).

use crate::store_worker::{QueueView, QueueWrite};

/// MOD-12 D9 (review L3): what a running queue adds in `htui --demo`, whose runtime never admits.
pub(crate) const DEMO_NOTHING_ADMITTED: &str = "demo: nothing is admitted";

/// MOD-12 D9: the status line after a queue write. `n` is the queue's length after it. In
/// `htui --demo` (`view.demo`) a running queue says nothing is admitted (review L3). The queue
/// overlay says its writes with it too (M3 review R1 N2). `pause` is the label of the key that
/// pauses on the caller's stack (MOD-67 M6 X-S): a refused pause names it, and drops the clause
/// that would when none is bound.
pub(crate) fn queue_sentence(
    write: &QueueWrite,
    key: &str,
    view: &QueueView,
    pause: Option<&str>,
) -> String {
    let n = view.entries.len();
    match *write {
        QueueWrite::Queued { .. } => match (view.open_batch.is_some(), view.demo) {
            (true, false) => format!("queued {key} ({n} in queue, running)"),
            (true, true) => {
                format!("queued {key} ({n} in queue, running; {DEMO_NOTHING_ADMITTED})")
            }
            (false, _) => format!("queued {key} ({n} in queue, paused)"),
        },
        QueueWrite::Dequeued {
            was_queued: true, ..
        } => format!("dequeued {key} ({n} in queue)"),
        QueueWrite::Dequeued {
            was_queued: false, ..
        } => format!("{key} was not queued"),
        QueueWrite::Resumed { already: false } if view.demo => {
            format!("queue resumed ({DEMO_NOTHING_ADMITTED})")
        }
        QueueWrite::Resumed { already: false } => "queue resumed".to_owned(),
        QueueWrite::Resumed { already: true } if view.demo => {
            format!("queue already running ({DEMO_NOTHING_ADMITTED})")
        }
        QueueWrite::Resumed { already: true } => "queue already running".to_owned(),
        QueueWrite::Paused { already: true, .. } => "queue already paused".to_owned(),
        QueueWrite::Paused { live: 0, .. } => "queue paused".to_owned(),
        QueueWrite::Paused { live: 1, .. } => {
            "queue paused \u{2014} 1 run still running".to_owned()
        }
        QueueWrite::Paused { live, .. } => {
            format!("queue paused \u{2014} {live} runs still running")
        }
        // MOD-12 M3 D7: only the queue overlay sends `MoveQueueEntry` (replies are addressed by
        // origin); it says these through this function (M3 review R1 N2).
        QueueWrite::Moved { moved: true } => format!("queue reordered ({n} in queue)"),
        QueueWrite::Moved { moved: false } => "already at that end of the queue".to_owned(),
        // M3 review R1 M1: `P` sent over a queue that changed since it was read; `view` is the
        // queue now.
        QueueWrite::Stale { pause: true } if view.open_batch.is_none() => match pause {
            Some(pause) => {
                format!("queue already paused: its batch closed before {pause} reached it")
            }
            None => "queue already paused: its batch closed first".to_owned(),
        },
        QueueWrite::Stale { pause: true } => match pause {
            Some(pause) => {
                format!(
                    "queue not paused: it was resumed since it was read; {pause} again pauses it"
                )
            }
            None => "queue not paused: it was resumed since it was read".to_owned(),
        },
        QueueWrite::Stale { pause: false } if view.open_batch.is_some() => {
            "queue already running: it was resumed since it was read".to_owned()
        }
        QueueWrite::Stale { pause: false } => {
            "queue not resumed: it changed since it was read".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use htui_core::model::ItemId;

    use super::*;

    /// MOD-12 D9 (blueprint §B.5.2): every row of the queue sentence table.
    #[test]
    fn queue_sentence_covers_every_write() {
        let item = ItemId::new();
        let paused = |n: usize| QueueView {
            entries: vec![item; n],
            open_batch: None,
            demo: false,
        };
        let running = QueueView {
            entries: vec![item; 3],
            open_batch: Some(htui_core::model::BatchId::new()),
            demo: false,
        };
        let demo = |view: QueueView| QueueView { demo: true, ..view };
        let cases = [
            (
                QueueWrite::Queued { item },
                paused(1),
                "queued K-1 (1 in queue, paused)",
            ),
            (
                QueueWrite::Queued { item },
                running.clone(),
                "queued K-1 (3 in queue, running)",
            ),
            (
                QueueWrite::Dequeued {
                    item,
                    was_queued: true,
                },
                paused(0),
                "dequeued K-1 (0 in queue)",
            ),
            (
                QueueWrite::Dequeued {
                    item,
                    was_queued: false,
                },
                paused(0),
                "K-1 was not queued",
            ),
            (
                QueueWrite::Resumed { already: false },
                running.clone(),
                "queue resumed",
            ),
            (
                QueueWrite::Resumed { already: true },
                running.clone(),
                "queue already running",
            ),
            // Review L3: `htui --demo` never admits, so a running queue says so; a paused one
            // and a dequeue are as true there as anywhere.
            (
                QueueWrite::Queued { item },
                demo(running.clone()),
                "queued K-1 (3 in queue, running; demo: nothing is admitted)",
            ),
            (
                QueueWrite::Queued { item },
                demo(paused(1)),
                "queued K-1 (1 in queue, paused)",
            ),
            (
                QueueWrite::Resumed { already: false },
                demo(running.clone()),
                "queue resumed (demo: nothing is admitted)",
            ),
            (
                QueueWrite::Resumed { already: true },
                demo(running.clone()),
                "queue already running (demo: nothing is admitted)",
            ),
            (
                QueueWrite::Paused {
                    live: 0,
                    already: false,
                },
                demo(paused(0)),
                "queue paused",
            ),
            (
                QueueWrite::Paused {
                    live: 0,
                    already: true,
                },
                paused(0),
                "queue already paused",
            ),
            (
                QueueWrite::Paused {
                    live: 0,
                    already: false,
                },
                paused(0),
                "queue paused",
            ),
            (
                QueueWrite::Paused {
                    live: 1,
                    already: false,
                },
                paused(0),
                "queue paused \u{2014} 1 run still running",
            ),
            (
                QueueWrite::Paused {
                    live: 2,
                    already: false,
                },
                paused(0),
                "queue paused \u{2014} 2 runs still running",
            ),
            // MOD-12 M3 D7: never sent by the Backlog; the match is exhaustive.
            (
                QueueWrite::Moved { moved: true },
                paused(2),
                "queue reordered (2 in queue)",
            ),
            (
                QueueWrite::Moved { moved: false },
                paused(2),
                "already at that end of the queue",
            ),
            // M3 review R1 M1: a pause or resume over a queue that changed since it was read.
            (
                QueueWrite::Stale { pause: true },
                paused(1),
                "queue already paused: its batch closed before P reached it",
            ),
            (
                QueueWrite::Stale { pause: true },
                running.clone(),
                "queue not paused: it was resumed since it was read; P again pauses it",
            ),
            (
                QueueWrite::Stale { pause: false },
                running.clone(),
                "queue already running: it was resumed since it was read",
            ),
            (
                QueueWrite::Stale { pause: false },
                paused(1),
                "queue not resumed: it changed since it was read",
            ),
        ];
        for (write, view, sentence) in cases {
            assert_eq!(
                queue_sentence(&write, "K-1", &view, Some("P")),
                sentence,
                "{write:?}"
            );
        }
    }

    /// MOD-67 M6 X-S (PA-5): a refused pause names the key that pauses, the one the caller's
    /// stack binds; with no key bound the sentence drops the clause that would name one.
    #[test]
    fn a_rebound_pause_key_is_the_one_the_stale_sentence_names() {
        let paused = QueueView {
            entries: vec![ItemId::new()],
            open_batch: None,
            demo: false,
        };
        let running = QueueView {
            open_batch: Some(htui_core::model::BatchId::new()),
            ..paused.clone()
        };
        let stale = QueueWrite::Stale { pause: true };
        let cases = [
            (
                &paused,
                Some("X"),
                "queue already paused: its batch closed before X reached it",
            ),
            (
                &running,
                Some("X"),
                "queue not paused: it was resumed since it was read; X again pauses it",
            ),
            (
                &paused,
                None,
                "queue already paused: its batch closed first",
            ),
            (
                &running,
                None,
                "queue not paused: it was resumed since it was read",
            ),
        ];
        for (view, pause, sentence) in cases {
            assert_eq!(
                queue_sentence(&stale, "", view, pause),
                sentence,
                "{pause:?}"
            );
        }
    }
}
