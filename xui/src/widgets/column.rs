use xui_interface::{
    DirtyFlags, Event, EventContext, EventResult, PaintCommand, Rect, Widget, WidgetKind,
    WidgetType,
};

#[derive(Debug)]
pub struct ColumnWidget {
    pub gap: f32,
}

impl Widget for ColumnWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Column
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Column { gap } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.gap != *gap {
            self.gap = *gap;
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, _rect: Rect, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
