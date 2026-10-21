use xui_components::*;
use xui_core::prelude::*;
use xui_macros::{component, xui};

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

#[derive(Clone, Copy)]
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

fn box_element(
    direction: FlexDirectionStyle,
    style: StyleOf<Container>,
    children: Vec<ElementDesc>,
) -> ElementDesc {
    container()
        .with::<container::flex_direction>(direction)
        .with::<container::style>(style)
        .into_element_desc(children)
}

fn row(style: StyleOf<Container>, children: Vec<ElementDesc>) -> ElementDesc {
    box_element(FlexDirectionStyle::Row, style, children)
}

fn column(style: StyleOf<Container>, children: Vec<ElementDesc>) -> ElementDesc {
    box_element(FlexDirectionStyle::Column, style, children)
}

fn empty(style: StyleOf<Container>) -> ElementDesc {
    container()
        .with::<container::style>(style)
        .into_element_desc(Vec::new())
}

fn label(value: impl Into<String>, size: f32, color: Color, weight: FontWeight) -> ElementDesc {
    let mut paragraph = ParagraphStyle::default();
    paragraph.vertical_align = TextVerticalAlign::Middle;
    text()
        .with::<text::content>(value.into())
        .with::<text::style>(
            StyleOf::new()
                .with::<container::color>(color)
                .with::<container::font_family>("Microsoft YaHei")
                .with::<container::font_size>(size)
                .with::<container::font_weight>(weight),
        )
        .with::<text::paragraph>(paragraph)
        .into_element_desc(Vec::new())
}

fn icon_view(kind: SourceIcon, size: f32, color: Color) -> ElementDesc {
    icon()
        .with::<icon::data>(source_icon(kind))
        .with::<icon::color>(color)
        .with::<icon::style>(StyleOf::new().with::<container::size>(Size::fix(size, size)))
        .into_element_desc(Vec::new())
}

fn dot(color: Color) -> ElementDesc {
    empty(
        StyleOf::new()
            .with::<container::size>(Size::fix(7.0, 7.0))
            .with::<container::background>(color)
            .with::<container::border_radius>(4.0),
    )
}

fn icon_button(kind: SourceIcon) -> ElementDesc {
    row(
        StyleOf::new()
            .with::<container::size>(Size::fix(26.0, 26.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::Center)
            .with::<container::border_radius>(3.0),
        vec![icon_view(kind, 14.0, MUTED)],
    )
}

fn component_button(text_value: &str, active: bool) -> ElementDesc {
    let variant = if active {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Secondary
    };
    let text_value = text_value.to_string();
    row(
        StyleOf::new()
            .with::<container::height>(26.0)
            .with::<container::font_size>(11.0),
        vec![xui! {
            <button
                text={text_value}
                variant={variant}
                size={ButtonSize::Small}
                style={StyleOf::new()
                    .with::<container::min_height>(26.0)
                    .with::<container::padding>(EdgeInsets::symmetric(8.0, 4.0))}
            />
        }],
    )
}

fn legend_item(color: Color, text_value: &str) -> ElementDesc {
    row(
        StyleOf::new()
            .with::<container::gap>(5.0)
            .with::<container::align>(AlignStyle::Center),
        vec![
            dot(color),
            label(text_value, 11.0, MUTED, FontWeight::Normal),
        ],
    )
}

fn chart_card(title: &str, primary: bool) -> ElementDesc {
    let legend_none = if title == "Time" { "无风险" } else { "无" };
    let chart_placeholder = empty(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(124.0)
            .with::<container::background>(PAGE)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
    );

    let header = row(
        StyleOf::new()
            .with::<container::height>(30.0)
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::SpaceBetween),
        vec![
            row(
                StyleOf::new()
                    .with::<container::gap>(8.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    dot(if primary {
                        GREEN
                    } else {
                        Color::hex("#444448")
                    }),
                    label(title, 16.0, TEXT, FontWeight::Bold),
                ],
            ),
            row(
                StyleOf::new()
                    .with::<container::gap>(5.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    component_button("散点图", false),
                    component_button("配置参数", false),
                    component_button("地图映射", primary),
                ],
            ),
        ],
    );

    let footer = row(
        StyleOf::new()
            .with::<container::height>(28.0)
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::SpaceBetween),
        vec![
            row(
                StyleOf::new()
                    .with::<container::gap>(10.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    legend_item(Color::hex("#6B7280"), legend_none),
                    legend_item(BLUE, "轻度"),
                    legend_item(ACCENT, "中度"),
                    legend_item(RED, "重度"),
                ],
            ),
            component_button("导出", false),
        ],
    );

    column(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(232.0)
            .with::<container::padding>(EdgeInsets::all(12.0))
            .with::<container::gap>(9.0)
            .with::<container::background>(PANEL)
            .with::<container::border_color>(if primary { Color::WHITE } else { BORDER })
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        vec![
            header,
            empty(
                StyleOf::new()
                    .with::<container::height>(1.0)
                    .with::<container::width>(Sizing::fill())
                    .with::<container::background>(BORDER),
            ),
            chart_placeholder,
            footer,
        ],
    )
}

fn map_guide() -> ElementDesc {
    column(
        StyleOf::new()
            .with::<container::width>(270.0)
            .with::<container::padding>(EdgeInsets::all(12.0))
            .with::<container::gap>(7.0)
            .with::<container::background>(Color::hex("#121214").alpha(0.96))
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        vec![
            label("地图操作指南", 13.0, TEXT, FontWeight::Bold),
            label(
                "• 拖拽地图可平移，滚轮缩放",
                11.0,
                MUTED,
                FontWeight::Normal,
            ),
            label(
                "• 点击航迹点可锁定并查看详情",
                11.0,
                MUTED,
                FontWeight::Normal,
            ),
            label(
                "• 虚线部分代表预估/未飞行航线",
                11.0,
                MUTED,
                FontWeight::Normal,
            ),
            empty(
                StyleOf::new()
                    .with::<container::height>(1.0)
                    .with::<container::width>(Sizing::fill())
                    .with::<container::background>(BORDER),
            ),
            label(
                "白色实线箭头: 预估可航行距离",
                11.0,
                MUTED,
                FontWeight::SemiBold,
            ),
        ],
    )
}

fn flight_selector() -> ElementDesc {
    column(
        StyleOf::new()
            .with::<container::width>(260.0)
            .with::<container::padding>(EdgeInsets::all(12.0))
            .with::<container::gap>(10.0)
            .with::<container::background>(Color::hex("#121214").alpha(0.96))
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        vec![
            row(
                StyleOf::new()
                    .with::<container::height>(34.0)
                    .with::<container::padding>(EdgeInsets::all(4.0))
                    .with::<container::background>(PAGE)
                    .with::<container::border_color>(BORDER)
                    .with::<container::border_width>(1.0)
                    .with::<container::border_radius>(3.0),
                vec![
                    row(
                        StyleOf::new()
                            .with::<container::width>(Sizing::percent(0.5))
                            .with::<container::height>(Sizing::fill())
                            .with::<container::gap>(5.0)
                            .with::<container::align>(AlignStyle::Center)
                            .with::<container::justify>(JustifyStyle::Center)
                            .with::<container::background>(CONTROL),
                        vec![dot(RED), label("实况", 11.0, ACCENT, FontWeight::Bold)],
                    ),
                    row(
                        StyleOf::new()
                            .with::<container::width>(Sizing::fill())
                            .with::<container::height>(Sizing::fill())
                            .with::<container::align>(AlignStyle::Center)
                            .with::<container::justify>(JustifyStyle::Center),
                        vec![label("历史", 11.0, MUTED, FontWeight::Bold)],
                    ),
                ],
            ),
            row(
                StyleOf::new()
                    .with::<container::height>(44.0)
                    .with::<container::padding>(EdgeInsets::symmetric(12.0, 6.0))
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::background>(PAGE)
                    .with::<container::border_color>(BORDER)
                    .with::<container::border_width>(1.0)
                    .with::<container::border_radius>(3.0),
                vec![label("20260702_B651N_01", 14.0, ACCENT, FontWeight::Bold)],
            ),
        ],
    )
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

fn map_panel(selected_product: usize, on_product_change: DropDownChangeCallback) -> ElementDesc {
    let product_selector = xui! {
        <drop_down
            items={icing_product_items()}
            selected={Some(selected_product)}
            on_change={Some(on_product_change)}
            style={icing_product_drop_down_style()}
            id_prefix={"icing-product".to_string()}
            z_index={2000}
        />
    };
    let toolbar = row(
        StyleOf::new()
            .with::<container::height>(42.0)
            .with::<container::padding>(EdgeInsets::symmetric(12.0, 6.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::SpaceBetween)
            .with::<container::background>(SURFACE),
        vec![
            row(
                StyleOf::new()
                    .with::<container::gap>(9.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    label("当前积冰监测产品:", 12.0, MUTED, FontWeight::SemiBold),
                    product_selector,
                ],
            ),
            row(
                StyleOf::new()
                    .with::<container::gap>(2.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    icon_button(SourceIcon::Crosshair),
                    icon_button(SourceIcon::ZoomIn),
                    icon_button(SourceIcon::ZoomOut),
                    icon_button(SourceIcon::RotateCcw),
                ],
            ),
        ],
    );

    let map_placeholder = empty(
        StyleOf::new()
            .with::<container::size>(Size::fill())
            .with::<container::background>(PAGE),
    );
    let overlay = row(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::padding>(EdgeInsets::all(16.0))
            .with::<container::gap>(14.0)
            .with::<container::justify>(JustifyStyle::SpaceBetween)
            .with::<container::align>(AlignStyle::Start),
        vec![map_guide(), flight_selector()],
    );
    let map = z_stack()
        .with::<z_stack::alignment>(Alignment::TOP_LEADING)
        .with::<z_stack::style>(
            StyleOf::new()
                .with::<container::size>(Size::fill())
                .with::<container::background>(PAGE)
                .with::<container::clip>(true),
        )
        .into_element_desc(vec![map_placeholder, overlay]);

    let footer = row(
        StyleOf::new()
            .with::<container::height>(31.0)
            .with::<container::padding>(EdgeInsets::symmetric(12.0, 5.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::SpaceBetween)
            .with::<container::background>(SURFACE),
        vec![
            row(
                StyleOf::new()
                    .with::<container::gap>(15.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    label(
                        "图区中心: 35.5000°N, 119.0000°E",
                        10.0,
                        DIM,
                        FontWeight::Normal,
                    ),
                    label("缩放比例: 1.10x", 10.0, DIM, FontWeight::Normal),
                    label("光标指向: ---, ---", 10.0, DIM, FontWeight::Normal),
                ],
            ),
            row(
                StyleOf::new()
                    .with::<container::gap>(6.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    label("系统连线状态: 正常", 10.0, MUTED, FontWeight::Normal),
                    dot(GREEN),
                ],
            ),
        ],
    );

    column(
        StyleOf::new()
            .with::<container::width>(Sizing::percent(0.4))
            .with::<container::height>(Sizing::fill())
            .with::<container::background>(PANEL)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0)
            .with::<container::clip>(true),
        vec![toolbar, map, footer],
    )
}

fn icing_monitor_content() -> ElementDesc {
    let left_cards = column(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::gap>(12.0),
        vec![
            chart_card("LWC", true),
            chart_card("IC指数", false),
            chart_card("假霜点温度", false),
            chart_card("XGB1", false),
            chart_card("XGB3", false),
        ],
    );
    let right_cards = column(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::gap>(12.0),
        vec![
            chart_card("Time", false),
            chart_card("IC改进", false),
            chart_card("积冰指数", false),
            chart_card("XGB2", false),
            chart_card("XGB4", false),
        ],
    );
    let cards = row(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::gap>(12.0)
            .with::<container::align>(AlignStyle::Start),
        vec![left_cards, right_cards],
    );
    let scroller = container()
        .with::<container::flex_direction>(FlexDirectionStyle::Column)
        .with::<container::style>(
            StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::height>(Sizing::fill())
                .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical)
                .with::<container::scrollbar_width>(5.0)
                .with::<container::scrollbar_track_color>(Color::TRANSPARENT)
                .with::<container::scrollbar_thumb_color>(Color::hex("#3A3A3D")),
        )
        .into_element_desc(vec![cards]);

    column(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(Sizing::fill())
            .with::<container::padding>(EdgeInsets::new(0.0, 0.0, 12.0, 0.0))
            .with::<container::gap>(10.0),
        vec![
            row(
                StyleOf::new()
                    .with::<container::height>(24.0)
                    .with::<container::align>(AlignStyle::Center)
                    .with::<container::justify>(JustifyStyle::SpaceBetween),
                vec![
                    label("积冰算法产品", 12.0, TEXT, FontWeight::SemiBold),
                    label("10 项算法产品", 10.0, DIM, FontWeight::Normal),
                ],
            ),
            empty(
                StyleOf::new()
                    .with::<container::height>(1.0)
                    .with::<container::width>(Sizing::fill())
                    .with::<container::background>(BORDER),
            ),
            scroller,
        ],
    )
}

fn metric_card(title: &str, value: &str, unit: &str) -> ElementDesc {
    column(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(112.0)
            .with::<container::padding>(EdgeInsets::all(14.0))
            .with::<container::gap>(9.0)
            .with::<container::background>(PANEL)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        vec![
            label(title, 12.0, MUTED, FontWeight::Medium),
            row(
                StyleOf::new()
                    .with::<container::gap>(8.0)
                    .with::<container::align>(AlignStyle::End),
                vec![
                    label(value, 28.0, TEXT, FontWeight::Bold),
                    label(unit, 11.0, MUTED, FontWeight::Normal),
                ],
            ),
        ],
    )
}

fn flight_monitor_content() -> ElementDesc {
    let metrics = xui! {
        <grid columns={GridTracks::adaptive(200.0)} width={Sizing::fill()} gap={12.0}>
            {metric_card("当前飞行高度 (ALT)", "7462", "米 (m)")}
            {metric_card("当前真空速 (TAS)", "440", "公里/小时")}
            {metric_card("磁航向 (HDG)", "213°", "导航姿态稳定")}
            {metric_card("当前垂直速度 (V/S)", "+3.2", "米/秒")}
        </grid>
    };
    column(
        StyleOf::new()
            .with::<container::size>(Size::fill())
            .with::<container::gap>(12.0),
        vec![
            metrics,
            column(
                StyleOf::new()
                    .with::<container::size>(Size::fill())
                    .with::<container::padding>(EdgeInsets::all(14.0))
                    .with::<container::gap>(10.0)
                    .with::<container::background>(PANEL)
                    .with::<container::border_color>(BORDER)
                    .with::<container::border_width>(1.0)
                    .with::<container::border_radius>(3.0),
                vec![
                    label(
                        "全航程高度 / 真空速 实时剖面趋势图",
                        15.0,
                        TEXT,
                        FontWeight::Bold,
                    ),
                    empty(
                        StyleOf::new()
                            .with::<container::size>(Size::fill())
                            .with::<container::background>(PAGE)
                            .with::<container::border_color>(BORDER)
                            .with::<container::border_width>(1.0)
                            .with::<container::border_radius>(3.0),
                    ),
                ],
            ),
        ],
    )
}

fn playback_button(kind: SourceIcon) -> ElementDesc {
    row(
        StyleOf::new()
            .with::<container::size>(Size::fix(28.0, 28.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::Center)
            .with::<container::background>(SURFACE)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        vec![icon_view(kind, 13.0, MUTED)],
    )
}

fn select_box(value: &str, width: f32) -> ElementDesc {
    row(
        StyleOf::new()
            .with::<container::size>(Size::fix(width, 27.0))
            .with::<container::padding>(EdgeInsets::symmetric(9.0, 4.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::SpaceBetween)
            .with::<container::background>(PAGE)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        vec![label(value, 11.0, TEXT, FontWeight::Medium)],
    )
}

fn timeline() -> ElementDesc {
    let controls = row(
        StyleOf::new()
            .with::<container::height>(32.0)
            .with::<container::gap>(7.0)
            .with::<container::align>(AlignStyle::Center),
        vec![
            playback_button(SourceIcon::Previous),
            playback_button(SourceIcon::Play),
            playback_button(SourceIcon::Next),
            label("倍速:", 11.0, MUTED, FontWeight::Normal),
            select_box("1x", 62.0),
            label("间隔:", 11.0, MUTED, FontWeight::Normal),
            select_box("1s", 70.0),
            label("12:24:00", 12.0, TEXT, FontWeight::Bold),
        ],
    );

    let track = row(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(42.0)
            .with::<container::padding>(EdgeInsets::symmetric(14.0, 7.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::background>(Color::hex("#2B2112"))
            .with::<container::border_color>(Color::hex("#6D4D20"))
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(3.0),
        vec![row(
            StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::SpaceBetween),
            vec![
                label("12:00", 9.0, MUTED, FontWeight::Normal),
                label("12:05", 9.0, MUTED, FontWeight::Normal),
                label("12:10", 9.0, MUTED, FontWeight::Normal),
                label("12:15", 9.0, MUTED, FontWeight::Normal),
                label("12:20", 9.0, MUTED, FontWeight::Normal),
                label("12:24", 9.0, MUTED, FontWeight::Normal),
            ],
        )],
    );

    column(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(104.0)
            .with::<container::padding>(EdgeInsets::symmetric(16.0, 9.0))
            .with::<container::gap>(7.0)
            .with::<container::background>(PAGE)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0),
        vec![controls, track],
    )
}

fn status_bar() -> ElementDesc {
    row(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(30.0)
            .with::<container::padding>(EdgeInsets::symmetric(16.0, 5.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::SpaceBetween)
            .with::<container::background>(SURFACE)
            .with::<container::border_color>(BORDER)
            .with::<container::border_width>(1.0),
        vec![
            row(
                StyleOf::new()
                    .with::<container::gap>(18.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![
                    label("演示系统状态: 运行正常", 10.0, MUTED, FontWeight::Normal),
                    label("数据源: 动态气象仿真数据", 10.0, MUTED, FontWeight::Normal),
                    label("数据更新情况: 正常", 10.0, MUTED, FontWeight::Normal),
                ],
            ),
            row(
                StyleOf::new()
                    .with::<container::gap>(6.0)
                    .with::<container::align>(AlignStyle::Center),
                vec![dot(GREEN), label("ONLINE", 10.0, MUTED, FontWeight::Bold)],
            ),
        ],
    )
}

fn dashboard_view(
    analytics: ElementDesc,
    selected_product: usize,
    on_product_change: DropDownChangeCallback,
) -> ElementDesc {
    let workspace = row(
        StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::height>(714.0)
            .with::<container::gap>(16.0),
        vec![map_panel(selected_product, on_product_change), analytics],
    );

    column(
        StyleOf::new()
            .with::<container::size>(Size::fill())
            .with::<container::padding>(EdgeInsets::all(16.0).set_top(48.0))
            .with::<container::gap>(10.0)
            .with::<container::background>(PAGE)
            .with::<container::color>(TEXT)
            .with::<container::font_family>("Hiragino Sans GB")
            .with::<container::clip>(true),
        vec![workspace, timeline(), status_bar()],
    )
}

#[component]
pub fn flight_icing_dashboard() {
    let selected_tab = cx.use_state(|| 1usize);
    let selected_product = cx.use_state(|| 0usize);
    let on_tab_change = cx.use_callback(selected_tab, move |index| selected_tab.set(index));
    let on_product_change =
        cx.use_callback(selected_product, move |index| selected_product.set(index));
    let items = vec![
        TabItem::new("flight", "飞行监测", flight_monitor_content()),
        TabItem::new("icing", "积冰监测", icing_monitor_content()),
    ];
    let tab_style = TabsStyle {
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
    };
    let analytics = xui! {
        <tabs
            items={items}
            selected={Some(*selected_tab.get())}
            on_change={Some(on_tab_change)}
            style={tab_style}
            id_prefix={"flight-icing-monitor".to_string()}
        />
    };
    dashboard_view(analytics, *selected_product.get(), on_product_change)
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
        let mut app = App::new(flight_icing_dashboard_component);
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
        let mut app = App::new(flight_icing_dashboard_component);
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
        let mut app = App::new(flight_icing_dashboard_component);
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
