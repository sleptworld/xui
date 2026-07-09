use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;
use xui_interface::{
    ComputedStyle, EventContext, EventRef, EventResult, Key, OverflowWrap, PaintCommand,
    ParagraphStyle, Rect, Style, TextBoxStyle, TextContent, TextOverflow, TextPaintCommand,
    TextPaintProps, TextPaintStyle, TextProps, Widget, WidgetType, WidgetUpdateFlags,
};

use super::{props_hash, widget_element_desc};

#[derive(Debug)]
pub struct TextWidget {
    pub key: Option<Key>,
    pub props: TextProps,
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl TextWidget {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            key: None,
            props: TextProps::new(text),
            style: Style::default(),
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
        self.style = style;
        self
    }

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
            &self.style,
        ))
    }

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        if self.props.text != next.props.text
            || self.props.paragraph != next.props.paragraph
            || self.props.text_box != next.props.text_box
        {
            flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
        }
        if self.style != next.style {
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }
        self.props = next.props.clone();
        self.style = next.style.clone();
        flags
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Text(TextPaintCommand {
            node_id: Default::default(),
            rect,
            paint: TextPaintProps::new(TextPaintStyle::from_computed(&style.text)),
        }));
    }

    fn handle_event(&mut self, _event: EventRef, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }

    fn text(&self) -> Option<TextContent> {
        Some(self.props.text.clone())
    }

    fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
        let mut props = self.props.clone();
        apply_text_style(&mut props, style);
        Some(props)
    }
}

pub(super) fn apply_text_style(text_props: &mut TextProps, style: &ComputedStyle) {
    text_props.style.color = style.text.color;
    text_props.style.font_family = style.text.font_family.clone();
    text_props.style.font_size = style.text.font_size;
    text_props.style.font_weight = style.text.font_weight;
    text_props.style.font_style = style.text.font_style;
    text_props.style.line_height = style.text.line_height;
    text_props.style.letter_spacing = style.text.letter_spacing;
    text_props.style.decoration = style.text.decoration;
}
