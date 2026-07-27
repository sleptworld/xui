use super::{BuiltFrame, BuiltItem, BuiltLayerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPlanOp {
    BeginLayer(BuiltLayerId),
    Draw {
        layer: BuiltLayerId,
        item: usize,
    },
    ContentEffect {
        layer: BuiltLayerId,
        effect: usize,
    },
    BackdropEffect {
        parent: BuiltLayerId,
        item: usize,
        effect: usize,
    },
    Composite {
        parent: BuiltLayerId,
        item: usize,
    },
    EndLayer(BuiltLayerId),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderPlan {
    pub ops: Vec<RenderPlanOp>,
}

impl RenderPlan {
    pub fn build(frame: &BuiltFrame) -> Self {
        let mut plan = Self::default();
        plan.push_layer(frame, frame.root_layer);
        plan
    }

    fn push_layer(&mut self, frame: &BuiltFrame, layer_id: BuiltLayerId) {
        self.ops.push(RenderPlanOp::BeginLayer(layer_id));
        let layer = &frame.layers[layer_id.0];
        for (item_index, item) in layer.items.iter().enumerate() {
            match item {
                BuiltItem::Draw(_) => self.ops.push(RenderPlanOp::Draw {
                    layer: layer_id,
                    item: item_index,
                }),
                BuiltItem::Layer(instance) => {
                    self.push_layer(frame, instance.layer);
                    for effect in 0..instance.backdrop_effects.len() {
                        self.ops.push(RenderPlanOp::BackdropEffect {
                            parent: layer_id,
                            item: item_index,
                            effect,
                        });
                    }
                    self.ops.push(RenderPlanOp::Composite {
                        parent: layer_id,
                        item: item_index,
                    });
                }
            }
        }
        for effect in 0..layer.effects.len() {
            self.ops.push(RenderPlanOp::ContentEffect {
                layer: layer_id,
                effect,
            });
        }
        self.ops.push(RenderPlanOp::EndLayer(layer_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::*;
    use std::sync::Arc;
    use xui_interface::{Affine, Rect};

    #[test]
    fn nested_layer_effects_and_composite_keep_scene_order() {
        let mut scene = RenderScene::new();
        let before = scene.insert_primitive(Primitive::Shape(ShapePrimitive {
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            shape: Shape::Rect,
            fill: None,
            stroke: None,
            shadow: None,
        }));
        let layer = scene.insert_layer(LayerDescriptor {
            effects: Arc::from([LayerEffect::Blur { sigma: 2.0 }]),
            backdrop_effects: Arc::from([BackdropEffect::Blur { sigma: 3.0 }]),
            composite: CompositeStyle {
                opacity: 0.5,
                transform: Affine::IDENTITY,
                blend_mode: BlendMode::Multiply,
            },
            ..LayerDescriptor::default()
        });
        let content = scene.insert_primitive(Primitive::Shape(ShapePrimitive {
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            shape: Shape::Rect,
            fill: None,
            stroke: None,
            shadow: None,
        }));
        scene.append_child(scene.root(), before).unwrap();
        scene.append_child(scene.root(), layer).unwrap();
        scene.set_child(layer, Some(content)).unwrap();
        let snapshot = scene.dirty_snapshot();
        let mut compiler = SceneCompiler::new();
        let compiled = compiler.compile(&scene, &snapshot).unwrap();
        let frame = FrameBuilder::new()
            .build(
                compiled,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                &FrameProperties::default(),
            )
            .unwrap();
        let plan = RenderPlan::build(&frame);
        assert!(matches!(
            plan.ops[1],
            RenderPlanOp::Draw {
                layer: BuiltLayerId(0),
                ..
            }
        ));
        assert!(
            plan.ops
                .iter()
                .any(|op| matches!(op, RenderPlanOp::ContentEffect { .. }))
        );
        let backdrop = plan
            .ops
            .iter()
            .position(|op| matches!(op, RenderPlanOp::BackdropEffect { .. }))
            .unwrap();
        let composite = plan
            .ops
            .iter()
            .position(|op| matches!(op, RenderPlanOp::Composite { .. }))
            .unwrap();
        assert!(backdrop < composite);
    }
}
