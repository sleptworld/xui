use crate::element::PortalBehavior;
use crate::element::PortalDesc;
use crate::event_system::interaction::HostInteraction;
use crate::fiber::{
    ComponentRender, ComponentState, EffectTag, ErasedProps, FiberArena, FiberId, FiberTag,
    HostState, Key, Node, PortalState,
};
use crate::lanes::{Lanes, NO_LANES, current_update_lane, includes_some_lane, should_interrupt};
use crate::state::{AsyncDispatcher, HookContext, HookStorage, Scheduler};
use crate::ticker::TickerRegistry;
use crate::ui_runtime::UiRuntime;
use crate::widgets::{OverlayEntryId, OverlayEntryOptions, OverlayScopeId};
use crate::widgets::{RootComponentRender, WidgetI};
use crate::{ComponentDesc, ElementDesc, ErasedPropsRef};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::fmt;
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
    // Signature of current fiber
    effect: EffectTag,
    children_resolved: bool,
    children: SmallVec<[WipId; 8]>,
    lanes: Lanes,
    child_lanes: Lanes,
    work: Option<Work>,
    binding: WorkBinding,
}

enum Work {
    HostWork(HostWork),
    ComponentWork(ComponentWork),
    PortalWork(PortalWork),
}

#[derive(Default)]
struct WorkBinding {
    current: Option<FiberId>,
    current_host: Option<NodeId>,
}

struct HostWork {
    widget: Option<WidgetI>,
    interaction: Option<HostInteraction>,
    props_hash: u64,
    pending_children: Vec<ElementDesc>,
}

struct ComponentWork {
    render: ComponentRender,
    key: Option<Key>,
    props: Option<ErasedProps>,
}

struct PortalWork {
    scope: Option<OverlayScopeId>,
    options: OverlayEntryOptions,
    behavior: PortalBehavior,
    pending_children: Vec<ElementDesc>,
    entry: Option<crate::widgets::OverlayEntryId>,
    visual_root: Option<NodeId>,
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
    fn host_work(&self) -> Option<&HostWork> {
        match self.work.as_ref() {
            Some(Work::HostWork(work)) => Some(work),
            _ => None,
        }
    }

    fn host_work_mut(&mut self) -> Option<&mut HostWork> {
        match self.work.as_mut() {
            Some(Work::HostWork(work)) => Some(work),
            _ => None,
        }
    }

    fn portal_work(&self) -> Option<&PortalWork> {
        match self.work.as_ref() {
            Some(Work::PortalWork(work)) => Some(work),
            _ => None,
        }
    }

    fn portal_work_mut(&mut self) -> Option<&mut PortalWork> {
        match self.work.as_mut() {
            Some(Work::PortalWork(work)) => Some(work),
            _ => None,
        }
    }

    fn take_component_work(&mut self) -> Option<ComponentWork> {
        if !matches!(self.work, Some(Work::ComponentWork(_))) {
            return None;
        }
        match self.work.take() {
            Some(Work::ComponentWork(work)) => Some(work),
            _ => unreachable!(),
        }
    }

    fn take_portal_work(&mut self) -> Option<PortalWork> {
        if !matches!(self.work, Some(Work::PortalWork(_))) {
            return None;
        }
        match self.work.take() {
            Some(Work::PortalWork(work)) => Some(work),
            _ => unreachable!(),
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
            tag: current.tag,
            children: SmallVec::new(),
            children_resolved: false,
            effect: EffectTag::empty(),
            lanes,
            child_lanes,
            work: None,
            binding: WorkBinding {
                current: Some(current.id),
                current_host: current.host.as_ref().and_then(|h| h.node_id),
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
        let work = match prepared.pending {
            PreparedPending::Host {
                widget,
                interaction,
                props_hash,
                children,
            } => Work::HostWork(HostWork {
                widget: Some(widget),
                interaction,
                props_hash,
                pending_children: children,
            }),
            PreparedPending::Component { key, render, props } => {
                Work::ComponentWork(ComponentWork { render, key, props })
            }
            PreparedPending::Portal {
                scope,
                options,
                behavior,
                children,
            } => Work::PortalWork(PortalWork {
                scope,
                options,
                behavior,
                pending_children: children,
                entry: current
                    .and_then(|id| nodes.node(id))
                    .and_then(|node| node.portal.as_ref())
                    .and_then(|portal| portal.entry),
                visual_root: current
                    .and_then(|id| nodes.node(id))
                    .and_then(|node| node.portal.as_ref())
                    .and_then(|portal| portal.visual_root),
            }),
        };

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
                current_host: current
                    .and_then(|c| nodes.node(c))
                    .and_then(|n| n.host.as_ref())
                    .and_then(|n| n.node_id),
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
        let Some(Work::ComponentWork(work)) = &self.work else {
            return false;
        };
        let Some(committed) = self
            .binding
            .current
            .and_then(|id| fiber_tree.node(id))
            .and_then(|node| node.component.as_ref())
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

    fn take_work_nodes(&mut self) -> Option<Vec<ElementDesc>> {
        match &mut self.work {
            Some(Work::HostWork(h)) => Some(std::mem::take(&mut h.pending_children)),
            Some(Work::PortalWork(p)) => Some(std::mem::take(&mut p.pending_children)),
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
            Some(Work::ComponentWork(c)) => Some((c.render, c.props.as_deref())),
            _ => self
                .binding
                .current
                .and_then(|id| fiber_tree.node(id))
                .and_then(|node| node.component.as_ref())
                .map(|component| (component.render, component.props.as_deref())),
        }
    }
}

struct PreparedElement {
    key: Option<Key>,
    tag: FiberTag,
    pending: PreparedPending,
}

enum PreparedPending {
    Host {
        widget: WidgetI,
        interaction: Option<HostInteraction>,
        props_hash: u64,
        children: Vec<ElementDesc>,
    },
    Component {
        key: Option<Key>,
        render: ComponentRender,
        props: Option<ErasedProps>,
    },
    Portal {
        scope: Option<OverlayScopeId>,
        options: OverlayEntryOptions,
        behavior: PortalBehavior,
        children: Vec<ElementDesc>,
    },
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
    root_render: RootComponentRender,
    work_in_progress: Option<WorkInProgress>,
    scheduler: Scheduler,
    async_dispatcher: AsyncDispatcher,
    tokio_handle: Option<TokioHandle>,
    /// Handed to every `HookContext`, so `use_ticker` can install into the
    /// list the runtime walks each frame.
    tickers: TickerRegistry,
    hooks: FxHashMap<FiberId, HookStorage>,
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
        let arena = FiberArena::new();
        let current = arena.root();
        scheduler.set_root(current);
        scheduler.mark_component_dirty(current, current_update_lane());

        Self {
            nodes: arena,
            current,
            root_render,
            root_widget,
            work_in_progress: None,
            scheduler,
            async_dispatcher,
            tokio_handle,
            tickers,
            hooks: FxHashMap::default(),
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
        let (fiber_id, tag, should_render, render_lanes) = {
            let work = self.work_in_progress.as_ref().expect("work missing");
            let node = self.wip_nodes.get(id).expect("work node missing");

            (
                node.fiber_id,
                node.tag,
                node.should_render_component(work.render_lanes, &self.nodes),
                work.render_lanes,
            )
        };

        macro_rules! cx {
            ($id: ident) => {{
                let storage = self.hooks.entry($id).or_default();
                let cx = HookContext::new_with_async(
                    storage,
                    $id,
                    self.scheduler.clone(),
                    render_lanes,
                    self.async_dispatcher.clone(),
                    self.tokio_handle.clone(),
                    self.tickers.clone(),
                );
                cx
            }};
        }

        match tag {
            FiberTag::Root => {
                if should_render {
                    let mut cx = cx!(fiber_id);
                    let element = (self.root_render)(&mut cx);
                    self.reconcile_children(id, [element]);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Component => {
                if should_render {
                    let wip_node = self.wip_nodes.get(id).unwrap();
                    if let Some((render, props)) = wip_node.component_render_props(&self.nodes) {
                        let mut cx = cx!(fiber_id);
                        let element = (render.call)(&mut cx, props);
                        self.reconcile_children(id, [element]);
                    }
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Host(_) => {
                let pending_children = self
                    .wip_nodes
                    .get_mut(id)
                    .and_then(WorkNode::take_work_nodes);

                if let Some(children) = pending_children {
                    self.reconcile_children(id, children);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Portal => {
                let pending_children = self
                    .wip_nodes
                    .get_mut(id)
                    .and_then(WorkNode::take_work_nodes);
                if let Some(children) = pending_children {
                    self.reconcile_children(id, children);
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

        for (position, element) in new_children.enumerate() {
            let prepared = self.prepare_element(element);

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
        match element {
            ElementDesc::Component(component) => self.prepare_component_element(component, key),
            ElementDesc::Host(host) => {
                let widget = host.widget;
                let props_hash = widget.props_hash();
                let tag = FiberTag::Host(widget.node_type());

                let interaction = widget.take_host_interaction();
                PreparedElement {
                    key,
                    tag,
                    pending: PreparedPending::Host {
                        widget,
                        interaction,
                        props_hash,
                        children: host.children,
                    },
                }
            }
            ElementDesc::Portal(portal) => self.prepare_portal_element(portal, key),
        }
    }

    fn prepare_portal_element(&self, portal: PortalDesc, key: Option<Key>) -> PreparedElement {
        PreparedElement {
            key,
            tag: FiberTag::Portal,
            pending: PreparedPending::Portal {
                scope: portal.scope,
                options: portal.options,
                behavior: portal.behavior,
                children: portal.children,
            },
        }
    }

    fn prepare_component_element(
        &self,
        component: ComponentDesc,
        key: Option<Key>,
    ) -> PreparedElement {
        PreparedElement {
            key,
            tag: FiberTag::Component,
            pending: PreparedPending::Component {
                key,
                render: component.render,
                props: component.props,
            },
        }
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
        let host_node = self
            .nodes
            .node(id)
            .and_then(|node| node.host.as_ref())
            .and_then(|host| host.node_id);
        let portal_entry = self
            .nodes
            .node(id)
            .and_then(|node| node.portal.as_ref())
            .and_then(|portal| portal.entry);
        self.scheduler.mark_unmounted(id);
        self.hooks.remove(&id);

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
            self.commit_update_if_host(wip_id, arena);
            self.commit_update_if_portal(wip_id, parent_host, arena);
        }

        let child_parent_host = match tag {
            FiberTag::Host(_) => self
                .host_node_for_wip(wip_id)
                .expect("host fiber missing host node after mutation"),
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

    fn host_node_for_wip(&self, wip_id: WipId) -> Option<NodeId> {
        let wip = self.wip_nodes.get(wip_id)?;

        if let Some(node_id) = wip.binding.current_host {
            return Some(node_id);
        }

        wip.binding.current_host
    }

    fn ensure_host_created(&mut self, wip_id: WipId, arena: &mut UiRuntime) -> Option<NodeId> {
        if let Some(id) = self
            .wip_nodes
            .get(wip_id)
            .and_then(|n| n.binding.current_host.as_ref())
        {
            return Some(*id);
        }

        let (key, props_hash, widget, interaction) = {
            let wip = self.wip_nodes.get_mut(wip_id)?;
            let key = wip.key;
            let host_work = wip.host_work_mut()?;
            let widget = host_work.widget.take()?;
            let interaction = host_work.interaction.take();
            (key, host_work.props_hash, widget, interaction)
        };

        let node_id = arena.create_node(key, props_hash, widget, interaction);
        self.wip_nodes.get_mut(wip_id)?.binding.current_host = Some(node_id);
        Some(node_id)
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

        if matches!(tag, FiberTag::Host(_)) {
            let host_id = self.ensure_host_created(wip_id, arena).expect("missing");
            arena.place(parent_host, host_id, before);
            if let Some(node) = self.wip_nodes.get_mut(wip_id) {
                node.effect.remove(EffectTag::PLACEMENT);
                node.effect.remove(EffectTag::MOVE);
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

    fn commit_update_if_host(&mut self, wip_id: WipId, arena: &mut UiRuntime) {
        if !self
            .wip_nodes
            .get(wip_id)
            .is_some_and(|node| matches!(node.tag, FiberTag::Host(_)))
        {
            return;
        }

        let node_id = self
            .host_node_for_wip(wip_id)
            .expect("update host fiber missing host node");
        let wip = self
            .wip_nodes
            .get_mut(wip_id)
            .expect("update host fiber missing work node");
        let key = wip.key;
        let host_work = wip.host_work_mut().expect("host update missing host work");
        let widget = host_work.widget.take().expect("host update missing widget");
        let interaction = host_work.interaction.take();

        arena.update_node(node_id, key, host_work.props_hash, widget, interaction);
        wip.binding.current_host = Some(node_id);
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
            // .filter(|node| matches!(node.tag, FiberTag::Portal))
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

    fn find_host_sibling_for_wip(&self, wip_id: WipId) -> Option<NodeId> {
        let mut node = wip_id;

        loop {
            let parent = self.wip_nodes.get(node)?.parent?;
            let siblings = &self.wip_nodes.get(parent)?.children;
            let index = siblings.iter().position(|id| *id == node)?;

            for sibling in siblings.iter().copied().skip(index + 1) {
                if let Some(host) = self.find_stable_host_in_wip_subtree(sibling) {
                    return Some(host);
                }
            }

            node = parent;

            if matches!(self.wip_nodes.get(node)?.tag, FiberTag::Host(_)) {
                return None;
            }
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

        for child in &node.children {
            if let Some(host) = self.find_stable_host_in_wip_subtree(*child) {
                return Some(host);
            }
        }

        None
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

        let host = self.freeze_host_state(&mut wip, arena);
        let component = self.freeze_component_state(&mut wip);
        let portal = self.freeze_portal_state(&mut wip);
        let frozen = crate::fiber::Node {
            id: fiber_id,
            parent: parent_fiber,
            child: None,
            sibling: None,
            key: wip.key,
            tag: wip.tag,
            effect: EffectTag::empty(),
            host,
            component,
            portal,
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

    fn freeze_host_state(&mut self, wip: &mut WorkNode, arena: &UiRuntime) -> Option<HostState> {
        if !matches!(wip.tag, FiberTag::Host(_)) {
            return None;
        }

        let current_host = wip
            .binding
            .current
            .and_then(|id| self.nodes.node_mut(id))
            .and_then(|node| node.host.take());
        let node_id = wip
            .binding
            .current_host
            .or_else(|| current_host.as_ref().and_then(|host| host.node_id))
            .expect("host fiber missing committed host node");
        let arena_node = arena
            .node(node_id)
            .expect("committed host node missing from arena");
        let props_hash = wip
            .host_work()
            .map(|host| host.props_hash)
            .or_else(|| current_host.as_ref().map(|host| host.props_hash))
            .unwrap_or(arena_node.new_props_hash);

        Some(HostState {
            node_id: Some(node_id),
            widget: Some(arena_node.widget.clone()),
            style: current_host
                .as_ref()
                .map(|host| host.style.clone())
                .unwrap_or_default(),
            computed_style: arena_node.target_style.clone(),
            layout: arena_node.layout,
            previous_layout: arena_node.previous_layout,
            props_hash,
        })
    }

    fn freeze_component_state(&mut self, wip: &mut WorkNode) -> Option<ComponentState> {
        if !matches!(wip.tag, FiberTag::Component) {
            return None;
        }

        if let Some(component) = wip.take_component_work() {
            return Some(component.into());
        }

        wip.binding
            .current
            .and_then(|id| self.nodes.node_mut(id))
            .and_then(|node| node.component.take())
    }

    fn freeze_portal_state(&mut self, wip: &mut WorkNode) -> Option<PortalState> {
        if !matches!(wip.tag, FiberTag::Portal) {
            return None;
        }
        if let Some(portal) = wip.take_portal_work() {
            return Some(PortalState {
                scope: portal.scope,
                options: portal.options,
                behavior: portal.behavior,
                entry: portal.entry,
                visual_root: portal.visual_root,
            });
        }
        wip.binding
            .current
            .and_then(|id| self.nodes.node_mut(id))
            .and_then(|node| node.portal.take())
    }

    fn trace_commit_work_node(&self, depth: usize, event: fmt::Arguments<'_>) {
        let indent = "  ".repeat(depth);
        eprintln!("[xui_core::commit] {indent}{event}");
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

fn find_reusable_child(
    nodes: &FiberArena,
    old_children: &[FiberId],
    used: &[bool],
    prepared: &PreparedElement,
    position: usize,
) -> Option<usize> {
    if let Some(key) = prepared.key.as_ref() {
        return old_children
            .iter()
            .copied()
            .enumerate()
            .find(|(index, old_id)| {
                !used[*index]
                    && nodes.node(*old_id).is_some_and(|old| {
                        old.key.as_ref() == Some(key) && can_reuse_prepared(old, prepared)
                    })
            })
            .map(|(index, _)| index);
    }

    old_children
        .get(position)
        .copied()
        .filter(|old_id| {
            !used[position]
                && nodes.node(*old_id).is_some_and(|old| {
                    old.key.is_none()
                        && old.position == position
                        && can_reuse_prepared(old, prepared)
                })
        })
        .map(|_| position)
}

fn can_reuse_prepared(old: &Node, prepared: &PreparedElement) -> bool {
    if old.tag != prepared.tag {
        return false;
    }

    match &prepared.pending {
        PreparedPending::Component { render, .. } => old
            .component
            .as_ref()
            .is_some_and(|component| component.render == *render),
        PreparedPending::Host { .. } => true,
        PreparedPending::Portal { .. } => true,
    }
}

fn prepared_needs_update(current: &Node, prepared: &PreparedElement) -> bool {
    if current.tag != prepared.tag || current.key != prepared.key {
        return true;
    }

    match (&current.host, &current.component, &prepared.pending) {
        (Some(host_state), _, PreparedPending::Host { props_hash, .. }) => {
            host_state.props_hash != *props_hash
        }
        (_, Some(_), PreparedPending::Component { .. }) => false,
        (
            _,
            _,
            PreparedPending::Portal {
                scope,
                options,
                behavior,
                ..
            },
        ) => current.portal.as_ref().is_none_or(|portal| {
            portal.scope != *scope || portal.options != *options || portal.behavior != *behavior
        }),
        _ => true,
    }
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
    use crate::widgets::{component, container};
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
        ComponentRender::new(ComponentType::new("counted_child"), counted_child)
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
        ComponentRender::new(ComponentType::new("stateful_leaf"), stateful_leaf)
    }

    fn middle(_cx: &mut HookContext<'_>, _props: Option<ErasedPropsRef<'_>>) -> ElementDesc {
        CHILD_RENDER_COUNT.fetch_add(1, Ordering::SeqCst);
        component(stateful_leaf_render()).props(NotHashProps).into()
    }

    fn middle_render() -> ComponentRender {
        ComponentRender::new(ComponentType::new("middle"), middle)
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
            runtime.nodes.node(portal_fiber).unwrap().tag,
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

    fn anchored_portal_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        use crate::anchor::AnchorPlacement;
        use xui_interface::{EdgeInsets, Size, Style};

        let menu = container()
            .key("menu")
            .style(Style::new().size(Size::fix(60.0, 40.0)))
            .into_element_desc(Vec::new());
        // Written inside the trigger, so the trigger owns the Portal.
        let trigger = container()
            .key("trigger")
            .style(Style::new().size(Size::fix(80.0, 20.0)))
            .into_element_desc(vec![
                portal(vec![menu])
                    .key("portal")
                    .anchor(AnchorPlacement::default().offset(0.0))
                    .on_dismiss(|_| {})
                    .into(),
            ]);
        container()
            .style(
                Style::new()
                    .size(Size::fill())
                    .padding(EdgeInsets::new(30.0, 0.0, 50.0, 0.0)),
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
