pub mod raw;
pub mod semantic;
pub mod shortcut;
pub use raw::*;
pub use semantic::*;
pub use shortcut::*;

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
