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

use xui_core::prelude::*;

use xui_macros::{component, style};

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

/// Commits a new value: kept in `own` unless `controlled` holds one, and reported.
pub(crate) fn commit<T: Clone + 'static>(
    cx: &mut Cx<'_>,
    value: T,
    controlled: &ReadSignal<Option<T>>,
    own: &Signal<T>,
    callback: &Callback<T>,
) {
    if controlled.with(Option::is_none) {
        own.set(cx, value.clone());
    }
    callback.call(cx, value);
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `indicator_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { checkbox(default_checked: true, label: view! { text("Remember me") }) }
/// }
/// ```
#[component]
#[defaults(
    checked = None::<bool>,
    default_checked = false,
    indeterminate = false,
    disabled = false,
    accessibility_label = None::<String>,
    style = StyleOf::new(),
    indicator_style = StyleOf::new(),
)]
pub fn checkbox(
    #[source] checked: &Option<bool>,
    default_checked: &bool,
    #[source] indeterminate: &bool,
    #[source] disabled: &bool,
    label: &Children,
    on_change: &Callback<bool>,
    #[source] accessibility_label: &Option<String>,
    style: &StyleOf<Container>,
    indicator_style: &StyleOf<Container>,
) {
    let own: bool = state!(*default_checked);
    let value: bool = memo!(checked.unwrap_or(*own));
    let box_style: StyleOf<Container> = memo!(
        style!(container {
            size: Size::fix(20.0, 20.0),
            border_width: 1.0,
            border_color: if *value || *indeterminate {
                ColorToken::Primary
            } else {
                ColorToken::Border
            },
            background: if *value || *indeterminate {
                ColorStyle::from(ColorToken::Primary)
            } else {
                ColorStyle::from(Color::TRANSPARENT)
            },
            border_radius: RadiusToken::Sm,
        })
        .merged(indicator_style)
        .with::<container::color>(ColorToken::PrimaryForeground)
    );
    let root_style: StyleOf<Container> = memo!(
        crate::layout::control_style(*disabled)
            .with::<container::align>(AlignStyle::Center)
            .with::<container::gap>(8.0)
            .with::<container::min_height>(32.0)
            .merged(style)
    );
    view! {
        row(
            style: root_style.clone(),
            focusable: !*disabled,
            tab_index: if *disabled { -1 } else { 0 },
            accessibility_role: AccessibilityRole::Checkbox,
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
            accessibility_checked: if *indeterminate {
                AccessibilityChecked::Mixed
            } else if *value {
                AccessibilityChecked::True
            } else {
                AccessibilityChecked::False
            },
            accessibility_disabled: *disabled,
            on_click: |cx, event, event_cx| {
                if disabled.get() || event.button.is_some_and(|button| button != PointerButton::Primary) {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                commit(cx, !value.get(), checked, own, on_change);
                EventResult::Consumed
            },
        ) {
            center(style: box_style.clone()) {
                xui_core::prelude::text(if *indeterminate { "−" } else if *value { "✓" } else { "" })
            }
            slot(label)
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `track_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `thumb_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { switch(default_checked: true, label: view! { text("Notifications") }) }
/// }
/// ```
#[component]
#[defaults(
    checked = None::<bool>,
    default_checked = false,
    disabled = false,
    accessibility_label = None::<String>,
    style = StyleOf::new(),
    track_style = StyleOf::new(),
    thumb_style = StyleOf::new(),
)]
pub fn switch(
    #[source] checked: &Option<bool>,
    default_checked: &bool,
    #[source] disabled: &bool,
    label: &Children,
    on_change: &Callback<bool>,
    #[source] accessibility_label: &Option<String>,
    style: &StyleOf<Container>,
    track_style: &StyleOf<Container>,
    thumb_style: &StyleOf<Container>,
) {
    let own: bool = state!(*default_checked);
    let value: bool = memo!(checked.unwrap_or(*own));
    let motion: Transition = Transition::new(Duration::from_millis(180)).ease(Easing::CubicOut);
    let resolved_track: StyleOf<Container> = memo!(
        style!(container {
            size: Size::fix(SWITCH_TRACK_WIDTH, SWITCH_TRACK_HEIGHT),
            padding: EdgeInsets::all(SWITCH_TRACK_PADDING),
            border_radius: SWITCH_TRACK_HEIGHT / 2.0,
            align: AlignStyle::Center,
            justify: JustifyStyle::Start,
            background: if *value {
                ColorStyle::from(ColorToken::Primary)
            } else {
                ColorStyle::from(ColorToken::MutedSurface)
            },
        })
        .merged(track_style)
    );
    let resolved_thumb: StyleOf<Container> = memo!(
        style!(container {
            size: Size::fix(SWITCH_THUMB_SIZE, SWITCH_THUMB_SIZE),
            border_radius: SWITCH_THUMB_SIZE / 2.0,
            background: ColorToken::Background,
            translate_x: if *value { SWITCH_TRAVEL } else { 0.0 },
        })
        .merged(thumb_style)
    );
    // `center` fills its parent by default; a switch is sized by its track and
    // label.
    let root_style: StyleOf<Container> = memo!(
        crate::layout::control_style(*disabled)
            .with::<container::size>(Size::new(Sizing::Hug, Sizing::Hug))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::gap>(8.0)
            .with::<container::min_height>(32.0)
            .merged(style)
    );
    view! {
        center(
            style: root_style.clone(),
            flex_direction: FlexDirectionStyle::Row,
            focusable: !*disabled,
            tab_index: if *disabled { -1 } else { 0 },
            accessibility_role: AccessibilityRole::Switch,
            accessibility_checked: if *value { AccessibilityChecked::True } else { AccessibilityChecked::False },
            accessibility_disabled: *disabled,
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
            on_click: |cx, event, event_cx| {
                if disabled.get() || event.button.is_some_and(|button| button != PointerButton::Primary) {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                commit(cx, !value.get(), checked, own, on_change);
                EventResult::Consumed
            },
        ) {
            container(style: resolved_track.clone(), transition: *motion) {
                container(style: resolved_thumb.clone(), transition: *motion)
            }
            slot(label)
        }
    }
}

/// An identified radio choice with a visual label, accessibility name, and disabled
/// state.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioItem {
    pub id: String,
    pub label: Children,
    pub accessibility_label: String,
    pub disabled: bool,
}

impl RadioItem {
    pub fn new(
        id: impl Into<String>,
        accessibility_label: impl Into<String>,
        label: impl Into<Children>,
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `item_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `indicator_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { radio_group(items: vec![RadioItem::new("a", "Choice A", "Choice A")]) }
/// }
/// ```
#[component]
#[defaults(
    selected = None::<usize>,
    default_selected = 0,
    disabled = false,
    orientation = SelectionOrientation::Vertical,
    accessibility_label = None::<String>,
    style = StyleOf::new(),
    item_style = StyleOf::new(),
    indicator_style = StyleOf::new(),
)]
pub fn radio_group(
    #[source] items: &Vec<RadioItem>,
    #[source] selected: &Option<usize>,
    default_selected: &usize,
    #[source] disabled: &bool,
    orientation: &SelectionOrientation,
    on_change: &Callback<usize>,
    #[source] accessibility_label: &Option<String>,
    style: &StyleOf<Container>,
    item_style: &StyleOf<Container>,
    indicator_style: &StyleOf<Container>,
) {
    let model: SelectionModel = memo!(SelectionModel::new(items.iter().map(|item| {
        SelectionItem::new(item.id.clone(), item.accessibility_label.clone(), *disabled || item.disabled)
    })));
    let own: usize = state!(*default_selected);
    let active: Option<usize> = memo!(model.normalize(selected.unwrap_or(*own)));
    let handles: Vec<FocusHandle> = memo!((0..items.len()).map(|_| FocusHandle::new()).collect());
    let shortcuts: Vec<ShortcutBinding> = [
        (Shortcut::named(if *orientation == SelectionOrientation::Horizontal { NamedKey::ArrowLeft } else { NamedKey::ArrowUp }).allow_repeat(true), PREVIOUS),
        (Shortcut::named(if *orientation == SelectionOrientation::Horizontal { NamedKey::ArrowRight } else { NamedKey::ArrowDown }).allow_repeat(true), NEXT),
        (Shortcut::named(NamedKey::Home), FIRST),
        (Shortcut::named(NamedKey::End), LAST),
    ]
    .map(|(shortcut, command)| ShortcutBinding { shortcut, command })
    .to_vec();
    view! {
        container(
            style: StyleOf::new().with::<container::gap>(4.0).merged(style),
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
            for (index, item) in items.iter().cloned().enumerate() {
                container(
                    style: crate::layout::control_style(*disabled || item.disabled)
                        .with::<container::align>(AlignStyle::Center)
                        .with::<container::gap>(8.0)
                        .with::<container::min_height>(32.0)
                        .merged(item_style),
                    flex_direction: FlexDirectionStyle::Row,
                    focusable: !(*disabled || item.disabled),
                    tab_index: if *active == Some(*index) { 0 } else { -1 },
                    focus_handle: handles[*index].clone(),
                    accessibility_role: AccessibilityRole::Radio,
                    accessibility_label: item.accessibility_label.clone(),
                    accessibility_checked: if *active == Some(*index) {
                        AccessibilityChecked::True
                    } else {
                        AccessibilityChecked::False
                    },
                    accessibility_disabled: *disabled || item.disabled,
                    on_click: |cx, _event, event_cx| {
                        if disabled.get() || item.disabled {
                            return EventResult::Ignored;
                        }
                        event_cx.request_focus();
                        commit(cx, *index, selected, own, on_change);
                        EventResult::Consumed
                    },
                    shortcuts: shortcuts.clone(),
                    on_command: |cx, event, event_cx| {
                        if disabled.get() || item.disabled {
                            return EventResult::Ignored;
                        }
                        let model = model.get();
                        let next = match event.command {
                            FIRST => model.first(),
                            LAST => model.last(),
                            PREVIOUS => model.adjacent(*index, -1),
                            NEXT => model.adjacent(*index, 1),
                            _ => return EventResult::Ignored,
                        };
                        let Some(next) = next else { return EventResult::Ignored };
                        handles.read()[next].request_focus(event_cx);
                        commit(cx, next, selected, own, on_change);
                        EventResult::Consumed
                    },
                ) {
                    container(
                        style: StyleOf::new()
                            .with::<container::size>(Size::fix(18.0, 18.0))
                            .with::<container::border_width>(if *active == Some(*index) { 5.0 } else { 1.0 })
                            .with::<container::border_color>(if *active == Some(*index) {
                                ColorToken::Primary
                            } else {
                                ColorToken::Border
                            })
                            .with::<container::border_radius>(9.0)
                            .merged(indicator_style),
                    )
                    slot(item.label)
                }
                .key(item.id.clone())
            }
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `track_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `fill_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `thumb_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { slider(min: 0.0, max: 10.0, step: 0.5, default_value: 4.0) }
/// }
/// ```
#[component]
#[defaults(
    value = None::<f32>,
    default_value = 0.0,
    min = 0.0,
    max = 100.0,
    step = 1.0,
    disabled = false,
    accessibility_label = None::<String>,
    style = StyleOf::new(),
    track_style = StyleOf::new(),
    fill_style = StyleOf::new(),
    thumb_style = StyleOf::new(),
)]
pub fn slider(
    #[source] value: &Option<f32>,
    default_value: &f32,
    #[source] min: &f32,
    #[source] max: &f32,
    #[source] step: &f32,
    #[source] disabled: &bool,
    on_change: &Callback<f32>,
    #[source] accessibility_label: &Option<String>,
    style: &StyleOf<Container>,
    track_style: &StyleOf<Container>,
    fill_style: &StyleOf<Container>,
    thumb_style: &StyleOf<Container>,
) {
    let bounds: (f32, f32, f32) = memo!(slider_bounds(*min, *max, *step));
    let own: f32 = state!({
        let (min, max, step) = slider_bounds(min.get(), max.get(), step.get());
        clamp_step(*default_value, min, max, step)
    });
    let current: f32 = memo!(clamp_step(value.unwrap_or(*own), bounds.0, bounds.1, bounds.2));
    let ratio: f32 = memo!({
        let span = f64::from(bounds.1) - f64::from(bounds.0);
        if span <= f64::from(f32::EPSILON) {
            0.0
        } else {
            ((f64::from(*current) - f64::from(bounds.0)) / span).clamp(0.0, 1.0) as f32
        }
    });
    let size: Option<Size<f32>> = size!();
    view! {
        container(
            style: crate::layout::control_style(*disabled)
                .with::<container::position>(PositionStyle::Relative)
                .with::<container::height>(32.0)
                .with::<container::justify>(JustifyStyle::Center)
                .merged(style),
            size_observer: size,
            focusable: !*disabled,
            tab_index: if *disabled { -1 } else { 0 },
            accessibility_role: AccessibilityRole::Slider,
            accessibility_numeric_value: *current as f64,
            accessibility_min_value: bounds.0 as f64,
            accessibility_max_value: bounds.1 as f64,
            accessibility_step_value: bounds.2 as f64,
            accessibility_disabled: *disabled,
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
            on_click: |cx, event, event_cx| {
                if disabled.get() || event.button.is_some_and(|button| button != PointerButton::Primary) {
                    return EventResult::Ignored;
                }
                let Some(pointer) = event.pointer.as_ref() else {
                    return EventResult::Ignored;
                };
                let (min, max, step) = bounds.get();
                let span = f64::from(max) - f64::from(min);
                let width = event_cx.node_ref.layout.width().max(1.0);
                let local = pointer.coords.window.x - event_cx.node_ref.world_origin.x;
                let next = clamp_step(
                    (f64::from(min) + f64::from((local / width).clamp(0.0, 1.0)) * span) as f32,
                    min,
                    max,
                    step,
                );
                commit(cx, next, value, own, on_change);
                event_cx.request_focus();
                EventResult::Consumed
            },
            shortcuts: navigation_shortcuts().map(|(shortcut, command)| ShortcutBinding { shortcut, command }).to_vec(),
            on_command: |cx, event| {
                if disabled.get() {
                    return EventResult::Ignored;
                }
                let ((min, max, step), current) = (bounds.get(), current.get());
                let next = match event.command {
                    PREVIOUS => current - step,
                    NEXT => current + step,
                    FIRST => min,
                    LAST => max,
                    _ => return EventResult::Ignored,
                };
                commit(cx, clamp_step(next, min, max, step), value, own, on_change);
                EventResult::Consumed
            },
        ) {
            container(
                style: StyleOf::new()
                    .with::<container::position>(PositionStyle::Relative)
                    .with::<container::width>(Sizing::Fill)
                    .with::<container::height>(6.0)
                    .with::<container::background>(ColorToken::MutedSurface)
                    .with::<container::border_radius>(3.0)
                    .merged(track_style),
            ) {
                container(
                    style: StyleOf::new()
                        .with::<container::width>(Sizing::percent(*ratio))
                        .with::<container::height>(6.0)
                        .with::<container::background>(ColorToken::Primary)
                        .with::<container::border_radius>(3.0)
                        .merged(fill_style),
                )
            }
            container(
                style: StyleOf::new()
                    .with::<container::position>(PositionStyle::Absolute)
                    .with::<container::inset>(EdgeInsets::new(0.0, 0.0, 8.0, 0.0))
                    .with::<container::size>(Size::fix(16.0, 16.0))
                    .with::<container::translate_x>(*ratio * (size.map_or(16.0, |size| size.width) - 16.0).max(0.0))
                    .with::<container::background>(ColorToken::Background)
                    .with::<container::border_color>(ColorToken::Primary)
                    .with::<container::border_width>(2.0)
                    .with::<container::border_radius>(8.0)
                    .merged(thumb_style),
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `indicator_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { progress(value: 40.0, max: 100.0) }
/// }
/// ```
#[component]
#[defaults(
    value = 0.0,
    max = 100.0,
    accessibility_label = None::<String>,
    style = StyleOf::new(),
    indicator_style = StyleOf::new(),
)]
pub fn progress(
    #[source] value: &f32,
    #[source] max: &f32,
    #[source] accessibility_label: &Option<String>,
    style: &StyleOf<Container>,
    indicator_style: &StyleOf<Container>,
) {
    let limit: f32 = memo!(if max.is_finite() && *max > 0.0 { *max } else { 1.0 });
    let amount: f32 = memo!(if value.is_finite() { value.clamp(0.0, *limit) } else { 0.0 });
    view! {
        container(
            style: StyleOf::new()
                .with::<container::width>(Sizing::Fill)
                .with::<container::height>(8.0)
                .with::<container::background>(ColorToken::MutedSurface)
                .with::<container::border_radius>(4.0)
                .merged(style),
            accessibility_role: AccessibilityRole::ProgressIndicator,
            accessibility_numeric_value: *amount as f64,
            accessibility_min_value: 0.0,
            accessibility_max_value: *limit as f64,
            accessibility_label: accessibility_label.clone().unwrap_or_default(),
        ) {
            container(
                style: StyleOf::new()
                    .with::<container::width>(Sizing::percent(*amount / *limit))
                    .with::<container::height>(Sizing::Fill)
                    .with::<container::background>(ColorToken::Primary)
                    .with::<container::border_radius>(4.0)
                    .merged(indicator_style),
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { separator(orientation: SeparatorOrientation::Horizontal) }
/// }
/// ```
#[component]
#[defaults(
    orientation = SeparatorOrientation::Horizontal,
    style = StyleOf::new(),
)]
pub fn separator(#[source] orientation: &SeparatorOrientation, style: &StyleOf<Container>) {
    view! {
        container(
            style: match orientation {
                SeparatorOrientation::Horizontal => StyleOf::new()
                    .with::<container::width>(Sizing::Fill)
                    .with::<container::height>(1.0),
                SeparatorOrientation::Vertical => StyleOf::new()
                    .with::<container::width>(1.0)
                    .with::<container::height>(Sizing::Fill),
            }
            .with::<container::background>(ColorToken::Border)
            .merged(style),
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `variant` | `BadgeVariant` | `BadgeVariant::Secondary` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { badge(text: "New", variant: BadgeVariant::Primary) }
/// }
/// ```
#[component]
#[defaults(
    style = StyleOf::new(),
    variant = BadgeVariant::Secondary,
)]
pub fn badge(#[source] text: &String, style: &StyleOf<Container>, #[source] variant: &BadgeVariant) {
    view! {
        container(
            style: match variant {
                BadgeVariant::Primary => StyleOf::new()
                    .with::<container::background>(ColorToken::Primary)
                    .with::<container::color>(ColorToken::PrimaryForeground),
                BadgeVariant::Secondary => StyleOf::new()
                    .with::<container::background>(ColorToken::Secondary)
                    .with::<container::color>(ColorToken::SecondaryForeground),
                BadgeVariant::Outline => StyleOf::new()
                    .with::<container::background>(ColorToken::Background)
                    .with::<container::color>(ColorToken::Foreground)
                    .with::<container::border_width>(1.0)
                    .with::<container::border_color>(ColorToken::Border),
                BadgeVariant::Danger => StyleOf::new()
                    .with::<container::background>(ColorToken::Destructive)
                    .with::<container::color>(ColorToken::DestructiveForeground),
            }
            .with::<container::padding>(EdgeInsets::symmetric(8.0, 3.0))
            .with::<container::border_radius>(999.0)
            .with::<container::font_size>(FontSizeToken::Sm)
            .merged(style),
        ) {
            xui_core::prelude::text(text.clone())
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `fallback_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { avatar(accessibility_label: "Alex", fallback: "AL") }
/// }
/// ```
#[component]
#[defaults(
    size = 40.0,
    fallback = "?".to_string(),
    style = StyleOf::new(),
    fallback_style = StyleOf::new(),
)]
pub fn avatar(
    #[source] accessibility_label: &String,
    #[source] size: &f32,
    image: &Children,
    #[source] fallback: &String,
    style: &StyleOf<Container>,
    fallback_style: &StyleOf<Text>,
) {
    let side: f32 = memo!(if size.is_finite() { size.max(0.0) } else { 40.0 });
    view! {
        container(
            style: StyleOf::new()
                .with::<container::size>(Size::fix(*side, *side))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::Center)
                .with::<container::clip>(true)
                .with::<container::background>(ColorToken::MutedSurface)
                .with::<container::border_radius>(*side / 2.0)
                .merged(style),
            accessibility_role: AccessibilityRole::Image,
            accessibility_label: accessibility_label.clone(),
        ) {
            if image.is_empty() {
                text(fallback.clone(), style: fallback_style.clone())
            } else {
                slot(image)
            }
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { card { text("Card content") } }
/// }
/// ```
#[component]
#[defaults(
    style = StyleOf::new(),
)]
pub fn card(children: &Children, style: &StyleOf<Container>) {
    view! {
        container(
            style: StyleOf::new()
                .with::<container::padding>(EdgeInsets::all(16.0))
                .with::<container::background>(ColorToken::Surface)
                .with::<container::border_width>(1.0)
                .with::<container::border_color>(ColorToken::Border)
                .with::<container::border_radius>(RadiusToken::Lg)
                .merged(style),
        ) {
            slot(children)
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
