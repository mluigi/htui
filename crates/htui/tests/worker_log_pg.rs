//! MOD-122 milestone 2 against live Postgres (plan D2, D7, T2): the worker's ship layer and
//! flusher over a real `PgStore`, read back through `Backend::worker_logs` as the TUI reads it.
//!
//! The decisions (level, cap, back-off, scrub, sweep) are pinned over a fake store in
//! `worker_log` itself; what only this file can prove is that a line the layer kept is a row the
//! TUI's read returns, within one flush interval and one poll, and that a level written through
//! `set_box_log_level` is the one the running flusher ships at.
//!
//! The `#[ignore]` case is the plan D2 measurement: 10 000 `info` lines a second for 30 seconds
//! through the real layer and flusher, with a `claim_run`-shaped query running beside it. Run it
//! with `--ignored --nocapture`.
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset.
#![cfg(feature = "testkit")]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use htui::worker_log::{FLUSH_EVERY, ShipHandle, ShipSource, run_flusher, spawn_flusher};
use htui_core::model::{BoxId, LogShipLevel, NewWorkerLog, WorkerLogLevel, WorkerLogLine};
use htui_core::store::{Result, StoreError, WorkerLogStore};
use htui_store::testkit::{self, TestDb};
use htui_store::{Backend, CacheStore, PgStore};
use serde_json::Value;
use tracing_subscriber::layer::SubscriberExt as _;

/// How often a case re-reads, and the slack the "one poll" stands for.
const POLL: Duration = Duration::from_millis(100);
const ONE_POLL: Duration = Duration::from_secs(1);

/// A demo database, a throwaway mirror and `Backend::Online` over the two.
struct Stack {
    db: TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    backend: Backend,
}

impl Stack {
    async fn new() -> Option<Self> {
        let db = testkit::demo_db().await?;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), "worker-log-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        Some(Self {
            db,
            _root: root,
            cache,
            backend,
        })
    }

    /// Re-reads through the TUI's read until `found` holds or `within` passed.
    async fn wait_for(
        &self,
        within: Duration,
        found: impl Fn(&[WorkerLogLine]) -> bool,
    ) -> Vec<WorkerLogLine> {
        let started = Instant::now();
        loop {
            let lines = self
                .backend
                .worker_logs(None, 2_000)
                .await
                .expect("the worker log reads");
            if found(&lines) || started.elapsed() > within {
                return lines;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    async fn drop_db(self) {
        self.cache.close().await;
        self.db.drop_db().await;
    }
}

fn texts(lines: &[WorkerLogLine]) -> Vec<&str> {
    lines.iter().map(|line| line.text.as_str()).collect()
}

/// Plan D2: a `warn` line the layer kept is readable through `Backend::worker_logs` within one
/// flush interval and one poll, stamped with this box.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_warn_line_is_read_back_within_one_flush() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let handle = ShipHandle::new();
    {
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(handle.layer()));
        tracing::info!(target: "htui::worker", "below the default level");
        tracing::warn!(target: "htui::worker", "the disk is nearly full");
    }
    let started = Instant::now();
    let flusher = spawn_flusher(stack.db.store.clone(), handle.clone());
    let lines = stack
        .wait_for(FLUSH_EVERY + ONE_POLL, |lines| !lines.is_empty())
        .await;
    let waited = started.elapsed();
    flusher.abort();

    assert_eq!(
        texts(&lines),
        ["the disk is nearly full"],
        "after {waited:?}"
    );
    let this_box = stack.db.store.this_box();
    let hostname = stack
        .db
        .store
        .box_row(this_box)
        .await
        .expect("the box reads")
        .expect("this box exists")
        .hostname;
    assert_eq!(lines[0].box_id, Some(this_box));
    assert_eq!(lines[0].box_name, hostname);
    assert_eq!(lines[0].level, WorkerLogLevel::Warn);
    assert_eq!(lines[0].target, "htui::worker");
    stack.drop_db().await;
}

/// Plan D7: a level written through `set_box_log_level` is picked up by the running flusher, and
/// the next `debug` line ships.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_level_set_on_the_box_row_is_picked_up_by_the_running_flusher() {
    let Some(stack) = Stack::new().await else {
        return;
    };
    let handle = ShipHandle::new();
    let flusher = spawn_flusher(stack.db.store.clone(), handle.clone());
    let this_box = stack.db.store.this_box();
    assert!(
        stack
            .db
            .store
            .set_box_log_level(this_box, Some(LogShipLevel::Debug))
            .await
            .expect("the level writes")
    );
    let started = Instant::now();
    while handle.level() != LogShipLevel::Debug && started.elapsed() < FLUSH_EVERY + ONE_POLL {
        tokio::time::sleep(POLL).await;
    }
    assert_eq!(
        handle.level(),
        LogShipLevel::Debug,
        "after {:?}",
        started.elapsed()
    );
    {
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(handle.layer()));
        tracing::debug!(target: "htui::worker", "a debug line");
    }
    let lines = stack
        .wait_for(FLUSH_EVERY + ONE_POLL, |lines| !lines.is_empty())
        .await;
    assert_eq!(texts(&lines), ["a debug line"]);
    assert_eq!(lines[0].level, WorkerLogLevel::Debug);

    stack
        .db
        .store
        .set_box_log_level(this_box, None)
        .await
        .expect("the level clears");
    let started = Instant::now();
    while handle.level() != LogShipLevel::Warn && started.elapsed() < FLUSH_EVERY + ONE_POLL {
        tokio::time::sleep(POLL).await;
    }
    assert_eq!(
        handle.level(),
        LogShipLevel::Warn,
        "a cleared level is the default"
    );
    flusher.abort();
    stack.drop_db().await;
}

// ---------------------------------------------------------------------------------------------
// The plan D2 measurement
// ---------------------------------------------------------------------------------------------

/// `PgStore` with each append timed.
#[derive(Clone)]
struct Timed {
    pg: PgStore,
    appends: Arc<Mutex<Vec<Duration>>>,
}

impl WorkerLogStore for Timed {
    async fn append_worker_logs(&self, box_id: BoxId, lines: &[NewWorkerLog]) -> Result<u64> {
        let started = Instant::now();
        let written = self.pg.append_worker_logs(box_id, lines).await;
        self.appends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(started.elapsed());
        written
    }

    async fn worker_logs(&self, after: Option<i64>, limit: u32) -> Result<Vec<WorkerLogLine>> {
        self.pg.worker_logs(after, limit).await
    }

    async fn prune_worker_logs(&self, older_than: DateTime<Utc>, cap: u32) -> Result<u64> {
        self.pg.prune_worker_logs(older_than, cap).await
    }

    async fn set_box_log_level(&self, box_id: BoxId, level: Option<LogShipLevel>) -> Result<bool> {
        self.pg.set_box_log_level(box_id, level).await
    }
}

impl ShipSource for Timed {
    fn ship_box(&self) -> BoxId {
        self.pg.ship_box()
    }

    async fn ship_box_settings(&self) -> std::result::Result<Option<Value>, StoreError> {
        self.pg.ship_box_settings().await
    }

    async fn ship_app_settings(&self) -> std::result::Result<BTreeMap<String, Value>, StoreError> {
        self.pg.ship_app_settings().await
    }
}

fn percentile(samples: &mut [Duration], pct: usize) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    samples.sort_unstable();
    samples[(samples.len() * pct / 100).min(samples.len() - 1)]
}

/// One `claim_run`-shaped statement: a transaction locking the oldest candidate run
/// `FOR UPDATE SKIP LOCKED`, timed. Runs until `stop`.
async fn claim_shaped(pool: sqlx::PgPool, stop: Arc<AtomicBool>) -> Vec<Duration> {
    let mut samples = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        let mut tx = pool.begin().await.expect("a transaction");
        let _: Option<(uuid::Uuid,)> = sqlx::query_as(
            "SELECT id FROM run \
              WHERE status IN ('queued', 'running') \
              ORDER BY queued_at, id \
              LIMIT 1 \
                FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await
        .expect("the claim-shaped read");
        tx.commit().await.expect("the commit");
        samples.push(started.elapsed());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    samples
}

/// Plan D2 measurement: 10 000 `info` lines a second for 30 s through the real layer and flusher.
/// Reports lines shipped and dropped, the p95 flush time, and the claim-shaped query's p95 before
/// and during the load. The defaults stand while nothing drops and the p95 flush stays under
/// 200 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a 40 s load measurement; run once with --ignored --nocapture"]
async fn measure_ten_thousand_lines_a_second_for_thirty_seconds() {
    const RATE: u64 = 10_000;
    const SECONDS: u64 = 30;
    const TICK: Duration = Duration::from_millis(10);
    let Some(stack) = Stack::new().await else {
        return;
    };
    let pg = stack.db.store.clone();
    pg.set_box_log_level(pg.this_box(), Some(LogShipLevel::Info))
        .await
        .expect("the level writes");

    // The claim-shaped query alone, for the baseline.
    let stop = Arc::new(AtomicBool::new(false));
    let baseline = tokio::spawn(claim_shaped(stack.db.pool.clone(), Arc::clone(&stop)));
    tokio::time::sleep(Duration::from_secs(5)).await;
    stop.store(true, Ordering::Relaxed);
    let mut baseline = baseline.await.expect("the baseline ran");

    let handle = ShipHandle::new();
    handle.set_level(LogShipLevel::Info);
    let appends = Arc::new(Mutex::new(Vec::new()));
    let flusher = tokio::spawn(run_flusher(
        Timed {
            pg: pg.clone(),
            appends: Arc::clone(&appends),
        },
        handle.clone(),
    ));
    let stop = Arc::new(AtomicBool::new(false));
    let loaded = tokio::spawn(claim_shaped(stack.db.pool.clone(), Arc::clone(&stop)));

    let emitted = Arc::new(AtomicU64::new(0));
    let producer = {
        let handle = handle.clone();
        let emitted = Arc::clone(&emitted);
        std::thread::spawn(move || {
            let subscriber = tracing_subscriber::registry().with(handle.layer());
            tracing::subscriber::with_default(subscriber, || {
                let per_tick = RATE * u64::try_from(TICK.as_millis()).expect("ms") / 1_000;
                let started = Instant::now();
                let mut next = started;
                let mut n: u64 = 0;
                while started.elapsed() < Duration::from_secs(SECONDS) {
                    for _ in 0..per_tick {
                        tracing::info!(target: "htui::load", n, "load line with some ordinary text");
                        n += 1;
                    }
                    emitted.store(n, Ordering::Relaxed);
                    next += TICK;
                    if let Some(rest) = next.checked_duration_since(Instant::now()) {
                        std::thread::sleep(rest);
                    }
                }
            });
        })
    };
    tokio::task::spawn_blocking(move || producer.join().expect("the producer ran"))
        .await
        .expect("the producer joined");
    stop.store(true, Ordering::Relaxed);
    let mut loaded = loaded.await.expect("the loaded query ran");

    // Drain: wait until the count stops moving.
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM worker_log WHERE target = 'htui::load'")
            .fetch_one(&stack.db.pool)
            .await
            .expect("the count")
    };
    let mut shipped = count().await;
    loop {
        tokio::time::sleep(FLUSH_EVERY + POLL).await;
        let now = count().await;
        if now == shipped {
            break;
        }
        shipped = now;
    }
    flusher.abort();
    let notices: Vec<String> = sqlx::query_scalar(
        "SELECT text FROM worker_log WHERE text LIKE '% log lines dropped while database unreachable'",
    )
    .fetch_all(&stack.db.pool)
    .await
    .expect("the notices");
    let reported: u64 = notices
        .iter()
        .filter_map(|text| text.split(' ').next()?.parse::<u64>().ok())
        .sum();
    let emitted = emitted.load(Ordering::Relaxed);
    let mut flushes = appends
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let flush_count = flushes.len();
    // Where the slowest flushes fell (index, time): a stall at start reads differently from one
    // under load.
    let mut slowest: Vec<(usize, Duration)> = flushes.iter().copied().enumerate().collect();
    slowest.sort_by_key(|(_, took)| std::cmp::Reverse(*took));
    slowest.truncate(5);
    println!("MEASURE slowest flushes {slowest:?}");
    println!(
        "MEASURE emitted={emitted} shipped={shipped} dropped={} reported_dropped={reported} \
         flushes={flush_count} flush_p50={:?} flush_p95={:?} flush_max={:?} \
         claim_baseline_p50={:?} claim_baseline_p95={:?} claim_loaded_p50={:?} \
         claim_loaded_p95={:?} claim_loaded_max={:?} (samples {} / {})",
        emitted.saturating_sub(u64::try_from(shipped).unwrap_or_default()),
        percentile(&mut flushes, 50),
        percentile(&mut flushes, 95),
        flushes.last().copied().unwrap_or_default(),
        percentile(&mut baseline, 50),
        percentile(&mut baseline, 95),
        percentile(&mut loaded, 50),
        percentile(&mut loaded, 95),
        loaded.last().copied().unwrap_or_default(),
        baseline.len(),
        loaded.len(),
    );
    stack.drop_db().await;
}
