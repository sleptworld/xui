//! The event vocabulary, declared once.
//!
//! Everything below — the handler enum, the per-node storage, the dispatch
//! match, the `EventMask` bits, and the whole `on_*` builder surface — is
//! generated from the `events!` table in the middle of this file.
//!
//! It replaces seven hand-maintained copies of the same list of 34 handler
//! names: the `EventHandlers` fields, its `is_empty`, its `Debug`, a
//! `CallbackHandleSet` of slotmap keys, a `CallbackStore` of 34 separate
//! `SlotMap`s, an `update_set` that touched each of them, and 525 lines of
//! builder methods in `widgets.rs`. Adding an event meant editing all seven.
//!
//! The slotmap layer is gone with them. Handler ids were never looked up from
//! anywhere but the node that owned them, so the indirection bought nothing
//! while costing a 272-byte `Copy` handle set per node and 34 allocators per
//! runtime. Handlers now live inline on the node, in a `SmallVec` that stays on
//! the stack for the two-or-so handlers a node typically has.

#![allow(deprecated)]

use smallvec::SmallVec;
use xui_interface::EventPhase;
use xui_interface::events::semantic::{
    ClickEvent, CommandEvent, ContextMenuEvent, DragEvent, FocusEvent, HoverEvent,
    PointerBoundaryEvent, PointerMoveEvent, PressEvent, ScrollEvent, SemanticEvent,
};

use super::{EventContext, Flow, Handler};

/// Which side of the propagation path a handler listens on.
///
/// The previous design encoded this in the handler's *name* (`on_click` versus
/// `on_click_capture`), doubling every list it appeared in and forcing an
/// `if is_capture { .. } else { .. }` into every arm of the dispatch match.
/// Phase was already a value; now the storage agrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListenPhase {
    /// Runs at the target and while bubbling back up.
    Bubble,
    /// Runs while descending towards the target.
    Capture,
}

impl ListenPhase {
    fn matches(self, phase: EventPhase) -> bool {
        match self {
            ListenPhase::Capture => phase == EventPhase::Capture,
            ListenPhase::Bubble => phase != EventPhase::Capture,
        }
    }
}

macro_rules! events {
    (
        $(
            $kind:ident : $variant:ident($event:ty) => $method:ident
                $(, capture = $capture_method:ident)?
                $(, widget = $widget_stage:ident)?
                $(, deprecated = $deprecated:literal)? ;
        )*
    ) => {
        /// One discriminant per handler slot.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum EventKind {
            /// `on_event` — every semantic event, whatever its kind.
            Any,
            $($kind,)*
        }

        bitflags::bitflags! {
            /// Which kinds a node listens for at all, in one word.
            ///
            /// Subsumes the hand-written `has_focus_callbacks()` and
            /// `has_drag_callbacks()` chains, and lets the runtime ask "does
            /// this node care?" without touching the handler list.
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
            pub struct EventMask: u64 {
                const ANY = 1 << 0;
                $( const $kind = 1 << (EventKind::$kind as u64 + 1); )*
            }
        }

        impl EventKind {
            fn mask(self) -> EventMask {
                match self {
                    EventKind::Any => EventMask::ANY,
                    $( EventKind::$kind => EventMask::$kind, )*
                }
            }

            /// Whether the owning widget's own handling runs at every phase, or
            /// only when the event is actually aimed at it.
            ///
            /// Target-only is the default because the alternative is a trap: an
            /// event that bubbles reaches every ancestor's widget too, and
            /// nothing reminds the widget author to check `current_target`. The
            /// one widget in this codebase that reads semantic events did not
            /// check, and only escaped the bug by being a leaf.
            fn widget_runs_at(self, phase: EventPhase) -> bool {
                #[allow(unused_mut, unused_assignments)]
                let mut all_phases = false;
                $(
                    if let EventKind::$kind = self {
                        $( all_phases = stringify!($widget_stage) == "all_phases"; )?
                    }
                )*
                all_phases || phase == EventPhase::Target
            }

            pub fn of(event: &SemanticEvent) -> Self {
                match event {
                    $( SemanticEvent::$variant(_) => EventKind::$kind, )*
                }
            }
        }

        /// A handler together with the event type it expects.
        ///
        /// One variant per slot rather than per payload type, so the dispatch
        /// match below is a straight 1:1 generation from the table.
        #[derive(Debug)]
        pub enum AnyHandler {
            Any(Handler<SemanticEvent>),
            $( $kind(Handler<$event>), )*
        }

        impl AnyHandler {
            fn kind(&self) -> EventKind {
                match self {
                    AnyHandler::Any(_) => EventKind::Any,
                    $( AnyHandler::$kind(_) => EventKind::$kind, )*
                }
            }

            fn call(&self, event: &SemanticEvent, cx: &mut EventContext<'_>) -> Flow {
                match (self, event) {
                    (AnyHandler::Any(handler), event) => handler.call(event, cx),
                    $(
                        (AnyHandler::$kind(handler), SemanticEvent::$variant(event)) => {
                            handler.call(event, cx)
                        }
                    )*
                    // A slot only ever holds the handler its kind implies, so
                    // this is unreachable; returning empty keeps it total.
                    _ => Flow::empty(),
                }
            }
        }

        /// The `on_*` builder surface, for every widget that owns handlers.
        ///
        /// Blanket-implemented over [`Listen`], the same way `StyleProps` is
        /// blanket-implemented over `Styled`: a widget opts in with a
        /// three-line impl and gets the whole vocabulary.
        pub trait EventProps: Listen {
            /// Every semantic event, before the kind-specific handler.
            fn on_event<R: Into<Flow>>(
                mut self,
                handler: impl for<'a> FnMut(&SemanticEvent, &mut EventContext<'a>) -> R + 'static,
            ) -> Self {
                self.handlers_mut()
                    .set(ListenPhase::Bubble, AnyHandler::Any(Handler::new(handler)));
                self
            }

            fn on_event_capture<R: Into<Flow>>(
                mut self,
                handler: impl for<'a> FnMut(&SemanticEvent, &mut EventContext<'a>) -> R + 'static,
            ) -> Self {
                self.handlers_mut()
                    .set(ListenPhase::Capture, AnyHandler::Any(Handler::new(handler)));
                self
            }

            $(
                $(#[deprecated(note = $deprecated)])?
            fn $method<R: Into<Flow>>(
                    mut self,
                    handler: impl for<'a> FnMut(&$event, &mut EventContext<'a>) -> R + 'static,
            ) -> Self {
                    self.handlers_mut().set(
                        ListenPhase::Bubble,
                        AnyHandler::$kind(Handler::new(handler)),
                    );
                    self
                }

                $(
                    fn $capture_method<R: Into<Flow>>(
                        mut self,
                        handler: impl for<'a> FnMut(&$event, &mut EventContext<'a>) -> R + 'static,
                    ) -> Self {
                        self.handlers_mut().set(
                            ListenPhase::Capture,
                            AnyHandler::$kind(Handler::new(handler)),
                        );
                        self
                    }
                )?
            )*
        }

        impl<T: Listen> EventProps for T {}
    };
}

// The whole event vocabulary. Adding a row adds the enum discriminant, the mask
// bit, the handler variant, the dispatch arm, and the builder methods.
events! {
    Command:      Command(CommandEvent)          => on_command;

    PointerMove:  PointerMove(PointerMoveEvent)  => on_pointer_move, capture = on_pointer_move_capture;
    PointerEnter: PointerEnter(PointerBoundaryEvent) => on_pointer_enter;
    PointerLeave: PointerLeave(PointerBoundaryEvent) => on_pointer_leave;

    Hovered:      Hovered(HoverEvent)            => on_hovered,
        deprecated = "use on_pointer_enter and on_pointer_leave instead";

    PressStart:   PressStart(PressEvent)         => on_press_start,   capture = on_press_start_capture;
    PressEnd:     PressEnd(PressEvent)           => on_press_end,     capture = on_press_end_capture;
    PressCancel:  PressCancel(PressEvent)        => on_press_cancel,  capture = on_press_cancel_capture;

    Click:        Click(ClickEvent)              => on_click,         capture = on_click_capture;
    DoubleClick:  DoubleClick(ClickEvent)        => on_double_click,  capture = on_double_click_capture;
    ContextMenu:  ContextMenu(ContextMenuEvent)  => on_context_menu,  capture = on_context_menu_capture;

    Focus:        Focus(FocusEvent)              => on_focus;
    Blur:         Blur(FocusEvent)               => on_blur;
    FocusIn:      FocusIn(FocusEvent)            => on_focus_in,      capture = on_focus_in_capture;
    FocusOut:     FocusOut(FocusEvent)           => on_focus_out,     capture = on_focus_out_capture;

    DragStart:    DragStart(DragEvent)           => on_drag_start,    capture = on_drag_start_capture;
    DragMove:     DragMove(DragEvent)            => on_drag_move,     capture = on_drag_move_capture;
    DragEnd:      DragEnd(DragEvent)             => on_drag_end,      capture = on_drag_end_capture;
    DragCancel:   DragCancel(DragEvent)          => on_drag_cancel,   capture = on_drag_cancel_capture;

    Scroll:       Scroll(ScrollEvent)            => on_scroll,        capture = on_scroll_capture;
}

impl EventMask {
    /// Focus-ish kinds, for `is_focusable`'s "does this node handle focus?".
    pub const FOCUS: Self = Self::Focus
        .union(Self::Blur)
        .union(Self::FocusIn)
        .union(Self::FocusOut);

    /// Drag kinds, for the translator's "should this node start a drag?".
    pub const DRAG: Self = Self::DragStart
        .union(Self::DragMove)
        .union(Self::DragEnd)
        .union(Self::DragCancel);
}

/// A builder that owns an [`EventHandlers`], which [`EventProps`] then decorates
/// with the whole `on_*` vocabulary.
pub trait Listen: Sized {
    fn handlers_mut(&mut self) -> &mut EventHandlers;
}

#[derive(Debug)]
struct Entry {
    phase: ListenPhase,
    handler: AnyHandler,
}

/// The handlers attached to one node.
///
/// Flat rather than a struct of 34 `Option`s: a node with an `on_click` used to
/// carry 33 empty fields plus, in the runtime, a 272-byte handle set. Nodes
/// almost always have zero or one handler, so a `SmallVec` inline capacity of
/// four covers the realistic cases without allocating.
#[derive(Default, Debug)]
pub struct EventHandlers {
    mask: EventMask,
    entries: SmallVec<[Entry; 4]>,
}

impl EventHandlers {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Which kinds this node listens for, on either phase.
    pub fn mask(&self) -> EventMask {
        self.mask
    }

    pub fn listens_for(&self, mask: EventMask) -> bool {
        self.mask.intersects(mask)
    }

    /// Replaces any handler already registered for the same slot, so setting an
    /// attribute twice behaves like assignment rather than accumulating.
    pub fn set(&mut self, phase: ListenPhase, handler: AnyHandler) {
        let kind = handler.kind();
        self.mask |= kind.mask();
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.phase == phase && entry.handler.kind() == kind)
        {
            Some(entry) => entry.handler = handler,
            None => self.entries.push(Entry { phase, handler }),
        }
    }

    fn get(&self, kind: EventKind, phase: EventPhase) -> Option<&AnyHandler> {
        self.entries
            .iter()
            .find(|entry| entry.handler.kind() == kind && entry.phase.matches(phase))
            .map(|entry| &entry.handler)
    }

    /// Runs this node's handlers for one event, in one phase.
    ///
    /// The generic `on_event` handler runs first and can stop the rest, matching
    /// the previous behaviour.
    pub(crate) fn dispatch(&self, event: &SemanticEvent, cx: &mut EventContext<'_>) -> Flow {
        let phase = cx.phase;
        let mut flow = Flow::empty();

        if let Some(handler) = self.get(EventKind::Any, phase) {
            flow |= handler.call(event, cx);
            if flow.stops_propagation() {
                return flow;
            }
        }

        if let Some(handler) = self.get(EventKind::of(event), phase) {
            flow |= handler.call(event, cx);
        }

        flow
    }

    /// Whether the owning widget's built-in handling should run for this event
    /// at this phase. See [`EventKind::widget_runs_at`].
    pub(crate) fn widget_stage_runs(event: &SemanticEvent, phase: EventPhase) -> bool {
        EventKind::of(event).widget_runs_at(phase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_with_no_handlers_carries_an_empty_mask() {
        let handlers = EventHandlers::default();
        assert!(handlers.is_empty());
        assert!(!handlers.listens_for(EventMask::all()));
    }

    #[test]
    fn setting_the_same_slot_twice_replaces_rather_than_accumulates() {
        let mut handlers = EventHandlers::default();
        handlers.set(
            ListenPhase::Bubble,
            AnyHandler::Click(Handler::new(|_, _| Flow::empty())),
        );
        handlers.set(
            ListenPhase::Bubble,
            AnyHandler::Click(Handler::new(|_, _| Flow::STOP_PROPAGATION)),
        );
        assert_eq!(handlers.entries.len(), 1);
    }

    #[test]
    fn the_two_phases_are_separate_slots() {
        let mut handlers = EventHandlers::default();
        handlers.set(
            ListenPhase::Bubble,
            AnyHandler::Click(Handler::new(|_, _| Flow::empty())),
        );
        handlers.set(
            ListenPhase::Capture,
            AnyHandler::Click(Handler::new(|_, _| Flow::empty())),
        );
        assert_eq!(handlers.entries.len(), 2);
        assert!(
            handlers
                .get(EventKind::Click, EventPhase::Capture)
                .is_some()
        );
        assert!(handlers.get(EventKind::Click, EventPhase::Target).is_some());
        assert!(handlers.get(EventKind::Click, EventPhase::Bubble).is_some());
    }

    #[test]
    fn the_mask_answers_group_queries_without_walking_the_list() {
        let mut handlers = EventHandlers::default();
        assert!(!handlers.listens_for(EventMask::DRAG));
        handlers.set(
            ListenPhase::Bubble,
            AnyHandler::DragMove(Handler::new(|_, _| Flow::empty())),
        );
        assert!(handlers.listens_for(EventMask::DRAG));
        assert!(!handlers.listens_for(EventMask::FOCUS));
    }

    #[test]
    fn a_widget_only_sees_a_bubbling_event_aimed_at_it() {
        // The default that keeps an ancestor `text_input` from treating a
        // descendant's `FocusIn` as its own.
        assert!(EventKind::FocusIn.widget_runs_at(EventPhase::Target));
        assert!(!EventKind::FocusIn.widget_runs_at(EventPhase::Bubble));
        assert!(!EventKind::FocusIn.widget_runs_at(EventPhase::Capture));
    }
}
