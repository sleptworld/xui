use xui::{
    Affine,
    render::{BuiltFrame, BuiltItem, RenderNodeId},
};
use xui_interface::Rect;
use xui_render_graph::{ProgramFingerprint, SampleExpansion};

use crate::wgpu::layer::LayerItemVersion;

#[derive(Debug, Clone)]
pub(super) struct LayerSnapshot {
    pub render_bounds: Rect,
    pub items: Vec<LayerItemSnapshot>,
}

pub(super) fn layer_snapshot(frame: &BuiltFrame, layer: &xui::render::BuiltLayer) -> LayerSnapshot {
    let items = layer
        .items
        .iter()
        .map(|item| match item {
            BuiltItem::Draw(draw) => {
                let common = draw.common();
                LayerItemSnapshot {
                    source: common.source,
                    version: LayerItemVersion {
                        content: common.content_version,
                        placement: Default::default(),
                    },
                    bounds: common.world_bounds,
                    kind: LayerItemKind::Draw,
                }
            }
            BuiltItem::Layer(instance_id) => {
                let instance = frame.layer_instance(*instance_id).expect("built instance");
                LayerItemSnapshot {
                    source: instance.source,
                    version: LayerItemVersion {
                        content: frame.layers[instance.layer.0].content_version,
                        placement: instance.placement_version,
                    },
                    bounds: instance.world_bounds,
                    kind: LayerItemKind::Layer {
                        source: frame.layers[instance.layer.0].source,
                        transform: instance.composite.transform,
                        program: instance.render_program.program().fingerprint(),
                        backdrop_expansion: instance
                            .render_program
                            .program()
                            .backdrop_input_expansion(),
                    },
                }
            }
        })
        .collect();

    LayerSnapshot {
        render_bounds: layer.render_bounds,
        items,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LayerItemSnapshot {
    pub source: RenderNodeId,
    pub version: LayerItemVersion,
    pub bounds: Rect,
    pub kind: LayerItemKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum LayerItemKind {
    Draw,
    Layer {
        source: RenderNodeId,
        transform: Affine,
        program: ProgramFingerprint,
        backdrop_expansion: SampleExpansion,
    },
}
