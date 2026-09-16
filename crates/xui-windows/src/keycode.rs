//! Win32 scan codes and virtual keys.
//!
//! A scan code names a key's position and is what the layout is applied *to*,
//! so it is what maps onto `PhysicalKey` -- the same reasoning as the macOS
//! host's virtual key codes. Named keys come from the virtual key, which
//! already means "the Enter key" rather than "the key at this spot".

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_APPS, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE,
    VK_F1, VK_F24, VK_HOME, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use xui_interface::Modifiers;
use xui_interface::events::{NamedKey, PhysicalKey};

/// Set-1 scan code plus the extended-key flag, which is what distinguishes the
/// keypad Enter from Return, right Alt from left, and the arrow cluster from
/// the numpad.
pub(crate) fn physical_key(scancode: u16, extended: bool) -> PhysicalKey {
    use PhysicalKey as K;
    if extended {
        return match scancode {
            0x1C => K::NumpadEnter,
            0x1D => K::ControlRight,
            0x35 => K::NumpadDivide,
            0x38 => K::AltRight,
            0x47 => K::Home,
            0x48 => K::ArrowUp,
            0x49 => K::PageUp,
            0x4B => K::ArrowLeft,
            0x4D => K::ArrowRight,
            0x4F => K::End,
            0x50 => K::ArrowDown,
            0x51 => K::PageDown,
            0x52 => K::Insert,
            0x53 => K::Delete,
            0x5B => K::SuperLeft,
            0x5C => K::SuperRight,
            0x5D => K::ContextMenu,
            0x45 => K::NumLock,
            _ => K::Unidentified,
        };
    }
    match scancode {
        0x01 => K::Escape,
        0x02 => K::Digit1,
        0x03 => K::Digit2,
        0x04 => K::Digit3,
        0x05 => K::Digit4,
        0x06 => K::Digit5,
        0x07 => K::Digit6,
        0x08 => K::Digit7,
        0x09 => K::Digit8,
        0x0A => K::Digit9,
        0x0B => K::Digit0,
        0x0C => K::Minus,
        0x0D => K::Equal,
        0x0E => K::Backspace,
        0x0F => K::Tab,
        0x10 => K::KeyQ,
        0x11 => K::KeyW,
        0x12 => K::KeyE,
        0x13 => K::KeyR,
        0x14 => K::KeyT,
        0x15 => K::KeyY,
        0x16 => K::KeyU,
        0x17 => K::KeyI,
        0x18 => K::KeyO,
        0x19 => K::KeyP,
        0x1A => K::BracketLeft,
        0x1B => K::BracketRight,
        0x1C => K::Enter,
        0x1D => K::ControlLeft,
        0x1E => K::KeyA,
        0x1F => K::KeyS,
        0x20 => K::KeyD,
        0x21 => K::KeyF,
        0x22 => K::KeyG,
        0x23 => K::KeyH,
        0x24 => K::KeyJ,
        0x25 => K::KeyK,
        0x26 => K::KeyL,
        0x27 => K::Semicolon,
        0x28 => K::Quote,
        0x29 => K::Backquote,
        0x2A => K::ShiftLeft,
        0x2B => K::Backslash,
        0x2C => K::KeyZ,
        0x2D => K::KeyX,
        0x2E => K::KeyC,
        0x2F => K::KeyV,
        0x30 => K::KeyB,
        0x31 => K::KeyN,
        0x32 => K::KeyM,
        0x33 => K::Comma,
        0x34 => K::Period,
        0x35 => K::Slash,
        0x36 => K::ShiftRight,
        0x37 => K::NumpadMultiply,
        0x38 => K::AltLeft,
        0x39 => K::Space,
        0x3A => K::CapsLock,
        0x3B => K::F1,
        0x3C => K::F2,
        0x3D => K::F3,
        0x3E => K::F4,
        0x3F => K::F5,
        0x40 => K::F6,
        0x41 => K::F7,
        0x42 => K::F8,
        0x43 => K::F9,
        0x44 => K::F10,
        0x45 => K::NumLock,
        0x46 => K::ScrollLock,
        0x47 => K::Numpad7,
        0x48 => K::Numpad8,
        0x49 => K::Numpad9,
        0x4A => K::NumpadSubtract,
        0x4B => K::Numpad4,
        0x4C => K::Numpad5,
        0x4D => K::Numpad6,
        0x4E => K::NumpadAdd,
        0x4F => K::Numpad1,
        0x50 => K::Numpad2,
        0x51 => K::Numpad3,
        0x52 => K::Numpad0,
        0x53 => K::NumpadDecimal,
        0x56 => K::IntlBackslash,
        0x57 => K::F11,
        0x58 => K::F12,
        0x59 => K::NumpadEqual,
        0x70 => K::IntlRo,
        0x73 => K::IntlRo,
        0x79 => K::Convert,
        0x7B => K::NonConvert,
        0x7D => K::IntlYen,
        _ => K::Unidentified,
    }
}

pub(crate) fn named_key(vk: u16) -> Option<NamedKey> {
    let key = VIRTUAL_KEY(vk);
    Some(match key {
        VK_TAB => NamedKey::Tab,
        VK_RETURN => NamedKey::Enter,
        VK_ESCAPE => NamedKey::Escape,
        VK_BACK => NamedKey::Backspace,
        VK_DELETE => NamedKey::Delete,
        VK_LEFT => NamedKey::ArrowLeft,
        VK_RIGHT => NamedKey::ArrowRight,
        VK_UP => NamedKey::ArrowUp,
        VK_DOWN => NamedKey::ArrowDown,
        VK_HOME => NamedKey::Home,
        VK_END => NamedKey::End,
        VK_PRIOR => NamedKey::PageUp,
        VK_NEXT => NamedKey::PageDown,
        VK_SPACE => NamedKey::Space,
        VK_APPS => NamedKey::ContextMenu,
        VK_SHIFT => NamedKey::Shift,
        VK_CONTROL => NamedKey::Control,
        VK_MENU => NamedKey::Alt,
        VK_LWIN | VK_RWIN => NamedKey::Meta,
        _ => {
            if (VK_F1.0..=VK_F24.0).contains(&vk) {
                NamedKey::F((vk - VK_F1.0 + 1) as u8)
            } else {
                return None;
            }
        }
    })
}

/// The modifier state, read from the keyboard rather than tracked, because
/// Windows reports no modifier-change message of its own -- the flags are
/// whatever they are when a key or mouse message arrives.
pub(crate) fn modifiers() -> Modifiers {
    Modifiers {
        shift: is_down(VK_SHIFT),
        ctrl: is_down(VK_CONTROL),
        alt: is_down(VK_MENU),
        meta: is_down(VK_LWIN) || is_down(VK_RWIN),
    }
}

fn is_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: no preconditions. The high bit is the "currently down" bit.
    (unsafe { GetKeyState(i32::from(key.0)) } as u16 & 0x8000) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_positions_not_layout() {
        assert_eq!(physical_key(0x1E, false), PhysicalKey::KeyA);
        assert_eq!(physical_key(0x10, false), PhysicalKey::KeyQ);
        assert_eq!(physical_key(0xFE, false), PhysicalKey::Unidentified);
    }

    #[test]
    fn the_extended_flag_splits_the_duplicated_keys() {
        assert_eq!(physical_key(0x1C, false), PhysicalKey::Enter);
        assert_eq!(physical_key(0x1C, true), PhysicalKey::NumpadEnter);
        assert_eq!(physical_key(0x38, false), PhysicalKey::AltLeft);
        assert_eq!(physical_key(0x38, true), PhysicalKey::AltRight);
    }

    #[test]
    fn maps_named_keys() {
        assert_eq!(named_key(VK_RETURN.0), Some(NamedKey::Enter));
        assert_eq!(named_key(VK_F1.0), Some(NamedKey::F(1)));
        assert_eq!(named_key(VK_F24.0), Some(NamedKey::F(24)));
        assert_eq!(named_key(b'A' as u16), None);
    }
}
