//! Deadlines nodes asked to be woken at.
//!
//! The counterpart of the animation loop for things that change rarely: a
//! caret that blinks twice a second has no business asking for every frame
//! the display can produce. A node names the instant it next changes; the
//! runtime reports the earliest of them as [`UiRuntime::next_wake`], the host
//! schedules a frame for then, and the frame delivers the wake as an event.
//! Nothing runs from a timer callback -- the deadline only decides when the
//! next frame is.

use std::time::Instant;
use xui_interface::NodeId;

use super::UiRuntime;

impl UiRuntime {
    /// Wakes `node` at `at`, replacing whatever wake it already asked for.
    ///
    /// # Preconditions
    /// - `node` is live.
    pub(crate) fn schedule_wake(&mut self, node: NodeId, at: Instant) {
        debug_assert!(self.hosts.contains_key(node));
        self.wakes.insert(node, at);
    }

    pub(crate) fn cancel_wake(&mut self, node: NodeId) {
        self.wakes.remove(node);
    }

    /// The earliest deadline anyone asked for, or `None` when nothing is
    /// waiting or nobody can see the window.
    ///
    /// Off screen is `None` for the reason `is_animating` is false there: what
    /// a wake changes cannot be seen, so it must not cost a frame. The wakes
    /// are kept, and the first frame after the window comes back delivers the
    /// ones that came due meanwhile.
    pub(crate) fn next_wake(&self) -> Option<Instant> {
        if !self.window_visible {
            return None;
        }
        self.wakes.values().copied().min()
    }

    /// Removes and returns every wake due at `now`, earliest first.
    ///
    /// Removed before they are delivered, so a node that asks again from its
    /// handler is scheduling its next wake, not keeping this one.
    pub(crate) fn take_due_wakes(&mut self, now: Instant) -> Vec<(NodeId, Instant)> {
        let mut due: Vec<_> = self
            .wakes
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(node, at)| (node, *at))
            .collect();
        for (node, _) in &due {
            self.wakes.remove(*node);
        }
        due.sort_by_key(|(_, at)| *at);
        due
    }
}
