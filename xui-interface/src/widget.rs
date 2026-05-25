use std::{any::Any, fmt::Debug, hash::Hash};

use slotmap::new_key_type;

use crate::{
    ComputedStyle, ComputedTextStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult,
    PaintCommand, Rect, Size, Style, TextContent, TextLayoutConstraints, WidgetState,
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
    StyleScope,
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
    fn measure_text(&mut self, text: &str, props: &ComputedTextStyle) -> Size;

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        props: &ComputedTextStyle,
        _constraints: TextLayoutConstraints,
    ) -> Size;
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
    fn default_style(&self) -> Style {
        Style::new()
    }
    fn style(&self) -> &Style {
        static STYLE: std::sync::LazyLock<Style> = std::sync::LazyLock::new(Style::new);
        &STYLE
    }
    fn state_style(&self, _state: WidgetState) -> Style {
        Style::new()
    }
    fn state(&self) -> WidgetState {
        WidgetState::default()
    }
    fn style_scope(&self) -> Option<&Style> {
        None
    }
    fn measure(&self, _style: &ComputedStyle, _measurer: &mut dyn TextMeasurer) -> Option<Size> {
        None
    }
    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>);
    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult;

    fn on_hovered_change(&mut self, _hovered: bool) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn on_click(&mut self) {}

    fn text(&self) -> Option<TextContent> {
        None
    }
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
