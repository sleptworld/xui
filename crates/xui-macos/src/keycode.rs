//! AppKit virtual key codes (`kVK_*` in `Carbon/HIToolbox/Events.h`).
//!
//! Key codes name a key's position, not what the layout prints on it, so they
//! map straight onto `PhysicalKey`. Named keys are position-bound too -- an
//! arrow is an arrow on every layout -- so they come from the same table; only
//! text depends on the layout, and that comes from the input context.

use xui_interface::events::{NamedKey, PhysicalKey};

// Device-dependent bits of `modifierFlags` (`NX_DEVICE*` in IOKit's
// `IOLLEvent.h`). AppKit's documented flags say that a modifier is down;
// these say which of the pair it is.
const LEFT_CONTROL: usize = 0x0000_0001;
const LEFT_SHIFT: usize = 0x0000_0002;
const RIGHT_SHIFT: usize = 0x0000_0004;
const LEFT_COMMAND: usize = 0x0000_0008;
const RIGHT_COMMAND: usize = 0x0000_0010;
const LEFT_ALT: usize = 0x0000_0020;
const RIGHT_ALT: usize = 0x0000_0040;
const RIGHT_CONTROL: usize = 0x0000_2000;

// The documented flags, as fallbacks.
const SHIFT: usize = 1 << 17;
const CONTROL: usize = 1 << 18;
const ALT: usize = 1 << 19;
const COMMAND: usize = 1 << 20;

pub(crate) fn physical_key(code: u16) -> PhysicalKey {
    use PhysicalKey as K;
    match code {
        0x00 => K::KeyA,
        0x01 => K::KeyS,
        0x02 => K::KeyD,
        0x03 => K::KeyF,
        0x04 => K::KeyH,
        0x05 => K::KeyG,
        0x06 => K::KeyZ,
        0x07 => K::KeyX,
        0x08 => K::KeyC,
        0x09 => K::KeyV,
        0x0A => K::IntlBackslash,
        0x0B => K::KeyB,
        0x0C => K::KeyQ,
        0x0D => K::KeyW,
        0x0E => K::KeyE,
        0x0F => K::KeyR,
        0x10 => K::KeyY,
        0x11 => K::KeyT,
        0x12 => K::Digit1,
        0x13 => K::Digit2,
        0x14 => K::Digit3,
        0x15 => K::Digit4,
        0x16 => K::Digit6,
        0x17 => K::Digit5,
        0x18 => K::Equal,
        0x19 => K::Digit9,
        0x1A => K::Digit7,
        0x1B => K::Minus,
        0x1C => K::Digit8,
        0x1D => K::Digit0,
        0x1E => K::BracketRight,
        0x1F => K::KeyO,
        0x20 => K::KeyU,
        0x21 => K::BracketLeft,
        0x22 => K::KeyI,
        0x23 => K::KeyP,
        0x24 => K::Enter,
        0x25 => K::KeyL,
        0x26 => K::KeyJ,
        0x27 => K::Quote,
        0x28 => K::KeyK,
        0x29 => K::Semicolon,
        0x2A => K::Backslash,
        0x2B => K::Comma,
        0x2C => K::Slash,
        0x2D => K::KeyN,
        0x2E => K::KeyM,
        0x2F => K::Period,
        0x30 => K::Tab,
        0x31 => K::Space,
        0x32 => K::Backquote,
        0x33 => K::Backspace,
        0x34 => K::NumpadEnter,
        0x35 => K::Escape,
        0x36 => K::SuperRight,
        0x37 => K::SuperLeft,
        0x38 => K::ShiftLeft,
        0x39 => K::CapsLock,
        0x3A => K::AltLeft,
        0x3B => K::ControlLeft,
        0x3C => K::ShiftRight,
        0x3D => K::AltRight,
        0x3E => K::ControlRight,
        0x3F => K::Fn,
        0x40 => K::F17,
        0x41 => K::NumpadDecimal,
        0x43 => K::NumpadMultiply,
        0x45 => K::NumpadAdd,
        0x47 => K::NumLock,
        0x48 => K::AudioVolumeUp,
        0x49 => K::AudioVolumeDown,
        0x4A => K::AudioVolumeMute,
        0x4B => K::NumpadDivide,
        0x4C => K::NumpadEnter,
        0x4E => K::NumpadSubtract,
        0x4F => K::F18,
        0x50 => K::F19,
        0x51 => K::NumpadEqual,
        0x52 => K::Numpad0,
        0x53 => K::Numpad1,
        0x54 => K::Numpad2,
        0x55 => K::Numpad3,
        0x56 => K::Numpad4,
        0x57 => K::Numpad5,
        0x58 => K::Numpad6,
        0x59 => K::Numpad7,
        0x5A => K::F20,
        0x5B => K::Numpad8,
        0x5C => K::Numpad9,
        0x5D => K::IntlYen,
        0x5E => K::IntlRo,
        0x5F => K::NumpadComma,
        0x60 => K::F5,
        0x61 => K::F6,
        0x62 => K::F7,
        0x63 => K::F3,
        0x64 => K::F8,
        0x65 => K::F9,
        0x66 => K::Lang2,
        0x67 => K::F11,
        0x68 => K::Lang1,
        0x69 => K::F13,
        0x6A => K::F16,
        0x6B => K::F14,
        0x6D => K::F10,
        0x6E => K::ContextMenu,
        0x6F => K::F12,
        0x71 => K::F15,
        // kVK_Help sits where Insert is on a PC keyboard.
        0x72 => K::Insert,
        0x73 => K::Home,
        0x74 => K::PageUp,
        0x75 => K::Delete,
        0x76 => K::F4,
        0x77 => K::End,
        0x78 => K::F2,
        0x79 => K::PageDown,
        0x7A => K::F1,
        0x7B => K::ArrowLeft,
        0x7C => K::ArrowRight,
        0x7D => K::ArrowDown,
        0x7E => K::ArrowUp,
        _ => K::Unidentified,
    }
}

pub(crate) fn named_key(code: u16) -> Option<NamedKey> {
    Some(match code {
        0x30 => NamedKey::Tab,
        0x24 | 0x34 | 0x4C => NamedKey::Enter,
        0x35 => NamedKey::Escape,
        0x33 => NamedKey::Backspace,
        0x75 => NamedKey::Delete,
        0x7B => NamedKey::ArrowLeft,
        0x7C => NamedKey::ArrowRight,
        0x7D => NamedKey::ArrowDown,
        0x7E => NamedKey::ArrowUp,
        0x73 => NamedKey::Home,
        0x77 => NamedKey::End,
        0x74 => NamedKey::PageUp,
        0x79 => NamedKey::PageDown,
        0x31 => NamedKey::Space,
        0x6E => NamedKey::ContextMenu,
        0x38 | 0x3C => NamedKey::Shift,
        0x3B | 0x3E => NamedKey::Control,
        0x3A | 0x3D => NamedKey::Alt,
        0x37 | 0x36 => NamedKey::Meta,
        0x7A => NamedKey::F(1),
        0x78 => NamedKey::F(2),
        0x63 => NamedKey::F(3),
        0x76 => NamedKey::F(4),
        0x60 => NamedKey::F(5),
        0x61 => NamedKey::F(6),
        0x62 => NamedKey::F(7),
        0x64 => NamedKey::F(8),
        0x65 => NamedKey::F(9),
        0x6D => NamedKey::F(10),
        0x67 => NamedKey::F(11),
        0x6F => NamedKey::F(12),
        0x69 => NamedKey::F(13),
        0x6B => NamedKey::F(14),
        0x71 => NamedKey::F(15),
        0x6A => NamedKey::F(16),
        0x40 => NamedKey::F(17),
        0x4F => NamedKey::F(18),
        0x50 => NamedKey::F(19),
        0x5A => NamedKey::F(20),
        _ => return None,
    })
}

/// For a modifier key's `flagsChanged:`, whether that key is now down.
///
/// `flags` is the event's raw `modifierFlags`. The side-specific bit is what
/// decides, so releasing one Shift while the other is held reports the release.
/// `None` for keys that are not tracked modifiers.
pub(crate) fn modifier_pressed(code: u16, flags: usize) -> Option<bool> {
    let (side, pair, shared) = match code {
        0x38 => (LEFT_SHIFT, LEFT_SHIFT | RIGHT_SHIFT, SHIFT),
        0x3C => (RIGHT_SHIFT, LEFT_SHIFT | RIGHT_SHIFT, SHIFT),
        0x3B => (LEFT_CONTROL, LEFT_CONTROL | RIGHT_CONTROL, CONTROL),
        0x3E => (RIGHT_CONTROL, LEFT_CONTROL | RIGHT_CONTROL, CONTROL),
        0x3A => (LEFT_ALT, LEFT_ALT | RIGHT_ALT, ALT),
        0x3D => (RIGHT_ALT, LEFT_ALT | RIGHT_ALT, ALT),
        0x37 => (LEFT_COMMAND, LEFT_COMMAND | RIGHT_COMMAND, COMMAND),
        0x36 => (RIGHT_COMMAND, LEFT_COMMAND | RIGHT_COMMAND, COMMAND),
        _ => return None,
    };
    // Synthesized events -- assistive software, key remappers -- can leave the
    // device bits clear, and then the combined flag is all there is to go on.
    Some(if flags & pair != 0 {
        flags & side != 0
    } else {
        flags & shared != 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_positions_not_layout() {
        assert_eq!(physical_key(0x00), PhysicalKey::KeyA);
        assert_eq!(physical_key(0x0C), PhysicalKey::KeyQ);
        assert_eq!(physical_key(0x4C), PhysicalKey::NumpadEnter);
        assert_eq!(physical_key(0xFF), PhysicalKey::Unidentified);
    }

    #[test]
    fn maps_named_keys() {
        assert_eq!(named_key(0x24), Some(NamedKey::Enter));
        assert_eq!(named_key(0x7B), Some(NamedKey::ArrowLeft));
        assert_eq!(named_key(0x63), Some(NamedKey::F(3)));
        assert_eq!(named_key(0x00), None);
    }

    #[test]
    fn modifier_keys_read_their_own_side() {
        // The left Shift is released while the right one is still held.
        let right_shift_held = SHIFT | RIGHT_SHIFT;
        assert_eq!(modifier_pressed(0x38, right_shift_held), Some(false));
        assert_eq!(modifier_pressed(0x3C, right_shift_held), Some(true));
        assert_eq!(modifier_pressed(0x00, right_shift_held), None);
    }

    #[test]
    fn a_released_modifier_is_up() {
        assert_eq!(modifier_pressed(0x38, 0), Some(false));
        assert_eq!(modifier_pressed(0x37, LEFT_COMMAND | COMMAND), Some(true));
    }

    #[test]
    fn events_without_device_bits_fall_back_to_the_combined_flag() {
        assert_eq!(modifier_pressed(0x3A, ALT), Some(true));
        assert_eq!(modifier_pressed(0x3D, ALT), Some(true));
        assert_eq!(modifier_pressed(0x3A, 0), Some(false));
    }
}
