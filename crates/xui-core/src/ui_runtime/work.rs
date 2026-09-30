//! Scheduling work on hosts and summarizing it up the tree.

use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::state::HostWorkFlags;
use crate::widgets::WidgetType;
use xui_interface::NodeId;

impl UiRuntime {
    /// Schedules `flags` on `id` and summarizes them up its ancestors.
    ///
    /// Only work not already scheduled is queued, so marking the same node
    /// twice in a frame is a flag test.
    ///
    /// # Preconditions
    /// - `id` is live.
    pub(super) fn mark_work(&mut self, id: NodeId, flags: HostWorkFlags) {
        let node = &mut self.hosts[id];
        let newly_added = flags & !node.work;
        if newly_added.is_empty() {
            return;
        }
        node.work |= flags;
        let is_canvas = node.node_type == WidgetType::Canvas;

        if newly_added.intersects(HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::SYNC_TREE) {
            // Host dirtiness alone is not enough: Taffy keeps intrinsic and
            // final layout entries per node. A resize must invalidate the
            // affected Taffy node too, otherwise a min-content text probe can
            // survive as the child's apparent final layout.
            self.layout_tree.invalidate(id);
            self.ui_state.mark_layout_dirty(id);
        }

        if newly_added.intersects(HostWorkFlags::RECALC_STYLE_SUBTREE) {
            self.style_system.mark_subtree_dirty(id);
        }

        if newly_added.intersects(HostWorkFlags::RECALC_STYLE) {
            self.style_system.mark_dirty(id);
        }

        if newly_added.intersects(HostWorkFlags::SYNC_STATE_CHANGE) {
            self.ui_state.mark_state_change_dirty(id);
        }

        if newly_added.intersects(HostWorkFlags::SHAPE_CHANGE) {
            if is_canvas {
                // A canvas cannot be compiled before layout: a painter needs the
                // measured size, and its text boxes do not exist until it runs.
                self.ui_state.mark_canvas_dirty(id);
            } else {
                self.ui_state.mark_shape_dirty(id);
            }
        }

        // Painters read the resolved style, so a theme switch has to reach them.
        if is_canvas && newly_added.intersects(HostWorkFlags::RECALC_STYLE) {
            self.ui_state.mark_canvas_dirty(id);
        }

        let mut current = id;
        let mut remaining = newly_added;

        while let Some(parent) = self.hosts.parent(current) {
            let new_for_parent = remaining & !self.hosts[parent].subtree_work;
            self.hosts[parent].subtree_work |= remaining;
            if new_for_parent.is_empty() {
                break;
            }
            remaining = new_for_parent;
            current = parent;
        }
    }

    pub(super) fn clear_work_subtree(&mut self, id: NodeId, flags: HostWorkFlags) -> HostWorkFlags {
        let current = self.hosts[id].work | self.hosts[id].subtree_work;
        if !current.intersects(flags) {
            return current;
        }
        let mut subtree_work = HostWorkFlags::empty();
        let mut cursor = self.hosts.link(id).first_child;
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).next_sibling;
            // A child with nothing to clear still contributes its residual
            // work to this node's subtree summary, exactly as the recursive
            // early return used to.
            let child_work = self.hosts[child].work | self.hosts[child].subtree_work;
            if !child_work.intersects(flags) {
                subtree_work |= child_work;
                continue;
            }
            subtree_work |= self.clear_work_subtree(child, flags);
        }

        let node = &mut self.hosts[id];
        node.old_props_hash = node.new_props_hash;
        node.work.remove(flags);
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    pub(super) fn rebuild_subtree_dirty(&mut self, id: NodeId) -> HostWorkFlags {
        if self.hosts[id].work.is_empty() && self.hosts[id].subtree_work.is_empty() {
            return HostWorkFlags::empty();
        }
        let mut subtree_work = HostWorkFlags::empty();
        let mut cursor = self.hosts.link(id).first_child;
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).next_sibling;
            if self.hosts[child].work.is_empty() && self.hosts[child].subtree_work.is_empty() {
                continue;
            }
            subtree_work |= self.rebuild_subtree_dirty(child);
        }

        let node = &mut self.hosts[id];
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }
}
