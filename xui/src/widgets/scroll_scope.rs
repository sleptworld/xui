use crate::{
    ElementDesc,
    animation::AnimatedStyle,
    event_system::callbacks::EventHandlers,
    widgets::{props_hash, widget_element_desc},
};
use xui_interface::{DirtyFlags, Event, Key, Style, Translation, Widget, events::RawEvent};

pub struct ScrollScope {
    key: Option<Key>,
    translation: Translation,
    animated_style: AnimatedStyle,
    children: Vec<ElementDesc>,
    pub event_handlers: EventHandlers,
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
            animated_style: AnimatedStyle::new(Style::new()),
            children: Vec::new(),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.animated_style.base = style;
        self
    }

    animated_style_methods!(animated_style);

    pub fn child(mut self, child: impl Into<ElementDesc>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn key(mut self, key: Key) -> Self {
        self.key = Some(key);
        self
    }

    pub fn into_element_desc(self, children: Vec<ElementDesc>) -> ElementDesc {
        widget_element_desc(self, children)
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
        props_hash(&(&self.key, &self.animated_style))
    }

    fn update_from(&mut self, next: &Self) -> xui_interface::DirtyFlags {
        if self.animated_style != next.animated_style {
            self.animated_style = next.animated_style.clone();
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn handle_event(
        &mut self,
        event: &xui_interface::Event,
        _cx: &mut xui_interface::EventContext<'_>,
    ) -> xui_interface::EventResult {
        match event {
            Event::Raw(RawEvent::Wheel(raw)) => {
                let delta = match raw.delta {
                    xui_interface::ScrollDelta::Pixels(delta) => delta,
                    xui_interface::ScrollDelta::Lines(delta) => {
                        xui_interface::Translation::new(delta.x * 16.0, delta.y * 16.0)
                    }
                    xui_interface::ScrollDelta::Pages(delta) => {
                        xui_interface::Translation::new(delta.x * 800.0, delta.y * 800.0)
                    }
                };
                self.translation
                    .translate(xui_interface::Point::new(delta.x, delta.y));
                xui_interface::EventResult::Consumed
            }

            _ => xui_interface::EventResult::Consumed,
        }
    }

    fn default_style(&self) -> xui_interface::Style {
        Style::new()
    }

    fn style(&self) -> &xui_interface::Style {
        &self.animated_style.base
    }

    fn paint(
        &self,
        _rect: xui_interface::Rect,
        _style: &xui_interface::ComputedStyle,
        commands: &mut Vec<xui_interface::PaintCommand>,
    ) {
        commands.push(xui_interface::PaintCommand::PushTransform {
            translate: self.translation,
        });
    }
}
