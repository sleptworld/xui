use std::hash::Hash;

use slotmap::new_key_type;

use crate::{DirtyFlags, Event, EventContext, EventResult, PaintCommand, Rect, Size};

new_key_type! {
    pub struct NodeId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeType {
    Root,
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

#[derive(Debug, Clone, PartialEq)]
pub enum WidgetKind {
    Root,
    Label {
        text: String,
        color: crate::Color,
        font_size: f32,
    },
    Button {
        text: String,
        pressed: bool,
        hovered: bool,
    },
    Column {
        gap: f32,
    },
    Row {
        gap: f32,
    },
    Container {
        background: crate::Color,
    },
}

impl WidgetKind {
    pub fn node_type(&self) -> NodeType {
        match self {
            Self::Root => NodeType::Root,
            Self::Label { .. } => NodeType::Label,
            Self::Button { .. } => NodeType::Button,
            Self::Column { .. } => NodeType::Column,
            Self::Row { .. } => NodeType::Row,
            Self::Container { .. } => NodeType::Container,
        }
    }
}

pub trait TextMeasurer {
    fn measure(&self, text: &str, font_size: f32) -> Size;
}

pub trait Widget {
    fn node_type(&self) -> NodeType;
    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags;
    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>);
    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult;
}
