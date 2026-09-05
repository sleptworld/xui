//! Metal handles behind a wgpu device.
//!
//! `wgpu::Device::as_hal` hands out the `MTLDevice`, and `wgpu::Queue::as_hal`
//! the `MTLCommandQueue`. That is all Skia needs to address the same memory: an
//! `MTLTexture` wgpu created is one Skia can sample, with no import step and no
//! copy.
//!
//! Both are handed to [`crate::metal::MetalPresenter`], which sets the device on
//! its `CAMetalLayer` and commits its presentation command buffer to the same
//! queue Skia renders on. One queue executes command buffers in commit order,
//! so nothing needs a fence or an event.
//!
//! # Version floor
//!
//! `metal::Queue::as_raw` requires **wgpu-hal 29.0.4 or newer**: it was dropped
//! in 29.0.0 and restored in 29.0.4 ("removed without good reason in v29", per
//! the changelog), and in between there was no way to reach the queue at all.
//! The floor is pinned by an explicit `wgpu-hal` dependency in `Cargo.toml`,
//! because the version of the `wgpu` facade says nothing about which
//! `wgpu-hal` resolved underneath it.

use objc2::{Message, rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLCommandQueue, MTLDevice};
use skia_safe::{
    Image,
    gpu::{self, DirectContext, Mipmapped, SurfaceOrigin, backend_textures, mtl},
};

use super::{color_space, color_type, init};
use crate::SkiaBackendError;

/// The Metal objects a presenter needs to build a layer and a Skia context.
pub(crate) struct PlatformDevice {
    pub(crate) device: Retained<ProtocolObject<dyn MTLDevice>>,
    pub(crate) queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

pub(crate) struct Interop {
    platform: PlatformDevice,
}

impl Interop {
    pub(crate) const NAME: &'static str = "Metal";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::METAL;

    pub(crate) fn new(
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Self, SkiaBackendError> {
        // SAFETY: both guards are dropped before this returns, and both objects
        // are retained out of them first -- the device by the clone, the queue
        // by `retain`.
        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| init("the wgpu device is not Metal-backed"))?;
        let device = hal_device.raw_device().clone();
        drop(hal_device);

        let hal_queue = unsafe { queue.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| init("the wgpu queue is not Metal-backed"))?;
        let queue: Retained<ProtocolObject<dyn MTLCommandQueue>> = hal_queue.as_raw().retain();
        drop(hal_queue);

        Ok(Self {
            platform: PlatformDevice { device, queue },
        })
    }

    pub(crate) fn platform_device(&self) -> &PlatformDevice {
        &self.platform
    }

    pub(crate) fn borrow_image(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<Image, SkiaBackendError> {
        let (width, height) = (texture.width() as i32, texture.height() as i32);
        // SAFETY: the guard lives across the `TextureInfo` construction, which
        // retains the `MTLTexture` it is given.
        let hal_texture = unsafe { texture.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| init("the wgpu texture is not Metal-backed"))?;
        let handle: *const ProtocolObject<dyn objc2_metal::MTLTexture> = hal_texture.raw_handle();
        let info = unsafe { mtl::TextureInfo::new(handle as mtl::Handle) };

        // SAFETY: the returned `BackendTexture` only borrows the `MTLTexture`;
        // `WgpuImage` holds the wgpu texture that keeps it alive.
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
}
