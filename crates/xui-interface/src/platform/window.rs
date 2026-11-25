//! The windows an application asks for: how to describe one, and the id the
//! runtime and the host both name it by.

use crate::Size;

slotmap::new_key_type! {
    /// One of the application's windows. Allocated by the runtime when it
    /// asks for a window; a host keeps its native window under it and names
    /// it in what it sends back.
    pub struct WindowId;
}

/// A size in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

impl PhysicalSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// The size in logical units at `scale_factor`.
    pub fn to_logical(self, scale_factor: f64) -> Size<f32> {
        let scale = (scale_factor as f32).max(f32::EPSILON);
        Size::new(self.width as f32 / scale, self.height as f32 / scale)
    }
}

/// A window size in either unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowSize {
    Physical(PhysicalSize),
    Logical(Size<f32>),
}

impl From<PhysicalSize> for WindowSize {
    fn from(size: PhysicalSize) -> Self {
        Self::Physical(size)
    }
}

impl From<Size<f32>> for WindowSize {
    fn from(size: Size<f32>) -> Self {
        Self::Logical(size)
    }
}

/// The window an application asks for.
///
/// Deliberately a small, common subset: each host maps it onto its own window
/// creation, so anything here has to mean something on every platform (or sit
/// in a per-platform group like [`MacOsWindowOptions`] that others ignore).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowOptions {
    pub title: String,
    /// `None` leaves the size to the platform.
    pub inner_size: Option<WindowSize>,
    pub min_inner_size: Option<WindowSize>,
    /// Whether the window should end up visible. Hosts create it hidden and
    /// show it once the first frame is on it either way.
    pub visible: bool,
    pub resizable: bool,
    pub decorations: bool,
    pub transparent: bool,
    pub maximized: bool,
    pub macos: MacOsWindowOptions,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "XUI".to_owned(),
            inner_size: Some(WindowSize::Physical(PhysicalSize::new(800, 600))),
            min_inner_size: None,
            visible: true,
            resizable: true,
            decorations: true,
            transparent: false,
            maximized: false,
            macos: MacOsWindowOptions::default(),
        }
    }
}

impl WindowOptions {
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_inner_size(mut self, size: impl Into<WindowSize>) -> Self {
        self.inner_size = Some(size.into());
        self
    }

    pub fn with_min_inner_size(mut self, size: impl Into<WindowSize>) -> Self {
        self.min_inner_size = Some(size.into());
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn with_resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    pub fn with_decorations(mut self, decorations: bool) -> Self {
        self.decorations = decorations;
        self
    }

    pub fn with_transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }

    pub fn with_maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    pub fn with_macos(mut self, macos: MacOsWindowOptions) -> Self {
        self.macos = macos;
        self
    }
}

/// macOS-only window chrome. Ignored on every other platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MacOsWindowOptions {
    pub title_hidden: bool,
    pub titlebar_transparent: bool,
    /// Extends the content view under the titlebar.
    pub fullsize_content_view: bool,
    /// Where the close, minimize and zoom buttons go: `(left, top)`, in
    /// points, from the window's top-left corner to the close button's. The
    /// title bar grows to keep the buttons inside it. `None`, or a negative
    /// value, leaves them where AppKit puts them.
    pub btn_margin: Option<(i32, i32)>,
}
