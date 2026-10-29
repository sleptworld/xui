//! Writing an attribute into a mounted host: what a render binding does
//! when what it reads changes.
use xui_interface::{NodeId, WidgetUpdateFlags};

use crate::capability::CommitCx;
use crate::caps::{Columns, Registered};
use crate::node::{Draft, NodeDef, OwnProps, WholeNode, WidgetInstance, WidgetT};
use crate::property::{CapScope, Property, PropertyMarker, Scope};
use crate::widgets::WidgetVariant;

/// A mounted host of type `N`, as a render binding writes its dynamic
/// attributes into it: its own props and state, and its capabilities' rows.
/// Public only because generated code names it.
#[doc(hidden)]
pub struct HostPatcher<'a, 'cx, N: WidgetT> {
    instance: &'a mut WidgetInstance<N>,
    id: NodeId,
    columns: &'a mut Columns,
    cx: &'a mut CommitCx<'cx>,
}

impl<'a, 'cx, N: WidgetT> HostPatcher<'a, 'cx, N> {
    pub(crate) fn new(
        instance: &'a mut WidgetInstance<N>,
        id: NodeId,
        columns: &'a mut Columns,
        cx: &'a mut CommitCx<'cx>,
    ) -> Self {
        Self { instance, id, columns, cx }
    }

    /// The host's property table, as the element macros look attributes up.
    #[inline(always)]
    pub fn __properties(&self) -> N::Properties {
        N::Properties::default()
    }

    /// Writes `value` as property `P`; returns what that dirtied.
    #[inline]
    pub fn __patch<P: Property>(&mut self, _: PropertyMarker<P>, value: P::Value) -> WidgetUpdateFlags
    where
        P::Scope: PatchScope<N>,
    {
        <P::Scope as PatchScope<N>>::patch::<P>(self, value)
    }
}

/// How a property of scope `Self` is written into a mounted host of type
/// `N`, if it can be. Every scope a host property can have implements it, so
/// that the element macros can ask any attribute whether it patches.
#[doc(hidden)]
pub trait PatchScope<N: WidgetT>: Scope + Sized {
    /// Whether a property of this scope can be written into a mounted host
    /// alone. When one of a host's dynamic attributes cannot, the host is
    /// rebuilt whole.
    const PATCHABLE: bool;

    fn patch<P: Property<Scope = Self>>(
        target: &mut HostPatcher<'_, '_, N>,
        value: P::Value,
    ) -> WidgetUpdateFlags;
}

/// A capability field: the row with the field replaced, written back the way
/// a scatter writes it, which compares, commits and reports.
impl<N: WidgetT, C: Registered> PatchScope<N> for CapScope<C>
where
    C::Storage: Clone,
{
    const PATCHABLE: bool = true;

    fn patch<P: Property<Scope = Self>>(
        target: &mut HostPatcher<'_, '_, N>,
        value: P::Value,
    ) -> WidgetUpdateFlags {
        let mut row = C::get(target.columns, target.id)
            .cloned()
            .unwrap_or_default();
        P::set(&mut row, value);
        C::write(target.columns, target.id, row, target.cx)
    }
}

/// One of the host's own props: replaced in place, then compared and handed
/// to the widget's state as a whole update would.
impl<N: WidgetT> PatchScope<N> for OwnProps<N>
where
    N::Props: Clone,
{
    const PATCHABLE: bool = true;

    fn patch<P: Property<Scope = Self>>(
        target: &mut HostPatcher<'_, '_, N>,
        value: P::Value,
    ) -> WidgetUpdateFlags {
        let instance = &mut *target.instance;
        let old = instance.props.clone();
        P::set(&mut instance.props, value);
        N::props_dirty(&old, &instance.props) | N::update(&old, &instance.props, &mut instance.state)
    }
}

/// `style`, which layers declarations and rules over the whole node: only a
/// rebuild knows what it replaced.
impl<N: WidgetT> PatchScope<N> for WholeNode<N> {
    const PATCHABLE: bool = false;

    fn patch<P: Property<Scope = Self>>(
        _: &mut HostPatcher<'_, '_, N>,
        _: P::Value,
    ) -> WidgetUpdateFlags {
        unreachable!("a host with a `style` attribute is rebuilt, never patched")
    }
}


/// Whether attribute `property` of a host of `tag`'s type can be patched.
#[doc(hidden)]
#[inline(always)]
pub fn patchable<N: WidgetT, P: Property>(
    _tag: fn() -> Draft<N>,
    _property: impl FnOnce(&N::Properties) -> PropertyMarker<P>,
) -> bool
where
    P::Scope: PatchScope<N>,
{
    <P::Scope as PatchScope<N>>::PATCHABLE
}

/// Asserts that `tag` constructs a host widget, as the element macros
/// assumed from its name.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a host widget",
    label = "the element macros treat this tag as a host widget by its name",
    note = "host widget names are reserved: a component cannot be called `container`, `text`, .."
)]
pub trait HostTag {}

impl<N: WidgetVariant> HostTag for Draft<N> {}

#[doc(hidden)]
pub fn host<N, R>(tag: fn() -> Draft<N, R>) -> fn() -> Draft<N, R>
where
    N: NodeDef,
    Draft<N, R>: HostTag,
{
    tag
}

