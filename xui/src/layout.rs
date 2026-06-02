use taffy::prelude as tf;
use taffy::{Overflow, Point as TaffyPoint};
pub use xui_interface::TextMeasurer;
use xui_interface::Widget;
use xui_interface::core::Sizing;

use crate::core::EdgeInsets;
use crate::style::{ComputedStyle, FlexDirectionStyle, ScrollDirectionStyle, Theme};
use crate::widgets::WidgetI;

pub fn computed_style_for_widget(
    widget: &WidgetI,
    parent: &ComputedStyle,
    theme: &Theme,
) -> ComputedStyle {
    widget.with_widgets(|widget| {
        let mut computed = parent.inherited_from(theme);
        if let Some(scope) = widget.style_scope() {
            computed.apply(parent, scope, theme);
        }
        computed.apply(parent, &widget.default_style(), theme);
        computed.apply(parent, widget.style(), theme);
        computed.apply(parent, &widget.state_style(widget.state()), theme);
        computed
    })
}

pub fn taffy_style_for_widget(parent: &ComputedStyle, computed: &ComputedStyle) -> tf::Style {
    computed_layout_style(parent.layout.flex_direction, computed)
}

pub fn computed_layout_style(
    parent_dire: FlexDirectionStyle,
    computed: &ComputedStyle,
) -> tf::Style {
    let layout = computed.layout;
    let mut style = tf::Style {
        display: tf::Display::Flex,
        flex_direction: match layout.flex_direction {
            FlexDirectionStyle::Row => tf::FlexDirection::Row,
            FlexDirectionStyle::Column => tf::FlexDirection::Column,
        },
        gap: tf::Size {
            width: length_percentage(layout.gap),
            height: length_percentage(layout.gap),
        },
        margin: edge_insets_auto(layout.margin),
        padding: edge_insets(layout.padding),
        overflow: scroll_overflow(computed.scroll.direction),
        scrollbar_width: computed.scroll.scrollbar.width.max(0.0),
        ..Default::default()
    };

    if let Some(size) = layout.size {
        style.size = tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        };

        match parent_dire {
            FlexDirectionStyle::Column => {
                if matches!(size.height(), Sizing::Fill) {
                    style.flex_grow = 1.0;
                }
                if matches!(size.height(), Sizing::Fix(_)) {
                    style.flex_shrink = 0.0;
                }
            }
            FlexDirectionStyle::Row => {
                if matches!(size.width(), Sizing::Fill) {
                    style.flex_grow = 1.0;
                }
                if matches!(size.width(), Sizing::Fix(_)) {
                    style.flex_shrink = 0.0;
                }
            }
        }
    }
    if let Some(size) = layout.min_size {
        style.min_size = tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        };
    }
    if let Some(size) = layout.max_size {
        style.max_size = tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        };
    }

    style
}

fn edge_insets(value: EdgeInsets) -> tf::Rect<tf::LengthPercentage> {
    tf::Rect {
        left: length_percentage(value.left),
        right: length_percentage(value.right),
        top: length_percentage(value.top),
        bottom: length_percentage(value.bottom),
    }
}

fn edge_insets_auto(value: EdgeInsets) -> tf::Rect<tf::LengthPercentageAuto> {
    tf::Rect {
        left: tf::LengthPercentageAuto::length(value.left),
        right: tf::LengthPercentageAuto::length(value.right),
        top: tf::LengthPercentageAuto::length(value.top),
        bottom: tf::LengthPercentageAuto::length(value.bottom),
    }
}

fn dimension(value: Sizing) -> tf::Dimension {
    match value {
        Sizing::Fill | Sizing::Hug => tf::Dimension::auto(),
        Sizing::Fix(v) => tf::Dimension::length(v.into_inner()),
        Sizing::Percent(v) => tf::Dimension::percent(v.into_inner()),
    }
}

// fn dimension(value: f32) -> tf::Dimension {
//     tf::Dimension::length(value)
// }

fn length_percentage(value: f32) -> tf::LengthPercentage {
    tf::LengthPercentage::length(value)
}

fn scroll_overflow(direction: ScrollDirectionStyle) -> TaffyPoint<Overflow> {
    let x = if direction.allows_horizontal() {
        Overflow::Scroll
    } else if direction.is_scrollable() {
        Overflow::Hidden
    } else {
        Overflow::Visible
    };
    let y = if direction.allows_vertical() {
        Overflow::Scroll
    } else if direction.is_scrollable() {
        Overflow::Hidden
    } else {
        Overflow::Visible
    };
    TaffyPoint { x, y }
}
