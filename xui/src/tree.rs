use slotmap::SlotMap;
use std::cell::Cell;
use taffy::prelude as tf;
use xui_interface::core::Sizing;
use xui_interface::{
    ComputedColorStyle, ComputedScrollbarStyle, ComputedStyle, ComputedTextStyle, DirtyFlags,
    EventHandlers, NodeId, ScrollbarVisibilityStyle, TextContent, TextLayoutConstraints,
    TextMeasurer, Theme, Translation,
};

use crate::core::{Point, Rect, Size};
use crate::event::{Event, EventResult};
use crate::event_system::{self, EventState};
use crate::fiber::Key;
use crate::layout::{computed_style_for_widget, taffy_style_for_widget};
use crate::render::{DamageRegion, PaintCommand};
use crate::widgets::{WidgetI, WidgetType, Widgets};

pub enum WidgetContext {
    Text(ComputedTextStyle, Option<TextContent>),
    Button(ComputedTextStyle, Option<TextContent>),
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
    pub event_handlers: EventHandlers,
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
        event_handlers: EventHandlers,
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
    event_state: EventState,
    theme: Theme,
    paint_frames: Cell<usize>,
    pub update_visits: usize,
    pub layout_passes: usize,
    pub repaint_passes: usize,
}

impl UiArena {
    pub fn new() -> Self {
        let mut taffy = tf::TaffyTree::new();
        let taffy_root = taffy
            .new_leaf(tf::Style {
                display: tf::Display::Flex,
                flex_direction: tf::FlexDirection::Column,
                ..Default::default()
            })
            .expect("failed to create taffy root");
        let mut nodes = SlotMap::with_key();
        let theme = Theme::default();
        let root_computed_style = ComputedStyle::initial(&theme);
        let root = nodes.insert_with_key(|id| {
            Node::new(
                id,
                None,
                0,
                0,
                // root_style,
                root_computed_style,
                crate::widgets::root_widget(),
                EventHandlers::default(),
                taffy_root,
            )
        });
        Self {
            nodes,
            taffy,
            root,
            damage: DamageRegion::new(),
            damage_nodes: vec![],
            event_state: EventState::default(),
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

    pub fn create_node(
        &mut self,
        key: Option<Key>,
        props_hash: u64,
        widget: WidgetI,
        event_handlers: EventHandlers,
        style: tf::Style,
        computed_style: ComputedStyle,
    ) -> NodeId {
        let node_type = widget.node_type();
        let taffy_node = match node_type {
            WidgetType::Text | WidgetType::Label => self.taffy.new_leaf_with_context(
                style,
                WidgetContext::Text(computed_style.text.clone(), widget.text()),
            ),
            WidgetType::Button => self.taffy.new_leaf_with_context(
                style,
                WidgetContext::Button(computed_style.text.clone(), widget.text()),
            ),
            _ => self.taffy.new_leaf(style),
        }
        .expect("failed to create taffy node");
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
        let node_type = widget.node_type();
        let taffy_node = match node_type {
            WidgetType::Text | WidgetType::Label => self.taffy.new_leaf_with_context(
                style,
                WidgetContext::Text(computed_style.text.clone(), widget.text()),
            ),
            WidgetType::Button => self.taffy.new_leaf_with_context(
                style,
                WidgetContext::Button(computed_style.text.clone(), widget.text()),
            ),
            _ => self.taffy.new_leaf(style),
        }
        .expect("failed to create taffy node");
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
        self.attach(parent, id);
        id
    }

    pub fn attach(&mut self, parent: NodeId, child: NodeId) {
        let parent_taffy = self.nodes[parent].taffy_node;
        let child_taffy = self.nodes[child].taffy_node;
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
        let taffy_children: Vec<_> = self.nodes[parent]
            .children
            .iter()
            .map(|id| self.nodes[*id].taffy_node)
            .collect();
        self.taffy
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to attach taffy child");
        self.reindex_children(parent);
        self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
        self.damage.add(self.nodes[child].layout);
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
        self.damage.add(old_layout);

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

        let _ = self.taffy.remove(self.nodes[id].taffy_node);
        self.nodes.remove(id);
    }

    pub fn mark_dirty(&mut self, id: NodeId, flags: DirtyFlags) {
        if flags.is_empty() || !self.nodes.contains_key(id) {
            return;
        }

        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.dirty |= flags;
        if flags.intersects(DirtyFlags::PAINT | DirtyFlags::STYLE) {
            self.damage.add(node.layout);
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

    pub fn hit_test(&self, point: crate::core::Point) -> Option<NodeId> {
        self.hit_test_from(self.root, point)
    }

    fn hit_test_from(&self, id: NodeId, point: crate::core::Point) -> Option<NodeId> {
        let node = self.nodes.get(id)?;
        if !node.layout.contains(point) {
            return None;
        }

        let child_point = if node.computed_style.scroll.direction.is_scrollable() {
            Point::new(
                point.x + node.scroll_offset.x,
                point.y + node.scroll_offset.y,
            )
        } else {
            point
        };

        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_from(*child, child_point) {
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

        let next = Point::new(
            (node.scroll_offset.x - delta.x).clamp(0.0, max_x),
            (node.scroll_offset.y - delta.y).clamp(0.0, max_y),
        );
        if next == node.scroll_offset {
            return false;
        }

        let old_layout = node.layout;
        let node = self.nodes.get_mut(id).expect("checked node existence");
        node.scroll_offset = next;
        self.damage.add(old_layout);
        self.mark_dirty(id, DirtyFlags::PAINT);
        true
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
                let style = taffy_style_for_widget(&parent_style, &computed_style);
                (computed_style, style)
            } else {
                let parent_style = ComputedStyle::initial(&self.theme);
                let computed_style = computed_style_for_widget(&widget, &parent_style, &self.theme);
                let style = taffy_style_for_widget(&parent_style, &computed_style);
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
        self.nodes[id].paint_cache = cache;
        if !self.damage_nodes.contains(&id) {
            self.damage_nodes.push(id);
        }
        self.damage.add(rect);
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
        let layout = *self
            .taffy
            .layout(taffy_node)
            .expect("missing taffy layout result");
        let content_size = Size::<f32>::new(layout.content_size.width, layout.content_size.height);
        let rect = Rect::new(
            offset_x + layout.location.x,
            offset_y + layout.location.y,
            layout.size.width,
            layout.size.height,
        );

        let (children, mut subtree_dirty) = {
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            let layout_changed = node.layout != rect;
            let should_sync_children = layout_changed
                || node.dirty.intersects(Self::layout_dirty_flags())
                || node.subtree_dirty.intersects(Self::layout_dirty_flags());

            if layout_changed {
                self.damage.add(node.layout);
                self.damage.add(rect);
            }
            node.previous_layout = node.layout;
            node.layout = rect;
            node.content_size = content_size;
            clamp_scroll_offset(node);
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

        let node = self.nodes.get_mut(id).expect("node removed during layout");
        node.subtree_dirty = subtree_dirty;
        node.dirty | node.subtree_dirty
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

    fn paint_node(&self, id: NodeId, damage: &DamageRegion, commands: &mut Vec<PaintCommand>) {
        self.paint_node_inner(id, damage, commands, false);
    }

    fn paint_node_inner(
        &self,
        id: NodeId,
        damage: &DamageRegion,
        commands: &mut Vec<PaintCommand>,
        force: bool,
    ) {
        let node = match self.nodes.get(id) {
            Some(node) => node,
            None => return,
        };

        if force || damage.intersects(node.layout) {
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
            for child in &node.children {
                self.paint_node_inner(*child, damage, commands, force || scrollable);
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

        {
            let text = widget.text();
            eprintln!("Updated str: {:?}", text);
        }

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

        let tree_changed = self.nodes[parent].children != children;
        self.nodes[parent].children = children;
        for child in self.nodes[parent].children.clone() {
            if self.nodes.contains_key(child) {
                self.nodes[child].parent = Some(parent);
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

    fn refresh_taffy_context(&mut self, id: NodeId) {
        let node = &self.nodes[id];

        let context = match node.node_type {
            WidgetType::Text | WidgetType::Label => Some(WidgetContext::Text(
                node.computed_style.text.clone(),
                node.widget.text(),
            )),
            WidgetType::Button => Some(WidgetContext::Button(
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
    available_space: tf::Size<tf::AvailableSpace>,
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
        Some(WidgetContext::Text(_props, t)) | Some(WidgetContext::Button(_props, t)) => {
            let str = t.as_ref().map(|t| t.as_str()).unwrap_or_default();
            let constraints = match available_space.width {
                tf::AvailableSpace::Definite(width) => TextLayoutConstraints::max_width(width),
                tf::AvailableSpace::MaxContent => TextLayoutConstraints::UNBOUNDED,
                _ => TextLayoutConstraints::UNBOUNDED,
            };
            measurer.measure_text_with_constraints(str, _props, constraints, None)
        }

        _ => Size::<f32>::ZERO,
    };

    tf::Size {
        width: known_dimensions.width.unwrap_or(measured.width),
        height: known_dimensions.height.unwrap_or(measured.height),
    }
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
// mod tests {
//     use super::*;
//     use crate::widgets::{TextWidget, button, container, label, row, style_scope};
//     use crate::{Color, PaintCommand, Style};

//     #[derive(Default)]
//     struct RecordingMeasurer {
//         constraints: Vec<TextLayoutConstraints>,
//     }

//     impl TextMeasurer for RecordingMeasurer {
//         fn measure_text(&mut self, text: &str, style: &ComputedTextStyle) -> Size {
//             self.measure_text_with_constraints(text, style, TextLayoutConstraints::UNBOUNDED)
//         }

//         fn measure_text_with_constraints(
//             &mut self,
//             _text: &str,
//             _style: &ComputedTextStyle,
//             constraints: TextLayoutConstraints,
//         ) -> Size {
//             self.constraints.push(constraints);
//             match constraints.max_width {
//                 Some(width) => Size::new(width, 20.0),
//                 None => Size::new(100.0, 10.0),
//             }
//         }
//     }

//     #[derive(Default)]
//     struct TextRecordingMeasurer {
//         calls: Vec<(String, f32)>,
//     }

//     impl TextMeasurer for TextRecordingMeasurer {
//         fn measure_text(&mut self, text: &str, style: &ComputedTextStyle) -> Size {
//             self.measure_text_with_constraints(text, style, TextLayoutConstraints::UNBOUNDED)
//         }

//         fn measure_text_with_constraints(
//             &mut self,
//             text: &str,
//             style: &ComputedTextStyle,
//             _constraints: TextLayoutConstraints,
//         ) -> Size {
//             self.calls.push((text.to_owned(), style.font_size));
//             Size::new(text.len() as f32 * style.font_size, style.font_size)
//         }
//     }

//     #[test]
//     fn text_layout_uses_available_width_as_measure_constraint() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let text = arena.insert(
//             root,
//             TextWidget::new("width constrained"),
//             tf::Style::default(),
//         );
//         let mut measurer = RecordingMeasurer::default();

//         arena.update_tree(root, Size::new(50.0, 100.0), &mut measurer);

//         assert!(
//             measurer
//                 .constraints
//                 .iter()
//                 .any(|constraints| constraints.max_width == Some(50.0))
//         );
//         assert_eq!(arena.node(text).unwrap().layout.width, 50.0);
//         assert_eq!(arena.node(text).unwrap().layout.height, 20.0);
//     }

//     #[test]
//     fn style_scope_inherits_text_style_to_label_and_local_style_overrides() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let element = style_scope(Style::new().color(Color::BLUE_500).font_size(20.0))
//             .child(label("inherited"))
//             .child(label("local").style(Style::new().color(Color::BLACK)));
//         let mut measurer = TextI::new();

//         let scope = arena.create_from_element(root, element.into(), 0, &mut measurer);
//         let inherited = arena.children(scope)[0];
//         let local = arena.children(scope)[1];

//         assert_eq!(
//             arena.node(inherited).unwrap().computed_style.text.color,
//             Color::BLUE_500
//         );
//         assert_eq!(
//             arena.node(inherited).unwrap().computed_style.text.font_size,
//             20.0
//         );
//         assert_eq!(
//             arena.node(local).unwrap().computed_style.text.color,
//             Color::BLACK
//         );
//         assert_eq!(
//             arena.node(local).unwrap().computed_style.text.font_size,
//             20.0
//         );
//     }

//     #[test]
//     fn container_style_emits_background_and_border_commands() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let element = container().style(
//             Style::new()
//                 .size(Size::new(40.0, 20.0))
//                 .background(Color::BLUE_500)
//                 .border_color(Color::BLACK)
//                 .border_width(2.0)
//                 .border_radius(4.0),
//         );
//         let mut measurer = TextI::new();
//         let node = arena.create_from_element(root, element.into(), 0, &mut measurer);
//         arena.node_mut(node).unwrap().layout = Rect::new(1.0, 2.0, 40.0, 20.0);
//         arena.mark_dirty(node, DirtyFlags::PAINT);
//         arena.repaint_if_needed(node);
//         let commands = arena.node(node).unwrap().paint_cache.clone();

//         assert!(commands.iter().any(|command| matches!(
//             command,
//             PaintCommand::FillRoundedRect { radius, color, .. }
//                 if *radius == 4.0 && *color == Color::BLUE_500
//         )));
//         assert!(commands.iter().any(|command| matches!(
//             command,
//             PaintCommand::StrokeRoundedRect { radius, color, width, .. }
//                 if *radius == 4.0 && *color == Color::BLACK && *width == 2.0
//         )));
//     }

//     #[test]
//     fn row_gap_from_style_affects_child_layout() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let element = row()
//             .style(Style::new().gap(12.0))
//             .child(container().style(Style::new().size(Size::new(10.0, 10.0))))
//             .child(container().style(Style::new().size(Size::new(10.0, 10.0))));
//         let mut measurer = TextI::new();

//         let row = arena.create_from_element(root, element.into(), 0, &mut measurer);
//         arena.update_tree(root, Size::new(100.0, 30.0), &mut measurer);
//         let first = arena.children(row)[0];
//         let second = arena.children(row)[1];

//         assert_eq!(arena.node(first).unwrap().layout.x, 0.0);
//         assert_eq!(arena.node(second).unwrap().layout.x, 22.0);
//     }

//     #[test]
//     fn paint_collection_matches_damage_bounds_scissor() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let mut measurer = TextI::new();
//         let top_color = Color::rgb(0.9, 0.1, 0.1);
//         let middle_color = Color::rgb(0.1, 0.9, 0.1);
//         let bottom_color = Color::rgb(0.1, 0.1, 0.9);
//         let top = arena.create_from_element(
//             root,
//             container()
//                 .style(
//                     Style::new()
//                         .size(Size::new(20.0, 10.0))
//                         .background(top_color),
//                 )
//                 .into(),
//             0,
//             &mut measurer,
//         );
//         let middle = arena.create_from_element(
//             root,
//             container()
//                 .style(
//                     Style::new()
//                         .size(Size::new(20.0, 10.0))
//                         .background(middle_color),
//                 )
//                 .into(),
//             1,
//             &mut measurer,
//         );
//         let bottom = arena.create_from_element(
//             root,
//             container()
//                 .style(
//                     Style::new()
//                         .size(Size::new(20.0, 10.0))
//                         .background(bottom_color),
//                 )
//                 .into(),
//             2,
//             &mut measurer,
//         );

//         arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);
//         arena.finish_paint();
//         assert_eq!(arena.node(middle).unwrap().layout.y, 10.0);

//         arena.mark_dirty(top, DirtyFlags::PAINT);
//         arena.mark_dirty(bottom, DirtyFlags::PAINT);
//         arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);
//         let (_, commands) = arena.prepare_paint_commands();

//         assert!(commands.iter().any(|command| matches!(
//             command,
//             PaintCommand::FillRect { rect, color }
//                 if *color == middle_color && *rect == arena.node(middle).unwrap().layout
//         )));
//     }

//     #[test]
//     fn multiple_layout_dirty_descendants_batch_into_one_layout_pass() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let element = row()
//             .child(container().style(Style::new().size(Size::new(10.0, 10.0))))
//             .child(container().style(Style::new().size(Size::new(10.0, 10.0))));
//         let mut measurer = TextI::new();

//         let row = arena.create_from_element(root, element.into(), 0, &mut measurer);
//         arena.update_tree(root, Size::new(100.0, 30.0), &mut measurer);
//         let first = arena.children(row)[0];
//         let second = arena.children(row)[1];
//         let passes_before = arena.layout_passes;

//         arena.mark_dirty(first, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
//         arena.mark_dirty(second, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
//         arena.update_tree(root, Size::new(100.0, 30.0), &mut measurer);

//         assert_eq!(arena.layout_passes, passes_before + 1);
//     }

//     #[test]
//     fn sync_layout_propagates_parent_offset_changes_to_clean_children() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let mut measurer = TextI::new();
//         let spacer = arena.create_from_element(
//             root,
//             container()
//                 .style(Style::new().size(Size::new(10.0, 0.0)))
//                 .into(),
//             0,
//             &mut measurer,
//         );
//         let parent = arena.create_from_element(
//             root,
//             container()
//                 .style(Style::new().size(Size::new(20.0, 20.0)))
//                 .child(container().style(Style::new().size(Size::new(5.0, 5.0))))
//                 .into(),
//             1,
//             &mut measurer,
//         );
//         arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);
//         let child = arena.children(parent)[0];
//         assert_eq!(arena.node(child).unwrap().layout.y, 0.0);

//         arena.update_widget_from_element(
//             spacer,
//             container()
//                 .style(Style::new().size(Size::new(10.0, 12.0)))
//                 .into(),
//             0,
//             &mut measurer,
//         );
//         arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

//         assert_eq!(arena.node(parent).unwrap().layout.y, 12.0);
//         assert_eq!(arena.node(child).unwrap().layout.y, 12.0);
//     }

//     #[test]
//     fn text_update_refreshes_measure_context_and_triggers_layout() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let mut text_i = TextI::new();
//         let mut measurer = TextRecordingMeasurer::default();
//         let text = arena.create_from_element(
//             root,
//             TextWidget::new("a")
//                 .style(Style::new().font_size(10.0))
//                 .into(),
//             0,
//             &mut text_i,
//         );
//         arena.update_tree(root, Size::new(500.0, 100.0), &mut measurer);
//         let initial_width = arena.node(text).unwrap().layout.width;
//         let passes_before = arena.layout_passes;

//         arena.update_widget_from_element(
//             text,
//             TextWidget::new("abcd")
//                 .style(Style::new().font_size(12.0))
//                 .into(),
//             0,
//             &mut text_i,
//         );
//         arena.update_tree(root, Size::new(500.0, 100.0), &mut measurer);

//         assert_eq!(arena.layout_passes, passes_before + 1);
//         assert_eq!(arena.node(text).unwrap().layout.width, 48.0);
//         assert!(arena.node(text).unwrap().layout.width > initial_width);
//         assert!(
//             measurer
//                 .calls
//                 .iter()
//                 .any(|(text, font_size)| text == "abcd" && *font_size == 12.0)
//         );
//     }

//     #[test]
//     fn button_pressed_state_style_repaints_with_primary_background() {
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         let mut measurer = TextI::new();
//         let node = arena.create_from_element(root, button("press").into(), 0, &mut measurer);
//         arena.node_mut(node).unwrap().layout = Rect::new(0.0, 0.0, 80.0, 30.0);
//         let mut dirty = DirtyFlags::empty();
//         let mut requests = crate::EventRequests::default();
//         arena.node_mut(node).unwrap().widget.with_mut(|widget| {
//             let mut cx = crate::EventContext::new(
//                 node,
//                 crate::EventPhase::Target,
//                 &mut dirty,
//                 &mut requests,
//             );
//             widget.handle_event(
//                 &Event::PointerDown {
//                     position: crate::Point::new(1.0, 1.0),
//                     button: crate::PointerButton::Primary,
//                 },
//                 &mut cx,
//             );
//         });
//         arena.mark_dirty(node, dirty);
//         arena.update_tree(root, Size::new(100.0, 50.0), &mut measurer);
//         let (_, commands) = arena.prepare_paint_commands();

//         assert!(commands.iter().any(|command| matches!(
//             command,
//             PaintCommand::FillRect { color, .. } if *color == Color::BLUE_500
//         )));
//     }
// }

// pub fn repaint_if_needed(tree: &mut UiArena, node: NodeId) {
//     tree.repaint_if_needed(node);
// }
