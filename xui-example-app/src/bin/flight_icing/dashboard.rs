use xui_components::*;
use xui_core::prelude::*;
use xui_macros::component;

const PAGE: Color = Color::rgb(10.0 / 255.0, 10.0 / 255.0, 11.0 / 255.0);
const PANEL: Color = Color::rgb(18.0 / 255.0, 18.0 / 255.0, 20.0 / 255.0);
const SURFACE: Color = Color::rgb(26.0 / 255.0, 26.0 / 255.0, 28.0 / 255.0);
const CONTROL: Color = Color::rgb(42.0 / 255.0, 42.0 / 255.0, 44.0 / 255.0);
const BORDER: Color = CONTROL;
const TEXT: Color = Color::rgb(224.0 / 255.0, 224.0 / 255.0, 224.0 / 255.0);
const MUTED: Color = Color::rgb(139.0 / 255.0, 139.0 / 255.0, 145.0 / 255.0);
const DIM: Color = Color::rgb(92.0 / 255.0, 92.0 / 255.0, 98.0 / 255.0);
const ACCENT: Color = Color::rgb(1.0, 159.0 / 255.0, 10.0 / 255.0);
const BLUE: Color = Color::rgb(10.0 / 255.0, 132.0 / 255.0, 1.0);
const RED: Color = Color::rgb(1.0, 59.0 / 255.0, 48.0 / 255.0);
const GREEN: Color = Color::rgb(89.0 / 255.0, 210.0 / 255.0, 111.0 / 255.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceIcon {
    Crosshair,
    ZoomIn,
    ZoomOut,
    RotateCcw,
    Previous,
    Play,
    Next,
}

fn source_icon(kind: SourceIcon) -> IconData {
    let body = match kind {
        SourceIcon::Crosshair => {
            r#"<circle cx="12" cy="12" r="10"/><line x1="22" x2="18" y1="12" y2="12"/><line x1="6" x2="2" y1="12" y2="12"/><line x1="12" x2="12" y1="6" y2="2"/><line x1="12" x2="12" y1="22" y2="18"/>"#
        }
        SourceIcon::ZoomIn => {
            r#"<circle cx="11" cy="11" r="8"/><line x1="21" x2="16.65" y1="21" y2="16.65"/><line x1="11" x2="11" y1="8" y2="14"/><line x1="8" x2="14" y1="11" y2="11"/>"#
        }
        SourceIcon::ZoomOut => {
            r#"<circle cx="11" cy="11" r="8"/><line x1="21" x2="16.65" y1="21" y2="16.65"/><line x1="8" x2="14" y1="11" y2="11"/>"#
        }
        SourceIcon::RotateCcw => {
            r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#
        }
        SourceIcon::Previous => r#"<path d="m19 20-9-8 9-8v16Z"/><path d="M5 19V5"/>"#,
        SourceIcon::Play => r#"<polygon points="6 3 20 12 6 21 6 3"/>"#,
        SourceIcon::Next => r#"<path d="m5 4 9 8-9 8V4Z"/><path d="M19 5v14"/>"#,
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#
    );
    IconData::from_svg(&svg).expect("source Lucide icon must parse")
}

fn icing_product_items() -> Vec<DropDownItem> {
    [
        "LWC",
        "Time",
        "IC指数",
        "IC改进",
        "假霜点温度",
        "积冰指数",
        "XGB1",
        "XGB2",
        "XGB3",
        "XGB4",
    ]
    .into_iter()
    .map(|name| DropDownItem::new(name, name))
    .collect()
}

fn icing_product_drop_down_style() -> DropDownStyle {
    DropDownStyle {
        root: StyleOf::new().with::<container::size>(Size::fix(158.0, 30.0)),
        trigger: StyleOf::new()
            .with::<container::size>(Size::fix(158.0, 30.0))
            .with::<container::padding>(EdgeInsets::symmetric(12.0, 4.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::background>(CONTROL)
            .with::<container::color>(TEXT)
            .with::<container::font_size>(13.0)
            .with::<container::font_weight>(FontWeight::Medium)
            .with::<container::border_color>(Color::hex("#55555A"))
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(4.0)
            .when_style(
                WidgetStateMatcher::all(WidgetState::HOVERED),
                StyleOf::<Container>::new().with::<container::background>(Color::hex("#343438")),
            )
            .when_style(
                WidgetStateMatcher::all(WidgetState::FOCUSED),
                StyleOf::<Container>::new().with::<container::border_color>(ACCENT),
            ),
        trigger_open: StyleOf::new().with::<container::border_color>(ACCENT),
        menu: StyleOf::new()
            .with::<container::padding>(EdgeInsets::all(4.0))
            .with::<container::background>(Color::hex("#1A1A1C"))
            .with::<container::border_color>(Color::hex("#55555A"))
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(4.0)
            .with::<container::max_height>(280.0)
            .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical),
        option: StyleOf::new()
            .with::<container::padding>(EdgeInsets::symmetric(10.0, 7.0))
            .with::<container::color>(TEXT)
            .with::<container::font_size>(12.0)
            .with::<container::border_radius>(3.0)
            .when_style(
                WidgetStateMatcher::all(WidgetState::HOVERED),
                StyleOf::<Container>::new().with::<container::background>(CONTROL),
            ),
        selected_option: StyleOf::new()
            .with::<container::padding>(EdgeInsets::symmetric(10.0, 7.0))
            .with::<container::background>(Color::hex("#253526"))
            .with::<container::color>(ACCENT)
            .with::<container::font_size>(12.0)
            .with::<container::font_weight>(FontWeight::Bold)
            .with::<container::border_radius>(3.0),
        disabled_option: StyleOf::new()
            .with::<container::padding>(EdgeInsets::symmetric(10.0, 7.0))
            .with::<container::color>(DIM)
            .with::<container::font_size>(12.0)
            .with::<container::border_radius>(3.0),
    }
}

// ---- building blocks -------------------------------------------------------

/// A box laying its children out in a row.
#[component]
fn hbox(style: &StyleOf<Container>, children: &Children) {
    view! {
        container(style: style.clone(), flex_direction: FlexDirectionStyle::Row) {
            slot(children)
        }
    }
}

/// A box laying its children out in a column.
#[component]
fn vbox(style: &StyleOf<Container>, children: &Children) {
    view! {
        container(style: style.clone(), flex_direction: FlexDirectionStyle::Column) {
            slot(children)
        }
    }
}

/// `gap` between children, centered on the cross axis.
fn aligned(gap: f32) -> StyleOf<Container> {
    StyleOf::new().with::<container::gap>(gap).with::<container::align>(AlignStyle::Center)
}

/// A 1px horizontal rule.
fn rule_style() -> StyleOf<Container> {
    StyleOf::new()
        .with::<container::height>(1.0)
        .with::<container::width>(Sizing::fill())
        .with::<container::background>(BORDER)
}

/// A bordered panel.
fn bordered(background: Color) -> StyleOf<Container> {
    StyleOf::new()
        .with::<container::background>(background)
        .with::<container::border_color>(BORDER)
        .with::<container::border_width>(1.0)
        .with::<container::border_radius>(3.0)
}

#[component]
fn label(value: &String, size: &f32, color: &Color, weight: &FontWeight) {
    let mut paragraph: ParagraphStyle = ParagraphStyle::default();
    paragraph.vertical_align = TextVerticalAlign::Middle;
    view! {
        text(
            value.clone(),
            style: StyleOf::new()
                .with::<container::color>(*color)
                .with::<container::font_family>("Microsoft YaHei")
                .with::<container::font_size>(*size)
                .with::<container::font_weight>(*weight),
            paragraph: paragraph.clone(),
        )
    }
}

#[component]
fn icon_view(kind: &SourceIcon, size: &f32, color: &Color) {
    view! {
        icon(
            data: source_icon(*kind),
            color: *color,
            style: StyleOf::new().with::<container::size>(Size::fix(*size, *size)),
        )
    }
}

#[component]
fn dot(color: &Color) {
    view! {
        container(
            style: StyleOf::new()
                .with::<container::size>(Size::fix(7.0, 7.0))
                .with::<container::background>(*color)
                .with::<container::border_radius>(4.0),
        )
    }
}

#[component]
fn icon_button(kind: &SourceIcon) {
    view! {
        hbox(
            style: StyleOf::new()
                .with::<container::size>(Size::fix(26.0, 26.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::Center)
                .with::<container::border_radius>(3.0),
        ) {
            icon_view(kind: *kind, size: 14.0, color: MUTED)
        }
    }
}

#[component]
fn component_button(text: &String, active: &bool) {
    view! {
        hbox(style: StyleOf::new().with::<container::height>(26.0).with::<container::font_size>(11.0)) {
            button(
                text: text.clone(),
                variant: if *active { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                size: ButtonSize::Small,
                style: StyleOf::new()
                    .with::<container::min_height>(26.0)
                    .with::<container::padding>(EdgeInsets::symmetric(8.0, 4.0)),
            )
        }
    }
}

#[component]
fn legend_item(color: &Color, text: &String) {
    view! {
        hbox(style: aligned(5.0)) {
            dot(color: *color)
            label(value: text.clone(), size: 11.0, color: MUTED, weight: FontWeight::Normal)
        }
    }
}

#[component]
fn chart_card(title: &String, primary: &bool) {
    view! {
        vbox(
            style: bordered(PANEL)
                .with::<container::width>(Sizing::fill())
                .with::<container::height>(232.0)
                .with::<container::padding>(EdgeInsets::all(12.0))
                .with::<container::gap>(9.0)
                .with::<container::border_color>(if *primary { Color::WHITE } else { BORDER }),
        ) {
            hbox(
                style: StyleOf::new()
                    .with::<container::height>(30.0)
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween),
            ) {
                hbox(style: aligned(8.0)) {
                    dot(color: if *primary { GREEN } else { Color::hex("#444448") })
                    label(value: title.clone(), size: 16.0, color: TEXT, weight: FontWeight::Bold)
                }
                hbox(style: aligned(5.0)) {
                    component_button(text: "散点图", active: false)
                    component_button(text: "配置参数", active: false)
                    component_button(text: "地图映射", active: *primary)
                }
            }
            container(style: rule_style())
            container(
                style: bordered(PAGE).with::<container::width>(Sizing::fill()).with::<container::height>(124.0),
            )
            hbox(
                style: StyleOf::new()
                    .with::<container::height>(28.0)
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween),
            ) {
                hbox(style: aligned(10.0)) {
                    legend_item(color: Color::hex("#6B7280"), text: if title == "Time" { "无风险" } else { "无" })
                    legend_item(color: BLUE, text: "轻度")
                    legend_item(color: ACCENT, text: "中度")
                    legend_item(color: RED, text: "重度")
                }
                component_button(text: "导出", active: false)
            }
        }
    }
}

#[component]
fn map_guide() {
    view! {
        vbox(
            style: bordered(Color::hex("#121214").alpha(0.96))
                .with::<container::width>(270.0)
                .with::<container::padding>(EdgeInsets::all(12.0))
                .with::<container::gap>(7.0),
        ) {
            label(value: "地图操作指南", size: 13.0, color: TEXT, weight: FontWeight::Bold)
            label(value: "• 拖拽地图可平移，滚轮缩放", size: 11.0, color: MUTED, weight: FontWeight::Normal)
            label(value: "• 点击航迹点可锁定并查看详情", size: 11.0, color: MUTED, weight: FontWeight::Normal)
            label(value: "• 虚线部分代表预估/未飞行航线", size: 11.0, color: MUTED, weight: FontWeight::Normal)
            container(style: rule_style())
            label(value: "白色实线箭头: 预估可航行距离", size: 11.0, color: MUTED, weight: FontWeight::SemiBold)
        }
    }
}

#[component]
fn flight_selector() {
    view! {
        vbox(
            style: bordered(Color::hex("#121214").alpha(0.96))
                .with::<container::width>(260.0)
                .with::<container::padding>(EdgeInsets::all(12.0))
                .with::<container::gap>(10.0),
        ) {
            hbox(style: bordered(PAGE).with::<container::height>(34.0).with::<container::padding>(EdgeInsets::all(4.0))) {
                hbox(
                    style: aligned(5.0)
                        .with::<container::width>(Sizing::percent(0.5))
                        .with::<container::height>(Sizing::fill())
                        .with::<container::justify>(JustifyStyle::Center)
                        .with::<container::background>(CONTROL),
                ) {
                    dot(color: RED)
                    label(value: "实况", size: 11.0, color: ACCENT, weight: FontWeight::Bold)
                }
                hbox(
                    style: StyleOf::new()
                        .with::<container::width>(Sizing::fill())
                        .with::<container::height>(Sizing::fill())
                        .with::<container::align>(AlignStyle::Center)
                        .with::<container::justify>(JustifyStyle::Center),
                ) {
                    label(value: "历史", size: 11.0, color: MUTED, weight: FontWeight::Bold)
                }
            }
            hbox(
                style: bordered(PAGE)
                    .with::<container::height>(44.0)
                    .with::<container::padding>(EdgeInsets::symmetric(12.0, 6.0))
                    .with::<container::align>(AlignStyle::Center),
            ) {
                label(value: "20260702_B651N_01", size: 14.0, color: ACCENT, weight: FontWeight::Bold)
            }
        }
    }
}

#[component]
fn map_panel(#[source] selected_product: &usize, on_product_change: &Callback<usize>) {
    view! {
        vbox(
            style: bordered(PANEL)
                .with::<container::width>(Sizing::percent(0.4))
                .with::<container::height>(Sizing::fill())
                .with::<container::clip>(true),
        ) {
            hbox(
                style: StyleOf::new()
                    .with::<container::height>(42.0)
                    .with::<container::padding>(EdgeInsets::symmetric(12.0, 6.0))
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween)
                    .with::<container::background>(SURFACE),
            ) {
                hbox(style: aligned(9.0)) {
                    label(value: "当前积冰监测产品:", size: 12.0, color: MUTED, weight: FontWeight::SemiBold)
                    drop_down(
                        items: icing_product_items(),
                        selected: Some(*selected_product),
                        on_change: on_product_change,
                        style: icing_product_drop_down_style(),
                        id_prefix: "icing-product",
                        z_index: 2000,
                    )
                }
                hbox(style: aligned(2.0)) {
                    icon_button(kind: SourceIcon::Crosshair)
                    icon_button(kind: SourceIcon::ZoomIn)
                    icon_button(kind: SourceIcon::ZoomOut)
                    icon_button(kind: SourceIcon::RotateCcw)
                }
            }
            z_stack(
                alignment: Alignment::TOP_LEADING,
                style: StyleOf::new()
                    .with::<container::size>(Size::fill())
                    .with::<container::background>(PAGE)
                    .with::<container::clip>(true),
            ) {
                container(style: StyleOf::new().with::<container::size>(Size::fill()).with::<container::background>(PAGE))
                hbox(
                    style: StyleOf::new()
                        .with::<container::width>(Sizing::fill())
                        .with::<container::padding>(EdgeInsets::all(16.0))
                        .with::<container::gap>(14.0)
                        .with::<container::justify>(JustifyStyle::SpaceBetween)
                        .with::<container::align>(AlignStyle::Start),
                ) {
                    map_guide()
                    flight_selector()
                }
            }
            hbox(
                style: StyleOf::new()
                    .with::<container::height>(31.0)
                    .with::<container::padding>(EdgeInsets::symmetric(12.0, 5.0))
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween)
                    .with::<container::background>(SURFACE),
            ) {
                hbox(style: aligned(15.0)) {
                    label(value: "图区中心: 35.5000°N, 119.0000°E", size: 10.0, color: DIM, weight: FontWeight::Normal)
                    label(value: "缩放比例: 1.10x", size: 10.0, color: DIM, weight: FontWeight::Normal)
                    label(value: "光标指向: ---, ---", size: 10.0, color: DIM, weight: FontWeight::Normal)
                }
                hbox(style: aligned(6.0)) {
                    label(value: "系统连线状态: 正常", size: 10.0, color: MUTED, weight: FontWeight::Normal)
                    dot(color: GREEN)
                }
            }
        }
    }
}

/// The ten algorithm products, in two columns.
#[component]
fn icing_monitor() {
    view! {
        vbox(
            style: StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::height>(Sizing::fill())
                .with::<container::padding>(EdgeInsets::new(0.0, 0.0, 12.0, 0.0))
                .with::<container::gap>(10.0),
        ) {
            hbox(
                style: StyleOf::new()
                    .with::<container::height>(24.0)
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween),
            ) {
                label(value: "积冰算法产品", size: 12.0, color: TEXT, weight: FontWeight::SemiBold)
                label(value: "10 项算法产品", size: 10.0, color: DIM, weight: FontWeight::Normal)
            }
            container(style: rule_style())
            vbox(
                style: StyleOf::new()
                    .with::<container::width>(Sizing::fill())
                    .with::<container::height>(Sizing::fill())
                    .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical)
                    .with::<container::scrollbar_width>(5.0)
                    .with::<container::scrollbar_track_color>(Color::TRANSPARENT)
                    .with::<container::scrollbar_thumb_color>(Color::hex("#3A3A3D")),
            ) {
                hbox(
                    style: StyleOf::new()
                        .with::<container::width>(Sizing::fill())
                        .with::<container::gap>(12.0)
                        .with::<container::align>(AlignStyle::Start),
                ) {
                    vbox(style: StyleOf::new().with::<container::width>(Sizing::fill()).with::<container::gap>(12.0)) {
                        chart_card(title: "LWC", primary: true)
                        chart_card(title: "IC指数", primary: false)
                        chart_card(title: "假霜点温度", primary: false)
                        chart_card(title: "XGB1", primary: false)
                        chart_card(title: "XGB3", primary: false)
                    }
                    vbox(style: StyleOf::new().with::<container::width>(Sizing::fill()).with::<container::gap>(12.0)) {
                        chart_card(title: "Time", primary: false)
                        chart_card(title: "IC改进", primary: false)
                        chart_card(title: "积冰指数", primary: false)
                        chart_card(title: "XGB2", primary: false)
                        chart_card(title: "XGB4", primary: false)
                    }
                }
            }
        }
    }
}

#[component]
fn metric_card(title: &String, value: &String, unit: &String) {
    view! {
        vbox(
            style: bordered(PANEL)
                .with::<container::width>(Sizing::fill())
                .with::<container::height>(112.0)
                .with::<container::padding>(EdgeInsets::all(14.0))
                .with::<container::gap>(9.0),
        ) {
            label(value: title.clone(), size: 12.0, color: MUTED, weight: FontWeight::Medium)
            hbox(style: StyleOf::new().with::<container::gap>(8.0).with::<container::align>(AlignStyle::End)) {
                label(value: value.clone(), size: 28.0, color: TEXT, weight: FontWeight::Bold)
                label(value: unit.clone(), size: 11.0, color: MUTED, weight: FontWeight::Normal)
            }
        }
    }
}

/// The flight's live metrics and its profile.
#[component]
fn flight_monitor() {
    view! {
        vbox(style: StyleOf::new().with::<container::size>(Size::fill()).with::<container::gap>(12.0)) {
            grid(columns: GridTracks::adaptive(200.0), width: Sizing::fill(), gap: 12.0) {
                metric_card(title: "当前飞行高度 (ALT)", value: "7462", unit: "米 (m)")
                metric_card(title: "当前真空速 (TAS)", value: "440", unit: "公里/小时")
                metric_card(title: "磁航向 (HDG)", value: "213°", unit: "导航姿态稳定")
                metric_card(title: "当前垂直速度 (V/S)", value: "+3.2", unit: "米/秒")
            }
            vbox(
                style: bordered(PANEL)
                    .with::<container::size>(Size::fill())
                    .with::<container::padding>(EdgeInsets::all(14.0))
                    .with::<container::gap>(10.0),
            ) {
                label(value: "全航程高度 / 真空速 实时剖面趋势图", size: 15.0, color: TEXT, weight: FontWeight::Bold)
                container(style: bordered(PAGE).with::<container::size>(Size::fill()))
            }
        }
    }
}

#[component]
fn playback_button(kind: &SourceIcon) {
    view! {
        hbox(
            style: bordered(SURFACE)
                .with::<container::size>(Size::fix(28.0, 28.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::Center),
        ) {
            icon_view(kind: *kind, size: 13.0, color: MUTED)
        }
    }
}

#[component]
fn select_box(value: &String, width: &f32) {
    view! {
        hbox(
            style: bordered(PAGE)
                .with::<container::size>(Size::fix(*width, 27.0))
                .with::<container::padding>(EdgeInsets::symmetric(9.0, 4.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::SpaceBetween),
        ) {
            label(value: value.clone(), size: 11.0, color: TEXT, weight: FontWeight::Medium)
        }
    }
}

#[component]
fn timeline() {
    view! {
        vbox(
            style: StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::height>(104.0)
                .with::<container::padding>(EdgeInsets::symmetric(16.0, 9.0))
                .with::<container::gap>(7.0)
                .with::<container::background>(PAGE)
                .with::<container::border_color>(BORDER)
                .with::<container::border_width>(1.0),
        ) {
            hbox(style: aligned(7.0).with::<container::height>(32.0)) {
                playback_button(kind: SourceIcon::Previous)
                playback_button(kind: SourceIcon::Play)
                playback_button(kind: SourceIcon::Next)
                label(value: "倍速:", size: 11.0, color: MUTED, weight: FontWeight::Normal)
                select_box(value: "1x", width: 62.0)
                label(value: "间隔:", size: 11.0, color: MUTED, weight: FontWeight::Normal)
                select_box(value: "1s", width: 70.0)
                label(value: "12:24:00", size: 12.0, color: TEXT, weight: FontWeight::Bold)
            }
            hbox(
                style: StyleOf::new()
                    .with::<container::width>(Sizing::fill())
                    .with::<container::height>(42.0)
                    .with::<container::padding>(EdgeInsets::symmetric(14.0, 7.0))
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::background>(Color::hex("#2B2112"))
                    .with::<container::border_color>(Color::hex("#6D4D20"))
                    .with::<container::border_width>(1.0)
                    .with::<container::border_radius>(3.0),
            ) {
                hbox(
                    style: StyleOf::new()
                        .with::<container::width>(Sizing::fill())
                        .with::<container::align>(AlignStyle::Center)
                        .with::<container::justify>(JustifyStyle::SpaceBetween),
                ) {
                    for time in ["12:00", "12:05", "12:10", "12:15", "12:20", "12:24"] {
                        label(value: *time, size: 9.0, color: MUTED, weight: FontWeight::Normal)
                    }
                }
            }
        }
    }
}

#[component]
fn status_bar() {
    view! {
        hbox(
            style: StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::height>(30.0)
                .with::<container::padding>(EdgeInsets::symmetric(16.0, 5.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::SpaceBetween)
                .with::<container::background>(SURFACE)
                .with::<container::border_color>(BORDER)
                .with::<container::border_width>(1.0),
        ) {
            hbox(style: aligned(18.0)) {
                label(value: "演示系统状态: 运行正常", size: 10.0, color: MUTED, weight: FontWeight::Normal)
                label(value: "数据源: 动态气象仿真数据", size: 10.0, color: MUTED, weight: FontWeight::Normal)
                label(value: "数据更新情况: 正常", size: 10.0, color: MUTED, weight: FontWeight::Normal)
            }
            hbox(style: aligned(6.0)) {
                dot(color: GREEN)
                label(value: "ONLINE", size: 10.0, color: MUTED, weight: FontWeight::Bold)
            }
        }
    }
}

fn tab_style() -> TabsStyle {
    TabsStyle {
        root: StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(Sizing::fill())
            .with::<container::padding>(EdgeInsets::all(16.0))
            .with::<container::gap>(12.0)
            .with::<container::background>(PANEL)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0)
            .with::<container::clip>(true),
        list: StyleOf::new()
            .with::<container::gap>(3.0)
            .with::<container::padding>(EdgeInsets::all(4.0))
            .with::<container::background>(Color::hex("#17171A"))
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        tab: StyleOf::new()
            .with::<container::padding>(EdgeInsets::symmetric(16.0, 6.0))
            .with::<container::color>(MUTED)
            .with::<container::font_size>(12.0)
            .with::<container::border_color>(Color::TRANSPARENT)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(2.0),
        selected_tab: StyleOf::new()
            .with::<container::padding>(EdgeInsets::symmetric(16.0, 6.0))
            .with::<container::background>(CONTROL)
            .with::<container::color>(ACCENT)
            .with::<container::font_size>(12.0)
            .with::<container::font_weight>(FontWeight::Bold)
            .with::<container::border_color>(Color::TRANSPARENT)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(2.0),
        disabled_tab: StyleOf::new()
            .with::<container::padding>(EdgeInsets::symmetric(16.0, 6.0))
            .with::<container::color>(DIM),
        panel: StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(Sizing::fill())
            .with::<container::padding>(EdgeInsets::zero()),
    }
}

#[component]
pub fn flight_icing_dashboard() {
    let selected_tab: usize = state!(1);
    let selected_product: usize = state!(0);
    let items: Vec<TabItem> = vec![
        TabItem::new("flight", "飞行监测", Children::component::<FlightMonitor>(FlightMonitorProps {})),
        TabItem::new("icing", "积冰监测", Children::component::<IcingMonitor>(IcingMonitorProps {})),
    ];
    view! {
        vbox(
            style: StyleOf::new()
                .with::<container::size>(Size::fill())
                .with::<container::padding>(EdgeInsets::all(16.0).set_top(48.0))
                .with::<container::gap>(10.0)
                .with::<container::background>(PAGE)
                .with::<container::color>(TEXT)
                .with::<container::font_family>("Hiragino Sans GB")
                .with::<container::clip>(true),
        ) {
            hbox(
                style: StyleOf::new()
                    .with::<container::width>(Sizing::fill())
                    .with::<container::height>(714.0)
                    .with::<container::gap>(16.0),
            ) {
                map_panel(selected_product: selected_product, on_product_change: |cx, index| selected_product.set(cx, index))
                tabs(
                    items: items.clone(),
                    selected: Some(*selected_tab),
                    on_change: |cx, index| selected_tab.set(cx, index),
                    style: tab_style(),
                    id_prefix: "flight-icing-monitor",
                )
            }
            timeline()
            status_bar()
        }
    }
}

/// What the app starts with: the dashboard, at the root.
pub fn dashboard_root(app: &mut App) {
    app.mount::<FlightIcingDashboard>(FlightIcingDashboardProps {});
}

#[cfg(all(test, feature = "skia"))]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::BufWriter;
    use xui_core::text::TextHost;
    use xui_cosmic::CosmicEngine;
    use xui_skia::{SkiaBackend, SkiaBackendOptions};

    #[test]
    fn renders_dashboard_headlessly() {
        let mut app = App::new(dashboard_root);
        app.resize(Size::new(1600.0, 900.0));
        let mut backend = SkiaBackend::<CosmicEngine>::headless(
            1.0,
            SkiaBackendOptions {
                clear_color: PAGE,
                ..SkiaBackendOptions::default()
            },
        );
        let mut text = TextHost::new(CosmicEngine::new(1.0));
        for _ in 0..16 {
            if !app.is_dirty() {
                break;
            }
            app.render(&mut backend, &mut text)
                .expect("dashboard should render");
        }
        assert!(!app.is_dirty(), "dashboard should finish rebuilding");
        let pixels = backend
            .read_pixels_rgba8()
            .expect("dashboard pixels should be readable");
        assert_eq!(pixels.len(), 1600 * 900 * 4);

        if let Ok(path) = std::env::var("XUI_SNAPSHOT_PATH") {
            let file = File::create(path).expect("snapshot output should be creatable");
            let writer = BufWriter::new(file);
            let mut encoder = png::Encoder::new(writer, 1600, 900);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut png = encoder.write_header().expect("PNG header should encode");
            png.write_image_data(&pixels)
                .expect("dashboard PNG should encode");
        }
    }

    /// Capture each resize step into a PNG so we can visually inspect what
    /// flickers. Set XUI_FLICKER_DIR to a directory and run with
    /// `cargo test --package xui-example-app -- --nocapture --ignored resize_flicker_capture`.
    #[test]
    #[ignore = "writes PNGs into XUI_FLICKER_DIR; run explicitly"]
    fn resize_flicker_capture() {
        let dir = std::env::var("XUI_FLICKER_DIR").expect("set XUI_FLICKER_DIR");
        let mut app = App::new(dashboard_root);
        let mut backend = SkiaBackend::<CosmicEngine>::headless(
            1.0,
            SkiaBackendOptions {
                clear_color: PAGE,
                ..SkiaBackendOptions::default()
            },
        );
        let mut text = TextHost::new(CosmicEngine::new(1.0));

        let render_at = |w: u32,
                         app: &mut App,
                         backend: &mut SkiaBackend<CosmicEngine>,
                         text: &mut TextHost<CosmicEngine>|
         -> Vec<u8> {
            app.resize(Size::new(w as f32, 900.0));
            for _ in 0..16 {
                if !app.is_dirty() {
                    break;
                }
                app.render(backend, text).expect("render");
            }
            app.render(backend, text).expect("final render");
            backend.read_pixels_rgba8().expect("pixels")
        };

        // Render exactly once per size (simulating 60Hz resize events),
        // sweeping sub-pixel widths around a baseline.
        let mut prev: Option<(usize, Vec<u8>)> = None;
        let mut report = Vec::new();
        for step in -40i32..=40 {
            let logical_w = 1600.0 + step as f32 * 0.25;
            app.resize(Size::new(logical_w, 900.0));
            // single render per resize event, no settle loop
            app.render(&mut backend, &mut text).expect("render");
            let pixels = backend.read_pixels_rgba8().expect("pixels");
            let phys_w = pixels.len() / (900 * 4);
            let path = format!("{dir}/subpx_{:03}.png", step + 40);
            let file = File::create(&path).expect("create");
            let writer = BufWriter::new(file);
            let mut encoder = png::Encoder::new(writer, phys_w as u32, 900);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut png = encoder.write_header().expect("header");
            png.write_image_data(&pixels).expect("encode");
            if let Some((prev_w, prev_px)) = &prev {
                let w = phys_w.min(*prev_w);
                let mut diff_count = 0usize;
                for i in 0..(w * 900) {
                    let o = i * 4;
                    if &pixels[o..o + 4] != &prev_px[o..o + 4] {
                        diff_count += 1;
                    }
                }
                let st = backend.frame_stats();
                report.push((step, logical_w, diff_count));
                eprintln!(
                    "step={step:+3} logical_w={logical_w:>8.2} phys_w={phys_w} diff_in_overlap={diff_count} root_damage_rects={} root_damage_area_sum={:.0}",
                    st.root_damage_rects, st.root_damage_area_sum
                );
            }
            prev = Some((phys_w, pixels));
        }
        // detect oscillation: a label flickers if diff_count alternates high/low/high
        let diffs: Vec<usize> = report.iter().map(|(_, _, d)| *d).collect();
        eprintln!("diff series: {:?}", diffs);
    }
}

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use xui_core::text::TextHost;
    use xui_cosmic::CosmicEngine;

    #[test]
    fn dashboard_builds_and_resizes_its_scene() {
        let mut app = App::new(dashboard_root);
        let mut backend = MockRenderBackend::default();
        let mut text = TextHost::new(CosmicEngine::new(1.0));
        for width in [1600.0, 1100.0, 1600.0] {
            app.resize(Size::new(width, 900.0));
            app.render(&mut backend, &mut text).expect("dashboard scene");
            let tree = app.ui_runtime();
            let mut pending = vec![tree.root()];
            let mut count = 0;
            while let Some(id) = pending.pop() {
                let node = tree.node(id).unwrap();
                assert!(node.layout.width().is_finite());
                assert!(node.layout.height().is_finite());
                pending.extend(tree.children(id));
                count += 1;
            }
            assert!(count > 30);
        }
    }
}
