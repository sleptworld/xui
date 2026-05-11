use winit::event::{
    ElementState, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::keyboard::{Key as WinitKey, NamedKey};
use xui::prelude::{RuntimeEvent, Size};
use xui_interface::{Event, InputKey, Point, PointerButton};

pub fn translate_mouse_button(button: WinitMouseButton) -> Option<PointerButton> {
    match button {
        WinitMouseButton::Left => Some(PointerButton::Primary),
        WinitMouseButton::Right => Some(PointerButton::Secondary),
        WinitMouseButton::Middle => Some(PointerButton::Middle),
        WinitMouseButton::Back | WinitMouseButton::Forward | WinitMouseButton::Other(_) => None,
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

pub fn translate_mouse_wheel(delta: &MouseScrollDelta) -> Point {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => Point::new(*x, *y),
        MouseScrollDelta::PixelDelta(position) => Point::new(position.x as f32, position.y as f32),
    }
}

pub fn translate_window_event(
    event: &WindowEvent,
    last_cursor_position: Option<Point>,
) -> Vec<RuntimeEvent> {
    match event {
        WindowEvent::Resized(size) => vec![RuntimeEvent::Resize(Size::new(
            size.width as f32,
            size.height as f32,
        ))],
        WindowEvent::CloseRequested | WindowEvent::Destroyed => vec![RuntimeEvent::Exit],
        WindowEvent::Focused(true) => vec![RuntimeEvent::Input(Event::FocusGained)],
        WindowEvent::Focused(false) => vec![RuntimeEvent::Input(Event::FocusLost)],
        WindowEvent::CursorMoved { position, .. } => {
            vec![RuntimeEvent::Input(Event::PointerMove {
                position: Point::new(position.x as f32, position.y as f32),
            })]
        }
        WindowEvent::MouseInput { state, button, .. } => {
            let Some(button) = translate_mouse_button(*button) else {
                return Vec::new();
            };
            let position = last_cursor_position.unwrap_or(Point::new(0.0, 0.0));
            let event = match state {
                ElementState::Pressed => Event::PointerDown { position, button },
                ElementState::Released => Event::PointerUp { position, button },
            };
            vec![RuntimeEvent::Input(event)]
        }
        WindowEvent::MouseWheel { delta, .. } => {
            vec![RuntimeEvent::Input(Event::Wheel {
                position: last_cursor_position.unwrap_or(Point::new(0.0, 0.0)),
                delta: translate_mouse_wheel(delta),
            })]
        }
        WindowEvent::KeyboardInput { event, .. } => translate_key_event(event),
        WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
            vec![RuntimeEvent::Input(Event::TextInput { text: text.clone() })]
        }
        WindowEvent::RedrawRequested => vec![RuntimeEvent::RedrawRequested],
        _ => Vec::new(),
    }
}

fn translate_key_event(event: &KeyEvent) -> Vec<RuntimeEvent> {
    let key = translate_key(&event.logical_key);
    let mut events = vec![RuntimeEvent::Input(match event.state {
        ElementState::Pressed => Event::KeyDown { key },
        ElementState::Released => Event::KeyUp { key },
    })];

    if event.state == ElementState::Pressed {
        if let Some(text) = event.text.as_ref() {
            if !text.is_empty() && text != "\r" && text != "\t" {
                events.push(RuntimeEvent::Input(Event::TextInput {
                    text: text.to_string(),
                }));
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
        assert_eq!(translate_mouse_button(WinitMouseButton::Other(9)), None);
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
            translate_mouse_wheel(&MouseScrollDelta::LineDelta(1.0, -2.0)),
            Point::new(1.0, -2.0)
        );
    }
}
