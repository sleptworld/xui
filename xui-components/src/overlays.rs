//! Portal-mounted dialogs, popovers, hints, menus, drawers, and toasts.
//!
//! [`dialog`], [`drawer`], [`popover`], [`tooltip`], [`menu`], [`context_menu`], and
//! [`toast`] share runtime stacking and dismissal behavior. Anchored components use
//! [`xui_core::anchor::AnchorPlacement`]. Modal portals block background pointer input;
//! automatic focus trapping/restoration is not provided by the current portal API.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`dialog`](fn@self::dialog) | A modal portal containing a titled dialog panel and backdrop. |
//! | [`popover`](fn@self::popover) | An anchored portal opened by a focusable trigger. |
//! | [`tooltip`](fn@self::tooltip) | A non-interactive anchored hint shown on pointer hover or descendant focus. |
//! | [`drawer`](fn@self::drawer) | An edge-mounted modal sheet with a configurable side and backdrop. |
//! | [`menu`](fn@self::menu) | A trigger-based menu with disabled actions and keyboard focus navigation. |
//! | [`context_menu`](fn@self::context_menu) | A menu opened by semantic context-menu events at a pointer or keyboard-requested position. |
//! | [`toast`](fn@self::toast) | A portal-mounted transient message with a semantic tone and live announcement. |

use crate::layout::MergeClone;
use xui_core::prelude::*;
use xui_macros::{component, view};

use crate::BoolChangeCallback;

fn notify_open(
    next: bool,
    controlled: bool,
    state: xui_core::state::State<bool>,
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

/// A modal portal containing a titled dialog panel and backdrop.
///
/// `open` controls visibility or `default_open` seeds internal state. `on_open_change`
/// reports dismiss requests. Escape and backdrop/pointer-outside dismissal can be
/// configured separately. `style`, `title_style`, and `backdrop_style` customize the
/// panel, title, and overlay. Modal stacking blocks background pointer input; automatic
/// focus trapping/restoration is not supplied by the current portal API.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `content` | `ElementDesc` | **Required** |
/// | `open` | `Option<bool>` | `None` |
/// | `default_open` | `bool` | `false` |
/// | `title` | `Option<String>` | `None` |
/// | `on_open_change` | `Option<BoolChangeCallback>` | `None` |
/// | `close_on_escape` | `bool` | `true` |
/// | `close_on_pointer_outside` | `bool` | `true` |
/// | `z_index` | `i32` | `2000` |
/// | `style` | `Style` | `Style::new()` |
/// | `title_style` | `Style` | `Style::new()` |
/// | `backdrop_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { dialog(content: view! { text("Dialog content") }, title: Some("Settings".to_owned()), open: Some(true)) };
/// ```
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
    title_style = Style::new(),
    backdrop_style = Style::new(),
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
    title_style: &Style,
    backdrop_style: &Style,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    if !shown {
        return view! {
            container(
                accessibility_hidden: true,
            )
        };
    }
    let callback = on_open_change.clone();
    let close_escape = *close_on_escape;
    let close_outside = *close_on_pointer_outside;
    let mut panel_style = overlay_panel_style()
        .min_width(0.0)
        .width(Sizing::percent(0.9))
        .max_width(640.0)
        .max_height(Sizing::percent(0.9))
        .scroll_vertical();
    panel_style.merge(style);
    let mut panel_children = Vec::with_capacity(2);
    if let Some(title) = title {
        panel_children.push(view! {
            xui_core::widgets::text(
                title.clone(),
                style: Style::new()
                    .font_size(FontSizeToken::Lg)
                    .font_weight(FontWeight::Bold).merge_clone(title_style),
            )
        });
    }
    panel_children.push(content.clone());
    let panel = view! {
        container(
            style: panel_style,
            flex_direction: FlexDirectionStyle::Column,
            gap: 12.0,
            focusable: true,
            tab_index: 0,
            accessibility_role: AccessibilityRole::Dialog,
            accessibility_label: title.clone().unwrap_or_default(),
            on_click: |_, _| EventResult::Consumed,
        ) {
            ..panel_children
        }
    };
    let root = view! {
        container(
            style: Style::new()
                .absolute()
                .inset(EdgeInsets::zero())
                .size(Size::fill())
                .align(AlignStyle::Center)
                .justify(JustifyStyle::Center)
                .background(Color::rgba(0.0, 0.0, 0.0, 0.46)).merge_clone(backdrop_style),
            on_click: { let callback = on_open_change.clone(); move |_, _| { if close_outside { notify_open(false, controlled, state, &callback); } EventResult::Consumed } },
        ) {
            ..panel
        }
    };
    view! {
        xui_core::element::portal_view(
            on_dismiss: move |reason| { if (reason == DismissReason::Escape && close_escape) || (reason == DismissReason::PointerOutside && close_outside) {
                    notify_open(false, controlled, state, &callback);
                } },
            z_index: *z_index,
            modal: true,
        ) {
            ..root
        }
    }
}

/// An anchored portal opened by a focusable trigger.
///
/// `open` controls visibility or `default_open` seeds internal state. `on_open_change`
/// reports trigger and dismissal requests. `placement` configures side, alignment, gap,
/// width matching, and collision handling. `modal` controls background pointer blocking.
/// `disabled` suppresses the trigger and open content; `trigger_style` and `style`
/// customize the trigger and panel.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `trigger` | `ElementDesc` | **Required** |
/// | `content` | `ElementDesc` | **Required** |
/// | `open` | `Option<bool>` | `None` |
/// | `default_open` | `bool` | `false` |
/// | `on_open_change` | `Option<BoolChangeCallback>` | `None` |
/// | `placement` | `AnchorPlacement` | `AnchorPlacement::default()` |
/// | `modal` | `bool` | `false` |
/// | `z_index` | `i32` | `1500` |
/// | `style` | `Style` | `Style::new()` |
/// | `trigger_style` | `Style` | `Style::new()` |
/// | `disabled` | `bool` | `false` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { popover(trigger: view! { text("More") }, content: view! { text("Additional details") }) };
/// ```
#[component]
#[defaults(
    open = None,
    default_open = false,
    on_open_change = None,
    placement = AnchorPlacement::default(),
    modal = false,
    z_index = 1500,
    style = Style::new(),
    trigger_style = Style::new(),
    disabled = false,
)]
pub fn popover(
    trigger: &ElementDesc,
    content: &ElementDesc,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    placement: &AnchorPlacement,
    modal: &bool,
    z_index: &i32,
    style: &Style,
    trigger_style: &Style,
    disabled: &bool,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = !*disabled && open.unwrap_or(*state.get());
    let disabled = *disabled;
    let callback = on_open_change.clone();
    let trigger_root = view! {
        container(
            style: crate::layout::control_style(disabled).merge_clone(trigger_style),
            accessibility_role: AccessibilityRole::Button,
            accessibility_expanded: shown,
            focusable: !disabled,
            tab_index: if disabled { -1 } else { 0 },
            accessibility_disabled: disabled,
            on_click: move |_, event_cx| {
                if disabled { return EventResult::Ignored; }
                event_cx.request_focus();
                notify_open(!shown, controlled, state, &callback);
                EventResult::Consumed
            },
        ) {
            ..trigger.clone()
        }
    };
    let mut children = vec![trigger_root];
    if shown {
        let callback = on_open_change.clone();
        let mut panel_style = overlay_panel_style();
        panel_style.merge(style);
        let panel = view! {
            container(
                style: panel_style.absolute(),
                on_click: |_, _| EventResult::Consumed,
            ) {
                ..content.clone()
            }
        };
        children.push(view! {
            xui_core::element::portal_view(
                on_dismiss: move |_| {
                    notify_open(false, controlled, state, &callback);
                },
                z_index: *z_index,
                modal: *modal,
                anchor: *placement,
            ) {
                ..panel
            }
        });
    }
    view! {
        container() {
            ..children
        }
    }
}

/// A non-interactive anchored hint shown on pointer hover or descendant focus.
///
/// The hint remains visible while either hover or focus is active. `placement` configures
/// the anchor and the component uses a six-point gap. `trigger_style` and `style`
/// customize the trigger wrapper and hint panel. The portal does not intercept pointer
/// input.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `trigger` | `ElementDesc` | **Required** |
/// | `text` | `String` | **Required** |
/// | `placement` | `AnchorPlacement` | `AnchorPlacement::new(AnchorSide::Top).align(AnchorAlign::Center)` |
/// | `z_index` | `i32` | `3000` |
/// | `style` | `Style` | `Style::new()` |
/// | `trigger_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { tooltip(trigger: view! { text("Help") }, text: "Show help") };
/// ```
#[component]
#[defaults(
    placement = AnchorPlacement::new(AnchorSide::Top).align(AnchorAlign::Center),
    z_index = 3000,
    style = Style::new(),
    trigger_style = Style::new(),
)]
pub fn tooltip(
    trigger: &ElementDesc,
    text: &String,
    placement: &AnchorPlacement,
    z_index: &i32,
    style: &Style,
    trigger_style: &Style,
) {
    let hovered = cx.use_state(|| false);
    let focused = cx.use_state(|| false);
    let trigger = view! {
        container(
            style: trigger_style.clone(),
            on_pointer_enter: move |_, _| {
                hovered.set(true);
                EventResult::Ignored
            },
            on_pointer_leave: move |_, _| {
                hovered.set(false);
                EventResult::Ignored
            },
            on_focus_in: move |_, _| {
                focused.set(true);
                EventResult::Ignored
            },
            on_focus_out: move |_, _| {
                focused.set(false);
                EventResult::Ignored
            },
        ) {
            ..trigger.clone()
        }
    };
    let mut children = vec![trigger];
    if *hovered.get() || *focused.get() {
        let mut tooltip_style = Style::new()
            .absolute()
            .padding(EdgeInsets::symmetric(8.0, 5.0))
            .background(ColorToken::Foreground)
            .color(ColorToken::Background)
            .border_radius(RadiusToken::Sm)
            .max_width(280.0)
            .font_size(FontSizeToken::Sm);
        tooltip_style.merge(style);
        let tip = view! {
            container(
                style: tooltip_style,
                accessibility_role: AccessibilityRole::Tooltip,
                accessibility_label: text.clone(),
            ) {
                xui_core::widgets::text(
                    text.clone(),
                )
            }
        };
        children.push(view! {
            xui_core::element::portal_view(
                z_index: *z_index,
                hit_test: false,
                anchor: (*placement).offset(6.0),
            ) {
                ..tip
            }
        });
    }
    view! {
        container() {
            ..children
        }
    }
}

/// The viewport edge from which a drawer panel is positioned.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum DrawerSide {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

/// An edge-mounted modal sheet with a configurable side and backdrop.
///
/// `side` chooses the edge and panel dimensions. `open` controls visibility or
/// `default_open` seeds internal state; dismiss requests notify `on_open_change`. `style`
/// customizes the panel and `backdrop_style` customizes the full-window backdrop. The
/// panel is sized to fit narrow windows; no opening/closing motion is currently added.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `content` | `ElementDesc` | **Required** |
/// | `open` | `Option<bool>` | `None` |
/// | `side` | `DrawerSide` | `DrawerSide::Right` |
/// | `on_open_change` | `Option<BoolChangeCallback>` | `None` |
/// | `z_index` | `i32` | `2200` |
/// | `style` | `Style` | `Style::new()` |
/// | `backdrop_style` | `Style` | `Style::new()` |
/// | `default_open` | `bool` | `false` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { drawer(content: view! { text("Settings") }, side: DrawerSide::Right, open: Some(true)) };
/// ```
#[component]
#[defaults(
    open = None,
    side = DrawerSide::Right,
    on_open_change = None,
    z_index = 2200,
    style = Style::new(),
    backdrop_style = Style::new(),
    default_open = false,
)]
pub fn drawer(
    content: &ElementDesc,
    open: &Option<bool>,
    side: &DrawerSide,
    on_open_change: &Option<BoolChangeCallback>,
    z_index: &i32,
    style: &Style,
    backdrop_style: &Style,
    default_open: &bool,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    if !shown {
        return view! {
            container(
                accessibility_hidden: true,
            )
        };
    }
    let callback = on_open_change.clone();
    let (direction, justify, sizing) = match side {
        DrawerSide::Left => (
            FlexDirectionStyle::Row,
            JustifyStyle::Start,
            Style::new()
                .width(Sizing::percent(0.9))
                .max_width(360.0)
                .height(Sizing::Fill),
        ),
        DrawerSide::Right => (
            FlexDirectionStyle::Row,
            JustifyStyle::End,
            Style::new()
                .width(Sizing::percent(0.9))
                .max_width(360.0)
                .height(Sizing::Fill),
        ),
        DrawerSide::Top => (
            FlexDirectionStyle::Column,
            JustifyStyle::Start,
            Style::new()
                .width(Sizing::Fill)
                .height(Sizing::percent(0.8))
                .max_height(280.0),
        ),
        DrawerSide::Bottom => (
            FlexDirectionStyle::Column,
            JustifyStyle::End,
            Style::new()
                .width(Sizing::Fill)
                .height(Sizing::percent(0.8))
                .max_height(280.0),
        ),
    };
    let mut panel_style = sizing
        .padding(EdgeInsets::all(20.0))
        .background(ColorToken::Surface);
    panel_style.merge(style);
    let panel = view! {
        container(
            style: panel_style,
            focusable: true,
            tab_index: 0,
            accessibility_role: AccessibilityRole::Dialog,
            on_click: |_, _| EventResult::Consumed,
        ) {
            ..content.clone()
        }
    };
    let root = view! {
        container(
            style: Style::new()
                .absolute()
                .inset(EdgeInsets::zero())
                .size(Size::fill())
                .justify(justify)
                .background(Color::rgba(0.0, 0.0, 0.0, 0.4)).merge_clone(backdrop_style),
            flex_direction: direction,
            on_click: { let callback = on_open_change.clone(); move |_, _| { notify_open(false, controlled, state, &callback); EventResult::Consumed } },
        ) {
            ..panel
        }
    };
    view! {
        xui_core::element::portal_view(
            on_dismiss: move |_| {
                notify_open(false, controlled, state, &callback);
            },
            z_index: *z_index,
            modal: true,
        ) {
            ..root
        }
    }
}

/// An identified menu action with a display label and disabled state.
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

/// Callback requesting an enabled menu action by its zero-based index.
pub type MenuSelectCallback = Callback<usize>;

/// A trigger-based menu with disabled actions and keyboard focus navigation.
///
/// `open` controls visibility or `default_open` seeds internal state. Selecting an
/// enabled item calls `on_select` with its index and requests closure through
/// `on_open_change`. Up/Down/Home/End move focus past disabled entries; Enter/Space
/// activate the focused item. The option subtree is built only while open. `item_style`
/// customizes entries and `style` customizes the menu content.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `trigger` | `ElementDesc` | **Required** |
/// | `items` | `Vec<MenuItem>` | **Required** |
/// | `open` | `Option<bool>` | `None` |
/// | `default_open` | `bool` | `false` |
/// | `on_open_change` | `Option<BoolChangeCallback>` | `None` |
/// | `on_select` | `Option<MenuSelectCallback>` | `None` |
/// | `placement` | `AnchorPlacement` | `AnchorPlacement::default()` |
/// | `z_index` | `i32` | `1600` |
/// | `style` | `Style` | `Style::new()` |
/// | `item_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { menu(trigger: view! { text("Actions") }, items: vec![MenuItem::new("copy", "Copy")]) };
/// ```
#[component]
#[defaults(
    open = None,
    default_open = false,
    on_open_change = None,
    on_select = None,
    placement = AnchorPlacement::default(),
    z_index = 1600,
    style = Style::new(),
    item_style = Style::new(),
)]
pub fn menu(
    trigger: &ElementDesc,
    items: &Vec<MenuItem>,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    on_select: &Option<MenuSelectCallback>,
    placement: &AnchorPlacement,
    z_index: &i32,
    style: &Style,
    item_style: &Style,
) {
    let handles = cx.use_memo_with(
        items.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        || {
            (0..items.len())
                .map(|_| FocusHandle::new())
                .collect::<Vec<_>>()
                .into_iter()
                .collect::<std::rc::Rc<[FocusHandle]>>()
        },
    );
    let model = std::rc::Rc::new(crate::SelectionModel::new(items.iter().map(|item| {
        crate::SelectionItem::new(item.id.clone(), item.label.clone(), item.disabled)
    })));
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    let selected = on_select.clone();
    let open_callback = on_open_change.clone();
    let rows = if shown {
        items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let selected = selected.clone();
            let open_callback = open_callback.clone();
            let disabled = item.disabled;
            let handles = handles.get().clone();
            let model = model.clone();
            let interactive = !disabled;

            view! {
                container(
                    style: Style::new()
                        .padding(EdgeInsets::symmetric(10.0, 7.0))
                        .border_radius(RadiusToken::Sm)
                        .when(WidgetState::HOVERED, |style| {
                            style.background(ColorToken::Accent)
                    }).merge_clone(&crate::layout::control_style(disabled)).merge_clone(item_style),
                    key: item.id.clone(),
                    focus_handle: handles[index].clone(),
                    shortcuts: crate::selection::selection_shortcuts(),
                    on_command: move |event, event_cx| {
                        if disabled { return EventResult::Ignored; }
                        let Some(target) = crate::selection::selection_target(&model, index, event.command) else { return EventResult::Ignored; };
                        handles[target].request_focus(event_cx);
                        EventResult::Consumed
                    },
                    focusable: !disabled,
                    tab_index: if disabled { -1 } else { 0 },
                    accessibility_role: AccessibilityRole::MenuItem,
                    accessibility_label: item.label.clone(),
                    accessibility_disabled: disabled,
                    on_click: move |_, _| { if !interactive { return EventResult::Ignored; }
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
                    },
                ) {
                    xui_core::widgets::text(
                        item.label.clone(),
                    )
                }
            }
        })
        .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut content_style = Style::new().min_width(180.0).padding(EdgeInsets::all(4.0));
    content_style.merge(style);
    let content = view! {
        container(
            style: content_style,
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Menu,
        ) {
            ..rows
        }
    };
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
    view! {
        popover(
            trigger: trigger.share(),
            content: content,
            open: Some(shown),
            on_open_change: Some(change),
            placement: *placement,
            modal: true,
            z_index: *z_index,
        )
    }
}

/// A menu opened by semantic context-menu events at a pointer or keyboard-requested
/// position.
///
/// `open` controls visibility or `default_open` seeds internal state. Enabled actions
/// notify `on_select` and request closure; keyboard navigation skips disabled items.
/// `item_style` customizes entries and `style` customizes the panel. The requested window
/// position is stored when a context-menu event arrives.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `trigger` | `ElementDesc` | **Required** |
/// | `items` | `Vec<MenuItem>` | **Required** |
/// | `open` | `Option<bool>` | `None` |
/// | `default_open` | `bool` | `false` |
/// | `on_open_change` | `Option<BoolChangeCallback>` | `None` |
/// | `on_select` | `Option<MenuSelectCallback>` | `None` |
/// | `z_index` | `i32` | `1700` |
/// | `style` | `Style` | `Style::new()` |
/// | `item_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { context_menu(trigger: view! { text("Right-click here") }, items: vec![MenuItem::new("copy", "Copy")]) };
/// ```
#[component]
#[defaults(
    open = None,
    default_open = false,
    on_open_change = None,
    on_select = None,
    z_index = 1700,
    style = Style::new(),
    item_style = Style::new(),
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
    item_style: &Style,
) {
    let handles = cx.use_memo_with(
        items.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        || {
            (0..items.len())
                .map(|_| FocusHandle::new())
                .collect::<Vec<_>>()
                .into_iter()
                .collect::<std::rc::Rc<[FocusHandle]>>()
        },
    );
    let model = std::rc::Rc::new(crate::SelectionModel::new(items.iter().map(|item| {
        crate::SelectionItem::new(item.id.clone(), item.label.clone(), item.disabled)
    })));
    let state = cx.use_state(|| *default_open);
    let position = cx.use_state(|| Point::new(0.0, 0.0));
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    let opened = on_open_change.clone();
    let trigger = view! {
        container(
            on_context_menu: move |event, event_cx| {
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
            },
        ) {
            ..trigger.clone()
        }
    };
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
            let handles = handles.get().clone();
            let model = model.clone();
                let navigation_enabled = !disabled;

                view! {
                    container(
                        style: Style::new()
                            .padding(EdgeInsets::symmetric(10.0, 7.0))
                            .border_radius(RadiusToken::Sm)
                            .when(WidgetState::HOVERED, |style| {
                                style.background(ColorToken::Accent)
                        }).merge_clone(&crate::layout::control_style(disabled)).merge_clone(item_style),
                        key: item.id.clone(),
                        focus_handle: handles[index].clone(),
                        shortcuts: crate::selection::selection_shortcuts(),
                        on_command: move |event, event_cx| {
                            if disabled { return EventResult::Ignored; }
                            let Some(target) = crate::selection::selection_target(&model, index, event.command) else { return EventResult::Ignored; };
                            handles[target].request_focus(event_cx);
                            EventResult::Consumed
                        },
                        focusable: !disabled,
                        tab_index: if disabled { -1 } else { 0 },
                        accessibility_role: AccessibilityRole::MenuItem,
                        accessibility_label: item.label.clone(),
                        accessibility_disabled: disabled,
                        on_click: move |_, _| { if !navigation_enabled { return EventResult::Ignored; }
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
                        },
                    ) {
                        xui_core::widgets::text(
                            item.label.clone(),
                        )
                    }
                }
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
        let panel = view! {
            container(
                style: panel_style,
                flex_direction: FlexDirectionStyle::Column,
                accessibility_role: AccessibilityRole::Menu,
                on_click: |_, _| EventResult::Consumed,
            ) {
                ..rows
            }
        };
        let closed = on_open_change.clone();
        let root = view! {
            container(
                style: Style::new()
                    .absolute()
                    .inset(EdgeInsets::zero())
                    .size(Size::fill()),
                on_click: { let callback = on_open_change.clone(); move |_, _| { notify_open(false, controlled, state, &callback); EventResult::Consumed } },
            ) {
                ..panel
            }
        };
        children.push(view! {
            xui_core::element::portal_view(
                on_dismiss: move |_| {
                    if !controlled {
                        state.set(false);
                    }
                    if let Some(callback) = &closed {
                        callback.call(false);
                    }
                },
                z_index: *z_index,
                modal: true,
            ) {
                ..root
            }
        });
    }
    view! {
        container() {
            ..children
        }
    }
}

/// The accent emphasis of a transient message.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum ToastTone {
    #[default]
    Neutral,
    Success,
    Warning,
    Danger,
}

/// A portal-mounted transient message with a semantic tone and live announcement.
///
/// `open` is caller-controlled; the component does not schedule automatic dismissal.
/// `tone` selects the message accent, `z_index` controls stacking, and `style` overrides
/// the message box. The portal does not intercept pointer input.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `message` | `String` | **Required** |
/// | `open` | `bool` | `true` |
/// | `tone` | `ToastTone` | `ToastTone::Neutral` |
/// | `z_index` | `i32` | `4000` |
/// | `style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { toast(message: "Saved", open: true) };
/// ```
#[component]
#[defaults(
    open = true,
    tone = ToastTone::Neutral,
    z_index = 4000,
    style = Style::new(),
)]
pub fn toast(message: &String, open: &bool, tone: &ToastTone, z_index: &i32, style: &Style) {
    if !*open {
        return view! {
            container(
                accessibility_hidden: true,
            )
        };
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
    let item = view! {
        container(
            style: toast_style,
            accessibility_live_region: AccessibilityLiveRegion::Polite,
            accessibility_label: message.clone(),
        ) {
            xui_core::widgets::text(
                message.clone(),
            )
        }
    };
    let root = view! {
        container(
            style: Style::new()
                .absolute()
                .inset(EdgeInsets::zero())
                .size(Size::fill())
                .padding(EdgeInsets::all(16.0))
                .align(AlignStyle::End)
                .justify(JustifyStyle::Start),
        ) {
            ..item
        }
    };
    view! {
        xui_core::element::portal_view(
            z_index: *z_index,
            hit_test: false,
        ) {
            ..root
        }
    }
}
