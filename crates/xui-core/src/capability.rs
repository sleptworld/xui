//! The capability protocols, and the two macros that define capabilities
//! from a field list: [`style_capability!`] for style, and
//! [`behavior_capability!`](crate::behavior_capability) for the rest.
//!
//! A style capability is one storage block of declared [`StyleValue`]s. It
//! resolves into the dense [`ComputedStyle`]: each property names the
//! computed field it writes, so a node that lacks a capability leaves those
//! fields at their initial (or inherited) values and the layout, paint and
//! text passes read one shape for every node.

use crate::property::{Capability, HostProperty};
use crate::scroll::ScrollRequestQueue;
use crate::size::SizeObserver;
use xui_interface::style::{ComputedStyle, Theme};
use xui_interface::{NodeId, WidgetUpdateFlags};

/// A capability a host node can select: the runtime keeps its storage in a
/// column of its own (see [`crate::caps`]), compares rows, and dirties
/// [`DIRTY`](Self::DIRTY) when one changes.
pub trait HostCapability: Capability<Storage: Clone + PartialEq> {
    /// What a row dirties when it first appears, and by default when it
    /// changes. Equal to every one of its properties'
    /// [`HostProperty::dirty`].
    const DIRTY: WidgetUpdateFlags;

    /// What replacing row `old` with an unequal `new` dirties. A capability
    /// whose rows change on every render without consequence — event
    /// handlers, recreated with their closures — narrows it.
    fn dirty(_old: &Self::Storage, _new: &Self::Storage) -> WidgetUpdateFlags {
        Self::DIRTY
    }
}

/// A capability whose storage binds something outside the node — a handle,
/// a registration — that must follow the node's commits.
///
/// Called only when a row changes: `old` is `None` on mount, `new` is `None`
/// on unmount. Never called for a render that does not commit.
pub(crate) trait CommitCapability: HostCapability {
    fn commit(
        node: NodeId,
        old: Option<&Self::Storage>,
        new: Option<&Self::Storage>,
        cx: &mut CommitCx<'_>,
    );
}

/// What a [`CommitCapability`] may touch while it commits. Public only
/// because generated code passes it along.
#[doc(hidden)]
pub struct CommitCx<'a> {
    /// Where every bound `ScrollController` queues its requests.
    pub(crate) scroll_requests: &'a ScrollRequestQueue,
    /// Observers that lost their node, notified once commit finishes.
    pub(crate) detached_size_observers: &'a mut Vec<SizeObserver>,
}

/// A capability whose storage is style declarations.
pub trait StyleCapability: Capability {
    /// Layers `other` over `target`: what `other` declares wins.
    fn merge(target: &mut Self::Storage, other: &Self::Storage);

    /// Resolves every declaration in `storage` into `target`, in field order.
    ///
    /// `target` starts as what the node inherits (see
    /// [`ComputedStyle::inherited_from`]); `initial` is the theme's initial
    /// style, passed in so it is built once per resolve, not once per field.
    fn apply(
        storage: &Self::Storage,
        target: &mut ComputedStyle,
        parent: &ComputedStyle,
        initial: &ComputedStyle,
        theme: &Theme,
    );

    /// Whether [`inherit_unset`](Self::inherit_unset) would change `current`.
    fn inherits_differently(
        storage: &Self::Storage,
        current: &ComputedStyle,
        parent: &ComputedStyle,
    ) -> bool;

    /// Copies from `parent` each inherited property `storage` leaves to the
    /// parent.
    fn inherit_unset(storage: &Self::Storage, target: &mut ComputedStyle, parent: &ComputedStyle);
}

/// A node's selected style capabilities, as one value: its base style and
/// each of its state layers. Generated per node; `()` selects none.
pub trait StyleSet: Default + Clone + PartialEq + 'static {
    /// Layers `other` over `self`, capability by capability.
    fn merge(&mut self, other: &Self);

    /// Resolves every selected capability into `target`. What the node did
    /// not select keeps its initial or inherited value.
    fn apply(
        &self,
        target: &mut ComputedStyle,
        parent: &ComputedStyle,
        initial: &ComputedStyle,
        theme: &Theme,
    );

    fn inherits_differently(&self, current: &ComputedStyle, parent: &ComputedStyle) -> bool;

    fn inherit_unset(&self, target: &mut ComputedStyle, parent: &ComputedStyle);
}

impl StyleSet for () {
    fn merge(&mut self, _: &Self) {}

    fn apply(&self, _: &mut ComputedStyle, _: &ComputedStyle, _: &ComputedStyle, _: &Theme) {}

    fn inherits_differently(&self, _: &ComputedStyle, _: &ComputedStyle) -> bool {
        false
    }

    fn inherit_unset(&self, _: &mut ComputedStyle, _: &ComputedStyle) {}
}

/// One declared field of a style capability.
pub trait StyleProperty: HostProperty {
    /// Whether an unset declaration takes the parent's computed value.
    const INHERITED: bool;

    /// Resolves `value` into the computed field this property writes.
    fn resolve(
        value: &Self::Value,
        target: &mut ComputedStyle,
        parent: &ComputedStyle,
        initial: &ComputedStyle,
        theme: &Theme,
    );

    fn inherits_differently(
        value: &Self::Value,
        current: &ComputedStyle,
        parent: &ComputedStyle,
    ) -> bool;

    fn inherit_unset(value: &Self::Value, target: &mut ComputedStyle, parent: &ComputedStyle);
}

/// Defines a style capability: its marker, its storage, and one
/// [`Property`](crate::property::Property) per field.
///
/// ```ignore
/// style_capability! {
///     /// Docs for the capability.
///     pub mod border: Border {
///         /// Docs for the field and its property.
///         border_color: ColorStyle => paint.border_color,
///         border_width: LengthValue => paint.border_width,
///         stroke: StrokeStyle => paint.stroke [resolve = resolve_stroke],
///     }
/// }
/// ```
///
/// expands to `pub mod border` holding
///
/// - `Border`, the [`Capability`] marker, also re-exported next to the module;
/// - `Storage`, one `StyleValue<_>` per field, which is `Border`'s storage;
/// - `props::border_color`, .. — one uninhabited enum per field, named exactly
///   like the attribute. Uninhabited rather than a unit struct, so that a
///   local `let border_width = ..` next to a glob import of `props` still
///   binds instead of being read as a pattern. Each implements `Property`,
///   [`HostProperty`] (always `STYLE_TARGET`; the computed-style diff decides
///   the real cost), [`StyleProperty`], and `Accepts` whatever
///   [`IntoStyleValue`](xui_interface::styles::units::IntoStyleValue) its
///   declared type: a declaration itself, or anything `Into` the type.
///
/// `=> path` is the [`ComputedStyle`] field the property resolves into, in
/// the type [`StyleCompute`](xui_interface::styles::compute::StyleCompute) maps the declared type to. A field may end in
/// options:
///
/// - `[inherited]` — an unset declaration takes the parent's value;
/// - `[resolve = f]` — `f(&value, target, parent, initial, theme)` replaces
///   the default [`resolve_field`](xui_interface::styles::compute::resolve_field), for a field that depends on another.
///
/// Fields resolve in the order written, so a custom resolver can read the
/// fields declared above it.
macro_rules! style_capability {
    (
        $(#[$cap_meta:meta])*
        $vis:vis mod $module:ident : $cap:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident : $declared:ty => $($path:ident).+ $([$($opt:tt)*])?
            ),* $(,)?
        }
    ) => {
        $(#[$cap_meta])*
        $vis mod $module {
            #[allow(unused_imports)]
            use super::*;

            $(#[$cap_meta])*
            pub struct $cap;

            impl $crate::property::Capability for $cap {
                type Storage = Storage;
            }

            /// A style row's change re-resolves the computed style, which
            /// decides what it really costs.
            impl $crate::capability::HostCapability for $cap {
                const DIRTY: $crate::__interface::WidgetUpdateFlags =
                    $crate::__interface::WidgetUpdateFlags::STYLE_TARGET;
            }

            #[doc = concat!("The declarations of [`", stringify!($cap), "`].")]
            #[derive(Debug, Clone, PartialEq, Default)]
            pub struct Storage {
                $(
                    $(#[$field_meta])*
                    pub $field: $crate::__interface::StyleValue<$declared>,
                )*
            }

            impl $crate::capability::StyleCapability for $cap {
                fn merge(target: &mut Storage, other: &Storage) {
                    $($crate::__interface::merge_value(&mut target.$field, &other.$field);)*
                }

                #[allow(unused_variables)]
                fn apply(
                    storage: &Storage,
                    target: &mut $crate::__interface::ComputedStyle,
                    parent: &$crate::__interface::ComputedStyle,
                    initial: &$crate::__interface::ComputedStyle,
                    theme: &$crate::__interface::Theme,
                ) {
                    $(
                        <props::$field as $crate::capability::StyleProperty>::resolve(
                            &storage.$field, target, parent, initial, theme,
                        );
                    )*
                }

                #[allow(unused_variables)]
                fn inherits_differently(
                    storage: &Storage,
                    current: &$crate::__interface::ComputedStyle,
                    parent: &$crate::__interface::ComputedStyle,
                ) -> bool {
                    false $(
                        || <props::$field as $crate::capability::StyleProperty>
                            ::inherits_differently(&storage.$field, current, parent)
                    )*
                }

                #[allow(unused_variables)]
                fn inherit_unset(
                    storage: &Storage,
                    target: &mut $crate::__interface::ComputedStyle,
                    parent: &$crate::__interface::ComputedStyle,
                ) {
                    $(
                        <props::$field as $crate::capability::StyleProperty>
                            ::inherit_unset(&storage.$field, target, parent);
                    )*
                }
            }

            /// One property per field, named like its attribute.
            #[allow(non_camel_case_types)]
            pub mod props {
                #[allow(unused_imports)]
                use super::*;

                $(
                    $(#[$field_meta])*
                    pub enum $field {}

                    impl $crate::property::Property for $field {
                        type Scope = $crate::property::CapScope<$cap>;
                        type Value = $crate::__interface::StyleValue<$declared>;

                        fn set(storage: &mut Storage, value: Self::Value) {
                            storage.$field = value;
                        }
                    }

                    impl $crate::property::FieldProperty for $field {
                        fn get(storage: &Storage) -> &Self::Value {
                            &storage.$field
                        }

                        fn same(a: &Self::Value, b: &Self::Value) -> bool {
                            a == b
                        }
                    }

                    impl $crate::property::HostProperty for $field {
                        fn dirty(_: &Self::Value, _: &Self::Value) -> $crate::__interface::WidgetUpdateFlags {
                            $crate::__interface::WidgetUpdateFlags::STYLE_TARGET
                        }
                    }

                    impl $crate::capability::StyleProperty for $field {
                        const INHERITED: bool =
                            $crate::capability::style_capability!(@inherited $($($opt)*)?);

                        fn resolve(
                            value: &Self::Value,
                            target: &mut $crate::__interface::ComputedStyle,
                            parent: &$crate::__interface::ComputedStyle,
                            initial: &$crate::__interface::ComputedStyle,
                            theme: &$crate::__interface::Theme,
                        ) {
                            $crate::capability::style_capability!(
                                @resolve [$($($opt)*)?]
                                (value, target, parent, initial, theme, $($path).+)
                            );
                        }

                        fn inherits_differently(
                            value: &Self::Value,
                            current: &$crate::__interface::ComputedStyle,
                            parent: &$crate::__interface::ComputedStyle,
                        ) -> bool {
                            <Self as $crate::capability::StyleProperty>::INHERITED
                                && $crate::__interface::leaves_to_parent(value)
                                && current.$($path).+ != parent.$($path).+
                        }

                        fn inherit_unset(
                            value: &Self::Value,
                            target: &mut $crate::__interface::ComputedStyle,
                            parent: &$crate::__interface::ComputedStyle,
                        ) {
                            if <Self as $crate::capability::StyleProperty>::INHERITED
                                && $crate::__interface::leaves_to_parent(value)
                            {
                                target.$($path).+ = parent.$($path).+.clone();
                            }
                        }
                    }

                    impl<T> $crate::property::Accepts<T> for $field
                    where
                        T: $crate::__interface::IntoStyleValue<$declared>,
                    {
                        fn accept(value: T) -> $crate::__interface::StyleValue<$declared> {
                            value.into_style_value()
                        }
                    }
                )*
            }
        }

        $vis use $module::$cap;
    };

    (@inherited) => { false };
    (@inherited inherited) => { true };
    (@inherited resolve = $resolve:path) => { false };

    (@resolve [$(inherited)?] ($value:ident, $target:ident, $parent:ident, $initial:ident, $theme:ident, $($path:ident).+)) => {
        $crate::__interface::resolve_field(
            $value,
            <Self as $crate::capability::StyleProperty>::INHERITED,
            &mut $target.$($path).+,
            &$parent.$($path).+,
            &$initial.$($path).+,
            $theme,
        )
    };
    (@resolve [resolve = $resolve:path] ($value:ident, $target:ident, $parent:ident, $initial:ident, $theme:ident, $($path:ident).+)) => {
        $resolve($value, $target, $parent, $initial, $theme)
    };
}

pub(crate) use style_capability;

/// Defines a behaviour capability: its marker, its storage, and one
/// [`Property`](crate::property::Property) per field.
///
/// ```ignore
/// behavior_capability! {
///     /// Docs for the capability.
///     pub mod shortcuts: Shortcuts dirty(STATE_CHANGE) {
///         /// Docs for the field and its property.
///         shortcuts: Vec<ShortcutBinding>,
///     }
/// }
/// ```
///
/// generates `pub mod shortcuts` with the `Shortcuts` marker (re-exported
/// next to the module), a `Storage` struct with one field per property, and
/// `props::shortcuts`. Every property is a `HostProperty` whose change
/// dirties the flags in `dirty(..)`.
///
/// The storage can instead be an existing type the runtime already reads,
/// each property naming the field it maps to:
///
/// ```ignore
/// pub mod focus: Focus(FocusProperties) dirty(STATE_CHANGE) {
///     focusable: Focusability => focusability,
///     tab_index: i32 => tab_index [optional],
/// }
/// ```
///
/// `(Storage, whole = name)` adds a property `name` covering the whole
/// storage. A field ends in at most one option:
///
/// - none — the property accepts anything `Into` the type;
/// - `[exact]` — only the type itself, so that integer literals infer;
/// - `[optional]` — the value is `Option<type>`, accepting the type, an
///   `Option` of it, or an [`OptionalValue`](crate::property::OptionalValue)
///   input such as `&str`.
#[macro_export]
macro_rules! behavior_capability {
    // Storage generated from the fields.
    (
        $(#[$cap_meta:meta])*
        $vis:vis mod $module:ident : $cap:ident dirty $dirty:tt {
            $(
                $(#[$field_meta:meta])*
                $prop:ident : $ty:ty $([$kind:ident])?
            ),* $(,)?
        }
    ) => {
        $(#[$cap_meta])*
        $vis mod $module {
            #[allow(unused_imports)]
            use super::*;

            #[doc = concat!("The storage of [`", stringify!($cap), "`].")]
            #[derive(Debug, Clone, PartialEq, Default)]
            pub struct Storage {
                $(
                    $(#[$field_meta])*
                    pub $prop: $crate::behavior_capability!(@value $ty $([$kind])?),
                )*
            }

            $crate::behavior_capability!(@capability $(#[$cap_meta])* $cap, $dirty);

            /// One property per field, named like its attribute.
            #[allow(non_camel_case_types)]
            pub mod props {
                #[allow(unused_imports)]
                use super::*;

                $(
                    $crate::behavior_capability!(
                        @prop $cap, $dirty, $(#[$field_meta])* $prop : $ty => $prop $([$kind])?
                    );
                )*
            }
        }

        $vis use $module::$cap;
    };

    // An existing storage type.
    (
        $(#[$cap_meta:meta])*
        $vis:vis mod $module:ident : $cap:ident ($storage:ty $(, whole = $whole:ident)?)
            dirty $dirty:tt
        {
            $(
                $(#[$field_meta:meta])*
                $prop:ident : $ty:ty => $field:ident $([$kind:ident])?
            ),* $(,)?
        }
    ) => {
        $(#[$cap_meta])*
        $vis mod $module {
            #[allow(unused_imports)]
            use super::*;

            #[doc = concat!("The storage of [`", stringify!($cap), "`].")]
            pub type Storage = $storage;

            $crate::behavior_capability!(@capability $(#[$cap_meta])* $cap, $dirty);

            /// One property per field, named like its attribute.
            #[allow(non_camel_case_types)]
            pub mod props {
                #[allow(unused_imports)]
                use super::*;

                $(
                    $crate::behavior_capability!(
                        @prop $cap, $dirty, $(#[$field_meta])* $prop : $ty => $field $([$kind])?
                    );
                )*

                $($crate::behavior_capability!(@whole $cap, $dirty, $whole);)?
            }
        }

        $vis use $module::$cap;
    };

    (@capability $(#[$cap_meta:meta])* $cap:ident, $dirty:tt) => {
        $(#[$cap_meta])*
        pub struct $cap;

        impl $crate::property::Capability for $cap {
            type Storage = Storage;
        }

        impl $crate::capability::HostCapability for $cap {
            const DIRTY: $crate::__interface::WidgetUpdateFlags =
                $crate::behavior_capability!(@flags $dirty);
        }
    };

    (@value $ty:ty) => { $ty };
    (@value $ty:ty [exact]) => { $ty };
    (@value $ty:ty [optional]) => { ::core::option::Option<$ty> };

    (@flags ($($flag:ident)|+)) => {
        $($crate::__interface::WidgetUpdateFlags::$flag)|+
    };

    (
        @prop $cap:ident, $dirty:tt,
        $(#[$field_meta:meta])* $prop:ident : $ty:ty => $field:ident $([$kind:ident])?
    ) => {
        $(#[$field_meta])*
        pub enum $prop {}

        impl $crate::property::Property for $prop {
            type Scope = $crate::property::CapScope<$cap>;
            type Value = $crate::behavior_capability!(@value $ty $([$kind])?);

            fn set(storage: &mut Storage, value: Self::Value) {
                storage.$field = value;
            }
        }

        impl $crate::property::FieldProperty for $prop {
            fn get(storage: &Storage) -> &Self::Value {
                &storage.$field
            }

            fn same(a: &Self::Value, b: &Self::Value) -> bool {
                a == b
            }
        }

        impl $crate::property::HostProperty for $prop {
            fn dirty(_: &Self::Value, _: &Self::Value) -> $crate::__interface::WidgetUpdateFlags {
                $crate::behavior_capability!(@flags $dirty)
            }
        }

        $crate::behavior_capability!(@accepts $prop : $ty $([$kind])?);
    };

    (@accepts $prop:ident : $ty:ty) => {
        impl<T: ::core::convert::Into<$ty>> $crate::property::Accepts<T> for $prop {
            fn accept(value: T) -> $ty {
                value.into()
            }
        }
    };
    (@accepts $prop:ident : $ty:ty [exact]) => {
        impl $crate::property::Accepts<$ty> for $prop {
            fn accept(value: $ty) -> $ty {
                value
            }
        }
    };
    (@accepts $prop:ident : $ty:ty [optional]) => {
        impl<T: $crate::property::OptionalValue<$ty>> $crate::property::Accepts<T> for $prop {
            fn accept(value: T) -> ::core::option::Option<$ty> {
                value.into_option()
            }
        }
    };

    (@whole $cap:ident, $dirty:tt, $whole:ident) => {
        #[doc = concat!("All of [`", stringify!($cap), "`] at once.")]
        pub enum $whole {}

        impl $crate::property::Property for $whole {
            type Scope = $crate::property::CapScope<$cap>;
            type Value = Storage;

            fn set(storage: &mut Storage, value: Storage) {
                *storage = value;
            }
        }

        impl $crate::property::FieldProperty for $whole {
            fn get(storage: &Storage) -> &Storage {
                storage
            }

            fn same(a: &Storage, b: &Storage) -> bool {
                a == b
            }
        }

        impl $crate::property::HostProperty for $whole {
            fn dirty(_: &Storage, _: &Storage) -> $crate::__interface::WidgetUpdateFlags {
                $crate::behavior_capability!(@flags $dirty)
            }
        }

        impl $crate::property::Accepts<Storage> for $whole {
            fn accept(value: Storage) -> Storage {
                value
            }
        }
    };
}
