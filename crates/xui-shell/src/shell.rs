use std::sync::Arc;
use std::time::{Duration, Instant};

use xui_core::App;
use xui_core::app::AppRenderError;
use xui_core::render::RenderBackend;
use xui_core::runtime::{ControlFlow, GuiRuntime, RuntimeEvent};
use xui_core::text::TextHost;
use xui_interface::events::{
    KeyState, KeyText, NamedKey, PhysicalKey, RawEvent, RawIme, RawKeyboard, XuiDeviceId,
    XuiPointerId,
};
use xui_interface::{
    Appearance, CommandId, ContextMenuTrigger, DragEvent, GestureKind, GesturePhase, Modifiers,
    PlatformCommand, PlatformError, PlatformOutput, PlatformReply, Point, PointerButton,
    PointerButtons, PointerKind, RawContextMenu, RawGesture, RawPointerButton, RawPointerCancel,
    RawPointerLeave, RawPointerMove, RawWheel, RawWindowEvent, RequestId, ScrollDelta, TextBackend,
    WindowState,
};

use crate::published::PublishedHandle;
use crate::window::{Performed, PhysicalSize, PlatformWindow};

/// A platform event, already translated out of the host's vocabulary.
///
/// Sizes are physical pixels, because that is what a surface is sized in;
/// positions and scroll deltas are logical, relative to the content area.
/// Timestamps and modifier state are filled in by [`Shell`], from the instant
/// the host passes to [`Shell::dispatch_event`].
///
/// New variants are added as hosts learn to produce them, so a `match` outside
/// this crate needs a wildcard arm.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ShellEvent {
    Resized(PhysicalSize),
    ScaleFactorChanged(f64),
    /// The user asked to close the window, or it is already gone.
    CloseRequested,
    Focused(bool),
    Occluded(bool),
    ModifiersChanged(Modifiers),
    PointerMoved {
        position: Point,
        device_id: Option<XuiDeviceId>,
    },
    PointerButton {
        button: PointerButton,
        pressed: bool,
        device_id: Option<XuiDeviceId>,
    },
    Wheel {
        delta: ScrollDelta,
        device_id: Option<XuiDeviceId>,
        is_inertial: bool,
    },
    Keyboard(KeyInput),
    Ime(RawIme),
    RedrawRequested,
    /// Async work finished; delivered by the callback the host registered with
    /// [`App::set_async_wake_callback`].
    Wake,

    /// The user started (`true`) or finished dragging the window's edge.
    ///
    /// While it lasts, a [`Self::Resized`] is painted on the spot instead of on
    /// the next requested frame: the platform shows the new window geometry
    /// right away, and a frame one turn late shows the old picture stretched
    /// over it. Hosts used to each write that rule themselves.
    LiveResize(bool),
    /// A frame, with what the host knows about when it will be shown. Stands in
    /// for [`Self::RedrawRequested`] on a host that has a display link.
    FrameTick(FrameInfo),
    /// The pointer left the window with no button held. Clears hover, which a
    /// move cannot: nothing moves once the pointer is outside.
    PointerLeft,
    /// The host lost the pointer without a release: capture stolen, an
    /// interrupted native drag, the window deactivated mid-press.
    PointerCancelled,
    ContextMenuRequested {
        /// `None` when the trigger has no position, as a keyboard's does.
        position: Option<Point>,
        trigger: ContextMenuTrigger,
    },
    Touch {
        /// Distinguishes simultaneous fingers; stable from `Started` to
        /// `Ended` or `Cancelled`.
        id: u64,
        phase: TouchPhase,
        position: Point,
        device_id: Option<XuiDeviceId>,
    },
    Gesture {
        kind: GestureKind,
        phase: GesturePhase,
        position: Point,
    },
    /// A menu item was chosen, or the system raised one of its standard
    /// actions (`xui_interface::commands`).
    Command(CommandId),
    /// The answer to a request the host replied `Pending` to.
    Reply(RequestId, PlatformReply),
    AppearanceChanged(Appearance),
    /// The window went fullscreen or was maximized, or left either, by the
    /// user's doing rather than the runtime's.
    WindowStateChanged(WindowState),
    /// A drag crossing the window. The answer the platform wants while it is in
    /// flight comes from [`PublishedHandle::drop_effect_at`], not from here.
    Drag(DragEvent),
}

/// A key event without the state [`Shell`] tracks itself.
#[derive(Debug, Clone)]
pub struct KeyInput {
    pub physical_key: PhysicalKey,
    pub named_key: Option<NamedKey>,
    pub state: KeyState,
    pub text: Option<KeyText>,
    pub is_repeat: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// What a host knows about a frame that the runtime cannot: when it will be on
/// screen. [`Shell`] passes `target` to the runtime as the frame's instant, so
/// animation is paced to when the frame is shown rather than when it was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameInfo {
    /// A display link's target presentation time. `None`: use the time the
    /// frame is built.
    pub target: Option<Instant>,
    /// One refresh period. Varies on adaptive-refresh displays.
    pub interval: Option<Duration>,
}

/// What the host should do with its event loop after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ShellControl {
    Continue,
    /// Stop the event loop. If a frame failed, [`Shell::take_render_error`]
    /// has the error.
    Exit,
}

/// What handling an event meant, returned to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ShellOutcome {
    pub control: ShellControl,
    /// The runtime acted on the event.
    ///
    /// This is what lets a host decide whether the native layer still gets it.
    /// For a key: `performKeyEquivalent:` returns it, and `DefWindowProc` runs
    /// only when it is false. For a wheel: an unconsumed one goes on to an
    /// enclosing view.
    pub consumed: bool,
}

impl ShellOutcome {
    /// Nothing happened that the host needs to react to.
    pub const CONTINUE: Self = Self {
        control: ShellControl::Continue,
        consumed: false,
    };

    fn from_control(control: ShellControl) -> Self {
        Self {
            control,
            consumed: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellOptions {
    /// Whether the window should be revealed after its first frame. The host
    /// creates it hidden regardless.
    pub visible: bool,
    /// Whether [`ShellEvent::CloseRequested`] ends the event loop, rather than
    /// only being forwarded to the runtime.
    pub exit_on_close_requested: bool,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            visible: true,
            exit_on_close_requested: true,
        }
    }
}

/// How many times the shell hands the host commands, delivers the replies, and
/// asks again before it stops. A reply handler may make another request, but a
/// loop that never settles is a bug in the application, not something to spin on.
const MAX_COMMAND_ROUNDS: usize = 8;

/// Drives a [`GuiRuntime`] from platform events.
///
/// The host owns the event loop and the window. It hands each translated event
/// to [`Shell::dispatch_event`], calls [`Shell::about_to_wait`] when the loop
/// goes idle, and acts on the [`ShellOutcome`] each returns. Everything that
/// does not depend on the platform happens here.
///
/// A host whose native callbacks can arrive while it is inside the shell -- all
/// of them can -- should hold a [`crate::ShellHandle`] instead, which queues
/// those events rather than requiring the host to.
pub struct Shell<W, B, T>
where
    W: PlatformWindow,
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    window: Arc<W>,
    runtime: GuiRuntime<B, T>,
    options: ShellOptions,
    last_cursor_position: Option<Point>,
    modifiers: Modifiers,
    pointer_buttons: PointerButtons,
    last_platform_output: PlatformOutput,
    published: PublishedHandle,
    /// Between [`ShellEvent::LiveResize`]`(true)` and `(false)`.
    live_resize: bool,
    /// Last reported by [`ShellEvent::Occluded`]. Not every platform sends it,
    /// so it stays false there and the other checks in
    /// `refresh_window_visibility` carry the platform.
    occluded: bool,
    /// True while the window is held back, waiting for its first frame. Cleared
    /// once it has been shown, and false from the start when the caller asked
    /// for a window that stays hidden.
    pending_first_present: bool,
    render_error: Option<AppRenderError<B::Error>>,
}

impl<W, B, T> Shell<W, B, T>
where
    W: PlatformWindow,
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    /// Starts the runtime on `window` and paints the first frame.
    ///
    /// The host must have created `window` hidden -- it is shown here once
    /// that frame is through the backend, if `options.visible` -- and should
    /// already have wired `app`'s async wake callback to deliver
    /// [`ShellEvent::Wake`] on the event-loop thread.
    pub fn new(
        window: Arc<W>,
        app: App,
        text: T,
        backend: B,
        options: ShellOptions,
    ) -> (Self, ShellControl) {
        let scale_factor = window.scale_factor();
        let mut runtime = GuiRuntime::new(app, backend, text);
        runtime.app_mut().set_scale_factor(scale_factor as f32);
        // Before the first frame, so the application's first build already
        // knows what the host can do and what the system looks like.
        runtime.handle_event(RuntimeEvent::Capabilities(window.capabilities()));
        runtime.handle_event(RuntimeEvent::Appearance(window.appearance()));
        runtime.handle_event(RuntimeEvent::Resize(
            window.surface_size().to_logical(scale_factor),
        ));
        let mut shell = Self {
            pending_first_present: options.visible,
            window,
            runtime,
            options,
            last_cursor_position: None,
            modifiers: Modifiers::default(),
            pointer_buttons: PointerButtons::default(),
            last_platform_output: PlatformOutput::default(),
            published: PublishedHandle::default(),
            live_resize: false,
            occluded: false,
            render_error: None,
        };
        // Paint before handing control back to the event loop: a hidden
        // window is not guaranteed to be sent a redraw, and this is the frame
        // that decides when it becomes visible.
        let control = shell.render();
        (shell, control)
    }

    pub fn window(&self) -> &Arc<W> {
        &self.window
    }

    pub fn runtime(&self) -> &GuiRuntime<B, T> {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut GuiRuntime<B, T> {
        &mut self.runtime
    }

    /// What the runtime has published about the window's surface. The handle
    /// stays current as the shell runs, and can be read without borrowing it.
    pub fn published(&self) -> PublishedHandle {
        self.published.clone()
    }

    /// The error that made a frame fail, if one did.
    pub fn take_render_error(&mut self) -> Option<AppRenderError<B::Error>> {
        self.render_error.take()
    }

    /// [`Self::dispatch_event`] stamped with the time it is called. Kept for a
    /// host that has no better timestamp, and for the control result alone.
    pub fn handle_event(&mut self, event: ShellEvent) -> ShellControl {
        self.dispatch_event(Instant::now(), event).control
    }

    /// Handles `event`, which happened at `at`.
    ///
    /// `at` is when the event happened natively, not when it was dequeued: an
    /// event that waited behind another keeps its own time, which is what
    /// double-click spacing and scroll velocity are measured in.
    pub fn dispatch_event(&mut self, at: Instant, event: ShellEvent) -> ShellOutcome {
        let timestamp = at;
        match event {
            ShellEvent::Resized(size) => {
                let size = size.to_logical(self.window.scale_factor());
                let outcome = if self.live_resize {
                    // The platform is showing the new geometry now. Painting
                    // here, inside the resize, is what keeps the picture and
                    // the window edge together.
                    self.runtime.handle_event(RuntimeEvent::Resize(size));
                    ShellOutcome::from_control(self.render())
                } else {
                    self.dispatch(RuntimeEvent::Resize(size))
                };
                // A minimized window reports no occlusion on Windows or X11;
                // what it reports is a resize to nothing.
                self.refresh_window_visibility();
                outcome
            }
            ShellEvent::ScaleFactorChanged(scale_factor) => {
                self.set_scale_factor(scale_factor);
                ShellOutcome::CONTINUE
            }
            ShellEvent::CloseRequested => self.dispatch(RuntimeEvent::Exit),
            ShellEvent::Focused(focused) => {
                let raw = RawWindowEvent {
                    timestamp,
                    modifiers: self.modifiers,
                };
                let outcome = self.dispatch(RuntimeEvent::Input(if focused {
                    RawEvent::WindowFocus(raw)
                } else {
                    RawEvent::WindowBlur(raw)
                }));
                // Not itself a visibility signal, but a reliable moment at
                // which one may have arrived unannounced.
                self.refresh_window_visibility();
                outcome
            }
            ShellEvent::Occluded(occluded) => {
                self.occluded = occluded;
                self.refresh_window_visibility();
                ShellOutcome::CONTINUE
            }
            ShellEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                ShellOutcome::CONTINUE
            }
            ShellEvent::PointerMoved {
                position,
                device_id,
            } => {
                self.last_cursor_position = Some(position);
                self.dispatch(RuntimeEvent::Input(RawEvent::PointerMove(RawPointerMove {
                    position,
                    device_id,
                    pointer_id: XuiPointerId::new(0),
                    kind: PointerKind::Mouse,
                    button: None,
                    buttons: self.pointer_buttons,
                    modifiers: self.modifiers,
                    timestamp,
                })))
            }
            ShellEvent::PointerButton {
                button,
                pressed,
                device_id,
            } => {
                self.pointer_buttons.set(button, pressed);
                let raw = RawPointerButton {
                    position: self.pointer_position(),
                    pointer_id: XuiPointerId::new(0),
                    device_id,
                    kind: PointerKind::Mouse,
                    button,
                    buttons: self.pointer_buttons,
                    modifiers: self.modifiers,
                    timestamp,
                };
                self.dispatch(RuntimeEvent::Input(if pressed {
                    RawEvent::PointerDown(raw)
                } else {
                    RawEvent::PointerUp(raw)
                }))
            }
            ShellEvent::Wheel {
                delta,
                device_id,
                is_inertial,
            } => self.dispatch(RuntimeEvent::Input(RawEvent::Wheel(RawWheel {
                position: self.pointer_position(),
                delta,
                device_id,
                pointer_id: Some(XuiPointerId::new(0)),
                modifiers: self.modifiers,
                timestamp,
                is_inertial,
            }))),
            ShellEvent::Keyboard(key) => {
                self.dispatch(RuntimeEvent::Input(RawEvent::Keyboard(RawKeyboard {
                    physical_key: key.physical_key,
                    named_key: key.named_key,
                    state: key.state,
                    text: key.text,
                    modifiers: self.modifiers,
                    timestamp,
                    is_repeat: key.is_repeat,
                })))
            }
            ShellEvent::Ime(ime) => self.dispatch(RuntimeEvent::Input(RawEvent::Ime(ime))),
            ShellEvent::RedrawRequested => ShellOutcome::from_control(self.render()),
            ShellEvent::Wake => {
                self.runtime.app_mut().drain_async_messages();
                self.settle();
                ShellOutcome::CONTINUE
            }

            ShellEvent::LiveResize(live) => {
                self.live_resize = live;
                ShellOutcome::CONTINUE
            }
            ShellEvent::FrameTick(info) => {
                ShellOutcome::from_control(self.render_at(info.target.unwrap_or_else(Instant::now)))
            }
            ShellEvent::PointerLeft => {
                let position = self.last_cursor_position.take();
                self.dispatch(RuntimeEvent::Input(RawEvent::PointerLeave(
                    RawPointerLeave {
                        pointer_id: XuiPointerId::new(0),
                        device_id: None,
                        kind: PointerKind::Mouse,
                        position,
                        modifiers: self.modifiers,
                        timestamp,
                    },
                )))
            }
            ShellEvent::PointerCancelled => {
                self.pointer_buttons = PointerButtons::default();
                self.dispatch(RuntimeEvent::Input(RawEvent::PointerCancel(
                    RawPointerCancel {
                        pointer_id: XuiPointerId::new(0),
                        device_id: None,
                        kind: PointerKind::Mouse,
                        position: self.last_cursor_position,
                        modifiers: self.modifiers,
                        timestamp,
                    },
                )))
            }
            ShellEvent::ContextMenuRequested { position, trigger } => {
                self.dispatch(RuntimeEvent::Input(RawEvent::ContextMenu(RawContextMenu {
                    trigger,
                    pointer: None,
                    position,
                    modifiers: self.modifiers,
                    timestamp,
                })))
            }
            ShellEvent::Touch {
                id,
                phase,
                position,
                device_id,
            } => {
                let mut outcome = ShellOutcome::CONTINUE;
                for raw in touch_events(id, phase, position, device_id, self.modifiers, timestamp) {
                    let step = self.dispatch(RuntimeEvent::Input(raw));
                    outcome.consumed |= step.consumed;
                    outcome.control = step.control;
                }
                outcome
            }
            ShellEvent::Gesture {
                kind,
                phase,
                position,
            } => self.dispatch(RuntimeEvent::Input(RawEvent::Gesture(RawGesture {
                kind,
                phase,
                position,
                modifiers: self.modifiers,
                timestamp,
            }))),
            ShellEvent::Command(command) => {
                self.dispatch(RuntimeEvent::Command { command, timestamp })
            }
            ShellEvent::Reply(id, reply) => self.dispatch(RuntimeEvent::Reply(id, reply)),
            ShellEvent::AppearanceChanged(appearance) => {
                self.dispatch(RuntimeEvent::Appearance(appearance))
            }
            ShellEvent::WindowStateChanged(state) => {
                self.dispatch(RuntimeEvent::WindowState(state))
            }
            ShellEvent::Drag(drag) => self.dispatch(RuntimeEvent::Drag(drag)),
        }
    }

    /// Call when the event loop has drained its queue and is about to block.
    pub fn about_to_wait(&mut self) -> ShellControl {
        match self.runtime.control_flow() {
            ControlFlow::Exit => ShellControl::Exit,
            ControlFlow::Poll => {
                self.request_redraw_if_dirty();
                ShellControl::Continue
            }
            ControlFlow::Wait => ShellControl::Continue,
        }
    }

    fn dispatch(&mut self, event: RuntimeEvent) -> ShellOutcome {
        match event {
            RuntimeEvent::RedrawRequested => ShellOutcome::from_control(self.render()),
            RuntimeEvent::Exit if self.options.exit_on_close_requested => ShellOutcome {
                control: ShellControl::Exit,
                consumed: true,
            },
            other => {
                let consumed = self
                    .runtime
                    .handle_event(other)
                    .iter()
                    .any(|result| result.is_consumed());
                self.settle();
                ShellOutcome {
                    control: ShellControl::Continue,
                    consumed,
                }
            }
        }
    }

    fn render(&mut self) -> ShellControl {
        self.render_at(Instant::now())
    }

    fn render_at(&mut self, now: Instant) -> ShellControl {
        match self.runtime.frame_at(now) {
            Err(error) => {
                self.render_error = Some(error);
                ShellControl::Exit
            }
            Ok(_) => {
                // Unconditionally, not only when the frame drew something: an
                // app whose first frame has nothing to draw still gets its
                // window, rather than staying hidden forever.
                self.show_window_once_painted();
                self.settle();
                ShellControl::Continue
            }
        }
    }

    /// Everything that follows a step of the runtime: the host is handed what
    /// the application asked of it, the state the runtime wants shown is applied,
    /// and a frame is requested if one is owed.
    ///
    /// In that order, because answering a request can change the state.
    fn settle(&mut self) {
        self.flush_platform_commands();
        self.sync_platform_output();
        self.request_redraw_if_dirty();
    }

    fn set_scale_factor(&mut self, scale_factor: f64) {
        let factor = scale_factor as f32;
        let _ = self.runtime.backend_mut().set_factor(factor);
        self.runtime.text_backend_mut().set_scale_factor(factor);
        self.runtime.app_mut().set_scale_factor(factor);
        let size = self.window.surface_size().to_logical(scale_factor);
        self.runtime.handle_event(RuntimeEvent::Resize(size));
        self.runtime.app_mut().mark_needs_rebuild();
        self.request_redraw_if_dirty();
    }

    fn pointer_position(&self) -> Point {
        self.last_cursor_position.unwrap_or(Point::new(0.0, 0.0))
    }

    /// Reveals the window once its first frame has been through the backend.
    fn show_window_once_painted(&mut self) {
        if !self.pending_first_present {
            return;
        }
        self.pending_first_present = false;
        self.window.set_visible(true);
        // A window ordered in after creation is not made key on its own.
        self.window.focus();
    }

    /// Hands the host what the application has asked of it, and delivers the
    /// answers.
    ///
    /// Answers go through the runtime's event route -- never from inside
    /// [`PlatformWindow::perform`] -- because the runtime is in the middle of
    /// handing over its command list at that point and must not be re-entered.
    fn flush_platform_commands(&mut self) {
        for _ in 0..MAX_COMMAND_ROUNDS {
            let commands = self.runtime.take_platform_commands();
            if commands.is_empty() {
                return;
            }

            let mut replies = Vec::new();
            for command in commands {
                let request = match &command {
                    PlatformCommand::Request(id, _) => Some(*id),
                    _ => None,
                };
                let label = command_label(&command);
                match (self.window.perform(command), request) {
                    (Performed::Done, None) | (Performed::Pending, Some(_)) => {}
                    (Performed::Done, Some(id)) => {
                        log::error!("host completed a {label} without replying to it");
                        replies.push((
                            id,
                            PlatformReply::Failed(PlatformError::new(
                                "the host completed the request without replying",
                            )),
                        ));
                    }
                    (Performed::Unsupported, None) => {
                        log::warn!("the host does not implement {label}");
                    }
                    (Performed::Unsupported, Some(id)) => {
                        replies.push((id, PlatformReply::Unsupported));
                    }
                    (Performed::Reply(reply), Some(id)) => replies.push((id, reply)),
                    (Performed::Reply(_), None) | (Performed::Pending, None) => {
                        log::error!("host answered {label}, which asked nothing");
                    }
                    (Performed::Failed(error), None) => {
                        log::warn!("{label} failed: {error}");
                    }
                    (Performed::Failed(error), Some(id)) => {
                        log::warn!("{label} failed: {error}");
                        replies.push((id, PlatformReply::Failed(error)));
                    }
                }
            }

            for (id, reply) in replies {
                self.runtime.handle_event(RuntimeEvent::Reply(id, reply));
            }
        }
        log::warn!(
            "platform commands were still being produced after {MAX_COMMAND_ROUNDS} rounds; \
             the rest wait for the next event"
        );
    }

    fn sync_platform_output(&mut self) {
        let next = self.runtime.platform_output().clone();

        if self.last_platform_output.text_input.is_some() != next.text_input.is_some() {
            self.window.set_ime_allowed(next.text_input.is_some());
        }

        let previous_area = self
            .last_platform_output
            .text_input
            .as_ref()
            .map(|session| session.cursor_area);
        let next_area = next.text_input.as_ref().map(|session| session.cursor_area);
        if previous_area != next_area
            && let Some(area) = next_area
        {
            self.window.set_ime_cursor_area(area);
        }

        if self.last_platform_output.cursor != next.cursor {
            self.window.set_cursor(next.cursor);
        }

        // A title the runtime stops asking for is left as it was: `None` means
        // "no opinion", not "clear it".
        if self.last_platform_output.title != next.title
            && let Some(title) = &next.title
        {
            self.window.set_title(title);
        }

        if self.last_platform_output.menu_bar != next.menu_bar {
            self.window.set_menu_bar(next.menu_bar.as_deref());
        }

        self.published.set(&next.surface);
        self.last_platform_output = next;
    }

    /// Tells the runtime whether the window can be seen, from whatever the
    /// platform is willing to say about it.
    ///
    /// No single event covers this. macOS and Wayland report occlusion; a
    /// minimized window on Windows and X11 instead reports a resize to nothing
    /// and answers `is_minimized`. So this asks all three and takes the
    /// pessimistic answer.
    ///
    /// Every path is best-effort. A platform that reports nothing leaves the
    /// window permanently visible.
    fn refresh_window_visibility(&mut self) {
        let size = self.window.surface_size();
        let visible = !self.occluded
            && !self.window.is_minimized().unwrap_or(false)
            && size.width > 0
            && size.height > 0;

        if self.runtime.window_visible() == visible {
            return;
        }
        self.runtime.set_window_visible(visible);
        // Coming back needs a frame to restart the loop from: the animations
        // that wanted one are still recorded, but nothing has asked since.
        self.request_redraw_if_dirty();
    }

    fn request_redraw_if_dirty(&self) {
        if self.runtime.app().is_dirty() {
            self.window.request_redraw();
        }
    }
}

/// What a command is, for the log. Never its contents: a clipboard write may be
/// large, and may be private.
fn command_label(command: &PlatformCommand) -> &'static str {
    match command {
        PlatformCommand::Window(_) => "a window command",
        PlatformCommand::WriteClipboard { .. } => "a clipboard write",
        PlatformCommand::OpenUrl(_) => "opening a URL",
        PlatformCommand::RevealPath(_) => "revealing a path",
        PlatformCommand::ShowContextMenu { .. } => "a context menu",
        PlatformCommand::Request(..) => "a request",
        PlatformCommand::Cancel(_) => "a request cancellation",
        _ => "a platform command",
    }
}

/// The raw input one finger's state change amounts to.
///
/// Touch rides the pointer events a mouse does, as [`PointerKind::Touch`] with a
/// pointer id of its own, so a widget written for the pointer works under a
/// finger. Id 0 is the mouse; fingers start at 1. A finger landing is a move to
/// where it landed followed by a press, which is how a hover-then-press widget
/// expects to be reached.
fn touch_events(
    id: u64,
    phase: TouchPhase,
    position: Point,
    device_id: Option<XuiDeviceId>,
    modifiers: Modifiers,
    timestamp: Instant,
) -> Vec<RawEvent> {
    let pointer_id = XuiPointerId::new(id.saturating_add(1));
    let down = PointerButtons::from_button(PointerButton::Primary);
    let moved = |buttons| {
        RawEvent::PointerMove(RawPointerMove {
            position,
            pointer_id,
            device_id,
            kind: PointerKind::Touch,
            button: None,
            buttons,
            modifiers,
            timestamp,
        })
    };
    let button = |buttons| RawPointerButton {
        position,
        pointer_id,
        device_id,
        kind: PointerKind::Touch,
        button: PointerButton::Primary,
        buttons,
        modifiers,
        timestamp,
    };
    match phase {
        TouchPhase::Started => vec![
            moved(PointerButtons::default()),
            RawEvent::PointerDown(button(down)),
        ],
        TouchPhase::Moved => vec![moved(down)],
        TouchPhase::Ended => vec![RawEvent::PointerUp(button(PointerButtons::default()))],
        TouchPhase::Cancelled => vec![RawEvent::PointerCancel(RawPointerCancel {
            pointer_id,
            device_id,
            kind: PointerKind::Touch,
            position: Some(position),
            modifiers,
            timestamp,
        })],
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };
    use xui_core::prelude::*;
    use xui_core::render::MockRenderBackend;
    use xui_f::FBackend;

    use crate::{
        PhysicalSize, PlatformWindow, Shell, ShellControl, ShellEvent, ShellOptions, SurfaceTarget,
    };

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        RequestRedraw,
        SetVisible(bool),
        Focus,
        SetCursor(CursorIcon),
        SetImeAllowed(bool),
        SetImeCursorArea,
    }

    struct FakeState {
        size: PhysicalSize,
        scale_factor: f64,
        minimized: bool,
        calls: Vec<Call>,
    }

    struct FakeWindow(Mutex<FakeState>);

    impl FakeWindow {
        fn new(size: PhysicalSize, scale_factor: f64) -> Self {
            Self(Mutex::new(FakeState {
                size,
                scale_factor,
                minimized: false,
                calls: Vec::new(),
            }))
        }

        fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.0.lock().unwrap()
        }

        fn record(&self, call: Call) {
            self.state().calls.push(call);
        }

        fn calls(&self) -> Vec<Call> {
            self.state().calls.clone()
        }
    }

    impl HasWindowHandle for FakeWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }

    impl HasDisplayHandle for FakeWindow {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }

    impl SurfaceTarget for FakeWindow {
        fn surface_size(&self) -> PhysicalSize {
            self.state().size
        }

        fn scale_factor(&self) -> f64 {
            self.state().scale_factor
        }
    }

    impl PlatformWindow for FakeWindow {
        fn request_redraw(&self) {
            self.record(Call::RequestRedraw);
        }

        fn set_visible(&self, visible: bool) {
            self.record(Call::SetVisible(visible));
        }

        fn focus(&self) {
            self.record(Call::Focus);
        }

        fn is_minimized(&self) -> Option<bool> {
            Some(self.state().minimized)
        }

        fn set_cursor(&self, cursor: CursorIcon) {
            self.record(Call::SetCursor(cursor));
        }

        fn set_ime_allowed(&self, allowed: bool) {
            self.record(Call::SetImeAllowed(allowed));
        }

        fn set_ime_cursor_area(&self, _area: Rect) {
            self.record(Call::SetImeCursorArea);
        }
    }

    type TestShell = Shell<FakeWindow, MockRenderBackend, FBackend>;

    fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
        container().into_element_desc(Vec::new())
    }

    fn start(options: ShellOptions) -> (TestShell, Arc<FakeWindow>, ShellControl) {
        let window = Arc::new(FakeWindow::new(PhysicalSize::new(800, 600), 2.0));
        let (shell, control) = Shell::new(
            window.clone(),
            App::new(root),
            FBackend::new(),
            MockRenderBackend::default(),
            options,
        );
        (shell, window, control)
    }

    fn send(shell: &mut TestShell, event: ShellEvent) {
        assert_eq!(shell.handle_event(event), ShellControl::Continue);
    }

    #[test]
    fn first_frame_is_painted_at_logical_size_before_the_window_is_shown() {
        let (mut shell, window, control) = start(ShellOptions::default());
        assert_eq!(control, ShellControl::Continue);
        assert_eq!(
            shell.runtime_mut().backend_mut().frame_size,
            Some(Size::new(400.0, 300.0))
        );
        let calls = window.calls();
        let shown = calls
            .iter()
            .position(|call| *call == Call::SetVisible(true));
        let focused = calls.iter().position(|call| *call == Call::Focus);
        assert!(shown.is_some() && shown < focused, "calls: {calls:?}");
    }

    #[test]
    fn a_window_asked_to_stay_hidden_is_never_shown() {
        let (_shell, window, _) = start(ShellOptions {
            visible: false,
            ..ShellOptions::default()
        });
        assert!(!window.calls().contains(&Call::SetVisible(true)));
    }

    #[test]
    fn visibility_follows_occlusion_minimize_and_zero_size() {
        let (mut shell, window, _) = start(ShellOptions::default());
        assert!(shell.runtime().window_visible());

        send(&mut shell, ShellEvent::Occluded(true));
        assert!(!shell.runtime().window_visible());
        send(&mut shell, ShellEvent::Occluded(false));
        assert!(shell.runtime().window_visible());

        window.state().minimized = true;
        send(&mut shell, ShellEvent::Focused(false));
        assert!(!shell.runtime().window_visible());
        window.state().minimized = false;
        send(&mut shell, ShellEvent::Focused(true));
        assert!(shell.runtime().window_visible());

        window.state().size = PhysicalSize::new(0, 0);
        send(&mut shell, ShellEvent::Resized(PhysicalSize::new(0, 0)));
        assert!(!shell.runtime().window_visible());
    }

    #[test]
    fn close_request_exits_only_when_configured_to() {
        let (mut shell, _, _) = start(ShellOptions::default());
        assert_eq!(
            shell.handle_event(ShellEvent::CloseRequested),
            ShellControl::Exit
        );

        let (mut shell, _, _) = start(ShellOptions {
            exit_on_close_requested: false,
            ..ShellOptions::default()
        });
        send(&mut shell, ShellEvent::CloseRequested);
    }

    #[test]
    fn scale_factor_change_resizes_to_the_new_logical_size() {
        let (mut shell, window, _) = start(ShellOptions::default());
        window.state().scale_factor = 1.0;
        send(&mut shell, ShellEvent::ScaleFactorChanged(1.0));
        send(&mut shell, ShellEvent::RedrawRequested);
        assert_eq!(
            shell.runtime_mut().backend_mut().frame_size,
            Some(Size::new(800.0, 600.0))
        );
    }
}
