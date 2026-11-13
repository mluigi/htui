//! The flusher (MOD-122 milestone 2, plan D2, D3, D4, D7): the buffer to `worker_log`, the level
//! from the worker's own box row, and the retention sweep.
//!
//! The loop is generic over [`ShipSource`] so a fake store can drive it (blueprint F7f, the shape
//! of `concepts::IndexSource`); [`spawn_flusher`] is concrete over [`PgStore`], as
//! `concepts::spawn_index_job` is.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use htui_core::model::{BoxId, LogShipLevel};
use htui_core::store::{StoreError, WorkerLogStore};
use htui_store::PgStore;
use serde_json::Value;

use super::{BACKOFF_MAX, FLUSH_EVERY, ShipHandle};

/// Plan D4: the `app_setting` key holding how many days a worker log line is kept.
pub const RETENTION_KEY: &str = "log_retention_days";
/// Plan D4: the retention when [`RETENTION_KEY`] is missing or not a whole number in `1..=365`.
pub const DEFAULT_RETENTION_DAYS: u64 = 14;
/// Plan D4: the most rows one box keeps.
pub const PER_BOX_CAP: u32 = 100_000;
/// Plan D4: how often the sweep runs after the one at start.
pub const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);

/// What the flusher reads and writes: the worker log store, plus the worker's own box and two
/// reads that are inherent on [`PgStore`], not on a trait (blueprint F7f).
#[allow(async_fn_in_trait, reason = "no dyn ShipSource is formed")]
pub trait ShipSource: WorkerLogStore {
    /// The box this worker ships as.
    fn ship_box(&self) -> BoxId;
    /// The settings blob of [`ship_box`](Self::ship_box)'s row, `None` when the row is gone.
    async fn ship_box_settings(&self) -> Result<Option<Value>, StoreError>;
    /// The `app_setting` map, [`retention_days`]'s input.
    async fn ship_app_settings(&self) -> Result<BTreeMap<String, Value>, StoreError>;
}

impl ShipSource for PgStore {
    fn ship_box(&self) -> BoxId {
        self.this_box()
    }

    async fn ship_box_settings(&self) -> Result<Option<Value>, StoreError> {
        Ok(self.box_row(self.this_box()).await?.map(|row| row.settings))
    }

    async fn ship_app_settings(&self) -> Result<BTreeMap<String, Value>, StoreError> {
        self.app_settings().await
    }
}

/// Plan D4: [`RETENTION_KEY`] in `app`, re-read each sweep; absent, `null`, a string, a fraction,
/// zero, a negative and anything above 365 all mean [`DEFAULT_RETENTION_DAYS`].
#[must_use]
pub fn retention_days(app: &BTreeMap<String, Value>) -> u64 {
    app.get(RETENTION_KEY)
        .and_then(Value::as_u64)
        .filter(|days| (1..=365).contains(days))
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

/// The flusher's state between cycles.
pub(crate) struct Flusher<S> {
    store: S,
    handle: ShipHandle,
    /// Failed flushes in a row; 0 while shipping works.
    failures: u32,
    /// When the next sweep is due; `None` until the first one succeeded (so it runs at start).
    next_sweep: Option<DateTime<Utc>>,
}

impl<S: ShipSource> Flusher<S> {
    pub(crate) const fn new(store: S, handle: ShipHandle) -> Self {
        Self {
            store,
            handle,
            failures: 0,
            next_sweep: None,
        }
    }

    /// Whether the last flush reached the store.
    pub(crate) const fn healthy(&self) -> bool {
        self.failures == 0
    }

    /// One timed cycle at `now`: the level from the box row (plan D7), the sweep when due (D4),
    /// then a flush. Returns the pause before the next cycle: [`FLUSH_EVERY`] while shipping
    /// works, doubling per failure up to [`BACKOFF_MAX`] (D3).
    pub(crate) async fn cycle(&mut self, now: DateTime<Utc>) -> Duration {
        // A failed read or a missing row keeps the level the worker has.
        if let Ok(Some(settings)) = self.store.ship_box_settings().await {
            self.handle.set_level(LogShipLevel::of(&settings));
        }
        if self.next_sweep.is_none_or(|due| now >= due) {
            self.sweep(now).await;
        }
        self.flush().await;
        pause_after(self.failures)
    }

    /// Plan D4: rows older than the retention, then beyond [`PER_BOX_CAP`] per box. Quiet: a
    /// failure leaves the sweep due, so the next cycle tries again.
    async fn sweep(&mut self, now: DateTime<Utc>) {
        let Ok(app) = self.store.ship_app_settings().await else {
            return;
        };
        // At most 365, so the conversion never falls back.
        let days = i64::try_from(retention_days(&app)).unwrap_or(365);
        let older_than = now - TimeDelta::days(days);
        if self
            .store
            .prune_worker_logs(older_than, PER_BOX_CAP)
            .await
            .is_ok()
        {
            self.next_sweep = Some(now + TimeDelta::from_std(SWEEP_EVERY).unwrap_or_default());
        }
    }

    /// Ships everything buffered in one call; a failure keeps the lines for the next cycle. Logs
    /// only on a change of state (plan D8).
    pub(crate) async fn flush(&mut self) {
        let Some(taken) = self.handle.take() else {
            return;
        };
        match self
            .store
            .append_worker_logs(self.store.ship_box(), &taken.lines)
            .await
        {
            Ok(_) => {
                self.handle.shipped(&taken);
                if self.failures > 0 {
                    tracing::info!(
                        failed = self.failures,
                        "worker log shipping recovered; the kept lines were shipped"
                    );
                }
                self.failures = 0;
            }
            Err(err) => {
                // The buffer's lock is not held here: the line below goes through the ship layer.
                self.handle.restore(taken);
                if self.failures == 0 {
                    tracing::warn!(
                        error = %err,
                        "worker log shipping failed; lines are kept and retried"
                    );
                }
                self.failures = self.failures.saturating_add(1);
            }
        }
    }
}

/// The back-off after `failures` failed flushes in a row.
fn pause_after(failures: u32) -> Duration {
    let Some(doublings) = failures.checked_sub(1) else {
        return FLUSH_EVERY;
    };
    FLUSH_EVERY
        .checked_mul(1 << doublings.min(5))
        .map_or(BACKOFF_MAX, |pause| pause.min(BACKOFF_MAX))
}

/// The flusher over `store` until aborted: a cycle every [`FLUSH_EVERY`] (or back-off), and a
/// flush in between whenever [`BATCH_LINES`](super::BATCH_LINES) lines wait while shipping works.
pub async fn run_flusher<S: ShipSource>(store: S, handle: ShipHandle) -> Infallible {
    let mut flusher = Flusher::new(store, handle.clone());
    loop {
        let pause = flusher.cycle(Utc::now()).await;
        let deadline = tokio::time::Instant::now() + pause;
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                () = handle.filled(), if flusher.healthy() => flusher.flush().await,
            }
        }
    }
}

/// The worker's flusher over `pg` (plan D2): spawned beside the index job, sharing the pool,
/// aborted at shutdown.
#[must_use]
pub fn spawn_flusher(pg: PgStore, handle: ShipHandle) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { match run_flusher(pg, handle).await {} })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use htui_core::model::{NewWorkerLog, WorkerLogLevel, WorkerLogLine};
    use htui_core::store::Result;
    use serde_json::json;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;
    use crate::worker_log::WITHHELD;

    /// What the fake store holds and was asked.
    #[derive(Default)]
    struct State {
        failing: bool,
        written: Vec<NewWorkerLog>,
        appends: usize,
        settings: Option<Value>,
        app: BTreeMap<String, Value>,
        sweeps: Vec<(DateTime<Utc>, u32)>,
    }

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<State>>);

    impl Fake {
        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn written(&self) -> Vec<String> {
            self.state()
                .written
                .iter()
                .map(|l| l.text.clone())
                .collect()
        }
    }

    fn down() -> StoreError {
        StoreError::Unreachable("the database is down".to_owned())
    }

    impl WorkerLogStore for Fake {
        async fn append_worker_logs(&self, _: BoxId, lines: &[NewWorkerLog]) -> Result<u64> {
            let mut state = self.state();
            state.appends += 1;
            if state.failing {
                return Err(down());
            }
            state.written.extend_from_slice(lines);
            Ok(lines.len() as u64)
        }

        async fn worker_logs(&self, _: Option<i64>, _: u32) -> Result<Vec<WorkerLogLine>> {
            Ok(Vec::new())
        }

        async fn prune_worker_logs(&self, older_than: DateTime<Utc>, cap: u32) -> Result<u64> {
            let mut state = self.state();
            if state.failing {
                return Err(down());
            }
            state.sweeps.push((older_than, cap));
            Ok(0)
        }

        async fn set_box_log_level(&self, _: BoxId, _: Option<LogShipLevel>) -> Result<bool> {
            Ok(true)
        }
    }

    impl ShipSource for Fake {
        fn ship_box(&self) -> BoxId {
            htui_core::fixtures::ids::BOX
        }

        async fn ship_box_settings(&self) -> Result<Option<Value>> {
            let state = self.state();
            if state.failing {
                return Err(down());
            }
            Ok(state.settings.clone())
        }

        async fn ship_app_settings(&self) -> Result<BTreeMap<String, Value>> {
            let state = self.state();
            if state.failing {
                return Err(down());
            }
            Ok(state.app.clone())
        }
    }

    fn line(text: &str) -> NewWorkerLog {
        NewWorkerLog {
            logged_at: Utc::now(),
            level: WorkerLogLevel::Info,
            target: "htui::test".to_owned(),
            text: text.to_owned(),
        }
    }

    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    /// The flusher's own events, as `(level, message)`.
    #[derive(Clone, Default)]
    struct Said(Arc<Mutex<Vec<(tracing::Level, String)>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Said {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut line = crate::logging::buffer::LineVisitor::default();
            event.record(&mut line);
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((*event.metadata().level(), line.text()));
        }
    }

    impl Said {
        fn lines(&self) -> Vec<(tracing::Level, String)> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    #[tokio::test]
    async fn worker_log_a_failed_flush_backs_off_and_the_recovery_ships_the_backlog_in_order() {
        let fake = Fake::default();
        let handle = ShipHandle::new();
        let mut flusher = Flusher::new(fake.clone(), handle.clone());
        handle.push(line("a"));
        assert_eq!(flusher.cycle(t0()).await, FLUSH_EVERY);
        assert_eq!(fake.written(), ["a"]);

        fake.state().failing = true;
        handle.push(line("b"));
        let mut pauses = Vec::new();
        for n in 0..6 {
            handle.push(line(&format!("c{n}")));
            pauses.push(flusher.cycle(t0()).await.as_secs());
        }
        assert_eq!(pauses, [2, 4, 8, 16, 30, 30]);
        assert!(!flusher.healthy());
        assert_eq!(fake.written(), ["a"], "nothing more was written");
        assert_eq!(handle.buffered().len(), 7, "the failed lines are kept");

        fake.state().failing = false;
        assert_eq!(flusher.cycle(t0()).await, FLUSH_EVERY);
        assert!(flusher.healthy());
        assert_eq!(
            fake.written(),
            ["a", "b", "c0", "c1", "c2", "c3", "c4", "c5"]
        );
        assert!(handle.buffered().is_empty());
    }

    #[tokio::test]
    async fn worker_log_drops_ship_as_one_warn_line_on_the_next_flush() {
        let fake = Fake::default();
        fake.state().failing = true;
        let handle = ShipHandle::with_cap(2);
        let mut flusher = Flusher::new(fake.clone(), handle.clone());
        for text in ["a", "b", "c", "d"] {
            handle.push(line(text));
        }
        flusher.flush().await;
        fake.state().failing = false;
        flusher.flush().await;
        let state = fake.state();
        let shipped: Vec<(WorkerLogLevel, &str)> = state
            .written
            .iter()
            .map(|l| (l.level, l.text.as_str()))
            .collect();
        assert_eq!(
            shipped,
            [
                (
                    WorkerLogLevel::Warn,
                    "2 log lines dropped while database unreachable"
                ),
                (WorkerLogLevel::Info, "c"),
                (WorkerLogLevel::Info, "d"),
            ]
        );
    }

    #[tokio::test]
    async fn worker_log_the_level_comes_from_the_box_row_each_cycle() {
        let fake = Fake::default();
        let handle = ShipHandle::new();
        let mut flusher = Flusher::new(fake.clone(), handle.clone());
        fake.state().settings = Some(json!({ "log_level": "debug", "executor": "x" }));
        flusher.cycle(t0()).await;
        assert_eq!(handle.level(), LogShipLevel::Debug);
        fake.state().settings = Some(json!({ "log_level": "error" }));
        flusher.cycle(t0()).await;
        assert_eq!(handle.level(), LogShipLevel::Error);
        // Unknown or missing reads as the default, like `executor`.
        fake.state().settings = Some(json!({ "log_level": "loud" }));
        flusher.cycle(t0()).await;
        assert_eq!(handle.level(), LogShipLevel::Warn);
        handle.set_level(LogShipLevel::Info);
        // A failed read or a missing row keeps the level the worker has.
        fake.state().settings = None;
        flusher.cycle(t0()).await;
        assert_eq!(handle.level(), LogShipLevel::Info);
        fake.state().failing = true;
        flusher.cycle(t0()).await;
        assert_eq!(handle.level(), LogShipLevel::Info);
    }

    #[tokio::test]
    async fn worker_log_the_shipper_logs_only_on_transitions() {
        let fake = Fake::default();
        let handle = ShipHandle::new();
        let mut flusher = Flusher::new(fake.clone(), handle.clone());
        let said = Said::default();
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(said.clone()));
        fake.state().failing = true;
        for n in 0..10 {
            handle.push(line(&n.to_string()));
            flusher.cycle(t0()).await;
        }
        assert_eq!(fake.state().appends, 10, "ten flushes failed");
        let lines = said.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].0, tracing::Level::WARN);

        fake.state().failing = false;
        for _ in 0..3 {
            handle.push(line("more"));
            flusher.cycle(t0()).await;
        }
        let lines = said.lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1].0, tracing::Level::INFO);
    }

    #[tokio::test]
    async fn worker_log_the_sweep_runs_at_start_and_hourly_with_the_retention_setting() {
        let fake = Fake::default();
        let handle = ShipHandle::new();
        let mut flusher = Flusher::new(fake.clone(), handle);
        flusher.cycle(t0()).await;
        flusher.cycle(t0() + TimeDelta::minutes(30)).await;
        assert_eq!(
            fake.state().sweeps,
            [(t0() - TimeDelta::days(14), PER_BOX_CAP)],
            "at start, then not before the hour"
        );
        fake.state().app.insert(RETENTION_KEY.to_owned(), json!(3));
        let hour = t0() + TimeDelta::hours(1);
        flusher.cycle(hour).await;
        assert_eq!(
            fake.state().sweeps[1],
            (hour - TimeDelta::days(3), PER_BOX_CAP)
        );
    }

    #[tokio::test]
    async fn worker_log_a_failed_sweep_is_retried_next_cycle() {
        let fake = Fake::default();
        fake.state().failing = true;
        let mut flusher = Flusher::new(fake.clone(), ShipHandle::new());
        flusher.cycle(t0()).await;
        fake.state().failing = false;
        flusher.cycle(t0() + TimeDelta::seconds(2)).await;
        assert_eq!(fake.state().sweeps.len(), 1);
    }

    #[test]
    fn worker_log_retention_days_reads_a_whole_number_in_range() {
        let of =
            |value: Value| retention_days(&BTreeMap::from([(RETENTION_KEY.to_owned(), value)]));
        assert_eq!(retention_days(&BTreeMap::new()), DEFAULT_RETENTION_DAYS);
        assert_eq!(of(json!(1)), 1);
        assert_eq!(of(json!(365)), 365);
        for bad in [
            json!(0),
            json!(366),
            json!(-3),
            json!(2.5),
            json!("7"),
            json!(null),
        ] {
            assert_eq!(of(bad.clone()), DEFAULT_RETENTION_DAYS, "{bad}");
        }
    }

    /// Plan D9: a refused line's text is in neither the buffer, the store nor what the shipper
    /// logs.
    #[tokio::test]
    async fn worker_log_a_withheld_line_reaches_nothing_with_its_text() {
        let secret = format!("{}{}", "ghp_", "Z9y8X7w6V5u4T3s2R1q0P9o8N7m6L5k4");
        let fake = Fake::default();
        let handle = ShipHandle::new();
        tracing::subscriber::with_default(
            tracing_subscriber::registry().with(handle.layer()),
            || tracing::error!(target: "htui::worker", "login with {secret} failed"),
        );
        assert!(!format!("{:?}", handle.buffered()).contains(&secret));
        let said = Said::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry()
                .with(said.clone())
                .with(handle.layer()),
        );
        let mut flusher = Flusher::new(fake.clone(), handle.clone());
        fake.state().failing = true;
        flusher.cycle(t0()).await;
        fake.state().failing = false;
        flusher.cycle(t0()).await;
        let state = fake.state();
        assert_eq!(state.written[0].text, WITHHELD);
        assert_eq!(state.written[0].level, WorkerLogLevel::Error);
        assert_eq!(state.written[0].target, "htui::worker");
        assert!(!format!("{:?}", state.written).contains(&secret));
        assert!(!format!("{:?}", said.lines()).contains(&secret));
    }
}
