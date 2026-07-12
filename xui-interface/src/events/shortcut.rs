use super::{KeyState, Modifiers, NamedKey, PhysicalKey, RawKeyboard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutKey {
    Physical(PhysicalKey),
    Named(NamedKey),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ShortcutModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub key: ShortcutKey,
    pub modifiers: ShortcutModifiers,
    pub allow_repeat: bool,
}

impl Shortcut {
    pub const fn physical(key: PhysicalKey) -> Self {
        Self {
            key: ShortcutKey::Physical(key),
            modifiers: ShortcutModifiers {
                shift: false,
                alt: false,
                control: false,
                meta: false,
                primary: false,
            },
            allow_repeat: false,
        }
    }

    pub const fn named(key: NamedKey) -> Self {
        Self {
            key: ShortcutKey::Named(key),
            modifiers: ShortcutModifiers {
                shift: false,
                alt: false,
                control: false,
                meta: false,
                primary: false,
            },
            allow_repeat: false,
        }
    }

    pub const fn modifiers(mut self, modifiers: ShortcutModifiers) -> Self {
        self.modifiers = modifiers;
        self
    }
    pub const fn allow_repeat(mut self, allow: bool) -> Self {
        self.allow_repeat = allow;
        self
    }

    pub fn matches(self, event: &RawKeyboard) -> bool {
        if event.state != KeyState::Down || (event.is_repeat && !self.allow_repeat) {
            return false;
        }
        let key_matches = match self.key {
            ShortcutKey::Physical(key) => event.physical_key == key,
            ShortcutKey::Named(key) => event.named_key == Some(key),
        };
        key_matches && self.resolved_modifiers() == event.modifiers
    }

    pub fn resolved_modifiers(self) -> Modifiers {
        let mut modifiers = Modifiers {
            shift: self.modifiers.shift,
            ctrl: self.modifiers.control,
            alt: self.modifiers.alt,
            meta: self.modifiers.meta,
        };
        if self.modifiers.primary {
            if cfg!(target_os = "macos") {
                modifiers.meta = true;
            } else {
                modifiers.ctrl = true;
            }
        }
        modifiers
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShortcutBinding {
    pub shortcut: Shortcut,
    pub command: CommandId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn event(modifiers: Modifiers, repeat: bool) -> RawKeyboard {
        RawKeyboard {
            physical_key: PhysicalKey::KeyS,
            named_key: None,
            state: KeyState::Down,
            text: None,
            modifiers,
            timestamp: Instant::now(),
            is_repeat: repeat,
        }
    }

    #[test]
    fn primary_modifier_is_exact_and_repeat_is_opt_in() {
        let shortcut = Shortcut::physical(PhysicalKey::KeyS).modifiers(ShortcutModifiers {
            primary: true,
            ..ShortcutModifiers::default()
        });
        let primary = shortcut.resolved_modifiers();
        assert!(shortcut.matches(&event(primary, false)));
        assert!(!shortcut.matches(&event(
            Modifiers {
                shift: true,
                ..primary
            },
            false
        )));
        assert!(!shortcut.matches(&event(primary, true)));
        assert!(shortcut.allow_repeat(true).matches(&event(primary, true)));
    }
}
