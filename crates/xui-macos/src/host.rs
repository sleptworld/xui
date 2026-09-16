//! The bridge from AppKit callbacks to the [`Shell`].

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Weak;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::NSPoint;
use xui_core::app::AppRenderError;
use xui_core::render::RenderBackend;
use xui_core::text::TextHost;
use xui_interface::TextBackend;
use xui_shell::{Shell, ShellControl, ShellEvent};

use crate::window::MacWindow;

/// Where AppKit callbacks deliver events. Object-safe, because the
/// Objective-C classes holding one cannot be generic over the backend.
pub(crate) trait EventSink {
    fn send(&self, event: ShellEvent);
}

thread_local! {
    /// The running host. A run has one window and so one sink; callbacks with
    /// no object to hang a sink on -- dispatch blocks, the app delegate --
    /// find it here.
    static CURRENT: RefCell<Option<Weak<dyn EventSink>>> = const { RefCell::new(None) };
    static REDRAW_PENDING: Cell<bool> = const { Cell::new(false) };
}

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

/// Schedules a [`ShellEvent::RedrawRequested`] on the main queue, coalescing
/// every request made before it runs.
///
/// The fallback for redraws: the view drives frames from a display link where
/// there is one, and this carries them on macOS before 14, and whenever a
/// redraw is asked for off the main thread. The main queue is serviced in the
/// run loop's common modes, so frames keep coming during live resize and other
/// tracking loops; pacing then comes from the presenter, whose `nextDrawable`
/// blocks once all of the layer's drawables are in flight.
pub(crate) fn request_redraw() {
    if MainThreadMarker::new().is_none() {
        DispatchQueue::main().exec_async(request_redraw);
        return;
    }
    if REDRAW_PENDING.replace(true) {
        return;
    }
    DispatchQueue::main().exec_async(|| {
        if REDRAW_PENDING.replace(false) {
            send_current(ShellEvent::RedrawRequested);
        }
    });
}

/// Wakes the runtime after async work. Callable from any thread.
pub(crate) fn wake() {
    DispatchQueue::main().exec_async(|| send_current(ShellEvent::Wake));
}

/// Ends `-[NSApplication run]`.
pub(crate) fn stop_app(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.stop(None);
    // `stop:` takes effect once the run loop has finished handling an event,
    // and there may never be another one.
    let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags(0),
        0.0,
        0,
        None,
        0,
        0,
        0,
    );
    if let Some(event) = event {
        app.postEvent_atStart(&event, false);
    }
}

pub(crate) struct Host<B, T>
where
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    shell: RefCell<Option<Shell<MacWindow, B, T>>>,
    /// Events wait here while the shell is busy.
    ///
    /// AppKit calls back synchronously from inside calls the shell makes --
    /// showing the window reports focus, turning IME off can end a
    /// composition -- so an event can arrive while the shell is already
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
    pub(crate) fn install(&self, mut shell: Shell<MacWindow, B, T>, control: ShellControl) {
        self.apply(&mut shell, control);
        *self.shell.borrow_mut() = Some(shell);
        self.drain();
    }

    pub(crate) fn take_render_error(&self) -> Option<AppRenderError<B::Error>> {
        self.render_error.borrow_mut().take()
    }

    /// Drops the shell -- the backend and the window with it -- here on the
    /// main thread, instead of wherever the last reference happens to go.
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
        // AppKit has no "about to wait" callback worth the trouble; a drained
        // queue is the same moment for everything the shell does with it.
        let control = shell.about_to_wait();
        self.apply(shell, control);
    }

    fn apply(&self, shell: &mut Shell<MacWindow, B, T>, control: ShellControl) {
        if control != ShellControl::Exit || self.exited.replace(true) {
            return;
        }
        *self.render_error.borrow_mut() = shell.take_render_error();
        if let Some(mtm) = MainThreadMarker::new() {
            stop_app(mtm);
        }
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
