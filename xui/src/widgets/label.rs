use crate::animation::AnimatedStyle;
use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;

use super::{props_hash, widget_element_desc};
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventResult, Key, PaintCommand, Rect, Style,
    TextContent, TextPaintCommand, TextProps, Widget, WidgetType,
};

#[derive(Debug)]
pub struct LabelWidget {
    pub key: Option<Key>,
    pub text: TextContent,
    pub animated_style: AnimatedStyle,
    pub event_handlers: EventHandlers,
}

impl LabelWidget {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            key: None,
            text: text.into(),
            animated_style: AnimatedStyle::new(Style::new()),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.animated_style.base = style;
        self
    }

    animated_style_methods!(animated_style);

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self) -> ElementDesc {
        widget_element_desc(self, Vec::new())
    }

    event_handler_methods!();
}

impl Widget for LabelWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Label
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(&self.text, &self.animated_style))
    }

    fn update_from(&mut self, next: &Self) -> DirtyFlags {
        let mut flags = DirtyFlags::empty();
        if self.text != next.text {
            self.text = next.text.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.animated_style != next.animated_style {
            self.animated_style = next.animated_style.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        flags
    }

    fn style(&self) -> &Style {
        &self.animated_style.base
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        let mut text_props = TextProps::new(self.text.clone());
        apply_text_style(&mut text_props, style);
        commands.push(PaintCommand::Text(TextPaintCommand {
            node_id: Default::default(),
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
