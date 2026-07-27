mod components;
use winit::dpi::PhysicalSize;
use winit::window::Window;
use xui::prelude::*;
use xui_components::*;
use xui_winit::{WinitRunnerOptions, runner};

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
    let mut button_prop = ButtonProp::default();
    button_prop.text_color = Color::BLUE_500.into();
    xui! {
        <row size={Size::fill()}>
            <container width={Sizing::percent(0.3)} height={Sizing::fill()} background ={Color::rgb(1.0,0.0,0.0)} />
            <container size={Size::fill()} background ={Color::BLUE_500} >
                <column gap={12.0}>
                    <text> {"Hello,World"} </text>
                    <input width={100.0} background={Color::WHITE} border_color={Color::BLACK} border_radius={5.0} border_width={2.0} />
                    <row gap={12.0}>
                        {icon(filled_icon()).color(Color::BLUE_500).into_element_desc()}
                        {icon(stroked_icon()).color(Color::WHITE).style(Style::new().size(Size::fix(48.0, 48.0))).into_element_desc()}
                        {icon(svg_icon()).color(Color::BLACK).into_element_desc()}
                    </row>
                    <button text={label.get()} ps = {button_prop}/>
                    <text font_weight={FontWeight::Thin}> {"啊，是关中王来啦"} </text>
                    <image src={"https://i1.hdslb.com/bfs/archive/8b96913b723e39495c0d1f171779faded87fcbc7.jpg"} height={100} fit={ImageFit::ScaleDown} />
                </column>
            </container>
        </row>
    }
}

#[xui::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = WinitRunnerOptions {
        window_attributes: Window::default_attributes()
            .with_title("XUI Example App")
            .with_inner_size(PhysicalSize::new(800, 600)),
        ..Default::default()
    };

    runner(editor_component, Some(options)).run().unwrap();
    Ok(())
}
