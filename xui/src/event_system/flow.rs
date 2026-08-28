//! What a handler tells the dispatcher to do next.

use xui_interface::EventResult;

bitflags::bitflags! {
    /// The outcome of one handler, merged across the handlers of a node.
    ///
    /// Replaces the single `EventResult::Consumed`, which conflated two
    /// independent decisions: "no ancestor should see this" and "the widget
    /// should not do its own thing with it". Handling a click on a text input
    /// without stopping it from reaching the row, or stopping it from reaching
    /// the row while still letting the caret move, were both unexpressible.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Flow: u8 {
        /// Do not visit any further node on the propagation path.
        const STOP_PROPAGATION = 1 << 0;
        /// Do not run this node's widget's own handling of the event.
        ///
        /// Only meaningful because user handlers run *before* the widget's
        /// built-in behaviour; see [`crate::event_system::dispatcher`].
        const PREVENT_DEFAULT = 1 << 1;
    }
}

impl Flow {
    /// Both: the caller wants nothing else to happen.
    pub const CONSUME: Self = Self::STOP_PROPAGATION.union(Self::PREVENT_DEFAULT);

    pub fn stops_propagation(self) -> bool {
        self.contains(Self::STOP_PROPAGATION)
    }

    pub fn prevents_default(self) -> bool {
        self.contains(Self::PREVENT_DEFAULT)
    }
}

/// `EventResult::Consumed` historically meant "stop propagating", so that is
/// what it keeps meaning. Handlers written against the old return type go on
/// working unchanged.
impl From<EventResult> for Flow {
    fn from(result: EventResult) -> Self {
        match result {
            EventResult::Consumed => Flow::STOP_PROPAGATION,
            EventResult::Ignored => Flow::empty(),
        }
    }
}

impl From<Flow> for EventResult {
    fn from(flow: Flow) -> Self {
        if flow.stops_propagation() {
            EventResult::Consumed
        } else {
            EventResult::Ignored
        }
    }
}

/// `()` from a handler body means "observed it, carry on" — the common case,
/// and one less thing to write than `EventResult::Ignored`.
impl From<()> for Flow {
    fn from((): ()) -> Self {
        Flow::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_result_maps_to_stopping_propagation_only() {
        assert_eq!(Flow::from(EventResult::Consumed), Flow::STOP_PROPAGATION);
        assert_eq!(Flow::from(EventResult::Ignored), Flow::empty());
    }

    #[test]
    fn preventing_default_alone_does_not_read_as_consumed() {
        assert_eq!(
            EventResult::from(Flow::PREVENT_DEFAULT),
            EventResult::Ignored
        );
        assert_eq!(
            EventResult::from(Flow::STOP_PROPAGATION),
            EventResult::Consumed
        );
    }
}
