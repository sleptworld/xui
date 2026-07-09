use slotmap::SlotMap;
use std::collections::HashMap;
use std::time::Duration;
use taffy::prelude as tf;
use xui_animation::{Animatable, Timeline, Transition};
use xui_interface::events::RawEvent;
use xui_interface::render::Damage;
use xui_interface::{
    ComputedColorStyle, ComputedScrollStyle, ComputedScrollbarStyle, ComputedStyle, DamageRegion,
    EventResult, NodeId, NodeLifecycleEvent, PaintCommand, ScrollbarVisibilityStyle,
    StyleDiffFlags, TextBackend, TextLayoutConstraints, TextLayoutInput, Theme, Translation,
    Widget, WidgetState, WidgetUpdateFlags,
};

use crate::animation::AnimableStyle;
use crate::core::{Point, Rect, Size};
use crate::event_system::callbacks::{CallbackHandleSet, CallbackStore, EventHandlers};
use crate::event_system::{self, EventState, translator::EventTranslator};
use crate::fiber::Key;
use crate::layout::{computed_style_for_widget, taffy_style_for_widget};
use crate::text::TextHost;
use crate::widgets::{WidgetI, WidgetType};

pub enum WidgetContext {
    Text(NodeId),
    Image(Size<f32>),
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct HostWorkFlags: u8 {
        const RECALC_STYLE = 1 << 0;
        const RECALC_STYLE_SUBTREE = 1 << 1;
        const RECALC_LAYOUT = 1 << 2;
        const REBUILD_PAINT = 1 << 3;
        const SYNC_TREE = 1 << 4;
        const SYNC_STATE_CHANGE = 1 << 5;
    }
}

impl HostWorkFlags {
    fn from_widget_update(flags: WidgetUpdateFlags) -> Self {
        let mut work = Self::empty();
        if flags.intersects(WidgetUpdateFlags::STYLE_TARGET) {
            work |= Self::RECALC_STYLE | Self::RECALC_LAYOUT;
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

        if flags.intersects(WidgetUpdateFlags::STATE_CHANGE) {
            work |= Self::SYNC_STATE_CHANGE;
        }
        work
    }

    fn from_style_diff(flags: StyleDiffFlags) -> Self {
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
        work
    }
}

struct ActiveStyleTransition {
    timeline: Timeline,
    from: AnimableStyle,
    to: AnimableStyle,
}

impl ActiveStyleTransition {
    fn new(
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> Option<Self> {
        let (from, to) = AnimableStyle::diff(from_style, target_style);
        if !to.has_properties() {
            return None;
        }

        Some(Self {
            timeline: Timeline::new(transition),
            from,
            to,
        })
    }

    fn tick(&mut self, delta: Duration, node: &mut Node, theme: &Theme) -> bool {
        let progress = self.timeline.tick(delta);
        if progress.completed {
            node.effective_style = node.target_style.clone();
            return true;
        }

        node.effective_style = node.target_style.clone();
        let interpolated = AnimableStyle::interpolate(&self.from, &self.to, progress.eased);
        interpolated.apply_to_computed(&mut node.effective_style, theme);
        false
    }
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
    work: HostWorkFlags,
    subtree_work: HostWorkFlags,
    pub old_props_hash: u64,
    pub new_props_hash: u64,
    // Style
    pub target_style: ComputedStyle,
    pub effective_style: ComputedStyle,
    style_initialized: bool,
    // State
    pub state: WidgetState,
    pub(crate) state_before_change: Option<WidgetState>,
    // Paint
    pub paint_cache: Vec<PaintCommand>,
    pub widget: WidgetI,
    pub event_callbacks: CallbackHandleSet,
}

impl Node {
    fn new(
        id: NodeId,
        key: Option<Key>,
        position: usize,
        props_hash: u64,
        target_style: ComputedStyle,
        widget: WidgetI,
        event_callbacks: CallbackHandleSet,
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
            work: HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT,
            subtree_work: HostWorkFlags::empty(),
            old_props_hash: 0,
            new_props_hash: props_hash,
            effective_style: target_style.clone(),
            target_style,
            style_initialized: false,
            // state
            state: WidgetState::default(),
            state_before_change: None,
            paint_cache: Vec::new(),
            widget,
            event_callbacks,
        }
    }

    #[inline(always)]
    fn scroll_style(&self) -> &ComputedScrollStyle {
        &self.target_style.scroll
    }
}

#[derive(Default)]
struct UiState {
    animation_driver: AnimationDriver,
    layout_dirty_list: Vec<NodeId>,
    style_subtree_dirty_list: Vec<NodeId>,
    style_dirty_list: Vec<NodeId>,
    state_change_dirty_list: Vec<NodeId>,
}

impl UiState {
    #[inline]
    fn mark_layout_dirty(&mut self, id: NodeId) {
        self.layout_dirty_list.push(id);
    }

    #[inline]
    fn start_style_transition(
        &mut self,
        id: NodeId,
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> bool {
        self.animation_driver
            .start_node(id, transition, from_style, target_style)
    }

    #[inline]
    fn mark_style_subtree_dirty(&mut self, id: NodeId) {
        self.style_subtree_dirty_list.push(id);
    }

    #[inline]
    fn mark_state_change_dirty(&mut self, id: NodeId) {
        self.state_change_dirty_list.push(id);
    }

    #[inline]
    fn mark_style_dirty(&mut self, id: NodeId) {
        self.style_dirty_list.push(id);
    }

    fn drain_subtree_dirty_list(&mut self) -> Vec<NodeId> {
        let mut list = std::mem::take(&mut self.style_subtree_dirty_list);
        list.sort();
        list.dedup();
        list
    }

    fn drain_style_dirty_list(&mut self) -> Vec<NodeId> {
        let mut list = std::mem::take(&mut self.style_dirty_list);
        list.sort();
        list.dedup();
        list
    }

    fn drain_state_change_dirty_list(&mut self) -> Vec<NodeId> {
        let mut list = std::mem::take(&mut self.state_change_dirty_list);
        list.sort();
        list.dedup();
        list
    }
}

#[derive(Default)]
struct AnimationDriver {
    nodes: HashMap<NodeId, ActiveStyleTransition>,
}

impl AnimationDriver {
    fn start_node(
        &mut self,
        id: NodeId,
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> bool {
        let Some(animation) = ActiveStyleTransition::new(transition, from_style, target_style)
        else {
            self.nodes.remove(&id);
            return false;
        };

        self.nodes.insert(id, animation);
        true
    }

    fn remove_node(&mut self, id: NodeId) {
        self.nodes.remove(&id);
    }

    fn is_running(&self) -> bool {
        !self.nodes.is_empty()
    }

    fn take_nodes(&mut self) -> HashMap<NodeId, ActiveStyleTransition> {
        std::mem::take(&mut self.nodes)
    }

    fn set_nodes(&mut self, nodes: HashMap<NodeId, ActiveStyleTransition>) {
        self.nodes = nodes;
    }
}

pub struct UiArena {
    nodes: SlotMap<NodeId, Node>,
    taffy: tf::TaffyTree<WidgetContext>,
    root: NodeId,
    damage: DamageRegion,
    damage_nodes: Vec<NodeId>,
    node_lifecycle_events: Vec<NodeLifecycleEvent>,
    pub event_state: EventState,
    event_callbacks: CallbackStore,
    theme: Theme,
    pub update_visits: usize,
    pub layout_passes: usize,
    pub repaint_passes: usize,
    default_style: ComputedStyle,
    ui_state: UiState,
}

pub struct PaintFrame {
    pub damage: DamageRegion,
    pub commands: Vec<PaintCommand>,
}

impl UiArena {
    pub fn new() -> Self {
        let mut taffy = tf::TaffyTree::new();
        let theme = Theme::default();
        let root_widget = crate::widgets::root_widget();
        let root_parent_style = ComputedStyle::initial(&theme);
        let root_computed_style = computed_style_for_widget(
            &root_widget,
            &root_parent_style,
            &theme,
            WidgetState::empty(),
        );
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
                root_computed_style,
                root_widget,
                CallbackHandleSet::default(),
                taffy_root,
            )
        });
        nodes[root].style_initialized = true;
        let default_style = ComputedStyle::initial(&theme);
        Self {
            nodes,
            taffy,
            root,
            damage: DamageRegion::new(),
            damage_nodes: vec![],
            node_lifecycle_events: Vec::new(),
            event_state: EventState::default(),
            event_callbacks: CallbackStore::default(),
            theme,
            update_visits: 0,
            layout_passes: 0,
            repaint_passes: 0,
            ui_state: UiState::default(),
            default_style,
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
            self.mark_work(
                self.root,
                HostWorkFlags::RECALC_STYLE
                    | HostWorkFlags::RECALC_STYLE_SUBTREE
                    | HostWorkFlags::RECALC_LAYOUT
                    | HostWorkFlags::REBUILD_PAINT,
            );
        }
    }

    pub(crate) fn event_state(&self) -> &EventState {
        &self.event_state
    }

    pub(crate) fn event_state_mut(&mut self) -> &mut EventState {
        &mut self.event_state
    }

    pub(crate) fn event_callbacks(&mut self) -> &mut CallbackStore {
        &mut self.event_callbacks
    }

    #[cfg(test)]
    pub(crate) fn set_event_handlers(&mut self, id: NodeId, event_handlers: EventHandlers) {
        let Some(current) = self.nodes.get(id).map(|node| node.event_callbacks) else {
            return;
        };
        let event_callbacks = self.event_callbacks.update_set(current, event_handlers);
        if let Some(node) = self.nodes.get_mut(id) {
            node.event_callbacks = event_callbacks;
        }
    }

    pub fn create_node(
        &mut self,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        event_handlers: EventHandlers,
    ) -> NodeId {
        let taffy_node = self
            .taffy
            .new_leaf(tf::Style::default())
            .expect("failed to create taffy node");
        let event_callbacks = self
            .event_callbacks
            .update_set(CallbackHandleSet::default(), event_handlers);

        let id = self.nodes.insert_with_key(|id| {
            Node::new(
                id,
                key,
                0,
                props_hash,
                self.default_style.clone(),
                widget,
                event_callbacks,
                taffy_node,
            )
        });
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Created(id));
        self.refresh_taffy_context(id);
        self.mark_work(
            id,
            HostWorkFlags::RECALC_STYLE
                | HostWorkFlags::RECALC_LAYOUT
                | HostWorkFlags::REBUILD_PAINT,
        );

        id
    }

    pub fn to_local(&self, node_id: NodeId, viewport_pos: Point) -> Option<Point> {
        let node = self.node(node_id)?;

        if let Some(parent_id) = node.parent {
            let parent = self.node(parent_id)?;

            let parent_local = self.to_local(parent_id, viewport_pos)?;
            let parent_content_pos = parent_local + parent.scroll_offset;

            let node_pos = Point {
                x: node.layout.x,
                y: node.layout.y,
            };

            Some(parent_content_pos - node_pos)
        } else {
            let root_pos = Point {
                x: node.layout.x,
                y: node.layout.y,
            };

            Some(viewport_pos - root_pos)
        }
    }

    pub fn to_content_local(&self, node_id: NodeId, viewport_pos: Point) -> Option<Point> {
        let node = self.node(node_id)?;

        let local = self.to_local(node_id, viewport_pos)?;

        Some(local + node.scroll_offset)
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
            self.mark_work(
                old_parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
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
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
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
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
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
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
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
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
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
        self.mark_work(
            parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
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
            self.mark_work(
                parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
        }

        self.event_state.clear_node(id);
        self.event_callbacks
            .clear_set(self.nodes[id].event_callbacks);
        self.ui_state.animation_driver.remove_node(id);

        let _ = self.taffy.remove(self.nodes[id].taffy_node);
        self.nodes.remove(id);
        self.node_lifecycle_events
            .push(NodeLifecycleEvent::Removed(id));
    }

    pub fn drain_node_lifecycle_events(&mut self) -> Vec<NodeLifecycleEvent> {
        std::mem::take(&mut self.node_lifecycle_events)
    }

    pub fn mark_dirty(&mut self, id: NodeId, flags: WidgetUpdateFlags) {
        self.mark_work(id, HostWorkFlags::from_widget_update(flags));
    }

    fn mark_work(&mut self, id: NodeId, flags: HostWorkFlags) {
        if flags.is_empty() || !self.nodes.contains_key(id) {
            return;
        }
        {
            let node = self.nodes.get_mut(id).expect("checked node existence");
            node.work |= flags;
        }

        if flags.intersects(HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::SYNC_TREE) {
            self.ui_state.mark_layout_dirty(id);
        }

        if flags.intersects(HostWorkFlags::RECALC_STYLE_SUBTREE) {
            self.ui_state.mark_style_subtree_dirty(id);
        }

        if flags.intersects(HostWorkFlags::RECALC_STYLE) {
            self.ui_state.mark_style_dirty(id);
        }

        if flags.intersects(HostWorkFlags::SYNC_STATE_CHANGE) {
            self.ui_state.mark_state_change_dirty(id);
        }

        let mut current = id;
        while let Some(parent) = self.nodes[current].parent {
            self.nodes[parent].subtree_work |= flags;
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
            let scroll_style = node.scroll_style();
            if scroll_style.direction.is_scrollable() {
                total_scroll_offset.x += node.scroll_offset.x;
                total_scroll_offset.y += node.scroll_offset.y;
            }
        }
        rect.x -= total_scroll_offset.x;
        rect.y -= total_scroll_offset.y;

        let mut clip_scroll_offset = Point::zero();
        for ancestor in ancestors.into_iter().rev() {
            let node = &self.nodes[ancestor];
            let scroll_style = node.scroll_style();
            let scrollable = scroll_style.direction.is_scrollable();

            let node_style = &node.target_style;

            if node_style.paint.clip || scrollable {
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

    pub fn clear_work(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.old_props_hash = node.new_props_hash;
            node.work = HostWorkFlags::empty();
            node.subtree_work = HostWorkFlags::empty();
        }
    }

    pub fn mark_subtree_layout_dirty(&mut self, id: NodeId) {
        self.mark_work(
            id,
            HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT,
        );
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
        let node_style = &node.target_style;

        let child_scroll_offset = if node_style.scroll.direction.is_scrollable() {
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

        let direction = node.scroll_style().direction;
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
        self.mark_work(id, HostWorkFlags::REBUILD_PAINT);
        true
    }

    fn add_scroll_damage(&mut self, id: NodeId, rect: Rect) {
        self.add_node_damage(id, rect);

        let mut cursor = self.nodes.get(id).and_then(|node| node.parent);
        while let Some(parent) = cursor {
            let node = &self.nodes[parent];
            let layout = node.layout;
            let next_parent = node.parent;
            let node_style = &node.target_style;
            let needs_damage = node_style.paint.clip || node_style.scroll.direction.is_scrollable();
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

    #[inline(always)]
    pub fn dispatch_event(
        &mut self,
        translator: &mut EventTranslator,
        event: RawEvent,
    ) -> EventResult {
        event_system::dispatch_event(self, translator, event)
    }

    pub fn tick_style_animations(&mut self, delta: Duration) -> bool {
        if !self.ui_state.animation_driver.is_running() {
            return false;
        }

        let mut changed = false;
        let mut remaining = HashMap::new();
        let active = self.ui_state.animation_driver.take_nodes();

        for (id, mut animation) in active {
            if !self.nodes.contains_key(id) {
                continue;
            }

            let completed = {
                let node = self.nodes.get_mut(id).expect("checked node existence");
                animation.tick(delta, node, &self.theme)
            };

            self.mark_work(id, HostWorkFlags::REBUILD_PAINT);
            changed = true;
            if !completed {
                remaining.insert(id, animation);
            }
        }

        self.ui_state.animation_driver.set_nodes(remaining);
        changed
    }

    pub fn has_running_style_animations(&self) -> bool {
        self.ui_state.animation_driver.is_running()
    }

    pub fn update_tree<T: TextBackend>(&mut self, size: Size<f32>, measurer: &mut TextHost<T>) {
        for node_id in self.ui_state.drain_state_change_dirty_list() {
            self.recompute_node_state(node_id);
        }

        // Fiber-style bailout: skip the whole branch when neither this node nor
        // any descendant has scheduled work.
        for node_id in self.ui_state.drain_style_dirty_list() {
            self.recompute_node_style(node_id);
        }

        for node_id in self.ui_state.drain_subtree_dirty_list() {
            self.recompute_subtree_styles(node_id);
        }

        // Recompute layout if needed.
        if self.has_layout_dirty() {
            self.compute_layout(size, measurer);
        }

        self.ui_state.layout_dirty_list.clear();
        self.rebuild_subtree_dirty(self.root);
        self.repaint_dirty_subtree(self.root);
        self.clear_work_subtree(self.root, HostWorkFlags::all());
    }

    fn recompute_node_state(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let state = self.nodes[id].state;
        let before = self.nodes[id].state_before_change.take().unwrap_or(state);
        let widget = self.nodes[id].widget.clone();

        // Recompute style if the widget's style affects the state change.
        if widget.with_widgets(|w| w.style().affects_state_change(before, state)) {
            self.mark_dirty(id, WidgetUpdateFlags::STYLE_TARGET);
        }
    }

    fn recompute_subtree_styles(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        self.recompute_node_style(id);

        let children = self.nodes[id].children.clone();
        for child in children {
            self.recompute_subtree_styles(child);
        }
    }

    fn recompute_node_style(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let widget = self.nodes[id].widget.clone();
        let state = self.nodes[id].state;
        let parent_style = self.nodes[id]
            .parent
            .and_then(|p| self.node(p))
            .map(|p| &p.target_style)
            .unwrap_or(&self.default_style);

        let computed_style = computed_style_for_widget(&widget, parent_style, &self.theme, state);
        let taffy_style = taffy_style_for_widget(&widget, parent_style, &computed_style);

        let node = self.node(id).unwrap();
        let current_taffy_style = self
            .taffy
            .style(node.taffy_node)
            .expect("Missing taffy node");

        let style_diff = node.target_style.diff(&computed_style);
        let mut work_flags = HostWorkFlags::from_style_diff(style_diff);
        if *current_taffy_style != taffy_style {
            self.taffy
                .set_style(node.taffy_node, taffy_style)
                .expect("failed to update taffy style");
            work_flags |= HostWorkFlags::RECALC_LAYOUT | HostWorkFlags::REBUILD_PAINT;
        }

        let transition = widget.transition();
        if !self.nodes[id].style_initialized {
            let node_mut = self.node_mut(id).unwrap();
            node_mut.target_style = computed_style;
            node_mut.effective_style = node_mut.target_style.clone();
            node_mut.style_initialized = true;
            node_mut.work |= work_flags;
            self.refresh_taffy_context(id);
            return;
        }

        if work_flags.is_empty() {
            return;
        }

        let from_style = self.nodes[id].effective_style.clone();
        let started_transition = transition
            .map(|transition| {
                self.ui_state
                    .start_style_transition(id, transition, &from_style, &computed_style)
            })
            .unwrap_or_else(|| {
                self.ui_state.animation_driver.remove_node(id);
                false
            });
        self.nodes[id].target_style = computed_style;

        if started_transition {
            work_flags |= HostWorkFlags::REBUILD_PAINT;
        } else {
            let effective_style = self.nodes[id].target_style.clone();
            self.nodes[id].effective_style = effective_style;
        }

        self.refresh_taffy_context(id);

        if work_flags.intersects(HostWorkFlags::RECALC_STYLE_SUBTREE) {
            self.ui_state.mark_style_subtree_dirty(id);
        }

        let node_mut = self.node_mut(id).unwrap();
        node_mut.work |= work_flags;
    }

    pub fn compute_layout_if_needed<T: TextBackend>(
        &mut self,
        size: Size<f32>,
        measurer: &mut TextHost<T>,
    ) {
        if !self.has_layout_dirty() {
            return;
        }
        self.compute_layout(size, measurer);
    }

    #[inline(always)]
    fn has_layout_dirty(&self) -> bool {
        self.nodes
            .values()
            .any(|node| node.work.intersects(Self::layout_work_flags()))
    }

    #[inline(always)]
    fn layout_work_flags() -> HostWorkFlags {
        HostWorkFlags::RECALC_LAYOUT
    }

    #[inline(always)]
    fn paint_work_flags() -> HostWorkFlags {
        HostWorkFlags::REBUILD_PAINT
    }

    fn effective_style(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.nodes.get(id).map(|node| &node.effective_style)
    }

    pub fn repaint_if_needed(&mut self, id: NodeId) {
        let should_repaint = self
            .nodes
            .get(id)
            .is_some_and(|node| node.work.intersects(HostWorkFlags::REBUILD_PAINT));
        if !should_repaint {
            return;
        }

        self.repaint_passes += 1;
        let rect = self.nodes[id].layout;
        let style = self
            .effective_style(id)
            .expect("checked node existence before repaint");
        let mut cache = Vec::new();
        self.nodes[id].widget.paint(rect, style, &mut cache);
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

    pub fn compute_layout<T: TextBackend>(&mut self, size: Size<f32>, measurer: &mut TextHost<T>) {
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
                        &self.nodes,
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

    fn sync_layout(&mut self, id: NodeId, offset_x: f32, offset_y: f32) -> HostWorkFlags {
        if !self.nodes.contains_key(id) {
            return HostWorkFlags::empty();
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

        let (children, mut subtree_work) = {
            let node = &mut self.nodes[id];
            let should_sync_children = layout_changed
                || node.work.intersects(Self::layout_work_flags())
                || node.subtree_work.intersects(Self::layout_work_flags());

            node.previous_layout = node.layout;
            node.layout = rect;
            node.work.remove(HostWorkFlags::RECALC_LAYOUT);
            if layout_changed {
                node.work.insert(HostWorkFlags::REBUILD_PAINT);
            }

            if should_sync_children {
                (node.children.clone(), HostWorkFlags::empty())
            } else {
                return node.work | node.subtree_work;
            }
        };

        for child in children {
            subtree_work |= self.sync_layout(child, rect.x, rect.y);
        }

        let content_size = self.content_size_from_children(id, taffy_content_size);
        let (scroll_dirty, rect) = {
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            let content_size_changed = node.content_size != content_size;
            let scroll_offset_before_clamp = node.scroll_offset;
            node.content_size = content_size;
            clamp_scroll_offset(node);
            (
                node.target_style.scroll.direction.is_scrollable()
                    && (content_size_changed || node.scroll_offset != scroll_offset_before_clamp),
                node.layout,
            )
        };
        if scroll_dirty {
            self.add_node_damage(id, rect);
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            node.work.insert(HostWorkFlags::REBUILD_PAINT);
        }
        let node = self.nodes.get_mut(id).expect("node removed during layout");
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
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
            .map(|node| {
                matches!(
                    node.widget.node_type(),
                    WidgetType::Text | WidgetType::TextInput
                )
            })
            .unwrap_or(false)
    }

    fn repaint_dirty_subtree(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let work = self.nodes[id].work;
        let subtree_work = self.nodes[id].subtree_work;
        if !work.intersects(Self::paint_work_flags())
            && !subtree_work.intersects(Self::paint_work_flags())
        {
            return;
        }

        if work.intersects(Self::paint_work_flags()) {
            self.repaint_if_needed(id);
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.repaint_dirty_subtree(child);
        }
    }

    fn clear_work_subtree(&mut self, id: NodeId, flags: HostWorkFlags) -> HostWorkFlags {
        if !self.nodes.contains_key(id) {
            return HostWorkFlags::empty();
        }

        let children = self.nodes[id].children.clone();
        let mut subtree_work = HostWorkFlags::empty();
        for child in children {
            subtree_work |= self.clear_work_subtree(child, flags);
        }

        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.old_props_hash = node.new_props_hash;
        node.work.remove(flags);
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    fn rebuild_subtree_dirty(&mut self, id: NodeId) -> HostWorkFlags {
        if !self.nodes.contains_key(id) {
            return HostWorkFlags::empty();
        }

        let children = self.nodes[id].children.clone();
        let mut subtree_work = HostWorkFlags::empty();
        for child in children {
            subtree_work |= self.rebuild_subtree_dirty(child);
        }

        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.subtree_work = subtree_work;
        node.work | node.subtree_work
    }

    // pub fn collect_paint_commands(&mut self) -> (DamageRegion, Vec<PaintCommand>) {
    //     let (damage, cmds) = self.prepare_paint_commands();
    //     self.finish_paint();
    //     (damage, cmds)
    // }

    pub fn collect_paint_commands(&self) -> PaintFrame {
        let damage = self.damage.clone();
        let mut commands = Vec::new();
        if !damage.is_empty() {
            self.paint_node(self.root, &damage, &mut commands);
        }

        PaintFrame { damage, commands }
    }

    pub fn prepare_paint_commands(&self) -> (&DamageRegion, Vec<PaintCommand>) {
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
        self.clear_work_subtree(self.root, HostWorkFlags::REBUILD_PAINT);
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
            let scrollable = node.target_style.scroll.direction.is_scrollable();
            if node.target_style.paint.clip || scrollable {
                commands.push(PaintCommand::PushClip(node.layout));
            }
            if node.paint_cache.is_empty() {
                node.widget
                    .paint(node.layout, &node.effective_style, commands);
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
            if node.target_style.paint.clip || scrollable {
                commands.push(PaintCommand::PopClip);
            }
        }

        Some(())
    }

    pub fn is_dirty(&self) -> bool {
        !self.damage.is_empty()
            || self.has_running_style_animations()
            || !self.ui_state.style_dirty_list.is_empty()
            || !self.ui_state.style_subtree_dirty_list.is_empty()
            || !self.ui_state.layout_dirty_list.is_empty()
            || self
                .nodes
                .values()
                .any(|node| !node.work.is_empty() || !node.subtree_work.is_empty())
    }

    pub fn update_widget_node_from_parts(
        &mut self,
        id: NodeId,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        event_handlers: EventHandlers,
    ) -> WidgetI {
        let mut flags = WidgetUpdateFlags::empty();
        let current_widget;
        let event_callbacks = {
            let current = self
                .nodes
                .get(id)
                .expect("reused node missing")
                .event_callbacks;
            self.event_callbacks.update_set(current, event_handlers)
        };

        {
            let node = self.nodes.get_mut(id).expect("reused node missing");
            node.key = key;
            node.new_props_hash = props_hash;

            if node.node_type != widget.node_type() {
                flags |= WidgetUpdateFlags::TREE;
            }

            let widget_flags = node.widget.update_from(&widget);

            flags |= widget_flags;
            node.event_callbacks = event_callbacks;
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
        self.mark_work(
            old_parent,
            HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
        );
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
                    self.mark_work(
                        old_parent,
                        HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
                    );
                }
                self.nodes[child].parent = Some(parent);
                self.nodes[child].position = new_position;
                self.record_node_move(child, old_parent, Some(parent), old_position, new_position);
            }
        }
        self.reindex_children(parent);
        self.sync_taffy_children(parent);

        if tree_changed {
            self.mark_work(
                parent,
                HostWorkFlags::SYNC_TREE | HostWorkFlags::RECALC_LAYOUT,
            );
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
            WidgetType::Text | WidgetType::TextInput => Some(WidgetContext::Text(id)),
            WidgetType::Image => node.widget.intrinsic_size().map(WidgetContext::Image),
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
mod tests {
    use super::*;
    use crate::event_system::callbacks::EventHandlers;
    use crate::event_system::translator::EventTranslator;
    use crate::text::testing::ZeroTextBackend;
    use crate::widgets::{WidgetI, container};
    use std::time::{Duration, Instant};
    use xui_animation::{Easing, Transition};
    use xui_interface::events::{
        Modifiers, PointerButtons, PointerKind, RawPointerMove, XuiPointerId,
    };
    use xui_interface::{Color, ComputedColorStyle, Style, WidgetState};

    fn create_host(arena: &mut UiArena, widget: WidgetI) -> NodeId {
        let parent = arena.root();
        let key = widget.key();
        let props_hash = widget.props_hash();
        let id = arena.create_node(key, props_hash, widget, EventHandlers::default());
        arena.append_child(parent, id);
        id
    }

    fn update_host(arena: &mut UiArena, id: NodeId, widget: WidgetI) {
        let key = widget.key();
        let props_hash = widget.props_hash();
        arena.update_widget_node_from_parts(id, key, props_hash, widget, EventHandlers::default());
    }

    fn pointer_move(position: Point) -> RawEvent {
        RawEvent::PointerMove(RawPointerMove {
            position,
            pointer_id: XuiPointerId::new(0),
            device_id: None,
            kind: PointerKind::Mouse,
            button: None,
            buttons: PointerButtons::default(),
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
        })
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

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be near {expected}"
        );
    }

    #[test]
    fn style_diff_domains_map_to_host_work_without_mixing_public_flags() {
        let text = HostWorkFlags::from_style_diff(StyleDiffFlags::TEXT);
        assert!(text.contains(HostWorkFlags::REBUILD_PAINT));
        assert!(text.contains(HostWorkFlags::RECALC_STYLE_SUBTREE));
        assert!(!text.contains(HostWorkFlags::RECALC_LAYOUT));

        let layout = HostWorkFlags::from_style_diff(StyleDiffFlags::LAYOUT);
        assert!(layout.contains(HostWorkFlags::RECALC_LAYOUT));
        assert!(layout.contains(HostWorkFlags::REBUILD_PAINT));
        assert!(!layout.contains(HostWorkFlags::RECALC_STYLE));

        let paint = HostWorkFlags::from_style_diff(StyleDiffFlags::PAINT);
        assert_eq!(paint, HostWorkFlags::REBUILD_PAINT);

        let scroll = HostWorkFlags::from_style_diff(StyleDiffFlags::SCROLL);
        assert!(scroll.contains(HostWorkFlags::RECALC_LAYOUT));
        assert!(scroll.contains(HostWorkFlags::REBUILD_PAINT));
    }

    #[test]
    fn style_target_change_starts_transition_and_repaints_effective_style() {
        let mut arena = UiArena::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
        let initial = WidgetI::new(
            container()
                .style(
                    Style::new()
                        .width(40.0)
                        .height(20.0)
                        .background(Color::BLACK),
                )
                .transition(transition),
        );
        let id = create_host(&mut arena, initial);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        let next = WidgetI::new(
            container()
                .style(
                    Style::new()
                        .width(40.0)
                        .height(20.0)
                        .background(Color::WHITE),
                )
                .transition(transition),
        );
        update_host(&mut arena, id, next);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert!(arena.has_running_style_animations());
        assert_background_near(&arena.nodes[id].effective_style, Color::BLACK);

        arena.tick_style_animations(Duration::from_millis(50));
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert_background_near(&arena.nodes[id].effective_style, Color::rgb(0.5, 0.5, 0.5));
        let PaintCommand::Rect { color, .. } = arena.nodes[id].paint_cache.first().unwrap() else {
            panic!("expected rect paint command");
        };
        let ComputedColorStyle::Solid(color) = *color else {
            panic!("expected solid painted background");
        };
        assert_near(color.r, 0.5);

        arena.tick_style_animations(Duration::from_millis(50));
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert!(!arena.has_running_style_animations());
        assert_background_near(&arena.nodes[id].effective_style, Color::WHITE);
    }

    #[test]
    fn semantic_hover_updates_widget_state_and_recomputes_style() {
        let mut arena = UiArena::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let id = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(40.0)
                        .height(20.0)
                        .background(Color::BLACK)
                        .when(WidgetState::HOVERED, |s| s.background(Color::WHITE)),
                ),
            ),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        let mut translator = EventTranslator::default();
        arena.dispatch_event(&mut translator, pointer_move(Point::new(1.0, 1.0)));
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert!(arena.nodes[id].state.contains(WidgetState::HOVERED));
        assert_background_near(&arena.nodes[id].target_style, Color::WHITE);
        assert_background_near(&arena.nodes[id].effective_style, Color::WHITE);
    }

    #[test]
    fn semantic_hover_leave_recomputes_default_style() {
        let mut arena = UiArena::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let id = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(40.0)
                        .height(20.0)
                        .background(Color::BLACK)
                        .when(WidgetState::HOVERED, |s| s.background(Color::WHITE)),
                ),
            ),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        let mut translator = EventTranslator::default();
        arena.dispatch_event(&mut translator, pointer_move(Point::new(1.0, 1.0)));
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        arena.dispatch_event(&mut translator, pointer_move(Point::new(80.0, 80.0)));
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert!(!arena.nodes[id].state.contains(WidgetState::HOVERED));
        assert_background_near(&arena.nodes[id].target_style, Color::BLACK);
        assert_background_near(&arena.nodes[id].effective_style, Color::BLACK);
    }

    #[test]
    fn target_layout_change_marks_layout_and_damage() {
        let mut arena = UiArena::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let id = create_host(
            &mut arena,
            WidgetI::new(container().style(Style::new().width(10.0).height(10.0))),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        arena.finish_paint();

        update_host(
            &mut arena,
            id,
            WidgetI::new(container().style(Style::new().width(20.0).height(10.0))),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        assert_near(arena.nodes[id].layout.width, 20.0);
        let frame = arena.collect_paint_commands();
        assert!(!frame.damage.is_empty());
    }

    #[test]
    fn damage_generates_paint_commands_and_finish_clears_frame_damage() {
        let mut arena = UiArena::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(40.0)
                        .height(20.0)
                        .background(Color::BLACK),
                ),
            ),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        let frame = arena.collect_paint_commands();
        assert!(!frame.damage.is_empty());
        assert!(!frame.commands.is_empty());

        arena.finish_paint();
        let frame = arena.collect_paint_commands();
        assert!(frame.damage.is_empty());
        assert!(frame.commands.is_empty());
    }

    #[test]
    fn mark_dirty_schedules_work_without_collecting_damage() {
        let mut arena = UiArena::new();
        let mut measurer = TextHost::new(ZeroTextBackend);
        let id = create_host(
            &mut arena,
            WidgetI::new(
                container().style(
                    Style::new()
                        .width(40.0)
                        .height(20.0)
                        .background(Color::BLACK),
                ),
            ),
        );
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        arena.finish_paint();

        arena.mark_dirty(id, WidgetUpdateFlags::PAINT_OUTPUT);

        assert!(arena.damage.is_empty());

        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        let frame = arena.collect_paint_commands();
        assert!(!frame.damage.is_empty());
        assert!(!frame.commands.is_empty());
    }
}

fn measure_layout_context<T: TextBackend>(
    ui_tree: &SlotMap<NodeId, Node>,
    known_dimensions: tf::Size<Option<f32>>,
    available_space: tf::Size<tf::AvailableSpace>,
    node_context: Option<&mut WidgetContext>,
    measurer: &mut TextHost<T>,
) -> tf::Size<f32> {
    let known_size = if let tf::Size {
        width: Some(width),
        height: Some(height),
    } = known_dimensions
    {
        Some(tf::Size { width, height })
    } else {
        None
    };

    let measured = match node_context {
        Some(WidgetContext::Text(node_id)) => {
            let node = ui_tree.get(*node_id).expect("node not found");
            if let Some(props) = node
                .widget
                .with_widgets(|w| w.text_layout_props(&node.effective_style))
            {
                let constraints = match known_dimensions.width {
                    Some(width) => TextLayoutConstraints::max_width(width),
                    None => match available_space.width {
                        tf::AvailableSpace::MaxContent => TextLayoutConstraints::UNBOUNDED,
                        tf::AvailableSpace::MinContent => TextLayoutConstraints::MIN_SIZE,
                        tf::AvailableSpace::Definite(width) => {
                            TextLayoutConstraints::max_width(width)
                        }
                    },
                };

                let font_context = measurer.backend().epoch();
                let input = TextLayoutInput::new(
                    props.text,
                    constraints,
                    props.style.into(),
                    props.paragraph,
                    font_context,
                );
                let layout = measurer.simple_doc(*node_id, input);
                let size = layout.kind.size();
                return tf::Size {
                    width: known_dimensions.width.unwrap_or(size.width),
                    height: known_dimensions.height.unwrap_or(size.height),
                };
            } else {
                return tf::Size {
                    width: known_dimensions.width.unwrap_or(0.0),
                    height: known_dimensions.height.unwrap_or(0.0),
                };
            }
        }
        Some(WidgetContext::Image(size)) => {
            return tf::Size {
                width: known_dimensions.width.unwrap_or(size.width),
                height: known_dimensions.height.unwrap_or(size.height),
            };
        }

        _ => {
            if let Some(size) = known_size {
                return size;
            }
            Size::<f32>::ZERO
        }
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
    let direction = node.target_style.scroll.direction;
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
    let direction = node.target_style.scroll.direction;
    let scrollbar = node.target_style.scroll.scrollbar;
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

//         arena.mark_dirty(scroller, WidgetUpdateFlags::PAINT_OUTPUT);

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

//         arena.mark_dirty(child, WidgetUpdateFlags::PAINT_OUTPUT);

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

//         arena.mark_dirty(child, WidgetUpdateFlags::PAINT_OUTPUT);

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
