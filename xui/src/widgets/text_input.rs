use xui_interface::events::{PointerButton, RawEvent, SemanticEvent, XuiPointerId};
use xui_interface::{
    Affine, Bounds, Color, ComputedStyle, EventRef, EventResult, Key, Point, RawIme, Rect, Style,
    TextCaret, TextContent, TextInputPurpose, TextInputSession, TextOffset, TextOffsetUnit,
    TextPaintProps, TextPaintStyle, TextProps, TextSelectionPaint, WhiteSpace, WidgetType,
    WidgetUpdateFlags,
};

use crate::element::ElementDesc;
use crate::event_system::EventContext;
use crate::event_system::callbacks::EventHandlers;
use crate::event_system::interaction::InteractionProperties;
use crate::widgets::text_input::controller::ImeSession;

use super::text::apply_text_style;
use super::{props_hash, widget_element_desc};
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

pub struct TextInputWidget {
    pub key: Option<Key>,
    pub controller: TextController,
    pub style: Style,
    pub event_handlers: EventHandlers,
    pub interaction: InteractionProperties,
    uses_external_controller: bool,
    last_text: String,
    focused: bool,
    scroll_x: f32,
    drag: Option<TextInputDrag>,
    ime_session: ImeSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextInputDrag {
    pointer_id: XuiPointerId,
    base: usize,
}

impl std::fmt::Debug for TextInputWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextInputWidget")
            .field("key", &self.key)
            .field("controller", &self.controller)
            .field("style", &self.style)
            .field("event_handlers", &self.event_handlers)
            .field("uses_external_controller", &self.uses_external_controller)
            .field("last_text", &self.last_text)
            .field("focused", &self.focused)
            .field("scroll_x", &self.scroll_x)
            .field("drag", &self.drag)
            .finish()
    }
}

impl TextInputWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            controller: TextController::new(),
            style: Style::default(),
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
            uses_external_controller: false,
            last_text: String::new(),
            focused: false,
            scroll_x: 0.0,
            drag: None,
            ime_session: ImeSession::new(),
        }
    }

    pub fn with_text(initial_text: impl AsRef<str>) -> Self {
        let initial_text = initial_text.as_ref();
        Self {
            controller: TextController::with_text(initial_text),
            last_text: initial_text.to_owned(),
            ..Self::new()
        }
    }

    pub fn controller(mut self, controller: TextController) -> Self {
        self.last_text = controller.text();
        self.controller = controller;
        self.uses_external_controller = true;
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self) -> ElementDesc {
        widget_element_desc(self, Vec::new())
    }

    event_handler_methods!();

    pub(crate) fn platform_text_input_session(
        &self,
        node_rect: Bounds,
        text_layout: &dyn crate::text::TextLayoutQuery,
    ) -> TextInputSession {
        let mut cursor_area = text_layout
            .caret_rect(self.controller.selection().extent)
            .unwrap_or_else(|| Rect::new(0.0, 0.0, 1.0, node_rect.height().max(1.0)));
        cursor_area.x += node_rect.x() - self.scroll_x;
        cursor_area.y += node_rect.y();
        cursor_area.width = cursor_area.width.max(1.0);
        cursor_area.height = cursor_area.height.max(1.0);

        TextInputSession {
            cursor_area,
            purpose: TextInputPurpose::Normal,
            multiline: false,
        }
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

    fn viewport_width(&self, cx: &EventContext<'_>) -> f32 {
        cx.node_ref.layout.width().max(0.0)
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
        Point::new(
            position.x - cx.node_ref.world_origin.x + self.scroll_x,
            position.y - cx.node_ref.world_origin.y,
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
        let local_x = position.x - cx.node_ref.layout.x();
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

    fn handle_key_down(
        &mut self,
        key: &xui_interface::events::RawKeyboard,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        match keymap::TextKeymap::platform_default().resolve(key) {
            Some(keymap::TextCommand::SelectAll) => self.select_all(cx),
            Some(command @ (keymap::TextCommand::Undo | keymap::TextCommand::Redo)) => {
                let changed = if matches!(command, keymap::TextCommand::Redo) {
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
            Some(
                keymap::TextCommand::Copy | keymap::TextCommand::Cut | keymap::TextCommand::Paste,
            ) => EventResult::Consumed,
            None => EventResult::Ignored,
        }
    }
}

impl Default for TextInputWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInputWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::TextInput
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.controller.text(),
            &self.style,
            self.uses_external_controller,
            self.focused,
            self.controller.selection().base,
            self.controller.selection().extent,
            self.scroll_x.to_bits(),
        ))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        let next_text = next.controller.text();

        if next.uses_external_controller {
            let mut controller_changed = false;
            if !self.controller.same_handle(&next.controller) {
                self.controller = next.controller.clone();
                controller_changed = true;
                flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
            } else if next_text != self.last_text {
                controller_changed = true;
                flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
            }
            self.last_text = next_text;
            self.uses_external_controller = true;
            if controller_changed {
                self.scroll_x = 0.0;
            }
        } else if self.uses_external_controller {
            self.controller = next.controller.clone();
            self.last_text = self.controller.text();
            self.uses_external_controller = false;
            self.scroll_x = 0.0;
            flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
        } else {
            self.last_text = self.controller.text();
        }

        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }

        flags
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new().min_width(40.0).min_height(20.0)
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        let mut paint = TextPaintProps::new(TextPaintStyle::from_computed(&style.text));
        paint.caret = self.focused.then_some(TextCaret {
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
                        vertical_align: xui_interface::TextVerticalAlign::Baseline,
                        paint,
                    }))?;
                    Ok(())
                })?;
                Ok(())
            })
            .expect("widget render tree must remain valid");
    }

    pub(super) fn handle_event(
        &mut self,
        event: EventRef<'_>,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
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
                        self.ime_session.end(&self.controller);
                    }
                }
                EventResult::Consumed
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

    pub(super) fn text_content(&self) -> Option<TextContent> {
        Some(TextContent::from(self.controller.text()))
    }

    pub(super) fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
        Some(self.text_props(style))
    }
}
