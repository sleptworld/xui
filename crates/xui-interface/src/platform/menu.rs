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

/// The menu bar that applies while one window is active, beside the
/// application's own [`MenuBar`], which is the default and what a platform
/// shows with no window at all.
///
/// Where it is drawn is the host's business: macOS shows the active window's
/// in the screen's menu bar, Windows attaches each window's to that window.
/// `Hidden` cannot empty a macOS menu bar, so there it is `Inherit`; on
/// Windows only the main window shows an inherited menu, so a secondary window
/// does not carry the whole application menu unless it asks to.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum WindowMenu {
    /// The application's menu bar.
    #[default]
    Inherit,
    /// No menu for this window.
    Hidden,
    /// A menu bar of the window's own.
    Own(MenuBar),
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
