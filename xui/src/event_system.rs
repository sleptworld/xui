pub mod callbacks;
pub mod dispatcher;
pub mod interaction;
pub mod translator;

use crate::event_system::dispatcher::DispatchReport;
use crate::event_system::translator::EventTranslator;
use crate::text::{TextHost, TextLayoutQuery};
use crate::ui_runtime::NodeView;
use crate::ui_runtime::UiRuntime;
use xui_interface::events::{EventResult, RawEvent};
use xui_interface::{
    EventPhase, EventRequest, EventRequests, NodeId, TextBackend, WidgetUpdateFlags,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventState {
    hovered: Option<NodeId>,
    pointer_capture: Option<NodeId>,
}

impl EventState {
    pub fn hovered(&self) -> Option<NodeId> {
        self.hovered
    }

    pub fn pointer_capture(&self) -> Option<NodeId> {
        self.pointer_capture
    }

    pub(crate) fn clear_node(&mut self, id: NodeId) {
        if self.hovered == Some(id) {
            self.hovered = None;
        }
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
