use std::any::Any;

use xui_interface::{
    Color, DirtyFlags, Event, EventContext, EventHandlers, EventResult, PaintCommand, Rect, Widget,
    WidgetType,
};

#[derive(Debug, Default)]
pub struct RootWidget {
    event_handlers: EventHandlers,
}

impl Widget for RootWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn props_hash(&self) -> u64 {
        0
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, _next: &dyn Widget) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn paint(&self, _rect: Rect, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Clear(Color::WHITE));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
