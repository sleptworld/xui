use std::time::Instant;

use std::sync::Arc;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use xui_interface::{
    Appearance, CursorIcon, HostCapabilities, MenuBar, PhysicalSize, PlatformCommand,
    PlatformError, PlatformReply, Rect, WindowGeometry, WindowId, WindowMenu, WindowOptions,
};

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

    /// Applied when the application's menu bar changes. `None` returns to the
    /// host's default. Called on the main window only: the menu bar is the
    /// app's, and a host with one menu bar keeps it there.
    fn set_menu_bar(&self, _menu_bar: Option<&MenuBar>) {}

    /// Applied when the menu that applies while this window is active
    /// changes. See [`WindowMenu`] for what each platform makes of it.
    fn set_menu(&self, _menu: &WindowMenu) {}

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

/// What a host that can open windows beside its first gives
/// [`crate::Shell::with_opener`].
///
/// The shell calls it when the application mounts a `window(..) { .. }`
/// block, and again when it unmounts one; everything in between -- events,
/// frames, state -- goes through the [`PlatformWindow`] it returned, as the
/// main window's do. The window is drawn by the one backend that draws the
/// main window, which the shell hands its surface ([`SurfaceBackend`]).
pub trait WindowOpener<W> {
    /// Makes a native window for `window` from `options`, **hidden**. The
    /// shell shows it once its first frame is through the backend, if
    /// `options.visible`. Its events go to
    /// [`crate::Shell::dispatch_window_event`] under `window`.
    ///
    /// `published` is what the runtime will publish about the window's
    /// surface, for the native callbacks that must answer without calling
    /// into the shell (is this point a title bar?).
    fn open(
        &mut self,
        window: WindowId,
        options: &WindowOptions,
        published: &crate::PublishedHandle,
    ) -> Result<Arc<W>, PlatformError>;

    /// The application closed `window`: destroy the native window. The
    /// backend has let go of its surface already.
    fn close(&mut self, window: WindowId, native: &Arc<W>);
}

/// What a render backend does for a host with more than one window: it was
/// made on the main window's surface, and draws any other window on a
/// surface the shell hands it, with the device and caches it has.
///
/// A backend that draws one surface only keeps the defaults; the shell then
/// opens no window beside the main one.
pub trait SurfaceBackend {
    /// Whether the backend draws windows beside the one it was made on.
    fn supports_surfaces(&self) -> bool {
        false
    }

    /// The surface the backend was made on is `window`'s. Told once, before
    /// the first frame.
    fn bind_primary(&mut self, _window: WindowId) {}

    /// Draws `window`'s frames on `surface` from now on, until
    /// `RenderBackend::remove_surface`.
    fn attach_surface(
        &mut self,
        _window: WindowId,
        _surface: Arc<dyn SurfaceTarget>,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::new("this backend draws one window only"))
    }
}

/// Draws as many windows as it is given: it draws none of them.
impl SurfaceBackend for xui_core::render::MockRenderBackend {
    fn supports_surfaces(&self) -> bool {
        true
    }

    fn attach_surface(
        &mut self,
        _window: WindowId,
        _surface: Arc<dyn SurfaceTarget>,
    ) -> Result<(), PlatformError> {
        Ok(())
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
