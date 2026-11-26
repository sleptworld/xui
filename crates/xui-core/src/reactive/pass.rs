//! What a parent hands a child besides values: callbacks, and its own
//! content as children.
use std::rc::Rc;

use super::block::{Row, RowTemplate};
use super::graph::InstanceId;
use super::runtime::{Cx, Owner, Runtime};
use super::template::Scope;
use crate::windows::Windows;

/// A function of the parent's a child calls, with the `Cx` it was given: a
/// prop typed `&Callback<A>`, which a parent sets with a closure
/// (`on_change: |cx, value| ..`). Unset, calling it does nothing.
pub struct Callback<A>(Option<Rc<dyn Fn(&mut Cx<'_>, A)>>);

impl<A> Callback<A> {
    pub fn new(f: impl Fn(&mut Cx<'_>, A) + 'static) -> Self {
        Self(Some(Rc::new(f)))
    }

    /// A callback that does nothing: an event prop left unset.
    pub fn noop() -> Self {
        Self(None)
    }

    pub fn call(&self, cx: &mut Cx<'_>, value: A) {
        if let Some(f) = &self.0 {
            f(cx, value);
        }
    }

    pub fn is_set(&self) -> bool {
        self.0.is_some()
    }
}

impl<A> Clone for Callback<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A> Default for Callback<A> {
    fn default() -> Self {
        Self::noop()
    }
}

impl<A> From<&Callback<A>> for Callback<A> {
    fn from(callback: &Callback<A>) -> Self {
        callback.clone()
    }
}

/// The same callback, not an equal one: closures do not compare.
impl<A> PartialEq for Callback<A> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<A> std::fmt::Debug for Callback<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_set() {
            "Callback(..)"
        } else {
            "Callback(none)"
        })
    }
}

/// What a callback prop holds, whatever its type is called: what a
/// parent's closure takes.
pub trait CallbackArg {
    type Arg;
    fn from_fn(f: impl Fn(&mut Cx<'_>, Self::Arg) + 'static) -> Self;
}

impl<A> CallbackArg for Callback<A> {
    type Arg = A;
    fn from_fn(f: impl Fn(&mut Cx<'_>, A) + 'static) -> Self {
        Self::new(f)
    }
}

/// A parent's closure for callback prop `P`, inferring its argument types
/// from the prop's.
#[doc(hidden)]
pub fn __callback<P: super::PropKind>(
    _: crate::property::PropertyMarker<P>,
    f: impl Fn(&mut Cx<'_>, <P::Item as CallbackArg>::Arg) + 'static,
) -> super::Expr<P::Item>
where
    P::Item: CallbackArg + Clone,
{
    let callback = P::Item::from_fn(f);
    super::props::__expr::<P>(
        crate::property::PropertyMarker::new(),
        Vec::new(),
        move || callback.clone(),
    )
}

type MountChildren = dyn Fn(&mut Runtime, &mut Windows, Owner) -> InstanceId;

/// A parent's content, given to a child as its `children` prop and mounted
/// where the child writes `slot(children)`. Its bindings belong to the
/// parent: they read the parent's sources, and the child never reruns for
/// them. Empty when the parent gave none.
#[derive(Clone, Default)]
pub struct Children(Option<Rc<MountChildren>>);

impl Children {
    pub fn empty() -> Self {
        Self(None)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub(crate) fn mount(
        &self,
        runtime: &mut Runtime,
        windows: &mut Windows,
        owner: Owner,
    ) -> Option<InstanceId> {
        self.0.as_ref().map(|mount| mount(runtime, windows, owner))
    }

    /// Component `C` with `props`, mounted wherever this is slotted, as
    /// many times as it is: content built away from any view, such as a
    /// tab's panel.
    pub fn component<C: super::Component>(
        props: <C::Instance as super::ComponentDef>::Props,
    ) -> Self
    where
        <C::Instance as super::ComponentDef>::Props: Clone,
    {
        Self(Some(Rc::new(
            move |runtime: &mut Runtime, windows: &mut Windows, owner| {
                runtime.mount_into::<C::Instance>(windows, props.clone(), super::Place(owner))
            },
        )))
    }
}

/// The same content, not an equal one.
impl PartialEq for Children {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Children {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_empty() {
            "Children(none)"
        } else {
            "Children(..)"
        })
    }
}

impl From<&Children> for Children {
    fn from(children: &Children) -> Self {
        children.clone()
    }
}

/// Text where content goes.
impl From<String> for Children {
    fn from(text: String) -> Self {
        Self::component::<TextContent>(TextContentProps { value: text })
    }
}

impl From<&str> for Children {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

use crate::widgets::text::text;

#[xui_macros::component]
fn text_content(value: &String) {
    view! { text(value.clone()) }
}

/// The content written after a component's tag, as a row of the scope it
/// is written in.
#[doc(hidden)]
pub fn __children<P: Scope>(parent: &Rc<P>, template: RowTemplate<'_, Row<P, ()>>) -> Children {
    let (parent, parts) = (parent.clone(), Rc::new(template.to_parts()));
    Children(Some(Rc::new(
        move |runtime: &mut Runtime, windows: &mut Windows, owner| {
            runtime.mount_row(windows, parent.clone(), (), parts.clone(), owner)
        },
    )))
}

/// A `portal { .. }`: its content, mounted under the overlayer as an
/// overlay entry, and what the entry does. Built where the portal is
/// written, once, at mount; attributes are its builder methods.
pub struct PortalSpec {
    pub(crate) content: Children,
    pub(crate) scope: Option<crate::widgets::OverlayScopeId>,
    pub(crate) options: crate::widgets::OverlayEntryOptions,
    pub(crate) anchor: Option<crate::anchor::AnchorPlacement>,
    pub(crate) on_dismiss: Option<crate::widgets::DismissHandler>,
}

impl PortalSpec {
    pub fn new(content: Children) -> Self {
        Self {
            content,
            scope: None,
            options: Default::default(),
            anchor: None,
            on_dismiss: None,
        }
    }

    pub fn scope(mut self, scope: crate::widgets::OverlayScopeId) -> Self {
        self.scope = Some(scope);
        self
    }

    pub fn z_index(mut self, z_index: i32) -> Self {
        self.options.z_index = z_index;
        self
    }

    pub fn hit_test(mut self, hit_test: bool) -> Self {
        self.options.hit_test = hit_test;
        self
    }

    pub fn modal(mut self, modal: bool) -> Self {
        self.options.modal = modal;
        self
    }

    /// Places the content's root next to the host the portal is written
    /// under; see `PortalDesc::anchor`.
    pub fn anchor(mut self, placement: crate::anchor::AnchorPlacement) -> Self {
        self.anchor = Some(placement);
        self
    }

    /// `on_dismiss: |cx, reason| ..`: the portal should close. It is not
    /// removed: the handler closes it by writing what shows it.
    #[doc(hidden)]
    pub fn __on_dismiss(
        mut self,
        notifier: &super::Notifier,
        f: impl Fn(&mut Cx<'_>, crate::widgets::DismissReason) + 'static,
    ) -> Self {
        let notifier = notifier.clone();
        self.on_dismiss = Some(crate::widgets::DismissHandler::new(move |reason| {
            f(&mut Cx::handler(&notifier), reason)
        }));
        self
    }
}

/// A `window(..) { .. }`: its content, mounted as the tree of a window of
/// its own, and the window asked for. Built where the block is written,
/// once, at mount; attributes are its builder methods. The window is there
/// while the block is mounted: a host's close button only asks
/// (`on_close_requested`), and the handler closes it by writing what shows it.
pub struct WindowSpec {
    pub(crate) content: Children,
    pub(crate) options: xui_interface::WindowOptions,
    pub(crate) on_close_requested: Option<std::rc::Rc<dyn Fn()>>,
}

impl WindowSpec {
    pub fn new(content: Children) -> Self {
        Self {
            content,
            options: xui_interface::WindowOptions::default(),
            on_close_requested: None,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.options.title = title.into();
        self
    }

    /// The content area's size, `(width, height)` in logical pixels.
    pub fn size(mut self, (width, height): (f32, f32)) -> Self {
        self.options.inner_size = Some(crate::core::Size::new(width, height).into());
        self
    }

    /// The smallest the content area may be resized to, in logical pixels.
    pub fn min_size(mut self, (width, height): (f32, f32)) -> Self {
        self.options.min_inner_size = Some(crate::core::Size::new(width, height).into());
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.options.resizable = resizable;
        self
    }

    pub fn decorations(mut self, decorations: bool) -> Self {
        self.options.decorations = decorations;
        self
    }

    pub fn transparent(mut self, transparent: bool) -> Self {
        self.options.transparent = transparent;
        self
    }

    pub fn maximized(mut self, maximized: bool) -> Self {
        self.options.maximized = maximized;
        self
    }

    pub fn macos(mut self, macos: xui_interface::MacOsWindowOptions) -> Self {
        self.options.macos = macos;
        self
    }

    /// `on_close_requested: |cx| ..`: the user asked to close the window.
    /// It is not closed: the handler closes it by writing what shows it.
    #[doc(hidden)]
    pub fn __on_close_requested(
        mut self,
        notifier: &super::Notifier,
        f: impl Fn(&mut Cx<'_>) + 'static,
    ) -> Self {
        let notifier = notifier.clone();
        self.on_close_requested = Some(std::rc::Rc::new(move || f(&mut Cx::handler(&notifier))));
        self
    }
}

/// The observer that binds a host to a `size!()` source, given to its
/// `size_observer` attribute.
#[doc(hidden)]
pub fn __size_observer(
    size: &super::Signal<Option<crate::core::Size<f32>>>,
    notifier: &super::Notifier,
) -> crate::size::SizeObserver {
    crate::size::SizeObserver::reactive(size.clone(), notifier.clone())
}
