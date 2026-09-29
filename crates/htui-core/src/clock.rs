//! The one place an instant enters the walk (MOD-4 plan D8) and, since MOD-40 plan D11, a
//! [`MemStore`](crate::store::MemStore) handle.
//!
//! [`Clock`] and [`SystemClock`] moved here from `htui-orch`'s `isolate.rs`, and `TestClock` and
//! `epoch` from its `fake.rs` (feature `test-support`), so a store and the engine writing through
//! it can read one clock. `htui-orch` re-exports all three at their old paths.

#[cfg(feature = "test-support")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "test-support")]
use chrono::TimeDelta;
use chrono::{DateTime, SubsecRound as _, Utc};

use crate::model::TIMESTAMPTZ_DIGITS;

/// The one place an instant enters the walk (MOD-4 plan D8).
///
/// Every seam writer takes its `at` from the caller, so the engine owns the clock; and
/// `docs/ANA-2.md:1766-1768` requires the harness's settle snapshots to be sleep-free, so the
/// engine must be able to be handed a clock a test moves. `TestClock` is that clock (behind
/// `test-support`, so it is not linked from here: a doc link into a gated item is
/// `broken_intra_doc_links` in a plain `cargo doc`).
///
/// Since MOD-40 plan D10 a lease's expiry is the **store's** clock plus a TTL, never an instant
/// read here: a clock is only compared with another reading of itself.
pub trait Clock: Send + Sync {
    /// Now, already truncated to the column's resolution.
    ///
    /// The truncation is the contract, not an implementation detail: `TIMESTAMPTZ` keeps
    /// microseconds and `chrono` keeps nanoseconds, so an untruncated instant round-trips
    /// differently through Postgres than through `MemStore` and the two backends disagree about a
    /// column neither changed (`crates/htui-core/src/model/run.rs:272`).
    fn now(&self) -> DateTime<Utc>;
}

/// The production [`Clock`]: `Utc::now()`, truncated.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS)
    }
}

/// The suite's fixed instant, `2026-09-06T00:00:00Z`: where every `TestClock` starts, and the
/// instant `htui_agent::conformance::epoch()` answers (it delegates here, MOD-40 blueprint B22).
///
/// # Panics
///
/// Never: the constant is a valid instant.
#[cfg(feature = "test-support")]
#[must_use]
pub fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_788_393_600_000).expect("the suite epoch is a valid instant")
}

/// A [`Clock`] a test moves, starting at [`epoch`].
///
/// [`epoch`] is the instant every `FakeDriver` envelope is stamped from, so starting here means a
/// step's store rows and its session rows share one origin and a snapshot of both reads as one
/// timeline. [`advance`] is how a case elapses time; no case ever sleeps.
///
/// **A clone is the same clock** (MOD-40 blueprint B23): it reads and moves one shared instant,
/// which is how a harness hands its engine and its `MemStore` handle one clock. A second,
/// independent clock is [`TestClock::at`].
///
/// [`advance`]: TestClock::advance
#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
pub struct TestClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

#[cfg(feature = "test-support")]
impl Default for TestClock {
    fn default() -> Self {
        Self::at(epoch())
    }
}

#[cfg(feature = "test-support")]
impl TestClock {
    /// A clock at [`epoch`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A new, independent clock at `now`, truncated like every other instant the walk hands a
    /// writer (plan D8).
    #[must_use]
    pub fn at(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now.trunc_subsecs(TIMESTAMPTZ_DIGITS))),
        }
    }

    /// Move the clock, and every clone of it, forward (or back, for a test that wants to).
    ///
    /// # Panics
    /// When the clock's lock is poisoned, which no case does.
    pub fn advance(&self, by: TimeDelta) {
        let mut now = self
            .now
            .lock()
            .expect("no panic holds the test clock's lock");
        *now = (*now + by).trunc_subsecs(TIMESTAMPTZ_DIGITS);
    }

    /// Put the clock, and every clone of it, at an exact instant.
    ///
    /// # Panics
    /// When the clock's lock is poisoned, which no case does.
    pub fn set(&self, to: DateTime<Utc>) {
        *self
            .now
            .lock()
            .expect("no panic holds the test clock's lock") = to.trunc_subsecs(TIMESTAMPTZ_DIGITS);
    }
}

#[cfg(feature = "test-support")]
impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self
            .now
            .lock()
            .expect("no panic holds the test clock's lock")
    }
}

#[cfg(test)]
mod tests {
    use chrono::{SubsecRound as _, Utc};

    use super::{Clock as _, SystemClock};
    use crate::model::TIMESTAMPTZ_DIGITS;

    /// Plan D8's whole point: a stamp that survives a Postgres round trip unchanged.
    #[test]
    fn system_clock_is_microsecond_truncated() {
        let now = SystemClock.now();
        assert_eq!(now, now.trunc_subsecs(TIMESTAMPTZ_DIGITS));
        assert_eq!(now.timestamp_subsec_nanos() % 1_000, 0);
        assert!(
            (Utc::now() - now).num_seconds().abs() < 5,
            "the production clock is the wall clock"
        );
    }
}

#[cfg(all(test, feature = "test-support"))]
mod test_clock_tests {
    use chrono::TimeDelta;

    use super::{Clock as _, TestClock, epoch};

    /// Plan D8: the origin is the fake driver's, the resolution is the column's, and time moves
    /// only when a test moves it.
    #[test]
    fn test_clock_starts_at_the_drivers_epoch_and_only_a_test_moves_it() {
        let clock = TestClock::new();
        assert_eq!(clock.now(), epoch());
        assert_eq!(clock.now(), clock.now(), "no wall clock is read");

        clock.advance(TimeDelta::seconds(2));
        assert_eq!(clock.now(), epoch() + TimeDelta::seconds(2));

        clock.set(epoch() + TimeDelta::nanoseconds(1_500));
        assert_eq!(
            clock.now(),
            epoch() + TimeDelta::microseconds(1),
            "every instant the walk hands a writer is microsecond-truncated"
        );
    }

    /// MOD-40 blueprint B23: a clone is a second handle on one instant, `at` is a new clock.
    #[test]
    fn a_test_clock_clone_moves_with_it_and_at_does_not() {
        let clock = TestClock::new();
        let handle = clock.clone();
        let other = TestClock::at(clock.now());
        clock.advance(TimeDelta::seconds(5));
        assert_eq!(
            handle.now(),
            clock.now(),
            "a clone reads the instant its original moved"
        );
        assert_eq!(other.now(), epoch(), "`at` started a clock of its own");
        handle.set(epoch());
        assert_eq!(
            clock.now(),
            epoch(),
            "and moving the clone moves the original"
        );
    }
}
