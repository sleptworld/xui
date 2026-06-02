use crate::{ElementDesc, widgets::props_hash};
use xui_interface::{Event, EventHandlers, Key, Style, Translation, Widget};

pub struct ScrollScope {
    key: Option<Key>,
    translation: Translation,
    style: Style,
    children: Vec<ElementDesc>,
    event_handlers: EventHandlers,
}

impl std::fmt::Debug for ScrollScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollScope").finish()
    }
}

impl ScrollScope {
    pub fn new() -> Self {
        Self {
            key: None,
            translation: Default::default(),
            style: Style::new(),
            children: Vec::new(),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn child(mut self, child: impl Into<ElementDesc>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn key(mut self, key: Key) -> Self {
        self.key = Some(key);
        self
    }

    event_handler_methods!();
}

impl Widget for ScrollScope {
    fn node_type(&self) -> xui_interface::WidgetType {
        xui_interface::WidgetType::ScrollScope
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.key)
    }

    fn update_from(&mut self, next: &Self) -> xui_interface::DirtyFlags {
        xui_interface::DirtyFlags::empty()
    }

    fn handle_event(
        &mut self,
        event: &xui_interface::Event,
        cx: &mut xui_interface::EventContext<'_>,
    ) -> xui_interface::EventResult {
        match event {
            Event::Wheel { delta, .. } => {
                self.translation.translate(*delta);
                xui_interface::EventResult::Consumed
            }

            _ => xui_interface::EventResult::Consumed,
        }
    }

    fn default_style(&self) -> xui_interface::Style {
        Style::new()
    }

    fn style(&self) -> &xui_interface::Style {
        &self.style
    }

    fn paint(
        &self,
        rect: xui_interface::Rect,
        style: &xui_interface::ComputedStyle,
        commands: &mut Vec<xui_interface::PaintCommand>,
    ) {
        commands.push(xui_interface::PaintCommand::PushTransform {
            translate: self.translation,
        });
    }
}
