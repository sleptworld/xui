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

use slotmap::SecondaryMap;
use xui_interface::{MenuBar, PhysicalSize, WindowId, WindowOptions};

use crate::published::PublishedHandle;
use crate::window::{Performed, PlatformWindow, SurfaceBackend, SurfaceTarget, WindowOpener};

/// Whether `command` acts on the window it is sent for, as opposed to the app.
fn targets_window(command: &PlatformCommand) -> bool {
    matches!(
        command,
        PlatformCommand::Window(_) | PlatformCommand::ShowContextMenu { .. }
    )
}

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
/// The host owns the event loop and the windows. It hands each translated
/// event to [`Shell::dispatch_event`] (the main window's) or
/// [`Shell::dispatch_window_event`], calls [`Shell::about_to_wait`] when the
/// loop goes idle, and acts on the [`ShellOutcome`] each returns. Everything
/// that does not depend on the platform happens here.
///
/// The main window is the one the shell starts on. Any other is opened when
/// the application mounts a `window(..) { .. }` block, through the
/// [`WindowOpener`] the host gave [`Shell::with_opener`]; a host that gave
/// none cannot open one, and such a window stays off screen.
///
/// A host whose native callbacks can arrive while it is inside the shell -- all
/// of them can -- should hold a [`crate::ShellHandle`] instead, which queues
/// those events rather than requiring the host to.
pub struct Shell<W, B, T>
where
    W: PlatformWindow,
    B: RenderBackend<TextHost<T>> + SurfaceBackend,
    T: TextBackend,
{
    runtime: GuiRuntime<B, T>,
    options: ShellOptions,
    main: WindowId,
    windows: SecondaryMap<WindowId, WindowShell<W>>,
    opener: Option<Box<dyn WindowOpener<W>>>,
    /// Windows asked for while there was no opener, in order, for one given
    /// later: a window block mounted with the root asks during [`Self::new`].
    unopened: Vec<(WindowId, WindowOptions)>,
    /// The application's menu bar as last applied, which is the app's rather
    /// than any one window's.
    last_menu_bar: Option<Arc<MenuBar>>,
    render_error: Option<AppRenderError<B::Error>>,
}

/// What the shell keeps of one native window.
struct WindowShell<W> {
    native: Arc<W>,
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
    /// The deadline last handed to [`PlatformWindow::request_redraw_at`], so a
    /// settle that finds the same one does not ask again. Forgotten at every
    /// frame: a timed frame that came before its deadline leaves the deadline
    /// pending, and it has to be asked for again rather than assumed.
    timed_redraw: Option<Instant>,
}

impl<W> WindowShell<W> {
    fn new(native: Arc<W>, visible: bool, published: PublishedHandle) -> Self {
        Self {
            native,
            last_cursor_position: None,
            modifiers: Modifiers::default(),
            pointer_buttons: PointerButtons::default(),
            last_platform_output: PlatformOutput::default(),
            published,
            live_resize: false,
            occluded: false,
            pending_first_present: visible,
            timed_redraw: None,
        }
    }

    fn pointer_position(&self) -> Point {
        self.last_cursor_position.unwrap_or(Point::new(0.0, 0.0))
    }
}

impl<W, B, T> Shell<W, B, T>
where
    W: PlatformWindow,
    B: RenderBackend<TextHost<T>> + SurfaceBackend,
    T: TextBackend,
{
    /// Starts the runtime on `window`, the main window, and paints the first
    /// frame.
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
        let mut backend = backend;
        backend.bind_primary(app.main_window());
        let mut runtime = GuiRuntime::new(app, backend, text);
        let main = runtime.app().main_window();
        // Before the first frame, so the application's first build already
        // knows what the host can do and what the system looks like.
        runtime.handle_event(RuntimeEvent::Capabilities(window.capabilities()));
        runtime.handle_event(RuntimeEvent::Appearance(window.appearance()));
        let mut windows = SecondaryMap::new();
        windows.insert(
            main,
            WindowShell::new(window, options.visible, PublishedHandle::default()),
        );
        let mut shell = Self {
            runtime,
            options,
            main,
            windows,
            opener: None,
            unopened: Vec::new(),
            last_menu_bar: None,
            render_error: None,
        };
        shell.size_window(main);
        // Paint before handing control back to the event loop: a hidden
        // window is not guaranteed to be sent a redraw, and this is the frame
        // that decides when it becomes visible.
        let control = shell.render(main);
        (shell, control)
    }

    /// Lets the shell open the windows the application asks for beyond the
    /// main one, if the backend can draw them. Without an opener, or with a
    /// backend that draws one window only, such a window is never made, and
    /// its `window::Visible` stays false.
    pub fn with_opener(mut self, opener: impl WindowOpener<W> + 'static) -> Self {
        self.opener = Some(Box::new(opener));
        if self.runtime.backend().supports_surfaces() {
            let capabilities = self.windows[self.main].native.capabilities()
                | xui_interface::HostCapabilities::MULTI_WINDOW;
            self.runtime
                .handle_event(RuntimeEvent::Capabilities(capabilities));
        }
        // A window block mounted with the root asked before there was one.
        for (window, options) in std::mem::take(&mut self.unopened) {
            if let Performed::Failed(error) = self.open_window(window, options) {
                log::warn!("opening a window failed: {error}");
            }
        }
        self.settle();
        self
    }

    /// The main window.
    pub fn window(&self) -> &Arc<W> {
        &self.windows[self.main].native
    }

    /// The main window's id.
    pub fn main_window(&self) -> WindowId {
        self.main
    }

    /// Window `window`'s native window, while the shell has it open.
    pub fn native_window(&self, window: WindowId) -> Option<&Arc<W>> {
        self.windows.get(window).map(|state| &state.native)
    }

    pub fn runtime(&self) -> &GuiRuntime<B, T> {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut GuiRuntime<B, T> {
        &mut self.runtime
    }

    /// What the runtime has published about the main window's surface. The
    /// handle stays current as the shell runs, and can be read without
    /// borrowing it.
    pub fn published(&self) -> PublishedHandle {
        self.windows[self.main].published.clone()
    }

    /// [`Self::published`] for window `window`, while the shell has it open.
    pub fn published_of(&self, window: WindowId) -> Option<PublishedHandle> {
        self.windows
            .get(window)
            .map(|state| state.published.clone())
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

    /// Handles `event`, which happened at `at`, as the main window's.
    ///
    /// `at` is when the event happened natively, not when it was dequeued: an
    /// event that waited behind another keeps its own time, which is what
    /// double-click spacing and scroll velocity are measured in.
    pub fn dispatch_event(&mut self, at: Instant, event: ShellEvent) -> ShellOutcome {
        self.dispatch_window_event(self.main, at, event)
    }

    /// Handles `event`, which happened at `at`, as window `window`'s. An event
    /// for a window the shell does not have open is dropped.
    pub fn dispatch_window_event(
        &mut self,
        window: WindowId,
        at: Instant,
        event: ShellEvent,
    ) -> ShellOutcome {
        let Some(state) = self.windows.get_mut(window) else {
            return ShellOutcome::CONTINUE;
        };
        let timestamp = at;
        let modifiers = state.modifiers;
        match event {
            ShellEvent::Resized(size) => {
                let size = size.to_logical(state.native.scale_factor());
                let outcome = if state.live_resize {
                    // The platform is showing the new geometry now. Painting
                    // here, inside the resize, is what keeps the picture and
                    // the window edge together.
                    self.runtime
                        .handle_window_event(window, RuntimeEvent::Resize(size));
                    ShellOutcome::from_control(self.render(window))
                } else {
                    self.dispatch(window, RuntimeEvent::Resize(size))
                };
                // A minimized window reports no occlusion on Windows or X11;
                // what it reports is a resize to nothing.
                self.refresh_window_visibility(window);
                outcome
            }
            ShellEvent::ScaleFactorChanged(scale_factor) => {
                self.set_scale_factor(window, scale_factor);
                ShellOutcome::CONTINUE
            }
            ShellEvent::CloseRequested if window == self.main => {
                self.dispatch(window, RuntimeEvent::Exit)
            }
            ShellEvent::CloseRequested => {
                // The application's to decide: its window block closes it, or
                // keeps it hidden.
                self.runtime.app_mut().request_close(window);
                self.settle();
                ShellOutcome {
                    control: ShellControl::Continue,
                    consumed: true,
                }
            }
            ShellEvent::Focused(focused) => {
                let raw = RawWindowEvent {
                    timestamp,
                    modifiers,
                };
                let outcome = self.dispatch(
                    window,
                    RuntimeEvent::Input(if focused {
                        RawEvent::WindowFocus(raw)
                    } else {
                        RawEvent::WindowBlur(raw)
                    }),
                );
                // Not itself a visibility signal, but a reliable moment at
                // which one may have arrived unannounced.
                self.refresh_window_visibility(window);
                outcome
            }
            ShellEvent::Occluded(occluded) => {
                state.occluded = occluded;
                self.refresh_window_visibility(window);
                ShellOutcome::CONTINUE
            }
            ShellEvent::ModifiersChanged(modifiers) => {
                state.modifiers = modifiers;
                ShellOutcome::CONTINUE
            }
            ShellEvent::PointerMoved {
                position,
                device_id,
            } => {
                state.last_cursor_position = Some(position);
                let buttons = state.pointer_buttons;
                self.dispatch(
                    window,
                    RuntimeEvent::Input(RawEvent::PointerMove(RawPointerMove {
                        position,
                        device_id,
                        pointer_id: XuiPointerId::new(0),
                        kind: PointerKind::Mouse,
                        button: None,
                        buttons,
                        modifiers,
                        timestamp,
                    })),
                )
            }
            ShellEvent::PointerButton {
                button,
                pressed,
                device_id,
            } => {
                state.pointer_buttons.set(button, pressed);
                let raw = RawPointerButton {
                    position: state.pointer_position(),
                    pointer_id: XuiPointerId::new(0),
                    device_id,
                    kind: PointerKind::Mouse,
                    button,
                    buttons: state.pointer_buttons,
                    modifiers,
                    timestamp,
                };
                self.dispatch(
                    window,
                    RuntimeEvent::Input(if pressed {
                        RawEvent::PointerDown(raw)
                    } else {
                        RawEvent::PointerUp(raw)
                    }),
                )
            }
            ShellEvent::Wheel {
                delta,
                device_id,
                is_inertial,
            } => {
                let position = state.pointer_position();
                self.dispatch(
                    window,
                    RuntimeEvent::Input(RawEvent::Wheel(RawWheel {
                        position,
                        delta,
                        device_id,
                        pointer_id: Some(XuiPointerId::new(0)),
                        modifiers,
                        timestamp,
                        is_inertial,
                    })),
                )
            }
            ShellEvent::Keyboard(key) => self.dispatch(
                window,
                RuntimeEvent::Input(RawEvent::Keyboard(RawKeyboard {
                    physical_key: key.physical_key,
                    named_key: key.named_key,
                    state: key.state,
                    text: key.text,
                    modifiers,
                    timestamp,
                    is_repeat: key.is_repeat,
                })),
            ),
            ShellEvent::Ime(ime) => self.dispatch(window, RuntimeEvent::Input(RawEvent::Ime(ime))),
            ShellEvent::RedrawRequested => ShellOutcome::from_control(self.render(window)),
            ShellEvent::Wake => {
                self.runtime.app_mut().drain_async_messages();
                self.settle();
                ShellOutcome::CONTINUE
            }

            ShellEvent::LiveResize(live) => {
                state.live_resize = live;
                ShellOutcome::CONTINUE
            }
            ShellEvent::FrameTick(info) => ShellOutcome::from_control(
                self.render_at(window, info.target.unwrap_or_else(Instant::now)),
            ),
            ShellEvent::PointerLeft => {
                let position = state.last_cursor_position.take();
                self.dispatch(
                    window,
                    RuntimeEvent::Input(RawEvent::PointerLeave(RawPointerLeave {
                        pointer_id: XuiPointerId::new(0),
                        device_id: None,
                        kind: PointerKind::Mouse,
                        position,
                        modifiers,
                        timestamp,
                    })),
                )
            }
            ShellEvent::PointerCancelled => {
                state.pointer_buttons = PointerButtons::default();
                let position = state.last_cursor_position;
                self.dispatch(
                    window,
                    RuntimeEvent::Input(RawEvent::PointerCancel(RawPointerCancel {
                        pointer_id: XuiPointerId::new(0),
                        device_id: None,
                        kind: PointerKind::Mouse,
                        position,
                        modifiers,
                        timestamp,
                    })),
                )
            }
            ShellEvent::ContextMenuRequested { position, trigger } => self.dispatch(
                window,
                RuntimeEvent::Input(RawEvent::ContextMenu(RawContextMenu {
                    trigger,
                    pointer: None,
                    position,
                    modifiers,
                    timestamp,
                })),
            ),
            ShellEvent::Touch {
                id,
                phase,
                position,
                device_id,
            } => {
                let mut outcome = ShellOutcome::CONTINUE;
                for raw in touch_events(id, phase, position, device_id, modifiers, timestamp) {
                    let step = self.dispatch(window, RuntimeEvent::Input(raw));
                    outcome.consumed |= step.consumed;
                    outcome.control = step.control;
                }
                outcome
            }
            ShellEvent::Gesture {
                kind,
                phase,
                position,
            } => self.dispatch(
                window,
                RuntimeEvent::Input(RawEvent::Gesture(RawGesture {
                    kind,
                    phase,
                    position,
                    modifiers,
                    timestamp,
                })),
            ),
            ShellEvent::Command(command) => {
                self.dispatch(window, RuntimeEvent::Command { command, timestamp })
            }
            ShellEvent::Reply(id, reply) => self.dispatch(window, RuntimeEvent::Reply(id, reply)),
            ShellEvent::AppearanceChanged(appearance) => {
                self.dispatch(window, RuntimeEvent::Appearance(appearance))
            }
            ShellEvent::WindowStateChanged(state) => {
                self.dispatch(window, RuntimeEvent::WindowState(state))
            }
            ShellEvent::Drag(drag) => self.dispatch(window, RuntimeEvent::Drag(drag)),
        }
    }

    /// Call when the event loop has drained its queue and is about to block.
    pub fn about_to_wait(&mut self) -> ShellControl {
        match self.runtime.control_flow() {
            ControlFlow::Exit => ShellControl::Exit,
            ControlFlow::Poll => {
                self.request_redraws();
                ShellControl::Continue
            }
            ControlFlow::Wait => ShellControl::Continue,
        }
    }

    fn dispatch(&mut self, window: WindowId, event: RuntimeEvent) -> ShellOutcome {
        match event {
            RuntimeEvent::RedrawRequested => ShellOutcome::from_control(self.render(window)),
            RuntimeEvent::Exit if self.options.exit_on_close_requested => ShellOutcome {
                control: ShellControl::Exit,
                consumed: true,
            },
            other => {
                let consumed = self
                    .runtime
                    .handle_window_event(window, other)
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

    fn render(&mut self, window: WindowId) -> ShellControl {
        self.render_at(window, Instant::now())
    }

    fn render_at(&mut self, window: WindowId, now: Instant) -> ShellControl {
        let Some(state) = self.windows.get_mut(window) else {
            return ShellControl::Continue;
        };
        state.timed_redraw = None;
        match self.runtime.frame_window_at(window, now) {
            Err(error) => {
                self.render_error = Some(error);
                ShellControl::Exit
            }
            Ok(_) => {
                // Unconditionally, not only when the frame drew something: an
                // app whose first frame has nothing to draw still gets its
                // window, rather than staying hidden forever.
                self.show_window_once_painted(window);
                self.settle();
                ShellControl::Continue
            }
        }
    }

    /// Everything that follows a step of the runtime: the host is handed what
    /// the application asked of it, the state the runtime wants shown is applied,
    /// and a frame is requested of every window that owes one.
    ///
    /// In that order, because answering a request can change the state.
    fn settle(&mut self) {
        self.flush_platform_commands();
        self.sync_platform_output();
        self.request_redraws();
    }

    /// Tells the runtime window `window`'s size and scale factor, as its
    /// native window has them now.
    fn size_window(&mut self, window: WindowId) {
        let native = &self.windows[window].native;
        let scale_factor = native.scale_factor();
        let size = native.surface_size().to_logical(scale_factor);
        self.runtime
            .app_mut()
            .set_scale_factor(window, scale_factor as f32);
        self.runtime
            .handle_window_event(window, RuntimeEvent::Resize(size));
    }

    fn set_scale_factor(&mut self, window: WindowId, scale_factor: f64) {
        let factor = scale_factor as f32;
        let _ = self.runtime.backend_mut().set_factor(window, factor);
        self.size_window(window);
        self.runtime.app_mut().mark_needs_rebuild(window);
        self.request_redraw_if_dirty(window);
    }

    /// Reveals the window once its first frame has been through the backend,
    /// and tells the runtime it is on screen.
    fn show_window_once_painted(&mut self, window: WindowId) {
        let state = &mut self.windows[window];
        if !state.pending_first_present {
            return;
        }
        state.pending_first_present = false;
        state.native.set_visible(true);
        // A window ordered in after creation is not made key on its own.
        state.native.focus();
        self.refresh_window_visibility(window);
    }

    /// Opens the window the application asked for, if the host can, and
    /// paints its first frame, which shows it.
    fn open_window(&mut self, window: WindowId, options: WindowOptions) -> Performed {
        let Some(opener) = self.opener.as_mut() else {
            self.unopened.push((window, options));
            return Performed::Unsupported;
        };
        if !self.runtime.backend().supports_surfaces() {
            return Performed::Unsupported;
        }
        let published = PublishedHandle::default();
        let native = match opener.open(window, &options, &published) {
            Ok(native) => native,
            Err(error) => return Performed::Failed(error),
        };
        let surface: Arc<dyn SurfaceTarget> = native.clone();
        if let Err(error) = self.runtime.backend_mut().attach_surface(window, surface) {
            opener.close(window, &native);
            return Performed::Failed(error);
        }
        self.runtime.attach(window);
        let scale_factor = native.scale_factor() as f32;
        self.windows
            .insert(window, WindowShell::new(native, options.visible, published));
        self.size_window(window);
        let _ = self.runtime.backend_mut().set_factor(window, scale_factor);
        // As for the main window: painted before it is shown.
        let _ = self.render(window);
        Performed::Done
    }

    /// Destroys the native window the application closed.
    fn close_window(&mut self, window: WindowId) -> Performed {
        let Some(state) = self.windows.remove(window) else {
            // Never opened: the host could not, or not yet.
            self.unopened.retain(|(waiting, _)| *waiting != window);
            return Performed::Done;
        };
        if let Some(opener) = self.opener.as_mut() {
            opener.close(window, &state.native);
        }
        Performed::Done
    }

    /// Hands the host what the application has asked of it, and delivers the
    /// answers.
    ///
    /// Answers go through the runtime's event route -- never from inside
    /// [`PlatformWindow::perform`] -- because the runtime is in the middle of
    /// handing over its command list at that point and must not be re-entered.
    fn flush_platform_commands(&mut self) {
        for _ in 0..MAX_COMMAND_ROUNDS {
            let commands = self.runtime.take_window_commands();
            if commands.is_empty() {
                return;
            }

            let mut replies = Vec::new();
            for (sender, command) in commands {
                let request = match &command {
                    PlatformCommand::Request(id, _) => Some(*id),
                    _ => None,
                };
                let label = command_label(&command);
                let performed = match command {
                    PlatformCommand::OpenWindow { window, options } => {
                        self.open_window(window, options)
                    }
                    PlatformCommand::CloseWindow(window) => self.close_window(window),
                    command => {
                        // A window's command is carried out on that window; the
                        // app's on whichever window sent it, or the main one.
                        let target = if self.windows.contains_key(sender) {
                            sender
                        } else if targets_window(&command) {
                            log::warn!("{label} for a window the host has not opened; dropped");
                            continue;
                        } else {
                            self.main
                        };
                        self.windows[target].native.perform(command)
                    }
                };
                match (performed, request) {
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
                self.runtime
                    .handle_window_event(self.main, RuntimeEvent::Reply(id, reply));
            }
        }
        log::warn!(
            "platform commands were still being produced after {MAX_COMMAND_ROUNDS} rounds; \
             the rest wait for the next event"
        );
    }

    fn sync_platform_output(&mut self) {
        let Self {
            runtime, windows, ..
        } = self;
        for (window, state) in windows.iter_mut() {
            if let Some(next) = runtime.window_platform_output(window) {
                sync_window_output(state, next);
            }
        }
        // The app's, applied once, to the main window, which is the one a
        // host with a single menu bar has it on.
        let menu_bar = &self.runtime.platform_output().menu_bar;
        if self.last_menu_bar != *menu_bar {
            self.windows[self.main]
                .native
                .set_menu_bar(menu_bar.as_deref());
            self.last_menu_bar = menu_bar.clone();
        }
    }

    /// Tells the runtime whether the window can be seen, from whatever the
    /// platform is willing to say about it.
    ///
    /// No single event covers this. macOS and Wayland report occlusion; a
    /// minimized window on Windows and X11 instead reports a resize to nothing
    /// and answers `is_minimized`. So this asks all three and takes the
    /// pessimistic answer. A window still waiting for its first frame is not
    /// on screen either.
    ///
    /// Every path is best-effort. A platform that reports nothing leaves the
    /// window permanently visible.
    fn refresh_window_visibility(&mut self, window: WindowId) {
        let Some(state) = self.windows.get(window) else {
            return;
        };
        let size = state.native.surface_size();
        let minimized = state.native.is_minimized();
        let held_back = state.pending_first_present;
        let occluded = state.occluded;
        if let Some(minimized) = minimized {
            self.runtime.set_minimized(window, minimized);
        }
        // The main window counts as on screen while it waits for its first
        // frame, as it always has: that frame is what shows it.
        let visible = !occluded
            && !minimized.unwrap_or(false)
            && size.width > 0
            && size.height > 0
            && (window == self.main || !held_back);

        if self.runtime.app().window_visible(window) == visible {
            return;
        }
        self.runtime.set_visible(window, visible);
        // Coming back needs a frame to restart the loop from: the animations
        // that wanted one are still recorded, but nothing has asked since.
        self.request_redraw_if_dirty(window);
    }

    /// Asks every window that owes a frame for one.
    ///
    /// Every window, not only the one an event came to: a signal written in
    /// one may be read in any other.
    fn request_redraws(&mut self) {
        let Self {
            runtime, windows, ..
        } = self;
        for (window, state) in windows.iter_mut() {
            request_redraw_if_dirty(runtime.app(), window, state);
        }
    }

    /// Asks for a frame of `window` now if one is owed, and otherwise for one
    /// at the next deadline anything in it is waiting on.
    fn request_redraw_if_dirty(&mut self, window: WindowId) {
        if let Some(state) = self.windows.get_mut(window) {
            request_redraw_if_dirty(self.runtime.app(), window, state);
        }
    }
}

/// Applies to `state`'s native window what changed in its output since the
/// last sync. Nothing is read or copied when nothing changed, which is the
/// usual case for every window but the one an event came to.
fn sync_window_output<W: PlatformWindow>(state: &mut WindowShell<W>, next: &PlatformOutput) {
    let last = &state.last_platform_output;
    if last == next {
        return;
    }
    let native = &state.native;

    if last.text_input.is_some() != next.text_input.is_some() {
        native.set_ime_allowed(next.text_input.is_some());
    }

    let previous_area = last.text_input.as_ref().map(|session| session.cursor_area);
    let next_area = next.text_input.as_ref().map(|session| session.cursor_area);
    if previous_area != next_area
        && let Some(area) = next_area
    {
        native.set_ime_cursor_area(area);
    }

    if last.cursor != next.cursor {
        native.set_cursor(next.cursor);
    }

    // A title the runtime stops asking for is left as it was: `None` means
    // "no opinion", not "clear it".
    if last.title != next.title
        && let Some(title) = &next.title
    {
        native.set_title(title);
    }

    if last.menu != next.menu {
        native.set_menu(&next.menu);
    }

    if last.surface != next.surface {
        state.published.set(&next.surface);
    }
    state.last_platform_output = next.clone();
}

/// Asks for a frame of `window` now if one is owed, and otherwise for one at
/// the next deadline anything in it is waiting on.
fn request_redraw_if_dirty<W: PlatformWindow>(
    app: &App,
    window: WindowId,
    state: &mut WindowShell<W>,
) {
    if app.window_is_dirty(window) {
        state.native.request_redraw();
    } else if let Some(at) = app.next_wake(window)
        && state.timed_redraw != Some(at)
    {
        state.timed_redraw = Some(at);
        state.native.request_redraw_at(at);
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
    use std::time::{Duration, Instant};

    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };
    use xui_core::prelude::*;
    use xui_core::render::MockRenderBackend;
    use xui_f::FBackend;

    use xui_interface::HostCapabilities;
    use xui_macros::component;

    use crate::{
        FrameInfo, PhysicalSize, PlatformWindow, Shell, ShellControl, ShellEvent, ShellOptions,
        SurfaceTarget,
    };

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        RequestRedraw,
        RequestRedrawAt(Instant),
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
        capabilities: HostCapabilities,
        calls: Vec<Call>,
    }

    struct FakeWindow(Mutex<FakeState>);

    impl FakeWindow {
        fn new(size: PhysicalSize, scale_factor: f64) -> Self {
            Self(Mutex::new(FakeState {
                size,
                scale_factor,
                minimized: false,
                capabilities: HostCapabilities::NONE,
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

        fn request_redraw_at(&self, at: Instant) {
            self.record(Call::RequestRedrawAt(at));
        }

        fn capabilities(&self) -> HostCapabilities {
            self.state().capabilities
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

    #[component]
    fn empty() {
        view! { container() }
    }

    fn root(app: &mut App) {
        app.mount::<Empty>(EmptyProps {});
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

    #[component]
    fn text_field() {
        view! {
            container {
                text_input(width: 120.0, height: 30.0)
            }
        }
    }

    fn field(app: &mut App) {
        app.mount::<TextField>(TextFieldProps {});
    }

    /// An idle focused field costs a timed frame per blink, and no frame in
    /// between: the shell hands the host the deadline instead of a redraw.
    #[test]
    fn a_blinking_caret_is_scheduled_on_its_deadline_rather_than_polled() {
        let window = Arc::new(FakeWindow::new(PhysicalSize::new(800, 600), 2.0));
        window.state().capabilities = HostCapabilities::TIMED_REDRAW;
        let (mut shell, _) = Shell::new(
            window.clone(),
            App::new(field),
            FBackend::new(),
            MockRenderBackend::default(),
            ShellOptions::default(),
        );
        let t0 = Instant::now();
        let half = Duration::from_millis(500);
        let _ = shell.dispatch_event(
            t0,
            ShellEvent::PointerMoved {
                position: Point::new(10.0, 10.0),
                device_id: None,
            },
        );
        for pressed in [true, false] {
            let _ = shell.dispatch_event(
                t0,
                ShellEvent::PointerButton {
                    button: PointerButton::Primary,
                    pressed,
                    device_id: None,
                },
            );
        }
        // The focus change is painted on an ordinary redraw.
        send(&mut shell, ShellEvent::RedrawRequested);
        window.state().calls.clear();
        let _ = shell.about_to_wait();
        assert!(
            window.calls().is_empty(),
            "already asked: {:?}",
            window.calls()
        );

        let _ = shell.dispatch_event(
            t0 + half,
            ShellEvent::FrameTick(FrameInfo {
                target: Some(t0 + half),
                interval: None,
            }),
        );
        assert_eq!(
            window.calls(),
            vec![Call::RequestRedrawAt(t0 + half * 2)],
            "the blink frame asks for the next blink, and for nothing sooner"
        );
    }
}
