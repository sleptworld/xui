use crate::event_system::callbacks::EventHandlers;
use xui_interface::{
    Color, ComputedStyle, EventContext, EventRef, EventResult, PaintCommand, Rect, Size, Style,
    Widget, WidgetType, WidgetUpdateFlags, core::Sizing,
};

use super::props_hash;

#[derive(Debug)]
pub struct RootWidget {
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl Default for RootWidget {
    fn default() -> Self {
        Self {
            style: Style::new(),
            event_handlers: EventHandlers::default(),
        }
    }
}

impl RootWidget {
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl Widget for RootWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.style)
    }

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        if self.style != next.style {
            self.style = next.style.clone();
            WidgetUpdateFlags::STYLE_TARGET
        } else {
            WidgetUpdateFlags::empty()
        }
    }

    fn default_style(&self) -> Style {
        Style::new().size(Size::<Sizing>::new(
            Sizing::Percent(1.0.try_into().unwrap()),
            Sizing::Percent(1.0.try_into().unwrap()),
        ))
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Clear(Color::WHITE));
    }

    fn handle_event(&mut self, _event: EventRef<'_>, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
