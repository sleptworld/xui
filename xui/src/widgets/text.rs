use std::any::Any;

use taffy::prelude as tf;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, OverflowWrap,
    PaintCommand, ParagraphStyle, Rect, Size, Style, TextBoxStyle, TextContent, TextMeasurer,
    TextOverflow, TextPaintCommand, TextProps, Widget, WidgetType,
};

use super::{LayoutStyledWidget, computed_layout_style, label::apply_text_style, props_hash};

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
            style: Style::new(),
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

    event_handler_methods!();
}

impl LayoutStyledWidget for TextWidget {
    fn layout_style(
        &self,
        computed: &ComputedStyle,
        _measurer: &mut dyn TextMeasurer,
    ) -> tf::Style {
        computed_layout_style(computed)
    }
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
        props_hash(&(
            &self.props.text,
            &self.props.paragraph,
            &self.props.text_box,
            &self.style,
        ))
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
            || self.props.paragraph != next.props.paragraph
            || self.props.text_box != next.props.text_box
            || self.style != next.style
        {
            flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }

        self.props = next.props.clone();
        self.style = next.style.clone();
        flags
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn measure(&self, style: &ComputedStyle, measurer: &mut dyn TextMeasurer) -> Option<Size> {
        Some(measurer.measure_text(self.props.text.as_str(), &style.text))
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        let mut props = self.props.clone();
        apply_text_style(&mut props, style);
        commands.push(PaintCommand::Text(TextPaintCommand { rect, props }));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }

    fn text(&self) -> Option<TextContent> {
        Some(self.props.text.clone())
    }
}
