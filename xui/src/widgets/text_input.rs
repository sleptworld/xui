use std::cell::RefCell;
use std::rc::Rc;

use ropey::Rope;
use xui_interface::events::{Key as InputKey, RawEvent, SemanticEvent};
use xui_interface::{
    ComputedStyle, EventContext, EventRef, EventResult, Key, PaintCommand, Rect, Style, TextCaret,
    TextContent, TextPaintCommand, TextPaintProps, TextPaintStyle, TextProps, Widget, WidgetType,
    WidgetUpdateFlags,
};

use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;

use super::text::apply_text_style;
use super::{props_hash, widget_element_desc};
pub mod controller;
pub mod value;

type TextInputChangeHandler =
    Rc<RefCell<dyn for<'a> FnMut(&TextInputChange, &mut EventContext<'a>) -> EventResult>>;

#[derive(Clone, Debug)]
pub struct TextController {
    inner: Rc<RefCell<TextControllerState>>,
}

#[derive(Debug)]
struct TextControllerState {
    buffer: Rope,
    cursor_char: usize,
}

impl TextController {
    pub fn new() -> Self {
        Self::with_text("")
    }

    pub fn with_text(text: impl AsRef<str>) -> Self {
        let text = text.as_ref();
        Self {
            inner: Rc::new(RefCell::new(TextControllerState {
                buffer: Rope::from_str(text),
                cursor_char: text.chars().count(),
            })),
        }
    }

    pub fn text(&self) -> String {
        self.inner.borrow().buffer.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.inner.borrow().buffer.len_chars()
    }

    pub fn cursor(&self) -> usize {
        self.inner.borrow().cursor_char
    }

    pub fn set_cursor_to_end(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.cursor_char = inner.buffer.len_chars();
    }

    pub fn insert_text(&self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }

        let mut inner = self.inner.borrow_mut();
        let cursor = inner.cursor_char.min(inner.buffer.len_chars());
        inner.buffer.insert(cursor, text);
        inner.cursor_char = cursor + text.chars().count();
        true
    }

    pub fn backspace(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        if inner.cursor_char == 0 {
            return false;
        }

        let cursor = inner.cursor_char.min(inner.buffer.len_chars());
        if cursor == 0 {
            inner.cursor_char = 0;
            return false;
        }

        inner.buffer.remove(cursor - 1..cursor);
        inner.cursor_char = cursor - 1;
        true
    }

    pub fn set_text(&self, text: impl AsRef<str>) {
        let text = text.as_ref();
        let mut inner = self.inner.borrow_mut();
        inner.buffer = Rope::from_str(text);
        inner.cursor_char = inner.buffer.len_chars();
    }

    fn same_handle(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Default for TextController {
    fn default() -> Self {
        Self::new()
    }
}

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
    on_changed: Option<TextInputChangeHandler>,
    uses_external_controller: bool,
    last_text: String,
    focused: bool,
}

impl std::fmt::Debug for TextInputWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextInputWidget")
            .field("key", &self.key)
            .field("controller", &self.controller)
            .field("style", &self.style)
            .field("event_handlers", &self.event_handlers)
            .field("on_changed", &self.on_changed.is_some())
            .field("uses_external_controller", &self.uses_external_controller)
            .field("last_text", &self.last_text)
            .field("focused", &self.focused)
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
            on_changed: None,
            uses_external_controller: false,
            last_text: String::new(),
            focused: false,
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

    pub fn on_changed(
        mut self,
        handler: impl for<'a> FnMut(&TextInputChange, &mut EventContext<'a>) -> EventResult + 'static,
    ) -> Self {
        self.on_changed = Some(Rc::new(RefCell::new(handler)));
        self
    }

    pub fn into_element_desc(self) -> ElementDesc {
        widget_element_desc(self, Vec::new())
    }

    event_handler_methods!();

    fn emit_changed(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        let Some(handler) = self.on_changed.as_ref().cloned() else {
            return EventResult::Consumed;
        };

        let change = TextInputChange {
            text: self.controller.text(),
            controller: self.controller.clone(),
        };
        (handler.borrow_mut())(&change, cx)
    }

    fn apply_text_edit(&mut self, cx: &mut EventContext<'_>) -> EventResult {
        self.last_text = self.controller.text();
        cx.mark_needs_layout();
        cx.mark_needs_paint();
        let result = self.emit_changed(cx);
        if result.is_consumed() {
            result
        } else {
            EventResult::Consumed
        }
    }

    fn text_props(&self, style: &ComputedStyle) -> TextProps {
        let mut props = TextProps::new(TextContent::from(self.controller.text()));
        apply_text_style(&mut props, style);
        props
    }
}

impl Default for TextInputWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TextInputWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::TextInput
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.controller.text(),
            &self.style,
            self.uses_external_controller,
            self.focused,
            self.on_changed.is_some(),
        ))
    }

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        let next_text = next.controller.text();

        if next.uses_external_controller {
            if !self.controller.same_handle(&next.controller) {
                self.controller = next.controller.clone();
                flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
            } else if next_text != self.last_text {
                flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
            }
            self.last_text = next_text;
            self.uses_external_controller = true;
        } else if self.uses_external_controller {
            self.controller = next.controller.clone();
            self.last_text = self.controller.text();
            self.uses_external_controller = false;
            flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
        } else {
            self.last_text = self.controller.text();
        }

        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }
        self.on_changed = next.on_changed.clone();

        flags
    }

    fn default_style(&self) -> Style {
        Style::new().min_width(40.0).min_height(20.0)
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        let mut paint = TextPaintProps::new(TextPaintStyle::from_computed(&style.text));
        paint.caret = self.focused.then_some(TextCaret {
            char_index: self.controller.cursor(),
            color: style.text.color,
            width: 1.0,
        });
        commands.push(PaintCommand::Text(TextPaintCommand {
            node_id: Default::default(),
            rect,
            paint,
        }));
    }

    fn handle_event(&mut self, event: EventRef<'_>, cx: &mut EventContext<'_>) -> EventResult {
        match event {
            EventRef::Raw(RawEvent::PointerDown(_)) => {
                self.controller.set_cursor_to_end();
                cx.request_focus();
                cx.mark_needs_paint();
                EventResult::Ignored
            }
            EventRef::Raw(RawEvent::TextInput(input)) => {
                let filtered: String = input
                    .text
                    .chars()
                    .filter(|ch| *ch != '\r' && *ch != '\n')
                    .collect();
                if self.controller.insert_text(&filtered) {
                    self.apply_text_edit(cx)
                } else {
                    EventResult::Ignored
                }
            }
            EventRef::Raw(RawEvent::KeyDown(key)) if key.key == InputKey::Backspace => {
                if self.controller.backspace() {
                    self.apply_text_edit(cx)
                } else {
                    EventResult::Ignored
                }
            }
            EventRef::Semantic(SemanticEvent::Focus(_))
            | EventRef::Semantic(SemanticEvent::FocusIn(_)) => {
                self.focused = true;
                self.controller.set_cursor_to_end();
                cx.request_focus();
                cx.mark_needs_paint();
                EventResult::Ignored
            }
            EventRef::Semantic(SemanticEvent::Blur(_))
            | EventRef::Semantic(SemanticEvent::FocusOut(_)) => {
                self.focused = false;
                cx.clear_focus();
                cx.mark_needs_paint();
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn text(&self) -> Option<TextContent> {
        Some(TextContent::from(self.controller.text()))
    }

    fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
        Some(self.text_props(style))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use xui_interface::events::{EventPhase, EventRequests, Modifiers, RawKey, RawTextInput};

    #[test]
    fn controller_edits_by_char_index() {
        let controller = TextController::with_text("a好🙂");

        assert_eq!(controller.len_chars(), 3);
        assert_eq!(controller.cursor(), 3);

        assert!(controller.backspace());
        assert_eq!(controller.text(), "a好");
        assert_eq!(controller.cursor(), 2);

        assert!(controller.insert_text("界🙂"));
        assert_eq!(controller.text(), "a好界🙂");
        assert_eq!(controller.len_chars(), 4);
        assert_eq!(controller.cursor(), 4);
    }

    #[test]
    fn internal_controller_survives_update_from_internal_widget() {
        let mut current = TextInputWidget::with_text("typed");
        assert!(current.controller.insert_text("!"));
        let next = TextInputWidget::with_text("initial");

        let flags = current.update_from(&next);

        assert!(flags.is_empty());
        assert_eq!(current.controller.text(), "typed!");
        assert!(!current.uses_external_controller);
    }

    #[test]
    fn external_controller_replaces_current_controller() {
        let first = TextController::with_text("first");
        let second = TextController::with_text("second");
        let mut current = TextInputWidget::new().controller(first.clone());
        let next = TextInputWidget::new().controller(second.clone());

        let flags = current.update_from(&next);

        assert!(flags.contains(WidgetUpdateFlags::LAYOUT_INPUT));
        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT));
        assert_eq!(current.controller.text(), "second");
        assert!(current.controller.same_handle(&second));
    }

    #[test]
    fn raw_text_input_filters_newlines_and_marks_dirty() {
        let mut widget = TextInputWidget::new();
        let mut update = WidgetUpdateFlags::empty();
        let mut requests = EventRequests::default();
        let mut cx = EventContext::new(
            Default::default(),
            EventPhase::Target,
            &mut update,
            &mut requests,
        );

        let result = widget.handle_event(
            EventRef::Raw(&RawEvent::TextInput(RawTextInput {
                text: "a\n好\r🙂".to_owned(),
                modifiers: Modifiers::default(),
                timestamp: Instant::now(),
            })),
            &mut cx,
        );

        assert_eq!(result, EventResult::Consumed);
        assert_eq!(widget.controller.text(), "a好🙂");
        assert!(update.contains(WidgetUpdateFlags::LAYOUT_INPUT));
        assert!(update.contains(WidgetUpdateFlags::PAINT_OUTPUT));
    }

    #[test]
    fn backspace_consumes_only_when_text_changes() {
        let mut widget = TextInputWidget::with_text("a好");
        let mut update = WidgetUpdateFlags::empty();
        let mut requests = EventRequests::default();
        let mut cx = EventContext::new(
            Default::default(),
            EventPhase::Target,
            &mut update,
            &mut requests,
        );

        let raw = RawEvent::KeyDown(RawKey {
            key: InputKey::Backspace,
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
            is_repeat: false,
        });

        let result = widget.handle_event(EventRef::Raw(&raw), &mut cx);

        assert_eq!(result, EventResult::Consumed);
        assert_eq!(widget.controller.text(), "a");
        assert!(update.contains(WidgetUpdateFlags::LAYOUT_INPUT));
        assert!(update.contains(WidgetUpdateFlags::PAINT_OUTPUT));
    }
}
