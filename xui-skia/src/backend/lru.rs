//! A bounded, insertion-ordered map used for the per-backend caches that are
//! too small to be worth a `moka` instance.

use std::{collections::HashMap, hash::Hash};

pub(super) struct LocalLru<K, V> {
    entries: HashMap<K, (V, u64)>,
    capacity: usize,
    clock: u64,
}

impl<K: Copy + Eq + Hash, V: Clone> LocalLru<K, V> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            clock: 0,
        }
    }

    pub(super) fn get(&mut self, key: &K) -> Option<V> {
        self.clock = self.clock.wrapping_add(1);
        let (value, used) = self.entries.get_mut(key)?;
        *used = self.clock;
        Some(value.clone())
    }

    pub(super) fn insert(&mut self, key: K, value: V) {
        self.clock = self.clock.wrapping_add(1);
        self.entries.insert(key, (value, self.clock));
        if self.entries.len() > self.capacity
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key)
        {
            self.entries.remove(&oldest);
        }
    }
}
