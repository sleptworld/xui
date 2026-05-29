use std::any::Any;

use taffy::prelude as tf;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand,
    Rect, Size, Style, TextContent, TextMeasurer, TextPaintCommand, TextProps, Widget, WidgetType,
};

use super::{LayoutStyledWidget, fixed_size_style, props_hash};

#[derive(Debug)]
pub struct LabelWidget {
    pub key: Option<Key>,
    pub text: TextContent,
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl LabelWidget {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            key: None,
            text: text.into(),
            style: Style::new(),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

impl LayoutStyledWidget for LabelWidget {
    fn layout_style(&self, computed: &ComputedStyle, measurer: &mut dyn TextMeasurer) -> tf::Style {
        self.measure(computed, measurer)
            .map(fixed_size_style)
            .unwrap_or_default()
    }
}

impl Widget for LabelWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Label
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(&self.text, &self.style))
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<LabelWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.text != next.text {
            self.text = next.text.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.style != next.style {
            self.style = next.style.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        flags
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn measure(&self, style: &ComputedStyle, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        Some(measurer.measure_text(self.text.as_str(), &style.text, None))
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        let mut text_props = TextProps::new(self.text.clone());
        apply_text_style(&mut text_props, style);
        commands.push(PaintCommand::Text(TextPaintCommand {
            rect,
            props: text_props,
        }));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }

    fn text(&self) -> Option<xui_interface::TextContent> {
        Some(self.text.clone())
    }
}

pub(crate) fn apply_text_style(text_props: &mut TextProps, style: &ComputedStyle) {
    text_props.style.color = style.text.color;
    text_props.style.font_family = style.text.font_family.clone();
    text_props.style.font_size = style.text.font_size;
    text_props.style.font_weight = style.text.font_weight;
    text_props.style.font_style = style.text.font_style;
    text_props.style.line_height = style.text.line_height;
    text_props.style.letter_spacing = style.text.letter_spacing;
    text_props.style.decoration = style.text.decoration;
}
