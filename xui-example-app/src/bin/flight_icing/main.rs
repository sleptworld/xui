//! The aircraft icing monitoring dashboard: a full application-style demo.
//!
//! `cargo run -p xui-example-app --bin flight-icing` (on Skia: add
//! `--features skia`)

mod dashboard;

#[cfg(not(feature = "skia"))]
use xui_macos::runner;
#[cfg(feature = "skia")]
use xui_macos::skia_runner as runner;
use xui_macos::{MacOsWindowOptions, MacRunnerOptions, PhysicalSize, WindowOptions};

#[xui_macros::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = MacRunnerOptions {
        window: WindowOptions::default()
            .with_title("飞机积冰协同态势监测与预测系统")
            .with_inner_size(PhysicalSize::new(1600, 900))
            .with_macos(MacOsWindowOptions {
                title_hidden: true,
                titlebar_transparent: true,
                fullsize_content_view: true,
                ..Default::default()
            }),
        ..Default::default()
    };

    runner(dashboard::dashboard_root, Some(options))
        .run()
        .unwrap();
    Ok(())
}
