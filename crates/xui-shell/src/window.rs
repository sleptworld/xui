use std::time::Instant;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use xui_interface::{
    Appearance, CursorIcon, HostCapabilities, MenuBar, PlatformCommand, PlatformError,
    PlatformReply, Rect, Size, WindowGeometry,
};

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

    /// Whether the user is dragging the window's edge right now.
    ///
    /// A presenter whose frames reach the screen outside the window system's
    /// own transaction (Metal's asynchronous `presentDrawable`) uses this to
    /// switch to a synchronized present while it is true, so the frame and the
    /// window's new geometry land together. Hosts that have no such mismatch
    /// keep the default.
    fn in_live_resize(&self) -> bool {
        false
    }
}

/// The window operations [`crate::Shell`] needs from a host.
///
/// Every method is a request the platform may honour late or not at all;
/// none of them report failure, except [`Self::perform`], which says what it did.
///
/// The methods after [`Self::set_ime_cursor_area`] all have defaults, so a host
/// adopts them one capability at a time: it overrides what it can do, reports it
/// through [`Self::capabilities`], and leaves the rest as the "not supported"
/// the defaults amount to.
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

    /// What this host can do. Read once, before the first frame, and handed to
    /// the application so it can hide or degrade UI the host cannot back.
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities::NONE
    }

    /// The system's appearance now. Later changes arrive as
    /// [`crate::ShellEvent::AppearanceChanged`].
    fn appearance(&self) -> Appearance {
        Appearance::default()
    }

    /// Asks for a [`crate::ShellEvent::RedrawRequested`] (or a
    /// [`crate::ShellEvent::FrameTick`]) at or soon after `at`, replacing any
    /// time asked for before.
    ///
    /// What a blinking caret waits on: the shell calls this when nothing needs a
    /// frame now but something has asked to change at a later instant, so the
    /// loop can sleep until then instead of polling. A frame that arrives for a
    /// replaced request is harmless; one that never arrives leaves the
    /// deadline waiting for the next event. A host that implements it reports
    /// [`HostCapabilities::TIMED_REDRAW`], which is what UI checks before
    /// relying on it.
    fn request_redraw_at(&self, _at: Instant) {}

    /// Applied when the title the runtime wants changes, never repeated.
    fn set_title(&self, _title: &str) {}

    /// Applied when the menu bar the runtime wants changes. `None` returns to
    /// the host's default.
    fn set_menu_bar(&self, _menu_bar: Option<&MenuBar>) {}

    /// Where the window is, for saving and restoring it.
    fn geometry(&self) -> Option<WindowGeometry> {
        None
    }

    /// Carries out something the application asked for that is not per-frame
    /// state: a clipboard write, a dialog, a window command.
    ///
    /// The one entry point for all of them, so adding a command never changes
    /// this trait: a host that does not know a variant answers
    /// [`Performed::Unsupported`] and keeps building.
    ///
    /// A host that shows something that outlives this call (a sheet, a portal
    /// dialog) must not block here. Blocking runs the platform's nested event
    /// loop, which delivers events while the shell is busy; answer
    /// [`Performed::Pending`] instead and reply later with
    /// [`crate::ShellEvent::Reply`].
    fn perform(&self, _command: PlatformCommand) -> Performed {
        Performed::Unsupported
    }
}

/// What a host did with a [`PlatformCommand`].
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub enum Performed {
    /// A command was carried out.
    Done,
    /// The host does not implement it. For a request the shell turns this into
    /// [`PlatformReply::Unsupported`], so whoever is waiting is never left
    /// hanging.
    Unsupported,
    /// A request answered on the spot.
    Reply(PlatformReply),
    /// A request the host will answer later, with
    /// [`crate::ShellEvent::Reply`]: a dialog shown as a sheet, a Wayland
    /// clipboard read.
    Pending,
    Failed(PlatformError),
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
    pub btn_margin: Option<(i32, i32)>,
}
