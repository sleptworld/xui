use slotmap::{SecondaryMap, SlotMap};
use taffy::prelude as tf;
use xui_interface::{DirtyFlags, NodeId, TextMeasurer};

use crate::core::{Rect, Size};
use crate::event::{Event, EventContext, EventPhase, EventResult};
use crate::render::{DamageRegion, PaintCommand};
use crate::state::HookStorage;
use crate::widgets::{Element, EventHandler, Key, NodeType, Widget, WidgetKind};

pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub taffy_node: tf::NodeId,
    pub node_type: NodeType,
    pub key: Option<Key>,
    pub position: usize,
    pub layout: Rect,
    pub previous_layout: Rect,
    pub dirty: DirtyFlags,
    pub subtree_dirty: DirtyFlags,
    pub old_props_hash: u64,
    pub new_props_hash: u64,
    pub style: tf::Style,
    pub paint_cache: Vec<PaintCommand>,
    pub kind: WidgetKind,
    pub widget: Box<dyn Widget>,
    pub on_event: Option<EventHandler>,
}

impl Node {
    fn new(
        id: NodeId,
        kind: WidgetKind,
        key: Option<Key>,
        position: usize,
        props_hash: u64,
        style: tf::Style,
        widget: Box<dyn Widget>,
        taffy_node: tf::NodeId,
    ) -> Self {
        let node_type = kind.node_type();
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
            dirty: DirtyFlags::default(),
            subtree_dirty: DirtyFlags::empty(),
            old_props_hash: 0,
            new_props_hash: props_hash,
            style,
            paint_cache: Vec::new(),
            kind,
            widget,
            on_event: None,
        }
    }
}

pub struct UiArena {
    nodes: SlotMap<NodeId, Node>,
    pub hooks: SecondaryMap<NodeId, HookStorage>,
    taffy: tf::TaffyTree,
    root: NodeId,
    damage: DamageRegion,
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    pointer_capture: Option<NodeId>,
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
        let root_style = tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Column,
            ..Default::default()
        };
        let root = nodes.insert_with_key(|id| {
            Node::new(
                id,
                WidgetKind::Root,
                None,
                0,
                0,
                root_style,
                crate::widgets::widget_from_kind(WidgetKind::Root, None),
                taffy_root,
            )
        });
        let mut hooks = SecondaryMap::new();
        hooks.insert(root, HookStorage::default());

        Self {
            nodes,
            hooks,
            taffy,
            root,
            damage: DamageRegion::new(),
            focused: None,
            hovered: None,
            pointer_capture: None,
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

    pub fn taffy(&self) -> &tf::TaffyTree {
        &self.taffy
    }

    pub fn taffy_mut(&mut self) -> &mut tf::TaffyTree {
        &mut self.taffy
    }

    pub fn insert(&mut self, parent: NodeId, kind: WidgetKind, style: tf::Style) -> NodeId {
        let widget = crate::widgets::widget_from_kind(kind.clone(), None);
        self.insert_node(parent, kind, None, 0, style, widget)
    }

    pub fn insert_node(
        &mut self,
        parent: NodeId,
        kind: WidgetKind,
        key: Option<Key>,
        props_hash: u64,
        style: tf::Style,
        widget: Box<dyn Widget>,
    ) -> NodeId {
        let taffy_node = self
            .taffy
            .new_leaf(style.clone())
            .expect("failed to create taffy node");
        let position = self.nodes[parent].children.len();
        let id = self.nodes.insert_with_key(|id| {
            Node::new(
                id, kind, key, position, props_hash, style, widget, taffy_node,
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
        self.mark_dirty(
            parent,
            DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
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
        self.mark_dirty(
            parent,
            DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
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
        self.damage.add(old_layout);
        self.hooks.remove(id);

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
            self.mark_dirty(
                parent,
                DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
            );
        }

        if self.focused == Some(id) {
            self.focused = None;
        }
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        if self.pointer_capture == Some(id) {
            self.pointer_capture = None;
        }

        self.nodes.remove(id);
    }

    pub fn mark_dirty(&mut self, id: NodeId, flags: DirtyFlags) {
        if flags.is_empty() || !self.nodes.contains_key(id) {
            return;
        }

        {
            let node = self.nodes.get_mut(id).expect("checked node existence");
            node.dirty |= flags;
            if flags.intersects(DirtyFlags::PAINT | DirtyFlags::LAYOUT | DirtyFlags::TREE) {
                self.damage.add(node.layout);
            }
        }

        // A parent with no own work can still find the dirty branch below it
        // through this aggregate subtree flag.
        let mut current = id;
        while let Some(parent) = self.nodes[current].parent {
            self.nodes[parent].subtree_dirty |= flags;
            current = parent;
        }
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

        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_from(*child, point) {
                return Some(hit);
            }
        }

        Some(id)
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
        let target = self
            .pointer_capture
            .or_else(|| {
                event
                    .pointer_position()
                    .and_then(|point| self.hit_test(point))
            })
            .or(self.focused)
            .unwrap_or(self.root);

        if matches!(event, Event::PointerMove { .. }) {
            self.hovered = Some(target);
        }

        let path = self.event_path(target);
        for id in path.iter().copied().take(path.len().saturating_sub(1)) {
            if self
                .dispatch_to_node(id, event, EventPhase::Capture)
                .is_consumed()
            {
                return EventResult::Consumed;
            }
        }

        if self
            .dispatch_to_node(target, event, EventPhase::Target)
            .is_consumed()
        {
            return EventResult::Consumed;
        }

        for id in path.into_iter().rev().skip(1) {
            if self
                .dispatch_to_node(id, event, EventPhase::Bubble)
                .is_consumed()
            {
                return EventResult::Consumed;
            }
        }

        EventResult::Ignored
    }

    fn dispatch_to_node(&mut self, id: NodeId, event: &Event, phase: EventPhase) -> EventResult {
        let mut request_dirty = DirtyFlags::empty();
        let result = {
            let node = match self.nodes.get_mut(id) {
                Some(node) => node,
                None => return EventResult::Ignored,
            };
            let mut cx = EventContext {
                node_id: id,
                phase,
                request_dirty: &mut request_dirty,
            };

            if let Some(handler) = node.on_event.as_mut() {
                let result = handler(event, &mut cx);
                if result.is_consumed() {
                    return result;
                }
            }

            if phase == EventPhase::Target {
                node.widget.handle_event(event, &mut cx)
            } else {
                EventResult::Ignored
            }
        };

        if !request_dirty.is_empty() {
            self.mark_dirty(id, request_dirty);
        }

        result
    }

    pub fn update_tree(&mut self, root: NodeId, size: Size) {
        self.update_node(root, size);
    }

    fn update_node(&mut self, id: NodeId, size: Size) {
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

        if dirty.intersects(DirtyFlags::LAYOUT | DirtyFlags::STYLE | DirtyFlags::TREE) {
            self.compute_layout_if_needed(size);
        }

        if dirty.intersects(DirtyFlags::PAINT | DirtyFlags::LAYOUT | DirtyFlags::STYLE) {
            self.repaint_if_needed(id);
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.update_node(child, size);
        }

        self.clear_dirty(id);
    }

    pub fn compute_layout_if_needed(&mut self, size: Size) {
        if !self.nodes.values().any(|node| {
            node.dirty
                .intersects(DirtyFlags::LAYOUT | DirtyFlags::STYLE | DirtyFlags::TREE)
        }) {
            return;
        }
        self.compute_layout(size);
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
        let mut cache = Vec::new();
        self.nodes[id].widget.paint(rect, &mut cache);
        self.nodes[id].paint_cache = cache;
        self.damage.add(rect);
    }

    pub fn compute_layout(&mut self, size: Size) {
        self.layout_passes += 1;
        let root_taffy = self.nodes[self.root].taffy_node;
        self.taffy
            .compute_layout(
                root_taffy,
                tf::Size {
                    width: tf::AvailableSpace::Definite(size.width),
                    height: tf::AvailableSpace::Definite(size.height),
                },
            )
            .expect("failed to compute layout");
        self.sync_layout(self.root, 0.0, 0.0);
    }

    fn sync_layout(&mut self, id: NodeId, offset_x: f32, offset_y: f32) {
        let taffy_node = self.nodes[id].taffy_node;
        let layout = *self
            .taffy
            .layout(taffy_node)
            .expect("missing taffy layout result");
        let rect = Rect::new(
            offset_x + layout.location.x,
            offset_y + layout.location.y,
            layout.size.width,
            layout.size.height,
        );

        let children = {
            let node = self.nodes.get_mut(id).expect("node removed during layout");
            let layout_changed = node.layout != rect;
            if layout_changed {
                self.damage.add(node.layout);
                self.damage.add(rect);
            }
            node.previous_layout = node.layout;
            node.layout = rect;
            node.dirty.remove(DirtyFlags::LAYOUT);
            if layout_changed {
                node.dirty.insert(DirtyFlags::PAINT);
            }
            node.children.clone()
        };

        for child in children {
            self.sync_layout(child, rect.x, rect.y);
        }
    }

    pub fn collect_paint_commands(&mut self) -> (DamageRegion, Vec<PaintCommand>) {
        let damage = core::mem::take(&mut self.damage);
        let mut commands = Vec::new();
        if damage.is_empty() {
            return (damage, commands);
        }
        self.paint_node(self.root, &damage, &mut commands);
        for (_, node) in self.nodes.iter_mut() {
            node.dirty.remove(DirtyFlags::PAINT);
        }
        (damage, commands)
    }

    fn paint_node(&self, id: NodeId, damage: &DamageRegion, commands: &mut Vec<PaintCommand>) {
        let node = match self.nodes.get(id) {
            Some(node) => node,
            None => return,
        };

        if damage.intersects(node.layout) {
            if node.paint_cache.is_empty() {
                node.widget.paint(node.layout, commands);
            } else {
                commands.extend_from_slice(&node.paint_cache);
            }
            for child in &node.children {
                self.paint_node(*child, damage, commands);
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.damage.is_empty()
            || self
                .nodes
                .values()
                .any(|node| !node.dirty.is_empty() || !node.subtree_dirty.is_empty())
    }

    pub fn diff_children(
        &mut self,
        parent: NodeId,
        new_children: Vec<Element>,
        measurer: &dyn TextMeasurer,
    ) {
        let old_children = self.nodes[parent].children.clone();
        let mut used = vec![false; old_children.len()];
        let mut next_children = Vec::with_capacity(new_children.len());
        let mut tree_changed = old_children.len() != new_children.len();

        // Keyed children are matched before positional children, so inserting a
        // sibling does not reset state for an existing keyed node.
        for (position, new_child) in new_children.into_iter().enumerate() {
            let matched = self.find_reusable_child(&old_children, &used, &new_child, position);
            let id = if let Some(old_index) = matched {
                used[old_index] = true;
                let id = old_children[old_index];
                if old_index != position {
                    tree_changed = true;
                }
                self.update_node_from_element(id, new_child, position, measurer);
                id
            } else {
                tree_changed = true;
                self.create_from_element(parent, new_child, position, measurer)
            };
            next_children.push(id);
        }

        for (index, old_child) in old_children.iter().copied().enumerate() {
            if !used[index] {
                tree_changed = true;
                self.remove_subtree_detached(old_child);
            }
        }

        self.nodes[parent].children = next_children;
        for child in self.nodes[parent].children.clone() {
            self.nodes[child].parent = Some(parent);
        }
        self.reindex_children(parent);
        self.sync_taffy_children(parent);

        if tree_changed {
            self.mark_dirty(
                parent,
                DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
            );
        }
    }

    fn find_reusable_child(
        &self,
        old_children: &[NodeId],
        used: &[bool],
        new_child: &Element,
        position: usize,
    ) -> Option<usize> {
        if let Some(key) = new_child.key() {
            return old_children
                .iter()
                .copied()
                .enumerate()
                .find(|(index, old_id)| {
                    !used[*index]
                        && self.nodes[*old_id].key.as_ref() == Some(&key)
                        && self.nodes[*old_id].node_type == new_child.node_type()
                })
                .map(|(index, _)| index);
        }

        old_children
            .get(position)
            .copied()
            .filter(|old_id| {
                !used[position] && should_reuse(&self.nodes[*old_id], new_child, position)
            })
            .map(|_| position)
    }

    pub fn create_from_element(
        &mut self,
        parent: NodeId,
        element: Element,
        position: usize,
        measurer: &dyn TextMeasurer,
    ) -> NodeId {
        let key = element.key();
        let props_hash = element.props_hash();
        let style = element.style(measurer);
        let (kind, widget, children) = element.into_parts();
        let id = self.insert_node(parent, kind, key, props_hash, style, widget);
        self.nodes[id].position = position;
        self.hooks.entry(id).unwrap().or_default();
        self.diff_children(id, children, measurer);
        id
    }

    fn update_node_from_element(
        &mut self,
        id: NodeId,
        element: Element,
        position: usize,
        measurer: &dyn TextMeasurer,
    ) {
        let new_props_hash = element.props_hash();
        let new_style = element.style(measurer);
        let (new_kind, new_widget, children) = element.into_parts();
        let mut flags = DirtyFlags::empty();

        {
            let node = self.nodes.get_mut(id).expect("reused node missing");
            node.position = position;
            node.new_props_hash = new_props_hash;
            if node.old_props_hash != new_props_hash {
                flags |= DirtyFlags::PROPS;
            }
            if node.style != new_style {
                node.style = new_style.clone();
                flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
                self.taffy
                    .set_style(node.taffy_node, new_style)
                    .expect("failed to update taffy style");
            }
            let widget_flags = node.widget.update_from_kind(&new_kind);
            flags |= widget_flags;
            flags |= crate::widgets::update_kind_from(&mut node.kind, new_kind.clone());
            if widget_flags.contains(DirtyFlags::TREE) {
                node.widget = new_widget;
            }
        }

        self.mark_dirty(id, flags);
        self.diff_children(id, children, measurer);
    }

    fn remove_subtree_detached(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) || id == self.root {
            return;
        }
        let children = self.nodes[id].children.clone();
        for child in children {
            self.remove_subtree_detached(child);
        }
        self.damage.add(self.nodes[id].layout);
        self.hooks.remove(id);
        if self.focused == Some(id) {
            self.focused = None;
        }
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        if self.pointer_capture == Some(id) {
            self.pointer_capture = None;
        }
        let _ = self.taffy.remove(self.nodes[id].taffy_node);
        self.nodes.remove(id);
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

    fn reindex_children(&mut self, parent: NodeId) {
        let children = self.nodes[parent].children.clone();
        for (position, child) in children.into_iter().enumerate() {
            self.nodes[child].position = position;
        }
    }
}

impl Default for UiArena {
    fn default() -> Self {
        Self::new()
    }
}

pub fn mark_dirty(tree: &mut UiArena, node: NodeId, flags: DirtyFlags) {
    tree.mark_dirty(node, flags);
}

pub fn clear_dirty(tree: &mut UiArena, node: NodeId) {
    tree.clear_dirty(node);
}

pub fn update_tree(tree: &mut UiArena, root: NodeId) {
    tree.update_tree(root, Size::ZERO);
}

pub fn diff_children(
    tree: &mut UiArena,
    parent: NodeId,
    new_children: Vec<Element>,
    measurer: &dyn TextMeasurer,
) {
    tree.diff_children(parent, new_children, measurer);
}

pub fn should_reuse(old: &Node, new: &Element, position: usize) -> bool {
    old.node_type == new.node_type()
        && old.key == new.key()
        && (old.key.is_some() || old.position == position)
}

pub fn compute_layout_if_needed(tree: &mut UiArena, _node: NodeId) {
    tree.compute_layout_if_needed(Size::ZERO);
}

pub fn repaint_if_needed(tree: &mut UiArena, node: NodeId) {
    tree.repaint_if_needed(node);
}
