use xui_interface::{
    Color, DirtyFlags, Event, EventContext, EventResult, PaintCommand, Rect, Widget, WidgetKind,
    WidgetType,
};

#[derive(Debug)]
pub struct RootWidget;

impl Widget for RootWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn update_from_kind(&mut self, _new_kind: &WidgetKind) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn paint(&self, _rect: Rect, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Clear(Color::WHITE));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
