use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::component::ComponentId;

#[derive(Default)]
pub struct HookStorage {
    slots: Vec<Box<dyn Any>>,
    cursor: usize,
}

impl HookStorage {
    pub fn begin(&mut self) {
        self.cursor = 0;
    }

    fn next_slot<T: 'static>(&mut self, init: impl FnOnce() -> T) -> Rc<RefCell<T>> {
        let index = self.cursor;
        self.cursor += 1;

        if index == self.slots.len() {
            self.slots.push(Box::new(Rc::new(RefCell::new(init()))));
        }

        self.slots[index]
            .downcast_ref::<Rc<RefCell<T>>>()
            .expect("hook order changed between rebuilds")
            .clone()
    }
}

pub struct HookContext<'a> {
    storage: &'a mut HookStorage,
    owner: ComponentId,
    scheduler: Scheduler,
}

impl<'a> HookContext<'a> {
    pub fn new(storage: &'a mut HookStorage, owner: ComponentId, scheduler: Scheduler) -> Self {
        storage.begin();
        Self {
            storage,
            owner,
            scheduler,
        }
    }

    pub fn use_state<T: Clone + 'static>(&mut self, init: impl FnOnce() -> T) -> State<T> {
        State {
            value: self.storage.next_slot(init),
            owner: self.owner,
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
    dirty_components: HashSet<ComponentId>,
    root_dirty: bool,
}

impl Scheduler {
    pub fn mark_component_dirty(&self, id: ComponentId) {
        self.inner.borrow_mut().dirty_components.insert(id);
    }

    pub fn mark_root_dirty(&self) {
        self.inner.borrow_mut().root_dirty = true;
    }

    pub fn take_root_dirty(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        let dirty = inner.root_dirty;
        inner.root_dirty = false;
        dirty
    }

    pub fn take_dirty_components(&self) -> Vec<ComponentId> {
        self.inner.borrow_mut().dirty_components.drain().collect()
    }

    pub fn is_dirty(&self) -> bool {
        let inner = self.inner.borrow();
        inner.root_dirty || !inner.dirty_components.is_empty()
    }
}

#[derive(Clone)]
pub struct State<T> {
    value: Rc<RefCell<T>>,
    owner: ComponentId,
    scheduler: Scheduler,
}

impl<T: Clone> State<T> {
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    pub fn set(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.scheduler.mark_component_dirty(self.owner);
    }

    pub fn update(&self, update: impl FnOnce(&mut T)) {
        update(&mut self.value.borrow_mut());
        self.scheduler.mark_component_dirty(self.owner);
    }
}
