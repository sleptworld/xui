//! Direct3D 12 presentation for Windows.
//!
//! Owns the `ID3D12Device`, the direct command queue Skia submits on, and a
//! DXGI flip-model swapchain. Each frame wraps the current back buffer in a
//! `skia_safe::Surface`, and presents it after Skia flushes.
//!
//! Frame pacing uses one fence: the value signalled after each `Present` is
//! recorded per back buffer, and re-acquiring that buffer waits for it. That
//! keeps at most `BUFFER_COUNT` frames in flight without stalling on every
//! frame the way a single `WaitForSingleObject` after each present would.

use std::sync::Arc;

use skia_safe::{
    ColorSpace, ColorType, Surface,
    gpu::{self, DirectContext, SurfaceOrigin, backend_render_targets, d3d},
    surfaces::BackendSurfaceAccess,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0},
        Graphics::{
            Direct3D::D3D_FEATURE_LEVEL_11_0,
            Direct3D12::{
                D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC,
                D3D12_COMMAND_QUEUE_FLAG_NONE, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_STATE_PRESENT,
                D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12Resource,
            },
            Dxgi::{
                Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                CreateDXGIFactory2, DXGI_ADAPTER_FLAG, DXGI_ADAPTER_FLAG_SOFTWARE,
                DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SCALING_STRETCH,
                DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
                DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter1, IDXGIFactory4, IDXGISwapChain3,
            },
        },
        System::Threading::{CreateEventW, INFINITE, WaitForSingleObject},
    },
    core::Interface,
};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

use crate::SkiaBackendError;

/// Flip-model swapchains need at least two buffers; three keeps the GPU fed
/// while one is on screen and one is queued.
const BUFFER_COUNT: usize = 3;
const SWAPCHAIN_FORMAT: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT =
    DXGI_FORMAT_B8G8R8A8_UNORM;

fn init(message: impl Into<String>) -> SkiaBackendError {
    SkiaBackendError::Direct3DInitialization(message.into())
}

fn present_error(message: impl Into<String>) -> SkiaBackendError {
    SkiaBackendError::Direct3DPresentation(message.into())
}

pub(crate) struct Direct3DPresenter {
    _window: Arc<Window>,
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    swapchain: IDXGISwapChain3,
    fence: ID3D12Fence,
    fence_event: HANDLE,
    /// Monotonic; the next value to signal on the queue.
    fence_value: u64,
    /// The fence value that must be reached before back buffer `i` is reusable.
    buffer_fences: [u64; BUFFER_COUNT],
    width: u32,
    height: u32,
    /// Index of the back buffer handed out by `acquire_surface`.
    acquired: Option<u32>,
}

impl Direct3DPresenter {
    pub(crate) fn new(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        let hwnd = match window
            .window_handle()
            .map_err(|error| init(error.to_string()))?
            .as_raw()
        {
            RawWindowHandle::Win32(handle) => HWND(handle.hwnd.get() as *mut _),
            _ => return Err(init("winit did not expose a Win32 window handle")),
        };

        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
            .map_err(|error| init(format!("could not create a DXGI factory: {error}")))?;

        let (adapter, device) = select_adapter(&factory)?;

        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            Priority: 0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        };
        let queue: ID3D12CommandQueue = unsafe { device.CreateCommandQueue(&queue_desc) }
            .map_err(|error| init(format!("could not create a D3D12 command queue: {error}")))?;

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: SWAPCHAIN_FORMAT,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: BUFFER_COUNT as u32,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        let swapchain =
            unsafe { factory.CreateSwapChainForHwnd(&queue, hwnd, &swapchain_desc, None, None) }
                .map_err(|error| init(format!("could not create a DXGI swapchain: {error}")))?;
        let swapchain: IDXGISwapChain3 = swapchain
            .cast()
            .map_err(|error| init(format!("DXGI swapchain is not an IDXGISwapChain3: {error}")))?;

        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }
            .map_err(|error| init(format!("could not create a D3D12 fence: {error}")))?;
        let fence_event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|error| init(format!("could not create the fence event: {error}")))?;

        let backend = d3d::BackendContext {
            adapter,
            device: device.clone(),
            queue: queue.clone(),
            memory_allocator: None,
            protected_context: gpu::Protected::No,
        };
        let context = unsafe { gpu::direct_contexts::make_d3d(&backend, None) }
            .ok_or_else(|| init("Skia could not create a Ganesh Direct3D context"))?;

        Ok((
            Self {
                _window: window,
                device,
                queue,
                swapchain,
                fence,
                fence_event,
                fence_value: 0,
                buffer_fences: [0; BUFFER_COUNT],
                width,
                height,
                acquired: None,
            },
            context,
        ))
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) -> Result<(), SkiaBackendError> {
        if (self.width, self.height) == (width, height) || width == 0 || height == 0 {
            return Ok(());
        }
        // Every back buffer is about to be released, so all frames referencing
        // them have to be off the GPU first.
        self.wait_for_fence(self.fence_value)?;
        unsafe {
            self.swapchain.ResizeBuffers(
                BUFFER_COUNT as u32,
                width,
                height,
                SWAPCHAIN_FORMAT,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        }
        .map_err(|error| present_error(format!("ResizeBuffers failed: {error}")))?;
        self.buffer_fences = [0; BUFFER_COUNT];
        self.width = width;
        self.height = height;
        self.acquired = None;
        Ok(())
    }

    pub(crate) fn acquire_surface(
        &mut self,
        context: &mut DirectContext,
        width: u32,
        height: u32,
    ) -> Result<Surface, SkiaBackendError> {
        self.resize(width, height)?;

        let index = unsafe { self.swapchain.GetCurrentBackBufferIndex() };
        // Do not record into a back buffer the GPU is still reading.
        self.wait_for_fence(self.buffer_fences[index as usize])?;

        let resource: ID3D12Resource = unsafe { self.swapchain.GetBuffer(index) }
            .map_err(|error| present_error(format!("GetBuffer({index}) failed: {error}")))?;

        let info = d3d::TextureResourceInfo {
            resource,
            alloc: None,
            // Flip-model back buffers come out of Present in the PRESENT state,
            // and Skia transitions from whatever we declare here.
            resource_state: D3D12_RESOURCE_STATE_PRESENT,
            format: SWAPCHAIN_FORMAT,
            sample_count: 1,
            level_count: 1,
            sample_quality_pattern: 0,
            protected: gpu::Protected::No,
        };
        let target =
            backend_render_targets::make_d3d((self.width as i32, self.height as i32), &info);
        let surface = gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            SurfaceOrigin::TopLeft,
            ColorType::BGRA8888,
            ColorSpace::new_srgb(),
            None,
        )
        .ok_or_else(|| present_error("Skia could not wrap the current back buffer"))?;

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
            .ok_or_else(|| present_error("no acquired back buffer to present"))?;

        let mut surface = surface;
        // `Present` access makes Skia leave the resource in the PRESENT state.
        context.flush_surface_with_access(
            &mut surface,
            BackendSurfaceAccess::Present,
            &gpu::FlushInfo::default(),
        );
        context.submit(None);
        drop(surface);

        unsafe { self.swapchain.Present(1, DXGI_PRESENT(0)) }
            .ok()
            .map_err(|error| present_error(format!("Present failed: {error}")))?;

        // Direct queues execute in submission order, so a signal queued now is
        // reached only after this frame's rendering has completed.
        self.fence_value += 1;
        unsafe { self.queue.Signal(&self.fence, self.fence_value) }
            .map_err(|error| present_error(format!("fence Signal failed: {error}")))?;
        self.buffer_fences[index as usize] = self.fence_value;
        Ok(())
    }

    fn wait_for_fence(&self, value: u64) -> Result<(), SkiaBackendError> {
        if value == 0 || unsafe { self.fence.GetCompletedValue() } >= value {
            return Ok(());
        }
        unsafe {
            self.fence
                .SetEventOnCompletion(value, self.fence_event)
                .map_err(|error| present_error(format!("SetEventOnCompletion failed: {error}")))?;
        }
        if unsafe { WaitForSingleObject(self.fence_event, INFINITE) } != WAIT_OBJECT_0 {
            return Err(present_error("waiting on the frame fence failed"));
        }
        Ok(())
    }
}

impl Drop for Direct3DPresenter {
    fn drop(&mut self) {
        let _ = self.wait_for_fence(self.fence_value);
        if !self.fence_event.is_invalid() {
            let _ = unsafe { CloseHandle(self.fence_event) };
        }
        let _ = &self.device;
    }
}

fn select_adapter(
    factory: &IDXGIFactory4,
) -> Result<(IDXGIAdapter1, ID3D12Device), SkiaBackendError> {
    let mut last_error = None;
    for index in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        let Ok(description) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        // WARP is a CPU rasterizer; the softbuffer fallback is a better answer
        // than pretending to be GPU-accelerated.
        if DXGI_ADAPTER_FLAG(description.Flags as i32) == DXGI_ADAPTER_FLAG_SOFTWARE {
            continue;
        }
        let mut device: Option<ID3D12Device> = None;
        match unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) } {
            Ok(()) => {
                if let Some(device) = device {
                    return Ok((adapter, device));
                }
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(init(match last_error {
        Some(error) => format!("no Direct3D 12 adapter is available: {error}"),
        None => "no Direct3D 12 adapter is available".to_owned(),
    }))
}
