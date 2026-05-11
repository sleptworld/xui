mod components;
use winit::dpi::PhysicalSize;
use winit::window::Window;
use xui::prelude::*;
use xui_winit::{WinitRunner, WinitRunnerOptions};

#[component]
fn counter() {
    let count = cx.use_state(|| 0);
    let count_for_increment = count.clone();
    let count_for_decrement = count.clone();

    xui! {
        <column gap={8.0}>
            <label color={Color::BLUE_500}>{format!("Current count: {}", count.get())}</label>
            <button on_click={move || count_for_increment.set(count_for_increment.get() + 1)}>
                {"Increment"}
            </button>
            <button on_click={move || count_for_decrement.set(count_for_decrement.get() - 1)}>
                {"Decrement"}
            </button>
        </column>
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = app(|_| {
        xui! {
            <container size={Size::new(360.0, 180.0)} padding={EdgeInsets::all(16.0)} background={Color::WHITE}>
                <column gap={12.0}>
                    <label color={Color::BLUE_500}>{"XUI winit example"}</label>
                    <counter key="counter" />
                    <component key="summary" render={components::summary_component} />
                </column>
            </container>
        }
    });

    let runtime = GuiRuntime::new(app, MockRenderBackend::default());
    let options = WinitRunnerOptions {
        window_attributes: Window::default_attributes()
            .with_title("XUI Example App")
            .with_inner_size(PhysicalSize::new(480, 320)),
        ..Default::default()
    };

    WinitRunner::with_options(runtime, options).run()?;
    Ok(())
}
