use super::BuiltFrame;
use xui_interface::{NodeLifecycleEvent, Size, WindowId};

/// Draws built frames. One backend draws every window of an app: each frame
/// is for one window, named by [`Self::begin_frame`], and goes to that
/// window's surface, while what is costly to keep -- the device, glyphs,
/// images -- is the backend's, for all of them.
///
/// How a backend comes by a window's surface is the host's business (see
/// `xui_shell::SurfaceBackend`); the surface it was made with is the main
/// window's. A backend that draws one surface only treats every window as
/// that one.
pub trait RenderBackend<T> {
    type Error;

    /// Starts a frame of `window`, at `size` in logical pixels.
    fn begin_frame(&mut self, window: WindowId, size: Size<f32>) -> Result<(), Self::Error>;
    fn submit(&mut self, frame: &BuiltFrame, text: &mut T) -> Result<(), Self::Error>;
    fn end_frame(&mut self) -> Result<(), Self::Error>;

    fn did_present(&self) -> bool {
        true
    }

    fn resize(&mut self, _window: WindowId, _size: Size<f32>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_factor(&mut self, _window: WindowId, _factor: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    /// `window` closed: let go of its surface and what was kept of its frames.
    fn remove_surface(&mut self, _window: WindowId) {}

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

pub trait RenderBackendExt<T>: RenderBackend<T> {}

#[derive(Debug, Clone, Default)]
pub struct MockRenderBackend {
    pub frame_size: Option<Size<f32>>,
    pub frames: usize,
    pub last_frame: Option<BuiltFrame>,
    /// The window each frame was begun for, in order.
    pub windows: Vec<WindowId>,
    /// The windows [`RenderBackend::remove_surface`] was told closed.
    pub removed: Vec<WindowId>,
}

impl MockRenderBackend {
    /// How many frames were begun for `window`.
    pub fn frames_of(&self, window: WindowId) -> usize {
        self.windows
            .iter()
            .filter(|&&drawn| drawn == window)
            .count()
    }
}

impl<T> RenderBackend<T> for MockRenderBackend {
    type Error = core::convert::Infallible;

    fn begin_frame(&mut self, window: WindowId, size: Size<f32>) -> Result<(), Self::Error> {
        self.frame_size = Some(size);
        self.windows.push(window);
        Ok(())
    }

    fn submit(&mut self, frame: &BuiltFrame, _text: &mut T) -> Result<(), Self::Error> {
        self.last_frame = Some(frame.clone());
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        self.frames += 1;
        Ok(())
    }

    fn remove_surface(&mut self, window: WindowId) {
        self.removed.push(window);
    }
}
