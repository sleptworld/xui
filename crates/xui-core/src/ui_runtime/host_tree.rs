use std::ops::{Index, IndexMut};

use slotmap::{SecondaryMap, SlotMap};
use xui_interface::{NodeId, WidgetState};

use crate::fiber::Key;
use crate::ui_runtime::state::HostWorkFlags;
use crate::widgets::{WidgetI, WidgetType};

/// Dense, high-frequency host data keyed by the topology's generational id.
pub(crate) struct HostData {
    pub node_type: WidgetType,
    /// Cached at creation: raw dispatch consults it to skip nodes whose widget
    /// never looks at `EventRef::Raw`.
    pub reads_raw_events: bool,
    pub key: Option<Key>,
    pub work: HostWorkFlags,
    pub subtree_work: HostWorkFlags,
    pub old_props_hash: u64,
    pub new_props_hash: u64,
    pub state: WidgetState,
    pub state_before_change: Option<WidgetState>,
    pub widget: WidgetI,
}

impl HostData {
    pub(crate) fn new(key: Option<Key>, props_hash: u64, widget: WidgetI) -> Self {
        let node_type = widget.node_type();
        let reads_raw_events = widget.reads_raw_events();
        Self {
            node_type,
            reads_raw_events,
            key,
            work: HostWorkFlags::empty(),
            subtree_work: HostWorkFlags::empty(),
            old_props_hash: 0,
            new_props_hash: props_hash,
            state: WidgetState::default(),
            state_before_change: None,
            widget,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HostNode {
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    pub child_count: usize,
}

impl HostNode {
    fn new(_: NodeId) -> Self {
        Self {
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            child_count: 0,
        }
    }
}

/// Generational host identity/topology plus a secondary core-data cache.
///
/// # Preconditions
///
/// Every method taking a `NodeId` requires it to be live -- present in the
/// tree -- and panics otherwise, except the one that exists to ask:
/// [`Self::contains_key`].
pub(crate) struct HostTree<D> {
    nodes: SlotMap<NodeId, HostNode>,
    data: SecondaryMap<NodeId, D>,
}

impl<D> HostTree<D> {
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            data: SecondaryMap::new(),
        }
    }

    pub fn insert_with_key(&mut self, make_data: impl FnOnce(NodeId) -> D) -> NodeId {
        let id = self.nodes.insert_with_key(HostNode::new);
        self.data.insert(id, make_data(id));
        id
    }

    #[inline]
    pub fn contains_key(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    #[inline]
    pub fn link(&self, id: NodeId) -> &HostNode {
        &self.nodes[id]
    }

    #[inline]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id].parent
    }

    pub fn children(&self, parent: NodeId) -> Children<'_> {
        let node = &self.nodes[parent];
        Children {
            links: &self.nodes,
            front: node.first_child,
            back: node.last_child,
            remaining: node.child_count,
        }
    }

    /// `id` and then each of its ancestors, root last.
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        debug_assert!(self.nodes.contains_key(id));
        Ancestors {
            links: &self.nodes,
            next: Some(id),
        }
    }

    /// `root` and everything under it, in document order.
    pub fn subtree(&self, root: NodeId) -> Dfs<'_> {
        debug_assert!(self.nodes.contains_key(root));
        Dfs {
            links: &self.nodes,
            stack: vec![root],
        }
    }

    /// Links `child` under `parent`, before `before` or last.
    ///
    /// # Preconditions
    /// - `child` has no parent.
    /// - `before`, if given, is a child of `parent`.
    /// - `child` is not `parent` or one of its ancestors.
    pub fn insert(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        debug_assert!(
            self.nodes[child].parent.is_none(),
            "detach before inserting"
        );
        debug_assert!(
            !self.ancestors(parent).any(|ancestor| ancestor == child),
            "attaching would create a cycle"
        );
        let previous = match before {
            Some(before) => {
                debug_assert_eq!(self.nodes[before].parent, Some(parent));
                let previous = self.nodes[before].prev_sibling;
                self.nodes[before].prev_sibling = Some(child);
                previous
            }
            None => {
                let previous = self.nodes[parent].last_child;
                self.nodes[parent].last_child = Some(child);
                previous
            }
        };
        let child_node = &mut self.nodes[child];
        child_node.parent = Some(parent);
        child_node.prev_sibling = previous;
        child_node.next_sibling = before;
        match previous {
            Some(previous) => self.nodes[previous].next_sibling = Some(child),
            None => self.nodes[parent].first_child = Some(child),
        }
        self.nodes[parent].child_count += 1;
    }

    /// Unlinks `child` from its parent. Returns the parent it had.
    pub fn detach(&mut self, child: NodeId) -> Option<NodeId> {
        let node = &mut self.nodes[child];
        let parent = node.parent.take()?;
        let previous = node.prev_sibling.take();
        let next = node.next_sibling.take();

        match previous {
            Some(previous) => self.nodes[previous].next_sibling = next,
            None => self.nodes[parent].first_child = next,
        }
        match next {
            Some(next) => self.nodes[next].prev_sibling = previous,
            None => self.nodes[parent].last_child = previous,
        }
        self.nodes[parent].child_count -= 1;
        Some(parent)
    }

    /// Removes `id`, which must have no children left.
    pub fn remove(&mut self, id: NodeId) -> D {
        debug_assert_eq!(
            self.nodes[id].child_count, 0,
            "remove children before their parent"
        );
        self.detach(id);
        self.nodes.remove(id);
        self.data.remove(id).expect("host data is dense")
    }
}

impl<D> Default for HostTree<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D> Index<NodeId> for HostTree<D> {
    type Output = D;

    #[inline]
    fn index(&self, index: NodeId) -> &Self::Output {
        &self.data[index]
    }
}

impl<D> IndexMut<NodeId> for HostTree<D> {
    #[inline]
    fn index_mut(&mut self, index: NodeId) -> &mut Self::Output {
        &mut self.data[index]
    }
}

type Links = SlotMap<NodeId, HostNode>;

pub struct Children<'a> {
    links: &'a Links,
    front: Option<NodeId>,
    back: Option<NodeId>,
    remaining: usize,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.front?;
        self.front = self.links[id].next_sibling;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.front = None;
            self.back = None;
        }
        Some(id)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl DoubleEndedIterator for Children<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let id = self.back?;
        self.back = self.links[id].prev_sibling;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.front = None;
            self.back = None;
        }
        Some(id)
    }
}

impl ExactSizeIterator for Children<'_> {}

pub struct Ancestors<'a> {
    links: &'a Links,
    next: Option<NodeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.next?;
        self.next = self.links[id].parent;
        Some(id)
    }
}

pub struct Dfs<'a> {
    links: &'a Links,
    stack: Vec<NodeId>,
}

impl Iterator for Dfs<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        let mut child = self.links[id].last_child;
        while let Some(current) = child {
            self.stack.push(current);
            child = self.links[current].prev_sibling;
        }
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (HostTree<&'static str>, NodeId, NodeId, NodeId, NodeId) {
        let mut tree = HostTree::new();
        let root = tree.insert_with_key(|_| "root");
        let a = tree.insert_with_key(|_| "a");
        let b = tree.insert_with_key(|_| "b");
        let c = tree.insert_with_key(|_| "c");
        tree.insert(root, a, None);
        tree.insert(root, b, None);
        tree.insert(a, c, None);
        (tree, root, a, b, c)
    }

    #[test]
    fn maintains_links_when_moving_and_inserting() {
        let (mut tree, root, a, b, c) = fixture();
        tree.detach(c);
        tree.insert(root, c, Some(b));
        assert_eq!(tree.children(root).collect::<Vec<_>>(), vec![a, c, b]);
        assert_eq!(tree.children(root).rev().collect::<Vec<_>>(), vec![b, c, a]);
        assert_eq!(tree.parent(c), Some(root));
        assert_eq!(tree.children(a).len(), 0);
    }

    #[test]
    fn traversals_follow_document_order() {
        let (tree, root, a, b, c) = fixture();
        assert_eq!(tree.subtree(root).collect::<Vec<_>>(), vec![root, a, c, b]);
        assert_eq!(tree.ancestors(c).collect::<Vec<_>>(), vec![c, a, root]);
    }

    #[test]
    fn child_first_removal_invalidates_data_and_identity() {
        let (mut tree, root, a, _b, c) = fixture();
        assert_eq!(tree.remove(c), "c");
        assert_eq!(tree.remove(a), "a");
        assert!(!tree.contains_key(c));
        assert_eq!(tree.children(root).len(), 1);
    }
}
