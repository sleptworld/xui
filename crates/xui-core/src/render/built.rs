use super::{
    BackdropIsolation, CachePolicy, ClipShape, ContentVersion, ImagePrimitive, LayerCacheKey,
    RenderNodeId, ShapePrimitive, TextPrimitive, VectorPrimitive,
};
use crate::render::render_graph::BuiltLayerProgram;
use xui_interface::{Affine, Bounds};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltLayerId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltLayerInstanceId(pub usize);

/// A logical version of one surface after its first `item_count` items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfacePrefix {
    pub layer: BuiltLayerId,
    pub item_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompositePrefixId(pub usize);

/// A persistent logical destination value assembled from one or more surface prefixes.
///
/// This is scene state, not a GPU texture or render-graph resource allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompositePrefix {
    pub parent: Option<CompositePrefixId>,
    pub local: SurfacePrefix,
    pub placement: Option<BuiltLayerInstanceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltClipChainId(pub usize);

#[derive(Debug, Clone)]
pub struct BuiltFrame {
    pub root_layer: BuiltLayerId,
    pub layers: Vec<BuiltLayer>,
    pub layer_instances: Vec<BuiltLayerInstance>,
    pub composite_prefixes: Vec<CompositePrefix>,
    pub clip_chains: Vec<BuiltClipChain>,
    pub live_layer_caches: Vec<LayerCacheId>,
    pub scene_revision: u64,
    pub properties_revision: u64,
}

impl BuiltFrame {
    pub(crate) fn next_layer_id(&self) -> BuiltLayerId {
        BuiltLayerId(self.layers.len())
    }

    pub fn layer_instance(&self, id: BuiltLayerInstanceId) -> Option<&BuiltLayerInstance> {
        self.layer_instances.get(id.0)
    }

    pub fn composite_prefix(&self, id: CompositePrefixId) -> Option<&CompositePrefix> {
        self.composite_prefixes.get(id.0)
    }

    /// Resolve a logical surface version to the exact ordered item slice it denotes.
    pub fn surface_prefix_items(&self, prefix: SurfacePrefix) -> Option<&[BuiltItem]> {
        self.layers
            .get(prefix.layer.0)?
            .items
            .get(..prefix.item_count)
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
    pub content_bounds: Bounds,
    pub render_bounds: Bounds,
    pub content_version: ContentVersion,
    pub cache_id: Option<LayerCacheId>,
    pub cache_policy: CachePolicy,
    pub backdrop_isolation: BackdropIsolation,
    pub items: Vec<BuiltItem>,
}

#[derive(Debug, Clone)]
pub enum BuiltItem {
    Draw(BuiltDraw),
    Layer(BuiltLayerInstanceId),
}

#[derive(Debug, Clone)]
pub struct BuiltLayerInstance {
    pub source: RenderNodeId,
    pub layer: BuiltLayerId,
    /// Dynamic values excluded from the static program fingerprint.
    pub composite: xui_render_graph::CompositeInstance,
    /// Reusable static IR paired with scene-owned external mask resources.
    pub render_program: BuiltLayerProgram,
    pub clip_chain: Option<BuiltClipChainId>,
    pub world_bounds: Bounds,
    pub placement_version: PlacementVersion,
    /// Logical destination observed before this operation starts.
    ///
    /// Present only when the static program declares an explicit destination read.
    pub destination_prefix: Option<CompositePrefixId>,
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
    pub world_bounds: Bounds,
}

#[derive(Debug, Clone)]
pub struct BuiltDrawData {
    pub source: RenderNodeId,
    pub content_version: ContentVersion,
    pub world_transform: Affine,
    pub world_bounds: Bounds,
    pub clip_chain: Option<BuiltClipChainId>,
}

#[derive(Debug, Clone)]
pub enum BuiltDraw {
    Shape(BuiltShape),
    Vector(BuiltVector),
    Image(BuiltImage),
    Text(BuiltText),
}

impl BuiltDraw {
    pub fn common(&self) -> &BuiltDrawData {
        match self {
            Self::Shape(value) => &value.common,
            Self::Vector(value) => &value.common,
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
pub struct BuiltVector {
    pub common: BuiltDrawData,
    pub primitive: VectorPrimitive,
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
