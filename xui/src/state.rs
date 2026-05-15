use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use xui_interface::NodeId;

use crate::lanes::{Lane, LaneRoot, Lanes, NO_LANES, current_update_lane, includes_some_lane};

type HookKey = (NodeId, usize);
type HookApply = Box<dyn FnOnce(&mut dyn Any)>;

#[derive(Default)]
pub struct HookStorage {
    slots: Vec<Box<dyn Any>>,
    cursor: usize,
}

impl HookStorage {
    pub fn begin(&mut self) {
        self.cursor = 0;
    }

    fn next_slot<T: 'static>(&mut self, init: impl FnOnce() -> T) -> (usize, Rc<RefCell<T>>) {
        let index = self.cursor;
        self.cursor += 1;

        if index == self.slots.len() {
            self.slots.push(Box::new(Rc::new(RefCell::new(init()))));
        }

        let value = self.slots[index]
            .downcast_ref::<Rc<RefCell<T>>>()
            .expect("hook order changed between rebuilds")
            .clone();
        (index, value)
    }
}

pub struct HookContext<'a> {
    storage: &'a mut HookStorage,
    owner: NodeId,
    scheduler: Scheduler,
    render_lanes: Lanes,
}

impl<'a> HookContext<'a> {
    pub fn new(
        storage: &'a mut HookStorage,
        owner: NodeId,
        scheduler: Scheduler,
        render_lanes: Lanes,
    ) -> Self {
        storage.begin();
        Self {
            storage,
            owner,
            scheduler,
            render_lanes,
        }
    }

    pub fn use_state<T: Clone + 'static>(&mut self, init: impl FnOnce() -> T) -> State<T> {
        let (hook_index, value) = self.storage.next_slot(init);
        self.scheduler
            .apply_hook_updates(self.owner, hook_index, self.render_lanes, &value);
        State {
            value,
            owner: self.owner,
            hook_index,
            scheduler: self.scheduler.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct Scheduler {
    inner: Rc<RefCell<SchedulerState>>,
}

#[derive(Default)]
struct SchedulerState {
    lane_root: LaneRoot,
    dirty_components: HashMap<NodeId, Lanes>,
    hook_updates: HashMap<HookKey, VecDeque<HookUpdate>>,
    mounted_components: HashSet<NodeId>,
    root: Option<NodeId>,
}

struct HookUpdate {
    lane: Lane,
    apply: HookApply,
}

impl Scheduler {
    pub fn set_root(&self, id: NodeId) {
        let mut inner = self.inner.borrow_mut();
        inner.root = Some(id);
        inner.mounted_components.insert(id);
    }

    pub fn mark_mounted(&self, id: NodeId) {
        self.inner.borrow_mut().mounted_components.insert(id);
    }

    pub fn mark_unmounted(&self, id: NodeId) {
        let mut inner = self.inner.borrow_mut();
        inner.mounted_components.remove(&id);
        inner.dirty_components.remove(&id);
        inner.hook_updates.retain(|(owner, _), _| *owner != id);
    }

    pub fn mark_component_dirty(&self, id: NodeId, lane: Lane) {
        let mut inner = self.inner.borrow_mut();
        if !inner.mounted_components.contains(&id) && inner.root != Some(id) {
            return;
        }
        *inner.dirty_components.entry(id).or_insert(NO_LANES) |= lane;
        inner.lane_root.mark_root_updated(lane);
    }

    pub fn mark_root_dirty(&self, lane: Lane) {
        let root = self.inner.borrow().root;
        if let Some(root) = root {
            self.mark_component_dirty(root, lane);
        }
    }

    pub fn component_lanes(&self, id: NodeId) -> Lanes {
        self.inner
            .borrow()
            .dirty_components
            .get(&id)
            .copied()
            .unwrap_or(NO_LANES)
    }

    pub fn enqueue_hook_update<T: 'static>(
        &self,
        owner: NodeId,
        hook_index: usize,
        update: impl FnOnce(&mut T) + 'static,
    ) {
        let lane = current_update_lane();
        let mut inner = self.inner.borrow_mut();
        if !inner.mounted_components.contains(&owner) {
            return;
        }

        inner
            .hook_updates
            .entry((owner, hook_index))
            .or_default()
            .push_back(HookUpdate {
                lane,
                apply: Box::new(move |value| {
                    update(
                        value
                            .downcast_mut::<T>()
                            .expect("hook state update type changed"),
                    );
                }),
            });
        *inner.dirty_components.entry(owner).or_insert(NO_LANES) |= lane;
        inner.lane_root.mark_root_updated(lane);
    }

    pub fn apply_hook_updates<T: 'static>(
        &self,
        owner: NodeId,
        hook_index: usize,
        render_lanes: Lanes,
        value: &Rc<RefCell<T>>,
    ) {
        let mut inner = self.inner.borrow_mut();
        let Some(updates) = inner.hook_updates.get_mut(&(owner, hook_index)) else {
            return;
        };

        let mut remaining = VecDeque::new();
        while let Some(update) = updates.pop_front() {
            if includes_some_lane(update.lane, render_lanes) {
                (update.apply)(&mut *value.borrow_mut());
            } else {
                remaining.push_back(update);
            }
        }

        if remaining.is_empty() {
            inner.hook_updates.remove(&(owner, hook_index));
        } else {
            *updates = remaining;
        }
    }

    pub fn pending_lanes(&self) -> Lanes {
        self.inner.borrow().lane_root.pending_lanes
    }

    pub fn get_next_lanes(&self, wip_lanes: Lanes) -> Lanes {
        self.inner.borrow().lane_root.get_next_lanes(wip_lanes)
    }

    pub fn mark_render_finished(&self, finished_lanes: Lanes) {
        let mut inner = self.inner.borrow_mut();
        for lanes in inner.dirty_components.values_mut() {
            *lanes &= !finished_lanes;
        }
        inner.dirty_components.retain(|_, lanes| *lanes != NO_LANES);

        let remaining = inner
            .dirty_components
            .values()
            .fold(NO_LANES, |acc, lanes| acc | *lanes)
            | inner
                .hook_updates
                .values()
                .flat_map(|updates| updates.iter().map(|update| update.lane))
                .fold(NO_LANES, |acc, lane| acc | lane);
        inner
            .lane_root
            .mark_root_finished(finished_lanes, remaining);
    }

    pub fn mark_starved_lanes_as_expired(&self, now_ms: u64) {
        self.inner
            .borrow_mut()
            .lane_root
            .mark_starved_lanes_as_expired(now_ms);
    }

    pub fn is_dirty(&self) -> bool {
        self.inner.borrow().lane_root.pending_lanes != NO_LANES
    }
}

#[derive(Clone)]
pub struct State<T> {
    value: Rc<RefCell<T>>,
    owner: NodeId,
    hook_index: usize,
    scheduler: Scheduler,
}

impl<T: Clone + 'static> State<T> {
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    pub fn set(&self, value: T) {
        self.scheduler
            .enqueue_hook_update(self.owner, self.hook_index, move |slot: &mut T| {
                *slot = value;
            });
    }

    pub fn update(&self, update: impl FnOnce(&mut T) + 'static) {
        self.scheduler
            .enqueue_hook_update(self.owner, self.hook_index, update);
    }
}