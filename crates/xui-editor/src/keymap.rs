//! Commands, and the keys bound to them.

use xui_interface::events::{KeyState, RawKeyboard};
use xui_interface::{NamedKey, PhysicalKey};

/// Where a move goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
    PageUp,
    PageDown,
}

/// Everything an editor does that a key, a menu or code can ask for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EditorCommand {
    /// Moves every caret; with `extend`, moves heads and keeps anchors.
    Move {
        motion: Motion,
        extend: bool,
    },
    /// Deletes the selection, or the grapheme before each caret.
    Backspace,
    /// Deletes the selection, or the grapheme after each caret.
    Delete,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToLineStart,
    /// A line break, indented like the line it breaks.
    Newline,
    /// Indents: the tab text at each caret, or every selected line.
    Indent,
    Outdent,
    Insert(String),
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    /// Back to the primary selection alone.
    CollapseSelections,
}

/// The key of a [`KeyChord`]: a named key, or a key by its position for
/// letters and digits, so bindings survive keyboard layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChordKey {
    Named(NamedKey),
    Physical(PhysicalKey),
}

/// A key with exactly these modifiers held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: ChordKey,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl KeyChord {
    pub fn new(key: ChordKey) -> Self {
        Self {
            key,
            shift: false,
            ctrl: false,
            alt: false,
            meta: false,
        }
    }

    pub fn named(key: NamedKey) -> Self {
        Self::new(ChordKey::Named(key))
    }

    pub fn physical(key: PhysicalKey) -> Self {
        Self::new(ChordKey::Physical(key))
    }

    pub fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub fn meta(mut self) -> Self {
        self.meta = true;
        self
    }

    /// The platform's command modifier: Cmd on macOS, Ctrl elsewhere.
    pub fn primary(self) -> Self {
        if cfg!(target_os = "macos") {
            self.meta()
        } else {
            self.ctrl()
        }
    }

    /// The modifier that moves by word: Option on macOS, Ctrl elsewhere.
    pub fn word(self) -> Self {
        if cfg!(target_os = "macos") {
            self.alt()
        } else {
            self.ctrl()
        }
    }

    fn matches(&self, key: &RawKeyboard) -> bool {
        let modifiers = key.modifiers;
        let key_matches = match self.key {
            ChordKey::Named(named) => key.named_key == Some(named),
            ChordKey::Physical(physical) => key.physical_key == physical,
        };
        key_matches
            && self.shift == modifiers.shift
            && self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.meta == modifiers.meta
    }
}

/// Which keys do what. Later bindings win over earlier ones for the same
/// chord, so an application rebinds by binding again.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    bindings: Vec<(KeyChord, EditorCommand)>,
}

impl Keymap {
    /// No bindings at all.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The bindings of the platform's own text views.
    pub fn platform_default() -> Self {
        let mut keymap = Self::empty();
        let mac = cfg!(target_os = "macos");
        let moves: &[(KeyChord, Motion)] = &[
            (KeyChord::named(NamedKey::ArrowLeft), Motion::Left),
            (KeyChord::named(NamedKey::ArrowRight), Motion::Right),
            (KeyChord::named(NamedKey::ArrowUp), Motion::Up),
            (KeyChord::named(NamedKey::ArrowDown), Motion::Down),
            (
                KeyChord::named(NamedKey::ArrowLeft).word(),
                Motion::WordLeft,
            ),
            (
                KeyChord::named(NamedKey::ArrowRight).word(),
                Motion::WordRight,
            ),
            (KeyChord::named(NamedKey::Home), Motion::LineStart),
            (KeyChord::named(NamedKey::End), Motion::LineEnd),
            (KeyChord::named(NamedKey::PageUp), Motion::PageUp),
            (KeyChord::named(NamedKey::PageDown), Motion::PageDown),
        ];
        for (chord, motion) in moves {
            keymap.bind_move(*chord, *motion);
        }
        if mac {
            keymap.bind_move(
                KeyChord::named(NamedKey::ArrowLeft).meta(),
                Motion::LineStart,
            );
            keymap.bind_move(
                KeyChord::named(NamedKey::ArrowRight).meta(),
                Motion::LineEnd,
            );
            keymap.bind_move(
                KeyChord::named(NamedKey::ArrowUp).meta(),
                Motion::DocumentStart,
            );
            keymap.bind_move(
                KeyChord::named(NamedKey::ArrowDown).meta(),
                Motion::DocumentEnd,
            );
        } else {
            keymap.bind_move(
                KeyChord::named(NamedKey::Home).ctrl(),
                Motion::DocumentStart,
            );
            keymap.bind_move(KeyChord::named(NamedKey::End).ctrl(), Motion::DocumentEnd);
        }

        use EditorCommand::*;
        let edits = [
            (KeyChord::named(NamedKey::Backspace), Backspace),
            (KeyChord::named(NamedKey::Backspace).shift(), Backspace),
            (
                KeyChord::named(NamedKey::Backspace).word(),
                DeleteWordBackward,
            ),
            (KeyChord::named(NamedKey::Delete), Delete),
            (KeyChord::named(NamedKey::Delete).word(), DeleteWordForward),
            (KeyChord::named(NamedKey::Enter), Newline),
            (KeyChord::named(NamedKey::Enter).shift(), Newline),
            (KeyChord::named(NamedKey::Tab), Indent),
            (KeyChord::named(NamedKey::Tab).shift(), Outdent),
            (KeyChord::named(NamedKey::Escape), CollapseSelections),
            (KeyChord::physical(PhysicalKey::KeyA).primary(), SelectAll),
            (KeyChord::physical(PhysicalKey::KeyC).primary(), Copy),
            (KeyChord::physical(PhysicalKey::KeyX).primary(), Cut),
            (KeyChord::physical(PhysicalKey::KeyV).primary(), Paste),
            (KeyChord::physical(PhysicalKey::KeyZ).primary(), Undo),
            (
                KeyChord::physical(PhysicalKey::KeyZ).primary().shift(),
                Redo,
            ),
        ];
        for (chord, command) in edits {
            keymap.bind(chord, command);
        }
        if mac {
            keymap.bind(
                KeyChord::named(NamedKey::Backspace).meta(),
                DeleteToLineStart,
            );
        } else {
            keymap.bind(KeyChord::physical(PhysicalKey::KeyY).ctrl(), Redo);
        }
        keymap
    }

    pub fn bind(&mut self, chord: KeyChord, command: EditorCommand) {
        self.unbind(chord);
        self.bindings.push((chord, command));
    }

    /// Binds `chord` to move, and `chord` with Shift to extend.
    pub fn bind_move(&mut self, chord: KeyChord, motion: Motion) {
        self.bind(
            chord,
            EditorCommand::Move {
                motion,
                extend: false,
            },
        );
        self.bind(
            chord.shift(),
            EditorCommand::Move {
                motion,
                extend: true,
            },
        );
    }

    pub fn unbind(&mut self, chord: KeyChord) {
        self.bindings.retain(|(bound, _)| *bound != chord);
    }

    /// The command a key press asks for. Releases ask for nothing.
    pub fn resolve(&self, key: &RawKeyboard) -> Option<EditorCommand> {
        if key.state != KeyState::Down {
            return None;
        }
        self.bindings
            .iter()
            .rev()
            .find(|(chord, _)| chord.matches(key))
            .map(|(_, command)| command.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use xui_interface::Modifiers;

    use super::*;

    fn press(named: Option<NamedKey>, physical: PhysicalKey, modifiers: Modifiers) -> RawKeyboard {
        RawKeyboard {
            physical_key: physical,
            named_key: named,
            state: KeyState::Down,
            text: None,
            modifiers,
            timestamp: Instant::now(),
            is_repeat: false,
        }
    }

    #[test]
    fn shift_extends_and_a_rebinding_wins() {
        let mut keymap = Keymap::platform_default();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let left = press(Some(NamedKey::ArrowLeft), PhysicalKey::ArrowLeft, shift);
        assert_eq!(
            keymap.resolve(&left),
            Some(EditorCommand::Move {
                motion: Motion::Left,
                extend: true
            })
        );
        keymap.bind(
            KeyChord::named(NamedKey::ArrowLeft).shift(),
            EditorCommand::SelectAll,
        );
        assert_eq!(keymap.resolve(&left), Some(EditorCommand::SelectAll));
        let plain = press(
            Some(NamedKey::ArrowLeft),
            PhysicalKey::ArrowLeft,
            Modifiers::default(),
        );
        assert!(matches!(
            keymap.resolve(&plain),
            Some(EditorCommand::Move { extend: false, .. })
        ));
    }
}
