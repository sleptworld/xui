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

    pub fn contains_key(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn get(&self, id: NodeId) -> Option<&D> {
        self.data.get(id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut D> {
        self.data.get_mut(id)
    }

    pub fn link(&self, id: NodeId) -> Option<&HostNode> {
        self.nodes.get(id)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(id).and_then(|node| node.parent)
    }

    pub fn position(&self, id: NodeId) -> Option<usize> {
        let mut position = 0;
        let mut cursor = self.nodes.get(id)?.prev_sibling;
        while let Some(sibling) = cursor {
            position += 1;
            cursor = self.nodes.get(sibling)?.prev_sibling;
        }
        Some(position)
    }

    pub fn children(&self, parent: NodeId) -> Children<'_> {
        let node = &self.nodes[parent];
        Children {
            tree: self,
            front: node.first_child,
            back: node.last_child,
            remaining: node.child_count,
        }
    }

    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            tree: self,
            next: self.nodes.contains_key(id).then_some(id),
        }
    }

    pub fn subtree(&self, root: NodeId) -> Dfs<'_> {
        Dfs {
            tree: self,
            stack: self
                .nodes
                .contains_key(root)
                .then_some(root)
                .into_iter()
                .collect(),
        }
    }

    pub fn walk<F>(&self, root: NodeId, control: F) -> Walker<'_, F>
    where
        F: FnMut(NodeId) -> WalkControl,
    {
        Walker {
            tree: self,
            stack: self
                .nodes
                .contains_key(root)
                .then_some(root)
                .into_iter()
                .collect(),
            control,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &D)> {
        self.nodes
            .keys()
            .filter_map(|id| self.data.get(id).map(|data| (id, data)))
    }

    pub fn values(&self) -> impl Iterator<Item = &D> {
        self.iter().map(|(_, data)| data)
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.assert_attachable(parent, child);
        self.detach(child);

        let previous = self.nodes[parent].last_child;
        {
            let child_node = &mut self.nodes[child];
            child_node.parent = Some(parent);
            child_node.prev_sibling = previous;
            child_node.next_sibling = None;
        }
        if let Some(previous) = previous {
            self.nodes[previous].next_sibling = Some(child);
        } else {
            self.nodes[parent].first_child = Some(child);
        }
        self.nodes[parent].last_child = Some(child);
        self.nodes[parent].child_count += 1;
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        self.assert_attachable(parent, child);
        assert_eq!(
            self.nodes[before].parent,
            Some(parent),
            "before node is not a child"
        );
        if child == before {
            return;
        }
        self.detach(child);

        let previous = self.nodes[before].prev_sibling;
        {
            let child_node = &mut self.nodes[child];
            child_node.parent = Some(parent);
            child_node.prev_sibling = previous;
            child_node.next_sibling = Some(before);
        }
        self.nodes[before].prev_sibling = Some(child);
        if let Some(previous) = previous {
            self.nodes[previous].next_sibling = Some(child);
        } else {
            self.nodes[parent].first_child = Some(child);
        }
        self.nodes[parent].child_count += 1;
    }

    pub fn detach(&mut self, child: NodeId) -> Option<NodeId> {
        let parent = self.nodes.get(child)?.parent?;
        let previous = self.nodes[child].prev_sibling;
        let next = self.nodes[child].next_sibling;

        if let Some(previous) = previous {
            self.nodes[previous].next_sibling = next;
        } else {
            self.nodes[parent].first_child = next;
        }
        if let Some(next) = next {
            self.nodes[next].prev_sibling = previous;
        } else {
            self.nodes[parent].last_child = previous;
        }
        self.nodes[parent].child_count -= 1;
        let child_node = &mut self.nodes[child];
        child_node.parent = None;
        child_node.prev_sibling = None;
        child_node.next_sibling = None;
        Some(parent)
    }

    pub fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        let old: Vec<_> = self.children(parent).collect();
        for child in old {
            self.detach(child);
        }
        for &child in children {
            self.append_child(parent, child);
        }
    }

    pub fn remove(&mut self, id: NodeId) -> Option<D> {
        if !self.nodes.contains_key(id) {
            return None;
        }
        assert_eq!(
            self.nodes[id].child_count, 0,
            "remove children before their parent"
        );
        self.detach(id);
        self.nodes.remove(id);
        self.data.remove(id)
    }

    fn assert_attachable(&self, parent: NodeId, child: NodeId) {
        assert!(self.nodes.contains_key(parent), "parent is missing");
        assert!(self.nodes.contains_key(child), "child is missing");
        assert_ne!(parent, child, "a node cannot parent itself");
        assert!(
            !self.ancestors(parent).any(|ancestor| ancestor == child),
            "attaching would create a cycle"
        );
    }
}

impl<D> Default for HostTree<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D> Index<NodeId> for HostTree<D> {
    type Output = D;

    fn index(&self, index: NodeId) -> &Self::Output {
        &self.data[index]
    }
}

impl<D> IndexMut<NodeId> for HostTree<D> {
    fn index_mut(&mut self, index: NodeId) -> &mut Self::Output {
        &mut self.data[index]
    }
}

pub struct Children<'a> {
    tree: &'a dyn HostLinks,
    front: Option<NodeId>,
    back: Option<NodeId>,
    remaining: usize,
}

trait HostLinks {
    fn host_link(&self, id: NodeId) -> &HostNode;
}

impl<D> HostLinks for HostTree<D> {
    fn host_link(&self, id: NodeId) -> &HostNode {
        &self.nodes[id]
    }
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.front?;
        self.front = self.tree.host_link(id).next_sibling;
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
        self.back = self.tree.host_link(id).prev_sibling;
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
    tree: &'a dyn HostLinks,
    next: Option<NodeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.next?;
        self.next = self.tree.host_link(id).parent;
        Some(id)
    }
}

pub struct Dfs<'a> {
    tree: &'a dyn HostLinks,
    stack: Vec<NodeId>,
}

impl Iterator for Dfs<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        let mut child = self.tree.host_link(id).last_child;
        while let Some(current) = child {
            self.stack.push(current);
            child = self.tree.host_link(current).prev_sibling;
        }
        Some(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalkControl {
    Continue,
    SkipChildren,
    Break,
}

pub struct Walker<'a, F>
where
    F: FnMut(NodeId) -> WalkControl,
{
    tree: &'a dyn HostLinks,
    stack: Vec<NodeId>,
    control: F,
}

impl<F> Iterator for Walker<'_, F>
where
    F: FnMut(NodeId) -> WalkControl,
{
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        match (self.control)(id) {
            WalkControl::Break => self.stack.clear(),
            WalkControl::SkipChildren => {}
            WalkControl::Continue => {
                let mut child = self.tree.host_link(id).last_child;
                while let Some(current) = child {
                    self.stack.push(current);
                    child = self.tree.host_link(current).prev_sibling;
                }
            }
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
        tree.append_child(root, a);
        tree.append_child(root, b);
        tree.append_child(a, c);
        (tree, root, a, b, c)
    }

    #[test]
    fn maintains_links_when_moving_and_inserting() {
        let (mut tree, root, a, b, c) = fixture();
        tree.insert_before(root, c, b);
        assert_eq!(tree.children(root).collect::<Vec<_>>(), vec![a, c, b]);
        assert_eq!(tree.children(root).rev().collect::<Vec<_>>(), vec![b, c, a]);
        assert_eq!(tree.parent(c), Some(root));
        assert_eq!(tree.position(c), Some(1));
        assert_eq!(tree.children(a).len(), 0);
    }

    #[test]
    fn traversals_honor_document_order_and_control() {
        let (tree, root, a, b, c) = fixture();
        assert_eq!(tree.subtree(root).collect::<Vec<_>>(), vec![root, a, c, b]);
        assert_eq!(tree.ancestors(c).collect::<Vec<_>>(), vec![c, a, root]);
        let visited = tree
            .walk(root, |id| {
                if id == a {
                    WalkControl::SkipChildren
                } else {
                    WalkControl::Continue
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(visited, vec![root, a, b]);

        let stopped = tree
            .walk(root, |id| {
                if id == a {
                    WalkControl::Break
                } else {
                    WalkControl::Continue
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(stopped, vec![root, a]);
    }

    #[test]
    fn child_first_removal_invalidates_data_and_identity() {
        let (mut tree, root, a, _b, c) = fixture();
        assert_eq!(tree.remove(c), Some("c"));
        assert_eq!(tree.remove(a), Some("a"));
        assert!(!tree.contains_key(c));
        assert_eq!(tree.children(root).len(), 1);
    }
}
