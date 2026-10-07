use std::cell::RefCell;
use std::rc::Rc;

use xui_interface::{DropAccepts, DropEffect, Point, PublishedSurface, WindowRegion};

/// A clonable, loop-thread-only handle to the latest [`PublishedSurface`].
///
/// [`crate::Shell`] writes it after every runtime step. A host reads it from any
/// native callback that needs an answer on the spot -- `WM_NCHITTEST`,
/// `draggingUpdated:` -- without borrowing the shell, which may be in the middle
/// of an event when the callback arrives.
#[derive(Debug, Clone, Default)]
pub struct PublishedHandle(Rc<RefCell<PublishedSurface>>);

impl PublishedHandle {
    /// The published surface as it is now. Cheap: its parts are shared.
    pub fn get(&self) -> PublishedSurface {
        self.0.borrow().clone()
    }

    /// What the point at `point` means to the platform.
    pub fn region_at(&self, point: Point) -> WindowRegion {
        self.0.borrow().region_at(point)
    }

    /// What a drag carrying `kinds` would do at `point`.
    pub fn drop_effect_at(&self, point: Point, kinds: DropAccepts) -> DropEffect {
        self.0.borrow().drop_effect_at(point, kinds)
    }

    pub(crate) fn set(&self, surface: &PublishedSurface) {
        if *self.0.borrow() != *surface {
            *self.0.borrow_mut() = surface.clone();
        }
    }
}
