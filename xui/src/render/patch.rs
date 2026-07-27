use super::{ClipShape, CompositeStyle, LayerDescriptor, Primitive, RenderNodeId};
use xui_interface::Affine;

/// Owned, thread-transferable scene mutation. Applying a sequence is ordered
/// and fail-fast; `apply_patches` is intentionally not transactional.
#[derive(Debug, Clone)]
pub enum ScenePatch {
    SetVisible {
        node: RenderNodeId,
        visible: bool,
    },
    UpdatePrimitive {
        node: RenderNodeId,
        primitive: Primitive,
    },
    UpdateTransform {
        node: RenderNodeId,
        transform: Affine,
    },
    UpdateClip {
        node: RenderNodeId,
        clip: ClipShape,
    },
    UpdateLayerComposite {
        node: RenderNodeId,
        composite: CompositeStyle,
    },
    UpdateLayer {
        node: RenderNodeId,
        descriptor: LayerDescriptor,
    },
    RemoveSubtree {
        node: RenderNodeId,
    },
}
