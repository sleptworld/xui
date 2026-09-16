use std::sync::Arc;
use std::time::Instant;

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
    Modifiers, PlatformOutput, Point, PointerButton, PointerButtons, PointerKind, RawPointerButton,
    RawPointerMove, RawWheel, RawWindowEvent, ScrollDelta, TextBackend,
};

use crate::window::{PhysicalSize, PlatformWindow};

/// A platform event, already translated out of the host's vocabulary.
///
/// Sizes are physical pixels, because that is what a surface is sized in;
/// positions and scroll deltas are logical, relative to the content area.
/// Timestamps and modifier state are filled in by [`Shell`].
#[derive(Debug, Clone)]
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

/// What the host should do with its event loop after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ShellControl {
    Continue,
    /// Stop the event loop. If a frame failed, [`Shell::take_render_error`]
    /// has the error.
    Exit,
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

/// Drives a [`GuiRuntime`] from platform events.
///
/// The host owns the event loop and the window. It hands each translated event
/// to [`Shell::handle_event`], calls [`Shell::about_to_wait`] when the loop goes
/// idle, and acts on the [`ShellControl`] each returns. Everything that does not
/// depend on the platform happens here.
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

    /// The error that made a frame fail, if one did.
    pub fn take_render_error(&mut self) -> Option<AppRenderError<B::Error>> {
        self.render_error.take()
    }

    pub fn handle_event(&mut self, event: ShellEvent) -> ShellControl {
        let timestamp = Instant::now();
        match event {
            ShellEvent::Resized(size) => {
                let size = size.to_logical(self.window.scale_factor());
                let control = self.dispatch(RuntimeEvent::Resize(size));
                // A minimized window reports no occlusion on Windows or X11;
                // what it reports is a resize to nothing.
                self.refresh_window_visibility();
                control
            }
            ShellEvent::ScaleFactorChanged(scale_factor) => {
                self.set_scale_factor(scale_factor);
                ShellControl::Continue
            }
            ShellEvent::CloseRequested => self.dispatch(RuntimeEvent::Exit),
            ShellEvent::Focused(focused) => {
                let raw = RawWindowEvent {
                    timestamp,
                    modifiers: self.modifiers,
                };
                let control = self.dispatch(RuntimeEvent::Input(if focused {
                    RawEvent::WindowFocus(raw)
                } else {
                    RawEvent::WindowBlur(raw)
                }));
                // Not itself a visibility signal, but a reliable moment at
                // which one may have arrived unannounced.
                self.refresh_window_visibility();
                control
            }
            ShellEvent::Occluded(occluded) => {
                self.occluded = occluded;
                self.refresh_window_visibility();
                ShellControl::Continue
            }
            ShellEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                ShellControl::Continue
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
            ShellEvent::RedrawRequested => self.render(),
            ShellEvent::Wake => {
                self.runtime.app_mut().drain_async_messages();
                self.request_redraw_if_dirty();
                ShellControl::Continue
            }
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

    fn dispatch(&mut self, event: RuntimeEvent) -> ShellControl {
        match event {
            RuntimeEvent::RedrawRequested => self.render(),
            RuntimeEvent::Exit if self.options.exit_on_close_requested => ShellControl::Exit,
            other => {
                self.runtime.handle_event(other);
                self.sync_platform_output();
                self.request_redraw_if_dirty();
                ShellControl::Continue
            }
        }
    }

    fn render(&mut self) -> ShellControl {
        match self.runtime.frame() {
            Err(error) => {
                self.render_error = Some(error);
                ShellControl::Exit
            }
            Ok(_) => {
                // Unconditionally, not only when the frame drew something: an
                // app whose first frame has nothing to draw still gets its
                // window, rather than staying hidden forever.
                self.show_window_once_painted();
                self.sync_platform_output();
                self.request_redraw_if_dirty();
                ShellControl::Continue
            }
        }
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
