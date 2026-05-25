use std::any::Any;

use xui_interface::{
    DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand, Rect, Size,
    TextMeasurer, TextPaintCommand, TextProps, Widget, WidgetType,
};

use crate::core::Color;

use super::props_hash;

#[derive(Debug)]
pub struct LabelWidget {
    pub key: Option<Key>,
    pub text: String,
    pub color: Color,
    pub font_size: f32,
    pub event_handlers: EventHandlers,
}

impl LabelWidget {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            key: None,
            text: text.into(),
            color: Color::BLACK,
            font_size: 14.0,
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

impl Widget for LabelWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Label
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        let color = self.color;
        props_hash(&(
            &self.text,
            color.r.to_bits(),
            color.g.to_bits(),
            color.b.to_bits(),
            color.a.to_bits(),
            self.font_size.to_bits(),
        ))
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<LabelWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.text != next.text {
            self.text = next.text.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.font_size != next.font_size {
            self.font_size = next.font_size;
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.color != next.color {
            self.color = next.color;
            flags |= DirtyFlags::PAINT;
        }
        flags
    }

    fn measure(&self, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        Some(measurer.measure(&self.text, self.font_size))
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        let mut text_props = TextProps::new(self.text.clone());
        text_props.style.color = self.color;
        text_props.style.font_size = self.font_size;
        commands.push(PaintCommand::Text(TextPaintCommand {
            rect,
            props: text_props,
        }));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
