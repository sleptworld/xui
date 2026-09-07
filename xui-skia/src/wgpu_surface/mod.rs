//! A wgpu device for Skia to share.
//!
//! wgpu creates an instance, adapter, device and queue -- **and nothing else**.
//! No surface, no swapchain, no presentation. The native presenters in
//! [`crate::present`] keep every bit of that: `CAMetalLayer`, the DXGI
//! flip-model swapchain with its buffer fences, the `VK_KHR_swapchain` with its
//! chosen present mode and image count. All they give up is the ten lines where
//! they used to *create* a device; they now accept one.
//!
//! # Why
//!
//! Skia can only composite a `wgpu::Texture` if both sit on the same device.
//! That is the canvas wgpu path: the caller draws with wgpu (WGSL, compute, 3D
//! -- whatever SkSL cannot express), hands the texture back, and
//! [`WgpuHost::borrow_image`] wraps it as an `SkImage` with no copy, which the
//! normal draw path then composites like any other image.
//!
//! Sharing a device is all that requires. An earlier version of this module
//! followed Slint's `wgpu_29_surface.rs` and let wgpu own the swapchain and
//! presentation too, which is how Slint does it -- but Slint has one renderer,
//! and handing wgpu the swapchain here meant bypassing three presenters that
//! already know what they are doing, plus a CPU stall on every resize and
//! wgpu's frame-latency defaults instead of the ones `crate::d3d` picked.
//! None of that bought anything: the device is the only shared object the
//! import actually needs.
//!
//! The direction matters and only one direction works. Building wgpu on top of
//! *Skia's* device -- the arrangement this shape might suggest -- needs
//! `device_from_raw`, which `wgpu-hal` 29 has for Vulkan and Metal but **not**
//! for D3D12. Reading raw handles back out of a wgpu device works everywhere,
//! so wgpu creates and Skia borrows.
//!
//! # Opting in
//!
//! Building `xui-skia` with the `wgpu` feature is the whole opt-in: the
//! presenters then take their device from here instead of creating one, and a
//! canvas GPU painter has something to draw with. Without the feature they
//! create their own devices exactly as before.
//!
//! Opening a wgpu device costs startup time (instance, adapter, device --
//! tens of milliseconds), and it is paid whether or not any canvas uses it,
//! because the device has to be chosen before Skia's context is built and long
//! before any widget exists. That is what the Cargo feature is for.
//!
//! `XUI_SKIA_WGPU=0` forces the self-owned path back on without a rebuild, for
//! working around a driver that misbehaves on the shared one -- the same
//! escape hatch `XUI_SKIA_GPU=0` is for the GPU presenters as a whole.
//!
//! # Platforms
//!
//! One backend each, matching the backends Skia's own binaries ship: Metal on
//! macOS, Vulkan on Linux, D3D12 on Windows. GL is excluded everywhere (Slint
//! hit artifacts there, and Skia's stock Windows binaries have no Vulkan
//! backend either).
//!
//! The presenters need more than a device from wgpu, and get it:
//!
//! - **Metal** -- the `MTLDevice` and, through `metal::Queue::as_raw`, wgpu's
//!   own `MTLCommandQueue`, so Skia and wgpu commit to one queue. That accessor
//!   is missing in wgpu-hal 29.0.0 through 29.0.3; see [`interop`] for why
//!   `Cargo.toml` pins a 29.0.4 floor.
//! - **Vulkan** -- the `ash::Entry` and `ash::Instance` as well, because the
//!   presenter creates its own `VkSurfaceKHR` from them. wgpu-hal enables every
//!   platform surface extension on its instance and requires `VK_KHR_swapchain`
//!   on every device it opens, so both are always available.
//! - **D3D12** -- the `ID3D12Device` and `ID3D12CommandQueue`; the presenter
//!   makes its own DXGI factory, which is independent of the device.
//!
//! # Synchronization
//!
//! Sharing a device removes cross-device memory sharing, not synchronization.
//! wgpu keeps its own resource-state tracker and Skia keeps its own, so a
//! caller must have *submitted* the work filling a texture before importing it.
//! With a shared queue that is enough -- no CPU wait.

use skia_safe::{Image, gpu::DirectContext};

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

pub(crate) use interop::{Interop, PlatformDevice};

/// The device and queue Skia is rendering on, for callers that want to draw
/// into a texture Skia will then composite.
///
/// Both handles are wgpu's own reference-counted ones, so a clone keeps the
/// device alive independently of the host. A caller must not create its own
/// device: a texture from a different device cannot be imported.
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

    /// The queue Skia submits on.
    ///
    /// Work submitted here must be submitted before the texture it writes is
    /// handed to `SkiaBackend::import_wgpu_texture`.
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
/// The same requirement Slint documents for `Image::try_from(wgpu::Texture)`:
/// `RENDER_ATTACHMENT` because the caller draws into it, `TEXTURE_BINDING`
/// because Skia samples it.
pub const IMPORTABLE_TEXTURE_USAGES: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING);

/// A Skia [`Image`] borrowing a caller's `wgpu::Texture`, with the texture held
/// alongside it.
///
/// Skia's `borrow_texture_from` does what its name says -- it wraps the
/// platform handle without copying and without retaining it, and skia-safe 0.99
/// exposes no variant that takes a release callback. So keeping the handle
/// alive is this type's job: it holds a clone of the `wgpu::Texture` (a
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

    /// Splits into the image and the handle that keeps it valid.
    ///
    /// Only for callers that store the two together and so preserve the
    /// invariant themselves -- dropping the texture while keeping the image
    /// leaves the image pointing at freed memory.
    pub(crate) fn into_parts(self) -> (Image, wgpu::Texture) {
        (self.image, self._texture)
    }
}

impl std::ops::Deref for WgpuImage {
    type Target = Image;

    fn deref(&self) -> &Image {
        &self.image
    }
}

/// So it can be passed straight to `Canvas::draw_image` and friends, which
/// take `impl AsRef<Image>` and do not see through `Deref`.
impl AsRef<Image> for WgpuImage {
    fn as_ref(&self) -> &Image {
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

/// The wgpu objects Skia shares, and the platform handles behind them.
pub(crate) struct WgpuHost {
    /// Kept alive because the adapter and device are derived from it.
    _instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    interop: Interop,
}

impl WgpuHost {
    /// Opens a device on the one backend Skia can interop with here.
    ///
    /// No window is involved: the presenter owns presentation, so nothing here
    /// needs a surface. The cost is that the adapter is chosen without a
    /// `compatible_surface` hint, so on Vulkan the presenter has to verify the
    /// chosen queue family can actually present to its window -- and fail, so
    /// the caller falls back to a self-owned device, when it cannot.
    pub(crate) fn new() -> Result<Self, SkiaBackendError> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self, SkiaBackendError> {
        // Restricted to the backend Skia can reach into on this platform.
        // Letting wgpu pick would mean failing later on a device we cannot
        // extract handles from.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = Interop::BACKEND;
        let instance = wgpu::Instance::new(descriptor);

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
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
                label: Some("xui-skia shared device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .map_err(|error| init(format!("could not open a wgpu device: {error}")))?;

        let interop = Interop::new(&adapter, &device, &queue)?;

        Ok(Self {
            _instance: instance,
            device,
            queue,
            interop,
        })
    }

    /// The platform handles a presenter needs to build its swapchain and its
    /// Skia context on this device.
    pub(crate) fn platform_device(&self) -> &PlatformDevice {
        self.interop.platform_device()
    }

    pub(crate) fn context(&self) -> WgpuContext {
        WgpuContext {
            device: self.device.clone(),
            queue: self.queue.clone(),
        }
    }

    /// Wraps a caller-rendered `wgpu::Texture` as an `SkImage`, without a copy.
    ///
    /// The texture must come from [`WgpuContext::device`] and declare
    /// [`IMPORTABLE_TEXTURE_USAGES`]. Keeping the platform handle alive is
    /// handled by [`WgpuImage`]. The caller is responsible for having submitted
    /// the work that fills the texture before calling this.
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

/// An escape hatch, in the shape of the `XUI_SKIA_GPU=0` one next to it.
///
/// The Cargo feature is the switch: building with `wgpu` is the decision to
/// share a device, and requiring a second runtime opt-in on top of it meant a
/// build that asked for the feature silently got nothing. This exists only so a
/// driver that misbehaves on the shared path can be worked around without a
/// rebuild.
pub(crate) fn disabled() -> bool {
    matches!(
        std::env::var("XUI_SKIA_WGPU").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}

pub(crate) fn init(message: impl Into<String>) -> SkiaBackendError {
    SkiaBackendError::WgpuInitialization(message.into())
}

/// Skia colour type for a wgpu texture format.
///
/// Only formats the import path can wrap without converting are listed --
/// converting would defeat the point of borrowing the texture, so an unknown
/// format is refused rather than silently copied.
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
            return Err(SkiaBackendError::WgpuTextureImport(format!(
                "no Skia colour type for the wgpu texture format {other:?}"
            )));
        }
    })
}

/// The colour space to read a wgpu texture format through.
///
/// `*UnormSrgb` formats already decode on sample, so Skia must be told the
/// texture is sRGB or it would apply the transfer function a second time.
pub(crate) fn color_space(format: wgpu::TextureFormat) -> skia_safe::ColorSpace {
    if format.is_srgb() {
        skia_safe::ColorSpace::new_srgb()
    } else {
        // A linear texture still carries sRGB primaries; only the transfer
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
        let Some((host, mut context)) = headless_host() else {
            eprintln!("no {} adapter available, skipping", Interop::NAME);
            return;
        };
        let device = host.context().device().clone();
        let queue = host.context().queue().clone();

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

        let image = host
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

    /// A [`WgpuHost`] plus a Skia context on its device, with no window
    /// involved -- exactly the split this module is built around.
    fn headless_host() -> Option<(WgpuHost, gpu::DirectContext)> {
        let host = WgpuHost::new().ok()?;
        let context = platform_context(&host)?;
        Some((host, context))
    }

    /// Builds the Skia context the presenter would build, from the same
    /// platform handles.
    #[cfg(target_os = "macos")]
    fn platform_context(host: &WgpuHost) -> Option<gpu::DirectContext> {
        use skia_safe::gpu::mtl;
        let platform = host.platform_device();
        let backend = unsafe {
            mtl::BackendContext::new(
                objc2::rc::Retained::as_ptr(&platform.device) as mtl::Handle,
                objc2::rc::Retained::as_ptr(&platform.queue) as mtl::Handle,
            )
        };
        gpu::direct_contexts::make_metal(&backend, None)
    }

    #[cfg(not(target_os = "macos"))]
    fn platform_context(_host: &WgpuHost) -> Option<gpu::DirectContext> {
        // The other backends build their context inside their presenter, which
        // needs a window; this test is headless.
        None
    }
}
