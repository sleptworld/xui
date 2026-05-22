use std::{any::Any, fmt::Debug, hash::Hash};

use slotmap::new_key_type;

use crate::{
    DirtyFlags, Event, EventContext, EventHandlers, EventResult, PaintCommand, Rect, Size,
    TextProps,
};

new_key_type! {
    pub struct NodeId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetType {
    Text,
    Label,
    Button,
    Column,
    Row,
    Container,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key(pub String);

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self(value)
    }
}

pub trait TextMeasurer {
    fn measure(&mut self, text: &str, font_size: f32) -> Size;

    fn measure_text(&mut self, props: &TextProps) -> Size {
        self.measure(props.text.as_str(), props.style.font_size)
    }
}

pub trait Widget: Debug {
    fn as_any(&self) -> &dyn Any;
    fn node_type(&self) -> WidgetType;
    fn key(&self) -> Option<&Key> {
        None
    }
    fn props_hash(&self) -> u64;
    fn event_handlers_mut(&mut self) -> &mut EventHandlers;
    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags;
    fn measure(&self, _measurer: &mut dyn TextMeasurer) -> Option<Size> {
        None
    }
    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>);
    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult;

    fn on_hovered_change(&mut self, _hovered: bool) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn on_click(&mut self) {}
}

pub trait Component<Context, Output> {
    fn render(&mut self, cx: &mut Context) -> Output;
}

impl<F, Context, Output> Component<Context, Output> for F
where
    F: FnMut(&mut Context) -> Output,
{
    fn render(&mut self, cx: &mut Context) -> Output {
        self(cx)
    }
}
