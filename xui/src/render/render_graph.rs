//! Conversion boundary between xui's retained layer styles and render-graph IR.
//!
//! Static style is compiled by `SceneCompiler`. Dynamic composite opacity and
//! transform remain frame data and are deliberately excluded from fingerprints.

use std::sync::Arc;

pub use xui_render_graph::*;

use super::{CompositeStyle, LayerDescriptor};
use xui_interface::{
    ComputedBackdropMask, ComputedBackdropStyle, ComputedEffect, ImageData, ImageKey,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ImageResource {
    Key(ImageKey),
    Data { key: ImageKey, data: ImageData },
}

pub type BuiltLayerProgram = xui_render_graph::BoundLayerProgram<ImageResource>;

impl LayerDescriptor {
    /// Convert retained style into the backend-independent static graph input.
    pub fn render_graph_descriptor(&self) -> LayerGraphDescriptor {
        LayerGraphDescriptor {
            backdrop: self
                .backdrop_style
                .as_ref()
                .map(|style| BackdropDescriptor {
                    filters: style.filters.to_vec(),
                    opacity: style.opacity,
                    blend_mode: style.blend_mode,
                    mask: style.mask.clone(),
                }),
            effects: self.effects.to_vec(),
            composite: CompositeDescriptor {
                blend_mode: self.composite.blend_mode,
                operator: self.composite.operator,
            },
            working_color_space: WorkingColorSpace::LinearScene,
        }
    }

    /// Compile the static style into a reusable program.
    pub fn compile_render_program(&self) -> Result<LayerProgram, CompileError> {
        compile_layer(&self.render_graph_descriptor())
    }

    /// Collect scene-owned mask resources in the same stable order used by
    /// `compile_render_program` when assigning external-resource ordinals.
    pub fn render_graph_bindings(&self) -> Result<ExternalBindings<ImageResource>, BindingError> {
        let backdrop_mask = self.backdrop_style.as_ref().and_then(|style| {
            if let ComputedBackdropMask::AlphaTexture { texture, .. } = &style.mask {
                Some(ImageResource::Key(texture.clone()))
            } else {
                None
            }
        });
        let layer_masks = self.effects.iter().filter_map(|effect| {
            if let ComputedEffect::ImageMask { image, data, .. } = effect {
                Some(ImageResource::Data {
                    key: image.clone(),
                    data: data.clone(),
                })
            } else {
                None
            }
        });

        ExternalBindings::new(backdrop_mask, layer_masks)
    }

    pub fn bind_render_program(
        &self,
        program: Arc<LayerProgram>,
    ) -> Result<BuiltLayerProgram, BindingError> {
        BoundLayerProgram::new(program, self.render_graph_bindings()?)
    }

    pub(crate) fn has_same_render_graph_style(&self, other: &Self) -> bool {
        same_backdrop_program_style(self.backdrop_style.as_ref(), other.backdrop_style.as_ref())
            && (Arc::ptr_eq(&self.effects, &other.effects)
                || same_effect_chain(&self.effects, &other.effects))
            && self.composite.blend_mode == other.composite.blend_mode
            && self.composite.operator == other.composite.operator
    }
}

impl CompositeStyle {
    /// Extract the per-frame values consumed by `LayerProgram::instantiate`.
    pub fn render_graph_instance(self) -> CompositeInstance {
        CompositeInstance {
            opacity: self.opacity,
            transform: self.transform,
        }
    }
}

fn same_backdrop_program_style(
    left: Option<&ComputedBackdropStyle>,
    right: Option<&ComputedBackdropStyle>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            (Arc::ptr_eq(&left.filters, &right.filters) || left.filters == right.filters)
                && left.opacity == right.opacity
                && left.blend_mode == right.blend_mode
                && same_backdrop_mask_program_style(&left.mask, &right.mask)
        }
        _ => false,
    }
}

fn same_backdrop_mask_program_style(
    left: &ComputedBackdropMask,
    right: &ComputedBackdropMask,
) -> bool {
    match (left, right) {
        (ComputedBackdropMask::None, ComputedBackdropMask::None) => true,
        (
            ComputedBackdropMask::Shape {
                shape: left_shape,
                transform: left_transform,
            },
            ComputedBackdropMask::Shape {
                shape: right_shape,
                transform: right_transform,
            },
        ) => left_shape == right_shape && left_transform == right_transform,
        (
            ComputedBackdropMask::AlphaTexture {
                transform: left_transform,
                ..
            },
            ComputedBackdropMask::AlphaTexture {
                transform: right_transform,
                ..
            },
        ) => left_transform == right_transform,
        _ => false,
    }
}

fn same_effect_chain(left: &[ComputedEffect], right: &[ComputedEffect]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (
                    ComputedEffect::ImageMask {
                        bounds: left_bounds,
                        ..
                    },
                    ComputedEffect::ImageMask {
                        bounds: right_bounds,
                        ..
                    },
                ) => left_bounds == right_bounds,
                _ => left == right,
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use xui_interface::{
        Affine, Bounds, Color, ComputedBackdropStyle, ComputedEffect, ImageData, ImageKey, Point,
        Rect, Size,
    };

    #[test]
    fn descriptor_splits_static_program_from_dynamic_composite() {
        let descriptor = LayerDescriptor {
            composite: CompositeStyle {
                opacity: 0.4,
                transform: Affine::translate(3.0, 5.0),
                blend_mode: BlendMode::ColorDodge,
                operator: CompositeOperator::DstOver,
            },
            effects: Arc::from([
                ComputedEffect::Blur {
                    sigma_x: 2.0,
                    sigma_y: 2.0,
                    quality: FilterQuality::Medium,
                },
                ComputedEffect::DropShadow {
                    color: Color::BLACK,
                    offset: Point::new(4.0, -2.0),
                    sigma_x: 3.0,
                    sigma_y: 3.0,
                    spread: 1.0,
                    quality: FilterQuality::Medium,
                },
            ]),
            force_offscreen: true,
            ..LayerDescriptor::default()
        };

        let program = descriptor.compile_render_program().unwrap();
        assert!(
            program
                .nodes()
                .iter()
                .any(|node| matches!(node.op, ProgramOp::DropShadow { .. }))
        );
        let instance = descriptor.composite.render_graph_instance();
        assert_eq!(instance.opacity, 0.4);
        assert_eq!(instance.transform, Affine::translate(3.0, 5.0));
    }

    #[test]
    fn backdrop_and_mask_resources_are_converted() {
        let descriptor = LayerDescriptor {
            backdrop_style: Some(ComputedBackdropStyle {
                filters: Arc::from([BackdropFilter::Blur {
                    sigma_x: 2.0,
                    sigma_y: 4.0,
                    quality: FilterQuality::High,
                }]),
                opacity: 0.75,
                blend_mode: BlendMode::Overlay,
                mask: Mask::Shape {
                    shape: MaskShape::RoundedRect(8.0),
                    transform: Affine::IDENTITY,
                },
            }),
            ..LayerDescriptor::default()
        };
        let program = descriptor.compile_render_program().unwrap();
        assert!(program.nodes().iter().any(|node| matches!(
            node.op,
            ProgramOp::BackdropComposite {
                blend_mode: BlendMode::Overlay,
                ..
            }
        )));
    }

    #[test]
    fn backdrop_and_non_default_operator_require_isolation() {
        let backdrop = LayerDescriptor {
            backdrop_style: Some(ComputedBackdropStyle {
                filters: Arc::from([]),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: Mask::Shape {
                    shape: MaskShape::Rect,
                    transform: Affine::IDENTITY,
                },
            }),
            ..LayerDescriptor::default()
        };
        assert!(backdrop.requires_isolation());

        let operator = LayerDescriptor {
            composite: CompositeStyle {
                operator: CompositeOperator::Src,
                ..CompositeStyle::default()
            },
            ..LayerDescriptor::default()
        };
        assert!(operator.requires_isolation());
    }

    #[test]
    fn invalid_effect_is_reported_by_graph_compiler() {
        let descriptor = LayerDescriptor {
            effects: Arc::from([ComputedEffect::Blur {
                sigma_x: f32::NAN,
                sigma_y: f32::NAN,
                quality: FilterQuality::Medium,
            }]),
            force_offscreen: true,
            ..LayerDescriptor::default()
        };
        assert!(matches!(
            descriptor.compile_render_program(),
            Err(CompileError::InvalidEffectParameter { .. })
        ));
    }

    #[test]
    fn layer_mask_bounds_become_texture_transform() {
        let descriptor = LayerDescriptor {
            effects: Arc::from([ComputedEffect::ImageMask {
                image: ImageKey::UserProvided(7),
                data: ImageData::rgba8(Size::new(1, 1), [255, 255, 255, 255]),
                bounds: Bounds::from_origin_size((10.0, 20.0), (30.0, 40.0)),
            }]),
            ..LayerDescriptor::default()
        };
        let program = descriptor.compile_render_program().unwrap();
        assert!(program.nodes().iter().any(|node| matches!(
            node.op,
            ProgramOp::ApplyMask {
                transform: Affine {
                    xx: 30.0,
                    yy: 40.0,
                    dx: 10.0,
                    dy: 20.0,
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn bindings_preserve_mask_handles_without_affecting_static_style() {
        fn pixels(value: u8) -> ImageData {
            ImageData::rgba8(xui_interface::Size::new(1, 1), vec![value; 4])
        }

        let descriptor = LayerDescriptor {
            backdrop_style: Some(ComputedBackdropStyle {
                filters: Arc::from([]),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: Mask::AlphaTexture {
                    texture: ImageKey::UserProvided(10),
                    transform: Affine::IDENTITY,
                },
            }),
            effects: Arc::from([
                ComputedEffect::ImageMask {
                    image: ImageKey::UserProvided(20),
                    data: pixels(20),
                    bounds: Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
                },
                ComputedEffect::Blur {
                    sigma_x: 2.0,
                    sigma_y: 2.0,
                    quality: FilterQuality::Medium,
                },
                ComputedEffect::ImageMask {
                    image: ImageKey::UserProvided(30),
                    data: pixels(30),
                    bounds: Bounds::from_origin_size((5.0, 5.0), (20.0, 20.0)),
                },
            ]),
            force_offscreen: true,
            ..LayerDescriptor::default()
        };
        let bound = descriptor
            .bind_render_program(Arc::new(descriptor.compile_render_program().unwrap()))
            .unwrap();

        assert!(matches!(
            bound.handle(ExternalResourceKind::BackdropMask),
            Some(ImageResource::Key(ImageKey::UserProvided(10)))
        ));
        assert!(matches!(
            bound.handle(ExternalResourceKind::LayerMask(0)),
            Some(ImageResource::Data {
                key: ImageKey::UserProvided(20),
                ..
            })
        ));
        assert!(matches!(
            bound.handle(ExternalResourceKind::LayerMask(1)),
            Some(ImageResource::Data {
                key: ImageKey::UserProvided(30),
                ..
            })
        ));

        let mut rebound = descriptor.clone();
        rebound.backdrop_style.as_mut().unwrap().mask = Mask::AlphaTexture {
            texture: ImageKey::UserProvided(99),
            transform: Affine::IDENTITY,
        };
        let ComputedEffect::ImageMask { image, data, .. } =
            &mut Arc::make_mut(&mut rebound.effects)[0]
        else {
            panic!()
        };
        *image = ImageKey::UserProvided(98);
        *data = pixels(98);

        assert!(descriptor.has_same_render_graph_style(&rebound));
        assert_eq!(
            descriptor.compile_render_program().unwrap().fingerprint(),
            rebound.compile_render_program().unwrap().fingerprint()
        );
    }

    #[test]
    fn separately_allocated_equal_chains_have_the_same_static_style() {
        let make = || LayerDescriptor {
            backdrop_style: Some(ComputedBackdropStyle {
                filters: Arc::from([BackdropFilter::Contrast(1.25)]),
                opacity: 0.8,
                blend_mode: BlendMode::Screen,
                mask: Mask::None,
            }),
            effects: Arc::from([ComputedEffect::Blur {
                sigma_x: 3.0,
                sigma_y: 4.0,
                quality: FilterQuality::High,
            }]),
            ..LayerDescriptor::default()
        };
        let left = make();
        let right = make();

        assert!(!Arc::ptr_eq(&left.effects, &right.effects));
        assert!(!Arc::ptr_eq(
            &left.backdrop_style.as_ref().unwrap().filters,
            &right.backdrop_style.as_ref().unwrap().filters,
        ));
        assert!(left.has_same_render_graph_style(&right));
    }
}
