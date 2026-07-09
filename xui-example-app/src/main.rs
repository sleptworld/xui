mod components;
use winit::dpi::PhysicalSize;
use winit::window::Window;
use xui::prelude::*;
use xui_components::*;
use xui_winit::{WinitRunnerOptions, runner};

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
