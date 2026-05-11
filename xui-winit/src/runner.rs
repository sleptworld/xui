use std::fmt;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::error::{EventLoopError, OsError};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};
use xui::{ControlFlow as XuiControlFlow, GuiRuntime, RuntimeEvent};
use xui_interface::{Point, RenderBackend};

use crate::translate::translate_window_event;

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

pub struct WinitRunner<B: RenderBackend> {
    runtime: GuiRuntime<B>,
    options: WinitRunnerOptions,
    window: Option<Window>,
    window_id: Option<WindowId>,
    last_cursor_position: Option<Point>,
    window_error: Option<OsError>,
    render_error: Option<B::Error>,
}

impl<B: RenderBackend> WinitRunner<B> {
    pub fn new(runtime: GuiRuntime<B>) -> Self {
        Self::with_options(runtime, WinitRunnerOptions::default())
    }

    pub fn with_options(runtime: GuiRuntime<B>, options: WinitRunnerOptions) -> Self {
        Self {
            runtime,
            options,
            window: None,
            window_id: None,
            last_cursor_position: None,
            window_error: None,
            render_error: None,
        }
    }

    pub fn runtime(&self) -> &GuiRuntime<B> {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut GuiRuntime<B> {
        &mut self.runtime
    }

    pub fn window(&self) -> Option<&Window> {
        self.window.as_ref()
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
                self.runtime.handle_event(other);
                self.request_redraw_if_dirty();
            }
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.runtime.frame() {
            self.render_error = Some(error);
            event_loop.exit();
        }
    }

    fn request_redraw_if_dirty(&self) {
        if self.runtime.app().is_dirty() {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }
}

impl<B: RenderBackend> ApplicationHandler for WinitRunner<B> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        match event_loop.create_window(self.options.window_attributes.clone()) {
            Ok(window) => {
                let size = window.inner_size();
                self.runtime
                    .app_mut()
                    .resize(xui::Size::new(size.width as f32, size.height as f32));
                self.window_id = Some(window.id());
                self.window = Some(window);
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

        if let WindowEvent::CursorMoved { position, .. } = &event {
            self.last_cursor_position = Some(Point::new(position.x as f32, position.y as f32));
        }

        for event in translate_window_event(&event, self.last_cursor_position) {
            self.handle_runtime_event(event_loop, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.runtime.control_flow() {
            XuiControlFlow::Exit => event_loop.exit(),
            XuiControlFlow::Poll => self.request_redraw_if_dirty(),
            XuiControlFlow::Wait => {}
        }
    }
}
