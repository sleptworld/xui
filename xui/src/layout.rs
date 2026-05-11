use taffy::prelude as tf;
pub use xui_interface::TextMeasurer;

use crate::core::{EdgeInsets, Size};
use crate::widgets::{Button, Column, Container, Element, Label, Row};

#[derive(Debug, Clone, Copy)]
pub struct MockTextMeasurer {
    pub average_glyph_width: f32,
    pub line_height: f32,
}

impl Default for MockTextMeasurer {
    fn default() -> Self {
        Self {
            average_glyph_width: 0.58,
            line_height: 1.25,
        }
    }
}

impl TextMeasurer for MockTextMeasurer {
    fn measure(&self, text: &str, font_size: f32) -> Size {
        Size::new(
            text.chars().count() as f32 * font_size * self.average_glyph_width,
            font_size * self.line_height,
        )
    }
}

pub fn style_for_element(element: &Element, measurer: &dyn TextMeasurer) -> tf::Style {
    match element {
        Element::Label(label) => label_style(label, measurer),
        Element::Button(button) => button_style(button, measurer),
        Element::Column(column) => column_style(column),
        Element::Row(row) => row_style(row),
        Element::Container(container) => container_style(container),
    }
}

fn label_style(label: &Label, measurer: &dyn TextMeasurer) -> tf::Style {
    let measured = measurer.measure(&label.text, label.font_size);
    fixed_size_style(measured)
}

fn button_style(button: &Button, measurer: &dyn TextMeasurer) -> tf::Style {
    let text = measurer.measure(&button.text, 14.0);
    fixed_size_style(Size::new(text.width + 16.0, text.height.max(20.0) + 10.0))
}

fn column_style(column: &Column) -> tf::Style {
    tf::Style {
        display: tf::Display::Flex,
        flex_direction: tf::FlexDirection::Column,
        gap: tf::Size {
            width: length_percentage(column.gap),
            height: length_percentage(column.gap),
        },
        ..Default::default()
    }
}

fn row_style(row: &Row) -> tf::Style {
    tf::Style {
        display: tf::Display::Flex,
        flex_direction: tf::FlexDirection::Row,
        gap: tf::Size {
            width: length_percentage(row.gap),
            height: length_percentage(row.gap),
        },
        ..Default::default()
    }
}

fn container_style(container: &Container) -> tf::Style {
    let mut style = tf::Style {
        display: tf::Display::Flex,
        flex_direction: tf::FlexDirection::Column,
        padding: edge_insets(container.padding),
        ..Default::default()
    };

    if let Some(size) = container.size {
        style.size = tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        };
    }

    style
}

fn fixed_size_style(size: Size) -> tf::Style {
    tf::Style {
        size: tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        },
        ..Default::default()
    }
}

fn edge_insets(value: EdgeInsets) -> tf::Rect<tf::LengthPercentage> {
    tf::Rect {
        left: length_percentage(value.left),
        right: length_percentage(value.right),
        top: length_percentage(value.top),
        bottom: length_percentage(value.bottom),
    }
}

fn dimension(value: f32) -> tf::Dimension {
    tf::Dimension::length(value)
}

fn length_percentage(value: f32) -> tf::LengthPercentage {
    tf::LengthPercentage::length(value)
}
