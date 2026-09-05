//! Vulkan interop between wgpu and Skia.
//!
//! Everything Skia's `vk::BackendContext` needs is reachable from
//! `wgpu-hal`: the instance and its `ash::Entry` (so `get_proc` can resolve
//! function pointers through the same loader wgpu used), the physical device,
//! the device, and the queue with its family index.
//!
//! # Version pinning
//!
//! The API version handed to Skia is pinned to 1.3, matching what wgpu 29
//! targets. This is the same guard Slint uses, and it exists because Skia will
//! otherwise probe for and use features of a newer API version that wgpu never
//! enabled on this device. For the same reason no extensions are declared:
//! Skia adapts to what it is told is available, so under-declaring costs a few
//! fast paths, while over-declaring is a crash.
//!
//! # Image layouts
//!
//! wgpu never touches the swapchain image on this path -- Skia renders into it
//! directly -- so wgpu's resource tracker has no state for it and `present()`
//! issues no barrier of its own. Both ends of the frame are therefore explicit
//! here: the image is wrapped as `UNDEFINED` (its contents after acquire are
//! undefined anyway, so this discards rather than preserves), and
//! [`Interop::finish_frame`] flushes with `BackendSurfaceAccess::Present`,
//! which leaves it in `PRESENT_SRC_KHR`.
//!
//! Note the consequence for damage tracking: this presenter does not preserve
//! the previous frame's contents, so a partial repaint that assumes it can read
//! back what was there will be wrong. The native Vulkan presenter in
//! [`crate::vulkan`] is the one that preserves.
//!
//! **Unverified on this machine** -- written against the wgpu-hal 29 and
//! skia-safe 0.99 sources but not compiled on Linux. See the branch notes.

use std::ffi::c_void;

use ash::vk::Handle;
use skia_safe::{
    Image, Surface,
    gpu::{
        self, DirectContext, SurfaceOrigin, backend_render_targets, backend_textures, vk as skia_vk,
    },
};

use super::{color_space, color_type, init, present_error};
use crate::SkiaBackendError;

pub(crate) struct Interop {
    queue_family: u32,
}

impl Interop {
    pub(crate) const NAME: &'static str = "Vulkan";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::VULKAN;

    pub(crate) fn new(
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(Self, DirectContext), SkiaBackendError> {
        // SAFETY: the handles are cloned out (`ash::Entry`, `ash::Instance` and
        // `ash::Device` are all reference-counted loaders, not owners) before
        // the hal guards are dropped, and none of them is destroyed here.
        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| init("the wgpu device is not Vulkan-backed"))?;
        let entry = hal_device.shared_instance().entry().clone();
        let instance = hal_device.shared_instance().raw_instance().clone();
        let physical_device = hal_device.raw_physical_device();
        let raw_device = hal_device.raw_device().handle();
        let queue_family = hal_device.queue_family_index();
        drop(hal_device);

        let hal_queue = unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| init("the wgpu queue is not Vulkan-backed"))?;
        let raw_queue = hal_queue.as_raw();
        drop(hal_queue);

        // Skia resolves its own entry points through this closure, consulted
        // only while `make_vulkan` builds the context.
        let get_proc = |of: skia_vk::GetProcOf| -> *const c_void {
            match of {
                skia_vk::GetProcOf::Instance(raw_instance, name) => {
                    let handle = ash::vk::Instance::from_raw(raw_instance as usize as u64);
                    unsafe { entry.get_instance_proc_addr(handle, name) }
                        .map_or(std::ptr::null(), |proc| proc as *const c_void)
                }
                skia_vk::GetProcOf::Device(device, name) => {
                    let handle = ash::vk::Device::from_raw(device as usize as u64);
                    unsafe { (instance.fp_v1_0().get_device_proc_addr)(handle, name) }
                        .map_or(std::ptr::null(), |proc| proc as *const c_void)
                }
            }
        };

        let backend = unsafe {
            skia_vk::BackendContext::new_builder(
                instance.handle().as_raw() as usize as skia_vk::Instance,
                physical_device.as_raw() as usize as skia_vk::PhysicalDevice,
                raw_device.as_raw() as usize as skia_vk::Device,
                (
                    raw_queue.as_raw() as usize as skia_vk::Queue,
                    queue_family as usize,
                ),
                &get_proc,
                Some(skia_vk::Version::new(1, 3, 0)),
            )
            .with_extensions(&[], &[])
            .build()
        };

        let context = gpu::direct_contexts::make_vulkan(&backend, None).ok_or_else(|| {
            init("Skia could not create a Ganesh Vulkan context on the wgpu device")
        })?;

        Ok((Self { queue_family }, context))
    }

    pub(crate) fn wrap_render_target(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<Surface, SkiaBackendError> {
        let (width, height) = (texture.width() as i32, texture.height() as i32);
        // The same flags the native presenter declares in `crate::vulkan`, and
        // the same ones `WgpuPresenter::new` asked the surface for.
        let info = self.image_info(
            texture,
            skia_vk::ImageLayout::UNDEFINED,
            ash::vk::ImageUsageFlags::COLOR_ATTACHMENT
                | ash::vk::ImageUsageFlags::TRANSFER_SRC
                | ash::vk::ImageUsageFlags::TRANSFER_DST,
        )?;
        let target = backend_render_targets::make_vk((width, height), &info);
        gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            SurfaceOrigin::TopLeft,
            color_type(texture.format())?,
            color_space(texture.format()),
            None,
        )
        .ok_or_else(|| present_error("Skia could not wrap the wgpu swapchain image"))
    }

    pub(crate) fn borrow_image(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<Image, SkiaBackendError> {
        let (width, height) = (texture.width() as i32, texture.height() as i32);
        // wgpu transitions lazily, by last use. The contract on
        // `WgpuPresenter::borrow_image` is that the caller *rendered into* this
        // texture, so wgpu left it as a colour attachment -- declaring
        // shader-read here would describe a transition that never happened.
        // Skia transitions to shader-read itself from what it is told.
        let info = self.image_info(
            texture,
            skia_vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ash::vk::ImageUsageFlags::COLOR_ATTACHMENT | ash::vk::ImageUsageFlags::SAMPLED,
        )?;
        // SAFETY: the `VkImage` is only borrowed; the contract on
        // `WgpuPresenter::borrow_image` requires the wgpu texture to outlive
        // the image built from it.
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

    /// Flushes the frame and leaves the image in `PRESENT_SRC_KHR`, which is
    /// what wgpu's `present()` assumes -- it never touched the image, so it
    /// emits no transition of its own.
    pub(crate) fn finish_frame(&self, context: &mut DirectContext, surface: &mut Surface) {
        context.flush_surface_with_access(
            surface,
            skia_safe::surfaces::BackendSurfaceAccess::Present,
            &gpu::FlushInfo::default(),
        );
        context.submit(gpu::SyncCpu::No);
    }

    fn image_info(
        &self,
        texture: &wgpu::Texture,
        layout: skia_vk::ImageLayout,
        usage: ash::vk::ImageUsageFlags,
    ) -> Result<skia_vk::ImageInfo, SkiaBackendError> {
        // SAFETY: the guard lives across the read of the raw handle, and the
        // image is not destroyed here.
        let hal_texture = unsafe { texture.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| init("the wgpu texture is not Vulkan-backed"))?;
        let image = unsafe { hal_texture.raw_handle() };
        let mut info = unsafe {
            skia_vk::ImageInfo::new(
                image.as_raw() as usize as skia_vk::Image,
                skia_vk::Alloc::default(),
                skia_vk::ImageTiling::OPTIMAL,
                layout,
                vk_format(texture.format())?,
                texture.mip_level_count(),
                self.queue_family,
                None,
                None,
                None,
            )
        };
        info.image_usage_flags = usage.as_raw();
        info.sample_count = texture.sample_count();
        Ok(info)
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
            return Err(init(format!(
                "no Vulkan format mapping for the wgpu texture format {other:?}"
            )));
        }
    })
}
