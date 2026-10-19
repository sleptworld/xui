use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;
use crate::event_system::interaction::InteractionProperties;
use xui_interface::{
    Bounds, ComputedStyle, Key, OverflowWrap, ParagraphStyle, Style, TextBoxStyle, TextContent,
    TextOverflow, TextPaintProps, TextPaintStyle, TextProps, WidgetType, WidgetUpdateFlags,
};

use super::{HostWidget, props_hash, sync, widget_element_desc};
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

    /// Merges a style in, rather than replacing what is already there.
    ///
    /// On a fresh builder — how nearly every call site uses it — merging into an
    /// all-unset style is indistinguishable from assignment. The difference
    /// shows in the `xui!` macro, where `style={..}` is one attribute among
    /// many: assignment made `<column padding={..} style={..} />` silently
    /// discard the padding, and whether an attribute survived depended on
    /// whether it was written before or after `style`.
    pub fn style(mut self, style: impl xui_interface::StyleMerge) -> Self {
        self.style.merge(&style);
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

impl HostWidget for TextWidget {
    const TYPE: WidgetType = WidgetType::Text;

    fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn current_style(&self) -> &Style {
        &self.style
    }

    fn style_mut(&mut self) -> Option<&mut Style> {
        Some(&mut self.style)
    }

    fn set_key(&mut self, key: Option<Key>) {
        self.key = key;
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.props.text,
            &self.props.paragraph,
            &self.props.text_box,
            &self.style,
        ))
    }

    fn instantiate(&self) -> Self {
        Self {
            key: self.key.clone(),
            props: self.props.clone(),
            style: self.style.clone(),
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
        }
    }

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let shape = WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
        // `text_layout_props` overwrites every field of `props.style` from the
        // computed style, so a change there has nothing of its own to redo.
        self.props.style.clone_from(&next.props.style);
        sync(&mut self.props.text, &next.props.text, shape)
            | sync(&mut self.props.paragraph, &next.props.paragraph, shape)
            | sync(&mut self.props.text_box, &next.props.text_box, shape)
            | sync(
                &mut self.style,
                &next.style,
                WidgetUpdateFlags::STYLE_TARGET,
            )
    }

    fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        // The paragraph was measured and wrapped inside the content box, so it
        // is painted there too; backends place glyphs at the bounds' origin.
        let padding = style.layout.padding;
        let content = Bounds::from_origin_size(
            xui_interface::Point::new(rect.x() + padding.left(), rect.y() + padding.top()),
            xui_interface::Size::new(
                (rect.width() - padding.left() - padding.right()).max(0.0),
                (rect.height() - padding.top() - padding.bottom()).max(0.0),
            ),
        );
        writer
            .primitive(Primitive::Text(TextPrimitive {
                node_id,
                bounds: content,
                slot: TextLayoutSlot::PRIMARY,
                layout_revision: self.props_hash(),
                vertical_align: self.props.paragraph.vertical_align,
                paint: TextPaintProps::new(TextPaintStyle::from_computed(&style.text)),
            }))
            .expect("widget render tree must remain valid");
    }

    fn text_content(&self) -> Option<TextContent> {
        Some(self.props.text.clone())
    }

    fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
        let mut props = self.props.clone();
        apply_text_style(&mut props, style);
        Some(props)
    }

    fn interaction(&self) -> Option<(&InteractionProperties, &EventHandlers)> {
        Some((&self.interaction, &self.event_handlers))
    }

    fn interaction_mut(&mut self) -> Option<(&mut InteractionProperties, &mut EventHandlers)> {
        Some((&mut self.interaction, &mut self.event_handlers))
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
