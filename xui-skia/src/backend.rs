use moka::sync::Cache;
#[cfg(not(target_os = "macos"))]
use std::num::NonZeroU32;
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    hash::Hash,
    sync::Arc,
};

use skia_safe::{
    AlphaType, BlendMode as SkBlendMode, Canvas, ClipOp, Color4f, ColorSpace, ColorType, Data,
    Font, FontMgr, FontStyle as SkFontStyle, GlyphId as SkGlyphId, IRect, Image, ImageFilter,
    ImageInfo, Matrix, Paint, PaintStyle, Path, PathBuilder, Point as SkPoint, RRect,
    Rect as SkRect, Region, RuntimeEffect, SamplingOptions, Surface, TextBlob, TextBlobBuilder,
    TileMode, Typeface as SkTypeface,
    gradient::{self, Colors, Gradient, Interpolation},
    images,
    paint::{Cap as SkCap, Join as SkJoin},
    runtime_effect::RuntimeShaderBuilder,
};
#[cfg(not(target_os = "macos"))]
use softbuffer::{Context, Surface as SoftSurface};
use winit::window::Window;
use xui::render::render_graph::ImageResource;
use xui::{
    render::{
        BackdropIsolation, BuiltClipChainId, BuiltDraw, BuiltFrame, BuiltItem, BuiltLayerId,
        BuiltLayerInstance, ClipShape, RenderBackend, Shape,
    },
    text::{TextHost, TextLayoutHandle},
};
use xui_interface::{
    Affine, Alignment, Bounds, Color, ComputedColorStyle, FontDataRef, FontDatabase, FontWeight,
    ImageData, ImageFit, ImageKey, ImageRepeat, ImageRotation, ImageStyle, ImageTransform, LineCap,
    LineJoin, NodeId, NodeLifecycleEvent, ParagraphLayout, PathData, PathDataId, PathFill,
    PathSegment, PathStroke, Rect, Sampling, Shaper, Size, TextBackend, TextVerticalAlign,
    VectorCommand, VectorScene, VectorSceneId,
};
use xui_render_graph::{
    BlendMode, CompositeOperator, ExternalAliasing, ExternalResourceKind, LayerPlanContext,
    LayerProgramEntry, LayerRenderPlan, MaskShape, Pass, PassOp, PlanLimits, PlanMask,
    PlanResourceId, PlanResourceKind, TextureClass,
};

use crate::{
    SkiaBackendError, SkiaFrameStats, SkiaLayerCacheStats,
    cache::LayerSurfaceCache,
    damage::{DamageRegion, DamageTracker},
    text::sk_font_style,
};

#[cfg(target_os = "macos")]
use crate::metal::MetalPresenter as WindowPresenter;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkiaBackendOptions {
    pub clear_color: Color,
    pub layer_cache_budget_bytes: u64,
}

impl Default for SkiaBackendOptions {
    fn default() -> Self {
        Self {
            clear_color: Color::rgba(0.08, 0.09, 0.11, 1.0),
            layer_cache_budget_bytes: 128 * 1024 * 1024,
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct WindowPresenter {
    _window: Arc<Window>,
    _context: Context<Arc<Window>>,
    surface: SoftSurface<Arc<Window>, Arc<Window>>,
}

#[cfg(not(target_os = "macos"))]
impl WindowPresenter {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CachedImageKey {
    data: u64,
    transform: ImageTransform,
    bytes: u32,
}

#[derive(Clone)]
struct RasterImage {
    image: Image,
    bounds: Bounds,
}

#[derive(Clone)]
struct CachedSourceImage {
    data_id: u64,
    image: Image,
    bytes: u32,
}

// Source and transformed images share the overall 256 MiB backend budget.
const IMAGE_CACHE_POOL_BUDGET_BYTES: u64 = 128 * 1024 * 1024;

fn image_cache() -> Cache<CachedImageKey, Image> {
    Cache::builder()
        .max_capacity(IMAGE_CACHE_POOL_BUDGET_BYTES)
        .weigher(|key: &CachedImageKey, _image: &Image| key.bytes)
        .build()
}

fn source_image_cache() -> Cache<ImageKey, CachedSourceImage> {
    Cache::builder()
        .max_capacity(IMAGE_CACHE_POOL_BUDGET_BYTES)
        .weigher(|_key: &ImageKey, image: &CachedSourceImage| image.bytes)
        .build()
}

fn image_bytes(data: &ImageData) -> u32 {
    u32::try_from(data.pixels.len()).unwrap_or(u32::MAX)
}

#[derive(Clone)]
enum CompiledVectorCommand {
    FillPath {
        path: Path,
        transform: Affine,
        fill: PathFill,
    },
    StrokePath {
        path: Path,
        transform: Affine,
        stroke: PathStroke,
    },
}

struct LocalLru<K, V> {
    entries: HashMap<K, (V, u64)>,
    capacity: usize,
    clock: u64,
}

impl<K: Copy + Eq + Hash, V: Clone> LocalLru<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            clock: 0,
        }
    }

    fn get(&mut self, key: &K) -> Option<V> {
        self.clock = self.clock.wrapping_add(1);
        let (value, used) = self.entries.get_mut(key)?;
        *used = self.clock;
        Some(value.clone())
    }

    fn insert(&mut self, key: K, value: V) {
        self.clock = self.clock.wrapping_add(1);
        self.entries.insert(key, (value, self.clock));
        if self.entries.len() > self.capacity
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key)
        {
            self.entries.remove(&oldest);
        }
    }
}

struct CachedTextBlob {
    blob: Option<TextBlob>,
    font_epoch: u64,
    owner: NodeId,
    last_used_frame: u64,
}

struct BackdropRequirements {
    layers: Vec<bool>,
}

impl BackdropRequirements {
    fn for_frame(frame: &BuiltFrame) -> Self {
        fn visit(
            frame: &BuiltFrame,
            layer_index: usize,
            memo: &mut [Option<bool>],
            visiting: &mut [bool],
        ) -> bool {
            if let Some(value) = memo.get(layer_index).copied().flatten() {
                return value;
            }
            let Some(layer) = frame.layers.get(layer_index) else {
                return false;
            };
            if visiting.get(layer_index).copied().unwrap_or(true) {
                return false;
            }
            visiting[layer_index] = true;
            let needs_backdrop = layer.items.iter().any(|item| {
                let BuiltItem::Layer(instance_id) = item else {
                    return false;
                };
                let Some(instance) = frame.layer_instance(*instance_id) else {
                    return false;
                };
                if instance
                    .render_program
                    .program()
                    .external_resource(ExternalResourceKind::Backdrop)
                    .is_some()
                {
                    return true;
                }
                frame.layers.get(instance.layer.0).is_some_and(|child| {
                    child.backdrop_isolation == BackdropIsolation::Passthrough
                        && visit(frame, instance.layer.0, memo, visiting)
                })
            });
            visiting[layer_index] = false;
            memo[layer_index] = Some(needs_backdrop);
            needs_backdrop
        }

        let mut memo = vec![None; frame.layers.len()];
        let mut visiting = vec![false; frame.layers.len()];
        for index in 0..frame.layers.len() {
            visit(frame, index, &mut memo, &mut visiting);
        }
        Self {
            layers: memo.into_iter().map(Option::unwrap_or_default).collect(),
        }
    }

    fn layer(&self, id: BuiltLayerId) -> bool {
        self.layers.get(id.0).copied().unwrap_or(false)
    }
}

pub struct SkiaBackend<T: TextBackend = crate::SkiaTextBackend> {
    presenter: Option<WindowPresenter>,
    raster: Option<Surface>,
    options: SkiaBackendOptions,
    scale_factor: f32,
    frame_size_px: Size<u32>,
    gpu_context: Option<skia_safe::gpu::DirectContext>,
    image_cache: Cache<CachedImageKey, Image>,
    source_images: Cache<ImageKey, CachedSourceImage>,
    vector_paths: LocalLru<PathDataId, Path>,
    vector_scenes: LocalLru<VectorSceneId, Arc<[CompiledVectorCommand]>>,
    runtime_effects: HashMap<&'static str, RuntimeEffect>,
    damage_tracker: DamageTracker,
    rollback_damage_tracker: Option<DamageTracker>,
    layer_cache: LayerSurfaceCache,
    pending_damage: DamageRegion,
    damage_history: VecDeque<Vec<softbuffer::Rect>>,
    // Font Cache
    font_cache: HashMap<(<T as FontDatabase>::FontId, FontWeight), SkTypeface>,
    font_cache_epoch: Option<u64>,
    /// Built once and reused. `FontMgr::new()` is a CoreText enumeration on
    /// macOS and costs ~30 ms a call, which used to be paid per typeface cache
    /// miss — 300 ms of the first frame, for ten distinct fonts.
    font_mgr: Option<FontMgr>,
    text_blob_cache: HashMap<TextLayoutHandle, CachedTextBlob>,
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
        #[cfg(target_os = "macos")]
        let (presenter, gpu_context) = {
            let (presenter, context) = WindowPresenter::new(window)?;
            (presenter, Some(context))
        };
        #[cfg(not(target_os = "macos"))]
        let (presenter, gpu_context) = (WindowPresenter::new(window)?, None);
        Ok(Self {
            presenter: Some(presenter),
            raster: None,
            options,
            scale_factor,
            frame_size_px: Size::new(0, 0),
            gpu_context,
            image_cache: image_cache(),
            source_images: source_image_cache(),
            vector_paths: LocalLru::new(4096),
            vector_scenes: LocalLru::new(1024),
            runtime_effects: HashMap::new(),
            damage_tracker: DamageTracker::default(),
            rollback_damage_tracker: None,
            layer_cache: LayerSurfaceCache::default(),
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
            options,
            scale_factor: valid_scale(scale_factor),
            frame_size_px: Size::new(0, 0),
            gpu_context: None,
            image_cache: image_cache(),
            source_images: source_image_cache(),
            vector_paths: LocalLru::new(4096),
            vector_scenes: LocalLru::new(1024),
            runtime_effects: HashMap::new(),
            damage_tracker: DamageTracker::default(),
            rollback_damage_tracker: None,
            layer_cache: LayerSurfaceCache::default(),
            pending_damage: DamageRegion::default(),
            damage_history: VecDeque::new(),
            font_cache: HashMap::new(),
            font_cache_epoch: None,
            font_mgr: None,
            text_blob_cache: HashMap::new(),
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

    fn ensure_surface(&mut self, logical: Size<f32>) -> Result<(), SkiaBackendError> {
        let width = (logical.width.max(0.0) * self.scale_factor).ceil().max(1.0) as u32;
        let height = (logical.height.max(0.0) * self.scale_factor)
            .ceil()
            .max(1.0) as u32;
        if self.gpu_context.is_some() {
            if self.frame_size_px != Size::new(width, height) {
                self.frame_size_px = Size::new(width, height);
                #[cfg(target_os = "macos")]
                if let Some(presenter) = self.presenter.as_ref() {
                    presenter.resize(width, height);
                }
                self.damage_tracker.clear();
                self.layer_cache.clear();
                self.damage_history.clear();
            }
            return Ok(());
        }
        if self.frame_size_px != Size::new(width, height) || self.raster.is_none() {
            self.raster = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32));
            if self.raster.is_none() {
                return Err(SkiaBackendError::SurfaceAllocation { width, height });
            }
            self.frame_size_px = Size::new(width, height);
            self.damage_tracker.clear();
            self.layer_cache.clear();
            self.damage_history.clear();
        }
        Ok(())
    }

    fn new_surface(&mut self, bounds: Bounds) -> Result<Surface, SkiaBackendError> {
        let width = (bounds.width().max(0.0) * self.scale_factor)
            .ceil()
            .max(1.0) as u32;
        let height = (bounds.height().max(0.0) * self.scale_factor)
            .ceil()
            .max(1.0) as u32;
        self.new_surface_px(width, height)
    }

    fn new_surface_px(&mut self, width: u32, height: u32) -> Result<Surface, SkiaBackendError> {
        self.frame_stats.offscreen_surface_allocations += 1;
        new_surface_px(width, height, self.gpu_context.as_mut())
    }

    fn transparent_image(&mut self, bounds: Bounds) -> Result<RasterImage, SkiaBackendError> {
        // let bounds = non_empty_bounds(bounds);
        let mut surface = self.new_surface(bounds)?;
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        Ok(self.snapshot_target(&mut surface, bounds))
    }

    fn validate_frame(&self, frame: &BuiltFrame) -> Result<(), SkiaBackendError> {
        for (index, instance) in frame.layer_instances.iter().enumerate() {
            if instance
                .render_program
                .program()
                .external_resource(ExternalResourceKind::Backdrop)
                .is_some()
            {
                let prefix = instance.destination_prefix.ok_or_else(|| {
                    SkiaBackendError::InvalidFrame(format!(
                        "backdrop layer instance {index} has no destination prefix"
                    ))
                })?;
                frame.composite_prefix(prefix).ok_or_else(|| {
                    SkiaBackendError::InvalidFrame(format!(
                        "backdrop layer instance {index} references a missing destination prefix"
                    ))
                })?;
            }
        }
        Ok(())
    }

    fn draw_frame(
        &mut self,
        surface: &mut Surface,
        frame: &BuiltFrame,
        damage: &DamageRegion,
        text: &mut TextHost<T>,
    ) -> Result<(), SkiaBackendError> {
        self.validate_frame(frame)?;
        self.prepare_frame_images(frame)?;
        let backdrop_requirements = BackdropRequirements::for_frame(frame);
        let viewport =
            Bounds::from_zero_size(self.frame_size_px().to_f32().unwrap() / self.scale_factor);
        if !damage.is_empty() {
            self.redraw_layer_region(
                surface,
                viewport,
                frame,
                frame.root_layer,
                None,
                damage,
                self.options.clear_color,
                text,
                &backdrop_requirements,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn redraw_layer_region(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        frame: &BuiltFrame,
        layer_id: BuiltLayerId,
        inherited_backdrop: Option<&RasterImage>,
        damage: &DamageRegion,
        clear_color: Color,
        text: &mut TextHost<T>,
        backdrop_requirements: &BackdropRequirements,
    ) -> Result<(), SkiaBackendError> {
        let region = damage_region(target, target_bounds, self.scale_factor, damage);
        if region.is_empty() {
            return Ok(());
        }
        let canvas = target.canvas();
        let save = canvas.save();
        canvas.reset_matrix();
        canvas.clip_region(&region, None);
        canvas.clear(sk_color(clear_color));
        self.draw_layer(
            target,
            target_bounds,
            frame,
            layer_id,
            inherited_backdrop,
            text,
            backdrop_requirements,
        )?;
        target.canvas().restore_to_count(save);
        Ok(())
    }

    fn prepare_frame_images(&mut self, frame: &BuiltFrame) -> Result<(), SkiaBackendError> {
        for layer in &frame.layers {
            for item in &layer.items {
                let BuiltItem::Draw(BuiltDraw::Image(draw)) = item else {
                    continue;
                };
                let primitive = &draw.primitive;
                if primitive.data.size.width == 0 || primitive.data.size.height == 0 {
                    continue;
                }
                let stale = self
                    .source_images
                    .get(&primitive.image)
                    .is_none_or(|cached| cached.data_id != primitive.data.id().raw());
                if stale {
                    self.source_images.insert(
                        primitive.image.clone(),
                        CachedSourceImage {
                            data_id: primitive.data.id().raw(),
                            image: make_image(&primitive.data, ImageTransform::default())?,
                            bytes: image_bytes(&primitive.data),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    fn draw_layer(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        frame: &BuiltFrame,
        layer_id: BuiltLayerId,
        inherited_backdrop: Option<&RasterImage>,
        text: &mut TextHost<T>,
        backdrop_requirements: &BackdropRequirements,
    ) -> Result<(), SkiaBackendError> {
        let layer = frame.layers.get(layer_id.0).ok_or_else(|| {
            SkiaBackendError::InvalidFrame(format!("missing layer {}", layer_id.0))
        })?;
        self.frame_stats.layer_draws += 1;
        for (item_index, item) in layer.items.iter().enumerate() {
            match item {
                BuiltItem::Draw(draw) => {
                    self.frame_stats.primitive_draws += 1;
                    let common = draw.common();
                    let canvas = target.canvas();
                    let save = canvas.save();
                    configure_canvas(canvas, target_bounds, self.scale_factor);
                    self.apply_clip_chain(canvas, frame, common.clip_chain, Affine::IDENTITY)?;
                    let transform = common.world_transform;
                    match draw {
                        BuiltDraw::Shape(value) => {
                            draw_shape(canvas, &value.primitive, transform, 1.0)
                        }
                        BuiltDraw::Vector(value) => {
                            let commands = self.compiled_vector_scene(&value.primitive.scene);
                            draw_vector(
                                canvas,
                                &commands,
                                value.primitive.transform,
                                transform,
                                1.0,
                            )
                        }
                        BuiltDraw::Image(value) => {
                            self.draw_image(canvas, &value.primitive, transform, 1.0)?
                        }
                        BuiltDraw::Text(value) => {
                            self.draw_text(canvas, &value.primitive, transform, 1.0, text)?
                        }
                    }
                    canvas.restore_to_count(save);
                }
                BuiltItem::Layer(instance_id) => {
                    let instance = frame.layer_instance(*instance_id).ok_or_else(|| {
                        SkiaBackendError::InvalidFrame(format!(
                            "missing layer instance {}",
                            instance_id.0
                        ))
                    })?;
                    if instance
                        .render_program
                        .program()
                        .external_resource(ExternalResourceKind::Backdrop)
                        .is_some()
                    {
                        let prefix_id = instance.destination_prefix.ok_or_else(|| {
                            SkiaBackendError::InvalidFrame(format!(
                                "backdrop layer instance {} has no destination prefix",
                                instance_id.0
                            ))
                        })?;
                        let prefix = frame.composite_prefix(prefix_id).ok_or_else(|| {
                            SkiaBackendError::InvalidFrame(format!(
                                "backdrop layer instance {} references a missing destination prefix",
                                instance_id.0
                            ))
                        })?;
                        if prefix.local.layer != layer_id || prefix.local.item_count != item_index {
                            return Err(SkiaBackendError::InvalidFrame(format!(
                                "backdrop prefix for layer instance {} does not match the active surface prefix",
                                instance_id.0
                            )));
                        }
                    }
                    let child_layer = frame.layers.get(instance.layer.0).ok_or_else(|| {
                        SkiaBackendError::InvalidFrame(format!(
                            "missing child layer {}",
                            instance.layer.0
                        ))
                    })?;
                    // let child_bounds = non_empty_bounds(child_layer.render_bounds);
                    let child_bounds = child_layer.render_bounds;
                    let program_needs_backdrop = instance
                        .render_program
                        .program()
                        .external_resource(ExternalResourceKind::Backdrop)
                        .is_some();
                    let child_needs_backdrop = child_layer.backdrop_isolation
                        == BackdropIsolation::Passthrough
                        && backdrop_requirements.layer(instance.layer);
                    let backdrop = if program_needs_backdrop || child_needs_backdrop {
                        self.frame_stats.backdrop_materializations += 1;
                        let prefix = self.snapshot_target(target, target_bounds);
                        Some(match inherited_backdrop {
                            Some(inherited) => {
                                self.composite_images(inherited, &prefix, target_bounds)?
                            }
                            None => prefix,
                        })
                    } else {
                        self.frame_stats.backdrop_materializations_avoided += 1;
                        None
                    };
                    let child_backdrop = if child_needs_backdrop {
                        let backdrop = backdrop.as_ref().ok_or_else(|| {
                            SkiaBackendError::InvalidFrame(
                                "a passthrough layer lost its inherited backdrop".into(),
                            )
                        })?;
                        let traversed = self.execute_backdrop_only(
                            frame,
                            instance,
                            backdrop,
                            child_layer.content_bounds,
                        )?;
                        inverse_affine(instance.composite.transform)
                            .map(|inverse| self.transform_image(&traversed, inverse, child_bounds))
                            .transpose()?
                    } else {
                        None
                    };

                    let mut lease = self.layer_cache.acquire(
                        child_layer,
                        self.scale_factor,
                        self.gpu_context.as_mut(),
                    )?;
                    let child_damage = if lease.reused {
                        self.damage_tracker.layer(child_layer.source)
                    } else {
                        DamageRegion::full(child_bounds)
                    };
                    if !child_damage.is_empty() {
                        self.redraw_layer_region(
                            &mut lease.surface,
                            child_bounds,
                            frame,
                            instance.layer,
                            child_backdrop.as_ref(),
                            &child_damage,
                            Color::TRANSPARENT,
                            text,
                            backdrop_requirements,
                        )?;
                        if lease.cache_id.is_some() {
                            self.layer_cache.record_update(
                                lease.reused && child_damage.bounds() != Some(child_bounds),
                            );
                        }
                    }
                    let child_image = self.snapshot_target(&mut lease.surface, child_bounds);
                    self.layer_cache
                        .release(child_layer, self.scale_factor, lease);
                    self.execute_instance(
                        target,
                        target_bounds,
                        frame,
                        instance,
                        &child_image,
                        backdrop.as_ref().unwrap_or(&child_image),
                        LayerProgramEntry::Complete,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn execute_backdrop_only(
        &mut self,
        frame: &BuiltFrame,
        instance: &BuiltLayerInstance,
        backdrop: &RasterImage,
        layer_content_bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        if instance
            .render_program
            .program()
            .external_resource(ExternalResourceKind::Backdrop)
            .is_none()
        {
            return Ok(backdrop.clone());
        }
        let mut target = self.new_surface(backdrop.bounds)?;
        target.canvas().clear(skia_safe::Color::TRANSPARENT);
        draw_raster_image(
            target.canvas(),
            backdrop.bounds,
            self.scale_factor,
            backdrop,
            Affine::IDENTITY,
            &Paint::default(),
        );
        let dummy = self.transparent_image(layer_content_bounds)?;
        self.execute_instance(
            &mut target,
            backdrop.bounds,
            frame,
            instance,
            &dummy,
            backdrop,
            LayerProgramEntry::BackdropOnly,
        )?;
        Ok(self.snapshot_target(&mut target, backdrop.bounds))
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_instance(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        frame: &BuiltFrame,
        instance: &BuiltLayerInstance,
        layer_content: &RasterImage,
        backdrop: &RasterImage,
        entry: LayerProgramEntry,
    ) -> Result<(), SkiaBackendError> {
        let child = frame.layers.get(instance.layer.0).ok_or_else(|| {
            SkiaBackendError::InvalidFrame(format!("missing layer {}", instance.layer.0))
        })?;
        let backdrop_bounds = target_bounds & instance.world_bounds;

        let plan = instance.render_program.program().instantiate_entry(
            entry,
            &LayerPlanContext {
                backdrop_source_bounds: backdrop.bounds,
                parent_destination_bounds: target_bounds,
                composite_clip_bounds: backdrop_bounds,
                layer_content_bounds: child.content_bounds,
                backdrop_bounds,
                composite: instance.composite,
                scale_factor: self.scale_factor,
                color_texture_class: TextureClass::LINEAR_COLOR,
                external_aliasing: ExternalAliasing::Distinct,
                limits: PlanLimits::default(),
            },
        )?;
        self.execute_plan(
            target,
            target_bounds,
            frame,
            instance,
            &plan,
            layer_content,
            backdrop,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_plan(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        frame: &BuiltFrame,
        instance: &BuiltLayerInstance,
        plan: &LayerRenderPlan,
        layer_content: &RasterImage,
        backdrop: &RasterImage,
    ) -> Result<(), SkiaBackendError> {
        let plan_stats = plan.stats();
        self.frame_stats.render_plans += 1;
        self.frame_stats.render_passes += plan_stats.pass_count as u64;
        self.frame_stats.planned_transient_resources += plan_stats.transient_resource_count as u64;
        self.frame_stats.planned_transient_slots += plan_stats.transient_slot_count as u64;
        self.frame_stats.planned_transient_texels += plan_stats.allocated_texels;
        self.frame_stats.planned_peak_live_texels += plan_stats.peak_live_texels;
        self.frame_stats.transient_surface_allocations += plan.slots().len() as u64;
        self.frame_stats.transient_surface_reuses += plan_stats
            .transient_resource_count
            .saturating_sub(plan_stats.transient_slot_count)
            as u64;

        let mut transient_surfaces = Vec::with_capacity(plan.slots().len());
        for slot in plan.slots() {
            transient_surfaces.push(self.new_surface_px(slot.extent.width, slot.extent.height)?);
        }
        let mut values: Vec<Option<RasterImage>> = vec![None; plan.resources().len()];
        for (pass_index, pass) in plan.passes().iter().enumerate() {
            if pass.output == plan.parent_destination() {
                self.execute_composite_pass(
                    target,
                    target_bounds,
                    frame,
                    instance,
                    plan,
                    pass,
                    &values,
                    layer_content,
                    backdrop,
                )?;
            } else {
                let resource = &plan.resources()[pass.output.index()];
                let slot = resource.slot.ok_or_else(|| {
                    SkiaBackendError::InvalidFrame(format!(
                        "transient resource {} has no allocated slot",
                        pass.output.index()
                    ))
                })?;
                let output_bounds = plan_resource_bounds(plan, pass.output, self.scale_factor);
                let output = self.execute_filter_pass(
                    target,
                    target_bounds,
                    instance,
                    plan,
                    pass,
                    &values,
                    layer_content,
                    backdrop,
                    output_bounds,
                    &mut transient_surfaces[slot.index()],
                )?;
                values[pass.output.index()] = Some(output);
            }

            for (resource, value) in plan.resources().iter().zip(&mut values) {
                let final_use = resource.last_read.or(resource.producer);
                if final_use.is_some_and(|last| last.index() == pass_index) {
                    *value = None;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_filter_pass(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        instance: &BuiltLayerInstance,
        plan: &LayerRenderPlan,
        pass: &Pass,
        values: &[Option<RasterImage>],
        layer_content: &RasterImage,
        backdrop: &RasterImage,
        output_bounds: Bounds,
        output_surface: &mut Surface,
    ) -> Result<RasterImage, SkiaBackendError> {
        let input = |this: &mut Self, index: usize, target: &mut Surface| {
            let id = *pass
                .inputs
                .get(index)
                .ok_or(SkiaBackendError::MissingResource(index))?;
            this.resolve_resource(
                target,
                target_bounds,
                instance,
                plan,
                id,
                values,
                layer_content,
                backdrop,
            )
        };

        match &pass.op {
            PassOp::ShadowComposite { color, offset_px } => {
                let original = input(self, 0, target)?;
                let alpha = input(self, 1, target)?;
                self.render_shadow(
                    output_surface,
                    &original,
                    &alpha,
                    *color,
                    *offset_px,
                    output_bounds,
                )
            }
            PassOp::ApplyMask { transform, mask } => {
                let source = input(self, 0, target)?;
                let mask = self.resolve_resource(
                    target,
                    target_bounds,
                    instance,
                    plan,
                    *mask,
                    values,
                    layer_content,
                    backdrop,
                )?;
                self.apply_texture_mask(output_surface, &source, &mask, *transform, output_bounds)
            }
            op => {
                let source = input(self, 0, target)?;
                let filter = match op {
                    PassOp::Copy => None,
                    PassOp::GaussianBlur { axis, sigma_px, .. } => {
                        let sigma = *sigma_px / self.scale_factor;
                        let value = match axis {
                            xui_render_graph::Axis::X => (sigma, 0.0),
                            xui_render_graph::Axis::Y => (0.0, sigma),
                        };
                        skia_safe::image_filters::blur(value, TileMode::Decal, None, None)
                    }
                    PassOp::ColorMatrix(matrix) => color_matrix_filter(*matrix, None),
                    PassOp::Pixelate {
                        block_width_px,
                        block_height_px,
                    } => Some(self.runtime_filter(
                        "pixelate",
                        PIXELATE_SKSL,
                        &[(
                            "block",
                            &[
                                *block_width_px / self.scale_factor,
                                *block_height_px / self.scale_factor,
                            ],
                        )],
                    )?),
                    PassOp::Refraction {
                        strength_px,
                        chromatic_aberration_px,
                    } => Some(self.runtime_filter(
                        "refraction",
                        REFRACTION_SKSL,
                        &[
                            (
                                "center",
                                &[
                                    output_bounds.x() + output_bounds.width() * 0.5,
                                    output_bounds.y() + output_bounds.height() * 0.5,
                                ],
                            ),
                            (
                                "amount",
                                &[
                                    *strength_px / self.scale_factor,
                                    *chromatic_aberration_px / self.scale_factor,
                                ],
                            ),
                        ],
                    )?),
                    PassOp::ChromaticAberration { offset_px } => Some(self.runtime_filter(
                        "chromatic-aberration",
                        CHROMATIC_ABERRATION_SKSL,
                        &[(
                            "offset",
                            &[
                                offset_px[0] / self.scale_factor,
                                offset_px[1] / self.scale_factor,
                            ],
                        )],
                    )?),
                    PassOp::ExtractAlpha => Some(extract_alpha_filter()),
                    PassOp::AlphaSpread { axis, radius_px } => {
                        let radius = *radius_px / self.scale_factor;
                        let value = match axis {
                            xui_render_graph::Axis::X => (radius, 0.0),
                            xui_render_graph::Axis::Y => (0.0, radius),
                        };
                        skia_safe::image_filters::dilate(value, None, None)
                    }
                    PassOp::ShadowComposite { .. }
                    | PassOp::ApplyMask { .. }
                    | PassOp::BackdropComposite { .. }
                    | PassOp::LayerComposite { .. } => unreachable!(),
                };
                self.filter_image(output_surface, &source, output_bounds, filter)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_composite_pass(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        frame: &BuiltFrame,
        instance: &BuiltLayerInstance,
        plan: &LayerRenderPlan,
        pass: &Pass,
        values: &[Option<RasterImage>],
        layer_content: &RasterImage,
        backdrop: &RasterImage,
    ) -> Result<(), SkiaBackendError> {
        match &pass.op {
            PassOp::BackdropComposite {
                opacity,
                blend_mode,
                mask,
                bounds,
            } => {
                let source_id = *pass
                    .inputs
                    .first()
                    .ok_or(SkiaBackendError::MissingResource(0))?;
                let mut source = self.resolve_resource(
                    target,
                    target_bounds,
                    instance,
                    plan,
                    source_id,
                    values,
                    layer_content,
                    backdrop,
                )?;
                if !matches!(mask, PlanMask::None) {
                    let mask_image = self.render_plan_mask(
                        target,
                        target_bounds,
                        instance,
                        plan,
                        mask,
                        values,
                        layer_content,
                        backdrop,
                        *bounds,
                    )?;
                    source = self.apply_rendered_mask(&source, &mask_image, *bounds)?;
                }
                let canvas = target.canvas();
                let save = canvas.save();
                configure_canvas(canvas, target_bounds, self.scale_factor);
                canvas.clip_rect(sk_bounds(*bounds), ClipOp::Intersect, true);
                self.apply_clip_chain(canvas, frame, instance.clip_chain, Affine::IDENTITY)?;
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_alpha_f(opacity.clamp(0.0, 1.0));
                paint.set_blend_mode(sk_blend_mode(*blend_mode));
                draw_image_logical(canvas, &source, Affine::IDENTITY, &paint);
                canvas.restore_to_count(save);
                Ok(())
            }
            PassOp::LayerComposite {
                opacity,
                transform,
                blend_mode,
                operator,
                bounds,
            } => {
                let source_id = *pass
                    .inputs
                    .first()
                    .ok_or(SkiaBackendError::MissingResource(0))?;
                let source = self.resolve_resource(
                    target,
                    target_bounds,
                    instance,
                    plan,
                    source_id,
                    values,
                    layer_content,
                    backdrop,
                )?;
                let canvas = target.canvas();
                let save = canvas.save();
                configure_canvas(canvas, target_bounds, self.scale_factor);
                canvas.clip_rect(sk_bounds(*bounds), ClipOp::Intersect, true);
                self.apply_clip_chain(canvas, frame, instance.clip_chain, Affine::IDENTITY)?;
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_alpha_f(opacity.clamp(0.0, 1.0));
                if *blend_mode != BlendMode::Normal && *operator != CompositeOperator::SrcOver {
                    paint.set_blender(self.runtime_blender(*blend_mode, *operator)?);
                } else {
                    paint.set_blend_mode(composite_blend_mode(*blend_mode, *operator));
                }
                draw_image_logical(canvas, &source, *transform, &paint);
                canvas.restore_to_count(save);
                Ok(())
            }
            _ => Err(SkiaBackendError::InvalidFrame(
                "a non-composite pass targeted the parent destination".into(),
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_resource(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        instance: &BuiltLayerInstance,
        plan: &LayerRenderPlan,
        id: PlanResourceId,
        values: &[Option<RasterImage>],
        layer_content: &RasterImage,
        backdrop: &RasterImage,
    ) -> Result<RasterImage, SkiaBackendError> {
        if let Some(value) = values.get(id.index()).and_then(Clone::clone) {
            return Ok(value);
        }
        match plan.resources()[id.index()].kind {
            PlanResourceKind::Transient => Err(SkiaBackendError::MissingResource(id.index())),
            PlanResourceKind::External(kind) => match kind {
                ExternalResourceKind::Backdrop => Ok(backdrop.clone()),
                ExternalResourceKind::ParentDestination => {
                    Ok(self.snapshot_target(target, target_bounds))
                }
                ExternalResourceKind::LayerContent => Ok(layer_content.clone()),
                ExternalResourceKind::BackdropMask | ExternalResourceKind::LayerMask(_) => {
                    self.resolve_mask_image(instance, kind)
                }
            },
        }
    }

    fn resolve_mask_image(
        &mut self,
        instance: &BuiltLayerInstance,
        kind: ExternalResourceKind,
    ) -> Result<RasterImage, SkiaBackendError> {
        let handle = instance.render_program.handle(kind).ok_or_else(|| {
            SkiaBackendError::InvalidFrame(format!("missing render-program binding for {kind:?}"))
        })?;
        let image = match handle {
            ImageResource::Data { key, data } => {
                let stale = self
                    .source_images
                    .get(key)
                    .is_none_or(|cached| cached.data_id != data.id().raw());
                if stale {
                    self.source_images.insert(
                        key.clone(),
                        CachedSourceImage {
                            data_id: data.id().raw(),
                            image: make_image(data, ImageTransform::default())?,
                            bytes: image_bytes(data),
                        },
                    );
                }
                self.source_images
                    .get(key)
                    .expect("source image was just inserted")
                    .image
            }
            ImageResource::Key(key) => self
                .source_images
                .get(key)
                .map(|cached| cached.image)
                .ok_or_else(|| SkiaBackendError::MissingMaskImage(key.clone()))?,
        };
        Ok(RasterImage {
            image,
            bounds: Bounds::from_zero_size((1.0, 1.)),
        })
    }

    fn snapshot_target(&mut self, target: &mut Surface, bounds: Bounds) -> RasterImage {
        self.frame_stats.image_snapshots += 1;
        RasterImage {
            image: target.image_snapshot(),
            bounds,
        }
    }

    fn snapshot_surface_output(
        &mut self,
        surface: &mut Surface,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        let (width, height) = physical_extent(bounds, self.scale_factor);
        let image = surface
            .image_snapshot_with_bounds(IRect::new(0, 0, width as i32, height as i32))
            .ok_or(SkiaBackendError::SurfaceAllocation { width, height })?;
        self.frame_stats.image_snapshots += 1;
        Ok(RasterImage { image, bounds })
    }

    fn composite_images(
        &mut self,
        back: &RasterImage,
        front: &RasterImage,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        let mut surface = self.new_surface(bounds)?;
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        for image in [back, front] {
            draw_raster_image(
                surface.canvas(),
                bounds,
                self.scale_factor,
                image,
                Affine::IDENTITY,
                &Paint::default(),
            );
        }
        Ok(self.snapshot_target(&mut surface, bounds))
    }

    fn transform_image(
        &mut self,
        source: &RasterImage,
        transform: Affine,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        let mut surface = self.new_surface(bounds)?;
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            source,
            transform,
            &Paint::default(),
        );
        Ok(self.snapshot_target(&mut surface, bounds))
    }

    fn filter_image(
        &mut self,
        surface: &mut Surface,
        source: &RasterImage,
        bounds: Bounds,
        filter: Option<ImageFilter>,
    ) -> Result<RasterImage, SkiaBackendError> {
        clear_surface_output(surface, bounds, self.scale_factor);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_image_filter(filter);
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            source,
            Affine::IDENTITY,
            &paint,
        );
        self.snapshot_surface_output(surface, bounds)
    }

    fn runtime_filter(
        &mut self,
        name: &'static str,
        source: &'static str,
        uniforms: &[(&str, &[f32])],
    ) -> Result<ImageFilter, SkiaBackendError> {
        let effect = self.runtime_effect(name, source, false)?;
        let mut builder = RuntimeShaderBuilder::new(effect);
        for (uniform, value) in uniforms {
            builder.set_uniform_float(uniform, value).map_err(|error| {
                SkiaBackendError::RuntimeUniform {
                    effect: name,
                    message: error.to_string(),
                }
            })?;
        }
        skia_safe::image_filters::runtime_shader(&builder, "source", None)
            .ok_or(SkiaBackendError::RuntimeShader(name))
    }

    fn runtime_effect(
        &mut self,
        name: &'static str,
        source: &'static str,
        blender: bool,
    ) -> Result<RuntimeEffect, SkiaBackendError> {
        if let Some(effect) = self.runtime_effects.get(name) {
            return Ok(effect.clone());
        }
        let effect = if blender {
            RuntimeEffect::make_for_blender(source, None)
        } else {
            RuntimeEffect::make_for_shader(source, None)
        }
        .map_err(|message| SkiaBackendError::RuntimeEffect {
            effect: name,
            message,
        })?;
        self.runtime_effects.insert(name, effect.clone());
        Ok(effect)
    }

    fn runtime_blender(
        &mut self,
        blend: BlendMode,
        operator: CompositeOperator,
    ) -> Result<skia_safe::Blender, SkiaBackendError> {
        let effect = self.runtime_effect("composite", COMPOSITE_SKSL, true)?;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&(blend_index(blend) as i32).to_ne_bytes());
        bytes.extend_from_slice(&(operator_index(operator) as i32).to_ne_bytes());
        effect
            .make_blender(Data::new_copy(&bytes), None)
            .ok_or(SkiaBackendError::RuntimeShader("composite"))
    }

    fn render_shadow(
        &mut self,
        surface: &mut Surface,
        original: &RasterImage,
        alpha: &RasterImage,
        color: Color,
        offset_px: [f32; 2],
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        clear_surface_output(surface, bounds, self.scale_factor);
        let mut shadow_paint = Paint::default();
        shadow_paint.set_color_filter(shadow_color_filter(color));
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            alpha,
            Affine::translate(
                offset_px[0] / self.scale_factor,
                offset_px[1] / self.scale_factor,
            ),
            &shadow_paint,
        );
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            original,
            Affine::IDENTITY,
            &Paint::default(),
        );
        self.snapshot_surface_output(surface, bounds)
    }

    fn apply_texture_mask(
        &mut self,
        surface: &mut Surface,
        source: &RasterImage,
        mask: &RasterImage,
        transform: Affine,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        clear_surface_output(surface, bounds, self.scale_factor);
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            source,
            Affine::IDENTITY,
            &Paint::default(),
        );
        let mut paint = Paint::default();
        paint.set_blend_mode(SkBlendMode::DstIn);
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            mask,
            transform,
            &paint,
        );
        self.snapshot_surface_output(surface, bounds)
    }

    fn apply_rendered_mask(
        &mut self,
        source: &RasterImage,
        mask: &RasterImage,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        let mut surface = self.new_surface(bounds)?;
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            source,
            Affine::IDENTITY,
            &Paint::default(),
        );
        let mut paint = Paint::default();
        paint.set_blend_mode(SkBlendMode::DstIn);
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            mask,
            Affine::IDENTITY,
            &paint,
        );
        Ok(self.snapshot_target(&mut surface, bounds))
    }

    fn render_transformed_mask(
        &mut self,
        mask: &RasterImage,
        transform: Affine,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        let mut surface = self.new_surface(bounds)?;
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        draw_raster_image(
            surface.canvas(),
            bounds,
            self.scale_factor,
            mask,
            transform,
            &Paint::default(),
        );
        Ok(self.snapshot_target(&mut surface, bounds))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_plan_mask(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        instance: &BuiltLayerInstance,
        plan: &LayerRenderPlan,
        mask: &PlanMask,
        values: &[Option<RasterImage>],
        layer_content: &RasterImage,
        backdrop: &RasterImage,
        bounds: Bounds,
    ) -> Result<RasterImage, SkiaBackendError> {
        match mask {
            PlanMask::None => self.transparent_image(bounds),
            PlanMask::Texture {
                transform,
                resource,
            } => {
                let image = self.resolve_resource(
                    target,
                    target_bounds,
                    instance,
                    plan,
                    *resource,
                    values,
                    layer_content,
                    backdrop,
                )?;
                self.render_transformed_mask(&image, *transform, bounds)
            }
            PlanMask::Shape { shape, transform } => {
                let mut surface = self.new_surface(bounds)?;
                surface.canvas().clear(skia_safe::Color::TRANSPARENT);
                let canvas = surface.canvas();
                let save = canvas.save();
                configure_canvas(canvas, bounds, self.scale_factor);
                canvas.concat(&sk_matrix(*transform));
                let paint = solid_paint(Color::WHITE);
                let shape = match *shape {
                    MaskShape::RoundedRect(radius) => {
                        let x_scale = transform.xx.hypot(transform.yx);
                        let y_scale = transform.xy.hypot(transform.yy);
                        MaskShape::RoundedRect(radius / x_scale.min(y_scale).max(f32::EPSILON))
                    }
                    value => value,
                };
                draw_mask_shape(canvas, shape, &paint);
                canvas.restore_to_count(save);
                Ok(self.snapshot_target(&mut surface, bounds))
            }
        }
    }

    fn apply_clip_chain(
        &self,
        canvas: &Canvas,
        frame: &BuiltFrame,
        clip: Option<BuiltClipChainId>,
        placement: Affine,
    ) -> Result<(), SkiaBackendError> {
        let mut chain = Vec::new();
        let mut current = clip;
        while let Some(id) = current {
            let value = frame.clip_chains.get(id.0).ok_or_else(|| {
                SkiaBackendError::InvalidFrame(format!("missing clip chain {}", id.0))
            })?;
            chain.push(value);
            current = value.parent;
        }
        for value in chain.into_iter().rev() {
            let matrix = sk_matrix(value.world_transform.then(placement));
            let mut builder = PathBuilder::new();
            match &value.clip {
                ClipShape::Rect(rect) => {
                    builder.add_rect(sk_bounds(*rect), None, None);
                }
                ClipShape::RoundedRect { rect, radius } => {
                    let rr = RRect::new_rect_xy(sk_bounds(*rect), *radius, *radius);
                    builder.add_rrect(rr, None, None);
                }
                ClipShape::Path { path, .. } => {
                    append_path(&mut builder, path);
                }
            }
            builder.transform(&matrix);
            canvas.clip_path(&builder.detach(), ClipOp::Intersect, true);
        }
        Ok(())
    }

    fn compiled_vector_scene(&mut self, scene: &VectorScene) -> Arc<[CompiledVectorCommand]> {
        if let Some(compiled) = self.vector_scenes.get(&scene.id()) {
            return compiled;
        }
        let compiled: Arc<[CompiledVectorCommand]> = scene
            .commands()
            .iter()
            .filter_map(|command| match command {
                VectorCommand::FillPath {
                    path,
                    transform,
                    fill,
                } => Some(CompiledVectorCommand::FillPath {
                    path: self.compiled_vector_path(path),
                    transform: *transform,
                    fill: *fill,
                }),
                VectorCommand::StrokePath {
                    path,
                    transform,
                    stroke,
                } => Some(CompiledVectorCommand::StrokePath {
                    path: self.compiled_vector_path(path),
                    transform: *transform,
                    stroke: *stroke,
                }),
                // See the vello backend: a vector scene only ever holds paths.
                VectorCommand::Shape { .. } | VectorCommand::TextBox { .. } => None,
            })
            .collect::<Vec<_>>()
            .into();
        self.vector_scenes.insert(scene.id(), Arc::clone(&compiled));
        compiled
    }

    fn compiled_vector_path(&mut self, path: &PathData) -> Path {
        if let Some(compiled) = self.vector_paths.get(&path.id()) {
            return compiled;
        }
        let compiled = sk_path(path);
        self.vector_paths.insert(path.id(), compiled.clone());
        compiled
    }

    fn draw_image(
        &mut self,
        canvas: &Canvas,
        primitive: &xui::render::ImagePrimitive,
        transform: Affine,
        opacity: f32,
    ) -> Result<(), SkiaBackendError> {
        if primitive.opacity <= 0.0
            || primitive.data.size.width == 0
            || primitive.data.size.height == 0
        {
            return Ok(());
        }
        let source_stale = self
            .source_images
            .get(&primitive.image)
            .is_none_or(|cached| cached.data_id != primitive.data.id().raw());
        if source_stale {
            self.source_images.insert(
                primitive.image.clone(),
                CachedSourceImage {
                    data_id: primitive.data.id().raw(),
                    image: make_image(&primitive.data, ImageTransform::default())?,
                    bytes: image_bytes(&primitive.data),
                },
            );
        }
        let source = self
            .source_images
            .get(&primitive.image)
            .expect("source image was just prepared")
            .image;
        let key = CachedImageKey {
            data: primitive.data.id().raw(),
            transform: primitive.variant.transform,
            bytes: image_bytes(&primitive.data),
        };
        let image = if primitive.variant.transform == ImageTransform::default() {
            source
        } else if let Some(image) = self.image_cache.get(&key) {
            image
        } else {
            let image = make_image(&primitive.data, primitive.variant.transform)?;
            self.image_cache.insert(key, image.clone());
            image
        };
        let oriented_size = Size::new(image.width() as u32, image.height() as u32);
        let Some(tile) = fitted_image_rect(primitive.bounds, oriented_size, primitive.style) else {
            return Ok(());
        };
        let save = canvas.save();
        canvas.concat(&sk_matrix(transform));
        canvas.clip_rect(sk_bounds(primitive.bounds), ClipOp::Intersect, true);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_alpha_f((primitive.opacity * opacity).clamp(0.0, 1.0));
        let sampling = sampling_options(primitive.style.sampling);
        for rect in image_tiles(primitive.bounds, tile, primitive.style.repeat) {
            canvas.draw_image_rect_with_sampling_options(
                &image,
                None,
                sk_bounds(rect),
                sampling,
                &paint,
            );
        }
        canvas.restore_to_count(save);
        Ok(())
    }

    fn draw_text(
        &mut self,
        canvas: &Canvas,
        primitive: &xui::render::TextPrimitive,
        transform: Affine,
        opacity: f32,
        text: &mut TextHost<T>,
    ) -> Result<(), SkiaBackendError> {
        let Some(handle) = text.active_slot(primitive.node_id, primitive.slot) else {
            return Err(SkiaBackendError::InvalidFrame(
                "text primitive has no active layout".into(),
            ));
        };
        let Some(layout) = text.layout(handle) else {
            return Err(SkiaBackendError::InvalidFrame(
                "text primitive layout is not resident".into(),
            ));
        };
        let save = canvas.save();
        canvas.concat(&sk_matrix(transform));
        canvas.clip_rect(sk_bounds(primitive.bounds), ClipOp::Intersect, true);
        let y_offset = match primitive.vertical_align {
            TextVerticalAlign::Top | TextVerticalAlign::Baseline => 0.0,
            TextVerticalAlign::Middle => {
                ((primitive.bounds.height() - layout.size().height) * 0.5).max(0.0)
            }
            TextVerticalAlign::Bottom => {
                (primitive.bounds.height() - layout.size().height).max(0.0)
            }
        };
        let origin =
            xui_interface::Point::new(primitive.bounds.x(), primitive.bounds.y() + y_offset);

        if let Some(selection) = primitive.paint.selection
            && let Some(query) = text.query(handle)
        {
            let mut paint = solid_paint(alpha_color(selection.color, opacity));
            paint.set_anti_alias(false);
            for rect in query.selection_rects(selection.range) {
                canvas.draw_rect(sk_rect(rect.translate(origin)), &paint);
            }
        }

        let color = alpha_color(primitive.paint.style.color, opacity);
        if let Some(blob) =
            self.text_blob_for_layout(text.backend(), handle, primitive.node_id, &layout)?
        {
            canvas.draw_text_blob(blob, (origin.x, origin.y), &solid_paint(color));
        }

        if let Some(caret) = primitive.paint.caret {
            let rect = text
                .query(handle)
                .and_then(|query| query.caret_rect(caret.char_index))
                .unwrap_or(Rect::new(
                    layout.size().width,
                    0.0,
                    caret.width,
                    primitive.paint.style.font_size * 1.2,
                ));
            let mut paint = solid_paint(alpha_color(caret.color, opacity));
            paint.set_stroke_width(caret.width.max(1.0));
            canvas.draw_line(
                (origin.x + rect.x, origin.y + rect.y),
                (origin.x + rect.x, origin.y + rect.y + rect.height),
                &paint,
            );
        }
        if let Some(ime) = primitive.paint.ime
            && let Some(query) = text.query(handle)
        {
            let mut paint = solid_paint(alpha_color(ime.underline_color, opacity));
            paint.set_stroke_width(ime.underline_width.max(1.0));
            for rect in query.selection_rects(ime.range) {
                let y = origin.y + rect.y + rect.height;
                canvas.draw_line(
                    (origin.x + rect.x, y),
                    (origin.x + rect.x + rect.width, y),
                    &paint,
                );
            }
        }
        draw_text_decorations(canvas, &layout.lines, primitive, origin, opacity);
        canvas.restore_to_count(save);
        Ok(())
    }

    fn load_font_from_path(
        &mut self,
        path: &std::path::Path,
        index: u32,
    ) -> std::io::Result<Option<SkTypeface>> {
        let file = File::open(path)?;
        let f = unsafe { memmap2::Mmap::map(&file) }?;
        Ok(self.load_font_from_bytes(&f, index))
    }

    fn load_font_from_bytes(&mut self, bytes: &[u8], index: u32) -> Option<SkTypeface> {
        self.font_mgr().new_from_data(bytes, Some(index as usize))
    }

    fn system_typeface(
        &mut self,
        family: &str,
        postscript_name: &str,
        style: SkFontStyle,
    ) -> Option<SkTypeface> {
        let mut styles = self.font_mgr().match_family(family);
        for index in 0..styles.count() {
            let Some(typeface) = styles.new_typeface(index) else {
                continue;
            };
            if typeface
                .post_script_name()
                .is_some_and(|name| name == postscript_name)
            {
                return Some(typeface);
            }
        }
        self.font_mgr().match_family_style(family, style)
    }

    /// The process-wide font manager, built on first use.
    ///
    /// `FontMgr::new()` enumerates CoreText on macOS and costs tens of
    /// milliseconds; building one per typeface cache miss put 300 ms into the
    /// first frame of a text-heavy window.
    fn font_mgr(&mut self) -> &FontMgr {
        self.font_mgr.get_or_insert_with(FontMgr::new)
    }

    fn typeface_for_font(
        &mut self,
        backend: &T,
        font_id: <T as FontDatabase>::FontId,
        font_weight: FontWeight,
    ) -> Result<SkTypeface, SkiaBackendError> {
        let epoch = backend.epoch();
        if self.font_cache_epoch != Some(epoch) {
            self.font_cache.clear();
            self.font_cache_epoch = Some(epoch);
        }
        let cache_key = (font_id, font_weight);
        if let Some(typeface) = self.font_cache.get(&cache_key) {
            return Ok(typeface.clone());
        }

        let font_data = backend.font_data(font_id).ok_or_else(|| {
            SkiaBackendError::FontDataError("the shaper did not expose data for a run font".into())
        })?;
        let typeface = match font_data {
            FontDataRef::Bytes { bytes, index } => self
                .load_font_from_bytes(bytes, index)
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not load font bytes at collection index {index}"
                    ))
                })?,
            FontDataRef::SystemMemory {
                bytes,
                index,
                family,
                postscript_name,
                style,
                stretch,
                ..
            } => self
                .system_typeface(
                    family,
                    postscript_name,
                    sk_font_style(font_weight, stretch, style),
                )
                .or_else(|| self.load_font_from_bytes(bytes, index))
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not resolve system font {family} ({postscript_name}) from in-memory collection index {index}"
                    ))
                })?,
            FontDataRef::System {
                path,
                index,
                family,
                postscript_name,
                style,
                stretch,
                ..
            } => self
                .system_typeface(
                    family,
                    postscript_name,
                    sk_font_style(font_weight, stretch, style),
                )
                .or_else(|| self.load_font_from_path(path, index).ok().flatten())
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not resolve system font {family} ({postscript_name}) from {} at collection index {index}",
                        path.display()
                    ))
                })?,
        };
        self.font_cache.insert(cache_key, typeface.clone());
        Ok(typeface)
    }

    fn build_text_blob(
        &mut self,
        backend: &T,
        layout: &ParagraphLayout<<T as Shaper>::FontId, <T as Shaper>::GlyphKey>,
    ) -> Result<Option<TextBlob>, SkiaBackendError> {
        let mut builder = TextBlobBuilder::new();
        let mut has_glyphs = false;
        for run in &layout.runs {
            let glyphs = layout.glyphs.get(run.glyph_range.clone()).ok_or_else(|| {
                SkiaBackendError::InvalidFrame(format!(
                    "text run glyph range {:?} exceeds {} glyphs",
                    run.glyph_range,
                    layout.glyphs.len()
                ))
            })?;
            if glyphs.is_empty() {
                continue;
            }

            let typeface = self.typeface_for_font(backend, run.font_id, run.font_weight)?;
            let mut font = Font::from_typeface(typeface, Some(run.font_size.max(1.0)));
            font.set_subpixel(true);
            font.set_edging(skia_safe::font::Edging::SubpixelAntiAlias);
            let (glyph_ids, positions) = builder.alloc_run_pos(&font, glyphs.len(), None);
            for ((glyph_id, position), glyph) in
                glyph_ids.iter_mut().zip(positions.iter_mut()).zip(glyphs)
            {
                *glyph_id = SkGlyphId::try_from(glyph.glyph_id).map_err(|_| {
                    SkiaBackendError::InvalidFrame(format!(
                        "glyph id {} cannot be represented by Skia",
                        glyph.glyph_id
                    ))
                })?;
                *position = SkPoint::new(glyph.draw_pos.x, glyph.draw_pos.y);
            }
            has_glyphs = true;
        }
        Ok(has_glyphs.then(|| builder.make()).flatten())
    }

    fn text_blob_for_layout(
        &mut self,
        backend: &T,
        handle: TextLayoutHandle,
        owner: NodeId,
        layout: &ParagraphLayout<<T as Shaper>::FontId, <T as Shaper>::GlyphKey>,
    ) -> Result<Option<TextBlob>, SkiaBackendError> {
        let font_epoch = backend.epoch();
        if let Some(cached) = self.text_blob_cache.get_mut(&handle)
            && cached.font_epoch == font_epoch
        {
            cached.last_used_frame = self.frame_index;
            return Ok(cached.blob.clone());
        }
        let blob = self.build_text_blob(backend, layout)?;
        self.text_blob_cache.insert(
            handle,
            CachedTextBlob {
                blob: blob.clone(),
                font_epoch,
                owner,
                last_used_frame: self.frame_index,
            },
        );
        Ok(blob)
    }

    #[cfg(test)]
    fn draw_glyphs(
        &mut self,
        backend: &T,
        canvas: &Canvas,
        layout: &ParagraphLayout<<T as Shaper>::FontId, <T as Shaper>::GlyphKey>,
        origin: xui_interface::Point,
        color: Color,
    ) -> Result<(), SkiaBackendError> {
        if let Some(blob) = self.build_text_blob(backend, layout)? {
            canvas.draw_text_blob(blob, (origin.x, origin.y), &solid_paint(color));
        }
        Ok(())
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
        #[cfg(target_os = "macos")]
        if let (Some(presenter), Some(context)) =
            (self.presenter.as_mut(), self.gpu_context.as_mut())
        {
            self.raster = Some(presenter.acquire_surface(
                context,
                self.frame_size_px.width,
                self.frame_size_px.height,
            )?);
        }
        self.submitted = false;
        self.presented = false;
        self.rollback_damage_tracker = None;
        Ok(())
    }

    fn submit(&mut self, frame: &BuiltFrame, text: &mut TextHost<T>) -> Result<(), Self::Error> {
        let mut next_damage_tracker = self.damage_tracker.clone();
        let mut root_damage = next_damage_tracker.update(frame);
        if self.gpu_context.is_some() {
            root_damage = DamageRegion::full(Bounds::from_zero_size(
                self.frame_size_px().to_f32().unwrap() / self.scale_factor,
            ));
        }
        if std::env::var("XUI_DEBUG_FRAME").is_ok() {
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
            .begin_frame(next_damage_tracker.dirty_region_count());
        let mut surface = self.raster.take().ok_or_else(|| {
            SkiaBackendError::InvalidFrame("begin_frame must be called before submit".into())
        })?;
        let previous_damage_tracker =
            std::mem::replace(&mut self.damage_tracker, next_damage_tracker);
        let result = self.draw_frame(&mut surface, frame, &root_damage, text);
        self.raster = Some(surface);
        if let Err(error) = result {
            self.damage_tracker = previous_damage_tracker;
            self.layer_cache.clear();
            return Err(error);
        }
        self.layer_cache.finish_frame(
            &frame.live_layer_caches,
            self.options.layer_cache_budget_bytes,
        );
        self.rollback_damage_tracker = Some(previous_damage_tracker);
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
        #[cfg(target_os = "macos")]
        if let (Some(presenter), Some(context)) =
            (self.presenter.as_mut(), self.gpu_context.as_mut())
        {
            let mut surface = self.raster.take().ok_or_else(|| {
                SkiaBackendError::InvalidFrame("Metal frame surface is unavailable".into())
            })?;
            context.flush_and_submit_surface(&mut surface, None);
            drop(surface);
            presenter.present()?;
            self.rollback_damage_tracker = None;
            self.pending_damage = DamageRegion::default();
            self.presented = true;
            return Ok(());
        }

        #[cfg(not(target_os = "macos"))]
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
                    let presenter = self.presenter.as_mut().expect("checked above");
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
                if let Some(previous) = self.rollback_damage_tracker.take() {
                    self.damage_tracker = previous;
                }
                self.layer_cache.clear();
                return Err(error);
            }
        }
        self.rollback_damage_tracker = None;
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
        self.frame_size_px = Size::new(0, 0);
        self.damage_tracker.clear();
        self.rollback_damage_tracker = None;
        self.layer_cache.clear();
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

const PIXELATE_SKSL: &str = r#"
uniform shader source;
uniform float2 block;
half4 main(float2 p) {
    float2 size = max(block, float2(1.0));
    float2 snapped = (floor(p / size) + float2(0.5)) * size;
    return source.eval(snapped);
}
"#;

const REFRACTION_SKSL: &str = r#"
uniform shader source;
uniform float2 center;
uniform float2 amount;
half4 main(float2 p) {
    float2 delta = p - center;
    float distance = max(length(delta), 0.0001);
    float2 direction = delta / distance;
    float2 displacement = direction * amount.x * exp(-distance * 0.02);
    float2 chroma = direction * amount.y;
    half4 middle = source.eval(p + displacement);
    return half4(source.eval(p + displacement + chroma).r,
                 middle.g,
                 source.eval(p + displacement - chroma).b,
                 middle.a);
}
"#;

const CHROMATIC_ABERRATION_SKSL: &str = r#"
uniform shader source;
uniform float2 offset;
half4 main(float2 p) {
    half4 middle = source.eval(p);
    return half4(source.eval(p + offset).r,
                 middle.g,
                 source.eval(p - offset).b,
                 middle.a);
}
"#;

const COMPOSITE_SKSL: &str = r#"
uniform int blend_mode;
uniform int composite_op;

float lum(float3 c) { return dot(c, float3(0.3, 0.59, 0.11)); }
float sat(float3 c) { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
float3 clip_color(float3 c) {
    float l = lum(c), n = min(c.r, min(c.g, c.b)), x = max(c.r, max(c.g, c.b));
    if (n < 0.0) c = float3(l) + (c - float3(l)) * l / (l - n);
    if (x > 1.0) c = float3(l) + (c - float3(l)) * (1.0 - l) / (x - l);
    return c;
}
float3 set_lum(float3 c, float l) { return clip_color(c + float3(l - lum(c))); }
float3 set_sat(float3 c, float s) {
    float lo = min(c.r, min(c.g, c.b)), hi = max(c.r, max(c.g, c.b));
    return hi <= lo ? float3(0.0) : (c - float3(lo)) * s / (hi - lo);
}
float soft_light(float b, float s) {
    if (s <= 0.5) return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    float d = b <= 0.25 ? ((16.0 * b - 12.0) * b + 4.0) * b : sqrt(b);
    return b + (2.0 * s - 1.0) * (d - b);
}
float3 blend(float3 b, float3 s) {
    if (blend_mode == 0) return s;
    if (blend_mode == 1) return b * s;
    if (blend_mode == 2) return b + s - b * s;
    if (blend_mode == 3) return mix(2.0*b*s, 1.0-2.0*(1.0-b)*(1.0-s), step(float3(0.5), b));
    if (blend_mode == 4) return min(b, s);
    if (blend_mode == 5) return max(b, s);
    if (blend_mode == 6) return min(float3(1.0), b / max(float3(0.00001), 1.0-s));
    if (blend_mode == 7) return 1.0-min(float3(1.0), (1.0-b)/max(s,float3(0.00001)));
    if (blend_mode == 8) return mix(2.0*b*s, 1.0-2.0*(1.0-b)*(1.0-s), step(float3(0.5), s));
    if (blend_mode == 9) return float3(soft_light(b.r,s.r),soft_light(b.g,s.g),soft_light(b.b,s.b));
    if (blend_mode == 10) return abs(b-s);
    if (blend_mode == 11) return b+s-2.0*b*s;
    if (blend_mode == 12) return set_lum(set_sat(s,sat(b)),lum(b));
    if (blend_mode == 13) return set_lum(set_sat(b,sat(s)),lum(b));
    if (blend_mode == 14) return set_lum(b,lum(s));
    return set_lum(s,lum(b));
}
half4 main(half4 source, half4 destination) {
    float sa = clamp(float(source.a),0.0,1.0), da = clamp(float(destination.a),0.0,1.0);
    float3 sc = sa > 0.000001 ? float3(source.rgb)/sa : float3(0.0);
    float3 dc = da > 0.000001 ? float3(destination.rgb)/da : float3(0.0);
    float3 mixed = (1.0-da)*sc + da*blend(dc,sc);
    float fa = 1.0, fb = 1.0-sa;
    if (composite_op == 1) fb = 0.0;
    if (composite_op == 2) { fa = 1.0-da; fb = 1.0; }
    float alpha = sa*fa + da*fb;
    return half4(sa*fa*mixed + da*fb*dc, alpha);
}
"#;

fn new_surface_px(
    width: u32,
    height: u32,
    gpu_context: Option<&mut skia_safe::gpu::DirectContext>,
) -> Result<Surface, SkiaBackendError> {
    if let Some(context) = gpu_context {
        let info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::BGRA8888,
            AlphaType::Premul,
            ColorSpace::new_srgb(),
        );
        return skia_safe::gpu::surfaces::render_target(
            context,
            skia_safe::gpu::Budgeted::Yes,
            &info,
            None,
            skia_safe::gpu::SurfaceOrigin::TopLeft,
            None,
            false,
            false,
        )
        .ok_or(SkiaBackendError::SurfaceAllocation { width, height });
    }
    skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
        .ok_or(SkiaBackendError::SurfaceAllocation { width, height })
}

fn physical_extent(bounds: Bounds, scale: f32) -> (u32, u32) {
    (
        (bounds.width().max(0.0) * scale).ceil().max(1.0) as u32,
        (bounds.height().max(0.0) * scale).ceil().max(1.0) as u32,
    )
}

fn clear_surface_output(surface: &mut Surface, bounds: Bounds, scale: f32) {
    let (width, height) = physical_extent(bounds, scale);
    let canvas = surface.canvas();
    let save = canvas.save();
    canvas.reset_matrix();
    canvas.clip_irect(
        IRect::new(0, 0, width as i32, height as i32),
        ClipOp::Intersect,
    );
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.restore_to_count(save);
}

fn damage_region(
    surface: &Surface,
    target_bounds: Bounds,
    scale: f32,
    damage: &DamageRegion,
) -> Region {
    let width = surface.width();
    let height = surface.height();
    let rects: Vec<_> = damage
        .rects()
        .iter()
        .filter_map(|rect| {
            let left = ((rect.x() - target_bounds.x()) * scale).floor() as i32;
            let top = ((rect.y() - target_bounds.y()) * scale).floor() as i32;
            let right = ((rect.x() + rect.width() - target_bounds.x()) * scale).ceil() as i32;
            let bottom = ((rect.y() + rect.height() - target_bounds.y()) * scale).ceil() as i32;
            let clipped = IRect::new(
                left.clamp(0, width),
                top.clamp(0, height),
                right.clamp(0, width),
                bottom.clamp(0, height),
            );
            (!clipped.is_empty()).then_some(clipped)
        })
        .collect();
    let mut region = Region::new();
    region.set_rects(&rects);
    region
}

#[cfg(not(target_os = "macos"))]
fn physical_damage_rects(
    damage: &DamageRegion,
    scale: f32,
    size: Size<u32>,
) -> Vec<softbuffer::Rect> {
    damage
        .rects()
        .iter()
        .filter_map(|rect| {
            let left = (rect.x * scale).floor().max(0.0) as u32;
            let top = (rect.y * scale).floor().max(0.0) as u32;
            let right = ((rect.x + rect.width) * scale).ceil().max(0.0) as u32;
            let bottom = ((rect.y + rect.height) * scale).ceil().max(0.0) as u32;
            let left = left.min(size.width);
            let top = top.min(size.height);
            let right = right.min(size.width);
            let bottom = bottom.min(size.height);
            Some(softbuffer::Rect {
                x: left,
                y: top,
                width: NonZeroU32::new(right.checked_sub(left)?)?,
                height: NonZeroU32::new(bottom.checked_sub(top)?)?,
            })
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn full_softbuffer_rect(size: Size<u32>) -> Result<softbuffer::Rect, SkiaBackendError> {
    Ok(softbuffer::Rect {
        x: 0,
        y: 0,
        width: NonZeroU32::new(size.width)
            .ok_or_else(|| SkiaBackendError::InvalidFrame("frame width is zero".into()))?,
        height: NonZeroU32::new(size.height)
            .ok_or_else(|| SkiaBackendError::InvalidFrame("frame height is zero".into()))?,
    })
}

#[cfg(not(target_os = "macos"))]
fn copy_surface_damage(
    surface: &mut Surface,
    destination: &mut [u32],
    frame_width: u32,
    damage: &[softbuffer::Rect],
) -> Result<(), SkiaBackendError> {
    for rect in damage {
        let width = rect.width.get();
        let height = rect.height.get();
        let mut rgba = vec![0; width as usize * height as usize * 4];
        let info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        if !surface.read_pixels(
            &info,
            &mut rgba,
            width as usize * 4,
            (rect.x as i32, rect.y as i32),
        ) {
            return Err(SkiaBackendError::PixelRead);
        }
        for row in 0..height as usize {
            let dst_start = (rect.y as usize + row) * frame_width as usize + rect.x as usize;
            let src_start = row * width as usize * 4;
            for column in 0..width as usize {
                let src = src_start + column * 4;
                destination[dst_start + column] = (u32::from(rgba[src]) << 16)
                    | (u32::from(rgba[src + 1]) << 8)
                    | u32::from(rgba[src + 2]);
            }
        }
    }
    Ok(())
}

fn non_empty_bounds(bounds: Rect) -> Rect {
    Rect::new(
        bounds.x,
        bounds.y,
        bounds.width.max(f32::EPSILON),
        bounds.height.max(f32::EPSILON),
    )
}

fn configure_canvas(canvas: &Canvas, bounds: Bounds, scale: f32) {
    canvas.scale((scale, scale));
    canvas.translate((-bounds.x(), -bounds.y()));
}

fn draw_raster_image(
    canvas: &Canvas,
    target_bounds: Bounds,
    scale: f32,
    source: &RasterImage,
    transform: Affine,
    paint: &Paint,
) {
    let save = canvas.save();
    configure_canvas(canvas, target_bounds, scale);
    draw_image_logical(canvas, source, transform, paint);
    canvas.restore_to_count(save);
}

fn draw_image_logical(canvas: &Canvas, source: &RasterImage, transform: Affine, paint: &Paint) {
    let save = canvas.save();
    canvas.concat(&sk_matrix(transform));
    canvas.draw_image_rect_with_sampling_options(
        &source.image,
        None,
        sk_bounds(source.bounds),
        SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::None),
        paint,
    );
    canvas.restore_to_count(save);
}

fn plan_resource_bounds(plan: &LayerRenderPlan, id: PlanResourceId, scale: f32) -> Bounds {
    let physical = plan.resources()[id.index()].physical_bounds;
    Bounds::from_origin_size(
        (physical.x as f32 / scale, physical.y as f32 / scale),
        (
            physical.width as f32 / scale,
            physical.height as f32 / scale,
        ),
    )
}

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

fn inverse_affine(value: Affine) -> Option<Affine> {
    let determinant = value.xx * value.yy - value.xy * value.yx;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let xx = value.yy / determinant;
    let xy = -value.xy / determinant;
    let yx = -value.yx / determinant;
    let yy = value.xx / determinant;
    Some(Affine::new(
        xx,
        xy,
        yx,
        yy,
        -(xx * value.dx + xy * value.dy),
        -(yx * value.dx + yy * value.dy),
    ))
}

fn color_matrix_filter(matrix: [f32; 20], input: Option<ImageFilter>) -> Option<ImageFilter> {
    let matrix = skia_safe::ColorMatrix::new(
        matrix[0], matrix[1], matrix[2], matrix[3], matrix[4], matrix[5], matrix[6], matrix[7],
        matrix[8], matrix[9], matrix[10], matrix[11], matrix[12], matrix[13], matrix[14],
        matrix[15], matrix[16], matrix[17], matrix[18], matrix[19],
    );
    let filter = skia_safe::color_filters::matrix(&matrix, None);
    skia_safe::image_filters::color_filter(filter, input, None)
}

fn extract_alpha_filter() -> ImageFilter {
    color_matrix_filter(
        [
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ],
        None,
    )
    .expect("a finite alpha color matrix is supported")
}

fn shadow_color_filter(color: Color) -> Option<skia_safe::ColorFilter> {
    let matrix = skia_safe::ColorMatrix::new(
        0.0, 0.0, 0.0, color.r, 0.0, 0.0, 0.0, 0.0, color.g, 0.0, 0.0, 0.0, 0.0, color.b, 0.0, 0.0,
        0.0, 0.0, color.a, 0.0,
    );
    Some(skia_safe::color_filters::matrix(&matrix, None))
}

fn draw_mask_shape(canvas: &Canvas, shape: MaskShape, paint: &Paint) {
    let unit = SkRect::from_xywh(0.0, 0.0, 1.0, 1.0);
    match shape {
        MaskShape::Rect => {
            canvas.draw_rect(unit, paint);
        }
        MaskShape::RoundedRect(radius) => {
            canvas.draw_round_rect(unit, radius.clamp(0.0, 0.5), radius.clamp(0.0, 0.5), paint);
        }
        MaskShape::Circle => {
            canvas.draw_circle((0.5, 0.5), 0.5, paint);
        }
        MaskShape::Ellipse => {
            canvas.draw_oval(unit, paint);
        }
        MaskShape::Line { from, to } => {
            let mut line = paint.clone();
            line.set_style(PaintStyle::Stroke);
            line.set_stroke_width(1.0);
            canvas.draw_line((from.x, from.y), (to.x, to.y), &line);
        }
    }
}

fn sk_blend_mode(value: BlendMode) -> SkBlendMode {
    match value {
        BlendMode::Normal => SkBlendMode::SrcOver,
        BlendMode::Multiply => SkBlendMode::Multiply,
        BlendMode::Screen => SkBlendMode::Screen,
        BlendMode::Overlay => SkBlendMode::Overlay,
        BlendMode::Darken => SkBlendMode::Darken,
        BlendMode::Lighten => SkBlendMode::Lighten,
        BlendMode::ColorDodge => SkBlendMode::ColorDodge,
        BlendMode::ColorBurn => SkBlendMode::ColorBurn,
        BlendMode::HardLight => SkBlendMode::HardLight,
        BlendMode::SoftLight => SkBlendMode::SoftLight,
        BlendMode::Difference => SkBlendMode::Difference,
        BlendMode::Exclusion => SkBlendMode::Exclusion,
        BlendMode::Hue => SkBlendMode::Hue,
        BlendMode::Saturation => SkBlendMode::Saturation,
        BlendMode::Color => SkBlendMode::Color,
        BlendMode::Luminosity => SkBlendMode::Luminosity,
    }
}

fn composite_blend_mode(blend: BlendMode, operator: CompositeOperator) -> SkBlendMode {
    if blend != BlendMode::Normal {
        return sk_blend_mode(blend);
    }
    match operator {
        CompositeOperator::SrcOver => SkBlendMode::SrcOver,
        CompositeOperator::Src => SkBlendMode::Src,
        CompositeOperator::DstOver => SkBlendMode::DstOver,
    }
}

fn blend_index(value: BlendMode) -> u32 {
    match value {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::HardLight => 8,
        BlendMode::SoftLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
        BlendMode::Hue => 12,
        BlendMode::Saturation => 13,
        BlendMode::Color => 14,
        BlendMode::Luminosity => 15,
    }
}

fn operator_index(value: CompositeOperator) -> u32 {
    match value {
        CompositeOperator::SrcOver => 0,
        CompositeOperator::Src => 1,
        CompositeOperator::DstOver => 2,
    }
}

fn valid_scale(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

fn sk_color(color: Color) -> skia_safe::Color {
    skia_safe::Color::from_argb(
        (color.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn sk_color4f(color: Color) -> Color4f {
    Color4f::new(color.r, color.g, color.b, color.a)
}

fn alpha_color(mut color: Color, opacity: f32) -> Color {
    color.a *= opacity.clamp(0.0, 1.0);
    color
}

fn sk_bounds(rect: Bounds) -> SkRect {
    SkRect::from_xywh(rect.x(), rect.y(), rect.width(), rect.height())
}

fn sk_rect(rect: Rect) -> SkRect {
    SkRect::from_xywh(rect.x, rect.y, rect.width, rect.height)
}

fn sk_matrix(value: Affine) -> Matrix {
    Matrix::new_all(
        value.xx, value.xy, value.dx, value.yx, value.yy, value.dy, 0.0, 0.0, 1.0,
    )
}

fn append_path(builder: &mut PathBuilder, path: &PathData) {
    for segment in path.segments() {
        match *segment {
            PathSegment::MoveTo(p) => {
                builder.move_to((p.x, p.y));
            }
            PathSegment::LineTo(p) => {
                builder.line_to((p.x, p.y));
            }
            PathSegment::QuadraticTo { control, to } => {
                builder.quad_to((control.x, control.y), (to.x, to.y));
            }
            PathSegment::CubicTo {
                control1,
                control2,
                to,
            } => {
                builder.cubic_to(
                    (control1.x, control1.y),
                    (control2.x, control2.y),
                    (to.x, to.y),
                );
            }
            PathSegment::Close => {
                builder.close();
            }
        }
    }
}

fn sk_path(path: &PathData) -> Path {
    let mut builder = PathBuilder::new();
    append_path(&mut builder, path);
    builder.detach()
}

fn solid_paint(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color4f(sk_color4f(color), ColorSpace::new_srgb().as_ref());
    paint
}

fn style_paint(style: ComputedColorStyle, rect: Bounds, opacity: f32) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    match style {
        ComputedColorStyle::Solid(color) => {
            paint.set_color4f(
                sk_color4f(alpha_color(color, opacity)),
                ColorSpace::new_srgb().as_ref(),
            );
        }
        ComputedColorStyle::LinearGradient(value) => {
            let colors = [
                sk_color4f(alpha_color(value.from, opacity)),
                sk_color4f(alpha_color(value.to, opacity)),
            ];
            let colors =
                Colors::new_evenly_spaced(&colors, TileMode::Clamp, ColorSpace::new_srgb());
            let gradient = Gradient::new(colors, Interpolation::default());
            let start = (
                rect.x() + rect.width() * value.start.x,
                rect.y() + rect.height() * value.start.y,
            );
            let end = (
                rect.x() + rect.width() * value.end.x,
                rect.y() + rect.height() * value.end.y,
            );
            if let Some(shader) = gradient::shaders::linear_gradient((start, end), &gradient, None)
            {
                paint.set_shader(shader);
            }
        }
        ComputedColorStyle::RadialGradient(value) => {
            let colors = [
                sk_color4f(alpha_color(value.from, opacity)),
                sk_color4f(alpha_color(value.to, opacity)),
            ];
            let colors =
                Colors::new_evenly_spaced(&colors, TileMode::Clamp, ColorSpace::new_srgb());
            let gradient = Gradient::new(colors, Interpolation::default());
            let center = (
                rect.x() + rect.width() * value.center.x,
                rect.y() + rect.height() * value.center.y,
            );
            let radius = value.radius * rect.width().min(rect.height());
            if let Some(shader) =
                gradient::shaders::radial_gradient((center, radius.max(0.001)), &gradient, None)
            {
                paint.set_shader(shader);
            }
        }
    }
    paint
}

fn draw_shape(
    canvas: &Canvas,
    primitive: &xui::render::ShapePrimitive,
    transform: Affine,
    opacity: f32,
) {
    let save = canvas.save();
    canvas.concat(&sk_matrix(transform));
    if let Some(shadow) = primitive.shadow.filter(|s| s.color.a > 0.0) {
        let mut paint = solid_paint(alpha_color(shadow.color, opacity));
        paint.set_mask_filter(skia_safe::MaskFilter::blur(
            skia_safe::BlurStyle::Normal,
            shadow.blur.max(0.0),
            false,
        ));
        draw_shape_geometry(
            canvas,
            primitive.shape,
            primitive.bounds.expand(shadow.spread),
            shadow.offset,
            &paint,
        );
    }
    if let Some(fill) = primitive.fill {
        let paint = style_paint(fill, primitive.bounds, opacity);
        draw_shape_geometry(
            canvas,
            primitive.shape,
            primitive.bounds,
            xui_interface::Point::new(0.0, 0.0),
            &paint,
        );
    }
    if let Some(stroke) = primitive.stroke.filter(|s| s.width > 0.0) {
        let mut paint = style_paint(stroke.color, primitive.bounds, opacity);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke.width);
        draw_shape_geometry(
            canvas,
            primitive.shape,
            primitive.bounds,
            xui_interface::Point::new(0.0, 0.0),
            &paint,
        );
    }
    canvas.restore_to_count(save);
}

fn draw_shape_geometry(
    canvas: &Canvas,
    shape: Shape,
    rect: Bounds,
    offset: xui_interface::Point,
    paint: &Paint,
) {
    let rect = rect.translate(offset);
    match shape {
        Shape::Rect => {
            canvas.draw_rect(sk_bounds(rect), paint);
        }
        Shape::RoundedRect(radius) => {
            canvas.draw_round_rect(sk_bounds(rect), radius, radius, paint);
        }
        Shape::Circle => {
            canvas.draw_circle(
                (
                    rect.x() + rect.width() * 0.5,
                    rect.y() + rect.height() * 0.5,
                ),
                rect.width().min(rect.height()) * 0.5,
                paint,
            );
        }
        Shape::Ellipse => {
            canvas.draw_oval(sk_bounds(rect), paint);
        }
        Shape::Line { from, to } => {
            canvas.draw_line(
                (from.x + offset.x, from.y + offset.y),
                (to.x + offset.x, to.y + offset.y),
                paint,
            );
        }
    }
}

fn draw_vector(
    canvas: &Canvas,
    commands: &[CompiledVectorCommand],
    primitive_transform: Affine,
    transform: Affine,
    opacity: f32,
) {
    let outer = primitive_transform.then(transform);
    for command in commands {
        match command {
            CompiledVectorCommand::FillPath {
                path,
                transform,
                fill,
            } => {
                let save = canvas.save();
                canvas.concat(&sk_matrix(transform.then(outer)));
                let mut path = path.clone();
                path.set_fill_type(match fill.rule {
                    xui_interface::FillRule::NonZero => skia_safe::PathFillType::Winding,
                    xui_interface::FillRule::EvenOdd => skia_safe::PathFillType::EvenOdd,
                });
                canvas.draw_path(&path, &solid_paint(alpha_color(fill.color, opacity)));
                canvas.restore_to_count(save);
            }
            CompiledVectorCommand::StrokePath {
                path,
                transform,
                stroke,
            } if stroke.width > 0.0 => {
                let save = canvas.save();
                canvas.concat(&sk_matrix(transform.then(outer)));
                let mut paint = solid_paint(alpha_color(stroke.color, opacity));
                paint.set_style(PaintStyle::Stroke);
                paint.set_stroke_width(stroke.width);
                paint.set_stroke_cap(match stroke.cap {
                    LineCap::Butt => SkCap::Butt,
                    LineCap::Square => SkCap::Square,
                    LineCap::Round => SkCap::Round,
                });
                paint.set_stroke_join(match stroke.join {
                    LineJoin::Miter => SkJoin::Miter,
                    LineJoin::Bevel => SkJoin::Bevel,
                    LineJoin::Round => SkJoin::Round,
                });
                if let Some(dash) = stroke.effective_dash() {
                    // Skia needs an even interval count; an odd pattern repeats
                    // to close the cycle, which is what SVG does too.
                    let mut intervals = dash.intervals().to_vec();
                    if intervals.len() % 2 == 1 {
                        intervals.extend_from_within(..);
                    }
                    if let Some(effect) = skia_safe::PathEffect::dash(&intervals, dash.offset) {
                        paint.set_path_effect(effect);
                    }
                }
                canvas.draw_path(path, &paint);
                canvas.restore_to_count(save);
            }
            CompiledVectorCommand::StrokePath { .. } => {}
        }
    }
}

fn make_image(data: &ImageData, transform: ImageTransform) -> Result<Image, SkiaBackendError> {
    if transform == ImageTransform::default() {
        return make_image_from_pixels(data.pixels.as_ref(), data.size.width, data.size.height);
    }
    let (pixels, width, height) = transform_image_pixels(data, transform);
    make_image_from_pixels(&pixels, width, height)
}

fn make_image_from_pixels(
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<Image, SkiaBackendError> {
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        ColorSpace::new_srgb(),
    );
    images::raster_from_data(&info, Data::new_copy(pixels), width as usize * 4)
        .ok_or(SkiaBackendError::SurfaceAllocation { width, height })
}

fn transform_image_pixels(data: &ImageData, transform: ImageTransform) -> (Vec<u8>, u32, u32) {
    let source_width = data.size.width;
    let source_height = data.size.height;
    let (width, height) = match transform.rotate {
        ImageRotation::Deg0 | ImageRotation::Deg180 => (source_width, source_height),
        ImageRotation::Deg90 | ImageRotation::Deg270 => (source_height, source_width),
    };
    let mut output = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let mut u = x;
            let mut v = y;
            if transform.flip_x {
                u = width - 1 - u;
            }
            if transform.flip_y {
                v = height - 1 - v;
            }
            let (sx, sy) = match transform.rotate {
                ImageRotation::Deg0 => (u, v),
                ImageRotation::Deg90 => (v, source_height - 1 - u),
                ImageRotation::Deg180 => (source_width - 1 - u, source_height - 1 - v),
                ImageRotation::Deg270 => (source_width - 1 - v, u),
            };
            let src = (sy as usize * source_width as usize + sx as usize) * 4;
            let dst = (y as usize * width as usize + x as usize) * 4;
            output[dst..dst + 4].copy_from_slice(&data.pixels[src..src + 4]);
        }
    }
    (output, width, height)
}

fn fitted_image_rect(container: Bounds, image: Size<u32>, style: ImageStyle) -> Option<Bounds> {
    if container.width() <= 0.0
        || container.height() <= 0.0
        || image.width == 0
        || image.height == 0
    {
        return None;
    }
    let iw = image.width as f32;
    let ih = image.height as f32;
    let sx = container.width() / iw;
    let sy = container.height() / ih;
    let scale = match style.fit {
        ImageFit::Fill => return Some(container),
        ImageFit::Contain => sx.min(sy),
        ImageFit::Cover => sx.max(sy),
        ImageFit::None => 1.0,
        ImageFit::ScaleDown => sx.min(sy).min(1.0),
    };
    let size = Size::new(iw * scale, ih * scale);
    Some(aligned_rect(container, size, style.alignment))
}

fn aligned_rect(container: Bounds, size: Size<f32>, alignment: Alignment) -> Bounds {
    Bounds::from_origin_size(
        (
            container.x() + (container.width() - size.width) * alignment.x,
            container.y() + (container.height() - size.height) * alignment.y,
        ),
        size,
    )
}

fn image_tiles(container: Bounds, tile: Bounds, repeat: ImageRepeat) -> Vec<Bounds> {
    let repeat_x = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatX);
    let repeat_y = matches!(repeat, ImageRepeat::Repeat | ImageRepeat::RepeatY);
    if !repeat_x && !repeat_y {
        return vec![tile];
    }
    let start_x = if repeat_x {
        container.x() + (tile.x() - container.x()).rem_euclid(tile.width()) - tile.width()
    } else {
        tile.x()
    };
    let start_y = if repeat_y {
        container.y() + (tile.y() - container.y()).rem_euclid(tile.height()) - tile.height()
    } else {
        tile.y()
    };
    let end_x = if repeat_x {
        container.x() + container.width()
    } else {
        tile.x() + tile.width()
    };
    let end_y = if repeat_y {
        container.y() + container.height()
    } else {
        tile.y() + tile.height()
    };
    let mut result = Vec::new();
    let mut y = start_y;
    while y < end_y {
        let mut x = start_x;
        while x < end_x {
            result.push(Bounds::from_origin_size(
                (x, y),
                (tile.width(), tile.height()),
            ));
            if !repeat_x {
                break;
            }
            x += tile.width();
        }
        if !repeat_y {
            break;
        }
        y += tile.height();
    }
    result
}

fn sampling_options(value: Sampling) -> SamplingOptions {
    match value {
        Sampling::Nearest => {
            SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None)
        }
        Sampling::Linear => {
            SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::None)
        }
        Sampling::Cubic => SamplingOptions::from(skia_safe::CubicResampler::catmull_rom()),
    }
}

fn draw_text_decorations(
    canvas: &Canvas,
    lines: &[xui_interface::LineLayout],
    primitive: &xui::render::TextPrimitive,
    origin: xui_interface::Point,
    opacity: f32,
) {
    let decoration = primitive.paint.style.decoration;
    if !decoration.underline && !decoration.line_through {
        return;
    }
    let mut paint = solid_paint(alpha_color(primitive.paint.style.color, opacity));
    paint.set_stroke_width((primitive.paint.style.font_size / 16.0).max(1.0));
    for line in lines {
        if decoration.underline {
            let y = origin.y + line.baseline + paint.stroke_width();
            canvas.draw_line(
                (origin.x + line.x, y),
                (origin.x + line.x + line.width, y),
                &paint,
            );
        }
        if decoration.line_through {
            let y = origin.y + line.y + line.height * 0.5;
            canvas.draw_line(
                (origin.x + line.x, y),
                (origin.x + line.x + line.width, y),
                &paint,
            );
        }
    }
}

#[cfg(test)]
mod text_draw_tests {
    use super::*;
    use xui_interface::{
        FontWeight, ParagraphStyle, Shaper, TextBoxStyle, TextContent, TextLayoutConstraints,
        TextLayoutInput, TextStyle,
    };
    use xui_text_engine::CosmicEngine;

    #[test]
    fn draws_shaper_output_without_backend_rasterization() {
        let mut text_backend = CosmicEngine::new(1.0);
        let mut state = text_backend.create_state();
        let layout = text_backend.layout_paragraph(
            &mut state,
            TextLayoutInput::new(
                TextContent::from_static("Skia draw_glyphs 啊，是关中王来啦"),
                TextLayoutConstraints::max_width(240.0),
                TextStyle {
                    font_weight: FontWeight::Thin,
                    ..TextStyle::default()
                }
                .into(),
                ParagraphStyle::default(),
                TextBoxStyle::default(),
                0,
            ),
        );
        assert!(!layout.runs.is_empty());

        let mut renderer =
            SkiaBackend::<CosmicEngine>::headless(1.0, SkiaBackendOptions::default());
        let mut surface = skia_safe::surfaces::raster_n32_premul((240, 80)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        renderer
            .draw_glyphs(
                &text_backend,
                surface.canvas(),
                &layout,
                xui_interface::Point::new(0.0, 0.0),
                Color::WHITE,
            )
            .unwrap();

        let mut pixels = vec![0; 240 * 80 * 4];
        let info = ImageInfo::new(
            (240, 80),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        assert!(surface.read_pixels(&info, &mut pixels, 240 * 4, (0, 0)));
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
        assert!(!renderer.font_cache.is_empty());
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use xui::render::{
//         BackdropIsolation, BuiltDrawData, BuiltLayer, BuiltLayerInstance, BuiltShape, CachePolicy,
//         CompositePrefix, CompositePrefixId, CompositeStyle, ContentVersion, LayerDescriptor,
//         PlacementVersion, ShapePrimitive, SurfacePrefix,
//     };
//     use xui_interface::{
//         ComputedBackdropFilter, ComputedBackdropMask, ComputedBackdropStyle, ComputedEffect,
//         FilterQuality, ImageKey, Point,
//     };
//     use xui_text_engine::CosmicEngine;

//     type TestBackend = super::SkiaBackend<CosmicEngine>;

//     fn shape_frame(clip: Option<Rect>) -> BuiltFrame {
//         let source = xui::render::RenderNodeId::default();
//         let clip_chains = clip
//             .map(|rect| {
//                 vec![xui::render::BuiltClipChain {
//                     source,
//                     parent: None,
//                     clip: ClipShape::Rect(rect),
//                     world_transform: Affine::IDENTITY,
//                     world_bounds: rect,
//                 }]
//             })
//             .unwrap_or_default();
//         BuiltFrame {
//             root_layer: BuiltLayerId(0),
//             layers: vec![BuiltLayer {
//                 source,
//                 content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                 render_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                 content_version: ContentVersion::default(),
//                 cache_id: None,
//                 cache_policy: CachePolicy::None,
//                 backdrop_isolation: BackdropIsolation::Isolate,
//                 items: vec![BuiltItem::Draw(BuiltDraw::Shape(BuiltShape {
//                     common: BuiltDrawData {
//                         source,
//                         content_version: ContentVersion::default(),
//                         world_transform: Affine::IDENTITY,
//                         world_bounds: Rect::new(1.0, 1.0, 8.0, 8.0),
//                         clip_chain: clip.map(|_| BuiltClipChainId(0)),
//                     },
//                     primitive: ShapePrimitive {
//                         bounds: Rect::new(1.0, 1.0, 8.0, 8.0),
//                         shape: Shape::Rect,
//                         fill: Some(ComputedColorStyle::Solid(Color::rgb(1.0, 0.0, 0.0))),
//                         stroke: None,
//                         shadow: None,
//                     },
//                 }))],
//             }],
//             layer_instances: Vec::new(),
//             composite_prefixes: Vec::new(),
//             clip_chains,
//             live_layer_caches: Vec::new(),
//             scene_revision: 1,
//             properties_revision: 0,
//         }
//     }

//     fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
//         let index = (y * width + x) * 4;
//         pixels[index..index + 4].try_into().unwrap()
//     }

//     fn layer_instance(descriptor: &LayerDescriptor) -> BuiltLayerInstance {
//         let source = xui::render::RenderNodeId::default();
//         BuiltLayerInstance {
//             source,
//             layer: BuiltLayerId(0),
//             composite: descriptor.composite.render_graph_instance(),
//             render_program: descriptor
//                 .bind_render_program(Arc::new(descriptor.compile_render_program().unwrap()))
//                 .unwrap(),
//             clip_chain: None,
//             world_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//             placement_version: PlacementVersion::default(),
//             destination_prefix: None,
//         }
//     }

//     fn shape_draw(rect: Rect, color: Color) -> BuiltItem {
//         let source = xui::render::RenderNodeId::default();
//         BuiltItem::Draw(BuiltDraw::Shape(BuiltShape {
//             common: BuiltDrawData {
//                 source,
//                 content_version: ContentVersion::default(),
//                 world_transform: Affine::IDENTITY,
//                 world_bounds: rect,
//                 clip_chain: None,
//             },
//             primitive: ShapePrimitive {
//                 bounds: rect,
//                 shape: Shape::Rect,
//                 fill: Some(ComputedColorStyle::Solid(color)),
//                 stroke: None,
//                 shadow: None,
//             },
//         }))
//     }

//     fn layered_frame(
//         descriptor: &LayerDescriptor,
//         mut root_items: Vec<BuiltItem>,
//         child_items: Vec<BuiltItem>,
//     ) -> BuiltFrame {
//         let source = xui::render::RenderNodeId::default();
//         let item_count = root_items.len();
//         let needs_backdrop = descriptor
//             .compile_render_program()
//             .unwrap()
//             .external_resource(ExternalResourceKind::Backdrop)
//             .is_some();
//         root_items.push(BuiltItem::Layer(xui::render::BuiltLayerInstanceId(0)));
//         let mut instance = layer_instance(descriptor);
//         instance.layer = BuiltLayerId(1);
//         instance.destination_prefix = needs_backdrop.then_some(CompositePrefixId(0));
//         BuiltFrame {
//             root_layer: BuiltLayerId(0),
//             layers: vec![
//                 BuiltLayer {
//                     source,
//                     content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     render_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     content_version: ContentVersion::default(),
//                     cache_id: None,
//                     cache_policy: CachePolicy::None,
//                     backdrop_isolation: BackdropIsolation::Isolate,
//                     items: root_items,
//                 },
//                 BuiltLayer {
//                     source,
//                     content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     render_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     content_version: ContentVersion::default(),
//                     cache_id: None,
//                     cache_policy: CachePolicy::None,
//                     backdrop_isolation: BackdropIsolation::Isolate,
//                     items: child_items,
//                 },
//             ],
//             layer_instances: vec![instance],
//             composite_prefixes: if needs_backdrop {
//                 vec![CompositePrefix {
//                     parent: None,
//                     local: SurfacePrefix {
//                         layer: BuiltLayerId(0),
//                         item_count,
//                     },
//                     placement: None,
//                 }]
//             } else {
//                 Vec::new()
//             },
//             clip_chains: Vec::new(),
//             live_layer_caches: Vec::new(),
//             scene_revision: 1,
//             properties_revision: 0,
//         }
//     }

//     fn render_frame(frame: &BuiltFrame) -> Vec<u8> {
//         let mut backend = TestBackend::headless(
//             1.0,
//             SkiaBackendOptions {
//                 clear_color: Color::BLACK,
//                 ..SkiaBackendOptions::default()
//             },
//         );
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend.submit(frame, &mut text).unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(&mut backend).unwrap();
//         backend.read_pixels_rgba8().unwrap()
//     }

//     fn cached_shape_frame(left: Color, left_version: u64, policy: CachePolicy) -> BuiltFrame {
//         let mut ids = xui::render::RenderScene::new();
//         let root_source = ids.root();
//         let child_source = ids.insert_group();
//         let left_source = ids.insert_group();
//         let right_source = ids.insert_group();
//         let descriptor = LayerDescriptor {
//             cache_policy: policy,
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let mut frame = layered_frame(
//             &descriptor,
//             Vec::new(),
//             vec![
//                 shape_draw(Rect::new(0.0, 0.0, 5.0, 10.0), left),
//                 shape_draw(Rect::new(5.0, 0.0, 5.0, 10.0), Color::rgb(0.0, 0.0, 1.0)),
//             ],
//         );
//         frame.layers[0].source = root_source;
//         frame.layers[1].source = child_source;
//         frame.layers[1].content_version.paint = left_version;
//         frame.layers[1].cache_policy = policy;
//         frame.layer_instances[0].source = child_source;
//         frame.layers[1].cache_id = Some(xui::render::LayerCacheId::Scene(child_source));
//         frame.live_layer_caches = (policy != CachePolicy::None)
//             .then_some(xui::render::LayerCacheId::Scene(child_source))
//             .into_iter()
//             .collect();
//         let BuiltItem::Draw(BuiltDraw::Shape(left_draw)) = &mut frame.layers[1].items[0] else {
//             unreachable!()
//         };
//         left_draw.common.source = left_source;
//         left_draw.common.content_version.paint = left_version;
//         let BuiltItem::Draw(BuiltDraw::Shape(right_draw)) = &mut frame.layers[1].items[1] else {
//             unreachable!()
//         };
//         right_draw.common.source = right_source;
//         frame
//     }

//     fn submit_headless_frame(backend: &mut TestBackend, frame: &BuiltFrame) -> Vec<u8> {
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend.submit(frame, &mut text).unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(backend).unwrap();
//         backend.read_pixels_rgba8().unwrap()
//     }

//     #[test]
//     fn cached_surface_is_reused_for_partial_repaint() {
//         let options = SkiaBackendOptions {
//             clear_color: Color::BLACK,
//             ..SkiaBackendOptions::default()
//         };
//         let mut backend = TestBackend::headless(1.0, options);
//         let first = cached_shape_frame(Color::rgb(1.0, 0.0, 0.0), 0, CachePolicy::Always);
//         submit_headless_frame(&mut backend, &first);
//         let initial = backend.layer_cache_stats();
//         assert_eq!(initial.misses, 1);
//         assert_eq!(initial.entries, 1);
//         assert_eq!(initial.full_updates, 1);

//         let mut second = first.clone();
//         second.layers[1].content_version.paint = 1;
//         let BuiltItem::Draw(BuiltDraw::Shape(left_draw)) = &mut second.layers[1].items[0] else {
//             unreachable!()
//         };
//         left_draw.common.content_version.paint = 1;
//         left_draw.primitive.fill = Some(ComputedColorStyle::Solid(Color::rgb(0.0, 1.0, 0.0)));
//         let partial_pixels = submit_headless_frame(&mut backend, &second);
//         let full_pixels = render_frame(&second);
//         assert_eq!(partial_pixels, full_pixels);
//         assert!(pixel(&partial_pixels, 10, 2, 5)[1] > 245);
//         assert!(pixel(&partial_pixels, 10, 7, 5)[2] > 245);
//         let updated = backend.layer_cache_stats();
//         assert_eq!(updated.hits, 1);
//         assert_eq!(updated.partial_updates, 1, "{updated:?}");
//         assert_eq!(updated.entries, 1);
//     }

//     #[test]
//     fn unchanged_frame_keeps_root_surface_generation() {
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         let frame = cached_shape_frame(Color::WHITE, 0, CachePolicy::Always);
//         let first_pixels = submit_headless_frame(&mut backend, &frame);
//         let generation = backend.raster.as_mut().unwrap().generation_id();
//         let stats = backend.layer_cache_stats();

//         let second_pixels = submit_headless_frame(&mut backend, &frame);
//         assert_eq!(first_pixels, second_pixels);
//         assert_eq!(backend.raster.as_mut().unwrap().generation_id(), generation);
//         assert_eq!(backend.layer_cache_stats().hits, stats.hits);
//         assert_eq!(backend.layer_cache_stats().dirty_regions, 0);
//     }

//     #[test]
//     fn moved_draw_clears_old_bounds_and_matches_full_repaint() {
//         let mut backend = TestBackend::headless(
//             1.0,
//             SkiaBackendOptions {
//                 clear_color: Color::BLACK,
//                 ..SkiaBackendOptions::default()
//             },
//         );
//         let first = shape_frame(None);
//         submit_headless_frame(&mut backend, &first);

//         let mut moved = first.clone();
//         let BuiltItem::Draw(BuiltDraw::Shape(draw)) = &mut moved.layers[0].items[0] else {
//             unreachable!()
//         };
//         let bounds = Rect::new(5.0, 1.0, 4.0, 8.0);
//         draw.common.world_bounds = bounds;
//         draw.common.content_version.geometry = 1;
//         draw.primitive.bounds = bounds;
//         let partial = submit_headless_frame(&mut backend, &moved);
//         assert_eq!(partial, render_frame(&moved));
//         assert!(pixel(&partial, 10, 2, 5)[0] < 10);
//         assert!(pixel(&partial, 10, 7, 5)[0] > 245);
//     }

//     #[cfg(not(target_os = "macos"))]
//     #[test]
//     fn logical_damage_is_rounded_outward_in_physical_pixels() {
//         let rects = physical_damage_rects(
//             &DamageRegion::full(Rect::new(0.25, 0.5, 1.0, 1.0)),
//             2.0,
//             Size::new(10, 10),
//         );
//         assert_eq!(rects.len(), 1);
//         assert_eq!(rects[0].x, 0);
//         assert_eq!(rects[0].y, 1);
//         assert_eq!(rects[0].width.get(), 3);
//         assert_eq!(rects[0].height.get(), 2);
//     }

//     #[test]
//     fn auto_surfaces_obey_budget_while_always_surfaces_remain() {
//         let options = SkiaBackendOptions {
//             clear_color: Color::BLACK,
//             layer_cache_budget_bytes: 0,
//         };
//         let mut auto = TestBackend::headless(1.0, options);
//         submit_headless_frame(
//             &mut auto,
//             &cached_shape_frame(Color::WHITE, 0, CachePolicy::Auto),
//         );
//         assert_eq!(auto.layer_cache_stats().entries, 0);

//         let mut always = TestBackend::headless(1.0, options);
//         submit_headless_frame(
//             &mut always,
//             &cached_shape_frame(Color::WHITE, 0, CachePolicy::Always),
//         );
//         assert_eq!(always.layer_cache_stats().entries, 1);
//         assert_eq!(always.layer_cache_stats().resident_bytes, 400);
//     }

//     fn nested_backdrop_frame(isolation: BackdropIsolation) -> BuiltFrame {
//         let source = xui::render::RenderNodeId::default();
//         let outer_descriptor = LayerDescriptor {
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let inner_descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([ComputedBackdropFilter::Invert(1.0)]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let mut outer = layer_instance(&outer_descriptor);
//         outer.layer = BuiltLayerId(1);
//         let mut inner = layer_instance(&inner_descriptor);
//         inner.layer = BuiltLayerId(2);
//         inner.destination_prefix = Some(CompositePrefixId(1));
//         let bounds = Rect::new(0.0, 0.0, 10.0, 10.0);
//         let built_layer = |items, backdrop_isolation| BuiltLayer {
//             source,
//             content_bounds: bounds,
//             render_bounds: bounds,
//             content_version: ContentVersion::default(),
//             cache_id: None,
//             cache_policy: CachePolicy::None,
//             backdrop_isolation,
//             items,
//         };
//         BuiltFrame {
//             root_layer: BuiltLayerId(0),
//             layers: vec![
//                 built_layer(
//                     vec![
//                         shape_draw(bounds, Color::rgb(1.0, 0.0, 0.0)),
//                         BuiltItem::Layer(xui::render::BuiltLayerInstanceId(0)),
//                     ],
//                     BackdropIsolation::Isolate,
//                 ),
//                 built_layer(
//                     vec![BuiltItem::Layer(xui::render::BuiltLayerInstanceId(1))],
//                     isolation,
//                 ),
//                 built_layer(Vec::new(), BackdropIsolation::Isolate),
//             ],
//             layer_instances: vec![outer, inner],
//             composite_prefixes: vec![
//                 CompositePrefix {
//                     parent: None,
//                     local: SurfacePrefix {
//                         layer: BuiltLayerId(0),
//                         item_count: 1,
//                     },
//                     placement: None,
//                 },
//                 CompositePrefix {
//                     parent: Some(CompositePrefixId(0)),
//                     local: SurfacePrefix {
//                         layer: BuiltLayerId(1),
//                         item_count: 0,
//                     },
//                     placement: Some(xui::render::BuiltLayerInstanceId(0)),
//                 },
//             ],
//             clip_chains: Vec::new(),
//             live_layer_caches: Vec::new(),
//             scene_revision: 1,
//             properties_revision: 0,
//         }
//     }

//     #[test]
//     fn headless_backend_renders_at_physical_scale() {
//         let mut backend = TestBackend::headless(2.0, SkiaBackendOptions::default());
//         let mut text = TextHost::new(CosmicEngine::new(2.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend.submit(&shape_frame(None), &mut text).unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(&mut backend).unwrap();

//         assert_eq!(backend.frame_size_px(), Size::new(20, 20));
//         let pixels = backend.read_pixels_rgba8().unwrap();
//         let inside = pixel(&pixels, 20, 10, 10);
//         assert!(inside[0] > 245 && inside[1] < 10 && inside[2] < 10);
//         assert!(<TestBackend as RenderBackend<TextHost<CosmicEngine>>>::did_present(&backend));
//     }

//     #[test]
//     fn clip_chain_limits_shape_output() {
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend
//             .submit(
//                 &shape_frame(Some(Rect::new(0.0, 0.0, 5.0, 10.0))),
//                 &mut text,
//             )
//             .unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(&mut backend).unwrap();

//         let pixels = backend.read_pixels_rgba8().unwrap();
//         assert!(pixel(&pixels, 10, 2, 5)[0] > 245);
//         let outside = pixel(&pixels, 10, 8, 5);
//         assert!(outside[0] < 40 && outside[1] < 40 && outside[2] < 40);
//     }

//     #[test]
//     fn image_rotation_and_flips_rearrange_pixels() {
//         let data = ImageData::rgba8(Size::new(2, 1), [255, 0, 0, 255, 0, 255, 0, 255]);
//         let (rotated, width, height) = transform_image_pixels(
//             &data,
//             ImageTransform {
//                 rotate: ImageRotation::Deg90,
//                 ..ImageTransform::default()
//             },
//         );
//         assert_eq!((width, height), (1, 2));
//         assert_eq!(&rotated[..4], &[255, 0, 0, 255]);
//         assert_eq!(&rotated[4..], &[0, 255, 0, 255]);

//         let (flipped, _, _) = transform_image_pixels(
//             &data,
//             ImageTransform {
//                 flip_x: true,
//                 ..ImageTransform::default()
//             },
//         );
//         assert_eq!(&flipped[..4], &[0, 255, 0, 255]);
//     }

//     #[test]
//     fn common_layer_effects_lower_to_an_executable_plan() {
//         let descriptor = LayerDescriptor {
//             effects: Arc::from([
//                 ComputedEffect::Blur {
//                     sigma_x: 1.5,
//                     sigma_y: 2.0,
//                     quality: FilterQuality::Medium,
//                 },
//                 ComputedEffect::ColorMatrix([
//                     1.0, 0.0, 0.0, 0.0, 0.0, // red
//                     0.0, 1.0, 0.0, 0.0, 0.0, // green
//                     0.0, 0.0, 1.0, 0.0, 0.0, // blue
//                     0.0, 0.0, 0.0, 1.0, 0.0, // alpha
//                 ]),
//                 ComputedEffect::DropShadow {
//                     color: Color::rgba(0.0, 0.0, 0.0, 0.75),
//                     offset: Point::new(2.0, 3.0),
//                     sigma_x: 2.0,
//                     sigma_y: 2.0,
//                     spread: 1.0,
//                     quality: FilterQuality::High,
//                 },
//             ]),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let instance = layer_instance(&descriptor);
//         let plan = instance
//             .render_program
//             .program()
//             .instantiate(&LayerPlanContext {
//                 backdrop_source_bounds: Rect::new(0.0, 0.0, 20.0, 20.0),
//                 parent_destination_bounds: Rect::new(0.0, 0.0, 20.0, 20.0),
//                 composite_clip_bounds: Some(Rect::new(0.0, 0.0, 10.0, 10.0)),
//                 layer_content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                 backdrop_bounds: None,
//                 composite: instance.composite,
//                 scale_factor: 1.0,
//                 color_texture_class: TextureClass::LINEAR_COLOR,
//                 external_aliasing: ExternalAliasing::Distinct,
//                 limits: PlanLimits::default(),
//             })
//             .unwrap();
//         assert!(plan.passes().len() >= 5);
//     }

//     #[test]
//     fn custom_effects_and_composite_blender_compile() {
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         assert!(
//             backend
//                 .runtime_filter("pixelate", PIXELATE_SKSL, &[("block", &[4.0, 4.0])])
//                 .is_ok()
//         );
//         assert!(
//             backend
//                 .runtime_filter(
//                     "refraction",
//                     REFRACTION_SKSL,
//                     &[("center", &[5.0, 5.0]), ("amount", &[2.0, 1.0])],
//                 )
//                 .is_ok()
//         );
//         assert!(
//             backend
//                 .runtime_filter(
//                     "chromatic-aberration",
//                     CHROMATIC_ABERRATION_SKSL,
//                     &[("offset", &[1.0, 0.0])],
//                 )
//                 .is_ok()
//         );
//         assert!(
//             backend
//                 .runtime_blender(BlendMode::Multiply, CompositeOperator::DstOver)
//                 .is_ok()
//         );
//     }

//     #[test]
//     fn image_mask_is_accepted_and_backdrop_requires_a_valid_prefix() {
//         let mut frame = shape_frame(None);
//         let backend = TestBackend::headless(1.0, SkiaBackendOptions::default());

//         let image_mask = LayerDescriptor {
//             effects: Arc::from([ComputedEffect::ImageMask {
//                 image: ImageKey::UserProvided(7),
//                 data: ImageData::rgba8(Size::new(1, 1), [255, 255, 255, 255]),
//                 bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
//             }]),
//             ..LayerDescriptor::default()
//         };
//         frame.layer_instances = vec![layer_instance(&image_mask)];
//         assert!(backend.validate_frame(&frame).is_ok());

//         let plain_backdrop = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([ComputedBackdropFilter::Blur {
//                     sigma_x: 2.0,
//                     sigma_y: 2.0,
//                     quality: FilterQuality::Medium,
//                 }]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             ..LayerDescriptor::default()
//         };
//         frame.layer_instances = vec![layer_instance(&plain_backdrop)];
//         assert!(matches!(
//             backend.validate_frame(&frame),
//             Err(SkiaBackendError::InvalidFrame(_))
//         ));
//     }

//     #[test]
//     fn image_mask_is_executed_by_the_offscreen_plan() {
//         let descriptor = LayerDescriptor {
//             effects: Arc::from([ComputedEffect::ImageMask {
//                 image: ImageKey::UserProvided(17),
//                 data: ImageData::rgba8(Size::new(2, 1), [255, 255, 255, 255, 255, 255, 255, 0]),
//                 bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//             }]),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(0.0, 0.0, 1.0),
//             )],
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(1.0, 0.0, 0.0),
//             )],
//         );
//         let pixels = render_frame(&frame);
//         let left = pixel(&pixels, 10, 2, 5);
//         let right = pixel(&pixels, 10, 8, 5);
//         assert!(left[0] > 200 && left[2] < 40, "left={left:?}");
//         assert!(right[2] > 200 && right[0] < 40, "right={right:?}");
//     }

//     #[test]
//     fn frame_stats_expose_render_graph_and_surface_work() {
//         let descriptor = LayerDescriptor {
//             effects: Arc::from([
//                 ComputedEffect::Blur {
//                     sigma_x: 2.0,
//                     sigma_y: 2.0,
//                     quality: FilterQuality::Medium,
//                 },
//                 ComputedEffect::ColorMatrix([
//                     1.0, 0.0, 0.0, 0.0, 0.0, // red
//                     0.0, 1.0, 0.0, 0.0, 0.0, // green
//                     0.0, 0.0, 1.0, 0.0, 0.0, // blue
//                     0.0, 0.0, 0.0, 1.0, 0.0, // alpha
//                 ]),
//                 ComputedEffect::DropShadow {
//                     color: Color::rgba(0.0, 0.0, 0.0, 0.5),
//                     offset: Point::new(1.0, 1.0),
//                     sigma_x: 1.0,
//                     sigma_y: 1.0,
//                     spread: 1.0,
//                     quality: FilterQuality::Medium,
//                 },
//             ]),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             Vec::new(),
//             vec![shape_draw(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE)],
//         );
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         submit_headless_frame(&mut backend, &frame);

//         let stats = backend.frame_stats();
//         assert_eq!(stats.frame_index, 1);
//         assert!(stats.root_damage_rects > 0, "{stats:?}");
//         assert_eq!(stats.layer_draws, 2);
//         assert_eq!(stats.primitive_draws, 1);
//         assert_eq!(stats.render_plans, 1);
//         assert!(stats.render_passes >= 3, "{stats:?}");
//         assert!(stats.planned_transient_resources > 0, "{stats:?}");
//         assert!(stats.planned_transient_slots > 0, "{stats:?}");
//         assert_eq!(
//             stats.transient_surface_allocations,
//             stats.planned_transient_slots
//         );
//         assert!(stats.transient_surface_reuses > 0, "{stats:?}");
//         assert!(stats.offscreen_surface_allocations > 0, "{stats:?}");
//         assert!(stats.image_snapshots > 0, "{stats:?}");
//         assert_eq!(stats.backdrop_materializations, 0, "{stats:?}");
//         assert_eq!(stats.backdrop_materializations_avoided, 1, "{stats:?}");
//     }

//     #[test]
//     fn pixelate_backdrop_samples_only_the_prior_destination() {
//         let descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([ComputedBackdropFilter::Pixelate {
//                     size: Size::new(4.0, 4.0),
//                 }]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![
//                 shape_draw(Rect::new(0.0, 0.0, 5.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
//                 shape_draw(Rect::new(5.0, 0.0, 5.0, 10.0), Color::rgb(0.0, 0.0, 1.0)),
//             ],
//             Vec::new(),
//         );
//         let pixels = render_frame(&frame);
//         let left = pixel(&pixels, 10, 1, 5);
//         let right = pixel(&pixels, 10, 8, 5);
//         assert!(left[0] > 180 && left[2] < 80, "left={left:?}");
//         assert!(right[2] > 180 && right[0] < 80, "right={right:?}");
//     }

//     #[test]
//     fn nested_backdrop_respects_passthrough_and_isolation() {
//         let passthrough_frame = nested_backdrop_frame(BackdropIsolation::Passthrough);
//         let isolated_frame = nested_backdrop_frame(BackdropIsolation::Isolate);
//         assert!(BackdropRequirements::for_frame(&passthrough_frame).layer(BuiltLayerId(0)));
//         assert!(!BackdropRequirements::for_frame(&isolated_frame).layer(BuiltLayerId(0)));

//         let passthrough = render_frame(&passthrough_frame);
//         let isolated = render_frame(&isolated_frame);
//         let passthrough = pixel(&passthrough, 10, 5, 5);
//         let isolated = pixel(&isolated, 10, 5, 5);
//         assert!(
//             passthrough[1] > 180 && passthrough[2] > 180 && passthrough[0] < 80,
//             "passthrough={passthrough:?}"
//         );
//         assert!(
//             isolated[0] > 180 && isolated[1] < 80 && isolated[2] < 80,
//             "isolated={isolated:?}"
//         );
//     }

//     #[test]
//     fn refraction_and_chromatic_aberration_execute_on_cpu_raster() {
//         let descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([
//                     ComputedBackdropFilter::Refraction {
//                         strength: 2.0,
//                         chromatic_aberration: 1.0,
//                     },
//                     ComputedBackdropFilter::ChromaticAberration { offset: [1.0, 0.0] },
//                 ]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![
//                 shape_draw(Rect::new(0.0, 0.0, 5.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
//                 shape_draw(Rect::new(5.0, 0.0, 5.0, 10.0), Color::rgb(0.0, 1.0, 1.0)),
//             ],
//             Vec::new(),
//         );
//         let pixels = render_frame(&frame);
//         let center = pixel(&pixels, 10, 5, 5);
//         assert!(center[3] > 200, "center={center:?}");
//         assert!(
//             center[0] != center[1] || center[1] != center[2],
//             "center={center:?}"
//         );
//     }

//     #[test]
//     fn artistic_blend_can_be_combined_with_src_operator() {
//         let descriptor = LayerDescriptor {
//             composite: CompositeStyle {
//                 blend_mode: BlendMode::Multiply,
//                 operator: CompositeOperator::Src,
//                 ..CompositeStyle::default()
//             },
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(0.0, 0.0, 1.0),
//             )],
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(1.0, 0.0, 0.0),
//             )],
//         );
//         let pixels = render_frame(&frame);
//         let center = pixel(&pixels, 10, 5, 5);
//         assert!(
//             center[0] < 40 && center[1] < 40 && center[2] < 40 && center[3] > 240,
//             "center={center:?}"
//         );
//     }

//     #[test]
//     fn missing_keyed_backdrop_mask_is_structured_error() {
//         let key = ImageKey::UserProvided(404);
//         let descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 mask: ComputedBackdropMask::AlphaTexture {
//                     texture: key.clone(),
//                     transform: Affine::scale(10.0, 10.0),
//                 },
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![shape_draw(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE)],
//             Vec::new(),
//         );
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         assert!(matches!(
//             backend.submit(&frame, &mut text),
//             Err(SkiaBackendError::MissingMaskImage(value)) if value == key
//         ));
//         assert!(!<TestBackend as RenderBackend<TextHost<CosmicEngine>>>::did_present(&backend));
//     }
// }
