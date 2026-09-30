use xui_interface::{NodeId, StyleDiffFlags, WidgetUpdateFlags};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct HostWorkFlags: u16 {
        const RECALC_STYLE = 1 << 0;
        const RECALC_STYLE_SUBTREE = 1 << 1;
        const RECALC_LAYOUT = 1 << 2;
        const REBUILD_PAINT = 1 << 3;
        const SHAPE_CHANGE = 1 << 4;
        const SYNC_TREE = 1 << 5;
        const SYNC_STATE_CHANGE = 1 << 6;
        const SYNC_RENDER = 1 << 7;
        /// Re-derive the node's paint transform from its effective style
        /// transform, anchor offset, and laid-out size. The render sync
        /// applies it once, after layout, whichever of those moved.
        const SYNC_TRANSFORM = 1 << 8;

        // Presets: the combinations the pipeline schedules together.

        /// A node's child list changed: re-sync its tree and lay it out again.
        const CHILDREN_CHANGED = Self::SYNC_TREE.bits() | Self::RECALC_LAYOUT.bits();
        /// A node joined, left, or changed parents, so everything it inherits
        /// may have moved.
        const REPARENTED = Self::RECALC_STYLE_SUBTREE.bits() | Self::RECALC_LAYOUT.bits();
        /// Geometry changed: lay out and paint again.
        const RELAYOUT = Self::RECALC_LAYOUT.bits() | Self::REBUILD_PAINT.bits();
        /// Everything a node needs on the frame it is created.
        const MOUNT = Self::RECALC_STYLE.bits() | Self::RELAYOUT.bits() | Self::SYNC_TRANSFORM.bits();
        /// Every style in the tree is stale, as after a theme switch.
        const RESTYLE_TREE = Self::MOUNT.bits() | Self::RECALC_STYLE_SUBTREE.bits();
        /// Work that rebuilds the node's part of the retained render scene.
        const SCENE = Self::SYNC_RENDER.bits() | Self::SYNC_TREE.bits() | Self::REBUILD_PAINT.bits();
        /// Everything the render sync walks for: the scene, and transforms.
        const RENDER = Self::SCENE.bits() | Self::SYNC_TRANSFORM.bits();
    }
}

#[derive(Default)]
pub(crate) struct UiState {
    pub(crate) layout_dirty_list: Vec<NodeId>,
    shape_dirty_list: Vec<NodeId>,
    state_change_dirty_list: Vec<NodeId>,
    /// Canvases whose drawing has to be rebuilt. Separate from
    /// `shape_dirty_list` because it is drained after layout, not before.
    pub(crate) canvas_dirty_list: Vec<NodeId>,
    /// Where sampled inheritance has to be republished this frame. Kept rather
    /// than reallocated: it is filled on every frame an inherited property
    /// animates.
    pub(crate) inheritance_sync_list: Vec<InheritanceSync>,
}

/// A node whose sampled inheritance may be stale.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InheritanceSync {
    pub(crate) node: NodeId,
    /// The node's own effective inherited properties already moved, so its
    /// children follow even if re-syncing the node itself changes nothing.
    pub(crate) descend: bool,
}

impl UiState {
    #[inline]
    pub(crate) fn mark_layout_dirty(&mut self, id: NodeId) {
        self.layout_dirty_list.push(id);
    }

    #[inline]
    pub(crate) fn mark_state_change_dirty(&mut self, id: NodeId) {
        self.state_change_dirty_list.push(id);
    }

    #[inline]
    pub(crate) fn mark_shape_dirty(&mut self, id: NodeId) {
        self.shape_dirty_list.push(id);
    }

    pub(crate) fn drain_state_change_dirty_list(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.state_change_dirty_list)
    }

    pub(crate) fn drain_shape_dirty_list(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.shape_dirty_list)
    }

    #[inline]
    pub(crate) fn mark_canvas_dirty(&mut self, id: NodeId) {
        self.canvas_dirty_list.push(id);
    }

    pub(crate) fn drain_canvas_dirty_list(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.canvas_dirty_list)
    }

    #[inline]
    pub(crate) fn mark_inheritance_sync(&mut self, node: NodeId, descend: bool) {
        self.inheritance_sync_list
            .push(InheritanceSync { node, descend });
    }
}

impl HostWorkFlags {
    pub fn from_widget_update(flags: WidgetUpdateFlags) -> Self {
        let mut work = Self::empty();
        if flags.intersects(WidgetUpdateFlags::STYLE_TARGET) {
            work |= Self::RECALC_STYLE;
        }
        if flags.intersects(WidgetUpdateFlags::LAYOUT_INPUT) {
            work |= Self::RECALC_LAYOUT;
        }
        if flags.intersects(WidgetUpdateFlags::PAINT_OUTPUT) {
            work |= Self::REBUILD_PAINT;
        }
        if flags.intersects(WidgetUpdateFlags::TREE) {
            work |= Self::SYNC_TREE | Self::MOUNT;
        }

        if flags.intersects(WidgetUpdateFlags::TEXT_SHAPE) {
            work |= Self::SHAPE_CHANGE;
        }

        if flags.intersects(WidgetUpdateFlags::STATE_CHANGE) {
            work |= Self::SYNC_STATE_CHANGE;
        }
        work
    }

    pub fn from_style_diff(flags: StyleDiffFlags) -> Self {
        let mut work = Self::empty();
        if flags.intersects(StyleDiffFlags::TEXT) {
            work |= Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::LAYOUT) {
            work |= Self::RELAYOUT;
        }
        if flags.intersects(StyleDiffFlags::PAINT) {
            work |= Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::SCROLL) {
            work |= Self::RELAYOUT;
        }
        if flags.intersects(StyleDiffFlags::EFFECT) {
            work |= Self::SYNC_RENDER;
        }
        if flags.intersects(StyleDiffFlags::TRANSFORM) {
            work |= Self::SYNC_TRANSFORM;
        }
        work
    }
}
