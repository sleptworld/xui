//! The per-frame walk: one analysis pass over every item, then the recursive
//! layer and item drawing that follows it.

use skia_safe::{Canvas, Surface};
use xui::render::{
    BackdropIsolation, BuiltClipChainId, BuiltDraw, BuiltFrame, BuiltItem, BuiltLayerId,
};
use xui::text::TextHost;
use xui_interface::{Affine, Bounds, Color, ImageTransform, TextBackend};
use xui_render_graph::{ExternalResourceKind, LayerProgramEntry};

use super::{
    SkiaBackend,
    convert::{inverse_affine, sk_matrix},
    image::{CachedSourceImage, RasterImage, image_bytes, make_image},
    paint::sk_color,
    surface::{configure_canvas, damage_region},
    vector::{apply_clip_shape, draw_shape, draw_vector},
};
use crate::{
    SkiaBackendError,
    cache::{Acquired, SurfaceLease},
    damage::DamageRegion,
};

/// What a single walk over the frame's items tells the drawing pass.
///
/// This replaces three separate walks over every item of every layer — one to
/// validate backdrop prefixes, one to upload image data, and one to work out
/// which layers contain something that reads the destination.
struct FrameAnalysis {
    /// Per layer: does anything inside it read the destination, either directly
    /// or through a passthrough descendant?
    needs_backdrop: Vec<bool>,
}

impl FrameAnalysis {
    fn layer(&self, id: BuiltLayerId) -> bool {
        self.needs_backdrop.get(id.0).copied().unwrap_or(false)
    }
}

impl<T: TextBackend> SkiaBackend<T> {
    /// Walks every item once, uploading image data and working out which
    /// layers need a materialized backdrop.
    ///
    /// `draw_layer` re-checks each backdrop instance's destination prefix as it
    /// reaches it, and more strictly than a pre-pass can — it also knows the
    /// prefix has to name the item position it is standing on — so there is no
    /// separate validation walk. A frame that fails mid-draw leaves the target
    /// partly written, which `submit` already handles by dropping the damage
    /// tracker and repainting in full.
    fn analyze_frame(&mut self, frame: &BuiltFrame) -> Result<FrameAnalysis, SkiaBackendError> {
        let count = frame.layers.len();
        let mut needs_backdrop = vec![false; count];
        // Passthrough parent -> child edges, along which the need propagates.
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (index, layer) in frame.layers.iter().enumerate() {
            for item in &layer.items {
                match item {
                    BuiltItem::Draw(BuiltDraw::Image(draw)) => {
                        self.prepare_image(&draw.primitive)?;
                    }
                    BuiltItem::Draw(_) => {}
                    BuiltItem::Layer(instance_id) => {
                        let Some(instance) = frame.layer_instance(*instance_id) else {
                            continue;
                        };
                        if instance
                            .render_program
                            .program()
                            .external_resource(ExternalResourceKind::Backdrop)
                            .is_some()
                        {
                            needs_backdrop[index] = true;
                        }
                        if frame.layers.get(instance.layer.0).is_some_and(|child| {
                            child.backdrop_isolation == BackdropIsolation::Passthrough
                        }) {
                            edges.push((index, instance.layer.0));
                        }
                    }
                }
            }
        }
        // A child layer is always pushed after its parent, so one reverse sweep
        // settles the common case; the loop covers any order and terminates
        // because each round either sets a flag or stops.
        let mut changed = !edges.is_empty();
        while changed {
            changed = false;
            for &(parent, child) in edges.iter().rev() {
                if needs_backdrop[child] && !needs_backdrop[parent] {
                    needs_backdrop[parent] = true;
                    changed = true;
                }
            }
        }
        Ok(FrameAnalysis { needs_backdrop })
    }

    /// Uploads a primitive's pixels if the cache does not already hold them.
    pub(super) fn prepare_image(
        &mut self,
        primitive: &xui::render::ImagePrimitive,
    ) -> Result<(), SkiaBackendError> {
        if primitive.data.size.width == 0 || primitive.data.size.height == 0 {
            return Ok(());
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
        Ok(())
    }

    pub(super) fn draw_frame(
        &mut self,
        surface: &mut Surface,
        frame: &BuiltFrame,
        damage: &DamageRegion,
        text: &mut TextHost<T>,
    ) -> Result<(), SkiaBackendError> {
        let analysis = self.analyze_frame(frame)?;
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
                &analysis,
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
        analysis: &FrameAnalysis,
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
            damage,
            text,
            analysis,
        )?;
        target.canvas().restore_to_count(save);
        Ok(())
    }

    /// Draws one layer's items into `target`.
    ///
    /// `damage` is in `target_bounds` coordinates and is what the canvas clip
    /// was built from. Items are tested against it up front, so an item outside
    /// the repaint costs one bounds intersection rather than a save, a clip
    /// chain rebuild, a paint and a draw call Skia then rejects. Skia would
    /// reject those draws too, but only after all of that has been paid for.
    #[allow(clippy::too_many_arguments)]
    fn draw_layer(
        &mut self,
        target: &mut Surface,
        target_bounds: Bounds,
        frame: &BuiltFrame,
        layer_id: BuiltLayerId,
        inherited_backdrop: Option<&RasterImage>,
        damage: &DamageRegion,
        text: &mut TextHost<T>,
        analysis: &FrameAnalysis,
    ) -> Result<(), SkiaBackendError> {
        let layer = frame.layers.get(layer_id.0).ok_or_else(|| {
            SkiaBackendError::InvalidFrame(format!("missing layer {}", layer_id.0))
        })?;
        self.frame_stats.layer_draws += 1;
        let cull = self.options.optimizations.cull;
        for (item_index, item) in layer.items.iter().enumerate() {
            match item {
                BuiltItem::Draw(draw) => {
                    let common = draw.common();
                    if cull && !damage.intersects(common.world_bounds) {
                        self.frame_stats.items_culled += 1;
                        continue;
                    }
                    self.frame_stats.primitive_draws += 1;
                    let canvas = target.canvas();
                    let save = canvas.save();
                    configure_canvas(canvas, target_bounds, self.scale_factor);
                    self.apply_clip_chain(canvas, frame, common.clip_chain, Affine::IDENTITY)?;
                    let transform = common.world_transform;
                    match draw {
                        BuiltDraw::Shape(value) => draw_shape(
                            canvas,
                            &mut self.gradients,
                            &value.primitive,
                            transform,
                            1.0,
                        ),
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
                    let program = instance.render_program.program();
                    let program_needs_backdrop = program
                        .external_resource(ExternalResourceKind::Backdrop)
                        .is_some();
                    if program_needs_backdrop {
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
                    let child_needs_backdrop = child_layer.backdrop_isolation
                        == BackdropIsolation::Passthrough
                        && analysis.layer(instance.layer);
                    // Anything that reads the destination is left alone: it is
                    // rare, and its cost is dominated by the backdrop snapshot
                    // rather than by the traversal a cull would save.
                    let plain = !program_needs_backdrop && !child_needs_backdrop;
                    // A blur or shadow paints outside the instance bounds, so
                    // the visible footprint is what decides visibility.
                    let visible_bounds = program
                        .layer_visual_expansion()
                        .apply_to_bounds(instance.world_bounds);
                    if cull && plain && !damage.intersects(visible_bounds) {
                        self.frame_stats.layer_instances_culled += 1;
                        continue;
                    }
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

                    let mut lease = match self.layer_cache.acquire(child_layer, self.scale_factor) {
                        Acquired::Hit(lease) => lease,
                        Acquired::Miss {
                            cache_id,
                            width,
                            height,
                        } => SurfaceLease {
                            surface: self.new_surface_px(width, height)?,
                            reused: false,
                            cache_id,
                        },
                    };
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
                            analysis,
                        )?;
                        if lease.cache_id.is_some() {
                            self.layer_cache.record_update(
                                lease.reused && child_damage.bounds() != Some(child_bounds),
                            );
                        }
                    }
                    let child_image = self.snapshot_target(&mut lease.surface, child_bounds);
                    let spent = self
                        .layer_cache
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
                    // After the composite, so nothing is still reading the
                    // snapshot when the surface goes back in the pool.
                    drop(child_image);
                    if let Some(surface) = spent {
                        self.recycle_surface(surface);
                    }
                }
            }
        }
        Ok(())
    }

    /// Applies a clip chain outermost-first.
    ///
    /// The chain is stored child-to-parent, so this recurses into the parent
    /// before clipping itself. Depth is the nesting depth of clipping scene
    /// nodes, which is small; the old iterative form allocated a `Vec` on every
    /// call, and this runs once per drawn item.
    pub(super) fn apply_clip_chain(
        &self,
        canvas: &Canvas,
        frame: &BuiltFrame,
        clip: Option<BuiltClipChainId>,
        placement: Affine,
    ) -> Result<(), SkiaBackendError> {
        let Some(id) = clip else {
            return Ok(());
        };
        let value = frame.clip_chains.get(id.0).ok_or_else(|| {
            SkiaBackendError::InvalidFrame(format!("missing clip chain {}", id.0))
        })?;
        self.apply_clip_chain(canvas, frame, value.parent, placement)?;
        let matrix = sk_matrix(value.world_transform.then(placement));
        apply_clip_shape(canvas, &value.clip, &matrix);
        Ok(())
    }
}
