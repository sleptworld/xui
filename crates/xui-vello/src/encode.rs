//! `BuiltFrame` -> what this crate draws: 2D content as `vello::Scene`s, and
//! the layer tree that composes them.
//!
//! Vello draws; it composes nothing. A target is a [`Surface`]: an ordered list
//! of 2D bands, textures and isolated child layers. Each child layer is a
//! surface of its own plus the render-graph plan that filters it and composites
//! it into its parent (backdrop, effects, opacity, blend, transform, clip), run
//! by `filter::graph`. Every layer the frame isolates is: which layers need it
//! is decided once, by `xui-core`, which already draws in place every layer
//! that would compose as its content does.
//!
//! Clips stay Vello's inside a band, and a band keeps them open from one draw
//! to the next: each draw pops only the clips it is not in and pushes only the
//! ones it lacks, so a list of rows inside a scroll view's clips pushes those
//! clips once, not once a row. Around a cut (a texture, an isolated layer) a
//! band is closed, so a clip that spans one is taken apart there: a band
//! closes the clips it opened and the next one opens them again, and the cut
//! itself is clipped by the compositor, per pixel: rectangles and rounded
//! or rotated ones in its shader ([`ClipSpec`]), paths and the overflow
//! through a coverage mask Vello draws ([`ClipMask`]).

use std::sync::{Arc, OnceLock};

use vello::{
    Scene,
    kurbo::{
        Affine as KurboAffine, Cap, Circle, Ellipse, Join, Line, Rect as KurboRect,
        RoundedRect, Shape as _, Stroke,
    },
    peniko::{
        Brush, Fill, Gradient, InterpolationAlphaSpace,
        color::{AlphaColor, Srgb},
    },
};
use xui_core::render::{
    BuiltClipChainId, BuiltDraw, BuiltFrame, BuiltItem, BuiltLayerId, BuiltLayerInstance,
    ClipShape, ShapePrimitive, VectorPrimitive,
};
use xui_core::render::render_graph::ImageResource;
use xui_core::text::TextHost;
use xui_interface::{ImageData, TextBackend};
use xui_wgpu_text::{TextBatch, TextRenderer, TextTarget};
use xui_render_graph::{
    CompositeInstance, ExternalAliasing, ExternalResourceKind, LayerPlanContext,
    LayerProgramEntry, LayerRenderPlan, PlanLimits, TextureClass,
};

use rustc_hash::FxHashSet;
use xui_core::render::{
    BackdropIsolation, BuiltLayer, CachePolicy, LayerCacheId, damage::DamageTracker,
};
use xui_render_graph::PixelRect;

use crate::{
    filter::{ClipSpec, TextureDraw},
    layer_cache::{CacheUpdate, CacheUse, FilterUse, LayerCache},
    image::{ImageCache, encode_image},
    text::{
        FontCache, MAX_BITMAP_PPEM, bitmap_parts, encode_selection, encode_text, has_selection,
        paragraph,
    },
    paths::PathCache,
    texture::ExternalTextures,
};
use xui_interface::{
    Affine, Bounds, Color, ComputedColorStyle, ComputedShadowStyle, FillRule, LineCap, LineJoin,
    Shape, VectorCommand,
};

/// Things the frame asked for that this backend does not draw as asked, so
/// the caller can report each once instead of every frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported {
    /// A run's font could not be loaded, so its glyphs were skipped.
    pub missing_font: bool,
    /// A layer's render plan could not be made (a texture past the device's
    /// limit, a transform that overflows). The layer is not drawn.
    pub layer_effect: bool,
    /// A mask whose image is not available as pixel data (it is a key into an
    /// asset cache this backend cannot see). Drawn unmasked.
    pub layer_mask: bool,
}

impl Unsupported {
    pub fn merge(&mut self, other: Self) {
        self.missing_font |= other.missing_font;
        self.layer_effect |= other.layer_effect;
        self.layer_mask |= other.layer_mask;
    }
}

/// One target: what is drawn into it, in paint order, composed from
/// transparent (or the frame's clear colour) by this crate.
pub(crate) struct Surface {
    /// Where the target's texel (0, 0) sits, in physical pixels of the space
    /// it is drawn in: its layer's local space, or the frame's.
    pub origin_px: (i32, i32),
    pub extent: (u32, u32),
    pub ops: Vec<SurfaceOp>,
}

pub(crate) enum SurfaceOp {
    /// 2D content on transparent, drawn over what is composed so far.
    Band(Scene),
    /// Glyphs, and the rectangles in front of them, drawn by `xui-wgpu-text`
    /// over what is composed so far.
    Text(TextBatch),
    /// A texture the application rendered, drawn over what is composed so far.
    Texture {
        draw: TextureDraw,
        clip_mask: Option<ClipMask>,
    },
    /// An isolated child layer, composed into what is composed so far.
    Layer(Box<LayerJob>),
}

/// A child layer: its content, and the plan that takes it into its parent.
///
/// The plan's parent destination is the parent surface as composed so far,
/// which is also its backdrop. Its layer content is `content`, rendered first;
/// for a cached layer, `content` is what to draw into the cached texture (all
/// of it, the damaged part, or nothing), which the plan then reads.
pub(crate) struct LayerJob {
    pub plan: LayerRenderPlan,
    pub content: Surface,
    pub cache: Option<CacheUse>,
    /// For a cached layer whose filtered result is recomputed this frame, all
    /// of it or where it changed: the plan's layer content is then that
    /// result, not `content`.
    pub filter: Option<FilterJob>,
    pub backdrop_mask: Option<ImageData>,
    /// By ordinal.
    pub layer_masks: Vec<ImageData>,
    /// Clips on the composite, per pixel, in the parent surface's pixels.
    pub clips: Vec<ClipSpec>,
    pub clip_mask: Option<ClipMask>,
    /// For a layer that lets what is beneath it through to a backdrop inside
    /// it: how to make what its content's backdrops see beneath it.
    pub inherit: Option<Inherit>,
}

/// What a backdrop inside a layer sees beneath the layer, made from the
/// backdrop the layer itself sees (in its parent's pixels): the layer's own
/// backdrop branch over it, if it has one, then that taken into the layer's
/// space, over the layer's content surface.
pub(crate) struct Inherit {
    /// The layer's backdrop branch, run onto a copy of the backdrop it sees,
    /// reading that backdrop.
    pub backdrop_only: Option<LayerRenderPlan>,
    /// The result placed by the inverse of the layer's composite transform,
    /// into the content surface's pixels.
    pub into_layer: LayerRenderPlan,
}

/// A cached layer's layer branch, run on its own over `region` of its
/// filtered result (physical pixels of the layer), onto transparent.
pub(crate) struct FilterJob {
    pub plan: LayerRenderPlan,
    pub region: PixelRect,
}

/// The coverage of the clips on a cut that the compositor does not evaluate
/// itself (a path, or more than it has slots for), drawn by Vello: opaque
/// inside every one of them. It lies over the parent surface from
/// `origin_px`, in that surface's pixels, pixel for pixel.
pub(crate) struct ClipMask {
    pub scene: Scene,
    pub origin_px: (i32, i32),
    pub extent: (u32, u32),
}

/// Encodes `region` of `frame` (physical pixels) as a [`Surface`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_frame<T: TextBackend>(
    frame: &BuiltFrame,
    scale_factor: f32,
    region: [i32; 4],
    max_texture_dimension: u32,
    cache: &LayerCache,
    damage: &DamageTracker,
    text: &TextHost<T>,
    fonts: &mut FontCache<T::FontId>,
    images: &ImageCache,
    textures: &mut ExternalTextures,
    paths: &PathCache,
    bitmap: Option<BitmapText<'_, T>>,
) -> (Surface, Unsupported) {
    let mut encoder = Encoder {
        frame,
        reads_beneath: reads_beneath(frame),
        cache,
        damage,
        claimed: FxHashSet::default(),
        text,
        fonts,
        images,
        textures,
        paths,
        bitmap,
        scale: scale_factor,
        limits: PlanLimits {
            max_texture_dimension_2d: max_texture_dimension,
        },
        unsupported: Unsupported::default(),
    };
    let mut target = Builder::new(
        (region[0], region[1]),
        ((region[2] - region[0]) as u32, (region[3] - region[1]) as u32),
        scale_factor,
    );
    encoder.layer(&mut target, frame.root_layer);
    (target.finish(), encoder.unsupported)
}

/// What draws text from glyph bitmaps, when the text backend can render them.
pub(crate) struct BitmapText<'a, T: TextBackend> {
    pub renderer: &'a mut TextRenderer<T::FontId>,
    pub rasterizer: &'a T::Rasterizer,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

struct Encoder<'a, T: TextBackend> {
    frame: &'a BuiltFrame,
    /// By layer: whether a backdrop in it, or in a layer inside it that lets
    /// what is beneath through, reads what is beneath it.
    reads_beneath: Vec<bool>,
    cache: &'a LayerCache,
    damage: &'a DamageTracker,
    /// Cache entries a job of this frame uses: one layer each.
    claimed: FxHashSet<LayerCacheId>,
    text: &'a TextHost<T>,
    fonts: &'a mut FontCache<T::FontId>,
    images: &'a ImageCache,
    textures: &'a mut ExternalTextures,
    paths: &'a PathCache,
    bitmap: Option<BitmapText<'a, T>>,
    scale: f32,
    limits: PlanLimits,
    unsupported: Unsupported,
}

/// A [`Surface`] being encoded: one layer's content, or the frame's.
struct Builder {
    origin_px: (i32, i32),
    extent: (u32, u32),
    /// The layer's logical space to the target's pixels.
    base: KurboAffine,
    ops: Vec<SurfaceOp>,
    /// The band being drawn.
    scene: Scene,
    /// Whether anything is drawn in `scene`.
    drawn: bool,
    /// The clips pushed into `scene` now, outermost first. A draw in some
    /// of the same keeps them open.
    open: Vec<BuiltClipChainId>,
    /// What a draw is clipped by; kept to spare an allocation a draw.
    wanted: Vec<BuiltClipChainId>,
    /// Text drawn after the band, not in it: nothing drawn in the band since
    /// overlaps it, so it can wait for the band to close, which spares a band
    /// a text. In paint order.
    pending: TextBatch,
    /// Where `pending` draws, in target pixels: a band draw that touches one
    /// of these closes the band first.
    pending_bounds: Vec<KurboRect>,
}

impl Builder {
    fn new(origin_px: (i32, i32), extent: (u32, u32), scale: f32) -> Self {
        Self {
            origin_px,
            extent,
            base: KurboAffine::translate((-origin_px.0 as f64, -origin_px.1 as f64))
                * KurboAffine::scale(scale as f64),
            ops: Vec::new(),
            scene: Scene::new(),
            drawn: false,
            open: Vec::new(),
            wanted: Vec::new(),
            pending: TextBatch::default(),
            pending_bounds: Vec::new(),
        }
    }

    fn viewport(&self) -> KurboRect {
        KurboRect::new(0.0, 0.0, self.extent.0 as f64, self.extent.1 as f64)
    }

    /// What the target covers, in its logical space.
    fn logical_bounds(&self, scale: f32) -> Bounds {
        let to_logical = |value: i32| value as f32 / scale;
        Bounds::new(
            xui_interface::Point::new(to_logical(self.origin_px.0), to_logical(self.origin_px.1)),
            xui_interface::Point::new(
                to_logical(self.origin_px.0 + self.extent.0 as i32),
                to_logical(self.origin_px.1 + self.extent.1 as i32),
            ),
        )
    }

    /// Target pixels back to its logical space.
    fn px_to_logical(&self, rect: KurboRect, scale: f32) -> Bounds {
        let x = |value: f64| ((value + self.origin_px.0 as f64) / scale as f64) as f32;
        let y = |value: f64| ((value + self.origin_px.1 as f64) / scale as f64) as f32;
        Bounds::new(
            xui_interface::Point::new(x(rect.x0), y(rect.y0)),
            xui_interface::Point::new(x(rect.x1), y(rect.y1)),
        )
    }

    /// Whether `bounds`, in the layer's space, touches the target. Everything
    /// outside it is left unencoded, which is what makes repainting a small
    /// region cheap.
    fn overlaps(&self, bounds: Bounds) -> bool {
        let rect = self.base.transform_rect_bbox(kurbo_bounds(bounds));
        let viewport = self.viewport();
        rect.x0 < viewport.x1 && rect.x1 > viewport.x0 && rect.y0 < viewport.y1 && rect.y1 > viewport.y0
    }

    /// Closes the band: what is drawn in it so far is one op, and the text
    /// waiting on it the next.
    fn cut(&mut self) {
        self.close_clips(0);
        let scene = std::mem::replace(&mut self.scene, Scene::new());
        if std::mem::take(&mut self.drawn) {
            self.ops.push(SurfaceOp::Band(scene));
        }
        self.pending_bounds.clear();
        let pending = std::mem::take(&mut self.pending);
        if !pending.is_empty() {
            self.ops.push(SurfaceOp::Text(pending));
        }
    }

    /// Text drawn over everything so far, in `bounds` (target pixels).
    fn defer(&mut self, batch: TextBatch, bounds: KurboRect) {
        if batch.is_empty() {
            return;
        }
        self.pending.append(batch);
        self.pending_bounds.push(bounds);
    }

    /// Readies the band for a draw over `rect` (target pixels): text waiting
    /// on the band that the draw would cover is drawn first.
    fn clear_of(&mut self, rect: KurboRect) {
        if self
            .pending_bounds
            .iter()
            .any(|bounds| bounds.overlaps(rect))
        {
            self.cut();
        }
    }

    /// Pops the clips pushed into the band past the first `keep`.
    fn close_clips(&mut self, keep: usize) {
        while self.open.len() > keep {
            self.open.pop();
            self.scene.pop_layer();
        }
    }

    fn finish(mut self) -> Surface {
        self.cut();
        Surface {
            origin_px: self.origin_px,
            extent: self.extent,
            ops: self.ops,
        }
    }
}

/// The clips on a cut, as the compositor applies them.
struct ClipSet {
    /// An axis-aligned rectangle, in target pixels, outside which they keep
    /// nothing; `None` for the whole target.
    bounds: Option<KurboRect>,
    /// What the shader evaluates itself.
    specs: Vec<ClipSpec>,
    /// The coverage of the rest.
    mask: Option<ClipMask>,
}

/// The shader evaluates this many clips.
const MAX_CLIPS: usize = 4;

impl<T: TextBackend> Encoder<'_, T> {
    /// Encodes `layer`'s items into `target`, which is drawn in its space.
    fn layer(&mut self, target: &mut Builder, layer: BuiltLayerId) {
        let frame = self.frame;
        let base = target.base;
        for item in &frame.layers[layer.0].items {
            match item {
                BuiltItem::Draw(draw) => {
                    let common = draw.common();
                    if !target.overlaps(common.world_bounds) {
                        continue;
                    }
                    if let BuiltDraw::Vector(value) = draw
                        && value
                            .primitive
                            .scene
                            .commands()
                            .iter()
                            .any(|command| matches!(command, VectorCommand::Texture { .. }))
                    {
                        self.vector_around_textures(target, common, &value.primitive);
                        continue;
                    }
                    let world = base * kurbo_affine(common.world_transform);
                    if let BuiltDraw::Text(value) = draw
                        && self.bitmap_text(target, common, world, &value.primitive)
                    {
                        continue;
                    }
                    target.clear_of(base.transform_rect_bbox(kurbo_bounds(common.world_bounds)));
                    self.clip_to(target, common.clip_chain);
                    match draw {
                        BuiltDraw::Shape(value) => {
                            encode_shape(&mut target.scene, world, &value.primitive)
                        }
                        BuiltDraw::Vector(value) => encode_vector(
                            &mut target.scene,
                            world * kurbo_affine(value.primitive.transform),
                            value.primitive.scene.commands(),
                            self.paths,
                        ),
                        BuiltDraw::Text(value) => {
                            let gaps = encode_text(
                                &mut target.scene,
                                world,
                                &value.primitive,
                                self.text,
                                self.fonts,
                            );
                            self.unsupported.missing_font |= gaps.missing_font;
                        }
                        BuiltDraw::Image(value) => {
                            encode_image(&mut target.scene, world, &value.primitive, self.images)
                        }
                    }
                    target.drawn = true;
                }
                BuiltItem::Layer(id) => {
                    let instance = frame.layer_instance(*id).expect("built instance");
                    if target.overlaps(instance.world_bounds) {
                        self.isolated(target, instance);
                    }
                }
            }
        }
    }

    /// A layer of its own: the band is cut, the layer's content is encoded as
    /// a surface, and its plan composes it into the target.
    fn isolated(&mut self, target: &mut Builder, instance: &BuiltLayerInstance) {
        let Some(clips) = self.clip_set(target, instance.clip_chain) else {
            return;
        };
        let layer = &self.frame.layers[instance.layer.0];
        let program: &xui_render_graph::LayerProgram = instance.render_program.program();
        // A backdrop inside a passthrough layer sees what is beneath the
        // layer, which is not the layer's content and changes without it.
        let inherits = layer.backdrop_isolation == BackdropIsolation::Passthrough
            && self.reads_beneath[instance.layer.0];
        // Decided before planning: a layer whose filtered result is kept is
        // composited from it, by its program with the layer branch cut out.
        let mut cache = if inherits { None } else { self.cache_use(layer) };
        let filter = cache.as_mut().and_then(|(use_, _)| {
            let filter = self.filter_use(layer, program, use_)?;
            use_.filter = Some(filter.0);
            Some(filter)
        });
        let cut;
        let (composite_program, composite_content) = match &filter {
            Some((_, _, domain)) => {
                cut = program.without_layer_effects();
                (&cut, *domain)
            }
            None => (program, layer.content_bounds),
        };

        let destination = target.logical_bounds(self.scale);
        let composite_clip = clips.bounds.map(|rect| target.px_to_logical(rect, self.scale));
        let placed = instance.world_bounds;
        let context = LayerPlanContext {
            backdrop_source_bounds: destination,
            parent_destination_bounds: destination,
            composite_clip_bounds: composite_clip,
            layer_content_bounds: composite_content,
            backdrop_bounds: Some(match composite_clip {
                Some(clip) => (placed & clip).unwrap_or(Bounds::ZERO),
                None => placed,
            }),
            composite: CompositeInstance {
                opacity: instance.composite.opacity,
                transform: instance.composite.transform,
            },
            scale_factor: self.scale,
            color_texture_class: TextureClass::LINEAR_COLOR,
            external_aliasing: ExternalAliasing::BackdropAndDestination,
            limits: self.limits,
        };
        let plan = match composite_program.instantiate(&context) {
            Ok(plan) => plan,
            Err(error) => {
                log::debug!("xui-vello: a layer is not drawn: {error}");
                self.unsupported.layer_effect = true;
                return;
            }
        };
        if plan.is_noop() {
            return;
        }
        // The layer branch, over the part of its result to recompute, onto
        // transparent: whatever the blend and the operator, that is the result.
        let filter = match filter {
            Some((use_, region, _)) if use_.update != CacheUpdate::Keep => {
                let region_bounds =
                    Builder::new((region.x, region.y), (region.width, region.height), self.scale)
                        .logical_bounds(self.scale);
                let context = LayerPlanContext {
                    backdrop_source_bounds: region_bounds,
                    parent_destination_bounds: region_bounds,
                    composite_clip_bounds: None,
                    layer_content_bounds: layer.content_bounds,
                    backdrop_bounds: None,
                    composite: CompositeInstance::default(),
                    scale_factor: self.scale,
                    color_texture_class: TextureClass::LINEAR_COLOR,
                    external_aliasing: ExternalAliasing::Distinct,
                    limits: self.limits,
                };
                match program.instantiate_entry(LayerProgramEntry::LayerOnly, &context) {
                    Ok(plan) => Some(FilterJob { plan, region }),
                    Err(error) => {
                        log::debug!("xui-vello: a layer is not drawn: {error}");
                        self.unsupported.layer_effect = true;
                        return;
                    }
                }
            }
            _ => None,
        };

        let content = match cache {
            // A backdrop inside sees as far as the layer reaches, wherever the
            // layer shows, as in the Skia backend: the layer is drawn whole.
            None if self.reads_beneath[instance.layer.0] => {
                let region = self
                    .whole(layer.content_bounds)
                    .unwrap_or(plan.resources()[plan.layer_content().index()].physical_bounds);
                self.content(instance.layer, region)
            }
            // Otherwise only what the plan reads of the content is drawn: the
            // visible part, grown by what its filters reach.
            None => self.content(
                instance.layer,
                plan.resources()[plan.layer_content().index()].physical_bounds,
            ),
            Some((_, region)) => self.content(instance.layer, region),
        };

        let inherit = if inherits {
            self.inherit(&context, instance, destination, &content)
        } else {
            None
        };

        let mut mask = |kind| match instance.render_program.handle(kind) {
            Some(ImageResource::Data { data, .. }) => data.clone(),
            // A key into an asset cache, or nothing bound: drawn unmasked.
            _ => {
                self.unsupported.layer_mask = true;
                opaque_mask().clone()
            }
        };
        let backdrop_mask = program
            .external_resource(ExternalResourceKind::BackdropMask)
            .map(|_| mask(ExternalResourceKind::BackdropMask));
        let layer_masks = (0..)
            .map(ExternalResourceKind::LayerMask)
            .take_while(|kind| program.external_resource(*kind).is_some())
            .map(&mut mask)
            .collect();

        target.cut();
        target.ops.push(SurfaceOp::Layer(Box::new(LayerJob {
            plan,
            content,
            cache: cache.map(|(use_, _)| use_),
            filter,
            backdrop_mask,
            layer_masks,
            clips: clips.specs,
            clip_mask: clips.mask,
            inherit,
        })));
    }

    /// How a backdrop inside `instance` sees what is beneath it: what the
    /// layer's own backdrop sees (the target's pixels, composed so far over
    /// whatever they inherit), with the layer's backdrop branch over it, put
    /// in the layer's space over `content`. `None` when the layer cannot be
    /// placed (a singular transform), and nothing inside it shows.
    fn inherit(
        &mut self,
        context: &LayerPlanContext,
        instance: &BuiltLayerInstance,
        destination: Bounds,
        content: &Surface,
    ) -> Option<Inherit> {
        let program = instance.render_program.program();
        let distinct = LayerPlanContext {
            external_aliasing: ExternalAliasing::Distinct,
            ..*context
        };
        let backdrop_only = program.backdrop().and_then(|_| {
            program
                .instantiate_entry(LayerProgramEntry::BackdropOnly, &distinct)
                .map_err(|error| log::debug!("xui-vello: a backdrop is not inherited: {error}"))
                .ok()
        });
        let into_layer = LayerPlanContext {
            backdrop_source_bounds: destination,
            parent_destination_bounds: Builder::new(content.origin_px, content.extent, self.scale)
                .logical_bounds(self.scale),
            composite_clip_bounds: None,
            layer_content_bounds: destination,
            backdrop_bounds: None,
            composite: CompositeInstance {
                opacity: 1.0,
                transform: instance.composite.transform.invert()?,
            },
            ..distinct
        };
        let into_layer = placement_program()
            .instantiate(&into_layer)
            .map_err(|error| log::debug!("xui-vello: a backdrop is not inherited: {error}"))
            .ok()?;
        Some(Inherit {
            backdrop_only,
            into_layer,
        })
    }

    /// `bounds` in physical pixels, when a texture can hold them.
    fn whole(&self, bounds: Bounds) -> Option<PixelRect> {
        let rect = physical_rect(bounds, self.scale)?;
        let limit = self.limits.max_texture_dimension_2d;
        (rect.width > 0 && rect.height > 0 && rect.width <= limit && rect.height <= limit)
            .then_some(rect)
    }

    /// `layer`'s content drawn over `rect` of its physical pixels; nothing for
    /// an empty one.
    fn content(&mut self, layer: BuiltLayerId, rect: PixelRect) -> Surface {
        let mut child = Builder::new(
            (rect.x, rect.y),
            (rect.width.max(1), rect.height.max(1)),
            self.scale,
        );
        if rect.width > 0 && rect.height > 0 {
            self.layer(&mut child, layer);
        }
        child.finish()
    }

    /// Whether `layer` is cached, and if so what of its content to draw: all
    /// of it when nothing fits in the cache, the part the damage tracker says
    /// changed when something does, or nothing (an empty rect).
    ///
    /// The cached texture is the whole of the content bounds, so whatever a
    /// later frame's plan reads of the content is in it.
    fn cache_use(&mut self, layer: &BuiltLayer) -> Option<(CacheUse, PixelRect)> {
        if layer.cache_policy == CachePolicy::None {
            return None;
        }
        let id = layer.cache_id?;
        let rect = physical_rect(layer.content_bounds, self.scale)?;
        let limit = self.limits.max_texture_dimension_2d;
        if rect.width == 0 || rect.height == 0 || rect.width > limit || rect.height > limit {
            return None;
        }
        // A key used twice in a frame is cached for the first layer only.
        if !self.claimed.insert(id) {
            return None;
        }
        let scale_bits = self.scale.to_bits();
        let entry = self.cache.get(id).filter(|entry| {
            entry.source == layer.source && entry.rect == rect && entry.scale_bits == scale_bits
        });
        let keep = PixelRect {
            x: rect.x,
            y: rect.y,
            width: 0,
            height: 0,
        };
        let (update, region) = if entry.is_none() {
            (CacheUpdate::Fresh, rect)
        } else {
            let damaged = self
                .damage
                .layer(layer.source)
                .bounds()
                .and_then(|bounds| physical_rect(bounds, self.scale))
                .and_then(|damaged| intersect_px(damaged, rect));
            match damaged {
                Some(damaged) => (CacheUpdate::Patch, damaged),
                None => (CacheUpdate::Keep, keep),
            }
        };
        Some((
            CacheUse {
                id,
                source: layer.source,
                policy: layer.cache_policy,
                rect,
                scale_bits,
                update,
                filter: None,
            },
            region,
        ))
    }

    /// Whether a cached layer keeps its filtered result, and if so what of it
    /// to recompute (an empty rect for nothing), with its domain: the content
    /// bounds grown by the effects.
    ///
    /// The damage tracker grows a layer's damage by its effects' reach, which
    /// is where their result can have changed. A change to the effects
    /// themselves is a different program, and recomputes all of it.
    fn filter_use(
        &self,
        layer: &BuiltLayer,
        program: &xui_render_graph::LayerProgram,
        content: &CacheUse,
    ) -> Option<(FilterUse, PixelRect, Bounds)> {
        if !program.has_layer_effects() {
            return None;
        }
        let reach = program.layer_visual_expansion();
        let domain = reach.apply_to_bounds(layer.content_bounds);
        let rect = physical_rect(domain, self.scale)?;
        let limit = self.limits.max_texture_dimension_2d;
        if rect.width == 0 || rect.height == 0 || rect.width > limit || rect.height > limit {
            return None;
        }
        let fingerprint = program.fingerprint();
        let kept = self
            .cache
            .get(content.id)
            .and_then(|entry| entry.filtered.as_ref())
            .is_some_and(|filtered| filtered.rect == rect && filtered.program == fingerprint);
        let (update, region) = match content.update {
            CacheUpdate::Fresh => (CacheUpdate::Fresh, rect),
            _ if !kept => (CacheUpdate::Fresh, rect),
            CacheUpdate::Keep => (CacheUpdate::Keep, rect),
            CacheUpdate::Patch => {
                let damaged = self
                    .damage
                    .layer(layer.source)
                    .bounds()
                    .and_then(|bounds| physical_rect(bounds, self.scale))
                    .and_then(|damaged| intersect_px(damaged, rect));
                match damaged {
                    Some(damaged) => (CacheUpdate::Patch, damaged),
                    None => (CacheUpdate::Keep, rect),
                }
            }
        };
        Some((
            FilterUse {
                rect,
                program: fingerprint,
                update,
            },
            region,
            domain,
        ))
    }

    /// Leaves the band clipped as a draw in `chain` is: what it shares with
    /// the clips open now stays open, the rest of those is popped, and what
    /// it lacks is pushed.
    fn clip_to(&mut self, target: &mut Builder, chain: Option<BuiltClipChainId>) {
        let mut wanted = std::mem::take(&mut target.wanted);
        chain_nodes(self.frame, chain, &mut wanted);
        let shared = target
            .open
            .iter()
            .zip(&wanted)
            .take_while(|(open, wanted)| open == wanted)
            .count();
        target.close_clips(shared);
        for id in &wanted[shared..] {
            let node = &self.frame.clip_chains[id.0];
            push_clip_shape(
                &mut target.scene,
                self.paths,
                &node.clip,
                target.base * kurbo_affine(node.world_transform),
            );
            target.open.push(*id);
        }
        target.wanted = wanted;
    }

    /// The clips on a cut in `chain`. `None` when they leave nothing.
    ///
    /// Axis-aligned rectangles are intersected into one; rounded and rotated
    /// clips take the shader's other slots; a path, and whatever does not fit
    /// in the slots, is drawn by Vello into a mask. Every clip is exact.
    fn clip_set(&mut self, target: &Builder, chain: Option<BuiltClipChainId>) -> Option<ClipSet> {
        let mut ids = Vec::new();
        chain_nodes(self.frame, chain, &mut ids);
        let nodes = ids.iter().map(|id| &self.frame.clip_chains[id.0]);

        let mut rect: Option<KurboRect> = None;
        let mut shaped = Vec::new();
        let mut masked = Vec::new();
        for node in nodes {
            let to_px = target.base * kurbo_affine(node.world_transform);
            // A singular clip transform keeps nothing.
            let inverse = to_px.inverse_checked()?;
            let [_, b, c, _, _, _] = to_px.as_coeffs();
            match &node.clip {
                ClipShape::Rect(bounds) if b == 0.0 && c == 0.0 => {
                    let px = to_px.transform_rect_bbox(kurbo_bounds(*bounds));
                    rect = Some(rect.map_or(px, |rect| rect.intersect(px)));
                }
                ClipShape::Rect(bounds) | ClipShape::RoundedRect { rect: bounds, .. } => {
                    let radius = match &node.clip {
                        ClipShape::RoundedRect { radius, .. } => *radius,
                        _ => 0.0,
                    };
                    shaped.push((
                        ClipSpec {
                            inverse,
                            rect: [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y],
                            radius,
                        },
                        node,
                        to_px,
                    ));
                }
                ClipShape::Path { .. } => masked.push((node, to_px)),
            }
        }
        if rect.is_some_and(|rect| !(rect.width() > 0.0 && rect.height() > 0.0)) {
            return None;
        }
        let room = MAX_CLIPS - rect.is_some() as usize;
        if shaped.len() > room {
            masked.extend(shaped.drain(room..).map(|(_, node, to_px)| (node, to_px)));
        }
        let specs = rect
            .map(|rect| ClipSpec {
                inverse: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                rect: [rect.x0 as f32, rect.y0 as f32, rect.x1 as f32, rect.y1 as f32],
                radius: 0.0,
            })
            .into_iter()
            .chain(shaped.into_iter().map(|(spec, _, _)| spec))
            .collect();
        if masked.is_empty() {
            return Some(ClipSet {
                bounds: rect,
                specs,
                mask: None,
            });
        }

        // The mask covers only what the other clips leave of the target, in
        // whole pixels.
        let mut area = rect.map_or(target.viewport(), |rect| rect.intersect(target.viewport()));
        for (node, to_px) in &masked {
            area = area.intersect(to_px.transform_rect_bbox(kurbo_bounds(node.clip.local_bounds())));
        }
        let area = KurboRect::new(area.x0.floor(), area.y0.floor(), area.x1.ceil(), area.y1.ceil())
            .intersect(target.viewport());
        if !(area.width() > 0.0 && area.height() > 0.0) {
            return None;
        }
        let to_mask = KurboAffine::translate((-area.x0, -area.y0));
        let mut scene = Scene::new();
        for (node, to_px) in &masked {
            push_clip_shape(&mut scene, self.paths, &node.clip, to_mask * *to_px);
        }
        scene.fill(
            Fill::NonZero,
            KurboAffine::IDENTITY,
            color(Color::rgb(1.0, 1.0, 1.0), 1.0),
            None,
            &KurboRect::new(0.0, 0.0, area.width(), area.height()),
        );
        pop_layers(&mut scene, masked.len());
        Some(ClipSet {
            bounds: Some(rect.map_or(area, |rect| rect.intersect(area))),
            specs,
            mask: Some(ClipMask {
                scene,
                origin_px: (area.x0 as i32, area.y0 as i32),
                extent: (area.width() as u32, area.height() as u32),
            }),
        })
    }

    /// Draws a text primitive from glyph bitmaps, deferred past the band:
    /// `false` when it has to be Vello's, as outlines (no rasterizer, a
    /// transform other than a translation and a uniform scale, a clip other
    /// than rectangles, glyphs too large, a glyph atlas at its limit), and
    /// nothing was drawn.
    fn bitmap_text(
        &mut self,
        target: &mut Builder,
        common: &xui_core::render::BuiltDrawData,
        world: KurboAffine,
        primitive: &xui_core::render::TextPrimitive,
    ) -> bool {
        if self.bitmap.is_none() {
            return false;
        }
        let [scale, b, c, d, _, _] = world.as_coeffs();
        if b != 0.0 || c != 0.0 || scale <= 0.0 || (scale - d).abs() > scale * 1e-6 {
            return false;
        }
        let Some(paragraph) = paragraph(self.text, primitive) else {
            return true;
        };
        if paragraph
            .layout
            .runs
            .iter()
            .any(|run| run.font_size * scale as f32 > MAX_BITMAP_PPEM)
        {
            return false;
        }
        let Some(clips) = self.clip_set(target, common.clip_chain) else {
            return true;
        };
        if clips.mask.is_some() || clips.specs.len() > usize::from(clips.bounds.is_some()) {
            return false;
        }
        let bounds = world.transform_rect_bbox(kurbo_bounds(primitive.bounds));
        let clip = clips
            .bounds
            .map_or(bounds, |rect| rect.intersect(bounds))
            .intersect(target.viewport());
        if !(clip.width() > 0.0 && clip.height() > 0.0) {
            return true;
        }

        let parts = bitmap_parts(
            world,
            primitive,
            self.text,
            &paragraph,
            [clip.x0 as f32, clip.y0 as f32, clip.x1 as f32, clip.y1 as f32],
        );
        let bitmap = self.bitmap.as_mut().expect("checked above");
        let batch = bitmap.renderer.prepare(
            bitmap.device,
            bitmap.queue,
            bitmap.rasterizer,
            TextTarget {
                origin_px: (0, 0),
                extent: target.extent,
            },
            &parts.draws(),
        );
        // The glyph atlas is at its limit: rather than text with holes, all
        // of this text is Vello's.
        if batch.incomplete() {
            return false;
        }
        if has_selection(primitive) {
            target.clear_of(clip);
            self.clip_to(target, common.clip_chain);
            encode_selection(&mut target.scene, world, primitive, self.text, &paragraph);
            target.drawn = true;
        }
        target.defer(batch, clip);
        true
    }

    /// A canvas whose commands include textures: the 2D commands either side
    /// of each texture are drawn by Vello in their own bands, the texture by
    /// this crate between them.
    fn vector_around_textures(
        &mut self,
        target: &mut Builder,
        common: &xui_core::render::BuiltDrawData,
        vector: &VectorPrimitive,
    ) {
        let world =
            target.base * kurbo_affine(common.world_transform) * kurbo_affine(vector.transform);
        let commands = vector.scene.commands();
        let mut start = 0;
        for (index, command) in commands.iter().enumerate() {
            let VectorCommand::Texture {
                id,
                source,
                bounds,
                opacity,
                sampling,
                ..
            } = command
            else {
                continue;
            };
            self.vector_run(target, common, world, &commands[start..index]);
            start = index + 1;
            let Some(texture) = self.textures.resolve(*id, source) else {
                // Not registered yet: a normal state at startup, drawn as nothing.
                continue;
            };
            let Some(inverse) = world.inverse_checked() else {
                continue;
            };
            let Some(clips) = self.clip_set(target, common.clip_chain) else {
                continue;
            };
            target.cut();
            target.ops.push(SurfaceOp::Texture {
                draw: TextureDraw {
                    texture,
                    inverse,
                    bounds: [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y],
                    opacity: *opacity,
                    nearest: *sampling == xui_interface::Sampling::Nearest,
                    clips: clips.specs,
                },
                clip_mask: clips.mask,
            });
        }
        self.vector_run(target, common, world, &commands[start..]);
    }

    /// Commands that are all Vello's, clipped as the draw they belong to is.
    fn vector_run(
        &mut self,
        target: &mut Builder,
        common: &xui_core::render::BuiltDrawData,
        world: KurboAffine,
        commands: &[VectorCommand],
    ) {
        if commands.is_empty() {
            return;
        }
        target.clear_of(target.base.transform_rect_bbox(kurbo_bounds(common.world_bounds)));
        self.clip_to(target, common.clip_chain);
        encode_vector(&mut target.scene, world, commands, self.paths);
        target.drawn = true;
    }
}

/// Which layers' content reads what is beneath them: a layer holding a
/// backdrop, or holding a passthrough layer whose content does. The need goes
/// up every passthrough edge, as the Skia backend has it.
fn reads_beneath(frame: &BuiltFrame) -> Vec<bool> {
    let mut reads = vec![false; frame.layers.len()];
    let mut passthrough = Vec::new();
    for (parent, layer) in frame.layers.iter().enumerate() {
        for item in &layer.items {
            let BuiltItem::Layer(id) = item else { continue };
            let instance = frame.layer_instance(*id).expect("built instance");
            if instance.render_program.program().backdrop().is_some() {
                reads[parent] = true;
            }
            if frame.layers[instance.layer.0].backdrop_isolation == BackdropIsolation::Passthrough {
                passthrough.push((parent, instance.layer.0));
            }
        }
    }
    // A child comes after its parent, so one reverse sweep usually settles
    // it; the loop is for any order.
    let mut changed = true;
    while changed {
        changed = false;
        for &(parent, child) in passthrough.iter().rev() {
            if reads[child] && !reads[parent] {
                reads[parent] = true;
                changed = true;
            }
        }
    }
    reads
}

/// The program that only places a layer: no backdrop, no effects, a normal
/// composite. With an inverse transform it takes a parent's pixels into a
/// layer's space.
fn placement_program() -> &'static xui_render_graph::LayerProgram {
    static PROGRAM: OnceLock<xui_render_graph::LayerProgram> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        xui_render_graph::compile_layer(&xui_render_graph::LayerGraphDescriptor::default())
            .expect("the empty layer compiles")
    })
}

/// `bounds` in physical pixels, rounded outward as the render graph rounds.
fn physical_rect(bounds: Bounds, scale: f32) -> Option<PixelRect> {
    let x0 = (bounds.min.x * scale).floor();
    let y0 = (bounds.min.y * scale).floor();
    let x1 = (bounds.max.x * scale).ceil();
    let y1 = (bounds.max.y * scale).ceil();
    let fits = |value: f32| value.is_finite() && value.abs() < i32::MAX as f32;
    if ![x0, y0, x1, y1].into_iter().all(fits) || x1 < x0 || y1 < y0 {
        return None;
    }
    Some(PixelRect {
        x: x0 as i32,
        y: y0 as i32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

fn intersect_px(a: PixelRect, b: PixelRect) -> Option<PixelRect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width as i32).min(b.x + b.width as i32);
    let y1 = (a.y + a.height as i32).min(b.y + b.height as i32);
    (x1 > x0 && y1 > y0).then(|| PixelRect {
        x: x0,
        y: y0,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

/// What an unavailable mask is replaced by: nothing masked.
fn opaque_mask() -> &'static ImageData {
    static MASK: OnceLock<ImageData> = OnceLock::new();
    MASK.get_or_init(|| {
        ImageData::rgba8(xui_interface::Size::new(1, 1), Arc::<[u8]>::from([255u8; 4]))
    })
}

/// The nodes of `chain` into `nodes`, outermost first. The chain is stored
/// child-to-parent.
fn chain_nodes(frame: &BuiltFrame, chain: Option<BuiltClipChainId>, nodes: &mut Vec<BuiltClipChainId>) {
    nodes.clear();
    let mut next = chain;
    while let Some(id) = next {
        nodes.push(id);
        next = frame.clip_chains[id.0].parent;
    }
    nodes.reverse();
}

/// Clips to one shape, placed by `transform`.
fn push_clip_shape(scene: &mut Scene, paths: &PathCache, clip: &ClipShape, transform: KurboAffine) {
    match clip {
        ClipShape::Rect(rect) => scene.push_clip_layer(Fill::NonZero, transform, &kurbo_bounds(*rect)),
        ClipShape::RoundedRect { rect, radius } => scene.push_clip_layer(
            Fill::NonZero,
            transform,
            &RoundedRect::from_rect(kurbo_bounds(*rect), *radius as f64),
        ),
        ClipShape::Path { path, .. } => {
            scene.push_clip_layer(Fill::NonZero, transform, paths.get(path).as_ref())
        }
    }
}

fn pop_layers(scene: &mut Scene, layers: usize) {
    for _ in 0..layers {
        scene.pop_layer();
    }
}

pub(crate) fn blend_index(mode: xui_interface::BlendMode) -> u32 {
    use xui_interface::BlendMode as B;
    match mode {
        B::Normal => 0,
        B::Multiply => 1,
        B::Screen => 2,
        B::Overlay => 3,
        B::Darken => 4,
        B::Lighten => 5,
        B::ColorDodge => 6,
        B::ColorBurn => 7,
        B::HardLight => 8,
        B::SoftLight => 9,
        B::Difference => 10,
        B::Exclusion => 11,
        B::Hue => 12,
        B::Saturation => 13,
        B::Color => 14,
        B::Luminosity => 15,
    }
}

/// A drop shadow, drawn as one closed-form blurred rounded rectangle.
///
/// This is the technique Zed describes for GPUI and Raph Levien derived for
/// Vello: the Gaussian-blurred rounded box has an analytic expression (an `erf`
/// along each axis), so the shadow costs one primitive and no offscreen blur.
/// Vello ships it as `draw_blurred_rounded_rect`. As in the Skia backend,
/// `blur` is the Gaussian's standard deviation, the shadow is the shape grown
/// by `spread` and moved by `offset`, and it is not knocked out under the shape.
///
/// An ellipse has no closed form here, so it gets the stadium of the same
/// short side, which matches a circle exactly and drifts for flat ellipses.
fn encode_shadow(scene: &mut Scene, world: KurboAffine, shape: &ShapePrimitive, shadow: ComputedShadowStyle) {
    let rect = shape.bounds.expand(shadow.spread).translate(shadow.offset);
    let (rect, radius) = match shape.shape {
        Shape::Rect => (rect, 0.0),
        Shape::RoundedRect(radius) => (rect, radius),
        Shape::Circle | Shape::Ellipse => (rect, rect.width().min(rect.height()) * 0.5),
        Shape::Line { .. } => return,
    };
    let rect = if matches!(shape.shape, Shape::Circle) {
        let side = rect.width().min(rect.height());
        let center = ((rect.min.x + rect.max.x) * 0.5, (rect.min.y + rect.max.y) * 0.5);
        Bounds::new(
            xui_interface::Point::new(center.0 - side * 0.5, center.1 - side * 0.5),
            xui_interface::Point::new(center.0 + side * 0.5, center.1 + side * 0.5),
        )
    } else {
        rect
    };
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let radius = radius.clamp(0.0, rect.width().min(rect.height()) * 0.5) as f64;
    let ink = color(shadow.color, 1.0);
    if shadow.blur <= 0.0 {
        // No blur: a plain rounded rectangle (the closed form divides by sigma).
        scene.fill(
            Fill::NonZero,
            world,
            ink,
            None,
            &RoundedRect::from_rect(kurbo_bounds(rect), radius),
        );
    } else {
        // Vello's `std_dev` is the scale of its `erf`, not the Gaussian's
        // standard deviation: it blurs by `std_dev / sqrt(2)`. Measured against
        // Skia's mask blur, a shadow drawn with `std_dev = blur` is a factor of
        // sqrt(2) too tight.
        scene.draw_blurred_rounded_rect(
            world,
            kurbo_bounds(rect),
            ink,
            radius,
            shadow.blur as f64 * std::f64::consts::SQRT_2,
        );
    }
}

fn encode_shape(scene: &mut Scene, world: KurboAffine, shape: &ShapePrimitive) {
    if let Some(shadow) = shape.shadow.filter(|shadow| shadow.color.a > 0.0) {
        encode_shadow(scene, world, shape, shadow);
    }
    let bounds = kurbo_bounds(shape.bounds);
    let path = match shape.shape {
        Shape::Rect => bounds.to_path(0.1),
        Shape::RoundedRect(radius) => RoundedRect::from_rect(bounds, radius as f64).to_path(0.1),
        Shape::Circle => {
            let radius = bounds.width().min(bounds.height()) * 0.5;
            Circle::new(bounds.center(), radius).to_path(0.1)
        }
        Shape::Ellipse => Ellipse::new(
            bounds.center(),
            (bounds.width() * 0.5, bounds.height() * 0.5),
            0.0,
        )
        .to_path(0.1),
        Shape::Line { from, to } => {
            Line::new((from.x as f64, from.y as f64), (to.x as f64, to.y as f64)).to_path(0.1)
        }
    };
    if !matches!(shape.shape, Shape::Line { .. })
        && let Some(fill) = shape.fill
    {
        scene.fill(Fill::NonZero, world, &brush(fill, shape.bounds), None, &path);
    }
    if let Some(stroke) = shape.stroke
        && stroke.width > 0.0
    {
        // Dashes and dots (`StrokeLineStyle`) are not lowered yet.
        scene.stroke(
            &Stroke::new(stroke.width as f64),
            world,
            &brush(stroke.color, shape.bounds),
            None,
            &path,
        );
    }
}

/// Vello's share of a canvas: its paths. `world` already includes the
/// canvas's own transform.
fn encode_vector(
    scene: &mut Scene,
    world: KurboAffine,
    commands: &[VectorCommand],
    paths: &PathCache,
) {
    for command in commands {
        match command {
            VectorCommand::FillPath { path, transform, fill } => {
                let rule = match fill.rule {
                    FillRule::NonZero => Fill::NonZero,
                    FillRule::EvenOdd => Fill::EvenOdd,
                };
                scene.fill(rule, world * kurbo_affine(*transform), color(fill.color, 1.0), None, paths.get(path).as_ref());
            }
            VectorCommand::StrokePath { path, transform, stroke } if stroke.width > 0.0 => {
                let mut style = Stroke::new(stroke.width as f64)
                    .with_caps(match stroke.cap {
                        LineCap::Butt => Cap::Butt,
                        LineCap::Square => Cap::Square,
                        LineCap::Round => Cap::Round,
                    })
                    .with_join(match stroke.join {
                        LineJoin::Miter => Join::Miter,
                        LineJoin::Bevel => Join::Bevel,
                        LineJoin::Round => Join::Round,
                    });
                if let Some(dash) = stroke.effective_dash() {
                    style = style.with_dashes(dash.offset as f64, dash.intervals().iter().map(|v| *v as f64));
                }
                scene.stroke(&style, world * kurbo_affine(*transform), color(stroke.color, 1.0), None, paths.get(path).as_ref());
            }
            VectorCommand::StrokePath { .. } => {}
            VectorCommand::Shape { .. } => {}
            // Shapes and text are lowered to primitives of their own before a
            // vector run is formed, so a scene reaching here holds neither.
            VectorCommand::TextBox { .. } => {}
            // Not Vello's: the commands are split around them (see
            // `vector_around_textures`), so none reaches here.
            VectorCommand::Texture { .. } => {}
        }
    }
}

/// A fill or stroke colour as a Vello brush.
///
/// Gradient geometry is authored as fractions of the primitive's bounds (a
/// radial gradient's radius as a fraction of the short side), exactly as the
/// Skia backend reads it. Both have two stops and clamp beyond them.
/// Colours are interpolated without premultiplying, which is what Skia's
/// default does and differs from Vello's: a gradient to transparent keeps its
/// hue instead of fading through a darkened one.
fn brush(style: ComputedColorStyle, bounds: Bounds) -> Brush {
    let at = |p: xui_interface::Point| {
        (
            (bounds.x() + bounds.width() * p.x) as f64,
            (bounds.y() + bounds.height() * p.y) as f64,
        )
    };
    let stops = |from: Color, to: Color| [(0.0, color(from, 1.0)), (1.0, color(to, 1.0))];
    let mut gradient = match style {
        ComputedColorStyle::Solid(value) => return Brush::Solid(color(value, 1.0)),
        ComputedColorStyle::LinearGradient(value) => {
            Gradient::new_linear(at(value.start), at(value.end)).with_stops(stops(value.from, value.to))
        }
        ComputedColorStyle::RadialGradient(value) => {
            let radius = (value.radius * bounds.width().min(bounds.height())).max(0.001);
            Gradient::new_radial(at(value.center), radius).with_stops(stops(value.from, value.to))
        }
    };
    gradient.interpolation_alpha_space = InterpolationAlphaSpace::Unpremultiplied;
    Brush::Gradient(gradient)
}

fn color(value: Color, opacity: f32) -> AlphaColor<Srgb> {
    AlphaColor::new([value.r, value.g, value.b, value.a * opacity.clamp(0.0, 1.0)])
}

/// An affine's inverse as six numbers, when it has one.
trait InverseChecked {
    fn inverse_checked(&self) -> Option<[f32; 6]>;
}

impl InverseChecked for KurboAffine {
    fn inverse_checked(&self) -> Option<[f32; 6]> {
        let det = self.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let c = self.inverse().as_coeffs();
        Some([c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32])
    }
}

fn kurbo_bounds(bounds: Bounds) -> KurboRect {
    KurboRect::new(
        bounds.min.x as f64,
        bounds.min.y as f64,
        bounds.max.x as f64,
        bounds.max.y as f64,
    )
}

fn kurbo_affine(value: Affine) -> KurboAffine {
    KurboAffine::new([
        value.xx as f64,
        value.yx as f64,
        value.xy as f64,
        value.yy as f64,
        value.dx as f64,
        value.dy as f64,
    ])
}



#[cfg(test)]
mod tests {
    use super::*;
    use xui_core::render::{
        FrameBuilder, FrameProperties, LayerDescriptor, Primitive, RenderNodeId, RenderScene,
        SceneCompiler,
    };
    use xui_cosmic::CosmicEngine;
    use xui_interface::{ComputedColorStyle, Point, Size};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds {
        Bounds::from_origin_size(Point::new(x, y), Size::new(width, height))
    }

    /// Three clips, each around a layer that holds the next; the innermost
    /// layer holds `draws` squares. Not `isolated`, the layers are drawn in
    /// place by the compiler, and the clips are all the draws' own.
    fn nested(isolated: bool, draws: usize) -> BuiltFrame {
        let mut source = RenderScene::new();
        let mut parent: RenderNodeId = source.root();
        for depth in 0..3 {
            let inset = depth as f32 * 10.0;
            let clip = source.insert_clip(ClipShape::RoundedRect {
                rect: rect(inset, inset, 100.0 - 2.0 * inset, 100.0 - 2.0 * inset),
                radius: 4.0,
            });
            let layer = source.insert_layer(LayerDescriptor {
                force_offscreen: isolated,
                ..LayerDescriptor::default()
            });
            let content = source.insert_group();
            source.append_child(parent, clip).unwrap();
            source.set_child(clip, Some(layer)).unwrap();
            source.set_child(layer, Some(content)).unwrap();
            parent = content;
        }
        for index in 0..draws {
            let square = source.insert_primitive(Primitive::Shape(ShapePrimitive {
                bounds: rect(30.0 + index as f32 * 5.0, 30.0, 20.0, 20.0),
                shape: Shape::Rect,
                fill: Some(ComputedColorStyle::Solid(Color::BLACK)),
                stroke: None,
                shadow: None,
            }));
            source.append_child(parent, square).unwrap();
        }
        let mut compiler = SceneCompiler::new();
        let scene = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        FrameBuilder::new()
            .build(scene, rect(0.0, 0.0, 100.0, 100.0), &FrameProperties::default())
            .unwrap()
    }

    /// The clip layers pushed into each band.
    fn pushed(frame: &BuiltFrame) -> Vec<u32> {
        encode(frame)
            .0
            .ops
            .iter()
            .filter_map(|op| match op {
                SurfaceOp::Band(scene) => Some(scene.encoding().n_clips / 2),
                _ => None,
            })
            .collect()
    }

    fn encode(frame: &BuiltFrame) -> (Surface, Unsupported) {
        let text = TextHost::new(CosmicEngine::new(1.0));
        let mut fonts = FontCache::new(1 << 20);
        let images = ImageCache::new(1 << 20);
        let mut textures = ExternalTextures::default();
        let paths = PathCache::default();
        encode_frame(
            frame,
            1.0,
            [0, 0, 100, 100],
            8192,
            &LayerCache::default(),
            &DamageTracker::default(),
            &text,
            &mut fonts,
            &images,
            &mut textures,
            &paths,
            None,
        )
    }

    /// Three layers the frame isolates (asked to be offscreen, and otherwise
    /// doing nothing) are each composed on their own, every one cut by its
    /// clip in the compositor; the draws inside them push no clip.
    #[test]
    fn every_layer_the_frame_isolates_is_composed_on_its_own() {
        let frame = nested(true, 2);
        assert_eq!(frame.layer_instances.len(), 3);
        let (mut surface, _) = encode(&frame);
        for depth in 0..3 {
            let [SurfaceOp::Layer(job)] = surface.ops.as_slice() else {
                panic!("depth {depth}: not one layer");
            };
            assert_eq!(job.clips.len(), 1, "depth {depth}");
            assert!(job.clips[0].radius > 0.0, "the rounded clip");
            let job = match surface.ops.pop() {
                Some(SurfaceOp::Layer(job)) => job,
                _ => unreachable!(),
            };
            surface = job.content;
        }
        let [SurfaceOp::Band(scene)] = surface.ops.as_slice() else {
            panic!("the innermost layer is not one band");
        };
        assert_eq!(scene.encoding().n_clips, 0);
    }

    /// The same three clips in one layer: every draw is in all of them, and
    /// they are pushed once for all the draws, not once a draw.
    #[test]
    fn draws_in_the_same_clips_share_them() {
        for draws in [1, 2, 10] {
            let frame = nested(false, draws);
            assert!(frame.layer_instances.is_empty());
            assert_eq!(pushed(&frame), vec![3], "{draws} draws");
        }
    }

    /// A draw in A, one in A then B, one in A again: A is pushed once and kept,
    /// B is pushed for the second and popped for the third.
    #[test]
    fn a_draw_pushes_only_the_clips_it_lacks() {
        let mut source = RenderScene::new();
        let clip = |source: &mut RenderScene, parent, inset: f32| {
            let clip = source.insert_clip(ClipShape::RoundedRect {
                rect: rect(inset, inset, 100.0 - 2.0 * inset, 100.0 - 2.0 * inset),
                radius: 4.0,
            });
            let content = source.insert_group();
            source.append_child(parent, clip).unwrap();
            source.set_child(clip, Some(content)).unwrap();
            content
        };
        let square = |source: &mut RenderScene, parent, x: f32| {
            let square = source.insert_primitive(Primitive::Shape(ShapePrimitive {
                bounds: rect(x, 40.0, 10.0, 10.0),
                shape: Shape::Rect,
                fill: Some(ComputedColorStyle::Solid(Color::BLACK)),
                stroke: None,
                shadow: None,
            }));
            source.append_child(parent, square).unwrap();
        };
        let root = source.root();
        let a = clip(&mut source, root, 0.0);
        square(&mut source, a, 20.0);
        let b = clip(&mut source, a, 10.0);
        square(&mut source, b, 40.0);
        square(&mut source, a, 60.0);
        let mut compiler = SceneCompiler::new();
        let scene = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        let frame = FrameBuilder::new()
            .build(scene, rect(0.0, 0.0, 100.0, 100.0), &FrameProperties::default())
            .unwrap();
        assert_eq!(pushed(&frame), vec![2]);
    }
}
