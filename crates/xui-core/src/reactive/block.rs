//! Dynamic blocks: `if`/`match` branches and `for` rows.
//!
//! A row is a scope of its own, mounted like a component instance: a
//! [`Row`] holds its parent scope and its data — the values its pattern
//! bound (`x` of `Some(x)`, the item of a `for`) — in a signal, so the
//! bindings inside it that read the data are bound to it like to any other
//! source. Re-running a block with the same branch, or an item with the same
//! key, only writes the new data; the row's bindings follow if it changed.
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use super::graph::{InstanceId, SourceId};
use super::runtime::{Owner, Runtime};
use super::signal::{Signal, Source};
use super::template::{ChildSpec, EffectSpec, Scope, StaticNode};
use crate::ui_runtime::UiRuntime;

pub struct Row<P, D> {
    pub parent: Rc<P>,
    pub data: Signal<D>,
    pub(crate) template: Rc<RowParts<Row<P, D>>>,
}

impl<P: Scope, D: 'static> Scope for Row<P, D> {
    fn source(&self, index: u16) -> SourceId {
        if index == self.template.level { self.data.id() } else { self.parent.source(index) }
    }
}

/// A row's template as the generated block hands it over. Its functions
/// are closures whose row type was inferred from the block's data, so it is
/// built where the block runs rather than in a `const`.
pub struct RowTemplate<'a, I> {
    /// The source index of the row's data.
    pub level: u16,
    pub nodes: &'a [StaticNode<I>],
    pub effects: &'a [EffectSpec<I>],
    pub children: &'a [ChildSpec<I>],
}

pub(crate) struct RowParts<I> {
    pub(crate) level: u16,
    pub(crate) nodes: Vec<StaticNode<I>>,
    pub(crate) effects: Vec<EffectSpec<I>>,
    pub(crate) children: Vec<ChildSpec<I>>,
}

impl<I> RowTemplate<'_, I> {
    pub(crate) fn to_parts(&self) -> RowParts<I> {
        RowParts {
            level: self.level,
            nodes: self.nodes.to_vec(),
            effects: self.effects.to_vec(),
            children: self.children.to_vec(),
        }
    }
}

/// What a block remembers between runs.
#[derive(Default)]
pub(crate) struct BlockState {
    /// Mounted rows, in order.
    pub(crate) rows: Vec<InstanceId>,
    /// The branch taken, for `if`/`match`.
    arm: Option<u16>,
    /// `Vec<K>`, the key of each row, for `for`.
    keys: Option<Box<dyn Any>>,
    /// `Rc<RowParts<..>>`, shared by a `for`'s rows.
    parts: Option<Rc<dyn Any>>,
}

/// Given to a running block.
pub struct BlockCx<'a> {
    pub(crate) runtime: &'a mut Runtime,
    pub(crate) ui: &'a mut UiRuntime,
    pub(crate) owner: InstanceId,
    /// The template node the block stands at.
    pub(crate) node: u16,
}

impl BlockCx<'_> {
    /// Shows branch `arm` with `data`, its pattern's bindings.
    /// `parent` is the scope the block runs in.
    pub fn arm<P: Scope, D: PartialEq + 'static>(
        &mut self,
        parent: &Rc<P>,
        arm: u16,
        data: D,
        template: RowTemplate<'_, Row<P, D>>,
    ) {
        let mut state = self.runtime.take_block(self.owner, self.node);
        if state.arm == Some(arm) {
            self.runtime.replace_row_data::<P, D>(state.rows[0], data);
        } else {
            for row in state.rows.drain(..) {
                self.runtime.unmount(self.ui, row);
            }
            // While its state is out, a block has no rows: the row goes
            // before whatever follows the block.
            let row = self.runtime.mount_row(self.ui, parent.clone(), data, Rc::new(template.to_parts()), self.row_owner());
            state.rows.push(row);
            state.arm = Some(arm);
        }
        self.runtime.put_block(self.owner, self.node, state);
    }

    /// Shows no branch: an `if` without `else` whose condition is false.
    pub fn none(&mut self) {
        let mut state = self.runtime.take_block(self.owner, self.node);
        for row in state.rows.drain(..) {
            self.runtime.unmount(self.ui, row);
        }
        state.arm = None;
        self.runtime.put_block(self.owner, self.node, state);
    }

    /// One row per item, matched to the previous rows by `key`.
    pub fn each<P: Scope, T: PartialEq + 'static, K: Hash + Eq + Clone + 'static>(
        &mut self,
        parent: &Rc<P>,
        items: impl IntoIterator<Item = T>,
        key: fn(&T) -> K,
        template: RowTemplate<'_, Row<P, T>>,
    ) {
        self.reconcile(parent, items, |_, item| key(item), template);
    }

    /// One row per item, matched to the previous rows by position.
    pub fn each_indexed<P: Scope, T: PartialEq + 'static>(
        &mut self,
        parent: &Rc<P>,
        items: impl IntoIterator<Item = T>,
        template: RowTemplate<'_, Row<P, T>>,
    ) {
        self.reconcile(parent, items, |index, _| index, template);
    }

    fn reconcile<P: Scope, T: PartialEq + 'static, K: Hash + Eq + Clone + 'static>(
        &mut self,
        parent: &Rc<P>,
        items: impl IntoIterator<Item = T>,
        mut key: impl FnMut(usize, &T) -> K,
        template: RowTemplate<'_, Row<P, T>>,
    ) {
        let mut state = self.runtime.take_block(self.owner, self.node);
        let parts = state
            .parts
            .take()
            .and_then(|p| p.downcast::<RowParts<Row<P, T>>>().ok())
            .unwrap_or_else(|| Rc::new(template.to_parts()));
        let keys = state.keys.take().and_then(|k| k.downcast::<Vec<K>>().ok()).map_or_else(Vec::new, |k| *k);
        let mut old: HashMap<K, InstanceId> = keys.into_iter().zip(state.rows.drain(..)).collect();

        let mut seen = HashSet::new();
        let mut keys = Vec::new();
        let mut rows = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            let k = key(index, &item);
            assert!(seen.insert(k.clone()), "duplicate key in a `for` block");
            let row = match old.remove(&k) {
                Some(row) => {
                    self.runtime.replace_row_data::<P, T>(row, item);
                    row
                }
                // Placed last among the block's hosts for now; ordered below.
                None => self.runtime.mount_row(self.ui, parent.clone(), item, parts.clone(), self.row_owner()),
            };
            rows.push(row);
            keys.push(k);
        }
        for row in old.into_values() {
            self.runtime.unmount(self.ui, row);
        }
        self.runtime.order_rows(self.ui, self.owner, self.node, &rows);
        state.rows = rows;
        state.keys = Some(Box::new(keys));
        state.parts = Some(parts);
        self.runtime.put_block(self.owner, self.node, state);
    }

    fn row_owner(&self) -> Owner {
        Owner::Row { instance: self.owner, node: self.node }
    }
}
