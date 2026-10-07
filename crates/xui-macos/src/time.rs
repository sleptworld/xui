//! Native event times.
//!
//! AppKit stamps an event with the time since the system started, in seconds --
//! the clock `NSEvent.timestamp`, `CADisplayLink` and `NSProcessInfo.systemUptime`
//! all read. The runtime measures in [`Instant`]s, so a native time is turned
//! into one by taking the current reading of both clocks and applying the
//! difference. That never needs the two to share an origin, only to tick at the
//! same rate.

use std::time::{Duration, Instant};

use objc2_foundation::NSProcessInfo;

/// When `timestamp` (seconds since system startup) was, given that the same
/// clock reads `uptime_now` at `now`.
///
/// A time that is not a real reading -- NaN, infinite, or further from now than
/// any event could be -- is taken to be now: a wrong time on one event is better
/// than a panic or a frame that thinks it is a day old.
pub(crate) fn instant_at(timestamp: f64, uptime_now: f64, now: Instant) -> Instant {
    /// More than this either way is not a clock difference, it is a bug.
    const MAX_SKEW: f64 = 3600.0;

    let age = uptime_now - timestamp;
    if !age.is_finite() || age.abs() > MAX_SKEW {
        return now;
    }
    if age >= 0.0 {
        now.checked_sub(Duration::from_secs_f64(age)).unwrap_or(now)
    } else {
        now + Duration::from_secs_f64(-age)
    }
}

/// [`instant_at`] for a timestamp read from AppKit just now.
pub(crate) fn instant_of(timestamp: f64) -> Instant {
    instant_at(timestamp, system_uptime(), Instant::now())
}

pub(crate) fn system_uptime() -> f64 {
    NSProcessInfo::processInfo().systemUptime()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_past_timestamp_is_that_long_ago() {
        let now = Instant::now();
        let at = instant_at(99.5, 100.0, now);
        assert_eq!(now - at, Duration::from_millis(500));
    }

    #[test]
    fn a_future_timestamp_is_that_long_from_now() {
        let now = Instant::now();
        let at = instant_at(100.008, 100.0, now);
        let ahead = at - now;
        assert!(
            ahead > Duration::from_micros(7990) && ahead < Duration::from_micros(8010),
            "{ahead:?}"
        );
    }

    #[test]
    fn a_timestamp_that_is_not_a_clock_reading_is_taken_to_be_now() {
        let now = Instant::now();
        assert_eq!(instant_at(f64::NAN, 100.0, now), now);
        assert_eq!(instant_at(f64::INFINITY, 100.0, now), now);
        assert_eq!(instant_at(0.0, 1.0e9, now), now);
    }

    #[test]
    fn ordering_between_events_is_preserved() {
        let now = Instant::now();
        let first = instant_at(10.000, 10.1, now);
        let second = instant_at(10.016, 10.1, now);
        assert!(second > first);
        assert_eq!(second - first, Duration::from_millis(16));
    }
}
