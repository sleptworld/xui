use crate::fiber::{
    ComponentRegistry, ComponentState, ComponentType, EffectTag, ErasedProps, FiberArena, FiberId,
    FiberTag, HostState, Key, Node,
};
use crate::font::TextI;
use crate::lanes::{Lanes, NO_LANES, current_update_lane, includes_some_lane, should_interrupt};
use crate::state::{HookContext, HookStorage, Scheduler};
use crate::tree::UiArena;
use crate::widgets::{ComponentRender, Element, WidgetRef};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use taffy as tf;
use xui_interface::{DirtyFlags, EventHandlers, NodeId, TextMeasurer};

pub struct WorkNode {
    id: FiberId,
    parent: Option<FiberId>,
    key: Option<Key>,
    tag: FiberTag,
    position: usize,
    current: Option<FiberId>,
    effect: EffectTag,
    children_resolved: bool,
    children: SmallVec<[FiberId; 20]>,
    pending_children: Option<Vec<Element>>,
    lanes: Lanes,
    child_lanes: Lanes,
    began: bool,
    host_work: Option<HostWork>,
    host_node: Option<NodeId>,
    component: Option<ComponentWork>,
}

struct HostWork {
    widget: Option<WidgetRef>,
    event_handlers: Option<EventHandlers>,
    style: tf::Style,
    props_hash: u64,
}

#[derive(Clone)]
struct ComponentWork {
    render: ComponentType,
    key: Option<Key>,
    props_hash: u64,
    props: ErasedProps,
}

impl WorkNode {
    fn from_current(
        current: &Node,
        parent: Option<FiberId>,
        children: impl IntoIterator<Item = FiberId>,
        position: usize,
        lanes: Lanes,
        child_lanes: Lanes,
    ) -> Self {
        Self {
            id: current.id,
            parent,
            key: current.key.clone(),
            position,
            tag: current.tag,
            children: children.into_iter().collect(),
            current: Some(current.id),
            children_resolved: false,
            effect: EffectTag::None,
            pending_children: None,
            lanes,
            child_lanes,
            began: false,
            host_node: current.host.as_ref().and_then(|host| host.node_id),
            host_work: None,
            component: current.component.as_ref().map(ComponentWork::from_state),
        }
    }

    fn from_prepared(
        nodes: &FiberArena,
        id: FiberId,
        parent: FiberId,
        position: usize,
        prepared: PreparedElement,
        current: Option<FiberId>,
        effect: EffectTag,
        lanes: Lanes,
        child_lanes: Lanes,
    ) -> Self {
        let host_node = current
            .and_then(|current| nodes.node(current))
            .and_then(|node| node.host.as_ref())
            .and_then(|host| host.node_id);

        let (host_work, pending_children, component) = match prepared.pending {
            PreparedPending::Host {
                widget,
                event_handlers,
                style,
                props_hash,
                children,
            } => (
                Some(HostWork {
                    widget: Some(widget),
                    event_handlers: Some(event_handlers),
                    style,
                    props_hash,
                }),
                Some(children),
                None,
            ),
            PreparedPending::Component {
                key,
                render,
                props_hash,
                props,
            } => (
                None,
                None,
                Some(ComponentWork {
                    render,
                    key,
                    props_hash,
                    props,
                }),
            ),
        };

        Self {
            id,
            parent: Some(parent),
            key: prepared.key,
            position,
            tag: prepared.tag,
            current,
            effect,
            children: SmallVec::new(),
            pending_children,
            children_resolved: false,
            lanes,
            child_lanes,
            began: false,
            host_work,
            host_node,
            component,
        }
    }

    fn needs_work(&self, render_lanes: Lanes) -> bool {
        self.effect != EffectTag::None
            || self.current.is_none()
            || self.pending_children.is_some()
            || self.component.is_some()
            || includes_some_lane(self.lanes | self.child_lanes, render_lanes)
    }
}

impl ComponentWork {
    fn from_state(state: &ComponentState) -> Self {
        Self {
            render: state.render,
            key: state.key.clone(),
            props_hash: state.props_hash,
            props: state.props.clone(),
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
        widget: WidgetRef,
        event_handlers: EventHandlers,
        style: tf::Style,
        props_hash: u64,
        children: Vec<Element>,
    },
    Component {
        key: Option<Key>,
        render: ComponentType,
        props_hash: u64,
        props: ErasedProps,
    },
}

pub struct WorkInProgress {
    nodes: FxHashMap<FiberId, WorkNode>,
    root: FiberId,
    next_work: Option<FiberId>,
    render_lanes: Lanes,
    deletions: Vec<FiberId>,
}

impl WorkInProgress {
    fn node(&self, id: FiberId) -> Option<&WorkNode> {
        self.nodes.get(&id)
    }
}

pub struct ComponentRuntime {
    nodes: FiberArena,
    current: FiberId,
    root_render: ComponentRender,
    work_in_progress: Option<WorkInProgress>,
    scheduler: Scheduler,
    hooks: FxHashMap<FiberId, HookStorage>,
    root_widget: NodeId,
    budget: Duration,

    component_registry: ComponentRegistry,
}

impl ComponentRuntime {
    pub fn new<I, F>(root_widget: NodeId, scheduler: Scheduler, init_components: I) -> Self
    where
        I: FnOnce(&mut ComponentRegistry) -> F,
        F: for<'a> FnMut(&mut HookContext<'a>) -> Element + 'static,
    {
        let arena = FiberArena::new();
        let current = arena.root();
        scheduler.set_root(current);
        scheduler.mark_component_dirty(current, current_update_lane());
        let mut component_registry = ComponentRegistry::default();
        let root_render = init_components(&mut component_registry);

        Self {
            nodes: arena,
            current,
            root_render: Rc::new(RefCell::new(root_render)),
            root_widget,
            work_in_progress: None,
            scheduler,
            hooks: FxHashMap::default(),
            budget: Duration::from_millis(4),
            component_registry,
        }
    }

    pub fn root(&self) -> FiberId {
        self.current
    }

    pub fn root_node(&self) -> &Node {
        self.nodes.node(self.root()).unwrap()
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

    pub fn rebuild_if_needed<T: TextMeasurer>(&mut self, arena: &mut UiArena, measurer: &mut T) {
        if self.is_dirty() {
            self.flush_sync(arena, measurer);
        }
    }

    pub fn flush_sync<T: TextMeasurer>(&mut self, arena: &mut UiArena, measurer: &mut T) {
        self.scheduler.mark_starved_lanes_as_expired(now_ms());
        loop {
            if self.scheduler.pending_lanes() == NO_LANES && self.work_in_progress.is_none() {
                break;
            }

            self.ensure_work();
            while self
                .work_in_progress
                .as_ref()
                .is_some_and(|work| work.next_work.is_some())
            {
                self.perform_unit_of_work(measurer);
            }
            self.commit_finished_work(arena);
        }
    }

    fn perform_unit_of_work<T: TextMeasurer>(&mut self, measurer: &mut T) {
        let Some(id) = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.next_work)
        else {
            return;
        };

        if let Some(child) = self.begin_work(id, measurer) {
            if let Some(work) = self.work_in_progress.as_mut() {
                work.next_work = Some(child);
            }
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

            let parent = self
                .work_in_progress
                .as_ref()
                .and_then(|work| work.nodes.get(&current))
                .and_then(|node| node.parent);

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

    fn begin_work<T: TextMeasurer>(&mut self, id: FiberId, measurer: &mut T) -> Option<FiberId> {
        if self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.node(id))
            .is_some_and(|node| node.began)
        {
            return self.first_child_needing_work(id);
        }

        let (tag, should_render, should_reconcile_pending, render_lanes) = {
            let work = self.work_in_progress.as_ref().expect("work missing");
            let node = work.node(id).expect("work node missing");
            (
                node.tag,
                node.effect != EffectTag::None
                    || includes_some_lane(node.lanes, work.render_lanes)
                    || node.current.is_none()
                    || node.component.is_some(),
                node.pending_children.is_some(),
                work.render_lanes,
            )
        };

        macro_rules! cx {
            ($id: ident) => {{
                let storage = self.hooks.entry($id).or_default();
                let cx = HookContext::new(storage, $id, self.scheduler.clone(), render_lanes);
                cx
            }};
        }

        match tag {
            FiberTag::Root => {
                if should_render {
                    let render = self.root_render.clone();
                    let mut cx = cx!(id);
                    let element = (render.borrow_mut())(&mut cx);
                    self.reconcile_children(id, [element], measurer);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Component => {
                if should_render {
                    let render = self
                        .work_in_progress
                        .as_ref()
                        .and_then(|work| work.node(id))
                        .and_then(|node| node.component.as_ref())
                        .map(|component| component.render.clone())
                        .or_else(|| {
                            self.nodes
                                .node(id)
                                .and_then(|node| node.component.as_ref())
                                .map(|component| component.render.clone())
                        })
                        .expect("component fiber missing render function");
                    let mut cx = cx!(id);
                    let props = self
                        .work_in_progress
                        .as_ref()
                        .and_then(|work| work.node(id))
                        .and_then(|node| node.component.as_ref())
                        .map(|component| component.props.clone())
                        .or_else(|| {
                            self.nodes
                                .node(id)
                                .and_then(|node| node.component.as_ref())
                                .map(|component| component.props.clone())
                        })
                        .expect("component fiber missing props");
                    let render = self.component_registry.get(render);
                    let element = render.call(&mut cx, props.as_ref());
                    self.reconcile_children(id, [element], measurer);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Host(_) => {
                if should_reconcile_pending {
                    let children = self
                        .work_in_progress
                        .as_mut()
                        .and_then(|w| w.nodes.get_mut(&id))
                        .and_then(|node| node.pending_children.take())
                        .unwrap_or_default();
                    self.reconcile_children(id, children, measurer);
                } else {
                    self.clone_current_children(id);
                }
            }
        }

        if let Some(work) = self.work_in_progress.as_mut() {
            if let Some(node) = work.nodes.get_mut(&id) {
                node.began = true;
            }
        }

        self.first_child_needing_work(id)
    }

    fn reconcile_children<I, T: TextMeasurer>(
        &mut self,
        parent: FiberId,
        new_children: I,
        measurer: &mut T,
    ) where
        I: IntoIterator<Item = Element>,
    {
        let old_children = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.node(parent))
            .and_then(|node| node.current)
            .and_then(|id| self.nodes.node(id))
            .map(|node| {
                node.children(&self.nodes)
                    .map(|child| child.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let render_lanes = self
            .work_in_progress
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(NO_LANES);

        let mut used = vec![false; old_children.len()];
        let mut next_children = SmallVec::with_capacity(20);

        for (position, element) in new_children.into_iter().enumerate() {
            let prepared = self.prepare_element(element, measurer);
            let matched =
                find_reusable_child(&self.nodes, &old_children, &used, &prepared, position);
            let (id, current, effect) = if let Some(old_index) = matched {
                used[old_index] = true;
                let current = old_children[old_index];
                let current_node = self.nodes.node(current).expect("matched fiber missing");
                let effect = if prepared_needs_update(current_node, &prepared) {
                    EffectTag::Update
                } else {
                    EffectTag::None
                };
                (current, Some(current), effect)
            } else {
                (self.nodes.new_id(), None, EffectTag::Placement)
            };

            let (lanes, child_lanes) = current
                .map(|current| {
                    let current_node = self.nodes.node(current).unwrap();
                    (
                        self.scheduler.component_lanes(current) & render_lanes,
                        child_tree_lanes(&self.nodes, current_node, &self.scheduler, render_lanes),
                    )
                })
                .unwrap_or((NO_LANES, NO_LANES));

            let node = WorkNode::from_prepared(
                &self.nodes,
                id,
                parent,
                position,
                prepared,
                current,
                effect,
                lanes,
                child_lanes,
            );
            self.work_in_progress
                .as_mut()
                .expect("work missing")
                .nodes
                .insert(id, node);
            next_children.push(id);
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

        if let Some(work) = self.work_in_progress.as_mut() {
            if let Some(node) = work.nodes.get_mut(&parent) {
                node.children = next_children;
                node.children_resolved = true;
            }
        }
    }

    fn prepare_element<T: TextMeasurer>(
        &self,
        element: Element,
        measurer: &mut T,
    ) -> PreparedElement {
        let key = element.key();
        match element {
            Element::Component(component) => self.prepare_component_element(component, key),
            element => {
                let props_hash = element.props_hash();
                let style = element.style(measurer);
                let tag = FiberTag::Host(element.node_type().expect("host element missing type"));
                let parts = element.into_parts();
                PreparedElement {
                    key,
                    tag,
                    pending: PreparedPending::Host {
                        widget: parts.widget,
                        event_handlers: parts.event_handlers,
                        style,
                        props_hash,
                        children: parts.children,
                    },
                }
            }
        }
    }

    fn prepare_component_element(
        &self,
        component: crate::widgets::ComponentElement,
        key: Option<Key>,
    ) -> PreparedElement {
        PreparedElement {
            key: key.clone(),
            tag: FiberTag::Component,
            pending: PreparedPending::Component {
                key,
                render: component.render,
                props_hash: component.props_hash,
                props: component.props,
            },
        }
    }

    fn commit_finished_work(&mut self, arena: &mut UiArena) {
        let Some(mut work) = self.work_in_progress.take() else {
            return;
        };
        if work.next_work.is_some() {
            self.work_in_progress = Some(work);
            return;
        }

        let deletions = std::mem::take(&mut work.deletions);
        for deletion in deletions {
            self.commit_deletion(deletion, arena, true);
        }

        self.commit_work_node(work.root, self.root_widget, arena, &mut work);
        let next_current = Self::freeze_work_tree(&mut self.nodes, work.root, &mut work);
        self.current = next_current;
        self.sync_host_children(arena);
        self.scheduler.mark_render_finished(work.render_lanes);
    }

    fn freeze_work_tree(nodes: &mut FiberArena, id: FiberId, work: &mut WorkInProgress) -> FiberId {
        let node = work.nodes.remove(&id).expect("freeze missing work node");

        if node.effect == EffectTag::None
            && node.lanes == NO_LANES
            && node.child_lanes == NO_LANES
            && node.current.is_some()
            && !node.children_resolved
        {
            return node.current.unwrap();
        }

        let children: Vec<_> = if !node.children_resolved {
            let current_node = node.current.and_then(|n| nodes.node(n));
            current_node
                .map(|c| c.children(&nodes))
                .map(|n| n.into_iter().map(|n| n.id).collect())
                .unwrap_or_default()
        } else {
            node.children
                .iter()
                .map(|child| Self::freeze_work_tree(nodes, *child, work))
                .collect()
        };

        let current_node = node.current.and_then(|n| nodes.node(n));

        let host = if matches!(node.tag, FiberTag::Host(_)) {
            let current_host = current_node.and_then(|c| c.host.as_ref());
            let node_host_work = node.host_work;

            node_host_work
                .map(|host| HostState {
                    node_id: node.host_node,
                    widget: host.widget,
                    taffy_node: current_host.and_then(|host| host.taffy_node),
                    style: host.style,
                    layout: current_host.map(|host| host.layout).unwrap_or_default(),
                    previous_layout: current_host
                        .map(|host| host.previous_layout)
                        .unwrap_or_default(),
                    paint_cache: Vec::new(),
                    props_hash: host.props_hash,
                })
                .or_else(|| {
                    let host_node = current_host.unwrap();
                    Some(HostState {
                        node_id: node.host_node,
                        widget: host_node.widget.clone(),
                        taffy_node: current_host.and_then(|host| host.taffy_node),
                        style: host_node.style.clone(),
                        layout: current_host.map(|host| host.layout).unwrap_or_default(),
                        previous_layout: current_host
                            .map(|host| host.previous_layout)
                            .unwrap_or_default(),
                        paint_cache: Vec::new(),
                        props_hash: host_node.props_hash,
                    })
                })
        } else {
            None
        };

        let component = node.component.as_ref().map(|component| ComponentState {
            key: component.key.clone(),
            render: component.render,
            props_hash: component.props_hash,
            props: component.props.clone(),
        });

        let frozen = Node {
            id: node.id,
            parent: node.parent,
            child: None,
            sibling: None,
            key: node.key.clone(),
            position: node.position,
            tag: node.tag,
            effect: EffectTag::None,
            dirty: DirtyFlags::empty(),
            subtree_dirty: DirtyFlags::empty(),
            pending_props: None,
            pending_children: None,
            memoized_props_hash: match (&host, &component) {
                (Some(host), _) => host.props_hash,
                (_, Some(component)) => component.props_hash,
                _ => 0,
            },
            host,
            component,
        };

        if let Some(existing) = nodes.node_mut(node.id) {
            *existing = frozen;
        } else {
            nodes.insert_node(node.id, frozen);
        }
        nodes.set_children(node.id, children);
        node.id
    }

    fn commit_deletion(&mut self, id: FiberId, arena: &mut UiArena, remove_host: bool) {
        let children = self.nodes.children(id);
        let host_node = self
            .nodes
            .node(id)
            .and_then(|node| node.host.as_ref())
            .and_then(|host| host.node_id);
        self.scheduler.mark_unmounted(id);
        self.hooks.remove(&id);

        if remove_host {
            if let Some(host_node) = host_node {
                arena.remove_subtree(host_node);
                for child in children {
                    self.commit_deletion(child, arena, false);
                }
                self.nodes.remove_node(id);
                return;
            }
        }

        for child in children {
            self.commit_deletion(child, arena, remove_host);
        }
        self.nodes.remove_node(id);
    }

    fn commit_work_node(
        &mut self,
        id: FiberId,
        parent_host: NodeId,
        arena: &mut UiArena,
        work: &mut WorkInProgress,
    ) {
        let tag = work.nodes.get(&id).map(|node| node.tag);
        let mut child_parent_host = parent_host;

        if matches!(tag, Some(FiberTag::Host(_))) {
            let effect = work.nodes[&id].effect;
            match effect {
                EffectTag::Placement => {
                    let key = work.nodes[&id].key.clone();
                    let host = work
                        .nodes
                        .get_mut(&id)
                        .and_then(|node| node.host_work.as_mut())
                        .expect("host placement missing host work");
                    let node_id = arena.insert_node(
                        parent_host,
                        key,
                        host.props_hash,
                        host.style.clone(),
                        host.widget
                            .as_ref()
                            .expect("host widget already committed")
                            .clone(),
                        host.event_handlers
                            .take()
                            .expect("host event handlers already committed"),
                    );
                    work.nodes.get_mut(&id).unwrap().host_node = Some(node_id);
                    child_parent_host = node_id;
                }
                EffectTag::Update => {
                    let node_id = work.nodes[&id]
                        .host_node
                        .expect("host update missing node id");
                    let key = work.nodes[&id].key.clone();
                    let host = work
                        .nodes
                        .get_mut(&id)
                        .and_then(|node| node.host_work.as_mut())
                        .expect("host update missing host work");
                    host.widget = Some(
                        arena.update_widget_node_from_parts(
                            node_id,
                            key,
                            host.props_hash,
                            host.style.clone(),
                            host.widget
                                .as_ref()
                                .expect("host widget already committed")
                                .clone(),
                            host.event_handlers
                                .take()
                                .expect("host event handlers already committed"),
                        ),
                    );
                    child_parent_host = node_id;
                }
                EffectTag::None => {
                    child_parent_host = work.nodes[&id]
                        .host_node
                        .expect("clean host missing node id");
                }
            }
        }

        let children = work
            .nodes
            .get(&id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        self.scheduler.mark_mounted(id);
        for child in children {
            self.commit_work_node(child, child_parent_host, arena, work);
        }
    }

    fn clone_current_children(&mut self, parent: FiberId) {
        let Some(current_children) = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.nodes.get(&parent))
            .and_then(|node| node.current)
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
            self.work_in_progress
                .as_mut()
                .expect("work missing")
                .nodes
                .insert(
                    current,
                    WorkNode::from_current(
                        current_node,
                        Some(parent),
                        current_node.children(&self.nodes).map(|child| child.id),
                        position,
                        lanes,
                        child_lanes,
                    ),
                );
            children.push(current);
        }

        if let Some(work) = self.work_in_progress.as_mut() {
            if let Some(node) = work.nodes.get_mut(&parent) {
                node.children = children;
                node.children_resolved = true;
            }
        }
    }

    fn first_child_needing_work(&self, parent: FiberId) -> Option<FiberId> {
        let work = self.work_in_progress.as_ref()?;
        work.node(parent)?.children.iter().copied().find(|child| {
            work.node(*child)
                .is_some_and(|node| node.needs_work(work.render_lanes))
        })
    }

    fn complete_work(&mut self, _id: FiberId) {}

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

    fn create_work_in_progress(&self, render_lanes: Lanes) -> WorkInProgress {
        let mut marks = FxHashMap::default();
        self.collect_lane_marks(self.root_node(), render_lanes, &mut marks);
        let mut nodes = FxHashMap::default();
        let (lanes, child_lanes) = marks.get(&self.root()).copied().unwrap_or_default();
        let current_node = self.nodes.node(self.current).unwrap();

        nodes.insert(
            self.root(),
            WorkNode::from_current(
                current_node,
                None,
                current_node.children(&self.nodes).map(|child| child.id),
                0,
                lanes,
                child_lanes,
            ),
        );

        WorkInProgress {
            nodes,
            root: self.root(),
            next_work: Some(self.root()),
            render_lanes,
            deletions: Vec::new(),
        }
    }

    fn discard_uncommitted_work(&mut self) {
        let Some(work) = self.work_in_progress.take() else {
            return;
        };

        for (id, node) in work.nodes {
            if node.current.is_none() {
                self.hooks.remove(&id);
                self.nodes.remove_id(id);
                self.scheduler.mark_unmounted(id);
            }
        }
    }

    fn collect_lane_marks(
        &self,
        node: &Node,
        render_lanes: Lanes,
        marks: &mut FxHashMap<FiberId, (Lanes, Lanes)>,
    ) -> Lanes {
        let own = self.scheduler.component_lanes(node.id) & render_lanes;
        let mut child_lanes = NO_LANES;

        for child in node.children(&self.nodes) {
            child_lanes |= self.collect_lane_marks(child, render_lanes, marks);
        }
        marks.insert(node.id, (own, child_lanes));
        own | child_lanes
    }

    fn next_sibling_needing_work(&self, id: FiberId) -> Option<FiberId> {
        let work = self.work_in_progress.as_ref()?;
        let node = work.nodes.get(&id)?;
        let parent = node.parent?;
        let siblings = &work.nodes.get(&parent)?.children;
        let index = siblings.iter().position(|child| *child == id)?;
        siblings.iter().copied().skip(index + 1).find(|sibling| {
            work.nodes
                .get(sibling)
                .is_some_and(|node| node.needs_work(work.render_lanes))
        })
    }

    fn sync_host_children(&self, arena: &mut UiArena) {
        self.sync_host_children_at(self.current, arena);
    }

    fn sync_host_children_at(&self, id: FiberId, arena: &mut UiArena) {
        let Some(node) = self.nodes.node(id) else {
            return;
        };

        if id == self.current {
            let children = self.flatten_host_children(id);
            arena.set_children(self.root_widget, children);
        } else if let Some(host_node) = node.host.as_ref().and_then(|host| host.node_id) {
            let children = self.flatten_host_children(id);
            arena.set_children(host_node, children);
        }

        for child in node.children(&self.nodes) {
            self.sync_host_children_at(child.id, arena);
        }
    }

    fn flatten_host_children(&self, parent: FiberId) -> Vec<NodeId> {
        let mut output = Vec::new();
        if let Some(parent) = self.nodes.node(parent) {
            for child in parent.children(&self.nodes) {
                self.flatten_host_child(child.id, &mut output);
            }
        }
        output
    }

    fn flatten_host_child(&self, id: FiberId, output: &mut Vec<NodeId>) {
        let Some(node) = self.nodes.node(id) else {
            return;
        };

        if let Some(host_node) = node.host.as_ref().and_then(|host| host.node_id) {
            output.push(host_node);
            return;
        }

        for child in node.children(&self.nodes) {
            self.flatten_host_child(child.id, output);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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
    }
}

fn prepared_needs_update(current: &Node, prepared: &PreparedElement) -> bool {
    if current.tag != prepared.tag || current.key != prepared.key {
        return true;
    }

    match (&current.host, &current.component, &prepared.pending) {
        (Some(_), _, PreparedPending::Host { .. }) => true,
        (
            _,
            Some(component),
            PreparedPending::Component {
                key,
                render,
                props_hash,
                ..
            },
        ) => {
            component.render != *render
                || component.key != *key
                || component.props_hash != *props_hash
        }
        _ => true,
    }
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

// #[cfg(test)]
// mod tests {
//     use std::cell::{Cell, RefCell};
//     use std::convert::Infallible;
//     use std::rc::Rc;

//     use crate::prelude::*;
//     use crate::widgets::{ButtonWidget, LabelWidget, TextWidget};

//     fn click(app: &mut App, node: NodeId) {
//         let rect = app.arena().node(node).unwrap().layout;
//         let point = Point::new(rect.x + 2.0, rect.y + 2.0);
//         app.dispatch_event(Event::PointerDown {
//             position: point,
//             button: PointerButton::Primary,
//         });
//         app.dispatch_event(Event::PointerUp {
//             position: point,
//             button: PointerButton::Primary,
//         });
//     }

//     fn fill_color_for_node(app: &App, backend: &MockRenderBackend, node: NodeId) -> Option<Color> {
//         let node_rect = app.arena().node(node).unwrap().layout;
//         backend
//             .last_commands
//             .iter()
//             .find_map(|command| match command {
//                 PaintCommand::FillRect { rect, color } if *rect == node_rect => Some(*color),
//                 _ => None,
//             })
//     }

//     fn label_text(app: &App, node: NodeId) -> String {
//         app.arena().node(node).unwrap().widget.with(|widget| {
//             widget
//                 .as_any()
//                 .downcast_ref::<LabelWidget>()
//                 .expect("node is not a label")
//                 .text
//                 .clone()
//         })
//     }

//     fn button_text(app: &App, node: NodeId) -> String {
//         app.arena().node(node).unwrap().widget.with(|widget| {
//             widget
//                 .as_any()
//                 .downcast_ref::<ButtonWidget>()
//                 .expect("node is not a button")
//                 .text
//                 .clone()
//         })
//     }

//     fn text_props(app: &App, node: NodeId) -> TextProps {
//         app.arena().node(node).unwrap().widget.with(|widget| {
//             widget
//                 .as_any()
//                 .downcast_ref::<TextWidget>()
//                 .expect("node is not text")
//                 .props
//                 .clone()
//         })
//     }

//     #[derive(Default)]
//     struct NonPresentingBackend {
//         paints: usize,
//     }

//     impl RenderBackend for NonPresentingBackend {
//         type Error = Infallible;

//         fn begin_frame(&mut self, _: Size) -> Result<(), Self::Error> {
//             Ok(())
//         }

//         fn paint(&mut self, _: &[PaintCommand], _: &DamageRegion) -> Result<(), Self::Error> {
//             self.paints += 1;
//             Ok(())
//         }

//         fn end_frame(&mut self) -> Result<(), Self::Error> {
//             Ok(())
//         }

//         fn did_present(&self) -> bool {
//             false
//         }
//     }

//     #[test]
//     fn initial_root_render_builds_host_tree() {
//         let mut app = app(|_| column().child(label("hello")).into());
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 100.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let column_id = app.arena().children(root)[0];
//         let label_id = app.arena().children(column_id)[0];
//         assert_eq!(label_text(&app, label_id), "hello");
//     }

//     #[test]
//     fn all_host_builders_register_click_handlers() {
//         let seen = Rc::new(Cell::new(0u8));
//         let mut arena = UiArena::new();
//         let root = arena.root();
//         arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 100.0, 140.0);

//         let mark = |bit| -> ClickEventHandler {
//             let seen = seen.clone();
//             Box::new(move |_| {
//                 seen.set(seen.get() | bit);
//                 EventResult::Consumed
//             })
//         };
//         let elements: Vec<Element> = vec![
//             label("label").on_click(mark(1)).into(),
//             button("button").on_click(mark(2)).into(),
//             container()
//                 .size(Size::new(24.0, 24.0))
//                 .on_click(mark(4))
//                 .into(),
//             row().on_click(mark(8)).into(),
//             column().on_click(mark(16)).into(),
//         ];

//         let mut hosts = Vec::new();
//         for (index, element) in elements.into_iter().enumerate() {
//             let parts = element.into_parts();
//             let host = arena.insert_node(
//                 root,
//                 None,
//                 0,
//                 taffy::prelude::Style::default(),
//                 parts.widget,
//                 parts.event_handlers,
//             );
//             arena.node_mut(host).unwrap().layout = Rect::new(0.0, index as f32 * 28.0, 24.0, 24.0);
//             hosts.push(host);
//         }

//         for host in hosts {
//             let rect = arena.node(host).unwrap().layout;
//             let point = Point::new(rect.x + 2.0, rect.y + 2.0);
//             arena.dispatch_event(&Event::PointerDown {
//                 position: point,
//                 button: PointerButton::Primary,
//             });
//             arena.dispatch_event(&Event::PointerUp {
//                 position: point,
//                 button: PointerButton::Primary,
//             });
//         }

//         assert_eq!(seen.get(), 0b1_1111);
//     }

//     #[test]
//     fn button_keeps_visual_pressed_state_while_click_uses_event_props() {
//         let clicks = Rc::new(Cell::new(0));
//         let clicks_for_app = clicks.clone();
//         let mut app = app(move |_| {
//             let clicks_for_handler = clicks_for_app.clone();
//             button("press")
//                 .on_click(move |_| {
//                     clicks_for_handler.set(clicks_for_handler.get() + 1);
//                     EventResult::Consumed
//                 })
//                 .into()
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();
//         let root = app.arena().root();
//         let button_id = app.arena().children(root)[0];
//         let rect = app.arena().node(button_id).unwrap().layout;
//         let point = Point::new(rect.x + 2.0, rect.y + 2.0);

//         app.dispatch_event(Event::PointerDown {
//             position: point,
//             button: PointerButton::Primary,
//         });
//         app.render(&mut backend).unwrap();
//         assert_eq!(
//             fill_color_for_node(&app, &backend, button_id),
//             Some(Color::BLUE_500)
//         );

//         app.dispatch_event(Event::PointerUp {
//             position: point,
//             button: PointerButton::Primary,
//         });
//         app.render(&mut backend).unwrap();
//         assert_eq!(clicks.get(), 1);
//         assert_eq!(
//             fill_color_for_node(&app, &backend, button_id),
//             Some(Color::GRAY_300)
//         );
//     }

//     #[test]
//     fn text_widget_builds_single_style_text_props() {
//         let mut app = app(|_| {
//             text("hello")
//                 .color(Color::BLUE_500)
//                 .font_size(20.0)
//                 .font_weight(FontWeight::Bold)
//                 .into()
//         });

//         app.resize(Size::new(200.0, 120.0));
//         let root = app.arena().root();
//         app.arena_mut().update_tree(root, Size::new(200.0, 120.0));

//         let text_id = app.arena().children(root)[0];
//         let props = text_props(&app, text_id);
//         assert_eq!(props.text.as_str(), "hello");
//         assert_eq!(props.style.color, Color::BLUE_500);
//         assert_eq!(props.style.font_size, 20.0);
//         assert_eq!(props.style.font_weight, FontWeight::Bold);
//     }

//     #[test]
//     fn render_without_present_keeps_damage_for_retry() {
//         let mut app = app(|_| container().child(label("hello")).into());
//         let mut blocked = NonPresentingBackend::default();

//         app.resize(Size::new(200.0, 100.0));
//         app.render(&mut blocked).unwrap();

//         assert_eq!(blocked.paints, 1);
//         assert!(app.is_dirty());

//         let mut backend = MockRenderBackend::default();
//         app.render(&mut backend).unwrap();

//         assert_eq!(backend.frames, 1);
//         assert!(!backend.last_damage.is_empty());
//         assert!(!app.is_dirty());
//     }

//     #[test]
//     fn component_state_rerenders_only_owner_component() {
//         const CHILD: ComponentType = ComponentType::new("xui::tests::owner_child");

//         let root_renders = Rc::new(Cell::new(0));
//         let child_renders = Rc::new(Cell::new(0));
//         let root_renders_for_app = root_renders.clone();
//         let child_renders_for_registry = child_renders.clone();

//         let mut app = App::with_component_registry(move |registry| {
//             let child_renders_for_component = child_renders_for_registry.clone();
//             registry.register(CHILD, move |cx| {
//                 child_renders_for_component.set(child_renders_for_component.get() + 1);
//                 let count = cx.use_state(|| 0);
//                 let count_for_click = count.clone();
//                 button(format!("count: {}", count.get()))
//                     .on_click(move |_| {
//                         count_for_click.set(count_for_click.get() + 1);
//                         EventResult::Consumed
//                     })
//                     .into()
//             });

//             move |_| {
//                 root_renders_for_app.set(root_renders_for_app.get() + 1);
//                 column().child(component(CHILD)).into()
//             }
//         });

//         let mut backend = MockRenderBackend::default();
//         app.resize(Size::new(240.0, 120.0));
//         app.render(&mut backend).unwrap();

//         assert_eq!(root_renders.get(), 1);
//         assert_eq!(child_renders.get(), 1);

//         let root = app.arena().root();
//         let column_id = app.arena().children(root)[0];
//         let button_id = app.arena().children(column_id)[0];
//         click(&mut app, button_id);
//         app.render(&mut backend).unwrap();

//         assert_eq!(root_renders.get(), 1);
//         assert_eq!(child_renders.get(), 2);
//         assert_eq!(button_text(&app, button_id), "count: 1");
//     }

//     #[test]
//     fn component_update_reports_local_damage_to_backend() {
//         let blue = Rc::new(Cell::new(false));
//         let blue_for_app = blue.clone();
//         let mut app = app(move |_| {
//             let changing = if blue_for_app.get() {
//                 container()
//                     .size(Size::new(48.0, 24.0))
//                     .background(Color::BLUE_500)
//             } else {
//                 container()
//                     .size(Size::new(48.0, 24.0))
//                     .background(Color::GRAY_100)
//             };
//             row()
//                 .child(changing)
//                 .child(
//                     container()
//                         .size(Size::new(20.0, 24.0))
//                         .background(Color::GRAY_300),
//                 )
//                 .into()
//         });

//         let mut backend = MockRenderBackend::default();
//         app.resize(Size::new(240.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let root_rect = app.arena().node(root).unwrap().layout;
//         let row_id = app.arena().children(root)[0];
//         let container_id = app.arena().children(row_id)[0];
//         let container_rect = app.arena().node(container_id).unwrap().layout;

//         app.arena_mut().repaint_passes = 0;
//         blue.set(true);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         assert_eq!(app.arena().repaint_passes, 1);
//         assert_eq!(backend.last_damage.bounds(), Some(container_rect));
//         assert_ne!(backend.last_damage.bounds(), Some(root_rect));
//     }

//     #[test]
//     fn keyed_component_reorder_preserves_hook_state() {
//         const FIRST: ComponentType = ComponentType::new("xui::tests::keyed_first");
//         const SECOND: ComponentType = ComponentType::new("xui::tests::keyed_second");

//         let reversed = Rc::new(Cell::new(false));
//         let reversed_for_app = reversed.clone();

//         let mut app = App::with_component_registry(move |registry| {
//             registry.register(FIRST, |cx| {
//                 let value = cx.use_state(|| "A".to_owned());
//                 label(value.get()).into()
//             });
//             registry.register(SECOND, |cx| {
//                 let value = cx.use_state(|| "B".to_owned());
//                 label(value.get()).into()
//             });

//             move |_| {
//                 let first = component(FIRST).key("a");
//                 let second = component(SECOND).key("b");

//                 if reversed_for_app.get() {
//                     row().child(second).child(first).into()
//                 } else {
//                     row().child(first).child(second).into()
//                 }
//             }
//         });

//         let mut backend = MockRenderBackend::default();
//         app.resize(Size::new(240.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let row_id = app.arena().children(root)[0];
//         let first_before = app.arena().children(row_id)[0];
//         let second_before = app.arena().children(row_id)[1];

//         reversed.set(true);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         let first_after = app.arena().children(row_id)[0];
//         let second_after = app.arena().children(row_id)[1];
//         assert_eq!(first_after, second_before);
//         assert_eq!(second_after, first_before);
//         assert_eq!(label_text(&app, first_after), "B");
//         assert_eq!(label_text(&app, second_after), "A");
//     }

//     #[test]
//     fn component_child_participates_in_layout() {
//         const CHILD: ComponentType = ComponentType::new("xui::tests::layout_child");

//         let mut app = App::with_component_registry(|registry| {
//             registry.register(CHILD, |_| container().size(Size::new(50.0, 20.0)).into());

//             |_| {
//                 column()
//                     .child(component(CHILD))
//                     .child(container().size(Size::new(30.0, 10.0)))
//                     .into()
//             }
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 200.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let column_id = app.arena().children(root)[0];
//         let first = app.arena().children(column_id)[0];
//         let second = app.arena().children(column_id)[1];

//         assert_eq!(app.arena().node(first).unwrap().layout.width, 50.0);
//         assert_eq!(app.arena().node(first).unwrap().layout.height, 20.0);
//         assert_eq!(app.arena().node(second).unwrap().layout.y, 20.0);
//     }

//     #[test]
//     fn component_render_update_refreshes_event_handler() {
//         const CAPTURE: ComponentType = ComponentType::new("xui::tests::capture_button");

//         let value = Rc::new(Cell::new(1));
//         let seen = Rc::new(Cell::new(0));
//         let value_for_registry = value.clone();
//         let seen_for_registry = seen.clone();

//         let mut app = App::with_component_registry(move |registry| {
//             let value_for_component = value_for_registry.clone();
//             let seen_for_component = seen_for_registry.clone();
//             registry.register(CAPTURE, move |_| {
//                 let captured = value_for_component.get();
//                 let seen_for_click = seen_for_component.clone();
//                 button("capture")
//                     .on_click(move |_| {
//                         seen_for_click.set(captured);
//                         EventResult::Consumed
//                     })
//                     .into()
//             });

//             |_| component(CAPTURE).into()
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         value.set(2);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let button_id = app.arena().children(root)[0];
//         click(&mut app, button_id);

//         assert_eq!(seen.get(), 2);
//     }

//     #[derive(Hash)]
//     struct TitleProps {
//         title: String,
//     }

//     #[test]
//     fn component_props_drive_registered_component_render() {
//         const TITLE: ComponentType = ComponentType::new("xui::tests::title_props");

//         let title = Rc::new(std::cell::RefCell::new("first".to_owned()));
//         let title_for_app = title.clone();
//         let mut app = App::with_component_registry(move |registry| {
//             registry.register_with_props::<TitleProps, _>(TITLE, |_, props| {
//                 label(props.title.clone()).into()
//             });

//             move |_| {
//                 component(TITLE)
//                     .props(TitleProps {
//                         title: title_for_app.borrow().clone(),
//                     })
//                     .into()
//             }
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let label_id = app.arena().children(root)[0];
//         assert_eq!(label_text(&app, label_id), "first");

//         *title.borrow_mut() = "second".to_owned();
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         assert_eq!(label_text(&app, label_id), "second");
//     }

//     #[derive(Hash)]
//     struct CounterProps {
//         prefix: String,
//     }

//     #[test]
//     fn component_state_update_rerenders_with_current_props() {
//         const COUNTER: ComponentType = ComponentType::new("xui::tests::counter_props");

//         let mut app = App::with_component_registry(|registry| {
//             registry.register_with_props::<CounterProps, _>(COUNTER, |cx, props| {
//                 let count = cx.use_state(|| 0);
//                 let count_for_click = count.clone();
//                 button(format!("{}: {}", props.prefix, count.get()))
//                     .on_click(move |_| {
//                         count_for_click.set(count_for_click.get() + 1);
//                         EventResult::Consumed
//                     })
//                     .into()
//             });

//             |_| {
//                 component(COUNTER)
//                     .props(CounterProps {
//                         prefix: "count".to_owned(),
//                     })
//                     .into()
//             }
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let button_id = app.arena().children(root)[0];
//         click(&mut app, button_id);
//         app.render(&mut backend).unwrap();

//         assert_eq!(button_text(&app, button_id), "count: 1");
//     }

//     struct ClickProps {
//         count: i32,
//         on_click: Rc<RefCell<ClickEventHandler>>,
//     }

//     #[test]
//     fn component_props_can_explicitly_forward_event_handler_to_host() {
//         const CHILD: ComponentType = ComponentType::new("xui::tests::event_props_child");

//         let mut app = App::with_component_registry(|registry| {
//             registry.register_with_props::<ClickProps, _>(CHILD, |_, props| {
//                 let on_click = props.on_click.clone();
//                 button(format!("count: {}", props.count))
//                     .on_click(move |cx| on_click.borrow_mut()(cx))
//                     .into()
//             });

//             move |cx| {
//                 let count = cx.use_state(|| 0);
//                 let count_for_click = count.clone();
//                 let current = count.get();
//                 let on_click: ClickEventHandler = Box::new(move |_| {
//                     count_for_click.set(count_for_click.get() + 1);
//                     EventResult::Consumed
//                 });

//                 component(CHILD)
//                     .props_with_hash(
//                         ClickProps {
//                             count: current,
//                             on_click: Rc::new(RefCell::new(on_click)),
//                         },
//                         current as u64,
//                     )
//                     .into()
//             }
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let button_id = app.arena().children(root)[0];
//         click(&mut app, button_id);
//         app.render(&mut backend).unwrap();

//         assert_eq!(button_text(&app, button_id), "count: 1");
//     }

//     #[test]
//     fn same_key_different_component_type_replaces_subtree() {
//         const FIRST: ComponentType = ComponentType::new("xui::tests::same_key_first");
//         const SECOND: ComponentType = ComponentType::new("xui::tests::same_key_second");

//         fn first(_: &mut HookContext<'_>) -> Element {
//             label("first").into()
//         }

//         fn second(_: &mut HookContext<'_>) -> Element {
//             label("second").into()
//         }

//         let use_second = Rc::new(Cell::new(false));
//         let use_second_for_app = use_second.clone();
//         let mut app = App::with_component_registry(move |registry| {
//             registry.register(FIRST, first);
//             registry.register(SECOND, second);

//             move |_| {
//                 if use_second_for_app.get() {
//                     component(SECOND).key("same").into()
//                 } else {
//                     component(FIRST).key("same").into()
//                 }
//             }
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let first_node = app.arena().children(root)[0];

//         use_second.set(true);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         let second_node = app.arena().children(root)[0];
//         assert_ne!(first_node, second_node);
//         assert!(!app.arena().contains(first_node));
//         assert_eq!(label_text(&app, second_node), "second");
//     }

//     #[test]
//     fn deleting_component_removes_rendered_widget_subtree() {
//         const CHILD: ComponentType = ComponentType::new("xui::tests::delete_child");

//         let show = Rc::new(Cell::new(true));
//         let show_for_app = show.clone();
//         let mut app = App::with_component_registry(move |registry| {
//             registry.register(CHILD, |_| {
//                 container()
//                     .key("box")
//                     .child(label("child").key("child"))
//                     .into()
//             });

//             move |_| {
//                 let mut root = column();
//                 if show_for_app.get() {
//                     root = root.child(component(CHILD));
//                 }
//                 root.into()
//             }
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let column_id = app.arena().children(root)[0];
//         let box_id = app.arena().children(column_id)[0];
//         let child_id = app.arena().children(box_id)[0];

//         show.set(false);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         assert!(app.arena().children(column_id).is_empty());
//         assert!(!app.arena().contains(box_id));
//         assert!(!app.arena().contains(child_id));
//     }
// }
