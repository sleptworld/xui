//! A run of text.

// The text backend's layout input; `TextProps` is this widget's own props.
use xui_interface::TextProps as LayoutProps;
use xui_interface::{
    Bounds, ComputedStyle, ParagraphStyle, TextBoxStyle, TextContent, TextPaintProps,
    TextPaintStyle,
};
use xui_macros::widget;

use super::props_hash;
use super::roles::{PaintContent, TextSource};
use crate::node::{Draft, WidgetInstance, WidgetT};
use crate::render::{Primitive, RenderTreeWriter, TextPrimitive};
use crate::text::TextLayoutSlot;

/// Text, laid out and painted by the runtime's text backend in the style it
/// computes — so it selects typography, and no decoration: a text paints its
/// glyphs, not a box.
#[widget(
    style(
        box_size, margin, padding, position, display, typography, visibility, transform, effects,
        cursor
    ),
    behavior(
        focus,
        accessibility,
        shortcuts,
        motion,
        focus_binding,
        size_observation,
        events
    )
)]
#[props(derive(Debug, Clone, PartialEq))]
pub struct Text {
    /// What it says.
    #[default]
    #[dirty(LAYOUT_INPUT | PAINT_OUTPUT)]
    pub content: TextContent,
    /// How lines break and align.
    #[default]
    #[dirty(LAYOUT_INPUT | PAINT_OUTPUT)]
    pub paragraph: ParagraphStyle,
    /// How the text fits its box: overflow, line limit.
    #[default]
    #[dirty(LAYOUT_INPUT | PAINT_OUTPUT)]
    pub text_box: TextBoxStyle,
}

impl WidgetT for Text {
    type State = ();

    fn create_state(_: &TextProps) {}
}

pub fn text() -> Draft<Text> {
    Draft::new(TextProps::builder().build())
}

pub(super) fn apply_text_style(text_props: &mut LayoutProps, style: &ComputedStyle) {
    text_props.style.color = style.text.color;
    text_props.style.font_family = style.text.font_family.clone();
    text_props.style.font_size = style.text.font_size;
    text_props.style.font_weight = style.text.font_weight;
    text_props.style.font_style = style.text.font_style;
    text_props.style.line_height = style.text.line_height;
    text_props.style.letter_spacing = style.text.letter_spacing;
    text_props.style.decoration = style.text.decoration;
}

impl PaintContent for WidgetInstance<Text> {
    fn paint(
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
                layout_revision: props_hash(&self.text_layout(style)),
                vertical_align: self.props.paragraph.vertical_align,
                paint: TextPaintProps::new(TextPaintStyle::from_computed(&style.text)),
            }))
            .expect("widget render tree must remain valid");
    }
}

impl TextSource for WidgetInstance<Text> {
    fn text(&self) -> TextContent {
        self.props.content.clone()
    }

    fn text_layout(&self, style: &ComputedStyle) -> LayoutProps {
        let mut props = LayoutProps::new(self.props.content.clone());
        props.paragraph = self.props.paragraph.clone();
        props.text_box = self.props.text_box.clone();
        apply_text_style(&mut props, style);
        props
    }
}
