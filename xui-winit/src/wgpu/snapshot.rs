use std::sync::Arc;
use xui::{
    Affine,
    render::{BackdropEffect, BuiltFrame, BuiltItem, ContentVersion, LayerEffect, RenderNodeId},
};
use xui_interface::Rect;

use crate::wgpu::layer::LayerItemVersion;

#[derive(Debug, Clone)]
pub(super) struct LayerSnapshot {
    pub content_version: ContentVersion,
    pub render_bounds: Rect,
    pub effects: Arc<[LayerEffect]>,
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
            BuiltItem::Layer(instance) => LayerItemSnapshot {
                source: instance.source,
                version: LayerItemVersion {
                    content: frame.layers[instance.layer.0].content_version,
                    placement: instance.placement_version,
                },
                bounds: instance.world_bounds,
                kind: LayerItemKind::Layer {
                    source: frame.layers[instance.layer.0].source,
                    transform: instance.composite.transform,
                    backdrop_expansion: instance
                        .backdrop_effects
                        .iter()
                        .map(BackdropEffect::sampling_expansion)
                        .reduce(f32::max),
                },
            },
        })
        .collect();

    LayerSnapshot {
        content_version: layer.content_version,
        render_bounds: layer.render_bounds,
        effects: Arc::clone(&layer.effects),
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum LayerItemKind {
    Draw,
    Layer {
        source: RenderNodeId,
        transform: Affine,
        backdrop_expansion: Option<f32>,
    },
}
