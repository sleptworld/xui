use crate::{NodeId, Point, Translation};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct XuiDeviceId(pub u32);

impl XuiDeviceId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct XuiPointerId(pub u64);

impl XuiPointerId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Primary,
    Secondary,
    Auxiliary,
    Back,
    Forward,
    Other(u16),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PointerButtons {
    pub primary: bool,
    pub secondary: bool,
    pub auxiliary: bool,
    pub back: bool,
    pub forward: bool,
}

impl PointerButtons {
    pub fn from_button(button: PointerButton) -> Self {
        let mut buttons = Self::default();
        buttons.set(button, true);
        buttons
    }

    pub fn set(&mut self, button: PointerButton, pressed: bool) {
        match button {
            PointerButton::Primary => self.primary = pressed,
            PointerButton::Secondary => self.secondary = pressed,
            PointerButton::Auxiliary => self.auxiliary = pressed,
            PointerButton::Back => self.back = pressed,
            PointerButton::Forward => self.forward = pressed,
            PointerButton::Other(_) => {}
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Mouse,
    Touch,
    Pen,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerCoords {
    pub window: Point,
    pub viewport: Point,
    pub target_local: Point,
    pub current_local: Point,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerSnapshot {
    pub pointer_id: XuiPointerId,
    pub button: Option<PointerButton>,
    pub buttons: PointerButtons,
    pub coords: PointerCoords,
    pub is_primary: bool,
    pub tilt_x: Option<f32>,
    pub tilt_y: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDelta {
    Pixels(Translation),
    Lines(Translation),
    Pages(Translation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuTrigger {
    PointerSecondary,
    Keyboard,
    LongPress,
    Programmatic,
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
pub enum RawEvent {
    PointerMove(RawPointerMove),
    PointerDown(RawPointerButton),
    PointerUp(RawPointerButton),
    PointerCancel(RawPointerCancel),
    Wheel(RawWheel),
    KeyDown(RawKey),
    KeyUp(RawKey),
    WindowBlur(RawWindowEvent),
    WindowFocus(RawWindowEvent),
    ContextMenu(RawContextMenu),
    TextInput(RawTextInput),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawPointerMove {
    pub position: Point,
    pub pointer_id: XuiPointerId,
    pub device_id: Option<XuiDeviceId>,
    pub kind: PointerKind,
    pub button: Option<PointerButton>,
    pub buttons: PointerButtons,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawPointerButton {
    pub position: Point,
    pub pointer_id: XuiPointerId,
    pub device_id: Option<XuiDeviceId>,
    pub kind: PointerKind,
    pub button: PointerButton,
    pub buttons: PointerButtons,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawPointerCancel {
    pub pointer_id: XuiPointerId,
    pub device_id: Option<XuiDeviceId>,
    pub kind: PointerKind,
    pub position: Option<Point>,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawWheel {
    pub position: Point,
    pub delta: ScrollDelta,
    pub device_id: Option<XuiDeviceId>,
    pub pointer_id: Option<XuiPointerId>,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
    pub is_inertial: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawKey {
    pub key: Key,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
    pub is_repeat: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawWindowEvent {
    pub timestamp: Instant,
    pub modifiers: Modifiers,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawContextMenu {
    pub trigger: ContextMenuTrigger,
    pub pointer: Option<PointerSnapshot>,
    pub position: Option<Point>,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawTextInput {
    pub text: String,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
}

impl RawEvent {
    pub fn pointer_position(&self) -> Option<Point> {
        match self {
            Self::PointerMove(event) => Some(event.position),
            Self::PointerDown(event) | Self::PointerUp(event) => Some(event.position),
            Self::PointerCancel(event) => event.position,
            Self::Wheel(event) => Some(event.position),
            Self::ContextMenu(event) => event.position,
            _ => None,
        }
    }
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
