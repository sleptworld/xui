pub mod raw;
pub mod semantic;
pub use raw::*;
pub use semantic::*;

use crate::{NodeId, WidgetUpdateFlags};

#[derive(Debug, Clone)]
pub enum Event {
    Raw(raw::RawEvent),
    Semantic(semantic::SemanticEvent),
}

#[derive(Debug, Clone, Copy)]
pub enum EventRef<'a> {
    Raw(&'a raw::RawEvent),
    Semantic(&'a semantic::SemanticEvent),
}

pub struct EventCtx<'a> {
    pub stopped: &'a mut bool,
    pub default_prevented: &'a mut bool,
    pub captured_pointer: &'a mut Option<raw::XuiPointerId>,
}

impl<'a> EventCtx<'a> {
    pub fn stop_propagation(&mut self) {
        *self.stopped = true;
    }

    pub fn prevent_default(&mut self) {
        *self.default_prevented = true;
    }

    pub fn capture_pointer(&mut self, pointer_id: raw::XuiPointerId) {
        *self.captured_pointer = Some(pointer_id);
    }

    pub fn release_pointer_capture(&mut self) {
        *self.captured_pointer = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    Ignored,
    Consumed,
}

impl EventResult {
    pub fn is_consumed(self) -> bool {
        matches!(self, Self::Consumed)
    }
}

pub struct EventContext<'a> {
    pub node_id: NodeId,
    pub phase: EventPhase,
    request_update: &'a mut WidgetUpdateFlags,
    requests: &'a mut EventRequests,
}

impl<'a> EventContext<'a> {
    pub fn new(
        node_id: NodeId,
        phase: EventPhase,
        request_update: &'a mut WidgetUpdateFlags,
        requests: &'a mut EventRequests,
    ) -> Self {
        Self {
            node_id,
            phase,
            request_update,
            requests,
        }
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

    pub fn invalidate(&mut self, flags: WidgetUpdateFlags) {
        *self.request_update |= flags;
    }

    pub fn request_focus(&mut self) {
        self.requests.push(EventRequest::Focus(self.node_id));
    }

    pub fn clear_focus(&mut self) {
        self.requests.push(EventRequest::ClearFocus);
    }

    pub fn capture_pointer(&mut self) {
        self.requests
            .push(EventRequest::CapturePointer(self.node_id));
    }

    pub fn release_pointer_capture(&mut self) {
        self.requests.push(EventRequest::ReleasePointerCapture);
    }
}
