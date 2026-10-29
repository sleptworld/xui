use std::sync::Arc;

use vello::{
    AaConfig, AaSupport, RenderParams, Renderer, RendererOptions,
    peniko::color::{AlphaColor, Srgb},
};
use xui_core::render::{BuiltFrame, BuiltItem, RenderBackend, damage::DamageTracker};
use xui_core::text::TextHost;
use xui_interface::{Color, NodeLifecycleEvent, Size, TextBackend};
use xui_shell::SurfaceTarget;
use xui_wgpu_text::{AtlasOptions, TextBatch, TextRenderer};

use crate::{
    VelloBackendError, WgpuContext,
    encode::{BitmapText, ClipMask, Inherit, Surface, SurfaceOp, Unsupported, encode_frame},
    filter::{ENCODED_FORMAT, Finish, FilterRenderer, WORK_FORMAT, Placed, PlanTextures, Slot},
    layer_cache::{CacheUpdate, LayerCache},
    texture::ExternalTextures,
    image::ImageCache,
    paths::PathCache,
    text::FontCache,
};

/// Where a target's last stage is drawn.
enum Dest {
    Root,
    Slot(Slot),
}

/// Vello renders into this; the blit pass reads it.
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelloBackendOptions {
    pub clear_color: Color,
    /// Bytes of decoded image pixels kept between frames. The least recently
    /// used go first, so an image scrolled out of view and back keeps its
    /// upload for as long as the budget allows.
    pub image_cache_budget_bytes: u64,
    /// Bytes of font files kept loaded.
    pub font_cache_budget_bytes: u64,
    /// Bytes of GPU textures kept for layer filters between frames.
    pub filter_texture_budget_bytes: u64,
    /// Bytes of cached layer content kept for layers that ask to be cached
    /// when it pays (`CachePolicy::Auto`), the least recently used going
    /// first. A layer that always wants it (`Always`) is not counted.
    pub layer_cache_budget_bytes: u64,
    /// Draw text from glyph bitmaps (`xui-wgpu-text`) when the text backend
    /// can render them, rather than as outlines through Vello. Text that is
    /// rotated, skewed, very large, or cut by a clip other than rectangles is
    /// outlines either way.
    pub bitmap_text: bool,
    /// Pages of the bitmap text atlas. A text that does not fit even at the
    /// atlas's limit is drawn as outlines for that frame.
    pub glyph_atlas: AtlasOptions,
}

impl Default for VelloBackendOptions {
    fn default() -> Self {
        Self {
            clear_color: Color::rgba(0.08, 0.09, 0.11, 1.0),
            image_cache_budget_bytes: 128 * 1024 * 1024,
            font_cache_budget_bytes: 256 * 1024 * 1024,
            filter_texture_budget_bytes: crate::filter::DEFAULT_POOL_BUDGET,
            layer_cache_budget_bytes: 128 * 1024 * 1024,
            bitmap_text: true,
            glyph_atlas: AtlasOptions::default(),
        }
    }
}

/// How much of each frame was drawn, for diagnosing and for tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Frames submitted.
    pub frames: u64,
    /// Frames in which nothing had changed, so nothing was drawn.
    pub unchanged: u64,
    /// The part of the last frame that was drawn, in physical pixels, as
    /// `[x0, y0, x1, y1]`; all zero when the frame was unchanged.
    pub last_region: [i32; 4],
    /// Pixels drawn in the last frame, and in all frames so far.
    pub last_pixels: u64,
    pub total_pixels: u64,
}

impl FrameStats {
    fn area(region: [i32; 4]) -> u64 {
        (region[2] - region[0]).max(0) as u64 * (region[3] - region[1]).max(0) as u64
    }
}

/// What the backend is holding on to, for diagnosing memory and for tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceStats {
    /// Oriented images cached, and their pixel bytes.
    pub image_entries: u64,
    pub image_bytes: u64,
    /// Images built from source pixels since the backend started. An image
    /// found in the cache does not count, so this stays put while a scene is
    /// merely redrawn.
    pub image_builds: u64,
    pub path_entries: u64,
    pub path_segments: u64,
    pub font_entries: usize,
    pub font_bytes: u64,
    /// Textures pooled for layer filters, and their bytes.
    pub filter_textures: usize,
    pub filter_texture_bytes: u64,
    pub mask_entries: u64,
    /// Cached layers, and the bytes of their content textures.
    pub layer_cache_entries: usize,
    pub layer_cache_bytes: u64,
    /// Since the backend started: a cached layer drawn from its texture (whole
    /// or patched), drawn afresh, and drawn only where it changed.
    pub layer_cache_hits: u64,
    pub layer_cache_misses: u64,
    pub layer_cache_patches: u64,
    /// The same, for a cached layer's filtered result: composited as kept
    /// (whole or patched), recomputed whole, and recomputed only where its
    /// content changed.
    pub layer_cache_filtered_hits: u64,
    pub layer_cache_filtered_misses: u64,
    pub layer_cache_filtered_patches: u64,
    /// Glyph bitmaps in the text atlas, inkless ones included, and the atlas
    /// pages (textures) holding them.
    pub glyph_atlas_entries: usize,
    pub glyph_atlas_pages: usize,
}

struct SceneTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Only a presenting backend blits the target, so only it has one.
    bind_group: Option<wgpu::BindGroup>,
    size: (u32, u32),
}

/// Everything that exists only when the backend draws to a window.
struct Presenter {
    /// Asked, before each acquire, whether the window's edge is being dragged.
    /// See [`VelloBackend::acquire`].
    window: Arc<dyn SurfaceTarget>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
}

impl Presenter {
    /// While the window's edge is being dragged, the frame has to reach the
    /// screen in the same Core Animation transaction as the new window
    /// geometry; Metal's asynchronous present lands a transaction late and the
    /// content jitters against the edge. wgpu reads the layer's
    /// `presentsWithTransaction` when the drawable is acquired and then
    /// presents it by hand once the GPU work is scheduled, so only the flag has
    /// to be set here. Outside a drag the asynchronous path is cheaper. Same
    /// policy as `xui-skia`'s Metal presenter.
    #[cfg(target_os = "macos")]
    fn sync_present_mode(&self) {
        let with_transaction = self.window.in_live_resize();
        // SAFETY: the hal surface is only read here, and the layer flag is
        // what wgpu-hal itself consults on the next acquire.
        if let Some(surface) = unsafe { self.surface.as_hal::<wgpu::hal::api::Metal>() } {
            let layer = surface.render_layer().lock();
            if layer.presentsWithTransaction() != with_transaction {
                layer.setPresentsWithTransaction(with_transaction);
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn sync_present_mode(&self) {}
}

pub struct VelloBackend<T: TextBackend> {
    /// `None` for a [headless](Self::headless) backend, which renders to its
    /// scene texture and never presents.
    presenter: Option<Presenter>,
    size_px: (u32, u32),
    context: WgpuContext,
    renderer: Renderer,
    target: Option<SceneTarget>,
    fonts: FontCache<T::FontId>,
    /// Bitmap text, made on the first frame that has some.
    text_renderer: Option<TextRenderer<T::FontId>>,
    images: ImageCache,
    paths: PathCache,
    filters: FilterRenderer,
    layer_cache: LayerCache,
    /// What changed since the last frame, so only that is drawn again.
    damage: DamageTracker,
    force_full: bool,
    /// The text backend's font epoch last frame; a change redraws everything.
    font_epoch: Option<u64>,
    /// A frame was drawn that the window has not been given yet.
    needs_present: bool,
    frame_stats: FrameStats,
    textures: ExternalTextures,
    options: VelloBackendOptions,
    scale_factor: f32,
    reported: Unsupported,
    presented: bool,
    _text: std::marker::PhantomData<fn() -> T>,
}

impl<T: TextBackend> VelloBackend<T> {
    pub fn new(window: Arc<dyn SurfaceTarget>) -> Result<Self, VelloBackendError> {
        Self::new_with_options(window, VelloBackendOptions::default())
    }

    pub fn new_with_options(
        window: Arc<dyn SurfaceTarget>,
        options: VelloBackendOptions,
    ) -> Result<Self, VelloBackendError> {
        pollster::block_on(Self::new_async(window, options))
    }

    async fn new_async(
        window: Arc<dyn SurfaceTarget>,
        options: VelloBackendOptions,
    ) -> Result<Self, VelloBackendError> {
        let size = window.surface_size();
        let scale_factor = window.scale_factor() as f32;
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await?;
        let features = adapter.get_texture_format_features(SCENE_FORMAT);
        if !features
            .allowed_usages
            .contains(wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(VelloBackendError::UnsupportedVectorTarget);
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("xui-vello device"),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await?;

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or(VelloBackendError::UnsupportedSurface)?;
        // Vello's output is already sRGB-encoded, so present through a
        // non-sRGB format and let the blit copy bytes through unchanged.
        let formats = surface.get_capabilities(&adapter).formats;
        let linear = config.format.remove_srgb_suffix();
        config.format = if formats.contains(&linear) {
            linear
        } else {
            formats
                .iter()
                .copied()
                .find(|format| !format.is_srgb())
                .unwrap_or(config.format)
        };
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        surface.configure(&device, &config);

        let renderer = new_renderer(&device)?;
        let filters = FilterRenderer::new(&device, options.filter_texture_budget_bytes);
        let (blit_pipeline, blit_layout) = create_blit_pipeline(&device, config.format);

        Ok(Self {
            size_px: (config.width, config.height),
            presenter: Some(Presenter {
                window,
                surface,
                config,
                blit_pipeline,
                blit_layout,
            }),
            context: WgpuContext { device, queue },
            renderer,
            target: None,
            fonts: FontCache::new(options.font_cache_budget_bytes),
            text_renderer: None,
            images: ImageCache::new(options.image_cache_budget_bytes),
            paths: PathCache::default(),
            filters,
            damage: DamageTracker::default(),
            layer_cache: LayerCache::default(),
            force_full: true,
            font_epoch: None,
            needs_present: true,
            frame_stats: FrameStats::default(),
            textures: ExternalTextures::default(),
            options,
            scale_factor,
            reported: Unsupported::default(),
            presented: false,
            _text: std::marker::PhantomData,
        })
    }

    /// A backend with no window: frames render to an offscreen texture that
    /// [`Self::read_pixels_rgba8`] reads back. This is what lets a test (or a
    /// screenshot tool) compare pixels without a display.
    ///
    /// Falls back to a software adapter when no GPU is available.
    pub fn headless(
        scale_factor: f32,
        options: VelloBackendOptions,
    ) -> Result<Self, VelloBackendError> {
        pollster::block_on(Self::headless_async(scale_factor, options))
    }

    async fn headless_async(
        scale_factor: f32,
        options: VelloBackendOptions,
    ) -> Result<Self, VelloBackendError> {
        let instance = wgpu::Instance::default();
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(_) => {
                instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        force_fallback_adapter: true,
                        ..Default::default()
                    })
                    .await?
            }
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("xui-vello headless device"),
                ..Default::default()
            })
            .await?;
        let renderer = new_renderer(&device)?;
        let filters = FilterRenderer::new(&device, options.filter_texture_budget_bytes);
        Ok(Self {
            presenter: None,
            size_px: (1, 1),
            context: WgpuContext { device, queue },
            renderer,
            target: None,
            fonts: FontCache::new(options.font_cache_budget_bytes),
            text_renderer: None,
            images: ImageCache::new(options.image_cache_budget_bytes),
            paths: PathCache::default(),
            filters,
            damage: DamageTracker::default(),
            layer_cache: LayerCache::default(),
            force_full: true,
            font_epoch: None,
            needs_present: true,
            frame_stats: FrameStats::default(),
            textures: ExternalTextures::default(),
            options,
            scale_factor,
            reported: Unsupported::default(),
            presented: false,
            _text: std::marker::PhantomData,
        })
    }

    /// The last rendered frame as tightly packed RGBA8, row 0 at the top.
    ///
    /// The values are sRGB-encoded and straight (not premultiplied) alpha is
    /// not guaranteed: the frame is opaque whenever the clear colour is.
    pub fn read_pixels_rgba8(&mut self) -> Result<Vec<u8>, VelloBackendError> {
        let target = self.target.as_ref().ok_or(VelloBackendError::NoFrame)?;
        read_texture(&self.context, &target.texture, target.size)
    }

    /// Everything any frame so far asked for that this backend skipped.
    pub const fn unsupported(&self) -> Unsupported {
        self.reported
    }

    /// What the caches are holding. Evictions are applied at the end of each
    /// frame, so this is exact between frames.
    pub fn resource_stats(&self) -> ResourceStats {
        let (filter_textures, filter_texture_bytes) = self.filters.pool_size();
        let (layer_cache_entries, layer_cache_bytes) = self.layer_cache.size(&self.filters);
        let counts = self.layer_cache.counts();
        ResourceStats {
            image_entries: self.images.entries(),
            image_bytes: self.images.bytes(),
            image_builds: self.images.misses(),
            path_entries: self.paths.entries(),
            path_segments: self.paths.segments(),
            font_entries: self.fonts.len(),
            font_bytes: self.fonts.bytes(),
            filter_textures,
            filter_texture_bytes,
            mask_entries: self.filters.mask_count(),
            layer_cache_entries,
            layer_cache_bytes,
            layer_cache_hits: counts.hits,
            layer_cache_misses: counts.misses,
            layer_cache_patches: counts.patches,
            layer_cache_filtered_hits: counts.filtered_hits,
            layer_cache_filtered_misses: counts.filtered_misses,
            layer_cache_filtered_patches: counts.filtered_patches,
            glyph_atlas_entries: self
                .text_renderer
                .as_ref()
                .map_or(0, |renderer| renderer.atlas().len()),
            glyph_atlas_pages: self
                .text_renderer
                .as_ref()
                .map_or(0, |renderer| renderer.atlas().page_count()),
        }
    }

    pub const fn frame_size_px(&self) -> (u32, u32) {
        self.size_px
    }

    /// The device and queue this backend renders on. A canvas GPU painter must
    /// create its textures here.
    pub fn wgpu_context(&self) -> WgpuContext {
        self.context.clone()
    }

    fn ensure_target(&mut self) {
        let size = self.size_px;
        if self.target.as_ref().is_some_and(|target| target.size == size) {
            return;
        }
        let device = &self.context.device;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui-vello scene"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.presenter.as_ref().map(|presenter| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("xui-vello blit"),
                layout: &presenter.blit_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            })
        });
        self.target = Some(SceneTarget {
            texture,
            view,
            bind_group,
            size,
        });
        // A new texture holds nothing: the whole frame has to be drawn into it.
        self.invalidate();
    }

    /// Forgets what is on screen, so the next frame is drawn in full. Needed
    /// when something outside the scene changed it: the window's contents were
    /// lost, or a caller knows a setting (a theme, a font) changed everywhere.
    pub fn invalidate(&mut self) {
        self.damage.clear();
        self.layer_cache.clear(&mut self.filters);
        self.force_full = true;
    }

    /// What the last frames drew.
    pub const fn frame_stats(&self) -> FrameStats {
        self.frame_stats
    }

    fn acquire(&mut self) -> Result<Option<wgpu::SurfaceTexture>, VelloBackendError> {
        let presenter = self.presenter.as_ref().expect("presenting backend");
        presenter.sync_present_mode();
        for attempt in 0..2 {
            match presenter.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => return Ok(Some(frame)),
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost
                    if attempt == 0 =>
                {
                    presenter
                        .surface
                        .configure(&self.context.device, &presenter.config);
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(None);
                }
                _ => return Err(VelloBackendError::AcquireSurface),
            }
        }
        Err(VelloBackendError::AcquireSurface)
    }

    /// Draws `surface` into `dest`.
    ///
    /// A surface that is one band of 2D content, on an opaque clear colour
    /// and followed by nothing but text, is one Vello render straight into
    /// `dest`, with the text drawn over it there. Anything else is composed
    /// here (see [`Self::compose`]) and written out as Vello writes. Both
    /// blend the same way, on encoded values, so which one a frame takes
    /// changes nothing it shows.
    fn render_surface(
        &mut self,
        surface: Surface,
        dest: Dest,
        clear: Color,
    ) -> Result<(), VelloBackendError> {
        if let Some((SurfaceOp::Band(scene), text)) = surface.ops.split_first()
            && (text.is_empty()
                || (opaque(clear) && text.iter().all(|op| matches!(op, SurfaceOp::Text(_)))))
        {
            let view = match dest {
                Dest::Root => &self.target.as_ref().expect("target").view,
                Dest::Slot(slot) => self.filters.view(slot),
            };
            self.renderer.render_to_texture(
                &self.context.device,
                &self.context.queue,
                scene,
                view,
                &render_params(surface.extent, clear),
            )?;
            let batches = text.iter().filter_map(|op| match op {
                SurfaceOp::Text(batch) => Some(batch),
                _ => None,
            });
            self.draw_text(view, ENCODED_FORMAT, batches);
            return Ok(());
        }
        let composed = self.compose(surface, clear, None)?;
        let (device, queue) = (&self.context.device, &self.context.queue);
        match dest {
            Dest::Root => {
                let view = &self.target.as_ref().expect("target").view;
                self.filters.finish(device, queue, composed, Finish::View(view));
            }
            Dest::Slot(slot) => self.filters.finish(device, queue, composed, Finish::Slot(slot)),
        }
        Ok(())
    }

    /// Draws text `batches` over `view`, a target of `format` and of the
    /// extent they were prepared for.
    fn draw_text<'b>(
        &self,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        batches: impl IntoIterator<Item = &'b TextBatch>,
    ) {
        let Some(renderer) = &self.text_renderer else {
            return;
        };
        let mut batches = batches.into_iter().filter(|batch| !batch.is_empty()).peekable();
        if batches.peek().is_none() {
            return;
        }
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("xui-vello text"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui-vello text"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for batch in batches {
                renderer.render(&mut pass, batch, format);
            }
        }
        self.context.queue.submit([encoder.finish()]);
    }

    /// Composes `surface` in paint order, from `clear`, and returns it
    /// premultiplied, in the work space.
    ///
    /// A band is rendered by Vello on transparent and drawn over what is
    /// composed so far; text and textures are drawn over it; a child layer has its own
    /// surface composed first, and then its plan runs with what is composed so
    /// far as both its backdrop and its destination. Nothing goes through
    /// Vello's image atlas.
    ///
    /// `inherited`, for a passthrough layer's content, is what is beneath the
    /// layer, over the surface's pixels: a backdrop in the surface sees the
    /// surface as composed so far over it.
    fn compose(
        &mut self,
        surface: Surface,
        clear: Color,
        inherited: Option<Placed>,
    ) -> Result<Slot, VelloBackendError> {
        let Surface {
            origin_px,
            extent,
            ops,
        } = surface;
        let mut composed: Option<Slot> = None;
        for op in ops {
            let device = &self.context.device;
            let queue = &self.context.queue;
            let below = |filters: &mut FilterRenderer, composed: Option<Slot>| {
                composed.unwrap_or_else(|| filters.clear(device, queue, extent, color_array(clear)))
            };
            composed = Some(match op {
                SurfaceOp::Band(scene) => {
                    let band = self.filters.acquire(device, extent);
                    // The first band starts the composition, so it carries the
                    // clear colour; the rest are on transparent.
                    let base = if composed.is_none() { clear } else { Color::TRANSPARENT };
                    self.renderer.render_to_texture(
                        device,
                        queue,
                        &scene,
                        self.filters.view(band),
                        &render_params(extent, base),
                    )?;
                    match composed {
                        None => self.filters.premultiplied(device, queue, band, extent),
                        Some(below) => self.filters.band_over(device, queue, band, below, extent),
                    }
                }
                SurfaceOp::Text(batch) => {
                    let below = below(&mut self.filters, composed);
                    self.draw_text(self.filters.view(below), WORK_FORMAT, [&batch]);
                    below
                }
                SurfaceOp::Texture { draw, clip_mask } => {
                    let below = below(&mut self.filters, composed);
                    let mask = self.render_clip_mask(clip_mask)?;
                    let out = self.filters.texture_over(
                        &self.context.device,
                        &self.context.queue,
                        below,
                        &draw,
                        mask,
                        extent,
                    );
                    if let Some(mask) = mask {
                        self.filters.release(mask.slot);
                    }
                    out
                }
                SurfaceOp::Layer(job) => {
                    let destination = Placed {
                        slot: below(&mut self.filters, composed),
                        origin: origin_px,
                    };
                    // What the layer's backdrop, and any inside it, see: the
                    // surface so far, over what it inherits from beneath.
                    let reads_beneath = job.plan.backdrop().is_some() || job.inherit.is_some();
                    let seen = match inherited {
                        Some(beneath) if reads_beneath => Some(Placed {
                            slot: self.filters.over(device, queue, destination.slot, beneath.slot, extent),
                            origin: origin_px,
                        }),
                        _ => None,
                    };
                    let backdrop = seen.unwrap_or(destination);
                    let beneath_content = match &job.inherit {
                        Some(inherit) => Some(self.inherited_backdrop(
                            inherit,
                            backdrop,
                            job.backdrop_mask.as_ref(),
                            &job.content,
                        )?),
                        None => None,
                    };
                    let content_origin = job.content.origin_px;
                    let (content, content_origin, cached) = match job.cache {
                        None => (
                            self.compose(job.content, Color::TRANSPARENT, beneath_content)?,
                            content_origin,
                            false,
                        ),
                        Some(use_) => {
                            let drawn = match use_.update {
                                CacheUpdate::Keep => None,
                                CacheUpdate::Fresh | CacheUpdate::Patch => Some((
                                    // A cached layer inherits nothing.
                                    self.compose(job.content, Color::TRANSPARENT, None)?,
                                    content_origin,
                                )),
                            };
                            let slot = self.layer_cache.apply(
                                &self.context.device,
                                &self.context.queue,
                                &mut self.filters,
                                use_,
                                drawn,
                            );
                            (slot, (use_.rect.x, use_.rect.y), true)
                        }
                    };
                    // A cached layer with effects is composited from their
                    // kept result, brought up to date first where the
                    // encoder found it out of date.
                    let (content, content_origin) = match (job.cache, job.cache.and_then(|use_| use_.filter)) {
                        (Some(use_), Some(filter_use)) => {
                            let filtered = match &job.filter {
                                None => None,
                                Some(filter) => {
                                    let region = filter.region;
                                    let origin = (region.x, region.y);
                                    let slot = self.filters.clear(
                                        &self.context.device,
                                        &self.context.queue,
                                        (region.width, region.height),
                                        [0.0; 4],
                                    );
                                    self.filters.execute_plan(
                                        &self.context.device,
                                        &self.context.queue,
                                        &filter.plan,
                                        &PlanTextures {
                                            destination: Placed { slot, origin },
                                            layer_content: Some(Placed {
                                                slot: content,
                                                origin: content_origin,
                                            }),
                                            backdrop: None,
                                            backdrop_mask: None,
                                            layer_masks: &job.layer_masks,
                                            clips: &[],
                                            clip_mask: None,
                                        },
                                        self.scale_factor,
                                    )?;
                                    Some((slot, origin))
                                }
                            };
                            let slot = self.layer_cache.apply_filtered(
                                &self.context.device,
                                &self.context.queue,
                                &mut self.filters,
                                use_.id,
                                filter_use,
                                filtered,
                            );
                            (slot, (filter_use.rect.x, filter_use.rect.y))
                        }
                        _ => (content, content_origin),
                    };
                    let clip_mask = self.render_clip_mask(job.clip_mask)?;
                    let result = self.filters.execute_plan(
                        &self.context.device,
                        &self.context.queue,
                        &job.plan,
                        &PlanTextures {
                            destination,
                            layer_content: Some(Placed {
                                slot: content,
                                origin: content_origin,
                            }),
                            backdrop: Some(backdrop),
                            backdrop_mask: job.backdrop_mask.as_ref(),
                            layer_masks: &job.layer_masks,
                            clips: &job.clips,
                            clip_mask,
                        },
                        self.scale_factor,
                    );
                    if !cached {
                        self.filters.release(content);
                    }
                    for spent in [seen, beneath_content].into_iter().flatten() {
                        self.filters.release(spent.slot);
                    }
                    if let Some(mask) = clip_mask {
                        self.filters.release(mask.slot);
                    }
                    result?;
                    destination.slot
                }
            });
        }
        Ok(match composed {
            Some(slot) => slot,
            None => self.filters.clear(
                &self.context.device,
                &self.context.queue,
                extent,
                color_array(clear),
            ),
        })
    }

    /// What a backdrop inside a passthrough layer sees beneath the layer, over
    /// its content surface: what the layer's own backdrop sees (`backdrop`,
    /// in the parent's pixels), with the layer's backdrop branch over it,
    /// taken into the layer's space.
    fn inherited_backdrop(
        &mut self,
        inherit: &Inherit,
        backdrop: Placed,
        backdrop_mask: Option<&xui_interface::ImageData>,
        content: &Surface,
    ) -> Result<Placed, VelloBackendError> {
        let (device, queue) = (&self.context.device, &self.context.queue);
        let traversed = match &inherit.backdrop_only {
            Some(plan) => {
                let copy = Placed {
                    slot: self.filters.duplicate(device, queue, backdrop.slot),
                    origin: backdrop.origin,
                };
                self.filters.execute_plan(
                    device,
                    queue,
                    plan,
                    &PlanTextures {
                        destination: copy,
                        layer_content: None,
                        backdrop: Some(backdrop),
                        backdrop_mask,
                        layer_masks: &[],
                        clips: &[],
                        clip_mask: None,
                    },
                    self.scale_factor,
                )?;
                Some(copy)
            }
            None => None,
        };
        let out = Placed {
            slot: self.filters.clear(device, queue, content.extent, [0.0; 4]),
            origin: content.origin_px,
        };
        let result = self.filters.execute_plan(
            device,
            queue,
            &inherit.into_layer,
            &PlanTextures {
                destination: out,
                layer_content: Some(traversed.unwrap_or(backdrop)),
                backdrop: None,
                backdrop_mask: None,
                layer_masks: &[],
                clips: &[],
                clip_mask: None,
            },
            self.scale_factor,
        );
        if let Some(copy) = traversed {
            self.filters.release(copy.slot);
        }
        result.map(|()| out)
    }

    /// Renders a clip's coverage, drawn by Vello, for the compositor to read.
    fn render_clip_mask(
        &mut self,
        mask: Option<ClipMask>,
    ) -> Result<Option<Placed>, VelloBackendError> {
        let Some(mask) = mask else {
            return Ok(None);
        };
        let slot = self.filters.acquire(&self.context.device, mask.extent);
        self.renderer.render_to_texture(
            &self.context.device,
            &self.context.queue,
            &mask.scene,
            self.filters.view(slot),
            &render_params(mask.extent, Color::TRANSPARENT),
        )?;
        Ok(Some(Placed {
            slot,
            origin: mask.origin_px,
        }))
    }

    /// Registers a `wgpu::Texture` under `id`, for a canvas to draw with
    /// `CanvasPainter::texture`.
    ///
    /// The texture must come from [`Self::wgpu_context`]'s device, declare
    /// `TEXTURE_BINDING`, and hold premultiplied alpha. It is sampled where it
    /// is, never copied; an sRGB format is read as such. Redrawing into the same
    /// texture needs no call here: bump the command's `revision`. Call again
    /// with the same id only when the texture object changes, as on a resize.
    pub fn set_wgpu_texture(&mut self, id: xui_interface::ExternalTextureId, texture: &wgpu::Texture) {
        self.textures.set(id, texture);
    }

    /// Drops a registration made by [`Self::set_wgpu_texture`]. A canvas still
    /// naming the id draws nothing there.
    pub fn remove_texture(&mut self, id: xui_interface::ExternalTextureId) -> bool {
        self.textures.remove(id)
    }

    /// The part of the frame that has to be drawn again, in physical pixels, or
    /// `None` when nothing changed.
    ///
    /// The tracker reports what the scene changed. Two things widen it: a
    /// layer that reads its backdrop is repainted whole, together with the
    /// pixels its filters read, whenever any of that is touched, because the
    /// frame holds the layer's *result*, not the pixels it was made from.
    fn damaged_region(&mut self, frame: &BuiltFrame, size: (u32, u32)) -> Option<[i32; 4]> {
        // The tracker diffs consecutive frames, so it sees every one.
        let damage = self.damage.update(frame);
        let whole = [0, 0, size.0 as i32, size.1 as i32];
        if std::mem::take(&mut self.force_full) {
            return Some(whole);
        }
        let bounds = damage.bounds()?;
        let scale = self.scale_factor;
        // One pixel of margin: antialiased edges reach past the exact bounds.
        let mut region = [
            ((bounds.min.x * scale).floor() as i32 - 1).max(0),
            ((bounds.min.y * scale).floor() as i32 - 1).max(0),
            ((bounds.max.x * scale).ceil() as i32 + 1).min(whole[2]),
            ((bounds.max.y * scale).ceil() as i32 + 1).min(whole[3]),
        ];
        if region[2] <= region[0] || region[3] <= region[1] {
            return None;
        }
        let Some(crops) = self.backdrop_crops(frame, scale, whole) else {
            return Some(whole);
        };
        loop {
            let mut grew = false;
            for crop in &crops {
                let touches = crop[0] < region[2]
                    && crop[2] > region[0]
                    && crop[1] < region[3]
                    && crop[3] > region[1];
                let contained = crop[0] >= region[0]
                    && crop[1] >= region[1]
                    && crop[2] <= region[2]
                    && crop[3] <= region[3];
                if touches && !contained {
                    region = [
                        region[0].min(crop[0]),
                        region[1].min(crop[1]),
                        region[2].max(crop[2]),
                        region[3].max(crop[3]),
                    ];
                    grew = true;
                }
            }
            if !grew {
                return Some(region);
            }
        }
    }

    /// For each layer that reads its backdrop, the area it reads: where it
    /// paints plus what its filters reach. `None` if any is somewhere this
    /// cannot place (not a direct child of the root), which means "redraw all".
    fn backdrop_crops(
        &self,
        frame: &BuiltFrame,
        scale: f32,
        whole: [i32; 4],
    ) -> Option<Vec<[i32; 4]>> {
        let program = |id| {
            frame
                .layer_instance(id)
                .map(|instance| (instance, instance.render_program.program()))
        };
        let wanted = frame
            .layer_instances
            .iter()
            .filter(|instance| instance.render_program.program().backdrop().is_some())
            .count();
        let mut crops = Vec::new();
        for item in &frame.layers[frame.root_layer.0].items {
            let BuiltItem::Layer(id) = item else { continue };
            let Some((instance, program)) = program(*id) else { continue };
            if program.backdrop().is_none() {
                continue;
            }
            let mut area = instance.world_bounds;
            if let Some(chain) = instance.clip_chain {
                area = (area & frame.clip_chains[chain.0].world_bounds)?;
            }
            let reach = program.backdrop_input_expansion();
            let area = reach.apply_to_bounds(area);
            crops.push([
                ((area.min.x * scale).floor() as i32).max(whole[0]),
                ((area.min.y * scale).floor() as i32).max(whole[1]),
                ((area.max.x * scale).ceil() as i32).min(whole[2]),
                ((area.max.y * scale).ceil() as i32).min(whole[3]),
            ]);
        }
        (crops.len() == wanted).then_some(crops)
    }

    /// Copies a freshly drawn region into the persistent frame.
    fn copy_into_target(&self, slot: Slot, extent: (u32, u32), at: (i32, i32)) {
        let target = self.target.as_ref().expect("target");
        let mut encoder = self
            .context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xui-vello damage copy"),
            });
        encoder.copy_texture_to_texture(
            self.filters.texture(slot).as_image_copy(),
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: at.0 as u32,
                    y: at.1 as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: extent.0,
                height: extent.1,
                depth_or_array_layers: 1,
            },
        );
        self.context.queue.submit([encoder.finish()]);
    }

    fn report(&mut self, found: Unsupported) {
        macro_rules! once {
            ($($field:ident),*) => {$(
                if found.$field && !self.reported.$field {
                    log::warn!("xui-vello: `{}` is not drawn yet", stringify!($field));
                }
            )*};
        }
        once!(missing_font, layer_effect, layer_mask);
        self.reported.merge(found);
    }
}

impl<T: TextBackend> RenderBackend<TextHost<T>> for VelloBackend<T> {
    type Error = VelloBackendError;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
        self.presented = false;
        let width = (size.width * self.scale_factor).round().max(1.0) as u32;
        let height = (size.height * self.scale_factor).round().max(1.0) as u32;
        if (width, height) != self.size_px {
            self.size_px = (width, height);
            if let Some(presenter) = &mut self.presenter {
                presenter.config.width = width;
                presenter.config.height = height;
                presenter
                    .surface
                    .configure(&self.context.device, &presenter.config);
            }
        }
        self.ensure_target();
        Ok(())
    }

    fn submit(&mut self, frame: &BuiltFrame, text: &mut TextHost<T>) -> Result<(), Self::Error> {
        self.filters.begin_frame();
        self.layer_cache.begin_frame();
        let size = self.target.as_ref().expect("begin_frame ensures a target").size;

        // Fonts are not part of the scene: a different set shapes differently.
        let font_epoch = Some(text.font_epoch());
        if self.font_epoch != font_epoch {
            self.font_epoch = font_epoch;
            self.invalidate();
        }

        self.frame_stats.frames += 1;
        let Some(region) = self.damaged_region(frame, size) else {
            // Nothing changed: what is in the target is already right.
            self.frame_stats.unchanged += 1;
            self.frame_stats.last_region = [0; 4];
            self.frame_stats.last_pixels = 0;
            return Ok(());
        };
        let pixels = FrameStats::area(region);
        self.frame_stats.last_region = region;
        self.frame_stats.last_pixels = pixels;
        self.frame_stats.total_pixels += pixels;
        self.needs_present = true;

        let rasterizer = if self.options.bitmap_text {
            text.backend().rasterizer().cloned()
        } else {
            None
        };
        let device = &self.context.device;
        let bitmap = rasterizer.as_deref().map(|rasterizer| {
            let renderer = self.text_renderer.get_or_insert_with(|| {
                TextRenderer::new(
                    device,
                    &[ENCODED_FORMAT, WORK_FORMAT],
                    self.options.glyph_atlas,
                )
            });
            renderer.begin_frame(text.font_epoch());
            BitmapText {
                renderer,
                rasterizer,
                device,
                queue: &self.context.queue,
            }
        });
        let (surface, found) = encode_frame(
            frame,
            self.scale_factor,
            region,
            device.limits().max_texture_dimension_2d,
            &self.layer_cache,
            &self.damage,
            text,
            &mut self.fonts,
            &self.images,
            &mut self.textures,
            &self.paths,
            bitmap,
        );
        self.report(found);
        if let Some(renderer) = &mut self.text_renderer {
            renderer.upload(&self.context.device, &self.context.queue);
        }

        let clear = self.options.clear_color;
        let extent = ((region[2] - region[0]) as u32, (region[3] - region[1]) as u32);
        if region == [0, 0, size.0 as i32, size.1 as i32] {
            self.render_surface(surface, Dest::Root, clear)?;
        } else {
            // Draw the region on a texture of its own and copy it over the
            // same place in the frame, which keeps everything else as it was.
            let slot = self.filters.acquire(&self.context.device, extent);
            self.render_surface(surface, Dest::Slot(slot), clear)?;
            self.copy_into_target(slot, extent, (region[0], region[1]));
            self.filters.release(slot);
        }
        self.layer_cache.finish_frame(
            frame,
            &self.damage,
            &mut self.filters,
            self.options.layer_cache_budget_bytes,
        );
        // Pending evictions apply now, so the budgets hold between frames.
        self.images.settle();
        self.paths.settle();
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        if self.presenter.is_none() || !self.needs_present {
            // Nothing new to show: the window still has the last frame.
            self.presented = true;
            return Ok(());
        }
        let Some(frame) = self.acquire()? else {
            return Ok(());
        };
        let presenter = self.presenter.as_ref().expect("presenting backend");
        let target = self.target.as_ref().expect("begin_frame ensures a target");
        let bind_group = target.bind_group.as_ref().expect("presenting target");
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xui-vello blit"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui-vello blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&presenter.blit_pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.context.queue.submit([encoder.finish()]);
        self.context.queue.present(frame);
        self.needs_present = false;
        self.presented = true;
        Ok(())
    }

    fn did_present(&self) -> bool {
        self.presented
    }

    fn resize(&mut self, _size: Size<f32>) -> Result<(), Self::Error> {
        // `begin_frame` reconfigures from the size it is given.
        Ok(())
    }

    fn set_factor(&mut self, factor: f32) -> Result<(), Self::Error> {
        self.scale_factor = factor;
        self.invalidate();
        Ok(())
    }

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

/// Whether a band drawn on `clear` covers every pixel, so its straight alpha
/// is premultiplied too.
fn opaque(clear: Color) -> bool {
    clear.a >= 1.0
}

/// Vello's parameters for one band of `extent` on `base`.
fn render_params(extent: (u32, u32), base: Color) -> RenderParams {
    RenderParams {
        base_color: AlphaColor::<Srgb>::new(color_array(base)),
        width: extent.0,
        height: extent.1,
        antialiasing_method: AaConfig::Area,
    }
}

fn color_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn new_renderer(device: &wgpu::Device) -> Result<Renderer, VelloBackendError> {
    Ok(Renderer::new(
        device,
        RendererOptions {
            antialiasing_support: AaSupport::area_only(),
            ..RendererOptions::default()
        },
    )?)
}

fn create_blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("xui-vello blit layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("xui-vello blit"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("xui-vello blit"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("xui-vello blit"),
        layout: Some(&pipeline_layout),
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
            targets: &[Some(format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}

#[cfg(test)]
mod tests {
    #[test]
    fn blit_shader_parses_as_wgsl() {
        naga::front::wgsl::parse_str(include_str!("blit.wgsl")).expect("blit.wgsl");
    }
}

/// A texture's pixels as tightly packed rows.
pub(crate) fn read_texture(
    context: &WgpuContext,
    texture: &wgpu::Texture,
    (width, height): (u32, u32),
) -> Result<Vec<u8>, VelloBackendError> {
    let device = &context.device;
    // Rows in a buffer copy are padded to a 256-byte multiple.
    let unpadded = width * 4;
    let padded = unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("xui-vello readback"),
        size: padded as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("xui-vello readback"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    context.queue.submit([encoder.finish()]);

    let (sender, receiver) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|error| VelloBackendError::Readback(error.to_string()))?;
    receiver
        .recv()
        .map_err(|error| VelloBackendError::Readback(error.to_string()))?
        .map_err(|error| VelloBackendError::Readback(error.to_string()))?;

    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|error| VelloBackendError::Readback(error.to_string()))?;
    let mut pixels = Vec::with_capacity((unpadded * height) as usize);
    for row in mapped.chunks_exact(padded as usize) {
        pixels.extend_from_slice(&row[..unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(pixels)
}
