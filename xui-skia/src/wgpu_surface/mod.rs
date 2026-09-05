//! Skia rendering on a wgpu-owned device.
//!
//! Modelled on Slint's `i-slint-renderer-skia/wgpu_29_surface.rs`: wgpu creates
//! the instance, adapter, device, queue and swapchain, Skia is handed the raw
//! platform handles sitting behind them through `wgpu-hal`, Skia renders into
//! the swapchain texture, and wgpu presents. The native presenters in
//! [`crate::present`] do the opposite -- they own the device and never let
//! anything else near it -- which is exactly why they cannot do the one thing
//! this module exists for:
//!
//! **A `wgpu::Texture` an application rendered itself can be composited by
//! Skia**, because both now sit on the same device. That is the canvas wgpu
//! path: the caller draws with wgpu (WGSL, compute, 3D -- whatever SkSL cannot
//! express), hands the texture back, and [`WgpuPresenter::borrow_image`] turns
//! it into an `SkImage` the normal draw path composites like any other image.
//!
//! A second, quieter benefit: the device, queue and swapchain here are ordinary
//! wgpu objects, so the `xui-winit` wgpu renderer and this one can eventually
//! share a single surface and present path instead of each owning a private
//! copy of the platform swapchain.
//!
//! # Opting in
//!
//! Build `xui-skia` with the `wgpu` feature and set `XUI_SKIA_WGPU=1`. Without
//! both, [`crate::present::WindowPresenter`] picks the native presenter as
//! before -- this path changes the bottom of the rendering stack and is not
//! something to switch on by default.
//!
//! # What each platform costs
//!
//! One backend per platform, chosen to match the Skia binary's own backends:
//! Metal on macOS, Vulkan on Linux, D3D12 on Windows. GL is excluded on every
//! platform (Slint hit artifacts there, and Skia's default binaries do not ship
//! a Vulkan backend on Windows either).
//!
//! Sharing a device removes cross-device memory sharing, not synchronization.
//! wgpu keeps its own resource-state tracker and Skia keeps its own, so every
//! handoff has to be explicit:
//!
//! - **Vulkan / D3D12** -- [`Interop::finish_frame`] flushes with
//!   `BackendSurfaceAccess::Present`, which leaves the swapchain image in
//!   `PRESENT_SRC_KHR` / `D3D12_RESOURCE_STATE_PRESENT`, the state wgpu's
//!   `present()` assumes because it never touched the texture itself.
//! - **Metal** -- Skia commits to wgpu's own `MTLCommandQueue`, reached through
//!   `metal::Queue::as_raw`, so commit order alone orders the two. That
//!   accessor is missing between wgpu-hal 29.0.0 and 29.0.3; see [`metal`] for
//!   why `Cargo.toml` pins a 29.0.4 floor.

use std::sync::Arc;

use skia_safe::{Image, Surface, gpu::DirectContext};
use winit::window::Window;

use crate::SkiaBackendError;

#[cfg(target_os = "macos")]
#[path = "metal.rs"]
mod interop;
#[cfg(target_os = "linux")]
#[path = "vulkan.rs"]
mod interop;
#[cfg(target_os = "windows")]
#[path = "dx12.rs"]
mod interop;
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
#[path = "unsupported.rs"]
mod interop;

pub(crate) use interop::Interop;

/// The device and queue Skia is rendering on, for callers that want to draw
/// into a texture Skia will then composite.
///
/// Both handles are `wgpu`'s own reference-counted ones, so a clone keeps the
/// device alive independently of the presenter. This is the equivalent of what
/// Slint hands out through `Window::set_rendering_notifier` -- the point is
/// that a caller must not create its own device, because a texture from a
/// different device cannot be imported.
#[derive(Clone)]
pub struct WgpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl WgpuContext {
    /// The device Skia renders on. Textures to be composited must come from it.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue Skia's swapchain is presented from.
    ///
    /// Work submitted here must be finished before the texture it writes is
    /// handed to `SkiaBackend::import_wgpu_texture`; see the module docs on
    /// synchronization.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

impl std::fmt::Debug for WgpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuContext").finish_non_exhaustive()
    }
}

/// Usages a texture must declare to be importable as an `SkImage`.
///
/// Same requirement Slint documents for `Image::try_from(wgpu::Texture)`:
/// `RENDER_ATTACHMENT` because the caller draws into it, `TEXTURE_BINDING`
/// because Skia samples it.
pub const IMPORTABLE_TEXTURE_USAGES: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING);

/// A Skia [`Image`] borrowing a caller's `wgpu::Texture`, with the texture
/// held alongside it.
///
/// Skia's `borrow_texture_from` does what its name says -- it wraps the
/// platform handle without copying and without retaining it, and skia-safe
/// 0.99 exposes no variant that takes a release callback. So the handle staying
/// alive is this type's job: it keeps a clone of the `wgpu::Texture` (a
/// reference-counted handle, not a copy of the pixels) for exactly as long as
/// the image can be drawn.
///
/// Deref gives the `Image`, so it draws like any other.
pub struct WgpuImage {
    image: Image,
    /// Not read. Its only purpose is to outlive `image`.
    _texture: wgpu::Texture,
}

impl WgpuImage {
    pub fn image(&self) -> &Image {
        &self.image
    }
}

impl std::ops::Deref for WgpuImage {
    type Target = Image;

    fn deref(&self) -> &Image {
        &self.image
    }
}

impl std::fmt::Debug for WgpuImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuImage")
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .finish_non_exhaustive()
    }
}

pub(crate) struct WgpuPresenter {
    _window: Arc<Window>,
    interop: Interop,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Held between `acquire_surface` and `present`, like the Metal presenter's
    /// drawable and the Direct3D presenter's back-buffer index.
    acquired: Option<wgpu::SurfaceTexture>,
}

impl WgpuPresenter {
    pub(crate) fn new(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: Arc<Window>) -> Result<(Self, DirectContext), SkiaBackendError> {
        // Restricted to the one backend Skia can interop with on this
        // platform: GL is excluded everywhere (Slint hit artifacts there and
        // Skia's stock Windows binaries have no Vulkan backend), and letting
        // wgpu pick would mean `Interop::new` failing on a device it cannot
        // reach into. `display` stays unset -- it is only consulted by GLES.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = Interop::BACKEND;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|error| init(format!("could not create a wgpu surface: {error}")))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|error| {
                init(format!(
                    "no wgpu adapter for the {} backend: {error}",
                    Interop::NAME
                ))
            })?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("xui-skia wgpu device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .map_err(|error| init(format!("could not open a wgpu device: {error}")))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| init("the wgpu adapter does not support this surface"))?;
        // Match what the native Vulkan presenter asks of its swapchain images:
        // colour attachment plus both transfer directions. Nothing samples the
        // swapchain -- `IMPORTABLE_TEXTURE_USAGES` is for textures a caller
        // hands *in* -- and asking for SAMPLED here would cost framebuffer
        // compression on some drivers for no gain. The transfers are what
        // Skia's readbacks and backdrop copies go through.
        //
        // Masked against what the surface actually supports, because
        // `configure` rejects a usage the platform cannot give.
        let supported = surface.get_capabilities(&adapter).usages;
        config.usage |= supported & (wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST);
        surface.configure(&device, &config);

        let (interop, context) = Interop::new(&adapter, &device, &queue)?;

        Ok((
            Self {
                _window: window,
                interop,
                device,
                queue,
                surface,
                config,
                acquired: None,
            },
            context,
        ))
    }

    pub(crate) fn context(&self) -> WgpuContext {
        WgpuContext {
            device: self.device.clone(),
            queue: self.queue.clone(),
        }
    }

    /// Reconfigures the swapchain.
    ///
    /// `context` is flushed and synced first for the same reason the Vulkan and
    /// Direct3D presenters do it: Skia keeps wrapped render targets in its
    /// resource cache after the wrapping surface is dropped, and those cached
    /// references have to go before wgpu will release the old swapchain images.
    pub(crate) fn resize(
        &mut self,
        context: Option<&mut DirectContext>,
        width: u32,
        height: u32,
    ) -> Result<(), SkiaBackendError> {
        let (width, height) = (width.max(1), height.max(1));
        if self.config.width == width && self.config.height == height {
            return Ok(());
        }
        self.acquired = None;
        if let Some(context) = context {
            context.flush_submit_and_sync_cpu();
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        Ok(())
    }

    pub(crate) fn acquire_surface(
        &mut self,
        context: &mut DirectContext,
        width: u32,
        height: u32,
    ) -> Result<Surface, SkiaBackendError> {
        self.resize(Some(context), width, height)?;
        let frame = self.acquire_frame(context)?;
        let surface = self.interop.wrap_render_target(context, &frame.texture)?;
        self.acquired = Some(frame);
        Ok(surface)
    }

    /// One reconfigure-and-retry, matching what the `xui-winit` wgpu renderer
    /// does. A frame that is merely occluded or timed out is not an error --
    /// there is nothing to draw into, so the caller is told to skip it.
    fn acquire_frame(
        &mut self,
        context: &mut DirectContext,
    ) -> Result<wgpu::SurfaceTexture, SkiaBackendError> {
        use wgpu::CurrentSurfaceTexture as Current;
        match self.surface.get_current_texture() {
            Current::Success(frame) | Current::Suboptimal(frame) => Ok(frame),
            Current::Outdated | Current::Lost => {
                context.flush_submit_and_sync_cpu();
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    Current::Success(frame) | Current::Suboptimal(frame) => Ok(frame),
                    other => Err(present_error(format!(
                        "could not acquire a wgpu surface texture after reconfiguring: {}",
                        describe_acquire(&other)
                    ))),
                }
            }
            other => Err(present_error(format!(
                "could not acquire a wgpu surface texture: {}",
                describe_acquire(&other)
            ))),
        }
    }

    pub(crate) fn present(
        &mut self,
        context: &mut DirectContext,
        surface: Surface,
    ) -> Result<(), SkiaBackendError> {
        let frame = self.acquired.take().ok_or_else(|| {
            present_error("no acquired wgpu surface texture to present".to_string())
        })?;
        let mut surface = surface;
        self.interop.finish_frame(context, &mut surface);
        // Skia holds a reference to the swapchain image through the wrapped
        // render target; wgpu will not hand it to the presentation engine while
        // that reference is alive.
        drop(surface);
        frame.present();
        Ok(())
    }

    /// Wraps a caller-rendered `wgpu::Texture` as an `SkImage`, without a copy.
    ///
    /// The texture must come from [`WgpuContext::device`] and declare
    /// [`IMPORTABLE_TEXTURE_USAGES`]. Keeping the platform handle alive is
    /// handled by [`WgpuImage`], which holds a clone of the texture.
    ///
    /// The caller is responsible for having submitted the work that fills the
    /// texture before calling this.
    pub(crate) fn borrow_image(
        &self,
        context: &mut DirectContext,
        texture: &wgpu::Texture,
    ) -> Result<WgpuImage, SkiaBackendError> {
        if !texture.usage().contains(IMPORTABLE_TEXTURE_USAGES) {
            return Err(SkiaBackendError::WgpuTextureImport(format!(
                "a texture imported into Skia must declare RENDER_ATTACHMENT | TEXTURE_BINDING, \
                 this one declares {:?}",
                texture.usage()
            )));
        }
        Ok(WgpuImage {
            image: self.interop.borrow_image(context, texture)?,
            _texture: texture.clone(),
        })
    }
}

fn describe_acquire(texture: &wgpu::CurrentSurfaceTexture) -> &'static str {
    use wgpu::CurrentSurfaceTexture as Current;
    match texture {
        Current::Success(_) => "success",
        Current::Suboptimal(_) => "suboptimal",
        Current::Timeout => "timeout",
        Current::Outdated => "outdated",
        Current::Lost => "lost",
        Current::Occluded => "occluded",
        Current::Validation => "validation error",
    }
}

/// Whether the wgpu path was asked for. Off unless explicitly requested --
/// see the module docs.
pub(crate) fn requested() -> bool {
    matches!(
        std::env::var("XUI_SKIA_WGPU").as_deref(),
        Ok("1") | Ok("on") | Ok("true")
    )
}

pub(crate) fn init(message: impl Into<String>) -> SkiaBackendError {
    SkiaBackendError::WgpuInitialization(message.into())
}

pub(crate) fn present_error(message: impl Into<String>) -> SkiaBackendError {
    SkiaBackendError::WgpuPresentation(message.into())
}

/// Skia colour type for a swapchain format.
///
/// Only the formats `Surface::get_default_config` actually hands out are
/// listed; anything else means the interop assumptions no longer hold and is
/// better refused than silently mis-sampled.
pub(crate) fn color_type(
    format: wgpu::TextureFormat,
) -> Result<skia_safe::ColorType, SkiaBackendError> {
    use skia_safe::ColorType;
    use wgpu::TextureFormat as Format;
    Ok(match format {
        Format::Bgra8Unorm | Format::Bgra8UnormSrgb => ColorType::BGRA8888,
        Format::Rgba8Unorm | Format::Rgba8UnormSrgb => ColorType::RGBA8888,
        Format::Rgba16Float => ColorType::RGBAF16,
        Format::Rgb10a2Unorm => ColorType::RGBA1010102,
        other => {
            return Err(init(format!(
                "no Skia colour type for the wgpu texture format {other:?}"
            )));
        }
    })
}

/// The colour space to read a swapchain format through.
///
/// `*UnormSrgb` formats already decode on sample, so Skia must be told the
/// surface is sRGB or it would apply the transfer function a second time.
pub(crate) fn color_space(format: wgpu::TextureFormat) -> skia_safe::ColorSpace {
    if format.is_srgb() {
        skia_safe::ColorSpace::new_srgb()
    } else {
        // A linear surface still carries sRGB primaries; only the transfer
        // function differs.
        skia_safe::ColorSpace::new_srgb_linear()
    }
}

#[cfg(test)]
mod tests {
    use skia_safe::{AlphaType, ColorType, ImageInfo, gpu};

    use super::*;

    const SIZE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// A real wgpu pipeline renders into a texture, Skia samples it, and the
    /// shader's output comes back.
    ///
    /// This is the canvas wgpu path end to end: a WGSL fragment shader writes a
    /// pattern Skia could not have produced by accident -- red on the left half
    /// of the texture, blue on the right -- and Skia draws the borrowed texture
    /// without a copy. A pixel that is neither means the handle Skia was given
    /// does not address the memory wgpu wrote.
    #[test]
    fn skia_samples_a_texture_wgpu_rendered() {
        let Some((adapter, device, queue)) = headless_device() else {
            eprintln!("no {} adapter available, skipping", Interop::NAME);
            return;
        };
        let (interop, mut context) = Interop::new(&adapter, &device, &queue)
            .expect("Skia should build a context on the wgpu device");

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("interop test target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: IMPORTABLE_TEXTURE_USAGES,
            view_formats: &[],
        });

        // A fullscreen triangle whose fragment shader splits the texture down
        // the middle. Written by wgpu and by nothing else.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("interop test shader"),
            source: wgpu::ShaderSource::Wgsl(SPLIT_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("interop test pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interop test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Green: a colour the shader never writes, so a pixel
                        // that comes back green means the draw did not land.
                        load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        // Skia and wgpu track resources separately, so the caller -- here, the
        // test -- is the one that has to make the write visible first.
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the clear should complete");

        let image = interop
            .borrow_image(&mut context, &texture)
            .expect("Skia should borrow the wgpu texture");
        assert_eq!(image.width(), SIZE as i32);
        assert_eq!(image.height(), SIZE as i32);

        // Same colour space in and out, so nothing is converted on the way
        // through and a mismatch can only mean the sample itself was wrong.
        let info = ImageInfo::new(
            (SIZE as i32, SIZE as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            color_space(FORMAT),
        );
        let mut surface = gpu::surfaces::render_target(
            &mut context,
            gpu::Budgeted::Yes,
            &info,
            None,
            gpu::SurfaceOrigin::TopLeft,
            None,
            false,
            false,
        )
        .expect("Skia should allocate a render target on the wgpu device");
        surface.canvas().draw_image(&image, (0, 0), None);
        context.flush_and_submit_surface(&mut surface, None);
        context.submit(gpu::SyncCpu::Yes);

        let row_bytes = SIZE as usize * 4;
        let mut pixels = vec![0u8; row_bytes * SIZE as usize];
        assert!(
            surface.read_pixels(&info, &mut pixels, row_bytes, (0, 0)),
            "reading back the Skia surface should succeed"
        );
        let pixel = |x: usize| &pixels[x * 4..x * 4 + 4];
        assert_eq!(
            pixel(0),
            &[255, 0, 0, 255],
            "the left half should be the red the WGSL fragment shader wrote"
        );
        assert_eq!(
            pixel(SIZE as usize - 1),
            &[0, 0, 255, 255],
            "the right half should be the blue the WGSL fragment shader wrote"
        );
    }

    /// A fullscreen triangle, split red/blue at the halfway column.
    const SPLIT_WGSL: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    return vec4<f32>(corners[index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    if (position.x < 32.0) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    return vec4<f32>(0.0, 0.0, 1.0, 1.0);
}
"#;

    fn headless_device() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = Interop::BACKEND;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("xui-skia interop test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        }))
        .ok()?;
        Some((adapter, device, queue))
    }
}
