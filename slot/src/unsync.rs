use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell, UnsafeCell},
    ptr::NonNull,
};

use crate::{Error, Pointer, Result, Runtime, ScopeStorage, SlotImp, Storage};

const FIRST_GENERATION: u64 = 1;

thread_local! {
    static FREE_SLOTS: RefCell<Vec<NonNull<Slot>>> = const { RefCell::new(Vec::new()) };
}

fn take_free_slot() -> Option<NonNull<Slot>> {
    FREE_SLOTS.with(|slots| slots.borrow_mut().pop())
}

fn recycle_slot(slot: NonNull<Slot>) {
    FREE_SLOTS.with(|slots| slots.borrow_mut().push(slot));
}

/// A single fixed-address slot allocated for a [`Scope`].
pub struct Slot {
    generation: Cell<u64>,
    occupied: Cell<bool>,
    type_id: Cell<TypeId>,
    value: UnsafeCell<Option<Box<dyn Any>>>,
}

impl Slot {
    fn new<T: 'static>(value: T) -> Self {
        Self {
            generation: Cell::new(FIRST_GENERATION),
            occupied: Cell::new(true),
            type_id: Cell::new(TypeId::of::<T>()),
            value: UnsafeCell::new(Some(Box::new(value))),
        }
    }

    fn generation(&self) -> u64 {
        self.generation.get()
    }

    fn insert<T: 'static>(&self, value: T) {
        debug_assert!(!self.occupied.get());

        // SAFETY: insertion requires mutable access to the owning Scope. A
        // vacant slot is not accessible through a valid current-generation
        // pointer, so replacing the empty value cannot alias an existing value.
        unsafe {
            *self.value.get() = Some(Box::new(value));
        }

        self.type_id.set(TypeId::of::<T>());
        self.occupied.set(true);
    }

    fn contains<T: 'static>(&self, generation: u64) -> bool {
        self.validate::<T>(generation).is_ok()
    }

    fn validate<T: 'static>(&self, generation: u64) -> Result<()> {
        if self.generation.get() != generation {
            return Err(Error::Stale);
        }

        if !self.occupied.get() {
            return Err(Error::Vacant);
        }

        if self.type_id.get() != TypeId::of::<T>() {
            return Err(Error::TypeMismatch);
        }

        Ok(())
    }

    fn next_generation(&self) -> Result<u64> {
        self.generation
            .get()
            .checked_add(1)
            .ok_or(Error::GenerationOverflow)
    }

    fn remove<T: 'static>(&self, generation: u64) -> Result<T> {
        self.validate::<T>(generation)?;
        let next_generation = self.next_generation()?;

        // SAFETY: validation proved the slot is occupied with T. `Scope::remove`
        // has `&mut self`, so it cannot race with another safe scope mutation.
        let value = unsafe {
            let storage = &mut *self.value.get();
            let value = storage.take().ok_or(Error::Vacant)?;

            match value.downcast::<T>() {
                Ok(value) => *value,
                Err(value) => {
                    *storage = Some(value);
                    return Err(Error::TypeMismatch);
                }
            }
        };

        self.occupied.set(false);
        self.type_id.set(TypeId::of::<()>());
        self.generation.set(next_generation);

        Ok(value)
    }

    fn clear(&self) {
        // SAFETY: clearing requires mutable access to the owning Scope. Existing
        // handles become stale before this slot can be reused.
        unsafe {
            (*self.value.get()).take();
        }

        self.occupied.set(false);
        self.type_id.set(TypeId::of::<()>());
        self.generation
            .set(self.next_generation().expect("slot generation overflowed"));
    }

    /// Try to read a pointer into this unsynchronized slot storage.
    ///
    /// # Safety
    ///
    /// The caller must ensure the owning scope is still alive and that the
    /// returned reference does not violate Rust's aliasing rules. Debug builds
    /// assert that a runtime phase is active.
    pub unsafe fn try_read<T: 'static>(pointer: Pointer<Self, T>) -> Result<&'static T> {
        unsafe { <Self as Storage<T>>::try_read(pointer) }
    }

    /// Try to mutably read a pointer into this unsynchronized slot storage.
    ///
    /// # Safety
    ///
    /// The caller must ensure the owning scope is still alive and that the
    /// returned mutable reference is unique for the duration of its use. Debug
    /// builds assert that a runtime phase allowing writes is active.
    pub unsafe fn try_write<T: 'static>(pointer: Pointer<Self, T>) -> Result<&'static mut T> {
        unsafe { <Self as Storage<T>>::try_write(pointer) }
    }
}

impl SlotImp for Slot {
    type Ref<'a, T: ?Sized + 'a> = &'a T;
    type Mut<'a, T: ?Sized + 'a> = &'a mut T;
}

impl<T: 'static> Storage<T> for Slot {
    unsafe fn try_read(pointer: Pointer<Self, T>) -> Result<&'static T> {
        Runtime::debug_assert_slot_read();

        // SAFETY: the caller guarantees that the slot pointer came from a live
        // scope allocation.
        let slot = unsafe { pointer.slot().as_ref() };
        slot.validate::<T>(pointer.generation())?;

        // SAFETY: validation ensures the value is present and has type T. The
        // caller is responsible for the returned 'static reference not outliving
        // the owning scope in practice.
        unsafe {
            let value = (*slot.value.get()).as_ref().ok_or(Error::Vacant)?;
            let value = value.downcast_ref::<T>().ok_or(Error::TypeMismatch)?;
            Ok(&*(value as *const T))
        }
    }

    unsafe fn try_write(pointer: Pointer<Self, T>) -> Result<&'static mut T> {
        Runtime::debug_assert_slot_write();

        // SAFETY: the caller guarantees that the slot pointer came from a live
        // scope allocation.
        let slot = unsafe { pointer.slot().as_ref() };
        slot.validate::<T>(pointer.generation())?;

        // SAFETY: validation ensures the value is present and has type T. The
        // caller is responsible for uniqueness of the returned mutable reference.
        unsafe {
            let value = (*slot.value.get()).as_mut().ok_or(Error::Vacant)?;
            let value = value.downcast_mut::<T>().ok_or(Error::TypeMismatch)?;
            Ok(&mut *(value as *mut T))
        }
    }
}

impl ScopeStorage for Slot {
    fn allocate<T: 'static>(value: T) -> Pointer<Self, T> {
        if let Some(slot) = take_free_slot() {
            // SAFETY: all thread-local free-list entries are leaked Slot
            // allocations that are vacant and ready to be reused.
            let slot_ref = unsafe { slot.as_ref() };
            slot_ref.insert(value);
            return Pointer::new(slot, slot_ref.generation());
        }

        let slot = Box::leak(Box::new(Slot::new(value)));
        let slot = NonNull::from(slot);
        Pointer::new(slot, FIRST_GENERATION)
    }

    fn recycle(slot: NonNull<Self>) {
        recycle_slot(slot);
    }

    unsafe fn remove<T: 'static>(slot: NonNull<Self>, generation: u64) -> Result<T> {
        // SAFETY: the caller guarantees this is a live slot allocated by this
        // storage backend.
        unsafe { slot.as_ref().remove(generation) }
    }

    unsafe fn contains<T: 'static>(slot: NonNull<Self>, generation: u64) -> bool {
        // SAFETY: the caller guarantees this is a live slot allocated by this
        // storage backend.
        unsafe { slot.as_ref().contains::<T>(generation) }
    }

    unsafe fn clear(slot: NonNull<Self>) {
        // SAFETY: the caller guarantees this is a live slot allocated by this
        // storage backend.
        unsafe { slot.as_ref().clear() };
    }
}

/// An unsynchronized scope backed by [`Slot`] storage.
pub type Scope = crate::Scope<Slot>;

/// Backwards-compatible name for the previous arena-shaped API.
pub type Arena = Scope;

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use crate::{Error, Pointer, RenderPhase, Result, Runtime, Scope};

    use super::Slot;

    fn read<T: 'static>(pointer: Pointer<Slot, T>) -> Result<&'static T> {
        Runtime::with_phase(RenderPhase::Render, || unsafe { pointer.try_read() })
    }

    fn write<T: 'static>(pointer: Pointer<Slot, T>) -> Result<&'static mut T> {
        Runtime::with_phase(RenderPhase::Event, || unsafe { pointer.try_write() })
    }

    #[test]
    fn insert_read_and_write_same_type() {
        let mut scope = Scope::new();
        let pointer = scope.insert(7_u32);

        assert_eq!(*read(pointer).unwrap(), 7);

        *write(pointer).unwrap() = 11;

        assert_eq!(*read(pointer).unwrap(), 11);
        assert!(scope.contains(pointer));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "slot read requires an active runtime phase")]
    fn read_requires_active_runtime_phase() {
        let mut scope = Scope::new();
        let pointer = scope.insert(7_u32);

        let _ = unsafe { pointer.try_read() };
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "slot write is not allowed during Render phase")]
    fn write_panics_during_render_phase() {
        let mut scope = Scope::new();
        let pointer = scope.insert(7_u32);

        let _ = Runtime::with_phase(RenderPhase::Render, || unsafe { pointer.try_write() });
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "slot write is not allowed during Commit phase")]
    fn write_panics_during_commit_phase() {
        let mut scope = Scope::new();
        let pointer = scope.insert(7_u32);

        let _ = Runtime::with_phase(RenderPhase::Commit, || unsafe { pointer.try_write() });
    }

    #[test]
    fn write_is_allowed_during_event_and_effect_phase() {
        let mut scope = Scope::new();
        let pointer = scope.insert(7_u32);

        *Runtime::with_phase(RenderPhase::Effect, || unsafe { pointer.try_write() }).unwrap() = 9;
        *write(pointer).unwrap() = 11;

        assert_eq!(*read(pointer).unwrap(), 11);
    }

    #[test]
    fn runtime_phase_is_restored_after_enter() {
        let runtime = Runtime::new(RenderPhase::Event);

        assert_eq!(Runtime::current_render_phase(), None);
        runtime.enter(|| {
            assert_eq!(Runtime::current_render_phase(), Some(RenderPhase::Event));
            Runtime::with_phase(RenderPhase::Render, || {
                assert_eq!(Runtime::current_render_phase(), Some(RenderPhase::Render));
            });
            assert_eq!(Runtime::current_render_phase(), Some(RenderPhase::Event));
        });
        assert_eq!(Runtime::current_render_phase(), None);
    }

    #[test]
    fn copied_pointer_reads_same_slot() {
        let mut scope = Scope::new();
        let pointer = scope.insert(String::from("hello"));
        let copied = pointer;

        assert_eq!(pointer, copied);
        assert_eq!(read(copied).unwrap().as_str(), "hello");
    }

    #[test]
    fn remove_invalidates_old_pointer() {
        let mut scope = Scope::new();
        let pointer = scope.insert(12_i32);

        assert_eq!(scope.remove(pointer).unwrap(), 12);
        assert!(!scope.contains(pointer));
        assert_eq!(read(pointer).unwrap_err(), Error::Stale);
    }

    #[test]
    fn reuse_keeps_slot_address_and_changes_generation() {
        let mut scope = Scope::new();
        let first = scope.insert(1_u32);
        let first_slot = first.slot_ptr();
        let first_generation = first.generation();

        assert_eq!(scope.remove(first).unwrap(), 1);

        let second = scope.insert(String::from("next"));

        assert_eq!(first_slot, second.slot_ptr());
        assert_ne!(first_generation, second.generation());
        assert_eq!(read(first).unwrap_err(), Error::Stale);
        assert_eq!(read(second).unwrap().as_str(), "next");
    }

    #[test]
    fn scope_stores_multiple_types() {
        #[derive(Debug, PartialEq)]
        struct Widget {
            id: u64,
        }

        let mut scope = Scope::new();
        let number = scope.insert(99_u64);
        let text = scope.insert(String::from("slot"));
        let widget = scope.insert(Widget { id: 5 });

        assert_eq!(*read(number).unwrap(), 99);
        assert_eq!(read(text).unwrap().as_str(), "slot");
        assert_eq!(read(widget).unwrap(), &Widget { id: 5 });
    }

    #[test]
    fn wrong_type_cast_returns_type_mismatch() {
        let mut scope = Scope::new();
        let number = scope.insert(99_u64);
        let wrong: Pointer<Slot, String> = unsafe { number.cast() };

        assert_eq!(read(wrong).unwrap_err(), Error::TypeMismatch);
        assert!(!scope.contains(wrong));
    }

    #[test]
    fn remove_returns_owned_value() {
        let mut scope = Scope::new();
        let pointer = scope.insert(String::from("owned"));

        let value = scope.remove(pointer).unwrap();

        assert_eq!(value, "owned");
    }

    #[test]
    fn clear_invalidates_existing_pointers() {
        let mut scope = Scope::new();
        let first = scope.insert(1_u32);
        let second = scope.insert(String::from("second"));

        scope.clear();

        assert!(!scope.contains(first));
        assert!(!scope.contains(second));
        assert_eq!(read(first).unwrap_err(), Error::Stale);
        assert_eq!(read(second).unwrap_err(), Error::Stale);
    }

    #[test]
    fn scope_drop_releases_values() {
        struct DropCounter(Rc<Cell<usize>>);

        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        {
            let mut scope = Scope::new();
            scope.insert(DropCounter(drops.clone()));
        }

        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn scope_drop_returns_slots_to_thread_local_free_pool() {
        let pointer;
        let slot_ptr;
        let generation;
        {
            let mut scope = Scope::new();
            pointer = scope.insert(5_u16);
            slot_ptr = pointer.slot_ptr();
            generation = pointer.generation();
        }

        assert_eq!(read(pointer).unwrap_err(), Error::Stale);

        let mut scope = Scope::new();
        let reused = scope.insert(String::from("reused"));

        assert_eq!(reused.slot_ptr(), slot_ptr);
        assert_ne!(reused.generation(), generation);
        assert_eq!(read(reused).unwrap().as_str(), "reused");
    }

    #[test]
    fn remove_rejects_foreign_pointer() {
        let mut first_scope = Scope::new();
        let mut second_scope = Scope::new();
        let pointer = first_scope.insert(1_u8);

        assert_eq!(second_scope.remove(pointer).unwrap_err(), Error::Foreign);
        assert_eq!(*read(pointer).unwrap(), 1);
        assert_eq!(first_scope.remove(pointer).unwrap(), 1);
    }
}
