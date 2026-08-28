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

/// How a container arranges its children.
///
/// `Sizing::Fill` means a different taffy property in each case, and the
/// previous code could not tell them apart: it took only the parent's
/// `flex_direction`, a field that is a leftover default on every parent that is
/// not a flex container. A `Fill` child of a grid, a z-stack, or a plain
/// (block) container was given `flex_grow: 1` — which those layout modes ignore
/// — and so silently did not fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentLayout {
    /// A plain container: children stack in block flow. Deliberate — only
    /// `row` and `column` turn flex on.
    Block,
    Flex(FlexDirectionStyle),
    Grid,
    /// A grid used as a single cell that every child shares.
    ZStack,
}

/// The layout kind a widget imposes on its children.
///
/// Must agree with the `display` chosen in [`computed_layout_style_for_parent`];
/// `container_layout_matches_the_taffy_display` holds them together.
pub fn container_layout(widget: &WidgetI) -> ParentLayout {
    widget.with_widgets(|widget| match widget {
        Widgets::Container(container) => match container.flex_direction {
            Some(direction) => ParentLayout::Flex(direction),
            None => ParentLayout::Block,
        },
        Widgets::Root(_) => ParentLayout::Flex(FlexDirectionStyle::Column),
        Widgets::ZStack(_) => ParentLayout::ZStack,
        Widgets::Grid(_) => ParentLayout::Grid,
        _ => ParentLayout::Block,
    })
}

#[inline(always)]
pub fn taffy_style_for_widget(
    widget: &WidgetI,
    parent_layout: ParentLayout,
    computed: &ComputedStyle,
) -> tf::Style {
    computed_layout_style_for_parent(widget, parent_layout, computed)
}

fn computed_layout_style_for_parent(
    widget: &WidgetI,
    parent_layout: ParentLayout,
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
    if parent_layout == ParentLayout::ZStack {
        style.grid_row = tf::line(1);
        style.grid_column = tf::line(1);
    }

    let size = layout.size();
    style.size = tf::Size {
        width: dimension_for_axis(size.width, Axis::Horizontal, parent_layout),
        height: dimension_for_axis(size.height, Axis::Vertical, parent_layout),
    };

    // Growing and shrinking are flex-only concepts. Setting them under a grid or
    // a block parent is not merely useless: it used to be paired with an `auto`
    // dimension, so the child silently kept its content size instead of filling.
    if let ParentLayout::Flex(direction) = parent_layout {
        let main = match direction {
            FlexDirectionStyle::Column => size.height(),
            FlexDirectionStyle::Row => size.width(),
        };
        if matches!(main, Sizing::Fill) {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = tf::Dimension::length(0.0);
        }
        if matches!(main, Sizing::Fix(_)) {
            style.flex_shrink = 0.0;
        }
    }

    let min_size = layout.min_size();
    style.min_size = tf::Size {
        width: min_size
            .width()
            .map(|w| dimension_for_axis(w, Axis::Horizontal, parent_layout))
            .unwrap_or(taffy::Dimension::ZERO),
        height: min_size
            .height()
            .map(|h| dimension_for_axis(h, Axis::Vertical, parent_layout))
            .unwrap_or(taffy::Dimension::ZERO),
    };

    let max_size = layout.max_size();

    style.max_size = tf::Size {
        width: max_size
            .width()
            .map(|w| dimension_for_axis(w, Axis::Horizontal, parent_layout))
            .unwrap_or(taffy::Dimension::auto()),
        height: max_size
            .height()
            .map(|h| dimension_for_axis(h, Axis::Vertical, parent_layout))
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

/// Whether `axis` is the axis a flex parent distributes free space along.
fn is_main_axis(axis: Axis, direction: FlexDirectionStyle) -> bool {
    matches!(
        (axis, direction),
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

fn dimension_for_axis(value: Sizing, axis: Axis, parent_layout: ParentLayout) -> tf::Dimension {
    match value {
        // Along a flex main axis the dimension stays `auto` and `flex_grow`
        // does the filling. Everywhere else — grid tracks, z-stack cells, block
        // flow — asking for the full extent of the containing block is what
        // "fill" means, and it degrades to `auto` on its own when the parent has
        // no definite size in that axis.
        Sizing::Fill => match parent_layout {
            ParentLayout::Flex(direction) if is_main_axis(axis, direction) => tf::Dimension::auto(),
            _ => tf::Dimension::percent(1.0),
        },
        Sizing::Hug => tf::Dimension::auto(),
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

    /// `container_layout` and the `display` chosen when building the taffy style
    /// are two readings of the same decision. If they drift, a child is told its
    /// parent lays out one way while the parent actually lays out another — the
    /// exact failure that made `Sizing::Fill` silently do nothing.
    #[test]
    fn container_layout_matches_the_taffy_display() {
        use crate::widgets::{GridWidget, TextWidget, root_widget, text_input};

        let theme = Theme::default();
        let parent = ComputedStyle::initial(&theme);
        let cases: Vec<WidgetI> = vec![
            WidgetI::new(container()),
            WidgetI::new(container().flex_direction(FlexDirectionStyle::Row)),
            WidgetI::new(container().flex_direction(FlexDirectionStyle::Column)),
            WidgetI::new(ZStackWidget::new()),
            WidgetI::new(GridWidget::new()),
            WidgetI::new(TextWidget::new("t")),
            WidgetI::new(text_input()),
            root_widget(),
        ];

        for widget in cases {
            let computed =
                computed_style_for_widget(&widget, &parent, &theme, WidgetState::empty());
            let style = taffy_style_for_widget(&widget, ParentLayout::Block, &computed);
            let expected = match container_layout(&widget) {
                ParentLayout::Block => tf::Display::Block,
                ParentLayout::Flex(_) => tf::Display::Flex,
                ParentLayout::Grid | ParentLayout::ZStack => tf::Display::Grid,
            };
            assert_eq!(
                style.display,
                expected,
                "`container_layout` says {:?} but the taffy style says {:?} for {:?}",
                container_layout(&widget),
                style.display,
                widget.node_type()
            );
        }
    }

    /// `Fill` used to mean `flex_grow: 1` regardless of the parent, and
    /// `flex_grow` is ignored by grid and block layout — so a `Fill` child of
    /// anything but a row or a column silently kept its content size.
    #[test]
    fn fill_resolves_against_the_parents_actual_layout_mode() {
        let theme = Theme::default();
        let root = ComputedStyle::initial(&theme);
        let widget = WidgetI::new(
            container().style(
                xui_interface::Style::new()
                    .width(Sizing::Fill)
                    .height(Sizing::Fill),
            ),
        );
        let computed = computed_style_for_widget(&widget, &root, &theme, WidgetState::empty());

        let full = tf::Dimension::percent(1.0);
        let auto = tf::Dimension::auto();

        // Flex: the main axis grows, the cross axis takes the full extent.
        let in_column =
            taffy_style_for_widget(&widget, ParentLayout::Flex(FlexDirectionStyle::Column), &computed);
        assert_eq!(in_column.size.width, full);
        assert_eq!(in_column.size.height, auto);
        assert_eq!(in_column.flex_grow, 1.0);

        let in_row =
            taffy_style_for_widget(&widget, ParentLayout::Flex(FlexDirectionStyle::Row), &computed);
        assert_eq!(in_row.size.width, auto);
        assert_eq!(in_row.size.height, full);
        assert_eq!(in_row.flex_grow, 1.0);

        // Everywhere else "fill" is the full extent on both axes, and asking to
        // grow would be meaningless.
        for parent in [ParentLayout::Grid, ParentLayout::ZStack, ParentLayout::Block] {
            let style = taffy_style_for_widget(&widget, parent, &computed);
            assert_eq!(style.size.width, full, "width under {parent:?}");
            assert_eq!(style.size.height, full, "height under {parent:?}");
            assert_eq!(style.flex_grow, 0.0, "flex_grow under {parent:?}");
        }
    }

    /// A z-stack child fills the shared cell instead of collapsing to its
    /// content height.
    #[test]
    fn a_filling_child_of_a_z_stack_covers_the_stack() {
        let mut taffy = tf::TaffyTree::<()>::new();
        let theme = Theme::default();
        let root = ComputedStyle::initial(&theme);

        let filler = WidgetI::new(
            container().style(
                xui_interface::Style::new()
                    .width(Sizing::Fill)
                    .height(Sizing::Fill),
            ),
        );
        let filler_computed =
            computed_style_for_widget(&filler, &root, &theme, WidgetState::empty());
        let filler_style = taffy_style_for_widget(&filler, ParentLayout::ZStack, &filler_computed);

        let filler_node = taffy.new_leaf(filler_style).unwrap();
        let sizer = taffy.new_leaf(fixed_grid_child(40.0, 30.0)).unwrap();
        let stack = WidgetI::new(ZStackWidget::new());
        let stack_computed = computed_style_for_widget(&stack, &root, &theme, WidgetState::empty());
        let stack_node = taffy
            .new_with_children(
                taffy_style_for_widget(&stack, ParentLayout::Block, &stack_computed),
                &[filler_node, sizer],
            )
            .unwrap();

        taffy
            .compute_layout(
                stack_node,
                tf::Size {
                    width: tf::AvailableSpace::MaxContent,
                    height: tf::AvailableSpace::MaxContent,
                },
            )
            .unwrap();

        let stack_size = taffy.layout(stack_node).unwrap().size;
        let filler_size = taffy.layout(filler_node).unwrap().size;
        assert_eq!(stack_size.width, 40.0);
        assert_eq!(stack_size.height, 30.0);
        assert_eq!(
            (filler_size.width, filler_size.height),
            (40.0, 30.0),
            "the filling child did not cover the stack"
        );
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
        let stack_style = taffy_style_for_widget(&stack, ParentLayout::Block, &stack_computed);
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
        let child_style = taffy_style_for_widget(&child, ParentLayout::ZStack, &child_computed);
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

        let style = taffy_style_for_widget(&widget, ParentLayout::Block, &computed);

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
        let style = taffy_style_for_widget(&widget, ParentLayout::Block, &computed);
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
        let style = taffy_style_for_widget(&widget, ParentLayout::Block, &computed);

        assert_eq!(style.position, tf::Position::Absolute);
        assert_eq!(style.inset.left, tf::LengthPercentageAuto::auto());
        assert_eq!(style.inset.right, tf::LengthPercentageAuto::auto());
        assert_eq!(style.inset.top, tf::LengthPercentageAuto::auto());
        assert_eq!(style.inset.bottom, tf::LengthPercentageAuto::auto());
    }
}

