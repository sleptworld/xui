use slotmap::{Key, KeyData, SecondaryMap};
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

/// Host-facing geometry, written back after every layout pass.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayoutNode {
    pub layout: Bounds,
    pub previous_layout: Bounds,
    pub world_origin: Point,
    pub content_size: Size<f32>,
    pub scroll_offset: Point,
}

impl LayoutNode {
    const EMPTY: Self = Self {
        layout: Bounds::ZERO,
        previous_layout: Bounds::ZERO,
        world_origin: Point::zero(),
        content_size: Size::<f32>::ZERO,
        scroll_offset: Point::zero(),
    };

    #[inline(always)]
    pub(crate) fn visual_bounds(&self, ancestor_scroll_offset: Point) -> Bounds {
        Bounds::from_origin_size(
            self.world_origin - ancestor_scroll_offset,
            self.layout.size(),
        )
    }
}

/// What Taffy reads and writes for one node.
struct PartialNode<C> {
    style: tf::Style,
    context: Option<C>,
    parent: Option<NodeId>,
    children: Vec<tf::NodeId>,
    cache: tf::Cache,
    unrounded_layout: tf::Layout,
    final_layout: tf::Layout,
}

/// XUI-owned tree storage for Taffy's low-level layout algorithms.
///
/// Keyed by host id: Taffy's `NodeId` for a host is that host id's FFI bits,
/// so there is no second id space to map through.
///
/// Geometry and Taffy's own per-node state live in separate maps: hit testing
/// and `visual_layout` walk geometry for many nodes at once, and should not
/// stride over Taffy's style and cache to do it.
///
/// # Preconditions
///
/// Every live host has exactly one entry, created with the host and removed
/// with it. Every method taking a `NodeId` requires one, and panics without.
pub(crate) struct LayoutTree<C> {
    geometry: SecondaryMap<NodeId, LayoutNode>,
    partials: SecondaryMap<NodeId, PartialNode<C>>,
    use_rounding: bool,
}

#[inline(always)]
fn taffy_id(id: NodeId) -> tf::NodeId {
    tf::NodeId::from(id.data().as_ffi())
}

#[inline(always)]
fn host_id(id: tf::NodeId) -> NodeId {
    NodeId::from(KeyData::from_ffi(u64::from(id)))
}

impl<C> LayoutTree<C> {
    pub fn new() -> Self {
        Self {
            geometry: SecondaryMap::new(),
            partials: SecondaryMap::new(),
            use_rounding: true,
        }
    }

    pub fn create(&mut self, id: NodeId, style: tf::Style, context: Option<C>) {
        self.geometry.insert(id, LayoutNode::EMPTY);
        self.partials.insert(
            id,
            PartialNode {
                style,
                context,
                parent: None,
                children: Vec::new(),
                cache: tf::Cache::new(),
                unrounded_layout: tf::Layout::with_order(0),
                final_layout: tf::Layout::with_order(0),
            },
        );
    }

    /// Drops `id`'s entry without touching its parent's child list.
    ///
    /// For removing a whole subtree: detach its root first, then forget every
    /// node in it, children before parents or not -- none of them is looked
    /// at again.
    pub fn forget(&mut self, id: NodeId) {
        self.geometry.remove(id);
        self.partials.remove(id);
    }

    #[cfg(test)]
    pub fn contains(&self, id: NodeId) -> bool {
        self.geometry.contains_key(id)
    }

    #[inline]
    pub(crate) fn node(&self, id: NodeId) -> &LayoutNode {
        &self.geometry[id]
    }

    #[inline]
    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut LayoutNode {
        &mut self.geometry[id]
    }

    /// Replaces `id`'s style. Returns whether it changed; only then is the
    /// cached layout thrown away.
    pub fn set_style(&mut self, id: NodeId, style: tf::Style) -> bool {
        let partial = &mut self.partials[id];
        if partial.style == style {
            return false;
        }
        partial.style = style;
        self.invalidate(id);
        true
    }

    pub fn set_context(&mut self, id: NodeId, context: Option<C>) {
        self.partials[id].context = context;
        self.invalidate(id);
    }

    /// Inserts `child` under `parent`, before `before` or last. `child` must
    /// not have a parent.
    pub fn insert(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        debug_assert!(self.partials[child].parent.is_none());
        self.partials[child].parent = Some(parent);
        let children = &mut self.partials[parent].children;
        let index = before
            .and_then(|before| {
                let before = taffy_id(before);
                children.iter().position(|id| *id == before)
            })
            .unwrap_or(children.len());
        children.insert(index, taffy_id(child));
        self.invalidate(parent);
    }

    /// Takes `child` out of its parent's child list, if it has a parent.
    pub fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.partials[child].parent.take() else {
            return;
        };
        let child = taffy_id(child);
        self.partials[parent].children.retain(|id| *id != child);
        self.invalidate(parent);
    }

    /// Throws away the cached layout of `id` and every ancestor.
    pub fn invalidate(&mut self, id: NodeId) {
        let mut current = Some(id);
        while let Some(id) = current {
            let partial = &mut self.partials[id];
            partial.cache.clear();
            current = partial.parent;
        }
    }

    /// The layout result `id` paints with: pixel-rounded unless rounding is off.
    #[inline]
    pub fn layout(&self, id: NodeId) -> &tf::Layout {
        let partial = &self.partials[id];
        if self.use_rounding {
            &partial.final_layout
        } else {
            &partial.unrounded_layout
        }
    }

    #[inline]
    pub fn unrounded_layout(&self, id: NodeId) -> &tf::Layout {
        &self.partials[id].unrounded_layout
    }

    pub fn compute_layout_with_measure<MeasureFunction>(
        &mut self,
        root: NodeId,
        available_space: tf::Size<tf::AvailableSpace>,
        measure_function: MeasureFunction,
    ) where
        MeasureFunction: FnMut(
            tf::Size<Option<f32>>,
            tf::Size<tf::AvailableSpace>,
            NodeId,
            Option<&mut C>,
            &tf::Style,
        ) -> MeasuredLeaf,
    {
        let use_rounding = self.use_rounding;
        let root = taffy_id(root);
        let mut view = LayoutView {
            tree: self,
            measure_function,
        };
        tf::compute_root_layout(&mut view, root, available_space);
        if use_rounding {
            tf::round_layout(&mut view, root);
        }
    }

    #[inline(always)]
    fn partial(&self, id: tf::NodeId) -> &PartialNode<C> {
        &self.partials[host_id(id)]
    }

    #[inline(always)]
    fn partial_mut(&mut self, id: tf::NodeId) -> &mut PartialNode<C> {
        &mut self.partials[host_id(id)]
    }
}

impl<C> Default for LayoutTree<C> {
    fn default() -> Self {
        Self::new()
    }
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
        NodeId,
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
            let display = tree.tree.partial(node_id).style.display;
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
                    let partial = tree.tree.partial_mut(node_id);
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
                                host_id(node_id),
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
        NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, parent: tf::NodeId) -> Self::ChildIter<'_> {
        ChildIter(self.tree.partial(parent).children.iter())
    }

    fn child_count(&self, parent: tf::NodeId) -> usize {
        self.tree.partial(parent).children.len()
    }

    fn get_child_id(&self, parent: tf::NodeId, index: usize) -> tf::NodeId {
        self.tree.partial(parent).children[index]
    }
}

impl<C, M> TraverseTree for LayoutView<'_, C, M> where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        NodeId,
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
        NodeId,
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
        &self.tree.partial(node).style
    }

    fn set_unrounded_layout(&mut self, node: tf::NodeId, layout: &tf::Layout) {
        self.tree.partial_mut(node).unrounded_layout = *layout;
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
        NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    fn cache_get(&self, node: tf::NodeId, input: &tf::LayoutInput) -> Option<tf::LayoutOutput> {
        self.tree.partial(node).cache.get(input)
    }

    fn cache_store(&mut self, node: tf::NodeId, input: &tf::LayoutInput, output: tf::LayoutOutput) {
        self.tree.partial_mut(node).cache.store(input, output);
    }

    fn cache_clear(&mut self, node: tf::NodeId) {
        self.tree.partial_mut(node).cache.clear();
    }
}

impl<C, M> LayoutBlockContainer for LayoutView<'_, C, M>
where
    M: FnMut(
        tf::Size<Option<f32>>,
        tf::Size<tf::AvailableSpace>,
        NodeId,
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
        NodeId,
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
        NodeId,
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
        NodeId,
        Option<&mut C>,
        &tf::Style,
    ) -> MeasuredLeaf,
{
    fn get_unrounded_layout(&self, node: tf::NodeId) -> tf::Layout {
        self.tree.partial(node).unrounded_layout
    }

    fn set_final_layout(&mut self, node: tf::NodeId, layout: &tf::Layout) {
        self.tree.partial_mut(node).final_layout = *layout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_tree_passes_leaf_baselines_to_flexbox() {
        let mut host_ids = slotmap::SlotMap::<NodeId, ()>::with_key();
        let root = host_ids.insert(());
        let first = host_ids.insert(());
        let second = host_ids.insert(());
        let mut tree = LayoutTree::<u8>::new();

        tree.create(
            root,
            tf::Style {
                display: tf::Display::Flex,
                flex_direction: tf::FlexDirection::Row,
                align_items: Some(tf::AlignItems::BASELINE),
                ..Default::default()
            },
            None,
        );
        tree.create(first, tf::Style::default(), Some(0));
        tree.create(second, tf::Style::default(), Some(1));
        tree.insert(root, first, None);
        tree.insert(root, second, None);

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
        );

        assert_eq!(tree.unrounded_layout(first).location.y, 0.0);
        assert_eq!(tree.unrounded_layout(second).location.y, 8.0);
    }
}
