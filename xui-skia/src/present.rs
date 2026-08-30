//! Window presentation backends.
//!
//! Every frame is rendered into a `skia_safe::Surface` and then handed to a
//! [`WindowPresenter`], which owns the platform swapchain:
//!
//! - macOS — a `CAMetalLayer` driven through [`crate::metal::MetalPresenter`].
//! - Windows — a DXGI flip-model swapchain on Direct3D 12, through
//!   [`crate::d3d::Direct3DPresenter`].
//! - Linux — a `VK_KHR_swapchain` on Vulkan, through
//!   [`crate::vulkan::VulkanPresenter`].
//! - Every platform — a CPU `softbuffer` blit ([`SoftwarePresenter`]).
//!
//! The GPU presenter is selected at runtime and falls back to the software one
//! when initialization fails, which is the common case in VMs, remote sessions
//! and CI containers with no usable driver. Set `XUI_SKIA_GPU=0` to force the
//! software path.

use std::sync::Arc;

use skia_safe::{Surface, gpu::DirectContext};
use softbuffer::{Context, Surface as SoftSurface};
use winit::window::Window;

use crate::SkiaBackendError;

/// CPU presentation: Skia renders into a raster surface and the damaged rows
/// are blitted into the window through `softbuffer`.
pub(crate) struct SoftwarePresenter {
    _window: Arc<Window>,
    _context: Context<Arc<Window>>,
    pub(crate) surface: SoftSurface<Arc<Window>, Arc<Window>>,
}

impl SoftwarePresenter {
    fn new(window: Arc<Window>) -> Result<Self, SkiaBackendError> {
        let context = Context::new(window.clone())?;
        let surface = SoftSurface::new(&context, window.clone())?;
        Ok(Self {
            _window: window,
            _context: context,
            surface,
        })
    }
}

pub(crate) enum WindowPresenter {
    #[cfg(target_os = "macos")]
    Metal(crate::metal::MetalPresenter),
    #[cfg(target_os = "windows")]
    Direct3D(crate::d3d::Direct3DPresenter),
    #[cfg(target_os = "linux")]
    Vulkan(Box<crate::vulkan::VulkanPresenter>),
    Software(SoftwarePresenter),
}

impl WindowPresenter {
    /// Creates the best presenter this platform offers, falling back to the
    /// software blit when the GPU one cannot be brought up.
    pub(crate) fn new(
        window: Arc<Window>,
    ) -> Result<(Self, Option<DirectContext>), SkiaBackendError> {
        if gpu_disabled() {
            return Ok((Self::Software(SoftwarePresenter::new(window)?), None));
        }
        match Self::new_gpu(window.clone()) {
            Ok((presenter, context)) => Ok((presenter, Some(context))),
            Err(error) => {
                eprintln!(
                    "xui-skia: no GPU presentation available, falling back to software rendering ({error})"
                );
                Ok((Self::Software(SoftwarePresenter::new(window)?), None))
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn new_gpu(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        let (presenter, context) = crate::metal::MetalPresenter::new(window)?;
        Ok((Self::Metal(presenter), context))
    }

    #[cfg(target_os = "windows")]
    fn new_gpu(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        let (presenter, context) = crate::d3d::Direct3DPresenter::new(window)?;
        Ok((Self::Direct3D(presenter), context))
    }

    #[cfg(target_os = "linux")]
    fn new_gpu(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        let (presenter, context) = crate::vulkan::VulkanPresenter::new(window)?;
        Ok((Self::Vulkan(Box::new(presenter)), context))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn new_gpu(_window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        Err(SkiaBackendError::NoGpuPresenter)
    }

    /// Resizes the swapchain. A no-op for the software presenter, which sizes
    /// its buffer while presenting.
    ///
    /// `context` is needed because rebuilding a swapchain releases its images,
    /// and Skia keeps wrapped render targets in its resource cache after the
    /// surface wrapping them is dropped. Those cached references have to go
    /// before the platform will hand the images back.
    pub(crate) fn resize(
        &mut self,
        context: Option<&mut DirectContext>,
        width: u32,
        height: u32,
    ) -> Result<(), SkiaBackendError> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Metal(presenter) => {
                // A `CAMetalLayer` hands out a fresh drawable a frame, so there
                // is nothing cached to release here.
                let _ = context;
                presenter.resize(width, height);
                Ok(())
            }
            #[cfg(target_os = "windows")]
            Self::Direct3D(presenter) => presenter.resize(context, width, height),
            #[cfg(target_os = "linux")]
            Self::Vulkan(presenter) => presenter.resize(context, width, height),
            Self::Software(_) => {
                let _ = (context, width, height);
                Ok(())
            }
        }
    }

    /// Acquires the next swapchain image and wraps it in a Skia surface.
    pub(crate) fn acquire_surface(
        &mut self,
        context: &mut DirectContext,
        width: u32,
        height: u32,
    ) -> Result<Surface, SkiaBackendError> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Metal(presenter) => presenter.acquire_surface(context, width, height),
            #[cfg(target_os = "windows")]
            Self::Direct3D(presenter) => presenter.acquire_surface(context, width, height),
            #[cfg(target_os = "linux")]
            Self::Vulkan(presenter) => presenter.acquire_surface(context, width, height),
            Self::Software(_) => {
                let _ = (context, width, height);
                Err(SkiaBackendError::InvalidFrame(
                    "the software presenter has no swapchain to acquire".into(),
                ))
            }
        }
    }

    /// Flushes `surface` and presents it. Takes the surface by value because
    /// every backend has to drop its reference to the swapchain image before
    /// handing it back to the presentation engine.
    pub(crate) fn present(
        &mut self,
        context: &mut DirectContext,
        surface: Surface,
    ) -> Result<(), SkiaBackendError> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Metal(presenter) => {
                let mut surface = surface;
                context.flush_and_submit_surface(&mut surface, None);
                drop(surface);
                presenter.present()
            }
            #[cfg(target_os = "windows")]
            Self::Direct3D(presenter) => presenter.present(context, surface),
            #[cfg(target_os = "linux")]
            Self::Vulkan(presenter) => presenter.present(context, surface),
            Self::Software(_) => {
                let _ = (context, surface);
                Err(SkiaBackendError::InvalidFrame(
                    "the software presenter has no swapchain to present".into(),
                ))
            }
        }
    }

    pub(crate) fn software_mut(&mut self) -> Option<&mut SoftwarePresenter> {
        match self {
            Self::Software(presenter) => Some(presenter),
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            _ => None,
        }
    }
}

fn gpu_disabled() -> bool {
    matches!(
        std::env::var("XUI_SKIA_GPU").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}
