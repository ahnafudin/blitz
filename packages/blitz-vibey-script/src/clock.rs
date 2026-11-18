//! The clock which drives JS-observable time (timers and `Date`).

use std::cell::RefCell;
use std::rc::Rc;

use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ECMAScript TimeClip's inclusive range, exactly representable as an integer f64.
const MAX_DATE_MILLIS: i64 = 8_640_000_000_000_000;

/// The clock used for JS timer deadlines and `Date`.
///
/// In the default real mode it tracks the system's monotonic clock. In
/// virtual mode, time only advances when [`advance_to`](ScriptClock::advance_to)
/// is called: embedders which drive timers manually (e.g. test runners) can
/// jump straight to the next timer deadline instead of sleeping until it,
/// while preserving timer ordering.
#[derive(Clone)]
pub(crate) struct ScriptClock {
    inner: Rc<RefCell<ClockMode>>,
}

enum ClockMode {
    Real,
    Virtual {
        now: Instant,
        origin: Instant,
        epoch_millis: Option<i64>,
    },
}

impl Default for ScriptClock {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(ClockMode::Real)),
        }
    }
}

impl ScriptClock {
    /// Create a stopped clock before constructing Boa or running bootstrap JS.
    pub fn virtual_at_epoch(epoch_millis: i64) -> Result<Self, &'static str> {
        if !(-MAX_DATE_MILLIS..=MAX_DATE_MILLIS).contains(&epoch_millis) {
            return Err("virtual epoch outside ECMAScript Date range");
        }
        let origin = Instant::now();
        Ok(Self {
            inner: Rc::new(RefCell::new(ClockMode::Virtual {
                now: origin,
                origin,
                epoch_millis: Some(epoch_millis),
            })),
        })
    }

    fn epoch_millis(&self) -> Option<i64> {
        match *self.inner.borrow() {
            ClockMode::Real => None,
            ClockMode::Virtual { epoch_millis, .. } => epoch_millis,
        }
    }

    /// The current time according to this clock
    pub fn now(&self) -> Instant {
        match *self.inner.borrow() {
            ClockMode::Real => Instant::now(),
            ClockMode::Virtual { now, .. } => now,
        }
    }

    /// Switch to virtual mode. Time stops at the current instant and only
    /// advances via [`advance_to`](Self::advance_to).
    pub fn make_virtual(&self) {
        let mut mode = self.inner.borrow_mut();
        if matches!(*mode, ClockMode::Real) {
            let now = Instant::now();
            *mode = ClockMode::Virtual {
                now,
                origin: now,
                epoch_millis: None,
            };
        }
    }

    /// Advance a virtual clock to `deadline` (never backwards).
    /// Does nothing in real mode.
    pub fn advance_to(&self, deadline: Instant) -> Result<(), &'static str> {
        if let ClockMode::Virtual {
            now,
            origin,
            epoch_millis,
        } = &mut *self.inner.borrow_mut()
        {
            let next = (*now).max(deadline);
            if let Some(epoch) = *epoch_millis {
                let elapsed = i64::try_from(next.saturating_duration_since(*origin).as_millis())
                    .map_err(|_| "virtual clock elapsed milliseconds overflow")?;
                let date = epoch
                    .checked_add(elapsed)
                    .ok_or("virtual clock Date milliseconds overflow")?;
                if !(-MAX_DATE_MILLIS..=MAX_DATE_MILLIS).contains(&date) {
                    return Err("virtual clock advance exceeds ECMAScript Date range");
                }
            }
            *now = next;
        }
        Ok(())
    }
}

/// Adapter exposing a [`ScriptClock`] as a Boa [`Clock`](boa_engine::context::Clock),
/// so that `Date` observes virtual time consistently with timers.
pub(crate) struct BoaClockAdapter {
    pub clock: ScriptClock,
    /// Anchor for converting `Instant`s to durations-since-an-epoch
    pub base: Instant,
    /// Wall-clock time (ms since the Unix epoch) at `base`
    pub base_system_millis: i64,
}

impl BoaClockAdapter {
    pub fn new(clock: ScriptClock) -> Self {
        Self::new_with_wall_millis(clock, || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis() as i64
        })
    }

    fn new_with_wall_millis(clock: ScriptClock, wall_millis: impl FnOnce() -> i64) -> Self {
        let base = clock.now();
        // Lazy fallback is essential: explicit epochs never consult wall time.
        let base_system_millis = clock.epoch_millis().unwrap_or_else(wall_millis);
        Self {
            clock,
            base,
            base_system_millis,
        }
    }

    fn elapsed(&self) -> Duration {
        self.clock.now().saturating_duration_since(self.base)
    }
}

impl boa_engine::context::Clock for BoaClockAdapter {
    fn now(&self) -> boa_engine::context::time::JsInstant {
        let elapsed = self.elapsed();
        boa_engine::context::time::JsInstant::new(elapsed.as_secs(), elapsed.subsec_nanos())
    }

    fn system_time_millis(&self) -> i64 {
        self.base_system_millis + self.elapsed().as_millis() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boa_engine::context::Clock;
    use std::cell::Cell;

    #[test]
    fn explicit_epoch_ignores_distinct_injected_wall_facts() {
        let reads = Cell::new(0);
        let a = BoaClockAdapter::new_with_wall_millis(
            ScriptClock::virtual_at_epoch(1_234).unwrap(),
            || {
                reads.set(reads.get() + 1);
                100_000
            },
        );
        let b = BoaClockAdapter::new_with_wall_millis(
            ScriptClock::virtual_at_epoch(1_234).unwrap(),
            || {
                reads.set(reads.get() + 1);
                900_000
            },
        );
        assert_eq!(a.system_time_millis(), 1_234);
        assert_eq!(b.system_time_millis(), 1_234);
        assert_eq!(a.elapsed(), Duration::ZERO);
        assert_eq!(b.elapsed(), Duration::ZERO);
        assert_eq!(reads.get(), 0, "explicit construction consulted wall time");
    }

    #[test]
    fn epoch_range_accepts_signed_boundaries_and_rejects_invalid_values() {
        for epoch in [-MAX_DATE_MILLIS, -1, 0, MAX_DATE_MILLIS] {
            let clock = ScriptClock::virtual_at_epoch(epoch).unwrap();
            let adapter = BoaClockAdapter::new_with_wall_millis(clock, || panic!("wall read"));
            assert_eq!(adapter.system_time_millis(), epoch);
        }
        for epoch in [
            i64::MIN,
            -MAX_DATE_MILLIS - 1,
            MAX_DATE_MILLIS + 1,
            i64::MAX,
        ] {
            assert!(ScriptClock::virtual_at_epoch(epoch).is_err());
        }
    }

    #[test]
    fn explicit_advance_refusal_is_atomic_and_backward_time_never_rewinds() {
        let clock = ScriptClock::virtual_at_epoch(MAX_DATE_MILLIS).unwrap();
        let origin = clock.now();
        assert!(clock.advance_to(origin + Duration::from_millis(1)).is_err());
        assert_eq!(clock.now(), origin);
        let clock = ScriptClock::virtual_at_epoch(-MAX_DATE_MILLIS).unwrap();
        let origin = clock.now();
        let adapter = BoaClockAdapter::new_with_wall_millis(clock.clone(), || panic!("wall read"));
        clock.advance_to(origin + Duration::from_millis(1)).unwrap();
        assert_eq!(adapter.system_time_millis(), -MAX_DATE_MILLIS + 1);
        clock.advance_to(origin).unwrap();
        assert_eq!(adapter.system_time_millis(), -MAX_DATE_MILLIS + 1);
    }

    #[test]
    fn real_time_wall_anchor_and_elapsed_arithmetic_keep_the_old_semantics() {
        let reads = Cell::new(0);
        let clock = ScriptClock::default();
        let adapter = BoaClockAdapter::new_with_wall_millis(clock.clone(), || {
            reads.set(reads.get() + 1);
            55_000
        });
        assert_eq!(adapter.base_system_millis, 55_000);
        assert_eq!(reads.get(), 1);
        // Inject a precise monotonic fact after exercising the actual real-mode
        // constructor; no sleep or wall-clock timing tolerance is involved.
        *clock.inner.borrow_mut() = ClockMode::Virtual {
            now: adapter.base + Duration::from_millis(25),
            origin: adapter.base,
            epoch_millis: None,
        };
        assert_eq!(adapter.system_time_millis(), 55_025);
        assert_eq!(adapter.elapsed(), Duration::from_millis(25));
    }

    #[test]
    fn real_mode_ignores_manual_advance_and_legacy_virtual_time_keeps_its_anchor() {
        let clock = ScriptClock::default();
        clock
            .advance_to(clock.now() + Duration::from_millis(10))
            .unwrap();
        assert!(matches!(*clock.inner.borrow(), ClockMode::Real));
        clock.make_virtual();
        let origin = clock.now();
        let adapter = BoaClockAdapter::new_with_wall_millis(clock.clone(), || 12_000);
        clock
            .advance_to(origin + Duration::from_millis(10))
            .unwrap();
        clock.advance_to(origin).unwrap();
        assert_eq!(adapter.system_time_millis(), 12_010);
        assert_eq!(clock.epoch_millis(), None);
    }
}
