//! Native menus. An item names a [`CommandId`], so choosing one is the same
//! path a shortcut takes: the runtime dispatches a command event.

use crate::{CommandId, Shortcut};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuBar(pub Vec<Menu>);

#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub label: String,
    pub entries: Vec<MenuEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
    Submenu(Menu),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// Raised as a command event when the item is chosen.
    pub command: CommandId,
    /// Shown next to the label. The runtime still owns the key handling.
    pub shortcut: Option<Shortcut>,
    pub enabled: bool,
    pub checked: Option<bool>,
    /// An item the platform gives special treatment (Quit, About, Hide, ...).
    pub role: Option<MenuRole>,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, command: CommandId) -> Self {
        Self {
            label: label.into(),
            command,
            shortcut: None,
            enabled: true,
            checked: None,
            role: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuRole {
    About,
    Quit,
    Hide,
    HideOthers,
    Minimize,
    Zoom,
    Fullscreen,
}

/// Well-known commands a host raises for the system's own standard actions --
/// the macOS responder chain's `copy:`, a Windows `WM_COPY` -- and that
/// standard menu items carry. Ordinary command ids, handled like any other.
pub mod commands {
    use crate::CommandId;

    pub const UNDO: CommandId = CommandId("edit.undo");
    pub const REDO: CommandId = CommandId("edit.redo");
    pub const CUT: CommandId = CommandId("edit.cut");
    pub const COPY: CommandId = CommandId("edit.copy");
    pub const PASTE: CommandId = CommandId("edit.paste");
    pub const SELECT_ALL: CommandId = CommandId("edit.select_all");
}
