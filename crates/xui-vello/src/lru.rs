//! A byte-budgeted least-recently-used map for the few caches that are small
//! enough not to need `moka`: fonts, whose key type is the text backend's own
//! and so cannot be asked to be `Send + Sync`.

use std::hash::Hash;

use rustc_hash::FxHashMap;

struct Slot<V> {
    value: V,
    bytes: u64,
    last_used: u64,
}

pub(crate) struct ByteLru<K, V> {
    entries: FxHashMap<K, Slot<V>>,
    bytes: u64,
    budget: u64,
    tick: u64,
}

impl<K: Copy + Eq + Hash, V: Clone> ByteLru<K, V> {
    pub(crate) fn new(budget: u64) -> Self {
        Self {
            entries: FxHashMap::default(),
            bytes: 0,
            budget,
            tick: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// The value for `key`, built by `make` (returning it and its size in
    /// bytes) when it is absent. The least recently used entries go first when
    /// the budget is exceeded; the entry just asked for never does.
    pub(crate) fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> (V, u64)) -> V {
        self.tick += 1;
        let tick = self.tick;
        if let Some(slot) = self.entries.get_mut(&key) {
            slot.last_used = tick;
            return slot.value.clone();
        }
        let (value, bytes) = make();
        self.entries.insert(
            key,
            Slot {
                value: value.clone(),
                bytes,
                last_used: tick,
            },
        );
        self.bytes += bytes;
        while self.bytes > self.budget && self.entries.len() > 1 {
            // Few entries, rarely evicted: a scan is simpler than a list.
            let oldest = self
                .entries
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, slot)| slot.last_used)
                .map(|(k, _)| *k)
                .expect("more than one entry");
            let removed = self.entries.remove(&oldest).expect("present");
            self.bytes -= removed.bytes;
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_used_entry_goes_first() {
        let mut lru = ByteLru::new(100);
        lru.get_or_insert_with(1, || ("a", 40));
        lru.get_or_insert_with(2, || ("b", 40));
        // Touch 1, so 2 is now the oldest.
        lru.get_or_insert_with(1, || unreachable!());
        lru.get_or_insert_with(3, || ("c", 40));
        assert_eq!(lru.len(), 2);
        assert_eq!(lru.get_or_insert_with(1, || unreachable!()), "a");
        assert_eq!(lru.get_or_insert_with(3, || unreachable!()), "c");
        // 2 was evicted, so it is built again.
        assert_eq!(lru.get_or_insert_with(2, || ("b2", 40)), "b2");
    }

    #[test]
    fn an_entry_larger_than_the_budget_is_kept_alone() {
        let mut lru = ByteLru::new(10);
        lru.get_or_insert_with(1, || ("big", 500));
        assert_eq!(lru.len(), 1);
        lru.get_or_insert_with(2, || ("also big", 500));
        assert_eq!(lru.len(), 1);
        assert_eq!(lru.bytes(), 500);
    }
}
