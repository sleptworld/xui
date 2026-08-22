use std::sync::atomic::{AtomicU32, Ordering};

use rustc_hash::FxHashMap;
use xui_interface::{
    Bounds, ComputedStyle, EdgeInsets, EventRef, EventResult, Key, NodeId, Size, Style,
    TextContent, TextProps, WidgetType, WidgetUpdateFlags,
};

use crate::{event_system::EventContext, render::RenderTreeWriter};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct OverlayScopeId(u32);

impl OverlayScopeId {
    fn next() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct OverlayEntryId(u32);

impl OverlayEntryId {
    fn next() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// One item in an overlay scope's paint order.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum OverlayChild {
    Scope(OverlayScopeId),
    Entry(OverlayEntryId),
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct OverlayEntryOptions {
    pub z_index: i32,
    pub hit_test: bool,
    pub modal: bool,
}

impl Default for OverlayEntryOptions {
    fn default() -> Self {
        Self {
            z_index: 0,
            hit_test: true,
            modal: false,
        }
    }
}

#[derive(Debug)]
pub struct OverlayScope {
    id: OverlayScopeId,
    parent: Option<OverlayScopeId>,
    children: Vec<OverlayChild>,
    z_index: i32,
    insertion_order: u64,
}

impl OverlayScope {
    pub fn id(&self) -> OverlayScopeId {
        self.id
    }

    pub fn parent(&self) -> Option<OverlayScopeId> {
        self.parent
    }

    /// Children in back-to-front paint order.
    pub fn children(&self) -> &[OverlayChild] {
        &self.children
    }

    pub fn z_index(&self) -> i32 {
        self.z_index
    }
}

#[derive(Debug)]
pub struct OverlayEntry {
    id: OverlayEntryId,
    scope: OverlayScopeId,
    visual_root: NodeId,
    z_index: i32,
    insertion_order: u64,
    hit_test: bool,
    modal: bool,
}

impl OverlayEntry {
    pub fn id(&self) -> OverlayEntryId {
        self.id
    }

    pub fn scope(&self) -> OverlayScopeId {
        self.scope
    }

    pub fn visual_root(&self) -> NodeId {
        self.visual_root
    }

    pub fn z_index(&self) -> i32 {
        self.z_index
    }

    pub fn hit_test(&self) -> bool {
        self.hit_test
    }

    pub fn modal(&self) -> bool {
        self.modal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayModelError {
    ScopeNotFound(OverlayScopeId),
    EntryNotFound(OverlayEntryId),
    VisualRootAlreadyMounted(NodeId),
    CannotRemoveRootScope,
}

/// The single transparent, viewport-sized overlay host owned by `UiRuntime`.
///
/// Scopes form nested stacking contexts. Entries are Portal visual roots. The
/// component tree remains their logical owner while this model records their
/// visual placement and ordering below the root overlayer.
#[derive(Debug)]
pub struct RootOverlayerWidget {
    key: Key,
    style: Style,
    root_scope: OverlayScopeId,
    scopes: FxHashMap<OverlayScopeId, OverlayScope>,
    entries: FxHashMap<OverlayEntryId, OverlayEntry>,
    entry_by_visual_root: FxHashMap<NodeId, OverlayEntryId>,
    next_insertion_order: u64,
}

impl RootOverlayerWidget {
    pub(crate) fn new() -> Self {
        let root_scope = OverlayScopeId::next();
        let mut scopes = FxHashMap::default();
        scopes.insert(
            root_scope,
            OverlayScope {
                id: root_scope,
                parent: None,
                children: Vec::new(),
                z_index: 0,
                insertion_order: 0,
            },
        );

        Self {
            key: "__xui_root_overlayer".into(),
            style: Style::new()
                .absolute()
                .inset(EdgeInsets::zero())
                .size(Size::fill()),
            root_scope,
            scopes,
            entries: FxHashMap::default(),
            entry_by_visual_root: FxHashMap::default(),
            next_insertion_order: 1,
        }
    }

    pub fn root_scope(&self) -> OverlayScopeId {
        self.root_scope
    }

    pub fn scope(&self, id: OverlayScopeId) -> Option<&OverlayScope> {
        self.scopes.get(&id)
    }

    pub fn entry(&self, id: OverlayEntryId) -> Option<&OverlayEntry> {
        self.entries.get(&id)
    }

    pub fn entry_for_visual_root(&self, visual_root: NodeId) -> Option<OverlayEntryId> {
        self.entry_by_visual_root.get(&visual_root).copied()
    }

    pub fn visual_roots_in_paint_order(&self) -> Vec<NodeId> {
        let mut roots = Vec::with_capacity(self.entries.len());
        self.collect_visual_roots(self.root_scope, &mut roots);
        roots
    }

    pub fn create_scope(
        &mut self,
        parent: OverlayScopeId,
        z_index: i32,
    ) -> Result<OverlayScopeId, OverlayModelError> {
        if !self.scopes.contains_key(&parent) {
            return Err(OverlayModelError::ScopeNotFound(parent));
        }

        let id = OverlayScopeId::next();
        let insertion_order = self.take_insertion_order();
        self.scopes.insert(
            id,
            OverlayScope {
                id,
                parent: Some(parent),
                children: Vec::new(),
                z_index,
                insertion_order,
            },
        );
        self.scopes
            .get_mut(&parent)
            .expect("parent scope was checked")
            .children
            .push(OverlayChild::Scope(id));
        self.sort_scope(parent);
        Ok(id)
    }

    pub fn insert_entry(
        &mut self,
        scope: OverlayScopeId,
        visual_root: NodeId,
        options: OverlayEntryOptions,
    ) -> Result<OverlayEntryId, OverlayModelError> {
        if !self.scopes.contains_key(&scope) {
            return Err(OverlayModelError::ScopeNotFound(scope));
        }
        if self.entry_by_visual_root.contains_key(&visual_root) {
            return Err(OverlayModelError::VisualRootAlreadyMounted(visual_root));
        }

        let id = OverlayEntryId::next();
        let insertion_order = self.take_insertion_order();
        self.entries.insert(
            id,
            OverlayEntry {
                id,
                scope,
                visual_root,
                z_index: options.z_index,
                insertion_order,
                hit_test: options.hit_test,
                modal: options.modal,
            },
        );
        self.entry_by_visual_root.insert(visual_root, id);
        self.scopes
            .get_mut(&scope)
            .expect("entry scope was checked")
            .children
            .push(OverlayChild::Entry(id));
        self.sort_scope(scope);
        Ok(id)
    }

    pub fn update_scope_z_index(
        &mut self,
        id: OverlayScopeId,
        z_index: i32,
    ) -> Result<(), OverlayModelError> {
        let scope = self
            .scopes
            .get_mut(&id)
            .ok_or(OverlayModelError::ScopeNotFound(id))?;
        scope.z_index = z_index;
        let parent = scope.parent;
        if let Some(parent) = parent {
            self.sort_scope(parent);
        }
        Ok(())
    }

    pub fn update_entry_options(
        &mut self,
        id: OverlayEntryId,
        options: OverlayEntryOptions,
    ) -> Result<(), OverlayModelError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(OverlayModelError::EntryNotFound(id))?;
        entry.z_index = options.z_index;
        entry.hit_test = options.hit_test;
        entry.modal = options.modal;
        let scope = entry.scope;
        self.sort_scope(scope);
        Ok(())
    }

    pub fn move_entry(
        &mut self,
        id: OverlayEntryId,
        next_scope: OverlayScopeId,
    ) -> Result<(), OverlayModelError> {
        if !self.scopes.contains_key(&next_scope) {
            return Err(OverlayModelError::ScopeNotFound(next_scope));
        }
        let old_scope = self
            .entries
            .get(&id)
            .ok_or(OverlayModelError::EntryNotFound(id))?
            .scope;
        if old_scope == next_scope {
            return Ok(());
        }

        self.scopes
            .get_mut(&old_scope)
            .expect("entry scope must exist")
            .children
            .retain(|child| *child != OverlayChild::Entry(id));
        let insertion_order = self.take_insertion_order();
        let entry = self.entries.get_mut(&id).expect("entry was checked");
        entry.scope = next_scope;
        entry.insertion_order = insertion_order;
        self.scopes
            .get_mut(&next_scope)
            .expect("destination scope was checked")
            .children
            .push(OverlayChild::Entry(id));
        self.sort_scope(next_scope);
        Ok(())
    }

    pub fn remove_entry(&mut self, id: OverlayEntryId) -> Result<NodeId, OverlayModelError> {
        let entry = self
            .entries
            .remove(&id)
            .ok_or(OverlayModelError::EntryNotFound(id))?;
        self.entry_by_visual_root.remove(&entry.visual_root);
        if let Some(scope) = self.scopes.get_mut(&entry.scope) {
            scope
                .children
                .retain(|child| *child != OverlayChild::Entry(id));
        }
        Ok(entry.visual_root)
    }

    /// Removes a complete stacking-context subtree and returns its Portal
    /// visual roots in back-to-front order.
    pub fn remove_scope(&mut self, id: OverlayScopeId) -> Result<Vec<NodeId>, OverlayModelError> {
        if id == self.root_scope {
            return Err(OverlayModelError::CannotRemoveRootScope);
        }
        let parent = self
            .scopes
            .get(&id)
            .ok_or(OverlayModelError::ScopeNotFound(id))?
            .parent;
        let mut visual_roots = Vec::new();
        self.remove_scope_descendants(id, &mut visual_roots);
        if let Some(parent) = parent.and_then(|parent| self.scopes.get_mut(&parent)) {
            parent
                .children
                .retain(|child| *child != OverlayChild::Scope(id));
        }
        Ok(visual_roots)
    }

    fn remove_scope_descendants(&mut self, id: OverlayScopeId, visual_roots: &mut Vec<NodeId>) {
        let children = self
            .scopes
            .get(&id)
            .map(|scope| scope.children.clone())
            .unwrap_or_default();
        for child in children {
            match child {
                OverlayChild::Scope(scope) => self.remove_scope_descendants(scope, visual_roots),
                OverlayChild::Entry(entry) => {
                    if let Ok(visual_root) = self.remove_entry(entry) {
                        visual_roots.push(visual_root);
                    }
                }
            }
        }
        self.scopes.remove(&id);
    }

    fn collect_visual_roots(&self, id: OverlayScopeId, roots: &mut Vec<NodeId>) {
        let Some(scope) = self.scopes.get(&id) else {
            return;
        };
        for child in &scope.children {
            match *child {
                OverlayChild::Scope(scope) => self.collect_visual_roots(scope, roots),
                OverlayChild::Entry(entry) => roots.push(self.entries[&entry].visual_root),
            }
        }
    }

    fn take_insertion_order(&mut self) -> u64 {
        let order = self.next_insertion_order;
        self.next_insertion_order = self.next_insertion_order.wrapping_add(1);
        order
    }

    fn sort_scope(&mut self, id: OverlayScopeId) {
        let Some(scope) = self.scopes.get(&id) else {
            return;
        };
        let mut children = scope.children.clone();
        children.sort_by_key(|child| match child {
            OverlayChild::Scope(id) => {
                let scope = &self.scopes[id];
                (scope.z_index, scope.insertion_order)
            }
            OverlayChild::Entry(id) => {
                let entry = &self.entries[id];
                (entry.z_index, entry.insertion_order)
            }
        });
        self.scopes
            .get_mut(&id)
            .expect("scope existed while sorting")
            .children = children;
    }

    no_event_handler_methods!();

    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::RootOverlayer
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        Some(&self.key)
    }

    pub(super) fn props_hash(&self) -> u64 {
        0
    }

    pub(super) fn update_from(&mut self, _next: &Self) -> WidgetUpdateFlags {
        WidgetUpdateFlags::empty()
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new()
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn render(
        &self,
        _node_id: NodeId,
        _rect: Bounds,
        _style: &ComputedStyle,
        _writer: &mut RenderTreeWriter<'_>,
    ) {
    }

    pub(super) fn handle_event(
        &mut self,
        _event: EventRef<'_>,
        _cx: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }

    pub(super) fn text_content(&self) -> Option<TextContent> {
        None
    }

    pub(super) fn text_layout_props(&self, _style: &ComputedStyle) -> Option<TextProps> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn scopes_and_entries_are_kept_in_stacking_order() {
        let mut overlay = RootOverlayerWidget::new();
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let root = overlay.root_scope();
        let front_scope = overlay.create_scope(root, 10).unwrap();
        let back = overlay
            .insert_entry(root, nodes.insert(()), OverlayEntryOptions::default())
            .unwrap();
        let front = overlay
            .insert_entry(
                root,
                nodes.insert(()),
                OverlayEntryOptions {
                    z_index: 10,
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(
            overlay.scope(root).unwrap().children(),
            &[
                OverlayChild::Entry(back),
                OverlayChild::Scope(front_scope),
                OverlayChild::Entry(front),
            ]
        );
    }

    #[test]
    fn removing_a_scope_cleans_up_all_nested_entries() {
        let mut overlay = RootOverlayerWidget::new();
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let root = overlay.root_scope();
        let scope = overlay.create_scope(root, 0).unwrap();
        let nested = overlay.create_scope(scope, 0).unwrap();
        let visual_root = nodes.insert(());
        let entry = overlay
            .insert_entry(nested, visual_root, OverlayEntryOptions::default())
            .unwrap();

        assert_eq!(overlay.remove_scope(scope).unwrap(), vec![visual_root]);
        assert!(overlay.scope(scope).is_none());
        assert!(overlay.scope(nested).is_none());
        assert!(overlay.entry(entry).is_none());
        assert!(overlay.scope(root).unwrap().children().is_empty());
    }
}
