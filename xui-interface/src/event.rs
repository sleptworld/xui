use crate::{DirtyFlags, NodeId, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Tab,
    Enter,
    Escape,
    Backspace,
    Character(String),
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    PointerMove {
        position: Point,
    },
    PointerDown {
        position: Point,
        button: PointerButton,
    },
    PointerUp {
        position: Point,
        button: PointerButton,
    },
    Wheel {
        position: Point,
        delta: Point,
    },
    KeyDown {
        key: Key,
    },
    KeyUp {
        key: Key,
    },
    TextInput {
        text: String,
    },
    FocusGained,
    FocusLost,
}

impl Event {
    pub fn pointer_position(&self) -> Option<Point> {
        match self {
            Self::PointerMove { position }
            | Self::PointerDown { position, .. }
            | Self::PointerUp { position, .. }
            | Self::Wheel { position, .. } => Some(*position),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    Capture,
    Target,
    Bubble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    Ignored,
    Consumed,
}

impl EventResult {
    pub fn is_consumed(self) -> bool {
        matches!(self, Self::Consumed)
    }
}

pub struct EventContext<'a> {
    pub node_id: NodeId,
    pub phase: EventPhase,
    pub request_dirty: &'a mut DirtyFlags,
}

impl EventContext<'_> {
    pub fn mark_needs_paint(&mut self) {
        *self.request_dirty |= DirtyFlags::PAINT;
    }

    pub fn mark_dirty(&mut self, flags: DirtyFlags) {
        *self.request_dirty |= flags;
    }
}
