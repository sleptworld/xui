use xui_interface::{FocusReason, NodeId};

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
}
