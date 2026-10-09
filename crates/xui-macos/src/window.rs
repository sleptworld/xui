use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Weak;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use dispatch2::MainThreadBound;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSScreen, NSView, NSWindow, NSWindowDelegate,
    NSWindowOcclusionState, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use raw_window_handle::{
    AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
    RawWindowHandle, WindowHandle,
};
use xui_interface::{CursorIcon, HostCapabilities, MenuBar, PlatformCommand, Rect, WindowState};
use xui_shell::{
    Performed, PhysicalSize, PlatformWindow, PublishedHandle, ShellEvent, SurfaceTarget,
    WindowOptions, WindowSize,
};

use crate::host::{self, EventSink};
use crate::menu::{self, BuiltMenuBar};
use crate::perform;
use crate::view::XuiView;

/// An `NSWindow` with an `xui` content view.
///
/// Every AppKit object is held behind a `MainThreadBound`, which is what lets
/// the window be `Send + Sync` as `SurfaceTarget` requires. The shell only
/// ever calls in on the main thread; a `PlatformWindow` request made from any
/// other thread is ignored, and size queries there answer from the last value
/// read on the main thread.
pub struct MacWindow {
    /// The content view's address, for the raw window handle. `view` keeps it
    /// alive.
    ns_view: usize,
    window: MainThreadBound<Retained<NSWindow>>,
    view: MainThreadBound<Retained<XuiView>>,
    delegate: MainThreadBound<Retained<WindowDelegate>>,
    metrics: Mutex<(PhysicalSize, f64)>,
    /// True while the delegate is inside `windowDidResize:`, where the shell
    /// paints the new size before AppKit shows it. See
    /// [`SurfaceTarget::in_live_resize`].
    resizing: Arc<AtomicBool>,
    /// The menu bar the application supplied, and the target its items point
    /// at, which AppKit does not retain.
    menu_bar: MainThreadBound<RefCell<Option<BuiltMenuBar>>>,
}

impl MacWindow {
    /// Creates the window, hidden, as `Shell::new` requires.
    pub(crate) fn new(mtm: MainThreadMarker, options: &WindowOptions) -> Arc<Self> {
        let (width, height) = options
            .inner_size
            .map(|size| logical_size(mtm, size))
            .unwrap_or((800.0, 600.0));
        let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));

        let mut style = if options.decorations {
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
        } else {
            NSWindowStyleMask::Borderless
        };
        if options.resizable {
            style |= NSWindowStyleMask::Resizable;
        }
        if options.macos.fullsize_content_view {
            style |= NSWindowStyleMask::FullSizeContentView;
        }

        // SAFETY: released-when-closed is turned off right below, as it must
        // be for a window that no window controller owns.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&options.title));
        if options.macos.title_hidden {
            window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        }
        if options.macos.titlebar_transparent {
            window.setTitlebarAppearsTransparent(true);
        }
        if options.transparent {
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
        }
        if let Some(size) = options.min_inner_size {
            let (width, height) = logical_size(mtm, size);
            window.setContentMinSize(NSSize::new(width, height));
        }
        window.setAcceptsMouseMovedEvents(true);

        let view = XuiView::new(mtm, content);
        window.setContentView(Some(view.as_super()));
        window.makeFirstResponder(Some(view.as_super().as_super()));

        let resizing = Arc::new(AtomicBool::new(false));
        let delegate = WindowDelegate::new(mtm, Arc::clone(&resizing));
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        window.center();
        if options.maximized {
            window.zoom(None);
        }

        let this = Self {
            ns_view: Retained::as_ptr(&view) as usize,
            window: MainThreadBound::new(window, mtm),
            view: MainThreadBound::new(view, mtm),
            delegate: MainThreadBound::new(delegate, mtm),
            metrics: Mutex::new((PhysicalSize::default(), 1.0)),
            resizing,
            menu_bar: MainThreadBound::new(RefCell::new(None), mtm),
        };
        this.metrics();
        Arc::new(this)
    }

    /// Routes the view's and the window delegate's events to `sink`.
    pub(crate) fn attach(&self, mtm: MainThreadMarker, sink: Weak<dyn EventSink>) {
        self.view.get(mtm).set_sink(sink.clone());
        self.delegate.get(mtm).set_sink(sink);
    }

    /// Lets the view answer hit tests -- is this point a title bar? -- from what
    /// the runtime has published, without calling into the shell.
    pub(crate) fn set_published(&self, mtm: MainThreadMarker, published: PublishedHandle) {
        self.view.get(mtm).set_published(published);
    }

    /// Starts moving the window with the pointer, from the press that is going
    /// on. `false` if no button is down.
    pub(crate) fn begin_window_drag(&self, mtm: MainThreadMarker) -> bool {
        self.view.get(mtm).begin_window_drag()
    }

    pub fn ns_window(&self, mtm: MainThreadMarker) -> &NSWindow {
        self.window.get(mtm)
    }

    pub fn ns_view(&self, mtm: MainThreadMarker) -> &NSView {
        self.view.get(mtm).as_super()
    }

    fn metrics(&self) -> (PhysicalSize, f64) {
        let mut cached = self.metrics.lock().unwrap();
        if let Some(mtm) = MainThreadMarker::new() {
            *cached = (
                backing_size(self.ns_view(mtm)),
                self.window.get(mtm).backingScaleFactor(),
            );
        }
        *cached
    }
}

impl Drop for MacWindow {
    fn drop(&mut self) {
        if let Some(mtm) = MainThreadMarker::new() {
            self.view.get(mtm).invalidate_display_link();
            let window = self.window.get(mtm);
            window.setDelegate(None);
            window.close();
        }
    }
}

impl HasWindowHandle for MacWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let view = NonNull::new(self.ns_view as *mut c_void).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(view));
        // SAFETY: the view is retained by `self`, which outlives the borrow.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for MacWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::appkit())
    }
}

impl SurfaceTarget for MacWindow {
    fn surface_size(&self) -> PhysicalSize {
        self.metrics().0
    }

    fn scale_factor(&self) -> f64 {
        self.metrics().1
    }

    fn in_live_resize(&self) -> bool {
        // Both: the resize callback covers a resize that is not a drag -- a zoom,
        // a fullscreen transition -- and `inLiveResize` covers the frames a drag
        // paints between callbacks.
        self.resizing.load(Ordering::Relaxed)
            || MainThreadMarker::new().is_some_and(|mtm| self.ns_view(mtm).inLiveResize())
    }
}

impl PlatformWindow for MacWindow {
    fn request_redraw(&self) {
        match MainThreadMarker::new() {
            Some(mtm) => self.view.get(mtm).request_redraw(),
            // The view cannot be touched from here; the main-queue path hops
            // over for us.
            None => host::request_redraw(),
        }
    }

    fn set_visible(&self, visible: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let window = self.window.get(mtm);
        if visible {
            window.makeKeyAndOrderFront(None);
        } else {
            window.orderOut(None);
        }
    }

    fn focus(&self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        // An unbundled binary launched from a terminal is not activated on its
        // own. `activate` is the replacement, but needs macOS 14.
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        self.window.get(mtm).makeKeyAndOrderFront(None);
    }

    fn is_minimized(&self) -> Option<bool> {
        MainThreadMarker::new().map(|mtm| self.window.get(mtm).isMiniaturized())
    }

    fn set_cursor(&self, cursor: CursorIcon) {
        if let Some(mtm) = MainThreadMarker::new() {
            self.view.get(mtm).set_cursor(cursor);
        }
    }

    fn set_ime_allowed(&self, allowed: bool) {
        if let Some(mtm) = MainThreadMarker::new() {
            self.view.get(mtm).set_ime_allowed(allowed);
        }
    }

    fn set_ime_cursor_area(&self, area: Rect) {
        if let Some(mtm) = MainThreadMarker::new() {
            self.view.get(mtm).set_ime_cursor_area(area);
        }
    }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities::CLIPBOARD
            | HostCapabilities::FULLSCREEN
            | HostCapabilities::WINDOW_DRAG
            | HostCapabilities::NATIVE_MENU_BAR
            | HostCapabilities::OPEN_URL
            | HostCapabilities::TIMED_REDRAW
    }

    fn request_redraw_at(&self, at: Instant) {
        host::request_redraw_at(at);
    }

    fn set_title(&self, title: &str) {
        if let Some(mtm) = MainThreadMarker::new() {
            self.window.get(mtm).setTitle(&NSString::from_str(title));
        }
    }

    fn set_menu_bar(&self, menu_bar: Option<&MenuBar>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let menus = menu::complete_menu_bar(&menu::application_name(), menu_bar);
        let built = menu::build_menu_bar(mtm, &menus);
        NSApplication::sharedApplication(mtm).setMainMenu(Some(&built.bar));
        // The previous bar goes now that nothing shows it; its target with it.
        self.menu_bar.get(mtm).replace(Some(built));
    }

    fn perform(&self, command: PlatformCommand) -> Performed {
        match MainThreadMarker::new() {
            Some(mtm) => perform::perform(mtm, self, command),
            None => Performed::Failed(xui_interface::PlatformError::new(
                "AppKit can only be driven from the main thread",
            )),
        }
    }
}

/// A requested window size in points.
fn logical_size(mtm: MainThreadMarker, size: WindowSize) -> (f64, f64) {
    match size {
        WindowSize::Logical(size) => (size.width as f64, size.height as f64),
        WindowSize::Physical(size) => {
            // The window has no screen yet. The main screen is where it opens,
            // and so the best guess at the scale it will have.
            let scale = NSScreen::mainScreen(mtm).map_or(1.0, |screen| screen.backingScaleFactor());
            (size.width as f64 / scale, size.height as f64 / scale)
        }
    }
}

fn backing_size(view: &NSView) -> PhysicalSize {
    let rect = view.convertRectToBacking(view.bounds());
    PhysicalSize::new(
        rect.size.width.round().max(0.0) as u32,
        rect.size.height.round().max(0.0) as u32,
    )
}

fn notification_window(notification: &NSNotification) -> Option<Retained<NSWindow>> {
    notification.object()?.downcast::<NSWindow>().ok()
}

fn content_size(window: &NSWindow) -> PhysicalSize {
    window
        .contentView()
        .map_or_else(PhysicalSize::default, |view| backing_size(&view))
}

struct WindowDelegateIvars {
    sink: OnceCell<Weak<dyn EventSink>>,
    /// See [`MacWindow::resizing`].
    resizing: Arc<AtomicBool>,
    /// The last window state the shell was told about.
    state: Cell<WindowState>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `WindowDelegate`
    // does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "XuiWindowDelegate"]
    #[ivars = WindowDelegateIvars]
    struct WindowDelegate;

    unsafe impl NSObjectProtocol for WindowDelegate {}

    unsafe impl NSWindowDelegate for WindowDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            // The shell decides. When it exits, the run ends and the window is
            // closed with it.
            self.send(ShellEvent::CloseRequested);
            false
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &NSNotification) {
            let Some(window) = notification_window(notification) else {
                return;
            };
            // The shell paints a resize at once while one is "live", so that the
            // new picture and the window's new edge reach the screen together
            // instead of the old frame stretched over the new size. AppKit says
            // so for a drag; any other resize -- a zoom, a fullscreen transition
            // -- is the same situation, so it is wrapped the same way.
            let drag = window.inLiveResize();
            self.ivars().resizing.store(true, Ordering::Relaxed);
            if !drag {
                self.send(ShellEvent::LiveResize(true));
            }
            self.send(ShellEvent::Resized(content_size(&window)));
            if !drag {
                self.send(ShellEvent::LiveResize(false));
            }
            self.ivars().resizing.store(false, Ordering::Relaxed);
            self.report_window_state(&window);
        }

        #[unsafe(method(windowWillStartLiveResize:))]
        fn window_will_start_live_resize(&self, _notification: &NSNotification) {
            self.send(ShellEvent::LiveResize(true));
        }

        #[unsafe(method(windowDidEndLiveResize:))]
        fn window_did_end_live_resize(&self, _notification: &NSNotification) {
            self.send(ShellEvent::LiveResize(false));
        }

        #[unsafe(method(windowDidEnterFullScreen:))]
        fn window_did_enter_full_screen(&self, notification: &NSNotification) {
            if let Some(window) = notification_window(notification) {
                self.report_window_state(&window);
            }
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn window_did_exit_full_screen(&self, notification: &NSNotification) {
            if let Some(window) = notification_window(notification) {
                self.report_window_state(&window);
            }
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            self.send(ShellEvent::Focused(true));
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            self.send(ShellEvent::Focused(false));
        }

        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn window_did_change_occlusion_state(&self, notification: &NSNotification) {
            if let Some(window) = notification_window(notification) {
                let visible = window
                    .occlusionState()
                    .contains(NSWindowOcclusionState::Visible);
                self.send(ShellEvent::Occluded(!visible));
            }
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn window_did_change_backing_properties(&self, notification: &NSNotification) {
            if let Some(window) = notification_window(notification) {
                self.send(ShellEvent::ScaleFactorChanged(window.backingScaleFactor()));
                self.send(ShellEvent::Resized(content_size(&window)));
            }
        }
    }
);

impl WindowDelegate {
    fn new(mtm: MainThreadMarker, resizing: Arc<AtomicBool>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WindowDelegateIvars {
            sink: OnceCell::new(),
            resizing,
            state: Cell::new(WindowState::default()),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn set_sink(&self, sink: Weak<dyn EventSink>) {
        let _ = self.ivars().sink.set(sink);
    }

    fn send(&self, event: ShellEvent) {
        if let Some(sink) = self.ivars().sink.get().and_then(Weak::upgrade) {
            let _ = sink.send(Instant::now(), event);
        }
    }

    /// Tells the shell when the window went fullscreen or was zoomed by someone
    /// other than the runtime -- the green button, a double-click on the title
    /// bar -- so what the application reads from the platform handle is true.
    fn report_window_state(&self, window: &NSWindow) {
        let state = WindowState {
            fullscreen: window.styleMask().contains(NSWindowStyleMask::FullScreen),
            maximized: window.isZoomed(),
        };
        if self.ivars().state.replace(state) != state {
            self.send(ShellEvent::WindowStateChanged(state));
        }
    }
}
