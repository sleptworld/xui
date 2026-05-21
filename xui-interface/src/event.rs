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

pub type EventHandler = Box<dyn for<'a> FnMut(&Event, &mut EventContext<'a>) -> EventResult>;
pub type ClickEventHandler = Box<dyn for<'a> FnMut(&mut EventContext<'a>) -> EventResult>;
pub type HoverChangeEventHandler =
    Box<dyn for<'a> FnMut(bool, &mut EventContext<'a>) -> EventResult>;
pub type PointerEventHandler = Box<dyn for<'a> FnMut(&mut EventContext<'a>) -> EventResult>;
pub type KeyEventHandler = Box<dyn for<'a> FnMut(&Key, &mut EventContext<'a>) -> EventResult>;

#[derive(Default)]
pub struct EventHandlers {
    pub on_event: Option<EventHandler>,
    pub on_click: Option<ClickEventHandler>,
    pub on_hover_change: Option<HoverChangeEventHandler>,
    pub on_pointer_down: Option<PointerEventHandler>,
    pub on_pointer_up: Option<PointerEventHandler>,
    pub on_pointer_move: Option<PointerEventHandler>,
    pub on_key_down: Option<KeyEventHandler>,
    pub on_key_up: Option<KeyEventHandler>,
}

impl std::fmt::Debug for EventHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventHandlers")
            .field("on_event", &self.on_event.is_some())
            .field("on_click", &self.on_click.is_some())
            .field("on_hover_change", &self.on_hover_change.is_some())
            .field("on_pointer_down", &self.on_pointer_down.is_some())
            .field("on_pointer_up", &self.on_pointer_up.is_some())
            .field("on_pointer_move", &self.on_pointer_move.is_some())
            .field("on_key_down", &self.on_key_down.is_some())
            .field("on_key_up", &self.on_key_up.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRequest {
    Focus(NodeId),
    ClearFocus,
    CapturePointer(NodeId),
    ReleasePointerCapture,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventRequests {
    requests: Vec<EventRequest>,
}

impl EventRequests {
    pub fn push(&mut self, request: EventRequest) {
        self.requests.push(request);
    }

    pub fn iter(&self) -> impl Iterator<Item = EventRequest> + '_ {
        self.requests.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

pub struct EventContext<'a> {
    pub node_id: NodeId,
    pub phase: EventPhase,
    request_dirty: &'a mut DirtyFlags,
    requests: &'a mut EventRequests,
}

impl<'a> EventContext<'a> {
    pub fn new(
        node_id: NodeId,
        phase: EventPhase,
        request_dirty: &'a mut DirtyFlags,
        requests: &'a mut EventRequests,
    ) -> Self {
        Self {
            node_id,
            phase,
            request_dirty,
            requests,
        }
    }

    pub fn mark_needs_paint(&mut self) {
        *self.request_dirty |= DirtyFlags::PAINT;
    }

    pub fn mark_dirty(&mut self, flags: DirtyFlags) {
        *self.request_dirty |= flags;
    }

    pub fn request_focus(&mut self) {
        self.requests.push(EventRequest::Focus(self.node_id));
    }

    pub fn clear_focus(&mut self) {
        self.requests.push(EventRequest::ClearFocus);
    }

    pub fn capture_pointer(&mut self) {
        self.requests
            .push(EventRequest::CapturePointer(self.node_id));
    }

    pub fn release_pointer_capture(&mut self) {
        self.requests.push(EventRequest::ReleasePointerCapture);
    }
}
