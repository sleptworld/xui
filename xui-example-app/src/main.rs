//! End-to-end demo application for the `xui` framework.
//!
//! Demonstrates the `xui!` macro, `#[component]` functions, the
//! `xui-components` widget set, asset loading, icons (path/SVG), text, layouts,
//! vector scenes, and the `xui-winit` Skia runner. This is the recommended
//! starting point for learning the framework.
//!
//! Build and run with `cargo xui run` — the `#[xui::main]` entry point
//! requires the asset bootstrap module that `cargo xui` generates (it reads the
//! `XUI_ASSETS_BOOTSTRAP` environment variable), so plain `cargo build` will not
//! compile this binary.

mod components;
mod flight_icing;
use winit::dpi::PhysicalSize;
use winit::platform::macos::WindowAttributesExtMacOS;
use winit::window::Window;
use xui::core::Bounds;
use xui::prelude::*;
use xui_components::*;
// Explicit: disambiguates the `<image>` tag from the `xui::image` host widget.
use xui_components::image::image;
use xui_winit::runner;
use xui_winit::{FontSet, WinitRunnerOptions};

fn filled_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(12.0, 2.0))
            .line_to(Point::new(22.0, 20.0))
            .line_to(Point::new(2.0, 20.0))
            .close();
        IconData::from_fill(Rect::new(0.0, 0.0, 24.0, 24.0), path.build())
    })
    .clone()
}

fn stroked_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(4.0, 12.0))
            .cubic_to(
                Point::new(4.0, 5.0),
                Point::new(20.0, 5.0),
                Point::new(20.0, 12.0),
            )
            .cubic_to(
                Point::new(20.0, 19.0),
                Point::new(4.0, 19.0),
                Point::new(4.0, 12.0),
            )
            .close();
        IconData::from_stroke(
            Rect::new(0.0, 0.0, 24.0, 24.0),
            path.build(),
            IconStroke::new(2.0)
                .cap(LineCap::Round)
                .join(LineJoin::Round),
        )
    })
    .clone()
}

fn svg_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        IconData::from_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"
                     fill="none" stroke="currentColor" stroke-width="2"
                     stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="9"/>
                    <path d="M8 12l3 3 5-6"/>
                </svg>"#,
        )
        .expect("embedded SVG icon must be valid")
    })
    .clone()
}

fn demo_canvas_scene(highlighted: bool) -> VectorScene {
    let mut scene = VectorSceneBuilder::new();
    let area_color = if highlighted {
        Color::rgba(0.74, 0.32, 0.96, 0.3)
    } else {
        Color::rgba(0.18, 0.42, 0.88, 0.28)
    };
    let line_color = if highlighted {
        Color::rgb(0.88, 0.55, 1.0)
    } else {
        Color::rgb(0.35, 0.72, 1.0)
    };

    let mut background = PathBuilder::new();
    background
        .move_to(Point::new(0.0, 0.0))
        .line_to(Point::new(440.0, 0.0))
        .line_to(Point::new(440.0, 220.0))
        .line_to(Point::new(0.0, 220.0))
        .close();
    let background_color = if highlighted {
        Color::rgb(0.16, 0.055, 0.22)
    } else {
        Color::rgb(0.04, 0.06, 0.11)
    };
    scene.fill_path(
        background.build(),
        Affine::IDENTITY,
        PathFill::new(background_color),
    );

    let mut grid = PathBuilder::new();
    for x in [40.0, 120.0, 200.0, 280.0, 360.0, 420.0] {
        grid.move_to(Point::new(x, 24.0))
            .line_to(Point::new(x, 192.0));
    }
    for y in [32.0, 72.0, 112.0, 152.0, 192.0] {
        grid.move_to(Point::new(24.0, y))
            .line_to(Point::new(420.0, y));
    }
    scene.stroke_path(
        grid.build(),
        Affine::IDENTITY,
        PathStroke::new(Color::rgba(0.55, 0.67, 0.86, 0.18), 1.0),
    );

    let mut area = PathBuilder::new();
    area.move_to(Point::new(24.0, 192.0))
        .line_to(Point::new(24.0, 158.0))
        .cubic_to(
            Point::new(76.0, 154.0),
            Point::new(92.0, 110.0),
            Point::new(136.0, 118.0),
        )
        .cubic_to(
            Point::new(188.0, 128.0),
            Point::new(202.0, 66.0),
            Point::new(252.0, 78.0),
        )
        .cubic_to(
            Point::new(302.0, 90.0),
            Point::new(326.0, 42.0),
            Point::new(368.0, 54.0),
        )
        .cubic_to(
            Point::new(392.0, 60.0),
            Point::new(406.0, 38.0),
            Point::new(420.0, 32.0),
        )
        .line_to(Point::new(420.0, 192.0))
        .close();
    scene.fill_path(area.build(), Affine::IDENTITY, PathFill::new(area_color));

    let mut caption = TextProps::new(if highlighted {
        "Stable CanvasTextId\nshapes only when text changes"
    } else {
        "Canvas text box\nshares the TextHost cache"
    });
    caption.style.color = Color::rgb(0.92, 0.96, 1.0);
    caption.style.font_size = 15.0;
    caption.style.font_weight = FontWeight::Medium;
    caption.paragraph.vertical_align = TextVerticalAlign::Middle;
    caption.text_box.max_lines = Some(2);
    caption.text_box.overflow = TextOverflow::Ellipsis;
    scene.text_box(
        CanvasTextId::new(1),
        Bounds::from_origin_size((38.0, 36.0), (190.0, 58.0)),
        caption,
    );

    let mut line = PathBuilder::new();
    line.move_to(Point::new(24.0, 158.0))
        .cubic_to(
            Point::new(76.0, 154.0),
            Point::new(92.0, 110.0),
            Point::new(136.0, 118.0),
        )
        .cubic_to(
            Point::new(188.0, 128.0),
            Point::new(202.0, 66.0),
            Point::new(252.0, 78.0),
        )
        .cubic_to(
            Point::new(302.0, 90.0),
            Point::new(326.0, 42.0),
            Point::new(368.0, 54.0),
        )
        .cubic_to(
            Point::new(392.0, 60.0),
            Point::new(406.0, 38.0),
            Point::new(420.0, 32.0),
        );
    scene.stroke_path(
        line.build(),
        Affine::IDENTITY,
        PathStroke::new(line_color, 4.0)
            .cap(LineCap::Round)
            .join(LineJoin::Round),
    );

    scene.build()
}

fn tab_panel(title: &str, description: &str) -> ElementDesc {
    ContainerWidget::new()
        .style(Style::new().gap(6.0))
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(vec![
            TextWidget::new(title.to_string())
                .style(
                    Style::new()
                        .color(Color::WHITE)
                        .font_size(16.0)
                        .font_weight(FontWeight::Medium),
                )
                .into_element_desc(),
            TextWidget::new(description.to_string())
                .style(
                    Style::new()
                        .color(Color::rgba(0.92, 0.96, 1.0, 0.68))
                        .font_size(13.0),
                )
                .into_element_desc(),
        ])
}

fn demo_tab_items() -> Vec<TabItem> {
    vec![
        TabItem::new(
            "overview",
            "Overview",
            tab_panel(
                "Tabs in xui-components",
                "Click a tab, or focus it and use Left/Right/Home/End.",
            ),
        ),
        TabItem::new(
            "keyboard",
            "Keyboard",
            tab_panel(
                "Roving keyboard focus",
                "Only the selected tab is in the Tab order; arrow keys wrap around.",
            ),
        ),
        TabItem::new(
            "accessibility",
            "A11y",
            tab_panel(
                "Accessible relationships",
                "TabList, Tab and TabPanel roles carry selected and labelled-by metadata.",
            ),
        ),
        TabItem::new(
            "disabled",
            "Disabled",
            tab_panel("Disabled", "Keyboard navigation skips this tab."),
        )
        .disabled(true),
    ]
}

#[component]
fn test_page() {
    xui! {
        <row gap={0.0}>
            <container background={Color::BLUE_500}>
                <text> {"HELLO<WORLF"}</text>
            </container>
        <container />
        </row>
    }
}

#[component]
fn editor() {
    let label = cx.use_state(|| "Hello,World".to_string());
    let selected_tab = cx.use_state(|| 0usize);
    let on_tab_change = cx.use_callback(selected_tab, move |index| selected_tab.set(index));
    let tab_items = demo_tab_items();
    let mut tabs_style = TabsStyle::default();
    tabs_style.root = tabs_style.root.width(440.0);
    let canvas_highlighted = cx.use_state(|| false);
    let canvas_controller = cx.use_ref(|| CanvasController::with_scene(demo_canvas_scene(false)));
    let canvas_handle = canvas_controller.get().clone();
    let canvas_click_handle = canvas_handle.clone();
    let button_style = Style::new().color(Color::BLUE_500);
    xui! {
        <row size={Size::fill()}>
            <container width={Sizing::percent(0.3)} height={Sizing::fill()} background ={Color::rgb(1.0,0.0,0.0)} />
            <container size={Size::fill()} background ={Color::hex("#171717")} >
                <column gap={12.0}>
                    <text> {"Hello,World"} </text>
                    <text color={Color::rgba(0.92, 0.96, 1.0, 0.78)} font_weight={FontWeight::Medium}>
                        {"Tabs · click or use the arrow keys"}
                    </text>
                    <tabs
                        items={tab_items}
                        selected={Some(*selected_tab.get())}
                        on_change={Some(on_tab_change)}
                        style={tabs_style}
                        id_prefix={"example-tabs".to_string()}
                    />
                    <input width={100.0} background={Color::WHITE} border_color={Color::BLACK} border_radius={5.0} border_width={2.0} />
                    <row gap={12.0}>
                        {icon().from_icon_data(filled_icon()).color(Color::BLUE_500).into_element_desc()}
                        {icon().from_icon_data(stroked_icon()).color(Color::WHITE).style(Style::new().size(Size::fix(48.0, 48.0))).into_element_desc()}
                        {icon().from_icon_data(svg_icon()).color(Color::BLACK).into_element_desc()}
                    </row>
                    <button text={label.get()} style={button_style}/>
                    <text font_weight={FontWeight::Thin}> {"啊，是关中王来啦"} </text>
                    <image src={"https://i1.hdslb.com/bfs/archive/8b96913b723e39495c0d1f171779faded87fcbc7.jpg"} height={100} fit={ImageFit::ScaleDown} />
                    <text font_weight={FontWeight::Medium}> {"Backdrop blur · click the glass card"} </text>
                    <z_stack style={Style::new().size(Size::fix(440.0, 220.0))}>
                        <canvas controller={canvas_handle} width={440.0} height={220.0} />
                        <container
                            style={Style::new()
                                .size(Size::fix(310.0, 118.0))
                                .padding(EdgeInsets::all(22.0))
                                .background(Color::rgba(0.92, 0.96, 1.0, 0.16))
                                .border_color(Color::rgba(1.0, 1.0, 1.0, 0.48))
                                .border_width(1.0)
                                .border_radius(20.0)
                                .clip(true)}
                            backdrop_blur={18.0}
                            on_click={move |_, _| {
                                let highlighted = !*canvas_highlighted.get();
                                canvas_click_handle.set_scene(demo_canvas_scene(highlighted));
                                canvas_highlighted.set(highlighted);
                                EventResult::Consumed
                            }}
                        >
                            <column gap={7.0}>
                                <text color={Color::WHITE} font_size={20.0} font_weight={FontWeight::Medium}>
                                    {"GPU Backdrop Blur"}
                                </text>
                                <text color={Color::rgba(0.92, 0.96, 1.0, 0.82)} font_size={13.0}>
                                    {"The live Canvas remains visible through an 18 px frosted layer."}
                                </text>
                            </column>
                        </container>
                    </z_stack>
                </column>
            </container>
        </row>
    }
}

#[xui::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = WinitRunnerOptions {
        window_attributes: Window::default_attributes()
            .with_title("飞机积冰协同态势监测与预测系统")
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
            .with_titlebar_transparent(true)
            .with_inner_size(PhysicalSize::new(1600, 900)),
        // The two faces this dashboard actually draws with. Scanning every
        // installed font instead costs a font-database build before the first
        // frame and, worse, makes the first layout fall back across hundreds of
        // faces — together about 400 ms of the startup this app used to spend
        // on a blank window. Paths that do not exist (any machine that is not
        // this one) fall back to the full system scan.
        fonts: FontSet::only_files([
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/SFNS.ttf",
        ])
        .with_sans_serif_family("Hiragino Sans GB"),
        ..Default::default()
    };

    runner(
        flight_icing::flight_icing_dashboard_component,
        Some(options),
    )
    .run()
    .unwrap();
    Ok(())
}
