use crate::event_system::callbacks::EventHandlers;
use crate::focus::FocusHandle;
use xui_interface::{AccessibilityProperties, FocusProperties, ShortcutBinding};

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct InteractionProperties {
    pub focus: FocusProperties,
    pub focus_handle: Option<FocusHandle>,
    pub accessibility: AccessibilityProperties,
    pub shortcuts: Vec<ShortcutBinding>,
}

#[derive(Default, Debug)]
pub struct HostInteraction {
    pub properties: InteractionProperties,
    pub handlers: EventHandlers,
}

impl HostInteraction {
    pub fn is_empty(&self) -> bool {
        self.properties == InteractionProperties::default() && self.handlers.is_empty()
    }
}
