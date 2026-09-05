//! Placeholder for platforms with no Skia GPU backend.
//!
//! The crate already refuses GPU presentation outside macOS, Linux and Windows
//! (see [`crate::present::WindowPresenter::new_gpu`]); this keeps the wgpu
//! module compiling there rather than making the feature a build error.

use skia_safe::{Image, gpu::DirectContext};

use super::init;
use crate::SkiaBackendError;

pub(crate) struct PlatformDevice;

pub(crate) struct Interop;

impl Interop {
    pub(crate) const NAME: &'static str = "unsupported";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::empty();

    pub(crate) fn new(
        _adapter: &wgpu::Adapter,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Result<Self, SkiaBackendError> {
        Err(init("this platform has no Skia/wgpu interop backend"))
    }

    pub(crate) fn platform_device(&self) -> &PlatformDevice {
        unreachable!("`Interop::new` never succeeds on this platform")
    }

    pub(crate) fn borrow_image(
        &self,
        _context: &mut DirectContext,
        _texture: &wgpu::Texture,
    ) -> Result<Image, SkiaBackendError> {
        Err(init("this platform has no Skia/wgpu interop backend"))
    }
}
