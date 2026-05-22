use xui_interface::{DirtyFlags, EventRequest, EventRequests, NodeId};

use crate::event::{Event, EventContext, EventPhase, EventResult, PointerButton};
use crate::tree::UiArena;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventState {
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    pointer_capture: Option<NodeId>,
    hovered_path: Vec<NodeId>,
    click_target: Option<NodeId>,
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
        if self.click_target == Some(id) {
            self.click_target = None;
        }
        self.hovered_path.retain(|node| *node != id);
    }

    fn apply(&mut self, request: EventRequest, request_node_exists: bool) {
        match request {
            EventRequest::Focus(id) if request_node_exists => self.focused = Some(id),
            EventRequest::Focus(_) => {}
            EventRequest::ClearFocus => self.focused = None,
            EventRequest::CapturePointer(id) if request_node_exists => {
                self.pointer_capture = Some(id)
            }
            EventRequest::CapturePointer(_) => {}
            EventRequest::ReleasePointerCapture => self.pointer_capture = None,
        }
    }

    fn set_hovered(&mut self, hovered: Option<NodeId>) {
        self.hovered = hovered;
    }
}

enum NodeDispatch<'a> {
    Raw(&'a Event),
    HoverChange(bool),
    Click,
}

pub fn dispatch_event(arena: &mut UiArena, event: &Event) -> EventResult {
    update_hover(arena, event);

    let target = resolve_target(arena, event);
    if matches!(
        event,
        Event::PointerDown {
            button: PointerButton::Primary,
            ..
        }
    ) {
        arena.event_state_mut().click_target = Some(target);
    }

    let result = dispatch_event_path(arena, target, event);

    if matches!(
        event,
        Event::PointerUp {
            button: PointerButton::Primary,
            ..
        }
    ) {
        let should_click = arena.event_state().click_target == Some(target);
        arena.event_state_mut().click_target = None;
        if should_click {
            let click_result = dispatch_target_and_bubble(arena, target);
            if click_result.is_consumed() {
                return click_result;
            }
        }
    }

    result
}

fn update_hover(arena: &mut UiArena, event: &Event) {
    if let Some(position) = event.pointer_position() {
        let hovered = arena.hit_test(position);
        let new_path = hovered.map(|id| arena.event_path(id)).unwrap_or_default();
        let old_path = arena.event_state().hovered_path.clone();

        #[cfg(debug_assertions)]
        {
            println!("==== OLD PATH ====");

            for o in old_path.iter() {
                if let Some(w) = arena.node(*o) {
                    println!(
                        "Node ID: {:?}, layout: {:?}, type: {:?}",
                        w.id, w.layout, w.node_type
                    );
                }
            }

            println!("==== NEW PATH ====");

            for n in new_path.iter() {
                if let Some(w) = arena.node(*n) {
                    println!(
                        "Node ID: {:?}, layout: {:?}, type: {:?}",
                        w.id, w.layout, w.node_type
                    );
                }
            }
        }

        let common = old_path
            .iter()
            .zip(new_path.iter())
            .take_while(|(old, new)| old == new)
            .count();

        for id in old_path[common..].iter().rev().copied() {
            dispatch_to_node(
                arena,
                id,
                NodeDispatch::HoverChange(false),
                EventPhase::Target,
            );
        }

        for id in new_path[common..].iter().copied() {
            dispatch_to_node(
                arena,
                id,
                NodeDispatch::HoverChange(true),
                EventPhase::Target,
            );
        }

        arena.event_state_mut().set_hovered(hovered);
        arena.event_state_mut().hovered_path = new_path;
    }
}

fn dispatch_event_path(arena: &mut UiArena, target: NodeId, event: &Event) -> EventResult {
    let path = arena.event_path(target);

    for id in path.iter().copied().take(path.len().saturating_sub(1)) {
        if dispatch_to_node(arena, id, NodeDispatch::Raw(event), EventPhase::Capture).is_consumed()
        {
            return EventResult::Consumed;
        }
    }

    if dispatch_to_node(arena, target, NodeDispatch::Raw(event), EventPhase::Target).is_consumed() {
        return EventResult::Consumed;
    }

    for id in path.into_iter().rev().skip(1) {
        if dispatch_to_node(arena, id, NodeDispatch::Raw(event), EventPhase::Bubble).is_consumed() {
            return EventResult::Consumed;
        }
    }

    EventResult::Ignored
}

fn dispatch_target_and_bubble(arena: &mut UiArena, target: NodeId) -> EventResult {
    if dispatch_to_node(arena, target, NodeDispatch::Click, EventPhase::Target).is_consumed() {
        return EventResult::Consumed;
    }

    for id in arena.event_path(target).into_iter().rev().skip(1) {
        if dispatch_to_node(arena, id, NodeDispatch::Click, EventPhase::Bubble).is_consumed() {
            return EventResult::Consumed;
        }
    }

    EventResult::Ignored
}

fn resolve_target(arena: &UiArena, event: &Event) -> NodeId {
    if let Some(position) = event.pointer_position() {
        if let Some(captured) = arena
            .event_state()
            .pointer_capture()
            .filter(|id| arena.contains(*id))
        {
            return captured;
        }

        if let Some(hit) = arena.hit_test(position) {
            return hit;
        }
    }

    arena
        .event_state()
        .focused()
        .filter(|id| arena.contains(*id))
        .unwrap_or_else(|| arena.root())
}

fn dispatch_to_node(
    arena: &mut UiArena,
    id: NodeId,
    dispatch: NodeDispatch<'_>,
    phase: EventPhase,
) -> EventResult {
    let mut request_dirty = DirtyFlags::empty();
    let mut requests = EventRequests::default();

    let result = {
        let Some(node) = arena.node_mut(id) else {
            return EventResult::Ignored;
        };
        let mut cx = EventContext::new(id, phase, &mut request_dirty, &mut requests);

        match dispatch {
            NodeDispatch::Raw(event) => {
                let handler_result = node
                    .event_handlers
                    .on_event
                    .as_mut()
                    .map(|handler| handler(event, &mut cx))
                    .unwrap_or(EventResult::Ignored);

                if handler_result.is_consumed() {
                    handler_result
                } else if phase == EventPhase::Capture {
                    EventResult::Ignored
                } else {
                    let specialized = match event {
                        Event::PointerDown { .. } => node
                            .event_handlers
                            .on_pointer_down
                            .as_mut()
                            .map(|handler| handler(&mut cx))
                            .unwrap_or(EventResult::Ignored),
                        Event::PointerUp { .. } => node
                            .event_handlers
                            .on_pointer_up
                            .as_mut()
                            .map(|handler| handler(&mut cx))
                            .unwrap_or(EventResult::Ignored),
                        Event::PointerMove { .. } => node
                            .event_handlers
                            .on_pointer_move
                            .as_mut()
                            .map(|handler| handler(&mut cx))
                            .unwrap_or(EventResult::Ignored),
                        Event::KeyDown { key } => node
                            .event_handlers
                            .on_key_down
                            .as_mut()
                            .map(|handler| handler(key, &mut cx))
                            .unwrap_or(EventResult::Ignored),
                        Event::KeyUp { key } => node
                            .event_handlers
                            .on_key_up
                            .as_mut()
                            .map(|handler| handler(key, &mut cx))
                            .unwrap_or(EventResult::Ignored),
                        _ => EventResult::Ignored,
                    };

                    if specialized.is_consumed() {
                        specialized
                    } else if phase == EventPhase::Target {
                        node.widget
                            .with_mut(|widget| widget.handle_event(event, &mut cx))
                    } else {
                        EventResult::Ignored
                    }
                }
            }
            NodeDispatch::HoverChange(hovered) => {
                let hover_dirty = node
                    .widget
                    .with_mut(|widget| widget.on_hovered_change(hovered));
                cx.mark_dirty(hover_dirty);

                node.event_handlers
                    .on_hover_change
                    .as_mut()
                    .map(|handler| handler(hovered, &mut cx))
                    .unwrap_or(EventResult::Ignored)
            }
            NodeDispatch::Click => node
                .event_handlers
                .on_click
                .as_mut()
                .map(|handler| handler(&mut cx))
                .unwrap_or(EventResult::Ignored),
        }
    };

    if !request_dirty.is_empty() {
        arena.mark_dirty(id, request_dirty);
    }

    if !requests.is_empty() {
        let requests: Vec<_> = requests.iter().collect();
        for request in requests {
            let request_node = match request {
                EventRequest::Focus(id) | EventRequest::CapturePointer(id) => Some(id),
                EventRequest::ClearFocus | EventRequest::ReleasePointerCapture => None,
            };
            let request_node_exists = request_node.is_none_or(|id| arena.contains(id));
            arena.event_state_mut().apply(request, request_node_exists);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use taffy::prelude as tf;
    use xui_interface::{Color, DirtyFlags, NodeId, Point, Rect};

    use super::*;

    fn test_tree() -> (UiArena, NodeId, NodeId) {
        let mut arena = UiArena::new();
        let root = arena.root();
        let parent = arena.insert(
            root,
            crate::widgets::ContainerWidget::new().background(Color::TRANSPARENT),
            tf::Style::default(),
        );
        let child = arena.insert(
            parent,
            crate::widgets::ContainerWidget::new().background(Color::TRANSPARENT),
            tf::Style::default(),
        );

        arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 100.0, 100.0);
        arena.node_mut(parent).unwrap().layout = Rect::new(0.0, 0.0, 80.0, 80.0);
        arena.node_mut(child).unwrap().layout = Rect::new(0.0, 0.0, 40.0, 40.0);
        (arena, parent, child)
    }

    #[test]
    fn dispatches_capture_target_and_bubble_in_order() {
        let (mut arena, parent, child) = test_tree();
        let seen = Rc::new(RefCell::new(Vec::new()));

        for id in [arena.root(), parent, child] {
            let seen = seen.clone();
            arena.node_mut(id).unwrap().event_handlers.on_event = Some(Box::new(move |_, cx| {
                seen.borrow_mut().push((cx.node_id, cx.phase));
                EventResult::Ignored
            }));
        }

        let result = arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });

        assert_eq!(result, EventResult::Ignored);
        assert_eq!(
            seen.borrow().as_slice(),
            &[
                (arena.root(), EventPhase::Capture),
                (parent, EventPhase::Capture),
                (child, EventPhase::Target),
                (parent, EventPhase::Bubble),
                (arena.root(), EventPhase::Bubble),
            ]
        );
    }

    #[test]
    fn consumed_handler_commits_dirty_before_stopping_propagation() {
        let (mut arena, parent, child) = test_tree();
        arena.node_mut(child).unwrap().event_handlers.on_event = Some(Box::new(|_, cx| {
            cx.mark_dirty(DirtyFlags::PAINT);
            EventResult::Consumed
        }));
        arena.node_mut(parent).unwrap().event_handlers.on_event = Some(Box::new(|_, cx| {
            if cx.phase == EventPhase::Bubble {
                panic!("bubble should be stopped");
            }
            EventResult::Ignored
        }));

        let result = arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });

        assert_eq!(result, EventResult::Consumed);
        assert!(arena.node(child).unwrap().dirty.contains(DirtyFlags::PAINT));
    }

    #[test]
    fn focus_routes_keyboard_events_to_focused_node() {
        let (mut arena, _parent, child) = test_tree();
        let key_events = Rc::new(RefCell::new(0));
        let key_events_for_child = key_events.clone();

        arena.node_mut(child).unwrap().event_handlers.on_event =
            Some(Box::new(move |event, cx| {
                if matches!(event, Event::PointerDown { .. }) {
                    cx.request_focus();
                    return EventResult::Consumed;
                }
                if matches!(event, Event::KeyDown { .. }) && cx.phase == EventPhase::Target {
                    *key_events_for_child.borrow_mut() += 1;
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }));

        arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });
        arena.dispatch_event(&Event::KeyDown {
            key: crate::event::Key::Enter,
        });

        assert_eq!(arena.focused_node(), Some(child));
        assert_eq!(*key_events.borrow(), 1);
    }

    #[test]
    fn pointer_capture_overrides_hit_test_until_released() {
        let (mut arena, parent, child) = test_tree();
        let child_hits = Rc::new(RefCell::new(0));
        let child_hits_for_handler = child_hits.clone();
        let parent_hits = Rc::new(RefCell::new(0));
        let parent_hits_for_handler = parent_hits.clone();

        arena.node_mut(child).unwrap().event_handlers.on_event =
            Some(Box::new(move |event, cx| match event {
                Event::PointerDown { .. } => {
                    cx.capture_pointer();
                    EventResult::Consumed
                }
                Event::PointerMove { .. } => {
                    *child_hits_for_handler.borrow_mut() += 1;
                    cx.release_pointer_capture();
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            }));
        arena.node_mut(parent).unwrap().event_handlers.on_event =
            Some(Box::new(move |event, cx| {
                if matches!(event, Event::PointerMove { .. }) && cx.phase == EventPhase::Target {
                    *parent_hits_for_handler.borrow_mut() += 1;
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }));

        arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });
        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(60.0, 60.0),
        });
        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(60.0, 60.0),
        });

        assert_eq!(*child_hits.borrow(), 1);
        assert_eq!(*parent_hits.borrow(), 1);
        assert_eq!(arena.pointer_capture_node(), None);
        assert_eq!(arena.hovered_node(), Some(parent));
    }

    #[test]
    fn capture_on_event_can_intercept_and_commit_requests() {
        let (mut arena, parent, child) = test_tree();
        let child_reached = Rc::new(RefCell::new(false));
        let child_reached_for_handler = child_reached.clone();

        arena.node_mut(parent).unwrap().event_handlers.on_event = Some(Box::new(|_, cx| {
            if cx.phase == EventPhase::Capture {
                cx.mark_dirty(DirtyFlags::PAINT);
                cx.request_focus();
                cx.capture_pointer();
                return EventResult::Consumed;
            }
            EventResult::Ignored
        }));
        arena.node_mut(child).unwrap().event_handlers.on_event = Some(Box::new(move |_, _| {
            *child_reached_for_handler.borrow_mut() = true;
            EventResult::Ignored
        }));

        let result = arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });

        assert_eq!(result, EventResult::Consumed);
        assert!(!*child_reached.borrow());
        assert_eq!(arena.focused_node(), Some(parent));
        assert_eq!(arena.pointer_capture_node(), Some(parent));
        assert!(
            arena
                .node(parent)
                .unwrap()
                .dirty
                .contains(DirtyFlags::PAINT)
        );
    }

    #[test]
    fn pointer_handlers_run_target_then_bubble_and_stop_on_consume() {
        let (mut arena, parent, child) = test_tree();
        let seen = Rc::new(RefCell::new(Vec::new()));

        for id in [arena.root(), parent, child] {
            let seen = seen.clone();
            arena.node_mut(id).unwrap().event_handlers.on_pointer_down =
                Some(Box::new(move |cx| {
                    seen.borrow_mut().push((cx.node_id, cx.phase));
                    if cx.node_id == parent {
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                }));
        }

        let result = arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });

        assert_eq!(result, EventResult::Consumed);
        assert_eq!(
            seen.borrow().as_slice(),
            &[(child, EventPhase::Target), (parent, EventPhase::Bubble),]
        );
    }

    #[test]
    fn key_handlers_run_target_then_bubble_for_focused_node() {
        let (mut arena, parent, child) = test_tree();
        let seen = Rc::new(RefCell::new(Vec::new()));

        arena.node_mut(child).unwrap().event_handlers.on_event = Some(Box::new(|event, cx| {
            if matches!(event, Event::PointerDown { .. }) && cx.phase == EventPhase::Target {
                cx.request_focus();
                return EventResult::Consumed;
            }
            EventResult::Ignored
        }));

        for id in [arena.root(), parent, child] {
            let seen = seen.clone();
            arena.node_mut(id).unwrap().event_handlers.on_key_down =
                Some(Box::new(move |key, cx| {
                    seen.borrow_mut().push((key.clone(), cx.node_id, cx.phase));
                    if cx.node_id == parent {
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                }));
        }

        arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });
        let result = arena.dispatch_event(&Event::KeyDown {
            key: crate::event::Key::Enter,
        });

        assert_eq!(result, EventResult::Consumed);
        assert_eq!(
            seen.borrow().as_slice(),
            &[
                (crate::event::Key::Enter, child, EventPhase::Target),
                (crate::event::Key::Enter, parent, EventPhase::Bubble),
            ]
        );
    }

    #[test]
    fn key_up_handler_receives_key_at_target() {
        let (mut arena, _parent, child) = test_tree();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_for_handler = seen.clone();

        arena.node_mut(child).unwrap().event_handlers.on_event = Some(Box::new(|event, cx| {
            if matches!(event, Event::PointerDown { .. }) {
                cx.request_focus();
                return EventResult::Consumed;
            }
            EventResult::Ignored
        }));
        arena.node_mut(child).unwrap().event_handlers.on_key_up = Some(Box::new(move |key, cx| {
            seen_for_handler
                .borrow_mut()
                .push((key.clone(), cx.node_id, cx.phase));
            EventResult::Consumed
        }));

        arena.dispatch_event(&Event::PointerDown {
            position: Point::new(2.0, 2.0),
            button: crate::event::PointerButton::Primary,
        });
        let result = arena.dispatch_event(&Event::KeyUp {
            key: crate::event::Key::Escape,
        });

        assert_eq!(result, EventResult::Consumed);
        assert_eq!(
            seen.borrow().as_slice(),
            &[(crate::event::Key::Escape, child, EventPhase::Target)]
        );
    }

    #[test]
    fn hover_change_only_reports_entered_and_left_path_segments() {
        let (mut arena, parent, child) = test_tree();
        let seen = Rc::new(RefCell::new(Vec::new()));

        for id in [arena.root(), parent, child] {
            let seen = seen.clone();
            arena.node_mut(id).unwrap().event_handlers.on_hover_change =
                Some(Box::new(move |hovered, cx| {
                    seen.borrow_mut().push((cx.node_id, hovered));
                    EventResult::Ignored
                }));
        }

        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(2.0, 2.0),
        });
        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(60.0, 60.0),
        });
        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(120.0, 120.0),
        });

        assert_eq!(
            seen.borrow().as_slice(),
            &[
                (arena.root(), true),
                (parent, true),
                (child, true),
                (child, false),
                (parent, false),
                (arena.root(), false),
            ]
        );
    }

    #[test]
    fn hover_change_notifies_widget_without_registered_handler() {
        let mut arena = UiArena::new();
        let root = arena.root();
        let button = arena.insert(
            root,
            crate::widgets::ButtonWidget::new("Press"),
            tf::Style::default(),
        );

        arena.node_mut(root).unwrap().layout = Rect::new(0.0, 0.0, 100.0, 100.0);
        arena.node_mut(button).unwrap().layout = Rect::new(0.0, 0.0, 40.0, 40.0);

        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(2.0, 2.0),
        });

        let hovered = arena.node(button).unwrap().widget.with(|widget| {
            widget
                .as_any()
                .downcast_ref::<crate::widgets::ButtonWidget>()
                .unwrap()
                .hovered
        });
        assert!(hovered);
        assert!(
            arena
                .node(button)
                .unwrap()
                .dirty
                .contains(DirtyFlags::PAINT)
        );

        arena.dispatch_event(&Event::PointerMove {
            position: Point::new(80.0, 80.0),
        });

        let hovered = arena.node(button).unwrap().widget.with(|widget| {
            widget
                .as_any()
                .downcast_ref::<crate::widgets::ButtonWidget>()
                .unwrap()
                .hovered
        });
        assert!(!hovered);
    }
}
