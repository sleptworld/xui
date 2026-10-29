//! An ordered sequence of keys with a height each, for virtual documents.
//!
//! A document of 10^5 paragraphs is edited, scrolled and measured one
//! paragraph at a time, so every operation a frame needs is O(log n): insert
//! and remove at a position, a key's position and its top, the key at a
//! position or at a y, and a height change. It is an implicit treap: each
//! node keeps its subtree's count and summed height, and a parent link so a
//! key finds its own position without a search.

use std::ops::Range;

use slotmap::{Key, SecondaryMap};

#[derive(Debug, Clone)]
struct Node<K> {
    left: Option<K>,
    right: Option<K>,
    parent: Option<K>,
    priority: u32,
    count: usize,
    height: f32,
    /// The heights of the whole subtree, this node's included.
    sum: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct HeightSequence<K: Key> {
    nodes: SecondaryMap<K, Node<K>>,
    root: Option<K>,
    seed: u32,
}

impl<K: Key> Default for HeightSequence<K> {
    fn default() -> Self {
        Self {
            nodes: SecondaryMap::new(),
            root: None,
            seed: 0x9E37_79B9,
        }
    }
}

impl<K: Key> HeightSequence<K> {
    pub(crate) fn len(&self) -> usize {
        self.count(self.root)
    }

    pub(crate) fn total(&self) -> f32 {
        self.sum(self.root)
    }

    pub(crate) fn contains(&self, key: K) -> bool {
        self.nodes.contains_key(key)
    }

    /// Puts `key` at `index`, or last when `index` is past the end. A key
    /// already in the sequence is moved there.
    pub(crate) fn insert(&mut self, index: usize, key: K, height: f32) {
        if self.contains(key) {
            self.remove(key);
        }
        let priority = self.next_priority();
        self.nodes.insert(
            key,
            Node {
                left: None,
                right: None,
                parent: None,
                priority,
                count: 1,
                height,
                sum: height,
            },
        );
        let index = index.min(self.len());
        let (left, right) = self.split(self.root, index);
        let left = self.merge(left, Some(key));
        let root = self.merge(left, right);
        self.set_root(root);
    }

    /// Takes `key` out. Returns whether it was in the sequence.
    pub(crate) fn remove(&mut self, key: K) -> bool {
        let Some(index) = self.index_of(key) else {
            return false;
        };
        let (left, rest) = self.split(self.root, index);
        let (_, right) = self.split(rest, 1);
        self.nodes.remove(key);
        let root = self.merge(left, right);
        self.set_root(root);
        true
    }

    pub(crate) fn set_height(&mut self, key: K, height: f32) -> bool {
        let Some(node) = self.nodes.get_mut(key) else {
            return false;
        };
        node.height = height;
        let mut cursor = Some(key);
        while let Some(current) = cursor {
            self.update(current);
            cursor = self.nodes[current].parent;
        }
        true
    }

    pub(crate) fn index_of(&self, key: K) -> Option<usize> {
        let node = self.nodes.get(key)?;
        let mut index = self.count(node.left);
        let mut child = key;
        while let Some(parent) = self.nodes[child].parent {
            let parent_node = &self.nodes[parent];
            if parent_node.right == Some(child) {
                index += self.count(parent_node.left) + 1;
            }
            child = parent;
        }
        Some(index)
    }

    /// The summed height of every key before `key`.
    pub(crate) fn top_of(&self, key: K) -> Option<f32> {
        let node = self.nodes.get(key)?;
        let mut top = self.sum(node.left);
        let mut child = key;
        while let Some(parent) = self.nodes[child].parent {
            let parent_node = &self.nodes[parent];
            if parent_node.right == Some(child) {
                top += self.sum(parent_node.left) + parent_node.height;
            }
            child = parent;
        }
        Some(top)
    }

    pub(crate) fn at(&self, mut index: usize) -> Option<K> {
        let mut cursor = self.root;
        while let Some(current) = cursor {
            let node = &self.nodes[current];
            let left = self.count(node.left);
            if index < left {
                cursor = node.left;
            } else if index == left {
                return Some(current);
            } else {
                index -= left + 1;
                cursor = node.right;
            }
        }
        None
    }

    /// The key whose span of height holds `y`, measured from the top of the
    /// first. `None` above the first and at or below the end of the last.
    pub(crate) fn at_y(&self, mut y: f32) -> Option<K> {
        if !(0.0..self.total()).contains(&y) {
            return None;
        }
        let mut cursor = self.root;
        let mut last = None;
        while let Some(current) = cursor {
            let node = &self.nodes[current];
            let left = self.sum(node.left);
            if y < left {
                cursor = node.left;
                continue;
            }
            y -= left;
            last = Some(current);
            if y < node.height {
                return Some(current);
            }
            y -= node.height;
            cursor = node.right;
        }
        // Rounding in the sums can leave `y` just past the last key that
        // still holds it.
        last
    }

    /// The keys at `range`, in order.
    pub(crate) fn keys(&self, range: Range<usize>) -> Vec<K> {
        let end = range.end.min(self.len());
        let start = range.start.min(end);
        (start..end).filter_map(|index| self.at(index)).collect()
    }

    fn count(&self, key: Option<K>) -> usize {
        key.map_or(0, |key| self.nodes[key].count)
    }

    fn sum(&self, key: Option<K>) -> f32 {
        key.map_or(0.0, |key| self.nodes[key].sum)
    }

    fn next_priority(&mut self) -> u32 {
        // xorshift32: deterministic, and only the spread matters.
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        x
    }

    fn set_root(&mut self, root: Option<K>) {
        self.root = root;
        if let Some(root) = root {
            self.nodes[root].parent = None;
        }
    }

    /// Recomputes `key`'s count and sum from its children, and makes it their
    /// parent.
    fn update(&mut self, key: K) {
        let (left, right) = {
            let node = &self.nodes[key];
            (node.left, node.right)
        };
        let count = self.count(left) + self.count(right) + 1;
        let sum = self.sum(left) + self.sum(right) + self.nodes[key].height;
        for child in [left, right].into_iter().flatten() {
            self.nodes[child].parent = Some(key);
        }
        let node = &mut self.nodes[key];
        node.count = count;
        node.sum = sum;
    }

    /// The first `index` keys of `tree`, and the rest.
    fn split(&mut self, tree: Option<K>, index: usize) -> (Option<K>, Option<K>) {
        let Some(root) = tree else {
            return (None, None);
        };
        let left_count = self.count(self.nodes[root].left);
        if index <= left_count {
            let left = self.nodes[root].left;
            let (first, rest) = self.split(left, index);
            self.nodes[root].left = rest;
            if let Some(first) = first {
                self.nodes[first].parent = None;
            }
            self.update(root);
            (first, Some(root))
        } else {
            let right = self.nodes[root].right;
            let (first, rest) = self.split(right, index - left_count - 1);
            self.nodes[root].right = first;
            if let Some(rest) = rest {
                self.nodes[rest].parent = None;
            }
            self.update(root);
            (Some(root), rest)
        }
    }

    /// `left` followed by `right`.
    fn merge(&mut self, left: Option<K>, right: Option<K>) -> Option<K> {
        match (left, right) {
            (None, tree) | (tree, None) => tree,
            (Some(left), Some(right)) => {
                if self.nodes[left].priority > self.nodes[right].priority {
                    let merged = self.merge(self.nodes[left].right, Some(right));
                    self.nodes[left].right = merged;
                    self.update(left);
                    Some(left)
                } else {
                    let merged = self.merge(Some(left), self.nodes[right].left);
                    self.nodes[right].left = merged;
                    self.update(right);
                    Some(right)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::{DefaultKey, SlotMap};

    /// Checks every query against a plain `Vec` of (key, height).
    fn check(sequence: &HeightSequence<DefaultKey>, model: &[(DefaultKey, f32)]) {
        assert_eq!(sequence.len(), model.len());
        let mut top = 0.0;
        for (index, &(key, height)) in model.iter().enumerate() {
            assert_eq!(sequence.at(index), Some(key));
            assert_eq!(sequence.index_of(key), Some(index));
            let found = sequence.top_of(key).unwrap();
            assert!(
                (found - top).abs() < 1e-3,
                "top of {index}: {found} vs {top}"
            );
            if height > 0.0 {
                assert_eq!(sequence.at_y(top + height * 0.5), Some(key));
            }
            top += height;
        }
        assert!((sequence.total() - top).abs() < 1e-3);
        assert_eq!(sequence.at(model.len()), None);
        assert_eq!(sequence.at_y(top + 1.0), None);
        assert_eq!(sequence.at_y(-1.0), None);
    }

    #[test]
    fn matches_a_vector_through_random_edits() {
        let mut keys = SlotMap::new();
        let mut sequence = HeightSequence::default();
        let mut model: Vec<(DefaultKey, f32)> = Vec::new();
        let mut seed = 12345u32;
        let mut random = move |bound: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % bound.max(1)
        };
        for step in 0..2000 {
            match random(4) {
                0 | 1 => {
                    let key = keys.insert(());
                    let index = random(model.len() as u32 + 1) as usize;
                    let height = (random(5) * 10) as f32;
                    sequence.insert(index, key, height);
                    model.insert(index, (key, height));
                }
                2 if !model.is_empty() => {
                    let index = random(model.len() as u32) as usize;
                    let (key, _) = model.remove(index);
                    assert!(sequence.remove(key));
                }
                _ if !model.is_empty() => {
                    let index = random(model.len() as u32) as usize;
                    let height = (random(7) * 3) as f32;
                    model[index].1 = height;
                    sequence.set_height(model[index].0, height);
                }
                _ => {}
            }
            if step % 97 == 0 {
                check(&sequence, &model);
            }
        }
        check(&sequence, &model);
    }

    #[test]
    fn inserting_a_present_key_moves_it() {
        let mut keys = SlotMap::new();
        let (a, b, c) = (keys.insert(()), keys.insert(()), keys.insert(()));
        let mut sequence = HeightSequence::default();
        for key in [a, b, c] {
            sequence.insert(usize::MAX, key, 1.0);
        }
        sequence.insert(0, c, 2.0);
        check(&sequence, &[(c, 2.0), (a, 1.0), (b, 1.0)]);
        assert_eq!(sequence.keys(1..10), [a, b]);
    }
}
