use std::any::TypeId;
use std::collections::HashSet;

use slotmap::{SlotMap, new_key_type};
use xui_interface::NodeId;

use crate::font::TextI;
use crate::state::{HookContext, HookStorage, Scheduler};
use crate::tree::UiArena;
use crate::widgets::{Element, Key, NodeType};

new_key_type! {
    pub struct RuntimeNodeId;
}

pub type ComponentId = RuntimeNodeId;

pub struct ComponentRuntime {
    nodes: SlotMap<RuntimeNodeId, RuntimeNode>,
    root: RuntimeNodeId,
    scheduler: Scheduler,
}

enum RuntimeNode {
    Component(ComponentNode),
    Widget(WidgetRuntimeNode),
}

struct ComponentNode {
    parent: Option<RuntimeNodeId>,
    parent_widget: NodeId,
    key: Option<Key>,
    type_id: TypeId,
    position: usize,
    hooks: HookStorage,
    render: Box<dyn FnMut(&mut HookContext<'_>) -> Element>,
    rendered_children: Vec<RuntimeNodeId>,
}

struct WidgetRuntimeNode {
    parent: RuntimeNodeId,
    node_id: NodeId,
    key: Option<Key>,
    node_type: NodeType,
    position: usize,
    children: Vec<RuntimeNodeId>,
}

impl ComponentRuntime {
    pub fn new(
        root_widget: NodeId,
        scheduler: Scheduler,
        root_component: impl FnMut(&mut HookContext<'_>) -> Element + 'static,
    ) -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(RuntimeNode::Component(ComponentNode {
            parent: None,
            parent_widget: root_widget,
            key: None,
            type_id: TypeId::of::<()>(),
            position: 0,
            hooks: HookStorage::default(),
            render: Box::new(root_component),
            rendered_children: Vec::new(),
        }));

        scheduler.mark_component_dirty(root);

        Self {
            nodes,
            root,
            scheduler,
        }
    }

    pub fn root(&self) -> ComponentId {
        self.root
    }

    pub fn is_dirty(&self) -> bool {
        self.scheduler.is_dirty()
    }

    pub fn mark_root_dirty(&self) {
        self.scheduler.mark_component_dirty(self.root);
        self.scheduler.mark_root_dirty();
    }

    pub fn rebuild_if_needed(&mut self, arena: &mut UiArena, measurer: &mut TextI) {
        let root_dirty = self.scheduler.take_root_dirty();
        if root_dirty {
            self.scheduler.mark_component_dirty(self.root);
        }

        let mut dirty = self.scheduler.take_dirty_components();
        if dirty.is_empty() {
            return;
        }

        let mut seen = HashSet::new();
        while let Some(id) = dirty.pop() {
            if !seen.insert(id) || !self.nodes.contains_key(id) {
                continue;
            }
            if !matches!(self.nodes.get(id), Some(RuntimeNode::Component(_))) {
                continue;
            }
            self.render_component(id, arena, measurer);
            self.sync_nearest_widget_parent(id, arena);

            for next in self.scheduler.take_dirty_components() {
                dirty.push(next);
            }
        }
    }

    fn render_component(&mut self, id: RuntimeNodeId, arena: &mut UiArena, measurer: &mut TextI) {
        let (parent_widget, element) = {
            let RuntimeNode::Component(node) = self.nodes.get_mut(id).expect("component missing")
            else {
                return;
            };
            let mut cx = HookContext::new(&mut node.hooks, id, self.scheduler.clone());
            (node.parent_widget, (node.render)(&mut cx))
        };

        let old_children = match self.nodes.get(id) {
            Some(RuntimeNode::Component(node)) => node.rendered_children.clone(),
            _ => return,
        };
        let next_children = self.diff_children(
            id,
            parent_widget,
            old_children,
            vec![element],
            arena,
            measurer,
        );

        if let Some(RuntimeNode::Component(node)) = self.nodes.get_mut(id) {
            node.rendered_children = next_children;
        }
    }

    fn diff_children(
        &mut self,
        parent: RuntimeNodeId,
        parent_widget: NodeId,
        old_children: Vec<RuntimeNodeId>,
        new_children: Vec<Element>,
        arena: &mut UiArena,
        measurer: &mut TextI,
    ) -> Vec<RuntimeNodeId> {
        let mut used = vec![false; old_children.len()];
        let mut next_children = Vec::with_capacity(new_children.len());

        for (position, element) in new_children.into_iter().enumerate() {
            let matched = self.find_reusable_child(&old_children, &used, &element, position);
            let id = if let Some(old_index) = matched {
                used[old_index] = true;
                let id = old_children[old_index];
                self.update_runtime_node(
                    id,
                    parent,
                    parent_widget,
                    element,
                    position,
                    arena,
                    measurer,
                );
                id
            } else {
                self.create_runtime_node(parent, parent_widget, element, position, arena, measurer)
            };
            next_children.push(id);
        }

        for (index, old_child) in old_children.into_iter().enumerate() {
            if !used[index] {
                self.unmount(old_child, arena);
            }
        }

        let widget_children = self.flatten_widget_children(&next_children);
        arena.set_children(parent_widget, widget_children);
        next_children
    }

    fn create_runtime_node(
        &mut self,
        parent: RuntimeNodeId,
        parent_widget: NodeId,
        element: Element,
        position: usize,
        arena: &mut UiArena,
        measurer: &mut TextI,
    ) -> RuntimeNodeId {
        match element {
            Element::Component(component) => {
                let key = component.key.clone();
                let type_id = component.type_id;
                let render = component.render;
                let id = self.nodes.insert(RuntimeNode::Component(ComponentNode {
                    parent: Some(parent),
                    parent_widget,
                    key,
                    type_id,
                    position,
                    hooks: HookStorage::default(),
                    render,
                    rendered_children: Vec::new(),
                }));
                self.render_component(id, arena, measurer);
                id
            }
            widget => {
                let key = widget.key();
                let node_type = widget
                    .node_type()
                    .expect("component elements are handled above");
                let (node_id, children) =
                    arena.create_widget_from_element(parent_widget, widget, position, measurer);
                let id = self.nodes.insert(RuntimeNode::Widget(WidgetRuntimeNode {
                    parent,
                    node_id,
                    key,
                    node_type,
                    position,
                    children: Vec::new(),
                }));
                let next_children =
                    self.diff_children(id, node_id, Vec::new(), children, arena, measurer);
                if let Some(RuntimeNode::Widget(node)) = self.nodes.get_mut(id) {
                    node.children = next_children;
                }
                id
            }
        }
    }

    fn update_runtime_node(
        &mut self,
        id: RuntimeNodeId,
        parent: RuntimeNodeId,
        parent_widget: NodeId,
        element: Element,
        position: usize,
        arena: &mut UiArena,
        measurer: &mut TextI,
    ) {
        match element {
            Element::Component(component) => {
                if let Some(RuntimeNode::Component(node)) = self.nodes.get_mut(id) {
                    node.parent = Some(parent);
                    node.parent_widget = parent_widget;
                    node.position = position;
                    node.key = component.key;
                    node.type_id = component.type_id;
                    node.render = component.render;
                }
                self.render_component(id, arena, measurer);
            }
            widget => {
                let (node_id, old_children) = {
                    let RuntimeNode::Widget(node) =
                        self.nodes.get_mut(id).expect("reused widget missing")
                    else {
                        return;
                    };
                    node.parent = parent;
                    node.position = position;
                    node.key = widget.key();
                    node.node_type = widget
                        .node_type()
                        .expect("component elements are handled above");
                    (node.node_id, node.children.clone())
                };

                let children =
                    arena.update_widget_from_element(node_id, widget, position, measurer);
                let next_children =
                    self.diff_children(id, node_id, old_children, children, arena, measurer);
                if let Some(RuntimeNode::Widget(node)) = self.nodes.get_mut(id) {
                    node.children = next_children;
                }
            }
        }
    }

    fn find_reusable_child(
        &self,
        old_children: &[RuntimeNodeId],
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
                        && self
                            .nodes
                            .get(*old_id)
                            .is_some_and(|old| old.matches_keyed(&key, new_child))
                })
                .map(|(index, _)| index);
        }

        old_children
            .get(position)
            .copied()
            .filter(|old_id| {
                !used[position]
                    && self
                        .nodes
                        .get(*old_id)
                        .is_some_and(|old| old.matches_unkeyed(new_child, position))
            })
            .map(|_| position)
    }

    fn unmount(&mut self, id: RuntimeNodeId, arena: &mut UiArena) {
        let Some(node) = self.nodes.remove(id) else {
            return;
        };

        match node {
            RuntimeNode::Component(component) => {
                for child in component.rendered_children {
                    self.unmount(child, arena);
                }
            }
            RuntimeNode::Widget(widget) => {
                for child in widget.children {
                    self.unmount(child, arena);
                }
                arena.remove_subtree(widget.node_id);
            }
        }
    }

    fn flatten_widget_children(&self, children: &[RuntimeNodeId]) -> Vec<NodeId> {
        let mut output = Vec::new();
        for child in children {
            self.flatten_widget_child(*child, &mut output);
        }
        output
    }

    fn flatten_widget_child(&self, id: RuntimeNodeId, output: &mut Vec<NodeId>) {
        match self.nodes.get(id) {
            Some(RuntimeNode::Widget(widget)) => output.push(widget.node_id),
            Some(RuntimeNode::Component(component)) => {
                for child in &component.rendered_children {
                    self.flatten_widget_child(*child, output);
                }
            }
            None => {}
        }
    }

    fn sync_nearest_widget_parent(&self, id: RuntimeNodeId, arena: &mut UiArena) {
        let mut cursor = id;
        loop {
            let parent = match self.nodes.get(cursor) {
                Some(RuntimeNode::Component(node)) => node.parent,
                Some(RuntimeNode::Widget(node)) => Some(node.parent),
                None => return,
            };

            match parent {
                Some(parent_id) => match self.nodes.get(parent_id) {
                    Some(RuntimeNode::Widget(widget)) => {
                        arena.set_children(
                            widget.node_id,
                            self.flatten_widget_children(&widget.children),
                        );
                        return;
                    }
                    Some(RuntimeNode::Component(_)) => cursor = parent_id,
                    None => return,
                },
                None => {
                    if let Some(RuntimeNode::Component(root)) = self.nodes.get(self.root) {
                        arena.set_children(
                            root.parent_widget,
                            self.flatten_widget_children(&root.rendered_children),
                        );
                    }
                    return;
                }
            }
        }
    }
}

impl RuntimeNode {
    fn matches_keyed(&self, key: &Key, element: &Element) -> bool {
        match (self, element) {
            (Self::Component(old), Element::Component(new)) => {
                old.key.as_ref() == Some(key) && old.type_id == new.type_id
            }
            (Self::Widget(old), _) => {
                old.key.as_ref() == Some(key) && Some(old.node_type) == element.node_type()
            }
            _ => false,
        }
    }

    fn matches_unkeyed(&self, element: &Element, position: usize) -> bool {
        match (self, element) {
            (Self::Component(old), Element::Component(new)) => {
                old.key.is_none() && old.position == position && old.type_id == new.type_id
            }
            (Self::Widget(old), _) => {
                old.key.is_none()
                    && old.position == position
                    && Some(old.node_type) == element.node_type()
            }
            _ => false,
        }
    }
}
