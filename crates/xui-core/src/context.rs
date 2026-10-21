//! Values a component hands down to its whole subtree, without threading them
//! through every component in between.
//!
//! ```
//! use xui_core::prelude::*;
//!
//! #[derive(Clone, Copy, PartialEq, Debug)]
//! enum Density { Comfortable, Compact }
//!
//! fn toolbar(cx: &mut HookContext<'_>) -> ElementDesc {
//!     cx.provide_context(Density::Compact);
//!     // ... every component rendered below sees `Density::Compact`
//!     container().into_element_desc(Vec::new())
//! }
//!
//! fn tool_button(cx: &mut HookContext<'_>) -> ElementDesc {
//!     let density = cx.use_context::<Density>().unwrap_or(Density::Comfortable);
//!     # let _ = density;
//!     container().into_element_desc(Vec::new())
//! }
//! ```
//!
//! A context is identified by its value type, so give each one its own type
//! (a newtype if need be) rather than providing a bare `bool` or `String`.
//!
//! # Resolution
//!
//! [`HookContext::use_context`] returns the value of the nearest *strict*
//! ancestor that provides that type, falling back to one set on the
//! application with [`App::provide_context`](crate::App::provide_context), and
//! `None` if neither exists. Ancestry is the component tree's, not the widget
//! tree's: content rendered through a [`portal`](crate::element::portal) still
//! sees the contexts around the place it was written. A component never sees
//! its own `provide_context`; that value is for what it renders.
//!
//! # Updates
//!
//! Providing a value that compares equal to the previous one is free. A
//! different value re-renders exactly the components that read it during their
//! last render, in the same pass, including those that would otherwise have
//! been skipped because their props did not change. Components in between that
//! do not read it are not re-rendered.
//!
//! `provide_context` is not a hook slot, so it may be called conditionally:
//! a component that stops providing a type sends its readers back to the next
//! provider out, and one that starts providing a type takes over the readers
//! below it. Each type may be provided at most once per render.

use std::any::{Any, TypeId, type_name};

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use crate::element::ElementDesc;
use crate::fiber::{FiberArena, FiberId};
use crate::lanes::{Lane, current_update_lane};
use crate::state::{HookContext, Scheduler};

/// Visits the strict ancestors of the component being rendered, nearest first,
/// until `visit` returns `true`.
pub(crate) trait Ancestors {
    fn find(&self, visit: &mut dyn FnMut(FiberId) -> bool);
}

/// The ancestors of the root component, which has none.
pub(crate) struct NoAncestors;

impl Ancestors for NoAncestors {
    fn find(&self, _visit: &mut dyn FnMut(FiberId) -> bool) {}
}

/// What a [`HookContext`] needs to provide and read contexts: the runtime's
/// registry, and the way up from the component it renders.
pub(crate) struct ContextScope<'a> {
    pub(crate) registry: &'a mut ContextRegistry,
    pub(crate) ancestors: &'a dyn Ancestors,
}

/// Where a consumer's value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    /// The nearest providing ancestor.
    Fiber(FiberId, TypeId),
    /// No ancestor provides the type: the application's value, or nothing.
    /// Readers that found nothing subscribe here too, so that a value set on
    /// the application later reaches them.
    App(TypeId),
}

struct Provided {
    ty: TypeId,
    value: Box<dyn Any>,
    subscribers: FxHashSet<FiberId>,
    /// Provided again by the render in progress. Whatever is still unset when
    /// the render finishes was dropped by it.
    renewed: bool,
}

#[derive(Default)]
struct AppEntry {
    value: Option<Box<dyn Any>>,
    subscribers: FxHashSet<FiberId>,
}

#[derive(Clone, Copy)]
enum Change {
    /// An existing entry's value changed.
    Value(TypeId),
    /// A type the component did not provide in its previous render.
    Added(TypeId),
}

/// Every provided value and who reads it. One per [`ComponentRuntime`](crate::ComponentRuntime),
/// which lends it to each render's [`HookContext`] as a field borrow beside
/// the hook storage; nothing outside the runtime holds on to it.
///
/// Values live here rather than in the provider's hook storage so that a
/// reader can find them by walking up, and so that `provide_context` needs no
/// hook slot. Like hook state, the registry is updated in place while a
/// component renders.
#[derive(Default)]
pub(crate) struct ContextRegistry {
    providers: FxHashMap<FiberId, SmallVec<[Provided; 1]>>,
    app: FxHashMap<TypeId, AppEntry>,
    /// What each consumer read during its last render, so the subscription
    /// can be dropped when it renders again or unmounts.
    subscriptions: FxHashMap<FiberId, SmallVec<[Source; 2]>>,
    /// Changes made by the render in progress, drained by `finish_render`.
    changes: SmallVec<[Change; 2]>,
}

impl ContextRegistry {
    /// Called before `fiber` renders: forgets what it read last time, since
    /// this render re-reads whatever it still needs.
    pub(crate) fn begin_render(&mut self, fiber: FiberId) {
        self.unsubscribe(fiber);
        self.changes.clear();
        if let Some(entries) = self.providers.get_mut(&fiber) {
            for entry in entries {
                entry.renewed = false;
            }
        }
    }

    /// Called after `fiber` rendered and before its children are reconciled,
    /// so that every reader its changes affect is marked in `lane` while the
    /// children's pending work is still being collected.
    ///
    /// `committed` is false for a fiber mounting in this render, which has no
    /// readers yet. `ancestors` are `fiber`'s own; `fibers` is the committed
    /// tree, the one existing readers are attached to.
    pub(crate) fn finish_render(
        &mut self,
        fiber: FiberId,
        committed: bool,
        ancestors: &dyn Ancestors,
        fibers: &FiberArena,
        scheduler: &Scheduler,
        lane: Lane,
    ) {
        let mut affected = Vec::new();
        if let Some(entries) = self.providers.get_mut(&fiber) {
            entries.retain(|entry| {
                if !entry.renewed {
                    affected.extend(entry.subscribers.iter().copied());
                }
                entry.renewed
            });
            if entries.is_empty() {
                self.providers.remove(&fiber);
            }
        }

        for change in std::mem::take(&mut self.changes) {
            match change {
                Change::Value(ty) => {
                    if let Some(entry) = self.entry(fiber, ty) {
                        affected.extend(entry.subscribers.iter().copied());
                    }
                }
                // Readers below `fiber` that resolved past it, to an outer
                // provider or the application, now resolve to `fiber`.
                Change::Added(ty) if committed => {
                    let outer = self.resolve(ty, ancestors);
                    affected.extend(
                        self.subscribers(outer)
                            .filter(|reader| is_descendant(fibers, *reader, fiber)),
                    );
                }
                Change::Added(_) => {}
            }
        }

        for reader in affected {
            scheduler.mark_component_dirty(reader, lane);
        }
    }

    /// Forgets everything `fiber` read and provided. Its readers are its
    /// descendants, which are going away with it.
    pub(crate) fn remove(&mut self, fiber: FiberId) {
        self.unsubscribe(fiber);
        self.providers.remove(&fiber);
    }

    fn provide<T: PartialEq + 'static>(&mut self, owner: FiberId, value: T) {
        let ty = TypeId::of::<T>();
        let entries = self.providers.entry(owner).or_default();
        let Some(entry) = entries.iter_mut().find(|entry| entry.ty == ty) else {
            entries.push(Provided {
                ty,
                value: Box::new(value),
                subscribers: FxHashSet::default(),
                renewed: true,
            });
            self.changes.push(Change::Added(ty));
            return;
        };
        assert!(
            !entry.renewed,
            "provide_context::<{}>() was called twice in one render; \
             a component provides each type at most once",
            type_name::<T>()
        );
        entry.renewed = true;
        if entry.value.downcast_ref::<T>() != Some(&value) {
            entry.value = Box::new(value);
            self.changes.push(Change::Value(ty));
        }
    }

    fn read<T: Clone + 'static>(
        &mut self,
        reader: FiberId,
        ancestors: &dyn Ancestors,
    ) -> Option<T> {
        let source = self.resolve(TypeId::of::<T>(), ancestors);
        self.subscribe(reader, source);
        let value = match source {
            Source::Fiber(fiber, ty) => self.entry(fiber, ty).map(|entry| &entry.value),
            Source::App(ty) => self.app.get(&ty).and_then(|entry| entry.value.as_ref()),
        }?;
        let value = value
            .downcast_ref::<T>()
            .expect("a context entry is stored under its own type");
        Some(value.clone())
    }

    /// Sets the application's value for `T`, under every provider in the tree.
    pub(crate) fn provide_app<T: PartialEq + 'static>(&mut self, value: T, scheduler: &Scheduler) {
        let entry = self.app.entry(TypeId::of::<T>()).or_default();
        let previous = entry
            .value
            .as_ref()
            .and_then(|value| value.downcast_ref::<T>());
        if previous == Some(&value) {
            return;
        }
        entry.value = Some(Box::new(value));
        notify(scheduler, &entry.subscribers);
    }

    /// Clears the application's value for `T`. Readers that found it now find
    /// nothing.
    pub(crate) fn remove_app<T: 'static>(&mut self, scheduler: &Scheduler) {
        if let Some(entry) = self.app.get_mut(&TypeId::of::<T>())
            && entry.value.take().is_some()
        {
            notify(scheduler, &entry.subscribers);
        }
    }

    pub(crate) fn app_value<T: Clone + 'static>(&self) -> Option<T> {
        self.app
            .get(&TypeId::of::<T>())?
            .value
            .as_ref()?
            .downcast_ref::<T>()
            .cloned()
    }

    #[cfg(test)]
    fn subscription_count(&self) -> usize {
        let providers: usize = self
            .providers
            .values()
            .flatten()
            .map(|entry| entry.subscribers.len())
            .sum();
        let app: usize = self.app.values().map(|entry| entry.subscribers.len()).sum();
        providers + app
    }

    fn entry(&self, fiber: FiberId, ty: TypeId) -> Option<&Provided> {
        self.providers
            .get(&fiber)?
            .iter()
            .find(|entry| entry.ty == ty)
    }

    fn entry_mut(&mut self, fiber: FiberId, ty: TypeId) -> Option<&mut Provided> {
        self.providers
            .get_mut(&fiber)?
            .iter_mut()
            .find(|entry| entry.ty == ty)
    }

    fn resolve(&self, ty: TypeId, ancestors: &dyn Ancestors) -> Source {
        let mut found = None;
        if !self.providers.is_empty() {
            ancestors.find(&mut |fiber| {
                let provides = self.entry(fiber, ty).is_some();
                if provides {
                    found = Some(fiber);
                }
                provides
            });
        }
        match found {
            Some(fiber) => Source::Fiber(fiber, ty),
            None => Source::App(ty),
        }
    }

    fn subscribers(&self, source: Source) -> impl Iterator<Item = FiberId> + '_ {
        let set = match source {
            Source::Fiber(fiber, ty) => self.entry(fiber, ty).map(|entry| &entry.subscribers),
            Source::App(ty) => self.app.get(&ty).map(|entry| &entry.subscribers),
        };
        set.into_iter().flatten().copied()
    }

    fn subscribe(&mut self, reader: FiberId, source: Source) {
        let inserted = match source {
            Source::Fiber(fiber, ty) => self
                .entry_mut(fiber, ty)
                .expect("a context resolves only to a provider that has the entry")
                .subscribers
                .insert(reader),
            Source::App(ty) => self.app.entry(ty).or_default().subscribers.insert(reader),
        };
        if inserted {
            self.subscriptions.entry(reader).or_default().push(source);
        }
    }

    fn unsubscribe(&mut self, reader: FiberId) {
        let Some(sources) = self.subscriptions.remove(&reader) else {
            return;
        };
        for source in sources {
            match source {
                // The provider may already have dropped the entry.
                Source::Fiber(fiber, ty) => {
                    if let Some(entry) = self.entry_mut(fiber, ty) {
                        entry.subscribers.remove(&reader);
                    }
                }
                Source::App(ty) => {
                    if let Some(entry) = self.app.get_mut(&ty) {
                        entry.subscribers.remove(&reader);
                        if entry.value.is_none() && entry.subscribers.is_empty() {
                            self.app.remove(&ty);
                        }
                    }
                }
            }
        }
    }
}

fn notify(scheduler: &Scheduler, readers: &FxHashSet<FiberId>) {
    let lane = current_update_lane();
    for reader in readers {
        scheduler.mark_component_dirty(*reader, lane);
    }
}

/// Whether `fiber` lies strictly below `ancestor` in the committed tree.
fn is_descendant(fibers: &FiberArena, fiber: FiberId, ancestor: FiberId) -> bool {
    let mut cursor = fibers.node(fiber).and_then(|node| node.parent);
    while let Some(id) = cursor {
        if id == ancestor {
            return true;
        }
        cursor = fibers.node(id).and_then(|node| node.parent);
    }
    false
}

impl HookContext<'_> {
    /// Makes `value` the context of type `T` for everything this component
    /// renders, down to the next component that provides a `T` itself.
    ///
    /// Readers re-render when `value` differs from the previous render's, by
    /// `PartialEq`; a value without a meaningful equality (a service handle,
    /// say) should compare by identity, as [`Callback`](crate::Callback) does.
    /// See the [module documentation](crate::context) for the rules.
    ///
    /// Does nothing on a context detached from an application
    /// ([`HookContext::new`]).
    ///
    /// # Panics
    ///
    /// If the same type is provided twice in one render.
    pub fn provide_context<T: PartialEq + 'static>(&mut self, value: T) {
        let owner = self.owner();
        if let Some(scope) = self.context_scope_mut() {
            scope.registry.provide(owner, value);
        }
    }

    /// The context of type `T` provided by the nearest ancestor, else the one
    /// set on the application, else `None`.
    ///
    /// This component re-renders whenever that value changes, or a different
    /// provider comes to supply it. Reading it only some of the time is fine:
    /// what is subscribed to is whatever the last render read.
    pub fn use_context<T: Clone + 'static>(&mut self) -> Option<T> {
        let owner = self.owner();
        let scope = self.context_scope_mut()?;
        scope.registry.read(owner, scope.ancestors)
    }

    /// [`Self::use_context`] for a context that the component cannot work
    /// without.
    ///
    /// # Panics
    ///
    /// If nothing provides a `T` above this component or on the application.
    #[track_caller]
    pub fn expect_context<T: Clone + 'static>(&mut self) -> T {
        self.use_context().unwrap_or_else(|| {
            panic!(
                "no context of type `{}` is provided above this component",
                type_name::<T>()
            )
        })
    }
}

// ---------------------------------------------------------------------------
// <context_provider>
// ---------------------------------------------------------------------------

/// `<context_provider value={v}>{child}</context_provider>` — provides `v` to
/// one child element, for when the subtree is not rendered by a component of
/// your own that could call [`HookContext::provide_context`].
///
/// # Panics
///
/// When given other than exactly one child: wrap several in a container.
#[xui_macros::component]
pub fn context_provider<T: Clone + PartialEq>(value: &T, children: &Vec<ElementDesc>) {
    cx.provide_context(value.clone());
    assert_eq!(
        children.len(),
        1,
        "<context_provider> takes exactly one child; wrap several in a container"
    );
    children[0].clone()
}

#[cfg(test)]
mod tests;
