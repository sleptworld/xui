use xui::render::{BuiltFrame, BuiltLayerInstanceId, CompositePrefixId, SurfacePrefix};
use xui_render_graph::LayerProgramEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompositePrefixPlan {
    pub steps: Box<[CompositePrefixStep]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompositePrefixStep {
    /// Replay exactly `SurfacePrefix.item_count` items from this surface.
    Replay(SurfacePrefix),
    /// Cross a placement while evaluating only its ancestor-facing backdrop
    /// branch. Layer content is replayed by the following surface prefix.
    TraversePlacement {
        instance: BuiltLayerInstanceId,
        entry: LayerProgramEntry,
    },
}

impl CompositePrefixPlan {
    pub fn from_tail(frame: &BuiltFrame, tail: CompositePrefixId) -> Option<Self> {
        let mut chain = Vec::new();
        let mut current = Some(tail);
        while let Some(id) = current {
            let node = *frame.composite_prefix(id)?;
            if node.local.item_count > frame.layers.get(node.local.layer.0)?.items.len() {
                return None;
            }
            chain.push(node);
            current = node.parent;
        }
        chain.reverse();

        let mut steps = Vec::with_capacity(chain.len() * 2);
        for node in chain {
            if let Some(instance) = node.placement {
                frame.layer_instance(instance)?;
                steps.push(CompositePrefixStep::TraversePlacement {
                    instance,
                    entry: LayerProgramEntry::BackdropOnly,
                });
            }
            steps.push(CompositePrefixStep::Replay(node.local));
        }
        Some(Self {
            steps: steps.into_boxed_slice(),
        })
    }

    pub fn tail(&self) -> Option<SurfacePrefix> {
        self.steps.iter().rev().find_map(|step| match step {
            CompositePrefixStep::Replay(prefix) => Some(*prefix),
            CompositePrefixStep::TraversePlacement { .. } => None,
        })
    }

    pub fn crosses_surface(&self) -> bool {
        self.steps
            .iter()
            .any(|step| matches!(step, CompositePrefixStep::TraversePlacement { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use xui::render::{
        FrameProperties, LayerDescriptor, Primitive, RenderScene, SceneCompiler, Shape,
        ShapePrimitive, frame_builder::FrameBuilder,
    };
    use xui_interface::{
        Affine, Color, ComputedBackdropMask, ComputedBackdropStyle, ComputedColorStyle, Rect,
    };

    fn append_shape(scene: &mut RenderScene, parent: xui::render::RenderNodeId, rect: Rect) {
        let shape = scene.insert_primitive(Primitive::Shape(ShapePrimitive {
            bounds: rect,
            shape: Shape::Rect,
            fill: Some(ComputedColorStyle::Solid(Color::BLACK)),
            stroke: None,
            shadow: None,
        }));
        scene.append_child(parent, shape).unwrap();
    }

    fn append_layer(
        scene: &mut RenderScene,
        parent: xui::render::RenderNodeId,
        descriptor: LayerDescriptor,
    ) -> (xui::render::RenderNodeId, xui::render::RenderNodeId) {
        let layer = scene.insert_layer(descriptor);
        let contents = scene.insert_group();
        scene.append_child(parent, layer).unwrap();
        scene.set_child(layer, Some(contents)).unwrap();
        (layer, contents)
    }

    #[test]
    fn cross_surface_prefix_keeps_exact_slices_and_backdrop_only_traversal() {
        let mut scene = RenderScene::new();
        let root = scene.root();
        append_shape(&mut scene, root, Rect::new(0.0, 0.0, 20.0, 20.0));
        let (outer, outer_contents) = append_layer(
            &mut scene,
            root,
            LayerDescriptor {
                force_offscreen: true,
                ..LayerDescriptor::default()
            },
        );
        append_shape(&mut scene, outer_contents, Rect::new(0.0, 0.0, 20.0, 20.0));
        let (inner, inner_contents) = append_layer(
            &mut scene,
            outer_contents,
            LayerDescriptor {
                backdrop_style: Some(ComputedBackdropStyle {
                    filters: Arc::from([]),
                    opacity: 1.0,
                    blend_mode: xui_interface::BlendMode::Normal,
                    mask: ComputedBackdropMask::Shape {
                        shape: xui_interface::ComputedMaskShape::Rect,
                        transform: Affine::IDENTITY,
                    },
                }),
                ..LayerDescriptor::default()
            },
        );
        append_shape(&mut scene, inner_contents, Rect::new(0.0, 0.0, 10.0, 10.0));

        let mut compiler = SceneCompiler::new();
        let compiled = compiler
            .compile(&scene, &scene.dirty_snapshot())
            .unwrap()
            .clone();
        let mut frame = FrameBuilder::new()
            .build(
                &compiled,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                &FrameProperties::default(),
            )
            .unwrap();
        let inner_instance = frame
            .layer_instances
            .iter()
            .find(|instance| instance.source == inner)
            .unwrap();
        let plan =
            CompositePrefixPlan::from_tail(&frame, inner_instance.destination_prefix.unwrap())
                .unwrap();
        assert!(plan.crosses_surface());
        assert!(plan.steps.iter().any(|step| matches!(
            step,
            CompositePrefixStep::TraversePlacement {
                entry: LayerProgramEntry::BackdropOnly,
                ..
            }
        )));
        let replays: Vec<_> = plan
            .steps
            .iter()
            .filter_map(|step| match step {
                CompositePrefixStep::Replay(prefix) => Some(*prefix),
                CompositePrefixStep::TraversePlacement { .. } => None,
            })
            .collect();
        assert_eq!(replays.len(), 2);
        assert_eq!(replays[0].item_count, 1);
        assert_eq!(replays[1].item_count, 1);
        let placement = plan
            .steps
            .iter()
            .find_map(|step| match step {
                CompositePrefixStep::TraversePlacement { instance, .. } => Some(*instance),
                CompositePrefixStep::Replay(_) => None,
            })
            .unwrap();
        frame.layer_instances[placement.0].composite.transform =
            Affine::new(2.0, 0.0, 0.0, 3.0, 10.0, 20.0);
        let demands = crate::wgpu::composite_prefix_stage_demands(
            &frame,
            &plan,
            Rect::new(1.0, 2.0, 3.0, 4.0),
        )
        .unwrap();
        assert_eq!(demands.len(), 2);
        assert_eq!(demands[0].demand, Rect::new(12.0, 26.0, 6.0, 12.0));
        assert_eq!(demands[1].demand, Rect::new(1.0, 2.0, 3.0, 4.0));
        assert!(
            frame
                .layer_instances
                .iter()
                .any(|instance| instance.source == outer)
        );
    }
}
