//! The component runtime: renders components and reconciles what they
//! return into the fiber tree, then commits that to the host tree.
//!
//! There is one fiber tree, changed in place. A pass renders every component
//! that needs it -- the dirty ones, and those a parent's render handed new
//! props -- shallowest first, so a component renders after every ancestor
//! that renders in the same pass, and at most once. What a component returns
//! is reconciled into its fiber's children there and then: templates,
//! fragments and portals have no render of their own, so they are matched,
//! created or updated as the reconcile reaches them, and only components
//! are left for the pass to render.
//!
//! A reconcile records what the host tree needs on the fibers themselves --
//! [`EffectTag`]s, and a template's [`TemplatePending`] -- and the commit
//! that ends the pass applies it: first the removed subtrees, then a preorder
//! walk down to every fiber with an effect, placing, moving and updating
//! hosts in sibling order.

use crate::context::{Ancestors, ContextRegistry, ContextScope};
use crate::element::PortalBehavior;
use crate::fiber::{
    ComponentRender, ComponentState, ComponentType, EffectTag, FiberArena, FiberId, FiberKind,
    FiberTag, Key, Node, PortalState, TemplateState,
};
use crate::lanes::{
    Lanes, NO_LANES, current_update_lane, get_highest_priority_lane, includes_some_lane,
};
use crate::state::{AsyncDispatcher, HookContext, HookStorage, Scheduler};
use crate::template::{
    BuildMode, TemplateHosts, TemplateMeta, TemplateNodeKind, TemplatePending, TemplateSlot,
};
use crate::ticker::TickerRegistry;
use crate::ui_runtime::UiRuntime;
use crate::widgets::{OverlayEntryId, RootComponentRender};
use crate::{ElementDesc, ErasedPropsRef};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::rc::Rc;
use tokio::runtime::Handle as TokioHandle;
use xui_interface::NodeId;

/// A fiber's children, as a reconcile reads them.
type Children = SmallVec<[FiberId; 8]>;

/// How many pending template builds' room a pass keeps for the next.
const PENDING_KEPT: usize = 256;

pub struct ComponentRuntime {
    nodes: FiberArena,
    current: FiberId,
    scheduler: Scheduler,
    async_dispatcher: AsyncDispatcher,
    tokio_handle: Option<TokioHandle>,
    /// Handed to every `HookContext`, so `use_ticker` can install into the
    /// list the runtime walks each frame.
    tickers: TickerRegistry,
    hooks: FxHashMap<FiberId, HookStorage>,
    contexts: ContextRegistry,
    root_widget: NodeId,
    /// The current pass, which a fiber's `rendered_in` is compared with.
    pass: u32,
    /// The components this pass still renders, shallowest first.
    queue: BinaryHeap<Reverse<(u32, FiberId)>>,
    /// The subtrees this pass detached, unmounted at commit.
    deletions: Vec<FiberId>,
    /// The fibers this pass created, mounted at commit.
    created: Vec<FiberId>,
    /// The templates' builds this pass leaves for the commit, each taken by
    /// its template, at the index the template holds.
    pending: Vec<Option<TemplatePending>>,
    /// Scratch for the scheduler's log of newly dirty components.
    marked: Vec<FiberId>,
    /// Scratch for [`Self::reconcile_list`], one per list being reconciled.
    lists: Vec<ListScratch>,
}

/// What reconciling one list needs besides the fibers.
#[derive(Default)]
struct ListScratch {
    old: Vec<FiberId>,
    used: Vec<bool>,
    keyed: FxHashMap<Key, usize>,
    next: Vec<FiberId>,
}

/// The strict ancestors of a fiber.
struct FiberAncestors<'a> {
    nodes: &'a FiberArena,
    of: FiberId,
}

impl Ancestors for FiberAncestors<'_> {
    fn find(&self, visit: &mut dyn FnMut(FiberId) -> bool) {
        let mut cursor = self.nodes.node(self.of).and_then(|node| node.parent);
        while let Some(id) = cursor {
            if visit(id) {
                return;
            }
            cursor = self.nodes.node(id).and_then(|node| node.parent);
        }
    }
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
            scheduler,
            async_dispatcher,
            tokio_handle,
            tickers,
            hooks: FxHashMap::default(),
            contexts: ContextRegistry::default(),
            pass: 0,
            queue: BinaryHeap::new(),
            deletions: Vec::new(),
            created: Vec::new(),
            pending: Vec::new(),
            marked: Vec::new(),
            lists: Vec::new(),
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

    pub fn is_dirty(&self) -> bool {
        self.scheduler.is_dirty()
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

    /// Renders every pending update, then commits it.
    pub fn flush_sync(&mut self, arena: &mut UiRuntime) {
        let render_lanes = self.scheduler.pending_lanes();
        if render_lanes == NO_LANES {
            return;
        }
        self.pass = self.pass.wrapping_add(1);

        let mut dirty = std::mem::take(&mut self.marked);
        dirty.clear();
        self.scheduler.for_each_dirty(|id, lanes| {
            if includes_some_lane(lanes, render_lanes) {
                dirty.push(id);
            }
        });
        for &id in &dirty {
            self.enqueue(id);
        }
        dirty.clear();
        self.marked = dirty;

        while let Some(Reverse((_, id))) = self.queue.pop() {
            self.render(id, render_lanes);
        }

        self.commit(arena);
        self.scheduler.mark_render_finished(render_lanes);
    }

    // -----------------------------------------------------------------------
    // Render
    // -----------------------------------------------------------------------

    /// Queues component `id` to render in this pass.
    ///
    /// Only a component renders, and every fiber that is marked dirty is one
    /// -- a mark comes from a hook, whose owner is the component rendering
    /// it -- but the scheduler is public, so any id can be marked: anything
    /// else is not queued, which is what [`Self::render`] relies on.
    fn enqueue(&mut self, id: FiberId) {
        if let Some(node) = self.nodes.node(id)
            && matches!(node.kind, FiberKind::Component(_))
            && !node.effect.contains(EffectTag::DELETION)
            && node.rendered_in != self.pass
        {
            self.queue.push(Reverse((node.depth, id)));
        }
    }

    /// Queues the components marked dirty since the last call: the readers
    /// of a context a render just changed.
    fn enqueue_marked(&mut self) {
        self.scheduler.take_marked(&mut self.marked);
        if self.marked.is_empty() {
            return;
        }
        let mut marked = std::mem::take(&mut self.marked);
        for &id in &marked {
            self.enqueue(id);
        }
        marked.clear();
        self.marked = marked;
    }

    /// Renders component `id` and reconciles what it returns into its
    /// children. Skips one removed in this pass, or rendered already.
    fn render(&mut self, id: FiberId, render_lanes: Lanes) {
        let Some(node) = self.nodes.node_mut(id) else {
            return;
        };
        if node.effect.contains(EffectTag::DELETION) || node.rendered_in == self.pass {
            return;
        }
        node.rendered_in = self.pass;
        // One mounting in this pass has no readers yet.
        let committed = !node.effect.contains(EffectTag::PLACEMENT);
        let (render, props) = match &node.kind {
            FiberKind::Component(component) => (component.render, component.props.clone()),
            _ => unreachable!("`enqueue` queues only components"),
        };

        let element = {
            let Self {
                nodes,
                hooks,
                contexts,
                scheduler,
                async_dispatcher,
                tokio_handle,
                tickers,
                ..
            } = self;
            let nodes: &FiberArena = nodes;
            let ancestors = FiberAncestors { nodes, of: id };
            contexts.begin_render(id);
            let mut cx = HookContext::new_with_async(
                hooks.entry(id).or_default(),
                id,
                scheduler.clone(),
                render_lanes,
                async_dispatcher.clone(),
                tokio_handle.clone(),
                tickers.clone(),
            )
            .with_contexts(ContextScope {
                registry: &mut *contexts,
                ancestors: &ancestors,
            });
            let element = (render.call)(&mut cx, props.as_ref());
            drop(cx);
            // Before the children are reconciled, so that a reader of a
            // context this render changed is queued with them.
            contexts.finish_render(
                id,
                committed,
                &ancestors,
                nodes,
                scheduler,
                get_highest_priority_lane(render_lanes),
            );
            element
        };
        self.enqueue_marked();
        self.reconcile_list(id, std::iter::once(element));
    }

    // -----------------------------------------------------------------------
    // Reconcile
    // -----------------------------------------------------------------------

    fn children(&self, parent: FiberId) -> Children {
        let mut children = Children::new();
        let mut cursor = self.nodes.node(parent).and_then(|node| node.child);
        while let Some(id) = cursor {
            children.push(id);
            cursor = self.nodes.node(id).and_then(|node| node.sibling);
        }
        children
    }

    /// Matches `elements`, a list, against `parent`'s children: by key, or
    /// by position for an unkeyed one. A match is updated in place, moved
    /// when it falls behind one matched before it; the rest are created, and
    /// the children left unmatched removed.
    fn reconcile_list<I>(&mut self, parent: FiberId, elements: I)
    where
        I: IntoIterator<Item = ElementDesc>,
        I::IntoIter: ExactSizeIterator,
    {
        let elements = elements.into_iter();
        // Reused from list to list; a nested list takes another.
        let mut scratch = self.lists.pop().unwrap_or_default();
        let ListScratch {
            old,
            used,
            keyed,
            next,
        } = &mut scratch;
        let mut cursor = self.nodes.node(parent).and_then(|node| node.child);
        while let Some(id) = cursor {
            let node = self.nodes.node(id).expect("a child");
            if let Some(key) = node.key {
                keyed.insert(key, old.len());
            }
            old.push(id);
            cursor = node.sibling;
        }
        used.resize(old.len(), false);
        next.reserve(elements.len());
        let mut last_placed = 0;

        for (position, element) in elements.enumerate() {
            let matched = match element.key() {
                Some(key) => keyed.get(&key).copied(),
                None => Some(position).filter(|&index| {
                    old.get(index)
                        .and_then(|id| self.nodes.node(*id))
                        .is_some_and(|node| node.key.is_none() && node.position == position)
                }),
            }
            .filter(|&index| {
                !used[index] && can_reuse(self.nodes.node(old[index]).expect("a child"), &element)
            });
            let id = match matched {
                Some(index) => {
                    used[index] = true;
                    let effect = if index < last_placed {
                        EffectTag::MOVE
                    } else {
                        last_placed = index;
                        EffectTag::empty()
                    };
                    let id = old[index];
                    self.update(id, element, effect);
                    id
                }
                None => self.create(parent, element),
            };
            next.push(id);
        }

        for (index, &id) in old.iter().enumerate() {
            if !used[index] {
                self.delete(id);
            }
        }
        self.nodes.set_children(parent, next);
        old.clear();
        used.clear();
        keyed.clear();
        next.clear();
        self.lists.push(scratch);
    }

    /// A template's slots against its committed ones. The same template has
    /// the same slots in the same places, so slot `i` can only be what slot
    /// `i` was: kept, updated in place, or replaced, never moved, and found
    /// in the template's slot array without the search
    /// [`Self::reconcile_list`] does. The children are relinked only when one
    /// is replaced.
    fn reconcile_slots(&mut self, template: FiberId, slots: Vec<TemplateSlot>) {
        if slots.is_empty() {
            return;
        }
        let mut fibers = std::mem::take(&mut self.template_mut(template).slots);
        if fibers.is_empty() {
            // Mounting: every slot is new.
            fibers = slots
                .into_iter()
                .map(|slot| {
                    let element = slot.into_element().expect("a template mounts every slot");
                    self.create(template, element)
                })
                .collect();
            self.nodes.set_children(template, &fibers);
            self.template_mut(template).slots = fibers;
            return;
        }
        debug_assert_eq!(
            fibers.len(),
            slots.len(),
            "a template is matched only to one with the same slots"
        );
        let mut replaced = false;
        for (position, slot) in slots.into_iter().enumerate() {
            let Some(element) = slot.into_element() else {
                continue;
            };
            let old = fibers[position];
            let node = self.nodes.node(old).expect("a slot's fiber");
            if node.key == element.key() && can_reuse(node, &element) {
                self.update(old, element, EffectTag::empty());
            } else {
                self.delete(old);
                fibers[position] = self.create(template, element);
                replaced = true;
            }
        }
        if replaced {
            self.nodes.set_children(template, &fibers);
        }
        self.template_mut(template).slots = fibers;
    }

    /// Gives fiber `id`, matched to `element`, this render's version of it.
    fn update(&mut self, id: FiberId, element: ElementDesc, mut effect: EffectTag) {
        let node = self.nodes.node_mut(id).expect("a matched fiber");
        node.key = element.key();
        match element {
            ElementDesc::Component(desc) => {
                let FiberKind::Component(component) = &mut node.kind else {
                    unreachable!("matched by kind");
                };
                // The same allocation cannot render anything new, and a
                // `memo` component's equal props neither.
                let unchanged = match (&component.props, &desc.props) {
                    (Some(old), Some(new)) => {
                        Rc::ptr_eq(old, new)
                            || desc
                                .render
                                .same_props
                                .is_some_and(|same| same(&**old, &**new))
                    }
                    (None, None) => true,
                    _ => false,
                };
                component.props = desc.props;
                if !unchanged {
                    self.enqueue(id);
                }
            }
            ElementDesc::Template(mut desc) => {
                let FiberKind::Template(template) = &mut node.kind else {
                    unreachable!("matched by kind");
                };
                let build = desc.build(BuildMode::Update, template.committed.as_ref());
                if build.writes_nothing() {
                    // Every value is what the template wrote last: the commit
                    // has nothing to do here, and the inputs it keeps are as
                    // good as these.
                    desc.recycle();
                } else {
                    template.pending = Some(push_pending(
                        &mut self.pending,
                        TemplatePending {
                            mode: BuildMode::Update,
                            desc,
                            hosts: build.hosts,
                            changed: build.changed,
                        },
                    ));
                    effect |= EffectTag::UPDATE;
                }
                self.mark(id, effect);
                self.reconcile_slots(id, build.slots);
                return;
            }
            ElementDesc::Fragment(fragment) => {
                self.mark(id, effect);
                self.reconcile_list(id, fragment.children);
                return;
            }
            ElementDesc::Portal(portal) => {
                let FiberKind::Portal(state) = &mut node.kind else {
                    unreachable!("matched by kind");
                };
                if state.scope != portal.scope
                    || state.options != portal.options
                    || state.behavior != portal.behavior
                {
                    state.scope = portal.scope;
                    state.options = portal.options;
                    state.behavior = portal.behavior;
                    effect |= EffectTag::UPDATE;
                }
                self.mark(id, effect);
                self.reconcile_list(id, portal.children);
                return;
            }
        }
        self.mark(id, effect);
    }

    /// Creates a fiber under `parent` for `element`, and everything under it
    /// short of rendering the components, which are queued.
    fn create(&mut self, parent: FiberId, element: ElementDesc) -> FiberId {
        let id = self.nodes.new_id();
        let depth = self.nodes.node(parent).expect("a parent").depth + 1;
        let key = element.key();
        let (kind, rest) = match element {
            ElementDesc::Component(desc) => (
                FiberKind::Component(ComponentState {
                    render: desc.render,
                    key,
                    props: desc.props,
                }),
                Created::Component,
            ),
            ElementDesc::Template(mut desc) => {
                let build = desc.build(BuildMode::Mount, None);
                (
                    FiberKind::Template(TemplateState {
                        meta: desc.meta,
                        identity: desc.identity,
                        nodes: Vec::new(),
                        pending: Some(push_pending(
                            &mut self.pending,
                            TemplatePending {
                                mode: BuildMode::Mount,
                                desc,
                                hosts: build.hosts,
                                changed: build.changed,
                            },
                        )),
                        committed: None,
                        slots: Box::default(),
                    }),
                    Created::Slots(build.slots),
                )
            }
            ElementDesc::Fragment(fragment) => {
                (FiberKind::Fragment, Created::List(fragment.children))
            }
            ElementDesc::Portal(portal) => (
                FiberKind::Portal(PortalState {
                    scope: portal.scope,
                    options: portal.options,
                    behavior: portal.behavior,
                    entry: None,
                    visual_root: None,
                }),
                Created::List(portal.children),
            ),
        };
        self.nodes.insert_node(
            id,
            Node {
                id,
                parent: Some(parent),
                child: None,
                sibling: None,
                key,
                position: 0,
                effect: EffectTag::empty(),
                kind,
                depth,
                rendered_in: 0,
            },
        );
        self.created.push(id);
        self.mark(id, EffectTag::PLACEMENT);
        match rest {
            Created::Component => self.enqueue(id),
            Created::Slots(slots) => self.reconcile_slots(id, slots),
            Created::List(children) => self.reconcile_list(id, children),
        }
        id
    }

    /// Detaches fiber `id`: it and everything under it are removed at
    /// commit, and none of it renders before then.
    fn delete(&mut self, id: FiberId) {
        let mut stack: SmallVec<[FiberId; 16]> = SmallVec::from_elem(id, 1);
        while let Some(fiber) = stack.pop() {
            if let Some(node) = self.nodes.node_mut(fiber) {
                node.effect |= EffectTag::DELETION;
                let mut cursor = node.child;
                while let Some(child) = cursor {
                    stack.push(child);
                    cursor = self.nodes.node(child).and_then(|node| node.sibling);
                }
            }
        }
        self.deletions.push(id);
    }

    /// Gives fiber `id` effect `effect`, and every ancestor the knowledge
    /// that one is below it.
    fn mark(&mut self, id: FiberId, effect: EffectTag) {
        if effect.is_empty() {
            return;
        }
        let Some(node) = self.nodes.node_mut(id) else {
            return;
        };
        node.effect |= effect;
        let mut cursor = node.parent;
        while let Some(ancestor) = cursor {
            let node = self.nodes.node_mut(ancestor).expect("an ancestor");
            if node.effect.contains(EffectTag::SUBTREE) {
                return;
            }
            node.effect |= EffectTag::SUBTREE;
            cursor = node.parent;
        }
    }

    // -----------------------------------------------------------------------
    // Commit
    // -----------------------------------------------------------------------

    fn commit(&mut self, arena: &mut UiRuntime) {
        for deletion in std::mem::take(&mut self.deletions) {
            self.commit_deletion(deletion, arena, true);
        }
        self.commit_mutation_effects(self.current, self.root_widget, arena);
        // Only the builds of templates removed in this pass are left. A pass
        // that wrote into many templates -- a mount, nearly always -- does not
        // keep room for as many: those that follow write into a few.
        if self.pending.capacity() > PENDING_KEPT {
            self.pending = Vec::new();
        } else {
            self.pending.clear();
        }
        for id in std::mem::take(&mut self.created) {
            if self.nodes.contains(id) {
                self.scheduler.mark_mounted(id);
            }
        }
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

    /// Applies the effects of `id` and of everything below it that has one,
    /// in preorder, so siblings are placed in their order. `parent_host` is
    /// the host `id`'s hosts go under.
    fn commit_mutation_effects(&mut self, id: FiberId, parent_host: NodeId, arena: &mut UiRuntime) {
        let Some(node) = self.nodes.node(id) else {
            return;
        };
        let (effect, tag) = (node.effect, node.tag());
        if effect.is_empty() {
            return;
        }

        if effect.intersects(EffectTag::PLACEMENT | EffectTag::MOVE) {
            let before = self.find_host_sibling(id);
            self.commit_placement_subtree(id, parent_host, before, arena);
        }

        if effect.contains(EffectTag::UPDATE) {
            self.commit_update_if_portal(id, parent_host, arena);
        }
        if tag == FiberTag::Template {
            // Mounted as it was placed; on an update, its dynamic hosts.
            self.commit_update_if_template(id, arena);
        }

        if tag == FiberTag::Template {
            // Each slot sits under the host the template wrote it in.
            let parent_hosts: SmallVec<[NodeId; 4]> = {
                let template = self
                    .nodes
                    .node(id)
                    .and_then(Node::template)
                    .expect("a template");
                template
                    .meta
                    .slots()
                    .map(|(_, slot)| {
                        let parent = slot.parent.expect("a template's root is a host");
                        template.nodes[usize::from(parent)].expect("a template's hosts are mounted")
                    })
                    .collect()
            };
            let mut cursor = self.nodes.node(id).and_then(|node| node.child);
            for parent_host in parent_hosts {
                let Some(child) = cursor else { break };
                cursor = self.nodes.node(child).and_then(|node| node.sibling);
                self.commit_mutation_effects(child, parent_host, arena);
            }
        } else {
            let child_parent_host = match tag {
                FiberTag::Portal => arena.root_overlayer(),
                _ => parent_host,
            };
            let mut cursor = self.nodes.node(id).and_then(|node| node.child);
            while let Some(child) = cursor {
                cursor = self.nodes.node(child).and_then(|node| node.sibling);
                self.commit_mutation_effects(child, child_parent_host, arena);
            }
            if tag == FiberTag::Portal {
                self.sync_portal_visual_root(id, parent_host, arena);
            }
        }

        if let Some(node) = self.nodes.node_mut(id) {
            node.effect = EffectTag::empty();
        }
    }

    fn commit_placement_subtree(
        &mut self,
        id: FiberId,
        parent_host: NodeId,
        before: Option<NodeId>,
        arena: &mut UiRuntime,
    ) {
        let Some(tag) = self.nodes.node(id).map(Node::tag) else {
            return;
        };

        if tag == FiberTag::Template {
            // Its slots are placed under its own hosts as the mutation pass
            // reaches them.
            let root = self.ensure_template_mounted(id, arena);
            arena.place(parent_host, root, before);
            self.clear_placement(id);
            return;
        }

        if tag == FiberTag::Portal {
            for child in self.children(id) {
                self.commit_placement_subtree(child, arena.root_overlayer(), None, arena);
            }
            let visual_root = self.first_host_in_subtree(id);
            let (scope, options, behavior, existing_entry) = {
                let portal = self
                    .nodes
                    .node(id)
                    .and_then(Node::portal)
                    .expect("a portal");
                (
                    portal.scope,
                    portal.options,
                    portal.behavior.clone(),
                    portal.entry,
                )
            };
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
            let portal = self.portal_mut(id);
            portal.entry = entry;
            portal.visual_root = visual_root;
            self.clear_placement(id);
            return;
        }

        for child in self.children(id) {
            self.commit_placement_subtree(child, parent_host, before, arena);
        }
    }

    fn clear_placement(&mut self, id: FiberId) {
        if let Some(node) = self.nodes.node_mut(id) {
            node.effect.remove(EffectTag::PLACEMENT | EffectTag::MOVE);
        }
    }

    fn portal_mut(&mut self, id: FiberId) -> &mut PortalState {
        match self.nodes.node_mut(id).map(|node| &mut node.kind) {
            Some(FiberKind::Portal(portal)) => portal,
            _ => unreachable!("a portal"),
        }
    }

    fn template_mut(&mut self, id: FiberId) -> &mut TemplateState {
        match self.nodes.node_mut(id).map(|node| &mut node.kind) {
            Some(FiberKind::Template(template)) => template,
            _ => unreachable!("a template"),
        }
    }

    /// Mounts a new template's hosts, each under its parent, and returns its
    /// root, unplaced. A mounted template's root as it is.
    fn ensure_template_mounted(&mut self, id: FiberId, arena: &mut UiRuntime) -> NodeId {
        let template = self.template_mut(id);
        if let Some(root) = template.root() {
            return root;
        }
        let index = template
            .pending
            .take()
            .expect("a new template has its build");
        let meta = template.meta;
        let mut pending = self.pending[index as usize]
            .take()
            .expect("a build is taken once");
        debug_assert_eq!(pending.mode, BuildMode::Mount);
        let mut nodes = vec![None; meta.nodes.len()];
        // Built in node order, so they arrive in the order they mount.
        let mut drafts = std::mem::take(&mut pending.hosts).into_iter();
        for (index, node) in meta.nodes.iter().enumerate() {
            if !matches!(node.kind, TemplateNodeKind::Host { .. }) {
                continue;
            }
            let draft = drafts
                .next()
                .filter(|(at, _)| usize::from(*at) == index)
                .map(|(_, draft)| draft)
                .unwrap_or_else(|| panic!("template {} built no draft for a host", meta.location));
            let host = arena.create_node(draft);
            if let Some(parent) = node.parent {
                // In preorder, so each lands after its earlier siblings.
                arena.place(
                    nodes[usize::from(parent)].expect("parents come first"),
                    host,
                    None,
                );
            }
            nodes[index] = Some(host);
        }
        pending.desc.recycle();
        let root = nodes[0].expect("a template's root is a host");
        self.template_mut(id).nodes = nodes;
        root
    }

    /// Updates a mounted template's dynamic hosts: those built whole from
    /// their drafts, the others with their dynamic values, which the
    /// template's generated `commit` writes in place.
    fn commit_update_if_template(&mut self, id: FiberId, arena: &mut UiRuntime) {
        let Some(index) = self.template_mut(id).pending else {
            return;
        };
        // A new template's build is for mounting it, which placing it does.
        if self.pending[index as usize]
            .as_ref()
            .is_none_or(|pending| pending.mode != BuildMode::Update)
        {
            return;
        }
        let mut pending = self.pending[index as usize].take().expect("checked above");
        let template = self.template_mut(id);
        template.pending = None;
        for (index, draft) in std::mem::take(&mut pending.hosts) {
            let host =
                template.nodes[usize::from(index)].expect("a mounted template keeps its hosts");
            arena.update_node(host, draft);
        }
        pending.desc.commit(&mut TemplateHosts::new(
            arena,
            &template.nodes,
            &pending.changed,
        ));
        // What this update wrote is what the next one compares with.
        if let Some(old) = template.committed.replace(pending.desc.into_committed()) {
            old.recycle();
        }
    }

    fn commit_update_if_portal(&mut self, id: FiberId, parent_host: NodeId, arena: &mut UiRuntime) {
        let Some(portal) = self.nodes.node(id).and_then(Node::portal) else {
            return;
        };
        if let (Some(entry), Some(visual_root)) = (portal.entry, portal.visual_root) {
            arena
                .update_overlay_entry(entry, portal.scope, portal.options)
                .expect("Portal entry update failed");
            apply_portal_behavior(arena, entry, visual_root, parent_host, &portal.behavior);
        }
    }

    fn sync_portal_visual_root(&mut self, id: FiberId, parent_host: NodeId, arena: &mut UiRuntime) {
        let next_visual_root = self.first_host_in_subtree(id);
        let Some(portal) = self.nodes.node(id).and_then(Node::portal) else {
            return;
        };
        if portal.visual_root == next_visual_root {
            return;
        }
        let (scope, options, behavior, current_entry) = (
            portal.scope,
            portal.options,
            portal.behavior.clone(),
            portal.entry,
        );

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
        let portal = self.portal_mut(id);
        portal.entry = next_entry;
        portal.visual_root = next_visual_root;
    }

    fn first_host_in_subtree(&self, id: FiberId) -> Option<NodeId> {
        let node = self.nodes.node(id)?;
        if let Some(host) = node.host() {
            return Some(host);
        }
        node.children(&self.nodes)
            .find_map(|child| self.first_host_in_subtree(child.id))
    }

    /// The host a fiber being placed goes before: the first host after it
    /// among its siblings, looking up through fibers that own no host.
    fn find_host_sibling(&self, id: FiberId) -> Option<NodeId> {
        let mut node = id;
        loop {
            let parent = self.nodes.node(node)?.parent?;
            let parent_tag = self.nodes.node(parent)?.tag();
            if parent_tag == FiberTag::Template {
                // A slot's siblings are the template's, not its fiber's.
                return self.template_anchor(parent, node);
            }
            let mut cursor = self.nodes.node(node)?.sibling;
            while let Some(sibling) = cursor {
                if let Some(host) = self.find_stable_host_in_subtree(sibling) {
                    return Some(host);
                }
                cursor = self.nodes.node(sibling)?.sibling;
            }
            if parent_tag == FiberTag::Portal {
                // Its children are the overlay entry's; nothing follows them.
                return None;
            }
            node = parent;
        }
    }

    /// The host a slot of `template` is placed before: the first sibling after
    /// it in the template that has one.
    fn template_anchor(&self, template: FiberId, slot_child: FiberId) -> Option<NodeId> {
        let state = self.nodes.node(template).and_then(Node::template)?;
        let (meta, nodes): (&TemplateMeta, &[Option<NodeId>]) = (state.meta, &state.nodes);
        let position = self.nodes.node(slot_child)?.position;
        let (slot_index, _) = meta.slots().nth(position)?;
        let children = &state.slots;
        for sibling in meta.following_siblings(slot_index) {
            match meta.nodes[sibling].kind {
                TemplateNodeKind::Host { .. } => return nodes[sibling],
                _ => {
                    let slot_position = meta.slots().position(|(index, _)| index == sibling)?;
                    if let Some(host) = children
                        .get(slot_position)
                        .and_then(|child| self.find_stable_host_in_subtree(*child))
                    {
                        return Some(host);
                    }
                }
            }
        }
        None
    }

    /// The first host in a subtree that is in place already: not one being
    /// placed or moved in this commit, and not under a portal.
    fn find_stable_host_in_subtree(&self, id: FiberId) -> Option<NodeId> {
        let node = self.nodes.node(id)?;
        if node
            .effect
            .intersects(EffectTag::PLACEMENT | EffectTag::MOVE)
        {
            return None;
        }
        if let Some(host) = node.host() {
            return Some(host);
        }
        // A portal's hosts live under the overlayer, not beside this one.
        if node.tag() == FiberTag::Portal {
            return None;
        }
        node.children(&self.nodes)
            .find_map(|child| self.find_stable_host_in_subtree(child.id))
    }
}

/// What a created fiber still needs once it is in the tree.
enum Created {
    Component,
    Slots(Vec<TemplateSlot>),
    List(Vec<ElementDesc>),
}

/// Leaves `build` for the commit; returns where the template finds it.
fn push_pending(pending: &mut Vec<Option<TemplatePending>>, build: TemplatePending) -> u32 {
    let index = u32::try_from(pending.len()).expect("fewer than 2^32 templates per pass");
    pending.push(Some(build));
    index
}

/// Whether the fiber `old` can take `element`: the same kind, and for a
/// component the same render function, for a template the same template.
fn can_reuse(old: &Node, element: &ElementDesc) -> bool {
    match (&old.kind, element) {
        (FiberKind::Component(old), ElementDesc::Component(new)) => old.render == new.render,
        (FiberKind::Template(old), ElementDesc::Template(new)) => old.identity == new.identity,
        (FiberKind::Fragment, ElementDesc::Fragment(_)) => true,
        (FiberKind::Portal(_), ElementDesc::Portal(_)) => true,
        _ => false,
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

    /// The scheduler is public, so a fiber that is not a component can be
    /// marked dirty: it is passed over rather than rendered.
    #[test]
    fn marking_a_fiber_that_is_not_a_component_renders_nothing() {
        let _exclusive = exclusive();
        reset_counters();
        let mut arena = UiRuntime::new();
        let mut runtime =
            ComponentRuntime::new(arena.root(), Scheduler::default(), component_parent);
        runtime.flush_sync(&mut arena);
        let child = runtime.nodes.children(runtime.root())[0];
        let template = runtime.nodes.children(child)[0];
        assert_eq!(
            runtime.nodes.node(template).unwrap().tag(),
            FiberTag::Template
        );
        let renders = CHILD_RENDER_COUNT.load(Ordering::SeqCst);

        runtime
            .scheduler()
            .mark_component_dirty(template, current_update_lane());
        runtime.flush_sync(&mut arena);

        assert_eq!(CHILD_RENDER_COUNT.load(Ordering::SeqCst), renders);
        assert!(!runtime.is_dirty(), "the mark was cleared with the pass");
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
