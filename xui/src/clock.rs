//! The frame clock: one time sample per frame, shared by everything that moves.
//!
//! Animation used to read the time wherever it happened to need it -- the style
//! transitions from their own `Instant` in the runtime, a canvas painter from
//! whatever `Instant` it captured when it started. Two consequences followed.
//! Sampling twice inside one frame lets two animations that should agree drift
//! apart by however long the frame took, and a wall clock kept running while
//! the frame loop was stopped, so a canvas resumed by jumping forward by the
//! whole pause rather than continuing.
//!
//! [`FrameClock`] is the single time source that fixes both. The runtime ticks
//! it once at the top of a frame and publishes the resulting [`FrameTime`] for
//! the frame's duration; everything downstream reads that value instead of the
//! clock. Because it is ticked rather than read, the time it reports is a
//! *presentation* clock -- it advances only across frames that were actually
//! produced, which is what makes pausing and resuming continuous.

use std::time::{Duration, Instant};

/// The time of one frame, sampled once and frozen until the next.
///
/// [`Self::timestamp`] is the sum of every [`Self::delta`] the clock has
/// produced, not the wall time since start. The two differ whenever a frame
/// took longer than the clock's cap or the loop stopped entirely, and keeping
/// the identity `timestamp == sum(delta)` is what lets a model that integrates
/// deltas and a model that samples the timestamp stay on the same schedule.
///
/// Input carries its own wall-clock timestamps and must keep doing so: a
/// double-click threshold or a fling velocity measured against a clock that
/// pauses would misjudge every gesture that spans a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTime {
    index: u64,
    timestamp: Duration,
    delta: Duration,
}

impl FrameTime {
    /// The time before any frame has been produced.
    pub const ZERO: Self = Self {
        index: 0,
        timestamp: Duration::ZERO,
        delta: Duration::ZERO,
    };

    /// How many frames the clock has produced, counting this one.
    ///
    /// The first frame is 1, so [`Self::ZERO`] -- the value in place before the
    /// first tick -- is distinguishable from it.
    pub fn index(self) -> u64 {
        self.index
    }

    /// Time on the presentation clock at the start of this frame.
    pub fn timestamp(self) -> Duration {
        self.timestamp
    }

    /// Time on the presentation clock since the previous frame.
    ///
    /// Zero on the first frame and on the first frame after a suspend, because
    /// there is no previous frame to measure from. Never longer than
    /// [`FrameClock::max_delta`].
    pub fn delta(self) -> Duration {
        self.delta
    }

    /// [`Self::timestamp`] in seconds, for the many samplers that want a float.
    pub fn timestamp_secs(self) -> f32 {
        self.timestamp.as_secs_f32()
    }

    /// [`Self::delta`] in seconds, for integrating a model over the frame.
    pub fn delta_secs(self) -> f32 {
        self.delta.as_secs_f32()
    }
}

/// Produces the per-frame [`FrameTime`], owned by the runtime driving the loop.
///
/// One per runtime rather than one per process: tests and benchmarks drive it
/// with the instants they choose, and two windows on displays of different
/// refresh rates are two independent sequences of frames.
#[derive(Debug)]
pub struct FrameClock {
    /// When the last frame was ticked, or `None` while suspended.
    last: Option<Instant>,
    frame: FrameTime,
    max_delta: Duration,
}

impl FrameClock {
    /// The longest delta a single frame may report.
    ///
    /// A frame that overruns -- a stall, a breakpoint, a page fault storm --
    /// would otherwise hand every animation a step long enough to skip its
    /// entire duration in one sample. Clamping turns that into slow motion,
    /// which is both recoverable and easier to see than a jump. 100ms is six
    /// frames at 60Hz: long enough that no real frame hits it, short enough
    /// that the jump it permits stays small.
    pub const DEFAULT_MAX_DELTA: Duration = Duration::from_millis(100);

    pub fn new() -> Self {
        Self {
            last: None,
            frame: FrameTime::ZERO,
            max_delta: Self::DEFAULT_MAX_DELTA,
        }
    }

    /// The cap applied to every delta. See [`Self::DEFAULT_MAX_DELTA`].
    pub fn max_delta(&self) -> Duration {
        self.max_delta
    }

    pub fn set_max_delta(&mut self, max_delta: Duration) {
        self.max_delta = max_delta;
    }

    /// The frame most recently produced, or [`FrameTime::ZERO`] before the
    /// first one.
    pub fn frame(&self) -> FrameTime {
        self.frame
    }

    /// Whether the next frame will start a fresh delta. See [`Self::suspend`].
    pub fn is_suspended(&self) -> bool {
        self.last.is_none()
    }

    /// Opens a frame at `now` and returns its time.
    ///
    /// `now` is a parameter rather than an `Instant::now()` inside, so the loop
    /// can be driven deterministically. The runtime passes the real instant.
    pub fn tick(&mut self, now: Instant) -> FrameTime {
        let delta = match self.last {
            Some(last) => now.saturating_duration_since(last).min(self.max_delta),
            // Either the first frame ever or the first after a suspend. There
            // is no previous frame to measure against, and inventing one from
            // the gap is exactly the jump this type exists to prevent.
            None => Duration::ZERO,
        };
        self.last = Some(now);
        self.frame = FrameTime {
            index: self.frame.index + 1,
            timestamp: self.frame.timestamp.saturating_add(delta),
            delta,
        };
        self.frame
    }

    /// Opens a frame that is painted but not animated through.
    ///
    /// A window that is off screen still repaints when something invalidates it
    /// -- a resize, a state change, a theme swap -- and that repaint has to
    /// produce a correct picture. What it must not do is move: the frames in
    /// between were never drawn, so advancing through them would make the
    /// return to the screen a jump.
    ///
    /// The frame index advances, because a frame did happen. The timestamp does
    /// not, and the delta is zero, so every animation samples exactly where it
    /// stopped. The clock is left suspended, so the first real frame after the
    /// window comes back also starts from a zero delta.
    pub fn hold(&mut self) -> FrameTime {
        self.last = None;
        self.frame = FrameTime {
            index: self.frame.index + 1,
            timestamp: self.frame.timestamp,
            delta: Duration::ZERO,
        };
        self.frame
    }

    /// Stops the clock until the next tick, without rewinding it.
    ///
    /// Called when the loop is about to idle -- nothing is animating, so no
    /// frame is scheduled and the gap until the next one is unbounded. The
    /// timestamp keeps the value it reached, and the frame that eventually
    /// arrives reports a zero delta instead of the length of the idle, so an
    /// animation starting after it begins at its first step rather than part
    /// way through.
    pub fn suspend(&mut self) {
        self.last = None;
    }
}

impl Default for FrameClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(clock_start: Instant, millis: u64) -> Instant {
        clock_start + Duration::from_millis(millis)
    }

    #[test]
    fn the_first_frame_has_no_delta_and_starts_at_zero() {
        let start = Instant::now();
        let mut clock = FrameClock::new();

        assert_eq!(clock.frame(), FrameTime::ZERO);
        let frame = clock.tick(at(start, 1234));

        assert_eq!(frame.index(), 1);
        assert_eq!(frame.delta(), Duration::ZERO);
        assert_eq!(
            frame.timestamp(),
            Duration::ZERO,
            "the clock starts when it is first ticked, not at the process epoch"
        );
    }

    #[test]
    fn the_timestamp_is_the_sum_of_the_deltas() {
        let start = Instant::now();
        let mut clock = FrameClock::new();
        clock.tick(at(start, 0));

        let mut summed = Duration::ZERO;
        for millis in [16, 33, 49, 66] {
            let frame = clock.tick(at(start, millis));
            summed += frame.delta();
            assert_eq!(frame.timestamp(), summed);
        }
        assert_eq!(clock.frame().index(), 5);
    }

    #[test]
    fn an_overlong_frame_is_clamped_to_slow_motion() {
        let start = Instant::now();
        let mut clock = FrameClock::new();
        clock.tick(at(start, 0));

        let frame = clock.tick(at(start, 5_000));

        assert_eq!(frame.delta(), FrameClock::DEFAULT_MAX_DELTA);
        assert_eq!(
            frame.timestamp(),
            FrameClock::DEFAULT_MAX_DELTA,
            "the identity timestamp == sum(delta) survives the clamp"
        );
    }

    #[test]
    fn suspending_keeps_the_timestamp_and_drops_the_gap() {
        let start = Instant::now();
        let mut clock = FrameClock::new();
        clock.tick(at(start, 0));
        let before = clock.tick(at(start, 16));

        clock.suspend();
        assert!(clock.is_suspended());
        let after = clock.tick(at(start, 60_000));

        assert_eq!(after.delta(), Duration::ZERO, "the idle is not a frame step");
        assert_eq!(
            after.timestamp(),
            before.timestamp(),
            "and it does not advance the clock either"
        );
        assert_eq!(after.index(), before.index() + 1);
        assert!(!clock.is_suspended());
    }

    #[test]
    fn a_held_frame_paints_without_moving_the_clock() {
        let start = Instant::now();
        let mut clock = FrameClock::new();
        clock.tick(at(start, 0));
        let last_drawn = clock.tick(at(start, 16));

        let held = clock.hold();
        assert_eq!(held.index(), last_drawn.index() + 1, "a frame did happen");
        assert_eq!(held.timestamp(), last_drawn.timestamp());
        assert_eq!(held.delta(), Duration::ZERO);
        assert!(clock.is_suspended(), "and the one after it starts fresh");

        let resumed = clock.tick(at(start, 90_000));
        assert_eq!(resumed.delta(), Duration::ZERO);
        assert_eq!(resumed.timestamp(), last_drawn.timestamp());
    }

    #[test]
    fn a_backwards_instant_does_not_panic() {
        let start = Instant::now();
        let mut clock = FrameClock::new();
        clock.tick(at(start, 100));

        // `Instant` is monotonic per the platform, but the clock takes the
        // instant from its caller and must not trust it to be ordered.
        let frame = clock.tick(at(start, 50));

        assert_eq!(frame.delta(), Duration::ZERO);
    }

    #[test]
    fn the_cap_is_configurable() {
        let start = Instant::now();
        let mut clock = FrameClock::new();
        clock.set_max_delta(Duration::from_millis(8));
        clock.tick(at(start, 0));

        assert_eq!(clock.tick(at(start, 100)).delta(), Duration::from_millis(8));
    }
}
