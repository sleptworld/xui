//! The bridge from AppKit callbacks to the [`Shell`].

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::time::Instant;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::NSPoint;
use xui_core::app::AppRenderError;
use xui_core::render::RenderBackend;
use xui_core::text::TextHost;
use xui_interface::TextBackend;
use xui_shell::{PublishedHandle, ShellControl, ShellEvent, ShellHandle, ShellOutcome};

use crate::window::MacWindow;

/// Where AppKit callbacks deliver events. Object-safe, because the
/// Objective-C classes holding one cannot be generic over the backend.
pub(crate) trait EventSink {
    /// Delivers `event`, which happened at `at`, and says what became of it.
    ///
    /// Safe to call at any moment, including from inside a call the shell itself
    /// is making: the event is queued and the call already running handles it.
    fn send(&self, at: Instant, event: ShellEvent) -> ShellOutcome;

    /// What the runtime has published about the window's surface, once there is
    /// a shell to publish it.
    fn published(&self) -> Option<PublishedHandle>;
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

/// Sends `event` to the running host, stamped with the time it is called.
pub(crate) fn send_current(event: ShellEvent) {
    // Upgraded and released before sending: the sink may call back in here.
    let sink = CURRENT.with(|current| current.borrow().as_ref().and_then(Weak::upgrade));
    if let Some(sink) = sink {
        let _ = sink.send(Instant::now(), event);
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
    shell: RefCell<Option<Rc<ShellHandle<MacWindow, B, T>>>>,
    /// Events that arrive before there is a shell to give them to.
    ///
    /// Starting the shell shows the window, and AppKit reports that
    /// synchronously, so a focus event can arrive while `Shell::new` is still
    /// running. Once the shell is installed, [`ShellHandle`] does this job
    /// itself, for the events that arrive while it is busy.
    starting: RefCell<VecDeque<(Instant, ShellEvent)>>,
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
            starting: RefCell::new(VecDeque::new()),
            exited: Cell::new(false),
            render_error: RefCell::new(None),
        }
    }

    /// Hands the host its shell, then delivers whatever arrived while the
    /// shell was starting.
    pub(crate) fn install(&self, shell: ShellHandle<MacWindow, B, T>, control: ShellControl) {
        let shell = Rc::new(shell);
        self.apply(&shell, control);
        *self.shell.borrow_mut() = Some(Rc::clone(&shell));
        loop {
            let next = self.starting.borrow_mut().pop_front();
            let Some((at, event)) = next else {
                break;
            };
            let _ = self.deliver(&shell, at, event);
        }
    }

    pub(crate) fn take_render_error(&self) -> Option<AppRenderError<B::Error>> {
        self.render_error.borrow_mut().take()
    }

    /// Drops the shell -- the backend and the window with it -- here on the
    /// main thread, instead of wherever the last reference happens to go.
    pub(crate) fn shut_down(&self) {
        self.starting.borrow_mut().clear();
        let shell = self.shell.borrow_mut().take();
        drop(shell);
    }

    fn deliver(
        &self,
        shell: &ShellHandle<MacWindow, B, T>,
        at: Instant,
        event: ShellEvent,
    ) -> ShellOutcome {
        let outcome = shell.submit(at, event);
        self.apply(shell, outcome.control);
        // AppKit has no "about to wait" callback worth the trouble; the moment an
        // event has been handled is the same moment for everything the shell does
        // with it. A no-op if this was a nested call and the outer one is still
        // running.
        let control = shell.about_to_wait();
        self.apply(shell, control);
        outcome
    }

    fn apply(&self, shell: &ShellHandle<MacWindow, B, T>, control: ShellControl) {
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
    fn send(&self, at: Instant, event: ShellEvent) -> ShellOutcome {
        if self.exited.get() {
            return ShellOutcome::CONTINUE;
        }
        // Cloned out so the cell is not held while the shell runs: the shell may
        // call back in here.
        let shell = self.shell.borrow().clone();
        match shell {
            Some(shell) => self.deliver(&shell, at, event),
            None => {
                self.starting.borrow_mut().push_back((at, event));
                ShellOutcome::CONTINUE
            }
        }
    }

    fn published(&self) -> Option<PublishedHandle> {
        self.shell.borrow().as_ref().map(|shell| shell.published())
    }
}
