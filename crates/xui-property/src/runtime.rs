//! Commit-only external resources. Render and Typed Slot apply never touch this.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegistrationId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutBinding {
    pub chord: String,
}
impl ShortcutBinding {
    pub fn new(chord: impl Into<String>) -> Self { Self { chord: chord.into() } }
}

#[derive(Debug, Default)]
pub struct ShortcutRegistry {
    next: u64,
    records: BTreeMap<RegistrationId, (NodeId, ShortcutBinding)>,
    pub registrations: usize,
    pub unregistrations: usize,
}
impl ShortcutRegistry {
    pub fn register(&mut self, owner: NodeId, binding: ShortcutBinding) -> RegistrationId {
        self.next += 1;
        let id = RegistrationId(self.next);
        self.records.insert(id, (owner, binding));
        self.registrations += 1;
        id
    }
    pub fn unregister(&mut self, id: RegistrationId) {
        if self.records.remove(&id).is_some() { self.unregistrations += 1; }
    }
    pub fn len(&self) -> usize { self.records.len() }
    pub fn is_empty(&self) -> bool { self.records.is_empty() }
    pub fn has(&self, owner: NodeId, chord: &str) -> bool {
        self.records.values().any(|(o, b)| *o == owner && b.chord == chord)
    }
}

#[derive(Debug, Default)]
pub struct AccessibilityRegistry {
    labels: BTreeMap<NodeId, String>,
    pub writes: usize,
    pub removals: usize,
}
impl AccessibilityRegistry {
    pub fn set_label(&mut self, owner: NodeId, label: String) {
        self.labels.insert(owner, label);
        self.writes += 1;
    }
    pub fn remove_label(&mut self, owner: NodeId) {
        if self.labels.remove(&owner).is_some() { self.removals += 1; }
    }
    pub fn label(&self, owner: NodeId) -> Option<&str> {
        self.labels.get(&owner).map(String::as_str)
    }
}

#[derive(Debug, Default)]
pub struct RuntimeServices {
    pub shortcuts: ShortcutRegistry,
    pub accessibility: AccessibilityRegistry,
}

pub struct CommitCtx<'a> {
    pub node: NodeId,
    pub services: &'a mut RuntimeServices,
}
