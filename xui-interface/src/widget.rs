use std::fmt::Debug;

use slotmap::new_key_type;
use smallstr::SmallString;

use crate::{
    ComputedStyle, EventContext, EventResult, PaintCommand, Rect, Style, TextContent, TextProps,
    TextStyle, WidgetUpdateFlags, events::EventRef,
};

new_key_type! {
    pub struct NodeId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetType {
    Text,
    TextInput,
    Label,
    Button,
    Column,
    Row,
    Container,
    StyleScope,
    ScrollScope,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key(pub SmallString<[u8; 64]>);

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

pub trait Widget: Debug {
    fn node_type(&self) -> WidgetType;

    fn key(&self) -> Option<&Key> {
        None
    }

    fn props_hash(&self) -> u64;

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags;

    fn default_style(&self) -> Style {
        Style::new()
    }

    fn style(&self) -> &Style;

    fn style_scope(&self) -> Option<&Style> {
        None
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>);

    fn handle_event(&mut self, event: EventRef<'_>, cx: &mut EventContext<'_>) -> EventResult;

    fn on_click(&mut self) {}

    fn text(&self) -> Option<TextContent> {
        None
    }

    fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
        self.text().map(|text| {
            let mut props = TextProps::new(text);
            props.style = TextStyle::from(&style.text);
            props
        })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLifecycleEvent {
    Created(NodeId),
    Moved {
        id: NodeId,
        old_parent: Option<NodeId>,
        new_parent: Option<NodeId>,
        old_position: usize,
        new_position: usize,
    },
    Removed(NodeId),
}
