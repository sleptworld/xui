use taffy::{prelude as tf, LengthPercentage};
use taffy::{Overflow, Point as TaffyPoint};
use xui_interface::core::Sizing;
use xui_interface::style::{AlignStyle, JustifyStyle};
use xui_interface::{Widget, WidgetState};

use crate::core::EdgeInsets;
use crate::style::{ComputedStyle, FlexDirectionStyle, ScrollDirectionStyle, Theme};
use crate::widgets::{WidgetI, Widgets};

pub(crate) fn computed_style_for_widget(
    widget: &WidgetI,
    parent: &ComputedStyle,
    theme: &Theme,
    state: WidgetState,
) -> ComputedStyle {
    widget.with_widgets(|widget| {
        let mut computed = parent.inherited_from(theme);
        if let Some(scope) = widget.style_scope() {
            computed.apply(parent, scope, theme);
        }
        let default_style = widget.default_style().patch_for_state(WidgetState::empty());
        let style = widget.style().patch_for_state(state);
        computed.apply(parent, &default_style, theme);
        computed.apply(parent, &style, theme);
        match widget {
            Widgets::Container(widget) => {
                if let Some(direction) = widget.flex_direction {
                    computed.layout.flex_direction = direction;
                }
            }
            Widgets::Root(_) => computed.layout.flex_direction = FlexDirectionStyle::Column,
            _ => {}
        }
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
    // let is_root = widget.with_widgets(|w| matches!(w, Widgets::Root(_)));
    let mut style = widget.with_widgets(|w| match w {
        Widgets::Container(widget) => match widget.flex_direction {
            Some(FlexDirectionStyle::Column) => flex_style(FlexDirectionStyle::Column, layout),
            Some(FlexDirectionStyle::Row) => flex_style(FlexDirectionStyle::Row, layout),
            None => tf::Style {
                display: tf::Display::Block,
                ..Default::default()
            },
        },
        Widgets::Root(_) => tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Column,
            size: tf::Size {
                width: tf::Dimension::percent(1.0),
                height: tf::Dimension::percent(1.0),
            },
            ..Default::default()
        },

        Widgets::Image(image) => {
            let aspect_ratio = image
                .image_data
                .as_ref()
                .map(|data| data.size.width as f32 / data.size.height as f32);
            tf::Style {
                display: tf::Display::Block,
                aspect_ratio,
                ..Default::default()
            }
        }
        _ => tf::Style {
            display: tf::Display::Block,
            ..Default::default()
        },
    });

    style.margin = edge_insets_auto(layout.margin);
    style.padding = edge_insets(layout.padding);
    style.overflow = scroll_overflow(computed.scroll.direction);
    style.scrollbar_width = computed.scroll.scrollbar.width.max(0.0);

    let size = layout.size();
    style.size = tf::Size {
        width: dimension_for_axis(size.width, Axis::Horizontal, parent_dire),
        height: dimension_for_axis(size.height, Axis::Vertical, parent_dire),
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

    let min_size = layout.min_size();
    style.min_size = tf::Size {
        width: min_size
            .width()
            .map(|w| dimension_for_axis(w, Axis::Horizontal, parent_dire))
            .unwrap_or(taffy::Dimension::auto()),
        height: min_size
            .height()
            .map(|h| dimension_for_axis(h, Axis::Vertical, parent_dire))
            .unwrap_or(taffy::Dimension::auto()),
    };

    let max_size = layout.max_size();

    style.max_size = tf::Size {
        width: max_size
            .width()
            .map(|w| dimension_for_axis(w, Axis::Horizontal, parent_dire))
            .unwrap_or(taffy::Dimension::auto()),
        height: max_size
            .height()
            .map(|h| dimension_for_axis(h, Axis::Vertical, parent_dire))
            .unwrap_or(taffy::Dimension::auto()),
    };

    style
}

fn flex_style(
    direction: FlexDirectionStyle,
    layout: xui_interface::ComputedLayoutStyle,
) -> tf::Style {
    let (flex_direction, gap) = match direction {
        FlexDirectionStyle::Column => (
            tf::FlexDirection::Column,
            tf::Size {
                height: length_percentage(layout.gap),
                width: LengthPercentage::length(0.0),
            },
        ),
        FlexDirectionStyle::Row => (
            tf::FlexDirection::Row,
            tf::Size {
                height: LengthPercentage::length(0.0),
                width: length_percentage(layout.gap),
            },
        ),
    };

    tf::Style {
        display: tf::Display::Flex,
        flex_direction,
        align_items: Some(align_items(layout.align)),
        justify_content: Some(justify_content(layout.justify)),
        gap,
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

fn is_main_axis(axis: Axis, parent_dire: FlexDirectionStyle) -> bool {
    matches!(
        (axis, parent_dire),
        (Axis::Horizontal, FlexDirectionStyle::Row) | (Axis::Vertical, FlexDirectionStyle::Column)
    )
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
        left: length_percentage(value.left()),
        right: length_percentage(value.right()),
        top: length_percentage(value.top()),
        bottom: length_percentage(value.bottom()),
    }
}

fn edge_insets_auto(value: EdgeInsets) -> tf::Rect<tf::LengthPercentageAuto> {
    tf::Rect {
        left: tf::LengthPercentageAuto::length(value.left()),
        right: tf::LengthPercentageAuto::length(value.right()),
        top: tf::LengthPercentageAuto::length(value.top()),
        bottom: tf::LengthPercentageAuto::length(value.bottom()),
    }
}

fn dimension_for_axis(value: Sizing, axis: Axis, parent_dire: FlexDirectionStyle) -> tf::Dimension {
    match value {
        Sizing::Fill if !is_main_axis(axis, parent_dire) => tf::Dimension::percent(1.0),
        Sizing::Fill | Sizing::Hug => tf::Dimension::auto(),
        Sizing::Fix(v) => tf::Dimension::length(v.into_inner()),
        Sizing::Percent(v) => tf::Dimension::percent(v.into_inner()),
    }
}

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
