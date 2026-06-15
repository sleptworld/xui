use slotmap::SlotMap;
use std::cell::Cell;
use std::time::Duration;
use taffy::prelude as tf;
use xui_interface::render::Damage;
use xui_interface::{
    ComputedColorStyle, ComputedScrollbarStyle, ComputedStyle, ComputedTextStyle, DirtyFlags,
    EventHandlers, EventTrigger, NodeId, NodeLifecycleEvent, ScrollbarVisibilityStyle, TextContent,
    TextLayoutConstraints, TextMeasurer, Theme, Translation,
};

use crate::animation::{ActiveAnimation, AnimableStyle, AnimationTransition, StyleAnimationRule};
use crate::core::{Point, Rect, Size};
use crate::event::{Event, EventHandlerSet, EventHandlerStore, EventResult};
use crate::event_system::{self, EventState};
use crate::fiber::Key;
use crate::layout::{computed_style_for_widget, taffy_style_for_widget};
use crate::render::{DamageRegion, PaintCommand};
use crate::widgets::{WidgetI, WidgetType, Widgets};

pub enum WidgetContext {
    Text(NodeId, ComputedTextStyle, Option<TextContent>),
    Button(NodeId, ComputedTextStyle, Option<TextContent>),
}

pub struct Node {
    pub id: NodeId,
    pub node_type: WidgetType,
    pub key: Option<Key>,
    // Link
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    // Layout
    pub taffy_node: tf::NodeId,
    pub position: usize,
    pub layout: Rect,
    pub previous_layout: Rect,
    pub content_size: Size<f32>,
    pub scroll_offset: Point,
    pub dirty: DirtyFlags,
    pub subtree_dirty: DirtyFlags,
    pub old_props_hash: u64,
    pub new_props_hash: u64,
    pub computed_style: ComputedStyle,
    pub animation_style: AnimableStyle,
    pub style_animation_rules: Vec<StyleAnimationRule>,
    pub pending_animation_triggers: Vec<EventTrigger>,
    pub active_animations: Vec<ActiveAnimation<AnimableStyle>>,
    pub paint_cache: Vec<PaintCommand>,
    pub widget: WidgetI,
    pub event_handlers: EventHandlerSet,
    // Text
}

impl Node {
    fn new(
        id: NodeId,
        key: Option<Key>,
        position: usize,
        props_hash: u64,
        computed_style: ComputedStyle,
        widget: WidgetI,
        event_handlers: EventHandlerSet,
        taffy_node: tf::NodeId,
    ) -> Self {
        let node_type = widget.node_type();

        Self {
            id,
            parent: None,
            children: Vec::new(),
            taffy_node,
            node_type,
            key,
            position,
            layout: Rect::ZERO,
            previous_layout: Rect::ZERO,
            content_size: Size::<f32>::ZERO,
            scroll_offset: Point::new(0.0, 0.0),
            dirty: DirtyFlags::default(),
            subtree_dirty: DirtyFlags::empty(),
            old_props_hash: 0,
            new_props_hash: props_hash,
            // style,
            computed_style,
            animation_style: AnimableStyle::default(),
            style_animation_rules: widget.style_animation_rules(),
            pending_animation_triggers: Vec::new(),
            active_animations: Vec::new(),
            paint_cache: Vec::new(),
            widget,
            event_handlers,
        }
    }

    fn start_style_animation(
        &mut self,
        trigger: EventTrigger,
        from_style: AnimableStyle,
        to_style: AnimableStyle,
        transition: AnimationTransition,
    ) {
        self.active_animations.clear();
        self.active_animations.push(ActiveAnimation::new(
            trigger, from_style, to_style, transition,
        ));
        self.refresh_animation_style();
    }

    fn tick_style_animations(&mut self, delta: Duration) -> bool {
        let mut changed = false;
        for animation in &mut self.active_animations {
            changed |= animation.tick(delta);
        }
        if changed {
            self.refresh_animation_style();
            self.active_animations.retain(ActiveAnimation::is_running);
        }
        changed
    }

    fn has_running_style_animations(&self) -> bool {
        self.active_animations
            .iter()
            .any(ActiveAnimation::is_running)
    }

    fn refresh_animation_style(&mut self) {
        let mut style = AnimableStyle::default();
        for animation in &self.active_animations {
            style.merge(&animation.sample());
        }
        self.animation_style = style;
    }

    fn queue_style_animation_trigger(&mut self, trigger: EventTrigger) -> bool {
        if self.style_animation_rule_for(trigger).is_none() {
            return false;
        }
        self.pending_animation_triggers.push(trigger);
        true
    }

    fn take_pending_animation_rule(&mut self) -> Option<StyleAnimationRule> {
        let triggers = std::mem::take(&mut self.pending_animation_triggers);
        triggers
            .into_iter()
            .rev()
            .find_map(|trigger| self.style_animation_rule_for(trigger))
    }

    fn style_animation_rule_for(&self, trigger: EventTrigger) -> Option<StyleAnimationRule> {
        if let Some(rule) = self
            .style_animation_rules
            .iter()
            .find(|rule| rule.trigger == trigger)
            .cloned()
        {
            return Some(rule);
        }

        let fallback = match trigger {
            EventTrigger::OnHoverStart => EventTrigger::OnHover,
            EventTrigger::OnHoverEnd => EventTrigger::OnHover,
            EventTrigger::OnPressStart => EventTrigger::OnPress,
            EventTrigger::OnPressEnd => EventTrigger::OnPress,
            _ => return None,
        };

        self.style_animation_rules
            .iter()
            .find(|rule| rule.trigger == fallback)
            .map(|rule| {
                if matches!(trigger, EventTrigger::OnHoverEnd | EventTrigger::OnPressEnd) {
                    StyleAnimationRule::reverse(trigger, rule.transition)
                } else {
                    StyleAnimationRule {
                        trigger,
                        style: rule.style.clone(),
                        transition: rule.transition,
                    }
                }
            })
    }

    fn clear_style_animation(&mut self) {
        self.animation_style = AnimableStyle::default();
        self.active_animations.clear();
    }
}

pub struct UiArena {
    nodes: SlotMap<NodeId, Node>,
    taffy: tf::TaffyTree<WidgetContext>,
    root: NodeId,
    damage: DamageRegion,
    damage_nodes: Vec<NodeId>,
    node_lifecycle_events: Vec<NodeLifecycleEvent>,
    event_state: EventState,
    event_handlers: EventHandlerStore,
    theme: Theme,
    paint_frames: Cell<usize>,
    pub update_visits: usize,
    pub layout_passes: usize,
    pub repaint_passes: usize,
}

impl UiArena {
    pub fn new() -> Self {
        let mut taffy = tf::TaffyTree::new();
        let theme = Theme::default();
        let root_widget = crate::widgets::root_widget();
        let root_parent_style = ComputedStyle::initial(&theme);
        let root_computed_style =
            computed_style_for_widget(&root_widget, &root_parent_style, &theme);
        let root_taffy_style =
            taffy_style_for_widget(&root_widget, &root_parent_style, &root_computed_style);
        let taffy_root = taffy
            .new_leaf(root_taffy_style)
            .expect("failed to create taffy root");
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert_with_key(|id| {
            Node::new(
                id,
                None,
                0,
                0,
                // root_style,
                root_computed_style,
                root_widget,
                EventHandlerSet::default(),
                taffy_root,
            )
        });
        Self {
            nodes,
            taffy,
            root,
            damage: DamageRegion::new(),
            damage_nodes: vec![],
            node_lifecycle_events: Vec::new(),
            event_state: EventState::default(),
            event_handlers: EventHandlerStore::default(),
            theme,
            paint_frames: Cell::new(0),
            update_visits: 0,
            layout_passes: 0,
            repaint_passes: 0,
        }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn start_style_animation(
        &mut self,
        id: NodeId,
        trigger: EventTrigger,
        from_style: AnimableStyle,
        to_style: AnimableStyle,
        transition: AnimationTransition,
    ) {
        if from_style.is_empty() && to_style.is_empty() {
            return;
        }
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };

        node.start_style_animation(trigger, from_style, to_style, transition);
        self.mark_dirty(id, DirtyFlags::ANIMATE);
    }

    pub(crate) fn queue_style_animation_trigger(&mut self, id: NodeId, trigger: EventTrigger) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.queue_style_animation_trigger(trigger) {
            self.mark_dirty(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }
    }

    pub fn tick_style_animations(&mut self, delta: Duration) -> bool {
        let mut changed = Vec::new();
        for (id, node) in self.nodes.iter_mut() {
            if node.tick_style_animations(delta) {
                changed.push(id);
            }
        }

        for id in &changed {
            self.mark_dirty(*id, DirtyFlags::ANIMATE);
        }

        !changed.is_empty()
    }

    #[inline(always)]
    pub fn has_running_style_animations(&self) -> bool {
        self.nodes.values().any(Node::has_running_style_animations)
    }

    #[inline(always)]
    pub fn effective_style(&self, id: NodeId) -> Option<ComputedStyle> {
        let node = self.nodes.get(id)?;
        Some(self.effective_style_for_node(node))
    }

    fn effective_style_for_node(&self, node: &Node) -> ComputedStyle {
        let mut style = node.computed_style.clone();
        node.animation_style
            .apply_to_computed(&mut style, &self.theme);
        style
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id].children
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.event_state.focused()
    }

    pub fn hovered_node(&self) -> Option<NodeId> {
        self.event_state.hovered()
    }

    pub fn pointer_capture_node(&self) -> Option<NodeId> {
        self.event_state.pointer_capture()
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.mark_dirty(
                self.root,
                DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
            );
        }
    }

    pub(crate) fn event_state(&self) -> &EventState {
        &self.event_state
    }

    pub(crate) fn event_state_mut(&mut self) -> &mut EventState {
        &mut self.event_state
    }

    pub(crate) fn event_handlers_mut(&mut self) -> &mut EventHandlerStore {
        &mut self.event_handlers
    }

    #[cfg(test)]
    pub(crate) fn set_event_handlers(&mut self, id: NodeId, event_handlers: EventHandlers) {
        let Some(current) = self.nodes.get(id).map(|node| node.event_handlers) else {
            return;
        };
        let event_handlers = self.event_handlers.update_set(current, event_handlers);
        if let Some(node) = self.nodes.get_mut(id) {
            node.event_handlers = event_handlers;
        }
    }

    pub fn create_node(
        &mut self,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        event_handlers: EventHandlers,
        style: tf::Style,
        computed_style: ComputedStyle,
    ) -> NodeId {
        let taffy_node = self
            .taffy
            .new_leaf(style)
            .expect("failed to create taffy node");
        let event_handlers = self
            .event_handlers
            .update_set(EventHandlerSet::default(), event_handlers);
        let id = self.nodes.insert_with_key(|id| {
            Node::new(
                id,
                key,
                0,
                props_hash,
                computed_style,
                widget,
                event_handlers,
                taffy_node,
            )
        });
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Created(id));
        self.refresh_taffy_context(id);

        id
    }

    pub fn insert(
        &mut self,
        parent: NodeId,
        widget: impl Into<Widgets>,
        style: tf::Style,
    ) -> NodeId {
        let parent_style = &self.nodes[parent].computed_style;
        let widget_ref = WidgetI::new(widget);
        let computed_style = computed_style_for_widget(&widget_ref, parent_style, &self.theme);
        self.insert_node(
            parent,
            None,
            0,
            style,
            computed_style,
            widget_ref,
            EventHandlers::default(),
        )
    }

    pub fn insert_node(
        &mut self,
        parent: NodeId,
        key: Option<Key>,
        props_hash: u64,
        style: tf::Style,
        computed_style: ComputedStyle,
        widget: WidgetI,
        event_handlers: EventHandlers,
    ) -> NodeId {
        let position = self.nodes[parent].children.len();
        let taffy_node = self
            .taffy
            .new_leaf(style)
            .expect("failed to create taffy node");
        let event_handlers = self
            .event_handlers
            .update_set(EventHandlerSet::default(), event_handlers);
        let id = self.nodes.insert_with_key(|id| {
            Node::new(
                id,
                key,
                position,
                props_hash,
                // style,
                computed_style,
                widget,
                event_handlers,
                taffy_node,
            )
        });
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Created(id));
        self.refresh_taffy_context(id);
        self.attach(parent, id);
        id
    }

    pub fn attach(&mut self, parent: NodeId, child: NodeId) {
        let old_parent = self.nodes[child].parent;
        let old_position = self.nodes[child].position;
        if let Some(old_parent) = old_parent.filter(|old_parent| *old_parent != parent) {
            self.nodes[old_parent]
                .children
                .retain(|candidate| *candidate != child);
            self.sync_taffy_children(old_parent);
            self.reindex_children(old_parent);
            self.mark_dirty(old_parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        }
        let parent_taffy = self.nodes[parent].taffy_node;
        let child_taffy = self.nodes[child].taffy_node;
        self.nodes[child].parent = Some(parent);
        if !self.nodes[parent].children.contains(&child) {
            self.nodes[parent].children.push(child);
        }
        let taffy_children: Vec<_> = self.nodes[parent]
            .children
            .iter()
            .map(|id| self.nodes[*id].taffy_node)
            .collect();
        self.taffy
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to attach taffy child");
        self.reindex_children(parent);
        let new_position = self.nodes[child].position;
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        self.add_node_damage(child, self.nodes[child].layout);
        let _ = child_taffy;
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if parent == child || !self.nodes.contains_key(parent) || !self.nodes.contains_key(child) {
            return;
        }

        let old_parent = self.nodes[child].parent;
        let old_position = self.nodes[child].position;

        if let Some(old_parent) = old_parent {
            self.detach_child_from_current_parent(child, old_parent);
        }

        if !self.nodes[parent].children.contains(&child) {
            self.nodes[parent].children.push(child);
        }
        self.nodes[child].parent = Some(parent);
        self.sync_taffy_children(parent);
        self.reindex_children(parent);

        let new_position = self.nodes[child].position;
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        self.add_node_damage(child, self.nodes[child].layout);
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        if child == before {
            return;
        }
        if parent == child || !self.nodes.contains_key(parent) || !self.nodes.contains_key(child) {
            return;
        }
        if !self.nodes.contains_key(before)
            || self.nodes.get(before).and_then(|node| node.parent) != Some(parent)
        {
            self.append_child(parent, child);
            return;
        }

        let old_parent = self.nodes[child].parent;
        let old_position = self.nodes[child].position;

        if let Some(old_parent) = old_parent {
            self.detach_child_from_current_parent(child, old_parent);
        }

        let Some(index) = self.nodes[parent]
            .children
            .iter()
            .position(|candidate| *candidate == before)
        else {
            panic!("ERROR");
        };

        self.nodes[parent].children.insert(index, child);
        self.nodes[child].parent = Some(parent);
        self.sync_taffy_children(parent);
        self.reindex_children(parent);

        let new_position = self.nodes[child].position;
        self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
        self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        self.add_node_damage(child, self.nodes[child].layout);
    }

    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if !self.nodes.contains_key(parent) || !self.nodes.contains_key(child) {
            return;
        }
        if self.nodes[child].parent != Some(parent) {
            return;
        }

        let old_position = self.nodes[child].position;
        self.nodes[parent]
            .children
            .retain(|candidate| *candidate != child);
        self.nodes[child].parent = None;
        self.nodes[child].position = 0;
        self.sync_taffy_children(parent);
        self.reindex_children(parent);
        self.record_node_move(child, Some(parent), None, old_position, 0);
        self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        self.add_node_damage(child, self.nodes[child].layout);
    }

    pub fn remove_from_parent(&mut self, child: NodeId) {
        let Some(parent) = self.nodes.get(child).and_then(|node| node.parent) else {
            return;
        };
        self.remove_child(parent, child);
    }

    pub fn clear_children(&mut self, parent: NodeId) {
        let children = self.nodes[parent].children.clone();
        for child in children {
            self.remove_subtree(child);
        }
        self.nodes[parent].children.clear();
        let parent_taffy = self.nodes[parent].taffy_node;
        self.taffy
            .set_children(parent_taffy, &[])
            .expect("failed to clear taffy children");
        self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
    }

    pub fn remove_subtree(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) || id == self.root {
            return;
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.remove_subtree(child);
        }

        let old_layout = self.nodes[id].layout;
        self.add_node_damage(id, old_layout);

        if let Some(parent) = self.nodes[id].parent {
            self.nodes[parent].children.retain(|child| *child != id);
            let parent_taffy = self.nodes[parent].taffy_node;
            let taffy_children: Vec<_> = self.nodes[parent]
                .children
                .iter()
                .map(|child| self.nodes[*child].taffy_node)
                .collect();
            let _ = self.taffy.set_children(parent_taffy, &taffy_children);
            self.reindex_children(parent);
            self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        }

        self.event_state.clear_node(id);
        self.event_handlers.clear_set(self.nodes[id].event_handlers);

        let _ = self.taffy.remove(self.nodes[id].taffy_node);
        self.nodes.remove(id);
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Removed(id));
    }

    pub fn drain_node_lifecycle_events(&mut self) -> Vec<NodeLifecycleEvent> {
        std::mem::take(&mut self.node_lifecycle_events)
    }

    pub fn mark_dirty(&mut self, id: NodeId, flags: DirtyFlags) {
        if flags.is_empty() || !self.nodes.contains_key(id) {
            return;
        }

        let rect = {
            let node = self.nodes.get_mut(id).expect("checked node existence");
            node.dirty |= flags;
            node.layout
        };
        if flags.intersects(DirtyFlags::PAINT | DirtyFlags::STYLE | DirtyFlags::ANIMATE) {
            self.add_node_damage(id, rect);
        }

        let mut current = id;
        while let Some(parent) = self.nodes[current].parent {
            self.nodes[parent].subtree_dirty |= flags;
            current = parent;
        }
    }

    pub fn add_damage(&mut self, damage: Damage) {
        self.damage.add(damage);
    }

    fn add_node_damage(&mut self, id: NodeId, rect: Rect) {
        if let Some(vis) = self.visual_damage_rect_for_node(id, rect) {
            self.damage.add(Damage::new(rect, vis));
        }
    }

    fn visual_damage_rect_for_node(&self, id: NodeId, mut rect: Rect) -> Option<Rect> {
        if !self.nodes.contains_key(id) {
            return None;
        }

        let mut ancestors = Vec::new();
        let mut cursor = self.nodes[id].parent;
        while let Some(parent) = cursor {
            ancestors.push(parent);
            cursor = self.nodes[parent].parent;
        }

        let mut total_scroll_offset = Point::zero();
        for ancestor in &ancestors {
            let node = &self.nodes[*ancestor];
            if node.computed_style.scroll.direction.is_scrollable() {
                total_scroll_offset.x += node.scroll_offset.x;
                total_scroll_offset.y += node.scroll_offset.y;
            }
        }
        rect.x -= total_scroll_offset.x;
        rect.y -= total_scroll_offset.y;

        let mut clip_scroll_offset = Point::zero();
        for ancestor in ancestors.into_iter().rev() {
            let node = &self.nodes[ancestor];
            let scrollable = node.computed_style.scroll.direction.is_scrollable();

            if node.computed_style.paint.clip || scrollable {
                let clip = Rect::new(
                    node.layout.x - clip_scroll_offset.x,
                    node.layout.y - clip_scroll_offset.y,
                    node.layout.width,
                    node.layout.height,
                );
                rect = intersect_rect(rect, clip)?;
            }

            if scrollable {
                clip_scroll_offset.x += node.scroll_offset.x;
                clip_scroll_offset.y += node.scroll_offset.y;
            }
        }

        Some(rect)
    }

    pub fn clear_dirty(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.old_props_hash = node.new_props_hash;
            node.dirty = DirtyFlags::empty();
            node.subtree_dirty = DirtyFlags::empty();
        }
    }

    pub fn mark_subtree_layout_dirty(&mut self, id: NodeId) {
        self.mark_dirty(id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
        let children = self.nodes[id].children.clone();
        for child in children {
            self.mark_subtree_layout_dirty(child);
        }
    }

    #[inline(always)]
    pub fn hit_test(&self, point: crate::core::Point) -> Option<NodeId> {
        self.hit_test_from(self.root, point, Point::zero())
    }

    fn hit_test_from(
        &self,
        id: NodeId,
        point: crate::core::Point,
        scroll_offset: Point,
    ) -> Option<NodeId> {
        let node = self.nodes.get(id)?;
        let visual_layout = Rect::new(
            node.layout.x - scroll_offset.x,
            node.layout.y - scroll_offset.y,
            node.layout.width,
            node.layout.height,
        );
        if !visual_layout.contains(point) {
            return None;
        }

        let child_scroll_offset = if node.computed_style.scroll.direction.is_scrollable() {
            Point::new(
                scroll_offset.x + node.scroll_offset.x,
                scroll_offset.y + node.scroll_offset.y,
            )
        } else {
            scroll_offset
        };

        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_from(*child, point, child_scroll_offset) {
                return Some(hit);
            }
        }

        Some(id)
    }

    pub(crate) fn scroll_node_by(&mut self, start: NodeId, delta: Point) -> bool {
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            if self.scroll_single_node_by(id, delta) {
                return true;
            }
            cursor = self.nodes.get(id).and_then(|node| node.parent);
        }
        false
    }

    fn scroll_single_node_by(&mut self, id: NodeId, delta: Point) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        let direction = node.computed_style.scroll.direction;
        if !direction.is_scrollable() {
            return false;
        }

        let max_x = if direction.allows_horizontal() {
            (node.content_size.width - node.layout.width).max(0.0)
        } else {
            0.0
        };
        let max_y = if direction.allows_vertical() {
            (node.content_size.height - node.layout.height).max(0.0)
        } else {
            0.0
        };
        if max_x <= 0.0 && max_y <= 0.0 {
            return false;
        }

        let scroll_delta = ergonomic_scroll_delta(delta);
        let next = Point::new(
            (node.scroll_offset.x - scroll_delta.x).clamp(0.0, max_x),
            (node.scroll_offset.y - scroll_delta.y).clamp(0.0, max_y),
        );
        if next == node.scroll_offset {
            return false;
        }

        let old_layout = node.layout;
        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.scroll_offset = next;
        self.add_scroll_damage(id, old_layout);
        self.mark_dirty(id, DirtyFlags::PAINT);
        true
    }

    fn add_scroll_damage(&mut self, id: NodeId, rect: Rect) {
        self.add_node_damage(id, rect);

        let mut cursor = self.nodes.get(id).and_then(|node| node.parent);
        while let Some(parent) = cursor {
            let node = &self.nodes[parent];
            let layout = node.layout;
            let next_parent = node.parent;
            let needs_damage = node.computed_style.paint.clip
                || node.computed_style.scroll.direction.is_scrollable();
            if needs_damage {
                self.add_node_damage(parent, layout);
            }
            cursor = next_parent;
        }
    }

    pub fn event_path(&self, target: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut cursor = Some(target);
        while let Some(id) = cursor {
            path.push(id);
            cursor = self.nodes[id].parent;
        }
        path.reverse();
        path
    }

    pub fn dispatch_event(&mut self, event: &Event) -> EventResult {
        event_system::dispatch_event(self, event)
    }

    pub fn update_tree<T: TextMeasurer>(
        &mut self,
        root: NodeId,
        size: Size<f32>,
        measurer: &mut T,
    ) {
        self.update_node(root, measurer);
        if self.has_layout_dirty() {
            self.compute_layout(size, measurer);
        }
        self.rebuild_subtree_dirty(root);
        self.repaint_dirty_subtree(root);
        self.clear_dirty_subtree(root);
    }

    fn update_node<T: TextMeasurer>(&mut self, id: NodeId, measurer: &mut T) {
        if !self.nodes.contains_key(id) {
            return;
        }
        let dirty = self.nodes[id].dirty;
        let subtree_dirty = self.nodes[id].subtree_dirty;
        // Fiber-style bailout: skip the whole branch when neither this node nor
        // any descendant has scheduled work.
        if dirty.is_empty() && subtree_dirty.is_empty() {
            return;
        }

        self.update_visits += 1;

        if dirty.intersects(DirtyFlags::STYLE) {
            self.recompute_subtree_styles(id, measurer);
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.update_node(child, measurer);
        }
    }

    fn recompute_subtree_styles<T: TextMeasurer>(&mut self, id: NodeId, measurer: &mut T) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let widget = self.nodes[id].widget.clone();
        let parent_style = self.nodes[id]
            .parent
            .and_then(|p| self.node(p))
            .map(|p| p.computed_style.clone())
            .unwrap_or_else(|| ComputedStyle::initial(&self.theme));
        let computed_style = computed_style_for_widget(&widget, &parent_style, &self.theme);
        let taffy_style = taffy_style_for_widget(&widget, &parent_style, &computed_style);

        let mut changed = false;
        let mut refresh_context = false;

        {
            let taffy_node_id = self
                .nodes
                .get(id)
                .map(|n| n.taffy_node)
                .expect("checked node existence");
            let previous_effective_style = self
                .effective_style(id)
                .expect("checked node existence before style recompute");
            let animation_rule = self
                .nodes
                .get_mut(id)
                .and_then(|n| n.take_pending_animation_rule());
            let animation_target = animation_rule.as_ref().map(|rule| {
                let mut target = computed_style.clone();
                if let Some(style) = &rule.style {
                    style.apply_to_computed(&mut target, &self.theme);
                }
                target
            });
            let has_animation_rule = animation_rule.is_some();

            if let Some(n) = self.node_mut(id) {
                if n.computed_style != computed_style {
                    let text_measure_changed =
                        matches!(n.node_type, WidgetType::Text | WidgetType::Label)
                            && n.computed_style.text != computed_style.text;
                    n.computed_style = computed_style.clone();
                    n.dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
                    if text_measure_changed {
                        n.dirty |= DirtyFlags::LAYOUT;
                        refresh_context = true;
                    }
                    changed = true;
                    if !has_animation_rule {
                        n.clear_style_animation();
                    }
                } else if has_animation_rule {
                    n.dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
                }

                if let Some((rule, target)) = animation_rule.zip(animation_target) {
                    let (from_style, to_style) =
                        AnimableStyle::diff(&previous_effective_style, &target);
                    if !to_style.is_empty() {
                        n.start_style_animation(
                            rule.trigger,
                            from_style,
                            to_style,
                            rule.transition,
                        );
                        n.dirty |= DirtyFlags::ANIMATE;
                    }
                }
            }

            let node_taffy_style = self.taffy.style(taffy_node_id).expect("No Taffy Node");

            if *node_taffy_style != taffy_style {
                if let Some(n) = self.node_mut(id) {
                    n.dirty |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
                }
                changed = true;

                self.taffy
                    .set_style(taffy_node_id, taffy_style)
                    .expect("failed to update taffy style");
            }
        }

        if refresh_context {
            self.refresh_taffy_context(id);
        }

        let children = self.nodes[id].children.clone();
        if changed {
            for child in &children {
                self.nodes[*child].dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
            }
        }
        for child in children {
            self.recompute_subtree_styles(child, measurer);
        }
    }

    pub fn compute_layout_if_needed<T: TextMeasurer>(&mut self, size: Size<f32>, measurer: &mut T) {
        if !self.has_layout_dirty() {
            return;
        }
        self.compute_layout(size, measurer);
    }

    #[inline(always)]
    fn has_layout_dirty(&self) -> bool {
        self.nodes
            .values()
            .any(|node| node.dirty.intersects(Self::layout_dirty_flags()))
    }

    #[inline(always)]
    fn layout_dirty_flags() -> DirtyFlags {
        DirtyFlags::LAYOUT | DirtyFlags::STYLE | DirtyFlags::TREE
    }

    #[inline(always)]
    fn paint_dirty_flags() -> DirtyFlags {
        DirtyFlags::PAINT | DirtyFlags::LAYOUT | DirtyFlags::STYLE | DirtyFlags::ANIMATE
    }

    pub fn repaint_if_needed(&mut self, id: NodeId) {
        let should_repaint = self
            .nodes
            .get(id)
            .is_some_and(|node| node.dirty.intersects(Self::paint_dirty_flags()));
        if !should_repaint {
            return;
        }

        self.repaint_passes += 1;
        let rect = self.nodes[id].layout;
        let style = self
            .effective_style(id)
            .expect("checked node existence before repaint");
        let mut cache = Vec::new();
        self.nodes[id].widget.paint(rect, &style, &mut cache);
        for command in &mut cache {
            if let PaintCommand::Text(command) = command {
                command.node_id = id;
            }
        }
        self.nodes[id].paint_cache = cache;
        if !self.damage_nodes.contains(&id) {
            self.damage_nodes.push(id);
        }
        self.add_node_damage(id, rect);
    }

    pub fn compute_layout<T: TextMeasurer>(&mut self, size: Size<f32>, measurer: &mut T) {
        self.layout_passes += 1;
        let root_taffy = self.nodes[self.root].taffy_node;
        self.taffy
            .compute_layout_with_measure(
                root_taffy,
                tf::Size {
                    width: tf::AvailableSpace::Definite(size.width),
                    height: tf::AvailableSpace::Definite(size.height),
                },
                |known_dimensions, available_space, _node_id, node_context, _style| {
                    measure_layout_context(
                        known_dimensions,
                        available_space,
                        node_context,
                        measurer,
                    )
                },
            )
            .expect("failed to compute layout");
        self.sync_layout(self.root, 0.0, 0.0);
    }

    fn sync_layout(&mut self, id: NodeId, offset_x: f32, offset_y: f32) -> DirtyFlags {
        if !self.nodes.contains_key(id) {
            return DirtyFlags::empty();
        }

        let taffy_node = self.nodes[id].taffy_node;
        let layout = self
            .taffy
            .layout(taffy_node)
            .expect("missing taffy layout result");
        let taffy_content_size =
            Size::<f32>::new(layout.content_size.width, layout.content_size.height);
        let rect = Rect::new(
            offset_x + layout.location.x,
            offset_y + layout.location.y,
            layout.size.width,
            layout.size.height,
        );

        let old_rect = self.nodes[id].layout;
        let layout_changed = old_rect != rect;
        if layout_changed {
            self.add_node_damage(id, old_rect);
            self.add_node_damage(id, rect);
        }

        let (children, mut subtree_dirty) = {
            let node = &mut self.nodes[id];
            let should_sync_children = layout_changed
                || node.dirty.intersects(Self::layout_dirty_flags())
                || node.subtree_dirty.intersects(Self::layout_dirty_flags());

            node.previous_layout = node.layout;
            node.layout = rect;
            node.dirty.remove(DirtyFlags::LAYOUT);
            if layout_changed {
                node.dirty.insert(DirtyFlags::PAINT);
            }

            if should_sync_children {
                (node.children.clone(), DirtyFlags::empty())
            } else {
                return node.dirty | node.subtree_dirty;
            }
        };

        for child in children {
            subtree_dirty |= self.sync_layout(child, rect.x, rect.y);
        }

        let content_size = self.content_size_from_children(id, taffy_content_size);
        let (scroll_dirty, rect) = {
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            let content_size_changed = node.content_size != content_size;
            let scroll_offset_before_clamp = node.scroll_offset;
            node.content_size = content_size;
            clamp_scroll_offset(node);
            (
                node.computed_style.scroll.direction.is_scrollable()
                    && (content_size_changed || node.scroll_offset != scroll_offset_before_clamp),
                node.layout,
            )
        };
        if scroll_dirty {
            self.add_node_damage(id, rect);
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            node.dirty.insert(DirtyFlags::PAINT);
        }
        let node = self.nodes.get_mut(id).expect("node removed during layout");
        node.subtree_dirty = subtree_dirty;
        node.dirty | node.subtree_dirty
    }

    fn content_size_from_children(&self, id: NodeId, taffy_content_size: Size<f32>) -> Size<f32> {
        let node = &self.nodes[id];
        let mut width = taffy_content_size.width.max(node.layout.width);
        let mut height = taffy_content_size.height.max(node.layout.height);

        for child in &node.children {
            let child = &self.nodes[*child];
            width = width.max(child.layout.x + child.layout.width - node.layout.x);
            height = height.max(child.layout.y + child.layout.height - node.layout.y);
        }

        Size::<f32>::new(width, height)
    }

    fn node_uses_unrounded_layout(&self, id: NodeId) -> bool {
        self.nodes
            .get(id)
            .map(|node| matches!(node.widget.node_type(), WidgetType::Text))
            .unwrap_or(false)
    }

    fn repaint_dirty_subtree(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let dirty = self.nodes[id].dirty;
        let subtree_dirty = self.nodes[id].subtree_dirty;
        if !dirty.intersects(Self::paint_dirty_flags())
            && !subtree_dirty.intersects(Self::paint_dirty_flags())
        {
            return;
        }

        if dirty.intersects(Self::paint_dirty_flags()) {
            self.repaint_if_needed(id);
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.repaint_dirty_subtree(child);
        }
    }

    fn clear_dirty_subtree(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let dirty = self.nodes[id].dirty;
        let subtree_dirty = self.nodes[id].subtree_dirty;
        if dirty.is_empty() && subtree_dirty.is_empty() {
            return;
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.clear_dirty_subtree(child);
        }
        self.clear_dirty(id);
    }

    fn rebuild_subtree_dirty(&mut self, id: NodeId) -> DirtyFlags {
        if !self.nodes.contains_key(id) {
            return DirtyFlags::empty();
        }

        let children = self.nodes[id].children.clone();
        let mut subtree_dirty = DirtyFlags::empty();
        for child in children {
            subtree_dirty |= self.rebuild_subtree_dirty(child);
        }

        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.subtree_dirty = subtree_dirty;
        node.dirty | node.subtree_dirty
    }

    // pub fn collect_paint_commands(&mut self) -> (DamageRegion, Vec<PaintCommand>) {
    //     let (damage, cmds) = self.prepare_paint_commands();
    //     self.finish_paint();
    //     (damage, cmds)
    // }

    pub fn prepare_paint_commands(&mut self) -> (&DamageRegion, Vec<PaintCommand>) {
        let damage = &self.damage;
        let mut commands = Vec::new();
        if damage.is_empty() {
            return (damage, commands);
        }

        self.paint_node(self.root, &damage, &mut commands);
        (damage, commands)
    }

    pub fn finish_paint(&mut self) {
        // println!("CLEAR DAMAGE");
        self.damage.clear();
        self.damage_nodes.clear();
        for (_, node) in self.nodes.iter_mut() {
            node.dirty.remove(DirtyFlags::PAINT | DirtyFlags::ANIMATE);
        }
    }

    #[inline(always)]
    fn paint_node(&self, id: NodeId, damage: &DamageRegion, commands: &mut Vec<PaintCommand>) {
        let _ = self.paint_node_inner(id, damage, commands, false, Point::zero());
    }

    fn paint_node_inner(
        &self,
        id: NodeId,
        damage: &DamageRegion,
        commands: &mut Vec<PaintCommand>,
        force: bool,
        scroll_offset: Point,
    ) -> Option<()> {
        let node = self.node(id)?;
        let visual_layout = node
            .layout
            .translate(Translation::new(-scroll_offset.x, -scroll_offset.y));

        // println!("DAMAGE : {damage:?}");
        // println!("VIS: {visual_layout:?}");

        if force || damage.intersects(visual_layout) {
            // println!("{id:?} NEED REPAINT");
            let scrollable = node.computed_style.scroll.direction.is_scrollable();
            if node.computed_style.paint.clip || scrollable {
                commands.push(PaintCommand::PushClip(node.layout));
            }
            if node.paint_cache.is_empty() {
                let style = self.effective_style_for_node(node);
                node.widget.paint(node.layout, &style, commands);
            } else {
                commands.extend_from_slice(&node.paint_cache);
            }
            if scrollable {
                commands.push(PaintCommand::PushTransform {
                    translate: Translation::new(-node.scroll_offset.x, -node.scroll_offset.y),
                });
            }
            let child_scroll_offset = if scrollable {
                scroll_offset + node.scroll_offset
            } else {
                scroll_offset
            };
            for child in &node.children {
                self.paint_node_inner(
                    *child,
                    damage,
                    commands,
                    force || scrollable,
                    child_scroll_offset,
                );
            }
            if scrollable {
                commands.push(PaintCommand::PopTransform);
                paint_scrollbars(node, commands);
            }
            if node.computed_style.paint.clip || scrollable {
                commands.push(PaintCommand::PopClip);
            }
        }

        Some(())
    }

    pub fn is_dirty(&self) -> bool {
        !self.damage.is_empty()
            || self.has_running_style_animations()
            || self
                .nodes
                .values()
                .any(|node| !node.dirty.is_empty() || !node.subtree_dirty.is_empty())
    }

    pub fn update_widget_node_from_parts(
        &mut self,
        id: NodeId,
        key: Option<Key>,
        props_hash: u64,
        style: tf::Style,
        computed_style: ComputedStyle,
        widget: WidgetI,
        event_handlers: EventHandlers,
    ) -> WidgetI {
        let mut flags = DirtyFlags::empty();
        let current_widget;
        let event_handlers = {
            let current = self
                .nodes
                .get(id)
                .expect("reused node missing")
                .event_handlers;
            self.event_handlers.update_set(current, event_handlers)
        };

        {
            let node = self.nodes.get_mut(id).expect("reused node missing");
            node.key = key;
            node.new_props_hash = props_hash;
            if node.old_props_hash != props_hash {
                flags |= DirtyFlags::PROPS;
            }
            if node.computed_style != computed_style {
                node.computed_style = computed_style.clone();
                flags |= DirtyFlags::STYLE | DirtyFlags::PAINT;
            }
            let node_taffy_id = node.taffy_node;
            let node_taffy_style = self
                .taffy
                .style(node_taffy_id)
                .expect("get taffy node style");

            if *node_taffy_style != style {
                flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
                self.taffy
                    .set_style(node.taffy_node, style)
                    .expect("failed to update taffy style");
            }
            if node.node_type != widget.node_type() {
                flags |= DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
            }
            let widget_flags = node.widget.update_from(&widget);

            flags |= widget_flags;
            node.event_handlers = event_handlers;
            node.style_animation_rules = node.widget.style_animation_rules();
            current_widget = node.widget.clone();
        }

        self.refresh_taffy_context(id);

        self.mark_dirty(id, flags);
        current_widget
    }

    fn sync_taffy_children(&mut self, parent: NodeId) {
        let parent_taffy = self.nodes[parent].taffy_node;
        let taffy_children: Vec<_> = self.nodes[parent]
            .children
            .iter()
            .map(|id| self.nodes[*id].taffy_node)
            .collect();
        self.taffy
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to sync taffy children");
    }

    fn detach_child_from_current_parent(&mut self, child: NodeId, old_parent: NodeId) {
        self.nodes[old_parent]
            .children
            .retain(|candidate| *candidate != child);
        self.nodes[child].parent = None;
        self.nodes[child].position = 0;
        self.sync_taffy_children(old_parent);
        self.reindex_children(old_parent);
        self.mark_dirty(old_parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
    }

    pub fn set_children(&mut self, parent: NodeId, children: Vec<NodeId>) {
        if !self.nodes.contains_key(parent) {
            return;
        }

        let old_children = self.nodes[parent].children.clone();
        let tree_changed = old_children != children;

        for (old_position, child) in old_children.iter().copied().enumerate() {
            if !children.contains(&child)
                && self.nodes.contains_key(child)
                && self.nodes[child].parent == Some(parent)
            {
                self.nodes[child].parent = None;
                self.nodes[child].position = 0;
                self.record_node_move(child, Some(parent), None, old_position, 0);
            }
        }

        self.nodes[parent].children = children;
        let new_children = self.nodes[parent].children.clone();
        for (new_position, child) in new_children.iter().copied().enumerate() {
            if self.nodes.contains_key(child) {
                let old_parent = self.nodes[child].parent;
                let old_position = self.nodes[child].position;
                if let Some(old_parent) = old_parent.filter(|old_parent| *old_parent != parent) {
                    self.nodes[old_parent]
                        .children
                        .retain(|candidate| *candidate != child);
                    self.sync_taffy_children(old_parent);
                    self.reindex_children(old_parent);
                    self.mark_dirty(old_parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
                }
                self.nodes[child].parent = Some(parent);
                self.nodes[child].position = new_position;
                self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
            }
        }
        self.reindex_children(parent);
        self.sync_taffy_children(parent);

        if tree_changed {
            self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        }
    }

    fn reindex_children(&mut self, parent: NodeId) {
        let children = self.nodes[parent].children.clone();
        for (position, child) in children.into_iter().enumerate() {
            self.nodes[child].position = position;
        }
    }

    fn record_node_move(
        &mut self,
        id: NodeId,
        old_parent: Option<NodeId>,
        new_parent: Option<NodeId>,
        old_position: usize,
        new_position: usize,
    ) {
        if old_parent == new_parent && old_position == new_position {
            return;
        }
        if old_parent.is_none() && new_parent.is_some() {
            return;
        }
        self.node_lifecycle_events.push(NodeLifecycleEvent::Moved {
            id,
            old_parent,
            new_parent,
            old_position,
            new_position,
        });
    }

    fn refresh_taffy_context(&mut self, id: NodeId) {
        let node = &self.nodes[id];

        let context = match node.node_type {
            WidgetType::Text | WidgetType::Label => Some(WidgetContext::Text(
                id,
                node.computed_style.text.clone(),
                node.widget.text(),
            )),
            WidgetType::Button => Some(WidgetContext::Button(
                id,
                node.computed_style.text.clone(),
                node.widget.text(),
            )),
            _ => None,
        };

        self.taffy
            .set_node_context(node.taffy_node, context)
            .expect("failed to update taffy context");
    }
}

impl Default for UiArena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use crate::animation::AnimationEasing;
    use crate::widgets::{button, column, container, text};
    use std::time::Duration;
    use xui_interface::{Color, EventTrigger, PointerButton, Style};

    fn default_style() -> tf::Style {
        tf::Style::default()
    }

    fn sized_style(width: f32, height: f32) -> tf::Style {
        tf::Style {
            size: tf::Size {
                width: tf::Dimension::length(width),
                height: tf::Dimension::length(height),
            },
            ..Default::default()
        }
    }

    fn linear_transition() -> AnimationTransition {
        AnimationTransition::new(Duration::from_millis(100)).ease(AnimationEasing::Linear)
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be near {expected}"
        );
    }

    fn assert_background_near(style: &ComputedStyle, expected: Color) {
        let ComputedColorStyle::Solid(color) = style.paint.background else {
            panic!("expected solid background");
        };
        assert_near(color.r, expected.r);
        assert_near(color.g, expected.g);
        assert_near(color.b, expected.b);
        assert_near(color.a, expected.a);
    }

    struct ZeroTextMeasurer;

    impl TextMeasurer for ZeroTextMeasurer {
        fn measure_text(&mut self, _text: &str, _props: &ComputedTextStyle) -> Size<f32> {
            Size::<f32>::ZERO
        }

        fn measure_text_with_constraints(
            &mut self,
            _text: &str,
            _props: &ComputedTextStyle,
            _constraints: TextLayoutConstraints,
        ) -> Size<f32> {
            Size::<f32>::ZERO
        }
    }

    #[test]
    fn append_child_moves_existing_child_to_end() {
        let mut arena = UiArena::new();
        let parent = arena.insert(arena.root(), column(), default_style());
        let a = arena.insert(parent, text("A"), default_style());
        let b = arena.insert(parent, text("B"), default_style());

        arena.append_child(parent, a);

        assert_eq!(arena.children(parent), &[b, a]);
        assert_eq!(arena.node(a).and_then(|node| node.parent), Some(parent));
        assert_eq!(arena.node(a).map(|node| node.position), Some(1));
    }

    #[test]
    fn insert_before_moves_existing_child() {
        let mut arena = UiArena::new();
        let parent = arena.insert(arena.root(), column(), default_style());
        let a = arena.insert(parent, text("A"), default_style());
        let b = arena.insert(parent, text("B"), default_style());
        let c = arena.insert(parent, text("C"), default_style());

        arena.insert_before(parent, c, b);

        assert_eq!(arena.children(parent), &[a, c, b]);
        assert_eq!(arena.node(c).and_then(|node| node.parent), Some(parent));
        assert_eq!(arena.node(c).map(|node| node.position), Some(1));
    }

    #[test]
    fn insert_before_child_itself_is_noop() {
        let mut arena = UiArena::new();
        let parent = arena.insert(arena.root(), column(), default_style());
        let a = arena.insert(parent, text("A"), default_style());
        let b = arena.insert(parent, text("B"), default_style());

        arena.insert_before(parent, a, a);

        assert_eq!(arena.children(parent), &[a, b]);
    }

    #[test]
    fn insert_before_missing_sibling_falls_back_to_append() {
        let mut arena = UiArena::new();
        let parent = arena.insert(arena.root(), column(), default_style());
        let other_parent = arena.insert(arena.root(), column(), default_style());
        let a = arena.insert(parent, text("A"), default_style());
        let b = arena.insert(parent, text("B"), default_style());
        let other = arena.insert(other_parent, text("Other"), default_style());

        arena.insert_before(parent, a, other);

        assert_eq!(arena.children(parent), &[b, a]);
        assert_eq!(arena.children(other_parent), &[other]);
    }

    #[test]
    fn remove_from_parent_detaches_child() {
        let mut arena = UiArena::new();
        let parent = arena.insert(arena.root(), column(), default_style());
        let a = arena.insert(parent, text("A"), default_style());
        let b = arena.insert(parent, text("B"), default_style());

        arena.remove_from_parent(a);

        assert_eq!(arena.children(parent), &[b]);
        assert_eq!(arena.node(a).and_then(|node| node.parent), None);
        assert_eq!(arena.node(a).map(|node| node.position), Some(0));
    }

    #[test]
    fn hover_change_starts_button_animation_and_repaints_sampled_style() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let button = arena.insert(
            root,
            button("Hover")
                .style(Style::new().background(Color::BLACK))
                .hover_style(Style::new().background(Color::WHITE))
                .hover_transition(linear_transition()),
            sized_style(40.0, 20.0),
        );
        let mut measurer = ZeroTextMeasurer;
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(1.0, 1.0),
        });
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        assert!(arena.has_running_style_animations());
        arena.tick_style_animations(Duration::from_millis(50));
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        let effective = arena.effective_style(button).unwrap();
        assert_background_near(&effective, Color::rgb(0.5, 0.5, 0.5));

        let paint_cache = &arena.node(button).unwrap().paint_cache;
        let PaintCommand::Rect { color, .. } = paint_cache.first().unwrap() else {
            panic!("expected button box paint command");
        };
        let ComputedColorStyle::Solid(color) = *color else {
            panic!("expected solid painted background");
        };
        assert_near(color.r, 0.5);
        assert_near(color.g, 0.5);
        assert_near(color.b, 0.5);
    }

    #[test]
    fn hover_change_starts_generic_widget_animation_and_repaints_sampled_style() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let item = arena.insert(
            root,
            container()
                .style(Style::new().background(Color::BLACK))
                .animation(
                    EventTrigger::OnHover,
                    AnimableStyle::new().background(Color::WHITE),
                    linear_transition(),
                ),
            sized_style(40.0, 20.0),
        );
        let mut measurer = ZeroTextMeasurer;
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(1.0, 1.0),
        });
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        assert!(arena.has_running_style_animations());
        arena.tick_style_animations(Duration::from_millis(50));
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        let effective = arena.effective_style(item).unwrap();
        assert_background_near(&effective, Color::rgb(0.5, 0.5, 0.5));

        let paint_cache = &arena.node(item).unwrap().paint_cache;
        let PaintCommand::Rect { color, .. } = paint_cache.first().unwrap() else {
            panic!("expected container box paint command");
        };
        let ComputedColorStyle::Solid(color) = *color else {
            panic!("expected solid painted background");
        };
        assert_near(color.r, 0.5);
        assert_near(color.g, 0.5);
        assert_near(color.b, 0.5);
    }

    #[test]
    fn click_starts_generic_widget_animation() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let item = arena.insert(
            root,
            container()
                .style(Style::new().background(Color::BLACK))
                .animation(
                    EventTrigger::OnClick,
                    AnimableStyle::new().background(Color::WHITE),
                    linear_transition(),
                ),
            sized_style(40.0, 20.0),
        );
        let mut measurer = ZeroTextMeasurer;
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        arena.dispatch_event(&Event::PointerDown {
            position: Point::new(1.0, 1.0),
            button: PointerButton::Primary,
        });
        arena.dispatch_event(&Event::PointerUp {
            position: Point::new(1.0, 1.0),
            button: PointerButton::Primary,
        });
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        assert!(arena.has_running_style_animations());
        arena.tick_style_animations(Duration::from_millis(50));

        let effective = arena.effective_style(item).unwrap();
        assert_background_near(&effective, Color::rgb(0.5, 0.5, 0.5));
    }

    #[test]
    fn pointer_press_uses_pressed_transition_rule() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let button = arena.insert(
            root,
            button("Press")
                .style(Style::new().background(Color::BLACK))
                .pressed_style(Style::new().background(Color::WHITE))
                .pressed_transition(linear_transition()),
            sized_style(40.0, 20.0),
        );
        let mut measurer = ZeroTextMeasurer;
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        arena.dispatch_event(&Event::PointerDown {
            position: Point::new(1.0, 1.0),
            button: PointerButton::Primary,
        });
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        assert!(arena.has_running_style_animations());
        arena.tick_style_animations(Duration::from_millis(50));

        let effective = arena.effective_style(button).unwrap();
        assert_background_near(&effective, Color::rgb(0.5, 0.5, 0.5));
    }
}

fn measure_layout_context<T: TextMeasurer>(
    known_dimensions: tf::Size<Option<f32>>,
    _available_space: tf::Size<tf::AvailableSpace>,
    node_context: Option<&mut WidgetContext>,
    measurer: &mut T,
) -> tf::Size<f32> {
    if let tf::Size {
        width: Some(width),
        height: Some(height),
    } = known_dimensions
    {
        return tf::Size { width, height };
    }

    let measured = match node_context {
        Some(WidgetContext::Text(node_id, props, t))
        | Some(WidgetContext::Button(node_id, props, t)) => {
            let str = t.as_ref().map(|t| t.as_str()).unwrap_or_default();
            let constraints = match known_dimensions.width {
                Some(width) => TextLayoutConstraints::max_width(width),
                None => TextLayoutConstraints::UNBOUNDED,
            };
            measurer.measure_node_text_with_constraints(*node_id, str, props, constraints)
        }

        _ => Size::<f32>::ZERO,
    };

    tf::Size {
        width: measured.width,
        height: measured.height,
    }
}

fn ergonomic_scroll_delta(delta: Point) -> Point {
    let magnitude = (delta.x * delta.x + delta.y * delta.y).sqrt();
    let factor = scroll_acceleration_factor(magnitude);
    Point::new(delta.x * factor, delta.y * factor)
}

fn scroll_acceleration_factor(magnitude: f32) -> f32 {
    const MIN_ACCELERATION_MAGNITUDE: f32 = 80.0;
    const FULL_ACCELERATION_MAGNITUDE: f32 = 160.0;
    const MAX_ACCELERATION: f32 = 2.75;

    if magnitude <= MIN_ACCELERATION_MAGNITUDE {
        return 1.0;
    }

    let progress =
        ((magnitude - MIN_ACCELERATION_MAGNITUDE) / FULL_ACCELERATION_MAGNITUDE).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - progress).powi(2);
    1.0 + eased * (MAX_ACCELERATION - 1.0)
}

fn clamp_scroll_offset(node: &mut Node) {
    let direction = node.computed_style.scroll.direction;
    let max_x = if direction.allows_horizontal() {
        (node.content_size.width - node.layout.width).max(0.0)
    } else {
        0.0
    };
    let max_y = if direction.allows_vertical() {
        (node.content_size.height - node.layout.height).max(0.0)
    } else {
        0.0
    };
    node.scroll_offset.x = node.scroll_offset.x.clamp(0.0, max_x);
    node.scroll_offset.y = node.scroll_offset.y.clamp(0.0, max_y);
}

fn paint_scrollbars(node: &Node, commands: &mut Vec<PaintCommand>) {
    let direction = node.computed_style.scroll.direction;
    let scrollbar = node.computed_style.scroll.scrollbar;
    if scrollbar.visibility == ScrollbarVisibilityStyle::Hidden || scrollbar.width <= 0.0 {
        return;
    }

    let max_x = (node.content_size.width - node.layout.width).max(0.0);
    let max_y = (node.content_size.height - node.layout.height).max(0.0);

    if direction.allows_vertical()
        && should_paint_scrollbar(scrollbar, max_y)
        && scrollbar.thumb_color.is_visible()
    {
        let track = vertical_scrollbar_track(node.layout, scrollbar.width);
        paint_scrollbar_part(track, scrollbar.track_color, scrollbar.radius, commands);

        if max_y > 0.0 {
            let ratio = (node.layout.height / node.content_size.height).clamp(0.0, 1.0);
            let thumb_height = (track.height * ratio)
                .max(scrollbar.width * 2.0)
                .min(track.height);
            let travel = (track.height - thumb_height).max(0.0);
            let top = track.y + travel * (node.scroll_offset.y / max_y);
            paint_scrollbar_part(
                Rect::new(track.x, top, track.width, thumb_height),
                scrollbar.thumb_color,
                scrollbar.radius,
                commands,
            );
        }
    }

    if direction.allows_horizontal()
        && should_paint_scrollbar(scrollbar, max_x)
        && scrollbar.thumb_color.is_visible()
    {
        let track = horizontal_scrollbar_track(node.layout, scrollbar.width);
        paint_scrollbar_part(track, scrollbar.track_color, scrollbar.radius, commands);

        if max_x > 0.0 {
            let ratio = (node.layout.width / node.content_size.width).clamp(0.0, 1.0);
            let thumb_width = (track.width * ratio)
                .max(scrollbar.width * 2.0)
                .min(track.width);
            let travel = (track.width - thumb_width).max(0.0);
            let left = track.x + travel * (node.scroll_offset.x / max_x);
            paint_scrollbar_part(
                Rect::new(left, track.y, thumb_width, track.height),
                scrollbar.thumb_color,
                scrollbar.radius,
                commands,
            );
        }
    }
}

fn should_paint_scrollbar(scrollbar: ComputedScrollbarStyle, max_offset: f32) -> bool {
    match scrollbar.visibility {
        ScrollbarVisibilityStyle::Auto => max_offset > 0.0,
        ScrollbarVisibilityStyle::Always => true,
        ScrollbarVisibilityStyle::Hidden => false,
    }
}

fn vertical_scrollbar_track(rect: Rect, width: f32) -> Rect {
    Rect::new(
        rect.x + (rect.width - width).max(0.0),
        rect.y,
        width.min(rect.width),
        rect.height,
    )
}

fn horizontal_scrollbar_track(rect: Rect, width: f32) -> Rect {
    Rect::new(
        rect.x,
        rect.y + (rect.height - width).max(0.0),
        rect.width,
        width.min(rect.height),
    )
}

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);

    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

fn paint_scrollbar_part(
    rect: Rect,
    color: ComputedColorStyle,
    radius: f32,
    commands: &mut Vec<PaintCommand>,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || !color.is_visible() {
        return;
    }

    if radius > 0.0 {
        commands.push(PaintCommand::RoundedRect {
            rect,
            radius,
            color,
            stroke: None,
            shadow: None,
        });
    } else {
        commands.push(PaintCommand::Rect {
            rect,
            color,
            stroke: None,
            shadow: None,
        });
    }
}

// #[cfg(test)]
// mod scroll_behavior_tests {
//     use taffy::prelude as tf;

//     use super::*;
//     use crate::{Style, widgets::ContainerWidget};

//     #[test]
//     fn scroll_acceleration_preserves_small_deltas_and_boosts_fast_deltas() {
//         assert_eq!(
//             ergonomic_scroll_delta(Point::new(0.0, -8.0)),
//             Point::new(0.0, -8.0)
//         );

//         let accelerated = ergonomic_scroll_delta(Point::new(0.0, -96.0));
//         assert!(accelerated.y < -96.0);
//         assert!(accelerated.y > -264.0);
//     }

//     #[test]
//     fn scroll_node_by_applies_ergonomic_delta() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let scroller = arena.insert(
//             root,
//             ContainerWidget::new().style(Style::new().scroll_vertical()),
//             tf::Style::default(),
//         );
//         let node = arena.node_mut(scroller).unwrap();
//         node.layout = Rect::new(0.0, 0.0, 100.0, 100.0);
//         node.content_size = Size::<f32>::new(100.0, 1_000.0);

//         assert!(arena.scroll_node_by(scroller, Point::new(0.0, -48.0)));
//         assert_eq!(arena.node(scroller).unwrap().scroll_offset.y, 48.0);
//     }

//     #[test]
//     fn dirty_scroll_container_damage_uses_viewport_rect() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let scroller = arena.insert(
//             root,
//             ContainerWidget::new().style(Style::new().scroll_vertical()),
//             tf::Style::default(),
//         );

//         arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 200.0);
//         arena.node_mut(scroller).unwrap().layout = Rect::new(10.0, 20.0, 80.0, 40.0);
//         arena.node_mut(scroller).unwrap().scroll_offset = Point::new(0.0, 30.0);
//         arena.finish_paint();

//         arena.mark_dirty(scroller, DirtyFlags::PAINT);

//         assert_eq!(
//             arena.prepare_paint_commands().0.rects(),
//             &[Rect::new(10.0, 20.0, 80.0, 40.0)]
//         );
//     }

//     #[test]
//     fn dirty_nested_child_damage_uses_all_ancestor_scroll_offsets() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let outer = arena.insert(
//             root,
//             ContainerWidget::new().style(Style::new().scroll_vertical()),
//             tf::Style::default(),
//         );
//         let inner = arena.insert(
//             outer,
//             ContainerWidget::new().style(Style::new().scroll_vertical()),
//             tf::Style::default(),
//         );
//         let child = arena.insert(inner, ContainerWidget::new(), tf::Style::default());

//         arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 200.0);
//         arena.node_mut(outer).unwrap().layout = Rect::new(0.0, 20.0, 100.0, 100.0);
//         arena.node_mut(outer).unwrap().scroll_offset = Point::new(0.0, 30.0);
//         arena.node_mut(inner).unwrap().layout = Rect::new(0.0, 70.0, 80.0, 80.0);
//         arena.node_mut(inner).unwrap().scroll_offset = Point::new(0.0, 10.0);
//         arena.node_mut(child).unwrap().layout = Rect::new(0.0, 90.0, 40.0, 40.0);
//         arena.finish_paint();

//         arena.mark_dirty(child, DirtyFlags::PAINT);

//         assert_eq!(
//             arena.prepare_paint_commands().0.rects(),
//             &[Rect::new(0.0, 50.0, 40.0, 40.0)]
//         );
//     }

//     #[test]
//     fn dirty_child_visible_after_inner_scroll_is_not_clipped_by_root_layout_position() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let scroller = arena.insert(
//             root,
//             ContainerWidget::new().style(Style::new().scroll_vertical()),
//             tf::Style::default(),
//         );
//         let child = arena.insert(scroller, ContainerWidget::new(), tf::Style::default());

//         arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 100.0);
//         arena.node_mut(scroller).unwrap().layout = Rect::new(0.0, 0.0, 180.0, 90.0);
//         arena.node_mut(scroller).unwrap().scroll_offset = Point::new(0.0, 80.0);
//         arena.node_mut(child).unwrap().layout = Rect::new(0.0, 132.0, 80.0, 40.0);
//         arena.finish_paint();

//         arena.mark_dirty(child, DirtyFlags::PAINT);

//         assert_eq!(
//             arena.prepare_paint_commands().0.rects(),
//             &[Rect::new(0.0, 52.0, 80.0, 38.0)]
//         );
//     }

//     #[test]
//     fn nested_scroll_invalidates_scrollable_ancestor_gutters() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let scroller = arena.insert(
//             root,
//             ContainerWidget::new().style(Style::new().scroll_vertical()),
//             tf::Style::default(),
//         );
//         let child = arena.insert(scroller, ContainerWidget::new(), tf::Style::default());

//         arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 100.0);
//         arena
//             .node_mut(root)
//             .unwrap()
//             .computed_style
//             .scroll
//             .direction = xui_interface::ScrollDirectionStyle::Both;
//         arena.node_mut(scroller).unwrap().layout = Rect::new(0.0, 0.0, 192.0, 92.0);
//         arena.node_mut(scroller).unwrap().content_size = Size::<f32>::new(192.0, 200.0);
//         arena.node_mut(child).unwrap().layout = Rect::new(0.0, 0.0, 100.0, 200.0);
//         arena.finish_paint();

//         assert!(arena.scroll_node_by(child, Point::new(0.0, -48.0)));

//         let (damage, _commands) = arena.prepare_paint_commands();
//         assert!(damage.rects().contains(&Rect::new(0.0, 0.0, 200.0, 100.0)));
//     }
// }

// let max_width = match known_dimensions.width {
//     Some(width) => Some(width),
//     None => match available_space.width {
//         tf::AvailableSpace::Definite(width) => Some(width),
//         tf::AvailableSpace::MinContent => Some(0.0),
//         tf::AvailableSpace::MaxContent => None,
//     },
// };

// let constraints = match max_width {
//     Some(width) => TextLayoutConstraints::max_width(width),
//     None => TextLayoutConstraints::UNBOUNDED,
// };
