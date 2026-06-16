pub mod callbacks;
pub mod dispatcher;
pub mod translator;

use xui_interface::events::{Event, EventResult, RawEvent};
use crate::event_system::dispatcher::DispatchReport;
use crate::tree::UiArena;
use xui_interface::NodeId;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventState {
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    pointer_capture: Option<NodeId>,
}

impl EventState {
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    pub fn hovered(&self) -> Option<NodeId> {
        self.hovered
    }

    pub fn pointer_capture(&self) -> Option<NodeId> {
        self.pointer_capture
    }

    pub(crate) fn clear_node(&mut self, id: NodeId) {
        if self.focused == Some(id) {
            self.focused = None;
        }
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        if self.pointer_capture == Some(id) {
            self.pointer_capture = None;
        }
    }

    pub(crate) fn focus(&mut self, id: NodeId) {
        self.focused = Some(id);
    }

    pub(crate) fn clear_focus(&mut self) {
        self.focused = None;
    }

    pub(crate) fn capture_pointer(&mut self, id: NodeId) {
        self.pointer_capture = Some(id);
    }

    pub(crate) fn release_pointer_capture(&mut self) {
        self.pointer_capture = None;
    }
}

pub fn dispatch_event(arena: &mut UiArena, event: &RawEvent) -> EventResult {
    dispatch_event_pipeline(arena, event).result()
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

pub fn dispatch_event_pipeline(arena: &mut UiArena, event: &RawEvent) -> EventDispatchReport {
    let raw = dispatcher::dispatch_raw(arena, event);

    let mut translator = arena.take_event_translator();
    let mut semantic_events = translator.translate_raw_event(event, arena);
    arena.replace_event_translator(translator);

    let semantic = semantic_events
        .iter_mut()
        .map(|event| dispatcher::dispatch_semantic(arena, event))
        .collect();

    EventDispatchReport { raw, semantic }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Size;
    use xui_interface::events::{EventPhase, RawEvent};
    use std::time::Instant;
    use xui_interface::XuiPointerId;
    use xui_interface::{
        ComputedTextStyle, Modifiers, PointerButtons, PointerKind, RawPointerMove,
        TextLayoutConstraints, TextMeasurer,
    };

    struct ZeroTextMeasurer;

    impl TextMeasurer for ZeroTextMeasurer {
        fn measure_text(&mut self, _text: &str, _props: &ComputedTextStyle) -> Size<f32> {
            Size::<f32>::ZERO
        }

        fn measure_text_with_constraints(
            &mut self,
            _text: &str,
            _props: &ComputedTextStyle,
            _constraints: TextLayoutConstraints,
        ) -> Size<f32> {
            Size::<f32>::ZERO
        }
    }

    fn pointer_move(position: crate::core::Point) -> RawEvent {
        RawEvent::PointerMove(RawPointerMove {
            position,
            pointer_id: XuiPointerId::new(0),
            device_id: None,
            kind: PointerKind::Mouse,
            button: None,
            buttons: PointerButtons::default(),
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
        })
    }

    #[test]
    fn dispatch_event_pipeline_translates_raw_and_dispatches_semantic_events() {
        let mut arena = UiArena::new();
        let target = arena.root();
        let mut measurer = ZeroTextMeasurer;
        arena.update_tree(target, Size::new(100.0, 100.0), &mut measurer);

        let event = pointer_move(crate::core::Point::new(1.0, 1.0));
        let report = dispatch_event_pipeline(&mut arena, &event);

        assert_eq!(report.raw.steps.last().map(|step| step.node), Some(target));
        assert!(report.semantic.iter().any(|semantic| {
            semantic.steps
                == vec![dispatcher::DispatchStep {
                    node: target,
                    phase: EventPhase::Target,
                }]
        }));
    }

    #[test]
    fn dispatch_event_pipeline_keeps_translator_state_between_events() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let mut measurer = ZeroTextMeasurer;
        arena.update_tree(root, Size::new(100.0, 100.0), &mut measurer);

        let event = pointer_move(crate::core::Point::new(1.0, 1.0));
        let first = dispatch_event_pipeline(&mut arena, &event);
        let second = dispatch_event_pipeline(&mut arena, &event);

        assert!(!first.semantic.is_empty());
        assert!(second.semantic.is_empty());
        assert!(!second.raw.steps.is_empty());
    }
}
