use xui_interface::{
    DirtyFlags, Event, EventContext, EventResult, PaintCommand, Rect, Widget, WidgetKind,
    WidgetType,
};

use crate::core::Color;

#[derive(Debug)]
pub struct ContainerWidget {
    pub background: Color,
}

impl Widget for ContainerWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Container { background } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.background != *background {
            self.background = *background;
            DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        if self.background.a > 0.0 {
            commands.push(PaintCommand::FillRect {
                rect,
                color: self.background,
            });
        }
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
