use std::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use slotmap::{SlotMap, new_key_type};
use taffy::prelude as tf;
use xui_interface::{NodeId, NodeType};

use crate::font::TextI;
use crate::lanes::{
    Lanes, NO_LANES, SYNC_LANE, current_update_lane, includes_some_lane, should_interrupt,
};
use crate::state::{HookContext, HookStorage, Scheduler};
use crate::tree::UiArena;
use crate::widgets::{Element, Key, Widget, WidgetKind};

new_key_type! {
    pub struct FiberId;
}

pub type ComponentId = FiberId;
pub type FiberElement = Element;
pub type FiberContext<'a> = HookContext<'a>;

type ComponentRender = Rc<RefCell<Box<dyn FnMut(&mut HookContext<'_>) -> Element>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FiberTag {
    Root,
    Host(NodeType),
    Component(TypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTag {
    None,
    Placement,
    Update,
}

pub struct FiberNode {
    pub id: FiberId,
    pub parent: Option<FiberId>,
    pub key: Option<Key>,
    pub position: usize,
    pub tag: FiberTag,
    pub host: Option<HostSnapshot>,
    component: Option<ComponentSnapshot>,
    pub children: Vec<Rc<FiberNode>>,
}

pub struct HostSnapshot {
    pub node_id: NodeId,
    pub kind: WidgetKind,
    pub style: tf::Style,
    pub props_hash: u64,
}

struct ComponentSnapshot {
    render: ComponentRender,
}

pub enum WorkStatus {
    Idle,
    Yielded,
    Committed,
}

pub struct FiberRuntime {
    ids: SlotMap<FiberId, ()>,
    current: Rc<FiberNode>,
    root: FiberId,
    root_widget: NodeId,
    scheduler: Scheduler,
    hooks: HashMap<FiberId, HookStorage>,
    work: Option<WorkInProgress>,
    budget: Duration,
}

struct WorkInProgress {
    nodes: HashMap<FiberId, WorkNode>,
    root: FiberId,
    next_work: Option<FiberId>,
    render_lanes: Lanes,
    deletions: Vec<Rc<FiberNode>>,
}

struct WorkNode {
    id: FiberId,
    parent: Option<FiberId>,
    key: Option<Key>,
    position: usize,
    tag: FiberTag,
    current: Option<Rc<FiberNode>>,
    host_node: Option<NodeId>,
    host: Option<HostWork>,
    render: Option<ComponentRender>,
    pending_children: Option<Vec<Element>>,
    children: Vec<FiberId>,
    children_resolved: bool,
    effect: EffectTag,
    lanes: Lanes,
    child_lanes: Lanes,
    began: bool,
}

struct HostWork {
    kind: WidgetKind,
    widget: Option<Box<dyn Widget>>,
    style: tf::Style,
    props_hash: u64,
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
        children: Vec<Element>,
    },
    Component {
        render: ComponentRender,
    },
}

impl FiberRuntime {
    pub fn new(
        root_widget: NodeId,
        scheduler: Scheduler,
        root_component: impl FnMut(&mut HookContext<'_>) -> Element + 'static,
    ) -> Self {
        let mut ids = SlotMap::with_key();
        let root = ids.insert(());
        let root_render: ComponentRender = Rc::new(RefCell::new(Box::new(root_component)));
        let current = Rc::new(FiberNode {
            id: root,
            parent: None,
            key: None,
            position: 0,
            tag: FiberTag::Root,
            host: Some(HostSnapshot {
                node_id: root_widget,
                kind: WidgetKind::Root,
                style: tf::Style::default(),
                props_hash: 0,
            }),
            component: Some(ComponentSnapshot {
                render: root_render,
            }),
            children: Vec::new(),
        });

        scheduler.set_root(root);
        scheduler.mark_component_dirty(root, SYNC_LANE);

        Self {
            ids,
            current,
            root,
            root_widget,
            scheduler,
            hooks: HashMap::new(),
            work: None,
            budget: Duration::from_millis(4),
        }
    }

    pub fn root(&self) -> FiberId {
        self.root
    }

    pub fn current(&self) -> Rc<FiberNode> {
        self.current.clone()
    }

    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    pub fn is_dirty(&self) -> bool {
        self.work.is_some() || self.scheduler.is_dirty()
    }

    pub fn mark_root_dirty(&self) {
        self.scheduler.mark_root_dirty(current_update_lane());
    }

    pub fn flush_sync(&mut self, arena: &mut UiArena, measurer: &mut TextI) {
        self.scheduler.mark_starved_lanes_as_expired(now_ms());
        loop {
            if self.scheduler.pending_lanes() == NO_LANES && self.work.is_none() {
                break;
            }
            self.ensure_work();
            while self
                .work
                .as_ref()
                .is_some_and(|work| work.next_work.is_some())
            {
                self.perform_unit_of_work(measurer);
            }
            self.commit_finished_work(arena);
        }
    }

    pub fn perform_work(
        &mut self,
        arena: &mut UiArena,
        measurer: &mut TextI,
        budget: Option<Duration>,
    ) -> WorkStatus {
        self.scheduler.mark_starved_lanes_as_expired(now_ms());
        if self.scheduler.pending_lanes() == NO_LANES && self.work.is_none() {
            return WorkStatus::Idle;
        }

        self.ensure_work();
        let deadline = budget.map(|budget| Instant::now() + budget);

        while self
            .work
            .as_ref()
            .is_some_and(|work| work.next_work.is_some())
        {
            self.perform_unit_of_work(measurer);
            if let Some(deadline) = deadline {
                let Some(work) = self.work.as_ref() else {
                    break;
                };
                if work.next_work.is_some() && Instant::now() >= deadline {
                    return WorkStatus::Yielded;
                }
            }
        }

        if self.work.is_some() {
            self.commit_finished_work(arena);
            WorkStatus::Committed
        } else {
            WorkStatus::Idle
        }
    }

    pub fn perform_budgeted_work(
        &mut self,
        arena: &mut UiArena,
        measurer: &mut TextI,
    ) -> WorkStatus {
        self.perform_work(arena, measurer, Some(self.budget))
    }

    fn ensure_work(&mut self) {
        let wip_lanes = self
            .work
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(NO_LANES);
        let next_lanes = self.scheduler.get_next_lanes(wip_lanes);
        if next_lanes == NO_LANES {
            return;
        }

        if self.work.is_none() || should_interrupt(wip_lanes, next_lanes) {
            if self.work.is_some() {
                self.discard_uncommitted_work();
            }
            self.work = Some(self.create_work_in_progress(next_lanes));
        }
    }

    fn discard_uncommitted_work(&mut self) {
        let Some(work) = self.work.take() else {
            return;
        };

        for (id, node) in work.nodes {
            if node.current.is_none() {
                self.hooks.remove(&id);
                self.ids.remove(id);
                self.scheduler.mark_unmounted(id);
            }
        }
    }

    fn create_work_in_progress(&self, render_lanes: Lanes) -> WorkInProgress {
        let mut marks = HashMap::new();
        self.collect_lane_marks(&self.current, render_lanes, &mut marks);

        let mut nodes = HashMap::new();
        let (lanes, child_lanes) = marks.get(&self.root).copied().unwrap_or_default();
        nodes.insert(
            self.root,
            WorkNode::from_current(self.current.clone(), None, 0, lanes, child_lanes),
        );

        WorkInProgress {
            nodes,
            root: self.root,
            next_work: Some(self.root),
            render_lanes,
            deletions: Vec::new(),
        }
    }

    fn collect_lane_marks(
        &self,
        node: &Rc<FiberNode>,
        render_lanes: Lanes,
        marks: &mut HashMap<FiberId, (Lanes, Lanes)>,
    ) -> Lanes {
        let own = self.scheduler.component_lanes(node.id) & render_lanes;
        let mut child_lanes = NO_LANES;
        for child in &node.children {
            child_lanes |= self.collect_lane_marks(child, render_lanes, marks);
        }
        marks.insert(node.id, (own, child_lanes));
        own | child_lanes
    }

    fn perform_unit_of_work(&mut self, measurer: &mut TextI) {
        let Some(id) = self.work.as_ref().and_then(|work| work.next_work) else {
            return;
        };

        if let Some(child) = self.begin_work(id, measurer) {
            if let Some(work) = self.work.as_mut() {
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
            .work
            .as_ref()
            .and_then(|work| work.nodes.get(&id))
            .is_some_and(|node| node.began)
        {
            return self.first_child_needing_work(id);
        }

        let (tag, should_render, should_reconcile_pending, pending_children) = {
            let work = self.work.as_ref().expect("work missing");
            let node = work.nodes.get(&id).expect("work node missing");
            (
                node.tag,
                node.effect != EffectTag::None
                    || includes_some_lane(node.lanes, work.render_lanes)
                    || node.current.is_none(),
                node.pending_children.is_some(),
                node.pending_children.as_ref().map(Vec::len),
            )
        };

        match tag {
            FiberTag::Root => {
                if should_render {
                    let render = self
                        .current_render(id)
                        .expect("root fiber missing render function");
                    let element = self.render_component(id, render);
                    self.reconcile_children(id, vec![element], measurer);
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Component(_) => {
                if should_render {
                    let render = self
                        .work
                        .as_ref()
                        .and_then(|work| work.nodes.get(&id))
                        .and_then(|node| node.render.clone())
                        .or_else(|| self.current_render(id))
                        .expect("component fiber missing render function");
                    let element = self.render_component(id, render);
                    self.reconcile_children(id, vec![element], measurer);
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

        if let Some(work) = self.work.as_mut() {
            if let Some(node) = work.nodes.get_mut(&id) {
                node.began = true;
            }
        }

        self.first_child_needing_work(id)
    }

    fn complete_work(&mut self, _id: FiberId) {}

    fn render_component(&mut self, id: FiberId, render: ComponentRender) -> Element {
        let render_lanes = self
            .work
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(SYNC_LANE);
        let storage = self.hooks.entry(id).or_default();
        let mut cx = HookContext::new(storage, id, self.scheduler.clone(), render_lanes);
        (render.borrow_mut())(&mut cx)
    }

    fn current_render(&self, id: FiberId) -> Option<ComponentRender> {
        self.find_current(id).and_then(|node| {
            node.component
                .as_ref()
                .map(|component| component.render.clone())
        })
    }

    fn reconcile_children(
        &mut self,
        parent: FiberId,
        new_children: Vec<Element>,
        measurer: &mut TextI,
    ) {
        let old_children = self
            .work
            .as_ref()
            .and_then(|work| work.nodes.get(&parent))
            .and_then(|node| {
                node.current
                    .as_ref()
                    .map(|current| current.children.clone())
            })
            .unwrap_or_default();
        let render_lanes = self
            .work
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
                (self.ids.insert(()), None, EffectTag::Placement)
            };

            let (lanes, child_lanes) = current
                .as_ref()
                .map(|current| {
                    (
                        self.scheduler.component_lanes(current.id) & render_lanes,
                        child_tree_lanes(current, &self.scheduler, render_lanes),
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

    fn prepare_element(&self, element: Element, measurer: &mut TextI) -> PreparedElement {
        match element {
            Element::Component(component) => {
                let key = component.key.clone();
                PreparedElement {
                    key,
                    tag: FiberTag::Component(component.type_id),
                    pending: PreparedPending::Component {
                        render: Rc::new(RefCell::new(component.render)),
                    },
                }
            }
            element => {
                let key = element.key();
                let props_hash = element.props_hash();
                let style = element.style(measurer);
                let (kind, widget, children) = element.into_parts();
                PreparedElement {
                    key,
                    tag: FiberTag::Host(kind.node_type()),
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

    fn clone_current_children(&mut self, parent: FiberId) {
        let Some(current_children) = self
            .work
            .as_ref()
            .and_then(|work| work.nodes.get(&parent))
            .and_then(|node| {
                node.current
                    .as_ref()
                    .map(|current| current.children.clone())
            })
        else {
            return;
        };

        let render_lanes = self
            .work
            .as_ref()
            .map(|work| work.render_lanes)
            .unwrap_or(NO_LANES);
        let mut children = Vec::with_capacity(current_children.len());

        for (position, current) in current_children.into_iter().enumerate() {
            let lanes = self.scheduler.component_lanes(current.id) & render_lanes;
            let child_lanes = child_tree_lanes(&current, &self.scheduler, render_lanes);
            self.work.as_mut().expect("work missing").nodes.insert(
                current.id,
                WorkNode::from_current(current.clone(), Some(parent), position, lanes, child_lanes),
            );
            children.push(current.id);
        }

        if let Some(work) = self.work.as_mut() {
            if let Some(node) = work.nodes.get_mut(&parent) {
                node.children = children;
                node.children_resolved = true;
            }
        }
    }

    fn first_child_needing_work(&self, parent: FiberId) -> Option<FiberId> {
        let work = self.work.as_ref()?;
        work.nodes
            .get(&parent)?
            .children
            .iter()
            .copied()
            .find(|id| {
                work.nodes
                    .get(id)
                    .is_some_and(|child| child.needs_work(work.render_lanes))
            })
    }

    fn next_sibling_needing_work(&self, id: FiberId) -> Option<FiberId> {
        let work = self.work.as_ref()?;
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

    fn commit_finished_work(&mut self, arena: &mut UiArena) {
        let Some(mut work) = self.work.take() else {
            return;
        };
        if work.next_work.is_some() {
            self.work = Some(work);
            return;
        }

        let deletions = std::mem::take(&mut work.deletions);
        for deletion in deletions {
            self.commit_deletion(&deletion, arena, true);
        }

        self.commit_work_node(work.root, self.root_widget, arena, &mut work);
        let next_current = self.freeze_work_tree(work.root, &work);
        self.current = next_current;
        self.sync_host_children(arena);
        self.scheduler.mark_render_finished(work.render_lanes);
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
                        .and_then(|node| node.host.as_mut())
                        .expect("host placement missing host work");
                    let node_id = arena.insert_node(
                        parent_host,
                        host.kind.clone(),
                        key,
                        host.props_hash,
                        host.style.clone(),
                        host.widget.take().expect("host widget already committed"),
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
                        .and_then(|node| node.host.as_mut())
                        .expect("host update missing host work");
                    arena.update_widget_node_from_parts(
                        node_id,
                        key,
                        host.props_hash,
                        host.style.clone(),
                        host.kind.clone(),
                        host.widget.take().expect("host widget already committed"),
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

    fn commit_deletion(&mut self, node: &Rc<FiberNode>, arena: &mut UiArena, remove_host: bool) {
        self.scheduler.mark_unmounted(node.id);
        self.hooks.remove(&node.id);
        self.ids.remove(node.id);

        if let Some(host) = node.host.as_ref().filter(|_| node.tag != FiberTag::Root) {
            if remove_host {
                arena.remove_subtree(host.node_id);
                for child in &node.children {
                    self.commit_deletion(child, arena, false);
                }
                return;
            }
        }

        for child in &node.children {
            self.commit_deletion(child, arena, remove_host);
        }
    }

    fn freeze_work_tree(&self, id: FiberId, work: &WorkInProgress) -> Rc<FiberNode> {
        let node = work.nodes.get(&id).expect("freeze missing work node");
        if node.effect == EffectTag::None
            && node.lanes == NO_LANES
            && node.child_lanes == NO_LANES
            && node.current.is_some()
            && !node.children_resolved
        {
            return node.current.as_ref().unwrap().clone();
        }

        let children = if !node.children_resolved {
            node.current
                .as_ref()
                .map(|current| current.children.clone())
                .unwrap_or_default()
        } else {
            node.children
                .iter()
                .map(|child| self.freeze_work_tree(*child, work))
                .collect()
        };

        Rc::new(FiberNode {
            id: node.id,
            parent: node.parent,
            key: node.key.clone(),
            position: node.position,
            tag: node.tag,
            host: match node.tag {
                FiberTag::Root => Some(HostSnapshot {
                    node_id: self.root_widget,
                    kind: WidgetKind::Root,
                    style: tf::Style::default(),
                    props_hash: 0,
                }),
                FiberTag::Host(_) => {
                    let host = node.host.as_ref();
                    let current_host = node
                        .current
                        .as_ref()
                        .and_then(|current| current.host.as_ref());
                    Some(HostSnapshot {
                        node_id: node
                            .host_node
                            .or_else(|| current_host.map(|host| host.node_id))
                            .expect("frozen host missing node id"),
                        kind: host
                            .map(|host| host.kind.clone())
                            .or_else(|| current_host.map(|host| host.kind.clone()))
                            .expect("frozen host missing kind"),
                        style: host
                            .map(|host| host.style.clone())
                            .or_else(|| current_host.map(|host| host.style.clone()))
                            .expect("frozen host missing style"),
                        props_hash: host
                            .map(|host| host.props_hash)
                            .or_else(|| current_host.map(|host| host.props_hash))
                            .expect("frozen host missing props hash"),
                    })
                }
                FiberTag::Component(_) => None,
            },
            component: match node.tag {
                FiberTag::Root | FiberTag::Component(_) => Some(ComponentSnapshot {
                    render: node
                        .render
                        .clone()
                        .or_else(|| {
                            node.current
                                .as_ref()
                                .and_then(|current| current.component.as_ref())
                                .map(|component| component.render.clone())
                        })
                        .expect("frozen component missing render"),
                }),
                FiberTag::Host(_) => None,
            },
            children,
        })
    }

    fn sync_host_children(&self, arena: &mut UiArena) {
        self.sync_host_children_from(&self.current, arena);
    }

    fn sync_host_children_from(&self, node: &Rc<FiberNode>, arena: &mut UiArena) {
        if let Some(host) = node.host.as_ref() {
            let children = flatten_host_children(node);
            arena.set_children(host.node_id, children);
        }

        for child in &node.children {
            self.sync_host_children_from(child, arena);
        }
    }

    fn find_current(&self, id: FiberId) -> Option<Rc<FiberNode>> {
        find_node(self.current.clone(), id)
    }
}

impl WorkNode {
    fn from_current(
        current: Rc<FiberNode>,
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
            host_node: current.host.as_ref().map(|host| host.node_id),
            current: Some(current),
            host: None,
            render: None,
            pending_children: None,
            children: Vec::new(),
            children_resolved: false,
            effect: EffectTag::None,
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
        current: Option<Rc<FiberNode>>,
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
}

fn prepared_needs_update(current: &FiberNode, prepared: &PreparedElement) -> bool {
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

fn find_reusable_child(
    old_children: &[Rc<FiberNode>],
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

fn child_tree_lanes(node: &FiberNode, scheduler: &Scheduler, render_lanes: Lanes) -> Lanes {
    let mut lanes = NO_LANES;
    for child in &node.children {
        lanes |= scheduler.component_lanes(child.id) & render_lanes;
        lanes |= child_tree_lanes(child, scheduler, render_lanes);
    }
    lanes
}

fn flatten_host_children(node: &FiberNode) -> Vec<NodeId> {
    let mut output = Vec::new();
    for child in &node.children {
        flatten_host_child(child, &mut output);
    }
    output
}

fn flatten_host_child(node: &FiberNode, output: &mut Vec<NodeId>) {
    if let Some(host) = node.host.as_ref() {
        if node.tag != FiberTag::Root {
            output.push(host.node_id);
            return;
        }
    }

    for child in &node.children {
        flatten_host_child(child, output);
    }
}

fn find_node(node: Rc<FiberNode>, id: FiberId) -> Option<Rc<FiberNode>> {
    if node.id == id {
        return Some(node);
    }

    for child in &node.children {
        if let Some(found) = find_node(child.clone(), id) {
            return Some(found);
        }
    }
    None
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::core::{Color, Size};
    use crate::lanes::{
        DEFAULT_LANE, TRANSITION_LANES, get_highest_priority_lane, includes_sync_lane,
        start_transition, with_update_lane,
    };
    use crate::render::MockRenderBackend;
    use crate::widgets::{column, component, container, label, row};

    fn render_all(runtime: &mut FiberRuntime, arena: &mut UiArena, measurer: &mut TextI) {
        while runtime.is_dirty() {
            runtime.perform_work(arena, measurer, None);
        }
    }

    fn root_child(arena: &UiArena) -> NodeId {
        arena.children(arena.root())[0]
    }

    #[test]
    fn commit_is_deferred_until_work_finishes() {
        let scheduler = Scheduler::default();
        let mut arena = UiArena::new();
        let mut text = TextI::new();
        let mut runtime = FiberRuntime::new(arena.root(), scheduler, |_| {
            column()
                .child(label("a"))
                .child(label("b"))
                .child(label("c"))
                .into()
        });
        runtime.set_budget(Duration::ZERO);

        assert!(arena.children(arena.root()).is_empty());
        assert!(matches!(
            runtime.perform_budgeted_work(&mut arena, &mut text),
            WorkStatus::Yielded
        ));
        assert!(arena.children(arena.root()).is_empty());

        render_all(&mut runtime, &mut arena, &mut text);
        assert_eq!(arena.children(arena.root()).len(), 1);
    }

    #[test]
    fn keyed_component_reorder_preserves_hook_state() {
        let reversed = Rc::new(Cell::new(false));
        let reversed_for_app = reversed.clone();
        let scheduler = Scheduler::default();
        let mut arena = UiArena::new();
        let mut text = TextI::new();
        let mut runtime = FiberRuntime::new(arena.root(), scheduler, move |_| {
            let first = component(|cx| {
                let value = cx.use_state(|| "A".to_owned());
                label(value.get()).into()
            })
            .key("a");
            let second = component(|cx| {
                let value = cx.use_state(|| "B".to_owned());
                label(value.get()).into()
            })
            .key("b");

            if reversed_for_app.get() {
                row().child(second).child(first).into()
            } else {
                row().child(first).child(second).into()
            }
        });

        render_all(&mut runtime, &mut arena, &mut text);
        let row_id = root_child(&arena);
        let before = arena.children(row_id).to_vec();

        reversed.set(true);
        runtime.mark_root_dirty();
        render_all(&mut runtime, &mut arena, &mut text);

        let after = arena.children(row_id).to_vec();
        assert_eq!(after, vec![before[1], before[0]]);
        assert!(matches!(
            &arena.node(after[0]).unwrap().kind,
            WidgetKind::Label { text, .. } if text == "B"
        ));
        assert!(matches!(
            &arena.node(after[1]).unwrap().kind,
            WidgetKind::Label { text, .. } if text == "A"
        ));
    }

    #[test]
    fn same_key_different_component_type_replaces_subtree() {
        fn first(_: &mut HookContext<'_>) -> Element {
            label("first").into()
        }
        fn second(_: &mut HookContext<'_>) -> Element {
            label("second").into()
        }

        let use_second = Rc::new(Cell::new(false));
        let use_second_for_app = use_second.clone();
        let scheduler = Scheduler::default();
        let mut arena = UiArena::new();
        let mut text = TextI::new();
        let mut runtime = FiberRuntime::new(arena.root(), scheduler, move |_| {
            if use_second_for_app.get() {
                component(second).key("same").into()
            } else {
                component(first).key("same").into()
            }
        });

        render_all(&mut runtime, &mut arena, &mut text);
        let first_node = root_child(&arena);

        use_second.set(true);
        runtime.mark_root_dirty();
        render_all(&mut runtime, &mut arena, &mut text);

        let second_node = root_child(&arena);
        assert_ne!(first_node, second_node);
        assert!(!arena.contains(first_node));
        assert!(matches!(
            &arena.node(second_node).unwrap().kind,
            WidgetKind::Label { text, .. } if text == "second"
        ));
    }

    #[test]
    fn deleting_component_cleans_host_subtree() {
        let show = Rc::new(Cell::new(true));
        let show_for_app = show.clone();
        let scheduler = Scheduler::default();
        let mut arena = UiArena::new();
        let mut text = TextI::new();
        let mut runtime = FiberRuntime::new(arena.root(), scheduler, move |_| {
            let mut root = column();
            if show_for_app.get() {
                root = root.child(component(|_| {
                    container()
                        .key("box")
                        .child(label("child").key("child"))
                        .into()
                }));
            }
            root.into()
        });

        render_all(&mut runtime, &mut arena, &mut text);
        let column_id = root_child(&arena);
        let box_id = arena.children(column_id)[0];
        let child_id = arena.children(box_id)[0];

        show.set(false);
        runtime.mark_root_dirty();
        render_all(&mut runtime, &mut arena, &mut text);

        assert!(arena.children(column_id).is_empty());
        assert!(!arena.contains(box_id));
        assert!(!arena.contains(child_id));
    }

    #[test]
    fn hook_set_queues_update_until_render_lane_runs() {
        let captured = Rc::new(RefCell::new(None));
        let captured_for_app = captured.clone();
        let scheduler = Scheduler::default();
        let mut arena = UiArena::new();
        let mut text = TextI::new();
        let mut runtime = FiberRuntime::new(arena.root(), scheduler.clone(), move |_| {
            let state = captured_for_app.clone();
            component(move |cx| {
                let count = cx.use_state(|| 0);
                *state.borrow_mut() = Some(count.clone());
                label(format!("count: {}", count.get())).into()
            })
            .into()
        });

        render_all(&mut runtime, &mut arena, &mut text);
        let state = captured.borrow().clone().unwrap();
        state.set(1);
        assert_eq!(state.get(), 0);
        assert!(scheduler.is_dirty());
        render_all(&mut runtime, &mut arena, &mut text);
        assert_eq!(state.get(), 1);
    }

    #[test]
    fn sync_update_interrupts_transition_work() {
        let scheduler = Scheduler::default();
        let mut arena = UiArena::new();
        let mut text = TextI::new();
        let mut runtime = FiberRuntime::new(arena.root(), scheduler, |_| {
            column()
                .child(label("a"))
                .child(label("b"))
                .child(label("c"))
                .child(label("d"))
                .into()
        });
        runtime.set_budget(Duration::ZERO);
        render_all(&mut runtime, &mut arena, &mut text);

        start_transition(|| runtime.mark_root_dirty());
        assert!(matches!(
            runtime.perform_budgeted_work(&mut arena, &mut text),
            WorkStatus::Yielded
        ));
        assert!(includes_some_lane(
            runtime.work.as_ref().unwrap().render_lanes,
            TRANSITION_LANES
        ));

        with_update_lane(SYNC_LANE, || runtime.mark_root_dirty());
        assert!(matches!(
            runtime.perform_budgeted_work(&mut arena, &mut text),
            WorkStatus::Yielded
        ));
        assert_eq!(runtime.work.as_ref().unwrap().render_lanes, SYNC_LANE);
    }

    #[test]
    fn app_paints_after_fiber_commit() {
        let mut app = crate::app::app(|_| label("hello").color(Color::BLACK).into());
        let mut backend = MockRenderBackend::default();
        app.resize(Size::new(100.0, 100.0));
        assert!(app.arena().is_dirty());
        app.render(&mut backend).unwrap();

        assert!(
            backend.last_commands.iter().any(|command| {
                matches!(command, crate::render::PaintCommand::Text { text, .. } if text == "hello")
            }),
            "commands={:?}, root={:?}, children={:?}",
            backend.last_commands,
            app.arena().node(app.arena().root()).unwrap().layout,
            app.arena()
                .children(app.arena().root())
                .iter()
                .map(|id| app.arena().node(*id).unwrap().layout)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn lane_helpers_group_default_before_transition() {
        let lanes = DEFAULT_LANE | TRANSITION_LANES;
        assert_eq!(get_highest_priority_lane(lanes), DEFAULT_LANE);
        assert!(!includes_sync_lane(lanes));
    }
}
