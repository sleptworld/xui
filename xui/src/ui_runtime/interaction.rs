use crate::event_system::interaction::{HostInteraction, InteractionProperties};
use crate::{
    event_system::{callbacks::EventHandlers, *},
    focus::FocusManager,
};
use slotmap::SparseSecondaryMap;
use xui_interface::NodeId;

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
    nodes: SparseSecondaryMap<NodeId, InteractionNode>,
}

impl InteractionSystem {
    pub fn new() -> Self {
        Self {
            event_state: EventState::default(),
            focus: FocusManager::default(),
            nodes: SparseSecondaryMap::new(),
        }
    }

    pub fn get(&self, id: NodeId) -> Option<&InteractionNode> {
        self.nodes.get(id)
    }

    pub fn update(&mut self, id: NodeId, interaction: Option<HostInteraction>) {
        let old = self.nodes.remove(id);
        if let Some(handle) = old
            .as_ref()
            .and_then(|node| node.properties.focus_handle.as_ref())
        {
            handle.unbind(id);
        }

        // Dropping the old node drops its handlers; there is no store to keep
        // in step, so no `update_set`/`clear_set` pair to get wrong.
        let Some(interaction) = interaction else {
            return;
        };

        if let Some(handle) = interaction.properties.focus_handle.as_ref() {
            handle.bind(id);
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
        if let Some(node) = self.nodes.remove(id)
            && let Some(handle) = node.properties.focus_handle.as_ref()
        {
            handle.unbind(id);
        }
    }
}

impl Default for InteractionSystem {
    fn default() -> Self {
        Self::new()
    }
}
