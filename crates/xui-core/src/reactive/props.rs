//! What a parent hands a child component: how each prop is delivered is the
//! child's declaration, and the property's [`Accepts`](crate::property::Accepts)
//! impls are what a parent can write.
//!
//! The parent's code builds the child's [`Draft`](crate::node::Draft)
//! through its property table, so a misspelt prop is a missing method and a
//! missing required prop fails the draft's requirement state. Per attribute
//! it passes either
//!
//! - [`SourceArg`], when the attribute is one of its sources by name, or
//! - [`Expr`], anything else: the expression as a function of the parent's
//!   scope, and the sources it reads.
//!
//! and the child's prop takes it by its mode:
//!
//! | declaration         | stores                | `SourceArg`     | `Expr`                 |
//! |---------------------|-----------------------|-----------------|------------------------|
//! | `x: &T`             | `T`                   | its value, now  | evaluated, now         |
//! | `#[source] x: &T`   | [`Tracked<T>`]        | the handle      | a derived source       |
//! | `#[state] x: &T`    | [`Signal<T>`]         | the signal only | not accepted           |
use super::graph::{Graph, SourceId};
use super::runtime::{Cx, Runtime};
use super::signal::{Memo, ReadSignal, Signal};
use super::template::{ComponentDef, PendingMount};
use crate::node::NodeDef;
use crate::property::{Property, PropertyMarker};

/// One of the parent's sources, passed by name.
pub struct SourceArg<H>(pub H);

/// An expression of the parent's scope: what it reads, and how to compute it.
pub struct Expr<T> {
    deps: Vec<SourceId>,
    compute: Box<dyn Fn() -> T>,
}

impl<T> Expr<T> {
    /// Its value now.
    pub fn once(self) -> T {
        (self.compute)()
    }
}

/// A prop of the component that names property `P`: the `T` it reads.
pub trait PropKind: Property {
    type Item: 'static;
}

/// An [`Expr`] for property `P`: the closure yields the prop's own type,
/// a numeric literal typed by the prop or a value [`__convert`]ed by it.
#[doc(hidden)]
pub fn __expr<P: PropKind>(
    _: PropertyMarker<P>,
    deps: Vec<SourceId>,
    compute: impl Fn() -> P::Item + 'static,
) -> Expr<P::Item> {
    Expr { deps, compute: Box::new(compute) }
}

/// How a parent's value of type `V` becomes prop `Self`: converted with
/// `Into` for most props, taken as is by a prop whose type names one of the
/// component's parameters, so that the parameter is inferred from it.
#[diagnostic::on_unimplemented(
    message = "prop `{Self}` does not take `{V}`",
    label = "not a value of this prop",
    note = "a `#[state]` prop takes one of the parent's sources by name"
)]
pub trait FromValue<V>: PropKind {
    fn from_value(value: V) -> Self::Item;
}

/// The parent's value for property `P`, converted by `P`. Called inside the
/// expression's closure, so the value may borrow what the closure captured.
#[doc(hidden)]
pub fn __convert<P: FromValue<V>, V>(_: PropertyMarker<P>, value: V) -> P::Item {
    P::from_value(value)
}

/// A source's current value, for a prop that takes values.
pub trait Current<T> {
    fn current(&self) -> T;
}

macro_rules! current {
    ($($handle:ident),*) => {$(
        impl<T: Clone> Current<T> for $handle<T> {
            fn current(&self) -> T {
                self.get()
            }
        }
    )*};
}
current!(Signal, Memo, ReadSignal);

/// A `#[source]` prop as the parent delivered it, until the child mounts.
pub enum Tracked<T> {
    Handle(ReadSignal<T>),
    Derived(Expr<T>),
    /// A default: a source that never changes.
    Constant(T),
}

/// A root's `#[source]` prop, handed state the application owns
/// (see [`App::signal`](crate::app::App::signal)).
impl<T> From<ReadSignal<T>> for Tracked<T> {
    fn from(handle: ReadSignal<T>) -> Self {
        Self::Handle(handle)
    }
}

impl<T> From<&Signal<T>> for Tracked<T> {
    fn from(signal: &Signal<T>) -> Self {
        Self::Handle(signal.read_only())
    }
}

impl<T: PartialEq + 'static> Tracked<T> {
    /// The source the child reads.
    pub fn resolve(self, cx: &mut PropCx<'_>) -> ReadSignal<T> {
        match self {
            Self::Handle(handle) => handle,
            Self::Constant(value) => {
                let id = cx.graph.insert();
                cx.owned.push(id);
                Signal::new(id, value).read_only()
            }
            // Reads nothing of the parent's: a constant, with no effect to bind.
            Self::Derived(Expr { deps, compute }) if deps.is_empty() => Self::Constant(compute()).resolve(cx),
            Self::Derived(Expr { deps, compute }) => {
                let id = cx.graph.insert();
                cx.owned.push(id);
                let memo = Memo::new(id, compute());
                let output = memo.clone();
                cx.derived.push(Derived {
                    deps,
                    output: id,
                    run: Box::new(move |cx| output.__replace(cx, compute())),
                });
                ReadSignal::from(memo)
            }
        }
    }
}

/// A source a child computes from its parent's: the child's effect, bound
/// to the parent's sources.
pub(crate) struct Derived {
    pub(crate) deps: Vec<SourceId>,
    pub(crate) output: SourceId,
    pub(crate) run: Box<dyn Fn(&mut Cx<'_>)>,
}

/// Given to a child's props as they are resolved, at mount.
pub struct PropCx<'a> {
    pub(crate) graph: &'a mut Graph,
    pub(crate) owned: Vec<SourceId>,
    pub(crate) derived: Vec<Derived>,
}

/// Implemented by `#[component]` on the component's marker, the type that
/// names it: what instance it mounts.
pub trait Component: NodeDef {
    type Instance: ComponentDef;
}

/// A completed draft of component `C`, ready to mount: implemented by
/// `#[component]` for the draft whose required props are all set, so a
/// parent mounts a child by its draft alone, without naming its marker.
pub trait MountDraft<C> {
    #[doc(hidden)]
    fn __mount(self) -> PendingMount;
}

/// A child's mount, its props resolved by `props` at mount.
#[doc(hidden)]
pub fn __pending_with<C: ComponentDef>(props: impl FnOnce(&mut PropCx<'_>) -> C::Props + 'static) -> PendingMount {
    Box::new(move |runtime: &mut Runtime, ui, place| runtime.mount_with::<C>(ui, props, place))
}
