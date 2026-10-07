//! What the runtime asks of the host that is not per-frame state: a one-shot
//! [`PlatformCommand`], or a [`PlatformRequest`] that gets a [`PlatformReply`].
//!
//! State the runtime wants *kept* true -- the cursor, the window title -- is
//! [`PlatformOutput`](super::PlatformOutput) instead, which a host diffs. A
//! command is an action, and replaying one every frame would be wrong.

use std::path::PathBuf;
use std::sync::Arc;

use crate::{Point, Size};

use super::menu::Menu;

/// Correlates a [`PlatformRequest`] with its [`PlatformReply`]. Allocated by
/// the runtime; a host only echoes it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlatformCommand {
    Window(WindowCommand),
    WriteClipboard {
        target: ClipboardTarget,
        content: ClipboardContent,
    },
    OpenUrl(String),
    /// Show a file in the platform's file manager.
    RevealPath(PathBuf),
    /// A context menu at `at`, in logical window coordinates. The chosen item
    /// comes back as a command event; dismissing the menu sends nothing.
    ShowContextMenu {
        menu: Arc<Menu>,
        at: Point,
    },
    /// Something that answers. The host replies inline or, if it cannot yet,
    /// later through the event route.
    Request(RequestId, PlatformRequest),
    /// The caller no longer wants the answer, because its component unmounted.
    /// A host dismisses what it can; a reply that still arrives is dropped.
    Cancel(RequestId),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WindowCommand {
    SetFullscreen(bool),
    SetMaximized(bool),
    Minimize,
    /// The size of the content area, in logical units.
    SetInnerSize(Size<f32>),
    SetSizeLimits {
        min: Option<Size<f32>>,
        max: Option<Size<f32>>,
    },
    SetAlwaysOnTop(bool),
    /// Start moving the window with the pointer, as a title bar does. Only
    /// meaningful while a pointer button is down; the host holds on to the
    /// native event that began the press.
    BeginMove,
    BeginResize(ResizeEdge),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClipboardTarget {
    #[default]
    General,
    /// The X11 / Wayland primary selection, pasted with a middle click.
    Selection,
}

/// Open-ended: images, HTML and file lists are added without touching the host
/// trait.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ClipboardContent {
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlatformRequest {
    ReadClipboard(ClipboardTarget),
    OpenFileDialog(FileDialog),
    SaveFileDialog(FileDialog),
    MessageBox(MessageBox),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlatformReply {
    /// The host cannot do what was asked. Always delivered for a request the
    /// host does not support, so a waiting caller is never left hanging.
    Unsupported,
    Failed(PlatformError),
    /// `None`: the clipboard held nothing of a kind the host could convert.
    Clipboard(Option<ClipboardContent>),
    /// `None`: the dialog was cancelled.
    Paths(Option<Vec<PathBuf>>),
    /// An index into [`MessageBox::buttons`]; `None` if it was dismissed.
    Button(Option<usize>),
}

/// Why a command or request did not happen. Carried rather than swallowed:
/// a command is fire-and-forget for its sender, but the shell logs every failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformError {
    pub message: String,
}

impl PlatformError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PlatformError {}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileDialog {
    pub title: Option<String>,
    pub directory: Option<PathBuf>,
    pub filters: Vec<FileFilter>,
    pub multiple: bool,
    pub pick_folders: bool,
    /// Save dialogs only.
    pub default_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileFilter {
    pub name: String,
    /// Without the dot.
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageBox {
    pub level: MessageLevel,
    pub title: String,
    pub message: String,
    pub buttons: Vec<String>,
    pub default_button: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageLevel {
    #[default]
    Info,
    Warning,
    Error,
}
