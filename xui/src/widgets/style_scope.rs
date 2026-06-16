use crate::animation::AnimatedStyle;
use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventRef, EventResult, Key, PaintCommand, Rect,
    Style, Widget, WidgetType,
};

use super::{props_hash, widget_element_desc};

pub struct StyleScopeWidget {
    pub key: Option<Key>,
    pub style: Style,
    pub local_style: AnimatedStyle,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for StyleScopeWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StyleScopeWidget")
            .field("key", &self.key)
            .field("style", &self.style)
            .field("local_style", &self.local_style)
            .finish()
    }
}

impl StyleScopeWidget {
    pub fn new(style: Style) -> Self {
        Self {
            key: None,
            style,
            local_style: AnimatedStyle::new(Style::new()),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.local_style.base = style;
        self
    }

    animated_style_methods!(local_style);

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self, children: Vec<ElementDesc>) -> ElementDesc {
        widget_element_desc(self, children)
    }

    event_handler_methods!();
}

impl Widget for StyleScopeWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::StyleScope
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(&self.style, &self.local_style))
    }

    fn update_from(&mut self, next: &Self) -> DirtyFlags {
        if self.style != next.style || self.local_style != next.local_style {
            self.style = next.style.clone();
            self.local_style = next.local_style.clone();
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn style_scope(&self) -> Option<&Style> {
        Some(&self.style)
    }

    fn style(&self) -> &Style {
        &self.local_style.base
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: EventRef, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
