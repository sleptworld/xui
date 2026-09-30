//! Modular host UI runtime.

use crate::fiber::Key;
use crate::render::{
    BuiltFrame, DirtySnapshot, FrameBuildError, FramePropertiesSnapshot, SceneCompileError,
};
use crate::widgets::{WidgetI, WidgetType};
use anchor::AnchorSystem;
use interaction::InteractionSystem;
use layout::{LayoutNode, LayoutTree, WidgetContext};
use render::RenderSystem;
use slotmap::SparseSecondaryMap;
use state::UiState;
use style::StyleSystem;
use tree::{HostData, HostTree};
use xui_interface::core::Bounds;
use xui_interface::{ComputedStyle, NodeId, NodeLifecycleEvent, Point, Size, Theme, WidgetState};

pub(crate) mod anchor;
pub(crate) mod interaction;
pub(crate) mod layout;
pub(crate) mod render;
pub(crate) mod state;
pub(crate) mod style;
#[path = "host_tree.rs"]
pub(crate) mod tree;

// The runtime's API, one file per caller.
mod commit;
mod config;
mod frame;
mod input;
mod query;
mod scroll;

// The pipeline's private stages.
mod layout_pass;
mod paint_pass;
mod style_pass;
mod work;

#[cfg(test)]
mod tests;

/// Cross-subsystem owner. Node identity lives only in `HostTree`; every other
/// subsystem stores its own `NodeId`-keyed data.
///
/// Outside this crate it is read-only: [`Self::root`], [`Self::children`],
/// [`Self::parent`], [`Self::node`], [`Self::contains`] and
/// [`Self::hit_test`]. Inside, its API is split by caller -- `commit` for the
/// fiber commit, `query` and `input` for the event system, `frame` and
/// `config` for `App` -- and the pipeline stages behind them are private to
/// this module.
///
/// # Preconditions
///
/// Unless a method says otherwise, every `NodeId` it takes must be live.
/// Ids that can go stale -- queued work, controller requests, overlay
/// entries -- are filtered once where they are drained, and nowhere else.
pub struct UiRuntime {
    pub(super) hosts: HostTree<HostData>,
    pub(super) layout_tree: LayoutTree<WidgetContext>,
    pub(super) root: NodeId,
    pub(super) root_overlayer: NodeId,
    pub(super) node_lifecycle_events: Vec<NodeLifecycleEvent>,
    pub(super) interaction_system: InteractionSystem,
    /// Index of live `WidgetType::Text` hosts. `HostData::node_type` is fixed at
    /// creation, so membership only changes when a node is created or removed.
    pub(super) text_nodes: SparseSecondaryMap<NodeId, ()>,
    /// Index of live `WidgetType::Canvas` hosts, kept for the same reason as
    /// `text_nodes`: the post-layout pass has to find them without walking the
    /// whole tree.
    pub(super) canvas_nodes: SparseSecondaryMap<NodeId, ()>,
    /// Canvases whose last drawing asked to be drawn again. Kept as a set so
    /// `is_dirty` -- consulted after every batch of events -- stays O(1)
    /// instead of borrowing every canvas widget to ask.
    pub(super) canvases_wanting_repaint: SparseSecondaryMap<NodeId, ()>,
    /// Whether the window is on screen. Only the animation loop consults it:
    /// a canvas that asked for another frame gets none while nothing can be
    /// seen. Defaults to true, so a platform that never reports occlusion
    /// behaves exactly as before.
    pub(super) window_visible: bool,
    /// The time of the frame in progress, published once at the top of it by
    /// [`UiRuntime::begin_frame`]. Read, never sampled: it is the reason two
    /// animations stepped in the same frame step by the same amount.
    pub(super) frame_time: crate::clock::FrameTime,
    /// Repaints requested by a `CanvasController` outside of any rebuild.
    pub(super) canvas_invalidations: crate::widgets::CanvasInvalidator,
    /// Callbacks installed by `use_ticker`, run once per frame. Held here
    /// rather than in the component runtime so that the one place deciding
    /// whether animation asks for frames -- `is_animating` -- can see them.
    pub(super) tickers: crate::ticker::TickerRegistry,
    /// Physical pixels per logical pixel, forwarded to canvas painters so they
    /// can size hairlines and snap to the device grid.
    pub(super) scale_factor: f32,
    /// The renderer's GPU device, when it has one to share. Set once at
    /// startup; a canvas GPU painter draws with it and draws nothing without
    /// it. See [`crate::widgets::CanvasGpuContext`].
    pub(super) gpu_context: Option<crate::widgets::CanvasGpuContext>,
    /// How many live hosts read raw device events. Almost always zero, which is
    /// what lets raw dispatch skip its whole ancestor walk.
    pub(super) raw_event_listeners: usize,
    pub(super) theme: Theme,
    pub(crate) stats: FrameStats,
    pub(super) style_system: StyleSystem,
    pub(super) anchor_system: AnchorSystem,
    pub(super) ui_state: UiState,
    pub(super) render_system: RenderSystem,
}

/// A node and its ancestors, root first, as dispatch walks them. Inline up to
/// a depth real trees rarely exceed, so building one per event allocates
/// nothing.
pub(crate) type EventPath = smallvec::SmallVec<[NodeId; 16]>;

/// Work counters, for tests and benchmarks to assert on.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameStats {
    /// Nodes whose state or style was recomputed in the last `update_tree`.
    pub update_visits: usize,
    /// Nodes whose sampled inheritance was re-resolved in the last
    /// `update_tree` or `tick_style_animations`, whichever ran later.
    pub inheritance_visits: usize,
    pub layout_passes: usize,
    pub repaint_passes: usize,
    /// Paint transforms re-derived, over the runtime's lifetime.
    pub transform_syncs: usize,
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
