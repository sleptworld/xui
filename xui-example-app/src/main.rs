mod components;
use winit::dpi::PhysicalSize;
use winit::window::Window;
use xui::prelude::*;
use xui_winit::{CosmicTextEngine, WGPUBackend, WinitRunner, WinitRunnerOptions, WinitTextEngine};

use xui_components::button::*;

#[component]
fn btn(name: &String) {
    xui! {
    <button
        style={Style::new()
            .width(Sizing::Hug)
            .background(Color::BLACK)
            .color(Color::WHITE)
            .padding(EdgeInsets::symmetric(12.0, 4.0))
            .border_radius(5.0)}
        hover_style={Style::new().background(Color::BLUE_500)}
    >
        {name}
    </button>}
}

#[component]
fn counter() {
    let count = cx.use_state(|| 0);

    xui! {<column gap={8.0}>
        <text font_size={16.0} font_weight={FontWeight::Bold}> {"HI!! NEW WORLD"} </text>
        <column gap={2.0}>
            <text font_family={"PingFang SC"}> {format!("Current count: {}", count.get())} </text>
            <text> {"你好吗, FUCK THE WORLD"} </text>
            <text> {"What the Fuck!!"} </text>
            <text> {"Do you know me, 哈哈哈哈哈哈"} </text>
        </column>

        <button on_click={{
            move |_, _| {
                count.set(count.get() + 1);
                EventResult::Consumed
            }
        }}>
            {"Increment"}
        </button>
        <button on_click={{
            move |_, _| {
                count.set(count.get() - 1);
                println!("{}", count.get());
                EventResult::Consumed
            }
        }}>
            {"Decrement"}
        </button>

        <row gap={2.0}>
            <btn props= {"Hello\nWorld".to_string()}/>
            <btn props= {"Hello".to_string()}/>
            <btn props= {"Hello".to_string()}/>
            <btn props= {"Hello".to_string()}/>
            <btn props= {"Oh".to_string()}/>
            <btn props= {"MY🤔".to_string()}/>
        </row>
    </column>}
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
                <column gap={8.0}>
                    <text> {"Hello,World"} </text>
                    <pbutton text={label.get()} ps = {button_prop}/>
                </column>
            </container>
        </row>
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = app(editor_component);

    let options = WinitRunnerOptions {
        window_attributes: Window::default_attributes()
            .with_title("XUI Example App")
            .with_inner_size(PhysicalSize::new(800, 600)),
        ..Default::default()
    };

    WinitRunner::with_backend_factory(
        |window| {
            (
                app,
                WinitTextEngine::<CosmicTextEngine>::new(),
                WGPUBackend::new(window),
            )
        },
        Some(options),
    )
    .run()
    .unwrap();

    Ok(())
}
