//! Observe a widget's size after layout, without changing its layout rules.
//!
//! ```ignore
//! #[component]
//! fn panel() {
//!     let size: Option<Size<f32>> = size!(); // None until laid out
//!     view! { container(size_observer: size) { text(format!("{size:?}")) } }
//! }
//! ```

use std::cell::Cell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use xui_interface::{NodeId, Size};


struct SizeState {
    node: Cell<Option<NodeId>>,
    size: Cell<Option<Size<f32>>>,
    /// Told the size whenever it changes.
    notify: Box<dyn Fn(Option<Size<f32>>)>,
}

/// What binds a host to a `size!()` source.
///
/// Bind it to one widget using `.with::<container::size_observer>(observer.clone())`. Sizes are
/// the final layout border box in logical pixels, including padding, excluding
/// transforms and overflowing content. A change writes the source, which the
/// next flush propagates. There is no layout containment and no
/// layout/rebuild loop before painting.
#[derive(Clone)]
pub struct SizeObserver {
    inner: Rc<SizeState>,
}

impl SizeObserver {
    /// An observer that writes the size into `signal`: what `size!()` binds.
    pub(crate) fn reactive(
        signal: crate::reactive::Signal<Option<Size<f32>>>,
        notifier: crate::reactive::Notifier,
    ) -> Self {
        Self::with_notify(Box::new(move |size| {
            signal.set(&mut crate::reactive::Cx::handler(&notifier), size)
        }))
    }

    fn with_notify(notify: Box<dyn Fn(Option<Size<f32>>)>) -> Self {
        Self {
            inner: Rc::new(SizeState {
                node: Cell::new(None),
                size: Cell::new(None),
                notify,
            }),
        }
    }

    /// The latest completed layout size, or `None` before layout or after
    /// detaching the observer. Reading does not trigger layout.
    pub fn get(&self) -> Option<Size<f32>> {
        self.inner.size.get()
    }

    pub(crate) fn bind(&self, node: NodeId) {
        assert!(
            self.inner.node.get().is_none_or(|bound| bound == node),
            "a use_size handle can observe only one widget at a time"
        );
        self.inner.node.set(Some(node));
    }

    pub(crate) fn unbind(&self, node: NodeId) -> bool {
        if self.inner.node.get() == Some(node) {
            self.inner.node.set(None);
            return self.inner.size.take().is_some();
        }
        false
    }

    pub(crate) fn offer(&self, node: NodeId, size: Size<f32>) {
        if self.inner.node.get() == Some(node) {
            self.publish(Some(size));
        }
    }

    fn publish(&self, size: Option<Size<f32>>) {
        if self.inner.size.replace(size) != size {
            self.notify();
        }
    }

    pub(crate) fn notify(&self) {
        (self.inner.notify)(self.inner.size.get());
    }
}

impl fmt::Debug for SizeObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SizeObserver")
            .field("node", &self.inner.node.get())
            .field("size", &self.get())
            .finish()
    }
}

impl PartialEq for SizeObserver {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for SizeObserver {}

impl Hash for SizeObserver {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.inner).hash(state);
    }
}
