//! Backend-agnostic retained scene, compiled scene, and built frame interfaces.

pub mod backend;
pub mod built;
pub mod compiled_scene;
pub mod compiler;
mod destination;
pub mod effect;
pub mod frame_builder;
pub mod geometry;
pub mod layer;
pub mod patch;
// pub mod plan;
pub mod primitive;
pub mod properties;
pub mod render_graph;
pub mod scene;
mod writer;

use slotmap::new_key_type;

new_key_type! {
    pub struct RenderNodeId;
    pub struct LayerCacheKey;
    pub struct PictureId;
    pub struct PrimitiveId;
    pub struct SpatialNodeId;
    pub struct CompiledClipId;
}

#[derive(Debug, Clone, PartialEq)]
pub enum SceneError {
    MissingNode(RenderNodeId),
    AlreadyHasParent(RenderNodeId),
    DuplicateChild {
        parent: RenderNodeId,
        child: RenderNodeId,
    },
    CycleDetected {
        parent: RenderNodeId,
        child: RenderNodeId,
    },
    CannotRemoveRoot,
    NodeCannotHaveChildren(RenderNodeId),
    UseSingleChildApi(RenderNodeId),
    UseGroupChildrenApi(RenderNodeId),
    WrongNodeKind {
        node: RenderNodeId,
        expected: &'static str,
    },
    InvalidChildIndex {
        parent: RenderNodeId,
        index: usize,
        len: usize,
    },
    ChildNotFound {
        parent: RenderNodeId,
        child: RenderNodeId,
    },
    InvalidHostBinding {
        field: &'static str,
        node: RenderNodeId,
    },
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingNode(node) => write!(f, "render node {node:?} does not exist"),
            Self::AlreadyHasParent(node) => {
                write!(f, "render node {node:?} already has a parent")
            }
            Self::DuplicateChild { parent, child } => {
                write!(f, "render node {child:?} is already a child of {parent:?}")
            }
            Self::CycleDetected { parent, child } => write!(
                f,
                "attaching render node {child:?} to {parent:?} would create a cycle"
            ),
            Self::CannotRemoveRoot => write!(f, "cannot remove the render scene root"),
            Self::NodeCannotHaveChildren(node) => {
                write!(f, "primitive render node {node:?} cannot have children")
            }
            Self::UseSingleChildApi(node) => {
                write!(f, "render node {node:?} accepts one child; use set_child")
            }
            Self::UseGroupChildrenApi(node) => write!(
                f,
                "render node {node:?} accepts multiple children; use a group child API"
            ),
            Self::WrongNodeKind { node, expected } => {
                write!(f, "render node {node:?} is not a {expected} node")
            }
            Self::InvalidChildIndex { parent, index, len } => write!(
                f,
                "child index {index} is invalid for render node {parent:?} with {len} children"
            ),
            Self::ChildNotFound { parent, child } => {
                write!(f, "render node {child:?} is not a child of {parent:?}")
            }
            Self::InvalidHostBinding { field, node } => write!(
                f,
                "host render binding field {field} refers to missing node {node:?}"
            ),
        }
    }
}

impl std::error::Error for SceneError {}

pub use backend::*;
pub use built::*;
pub use compiled_scene::*;
pub use compiler::*;
pub use effect::*;
pub use frame_builder::*;
pub use geometry::*;
pub use layer::*;
pub use patch::*;
pub use primitive::*;
pub use properties::*;
pub use scene::*;
pub(crate) use writer::*;
