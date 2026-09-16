//! The bridge from window-procedure callbacks to the [`Shell`].

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Weak;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, PostQuitMessage, WM_APP};
use xui_core::app::AppRenderError;
use xui_core::render::RenderBackend;
use xui_core::text::TextHost;
use xui_interface::TextBackend;
use xui_shell::{Shell, ShellControl, ShellEvent};

use crate::window::WinWindow;

/// Posted to wake the runtime after async work finished on another thread.
pub(crate) const WM_XUI_WAKE: u32 = WM_APP + 1;

/// Where window-procedure callbacks deliver events. Object-safe, because the
/// window procedure is a bare `extern "system"` function and cannot be generic
/// over the backend.
pub(crate) trait EventSink {
    fn send(&self, event: ShellEvent);
}

thread_local! {
    /// The running host. A run has one window and so one sink; the window
    /// procedure finds it here rather than through `GWLP_USERDATA`, which
    /// would have to be set before the first message arrives.
    static CURRENT: RefCell<Option<Weak<dyn EventSink>>> = const { RefCell::new(None) };
}

/// The window to post wake messages to, as a raw `HWND`. Read from whatever
/// thread finished the async work, which is why it is not the thread-local
/// above.
static WAKE_TARGET: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn set_current(sink: Option<Weak<dyn EventSink>>) {
    CURRENT.with(|current| *current.borrow_mut() = sink);
}

pub(crate) fn send_current(event: ShellEvent) {
    // Upgraded and released before sending: the sink may call back in here.
    let sink = CURRENT.with(|current| current.borrow().as_ref().and_then(Weak::upgrade));
    if let Some(sink) = sink {
        sink.send(event);
    }
}

pub(crate) fn set_wake_target(hwnd: HWND) {
    WAKE_TARGET.store(hwnd.0 as isize, Ordering::Release);
}

/// Wakes the runtime after async work. Callable from any thread: `PostMessage`
/// is the one documented way into a window's queue from outside its thread.
pub(crate) fn wake() {
    let hwnd = WAKE_TARGET.load(Ordering::Acquire);
    if hwnd == 0 {
        return;
    }
    // SAFETY: the handle was stored by the window that owns the queue, and is
    // cleared before that window is destroyed. A stale post fails harmlessly.
    unsafe {
        let _ = PostMessageW(
            Some(HWND(hwnd as *mut _)),
            WM_XUI_WAKE,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// Ends the message loop.
fn stop_loop() {
    // SAFETY: posts WM_QUIT to this thread's queue; no preconditions.
    unsafe { PostQuitMessage(0) };
}

pub(crate) struct Host<B, T>
where
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    shell: RefCell<Option<Shell<WinWindow, B, T>>>,
    /// Events wait here while the shell is busy.
    ///
    /// A window procedure is reentrant: calls the shell makes -- showing the
    /// window, moving it, turning IME off -- send further messages
    /// synchronously, so a callback can arrive while the shell is already
    /// borrowed. It is queued, and the `drain` already running picks it up.
    queue: RefCell<VecDeque<ShellEvent>>,
    exited: Cell<bool>,
    render_error: RefCell<Option<AppRenderError<B::Error>>>,
}

impl<B, T> Host<B, T>
where
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    pub(crate) fn new() -> Self {
        Self {
            shell: RefCell::new(None),
            queue: RefCell::new(VecDeque::new()),
            exited: Cell::new(false),
            render_error: RefCell::new(None),
        }
    }

    /// Hands the host its shell, then delivers whatever arrived while the
    /// shell was starting.
    pub(crate) fn install(&self, mut shell: Shell<WinWindow, B, T>, control: ShellControl) {
        self.apply(&mut shell, control);
        *self.shell.borrow_mut() = Some(shell);
        self.drain();
    }

    pub(crate) fn take_render_error(&self) -> Option<AppRenderError<B::Error>> {
        self.render_error.borrow_mut().take()
    }

    /// Drops the shell -- the backend and the window with it -- on the thread
    /// that owns them, instead of wherever the last reference happens to go.
    pub(crate) fn shut_down(&self) {
        self.queue.borrow_mut().clear();
        let shell = self.shell.borrow_mut().take();
        drop(shell);
    }

    fn drain(&self) {
        let Ok(mut slot) = self.shell.try_borrow_mut() else {
            return;
        };
        let Some(shell) = slot.as_mut() else {
            return;
        };
        loop {
            if self.exited.get() {
                self.queue.borrow_mut().clear();
                return;
            }
            let next = self.queue.borrow_mut().pop_front();
            let Some(event) = next else {
                break;
            };
            let control = shell.handle_event(event);
            self.apply(shell, control);
        }
        // Win32 has no "about to wait" callback worth the trouble; a drained
        // queue is the same moment for everything the shell does with it.
        let control = shell.about_to_wait();
        self.apply(shell, control);
    }

    fn apply(&self, shell: &mut Shell<WinWindow, B, T>, control: ShellControl) {
        if control != ShellControl::Exit || self.exited.replace(true) {
            return;
        }
        *self.render_error.borrow_mut() = shell.take_render_error();
        stop_loop();
    }
}

impl<B, T> EventSink for Host<B, T>
where
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    fn send(&self, event: ShellEvent) {
        if self.exited.get() {
            return;
        }
        self.queue.borrow_mut().push_back(event);
        self.drain();
    }
}
