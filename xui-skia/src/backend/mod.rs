//! The Skia render backend, split by concern.
//!
//! [`SkiaBackend`] is one type with one large surface; the work it does falls
//! into groups that barely touch each other, so the inherent `impl` is spread
//! over the modules below and this file keeps only the type itself, its
//! construction and the [`RenderBackend`] implementation.
//!
//! - [`surface`] — surface allocation, the recycling pool, presentation blits
//!   and the damage-to-pixels mapping.
//! - [`frame`] — the per-frame walk: analysis, then layer and item drawing.
//! - [`plan`] — executing a layer's render plan (filter and composite passes,
//!   backdrops, masks).
//! - [`effects`] — the SkSL sources and the runtime effect cache.
//! - [`image`] — image decoding, the image caches and image drawing.
//! - [`text`] — typeface resolution, text blobs and glyph drawing.
//! - [`vector`] — shape, path and vector-scene drawing.
//! - [`paint`] — turning a style into a `Paint`, including gradients.
//! - [`convert`] — small `xui-interface` to `skia-safe` conversions.
//! - [`lru`] — the bounded map the per-backend caches are built from.

mod convert;
mod effects;
mod frame;
#[cfg(test)]
mod frame_tests;
mod image;
mod lru;
mod paint;
mod plan;
mod surface;
mod text;
mod vector;

use moka::sync::Cache;
use rustc_hash::FxHashMap;
use skia_safe::{
    AlphaType, ColorSpace, ColorType, FontMgr, Image, ImageInfo, Path, RuntimeEffect, Shader,
    Surface, Typeface as SkTypeface,
};
use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroU32,
    sync::{Arc, LazyLock},
};
use winit::window::Window;
use xui::{
    render::{BuiltFrame, RenderBackend},
    text::{TextHost, TextLayoutHandle},
};
use xui_interface::{
    Color, FontDatabase, FontWeight, ImageKey, NodeLifecycleEvent, PathDataId, Size, TextBackend,
    VectorSceneId,
};

use self::{
    convert::valid_scale,
    image::{CachedImageKey, CachedSourceImage, image_cache, source_image_cache},
    lru::LocalLru,
    paint::GradientKey,
    surface::{SurfacePool, copy_surface_damage, full_softbuffer_rect, physical_damage_rects},
    text::CachedTextBlob,
    vector::CompiledVectorCommand,
};
use crate::{
    SkiaBackendError, SkiaFrameStats, SkiaLayerCacheStats,
    cache::LayerSurfaceCache,
    damage::{DamageRegion, DamageTracker},
    present::WindowPresenter,
};

/// `XUI_DEBUG_FRAME=1` prints a per-frame damage summary. Read once: an env
/// lookup takes the process-wide env lock and allocates.
static DEBUG_FRAME: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("XUI_DEBUG_FRAME").is_some());

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkiaBackendOptions {
    pub clear_color: Color,
    pub layer_cache_budget_bytes: u64,
    /// Ceiling on the idle offscreen surfaces kept for reuse between frames.
    pub surface_pool_budget_bytes: u64,
    pub optimizations: SkiaOptimizations,
}

impl Default for SkiaBackendOptions {
    fn default() -> Self {
        Self {
            clear_color: Color::rgba(0.08, 0.09, 0.11, 1.0),
            layer_cache_budget_bytes: 128 * 1024 * 1024,
            surface_pool_budget_bytes: 64 * 1024 * 1024,
            optimizations: SkiaOptimizations::default(),
        }
    }
}

/// Switches for the frame-level drawing optimizations.
///
/// All are on by default. They exist as switches so the tests can render one
/// frame both ways and compare the pixels, which is the only cheap way to keep
/// a fast path honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkiaOptimizations {
    /// Skip draw items and layer instances that fall outside the repainted
    /// region. Correctness rests on the damage tracker covering every change,
    /// which is the contract it already has to meet for the cached-layer path.
    pub cull: bool,
    /// Recycle offscreen surfaces within a frame instead of allocating each.
    pub surface_pool: bool,
}

impl Default for SkiaOptimizations {
    fn default() -> Self {
        Self {
            cull: true,
            surface_pool: true,
        }
    }
}

impl SkiaOptimizations {
    /// Every fast path off — the reference drawing path.
    pub const NONE: Self = Self {
        cull: false,
        surface_pool: false,
    };
}

pub struct SkiaBackend<T: TextBackend = crate::SkiaTextBackend> {
    presenter: Option<WindowPresenter>,
    /// Where the frame is presented from: the raster surface on the software
    /// path, the acquired swapchain image on the GPU path.
    raster: Option<Surface>,
    /// The GPU path's persistent frame store.
    ///
    /// A swapchain image comes back with undefined contents, so drawing
    /// straight into one forces a full repaint every frame and throws away
    /// everything the damage tracker computed. Scene drawing goes to this
    /// surface instead, which does keep its pixels, and the finished frame is
    /// blitted to the swapchain image whole. A full-frame blit costs one
    /// screen-sized textured quad; a full-frame repaint costs the scene.
    compositor: Option<Surface>,
    options: SkiaBackendOptions,
    scale_factor: f32,
    frame_size_px: Size<u32>,
    gpu_context: Option<skia_safe::gpu::DirectContext>,
    image_cache: Cache<CachedImageKey, Image>,
    source_images: Cache<ImageKey, CachedSourceImage>,
    vector_paths: LocalLru<PathDataId, Path>,
    vector_scenes: LocalLru<VectorSceneId, Arc<[CompiledVectorCommand]>>,
    gradients: LocalLru<GradientKey, Shader>,
    runtime_effects: FxHashMap<&'static str, RuntimeEffect>,
    damage_tracker: DamageTracker,
    /// Set once `submit` has mutated the damage tracker for a frame that has
    /// not been presented yet, so a failed present can invalidate it.
    damage_rollback_pending: bool,
    layer_cache: LayerSurfaceCache,
    surface_pool: SurfacePool,
    /// A 1x1 transparent pixel, stretched wherever an empty image is needed.
    empty_image: Option<Image>,
    pending_damage: DamageRegion,
    damage_history: VecDeque<Vec<softbuffer::Rect>>,
    // Font Cache
    font_cache: FxHashMap<(<T as FontDatabase>::FontId, FontWeight), SkTypeface>,
    font_cache_epoch: Option<u64>,
    /// Built once and reused. `FontMgr::new()` is a CoreText enumeration on
    /// macOS and costs ~30 ms a call, which used to be paid per typeface cache
    /// miss — 300 ms of the first frame, for ten distinct fonts.
    font_mgr: Option<FontMgr>,
    text_blob_cache: FxHashMap<TextLayoutHandle, CachedTextBlob>,
    frame_stats: SkiaFrameStats,
    frame_index: u64,
    submitted: bool,
    presented: bool,
}

impl<T: TextBackend> SkiaBackend<T> {
    pub fn new(window: Arc<Window>) -> Result<Self, SkiaBackendError> {
        Self::new_with_options(window, SkiaBackendOptions::default())
    }

    pub fn new_with_options(
        window: Arc<Window>,
        options: SkiaBackendOptions,
    ) -> Result<Self, SkiaBackendError> {
        let scale_factor = window.scale_factor() as f32;
        let (presenter, gpu_context) = WindowPresenter::new(window)?;
        Ok(Self {
            presenter: Some(presenter),
            raster: None,
            compositor: None,
            options,
            scale_factor,
            frame_size_px: Size::new(0, 0),
            gpu_context,
            image_cache: image_cache(),
            source_images: source_image_cache(),
            vector_paths: LocalLru::new(4096),
            vector_scenes: LocalLru::new(1024),
            gradients: LocalLru::new(256),
            runtime_effects: HashMap::default(),
            damage_tracker: DamageTracker::default(),
            damage_rollback_pending: false,
            layer_cache: LayerSurfaceCache::default(),
            surface_pool: SurfacePool::default(),
            empty_image: None,
            pending_damage: DamageRegion::default(),
            damage_history: VecDeque::new(),
            font_cache: HashMap::default(),
            font_cache_epoch: None,
            font_mgr: None,
            text_blob_cache: HashMap::default(),
            frame_stats: SkiaFrameStats::default(),
            frame_index: 0,
            submitted: false,
            presented: false,
        })
    }

    pub fn headless(scale_factor: f32, options: SkiaBackendOptions) -> Self {
        Self {
            presenter: None,
            raster: None,
            compositor: None,
            options,
            scale_factor: valid_scale(scale_factor),
            frame_size_px: Size::new(0, 0),
            gpu_context: None,
            image_cache: image_cache(),
            source_images: source_image_cache(),
            vector_paths: LocalLru::new(4096),
            vector_scenes: LocalLru::new(1024),
            gradients: LocalLru::new(256),
            runtime_effects: HashMap::default(),
            damage_tracker: DamageTracker::default(),
            damage_rollback_pending: false,
            layer_cache: LayerSurfaceCache::default(),
            surface_pool: SurfacePool::default(),
            empty_image: None,
            pending_damage: DamageRegion::default(),
            damage_history: VecDeque::new(),
            font_cache: HashMap::default(),
            font_cache_epoch: None,
            font_mgr: None,
            text_blob_cache: HashMap::default(),
            frame_stats: SkiaFrameStats::default(),
            frame_index: 0,
            submitted: false,
            presented: false,
        }
    }

    pub const fn frame_size_px(&self) -> Size<u32> {
        self.frame_size_px
    }

    pub const fn is_gpu_accelerated(&self) -> bool {
        self.gpu_context.is_some()
    }

    pub fn layer_cache_stats(&self) -> SkiaLayerCacheStats {
        self.layer_cache.stats()
    }

    pub const fn frame_stats(&self) -> SkiaFrameStats {
        self.frame_stats
    }

    /// Drops every incremental-rendering assumption, so the next frame
    /// repaints in full.
    ///
    /// Needed whenever something outside this backend may have changed what is
    /// on the surface — and by the tests, to render a reference frame that owes
    /// nothing to the frames before it.
    pub fn invalidate(&mut self) {
        // The compositor surface is kept: the next frame repaints it in full,
        // and reallocating a screen-sized surface on every invalidation would
        // cost more than the repaint it is there to avoid.
        self.damage_tracker.clear();
        self.layer_cache.clear();
        self.surface_pool.clear();
        self.damage_history.clear();
        self.damage_rollback_pending = false;
    }

    pub fn read_pixels_rgba8(&mut self) -> Result<Vec<u8>, SkiaBackendError> {
        let width = self.frame_size_px.width;
        let height = self.frame_size_px.height;
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }
        let mut pixels = vec![0; width as usize * height as usize * 4];
        let info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        let surface = self.raster.as_mut().ok_or(SkiaBackendError::PixelRead)?;
        if !surface.read_pixels(&info, &mut pixels, width as usize * 4, (0, 0)) {
            return Err(SkiaBackendError::PixelRead);
        }
        Ok(pixels)
    }
}

impl<T: TextBackend> RenderBackend<TextHost<T>> for SkiaBackend<T> {
    type Error = SkiaBackendError;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
        self.ensure_surface(size)?;
        self.frame_index = self.frame_index.wrapping_add(1).max(1);
        self.frame_stats = SkiaFrameStats {
            frame_index: self.frame_index,
            ..SkiaFrameStats::default()
        };
        if self.frame_index.is_multiple_of(120) {
            let oldest = self.frame_index.saturating_sub(120);
            self.text_blob_cache
                .retain(|_, cached| cached.last_used_frame >= oldest);
        }
        if let (Some(presenter), Some(context)) =
            (self.presenter.as_mut(), self.gpu_context.as_mut())
        {
            self.raster = Some(presenter.acquire_surface(
                context,
                self.frame_size_px.width,
                self.frame_size_px.height,
            )?);
        }
        self.surface_pool.begin_frame();
        self.submitted = false;
        self.presented = false;
        self.damage_rollback_pending = false;
        Ok(())
    }

    fn submit(&mut self, frame: &BuiltFrame, text: &mut TextHost<T>) -> Result<(), Self::Error> {
        // The tracker rebuilds its snapshot map from scratch, so cloning it
        // first only duplicated a full scene snapshot per frame. A failed frame
        // clears it instead of restoring it: the target surface has already
        // been partially written, so the only sound recovery is a full repaint.
        let root_damage = self.damage_tracker.update(frame);
        if *DEBUG_FRAME {
            let root_layer = &frame.layers[frame.root_layer.0];
            eprintln!(
                "[frame {}] frame_size_px={:?} root_layer render_bounds={:?} items={} root_damage_rects={} root_damage_area_sum={:.0} gpu={}",
                self.frame_index,
                self.frame_size_px,
                root_layer.render_bounds,
                root_layer.items.len(),
                root_damage.rects().len(),
                root_damage
                    .rects()
                    .iter()
                    .map(|r| r.width() * r.height())
                    .sum::<f32>(),
                self.gpu_context.is_some(),
            );
        }
        self.frame_stats.root_damage_rects = root_damage.rects().len();
        self.frame_stats.root_damage_area_sum = root_damage
            .rects()
            .iter()
            .map(|rect| rect.width() * rect.height())
            .sum();
        self.layer_cache
            .begin_frame(self.damage_tracker.dirty_region_count());
        // On the GPU path the scene is drawn to the persistent compositor
        // surface and blitted to the acquired swapchain image afterwards; on
        // the software path the raster surface is itself persistent.
        let on_gpu = self.gpu_context.is_some();
        let mut surface = if on_gpu {
            self.compositor.take()
        } else {
            self.raster.take()
        }
        .ok_or_else(|| {
            SkiaBackendError::InvalidFrame("begin_frame must be called before submit".into())
        })?;
        let mut result = self.draw_frame(&mut surface, frame, &root_damage, text);
        if result.is_ok() && on_gpu {
            result = self.blit_to_swapchain(&mut surface);
        }
        if on_gpu {
            self.compositor = Some(surface);
        } else {
            self.raster = Some(surface);
        }
        if let Err(error) = result {
            self.damage_tracker.clear();
            self.layer_cache.clear();
            self.surface_pool.clear();
            return Err(error);
        }
        self.layer_cache.finish_frame(
            &frame.live_layer_caches,
            self.options.layer_cache_budget_bytes,
        );
        self.damage_rollback_pending = true;
        self.pending_damage = root_damage;
        self.submitted = true;
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        if !self.submitted {
            return Err(SkiaBackendError::InvalidFrame(
                "submit must be called before end_frame".into(),
            ));
        }
        if let (Some(presenter), Some(context)) =
            (self.presenter.as_mut(), self.gpu_context.as_mut())
        {
            let surface = self.raster.take().ok_or_else(|| {
                SkiaBackendError::InvalidFrame("the GPU frame surface is unavailable".into())
            })?;
            presenter.present(context, surface)?;
            self.damage_rollback_pending = false;
            self.pending_damage = DamageRegion::default();
            self.presented = true;
            return Ok(());
        }

        {
            let present_result = (|| {
                if self.presenter.is_some() && !self.pending_damage.is_empty() {
                    let current_damage = physical_damage_rects(
                        &self.pending_damage,
                        self.scale_factor,
                        self.frame_size_px,
                    );
                    if current_damage.is_empty() {
                        return Ok(());
                    }
                    let width = NonZeroU32::new(self.frame_size_px.width).ok_or_else(|| {
                        SkiaBackendError::InvalidFrame("frame width is zero".into())
                    })?;
                    let height = NonZeroU32::new(self.frame_size_px.height).ok_or_else(|| {
                        SkiaBackendError::InvalidFrame("frame height is zero".into())
                    })?;
                    let presenter = self
                        .presenter
                        .as_mut()
                        .and_then(WindowPresenter::software_mut)
                        .ok_or_else(|| {
                            SkiaBackendError::InvalidFrame(
                                "no software presenter to blit the frame into".into(),
                            )
                        })?;
                    presenter.surface.resize(width, height)?;
                    let mut buffer = presenter.surface.buffer_mut()?;
                    let age = usize::from(buffer.age());
                    let mut copy_damage = current_damage.clone();
                    if age == 0 || age > self.damage_history.len() + 1 {
                        copy_damage = vec![full_softbuffer_rect(self.frame_size_px)?];
                    } else {
                        for historical in self.damage_history.iter().take(age.saturating_sub(1)) {
                            copy_damage.extend(historical.iter().copied());
                        }
                    }
                    let raster = self.raster.as_mut().ok_or(SkiaBackendError::PixelRead)?;
                    copy_surface_damage(
                        raster,
                        &mut buffer,
                        self.frame_size_px.width,
                        &copy_damage,
                    )?;
                    buffer.present_with_damage(&current_damage)?;
                    self.damage_history.push_front(current_damage);
                    self.damage_history.truncate(8);
                }
                Ok::<_, SkiaBackendError>(())
            })();
            if let Err(error) = present_result {
                if std::mem::take(&mut self.damage_rollback_pending) {
                    self.damage_tracker.clear();
                }
                self.layer_cache.clear();
                return Err(error);
            }
        }
        self.damage_rollback_pending = false;
        self.presented = true;
        Ok(())
    }

    fn did_present(&self) -> bool {
        self.presented
    }

    fn resize(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
        self.ensure_surface(size)
    }

    fn set_factor(&mut self, factor: f32) -> Result<(), Self::Error> {
        self.scale_factor = valid_scale(factor);
        self.raster = None;
        self.compositor = None;
        self.frame_size_px = Size::new(0, 0);
        self.damage_tracker.clear();
        self.damage_rollback_pending = false;
        self.layer_cache.clear();
        self.surface_pool.clear();
        self.damage_history.clear();
        Ok(())
    }

    fn handle_node_lifecycle(&mut self, event: &NodeLifecycleEvent) {
        if let NodeLifecycleEvent::Removed(owner) = event {
            self.text_blob_cache
                .retain(|_, cached| cached.owner != *owner);
        }
    }
}
