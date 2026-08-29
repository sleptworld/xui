use crate::{NodeId, Point, TextRange, Translation};
use std::sync::Arc;
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

#[derive(Debug, Clone, PartialEq)]
pub enum RawEvent {
    PointerMove(RawPointerMove),
    PointerDown(RawPointerButton),
    PointerUp(RawPointerButton),
    PointerCancel(RawPointerCancel),
    Wheel(RawWheel),
    WindowBlur(RawWindowEvent),
    WindowFocus(RawWindowEvent),
    ContextMenu(RawContextMenu),
    Keyboard(RawKeyboard),
    Ime(RawIme),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyState {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyText {
    inner: fixedstr::str32,
}

impl KeyText {
    pub fn try_new(text: &str) -> Option<Self> {
        if text.len() <= 31 {
            Some(Self {
                inner: fixedstr::str32::from(text),
            })
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &str {
        self.inner.to_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Tab,
    Enter,
    Escape,
    Backspace,
    Delete,

    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,

    Home,
    End,
    PageUp,
    PageDown,

    Space,
    ContextMenu,

    Shift,
    Control,
    Alt,
    Meta,

    F(u8),

    Unidentified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicalKey {
    // Writing system keys
    Backquote,
    Backslash,
    BracketLeft,
    BracketRight,
    Comma,

    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    Equal,
    IntlBackslash,
    IntlRo,
    IntlYen,

    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,

    Minus,
    Period,
    Quote,
    Semicolon,
    Slash,

    // Modifier / control keys
    AltLeft,
    AltRight,
    Backspace,
    CapsLock,
    ContextMenu,
    ControlLeft,
    ControlRight,
    Enter,
    SuperLeft,
    SuperRight,
    ShiftLeft,
    ShiftRight,
    Space,
    Tab,

    // IME / language keys
    Convert,
    KanaMode,
    Lang1,
    Lang2,
    Lang3,
    Lang4,
    Lang5,
    NonConvert,

    // Navigation / editing
    Delete,
    End,
    Help,
    Home,
    Insert,
    PageDown,
    PageUp,

    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,

    // Numpad
    NumLock,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,

    NumpadAdd,
    NumpadBackspace,
    NumpadClear,
    NumpadClearEntry,
    NumpadComma,
    NumpadDecimal,
    NumpadDivide,
    NumpadEnter,
    NumpadEqual,
    NumpadHash,
    NumpadMemoryAdd,
    NumpadMemoryClear,
    NumpadMemoryRecall,
    NumpadMemoryStore,
    NumpadMemorySubtract,
    NumpadMultiply,
    NumpadParenLeft,
    NumpadParenRight,
    NumpadStar,
    NumpadSubtract,

    // System / function
    Escape,
    Fn,
    FnLock,
    PrintScreen,
    ScrollLock,
    Pause,

    // Browser / app launch
    BrowserBack,
    BrowserFavorites,
    BrowserForward,
    BrowserHome,
    BrowserRefresh,
    BrowserSearch,
    BrowserStop,

    Eject,
    LaunchApp1,
    LaunchApp2,
    LaunchMail,

    // Media
    MediaPlayPause,
    MediaSelect,
    MediaStop,
    MediaTrackNext,
    MediaTrackPrevious,

    // Power
    Power,
    Sleep,
    WakeUp,

    // Audio
    AudioVolumeDown,
    AudioVolumeMute,
    AudioVolumeUp,

    // Extra modifiers / system keys
    Meta,
    Hyper,
    Turbo,

    // Editing / application command keys
    Abort,
    Resume,
    Suspend,
    Again,
    Copy,
    Cut,
    Find,
    Open,
    Paste,
    Props,
    Select,
    Undo,

    // Japanese-specific
    Hiragana,
    Katakana,

    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    F25,
    F26,
    F27,
    F28,
    F29,
    F30,
    F31,
    F32,
    F33,
    F34,
    F35,

    Native(u32),

    Unidentified,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawKeyboard {
    pub physical_key: PhysicalKey,
    pub named_key: Option<NamedKey>,
    pub state: KeyState,
    pub text: Option<KeyText>,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
    pub is_repeat: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextPayload {
    Small(KeyText),
    Shared(Arc<str>),
}

impl TextPayload {
    pub fn new(text: &str) -> Self {
        if let Some(small) = KeyText::try_new(text) {
            Self::Small(small)
        } else {
            Self::Shared(Arc::<str>::from(text))
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Small(s) => s.as_str(),
            Self::Shared(s) => s.as_ref(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawIme {
    Enabled {
        timestamp: Instant,
    },

    Preedit {
        text: TextPayload,
        /// 相对于 preedit text 的 byte range
        cursor: Option<TextRange>,
        timestamp: Instant,
    },

    Commit {
        text: TextPayload,
        timestamp: Instant,
    },

    Disabled {
        timestamp: Instant,
    },
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawKeyInput {
    pub text: fixedstr::str32,
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
