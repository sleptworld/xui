use slotmap::{SecondaryMap, SlotMap, new_key_type};
use std::any::{Any, TypeId};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;
pub use xui_interface::Key;
use xui_interface::NodeId;

use crate::HookContext;
use crate::element::ElementDesc;
use crate::element::PortalBehavior;
use crate::widgets::{OverlayEntryId, OverlayEntryOptions, OverlayScopeId};

pub type ErasedProps = Rc<dyn Any>;
/// The props a [`ComponentCall`] is handed: the allocation itself, which
/// [`Share::share`](crate::prop::Share::share) projects into.
pub type ErasedPropsRef<'a> = &'a ErasedProps;
pub type ComponentCall = fn(&mut HookContext<'_>, Option<ErasedPropsRef<'_>>) -> ElementDesc;

new_key_type! {
    pub struct FiberId;
}

/// Identity of a component for reconciliation: two fibers are the same
/// component only if both the name and the type id agree.
///
/// The name alone is not enough for a generic component: `list::<A>` and
/// `list::<B>` share one name, and reusing a fiber across them would hand
/// `list::<B>` props and hook slots typed for `list::<A>`. `#[component]`
/// passes the props type, so every monomorphization gets its own id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentType {
    name: &'static str,
    id: TypeId,
}

struct RootComponent;

impl ComponentType {
    pub const ROOT: Self = Self::of::<RootComponent>("__xui_root");

    pub const fn new(name: &'static str, id: TypeId) -> Self {
        Self { name, id }
    }

    /// A component identified by `name` and the type `T`, typically its props.
    pub const fn of<T: ?Sized + 'static>(name: &'static str) -> Self {
        Self::new(name, TypeId::of::<T>())
    }

    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn id(self) -> TypeId {
        self.id
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

/// Which kind of fiber a node is, without its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FiberTag {
    /// A component, the root included: hooks and a render function.
    Component,
    /// A template: the hosts it owns, and its slots as children. The only
    /// fiber that owns hosts.
    Template,
    /// A list of elements standing in one place.
    Fragment,
    /// Elements mounted under the overlayer, kept in the logical tree here.
    Portal,
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

pub struct ComponentState {
    pub render: ComponentRender,
    pub key: Option<Key>,
    pub props: Option<ErasedProps>,
}

pub struct PortalState {
    pub scope: Option<OverlayScopeId>,
    pub options: OverlayEntryOptions,
    pub behavior: PortalBehavior,
    pub entry: Option<OverlayEntryId>,
    pub visual_root: Option<NodeId>,
}

pub struct Node {
    pub id: FiberId,
    pub parent: Option<FiberId>,
    pub child: Option<FiberId>,
    pub sibling: Option<FiberId>,
    pub key: Option<Key>,
    pub position: usize,
    pub effect: EffectTag,
    pub kind: FiberKind,
}

/// A fiber's kind, with what it keeps between renders.
pub enum FiberKind {
    Component(ComponentState),
    Template(TemplateState),
    Fragment,
    Portal(PortalState),
}

impl FiberKind {
    pub fn tag(&self) -> FiberTag {
        match self {
            Self::Component(_) => FiberTag::Component,
            Self::Template(_) => FiberTag::Template,
            Self::Fragment => FiberTag::Fragment,
            Self::Portal(_) => FiberTag::Portal,
        }
    }
}

/// A mounted template: which call site it is, and its hosts.
pub struct TemplateState {
    pub(crate) meta: &'static crate::template::TemplateMeta,
    pub(crate) identity: crate::template::TemplateIdentity,
    /// The host each node is, by node index; `None` for a slot.
    pub(crate) nodes: Vec<Option<NodeId>>,
}

impl TemplateState {
    /// The template's root host.
    pub(crate) fn root(&self) -> NodeId {
        self.nodes[0].expect("a template's root is a host")
    }
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
            effect: EffectTag::empty(),
            // Made the root component by the runtime that owns the arena.
            kind: FiberKind::Fragment,
        }
    }

    pub fn tag(&self) -> FiberTag {
        self.kind.tag()
    }

    /// The host this fiber is, when it is a template: its root.
    pub(crate) fn host(&self) -> Option<NodeId> {
        match &self.kind {
            FiberKind::Template(template) => Some(template.root()),
            _ => None,
        }
    }

    pub(crate) fn component(&self) -> Option<&ComponentState> {
        match &self.kind {
            FiberKind::Component(component) => Some(component),
            _ => None,
        }
    }

    pub(crate) fn template(&self) -> Option<&TemplateState> {
        match &self.kind {
            FiberKind::Template(template) => Some(template),
            _ => None,
        }
    }

    pub(crate) fn portal(&self) -> Option<&PortalState> {
        match &self.kind {
            FiberKind::Portal(portal) => Some(portal),
            _ => None,
        }
    }

    pub fn children<'a>(&self, arena: &'a FiberArena) -> NodeChildren<'a, Self> {
        NodeChildren {
            arena,
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
    root: FiberId,
}

impl FiberArena {
    pub fn new() -> Self {
        let mut ids = SlotMap::with_key();
        let mut nodes = SecondaryMap::new();
        let root = ids.insert(());
        nodes.insert(root, Node::root(root));
        Self { ids, nodes, root }
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

    pub fn children(&self, parent: FiberId) -> Vec<FiberId> {
        let mut output = Vec::new();
        let mut child = self.nodes.get(parent).and_then(|node| node.child);
        while let Some(id) = child {
            output.push(id);
            child = self.nodes.get(id).and_then(|node| node.sibling);
        }
        output
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
