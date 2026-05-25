use slotmap::SlotMap;
use taffy::prelude as tf;
use xui_interface::{
    ComputedStyle, ComputedTextStyle, DirtyFlags, EventHandlers, NodeId, TextContent,
    TextLayoutConstraints, TextMeasurer, Theme,
};

use crate::LayoutStyledWidget;
use crate::core::{Rect, Size};
use crate::event::{Event, EventResult};
use crate::event_system::{self, EventState};
use crate::fiber::Key;
use crate::font::TextI;
use crate::render::{DamageRegion, PaintCommand};
use crate::widgets::{
    Element, WidgetRef, WidgetType, computed_style_for_widget, taffy_style_for_widget,
};

pub enum WidgetContext {
    Text(ComputedTextStyle, Option<TextContent>),
}

pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub taffy_node: tf::NodeId,
    pub node_type: WidgetType,
    pub key: Option<Key>,
    pub position: usize,
    pub layout: Rect,
    pub previous_layout: Rect,
    pub dirty: DirtyFlags,
    pub subtree_dirty: DirtyFlags,
    pub old_props_hash: u64,
    pub new_props_hash: u64,
    // pub style: tf::Style,
    pub computed_style: ComputedStyle,
    pub paint_cache: Vec<PaintCommand>,
    pub widget: WidgetRef,
    pub event_handlers: EventHandlers,
}

impl Node {
    fn new(
        id: NodeId,
        key: Option<Key>,
        position: usize,
        props_hash: u64,
        // style: tf::Style,
        computed_style: ComputedStyle,
        widget: WidgetRef,
        event_handlers: EventHandlers,
        taffy_node: tf::NodeId,
    ) -> Self {
        let node_type = widget.with(|widget| widget.node_type());

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
    event_state: EventState,
    theme: Theme,
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
                crate::widgets::root_widget().into(),
                EventHandlers::default(),
                taffy_root,
            )
        });
        Self {
            nodes,
            taffy,
            root,
            damage: DamageRegion::new(),
            event_state: EventState::default(),
            theme,
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

    pub fn insert(
        &mut self,
        parent: NodeId,
        widget: impl LayoutStyledWidget + 'static,
        style: tf::Style,
    ) -> NodeId {
        let parent_style = &self.nodes[parent].computed_style;
        let widget_ref = WidgetRef::new(widget);
        let computed_style =
            widget_ref.with(|widget| computed_style_for_widget(widget, parent_style, &self.theme));
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
        widget: WidgetRef,
        event_handlers: EventHandlers,
    ) -> NodeId {
        let position = self.nodes[parent].children.len();
        let node_type = widget.with(|w| w.node_type());
        let taffy_node = match node_type {
            WidgetType::Text => self.taffy.new_leaf_with_context(
                style,
                WidgetContext::Text(computed_style.text.clone(), widget.with(|w| w.text())),
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
        event_system::dispatch_event(self, event)
    }

    pub fn update_tree<T: TextMeasurer>(&mut self, root: NodeId, size: Size, measurer: &mut T) {
        self.update_node(root, size, measurer);
    }

    fn update_node<T: TextMeasurer>(&mut self, id: NodeId, size: Size, measurer: &mut T) {
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

        if self.nodes[id]
            .dirty
            .intersects(DirtyFlags::LAYOUT | DirtyFlags::STYLE | DirtyFlags::TREE)
        {
            self.compute_layout_if_needed(size, measurer);
        }

        let dirty = self.nodes[id].dirty;
        if dirty.intersects(DirtyFlags::PAINT | DirtyFlags::STYLE) {
            self.repaint_if_needed(id);
        }

        let children = self.nodes[id].children.clone();
        for child in children {
            self.update_node(child, size, measurer);
        }

        self.clear_dirty(id);
    }

    fn recompute_subtree_styles<T: TextMeasurer>(&mut self, id: NodeId, measurer: &mut T) {
        if !self.nodes.contains_key(id) {
            return;
        }

        let widget = self.nodes[id].widget.clone();
        let computed_style = if let Some(p) = self.nodes[id].parent.and_then(|p| self.node(p)) {
            let parent_style = &p.computed_style;
            widget.with(|widget| computed_style_for_widget(widget, parent_style, &self.theme))
        } else {
            widget.with(|widget| {
                computed_style_for_widget(widget, &ComputedStyle::initial(&self.theme), &self.theme)
            })
        };

        let taffy_style =
            widget.layout_with(|widget| taffy_style_for_widget(widget, &computed_style, measurer));
        let mut changed = false;

        {
            let taffy_node_id = self
                .nodes
                .get(id)
                .map(|n| n.taffy_node)
                .expect("checked node existence");

            if let Some(n) = self.node_mut(id) {
                if n.computed_style != computed_style {
                    n.computed_style = computed_style;
                    n.dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
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

    pub fn compute_layout_if_needed<T: TextMeasurer>(&mut self, size: Size, measurer: &mut T) {
        if !self.nodes.values().any(|node| {
            node.dirty
                .intersects(DirtyFlags::LAYOUT | DirtyFlags::STYLE | DirtyFlags::TREE)
        }) {
            return;
        }
        self.compute_layout(size, measurer);
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
        self.nodes[id]
            .widget
            .with(|widget| widget.paint(rect, &style, &mut cache));
        self.nodes[id].paint_cache = cache;
        self.damage.add(rect);
    }

    pub fn compute_layout<T: TextMeasurer>(&mut self, size: Size, measurer: &mut T) {
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
        self.paint_node(self.root, &damage, &mut commands);
        (damage, commands)
    }

    pub fn finish_paint(&mut self) {
        self.damage = DamageRegion::new();
        for (_, node) in self.nodes.iter_mut() {
            node.dirty.remove(DirtyFlags::PAINT);
        }
    }

    fn paint_node(&self, id: NodeId, damage: &DamageRegion, commands: &mut Vec<PaintCommand>) {
        let node = match self.nodes.get(id) {
            Some(node) => node,
            None => return,
        };

        if damage.intersects(node.layout) {
            if node.computed_style.paint.clip {
                commands.push(PaintCommand::PushClip(node.layout));
            }
            if node.paint_cache.is_empty() {
                node.widget
                    .with(|widget| widget.paint(node.layout, &node.computed_style, commands));
            } else {
                commands.extend_from_slice(&node.paint_cache);
            }
            for child in &node.children {
                self.paint_node(*child, damage, commands);
            }
            if node.computed_style.paint.clip {
                commands.push(PaintCommand::PopClip);
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
        measurer: &mut TextI,
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
            self.mark_dirty(parent, DirtyFlags::TREE | DirtyFlags::LAYOUT);
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
                        && Some(self.nodes[*old_id].node_type) == new_child.node_type()
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
        measurer: &mut TextI,
    ) -> NodeId {
        let key = element.key();
        let props_hash = element.props_hash();
        let parts = element.into_parts();
        let parent_style = self.nodes[parent].computed_style.clone();
        let computed_style = parts
            .widget
            .with(|widget| computed_style_for_widget(widget, &parent_style, &self.theme));
        let style = parts
            .widget
            .layout_with(|widget| taffy_style_for_widget(widget, &computed_style, measurer));
        let id = self.insert_node(
            parent,
            key,
            props_hash,
            style,
            computed_style,
            parts.widget,
            parts.event_handlers,
        );
        self.nodes[id].position = position;
        self.diff_children(id, parts.children, measurer);
        id
    }

    pub fn create_widget_from_element(
        &mut self,
        parent: NodeId,
        element: Element,
        position: usize,
        measurer: &mut TextI,
    ) -> (NodeId, Vec<Element>) {
        let key = element.key();
        let props_hash = element.props_hash();
        let parts = element.into_parts();
        let parent_style = self.nodes[parent].computed_style.clone();
        let computed_style = parts
            .widget
            .with(|widget| computed_style_for_widget(widget, &parent_style, &self.theme));
        let style = parts
            .widget
            .layout_with(|widget| taffy_style_for_widget(widget, &computed_style, measurer));
        let id = self.insert_node(
            parent,
            key,
            props_hash,
            style,
            computed_style,
            parts.widget,
            parts.event_handlers,
        );
        self.nodes[id].position = position;
        (id, parts.children)
    }

    pub fn update_widget_from_element(
        &mut self,
        id: NodeId,
        element: Element,
        position: usize,
        measurer: &mut TextI,
    ) -> Vec<Element> {
        self.update_widget_node_from_element(id, element, position, measurer)
    }

    pub fn update_widget_node_from_parts(
        &mut self,
        id: NodeId,
        key: Option<Key>,
        props_hash: u64,
        style: tf::Style,
        computed_style: ComputedStyle,
        widget: WidgetRef,
        event_handlers: EventHandlers,
    ) -> WidgetRef {
        let mut flags = DirtyFlags::empty();
        let current_widget;

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
            if node.node_type != widget.with(|widget| widget.node_type()) {
                flags |= DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
            }
            let widget_flags = node
                .widget
                .with_mut(|current| widget.with(|next| current.update_from(next)));
            flags |= widget_flags;
            node.event_handlers = event_handlers;
            current_widget = node.widget.clone();
        }

        self.mark_dirty(id, flags);
        current_widget
    }

    fn update_node_from_element(
        &mut self,
        id: NodeId,
        element: Element,
        position: usize,
        measurer: &mut TextI,
    ) {
        let children = self.update_widget_node_from_element(id, element, position, measurer);
        self.diff_children(id, children, measurer);
    }

    fn update_widget_node_from_element(
        &mut self,
        id: NodeId,
        element: Element,
        position: usize,
        measurer: &mut TextI,
    ) -> Vec<Element> {
        let new_props_hash = element.props_hash();
        let parts = element.into_parts();
        let parent_style = self.nodes[id]
            .parent
            .map(|parent| self.nodes[parent].computed_style.clone())
            .unwrap_or_else(|| ComputedStyle::initial(&self.theme));
        let computed_style = parts
            .widget
            .with(|widget| computed_style_for_widget(widget, &parent_style, &self.theme));
        let new_style = parts
            .widget
            .layout_with(|widget| taffy_style_for_widget(widget, &computed_style, measurer));
        let mut flags = DirtyFlags::empty();

        {
            let node = self.nodes.get_mut(id).expect("reused node missing");
            node.position = position;
            node.new_props_hash = new_props_hash;
            if node.old_props_hash != new_props_hash {
                flags |= DirtyFlags::PROPS;
            }
            if node.computed_style != computed_style {
                node.computed_style = computed_style.clone();
                flags |= DirtyFlags::STYLE | DirtyFlags::PAINT;
            }

            let node_taffy_style = self
                .taffy
                .style(node.taffy_node)
                .expect("get taffy node style");
            if *node_taffy_style != new_style {
                flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
                self.taffy
                    .set_style(node.taffy_node, new_style)
                    .expect("failed to update taffy style");
            }
            if node.node_type != parts.widget.with(|widget| widget.node_type()) {
                flags |= DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
            }
            let widget_flags = node
                .widget
                .with_mut(|current| parts.widget.with(|next| current.update_from(next)));
            flags |= widget_flags;
            node.event_handlers = parts.event_handlers;
        }

        self.mark_dirty(id, flags);
        parts.children
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
        self.event_state.clear_node(id);
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
}

impl Default for UiArena {
    fn default() -> Self {
        Self::new()
    }
}

pub fn should_reuse(old: &Node, new: &Element, position: usize) -> bool {
    Some(old.node_type) == new.node_type()
        && old.key == new.key()
        && (old.key.is_some() || old.position == position)
}

pub fn compute_layout_if_needed(tree: &mut UiArena, _node: NodeId) {
    let mut measurer = crate::layout::MockTextMeasurer::default();
    tree.compute_layout_if_needed(Size::ZERO, &mut measurer);
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
        Some(WidgetContext::Text(_props, t)) => {
            let str = t.as_ref().map(|t| t.as_str()).unwrap_or_default();
            let constraints = match available_space.width {
                tf::AvailableSpace::Definite(width) => TextLayoutConstraints::max_width(width),
                tf::AvailableSpace::MaxContent => TextLayoutConstraints::UNBOUNDED,
                _ => TextLayoutConstraints::UNBOUNDED,
            };
            measurer.measure_text_with_constraints(str, _props, constraints);
            Size::ZERO
        }

        _ => Size::ZERO,
    };

    tf::Size {
        width: known_dimensions.width.unwrap_or(measured.width),
        height: known_dimensions.height.unwrap_or(measured.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{TextWidget, button, container, label, row, style_scope};
    use crate::{Color, PaintCommand, Style};

    #[derive(Default)]
    struct RecordingMeasurer {
        constraints: Vec<TextLayoutConstraints>,
    }

    impl TextMeasurer for RecordingMeasurer {
        fn measure_text(&mut self, text: &str, style: &ComputedTextStyle) -> Size {
            self.measure_text_with_constraints(text, style, TextLayoutConstraints::UNBOUNDED)
        }

        fn measure_text_with_constraints(
            &mut self,
            _text: &str,
            _style: &ComputedTextStyle,
            constraints: TextLayoutConstraints,
        ) -> Size {
            self.constraints.push(constraints);
            match constraints.max_width {
                Some(width) => Size::new(width, 20.0),
                None => Size::new(100.0, 10.0),
            }
        }
    }

    #[test]
    fn text_layout_uses_available_width_as_measure_constraint() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let text = arena.insert(
            root,
            TextWidget::new("width constrained"),
            tf::Style::default(),
        );
        let mut measurer = RecordingMeasurer::default();

        arena.update_tree(root, Size::new(50.0, 100.0), &mut measurer);

        assert!(
            measurer
                .constraints
                .iter()
                .any(|constraints| constraints.max_width == Some(50.0))
        );
        assert_eq!(arena.node(text).unwrap().layout.width, 50.0);
        assert_eq!(arena.node(text).unwrap().layout.height, 20.0);
    }

    #[test]
    fn style_scope_inherits_text_style_to_label_and_local_style_overrides() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let element = style_scope(Style::new().color(Color::BLUE_500).font_size(20.0))
            .child(label("inherited"))
            .child(label("local").style(Style::new().color(Color::BLACK)));
        let mut measurer = TextI::new();

        let scope = arena.create_from_element(root, element.into(), 0, &mut measurer);
        let inherited = arena.children(scope)[0];
        let local = arena.children(scope)[1];

        assert_eq!(
            arena.node(inherited).unwrap().computed_style.text.color,
            Color::BLUE_500
        );
        assert_eq!(
            arena.node(inherited).unwrap().computed_style.text.font_size,
            20.0
        );
        assert_eq!(
            arena.node(local).unwrap().computed_style.text.color,
            Color::BLACK
        );
        assert_eq!(
            arena.node(local).unwrap().computed_style.text.font_size,
            20.0
        );
    }

    #[test]
    fn container_style_emits_background_and_border_commands() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let element = container().style(
            Style::new()
                .size(Size::new(40.0, 20.0))
                .background(Color::BLUE_500)
                .border_color(Color::BLACK)
                .border_width(2.0)
                .border_radius(4.0),
        );
        let mut measurer = TextI::new();
        let node = arena.create_from_element(root, element.into(), 0, &mut measurer);
        arena.node_mut(node).unwrap().layout = Rect::new(1.0, 2.0, 40.0, 20.0);
        arena.mark_dirty(node, DirtyFlags::PAINT);
        arena.repaint_if_needed(node);
        let commands = arena.node(node).unwrap().paint_cache.clone();

        assert!(commands.iter().any(|command| matches!(
            command,
            PaintCommand::FillRoundedRect { radius, color, .. }
                if *radius == 4.0 && *color == Color::BLUE_500
        )));
        assert!(commands.iter().any(|command| matches!(
            command,
            PaintCommand::StrokeRoundedRect { radius, color, width, .. }
                if *radius == 4.0 && *color == Color::BLACK && *width == 2.0
        )));
    }

    #[test]
    fn row_gap_from_style_affects_child_layout() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let element = row()
            .style(Style::new().gap(12.0))
            .child(container().style(Style::new().size(Size::new(10.0, 10.0))))
            .child(container().style(Style::new().size(Size::new(10.0, 10.0))));
        let mut measurer = TextI::new();

        let row = arena.create_from_element(root, element.into(), 0, &mut measurer);
        arena.update_tree(root, Size::new(100.0, 30.0), &mut measurer);
        let first = arena.children(row)[0];
        let second = arena.children(row)[1];

        assert_eq!(arena.node(first).unwrap().layout.x, 0.0);
        assert_eq!(arena.node(second).unwrap().layout.x, 22.0);
    }

    #[test]
    fn button_pressed_state_style_repaints_with_primary_background() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let mut measurer = TextI::new();
        let node = arena.create_from_element(root, button("press").into(), 0, &mut measurer);
        arena.node_mut(node).unwrap().layout = Rect::new(0.0, 0.0, 80.0, 30.0);
        let mut dirty = DirtyFlags::empty();
        let mut requests = crate::EventRequests::default();
        arena.node_mut(node).unwrap().widget.with_mut(|widget| {
            let mut cx = crate::EventContext::new(
                node,
                crate::EventPhase::Target,
                &mut dirty,
                &mut requests,
            );
            widget.handle_event(
                &Event::PointerDown {
                    position: crate::Point::new(1.0, 1.0),
                    button: crate::PointerButton::Primary,
                },
                &mut cx,
            );
        });
        arena.mark_dirty(node, dirty);
        arena.update_tree(root, Size::new(100.0, 50.0), &mut measurer);
        let (_, commands) = arena.prepare_paint_commands();

        assert!(commands.iter().any(|command| matches!(
            command,
            PaintCommand::FillRect { color, .. } if *color == Color::BLUE_500
        )));
    }
}

pub fn repaint_if_needed(tree: &mut UiArena, node: NodeId) {
    tree.repaint_if_needed(node);
}
