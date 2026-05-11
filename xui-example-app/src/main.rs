use winit::dpi::PhysicalSize;
use winit::window::Window;
use xui::{
    Color, EdgeInsets, GuiRuntime, MockRenderBackend, Size, app, button, column, container, label,
};
use xui_winit::{WinitRunner, WinitRunnerOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = app(|cx| {
        let count = cx.use_state(|| 0);
        let count_for_click = count.clone();

        container()
            .size(Size::new(360.0, 180.0))
            .padding(EdgeInsets::all(16.0))
            .background(Color::WHITE)
            .child(
                column()
                    .gap(12.0)
                    .child(label("XUI winit example").color(Color::BLUE_500))
                    .child(label(format!("Clicked {} times", count.get())))
                    .child(button("Increment").on_click(move || {
                        count_for_click.set(count_for_click.get() + 1);
                    })),
            )
            .into()
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
