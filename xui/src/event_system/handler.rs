//! A shareable event handler with an identity.

use std::cell::RefCell;
use std::rc::Rc;

use super::{EventContext, Flow};

type HandlerFn<E> = dyn for<'a> FnMut(&E, &mut EventContext<'a>) -> Flow;

/// A handler for events of type `E`.
///
/// The previous representation was a bare
/// `Box<dyn for<'a> FnMut(&E, &mut EventContext<'a>) -> EventResult>`, which
/// could not be cloned or compared. That made three things impossible:
///
/// - **crossing a component boundary** — a component receives `&Props`, so it
///   could never move a boxed handler out to attach it to a widget. Components
///   had to invent a semantic `Callback<Payload, ()>` prop per event and adapt
///   internally, losing [`EventContext`] in the process.
/// - **skipping unchanged work** — with no identity, every render replaced every
///   handler wholesale.
/// - **reuse** — one handler could not be attached to two widgets.
///
/// `Callback<Args, Output>` cannot fill the gap: its `Args` must be `'static`,
/// and `(&E, &mut EventContext<'a>)` is not. Here the higher-ranked lifetime
/// lives *inside* the trait object, so the context survives intact.
pub struct Handler<E: 'static> {
    inner: Rc<RefCell<HandlerFn<E>>>,
}

impl<E: 'static> Handler<E> {
    /// A handler body may return [`Flow`], the older `EventResult`, or `()`.
    pub fn new<R: Into<Flow>>(
        mut handler: impl for<'a> FnMut(&E, &mut EventContext<'a>) -> R + 'static,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(move |event: &E, cx: &mut EventContext<'_>| {
                handler(event, cx).into()
            })),
        }
    }

    /// Runs the handler.
    ///
    /// A handler that is already running — reached again through an event it
    /// dispatched itself — is skipped rather than panicking. The old
    /// `Box<dyn FnMut>` made that case a borrow-checker error at compile time;
    /// sharing moves the check to runtime, so re-entrancy needs a defined
    /// answer, and "the outer call wins" is the one that cannot deadlock or
    /// abort a frame.
    pub fn call(&self, event: &E, cx: &mut EventContext<'_>) -> Flow {
        match self.inner.try_borrow_mut() {
            Ok(mut handler) => handler(event, cx),
            Err(_) => {
                debug_assert!(
                    false,
                    "event handler re-entered while running; the inner call was skipped"
                );
                Flow::empty()
            }
        }
    }

    /// Identity, not behaviour: two handlers are the same handler only when they
    /// are the same allocation.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Views a shared handler as a closure, for attaching it to a widget.
    ///
    /// The `on_*` builders take a closure bound directly rather than
    /// `impl Into<Handler<_>>`, because a closure only infers its argument types
    /// — and, critically, its higher-ranked lifetime — from an `Fn` bound
    /// written in the signature it is passed to. Behind an `Into`, every
    /// `|event, cx| ..` at every call site would need explicit annotations.
    /// Forwarding a ready-made handler is the rarer case, so it pays the
    /// adapter instead.
    pub fn into_fn(self) -> impl for<'a> FnMut(&E, &mut EventContext<'a>) -> Flow + 'static {
        move |event, cx| self.call(event, cx)
    }
}

impl<E: 'static> Clone for Handler<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<E: 'static> PartialEq for Handler<E> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
    }
}

impl<E: 'static> std::fmt::Debug for Handler<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handler")
            .field("type", &std::any::type_name::<E>())
            .finish()
    }
}

/// Lets a bare closure be passed wherever a `Handler` is expected, and lets that
/// closure return `Flow`, the older `EventResult`, or `()` — anything that
/// converts into a [`Flow`].
impl<E, F, R> From<F> for Handler<E>
where
    E: 'static,
    F: for<'a> FnMut(&E, &mut EventContext<'a>) -> R + 'static,
    R: Into<Flow>,
{
    fn from(handler: F) -> Self {
        Handler::new(handler)
    }
}
