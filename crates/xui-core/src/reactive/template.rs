//! The static half of a component, written by `#[component]`.
use std::rc::Rc;

use super::block::BlockCx;
use super::graph::{Notifier, SourceId};
use super::runtime::{Contexts, Cx, Runtime};
use super::signal::{Memo, ReadSignal, Signal};
use crate::node::{Draft, NodeDef};
use crate::widgets::ErasedDraft;

/// When an effect runs within a flush.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// Recomputes a memo, lowest level first.
    Memo,
    /// An `if`/`match`/`for` among children: picks the branch or reconciles
    /// the rows, outer blocks first, once every memo has settled.
    Block,
    /// Writes an attribute of a host, once the blocks have settled.
    Render,
    /// `effect!(..)`, after rendering; may write signals, which starts another round.
    Effect,
    /// `ticker!(..)`: run by the frame loop while its ticker runs, never by a flush.
    Tick,
}

/// What an effect calls. Effects take the scope by `Rc`, since a host
/// rebuilt whole binds its handlers to it again.
pub enum Run<I> {
    Cx(fn(&Rc<I>, &mut Cx<'_>)),
    /// A block, with the template node it stands at.
    Block(u16, fn(&Rc<I>, &mut BlockCx<'_>)),
    /// A ticker's callback.
    Tick(fn(&Rc<I>, &mut Cx<'_>, crate::clock::FrameTime) -> crate::ticker::Tick),
}

pub struct EffectSpec<I> {
    pub label: &'static str,
    pub lane: Lane,
    /// Indices into [`Template::sources`] / [`Scope::source`].
    pub deps: &'static [u16],
    /// The memo this effect computes, as a source index.
    pub output: Option<u16>,
    pub run: Run<I>,
}

/// Evaluates a child component's props against its scope (once, at mount)
/// and returns the deferred mount.
pub struct ChildSpec<I> {
    pub label: &'static str,
    pub prepare: fn(&Rc<I>) -> PendingMount,
}

/// Builds a host's draft as it is now: its constant attributes, its dynamic
/// ones at their current values, its handlers bound to the scope.
pub type BuildHost<I> = fn(&Rc<I>, &Notifier) -> HostDraft;

/// A host widget's draft, ready to mount.
pub struct HostDraft(pub(crate) Box<dyn ErasedDraft>);

impl HostDraft {
    #[allow(private_bounds)]
    pub fn new<N: NodeDef>(draft: Draft<N>) -> Self
    where
        Draft<N>: ErasedDraft,
    {
        Self(Box::new(draft))
    }
}

/// Asserts that `tag` constructs a host widget, as `#[component]` assumed
/// from its name, and hands it back.
#[doc(hidden)]
pub fn __host<T: super::host::HostTag>(tag: fn() -> T) -> fn() -> T {
    tag
}

// Function pointers are `Copy` whatever `I` is; a derive would ask `I: Copy`.
macro_rules! copy {
    ($($ty:ident),*) => {$(
        impl<I> Clone for $ty<I> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<I> Copy for $ty<I> {}
    )*};
}
copy!(Run, EffectSpec, ChildSpec, StaticNode, NodeKind);

pub type PendingMount =
    Box<dyn FnOnce(&mut Runtime, &mut crate::ui_runtime::UiRuntime, super::runtime::Place) -> super::graph::InstanceId>;

pub enum NodeKind<I> {
    /// A host widget.
    Host(BuildHost<I>),
    /// Index into [`Template::children`].
    Component(u16),
    /// Where a block's rows stand among their siblings. Not a host: the
    /// rows' hosts are placed directly under the enclosing host.
    Block,
    /// `slot(children)`: where the content a parent passed is mounted.
    Outlet(fn(&Rc<I>) -> super::Children),
    /// `portal { .. }`: content mounted under the overlayer.
    Portal(fn(&Rc<I>, &Notifier) -> super::PortalSpec),
}

/// Nodes are in preorder, so a parent always precedes its children.
/// Nodes without a parent are the scope's roots.
pub struct StaticNode<I> {
    pub kind: NodeKind<I>,
    /// Its parent's index; a parent is always a host.
    pub parent: Option<u16>,
}

pub struct Template<I: 'static> {
    /// Names of the instance's sources (reactive props, signals, memos) in
    /// declaration order; an index here is a dependency in [`EffectSpec::deps`].
    /// Rows of nested blocks number their data after these, one index per
    /// nesting level.
    pub sources: &'static [&'static str],
    pub nodes: &'static [StaticNode<I>],
    pub effects: &'static [EffectSpec<I>],
    pub children: &'static [ChildSpec<I>],
}

/// What bindings run against: a component instance or a block's row.
pub trait Scope: 'static {
    /// Resolves a source index of the template to this scope's source.
    fn source(&self, index: u16) -> SourceId;
}

/// Implemented by `#[component]` on the instance struct.
pub trait ComponentDef: Scope + Sized {
    type Props: 'static;
    const TEMPLATE: Template<Self>;

    /// Runs the component's setup: creates its signals, computes its memos.
    fn create(props: Self::Props, cx: &mut MountCx<'_>) -> Self;
}

pub struct MountCx<'a> {
    pub(crate) graph: &'a mut super::graph::Graph,
    /// Sources this instance creates and releases on unmount.
    pub(crate) owned: Vec<SourceId>,
    /// The contexts its ancestors provide, then the app's.
    pub(crate) contexts: Contexts<'a>,
    /// What it provides its descendants.
    pub(crate) provided: Vec<Context>,
    pub(crate) tickers: crate::ticker::TickerRegistry,
    /// One per `ticker!`, in order, until mount gives each its callback.
    pub(crate) ticks: Vec<crate::ticker::TickerHandle>,
}

/// A source provided as context: a `ReadSignal<T>`, under `T`'s type id.
pub(crate) type Context = (std::any::TypeId, Box<dyn std::any::Any>);

impl MountCx<'_> {
    /// `context!()`: the nearest `T` an ancestor provides, else the app's.
    pub fn context<T: 'static>(&mut self) -> ReadSignal<T> {
        self.try_context().unwrap_or_else(|| {
            panic!(
                "no `{}` context: provide one with `provide!(..)` in an ancestor, or with `App::provide_context`",
                std::any::type_name::<T>()
            )
        })
    }

    /// `ticker!(..)`'s handle: registered now, running once the instance is
    /// mounted and the ticker has its callback.
    #[doc(hidden)]
    pub fn __ticker(&mut self) -> crate::ticker::Ticker {
        let handle = self.tickers.install(Box::new(|_| crate::ticker::Tick::Continue));
        let ticker = handle.handle();
        self.ticks.push(handle);
        ticker
    }

    /// The nearest `T` an ancestor provides, else the app's, if there is one.
    pub fn try_context<T: 'static>(&self) -> Option<ReadSignal<T>> {
        let source = self.contexts.find(std::any::TypeId::of::<T>())?;
        Some(source.downcast_ref::<ReadSignal<T>>().expect("context under another type's id").clone())
    }

    /// `provide!(source)`: hands `source` to this component's descendants.
    pub fn provide<T: 'static>(&mut self, source: ReadSignal<T>) {
        self.provided.push((std::any::TypeId::of::<T>(), Box::new(source)));
    }

    /// `provide!(value)`: a value that never changes, as a source.
    pub fn provide_value<T: 'static>(&mut self, value: T) {
        let source = self.signal(value).read_only();
        self.provide(source);
    }

    pub fn signal<T>(&mut self, value: T) -> Signal<T> {
        let id = self.graph.insert();
        self.owned.push(id);
        Signal::new(id, value)
    }

    pub fn memo<T: PartialEq>(&mut self, value: T) -> Memo<T> {
        let id = self.graph.insert();
        self.owned.push(id);
        Memo::new(id, value)
    }
}

/// A child component's mount, its props evaluated: what a parent's
/// `ChildSpec::prepare` returns.
#[doc(hidden)]
pub fn __pending<C: ComponentDef>(props: C::Props) -> PendingMount {
    Box::new(move |runtime, ui, place| runtime.mount_into::<C>(ui, props, place))
}

/// A host's `draft: |d| ..` attribute, applied with the draft's type known,
/// so the closure infers its argument.
#[doc(hidden)]
pub fn __apply<D>(draft: D, f: impl FnOnce(D) -> D) -> D {
    f(draft)
}
