use crate::animation::AnimatedStyle;
use crate::element::ElementDesc;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand,
    Rect, Size, Style, Widget, WidgetType,
};

use super::{props_hash, widget_element_desc};

pub struct RowWidget {
    pub key: Option<Key>,
    pub animated_style: AnimatedStyle,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for RowWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowWidget")
            .field("key", &self.key)
            .field("animated_style", &self.animated_style)
            .finish()
    }
}

impl RowWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            animated_style: AnimatedStyle::new(Style::new()),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.animated_style.base = style;
        self
    }

    animated_style_methods!(animated_style);

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self, children: Vec<ElementDesc>) -> ElementDesc {
        widget_element_desc(self, children)
    }

    event_handler_methods!();
}

impl Default for RowWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for RowWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Row
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.animated_style)
    }

    fn update_from(&mut self, next: &Self) -> DirtyFlags {
        if self.animated_style != next.animated_style {
            self.animated_style = next.animated_style.clone();
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn default_style(&self) -> Style {
        Style::new().size(Size::hug())
    }

    fn style(&self) -> &Style {
        &self.animated_style.base
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
