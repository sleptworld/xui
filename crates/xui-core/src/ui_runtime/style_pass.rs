//! The style pass: targets, transitions, and sampled inheritance.

use crate::animation::has_animatable_difference;
use crate::layout::{
    ParentLayout, computed_style_for_widget, container_layout, taffy_style_for_widget,
};
use crate::text::{TextHost, TextLayoutSlot};
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::commit::taffy_context;
use crate::ui_runtime::state::{HostWorkFlags, InheritanceSync};
use crate::widgets::WidgetType;
use taffy::prelude as tf;
use xui_interface::{ComputedStyle, NodeId, TextBackend, TextLayoutConstraints, TextLayoutInput};

impl UiRuntime {
    pub(super) fn recompute_node_state(&mut self, id: NodeId) {
        self.stats.update_visits += 1;

        let state = self.hosts[id].state;
        let before = self.hosts[id].state_before_change.take().unwrap_or(state);
        let widget = self.hosts[id].widget.clone();

        // Recompute style if the widget's style affects the state change.
        if widget.with_widgets(|w| w.style().affects_state_change(before, state)) {
            self.mark_work(id, HostWorkFlags::RECALC_STYLE);
        }
    }

    /// Republishes sampled inheritance from every node queued this frame.
    ///
    /// Each queued node re-syncs against its parent, then its children follow
    /// only while inherited properties keep moving, so the walk stays within
    /// the subtrees that actually changed. The order of the queue does not
    /// matter: re-syncing a node that moves pushes the change down again.
    pub(super) fn sync_pending_inheritance(&mut self) {
        let mut pending = std::mem::take(&mut self.ui_state.inheritance_sync_list);
        for InheritanceSync { node, descend } in pending.drain(..) {
            // Removed later in the frame that queued it.
            if !self.hosts.contains_key(node) {
                continue;
            }
            let moved = self.resync_inheritance(node);
            if moved || descend {
                self.sync_children_inheritance(node);
            }
        }
        debug_assert!(
            self.ui_state.inheritance_sync_list.is_empty(),
            "syncing inheritance must not queue more of it"
        );
        // Hand the allocation back for the next frame.
        self.ui_state.inheritance_sync_list = pending;
    }

    /// Re-syncs a queued node against its parent. Its sample is only stale when
    /// the parent shows something other than its target on an inherited
    /// property; otherwise the node's target, resolved against that target,
    /// is already right.
    fn resync_inheritance(&mut self, id: NodeId) -> bool {
        let Some(parent) = self.hosts.parent(id) else {
            return false;
        };
        let (target, effective) = self.style_system.styles(parent);
        if target.inherited_eq(effective) {
            return false;
        }
        let parent_effective = effective.clone();
        self.sync_inheritance_against(id, &parent_effective)
    }

    fn sync_children_inheritance(&mut self, id: NodeId) {
        let parent_effective = self.style_system.effective(id).clone();
        let mut cursor = self.hosts.link(id).first_child;
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).next_sibling;
            // A child whose inherited properties did not move hands its own
            // children the same parent values as before: nothing below it can
            // have changed.
            if self.sync_inheritance_against(child, &parent_effective) {
                self.sync_children_inheritance(child);
            }
        }
    }

    /// Returns whether one of `id`'s effective inherited properties moved.
    fn sync_inheritance_against(&mut self, id: NodeId, parent: &ComputedStyle) -> bool {
        self.stats.inheritance_visits += 1;
        let state = self.hosts[id].state;
        let style_system = &mut self.style_system;
        let (diff, requires_layout) = self.hosts[id].widget.with_widgets(|widget| {
            widget.style().with_patch_for_state(state, |patch| {
                style_system.sync_inherited(id, parent, patch)
            })
        });
        if !diff.is_empty() {
            let mut work = HostWorkFlags::from_style_diff(diff);
            if requires_layout {
                self.sync_effective_taffy_style(id);
                self.refresh_taffy_context(id);
                work |= HostWorkFlags::RELAYOUT;
            }
            self.mark_work(id, work);
        }
        diff.intersects(xui_interface::StyleDiffFlags::INHERITED)
    }

    pub(super) fn recompute_subtree_styles(&mut self, id: NodeId) {
        self.recompute_node_style(id);

        let mut cursor = self.hosts.link(id).first_child;
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).next_sibling;
            self.recompute_subtree_styles(child);
        }
    }

    /// Settling a new target drops the node's inherited sample, so every node
    /// that gets one is queued to re-sync against its parent's sample.
    pub(super) fn recompute_node_style(&mut self, id: NodeId) {
        self.stats.update_visits += 1;

        let widget = self.hosts[id].widget.clone();
        let state = self.hosts[id].state;
        let parent_style = match self.hosts.parent(id) {
            Some(parent) => self.style_system.computed(parent),
            None => self.style_system.default_style(),
        };
        let computed_style = computed_style_for_widget(&widget, parent_style, &self.theme, state);
        let current_style = self.style_system.computed(id);

        //  Initialize
        if !self.style_system.initialized(id) {
            self.style_system.set_computed(id, computed_style);
            self.style_system.set_initialized(id);
            let mut work_flags = HostWorkFlags::empty();
            if self.sync_effective_taffy_style(id) {
                work_flags |= HostWorkFlags::RELAYOUT;
            }
            self.mark_work(id, work_flags);
            self.refresh_taffy_context(id);
            // Its children are new too, or were reparented, and queue their own.
            self.ui_state.mark_inheritance_sync(id, false);
            return;
        }

        // Children resolve their targets against this node's target, so a
        // moved inherited property must reach them now, even while the
        // effective style still shows the old value: a transition samples its
        // `from` style on the frame it starts.
        let inherited_target_changed = !current_style.inherited_eq(&computed_style);
        let animatable_target_changed = has_animatable_difference(current_style, &computed_style);

        let effective_before = self.style_system.effective(id).clone();

        let target_unchanged = *current_style == computed_style;

        let mut cancelled_transition = false;
        let transition = widget.transition();
        let _started_transition = match (transition, animatable_target_changed) {
            (Some(transition), true) => self.style_system.start_transition(
                id,
                transition,
                &effective_before,
                &computed_style,
            ),
            (Some(_), false) => {
                self.style_system
                    .sync_transition_target(id, &computed_style);
                false
            }
            (None, _) => {
                cancelled_transition = self.style_system.remove_transition(id);
                false
            }
        };

        if target_unchanged && !cancelled_transition {
            return;
        }

        self.style_system.set_computed(id, computed_style);

        let effective_diff = effective_before.diff(self.style_system.effective(id));
        let mut work_flags = HostWorkFlags::from_style_diff(effective_diff);

        if inherited_target_changed {
            self.style_system.mark_subtree_dirty(id);
        }

        if self.sync_effective_taffy_style(id) {
            work_flags |= HostWorkFlags::RELAYOUT;
        }

        self.refresh_taffy_context(id);
        self.mark_work(id, work_flags);

        self.ui_state.mark_inheritance_sync(
            id,
            effective_diff.intersects(xui_interface::StyleDiffFlags::INHERITED),
        );
    }

    /// Re-derives `id`'s Taffy style. Returns whether it changed, in which case
    /// its cached layout was already thrown away.
    pub(super) fn sync_effective_taffy_style(&mut self, id: NodeId) -> bool {
        let parent = self.hosts.parent(id);
        // How the parent arranges its children — not just which way it points.
        // `Sizing::Fill` resolves differently under flex, grid, and block, and
        // reading only `flex_direction` could not tell them apart.
        let parent_layout = parent
            .map(|parent| container_layout(&self.hosts[parent].widget))
            .unwrap_or(ParentLayout::Block);
        let mut taffy_style = taffy_style_for_widget(
            &self.hosts[id].widget,
            parent_layout,
            self.style_system.effective(id),
        );
        if let Some(anchor) = self.anchor_system.get(id) {
            apply_anchored_layout(&mut taffy_style, anchor.width);
        }
        self.layout_tree.set_style(id, taffy_style)
    }

    pub(super) fn refresh_taffy_context(&mut self, id: NodeId) {
        let context = taffy_context(id, &self.hosts[id]);
        self.layout_tree.set_context(id, context);
    }

    pub(super) fn recompute_node_text_shape<T: TextBackend>(
        &mut self,
        node_id: NodeId,
        measurer: &mut TextHost<T>,
    ) {
        self.stats.update_visits += 1;
        let node = &self.hosts[node_id];
        match node.node_type {
            WidgetType::TextInput => {
                let style = self.style_system.effective(node_id);
                let props = node
                    .widget
                    .with_widgets(|w| w.text_layout_props(style))
                    .expect("a text input always lays out text");

                let constraints = TextLayoutConstraints::default();
                let font_context = measurer.backend().epoch();

                let input = TextLayoutInput::new(
                    props.text,
                    constraints,
                    props.style.into(),
                    props.paragraph,
                    props.text_box,
                    font_context,
                );
                measurer.get_or_shape_slot(node_id, TextLayoutSlot::PRIMARY, input);
            }
            _ => {}
        }
    }
}

/// Takes an anchored container out of flow, pinned to its parent's origin;
/// its anchor then translates it from there at paint and hit-test time.
fn apply_anchored_layout(style: &mut tf::Style, width: Option<f32>) {
    style.position = tf::Position::Absolute;
    style.inset = tf::Rect {
        left: tf::LengthPercentageAuto::length(0.0),
        top: tf::LengthPercentageAuto::length(0.0),
        right: tf::LengthPercentageAuto::auto(),
        bottom: tf::LengthPercentageAuto::auto(),
    };
    if let Some(width) = width {
        style.size.width = tf::Dimension::length(width);
    }
}
