use xui_interface::{
    DirtyFlags, Event, EventContext, EventResult, PaintCommand, Point, Rect, Size, TextMeasurer,
    Widget, WidgetKind, WidgetType,
};

use crate::core::Color;

#[derive(Debug)]
pub struct LabelWidget {
    pub text: String,
    pub color: Color,
    pub font_size: f32,
}

impl Widget for LabelWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Label
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Label {
            text,
            color,
            font_size,
        } = new_kind
        else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.text != *text {
            self.text = text.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.font_size != *font_size {
            self.font_size = *font_size;
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.color != *color {
            self.color = *color;
            flags |= DirtyFlags::PAINT;
        }
        flags
    }

    fn measure(&self, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        Some(measurer.measure(&self.text, self.font_size))
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Text {
            position: Point::new(rect.x, rect.y + self.font_size),
            text: self.text.clone(),
            color: self.color,
            size: self.font_size,
        });
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
