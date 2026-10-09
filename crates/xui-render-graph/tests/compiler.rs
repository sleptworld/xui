use xui_interface::{Affine, Bounds, Color, ImageData, ImageKey, Point, Size};
use xui_render_graph::{
    BackdropDescriptor, BackdropFilter, BlendMode, CompileError, CompositeDescriptor,
    CompositeOperator, ExternalResourceKind, FilterQuality, LayerEffect, LayerGraphDescriptor,
    Mask, MaskShape, ProgramOp, WorkingColorSpace, compile_layer,
};

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
fn backdrop_external_is_declared_only_when_used() {
    let plain = compile_layer(&descriptor()).unwrap();
    assert_eq!(plain.backdrop(), None);
    assert_eq!(
        plain.external_resource(ExternalResourceKind::Backdrop),
        None
    );

    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor::default());
    let backdrop = compile_layer(&value).unwrap();
    assert_eq!(
        backdrop.backdrop(),
        backdrop.external_resource(ExternalResourceKind::Backdrop)
    );
    assert!(backdrop.backdrop().is_some());
}

#[test]
fn direct_replacement_api_builds_one_static_layer_program() {
    let value = LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Brightness(0.8)],
            opacity: 0.7,
            blend_mode: BlendMode::Screen,
            mask: Mask::None,
        }),
        effects: vec![LayerEffect::Blur {
            sigma_x: 2.0,
            sigma_y: 3.0,
            quality: FilterQuality::Medium,
        }],
        composite: CompositeDescriptor {
            blend_mode: BlendMode::Overlay,
            operator: CompositeOperator::SrcOver,
        },
        working_color_space: WorkingColorSpace::LinearScene,
    };
    let program = compile_layer(&value).unwrap();
    let ops: Vec<_> = program.nodes().iter().map(|node| &node.op).collect();
    assert!(
        ops.iter()
            .any(|op| matches!(op, ProgramOp::BackdropComposite { .. }))
    );
    assert!(ops.iter().any(|op| matches!(op, ProgramOp::Blur { .. })));
    assert!(matches!(
        ops.last().unwrap(),
        ProgramOp::LayerComposite { .. }
    ));
}

#[test]
fn consecutive_color_filters_are_fused_in_order() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        filters: vec![
            BackdropFilter::Brightness(0.5),
            BackdropFilter::Contrast(2.0),
            BackdropFilter::Blur {
                sigma_x: 1.0,
                sigma_y: 0.0,
                quality: FilterQuality::Low,
            },
        ],
        ..BackdropDescriptor::default()
    });
    value.effects = vec![
        LayerEffect::ColorMatrix([
            0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ]),
        LayerEffect::ColorMatrix([
            3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ]),
    ];
    let program = compile_layer(&value).unwrap();
    assert_eq!(
        program
            .nodes()
            .iter()
            .filter(|node| matches!(node.op, ProgramOp::ColorMatrix(_)))
            .count(),
        2
    );
    let layer_matrix = program
        .nodes()
        .iter()
        .rev()
        .find_map(|node| match node.op {
            ProgramOp::ColorMatrix(matrix) => Some(matrix),
            _ => None,
        })
        .unwrap();
    assert_eq!(layer_matrix[0], 1.5);
}

#[test]
fn noop_operations_disappear_but_layer_composite_remains() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        filters: vec![
            BackdropFilter::Blur {
                sigma_x: -1.0,
                sigma_y: 0.0,
                quality: FilterQuality::High,
            },
            BackdropFilter::Brightness(1.0),
            BackdropFilter::HueRotate(std::f32::consts::TAU),
        ],
        opacity: 0.0,
        ..BackdropDescriptor::default()
    });
    value.effects = vec![LayerEffect::Blur {
        sigma_x: 0.0,
        sigma_y: -2.0,
        quality: FilterQuality::Low,
    }];
    let program = compile_layer(&value).unwrap();
    assert_eq!(program.nodes().len(), 1);
    assert!(matches!(
        program.nodes()[0].op,
        ProgramOp::LayerComposite { .. }
    ));
}

#[test]
fn texture_masks_receive_stable_external_ordinals() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        mask: Mask::AlphaTexture {
            texture: ImageKey::UserProvided(1),
            transform: Affine::IDENTITY,
        },
        ..BackdropDescriptor::default()
    });
    value.effects = vec![
        image_mask(2, rect_bounds(0.0, 0.0, 1.0, 1.0)),
        LayerEffect::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
            quality: FilterQuality::Low,
        },
        image_mask(3, rect_bounds(2.0, 3.0, 1.0, 1.0)),
    ];
    let program = compile_layer(&value).unwrap();
    assert!(
        program
            .external_resource(ExternalResourceKind::BackdropMask)
            .is_some()
    );
    assert!(
        program
            .external_resource(ExternalResourceKind::LayerMask(0))
            .is_some()
    );
    assert!(
        program
            .external_resource(ExternalResourceKind::LayerMask(1))
            .is_some()
    );
    let ordinals: Vec<_> = program
        .nodes()
        .iter()
        .filter_map(|node| match node.op {
            ProgramOp::ApplyMask { ordinal, .. } => Some(ordinal),
            _ => None,
        })
        .collect();
    assert_eq!(ordinals, [0, 1]);
}

#[test]
fn masks_are_normalized_and_must_be_invertible() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        mask: Mask::Shape {
            shape: MaskShape::RoundedRect(-4.0),
            transform: Affine::IDENTITY,
        },
        ..BackdropDescriptor::default()
    });
    let program = compile_layer(&value).unwrap();
    assert!(matches!(
        program.nodes()[0].op,
        ProgramOp::BackdropComposite {
            mask: xui_render_graph::MaskProgram::Shape {
                shape: MaskShape::RoundedRect(0.0),
                ..
            },
            ..
        }
    ));

    value
        .effects
        .push(image_mask(4, rect_bounds(0.0, 0.0, 0.0, 1.0)));
    assert!(matches!(
        compile_layer(&value),
        Err(CompileError::InvalidStyleParameter {
            field: "layer_mask.transform",
            ..
        })
    ));
}

#[test]
fn invalid_values_and_derived_overflow_are_rejected() {
    let mut value = descriptor();
    value.effects.push(LayerEffect::DropShadow {
        color: Color::BLACK,
        offset: Point::new(f32::NAN, 0.0),
        sigma_x: 1.0,
        sigma_y: 1.0,
        spread: 0.0,
        quality: FilterQuality::Medium,
    });
    assert!(matches!(
        compile_layer(&value),
        Err(CompileError::InvalidEffectParameter {
            index: 0,
            field: "offset.x",
            ..
        })
    ));

    // Two huge matrices used to overflow when fused. The first one clamps, so
    // they are no longer fused at all and nothing is derived from them: the
    // program is just the two matrices, each finite on its own.
    value.effects = vec![
        LayerEffect::ColorMatrix([f32::MAX; 20]),
        LayerEffect::ColorMatrix([f32::MAX; 20]),
    ];
    let program = compile_layer(&value).expect("each matrix is finite by itself");
    assert_eq!(
        program
            .nodes()
            .iter()
            .filter(|node| matches!(node.op, ProgramOp::ColorMatrix(_)))
            .count(),
        2
    );

    value.effects = vec![LayerEffect::Blur {
        sigma_x: f32::MAX,
        sigma_y: 0.0,
        quality: FilterQuality::High,
    }];
    assert!(matches!(
        compile_layer(&value),
        Err(CompileError::InvalidEffectParameter {
            index: 0,
            field: "sampling_expansion",
            ..
        })
    ));
}

#[test]
fn every_backdrop_family_and_drop_shadow_are_represented() {
    let mut value = descriptor();
    value.backdrop = Some(BackdropDescriptor {
        filters: vec![
            BackdropFilter::Blur {
                sigma_x: 1.0,
                sigma_y: 2.0,
                quality: FilterQuality::Medium,
            },
            BackdropFilter::Saturate(0.5),
            BackdropFilter::Brightness(0.8),
            BackdropFilter::Contrast(1.2),
            BackdropFilter::Grayscale(0.2),
            BackdropFilter::Sepia(0.3),
            BackdropFilter::HueRotate(0.4),
            BackdropFilter::Invert(0.1),
            BackdropFilter::ColorMatrix([
                1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
            ]),
            BackdropFilter::Pixelate {
                size: Size::new(4.0, 5.0),
            },
            BackdropFilter::Refraction {
                strength: 2.0,
                chromatic_aberration: 1.0,
            },
            BackdropFilter::ChromaticAberration {
                offset: [1.0, -2.0],
            },
        ],
        ..BackdropDescriptor::default()
    });
    value.effects.push(LayerEffect::DropShadow {
        color: Color::rgba(0.1, 0.2, 0.3, 0.5),
        offset: Point::new(4.0, -3.0),
        sigma_x: 2.0,
        sigma_y: 1.0,
        spread: 2.0,
        quality: FilterQuality::Medium,
    });
    let program = compile_layer(&value).unwrap();
    for predicate in [
        |op: &ProgramOp| matches!(op, ProgramOp::Blur { .. }),
        |op: &ProgramOp| matches!(op, ProgramOp::ColorMatrix(_)),
        |op: &ProgramOp| matches!(op, ProgramOp::Pixelate { .. }),
        |op: &ProgramOp| matches!(op, ProgramOp::Refraction { .. }),
        |op: &ProgramOp| matches!(op, ProgramOp::ChromaticAberration { .. }),
        |op: &ProgramOp| matches!(op, ProgramOp::DropShadow { .. }),
    ] {
        assert!(program.nodes().iter().any(|node| predicate(&node.op)));
    }
}

#[test]
fn program_is_topological_and_fingerprint_is_static_only() {
    let mut value = descriptor();
    value.effects = vec![LayerEffect::Blur {
        sigma_x: 2.0,
        sigma_y: 1.0,
        quality: FilterQuality::Low,
    }];
    let first = compile_layer(&value).unwrap();
    for (node_index, node) in first.nodes().iter().enumerate() {
        assert_eq!(
            first.resources()[node.output.index()]
                .producer
                .unwrap()
                .index(),
            node_index
        );
        for input in node.inputs.iter().copied() {
            if let Some(producer) = first.resources()[input.index()].producer {
                assert!(producer.index() < node_index);
            }
        }
    }
    let same = compile_layer(&value).unwrap();
    assert_eq!(first.fingerprint(), same.fingerprint());
    value.composite.operator = CompositeOperator::DstOver;
    assert_ne!(
        first.fingerprint(),
        compile_layer(&value).unwrap().fingerprint()
    );
    value.composite.operator = CompositeOperator::SrcOver;
    value.composite.blend_mode = BlendMode::ColorDodge;
    assert_ne!(
        first.fingerprint(),
        compile_layer(&value).unwrap().fingerprint()
    );
}

/// One colour through one matrix, clamped, as every executor does it.
fn apply_clamped(matrix: &[f32; 20], colour: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for row in 0..4 {
        let base = row * 5;
        let value = matrix[base + 4]
            + (0..4).map(|column| matrix[base + column] * colour[column]).sum::<f32>();
        out[row] = value.clamp(0.0, 1.0);
    }
    out
}

fn compiled_matrices(effects: &[[f32; 20]]) -> Vec<[f32; 20]> {
    let mut value = descriptor();
    value.effects = effects.iter().copied().map(LayerEffect::ColorMatrix).collect();
    compile_layer(&value)
        .unwrap()
        .nodes()
        .iter()
        .filter_map(|node| match node.op {
            ProgramOp::ColorMatrix(matrix) => Some(matrix),
            _ => None,
        })
        .collect()
}

fn scale(amount: f32) -> [f32; 20] {
    [
        amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
    ]
}

#[test]
fn matrices_that_clamp_in_between_are_not_fused() {
    // x2 clamps 0.8 to 1.0 before x0.5 makes it 0.5. Fused they would be the
    // identity, vanish, and leave 0.8.
    let program = compiled_matrices(&[scale(2.0), scale(0.5)]);
    assert_eq!(program.len(), 2, "the clamp between them must survive");
    let mut colour = [0.8, 0.8, 0.8, 1.0];
    for matrix in &program {
        colour = apply_clamped(matrix, colour);
    }
    assert_eq!(colour, [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn matrices_that_never_clamp_in_between_are_still_fused() {
    // Both only darken, so nothing leaves [0, 1] between them.
    let program = compiled_matrices(&[scale(0.5), scale(0.5)]);
    assert_eq!(program.len(), 1);
    assert_eq!(program[0][0], 0.25);
}

#[test]
fn a_compiled_chain_gives_what_the_effects_give_one_by_one() {
    // Brightening, darkening, contrast, inversion, a channel swap: every
    // ordering of the first three, over a grid of colours.
    let swap = [
        0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0,
    ];
    let contrast = [
        2.0, 0.0, 0.0, 0.0, -0.5, 0.0, 2.0, 0.0, 0.0, -0.5, 0.0, 0.0, 2.0, 0.0, -0.5, 0.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    let invert = [
        -1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    let pool = [scale(2.0), scale(0.5), contrast, invert, swap];
    let levels = [0.0, 0.2, 0.5, 0.8, 1.0];
    let mut checked = 0;
    for a in 0..pool.len() {
        for b in 0..pool.len() {
            for c in 0..pool.len() {
                let chain = [pool[a], pool[b], pool[c]];
                let compiled = compiled_matrices(&chain);
                for &r in &levels {
                    for &g in &levels {
                        for &bl in &levels {
                            let start = [r, g, bl, 1.0];
                            let written = chain.iter().fold(start, |c, m| apply_clamped(m, c));
                            let run = compiled.iter().fold(start, |c, m| apply_clamped(m, c));
                            for channel in 0..4 {
                                assert!(
                                    (written[channel] - run[channel]).abs() < 1e-4,
                                    "chain {a}{b}{c} on {start:?}: written {written:?}, compiled {run:?}"
                                );
                            }
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 10_000);
}

fn glass_with_effects(effects: Vec<LayerEffect>) -> LayerGraphDescriptor {
    LayerGraphDescriptor {
        backdrop: Some(BackdropDescriptor {
            filters: vec![BackdropFilter::Blur {
                sigma_x: 3.0,
                sigma_y: 3.0,
                quality: FilterQuality::Medium,
            }],
            mask: Mask::AlphaTexture {
                texture: ImageKey::UserProvided(7),
                transform: Affine::scale(10.0, 10.0),
            },
            ..BackdropDescriptor::default()
        }),
        effects,
        composite: CompositeDescriptor {
            blend_mode: BlendMode::Multiply,
            operator: CompositeOperator::SrcOver,
        },
        ..descriptor()
    }
}

#[test]
fn cutting_the_layer_branch_keeps_the_backdrop_and_composites_the_content() {
    let program = compile_layer(&glass_with_effects(vec![
        LayerEffect::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
            quality: FilterQuality::Medium,
        },
        image_mask(1, rect_bounds(0.0, 0.0, 10.0, 10.0)),
    ]))
    .unwrap();
    assert!(program.has_layer_effects());
    let cut = program.without_layer_effects();
    assert!(!cut.has_layer_effects());

    let ops: Vec<_> = cut.nodes().iter().map(|node| &node.op).collect();
    assert!(matches!(ops[0], ProgramOp::Blur { .. }), "{ops:?}");
    assert!(matches!(ops[1], ProgramOp::BackdropComposite { .. }), "{ops:?}");
    assert!(
        matches!(
            ops[2],
            ProgramOp::LayerComposite {
                blend_mode: BlendMode::Multiply,
                ..
            }
        ),
        "{ops:?}"
    );
    assert_eq!(ops.len(), 3);

    // The composite reads the content and composites onto the destination.
    let composite = &cut.nodes()[2];
    assert_eq!(composite.inputs[0], cut.layer_content());
    assert_eq!(composite.inputs[1], cut.parent_destination());
    // The backdrop mask is still the backdrop mask; the layer mask is gone.
    let ProgramOp::BackdropComposite { mask, .. } = &cut.nodes()[1].op else {
        unreachable!()
    };
    let xui_render_graph::MaskProgram::Texture { resource, .. } = mask else {
        panic!("{mask:?}")
    };
    assert_eq!(
        Some(*resource),
        cut.external_resource(ExternalResourceKind::BackdropMask)
    );
    assert_eq!(cut.external_resource(ExternalResourceKind::LayerMask(0)), None);
    // Every resource's producer is the node that writes it.
    for (index, node) in cut.nodes().iter().enumerate() {
        assert_eq!(cut.resources()[node.output.index()].producer.unwrap().index(), index);
    }
    assert_eq!(cut.backdrop_input_expansion(), program.backdrop_input_expansion());
    assert_eq!(cut.layer_visual_expansion(), xui_render_graph::SampleExpansion::ZERO);
}

#[test]
fn programs_that_differ_only_in_layer_effects_cut_to_the_same_program() {
    let blur = |sigma| LayerEffect::Blur {
        sigma_x: sigma,
        sigma_y: sigma,
        quality: FilterQuality::Medium,
    };
    let a = compile_layer(&glass_with_effects(vec![blur(2.0)])).unwrap();
    let b = compile_layer(&glass_with_effects(vec![blur(9.0), blur(1.0)])).unwrap();
    let none = compile_layer(&glass_with_effects(vec![])).unwrap();
    assert_ne!(a.fingerprint(), b.fingerprint());
    assert_eq!(
        a.without_layer_effects().fingerprint(),
        b.without_layer_effects().fingerprint()
    );
    // A program with no layer branch is what it cuts to.
    assert!(!none.has_layer_effects());
    assert_eq!(none.without_layer_effects().fingerprint(), none.fingerprint());
}
