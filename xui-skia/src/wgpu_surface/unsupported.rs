//! Placeholder interop for platforms with no Skia GPU backend.
//!
//! The crate already refuses GPU presentation outside macOS, Linux and Windows
//! (see [`crate::present::WindowPresenter::new_gpu`]); this keeps the wgpu
//! module compiling there rather than making the feature a build error.

use skia_safe::{Image, Surface, gpu::DirectContext};

use super::init;
use crate::SkiaBackendError;

pub(crate) struct Interop {
    _private: (),
}

impl Interop {
    pub(crate) const NAME: &'static str = "unsupported";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::empty();

    pub(crate) fn new(
        _adapter: &wgpu::Adapter,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Result<(Self, DirectContext), SkiaBackendError> {
        Err(init("this platform has no Skia/wgpu interop backend"))
    }

    pub(crate) fn wrap_render_target(
        &self,
        _context: &mut DirectContext,
        _texture: &wgpu::Texture,
    ) -> Result<Surface, SkiaBackendError> {
        Err(init("this platform has no Skia/wgpu interop backend"))
    }

    pub(crate) fn borrow_image(
        &self,
        _context: &mut DirectContext,
        _texture: &wgpu::Texture,
    ) -> Result<Image, SkiaBackendError> {
        Err(init("this platform has no Skia/wgpu interop backend"))
    }

    pub(crate) fn finish_frame(&self, _context: &mut DirectContext, _surface: &mut Surface) {}
}
