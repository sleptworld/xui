use std::ops::{Index, IndexMut};

use slotmap::SecondaryMap;
use xui_interface::{NodeId, WidgetState};

use crate::caps::CapMask;
use crate::ui_runtime::state::HostWorkFlags;
use crate::widgets::{WidgetI, WidgetType};
use xui_interface::Key;

/// Dense, high-frequency host data keyed by the topology's generational id.
pub(crate) struct HostData {
    pub node_type: WidgetType,
    /// Cached at creation: raw dispatch consults it to skip nodes whose widget
    /// never looks at `EventRef::Raw`.
    pub reads_raw_events: bool,
    pub key: Option<Key>,
    /// The capabilities the widget selects: which columns hold its rows.
    pub caps: CapMask,
    pub work: HostWorkFlags,
    pub subtree_work: HostWorkFlags,
    pub state: WidgetState,
    pub state_before_change: Option<WidgetState>,
    pub widget: WidgetI,
}

impl HostData {
    pub(crate) fn new(key: Option<Key>, widget: WidgetI) -> Self {
        let node_type = widget.node_type();
        let reads_raw_events = widget.reads_raw_events();
        Self {
            node_type,
            reads_raw_events,
            key,
            caps: widget.caps(),
            work: HostWorkFlags::empty(),
            subtree_work: HostWorkFlags::empty(),
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
    fn new() -> Self {
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

/// Bits of a [`NodeId`]'s version that name the window the node is in.
pub(crate) const TAG_BITS: u32 = 10;
/// How many windows can be open at once: one tag each.
pub(crate) const TAGS: usize = 1 << TAG_BITS;
/// Bits of a version that count a slot's reuses within its window.
const GENERATION_BITS: u32 = 21;
/// The last generation a slot is given. A slot freed at it is not reused.
pub(crate) const MAX_GENERATION: u32 = (1 << GENERATION_BITS) - 1;

/// Where one window's [`NodeId`]s come from.
///
/// The index of an id is the window's own, dense from 0, so every table a
/// window keys by node id is as long as the window has nodes, not as the
/// whole app has. The window is in the id's version instead -- its tag, in
/// the top [`TAG_BITS`] -- so an id still names one node app-wide, and what
/// is keyed by `NodeId` outside a window (the text host, a platform reply)
/// needs no window beside it. Below the tag is the slot's generation, bumped
/// each time the slot is freed; a slot whose generation runs out is retired
/// rather than reused, so an id is never given out twice.
///
/// A table keyed by index (`SecondaryMap`, `SparseSecondaryMap`) must hold
/// one window's ids only: two windows' nodes with the same index would share
/// its slot. Across windows, key a hash map.
///
/// See `docs/multi-window.md` (D3).
pub(crate) struct NodeAlloc {
    tag: u32,
    /// Each slot's generation: the one its live node has, or the one the
    /// next node in it will.
    generations: Vec<u32>,
    free: Vec<u32>,
    /// The generation slots start at. A window given a tag another had starts
    /// above every generation that one gave out, so the other's stale ids
    /// never name one of its nodes.
    floor: u32,
    /// One past the highest generation given out or set for reuse.
    next_floor: u32,
}

impl NodeAlloc {
    /// Ids for the window tagged `tag`, from generation `floor` on.
    pub(crate) fn new(tag: u32, floor: u32) -> Self {
        debug_assert!((tag as usize) < TAGS);
        debug_assert!(floor <= MAX_GENERATION);
        Self {
            tag,
            generations: Vec::new(),
            free: Vec::new(),
            floor,
            next_floor: floor,
        }
    }

    fn insert(&mut self) -> NodeId {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.generations.push(self.floor);
                (self.generations.len() - 1) as u32
            }
        };
        let generation = self.generations[index as usize];
        self.next_floor = self.next_floor.max(generation + 1);
        encode(self.tag, generation, index)
    }

    fn remove(&mut self, id: NodeId) {
        let (tag, generation, index) = decode(id);
        debug_assert_eq!(tag, self.tag, "an id of another window");
        let next = generation + 1;
        self.next_floor = self.next_floor.max(next + 1);
        self.generations[index as usize] = next;
        if next <= MAX_GENERATION {
            self.free.push(index);
        }
    }

    /// The window's tag.
    pub(crate) fn tag(&self) -> u32 {
        self.tag
    }

    /// Where a window given this tag next should start: above every
    /// generation this one gave out.
    pub(crate) fn next_floor(&self) -> u32 {
        self.next_floor.min(MAX_GENERATION + 1)
    }
}

fn encode(tag: u32, generation: u32, index: u32) -> NodeId {
    let version = (tag << (GENERATION_BITS + 1)) | (generation << 1) | 1;
    NodeId::from(slotmap::KeyData::from_ffi(
        (u64::from(version) << 32) | u64::from(index),
    ))
}

/// `(tag, generation, index)` of `id`.
fn decode(id: NodeId) -> (u32, u32, u32) {
    let ffi = slotmap::Key::data(&id).as_ffi();
    let (version, index) = ((ffi >> 32) as u32, ffi as u32);
    (
        version >> (GENERATION_BITS + 1),
        (version >> 1) & MAX_GENERATION,
        index,
    )
}

/// The tag of the window `id` was made in.
pub(crate) fn tag_of(id: NodeId) -> u32 {
    decode(id).0
}

/// Generational host identity/topology plus a secondary core-data cache.
///
/// The ids come from the tree's own [`NodeAlloc`].
///
/// # Preconditions
///
/// Every method taking a `NodeId` requires it to be live -- present in the
/// tree -- and panics otherwise, except the one that exists to ask:
/// [`Self::contains_key`]. An id of another window's tree is never live in
/// this one.
pub(crate) struct HostTree<D> {
    ids: NodeAlloc,
    nodes: Links,
    data: SecondaryMap<NodeId, D>,
}

impl<D> HostTree<D> {
    /// A tree whose ids are tagged 0.
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_ids(NodeAlloc::new(0, 0))
    }

    /// A tree taking its ids from `ids`.
    pub fn with_ids(ids: NodeAlloc) -> Self {
        Self {
            ids,
            nodes: SecondaryMap::new(),
            data: SecondaryMap::new(),
        }
    }

    /// Where the tree's ids come from.
    pub(crate) fn ids(&self) -> &NodeAlloc {
        &self.ids
    }

    pub fn insert_with_key(&mut self, make_data: impl FnOnce(NodeId) -> D) -> NodeId {
        let id = self.ids.insert();
        self.nodes.insert(id, HostNode::new());
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
        self.ids.remove(id);
        self.data.remove(id).expect("host data is dense")
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

type Links = SecondaryMap<NodeId, HostNode>;

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
    fn two_windows_ids_never_collide_though_their_indices_do() {
        let mut a = HostTree::with_ids(NodeAlloc::new(0, 0));
        let mut b = HostTree::with_ids(NodeAlloc::new(1, 0));
        let in_a: Vec<_> = (0..8).map(|_| a.insert_with_key(|_| ())).collect();
        let in_b: Vec<_> = (0..8).map(|_| b.insert_with_key(|_| ())).collect();

        // Dense within each window, from 0: the same indices in both.
        let index = |id: NodeId| decode(id).2;
        assert_eq!(
            in_a.iter().map(|&id| index(id)).collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        assert_eq!(
            in_b.iter().map(|&id| index(id)).collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        // Yet every id is its window's alone.
        assert!(in_a.iter().all(|id| !in_b.contains(id)));
        assert!(
            in_a.iter()
                .all(|id| a.contains_key(*id) && !b.contains_key(*id))
        );
        assert_eq!(tag_of(in_a[0]), 0);
        assert_eq!(tag_of(in_b[0]), 1);
    }

    #[test]
    fn a_reused_slot_gives_a_new_id_and_a_spent_one_is_retired() {
        let mut ids = NodeAlloc::new(3, MAX_GENERATION - 1);
        let first = ids.insert();
        ids.remove(first);
        let second = ids.insert();
        assert_eq!(decode(second).2, decode(first).2, "the slot is reused");
        assert_ne!(second, first);
        assert_eq!(decode(second).1, MAX_GENERATION);
        // Freed at the last generation: retired, so the next id is a new slot.
        ids.remove(second);
        let third = ids.insert();
        assert_ne!(decode(third).2, decode(second).2);
        assert_eq!(tag_of(third), 3);
    }

    #[test]
    fn a_tag_reused_from_its_floor_never_repeats_an_old_id() {
        let mut old = NodeAlloc::new(5, 0);
        let old_ids: Vec<_> = (0..4).map(|_| old.insert()).collect();
        old.remove(old_ids[1]);
        let mut new = NodeAlloc::new(5, old.next_floor());
        let new_ids: Vec<_> = (0..4).map(|_| new.insert()).collect();
        assert!(new_ids.iter().all(|id| !old_ids.contains(id)));
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
