use xui_interface::{Affine, Bounds, Color, ImageData, ImageKey, Point, Size};
use xui_render_graph::{
    AttachmentBlend, Axis, BackdropDescriptor, BackdropFilter, BlendMode, CompositeDescriptor,
    CompositeInstance, CompositeOperator, CoordinateSpace, DrawShader, ExternalAliasing,
    ExternalResourceKind, FilterQuality, LayerEffect, LayerGraphDescriptor, LayerPlanContext,
    LayerProgramEntry, PassOp, PipelineKey, PlanError, PlanLimits, PlanResourceKind, TextureClass,
    compile_layer,
};

fn context() -> LayerPlanContext {
    LayerPlanContext {
        backdrop_source_bounds: rect_bounds(0.0, 0.0, 200.0, 150.0),
        parent_destination_bounds: rect_bounds(0.0, 0.0, 200.0, 150.0),
        composite_clip_bounds: None,
        layer_content_bounds: rect_bounds(20.0, 30.0, 40.0, 30.0),
        backdrop_bounds: Some(rect_bounds(20.0, 30.0, 40.0, 30.0)),
        composite: CompositeInstance::default(),
        scale_factor: 1.0,
        color_texture_class: TextureClass::LINEAR_COLOR,
        external_aliasing: ExternalAliasing::Distinct,
        limits: PlanLimits::default(),
    }
}

fn descriptor() -> LayerGraphDescriptor {
    LayerGraphDescriptor::default()
}

fn rect_bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds {
    Bounds::from_origin_size(Point::new(x, y), Size::new(w, h))
}

fn image_mask(key: u64, bounds: Bounds) -> LayerEffect {
    LayerEffect::ImageMask {
        image: ImageKey::UserProvided(key),
        data: ImageData::rgba8(Size::new(1, 1), [255, 255, 255, 255]),
        bounds,
    }
}

#[test]
fn backdrop_effects_and_layer_composite_have_fixed_order() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Blur {
                sigma_x: 2.0,
                sigma_y: 3.0,
                quality: FilterQuality::Medium,
            }],
            ..BackdropDescriptor::default()
        }),
        effects: vec![LayerEffect::ColorMatrix([
            0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ])],
        ..descriptor()
    };
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&context())
        .unwrap();
    assert!(matches!(
        plan.passes()[0].op,
        PassOp::GaussianBlur { axis: Axis::X, .. }
    ));
    assert!(matches!(
        plan.passes()[1].op,
        PassOp::GaussianBlur { axis: Axis::Y, .. }
    ));
    assert!(matches!(
        plan.passes()[2].op,
        PassOp::BackdropComposite { .. }
    ));
    assert!(matches!(plan.passes()[3].op, PassOp::ColorMatrix(_)));
    assert!(matches!(plan.passes()[4].op, PassOp::LayerComposite { .. }));
}

#[test]
fn explicit_program_entries_lower_only_the_selected_branch() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Blur {
                sigma_x: 2.0,
                sigma_y: 0.0,
                quality: FilterQuality::Medium,
            }],
            ..BackdropDescriptor::default()
        }),
        effects: vec![LayerEffect::Blur {
            sigma_x: 0.0,
            sigma_y: 3.0,
            quality: FilterQuality::Medium,
        }],
        ..descriptor()
    };
    let program = compile_layer(&value).unwrap();
    let backdrop = program
        .instantiate_entry(LayerProgramEntry::BackdropOnly, &context())
        .unwrap();
    assert!(
        backdrop
            .passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
    );
    assert!(
        !backdrop
            .passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::LayerComposite { .. }))
    );

    let layer = program
        .instantiate_entry(LayerProgramEntry::LayerOnly, &context())
        .unwrap();
    assert!(
        !layer
            .passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
    );
    assert!(
        layer
            .passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::LayerComposite { .. }))
    );
}

#[test]
fn passes_fully_specify_pipeline_bindings_and_draw_program() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Brightness(0.8)],
            ..BackdropDescriptor::default()
        }),
        effects: vec![LayerEffect::ColorMatrix([
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ])],
        ..descriptor()
    };
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&context())
        .unwrap();
    for pass in plan.passes() {
        assert_eq!(pass.draw.vertex_count, 3);
        assert!(pass.draw.viewport.width > 0);
        assert!(pass.bindings.texture0.is_some());
        assert_eq!(pass.uniforms.scale_factor, context().scale_factor);
    }
    let backdrop = plan
        .passes()
        .iter()
        .find(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
        .unwrap();
    assert_eq!(
        backdrop.pipeline,
        PipelineKey::Composite(AttachmentBlend::SrcOver)
    );
    assert_eq!(backdrop.draw.shader, DrawShader::AttachmentBackdrop);
    let layer = plan.passes().last().unwrap();
    assert_eq!(
        layer.pipeline,
        PipelineKey::Composite(AttachmentBlend::SrcOver)
    );
    assert_eq!(layer.draw.shader, DrawShader::AttachmentLayer);
}

#[test]
fn backdrop_source_demand_can_extend_beyond_parent_destination_tile() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Blur {
                sigma_x: 8.0,
                sigma_y: 8.0,
                quality: FilterQuality::Medium,
            }],
            ..BackdropDescriptor::default()
        }),
        ..descriptor()
    };
    let mut ctx = context();
    ctx.backdrop_source_bounds = rect_bounds(0.0, 0.0, 200.0, 150.0);
    ctx.parent_destination_bounds = rect_bounds(50.0, 50.0, 20.0, 20.0);
    ctx.backdrop_bounds = Some(ctx.parent_destination_bounds);
    let plan = compile_layer(&value).unwrap().instantiate(&ctx).unwrap();
    let backdrop = &plan.resources()[plan.backdrop().unwrap().index()];
    assert!(backdrop.logical_bounds.x() < ctx.parent_destination_bounds.x());
    assert!(backdrop.logical_bounds.y() < ctx.parent_destination_bounds.y());
    assert!(
        backdrop.logical_bounds.x() + backdrop.logical_bounds.width()
            > ctx.parent_destination_bounds.x() + ctx.parent_destination_bounds.width()
    );
}

#[test]
fn large_blur_and_spread_are_expanded_into_actual_passes() {
    let value = LayerGraphDescriptor {
        effects: vec![
            LayerEffect::Blur {
                sigma_x: 64.0,
                sigma_y: 0.0,
                quality: FilterQuality::High,
            },
            LayerEffect::DropShadow {
                color: Color::BLACK,
                offset: Point::new(0.0, 0.0),
                sigma_x: 0.0,
                sigma_y: 0.0,
                spread: 257.0,
                quality: FilterQuality::Medium,
            },
        ],
        ..descriptor()
    };
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&context())
        .unwrap();
    assert!(
        plan.passes()
            .iter()
            .filter(|pass| matches!(pass.op, PassOp::GaussianBlur { axis: Axis::X, .. }))
            .count()
            > 1
    );
    assert_eq!(
        plan.passes()
            .iter()
            .filter(|pass| matches!(pass.op, PassOp::AlphaSpread { axis: Axis::X, .. }))
            .count(),
        3
    );
    assert_eq!(
        plan.passes()
            .iter()
            .filter(|pass| matches!(pass.op, PassOp::AlphaSpread { axis: Axis::Y, .. }))
            .count(),
        3
    );
}

#[test]
fn drop_shadow_lowers_to_explicit_dag_and_asymmetric_bounds() {
    let mut value = descriptor();
    value.effects.push(LayerEffect::DropShadow {
        color: Color::rgba(0.0, 0.0, 0.0, 0.5),
        offset: Point::new(5.0, -3.0),
        sigma_x: 2.0,
        sigma_y: 1.0,
        spread: 1.0,
        quality: FilterQuality::Medium,
    });
    let program = compile_layer(&value).unwrap();
    assert_eq!(program.layer_visual_expansion().left, 2.0);
    assert_eq!(program.layer_visual_expansion().right, 12.0);
    assert_eq!(program.layer_visual_expansion().top, 7.0);
    assert_eq!(program.layer_visual_expansion().bottom, 1.0);
    let plan = program.instantiate(&context()).unwrap();
    assert!(matches!(plan.passes()[0].op, PassOp::ExtractAlpha));
    assert!(matches!(
        plan.passes()[1].op,
        PassOp::AlphaSpread { axis: Axis::X, .. }
    ));
    assert!(matches!(
        plan.passes()[2].op,
        PassOp::AlphaSpread { axis: Axis::Y, .. }
    ));
    assert!(
        plan.passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::GaussianBlur { axis: Axis::X, .. }))
    );
    assert!(
        plan.passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::GaussianBlur { axis: Axis::Y, .. }))
    );
    let merge = plan
        .passes()
        .iter()
        .find(|pass| matches!(pass.op, PassOp::ShadowComposite { .. }))
        .unwrap();
    assert!(matches!(merge.op, PassOp::ShadowComposite { .. }));
    assert_eq!(merge.inputs[0], plan.layer_content());
    assert_eq!(merge.inputs.len(), 2);
    assert!(matches!(
        plan.passes().last().unwrap().op,
        PassOp::LayerComposite { .. }
    ));
}

#[test]
fn shadow_skips_zero_spread_and_blur_but_preserves_merge() {
    let mut value = descriptor();
    value.effects.push(LayerEffect::DropShadow {
        color: Color::BLACK,
        offset: Point::new(2.0, 0.0),
        sigma_x: 0.0,
        sigma_y: 0.0,
        spread: 0.0,
        quality: FilterQuality::High,
    });
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&context())
        .unwrap();
    assert_eq!(
        plan.passes()
            .iter()
            .filter(|pass| matches!(pass.op, PassOp::ExtractAlpha))
            .count(),
        1
    );
    assert!(!plan.passes().iter().any(|pass| matches!(
        pass.op,
        PassOp::AlphaSpread { .. } | PassOp::GaussianBlur { .. }
    )));
    assert!(
        plan.passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::ShadowComposite { .. }))
    );
}

#[test]
fn advanced_blends_snapshot_at_each_terminal_and_attachment_modes_do_not() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        filters: vec![BackdropFilter::Brightness(0.8)],
        blend_mode: BlendMode::Multiply,
        ..BackdropDescriptor::default()
    });
    value.composite = CompositeDescriptor {
        blend_mode: BlendMode::Screen,
        operator: CompositeOperator::SrcOver,
    };
    let mut aliased_context = context();
    aliased_context.external_aliasing = ExternalAliasing::BackdropAndDestination;
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&aliased_context)
        .unwrap();
    let backdrop = plan
        .passes()
        .iter()
        .position(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
        .unwrap();
    let layer = plan
        .passes()
        .iter()
        .position(|pass| matches!(pass.op, PassOp::LayerComposite { .. }))
        .unwrap();
    assert!(matches!(plan.passes()[backdrop - 1].op, PassOp::Copy));
    assert!(matches!(plan.passes()[layer - 1].op, PassOp::Copy));
    assert!(backdrop < layer - 1);

    value.backdrop.as_mut().unwrap().blend_mode = BlendMode::Normal;
    value.composite.blend_mode = BlendMode::Normal;
    value.composite.operator = CompositeOperator::DstOver;
    let attachment = compile_layer(&value)
        .unwrap()
        .instantiate(&aliased_context)
        .unwrap();
    assert!(
        !attachment
            .passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::Copy))
    );
}

#[test]
fn layer_snapshot_observes_destination_after_backdrop_composite() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor::default());
    value.composite.blend_mode = BlendMode::Difference;
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&context())
        .unwrap();
    let backdrop = plan
        .passes()
        .iter()
        .position(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
        .unwrap();
    let snapshot = plan
        .passes()
        .iter()
        .position(|pass| matches!(pass.op, PassOp::Copy))
        .unwrap();
    let layer = plan
        .passes()
        .iter()
        .position(|pass| matches!(pass.op, PassOp::LayerComposite { .. }))
        .unwrap();
    assert!(backdrop < snapshot && snapshot + 1 == layer);
    assert_eq!(plan.passes()[snapshot].inputs[0], plan.parent_destination());
}

#[test]
fn transform_maps_forward_and_crops_demand_back_to_layer_space() {
    let mut value = descriptor();
    value.effects.push(LayerEffect::Blur {
        sigma_x: 2.0,
        sigma_y: 0.0,
        quality: FilterQuality::Medium,
    });
    let mut ctx = context();
    ctx.layer_content_bounds = rect_bounds(0.0, 0.0, 100.0, 80.0);
    ctx.parent_destination_bounds = rect_bounds(50.0, 20.0, 30.0, 40.0);
    ctx.composite.transform = Affine::new(1.0, 0.25, 0.5, 1.0, 40.0, 10.0);
    let plan = compile_layer(&value).unwrap().instantiate(&ctx).unwrap();
    let layer = &plan.resources()[plan.layer_content().index()];
    assert_eq!(layer.coordinate_space, CoordinateSpace::LayerLocal);
    assert!(layer.logical_bounds.width() < ctx.layer_content_bounds.width());
    let PassOp::LayerComposite { bounds, .. } = plan.passes().last().unwrap().op else {
        panic!("layer terminal")
    };
    assert_eq!(bounds, ctx.parent_destination_bounds);
}

#[test]
fn translate_scale_and_rotation_produce_expected_terminal_bounds() {
    let value = descriptor();
    let mut ctx = context();
    ctx.layer_content_bounds = rect_bounds(0.0, 0.0, 10.0, 20.0);
    ctx.parent_destination_bounds = rect_bounds(-100.0, -100.0, 300.0, 300.0);
    ctx.composite.transform = Affine::new(0.0, 2.0, -3.0, 0.0, 50.0, 10.0);
    let plan = compile_layer(&value).unwrap().instantiate(&ctx).unwrap();
    let PassOp::LayerComposite { bounds, .. } = plan.passes().last().unwrap().op else {
        panic!("layer terminal")
    };
    assert_eq!(bounds, rect_bounds(-10.0, 10.0, 60.0, 20.0));
}

#[test]
fn singular_transform_skips_layer_but_not_backdrop() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor::default());
    value.effects.push(LayerEffect::Blur {
        sigma_x: 3.0,
        sigma_y: 3.0,
        quality: FilterQuality::Medium,
    });
    let mut ctx = context();
    ctx.composite.transform = Affine::scale(0.0, 1.0);
    let plan = compile_layer(&value).unwrap().instantiate(&ctx).unwrap();
    assert!(
        plan.passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
    );
    assert!(!plan.passes().iter().any(|pass| matches!(
        pass.op,
        PassOp::LayerComposite { .. }
            | PassOp::GaussianBlur { .. }
            | PassOp::ColorMatrix(_)
            | PassOp::ExtractAlpha
    )));
}

#[test]
fn zero_dynamic_opacity_skips_only_layer_branch() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor::default());
    value.effects.push(LayerEffect::ColorMatrix([0.5; 20]));
    let mut ctx = context();
    ctx.composite.opacity = 0.0;
    let plan = compile_layer(&value).unwrap().instantiate(&ctx).unwrap();
    assert!(
        plan.passes()
            .iter()
            .any(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
    );
    assert!(!plan.passes().iter().any(|pass| matches!(
        pass.op,
        PassOp::LayerComposite { .. } | PassOp::ColorMatrix(_)
    )));
}

#[test]
fn empty_reverse_clip_skips_the_complete_layer_branch() {
    let mut value = descriptor();
    value.effects.push(LayerEffect::DropShadow {
        color: Color::BLACK,
        offset: Point::new(8.0, 8.0),
        sigma_x: 4.0,
        sigma_y: 4.0,
        spread: 2.0,
        quality: FilterQuality::Medium,
    });
    let mut ctx = context();
    ctx.parent_destination_bounds = rect_bounds(1_000.0, 1_000.0, 20.0, 20.0);
    let plan = compile_layer(&value).unwrap().instantiate(&ctx).unwrap();
    assert!(plan.is_noop());
    assert_eq!(
        plan.resources()[plan.layer_content().index()].logical_bounds,
        Bounds::ZERO
    );
}

#[test]
fn external_masks_never_allocate_slots_and_spaces_are_explicit() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        mask: xui_render_graph::Mask::AlphaTexture {
            texture: ImageKey::UserProvided(1),
            transform: Affine::IDENTITY,
        },
        ..BackdropDescriptor::default()
    });
    value.effects = vec![
        image_mask(2, rect_bounds(0.0, 0.0, 1.0, 1.0)),
        image_mask(3, rect_bounds(1.0, 0.0, 1.0, 1.0)),
    ];
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&context())
        .unwrap();
    for kind in [
        ExternalResourceKind::BackdropMask,
        ExternalResourceKind::LayerMask(0),
        ExternalResourceKind::LayerMask(1),
    ] {
        let id = plan.external_resource(kind).unwrap();
        let resource = &plan.resources()[id.index()];
        assert!(matches!(resource.kind, PlanResourceKind::External(_)));
        assert_eq!(resource.slot, None);
    }
    assert_eq!(
        plan.resources()[plan
            .external_resource(ExternalResourceKind::BackdropMask)
            .unwrap()
            .index()]
        .coordinate_space,
        CoordinateSpace::Parent
    );
    assert_eq!(
        plan.resources()[plan
            .external_resource(ExternalResourceKind::LayerMask(0))
            .unwrap()
            .index()]
        .coordinate_space,
        CoordinateSpace::LayerLocal
    );
}

#[test]
fn transient_allocator_reuses_across_finished_branches_deterministically() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        filters: vec![
            BackdropFilter::Brightness(0.8),
            BackdropFilter::Pixelate {
                size: Size::new(2.0, 2.0),
            },
        ],
        ..BackdropDescriptor::default()
    });
    value.effects = vec![LayerEffect::ColorMatrix([
        0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0,
    ])];
    let program = compile_layer(&value).unwrap();
    let first = program.instantiate(&context()).unwrap();
    let second = program.instantiate(&context()).unwrap();
    assert_eq!(first.passes(), second.passes());
    assert_eq!(first.resources(), second.resources());
    assert_eq!(first.slots(), second.slots());
    let backdrop_output = first.passes()[1].output;
    let layer_output = first.passes()[3].output;
    assert_eq!(
        first.resources()[backdrop_output.index()].slot,
        first.resources()[layer_output.index()].slot
    );
}

#[test]
fn scale_rounding_limits_and_context_validation_are_enforced() {
    let mut ctx = context();
    ctx.parent_destination_bounds = rect_bounds(0.25, 0.25, 10.1, 5.1);
    ctx.layer_content_bounds = rect_bounds(0.25, 0.25, 10.1, 5.1);
    ctx.scale_factor = 2.0;
    let plan = compile_layer(&descriptor())
        .unwrap()
        .instantiate(&ctx)
        .unwrap();
    let destination = &plan.resources()[plan.parent_destination().index()];
    assert_eq!(destination.physical_bounds.width, 21);
    assert_eq!(destination.physical_bounds.height, 11);
    ctx.limits.max_texture_dimension_2d = 20;
    assert!(matches!(
        compile_layer(&descriptor()).unwrap().instantiate(&ctx),
        Err(PlanError::TextureTooLarge { .. })
    ));
    ctx.limits.max_texture_dimension_2d = 100;
    ctx.scale_factor = 0.0;
    assert!(matches!(
        compile_layer(&descriptor()).unwrap().instantiate(&ctx),
        Err(PlanError::InvalidContext {
            field: "scale_factor",
            ..
        })
    ));
}

#[test]
fn dynamic_instance_does_not_change_program_fingerprint() {
    let program = compile_layer(&descriptor()).unwrap();
    let mut first_context = context();
    let mut second_context = context();
    second_context.composite.opacity = 0.5;
    second_context.composite.transform = Affine::translate(10.0, 20.0);
    let _ = program.instantiate(&first_context).unwrap();
    let fingerprint = program.fingerprint();
    let _ = program.instantiate(&second_context).unwrap();
    assert_eq!(fingerprint, program.fingerprint());
    first_context.external_aliasing = ExternalAliasing::BackdropAndDestination;
    let _ = program.instantiate(&first_context).unwrap();
    assert_eq!(fingerprint, program.fingerprint());
}

#[test]
fn randomized_chains_have_valid_non_overlapping_slot_lifetimes() {
    let mut state = 0x1234_5678_u32;
    for _ in 0..64 {
        let mut value = descriptor();
        value.backdrop = Some(BackdropDescriptor::default());
        for _ in 0..10 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let amount = (state & 0xff) as f32 / 48.0;
            value
                .backdrop
                .as_mut()
                .unwrap()
                .filters
                .push(match state % 4 {
                    0 => BackdropFilter::Brightness(amount),
                    1 => BackdropFilter::Blur {
                        sigma_x: amount,
                        sigma_y: amount * 0.5,
                        quality: FilterQuality::Low,
                    },
                    2 => BackdropFilter::Pixelate {
                        size: Size::new(amount + 1.0, amount + 2.0),
                    },
                    _ => BackdropFilter::ChromaticAberration {
                        offset: [amount, -amount],
                    },
                });
        }
        let plan = compile_layer(&value)
            .unwrap()
            .instantiate(&context())
            .unwrap();
        for slot in plan.slots() {
            for (left_index, left) in slot.resources.iter().enumerate() {
                for right in slot.resources.iter().skip(left_index + 1) {
                    let left = &plan.resources()[left.index()];
                    let right = &plan.resources()[right.index()];
                    let left_first = left.producer.unwrap().index();
                    let left_last = left.last_read.unwrap_or(left.producer.unwrap()).index();
                    let right_first = right.producer.unwrap().index();
                    let right_last = right.last_read.unwrap_or(right.producer.unwrap()).index();
                    assert!(left_last < right_first || right_last < left_first);
                }
            }
        }
    }
}

#[test]
fn a_gaussian_is_cut_so_no_pass_reads_more_than_the_limit() {
    use xui_render_graph::{MAX_PASS_RADIUS_PX, gaussian_stages};
    let support = 3.0;
    for sigma in [0.5, 4.0, 40.0, 42.0, 43.0, 60.0, 100.0, 400.0, 2_000.0] {
        let (stages, stage_sigma) = gaussian_stages(sigma, support);
        assert!(stages >= 1);
        // Each pass is short enough to run...
        assert!(
            stage_sigma * support <= MAX_PASS_RADIUS_PX as f32 + 1e-3,
            "sigma {sigma}: {stages} passes of {stage_sigma} still read {}",
            stage_sigma * support
        );
        // ...and together they are the blur asked for: variances add.
        let total = (stages as f32 * stage_sigma * stage_sigma).sqrt();
        assert!((total - sigma).abs() < sigma * 1e-4, "sigma {sigma} became {total}");
    }
    // A blur that fits is one pass, untouched.
    assert_eq!(gaussian_stages(10.0, 3.0), (1, 10.0));
}

#[test]
fn a_spread_is_cut_into_chunks_that_add_up() {
    use xui_render_graph::{MAX_PASS_RADIUS_PX, spread_chunks};
    for radius in [0.0, 1.0, 5.5, 128.0, 129.0, 300.0, 1_000.0] {
        let chunks = spread_chunks(radius);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| *c <= MAX_PASS_RADIUS_PX));
        assert_eq!(chunks.iter().sum::<u32>(), radius.ceil() as u32, "radius {radius}");
    }
}

#[test]
fn refraction_bends_around_the_layer_however_its_output_is_cropped() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Refraction {
                strength: 10.0,
                chromatic_aberration: 2.0,
            }],
            ..BackdropDescriptor::default()
        }),
        ..descriptor()
    };
    let program = compile_layer(&value).unwrap();
    let center_with = |backdrop_bounds: Bounds, transform: Affine| {
        let plan = program
            .instantiate(&LayerPlanContext {
                backdrop_bounds: Some(backdrop_bounds),
                composite: CompositeInstance {
                    transform,
                    ..CompositeInstance::default()
                },
                ..context()
            })
            .unwrap();
        plan.passes()
            .iter()
            .find_map(|pass| match pass.op {
                PassOp::Refraction { center, .. } => Some(center),
                _ => None,
            })
            .unwrap()
    };
    // Content is 20..60 x 30..60: the middle is (40, 45), wherever the
    // backdrop is cut, as a clip or a tile would.
    let whole = center_with(rect_bounds(20.0, 30.0, 40.0, 30.0), Affine::IDENTITY);
    let left = center_with(rect_bounds(20.0, 30.0, 10.0, 30.0), Affine::IDENTITY);
    assert_eq!(whole, Point::new(40.0, 45.0));
    assert_eq!(left, whole);
    // The backdrop is in parent space, so the layer's placement moves it.
    let moved = center_with(rect_bounds(0.0, 0.0, 200.0, 150.0), Affine::translate(30.0, 5.0));
    assert_eq!(moved, Point::new(70.0, 50.0));
}

#[test]
fn an_aliased_backdrop_is_copied_before_it_is_composited_onto_itself() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            opacity: 0.5,
            ..BackdropDescriptor::default()
        }),
        ..descriptor()
    };
    let program = compile_layer(&value).unwrap();
    for (aliasing, copies) in [
        (ExternalAliasing::Distinct, 0),
        (ExternalAliasing::BackdropAndDestination, 1),
    ] {
        let plan = program
            .instantiate_entry(
                LayerProgramEntry::BackdropOnly,
                &LayerPlanContext {
                    external_aliasing: aliasing,
                    ..context()
                },
            )
            .unwrap();
        let backdrop = plan.backdrop().unwrap();
        let composite = plan
            .passes()
            .iter()
            .find(|pass| matches!(pass.op, PassOp::BackdropComposite { .. }))
            .unwrap();
        assert_eq!(composite.output, plan.parent_destination());
        assert_eq!(
            composite.inputs.contains(&backdrop),
            copies == 0,
            "{aliasing:?}"
        );
        let copy_count = plan
            .passes()
            .iter()
            .filter(|pass| matches!(pass.op, PassOp::Copy) && pass.inputs[0] == backdrop)
            .count();
        assert_eq!(copy_count, copies, "{aliasing:?}");
    }
}

#[test]
fn a_filtered_aliased_backdrop_needs_no_copy() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Blur {
                sigma_x: 2.0,
                sigma_y: 2.0,
                quality: FilterQuality::Medium,
            }],
            ..BackdropDescriptor::default()
        }),
        ..descriptor()
    };
    let plan = compile_layer(&value)
        .unwrap()
        .instantiate(&LayerPlanContext {
            external_aliasing: ExternalAliasing::BackdropAndDestination,
            ..context()
        })
        .unwrap();
    assert!(!plan.passes().iter().any(|pass| matches!(pass.op, PassOp::Copy)));
    // Every read of the backdrop happens before the destination is written.
    let backdrop = plan.backdrop().unwrap();
    let first_write = plan
        .passes()
        .iter()
        .position(|pass| pass.output == plan.parent_destination())
        .unwrap();
    let last_read = plan
        .passes()
        .iter()
        .rposition(|pass| pass.inputs.contains(&backdrop))
        .unwrap();
    assert!(last_read < first_write);
}
