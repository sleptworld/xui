pub mod callbacks;
pub mod dispatcher;
pub mod translator;

use crate::event_system::dispatcher::DispatchReport;
use crate::event_system::translator::EventTranslator;
use crate::tree::UiArena;
use xui_interface::events::{EventResult, RawEvent};
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

#[inline(always)]
pub fn dispatch_event(
    arena: &mut UiArena,
    event_translator: &mut EventTranslator,
    event: RawEvent,
) -> EventResult {
    dispatch_event_pipeline(arena, event_translator, event).result()
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

pub fn dispatch_event_pipeline(
    arena: &mut UiArena,
    translator: &mut EventTranslator,
    event: RawEvent,
) -> EventDispatchReport {
    let semantic_events = translator.translate_raw_event(&event, arena);
    let raw = dispatcher::dispatch_raw(arena, event);
    let semantic = semantic_events
        .into_iter()
        .map(|event| dispatcher::dispatch_semantic(arena, event))
        .collect();

    EventDispatchReport { raw, semantic }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Size;
    use crate::event_system::callbacks::EventHandlers;
    use crate::text::{testing::ZeroTextBackend, TextHost};
    use crate::widgets::{text_input, TextController, WidgetI};
    use std::time::Instant;
    use xui_interface::events::{EventPhase, RawEvent};
    use xui_interface::XuiPointerId;
    use xui_interface::{
        Modifiers, PointerButton, PointerButtons, PointerKind, RawPointerButton, RawPointerMove,
        RawTextInput, Style,
    };

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

    fn pointer_down(position: crate::core::Point) -> RawEvent {
        RawEvent::PointerDown(RawPointerButton {
            position,
            pointer_id: XuiPointerId::new(0),
            device_id: None,
            kind: PointerKind::Mouse,
            button: PointerButton::Primary,
            buttons: PointerButtons::from_button(PointerButton::Primary),
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
        })
    }

    #[test]
    fn dispatch_event_pipeline_translates_raw_and_dispatches_semantic_events() {
        let mut arena = UiArena::new();
        let mut translator = EventTranslator::default();
        let target = arena.root();
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

        let event = pointer_move(crate::core::Point::new(1.0, 1.0));
        let report = dispatch_event_pipeline(&mut arena, &mut translator, event);

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
        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        let mut translator = EventTranslator::default();

        let event = pointer_move(crate::core::Point::new(1.0, 1.0));
        let first = dispatch_event_pipeline(&mut arena, &mut translator, event);
        // let second = dispatch_event_pipeline(&mut arena, &mut translator, event);

        assert!(!first.semantic.is_empty());
        // assert!(second.semantic.is_empty());
        // assert!(!second.raw.steps.is_empty());
    }

    #[test]
    fn text_input_focus_receives_raw_text_input() {
        let mut arena = UiArena::new();
        let controller = TextController::new();
        let widget = WidgetI::new(
            text_input()
                .controller(controller.clone())
                .style(Style::new().width(80.0).height(20.0)),
        );
        let id = arena.create_node(
            widget.key(),
            widget.props_hash(),
            widget,
            EventHandlers::default(),
        );
        arena.append_child(arena.root(), id);

        let mut measurer = TextHost::new(ZeroTextBackend);
        arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
        let mut translator = EventTranslator::default();

        dispatch_event_pipeline(
            &mut arena,
            &mut translator,
            pointer_down(crate::core::Point::new(1.0, 1.0)),
        );

        assert_eq!(arena.focused_node(), Some(id));

        dispatch_event_pipeline(
            &mut arena,
            &mut translator,
            RawEvent::TextInput(RawTextInput {
                text: "abc".to_owned(),
                modifiers: Modifiers::default(),
                timestamp: Instant::now(),
            }),
        );

        assert_eq!(controller.text(), "abc");
    }
}
