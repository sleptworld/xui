use crate::context::{Ancestors, ContextRegistry, ContextScope};
use crate::element::PortalBehavior;
use crate::fiber::{
    ComponentRender, ComponentState, ComponentType, EffectTag, ErasedProps, FiberArena, FiberId,
    FiberKind, FiberTag, Key, Node, PortalState, TemplateState,
};
use crate::lanes::{
    Lanes, NO_LANES, current_update_lane, get_highest_priority_lane, includes_some_lane,
    should_interrupt,
};
use crate::state::{AsyncDispatcher, HookContext, HookStorage, Scheduler};
use crate::template::{
    BuildMode, TemplateDesc, TemplateIdentity, TemplateMeta, TemplateNodeKind, TemplateSlot,
};
use crate::ticker::TickerRegistry;
use crate::ui_runtime::UiRuntime;
use crate::widgets::{ErasedDraft, RootComponentRender};
use crate::widgets::{OverlayEntryId, OverlayEntryOptions, OverlayScopeId};
use crate::{ElementDesc, ErasedPropsRef};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::ops::RangeBounds;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::runtime::Handle as TokioHandle;
use xui_interface::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WipId(usize);
pub struct WorkNode {
    fiber_id: FiberId,
    parent: Option<WipId>,
    key: Option<Key>,
    tag: FiberTag,
    position: usize,
    effect: EffectTag,
    children_resolved: bool,
    children: SmallVec<[WipId; 8]>,
    lanes: Lanes,
    child_lanes: Lanes,
    /// What this render brought the fiber; `None` for one cloned as it was.
    work: Option<Pending>,
    binding: WorkBinding,
}

/// What a render brings a fiber, by kind.
enum Pending {
    Component(ComponentWork),
    Template(TemplateWork),
    Fragment(Vec<ElementDesc>),
    Portal(PortalWork),
}

struct ComponentWork {
    render: ComponentRender,
    key: Option<Key>,
    props: Option<ErasedProps>,
}

struct TemplateWork {
    meta: &'static TemplateMeta,
    identity: TemplateIdentity,
    /// The render's inputs, until `begin_work` builds them.
    desc: Option<TemplateDesc>,
    /// What the build is for: mounting every host, or updating the dynamic
    /// ones of a mounted template.
    mode: BuildMode,
    /// The drafts the commit mounts or updates, by node index.
    hosts: Vec<Option<Box<dyn ErasedDraft>>>,
    /// The hosts once mounted, by node index.
    nodes: Vec<Option<NodeId>>,
}

struct PortalWork {
    scope: Option<OverlayScopeId>,
    options: OverlayEntryOptions,
    behavior: PortalBehavior,
    pending_children: Vec<ElementDesc>,
    entry: Option<crate::widgets::OverlayEntryId>,
    visual_root: Option<NodeId>,
}

#[derive(Default)]
struct WorkBinding {
    /// The committed fiber this one is the next version of.
    current: Option<FiberId>,
    /// The host this fiber is, once known: a template's root.
    current_host: Option<NodeId>,
}

impl From<ComponentWork> for ComponentState {
    fn from(val: ComponentWork) -> Self {
        ComponentState {
            key: val.key,
            render: val.render,
            props: val.props,
        }
    }
}

impl WorkNode {
    fn template_work(&self) -> Option<&TemplateWork> {
        match self.work.as_ref() {
            Some(Pending::Template(work)) => Some(work),
            _ => None,
        }
    }

    fn template_work_mut(&mut self) -> Option<&mut TemplateWork> {
        match self.work.as_mut() {
            Some(Pending::Template(work)) => Some(work),
            _ => None,
        }
    }

    fn portal_work(&self) -> Option<&PortalWork> {
        match self.work.as_ref() {
            Some(Pending::Portal(work)) => Some(work),
            _ => None,
        }
    }

    fn portal_work_mut(&mut self) -> Option<&mut PortalWork> {
        match self.work.as_mut() {
            Some(Pending::Portal(work)) => Some(work),
            _ => None,
        }
    }

    fn from_current(
        current: &Node,
        parent: Option<WipId>,
        position: usize,
        lanes: Lanes,
        child_lanes: Lanes,
    ) -> Self {
        Self {
            fiber_id: current.id,
            parent,
            key: current.key,
            position,
            tag: current.tag(),
            children: SmallVec::new(),
            children_resolved: false,
            effect: EffectTag::empty(),
            lanes,
            child_lanes,
            work: None,
            binding: WorkBinding {
                current: Some(current.id),
                current_host: current.host(),
            },
        }
    }

    fn from_prepared(
        nodes: &FiberArena,
        fiber_id: FiberId,
        parent: WipId,
        position: usize,
        prepared: PreparedElement,
        current: Option<FiberId>,
        effect: EffectTag,
        lanes: Lanes,
        child_lanes: Lanes,
    ) -> Self {
        let current_node = current.and_then(|id| nodes.node(id));
        let mut work = prepared.pending;
        match &mut work {
            Pending::Template(template) => {
                // A template matched to a committed one updates it.
                if let Some(state) = current_node.and_then(Node::template) {
                    template.mode = BuildMode::Update;
                    template.nodes = state.nodes.clone();
                }
            }
            Pending::Portal(portal) => {
                if let Some(state) = current_node.and_then(Node::portal) {
                    portal.entry = state.entry;
                    portal.visual_root = state.visual_root;
                }
            }
            Pending::Component(_) | Pending::Fragment(_) => {}
        }

        Self {
            fiber_id,
            parent: Some(parent),
            key: prepared.key,
            position,
            tag: prepared.tag,
            effect,
            children: SmallVec::new(),
            children_resolved: false,
            lanes,
            child_lanes,
            work: Some(work),
            binding: WorkBinding {
                current,
                current_host: current_node.and_then(Node::host),
            },
        }
    }

    fn needs_begin_work(&self, render_lanes: Lanes) -> bool {
        self.is_uncommited()
            || self.work.is_some()
            || includes_some_lane(self.lanes | self.child_lanes, render_lanes)
    }

    /// Whether this component's function has to run again, as opposed to
    /// cloning the committed subtree.
    ///
    /// A parent re-render always hands its children a freshly built
    /// `ComponentDesc`, so the presence of `ComponentWork` alone says nothing
    /// about whether anything changed. When the incoming props are the *same
    /// allocation* the child is already holding, re-running the function
    /// cannot produce a different tree, so the subtree is reused.
    ///
    /// Only pointer equality is checked, never contents: it is the cheap,
    /// always-correct half. It pays off exactly where a caller deliberately
    /// preserved an element -- forwarded `children`, or an element cached in a
    /// `use_memo` -- which mirrors React's `oldProps === newProps` bailout.
    fn should_render_component(&self, render_lanes: Lanes, fiber_tree: &FiberArena) -> bool {
        if self.is_uncommited() || includes_some_lane(self.lanes, render_lanes) {
            return true;
        }
        let Some(Pending::Component(work)) = &self.work else {
            return false;
        };
        let Some(committed) = self
            .binding
            .current
            .and_then(|id| fiber_tree.node(id))
            .and_then(Node::component)
        else {
            return true;
        };
        if committed.render != work.render || committed.key != work.key {
            return true;
        }
        match (&committed.props, &work.props) {
            (Some(committed), Some(next)) => !Rc::ptr_eq(committed, next),
            (None, None) => false,
            _ => true,
        }
    }

    /// A fragment's or a portal's new children.
    fn take_pending_children(&mut self) -> Option<Vec<ElementDesc>> {
        match &mut self.work {
            Some(Pending::Fragment(children)) => Some(std::mem::take(children)),
            Some(Pending::Portal(portal)) => Some(std::mem::take(&mut portal.pending_children)),
            _ => None,
        }
    }

    #[inline(always)]
    fn is_uncommited(&self) -> bool {
        self.binding.current.is_none()
    }

    fn component_render_props<'a, 'b: 'a>(
        &'a self,
        fiber_tree: &'b FiberArena,
    ) -> Option<(ComponentRender, Option<ErasedPropsRef<'a>>)> {
        match &self.work {
            Some(Pending::Component(c)) => Some((c.render, c.props.as_ref())),
            _ => self
                .binding
                .current
                .and_then(|id| fiber_tree.node(id))
                .and_then(Node::component)
                .map(|component| (component.render, component.props.as_ref())),
        }
    }
}

/// An element, ready to be matched against the fiber in its place.
struct PreparedElement {
    key: Option<Key>,
    tag: FiberTag,
    pending: Pending,
}

pub struct WorkInProgress {
    root: WipId,
    next_work: Option<WipId>,
    render_lanes: Lanes,
    deletions: Vec<FiberId>,
}

impl WorkInProgress {
    fn live<'a>(&'a self, nodes: &'a mut WipArena) -> WorkInProgressLive<'a> {
        WorkInProgressLive { nodes }
    }
}

struct WorkInProgressLive<'a> {
    nodes: &'a mut WipArena,
}

impl<'a> WorkInProgressLive<'a> {
    fn alloc_node(&mut self, node: WorkNode) -> WipId {
        let id = WipId(self.nodes.len());
        self.nodes.push(Some(node));
        id
    }

    fn take_node(&mut self, id: WipId) -> Option<WorkNode> {
        self.nodes.take_node(id)
    }
}

struct WipArena {
    wip_nodes: Vec<Option<WorkNode>>,
}

/// The strict ancestors of a node in the tree being worked on. Every node a
/// render reaches has its whole ancestry in that tree, rendered or cloned.
struct WipAncestors<'a> {
    nodes: &'a WipArena,
    of: WipId,
}

impl Ancestors for WipAncestors<'_> {
    fn find(&self, visit: &mut dyn FnMut(FiberId) -> bool) {
        let mut cursor = self.nodes.get(self.of).and_then(|node| node.parent);
        while let Some(id) = cursor {
            let node = self.nodes.get(id).expect("work node ancestor missing");
            if visit(node.fiber_id) {
                return;
            }
            cursor = node.parent;
        }
    }
}

impl WipArena {
    fn new() -> Self {
        Self {
            wip_nodes: Vec::with_capacity(200),
        }
    }

    fn len(&self) -> usize {
        self.wip_nodes.len()
    }

    fn push(&mut self, node: Option<WorkNode>) {
        self.wip_nodes.push(node);
    }

    fn get(&self, id: WipId) -> Option<&WorkNode> {
        self.wip_nodes.get(id.0).and_then(|n| n.as_ref())
    }

    fn get_mut(&mut self, id: WipId) -> Option<&mut WorkNode> {
        self.wip_nodes.get_mut(id.0).and_then(|n| n.as_mut())
    }

    fn take_node(&mut self, id: WipId) -> Option<WorkNode> {
        self.wip_nodes.get_mut(id.0).and_then(Option::take)
    }

    fn clear(&mut self) {
        self.wip_nodes.clear();
    }

    fn drain<R>(&mut self, range: R) -> std::vec::Drain<'_, Option<WorkNode>>
    where
        R: RangeBounds<usize>,
    {
        self.wip_nodes.drain(range)
    }
}

pub struct ComponentRuntime {
    nodes: FiberArena,
    current: FiberId,
    work_in_progress: Option<WorkInProgress>,
    scheduler: Scheduler,
    async_dispatcher: AsyncDispatcher,
    tokio_handle: Option<TokioHandle>,
    /// Handed to every `HookContext`, so `use_ticker` can install into the
    /// list the runtime walks each frame.
    tickers: TickerRegistry,
    hooks: FxHashMap<FiberId, HookStorage>,
    contexts: ContextRegistry,
    root_widget: NodeId,
    budget: Duration,
    wip_nodes: WipArena,
}

impl ComponentRuntime {
    pub fn new(
        root_widget: NodeId,
        scheduler: Scheduler,
        root_render: fn(&mut HookContext) -> ElementDesc,
    ) -> Self {
        Self::new_with_async(
            root_widget,
            scheduler,
            AsyncDispatcher::noop(),
            None,
            TickerRegistry::default(),
            root_render,
        )
    }

    pub(crate) fn new_with_async(
        root_widget: NodeId,
        scheduler: Scheduler,
        async_dispatcher: AsyncDispatcher,
        tokio_handle: Option<TokioHandle>,
        tickers: TickerRegistry,
        root_render: fn(&mut HookContext) -> ElementDesc,
    ) -> Self {
        let mut arena = FiberArena::new();
        let current = arena.root();
        // The root is a component like any other, rendering `root_render`.
        arena.node_mut(current).expect("the arena has a root").kind =
            FiberKind::Component(ComponentState {
                render: ComponentRender::new(ComponentType::ROOT, render_root),
                key: None,
                props: Some(Rc::new(root_render)),
            });
        scheduler.set_root(current);
        scheduler.mark_component_dirty(current, current_update_lane());

        Self {
            nodes: arena,
            current,
            root_widget,
            work_in_progress: None,
            scheduler,
            async_dispatcher,
            tokio_handle,
            tickers,
            hooks: FxHashMap::default(),
            contexts: ContextRegistry::default(),
            budget: Duration::from_millis(4),
            wip_nodes: WipArena::new(),
        }
    }

    pub fn root(&self) -> FiberId {
        self.current
    }

    pub fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    pub fn root_node(&self) -> &Node {
        self.nodes.node(self.root()).unwrap()
    }

    fn alloc_wip_node(&mut self, node: WorkNode) -> WipId {
        self.work_in_progress
            .as_ref()
            .expect("work missing")
            .live(&mut self.wip_nodes)
            .alloc_node(node)
    }

    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    pub fn is_dirty(&self) -> bool {
        self.work_in_progress.is_some() || self.scheduler.is_dirty()
    }

    pub fn mark_root_dirty(&self) {
        self.scheduler.mark_root_dirty(current_update_lane());
    }

    /// Sets the application-wide context of type `T`, which every component
    /// reads unless an ancestor provides its own. Re-renders the components
    /// that read it, and only those, when it changes.
    pub fn provide_context<T: PartialEq + 'static>(&mut self, value: T) {
        self.contexts.provide_app(value, &self.scheduler);
    }

    /// Clears the application-wide context of type `T`.
    pub fn remove_context<T: 'static>(&mut self) {
        self.contexts.remove_app::<T>(&self.scheduler);
    }

    /// The application-wide context of type `T`, if one is set.
    pub fn context<T: Clone + 'static>(&self) -> Option<T> {
        self.contexts.app_value()
    }

    #[cfg(test)]
    pub(crate) fn context_registry(&self) -> &ContextRegistry {
        &self.contexts
    }

    pub fn rebuild_sync_if_needed(&mut self, arena: &mut UiRuntime) {
        if self.is_dirty() {
            self.flush_sync(arena);
        }
    }

    pub fn rebuild_slice_if_needed(&mut self, arena: &mut UiRuntime) -> bool {
        if !self.is_dirty() {
            return true;
        }

        self.work_loop(arena, Some(Instant::now() + self.budget))
    }

    pub fn flush_sync(&mut self, arena: &mut UiRuntime) {
        self.work_loop(arena, None);
    }

    fn work_loop(&mut self, arena: &mut UiRuntime, deadline: Option<Instant>) -> bool {
        self.scheduler.mark_starved_lanes_as_expired();
        loop {
            if self.scheduler.pending_lanes() == NO_LANES && self.work_in_progress.is_none() {
                return true;
            }

            self.ensure_work();
            if self.work_in_progress.is_none() {
                return false;
            }
            while self
                .work_in_progress
                .as_ref()
                .is_some_and(|work| work.next_work.is_some())
            {
                self.perform_unit_of_work();
                let more_work = self.need_more_work();
                if more_work && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return false;
                }
            }
            self.commit_finished_work(arena);
            if deadline.is_some() {
                return true;
            }
        }
    }

    #[inline]
    fn need_more_work(&self) -> bool {
        self.work_in_progress
            .as_ref()
            .is_some_and(|wip| wip.next_work.is_some())
    }

    fn perform_unit_of_work(&mut self) {
        let Some(id) = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.next_work)
        else {
            return;
        };

        if let Some(child) = self.begin_work(id) {
            self.work_in_progress.as_mut().unwrap().next_work = Some(child);
            return;
        }

        let mut current = id;
        loop {
            self.complete_work(current);
            if let Some(sibling) = self.next_sibling_needing_work(current) {
                if let Some(work) = self.work_in_progress.as_mut() {
                    work.next_work = Some(sibling);
                }
                return;
            }
            let parent = self.wip_nodes.get(current).and_then(|n| n.parent);

            match parent {
                Some(parent) => current = parent,
                None => {
                    if let Some(work) = self.work_in_progress.as_mut() {
                        work.next_work = None;
                    }
                    return;
                }
            }
        }
    }

    fn begin_work(&mut self, id: WipId) -> Option<WipId> {
        let (fiber_id, tag, should_render, render_lanes, committed) = {
            let work = self.work_in_progress.as_ref().expect("work missing");
            let node = self.wip_nodes.get(id).expect("work node missing");

            (
                node.fiber_id,
                node.tag,
                node.should_render_component(work.render_lanes, &self.nodes),
                work.render_lanes,
                !node.is_uncommited(),
            )
        };

        macro_rules! cx {
            ($id: ident, $ancestors: expr) => {{
                self.contexts.begin_render($id);
                let storage = self.hooks.entry($id).or_default();
                let cx = HookContext::new_with_async(
                    storage,
                    $id,
                    self.scheduler.clone(),
                    render_lanes,
                    self.async_dispatcher.clone(),
                    self.tokio_handle.clone(),
                    self.tickers.clone(),
                )
                .with_contexts(ContextScope {
                    registry: &mut self.contexts,
                    ancestors: $ancestors,
                });
                cx
            }};
        }

        // Runs after a component rendered and before its children are
        // reconciled, so that readers of a context it changed are marked
        // before the children's pending work is collected.
        macro_rules! finish_render {
            ($ancestors: expr) => {
                self.contexts.finish_render(
                    fiber_id,
                    committed,
                    $ancestors,
                    &self.nodes,
                    &self.scheduler,
                    get_highest_priority_lane(render_lanes),
                )
            };
        }

        match tag {
            FiberTag::Component => {
                if should_render {
                    let ancestors = WipAncestors {
                        nodes: &self.wip_nodes,
                        of: id,
                    };
                    let wip_node = self.wip_nodes.get(id).unwrap();
                    if let Some((render, props)) = wip_node.component_render_props(&self.nodes) {
                        let mut cx = cx!(fiber_id, &ancestors);
                        let element = (render.call)(&mut cx, props);
                        finish_render!(&ancestors);
                        self.reconcile_children(id, [element]);
                    }
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Fragment | FiberTag::Portal => {
                let pending_children = self
                    .wip_nodes
                    .get_mut(id)
                    .and_then(WorkNode::take_pending_children);
                if let Some(children) = pending_children {
                    self.reconcile_children(id, children);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Template => {
                let build = self
                    .wip_nodes
                    .get_mut(id)
                    .and_then(WorkNode::template_work_mut)
                    .and_then(|work| work.desc.take().map(|desc| (desc, work.mode)))
                    .map(|(desc, mode)| desc.build(mode));
                if let Some(mut build) = build {
                    let slots = std::mem::take(&mut build.slots);
                    let work = self
                        .wip_nodes
                        .get_mut(id)
                        .and_then(WorkNode::template_work_mut)
                        .expect("template work missing");
                    work.hosts = std::mem::take(&mut build.hosts);
                    // Slots match by position: the same template has the
                    // same slots. A kept one stays as it is.
                    let prepared = slots
                        .into_iter()
                        .map(|slot| match slot {
                            TemplateSlot::Element(element) => Some(self.prepare_element(element)),
                            TemplateSlot::Keep => None,
                        })
                        .collect::<Vec<_>>();
                    self.reconcile_prepared(id, prepared);
                } else {
                    self.clone_current_children(id);
                }
            }
        }

        self.first_child_needing_work(id)
    }

    fn reconcile_children<I>(&mut self, parent: WipId, new_children: I)
    where
        I: IntoIterator<Item = ElementDesc>,
    {
        let prepared = new_children
            .into_iter()
            .map(|element| Some(self.prepare_element(element)))
            .collect::<Vec<_>>();
        self.reconcile_prepared(parent, prepared);
    }

    /// Matches `new_children` against the committed children of `parent`.
    /// A `None` keeps the committed child in that position as it is.
    fn reconcile_prepared(&mut self, parent: WipId, new_children: Vec<Option<PreparedElement>>) {
        let work_node = self.wip_nodes.get(parent).unwrap();
        let work_node_current = work_node.binding.current;

        let old_children = work_node_current
            .and_then(|id| self.nodes.node(id))
            .map(|node| node.children(&self.nodes).map(|n| n.id).collect::<Vec<_>>())
            .unwrap_or_default();
        let new_children = new_children.into_iter();
        let render_lanes = self
            .work_in_progress
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(NO_LANES);

        let mut used = vec![false; old_children.len()];
        let mut next_children = SmallVec::with_capacity(20);
        let mut last_placed_index = 0;

        let old_key_map = old_children
            .iter()
            .enumerate()
            .map(|(pos, o)| {
                let node = self.nodes.node(*o).unwrap();
                (node.key, pos)
            })
            .filter(|(a, _)| a.is_some())
            .map(|(k, pos)| (k.unwrap(), pos))
            .collect();

        for (position, prepared) in new_children.into_iter().enumerate() {
            let Some(prepared) = prepared else {
                let old_id = old_children[position];
                used[position] = true;
                let old = self.nodes.node(old_id).expect("a kept fiber is committed");
                let lanes = self.scheduler.component_lanes(old_id) & render_lanes;
                let child_lanes = child_tree_lanes(&self.nodes, old, &self.scheduler, render_lanes);
                let child = self.alloc_wip_node(WorkNode::from_current(
                    old,
                    Some(parent),
                    position,
                    lanes,
                    child_lanes,
                ));
                next_children.push(child);
                last_placed_index = last_placed_index.max(position);
                continue;
            };

            let wip_node = if let Some(matched) = find_reusable_child_fast(
                &self.nodes,
                &old_children,
                &used,
                &old_key_map,
                &prepared,
                position,
            ) {
                let old_index = matched.old_index;
                let old_id = matched.old_id;
                let reused_child_node_id = old_children[old_index];
                let reused_child_node = self.nodes.node(reused_child_node_id).unwrap();
                used[old_index] = true;

                let current_lane = self.scheduler.component_lanes(old_id) & render_lanes;
                let children_lanes = child_tree_lanes(
                    &self.nodes,
                    reused_child_node,
                    &self.scheduler,
                    render_lanes,
                );

                let mut effect = if prepared_needs_update(reused_child_node, &prepared) {
                    EffectTag::UPDATE
                } else {
                    EffectTag::empty()
                };

                if old_index < last_placed_index {
                    effect |= EffectTag::MOVE;
                } else {
                    last_placed_index = old_index;
                }

                WorkNode::from_prepared(
                    &self.nodes,
                    old_id,
                    parent,
                    position,
                    prepared,
                    Some(old_id),
                    effect,
                    current_lane,
                    children_lanes,
                )
            } else {
                let id = self.nodes.new_id();
                WorkNode::from_prepared(
                    &self.nodes,
                    id,
                    parent,
                    position,
                    prepared,
                    None,
                    EffectTag::PLACEMENT,
                    NO_LANES,
                    NO_LANES,
                )
            };

            let child = self.alloc_wip_node(wip_node);
            next_children.push(child);
        }

        for (index, old_child) in old_children.into_iter().enumerate() {
            if !used[index] {
                self.work_in_progress
                    .as_mut()
                    .expect("work missing")
                    .deletions
                    .push(old_child);
            }
        }
        let node = self.wip_nodes.get_mut(parent).unwrap();
        node.children = next_children;
        node.children_resolved = true;
    }

    fn prepare_element(&self, element: ElementDesc) -> PreparedElement {
        let key = element.key();
        let (tag, pending) = match element {
            ElementDesc::Component(component) => (
                FiberTag::Component,
                Pending::Component(ComponentWork {
                    render: component.render,
                    key,
                    props: component.props,
                }),
            ),
            ElementDesc::Template(desc) => (
                FiberTag::Template,
                Pending::Template(TemplateWork {
                    meta: desc.meta,
                    identity: desc.identity,
                    desc: Some(desc),
                    // Made an update in `from_prepared` when it matches a
                    // committed template.
                    mode: BuildMode::Mount,
                    hosts: Vec::new(),
                    nodes: Vec::new(),
                }),
            ),
            ElementDesc::Fragment(fragment) => {
                (FiberTag::Fragment, Pending::Fragment(fragment.children))
            }
            ElementDesc::Portal(portal) => (
                FiberTag::Portal,
                Pending::Portal(PortalWork {
                    scope: portal.scope,
                    options: portal.options,
                    behavior: portal.behavior,
                    pending_children: portal.children,
                    entry: None,
                    visual_root: None,
                }),
            ),
        };
        PreparedElement { key, tag, pending }
    }

    fn commit_finished_work(&mut self, arena: &mut UiRuntime) {
        let Some(mut work) = self.work_in_progress.take() else {
            return;
        };

        if work.next_work.is_some() {
            self.work_in_progress = Some(work);
            return;
        }

        let render_lanes = work.render_lanes;
        let deletions = std::mem::take(&mut work.deletions);
        for deletion in deletions {
            self.commit_deletion(deletion, arena, true);
        }

        self.commit_mutation_effects(work.root, self.root_widget, arena);

        let next_current = self.freeze_work_tree(work.root, None, &mut work, arena);
        self.current = next_current;
        self.scheduler.mark_render_finished(render_lanes);
    }

    fn commit_deletion(&mut self, id: FiberId, arena: &mut UiRuntime, remove_host: bool) {
        let children = self.nodes.children(id);
        let host_node = self.nodes.node(id).and_then(Node::host);
        // A portal starts a separate physical subtree, even when its logical
        // ancestor's host subtree has already been removed.
        let remove_host = remove_host || self.nodes.node(id).and_then(Node::portal).is_some();
        let portal_entry = self
            .nodes
            .node(id)
            .and_then(Node::portal)
            .and_then(|portal| portal.entry);
        self.scheduler.mark_unmounted(id);
        self.hooks.remove(&id);
        self.contexts.remove(id);

        if let Some(entry) = portal_entry {
            let _ = detach_portal(arena, entry);
        }

        if remove_host && let Some(host_node) = host_node {
            arena.remove_subtree(host_node);
            for child in children {
                self.commit_deletion(child, arena, false);
            }
            self.nodes.remove_node(id);
            return;
        }

        for child in children {
            self.commit_deletion(child, arena, remove_host);
        }
        self.nodes.remove_node(id);
    }

    fn commit_mutation_effects(
        &mut self,
        wip_id: WipId,
        parent_host: NodeId,
        arena: &mut UiRuntime,
    ) {
        let Some((effect, tag, children)) = self
            .wip_nodes
            .get(wip_id)
            .map(|node| (node.effect, node.tag, node.children.clone()))
        else {
            return;
        };

        if effect.intersects(EffectTag::PLACEMENT.union(EffectTag::MOVE)) {
            let before = self.find_host_sibling_for_wip(wip_id);
            self.commit_placement_subtree(wip_id, parent_host, before, arena);
        }

        if effect.contains(EffectTag::UPDATE) {
            self.commit_update_if_portal(wip_id, parent_host, arena);
            self.commit_update_if_template(wip_id, arena);
        }

        if matches!(tag, FiberTag::Template) {
            // Each slot sits under the host the template wrote it in.
            let (meta, nodes) = self.template_nodes(wip_id);
            for (child, (_, slot)) in children.into_iter().zip(meta.slots()) {
                let parent = slot.parent.expect("a template's root is a host") as usize;
                let parent_host = nodes[parent].expect("a template's hosts are mounted");
                self.commit_mutation_effects(child, parent_host, arena);
            }
            return;
        }

        let child_parent_host = match tag {
            FiberTag::Portal => arena.root_overlayer(),
            _ => parent_host,
        };

        for child in children {
            self.commit_mutation_effects(child, child_parent_host, arena);
        }

        if matches!(tag, FiberTag::Portal) {
            self.sync_portal_visual_root(wip_id, parent_host, arena);
        }
    }

    /// The host a fiber is: a template's root, once mounted.
    fn host_node_for_wip(&self, wip_id: WipId) -> Option<NodeId> {
        self.wip_nodes.get(wip_id)?.binding.current_host
    }

    fn commit_placement_subtree(
        &mut self,
        wip_id: WipId,
        parent_host: NodeId,
        before: Option<NodeId>,
        arena: &mut UiRuntime,
    ) {
        let Some((tag, children)) = self
            .wip_nodes
            .get(wip_id)
            .map(|node| (node.tag, node.children.clone()))
        else {
            return;
        };

        if matches!(tag, FiberTag::Template) {
            // Its slots are placed under its own hosts as the mutation pass
            // reaches them.
            let root = self.ensure_template_mounted(wip_id, arena);
            arena.place(parent_host, root, before);
            if let Some(node) = self.wip_nodes.get_mut(wip_id) {
                node.effect.remove(EffectTag::PLACEMENT | EffectTag::MOVE);
            }
            return;
        }

        if matches!(tag, FiberTag::Portal) {
            for child in children {
                self.commit_placement_subtree(child, arena.root_overlayer(), None, arena);
            }
            let visual_root = self.first_host_in_wip_subtree(wip_id);
            let (scope, options, behavior, existing_entry) = self
                .wip_nodes
                .get_mut(wip_id)
                .and_then(WorkNode::portal_work_mut)
                .map(|portal| {
                    (
                        portal.scope,
                        portal.options,
                        portal.behavior.clone(),
                        portal.entry,
                    )
                })
                .expect("Portal placement is missing its model");
            let entry = if let Some(entry) = existing_entry {
                arena
                    .update_overlay_entry(entry, scope, options)
                    .expect("Portal entry could not be moved");
                Some(entry)
            } else {
                visual_root.map(|visual_root| {
                    arena
                        .mount_overlay_entry(visual_root, scope, options)
                        .expect("Portal entry could not be mounted")
                })
            };
            if let (Some(entry), Some(visual_root)) = (entry, visual_root) {
                apply_portal_behavior(arena, entry, visual_root, parent_host, &behavior);
            }
            let portal = self
                .wip_nodes
                .get_mut(wip_id)
                .and_then(WorkNode::portal_work_mut)
                .expect("Portal placement model disappeared");
            portal.entry = entry;
            portal.visual_root = visual_root;
            if let Some(node) = self.wip_nodes.get_mut(wip_id) {
                node.effect.remove(EffectTag::PLACEMENT | EffectTag::MOVE);
            }
            return;
        }

        for child in children {
            self.commit_placement_subtree(child, parent_host, before, arena);
        }
    }

    /// Mounts a new template's hosts, each under its parent, and returns its
    /// root, unplaced. A mounted template's root as it is.
    fn ensure_template_mounted(&mut self, wip_id: WipId, arena: &mut UiRuntime) -> NodeId {
        if let Some(root) = self
            .wip_nodes
            .get(wip_id)
            .and_then(|n| n.binding.current_host)
        {
            return root;
        }
        let work = self
            .wip_nodes
            .get_mut(wip_id)
            .and_then(WorkNode::template_work_mut)
            .expect("a new template has its work");
        debug_assert_eq!(work.mode, BuildMode::Mount);
        let meta = work.meta;
        let mut nodes = vec![None; meta.nodes.len()];
        for (index, node) in meta.nodes.iter().enumerate() {
            if !matches!(node.kind, TemplateNodeKind::Host { .. }) {
                continue;
            }
            let draft = work.hosts[index]
                .take()
                .unwrap_or_else(|| panic!("template {} built no draft for a host", meta.location));
            let id = arena.create_node(draft);
            if let Some(parent) = node.parent {
                // In preorder, so each lands after its earlier siblings.
                arena.place(
                    nodes[parent as usize].expect("parents come first"),
                    id,
                    None,
                );
            }
            nodes[index] = Some(id);
        }
        let root = nodes[0].expect("a template's root is a host");
        work.nodes = nodes;
        self.wip_nodes.get_mut(wip_id).unwrap().binding.current_host = Some(root);
        root
    }

    /// Rebuilds a mounted template's dynamic hosts from this render's drafts.
    fn commit_update_if_template(&mut self, wip_id: WipId, arena: &mut UiRuntime) {
        let Some(work) = self
            .wip_nodes
            .get_mut(wip_id)
            .and_then(WorkNode::template_work_mut)
        else {
            return;
        };
        if work.mode != BuildMode::Update {
            return;
        }
        for (index, draft) in work.hosts.iter_mut().enumerate() {
            if let Some(draft) = draft.take() {
                let id = work.nodes[index].expect("a mounted template keeps its hosts");
                arena.update_node(id, draft);
            }
        }
    }

    /// A template's structure and hosts: from its work this render, or as it
    /// was committed.
    fn template_nodes(&self, wip_id: WipId) -> (&'static TemplateMeta, Vec<Option<NodeId>>) {
        let wip = self.wip_nodes.get(wip_id).expect("template work node");
        if let Some(work) = wip.template_work() {
            return (work.meta, work.nodes.clone());
        }
        let state = wip
            .binding
            .current
            .and_then(|id| self.nodes.node(id))
            .and_then(Node::template)
            .expect("a template fiber without work is committed");
        (state.meta, state.nodes.clone())
    }

    /// The host a slot of `template` is placed before: the first sibling after
    /// it in the template that has one.
    fn template_anchor(&self, template: WipId, slot_child: WipId) -> Option<NodeId> {
        let (meta, nodes) = self.template_nodes(template);
        let children = &self.wip_nodes.get(template)?.children;
        let position = children.iter().position(|child| *child == slot_child)?;
        let (slot_index, _) = meta.slots().nth(position)?;
        for sibling in meta.following_siblings(slot_index) {
            match meta.nodes[sibling].kind {
                TemplateNodeKind::Host { .. } => return nodes[sibling],
                _ => {
                    let slot_position = meta.slots().position(|(index, _)| index == sibling)?;
                    if let Some(host) = children
                        .get(slot_position)
                        .and_then(|child| self.find_stable_host_in_wip_subtree(*child))
                    {
                        return Some(host);
                    }
                }
            }
        }
        None
    }

    fn commit_update_if_portal(
        &mut self,
        wip_id: WipId,
        parent_host: NodeId,
        arena: &mut UiRuntime,
    ) {
        let Some(portal) = self
            .wip_nodes
            .get_mut(wip_id)
            .filter(|node| matches!(node.tag, FiberTag::Portal))
            .and_then(WorkNode::portal_work_mut)
        else {
            return;
        };
        if let (Some(entry), Some(visual_root)) = (portal.entry, portal.visual_root) {
            arena
                .update_overlay_entry(entry, portal.scope, portal.options)
                .expect("Portal entry update failed");
            apply_portal_behavior(arena, entry, visual_root, parent_host, &portal.behavior);
        }
    }

    fn sync_portal_visual_root(
        &mut self,
        wip_id: WipId,
        parent_host: NodeId,
        arena: &mut UiRuntime,
    ) {
        let next_visual_root = self.first_host_in_wip_subtree(wip_id);
        let Some((scope, options, behavior, current_entry, current_visual_root)) = self
            .wip_nodes
            .get(wip_id)
            .and_then(WorkNode::portal_work)
            .map(|portal| {
                (
                    portal.scope,
                    portal.options,
                    portal.behavior.clone(),
                    portal.entry,
                    portal.visual_root,
                )
            })
        else {
            return;
        };
        if current_visual_root == next_visual_root {
            return;
        }

        if let Some(entry) = current_entry {
            detach_portal(arena, entry).expect("Portal's previous entry could not be unmounted");
        }
        let next_entry = next_visual_root.map(|visual_root| {
            let entry = arena
                .mount_overlay_entry(visual_root, scope, options)
                .expect("Portal's replacement entry could not be mounted");
            apply_portal_behavior(arena, entry, visual_root, parent_host, &behavior);
            entry
        });
        let portal = self
            .wip_nodes
            .get_mut(wip_id)
            .and_then(WorkNode::portal_work_mut)
            .expect("Portal model disappeared while synchronizing its visual root");
        portal.entry = next_entry;
        portal.visual_root = next_visual_root;
    }

    fn first_host_in_wip_subtree(&self, wip_id: WipId) -> Option<NodeId> {
        let node = self.wip_nodes.get(wip_id)?;
        if let Some(host) = self.host_node_for_wip(wip_id) {
            return Some(host);
        }
        node.children
            .iter()
            .find_map(|child| self.first_host_in_wip_subtree(*child))
    }

    /// The host a newly placed fiber goes before: the first host after it
    /// among its siblings, looking up through fibers that own no host.
    fn find_host_sibling_for_wip(&self, wip_id: WipId) -> Option<NodeId> {
        let mut node = wip_id;
        loop {
            let parent = self.wip_nodes.get(node)?.parent?;
            let parent_tag = self.wip_nodes.get(parent)?.tag;
            if parent_tag == FiberTag::Template {
                // A slot's siblings are the template's, not its fiber's.
                return self.template_anchor(parent, node);
            }
            let siblings = &self.wip_nodes.get(parent)?.children;
            let index = siblings.iter().position(|id| *id == node)?;
            for sibling in siblings.iter().copied().skip(index + 1) {
                if let Some(host) = self.find_stable_host_in_wip_subtree(sibling) {
                    return Some(host);
                }
            }
            if parent_tag == FiberTag::Portal {
                // Its children are the overlay entry's; nothing follows them.
                return None;
            }
            node = parent;
        }
    }

    fn find_stable_host_in_wip_subtree(&self, wip_id: WipId) -> Option<NodeId> {
        let node = self.wip_nodes.get(wip_id)?;

        if node
            .effect
            .intersects(EffectTag::PLACEMENT | EffectTag::MOVE)
        {
            return None;
        }

        if let Some(host_id) = self.host_node_for_wip(wip_id) {
            return Some(host_id);
        }

        // A portal's hosts live under the overlayer, not beside this one.
        if matches!(node.tag, FiberTag::Portal) {
            return None;
        }

        if !node.children_resolved {
            // Not visited this render: its committed subtree is what is
            // mounted.
            return node
                .binding
                .current
                .and_then(|current| self.first_committed_host(current));
        }

        for child in &node.children {
            if let Some(host) = self.find_stable_host_in_wip_subtree(*child) {
                return Some(host);
            }
        }

        None
    }

    /// The first host in a committed fiber subtree, not looking into portals.
    fn first_committed_host(&self, fiber: FiberId) -> Option<NodeId> {
        let node = self.nodes.node(fiber)?;
        if let Some(host) = node.host() {
            return Some(host);
        }
        if node.tag() == FiberTag::Portal {
            return None;
        }
        node.children(&self.nodes)
            .find_map(|child| self.first_committed_host(child.id))
    }

    fn freeze_work_tree(
        &mut self,
        wip_id: WipId,
        parent_fiber: Option<FiberId>,
        work: &mut WorkInProgress,
        arena: &UiRuntime,
    ) -> FiberId {
        let mut wip = work
            .live(&mut self.wip_nodes)
            .take_node(wip_id)
            .expect("commit missing work node");

        let fiber_id = wip.fiber_id;
        let fiber_children = if wip.children_resolved {
            std::mem::take(&mut wip.children)
                .into_iter()
                .map(|child| self.freeze_work_tree(child, Some(fiber_id), work, arena))
                .collect::<Vec<_>>()
        } else {
            self.collect_current_child_ids(wip.binding.current)
        };

        let kind = self.freeze_kind(&mut wip, arena);
        let frozen = crate::fiber::Node {
            id: fiber_id,
            parent: parent_fiber,
            child: None,
            sibling: None,
            key: wip.key,
            effect: EffectTag::empty(),
            kind,
            position: wip.position,
        };

        self.nodes.insert_node(fiber_id, frozen);
        self.nodes.set_children(fiber_id, &fiber_children);
        self.scheduler.mark_mounted(fiber_id);

        fiber_id
    }

    fn collect_current_child_ids(&self, current: Option<FiberId>) -> Vec<FiberId> {
        current
            .and_then(|id| self.nodes.node(id))
            .map(|node| node.children(&self.nodes).map(|child| child.id).collect())
            .unwrap_or_default()
    }

    /// What the fiber keeps: this render's work, or what it kept before.
    fn freeze_kind(&mut self, wip: &mut WorkNode, arena: &UiRuntime) -> FiberKind {
        let kind = match wip.work.take() {
            Some(Pending::Component(work)) => FiberKind::Component(work.into()),
            Some(Pending::Template(work)) => FiberKind::Template(TemplateState {
                meta: work.meta,
                identity: work.identity,
                nodes: work.nodes,
            }),
            Some(Pending::Fragment(_)) => FiberKind::Fragment,
            Some(Pending::Portal(portal)) => FiberKind::Portal(PortalState {
                scope: portal.scope,
                options: portal.options,
                behavior: portal.behavior,
                entry: portal.entry,
                visual_root: portal.visual_root,
            }),
            None => {
                let current = wip
                    .binding
                    .current
                    .and_then(|id| self.nodes.node_mut(id))
                    .expect("a fiber without work is committed");
                std::mem::replace(&mut current.kind, FiberKind::Fragment)
            }
        };
        if let FiberKind::Template(template) = &kind {
            debug_assert!(
                arena.contains(template.root()),
                "committed template root missing from arena"
            );
        }
        kind
    }

    fn clone_current_children(&mut self, parent: WipId) {
        let Some(current_children) = self
            .wip_nodes
            .get(parent)
            .and_then(|node| node.binding.current)
            .and_then(|id| self.nodes.node(id))
            .map(|node| {
                node.children(&self.nodes)
                    .map(|child| child.id)
                    .collect::<Vec<_>>()
            })
        else {
            return;
        };

        let render_lanes = self
            .work_in_progress
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(NO_LANES);
        let mut children = SmallVec::with_capacity(current_children.len());

        for (position, current) in current_children.into_iter().enumerate() {
            let current_node = self.nodes.node(current).expect("current child missing");
            let lanes = self.scheduler.component_lanes(current) & render_lanes;
            let child_lanes =
                child_tree_lanes(&self.nodes, current_node, &self.scheduler, render_lanes);
            let child = self.alloc_wip_node(WorkNode::from_current(
                current_node,
                Some(parent),
                position,
                lanes,
                child_lanes,
            ));
            children.push(child);
        }

        if let Some(node) = self.wip_nodes.get_mut(parent) {
            node.children = children;
            node.children_resolved = true;
        }
    }

    fn first_child_needing_work(&self, parent: WipId) -> Option<WipId> {
        let work = self.work_in_progress.as_ref()?;
        self.wip_nodes
            .get(parent)?
            .children
            .iter()
            .copied()
            .find(|child| {
                self.wip_nodes
                    .get(*child)
                    .is_some_and(|node| node.needs_begin_work(work.render_lanes))
            })
    }

    fn complete_work(&mut self, _id: WipId) {}

    fn ensure_work(&mut self) {
        let wip_lanes = self
            .work_in_progress
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(NO_LANES);
        let next_lanes = self.scheduler.get_next_lanes(wip_lanes);
        if next_lanes == NO_LANES {
            return;
        }

        if self.work_in_progress.is_none() || should_interrupt(wip_lanes, next_lanes) {
            if self.work_in_progress.is_some() {
                self.discard_uncommitted_work();
            }
            self.work_in_progress = Some(self.create_work_in_progress(next_lanes));
        }
    }

    fn create_work_in_progress(&mut self, render_lanes: Lanes) -> WorkInProgress {
        let (lanes, child_lanes) = self.collect_lane_marks(self.root_node(), render_lanes);
        self.wip_nodes.clear();
        let root_node = {
            let current_node = self.nodes.node(self.current).unwrap();
            WorkNode::from_current(current_node, None, 0, lanes, child_lanes)
        };

        let mut work = WorkInProgress {
            root: WipId(0),
            next_work: None,
            render_lanes,
            deletions: Vec::new(),
        };
        let root = work.live(&mut self.wip_nodes).alloc_node(root_node);
        work.root = root;
        work.next_work = Some(root);
        work
    }

    fn discard_uncommitted_work(&mut self) {
        let Some(_work) = self.work_in_progress.take() else {
            return;
        };

        for node in self.wip_nodes.drain(..).flatten() {
            if node.binding.current.is_none() {
                self.hooks.remove(&node.fiber_id);
                self.contexts.remove(node.fiber_id);
                self.nodes.remove_id(node.fiber_id);
                self.scheduler.mark_unmounted(node.fiber_id);
            }
        }
    }

    fn collect_lane_marks(&self, node: &Node, render_lanes: Lanes) -> (Lanes, Lanes) {
        let own = self.scheduler.component_lanes(node.id) & render_lanes;
        let mut child_lanes = NO_LANES;

        for child in node.children(&self.nodes) {
            let (child_own, child_subtree) = self.collect_lane_marks(child, render_lanes);
            child_lanes |= child_own | child_subtree;
        }
        (own, child_lanes)
    }

    fn next_sibling_needing_work(&self, id: WipId) -> Option<WipId> {
        let work = self.work_in_progress.as_ref()?;
        let node = self.wip_nodes.get(id)?;
        let parent = node.parent?;
        let siblings = &self.wip_nodes.get(parent)?.children;
        let index = siblings.iter().position(|child| *child == id)?;
        siblings.iter().copied().skip(index + 1).find(|sibling| {
            self.wip_nodes
                .get(*sibling)
                .is_some_and(|node| node.needs_begin_work(work.render_lanes))
        })
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Diff {
    ReuseClean,
    Update,
    Replace,
}

struct ChildMatch {
    old_index: usize,
    old_id: FiberId,
}

fn find_reusable_child_fast(
    nodes: &FiberArena,
    old_children: &[FiberId],
    used: &[bool],
    keyed_old: &FxHashMap<Key, usize>,
    prepared: &PreparedElement,
    position: usize,
) -> Option<ChildMatch> {
    let old_index = if let Some(key) = prepared.key.as_ref() {
        *keyed_old.get(key)?
    } else {
        position
    };

    if used.get(old_index).copied().unwrap_or(false) {
        return None;
    }

    let old = *old_children.get(old_index)?;
    let old_node = nodes.node(old)?;

    if prepared.key.is_none() && (old_node.key.is_some() || old_node.position != position) {
        return None;
    }

    if !can_reuse_prepared(old_node, prepared) {
        return None;
    }

    Some(ChildMatch {
        old_index,
        old_id: old,
    })
}

/// Whether the committed `old` can take `prepared`: the same kind, and for
/// a component the same render function, for a template the same template.
fn can_reuse_prepared(old: &Node, prepared: &PreparedElement) -> bool {
    match (&old.kind, &prepared.pending) {
        (FiberKind::Component(old), Pending::Component(new)) => old.render == new.render,
        (FiberKind::Template(old), Pending::Template(new)) => old.identity == new.identity,
        (FiberKind::Fragment, Pending::Fragment(_)) => true,
        (FiberKind::Portal(_), Pending::Portal(_)) => true,
        _ => false,
    }
}

/// Whether a reused fiber has anything to update. A component decides in
/// `should_render_component`; a template and a fragment always carry this
/// render's inputs and children.
fn prepared_needs_update(current: &Node, prepared: &PreparedElement) -> bool {
    if current.key != prepared.key {
        return true;
    }
    match (&current.kind, &prepared.pending) {
        (FiberKind::Component(_), Pending::Component(_)) => false,
        (FiberKind::Portal(old), Pending::Portal(new)) => {
            old.scope != new.scope || old.options != new.options || old.behavior != new.behavior
        }
        _ => true,
    }
}

/// The root component's call: its props are the application's render
/// function.
fn render_root(cx: &mut HookContext<'_>, props: Option<ErasedPropsRef<'_>>) -> ElementDesc {
    let render = props
        .and_then(|props| props.downcast_ref::<RootComponentRender>())
        .expect("the root component's props are its render function");
    render(cx)
}

/// Hands each part of a Portal's behavior to the runtime subsystem that runs
/// it. `owner` is the host the Portal is written under.
///
/// A Portal's lifecycle touches three subsystems -- the overlay entry, its
/// dismiss handler, and anchoring -- and this and [`detach_portal`] are the
/// only places that keep them in step.
fn apply_portal_behavior(
    arena: &mut UiRuntime,
    entry: OverlayEntryId,
    visual_root: NodeId,
    owner: NodeId,
    behavior: &PortalBehavior,
) {
    arena
        .set_overlay_entry_dismiss(entry, behavior.on_dismiss.clone())
        .expect("Portal entry disappeared while recording its dismiss handler");
    // Directly under the root or another Portal there is no host to follow.
    let target = (owner != arena.root_overlayer() && owner != arena.root()).then_some(owner);
    arena.set_anchor(
        visual_root,
        behavior.anchor.map(|placement| (target, placement)),
    );
}

/// Takes a Portal's entry off the overlayer and releases its visual root from
/// anchoring, which may outlive the entry when the Portal swaps roots.
fn detach_portal(
    arena: &mut UiRuntime,
    entry: OverlayEntryId,
) -> Result<NodeId, crate::widgets::OverlayModelError> {
    let visual_root = arena.unmount_overlay_entry(entry)?;
    arena.set_anchor(visual_root, None);
    Ok(visual_root)
}

fn child_tree_lanes(
    nodes: &FiberArena,
    node: &Node,
    scheduler: &Scheduler,
    render_lanes: Lanes,
) -> Lanes {
    let mut lanes = NO_LANES;
    for child in node.children(nodes) {
        lanes |= scheduler.component_lanes(child.id) & render_lanes;
        lanes |= child_tree_lanes(nodes, child, scheduler, render_lanes);
    }
    lanes
}

#[cfg(test)]
mod portal_tests {
    use super::*;
    use crate::element::portal;
    use crate::fiber::ComponentType;
    #[allow(unused_imports)]
    use crate::test_prelude::*;
    use crate::widgets::component;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    static PORTAL_MODE: AtomicU8 = AtomicU8::new(0);
    static ANCHOR_MODE: AtomicUsize = AtomicUsize::new(0);
    static CHILD_RENDER_COUNT: AtomicUsize = AtomicUsize::new(0);
    static LEAF_RENDER_COUNT: AtomicUsize = AtomicUsize::new(0);
    /// The fixtures below drive process-wide counters, so they cannot overlap.
    static RENDER_COUNT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        RENDER_COUNT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    thread_local! {
        static SHARED_CHILD: std::cell::RefCell<Option<ElementDesc>> =
            const { std::cell::RefCell::new(None) };
        static LEAF_STATE: std::cell::RefCell<Option<crate::state::State<u32>>> =
            const { std::cell::RefCell::new(None) };
    }

    struct NotHashProps;

    fn counted_child(_cx: &mut HookContext<'_>, props: Option<ErasedPropsRef<'_>>) -> ElementDesc {
        props
            .and_then(|props| props.downcast_ref::<NotHashProps>())
            .expect("counted child props changed type");
        CHILD_RENDER_COUNT.fetch_add(1, Ordering::SeqCst);
        container().into_element_desc(Vec::new())
    }

    fn counted_child_render() -> ComponentRender {
        ComponentRender::new(
            ComponentType::of::<NotHashProps>("counted_child"),
            counted_child,
        )
    }

    fn component_parent(_cx: &mut HookContext<'_>) -> ElementDesc {
        component(counted_child_render()).props(NotHashProps).into()
    }

    /// A child that owns state, so a scheduled update can be aimed at it while
    /// its props stay untouched.
    fn stateful_leaf(cx: &mut HookContext<'_>, _props: Option<ErasedPropsRef<'_>>) -> ElementDesc {
        let state = cx.use_state(|| 0u32);
        LEAF_STATE.with(|slot| *slot.borrow_mut() = Some(state));
        LEAF_RENDER_COUNT.fetch_add(1, Ordering::SeqCst);
        container().into_element_desc(Vec::new())
    }

    fn stateful_leaf_render() -> ComponentRender {
        ComponentRender::new(
            ComponentType::of::<NotHashProps>("stateful_leaf"),
            stateful_leaf,
        )
    }

    fn middle(_cx: &mut HookContext<'_>, _props: Option<ErasedPropsRef<'_>>) -> ElementDesc {
        CHILD_RENDER_COUNT.fetch_add(1, Ordering::SeqCst);
        component(stateful_leaf_render()).props(NotHashProps).into()
    }

    fn middle_render() -> ComponentRender {
        ComponentRender::new(ComponentType::of::<NotHashProps>("middle"), middle)
    }

    /// Hands its child the *same* element every render, which is what
    /// forwarding `children` or caching an element in `use_memo` produces.
    fn shared_element_parent(_cx: &mut HookContext<'_>) -> ElementDesc {
        SHARED_CHILD.with(|slot| {
            slot.borrow_mut()
                .get_or_insert_with(|| component(counted_child_render()).props(NotHashProps).into())
                .clone()
        })
    }

    fn shared_middle_parent(_cx: &mut HookContext<'_>) -> ElementDesc {
        SHARED_CHILD.with(|slot| {
            slot.borrow_mut()
                .get_or_insert_with(|| component(middle_render()).props(NotHashProps).into())
                .clone()
        })
    }

    fn reset_counters() {
        CHILD_RENDER_COUNT.store(0, Ordering::SeqCst);
        LEAF_RENDER_COUNT.store(0, Ordering::SeqCst);
        SHARED_CHILD.with(|slot| *slot.borrow_mut() = None);
        LEAF_STATE.with(|slot| *slot.borrow_mut() = None);
    }

    fn portal_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        let mode = PORTAL_MODE.load(Ordering::SeqCst);
        if mode < 2 {
            portal(vec![
                container()
                    .key(if mode == 0 {
                        "portal-visual-root"
                    } else {
                        "portal-replacement-root"
                    })
                    .into_element_desc(Vec::new()),
            ])
            .key("portal")
            .z_index(100)
            .into()
        } else {
            container().key("content").into_element_desc(Vec::new())
        }
    }

    #[test]
    fn portal_keeps_logical_fiber_but_mounts_and_unmounts_under_overlayer() {
        PORTAL_MODE.store(0, Ordering::SeqCst);
        let mut arena = UiRuntime::new();
        let mut runtime = ComponentRuntime::new(arena.root(), Scheduler::default(), portal_root);
        runtime.flush_sync(&mut arena);

        let portal_fiber = runtime.nodes.children(runtime.root())[0];
        assert_eq!(
            runtime.nodes.node(portal_fiber).unwrap().tag(),
            FiberTag::Portal
        );
        let visual_children = arena.children(arena.root_overlayer()).collect::<Vec<_>>();
        assert_eq!(visual_children.len(), 1);
        assert_eq!(
            arena.node(visual_children[0]).unwrap().key,
            Some(&Key::from("portal-visual-root"))
        );

        PORTAL_MODE.store(1, Ordering::SeqCst);
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        let replacement_children = arena.children(arena.root_overlayer()).collect::<Vec<_>>();
        assert_eq!(replacement_children.len(), 1);
        assert_eq!(
            arena.node(replacement_children[0]).unwrap().key,
            Some(&Key::from("portal-replacement-root"))
        );

        PORTAL_MODE.store(2, Ordering::SeqCst);
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        assert!(arena.children(arena.root_overlayer()).next().is_none());
    }

    #[test]
    fn removing_a_template_releases_nested_portal_hosts() {
        thread_local! {
            static VISIBLE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        }
        fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
            if VISIBLE.get() {
                container().into_element_desc(vec![
                    portal(vec![container().into_element_desc(Vec::new())]).into(),
                ])
            } else {
                crate::element::FragmentDesc::default().into()
            }
        }
        VISIBLE.set(true);
        let mut arena = UiRuntime::new();
        let mut runtime = ComponentRuntime::new(arena.root(), Scheduler::default(), root);
        runtime.flush_sync(&mut arena);
        let visual_root = arena.children(arena.root_overlayer()).next().unwrap();
        VISIBLE.set(false);
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        assert!(arena.children(arena.root_overlayer()).next().is_none());
        assert!(arena.node(visual_root).is_none());
    }

    fn anchored_portal_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        use crate::anchor::AnchorPlacement;
        use xui_interface::{EdgeInsets, Size};

        let menu = container()
            .key("menu")
            .with::<container::size>(Size::fix(60.0, 40.0))
            .into_element_desc(Vec::new());
        // Written inside the trigger, so the trigger owns the Portal.
        let trigger = container()
            .key("trigger")
            .with::<container::size>(Size::fix(80.0, 20.0))
            .into_element_desc(vec![
                portal(vec![menu])
                    .key("portal")
                    .anchor(AnchorPlacement::default().offset(0.0))
                    .on_dismiss(|_| {})
                    .into(),
            ]);
        container()
            .with::<container::style>(
                StyleOf::<Container>::new()
                    .with::<container::size>(Size::fill())
                    .with::<container::padding>(EdgeInsets::new(30.0, 0.0, 50.0, 0.0)),
            )
            .into_element_desc(vec![trigger])
    }

    /// 0: anchored. 1: the same Portal without an anchor. 2: anchored again,
    /// on a new root. 3: gone.
    fn anchor_lifecycle_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        use crate::anchor::AnchorPlacement;

        let mode = ANCHOR_MODE.load(Ordering::SeqCst);
        let mut children = Vec::new();
        if mode < 3 {
            let menu = container()
                .key(if mode < 2 { "menu" } else { "replacement" })
                .into_element_desc(Vec::new());
            let mut desc = portal(vec![menu]).key("portal");
            if mode != 1 {
                desc = desc.anchor(AnchorPlacement::default());
            }
            children.push(desc.into());
        }
        container().key("trigger").into_element_desc(children)
    }

    /// Every commit path keeps anchoring in step with the Portal: placement,
    /// an update that drops or restores the anchor, a swapped visual root,
    /// and deletion.
    #[test]
    fn a_portal_keeps_its_anchor_in_step_through_every_commit_path() {
        ANCHOR_MODE.store(0, Ordering::SeqCst);
        let mut arena = UiRuntime::new();
        let mut runtime =
            ComponentRuntime::new(arena.root(), Scheduler::default(), anchor_lifecycle_root);
        runtime.flush_sync(&mut arena);
        let overlay_root = |arena: &UiRuntime| arena.children(arena.root_overlayer()).next();
        let trigger = arena
            .children(arena.root())
            .find(|id| *id != arena.root_overlayer())
            .unwrap();

        let menu = overlay_root(&arena).unwrap();
        let anchor = arena.anchors().get(menu).expect("placement anchors");
        assert_eq!(anchor.target, Some(trigger));

        ANCHOR_MODE.store(1, Ordering::SeqCst);
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        assert_eq!(overlay_root(&arena), Some(menu));
        assert!(
            arena.anchors().get(menu).is_none(),
            "an update without an anchor must release it"
        );

        ANCHOR_MODE.store(2, Ordering::SeqCst);
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        let replacement = overlay_root(&arena).unwrap();
        assert_ne!(replacement, menu);
        assert_eq!(
            arena.anchors().get(replacement).map(|anchor| anchor.target),
            Some(Some(trigger))
        );

        ANCHOR_MODE.store(3, Ordering::SeqCst);
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        assert!(overlay_root(&arena).is_none());
        assert!(arena.anchors().is_empty());
    }

    #[test]
    fn a_portal_anchors_its_content_to_the_host_it_is_written_in() {
        use crate::text::{TextHost, testing::ZeroTextBackend};
        use xui_interface::{Point, Size};

        let mut arena = UiRuntime::new();
        let mut runtime =
            ComponentRuntime::new(arena.root(), Scheduler::default(), anchored_portal_root);
        runtime.flush_sync(&mut arena);
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(400.0, 300.0), &mut measurer);

        let find = |key: &str| {
            let mut stack = vec![arena.root()];
            while let Some(id) = stack.pop() {
                if arena.node(id).unwrap().key == Some(&Key::from(key)) {
                    return id;
                }
                stack.extend(arena.children(id));
            }
            panic!("no host keyed {key}");
        };
        let menu = find("menu");
        assert_eq!(
            arena.children(arena.root_overlayer()).collect::<Vec<_>>(),
            [menu]
        );
        assert_eq!(
            arena.visual_layout(menu).min,
            Point::new(30.0, 70.0),
            "placed below the trigger at (30, 50)"
        );
        assert_eq!(
            arena.dismissable_overlay().map(|(root, _)| root),
            Some(menu)
        );
    }

    /// The bailout must not fire when the parent built fresh props, even
    /// though their contents are identical: only pointer equality counts.
    #[test]
    fn ordinary_component_renders_again_when_its_parent_renders() {
        let _exclusive = exclusive();
        CHILD_RENDER_COUNT.store(0, Ordering::SeqCst);
        let mut arena = UiRuntime::new();
        let mut runtime =
            ComponentRuntime::new(arena.root(), Scheduler::default(), component_parent);

        runtime.flush_sync(&mut arena);
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);

        assert_eq!(CHILD_RENDER_COUNT.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_component_handed_the_same_props_allocation_is_not_re_rendered() {
        let _exclusive = exclusive();
        reset_counters();
        let mut arena = UiRuntime::new();
        let mut runtime =
            ComponentRuntime::new(arena.root(), Scheduler::default(), shared_element_parent);

        runtime.flush_sync(&mut arena);
        assert_eq!(CHILD_RENDER_COUNT.load(Ordering::SeqCst), 1, "mount");

        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        assert_eq!(
            CHILD_RENDER_COUNT.load(Ordering::SeqCst),
            1,
            "the child re-ran even though it was handed the same props"
        );
    }

    /// A scheduled update on the component itself outranks the bailout.
    #[test]
    fn a_bailed_out_component_still_renders_for_its_own_update() {
        let _exclusive = exclusive();
        reset_counters();
        let mut arena = UiRuntime::new();
        let mut runtime =
            ComponentRuntime::new(arena.root(), Scheduler::default(), shared_middle_parent);

        runtime.flush_sync(&mut arena);
        assert_eq!(LEAF_RENDER_COUNT.load(Ordering::SeqCst), 1, "mount");

        // Nothing changed: both levels bail out.
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);
        assert_eq!(
            CHILD_RENDER_COUNT.load(Ordering::SeqCst),
            1,
            "middle re-ran"
        );
        assert_eq!(LEAF_RENDER_COUNT.load(Ordering::SeqCst), 1, "leaf re-ran");
    }

    /// The middle component bails out, but its descendant has work scheduled.
    /// Cloning the subtree must not strand that update.
    #[test]
    fn a_bailout_does_not_strand_a_descendant_with_scheduled_work() {
        let _exclusive = exclusive();
        reset_counters();
        let mut arena = UiRuntime::new();
        let mut runtime =
            ComponentRuntime::new(arena.root(), Scheduler::default(), shared_middle_parent);

        runtime.flush_sync(&mut arena);
        let mounted_leaf = LEAF_RENDER_COUNT.load(Ordering::SeqCst);
        let mounted_middle = CHILD_RENDER_COUNT.load(Ordering::SeqCst);

        // Both at once: the root re-renders, so the middle is handed
        // ComponentWork and bails out on pointer equality -- while its child
        // has an update that still has to get through.
        LEAF_STATE.with(|slot| {
            slot.borrow().as_ref().expect("leaf mounted").set(1);
        });
        runtime.mark_root_dirty();
        runtime.flush_sync(&mut arena);

        assert_eq!(
            LEAF_RENDER_COUNT.load(Ordering::SeqCst),
            mounted_leaf + 1,
            "the leaf's own update was lost behind its parent's bailout"
        );
        assert_eq!(
            CHILD_RENDER_COUNT.load(Ordering::SeqCst),
            mounted_middle,
            "the middle re-ran although only its child had work"
        );
    }
}
