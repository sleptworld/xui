use super::{
    ClipShape, CompiledClipId, ContentVersion, LayerDescriptor, PictureId, Primitive, PrimitiveId,
    RenderNodeId, SpatialNodeId,
};
use crate::render::render_graph::BuiltLayerProgram;
use slotmap::{SecondaryMap, SlotMap};
use std::collections::HashMap;
use xui_interface::Affine;

#[derive(Debug, Clone)]
pub struct CompiledScene {
    pub(crate) root_picture: PictureId,
    pub(crate) root_spatial: SpatialNodeId,
    pub(crate) pictures: SlotMap<PictureId, CompiledPicture>,
    pub(crate) primitives: SlotMap<PrimitiveId, CompiledPrimitive>,
    pub(crate) spatial_nodes: SlotMap<SpatialNodeId, CompiledSpatialNode>,
    pub(crate) clips: SlotMap<CompiledClipId, CompiledClip>,
    pub(crate) picture_by_source: HashMap<RenderNodeId, PictureId>,
    pub(crate) primitive_by_source: HashMap<RenderNodeId, PrimitiveId>,
    pub(crate) spatial_by_source: HashMap<RenderNodeId, SpatialNodeId>,
    pub(crate) clip_by_source: HashMap<RenderNodeId, CompiledClipId>,
    /// Source-scene metadata, stamped with `metadata_epoch` instead of being
    /// cleared: a structural rebuild bumps the epoch and restamps what it
    /// visits, so retiring the previous contents costs nothing.
    pub(crate) source_epoch: SecondaryMap<RenderNodeId, u64>,
    pub(crate) layer_isolation: SecondaryMap<RenderNodeId, (u64, bool)>,
    pub(crate) metadata_epoch: u64,
    pub(crate) scene_revision: u64,
}

impl CompiledScene {
    pub fn root_picture(&self) -> PictureId {
        self.root_picture
    }

    pub fn root_spatial(&self) -> SpatialNodeId {
        self.root_spatial
    }

    pub fn scene_revision(&self) -> u64 {
        self.scene_revision
    }

    pub fn picture(&self, id: PictureId) -> Option<&CompiledPicture> {
        self.pictures.get(id)
    }

    pub fn primitive(&self, id: PrimitiveId) -> Option<&CompiledPrimitive> {
        self.primitives.get(id)
    }

    pub fn spatial_node(&self, id: SpatialNodeId) -> Option<&CompiledSpatialNode> {
        self.spatial_nodes.get(id)
    }

    pub fn clip(&self, id: CompiledClipId) -> Option<&CompiledClip> {
        self.clips.get(id)
    }

    pub fn picture_for_source(&self, source: RenderNodeId) -> Option<PictureId> {
        self.picture_by_source.get(&source).copied()
    }

    pub fn primitive_for_source(&self, source: RenderNodeId) -> Option<PrimitiveId> {
        self.primitive_by_source.get(&source).copied()
    }

    pub fn spatial_for_source(&self, source: RenderNodeId) -> Option<SpatialNodeId> {
        self.spatial_by_source.get(&source).copied()
    }

    pub fn clip_for_source(&self, source: RenderNodeId) -> Option<CompiledClipId> {
        self.clip_by_source.get(&source).copied()
    }

    pub fn contains_source(&self, source: RenderNodeId) -> bool {
        self.source_epoch.get(source) == Some(&self.metadata_epoch)
    }

    pub(crate) fn layer_isolation(&self, source: RenderNodeId) -> Option<bool> {
        self.layer_isolation
            .get(source)
            .filter(|(epoch, _)| *epoch == self.metadata_epoch)
            .map(|(_, isolated)| *isolated)
    }
}

#[derive(Debug, Clone)]
pub struct CompiledPicture {
    pub source: RenderNodeId,
    pub items: Vec<CompiledPictureItem>,
    pub descriptor: LayerDescriptor,
    /// Static render-graph IR and scene-owned mask bindings. Root pictures do
    /// not need a layer composite.
    pub render_program: Option<BuiltLayerProgram>,
    pub placement_spatial: SpatialNodeId,
    pub placement_clip: Option<CompiledClipId>,
    pub content_version: ContentVersion,
    pub composite_version: u64,
    pub is_root: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompiledPictureItem {
    Primitive(PrimitiveId),
    Picture(PictureId),
}

#[derive(Debug, Clone)]
pub struct CompiledPrimitive {
    pub source: RenderNodeId,
    pub primitive: Primitive,
    pub spatial: SpatialNodeId,
    pub clip: Option<CompiledClipId>,
    pub content_version: ContentVersion,
}

#[derive(Debug, Clone, Copy)]
pub struct CompiledSpatialNode {
    pub source: RenderNodeId,
    pub parent: Option<SpatialNodeId>,
    pub local_transform: Affine,
    pub content_version: ContentVersion,
}

#[derive(Debug, Clone)]
pub struct CompiledClip {
    pub source: RenderNodeId,
    pub parent: Option<CompiledClipId>,
    pub spatial: SpatialNodeId,
    pub clip: ClipShape,
    pub content_version: ContentVersion,
}
