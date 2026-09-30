//! End-to-end demo application for the `xui` framework: the Component Gallery.
//!
//! Every `xui-components` control on the shadcn/ui-style theme, one page per
//! component, plus icons, canvas scenes, and backdrop blur. Pages are built
//! with the `xui!` macro and `#[component]` functions; the image page loads
//! from the packed `assets/`.
//!
//! `cargo run -p xui-example-app` (add `XUI_THEME=dark` for the dark palette).
//! Its build script packs `assets/` through `xui-build`; `cargo xui run` works
//! too, and additionally mounts `assets/` live in release builds.
//!
//! The aircraft icing dashboard demo is the `flight-icing` binary.

mod gallery;

use xui_core::prelude::*;
use xui_macos::{MacOsWindowOptions, MacRunner, MacRunnerOptions, PhysicalSize, WindowOptions};

#[xui_macros::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let theme = match std::env::var("XUI_THEME").as_deref() {
        Ok("dark") => Theme::dark(),
        _ => Theme::light(),
    };
    let options = MacRunnerOptions {
        window: WindowOptions::default()
            .with_title("xui Component Gallery")
            .with_inner_size(PhysicalSize::new(1440, 960))
            .with_macos(MacOsWindowOptions {
                title_hidden: true,
                fullsize_content_view: true,
                titlebar_transparent: true,
            }),
        ..Default::default()
    };

    // `xui_macos::runner` with a theme applied before the first frame.
    MacRunner::with_fallible_options(
        move |window| -> Result<_, std::io::Error> {
            let mut app = App::new(gallery::gallery_component);
            app.set_theme(theme);
            let backend = xui_skia::SkiaBackend::<xui_f::FBackend>::new(window)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok((app, xui_f::FBackend::new(), backend))
        },
        options,
    )
    .run()
    .unwrap();
    Ok(())
}
