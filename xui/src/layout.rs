use taffy::style_helpers::TaffyZero;
use taffy::{LengthPercentage, prelude as tf};
use taffy::{Overflow, Point as TaffyPoint};
use xui_interface::core::Sizing;
use xui_interface::style::{AlignStyle, JustifyStyle};
use xui_interface::{PositionStyle, WidgetState};

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
    parent_is_z_stack: bool,
) -> tf::Style {
    computed_layout_style_for_parent(
        widget,
        parent.layout.flex_direction,
        computed,
        parent_is_z_stack,
    )
}

pub fn computed_layout_style(
    widget: &WidgetI,
    parent_dire: FlexDirectionStyle,
    computed: &ComputedStyle,
) -> tf::Style {
    computed_layout_style_for_parent(widget, parent_dire, computed, false)
}

fn computed_layout_style_for_parent(
    widget: &WidgetI,
    parent_dire: FlexDirectionStyle,
    computed: &ComputedStyle,
    parent_is_z_stack: bool,
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
        Widgets::ZStack(stack) => tf::Style {
            display: tf::Display::Grid,
            justify_items: Some(stack_alignment(stack.alignment.x)),
            align_items: Some(stack_alignment(stack.alignment.y)),
            ..Default::default()
        },
        Widgets::Grid(grid) => {
            let mut style = tf::Style::default();
            crate::widgets::grid_widget_to_taffy(&mut style, grid);
            style.gap = tf::Size {
                width: length_percentage(layout.gap),
                height: length_percentage(layout.gap),
            };
            style
        }

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
    style.position = match layout.position {
        PositionStyle::Relative => tf::Position::Relative,
        PositionStyle::Absolute => tf::Position::Absolute,
    };
    style.inset = optional_edge_insets(layout.inset);
    if parent_is_z_stack {
        style.grid_row = tf::line(1);
        style.grid_column = tf::line(1);
    }

    let size = layout.size();
    style.size = tf::Size {
        width: dimension_for_axis(size.width, Axis::Horizontal, parent_dire),
        height: dimension_for_axis(size.height, Axis::Vertical, parent_dire),
    };

    match parent_dire {
        FlexDirectionStyle::Column => {
            if matches!(size.height(), Sizing::Fill) {
                style.flex_grow = 1.0;
                style.flex_shrink = 1.0;
                style.flex_basis = tf::Dimension::length(0.0);
            }
            if matches!(size.height(), Sizing::Fix(_)) {
                style.flex_shrink = 0.0;
            }
        }
        FlexDirectionStyle::Row => {
            if matches!(size.width(), Sizing::Fill) {
                style.flex_grow = 1.0;
                style.flex_shrink = 1.0;
                style.flex_basis = tf::Dimension::length(0.0);
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
            .unwrap_or(taffy::Dimension::ZERO),
        height: min_size
            .height()
            .map(|h| dimension_for_axis(h, Axis::Vertical, parent_dire))
            .unwrap_or(taffy::Dimension::ZERO),
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

fn stack_alignment(value: f32) -> tf::AlignItems {
    if value <= 0.25 {
        tf::AlignItems::START
    } else if value >= 0.75 {
        tf::AlignItems::END
    } else {
        tf::AlignItems::CENTER
    }
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
        AlignStyle::Start => tf::AlignItems::START,
        AlignStyle::Center => tf::AlignItems::CENTER,
        AlignStyle::End => tf::AlignItems::END,
        AlignStyle::Stretch => tf::AlignItems::STRETCH,
    }
}

#[inline]
fn justify_content(value: JustifyStyle) -> tf::JustifyContent {
    match value {
        JustifyStyle::Start => tf::JustifyContent::START,
        JustifyStyle::Center => tf::JustifyContent::CENTER,
        JustifyStyle::End => tf::JustifyContent::END,
        JustifyStyle::SpaceBetween => tf::JustifyContent::SPACE_BETWEEN,
        JustifyStyle::SpaceAround => tf::JustifyContent::SPACE_AROUND,
        JustifyStyle::SpaceEvenly => tf::JustifyContent::SPACE_EVENLY,
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

fn optional_edge_insets(value: Option<EdgeInsets>) -> tf::Rect<tf::LengthPercentageAuto> {
    value.map_or(
        tf::Rect {
            left: tf::LengthPercentageAuto::auto(),
            right: tf::LengthPercentageAuto::auto(),
            top: tf::LengthPercentageAuto::auto(),
            bottom: tf::LengthPercentageAuto::auto(),
        },
        edge_insets_auto,
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{
        GridFlow, GridTrackSize, GridTracks, GridWidget, WidgetI, ZStackWidget, container,
    };

    fn fixed_grid_child(width: f32, height: f32) -> tf::Style {
        tf::Style {
            size: tf::Size {
                width: tf::Dimension::length(width),
                height: tf::Dimension::length(height),
            },
            grid_row: tf::line(1),
            grid_column: tf::line(1),
            ..Default::default()
        }
    }

    #[test]
    fn z_stack_grid_uses_largest_child_and_centers_all_children() {
        let mut taffy = tf::TaffyTree::<()>::new();
        let small = taffy.new_leaf(fixed_grid_child(20.0, 10.0)).unwrap();
        let large = taffy.new_leaf(fixed_grid_child(40.0, 30.0)).unwrap();
        let stack = taffy
            .new_with_children(
                tf::Style {
                    display: tf::Display::Grid,
                    align_items: Some(tf::AlignItems::CENTER),
                    justify_items: Some(tf::AlignItems::CENTER),
                    ..Default::default()
                },
                &[small, large],
            )
            .unwrap();
        taffy
            .compute_layout(
                stack,
                tf::Size {
                    width: tf::AvailableSpace::MaxContent,
                    height: tf::AvailableSpace::MaxContent,
                },
            )
            .unwrap();

        assert_eq!(taffy.layout(stack).unwrap().size.width, 40.0);
        assert_eq!(taffy.layout(stack).unwrap().size.height, 30.0);
        assert_eq!(taffy.layout(small).unwrap().location.x, 10.0);
        assert_eq!(taffy.layout(small).unwrap().location.y, 10.0);
        assert_eq!(taffy.layout(large).unwrap().location.x, 0.0);
        assert_eq!(taffy.layout(large).unwrap().location.y, 0.0);
    }

    #[test]
    fn z_stack_host_and_its_children_map_to_one_grid_cell() {
        let theme = Theme::default();
        let parent = ComputedStyle::initial(&theme);
        let stack = WidgetI::new(ZStackWidget::new().alignment(xui_interface::Alignment::END));
        let stack_computed =
            computed_style_for_widget(&stack, &parent, &theme, xui_interface::WidgetState::empty());
        let stack_style = taffy_style_for_widget(&stack, &parent, &stack_computed, false);
        assert_eq!(stack_style.display, tf::Display::Grid);
        assert_eq!(stack_style.align_items, Some(tf::AlignItems::END));
        assert_eq!(stack_style.justify_items, Some(tf::AlignItems::END));

        let child = WidgetI::new(container());
        let child_computed = computed_style_for_widget(
            &child,
            &stack_computed,
            &theme,
            xui_interface::WidgetState::empty(),
        );
        let child_style = taffy_style_for_widget(&child, &stack_computed, &child_computed, true);
        assert_eq!(child_style.grid_row, tf::line(1));
        assert_eq!(child_style.grid_column, tf::line(1));
    }

    #[test]
    fn grid_widget_style_is_used_by_the_layout_pipeline() {
        let theme = Theme::default();
        let parent = ComputedStyle::initial(&theme);
        let widget = WidgetI::new(
            GridWidget::new()
                .columns(GridTracks::repeat(3, GridTrackSize::flexible()))
                .rows(GridTracks::explicit([GridTrackSize::fixed(24.0)]))
                .flow(GridFlow::ColumnDense)
                .style(xui_interface::Style::new().gap(12.0)),
        );
        let computed = computed_style_for_widget(
            &widget,
            &parent,
            &theme,
            xui_interface::WidgetState::empty(),
        );

        let style = taffy_style_for_widget(&widget, &parent, &computed, false);

        assert_eq!(style.display, tf::Display::Grid);
        assert_eq!(style.grid_auto_flow, tf::GridAutoFlow::ColumnDense);
        assert_eq!(style.grid_template_columns.len(), 1);
        assert_eq!(style.grid_template_rows.len(), 1);
        assert_eq!(style.gap.width, tf::LengthPercentage::length(12.0));
        assert_eq!(style.gap.height, tf::LengthPercentage::length(12.0));
    }

    #[test]
    fn absolute_child_does_not_expand_parent_intrinsic_size() {
        let mut taffy = tf::TaffyTree::<()>::new();
        let normal = taffy
            .new_leaf(tf::Style {
                size: tf::Size {
                    width: tf::Dimension::length(40.0),
                    height: tf::Dimension::length(30.0),
                },
                ..Default::default()
            })
            .unwrap();
        let absolute = taffy
            .new_leaf(tf::Style {
                position: tf::Position::Absolute,
                size: tf::Size {
                    width: tf::Dimension::length(100.0),
                    height: tf::Dimension::length(100.0),
                },
                ..Default::default()
            })
            .unwrap();
        let parent = taffy
            .new_with_children(tf::Style::default(), &[normal, absolute])
            .unwrap();
        taffy
            .compute_layout(
                parent,
                tf::Size {
                    width: tf::AvailableSpace::MaxContent,
                    height: tf::AvailableSpace::MaxContent,
                },
            )
            .unwrap();
        assert_eq!(taffy.layout(parent).unwrap().size.width, 40.0);
        assert_eq!(taffy.layout(parent).unwrap().size.height, 30.0);
    }

    #[test]
    fn absolute_and_inset_style_are_forwarded_to_taffy() {
        let theme = Theme::default();
        let parent = ComputedStyle::initial(&theme);
        let widget = WidgetI::new(
            container().style(
                xui_interface::Style::new()
                    .absolute()
                    .inset(EdgeInsets::new(3.0, 4.0, 5.0, 6.0)),
            ),
        );
        let computed = computed_style_for_widget(
            &widget,
            &parent,
            &theme,
            xui_interface::WidgetState::empty(),
        );
        let style = taffy_style_for_widget(&widget, &parent, &computed, false);
        assert_eq!(style.position, tf::Position::Absolute);
        assert_eq!(style.inset.left, tf::LengthPercentageAuto::length(3.0));
        assert_eq!(style.inset.right, tf::LengthPercentageAuto::length(4.0));
        assert_eq!(style.inset.top, tf::LengthPercentageAuto::length(5.0));
        assert_eq!(style.inset.bottom, tf::LengthPercentageAuto::length(6.0));
    }

    #[test]
    fn absolute_without_inset_keeps_taffy_auto_offsets() {
        let theme = Theme::default();
        let parent = ComputedStyle::initial(&theme);
        let widget = WidgetI::new(container().style(xui_interface::Style::new().absolute()));
        let computed = computed_style_for_widget(
            &widget,
            &parent,
            &theme,
            xui_interface::WidgetState::empty(),
        );
        let style = taffy_style_for_widget(&widget, &parent, &computed, false);

        assert_eq!(style.position, tf::Position::Absolute);
        assert_eq!(style.inset.left, tf::LengthPercentageAuto::auto());
        assert_eq!(style.inset.right, tf::LengthPercentageAuto::auto());
        assert_eq!(style.inset.top, tf::LengthPercentageAuto::auto());
        assert_eq!(style.inset.bottom, tf::LengthPercentageAuto::auto());
    }
}
