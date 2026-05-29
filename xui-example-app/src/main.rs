mod components;
use winit::dpi::PhysicalSize;
use winit::window::Window;
use xui::prelude::*;
use xui_winit::{WGPUBackend, WinitRunner, WinitRunnerOptions};

pub struct MainProp {
    pub info: String,
}

component_fn! {
    fn counter() {
        let count = cx.use_state(|| 0);
        let count_for_increment = count.clone();
        let count_for_decrement = count.clone();

        <column gap={8.0}>
            <text font_size={16.0} font_weight={FontWeight::Bold}> {"HI!! NEW WORLD"} </text>
            <text> {format!("Current count: {}", count.get())} </text>
            <button on_click={{
                let count_for_increment = count_for_increment.clone();
                move |_| {
                    count_for_increment.set(count_for_increment.get() + 1);
                    EventResult::Consumed
                }
            }}>
                {"Increment"}
            </button>
            <button on_click={{
                let count_for_decrement = count_for_decrement.clone();
                move |_| {
                    count_for_decrement.set(count_for_decrement.get() - 1);
                    println!("{}", count_for_decrement.get());
                    EventResult::Consumed
                }
            }}>
                {"Decrement"}
            </button>
        </column>
    }

    fn test_page() {
        <row gap={0.0}>
            <container background={Color::BLUE_500}>
                <text> {"HELLO<WORLF"}</text>
            </container>
        <container />
        </row>

    }

    fn main_page(MainProp{info}: &MainProp) {

        <column gap={12.0}>
            <label color={Color::BLUE_500}>{info}</label>
            <counter />
        </column>
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::with_component_registry(|registry| {
        register_counter_component(registry);
        register_test_page_component(registry);
        components::register_components(registry);
        xui_components::register_components(registry);

        |_| {
            xui! {
                <container padding={EdgeInsets::all(16.0)}>
                <column gap={12.0}>

                <container size={Some(Size::new(200.0,200.0))}
                    background={Color::BLACK} border_radius={15.0}
                    shadow={ShadowStyle::default()
                        .color(Color::BLACK)
                        .offset(Point::new(0., 1.)).blur(5.)}
                />

                <container size={Some(Size::new(200.0,200.0))}
                    background={Color::BLUE_500} border_radius={15.0}
                    shadow={ShadowStyle::default()
                        .color(Color::BLACK)
                        .offset(Point::new(0., 1.)).blur(5.)}
                />
                </column>
                    <column gap={12.0}>
                        <label color={Color::BLUE_500}>{"XUI winit example"}</label>
                        <counter key="counter" />
                    </column>
                </container>
            }
        }
    });

    let options = WinitRunnerOptions {
        window_attributes: Window::default_attributes()
            .with_title("XUI Example App")
            .with_inner_size(PhysicalSize::new(800, 600)),
        ..Default::default()
    };

    WinitRunner::with_backend_factory(
        |window| (app, Engine::new(), WGPUBackend::new(window)),
        Some(options),
    )
    .run()
    .unwrap();

    Ok(())
}
