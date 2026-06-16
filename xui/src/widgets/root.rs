use crate::animation::AnimatedStyle;
use crate::event_system::callbacks::EventHandlers;
use xui_interface::{
    Color, ComputedStyle, DirtyFlags, Event, EventContext, EventRef, EventResult, PaintCommand,
    Rect, Size, Style, Widget, WidgetType, core::Sizing,
};

use super::props_hash;

#[derive(Debug)]
pub struct RootWidget {
    pub animated_style: AnimatedStyle,
    pub event_handlers: EventHandlers,
}

impl Default for RootWidget {
    fn default() -> Self {
        Self {
            animated_style: AnimatedStyle::new(Style::new()),
            event_handlers: EventHandlers::default(),
        }
    }
}

impl RootWidget {
    pub fn style(mut self, style: Style) -> Self {
        self.animated_style.base = style;
        self
    }

    animated_style_methods!(animated_style);
}

impl Widget for RootWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
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
        Style::new().size(Size::<Sizing>::new(
            Sizing::Percent(1.0.try_into().unwrap()),
            Sizing::Percent(1.0.try_into().unwrap()),
        ))
    }

    fn style(&self) -> &Style {
        &self.animated_style.base
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Clear(Color::WHITE));
    }

    fn handle_event(&mut self, _event: EventRef<'_>, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
