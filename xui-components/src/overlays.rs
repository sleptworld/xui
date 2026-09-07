use xui::prelude::*;

use crate::BoolChangeCallback;

fn notify_open(
    next: bool,
    controlled: bool,
    state: xui::state::State<bool>,
    callback: &Option<BoolChangeCallback>,
) {
    if !controlled {
        state.set(next);
    }
    if let Some(callback) = callback {
        callback.call(next);
    }
}

fn overlay_panel_style() -> Style {
    Style::new()
        .padding(EdgeInsets::all(16.0))
        .background(ColorToken::Surface)
        .border_width(1.0)
        .border_color(ColorToken::Border)
        .border_radius(RadiusToken::Lg)
}

#[component]
#[defaults(
    open = None,
    default_open = false,
    title = None,
    on_open_change = None,
    close_on_escape = true,
    close_on_pointer_outside = true,
    z_index = 2000,
    style = Style::new(),
)]
pub fn dialog(
    content: &ElementDesc,
    open: &Option<bool>,
    default_open: &bool,
    title: &Option<String>,
    on_open_change: &Option<BoolChangeCallback>,
    close_on_escape: &bool,
    close_on_pointer_outside: &bool,
    z_index: &i32,
    style: &Style,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    if !shown {
        return ContainerWidget::new()
            .accessibility_hidden(true)
            .into_element_desc(vec![]);
    }
    let callback = on_open_change.clone();
    let mut panel_style = overlay_panel_style()
        .min_width(320.0)
        .max_width(640.0)
        .max_height(Sizing::percent(0.9));
    panel_style.merge(style);
    let mut panel_children = Vec::with_capacity(2);
    if let Some(title) = title {
        panel_children.push(
            TextWidget::new(title.clone())
                .style(
                    Style::new()
                        .font_size(FontSizeToken::Lg)
                        .font_weight(FontWeight::Bold),
                )
                .into_element_desc(),
        );
    }
    panel_children.push(content.clone());
    let panel = ContainerWidget::new()
        .style(panel_style)
        .flex_direction(FlexDirectionStyle::Column)
        .gap(12.0)
        .focusable(true)
        .tab_index(0)
        .accessibility_role(AccessibilityRole::Dialog)
        .on_click(|_, _| EventResult::Consumed)
        .into_element_desc(panel_children);
    let root = ContainerWidget::new()
        .style(
            Style::new()
                .absolute()
                .inset(EdgeInsets::zero())
                .size(Size::fill())
                .align(AlignStyle::Center)
                .justify(JustifyStyle::Center)
                .background(Color::rgba(0.0, 0.0, 0.0, 0.46)),
        )
        .on_dismiss(move |_, _| {
            notify_open(false, controlled, state, &callback);
            EventResult::Consumed
        })
        .into_element_desc(vec![panel]);
    portal(vec![root])
        .z_index(*z_index)
        .modal(true)
        .dismiss_on_escape(*close_on_escape)
        .dismiss_on_pointer_outside(*close_on_pointer_outside)
        .into()
}

#[component]
#[defaults(
    open = None,
    default_open = false,
    on_open_change = None,
    placement = OverlayPlacement::BottomStart,
    modal = false,
    z_index = 1500,
    style = Style::new(),
)]
pub fn popover(
    trigger: &ElementDesc,
    content: &ElementDesc,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    placement: &OverlayPlacement,
    modal: &bool,
    z_index: &i32,
    style: &Style,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    let anchor = cx.use_memo(AnchorHandle::new);
    let callback = on_open_change.clone();
    let trigger_root = ContainerWidget::new()
        .anchor_handle(anchor.get().clone())
        .focusable(true)
        .tab_index(0)
        .on_click(move |_, event_cx| {
            event_cx.request_focus();
            notify_open(!shown, controlled, state, &callback);
            EventResult::Consumed
        })
        .into_element_desc(vec![trigger.clone()]);
    let mut children = vec![trigger_root];
    if shown {
        let callback = on_open_change.clone();
        let mut panel_style = overlay_panel_style();
        panel_style.merge(style);
        let panel = ContainerWidget::new()
            .style(panel_style.absolute())
            .on_click(|_, _| EventResult::Consumed)
            .on_dismiss(move |_, _| {
                notify_open(false, controlled, state, &callback);
                EventResult::Consumed
            })
            .into_element_desc(vec![content.clone()]);
        children.push(
            portal(vec![panel])
                .z_index(*z_index)
                .modal(*modal)
                .focus_scope(FocusScopeOptions {
                    trap: *modal,
                    auto_focus: *modal,
                    restore_focus: true,
                })
                .dismiss_on_escape(true)
                .dismiss_on_pointer_outside(true)
                .anchor(
                    anchor.get(),
                    OverlayPositionOptions {
                        placement: *placement,
                        ..Default::default()
                    },
                )
                .into(),
        );
    }
    ContainerWidget::new().into_element_desc(children)
}

#[component]
#[defaults(
    placement = OverlayPlacement::Top,
    z_index = 3000,
    style = Style::new(),
)]
pub fn tooltip(
    trigger: &ElementDesc,
    text: &String,
    placement: &OverlayPlacement,
    z_index: &i32,
    style: &Style,
) {
    let shown = cx.use_state(|| false);
    let anchor = cx.use_memo(AnchorHandle::new);
    let trigger = ContainerWidget::new()
        .anchor_handle(anchor.get().clone())
        .on_pointer_enter(move |_, _| {
            shown.set(true);
            EventResult::Ignored
        })
        .on_pointer_leave(move |_, _| {
            shown.set(false);
            EventResult::Ignored
        })
        .on_focus(move |_, _| {
            shown.set(true);
            EventResult::Ignored
        })
        .on_blur(move |_, _| {
            shown.set(false);
            EventResult::Ignored
        })
        .into_element_desc(vec![trigger.clone()]);
    let mut children = vec![trigger];
    if *shown.get() {
        let mut tooltip_style = Style::new()
            .absolute()
            .padding(EdgeInsets::symmetric(8.0, 5.0))
            .background(Color::rgba(0.08, 0.09, 0.11, 0.96))
            .color(Color::WHITE)
            .border_radius(RadiusToken::Sm)
            .max_width(280.0)
            .font_size(FontSizeToken::Sm);
        tooltip_style.merge(style);
        let tip = ContainerWidget::new()
            .style(tooltip_style)
            .accessibility_role(AccessibilityRole::Tooltip)
            .accessibility_label(text.clone())
            .into_element_desc(vec![TextWidget::new(text.clone()).into_element_desc()]);
        children.push(
            portal(vec![tip])
                .z_index(*z_index)
                .hit_test(false)
                .anchor(
                    anchor.get(),
                    OverlayPositionOptions {
                        placement: *placement,
                        offset: 6.0,
                        ..Default::default()
                    },
                )
                .into(),
        );
    }
    ContainerWidget::new().into_element_desc(children)
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum DrawerSide {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

#[component]
#[defaults(
    open = false,
    side = DrawerSide::Right,
    on_open_change = None,
    z_index = 2200,
    style = Style::new(),
)]
pub fn drawer(
    content: &ElementDesc,
    open: &bool,
    side: &DrawerSide,
    on_open_change: &Option<BoolChangeCallback>,
    z_index: &i32,
    style: &Style,
) {
    if !*open {
        return ContainerWidget::new()
            .accessibility_hidden(true)
            .into_element_desc(vec![]);
    }
    let callback = on_open_change.clone();
    let (direction, justify, sizing) = match side {
        DrawerSide::Left => (
            FlexDirectionStyle::Row,
            JustifyStyle::Start,
            Style::new().width(360.0).height(Sizing::Fill),
        ),
        DrawerSide::Right => (
            FlexDirectionStyle::Row,
            JustifyStyle::End,
            Style::new().width(360.0).height(Sizing::Fill),
        ),
        DrawerSide::Top => (
            FlexDirectionStyle::Column,
            JustifyStyle::Start,
            Style::new().width(Sizing::Fill).height(280.0),
        ),
        DrawerSide::Bottom => (
            FlexDirectionStyle::Column,
            JustifyStyle::End,
            Style::new().width(Sizing::Fill).height(280.0),
        ),
    };
    let mut panel_style = sizing
        .padding(EdgeInsets::all(20.0))
        .background(ColorToken::Surface);
    panel_style.merge(style);
    let panel = ContainerWidget::new()
        .style(panel_style)
        .focusable(true)
        .tab_index(0)
        .accessibility_role(AccessibilityRole::Dialog)
        .on_click(|_, _| EventResult::Consumed)
        .into_element_desc(vec![content.clone()]);
    let root = ContainerWidget::new()
        .style(
            Style::new()
                .absolute()
                .inset(EdgeInsets::zero())
                .size(Size::fill())
                .justify(justify)
                .background(Color::rgba(0.0, 0.0, 0.0, 0.4)),
        )
        .flex_direction(direction)
        .on_dismiss(move |_, _| {
            if let Some(callback) = &callback {
                callback.call(false);
            }
            EventResult::Consumed
        })
        .into_element_desc(vec![panel]);
    portal(vec![root])
        .z_index(*z_index)
        .modal(true)
        .dismiss_on_escape(true)
        .dismiss_on_pointer_outside(true)
        .into()
}

#[derive(Clone, Debug, Hash)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub disabled: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

pub type MenuSelectCallback = Callback<usize>;

#[component]
#[defaults(
    open = None,
    default_open = false,
    on_open_change = None,
    on_select = None,
    placement = OverlayPlacement::BottomStart,
    z_index = 1600,
    style = Style::new(),
)]
pub fn menu(
    trigger: &ElementDesc,
    items: &Vec<MenuItem>,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    on_select: &Option<MenuSelectCallback>,
    placement: &OverlayPlacement,
    z_index: &i32,
    style: &Style,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    let selected = on_select.clone();
    let open_callback = on_open_change.clone();
    let rows = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let selected = selected.clone();
            let open_callback = open_callback.clone();
            let disabled = item.disabled;
            let mut row = ContainerWidget::new()
                .style(
                    Style::new()
                        .padding(EdgeInsets::symmetric(10.0, 7.0))
                        .border_radius(RadiusToken::Sm)
                        .when(WidgetState::HOVERED, |style| {
                            style.background(ColorToken::MutedSurface)
                        }),
                )
                .focusable(!disabled)
                .tab_index(if disabled { -1 } else { 0 })
                .accessibility_role(AccessibilityRole::MenuItem)
                .accessibility_label(item.label.clone())
                .accessibility_disabled(disabled);
            if !disabled {
                row = row.on_click(move |_, _| {
                    if let Some(callback) = &selected {
                        callback.call(index);
                    }
                    if !controlled {
                        state.set(false);
                    }
                    if let Some(callback) = &open_callback {
                        callback.call(false);
                    }
                    EventResult::Consumed
                });
            }
            row.into_element_desc(vec![
                TextWidget::new(item.label.clone()).into_element_desc(),
            ])
        })
        .collect::<Vec<_>>();
    let mut content_style = Style::new().min_width(180.0).padding(EdgeInsets::all(4.0));
    content_style.merge(style);
    let content = ContainerWidget::new()
        .style(content_style)
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::Menu)
        .into_element_desc(rows);
    let change = cx.use_callback((controlled, state, on_open_change.clone()), {
        let callback = on_open_change.clone();
        move |next| {
            if !controlled {
                state.set(next);
            }
            if let Some(callback) = &callback {
                callback.call(next);
            }
        }
    });
    xui! {
        <popover
            trigger={trigger.clone()}
            content={content}
            open={Some(shown)}
            on_open_change={Some(change)}
            placement={*placement}
            modal={true}
            z_index={*z_index}
        />
    }
}

/// A pointer- or keyboard-triggered menu positioned at the requested window
/// coordinate. It uses the semantic context-menu event, so platform keyboard
/// shortcuts and long-press synthesis share the same path as right-click.
#[component]
#[defaults(
    open = None,
    default_open = false,
    on_open_change = None,
    on_select = None,
    z_index = 1700,
    style = Style::new(),
)]
pub fn context_menu(
    trigger: &ElementDesc,
    items: &Vec<MenuItem>,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    on_select: &Option<MenuSelectCallback>,
    z_index: &i32,
    style: &Style,
) {
    let state = cx.use_state(|| *default_open);
    let position = cx.use_state(|| Point::new(0.0, 0.0));
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    let opened = on_open_change.clone();
    let trigger = ContainerWidget::new()
        .on_context_menu(move |event, event_cx| {
            let point = event
                .position
                .or_else(|| event.pointer.map(|pointer| pointer.coords.window))
                .unwrap_or(event_cx.node_ref.world_origin);
            position.set(point);
            if !controlled {
                state.set(true);
            }
            if let Some(callback) = &opened {
                callback.call(true);
            }
            EventResult::Consumed
        })
        .into_element_desc(vec![trigger.clone()]);
    let mut children = vec![trigger];
    if shown {
        let selected = on_select.clone();
        let closed = on_open_change.clone();
        let rows = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let selected = selected.clone();
                let closed = closed.clone();
                let disabled = item.disabled;
                let mut row = ContainerWidget::new()
                    .style(
                        Style::new()
                            .padding(EdgeInsets::symmetric(10.0, 7.0))
                            .border_radius(RadiusToken::Sm)
                            .when(WidgetState::HOVERED, |style| {
                                style.background(ColorToken::MutedSurface)
                            }),
                    )
                    .focusable(!disabled)
                    .tab_index(if disabled { -1 } else { 0 })
                    .accessibility_role(AccessibilityRole::MenuItem)
                    .accessibility_label(item.label.clone())
                    .accessibility_disabled(disabled);
                if !disabled {
                    row = row.on_click(move |_, _| {
                        if let Some(callback) = &selected {
                            callback.call(index);
                        }
                        if !controlled {
                            state.set(false);
                        }
                        if let Some(callback) = &closed {
                            callback.call(false);
                        }
                        EventResult::Consumed
                    });
                }
                row.into_element_desc(vec![
                    TextWidget::new(item.label.clone()).into_element_desc(),
                ])
            })
            .collect::<Vec<_>>();
        let point = *position.get();
        let mut panel_style = overlay_panel_style()
            .min_width(180.0)
            .padding(EdgeInsets::all(4.0))
            .margin(EdgeInsets::new(
                point.x.max(0.0),
                0.0,
                point.y.max(0.0),
                0.0,
            ));
        panel_style.merge(style);
        let panel = ContainerWidget::new()
            .style(panel_style)
            .flex_direction(FlexDirectionStyle::Column)
            .accessibility_role(AccessibilityRole::Menu)
            .on_click(|_, _| EventResult::Consumed)
            .into_element_desc(rows);
        let closed = on_open_change.clone();
        let root = ContainerWidget::new()
            .style(
                Style::new()
                    .absolute()
                    .inset(EdgeInsets::zero())
                    .size(Size::fill()),
            )
            .on_dismiss(move |_, _| {
                if !controlled {
                    state.set(false);
                }
                if let Some(callback) = &closed {
                    callback.call(false);
                }
                EventResult::Consumed
            })
            .into_element_desc(vec![panel]);
        children.push(
            portal(vec![root])
                .z_index(*z_index)
                .modal(true)
                .dismiss_on_escape(true)
                .dismiss_on_pointer_outside(true)
                .into(),
        );
    }
    ContainerWidget::new().into_element_desc(children)
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum ToastTone {
    #[default]
    Neutral,
    Success,
    Warning,
    Danger,
}

#[component]
#[defaults(open = true, tone = ToastTone::Neutral, z_index = 4000, style = Style::new())]
pub fn toast(message: &String, open: &bool, tone: &ToastTone, z_index: &i32, style: &Style) {
    if !*open {
        return ContainerWidget::new()
            .accessibility_hidden(true)
            .into_element_desc(vec![]);
    }
    let background: ColorStyle = match tone {
        ToastTone::Neutral => ColorToken::Surface.into(),
        ToastTone::Success => Color::hex("#166534").into(),
        ToastTone::Warning => Color::hex("#92400e").into(),
        ToastTone::Danger => Color::hex("#991b1b").into(),
    };
    let mut toast_style = Style::new()
        .max_width(420.0)
        .padding(EdgeInsets::all(12.0))
        .background(background)
        .border_radius(RadiusToken::Md);
    toast_style.merge(style);
    let item = ContainerWidget::new()
        .style(toast_style)
        .accessibility_live_region(AccessibilityLiveRegion::Polite)
        .accessibility_label(message.clone())
        .into_element_desc(vec![TextWidget::new(message.clone()).into_element_desc()]);
    let root = ContainerWidget::new()
        .style(
            Style::new()
                .absolute()
                .inset(EdgeInsets::zero())
                .size(Size::fill())
                .padding(EdgeInsets::all(16.0))
                .align(AlignStyle::End)
                .justify(JustifyStyle::Start),
        )
        .into_element_desc(vec![item]);
    portal(vec![root]).z_index(*z_index).hit_test(false).into()
}
