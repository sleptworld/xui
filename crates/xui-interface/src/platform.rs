use std::sync::Arc;

use crate::Rect;
use crate::style::CursorIcon;

mod command;
mod environment;
mod menu;
mod surface;

pub use command::{
    ClipboardContent, ClipboardTarget, FileDialog, FileFilter, MessageBox, MessageLevel,
    PlatformCommand, PlatformError, PlatformReply, PlatformRequest, RequestId, ResizeEdge,
    WindowCommand,
};
pub use environment::{Appearance, ColorScheme, HostCapabilities, WindowGeometry, WindowState};
pub use menu::{Menu, MenuBar, MenuEntry, MenuItem, MenuRole, commands};
pub use surface::{
    DragEvent, DragPayload, DropAccepts, DropEffect, DropTarget, PublishedSurface, RegionRect,
    TextDocument, WindowRegion,
};

/// Platform-facing state desired by the UI runtime.
///
/// Backends should compare this value with the last state they applied and
/// translate only the differences to their native windowing API.
///
/// Everything here is state the runtime wants to *stay* true. One-shot actions
/// are [`PlatformCommand`]s instead.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlatformOutput {
    pub text_input: Option<TextInputSession>,
    /// The pointer shape the window should show right now, resolved from the
    /// node under the pointer — or, while a pointer is captured, from the node
    /// holding the capture.
    pub cursor: CursorIcon,
    /// The window title. `None` leaves whatever the window was created with.
    pub title: Option<String>,
    /// The application's menu bar. `None` leaves the host's default. Shared,
    /// because this struct is cloned and compared on every sync.
    pub menu_bar: Option<Arc<MenuBar>>,
    /// What a native callback may need to know without asking the runtime.
    pub surface: PublishedSurface,
}

/// The active native text-input/IME session.
#[derive(Debug, Clone, PartialEq)]
pub struct TextInputSession {
    /// Candidate-window anchor in logical coordinates relative to the window.
    pub cursor_area: Rect,
    pub purpose: TextInputPurpose,
    pub multiline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextInputPurpose {
    #[default]
    Normal,
    Password,
    Email,
    Number,
    Phone,
    Url,
}
