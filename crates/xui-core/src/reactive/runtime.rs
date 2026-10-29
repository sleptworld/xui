//! Mounting (binding), flushing, and the host tree it writes into.
//!
//! A scope's template nodes are hosts, child components and blocks. Hosts
//! are created in [`UiRuntime`] and placed under their parent host; a child
//! component's or a block's rows' hosts are placed directly under the host
//! the component or block stands in, before the next host that follows it.
//! There is no fragment host: a block is only a position.
use std::any::Any;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::rc::Rc;

use slotmap::SlotMap;
use xui_interface::{NodeId, WidgetUpdateFlags};

use super::block::{BlockCx, BlockState, Row, RowParts};
use super::graph::{EffectId, Graph, InstanceId, Notifier, SourceId};
use super::props::{Component, Derived, PropCx};
use super::signal::Signal;
use super::template::{ChildSpec, Context, HostDraft, ComponentDef, EffectSpec, Lane, MountCx, NodeKind, PendingMount, Run, Scope, StaticNode};
use crate::node::Draft;
use crate::property::{IntoProp, Property, PropertyMarker};
use super::host::{HostPatcher, PatchScope};
use crate::ui_runtime::UiRuntime;
use crate::widgets::WidgetVariant;

/// What a running effect or handler gets. Writes go through it: only code
/// given a `Cx` can set a signal.
pub struct Cx<'a> {
    notifier: &'a Notifier,
    /// The hosts of the scope a binding runs in; `None` for handlers and
    /// `effect!`s, which write no host.
    host: Option<HostCx<'a>>,
}

struct HostCx<'a> {
    ui: &'a mut UiRuntime,
    hosts: &'a [Option<NodeId>],
    instance: &'a Rc<dyn AnyInstance>,
}

impl<'a> Cx<'a> {
    /// The `Cx` of a handler: writes, and nothing else.
    pub(crate) fn handler(notifier: &'a Notifier) -> Self {
        Self { notifier, host: None }
    }

    #[doc(hidden)]
    pub fn __handler(notifier: &'a Notifier) -> Self {
        Self::handler(notifier)
    }

    pub(crate) fn notify(&mut self, id: SourceId) {
        self.notifier.push(id);
    }

    #[doc(hidden)]
    pub fn __notifier(&self) -> &Notifier {
        self.notifier
    }

    fn host(&mut self) -> &mut HostCx<'a> {
        self.host.as_mut().expect("only a binding writes its hosts")
    }

    /// Writes some of host `node`'s properties through `patch`.
    #[doc(hidden)]
    pub fn __patch<N: WidgetVariant>(
        &mut self,
        node: u16,
        _tag: fn() -> Draft<N>,
        patch: impl FnOnce(&mut HostPatcher<'_, '_, N>) -> WidgetUpdateFlags,
    ) {
        let host = self.host();
        let id = host.hosts[usize::from(node)].expect("a binding writes a mounted host");
        host.ui.patch_node(id, |widgets, id, columns, cx| {
            let instance = N::unwrap_mut(widgets).expect("a binding patches its own host");
            patch(&mut HostPatcher::new(instance, id, columns, cx))
        });
    }

    /// Writes `value` as attribute `property` of host `node`, of `tag`'s
    /// type: alone when the property can be, by building the host again
    /// otherwise.
    #[doc(hidden)]
    pub fn __attr<N, P>(
        &mut self,
        node: u16,
        tag: fn() -> Draft<N>,
        property: impl FnOnce(&N::Properties) -> PropertyMarker<P>,
        value: impl IntoProp<P>,
    ) where
        N: WidgetVariant,
        P: Property,
        P::Scope: PatchScope<N>,
    {
        if <P::Scope as PatchScope<N>>::PATCHABLE {
            let marker = property(&N::Properties::default());
            let value = value.into_prop();
            self.__patch(node, tag, |patcher| patcher.__patch(marker, value));
        } else {
            self.__rebuild(node);
        }
    }

    /// Builds host `node` again from its current values: for an attribute
    /// that cannot be written alone (`style`).
    #[doc(hidden)]
    pub fn __rebuild(&mut self, node: u16) {
        let notifier = self.notifier;
        let host = self.host();
        let id = host.hosts[usize::from(node)].expect("a binding writes a mounted host");
        let draft = host.instance.clone().build_host(node, notifier);
        host.ui.update_node(id, draft.0);
    }
}

/// How many effects of each lane one flush ran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlushReport {
    pub memos: usize,
    pub blocks: usize,
    pub renders: usize,
    pub effects: usize,
}

impl FlushReport {
    fn total(&self) -> usize {
        self.memos + self.blocks + self.renders + self.effects
    }
}

/// A flush running more effects than this is taken to be a cycle through effects.
const MAX_RUNS: usize = 100_000;

/// Where a mounted scope stands.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Owner {
    /// A root, under a host of its own.
    Root(NodeId),
    /// A child component, at template node `node` of `instance`.
    Child { instance: InstanceId, node: u16 },
    /// A row of the block at template node `node` of `instance`.
    Row { instance: InstanceId, node: u16 },
    /// A portal's content, of `instance`, under the overlayer `layer`.
    Portal { instance: InstanceId, layer: NodeId },
}

/// Where a pending mount goes. Opaque: only the runtime makes one.
#[derive(Clone, Copy, Debug)]
pub struct Place(pub(crate) Owner);

/// A scope's template, wherever it lives: a component's `const`, or a row's
/// `Rc` shared by the rows of one block.
pub(crate) struct Parts<'a, I> {
    pub(crate) nodes: &'a [StaticNode<I>],
    pub(crate) effects: &'a [EffectSpec<I>],
    pub(crate) children: &'a [ChildSpec<I>],
}

pub(crate) trait HasParts: Scope + Sized {
    fn parts(&self) -> Parts<'_, Self>;
}

impl<C: ComponentDef> HasParts for C {
    fn parts(&self) -> Parts<'_, Self> {
        let t = &C::TEMPLATE;
        Parts { nodes: t.nodes, effects: t.effects, children: t.children }
    }
}

impl<P: Scope, D: 'static> HasParts for Row<P, D> {
    fn parts(&self) -> Parts<'_, Self> {
        let t: &RowParts<Self> = &self.template;
        Parts { nodes: &t.nodes, effects: &t.effects, children: &t.children }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeTag {
    Host,
    Component(u16),
    /// A block or a slot: rows, kept in the scope's block states.
    Block,
    /// A portal: a row too, whose hosts are under the overlayer, outside
    /// the flow of the scope's other hosts.
    Portal,
}

/// The type-erased face of a mounted scope.
pub(crate) trait AnyInstance {
    fn node_count(&self) -> usize;
    fn node(&self, index: u16) -> (NodeTag, Option<u16>);
    fn build_host(self: Rc<Self>, index: u16, notifier: &Notifier) -> HostDraft;
    fn effect_count(&self) -> usize;
    fn lane(&self, slot: u16) -> Lane;
    fn deps(&self, slot: u16) -> &'static [u16];
    fn output(&self, slot: u16) -> Option<u16>;
    fn block_node(&self, slot: u16) -> u16;
    fn source(&self, index: u16) -> SourceId;
    fn run(self: Rc<Self>, slot: u16, cx: &mut Cx<'_>);
    fn run_block(self: Rc<Self>, slot: u16, cx: &mut BlockCx<'_>);
    fn tick(self: Rc<Self>, slot: u16, cx: &mut Cx<'_>, frame: crate::clock::FrameTime) -> crate::ticker::Tick;
    fn prepare_children(self: Rc<Self>) -> Vec<Option<PendingMount>>;
    /// What slot `index` mounts; `None` for a block.
    fn slot(self: Rc<Self>, index: u16) -> Option<super::Children>;
    fn portal(self: Rc<Self>, index: u16, notifier: &Notifier) -> super::PortalSpec;
    fn as_any(&self) -> &dyn Any;
}

impl<T: HasParts> AnyInstance for T {
    fn node_count(&self) -> usize {
        self.parts().nodes.len()
    }
    fn node(&self, index: u16) -> (NodeTag, Option<u16>) {
        let node = &self.parts().nodes[usize::from(index)];
        let tag = match node.kind {
            NodeKind::Host(_) => NodeTag::Host,
            NodeKind::Component(child) => NodeTag::Component(child),
            NodeKind::Block | NodeKind::Outlet(_) => NodeTag::Block,
            NodeKind::Portal(_) => NodeTag::Portal,
        };
        (tag, node.parent)
    }
    fn build_host(self: Rc<Self>, index: u16, notifier: &Notifier) -> HostDraft {
        match self.parts().nodes[usize::from(index)].kind {
            NodeKind::Host(build) => build(&self, notifier),
            _ => unreachable!("not a host"),
        }
    }
    fn effect_count(&self) -> usize {
        self.parts().effects.len()
    }
    fn lane(&self, slot: u16) -> Lane {
        self.parts().effects[usize::from(slot)].lane
    }
    fn deps(&self, slot: u16) -> &'static [u16] {
        self.parts().effects[usize::from(slot)].deps
    }
    fn output(&self, slot: u16) -> Option<u16> {
        self.parts().effects[usize::from(slot)].output
    }
    fn block_node(&self, slot: u16) -> u16 {
        match self.parts().effects[usize::from(slot)].run {
            Run::Block(node, _) => node,
            Run::Cx(_) | Run::Tick(_) => unreachable!("not a block"),
        }
    }
    fn source(&self, index: u16) -> SourceId {
        Scope::source(self, index)
    }
    fn run(self: Rc<Self>, slot: u16, cx: &mut Cx<'_>) {
        match self.parts().effects[usize::from(slot)].run {
            Run::Cx(run) => run(&self, cx),
            Run::Block(..) | Run::Tick(_) => unreachable!("not run by a flush"),
        }
    }
    fn run_block(self: Rc<Self>, slot: u16, cx: &mut BlockCx<'_>) {
        match self.parts().effects[usize::from(slot)].run {
            Run::Block(_, run) => run(&self, cx),
            Run::Cx(_) | Run::Tick(_) => unreachable!("not a block"),
        }
    }
    fn tick(self: Rc<Self>, slot: u16, cx: &mut Cx<'_>, frame: crate::clock::FrameTime) -> crate::ticker::Tick {
        match self.parts().effects[usize::from(slot)].run {
            Run::Tick(tick) => tick(&self, cx, frame),
            _ => unreachable!("not a ticker"),
        }
    }
    fn prepare_children(self: Rc<Self>) -> Vec<Option<PendingMount>> {
        self.parts().children.iter().map(|child| Some((child.prepare)(&self))).collect()
    }
    fn slot(self: Rc<Self>, index: u16) -> Option<super::Children> {
        match self.parts().nodes[usize::from(index)].kind {
            NodeKind::Outlet(children) => Some(children(&self)),
            _ => None,
        }
    }
    fn portal(self: Rc<Self>, index: u16, notifier: &Notifier) -> super::PortalSpec {
        match self.parts().nodes[usize::from(index)].kind {
            NodeKind::Portal(spec) => spec(&self, notifier),
            _ => unreachable!("not a portal"),
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A mounted portal: its overlay entry and that entry's visual root.
struct MountedPortal {
    node: u16,
    entry: crate::widgets::OverlayEntryId,
    visual_root: NodeId,
}

struct Mounted {
    instance: Rc<dyn AnyInstance>,
    owner: Owner,
    /// The host each template node is; `None` for components and blocks.
    hosts: Vec<Option<NodeId>>,
    /// The child component each template node is.
    children: Vec<Option<InstanceId>>,
    scheduled: Vec<bool>,
    /// Block node → what its rows are.
    blocks: Vec<(u16, BlockState)>,
    /// Sources computed from the parent's, for `#[source]` props given an
    /// expression; effect slots after the template's.
    derived: Vec<Derived>,
    portals: Vec<MountedPortal>,
    /// What it provides its descendants as context.
    provided: Vec<Context>,
    /// Its tickers; dropping them unregisters them.
    ticks: Vec<crate::ticker::TickerHandle>,
    owned: Vec<SourceId>,
    /// Mount order: a scope is always mounted after the one it is in, so
    /// outer blocks run before inner ones.
    seq: u64,
}

type AppContexts = std::collections::HashMap<std::any::TypeId, (Box<dyn Any>, Box<dyn Any>)>;

/// The contexts a scope mounted at `owner` sees, looked up on demand: its
/// ancestors', nearest first, then the app's.
pub(crate) struct Contexts<'a> {
    instances: &'a SlotMap<InstanceId, Mounted>,
    app: &'a AppContexts,
    owner: Owner,
}

impl Contexts<'_> {
    /// The `ReadSignal` provided under `ty`.
    pub(crate) fn find(&self, ty: std::any::TypeId) -> Option<&dyn Any> {
        let mut owner = self.owner;
        while let Owner::Child { instance, .. } | Owner::Row { instance, .. } | Owner::Portal { instance, .. } = owner {
            let mounted = &self.instances[instance];
            if let Some((_, source)) = mounted.provided.iter().find(|(t, _)| *t == ty) {
                return Some(&**source);
            }
            owner = mounted.owner;
        }
        self.app.get(&ty).map(|(read, _)| &**read)
    }
}

#[derive(Default)]
pub struct Runtime {
    graph: Graph,
    notifier: Notifier,
    instances: SlotMap<InstanceId, Mounted>,
    next_seq: u64,
    memos: BinaryHeap<Reverse<(u32, EffectId)>>,
    blocks: BinaryHeap<Reverse<(u64, EffectId)>>,
    renders: Vec<EffectId>,
    effects: VecDeque<EffectId>,
    tasks: super::tasks::Tasks,
    /// The app's contexts: under each type id, the `ReadSignal<T>` scopes
    /// read and the `Signal<T>` the app writes.
    contexts: AppContexts,
}

impl Runtime {
    pub fn new() -> Self {
        Self::default()
    }

    /// A signal owned by the runtime rather than a component, e.g. app state
    /// handed to components as a prop.
    pub fn signal<T>(&mut self, value: T) -> Signal<T> {
        Signal::new(self.graph.insert(), value)
    }

    /// Runs `f`, which may write signals, as a handler would, and flushes.
    pub fn batch(&mut self, ui: &mut UiRuntime, f: impl FnOnce(&mut Cx<'_>)) -> FlushReport {
        f(&mut Cx::handler(&self.notifier));
        self.flush(ui)
    }

    /// What every handler writes through.
    pub(crate) fn notifier(&self) -> &Notifier {
        &self.notifier
    }

    pub(crate) fn tasks(&self) -> &super::tasks::Tasks {
        &self.tasks
    }

    /// Where `resource!`'s futures run.
    pub(crate) fn tasks_mut(&mut self) -> &mut super::tasks::Tasks {
        &mut self.tasks
    }

    /// Writes the result a finished `resource!` future sent, if it is
    /// still wanted.
    pub(crate) fn finish_task(&mut self, (ticket, result): super::tasks::Finished) {
        if let Some(apply) = self.tasks.finish(ticket) {
            apply(result, &mut Cx::handler(&self.notifier));
        }
    }

    /// Whether a write is waiting for [`flush`](Self::flush).
    pub fn is_dirty(&self) -> bool {
        !self.notifier.is_empty() || !self.graph.pending.is_empty()
    }

    /// Mounts component `C` under `parent` and runs its first effects.
    pub fn mount<C: Component>(
        &mut self,
        ui: &mut UiRuntime,
        parent: NodeId,
        props: <C::Instance as ComponentDef>::Props,
    ) -> InstanceId {
        let id = self.mount_into::<C::Instance>(ui, props, Place(Owner::Root(parent)));
        self.flush(ui);
        id
    }

    #[doc(hidden)]
    pub fn mount_into<C: ComponentDef>(&mut self, ui: &mut UiRuntime, props: C::Props, place: Place) -> InstanceId {
        self.mount_with::<C>(ui, |_| props, place)
    }

    /// Mounts `C` with the props `props` resolves, which may derive sources
    /// of its own from its parent's.
    pub(crate) fn mount_with<C: ComponentDef>(
        &mut self,
        ui: &mut UiRuntime,
        props: impl FnOnce(&mut PropCx<'_>) -> C::Props,
        place: Place,
    ) -> InstanceId {
        let mut prop_cx = PropCx { graph: &mut self.graph, owned: Vec::new(), derived: Vec::new() };
        let props = props(&mut prop_cx);
        let (mut owned, derived) = (prop_cx.owned, prop_cx.derived);
        let mut mount_cx = MountCx {
            graph: &mut self.graph,
            owned: Vec::new(),
            contexts: Contexts { instances: &self.instances, app: &self.contexts, owner: place.0 },
            provided: Vec::new(),
            tickers: ui.tickers(),
            ticks: Vec::new(),
        };
        let instance = C::create(props, &mut mount_cx);
        owned.extend(mount_cx.owned);
        let (provided, ticks) = (mount_cx.provided, mount_cx.ticks);
        let instance: Rc<dyn AnyInstance> = Rc::new(instance);
        // Each `ticker!` gets its callback: the instance's, given a `Cx`.
        let tick_slots = (0..instance.effect_count() as u16).filter(|&slot| instance.lane(slot) == Lane::Tick);
        for (handle, slot) in ticks.iter().zip(tick_slots) {
            let (instance, notifier) = (instance.clone(), self.notifier.clone());
            handle.set_callback(Box::new(move |frame| {
                instance.clone().tick(slot, &mut Cx::handler(&notifier), frame)
            }));
        }
        let id = self.mount_instance(ui, instance, owned, derived, provided, place.0);
        self.instances[id].ticks = ticks;
        id
    }

    /// Sets the app's context of type `T`, which scopes without an ancestor
    /// providing one read; a scope reading it reruns when it changes.
    pub fn provide_context<T: PartialEq + 'static>(&mut self, value: T) {
        let id = std::any::TypeId::of::<T>();
        if let Some((_, write)) = self.contexts.get(&id) {
            let signal = write.downcast_ref::<Signal<T>>().expect("context of another type");
            if *signal.read() != value {
                signal.set(&mut Cx::handler(&self.notifier), value);
            }
            return;
        }
        let signal = self.signal(value);
        self.contexts.insert(id, (Box::new(signal.read_only()), Box::new(signal)));
    }

    pub(crate) fn mount_row<P: Scope, D: 'static>(
        &mut self,
        ui: &mut UiRuntime,
        parent: Rc<P>,
        data: D,
        template: Rc<RowParts<Row<P, D>>>,
        owner: Owner,
    ) -> InstanceId {
        let source = self.graph.insert();
        let row = Row { parent, data: Signal::new(source, data), template };
        self.mount_instance(ui, Rc::new(row), vec![source], Vec::new(), Vec::new(), owner)
    }

    /// Binds the scope's effects to its sources, builds its hosts (mounting
    /// child components in place), writes its render bindings and runs its
    /// blocks. Its `effect!`s wait for the next flush.
    fn mount_instance(
        &mut self,
        ui: &mut UiRuntime,
        instance: Rc<dyn AnyInstance>,
        owned: Vec<SourceId>,
        derived: Vec<Derived>,
        provided: Vec<Context>,
        owner: Owner,
    ) -> InstanceId {
        let mut pending = instance.clone().prepare_children();
        let effects = instance.effect_count();
        let count = instance.node_count();
        self.next_seq += 1;
        let id = self.instances.insert(Mounted {
            instance: instance.clone(),
            owner,
            hosts: vec![None; count],
            children: vec![None; count],
            scheduled: vec![false; effects + derived.len()],
            blocks: Vec::new(),
            derived,
            portals: Vec::new(),
            provided,
            ticks: Vec::new(),
            owned,
            seq: self.next_seq,
        });

        // Binding: the template's static dependency lists, resolved to this
        // scope's sources. Done before children mount so a child bound to
        // one of this scope's memos sees that memo's level.
        for slot in 0..effects as u16 {
            let effect = EffectId { instance: id, slot };
            let mut level = 0;
            for &dep in instance.deps(slot) {
                level = level.max(self.graph.subscribe(instance.source(dep), effect));
            }
            if let Some(output) = instance.output(slot) {
                self.graph.set_level(instance.source(output), level + 1);
            }
        }
        for (i, derived) in self.instances[id].derived.iter().enumerate() {
            let effect = EffectId { instance: id, slot: (effects + i) as u16 };
            let mut level = 0;
            for &dep in &derived.deps {
                level = level.max(self.graph.subscribe(dep, effect));
            }
            self.graph.set_level(derived.output, level + 1);
        }

        for index in 0..count as u16 {
            match instance.node(index).0 {
                NodeTag::Host => {
                    let draft = instance.clone().build_host(index, &self.notifier);
                    let host = ui.create_node(draft.0);
                    let (parent, before) = (self.parent_host(id, index), self.host_after(id, index));
                    ui.place(parent, host, before);
                    self.instances[id].hosts[usize::from(index)] = Some(host);
                }
                NodeTag::Component(child) => {
                    let mount = pending[usize::from(child)].take().expect("child mounted twice");
                    let child = mount(self, ui, Place(Owner::Child { instance: id, node: index }));
                    self.instances[id].children[usize::from(index)] = Some(child);
                }
                NodeTag::Block => {
                    // A slot mounts its content now, in order; a block when it runs.
                    if let Some(children) = instance.clone().slot(index) {
                        let row = children.mount(self, ui, Owner::Row { instance: id, node: index });
                        let mut state = BlockState::default();
                        state.rows.extend(row);
                        self.put_block(id, index, state);
                    }
                }
                NodeTag::Portal => self.mount_portal(ui, id, index),
            }
        }

        for slot in 0..effects as u16 {
            let effect = EffectId { instance: id, slot };
            match instance.lane(slot) {
                Lane::Memo => {}
                Lane::Block | Lane::Render => self.run(ui, effect),
                Lane::Effect => self.graph.pending.push(effect),
                Lane::Tick => {}
            }
        }
        id
    }

    /// Mounts portal `node`'s content under the overlayer and makes its first
    /// host an overlay entry.
    fn mount_portal(&mut self, ui: &mut UiRuntime, id: InstanceId, node: u16) {
        let spec = self.instances[id].instance.clone().portal(node, &self.notifier);
        let layer = ui.root_overlayer();
        let row = spec.content.mount(self, ui, Owner::Portal { instance: id, layer });
        let mut state = BlockState::default();
        state.rows.extend(row);
        self.put_block(id, node, state);
        let Some(visual_root) = row.and_then(|row| self.first_top_host(row)) else { return };
        let entry = ui
            .mount_overlay_entry(visual_root, spec.scope, spec.options)
            .expect("portal entry could not be mounted");
        ui.set_overlay_entry_dismiss(entry, spec.on_dismiss)
            .expect("portal entry disappeared while recording its dismiss handler");
        // Directly under the root there is no host to follow.
        let owner = self.parent_host(id, node);
        let target = (owner != ui.root() && owner != layer).then_some(owner);
        ui.set_anchor(visual_root, spec.anchor.map(|placement| (target, placement)));
        self.instances[id].portals.push(MountedPortal { node, entry, visual_root });
    }

    /// The host a node's hosts go under.
    fn parent_host(&self, id: InstanceId, node: u16) -> NodeId {
        let mounted = &self.instances[id];
        if let Some(parent) = mounted.instance.node(node).1 {
            return mounted.hosts[usize::from(parent)].expect("a parent is mounted before its children");
        }
        match mounted.owner {
            Owner::Root(host) => host,
            Owner::Portal { layer, .. } => layer,
            Owner::Child { instance, node } | Owner::Row { instance, node } => self.parent_host(instance, node),
        }
    }

    /// The first host after node `node`'s hosts under their parent host:
    /// what a node's hosts are placed before. `None` places them last.
    fn host_after(&self, id: InstanceId, node: u16) -> Option<NodeId> {
        let mounted = &self.instances[id];
        let parent = mounted.instance.node(node).1;
        for next in node + 1..mounted.instance.node_count() as u16 {
            if mounted.instance.node(next).1 == parent
                && let Some(host) = self.first_host(id, next)
            {
                return Some(host);
            }
        }
        if parent.is_some() {
            return None;
        }
        match mounted.owner {
            Owner::Root(_) | Owner::Portal { .. } => None,
            Owner::Child { instance, node } => self.host_after(instance, node),
            Owner::Row { instance, node } => {
                let rows = self.block(instance, node).map_or(&[][..], |b| &b.rows[..]);
                let at = rows.iter().position(|row| *row == id);
                let later = at.map_or(&[][..], |at| &rows[at + 1..]);
                later.iter().find_map(|row| self.first_top_host(*row)).or_else(|| self.host_after(instance, node))
            }
        }
    }

    fn block(&self, id: InstanceId, node: u16) -> Option<&BlockState> {
        self.instances[id].blocks.iter().find(|(n, _)| *n == node).map(|(_, b)| b)
    }

    /// The first host node `node` stands for, if it has any yet.
    fn first_host(&self, id: InstanceId, node: u16) -> Option<NodeId> {
        let mounted = &self.instances[id];
        match mounted.instance.node(node).0 {
            NodeTag::Host => mounted.hosts[usize::from(node)],
            NodeTag::Component(_) => mounted.children[usize::from(node)].and_then(|child| self.first_top_host(child)),
            NodeTag::Block => self.block(id, node)?.rows.iter().find_map(|row| self.first_top_host(*row)),
            NodeTag::Portal => None,
        }
    }

    fn first_top_host(&self, id: InstanceId) -> Option<NodeId> {
        let instance = &self.instances[id].instance;
        (0..instance.node_count() as u16)
            .filter(|&node| instance.node(node).1.is_none())
            .find_map(|node| self.first_host(id, node))
    }

    /// The hosts directly under the scope's parent host, in order.
    fn top_hosts(&self, id: InstanceId, out: &mut Vec<NodeId>) {
        let mounted = &self.instances[id];
        for node in 0..mounted.instance.node_count() as u16 {
            let (tag, parent) = mounted.instance.node(node);
            if parent.is_some() {
                continue;
            }
            match tag {
                NodeTag::Host => out.extend(mounted.hosts[usize::from(node)]),
                NodeTag::Component(_) => {
                    if let Some(child) = mounted.children[usize::from(node)] {
                        self.top_hosts(child, out);
                    }
                }
                NodeTag::Block => {
                    for &row in self.block(id, node).map_or(&[][..], |b| &b.rows[..]) {
                        self.top_hosts(row, out);
                    }
                }
                NodeTag::Portal => {}
            }
        }
    }

    /// Unmounts `id` and everything mounted in it, dropping their
    /// subscriptions, sources and hosts.
    pub fn unmount(&mut self, ui: &mut UiRuntime, id: InstanceId) {
        if !self.instances.contains_key(id) {
            return;
        }
        let mut hosts = Vec::new();
        self.top_hosts(id, &mut hosts);
        let mut portals = Vec::new();
        self.portals_within(id, &mut portals);
        self.release(id);
        for host in hosts {
            ui.remove_subtree(host);
        }
        for (portal, hosts) in portals {
            // Taking the entry off the overlayer detaches its root, not removes it.
            ui.unmount_overlay_entry(portal.entry).expect("portal entry missing");
            ui.set_anchor(portal.visual_root, None);
            for host in hosts {
                ui.remove_subtree(host);
            }
        }
    }

    /// The portals of `id` and of everything mounted in it, each with its
    /// content's hosts, which are not under any of their hosts.
    fn portals_within(&self, id: InstanceId, out: &mut Vec<(MountedPortal, Vec<NodeId>)>) {
        let mounted = &self.instances[id];
        for portal in &mounted.portals {
            let mut hosts = Vec::new();
            for &row in self.block(id, portal.node).map_or(&[][..], |b| &b.rows[..]) {
                self.top_hosts(row, &mut hosts);
            }
            let MountedPortal { node, entry, visual_root } = *portal;
            out.push((MountedPortal { node, entry, visual_root }, hosts));
        }
        let rows = mounted.blocks.iter().flat_map(|(_, block)| block.rows.iter());
        for &inner in mounted.children.iter().flatten().chain(rows) {
            self.portals_within(inner, out);
        }
    }

    fn release(&mut self, id: InstanceId) {
        let Some(mounted) = self.instances.remove(id) else { return };
        // Inner scopes first: they may be bound to this one's sources.
        let rows = mounted.blocks.iter().flat_map(|(_, block)| block.rows.iter());
        for &inner in mounted.children.iter().flatten().chain(rows) {
            self.release(inner);
        }
        let instance = &mounted.instance;
        let effects = instance.effect_count();
        for slot in 0..effects as u16 {
            for &dep in instance.deps(slot) {
                self.graph.unsubscribe(instance.source(dep), EffectId { instance: id, slot });
            }
        }
        for (i, derived) in mounted.derived.iter().enumerate() {
            for &dep in &derived.deps {
                self.graph.unsubscribe(dep, EffectId { instance: id, slot: (effects + i) as u16 });
            }
        }
        for source in mounted.owned {
            self.graph.remove(source);
        }
    }

    pub(crate) fn take_block(&mut self, owner: InstanceId, node: u16) -> BlockState {
        let blocks = &mut self.instances[owner].blocks;
        blocks.iter().position(|(n, _)| *n == node).map(|at| blocks.swap_remove(at).1).unwrap_or_default()
    }

    pub(crate) fn put_block(&mut self, owner: InstanceId, node: u16, state: BlockState) {
        self.instances[owner].blocks.push((node, state));
    }

    /// Writes a kept row's data; its bindings rerun only if it changed.
    pub(crate) fn replace_row_data<P: Scope, D: PartialEq + 'static>(&mut self, row: InstanceId, data: D) {
        let row = self.instances[row].instance.as_any().downcast_ref::<Row<P, D>>().expect("row of another type");
        row.data.replace_if_changed(&mut self.graph, data);
    }

    /// Places the rows' hosts in the rows' order, where the block stands.
    /// Last first, each before the one after it: a host already there
    /// stays, so rows in order move nothing.
    pub(crate) fn order_rows(&mut self, ui: &mut UiRuntime, owner: InstanceId, node: u16, rows: &[InstanceId]) {
        let (parent, mut before) = (self.parent_host(owner, node), self.host_after(owner, node));
        let mut hosts = Vec::new();
        for &row in rows {
            self.top_hosts(row, &mut hosts);
        }
        for host in hosts.into_iter().rev() {
            ui.place(parent, host, before);
            before = Some(host);
        }
    }

    /// Settles every pending write: memos by level, then blocks outermost
    /// first, then render bindings, then effects; an effect's writes start
    /// the next round.
    pub fn flush(&mut self, ui: &mut UiRuntime) -> FlushReport {
        let mut report = FlushReport::default();
        loop {
            self.schedule_pending();
            let (effect, count) = if let Some(Reverse((_, effect))) = self.memos.pop() {
                (effect, &mut report.memos)
            } else if let Some(Reverse((_, effect))) = self.blocks.pop() {
                (effect, &mut report.blocks)
            } else if let Some(effect) = self.renders.pop() {
                (effect, &mut report.renders)
            } else if let Some(effect) = self.effects.pop_front() {
                (effect, &mut report.effects)
            } else {
                return report;
            };
            *count += 1;
            assert!(report.total() < MAX_RUNS, "flush did not settle after {MAX_RUNS} runs; an effect probably feeds itself");
            self.run(ui, effect);
        }
    }

    fn schedule_pending(&mut self) {
        for spawn in self.notifier.take_spawns() {
            self.tasks.start(spawn);
        }
        self.graph.drain(&self.notifier);
        for effect in std::mem::take(&mut self.graph.pending) {
            // Ids from unmounted scopes are dropped here, once.
            let Some(mounted) = self.instances.get_mut(effect.instance) else { continue };
            let scheduled = &mut mounted.scheduled[usize::from(effect.slot)];
            if *scheduled {
                continue;
            }
            *scheduled = true;
            let instance = &mounted.instance;
            let effects = instance.effect_count();
            if let Some(derived) = usize::from(effect.slot).checked_sub(effects).map(|i| &mounted.derived[i]) {
                self.memos.push(Reverse((self.graph.level(derived.output), effect)));
                continue;
            }
            match instance.lane(effect.slot) {
                Lane::Memo => {
                    let output = instance.output(effect.slot).expect("memo without output");
                    self.memos.push(Reverse((self.graph.level(instance.source(output)), effect)));
                }
                Lane::Block => self.blocks.push(Reverse((mounted.seq, effect))),
                Lane::Render => self.renders.push(effect),
                Lane::Effect => self.effects.push_back(effect),
                Lane::Tick => unreachable!("a ticker has no dependencies"),
            }
        }
    }

    fn run(&mut self, ui: &mut UiRuntime, effect: EffectId) {
        // A block may have unmounted this scope since it was scheduled.
        let Some(mounted) = self.instances.get_mut(effect.instance) else { return };
        mounted.scheduled[usize::from(effect.slot)] = false;
        let instance = mounted.instance.clone();
        if let Some(i) = usize::from(effect.slot).checked_sub(instance.effect_count()) {
            (mounted.derived[i].run)(&mut Cx::handler(&self.notifier));
            return;
        }
        match instance.lane(effect.slot) {
            Lane::Block => {
                let node = instance.block_node(effect.slot);
                let mut cx = BlockCx { runtime: self, ui, owner: effect.instance, node };
                instance.run_block(effect.slot, &mut cx);
            }
            Lane::Effect => instance.run(effect.slot, &mut Cx::handler(&self.notifier)),
            Lane::Tick => unreachable!("a ticker is run by the frame loop"),
            Lane::Memo | Lane::Render => {
                let Self { instances, notifier, .. } = self;
                let hosts = &instances[effect.instance].hosts;
                let mut cx = Cx { notifier, host: Some(HostCx { ui, hosts, instance: &instance }) };
                instance.clone().run(effect.slot, &mut cx);
            }
        }
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// A mounted component's instance, for tests and tools.
    pub fn instance<C: Component>(&self, id: InstanceId) -> Option<&C::Instance> {
        self.instances.get(id)?.instance.as_any().downcast_ref()
    }

    /// The hosts a mounted scope puts under its parent host, in order.
    pub fn hosts(&self, id: InstanceId) -> Vec<NodeId> {
        let mut out = Vec::new();
        if self.instances.contains_key(id) {
            self.top_hosts(id, &mut out);
        }
        out
    }
}
