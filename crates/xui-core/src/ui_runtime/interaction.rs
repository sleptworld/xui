use crate::capability::CommitCx;
use crate::scroll::{AppliedScroll, ScrollRequestQueue};
use crate::{
    event_system::{callbacks::EventHandlers, *},
    focus::FocusManager,
};
use xui_interface::{NodeId, Point};

/// Event state, focus, and what commit hands the binding capabilities. A
/// node's own interaction data — handlers, focus, accessibility, shortcuts,
/// the handles it binds — is in its capability rows, not here.
pub(crate) struct InteractionSystem {
    pub event_state: EventState,
    pub focus: FocusManager,
    /// Where every bound `ScrollController` queues its requests.
    pub scroll_requests: ScrollRequestQueue,
    /// Offsets layout moved back into range, awaiting their `Scroll` events.
    clamped_scrolls: Vec<AppliedScroll>,
    /// Detachment notifications wait until commit finishes, so finishing the
    /// current render cannot consume their scheduled update.
    detached_size_observers: Vec<crate::size::SizeObserver>,
    /// Handed out for nodes that registered no handlers. Owned rather than a
    /// `static` because `EventHandlers` holds `Rc` callbacks and so is not
    /// `Sync`; it borrows from `&self` instead, which is the lifetime callers
    /// want anyway.
    pub no_handlers: EventHandlers,
}

impl InteractionSystem {
    pub fn new() -> Self {
        Self {
            event_state: EventState::default(),
            focus: FocusManager::default(),
            scroll_requests: ScrollRequestQueue::default(),
            clamped_scrolls: Vec::new(),
            detached_size_observers: Vec::new(),
            no_handlers: EventHandlers::EMPTY,
        }
    }

    /// What a capability may touch while it commits.
    pub fn commit_cx(&mut self) -> CommitCx<'_> {
        CommitCx {
            scroll_requests: &self.scroll_requests,
            detached_size_observers: &mut self.detached_size_observers,
        }
    }

    pub fn notify_detached_size_observers(&mut self) {
        for observer in self.detached_size_observers.drain(..) {
            observer.notify();
        }
    }

    /// Records that layout clamped `node` from `before` to `after`.
    ///
    /// Several layouts can run before the next flush; they collapse into one
    /// report per node spanning all of them, and one that ends where it began
    /// is dropped.
    pub fn record_clamped_scroll(&mut self, node: NodeId, before: Point, after: Point) {
        if let Some(index) = self
            .clamped_scrolls
            .iter()
            .position(|scroll| scroll.node == node)
        {
            let scroll = &mut self.clamped_scrolls[index];
            scroll.after = after;
            if scroll.before == scroll.after {
                self.clamped_scrolls.swap_remove(index);
            }
            return;
        }
        self.clamped_scrolls.push(AppliedScroll {
            node,
            before,
            after,
        });
    }

    pub fn take_clamped_scrolls(&mut self) -> Vec<AppliedScroll> {
        std::mem::take(&mut self.clamped_scrolls)
    }

    /// Forgets `id`'s event and focus state. Its rows go with the columns.
    pub fn remove(&mut self, id: NodeId) {
        self.event_state.clear_node(id);
        self.focus.clear_node(id);
        self.clamped_scrolls.retain(|scroll| scroll.node != id);
    }
}

impl Default for InteractionSystem {
    fn default() -> Self {
        Self::new()
    }
}
