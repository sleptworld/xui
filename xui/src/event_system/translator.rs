use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use xui_interface::{NodeId, Point, PointerButton, Translation, WidgetUpdateFlags, XuiPointerId};

use crate::tree::UiArena;
use xui_interface::events::*;

pub struct EventTranslator {
    next_event_id: EventId,
    next_press_id: u64,
    next_drag_id: u64,

    hover_paths: FxHashMap<XuiPointerId, Vec<NodeId>>,
    active_presses: FxHashMap<(XuiPointerId, PointerButton), ActivePress>,
    active_drags: FxHashMap<XuiPointerId, ActiveDrag>,
    last_click: Option<ClickRecord>,
    pointer_capture: FxHashMap<XuiPointerId, NodeId>,
    config: EventTranslatorConfig,
}

#[derive(Debug, Clone)]
pub struct EventTranslatorConfig {
    pub drag_threshold: f32,
    pub double_click_timeout: Duration,
    pub double_click_max_distance: f32,
}

impl Default for EventTranslatorConfig {
    fn default() -> Self {
        Self {
            drag_threshold: 4.0,
            double_click_timeout: Duration::from_millis(500),
            double_click_max_distance: 4.0,
        }
    }
}

pub struct ActivePress {
    pub press_id: PressId,
    pub pointer_id: XuiPointerId,
    pub kind: PointerKind,
    pub button: PointerButton,
    pub target: NodeId,
    pub path: Vec<NodeId>,
    pub start_position: Point,
    pub current_position: Point,
    pub started_at: Instant,
    pub modifiers: Modifiers,
    /// 是否已经因为移动距离过大进入 drag
    pub became_drag: bool,
}

pub struct ActiveDrag {
    pub drag_id: DragId,
    pub pointer_id: XuiPointerId,
    pub source: NodeId,
    pub start_position: Point,
    pub previous_position: Point,
    pub current_position: Point,
    pub started_at: Instant,
}

pub struct ClickRecord {
    pub target: NodeId,
    pub button: PointerButton,
    pub position: Point,
    pub timestamp: Instant,
    pub click_count: u8,
}

impl Default for EventTranslator {
    fn default() -> Self {
        Self::new(EventTranslatorConfig::default())
    }
}

impl EventTranslator {
    pub(crate) fn apply_focus_request(
        &mut self,
        arena: &mut UiArena,
        request: crate::focus::FocusRequest,
        timestamp: Instant,
        modifiers: Modifiers,
    ) -> Vec<SemanticEvent> {
        let source = match request.reason {
            FocusReason::Pointer => EventSource::Pointer,
            FocusReason::Keyboard => EventSource::Keyboard,
            FocusReason::Window => EventSource::Window,
            FocusReason::Programmatic | FocusReason::NodeRemoved | FocusReason::Disabled => {
                EventSource::Programmatic
            }
        };
        let mut out = Vec::new();
        self.change_focus(
            arena,
            request.target,
            request.reason,
            source,
            timestamp,
            modifiers,
            &mut out,
        );
        out
    }
    pub(crate) fn command_event(
        &mut self,
        target: NodeId,
        command: xui_interface::CommandId,
        shortcut: xui_interface::Shortcut,
        raw: &xui_interface::RawKeyboard,
    ) -> SemanticEvent {
        SemanticEvent::Command(xui_interface::CommandEvent {
            meta: self.make_meta(
                raw.timestamp,
                target,
                target,
                EventPhase::Target,
                EventSource::Keyboard,
                raw.modifiers,
            ),
            command,
            shortcut,
        })
    }
    pub fn new(config: EventTranslatorConfig) -> Self {
        Self {
            next_event_id: 1,
            next_press_id: 1,
            next_drag_id: 1,
            hover_paths: FxHashMap::default(),
            active_presses: FxHashMap::default(),
            active_drags: FxHashMap::default(),
            last_click: None,
            pointer_capture: FxHashMap::default(),
            config,
        }
    }

    pub fn set_pointer_capture(&mut self, pointer_id: XuiPointerId, target: NodeId) {
        self.pointer_capture.insert(pointer_id, target);
    }

    pub fn release_pointer_capture(&mut self, pointer_id: XuiPointerId) {
        self.pointer_capture.remove(&pointer_id);
    }

    pub fn translate_raw_event(
        &mut self,
        raw: &RawEvent,
        arena: &mut UiArena,
    ) -> Vec<SemanticEvent> {
        match raw {
            RawEvent::PointerMove(raw) => self.translate_pointer_move(raw, arena),
            RawEvent::PointerDown(raw) => self.translate_pointer_down(raw, arena),
            RawEvent::PointerUp(raw) => self.translate_pointer_up(raw, arena),
            RawEvent::PointerCancel(raw) => self.translate_pointer_cancel(raw, arena),
            RawEvent::Wheel(raw) => self.translate_wheel(raw, arena),
            RawEvent::Keyboard(raw) if raw.state == xui_interface::events::KeyState::Down => {
                self.translate_key_down(&raw, arena)
            }
            RawEvent::Keyboard(_) => Vec::new(),
            RawEvent::WindowBlur(raw) => self.translate_window_blur(raw, arena),
            RawEvent::WindowFocus(_) => Vec::new(),
            RawEvent::ContextMenu(raw) => self.translate_context_menu(raw, arena),
            RawEvent::Ime(_) => Vec::new(),
        }
    }

    fn alloc_event_id(&mut self) -> EventId {
        let id = self.next_event_id;
        self.next_event_id += 1;
        id
    }

    fn alloc_press_id(&mut self) -> PressId {
        let id = self.next_press_id;
        self.next_press_id += 1;
        PressId(id)
    }

    fn alloc_drag_id(&mut self) -> DragId {
        let id = self.next_drag_id;
        self.next_drag_id += 1;
        DragId(id)
    }

    fn translate_pointer_move(
        &mut self,
        raw: &RawPointerMove,
        arena: &mut UiArena,
    ) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        let hit_target = self.resolve_pointer_target(raw.pointer_id, raw.position, arena);
        let pointer = self.make_pointer_snapshot(
            raw.pointer_id,
            raw.button,
            raw.buttons,
            raw.position,
            hit_target,
            arena,
        );

        self.update_hover(
            raw.pointer_id,
            hit_target,
            pointer,
            raw.timestamp,
            raw.modifiers,
            arena,
            &mut out,
        );

        self.update_press_or_drag_on_move(
            raw.pointer_id,
            pointer,
            raw.timestamp,
            raw.modifiers,
            arena,
            &mut out,
        );

        out
    }

    fn translate_pointer_down(
        &mut self,
        raw: &RawPointerButton,
        arena: &mut UiArena,
    ) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        let Some(target) = self.resolve_pointer_target(raw.pointer_id, raw.position, arena) else {
            return out;
        };

        let pointer = self.make_pointer_snapshot(
            raw.pointer_id,
            Some(raw.button),
            raw.buttons,
            raw.position,
            Some(target),
            arena,
        );

        if let Some(focus_target) = nearest_focusable_ancestor(arena, target) {
            self.change_focus(
                arena,
                Some(focus_target),
                FocusReason::Pointer,
                EventSource::Pointer,
                raw.timestamp,
                raw.modifiers,
                &mut out,
            );
        }

        let press_id = self.alloc_press_id();
        let path = arena.event_path(target);

        self.active_presses.insert(
            (raw.pointer_id, raw.button),
            ActivePress {
                press_id,
                pointer_id: raw.pointer_id,
                kind: raw.kind,
                button: raw.button,
                target,
                path,
                start_position: raw.position,
                current_position: raw.position,
                started_at: raw.timestamp,
                modifiers: raw.modifiers,
                became_drag: false,
            },
        );

        let meta = self.make_meta(
            raw.timestamp,
            target,
            target,
            EventPhase::Target,
            EventSource::Pointer,
            raw.modifiers,
        );

        out.push(SemanticEvent::PressStart(PressEvent {
            meta,
            press_id,
            pointer,
            press_target: target,
            start_position: raw.position,
            current_position: raw.position,
            delta: Translation::zero(),
            duration: None,
            cancel_reason: None,
        }));

        out
    }

    fn translate_pointer_up(
        &mut self,
        raw: &RawPointerButton,
        arena: &mut UiArena,
    ) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        let release_target = self.resolve_pointer_target(raw.pointer_id, raw.position, arena);
        let pointer = self.make_pointer_snapshot(
            raw.pointer_id,
            Some(raw.button),
            raw.buttons,
            raw.position,
            release_target,
            arena,
        );

        if let Some(drag) = self.active_drags.remove(&raw.pointer_id) {
            let meta = self.make_meta(
                raw.timestamp,
                drag.source,
                drag.source,
                EventPhase::Target,
                EventSource::Pointer,
                raw.modifiers,
            );

            out.push(SemanticEvent::DragEnd(DragEvent {
                meta,
                drag_id: drag.drag_id,
                pointer,
                source: drag.source,
                over: release_target,
                start_position: drag.start_position,
                previous_position: drag.previous_position,
                current_position: raw.position,
                delta: (raw.position - drag.previous_position).into(),
                total_delta: (raw.position - drag.start_position).into(),
                duration: Some(raw.timestamp.duration_since(drag.started_at)),
                cancel_reason: None,
            }));

            self.active_presses.remove(&(raw.pointer_id, raw.button));
            return out;
        }

        let Some(press) = self.active_presses.remove(&(raw.pointer_id, raw.button)) else {
            return out;
        };

        let meta = self.make_meta(
            raw.timestamp,
            press.target,
            press.target,
            EventPhase::Target,
            EventSource::Pointer,
            raw.modifiers,
        );

        out.push(SemanticEvent::PressEnd(PressEvent {
            meta,
            press_id: press.press_id,
            pointer,
            press_target: press.target,
            start_position: press.start_position,
            current_position: raw.position,
            delta: (raw.position - press.start_position).into(),
            duration: Some(raw.timestamp.duration_since(press.started_at)),
            cancel_reason: None,
        }));

        let is_valid_click = release_target == Some(press.target)
            && raw.button == PointerButton::Primary
            && distance(raw.position, press.start_position) <= self.config.drag_threshold;

        if is_valid_click {
            let click_count =
                self.compute_click_count(press.target, raw.button, raw.position, raw.timestamp);
            let click_meta = self.make_meta(
                raw.timestamp,
                press.target,
                press.target,
                EventPhase::Target,
                EventSource::Pointer,
                raw.modifiers,
            );

            let click_event = ClickEvent {
                meta: click_meta,
                activation: ActivationKind::Pointer,
                pointer: Some(pointer),
                button: Some(raw.button),
                click_count,
                press_target: Some(press.target),
                release_target,
                duration: Some(raw.timestamp.duration_since(press.started_at)),
            };

            if click_count == 2 {
                out.push(SemanticEvent::DoubleClick(click_event.clone()));
            }
            out.push(SemanticEvent::Click(click_event));
        }

        out
    }

    fn translate_pointer_cancel(
        &mut self,
        raw: &RawPointerCancel,
        arena: &mut UiArena,
    ) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        let position = raw
            .position
            .or_else(|| {
                self.active_drags
                    .get(&raw.pointer_id)
                    .map(|drag| drag.current_position)
            })
            .or_else(|| {
                self.active_presses
                    .values()
                    .find(|press| press.pointer_id == raw.pointer_id)
                    .map(|press| press.current_position)
            })
            .unwrap_or_else(Point::zero);

        let pointer = self.make_pointer_snapshot(
            raw.pointer_id,
            None,
            PointerButtons::default(),
            position,
            None,
            arena,
        );

        if let Some(drag) = self.active_drags.remove(&raw.pointer_id) {
            let meta = self.make_meta(
                raw.timestamp,
                drag.source,
                drag.source,
                EventPhase::Target,
                EventSource::Pointer,
                raw.modifiers,
            );

            out.push(SemanticEvent::DragCancel(DragEvent {
                meta,
                drag_id: drag.drag_id,
                pointer,
                source: drag.source,
                over: None,
                start_position: drag.start_position,
                previous_position: drag.previous_position,
                current_position: position,
                delta: (position - drag.previous_position).into(),
                total_delta: (position - drag.start_position).into(),
                duration: Some(raw.timestamp.duration_since(drag.started_at)),
                cancel_reason: Some(DragCancelReason::PointerCaptureLost),
            }));
        }

        let keys: Vec<_> = self
            .active_presses
            .keys()
            .filter(|(pointer_id, _)| *pointer_id == raw.pointer_id)
            .copied()
            .collect();

        for key in keys {
            if let Some(press) = self.active_presses.remove(&key) {
                let meta = self.make_meta(
                    raw.timestamp,
                    press.target,
                    press.target,
                    EventPhase::Target,
                    EventSource::Pointer,
                    raw.modifiers,
                );

                out.push(SemanticEvent::PressCancel(PressEvent {
                    meta,
                    press_id: press.press_id,
                    pointer,
                    press_target: press.target,
                    start_position: press.start_position,
                    current_position: position,
                    delta: (position - press.start_position).into(),
                    duration: Some(raw.timestamp.duration_since(press.started_at)),
                    cancel_reason: Some(PressCancelReason::PointerCaptureLost),
                }));
            }
        }

        out
    }

    fn translate_wheel(&mut self, raw: &RawWheel, arena: &mut UiArena) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        let Some(hit) = arena.hit_test(raw.position) else {
            return out;
        };

        let mut remaining = normalize_scroll_delta(raw.delta);
        let mut cursor = Some(hit);

        while let Some(node) = cursor {
            cursor = parent_of(arena, node);

            let Some((offset_before, offset_after, consumed)) =
                consume_scroll_delta(arena, node, remaining)
            else {
                continue;
            };

            let remaining_after =
                Translation::new(remaining.x - consumed.x, remaining.y - consumed.y);
            let meta = self.make_meta(
                raw.timestamp,
                node,
                node,
                EventPhase::Target,
                EventSource::Scroll,
                raw.modifiers,
            );

            out.push(SemanticEvent::Scroll(ScrollEvent {
                meta,
                source: ScrollSource::Wheel,
                phase: if raw.is_inertial {
                    ScrollPhase::Momentum
                } else {
                    ScrollPhase::Move
                },
                delta: raw.delta,
                pixel_delta: remaining,
                scroll_target: node,
                offset_before,
                offset_after,
                consumed_delta: consumed,
                remaining_delta: remaining_after,
                is_inertial: raw.is_inertial,
                pointer: raw.pointer_id.map(|pointer_id| {
                    self.make_pointer_snapshot(
                        pointer_id,
                        None,
                        PointerButtons::default(),
                        raw.position,
                        Some(hit),
                        arena,
                    )
                }),
            }));

            remaining = remaining_after;
            if remaining.is_zero() {
                break;
            }
        }

        out
    }

    fn translate_key_down(&mut self, raw: &RawKeyboard, arena: &mut UiArena) -> Vec<SemanticEvent> {
        let mut out = Vec::new();

        match raw.named_key {
            Some(NamedKey::Tab) => {
                let next = next_focusable(arena, arena.focused_node(), raw.modifiers.shift);
                self.change_focus(
                    arena,
                    next,
                    FocusReason::Keyboard,
                    EventSource::Keyboard,
                    raw.timestamp,
                    raw.modifiers,
                    &mut out,
                );
            }
            Some(NamedKey::Enter) | Some(NamedKey::Space) => {
                self.push_keyboard_click(arena.focused_node(), raw, &mut out)
            }
            Some(NamedKey::ContextMenu) => {
                if let Some(target) = arena.focused_node() {
                    let position = node_center(arena, target);
                    let meta = self.make_meta(
                        raw.timestamp,
                        target,
                        target,
                        EventPhase::Target,
                        EventSource::Keyboard,
                        raw.modifiers,
                    );

                    out.push(SemanticEvent::ContextMenu(ContextMenuEvent {
                        meta,
                        trigger: ContextMenuTrigger::Keyboard,
                        pointer: None,
                        position,
                    }));
                }
            }
            Some(NamedKey::Escape) => {
                self.cancel_active_drags(
                    DragCancelReason::EscapePressed,
                    raw.timestamp,
                    raw.modifiers,
                    arena,
                    &mut out,
                );
            }
            _ => {}
        }

        out
    }

    fn translate_context_menu(
        &mut self,
        raw: &RawContextMenu,
        arena: &mut UiArena,
    ) -> Vec<SemanticEvent> {
        let target = match raw.position {
            Some(position) => arena.hit_test(position),
            None => arena.focused_node(),
        };

        let Some(target) = target else {
            return Vec::new();
        };

        let meta = self.make_meta(
            raw.timestamp,
            target,
            target,
            EventPhase::Target,
            EventSource::Pointer,
            raw.modifiers,
        );

        vec![SemanticEvent::ContextMenu(ContextMenuEvent {
            meta,
            trigger: raw.trigger,
            pointer: raw.pointer,
            position: raw.position.or_else(|| node_center(arena, target)),
        })]
    }

    fn translate_window_blur(
        &mut self,
        raw: &RawWindowEvent,
        arena: &mut UiArena,
    ) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        self.cancel_active_drags(
            DragCancelReason::WindowBlur,
            raw.timestamp,
            raw.modifiers,
            arena,
            &mut out,
        );

        let presses: Vec<_> = self
            .active_presses
            .drain()
            .map(|(_, press)| press)
            .collect();
        for press in presses {
            let pointer = self.synthetic_pointer_snapshot(press.pointer_id, press.current_position);
            let meta = self.make_meta(
                raw.timestamp,
                press.target,
                press.target,
                EventPhase::Target,
                EventSource::Pointer,
                raw.modifiers,
            );

            out.push(SemanticEvent::PressCancel(PressEvent {
                meta,
                press_id: press.press_id,
                pointer,
                press_target: press.target,
                start_position: press.start_position,
                current_position: press.current_position,
                delta: (press.current_position - press.start_position).into(),
                duration: Some(raw.timestamp.duration_since(press.started_at)),
                cancel_reason: Some(PressCancelReason::WindowBlur),
            }));
        }

        self.change_focus(
            arena,
            None,
            FocusReason::Window,
            EventSource::Window,
            raw.timestamp,
            raw.modifiers,
            &mut out,
        );
        self.hover_paths.clear();
        self.pointer_capture.clear();

        out
    }

    #[inline(always)]
    fn resolve_pointer_target(
        &self,
        pointer_id: XuiPointerId,
        position: Point,
        arena: &UiArena,
    ) -> Option<NodeId> {
        self.pointer_capture
            .get(&pointer_id)
            .copied()
            .filter(|target| arena.contains(*target))
            .or_else(|| arena.hit_test(position))
    }

    fn make_pointer_snapshot(
        &self,
        pointer_id: XuiPointerId,
        button: Option<PointerButton>,
        buttons: PointerButtons,
        position: Point,
        target: Option<NodeId>,
        arena: &UiArena,
    ) -> PointerSnapshot {
        let target_local = target
            .map(|node| arena.to_local(node, position))
            .flatten()
            .unwrap_or(position);

        PointerSnapshot {
            pointer_id,
            button,
            buttons,
            coords: PointerCoords {
                window: position,
                viewport: position,
                target_local,
                current_local: target_local,
            },
            is_primary: pointer_id == XuiPointerId::new(0),
            tilt_x: None,
            tilt_y: None,
        }
    }

    fn synthetic_pointer_snapshot(
        &self,
        pointer_id: XuiPointerId,
        position: Point,
    ) -> PointerSnapshot {
        PointerSnapshot {
            pointer_id,
            button: None,
            buttons: PointerButtons::default(),
            coords: PointerCoords {
                window: position,
                viewport: position,
                target_local: position,
                current_local: position,
            },
            is_primary: pointer_id == XuiPointerId::new(0),
            tilt_x: None,
            tilt_y: None,
        }
    }

    fn update_hover(
        &mut self,
        pointer_id: XuiPointerId,
        new_target: Option<NodeId>,
        pointer: PointerSnapshot,
        timestamp: Instant,
        modifiers: Modifiers,
        arena: &UiArena,
        out: &mut Vec<SemanticEvent>,
    ) {
        let old_path = self
            .hover_paths
            .get(&pointer_id)
            .cloned()
            .unwrap_or_default();
        let new_path = new_target
            .map(|target| arena.event_path(target))
            .unwrap_or_default();

        if old_path == new_path {
            return;
        }

        let common = common_prefix_len(&old_path, &new_path);
        let left: Vec<NodeId> = old_path[common..].iter().rev().copied().collect();
        let entered: Vec<NodeId> = new_path[common..].iter().copied().collect();
        let old_target = old_path.last().copied();
        let new_target = new_path.last().copied();

        for node in &left {
            let meta = self.make_meta(
                timestamp,
                *node,
                *node,
                EventPhase::Target,
                EventSource::Pointer,
                modifiers,
            );
            out.push(SemanticEvent::HoverLeave(HoverEvent {
                meta,
                pointer,
                related_target: new_target,
            }));
        }

        for node in &entered {
            let meta = self.make_meta(
                timestamp,
                *node,
                *node,
                EventPhase::Target,
                EventSource::Pointer,
                modifiers,
            );
            out.push(SemanticEvent::HoverEnter(HoverEvent {
                meta,
                pointer,
                related_target: old_target,
            }));
        }

        if let Some(target) = new_target.or(old_target) {
            let meta = self.make_meta(
                timestamp,
                target,
                target,
                EventPhase::Target,
                EventSource::Pointer,
                modifiers,
            );
            out.push(SemanticEvent::HoverChange(HoverChangeEvent {
                meta,
                pointer,
                old_target,
                new_target,
                entered,
                left,
            }));
        }

        if new_path.is_empty() {
            self.hover_paths.remove(&pointer_id);
        } else {
            self.hover_paths.insert(pointer_id, new_path);
        }
    }

    fn update_press_or_drag_on_move(
        &mut self,
        pointer_id: XuiPointerId,
        pointer: PointerSnapshot,
        timestamp: Instant,
        modifiers: Modifiers,
        arena: &UiArena,
        out: &mut Vec<SemanticEvent>,
    ) {
        if self.active_drags.contains_key(&pointer_id) {
            let event_id = self.alloc_event_id();
            let drag = self
                .active_drags
                .get_mut(&pointer_id)
                .expect("drag existence checked before update");
            let previous = drag.current_position;
            let current = pointer.coords.viewport;

            drag.previous_position = previous;
            drag.current_position = current;

            let meta = make_meta_with(
                event_id,
                timestamp,
                drag.source,
                drag.source,
                EventPhase::Target,
                EventSource::Pointer,
                modifiers,
            );

            out.push(SemanticEvent::DragMove(DragEvent {
                meta,
                drag_id: drag.drag_id,
                pointer,
                source: drag.source,
                over: arena.hit_test(current),
                start_position: drag.start_position,
                previous_position: previous,
                current_position: current,
                delta: (current - previous).into(),
                total_delta: (current - drag.start_position).into(),
                duration: None,
                cancel_reason: None,
            }));

            return;
        }

        let key = self
            .active_presses
            .keys()
            .find(|(active_pointer_id, _)| *active_pointer_id == pointer_id)
            .copied();

        let Some(key) = key else {
            return;
        };

        let current = pointer.coords.viewport;
        let Some(press) = self.active_presses.get_mut(&key) else {
            return;
        };

        press.current_position = current;
        if press.became_drag
            || distance(current, press.start_position) < self.config.drag_threshold
            || !is_draggable(arena, press.target)
        {
            return;
        }

        press.became_drag = true;
        let source = press.target;
        let start_position = press.start_position;
        let drag_id = self.alloc_drag_id();

        self.active_drags.insert(
            pointer_id,
            ActiveDrag {
                drag_id,
                pointer_id,
                source,
                start_position,
                previous_position: start_position,
                current_position: current,
                started_at: timestamp,
            },
        );

        let meta = self.make_meta(
            timestamp,
            source,
            source,
            EventPhase::Target,
            EventSource::Pointer,
            modifiers,
        );

        out.push(SemanticEvent::DragStart(DragEvent {
            meta,
            drag_id,
            pointer,
            source,
            over: arena.hit_test(current),
            start_position,
            previous_position: start_position,
            current_position: current,
            delta: (current - start_position).into(),
            total_delta: (current - start_position).into(),
            duration: None,
            cancel_reason: None,
        }));
    }

    fn change_focus(
        &mut self,
        arena: &mut UiArena,
        new_focused: Option<NodeId>,
        reason: FocusReason,
        source: EventSource,
        timestamp: Instant,
        modifiers: Modifiers,
        out: &mut Vec<SemanticEvent>,
    ) {
        if new_focused.is_some_and(|target| !is_focusable(arena, target)) {
            return;
        }
        let Some(transition) = arena.focus_manager_mut().commit(new_focused, reason) else {
            return;
        };
        let old_focused = transition.old;
        let new_focused = transition.new;

        if let Some(old) = old_focused {
            let blur_meta =
                self.make_meta(timestamp, old, old, EventPhase::Target, source, modifiers);
            out.push(SemanticEvent::Blur(FocusEvent {
                meta: blur_meta,
                old_focused,
                new_focused,
                related_target: new_focused,
                reason,
            }));

            let focus_out_meta =
                self.make_meta(timestamp, old, old, EventPhase::Target, source, modifiers);
            out.push(SemanticEvent::FocusOut(FocusEvent {
                meta: focus_out_meta,
                old_focused,
                new_focused,
                related_target: new_focused,
                reason,
            }));
        }

        if let Some(new) = new_focused {
            let focus_in_meta =
                self.make_meta(timestamp, new, new, EventPhase::Target, source, modifiers);
            out.push(SemanticEvent::FocusIn(FocusEvent {
                meta: focus_in_meta,
                old_focused,
                new_focused,
                related_target: old_focused,
                reason,
            }));

            let focus_meta =
                self.make_meta(timestamp, new, new, EventPhase::Target, source, modifiers);
            out.push(SemanticEvent::Focus(FocusEvent {
                meta: focus_meta,
                old_focused,
                new_focused,
                related_target: old_focused,
                reason,
            }));
        }
    }

    fn push_keyboard_click(
        &mut self,
        focused: Option<NodeId>,
        raw: &RawKeyboard,
        out: &mut Vec<SemanticEvent>,
    ) {
        if let Some(target) = focused {
            let meta = self.make_meta(
                raw.timestamp,
                target,
                target,
                EventPhase::Target,
                EventSource::Keyboard,
                raw.modifiers,
            );
            out.push(SemanticEvent::Click(ClickEvent {
                meta,
                activation: ActivationKind::Keyboard,
                pointer: None,
                button: None,
                click_count: 1,
                press_target: None,
                release_target: None,
                duration: None,
            }));
        }
    }

    fn cancel_active_drags(
        &mut self,
        reason: DragCancelReason,
        timestamp: Instant,
        modifiers: Modifiers,
        arena: &UiArena,
        out: &mut Vec<SemanticEvent>,
    ) {
        let drags: Vec<_> = self.active_drags.drain().map(|(_, drag)| drag).collect();
        for drag in drags {
            let pointer = self.make_pointer_snapshot(
                drag.pointer_id,
                None,
                PointerButtons::default(),
                drag.current_position,
                arena.hit_test(drag.current_position),
                arena,
            );
            let meta = self.make_meta(
                timestamp,
                drag.source,
                drag.source,
                EventPhase::Target,
                EventSource::Pointer,
                modifiers,
            );
            out.push(SemanticEvent::DragCancel(DragEvent {
                meta,
                drag_id: drag.drag_id,
                pointer,
                source: drag.source,
                over: None,
                start_position: drag.start_position,
                previous_position: drag.previous_position,
                current_position: drag.current_position,
                delta: (drag.current_position - drag.previous_position).into(),
                total_delta: (drag.current_position - drag.start_position).into(),
                duration: Some(timestamp.duration_since(drag.started_at)),
                cancel_reason: Some(reason),
            }));
        }
    }

    fn compute_click_count(
        &mut self,
        target: NodeId,
        button: PointerButton,
        position: Point,
        timestamp: Instant,
    ) -> u8 {
        let click_count = self
            .last_click
            .as_ref()
            .filter(|record| record.target == target)
            .filter(|record| record.button == button)
            .filter(|record| {
                timestamp.duration_since(record.timestamp) <= self.config.double_click_timeout
            })
            .filter(|record| {
                distance(record.position, position) <= self.config.double_click_max_distance
            })
            .map(|record| record.click_count.saturating_add(1))
            .unwrap_or(1);

        self.last_click = Some(ClickRecord {
            target,
            button,
            position,
            timestamp,
            click_count,
        });

        click_count
    }

    fn make_meta(
        &mut self,
        timestamp: Instant,
        target: NodeId,
        current_target: NodeId,
        phase: EventPhase,
        source: EventSource,
        modifiers: Modifiers,
    ) -> EventMeta {
        make_meta_with(
            self.alloc_event_id(),
            timestamp,
            target,
            current_target,
            phase,
            source,
            modifiers,
        )
    }
}

fn make_meta_with(
    id: EventId,
    timestamp: Instant,
    target: NodeId,
    current_target: NodeId,
    phase: EventPhase,
    source: EventSource,
    modifiers: Modifiers,
) -> EventMeta {
    EventMeta {
        id,
        timestamp,
        target,
        current_target,
        phase,
        source,
        modifiers,
    }
}

fn common_prefix_len(a: &[NodeId], b: &[NodeId]) -> usize {
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    i
}

fn distance(a: Point, b: Point) -> f32 {
    a.distance_to(b)
}

fn normalize_scroll_delta(delta: ScrollDelta) -> Translation {
    match delta {
        ScrollDelta::Pixels(delta) => delta,
        ScrollDelta::Lines(delta) => Translation::new(delta.x * 16.0, delta.y * 16.0),
        ScrollDelta::Pages(delta) => Translation::new(delta.x * 800.0, delta.y * 800.0),
    }
}

fn parent_of(arena: &UiArena, node: NodeId) -> Option<NodeId> {
    arena.node(node).and_then(|node| node.parent)
}

fn nearest_focusable_ancestor(arena: &UiArena, target: NodeId) -> Option<NodeId> {
    let mut cursor = Some(target);
    while let Some(node_id) = cursor {
        if is_focusable(arena, node_id) {
            return Some(node_id);
        }
        cursor = parent_of(arena, node_id);
    }
    None
}

fn is_focusable(arena: &UiArena, node_id: NodeId) -> bool {
    arena.node(node_id).is_some_and(|node| node.is_focusable())
}

fn is_draggable(arena: &UiArena, node_id: NodeId) -> bool {
    let Some(node) = arena.node(node_id) else {
        return false;
    };

    node.event_callbacks.has_drag_callbacks()
}

fn next_focusable(arena: &UiArena, focused: Option<NodeId>, reverse: bool) -> Option<NodeId> {
    let focusables = focusable_nodes(arena);
    if focusables.is_empty() {
        return None;
    }

    let Some(focused) = focused else {
        return if reverse {
            focusables.last().copied()
        } else {
            focusables.first().copied()
        };
    };

    let Some(current) = focusables.iter().position(|node| *node == focused) else {
        return if reverse {
            focusables.last().copied()
        } else {
            focusables.first().copied()
        };
    };
    let next = if reverse {
        current
            .checked_sub(1)
            .unwrap_or_else(|| focusables.len().saturating_sub(1))
    } else {
        (current + 1) % focusables.len()
    };

    focusables.get(next).copied()
}

fn focusable_nodes(arena: &UiArena) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![arena.root()];
    let mut document_order = 0usize;

    while let Some(node_id) = stack.pop() {
        if let Some(node) = arena.node(node_id) {
            if node.is_sequentially_focusable() {
                out.push((node_id, node.focus.tab_index.unwrap_or(0), document_order));
            }
            document_order += 1;

            stack.extend(node.children.iter().rev().copied());
        }
    }

    out.sort_by_key(|(_, tab_index, order)| {
        if *tab_index > 0 {
            (0u8, *tab_index, *order)
        } else {
            (1u8, 0, *order)
        }
    });
    out.into_iter().map(|(node, _, _)| node).collect()
}

fn node_center(arena: &UiArena, node_id: NodeId) -> Option<Point> {
    arena.node(node_id).map(|node| {
        Point::new(
            node.layout.x + node.layout.width * 0.5,
            node.layout.y + node.layout.height * 0.5,
        )
    })
}

fn consume_scroll_delta(
    arena: &mut UiArena,
    node_id: NodeId,
    delta: Translation,
) -> Option<(Translation, Translation, Translation)> {
    let node = arena.node(node_id)?;
    let direction = node.target_style.scroll.direction;
    if !direction.is_scrollable() {
        return None;
    }

    let max_x = if direction.allows_horizontal() {
        (node.content_size.width - node.layout.width).max(0.0)
    } else {
        0.0
    };
    let max_y = if direction.allows_vertical() {
        (node.content_size.height - node.layout.height).max(0.0)
    } else {
        0.0
    };
    if max_x <= 0.0 && max_y <= 0.0 {
        return None;
    }

    let offset_before = node.scroll_offset;
    let scroll_delta = ergonomic_scroll_delta(delta);
    let offset_after = Point::new(
        (offset_before.x - scroll_delta.x).clamp(0.0, max_x),
        (offset_before.y - scroll_delta.y).clamp(0.0, max_y),
    );
    if offset_after == offset_before {
        return None;
    }

    let consumed = Translation::new(
        offset_before.x - offset_after.x,
        offset_before.y - offset_after.y,
    );
    let node = arena
        .node_mut(node_id)
        .expect("node was checked before scroll mutation");
    node.scroll_offset = offset_after;
    arena.mark_dirty(node_id, WidgetUpdateFlags::PAINT_OUTPUT);

    Some((offset_before.into(), offset_after.into(), consumed))
}

fn ergonomic_scroll_delta(delta: Translation) -> Translation {
    let magnitude = (delta.x * delta.x + delta.y * delta.y).sqrt();
    let factor = scroll_acceleration_factor(magnitude);
    Translation::new(delta.x * factor, delta.y * factor)
}

fn scroll_acceleration_factor(magnitude: f32) -> f32 {
    const MIN_ACCELERATION_MAGNITUDE: f32 = 80.0;
    const FULL_ACCELERATION_MAGNITUDE: f32 = 160.0;
    const MAX_ACCELERATION: f32 = 2.75;

    if magnitude <= MIN_ACCELERATION_MAGNITUDE {
        return 1.0;
    }

    let progress = ((magnitude - MIN_ACCELERATION_MAGNITUDE)
        / (FULL_ACCELERATION_MAGNITUDE - MIN_ACCELERATION_MAGNITUDE))
        .clamp(0.0, 1.0);
    1.0 + (MAX_ACCELERATION - 1.0) * progress
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{WidgetI, container};

    fn append(arena: &mut UiArena, widget: crate::widgets::ContainerWidget) -> NodeId {
        let widget = WidgetI::new(widget);
        let props_hash = widget.props_hash();
        let handlers = widget.take_event_handlers();
        let node = arena.create_node(None, props_hash, widget, handlers);
        arena.append_child(arena.root(), node);
        node
    }

    #[test]
    fn sequential_focus_uses_tab_index_and_skips_negative_values() {
        let mut arena = UiArena::new();
        let zero = append(&mut arena, container().tab_index(0));
        let second = append(&mut arena, container().tab_index(2));
        let first = append(&mut arena, container().tab_index(1));
        let negative = append(&mut arena, container().tab_index(-1));
        let disabled = append(&mut arena, container().focusable(false).tab_index(0));

        assert_eq!(focusable_nodes(&arena), vec![first, second, zero]);
        assert!(is_focusable(&arena, negative));
        assert!(!is_focusable(&arena, disabled));
    }

    #[test]
    fn tab_from_programmatic_only_focus_enters_at_order_boundary() {
        let mut arena = UiArena::new();
        let first = append(&mut arena, container().tab_index(0));
        let second = append(&mut arena, container().tab_index(0));
        let programmatic = append(&mut arena, container().tab_index(-1));

        assert_eq!(
            next_focusable(&arena, Some(programmatic), false),
            Some(first)
        );
        assert_eq!(
            next_focusable(&arena, Some(programmatic), true),
            Some(second)
        );
    }
}
