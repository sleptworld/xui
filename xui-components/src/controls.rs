//! Boolean, choice, range, and compact content components.
//!
//! Use [`checkbox`], [`switch`], [`radio_group`], and [`slider`] for interaction;
//! [`progress`] reports a numeric value. [`badge`], [`avatar`], [`card`], and
//! [`separator`] provide themed content structure. Stateful controls accept optional
//! controlled values, defaults, and callbacks.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`checkbox`](fn@self::checkbox) | A labeled checkbox supporting controlled, uncontrolled, and indeterminate states. |
//! | [`switch`](fn@self::switch) | A boolean switch with a sliding thumb and optional label. |
//! | [`radio_group`](fn@self::radio_group) | A group of mutually exclusive radio choices with roving keyboard focus. |
//! | [`slider`](fn@self::slider) | A numeric slider with stepped pointer and keyboard adjustment. |
//! | [`progress`](fn@self::progress) | A determinate progress bar with accessible numeric value metadata. |
//! | [`separator`](fn@self::separator) | A decorative horizontal or vertical divider. |
//! | [`badge`](fn@self::badge) | A compact text badge with semantic color variants. |
//! | [`avatar`](fn@self::avatar) | A circular avatar showing a supplied image element or text fallback. |
//! | [`card`](fn@self::card) | A themed, bordered content panel with rounded corners. |

use std::time::Duration;

use crate::layout::MergeClone;
use xui_core::prelude::*;

use xui_macros::{component, style, view};

use crate::{SelectionItem, SelectionModel, SelectionOrientation};

/// Callback requesting a boolean state change.
pub type BoolChangeCallback = Callback<bool>;
/// Callback requesting a zero-based selected item index.
pub type IndexChangeCallback = Callback<usize>;
/// Callback requesting a numeric control value.
pub type ValueChangeCallback = Callback<f32>;

const PREVIOUS: CommandId = CommandId("xui.control.previous");
const NEXT: CommandId = CommandId("xui.control.next");
const FIRST: CommandId = CommandId("xui.control.first");
const LAST: CommandId = CommandId("xui.control.last");
fn navigation_shortcuts() -> [(Shortcut, CommandId); 6] {
    [
        (
            Shortcut::named(NamedKey::ArrowLeft).allow_repeat(true),
            PREVIOUS,
        ),
        (
            Shortcut::named(NamedKey::ArrowDown).allow_repeat(true),
            PREVIOUS,
        ),
        (
            Shortcut::named(NamedKey::ArrowRight).allow_repeat(true),
            NEXT,
        ),
        (Shortcut::named(NamedKey::ArrowUp).allow_repeat(true), NEXT),
        (Shortcut::named(NamedKey::Home), FIRST),
        (Shortcut::named(NamedKey::End), LAST),
    ]
}

fn commit_bool(
    value: bool,
    controlled: bool,
    state: xui_core::state::State<bool>,
    callback: &Option<BoolChangeCallback>,
) {
    if !controlled {
        state.set(value);
    }
    if let Some(callback) = callback {
        callback.call(value);
    }
}

/// A labeled checkbox supporting controlled, uncontrolled, and indeterminate states.
///
/// `checked` is the controlled value; otherwise `default_checked` seeds internal state.
/// `on_change` reports requested boolean changes, including in controlled mode.
/// `indeterminate` exposes a mixed accessibility state. Disabled checkboxes ignore
/// activation and leave the tab order. `indicator_style` customizes the mark box; `style`
/// customizes the labeled row.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `checked` | `Option<bool>` | `None` |
/// | `default_checked` | `bool` | `false` |
/// | `indeterminate` | `bool` | `false` |
/// | `disabled` | `bool` | `false` |
/// | `label` | `Option<ElementDesc>` | `None` |
/// | `on_change` | `Option<BoolChangeCallback>` | `None` |
/// | `accessibility_label` | `Option<String>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `indicator_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { checkbox(default_checked: true, label: Some(view! { text("Remember me") })) };
/// ```
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
    indicator_style = Style::new(),
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
    indicator_style: &Style,
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

    let box_style = style! {
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
    }
    .merge_clone(indicator_style);

    let disabled = *disabled;
    let root_style = crate::layout::control_style(disabled)
        .align(AlignStyle::Center)
        .gap(8.0)
        .min_height(32.0)
        .merge_clone(style);
    view! {
        row(style: root_style, focusable: !disabled,
            tab_index: if disabled { -1 } else { 0 },
            accessibility_role: AccessibilityRole::Checkbox,
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
            accessibility_checked: checked_state, accessibility_disabled: disabled,
            on_click: move |event, event_cx| {
                if disabled || event.button.is_some_and(|button| button != PointerButton::Primary) {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                commit_bool(!value, controlled, state, &callback);
                EventResult::Consumed
        }) {
            center(style: box_style.color(ColorToken::PrimaryForeground)) {
                xui_core::widgets::text(mark)
            }
            if let Some(label) = label { ..label.clone() }
        }
    }
}

pub const SWITCH_TRACK_WIDTH: f32 = 44.0;
pub const SWITCH_TRACK_HEIGHT: f32 = 24.0;
pub const SWITCH_THUMB_SIZE: f32 = 20.0;
pub const SWITCH_TRACK_PADDING: f32 = 2.0;

/// 滑块从关到开要走的距离：轨道内宽减去滑块本身。
const SWITCH_TRAVEL: f32 = SWITCH_TRACK_WIDTH - SWITCH_THUMB_SIZE - SWITCH_TRACK_PADDING * 2.0;

/// A boolean switch with a sliding thumb and optional label.
///
/// `checked` controls the value or `default_checked` seeds internal state; `on_change`
/// reports requested changes. Disabled switches ignore activation and leave the tab
/// order. `track_style`, `thumb_style`, and `style` customize the track, thumb, and
/// labeled row independently.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `checked` | `Option<bool>` | `None` |
/// | `default_checked` | `bool` | `false` |
/// | `disabled` | `bool` | `false` |
/// | `label` | `Option<ElementDesc>` | `None` |
/// | `on_change` | `Option<BoolChangeCallback>` | `None` |
/// | `accessibility_label` | `Option<String>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `track_style` | `Style` | `Style::new()` |
/// | `thumb_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { switch(default_checked: true, label: Some(view! { text("Notifications") })) };
/// ```
#[component]
#[defaults(
    checked = None,
    default_checked = false,
    disabled = false,
    label = None,
    on_change = None,
    accessibility_label = None,
    style = Style::new(),
    track_style = Style::new(),
    thumb_style = Style::new(),
)]
pub fn switch(
    checked: &Option<bool>,
    default_checked: &bool,
    disabled: &bool,
    label: &Option<ElementDesc>,
    on_change: &Option<BoolChangeCallback>,
    accessibility_label: &Option<String>,
    style: &Style,
    track_style: &Style,
    thumb_style: &Style,
) {
    let state = cx.use_state(|| *default_checked);
    let controlled = checked.is_some();
    let value = checked.unwrap_or(*state.get());
    let callback = on_change.clone();
    let disabled = *disabled;

    let motion = Transition::new(Duration::from_millis(180)).ease(Easing::CubicOut);

    let resolved_track = style! {
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
    }
    .transition(motion)
    .merge_clone(track_style);

    let resolved_thumb = style! {
        size: Size::fix(SWITCH_THUMB_SIZE, SWITCH_THUMB_SIZE),
        border_radius: SWITCH_THUMB_SIZE / 2.0,
        background: ColorToken::Background,
        translate_x: if value { SWITCH_TRAVEL } else { 0.0 },
    }
    .transition(motion)
    .merge_clone(thumb_style);

    let track = view! {
        container(style: resolved_track) { container(style: resolved_thumb) }
    };

    let mut children = vec![track];
    if let Some(label) = label {
        children.push(label.clone());
    }

    let root_style = crate::layout::control_style(disabled)
        .align(AlignStyle::Center)
        .gap(8.0)
        .min_height(32.0)
        .merge_clone(style);

    view! {
        center(style: root_style, flex_direction: FlexDirectionStyle::Row, focusable: !disabled, tab_index: if disabled { -1 } else { 0 }, accessibility_role: AccessibilityRole::Switch, accessibility_checked: if value {
                AccessibilityChecked::True
            } else {
                AccessibilityChecked::False
            }, accessibility_disabled: disabled, accessibility_label: accessibility_label.clone().unwrap_or_default(), on_click: move |event, event_cx| {
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
        }) { ..
            children
        }
    }
}

/// An identified radio choice with a visual label, accessibility name, and disabled
/// state.
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

/// A group of mutually exclusive radio choices with roving keyboard focus.
///
/// `selected` controls the index or `default_selected` seeds internal state. Arrows
/// follow `orientation` and skip disabled items; Home/End choose the endpoints. Keyboard
/// and pointer selection both notify `on_change`. `item_style` and `indicator_style`
/// customize each choice and radio mark; `style` customizes the group.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `items` | `Vec<RadioItem>` | **Required** |
/// | `selected` | `Option<usize>` | `None` |
/// | `default_selected` | `usize` | `0` |
/// | `disabled` | `bool` | `false` |
/// | `orientation` | `SelectionOrientation` | `SelectionOrientation::Vertical` |
/// | `on_change` | `Option<IndexChangeCallback>` | `None` |
/// | `accessibility_label` | `Option<String>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `item_style` | `Style` | `Style::new()` |
/// | `indicator_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { radio_group(items: vec![RadioItem::new("a", "Choice A", view! { text("Choice A") })]) };
/// ```
#[component]
#[defaults(
    selected = None,
    default_selected = 0,
    disabled = false,
    orientation = SelectionOrientation::Vertical,
    on_change = None,
    accessibility_label = None,
    style = Style::new(),
    item_style = Style::new(),
    indicator_style = Style::new(),
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
    item_style: &Style,
    indicator_style: &Style,
) {
    let orientation = *orientation;
    let model = std::rc::Rc::new(SelectionModel::new(items.iter().map(|item| {
        SelectionItem::new(
            item.id.clone(),
            item.accessibility_label.clone(),
            *disabled || item.disabled,
        )
    })));
    let initial = model.normalize(*default_selected).unwrap_or(0);
    let state = cx.use_state(|| initial);
    let active = model.normalize(selected.unwrap_or(*state.get()));
    let controlled = selected.is_some();
    let handles = cx.use_memo_with(
        items.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        || {
            let handles: std::rc::Rc<[FocusHandle]> = (0..items.len())
                .map(|_| FocusHandle::new())
                .collect::<Vec<_>>()
                .into();
            handles
        },
    );
    let mut children = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let selected_now = active == Some(index);
        let item_disabled = *disabled || item.disabled;
        let callback = on_change.clone();
        let keyboard_callback = on_change.clone();
        let model = model.clone();
        let handles = handles.get().clone();
        let interactive = !item_disabled;

        let dot = view! {
            container(
                style: Style::new()
                    .size(Size::fix(18.0, 18.0))
                    .border_width(if selected_now { 5.0 } else { 1.0 })
                    .border_color(if selected_now {
                        ColorToken::Primary
                    } else {
                        ColorToken::Border
                })
                    .border_radius(9.0).merge_clone(indicator_style),
            )
        };
        children.push(view! {
            container(
                style: crate::layout::control_style(item_disabled)
                    .align(AlignStyle::Center)
                    .gap(8.0)
                    .min_height(32.0).merge_clone(item_style),
                flex_direction: FlexDirectionStyle::Row,
                focusable: !item_disabled,
                tab_index: if selected_now { 0 } else { -1 },
                focus_handle: handles[index].clone(),
                accessibility_role: AccessibilityRole::Radio,
                accessibility_label: item.accessibility_label.clone(),
                accessibility_checked: if selected_now {
                    AccessibilityChecked::True
                } else {
                    AccessibilityChecked::False
                },
                accessibility_disabled: item_disabled,
                on_click: move |_, event_cx| { if !interactive { return EventResult::Ignored; }
                    event_cx.request_focus();
                    if !controlled {
                        state.set(index);
                    }
                    if let Some(callback) = &callback {
                        callback.call(index);
                    }
                    EventResult::Consumed
                },
                shortcuts: [
                    (Shortcut::named(if orientation == SelectionOrientation::Horizontal { NamedKey::ArrowLeft } else { NamedKey::ArrowUp }).allow_repeat(true), PREVIOUS),
                    (Shortcut::named(if orientation == SelectionOrientation::Horizontal { NamedKey::ArrowRight } else { NamedKey::ArrowDown }).allow_repeat(true), NEXT),
                    (Shortcut::named(NamedKey::Home), FIRST), (Shortcut::named(NamedKey::End), LAST),
                ],
                on_command: move |event, event_cx| { if !interactive { return EventResult::Ignored; }
                    let next = match event.command {
                        FIRST => model.first(), LAST => model.last(),
                        PREVIOUS => model.adjacent(index, -1),
                        NEXT => model.adjacent(index, 1),
                        _ => return EventResult::Ignored,
                    };
                    let Some(next) = next else { return EventResult::Ignored; };
                    handles[next].request_focus(event_cx);
                    if !controlled {
                        state.set(next);
                    }
                    if let Some(callback) = &keyboard_callback { callback.call(next); }
                    EventResult::Consumed
                },
            ) {
                ..dot
                ..item.label.clone()
            }
        });
    }

    view! {
        container(
            style: Style::new().gap(4.0).merge_clone(style),
            flex_direction: match orientation {
                SelectionOrientation::Horizontal => FlexDirectionStyle::Row,
                SelectionOrientation::Vertical => FlexDirectionStyle::Column,
            },
            accessibility_role: AccessibilityRole::RadioGroup,
            accessibility_orientation: match orientation {
                SelectionOrientation::Horizontal => AccessibilityOrientation::Horizontal,
                SelectionOrientation::Vertical => AccessibilityOrientation::Vertical,
            },
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
        ) {
            ..children
        }
    }
}

fn slider_bounds(min: f32, max: f32, step: f32) -> (f32, f32, f32) {
    let min = if min.is_finite() { min } else { 0.0 };
    let max = if max.is_finite() { max } else { min };
    let step = if step.is_finite() && step.abs() > f32::EPSILON {
        step.abs()
    } else {
        1.0
    };
    (min.min(max), min.max(max), step)
}
fn clamp_step(value: f32, min: f32, max: f32, step: f32) -> f32 {
    let (min, max, step) = slider_bounds(min, max, step);
    let value = if value.is_nan() {
        min
    } else {
        value.clamp(min, max)
    };
    // f64 avoids overflow for valid but very large f32 ranges.
    let steps = ((f64::from(value) - f64::from(min)) / f64::from(step)).round();
    (f64::from(min) + steps * f64::from(step)).clamp(f64::from(min), f64::from(max)) as f32
}

/// A numeric slider with stepped pointer and keyboard adjustment.
///
/// `value` controls the number or `default_value` seeds internal state. Bounds are
/// ordered and the step is normalized to a positive finite value. Arrows change one step;
/// Home/End choose the bounds. `disabled` prevents changes. `track_style`, `fill_style`,
/// `thumb_style`, and `style` customize its visual parts. It uses a fixed horizontal
/// track.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `value` | `Option<f32>` | `None` |
/// | `default_value` | `f32` | `0.0` |
/// | `min` | `f32` | `0.0` |
/// | `max` | `f32` | `100.0` |
/// | `step` | `f32` | `1.0` |
/// | `disabled` | `bool` | `false` |
/// | `on_change` | `Option<ValueChangeCallback>` | `None` |
/// | `accessibility_label` | `Option<String>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `track_style` | `Style` | `Style::new()` |
/// | `fill_style` | `Style` | `Style::new()` |
/// | `thumb_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { slider(min: 0.0, max: 10.0, step: 0.5, default_value: 4.0) };
/// ```
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
    track_style = Style::new(),
    fill_style = Style::new(),
    thumb_style = Style::new(),
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
    track_style: &Style,
    fill_style: &Style,
    thumb_style: &Style,
) {
    let (min, max, step) = slider_bounds(*min, *max, *step);
    let state = cx.use_state(|| clamp_step(*default_value, min, max, step));
    let controlled = value.is_some();
    let current = clamp_step(value.unwrap_or(*state.get()), min, max, step);
    let span = f64::from(max) - f64::from(min);
    let ratio = if span <= f64::from(f32::EPSILON) {
        0.0
    } else {
        ((f64::from(current) - f64::from(min)) / span).clamp(0.0, 1.0) as f32
    };
    let commit = on_change.clone();
    let size = cx.use_size();
    let width = size.get().map_or(16.0, |size| size.width);

    let navigation_enabled = !*disabled;

    let rail = view! {
        container(
            style: Style::new()
                .relative()
                .width(Sizing::Fill)
                .height(6.0)
                .background(ColorToken::MutedSurface)
                .border_radius(3.0).merge_clone(track_style),
        ) {
            container(
                style: Style::new()
                    .width(Sizing::percent(ratio))
                    .height(6.0)
                    .background(ColorToken::Primary)
                    .border_radius(3.0).merge_clone(fill_style),
            )
        }
    };
    view! {
        container(
            style: crate::layout::control_style(*disabled)
                .relative()
                .height(32.0)
                .justify(JustifyStyle::Center)
                .merge_clone(style),
            observe_size: size,
            focusable: !*disabled,
            tab_index: if *disabled { -1 } else { 0 },
            accessibility_role: AccessibilityRole::Slider,
            accessibility_numeric_value: current as f64,
            accessibility_value_range: (min as f64, max as f64, Some(step as f64)),
            accessibility_disabled: *disabled,
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
            on_click: move |event, event_cx| { if !navigation_enabled { return EventResult::Ignored; }
                if event.button.is_some_and(|button| button != PointerButton::Primary) { return EventResult::Ignored; }
                let Some(pointer) = event.pointer.as_ref() else {
                    return EventResult::Ignored;
                };
                let width = event_cx.node_ref.layout.width().max(1.0);
                let local = pointer.coords.window.x - event_cx.node_ref.world_origin.x;
                let next = clamp_step(
                    (f64::from(min) + f64::from((local / width).clamp(0.0, 1.0)) * span) as f32,
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
            },
            shortcuts: navigation_shortcuts(),
            on_command: {
                let callback = on_change.clone();
                move |event, _| {
                    if !navigation_enabled { return EventResult::Ignored; }
                    let next = match event.command {
                        PREVIOUS => current - step,
                        NEXT => current + step,
                        FIRST => min,
                        LAST => max,
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
            },
        ) {
            ..rail
            container(
                style: Style::new().absolute().inset(EdgeInsets::new(0.0, 0.0, 8.0, 0.0))
                    .size(Size::fix(16.0, 16.0)).translate_x(ratio * (width - 16.0).max(0.0))
                    .background(ColorToken::Background).border_color(ColorToken::Primary)
                    .border_width(2.0).border_radius(8.0).merge_clone(thumb_style),
            )
        }
    }
}

/// A determinate progress bar with accessible numeric value metadata.
///
/// `value` is clamped to the positive finite `max`; invalid inputs receive safe defaults.
/// `indicator_style` customizes the filled portion and `style` customizes the track. Set
/// `accessibility_label` to describe the operation being measured.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `value` | `f32` | `0.0` |
/// | `max` | `f32` | `100.0` |
/// | `accessibility_label` | `Option<String>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `indicator_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { progress(value: 40.0, max: 100.0) };
/// ```
#[component]
#[defaults(
    value = 0.0,
    max = 100.0,
    accessibility_label = None,
    style = Style::new(),
    indicator_style = Style::new(),
)]
pub fn progress(
    value: &f32,
    max: &f32,
    accessibility_label: &Option<String>,
    style: &Style,
    indicator_style: &Style,
) {
    let max = if max.is_finite() && *max > 0.0 {
        *max
    } else {
        1.0
    };
    let value = if value.is_finite() {
        value.clamp(0.0, max)
    } else {
        0.0
    };

    view! {
        container(
            style: Style::new()
                .width(Sizing::Fill)
                .height(8.0)
                .background(ColorToken::MutedSurface)
                .border_radius(4.0)
                .merge_clone(style),
            accessibility_role: AccessibilityRole::ProgressIndicator,
            accessibility_numeric_value: value as f64,
            accessibility_value_range: (0.0, max as f64, None),
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
        ) {
            container(
                style: Style::new()
                    .width(Sizing::percent(value / max))
                    .height(Sizing::Fill)
                    .background(ColorToken::Primary)
                    .border_radius(4.0).merge_clone(indicator_style),
            )
        }
    }
}

/// The axis along which a decorative separator stretches.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SeparatorOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// A decorative horizontal or vertical divider.
///
/// `orientation` determines which axis fills the available space. `style` overrides the
/// border-colored line. The divider is hidden from accessibility.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `orientation` | `SeparatorOrientation` | `SeparatorOrientation::Horizontal` |
/// | `style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { separator(orientation: SeparatorOrientation::Horizontal) };
/// ```
#[component]
#[defaults(
    orientation = SeparatorOrientation::Horizontal,
    style = Style::new(),
)]
pub fn separator(orientation: &SeparatorOrientation, style: &Style) {
    let base = match orientation {
        SeparatorOrientation::Horizontal => Style::new().width(Sizing::Fill).height(1.0),
        SeparatorOrientation::Vertical => Style::new().width(1.0).height(Sizing::Fill),
    };
    view! {
        container(
            style: base.background(ColorToken::Border).merge_clone(style),
            accessibility_hidden: true,
        )
    }
}

/// Visual emphasis for inline status labels.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum BadgeVariant {
    Primary,
    #[default]
    Secondary,
    Outline,
    Danger,
}

/// A compact text badge with semantic color variants.
///
/// `variant` selects primary, secondary, outline, or destructive emphasis. `style`
/// overrides the themed fill, foreground, dimensions, and rounded shape.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `text` | `String` | **Required** |
/// | `style` | `Style` | `Style::new()` |
/// | `variant` | `BadgeVariant` | `BadgeVariant::Secondary` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { badge(text: "New", variant: BadgeVariant::Primary) };
/// ```
#[component]
#[defaults(
    style = Style::new(),
    variant = BadgeVariant::Secondary,
)]
pub fn badge(text: &String, style: &Style, variant: &BadgeVariant) {
    let variant_style = match variant {
        BadgeVariant::Primary => Style::new()
            .background(ColorToken::Primary)
            .color(ColorToken::PrimaryForeground),
        BadgeVariant::Secondary => Style::new()
            .background(ColorToken::Secondary)
            .color(ColorToken::SecondaryForeground),
        BadgeVariant::Outline => Style::new()
            .background(ColorToken::Background)
            .color(ColorToken::Foreground)
            .border_width(1.0)
            .border_color(ColorToken::Border),
        BadgeVariant::Danger => Style::new()
            .background(ColorToken::Destructive)
            .color(ColorToken::DestructiveForeground),
    };
    view! {
        container(
            style: variant_style
                .padding(EdgeInsets::symmetric(8.0, 3.0))
                .border_radius(999.0)
                .font_size(FontSizeToken::Sm)
                .merge_clone(style),
        ) {
            xui_core::widgets::text(
                text.clone(),
            )
        }
    }
}

/// A circular avatar showing a supplied image element or text fallback.
///
/// `image` replaces the fallback element. `fallback_style` customizes fallback text and
/// `style` customizes the clipped circular box. `size` controls the default diameter, and
/// `accessibility_label` names the person or entity represented.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `accessibility_label` | `String` | **Required** |
/// | `size` | `f32` | `40.0` |
/// | `image` | `Option<ElementDesc>` | `None` |
/// | `fallback` | `String` | `"?".to_string()` |
/// | `style` | `Style` | `Style::new()` |
/// | `fallback_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { avatar(accessibility_label: "Alex", fallback: "AL") };
/// ```
#[component]
#[defaults(
    size = 40.0,
    image = None,
    fallback = "?".to_string(),
    style = Style::new(),
    fallback_style = Style::new(),
)]
pub fn avatar(
    accessibility_label: &String,
    size: &f32,
    image: &Option<ElementDesc>,
    fallback: &String,
    style: &Style,
    fallback_style: &Style,
) {
    let size = &if size.is_finite() {
        size.max(0.0)
    } else {
        40.0
    };
    view! {
        container(
            style: Style::new()
                .size(Size::fix(*size, *size))
                .align(AlignStyle::Center)
                .justify(JustifyStyle::Center)
                .clip(true)
                .background(ColorToken::MutedSurface)
                .border_radius(*size / 2.0)
                .merge_clone(style),
            accessibility_role: AccessibilityRole::Image,
            accessibility_label: accessibility_label.clone(),
        ) {
            ..image
                .clone()
                .unwrap_or_else(|| view! {
                    xui_core::widgets::text(
                        fallback.clone(),
                        style: fallback_style.clone(),
                    )
            })
        }
    }
}

/// A themed, bordered content panel with rounded corners.
///
/// `content` accepts an arbitrary element subtree. `style` overrides the default padding,
/// surface fill, border, radius, and layout.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `content` | `ElementDesc` | **Required** |
/// | `style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { card(content: view! { text("Card content") }) };
/// ```
#[component]
#[defaults(
    style = Style::new(),
)]
pub fn card(content: &ElementDesc, style: &Style) {
    view! {
        container(
            style: Style::new()
                .padding(EdgeInsets::all(16.0))
                .background(ColorToken::Surface)
                .border_width(1.0)
                .border_color(ColorToken::Border)
                .border_radius(RadiusToken::Lg)
                .merge_clone(style),
        ) {
            ..content.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepped_values_are_clamped_and_rounded() {
        assert_eq!(
            clamp_step(f32::INFINITY, -f32::MAX, f32::MAX, 1.0),
            f32::MAX
        );
        assert_eq!(clamp_step(f32::NAN, 10.0, -10.0, 0.0), -10.0);
        assert_eq!(slider_bounds(10.0, -10.0, -2.0), (-10.0, 10.0, 2.0));
        assert_eq!(clamp_step(10.6, 0.0, 10.0, 1.0), 10.0);
        assert_eq!(clamp_step(4.9, 0.0, 10.0, 2.0), 4.0);
        assert_eq!(clamp_step(-2.0, 0.0, 10.0, 1.0), 0.0);
    }
}
