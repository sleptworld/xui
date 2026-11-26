//! The direct repaint channel: how a widget's own handle reaches the nodes
//! showing it, without a component rebuild.

use std::{cell::RefCell, rc::Rc};

use xui_interface::{NodeId, WidgetUpdateFlags};

/// One window's queue of repaints requested from outside the component tree.
///
/// A widget whose content lives in a handle of its own -- a
/// [`super::CanvasController`], a [`super::TextSurfaceHandle`] -- changes
/// nothing about layout or the component tree when that content changes, so
/// repainting it through a component rebuild pays for a full reconcile to move
/// some pixels. The handle marks the host node through this queue instead, and
/// the runtime reports the pending work through `is_dirty`, which is what gets
/// the frame scheduled.
#[derive(Clone, Default)]
pub(crate) struct RepaintQueue {
    pending: Rc<RefCell<Vec<(NodeId, WidgetUpdateFlags)>>>,
    wake: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl RepaintQueue {
    /// Whether the two are the same queue, which is to say the same window.
    pub(crate) fn same_queue(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.pending, &other.pending)
    }

    pub(crate) fn set_wake(&self, wake: Rc<dyn Fn()>) {
        *self.wake.borrow_mut() = Some(wake);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.borrow().is_empty()
    }

    pub(crate) fn drain(&self) -> Vec<(NodeId, WidgetUpdateFlags)> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    pub(crate) fn push(&self, id: NodeId, flags: WidgetUpdateFlags) {
        self.pending.borrow_mut().push((id, flags));
        let wake = self.wake.borrow().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

/// The mounted nodes a handle repaints, each with the queue of the window it
/// is in.
///
/// A node id alone does not name a node to repaint: every window drains its
/// own queue, and one that finds another window's node in it drops it. So
/// a handle drawn in two windows has to send each node to its own.
#[derive(Clone, Default)]
pub(crate) struct RepaintBindings(Vec<(NodeId, RepaintQueue)>);

impl RepaintBindings {
    pub(crate) fn bind(&mut self, id: NodeId, invalidator: RepaintQueue) {
        let bound = self
            .0
            .iter()
            .any(|(node, queue)| *node == id && queue.same_queue(&invalidator));
        if !bound {
            self.0.push((id, invalidator));
        }
    }

    pub(crate) fn unbind(&mut self, id: NodeId, invalidator: &RepaintQueue) {
        self.0
            .retain(|(node, queue)| !(*node == id && queue.same_queue(invalidator)));
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Queues `flags` on every bound node, in its own window.
    ///
    /// Takes the bindings by value: a push wakes the host, which may reach
    /// back into the handle that holds them.
    pub(crate) fn push_all(self, flags: WidgetUpdateFlags) {
        for (id, invalidator) in self.0 {
            invalidator.push(id, flags);
        }
    }
}
