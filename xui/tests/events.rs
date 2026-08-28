//! Handing an event handler to a component.
//!
//! This is what the previous design could not do. `TypedEventHandler` was a
//! bare `Box<dyn FnMut>`: not `Clone`, so a component — which only ever sees
//! `&Props` — could never move one out to attach it to a widget. `Callback`
//! could not stand in either, because its `Args` must be `'static` and an event
//! handler's arguments include `&mut EventContext<'a>`. Components were left
//! declaring a bespoke semantic payload per event and losing the context.

use std::cell::Cell;
use std::rc::Rc;

use xui::event_system::EventContext;
use xui::prelude::*;
use xui::{component, xui};

/// A component that takes a real host event handler and forwards it to the
/// widget it renders.
#[component]
#[defaults(label = String::new())]
fn clickable(label: &String, on_click: &Handler<ClickEvent>) {
    xui! {
        <container on_click={on_click.clone().into_fn()}>
            <text>{label.clone()}</text>
        </container>
    }
}

#[test]
fn a_component_accepts_a_host_event_handler_as_a_prop() {
    let handler = Handler::<ClickEvent>::new(|_, _| {});
    let element = xui! { <clickable label={String::from("go")} on_click={handler} /> };
    assert!(matches!(element, ElementDesc::Component(_)));
}

#[test]
fn a_forwarded_handler_keeps_the_context_the_old_callback_type_could_not_carry() {
    let seen_phase = Rc::new(Cell::new(None::<EventPhase>));
    let flag = Rc::clone(&seen_phase);

    // `Callback<Args, Output>` cannot express this signature at all: `Args`
    // would have to contain `&mut EventContext<'a>`, which is not `'static`.
    let handler = Handler::<ClickEvent>::new(move |_event, cx: &mut EventContext<'_>| {
        flag.set(Some(cx.phase));
        let _ = cx.node_id();
        Flow::empty()
    });

    // Attaching it twice is fine, and both attachments are the same handler.
    let clone = handler.clone();
    assert!(handler.ptr_eq(&clone));

    let element = xui! { <clickable label={String::from("go")} on_click={clone} /> };
    assert!(matches!(element, ElementDesc::Component(_)));
    assert_eq!(seen_phase.get(), None, "nothing has been dispatched yet");
}

/// A handler body may return `Flow`, the older `EventResult`, or nothing.
#[test]
fn handler_bodies_may_return_any_of_the_three_shapes() {
    let _unit = Handler::<ClickEvent>::new(|_, _| {});
    let _flow = Handler::<ClickEvent>::new(|_, _| Flow::CONSUME);
    let _legacy = Handler::<ClickEvent>::new(|_, _| EventResult::Consumed);
}

/// The two halves of the old `Consumed` are now separable.
#[test]
fn stopping_propagation_and_preventing_the_default_are_independent() {
    assert!(Flow::STOP_PROPAGATION.stops_propagation());
    assert!(!Flow::STOP_PROPAGATION.prevents_default());

    assert!(Flow::PREVENT_DEFAULT.prevents_default());
    assert!(!Flow::PREVENT_DEFAULT.stops_propagation());

    assert!(Flow::CONSUME.stops_propagation() && Flow::CONSUME.prevents_default());
}
