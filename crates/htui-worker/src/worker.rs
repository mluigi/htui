//! `htui worker`'s loop (MOD-41 plan D14): poll, command poll (MOD-42 plan D13), box heartbeat,
//! shutdown.

use std::future::Future;
use std::time::Duration;

use htui_store::PgStore;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::{LiveChats, RunRuntime, Unaddressed};

/// How often the worker sweeps and scans (plan D14): hand-back latency is bounded by this.
pub const WORKER_POLL: Duration = Duration::from_secs(5);

/// MOD-42 plan D13: how often pending run commands are read (a `const`, so `WorkerConfig`
/// literals stay as they are).
pub const COMMAND_POLL: Duration = Duration::from_secs(1);

/// A cancelled walk's graceful window on shutdown, as the TUI's `CANCEL_GRACE`.
pub const WALK_GRACE: Duration = Duration::from_secs(2);

/// The worker's periods (blueprint B-11: tests shorten them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerConfig {
    /// Sweep and claim scan.
    pub poll: Duration,
    /// `box.last_seen_at`; the first beat is at start.
    pub box_beat: Duration,
    /// Walk grace on shutdown.
    pub grace: Duration,
}

impl WorkerConfig {
    /// Production: 5 s, `BOX_HEARTBEAT`, 2 s.
    pub const PRODUCTION: Self = Self {
        poll: WORKER_POLL,
        box_beat: htui_store::connect::BOX_HEARTBEAT,
        grace: WALK_GRACE,
    };
}

/// The loop until `shutdown` resolves, then the runtime's shutdown (walks cancelled, leases
/// given back). A store outage is logged by each arm and the loop carries on (plan D14).
///
/// Every ticker fires at once (blueprint B-11): the first sweep, the first command poll (MOD-42
/// plan D13, every [`COMMAND_POLL`]) and the first box beat are at start, not one period out.
/// One beat is in flight at a time, on its own task, so a slow server never delays a sweep; a
/// beat still in flight at shutdown is aborted.
pub async fn run(
    host: PgStore,
    mut runtime: RunRuntime<PgStore, Unaddressed>,
    config: WorkerConfig,
    shutdown: impl Future<Output = ()>,
) {
    let mut poll = tokio::time::interval(config.poll);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut beat = tokio::time::interval(config.box_beat);
    beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut commands = tokio::time::interval(COMMAND_POLL);
    commands.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut beating: Option<JoinHandle<()>> = None;
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => break,
            _ = poll.tick() => runtime.sweep_with(&host, &Unaddressed),
            // MOD-42 plan D13: the worker has no chat, so no live chat refuses a cancel.
            _ = commands.tick() => {
                runtime.poll_commands_with(&host, &Unaddressed, LiveChats::default());
            }
            _ = beat.tick(), if beating.is_none() => {
                beating = Some(tokio::spawn(beat_once(host.clone())));
            }
            () = in_flight(&mut beating), if beating.is_some() => beating = None,
        }
    }
    if let Some(beat) = beating {
        beat.abort();
    }
    runtime.shutdown(config.grace).await;
}

/// The box heartbeat in flight, to its end: pending when there is none. A beat that panicked or
/// was aborted ends it all the same (as `htui`'s store loop).
async fn in_flight(beat: &mut Option<JoinHandle<()>>) {
    match beat {
        Some(handle) => drop(handle.await),
        None => std::future::pending().await,
    }
}

/// One box heartbeat (MOD-40 plan D7), whatever the box's executor. A failure is logged and
/// nothing else: the next sweep meets the same outage and the loop carries on.
async fn beat_once(pg: PgStore) {
    let id = pg.this_box();
    match pg.touch_box(id).await {
        Ok(true) => {}
        Ok(false) => tracing::warn!(box_id = %id, "the box heartbeat found no box row to touch"),
        Err(err) => tracing::warn!(box_id = %id, %err, "the box heartbeat failed"),
    }
}
