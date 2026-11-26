//! End-to-end demo application for the `xui` framework: the Component Gallery.
//!
//! Every `xui-components` control on the shadcn/ui-style theme, one page per
//! component, plus icons, canvas scenes, and backdrop blur. Pages are
//! `#[component]`s; the image page loads from the packed `assets/`.
//!
//! `cargo run -p xui-example-app` (add `XUI_THEME=dark` for the dark palette;
//! `--features skia` runs it on Skia instead of Vello).
//! Its build script packs `assets/` through `xui-build`; `cargo xui run` works
//! too, and additionally mounts `assets/` live in release builds.

mod gallery;

use xui_core::prelude::*;
use xui_macos::{MacOsWindowOptions, MacRunner, MacRunnerOptions, PhysicalSize, WindowOptions};

#[xui_macros::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = match std::env::var("XUI_THEME").as_deref() {
        Ok("dark") => ThemeMode::Dark,
        Ok("light") => ThemeMode::Light,
        _ => ThemeMode::System,
    };
    let options = MacRunnerOptions {
        window: WindowOptions::default()
            .with_title("xui Component Gallery")
            .with_inner_size(PhysicalSize::new(1440, 960))
            .with_macos(MacOsWindowOptions {
                title_hidden: true,
                fullsize_content_view: true,
                titlebar_transparent: true,
                ..Default::default()
            }),
        ..Default::default()
    };

    #[cfg(feature = "skia")]
    return run_skia(mode, options);

    #[cfg(not(feature = "skia"))]
    run_vello(mode, options)
}

/// `xui_macos::runner` with a theme mode applied before the first frame.
#[cfg(not(feature = "skia"))]
fn run_vello(mode: ThemeMode, options: MacRunnerOptions) -> Result<(), Box<dyn std::error::Error>> {
    MacRunner::with_fallible_options(
        move |window| -> Result<_, std::io::Error> {
            let mut app = App::new(gallery::gallery_root);
            app.set_theme_mode(mode);
            let backend = xui_vello::VelloBackend::<xui_f::FBackend>::new(window)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let context = backend.wgpu_context();
            app.set_gpu_context(xui_core::widgets::CanvasGpuContext::new(
                context.device().clone(),
                context.queue().clone(),
            ));
            Ok((app, xui_f::FBackend::new(), backend))
        },
        options,
    )
    .run()?;
    Ok(())
}

/// The same gallery on Skia: `cargo run -p xui-example-app --features skia`.
#[cfg(feature = "skia")]
fn run_skia(mode: ThemeMode, options: MacRunnerOptions) -> Result<(), Box<dyn std::error::Error>> {
    MacRunner::with_fallible_options(
        move |window| -> Result<_, std::io::Error> {
            let mut app = App::new(gallery::gallery_root);
            app.set_theme_mode(mode);
            let backend = xui_skia::SkiaBackend::<xui_f::FBackend>::new(window)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok((app, xui_f::FBackend::new(), backend))
        },
        options,
    )
    .run()?;
    Ok(())
}
