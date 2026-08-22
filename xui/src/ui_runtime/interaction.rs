use crate::event_system::interaction::{HostInteraction, InteractionProperties};
use crate::{
    event_system::{
        callbacks::{CallbackHandleSet, CallbackStore},
        *,
    },
    focus::FocusManager,
};
use slotmap::SparseSecondaryMap;
use xui_interface::NodeId;

pub(crate) struct InteractionNode {
    pub properties: InteractionProperties,
    pub callbacks: CallbackHandleSet,
}

pub(crate) struct InteractionSystem {
    pub event_state: EventState,
    pub focus: FocusManager,
    pub callbacks: CallbackStore,
    nodes: SparseSecondaryMap<NodeId, InteractionNode>,
}

impl InteractionSystem {
    pub fn new() -> Self {
        Self {
            event_state: EventState::default(),
            focus: FocusManager::default(),
            callbacks: CallbackStore::default(),
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

        let current_callbacks = old.as_ref().map(|node| node.callbacks).unwrap_or_default();
        let Some(interaction) = interaction else {
            self.callbacks.clear_set(current_callbacks);
            return;
        };

        let callbacks = self
            .callbacks
            .update_set(current_callbacks, interaction.handlers);
        if let Some(handle) = interaction.properties.focus_handle.as_ref() {
            handle.bind(id);
        }
        self.nodes.insert(
            id,
            InteractionNode {
                properties: interaction.properties,
                callbacks,
            },
        );
    }

    pub fn remove(&mut self, id: NodeId) {
        self.event_state.clear_node(id);
        self.focus.clear_node(id);
        if let Some(node) = self.nodes.remove(id) {
            if let Some(handle) = node.properties.focus_handle.as_ref() {
                handle.unbind(id);
            }
            self.callbacks.clear_set(node.callbacks);
        }
    }
}

impl Default for InteractionSystem {
    fn default() -> Self {
        Self::new()
    }
}
