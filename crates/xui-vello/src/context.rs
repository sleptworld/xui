/// The wgpu device and queue the backend renders on.
///
/// Same shape as `xui_skia::WgpuContext`, and used the same way: a canvas
/// `GpuPainter` must draw on *this* device, because a texture from another
/// device cannot be sampled here. Hand it to the app as
/// `xui_core::widgets::CanvasGpuContext::new(ctx.device().clone(), ctx.queue().clone())`.
#[derive(Clone)]
pub struct WgpuContext {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

impl WgpuContext {
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

impl std::fmt::Debug for WgpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuContext").finish_non_exhaustive()
    }
}
