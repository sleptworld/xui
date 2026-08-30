use slotmap::{DefaultKey, SecondaryMap, SlotMap};
use taffy as tf;
use taffy::{
    CacheTree, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutPartialTree, RoundTree, TraversePartialTree, TraverseTree,
};
use xui_interface::{NodeId, Point, Size, core::Bounds};

pub(crate) enum WidgetContext {
    Text(NodeId),
    Image(Size<f32>),
}

/// Geometry contributed by a measured leaf. The baseline is relative to the
/// leaf's border-box top edge and belongs to the same measurement as `size`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MeasuredLeaf {
    pub size: tf::Size<f32>,
    pub first_baseline: Option<f32>,
}

impl MeasuredLeaf {
    pub const fn from_size(size: tf::Size<f32>) -> Self {
        Self {
            size,
            first_baseline: None,
        }
    }
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
    pub(crate) fn visual_bounds(&self, ancestor_scroll_offset: Point) -> Bounds {
        Bounds::from_origin_size(
            self.world_origin - ancestor_scroll_offset,
            self.layout.size(),
        )
    }
}

struct PartialNode<C> {
    style: tf::Style,
    context: Option<C>,
    parent: Option<tf::NodeId>,
    children: Vec<tf::NodeId>,
    cache: tf::Cache,
    unrounded_layout: tf::Layout,
    final_layout: tf::Layout,
}

impl<C> PartialNode<C> {
    fn new(style: tf::Style) -> Self {
        Self {
            style,
            context: None,
            parent: None,
            children: Vec::new(),
            cache: tf::Cache::new(),
            unrounded_layout: tf::Layout::with_order(0),
            final_layout: tf::Layout::with_order(0),
        }
    }
}

/// XUI-owned tree storage for Taffy's low-level layout algorithms.
pub(crate) struct LayoutTree<C> {
    partial_nodes: SlotMap<DefaultKey, PartialNode<C>>,
    hosts: SecondaryMap<NodeId, LayoutNode>,
    use_rounding: bool,
}

impl<C> LayoutTree<C> {
    pub fn new() -> Self {
        Self {
            partial_nodes: SlotMap::with_key(),
            hosts: SecondaryMap::new(),
            use_rounding: true,
        }
    }

    pub fn create_host(&mut self, host: NodeId, style: tf::Style) -> tf::NodeId {
        let taffy_node = tf::NodeId::from(self.partial_nodes.insert(PartialNode::new(style)));
        self.hosts.insert(host, LayoutNode::new(taffy_node));
        taffy_node
    }

    pub fn node_id(&self, host: NodeId) -> tf::NodeId {
        self.hosts[host].taffy_node
    }

    pub(crate) fn host(&self, host: NodeId) -> Option<&LayoutNode> {
        self.hosts.get(host)
    }

    pub(crate) fn host_mut(&mut self, host: NodeId) -> Option<&mut LayoutNode> {
        self.hosts.get_mut(host)
    }

    pub fn contains_host(&self, host: NodeId) -> bool {
        self.hosts.contains_key(host)
    }

    pub fn remove_host(&mut self, host: NodeId) {
        let Some(layout_node) = self.hosts.remove(host) else {
            return;
        };
        let node_id = layout_node.taffy_node;
        let Some(node) = self.partial_nodes.remove(key(node_id)) else {
            return;
        };

        if let Some(parent) = node.parent {
            if let Some(parent_node) = self.partial_node_mut(parent) {
                parent_node.children.retain(|child| *child != node_id);
            }
            let _ = self.mark_dirty(parent);
        }
        for child in node.children {
            if let Some(child_node) = self.partial_node_mut(child)
                && child_node.parent == Some(node_id)
            {
                child_node.parent = None;
            }
        }
    }

    pub fn style(&self, node: tf::NodeId) -> tf::TaffyResult<&tf::Style> {
        self.partial_node(node)
            .map(|node| &node.style)
            .ok_or(tf::TaffyError::InvalidInputNode(node))
    }

    pub fn set_style(&mut self, node: tf::NodeId, style: tf::Style) -> tf::TaffyResult<()> {
        self.partial_node_mut(node)
            .ok_or(tf::TaffyError::InvalidInputNode(node))?
            .style = style;
        self.mark_dirty(node)
    }

    pub fn set_node_context(
        &mut self,
        node: tf::NodeId,
        context: Option<C>,
    ) -> tf::TaffyResult<()> {
        self.partial_node_mut(node)
            .ok_or(tf::TaffyError::InvalidInputNode(node))?
            .context = context;
        self.mark_dirty(node)
    }

    pub fn set_children(
        &mut self,
        parent: tf::NodeId,
        children: &[tf::NodeId],
    ) -> tf::TaffyResult<()> {
        if self.partial_node(parent).is_none() {
            return Err(tf::TaffyError::InvalidParentNode(parent));
        }
        if let Some(child) = children
            .iter()
            .copied()
            .find(|child| self.partial_node(*child).is_none())
        {
            return Err(tf::TaffyError::InvalidChildNode(child));
        }

        let old_children = self.partial_node(parent).unwrap().children.clone();
        for child in old_children {
            if let Some(child_node) = self.partial_node_mut(child)
                && child_node.parent == Some(parent)
            {
                child_node.parent = None;
            }
        }

        for child in children.iter().copied() {
            let old_parent = self.partial_node(child).and_then(|node| node.parent);
            if let Some(old_parent) = old_parent.filter(|old| *old != parent) {
                if let Some(old) = self.partial_node_mut(old_parent) {
                    old.children.retain(|candidate| *candidate != child);
                }
                self.mark_dirty(old_parent)?;
            }
            self.partial_node_mut(child).unwrap().parent = Some(parent);
        }

        self.partial_node_mut(parent).unwrap().children = children.to_vec();
        self.mark_dirty(parent)
    }

    pub fn mark_dirty(&mut self, node: tf::NodeId) -> tf::TaffyResult<()> {
        if self.partial_node(node).is_none() {
            return Err(tf::TaffyError::InvalidInputNode(node));
        }
        let mut current = Some(node);
        while let Some(id) = current {
            let partial = self.partial_node_mut(id).unwrap();
            partial.cache.clear();
            current = partial.parent;
        }
        Ok(())
    }

    pub fn layout(&self, node: tf::NodeId) -> tf::TaffyResult<&tf::Layout> {
        let partial = self
            .partial_node(node)
            .ok_or(tf::TaffyError::InvalidInputNode(node))?;
        Ok(if self.use_rounding {
            &partial.final_layout
        } else {
            &partial.unrounded_layout
        })
    }

    pub fn unrounded_layout(&self, node: tf::NodeId) -> &tf::Layout {
        &self
            .partial_node(node)
            .expect("unrounded layout requested for an invalid node")
            .unrounded_layout
    }

    pub fn compute_layout_with_measure<MeasureFunction>(
        &mut self,
        root: tf::NodeId,
        available_space: tf::Size<tf::AvailableSpace>,
        measure_function: MeasureFunction,
    ) -> tf::TaffyResult<()>
    where
        MeasureFunction: FnMut(
            tf::Size<Option<f32>>,
            tf::Size<tf::AvailableSpace>,
            tf::NodeId,
            Option<&mut C>,
            &tf::Style,
        ) -> MeasuredLeaf,
    {
        if self.partial_node(root).is_none() {
            return Err(tf::TaffyError::InvalidInputNode(root));
        }
        let use_rounding = self.use_rounding;
        let mut view = LayoutView {
            tree: self,
            measure_function,
        };
        tf::compute_root_layout(&mut view, root, available_space);
        if use_rounding {
            tf::round_layout(&mut view, root);
        }
        Ok(())
    }

    fn partial_node(&self, node: tf::NodeId) -> Option<&PartialNode<C>> {
        self.partial_nodes.get(key(node))
    }

    fn partial_node_mut(&mut self, node: tf::NodeId) -> Option<&mut PartialNode<C>> {
        self.partial_nodes.get_mut(key(node))
    }
}

impl<C> Default for LayoutTree<C> {
    fn default() -> Self {
        Self::new()
    }
}

fn key(node: tf::NodeId) -> DefaultKey {
    node.into()
}

struct ChildIter<'a>(std::slice::Iter<'a, tf::NodeId>);

impl Iterator for ChildIter<'_> {
    type Item = tf::NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied()
    }
}

struct LayoutView<'a, C, MeasureFunction> {
    tree: &'a mut LayoutTree<C>,
    measure_function: MeasureFunction,
}

impl<C, MeasureFunction> LayoutView<'_, C, MeasureFunction>
where
    MeasureFunction: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    fn compute_child_layout_impl(
        &mut self,
        node_id: tf::NodeId,
        inputs: tf::LayoutInput,
        block_context: Option<&mut tf::BlockContext<'_>>,
    ) -> tf::LayoutOutput {
        if inputs.run_mode == tf::RunMode::PerformHiddenLayout {
            return tf::compute_hidden_layout(self, node_id);
        }

        tf::compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let display = tree.tree.partial_node(node_id).unwrap().style.display;
            let has_children = tree.child_count(node_id) > 0;
            match (display, has_children) {
                (tf::Display::None, _) => tf::compute_hidden_layout(tree, node_id),
                (tf::Display::Block, true) => {
                    tf::compute_block_layout(tree, node_id, inputs, block_context)
                }
                (tf::Display::FlowRoot, true) => {
                    tf::compute_block_layout(tree, node_id, inputs, None)
                }
                (tf::Display::Flex, true) => tf::compute_flexbox_layout(tree, node_id, inputs),
                (tf::Display::Grid, true) => tf::compute_grid_layout(tree, node_id, inputs),
                (_, false) => {
                    let partial = tree.tree.partial_nodes.get_mut(key(node_id)).unwrap();
                    let PartialNode { style, context, .. } = partial;
                    let measure_function = &mut tree.measure_function;
                    let mut first_baseline = None;
                    let mut output = tf::compute_leaf_layout(
                        inputs,
                        style,
                        |_value, _basis| 0.0,
                        |known_dimensions, available_space| {
                            let measured = measure_function(
                                known_dimensions,
                                available_space,
                                node_id,
                                context.as_mut(),
                                style,
                            );
                            first_baseline = measured.first_baseline;
                            measured.size
                        },
                    );
                    output.first_baselines.y = first_baseline;
                    output
                }
            }
        })
    }
}

impl<C, M> TraversePartialTree for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, parent: tf::NodeId) -> Self::ChildIter<'_> {
        ChildIter(self.tree.partial_node(parent).unwrap().children.iter())
    }

    fn child_count(&self, parent: tf::NodeId) -> usize {
        self.tree.partial_node(parent).unwrap().children.len()
    }

    fn get_child_id(&self, parent: tf::NodeId, index: usize) -> tf::NodeId {
        self.tree.partial_node(parent).unwrap().children[index]
    }
}

impl<C, M> TraverseTree for LayoutView<'_, C, M> where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf
{
}

impl<C, M> LayoutPartialTree for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    type CoreContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type CustomIdent = String;

    fn get_core_container_style(&self, node: tf::NodeId) -> Self::CoreContainerStyle<'_> {
        &self.tree.partial_node(node).unwrap().style
    }

    fn set_unrounded_layout(&mut self, node: tf::NodeId, layout: &tf::Layout) {
        self.tree.partial_node_mut(node).unwrap().unrounded_layout = *layout;
    }

    fn resolve_calc_value(&self, _value: *const (), _basis: f32) -> f32 {
        0.0
    }

    fn compute_child_layout(
        &mut self,
        node: tf::NodeId,
        inputs: tf::LayoutInput,
    ) -> tf::LayoutOutput {
        self.compute_child_layout_impl(node, inputs, None)
    }
}

impl<C, M> CacheTree for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    fn cache_get(&self, node: tf::NodeId, input: &tf::LayoutInput) -> Option<tf::LayoutOutput> {
        self.tree.partial_node(node).unwrap().cache.get(input)
    }

    fn cache_store(&mut self, node: tf::NodeId, input: &tf::LayoutInput, output: tf::LayoutOutput) {
        self.tree
            .partial_node_mut(node)
            .unwrap()
            .cache
            .store(input, output);
    }

    fn cache_clear(&mut self, node: tf::NodeId) {
        self.tree.partial_node_mut(node).unwrap().cache.clear();
    }
}

impl<C, M> LayoutBlockContainer for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    type BlockContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node: tf::NodeId) -> Self::BlockContainerStyle<'_> {
        self.get_core_container_style(node)
    }

    fn get_block_child_style(&self, node: tf::NodeId) -> Self::BlockItemStyle<'_> {
        self.get_core_container_style(node)
    }

    fn compute_block_child_layout(
        &mut self,
        node: tf::NodeId,
        inputs: tf::LayoutInput,
        context: Option<&mut tf::BlockContext<'_>>,
    ) -> tf::LayoutOutput {
        self.compute_child_layout_impl(node, inputs, context)
    }
}

impl<C, M> LayoutFlexboxContainer for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    type FlexboxContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node: tf::NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.get_core_container_style(node)
    }

    fn get_flexbox_child_style(&self, node: tf::NodeId) -> Self::FlexboxItemStyle<'_> {
        self.get_core_container_style(node)
    }
}

impl<C, M> LayoutGridContainer for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    type GridContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node: tf::NodeId) -> Self::GridContainerStyle<'_> {
        self.get_core_container_style(node)
    }

    fn get_grid_child_style(&self, node: tf::NodeId) -> Self::GridItemStyle<'_> {
        self.get_core_container_style(node)
    }
}

impl<C, M> RoundTree for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        tf::NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    fn get_unrounded_layout(&self, node: tf::NodeId) -> tf::Layout {
        self.tree.partial_node(node).unwrap().unrounded_layout
    }

    fn set_final_layout(&mut self, node: tf::NodeId, layout: &tf::Layout) {
        self.tree.partial_node_mut(node).unwrap().final_layout = *layout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_tree_passes_leaf_baselines_to_flexbox() {
        let mut host_ids = SlotMap::<NodeId, ()>::with_key();
        let root_host = host_ids.insert(());
        let first_host = host_ids.insert(());
        let second_host = host_ids.insert(());
        let mut tree = LayoutTree::<u8>::new();

        let root = tree.create_host(
            root_host,
            tf::Style {
                display: tf::Display::Flex,
                flex_direction: tf::FlexDirection::Row,
                align_items: Some(tf::AlignItems::BASELINE),
                ..Default::default()
            },
        );
        let first = tree.create_host(first_host, tf::Style::default());
        let second = tree.create_host(second_host, tf::Style::default());
        tree.set_node_context(first, Some(0)).unwrap();
        tree.set_node_context(second, Some(1)).unwrap();
        tree.set_children(root, &[first, second]).unwrap();

        tree.compute_layout_with_measure(
            root,
            tf::Size {
                width: tf::AvailableSpace::MaxContent,
                height: tf::AvailableSpace::MaxContent,
            },
            |_, _, _, context, _| match context.copied().unwrap() {
                0 => MeasuredLeaf {
                    size: tf::Size {
                        width: 100.0,
                        height: 60.0,
                    },
                    first_baseline: Some(20.0),
                },
                _ => MeasuredLeaf {
                    size: tf::Size {
                        width: 80.0,
                        height: 30.0,
                    },
                    first_baseline: Some(12.0),
                },
            },
        )
        .unwrap();

        assert_eq!(tree.unrounded_layout(first).location.y, 0.0);
        assert_eq!(tree.unrounded_layout(second).location.y, 8.0);
    }
}
