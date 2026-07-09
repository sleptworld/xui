use slotmap::{SecondaryMap, SlotMap, new_key_type};
use std::any::Any;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;
use taffy::prelude as tf;
pub use xui_interface::Key;
use xui_interface::widget::WidgetType;
use xui_interface::{ComputedStyle, NodeId, PaintCommand};

use crate::HookContext;
use crate::core::Rect;
use crate::element::{ComponentDesc, ElementDesc};
use crate::lanes::{Lanes, NO_LANES};
use crate::widgets::WidgetI;

pub type ErasedProps = Rc<dyn Any>;
pub type ErasedPropsRef<'a> = &'a dyn Any;
pub type ComponentCall = fn(&mut HookContext<'_>, Option<ErasedPropsRef<'_>>) -> ElementDesc;

new_key_type! {
    pub struct FiberId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentType(&'static str);

impl ComponentType {
    pub const ROOT: Self = Self("__xui_root");

    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn name(self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ComponentRender {
    pub component_type: ComponentType,
    pub name: &'static str,
    pub call: ComponentCall,
}

impl ComponentRender {
    pub const fn new(component_type: ComponentType, call: ComponentCall) -> Self {
        Self {
            component_type,
            name: component_type.name(),
            call,
        }
    }
}

impl PartialEq for ComponentRender {
    fn eq(&self, other: &Self) -> bool {
        self.component_type == other.component_type
    }
}

impl Eq for ComponentRender {}

impl Hash for ComponentRender {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.component_type.hash(state);
    }
}

pub struct FiberContext<'a> {
    node_id: FiberId,
    hook_context: HookContext<'a>,
    _marker: std::marker::PhantomData<&'a mut ()>,
}

impl<'a> FiberContext<'a> {
    pub fn new(node_id: FiberId, hook_context: HookContext<'a>) -> Self {
        Self {
            node_id,
            hook_context,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn node_id(&self) -> FiberId {
        self.node_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FiberTag {
    Root,
    Host(WidgetType),
    Component,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct EffectTag: u8 {
        const NONE      = 0;
        const PLACEMENT = 1 << 0;
        const UPDATE    = 1 << 1;
        const MOVE      = 1 << 2;
        const DELETION  = 1 << 3;
    }
}

pub struct HostState {
    pub node_id: Option<NodeId>,
    pub widget: Option<WidgetI>,
    pub taffy_node: Option<tf::NodeId>,
    pub style: tf::Style,
    pub computed_style: ComputedStyle,
    pub layout: Rect,
    pub previous_layout: Rect,
    pub paint_cache: Vec<PaintCommand>,
    pub props_hash: u64,
}

pub struct ComponentState {
    pub render: ComponentRender,
    pub key: Option<Key>,
    pub props_hash: u64,
    pub props: Option<ErasedProps>,
}

pub enum PendingProps {
    Host(HostUpdate),
    Component(ComponentProps),
}

pub struct HostUpdate {
    pub widget: WidgetI,
    pub props_hash: u64,
}

pub struct ComponentProps {
    pub render: ComponentRender,
    pub props_hash: u64,
    pub props: Option<ErasedProps>,
}

pub struct Node {
    pub id: FiberId,
    pub parent: Option<FiberId>,
    pub child: Option<FiberId>,
    pub sibling: Option<FiberId>,
    pub key: Option<Key>,
    pub position: usize,
    pub tag: FiberTag,
    pub effect: EffectTag,
    pub host: Option<HostState>,
    pub component: Option<ComponentState>,
}

impl Node {
    fn root(id: FiberId) -> Self {
        Self {
            id,
            parent: None,
            child: None,
            sibling: None,
            key: None,
            position: 0,
            tag: FiberTag::Root,
            effect: EffectTag::empty(),
            host: None,
            component: None,
        }
    }

    pub fn component(id: FiberId, element: ComponentDesc) -> Self {
        Self {
            id,
            parent: None,
            child: None,
            sibling: None,
            key: element.key.clone(),
            position: 0,
            tag: FiberTag::Component,
            effect: EffectTag::PLACEMENT,
            host: None,
            component: Some(ComponentState {
                render: element.render,
                key: element.key,
                props_hash: element.props_hash,
                props: element.props,
            }),
        }
    }

    pub fn children<'a>(&self, arena: &'a FiberArena) -> NodeChildren<'a, Self> {
        NodeChildren {
            arena: arena,
            child: self.child,
            _marker: PhantomData,
        }
    }
}

pub struct NodeChildren<'a, T> {
    pub arena: &'a FiberArena,
    pub child: Option<FiberId>,
    pub _marker: PhantomData<T>,
}

impl<'a> Iterator for NodeChildren<'a, Node> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(id) = self.child.take() {
            let node = self.arena.node(id);
            self.child = node.and_then(|n| n.sibling);
            node
        } else {
            None
        }
    }
}

pub struct FiberArena {
    ids: SlotMap<FiberId, ()>,
    nodes: SecondaryMap<FiberId, Node>,
    taffy: tf::TaffyTree,
    root: FiberId,
    root_taffy: tf::NodeId,
    next_work: Option<FiberId>,
    deletions: Vec<FiberId>,
    render_lanes: Lanes,
}

impl FiberArena {
    pub fn new() -> Self {
        let mut taffy = tf::TaffyTree::new();
        let root_style = tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Column,
            ..Default::default()
        };
        let root_taffy = taffy
            .new_leaf(root_style)
            .expect("failed to create fiber root taffy node");
        let mut ids = SlotMap::with_key();
        let mut nodes = SecondaryMap::new();
        let root = ids.insert(());
        nodes.insert(root, Node::root(root));
        Self {
            ids,
            nodes,
            taffy,
            root,
            root_taffy,
            next_work: None,
            deletions: Vec::new(),
            render_lanes: NO_LANES,
        }
    }

    pub fn root(&self) -> FiberId {
        self.root
    }

    pub fn contains(&self, id: FiberId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn node(&self, id: FiberId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn node_mut(&mut self, id: FiberId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn insert_node(&mut self, id: FiberId, node: Node) {
        if self.ids.contains_key(id) {
            self.nodes.insert(id, node);
        }
    }

    #[inline]
    pub fn next_work(&self) -> Option<FiberId> {
        self.next_work
    }

    pub fn taffy(&self) -> &tf::TaffyTree {
        &self.taffy
    }

    pub fn children(&self, parent: FiberId) -> Vec<FiberId> {
        let mut output = Vec::new();
        let mut child = self.nodes.get(parent).and_then(|node| node.child);
        while let Some(id) = child {
            output.push(id);
            child = self.nodes.get(id).and_then(|node| node.sibling);
        }
        output
    }

    pub fn cursor(&self, start: FiberId) -> Cursor<'_> {
        Cursor {
            arena: self,
            next: self.nodes.contains_key(start).then_some(start),
        }
    }

    pub fn append_child(&mut self, parent: FiberId, child: FiberId) {
        let mut children = self.children(parent);
        children.push(child);
        self.set_children(parent, &children);
    }

    pub fn set_children(&mut self, parent: FiberId, children: &[FiberId]) {
        if !self.nodes.contains_key(parent) {
            return;
        }

        self.nodes[parent].child = children.first().copied();
        for (position, child) in children.iter().copied().enumerate() {
            if !self.nodes.contains_key(child) {
                continue;
            }
            self.nodes[child].parent = Some(parent);
            self.nodes[child].position = position;
            self.nodes[child].sibling = children.get(position + 1).copied();
        }
    }

    fn remove_subtree_detached(&mut self, id: FiberId) {
        if id == self.root || !self.nodes.contains_key(id) {
            return;
        }

        let children = self.children(id);
        for child in children {
            self.remove_subtree_detached(child);
        }

        if let Some(host) = self.nodes[id].host.as_ref() {
            if let Some(taffy_node) = host.taffy_node {
                let _ = self.taffy.remove(taffy_node);
            }
        }
        self.nodes.remove(id);
    }

    pub fn new_id(&mut self) -> FiberId {
        self.ids.insert(())
    }

    pub fn remove_node(&mut self, id: FiberId) -> Option<Node> {
        self.ids.remove(id);
        self.nodes.remove(id)
    }

    pub fn remove_id(&mut self, id: FiberId) {
        self.ids.remove(id);
    }
}

impl Default for FiberArena {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Cursor<'a> {
    arena: &'a FiberArena,
    next: Option<FiberId>,
}

impl Iterator for Cursor<'_> {
    type Item = FiberId;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.next_after(current);
        Some(current)
    }
}

impl Cursor<'_> {
    fn next_after(&self, current: FiberId) -> Option<FiberId> {
        if let Some(child) = self.arena.nodes.get(current).and_then(|node| node.child) {
            return Some(child);
        }

        let mut cursor = current;
        loop {
            if let Some(sibling) = self.arena.nodes.get(cursor).and_then(|node| node.sibling) {
                return Some(sibling);
            }
            cursor = self.arena.nodes.get(cursor).and_then(|node| node.parent)?;
        }
    }
}
