use crate::event_system::callbacks::EventHandlers;
use crate::event_system::interaction::InteractionProperties;
use crate::{element::ElementDesc, event_system::EventContext};
use xui_interface::{
    Bounds, ComputedStyle, EventRef, EventResult, Key, OverflowWrap, ParagraphStyle,
    Style, TextBoxStyle, TextContent, TextOverflow, TextPaintProps, TextPaintStyle, TextProps,
    WidgetType, WidgetUpdateFlags,
};

use super::{props_hash, widget_element_desc};
use crate::render::{Primitive, RenderTreeWriter, TextPrimitive};
use crate::text::TextLayoutSlot;

#[derive(Debug)]
pub struct TextWidget {
    pub key: Option<Key>,
    pub props: TextProps,
    pub style: Style,
    pub event_handlers: EventHandlers,
    pub interaction: InteractionProperties,
}

impl TextWidget {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            key: None,
            props: TextProps::new(text),
            style: Style::default(),
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
        }
    }

    pub fn props(mut self, props: TextProps) -> Self {
        self.props = props;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
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

    pub fn set_key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self) -> ElementDesc {
        widget_element_desc(self, Vec::new())
    }

    event_handler_methods!();
}

impl TextWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Text
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.props.text,
            &self.props.paragraph,
            &self.props.text_box,
            &self.style,
        ))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
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

    pub(super) fn default_style(&self) -> Style {
        Style::new()
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
        writer
            .primitive(Primitive::Text(TextPrimitive {
                node_id,
                bounds: rect,
                slot: TextLayoutSlot::PRIMARY,
                layout_revision: self.props_hash(),
                vertical_align: self.props.paragraph.vertical_align,
                paint: TextPaintProps::new(TextPaintStyle::from_computed(&style.text)),
            }))
            .expect("widget render tree must remain valid");
    }

    pub(super) fn handle_event(
        &mut self,
        _event: EventRef,
        _cx: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }

    pub(super) fn text_content(&self) -> Option<TextContent> {
        Some(self.props.text.clone())
    }

    pub(super) fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
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

/// Some Utils
impl<T> From<T> for ElementDesc
where
    T: Into<TextContent>,
{
    fn from(value: T) -> Self {
        TextWidget::new(value).into_element_desc()
    }
}
