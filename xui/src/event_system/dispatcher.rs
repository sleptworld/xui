use crate::event_system::EventContext;
use crate::text::{TextHost, TextLayoutQuery, TextLayoutSlot};
use crate::ui_runtime::UiRuntime;
use xui_interface::events::semantic::SemanticEvent;
use xui_interface::events::{EventPhase, PropagationMode, RawEvent};
use xui_interface::{
    EventRef, EventRequest, EventRequests, EventResult, NodeId, TextBackend, WidgetUpdateFlags,
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
    pub fn dispatch_raw<B: TextBackend>(
        arena: &mut UiRuntime,
        host_text_cache: &TextHost<B>,
        event: RawEvent,
    ) -> DispatchReport {
        let Some(target) = resolve_raw_target(arena, &event) else {
            return DispatchReport::default();
        };

        let path = arena.event_path(target);
        dispatch_path(
            path,
            PropagationMode::CaptureTargetBubble,
            move |node, phase| dispatch_raw_to_node(arena, host_text_cache, node, &event, phase),
        )
    }

    pub fn dispatch_semantic<B: TextBackend>(
        arena: &mut UiRuntime,
        host_text_cache: &TextHost<B>,
        mut event: SemanticEvent,
    ) -> DispatchReport {
        let target = event.meta().target;
        if !arena.contains(target) {
            return DispatchReport::default();
        }

        let path = arena.event_path(target);
        let mode = event.propagation_mode();

        dispatch_path(path, mode, move |node, phase| {
            dispatch_semantic_to_node(arena, host_text_cache, node, &mut event, phase)
        })
    }
}

#[inline]
pub fn dispatch_raw<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    event: RawEvent,
) -> DispatchReport {
    EventDispatcher::dispatch_raw(arena, host_text_cache, event)
}

#[inline]
pub fn dispatch_semantic<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    event: SemanticEvent,
) -> DispatchReport {
    EventDispatcher::dispatch_semantic(arena, host_text_cache, event)
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

fn resolve_raw_target(arena: &UiRuntime, event: &RawEvent) -> Option<NodeId> {
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

    if let Some(focused) = arena.focused_node().filter(|node| arena.contains(*node)) {
        return Some(focused);
    }

    Some(arena.root())
}

fn dispatch_raw_to_node<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    node: NodeId,
    event: &RawEvent,
    phase: EventPhase,
) -> EventResult {
    dispatch_builtin_event(arena, host_text_cache, node, EventRef::Raw(event), phase)
}

fn dispatch_semantic_to_node<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    node: NodeId,
    event: &mut SemanticEvent,
    phase: EventPhase,
) -> EventResult {
    let meta = event.meta_mut();
    meta.current_target = node;
    meta.phase = phase;

    apply_semantic_state(arena, node, event, phase);

    dispatch_builtin_event(
        arena,
        host_text_cache,
        node,
        EventRef::Semantic(&event),
        phase,
    );

    let Some(handles) = arena.callback_handles(node) else {
        return EventResult::Ignored;
    };

    let mut request_update = WidgetUpdateFlags::empty();
    let mut requests = EventRequests::default();
    let result = {
        let (node, callbacks) = arena
            .node_and_callbacks_mut(node)
            .expect("event target disappeared during dispatch");
        let text_layout = primary_text_query(host_text_cache, node.id);
        let mut cx =
            EventContext::new(node, text_layout, phase, &mut request_update, &mut requests);
        let result = callbacks.dispatch_semantic(handles, event, &mut cx);
        result
    };

    apply_event_context(arena, node, request_update, &requests);
    result
}

fn apply_semantic_state(
    arena: &mut UiRuntime,
    node: NodeId,
    event: &SemanticEvent,
    phase: EventPhase,
) {
    if phase != EventPhase::Target {
        return;
    }

    let Some((flag, enabled)) = semantic_state_change(event) else {
        return;
    };
    arena.set_widget_state_flag(node, flag, enabled);
}

fn semantic_state_change(event: &SemanticEvent) -> Option<(xui_interface::WidgetState, bool)> {
    match event {
        SemanticEvent::HoverEnter(_) => Some((xui_interface::WidgetState::HOVERED, true)),
        SemanticEvent::HoverLeave(_) => Some((xui_interface::WidgetState::HOVERED, false)),
        SemanticEvent::PressStart(_) => Some((xui_interface::WidgetState::PRESSED, true)),
        SemanticEvent::PressEnd(_) | SemanticEvent::PressCancel(_) => {
            Some((xui_interface::WidgetState::PRESSED, false))
        }
        SemanticEvent::Focus(_) | SemanticEvent::FocusIn(_) => {
            Some((xui_interface::WidgetState::FOCUSED, true))
        }
        SemanticEvent::Blur(_) | SemanticEvent::FocusOut(_) => {
            Some((xui_interface::WidgetState::FOCUSED, false))
        }
        SemanticEvent::DragStart(_) | SemanticEvent::DragMove(_) => {
            Some((xui_interface::WidgetState::DRAGGING, true))
        }
        SemanticEvent::DragEnd(_) | SemanticEvent::DragCancel(_) => {
            Some((xui_interface::WidgetState::DRAGGING, false))
        }
        _ => None,
    }
}

fn dispatch_builtin_event<B: TextBackend>(
    arena: &mut UiRuntime,
    host_text_cache: &TextHost<B>,
    node_id: NodeId,
    event: EventRef<'_>,
    phase: EventPhase,
) -> EventResult {
    let mut update = WidgetUpdateFlags::empty();
    let mut requests = EventRequests::default();
    let result = {
        arena
            .node(node_id)
            .map(|node| {
                let text_layout = primary_text_query(host_text_cache, node_id);
                let mut cx =
                    EventContext::new(node, text_layout, phase, &mut update, &mut requests);
                node.widget.handle_event(event, &mut cx)
            })
            .unwrap_or(EventResult::Ignored)
    };

    apply_event_context(arena, node_id, update, &requests);
    result
}

fn primary_text_query<B: TextBackend>(
    host: &TextHost<B>,
    owner: NodeId,
) -> Option<&dyn TextLayoutQuery> {
    let handle = host.active_slot(owner, TextLayoutSlot::PRIMARY)?;
    host.query(handle)
}

fn apply_event_context(
    arena: &mut UiRuntime,
    node: NodeId,
    request_update: WidgetUpdateFlags,
    requests: &EventRequests,
) {
    if !request_update.is_empty() && arena.contains(node) {
        arena.mark_dirty(node, request_update);
    }

    for request in requests.iter() {
        match request {
            EventRequest::Focus(node) if arena.contains(node) => {
                arena
                    .focus_manager_mut()
                    .request_focus(Some(node), xui_interface::FocusReason::Programmatic);
            }
            EventRequest::ClearFocus => arena
                .focus_manager_mut()
                .request_focus(None, xui_interface::FocusReason::Programmatic),
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
