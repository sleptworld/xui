use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
    dirty_signal: Rc<Cell<bool>>,
}

impl<'a> HookContext<'a> {
    pub fn new(storage: &'a mut HookStorage, dirty_signal: Rc<Cell<bool>>) -> Self {
        storage.begin();
        Self {
            storage,
            dirty_signal,
        }
    }

    pub fn use_state<T: Clone + 'static>(&mut self, init: impl FnOnce() -> T) -> State<T> {
        State {
            value: self.storage.next_slot(init),
            dirty_signal: self.dirty_signal.clone(),
        }
    }
}

#[derive(Clone)]
pub struct State<T> {
    value: Rc<RefCell<T>>,
    dirty_signal: Rc<Cell<bool>>,
}

impl<T: Clone> State<T> {
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    pub fn set(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.dirty_signal.set(true);
    }

    pub fn update(&self, update: impl FnOnce(&mut T)) {
        update(&mut self.value.borrow_mut());
        self.dirty_signal.set(true);
    }
}
