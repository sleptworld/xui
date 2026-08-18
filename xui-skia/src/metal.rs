use std::sync::Arc;

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_core_foundation::CGSize;
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable,
    MTLPixelFormat,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use raw_window_metal::Layer;
use skia_safe::{
    ColorSpace, ColorType, Surface,
    gpu::{self, DirectContext, SurfaceOrigin, backend_render_targets, mtl},
};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

use crate::SkiaBackendError;

pub(crate) struct MetalPresenter {
    _window: Arc<Window>,
    layer: Layer,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
}

impl MetalPresenter {
    pub(crate) fn new(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        let raw = window
            .window_handle()
            .map_err(|error| SkiaBackendError::MetalInitialization(error.to_string()))?
            .as_raw();
        let layer = match raw {
            RawWindowHandle::AppKit(handle) => unsafe { Layer::from_ns_view(handle.ns_view) },
            _ => {
                return Err(SkiaBackendError::MetalInitialization(
                    "winit did not expose an AppKit window handle".into(),
                ));
            }
        };
        let metal_layer = unsafe { layer.as_ptr().cast::<CAMetalLayer>().as_ref() };
        let device = MTLCreateSystemDefaultDevice().ok_or_else(|| {
            SkiaBackendError::MetalInitialization("no system Metal device is available".into())
        })?;
        let command_queue = device.newCommandQueue().ok_or_else(|| {
            SkiaBackendError::MetalInitialization("could not create a Metal command queue".into())
        })?;

        metal_layer.setDevice(Some(&device));
        metal_layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        metal_layer.setFramebufferOnly(false);
        metal_layer.setPresentsWithTransaction(false);
        let size = window.inner_size();
        metal_layer.setDrawableSize(CGSize::new(size.width as f64, size.height as f64));

        let backend = unsafe {
            mtl::BackendContext::new(
                Retained::as_ptr(&device) as mtl::Handle,
                Retained::as_ptr(&command_queue) as mtl::Handle,
            )
        };
        let context = gpu::direct_contexts::make_metal(&backend, None).ok_or_else(|| {
            SkiaBackendError::MetalInitialization(
                "Skia could not create a Ganesh Metal context".into(),
            )
        })?;

        Ok((
            Self {
                _window: window,
                layer,
                command_queue,
                drawable: None,
            },
            context,
        ))
    }

    pub(crate) fn resize(&self, width: u32, height: u32) {
        self.metal_layer()
            .setDrawableSize(CGSize::new(width as f64, height as f64));
    }

    pub(crate) fn acquire_surface(
        &mut self,
        context: &mut DirectContext,
        width: u32,
        height: u32,
    ) -> Result<Surface, SkiaBackendError> {
        self.resize(width, height);
        let drawable = self.metal_layer().nextDrawable().ok_or_else(|| {
            SkiaBackendError::MetalPresentation("CAMetalLayer returned no drawable".into())
        })?;
        let texture = drawable.texture();
        let texture_info =
            unsafe { mtl::TextureInfo::new(Retained::as_ptr(&texture) as mtl::Handle) };
        let target = backend_render_targets::make_mtl((width as i32, height as i32), &texture_info);
        let surface = gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            SurfaceOrigin::TopLeft,
            ColorType::BGRA8888,
            ColorSpace::new_srgb(),
            None,
        )
        .ok_or_else(|| {
            SkiaBackendError::MetalPresentation(
                "Skia could not wrap the current Metal drawable".into(),
            )
        })?;
        self.drawable = Some(drawable);
        Ok(surface)
    }

    pub(crate) fn present(&mut self) -> Result<(), SkiaBackendError> {
        let drawable = self.drawable.take().ok_or_else(|| {
            SkiaBackendError::MetalPresentation("no acquired drawable to present".into())
        })?;
        let command_buffer = self.command_queue.commandBuffer().ok_or_else(|| {
            SkiaBackendError::MetalPresentation(
                "could not create a presentation command buffer".into(),
            )
        })?;
        let drawable: Retained<ProtocolObject<dyn MTLDrawable>> = (&drawable).into();
        command_buffer.presentDrawable(&drawable);
        command_buffer.commit();
        Ok(())
    }

    fn metal_layer(&self) -> &CAMetalLayer {
        unsafe { self.layer.as_ptr().cast::<CAMetalLayer>().as_ref() }
    }
}
