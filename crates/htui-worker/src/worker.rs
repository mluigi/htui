//! `htui worker`'s loop (MOD-41 plan D14): poll, box heartbeat, shutdown.

use std::future::Future;
use std::time::Duration;

use htui_store::PgStore;

use crate::{RunRuntime, Unaddressed};

/// How often the worker sweeps and scans (plan D14): hand-back latency is bounded by this.
pub const WORKER_POLL: Duration = Duration::from_secs(5);

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
/// Signature only (T12's red commit): it waits for `shutdown` and does nothing else yet.
pub async fn run(
    host: PgStore,
    mut runtime: RunRuntime<PgStore, Unaddressed>,
    config: WorkerConfig,
    shutdown: impl Future<Output = ()>,
) {
    drop(host);
    shutdown.await;
    runtime.shutdown(config.grace).await;
}
