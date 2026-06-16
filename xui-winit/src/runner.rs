use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use crate::device::WinitDeviceRegistry;
use crate::translate::{translate_key, translate_mouse_button, translate_mouse_wheel};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};
use xui::App;
use xui::{runtime::ControlFlow as XuiControlFlow, runtime::GuiRuntime, runtime::RuntimeEvent};
use xui_interface::events::{RawEvent, XuiPointerId};
use xui_interface::{
    Event, Modifiers, Point, PointerButtons, PointerKind, RawKey, RawPointerButton, RawPointerMove,
    RawTextInput, RawWheel, RawWindowEvent, RenderBackend, Size, TextMeasurer,
};

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
    Render(E),
}

impl<E: fmt::Debug> fmt::Debug for WinitRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => f.debug_tuple("EventLoop").field(error).finish(),
            Self::Window(error) => f.debug_tuple("Window").field(error).finish(),
            Self::Render(error) => f.debug_tuple("Render").field(error).finish(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for WinitRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => write!(f, "winit event loop error: {error}"),
            Self::Window(error) => write!(f, "winit window error: {error}"),
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
            Self::Render(error) => Some(error),
        }
    }
}

impl<E> From<EventLoopError> for WinitRunError<E> {
    fn from(value: EventLoopError) -> Self {
        Self::EventLoop(value)
    }
}

pub struct WinitRunner<B: RenderBackend<T>, T: TextMeasurer, F>
where
    F: FnOnce(Arc<Window>) -> (App, T, B),
{
    f_init: Option<F>,
    runtime: Option<GuiRuntime<B, T>>,
    options: WinitRunnerOptions,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    last_cursor_position: Option<Point>,
    modifiers: Modifiers,
    pointer_buttons: PointerButtons,
    window_error: Option<OsError>,
    render_error: Option<B::Error>,
    device_registry: WinitDeviceRegistry,
}

impl<B: RenderBackend<T>, T: TextMeasurer, F> WinitRunner<B, T, F>
where
    F: FnOnce(Arc<Window>) -> (App, T, B),
{
    pub fn with_backend_factory(factory: F, option: Option<WinitRunnerOptions>) -> Self {
        Self::with_options(factory, option.unwrap_or_default())
    }

    pub fn with_options(factory: F, options: WinitRunnerOptions) -> Self {
        Self {
            f_init: Some(factory),
            runtime: None,
            options,
            window: None,
            window_id: None,
            last_cursor_position: None,
            modifiers: Modifiers::default(),
            pointer_buttons: PointerButtons::default(),
            window_error: None,
            render_error: None,
            device_registry: WinitDeviceRegistry::default(),
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

    pub fn run(mut self) -> Result<(), WinitRunError<B::Error>> {
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut self)?;

        if let Some(error) = self.window_error {
            return Err(WinitRunError::Window(error));
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
                self.request_redraw_if_dirty();
            }
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.runtime_mut().frame() {
            self.render_error = Some(error);
            event_loop.exit();
        } else {
            self.request_redraw_if_dirty();
        }
    }

    fn request_redraw_if_dirty(&self) {
        if self.runtime().app().is_dirty() {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
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
                let raw = RawKey {
                    key: translate_key(&event.logical_key),
                    modifiers: self.modifiers,
                    timestamp,
                    is_repeat: event.repeat,
                };
                let event = match event.state {
                    ElementState::Pressed => RawEvent::KeyDown(raw),
                    ElementState::Released => RawEvent::KeyUp(raw),
                };
                (vec![RuntimeEvent::Input(event)], None)
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => (
                vec![RuntimeEvent::Input(RawEvent::TextInput(RawTextInput {
                    text: text.clone(),
                    modifiers: self.modifiers,
                    timestamp,
                }))],
                None,
            ),
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

fn translate_modifiers(modifiers: ModifiersState) -> Modifiers {
    Modifiers {
        shift: modifiers.shift_key(),
        ctrl: modifiers.control_key(),
        alt: modifiers.alt_key(),
        meta: modifiers.super_key(),
    }
}

impl<B: RenderBackend<T>, T: TextMeasurer, F> ApplicationHandler for WinitRunner<B, T, F>
where
    F: FnOnce(Arc<Window>) -> (App, T, B),
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        match event_loop.create_window(self.options.window_attributes.clone()) {
            Ok(window) => {
                self.window_id = Some(window.id());
                let window = Arc::new(window);
                let (app, text, backend) = (self.f_init.take().unwrap())(window.clone());
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
        if self.window_id != Some(window_id) {
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
            if let Some(window) = self.window.as_ref() {
                let size = Self::logical_size_at_scale(window.inner_size(), *scale_factor as f32);
                self.runtime_mut().handle_event(RuntimeEvent::Resize(size));
            }

            self.runtime_mut().app_mut().mark_needs_rebuild();
            self.request_redraw_if_dirty();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.runtime().control_flow() {
            XuiControlFlow::Exit => event_loop.exit(),
            XuiControlFlow::Poll => self.request_redraw_if_dirty(),
            XuiControlFlow::Wait => {}
        }
    }
}
