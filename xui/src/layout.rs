use taffy::{LengthPercentage, prelude as tf};
use taffy::{Overflow, Point as TaffyPoint};
pub use xui_interface::TextMeasurer;
use xui_interface::Widget;
use xui_interface::core::Sizing;
use xui_interface::style::{AlignStyle, JustifyStyle};

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
        match widget {
            Widgets::Row(_) => computed.layout.flex_direction = FlexDirectionStyle::Row,
            Widgets::Column(_) | Widgets::Root(_) => {
                computed.layout.flex_direction = FlexDirectionStyle::Column;
            }
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
        Widgets::Root(_) => tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Column,
            size: tf::Size {
                width: tf::Dimension::percent(1.0),
                height: tf::Dimension::percent(1.0),
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

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::prelude::*;
//     use xui_interface::TextLayoutConstraints;

//     struct ZeroTextMeasurer;

//     impl TextMeasurer for ZeroTextMeasurer {
//         fn measure_text(&mut self, _text: &str, _props: &ComputedTextStyle) -> Size<f32> {
//             Size::<f32>::ZERO
//         }

//         fn measure_text_with_constraints(
//             &mut self,
//             _text: &str,
//             _props: &ComputedTextStyle,
//             _constraints: TextLayoutConstraints,
//         ) -> Size<f32> {
//             Size::<f32>::ZERO
//         }
//     }

//     #[test]
//     fn fill_editor_shell_occupies_window() {
//         let mut app = App::new(|_| {
//             row()
//                 .style(Style::new().size(Size::fill()))
//                 .into_element_desc(vec![
//                     container()
//                         .style(
//                             Style::new()
//                                 .width(Sizing::fix(300.0))
//                                 .height(Sizing::Fill)
//                                 .background(Color::BLACK),
//                         )
//                         .into_element_desc(Vec::new()),
//                     container()
//                         .style(Style::new().size(Size::fill()).background(Color::BLUE_500))
//                         .into_element_desc(Vec::new()),
//                 ])
//         });
//         let mut backend = MockRenderBackend::default();
//         let mut measurer = ZeroTextMeasurer;

//         app.resize(Size::<f32>::new(800.0, 600.0));
//         app.render(&mut backend, &mut measurer).unwrap();

//         let root = app.arena().root();
//         let row_id = app.arena().children(root)[0];
//         let pane_ids = app.arena().children(row_id);
//         let row_layout = app.arena().node(row_id).unwrap().layout;
//         let left_layout = app.arena().node(pane_ids[0]).unwrap().layout;
//         let right_layout = app.arena().node(pane_ids[1]).unwrap().layout;

//         assert_eq!(row_layout, Rect::new(0.0, 0.0, 792.0, 592.0));
//         assert_eq!(left_layout, Rect::new(0.0, 0.0, 300.0, 592.0));
//         assert_eq!(right_layout, Rect::new(300.0, 0.0, 492.0, 592.0));

//         let painted_rects = backend
//             .last_commands
//             .iter()
//             .filter_map(|command| match command {
//                 PaintCommand::Rect { rect, color, .. } => Some((*rect, *color)),
//                 _ => None,
//             })
//             .collect::<Vec<_>>();
//         assert!(painted_rects.contains(&(
//             Rect::new(0.0, 0.0, 300.0, 592.0),
//             ComputedColorStyle::Solid(Color::BLACK),
//         )));
//         assert!(painted_rects.contains(&(
//             Rect::new(300.0, 0.0, 492.0, 592.0),
//             ComputedColorStyle::Solid(Color::BLUE_500),
//         )));
//     }
// }
