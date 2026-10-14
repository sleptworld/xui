//! Gregorian dates, month arithmetic, calendar grids, and date selection.
//!
//! [`CalendarMonth`] provides normalized month arithmetic and [`CalendarDate`] validates
//! dates. [`calendar`](fn@self::calendar) requests date/month updates, and
//! [`date_picker`] places that calendar in a popover with independently controlled
//! disclosure.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`calendar`](fn@self::calendar) | A month grid with selectable dates and previous/next navigation requests. |
//! | [`date_picker`](fn@self::date_picker) | A date-selection popover combining a supplied trigger and month calendar. |

use crate::layout::MergeClone;
use xui_core::prelude::*;
use xui_macros::{component, view};

/// A Gregorian year and month, with normalized construction and month offsets.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct CalendarMonth {
    pub year: i32,
    pub month: u8,
}

impl CalendarMonth {
    pub fn new(year: i32, month: u8) -> Self {
        let month = month.clamp(1, 12);
        Self { year, month }
    }

    pub fn offset(self, months: i32) -> Self {
        let absolute =
            i64::from(self.year) * 12 + i64::from(self.month.clamp(1, 12)) - 1 + i64::from(months);
        Self {
            year: absolute
                .div_euclid(12)
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            month: (absolute.rem_euclid(12) + 1) as u8,
        }
    }
}

/// A Gregorian date validated against the length of its month.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct CalendarDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl CalendarDate {
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        let month_value = CalendarMonth::new(year, month);
        (month == month_value.month && day >= 1 && day <= days_in_month(year, month))
            .then_some(Self { year, month, day })
    }
}

/// Callback requesting a selected Gregorian date.
pub type DateSelectCallback = Callback<CalendarDate>;
/// Callback requesting a displayed calendar month.
pub type MonthChangeCallback = Callback<CalendarMonth>;

pub fn is_leap_year(year: i32) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}

pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn weekday_monday_zero(year: i32, month: u8, day: u8) -> usize {
    let mut year = i64::from(year);
    let mut month = i64::from(month);
    if month < 3 {
        year -= 1;
        month += 12;
    }
    let weekday_sunday_zero = (year + year.div_euclid(4) - year.div_euclid(100)
        + year.div_euclid(400)
        + (13 * (month + 1)).div_euclid(5)
        + i64::from(day))
    .rem_euclid(7);
    ((weekday_sunday_zero + 5).rem_euclid(7)) as usize
}

/// A month grid with selectable dates and previous/next navigation requests.
///
/// `month` and `selected` are supplied by the caller. `on_month_change` and `on_select`
/// request updates; the component does not maintain its own month or selected date.
/// `disabled` blocks navigation and selection. Header, day, selected-day, and root styles
/// can be customized.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `month` | `CalendarMonth` | **Required** |
/// | `selected` | `Option<CalendarDate>` | `None` |
/// | `on_select` | `Option<DateSelectCallback>` | `None` |
/// | `on_month_change` | `Option<MonthChangeCallback>` | `None` |
/// | `disabled` | `bool` | `false` |
/// | `style` | `Style` | `Style::new()` |
/// | `header_style` | `Style` | `Style::new()` |
/// | `day_style` | `Style` | `Style::new()` |
/// | `selected_day_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { calendar(month: CalendarMonth::new(2026, 10)) };
/// ```
#[component]
#[defaults(
    selected = None,
    on_select = None,
    on_month_change = None,
    disabled = false,
    style = Style::new(),
    header_style = Style::new(),
    day_style = Style::new(),
    selected_day_style = Style::new(),
)]
pub fn calendar(
    month: &CalendarMonth,
    selected: &Option<CalendarDate>,
    on_select: &Option<DateSelectCallback>,
    on_month_change: &Option<MonthChangeCallback>,
    disabled: &bool,
    style: &Style,
    header_style: &Style,
    day_style: &Style,
    selected_day_style: &Style,
) {
    let month = &CalendarMonth::new(month.year, month.month);
    let disabled_navigation = *disabled;
    let month_names = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let previous = on_month_change.clone();
    let next = on_month_change.clone();
    let header = view! {
        container(
            style: Style::new()
                .align(AlignStyle::Center)
                .justify(JustifyStyle::SpaceBetween).merge_clone(header_style),
            flex_direction: FlexDirectionStyle::Row,
        ) {
            container(
                focusable: !*disabled,
                tab_index: if *disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::Button,
                accessibility_label: "Previous month",
                on_click: {
                    let month = *month;
                    move |_, _| {
                        if disabled_navigation { return EventResult::Ignored; }
                        if let Some(callback) = &previous {
                            callback.call(month.offset(-1));
                        }
                        EventResult::Consumed
                    }
                },
            ) {
                xui_core::widgets::text(
                    "‹",
                )
            }
            xui_core::widgets::text(
                format!(
                    "{} {}",
                    month_names[usize::from(month.month.saturating_sub(1))],
                    month.year
                ),
                style: Style::new().font_weight(FontWeight::Bold),
            )
            container(
                focusable: !*disabled,
                tab_index: if *disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::Button,
                accessibility_label: "Next month",
                on_click: {
                    let month = *month;
                    move |_, _| {
                        if disabled_navigation { return EventResult::Ignored; }
                        if let Some(callback) = &next {
                            callback.call(month.offset(1));
                        }
                        EventResult::Consumed
                    }
                },
            ) {
                xui_core::widgets::text(
                    "›",
                )
            }
        }
    };
    let mut cells = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .into_iter()
        .map(|label| {
            view! {
                xui_core::widgets::text(
                    label,
                    style: Style::new().font_size(FontSizeToken::Sm),
                )
            }
        })
        .collect::<Vec<_>>();
    let leading = weekday_monday_zero(month.year, month.month, 1);
    for slot in 0..42 {
        let day = slot - leading as i32 + 1;
        if day < 1 || day > i32::from(days_in_month(month.year, month.month)) {
            cells.push(view! {
                container()
            });
            continue;
        }
        let date = CalendarDate {
            year: month.year,
            month: month.month,
            day: day as u8,
        };
        let callback = on_select.clone();
        let is_selected = selected == &Some(date);
        let interactive = !*disabled;

        cells.push(
            view! {
                container(
                    style: Style::new()
                        .size(Size::fix(34.0, 34.0))
                        .align(AlignStyle::Center)
                        .justify(JustifyStyle::Center)
                        .border_radius(RadiusToken::Sm)
                        .background(if is_selected {
                            ColorStyle::from(ColorToken::Primary)
                        } else {
                            ColorStyle::from(Color::TRANSPARENT)
                    }).merge_clone(&crate::layout::control_style(*disabled))
                        .color(if *disabled { ColorToken::MutedForeground } else if is_selected { ColorToken::PrimaryForeground } else { ColorToken::Foreground })
                        .merge_clone(day_style).merge_clone(if is_selected { selected_day_style } else { day_style }),
                    focusable: !*disabled,
                    tab_index: if *disabled { -1 } else { 0 },
                    accessibility_role: AccessibilityRole::Button,
                    accessibility_label: format!("{}-{:02}-{:02}", date.year, date.month, date.day),
                    key: format!("calendar-{}-{}-{day}", month.year, month.month),
                    accessibility_selected: is_selected,
                    accessibility_disabled: *disabled,
                    on_click: move |_, _| { if !interactive { return EventResult::Ignored; }
                        if let Some(callback) = &callback {
                            callback.call(date);
                        }
                        EventResult::Consumed
                    },
                ) {
                    xui_core::widgets::text(
                        day.to_string(),
                    )
                }
            },
        );
    }
    let grid = view! {
        grid(
            columns: GridTracks::repeat(7, GridTrackSize::fixed(34.0)),
            gap: 4.0,
        ) {
            ..cells
        }
    };
    let mut root_style = Style::new().gap(10.0).padding(EdgeInsets::all(12.0));
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Group,
            accessibility_label: format!(
                "{} {}",
                month_names[usize::from(month.month.saturating_sub(1))],
                month.year
            ),
        ) {
            ..header
            ..grid
        }
    }
}

/// A date-selection popover combining a supplied trigger and month calendar.
///
/// `month` and `selected` are caller-controlled; date/month callbacks request updates.
/// `open` controls disclosure or `default_open` seeds internal state. Selecting a date
/// requests closure. `disabled` is forwarded to the popover and calendar, `placement`
/// controls anchoring, and `style` customizes the calendar.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `trigger` | `ElementDesc` | **Required** |
/// | `month` | `CalendarMonth` | **Required** |
/// | `selected` | `Option<CalendarDate>` | `None` |
/// | `open` | `Option<bool>` | `None` |
/// | `default_open` | `bool` | `false` |
/// | `on_open_change` | `Option<crate::BoolChangeCallback>` | `None` |
/// | `on_select` | `Option<DateSelectCallback>` | `None` |
/// | `on_month_change` | `Option<MonthChangeCallback>` | `None` |
/// | `placement` | `AnchorPlacement` | `AnchorPlacement::default()` |
/// | `style` | `Style` | `Style::new()` |
/// | `disabled` | `bool` | `false` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { date_picker(trigger: view! { text("Choose date") }, month: CalendarMonth::new(2026, 10)) };
/// ```
#[component]
#[defaults(
    selected = None,
    open = None,
    default_open = false,
    on_open_change = None,
    on_select = None,
    on_month_change = None,
    placement = AnchorPlacement::default(),
    style = Style::new(),
    disabled = false,
)]
pub fn date_picker(
    trigger: &ElementDesc,
    month: &CalendarMonth,
    selected: &Option<CalendarDate>,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<crate::BoolChangeCallback>,
    on_select: &Option<DateSelectCallback>,
    on_month_change: &Option<MonthChangeCallback>,
    placement: &AnchorPlacement,
    style: &Style,
    disabled: &bool,
) {
    let internal_open = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*internal_open.get());
    let selected_callback = on_select.clone();
    let open_callback = on_open_change.clone();
    let select = cx.use_callback(
        (
            controlled,
            internal_open,
            on_select.clone(),
            on_open_change.clone(),
        ),
        move |date| {
            if let Some(callback) = &selected_callback {
                callback.call(date);
            }
            if !controlled {
                internal_open.set(false);
            }
            if let Some(callback) = &open_callback {
                callback.call(false);
            }
        },
    );
    let content = view! {
        calendar(
            month: *month,
            selected: *selected,
            on_select: Some(select),
            on_month_change: on_month_change.clone(),
            style: style.clone(),
            disabled: *disabled,
        )
    };
    let changed = cx.use_callback((controlled, internal_open, on_open_change.clone()), {
        let callback = on_open_change.clone();
        move |next| {
            if !controlled {
                internal_open.set(next);
            }
            if let Some(callback) = &callback {
                callback.call(next);
            }
        }
    });
    view! {
        crate::popover(
            trigger: trigger.clone(),
            content: content,
            open: Some(shown),
            on_open_change: Some(changed),
            placement: *placement,
            disabled: *disabled,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_math_handles_leap_years_and_negative_offsets() {
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(
            CalendarMonth::new(2026, 1).offset(-1),
            CalendarMonth::new(2025, 12)
        );
        assert_eq!(weekday_monday_zero(2026, 9, 7), 0);
    }

    #[test]
    fn invalid_dates_are_rejected() {
        assert!(CalendarDate::new(2025, 2, 29).is_none());
        assert!(CalendarDate::new(2024, 2, 29).is_some());
    }
}
