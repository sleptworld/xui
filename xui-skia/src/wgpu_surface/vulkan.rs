//! Vulkan handles behind a wgpu device.
//!
//! Everything [`crate::vulkan::VulkanPresenter::with_device`] needs is
//! reachable from `wgpu-hal`: the `ash::Entry` and `ash::Instance` (so the
//! presenter can create its own `VkSurfaceKHR` through the same loader), the
//! physical device, the device, and the queue with its family index.
//!
//! Two things make this work without wgpu owning any presentation:
//!
//! - wgpu-hal enables `VK_KHR_surface` and every platform surface extension on
//!   its instance unconditionally, so `ash_window::create_surface` succeeds on
//!   it.
//! - wgpu-hal requires `VK_KHR_swapchain` on every device it opens, also
//!   unconditionally, so the presenter can build its swapchain on wgpu's
//!   device.
//!
//! What is *not* guaranteed is that the queue family wgpu picked can present to
//! our window -- the adapter was chosen with no `compatible_surface` hint. The
//! presenter checks and refuses, so the caller falls back to a self-owned
//! device.
//!
//! **Unverified on this machine** -- written against the wgpu-hal 29.0.4 and
//! skia-safe 0.99 sources but not compiled on Linux.

use ash::vk::Handle;
use skia_safe::{
    Image,
    gpu::{self, DirectContext, SurfaceOrigin, backend_textures, vk as skia_vk},
};

use super::{color_space, color_type, init};
use crate::SkiaBackendError;

/// The Vulkan objects a presenter needs to build a surface, a swapchain and a
/// Skia context.
pub(crate) struct PlatformDevice {
    pub(crate) entry: ash::Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) physical_device: ash::vk::PhysicalDevice,
    pub(crate) device: ash::Device,
    pub(crate) queue: ash::vk::Queue,
    pub(crate) queue_family: u32,
}

pub(crate) struct Interop {
    platform: PlatformDevice,
}

impl Interop {
    pub(crate) const NAME: &'static str = "Vulkan";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::VULKAN;

    pub(crate) fn new(
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Self, SkiaBackendError> {
        // SAFETY: the handles are cloned out before the hal guards drop.
        // `ash::Entry`, `ash::Instance` and `ash::Device` are loaders holding
        // function pointers, not owners, so cloning them destroys nothing.
        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| init("the wgpu device is not Vulkan-backed"))?;
        let entry = hal_device.shared_instance().entry().clone();
        let instance = hal_device.shared_instance().raw_instance().clone();
        let physical_device = hal_device.raw_physical_device();
        let device = hal_device.raw_device().clone();
        let queue_family = hal_device.queue_family_index();
        drop(hal_device);

        let hal_queue = unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| init("the wgpu queue is not Vulkan-backed"))?;
        let queue = hal_queue.as_raw();
        drop(hal_queue);

        Ok(Self {
            platform: PlatformDevice {
                entry,
                instance,
                physical_device,
                device,
                queue,
                queue_family,
            },
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

        // SAFETY: the guard lives across the read of the raw handle, and the
        // image is not destroyed here.
        let hal_texture = unsafe { texture.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| init("the wgpu texture is not Vulkan-backed"))?;
        let image = unsafe { hal_texture.raw_handle() };

        // wgpu transitions lazily, by last use. The contract on
        // `WgpuHost::borrow_image` is that the caller *rendered into* this
        // texture, so wgpu left it as a colour attachment -- declaring
        // shader-read here would describe a transition that never happened.
        // Skia transitions there itself from what it is told.
        let mut info = unsafe {
            skia_vk::ImageInfo::new(
                image.as_raw() as usize as skia_vk::Image,
                skia_vk::Alloc::default(),
                skia_vk::ImageTiling::OPTIMAL,
                skia_vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk_format(texture.format())?,
                texture.mip_level_count(),
                self.platform.queue_family,
                None,
                None,
                None,
            )
        };
        info.image_usage_flags = (ash::vk::ImageUsageFlags::COLOR_ATTACHMENT
            | ash::vk::ImageUsageFlags::SAMPLED)
            .as_raw();
        info.sample_count = texture.sample_count();
        drop(hal_texture);

        // SAFETY: the `VkImage` is only borrowed; `WgpuImage` holds the wgpu
        // texture that keeps it alive.
        let backend = unsafe {
            backend_textures::make_vk((width, height), &info, "xui-skia imported wgpu texture")
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
                "Skia could not borrow the imported Vulkan image".into(),
            )
        })
    }
}

fn vk_format(format: wgpu::TextureFormat) -> Result<skia_vk::Format, SkiaBackendError> {
    use skia_vk::Format;
    use wgpu::TextureFormat as Wgpu;
    Ok(match format {
        Wgpu::Bgra8Unorm => Format::B8G8R8A8_UNORM,
        Wgpu::Bgra8UnormSrgb => Format::B8G8R8A8_SRGB,
        Wgpu::Rgba8Unorm => Format::R8G8B8A8_UNORM,
        Wgpu::Rgba8UnormSrgb => Format::R8G8B8A8_SRGB,
        Wgpu::Rgba16Float => Format::R16G16B16A16_SFLOAT,
        Wgpu::Rgb10a2Unorm => Format::A2B10G10R10_UNORM_PACK32,
        other => {
            return Err(SkiaBackendError::WgpuTextureImport(format!(
                "no Vulkan format mapping for the wgpu texture format {other:?}"
            )));
        }
    })
}
