use xui_interface::{KeyState, NamedKey, PhysicalKey, RawKeyboard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCommand {
    SelectAll,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    MoveLeft { extend: bool },
    MoveRight { extend: bool },
    MoveHome { extend: bool },
    MoveEnd { extend: bool },
    DeleteBackward,
    DeleteForward,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TextKeymap;

impl TextKeymap {
    pub const fn platform_default() -> Self {
        Self
    }

    pub fn resolve(&self, event: &RawKeyboard) -> Option<TextCommand> {
        if event.state != KeyState::Down {
            return None;
        }
        let primary = if cfg!(target_os = "macos") {
            event.modifiers.meta
        } else {
            event.modifiers.ctrl
        };
        if primary && !event.modifiers.alt {
            let command = match event.physical_key {
                PhysicalKey::KeyA => TextCommand::SelectAll,
                PhysicalKey::KeyZ if event.modifiers.shift => TextCommand::Redo,
                PhysicalKey::KeyZ => TextCommand::Undo,
                PhysicalKey::KeyY if !cfg!(target_os = "macos") => TextCommand::Redo,
                PhysicalKey::KeyC => TextCommand::Copy,
                PhysicalKey::KeyX => TextCommand::Cut,
                PhysicalKey::KeyV => TextCommand::Paste,
                _ => return None,
            };
            return (!event.is_repeat).then_some(command);
        }

        if event.modifiers.ctrl || event.modifiers.meta || event.modifiers.alt {
            return None;
        }
        let extend = event.modifiers.shift;
        let named_command = match event.named_key {
            Some(NamedKey::ArrowLeft) => Some(TextCommand::MoveLeft { extend }),
            Some(NamedKey::ArrowRight) => Some(TextCommand::MoveRight { extend }),
            Some(NamedKey::Home) => Some(TextCommand::MoveHome { extend }),
            Some(NamedKey::End) => Some(TextCommand::MoveEnd { extend }),
            Some(NamedKey::Backspace) => Some(TextCommand::DeleteBackward),
            Some(NamedKey::Delete) => Some(TextCommand::DeleteForward),
            _ => None,
        };
        named_command.or(match event.physical_key {
            PhysicalKey::ArrowLeft => Some(TextCommand::MoveLeft { extend }),
            PhysicalKey::ArrowRight => Some(TextCommand::MoveRight { extend }),
            PhysicalKey::Home => Some(TextCommand::MoveHome { extend }),
            PhysicalKey::End => Some(TextCommand::MoveEnd { extend }),
            PhysicalKey::Backspace | PhysicalKey::NumpadBackspace => {
                Some(TextCommand::DeleteBackward)
            }
            PhysicalKey::Delete => Some(TextCommand::DeleteForward),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use xui_interface::{KeyState, Modifiers};

    fn key(physical_key: PhysicalKey, modifiers: Modifiers, repeat: bool) -> RawKeyboard {
        RawKeyboard {
            physical_key,
            named_key: None,
            state: KeyState::Down,
            text: None,
            modifiers,
            timestamp: Instant::now(),
            is_repeat: repeat,
        }
    }

    #[test]
    fn primary_edit_commands_do_not_repeat() {
        let modifiers = if cfg!(target_os = "macos") {
            Modifiers {
                meta: true,
                ..Modifiers::default()
            }
        } else {
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            }
        };
        let map = TextKeymap::platform_default();
        assert_eq!(
            map.resolve(&key(PhysicalKey::KeyA, modifiers, false)),
            Some(TextCommand::SelectAll)
        );
        assert_eq!(map.resolve(&key(PhysicalKey::KeyA, modifiers, true)), None);
    }

    #[test]
    fn editing_keys_fall_back_to_physical_key() {
        let map = TextKeymap::platform_default();
        assert_eq!(
            map.resolve(&key(PhysicalKey::Backspace, Modifiers::default(), false)),
            Some(TextCommand::DeleteBackward)
        );
        assert_eq!(
            map.resolve(&key(PhysicalKey::Delete, Modifiers::default(), false)),
            Some(TextCommand::DeleteForward)
        );
        assert_eq!(
            map.resolve(&key(PhysicalKey::ArrowLeft, Modifiers::default(), false)),
            Some(TextCommand::MoveLeft { extend: false })
        );

        let mut logical_delete = key(PhysicalKey::Backspace, Modifiers::default(), false);
        logical_delete.named_key = Some(NamedKey::Delete);
        assert_eq!(
            map.resolve(&logical_delete),
            Some(TextCommand::DeleteForward)
        );
    }
}
