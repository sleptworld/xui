use crate::animation::AnimatedStyle;
use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventResult, Key, OverflowWrap, PaintCommand,
    ParagraphStyle, Rect, Style, TextBoxStyle, TextContent, TextOverflow, TextPaintCommand,
    TextProps, Widget, WidgetType,
};

use super::{label::apply_text_style, props_hash, widget_element_desc};

#[derive(Debug)]
pub struct TextWidget {
    pub key: Option<Key>,
    pub props: TextProps,
    pub animated_style: AnimatedStyle,
    pub event_handlers: EventHandlers,
}

impl TextWidget {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            key: None,
            props: TextProps::new(text),
            animated_style: AnimatedStyle::new(Style::new()),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn props(mut self, props: TextProps) -> Self {
        self.props = props;
        self
    }

    pub fn text(mut self, text: impl Into<TextContent>) -> Self {
        self.props.text = text.into();
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.animated_style.base = style;
        self
    }

    animated_style_methods!(animated_style);

    pub fn paragraph(mut self, paragraph: ParagraphStyle) -> Self {
        self.props.paragraph = paragraph;
        self
    }

    pub fn text_box(mut self, text_box: TextBoxStyle) -> Self {
        self.props.text_box = text_box;
        self
    }

    pub fn overflow_wrap(mut self, overflow_wrap: OverflowWrap) -> Self {
        self.props.paragraph.overflow_wrap = overflow_wrap;
        self
    }

    pub fn overflow(mut self, overflow: TextOverflow) -> Self {
        self.props.text_box.overflow = overflow;
        self
    }

    pub fn max_lines(mut self, max_lines: impl Into<Option<usize>>) -> Self {
        self.props.text_box.max_lines = max_lines.into();
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
}

impl Widget for TextWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Text
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.props.text,
            &self.props.paragraph,
            &self.props.text_box,
            &self.animated_style,
        ))
    }

    fn update_from(&mut self, next: &Self) -> DirtyFlags {
        let mut flags = DirtyFlags::empty();
        if self.props.text != next.props.text
            || self.props.paragraph != next.props.paragraph
            || self.props.text_box != next.props.text_box
            || self.animated_style != next.animated_style
        {
            flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }

        self.props = next.props.clone();
        self.animated_style = next.animated_style.clone();
        flags
    }

    fn style(&self) -> &Style {
        &self.animated_style.base
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        let mut props = self.props.clone();
        apply_text_style(&mut props, style);
        commands.push(PaintCommand::Text(TextPaintCommand {
            node_id: Default::default(),
            rect,
            props,
        }));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }

    fn text(&self) -> Option<TextContent> {
        Some(self.props.text.clone())
    }
}
