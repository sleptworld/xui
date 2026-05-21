use std::fmt::Debug;

use xui_interface::{
    Color, DirtyFlags, Event, EventContext, EventResult, PaintCommand, Point, PointerButton, Rect,
    Size, TextMeasurer, Widget, WidgetKind, WidgetType,
};

pub struct ButtonWidget {
    pub text: String,
    pub pressed: bool,
    pub hovered: bool,
}

impl Debug for ButtonWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("")
    }
}

impl Widget for ButtonWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Button
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Button { text, .. } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        if self.text != *text {
            self.text = text.clone();
            DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn measure(&self, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        let text = measurer.measure(&self.text, 14.0);
        Some(Size::new(text.width + 16.0, text.height.max(20.0) + 10.0))
    }

    fn on_hovered_change(&mut self, hovered: bool) {
        println!("hovered: {}", hovered);
        self.hovered = hovered;
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        let background = if self.pressed {
            Color::BLUE_500
        } else if self.hovered {
            Color::GRAY_300
        } else {
            Color::GRAY_100
        };
        let text_color = if self.pressed {
            Color::WHITE
        } else {
            Color::BLACK
        };
        commands.push(PaintCommand::FillRect {
            rect,
            color: background,
        });
        commands.push(PaintCommand::StrokeRect {
            rect,
            color: Color::GRAY_300,
            width: 1.0,
        });
        commands.push(PaintCommand::Text {
            position: Point::new(rect.x + 8.0, rect.y + 18.0),
            text: self.text.clone(),
            color: text_color,
            size: 14.0,
        });
    }

    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Consumed
    }
}
