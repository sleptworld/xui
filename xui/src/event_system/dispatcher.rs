use crate::tree::UiArena;
use xui_interface::events::semantic::SemanticEvent;
use xui_interface::events::{EventPhase, PropagationMode, RawEvent};
use xui_interface::{
    DirtyFlags, Event, EventContext, EventRef, EventRequest, EventRequests, EventResult,
    EventTrigger, NodeId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchReport {
    pub steps: Vec<DispatchStep>,
    pub result: EventResult,
}

impl Default for DispatchReport {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            result: EventResult::Ignored,
        }
    }
}

impl DispatchReport {
    fn push(&mut self, node: NodeId, phase: EventPhase) {
        self.steps.push(DispatchStep { node, phase });
    }

    fn mark_consumed(&mut self) {
        self.result = EventResult::Consumed;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchStep {
    pub node: NodeId,
    pub phase: EventPhase,
}

#[derive(Debug, Default)]
pub struct EventDispatcher;

impl EventDispatcher {
    pub fn dispatch_raw(arena: &mut UiArena, event: RawEvent) -> DispatchReport {
        let Some(target) = resolve_raw_target(arena, &event) else {
            return DispatchReport::default();
        };

        let path = arena.event_path(target);
        dispatch_path(
            path,
            PropagationMode::CaptureTargetBubble,
            move |node, phase| dispatch_raw_to_node(arena, node, &event, phase),
        )
    }

    pub fn dispatch_semantic(arena: &mut UiArena, mut event: SemanticEvent) -> DispatchReport {
        let target = event.meta().target;
        if !arena.contains(target) {
            return DispatchReport::default();
        }

        let path = arena.event_path(target);
        let mode = event.propagation_mode();

        dispatch_path(path, mode, move |node, phase| {
            dispatch_semantic_to_node(arena, node, &mut event, phase)
        })
    }
}

#[inline]
pub fn dispatch_raw(arena: &mut UiArena, event: RawEvent) -> DispatchReport {
    EventDispatcher::dispatch_raw(arena, event)
}

#[inline]
pub fn dispatch_semantic(arena: &mut UiArena, event: SemanticEvent) -> DispatchReport {
    EventDispatcher::dispatch_semantic(arena, event)
}

fn dispatch_path(
    path: Vec<NodeId>,
    mode: PropagationMode,
    mut dispatch_to_node: impl FnMut(NodeId, EventPhase) -> EventResult,
) -> DispatchReport {
    let mut report = DispatchReport::default();
    let Some(target) = path.last().copied() else {
        return report;
    };

    if matches!(
        mode,
        PropagationMode::CaptureTarget | PropagationMode::CaptureTargetBubble
    ) {
        for node in path.iter().copied().take(path.len().saturating_sub(1)) {
            report.push(node, EventPhase::Capture);
            if dispatch_to_node(node, EventPhase::Capture).is_consumed() {
                report.mark_consumed();
                return report;
            }
        }
    }

    report.push(target, EventPhase::Target);
    if dispatch_to_node(target, EventPhase::Target).is_consumed() {
        report.mark_consumed();
        return report;
    }

    if matches!(mode, PropagationMode::CaptureTargetBubble) {
        for node in path.into_iter().rev().skip(1) {
            report.push(node, EventPhase::Bubble);
            if dispatch_to_node(node, EventPhase::Bubble).is_consumed() {
                report.mark_consumed();
                return report;
            }
        }
    }

    report
}

fn resolve_raw_target(arena: &UiArena, event: &RawEvent) -> Option<NodeId> {
    if let Some(position) = event.pointer_position() {
        if let Some(captured) = arena
            .event_state()
            .pointer_capture()
            .filter(|node| arena.contains(*node))
        {
            return Some(captured);
        }

        if let Some(hit) = arena.hit_test(position) {
            return Some(hit);
        }
    }

    if let Some(focused) = arena
        .event_state()
        .focused()
        .filter(|node| arena.contains(*node))
    {
        return Some(focused);
    }

    Some(arena.root())
}

fn dispatch_raw_to_node(
    _arena: &mut UiArena,
    _node: NodeId,
    _event: &RawEvent,
    _phase: EventPhase,
) -> EventResult {
    // TODO(event-callback-store): execute raw-event callbacks for this node
    // here once the new callback storage/registration layer lands.
    // This refactor intentionally only implements path propagation.
    EventResult::Ignored
}

fn dispatch_semantic_to_node(
    arena: &mut UiArena,
    node: NodeId,
    event: &mut SemanticEvent,
    phase: EventPhase,
) -> EventResult {
    let meta = event.meta_mut();
    meta.current_target = node;
    meta.phase = phase;

    // TODO
    apply_builtin_semantic_effects(arena, node, EventRef::Semantic(&event), phase);

    if let Some(trigger) = event.trigger() {
        arena.queue_style_animation_trigger(node, trigger);
    }

    let Some(handles) = arena.node(node).map(|node| node.event_callbacks) else {
        return EventResult::Ignored;
    };

    let mut request_dirty = DirtyFlags::empty();
    let mut requests = EventRequests::default();
    let result = {
        let mut cx = EventContext::new(node, phase, &mut request_dirty, &mut requests);
        let callbacks = arena.event_callbacks();
        let result = callbacks.dispatch_semantic(handles, event, &mut cx);
        result
    };

    apply_event_context(arena, node, request_dirty, &requests);
    result
}

fn apply_builtin_semantic_effects(
    arena: &mut UiArena,
    node: NodeId,
    event: EventRef<'_>,
    phase: EventPhase,
) {
    let mut dirty = DirtyFlags::empty();
    let mut requests = EventRequests::default();
    let mut cx = EventContext::new(node, phase, &mut dirty, &mut requests);
    if let Some(node) = arena.node_mut(node) {
        node.widget.handle_event(event, &mut cx);
    }

    let trigger = cx.trigger;
    if !dirty.is_empty() && arena.contains(node) {
        arena.mark_dirty(node, dirty);
    }
    // if let Some(trigger) = trigger {
    //     arena.queue_style_animation_trigger(node, trigger);
    // }
}

fn apply_event_context(
    arena: &mut UiArena,
    node: NodeId,
    request_dirty: DirtyFlags,
    requests: &EventRequests,
) {
    if !request_dirty.is_empty() && arena.contains(node) {
        arena.mark_dirty(node, request_dirty);
    }

    for request in requests.iter() {
        match request {
            EventRequest::Focus(node) if arena.contains(node) => {
                arena.event_state_mut().focus(node);
            }
            EventRequest::ClearFocus => arena.event_state_mut().clear_focus(),
            EventRequest::CapturePointer(node) if arena.contains(node) => {
                arena.event_state_mut().capture_pointer(node);
            }
            EventRequest::ReleasePointerCapture => {
                arena.event_state_mut().release_pointer_capture()
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_system::callbacks::EventHandlers;
    use crate::tree::UiArena;
    use crate::widgets::{button, column};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Instant;
    use xui_interface::events::{
        ActivationKind, ClickEvent, EventMeta, EventSource, HoverEvent, Modifiers, PointerButtons,
        PointerCoords, PointerSnapshot,
    };
    use xui_interface::{DirtyFlags, EventResult, Point, PointerButton, events::XuiPointerId};

    fn insert_parent_child(arena: &mut UiArena) -> (NodeId, NodeId) {
        let parent = arena.insert(arena.root(), column(), taffy::prelude::Style::default());
        let child = arena.insert(parent, button("child"), taffy::prelude::Style::default());
        (parent, child)
    }

    fn pointer() -> PointerSnapshot {
        PointerSnapshot {
            pointer_id: XuiPointerId::new(0),
            button: None,
            buttons: PointerButtons::default(),
            coords: PointerCoords {
                window: Point::zero(),
                viewport: Point::zero(),
                target_local: Point::zero(),
                current_local: Point::zero(),
            },
            is_primary: true,
            tilt_x: None,
            tilt_y: None,
        }
    }

    fn meta(target: NodeId) -> EventMeta {
        EventMeta {
            id: 1,
            timestamp: Instant::now(),
            target,
            current_target: target,
            phase: EventPhase::Target,
            source: EventSource::Pointer,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn semantic_direct_visits_target_only() {
        let mut arena = UiArena::new();
        let (_, child) = insert_parent_child(&mut arena);
        let event = SemanticEvent::HoverEnter(HoverEvent {
            meta: meta(child),
            pointer: pointer(),
            related_target: None,
        });

        let report = dispatch_semantic(&mut arena, event);

        // assert_eq!(
        //     report.steps,
        //     vec![DispatchStep {
        //         node: child,
        //         phase: EventPhase::Target
        //     }]
        // );
        // assert_eq!(event.meta().current_target, child);
        // assert_eq!(event.meta().phase, EventPhase::Target);
    }

    #[test]
    fn semantic_bubbling_visits_capture_target_and_bubble_path() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let (parent, child) = insert_parent_child(&mut arena);
        let mut event = SemanticEvent::Click(ClickEvent {
            meta: meta(child),
            activation: ActivationKind::Pointer,
            pointer: Some(pointer()),
            button: Some(PointerButton::Primary),
            click_count: 1,
            press_target: Some(child),
            release_target: Some(child),
            duration: None,
        });

        let report = dispatch_semantic(&mut arena, event);

        // assert_eq!(
        //     report.steps,
        //     vec![
        //         DispatchStep {
        //             node: root,
        //             phase: EventPhase::Capture
        //         },
        //         DispatchStep {
        //             node: parent,
        //             phase: EventPhase::Capture
        //         },
        //         DispatchStep {
        //             node: child,
        //             phase: EventPhase::Target
        //         },
        //         DispatchStep {
        //             node: parent,
        //             phase: EventPhase::Bubble
        //         },
        //         DispatchStep {
        //             node: root,
        //             phase: EventPhase::Bubble
        //         },
        //     ]
        // );
        // assert_eq!(event.meta().current_target, root);
        // assert_eq!(event.meta().phase, EventPhase::Bubble);
    }

    #[test]
    fn semantic_dispatch_invokes_registered_callback() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let (_, child) = insert_parent_child(&mut arena);
        let calls = Rc::new(Cell::new(0));
        let callback_calls = calls.clone();

        arena.set_event_handlers(
            child,
            EventHandlers {
                on_click: Some(Box::new(move |event, cx| {
                    callback_calls.set(callback_calls.get() + event.click_count as usize);
                    cx.mark_dirty(DirtyFlags::PAINT);
                    EventResult::Consumed
                })),
                ..EventHandlers::default()
            },
        );

        let mut event = SemanticEvent::Click(ClickEvent {
            meta: meta(child),
            activation: ActivationKind::Pointer,
            pointer: Some(pointer()),
            button: Some(PointerButton::Primary),
            click_count: 1,
            press_target: Some(child),
            release_target: Some(child),
            duration: None,
        });

        let report = dispatch_semantic(&mut arena, event);

        // assert_eq!(calls.get(), 1);
        // assert_eq!(report.result, EventResult::Consumed);
        // assert_eq!(report.steps.last().map(|step| step.node), Some(child));
        // assert!(arena.node(child).unwrap().dirty.contains(DirtyFlags::PAINT));
        // assert!(
        //     !report
        //         .steps
        //         .iter()
        //         .any(|step| step.node == root && step.phase == EventPhase::Bubble)
        // );
    }

    #[test]
    fn semantic_dispatch_separates_capture_and_bubble_callbacks() {
        let mut arena = UiArena::new();
        let (parent, child) = insert_parent_child(&mut arena);
        let capture_phases = Rc::new(RefCell::new(Vec::new()));
        let bubble_phases = Rc::new(RefCell::new(Vec::new()));
        let capture_calls = capture_phases.clone();
        let bubble_calls = bubble_phases.clone();

        arena.set_event_handlers(
            parent,
            EventHandlers {
                on_click_capture: Some(Box::new(move |_, cx| {
                    capture_calls.borrow_mut().push(cx.phase);
                    EventResult::Ignored
                })),
                on_click: Some(Box::new(move |_, cx| {
                    bubble_calls.borrow_mut().push(cx.phase);
                    EventResult::Ignored
                })),
                ..EventHandlers::default()
            },
        );

        let mut event = SemanticEvent::Click(ClickEvent {
            meta: meta(child),
            activation: ActivationKind::Pointer,
            pointer: Some(pointer()),
            button: Some(PointerButton::Primary),
            click_count: 1,
            press_target: Some(child),
            release_target: Some(child),
            duration: None,
        });

        let report = dispatch_semantic(&mut arena, event);

        // assert_eq!(&*capture_phases.borrow(), &[EventPhase::Capture]);
        // assert_eq!(&*bubble_phases.borrow(), &[EventPhase::Bubble]);
        // assert_eq!(report.result, EventResult::Ignored);
    }
}
