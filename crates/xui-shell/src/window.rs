use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use xui_interface::{CursorIcon, Rect, Size};

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

/// Something a render backend can present into.
///
/// This is all a backend learns about the window: the raw handles its
/// swapchain is created from, and the size and scale factor to create it at.
///
/// `Send + Sync` because wgpu requires it of a surface's window. A host whose
/// native window is main-thread-only can still meet it by holding only the
/// handles and forwarding everything else to the main thread, as winit does.
pub trait SurfaceTarget: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static {
    /// The drawable area, in physical pixels.
    fn surface_size(&self) -> PhysicalSize;
    fn scale_factor(&self) -> f64;
}

/// The window operations [`crate::Shell`] needs from a host.
///
/// Every method is a request the platform may honour late or not at all;
/// none of them report failure.
pub trait PlatformWindow: SurfaceTarget {
    /// Asks for a [`crate::ShellEvent::RedrawRequested`] to be delivered.
    fn request_redraw(&self);
    fn set_visible(&self, visible: bool);
    /// Makes the window key / foreground.
    fn focus(&self);
    /// `None` when the platform cannot tell.
    fn is_minimized(&self) -> Option<bool>;
    /// [`CursorIcon::None`] hides the cursor.
    fn set_cursor(&self, cursor: CursorIcon);
    fn set_ime_allowed(&self, allowed: bool);
    /// Anchors the IME candidate window, in logical window coordinates.
    fn set_ime_cursor_area(&self, area: Rect);
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
}
