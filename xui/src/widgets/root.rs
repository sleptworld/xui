use xui_interface::{
    Color, ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult,
    PaintCommand, Rect, ScrollDirectionStyle, Size, Style, Widget, WidgetType, core::Sizing,
};

#[derive(Debug)]
pub struct RootWidget {
    pub event_handlers: EventHandlers,
}

impl Default for RootWidget {
    fn default() -> Self {
        Self {
            event_handlers: EventHandlers::default(),
        }
    }
}

impl Widget for RootWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn props_hash(&self) -> u64 {
        0
    }

    fn update_from(&mut self, _next: &Self) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn default_style(&self) -> Style {
        Style::new()
            .size(Size::<Sizing>::new(
                Sizing::Percent(1.0.try_into().unwrap()),
                Sizing::Percent(1.0.try_into().unwrap()),
            ))
            .scroll_direction(ScrollDirectionStyle::Both)
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Clear(Color::WHITE));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
