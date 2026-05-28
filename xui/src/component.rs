use crate::fiber::{
    ComponentRegistry, ComponentState, ComponentType, EffectTag, ErasedProps, FiberArena, FiberId,
    FiberTag, HostState, Key, Node,
};
use crate::lanes::{Lanes, NO_LANES, current_update_lane, includes_some_lane, should_interrupt};
use crate::state::{HookContext, HookStorage, Scheduler};
use crate::style::{ComputedStyle, Theme};
use crate::tree::UiArena;
use crate::widgets::{
    ComponentRender, Element, WidgetRef, computed_style_for_widget, taffy_style_for_widget,
};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::fmt;
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
    computed_style: ComputedStyle,
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
        let children: SmallVec<[FiberId; 20]> = children.into_iter().collect();

        Self {
            id: current.id,
            parent,
            key: current.key.clone(),
            position,
            tag: current.tag,
            children,
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
        computed_style: ComputedStyle,
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

impl fmt::Debug for WorkInProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "WorkInProgress")?;
        writeln!(
            f,
            "  render_lanes: {:#018b} ({:#x})",
            self.render_lanes, self.render_lanes
        )?;
        writeln!(f, "  root: {:?}", self.root)?;
        writeln!(f, "  next: {:?}", self.next_work)?;
        if !self.deletions.is_empty() {
            writeln!(f, "  deletions: {:?}", self.deletions)?;
        }
        writeln!(f, "  tree:")?;

        let mut visited = Vec::new();
        self.fmt_node(f, self.root, "    ", true, &mut visited)?;

        if self.nodes.len() > visited.len() {
            writeln!(f, "  detached:")?;
            let mut detached: Vec<_> = self
                .nodes
                .keys()
                .copied()
                .filter(|id| !visited.contains(id))
                .collect();
            detached.sort_by_key(|id| format!("{id:?}"));
            for id in detached {
                self.fmt_node(f, id, "    ", true, &mut visited)?;
            }
        }

        Ok(())
    }
}

impl WorkInProgress {
    fn fmt_node(
        &self,
        f: &mut fmt::Formatter<'_>,
        id: FiberId,
        prefix: &str,
        is_last: bool,
        visited: &mut Vec<FiberId>,
    ) -> fmt::Result {
        let branch = if is_last { "`-" } else { "+-" };
        let child_prefix = if is_last { "  " } else { "| " };

        let Some(node) = self.nodes.get(&id) else {
            writeln!(f, "{prefix}{branch} {:?} <missing work node>", id)?;
            return Ok(());
        };

        let details_prefix = format!("{prefix}{child_prefix}  ");

        write!(f, "{prefix}{branch} {:?} {}", node.id, WorkNodeTitle(node),)?;
        self.fmt_node_badges(f, node)?;
        writeln!(f)?;

        self.fmt_node_details(f, node, &details_prefix)?;

        if visited.contains(&id) {
            writeln!(f, "{prefix}{child_prefix}`- <cycle>")?;
            return Ok(());
        }
        visited.push(id);

        let next_prefix = format!("{prefix}{child_prefix}");
        for (index, child) in node.children.iter().enumerate() {
            self.fmt_node(
                f,
                *child,
                &next_prefix,
                index + 1 == node.children.len(),
                visited,
            )?;
        }

        Ok(())
    }

    fn fmt_node_badges(&self, f: &mut fmt::Formatter<'_>, node: &WorkNode) -> fmt::Result {
        let mut wrote = false;
        let mut write_badge = |f: &mut fmt::Formatter<'_>, label: &str| {
            if !wrote {
                write!(f, " [")?;
                wrote = true;
            } else {
                write!(f, ", ")?;
            }
            write!(f, "{label}")
        };

        if self.next_work == Some(node.id) {
            write_badge(f, "next")?;
        }
        match node.effect {
            EffectTag::None => {}
            EffectTag::Placement => write_badge(f, "placement")?,
            EffectTag::Update => write_badge(f, "update")?,
        }
        if node.current.is_none() {
            write_badge(f, "new")?;
        }
        if node.began {
            write_badge(f, "began")?;
        }
        if node.children_resolved {
            write_badge(f, "children resolved")?;
        }
        if node.host_work.is_some() {
            write_badge(f, "host work")?;
        }
        if let Some(children) = &node.pending_children {
            write_badge(f, &format!("pending children: {}", children.len()))?;
        }
        if node.lanes != NO_LANES {
            write_badge(f, &format!("lanes: {:#x}", node.lanes))?;
        }
        if node.child_lanes != NO_LANES {
            write_badge(f, &format!("child lanes: {:#x}", node.child_lanes))?;
        }

        if wrote {
            write!(f, "]")?;
        }
        Ok(())
    }

    fn fmt_node_details(
        &self,
        f: &mut fmt::Formatter<'_>,
        node: &WorkNode,
        prefix: &str,
    ) -> fmt::Result {
        write!(f, "{prefix}meta: pos={} current=", node.position,)?;
        match node.current {
            Some(current) => write!(f, "{current:?}")?,
            None => write!(f, "new")?,
        }
        if let Some(parent) = node.parent {
            write!(f, " parent={parent:?}")?;
        }
        if let Some(key) = &node.key {
            write!(f, " key={key:?}")?;
        }
        if let Some(host_node) = node.host_node {
            write!(f, " host_node={host_node:?}")?;
        }
        writeln!(f)?;

        if let Some(component) = &node.component {
            writeln!(
                f,
                "{prefix}component: render={} props_hash={:#x}",
                component.render.name(),
                component.props_hash,
            )?;
        }

        if let Some(children) = &node.pending_children {
            writeln!(f, "{prefix}pending children: {}", children.len())?;
            for (index, child) in children.iter().enumerate() {
                writeln!(f, "{prefix}  {index}: {:?}", PendingElementDebug(child))?;
            }
        }

        Ok(())
    }
}

struct PendingElementDebug<'a>(&'a Element);

struct WorkNodeTitle<'a>(&'a WorkNode);

impl fmt::Display for WorkNodeTitle<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.tag {
            FiberTag::Root => write!(f, "Root"),
            FiberTag::Host(widget) => write!(f, "Host({widget:?})"),
            FiberTag::Component => {
                if let Some(component) = &self.0.component {
                    write!(f, "Component({})", component.render.name())
                } else {
                    write!(f, "Component")
                }
            }
        }
    }
}

impl fmt::Debug for PendingElementDebug<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Element::Host(_) => f
                .debug_struct("Host")
                .field("tag", &self.0.node_type())
                .field("key", &self.0.key())
                .field("props_hash", &format_args!("{:#x}", self.0.props_hash()))
                .finish(),
            Element::Component(component) => f
                .debug_struct("Component")
                .field("render", &component.render.name())
                .field("key", &component.key)
                .field("props_hash", &format_args!("{:#x}", component.props_hash))
                .finish(),
        }
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

    pub fn scheduler(&self) -> &Scheduler {
        &self.scheduler
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
            let theme = arena.theme();
            while self
                .work_in_progress
                .as_ref()
                .is_some_and(|work| work.next_work.is_some())
            {
                self.perform_unit_of_work(measurer, theme);
            }
            self.commit_finished_work(arena);
        }
    }

    fn perform_unit_of_work<T: TextMeasurer>(&mut self, measurer: &mut T, theme: &Theme) {
        let Some(id) = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.next_work)
        else {
            return;
        };

        self.trace_work_in_progress(format_args!("begin unit {id:?}"));

        if let Some(child) = self.begin_work(id, measurer, theme) {
            if let Some(work) = self.work_in_progress.as_mut() {
                work.next_work = Some(child);
            }
            self.trace_work_in_progress(format_args!("descend from {id:?} to {child:?}"));
            return;
        }

        let mut current = id;
        loop {
            self.complete_work(current);
            if let Some(sibling) = self.next_sibling_needing_work(current) {
                if let Some(work) = self.work_in_progress.as_mut() {
                    work.next_work = Some(sibling);
                }
                self.trace_work_in_progress(format_args!(
                    "complete {current:?}; advance to sibling {sibling:?}"
                ));
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
                    self.trace_work_in_progress(format_args!("complete tree from {id:?}"));
                    return;
                }
            }
        }
    }

    fn trace_work_in_progress(&self, event: fmt::Arguments<'_>) {
        if let Some(work) = &self.work_in_progress {
            eprintln!("[xui::work] {event}\n{work:#?}");
        }
    }

    fn begin_work<T: TextMeasurer>(
        &mut self,
        id: FiberId,
        measurer: &mut T,
        theme: &Theme,
    ) -> Option<FiberId> {
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
                    self.reconcile_children(id, [element], measurer, theme);
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
                    self.reconcile_children(id, [element], measurer, theme);
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
                    self.reconcile_children(id, children, measurer, theme);
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
        theme: &Theme,
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
            let prepared = self.prepare_element(parent, element, measurer, theme);
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
        parent: FiberId,
        element: Element,
        measurer: &mut T,
        theme: &Theme,
    ) -> PreparedElement {
        let key = element.key();
        match element {
            Element::Component(component) => self.prepare_component_element(component, key),
            element => {
                let props_hash = element.props_hash();
                let tag = FiberTag::Host(element.node_type().expect("host element missing type"));
                let parts = element.into_parts();
                let computed_style = if let Some(parent_style) = self.parent_style_for_work(parent)
                {
                    parts
                        .widget
                        .with(|widget| computed_style_for_widget(widget, &parent_style, theme))
                } else {
                    let parent_style = ComputedStyle::initial(theme);
                    parts
                        .widget
                        .with(|widget| computed_style_for_widget(widget, &parent_style, theme))
                };

                let style = parts.widget.layout_with(|widget| {
                    taffy_style_for_widget(widget, &computed_style, measurer)
                });
                PreparedElement {
                    key,
                    tag,
                    pending: PreparedPending::Host {
                        widget: parts.widget,
                        event_handlers: parts.event_handlers,
                        style,
                        computed_style,
                        props_hash,
                        children: parts.children,
                    },
                }
            }
        }
    }

    fn parent_style_for_work(&self, parent: FiberId) -> Option<&ComputedStyle> {
        let mut cursor = Some(parent);
        while let Some(id) = cursor {
            if let Some(host) = self
                .work_in_progress
                .as_ref()
                .and_then(|work| work.nodes.get(&id))
                .and_then(|node| node.host_work.as_ref())
            {
                return Some(&host.computed_style);
            }
            if let Some(host) = self.nodes.node(id).and_then(|node| node.host.as_ref()) {
                return Some(&host.computed_style);
            }
            cursor = self
                .work_in_progress
                .as_ref()
                .and_then(|work| work.nodes.get(&id))
                .and_then(|node| node.parent)
                .or_else(|| self.nodes.node(id).and_then(|node| node.parent));
        }
        None
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

        self.commit_work_node(work.root, self.root_widget, arena, &mut work, 0);
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
                    computed_style: host.computed_style,
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
                        computed_style: host_node.computed_style.clone(),
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
        depth: usize,
    ) {
        let tag = work.nodes.get(&id).map(|node| node.tag);
        let mut child_parent_host = parent_host;
        self.trace_commit_work_node(
            depth,
            format_args!(
                "enter {id:?} {} parent_host={parent_host:?}",
                work.nodes
                    .get(&id)
                    .map(WorkNodeTitle)
                    .map(|title| title.to_string())
                    .unwrap_or_else(|| "<missing>".to_owned())
            ),
        );

        if matches!(tag, Some(FiberTag::Host(_))) {
            let effect = work.nodes[&id].effect;
            match effect {
                EffectTag::Placement => {
                    let key = work.nodes[&id].key.clone();
                    self.trace_commit_work_node(
                        depth,
                        format_args!("place host {id:?} under {parent_host:?}"),
                    );
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
                        host.computed_style.clone(),
                        host.widget
                            .as_ref()
                            .expect("host widget already committed")
                            .clone(),
                        host.event_handlers
                            .take()
                            .expect("host event handlers already committed"),
                    );
                    self.trace_commit_work_node(
                        depth,
                        format_args!("placed host {id:?} as {node_id:?}"),
                    );
                    work.nodes.get_mut(&id).unwrap().host_node = Some(node_id);
                    child_parent_host = node_id;
                }
                EffectTag::Update => {
                    let node_id = work.nodes[&id]
                        .host_node
                        .expect("host update missing node id");
                    let key = work.nodes[&id].key.clone();
                    self.trace_commit_work_node(
                        depth,
                        format_args!("update host {id:?} at {node_id:?}"),
                    );
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
                            host.computed_style.clone(),
                            host.widget
                                .as_ref()
                                .expect("host widget already committed")
                                .clone(),
                            host.event_handlers
                                .take()
                                .expect("host event handlers already committed"),
                        ),
                    );
                    self.trace_commit_work_node(
                        depth,
                        format_args!("updated host {id:?} at {node_id:?}"),
                    );
                    child_parent_host = node_id;
                }
                EffectTag::None => {
                    let node_id = work.nodes[&id]
                        .host_node
                        .expect("clean host missing node id");
                    self.trace_commit_work_node(
                        depth,
                        format_args!("reuse clean host {id:?} at {node_id:?}"),
                    );
                    child_parent_host = node_id;
                }
            }
        } else {
            self.trace_commit_work_node(
                depth,
                format_args!("no host work; children inherit {child_parent_host:?}"),
            );
        }

        let children = work
            .nodes
            .get(&id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        self.trace_commit_work_node(
            depth,
            format_args!(
                "mark mounted {id:?}; commit {} children with parent_host={child_parent_host:?}",
                children.len()
            ),
        );
        self.scheduler.mark_mounted(id);
        for child in children {
            self.commit_work_node(child, child_parent_host, arena, work, depth + 1);
        }
        self.trace_commit_work_node(depth, format_args!("leave {id:?}"));
    }

    fn trace_commit_work_node(&self, depth: usize, event: fmt::Arguments<'_>) {
        let indent = "  ".repeat(depth);
        eprintln!("[xui::commit] {indent}{event}");
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
