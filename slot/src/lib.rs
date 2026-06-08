use std::{
    cell::Cell,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};
pub mod error;
pub mod unsync;

pub use crate::error::{Error, Result};

thread_local! {
    static CURRENT_RENDER_PHASE: Cell<Option<RenderPhase>> = const { Cell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderPhase {
    Render,
    Event,
    Effect,
    Commit,
}
pub struct Runtime {
    render_phase: RenderPhase,
}

impl Runtime {
    pub fn new(render_phase: RenderPhase) -> Self {
        Self { render_phase }
    }

    pub fn render_phase(&self) -> RenderPhase {
        self.render_phase
    }

    pub fn set_render_phase(&mut self, render_phase: RenderPhase) {
        self.render_phase = render_phase;
    }

    pub fn current_render_phase() -> Option<RenderPhase> {
        CURRENT_RENDER_PHASE.with(Cell::get)
    }

    pub fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        Self::with_phase(self.render_phase, f)
    }

    pub fn with_phase<R>(render_phase: RenderPhase, f: impl FnOnce() -> R) -> R {
        CURRENT_RENDER_PHASE.with(|current| {
            struct PhaseGuard<'a> {
                current: &'a Cell<Option<RenderPhase>>,
                previous: Option<RenderPhase>,
            }

            impl Drop for PhaseGuard<'_> {
                fn drop(&mut self) {
                    self.current.set(self.previous);
                }
            }

            let previous = current.replace(Some(render_phase));
            let _guard = PhaseGuard { current, previous };
            f()
        })
    }

    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn debug_assert_slot_read() {
        #[cfg(debug_assertions)]
        {
            match Self::current_render_phase() {
                Some(phase) if phase.allows_slot_read() => {}
                Some(phase) => panic!("slot read is not allowed during {phase:?} phase"),
                None => panic!("slot read requires an active runtime phase"),
            }
        }
    }

    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn debug_assert_slot_write() {
        #[cfg(debug_assertions)]
        {
            match Self::current_render_phase() {
                Some(phase) if phase.allows_slot_write() => {}
                Some(phase) => panic!("slot write is not allowed during {phase:?} phase"),
                None => panic!("slot write requires an active runtime phase"),
            }
        }
    }
}

impl RenderPhase {
    pub fn allows_slot_read(self) -> bool {
        true
    }

    pub fn allows_slot_write(self) -> bool {
        matches!(self, Self::Event | Self::Effect)
    }
}

pub trait SlotImp {
    type Ref<'a, T: ?Sized + 'a>: Deref<Target = T>;
    type Mut<'a, T: ?Sized + 'a>: DerefMut<Target = T>;
}

pub trait Storage<Data: 'static = ()>: SlotImp + Sized {
    /// Try to read the value referenced by `p`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the scope that owns `p` is still alive and that
    /// returning this reference does not violate Rust's aliasing rules. Debug
    /// builds assert that a runtime phase allowing slot reads is active.
    unsafe fn try_read(p: Pointer<Self, Data>) -> Result<Self::Ref<'static, Data>>;

    /// Try to mutably read the value referenced by `p`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the scope that owns `p` is still alive and that
    /// the returned mutable reference is unique for the duration of its use.
    /// Debug builds assert that a runtime phase allowing slot writes is active.
    unsafe fn try_write(p: Pointer<Self, Data>) -> Result<Self::Mut<'static, Data>>;
}

pub trait ScopeStorage: Sized {
    fn allocate<T: 'static>(value: T) -> Pointer<Self, T>;

    fn recycle(slot: NonNull<Self>);

    /// Remove and return the value currently stored in `slot`.
    ///
    /// # Safety
    ///
    /// `slot` must be a live slot allocated by this storage backend.
    unsafe fn remove<T: 'static>(slot: NonNull<Self>, generation: u64) -> Result<T>;

    /// Return whether `slot` currently contains a value of type `T` at `generation`.
    ///
    /// # Safety
    ///
    /// `slot` must be a live slot allocated by this storage backend.
    unsafe fn contains<T: 'static>(slot: NonNull<Self>, generation: u64) -> bool;

    /// Clear the current value in `slot` and invalidate existing handles.
    ///
    /// # Safety
    ///
    /// `slot` must be a live slot allocated by this storage backend.
    unsafe fn clear(slot: NonNull<Self>);
}

/// Owns the lifetime of a set of allocated slots for a storage backend.
pub struct Scope<S: ScopeStorage = unsync::Slot> {
    slots: Vec<NonNull<S>>,
}

impl<S: ScopeStorage> Scope<S> {
    pub fn insert<T: 'static>(&mut self, value: T) -> Pointer<S, T> {
        let pointer = S::allocate(value);
        self.slots.push(pointer.slot());
        pointer
    }

    pub fn remove<T: 'static>(&mut self, pointer: Pointer<S, T>) -> Result<T> {
        let slot = pointer.slot();
        let Some(index) = self.index_of(slot) else {
            return Err(Error::Foreign);
        };

        // SAFETY: ownership was checked against this scope's active slots.
        let value = unsafe { S::remove(slot, pointer.generation())? };
        self.slots.swap_remove(index);
        S::recycle(slot);

        Ok(value)
    }

    pub fn contains<T: 'static>(&self, pointer: Pointer<S, T>) -> bool {
        let slot = pointer.slot();
        if !self.owns(slot) {
            return false;
        }

        // SAFETY: ownership was checked against this scope's active slots.
        unsafe { S::contains::<T>(slot, pointer.generation()) }
    }

    pub fn clear(&mut self) {
        for slot in self.slots.drain(..) {
            // SAFETY: every pointer in `self.slots` is currently allocated to
            // this scope.
            unsafe { S::clear(slot) };
            S::recycle(slot);
        }
    }

    fn owns(&self, slot: NonNull<S>) -> bool {
        self.index_of(slot).is_some()
    }

    fn index_of(&self, slot: NonNull<S>) -> Option<usize> {
        self.slots.iter().position(|candidate| *candidate == slot)
    }
}

impl Scope<unsync::Slot> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<S: ScopeStorage> Default for Scope<S> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<S: ScopeStorage> Drop for Scope<S> {
    fn drop(&mut self) {
        self.clear();
    }
}

pub struct Pointer<S: ?Sized, T> {
    slot: NonNull<S>,
    generation: u64,
    _marker: PhantomData<fn() -> T>,
}

impl<S: ?Sized, T> Pointer<S, T> {
    pub(crate) fn new(slot: NonNull<S>, generation: u64) -> Self {
        Self {
            slot,
            generation,
            _marker: PhantomData,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn slot_ptr(&self) -> *const S {
        self.slot.as_ptr()
    }

    pub(crate) fn slot(&self) -> NonNull<S> {
        self.slot
    }

    /// Reinterpret this pointer as targeting another value type.
    ///
    /// # Safety
    ///
    /// The caller is responsible for using the returned pointer only with APIs
    /// that can tolerate a runtime type mismatch.
    pub unsafe fn cast<U>(self) -> Pointer<S, U> {
        Pointer::new(self.slot, self.generation)
    }
}

impl<S, T> Pointer<S, T>
where
    S: Storage<T>,
    T: 'static,
{
    /// Try to read the value referenced by this pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure the scope that owns this pointer is still alive
    /// and that returning this reference does not violate Rust's aliasing rules.
    /// Debug builds assert that a runtime phase allowing slot reads is active.
    pub unsafe fn try_read(self) -> Result<S::Ref<'static, T>> {
        unsafe { S::try_read(self) }
    }

    /// Try to mutably read the value referenced by this pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure the scope that owns this pointer is still alive
    /// and that the returned mutable reference is unique for the duration of
    /// its use. Debug builds assert that a runtime phase allowing slot writes is active.
    pub unsafe fn try_write(self) -> Result<S::Mut<'static, T>> {
        unsafe { S::try_write(self) }
    }
}

impl<S: ?Sized, T> Copy for Pointer<S, T> {}

impl<S: ?Sized, T> Clone for Pointer<S, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: ?Sized, T> PartialEq<Pointer<S, T>> for Pointer<S, T> {
    fn eq(&self, other: &Pointer<S, T>) -> bool {
        std::ptr::eq(self.slot.as_ptr(), other.slot.as_ptr()) && self.generation == other.generation
    }
}

impl<S: ?Sized, T> Eq for Pointer<S, T> {}

impl<S: ?Sized, T> Hash for Pointer<S, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.slot.hash(state);
        self.generation.hash(state);
    }
}

impl<S: ?Sized, T> fmt::Debug for Pointer<S, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pointer")
            .field("slot", &self.slot)
            .field("generation", &self.generation)
            .finish()
    }
}
