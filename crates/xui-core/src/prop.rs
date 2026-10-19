//! Storage for component props, and zero-copy forwarding of one component's
//! props to another.
//!
//! Every field of a generated `{Name}Props` struct is a [`Prop<T>`]. A prop
//! normally holds its value inline, but it can instead *project* into a field
//! of an ancestor's props: it keeps that ancestor's props allocation alive and
//! reads the field through it, so the value itself is never copied.
//!
//! A component forwards a prop it received by calling [`Share::share`] on the
//! binding:
//!
//! ```ignore
//! #[component]
//! fn tabs(items: &Vec<TabItem>, label: &str) {
//!     xui! { <tab_list items={items.share()} label={label.share()} /> }
//! }
//! ```
//!
//! `share()` recognises the binding by address among the props of the
//! component currently rendering, so it only projects a value that really is
//! one of those props; anything else -- a local, a field of a prop, a value
//! outside any render -- is cloned instead, exactly as `.clone()` would. A
//! forwarded prop that is itself a projection is forwarded as the projection it
//! already is, so a value passed down any number of levels still points
//! straight at the component that first received it.
//!
//! A small value that owns nothing (no drop glue, no larger than a projection)
//! is copied even when it is a prop: hook handles such as `State` and `Memo`
//! are already just a slot pointer, so a projection would only add an
//! indirection to every read. Handles that hold an `Rc`, like `Callback`, are
//! projected like any other prop.
//!
//! Projections cannot form cycles: a props allocation is immutable once built,
//! and only elements built *while its component renders* can point into it --
//! none of which it can contain.
//!
//! Plain `items={items.clone()}` keeps working; `share()` is how a forward
//! avoids the copy. It has to be spelled out because a setter taking both
//! `impl Into<T>` and a forwarded `&T` would be ambiguous for every type with a
//! `From<&T>` -- `String` and `ElementDesc` among them.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::ops::Deref;

use crate::fiber::ErasedProps;

/// Reads one field of a props struct as `&Prop<T>`, given the whole struct.
#[doc(hidden)]
pub type PropGetter = fn(&dyn Any) -> &dyn Any;

/// The fields of a generated props struct, in a form [`Share::share`] can
/// search. Implemented by `#[component]`.
#[doc(hidden)]
pub trait PropFields: 'static {
    const FIELDS: &'static [PropGetter];
}

/// One field of a component's props: either the value itself, or a
/// projection into a field of an ancestor's props. Dereferences to `T` either
/// way.
pub struct Prop<T: 'static>(Repr<T>);

enum Repr<T: 'static> {
    Inline(T),
    Projected {
        owner: ErasedProps,
        field: PropGetter,
    },
}

impl<T: 'static> Prop<T> {
    pub fn new(value: T) -> Self {
        Self(Repr::Inline(value))
    }

    /// Whether this prop reads through another component's props rather than
    /// holding its value.
    pub fn is_projected(&self) -> bool {
        matches!(self.0, Repr::Projected { .. })
    }

    /// A projection onto this prop, which `owner` is the props of. A prop that
    /// is already a projection is shared as it is, so chains never form.
    fn project_from(&self, owner: &ErasedProps, field: PropGetter) -> Self {
        match &self.0 {
            Repr::Inline(_) => Self(Repr::Projected {
                owner: owner.clone(),
                field,
            }),
            Repr::Projected { owner, field } => Self(Repr::Projected {
                owner: owner.clone(),
                field: *field,
            }),
        }
    }
}

impl<T: 'static> Deref for Prop<T> {
    type Target = T;

    fn deref(&self) -> &T {
        match &self.0 {
            Repr::Inline(value) => value,
            Repr::Projected { owner, field } => field(&**owner)
                .downcast_ref::<Prop<T>>()
                .expect("projected prop changed type")
                .deref(),
        }
    }
}

impl<T: 'static> From<T> for Prop<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Clone + 'static> Clone for Prop<T> {
    fn clone(&self) -> Self {
        match &self.0 {
            Repr::Inline(value) => Self::new(value.clone()),
            Repr::Projected { owner, field } => Self(Repr::Projected {
                owner: owner.clone(),
                field: *field,
            }),
        }
    }
}

impl<T: Default + 'static> Default for Prop<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Prop<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: PartialEq<U> + 'static, U> PartialEq<U> for Prop<T> {
    fn eq(&self, other: &U) -> bool {
        **self == *other
    }
}

/// A prop binding marked for forwarding; see [`Share::share`].
pub struct Shared<'a, T: ?Sized>(&'a T);

/// Forwards a prop to a child component without copying it.
///
/// Implemented for every type; see the [module docs](self).
pub trait Share {
    /// Hands this prop to a child component's prop of the same type. When it
    /// is one of the rendering component's own props, the child reads it
    /// through them; otherwise it is cloned.
    fn share(&self) -> Shared<'_, Self> {
        Shared(self)
    }
}

impl<T: ?Sized> Share for T {}

/// What a component prop setter accepts: anything `Into` the prop's type, or
/// a [`Shared`] prop of that type.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as a `{T}` prop",
    label = "expected something convertible into `{T}`",
    note = "a prop takes any value that is `Into<{T}>`, or `.share()` of a `&{T}`"
)]
pub trait IntoProp<T: 'static, Via> {
    fn into_prop(self) -> Prop<T>;
}

#[doc(hidden)]
pub struct ViaInto;

#[doc(hidden)]
pub struct ViaShare;

impl<T: 'static, U: Into<T>> IntoProp<T, ViaInto> for U {
    fn into_prop(self) -> Prop<T> {
        Prop::new(self.into())
    }
}

impl<T: Clone + 'static> IntoProp<T, ViaShare> for Shared<'_, T> {
    fn into_prop(self) -> Prop<T> {
        if copies_inline::<T>() {
            return Prop::new(self.0.clone());
        }
        project::<T>(|value| std::ptr::eq(value, self.0))
            .unwrap_or_else(|| Prop::new(self.0.clone()))
    }
}

/// Sharing a props field directly, rather than the binding `#[component]` gives
/// the body.
impl<T: Clone + 'static> IntoProp<T, ViaShare> for Shared<'_, Prop<T>> {
    fn into_prop(self) -> Prop<T> {
        if copies_inline::<T>() {
            return Prop::new((**self.0).clone());
        }
        project::<T>(|value| std::ptr::eq(value, &**self.0)).unwrap_or_else(|| self.0.clone())
    }
}

/// Whether sharing a `T` copies it instead of projecting onto it.
///
/// A value without drop glue owns nothing -- no heap buffer, no reference
/// count -- so when it is no larger than the projection itself, copying it
/// costs no more than building the projection, and reading it then skips the
/// indirection and leaves the owner's props free. Hook handles (`State`,
/// `Memo`, `HookRef`), ids and plain numbers are all such values.
const fn copies_inline<T>() -> bool {
    !std::mem::needs_drop::<T>()
        && std::mem::size_of::<T>() <= std::mem::size_of::<(ErasedProps, PropGetter)>()
}

/// A `&str` binding is how `#[component]` hands out a `&str` prop, which is
/// stored as a `String`.
impl IntoProp<String, ViaShare> for Shared<'_, str> {
    fn into_prop(self) -> Prop<String> {
        project::<String>(|value| std::ptr::eq(value.as_str(), self.0))
            .unwrap_or_else(|| Prop::new(self.0.to_owned()))
    }
}

/// For a setter whose type has to stay inferable, which `IntoProp`'s blanket
/// `Into` would leave ambiguous: the setters of fields naming a component's
/// type parameter take `impl Into<Prop<T>>`.
impl<T: Clone + 'static> From<Shared<'_, T>> for Prop<T> {
    fn from(shared: Shared<'_, T>) -> Self {
        shared.into_prop()
    }
}

struct Rendering {
    owner: ErasedProps,
    fields: &'static [PropGetter],
}

thread_local! {
    static RENDERING: RefCell<Option<Rendering>> = const { RefCell::new(None) };
}

/// Projects onto the rendering component's prop of type `T` that `is` picks
/// out, if any.
fn project<T: 'static>(is: impl Fn(&T) -> bool) -> Option<Prop<T>> {
    RENDERING.with_borrow(|rendering| {
        let rendering = rendering.as_ref()?;
        rendering.fields.iter().find_map(|&field| {
            let prop = field(&*rendering.owner).downcast_ref::<Prop<T>>()?;
            is(prop).then(|| prop.project_from(&rendering.owner, field))
        })
    })
}

/// Runs a component's render with `owner` as the props [`Share::share`]
/// projects into. Called by the code `#[component]` generates.
#[doc(hidden)]
pub fn render_with_props<P: PropFields, R>(owner: &ErasedProps, render: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rendering>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RENDERING.set(self.0.take());
        }
    }

    let previous = RENDERING.replace(Some(Rendering {
        owner: owner.clone(),
        fields: P::FIELDS,
    }));
    let _restore = Restore(previous);
    render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    struct ParentProps {
        items: Prop<Vec<u32>>,
        label: Prop<String>,
    }

    impl PropFields for ParentProps {
        const FIELDS: &'static [PropGetter] = &[
            |props| &props.downcast_ref::<Self>().unwrap().items,
            |props| &props.downcast_ref::<Self>().unwrap().label,
        ];
    }

    struct ChildProps {
        items: Prop<Vec<u32>>,
        label: Prop<String>,
    }

    impl PropFields for ChildProps {
        const FIELDS: &'static [PropGetter] = &[
            |props| &props.downcast_ref::<Self>().unwrap().items,
            |props| &props.downcast_ref::<Self>().unwrap().label,
        ];
    }

    fn into_prop<T: 'static, Via>(value: impl IntoProp<T, Via>) -> Prop<T> {
        value.into_prop()
    }

    fn parent() -> ErasedProps {
        Rc::new(ParentProps {
            items: Prop::new(vec![1, 2, 3]),
            label: Prop::new("label".to_string()),
        })
    }

    #[test]
    fn sharing_a_prop_reads_it_through_the_owner() {
        let owner = parent();
        let props = owner.downcast_ref::<ParentProps>().unwrap();
        let child = render_with_props::<ParentProps, _>(&owner, || ChildProps {
            items: into_prop(props.items.share()),
            label: into_prop(props.label.as_str().share()),
        });

        assert!(child.items.is_projected());
        assert!(child.label.is_projected());
        assert!(std::ptr::eq(&*child.items, &*props.items));
        assert_eq!(child.label, "label");
        assert_eq!(Rc::strong_count(&owner), 3);
    }

    #[test]
    fn a_forwarded_projection_points_at_the_original_owner() {
        let owner = parent();
        let props = owner.downcast_ref::<ParentProps>().unwrap();
        let child: ErasedProps = render_with_props::<ParentProps, _>(&owner, || {
            Rc::new(ChildProps {
                items: into_prop(props.items.share()),
                label: into_prop(props.label.share()),
            })
        });
        let child_props = child.downcast_ref::<ChildProps>().unwrap();
        let grandchild = render_with_props::<ChildProps, _>(&child, || {
            into_prop::<Vec<u32>, _>(child_props.items.share())
        });

        assert!(std::ptr::eq(&*grandchild, &*props.items));
        // Only the projections hold the child's props, not the grandchild's.
        assert_eq!(Rc::strong_count(&child), 1);
    }

    struct PlainProps {
        count: Prop<u32>,
        wide: Prop<[u64; 8]>,
    }

    impl PropFields for PlainProps {
        const FIELDS: &'static [PropGetter] = &[
            |props| &props.downcast_ref::<Self>().unwrap().count,
            |props| &props.downcast_ref::<Self>().unwrap().wide,
        ];
    }

    #[test]
    fn a_small_value_that_owns_nothing_is_copied_not_projected() {
        let owner: ErasedProps = Rc::new(PlainProps {
            count: Prop::new(7),
            wide: Prop::new([1; 8]),
        });
        let props = owner.downcast_ref::<PlainProps>().unwrap();
        let (count, from_field, wide) = render_with_props::<PlainProps, _>(&owner, || {
            (
                into_prop::<u32, _>((*props.count).share()),
                into_prop::<u32, _>(props.count.share()),
                into_prop::<[u64; 8], _>((*props.wide).share()),
            )
        });

        assert!(!count.is_projected() && count == 7);
        assert!(!from_field.is_projected() && from_field == 7);
        // Too large to copy for free: still read through the owner.
        assert!(wide.is_projected());
        assert_eq!(Rc::strong_count(&owner), 2);
    }

    #[test]
    fn sharing_anything_else_clones_it() {
        let owner = parent();
        let props = owner.downcast_ref::<ParentProps>().unwrap();
        let local = vec![9];
        let (from_local, from_part) = render_with_props::<ParentProps, _>(&owner, || {
            (
                into_prop::<Vec<u32>, _>(local.share()),
                into_prop::<String, _>(props.label[1..].share()),
            )
        });
        let outside = into_prop::<Vec<u32>, _>(props.items.share());

        assert!(!from_local.is_projected() && from_local == vec![9]);
        assert!(!from_part.is_projected() && from_part == "abel");
        assert!(!outside.is_projected());
    }
}
