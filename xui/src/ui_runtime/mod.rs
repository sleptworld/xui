//! Modular host UI runtime.

use crate::fiber::Key;
use crate::render::{
    BuiltFrame, DirtySnapshot, FrameBuildError, FramePropertiesSnapshot, SceneCompileError,
};
use crate::widgets::{WidgetI, WidgetType};
use interaction::InteractionSystem;
use layout::{LayoutNode, LayoutTree, WidgetContext};
use render::RenderSystem;
use state::UiState;
use style::StyleSystem;
use tree::{HostData, HostTree};
use xui_interface::core::Bounds;
use xui_interface::{
    ComputedStyle, NodeId, NodeLifecycleEvent, Point, Rect, Size, Theme, WidgetState,
};

pub(crate) mod interaction;
pub(crate) mod layout;
pub(crate) mod render;
pub(crate) mod state;
pub(crate) mod style;
#[path = "host_tree.rs"]
pub(crate) mod tree;

mod pipeline;

/// Cross-subsystem owner. Node identity lives only in `HostTree`; every other
/// subsystem stores its own `NodeId`-keyed data.
pub struct UiRuntime {
    pub(crate) hosts: HostTree<HostData>,
    pub(crate) layout_tree: LayoutTree<WidgetContext>,
    pub(crate) root: NodeId,
    pub(crate) root_overlayer: NodeId,
    pub(crate) node_lifecycle_events: Vec<NodeLifecycleEvent>,
    pub(crate) interaction_system: InteractionSystem,
    pub(crate) theme: Theme,
    pub update_visits: usize,
    pub layout_passes: usize,
    pub repaint_passes: usize,
    pub(crate) style_system: StyleSystem,
    pub(crate) ui_state: UiState,
    pub(crate) render_system: RenderSystem,
}

/// A read-only projection assembled from host and layout subsystem storage.
#[derive(Clone, Copy)]
pub struct NodeView<'a> {
    pub id: NodeId,
    pub node_type: WidgetType,
    pub key: Option<&'a Key>,
    pub layout: Bounds,
    pub previous_layout: Bounds,
    pub world_origin: Point,
    pub content_size: Size<f32>,
    pub scroll_offset: Point,
    pub old_props_hash: u64,
    pub new_props_hash: u64,
    pub target_style: &'a ComputedStyle,
    pub effective_style: &'a ComputedStyle,
    pub state: WidgetState,
    pub widget: &'a WidgetI,
}

impl<'a> NodeView<'a> {
    pub(crate) fn new(
        id: NodeId,
        host: &'a HostData,
        layout: &LayoutNode,
        target_style: &'a ComputedStyle,
        effective_style: &'a ComputedStyle,
    ) -> Self {
        Self {
            id,
            node_type: host.node_type,
            key: host.key.as_ref(),
            layout: layout.layout,
            previous_layout: layout.previous_layout,
            world_origin: layout.world_origin,
            content_size: layout.content_size,
            scroll_offset: layout.scroll_offset,
            old_props_hash: host.old_props_hash,
            new_props_hash: host.new_props_hash,
            target_style,
            effective_style,
            state: host.state,
            widget: &host.widget,
        }
    }
}

pub struct RenderFrame {
    pub built: BuiltFrame,
    pub dirty_snapshot: DirtySnapshot,
    pub properties_snapshot: FramePropertiesSnapshot,
    pub viewport: Bounds,
}

#[derive(Debug)]
pub enum RenderFrameError {
    Compile(SceneCompileError),
    Build(FrameBuildError),
}

impl std::fmt::Display for RenderFrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(error) => error.fmt(formatter),
            Self::Build(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RenderFrameError {}

impl From<SceneCompileError> for RenderFrameError {
    fn from(value: SceneCompileError) -> Self {
        Self::Compile(value)
    }
}

impl From<FrameBuildError> for RenderFrameError {
    fn from(value: FrameBuildError) -> Self {
        Self::Build(value)
    }
}
