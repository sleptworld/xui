use super::BuiltFrame;
use xui_interface::{NodeLifecycleEvent, Size};

pub trait RenderBackend<T> {
    type Error;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error>;
    fn submit(&mut self, frame: &BuiltFrame, text: &mut T) -> Result<(), Self::Error>;
    fn end_frame(&mut self) -> Result<(), Self::Error>;

    fn did_present(&self) -> bool {
        true
    }

    fn resize(&mut self, _size: Size<f32>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_factor(&mut self, _factor: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

#[derive(Debug, Clone, Default)]
pub struct MockRenderBackend {
    pub frame_size: Option<Size<f32>>,
    pub frames: usize,
    pub last_frame: Option<BuiltFrame>,
}

impl<T> RenderBackend<T> for MockRenderBackend {
    type Error = core::convert::Infallible;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
        self.frame_size = Some(size);
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
}
