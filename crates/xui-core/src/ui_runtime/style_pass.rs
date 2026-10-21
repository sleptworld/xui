//! The style pass: targets, transitions, keyframe animations, and sampled
//! inheritance.

use crate::animation::has_animatable_difference;
use crate::layout::{ParentLayout, container_layout, taffy_style_for_widget};
use crate::text::{TextHost, TextLayoutSlot};
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::commit::taffy_context;
use crate::ui_runtime::state::{HostWorkFlags, InheritanceSync};
use crate::ui_runtime::style::style_change_requires_layout;
use crate::widgets::WidgetType;
use taffy::prelude as tf;
use xui_interface::{ComputedStyle, NodeId, TextBackend, TextLayoutConstraints, TextLayoutInput};

impl UiRuntime {
    pub(super) fn recompute_node_state(&mut self, id: NodeId) {
        self.stats.update_visits += 1;

        let state = self.hosts[id].state;
        let before = self.hosts[id].state_before_change.take().unwrap_or(state);
        let caps = self.hosts[id].caps;

        // Recompute style if one of its state layers starts or stops.
        if self.columns.affects_state_change(id, caps, before, state) {
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
        let caps = self.hosts[id].caps;
        let columns = &self.columns;
        let before = self.style_system.sync_inherited(
            id,
            parent,
            |current, parent| columns.inherits_differently(id, caps, state, current, parent),
            |target, parent| columns.inherit_unset(id, caps, state, target, parent),
        );
        let Some(before) = before else {
            return false;
        };
        // The base under a keyframe animation moved: its keyframes go on top of
        // the new one.
        self.rebase_keyframes(id, false);
        let after = self.style_system.effective(id);
        let diff = before.diff(after);
        let requires_layout = style_change_requires_layout(&before, after);
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

        let state = self.hosts[id].state;
        let caps = self.hosts[id].caps;
        let initial = self.style_system.default_style();
        let parent_style = match self.hosts.parent(id) {
            Some(parent) => self.style_system.computed(parent),
            None => initial,
        };
        let computed_style =
            self.columns
                .compute_style(id, caps, state, parent_style, initial, &self.theme);
        let animation = self.columns.animation(id);
        let current_style = self.style_system.computed(id);

        //  Initialize
        if !self.style_system.initialized(id) {
            self.style_system.set_computed(id, computed_style);
            // Mounted with it: plays from its first frame, and the first frame
            // already shows it.
            if let Some(animation) = animation {
                self.style_system.start_animation(id, animation);
                self.rebase_keyframes(id, true);
            }
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

        // What the node shows, for the diff, and what lies under its keyframe
        // animation, which is where a transition starts from: the keyframes
        // are a layer of their own and go on top of whatever it does.
        let effective_before = self.style_system.effective(id).clone();
        let base_before = self.style_system.base(id).clone();

        let target_unchanged = *current_style == computed_style;

        // The same animation keeps playing across re-renders; a different one
        // starts over.
        let animation_started = match animation {
            Some(animation) if self.style_system.animation_config(id) == Some(&animation) => false,
            Some(animation) => {
                self.style_system.start_animation(id, animation);
                true
            }
            None => self.style_system.remove_animation(id),
        };

        let mut cancelled_transition = false;
        let transition = self.columns.transition(id);
        let _started_transition = match (transition, animatable_target_changed) {
            (Some(transition), true) => {
                self.style_system
                    .start_transition(id, transition, &base_before, &computed_style)
            }
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

        let nothing_else_moved = target_unchanged && !cancelled_transition && !animation_started;
        if !nothing_else_moved {
            self.style_system.set_computed(id, computed_style);
        }
        // Even when nothing else moved: a keyframe can name a token, or
        // inherit from the parent, where the node's own style does neither, so
        // a theme switch can move it alone.
        let keyframes_moved = self.rebase_keyframes(id, true);
        if nothing_else_moved && !keyframes_moved {
            return;
        }

        let effective_after = self.style_system.effective(id);
        let effective_diff = effective_before.diff(effective_after);
        let mut work_flags = HostWorkFlags::from_style_diff(effective_diff);
        // Shaping inputs never reach the Taffy style, so the sync below cannot
        // see them: the text has to be measured again for its own sake.
        if style_change_requires_layout(&effective_before, effective_after) {
            work_flags |= HostWorkFlags::RELAYOUT;
        }

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
                let font_context = measurer.font_epoch();

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

impl UiRuntime {
    /// Re-resolves `id`'s keyframe animation against what lies under it, when
    /// that moved or when `force` says something else might have -- the theme,
    /// the parent. Returns whether the keyframes changed. Does nothing for a
    /// node with no keyframe animation.
    pub(super) fn rebase_keyframes(&mut self, id: NodeId, force: bool) -> bool {
        if self.style_system.animation_config(id).is_none() {
            return false;
        }
        if !force && !self.style_system.keyframe_base_moved(id) {
            return false;
        }
        let initial = self.style_system.default_style();
        let parent = match self.hosts.parent(id) {
            Some(parent) => self.style_system.effective(parent),
            None => initial,
        };
        let base = self.style_system.base(id);
        // `base` with each keyframe resolved on top: equal to it on every
        // property neither keyframe names, which keeps the layers below
        // showing there.
        let (from, to) = self.columns.keyframe_styles(
            id,
            self.hosts[id].caps,
            base,
            parent,
            initial,
            &self.theme,
        );
        let base = base.clone();
        self.style_system.rebase_animation(id, base, from, to)
    }
}
