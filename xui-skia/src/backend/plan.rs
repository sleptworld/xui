//! Executing a layer's render plan: filter and composite passes, backdrop
//! sampling, and the masks both of them can carry.

use skia_safe::{BlendMode as SkBlendMode, ClipOp, ImageFilter, Paint, Surface, TileMode};
use xui::render::render_graph::ImageResource;
use xui::render::{BuiltFrame, BuiltLayerInstance};
use xui_interface::{Affine, Bounds, Color, ImageTransform, TextBackend};
use xui_render_graph::{
    BlendMode, CompositeOperator, ExternalAliasing, ExternalResourceKind, LayerPlanContext,
    LayerProgramEntry, LayerRenderPlan, MaskShape, Pass, PassOp, PlanLimits, PlanMask,
    PlanResourceId, PlanResourceKind, TextureClass,
};

use super::{
    SkiaBackend,
    convert::{sk_bounds, sk_matrix},
    effects::{CHROMATIC_ABERRATION_SKSL, PIXELATE_SKSL, REFRACTION_SKSL},
    image::{
        CachedSourceImage, RasterImage, draw_image_logical, draw_raster_image, image_bytes,
        make_image,
    },
    paint::{
        color_matrix_filter, composite_blend_mode, extract_alpha_filter, shadow_color_filter,
        sk_blend_mode, solid_paint,
    },
    surface::{clear_surface_output, configure_canvas},
    vector::draw_mask_shape,
};
use crate::SkiaBackendError;

impl<T: TextBackend> SkiaBackend<T> {
    pub(super) fn execute_backdrop_only(
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
    pub(super) fn execute_instance(
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
        // Drop the snapshots before the surfaces they were taken from, so the
        // pool hands back surfaces nothing is reading.
        drop(values);
        for surface in transient_surfaces {
            self.recycle_surface(surface);
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

    pub(super) fn composite_images(
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

    pub(super) fn transform_image(
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
