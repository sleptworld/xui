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
//! With the `wgpu` feature, the GPU presenters are built on a device wgpu
//! opened rather than one they create themselves -- see
//! [`crate::wgpu_surface`]. Everything above stays exactly as described:
//! the layer, the swapchain, the present mode and the buffer count are still
//! each presenter's own. Only the device is shared, because that is all it
//! takes for Skia to composite a `wgpu::Texture` the application rendered.
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

/// What [`WindowPresenter::new`] produced: the presenter, the Skia context it
/// created (absent for the software path), and the shared wgpu device when one
/// was opened.
pub(crate) struct PresenterSetup {
    pub(crate) presenter: WindowPresenter,
    pub(crate) context: Option<DirectContext>,
    #[cfg(feature = "wgpu")]
    pub(crate) wgpu: Option<crate::wgpu_surface::WgpuHost>,
}

impl PresenterSetup {
    fn new(presenter: WindowPresenter, context: Option<DirectContext>) -> Self {
        Self {
            presenter,
            context,
            #[cfg(feature = "wgpu")]
            wgpu: None,
        }
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
    pub(crate) fn new(window: Arc<Window>) -> Result<PresenterSetup, SkiaBackendError> {
        if gpu_disabled() {
            return Ok(PresenterSetup::new(
                Self::Software(SoftwarePresenter::new(window)?),
                None,
            ));
        }
        #[cfg(feature = "wgpu")]
        if !crate::wgpu_surface::disabled() {
            match Self::new_shared(window.clone()) {
                Ok(setup) => return Ok(setup),
                // Falling through to a self-owned device rather than to
                // software: failing to open a shared device says nothing about
                // whether this machine can drive Metal/Vulkan/D3D at all. The
                // UI renders exactly as before -- only a canvas GPU painter
                // loses anything, and it draws nothing rather than failing.
                Err(error) => eprintln!(
                    "xui-skia: could not open a shared wgpu device, using a self-owned one; \
                     canvases with a GPU painter will not draw ({error})"
                ),
            }
        }
        match Self::new_gpu(window.clone()) {
            Ok((presenter, context)) => Ok(PresenterSetup::new(presenter, Some(context))),
            Err(error) => {
                eprintln!(
                    "xui-skia: no GPU presentation available, falling back to software rendering ({error})"
                );
                Ok(PresenterSetup::new(
                    Self::Software(SoftwarePresenter::new(window)?),
                    None,
                ))
            }
        }
    }

    /// Opens a wgpu device and builds this platform's presenter on it.
    ///
    /// The presenter is the same one `new_gpu` would build and behaves
    /// identically; it just did not create the device it renders through.
    #[cfg(feature = "wgpu")]
    fn new_shared(window: Arc<Window>) -> Result<PresenterSetup, SkiaBackendError> {
        let host = crate::wgpu_surface::WgpuHost::new()?;
        let platform = host.platform_device();

        #[cfg(target_os = "macos")]
        let (presenter, context) = {
            let (presenter, context) = crate::metal::MetalPresenter::with_device(
                window,
                platform.device.clone(),
                platform.queue.clone(),
            )?;
            (Self::Metal(presenter), context)
        };
        #[cfg(target_os = "linux")]
        let (presenter, context) = {
            let (presenter, context) = crate::vulkan::VulkanPresenter::with_device(
                window,
                platform.entry.clone(),
                platform.instance.clone(),
                platform.physical_device,
                platform.device.clone(),
                platform.queue,
                platform.queue_family,
            )?;
            (Self::Vulkan(Box::new(presenter)), context)
        };
        #[cfg(target_os = "windows")]
        let (presenter, context) = {
            let (presenter, context) = crate::d3d::Direct3DPresenter::with_device(
                window,
                platform.adapter.clone(),
                platform.device.clone(),
                platform.queue.clone(),
            )?;
            (Self::Direct3D(presenter), context)
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let (presenter, context) = {
            let _ = (window, platform);
            return Err(SkiaBackendError::NoGpuPresenter);
        };

        Ok(PresenterSetup {
            presenter,
            context: Some(context),
            wgpu: Some(host),
        })
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
