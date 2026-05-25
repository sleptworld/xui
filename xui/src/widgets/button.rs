use std::any::Any;
use std::fmt::Debug;

use taffy::prelude as tf;
use xui_interface::{
    ColorToken, ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key,
    PaintCommand, PointerButton, Rect, Size, Style, TextContent, TextMeasurer, TextPaintCommand,
    TextProps, Widget, WidgetState, WidgetType,
};

use super::{
    LayoutStyledWidget, container::paint_box, fixed_size_style, label::apply_text_style, props_hash,
};

pub struct ButtonWidget {
    pub key: Option<Key>,
    pub text: TextContent,
    pub style: Style,
    pub hover_style: Style,
    pub pressed_style: Style,
    pub disabled_style: Style,
    pub event_handlers: EventHandlers,
    pub pressed: bool,
    pub hovered: bool,
    pub disabled: bool,
}

impl ButtonWidget {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            key: None,
            text: text.into(),
            style: Style::new(),
            hover_style: Style::new(),
            pressed_style: Style::new(),
            disabled_style: Style::new(),
            event_handlers: EventHandlers::default(),
            pressed: false,
            hovered: false,
            disabled: false,
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn hover_style(mut self, style: Style) -> Self {
        self.hover_style = style;
        self
    }

    pub fn pressed_style(mut self, style: Style) -> Self {
        self.pressed_style = style;
        self
    }

    pub fn disabled_style(mut self, style: Style) -> Self {
        self.disabled_style = style;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

impl LayoutStyledWidget for ButtonWidget {
    fn layout_style(&self, computed: &ComputedStyle, measurer: &mut dyn TextMeasurer) -> tf::Style {
        self.measure(computed, measurer)
            .map(fixed_size_style)
            .unwrap_or_default()
    }
}

impl Debug for ButtonWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("")
    }
}

impl Widget for ButtonWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Button
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.text,
            &self.style,
            &self.hover_style,
            &self.pressed_style,
            &self.disabled_style,
            self.disabled,
        ))
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<ButtonWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        if self.text != next.text
            || self.style != next.style
            || self.hover_style != next.hover_style
            || self.pressed_style != next.pressed_style
            || self.disabled_style != next.disabled_style
            || self.disabled != next.disabled
        {
            self.text = next.text.clone();
            self.style = next.style.clone();
            self.hover_style = next.hover_style.clone();
            self.pressed_style = next.pressed_style.clone();
            self.disabled_style = next.disabled_style.clone();
            self.disabled = next.disabled;
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn default_style(&self) -> Style {
        Style::new()
            .background(ColorToken::Surface)
            .border_color(ColorToken::Border)
            .border_width(1.0)
            .padding(crate::core::EdgeInsets {
                left: 8.0,
                right: 8.0,
                top: 4.0,
                bottom: 4.0,
            })
            .color(ColorToken::Text)
            .font_size(14.0)
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn state_style(&self, state: WidgetState) -> Style {
        let mut style = Style::new();
        if state.hovered {
            style.merge(&Style::new().background(ColorToken::MutedSurface));
            style.merge(&self.hover_style);
        }
        if state.pressed {
            style.merge(
                &Style::new()
                    .background(ColorToken::Primary)
                    .color(ColorToken::InverseText),
            );
            style.merge(&self.pressed_style);
        }
        if state.disabled {
            style.merge(&self.disabled_style);
        }
        style
    }

    fn state(&self) -> WidgetState {
        WidgetState {
            hovered: self.hovered,
            pressed: self.pressed,
            disabled: self.disabled,
        }
    }

    fn measure(&self, style: &ComputedStyle, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        let text = measurer.measure_text(self.text.as_str(), &style.text);
        let padding = style.layout.padding;
        Some(Size::new(
            (text.width + padding.left + padding.right).max(text.width + 16.0),
            (text.height + padding.top + padding.bottom).max(text.height.max(20.0) + 10.0),
        ))
    }

    fn on_hovered_change(&mut self, hovered: bool) -> DirtyFlags {
        self.hovered = hovered;
        DirtyFlags::STYLE | DirtyFlags::PAINT
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        paint_box(rect, style, commands);
        let mut text_props = TextProps::new(self.text.clone());
        apply_text_style(&mut text_props, style);
        let padding = style.layout.padding;
        commands.push(PaintCommand::Text(TextPaintCommand {
            rect: Rect::new(
                rect.x + padding.left,
                rect.y + padding.top,
                (rect.width - padding.left - padding.right).max(0.0),
                (rect.height - padding.top - padding.bottom).max(0.0),
            ),
            props: text_props,
        }));
    }

    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult {
        match event {
            Event::PointerDown {
                button: PointerButton::Primary,
                ..
            } => {
                if self.disabled {
                    return EventResult::Ignored;
                }
                self.pressed = true;
                cx.capture_pointer();
                cx.mark_dirty(DirtyFlags::STYLE | DirtyFlags::PAINT);
                EventResult::Consumed
            }
            Event::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                self.pressed = false;
                cx.release_pointer_capture();
                cx.mark_dirty(DirtyFlags::STYLE | DirtyFlags::PAINT);
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }

    fn text(&self) -> Option<xui_interface::TextContent> {
        Some(self.text.clone())
    }
}
