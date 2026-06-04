use slotmap::{Key as SlotMapKey, SlotMap, new_key_type};

pub use xui_interface::{
    ClickEventHandler, Event, EventContext, EventHandler, EventHandlers, EventPhase, EventRequest,
    EventRequests, EventResult, HoverChangeEventHandler, InputKey as Key, KeyEventHandler,
    PointerButton, PointerEventHandler,
};

new_key_type! {
    pub struct RawEventHandlerId;
    pub struct ClickEventHandlerId;
    pub struct HoverChangeEventHandlerId;
    pub struct PointerEventHandlerId;
    pub struct KeyEventHandlerId;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventHandlerSet {
    pub on_event: Option<RawEventHandlerId>,
    pub on_click: Option<ClickEventHandlerId>,
    pub on_hover_change: Option<HoverChangeEventHandlerId>,
    pub on_pointer_down: Option<PointerEventHandlerId>,
    pub on_pointer_up: Option<PointerEventHandlerId>,
    pub on_pointer_move: Option<PointerEventHandlerId>,
    pub on_key_down: Option<KeyEventHandlerId>,
    pub on_key_up: Option<KeyEventHandlerId>,
}

#[derive(Default)]
pub(crate) struct EventHandlerStore {
    on_event: SlotMap<RawEventHandlerId, EventHandler>,
    on_click: SlotMap<ClickEventHandlerId, ClickEventHandler>,
    on_hover_change: SlotMap<HoverChangeEventHandlerId, HoverChangeEventHandler>,
    on_pointer_down: SlotMap<PointerEventHandlerId, PointerEventHandler>,
    on_pointer_up: SlotMap<PointerEventHandlerId, PointerEventHandler>,
    on_pointer_move: SlotMap<PointerEventHandlerId, PointerEventHandler>,
    on_key_down: SlotMap<KeyEventHandlerId, KeyEventHandler>,
    on_key_up: SlotMap<KeyEventHandlerId, KeyEventHandler>,
}

impl EventHandlerStore {
    pub(crate) fn update_set(
        &mut self,
        current: EventHandlerSet,
        handlers: EventHandlers,
    ) -> EventHandlerSet {
        EventHandlerSet {
            on_event: update_handler(&mut self.on_event, current.on_event, handlers.on_event),
            on_click: update_handler(&mut self.on_click, current.on_click, handlers.on_click),
            on_hover_change: update_handler(
                &mut self.on_hover_change,
                current.on_hover_change,
                handlers.on_hover_change,
            ),
            on_pointer_down: update_handler(
                &mut self.on_pointer_down,
                current.on_pointer_down,
                handlers.on_pointer_down,
            ),
            on_pointer_up: update_handler(
                &mut self.on_pointer_up,
                current.on_pointer_up,
                handlers.on_pointer_up,
            ),
            on_pointer_move: update_handler(
                &mut self.on_pointer_move,
                current.on_pointer_move,
                handlers.on_pointer_move,
            ),
            on_key_down: update_handler(
                &mut self.on_key_down,
                current.on_key_down,
                handlers.on_key_down,
            ),
            on_key_up: update_handler(&mut self.on_key_up, current.on_key_up, handlers.on_key_up),
        }
    }

    pub(crate) fn clear_set(&mut self, handlers: EventHandlerSet) {
        remove_handler(&mut self.on_event, handlers.on_event);
        remove_handler(&mut self.on_click, handlers.on_click);
        remove_handler(&mut self.on_hover_change, handlers.on_hover_change);
        remove_handler(&mut self.on_pointer_down, handlers.on_pointer_down);
        remove_handler(&mut self.on_pointer_up, handlers.on_pointer_up);
        remove_handler(&mut self.on_pointer_move, handlers.on_pointer_move);
        remove_handler(&mut self.on_key_down, handlers.on_key_down);
        remove_handler(&mut self.on_key_up, handlers.on_key_up);
    }

    pub(crate) fn dispatch_on_event(
        &mut self,
        handler: Option<RawEventHandlerId>,
        event: &Event,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_event.get_mut(id)) {
            return handler(event, cx);
        }
        EventResult::Ignored
    }

    pub(crate) fn dispatch_on_click(
        &mut self,
        handler: Option<ClickEventHandlerId>,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_click.get_mut(id)) {
            return handler(cx);
        }
        EventResult::Ignored
    }

    pub(crate) fn dispatch_on_hover_change(
        &mut self,
        handler: Option<HoverChangeEventHandlerId>,
        hovered: bool,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_hover_change.get_mut(id)) {
            return handler(hovered, cx);
        }
        EventResult::Ignored
    }

    pub(crate) fn dispatch_on_pointer_down(
        &mut self,
        handler: Option<PointerEventHandlerId>,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_pointer_down.get_mut(id)) {
            return handler(cx);
        }
        EventResult::Ignored
    }

    pub(crate) fn dispatch_on_pointer_up(
        &mut self,
        handler: Option<PointerEventHandlerId>,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_pointer_up.get_mut(id)) {
            return handler(cx);
        }
        EventResult::Ignored
    }

    pub(crate) fn dispatch_on_pointer_move(
        &mut self,
        handler: Option<PointerEventHandlerId>,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_pointer_move.get_mut(id)) {
            return handler(cx);
        }
        EventResult::Ignored
    }

    pub(crate) fn dispatch_on_key_down(
        &mut self,
        handler: Option<KeyEventHandlerId>,
        key: &Key,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_key_down.get_mut(id)) {
            return handler(key, cx);
        }
        EventResult::Ignored
    }

    pub(crate) fn dispatch_on_key_up(
        &mut self,
        handler: Option<KeyEventHandlerId>,
        key: &Key,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(handler) = handler.and_then(|id| self.on_key_up.get_mut(id)) {
            return handler(key, cx);
        }
        EventResult::Ignored
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
