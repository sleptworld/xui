use crate::fiber::{
    ComponentRender, ComponentState, EffectTag, ErasedProps, FiberArena, FiberId, FiberTag,
    HostState, Key, Node,
};
use crate::lanes::{Lanes, NO_LANES, current_update_lane, includes_some_lane, should_interrupt};
use crate::layout::{computed_style_for_widget, taffy_style_for_widget};
use crate::state::{HookContext, HookStorage, Scheduler};
use crate::style::{ComputedStyle, Theme};
use crate::tree::UiArena;
use crate::widgets::{RootComponentRender, WidgetI};
use crate::{ComponentDesc, ElementDesc, ErasedPropsRef};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::fmt;
use std::ops::RangeBounds;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use taffy as tf;
use xui_interface::{DirtyFlags, EventHandlers, NodeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WipId(usize);
pub struct WorkNode {
    fiber_id: FiberId,
    parent: Option<WipId>,
    key: Option<Key>,
    tag: FiberTag,
    position: usize,
    // Signature of current fiber
    current: Option<FiberId>,
    effect: EffectTag,
    children_resolved: bool,
    children: SmallVec<[WipId; 20]>,
    lanes: Lanes,
    child_lanes: Lanes,
    // Host Widget
    host_work: Option<HostWork>,
    component_work: Option<ComponentWork>,
    host_node: Option<NodeId>,
}

struct HostWork {
    widget: Option<WidgetI>,
    event_handlers: Option<EventHandlers>,
    style: tf::Style,
    computed_style: ComputedStyle,
    props_hash: u64,
    pending_children: Vec<ElementDesc>,
}

struct ComponentWork {
    render: ComponentRender,
    key: Option<Key>,
    props_hash: u64,
    props: Option<ErasedProps>,
}

impl Into<ComponentState> for ComponentWork {
    fn into(self) -> ComponentState {
        ComponentState {
            key: self.key,
            render: self.render,
            props_hash: self.props_hash,
            props: self.props,
        }
    }
}

impl WorkNode {
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
            key: current.key.clone(),
            position,
            tag: current.tag,
            children: SmallVec::new(),
            current: Some(current.id),
            children_resolved: false,
            effect: EffectTag::empty(),
            lanes,
            child_lanes,
            host_node: current.host.as_ref().and_then(|host| host.node_id),
            host_work: None,
            component_work: None,
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
        let host_node = current
            .and_then(|current| nodes.node(current))
            .and_then(|node| node.host.as_ref())
            .and_then(|host| host.node_id);

        let (host_work, component_work) = match prepared.pending {
            PreparedPending::Host {
                widget,
                event_handlers,
                style,
                computed_style,
                props_hash,
                children,
            } => (
                Some(HostWork {
                    widget: Some(widget),
                    event_handlers: Some(event_handlers),
                    style,
                    computed_style,
                    props_hash,
                    pending_children: children,
                }),
                None,
            ),
            PreparedPending::Component {
                key,
                render,
                props_hash,
                props,
            } => (
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
            fiber_id,
            parent: Some(parent),
            key: prepared.key,
            position,
            tag: prepared.tag,
            current,
            effect,
            children: SmallVec::new(),
            children_resolved: false,
            lanes,
            child_lanes,
            host_work,
            host_node,
            component_work,
        }
    }

    fn needs_work(&self, render_lanes: Lanes) -> bool {
        !self.effect.is_empty()
            || self.is_uncommited()
            || self
                .host_work
                .as_ref()
                .is_some_and(|h| !h.pending_children.is_empty())
            || includes_some_lane(self.lanes | self.child_lanes, render_lanes)
    }

    fn take_work_nodes(&mut self) -> Option<Vec<ElementDesc>> {
        self.host_work
            .as_mut()
            .map(|h| std::mem::take(&mut h.pending_children))
    }

    #[inline(always)]
    fn is_uncommited(&self) -> bool {
        self.current.is_none()
    }

    #[inline(always)]
    fn need_update(&self) -> bool {
        self.effect.intersects(EffectTag::UPDATE)
    }

    #[inline(always)]
    fn need_placement(&self) -> bool {
        self.effect.intersects(EffectTag::PLACEMENT)
    }

    #[inline(always)]
    fn need_move(&self) -> bool {
        self.effect.intersects(EffectTag::MOVE)
    }

    #[inline(always)]
    fn is_from_current(&self) -> bool {
        self.current.is_some()
    }

    fn component_render_props<'a, 'b: 'a>(
        &'a self,
        fiber_tree: &'b FiberArena,
    ) -> Option<(ComponentRender, Option<ErasedPropsRef<'a>>)> {
        if let Some(component) = self.component_work.as_ref() {
            Some((component.render, component.props.as_ref().map(|p| &**p)))
        } else {
            let fiber_node_component = self
                .current
                .and_then(|c| fiber_tree.node(c))
                .and_then(|n| n.component.as_ref());
            fiber_node_component.map(|c| (c.render, c.props.as_ref().map(|p| &**p)))
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
        event_handlers: EventHandlers,
        style: tf::Style,
        computed_style: ComputedStyle,
        props_hash: u64,
        children: Vec<ElementDesc>,
    },
    Component {
        key: Option<Key>,
        render: ComponentRender,
        props_hash: u64,
        props: Option<ErasedProps>,
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
        self.wip_nodes.get(id.0).map(|n| n.as_ref()).flatten()
    }

    fn get_mut(&mut self, id: WipId) -> Option<&mut WorkNode> {
        self.wip_nodes.get_mut(id.0).map(|n| n.as_mut()).flatten()
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

    pub fn rebuild_sync_if_needed(&mut self, arena: &mut UiArena) {
        if self.is_dirty() {
            self.flush_sync(arena);
        }
    }

    pub fn rebuild_slice_if_needed(&mut self, arena: &mut UiArena) -> bool {
        if !self.is_dirty() {
            return true;
        }

        self.work_loop(arena, Some(Instant::now() + self.budget))
    }

    pub fn flush_sync(&mut self, arena: &mut UiArena) {
        self.work_loop(arena, None);
    }

    fn work_loop(&mut self, arena: &mut UiArena, deadline: Option<Instant>) -> bool {
        self.scheduler.mark_starved_lanes_as_expired(now_ms());
        loop {
            if self.scheduler.pending_lanes() == NO_LANES && self.work_in_progress.is_none() {
                return true;
            }

            self.ensure_work();
            if self.work_in_progress.is_none() {
                return false;
            }
            let theme = arena.theme();
            while self
                .work_in_progress
                .as_ref()
                .is_some_and(|work| work.next_work.is_some())
            {
                self.perform_unit_of_work(theme);
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

    fn perform_unit_of_work(&mut self, theme: &Theme) {
        let Some(id) = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.next_work)
        else {
            return;
        };

        if let Some(child) = self.begin_work(id, theme) {
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

    fn begin_work(&mut self, id: WipId, theme: &Theme) -> Option<WipId> {
        let (fiber_id, tag, should_render, should_reconcile_pending, render_lanes) = {
            let work = self.work_in_progress.as_ref().expect("work missing");
            let node = self.wip_nodes.get(id).expect("work node missing");
            let should_reconcile_pending = node
                .host_work
                .as_ref()
                .is_some_and(|h| !h.pending_children.is_empty());

            (
                node.fiber_id,
                node.tag,
                node.needs_work(work.render_lanes),
                should_reconcile_pending,
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
                    let mut cx = cx!(fiber_id);
                    let element = (self.root_render)(&mut cx);
                    self.reconcile_children(id, [element], theme);
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
                        self.reconcile_children(id, [element], theme);
                    }
                } else {
                    self.clone_current_children(id);
                }
            }
            FiberTag::Host(_) => {
                if should_reconcile_pending {
                    let children = self
                        .wip_nodes
                        .get_mut(id)
                        .and_then(|n| n.take_work_nodes())
                        .unwrap_or_default();
                    self.reconcile_children(id, children, theme);
                } else {
                    self.clone_current_children(id);
                }
            }
        }

        self.first_child_needing_work(id)
    }

    fn reconcile_children<I>(&mut self, parent: WipId, new_children: I, theme: &Theme)
    where
        I: IntoIterator<Item = ElementDesc>,
    {
        let work_node = self.wip_nodes.get(parent).unwrap();
        let work_node_current = work_node.current;

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
                (node.key.clone(), pos)
            })
            .filter(|(a, _)| a.is_some())
            .map(|(k, pos)| (k.unwrap(), pos))
            .collect();

        for (position, element) in new_children.enumerate() {
            let prepared = self.prepare_element(parent, element, theme);

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

    fn prepare_element(
        &self,
        parent: WipId,
        element: ElementDesc,
        theme: &Theme,
    ) -> PreparedElement {
        let key = element.key();
        match element {
            ElementDesc::Component(component) => self.prepare_component_element(component, key),
            ElementDesc::Host(host) => {
                let widget = host.widget;
                let props_hash = widget.props_hash();
                let tag = FiberTag::Host(widget.node_type());
                let (computed_style, style) = if let Some(parent_style) =
                    self.parent_style_for_work(parent)
                {
                    let computed_style = computed_style_for_widget(&widget, parent_style, theme);
                    let style = taffy_style_for_widget(&widget, &parent_style, &computed_style);
                    (computed_style, style)
                } else {
                    let parent_style = ComputedStyle::initial(theme);
                    let computed_style = computed_style_for_widget(&widget, &parent_style, theme);
                    let style = taffy_style_for_widget(&widget, &parent_style, &computed_style);
                    (computed_style, style)
                };

                let event_handlers = widget.take_event_handlers();
                PreparedElement {
                    key,
                    tag,
                    pending: PreparedPending::Host {
                        widget,
                        event_handlers,
                        style,
                        computed_style,
                        props_hash,
                        children: host.children,
                    },
                }
            }
        }
    }

    fn parent_style_for_work(&self, parent: WipId) -> Option<&ComputedStyle> {
        let mut cursor = Some(parent);
        while let Some(id) = cursor {
            let Some(node) = self.wip_nodes.get(id) else {
                return None;
            };
            if let Some(host) = node.host_work.as_ref() {
                return Some(&host.computed_style);
            }
            if let Some(host) = node
                .current
                .or(Some(node.fiber_id))
                .and_then(|fiber_id| self.nodes.node(fiber_id))
                .and_then(|node| node.host.as_ref())
            {
                return Some(&host.computed_style);
            }
            cursor = node.parent;
        }
        None
    }

    fn prepare_component_element(
        &self,
        component: ComponentDesc,
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

    fn commit_mutation_effects(&mut self, wip_id: WipId, parent_host: NodeId, arena: &mut UiArena) {
        let Some((effect, tag, children)) = self
            .wip_nodes
            .get(wip_id)
            .map(|node| (node.effect, node.tag, node.children.clone()))
        else {
            return;
        };

        if effect.contains(EffectTag::PLACEMENT) {
            let before = self.find_host_sibling_for_wip(wip_id);
            self.commit_placement_subtree(wip_id, parent_host, before, arena);
        } else {
            if effect.contains(EffectTag::UPDATE) {
                self.commit_update_if_host(wip_id, arena);
            }

            if effect.contains(EffectTag::MOVE) {
                let before = self.find_host_sibling_for_wip(wip_id);
                self.commit_move_subtree(wip_id, parent_host, before, arena);
            }
        }

        let child_parent_host = if matches!(tag, FiberTag::Host(_)) {
            self.host_node_for_wip(wip_id)
                .expect("host fiber missing host node after mutation")
        } else {
            parent_host
        };

        for child in children {
            self.commit_mutation_effects(child, child_parent_host, arena);
        }
    }

    fn host_node_for_wip(&self, wip_id: WipId) -> Option<NodeId> {
        let wip = self.wip_nodes.get(wip_id)?;

        if let Some(node_id) = wip.host_node {
            return Some(node_id);
        }

        wip.current
            .and_then(|id| self.nodes.node(id))
            .and_then(|node| node.host.as_ref())
            .and_then(|host| host.node_id)
    }

    fn ensure_host_created(&mut self, wip_id: WipId, arena: &mut UiArena) -> NodeId {
        if let Some(node_id) = self.wip_nodes.get(wip_id).and_then(|node| node.host_node) {
            return node_id;
        }

        let (key, props_hash, style, computed_style, widget, event_handlers) = {
            let wip = self
                .wip_nodes
                .get_mut(wip_id)
                .expect("placement host fiber missing work node");
            let key = wip.key.clone();
            let host_work = wip
                .host_work
                .as_mut()
                .expect("placement host fiber missing host work");
            let widget = host_work
                .widget
                .take()
                .expect("placement host work missing widget");
            let event_handlers = host_work
                .event_handlers
                .take()
                .expect("placement host work missing event handlers");
            (
                key,
                host_work.props_hash,
                host_work.style.clone(),
                host_work.computed_style.clone(),
                widget,
                event_handlers,
            )
        };

        let node_id = arena.create_node(
            key,
            props_hash,
            widget,
            event_handlers,
            style,
            computed_style,
        );
        self.wip_nodes
            .get_mut(wip_id)
            .expect("placement host fiber missing work node")
            .host_node = Some(node_id);
        node_id
    }

    fn commit_placement_subtree(
        &mut self,
        wip_id: WipId,
        parent_host: NodeId,
        before: Option<NodeId>,
        arena: &mut UiArena,
    ) {
        let Some((tag, children)) = self
            .wip_nodes
            .get(wip_id)
            .map(|node| (node.tag, node.children.clone()))
        else {
            return;
        };

        if matches!(tag, FiberTag::Host(_)) {
            let host_id = self.ensure_host_created(wip_id, arena);
            if let Some(before) = before {
                arena.insert_before(parent_host, host_id, before);
            } else {
                arena.append_child(parent_host, host_id);
            }
            if let Some(node) = self.wip_nodes.get_mut(wip_id) {
                node.effect.remove(EffectTag::PLACEMENT);
            }
            return;
        }

        for child in children {
            self.commit_placement_subtree(child, parent_host, before, arena);
        }
    }

    fn commit_move_subtree(
        &mut self,
        wip_id: WipId,
        parent_host: NodeId,
        before: Option<NodeId>,
        arena: &mut UiArena,
    ) {
        let Some((tag, children)) = self
            .wip_nodes
            .get(wip_id)
            .map(|node| (node.tag, node.children.clone()))
        else {
            return;
        };

        if matches!(tag, FiberTag::Host(_)) {
            let host_id = self
                .host_node_for_wip(wip_id)
                .expect("move host fiber missing host node");
            if let Some(before) = before {
                arena.insert_before(parent_host, host_id, before);
            } else {
                arena.append_child(parent_host, host_id);
            }
            if let Some(node) = self.wip_nodes.get_mut(wip_id) {
                node.effect.remove(EffectTag::MOVE);
            }
            return;
        }

        for child in children {
            self.commit_move_subtree(child, parent_host, before, arena);
        }
    }

    fn commit_update_if_host(&mut self, wip_id: WipId, arena: &mut UiArena) {
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
        let key = wip.key.clone();
        let host_work = wip
            .host_work
            .as_mut()
            .expect("host update missing host work");
        let widget = host_work.widget.take().expect("host update missing widget");
        let event_handlers = host_work
            .event_handlers
            .take()
            .expect("host update missing event handlers");

        arena.update_widget_node_from_parts(
            node_id,
            key,
            host_work.props_hash,
            host_work.style.clone(),
            host_work.computed_style.clone(),
            widget,
            event_handlers,
        );
        wip.host_node = Some(node_id);
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
        arena: &UiArena,
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
            self.collect_current_child_ids(wip.current)
        };

        let host = self.freeze_host_state(&mut wip, arena);
        let component = self.freeze_component_state(&mut wip);
        let frozen = crate::fiber::Node {
            id: fiber_id,
            parent: parent_fiber,
            child: None,
            sibling: None,
            key: wip.key,
            tag: wip.tag,
            effect: EffectTag::empty(),
            dirty: DirtyFlags::empty(),
            subtree_dirty: DirtyFlags::empty(),
            host,
            component,
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

    fn freeze_host_state(&mut self, wip: &mut WorkNode, arena: &UiArena) -> Option<HostState> {
        if !matches!(wip.tag, FiberTag::Host(_)) {
            return None;
        }

        let current_host = wip
            .current
            .and_then(|id| self.nodes.node_mut(id))
            .and_then(|node| node.host.take());
        let node_id = wip
            .host_node
            .or_else(|| current_host.as_ref().and_then(|host| host.node_id))
            .expect("host fiber missing committed host node");
        let arena_node = arena
            .node(node_id)
            .expect("committed host node missing from arena");
        let style = wip
            .host_work
            .as_ref()
            .map(|host| host.style.clone())
            .or_else(|| current_host.as_ref().map(|host| host.style.clone()))
            .unwrap_or_default();
        let props_hash = wip
            .host_work
            .as_ref()
            .map(|host| host.props_hash)
            .or_else(|| current_host.as_ref().map(|host| host.props_hash))
            .unwrap_or(arena_node.new_props_hash);

        Some(HostState {
            node_id: Some(node_id),
            widget: Some(arena_node.widget.clone()),
            taffy_node: Some(arena_node.taffy_node),
            style,
            computed_style: arena_node.computed_style.clone(),
            layout: arena_node.layout,
            previous_layout: arena_node.previous_layout,
            paint_cache: arena_node.paint_cache.clone(),
            props_hash,
        })
    }

    fn freeze_component_state(&mut self, wip: &mut WorkNode) -> Option<ComponentState> {
        if !matches!(wip.tag, FiberTag::Component) {
            return None;
        }

        if let Some(component) = wip.component_work.take() {
            return Some(component.into());
        }

        wip.current
            .and_then(|id| self.nodes.node_mut(id))
            .and_then(|node| node.component.take())
    }

    fn trace_commit_work_node(&self, depth: usize, event: fmt::Arguments<'_>) {
        let indent = "  ".repeat(depth);
        eprintln!("[xui::commit] {indent}{event}");
    }

    fn clone_current_children(&mut self, parent: WipId) {
        let Some(current_children) = self
            .wip_nodes
            .get(parent)
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
                    .is_some_and(|node| node.needs_work(work.render_lanes))
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
        println!("REBUILD WORK IN PROGRESS, lanes: {}", render_lanes);
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
            if node.current.is_none() {
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Diff {
    ReuseClean,
    Update,
    Replace,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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

    if prepared.key.is_none() {
        if old_node.key.is_some() || old_node.position != position {
            return None;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber::{ComponentType, ErasedPropsRef};
    use crate::widgets::{column, component as component_desc, text};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static HOST_REORDER_STEP: AtomicUsize = AtomicUsize::new(0);
    static COMPONENT_REORDER_STEP: AtomicUsize = AtomicUsize::new(0);
    static INSERT_STEP: AtomicUsize = AtomicUsize::new(0);
    static APPEND_STEP: AtomicUsize = AtomicUsize::new(0);
    static DELETE_STEP: AtomicUsize = AtomicUsize::new(0);
    static MOVE_UPDATE_STEP: AtomicUsize = AtomicUsize::new(0);

    #[derive(Hash)]
    struct ItemProps {
        text: &'static str,
    }

    fn item_call(cx: &mut HookContext<'_>, props: Option<ErasedPropsRef<'_>>) -> ElementDesc {
        let _state = cx.use_state(|| 1usize);
        let props = props
            .expect("item props missing")
            .downcast_ref::<ItemProps>()
            .expect("item props type changed");
        text(props.text).into_element_desc()
    }

    fn item_render() -> ComponentRender {
        ComponentRender::new(ComponentType::new("__xui_test_item"), item_call)
    }

    fn item(key: &'static str, label: &'static str) -> ElementDesc {
        component_desc(item_render())
            .key(key)
            .props(ItemProps { text: label })
            .into()
    }

    fn keyed_text(key: &'static str, label: &'static str) -> ElementDesc {
        text(label).key(key).into_element_desc()
    }

    fn column_with(children: Vec<ElementDesc>) -> ElementDesc {
        column().into_element_desc(children)
    }

    fn runtime(root: RootComponentRender) -> (UiArena, ComponentRuntime) {
        let arena = UiArena::new();
        let runtime = ComponentRuntime::new(arena.root(), Scheduler::default(), root);
        (arena, runtime)
    }

    fn rerender(runtime: &mut ComponentRuntime, arena: &mut UiArena) {
        runtime.mark_root_dirty();
        runtime.flush_sync(arena);
    }

    fn column_fiber(runtime: &ComponentRuntime) -> FiberId {
        let root_children = runtime.nodes.children(runtime.root());
        assert_eq!(root_children.len(), 1);
        root_children[0]
    }

    fn column_host(runtime: &ComponentRuntime) -> NodeId {
        runtime
            .nodes
            .node(column_fiber(runtime))
            .and_then(|node| node.host.as_ref())
            .and_then(|host| host.node_id)
            .expect("column host missing")
    }

    fn fiber_key_order(runtime: &ComponentRuntime, parent: FiberId) -> Vec<Key> {
        runtime
            .nodes
            .children(parent)
            .into_iter()
            .map(|id| runtime.nodes.node(id).unwrap().key.clone().unwrap())
            .collect()
    }

    fn host_text_order(arena: &UiArena, parent: NodeId) -> Vec<String> {
        arena
            .children(parent)
            .iter()
            .map(|id| {
                arena
                    .node(*id)
                    .and_then(|node| node.widget.text())
                    .expect("text node missing text")
                    .as_str()
                    .to_owned()
            })
            .collect()
    }

    fn host_reorder_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        if HOST_REORDER_STEP.load(Ordering::SeqCst) == 0 {
            column_with(vec![keyed_text("a", "A"), keyed_text("b", "B")])
        } else {
            column_with(vec![keyed_text("b", "B"), keyed_text("a", "A")])
        }
    }

    #[test]
    fn host_children_reorder_updates_fiber_and_host_order() {
        HOST_REORDER_STEP.store(0, Ordering::SeqCst);
        let (mut arena, mut runtime) = runtime(host_reorder_root);
        runtime.flush_sync(&mut arena);

        HOST_REORDER_STEP.store(1, Ordering::SeqCst);
        rerender(&mut runtime, &mut arena);

        let column = column_fiber(&runtime);
        let column_host = column_host(&runtime);
        assert_eq!(
            fiber_key_order(&runtime, column),
            vec![Key::from("b"), Key::from("a")]
        );
        assert_eq!(host_text_order(&arena, column_host), ["B", "A"]);
    }

    fn component_reorder_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        if COMPONENT_REORDER_STEP.load(Ordering::SeqCst) == 0 {
            column_with(vec![item("a", "A"), item("b", "B")])
        } else {
            column_with(vec![item("b", "B"), item("a", "A")])
        }
    }

    #[test]
    fn component_children_reorder_moves_descendant_hosts() {
        COMPONENT_REORDER_STEP.store(0, Ordering::SeqCst);
        let (mut arena, mut runtime) = runtime(component_reorder_root);
        runtime.flush_sync(&mut arena);

        COMPONENT_REORDER_STEP.store(1, Ordering::SeqCst);
        rerender(&mut runtime, &mut arena);

        let column = column_fiber(&runtime);
        let column_host = column_host(&runtime);
        assert_eq!(
            fiber_key_order(&runtime, column),
            vec![Key::from("b"), Key::from("a")]
        );
        assert_eq!(host_text_order(&arena, column_host), ["B", "A"]);
    }

    fn insert_before_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        if INSERT_STEP.load(Ordering::SeqCst) == 0 {
            column_with(vec![keyed_text("a", "A"), keyed_text("c", "C")])
        } else {
            column_with(vec![
                keyed_text("a", "A"),
                keyed_text("b", "B"),
                keyed_text("c", "C"),
            ])
        }
    }

    #[test]
    fn insertion_uses_stable_host_sibling() {
        INSERT_STEP.store(0, Ordering::SeqCst);
        let (mut arena, mut runtime) = runtime(insert_before_root);
        runtime.flush_sync(&mut arena);
        let column_host = column_host(&runtime);
        let c_node = arena.children(column_host)[1];

        INSERT_STEP.store(1, Ordering::SeqCst);
        rerender(&mut runtime, &mut arena);

        let children = arena.children(column_host);
        assert_eq!(host_text_order(&arena, column_host), ["A", "B", "C"]);
        assert_eq!(children[2], c_node);
    }

    fn append_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        if APPEND_STEP.load(Ordering::SeqCst) == 0 {
            column_with(vec![keyed_text("a", "A")])
        } else {
            column_with(vec![keyed_text("a", "A"), keyed_text("b", "B")])
        }
    }

    #[test]
    fn insertion_appends_when_no_stable_sibling_exists() {
        APPEND_STEP.store(0, Ordering::SeqCst);
        let (mut arena, mut runtime) = runtime(append_root);
        runtime.flush_sync(&mut arena);
        let column_host = column_host(&runtime);
        let a_node = arena.children(column_host)[0];

        APPEND_STEP.store(1, Ordering::SeqCst);
        rerender(&mut runtime, &mut arena);

        let children = arena.children(column_host);
        assert_eq!(host_text_order(&arena, column_host), ["A", "B"]);
        assert_eq!(children[0], a_node);
    }

    fn delete_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        if DELETE_STEP.load(Ordering::SeqCst) == 0 {
            column_with(vec![item("a", "A"), item("b", "B")])
        } else {
            column_with(vec![item("b", "B")])
        }
    }

    #[test]
    fn deleting_component_subtree_removes_descendant_host_and_state() {
        DELETE_STEP.store(0, Ordering::SeqCst);
        let (mut arena, mut runtime) = runtime(delete_root);
        runtime.flush_sync(&mut arena);
        let column = column_fiber(&runtime);
        let column_host = column_host(&runtime);
        let item_a = runtime.nodes.children(column)[0];
        let text_a = arena.children(column_host)[0];

        DELETE_STEP.store(1, Ordering::SeqCst);
        rerender(&mut runtime, &mut arena);

        assert!(!runtime.nodes.contains(item_a));
        assert!(!runtime.hooks.contains_key(&item_a));
        assert!(!arena.contains(text_a));
        assert_eq!(
            fiber_key_order(&runtime, column_fiber(&runtime)),
            vec![Key::from("b")]
        );
        assert_eq!(host_text_order(&arena, column_host), ["B"]);
    }

    fn move_update_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        if MOVE_UPDATE_STEP.load(Ordering::SeqCst) == 0 {
            column_with(vec![keyed_text("a", "A"), keyed_text("b", "B")])
        } else {
            column_with(vec![keyed_text("b", "B2"), keyed_text("a", "A")])
        }
    }

    #[test]
    fn moving_host_can_also_update_it() {
        MOVE_UPDATE_STEP.store(0, Ordering::SeqCst);
        let (mut arena, mut runtime) = runtime(move_update_root);
        runtime.flush_sync(&mut arena);
        let column_host = column_host(&runtime);
        let b_node = arena.children(column_host)[1];

        MOVE_UPDATE_STEP.store(1, Ordering::SeqCst);
        rerender(&mut runtime, &mut arena);

        let children = arena.children(column_host);
        assert_eq!(children[0], b_node);
        assert_eq!(host_text_order(&arena, column_host), ["B2", "A"]);
        assert_eq!(
            fiber_key_order(&runtime, column_fiber(&runtime)),
            vec![Key::from("b"), Key::from("a")]
        );
    }
}
