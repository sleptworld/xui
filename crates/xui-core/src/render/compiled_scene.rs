use super::{
    ClipShape, CompiledClipId, ContentVersion, LayerDescriptor, PictureId, Primitive, PrimitiveId,
    RenderNodeId, SpatialNodeId,
};
use crate::render::render_graph::BuiltLayerProgram;
use slotmap::{SecondaryMap, SlotMap};
use xui_interface::{Affine, core::Bounds};

/// The render scene compiled into pictures, primitives, spatial nodes and
/// clips.
///
/// Each kind lives densely in its own map, in the order the walk made them,
/// which is what the frame builder iterates every frame. `nodes` maps a source
/// node to what it compiled to; the node's kind fixes which entity that is, so
/// it holds at most one.
#[derive(Debug, Clone)]
pub struct CompiledScene {
    pub(crate) root_picture: PictureId,
    pub(crate) root_spatial: SpatialNodeId,
    pub(crate) pictures: SlotMap<PictureId, CompiledPicture>,
    pub(crate) primitives: SlotMap<PrimitiveId, CompiledPrimitive>,
    pub(crate) spatial_nodes: SlotMap<SpatialNodeId, CompiledSpatialNode>,
    pub(crate) clips: SlotMap<CompiledClipId, CompiledClip>,
    pub(crate) nodes: SecondaryMap<RenderNodeId, CompiledNode>,
    /// Fresh whenever an existing spatial node's parent, local transform or
    /// content version changes, which is all a world transform depends on
    /// besides frame properties; see [`fresh_token`](super::fresh_token).
    /// Nodes made since need no new one: nothing can have cached them.
    pub(crate) spatial_token: u64,
    /// Bumped by every structural walk; see [`NodeItemLayout::generation`].
    pub(crate) walk_generation: u64,
    pub(crate) scene_revision: u64,
    /// Item buffers a structural walk replaced, kept so the next one fills
    /// them instead of growing new ones.
    pub(crate) spare_items: Vec<Vec<CompiledPictureItem>>,
    pub(crate) spare_spans: Vec<Vec<CompiledItemSpan>>,
}

/// What one source node compiled to.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CompiledNode {
    /// Where the last walk put the node's items; `None` until it is walked.
    pub layout: Option<NodeItemLayout>,
    pub entity: CompiledEntity,
}

/// The entity a node compiled to. Holding one here and holding its slot in
/// the entity's map go together: each is only ever set or cleared with the
/// other.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum CompiledEntity {
    /// A group, a layer that does not isolate, or a node not compiled yet. The
    /// root, a group, holds the root picture.
    #[default]
    None,
    Primitive(PrimitiveId),
    Spatial(SpatialNodeId),
    Clip(CompiledClipId),
    Picture(PictureId),
}

/// What the last walk of one source node contributed to its picture.
///
/// A node's items are contiguous in pre-order, so `offset` and `count` locate
/// them relative to its parent's first item; an isolated layer's child starts
/// its own picture, at offset 0.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NodeItemLayout {
    pub parent: Option<RenderNodeId>,
    pub offset: u32,
    pub count: u32,
    /// The subtree's paint bounds in the space of the spatial node it sits in.
    pub local_bounds: Option<Bounds>,
    /// False once the subtree holds an isolated picture, whose effects may
    /// paint outside any recorded bounds.
    pub boundable: bool,
    /// Cleared, along with every ancestor's, when an in-place update moves
    /// geometry under the node: its recorded bounds may then be too small.
    pub bounds_valid: bool,
    /// The walk that last placed this node under `parent`.
    pub generation: u64,
    /// The walk that last placed this node's children. A child's layout only
    /// describes the current items when its `generation` matches this.
    pub children_generation: u64,
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
        match self.nodes.get(source)?.entity {
            CompiledEntity::Picture(id) => Some(id),
            _ => None,
        }
    }

    pub fn primitive_for_source(&self, source: RenderNodeId) -> Option<PrimitiveId> {
        match self.nodes.get(source)?.entity {
            CompiledEntity::Primitive(id) => Some(id),
            _ => None,
        }
    }

    pub fn spatial_for_source(&self, source: RenderNodeId) -> Option<SpatialNodeId> {
        match self.nodes.get(source)?.entity {
            CompiledEntity::Spatial(id) => Some(id),
            _ => None,
        }
    }

    pub(crate) fn entity(&self, source: RenderNodeId) -> CompiledEntity {
        self.nodes
            .get(source)
            .map_or(CompiledEntity::None, |node| node.entity)
    }

    pub(crate) fn layout(&self, source: RenderNodeId) -> Option<&NodeItemLayout> {
        self.nodes.get(source)?.layout.as_ref()
    }

    /// The node's slot, made on first use.
    pub(crate) fn node_mut(&mut self, source: RenderNodeId) -> &mut CompiledNode {
        self.nodes
            .entry(source)
            .expect("compiled node is live in the scene")
            .or_default()
    }

    pub fn clip_for_source(&self, source: RenderNodeId) -> Option<CompiledClipId> {
        match self.nodes.get(source)?.entity {
            CompiledEntity::Clip(id) => Some(id),
            _ => None,
        }
    }
}

/// A contiguous run of `CompiledPicture::items` that all sit under one spatial
/// node, bounded by that node's
/// [`subtree_bounds`](CompiledSpatialNode::subtree_bounds).
///
/// Spans let the frame builder reject a whole subtree with a single bounds
/// test instead of resolving and transforming every primitive inside it. They
/// are stored pre-order (an enclosing span precedes the spans nested in it) and
/// sorted by `start`, and they are advisory: dropping one only costs culling
/// precision, never correctness.
#[derive(Debug, Clone, Copy)]
pub struct CompiledItemSpan {
    pub spatial: SpatialNodeId,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone)]
pub struct CompiledPicture {
    pub source: RenderNodeId,
    pub items: Vec<CompiledPictureItem>,
    /// Culling index over `items`; see [`CompiledItemSpan`].
    pub spans: Vec<CompiledItemSpan>,
    pub descriptor: LayerDescriptor,
    /// Static render-graph IR and scene-owned mask bindings. Root pictures do
    /// not need a layer composite.
    pub render_program: Option<BuiltLayerProgram>,
    pub placement_spatial: SpatialNodeId,
    pub placement_clip: Option<CompiledClipId>,
    pub content_version: ContentVersion,
    pub composite_version: u64,
    /// The layer's own effects; see [`RenderEpochs::effect`](super::RenderEpochs::effect).
    pub effect_version: u64,
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
    /// Number of spatial ancestors; the root spatial node is at 0.
    pub depth: u32,
    pub local_transform: Affine,
    pub content_version: ContentVersion,
    /// The paint bounds of everything under the node, in its local space,
    /// i.e. before its own transform: a dynamic transform on the node leaves
    /// them correct, one on a descendant does not, which the builder checks
    /// for. `None` when the subtree is empty, or holds an isolated picture
    /// whose effects may paint anywhere.
    ///
    /// A walk records them exactly; an in-place geometry update only grows
    /// them to cover what moved, so they may be larger than needed until the
    /// node is walked again.
    pub subtree_bounds: Option<Bounds>,
}

#[derive(Debug, Clone)]
pub struct CompiledClip {
    pub source: RenderNodeId,
    pub parent: Option<CompiledClipId>,
    pub spatial: SpatialNodeId,
    pub clip: ClipShape,
    pub content_version: ContentVersion,
}
