//! Input dispatch, in two layers.
//!
//! # Raw and semantic
//!
//! A platform event is dispatched twice, but not as the same event twice — the
//! two layers describe different things and resolve their targets differently.
//!
//! | | raw | semantic |
//! |---|---|---|
//! | what it is | a device fact: the pointer moved, a key went down | an intent: click, press, drag, hover, focus |
//! | who produces it | the platform | [`translator::EventTranslator`], from the raw stream plus its own gesture state |
//! | how many | one in | zero or more out, each with its own target |
//! | target | pointer capture, else hit test, else the focused node, else the root | carried on the event, computed by the translator |
//! | propagation | always capture → target → bubble | whatever the event declares; hover is `Direct`, click bubbles |
//! | who receives it | **widgets only** | user handlers *and* widgets |
//!
//! One `PointerUp` can become `PressEnd` and `Click` and `DoubleClick`, aimed at
//! different nodes; one `PointerMove` can become a `Hovered { hovered: false }`
//! on each node being left, a `Hovered { hovered: true }` on each node being
//! entered, and a `DragMove` on the node the gesture started from. No single
//! traversal could deliver that, which is why the translator sits between the
//! layers.
//!
//! # Raw events are widget-private
//!
//! There is no way to register a user handler for a raw event, and that is
//! deliberate: raw events carry device detail (which physical key, exact device
//! coordinates, platform IME phases) that should not leak into application code.
//! Widgets consume them to implement behaviour — text selection, IME, key
//! editing — and publish the *result* as semantic events, which are the stable
//! contract. `EventProps` therefore covers the semantic vocabulary only.
//!
//! The practical consequence is that raw dispatch has no listeners at all on
//! most trees, so it starts by asking the runtime whether any live widget reads
//! raw events and does nothing when none does.
//!
//! # Order within one platform event
//!
//! ```text
//! dispatch_event_pipeline(raw)
//!   1. dispatch_raw(raw)              widget built-ins, whole path
//!   2. drain_focus_requests()         step 1 may have asked for focus
//!   3. translate(raw) -> [semantic]
//!        for each: dispatch_semantic(event)
//!   4. drain_focus_requests()         step 3's user handlers may have too
//! ```
//!
//! Focus is drained twice because a focus change has to become real — and emit
//! its own `Focus`/`Blur`/`FocusIn`/`FocusOut` events — within the same platform
//! event that caused it, and the request can come from either layer.
//!
//! At each node a semantic event visits, the order is:
//!
//! 1. `apply_semantic_state` — at the target only, updates `WidgetState`
//! 2. user handlers — the catch-all `on_event`, then the kind-specific one
//! 3. the widget's own handling — unless a handler returned
//!    [`Flow::PREVENT_DEFAULT`], and by default only at the target
//!
//! Handlers run *before* the widget so that preventing the default action has
//! something left to prevent.
//!
//! # One pointer capture
//!
//! [`EventState::pointer_capture`] is the single source of truth for where a
//! captured pointer aims, read by both layers. Capture and the translator's
//! gesture bookkeeping (`active_presses`, `active_drags`) answer different
//! questions and both are needed: capture redirects events that would otherwise
//! hit-test elsewhere, while the gesture state remembers which node a press or
//! drag belongs to.

pub mod callbacks;
pub mod dispatcher;
pub mod flow;
pub mod handler;
pub mod interaction;
pub mod translator;

pub use flow::Flow;
pub use handler::Handler;

use crate::event_system::dispatcher::DispatchReport;
use crate::event_system::translator::EventTranslator;
use crate::text::{TextHost, TextLayoutQuery};
use crate::ui_runtime::NodeView;
use crate::ui_runtime::UiRuntime;
use xui_interface::events::{EventResult, RawEvent};
use xui_interface::{
    EventPhase, EventRequest, EventRequests, NodeId, TextBackend, WidgetUpdateFlags,
};

/// Input state the runtime owns, as opposed to the gesture state the
/// translator owns.
///
/// Hover used to live here too, as a field that nothing ever assigned and
/// nothing ever read: the real hover state is `WidgetState::HOVERED` on the node
/// and `EventTranslator::hover_paths` for the enter/leave bookkeeping.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventState {
    pointer_capture: Option<NodeId>,
}

impl EventState {
    /// The node that currently receives pointer events regardless of hit
    /// testing. Both dispatch layers resolve their target through this, so a
    /// captured pointer cannot send raw events to one node and semantic events
    /// to another.
    pub fn pointer_capture(&self) -> Option<NodeId> {
        self.pointer_capture
    }

    pub(crate) fn clear_node(&mut self, id: NodeId) {
        if self.pointer_capture == Some(id) {
            self.pointer_capture = None;
        }
    }

    pub(crate) fn capture_pointer(&mut self, id: NodeId) {
        self.pointer_capture = Some(id);
    }

    pub(crate) fn release_pointer_capture(&mut self) {
        self.pointer_capture = None;
    }
}

pub struct EventContext<'a> {
    pub phase: EventPhase,
    pub node_ref: NodeView<'a>,
    text_layout: Option<&'a dyn TextLayoutQuery>,
    request_update: &'a mut WidgetUpdateFlags,
    requests: &'a mut EventRequests,
}

impl<'a> EventContext<'a> {
    pub fn new(
        node_ref: NodeView<'a>,
        text_layout: Option<&'a dyn TextLayoutQuery>,
        phase: EventPhase,
        request_update: &'a mut WidgetUpdateFlags,
        requests: &'a mut EventRequests,
    ) -> Self {
        Self {
            node_ref,
            text_layout,
            phase,
            request_update,
            requests,
        }
    }

    pub fn node_id(&self) -> NodeId {
        self.node_ref.id
    }

    pub fn text_layout(&self) -> Option<&'a dyn TextLayoutQuery> {
        self.text_layout
    }

    pub fn mark_needs_style(&mut self) {
        *self.request_update |= WidgetUpdateFlags::STYLE_TARGET;
    }

    pub fn mark_needs_layout(&mut self) {
        *self.request_update |= WidgetUpdateFlags::LAYOUT_INPUT;
    }

    pub fn mark_needs_paint(&mut self) {
        *self.request_update |= WidgetUpdateFlags::PAINT_OUTPUT;
    }

    pub fn mark_needs_text_shape(&mut self) {
        *self.request_update |= WidgetUpdateFlags::TEXT_SHAPE;
    }

    pub fn invalidate(&mut self, flags: WidgetUpdateFlags) {
        *self.request_update |= flags;
    }

    pub fn request_focus(&mut self) {
        self.request_focus_node(self.node_id());
    }

    pub(crate) fn request_focus_node(&mut self, node: NodeId) {
        self.requests.push(EventRequest::Focus(node));
    }

    pub fn clear_focus(&mut self) {
        self.requests.push(EventRequest::ClearFocus);
    }

    pub fn capture_pointer(&mut self) {
        self.requests
            .push(EventRequest::CapturePointer(self.node_id()));
    }

    pub fn release_pointer_capture(&mut self) {
        self.requests.push(EventRequest::ReleasePointerCapture);
    }
}

#[inline(always)]
pub fn dispatch_event<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    event_translator: &mut EventTranslator,
    event: RawEvent,
) -> EventResult {
    dispatch_event_pipeline(arena, host_text_cache, event_translator, event).result()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventDispatchReport {
    pub raw: DispatchReport,
    pub semantic: Vec<DispatchReport>,
}

impl EventDispatchReport {
    pub fn result(&self) -> EventResult {
        if self.raw.result.is_consumed()
            || self
                .semantic
                .iter()
                .any(|report| report.result.is_consumed())
        {
            EventResult::Consumed
        } else {
            EventResult::Ignored
        }
    }
}

pub fn dispatch_event_pipeline<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    translator: &mut EventTranslator,
    event: RawEvent,
) -> EventDispatchReport {
    let (timestamp, modifiers) = raw_event_context(&event);
    let raw = dispatcher::dispatch_raw(arena, host_text_cache, event.clone());
    let mut semantic = Vec::new();

    drain_focus_requests(
        arena,
        host_text_cache,
        translator,
        timestamp,
        modifiers,
        &mut semantic,
    );

    for event in translator.translate_raw_event(&event, arena) {
        semantic.push(dispatcher::dispatch_semantic(arena, host_text_cache, event));
    }

    drain_focus_requests(
        arena,
        host_text_cache,
        translator,
        timestamp,
        modifiers,
        &mut semantic,
    );

    EventDispatchReport { raw, semantic }
}

fn drain_focus_requests<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    translator: &mut EventTranslator,
    timestamp: std::time::Instant,
    modifiers: xui_interface::Modifiers,
    reports: &mut Vec<DispatchReport>,
) {
    while let Some(request) = arena.focus_manager_mut().take_request() {
        let events = translator.apply_focus_request(arena, request, timestamp, modifiers);
        for event in events {
            reports.push(dispatcher::dispatch_semantic(arena, host_text_cache, event));
        }
    }
}

fn raw_event_context(event: &RawEvent) -> (std::time::Instant, xui_interface::Modifiers) {
    match event {
        RawEvent::PointerMove(event) => (event.timestamp, event.modifiers),
        RawEvent::PointerDown(event) | RawEvent::PointerUp(event) => {
            (event.timestamp, event.modifiers)
        }
        RawEvent::PointerCancel(event) => (event.timestamp, event.modifiers),
        RawEvent::Wheel(event) => (event.timestamp, event.modifiers),
        RawEvent::Keyboard(event) => (event.timestamp, event.modifiers),
        RawEvent::WindowBlur(event) | RawEvent::WindowFocus(event) => {
            (event.timestamp, event.modifiers)
        }
        RawEvent::ContextMenu(event) => (event.timestamp, event.modifiers),
        RawEvent::Ime(event) => match event {
            xui_interface::RawIme::Enabled { timestamp }
            | xui_interface::RawIme::Preedit { timestamp, .. }
            | xui_interface::RawIme::Commit { timestamp, .. }
            | xui_interface::RawIme::Disabled { timestamp } => {
                (*timestamp, xui_interface::Modifiers::default())
            }
        },
    }
}
