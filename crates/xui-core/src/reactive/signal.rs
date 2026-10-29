//! Source handles. Reads are plain `RefCell` borrows and record nothing:
//! who depends on what was decided by `#[component]` and bound at mount.
use std::cell::{Ref, RefCell};
use std::fmt;
use std::rc::Rc;

use super::Cx;
use super::graph::{Graph, SourceId};

struct SourceCell<T> {
    id: SourceId,
    value: RefCell<T>,
}

/// Anything the runtime can subscribe an effect to.
pub trait Source {
    fn id(&self) -> SourceId;
}

macro_rules! handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        pub struct $name<T>(Rc<SourceCell<T>>);

        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                Self(self.0.clone())
            }
        }

        impl<T> Source for $name<T> {
            fn id(&self) -> SourceId {
                self.0.id
            }
        }

        impl<T> $name<T> {
            /// Panics if a write to this source is in progress.
            pub fn read(&self) -> Ref<'_, T> {
                self.0.value.borrow()
            }

            pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
                f(&self.0.value.borrow())
            }

            pub fn get(&self) -> T
            where
                T: Clone,
            {
                self.0.value.borrow().clone()
            }

            /// Same as [`Self::get`] at run time, but `#[component]` does not
            /// count `x.peek()` as a read, so it subscribes nothing.
            pub fn peek(&self) -> T
            where
                T: Clone,
            {
                self.get()
            }

            pub fn read_only(&self) -> ReadSignal<T> {
                ReadSignal(self.0.clone())
            }
        }

        impl<T> From<&$name<T>> for ReadSignal<T> {
            fn from(handle: &$name<T>) -> Self {
                ReadSignal(handle.0.clone())
            }
        }

        /// The current value, for handles printed in untracked code.
        impl<T: fmt::Display> fmt::Display for $name<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.read().fmt(f)
            }
        }

        impl<T: fmt::Debug> fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&*self.read()).finish()
            }
        }
    };
}

handle! {
    /// Writable state.
    Signal
}
handle! {
    /// A derived value; written only by its own computation.
    Memo
}
handle! {
    /// A read-only view of a signal or memo, typically passed down as a prop.
    ReadSignal
}

impl<T> From<Signal<T>> for ReadSignal<T> {
    fn from(handle: Signal<T>) -> Self {
        ReadSignal(handle.0)
    }
}

/// A `#[state]` prop is passed its parent's signal.
impl<T> From<&Signal<T>> for Signal<T> {
    fn from(handle: &Signal<T>) -> Self {
        handle.clone()
    }
}

impl<T> From<Memo<T>> for ReadSignal<T> {
    fn from(handle: Memo<T>) -> Self {
        ReadSignal(handle.0)
    }
}

impl<T> ReadSignal<T> {
    /// A source that never changes, outside any runtime: a `#[source]` prop
    /// of a component mounted with a fixed value.
    pub fn constant(value: T) -> Self {
        Signal::new(SourceId::default(), value).read_only()
    }
}

impl<T> Signal<T> {
    pub(crate) fn new(id: SourceId, value: T) -> Self {
        Self(Rc::new(SourceCell { id, value: RefCell::new(value) }))
    }

    /// Always notifies; there is no equality check on signals.
    pub fn set(&self, cx: &mut Cx<'_>, value: T) {
        *self.0.value.borrow_mut() = value;
        cx.notify(self.0.id);
    }

    pub fn update(&self, cx: &mut Cx<'_>, f: impl FnOnce(&mut T)) {
        f(&mut self.0.value.borrow_mut());
        cx.notify(self.0.id);
    }
}

impl<T: PartialEq> Signal<T> {
    /// A row's data: kept rows are given their new item every time their block runs.
    pub(crate) fn replace_if_changed(&self, graph: &mut Graph, value: T) {
        if *self.0.value.borrow() != value {
            *self.0.value.borrow_mut() = value;
            graph.notify(self.0.id);
        }
    }
}

impl<T: PartialEq> Memo<T> {
    pub(crate) fn new(id: SourceId, value: T) -> Self {
        Self(Rc::new(SourceCell { id, value: RefCell::new(value) }))
    }

    /// Called by the memo's generated computation. Unchanged values stop here.
    #[doc(hidden)]
    pub fn __replace(&self, cx: &mut Cx<'_>, value: T) {
        if *self.0.value.borrow() == value {
            return;
        }
        *self.0.value.borrow_mut() = value;
        cx.notify(self.0.id);
    }
}
