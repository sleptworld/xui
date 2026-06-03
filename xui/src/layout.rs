use taffy::{LengthPercentage, prelude as tf};
use taffy::{Overflow, Point as TaffyPoint};
pub use xui_interface::TextMeasurer;
use xui_interface::core::Sizing;
use xui_interface::style::{AlignStyle, JustifyStyle};
use xui_interface::{Size, Widget};

use crate::core::EdgeInsets;
use crate::style::{ComputedStyle, FlexDirectionStyle, ScrollDirectionStyle, Theme};
use crate::widgets::{WidgetI, Widgets};

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

#[inline(always)]
pub fn taffy_style_for_widget(
    widget: &WidgetI,
    parent: &ComputedStyle,
    computed: &ComputedStyle,
) -> tf::Style {
    computed_layout_style(widget, parent.layout.flex_direction, computed)
}

pub fn computed_layout_style(
    widget: &WidgetI,
    parent_dire: FlexDirectionStyle,
    computed: &ComputedStyle,
) -> tf::Style {
    let layout = computed.layout;
    let mut style = widget.with_widgets(|w| match w {
        Widgets::Column(_) => tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Column,
            align_items: Some(align_items(layout.align)),
            justify_content: Some(justify_content(layout.justify)),
            gap: tf::Size {
                height: length_percentage(layout.gap),
                width: LengthPercentage::length(0.0),
            },
            ..Default::default()
        },
        Widgets::Row(_) => tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Row,
            align_items: Some(align_items(layout.align)),
            justify_content: Some(justify_content(layout.justify)),
            gap: tf::Size {
                height: LengthPercentage::length(0.0),
                width: length_percentage(layout.gap),
            },
            ..Default::default()
        },
        _ => tf::Style {
            display: tf::Display::Block,
            ..Default::default()
        },
    });

    style.margin = edge_insets_auto(layout.margin);
    style.padding = edge_insets(layout.padding);
    style.overflow = scroll_overflow(computed.scroll.direction);
    style.scrollbar_width = computed.scroll.scrollbar.width.max(0.0);

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
    } else {
        style.size = tf::Size {
            width: tf::Dimension::auto(),
            height: tf::Dimension::auto(),
        };
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

#[inline]
fn align_items(value: AlignStyle) -> tf::AlignItems {
    match value {
        AlignStyle::Start => tf::AlignItems::Start,
        AlignStyle::Center => tf::AlignItems::Center,
        AlignStyle::End => tf::AlignItems::End,
        AlignStyle::Stretch => tf::AlignItems::Stretch,
    }
}

#[inline]
fn justify_content(value: JustifyStyle) -> tf::JustifyContent {
    match value {
        JustifyStyle::Start => tf::JustifyContent::Start,
        JustifyStyle::Center => tf::JustifyContent::Center,
        JustifyStyle::End => tf::JustifyContent::End,
        JustifyStyle::SpaceBetween => tf::JustifyContent::SpaceBetween,
        JustifyStyle::SpaceAround => tf::JustifyContent::SpaceAround,
        JustifyStyle::SpaceEvenly => tf::JustifyContent::SpaceEvenly,
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
