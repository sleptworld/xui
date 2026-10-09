//! The winit side of the `xui-shell` window contract.

use std::ops::Deref;
use std::sync::Mutex;
use std::time::Instant;

use winit::dpi::{LogicalPosition, LogicalSize};
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::{Window, WindowAttributes};
use xui_interface::{CursorIcon, HostCapabilities, Rect};
use xui_shell::{PhysicalSize, PlatformWindow, SurfaceTarget, WindowOptions, WindowSize};

/// A winit window as a [`PlatformWindow`].
///
/// A newtype rather than an impl on `winit::window::Window` directly, which the
/// orphan rule forbids. Derefs to the winit window for everything else.
#[derive(Debug)]
pub struct WinitWindow {
    window: Window,
    /// When the shell last asked for a timed frame. A window cannot set the
    /// event loop's control flow itself, so the runner reads this when the loop
    /// is about to wait. See [`Self::take_due_redraw`].
    redraw_at: Mutex<Option<Instant>>,
}

impl WinitWindow {
    pub fn new(window: Window) -> Self {
        Self {
            window,
            redraw_at: Mutex::new(None),
        }
    }

    pub fn winit(&self) -> &Window {
        &self.window
    }

    /// Where the event loop should wait until: the pending timed frame, or
    /// `None` to wait for the next event.
    ///
    /// A deadline that has already passed is taken, and turned into an ordinary
    /// redraw request, so the loop never waits on a time that is behind it.
    pub(crate) fn take_due_redraw(&self, now: Instant) -> Option<Instant> {
        let mut redraw_at = self.redraw_at.lock().unwrap();
        match *redraw_at {
            Some(at) if at <= now => {
                *redraw_at = None;
                self.window.request_redraw();
                None
            }
            pending => pending,
        }
    }
}

impl Deref for WinitWindow {
    type Target = Window;

    fn deref(&self) -> &Window {
        &self.window
    }
}

impl HasWindowHandle for WinitWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

impl HasDisplayHandle for WinitWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.display_handle()
    }
}

impl SurfaceTarget for WinitWindow {
    fn surface_size(&self) -> PhysicalSize {
        let size = self.window.inner_size();
        PhysicalSize::new(size.width, size.height)
    }

    fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }
}

impl PlatformWindow for WinitWindow {
    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn request_redraw_at(&self, at: Instant) {
        *self.redraw_at.lock().unwrap() = Some(at);
    }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities::TIMED_REDRAW
    }

    fn set_visible(&self, visible: bool) {
        self.window.set_visible(visible);
    }

    fn focus(&self) {
        self.window.focus_window();
    }

    fn is_minimized(&self) -> Option<bool> {
        self.window.is_minimized()
    }

    fn set_cursor(&self, cursor: CursorIcon) {
        match to_winit_cursor(cursor) {
            Some(icon) => {
                self.window.set_cursor_visible(true);
                self.window.set_cursor(icon);
            }
            None => self.window.set_cursor_visible(false),
        }
    }

    fn set_ime_allowed(&self, allowed: bool) {
        self.window.set_ime_allowed(allowed);
    }

    fn set_ime_cursor_area(&self, area: Rect) {
        self.window.set_ime_cursor_area(
            LogicalPosition::new(area.x as f64, area.y as f64),
            LogicalSize::new(area.width as f64, area.height as f64),
        );
    }
}

/// Maps [`WindowOptions`] onto winit's window attributes.
pub fn window_attributes(options: &WindowOptions) -> WindowAttributes {
    let mut attributes = Window::default_attributes()
        .with_title(options.title.clone())
        .with_visible(options.visible)
        .with_resizable(options.resizable)
        .with_decorations(options.decorations)
        .with_transparent(options.transparent)
        .with_maximized(options.maximized);
    if let Some(size) = options.inner_size {
        attributes = attributes.with_inner_size(winit_size(size));
    }
    if let Some(size) = options.min_inner_size {
        attributes = attributes.with_min_inner_size(winit_size(size));
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attributes = attributes
            .with_title_hidden(options.macos.title_hidden)
            .with_titlebar_transparent(options.macos.titlebar_transparent)
            .with_fullsize_content_view(options.macos.fullsize_content_view);
    }
    attributes
}

fn winit_size(size: WindowSize) -> winit::dpi::Size {
    match size {
        WindowSize::Physical(size) => winit::dpi::PhysicalSize::new(size.width, size.height).into(),
        WindowSize::Logical(size) => LogicalSize::new(size.width as f64, size.height as f64).into(),
    }
}

/// The one place that knows about winit's cursor vocabulary.
///
/// `None` means "hide the cursor", which winit models as a visibility flag
/// rather than an icon.
fn to_winit_cursor(cursor: CursorIcon) -> Option<winit::window::CursorIcon> {
    use winit::window::CursorIcon as Winit;
    Some(match cursor {
        CursorIcon::Default => Winit::Default,
        CursorIcon::Pointer => Winit::Pointer,
        CursorIcon::Text => Winit::Text,
        CursorIcon::Crosshair => Winit::Crosshair,
        CursorIcon::Move => Winit::Move,
        CursorIcon::Grab => Winit::Grab,
        CursorIcon::Grabbing => Winit::Grabbing,
        CursorIcon::NotAllowed => Winit::NotAllowed,
        CursorIcon::Wait => Winit::Wait,
        CursorIcon::Progress => Winit::Progress,
        CursorIcon::Help => Winit::Help,
        CursorIcon::ColumnResize => Winit::ColResize,
        CursorIcon::RowResize => Winit::RowResize,
        CursorIcon::EastWestResize => Winit::EwResize,
        CursorIcon::NorthSouthResize => Winit::NsResize,
        CursorIcon::None => return None,
    })
}
