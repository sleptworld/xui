use xui_interface::{NodeId, StyleDiffFlags, WidgetUpdateFlags};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct HostWorkFlags: u8 {
        const RECALC_STYLE = 1 << 0;
        const RECALC_STYLE_SUBTREE = 1 << 1;
        const RECALC_LAYOUT = 1 << 2;
        const REBUILD_PAINT = 1 << 3;
        const SHAPE_CHANGE = 1 << 4;
        const SYNC_TREE = 1 << 5;
        const SYNC_STATE_CHANGE = 1 << 6;
        const SYNC_RENDER = 1 << 7;
    }
}

#[derive(Default)]
pub(crate) struct UiState {
    pub(crate) layout_dirty_list: Vec<NodeId>,
    shape_dirty_list: Vec<NodeId>,
    state_change_dirty_list: Vec<NodeId>,
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
            work |=
                Self::SYNC_TREE | Self::RECALC_STYLE | Self::RECALC_LAYOUT | Self::REBUILD_PAINT;
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
            work |= Self::REBUILD_PAINT | Self::RECALC_STYLE_SUBTREE;
        }
        if flags.intersects(StyleDiffFlags::LAYOUT) {
            work |= Self::RECALC_LAYOUT | Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::PAINT) {
            work |= Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::SCROLL) {
            work |= Self::RECALC_LAYOUT | Self::REBUILD_PAINT;
        }
        if flags.intersects(StyleDiffFlags::EFFECT) {
            work |= Self::SYNC_RENDER;
        }
        work
    }
}
