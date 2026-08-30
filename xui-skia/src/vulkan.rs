//! Vulkan presentation for Linux (X11 and Wayland).
//!
//! Owns the `VkInstance`/`VkDevice`/`VK_KHR_swapchain` that Skia's Ganesh
//! Vulkan backend renders through. Each frame acquires a swapchain image,
//! wraps it in a `skia_safe::Surface`, and presents it after Skia flushes.
//!
//! ## Synchronization
//!
//! `skia-safe` 0.99 exposes no way to construct a `BackendSemaphore` or to put
//! signal semaphores into a `GrFlushInfo` (the fields on `gpu::FlushInfo` are
//! private and the crate carries a `TODO` about wrapping them safely). So this
//! presenter synchronizes on the CPU instead of with semaphores:
//!
//! - the image is acquired with a *fence*, waited on before Skia draws, so the
//!   presentation engine is provably done with the image;
//! - Skia's submit uses `SyncCpu::Yes`, so rendering is provably complete
//!   before `vkQueuePresentKHR` runs with no wait semaphores.
//!
//! That is correct but costs two CPU/GPU round trips a frame. Replacing both
//! with semaphores is a drop-in change once skia-safe exposes the API.

use std::{ffi::c_void, sync::Arc};

use ash::{
    Device, Entry, Instance, khr,
    vk::{self, Handle},
};
use skia_safe::{
    ColorSpace, ColorType, Surface,
    gpu::{
        self, DirectContext, SurfaceOrigin, backend_render_targets,
        vk::{self as skia_vk, mutable_texture_states},
    },
};
use winit::{
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::Window,
};

use crate::SkiaBackendError;

/// Enough swapchain images to keep one in flight while another is on screen,
/// clamped later to what the surface actually allows.
const PREFERRED_IMAGE_COUNT: u32 = 3;

const IMAGE_USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::COLOR_ATTACHMENT.as_raw()
        | vk::ImageUsageFlags::TRANSFER_SRC.as_raw()
        | vk::ImageUsageFlags::TRANSFER_DST.as_raw(),
);

fn init(message: impl Into<String>) -> SkiaBackendError {
    SkiaBackendError::VulkanInitialization(message.into())
}

fn present_error(message: impl Into<String>) -> SkiaBackendError {
    SkiaBackendError::VulkanPresentation(message.into())
}

pub(crate) struct VulkanPresenter {
    // Dropped last-to-first: the fields below borrow from these handles, and
    // `Drop` tears them down explicitly in the right order.
    _window: Arc<Window>,
    entry: Entry,
    instance: Instance,
    surface_ext: khr::surface::Instance,
    swapchain_ext: khr::swapchain::Device,
    device: Device,
    physical_device: vk::PhysicalDevice,
    queue: vk::Queue,
    queue_family: u32,
    surface: vk::SurfaceKHR,
    surface_format: vk::SurfaceFormatKHR,
    present_mode: vk::PresentModeKHR,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    /// The extent the swapchain was actually created with. A compositor may
    /// pin this to the window's real size and ignore what we asked for.
    extent: vk::Extent2D,
    /// The size `resize` was last called with. Compared against instead of
    /// `extent`, so that a pinned extent does not look like a size change and
    /// rebuild the swapchain on every frame.
    requested: (u32, u32),
    acquire_fence: vk::Fence,
    /// Index of the image handed out by `acquire_surface`, cleared by `present`.
    acquired: Option<u32>,
}

impl VulkanPresenter {
    pub(crate) fn new(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        let entry = unsafe { Entry::load() }
            .map_err(|error| init(format!("could not load the Vulkan loader: {error}")))?;

        let display_handle = window
            .display_handle()
            .map_err(|error| init(error.to_string()))?
            .as_raw();
        let window_handle = window
            .window_handle()
            .map_err(|error| init(error.to_string()))?
            .as_raw();

        let extensions = ash_window::enumerate_required_extensions(display_handle)
            .map_err(|error| {
                init(format!(
                    "no Vulkan surface extension for this display: {error}"
                ))
            })?
            .to_vec();

        // Skia requires Vulkan 1.1 as its minimum API version.
        let application_info = vk::ApplicationInfo::default()
            .application_name(c"xui")
            .api_version(vk::API_VERSION_1_1);
        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&application_info)
            .enabled_extension_names(&extensions);
        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|error| init(format!("could not create a Vulkan instance: {error}")))?;

        let surface_ext = khr::surface::Instance::new(&entry, &instance);
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)
        }
        .map_err(|error| init(format!("could not create a Vulkan surface: {error}")))?;

        let (physical_device, queue_family) =
            match select_physical_device(&instance, &surface_ext, surface) {
                Ok(selection) => selection,
                Err(error) => {
                    unsafe { surface_ext.destroy_surface(surface, None) };
                    unsafe { instance.destroy_instance(None) };
                    return Err(error);
                }
            };

        let device_extensions = [khr::swapchain::NAME.as_ptr()];
        let priorities = [1.0_f32];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities)];
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&device_extensions);
        let device = match unsafe { instance.create_device(physical_device, &device_info, None) } {
            Ok(device) => device,
            Err(error) => {
                unsafe { surface_ext.destroy_surface(surface, None) };
                unsafe { instance.destroy_instance(None) };
                return Err(init(format!("could not create a Vulkan device: {error}")));
            }
        };

        let swapchain_ext = khr::swapchain::Device::new(&instance, &device);
        let queue = unsafe { device.get_device_queue(queue_family, 0) };

        let surface_format = select_surface_format(&surface_ext, physical_device, surface)?;
        let present_mode = select_present_mode(&surface_ext, physical_device, surface)?;

        let acquire_fence =
            unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }
                .map_err(|error| init(format!("could not create the acquire fence: {error}")))?;

        let context = make_direct_context(
            &entry,
            &instance,
            physical_device,
            &device,
            queue,
            queue_family,
        )?;

        let size = window.inner_size();
        let mut presenter = Self {
            _window: window,
            entry,
            instance,
            surface_ext,
            swapchain_ext,
            device,
            physical_device,
            queue,
            queue_family,
            surface,
            surface_format,
            present_mode,
            swapchain: vk::SwapchainKHR::null(),
            images: Vec::new(),
            extent: vk::Extent2D::default(),
            requested: (0, 0),
            acquire_fence,
            acquired: None,
        };
        presenter.recreate_swapchain(size.width.max(1), size.height.max(1))?;
        Ok((presenter, context))
    }

    pub(crate) fn resize(
        &mut self,
        context: Option<&mut DirectContext>,
        width: u32,
        height: u32,
    ) -> Result<(), SkiaBackendError> {
        if self.requested == (width, height) && self.swapchain != vk::SwapchainKHR::null() {
            return Ok(());
        }
        // The old swapchain's images are about to be destroyed. Skia caches the
        // render targets wrapping them, so those have to go first, or the cache
        // is left holding dead `VkImage` handles that a recycled handle could
        // later collide with.
        if let Some(context) = context {
            context.flush_submit_and_sync_cpu();
            context.free_gpu_resources();
        }
        self.recreate_swapchain(width, height)
    }

    pub(crate) fn acquire_surface(
        &mut self,
        context: &mut DirectContext,
        width: u32,
        height: u32,
    ) -> Result<Surface, SkiaBackendError> {
        self.resize(Some(context), width, height)?;

        // Two attempts: a swapchain can go out of date between the resize above
        // and the acquire below (a compositor-driven resize, a mode change).
        let index = match self.acquire_index()? {
            Some(index) => index,
            None => {
                context.flush_submit_and_sync_cpu();
                context.free_gpu_resources();
                self.recreate_swapchain(width, height)?;
                self.acquire_index()?
                    .ok_or_else(|| present_error("the swapchain went out of date twice in a row"))?
            }
        };

        let image = self.images[index as usize];
        let mut image_info = unsafe {
            skia_vk::ImageInfo::new(
                image.as_raw() as usize as skia_vk::Image,
                skia_vk::Alloc::default(),
                skia_vk::ImageTiling::OPTIMAL,
                // Skia is free to discard the contents: every frame that reaches
                // the GPU path redraws the whole surface (see `SkiaBackend::submit`,
                // which widens damage to the full frame whenever a GPU context
                // is present).
                skia_vk::ImageLayout::UNDEFINED,
                skia_format(self.surface_format.format)?,
                1,
                self.queue_family,
                None,
                None,
                None,
            )
        };
        image_info.image_usage_flags = IMAGE_USAGE.as_raw();
        image_info.sample_count = 1;

        let target = backend_render_targets::make_vk(
            (self.extent.width as i32, self.extent.height as i32),
            &image_info,
        );
        let surface = gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            SurfaceOrigin::TopLeft,
            skia_color_type(self.surface_format.format)?,
            ColorSpace::new_srgb(),
            None,
        )
        .ok_or_else(|| present_error("Skia could not wrap the acquired swapchain image"))?;

        self.acquired = Some(index);
        Ok(surface)
    }

    pub(crate) fn present(
        &mut self,
        context: &mut DirectContext,
        surface: Surface,
    ) -> Result<(), SkiaBackendError> {
        let index = self
            .acquired
            .take()
            .ok_or_else(|| present_error("no acquired swapchain image to present"))?;

        let mut surface = surface;
        let present_state = mutable_texture_states::new_vulkan(
            skia_vk::ImageLayout::PRESENT_SRC_KHR,
            self.queue_family,
        );
        context.flush_surface_with_texture_state(
            &mut surface,
            &gpu::FlushInfo::default(),
            Some(&present_state),
        );
        // `SyncCpu::Yes` stands in for the signal semaphore we cannot hand to
        // Skia; see the module docs.
        context.submit(gpu::SyncCpu::Yes);
        drop(surface);

        let swapchains = [self.swapchain];
        let indices = [index];
        let present_info = vk::PresentInfoKHR::default()
            .swapchains(&swapchains)
            .image_indices(&indices);
        match unsafe { self.swapchain_ext.queue_present(self.queue, &present_info) } {
            Ok(_) => Ok(()),
            // The next `acquire_surface` rebuilds the swapchain; the frame that
            // was just drawn is simply not shown.
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                self.invalidate();
                Ok(())
            }
            Err(error) => Err(present_error(format!("vkQueuePresentKHR failed: {error}"))),
        }
    }

    /// Returns `Ok(None)` when the swapchain is out of date and has to be
    /// rebuilt before another attempt.
    fn acquire_index(&mut self) -> Result<Option<u32>, SkiaBackendError> {
        unsafe { self.device.reset_fences(&[self.acquire_fence]) }.map_err(|error| {
            present_error(format!("could not reset the acquire fence: {error}"))
        })?;

        let acquired = unsafe {
            self.swapchain_ext.acquire_next_image(
                self.swapchain,
                u64::MAX,
                vk::Semaphore::null(),
                self.acquire_fence,
            )
        };
        let index = match acquired {
            Ok((index, _suboptimal)) => index,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(None),
            Err(error) => {
                return Err(present_error(format!(
                    "vkAcquireNextImageKHR failed: {error}"
                )));
            }
        };

        // Wait on the CPU: Skia cannot be handed the acquire semaphore, so the
        // image has to be provably free before it starts recording into it.
        unsafe {
            self.device
                .wait_for_fences(&[self.acquire_fence], true, u64::MAX)
        }
        .map_err(|error| present_error(format!("waiting for the acquire fence failed: {error}")))?;
        Ok(Some(index))
    }

    fn recreate_swapchain(&mut self, width: u32, height: u32) -> Result<(), SkiaBackendError> {
        let capabilities = unsafe {
            self.surface_ext
                .get_physical_device_surface_capabilities(self.physical_device, self.surface)
        }
        .map_err(|error| init(format!("could not query surface capabilities: {error}")))?;

        // `u32::MAX` means "the surface takes its size from the swapchain".
        let extent = if capabilities.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: width.clamp(
                    capabilities.min_image_extent.width,
                    capabilities.max_image_extent.width,
                ),
                height: height.clamp(
                    capabilities.min_image_extent.height,
                    capabilities.max_image_extent.height,
                ),
            }
        } else {
            capabilities.current_extent
        };
        if extent.width == 0 || extent.height == 0 {
            return Err(present_error("the window surface has a zero-sized extent"));
        }

        let mut image_count = PREFERRED_IMAGE_COUNT.max(capabilities.min_image_count);
        if capabilities.max_image_count > 0 {
            image_count = image_count.min(capabilities.max_image_count);
        }

        if !capabilities.supported_usage_flags.contains(IMAGE_USAGE) {
            return Err(init(
                "the window surface does not support the image usage Skia needs",
            ));
        }

        let old_swapchain = self.swapchain;
        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(self.surface_format.format)
            .image_color_space(self.surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(IMAGE_USAGE)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(
                if capabilities
                    .supported_transforms
                    .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
                {
                    vk::SurfaceTransformFlagsKHR::IDENTITY
                } else {
                    capabilities.current_transform
                },
            )
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(self.present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain);

        // Everything below runs on images the old swapchain may still own.
        unsafe { self.device.device_wait_idle() }
            .map_err(|error| init(format!("vkDeviceWaitIdle failed: {error}")))?;

        let swapchain = unsafe { self.swapchain_ext.create_swapchain(&create_info, None) }
            .map_err(|error| init(format!("could not create a swapchain: {error}")))?;
        if old_swapchain != vk::SwapchainKHR::null() {
            unsafe { self.swapchain_ext.destroy_swapchain(old_swapchain, None) };
        }

        self.images = unsafe { self.swapchain_ext.get_swapchain_images(swapchain) }
            .map_err(|error| init(format!("could not read the swapchain images: {error}")))?;
        self.swapchain = swapchain;
        self.extent = extent;
        self.requested = (width, height);
        self.acquired = None;
        Ok(())
    }

    /// Marks the swapchain as needing a rebuild before the next acquire.
    fn invalidate(&mut self) {
        self.requested = (0, 0);
        self.acquired = None;
    }
}

impl Drop for VulkanPresenter {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_fence(self.acquire_fence, None);
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_ext.destroy_swapchain(self.swapchain, None);
            }
            self.device.destroy_device(None);
            self.surface_ext.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

fn make_direct_context(
    entry: &Entry,
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    device: &Device,
    queue: vk::Queue,
    queue_family: u32,
) -> Result<DirectContext, SkiaBackendError> {
    // Skia resolves its own Vulkan entry points through this closure. It is
    // only consulted while `make_vulkan` builds the context, so a borrow that
    // lives for this function is enough.
    let get_proc = |of: skia_vk::GetProcOf| -> *const c_void {
        match of {
            skia_vk::GetProcOf::Instance(raw_instance, name) => {
                let handle = vk::Instance::from_raw(raw_instance as usize as u64);
                unsafe { entry.get_instance_proc_addr(handle, name) }
                    .map_or(std::ptr::null(), |proc| proc as *const c_void)
            }
            skia_vk::GetProcOf::Device(raw_device, name) => {
                let handle = vk::Device::from_raw(raw_device as usize as u64);
                unsafe { (instance.fp_v1_0().get_device_proc_addr)(handle, name) }
                    .map_or(std::ptr::null(), |proc| proc as *const c_void)
            }
        }
    };

    let backend = unsafe {
        skia_vk::BackendContext::new_builder(
            instance.handle().as_raw() as usize as skia_vk::Instance,
            physical_device.as_raw() as usize as skia_vk::PhysicalDevice,
            device.handle().as_raw() as usize as skia_vk::Device,
            (
                queue.as_raw() as usize as skia_vk::Queue,
                queue_family as usize,
            ),
            &get_proc,
            Some(skia_vk::Version::new(1, 1, 0)),
        )
        .with_extensions(
            &["VK_KHR_surface"],
            &[khr::swapchain::NAME.to_str().unwrap()],
        )
        .build()
    };

    gpu::direct_contexts::make_vulkan(&backend, None)
        .ok_or_else(|| init("Skia could not create a Ganesh Vulkan context"))
}

fn select_physical_device(
    instance: &Instance,
    surface_ext: &khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, u32), SkiaBackendError> {
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|error| init(format!("could not enumerate Vulkan devices: {error}")))?;

    let mut best: Option<(vk::PhysicalDevice, u32, u32)> = None;
    for device in devices {
        if !supports_swapchain(instance, device) {
            continue;
        }
        let families = unsafe { instance.get_physical_device_queue_family_properties(device) };
        let Some(family) = families.iter().enumerate().position(|(index, family)| {
            family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                && unsafe {
                    surface_ext.get_physical_device_surface_support(device, index as u32, surface)
                }
                .unwrap_or(false)
        }) else {
            continue;
        };
        let properties = unsafe { instance.get_physical_device_properties(device) };
        let score = match properties.device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => 3,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
            vk::PhysicalDeviceType::VIRTUAL_GPU => 1,
            _ => 0,
        };
        if best.is_none_or(|(_, _, current)| score > current) {
            best = Some((device, family as u32, score));
        }
    }

    best.map(|(device, family, _)| (device, family))
        .ok_or_else(|| init("no Vulkan device can present to this window"))
}

fn supports_swapchain(instance: &Instance, device: vk::PhysicalDevice) -> bool {
    let Ok(extensions) = (unsafe { instance.enumerate_device_extension_properties(device) }) else {
        return false;
    };
    extensions.iter().any(|extension| {
        extension
            .extension_name_as_c_str()
            .is_ok_and(|name| name == khr::swapchain::NAME)
    })
}

fn select_surface_format(
    surface_ext: &khr::surface::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Result<vk::SurfaceFormatKHR, SkiaBackendError> {
    let formats =
        unsafe { surface_ext.get_physical_device_surface_formats(physical_device, surface) }
            .map_err(|error| init(format!("could not query surface formats: {error}")))?;

    // Only the two 8-bit BGRA/RGBA formats are wired through to a Skia
    // `ColorType` below, so a surface offering nothing else is unusable.
    formats
        .iter()
        .find(|format| {
            format.format == vk::Format::B8G8R8A8_UNORM
                && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .or_else(|| {
            formats.iter().find(|format| {
                format.format == vk::Format::R8G8B8A8_UNORM
                    && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
        })
        .copied()
        .ok_or_else(|| init("the window surface offers no 8-bit BGRA or RGBA format"))
}

fn select_present_mode(
    surface_ext: &khr::surface::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Result<vk::PresentModeKHR, SkiaBackendError> {
    let modes =
        unsafe { surface_ext.get_physical_device_surface_present_modes(physical_device, surface) }
            .map_err(|error| init(format!("could not query present modes: {error}")))?;
    // FIFO is the only mode Vulkan guarantees, and it is the vsync behaviour
    // the Metal path already has.
    Ok(if modes.contains(&vk::PresentModeKHR::FIFO) {
        vk::PresentModeKHR::FIFO
    } else {
        *modes
            .first()
            .ok_or_else(|| init("the window surface reports no present modes"))?
    })
}

fn skia_format(format: vk::Format) -> Result<skia_vk::Format, SkiaBackendError> {
    match format {
        vk::Format::B8G8R8A8_UNORM => Ok(skia_vk::Format::B8G8R8A8_UNORM),
        vk::Format::R8G8B8A8_UNORM => Ok(skia_vk::Format::R8G8B8A8_UNORM),
        other => Err(present_error(format!(
            "unsupported swapchain format {other:?}"
        ))),
    }
}

fn skia_color_type(format: vk::Format) -> Result<ColorType, SkiaBackendError> {
    match format {
        vk::Format::B8G8R8A8_UNORM => Ok(ColorType::BGRA8888),
        vk::Format::R8G8B8A8_UNORM => Ok(ColorType::RGBA8888),
        other => Err(present_error(format!(
            "unsupported swapchain format {other:?}"
        ))),
    }
}

// The handle casts above round-trip Vulkan handles through `usize`, which is
// only lossless where Skia's bindings represent them as pointers.
const _: () = assert!(size_of::<usize>() == size_of::<u64>());
