use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use crate::device::WinitDeviceRegistry;
use crate::translate::{
    translate_mouse_button, translate_mouse_wheel, translate_named_key, translate_physical_key,
};
#[cfg(feature = "wgpu")]
use crate::wgpu::{WGPUBackend, WgpuBackendInitError};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};
use xui::App;
use xui::app::ComponentFn;
use xui::text::TextHost;
use xui::{
    app::AppRenderError, render::RenderBackend, runtime::ControlFlow as XuiControlFlow,
    runtime::GuiRuntime, runtime::RuntimeEvent,
};
use xui_interface::events::{
    KeyState, KeyText, RawEvent, RawIme, RawKeyboard, TextPayload, XuiPointerId,
};
use xui_interface::{
    CursorIcon, Modifiers, PlatformOutput, Point, PointerButtons, PointerKind, RawPointerButton,
    RawPointerMove, RawWheel, RawWindowEvent, Size, TextBackend, TextOffset, TextRange,
};
use xui_text_engine::CosmicEngine;

#[derive(Debug, Clone)]
pub struct WinitRunnerOptions {
    pub window_attributes: WindowAttributes,
    pub exit_on_close_requested: bool,
}

impl Default for WinitRunnerOptions {
    fn default() -> Self {
        Self {
            window_attributes: Window::default_attributes()
                .with_title("XUI")
                .with_inner_size(PhysicalSize::new(800, 600)),
            exit_on_close_requested: true,
        }
    }
}

pub enum WinitRunError<E> {
    EventLoop(EventLoopError),
    Window(OsError),
    BackendInit(WinitBackendInitError),
    Render(E),
}

pub type WinitBackendInitError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Copy)]
pub enum WinitUserEvent {
    Wake,
}

impl<E: fmt::Debug> fmt::Debug for WinitRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => f.debug_tuple("EventLoop").field(error).finish(),
            Self::Window(error) => f.debug_tuple("Window").field(error).finish(),
            Self::BackendInit(error) => f.debug_tuple("BackendInit").field(error).finish(),
            Self::Render(error) => f.debug_tuple("Render").field(error).finish(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for WinitRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => write!(f, "winit event loop error: {error}"),
            Self::Window(error) => write!(f, "winit window error: {error}"),
            Self::BackendInit(error) => write!(f, "render backend initialization error: {error}"),
            Self::Render(error) => write!(f, "render backend error: {error}"),
        }
    }
}

impl<E> std::error::Error for WinitRunError<E>
where
    E: fmt::Debug + fmt::Display + std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EventLoop(error) => Some(error),
            Self::Window(error) => Some(error),
            Self::BackendInit(error) => Some(error.as_ref()),
            Self::Render(error) => Some(error),
        }
    }
}

impl<E> From<EventLoopError> for WinitRunError<E> {
    fn from(value: EventLoopError) -> Self {
        Self::EventLoop(value)
    }
}

pub struct WinitRunner<B: RenderBackend<TextHost<T>>, T: TextBackend> {
    f_init: Option<
        Box<dyn FnOnce(Arc<Window>) -> Result<(App, T, B), WinitBackendInitError> + 'static>,
    >,
    runtime: Option<GuiRuntime<B, T>>,
    options: WinitRunnerOptions,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    last_cursor_position: Option<Point>,
    modifiers: Modifiers,
    pointer_buttons: PointerButtons,
    window_error: Option<OsError>,
    backend_init_error: Option<WinitBackendInitError>,
    render_error: Option<AppRenderError<B::Error>>,
    device_registry: WinitDeviceRegistry,
    event_proxy: Option<EventLoopProxy<WinitUserEvent>>,
    last_platform_output: PlatformOutput,
}

impl<B: RenderBackend<TextHost<T>>, T: TextBackend> WinitRunner<B, T> {
    pub fn with_backend_factory<F>(factory: F, option: Option<WinitRunnerOptions>) -> Self
    where
        F: FnOnce(Arc<Window>) -> (App, T, B) + 'static,
    {
        Self::with_options(factory, option.unwrap_or_default())
    }

    pub fn with_options<F>(factory: F, options: WinitRunnerOptions) -> Self
    where
        F: FnOnce(Arc<Window>) -> (App, T, B) + 'static,
    {
        Self::with_fallible_options(
            move |window| Ok::<(App, T, B), std::convert::Infallible>(factory(window)),
            options,
        )
    }

    pub fn with_fallible_backend_factory<F, E>(
        factory: F,
        option: Option<WinitRunnerOptions>,
    ) -> Self
    where
        F: FnOnce(Arc<Window>) -> Result<(App, T, B), E> + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::with_fallible_options(factory, option.unwrap_or_default())
    }

    pub fn with_fallible_options<F, E>(factory: F, options: WinitRunnerOptions) -> Self
    where
        F: FnOnce(Arc<Window>) -> Result<(App, T, B), E> + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        Self {
            f_init: Some(Box::new(move |window| {
                factory(window).map_err(|error| Box::new(error) as WinitBackendInitError)
            })),
            runtime: None,
            options,
            window: None,
            window_id: None,
            last_cursor_position: None,
            modifiers: Modifiers::default(),
            pointer_buttons: PointerButtons::default(),
            window_error: None,
            backend_init_error: None,
            render_error: None,
            device_registry: WinitDeviceRegistry::default(),
            event_proxy: None,
            last_platform_output: PlatformOutput::default(),
        }
    }

    pub fn runtime(&self) -> &GuiRuntime<B, T> {
        self.runtime.as_ref().unwrap()
    }

    pub fn runtime_mut(&mut self) -> &mut GuiRuntime<B, T> {
        self.runtime.as_mut().unwrap()
    }

    pub fn window(&self) -> Option<&Window> {
        self.window.as_deref()
    }

    pub fn run(mut self) -> Result<(), WinitRunError<AppRenderError<B::Error>>> {
        let event_loop = EventLoop::<WinitUserEvent>::with_user_event().build()?;
        self.event_proxy = Some(event_loop.create_proxy());
        event_loop.run_app(&mut self)?;

        if let Some(error) = self.window_error {
            return Err(WinitRunError::Window(error));
        }
        if let Some(error) = self.backend_init_error {
            return Err(WinitRunError::BackendInit(error));
        }
        if let Some(error) = self.render_error {
            return Err(WinitRunError::Render(error));
        }
        Ok(())
    }

    fn handle_runtime_event(&mut self, event_loop: &ActiveEventLoop, event: RuntimeEvent) {
        match event {
            RuntimeEvent::RedrawRequested => self.render(event_loop),
            RuntimeEvent::Exit if self.options.exit_on_close_requested => event_loop.exit(),
            other => {
                self.runtime_mut().handle_event(other);
                self.sync_platform_output();
                self.request_redraw_if_dirty();
            }
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.runtime_mut().frame() {
            self.render_error = Some(error);
            event_loop.exit();
        } else {
            self.sync_platform_output();
            self.request_redraw_if_dirty();
        }
    }

    fn sync_platform_output(&mut self) {
        let next = self.runtime().platform_output().clone();
        let Some(window) = self.window.as_ref() else {
            return;
        };

        if self.last_platform_output.text_input.is_some() != next.text_input.is_some() {
            window.set_ime_allowed(next.text_input.is_some());
        }

        let previous_area = self
            .last_platform_output
            .text_input
            .as_ref()
            .map(|session| session.cursor_area);
        let next_area = next.text_input.as_ref().map(|session| session.cursor_area);
        if previous_area != next_area
            && let Some(area) = next_area {
                window.set_ime_cursor_area(
                    LogicalPosition::new(area.x as f64, area.y as f64),
                    LogicalSize::new(area.width as f64, area.height as f64),
                );
            }

        if self.last_platform_output.cursor != next.cursor {
            match to_winit_cursor(next.cursor) {
                Some(icon) => {
                    window.set_cursor_visible(true);
                    window.set_cursor(icon);
                }
                None => window.set_cursor_visible(false),
            }
        }

        self.last_platform_output = next;
    }

    fn request_redraw_if_dirty(&self) {
        if self.runtime().app().is_dirty()
            && let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
    }

    #[inline(always)]
    fn translate_pointer_position(&self, position: &winit::dpi::PhysicalPosition<f64>) -> Point {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        Point::new(position.x as f32, position.y as f32).scale(1. / scale)
    }

    fn translate_window_event(
        &mut self,
        event: &WindowEvent,
        last_cursor_position: Option<Point>,
    ) -> (Vec<RuntimeEvent>, Option<Point>) {
        let timestamp = Instant::now();
        match event {
            WindowEvent::Resized(size) => {
                (vec![RuntimeEvent::Resize(self.logical_size(*size))], None)
            }
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                (vec![RuntimeEvent::Exit], None)
            }
            WindowEvent::Focused(true) => (
                vec![RuntimeEvent::Input(RawEvent::WindowFocus(RawWindowEvent {
                    timestamp,
                    modifiers: self.modifiers,
                }))],
                None,
            ),
            WindowEvent::Focused(false) => (
                vec![RuntimeEvent::Input(RawEvent::WindowBlur(RawWindowEvent {
                    timestamp,
                    modifiers: self.modifiers,
                }))],
                None,
            ),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = translate_modifiers(modifiers.state());
                (Vec::new(), None)
            }
            WindowEvent::CursorMoved {
                position,
                device_id,
            } => {
                let pointer = self.translate_pointer_position(position);
                let device_id = self.device_registry.get_or_insert(*device_id);

                (
                    vec![RuntimeEvent::Input(RawEvent::PointerMove(RawPointerMove {
                        position: pointer,
                        device_id: Some(device_id),
                        pointer_id: XuiPointerId::new(0),
                        kind: PointerKind::Mouse,
                        button: None,
                        buttons: self.pointer_buttons,
                        modifiers: self.modifiers,
                        timestamp,
                    }))],
                    Some(pointer),
                )
            }
            WindowEvent::MouseInput {
                state,
                button,
                device_id,
                ..
            } => {
                let Some(button) = translate_mouse_button(*button) else {
                    return (Vec::new(), None);
                };
                let position = last_cursor_position.unwrap_or(Point::new(0.0, 0.0));
                self.pointer_buttons
                    .set(button, *state == ElementState::Pressed);
                let device_id = self.device_registry.get_or_insert(*device_id);
                let raw = RawPointerButton {
                    position,
                    pointer_id: XuiPointerId::new(0),
                    device_id: Some(device_id),
                    kind: PointerKind::Mouse,
                    button,
                    buttons: self.pointer_buttons,
                    modifiers: self.modifiers,
                    timestamp,
                };
                let event = match state {
                    ElementState::Pressed => RawEvent::PointerDown(raw),
                    ElementState::Released => RawEvent::PointerUp(raw),
                };
                (vec![RuntimeEvent::Input(event)], None)
            }
            WindowEvent::MouseWheel {
                delta, device_id, ..
            } => (
                vec![RuntimeEvent::Input(RawEvent::Wheel(RawWheel {
                    position: last_cursor_position.unwrap_or(Point::new(0.0, 0.0)),
                    delta: translate_mouse_wheel(self.scale_factor(), delta),
                    device_id: Some(self.device_registry.get_or_insert(*device_id)),
                    pointer_id: Some(XuiPointerId::new(0)),
                    modifiers: self.modifiers,
                    timestamp,
                    is_inertial: false,
                }))],
                None,
            ),
            WindowEvent::KeyboardInput { event, .. } => {
                let raw = RawKeyboard {
                    physical_key: translate_physical_key(event.physical_key),
                    named_key: translate_named_key(&event.logical_key),
                    state: match event.state {
                        ElementState::Pressed => KeyState::Down,
                        ElementState::Released => KeyState::Up,
                    },
                    text: event.text.as_deref().and_then(KeyText::try_new),
                    modifiers: self.modifiers,
                    timestamp,
                    is_repeat: event.repeat,
                };
                (vec![RuntimeEvent::Input(RawEvent::Keyboard(raw))], None)
            }
            WindowEvent::Ime(ime) => {
                let ime = match ime {
                    winit::event::Ime::Enabled => RawIme::Enabled { timestamp },
                    winit::event::Ime::Preedit(text, cursor) => RawIme::Preedit {
                        text: TextPayload::new(text),
                        cursor: cursor.map(|(start, end)| {
                            TextRange::new(
                                TextOffset::byte_offset(start),
                                TextOffset::byte_offset(end),
                            )
                        }),
                        timestamp,
                    },
                    winit::event::Ime::Commit(text) => RawIme::Commit {
                        text: TextPayload::new(text),
                        timestamp,
                    },
                    winit::event::Ime::Disabled => RawIme::Disabled { timestamp },
                };
                (vec![RuntimeEvent::Input(RawEvent::Ime(ime))], None)
            }
            WindowEvent::RedrawRequested => (vec![RuntimeEvent::RedrawRequested], None),
            _ => (Vec::new(), None),
        }
    }

    #[inline(always)]
    fn logical_size(&self, size: PhysicalSize<u32>) -> Size<f32> {
        Self::logical_size_at_scale(size, self.scale_factor())
    }

    #[inline(always)]
    fn scale_factor(&self) -> f32 {
        self.window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0)
    }

    #[inline(always)]
    fn logical_size_at_scale(size: PhysicalSize<u32>, scale: f32) -> Size<f32> {
        let scale = scale.max(f32::EPSILON);
        Size::<f32>::new(size.width as f32 / scale, size.height as f32 / scale)
    }
}

#[cfg(feature = "wgpu")]
pub fn runner(
    app: ComponentFn,
    options: Option<WinitRunnerOptions>,
) -> WinitRunner<WGPUBackend, CosmicEngine> {
    let app = App::new(app);
    WinitRunner::with_fallible_backend_factory(
        |w| -> Result<_, WgpuBackendInitError> {
            Ok((
                app,
                CosmicEngine::new(w.scale_factor() as f32),
                WGPUBackend::new(w)?,
            ))
        },
        options,
    )
}

#[cfg(feature = "skia")]
pub fn runner(
    app: ComponentFn,
    options: Option<WinitRunnerOptions>,
) -> WinitRunner<xui_skia::SkiaBackend<CosmicEngine>, CosmicEngine> {
    let app = App::new(app);
    WinitRunner::with_fallible_backend_factory(
        |window| -> Result<_, std::io::Error> {
            let backend = xui_skia::SkiaBackend::<CosmicEngine>::new(window.clone())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok((
                app,
                CosmicEngine::new(window.scale_factor() as f32),
                backend,
            ))
        },
        options,
    )
}

fn translate_modifiers(modifiers: ModifiersState) -> Modifiers {
    Modifiers {
        shift: modifiers.shift_key(),
        ctrl: modifiers.control_key(),
        alt: modifiers.alt_key(),
        meta: modifiers.super_key(),
    }
}

impl<B: RenderBackend<TextHost<T>>, T: TextBackend> ApplicationHandler<WinitUserEvent>
    for WinitRunner<B, T>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        match event_loop.create_window(self.options.window_attributes.clone()) {
            Ok(window) => {
                self.window_id = Some(window.id());
                let window = Arc::new(window);
                let (app, text, backend) = match (self.f_init.take().unwrap())(window.clone()) {
                    Ok(result) => result,
                    Err(error) => {
                        self.backend_init_error = Some(error);
                        event_loop.exit();
                        return;
                    }
                };
                if let Some(proxy) = self.event_proxy.clone() {
                    app.set_async_wake_callback(move || {
                        let _ = proxy.send_event(WinitUserEvent::Wake);
                    });
                }
                self.runtime = Some(GuiRuntime::new(app, backend, text));
                let size = window.inner_size();
                let init_scale_factor = window.scale_factor();
                self.window = Some(window);
                let logical_size = Self::logical_size_at_scale(size, init_scale_factor as f32);
                self.runtime_mut()
                    .handle_event(RuntimeEvent::Resize(logical_size));
                self.request_redraw_if_dirty();
            }
            Err(error) => {
                self.window_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) || self.runtime.is_none() {
            return;
        }

        let (events, cursor_position) =
            self.translate_window_event(&event, self.last_cursor_position);

        if let Some(position) = cursor_position {
            self.last_cursor_position = Some(position);
        }

        for event in events {
            self.handle_runtime_event(event_loop, event);
        }

        if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = &event {
            let _ = self
                .runtime_mut()
                .backend_mut()
                .set_factor(*scale_factor as f32);
            self.runtime_mut()
                .text_backend_mut()
                .set_scale_factor(*scale_factor as f32);
            if let Some(window) = self.window.as_ref() {
                let size = Self::logical_size_at_scale(window.inner_size(), *scale_factor as f32);
                self.runtime_mut().handle_event(RuntimeEvent::Resize(size));
            }

            self.runtime_mut().app_mut().mark_needs_rebuild();
            self.request_redraw_if_dirty();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.runtime.is_none() {
            return;
        }
        match self.runtime().control_flow() {
            XuiControlFlow::Exit => event_loop.exit(),
            XuiControlFlow::Poll => self.request_redraw_if_dirty(),
            XuiControlFlow::Wait => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: WinitUserEvent) {
        if self.runtime.is_none() {
            return;
        }
        match event {
            WinitUserEvent::Wake => {
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.app_mut().drain_async_messages();
                }
                self.request_redraw_if_dirty();
            }
        }
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
