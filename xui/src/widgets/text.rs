use std::any::Any;

use xui_interface::{
    DirtyFlags, Event, EventContext, EventHandlers, EventResult, FontFamily, FontStyle, FontWeight,
    Key, LineHeight, OverflowWrap, PaintCommand, ParagraphStyle, Rect, Size, TextBoxStyle,
    TextContent, TextMeasurer, TextOverflow, TextPaintCommand, TextProps, TextStyle, Widget,
    WidgetType,
};

use super::props_hash;

#[derive(Debug)]
pub struct TextWidget {
    pub key: Option<Key>,
    pub props: TextProps,
    pub event_handlers: EventHandlers,
}

impl TextWidget {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            key: None,
            props: TextProps::new(text),
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

    pub fn style(mut self, style: TextStyle) -> Self {
        self.props.style = style;
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

    pub fn color(mut self, color: crate::core::Color) -> Self {
        self.props.style.color = color;
        self
    }

    pub fn font_family(mut self, font_family: FontFamily) -> Self {
        self.props.style.font_family = font_family;
        self
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.props.style.font_size = font_size;
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.props.style.font_weight = font_weight;
        self
    }

    pub fn font_style(mut self, font_style: FontStyle) -> Self {
        self.props.style.font_style = font_style;
        self
    }

    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.props.style.line_height = line_height;
        self
    }

    pub fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.props.style.letter_spacing = letter_spacing;
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

    event_handler_methods!();
}

impl Widget for TextWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Text
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.props)
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<TextWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.props.text != next.props.text
            || text_layout_style(&self.props) != text_layout_style(&next.props)
        {
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        } else if self.props.style.color != next.props.style.color
            || self.props.style.decoration != next.props.style.decoration
        {
            flags |= DirtyFlags::PAINT;
        }

        self.props = next.props.clone();
        flags
    }

    fn measure(&self, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        Some(measurer.measure_text(&self.props))
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Text(TextPaintCommand {
            rect,
            props: self.props.clone(),
        }));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

fn text_layout_style(props: &TextProps) -> TextLayoutStyle<'_> {
    TextLayoutStyle {
        font_family: &props.style.font_family,
        font_size: props.style.font_size,
        font_weight: props.style.font_weight,
        font_style: props.style.font_style,
        line_height: props.style.line_height,
        letter_spacing: props.style.letter_spacing,
        paragraph: &props.paragraph,
        text_box: &props.text_box,
    }
}

#[derive(PartialEq)]
struct TextLayoutStyle<'a> {
    font_family: &'a FontFamily,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    line_height: LineHeight,
    letter_spacing: f32,
    paragraph: &'a ParagraphStyle,
    text_box: &'a TextBoxStyle,
}
