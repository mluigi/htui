//! ANA-2 §4.9's lease heartbeat and the sweep's pure half (plan D85, D86, D90, D91, D97).
//!
//! Nothing here reads or writes a store. The heartbeat takes its refresh as a closure (plan D106),
//! so the engine hands it `|until| store.refresh_lease(run, owner, until)` and a test hands it a
//! script; everything else is a function of rows the caller already read.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use htui_core::store::StoreError;
use serde_json::Value;

use crate::isolate::Clock;

/// `app_setting` key of the lease's time to live, in seconds (`0003_orchestration.sql:135`).
pub const LEASE_TTL_KEY: &str = "lease_ttl_seconds";

/// `app_setting` key of the heartbeat's interval, in seconds (`0003_orchestration.sql:136`).
pub const LEASE_REFRESH_KEY: &str = "lease_refresh_seconds";

/// The TTL when [`LEASE_TTL_KEY`] is absent or not a positive integer: ANA-2 §10's 120 s (PRD D2).
pub const DEFAULT_LEASE_TTL_SECONDS: i64 = 120;

/// The interval when [`LEASE_REFRESH_KEY`] is absent or not a positive integer: §10's 60 s.
pub const DEFAULT_LEASE_REFRESH_SECONDS: i64 = 60;

/// How long a lease lives and how often it is renewed (plan D85).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseTimes {
    /// Added to the clock's instant for every `lease_expires_at` this process writes.
    pub ttl: TimeDelta,
    /// How long the heartbeat sleeps between two refreshes; always shorter than `ttl`.
    pub refresh: Duration,
}

impl LeaseTimes {
    /// Positive `i64` per key, else the default; then `refresh >= ttl` → `ttl / 2`, in
    /// **milliseconds** so a 1 s TTL still beats at 500 ms.
    ///
    /// A refresh at or above the TTL would let a live lease expire between two beats, and another
    /// box's sweep would adopt a run this process is still walking.
    #[must_use]
    pub fn from_app(app: &BTreeMap<String, Value>) -> Self {
        // A TTL too large for a `TimeDelta` (or for its half in milliseconds) is read as silence.
        let ttl_seconds = app_positive(app, LEASE_TTL_KEY)
            .filter(|seconds| TimeDelta::try_seconds(*seconds).is_some())
            .unwrap_or(DEFAULT_LEASE_TTL_SECONDS);
        let refresh_seconds =
            app_positive(app, LEASE_REFRESH_KEY).unwrap_or(DEFAULT_LEASE_REFRESH_SECONDS);
        let ttl = TimeDelta::seconds(ttl_seconds);
        let refresh = if refresh_seconds >= ttl_seconds {
            Duration::from_millis(ttl.num_milliseconds().unsigned_abs() / 2)
        } else {
            Duration::from_secs(refresh_seconds.unsigned_abs())
        };
        Self { ttl, refresh }
    }
}

/// One `app_setting` value as a positive `i64`: `graph.rs`'s private rule, restated here rather
/// than shared (blueprint F-U). Absent, `null`, a string, a bool, zero and a negative all mean
/// "not set", so a planted `0` falls back to the default rather than pinning the lease to nothing.
fn app_positive(app: &BTreeMap<String, Value>, key: &str) -> Option<i64> {
    app.get(key).and_then(Value::as_i64).filter(|n| *n > 0)
}

/// Why [`heartbeat`] returned. It has one answer, because it never returns otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    /// A refresh touched zero rows: another orchestrator took the lease, and ANA-2 `:1280-1282`
    /// has this one abandon the run without writing further.
    Abandoned,
}

/// Sleep `times.refresh` (`tokio::time::sleep`), then `refresh(clock.now() + times.ttl)`:
/// `Ok(true)` → loop; `Ok(false)` → `Abandoned`; `Err(e)` → `tracing::warn!` and loop (D86, D103).
/// Never returns otherwise.
///
/// A store error is not a taken lease: offline, the lease is left to expire and the reconnect
/// sweep adjudicates (`docs/ANA-2.md:1325-1336`). Needs a runtime with the time driver (H-3).
pub async fn heartbeat<F, Fut, C>(mut refresh: F, clock: &C, times: LeaseTimes) -> Heartbeat
where
    F: FnMut(DateTime<Utc>) -> Fut,
    Fut: Future<Output = Result<bool, StoreError>>,
    C: Clock + ?Sized,
{
    loop {
        tokio::time::sleep(times.refresh).await;
        match refresh(clock.now() + times.ttl).await {
            Ok(true) => {}
            Ok(false) => return Heartbeat::Abandoned,
            Err(error) => {
                tracing::warn!(%error, "lease refresh failed; beating again at the next interval");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
    use htui_core::store::StoreError;
    use serde_json::{Value, json};

    use crate::isolate::Clock;
    use crate::recover::{Heartbeat, LeaseTimes, heartbeat};

    // -----------------------------------------------------------------------------------------
    // Lease times
    // -----------------------------------------------------------------------------------------

    fn app(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn lease_times_read_the_two_app_settings() {
        let times = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(300)),
            ("lease_refresh_seconds", json!(90)),
        ]));
        assert_eq!(times.ttl, TimeDelta::seconds(300));
        assert_eq!(times.refresh, Duration::from_secs(90));
    }

    #[test]
    fn lease_times_fall_back_to_120_and_60() {
        for silent in [None, Some(json!(0)), Some(json!(-5)), Some(json!("120"))] {
            let settings = silent.map_or_else(BTreeMap::new, |value| {
                app(&[
                    ("lease_ttl_seconds", value.clone()),
                    ("lease_refresh_seconds", value),
                ])
            });
            let times = LeaseTimes::from_app(&settings);
            assert_eq!(times.ttl, TimeDelta::seconds(120), "{settings:?}");
            assert_eq!(times.refresh, Duration::from_secs(60), "{settings:?}");
        }
    }

    #[test]
    fn a_refresh_at_or_above_the_ttl_reads_as_half() {
        let equal = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(60)),
            ("lease_refresh_seconds", json!(60)),
        ]));
        assert_eq!(equal.ttl, TimeDelta::seconds(60));
        assert_eq!(equal.refresh, Duration::from_secs(30));

        let above = LeaseTimes::from_app(&app(&[
            ("lease_ttl_seconds", json!(1)),
            ("lease_refresh_seconds", json!(5)),
        ]));
        assert_eq!(above.ttl, TimeDelta::seconds(1));
        assert_eq!(above.refresh, Duration::from_millis(500));
    }

    // -----------------------------------------------------------------------------------------
    // Heartbeat
    // -----------------------------------------------------------------------------------------

    /// A [`Clock`] that reads tokio's paused time, so `clock.now()` moves exactly as far as the
    /// heartbeat's `sleep` did.
    struct PausedClock {
        origin: DateTime<Utc>,
        start: tokio::time::Instant,
    }

    impl PausedClock {
        fn new() -> Self {
            Self {
                origin: Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap(),
                start: tokio::time::Instant::now(),
            }
        }

        fn elapsed(&self) -> Duration {
            self.start.elapsed()
        }
    }

    impl Clock for PausedClock {
        fn now(&self) -> DateTime<Utc> {
            self.origin + TimeDelta::from_std(self.elapsed()).expect("a test's elapsed time fits")
        }
    }

    /// The default 120 s / 60 s.
    fn default_times() -> LeaseTimes {
        LeaseTimes::from_app(&BTreeMap::new())
    }

    /// A refresh closure answering `script` in order (then `Ok(true)` for ever), recording each
    /// `until` beside the clock's instant at the call.
    #[allow(clippy::type_complexity)]
    fn scripted(
        clock: &PausedClock,
        script: Vec<Result<bool, StoreError>>,
    ) -> (
        Arc<Mutex<Vec<(DateTime<Utc>, DateTime<Utc>)>>>,
        impl FnMut(DateTime<Utc>) -> std::future::Ready<Result<bool, StoreError>> + '_,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let mut script = script.into_iter();
        let refresh = move |until| {
            seen.lock().unwrap().push((clock.now(), until));
            std::future::ready(script.next().unwrap_or(Ok(true)))
        };
        (calls, refresh)
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_refreshes_every_interval() {
        let clock = PausedClock::new();
        let times = default_times();
        let (calls, refresh) = scripted(&clock, vec![Ok(true), Ok(true), Ok(true)]);
        let outcome =
            tokio::time::timeout(Duration::from_secs(185), heartbeat(refresh, &clock, times)).await;
        assert!(
            outcome.is_err(),
            "the heartbeat never returns on `Ok(true)`"
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "a beat at 60 s, 120 s and 180 s");
        for (beat, (now, until)) in (1..).zip(calls.iter()) {
            assert_eq!(*now, clock.origin + TimeDelta::seconds(60 * beat));
            assert_eq!(*until, *now + times.ttl, "beat {beat}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_returns_abandoned_on_a_zero_row_refresh() {
        let clock = PausedClock::new();
        let (calls, refresh) = scripted(&clock, vec![Ok(true), Ok(false)]);
        let outcome = heartbeat(refresh, &clock, default_times()).await;
        assert_eq!(outcome, Heartbeat::Abandoned);
        assert_eq!(clock.elapsed(), Duration::from_secs(120));
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_survives_a_store_error() {
        let clock = PausedClock::new();
        let (calls, refresh) = scripted(
            &clock,
            vec![
                Err(StoreError::Unreachable("socket closed".to_owned())),
                Err(StoreError::Backend("deadlock".to_owned())),
                Ok(false),
            ],
        );
        let outcome = heartbeat(refresh, &clock, default_times()).await;
        assert_eq!(outcome, Heartbeat::Abandoned);
        assert_eq!(
            calls.lock().unwrap().len(),
            3,
            "abandoned on the third beat"
        );
        assert_eq!(clock.elapsed(), Duration::from_secs(180));
    }
}
