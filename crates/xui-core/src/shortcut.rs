use xui_interface::{CommandId, RawKeyboard, Shortcut, ShortcutBinding};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShortcutRegistrationId(u64);

#[derive(Debug, Default)]
pub struct ShortcutManager {
    next_id: u64,
    bindings: Vec<(ShortcutRegistrationId, ShortcutBinding)>,
}

impl ShortcutManager {
    pub fn register(&mut self, shortcut: Shortcut, command: CommandId) -> ShortcutRegistrationId {
        self.bindings
            .retain(|(_, binding)| binding.shortcut != shortcut);
        let id = ShortcutRegistrationId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.bindings
            .push((id, ShortcutBinding { shortcut, command }));
        id
    }

    pub fn unregister(&mut self, id: ShortcutRegistrationId) -> bool {
        let old_len = self.bindings.len();
        self.bindings.retain(|(registered, _)| *registered != id);
        old_len != self.bindings.len()
    }

    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    pub fn resolve(&self, event: &RawKeyboard) -> Option<ShortcutBinding> {
        self.bindings
            .iter()
            .rev()
            .map(|(_, binding)| *binding)
            .find(|binding| binding.shortcut.matches(event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use xui_interface::{KeyState, Modifiers, PhysicalKey, ShortcutModifiers};

    fn event(key: PhysicalKey) -> RawKeyboard {
        RawKeyboard {
            physical_key: key,
            named_key: None,
            state: KeyState::Down,
            text: None,
            modifiers: Modifiers {
                ctrl: !cfg!(target_os = "macos"),
                meta: cfg!(target_os = "macos"),
                ..Modifiers::default()
            },
            timestamp: Instant::now(),
            is_repeat: false,
        }
    }

    #[test]
    fn later_registration_replaces_same_shortcut() {
        let shortcut = Shortcut::physical(PhysicalKey::KeyS).modifiers(ShortcutModifiers {
            primary: true,
            ..ShortcutModifiers::default()
        });
        let mut manager = ShortcutManager::default();
        let old = manager.register(shortcut, CommandId("old"));
        let new = manager.register(shortcut, CommandId("new"));
        assert!(!manager.unregister(old));
        assert_eq!(
            manager.resolve(&event(PhysicalKey::KeyS)).unwrap().command,
            CommandId("new")
        );
        assert!(manager.unregister(new));
    }

    #[test]
    fn managers_are_isolated_per_runtime_owner() {
        let shortcut = Shortcut::physical(PhysicalKey::KeyS).modifiers(ShortcutModifiers {
            primary: true,
            ..ShortcutModifiers::default()
        });
        let mut first = ShortcutManager::default();
        let second = ShortcutManager::default();
        first.register(shortcut, CommandId("save"));
        assert!(first.resolve(&event(PhysicalKey::KeyS)).is_some());
        assert!(second.resolve(&event(PhysicalKey::KeyS)).is_none());
    }
}
