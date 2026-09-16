use std::cell::Cell;
use std::num::NonZeroIsize;
use std::sync::Mutex;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{HBRUSH, InvalidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DestroyWindow, GetClientRect, HCURSOR,
    HICON, HMENU, IsIconic, RegisterClassExW, SW_HIDE, SW_MAXIMIZE, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSEXW, WS_CAPTION, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};
use windows::core::PCWSTR;
use xui_interface::{CursorIcon, Rect};
use xui_shell::{PhysicalSize, PlatformWindow, SurfaceTarget, WindowOptions, WindowSize};

use crate::wndproc::wndproc;
use crate::{cursor, ime};

/// The DPI Windows reports for an unscaled display; every scale factor is a
/// ratio against it.
pub(crate) const BASE_DPI: f64 = 96.0;

const CLASS_NAME: PCWSTR = windows::core::w!("XuiWindow");

thread_local! {
    /// Per-window state the window procedure and [`WinWindow`] both touch.
    ///
    /// A run has one window, so this is it. Every field is a `Cell`: the window
    /// procedure is reentrant, and a borrow held across a `send` would be a
    /// panic waiting for the first message that arrives from inside another.
    pub(crate) static STATE: WindowState = WindowState::new();
}

pub(crate) struct WindowState {
    pub(crate) hwnd: Cell<isize>,
    pub(crate) modifiers: Cell<xui_interface::Modifiers>,
    pub(crate) last_position: Cell<Option<xui_interface::Point>>,
    /// Buttons currently down, as a bitmask, so the mouse is captured for as
    /// long as any of them is -- a drag that leaves the window still reports.
    pub(crate) buttons_down: Cell<u32>,
    pub(crate) cursor: Cell<isize>,
    pub(crate) cursor_hidden: Cell<bool>,
    pub(crate) ime_allowed: Cell<bool>,
    /// The candidate-window anchor, in logical window coordinates.
    pub(crate) ime_area: Cell<Rect>,
    pub(crate) composing: Cell<bool>,
    /// Set when a redraw has been asked for and `WM_PAINT` has not arrived.
    pub(crate) redraw_pending: Cell<bool>,
    /// Minimum client size in logical units, enforced in `WM_GETMINMAXINFO`.
    pub(crate) min_size: Cell<Option<(f64, f64)>>,
}

impl WindowState {
    fn new() -> Self {
        Self {
            hwnd: Cell::new(0),
            modifiers: Cell::new(xui_interface::Modifiers::default()),
            last_position: Cell::new(None),
            buttons_down: Cell::new(0),
            cursor: Cell::new(0),
            cursor_hidden: Cell::new(false),
            ime_allowed: Cell::new(false),
            ime_area: Cell::new(Rect::new(0.0, 0.0, 0.0, 0.0)),
            composing: Cell::new(false),
            redraw_pending: Cell::new(false),
            min_size: Cell::new(None),
        }
    }
}

/// A Win32 window with an `xui` client area.
///
/// The handle is kept as a raw `isize` so the window is `Send + Sync`, as
/// `SurfaceTarget` requires. Every call that touches it has to happen on the
/// thread that created it -- Win32's rule, not this crate's -- so a
/// `PlatformWindow` request from any other thread is ignored, and size queries
/// there answer from the last value read on the owning thread.
pub struct WinWindow {
    hwnd: isize,
    /// The thread that owns the window's message queue.
    thread: u32,
    metrics: Mutex<(PhysicalSize, f64)>,
}

impl WinWindow {
    /// Creates the window, hidden, as `Shell::new` requires.
    pub(crate) fn new(options: &WindowOptions) -> windows::core::Result<Self> {
        // SAFETY: every call below is a plain Win32 call with checked
        // arguments; the class is registered before it is named.
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                // Resizing invalidates the whole client area, so a frame is
                // asked for as part of the resize rather than after it.
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                // No class cursor: `WM_SETCURSOR` sets whatever the runtime
                // last asked for. No background brush: every pixel is painted
                // by the render backend, and letting GDI erase first flickers.
                hCursor: HCURSOR::default(),
                hbrBackground: HBRUSH::default(),
                hIcon: HICON::default(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            // Registering a class that already exists fails, which is fine and
            // expected on a second run in the same process.
            let _ = RegisterClassExW(&class);

            let mut style = if options.decorations {
                WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN | WS_CLIPSIBLINGS
            } else {
                WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS
            };
            if options.resizable {
                style |= WS_THICKFRAME | WS_MAXIMIZEBOX;
            }

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS_NAME,
                &encode_wide(&options.title),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None::<HMENU>,
                Some(instance.into()),
                None,
            )?;

            let window = Self {
                hwnd: hwnd.0 as isize,
                thread: windows::Win32::System::Threading::GetCurrentThreadId(),
                metrics: Mutex::new((PhysicalSize::default(), 1.0)),
            };
            STATE.with(|state| {
                state.hwnd.set(window.hwnd);
                state.min_size.set(
                    options
                        .min_inner_size
                        .map(|size| logical_size(size, window.scale_factor())),
                );
            });
            crate::host::set_wake_target(hwnd);

            // Sized after creation rather than in `CreateWindowExW`: the window
            // has no DPI until it exists, and on a scaled monitor the size the
            // caller asked for means something different than it would at 96.
            let (width, height) = options
                .inner_size
                .map(|size| logical_size(size, window.scale_factor()))
                .unwrap_or((800.0, 600.0));
            window.set_client_size(style, width, height)?;
            window.refresh_metrics();
            Ok(window)
        }
    }

    /// The window's handle, for anything Win32 offers that this crate does not.
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    pub(crate) fn handle(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    /// Whether the caller is on the thread that owns the window.
    fn owned_here(&self) -> bool {
        // SAFETY: no preconditions.
        unsafe { windows::Win32::System::Threading::GetCurrentThreadId() == self.thread }
    }

    /// Resizes so that the *client* area is the requested logical size.
    fn set_client_size(
        &self,
        style: WINDOW_STYLE,
        width: f64,
        height: f64,
    ) -> windows::core::Result<()> {
        let scale = self.scale_factor();
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (width * scale).round() as i32,
            bottom: (height * scale).round() as i32,
        };
        // SAFETY: `rect` is a valid out-parameter for the lifetime of the call.
        unsafe {
            AdjustWindowRectExForDpi(
                &mut rect,
                style,
                false,
                WINDOW_EX_STYLE::default(),
                (scale * BASE_DPI) as u32,
            )?;
            SetWindowPos(
                self.handle(),
                None,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
    }

    pub(crate) fn refresh_metrics(&self) -> (PhysicalSize, f64) {
        let mut cached = self.metrics.lock().unwrap();
        if self.owned_here() {
            *cached = (client_size(self.handle()), scale_of(self.handle()));
        }
        *cached
    }
}

impl Drop for WinWindow {
    fn drop(&mut self) {
        if !self.owned_here() {
            return;
        }
        crate::host::set_wake_target(HWND::default());
        STATE.with(|state| state.hwnd.set(0));
        // SAFETY: the window belongs to this thread and has not been destroyed
        // -- `WM_CLOSE` is answered without destroying it.
        unsafe {
            let _ = DestroyWindow(self.handle());
        }
    }
}

impl HasWindowHandle for WinWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.hwnd).ok_or(HandleError::Unavailable)?;
        let mut handle = Win32WindowHandle::new(hwnd);
        // SAFETY: no preconditions; the module handle is process-wide.
        if let Ok(instance) = unsafe { GetModuleHandleW(None) } {
            handle.hinstance = NonZeroIsize::new(instance.0 as isize);
        }
        let raw = RawWindowHandle::Win32(handle);
        // SAFETY: the window is alive for as long as `self`, which outlives the
        // borrow.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for WinWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

impl SurfaceTarget for WinWindow {
    fn surface_size(&self) -> PhysicalSize {
        self.refresh_metrics().0
    }

    fn scale_factor(&self) -> f64 {
        if self.owned_here() {
            scale_of(self.handle())
        } else {
            self.metrics.lock().unwrap().1
        }
    }
}

impl PlatformWindow for WinWindow {
    fn request_redraw(&self) {
        if !self.owned_here() {
            return;
        }
        STATE.with(|state| state.redraw_pending.set(true));
        // Coalesced by the window manager: repeated invalidations before the
        // queue is drained still produce one `WM_PAINT`.
        // SAFETY: a live window handle; a null rect means the whole client area.
        unsafe {
            let _ = InvalidateRect(Some(self.handle()), None, false);
        }
    }

    fn set_visible(&self, visible: bool) {
        if !self.owned_here() {
            return;
        }
        // SAFETY: a live window handle.
        unsafe {
            let _ = ShowWindow(self.handle(), if visible { SW_SHOW } else { SW_HIDE });
        }
    }

    fn focus(&self) {
        if !self.owned_here() {
            return;
        }
        // SAFETY: a live window handle.
        unsafe {
            let _ = SetForegroundWindow(self.handle());
        }
    }

    fn is_minimized(&self) -> Option<bool> {
        if !self.owned_here() {
            return None;
        }
        // SAFETY: a live window handle.
        Some(unsafe { IsIconic(self.handle()).as_bool() })
    }

    fn set_cursor(&self, icon: CursorIcon) {
        if self.owned_here() {
            cursor::apply(icon);
        }
    }

    fn set_ime_allowed(&self, allowed: bool) {
        if self.owned_here() {
            ime::set_allowed(self.handle(), allowed);
        }
    }

    fn set_ime_cursor_area(&self, area: Rect) {
        if !self.owned_here() {
            return;
        }
        STATE.with(|state| state.ime_area.set(area));
        ime::move_candidate_window(self.handle());
    }
}

/// Maximizes the window, for [`WindowOptions::maximized`].
pub(crate) fn maximize(hwnd: HWND) {
    // SAFETY: a live window handle.
    unsafe {
        let _ = ShowWindow(hwnd, SW_MAXIMIZE);
    }
}

pub(crate) fn scale_of(hwnd: HWND) -> f64 {
    // SAFETY: a live window handle. Returns 0 for an invalid one, which the
    // guard below turns into an unscaled display rather than a division by zero.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / BASE_DPI
    }
}

pub(crate) fn client_size(hwnd: HWND) -> PhysicalSize {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid out-parameter for the lifetime of the call.
    if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
        return PhysicalSize::default();
    }
    PhysicalSize::new(
        (rect.right - rect.left).max(0) as u32,
        (rect.bottom - rect.top).max(0) as u32,
    )
}

/// A requested window size in logical units.
fn logical_size(size: WindowSize, scale: f64) -> (f64, f64) {
    match size {
        WindowSize::Logical(size) => (size.width as f64, size.height as f64),
        WindowSize::Physical(size) => (
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
        ),
    }
}

pub(crate) fn encode_wide(text: &str) -> PCWSTR {
    // Leaked deliberately: a `PCWSTR` borrows, and the alternative is threading
    // the buffer's lifetime through window creation for the sake of one title.
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    PCWSTR(Box::leak(wide.into_boxed_slice()).as_ptr())
}

/// The client-area position carried by a mouse message, in logical units.
pub(crate) fn mouse_position(lparam: LPARAM, scale: f64) -> xui_interface::Point {
    let x = (lparam.0 & 0xFFFF) as i16 as f64;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f64;
    xui_interface::Point::new((x / scale) as f32, (y / scale) as f32)
}

/// The wheel delta carried by `WM_MOUSEWHEEL`, in notches.
pub(crate) fn wheel_delta(wparam: WPARAM) -> f32 {
    const WHEEL_DELTA: f32 = 120.0;
    (((wparam.0 >> 16) & 0xFFFF) as i16 as f32) / WHEEL_DELTA
}

/// Converts a screen point to client coordinates, for the wheel messages,
/// which report where the pointer is on the desktop rather than in the window.
pub(crate) fn screen_to_client(hwnd: HWND, lparam: LPARAM, scale: f64) -> xui_interface::Point {
    let mut point = POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    };
    // SAFETY: `point` is a valid in-out parameter for the lifetime of the call.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    xui_interface::Point::new(
        (f64::from(point.x) / scale) as f32,
        (f64::from(point.y) / scale) as f32,
    )
}
