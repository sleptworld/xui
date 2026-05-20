mod components;
use winit::dpi::PhysicalSize;
use winit::window::Window;
use xui::prelude::*;
use xui_winit::{WGPUBackend, WinitRunner, WinitRunnerOptions};

#[component]
fn counter() {
    let count = cx.use_state(|| 0);
    let count_for_increment = count.clone();
    let count_for_decrement = count.clone();

    xui! {
        <column gap={8.0}>
            <label color={Color::BLUE_500}>{format!("Current count: {}", count.get())}</label>
            <button on_click={{
                let count_for_increment = count_for_increment.clone();
                move || count_for_increment.set(count_for_increment.get() + 1)
            }}>
                {"Increment"}
            </button>
            <button on_click={{
                let count_for_decrement = count_for_decrement.clone();
                move || count_for_decrement.set(count_for_decrement.get() - 1)
            }}>
                {"Decrement"}
            </button>
        </column>
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::with_component_registry(|registry| {
        register_counter_component(registry);
        components::register_components(registry);

        |_| {
            xui! {
                <container padding={EdgeInsets::all(16.0)}>
                <container size={Some(Size::new(100.0,100.0))} background={Color::BLACK}/>
                    <column gap={12.0}>
                        <label color={Color::BLUE_500}>{"XUI winit example"}</label>
                        <counter key="counter" />
                        // <component key="summary" render={components::summary_component} />
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

    WinitRunner::with_backend_factory(|window| (app, WGPUBackend::new(window)), Some(options))
        .run()
        .unwrap();

    Ok(())
}
