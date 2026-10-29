//! The dependency graph: sources and the effects bound to them.
//! Values live in the source handles ([`Signal`](super::Signal),
//! [`Memo`](super::Memo)); the graph only knows who to notify.
use std::cell::RefCell;
use std::rc::Rc;

use slotmap::{SlotMap, new_key_type};

use super::tasks::Spawn;

new_key_type! {
    pub struct SourceId;
    pub struct InstanceId;
}

/// One dynamic part of one mounted instance: `slot` indexes its
/// component's `TEMPLATE.effects`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectId {
    pub instance: InstanceId,
    pub slot: u16,
}

struct SourceRecord {
    subscribers: Vec<EffectId>,
    version: u64,
    /// 0 for signals; a memo sits one above the highest source it reads.
    /// Memos are recomputed lowest level first, so none runs before its inputs settle.
    level: u32,
}

#[derive(Default)]
pub struct Graph {
    sources: SlotMap<SourceId, SourceRecord>,
    pub(crate) pending: Vec<EffectId>,
}

impl Graph {
    pub(crate) fn insert(&mut self) -> SourceId {
        self.sources.insert(SourceRecord { subscribers: Vec::new(), version: 0, level: 0 })
    }

    pub(crate) fn remove(&mut self, id: SourceId) {
        self.sources.remove(id);
    }

    /// Returns the source's level. A source the graph does not hold — a
    /// constant — never notifies, so there is nothing to record.
    pub(crate) fn subscribe(&mut self, id: SourceId, effect: EffectId) -> u32 {
        let Some(record) = self.sources.get_mut(id) else { return 0 };
        record.subscribers.push(effect);
        record.level
    }

    pub(crate) fn unsubscribe(&mut self, id: SourceId, effect: EffectId) {
        if let Some(record) = self.sources.get_mut(id)
            && let Some(at) = record.subscribers.iter().position(|e| *e == effect)
        {
            record.subscribers.swap_remove(at);
        }
    }

    pub(crate) fn set_level(&mut self, id: SourceId, level: u32) {
        self.sources[id].level = level;
    }

    pub(crate) fn level(&self, id: SourceId) -> u32 {
        self.sources[id].level
    }

    /// Takes the writes `notifier` collected since the last call.
    pub(crate) fn drain(&mut self, notifier: &Notifier) {
        let written = std::mem::take(&mut notifier.0.borrow_mut().written);
        for id in written {
            self.notify(id);
        }
    }

    /// A handle can outlive its record (an unmounted instance's signal kept
    /// elsewhere); writing to it is then a no-op for the graph.
    pub(crate) fn notify(&mut self, id: SourceId) {
        if let Some(record) = self.sources.get_mut(id) {
            record.version += 1;
            self.pending.extend_from_slice(&record.subscribers);
        }
    }

    pub fn version(&self, id: SourceId) -> Option<u64> {
        self.sources.get(id).map(|r| r.version)
    }

    pub fn subscribers(&self, id: SourceId) -> &[EffectId] {
        self.sources.get(id).map_or(&[], |r| &r.subscribers)
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// Where writes go until the runtime flushes: handlers run inside the host's
/// event dispatch, out of reach of the graph, so a write only records which
/// source changed. Shared by the runtime and every handler it installs.
#[derive(Clone, Default)]
pub struct Notifier(Rc<RefCell<Queued>>);

#[derive(Default)]
struct Queued {
    written: Vec<SourceId>,
    /// `resource!`'s futures, which every `Cx` can start.
    spawns: Vec<Spawn>,
}

impl Notifier {
    pub(crate) fn push(&self, id: SourceId) {
        self.0.borrow_mut().written.push(id);
    }

    pub(crate) fn spawn(&self, spawn: Spawn) {
        self.0.borrow_mut().spawns.push(spawn);
    }

    /// Takes the futures queued since the last call.
    pub(crate) fn take_spawns(&self) -> Vec<Spawn> {
        std::mem::take(&mut self.0.borrow_mut().spawns)
    }

    pub(crate) fn is_empty(&self) -> bool {
        let queued = self.0.borrow();
        queued.written.is_empty() && queued.spawns.is_empty()
    }
}
