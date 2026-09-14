//! Per-frame callbacks, paced by the frame loop rather than by a timer.
//!
//! A timer is the wrong instrument for anything that has to look right on
//! screen. It fires on its own schedule, so it lands twice in one frame or not
//! at all in another; it knows nothing about whether the window is visible, so
//! it keeps a hidden window's simulation running; and it cannot tell the
//! renderer that the work it just did needs painting.
//!
//! A ticker is the frame loop's own callback. It runs at most once per frame
//! that is actually produced, it is handed that frame's [`FrameTime`] -- the
//! same one every animation in the frame is sampled against -- and it stops
//! with the loop when the window goes off screen, resuming where it left off
//! rather than where a wall clock got to.
//!
//! What it deliberately does *not* do is rebuild the component that installed
//! it. A ticker that calls a state setter every frame has turned the frame loop
//! into a full reconcile per frame, which is exactly the cost the retained tree
//! exists to avoid. The intended shape is a model in a [`crate::state::HookRef`]
//! and a canvas told to repaint:
//!
//! ```ignore
//! let model = cx.use_ref(Simulation::new);
//! let canvas = cx.use_memo(CanvasController::new);
//! cx.use_ticker(move |frame| {
//!     model.update(|model| model.advance(frame.delta()));
//!     canvas.get().invalidate();
//!     Tick::Continue
//! });
//! ```
//!
//! Reach for a state setter only when the frame really should rebuild the tree.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use crate::clock::FrameTime;

/// What a ticker callback says about the frame after this one.
///
/// The same per-frame contract as
/// [`CanvasGpuPainter::request_repaint`](crate::widgets::CanvasGpuPainter::request_repaint):
/// the callback is the only place that knows whether the animation is
/// finished, so it says so each time rather than leaving a flag on somewhere
/// that something else has to remember to clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// Run again on the next frame.
    Continue,
    /// Stop. [`Ticker::start`] is what starts it again.
    Stop,
}

type TickFn = dyn FnMut(FrameTime) -> Tick;

struct TickerState {
    callback: RefCell<Box<TickFn>>,
    running: Cell<bool>,
}

/// A handle to a ticker installed by [`crate::state::HookContext::use_ticker`].
///
/// Cloneable and `'static`, so it can be captured by an event handler -- a
/// play/pause button is the ordinary reason to hold one.
///
/// Deliberately a weak handle. The hook slot owns the ticker, so unmounting the
/// component that installed it is what ends it; a handle left behind in a
/// captured closure cannot keep a dead component's callback running. Every
/// method on a handle whose ticker is gone is a no-op.
#[derive(Clone)]
pub struct Ticker {
    inner: Weak<TickerState>,
}

impl Ticker {
    /// Runs the callback again from the next frame on.
    ///
    /// The way back from a callback that returned [`Tick::Stop`], and from
    /// [`Self::stop`]. A ticker that is already running is unaffected -- there
    /// is no count to balance.
    pub fn start(&self) {
        if let Some(state) = self.inner.upgrade() {
            state.running.set(true);
        }
    }

    /// Stops the callback from the next frame on.
    ///
    /// The frame in progress is not interrupted: a ticker that stops itself
    /// from inside its own callback still finishes that call.
    pub fn stop(&self) {
        if let Some(state) = self.inner.upgrade() {
            state.running.set(false);
        }
    }

    pub fn is_running(&self) -> bool {
        self.inner
            .upgrade()
            .is_some_and(|state| state.running.get())
    }
}

impl std::fmt::Debug for Ticker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Ticker")
            .field("running", &self.is_running())
            .finish()
    }
}

/// What the hook slot owns. Dropping it unregisters the ticker.
pub(crate) struct TickerHandle {
    state: Rc<TickerState>,
}

impl TickerHandle {
    /// Replaces the callback, which every render does.
    ///
    /// A closure captures the values of the render that built it, so keeping
    /// the first one would quietly pin the ticker to that render's state. The
    /// alternative -- a deps list -- makes the stale capture the default and
    /// the correct behaviour opt-in, which is the wrong way round for
    /// something that runs sixty times a second.
    pub(crate) fn set_callback(&self, callback: Box<TickFn>) {
        *self.state.callback.borrow_mut() = callback;
    }

    pub(crate) fn handle(&self) -> Ticker {
        Ticker {
            inner: Rc::downgrade(&self.state),
        }
    }
}

/// The live tickers, walked once a frame by the runtime.
///
/// A shared handle rather than an owned list, for the same reason
/// [`crate::widgets::CanvasController`] is: the hook installing a ticker and
/// the runtime running it are on opposite sides of the render, and neither
/// owns the other.
#[derive(Clone, Default)]
pub(crate) struct TickerRegistry {
    entries: Rc<RefCell<Vec<Weak<TickerState>>>>,
}

impl TickerRegistry {
    /// Installs a callback and returns the handle the hook slot keeps alive.
    pub(crate) fn install(&self, callback: Box<TickFn>) -> TickerHandle {
        let state = Rc::new(TickerState {
            callback: RefCell::new(callback),
            running: Cell::new(true),
        });
        self.entries.borrow_mut().push(Rc::downgrade(&state));
        TickerHandle { state }
    }

    /// Whether any live ticker wants the next frame.
    ///
    /// Consulted from `is_dirty` after every batch of events, so it has to stay
    /// cheap: an empty list -- the overwhelmingly common case -- is one
    /// `is_empty`, and a populated one is a walk of a handful of refcounts.
    pub(crate) fn is_running(&self) -> bool {
        self.entries
            .borrow()
            .iter()
            .any(|entry| entry.upgrade().is_some_and(|state| state.running.get()))
    }

    /// Runs every live, running ticker once.
    pub(crate) fn tick(&self, frame: FrameTime) {
        // Upgraded into a local list first: a callback may mount or unmount
        // components, and either one registers or drops tickers, which would
        // otherwise be a write into the list this loop is reading.
        let live: Vec<_> = self
            .entries
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        if live.is_empty() {
            return;
        }

        for state in live {
            if !state.running.get() {
                continue;
            }
            // Held only for the call. A callback that reaches its own `Ticker`
            // touches `running`, never this.
            let outcome = (state.callback.borrow_mut())(frame);
            if outcome == Tick::Stop {
                state.running.set(false);
            }
        }

        // Tickers whose component has gone. Done here rather than on unmount
        // because unmounting does not know what the hook slots it is dropping
        // contained.
        self.entries
            .borrow_mut()
            .retain(|entry| entry.strong_count() > 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FrameClock;
    use std::time::{Duration, Instant};

    fn frames(count: usize) -> Vec<FrameTime> {
        let start = Instant::now();
        let mut clock = FrameClock::new();
        (0..count)
            .map(|step| clock.tick(start + Duration::from_millis(16 * step as u64)))
            .collect()
    }

    #[test]
    fn a_ticker_runs_once_per_frame_with_that_frames_time() {
        let registry = TickerRegistry::default();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let recorder = Rc::clone(&seen);
        let _handle = registry.install(Box::new(move |frame| {
            recorder.borrow_mut().push(frame);
            Tick::Continue
        }));

        let frames = frames(3);
        for frame in &frames {
            registry.tick(*frame);
        }

        assert_eq!(seen.borrow().as_slice(), frames.as_slice());
    }

    #[test]
    fn stopping_from_inside_the_callback_ends_it_after_that_frame() {
        let registry = TickerRegistry::default();
        let runs = Rc::new(Cell::new(0u32));
        let counted = Rc::clone(&runs);
        let handle = registry.install(Box::new(move |_| {
            counted.set(counted.get() + 1);
            if counted.get() == 2 {
                Tick::Stop
            } else {
                Tick::Continue
            }
        }));

        for frame in frames(5) {
            registry.tick(frame);
        }

        assert_eq!(runs.get(), 2);
        assert!(!registry.is_running());
        assert!(!handle.handle().is_running());
    }

    #[test]
    fn a_stopped_ticker_can_be_started_again() {
        let registry = TickerRegistry::default();
        let runs = Rc::new(Cell::new(0u32));
        let counted = Rc::clone(&runs);
        let handle = registry.install(Box::new(move |_| {
            counted.set(counted.get() + 1);
            Tick::Continue
        }));
        let ticker = handle.handle();

        let frames = frames(6);
        registry.tick(frames[0]);
        ticker.stop();
        registry.tick(frames[1]);
        registry.tick(frames[2]);
        assert_eq!(runs.get(), 1, "a stopped ticker costs nothing per frame");
        assert!(!registry.is_running());

        ticker.start();
        registry.tick(frames[3]);
        assert_eq!(runs.get(), 2);
        assert!(registry.is_running());
    }

    #[test]
    fn dropping_the_hooks_handle_unregisters_the_ticker() {
        let registry = TickerRegistry::default();
        let runs = Rc::new(Cell::new(0u32));
        let counted = Rc::clone(&runs);
        let handle = registry.install(Box::new(move |_| {
            counted.set(counted.get() + 1);
            Tick::Continue
        }));
        let orphan = handle.handle();

        let frames = frames(3);
        registry.tick(frames[0]);
        drop(handle);

        assert!(
            !registry.is_running(),
            "an unmounted component must not keep the loop awake"
        );
        registry.tick(frames[1]);
        assert_eq!(runs.get(), 1);
        assert!(!orphan.is_running(), "and a handle left behind is inert");
        orphan.start();
        assert!(!orphan.is_running(), "including one someone tries to revive");
    }

    #[test]
    fn replacing_the_callback_keeps_the_handle_and_the_running_flag() {
        let registry = TickerRegistry::default();
        let first = Rc::new(Cell::new(0u32));
        let counted = Rc::clone(&first);
        let handle = registry.install(Box::new(move |_| {
            counted.set(counted.get() + 1);
            Tick::Continue
        }));
        let ticker = handle.handle();

        let second = Rc::new(Cell::new(0u32));
        let recounted = Rc::clone(&second);
        handle.set_callback(Box::new(move |_| {
            recounted.set(recounted.get() + 1);
            Tick::Continue
        }));

        for frame in frames(2) {
            registry.tick(frame);
        }

        assert_eq!(first.get(), 0, "the render's own closure is what runs");
        assert_eq!(second.get(), 2);
        assert!(ticker.is_running(), "the handle survives the replacement");
    }
}
