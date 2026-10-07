use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Weak;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{
    AllocAnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message,
    define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSCursor, NSEvent, NSEventModifierFlags, NSEventPhase, NSEventType, NSResponder,
    NSTextInputClient, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSObject, NSObjectProtocol,
    NSPoint, NSRange, NSRangePointer, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
    NSUInteger,
};
use objc2_quartz_core::CADisplayLink;
use xui_interface::events::{KeyState, KeyText, RawIme, TextPayload};
use xui_interface::{
    ContextMenuTrigger, CursorIcon, Modifiers, Point, PointerButton, Rect, ScrollDelta, TextOffset,
    TextRange, Translation, WindowRegion, commands,
};
use xui_shell::{FrameInfo, KeyInput, PublishedHandle, ShellEvent, ShellOutcome};

use crate::host::{self, EventSink};
use crate::{cursor, keycode, time};

/// What the input context did with the key `keyDown:` is handling.
#[derive(Default)]
struct KeyDown {
    /// Text inserted straight through, with no composition involved.
    text: Option<String>,
    /// The context composed or committed: the key is not the runtime's.
    consumed: bool,
}

pub(crate) struct ViewIvars {
    sink: OnceCell<Weak<dyn EventSink>>,
    modifiers: Cell<Modifiers>,
    last_position: Cell<Option<Point>>,
    cursor: RefCell<Retained<NSCursor>>,
    cursor_hidden: Cell<bool>,
    ime_allowed: Cell<bool>,
    /// The candidate-window anchor, in view coordinates.
    ime_area: Cell<Rect>,
    marked_text: RefCell<String>,
    /// `Some` while `keyDown:` is inside `interpretKeyEvents:`.
    key_down: RefCell<Option<KeyDown>>,
    /// Paces frames with the display. `None` until the view has a window, and
    /// on macOS before 14, where the main queue carries redraws instead.
    display_link: RefCell<Option<Retained<CADisplayLink>>>,
    /// Whether a frame has been asked for since the last one was delivered.
    redraw_pending: Cell<bool>,
    /// What the runtime has published about the window, for hit tests that
    /// cannot wait for it: is a press on the title bar?
    published: RefCell<Option<PublishedHandle>>,
    /// The press that is going on, which `performWindowDragWithEvent:` needs to
    /// start a move from code.
    last_mouse_down: RefCell<Option<Retained<NSEvent>>>,
    /// A control-click is a right click on a Mac; its release has to be one too.
    control_click: Cell<bool>,
    /// A press the host kept for itself -- a title-bar drag -- so the runtime
    /// never saw it go down and must not see it come up.
    swallowed_press: Cell<bool>,
    /// The key event `performKeyEquivalent:` offered to the runtime and was
    /// refused, so `keyDown:` -- which AppKit then calls with the same event --
    /// does not deliver it twice.
    offered_key: Cell<Option<(u64, u16)>>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
}

define_class!(
    // SAFETY: NSView's subclassing requirements are to be used on the main
    // thread, which `MainThreadOnly` enforces. `XuiView` does not implement
    // `Drop`.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "XuiView"]
    #[ivars = ViewIvars]
    pub(crate) struct XuiView;

    impl XuiView {
        /// Top-left origin, as xui's logical coordinates are.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// The click that activates the window also reaches the UI.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &self.ivars().cursor.borrow());
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.pointer_button(event, true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.pointer_button(event, false);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.pointer_button(event, true);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.pointer_button(event, false);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.pointer_button(event, true);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.pointer_button(event, false);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.track_position(event);
            let delta = Translation::new(
                event.scrollingDeltaX() as f32,
                event.scrollingDeltaY() as f32,
            );
            let delta = if event.hasPreciseScrollingDeltas() {
                ScrollDelta::Pixels(delta)
            } else {
                ScrollDelta::Lines(delta)
            };
            self.send_native(
                event,
                ShellEvent::Wheel {
                    delta,
                    device_id: None,
                    is_inertial: event.momentumPhase() != NSEventPhase::None,
                },
            );
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.handle_key_down(event);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            self.sync_modifiers(event);
            self.send_native(
                event,
                ShellEvent::Keyboard(key_input(event, KeyState::Up, None, false)),
            );
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            self.sync_modifiers(event);
            let code = event.keyCode();
            let flags = event.modifierFlags().0;
            if let Some(pressed) = keycode::modifier_pressed(code, flags) {
                let state = if pressed { KeyState::Down } else { KeyState::Up };
                // `isARepeat` raises on a flags-changed event.
                self.send_native(
                    event,
                    ShellEvent::Keyboard(key_input(event, state, None, false)),
                );
            }
        }

        /// Tracking is what makes the pointer leaving the window a thing the
        /// runtime can hear about: nothing moves once it is outside.
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe { msg_send![super(self), updateTrackingAreas] }
            let ivars = self.ivars();
            if let Some(old) = ivars.tracking_area.take() {
                self.removeTrackingArea(&old);
            }
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    self.bounds(),
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveInActiveApp
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            ivars.tracking_area.replace(Some(area));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            // With a button down the pointer is captured and keeps its hover; the
            // runtime would ignore this anyway, but it is not true to say the
            // pointer left something it is still dragging.
            if NSEvent::pressedMouseButtons() == 0 {
                self.ivars().last_position.set(None);
                self.send_native(event, ShellEvent::PointerLeft);
            }
        }

        // The standard edit actions, which the Edit menu and the system's own
        // key equivalents send down the responder chain. The runtime handles
        // them as commands.
        #[unsafe(method(undo:))]
        fn edit_undo(&self, _sender: Option<&AnyObject>) {
            self.send(ShellEvent::Command(commands::UNDO));
        }

        #[unsafe(method(redo:))]
        fn edit_redo(&self, _sender: Option<&AnyObject>) {
            self.send(ShellEvent::Command(commands::REDO));
        }

        #[unsafe(method(cut:))]
        fn edit_cut(&self, _sender: Option<&AnyObject>) {
            self.send(ShellEvent::Command(commands::CUT));
        }

        #[unsafe(method(copy:))]
        fn edit_copy(&self, _sender: Option<&AnyObject>) {
            self.send(ShellEvent::Command(commands::COPY));
        }

        #[unsafe(method(paste:))]
        fn edit_paste(&self, _sender: Option<&AnyObject>) {
            self.send(ShellEvent::Command(commands::PASTE));
        }

        #[unsafe(method(selectAll:))]
        fn edit_select_all(&self, _sender: Option<&AnyObject>) {
            self.send(ShellEvent::Command(commands::SELECT_ALL));
        }

        /// Offers a Command or Control key to the runtime before the menu bar
        /// sees it, so a shortcut the application has bound wins over a menu
        /// item that shows the same keys. If the runtime does not take it, AppKit
        /// goes on to the menus, and to `keyDown:` if no menu wants it either.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> Bool {
            let flags = event.modifierFlags();
            let command = flags.contains(NSEventModifierFlags::Command);
            let control = flags.contains(NSEventModifierFlags::Control);
            if event.r#type() != NSEventType::KeyDown || !(command || control) {
                return Bool::NO;
            }
            // Input methods use Control chords of their own -- Ctrl+J and Ctrl+K
            // convert kana, for one -- and while one is composing, or may be, the
            // key has to reach `interpretKeyEvents:` in `keyDown:` first. Offering
            // it here and skipping that later would take it from the method.
            if self.composing() || (control && !command && self.ivars().ime_allowed.get()) {
                return Bool::NO;
            }
            self.sync_modifiers(event);
            let outcome = self.submit_native(
                event,
                ShellEvent::Keyboard(key_input(event, KeyState::Down, None, event.isARepeat())),
            );
            if !outcome.consumed {
                self.ivars().offered_key.set(Some(key_identity(event)));
            }
            Bool::new(outcome.consumed)
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            unsafe { msg_send![super(self), viewDidMoveToWindow] }
            if self.window().is_some() {
                self.start_display_link();
            } else {
                // Also breaks the retain cycle: the link retains this view.
                self.invalidate_display_link();
            }
        }

        /// One tick of the display link: the frame the display is about to
        /// show.
        #[unsafe(method(xuiDisplayLinkFired:))]
        fn display_link_fired(&self, link: &CADisplayLink) {
            if self.ivars().redraw_pending.replace(false) {
                // When this frame will be on screen, and how long a refresh
                // lasts, so animation is paced to the display rather than to
                // whenever the frame happened to be built.
                let target = link.targetTimestamp();
                self.send(ShellEvent::FrameTick(FrameInfo {
                    target: Some(time::instant_of(target)),
                    interval: Duration::try_from_secs_f64(target - link.timestamp()).ok(),
                }));
            } else if let Some(link) = self.ivars().display_link.borrow().as_ref() {
                // Nothing to draw: stop ticking until something asks again.
                link.setPaused(true);
            }
        }
    }

    unsafe impl NSObjectProtocol for XuiView {}

    unsafe impl NSTextInputClient for XuiView {
        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.composing()
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let length = self.ivars().marked_text.borrow().encode_utf16().count();
            if length == 0 {
                not_found()
            } else {
                NSRange::new(0, length)
            }
        }

        /// The runtime does not share its document with the input context.
        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            not_found()
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(
            &self,
            string: &AnyObject,
            selected_range: NSRange,
            _replacement_range: NSRange,
        ) {
            let text = ns_text(string);
            let ivars = self.ivars();
            if let Some(key_down) = ivars.key_down.borrow_mut().as_mut() {
                key_down.consumed = true;
            }
            let start = utf16_to_byte(&text, selected_range.location);
            let end = utf16_to_byte(
                &text,
                selected_range.location.saturating_add(selected_range.length),
            );
            ivars.marked_text.replace(text.clone());
            self.send(ShellEvent::Ime(RawIme::Preedit {
                text: TextPayload::new(&text),
                cursor: Some(TextRange::new(
                    TextOffset::byte_offset(start),
                    TextOffset::byte_offset(end),
                )),
                timestamp: Instant::now(),
            }));
        }

        /// Accepts the composition as it stands.
        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let text = self.ivars().marked_text.take();
            if !text.is_empty() {
                self.commit(&text, true);
            }
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring_for_proposed_range(
            &self,
            _range: NSRange,
            _actual_range: NSRangePointer,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _replacement_range: NSRange) {
            let text = ns_text(string);
            let ivars = self.ivars();
            let was_composing = !ivars.marked_text.take().is_empty();
            {
                let mut key_down = ivars.key_down.borrow_mut();
                if let Some(key_down) = key_down.as_mut() {
                    if !was_composing {
                        // Plain typing: the text belongs to the key event.
                        key_down.text.get_or_insert_with(String::new).push_str(&text);
                        return;
                    }
                    key_down.consumed = true;
                }
            }
            self.commit(&text, was_composing);
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }

        /// Where the candidate window goes, in screen coordinates.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect_for_character_range(
            &self,
            _range: NSRange,
            _actual_range: NSRangePointer,
        ) -> NSRect {
            let area = self.ivars().ime_area.get();
            let rect = NSRect::new(
                NSPoint::new(area.x as f64, area.y as f64),
                NSSize::new(area.width as f64, area.height as f64),
            );
            let in_window = self.convertRect_toView(rect, None);
            self.window()
                .map_or(in_window, |window| window.convertRectToScreen(in_window))
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Nothing: the key reaches the runtime as a key once
            // `interpretKeyEvents:` returns. Handling it here keeps AppKit
            // from beeping about an unimplemented command.
        }
    }
);

impl XuiView {
    pub(crate) fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            sink: OnceCell::new(),
            modifiers: Cell::new(Modifiers::default()),
            last_position: Cell::new(None),
            cursor: RefCell::new(NSCursor::arrowCursor()),
            cursor_hidden: Cell::new(false),
            ime_allowed: Cell::new(false),
            ime_area: Cell::new(Rect::new(0.0, 0.0, 0.0, 0.0)),
            marked_text: RefCell::new(String::new()),
            key_down: RefCell::new(None),
            display_link: RefCell::new(None),
            redraw_pending: Cell::new(false),
            published: RefCell::new(None),
            last_mouse_down: RefCell::new(None),
            control_click: Cell::new(false),
            swallowed_press: Cell::new(false),
            offered_key: Cell::new(None),
            tracking_area: RefCell::new(None),
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    pub(crate) fn set_sink(&self, sink: Weak<dyn EventSink>) {
        let _ = self.ivars().sink.set(sink);
    }

    pub(crate) fn set_cursor(&self, icon: CursorIcon) {
        let ivars = self.ivars();
        match cursor::ns_cursor(icon) {
            None => {
                if !ivars.cursor_hidden.replace(true) {
                    NSCursor::hide();
                }
            }
            Some(ns_cursor) => {
                if ivars.cursor_hidden.replace(false) {
                    NSCursor::unhide();
                }
                // Set now for the pointer that is already over the view; the
                // cursor rect keeps it from being reset on the next move.
                ns_cursor.set();
                ivars.cursor.replace(ns_cursor);
                if let Some(window) = self.window() {
                    window.invalidateCursorRectsForView(self);
                }
            }
        }
    }

    pub(crate) fn set_ime_allowed(&self, allowed: bool) {
        let ivars = self.ivars();
        if ivars.ime_allowed.replace(allowed) == allowed {
            return;
        }
        let timestamp = Instant::now();
        if allowed {
            self.send(ShellEvent::Ime(RawIme::Enabled { timestamp }));
            return;
        }
        if !ivars.marked_text.take().is_empty()
            && let Some(context) = self.inputContext()
        {
            context.discardMarkedText();
        }
        self.send(ShellEvent::Ime(RawIme::Disabled { timestamp }));
    }

    pub(crate) fn set_ime_cursor_area(&self, area: Rect) {
        self.ivars().ime_area.set(area);
    }

    /// Asks for a frame on the display's next refresh.
    pub(crate) fn request_redraw(&self) {
        self.ivars().redraw_pending.set(true);
        match self.ivars().display_link.borrow().as_ref() {
            Some(link) => link.setPaused(false),
            // Before macOS 14 there is no display link on a view; the main
            // queue coalesces requests instead, and the presenter paces them.
            None => host::request_redraw(),
        }
    }

    /// Creates the view's display link, on a system that has them.
    ///
    /// It starts paused, and ticks only while a frame is pending, so an idle
    /// window costs nothing.
    fn start_display_link(&self) {
        let ivars = self.ivars();
        if ivars.display_link.borrow().is_some() {
            return;
        }
        // `displayLinkWithTarget:selector:` is macOS 14.
        if !Self::class().responds_to(sel!(displayLinkWithTarget:selector:)) {
            return;
        }
        // SAFETY: `xuiDisplayLinkFired:` is defined above and takes the link,
        // which is the callback shape this asks for.
        let link = unsafe { self.displayLinkWithTarget_selector(self, sel!(xuiDisplayLinkFired:)) };
        // SAFETY: the run loop is this thread's, and the mode is a constant.
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::currentRunLoop(), NSRunLoopCommonModes) };
        link.setPaused(!ivars.redraw_pending.get());
        ivars.display_link.replace(Some(link));
    }

    /// Stops the link and releases it -- and with it, this view, which it
    /// retains.
    pub(crate) fn invalidate_display_link(&self) {
        if let Some(link) = self.ivars().display_link.take() {
            link.invalidate();
        }
    }

    /// Delivers `event`, stamped with the time it is sent.
    fn send(&self, event: ShellEvent) {
        let _ = self.send_at(Instant::now(), event);
    }

    /// Delivers `event`, stamped with the time AppKit says `native` happened:
    /// the time that matters for double-click spacing and scroll velocity,
    /// rather than whenever the event got here.
    fn send_native(&self, native: &NSEvent, event: ShellEvent) {
        let _ = self.submit_native(native, event);
    }

    /// [`Self::send_native`], and what the runtime made of the event.
    fn submit_native(&self, native: &NSEvent, event: ShellEvent) -> ShellOutcome {
        self.send_at(time::instant_of(native.timestamp()), event)
    }

    fn send_at(&self, at: Instant, event: ShellEvent) -> ShellOutcome {
        match self.ivars().sink.get().and_then(Weak::upgrade) {
            Some(sink) => sink.send(at, event),
            None => ShellOutcome::CONTINUE,
        }
    }

    pub(crate) fn set_published(&self, published: PublishedHandle) {
        self.ivars().published.replace(Some(published));
    }

    /// Whether the press at `event` is on a region the runtime says is a title
    /// bar. If it is, the host keeps it: a drag moves the window, a
    /// double-click zooms it, and the runtime never hears of it.
    fn press_on_caption(&self, event: &NSEvent) -> bool {
        let on_caption = self
            .ivars()
            .published
            .borrow()
            .as_ref()
            .is_some_and(|published| {
                published.region_at(self.position(event)) == WindowRegion::Caption
            });
        if !on_caption {
            return false;
        }
        if let Some(window) = self.window() {
            if event.clickCount() == 2 {
                // The system setting may say minimize instead; zoom is what the
                // default is.
                window.zoom(None);
            } else {
                window.performWindowDragWithEvent(event);
            }
        }
        true
    }

    /// Starts moving the window from the press that is going on, for the
    /// runtime's `BeginMove` command. `false` if no button is down.
    pub(crate) fn begin_window_drag(&self) -> bool {
        let down = NSEvent::pressedMouseButtons() & 1 != 0;
        let event = self.ivars().last_mouse_down.borrow().clone();
        match (down, event, self.window()) {
            (true, Some(event), Some(window)) => {
                window.performWindowDragWithEvent(&event);
                true
            }
            _ => false,
        }
    }

    fn composing(&self) -> bool {
        !self.ivars().marked_text.borrow().is_empty()
    }

    fn commit(&self, text: &str, end_composition: bool) {
        let timestamp = Instant::now();
        if end_composition {
            self.send(ShellEvent::Ime(RawIme::Preedit {
                text: TextPayload::new(""),
                cursor: None,
                timestamp,
            }));
        }
        self.send(ShellEvent::Ime(RawIme::Commit {
            text: TextPayload::new(text),
            timestamp,
        }));
    }

    fn position(&self, event: &NSEvent) -> Point {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        Point::new(point.x as f32, point.y as f32)
    }

    /// Brings the runtime's modifier state in line with the event's. Modifier
    /// changes made while another app was active arrive no other way.
    fn sync_modifiers(&self, event: &NSEvent) {
        let modifiers = modifiers(event.modifierFlags());
        if self.ivars().modifiers.replace(modifiers) != modifiers {
            self.send(ShellEvent::ModifiersChanged(modifiers));
        }
    }

    fn pointer_moved(&self, event: &NSEvent) {
        self.sync_modifiers(event);
        let position = self.position(event);
        self.ivars().last_position.set(Some(position));
        self.send_native(
            event,
            ShellEvent::PointerMoved {
                position,
                device_id: None,
            },
        );
    }

    /// The shell reports buttons and wheels at the last moved-to position, and
    /// an event is not always preceded by a move to where it happened -- the
    /// click that activates the window is not.
    fn track_position(&self, event: &NSEvent) {
        if self.ivars().last_position.get() != Some(self.position(event)) {
            self.pointer_moved(event);
        } else {
            self.sync_modifiers(event);
        }
    }

    fn pointer_button(&self, event: &NSEvent, pressed: bool) {
        self.track_position(event);
        let ivars = self.ivars();
        let mut button = button_of(event);

        // A control-click is a right click on a Mac, and so is its release.
        if button == PointerButton::Primary {
            if pressed
                && event
                    .modifierFlags()
                    .contains(NSEventModifierFlags::Control)
            {
                ivars.control_click.set(true);
            }
            if ivars.control_click.get() {
                button = PointerButton::Secondary;
                if !pressed {
                    ivars.control_click.set(false);
                }
            }
        }

        if button == PointerButton::Primary {
            if pressed {
                ivars.last_mouse_down.replace(Some(event.retain()));
                if self.press_on_caption(event) {
                    ivars.swallowed_press.set(true);
                    return;
                }
            } else if ivars.swallowed_press.replace(false) {
                return;
            }
        }

        self.send_native(
            event,
            ShellEvent::PointerButton {
                button,
                pressed,
                device_id: None,
            },
        );
        // The context menu comes with the press on a Mac, after the button
        // event, the way a page sees `mousedown` and then `contextmenu`.
        if pressed && button == PointerButton::Secondary {
            self.send_native(
                event,
                ShellEvent::ContextMenuRequested {
                    position: Some(self.position(event)),
                    trigger: ContextMenuTrigger::PointerSecondary,
                },
            );
        }
    }

    fn handle_key_down(&self, event: &NSEvent) {
        self.sync_modifiers(event);
        let ivars = self.ivars();
        // `performKeyEquivalent:` already offered this one to the runtime, and
        // nothing took it.
        if ivars.offered_key.take() == Some(key_identity(event)) {
            return;
        }
        // Command shortcuts are the runtime's, never text.
        let text = if ivars.ime_allowed.get() && !ivars.modifiers.get().meta {
            let was_composing = self.composing();
            ivars.key_down.replace(Some(KeyDown::default()));
            self.interpretKeyEvents(&NSArray::from_slice(&[event]));
            let key_down = ivars.key_down.take().unwrap_or_default();
            if key_down.consumed || was_composing {
                return;
            }
            key_down.text
        } else {
            plain_text(event)
        };
        self.send_native(
            event,
            ShellEvent::Keyboard(key_input(
                event,
                KeyState::Down,
                text.as_deref(),
                event.isARepeat(),
            )),
        );
    }
}

/// Which button a mouse event is for.
///
/// Read from the event's type for the left and right buttons, because
/// `buttonNumber` is only documented for the others and answers 0 for an event
/// that did not come from the hardware.
fn button_of(event: &NSEvent) -> PointerButton {
    match event.r#type() {
        NSEventType::LeftMouseDown | NSEventType::LeftMouseUp | NSEventType::LeftMouseDragged => {
            PointerButton::Primary
        }
        NSEventType::RightMouseDown
        | NSEventType::RightMouseUp
        | NSEventType::RightMouseDragged => PointerButton::Secondary,
        _ => match event.buttonNumber() {
            2 => PointerButton::Auxiliary,
            3 => PointerButton::Back,
            4 => PointerButton::Forward,
            other => PointerButton::Other(other as u16),
        },
    }
}

/// Enough to recognize one key event as the same one seen twice.
fn key_identity(event: &NSEvent) -> (u64, u16) {
    (event.timestamp().to_bits(), event.keyCode())
}

fn key_input(event: &NSEvent, state: KeyState, text: Option<&str>, is_repeat: bool) -> KeyInput {
    let code = event.keyCode();
    KeyInput {
        physical_key: keycode::physical_key(code),
        named_key: keycode::named_key(code),
        state,
        text: text.and_then(KeyText::try_new),
        is_repeat,
    }
}

/// The event's characters, unless they are a control character or one of
/// AppKit's private-use function-key codes.
fn plain_text(event: &NSEvent) -> Option<String> {
    let text = event.characters()?.to_string();
    let printable = !text.is_empty()
        && !text
            .chars()
            .any(|ch| ch.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&ch));
    printable.then_some(text)
}

fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        ctrl: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        meta: flags.contains(NSEventModifierFlags::Command),
    }
}

/// `insertText:` and `setMarkedText:` take an `NSString` or an
/// `NSAttributedString`.
fn ns_text(string: &AnyObject) -> String {
    if let Some(attributed) = string.downcast_ref::<NSAttributedString>() {
        attributed.string().to_string()
    } else if let Some(string) = string.downcast_ref::<NSString>() {
        string.to_string()
    } else {
        String::new()
    }
}

fn not_found() -> NSRange {
    NSRange::new(NSNotFound as NSUInteger, 0)
}

/// The byte offset `units` UTF-16 code units into `text`, clamped to its end.
fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (byte, ch) in text.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::utf16_to_byte;

    #[test]
    fn utf16_offsets_become_byte_offsets() {
        assert_eq!(utf16_to_byte("abc", 2), 2);
        assert_eq!(utf16_to_byte("中文", 1), 3);
        // One astral character is two UTF-16 units and four bytes.
        assert_eq!(utf16_to_byte("😀x", 2), 4);
        assert_eq!(utf16_to_byte("ab", 9), 2);
    }
}
