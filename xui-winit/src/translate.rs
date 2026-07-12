use winit::event::{
    ElementState, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::keyboard::{
    Key as WinitKey, KeyCode, NamedKey as WinitNamedKey, PhysicalKey as WinitPhysicalKey,
};
use xui::prelude::{RuntimeEvent, Size};
use xui_interface::events::{KeyState, KeyText, NamedKey, PhysicalKey, RawEvent, RawKeyboard};
use xui_interface::{
    Event, Modifiers, Point, PointerButton, PointerButtons, PointerKind, RawPointerButton,
    RawPointerMove, RawWheel, RawWindowEvent, ScrollDelta, Translation, events::XuiPointerId,
};
use xui_interface::{RawIme, TextOffset, TextPayload, TextRange};

pub fn translate_mouse_button(button: WinitMouseButton) -> Option<PointerButton> {
    match button {
        WinitMouseButton::Left => Some(PointerButton::Primary),
        WinitMouseButton::Right => Some(PointerButton::Secondary),
        WinitMouseButton::Middle => Some(PointerButton::Auxiliary),
        WinitMouseButton::Back => Some(PointerButton::Back),
        WinitMouseButton::Forward => Some(PointerButton::Forward),
        WinitMouseButton::Other(button) => Some(PointerButton::Other(button)),
    }
}

pub fn translate_named_key(key: &WinitKey) -> Option<NamedKey> {
    let WinitKey::Named(key) = key else {
        return None;
    };
    Some(match key {
        WinitNamedKey::Tab => NamedKey::Tab,
        WinitNamedKey::Enter => NamedKey::Enter,
        WinitNamedKey::Escape => NamedKey::Escape,
        WinitNamedKey::Backspace => NamedKey::Backspace,
        WinitNamedKey::Delete => NamedKey::Delete,
        WinitNamedKey::ArrowLeft => NamedKey::ArrowLeft,
        WinitNamedKey::ArrowRight => NamedKey::ArrowRight,
        WinitNamedKey::ArrowUp => NamedKey::ArrowUp,
        WinitNamedKey::ArrowDown => NamedKey::ArrowDown,
        WinitNamedKey::Home => NamedKey::Home,
        WinitNamedKey::End => NamedKey::End,
        WinitNamedKey::PageUp => NamedKey::PageUp,
        WinitNamedKey::PageDown => NamedKey::PageDown,
        WinitNamedKey::Space => NamedKey::Space,
        WinitNamedKey::ContextMenu => NamedKey::ContextMenu,
        WinitNamedKey::Shift => NamedKey::Shift,
        WinitNamedKey::Control => NamedKey::Control,
        WinitNamedKey::Alt => NamedKey::Alt,
        WinitNamedKey::Super | WinitNamedKey::Meta | WinitNamedKey::Hyper => NamedKey::Meta,
        WinitNamedKey::F1 => NamedKey::F(1),
        WinitNamedKey::F2 => NamedKey::F(2),
        WinitNamedKey::F3 => NamedKey::F(3),
        WinitNamedKey::F4 => NamedKey::F(4),
        WinitNamedKey::F5 => NamedKey::F(5),
        WinitNamedKey::F6 => NamedKey::F(6),
        WinitNamedKey::F7 => NamedKey::F(7),
        WinitNamedKey::F8 => NamedKey::F(8),
        WinitNamedKey::F9 => NamedKey::F(9),
        WinitNamedKey::F10 => NamedKey::F(10),
        WinitNamedKey::F11 => NamedKey::F(11),
        WinitNamedKey::F12 => NamedKey::F(12),
        WinitNamedKey::F13 => NamedKey::F(13),
        WinitNamedKey::F14 => NamedKey::F(14),
        WinitNamedKey::F15 => NamedKey::F(15),
        WinitNamedKey::F16 => NamedKey::F(16),
        WinitNamedKey::F17 => NamedKey::F(17),
        WinitNamedKey::F18 => NamedKey::F(18),
        WinitNamedKey::F19 => NamedKey::F(19),
        WinitNamedKey::F20 => NamedKey::F(20),
        WinitNamedKey::F21 => NamedKey::F(21),
        WinitNamedKey::F22 => NamedKey::F(22),
        WinitNamedKey::F23 => NamedKey::F(23),
        WinitNamedKey::F24 => NamedKey::F(24),
        WinitNamedKey::F25 => NamedKey::F(25),
        WinitNamedKey::F26 => NamedKey::F(26),
        WinitNamedKey::F27 => NamedKey::F(27),
        WinitNamedKey::F28 => NamedKey::F(28),
        WinitNamedKey::F29 => NamedKey::F(29),
        WinitNamedKey::F30 => NamedKey::F(30),
        WinitNamedKey::F31 => NamedKey::F(31),
        WinitNamedKey::F32 => NamedKey::F(32),
        WinitNamedKey::F33 => NamedKey::F(33),
        WinitNamedKey::F34 => NamedKey::F(34),
        WinitNamedKey::F35 => NamedKey::F(35),
        _ => NamedKey::Unidentified,
    })
}

pub fn translate_physical_key(key: WinitPhysicalKey) -> PhysicalKey {
    let WinitPhysicalKey::Code(code) = key else {
        return PhysicalKey::Unidentified;
    };
    macro_rules! map_codes { ($($name:ident),* $(,)?) => { match code { $(KeyCode::$name => PhysicalKey::$name,)* _ => PhysicalKey::Unidentified } }; }
    map_codes!(
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
        Convert,
        KanaMode,
        Lang1,
        Lang2,
        Lang3,
        Lang4,
        Lang5,
        NonConvert,
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
        Escape,
        Fn,
        FnLock,
        PrintScreen,
        ScrollLock,
        Pause,
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
        MediaPlayPause,
        MediaSelect,
        MediaStop,
        MediaTrackNext,
        MediaTrackPrevious,
        Power,
        Sleep,
        WakeUp,
        AudioVolumeDown,
        AudioVolumeMute,
        AudioVolumeUp,
        Meta,
        Hyper,
        Turbo,
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
        Hiragana,
        Katakana,
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
        F35
    )
}

pub fn translate_mouse_wheel(scale: f32, delta: &MouseScrollDelta) -> ScrollDelta {
    let scale = scale.max(1.0);
    match delta {
        MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines(Translation::new(*x, *y)),
        MouseScrollDelta::PixelDelta(position) => ScrollDelta::Pixels(
            Point::new(position.x as f32, position.y as f32)
                .scale(1.0 / scale)
                .into(),
        ),
    }
}

pub fn translate_window_event(
    scale: f32,
    event: &WindowEvent,
    last_cursor_position: Option<Point>,
) -> Vec<RuntimeEvent> {
    match event {
        WindowEvent::Resized(size) => vec![RuntimeEvent::Resize(Size::<f32>::new(
            size.width as f32,
            size.height as f32,
        ))],
        WindowEvent::CloseRequested | WindowEvent::Destroyed => vec![RuntimeEvent::Exit],
        WindowEvent::Focused(true) => {
            vec![RuntimeEvent::Input(RawEvent::WindowFocus(RawWindowEvent {
                timestamp: std::time::Instant::now(),
                modifiers: Modifiers::default(),
            }))]
        }
        WindowEvent::Focused(false) => {
            vec![RuntimeEvent::Input(RawEvent::WindowBlur(RawWindowEvent {
                timestamp: std::time::Instant::now(),
                modifiers: Modifiers::default(),
            }))]
        }
        WindowEvent::CursorMoved { position, .. } => {
            vec![RuntimeEvent::Input(RawEvent::PointerMove(RawPointerMove {
                position: Point::new(position.x as f32, position.y as f32).scale(1. / scale),
                pointer_id: XuiPointerId::new(0),
                device_id: None,
                kind: PointerKind::Mouse,
                button: None,
                buttons: PointerButtons::default(),
                modifiers: Modifiers::default(),
                timestamp: std::time::Instant::now(),
            }))]
        }
        WindowEvent::MouseInput { state, button, .. } => {
            let Some(button) = translate_mouse_button(*button) else {
                return Vec::new();
            };
            let position = last_cursor_position.unwrap_or(Point::new(0.0, 0.0));
            let buttons = match state {
                ElementState::Pressed => PointerButtons::from_button(button),
                ElementState::Released => PointerButtons::default(),
            };
            let event = RawPointerButton {
                position,
                pointer_id: XuiPointerId::new(0),
                device_id: None,
                kind: PointerKind::Mouse,
                button,
                buttons,
                modifiers: Modifiers::default(),
                timestamp: std::time::Instant::now(),
            };
            let event = match state {
                ElementState::Pressed => RawEvent::PointerDown(event),
                ElementState::Released => RawEvent::PointerUp(event),
            };
            vec![RuntimeEvent::Input(event)]
        }
        WindowEvent::MouseWheel { delta, .. } => {
            vec![RuntimeEvent::Input(RawEvent::Wheel(RawWheel {
                position: last_cursor_position.unwrap_or(Point::new(0.0, 0.0)),
                delta: translate_mouse_wheel(scale, delta),
                device_id: None,
                pointer_id: Some(XuiPointerId::new(0)),
                modifiers: Modifiers::default(),
                timestamp: std::time::Instant::now(),
                is_inertial: false,
            }))]
        }
        WindowEvent::KeyboardInput { event, .. } => translate_key_event(event),
        WindowEvent::Ime(ime) => translate_ime_event(ime),
        WindowEvent::RedrawRequested => vec![RuntimeEvent::RedrawRequested],
        _ => Vec::new(),
    }
}

fn translate_ime_event(event: &winit::event::Ime) -> Vec<RuntimeEvent> {
    let timestamp = std::time::Instant::now();
    let raw = match event {
        winit::event::Ime::Commit(text) => RawIme::Commit {
            text: TextPayload::new(text),
            timestamp,
        },
        winit::event::Ime::Disabled => RawIme::Disabled { timestamp },
        winit::event::Ime::Enabled => RawIme::Enabled { timestamp },
        winit::event::Ime::Preedit(a, b) => RawIme::Preedit {
            text: TextPayload::new(a),
            cursor: b.as_ref().map(|r| {
                TextRange::new(TextOffset::byte_offset(r.0), TextOffset::byte_offset(r.1))
            }),
            timestamp,
        },
    };

    vec![RuntimeEvent::Input(RawEvent::Ime(raw))]
}

fn translate_key_event(event: &KeyEvent) -> Vec<RuntimeEvent> {
    let raw = RawKeyboard {
        physical_key: translate_physical_key(event.physical_key),
        named_key: translate_named_key(&event.logical_key),
        state: match event.state {
            ElementState::Pressed => KeyState::Down,
            ElementState::Released => KeyState::Up,
        },
        text: event.text.as_deref().and_then(KeyText::try_new),
        modifiers: Modifiers::default(),
        timestamp: std::time::Instant::now(),
        is_repeat: event.repeat,
    };
    vec![RuntimeEvent::Input(RawEvent::Keyboard(raw))]
}

#[cfg(test)]
mod tests {
    use winit::event::{MouseButton as WinitMouseButton, MouseScrollDelta};
    use winit::keyboard::{Key as WinitKey, NamedKey};

    use super::*;

    #[test]
    fn maps_mouse_buttons_to_xui_pointer_buttons() {
        assert_eq!(
            translate_mouse_button(WinitMouseButton::Left),
            Some(PointerButton::Primary)
        );
        assert_eq!(
            translate_mouse_button(WinitMouseButton::Right),
            Some(PointerButton::Secondary)
        );
        assert_eq!(
            translate_mouse_button(WinitMouseButton::Middle),
            Some(PointerButton::Auxiliary)
        );
        assert_eq!(
            translate_mouse_button(WinitMouseButton::Other(9)),
            Some(PointerButton::Other(9))
        );
    }

    #[test]
    fn maps_named_keys() {
        assert_eq!(
            translate_named_key(&WinitKey::Named(NamedKey::Enter)),
            Some(xui_interface::events::NamedKey::Enter)
        );
        assert_eq!(translate_named_key(&WinitKey::Character("x".into())), None);
        assert_eq!(
            translate_named_key(&WinitKey::Named(NamedKey::ArrowLeft)),
            Some(xui_interface::events::NamedKey::ArrowLeft)
        );
        assert_eq!(
            translate_named_key(&WinitKey::Named(NamedKey::Delete)),
            Some(xui_interface::events::NamedKey::Delete)
        );
        assert_eq!(
            translate_named_key(&WinitKey::Named(NamedKey::Backspace)),
            Some(xui_interface::events::NamedKey::Backspace)
        );
    }

    #[test]
    fn maps_physical_keys() {
        assert_eq!(
            translate_physical_key(winit::keyboard::PhysicalKey::Code(KeyCode::KeyA)),
            PhysicalKey::KeyA
        );
        assert_eq!(
            translate_physical_key(winit::keyboard::PhysicalKey::Code(KeyCode::NumpadEnter)),
            PhysicalKey::NumpadEnter
        );
        assert_eq!(
            translate_physical_key(winit::keyboard::PhysicalKey::Code(KeyCode::Backspace)),
            PhysicalKey::Backspace
        );
    }

    #[test]
    fn maps_scroll_delta() {
        assert_eq!(
            translate_mouse_wheel(1.0, &MouseScrollDelta::LineDelta(1.0, -2.0)),
            ScrollDelta::Lines(Translation::new(1.0, -2.0))
        );
        assert_eq!(
            translate_mouse_wheel(
                2.0,
                &MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(20.0, -10.0))
            ),
            ScrollDelta::Pixels(Translation::new(10.0, -5.0))
        );
    }
}
