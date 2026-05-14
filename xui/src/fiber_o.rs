use std::any::Any;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use rustc_hash::FxHasher;
use slotmap::SlotMap;
use taffy::prelude as tf;
use xui_interface::{DirtyFlags, NodeId, NodeType};

use crate::core::Rect;
use crate::render::PaintCommand;
use crate::widgets::{Key, Widget, WidgetKind};

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

pub struct Fiber {
    component_registry: ComponentRegistry,
    arena: FiberArena,
}

impl Fiber {
    pub fn new() -> Self {
        Self {
            component_registry: ComponentRegistry::new(),
            arena: FiberArena::new(),
        }
    }

    pub fn get_component(&self, component_type: ComponentType) -> &ComponentDef {
        self.component_registry.get(component_type)
    }
}

impl Deref for Fiber {
    type Target = FiberArena;

    fn deref(&self) -> &Self::Target {
        &self.arena
    }
}

impl DerefMut for Fiber {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.arena
    }
}

pub struct FiberContext<'a> {
    node_id: NodeId,
    _marker: std::marker::PhantomData<&'a mut ()>,
}

impl<'a> FiberContext<'a> {
    fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn node_id(&self) -> NodeId {
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
        widget: Box<dyn Widget>,
        style: tf::Style,
        props_hash: u64,
        children: Vec<FiberElement>,
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

    pub fn component<T>(component_type: ComponentType, props: T) -> Self
    where
        T: Any + Hash,
    {
        Self::component_with_hash(component_type, fx_hash(&props), Box::new(props))
    }

    pub fn component_with_hash(
        component_type: ComponentType,
        props_hash: u64,
        props: ErasedProps,
    ) -> Self {
        Self::Component(ComponentElement {
            key: None,
            component_type,
            props_hash,
            props,
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
            element.children.push(child);
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
            Self::Component(element) => FiberTag::Component(element.component_type),
        }
    }
}

pub struct HostElement {
    pub key: Option<Key>,
    pub kind: WidgetKind,
    pub widget: Box<dyn Widget>,
    pub style: tf::Style,
    pub props_hash: u64,
    pub children: Vec<FiberElement>,
}

pub struct ComponentElement {
    pub key: Option<Key>,
    pub component_type: ComponentType,
    pub props_hash: u64,
    pub props: ErasedProps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FiberTag {
    Root,
    Host(NodeType),
    Component(ComponentType),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTag {
    None,
    Placement,
    Update,
}

pub struct HostState {
    pub kind: WidgetKind,
    pub widget: Box<dyn Widget>,
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
    pub widget: Box<dyn Widget>,
    pub style: tf::Style,
    pub props_hash: u64,
}

pub struct ComponentProps {
    pub component_type: ComponentType,
    pub props_hash: u64,
    pub props: ErasedProps,
}

pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub child: Option<NodeId>,
    pub sibling: Option<NodeId>,
    pub key: Option<Key>,
    pub position: usize,
    pub tag: FiberTag,
    pub effect: EffectTag,
    pub dirty: DirtyFlags,
    pub subtree_dirty: DirtyFlags,
    pub pending_props: Option<PendingProps>,
    pub pending_children: Option<Vec<FiberElement>>,
    pub memoized_props_hash: u64,
    pub host: Option<HostState>,
}

impl Node {
    fn root(id: NodeId) -> Self {
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

    fn host(id: NodeId, element: HostElement, taffy_node: tf::NodeId) -> Self {
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
            pending_children: Some(element.children),
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

    fn component(id: NodeId, element: ComponentElement) -> Self {
        Self {
            id,
            parent: None,
            child: None,
            sibling: None,
            key: element.key,
            position: 0,
            tag: FiberTag::Component(element.component_type),
            effect: EffectTag::Placement,
            dirty: DirtyFlags::STATE,
            subtree_dirty: DirtyFlags::empty(),
            pending_props: Some(PendingProps::Component(ComponentProps {
                component_type: element.component_type,
                props_hash: element.props_hash,
                props: element.props,
            })),
            pending_children: None,
            memoized_props_hash: 0,
            host: None,
        }
    }
}

pub struct FiberArena {
    nodes: SlotMap<NodeId, Node>,
    taffy: tf::TaffyTree,
    root: NodeId,
    root_taffy: tf::NodeId,
    next_work: Option<NodeId>,
    deletions: Vec<NodeId>,
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
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert_with_key(Node::root);
        Self {
            nodes,
            taffy,
            root,
            root_taffy,
            next_work: None,
            deletions: Vec::new(),
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

    pub fn taffy(&self) -> &tf::TaffyTree {
        &self.taffy
    }

    pub fn children(&self, parent: NodeId) -> Vec<NodeId> {
        let mut output = Vec::new();
        let mut child = self.nodes.get(parent).and_then(|node| node.child);
        while let Some(id) = child {
            output.push(id);
            child = self.nodes.get(id).and_then(|node| node.sibling);
        }
        output
    }

    pub fn cursor(&self, start: NodeId) -> Cursor<'_> {
        Cursor {
            arena: self,
            next: self.nodes.contains_key(start).then_some(start),
        }
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        let mut children = self.children(parent);
        children.push(child);
        self.set_children(parent, children);
    }

    pub fn set_children(&mut self, parent: NodeId, children: Vec<NodeId>) {
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

    pub fn reconcile(&mut self, root_element: FiberElement, registry: &ComponentRegistry) {
        self.nodes[self.root].pending_children = Some(vec![root_element]);
        self.nodes[self.root].dirty.insert(DirtyFlags::TREE);
        self.next_work = Some(self.root);
        self.perform_work(registry);
    }

    pub fn perform_work(&mut self, registry: &ComponentRegistry) {
        while let Some(work) = self.next_work {
            self.next_work = self.perform_unit_of_work(work, registry);
        }
    }

    pub fn commit(&mut self) {
        let deletions = std::mem::take(&mut self.deletions);
        for deletion in deletions {
            self.remove_subtree_detached(deletion);
        }

        let nodes: Vec<_> = self.cursor(self.root).collect();
        for id in nodes.iter().copied() {
            self.commit_node(id);
        }
        for id in nodes {
            self.sync_host_children(id);
        }
    }

    pub fn render(&mut self, root_element: FiberElement, registry: &ComponentRegistry) {
        self.reconcile(root_element, registry);
        self.commit();
    }

    fn perform_unit_of_work(&mut self, id: NodeId, registry: &ComponentRegistry) -> Option<NodeId> {
        if let Some(child) = self.begin_work(id, registry) {
            return Some(child);
        }

        let mut current = id;
        loop {
            self.complete_work(current);
            if let Some(sibling) = self.nodes.get(current).and_then(|node| node.sibling) {
                return Some(sibling);
            }
            current = self.nodes.get(current).and_then(|node| node.parent)?;
        }
    }

    fn begin_work(&mut self, id: NodeId, registry: &ComponentRegistry) -> Option<NodeId> {
        let tag = self.nodes[id].tag;
        match tag {
            FiberTag::Root => {
                let children = self.nodes[id].pending_children.take().unwrap_or_default();
                self.reconcile_children(id, children);
            }
            FiberTag::Host(_) => {
                let children = self.nodes[id].pending_children.take().unwrap_or_default();
                self.reconcile_children(id, children);
            }
            FiberTag::Component(component_type) => {
                let rendered = {
                    let node = self.nodes.get(id).expect("component node missing");
                    let Some(PendingProps::Component(props)) = node.pending_props.as_ref() else {
                        return node.child;
                    };
                    let mut cx = FiberContext::new(id);
                    registry
                        .get(component_type)
                        .call(&mut cx, props.props.as_ref())
                };
                self.reconcile_children(id, vec![rendered]);
            }
        }
        self.nodes[id].child
    }

    fn complete_work(&mut self, _id: NodeId) {}

    fn reconcile_children(&mut self, parent: NodeId, new_children: Vec<FiberElement>) {
        let old_children = self.children(parent);
        let mut used = vec![false; old_children.len()];
        let mut next_children = Vec::with_capacity(new_children.len());

        for (position, element) in new_children.into_iter().enumerate() {
            let matched = self.find_reusable_child(&old_children, &used, &element, position);
            let child = if let Some(old_index) = matched {
                used[old_index] = true;
                let id = old_children[old_index];
                self.prepare_reused_node(id, element, position);
                id
            } else {
                self.create_node_from_element(element, position)
            };
            next_children.push(child);
        }

        for (index, old_child) in old_children.into_iter().enumerate() {
            if !used[index] {
                self.deletions.push(old_child);
            }
        }

        self.set_children(parent, next_children);
        self.mark_subtree_dirty(parent, DirtyFlags::TREE);
    }

    fn find_reusable_child(
        &self,
        old_children: &[NodeId],
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

    fn create_node_from_element(&mut self, element: FiberElement, position: usize) -> NodeId {
        match element {
            FiberElement::Host(element) => {
                let taffy_node = self
                    .taffy
                    .new_leaf(element.style.clone())
                    .expect("failed to create fiber taffy node");
                let id = self
                    .nodes
                    .insert_with_key(|id| Node::host(id, element, taffy_node));
                self.nodes[id].position = position;
                id
            }
            FiberElement::Component(element) => {
                let id = self
                    .nodes
                    .insert_with_key(|id| Node::component(id, element));
                self.nodes[id].position = position;
                id
            }
        }
    }

    fn prepare_reused_node(&mut self, id: NodeId, element: FiberElement, position: usize) {
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
                self.nodes[id].pending_children = Some(element.children);
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
                self.nodes[id].pending_props = Some(PendingProps::Component(ComponentProps {
                    component_type: element.component_type,
                    props_hash: element.props_hash,
                    props: element.props,
                }));
            }
        }

        self.nodes[id].position = position;
        self.nodes[id].sibling = None;
        self.nodes[id].effect = effect;
        if effect == EffectTag::Update {
            self.mark_subtree_dirty(id, DirtyFlags::PROPS);
        }
    }

    fn commit_node(&mut self, id: NodeId) {
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

    fn sync_host_children(&mut self, id: NodeId) {
        let Some(parent_taffy) = self.host_taffy_node(id) else {
            return;
        };
        let taffy_children = self.flatten_host_children(id);
        self.taffy
            .set_children(parent_taffy, &taffy_children)
            .expect("failed to sync fiber taffy children");
    }

    fn host_taffy_node(&self, id: NodeId) -> Option<tf::NodeId> {
        if id == self.root {
            return Some(self.root_taffy);
        }
        self.nodes
            .get(id)
            .and_then(|node| node.host.as_ref())
            .map(|host| host.taffy_node)
    }

    fn flatten_host_children(&self, parent: NodeId) -> Vec<tf::NodeId> {
        let mut output = Vec::new();
        let mut child = self.nodes.get(parent).and_then(|node| node.child);
        while let Some(id) = child {
            self.flatten_host_child(id, &mut output);
            child = self.nodes.get(id).and_then(|node| node.sibling);
        }
        output
    }

    fn flatten_host_child(&self, id: NodeId, output: &mut Vec<tf::NodeId>) {
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

    fn remove_subtree_detached(&mut self, id: NodeId) {
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

    fn mark_subtree_dirty(&mut self, id: NodeId, flags: DirtyFlags) {
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
}

impl Default for FiberArena {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Cursor<'a> {
    arena: &'a FiberArena,
    next: Option<NodeId>,
}

impl Iterator for Cursor<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.next_after(current);
        Some(current)
    }
}

impl Cursor<'_> {
    fn next_after(&self, current: NodeId) -> Option<NodeId> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::widget_from_kind;

    fn style() -> tf::Style {
        tf::Style::default()
    }

    fn host(kind: WidgetKind, children: Vec<FiberElement>) -> FiberElement {
        FiberElement::host(
            kind.clone(),
            widget_from_kind(kind.clone(), None),
            style(),
            test_kind_hash(&kind),
            children,
        )
    }

    fn test_kind_hash(kind: &WidgetKind) -> u64 {
        let mut hasher = FxHasher::default();
        kind.node_type().hash(&mut hasher);
        match kind {
            WidgetKind::Root => {}
            WidgetKind::Label {
                text,
                color,
                font_size,
            } => {
                text.hash(&mut hasher);
                color.r.to_bits().hash(&mut hasher);
                color.g.to_bits().hash(&mut hasher);
                color.b.to_bits().hash(&mut hasher);
                color.a.to_bits().hash(&mut hasher);
                font_size.to_bits().hash(&mut hasher);
            }
            WidgetKind::Button {
                text,
                pressed,
                hovered,
            } => {
                text.hash(&mut hasher);
                pressed.hash(&mut hasher);
                hovered.hash(&mut hasher);
            }
            WidgetKind::Column { gap } | WidgetKind::Row { gap } => {
                gap.to_bits().hash(&mut hasher);
            }
            WidgetKind::Container { background } => {
                background.r.to_bits().hash(&mut hasher);
                background.g.to_bits().hash(&mut hasher);
                background.b.to_bits().hash(&mut hasher);
                background.a.to_bits().hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn keyed_host(key: &str, kind: WidgetKind, children: Vec<FiberElement>) -> FiberElement {
        host(kind, children).key(key)
    }

    fn label(text: &str) -> FiberElement {
        host(
            WidgetKind::Label {
                text: text.to_owned(),
                color: crate::core::Color::BLACK,
                font_size: 14.0,
            },
            Vec::new(),
        )
    }

    fn row(children: Vec<FiberElement>) -> FiberElement {
        host(WidgetKind::Row { gap: 0.0 }, children)
    }

    #[test]
    fn cursor_traverses_preorder_without_recursion() {
        let registry = ComponentRegistry::new();
        let mut arena = FiberArena::new();
        arena.render(
            row(vec![
                keyed_host("a", WidgetKind::Column { gap: 0.0 }, vec![label("a1")]),
                keyed_host(
                    "b",
                    WidgetKind::Container {
                        background: crate::core::Color::WHITE,
                    },
                    vec![],
                ),
                keyed_host("c", WidgetKind::Row { gap: 1.0 }, vec![label("c1")]),
            ]),
            &registry,
        );

        let ids: Vec<_> = arena.cursor(arena.root()).collect();
        let tags: Vec<_> = ids.iter().map(|id| arena.node(*id).unwrap().tag).collect();

        assert_eq!(
            tags,
            vec![
                FiberTag::Root,
                FiberTag::Host(NodeType::Row),
                FiberTag::Host(NodeType::Column),
                FiberTag::Host(NodeType::Label),
                FiberTag::Host(NodeType::Container),
                FiberTag::Host(NodeType::Row),
                FiberTag::Host(NodeType::Label),
            ]
        );
    }

    #[test]
    fn set_children_maintains_parent_sibling_and_position_links() {
        let mut arena = FiberArena::new();
        let a = arena.create_node_from_element(label("a"), 0);
        let b = arena.create_node_from_element(label("b"), 0);
        let c = arena.create_node_from_element(label("c"), 0);

        arena.append_child(arena.root(), a);
        arena.set_children(arena.root(), vec![c, a, b]);

        assert_eq!(arena.children(arena.root()), vec![c, a, b]);
        assert_eq!(arena.node(c).unwrap().parent, Some(arena.root()));
        assert_eq!(arena.node(c).unwrap().position, 0);
        assert_eq!(arena.node(c).unwrap().sibling, Some(a));
        assert_eq!(arena.node(a).unwrap().position, 1);
        assert_eq!(arena.node(a).unwrap().sibling, Some(b));
        assert_eq!(arena.node(b).unwrap().position, 2);
        assert_eq!(arena.node(b).unwrap().sibling, None);
    }

    #[test]
    fn registry_component_commits_flattened_host_children() {
        fn render_label(_cx: &mut FiberContext<'_>, props: &String) -> FiberElement {
            label(props)
        }

        let mut registry = ComponentRegistry::new();
        let component_type = registry.register("LabelComponent", render_label);
        let mut arena = FiberArena::new();

        arena.render(
            row(vec![FiberElement::component(
                component_type,
                "inside".to_owned(),
            )]),
            &registry,
        );

        let root_child = arena.children(arena.root())[0];
        let component = arena.children(root_child)[0];
        let rendered_label = arena.children(component)[0];

        assert_eq!(
            arena.node(component).unwrap().tag,
            FiberTag::Component(component_type)
        );
        assert_eq!(
            arena.node(rendered_label).unwrap().tag,
            FiberTag::Host(NodeType::Label)
        );
        let row_taffy = arena
            .node(root_child)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .taffy_node;
        let label_taffy = arena
            .node(rendered_label)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .taffy_node;
        assert_eq!(arena.taffy.children(row_taffy).unwrap(), vec![label_taffy]);
    }

    #[test]
    fn keyed_reorder_reuses_node_ids_and_updates_sibling_order() {
        let registry = ComponentRegistry::new();
        let mut arena = FiberArena::new();

        arena.render(
            row(vec![
                keyed_host(
                    "a",
                    WidgetKind::Label {
                        text: "a".to_owned(),
                        color: crate::core::Color::BLACK,
                        font_size: 14.0,
                    },
                    vec![],
                ),
                keyed_host(
                    "b",
                    WidgetKind::Label {
                        text: "b".to_owned(),
                        color: crate::core::Color::BLACK,
                        font_size: 14.0,
                    },
                    vec![],
                ),
            ]),
            &registry,
        );
        let row_id = arena.children(arena.root())[0];
        let before = arena.children(row_id);

        arena.render(
            row(vec![
                keyed_host(
                    "b",
                    WidgetKind::Label {
                        text: "b".to_owned(),
                        color: crate::core::Color::BLACK,
                        font_size: 14.0,
                    },
                    vec![],
                ),
                keyed_host(
                    "a",
                    WidgetKind::Label {
                        text: "a".to_owned(),
                        color: crate::core::Color::BLACK,
                        font_size: 14.0,
                    },
                    vec![],
                ),
            ]),
            &registry,
        );

        let after = arena.children(row_id);
        assert_eq!(after, vec![before[1], before[0]]);
        assert_eq!(arena.node(after[0]).unwrap().sibling, Some(after[1]));
        assert_eq!(arena.node(after[1]).unwrap().sibling, None);
    }

    #[test]
    fn same_key_different_component_type_replaces_subtree() {
        fn render_a(_cx: &mut FiberContext<'_>, _props: &String) -> FiberElement {
            label("a")
        }
        fn render_b(_cx: &mut FiberContext<'_>, _props: &String) -> FiberElement {
            label("b")
        }

        let mut registry = ComponentRegistry::new();
        let a_type = registry.register("A", render_a);
        let b_type = registry.register("B", render_b);
        let mut arena = FiberArena::new();

        arena.render(FiberElement::component(a_type, ()).key("same"), &registry);
        let first_component = arena.children(arena.root())[0];

        arena.render(FiberElement::component(b_type, ()).key("same"), &registry);
        let second_component = arena.children(arena.root())[0];

        assert_ne!(first_component, second_component);
        assert!(!arena.contains(first_component));
        assert_eq!(
            arena.node(second_component).unwrap().tag,
            FiberTag::Component(b_type)
        );
    }

    #[test]
    fn deletion_commit_removes_old_subtree() {
        let registry = ComponentRegistry::new();
        let mut arena = FiberArena::new();

        arena.render(
            row(vec![
                keyed_host("a", WidgetKind::Column { gap: 0.0 }, vec![label("child")]),
                keyed_host(
                    "b",
                    WidgetKind::Container {
                        background: crate::core::Color::WHITE,
                    },
                    vec![],
                ),
            ]),
            &registry,
        );
        let row_id = arena.children(arena.root())[0];
        let removed_parent = arena.children(row_id)[0];
        let removed_child = arena.children(removed_parent)[0];

        arena.render(
            row(vec![keyed_host(
                "b",
                WidgetKind::Container {
                    background: crate::core::Color::WHITE,
                },
                vec![],
            )]),
            &registry,
        );

        assert!(!arena.contains(removed_parent));
        assert!(!arena.contains(removed_child));
    }

    #[test]
    fn host_update_marks_update_not_placement() {
        let registry = ComponentRegistry::new();
        let mut arena = FiberArena::new();

        arena.render(label("before").key("label"), &registry);
        let label_id = arena.children(arena.root())[0];

        arena.reconcile(label("after").key("label"), &registry);

        assert_eq!(arena.node(label_id).unwrap().effect, EffectTag::Update);
        arena.commit();
        assert_eq!(arena.node(label_id).unwrap().effect, EffectTag::None);
    }
}
