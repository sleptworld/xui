use std::cell::Cell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use xui_interface::{FocusReason, NodeId};

/// A stable handle that follows a host node across component rebuilds.
///
/// Attach it with `widget.focus_handle(handle.clone())`, then call
/// [`FocusHandle::request_focus`] from any event handler to focus that widget.
#[derive(Clone, Default)]
pub struct FocusHandle {
    node: Rc<Cell<Option<NodeId>>>,
}

impl FocusHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node_id(&self) -> Option<NodeId> {
        self.node.get()
    }

    pub fn is_bound(&self) -> bool {
        self.node_id().is_some()
    }

    /// Queues focus for the node currently bound to this handle.
    ///
    /// Returns `false` when the widget has not mounted or has been removed.
    pub fn request_focus(&self, cx: &mut crate::event_system::EventContext<'_>) -> bool {
        let Some(node) = self.node_id() else {
            return false;
        };
        cx.request_focus_node(node);
        true
    }

    pub(crate) fn bind(&self, node: NodeId) {
        self.node.set(Some(node));
    }

    pub(crate) fn unbind(&self, node: NodeId) {
        if self.node.get() == Some(node) {
            self.node.set(None);
        }
    }
}

impl fmt::Debug for FocusHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FocusHandle")
            .field("node", &self.node_id())
            .finish()
    }
}

impl PartialEq for FocusHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.node, &other.node)
    }
}

impl Eq for FocusHandle {}

impl Hash for FocusHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.node).hash(state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusRequest {
    pub target: Option<NodeId>,
    pub reason: FocusReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusTransition {
    pub old: Option<NodeId>,
    pub new: Option<NodeId>,
    pub reason: FocusReason,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FocusManager {
    focused: Option<NodeId>,
    pending: Option<FocusRequest>,
}

impl FocusManager {
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    pub(crate) fn request_focus(&mut self, target: Option<NodeId>, reason: FocusReason) {
        self.pending = Some(FocusRequest { target, reason });
    }

    pub(crate) fn take_request(&mut self) -> Option<FocusRequest> {
        self.pending.take()
    }

    pub(crate) fn commit(
        &mut self,
        target: Option<NodeId>,
        reason: FocusReason,
    ) -> Option<FocusTransition> {
        if self.focused == target {
            return None;
        }
        let transition = FocusTransition {
            old: self.focused,
            new: target,
            reason,
        };
        self.focused = target;
        Some(transition)
    }

    pub(crate) fn clear_node(&mut self, node: NodeId) {
        if self.focused == Some(node) {
            self.focused = None;
        }
        if self
            .pending
            .is_some_and(|request| request.target == Some(node))
        {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::KeyData;

    fn node(value: u64) -> NodeId {
        NodeId::from(KeyData::from_ffi(value))
    }

    #[test]
    fn commit_is_atomic_and_idempotent() {
        let mut manager = FocusManager::default();
        let first = node(1);
        let transition = manager
            .commit(Some(first), FocusReason::Programmatic)
            .unwrap();
        assert_eq!((transition.old, transition.new), (None, Some(first)));
        assert_eq!(manager.focused(), Some(first));
        assert!(manager.commit(Some(first), FocusReason::Keyboard).is_none());
    }

    #[test]
    fn latest_request_wins_and_removal_clears_state() {
        let mut manager = FocusManager::default();
        let first = node(1);
        let second = node(2);
        manager.request_focus(Some(first), FocusReason::Programmatic);
        manager.request_focus(Some(second), FocusReason::Programmatic);
        assert_eq!(manager.take_request().unwrap().target, Some(second));
        manager.commit(Some(second), FocusReason::Programmatic);
        manager.clear_node(second);
        assert_eq!(manager.focused(), None);
    }

    #[test]
    fn focus_handle_only_unbinds_its_current_node() {
        let handle = FocusHandle::new();
        let first = node(1);
        let second = node(2);
        handle.bind(first);
        handle.bind(second);
        handle.unbind(first);
        assert_eq!(handle.node_id(), Some(second));
        handle.unbind(second);
        assert!(!handle.is_bound());
    }
}
