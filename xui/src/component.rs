use std::time::{Duration, SystemTime, UNIX_EPOCH};

use xui_interface::NodeId;

use crate::fiber::{EffectTag, FiberArena, FiberTag, Node};
use crate::font::TextI;
use crate::lanes::{Lanes, NO_LANE, NO_LANES, current_update_lane, should_interrupt};
use crate::state::{HookContext, HookStorage, Scheduler};
use crate::tree::UiArena;
use crate::widgets::{Element, Key};
use rustc_hash::FxHashMap;

type FiberId = NodeId;

pub struct FiberNode {
    node: FiberId
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
            lanes,
            child_lanes,
            began: false,
        }
    }

}

pub struct WorkInProgress {
    nodes: FxHashMap<FiberId, WorkNode>,
    root: FiberId,
    next_work: Option<FiberId>,
    render_lanes: Lanes,
    deletions: Vec<FiberId>,
}



pub struct ComponentRuntime {
    nodes: FiberArena,
    current: FiberId,
    work_in_progress: Option<WorkInProgress>,
    scheduler: Scheduler,
    hooks: FxHashMap<NodeId, HookStorage>,
    budget: Duration,
}

impl ComponentRuntime {
    pub fn new(
        root_widget: NodeId,
        scheduler: Scheduler,
        root_component: impl FnMut(&mut HookContext<'_>) -> Element + 'static,
    ) -> Self {
        let arena = FiberArena::new();
        let current = FiberNode { node: root_widget };

        Self {
            nodes:arena,
            current,
            work_in_progress:None,
            scheduler,
            hooks: FxHashMap::default(),
            budget: Duration::from_millis(4),
        }
    }

    pub fn root(&self) -> NodeId {
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
                .is_some_and(|work| work.next_work().is_some())
            {
                self.perform_unit_of_work(measurer);
            }
            self.commit_finished_work(arena);
        }
    }

    fn perform_unit_of_work(&mut self, measurer: &mut TextI) {
        let Some(id) = self.work_in_progress.as_ref().and_then(|work| work.next_work()) else {
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

    fn begin_work(&mut self, id: NodeId, measurer: &mut TextI) -> Option<NodeId> {
        if self
            .work_in_progress
            .as_ref()
            .and_then(|work| work.node(id))
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

    fn first_child_needing_work(&self, parent: NodeId) -> Option<NodeId> {
        let work = self.work_in_progress.as_ref()?;
        // work.nodes
        //     .get(&parent)?
        //     .children
        //     .iter()
        //     .copied()
        //     .find(|id| {
        //         work.nodes
        //             .get(id)
        //             .is_some_and(|child| child.needs_work(work.render_lanes))
        //     })

        work.node(parent)?.children(work).find(|node| {
            node.ch
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

        nodes.insert(
            self.root(),
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
        marks: &mut FxHashMap<NodeId, (Lanes, Lanes)>,
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