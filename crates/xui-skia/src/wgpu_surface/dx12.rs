//! Direct3D 12 handles behind a wgpu device.
//!
//! The tidiest of the three: `wgpu-hal`'s `dx12::Device` exposes both
//! `raw_device()` and `raw_queue()`, so Skia and wgpu genuinely submit to the
//! same `ID3D12CommandQueue` and submission order alone orders their work.
//!
//! [`crate::d3d::Direct3DPresenter::with_device`] keeps the DXGI flip-model
//! swapchain, its three buffers and its fences; the DXGI factory is independent
//! of the device, and `CreateSwapChainForHwnd` takes the queue to present from,
//! which is now the shared one.
//!
//! D3D12 is also the only backend Skia's stock Windows binaries offer -- their
//! Vulkan backend is not built in -- which is why [`Interop::BACKEND`] pins
//! wgpu to `DX12` rather than letting it pick.
//!
//! **Unverified on this machine** -- written against the wgpu-hal 29.0.4 and
//! skia-safe 0.99 sources but not compiled on Windows.

use skia_safe::{
    Image,
    gpu::{self, DirectContext, SurfaceOrigin, backend_textures, d3d},
};
use windows::{
    Win32::Graphics::{
        Direct3D12::{
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, ID3D12CommandQueue, ID3D12Device,
        },
        Dxgi::{
            Common::{
                DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
                DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
                DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
            },
            IDXGIAdapter1,
        },
    },
    core::Interface,
};

use super::{color_space, color_type, init};
use crate::SkiaBackendError;

/// The Direct3D objects a presenter needs to build a swapchain and a Skia
/// context.
pub(crate) struct PlatformDevice {
    pub(crate) adapter: IDXGIAdapter1,
    pub(crate) device: ID3D12Device,
    pub(crate) queue: ID3D12CommandQueue,
}

pub(crate) struct Interop {
    platform: PlatformDevice,
}

impl Interop {
    pub(crate) const NAME: &'static str = "Direct3D 12";
    pub(crate) const BACKEND: wgpu::Backends = wgpu::Backends::DX12;

    pub(crate) fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Result<Self, SkiaBackendError> {
        // SAFETY: the COM interfaces are cloned (AddRef'd) out of the hal
        // guards before they drop; nothing is released here.
        let hal_adapter = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| init("the wgpu adapter is not Direct3D 12-backed"))?;
        let adapter: IDXGIAdapter1 =
            hal_adapter.raw_adapter().as_raw().cast().map_err(|error| {
                init(format!(
                    "the wgpu DXGI adapter is not an IDXGIAdapter1: {error}"
                ))
            })?;
        drop(hal_adapter);

        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| init("the wgpu device is not Direct3D 12-backed"))?;
        let device = hal_device.raw_device().clone();
        let queue = hal_device.raw_queue().clone();
        drop(hal_device);

        Ok(Self {
            platform: PlatformDevice {
                adapter,
                device,
                queue,
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

        // SAFETY: the guard lives across the clone of the resource interface,
        // which AddRefs it; nothing is released here.
        let hal_texture = unsafe { texture.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| init("the wgpu texture is not Direct3D 12-backed"))?;
        let resource = unsafe { hal_texture.raw_resource() }.clone();
        drop(hal_texture);

        let info = d3d::TextureResourceInfo {
            resource,
            alloc: None,
            // The caller rendered into this texture and Skia is about to sample
            // it; wgpu leaves a sampled render target in the shader-resource
            // state, and Skia transitions from whatever it is told.
            resource_state: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            format: dxgi_format(texture.format())?,
            sample_count: texture.sample_count(),
            level_count: texture.mip_level_count(),
            sample_quality_pattern: 0,
            protected: gpu::Protected::No,
        };
        let backend =
            backend_textures::make_d3d((width, height), &info, "xui-skia imported wgpu texture");
        let color_type = color_type(texture.format())?;
        gpu::images::borrow_texture_from(
            context,
            &backend,
            SurfaceOrigin::TopLeft,
            color_type,
            skia_safe::AlphaType::Premul,
            color_space(texture.format()),
        )
        .ok_or_else(|| {
            SkiaBackendError::WgpuTextureImport(format!(
                "Skia could not borrow the imported Direct3D resource: {width}x{height} {:?} \
                 as {color_type:?}. Skia validates the colour type against the texture's \
                 platform format, so a mismatch there is the usual cause",
                texture.format(),
            ))
        })
    }
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
            return Err(SkiaBackendError::WgpuTextureImport(format!(
                "no DXGI format mapping for the wgpu texture format {other:?}"
            )));
        }
    })
}
