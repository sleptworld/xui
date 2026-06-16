use winit::event::{
    ElementState, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::keyboard::{Key as WinitKey, NamedKey};
use xui::prelude::{RuntimeEvent, Size};
use xui_interface::events::RawEvent;
use xui_interface::{
    Event, InputKey, Modifiers, Point, PointerButton, PointerButtons, PointerKind, RawKey,
    RawPointerButton, RawPointerMove, RawTextInput, RawWheel, RawWindowEvent, ScrollDelta,
    Translation, events::XuiPointerId,
};

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

pub fn translate_key(key: &WinitKey) -> InputKey {
    match key {
        WinitKey::Named(NamedKey::Tab) => InputKey::Tab,
        WinitKey::Named(NamedKey::Enter) => InputKey::Enter,
        WinitKey::Named(NamedKey::Escape) => InputKey::Escape,
        WinitKey::Named(NamedKey::Backspace) => InputKey::Backspace,
        WinitKey::Named(named) => InputKey::Other(format!("{named:?}")),
        WinitKey::Character(text) => InputKey::Character(text.to_string()),
        WinitKey::Unidentified(native) => InputKey::Other(format!("{native:?}")),
        WinitKey::Dead(Some(ch)) => InputKey::Character(ch.to_string()),
        WinitKey::Dead(None) => InputKey::Other("Dead".to_owned()),
    }
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
        WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
            vec![RuntimeEvent::Input(RawEvent::TextInput(RawTextInput {
                text: text.clone(),
                modifiers: Modifiers::default(),
                timestamp: std::time::Instant::now(),
            }))]
        }
        WindowEvent::RedrawRequested => vec![RuntimeEvent::RedrawRequested],
        _ => Vec::new(),
    }

}

pub fn translate_key_event(event: &KeyEvent) -> Vec<RuntimeEvent> {
    let key = translate_key(&event.logical_key);
    let raw = RawKey {
        key,
        modifiers: Modifiers::default(),
        timestamp: std::time::Instant::now(),
        is_repeat: event.repeat,
    };
    let mut events = vec![RuntimeEvent::Input(match event.state {
        ElementState::Pressed => RawEvent::KeyDown(raw),
        ElementState::Released => RawEvent::KeyUp(raw),
    })];

    if event.state == ElementState::Pressed {
        if let Some(text) = event.text.as_ref() {
            if !text.is_empty() && text != "\r" && text != "\t" {
                events.push(RuntimeEvent::Input(RawEvent::TextInput(RawTextInput {
                    text: text.to_string(),
                    modifiers: Modifiers::default(),
                    timestamp: std::time::Instant::now(),
                })));
            }
        }
    }

    events
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
    fn maps_named_and_character_keys() {
        assert_eq!(
            translate_key(&WinitKey::Named(NamedKey::Enter)),
            InputKey::Enter
        );
        assert_eq!(
            translate_key(&WinitKey::Named(NamedKey::Tab)),
            InputKey::Tab
        );
        assert_eq!(
            translate_key(&WinitKey::Character("x".into())),
            InputKey::Character("x".to_owned())
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
