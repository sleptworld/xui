use std::sync::Arc;

use super::{
    BackdropEffect, CachePolicy, ClipShape, CompositeStyle, ContentVersion, ImagePrimitive,
    LayerCacheKey, LayerEffect, PathPrimitive, RenderNodeId, ShapePrimitive, TextPrimitive,
};
use xui_interface::{Affine, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltLayerId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltClipChainId(pub usize);

#[derive(Debug, Clone)]
pub struct BuiltFrame {
    pub root_layer: BuiltLayerId,
    pub layers: Vec<BuiltLayer>,
    pub clip_chains: Vec<BuiltClipChain>,
    pub live_layer_caches: Vec<LayerCacheId>,
    pub scene_revision: u64,
    pub properties_revision: u64,
}

impl BuiltFrame {
    pub(crate) fn next_layer_id(&self) -> BuiltLayerId {
        BuiltLayerId(self.layers.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerCacheId {
    Explicit(LayerCacheKey),
    Scene(RenderNodeId),
}

#[derive(Debug, Clone)]
pub struct BuiltLayer {
    pub source: RenderNodeId,
    pub content_bounds: Rect,
    pub render_bounds: Rect,
    pub content_version: ContentVersion,
    pub cache_id: Option<LayerCacheId>,
    pub cache_policy: CachePolicy,
    pub items: Vec<BuiltItem>,
    pub effects: Arc<[LayerEffect]>,
}

#[derive(Debug, Clone)]
pub enum BuiltItem {
    Draw(BuiltDraw),
    Layer(BuiltLayerInstance),
}

#[derive(Debug, Clone)]
pub struct BuiltLayerInstance {
    pub source: RenderNodeId,
    pub layer: BuiltLayerId,
    pub composite: CompositeStyle,
    pub backdrop_effects: Arc<[BackdropEffect]>,
    pub clip_chain: Option<BuiltClipChainId>,
    pub world_bounds: Rect,
    pub placement_version: PlacementVersion,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PlacementVersion {
    pub scene: u64,
    pub dynamic: u64,
}

#[derive(Debug, Clone)]
pub struct BuiltClipChain {
    pub source: RenderNodeId,
    pub parent: Option<BuiltClipChainId>,
    pub clip: ClipShape,
    pub world_transform: Affine,
    pub world_bounds: Rect,
}

#[derive(Debug, Clone)]
pub struct BuiltDrawData {
    pub source: RenderNodeId,
    pub content_version: ContentVersion,
    pub world_transform: Affine,
    pub world_bounds: Rect,
    pub clip_chain: Option<BuiltClipChainId>,
}

#[derive(Debug, Clone)]
pub enum BuiltDraw {
    Shape(BuiltShape),
    Path(BuiltPath),
    Image(BuiltImage),
    Text(BuiltText),
}

impl BuiltDraw {
    pub fn common(&self) -> &BuiltDrawData {
        match self {
            Self::Shape(value) => &value.common,
            Self::Path(value) => &value.common,
            Self::Image(value) => &value.common,
            Self::Text(value) => &value.common,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BuiltShape {
    pub common: BuiltDrawData,
    pub primitive: ShapePrimitive,
}

#[derive(Debug, Clone)]
pub struct BuiltPath {
    pub common: BuiltDrawData,
    pub primitive: PathPrimitive,
}

#[derive(Debug, Clone)]
pub struct BuiltImage {
    pub common: BuiltDrawData,
    pub primitive: ImagePrimitive,
}

#[derive(Debug, Clone)]
pub struct BuiltText {
    pub common: BuiltDrawData,
    pub primitive: TextPrimitive,
}
