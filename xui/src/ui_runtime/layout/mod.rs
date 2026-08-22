use std::ops::{Deref, DerefMut};

use slotmap::SecondaryMap;
use taffy as tf;
use xui_interface::{core::Bounds, NodeId, Point, Size};

pub(crate) enum WidgetContext {
    Text(NodeId),
    Image(Size<f32>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LayoutNode {
    pub taffy_node: tf::NodeId,
    pub layout: Bounds,
    pub previous_layout: Bounds,
    pub world_origin: Point,
    pub content_size: Size<f32>,
    pub scroll_offset: Point,
}

impl LayoutNode {
    fn new(taffy_node: tf::NodeId) -> Self {
        Self {
            taffy_node,
            layout: Bounds::ZERO,
            previous_layout: Bounds::ZERO,
            world_origin: Point::zero(),
            content_size: Size::<f32>::ZERO,
            scroll_offset: Point::zero(),
        }
    }

    #[inline(always)]
    pub(crate) fn visual_bounds(&self) -> Bounds {
        Bounds::from_origin_size(self.world_origin - self.scroll_offset, self.layout.size())
    }
}

/// Taffy engine and its host-keyed node cache.
pub(crate) struct LayoutTree<C> {
    engine: tf::TaffyTree<C>,
    nodes: SecondaryMap<NodeId, LayoutNode>,
}

impl<C> LayoutTree<C> {
    pub fn new() -> Self {
        Self {
            engine: tf::TaffyTree::new(),
            nodes: SecondaryMap::new(),
        }
    }

    pub fn create_host(&mut self, host: NodeId, style: tf::Style) -> tf::NodeId {
        let taffy_node = self
            .engine
            .new_leaf(style)
            .expect("failed to create taffy node");
        self.nodes.insert(host, LayoutNode::new(taffy_node));
        taffy_node
    }

    pub fn node_id(&self, host: NodeId) -> tf::NodeId {
        self.nodes[host].taffy_node
    }

    pub(crate) fn host(&self, host: NodeId) -> Option<&LayoutNode> {
        self.nodes.get(host)
    }

    pub(crate) fn host_mut(&mut self, host: NodeId) -> Option<&mut LayoutNode> {
        self.nodes.get_mut(host)
    }

    pub fn contains_host(&self, host: NodeId) -> bool {
        self.nodes.contains_key(host)
    }

    pub fn remove_host(&mut self, host: NodeId) {
        if let Some(node) = self.nodes.remove(host) {
            let _ = self.engine.remove(node.taffy_node);
        }
    }
}

impl<C> Default for LayoutTree<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> Deref for LayoutTree<C> {
    type Target = tf::TaffyTree<C>;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

impl<C> DerefMut for LayoutTree<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.engine
    }
}
