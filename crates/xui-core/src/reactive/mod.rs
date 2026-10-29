//! Fine-grained reactive components, with dependencies resolved at compile
//! time instead of tracked while code runs. See `docs/reactive.md`.
//!
//! Three kinds of reactive nodes:
//!
//! - [`Signal`]: writable state. Setting one notifies its subscribers.
//! - [`Memo`]: a derived value, recomputed when its sources change; it only
//!   notifies its own subscribers when the new value differs (`PartialEq`).
//! - effects: everything that reads and is never read — a view's attribute
//!   bindings ([`Lane::Render`]), its `if`/`match`/`for` blocks
//!   ([`Lane::Block`]) and `effect!` closures ([`Lane::Effect`]).
//!
//! `#[component]` reads every dynamic part of a component and decides at
//! expansion time which sources it depends on. The result is a static
//! [`Template`]: one [`EffectSpec`] per dynamic part, carrying its
//! dependency list as indices into the component's sources. Reads at run
//! time are plain borrows; nothing is recorded while code runs. When an
//! instance is mounted the runtime resolves those indices to the instance's
//! [`SourceId`]s and records the subscriptions once ("binding").
//!
//! The analysis is sound by construction: each dynamic part compiles to a
//! function whose parameters are exactly the names the macro found, so code
//! cannot reach a source it was not seen to read — a miss is a compile error,
//! never a stale UI. Over-approximation (a name the code mentions but does
//! not really read) only costs a spurious re-run.
//!
//! A block is bound to what its condition, scrutinee or iterator reads, and
//! nothing else: the bindings inside a branch or row are bound when it is
//! mounted, and released with it. Each branch or row is a [`Row`] with its
//! own data — the values its pattern bound — in a signal, so a binding that
//! reads `n` of `Some(n)`, or the item of a `for`, depends on that row's data
//! like on any other source. Kept branches and rows (same arm, same key) are
//! given their new data, and only the bindings reading it rerun.
//!
//! Names resolve as Rust resolves them, so a memo may only read what is
//! declared before it, and cycles cannot be written:
//!
//! ```compile_fail
//! # use xui_core::*;
//! use xui_macros::component;
//! #[component]
//! fn bad() {
//!     let a: i32 = memo!(*b + 1);
//!     let b: i32 = memo!(0);
//!     view! { text(format!("{a}")) }
//! }
//! # fn main() {}
//! ```
//!
//! Only effects and handlers are given a `Cx` and handles, so nothing else
//! can write:
//!
//! ```compile_fail
//! # use xui_core::*;
//! use xui_macros::component;
//! #[component]
//! fn bad() {
//!     let n: i32 = state!(0);
//!     view! { text({ n.set(cx, 1); "" }) }
//! }
//! # fn main() {}
//! ```
//!
//! A prop holding a handle must say it is a source, or it would be read
//! without being subscribed to:
//!
//! ```compile_fail
//! # use xui_core::*;
//! use xui_core::reactive::ReadSignal;
//! use xui_macros::component;
//! #[component]
//! fn bad(value: &ReadSignal<i32>) {
//!     view! { text(format!("{value}")) }
//! }
//! # fn main() {}
//! ```
//!
//! The same component, written so it compiles:
//!
//! ```
//! # use xui_core::*;
//! use xui_macros::component;
//! #[component]
//! fn good(#[source] value: &i32) {
//!     let doubled: i32 = memo!(*value * 2);
//!     view! { text(format!("{value} {doubled}")) }
//! }
//! # fn main() {}
//! ```
//!
//! A component may take type and const parameters. Every item it generates
//! carries them, with a `T: 'static` bound per type parameter and a
//! `PartialEq` one on the type of a `#[source]` prop or a `memo!`, which
//! compare values to tell whether they changed. A parameter
//! is inferred from the props whose types name it — such a prop takes its
//! value as is, where any other converts it with `Into` — and one no prop
//! names is given with a turbofish, as `typed::<u8>(..)`. A closure for a
//! prop typed by a parameter goes in parentheses, `render: (|v| ..)`, since
//! a bare closure is a callback:
//!
//! ```
//! # use xui_core::*;
//! use std::fmt::Display;
//! use xui_macros::component;
//!
//! #[component]
//! fn labeled<T: Display>(#[source] value: &T) {
//!     view! { text(value.to_string()) }
//! }
//!
//! #[component]
//! fn fixed<const N: usize>(values: &[f32; N]) {
//!     view! { text(format!("{N} values")) }
//! }
//!
//! #[component]
//! fn uses() {
//!     let count: i32 = state!(0);
//!     view! {
//!         column {
//!             labeled(value: count)
//!             labeled(value: "text")
//!             fixed(values: [1.0, 2.0])
//!         }
//!     }
//! }
//! # fn main() {}
//! ```

mod block;
mod graph;
mod host;
mod pass;
mod props;
mod runtime;
mod signal;
mod tasks;
mod template;

pub use block::{BlockCx, Row, RowTemplate};
pub use graph::{EffectId, Graph, InstanceId, Notifier, SourceId};
pub use host::{HostPatcher, HostTag, PatchScope, host, patchable};
pub use pass::{Callback, CallbackArg, Children, PortalSpec, __callback, __children, __size_observer};
pub use props::{
    Component, Current, Expr, FromValue, MountDraft, PropCx, PropKind, SourceArg, Tracked, __convert, __expr, __pending_with,
};
pub use runtime::{Cx, FlushReport, Place, Runtime};
pub use tasks::AsyncValue;
pub use signal::{Memo, ReadSignal, Signal, Source};
pub use template::{
    BuildHost, ChildSpec, HostDraft, ComponentDef, EffectSpec, Lane, MountCx, NodeKind, PendingMount, Run, Scope, StaticNode,
    Template, __apply, __host, __pending,
};

#[cfg(test)]
mod tests;
