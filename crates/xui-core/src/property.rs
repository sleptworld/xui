//! The typed property protocol shared by widget props, capability storage and
//! `view!` slots.
//!
//! A [`Property`] is a zero-sized type naming something settable in one
//! [`Scope`]'s storage: a widget's own props, or the storage block of a
//! [`Capability`] ([`CapScope`]). A node exposes a scope by implementing [`Access`] for it, and
//! that implementation is the only thing that decides whether a property can
//! be set on the node. A capability the node did not select has no `Access`
//! impl, so setting one of its properties fails to compile instead of being
//! stored and ignored.
//!
//! The protocol has three layers, each a property opts into:
//!
//! - [`Property`] — it can be written. Every property, including the
//!   write-only ones: an event handler slot (a handler cannot be read back
//!   out of the node's handler list) or a shorthand writing several fields.
//! - [`FieldProperty`] — it is one stored field, so it can be read back and
//!   compared: a widget's own prop, a capability's field.
//! - [`HostProperty`] — what a change makes the host redo. A component prop
//!   has no such answer — a change means the component renders again.
//!
//! [`Slot`] is the same property seen from an update: it writes unless the
//! value is unchanged, and reports that it did ([`Slot::apply`]) or what the
//! change dirties ([`Slot::update`]). A write-only property can only be
//! written ([`Slot::write`]).

use std::marker::PhantomData;

use xui_interface::WidgetUpdateFlags;

/// A storage block properties live in.
pub trait Scope {
    type Storage: 'static;
}

/// Something settable in a [`Scope`]'s storage.
pub trait Property {
    type Scope: Scope;
    type Value: 'static;

    fn set(storage: &mut <Self::Scope as Scope>::Storage, value: Self::Value);
}

/// A property stored as one field, so that it reads back what was set.
pub trait FieldProperty: Property {
    fn get(storage: &<Self::Scope as Scope>::Storage) -> &Self::Value;

    /// Whether replacing `a` with `b` is a no-op. Handlers compare by
    /// identity.
    fn same(a: &Self::Value, b: &Self::Value) -> bool;
}

/// A field the host pipeline consumes: a host widget's own prop, or a
/// capability's (style, interaction, ..).
///
/// A style property answers [`WidgetUpdateFlags::STYLE_TARGET`] whatever the
/// values; what the change really costs comes out of the computed-style diff.
pub trait HostProperty: FieldProperty {
    /// What replacing `old` with `new` dirties. Only called when the two
    /// differ.
    fn dirty(old: &Self::Value, new: &Self::Value) -> WidgetUpdateFlags;
}

/// A node that holds the storage of scope `S`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no `{S}` storage",
    label = "this node cannot hold properties of this scope",
    note = "a capability's properties exist only on nodes that select that capability"
)]
pub trait Access<S: Scope> {
    fn access(&self) -> &S::Storage;
    fn access_mut(&mut self) -> &mut S::Storage;
}

/// An input property `Self` accepts, and how it becomes the property's value.
///
/// Each property lists the inputs it takes (`f32` for a length, a token for a
/// colour, ..) instead of relying on `Into<P::Value>`, which cannot be
/// implemented for a foreign wrapper such as `StyleValue<T>` per input type.
///
/// Implemented on the property, not on the input: a property declared in a
/// downstream crate is local there, so `impl<T: Into<String>> Accepts<T> for
/// content` is allowed, where `impl<T> IntoProp<content> for T` would leave
/// `T` uncovered before the first local type.
///
/// There is deliberately no blanket impl for the value type itself: it would
/// overlap every `impl<T: Into<V>> Accepts<T>` whose `V` is the value type
/// (`String: Into<String>`). A property that takes its value type says so, as
/// the generated ones do.
#[diagnostic::on_unimplemented(
    message = "property `{Self}` does not accept `{T}`",
    label = "not a value of `{Self}`"
)]
pub trait Accepts<T>: Property {
    fn accept(value: T) -> Self::Value;
}

/// The argument side of [`Accepts`]: what setters take, as
/// `value: impl IntoProp<P>`, so a call names only the property
/// (`set::<width>(10.0)`).
pub trait IntoProp<P: Property> {
    fn into_prop(self) -> P::Value;
}

impl<T, P: Accepts<T>> IntoProp<P> for T {
    fn into_prop(self) -> P::Value {
        P::accept(self)
    }
}

/// What an optional property accepts: the value, an `Option` of it (`None`
/// clears the property), and the inputs that read naturally for a few value
/// types — a `&str` for text, an `f64` for an ordered float.
///
/// A trait of its own because `Into<Option<String>>` takes neither `&str` nor
/// `None` without an annotation, and an `Accepts` impl per input type would
/// overlap `Option<T>` with `T` for two foreign types.
pub trait OptionalValue<T> {
    fn into_option(self) -> Option<T>;
}

impl<T> OptionalValue<T> for T {
    fn into_option(self) -> Option<T> {
        Some(self)
    }
}

impl<T> OptionalValue<T> for Option<T> {
    fn into_option(self) -> Option<T> {
        self
    }
}

impl OptionalValue<String> for &str {
    fn into_option(self) -> Option<String> {
        Some(self.to_owned())
    }
}

impl OptionalValue<xui_interface::OrderedFloat<f64>> for f64 {
    fn into_option(self) -> Option<xui_interface::OrderedFloat<f64>> {
        Some(xui_interface::OrderedFloat(self))
    }
}

/// A unit of opt-in node behaviour that owns one storage block: a style group,
/// interaction, focus, ..
///
/// Only the storage is universal. What a capability does with it (resolve a
/// style, handle routed events, register resources at commit) is a separate
/// trait, so a node never carries an empty implementation of behaviour it
/// does not have.
pub trait Capability {
    type Storage: Default + 'static;
}

/// Implemented by a node's capability storage for each capability the node
/// selected, and for no other.
pub trait Has<C: Capability> {
    fn get(&self) -> &C::Storage;
    fn get_mut(&mut self) -> &mut C::Storage;
}

/// The scope of capability `C`'s storage.
pub struct CapScope<C>(PhantomData<fn() -> C>);

impl<C: Capability> Scope for CapScope<C> {
    type Storage = C::Storage;
}

/// Property `P` as an update target. Zero-sized: `view!` names one per dynamic
/// attribute without allocating.
pub struct Slot<P>(PhantomData<fn() -> P>);

impl<P> Clone for Slot<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for Slot<P> {}

impl<P> Default for Slot<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P> Slot<P> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<P: Property> Slot<P> {
    /// Writes `value`, whatever was there: a write-only property cannot tell.
    pub fn write<T: Access<P::Scope>>(self, target: &mut T, value: impl IntoProp<P>) {
        P::set(target.access_mut(), value.into_prop());
    }
}

impl<P: FieldProperty> Slot<P> {
    pub fn get<T: Access<P::Scope>>(self, target: &T) -> &P::Value {
        P::get(target.access())
    }

    /// Writes `value` unless it equals the current one, and returns whether
    /// it wrote.
    pub fn apply<T: Access<P::Scope>>(self, target: &mut T, value: impl IntoProp<P>) -> bool {
        let value = value.into_prop();
        if P::same(P::get(target.access()), &value) {
            return false;
        }
        P::set(target.access_mut(), value);
        true
    }
}

impl<P: HostProperty> Slot<P> {
    /// Writes `value` unless it equals the current one, and returns what the
    /// write dirtied — empty when nothing was written.
    pub fn update<T: Access<P::Scope>>(
        self,
        target: &mut T,
        value: impl IntoProp<P>,
    ) -> WidgetUpdateFlags {
        let value = value.into_prop();
        let current = P::get(target.access());
        if P::same(current, &value) {
            return WidgetUpdateFlags::empty();
        }
        let flags = P::dirty(current, &value);
        P::set(target.access_mut(), value);
        flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq)]
    struct BoxStorage {
        width: Option<f32>,
    }

    struct BoxLayout;
    impl Capability for BoxLayout {
        type Storage = BoxStorage;
    }

    struct Width;
    impl Property for Width {
        type Scope = CapScope<BoxLayout>;
        type Value = Option<f32>;
        fn set(storage: &mut BoxStorage, value: Option<f32>) {
            storage.width = value;
        }
    }

    impl FieldProperty for Width {
        fn get(storage: &BoxStorage) -> &Option<f32> {
            &storage.width
        }
        fn same(a: &Option<f32>, b: &Option<f32>) -> bool {
            a == b
        }
    }

    impl HostProperty for Width {
        fn dirty(_: &Option<f32>, _: &Option<f32>) -> WidgetUpdateFlags {
            WidgetUpdateFlags::LAYOUT_INPUT
        }
    }

    impl Accepts<f32> for Width {
        fn accept(value: f32) -> Option<f32> {
            Some(value)
        }
    }

    impl Accepts<Option<f32>> for Width {
        fn accept(value: Option<f32>) -> Option<f32> {
            value
        }
    }

    #[derive(Default)]
    struct Node {
        layout: BoxStorage,
    }

    impl Has<BoxLayout> for Node {
        fn get(&self) -> &BoxStorage {
            &self.layout
        }
        fn get_mut(&mut self) -> &mut BoxStorage {
            &mut self.layout
        }
    }

    impl<C: Capability> Access<CapScope<C>> for Node
    where
        Node: Has<C>,
    {
        fn access(&self) -> &C::Storage {
            Has::<C>::get(self)
        }
        fn access_mut(&mut self) -> &mut C::Storage {
            Has::<C>::get_mut(self)
        }
    }

    #[test]
    fn update_writes_only_changes() {
        let mut node = Node::default();
        let width = Slot::<Width>::new();

        assert_eq!(
            width.update(&mut node, 10.0),
            WidgetUpdateFlags::LAYOUT_INPUT
        );
        assert_eq!(*width.get(&node), Some(10.0));
        assert_eq!(
            width.update(&mut node, Some(10.0)),
            WidgetUpdateFlags::empty()
        );
        assert_eq!(
            width.update(&mut node, None),
            WidgetUpdateFlags::LAYOUT_INPUT
        );
        assert_eq!(node.layout, BoxStorage { width: None });
    }

    #[test]
    fn apply_reports_whether_it_wrote() {
        let mut node = Node::default();
        let width = Slot::<Width>::new();

        assert!(width.apply(&mut node, 4.0));
        assert!(!width.apply(&mut node, 4.0));
        assert_eq!(*width.get(&node), Some(4.0));
    }

    #[test]
    fn slot_is_zero_sized() {
        assert_eq!(std::mem::size_of::<Slot<Width>>(), 0);
    }
}
