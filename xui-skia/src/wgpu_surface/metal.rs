//! Metal interop between wgpu and Skia.
//!
//! `wgpu::Device::as_hal` hands out the `MTLDevice` behind the wgpu device, and
//! that is all Skia needs to address the same memory: an `MTLTexture` created
//! by wgpu is a texture Skia can render into or sample, with no import step and
//! no copy.
//!
//! # The command queue
//!
//! Skia's `mtl::BackendContext` wants a device *and* a command queue, and
//! `wgpu-hal` 29 does not expose the one it uses -- `metal::Queue` keeps its
//! `QueueShared` private, with no accessor. So Skia gets a second queue created
//! on the shared device.
//!
//! Two `MTLCommandQueue`s on one `MTLDevice` share memory but have no ordering
//! relative to each other, and wgpu presents from its queue while Skia rendered
//! from ours. [`Interop::finish_frame`] therefore waits for Skia's work on the
//! CPU before the caller presents. That is a real stall, and it is the price of
//! the missing accessor rather than anything inherent to the design: the moment
//! `wgpu-hal` exposes its raw queue -- or we build the wgpu device ourselves
//! from `hal::OpenDevice { device: metal::Device::device_from_raw(..), queue:
//! metal::Queue::queue_from_raw(..) }` so that both sides share *our* queue --
//! the sync collapses into plain submission ordering and this comment goes
//! away.

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLCommandQueue, MTLDevice};
use skia_safe::{
    Image, Surface,
    gpu::{
        self, DirectContext, Mipmapped, SurfaceOrigin, backend_render_targets, backend_textures,
        mtl,
    },
};

use super::{color_space, color_type, init, present_error};
use crate::SkiaBackendError;

pub(crate) struct Interop {
    /// Kept alive for as long as the Skia context: `mtl::BackendContext` only
    /// borrows the raw pointer we handed it.
    _command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl Interop {
    pub(crate) const NAME: &'static str = "Metal";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::METAL;

    pub(crate) fn new(
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Result<(Self, DirectContext), SkiaBackendError> {
        // SAFETY: the guard is dropped before this function returns, and the
        // `MTLDevice` it exposes is retained by `newCommandQueue` and by the
        // wgpu device itself for as long as the Skia context lives.
        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| init("the wgpu device is not Metal-backed"))?;
        let metal_device = hal_device.raw_device().clone();
        drop(hal_device);

        let command_queue = metal_device
            .newCommandQueue()
            .ok_or_else(|| init("could not create a Metal command queue on the wgpu device"))?;

        // SAFETY: both pointers outlive the context -- the device through the
        // wgpu device we were handed, the queue through `Self::_command_queue`.
        let backend = unsafe {
            mtl::BackendContext::new(
                Retained::as_ptr(&metal_device) as mtl::Handle,
                Retained::as_ptr(&command_queue) as mtl::Handle,
            )
        };
        let context = gpu::direct_contexts::make_metal(&backend, None).ok_or_else(|| {
            init("Skia could not create a Ganesh Metal context on the wgpu device")
        })?;

        Ok((
            Self {
                _command_queue: command_queue,
            },
            context,
        ))
    }

    pub(crate) fn wrap_render_target(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<Surface, SkiaBackendError> {
        let (width, height) = (texture.width() as i32, texture.height() as i32);
        let info = texture_info(texture)?;
        let target = backend_render_targets::make_mtl((width, height), &info);
        gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            SurfaceOrigin::TopLeft,
            color_type(texture.format())?,
            color_space(texture.format()),
            None,
        )
        .ok_or_else(|| present_error("Skia could not wrap the wgpu swapchain texture"))
    }

    pub(crate) fn borrow_image(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<Image, SkiaBackendError> {
        let (width, height) = (texture.width() as i32, texture.height() as i32);
        let info = texture_info(texture)?;
        // SAFETY: the returned `BackendTexture` only borrows the `MTLTexture`;
        // the caller contract on `WgpuPresenter::borrow_image` requires the
        // wgpu texture to outlive the image built from it.
        let backend = unsafe {
            backend_textures::make_mtl(
                (width, height),
                Mipmapped::No,
                &info,
                "xui-skia imported wgpu texture",
            )
        };
        gpu::images::borrow_texture_from(
            context,
            &backend,
            SurfaceOrigin::TopLeft,
            color_type(texture.format())?,
            skia_safe::AlphaType::Premul,
            color_space(texture.format()),
        )
        .ok_or_else(|| {
            SkiaBackendError::WgpuTextureImport(
                "Skia could not borrow the imported Metal texture".into(),
            )
        })
    }

    /// Flushes Skia's frame and waits for it.
    ///
    /// `BackendSurfaceAccess` is a no-op on Metal -- there are no image layouts
    /// to hand back. The CPU sync is the cross-queue hazard described in the
    /// module docs, not a Metal requirement.
    pub(crate) fn finish_frame(&self, context: &mut DirectContext, surface: &mut Surface) {
        context.flush_and_submit_surface(surface, None);
        context.submit(gpu::SyncCpu::Yes);
    }
}

fn texture_info(texture: &wgpu::Texture) -> Result<mtl::TextureInfo, SkiaBackendError> {
    // SAFETY: the guard lives across the `TextureInfo` construction, which
    // retains the `MTLTexture` it is given.
    let hal_texture = unsafe { texture.as_hal::<wgpu::hal::api::Metal>() }
        .ok_or_else(|| init("the wgpu texture is not Metal-backed"))?;
    let handle: *const ProtocolObject<dyn objc2_metal::MTLTexture> = hal_texture.raw_handle();
    Ok(unsafe { mtl::TextureInfo::new(handle as mtl::Handle) })
}
