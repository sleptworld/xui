use xui::prelude::*;

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
        let absolute = self.year * 12 + i32::from(self.month) - 1 + months;
        Self {
            year: absolute.div_euclid(12),
            month: (absolute.rem_euclid(12) + 1) as u8,
        }
    }
}

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

pub type DateSelectCallback = Callback<CalendarDate>;
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
    let mut year = year;
    let mut month = i32::from(month);
    if month < 3 {
        year -= 1;
        month += 12;
    }
    let weekday_sunday_zero = (year + year.div_euclid(4) - year.div_euclid(100)
        + year.div_euclid(400)
        + (13 * (month + 1)).div_euclid(5)
        + i32::from(day))
    .rem_euclid(7);
    ((weekday_sunday_zero + 5).rem_euclid(7)) as usize
}

#[component]
#[defaults(
    selected = None,
    on_select = None,
    on_month_change = None,
    disabled = false,
    style = Style::new(),
)]
pub fn calendar(
    month: &CalendarMonth,
    selected: &Option<CalendarDate>,
    on_select: &Option<DateSelectCallback>,
    on_month_change: &Option<MonthChangeCallback>,
    disabled: &bool,
    style: &Style,
) {
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
    let header = ContainerWidget::new()
        .style(
            Style::new()
                .align(AlignStyle::Center)
                .justify(JustifyStyle::SpaceBetween),
        )
        .flex_direction(FlexDirectionStyle::Row)
        .into_element_desc(vec![
            ContainerWidget::new()
                .focusable(!*disabled)
                .tab_index(if *disabled { -1 } else { 0 })
                .accessibility_role(AccessibilityRole::Button)
                .accessibility_label("Previous month")
                .on_click({
                    let month = *month;
                    move |_, _| {
                        if let Some(callback) = &previous {
                            callback.call(month.offset(-1));
                        }
                        EventResult::Consumed
                    }
                })
                .into_element_desc(vec![TextWidget::new("‹").into_element_desc()]),
            TextWidget::new(format!(
                "{} {}",
                month_names[usize::from(month.month.saturating_sub(1))],
                month.year
            ))
            .style(Style::new().font_weight(FontWeight::Bold))
            .into_element_desc(),
            ContainerWidget::new()
                .focusable(!*disabled)
                .tab_index(if *disabled { -1 } else { 0 })
                .accessibility_role(AccessibilityRole::Button)
                .accessibility_label("Next month")
                .on_click({
                    let month = *month;
                    move |_, _| {
                        if let Some(callback) = &next {
                            callback.call(month.offset(1));
                        }
                        EventResult::Consumed
                    }
                })
                .into_element_desc(vec![TextWidget::new("›").into_element_desc()]),
        ]);
    let mut cells = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .into_iter()
        .map(|label| {
            TextWidget::new(label)
                .style(Style::new().font_size(FontSizeToken::Sm))
                .into_element_desc()
        })
        .collect::<Vec<_>>();
    let leading = weekday_monday_zero(month.year, month.month, 1);
    for slot in 0..42 {
        let day = slot - leading as i32 + 1;
        if day < 1 || day > i32::from(days_in_month(month.year, month.month)) {
            cells.push(ContainerWidget::new().into_element_desc(vec![]));
            continue;
        }
        let date = CalendarDate {
            year: month.year,
            month: month.month,
            day: day as u8,
        };
        let callback = on_select.clone();
        let is_selected = selected == &Some(date);
        let mut cell = ContainerWidget::new()
            .style(
                Style::new()
                    .size(Size::fix(34.0, 34.0))
                    .align(AlignStyle::Center)
                    .justify(JustifyStyle::Center)
                    .border_radius(RadiusToken::Sm)
                    .background(if is_selected {
                        ColorStyle::from(ColorToken::Primary)
                    } else {
                        ColorStyle::from(Color::TRANSPARENT)
                    }),
            )
            .focusable(!*disabled)
            .tab_index(if *disabled { -1 } else { 0 })
            .accessibility_role(AccessibilityRole::Button)
            .accessibility_label(format!("{}-{:02}-{:02}", date.year, date.month, date.day))
            .accessibility_selected(is_selected)
            .accessibility_disabled(*disabled);
        if !*disabled {
            cell = cell.on_click(move |_, _| {
                if let Some(callback) = &callback {
                    callback.call(date);
                }
                EventResult::Consumed
            });
        }
        cells.push(
            cell.into_element_desc(vec![TextWidget::new(day.to_string()).into_element_desc()]),
        );
    }
    let grid = GridWidget::new()
        .columns(GridTracks::repeat(7, GridTrackSize::fixed(34.0)))
        .gap(4.0)
        .into_element_desc(cells);
    let mut root_style = Style::new().gap(10.0).padding(EdgeInsets::all(12.0));
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::Group)
        .accessibility_label(format!(
            "{} {}",
            month_names[usize::from(month.month.saturating_sub(1))],
            month.year
        ))
        .into_element_desc(vec![header, grid])
}

#[component]
#[defaults(
    selected = None,
    open = None,
    default_open = false,
    on_open_change = None,
    on_select = None,
    on_month_change = None,
    placement = OverlayPlacement::BottomStart,
    style = Style::new(),
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
    placement: &OverlayPlacement,
    style: &Style,
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
    let content = xui! {
        <calendar
            month={*month}
            selected={*selected}
            on_select={Some(select)}
            on_month_change={on_month_change.clone()}
            style={style.clone()}
        />
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
    xui! {
        <crate::popover
            trigger={trigger.clone()}
            content={content}
            open={Some(shown)}
            on_open_change={Some(changed)}
            placement={*placement}
        />
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
