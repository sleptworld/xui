use super::EventContext;
use crate::focus::FocusHandle;
use slotmap::{Key as SlotMapKey, SlotMap, new_key_type};
use xui_interface::events::semantic::{
    ClickEvent, CommandEvent, ContextMenuEvent, DragEvent, FocusEvent, HoverChangeEvent,
    HoverEvent, PressEvent, ScrollEvent, SemanticEvent,
};
use xui_interface::{AccessibilityProperties, EventPhase, EventResult, FocusProperties};

pub type TypedEventHandler<E> = Box<dyn for<'a> FnMut(&E, &mut EventContext<'a>) -> EventResult>;

pub type SemanticEventHandler = TypedEventHandler<SemanticEvent>;
pub type HoverEventHandler = TypedEventHandler<HoverEvent>;
pub type HoverChangeEventHandler = TypedEventHandler<HoverChangeEvent>;
pub type PressEventHandler = TypedEventHandler<PressEvent>;
pub type ClickEventHandler = TypedEventHandler<ClickEvent>;
pub type ContextMenuEventHandler = TypedEventHandler<ContextMenuEvent>;
pub type FocusEventHandler = TypedEventHandler<FocusEvent>;
pub type DragEventHandler = TypedEventHandler<DragEvent>;
pub type ScrollEventHandler = TypedEventHandler<ScrollEvent>;
pub type CommandEventHandler = TypedEventHandler<CommandEvent>;

new_key_type! {
    pub struct SemanticEventHandlerId;
    pub struct HoverEventHandlerId;
    pub struct HoverChangeEventHandlerId;
    pub struct PressEventHandlerId;
    pub struct ClickEventHandlerId;
    pub struct ContextMenuEventHandlerId;
    pub struct FocusEventHandlerId;
    pub struct DragEventHandlerId;
    pub struct ScrollEventHandlerId;
    pub struct CommandEventHandlerId;
}

#[derive(Default)]
pub struct EventHandlers {
    pub focus: FocusProperties,
    pub focus_handle: Option<FocusHandle>,
    pub accessibility: AccessibilityProperties,
    pub shortcuts: Vec<xui_interface::ShortcutBinding>,
    pub on_command: Option<CommandEventHandler>,
    pub on_event: Option<SemanticEventHandler>,
    pub on_event_capture: Option<SemanticEventHandler>,
    pub on_hover_enter: Option<HoverEventHandler>,
    pub on_hover_leave: Option<HoverEventHandler>,
    pub on_hover_change: Option<HoverChangeEventHandler>,
    pub on_press_start: Option<PressEventHandler>,
    pub on_press_start_capture: Option<PressEventHandler>,
    pub on_press_end: Option<PressEventHandler>,
    pub on_press_end_capture: Option<PressEventHandler>,
    pub on_press_cancel: Option<PressEventHandler>,
    pub on_press_cancel_capture: Option<PressEventHandler>,
    pub on_click: Option<ClickEventHandler>,
    pub on_click_capture: Option<ClickEventHandler>,
    pub on_double_click: Option<ClickEventHandler>,
    pub on_double_click_capture: Option<ClickEventHandler>,
    pub on_context_menu: Option<ContextMenuEventHandler>,
    pub on_context_menu_capture: Option<ContextMenuEventHandler>,
    pub on_focus: Option<FocusEventHandler>,
    pub on_blur: Option<FocusEventHandler>,
    pub on_focus_in: Option<FocusEventHandler>,
    pub on_focus_in_capture: Option<FocusEventHandler>,
    pub on_focus_out: Option<FocusEventHandler>,
    pub on_focus_out_capture: Option<FocusEventHandler>,
    pub on_drag_start: Option<DragEventHandler>,
    pub on_drag_start_capture: Option<DragEventHandler>,
    pub on_drag_move: Option<DragEventHandler>,
    pub on_drag_move_capture: Option<DragEventHandler>,
    pub on_drag_end: Option<DragEventHandler>,
    pub on_drag_end_capture: Option<DragEventHandler>,
    pub on_drag_cancel: Option<DragEventHandler>,
    pub on_drag_cancel_capture: Option<DragEventHandler>,
    pub on_scroll: Option<ScrollEventHandler>,
    pub on_scroll_capture: Option<ScrollEventHandler>,
}

impl std::fmt::Debug for EventHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventHandlers")
            .field("focus", &self.focus)
            .field("focus_handle", &self.focus_handle)
            .field("accessibility", &self.accessibility)
            .field("shortcuts", &self.shortcuts)
            .field("on_command", &self.on_command.is_some())
            .field("on_event", &self.on_event.is_some())
            .field("on_event_capture", &self.on_event_capture.is_some())
            .field("on_hover_enter", &self.on_hover_enter.is_some())
            .field("on_hover_leave", &self.on_hover_leave.is_some())
            .field("on_hover_change", &self.on_hover_change.is_some())
            .field("on_press_start", &self.on_press_start.is_some())
            .field(
                "on_press_start_capture",
                &self.on_press_start_capture.is_some(),
            )
            .field("on_press_end", &self.on_press_end.is_some())
            .field("on_press_end_capture", &self.on_press_end_capture.is_some())
            .field("on_press_cancel", &self.on_press_cancel.is_some())
            .field(
                "on_press_cancel_capture",
                &self.on_press_cancel_capture.is_some(),
            )
            .field("on_click", &self.on_click.is_some())
            .field("on_click_capture", &self.on_click_capture.is_some())
            .field("on_double_click", &self.on_double_click.is_some())
            .field(
                "on_double_click_capture",
                &self.on_double_click_capture.is_some(),
            )
            .field("on_context_menu", &self.on_context_menu.is_some())
            .field(
                "on_context_menu_capture",
                &self.on_context_menu_capture.is_some(),
            )
            .field("on_focus", &self.on_focus.is_some())
            .field("on_blur", &self.on_blur.is_some())
            .field("on_focus_in", &self.on_focus_in.is_some())
            .field("on_focus_in_capture", &self.on_focus_in_capture.is_some())
            .field("on_focus_out", &self.on_focus_out.is_some())
            .field("on_focus_out_capture", &self.on_focus_out_capture.is_some())
            .field("on_drag_start", &self.on_drag_start.is_some())
            .field(
                "on_drag_start_capture",
                &self.on_drag_start_capture.is_some(),
            )
            .field("on_drag_move", &self.on_drag_move.is_some())
            .field("on_drag_move_capture", &self.on_drag_move_capture.is_some())
            .field("on_drag_end", &self.on_drag_end.is_some())
            .field("on_drag_end_capture", &self.on_drag_end_capture.is_some())
            .field("on_drag_cancel", &self.on_drag_cancel.is_some())
            .field(
                "on_drag_cancel_capture",
                &self.on_drag_cancel_capture.is_some(),
            )
            .field("on_scroll", &self.on_scroll.is_some())
            .field("on_scroll_capture", &self.on_scroll_capture.is_some())
            .finish()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CallbackHandleSet {
    pub on_command: Option<CommandEventHandlerId>,
    pub on_event: Option<SemanticEventHandlerId>,
    pub on_event_capture: Option<SemanticEventHandlerId>,
    pub on_hover_enter: Option<HoverEventHandlerId>,
    pub on_hover_leave: Option<HoverEventHandlerId>,
    pub on_hover_change: Option<HoverChangeEventHandlerId>,
    pub on_press_start: Option<PressEventHandlerId>,
    pub on_press_start_capture: Option<PressEventHandlerId>,
    pub on_press_end: Option<PressEventHandlerId>,
    pub on_press_end_capture: Option<PressEventHandlerId>,
    pub on_press_cancel: Option<PressEventHandlerId>,
    pub on_press_cancel_capture: Option<PressEventHandlerId>,
    pub on_click: Option<ClickEventHandlerId>,
    pub on_click_capture: Option<ClickEventHandlerId>,
    pub on_double_click: Option<ClickEventHandlerId>,
    pub on_double_click_capture: Option<ClickEventHandlerId>,
    pub on_context_menu: Option<ContextMenuEventHandlerId>,
    pub on_context_menu_capture: Option<ContextMenuEventHandlerId>,
    pub on_focus: Option<FocusEventHandlerId>,
    pub on_blur: Option<FocusEventHandlerId>,
    pub on_focus_in: Option<FocusEventHandlerId>,
    pub on_focus_in_capture: Option<FocusEventHandlerId>,
    pub on_focus_out: Option<FocusEventHandlerId>,
    pub on_focus_out_capture: Option<FocusEventHandlerId>,
    pub on_drag_start: Option<DragEventHandlerId>,
    pub on_drag_start_capture: Option<DragEventHandlerId>,
    pub on_drag_move: Option<DragEventHandlerId>,
    pub on_drag_move_capture: Option<DragEventHandlerId>,
    pub on_drag_end: Option<DragEventHandlerId>,
    pub on_drag_end_capture: Option<DragEventHandlerId>,
    pub on_drag_cancel: Option<DragEventHandlerId>,
    pub on_drag_cancel_capture: Option<DragEventHandlerId>,
    pub on_scroll: Option<ScrollEventHandlerId>,
    pub on_scroll_capture: Option<ScrollEventHandlerId>,
}

impl CallbackHandleSet {
    pub fn has_focus_callbacks(self) -> bool {
        self.on_focus.is_some()
            || self.on_blur.is_some()
            || self.on_focus_in.is_some()
            || self.on_focus_in_capture.is_some()
            || self.on_focus_out.is_some()
            || self.on_focus_out_capture.is_some()
    }

    pub fn has_drag_callbacks(self) -> bool {
        self.on_drag_start.is_some()
            || self.on_drag_start_capture.is_some()
            || self.on_drag_move.is_some()
            || self.on_drag_move_capture.is_some()
            || self.on_drag_end.is_some()
            || self.on_drag_end_capture.is_some()
            || self.on_drag_cancel.is_some()
            || self.on_drag_cancel_capture.is_some()
    }
}

#[derive(Default)]
pub(crate) struct CallbackStore {
    on_command: SlotMap<CommandEventHandlerId, CommandEventHandler>,
    on_event: SlotMap<SemanticEventHandlerId, SemanticEventHandler>,
    on_event_capture: SlotMap<SemanticEventHandlerId, SemanticEventHandler>,
    on_hover_enter: SlotMap<HoverEventHandlerId, HoverEventHandler>,
    on_hover_leave: SlotMap<HoverEventHandlerId, HoverEventHandler>,
    on_hover_change: SlotMap<HoverChangeEventHandlerId, HoverChangeEventHandler>,
    on_press_start: SlotMap<PressEventHandlerId, PressEventHandler>,
    on_press_start_capture: SlotMap<PressEventHandlerId, PressEventHandler>,
    on_press_end: SlotMap<PressEventHandlerId, PressEventHandler>,
    on_press_end_capture: SlotMap<PressEventHandlerId, PressEventHandler>,
    on_press_cancel: SlotMap<PressEventHandlerId, PressEventHandler>,
    on_press_cancel_capture: SlotMap<PressEventHandlerId, PressEventHandler>,
    on_click: SlotMap<ClickEventHandlerId, ClickEventHandler>,
    on_click_capture: SlotMap<ClickEventHandlerId, ClickEventHandler>,
    on_double_click: SlotMap<ClickEventHandlerId, ClickEventHandler>,
    on_double_click_capture: SlotMap<ClickEventHandlerId, ClickEventHandler>,
    on_context_menu: SlotMap<ContextMenuEventHandlerId, ContextMenuEventHandler>,
    on_context_menu_capture: SlotMap<ContextMenuEventHandlerId, ContextMenuEventHandler>,
    on_focus: SlotMap<FocusEventHandlerId, FocusEventHandler>,
    on_blur: SlotMap<FocusEventHandlerId, FocusEventHandler>,
    on_focus_in: SlotMap<FocusEventHandlerId, FocusEventHandler>,
    on_focus_in_capture: SlotMap<FocusEventHandlerId, FocusEventHandler>,
    on_focus_out: SlotMap<FocusEventHandlerId, FocusEventHandler>,
    on_focus_out_capture: SlotMap<FocusEventHandlerId, FocusEventHandler>,
    on_drag_start: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_drag_start_capture: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_drag_move: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_drag_move_capture: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_drag_end: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_drag_end_capture: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_drag_cancel: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_drag_cancel_capture: SlotMap<DragEventHandlerId, DragEventHandler>,
    on_scroll: SlotMap<ScrollEventHandlerId, ScrollEventHandler>,
    on_scroll_capture: SlotMap<ScrollEventHandlerId, ScrollEventHandler>,
}

impl CallbackStore {
    pub(crate) fn update_set(
        &mut self,
        current: CallbackHandleSet,
        handlers: EventHandlers,
    ) -> CallbackHandleSet {
        CallbackHandleSet {
            on_command: update_handler(
                &mut self.on_command,
                current.on_command,
                handlers.on_command,
            ),
            on_event: update_handler(&mut self.on_event, current.on_event, handlers.on_event),
            on_event_capture: update_handler(
                &mut self.on_event_capture,
                current.on_event_capture,
                handlers.on_event_capture,
            ),
            on_hover_enter: update_handler(
                &mut self.on_hover_enter,
                current.on_hover_enter,
                handlers.on_hover_enter,
            ),
            on_hover_leave: update_handler(
                &mut self.on_hover_leave,
                current.on_hover_leave,
                handlers.on_hover_leave,
            ),
            on_hover_change: update_handler(
                &mut self.on_hover_change,
                current.on_hover_change,
                handlers.on_hover_change,
            ),
            on_press_start: update_handler(
                &mut self.on_press_start,
                current.on_press_start,
                handlers.on_press_start,
            ),
            on_press_start_capture: update_handler(
                &mut self.on_press_start_capture,
                current.on_press_start_capture,
                handlers.on_press_start_capture,
            ),
            on_press_end: update_handler(
                &mut self.on_press_end,
                current.on_press_end,
                handlers.on_press_end,
            ),
            on_press_end_capture: update_handler(
                &mut self.on_press_end_capture,
                current.on_press_end_capture,
                handlers.on_press_end_capture,
            ),
            on_press_cancel: update_handler(
                &mut self.on_press_cancel,
                current.on_press_cancel,
                handlers.on_press_cancel,
            ),
            on_press_cancel_capture: update_handler(
                &mut self.on_press_cancel_capture,
                current.on_press_cancel_capture,
                handlers.on_press_cancel_capture,
            ),
            on_click: update_handler(&mut self.on_click, current.on_click, handlers.on_click),
            on_click_capture: update_handler(
                &mut self.on_click_capture,
                current.on_click_capture,
                handlers.on_click_capture,
            ),
            on_double_click: update_handler(
                &mut self.on_double_click,
                current.on_double_click,
                handlers.on_double_click,
            ),
            on_double_click_capture: update_handler(
                &mut self.on_double_click_capture,
                current.on_double_click_capture,
                handlers.on_double_click_capture,
            ),
            on_context_menu: update_handler(
                &mut self.on_context_menu,
                current.on_context_menu,
                handlers.on_context_menu,
            ),
            on_context_menu_capture: update_handler(
                &mut self.on_context_menu_capture,
                current.on_context_menu_capture,
                handlers.on_context_menu_capture,
            ),
            on_focus: update_handler(&mut self.on_focus, current.on_focus, handlers.on_focus),
            on_blur: update_handler(&mut self.on_blur, current.on_blur, handlers.on_blur),
            on_focus_in: update_handler(
                &mut self.on_focus_in,
                current.on_focus_in,
                handlers.on_focus_in,
            ),
            on_focus_in_capture: update_handler(
                &mut self.on_focus_in_capture,
                current.on_focus_in_capture,
                handlers.on_focus_in_capture,
            ),
            on_focus_out: update_handler(
                &mut self.on_focus_out,
                current.on_focus_out,
                handlers.on_focus_out,
            ),
            on_focus_out_capture: update_handler(
                &mut self.on_focus_out_capture,
                current.on_focus_out_capture,
                handlers.on_focus_out_capture,
            ),
            on_drag_start: update_handler(
                &mut self.on_drag_start,
                current.on_drag_start,
                handlers.on_drag_start,
            ),
            on_drag_start_capture: update_handler(
                &mut self.on_drag_start_capture,
                current.on_drag_start_capture,
                handlers.on_drag_start_capture,
            ),
            on_drag_move: update_handler(
                &mut self.on_drag_move,
                current.on_drag_move,
                handlers.on_drag_move,
            ),
            on_drag_move_capture: update_handler(
                &mut self.on_drag_move_capture,
                current.on_drag_move_capture,
                handlers.on_drag_move_capture,
            ),
            on_drag_end: update_handler(
                &mut self.on_drag_end,
                current.on_drag_end,
                handlers.on_drag_end,
            ),
            on_drag_end_capture: update_handler(
                &mut self.on_drag_end_capture,
                current.on_drag_end_capture,
                handlers.on_drag_end_capture,
            ),
            on_drag_cancel: update_handler(
                &mut self.on_drag_cancel,
                current.on_drag_cancel,
                handlers.on_drag_cancel,
            ),
            on_drag_cancel_capture: update_handler(
                &mut self.on_drag_cancel_capture,
                current.on_drag_cancel_capture,
                handlers.on_drag_cancel_capture,
            ),
            on_scroll: update_handler(&mut self.on_scroll, current.on_scroll, handlers.on_scroll),
            on_scroll_capture: update_handler(
                &mut self.on_scroll_capture,
                current.on_scroll_capture,
                handlers.on_scroll_capture,
            ),
        }
    }

    pub(crate) fn clear_set(&mut self, handlers: CallbackHandleSet) {
        remove_handler(&mut self.on_command, handlers.on_command);
        remove_handler(&mut self.on_event, handlers.on_event);
        remove_handler(&mut self.on_event_capture, handlers.on_event_capture);
        remove_handler(&mut self.on_hover_enter, handlers.on_hover_enter);
        remove_handler(&mut self.on_hover_leave, handlers.on_hover_leave);
        remove_handler(&mut self.on_hover_change, handlers.on_hover_change);
        remove_handler(&mut self.on_press_start, handlers.on_press_start);
        remove_handler(
            &mut self.on_press_start_capture,
            handlers.on_press_start_capture,
        );
        remove_handler(&mut self.on_press_end, handlers.on_press_end);
        remove_handler(
            &mut self.on_press_end_capture,
            handlers.on_press_end_capture,
        );
        remove_handler(&mut self.on_press_cancel, handlers.on_press_cancel);
        remove_handler(
            &mut self.on_press_cancel_capture,
            handlers.on_press_cancel_capture,
        );
        remove_handler(&mut self.on_click, handlers.on_click);
        remove_handler(&mut self.on_click_capture, handlers.on_click_capture);
        remove_handler(&mut self.on_double_click, handlers.on_double_click);
        remove_handler(
            &mut self.on_double_click_capture,
            handlers.on_double_click_capture,
        );
        remove_handler(&mut self.on_context_menu, handlers.on_context_menu);
        remove_handler(
            &mut self.on_context_menu_capture,
            handlers.on_context_menu_capture,
        );
        remove_handler(&mut self.on_focus, handlers.on_focus);
        remove_handler(&mut self.on_blur, handlers.on_blur);
        remove_handler(&mut self.on_focus_in, handlers.on_focus_in);
        remove_handler(&mut self.on_focus_in_capture, handlers.on_focus_in_capture);
        remove_handler(&mut self.on_focus_out, handlers.on_focus_out);
        remove_handler(
            &mut self.on_focus_out_capture,
            handlers.on_focus_out_capture,
        );
        remove_handler(&mut self.on_drag_start, handlers.on_drag_start);
        remove_handler(
            &mut self.on_drag_start_capture,
            handlers.on_drag_start_capture,
        );
        remove_handler(&mut self.on_drag_move, handlers.on_drag_move);
        remove_handler(
            &mut self.on_drag_move_capture,
            handlers.on_drag_move_capture,
        );
        remove_handler(&mut self.on_drag_end, handlers.on_drag_end);
        remove_handler(&mut self.on_drag_end_capture, handlers.on_drag_end_capture);
        remove_handler(&mut self.on_drag_cancel, handlers.on_drag_cancel);
        remove_handler(
            &mut self.on_drag_cancel_capture,
            handlers.on_drag_cancel_capture,
        );
        remove_handler(&mut self.on_scroll, handlers.on_scroll);
        remove_handler(&mut self.on_scroll_capture, handlers.on_scroll_capture);
    }

    pub(crate) fn dispatch_semantic(
        &mut self,
        handlers: CallbackHandleSet,
        event: &SemanticEvent,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        let is_capture = cx.phase == EventPhase::Capture;
        let generic_result = if is_capture {
            dispatch_handler(
                &mut self.on_event_capture,
                handlers.on_event_capture,
                event,
                cx,
            )
        } else {
            dispatch_handler(&mut self.on_event, handlers.on_event, event, cx)
        };
        if generic_result.is_consumed() {
            return EventResult::Consumed;
        }

        match event {
            SemanticEvent::Command(event) => {
                dispatch_handler(&mut self.on_command, handlers.on_command, event, cx)
            }
            SemanticEvent::HoverEnter(event) => {
                dispatch_handler(&mut self.on_hover_enter, handlers.on_hover_enter, event, cx)
            }
            SemanticEvent::HoverLeave(event) => {
                dispatch_handler(&mut self.on_hover_leave, handlers.on_hover_leave, event, cx)
            }
            SemanticEvent::HoverChange(event) => dispatch_handler(
                &mut self.on_hover_change,
                handlers.on_hover_change,
                event,
                cx,
            ),
            SemanticEvent::PressStart(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_press_start_capture,
                        handlers.on_press_start_capture,
                    )
                } else {
                    (&mut self.on_press_start, handlers.on_press_start)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::PressEnd(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_press_end_capture,
                        handlers.on_press_end_capture,
                    )
                } else {
                    (&mut self.on_press_end, handlers.on_press_end)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::PressCancel(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_press_cancel_capture,
                        handlers.on_press_cancel_capture,
                    )
                } else {
                    (&mut self.on_press_cancel, handlers.on_press_cancel)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::Click(event) => {
                let (store, handler) = if is_capture {
                    (&mut self.on_click_capture, handlers.on_click_capture)
                } else {
                    (&mut self.on_click, handlers.on_click)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::DoubleClick(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_double_click_capture,
                        handlers.on_double_click_capture,
                    )
                } else {
                    (&mut self.on_double_click, handlers.on_double_click)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::ContextMenu(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_context_menu_capture,
                        handlers.on_context_menu_capture,
                    )
                } else {
                    (&mut self.on_context_menu, handlers.on_context_menu)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::Focus(event) => {
                dispatch_handler(&mut self.on_focus, handlers.on_focus, event, cx)
            }
            SemanticEvent::Blur(event) => {
                dispatch_handler(&mut self.on_blur, handlers.on_blur, event, cx)
            }
            SemanticEvent::FocusIn(event) => {
                let (store, handler) = if is_capture {
                    (&mut self.on_focus_in_capture, handlers.on_focus_in_capture)
                } else {
                    (&mut self.on_focus_in, handlers.on_focus_in)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::FocusOut(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_focus_out_capture,
                        handlers.on_focus_out_capture,
                    )
                } else {
                    (&mut self.on_focus_out, handlers.on_focus_out)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::DragStart(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_drag_start_capture,
                        handlers.on_drag_start_capture,
                    )
                } else {
                    (&mut self.on_drag_start, handlers.on_drag_start)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::DragMove(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_drag_move_capture,
                        handlers.on_drag_move_capture,
                    )
                } else {
                    (&mut self.on_drag_move, handlers.on_drag_move)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::DragEnd(event) => {
                let (store, handler) = if is_capture {
                    (&mut self.on_drag_end_capture, handlers.on_drag_end_capture)
                } else {
                    (&mut self.on_drag_end, handlers.on_drag_end)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::DragCancel(event) => {
                let (store, handler) = if is_capture {
                    (
                        &mut self.on_drag_cancel_capture,
                        handlers.on_drag_cancel_capture,
                    )
                } else {
                    (&mut self.on_drag_cancel, handlers.on_drag_cancel)
                };
                dispatch_handler(store, handler, event, cx)
            }
            SemanticEvent::Scroll(event) => {
                let (store, handler) = if is_capture {
                    (&mut self.on_scroll_capture, handlers.on_scroll_capture)
                } else {
                    (&mut self.on_scroll, handlers.on_scroll)
                };
                dispatch_handler(store, handler, event, cx)
            }
        }
    }
}

fn update_handler<K, H>(
    handlers: &mut SlotMap<K, H>,
    current: Option<K>,
    next: Option<H>,
) -> Option<K>
where
    K: SlotMapKey,
{
    match (current, next) {
        (Some(id), Some(next)) => {
            if let Some(current) = handlers.get_mut(id) {
                *current = next;
                Some(id)
            } else {
                Some(handlers.insert(next))
            }
        }
        (Some(id), None) => {
            handlers.remove(id);
            None
        }
        (None, Some(next)) => Some(handlers.insert(next)),
        (None, None) => None,
    }
}

fn remove_handler<K, H>(handlers: &mut SlotMap<K, H>, id: Option<K>)
where
    K: SlotMapKey,
{
    if let Some(id) = id {
        handlers.remove(id);
    }
}

fn dispatch_handler<K, E>(
    handlers: &mut SlotMap<K, TypedEventHandler<E>>,
    id: Option<K>,
    event: &E,
    cx: &mut EventContext<'_>,
) -> EventResult
where
    K: SlotMapKey,
{
    if let Some(handler) = id.and_then(|id| handlers.get_mut(id)) {
        handler(event, cx)
    } else {
        EventResult::Ignored
    }
}
