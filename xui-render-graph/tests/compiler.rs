use xui_interface::{Affine, Color, ImageData, ImageKey, Point, Rect, Size};
use xui_render_graph::{
    BackdropDescriptor, BackdropFilter, BlendMode, CompileError, CompositeDescriptor,
    CompositeOperator, ExternalResourceKind, FilterQuality, LayerEffect, LayerGraphDescriptor,
    Mask, MaskShape, ProgramOp, WorkingColorSpace, compile_layer,
};

fn descriptor() -> LayerGraphDescriptor {
    LayerGraphDescriptor::default()
}

fn image_mask(key: u64, bounds: Rect) -> LayerEffect {
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
        image_mask(2, Rect::new(0.0, 0.0, 1.0, 1.0)),
        LayerEffect::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
            quality: FilterQuality::Low,
        },
        image_mask(3, Rect::new(2.0, 3.0, 1.0, 1.0)),
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
        .push(image_mask(4, Rect::new(0.0, 0.0, 0.0, 1.0)));
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

    value.effects = vec![
        LayerEffect::ColorMatrix([f32::MAX; 20]),
        LayerEffect::ColorMatrix([f32::MAX; 20]),
    ];
    assert!(matches!(
        compile_layer(&value),
        Err(CompileError::InvalidEffectParameter {
            index: 1,
            field: "matrix",
            ..
        })
    ));

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
