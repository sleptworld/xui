use std::any::Any;
use std::fmt::Debug;

use xui_interface::{
    Color, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand,
    PointerButton, Rect, Size, TextMeasurer, TextPaintCommand, TextProps, Widget, WidgetType,
};

use super::props_hash;

pub struct ButtonWidget {
    pub key: Option<Key>,
    pub text: String,
    pub event_handlers: EventHandlers,
    pub pressed: bool,
    pub hovered: bool,
}

impl ButtonWidget {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            key: None,
            text: text.into(),
            event_handlers: EventHandlers::default(),
            pressed: false,
            hovered: false,
        }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

impl Debug for ButtonWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("")
    }
}

impl Widget for ButtonWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Button
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.text)
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<ButtonWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        if self.text != next.text {
            self.text = next.text.clone();
            DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn measure(&self, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        let text = measurer.measure(&self.text, 14.0);
        Some(Size::new(text.width + 16.0, text.height.max(20.0) + 10.0))
    }

    fn on_hovered_change(&mut self, hovered: bool) -> DirtyFlags {
        self.hovered = hovered;
        DirtyFlags::PAINT
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
        let mut text_props = TextProps::new(self.text.clone());
        text_props.style.color = text_color;
        text_props.style.font_size = 14.0;
        commands.push(PaintCommand::Text(TextPaintCommand {
            rect: Rect::new(
                rect.x + 8.0,
                rect.y + 4.0,
                (rect.width - 16.0).max(0.0),
                (rect.height - 8.0).max(0.0),
            ),
            props: text_props,
        }));
    }

    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult {
        match event {
            Event::PointerDown {
                button: PointerButton::Primary,
                ..
            } => {
                self.pressed = true;
                cx.capture_pointer();
                cx.mark_dirty(DirtyFlags::PAINT);
                EventResult::Consumed
            }
            Event::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                self.pressed = false;
                cx.release_pointer_capture();
                cx.mark_dirty(DirtyFlags::PAINT);
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }
}
