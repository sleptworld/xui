//! The node abstraction host widgets and components share.
//!
//! A [`NodeDef`] names what a node is made of: its own props, the style
//! capabilities it selected (gathered into one [`StyleSet`]), and the rest of
//! its capabilities (`Behavior`: interaction, focus, ..). Everything settable
//! lives in one [`NodeStorage`], and both the declarative prototype
//! ([`Draft`]) and the mounted instance implement [`Node`] over it, so there is
//! exactly one setter, [`Node::set`], and it exists only for the properties
//! the node has:
//!
//! - a prop of the node's own resolves through [`OwnProps`];
//! - a capability property resolves through [`CapScope`], which a node can
//!   access only if its definition [`Selects`] that capability.
//!
//! Anything else fails to compile at the `set` call.
//!
//! State rules (`hovered`, `pressed`, ..) are layers of the node's `Style`
//! alone, so a rule can declare style but can never reach a prop or a
//! behaviour capability such as a click handler.

use std::marker::PhantomData;

use crate::capability::{CommitCx, StyleSet};
use crate::caps::behavior::{Motion, motion};
use crate::caps::{CapMask, Columns, Registered};
use crate::fiber::Key;
use crate::event_system::{EventContext, Flow, Handler, HandlerValue};
use crate::property::{Access, CapScope, Capability, IntoProp, Property, Scope};
use crate::property::HostProperty;
use xui_interface::style::{ComputedStyle, Theme, WidgetStateMatcher};
use xui_interface::{Animation, NodeId, WidgetState, WidgetUpdateFlags};

mod sealed {
    pub trait Sealed {}
}

/// Whether a node is a host widget or a component.
pub trait NodeKind: sealed::Sealed + 'static {}

/// A node the runtime keeps in the retained tree, laid out and painted.
pub enum HostKind {}
/// A node that renders into other nodes.
pub enum ComponentKind {}

impl sealed::Sealed for HostKind {}
impl sealed::Sealed for ComponentKind {}

impl NodeKind for HostKind {}
impl NodeKind for ComponentKind {}

/// What a node is made of. Implemented by a widget's or component's marker
/// type.
pub trait NodeDef: Sized + 'static {
    type Kind: NodeKind;
    type Props: 'static;
    /// The selected style capabilities' storage; `()` for none. State rules
    /// are layers of this type.
    type Style: StyleSet;
    /// The selected non-style capabilities' storage; `()` for none.
    type Behavior: Default + 'static;

    /// The node's style transition, when it selects the `motion` capability.
    fn transition(_node: &NodeStorage<Self>) -> Option<xui_interface::Transition> {
        None
    }
}

/// Where a node keeps the storage of capability `C`. Implemented once per
/// capability the node selected, and for no other.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not select the `{C}` capability",
    label = "this node has no `{C}` storage"
)]
pub trait Selects<C: Capability>: NodeDef {
    fn storage(node: &NodeStorage<Self>) -> &C::Storage;
    fn storage_mut(node: &mut NodeStorage<Self>) -> &mut C::Storage;
}

/// Everything about a node that a property can set.
pub struct NodeStorage<N: NodeDef> {
    pub props: N::Props,
    pub style: N::Style,
    pub behavior: N::Behavior,
}

impl<N: NodeDef> NodeStorage<N> {
    pub fn new(props: N::Props) -> Self {
        Self {
            props,
            style: N::Style::default(),
            behavior: N::Behavior::default(),
        }
    }
}

impl<N: NodeDef> Clone for NodeStorage<N>
where
    N::Props: Clone,
    N::Behavior: Clone,
{
    fn clone(&self) -> Self {
        Self {
            props: self.props.clone(),
            style: self.style.clone(),
            behavior: self.behavior.clone(),
        }
    }
}

/// The scope of node `N`'s own props.
pub struct OwnProps<N>(PhantomData<fn() -> N>);

impl<N: NodeDef> Scope for OwnProps<N> {
    type Storage = N::Props;
}

/// A node's prototype or its mounted instance: anything holding a
/// [`NodeStorage`].
pub trait Node {
    type Def: NodeDef;

    fn storage(&self) -> &NodeStorage<Self::Def>;
    fn storage_mut(&mut self) -> &mut NodeStorage<Self::Def>;

    /// Sets property `P`. Exists only for the properties this node has.
    fn set<P: Property>(&mut self, value: impl IntoProp<P>)
    where
        Self: Access<P::Scope>,
    {
        P::set(self.access_mut(), value.into_prop());
    }

    /// Sets handler property `P` from a closure: [`set`](Self::set) with the
    /// closure made a [`Handler`].
    ///
    /// The handler's `Fn` bound is spelled out here, derived from `P`'s value,
    /// rather than behind `set`'s `impl IntoProp<P>`: a closure infers its
    /// argument types and its higher-ranked lifetime only from a bound
    /// written where it is passed.
    fn on<P: Property, R: Into<Flow>>(
        &mut self,
        handler: impl for<'a> FnMut(&<P::Value as HandlerValue>::Event, &mut EventContext<'a>) -> R
        + 'static,
    ) where
        P::Value: HandlerValue,
        Self: Access<P::Scope>,
    {
        P::set(
            self.access_mut(),
            P::Value::from_handler(Handler::new(handler)),
        );
    }
}

impl<T: Node> Access<OwnProps<T::Def>> for T {
    fn access(&self) -> &<T::Def as NodeDef>::Props {
        &self.storage().props
    }

    fn access_mut(&mut self) -> &mut <T::Def as NodeDef>::Props {
        &mut self.storage_mut().props
    }
}

impl<T: Node, C: Capability> Access<CapScope<C>> for T
where
    T::Def: Selects<C>,
{
    fn access(&self) -> &C::Storage {
        <T::Def as Selects<C>>::storage(self.storage())
    }

    fn access_mut(&mut self) -> &mut C::Storage {
        <T::Def as Selects<C>>::storage_mut(self.storage_mut())
    }
}

/// One state-dependent layer of a node's style.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleRule<S> {
    pub matcher: WidgetStateMatcher,
    pub style: Layer<S>,
}

/// The two keyframes of a node's animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframes<S> {
    pub from: Layer<S>,
    pub to: Layer<S>,
}

/// A reusable style for nodes of definition `N`: declarations, and state
/// rules over them. Typed by the node, so a style for a container cannot be
/// given to a text, and a property `N` lacks cannot be written into it.
pub struct StyleOf<N: NodeDef> {
    pub base: Layer<N::Style>,
    pub rules: Vec<StyleRule<N::Style>>,
}

impl<N: NodeDef> StyleOf<N> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets style property `P`, chained.
    pub fn with<P: Property>(mut self, value: impl IntoProp<P>) -> Self
    where
        Layer<N::Style>: Access<P::Scope>,
    {
        self.base.set::<P>(value);
        self
    }

    /// Adds a layer that applies while `matcher` matches the node's state.
    pub fn when(
        mut self,
        matcher: WidgetStateMatcher,
        build: impl FnOnce(&mut Layer<N::Style>),
    ) -> Self {
        let mut style = Layer::default();
        build(&mut style);
        self.rules.push(StyleRule { matcher, style });
        self
    }

    /// Layers `other` over `self`.
    pub fn merge(&mut self, other: &Self) {
        self.base.0.merge(&other.base.0);
        self.rules.extend(other.rules.iter().cloned());
    }

    /// [`merge`](Self::merge), by value.
    pub fn merged(mut self, other: &Self) -> Self {
        self.merge(other);
        self
    }
}

impl<N: NodeDef> Default for StyleOf<N> {
    fn default() -> Self {
        Self {
            base: Layer::default(),
            rules: Vec::new(),
        }
    }
}

impl<N: NodeDef> Clone for StyleOf<N> {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            rules: self.rules.clone(),
        }
    }
}

impl<N: NodeDef> PartialEq for StyleOf<N> {
    fn eq(&self, other: &Self) -> bool {
        self.base == other.base && self.rules == other.rules
    }
}

impl<N: NodeDef> std::fmt::Debug for StyleOf<N>
where
    N::Style: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StyleOf")
            .field("base", &self.base.0)
            .field("rules", &self.rules)
            .finish()
    }
}

/// A style layer: what a state rule declares. It can hold only the style
/// capabilities the node selected, through the style's own `Has` impls.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layer<S>(pub S);

impl<S> Layer<S> {
    /// Sets style property `P` on this layer.
    pub fn set<P: Property>(&mut self, value: impl IntoProp<P>)
    where
        Self: Access<P::Scope>,
    {
        P::set(self.access_mut(), value.into_prop());
    }
}

impl<S: crate::property::Has<C>, C: Capability> Access<CapScope<C>> for Layer<S> {
    fn access(&self) -> &C::Storage {
        self.0.get()
    }

    fn access_mut(&mut self) -> &mut C::Storage {
        self.0.get_mut()
    }
}

/// A node as written: its storage and its state rules, before it mounts.
pub struct Draft<N: NodeDef> {
    pub key: Option<Key>,
    pub storage: NodeStorage<N>,
    pub rules: Vec<StyleRule<N::Style>>,
    /// The keyframes of the node's animation, whose timing is the `motion`
    /// capability's.
    pub keyframes: Option<Keyframes<N::Style>>,
}

impl<N: NodeDef> Clone for Draft<N>
where
    N::Props: Clone,
    N::Behavior: Clone,
{
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            storage: self.storage.clone(),
            rules: self.rules.clone(),
            keyframes: self.keyframes.clone(),
        }
    }
}

impl<N: NodeDef> Draft<N> {
    pub fn new(props: N::Props) -> Self {
        Self {
            key: None,
            storage: NodeStorage::new(props),
            rules: Vec::new(),
            keyframes: None,
        }
    }

    /// The node's identity among its siblings, across renders.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Layers `style` over the node's: its declarations over the node's own,
    /// its state rules after the node's.
    pub fn style(mut self, style: StyleOf<N>) -> Self {
        self.storage.style.merge(&style.base.0);
        self.rules.extend(style.rules);
        self
    }

    /// Plays `animation` between two keyframes, each a layer of the node's
    /// style. The node must select `motion`, which holds the timing.
    pub fn animate(
        mut self,
        animation: Animation,
        from: impl FnOnce(&mut Layer<N::Style>),
        to: impl FnOnce(&mut Layer<N::Style>),
    ) -> Self
    where
        Self: Access<CapScope<Motion>>,
    {
        let mut keyframes = Keyframes {
            from: Layer::default(),
            to: Layer::default(),
        };
        from(&mut keyframes.from);
        to(&mut keyframes.to);
        self.set::<motion::props::animation>(animation);
        self.keyframes = Some(keyframes);
        self
    }

    /// [`Node::set`], chained.
    pub fn with<P: Property>(mut self, value: impl IntoProp<P>) -> Self
    where
        Self: Access<P::Scope>,
    {
        self.set::<P>(value);
        self
    }

    /// [`Node::on`], chained.
    pub fn with_on<P: Property, R: Into<Flow>>(
        mut self,
        handler: impl for<'a> FnMut(&<P::Value as HandlerValue>::Event, &mut EventContext<'a>) -> R
        + 'static,
    ) -> Self
    where
        P::Value: HandlerValue,
        Self: Access<P::Scope>,
    {
        self.on::<P, R>(handler);
        self
    }

    /// Adds a style layer that applies while `matcher` matches the node's
    /// state. Later rules win over earlier ones.
    pub fn when(
        mut self,
        matcher: WidgetStateMatcher,
        build: impl FnOnce(&mut Layer<N::Style>),
    ) -> Self {
        let mut style = Layer::default();
        build(&mut style);
        self.rules.push(StyleRule { matcher, style });
        self
    }

    /// The node's computed style in `state`, under `parent`.
    pub fn compute_style(
        &self,
        state: WidgetState,
        parent: &ComputedStyle,
        theme: &Theme,
    ) -> ComputedStyle {
        compute_style(&self.storage.style, &self.rules, state, parent, theme)
    }
}

impl<N: NodeDef> Node for Draft<N> {
    type Def = N;

    fn storage(&self) -> &NodeStorage<N> {
        &self.storage
    }

    fn storage_mut(&mut self) -> &mut NodeStorage<N> {
        &mut self.storage
    }
}

/// A host node definition as the runtime keeps it: the capabilities it
/// selects, and how a draft of it spreads over the runtime's columns.
/// Generated by `#[widget]`.
pub trait HostNode: NodeDef<Kind = HostKind> {
    /// Every capability the node selects.
    const CAPS: CapMask;

    /// Moves `draft` into `columns` as node `id`'s rows, returning the node's
    /// own props and what changed: every row and every style capability's
    /// state layers compare against what `id` held before.
    #[doc(hidden)]
    fn scatter(
        draft: Draft<Self>,
        id: NodeId,
        columns: &mut Columns,
        cx: &mut CommitCx<'_>,
    ) -> (Self::Props, WidgetUpdateFlags);

    /// What replacing props `old` with `new` dirties, prop by prop.
    fn props_dirty(old: &Self::Props, new: &Self::Props) -> WidgetUpdateFlags;
}

/// A host widget: a host node with state of its own, created from its props
/// when it mounts. Implemented by the widget's author; `#[widget]` generates
/// the node half (`HostNode`) and the state struct, not this.
pub trait WidgetT: HostNode {
    type State: 'static;

    fn create_state(props: &Self::Props) -> Self::State;

    /// Called after a commit replaced the props `old` with `new`, so the state
    /// can follow them. Returns what the state's own change dirties, beyond
    /// what the props' [`HostNode::props_dirty`] already says.
    fn update(_old: &Self::Props, _new: &Self::Props, _state: &mut Self::State) -> WidgetUpdateFlags {
        WidgetUpdateFlags::empty()
    }
}

/// A mounted widget's own data: its props and its state. Its capabilities'
/// rows live in the runtime's columns. The widget's roles are implemented on
/// this type.
pub struct WidgetInstance<W: WidgetT> {
    pub props: W::Props,
    pub state: W::State,
}

impl<W: WidgetT> std::fmt::Debug for WidgetInstance<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetInstance")
            .field("widget", &std::any::type_name::<W>())
            .finish_non_exhaustive()
    }
}

/// A mounted host node, restored to its type: its own props, and its rows in
/// the runtime's columns.
///
/// It updates rather than hands out `&mut` storage, so that a capability's
/// row changes only through [`Registered::write`], which reports its dirt and
/// commits a binding: a focus handle swapped in place would never rebind.
// Only tests restore nodes until the runtime keeps them in columns.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Mounted<'a, 'cx, N: HostNode> {
    pub(crate) id: NodeId,
    pub(crate) props: &'a mut N::Props,
    pub(crate) columns: &'a mut Columns,
    pub(crate) cx: &'a mut CommitCx<'cx>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl<N: HostNode> Mounted<'_, '_, N> {
    /// Sets property `P`, unless it already holds `value`; returns what that
    /// dirtied. Exists only for the properties the node has.
    pub(crate) fn update<P: HostProperty>(&mut self, value: impl IntoProp<P>) -> WidgetUpdateFlags
    where
        Self: UpdateScope<P::Scope>,
    {
        <Self as UpdateScope<P::Scope>>::update::<P>(self, value.into_prop())
    }

    /// Sets property `P`, which may be write-only: the row it lands in is
    /// compared whole, so the answer is the capability's
    /// [`dirty`](crate::capability::HostCapability::dirty).
    pub(crate) fn set<P: Property>(&mut self, value: impl IntoProp<P>) -> WidgetUpdateFlags
    where
        Self: SetScope<P::Scope>,
    {
        <Self as SetScope<P::Scope>>::set::<P>(self, value.into_prop())
    }

    /// [`set`](Self::set) with a closure made a [`Handler`], as [`Node::on`].
    pub(crate) fn on<P: Property, R: Into<Flow>>(
        &mut self,
        handler: impl for<'a> FnMut(&<P::Value as HandlerValue>::Event, &mut EventContext<'a>) -> R
        + 'static,
    ) -> WidgetUpdateFlags
    where
        P::Value: HandlerValue,
        Self: SetScope<P::Scope>,
    {
        <Self as SetScope<P::Scope>>::set::<P>(self, P::Value::from_handler(Handler::new(handler)))
    }
}

/// How a [`Mounted`] node writes any property, write-only ones included,
/// into one capability scope.
pub(crate) trait SetScope<S: Scope> {
    fn set<P: Property<Scope = S>>(&mut self, value: P::Value) -> WidgetUpdateFlags;
}

impl<N, C> SetScope<CapScope<C>> for Mounted<'_, '_, N>
where
    N: HostNode + Selects<C>,
    C: Registered,
{
    fn set<P: Property<Scope = CapScope<C>>>(&mut self, value: P::Value) -> WidgetUpdateFlags {
        let mut row = C::get(self.columns, self.id)
            .expect("a node has a row for every capability it selects")
            .clone();
        P::set(&mut row, value);
        C::write(self.columns, self.id, row, self.cx)
    }
}

/// How a [`Mounted`] node writes a field into one scope.
pub(crate) trait UpdateScope<S: Scope> {
    fn update<P: HostProperty<Scope = S>>(&mut self, value: P::Value) -> WidgetUpdateFlags;
}

impl<N: HostNode> UpdateScope<OwnProps<N>> for Mounted<'_, '_, N> {
    fn update<P: HostProperty<Scope = OwnProps<N>>>(
        &mut self,
        value: P::Value,
    ) -> WidgetUpdateFlags {
        let current = P::get(self.props);
        if P::same(current, &value) {
            return WidgetUpdateFlags::empty();
        }
        let dirty = P::dirty(current, &value);
        P::set(self.props, value);
        dirty
    }
}

impl<N, C> UpdateScope<CapScope<C>> for Mounted<'_, '_, N>
where
    N: HostNode + Selects<C>,
    C: Registered,
{
    fn update<P: HostProperty<Scope = CapScope<C>>>(
        &mut self,
        value: P::Value,
    ) -> WidgetUpdateFlags {
        let row = C::get(self.columns, self.id)
            .expect("a node has a row for every capability it selects");
        if P::same(P::get(row), &value) {
            return WidgetUpdateFlags::empty();
        }
        let mut row = row.clone();
        P::set(&mut row, value);
        C::write(self.columns, self.id, row, self.cx)
    }
}

/// Resolves `base` with every rule matching `state` layered over it, against
/// `parent`.
///
/// The layers merge before anything resolves: resolving them one after the
/// other would differ for a property derived from another, such as a stroke
/// following a border a rule changes.
pub fn compute_style<S: StyleSet>(
    base: &S,
    rules: &[StyleRule<S>],
    state: WidgetState,
    parent: &ComputedStyle,
    theme: &Theme,
) -> ComputedStyle {
    let initial = ComputedStyle::initial(theme);
    let mut computed = parent.inherited_from(theme);
    let mut matching = rules.iter().filter(|rule| rule.matcher.matches(state));
    match matching.next() {
        None => base.apply(&mut computed, parent, &initial, theme),
        Some(first) => {
            let mut effective = base.clone();
            effective.merge(&first.style.0);
            for rule in matching {
                effective.merge(&rule.style.0);
            }
            effective.apply(&mut computed, parent, &initial, theme);
        }
    }
    computed
}

#[cfg(test)]
mod tests {
    use xui_macros::widget;

    use super::{Draft, Node};
    use crate::caps::style::box_size;
    use xui_interface::styles::units::StyleValue;

    /// `#[widget]` inside `xui-core` itself names the runtime as `crate`, as
    /// the built-in widgets will use it.
    #[widget(style(box_size), behavior(focus))]
    struct Probe {
        #[default]
        #[dirty(LAYOUT_INPUT)]
        wrap: bool,
    }

    #[test]
    fn widget_macro_expands_inside_the_crate() {
        let mut node = Draft::<Probe>::new(ProbeProps::builder().build());
        node.set::<probe::width>(10.0);
        node.set::<probe::wrap>(true);
        node.set::<probe::tab_index>(1);

        let storage = &node.storage;
        assert!(storage.props.wrap);
        assert_eq!(
            storage.style.box_size,
            box_size::Storage {
                width: StyleValue::Value(10.0.into()),
                ..Default::default()
            }
        );
        assert_eq!(storage.behavior.focus.tab_index, Some(1));
    }
}
