use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use crate::device::WinitDeviceRegistry;
use crate::translate::{
    translate_mouse_button, translate_mouse_wheel, translate_named_key, translate_physical_key,
};
#[cfg(feature = "wgpu")]
use crate::wgpu::{WGPUBackend, WgpuBackendInitError};
use crate::window::{window_attributes, WinitWindow};
use winit::application::ApplicationHandler;
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};
use xui_core::app::{AppRenderError, ComponentFn};
use xui_core::render::RenderBackend;
use xui_core::runtime::GuiRuntime;
use xui_core::text::TextHost;
use xui_core::App;
#[cfg(feature = "wgpu")]
use xui_cosmic::{CosmicEngine, FontSet};
use xui_f::FBackend;
use xui_interface::events::{KeyState, KeyText, RawIme, TextPayload};
use xui_interface::{Modifiers, Point, TextBackend, TextOffset, TextRange};
use xui_shell::{
    KeyInput, PhysicalSize, Shell, ShellControl, ShellEvent, ShellOptions, WindowOptions,
};

#[derive(Debug, Clone)]
pub struct WinitRunnerOptions {
    /// The window to open. It is created hidden and shown once its first frame
    /// is painted, unless `window.visible` is false.
    pub window: WindowOptions,
    pub exit_on_close_requested: bool,
}

impl Default for WinitRunnerOptions {
    fn default() -> Self {
        Self {
            window: WindowOptions::default(),
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

type BackendFactory<B, T> =
    Box<dyn FnOnce(Arc<WinitWindow>) -> Result<(App, T, B), WinitBackendInitError> + 'static>;

/// Hosts an [`App`] in a winit window.
///
/// Only the winit half lives here: the event loop, window creation, and
/// translating winit events into [`ShellEvent`]s. Everything else is
/// [`Shell`]'s.
pub struct WinitRunner<B: RenderBackend<TextHost<T>>, T: TextBackend> {
    f_init: Option<BackendFactory<B, T>>,
    options: WinitRunnerOptions,
    shell: Option<Shell<WinitWindow, B, T>>,
    window_id: Option<WindowId>,
    window_error: Option<OsError>,
    backend_init_error: Option<WinitBackendInitError>,
    render_error: Option<AppRenderError<B::Error>>,
    device_registry: WinitDeviceRegistry,
    event_proxy: Option<EventLoopProxy<WinitUserEvent>>,
}

impl<B: RenderBackend<TextHost<T>>, T: TextBackend> WinitRunner<B, T> {
    pub fn with_backend_factory<F>(factory: F, option: Option<WinitRunnerOptions>) -> Self
    where
        F: FnOnce(Arc<WinitWindow>) -> (App, T, B) + 'static,
    {
        Self::with_options(factory, option.unwrap_or_default())
    }

    pub fn with_options<F>(factory: F, options: WinitRunnerOptions) -> Self
    where
        F: FnOnce(Arc<WinitWindow>) -> (App, T, B) + 'static,
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
        F: FnOnce(Arc<WinitWindow>) -> Result<(App, T, B), E> + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::with_fallible_options(factory, option.unwrap_or_default())
    }

    pub fn with_fallible_options<F, E>(factory: F, options: WinitRunnerOptions) -> Self
    where
        F: FnOnce(Arc<WinitWindow>) -> Result<(App, T, B), E> + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        Self {
            f_init: Some(Box::new(move |window| {
                factory(window).map_err(|error| Box::new(error) as WinitBackendInitError)
            })),
            options,
            shell: None,
            window_id: None,
            window_error: None,
            backend_init_error: None,
            render_error: None,
            device_registry: WinitDeviceRegistry::default(),
            event_proxy: None,
        }
    }

    pub fn runtime(&self) -> &GuiRuntime<B, T> {
        self.shell.as_ref().unwrap().runtime()
    }

    pub fn runtime_mut(&mut self) -> &mut GuiRuntime<B, T> {
        self.shell.as_mut().unwrap().runtime_mut()
    }

    pub fn window(&self) -> Option<&Window> {
        self.shell.as_ref().map(|shell| shell.window().winit())
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

    fn apply(&mut self, event_loop: &ActiveEventLoop, control: ShellControl) {
        if control == ShellControl::Exit {
            if let Some(error) = self
                .shell
                .as_mut()
                .and_then(|shell| shell.take_render_error())
            {
                self.render_error = Some(error);
            }
            event_loop.exit();
        }
    }

    fn translate_window_event(&mut self, event: &WindowEvent) -> Option<ShellEvent> {
        let scale = self.shell.as_ref()?.window().scale_factor() as f32;
        Some(match event {
            WindowEvent::Resized(size) => {
                ShellEvent::Resized(PhysicalSize::new(size.width, size.height))
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                ShellEvent::ScaleFactorChanged(*scale_factor)
            }
            WindowEvent::CloseRequested | WindowEvent::Destroyed => ShellEvent::CloseRequested,
            WindowEvent::Focused(focused) => ShellEvent::Focused(*focused),
            WindowEvent::Occluded(occluded) => ShellEvent::Occluded(*occluded),
            WindowEvent::ModifiersChanged(modifiers) => {
                ShellEvent::ModifiersChanged(translate_modifiers(modifiers.state()))
            }
            WindowEvent::CursorMoved {
                position,
                device_id,
            } => ShellEvent::PointerMoved {
                position: Point::new(position.x as f32, position.y as f32).scale(1. / scale),
                device_id: Some(self.device_registry.get_or_insert(*device_id)),
            },
            WindowEvent::MouseInput {
                state,
                button,
                device_id,
            } => ShellEvent::PointerButton {
                button: translate_mouse_button(*button)?,
                pressed: *state == ElementState::Pressed,
                device_id: Some(self.device_registry.get_or_insert(*device_id)),
            },
            WindowEvent::MouseWheel {
                delta, device_id, ..
            } => ShellEvent::Wheel {
                delta: translate_mouse_wheel(scale, delta),
                device_id: Some(self.device_registry.get_or_insert(*device_id)),
                is_inertial: false,
            },
            WindowEvent::KeyboardInput { event, .. } => ShellEvent::Keyboard(KeyInput {
                physical_key: translate_physical_key(event.physical_key),
                named_key: translate_named_key(&event.logical_key),
                state: match event.state {
                    ElementState::Pressed => KeyState::Down,
                    ElementState::Released => KeyState::Up,
                },
                text: event.text.as_deref().and_then(KeyText::try_new),
                is_repeat: event.repeat,
            }),
            WindowEvent::Ime(ime) => ShellEvent::Ime(translate_ime(ime)),
            WindowEvent::RedrawRequested => ShellEvent::RedrawRequested,
            _ => return None,
        })
    }
}

#[cfg(feature = "wgpu")]
pub fn runner(
    app: ComponentFn,
    options: Option<WinitRunnerOptions>,
) -> WinitRunner<WGPUBackend, CosmicEngine> {
    let app = App::new(app);
    let options = options.unwrap_or_default();
    let fonts = options.fonts.clone();
    WinitRunner::with_fallible_options(
        move |w| -> Result<_, WgpuBackendInitError> {
            Ok((
                app,
                CosmicEngine::with_fonts(w.scale_factor() as f32, fonts),
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
) -> WinitRunner<xui_skia::SkiaBackend<FBackend>, FBackend> {
    let app = App::new(app);
    let options = options.unwrap_or_default();
    WinitRunner::with_fallible_options(
        move |window| -> Result<_, std::io::Error> {
            #[allow(unused_mut)]
            let mut app = app;
            let backend = xui_skia::SkiaBackend::<FBackend>::new(window.clone())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            // Hand the app the renderer's device, so a canvas GPU painter draws
            // on the device Skia composites from. `None` means Skia is on a
            // self-owned device -- the shared path failed to start, or was
            // turned off -- and a GPU painter then draws nothing.
            #[cfg(feature = "skia-wgpu")]
            if let Some(context) = backend.wgpu_context() {
                app.set_gpu_context(xui_core::widgets::CanvasGpuContext::new(
                    context.device().clone(),
                    context.queue().clone(),
                ));
            }
            Ok((app, FBackend::new(), backend))
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

fn translate_ime(ime: &winit::event::Ime) -> RawIme {
    let timestamp = Instant::now();
    match ime {
        winit::event::Ime::Enabled => RawIme::Enabled { timestamp },
        winit::event::Ime::Preedit(text, cursor) => RawIme::Preedit {
            text: TextPayload::new(text),
            cursor: cursor.map(|(start, end)| {
                TextRange::new(TextOffset::byte_offset(start), TextOffset::byte_offset(end))
            }),
            timestamp,
        },
        winit::event::Ime::Commit(text) => RawIme::Commit {
            text: TextPayload::new(text),
            timestamp,
        },
        winit::event::Ime::Disabled => RawIme::Disabled { timestamp },
    }
}

impl<B: RenderBackend<TextHost<T>>, T: TextBackend> ApplicationHandler<WinitUserEvent>
    for WinitRunner<B, T>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.shell.is_some() {
            return;
        }

        // Creating the window visible puts an unpainted — white — window on
        // screen for as long as backend setup, font loading and the first
        // build take (close to a second in release, several times that in a
        // debug build). Nothing can paint into it during that window, so hold
        // it back; `Shell` shows it once the first frame is actually on it.
        let attributes = window_attributes(&self.options.window).with_visible(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(WinitWindow::new(window)),
            Err(error) => {
                self.window_error = Some(error);
                event_loop.exit();
                return;
            }
        };
        self.window_id = Some(window.id());

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

        let options = ShellOptions {
            visible: self.options.window.visible,
            exit_on_close_requested: self.options.exit_on_close_requested,
        };
        let (shell, control) = Shell::new(window, app, text, backend, options);
        self.shell = Some(shell);
        self.apply(event_loop, control);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) {
            return;
        }
        let Some(event) = self.translate_window_event(&event) else {
            return;
        };
        let Some(shell) = self.shell.as_mut() else {
            return;
        };
        let control = shell.handle_event(event);
        self.apply(event_loop, control);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(shell) = self.shell.as_mut() else {
            return;
        };
        let control = shell.about_to_wait();
        self.apply(event_loop, control);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WinitUserEvent) {
        let Some(shell) = self.shell.as_mut() else {
            return;
        };
        let control = match event {
            WinitUserEvent::Wake => shell.handle_event(ShellEvent::Wake),
        };
        self.apply(event_loop, control);
    }
}
