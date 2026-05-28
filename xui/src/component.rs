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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
    component_state: Option<ComponentWork>,
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
            component_state: current.component.as_ref().map(ComponentWork::from_state),
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

        let (host_work, pending_children, component_state) = match prepared.pending {
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
            component_state,
        }
    }

    fn needs_work(&self, render_lanes: Lanes) -> bool {
        self.effect != EffectTag::None
            || self.current.is_none()
            || self.pending_children.is_some()
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

        if let Some(component) = &node.component_state {
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
                if let Some(component) = &self.0.component_state {
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

    pub fn rebuild_sync_if_needed<T: TextMeasurer>(
        &mut self,
        arena: &mut UiArena,
        measurer: &mut T,
    ) {
        if self.is_dirty() {
            self.flush_sync(arena, measurer);
        }
    }

    pub fn rebuild_slice_if_needed<T: TextMeasurer>(
        &mut self,
        arena: &mut UiArena,
        measurer: &mut T,
    ) -> bool {
        if !self.is_dirty() {
            return true;
        }

        self.work_loop(arena, measurer, Some(Instant::now() + self.budget))
    }

    pub fn flush_sync<T: TextMeasurer>(&mut self, arena: &mut UiArena, measurer: &mut T) {
        self.work_loop(arena, measurer, None);
    }

    fn work_loop<T: TextMeasurer>(
        &mut self,
        arena: &mut UiArena,
        measurer: &mut T,
        deadline: Option<Instant>,
    ) -> bool {
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
                self.perform_unit_of_work(measurer, theme);
                let more_work = self
                    .work_in_progress
                    .as_ref()
                    .is_some_and(|work| work.next_work.is_some());
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

    fn perform_unit_of_work<T: TextMeasurer>(&mut self, measurer: &mut T, theme: &Theme) {
        let Some(id) = self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.next_work)
        else {
            return;
        };

        if let Some(child) = self.begin_work(id, measurer, theme) {
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
                    || node.current.is_none(),
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
                        .and_then(|node| node.component_state.as_ref())
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
                        .and_then(|node| node.component_state.as_ref())
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

        let next_current =
            self.commit_and_freeze_work_tree(work.root, self.root_widget, arena, &mut work, 0);
        self.current = next_current;
        self.sync_host_children(arena);
        self.scheduler.mark_render_finished(work.render_lanes);
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

    fn commit_and_freeze_work_tree(
        &mut self,
        id: FiberId,
        parent_host: NodeId,
        arena: &mut UiArena,
        work: &mut WorkInProgress,
        depth: usize,
    ) -> FiberId {
        let mut node = work.nodes.remove(&id).expect("commit missing work node");
        let mut child_parent_host = parent_host;

        if node.effect == EffectTag::None
            && node.lanes == NO_LANES
            && node.child_lanes == NO_LANES
            && node.current.is_some()
            && !node.children_resolved
        {
            self.scheduler.mark_mounted(id);
            return node.current.unwrap();
        }

        if matches!(node.tag, FiberTag::Host(_)) {
            match node.effect {
                EffectTag::Placement => {
                    let host = node
                        .host_work
                        .as_mut()
                        .expect("host placement missing host work");
                    let node_id = arena.create_node(
                        node.key.clone(),
                        host.props_hash,
                        host.widget
                            .as_ref()
                            .expect("host widget already committed")
                            .clone(),
                        host.event_handlers
                            .take()
                            .expect("host event handlers already committed"),
                        host.style.clone(),
                        host.computed_style.clone(),
                    );
                    eprintln!("NEW NODE {node_id:?} {:?};", node.tag);
                    node.host_node = Some(node_id);
                    child_parent_host = node_id;
                }
                EffectTag::Update => {
                    let node_id = node.host_node.expect("host update missing node id");
                    let host = node
                        .host_work
                        .as_mut()
                        .expect("host update missing host work");
                    host.widget = Some(
                        arena.update_widget_node_from_parts(
                            node_id,
                            node.key.clone(),
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

                    eprintln!("UPDATE {node_id:?} {:?};", node.tag);

                    child_parent_host = node_id;
                }
                EffectTag::None => {
                    let node_id = node.host_node.expect("clean host missing node id");
                    eprintln!("NOT UPDATE {node_id:?} {:?};", node.tag);
                    child_parent_host = node_id;
                }
            }
        }

        let children: Vec<_> = if !node.children_resolved {
            node.current
                .and_then(|current| self.nodes.node(current))
                .map(|current| {
                    current
                        .children(&self.nodes)
                        .map(|child| child.id)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            std::mem::take(&mut node.children)
                .into_iter()
                .map(|child| {
                    self.commit_and_freeze_work_tree(
                        child,
                        child_parent_host,
                        arena,
                        work,
                        depth + 1,
                    )
                })
                .collect()
        };
        // self.trace_commit_work_node(
        //     depth,
        //     format_args!(
        //         "mark mounted {id:?}; commit {} children with parent_host={child_parent_host:?}",
        //         children.len()
        //     ),
        // );
        self.scheduler.mark_mounted(id);

        let current_host = if matches!(node.tag, FiberTag::Host(_)) {
            node.current
                .filter(|current| *current == node.id)
                .and_then(|current| self.nodes.node_mut(current))
                .and_then(|current| current.host.take())
        } else {
            None
        };

        let host = if matches!(node.tag, FiberTag::Host(_)) {
            node.host_work
                .map(|host| HostState {
                    node_id: node.host_node,
                    widget: host.widget,
                    taffy_node: current_host.as_ref().and_then(|host| host.taffy_node),
                    style: host.style,
                    computed_style: host.computed_style,
                    layout: current_host
                        .as_ref()
                        .map(|host| host.layout)
                        .unwrap_or_default(),
                    previous_layout: current_host
                        .as_ref()
                        .map(|host| host.previous_layout)
                        .unwrap_or_default(),
                    paint_cache: Vec::new(),
                    props_hash: host.props_hash,
                })
                .or_else(|| {
                    let mut host = current_host.expect("clean host missing host state");
                    host.node_id = node.host_node;
                    host.paint_cache.clear();
                    Some(host)
                })
        } else {
            None
        };

        let component = node.component_state.map(|component| ComponentState {
            key: component.key,
            render: component.render,
            props_hash: component.props_hash,
            props: component.props,
        });

        let memoized_props_hash = match (&host, &component) {
            (Some(host), _) => host.props_hash,
            (_, Some(component)) => component.props_hash,
            _ => 0,
        };

        let frozen = Node {
            id: node.id,
            parent: node.parent,
            child: None,
            sibling: None,
            key: node.key,
            position: node.position,
            tag: node.tag,
            effect: EffectTag::None,
            dirty: DirtyFlags::empty(),
            subtree_dirty: DirtyFlags::empty(),
            pending_props: None,
            pending_children: None,
            memoized_props_hash,
            host,
            component,
        };

        if let Some(existing) = self.nodes.node_mut(node.id) {
            *existing = frozen;
        } else {
            self.nodes.insert_node(node.id, frozen);
        }
        self.nodes.set_children(node.id, children);
        // self.trace_commit_work_node(depth, format_args!("leave {id:?}"));
        node.id
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
        println!("REBUILD WORK IN PROGRESS, lanes: {}", render_lanes);
        let mut marks = FxHashMap::default();
        let (lanes, child_lanes) =
            self.collect_lane_marks(self.root_node(), render_lanes, &mut marks);
        let mut nodes = FxHashMap::default();
        // let (lanes, child_lanes) = marks.get(&self.root()).copied().unwrap_or_default();
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
    ) -> (Lanes, Lanes) {
        let own = self.scheduler.component_lanes(node.id) & render_lanes;
        let mut child_lanes = NO_LANES;

        for child in node.children(&self.nodes) {
            child_lanes |= self.collect_lane_marks(child, render_lanes, marks).1;
        }
        marks.insert(node.id, (own, child_lanes));
        (own, child_lanes)
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
