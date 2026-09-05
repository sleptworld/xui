//! Direct3D 12 interop between wgpu and Skia.
//!
//! The best-behaved of the three backends here, for one reason:
//! `wgpu-hal`'s `dx12::Device` exposes *both* `raw_device()` and
//! `raw_queue()`, so Skia and wgpu genuinely submit to the same
//! `ID3D12CommandQueue`. Submission order alone then orders their work, with no
//! second queue (as on Metal) and no CPU stall.
//!
//! D3D12 is also the only backend Skia's stock Windows binaries offer -- their
//! Vulkan backend is not built in -- which is why [`Interop::BACKEND`] pins
//! wgpu to `DX12` rather than letting it pick.
//!
//! # Resource states
//!
//! wgpu never touches the swapchain buffer on this path, so its tracker holds
//! no state for it and `present()` emits no transition. The buffer comes out of
//! `get_current_texture` in `PRESENT`, which is what it is declared as here,
//! and [`Interop::finish_frame`] flushes with `BackendSurfaceAccess::Present`
//! to put it back before wgpu presents.
//!
//! **Unverified on this machine** -- written against the wgpu-hal 29 and
//! skia-safe 0.99 sources but not compiled on Windows. See the branch notes.

use skia_safe::{
    Image, Surface,
    gpu::{self, DirectContext, SurfaceOrigin, backend_render_targets, backend_textures, d3d},
};
use windows::{
    Win32::Graphics::Dxgi::{
        Common::{
            DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
        },
        IDXGIAdapter1,
    },
    core::Interface,
};

use super::{color_space, color_type, init, present_error};
use crate::SkiaBackendError;

pub(crate) struct Interop {
    _private: (),
}

impl Interop {
    pub(crate) const NAME: &'static str = "Direct3D 12";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::DX12;

    pub(crate) fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Result<(Self, DirectContext), SkiaBackendError> {
        // SAFETY: the COM interfaces are cloned (AddRef'd) out of the hal
        // guards before they are dropped, and nothing is released here.
        let hal_adapter = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| init("the wgpu adapter is not Direct3D 12-backed"))?;
        let dxgi_adapter: IDXGIAdapter1 =
            hal_adapter.raw_adapter().as_raw().cast().map_err(|error| {
                init(format!(
                    "the wgpu DXGI adapter is not an IDXGIAdapter1: {error}"
                ))
            })?;
        drop(hal_adapter);

        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| init("the wgpu device is not Direct3D 12-backed"))?;
        let d3d_device = hal_device.raw_device().clone();
        // wgpu's own queue: Skia submits into it, so submission order is the
        // only synchronization the two need.
        let d3d_queue = hal_device.raw_queue().clone();
        drop(hal_device);

        let backend = d3d::BackendContext {
            adapter: dxgi_adapter,
            device: d3d_device,
            queue: d3d_queue,
            memory_allocator: None,
            protected_context: gpu::Protected::No,
        };
        let context =
            unsafe { gpu::direct_contexts::make_d3d(&backend, None) }.ok_or_else(|| {
                init("Skia could not create a Ganesh Direct3D context on the wgpu device")
            })?;

        Ok((Self { _private: () }, context))
    }

    pub(crate) fn wrap_render_target(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<Surface, SkiaBackendError> {
        let (width, height) = (texture.width() as i32, texture.height() as i32);
        let info = resource_info(
            texture,
            windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATE_PRESENT,
        )?;
        let target = backend_render_targets::make_d3d((width, height), &info);
        gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            SurfaceOrigin::TopLeft,
            color_type(texture.format())?,
            color_space(texture.format()),
            None,
        )
        .ok_or_else(|| present_error("Skia could not wrap the wgpu swapchain back buffer"))
    }

    pub(crate) fn borrow_image(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<Image, SkiaBackendError> {
        let (width, height) = (texture.width() as i32, texture.height() as i32);
        // The caller rendered into this texture with wgpu, which leaves a
        // sampled render target in the shader-resource state.
        let info = resource_info(
            texture,
            windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        )?;
        let backend =
            backend_textures::make_d3d((width, height), &info, "xui-skia imported wgpu texture");
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
                "Skia could not borrow the imported Direct3D resource".into(),
            )
        })
    }

    /// Flushes the frame and returns the back buffer to `PRESENT`.
    ///
    /// No CPU sync: Skia and wgpu share one queue here, so `present()` is
    /// ordered after this submission by the queue itself.
    pub(crate) fn finish_frame(&self, context: &mut DirectContext, surface: &mut Surface) {
        context.flush_surface_with_access(
            surface,
            skia_safe::surfaces::BackendSurfaceAccess::Present,
            &gpu::FlushInfo::default(),
        );
        context.submit(gpu::SyncCpu::No);
    }
}

fn resource_info(
    texture: &wgpu::Texture,
    resource_state: windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATES,
) -> Result<d3d::TextureResourceInfo, SkiaBackendError> {
    // SAFETY: the guard lives across the clone of the resource interface,
    // which AddRefs it; nothing is released here.
    let hal_texture = unsafe { texture.as_hal::<wgpu::hal::api::Dx12>() }
        .ok_or_else(|| init("the wgpu texture is not Direct3D 12-backed"))?;
    let resource = unsafe { hal_texture.raw_resource() }.clone();
    Ok(d3d::TextureResourceInfo {
        resource,
        alloc: None,
        resource_state,
        format: dxgi_format(texture.format())?,
        sample_count: texture.sample_count(),
        level_count: texture.mip_level_count(),
        sample_quality_pattern: 0,
        protected: gpu::Protected::No,
    })
}

fn dxgi_format(format: wgpu::TextureFormat) -> Result<DXGI_FORMAT, SkiaBackendError> {
    use wgpu::TextureFormat as Wgpu;
    Ok(match format {
        Wgpu::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
        Wgpu::Bgra8UnormSrgb => DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        Wgpu::Rgba8Unorm => DXGI_FORMAT_R8G8B8A8_UNORM,
        Wgpu::Rgba8UnormSrgb => DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        Wgpu::Rgba16Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
        Wgpu::Rgb10a2Unorm => DXGI_FORMAT_R10G10B10A2_UNORM,
        other => {
            return Err(init(format!(
                "no DXGI format mapping for the wgpu texture format {other:?}"
            )));
        }
    })
}
