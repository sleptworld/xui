//! Runs a lowered `xui-render-graph` plan: every pass of it, composites
//! included.
//!
//! The plan decides what is drawn, where, from what, and into which transient
//! slot; this only carries it out. Vello takes no part: it draws the 2D content
//! that arrives here as a texture (the layer's content, the parent it is
//! composited into), and everything done to those textures is a pass here.
//!
//! A plan's transients share slots by lifetime, so a chain of filters runs
//! between two textures, ping-pong. Each slot is one pooled texture for the
//! length of the plan; a resource sits in its top-left corner and the shader
//! is told its extent, so a slot larger than what it holds now reads nothing
//! stale.

use xui_interface::{Affine, ImageData};
use xui_render_graph::{
    AttachmentBlend, Axis, ExternalResourceKind, LayerRenderPlan, MaskShape, Pass, PassOp,
    PipelineKey, PlanMask, PlanResourceId, PlanResourceKind,
};

use super::{Blend, ClipSpec, FilterRenderer, Input, Params, Slot, Space, Target, matrix_rows};
use crate::VelloBackendError;

/// A pooled linear texture, and where its texel (0, 0) sits in physical
/// pixels of the coordinate space the plan gives the resource it stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placed {
    pub slot: Slot,
    pub origin: (i32, i32),
}

/// What a plan's external resources are bound to.
///
/// Every texture is linear and premultiplied. The destination is written in
/// place; it may also be the backdrop, when the plan was instantiated with
/// `ExternalAliasing::BackdropAndDestination`.
pub(crate) struct PlanTextures<'a> {
    pub destination: Placed,
    pub layer_content: Option<Placed>,
    pub backdrop: Option<Placed>,
    pub backdrop_mask: Option<&'a ImageData>,
    /// By ordinal.
    pub layer_masks: &'a [ImageData],
    /// Clips on the composites, per pixel, in the destination's pixels. At
    /// most four are evaluated.
    pub clips: &'a [ClipSpec],
    /// Coverage of every other clip on the composites, in its alpha; its
    /// origin is in the destination's texels.
    pub clip_mask: Option<Placed>,
}

/// What a plan resource is, here.
#[derive(Clone, Copy)]
enum Bound {
    Texture {
        slot: Slot,
        origin: (i32, i32),
        extent: (u32, u32),
    },
    Mask(xui_interface::ImageDataId),
}

impl FilterRenderer {
    /// Runs every pass of `plan`, in order, in one submission. The result is
    /// in `textures.destination`.
    pub(crate) fn execute_plan(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plan: &LayerRenderPlan,
        textures: &PlanTextures<'_>,
        scale: f32,
    ) -> Result<(), VelloBackendError> {
        // One texture per slot, for the whole plan.
        let slots: Vec<Slot> = plan
            .slots()
            .iter()
            .map(|slot| {
                let extent = (slot.extent.width.max(1), slot.extent.height.max(1));
                self.acquire_in(device, extent, Space::Work)
            })
            .collect();
        let result = self.encode_plan(device, queue, plan, textures, scale, &slots);
        for slot in slots {
            self.release(slot);
        }
        result
    }

    fn encode_plan(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plan: &LayerRenderPlan,
        textures: &PlanTextures<'_>,
        scale: f32,
        slots: &[Slot],
    ) -> Result<(), VelloBackendError> {
        let mut bound = Vec::with_capacity(plan.resources().len());
        for resource in plan.resources() {
            bound.push(match resource.kind {
                PlanResourceKind::Transient => {
                    let slot = resource
                        .slot
                        .and_then(|slot| slots.get(slot.index()))
                        .ok_or_else(|| invalid("a transient resource has no slot"))?;
                    let rect = resource.physical_bounds;
                    Some(Bound::Texture {
                        slot: *slot,
                        origin: (rect.x, rect.y),
                        extent: (rect.width, rect.height),
                    })
                }
                PlanResourceKind::External(kind) => self.bind_external(device, queue, kind, textures),
            });
        }
        let resolve = |id: PlanResourceId| {
            bound
                .get(id.index())
                .copied()
                .flatten()
                .ok_or_else(|| invalid("a pass uses an external resource nothing is bound to"))
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello render plan"),
        });
        for pass in plan.passes() {
            let Bound::Texture {
                slot: out_slot,
                origin,
                extent,
            } = resolve(pass.output)?
            else {
                return Err(invalid("a pass writes a mask"));
            };
            let (blend, scissor) = match pass.pipeline {
                PipelineKey::Filter => (Blend::Replace, Some([0, 0, extent.0, extent.1])),
                PipelineKey::Composite(attachment) => {
                    let bounds = match pass.op {
                        PassOp::BackdropComposite { bounds, .. }
                        | PassOp::LayerComposite { bounds, .. } => bounds,
                        _ => return Err(invalid("a composite pipeline on a filter pass")),
                    };
                    let blend = match attachment {
                        AttachmentBlend::Replace => Blend::Overwrite,
                        AttachmentBlend::SrcOver => Blend::Over,
                        AttachmentBlend::DstOver => Blend::Under,
                    };
                    let pixels = self.pool[out_slot.0].extent;
                    (blend, scissor_in(bounds, scale, origin, pixels))
                }
            };
            // Nothing of it is wanted, or nothing of it fits: a resource the
            // plan found no demand for is empty, and reads as transparent.
            let Some(scissor) = scissor.filter(|[_, _, w, h]| *w > 0 && *h > 0) else {
                continue;
            };

            let mut params = pass_params(pass, scale, origin)?;
            params.origin = [origin.0, origin.1, 0, 0];
            let mut inputs = [None; 4];
            if matches!(pass.pipeline, PipelineKey::Composite(_)) {
                set_clips(&mut params, textures.clips);
                inputs[3] = self.clip_mask_input(&mut params, textures.clip_mask);
            }
            let bindings = [
                pass.bindings.texture0,
                pass.bindings.texture1,
                pass.bindings.texture2,
            ];
            for (index, id) in bindings.into_iter().enumerate() {
                let Some(id) = id else { continue };
                inputs[index] = Some(match resolve(id)? {
                    Bound::Texture {
                        slot,
                        origin,
                        extent,
                    } => {
                        if slot == out_slot {
                            return Err(invalid("a pass samples the texture it writes"));
                        }
                        params.inputs[index] = [
                            origin.0,
                            origin.1,
                            i32::try_from(extent.0).unwrap_or(i32::MAX),
                            i32::try_from(extent.1).unwrap_or(i32::MAX),
                        ];
                        Input::Slot(slot)
                    }
                    Bound::Mask(id) => Input::Mask(id),
                });
            }
            self.encode_to(
                device,
                &mut encoder,
                params,
                inputs,
                Target {
                    view: &self.pool[out_slot.0].view,
                    space: Space::Work,
                    blend,
                    scissor: Some(scissor),
                },
            );
        }
        queue.submit([encoder.finish()]);
        Ok(())
    }

    fn bind_external(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        kind: ExternalResourceKind,
        textures: &PlanTextures<'_>,
    ) -> Option<Bound> {
        let placed = |placed: Placed| Bound::Texture {
            slot: placed.slot,
            origin: placed.origin,
            extent: self.pool[placed.slot.0].extent,
        };
        let mask = |this: &mut Self, data: &ImageData| {
            this.ensure_mask(device, queue, data);
            Bound::Mask(data.id())
        };
        match kind {
            ExternalResourceKind::ParentDestination => Some(placed(textures.destination)),
            ExternalResourceKind::LayerContent => textures.layer_content.map(placed),
            ExternalResourceKind::Backdrop => textures.backdrop.map(placed),
            ExternalResourceKind::BackdropMask => textures.backdrop_mask.map(|data| mask(self, data)),
            ExternalResourceKind::LayerMask(ordinal) => textures
                .layer_masks
                .get(ordinal as usize)
                .map(|data| mask(self, data)),
        }
    }
}

/// The shader's parameters for one pass, but for where its textures sit.
fn pass_params(pass: &Pass, scale: f32, origin: (i32, i32)) -> Result<Params, VelloBackendError> {
    let origin_f = [origin.0 as f32, origin.1 as f32];
    let params = match &pass.op {
        PassOp::Copy => Params::op(12),
        PassOp::GaussianBlur {
            axis,
            sigma_px,
            support,
        } => {
            let mut params = Params::op(match axis {
                Axis::X => 1,
                Axis::Y => 2,
            });
            params.p0 = [*sigma_px, *support, 0.0, 0.0];
            params
        }
        PassOp::ColorMatrix(matrix) => {
            let mut params = Params::op(3);
            params.m = matrix_rows(matrix);
            params
        }
        PassOp::Pixelate {
            block_width_px,
            block_height_px,
        } => {
            let mut params = Params::op(14);
            params.p0 = [*block_width_px, *block_height_px, 0.0, 0.0];
            params
        }
        PassOp::Refraction {
            strength_px,
            chromatic_aberration_px,
            center,
        } => {
            let mut params = Params::op(15);
            params.p0 = [*strength_px, *chromatic_aberration_px, 0.0, 0.0];
            // The shader wants the centre in this output's texels.
            params.p1 = [
                center.x * scale - origin_f[0],
                center.y * scale - origin_f[1],
                scale,
                0.0,
            ];
            params
        }
        PassOp::ChromaticAberration { offset_px } => {
            let mut params = Params::op(16);
            params.p0 = [offset_px[0], offset_px[1], 0.0, 0.0];
            params
        }
        PassOp::ExtractAlpha => Params::op(7),
        PassOp::AlphaSpread { axis, radius_px } => {
            let mut params = Params::op(match axis {
                Axis::X => 8,
                Axis::Y => 9,
            });
            params.p0[0] = *radius_px;
            params
        }
        PassOp::ShadowComposite { color, offset_px } => {
            let mut params = Params::op(10);
            params.p0 = [offset_px[0], offset_px[1], 0.0, 0.0];
            params.color = [color.r, color.g, color.b, color.a];
            params
        }
        PassOp::ApplyMask { transform, .. } => {
            let mut params = Params::op(11);
            params.p1 = [origin_f[0], origin_f[1], scale, 0.0];
            set_inverse(&mut params, *transform);
            params
        }
        PassOp::BackdropComposite {
            opacity,
            blend_mode,
            mask,
            ..
        } => {
            let mut params = Params::op(23);
            params.op[1] = crate::encode::blend_index(*blend_mode);
            params.op[3] = snapshot_flag(*blend_mode);
            params.p1 = [origin_f[0], origin_f[1], scale, *opacity];
            set_mask(&mut params, mask);
            params
        }
        PassOp::LayerComposite {
            opacity,
            transform,
            blend_mode,
            ..
        } => {
            let mut params = Params::op(22);
            params.op[1] = crate::encode::blend_index(*blend_mode);
            params.op[3] = snapshot_flag(*blend_mode);
            params.p1 = [origin_f[0], origin_f[1], scale, *opacity];
            let inverse = transform
                .invert()
                .ok_or_else(|| invalid("a scheduled composite transform is singular"))?;
            params.inv0 = [inverse.xx, inverse.yx, inverse.xy, inverse.yy];
            params.inv1 = [inverse.dx, inverse.dy, 0.0, 0.0];
            params
        }
    };
    Ok(params)
}

/// A blend mode other than normal comes with a snapshot of the destination
/// (input 1) and is blended in the shader, which then overwrites; a normal one
/// is left to the attachment blend.
fn snapshot_flag(mode: xui_render_graph::BlendMode) -> u32 {
    mode.requires_destination_snapshot() as u32
}

/// The clips, as `texture_over` reads them too: a count in `tex1.w`, four
/// vec4 each.
pub(super) fn set_clips(params: &mut Params, clips: &[ClipSpec]) {
    let clips = &clips[..clips.len().min(4)];
    params.tex1[3] = clips.len() as f32;
    for (index, clip) in clips.iter().enumerate() {
        let at = index * 4;
        params.clips[at] = [clip.inverse[0], clip.inverse[1], clip.inverse[2], clip.inverse[3]];
        params.clips[at + 1] = [clip.inverse[4], clip.inverse[5], 0.0, clip.radius];
        params.clips[at + 2] = clip.rect;
    }
}

/// `inv0`/`inv1` take whatever maps logical space onto a mask's unit square:
/// the inverse of `transform`. A singular one covers nothing, so it maps
/// everything outside the square.
fn set_inverse(params: &mut Params, transform: Affine) {
    match transform.invert() {
        Some(inverse) => {
            params.inv0 = [inverse.xx, inverse.yx, inverse.xy, inverse.yy];
            params.inv1 = [inverse.dx, inverse.dy, 0.0, 0.0];
        }
        None => {
            params.inv0 = [0.0; 4];
            params.inv1 = [-1.0, -1.0, 0.0, 0.0];
        }
    }
}

/// A backdrop mask in the shader's terms (`mask_alpha`): kind in `op.z`.
fn set_mask(params: &mut Params, mask: &PlanMask) {
    match mask {
        PlanMask::None => params.op[2] = 0,
        PlanMask::Texture { transform, .. } => {
            params.op[2] = 6;
            set_inverse(params, *transform);
        }
        PlanMask::Shape { shape, transform } => {
            set_inverse(params, *transform);
            params.op[2] = match shape {
                MaskShape::Rect => 1,
                MaskShape::RoundedRect(radius) => {
                    // The shader works in the unit square, so the radius is
                    // taken in its units.
                    let x_scale = transform.xx.hypot(transform.yx);
                    let y_scale = transform.xy.hypot(transform.yy);
                    params.p0[3] = *radius / x_scale.min(y_scale).max(f32::EPSILON);
                    2
                }
                MaskShape::Circle => 3,
                MaskShape::Ellipse => 4,
                MaskShape::Line { from, to } => {
                    params.p0 = [from.x, from.y, 0.5, 0.0];
                    params.color = [to.x, to.y, 0.0, 0.0];
                    5
                }
            };
        }
    }
}

/// `bounds` (logical) as a scissor in a texture whose texel (0, 0) is at
/// `origin` physical pixels and which is `pixels` large; `None` if they do not
/// meet. Rounded outward, as the plan rounds every rectangle.
fn scissor_in(
    bounds: xui_interface::Bounds,
    scale: f32,
    origin: (i32, i32),
    pixels: (u32, u32),
) -> Option<[u32; 4]> {
    let to_texels = |value: f32, origin: i32| f64::from(value) * f64::from(scale) - f64::from(origin);
    let x0 = to_texels(bounds.min.x, origin.0).floor().max(0.0);
    let y0 = to_texels(bounds.min.y, origin.1).floor().max(0.0);
    let x1 = to_texels(bounds.max.x, origin.0).ceil().min(f64::from(pixels.0));
    let y1 = to_texels(bounds.max.y, origin.1).ceil().min(f64::from(pixels.1));
    (x1 > x0 && y1 > y0).then(|| [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
}

fn invalid(reason: &'static str) -> VelloBackendError {
    VelloBackendError::Plan(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{DEFAULT_POOL_BUDGET, Finish};
    use xui_interface::{Bounds, Color, Point, Size};
    use xui_render_graph::{
        BackdropDescriptor, BackdropFilter, BlendMode, CompositeInstance, ExternalAliasing,
        FilterQuality, LayerEffect, LayerGraphDescriptor, LayerPlanContext, LayerProgramEntry,
        PlanLimits, TextureClass, compile_layer,
    };

    struct Gpu {
        context: crate::WgpuContext,
        filters: FilterRenderer,
    }

    fn gpu() -> Option<Gpu> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).ok()?;
        let filters = FilterRenderer::new(&device, DEFAULT_POOL_BUDGET);
        Some(Gpu {
            context: crate::WgpuContext { device, queue },
            filters,
        })
    }

    impl Gpu {
        /// A linear, premultiplied texture of `extent` whose texel (0, 0) is at
        /// `origin`, from sRGB-encoded straight pixels.
        fn upload(
            &mut self,
            origin: (i32, i32),
            extent: (u32, u32),
            pixel: impl Fn(i32, i32) -> [u8; 4],
        ) -> Placed {
            let encoded = self.filters.acquire(&self.context.device, extent);
            let pixels: Vec<u8> = (0..extent.1 as i32)
                .flat_map(|y| (0..extent.0 as i32).map(move |x| (x, y)))
                .flat_map(|(x, y)| pixel(origin.0 + x, origin.1 + y))
                .collect();
            self.context.queue.write_texture(
                self.filters.texture(encoded).as_image_copy(),
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(extent.0 * 4),
                    rows_per_image: Some(extent.1),
                },
                wgpu::Extent3d {
                    width: extent.0,
                    height: extent.1,
                    depth_or_array_layers: 1,
                },
            );
            let slot = self.filters.premultiplied(
                &self.context.device,
                &self.context.queue,
                encoded,
                extent,
            );
            Placed { slot, origin }
        }

        /// What `placed` holds, sRGB-encoded and straight. It is released.
        fn read(&mut self, placed: Placed) -> Vec<u8> {
            let extent = self.filters.pool[placed.slot.0].extent;
            let out = self.filters.acquire(&self.context.device, extent);
            self.filters.finish(
                &self.context.device,
                &self.context.queue,
                placed.slot,
                Finish::Slot(out),
            );
            crate::backend::read_texture(&self.context, self.filters.texture(out), extent)
                .unwrap()
        }

        fn run(&mut self, plan: &LayerRenderPlan, textures: &PlanTextures<'_>) {
            self.filters
                .execute_plan(&self.context.device, &self.context.queue, plan, textures, 1.0)
                .unwrap();
        }
    }

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds {
        Bounds::from_origin_size(Point::new(x, y), Size::new(width, height))
    }

    fn context(destination: Bounds, content: Bounds) -> LayerPlanContext {
        LayerPlanContext {
            backdrop_source_bounds: destination,
            parent_destination_bounds: destination,
            composite_clip_bounds: None,
            layer_content_bounds: content,
            backdrop_bounds: None,
            composite: CompositeInstance::default(),
            scale_factor: 1.0,
            color_texture_class: TextureClass::COLOR,
            external_aliasing: ExternalAliasing::Distinct,
            limits: PlanLimits::default(),
        }
    }

    fn textures(destination: Placed, layer_content: Option<Placed>) -> PlanTextures<'static> {
        PlanTextures {
            destination,
            layer_content,
            backdrop: None,
            backdrop_mask: None,
            layer_masks: &[],
            clips: &[],
            clip_mask: None,
        }
    }

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];
    const RED: [u8; 4] = [255, 0, 0, 255];

    fn alpha(pixels: &[u8], x: usize) -> u8 {
        pixels[x * 4 + 3]
    }

    /// The standard normal CDF (Abramowitz and Stegun 7.1.26 for erf).
    fn phi(x: f32) -> f32 {
        let z = x / std::f32::consts::SQRT_2;
        let t = 1.0 / (1.0 + 0.327_591_1 * z.abs());
        let poly = t
            * (0.254_829_6
                + t * (-0.284_496_74 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
        let erf = 1.0 - poly * (-z * z).exp();
        0.5 * (1.0 + erf.copysign(z))
    }

    fn blurred_half_plane() -> LayerGraphDescriptor {
        LayerGraphDescriptor {
            effects: vec![LayerEffect::Blur {
                sigma_x: 20.0,
                sigma_y: 0.0,
                quality: FilterQuality::Medium,
            }],
            ..LayerGraphDescriptor::default()
        }
    }

    /// Layer-local 100..900 with its edge at 500, placed 100 to the left: the
    /// edge lands at 400 in an 800-wide parent. Every texture is offset from
    /// every other, so a wrong origin anywhere moves the edge.
    #[test]
    fn a_blurred_layer_composites_through_the_plan_as_the_gaussian() {
        let Some(mut gpu) = gpu() else { return };
        let mut ctx = context(bounds(0.0, 0.0, 800.0, 1.0), bounds(100.0, 0.0, 800.0, 1.0));
        ctx.composite.transform = Affine::translate(-100.0, 0.0);
        let plan = compile_layer(&blurred_half_plane())
            .unwrap()
            .instantiate(&ctx)
            .unwrap();
        let content = gpu.upload((100, 0), (800, 1), |x, _| if x < 500 { WHITE } else { CLEAR });
        let destination = gpu.upload((0, 0), (800, 1), |_, _| CLEAR);
        gpu.run(&plan, &textures(destination, Some(content)));
        let out = gpu.read(destination);
        let worst = (300..500)
            .map(|x| {
                let expected = 255.0 * (1.0 - phi((x as f32 + 0.5 - 400.0) / 20.0));
                (alpha(&out, x) as f32 - expected).abs()
            })
            .fold(0.0, f32::max);
        assert!(worst < 3.0, "off by {worst}/255");
    }

    /// Repainting part of a blurred layer: only what the plan demands of the
    /// layer's content is rendered, and the clipped composite still comes out
    /// as it does when everything is drawn. What is outside the clip is left
    /// alone.
    #[test]
    fn a_clipped_composite_from_demanded_content_has_no_seam() {
        let Some(mut gpu) = gpu() else { return };
        let program = compile_layer(&blurred_half_plane()).unwrap();
        let half_plane = |x: i32, _| if x < 400 { WHITE } else { CLEAR };
        let whole_ctx = context(bounds(0.0, 0.0, 800.0, 1.0), bounds(0.0, 0.0, 800.0, 1.0));
        let whole = {
            let plan = program.instantiate(&whole_ctx).unwrap();
            let content = gpu.upload((0, 0), (800, 1), half_plane);
            let destination = gpu.upload((0, 0), (800, 1), |_, _| RED);
            gpu.run(&plan, &textures(destination, Some(content)));
            gpu.read(destination)
        };
        let clip = (380, 420);
        let clipped = {
            let plan = program
                .instantiate(&LayerPlanContext {
                    composite_clip_bounds: Some(bounds(clip.0 as f32, 0.0, 40.0, 1.0)),
                    ..whole_ctx
                })
                .unwrap();
            let demand = plan.resources()[plan.layer_content().index()].physical_bounds;
            // Narrower than the layer, wider than the clip by the blur's reach.
            assert!(demand.x > 0 && demand.x < clip.0 - 50, "{demand:?}");
            assert!(demand.width < 800, "{demand:?}");
            let content = gpu.upload((demand.x, demand.y), (demand.width, demand.height), half_plane);
            let destination = gpu.upload((0, 0), (800, 1), |_, _| RED);
            gpu.run(&plan, &textures(destination, Some(content)));
            gpu.read(destination)
        };
        for x in 0..800 {
            let got = &clipped[x * 4..x * 4 + 4];
            if (clip.0..clip.1).contains(&(x as i32)) {
                let want = &whole[x * 4..x * 4 + 4];
                let diff = got.iter().zip(want).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                assert!(diff <= 1, "x = {x}: {got:?} against {want:?}");
            } else {
                assert_eq!(got, RED, "x = {x} is outside the clip");
            }
        }
    }

    #[test]
    fn a_translated_layer_lands_on_whole_pixels() {
        let Some(mut gpu) = gpu() else { return };
        let mut ctx = context(bounds(0.0, 0.0, 10.0, 10.0), bounds(0.0, 0.0, 4.0, 4.0));
        ctx.composite.transform = Affine::translate(3.0, 2.0);
        let plan = compile_layer(&LayerGraphDescriptor::default())
            .unwrap()
            .instantiate(&ctx)
            .unwrap();
        let content = gpu.upload((0, 0), (4, 4), |_, _| WHITE);
        let destination = gpu.upload((0, 0), (10, 10), |_, _| CLEAR);
        gpu.run(&plan, &textures(destination, Some(content)));
        let out = gpu.read(destination);
        for y in 0..10 {
            for x in 0..10 {
                let inside = (3..7).contains(&x) && (2..6).contains(&y);
                assert_eq!(
                    alpha(&out, y * 10 + x),
                    if inside { 255 } else { 0 },
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn a_shadow_spread_wider_than_one_pass_reaches_its_full_radius() {
        let Some(mut gpu) = gpu() else { return };
        let value = LayerGraphDescriptor {
            effects: vec![LayerEffect::DropShadow {
                color: Color::rgba(0.0, 0.0, 0.0, 1.0),
                offset: Point::new(0.0, 0.0),
                sigma_x: 0.0,
                sigma_y: 0.0,
                spread: 200.0,
                quality: FilterQuality::Medium,
            }],
            ..LayerGraphDescriptor::default()
        };
        let plan = compile_layer(&value)
            .unwrap()
            .instantiate(&context(
                bounds(0.0, 0.0, 1000.0, 1.0),
                bounds(500.0, 0.0, 1.0, 1.0),
            ))
            .unwrap();
        let content = gpu.upload((500, 0), (1, 1), |_, _| WHITE);
        let destination = gpu.upload((0, 0), (1000, 1), |_, _| CLEAR);
        gpu.run(&plan, &textures(destination, Some(content)));
        let out = gpu.read(destination);
        assert_eq!(alpha(&out, 300), 255, "the shadow stops short of 200px");
        assert_eq!(alpha(&out, 700), 255);
        assert_eq!(alpha(&out, 299), 0, "the shadow runs past 200px");
        assert_eq!(alpha(&out, 701), 0);
        assert_eq!(&out[500 * 4..501 * 4], WHITE, "the layer is over its shadow");
    }

    /// The backdrop is the texture it is composited into. Inverted, it reads
    /// before it writes; multiplied with no filter, the plan copies it first.
    #[test]
    fn a_backdrop_composites_onto_the_texture_it_reads() {
        let Some(mut gpu) = gpu() else { return };
        let gray = 128u8;
        let cases = [
            (
                // On the encoded values, as everything here is.
                BackdropFilter::Invert(1.0),
                BlendMode::Normal,
                255 - gray,
            ),
            (
                // An identity filter, dropped by the compiler: no filter pass.
                BackdropFilter::Invert(0.0),
                BlendMode::Multiply,
                ((gray as f32 / 255.0).powi(2) * 255.0).round() as u8,
            ),
        ];
        for (filter, blend_mode, expected) in cases {
            let value = LayerGraphDescriptor {
                backdrop: Some(BackdropDescriptor {
                    filters: vec![filter],
                    blend_mode,
                    ..BackdropDescriptor::default()
                }),
                ..LayerGraphDescriptor::default()
            };
            let plan = compile_layer(&value)
                .unwrap()
                .instantiate_entry(
                    LayerProgramEntry::BackdropOnly,
                    &LayerPlanContext {
                        backdrop_bounds: Some(bounds(2.0, 0.0, 4.0, 1.0)),
                        external_aliasing: ExternalAliasing::BackdropAndDestination,
                        ..context(bounds(0.0, 0.0, 8.0, 1.0), bounds(2.0, 0.0, 4.0, 1.0))
                    },
                )
                .unwrap();
            let target = gpu.upload((0, 0), (8, 1), |_, _| [gray, gray, gray, 255]);
            gpu.run(
                &plan,
                &PlanTextures {
                    backdrop: Some(target),
                    ..textures(target, None)
                },
            );
            let out = gpu.read(target);
            for x in 0..8 {
                let want = if (2..6).contains(&x) { expected } else { gray };
                assert!(
                    out[x * 4].abs_diff(want) <= 1 && out[x * 4 + 3] == 255,
                    "{blend_mode:?}, x = {x}: {:?}, want {want}",
                    &out[x * 4..x * 4 + 4]
                );
            }
        }
    }

    /// A chain of filters runs between two textures, whatever its length.
    #[test]
    fn a_chain_of_filters_runs_ping_pong_in_two_textures() {
        let Some(mut gpu) = gpu() else { return };
        let value = LayerGraphDescriptor {
            effects: (0..4)
                .map(|_| LayerEffect::Blur {
                    sigma_x: 1.0,
                    sigma_y: 1.0,
                    quality: FilterQuality::Medium,
                })
                .collect(),
            ..LayerGraphDescriptor::default()
        };
        let plan = compile_layer(&value)
            .unwrap()
            .instantiate(&context(bounds(0.0, 0.0, 64.0, 64.0), bounds(16.0, 16.0, 32.0, 32.0)))
            .unwrap();
        assert_eq!(plan.stats().transient_resource_count, 8);
        assert_eq!(plan.slots().len(), 2);
        let content = gpu.upload((16, 16), (32, 32), |_, _| WHITE);
        let destination = gpu.upload((0, 0), (64, 64), |_, _| CLEAR);
        let (before, _) = gpu.filters.pool_size();
        gpu.run(&plan, &textures(destination, Some(content)));
        let (after, _) = gpu.filters.pool_size();
        assert!(after - before <= 2, "{} textures for two slots", after - before);
        let out = gpu.read(destination);
        // Blurred: the middle is opaque, the corner of the parent untouched.
        assert_eq!(alpha(&out, 32 * 64 + 32), 255);
        assert_eq!(alpha(&out, 0), 0);
    }

    /// The layer branch run on its own, onto transparent and over its whole
    /// domain, then composited by the program with that branch cut out, is
    /// the whole program: what a cached filter result relies on.
    #[test]
    fn a_layer_branch_run_apart_composites_as_the_whole_program() {
        let Some(mut gpu) = gpu() else { return };
        let value = LayerGraphDescriptor {
            effects: vec![
                LayerEffect::Blur {
                    sigma_x: 2.0,
                    sigma_y: 2.0,
                    quality: FilterQuality::Medium,
                },
                LayerEffect::DropShadow {
                    color: Color::rgba(0.0, 0.0, 0.5, 1.0),
                    offset: Point::new(4.0, 3.0),
                    sigma_x: 2.0,
                    sigma_y: 2.0,
                    spread: 1.0,
                    quality: FilterQuality::Medium,
                },
            ],
            composite: xui_render_graph::CompositeDescriptor {
                blend_mode: BlendMode::Multiply,
                ..Default::default()
            },
            ..LayerGraphDescriptor::default()
        };
        let program = compile_layer(&value).unwrap();
        let content = bounds(10.0, 10.0, 20.0, 20.0);
        let mut ctx = context(bounds(0.0, 0.0, 64.0, 64.0), content);
        ctx.composite.transform = Affine::translate(5.0, 4.0);
        let square = |x: i32, y: i32| {
            if (10..30).contains(&x) && (10..30).contains(&y) {
                [255, 200, 40, 255]
            } else {
                CLEAR
            }
        };
        let gray = |_, _| [160, 160, 160, 255];

        let whole = {
            let plan = program.instantiate(&ctx).unwrap();
            let layer = gpu.upload((10, 10), (20, 20), square);
            let destination = gpu.upload((0, 0), (64, 64), gray);
            gpu.run(&plan, &textures(destination, Some(layer)));
            gpu.read(destination)
        };

        let split = {
            let e = program.layer_visual_expansion();
            let domain = Bounds::new(
                Point::new(content.min.x - e.left, content.min.y - e.top),
                Point::new(content.max.x + e.right, content.max.y + e.bottom),
            );
            let (x0, y0) = (domain.min.x.floor() as i32, domain.min.y.floor() as i32);
            let (x1, y1) = (domain.max.x.ceil() as i32, domain.max.y.ceil() as i32);
            let extent = ((x1 - x0) as u32, (y1 - y0) as u32);
            let filter_plan = program
                .instantiate_entry(
                    LayerProgramEntry::LayerOnly,
                    &context(domain, content),
                )
                .unwrap();
            let layer = gpu.upload((10, 10), (20, 20), square);
            let filtered = gpu.upload((x0, y0), extent, |_, _| CLEAR);
            gpu.run(&filter_plan, &textures(filtered, Some(layer)));

            let cut_plan = program
                .without_layer_effects()
                .instantiate(&LayerPlanContext {
                    layer_content_bounds: domain,
                    ..ctx
                })
                .unwrap();
            let destination = gpu.upload((0, 0), (64, 64), gray);
            gpu.run(&cut_plan, &textures(destination, Some(filtered)));
            gpu.read(destination)
        };
        let worst = whole
            .iter()
            .zip(&split)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 1, "split differs by {worst}");
        // And the layer did reach the destination.
        assert_ne!(&whole[(20 * 64 + 20) * 4..][..4], &[160, 160, 160, 255]);
    }

}
