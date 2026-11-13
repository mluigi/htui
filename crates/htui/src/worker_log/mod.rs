//! The headless worker's log shipper (MOD-122 milestone 2, plan D2, D3, D7, D8, D9).
//!
//! [`ShipHandle::layer`] is a `tracing` layer behind its own level filter: every event at or above
//! the box's [`LogShipLevel`] is formatted as the M1 memory layer formats it, scrubbed, and kept
//! in a bounded buffer. [`spawn_flusher`] writes the buffer to `worker_log` through the worker's
//! [`PgStore`](htui_store::PgStore) every [`FLUSH_EVERY`] or [`BATCH_LINES`] lines, re-reads the
//! level from the worker's own box row each cycle, and sweeps old rows at start and hourly.
//!
//! Nothing here formats a DSN, a key, a token or an environment value; a line the scrubber refuses
//! keeps its level and target and ships as [`WITHHELD`], and a refused target ships as
//! [`WITHHELD_TARGET`]. A kept text is cut to [`WORKER_LOG_TEXT_MAX`] after the scrub. At
//! shutdown, [`FlusherTask::shutdown`] ships what is left, within [`FINAL_FLUSH_MAX`].

pub mod ship;

pub use ship::{
    DEFAULT_RETENTION_DAYS, FINAL_FLUSH_MAX, FlusherTask, PER_BOX_CAP, SWEEP_EVERY, ShipSource,
    retention_days, run_flusher, spawn_flusher,
};

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::Utc;
use htui_core::model::{LogShipLevel, NewWorkerLog, WORKER_LOG_TEXT_MAX, WorkerLogLevel};
use htui_core::scrub::{MinimalScrubber, Scrubber as _};
use serde_json::Value;
use tokio::sync::Notify;
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::{FilterFn, Filtered, LevelFilter};

use crate::logging::buffer::LineVisitor;

/// Plan D2: a flush is due once this many lines wait, or after [`FLUSH_EVERY`].
pub const BATCH_LINES: usize = 200;
/// Plan D2: the flush interval while shipping works; also the first back-off step.
pub const FLUSH_EVERY: Duration = Duration::from_secs(2);
/// Plan D3: the longest pause between two failed flushes.
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Plan D3: the most lines the worker holds, buffered and in flight together. Raised from the
/// plan's 5 000 by T2's measurement (10 000 `info` lines a second for 30 s): the sandbox Postgres
/// stalled single flushes for up to 1.1 s, and at 5 000 one run dropped 22 600 of 300 000 lines;
/// at 20 000 (about 3 MiB of short lines) nothing dropped, flush p95 4.3 ms, max 0.44 s.
pub const BUFFER_CAP: usize = 20_000;
/// Plan D9: the text of a line the scrubber refused.
pub const WITHHELD: &str = "[line withheld: it matched a credential rule]";
/// The target of a line whose target the scrubber refused (R1 L5): the line ships under it.
pub const WITHHELD_TARGET: &str = "withheld";

/// The ship filter's predicate: the level read live, and the `sqlx` rule.
type ShipAdmit = Box<dyn Fn(&Metadata<'_>) -> bool + Send + Sync>;

/// The filter [`ShipHandle::layer`] puts in front of [`ShipLayer`].
pub type ShipFilter = FilterFn<ShipAdmit>;

/// What the layer and the flusher share.
struct Shared {
    /// [`LogShipLevel`]'s index in [`LogShipLevel::ALL`].
    level: AtomicU8,
    pending: Mutex<Pending>,
    /// Woken once [`BATCH_LINES`] lines wait.
    filled: Notify,
    /// Patterns only (blueprint F7e): the DSN is wiped at connect, so there is no value to mask.
    scrubber: MinimalScrubber,
    cap: usize,
}

/// The buffer behind the lock.
#[derive(Default)]
struct Pending {
    lines: VecDeque<NewWorkerLog>,
    /// Lines a flush took and has not settled: they count against the cap.
    in_flight: usize,
    /// Lines dropped at the cap and not yet reported.
    dropped: u64,
}

/// What one flush took from the buffer: the drop notice (when lines were dropped) first, then the
/// buffered lines oldest first. Settled by [`ShipHandle::shipped`] or [`ShipHandle::restore`].
#[derive(Debug)]
pub(crate) struct Taken {
    pub(crate) lines: Vec<NewWorkerLog>,
    /// How many drops the notice line reports (0: no notice).
    dropped: u64,
}

/// The shipper's control: the level, the buffer, and the layer that fills it. Cheap to clone.
#[derive(Clone)]
pub struct ShipHandle(Arc<Shared>);

impl ShipHandle {
    /// A handle at the default level ([`LogShipLevel::Warn`]) with an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::with_cap(BUFFER_CAP)
    }

    /// [`new`](Self::new) holding at most `cap` lines.
    pub(crate) fn with_cap(cap: usize) -> Self {
        Self(Arc::new(Shared {
            level: AtomicU8::new(level_index(LogShipLevel::default())),
            pending: Mutex::new(Pending::default()),
            filled: Notify::new(),
            scrubber: MinimalScrubber::new([]),
            cap,
        }))
    }

    /// The level the layer ships at now.
    #[must_use]
    pub fn level(&self) -> LogShipLevel {
        let index = usize::from(self.0.level.load(Ordering::Relaxed));
        LogShipLevel::ALL.get(index).copied().unwrap_or_default()
    }

    /// Ships at `level` from the next event on. A change rebuilds `tracing`'s callsite interest
    /// cache (plan D8, claim V2): a callsite first seen while its level was refused would
    /// otherwise stay refused.
    pub fn set_level(&self, level: LogShipLevel) {
        let old = self.0.level.swap(level_index(level), Ordering::Relaxed);
        if old != level_index(level) {
            tracing::callsite::rebuild_interest_cache();
        }
    }

    /// The layer, behind its own level filter (plan D8). The filter's hint is the static ceiling
    /// `debug`, so the `log` bridge's level set at init already admits what a later level ships
    /// (blueprint F7c).
    #[must_use]
    pub fn layer<S>(&self) -> Filtered<ShipLayer, ShipFilter, S>
    where
        S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        let shared = Arc::clone(&self.0);
        let admit: ShipAdmit = Box::new(move |meta: &Metadata<'_>| {
            admits(level_of(&shared.level), meta.level(), meta.target())
        });
        let filter = FilterFn::new(admit);
        ShipLayer {
            shared: Arc::clone(&self.0),
        }
        .with_filter(filter.with_max_level_hint(LevelFilter::DEBUG))
    }

    /// Resolves once [`BATCH_LINES`] lines wait (a wake stored while nobody waited counts).
    pub(crate) async fn filled(&self) {
        self.0.filled.notified().await;
    }

    /// Keeps `line`; at the cap drops the oldest line below `error`, or the oldest error line when
    /// only errors are held, or the newcomer when it is below `error` or nothing is held (every
    /// line in flight), and counts the drop (plan D3).
    pub(crate) fn push(&self, line: NewWorkerLog) {
        let waiting = {
            let mut pending = self.pending();
            if pending.lines.len() + pending.in_flight >= self.0.cap {
                pending.dropped += 1;
                if let Some(at) = pending
                    .lines
                    .iter()
                    .position(|held| held.level != WorkerLogLevel::Error)
                {
                    pending.lines.remove(at);
                } else if line.level != WorkerLogLevel::Error || pending.lines.is_empty() {
                    // Everything held is an error, so the newcomer is the oldest line below one;
                    // or nothing is held, so there is no older line to make room.
                    return;
                } else {
                    pending.lines.pop_front();
                }
            }
            pending.lines.push_back(line);
            pending.lines.len()
        };
        if waiting >= BATCH_LINES {
            self.0.filled.notify_one();
        }
    }

    /// Everything buffered that the current level ships, the drop notice first, or `None` when
    /// there is nothing to ship. A buffered line the level no longer ships (it was lowered, or set
    /// to `off`, after the line was kept) is discarded here, not shipped (R1 M1).
    pub(crate) fn take(&self) -> Option<Taken> {
        let level = self.level();
        let mut pending = self.pending();
        let dropped = std::mem::take(&mut pending.dropped);
        let mut lines = Vec::with_capacity(pending.lines.len() + 1);
        if dropped > 0 {
            lines.push(NewWorkerLog {
                logged_at: Utc::now(),
                level: WorkerLogLevel::Warn,
                target: module_path!().to_owned(),
                text: drop_notice(dropped),
            });
        }
        lines.extend(
            pending
                .lines
                .drain(..)
                .filter(|line| level.ships(line.level)),
        );
        if lines.is_empty() {
            return None;
        }
        pending.in_flight += lines.len() - usize::from(dropped > 0);
        Some(Taken { lines, dropped })
    }

    /// `taken` reached the store.
    pub(crate) fn shipped(&self, taken: &Taken) {
        let mut pending = self.pending();
        pending.in_flight = pending.in_flight.saturating_sub(held_lines(taken));
    }

    /// `taken` did not reach the store: its lines go back in front, in order, and its drops are
    /// counted again.
    pub(crate) fn restore(&self, taken: Taken) {
        let mut pending = self.pending();
        let held = held_lines(&taken);
        pending.in_flight = pending.in_flight.saturating_sub(held);
        pending.dropped += taken.dropped;
        let notice = usize::from(taken.dropped > 0);
        for line in taken.lines.into_iter().skip(notice).rev() {
            pending.lines.push_front(line);
        }
    }

    /// What is buffered now, oldest first.
    #[cfg(test)]
    pub(crate) fn buffered(&self) -> Vec<NewWorkerLog> {
        self.pending().lines.iter().cloned().collect()
    }

    /// Drops counted and not yet reported.
    #[cfg(test)]
    pub(crate) fn dropped(&self) -> u64 {
        self.pending().dropped
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.0
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for ShipHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ShipHandle {
    /// The level and the sizes, never a line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pending = self.pending();
        f.debug_struct("ShipHandle")
            .field("level", &self.level())
            .field("buffered", &pending.lines.len())
            .field("in_flight", &pending.in_flight)
            .field("dropped", &pending.dropped)
            .finish()
    }
}

/// How many of `taken`'s lines came from the buffer (the notice did not).
fn held_lines(taken: &Taken) -> usize {
    taken.lines.len() - usize::from(taken.dropped > 0)
}

/// The layer [`ShipHandle::layer`] filters: formats, scrubs and buffers each event it is given.
pub struct ShipLayer {
    shared: Arc<Shared>,
}

impl fmt::Debug for ShipLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShipLayer").finish_non_exhaustive()
    }
}

impl<S: Subscriber> Layer<S> for ShipLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let mut line = LineVisitor::default();
        // Formatted and scrubbed before the buffer's lock is taken (blueprint F7h).
        event.record(&mut line);
        // A record bridged from `log` reaches `on_event` under the target `log`; its real target
        // is a field (blueprint F7b), so the `sqlx` rule is applied again here.
        let target = line
            .log_target
            .take()
            .unwrap_or_else(|| meta.target().to_owned());
        if !admits(level_of(&self.shared.level), meta.level(), &target) {
            return;
        }
        let mut kept = NewWorkerLog {
            logged_at: Utc::now(),
            level: worker_level(*meta.level()),
            target: scrubbed(&self.shared.scrubber, target)
                .unwrap_or_else(|| WITHHELD_TARGET.to_owned()),
            text: scrubbed(&self.shared.scrubber, line.text())
                .unwrap_or_else(|| WITHHELD.to_owned()),
        };
        // Cut only after the scrub (R1 L1): cutting first could shorten a credential below a
        // rule's minimum and let part of it through.
        if kept.text.len() > WORKER_LOG_TEXT_MAX {
            kept.text = kept.stored_text().into_owned();
        }
        ShipHandle(Arc::clone(&self.shared)).push(kept);
    }
}

/// Whether a `level` event under `target` ships at `ship`: at or above the level, and never a
/// `sqlx` event below `warn` (plan D8, claim V18: at `debug` each flush would ship its own
/// statement log).
fn admits(ship: LogShipLevel, level: &Level, target: &str) -> bool {
    ship.ships(worker_level(*level)) && !(is_sqlx(target) && *level > Level::WARN)
}

/// `sqlx`, `sqlx::…` or `sqlx_…`.
fn is_sqlx(target: &str) -> bool {
    target == "sqlx" || target.starts_with("sqlx::") || target.starts_with("sqlx_")
}

const fn worker_level(level: Level) -> WorkerLogLevel {
    match level {
        Level::ERROR => WorkerLogLevel::Error,
        Level::WARN => WorkerLogLevel::Warn,
        Level::INFO => WorkerLogLevel::Info,
        Level::DEBUG => WorkerLogLevel::Debug,
        Level::TRACE => WorkerLogLevel::Trace,
    }
}

fn level_index(level: LogShipLevel) -> u8 {
    let index = LogShipLevel::ALL
        .iter()
        .position(|known| *known == level)
        .unwrap_or_default();
    u8::try_from(index).unwrap_or_default()
}

fn level_of(level: &AtomicU8) -> LogShipLevel {
    LogShipLevel::ALL
        .get(usize::from(level.load(Ordering::Relaxed)))
        .copied()
        .unwrap_or_default()
}

/// The drop notice for `dropped` lines (plan D3's words).
fn drop_notice(dropped: u64) -> String {
    if dropped == 1 {
        "1 log line was dropped while the database was unreachable".to_owned()
    } else {
        format!("{dropped} log lines were dropped while the database was unreachable")
    }
}

/// `text` as the scrubber leaves it, or `None` when a rule refuses it (plan D9; the shape of
/// `htui-agent`'s `scrubbed_text`): the caller puts [`WITHHELD`] or [`WITHHELD_TARGET`] in its
/// place. The refused text goes nowhere.
fn scrubbed(scrubber: &MinimalScrubber, text: String) -> Option<String> {
    let mut value = Value::String(text);
    match scrubber.scrub(&mut value) {
        Ok(()) => match value {
            Value::String(text) => Some(text),
            _ => None,
        },
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;

    /// A credential shape the pattern rules refuse (`github_token`), split so no scanner trips.
    fn token() -> String {
        format!("{}{}", "ghp_", "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6")
    }

    fn texts(handle: &ShipHandle) -> Vec<String> {
        handle
            .buffered()
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    fn captured(handle: &ShipHandle, emit: impl FnOnce()) {
        let subscriber = tracing_subscriber::registry().with(handle.layer());
        tracing::subscriber::with_default(subscriber, emit);
    }

    fn line(level: WorkerLogLevel, text: &str) -> NewWorkerLog {
        NewWorkerLog {
            logged_at: Utc::now(),
            level,
            target: "htui::test".to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn worker_log_lines_at_or_above_the_level_land_and_below_do_not() {
        let handle = ShipHandle::new();
        assert_eq!(handle.level(), LogShipLevel::Warn);
        captured(&handle, || {
            tracing::error!(target: "htui", "e");
            tracing::warn!(target: "htui", "w");
            tracing::info!(target: "htui", "i");
            tracing::debug!(target: "htui", "d");
        });
        assert_eq!(texts(&handle), ["e", "w"]);
        let levels: Vec<WorkerLogLevel> = handle.buffered().iter().map(|l| l.level).collect();
        assert_eq!(levels, [WorkerLogLevel::Error, WorkerLogLevel::Warn]);
        assert_eq!(handle.buffered()[0].target, "htui");

        let handle = ShipHandle::new();
        handle.set_level(LogShipLevel::Info);
        captured(&handle, || {
            tracing::info!(target: "htui", n = 3, "i");
            tracing::debug!(target: "htui", "d");
        });
        assert_eq!(texts(&handle), ["i n=3"]);
    }

    #[test]
    fn worker_log_off_ships_nothing() {
        let handle = ShipHandle::new();
        handle.set_level(LogShipLevel::Off);
        captured(&handle, || {
            tracing::error!(target: "htui", "e");
            tracing::warn!(target: "htui", "w");
        });
        assert!(handle.buffered().is_empty());
    }

    #[test]
    fn worker_log_sqlx_below_warn_is_ignored_and_its_warn_ships() {
        let handle = ShipHandle::new();
        handle.set_level(LogShipLevel::Debug);
        captured(&handle, || {
            tracing::debug!(target: "sqlx::query", "SELECT 1");
            tracing::info!(target: "sqlx_postgres::options", "connected");
            tracing::info!(target: "sqlx", "pool");
            // The `log` bridge's shape (M1 `buffer.rs`): the real target is a field.
            tracing::debug!(target: "log", { "log.target" = "sqlx::query", "log.module_path" = "sqlx" }, "bridged");
            tracing::warn!(target: "sqlx::query", "slow statement");
            tracing::warn!(target: "log", { "log.target" = "sqlx::query" }, "slow bridged");
            tracing::debug!(target: "sqlxish", "kept");
        });
        assert_eq!(texts(&handle), ["slow statement", "slow bridged", "kept"]);
        assert_eq!(handle.buffered()[1].target, "sqlx::query");
    }

    /// Set in the child [`worker_log_a_level_change_reaches_a_callsite_seen_while_refused`] runs.
    const GLOBAL_CHILD: &str = "HTUI_WORKER_LOG_GLOBAL_CHILD";

    /// Claim V2: a callsite first seen while its level was refused ships once the level admits it.
    /// Proved under a global subscriber, the worker's shape, where `tracing` caches the callsite's
    /// interest (a scoped dispatcher re-asks, so it passes even without `rebuild_interest_cache`).
    /// A global subscriber is once per process: the case runs in a child of this test binary.
    #[test]
    fn worker_log_a_level_change_reaches_a_callsite_seen_while_refused() {
        if std::env::var_os(GLOBAL_CHILD).is_some() {
            let handle = ShipHandle::new();
            tracing::subscriber::set_global_default(
                tracing_subscriber::registry().with(handle.layer()),
            )
            .expect("the child's only global subscriber");
            for round in 0..2 {
                if round == 1 {
                    handle.set_level(LogShipLevel::Info);
                }
                tracing::info!(target: "htui", "round {round}");
            }
            assert_eq!(texts(&handle), ["round 1"]);
            return;
        }
        let name = module_path!()
            .split_once("::")
            .map(|(_, tests)| {
                format!("{tests}::worker_log_a_level_change_reaches_a_callsite_seen_while_refused")
            })
            .expect("a module below the crate");
        let child = std::process::Command::new(std::env::current_exe().expect("this test binary"))
            .args(["--exact", &name, "--test-threads=1", "--nocapture"])
            .env(GLOBAL_CHILD, "1")
            .output()
            .expect("the child runs");
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        assert!(child.status.success(), "{said}");
        assert!(said.contains("1 passed"), "the child ran the case: {said}");
    }

    #[test]
    fn worker_log_the_cap_drops_the_oldest_non_error_then_an_error() {
        let handle = ShipHandle::with_cap(3);
        for (level, text) in [
            (WorkerLogLevel::Error, "e1"),
            (WorkerLogLevel::Info, "i1"),
            (WorkerLogLevel::Info, "i2"),
            (WorkerLogLevel::Info, "i3"),
        ] {
            handle.push(line(level, text));
        }
        assert_eq!(texts(&handle), ["e1", "i2", "i3"]);
        handle.push(line(WorkerLogLevel::Error, "e2"));
        handle.push(line(WorkerLogLevel::Error, "e3"));
        assert_eq!(texts(&handle), ["e1", "e2", "e3"]);
        // Only errors are held: the newcomer below error is the one dropped.
        handle.push(line(WorkerLogLevel::Info, "i4"));
        assert_eq!(texts(&handle), ["e1", "e2", "e3"]);
        handle.push(line(WorkerLogLevel::Error, "e4"));
        assert_eq!(texts(&handle), ["e2", "e3", "e4"]);
        assert_eq!(handle.dropped(), 5);

        let taken = handle.take().expect("lines wait");
        let shipped: Vec<(WorkerLogLevel, &str)> = taken
            .lines
            .iter()
            .map(|l| (l.level, l.text.as_str()))
            .collect();
        assert_eq!(
            shipped,
            [
                (
                    WorkerLogLevel::Warn,
                    "5 log lines were dropped while the database was unreachable"
                ),
                (WorkerLogLevel::Error, "e2"),
                (WorkerLogLevel::Error, "e3"),
                (WorkerLogLevel::Error, "e4"),
            ]
        );
        handle.shipped(&taken);
        assert_eq!(handle.dropped(), 0);
        assert!(handle.take().is_none(), "the notice ships once");
    }

    #[test]
    fn worker_log_a_failed_flush_keeps_its_lines_in_front_and_counts_against_the_cap() {
        let handle = ShipHandle::with_cap(4);
        handle.set_level(LogShipLevel::Info);
        handle.push(line(WorkerLogLevel::Info, "a"));
        handle.push(line(WorkerLogLevel::Info, "b"));
        let taken = handle.take().expect("lines wait");
        assert!(handle.buffered().is_empty());
        handle.push(line(WorkerLogLevel::Info, "c"));
        handle.push(line(WorkerLogLevel::Info, "d"));
        // Two in flight and two held: the cap is reached, so `c` (the oldest held) goes.
        handle.push(line(WorkerLogLevel::Info, "e"));
        assert_eq!(texts(&handle), ["d", "e"]);
        handle.restore(taken);
        assert_eq!(texts(&handle), ["a", "b", "d", "e"]);
        assert_eq!(handle.dropped(), 1);

        let taken = handle.take().expect("lines wait");
        assert_eq!(
            taken.lines[0].text,
            "1 log line was dropped while the database was unreachable"
        );
        handle.restore(taken);
        assert_eq!(texts(&handle), ["a", "b", "d", "e"]);
        assert_eq!(handle.dropped(), 1, "a failed notice is counted again");
    }

    #[test]
    fn worker_log_a_credential_line_is_withheld_with_its_level_and_target() {
        let handle = ShipHandle::new();
        let secret = token();
        captured(&handle, || {
            tracing::warn!(target: "htui::secrets", "token {secret} rejected");
            tracing::warn!(target: "htui", "plain");
        });
        let lines = handle.buffered();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, WITHHELD);
        assert_eq!(lines[0].level, WorkerLogLevel::Warn);
        assert_eq!(lines[0].target, "htui::secrets");
        assert_eq!(lines[1].text, "plain");
        assert!(!format!("{lines:?}").contains(&secret));
        assert!(!format!("{handle:?}").contains(&secret));
    }

    #[test]
    fn worker_log_a_full_batch_wakes_the_flusher() {
        let handle = ShipHandle::new();
        for n in 0..BATCH_LINES {
            handle.push(line(WorkerLogLevel::Info, &n.to_string()));
        }
        let woke = ready_now(handle.filled());
        assert!(woke, "{BATCH_LINES} lines store a wake");
    }

    /// R1 L4: the plan's wording, plural and singular.
    #[test]
    fn worker_log_the_drop_notice_and_the_withheld_text_use_the_plans_words() {
        assert_eq!(WITHHELD, "[line withheld: it matched a credential rule]");
        assert_eq!(
            drop_notice(2),
            "2 log lines were dropped while the database was unreachable"
        );
        assert_eq!(
            drop_notice(1),
            "1 log line was dropped while the database was unreachable"
        );
    }

    /// R1 M1: a line buffered at a level since lowered is not taken.
    #[test]
    fn worker_log_lines_the_level_no_longer_ships_are_not_taken() {
        let handle = ShipHandle::new();
        handle.push(line(WorkerLogLevel::Warn, "w"));
        handle.set_level(LogShipLevel::Off);
        assert!(handle.take().is_none(), "off ships nothing buffered");
        assert!(handle.buffered().is_empty());

        handle.set_level(LogShipLevel::Info);
        handle.push(line(WorkerLogLevel::Info, "i"));
        handle.push(line(WorkerLogLevel::Error, "e"));
        handle.set_level(LogShipLevel::Error);
        let taken = handle.take().expect("the error waits");
        let texts: Vec<&str> = taken.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["e"]);
        handle.shipped(&taken);
        assert!(format!("{handle:?}").contains("in_flight: 0"), "{handle:?}");
    }

    /// R1 L2: at the cap with every line in flight, an incoming error is the line dropped, and the
    /// buffer stays within the cap.
    #[test]
    fn worker_log_the_cap_holds_when_every_line_is_in_flight() {
        let handle = ShipHandle::with_cap(2);
        handle.push(line(WorkerLogLevel::Error, "e1"));
        handle.push(line(WorkerLogLevel::Error, "e2"));
        let taken = handle.take().expect("lines wait");
        handle.push(line(WorkerLogLevel::Error, "e3"));
        handle.push(line(WorkerLogLevel::Info, "i1"));
        assert!(handle.buffered().is_empty(), "{:?}", texts(&handle));
        assert_eq!(handle.dropped(), 2);
        handle.restore(taken);
        assert_eq!(texts(&handle), ["e1", "e2"]);
    }

    /// R1 L1: a huge field is held at most [`WORKER_LOG_TEXT_MAX`] bytes, with the stored `…`.
    #[test]
    fn worker_log_a_huge_line_is_held_capped() {
        let handle = ShipHandle::new();
        let huge = "x".repeat(1024 * 1024);
        captured(&handle, || tracing::warn!(target: "htui", "big {huge}"));
        let lines = handle.buffered();
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].text.len() <= WORKER_LOG_TEXT_MAX,
            "{}",
            lines[0].text.len()
        );
        assert!(lines[0].text.starts_with("big xxx"));
        assert!(lines[0].text.ends_with('…'));
    }

    /// R1 L5: a target the scrubber refuses, bridged or static, is kept as [`WITHHELD_TARGET`].
    #[test]
    fn worker_log_a_credential_target_is_withheld() {
        let handle = ShipHandle::new();
        let secret = token();
        captured(&handle, || {
            tracing::warn!(target: "log", { "log.target" = secret.as_str() }, "bridged");
            tracing::warn!(target: concat!("ghp_", "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6"), "static");
            tracing::warn!(target: "htui::plain", "plain");
        });
        let lines = handle.buffered();
        let targets: Vec<&str> = lines.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, [WITHHELD_TARGET, WITHHELD_TARGET, "htui::plain"]);
        assert_eq!(lines[0].text, "bridged");
        assert!(!format!("{lines:?}").contains(&secret));
    }

    /// Polls `future` once: whether it was ready.
    fn ready_now(future: impl Future<Output = ()>) -> bool {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        future.as_mut().poll(&mut cx).is_ready()
    }
}
