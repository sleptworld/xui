use std::time::Duration;

use xui::prelude::*;

use crate::{SelectionItem, SelectionModel, SelectionOrientation};

pub type BoolChangeCallback = Callback<bool>;
pub type IndexChangeCallback = Callback<usize>;
pub type ValueChangeCallback = Callback<f32>;

fn commit_bool(
    value: bool,
    controlled: bool,
    state: xui::state::State<bool>,
    callback: &Option<BoolChangeCallback>,
) {
    if !controlled {
        state.set(value);
    }
    if let Some(callback) = callback {
        callback.call(value);
    }
}

#[component]
#[defaults(
    checked = None,
    default_checked = false,
    indeterminate = false,
    disabled = false,
    label = None,
    on_change = None,
    accessibility_label = None,
    style = Style::new(),
)]
pub fn checkbox(
    checked: &Option<bool>,
    default_checked: &bool,
    indeterminate: &bool,
    disabled: &bool,
    label: &Option<ElementDesc>,
    on_change: &Option<BoolChangeCallback>,
    accessibility_label: &Option<String>,
    style: &Style,
) {
    let state = cx.use_state(|| *default_checked);
    let controlled = checked.is_some();
    let value = checked.unwrap_or(*state.get());
    let callback = on_change.clone();
    let mark = if *indeterminate {
        "−"
    } else if value {
        "✓"
    } else {
        ""
    };
    let checked_state = if *indeterminate {
        AccessibilityChecked::Mixed
    } else if value {
        AccessibilityChecked::True
    } else {
        AccessibilityChecked::False
    };

    let box_style = style!{
        size: Size::fix(20.0, 20.0),
        border_width: 1.0,
        border_color: if value || *indeterminate {
            ColorToken::Primary
        } else {
            ColorToken::Border
        },
        background: if value || *indeterminate {
            ColorStyle::from(ColorToken::Primary)
        } else {
            ColorStyle::from(Color::TRANSPARENT)
        },
        border_radius: RadiusToken::Sm,
    };

    let children = xui!{

        <center 
            style={box_style}>
                <text>
                {
                    mark
                }
                </text>
        </center>

    };
    
    // let mut children = vec![
    //     ContainerWidget::new()
    //         .style(box_style)
    //         .into_element_desc(vec![TextWidget::new(mark).into_element_desc()]),
    // ];

    // if let Some(label) = label {
    //     children.push(label.clone());
    // }

    // let mut root = ContainerWidget::new()
    //     .style(
    //         Style::new()
    //             .align(AlignStyle::Center)
    //             .gap(8.0)
    //             .min_height(32.0)
    //             .merge_clone(style),
    //     )
    //     .flex_direction(FlexDirectionStyle::Row)
    //     .focusable(!*disabled)
    //     .tab_index(if *disabled { -1 } else { 0 })
    //     .accessibility_role(AccessibilityRole::Checkbox)
    //     .accessibility_checked(checked_state)
    //     .accessibility_disabled(*disabled);
    // if let Some(label) = accessibility_label {
    //     root = root.accessibility_label(label.clone());
    // }
    // if !*disabled {
    //     root = root.on_click(move |event, event_cx| {
    //         if event
    //             .button
    //             .is_some_and(|button| button != PointerButton::Primary)
    //         {
    //             return EventResult::Ignored;
    //         }
    //         event_cx.request_focus();
    //         commit_bool(!value, controlled, state, &callback);
    //         EventResult::Consumed
    //     });
    // }
    // root.into_element_desc(children);

    xui!{
        <center
            gap={8.0}
            min_height={32.}

            flex_direction = {FlexDirectionStyle::Row}
            focusable = {!*disabled}
            tab_index = {if *disabled { -1 } else { 0 }}
            accessibility_role = {AccessibilityRole::Checkbox}
            accessibility_checked = {checked_state}
            accessibility_disabled = {*disabled}

            on_click={move |event, event_cx|{
                if event
                    .button
                    .is_some_and(|button| button != PointerButton::Primary)
                {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                commit_bool(!value, controlled, state, &callback);
                EventResult::Consumed
            }}
        >

        {
            children
        }
        </center>
    }
}

pub const SWITCH_TRACK_WIDTH: f32 = 44.0;
pub const SWITCH_TRACK_HEIGHT: f32 = 24.0;
pub const SWITCH_THUMB_SIZE: f32 = 20.0;
pub const SWITCH_TRACK_PADDING: f32 = 2.0;

/// 滑块从关到开要走的距离：轨道内宽减去滑块本身。
const SWITCH_TRAVEL: f32 =
    SWITCH_TRACK_WIDTH - SWITCH_THUMB_SIZE - SWITCH_TRACK_PADDING * 2.0;

#[component]
#[defaults(
    checked = None,
    default_checked = false,
    disabled = false,
    label = None,
    on_change = None,
    accessibility_label = None,
    style = Style::new(),
)]
pub fn switch(
    checked: &Option<bool>,
    default_checked: &bool,
    disabled: &bool,
    label: &Option<ElementDesc>,
    on_change: &Option<BoolChangeCallback>,
    accessibility_label: &Option<String>,
    style: &Style,
) {
    let state = cx.use_state(|| *default_checked);
    let controlled = checked.is_some();
    let value = checked.unwrap_or(*state.get());
    let callback = on_change.clone();
    let disabled = *disabled;

    let motion = Transition::new(Duration::from_millis(180)).ease(Easing::CubicOut);

    let track_style = style!{
        size: Size::fix(SWITCH_TRACK_WIDTH, SWITCH_TRACK_HEIGHT),
        padding: EdgeInsets::all(SWITCH_TRACK_PADDING),
        border_radius: SWITCH_TRACK_HEIGHT / 2.0,
        align: AlignStyle::Center,
        justify: JustifyStyle::Start,
        background: if value {
            ColorStyle::from(ColorToken::Primary)
        } else {
            ColorStyle::from(ColorToken::MutedSurface)
        },
    }.transition(motion);

    let thumb_style = style!{
        size: Size::fix(SWITCH_THUMB_SIZE, SWITCH_THUMB_SIZE),
        border_radius: SWITCH_THUMB_SIZE / 2.0,
        background: Color::WHITE,
        translate_x: if value { SWITCH_TRAVEL } else { 0.0 },
    }.transition(motion);

    let track = xui!{
        <container style={track_style}>
            <container style={thumb_style} />
        </container>
    };

    let mut children = vec![track];
    if let Some(label) = label {
        children.push(label.clone());
    }

    let root_style = Style::new()
        .align(AlignStyle::Center)
        .gap(8.0)
        .min_height(32.0)
        .merge_clone(style);

    xui!{
        <center
            style={root_style}

            flex_direction={FlexDirectionStyle::Row}
            focusable={!disabled}
            tab_index={if disabled { -1 } else { 0 }}
            accessibility_role={AccessibilityRole::Switch}
            accessibility_checked={if value {
                AccessibilityChecked::True
            } else {
                AccessibilityChecked::False
            }}
            accessibility_disabled={disabled}
            accessibility_label={accessibility_label.clone().unwrap_or_default()}

            on_click={move |event, event_cx| {
                if disabled {
                    return EventResult::Ignored;
                }
                if event
                    .button
                    .is_some_and(|button| button != PointerButton::Primary)
                {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                commit_bool(!value, controlled, state, &callback);
                EventResult::Consumed
            }}
        >
        {
            children
        }
        </center>
    }
}


#[derive(Clone, Debug)]
pub struct RadioItem {
    pub id: String,
    pub label: ElementDesc,
    pub accessibility_label: String,
    pub disabled: bool,
}

impl RadioItem {
    pub fn new(
        id: impl Into<String>,
        accessibility_label: impl Into<String>,
        label: impl Into<ElementDesc>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            accessibility_label: accessibility_label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[component]
#[defaults(
    selected = None,
    default_selected = 0,
    disabled = false,
    orientation = SelectionOrientation::Vertical,
    on_change = None,
    accessibility_label = None,
    style = Style::new(),
)]
pub fn radio_group(
    items: &Vec<RadioItem>,
    selected: &Option<usize>,
    default_selected: &usize,
    disabled: &bool,
    orientation: &SelectionOrientation,
    on_change: &Option<IndexChangeCallback>,
    accessibility_label: &Option<String>,
    style: &Style,
) {
    let orientation = *orientation;
    let model = SelectionModel::new(items.iter().map(|item| {
        SelectionItem::new(
            item.id.clone(),
            item.accessibility_label.clone(),
            *disabled || item.disabled,
        )
    }));
    let initial = model.normalize(*default_selected).unwrap_or(0);
    let state = cx.use_state(|| initial);
    let active = model.normalize(selected.unwrap_or(*state.get()));
    let controlled = selected.is_some();
    let handles = cx.use_memo_with(
        items.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        || {
            (0..items.len())
                .map(|_| FocusHandle::new())
                .collect::<Vec<_>>()
        },
    );
    let mut children = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let selected_now = active == Some(index);
        let item_disabled = *disabled || item.disabled;
        let callback = on_change.clone();
        let model = model.clone();
        let handles = handles.get().clone();
        let mut row = ContainerWidget::new()
            .style(
                Style::new()
                    .align(AlignStyle::Center)
                    .gap(8.0)
                    .min_height(32.0),
            )
            .flex_direction(FlexDirectionStyle::Row)
            .focusable(!item_disabled)
            .tab_index(if selected_now { 0 } else { -1 })
            .focus_handle(handles[index].clone())
            .accessibility_role(AccessibilityRole::Radio)
            .accessibility_label(item.accessibility_label.clone())
            .accessibility_checked(if selected_now {
                AccessibilityChecked::True
            } else {
                AccessibilityChecked::False
            })
            .accessibility_disabled(item_disabled);
        if !item_disabled {
            row = row
                .on_click(move |_, event_cx| {
                    event_cx.request_focus();
                    if !controlled {
                        state.set(index);
                    }
                    if let Some(callback) = &callback {
                        callback.call(index);
                    }
                    EventResult::Consumed
                })
                .on_key_down(move |event, event_cx| {
                    let direction = match (orientation, event.named_key) {
                        (SelectionOrientation::Horizontal, Some(NamedKey::ArrowLeft))
                        | (SelectionOrientation::Vertical, Some(NamedKey::ArrowUp)) => -1,
                        (SelectionOrientation::Horizontal, Some(NamedKey::ArrowRight))
                        | (SelectionOrientation::Vertical, Some(NamedKey::ArrowDown)) => 1,
                        _ => return EventResult::Ignored,
                    };
                    let Some(next) = model.adjacent(index, direction) else {
                        return EventResult::Ignored;
                    };
                    handles[next].request_focus(event_cx);
                    if !controlled {
                        state.set(next);
                    }
                    EventResult::Consumed
                });
        }
        let dot = ContainerWidget::new()
            .style(
                Style::new()
                    .size(Size::fix(18.0, 18.0))
                    .border_width(if selected_now { 5.0 } else { 1.0 })
                    .border_color(if selected_now {
                        ColorToken::Primary
                    } else {
                        ColorToken::Border
                    })
                    .border_radius(9.0),
            )
            .into_element_desc(vec![]);
        children.push(row.into_element_desc(vec![dot, item.label.clone()]));
    }
    let mut root = ContainerWidget::new()
        .style(Style::new().gap(4.0).merge_clone(style))
        .flex_direction(match orientation {
            SelectionOrientation::Horizontal => FlexDirectionStyle::Row,
            SelectionOrientation::Vertical => FlexDirectionStyle::Column,
        })
        .accessibility_role(AccessibilityRole::RadioGroup)
        .accessibility_orientation(match orientation {
            SelectionOrientation::Horizontal => AccessibilityOrientation::Horizontal,
            SelectionOrientation::Vertical => AccessibilityOrientation::Vertical,
        });
    if let Some(label) = accessibility_label {
        root = root.accessibility_label(label.clone());
    }
    root.into_element_desc(children)
}

fn clamp_step(value: f32, min: f32, max: f32, step: f32) -> f32 {
    let min = min.min(max);
    let max = max.max(min);
    let step = step.abs().max(f32::EPSILON);
    let steps = ((value - min) / step).round();
    (min + steps * step).clamp(min, max)
}

#[component]
#[defaults(
    value = None,
    default_value = 0.0,
    min = 0.0,
    max = 100.0,
    step = 1.0,
    disabled = false,
    on_change = None,
    accessibility_label = None,
    style = Style::new(),
)]
pub fn slider(
    value: &Option<f32>,
    default_value: &f32,
    min: &f32,
    max: &f32,
    step: &f32,
    disabled: &bool,
    on_change: &Option<ValueChangeCallback>,
    accessibility_label: &Option<String>,
    style: &Style,
) {
    let min = *min;
    let max = *max;
    let step = *step;
    let state = cx.use_state(|| clamp_step(*default_value, min, max, step));
    let controlled = value.is_some();
    let current = clamp_step(value.unwrap_or(*state.get()), min, max, step);
    let span = (max - min).abs();
    let ratio = if span <= f32::EPSILON {
        0.0
    } else {
        ((current - min.min(max)) / span).clamp(0.0, 1.0)
    };
    let commit = on_change.clone();
    let mut root = ContainerWidget::new()
        .style(
            Style::new()
                .relative()
                .height(32.0)
                .justify(JustifyStyle::Center)
                .merge_clone(style),
        )
        .focusable(!*disabled)
        .tab_index(if *disabled { -1 } else { 0 })
        .accessibility_role(AccessibilityRole::Slider)
        .accessibility_numeric_value(current as f64)
        .accessibility_value_range(min as f64, max as f64, Some(step as f64))
        .accessibility_disabled(*disabled);
    if let Some(label) = accessibility_label {
        root = root.accessibility_label(label.clone());
    }
    if !*disabled {
        root = root
            .on_click(move |event, event_cx| {
                let Some(pointer) = event.pointer.as_ref() else {
                    return EventResult::Ignored;
                };
                let width = event_cx.node_ref.layout.width().max(1.0);
                let local = pointer.coords.window.x - event_cx.node_ref.world_origin.x;
                let next = clamp_step(
                    min + (local / width).clamp(0.0, 1.0) * (max - min),
                    min,
                    max,
                    step,
                );
                if !controlled {
                    state.set(next);
                }
                if let Some(callback) = &commit {
                    callback.call(next);
                }
                event_cx.request_focus();
                EventResult::Consumed
            })
            .on_key_down({
                let callback = on_change.clone();
                move |event, _| {
                    let next = match event.named_key {
                        Some(NamedKey::ArrowLeft) | Some(NamedKey::ArrowDown) => current - step,
                        Some(NamedKey::ArrowRight) | Some(NamedKey::ArrowUp) => current + step,
                        Some(NamedKey::Home) => min,
                        Some(NamedKey::End) => max,
                        _ => return EventResult::Ignored,
                    };
                    let next = clamp_step(next, min, max, step);
                    if !controlled {
                        state.set(next);
                    }
                    if let Some(callback) = &callback {
                        callback.call(next);
                    }
                    EventResult::Consumed
                }
            });
    }
    let rail = ContainerWidget::new()
        .style(
            Style::new()
                .relative()
                .width(Sizing::Fill)
                .height(6.0)
                .background(ColorToken::MutedSurface)
                .border_radius(3.0),
        )
        .into_element_desc(vec![
            ContainerWidget::new()
                .style(
                    Style::new()
                        .width(Sizing::percent(ratio))
                        .height(6.0)
                        .background(ColorToken::Primary)
                        .border_radius(3.0),
                )
                .into_element_desc(vec![]),
        ]);
    root.into_element_desc(vec![rail])
}

#[component]
#[defaults(value = 0.0, max = 100.0, accessibility_label = None, style = Style::new())]
pub fn progress(value: &f32, max: &f32, accessibility_label: &Option<String>, style: &Style) {
    let max = max.abs().max(f32::EPSILON);
    let value = value.clamp(0.0, max);
    let mut root = ContainerWidget::new()
        .style(
            Style::new()
                .width(Sizing::Fill)
                .height(8.0)
                .background(ColorToken::MutedSurface)
                .border_radius(4.0)
                .merge_clone(style),
        )
        .accessibility_role(AccessibilityRole::ProgressIndicator)
        .accessibility_numeric_value(value as f64)
        .accessibility_value_range(0.0, max as f64, None);
    if let Some(label) = accessibility_label {
        root = root.accessibility_label(label.clone());
    }
    root.into_element_desc(vec![
        ContainerWidget::new()
            .style(
                Style::new()
                    .width(Sizing::percent(value / max))
                    .height(Sizing::Fill)
                    .background(ColorToken::Primary)
                    .border_radius(4.0),
            )
            .into_element_desc(vec![]),
    ])
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SeparatorOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[component]
#[defaults(orientation = SeparatorOrientation::Horizontal, style = Style::new())]
pub fn separator(orientation: &SeparatorOrientation, style: &Style) {
    let base = match orientation {
        SeparatorOrientation::Horizontal => Style::new().width(Sizing::Fill).height(1.0),
        SeparatorOrientation::Vertical => Style::new().width(1.0).height(Sizing::Fill),
    };
    ContainerWidget::new()
        .style(base.background(ColorToken::Border).merge_clone(style))
        .accessibility_hidden(true)
        .into_element_desc(vec![])
}

#[component]
#[defaults(style = Style::new())]
pub fn badge(text: &String, style: &Style) {
    ContainerWidget::new()
        .style(
            Style::new()
                .padding(EdgeInsets::symmetric(8.0, 3.0))
                .background(ColorToken::MutedSurface)
                .border_radius(999.0)
                .font_size(FontSizeToken::Sm)
                .merge_clone(style),
        )
        .into_element_desc(vec![TextWidget::new(text.clone()).into_element_desc()])
}

#[component]
#[defaults(size = 40.0, image = None, fallback = "?".to_string(), style = Style::new())]
pub fn avatar(
    accessibility_label: &String,
    size: &f32,
    image: &Option<ElementDesc>,
    fallback: &String,
    style: &Style,
) {
    ContainerWidget::new()
        .style(
            Style::new()
                .size(Size::fix(*size, *size))
                .align(AlignStyle::Center)
                .justify(JustifyStyle::Center)
                .clip(true)
                .background(ColorToken::MutedSurface)
                .border_radius(*size / 2.0)
                .merge_clone(style),
        )
        .accessibility_role(AccessibilityRole::Image)
        .accessibility_label(accessibility_label.clone())
        .into_element_desc(vec![
            image
                .clone()
                .unwrap_or_else(|| TextWidget::new(fallback.clone()).into_element_desc()),
        ])
}

#[component]
#[defaults(style = Style::new())]
pub fn card(content: &ElementDesc, style: &Style) {
    ContainerWidget::new()
        .style(
            Style::new()
                .padding(EdgeInsets::all(16.0))
                .background(ColorToken::Surface)
                .border_width(1.0)
                .border_color(ColorToken::Border)
                .border_radius(RadiusToken::Lg)
                .merge_clone(style),
        )
        .into_element_desc(vec![content.clone()])
}

trait MergeClone {
    fn merge_clone(self, other: &Style) -> Style;
}

impl MergeClone for Style {
    fn merge_clone(mut self, other: &Style) -> Style {
        self.merge(other);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepped_values_are_clamped_and_rounded() {
        assert_eq!(clamp_step(10.6, 0.0, 10.0, 1.0), 10.0);
        assert_eq!(clamp_step(4.9, 0.0, 10.0, 2.0), 4.0);
        assert_eq!(clamp_step(-2.0, 0.0, 10.0, 1.0), 0.0);
    }
}
