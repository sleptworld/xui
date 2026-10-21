use super::roles::{DefaultFocus, HandleEvent, PaintContent, PlatformTextInput, TextSource};
use crate::node::{Draft, WidgetT};
use std::time::{Duration, Instant};
use xui_macros::widget;

use xui_interface::events::{PointerButton, RawEvent, SemanticEvent, XuiPointerId};
use xui_interface::{
    Affine, Bounds, ClipboardContent, ClipboardTarget, Color, CommandId, ComputedStyle, EventRef,
    EventResult, PlatformCommand, PlatformReply, PlatformRequest, Point, RawIme, Rect, RequestId,
    TextCaret, TextContent, TextInputPurpose, TextInputSession, TextOffset, TextOffsetUnit,
    TextPaintProps, TextPaintStyle, TextProps, TextSelectionPaint, WhiteSpace, WidgetUpdateFlags,
    commands,
};

use crate::event_system::EventContext;
use crate::widgets::text_input::controller::ImeSession;

use super::text::apply_text_style;
use crate::render::{ClipShape, Primitive, RenderTreeWriter, TextPrimitive};
pub mod controller;
pub mod keymap;
pub mod value;
pub use controller::TextController;

#[derive(Clone, Debug)]
pub struct TextInputChange {
    pub text: String,
    pub controller: TextController,
}

/// A single-line text field. It paints its own text, caret and selection, so
/// it selects no decoration; a field's box is usually a container around it.
#[widget(
    style(
        box_size, margin, padding, position, display, typography, visibility, transform, effects,
        cursor
    ),
    behavior(
        focus,
        accessibility,
        shortcuts,
        motion,
        focus_binding,
        size_observation,
        events
    )
)]
#[props(derive(Debug, Clone, PartialEq))]
pub struct TextInput {
    /// Blocks editing and takes the field out of focus navigation.
    #[default]
    #[dirty(PAINT_OUTPUT)]
    pub disabled: bool,
    /// Allows selecting and copying, blocks editing, paste and IME.
    #[default]
    #[dirty(PAINT_OUTPUT)]
    pub read_only: bool,
    /// A controller the caller owns and reads; `None` edits one of the
    /// field's own.
    #[default]
    #[dirty(LAYOUT_INPUT | PAINT_OUTPUT)]
    pub controller: Option<TextController>,
    /// What the field's own controller starts with.
    #[default]
    #[dirty(LAYOUT_INPUT | PAINT_OUTPUT)]
    pub initial_text: String,
}

impl WidgetT for TextInput {
    type State = TextInputState;

    fn create_state(props: &TextInputProps) -> TextInputState {
        TextInputState::mounted(props)
    }

    fn update(
        _: &TextInputProps,
        new: &TextInputProps,
        state: &mut TextInputState,
    ) -> WidgetUpdateFlags {
        state.follow(new)
    }
}

/// A text field, at least 40 × 20 and showing the text cursor; both go
/// through the ordinary style, so a tag can override them.
pub fn text_input() -> Draft<TextInput> {
    Draft::new(TextInputProps::builder().build())
        .with::<text_input::min_width>(40.0)
        .with::<text_input::min_height>(20.0)
        .with::<text_input::cursor>(xui_interface::CursorIcon::Text)
}

/// Focusable unless disabled.
impl DefaultFocus for TextInputState {
    fn focusable(&self) -> bool {
        !self.disabled
    }
}

/// A text input's state while mounted: the controller it edits, its caret,
/// scroll and IME session. Its roles are implemented here, on the state.
pub struct TextInputState {
    controller: TextController,
    uses_external_controller: bool,
    last_text: String,
    focused: bool,
    disabled: bool,
    read_only: bool,
    scroll_x: f32,
    drag: Option<TextInputDrag>,
    ime_session: ImeSession,
    /// The clipboard read a paste is waiting on. A reply carrying any other id is
    /// not this widget's to act on.
    pending_paste: Option<RequestId>,
    /// Whether the caret is in the shown half of its blink. Only meaningful
    /// while focused; see [`TextInputState::restart_blink`].
    caret_on: bool,
}

/// How long the caret stays in each half of a blink.
///
/// Roughly what the platforms default to: 530ms on Windows, about 500ms on
/// macOS.
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextInputDrag {
    pointer_id: XuiPointerId,
    base: usize,
}

impl std::fmt::Debug for TextInputState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextInputState")
            .field("controller", &self.controller)
            .field("uses_external_controller", &self.uses_external_controller)
            .field("last_text", &self.last_text)
            .field("focused", &self.focused)
            .field("scroll_x", &self.scroll_x)
            .field("drag", &self.drag)
            .field("caret_on", &self.caret_on)
            .finish()
    }
}

/// The top of the single text line inside a content box `box_height` tall,
/// matching `TextVerticalAlign::Middle` in the render backends.
fn text_y_offset(box_height: f32, text_height: f32) -> f32 {
    ((box_height - text_height) * 0.5).max(0.0)
}

/// `rect` minus `padding`: the area the text is laid out, painted, and clipped
/// in. Padding stays part of the node, so pressing it still focuses the input.
pub(crate) fn text_input_content_box(rect: Bounds, padding: xui_interface::EdgeInsets) -> Bounds {
    Bounds::from_origin_size(
        Point::new(rect.x() + padding.left(), rect.y() + padding.top()),
        (
            (rect.width() - padding.left() - padding.right()).max(0.0),
            (rect.height() - padding.top() - padding.bottom()).max(0.0),
        ),
    )
}

/// `text` as one line: line breaks at either end are dropped, each run of them
/// inside becomes a single space, and each tab becomes a space.
///
/// What a single-line field does with a paste. Typing already drops the
/// characters it cannot show, but dropping them from a paste would run the last
/// word of one line into the first of the next. The ends are different: a line
/// copied whole usually carries its trailing break, which is not a space the
/// user meant to paste.
fn single_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_break = false;
    for ch in text.trim_matches(['\r', '\n']).chars() {
        match ch {
            '\r' | '\n' => {
                if !in_break {
                    out.push(' ');
                }
                in_break = true;
            }
            '\t' => {
                out.push(' ');
                in_break = false;
            }
            other => {
                out.push(other);
                in_break = false;
            }
        }
    }
    out
}

impl TextInputState {
    pub(crate) fn new() -> Self {
        Self {
            controller: TextController::new(),
            uses_external_controller: false,
            last_text: String::new(),
            focused: false,
            disabled: false,
            read_only: false,
            scroll_x: 0.0,
            drag: None,
            ime_session: ImeSession::new(),
            pending_paste: None,
            caret_on: true,
        }
    }

    /// The state for `props` on mount: their controller, or one holding their
    /// initial text.
    fn mounted(props: &TextInputProps) -> Self {
        let mut state = Self::new();
        match &props.controller {
            Some(controller) => {
                state.last_text = controller.text();
                state.controller = controller.clone();
                state.uses_external_controller = true;
            }
            None if !props.initial_text.is_empty() => {
                state.controller = TextController::with_text(&props.initial_text);
                state.last_text = props.initial_text.clone();
            }
            None => {}
        }
        state.disabled = props.disabled;
        state.read_only = props.read_only;
        state
    }

    /// Follows a commit's new props; returns what that dirties beyond the
    /// props themselves — an external controller whose text moved under it.
    fn follow(&mut self, next: &TextInputProps) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        if self.disabled != next.disabled || self.read_only != next.read_only {
            self.disabled = next.disabled;
            self.read_only = next.read_only;
            self.pending_paste = None;
        }

        match &next.controller {
            Some(controller) => {
                let next_text = controller.text();
                let mut controller_changed = false;
                if !self.controller.same_handle(controller) {
                    self.controller = controller.clone();
                    controller_changed = true;
                } else if next_text != self.last_text {
                    controller_changed = true;
                    flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
                }
                self.last_text = next_text;
                self.uses_external_controller = true;
                if controller_changed {
                    self.scroll_x = 0.0;
                }
            }
            None if self.uses_external_controller => {
                self.controller = TextController::with_text(&next.initial_text);
                self.last_text = self.controller.text();
                self.uses_external_controller = false;
                self.scroll_x = 0.0;
                flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
            }
            None => self.last_text = self.controller.text(),
        }
        flags
    }

    fn apply_text_edit(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        let previous_len = self.last_text.chars().count();
        let next_len = self.controller.len_chars();
        self.last_text = self.controller.text();
        self.ensure_caret_visible(cx);
        if next_len > previous_len {
            self.ensure_pending_caret_visible(cx, previous_len);
        }
        cx.mark_needs_text_shape();
        cx.mark_needs_paint();
        EventResult::Consumed
    }

    fn text_props(&self, style: &ComputedStyle) -> TextProps {
        let mut props = TextProps::new(TextContent::from(self.controller.text()));
        apply_text_style(&mut props, style);
        props.paragraph.white_space = WhiteSpace::NoWrap;
        props.text_box.max_lines = Some(1);
        props
    }

    /// The text area in window coordinates.
    fn content_box(&self, cx: &EventContext<'_>) -> Bounds {
        let layout = cx.node_ref.layout;
        text_input_content_box(
            Bounds::from_origin_size(cx.node_ref.world_origin, (layout.width(), layout.height())),
            cx.node_ref.effective_style.layout.padding,
        )
    }

    fn viewport_width(&self, cx: &EventContext<'_>) -> f32 {
        self.content_box(cx).width()
    }

    fn max_scroll_x(&self, cx: &EventContext<'_>) -> f32 {
        let content_width = cx
            .text_layout()
            .map(|layout| layout.size().width)
            .unwrap_or(0.0);
        (content_width - self.viewport_width(cx)).max(0.0)
    }

    fn clamp_scroll_x(&mut self, cx: &EventContext<'_>) -> bool {
        let old = self.scroll_x;
        self.scroll_x = self.scroll_x.clamp(0.0, self.max_scroll_x(cx));
        old != self.scroll_x
    }

    fn ensure_caret_visible(&mut self, cx: &EventContext<'_>) -> bool {
        let old = self.scroll_x;
        let viewport_width = self.viewport_width(cx);
        if viewport_width <= 0.0 {
            self.scroll_x = 0.0;
            return old != self.scroll_x;
        }

        if let Some(caret) = cx
            .text_layout()
            .and_then(|layout| layout.caret_rect(self.controller.selection().extent))
        {
            let caret_left = caret.x;
            let caret_right = caret.x + caret.width.max(1.0);
            if caret_left < self.scroll_x {
                self.scroll_x = caret_left;
            } else if caret_right > self.scroll_x + viewport_width {
                self.scroll_x = caret_right - viewport_width;
            }

            let caret_scroll_limit = (caret_right - viewport_width).max(0.0);
            let scroll_limit = self.max_scroll_x(cx).max(caret_scroll_limit);
            self.scroll_x = self.scroll_x.clamp(0.0, scroll_limit);
        } else {
            self.clamp_scroll_x(cx);
        }
        old != self.scroll_x
    }

    fn ensure_pending_caret_visible(&mut self, cx: &EventContext<'_>, previous_len: usize) -> bool {
        let viewport_width = self.viewport_width(cx);
        if viewport_width <= 0.0 {
            return false;
        }
        let Some(layout) = cx.text_layout() else {
            return false;
        };

        let average_advance = if previous_len == 0 {
            8.0
        } else {
            (layout.size().width / previous_len as f32).max(1.0)
        };
        let caret_x = self.controller.selection().extent as f32 * average_advance;
        let old = self.scroll_x;
        if caret_x < self.scroll_x {
            self.scroll_x = caret_x;
        } else if caret_x > self.scroll_x + viewport_width {
            self.scroll_x = caret_x - viewport_width;
        }
        self.scroll_x = self.scroll_x.max(0.0);
        old != self.scroll_x
    }

    fn event_point_to_text_point(&self, cx: &EventContext<'_>, position: Point) -> Point {
        let content = self.content_box(cx);
        let y_offset = cx
            .text_layout()
            .map(|layout| text_y_offset(content.height(), layout.size().height))
            .unwrap_or(0.0);
        Point::new(
            position.x - content.x() + self.scroll_x,
            position.y - content.y() - y_offset,
        )
    }

    fn hit_test_text(&self, cx: &EventContext<'_>, position: Point) -> usize {
        let point = self.event_point_to_text_point(cx, position);
        cx.text_layout()
            .and_then(|layout| layout.hit_test_point(point))
            .map(|position| match position.offset.unit {
                TextOffsetUnit::Char => position.offset.raw,
                TextOffsetUnit::Utf8Byte => self
                    .controller
                    .value()
                    .text
                    .byte_to_char(position.offset.raw),
                TextOffsetUnit::Utf16CodeUnit => self
                    .controller
                    .value()
                    .text
                    .byte_to_char(position.offset.raw),
            })
            .unwrap_or_else(|| self.controller.len_chars())
            .min(self.controller.len_chars())
    }

    fn set_selection_from_pointer(
        &mut self,
        cx: &mut EventContext<'_>,
        position: Point,
        shift: bool,
        drag_base: Option<usize>,
    ) -> EventResult {
        let hit = self.hit_test_text(cx, position);
        let selection = if let Some(base) = drag_base {
            value::TextSelection::new(base, hit)
        } else if shift {
            value::TextSelection::new(self.controller.selection().base, hit)
        } else {
            value::TextSelection::collapsed(hit)
        };

        if self.controller.set_selection(selection).is_ok() {
            self.ensure_caret_visible(cx);
            cx.mark_needs_paint();
            EventResult::Consumed
        } else {
            EventResult::Ignored
        }
    }

    fn scroll_for_drag(&mut self, cx: &mut EventContext<'_>, position: Point) -> bool {
        let viewport_width = self.viewport_width(cx);
        if viewport_width <= 0.0 {
            return false;
        }
        let local_x = position.x - self.content_box(cx).x();
        let old = self.scroll_x;
        if local_x < 0.0 {
            self.scroll_x += local_x;
        } else if local_x > viewport_width {
            self.scroll_x += local_x - viewport_width;
        }
        self.clamp_scroll_x(cx);
        old != self.scroll_x
    }

    fn set_cursor(
        &mut self,
        cx: &mut EventContext<'_>,
        offset: usize,
        extend: bool,
    ) -> EventResult {
        let len = self.controller.len_chars();
        let offset = offset.min(len);
        let selection = if extend {
            value::TextSelection::new(self.controller.selection().base, offset)
        } else {
            value::TextSelection::collapsed(offset)
        };

        if self.controller.set_selection(selection).is_ok() {
            self.ensure_caret_visible(cx);
            cx.mark_needs_paint();
            EventResult::Consumed
        } else {
            EventResult::Ignored
        }
    }

    fn move_left(&mut self, cx: &mut EventContext<'_>, extend: bool) -> EventResult {
        let selection = self.controller.selection();
        let offset = if !extend && !selection.is_collapsed() {
            selection.start()
        } else {
            selection.extent.saturating_sub(1)
        };
        self.set_cursor(cx, offset, extend)
    }

    fn move_right(&mut self, cx: &mut EventContext<'_>, extend: bool) -> EventResult {
        let selection = self.controller.selection();
        let offset = if !extend && !selection.is_collapsed() {
            selection.end()
        } else {
            selection.extent.saturating_add(1)
        };
        self.set_cursor(cx, offset, extend)
    }

    fn delete_forward(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        let selection = self.controller.selection();
        let range = selection.range();
        let result = if !range.is_empty() {
            self.controller.delete(range.as_range()).map(Some)
        } else if selection.extent < self.controller.len_chars() {
            self.controller
                .delete(selection.extent..selection.extent + 1)
                .map(Some)
        } else {
            Ok(None)
        };

        if matches!(result, Ok(Some(_))) {
            self.apply_text_edit(cx)
        } else {
            EventResult::Ignored
        }
    }

    fn select_all(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        let len = self.controller.len_chars();
        if self
            .controller
            .set_selection(value::TextSelection::new(0, len))
            .is_ok()
        {
            self.ensure_caret_visible(cx);
            cx.mark_needs_paint();
            EventResult::Consumed
        } else {
            EventResult::Ignored
        }
    }

    /// The selected text, if any is selected.
    fn selected_text(&self) -> Option<String> {
        let range = self.controller.selection().range();
        if range.is_empty() {
            return None;
        }
        self.controller
            .value()
            .text
            .get_slice(range.start..range.end)
            .map(|slice| slice.to_string())
    }

    /// Copies the selection to the clipboard. With nothing selected it does
    /// nothing, and leaves the clipboard as it was: a copy of nothing is not a
    /// request to empty it.
    fn copy_selection(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        let Some(text) = self.selected_text() else {
            return EventResult::Consumed;
        };
        if cx.send_platform(PlatformCommand::WriteClipboard {
            target: ClipboardTarget::General,
            content: ClipboardContent::Text(text),
        }) {
            EventResult::Consumed
        } else {
            EventResult::Ignored
        }
    }

    fn cut_selection(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        let Some(text) = self.selected_text() else {
            return EventResult::Consumed;
        };
        if !cx.send_platform(PlatformCommand::WriteClipboard {
            target: ClipboardTarget::General,
            content: ClipboardContent::Text(text),
        }) {
            return EventResult::Ignored;
        }
        let range = self.controller.selection().range();
        if self.controller.delete(range.as_range()).is_ok() {
            self.apply_text_edit(cx)
        } else {
            EventResult::Consumed
        }
    }

    /// Asks for the clipboard. What comes back is handled in
    /// [`Self::insert_pasted`], when the reply arrives as an event.
    fn request_paste(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        match cx.request_platform(PlatformRequest::ReadClipboard(ClipboardTarget::General)) {
            Some(id) => {
                self.pending_paste = Some(id);
                EventResult::Consumed
            }
            None => EventResult::Ignored,
        }
    }

    /// Puts pasted text at the caret, over the selection if there is one.
    fn insert_pasted(&mut self, cx: &mut EventContext<'_>, text: &str) -> EventResult {
        let line = single_line(text);
        if line.is_empty() {
            return EventResult::Ignored;
        }
        if self.controller.insert_text(line).is_ok() {
            self.apply_text_edit(cx)
        } else {
            EventResult::Ignored
        }
    }

    fn undo_redo(&mut self, cx: &mut EventContext<'_>, redo: bool) -> EventResult {
        let changed = if redo {
            self.controller.redo().unwrap_or(false)
        } else {
            self.controller.undo().unwrap_or(false)
        };
        if changed {
            self.apply_text_edit(cx)
        } else {
            EventResult::Ignored
        }
    }

    /// One of the standard edit commands, however it arrived: from the key
    /// bound to it, or from the platform raising it through a menu item.
    fn handle_edit_command(
        &mut self,
        command: CommandId,
        cx: &mut EventContext<'_>,
    ) -> Option<EventResult> {
        if self.read_only
            && [
                commands::CUT,
                commands::PASTE,
                commands::UNDO,
                commands::REDO,
            ]
            .contains(&command)
        {
            return Some(EventResult::Ignored);
        }
        Some(match command {
            c if c == commands::COPY => self.copy_selection(cx),
            c if c == commands::CUT => self.cut_selection(cx),
            c if c == commands::PASTE => self.request_paste(cx),
            c if c == commands::SELECT_ALL => self.select_all(cx),
            c if c == commands::UNDO => self.undo_redo(cx, false),
            c if c == commands::REDO => self.undo_redo(cx, true),
            _ => return None,
        })
    }

    /// Shows the caret and starts its blink over from `at`.
    ///
    /// Called whenever the user does something to the field, so the caret
    /// holds still while it is being typed into or moved, and only blinks once
    /// the field is left alone -- what every platform's text field does.
    ///
    /// The blink is a wake per half-period, never a frame per display refresh:
    /// a focused field sitting idle costs two frames a second. On a host that
    /// cannot schedule a frame for a deadline the caret stays shown instead,
    /// since a wake there would only arrive with whatever event came next.
    fn restart_blink(&mut self, cx: &mut EventContext<'_>, at: Instant) {
        if !self.caret_on {
            self.caret_on = true;
            cx.mark_needs_paint();
        }
        if cx
            .host_capabilities()
            .contains(xui_interface::HostCapabilities::TIMED_REDRAW)
        {
            cx.request_wake_at(at + CARET_BLINK_INTERVAL);
        }
    }

    /// Whether the caret would be painted now.
    #[cfg(test)]
    pub(crate) fn caret_shown(&self) -> bool {
        self.focused && self.caret_on
    }

    /// Stops blinking. The caret is left in its shown state, for the next time
    /// the field is focused.
    fn stop_blink(&mut self, cx: &mut EventContext<'_>) {
        self.caret_on = true;
        cx.cancel_wake();
    }

    /// One half-period of the blink has passed.
    fn blink(&mut self, wake: &xui_interface::WakeEvent, cx: &mut EventContext<'_>) -> EventResult {
        if !self.focused {
            return EventResult::Ignored;
        }
        self.caret_on = !self.caret_on;
        cx.mark_needs_paint();
        // From the deadline, so a late frame does not push every later blink
        // back with it. A wake that arrives more than a period late -- the
        // window was off screen -- starts over from now instead of firing a
        // burst to catch up.
        let mut next = wake.deadline + CARET_BLINK_INTERVAL;
        if next <= wake.meta.timestamp {
            next = wake.meta.timestamp + CARET_BLINK_INTERVAL;
        }
        cx.request_wake_at(next);
        EventResult::Consumed
    }

    fn handle_key_down(
        &mut self,
        key: &xui_interface::events::RawKeyboard,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        let command = keymap::TextKeymap::platform_default().resolve(key);
        if self.read_only
            && matches!(
                command,
                Some(
                    keymap::TextCommand::Undo
                        | keymap::TextCommand::Redo
                        | keymap::TextCommand::DeleteBackward
                        | keymap::TextCommand::DeleteForward
                        | keymap::TextCommand::Cut
                        | keymap::TextCommand::Paste
                )
            )
        {
            return EventResult::Ignored;
        }
        match command {
            Some(keymap::TextCommand::SelectAll) => self.select_all(cx),
            Some(keymap::TextCommand::Undo) => self.undo_redo(cx, false),
            Some(keymap::TextCommand::Redo) => self.undo_redo(cx, true),
            Some(keymap::TextCommand::MoveLeft { extend }) => self.move_left(cx, extend),
            Some(keymap::TextCommand::MoveRight { extend }) => self.move_right(cx, extend),
            Some(keymap::TextCommand::MoveHome { extend }) => self.set_cursor(cx, 0, extend),
            Some(keymap::TextCommand::MoveEnd { extend }) => {
                self.set_cursor(cx, self.controller.len_chars(), extend)
            }
            Some(keymap::TextCommand::DeleteBackward) => {
                if matches!(self.controller.backspace(), Ok(Some(_))) {
                    self.apply_text_edit(cx)
                } else {
                    EventResult::Ignored
                }
            }
            Some(keymap::TextCommand::DeleteForward) => self.delete_forward(cx),
            Some(keymap::TextCommand::Copy) => self.copy_selection(cx),
            Some(keymap::TextCommand::Cut) => self.cut_selection(cx),
            Some(keymap::TextCommand::Paste) => self.request_paste(cx),
            None => EventResult::Ignored,
        }
    }
}

impl Default for TextInputState {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInputState {
    fn handle_input(&mut self, event: EventRef<'_>, cx: &mut EventContext<'_>) -> EventResult {
        if self.disabled
            && !matches!(
                event,
                EventRef::Semantic(SemanticEvent::Blur(_) | SemanticEvent::FocusOut(_))
            )
        {
            return EventResult::Ignored;
        }
        match event {
            EventRef::Raw(RawEvent::PointerDown(pointer))
                if pointer.button == PointerButton::Primary =>
            {
                self.focused = true;
                let result = self.set_selection_from_pointer(
                    cx,
                    pointer.position,
                    pointer.modifiers.shift,
                    None,
                );
                self.drag = Some(TextInputDrag {
                    pointer_id: pointer.pointer_id,
                    base: self.controller.selection().base,
                });
                cx.capture_pointer();
                cx.mark_needs_paint();
                result
            }
            EventRef::Raw(RawEvent::PointerMove(pointer)) => {
                let Some(drag) = self
                    .drag
                    .filter(|drag| drag.pointer_id == pointer.pointer_id)
                else {
                    return EventResult::Ignored;
                };
                if self.scroll_for_drag(cx, pointer.position) {
                    cx.mark_needs_paint();
                }
                self.set_selection_from_pointer(cx, pointer.position, true, Some(drag.base))
            }
            EventRef::Raw(RawEvent::PointerUp(pointer))
                if self
                    .drag
                    .is_some_and(|drag| drag.pointer_id == pointer.pointer_id) =>
            {
                self.drag = None;
                cx.release_pointer_capture();
                EventResult::Consumed
            }
            EventRef::Raw(RawEvent::PointerCancel(pointer))
                if self
                    .drag
                    .is_some_and(|drag| drag.pointer_id == pointer.pointer_id) =>
            {
                self.drag = None;
                cx.release_pointer_capture();
                EventResult::Consumed
            }
            EventRef::Raw(RawEvent::Keyboard(input))
                if input.state == xui_interface::events::KeyState::Down =>
            {
                if self.read_only {
                    return self.handle_key_down(input, cx);
                }
                if keymap::TextKeymap::platform_default()
                    .resolve(input)
                    .is_some()
                {
                    return self.handle_key_down(input, cx);
                }
                if input.modifiers.ctrl || input.modifiers.meta {
                    return self.handle_key_down(input, cx);
                }
                let Some(text) = input.text else {
                    return self.handle_key_down(input, cx);
                };
                let filtered: String = text
                    .as_str()
                    .chars()
                    .filter(|ch| *ch != '\r' && *ch != '\n' && *ch != '\t')
                    .collect();
                if !filtered.is_empty() && self.controller.insert_text(filtered).is_ok() {
                    self.apply_text_edit(cx)
                } else {
                    self.handle_key_down(input, cx)
                }
            }
            EventRef::Raw(RawEvent::Ime(e)) => {
                if self.read_only {
                    return EventResult::Ignored;
                }
                match e {
                    RawIme::Enabled { .. } => {}
                    RawIme::Preedit {
                        text,
                        cursor,
                        timestamp: _,
                    } => {
                        if !self.ime_session.is_active() {
                            self.ime_session.begin(&self.controller).unwrap();
                        }
                        if self
                            .ime_session
                            .preedit(&self.controller, text, *cursor)
                            .is_ok()
                        {
                            self.apply_text_edit(cx);
                        }
                    }
                    RawIme::Commit { text, timestamp: _ } => {
                        if self.ime_session.commit(&self.controller, text).is_ok() {
                            self.apply_text_edit(cx);
                        }
                    }
                    RawIme::Disabled { timestamp: _ } => {
                        let _ = self.ime_session.end(&self.controller);
                    }
                }
                EventResult::Consumed
            }
            EventRef::Semantic(SemanticEvent::Command(event)) => self
                .handle_edit_command(event.command, cx)
                .unwrap_or(EventResult::Ignored),
            EventRef::Semantic(SemanticEvent::PlatformReply(reply))
                if self.pending_paste == Some(reply.request) =>
            {
                self.pending_paste = None;
                match &reply.reply {
                    PlatformReply::Clipboard(Some(ClipboardContent::Text(text)))
                        if !self.read_only =>
                    {
                        self.insert_pasted(cx, text)
                    }
                    // Nothing on the clipboard, or no clipboard to ask: a paste
                    // that has nothing to paste.
                    _ => EventResult::Ignored,
                }
            }
            EventRef::Semantic(SemanticEvent::Focus(_))
            | EventRef::Semantic(SemanticEvent::FocusIn(_)) => {
                self.focused = true;
                self.ensure_caret_visible(cx);
                cx.mark_needs_paint();
                EventResult::Ignored
            }
            EventRef::Semantic(SemanticEvent::Blur(_))
            | EventRef::Semantic(SemanticEvent::FocusOut(_)) => {
                self.focused = false;
                self.drag = None;
                cx.release_pointer_capture();
                cx.mark_needs_paint();
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }
}

impl PaintContent for TextInputState {
    fn paint(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        let rect = text_input_content_box(rect, style.layout.padding);
        let mut paint = TextPaintProps::new(TextPaintStyle::from_computed(&style.text));
        paint.caret = (self.focused && self.caret_on).then_some(TextCaret {
            char_index: self.controller.selection().extent,
            color: style.text.color,
            width: 1.0,
        });
        let selection = self.controller.selection().range();
        if self.focused && !selection.is_empty() {
            paint.selection = Some(TextSelectionPaint {
                range: xui_interface::TextRange::new(
                    TextOffset::char_offset(selection.start),
                    TextOffset::char_offset(selection.end),
                ),
                color: Color::rgba(0.18, 0.42, 0.88, 0.28),
            });
        }
        writer
            .clip(ClipShape::Rect(rect), |writer| {
                writer.transform(Affine::translate(-self.scroll_x, 0.0), |writer| {
                    writer.primitive(Primitive::Text(TextPrimitive {
                        node_id,
                        bounds: Bounds::from_origin_size(
                            rect.origin(),
                            (rect.width() + self.scroll_x, rect.height()),
                        ),
                        slot: crate::text::TextLayoutSlot::PRIMARY,
                        layout_revision: self.controller.revision(),
                        // Centered, so a box taller than one line keeps
                        // its text, caret, and selection on the midline.
                        vertical_align: xui_interface::TextVerticalAlign::Middle,
                        paint,
                    }))?;
                    Ok(())
                })?;
                Ok(())
            })
            .expect("widget render tree must remain valid");
    }
}

impl TextSource for TextInputState {
    fn text(&self) -> TextContent {
        TextContent::from(self.controller.text())
    }

    fn text_layout(&self, style: &ComputedStyle) -> TextProps {
        self.text_props(style)
    }
}

impl HandleEvent for TextInputState {
    fn handle_event(&mut self, event: EventRef<'_>, cx: &mut EventContext<'_>) -> EventResult {
        if let EventRef::Semantic(SemanticEvent::Wake(wake)) = event {
            return self.blink(wake, cx);
        }
        let was_focused = self.focused;
        let result = self.handle_input(event, cx);
        if self.focused {
            // Gaining focus, or anything the field acted on.
            if !was_focused || result.is_consumed() {
                let at = match event {
                    EventRef::Raw(raw) => raw.timestamp(),
                    EventRef::Semantic(semantic) => semantic.meta().timestamp,
                };
                self.restart_blink(cx, at);
            }
        } else if was_focused {
            self.stop_blink(cx);
        }
        result
    }

    /// Reads key and pointer input raw, for editing and selection.
    fn raw_events(&self) -> bool {
        true
    }
}

impl PlatformTextInput for TextInputState {
    /// `content_rect` is the node's rect inside its padding
    /// ([`text_input_content_box`]).
    fn text_input_session(
        &self,
        content_rect: Bounds,
        text_layout: &dyn crate::text::TextLayoutQuery,
    ) -> TextInputSession {
        let mut cursor_area = text_layout
            .caret_rect(self.controller.selection().extent)
            .unwrap_or_else(|| Rect::new(0.0, 0.0, 1.0, content_rect.height().max(1.0)));
        cursor_area.x += content_rect.x() - self.scroll_x;
        cursor_area.y +=
            content_rect.y() + text_y_offset(content_rect.height(), text_layout.size().height);
        cursor_area.width = cursor_area.width.max(1.0);
        cursor_area.height = cursor_area.height.max(1.0);

        TextInputSession {
            cursor_area,
            purpose: TextInputPurpose::Normal,
            multiline: false,
        }
    }
}

#[cfg(test)]
mod single_line_tests {
    use super::single_line;

    #[test]
    fn interior_breaks_become_one_space_and_end_breaks_vanish() {
        assert_eq!(single_line("one\r\ntwo\n\nthree"), "one two three");
        assert_eq!(single_line("a whole line\n"), "a whole line");
        assert_eq!(single_line("\r\nleading"), "leading");
        assert_eq!(single_line("\n\r\n"), "");
    }

    #[test]
    fn tabs_become_spaces_and_spaces_are_kept() {
        assert_eq!(single_line("a\tb"), "a b");
        assert_eq!(single_line(" "), " ");
        assert_eq!(single_line("  padded  "), "  padded  ");
    }
}
