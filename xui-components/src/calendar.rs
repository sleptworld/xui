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

use xui_core::prelude::*;
use xui_macros::component;

use crate::controls::commit;

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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `header_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `day_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `selected_day_style` | `StyleOf<Container>` | `StyleOf::new()` |
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
///     view! { calendar(month: CalendarMonth::new(2026, 10)) }
/// }
/// ```
#[component]
#[defaults(
    selected = None::<CalendarDate>,
    disabled = false,
    style = StyleOf::new(),
    header_style = StyleOf::new(),
    day_style = StyleOf::new(),
    selected_day_style = StyleOf::new(),
)]
pub fn calendar(
    #[source] month: &CalendarMonth,
    #[source] selected: &Option<CalendarDate>,
    on_select: &Callback<CalendarDate>,
    on_month_change: &Callback<CalendarMonth>,
    #[source] disabled: &bool,
    style: &StyleOf<Container>,
    header_style: &StyleOf<Container>,
    day_style: &StyleOf<Container>,
    selected_day_style: &StyleOf<Container>,
) {
    let shown: CalendarMonth = memo!(CalendarMonth::new(month.year, month.month));
    let title: String = memo!(format!("{} {}", MONTH_NAMES[usize::from(shown.month.saturating_sub(1))], shown.year));
    // The six weeks the grid shows, Monday first: a date, or a blank.
    let days: Vec<Option<CalendarDate>> = memo!({
        let leading = weekday_monday_zero(shown.year, shown.month, 1) as i32;
        let count = i32::from(days_in_month(shown.year, shown.month));
        (0..42)
            .map(|slot| {
                let day = slot - leading + 1;
                (1..=count).contains(&day).then_some(CalendarDate { year: shown.year, month: shown.month, day: day as u8 })
            })
            .collect()
    });
    let mut root_style: StyleOf<Container> = StyleOf::new()
        .with::<container::gap>(10.0)
        .with::<container::padding>(EdgeInsets::all(12.0));
    root_style.merge(style);
    view! {
        container(
            style: root_style.clone(),
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Group,
            accessibility_label: title.clone(),
        ) {
            container(
                style: StyleOf::new()
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween)
                    .merged(header_style),
                flex_direction: FlexDirectionStyle::Row,
            ) {
                container(
                    focusable: !*disabled,
                    tab_index: if *disabled { -1 } else { 0 },
                    accessibility_role: AccessibilityRole::Button,
                    accessibility_label: "Previous month",
                    on_click: |cx| {
                        if disabled.get() {
                            return EventResult::Ignored;
                        }
                        on_month_change.call(cx, shown.get().offset(-1));
                        EventResult::Consumed
                    },
                ) {
                    text("‹")
                }
                text(title.clone(), style: StyleOf::new().with::<container::font_weight>(FontWeight::Bold))
                container(
                    focusable: !*disabled,
                    tab_index: if *disabled { -1 } else { 0 },
                    accessibility_role: AccessibilityRole::Button,
                    accessibility_label: "Next month",
                    on_click: |cx| {
                        if disabled.get() {
                            return EventResult::Ignored;
                        }
                        on_month_change.call(cx, shown.get().offset(1));
                        EventResult::Consumed
                    },
                ) {
                    text("›")
                }
            }
            grid(
                columns: GridTracks::repeat(7, GridTrackSize::fixed(34.0)),
                gap: 4.0,
            ) {
                for label in WEEKDAYS {
                    text(*label, style: StyleOf::new().with::<container::font_size>(FontSizeToken::Sm))
                }
                for day in days.clone() {
                    if let Some(date) = *day {
                        container(
                            style: StyleOf::new()
                                .with::<container::size>(Size::fix(34.0, 34.0))
                                .with::<container::align>(AlignStyle::Center)
                                .with::<container::justify>(JustifyStyle::Center)
                                .with::<container::border_radius>(RadiusToken::Sm)
                                .with::<container::background>(if *selected == Some(*date) {
                                    ColorStyle::from(ColorToken::Primary)
                                } else {
                                    ColorStyle::from(Color::TRANSPARENT)
                                })
                                .merged(&crate::layout::control_style(*disabled))
                                .with::<container::color>(if *disabled {
                                    ColorToken::MutedForeground
                                } else if *selected == Some(*date) {
                                    ColorToken::PrimaryForeground
                                } else {
                                    ColorToken::Foreground
                                })
                                .merged(day_style)
                                .merged(if *selected == Some(*date) { selected_day_style } else { day_style }),
                            focusable: !*disabled,
                            tab_index: if *disabled { -1 } else { 0 },
                            accessibility_role: AccessibilityRole::Button,
                            accessibility_label: format!("{}-{:02}-{:02}", date.year, date.month, date.day),
                            accessibility_selected: *selected == Some(*date),
                            accessibility_disabled: *disabled,
                            on_click: |cx| {
                                if disabled.get() {
                                    return EventResult::Ignored;
                                }
                                on_select.call(cx, *date);
                                EventResult::Consumed
                            },
                        ) {
                            text(date.day.to_string())
                        }
                    } else {
                        container()
                    }
                }
            }
        }
    }
}

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `disabled` | `bool` | `false` |
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
///     view! { date_picker(trigger: view! { text("Choose date") }, month: CalendarMonth::new(2026, 10)) }
/// }
/// ```
#[component]
#[defaults(
    selected = None::<CalendarDate>,
    open = None::<bool>,
    default_open = false,
    placement = AnchorPlacement::default(),
    style = StyleOf::new(),
    disabled = false,
)]
pub fn date_picker(
    trigger: &Children,
    #[source] month: &CalendarMonth,
    #[source] selected: &Option<CalendarDate>,
    #[source] open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Callback<bool>,
    on_select: &Callback<CalendarDate>,
    on_month_change: &Callback<CalendarMonth>,
    placement: &AnchorPlacement,
    style: &StyleOf<Container>,
    #[source] disabled: &bool,
) {
    let own: bool = state!(*default_open);
    let shown: bool = memo!(open.unwrap_or(*own));
    view! {
        container() {
            crate::popover(
                trigger: trigger,
                open: Some(*shown),
                on_open_change: |cx, next| commit(cx, next, open, own, on_open_change),
                placement: *placement,
                disabled: *disabled,
            ) {
                calendar(
                    month: *month,
                    selected: *selected,
                    on_select: |cx, date| {
                        on_select.call(cx, date);
                        commit(cx, false, open, own, on_open_change);
                    },
                    on_month_change: on_month_change,
                    style: style.clone(),
                    disabled: *disabled,
                )
            }
        }
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
