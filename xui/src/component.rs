use crate::fiber::{
    ComponentDef, ComponentRegistry, ComponentType, EffectTag, FiberArena, FiberContext,
    FiberElement, FiberId, FiberTag, Node, NodeChildren,
};
use crate::font::TextI;
use crate::lanes::{Lanes, NO_LANES, current_update_lane, includes_some_lane, should_interrupt};
use crate::state::{HookContext, HookStorage, Scheduler};
use crate::tree::UiArena;
use crate::widgets::{Element, Key};
use rustc_hash::FxHashMap;
use slotmap::SlotMap;
use std::any::Any;
use std::marker::PhantomData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use taffy as tf;
use xui_interface::{Widget, WidgetKind};

pub struct FiberNode {
    node: FiberId,
}

pub struct WorkNode {
    id: FiberId,
    parent: Option<FiberId>,
    key: Option<Key>,
    tag: FiberTag,
    position: usize,
    current: Option<FiberId>,
    effect: EffectTag,
    child: Option<FiberId>,
    sibling: Option<FiberId>,
    children_resolved: bool,
    pending_children: Option<Vec<Element>>,
    lanes: Lanes,
    child_lanes: Lanes,
    began: bool,
}

impl WorkNode {
    fn from_current(
        current: &Node,
        parent: Option<FiberId>,
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
            child: current.child,
            sibling: current.sibling,
            current: Some(current.id),
            children_resolved: false,
            effect: EffectTag::None,
            pending_children: None,
            lanes,
            child_lanes,
            began: false,
        }
    }

    fn from_prepared(
        id: FiberId,
        parent: FiberId,
        position: usize,
        prepared: PreparedElement,
        current: Option<&Node>,
        effect: EffectTag,
        lanes: Lanes,
        child_lanes: Lanes,
    ) -> Self {
        let host_node = current
            .as_ref()
            .and_then(|current| current.host.as_ref().map(|host| host.node_id));
        let (host, render, pending_children) = match prepared.pending {
            PreparedPending::Host {
                kind,
                widget,
                style,
                props_hash,
                children,
            } => (
                Some(HostWork {
                    kind,
                    widget: Some(widget),
                    style,
                    props_hash,
                }),
                None,
                Some(children),
            ),
            PreparedPending::Component { render } => (None, Some(render), None),
        };

        Self {
            id,
            parent: Some(parent),
            key: prepared.key,
            position,
            tag: prepared.tag,
            current,
            host_node,
            host,
            render,
            pending_children,
            children: Vec::new(),
            children_resolved: false,
            effect,
            lanes,
            child_lanes,
            began: false,
        }
    }

    fn needs_work(&self, render_lanes: Lanes) -> bool {
        self.effect != EffectTag::None
            || self.current.is_none()
            || self.pending_children.is_some()
            || includes_some_lane(self.lanes | self.child_lanes, render_lanes)
    }

    fn children<'a>(&self, arena: &'a FiberArena) -> NodeChildren<'a, Self> {
        NodeChildren {
            arena,
            child: self.child,
            _marker: PhantomData,
        }
    }
}

impl<'a> Iterator for NodeChildren<'a, WorkNode> {
    type Item = &'a WorkNode;
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

struct PreparedElement {
    key: Option<Key>,
    tag: FiberTag,
    pending: PreparedPending,
}

enum PreparedPending {
    Host {
        kind: WidgetKind,
        widget: Box<dyn Widget>,
        style: tf::Style,
        props_hash: u64,
        children: Vec<FiberElement>,
    },
    Component {
        render: ComponentType,
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
    fn node(&self, parent: FiberId) -> Option<&WorkNode> {
        self.nodes.get(&parent)
    }
}

pub struct ComponentRuntime {
    components_registry: ComponentRegistry,
    nodes: FiberArena,
    current: FiberId,
    work_in_progress: Option<WorkInProgress>,
    scheduler: Scheduler,
    hooks: FxHashMap<FiberId, HookStorage>,
    props: FxHashMap<FiberId, Box<dyn Any>>,
    budget: Duration,
}

impl ComponentRuntime {
    pub fn new(root_widget: FiberId, scheduler: Scheduler) -> Self {
        let arena = FiberArena::new();
        let current = FiberNode { node: root_widget };
        let components_registry = ComponentRegistry::new();

        Self {
            components_registry,
            nodes: arena,
            current,
            work_in_progress: None,
            scheduler,
            hooks: FxHashMap::default(),
            budget: Duration::from_millis(4),
        }
    }

    pub fn root(&self) -> FiberId {
        self.current
    }

    pub fn root_node(&self) -> &Node {
        // self.current.node(self.root()).unwrap()
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

    pub fn flush_sync(&mut self, arena: &mut UiArena, measurer: &mut TextI) {
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

    fn perform_unit_of_work(&mut self, measurer: &mut TextI) {
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
                if let Some(work) = self.work.as_mut() {
                    work.next_work = Some(sibling);
                }
                return;
            }

            let parent = self
                .work
                .as_ref()
                .and_then(|work| work.nodes.get(&current))
                .and_then(|node| node.parent);

            match parent {
                Some(parent) => current = parent,
                None => {
                    if let Some(work) = self.work.as_mut() {
                        work.next_work = None;
                    }
                    return;
                }
            }
        }
    }

    fn begin_work(&mut self, id: FiberId, measurer: &mut TextI) -> Option<FiberId> {
        if self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.node(id))
            .is_some_and(|node| node.began)
        {
            return self.first_child_needing_work(id);
        }

        let (tag, should_render, should_reconcile_pending, pending_children) = {
            let work = self.work_in_progress.as_ref().expect("work missing");
            let node = work.node(id).expect("work node missing");
            (
                node.tag,
                node.effect != EffectTag::None
                    || includes_some_lane(node.lanes, work.render_lanes)
                    || node.current.is_none(),
                node.pending_children.is_some(),
                node.pending_children.as_ref().map(Vec::len),
            )
        };

        let render_lanes = self.work_in_progress.as_ref().unwrap().render_lanes;

        match tag {
            FiberTag::Root => {
                if should_render {
                    let render = self
                        .current_render(id)
                        .expect("root fiber missing render function");
                    self.reconcile_children(id, &[element], measurer);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Component(typ) => {
                if should_render {
                    let render = self.components_registry.get(typ);
                    let mut cx = FiberContext::new(id, self.hook_context(id, render_lanes));
                    let props = self.props.get(&id).unwrap();
                    let element = render.call(&mut cx, props);
                    self.reconcile_children(id, &[element], measurer);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Host(_) => {
                if should_reconcile_pending {
                    let children = self
                        .work
                        .as_mut()
                        .and_then(|work| work.nodes.get_mut(&id))
                        .and_then(|node| node.pending_children.take())
                        .unwrap_or_default();
                    self.reconcile_children(id, children, measurer);
                } else if pending_children == Some(0) {
                    self.reconcile_children(id, Vec::new(), measurer);
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

    fn reconcile_children(
        &mut self,
        parent: FiberId,
        new_children: &[FiberElement],
        measurer: &mut TextI,
    ) {
        let old_children = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.node(parent))
            .and_then(|node| node.current.and_then(|c| self.nodes.node(c)))
            .map(|c| c.children(&self.nodes).collect::<Vec<_>>())
            .unwrap_or_default();

        let render_lanes = self
            .work_in_progress
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(NO_LANES);

        let mut used = vec![false; old_children.len()];
        let mut next_children = Vec::with_capacity(new_children.len());

        for (position, element) in new_children.into_iter().enumerate() {
            let prepared = self.prepare_element(element, measurer);
            let matched = find_reusable_child(&old_children, &used, &prepared, position);
            let (id, current, effect) = if let Some(old_index) = matched {
                used[old_index] = true;
                let current = old_children[old_index].clone();
                let effect = if prepared_needs_update(&current, &prepared) {
                    EffectTag::Update
                } else {
                    EffectTag::None
                };
                (current.id, Some(current), effect)
            } else {
                (self.nodes.new_id(), None, EffectTag::Placement)
            };

            let (lanes, child_lanes) = current
                .as_ref()
                .map(|current| {
                    (
                        self.scheduler.component_lanes(current.id) & render_lanes,
                        child_tree_lanes(&self.nodes, current, &self.scheduler, render_lanes),
                    )
                })
                .unwrap_or((NO_LANES, NO_LANES));

            let node = WorkNode::from_prepared(
                id,
                parent,
                position,
                prepared,
                current,
                effect,
                lanes,
                child_lanes,
            );
            self.work
                .as_mut()
                .expect("work missing")
                .nodes
                .insert(id, node);
            next_children.push(id);
        }

        for (index, old_child) in old_children.into_iter().enumerate() {
            if !used[index] {
                self.work
                    .as_mut()
                    .expect("work missing")
                    .deletions
                    .push(old_child);
            }
        }

        if let Some(work) = self.work.as_mut() {
            if let Some(node) = work.nodes.get_mut(&parent) {
                node.children = next_children;
                node.children_resolved = true;
            }
        }
    }

    fn prepare_element(&self, element: &FiberElement, measurer: &mut TextI) -> PreparedElement {
        match element {
            FiberElement::Component(component) => {
                let key = component.key.clone();
                PreparedElement {
                    key,
                    tag: FiberTag::Component(component.component_type),
                    pending: PreparedPending::Component {
                        render: component.component_type,
                    },
                }
            }
            FiberElement::Host(host) => {
                let key = host.key;
                let props_hash = host.props_hash;
                let style = host.style;
                let kind = host.kind;
                let widget = host.widget;
                let children = host.children;
                PreparedElement {
                    key,
                    tag: FiberTag::Host(host.widget.node_type()),
                    pending: PreparedPending::Host {
                        kind,
                        widget,
                        style,
                        props_hash,
                        children,
                    },
                }
            }
        }
    }

    fn render_component(
        &self,
        context: &mut FiberContext,
        props: &dyn Any,
        render: &ComponentDef,
    ) -> FiberElement {
        render.call(context, props)
    }

    fn hook_context(&mut self, fiber: FiberId, lanes: Lanes) -> HookContext {
        let storage = self.hooks.entry(fiber).or_default();
        HookContext::new(storage, fiber, self.scheduler.clone(), lanes)
    }

    fn first_child_needing_work(&self, parent: FiberId) -> Option<FiberId> {
        let work = self.work_in_progress.as_ref()?;
        work.node(parent)?
            .children(&self.nodes)
            .find(|child| {
                work.node(child.id)
                    .is_some_and(|c| c.needs_work(work.render_lanes))
            })
            .map(|w| w.id)
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
            WorkNode::from_current(current_node, None, 0, lanes, child_lanes),
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

        for id in work.cursor(work.root()) {
            let node = work.node_mut(id).unwrap();
            // if node.current().is_none() {
            //     self.hooks.remove(&id);
            //     self.ids.remove(id);
            //     self.scheduler.mark_unmounted(id);
            // }
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
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn find_reusable_child(
    old_children: &[&Node],
    used: &[bool],
    prepared: &PreparedElement,
    position: usize,
) -> Option<usize> {
    if let Some(key) = prepared.key.as_ref() {
        return old_children
            .iter()
            .enumerate()
            .find(|(index, old)| {
                !used[*index] && old.key.as_ref() == Some(key) && old.tag == prepared.tag
            })
            .map(|(index, _)| index);
    }

    old_children
        .get(position)
        .filter(|old| {
            !used[position]
                && old.key.is_none()
                && old.tag == prepared.tag
                && old.position == position
        })
        .map(|_| position)
}

fn prepared_needs_update(current: &Node, prepared: &PreparedElement) -> bool {
    if current.tag != prepared.tag || current.key != prepared.key {
        return true;
    }

    match (&current.host, &prepared.pending) {
        (
            Some(host),
            PreparedPending::Host {
                kind,
                style,
                props_hash,
                ..
            },
        ) => host.props_hash != *props_hash || host.kind != *kind || host.style != *style,
        (_, PreparedPending::Component { .. }) => true,
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
