use slotmap::SlotMap;
use std::cell::Cell;
use taffy::prelude as tf;
use xui_interface::{
    ComputedColorStyle, ComputedScrollbarStyle, ComputedStyle, ComputedTextStyle, DirtyFlags,
    EventHandlers, NodeId, NodeLifecycleEvent, ScrollbarVisibilityStyle, TextContent,
    TextLayoutConstraints, TextMeasurer, Theme, Translation,
};

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
    pub paint_cache: Vec<PaintCommand>,
    pub widget: WidgetI,
    pub event_handlers: EventHandlerSet,
    // Text
}

struct PaintTraceNode {
    id: NodeId,
    node_type: WidgetType,
    key: Option<Key>,
    rect: Rect,
    dirty: DirtyFlags,
    subtree_dirty: DirtyFlags,
    command_start: usize,
    own_command_end: usize,
    close_command_index: Option<usize>,
    subtree_command_end: usize,
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
            paint_cache: Vec::new(),
            widget,
            event_handlers,
        }
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
        if flags.intersects(DirtyFlags::PAINT | DirtyFlags::STYLE) {
            self.add_node_damage(id, rect);
        }

        // A parent with no own work can still find the dirty branch below it
        // through this aggregate subtree flag.
        let mut current = id;
        while let Some(parent) = self.nodes[current].parent {
            self.nodes[parent].subtree_dirty |= flags;
            current = parent;
        }
    }

    pub fn add_damage(&mut self, rect: Rect) {
        self.damage.add(rect);
    }

    fn add_node_damage(&mut self, id: NodeId, rect: Rect) {
        if let Some(rect) = self.visual_damage_rect_for_node(id, rect) {
            self.damage.add(rect);
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
        let (computed_style, taffy_style) =
            if let Some(p) = self.nodes[id].parent.and_then(|p| self.node(p)) {
                let parent_style = &p.computed_style;
                let computed_style = computed_style_for_widget(&widget, parent_style, &self.theme);
                let style = taffy_style_for_widget(&widget, &parent_style, &computed_style);
                (computed_style, style)
            } else {
                let parent_style = ComputedStyle::initial(&self.theme);
                let computed_style = computed_style_for_widget(&widget, &parent_style, &self.theme);
                let style = taffy_style_for_widget(&widget, &parent_style, &computed_style);
                (computed_style, style)
            };

        let mut changed = false;
        let mut refresh_context = false;

        {
            let taffy_node_id = self
                .nodes
                .get(id)
                .map(|n| n.taffy_node)
                .expect("checked node existence");

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

    fn has_layout_dirty(&self) -> bool {
        self.nodes
            .values()
            .any(|node| node.dirty.intersects(Self::layout_dirty_flags()))
    }

    fn layout_dirty_flags() -> DirtyFlags {
        DirtyFlags::LAYOUT | DirtyFlags::STYLE | DirtyFlags::TREE
    }

    fn paint_dirty_flags() -> DirtyFlags {
        DirtyFlags::PAINT | DirtyFlags::LAYOUT | DirtyFlags::STYLE
    }

    pub fn repaint_if_needed(&mut self, id: NodeId) {
        let should_repaint = self.nodes.get(id).is_some_and(|node| {
            node.dirty
                .intersects(DirtyFlags::PAINT | DirtyFlags::LAYOUT | DirtyFlags::STYLE)
        });
        if !should_repaint {
            return;
        }

        self.repaint_passes += 1;
        let rect = self.nodes[id].layout;
        let style = self.nodes[id].computed_style.clone();
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
        let layout = if self.node_uses_unrounded_layout(id) {
            *self.taffy.unrounded_layout(taffy_node)
        } else {
            *self
                .taffy
                .layout(taffy_node)
                .expect("missing taffy layout result")
        };
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

    pub fn collect_paint_commands(&mut self) -> (DamageRegion, Vec<PaintCommand>) {
        let (damage, commands) = self.prepare_paint_commands();
        self.finish_paint();
        (damage, commands)
    }

    pub fn prepare_paint_commands(&self) -> (DamageRegion, Vec<PaintCommand>) {
        let damage = self.damage.clone();
        let mut commands = Vec::new();
        if damage.is_empty() {
            return (damage, commands);
        }
        let mut paint_region = DamageRegion::new();
        if let Some(bounds) = damage.bounds() {
            paint_region.add(bounds);
        }
        self.paint_node(self.root, &paint_region, &mut commands);
        (damage, commands)
    }

    pub fn finish_paint(&mut self) {
        self.damage = DamageRegion::new();
        self.damage_nodes.clear();
        for (_, node) in self.nodes.iter_mut() {
            node.dirty.remove(DirtyFlags::PAINT);
        }
    }

    #[inline(always)]
    fn paint_node(&self, id: NodeId, damage: &DamageRegion, commands: &mut Vec<PaintCommand>) {
        self.paint_node_inner(id, damage, commands, false, Point::zero());
    }

    fn paint_node_inner(
        &self,
        id: NodeId,
        damage: &DamageRegion,
        commands: &mut Vec<PaintCommand>,
        force: bool,
        scroll_offset: Point,
    ) {
        let node = match self.nodes.get(id) {
            Some(node) => node,
            None => return,
        };

        let visual_layout = Rect::new(
            node.layout.x - scroll_offset.x,
            node.layout.y - scroll_offset.y,
            node.layout.width,
            node.layout.height,
        );

        if force || damage.intersects(visual_layout) {
            let scrollable = node.computed_style.scroll.direction.is_scrollable();
            if node.computed_style.paint.clip || scrollable {
                commands.push(PaintCommand::PushClip(node.layout));
            }
            if node.paint_cache.is_empty() {
                node.widget
                    .paint(node.layout, &node.computed_style, commands);
            } else {
                commands.extend_from_slice(&node.paint_cache);
            }
            if scrollable {
                commands.push(PaintCommand::PushTransform {
                    translate: Translation::new(-node.scroll_offset.x, -node.scroll_offset.y),
                });
            }
            let child_scroll_offset = if scrollable {
                Point::new(
                    scroll_offset.x + node.scroll_offset.x,
                    scroll_offset.y + node.scroll_offset.y,
                )
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
    }

    fn trace_paint_frame(
        &self,
        damage: &DamageRegion,
        commands: &[PaintCommand],
        trace_nodes: &[PaintTraceNode],
    ) {
        let frame = self.paint_frames.get() + 1;
        self.paint_frames.set(frame);
        eprintln!(
            "[xui::paint] frame #{frame} damage={} bounds={:?} commands={} nodes={}",
            Self::format_damage_rects(damage),
            damage.bounds(),
            commands.len(),
            trace_nodes.len(),
        );

        for node in trace_nodes {
            eprintln!(
                "[xui::paint]   node {:?} {:?} rect={:?} key={:?} dirty={:?} subtree_dirty={:?} own_commands={} subtree_commands={}",
                node.id,
                node.node_type,
                node.rect,
                node.key,
                node.dirty,
                node.subtree_dirty,
                node.own_command_end.saturating_sub(node.command_start),
                node.subtree_command_end.saturating_sub(node.command_start),
            );
            for (index, command) in commands[node.command_start..node.own_command_end]
                .iter()
                .enumerate()
            {
                eprintln!(
                    "[xui::paint]     command #{} {:?}",
                    node.command_start + index,
                    command
                );
            }
            if let Some(command_index) = node.close_command_index {
                if let Some(command) = commands.get(command_index) {
                    eprintln!("[xui::paint]     command #{} {:?}", command_index, command);
                }
            }
        }
    }

    fn format_damage_rects(damage: &DamageRegion) -> String {
        let rects = damage
            .rects()
            .iter()
            .map(|rect| format!("{rect:?}"))
            .collect::<Vec<_>>();
        if rects.is_empty() {
            "<none>".to_owned()
        } else {
            rects.join(", ")
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.damage.is_empty()
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

#[cfg(test)]
mod scroll_behavior_tests {
    use taffy::prelude as tf;

    use super::*;
    use crate::{Style, widgets::ContainerWidget};

    #[test]
    fn scroll_acceleration_preserves_small_deltas_and_boosts_fast_deltas() {
        assert_eq!(
            ergonomic_scroll_delta(Point::new(0.0, -8.0)),
            Point::new(0.0, -8.0)
        );

        let accelerated = ergonomic_scroll_delta(Point::new(0.0, -96.0));
        assert!(accelerated.y < -96.0);
        assert!(accelerated.y > -264.0);
    }

    #[test]
    fn scroll_node_by_applies_ergonomic_delta() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let scroller = arena.insert(
            root,
            ContainerWidget::new().style(Style::new().scroll_vertical()),
            tf::Style::default(),
        );
        let node = arena.node_mut(scroller).unwrap();
        node.layout = Rect::new(0.0, 0.0, 100.0, 100.0);
        node.content_size = Size::<f32>::new(100.0, 1_000.0);

        assert!(arena.scroll_node_by(scroller, Point::new(0.0, -48.0)));
        assert_eq!(arena.node(scroller).unwrap().scroll_offset.y, 48.0);
    }

    #[test]
    fn dirty_scroll_container_damage_uses_viewport_rect() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let scroller = arena.insert(
            root,
            ContainerWidget::new().style(Style::new().scroll_vertical()),
            tf::Style::default(),
        );

        arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 200.0);
        arena.node_mut(scroller).unwrap().layout = Rect::new(10.0, 20.0, 80.0, 40.0);
        arena.node_mut(scroller).unwrap().scroll_offset = Point::new(0.0, 30.0);
        arena.finish_paint();

        arena.mark_dirty(scroller, DirtyFlags::PAINT);

        assert_eq!(
            arena.prepare_paint_commands().0.rects(),
            &[Rect::new(10.0, 20.0, 80.0, 40.0)]
        );
    }

    #[test]
    fn dirty_nested_child_damage_uses_all_ancestor_scroll_offsets() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let outer = arena.insert(
            root,
            ContainerWidget::new().style(Style::new().scroll_vertical()),
            tf::Style::default(),
        );
        let inner = arena.insert(
            outer,
            ContainerWidget::new().style(Style::new().scroll_vertical()),
            tf::Style::default(),
        );
        let child = arena.insert(inner, ContainerWidget::new(), tf::Style::default());

        arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 200.0);
        arena.node_mut(outer).unwrap().layout = Rect::new(0.0, 20.0, 100.0, 100.0);
        arena.node_mut(outer).unwrap().scroll_offset = Point::new(0.0, 30.0);
        arena.node_mut(inner).unwrap().layout = Rect::new(0.0, 70.0, 80.0, 80.0);
        arena.node_mut(inner).unwrap().scroll_offset = Point::new(0.0, 10.0);
        arena.node_mut(child).unwrap().layout = Rect::new(0.0, 90.0, 40.0, 40.0);
        arena.finish_paint();

        arena.mark_dirty(child, DirtyFlags::PAINT);

        assert_eq!(
            arena.prepare_paint_commands().0.rects(),
            &[Rect::new(0.0, 50.0, 40.0, 40.0)]
        );
    }

    #[test]
    fn dirty_child_visible_after_inner_scroll_is_not_clipped_by_root_layout_position() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let scroller = arena.insert(
            root,
            ContainerWidget::new().style(Style::new().scroll_vertical()),
            tf::Style::default(),
        );
        let child = arena.insert(scroller, ContainerWidget::new(), tf::Style::default());

        arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 100.0);
        arena.node_mut(scroller).unwrap().layout = Rect::new(0.0, 0.0, 180.0, 90.0);
        arena.node_mut(scroller).unwrap().scroll_offset = Point::new(0.0, 80.0);
        arena.node_mut(child).unwrap().layout = Rect::new(0.0, 132.0, 80.0, 40.0);
        arena.finish_paint();

        arena.mark_dirty(child, DirtyFlags::PAINT);

        assert_eq!(
            arena.prepare_paint_commands().0.rects(),
            &[Rect::new(0.0, 52.0, 80.0, 38.0)]
        );
    }

    #[test]
    fn nested_scroll_invalidates_scrollable_ancestor_gutters() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let scroller = arena.insert(
            root,
            ContainerWidget::new().style(Style::new().scroll_vertical()),
            tf::Style::default(),
        );
        let child = arena.insert(scroller, ContainerWidget::new(), tf::Style::default());

        arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 200.0, 100.0);
        arena
            .node_mut(root)
            .unwrap()
            .computed_style
            .scroll
            .direction = xui_interface::ScrollDirectionStyle::Both;
        arena.node_mut(scroller).unwrap().layout = Rect::new(0.0, 0.0, 192.0, 92.0);
        arena.node_mut(scroller).unwrap().content_size = Size::<f32>::new(192.0, 200.0);
        arena.node_mut(child).unwrap().layout = Rect::new(0.0, 0.0, 100.0, 200.0);
        arena.finish_paint();

        assert!(arena.scroll_node_by(child, Point::new(0.0, -48.0)));

        let (damage, _commands) = arena.prepare_paint_commands();
        assert!(damage.rects().contains(&Rect::new(0.0, 0.0, 200.0, 100.0)));
    }
}
