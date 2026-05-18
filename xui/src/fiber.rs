use rustc_hash::FxHasher;
use slotmap::{SecondaryMap, SlotMap, new_key_type};
use smallvec::SmallVec;
use std::any::Any;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;
use taffy::prelude as tf;
use xui_interface::DirtyFlags;
use xui_interface::widget::WidgetType;

use crate::HookContext;
use crate::core::Rect;
use crate::lanes::{Lanes, NO_LANES, SYNC_LANE};
use crate::render::PaintCommand;
use crate::widgets::{Key, Widget, WidgetKind, WidgetRef};

new_key_type! {
    pub struct FiberId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentType(u32);

impl ComponentType {
    pub const ROOT: Self = Self(0);

    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

pub type ErasedProps = Box<dyn Any>;
pub type ErasedPropsRef<'a> = &'a dyn Any;

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

pub struct TypedComponent<F, P> {
    f: F,
    _props: PhantomData<fn() -> P>,
}

impl<F, P> TypedComponent<F, P> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            _props: PhantomData,
        }
    }
}

pub trait ComponentFn {
    fn call(&self, cx: &mut FiberContext<'_>, props: ErasedPropsRef) -> FiberElement;
}

impl<F, P> ComponentFn for TypedComponent<F, P>
where
    P: Any,
    F: for<'cx, 'ctx, 'p> Fn(&'cx mut FiberContext<'ctx>, &'p P) -> FiberElement,
{
    fn call(&self, cx: &mut FiberContext<'_>, props: ErasedPropsRef) -> FiberElement {
        (self.f)(
            cx,
            props
                .downcast_ref::<P>()
                .expect("invalid component props type"),
        )
    }
}

pub struct ComponentDef {
    pub name: &'static str,
    create: Box<dyn ComponentFn>,
}

impl ComponentDef {
    pub fn call(&self, cx: &mut FiberContext<'_>, props: ErasedPropsRef<'_>) -> FiberElement {
        self.create.call(cx, props)
    }
}

#[derive(Default)]
pub struct ComponentRegistry {
    components: Vec<ComponentDef>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P: 'static>(
        &mut self,
        name: &'static str,
        create: for<'cx, 'ctx, 'p> fn(&'cx mut FiberContext<'ctx>, &'p P) -> FiberElement,
    ) -> ComponentType {
        let id = self.components.len() as u32 + 1;
        let typed_component: TypedComponent<fn(&mut FiberContext<'_>, &P) -> FiberElement, P> =
            TypedComponent::new(create);

        self.components.push(ComponentDef {
            name,
            create: Box::new(typed_component) as Box<dyn ComponentFn>,
        });

        ComponentType::new(id)
    }

    pub fn get(&self, component_type: ComponentType) -> &ComponentDef {
        assert_ne!(
            component_type,
            ComponentType::ROOT,
            "root is not a registered component"
        );
        self.components
            .get((component_type.0 - 1) as usize)
            .expect("invalid component type")
    }
}

pub enum FiberElement {
    Host(HostElement),
    Component(ComponentElement),
}

impl FiberElement {
    pub fn host(
        kind: WidgetKind,
        widget: WidgetRef,
        style: tf::Style,
        props_hash: u64,
        children: SmallVec<[Rc<FiberElement>;20]>,
    ) -> Self {
        Self::Host(HostElement {
            key: None,
            kind,
            widget,
            style,
            props_hash,
            children,
        })
    }

    pub fn component(component_type: ComponentType, props_hash: u64) -> Self
    {
        Self::component_with_hash(component_type, props_hash)
    }

    pub fn component_with_hash(
        component_type: ComponentType,
        props_hash: u64,
    ) -> Self {
        Self::Component(ComponentElement {
            key: None,
            component_type,
            props_hash,
        })
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        match &mut self {
            Self::Host(element) => element.key = Some(key.into()),
            Self::Component(element) => element.key = Some(key.into()),
        }
        self
    }

    pub fn child(mut self, child: FiberElement) -> Self {
        if let Self::Host(element) = &mut self {
            element.children.push(Rc::new(child));
        }
        self
    }

    fn key_ref(&self) -> Option<&Key> {
        match self {
            Self::Host(element) => element.key.as_ref(),
            Self::Component(element) => element.key.as_ref(),
        }
    }

    fn tag(&self) -> FiberTag {
        match self {
            Self::Host(element) => FiberTag::Host(element.kind.node_type()),
            Self::Component(_) => FiberTag::Component,
        }
    }
}

pub struct HostElement {
    pub key: Option<Key>,
    pub kind: WidgetKind,
    pub widget: WidgetRef,
    pub style: tf::Style,
    pub props_hash: u64,
    pub children: SmallVec<[Rc<FiberElement>;20]>,
}

pub struct ComponentElement {
    pub key: Option<Key>,
    pub component_type: ComponentType,
    pub props_hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FiberTag {
    Root,
    Host(WidgetType),
    Component,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTag {
    None,
    Placement,
    Update,
}

pub struct HostState {
    pub kind: WidgetKind,
    pub widget: WidgetRef,
    pub taffy_node: tf::NodeId,
    pub style: tf::Style,
    pub layout: Rect,
    pub previous_layout: Rect,
    pub paint_cache: Vec<PaintCommand>,
}

pub enum PendingProps {
    Host(HostUpdate),
    Component(ComponentProps),
}

pub struct HostUpdate {
    pub kind: WidgetKind,
    pub widget: WidgetRef,
    pub style: tf::Style,
    pub props_hash: u64,
}

pub struct ComponentProps {
    pub component_type: ComponentType,
    pub props_hash: u64,
    pub props: ErasedProps,
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
    pub dirty: DirtyFlags,
    pub subtree_dirty: DirtyFlags,
    pub pending_props: Option<PendingProps>,
    pub pending_children: Option<SmallVec<[FiberElement;20]>>,
    pub memoized_props_hash: u64,
    pub host: Option<HostState>,
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
            effect: EffectTag::None,
            dirty: DirtyFlags::default(),
            subtree_dirty: DirtyFlags::empty(),
            pending_props: None,
            pending_children: None,
            memoized_props_hash: 0,
            host: None,
        }
    }

    fn host(id: FiberId, element: HostElement, taffy_node: tf::NodeId) -> Self {
        let tag = FiberTag::Host(element.kind.node_type());
        Self {
            id,
            parent: None,
            child: None,
            sibling: None,
            key: element.key,
            position: 0,
            tag,
            effect: EffectTag::Placement,
            dirty: DirtyFlags::default(),
            subtree_dirty: DirtyFlags::empty(),
            pending_props: None,
            pending_children: None,
            memoized_props_hash: element.props_hash,
            host: Some(HostState {
                kind: element.kind,
                widget: element.widget,
                taffy_node,
                style: element.style,
                layout: Rect::ZERO,
                previous_layout: Rect::ZERO,
                paint_cache: Vec::new(),
            }),
        }
    }

    fn component(id: FiberId, element: ComponentElement) -> Self {
        Self {
            id,
            parent: None,
            child: None,
            sibling: None,
            key: element.key,
            position: 0,
            tag: FiberTag::Component,
            effect: EffectTag::Placement,
            dirty: DirtyFlags::STATE,
            subtree_dirty: DirtyFlags::empty(),
            pending_props: None,
            pending_children: None,
            memoized_props_hash: 0,
            host: None,
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
        let nodes = SecondaryMap::new();
        // let root = nodes.insert_with_key(Node::root);
        let root = ids.insert(());
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
        self.set_children(parent, children);
    }

    pub fn set_children(&mut self, parent: FiberId, children: Vec<FiberId>) {
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

    fn find_reusable_child(
        &self,
        old_children: &[FiberId],
        used: &[bool],
        element: &FiberElement,
        position: usize,
    ) -> Option<usize> {
        let tag = element.tag();
        if let Some(key) = element.key_ref() {
            return old_children
                .iter()
                .copied()
                .enumerate()
                .find(|(index, old_id)| {
                    !used[*index]
                        && self.nodes[*old_id].key.as_ref() == Some(key)
                        && self.nodes[*old_id].tag == tag
                })
                .map(|(index, _)| index);
        }

        old_children
            .get(position)
            .copied()
            .filter(|old_id| {
                !used[position]
                    && self.nodes[*old_id].key.is_none()
                    && self.nodes[*old_id].tag == tag
                    && self.nodes[*old_id].position == position
            })
            .map(|_| position)
    }

    fn prepare_reused_node(&mut self, id: FiberId, element: FiberElement, position: usize) {
        let mut effect = EffectTag::None;
        match element {
            FiberElement::Host(element) => {
                let host = self.nodes[id]
                    .host
                    .as_ref()
                    .expect("host fiber missing host state");
                if self.nodes[id].memoized_props_hash != element.props_hash
                    || host.kind != element.kind
                    || host.style != element.style
                {
                    effect = EffectTag::Update;
                }
                self.nodes[id].key = element.key;
                // self.nodes[id].pending_children = Some(element.children);
                self.nodes[id].pending_props = Some(PendingProps::Host(HostUpdate {
                    kind: element.kind,
                    widget: element.widget,
                    style: element.style,
                    props_hash: element.props_hash,
                }));
            }
            FiberElement::Component(element) => {
                if self.nodes[id].memoized_props_hash != element.props_hash {
                    effect = EffectTag::Update;
                }
                self.nodes[id].key = element.key;
                // self.nodes[id].pending_props = Some(PendingProps::Component(ComponentProps {
                //     component_type: element.component_type,
                //     props_hash: element.props_hash,
                //     props: element.props,
                // }));
            }
        }

        self.nodes[id].position = position;
        self.nodes[id].sibling = None;
        self.nodes[id].effect = effect;
        if effect == EffectTag::Update {
            self.mark_subtree_dirty(id, DirtyFlags::PROPS);
        }
    }

    fn commit_node(&mut self, id: FiberId) {
        let pending = self.nodes[id].pending_props.take();
        match pending {
            Some(PendingProps::Host(update)) => {
                if let Some(host) = self.nodes[id].host.as_mut() {
                    if host.style != update.style {
                        self.taffy
                            .set_style(host.taffy_node, update.style.clone())
                            .expect("failed to update fiber taffy style");
                    }
                    host.kind = update.kind;
                    host.widget = update.widget;
                    host.style = update.style;
                    self.nodes[id].memoized_props_hash = update.props_hash;
                }
            }
            Some(PendingProps::Component(props)) => {
                self.nodes[id].memoized_props_hash = props.props_hash;
            }
            None => {}
        }

        self.nodes[id].effect = EffectTag::None;
        self.nodes[id].dirty = DirtyFlags::empty();
        self.nodes[id].subtree_dirty = DirtyFlags::empty();
    }

    fn sync_host_children(&mut self, id: FiberId) {
        let Some(parent_taffy) = self.host_taffy_node(id) else {
            return;
        };
        let taffy_children = self.flatten_host_children(id);
        self.taffy
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to sync fiber taffy children");
    }

    fn host_taffy_node(&self, id: FiberId) -> Option<tf::NodeId> {
        if id == self.root {
            return Some(self.root_taffy);
        }
        self.nodes
            .get(id)
            .and_then(|node| node.host.as_ref())
            .map(|host| host.taffy_node)
    }

    fn flatten_host_children(&self, parent: FiberId) -> Vec<tf::NodeId> {
        let mut output = Vec::new();
        let mut child = self.nodes.get(parent).and_then(|node| node.child);
        while let Some(id) = child {
            self.flatten_host_child(id, &mut output);
            child = self.nodes.get(id).and_then(|node| node.sibling);
        }
        output
    }

    fn flatten_host_child(&self, id: FiberId, output: &mut Vec<tf::NodeId>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };

        if let Some(host) = node.host.as_ref() {
            output.push(host.taffy_node);
            return;
        }

        let mut child = node.child;
        while let Some(child_id) = child {
            self.flatten_host_child(child_id, output);
            child = self.nodes.get(child_id).and_then(|node| node.sibling);
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
            let _ = self.taffy.remove(host.taffy_node);
        }
        self.nodes.remove(id);
    }

    fn mark_subtree_dirty(&mut self, id: FiberId, flags: DirtyFlags) {
        if flags.is_empty() || !self.nodes.contains_key(id) {
            return;
        }

        self.nodes[id].dirty |= flags;
        let mut current = id;
        while let Some(parent) = self.nodes[current].parent {
            self.nodes[parent].subtree_dirty |= flags;
            current = parent;
        }
    }

    pub fn new_id(&mut self) -> FiberId {
        self.ids.insert(())
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

fn fx_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}
