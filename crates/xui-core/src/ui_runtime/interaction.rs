use crate::event_system::interaction::{HostInteraction, InteractionProperties};
use crate::scroll::{AppliedScroll, ScrollController, ScrollRequestQueue};
use crate::{
    event_system::{callbacks::EventHandlers, *},
    focus::FocusManager,
};
use slotmap::SparseSecondaryMap;
use xui_interface::{NodeId, Point};

pub(crate) struct InteractionNode {
    pub properties: InteractionProperties,
    /// Owned outright. The handlers used to live in 34 global `SlotMap`s with
    /// only their keys stored here; nothing ever resolved a key from anywhere
    /// but this node, so the indirection was pure overhead.
    pub handlers: EventHandlers,
}

pub(crate) struct InteractionSystem {
    pub event_state: EventState,
    pub focus: FocusManager,
    /// Where every bound `ScrollController` queues its requests.
    pub scroll_requests: ScrollRequestQueue,
    /// Offsets layout moved back into range, awaiting their `Scroll` events.
    clamped_scrolls: Vec<AppliedScroll>,
    nodes: SparseSecondaryMap<NodeId, InteractionNode>,
    /// Index of nodes with a bound scroll controller, so publishing metrics
    /// after layout visits only those instead of every interaction node.
    scroll_controllers: SparseSecondaryMap<NodeId, ()>,
    /// Handed out by [`InteractionSystem::handlers`] for nodes that registered
    /// none. Owned rather than a `static` because `EventHandlers` holds `Rc`
    /// callbacks and so is not `Sync`; it borrows from `&self` instead, which
    /// is the lifetime callers want anyway.
    no_handlers: EventHandlers,
}

impl InteractionSystem {
    pub fn new() -> Self {
        Self {
            event_state: EventState::default(),
            focus: FocusManager::default(),
            scroll_requests: ScrollRequestQueue::default(),
            clamped_scrolls: Vec::new(),
            nodes: SparseSecondaryMap::new(),
            scroll_controllers: SparseSecondaryMap::new(),
            no_handlers: EventHandlers::EMPTY,
        }
    }

    /// The handlers registered for `id`, empty when it registered none.
    ///
    /// Distinct from `get(id).map(..)`: a node with no handlers has no
    /// interaction node at all, and that ordinary case must not be reported
    /// the same way as a node that is missing outright.
    pub fn handlers(&self, id: NodeId) -> &EventHandlers {
        self.get(id)
            .map_or(&self.no_handlers, |node| &node.handlers)
    }

    pub fn get(&self, id: NodeId) -> Option<&InteractionNode> {
        self.nodes.get(id)
    }

    pub fn scroll_controller(&self, id: NodeId) -> Option<&ScrollController> {
        self.get(id)?.properties.scroll_controller.as_ref()
    }

    pub fn scroll_controller_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.scroll_controllers.keys()
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

    pub fn update(&mut self, id: NodeId, interaction: Option<HostInteraction>) {
        let old = self.nodes.remove(id);
        if let Some(old) = old.as_ref() {
            unbind_handles(id, &old.properties);
        }
        self.scroll_controllers.remove(id);

        // Dropping the old node drops its handlers; there is no store to keep
        // in step, so no `update_set`/`clear_set` pair to get wrong.
        let Some(interaction) = interaction else {
            return;
        };

        if let Some(handle) = interaction.properties.focus_handle.as_ref() {
            handle.bind(id);
        }
        if let Some(controller) = interaction.properties.scroll_controller.as_ref() {
            controller.bind(id, self.scroll_requests.clone());
            self.scroll_controllers.insert(id, ());
        }
        self.nodes.insert(
            id,
            InteractionNode {
                properties: interaction.properties,
                handlers: interaction.handlers,
            },
        );
    }

    pub fn remove(&mut self, id: NodeId) {
        self.event_state.clear_node(id);
        self.focus.clear_node(id);
        self.scroll_controllers.remove(id);
        self.clamped_scrolls.retain(|scroll| scroll.node != id);
        if let Some(node) = self.nodes.remove(id) {
            unbind_handles(id, &node.properties);
        }
    }
}

fn unbind_handles(id: NodeId, properties: &InteractionProperties) {
    if let Some(handle) = properties.focus_handle.as_ref() {
        handle.unbind(id);
    }
    if let Some(controller) = properties.scroll_controller.as_ref() {
        controller.unbind(id);
    }
}

impl Default for InteractionSystem {
    fn default() -> Self {
        Self::new()
    }
}
