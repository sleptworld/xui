//! Checks that the window reports its appearance, and that an app in
//! `ThemeMode::System` (the default) switches theme with it.
//!
//! ```sh
//! cargo run -p xui-macos --example appearance
//! ```
//!
//! Opens a window for a few seconds, sets the application's appearance to
//! dark, to light and back to the system's, reads which theme is in use
//! through the handle the app's components get after each, prints one line per check, and exits non-zero if
//! any failed. The system setting itself is left alone. Needs a logged-in
//! desktop session.

use std::cell::RefCell;
use std::process::ExitCode;
use std::time::Duration;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};
use objc2_foundation::NSArray;
use xui_core::App;
use xui_core::prelude::*;
use xui_core::render::MockRenderBackend;
use xui_f::FBackend;
use xui_interface::{ColorScheme, Size};
use xui_macos::{MacRunner, MacRunnerOptions, WindowOptions, WindowSize};
use xui_macros::component;

thread_local! {
    /// The theme handle the component got through its context.
    static HANDLE: RefCell<Option<ThemeHandle>> = const { RefCell::new(None) };
    static RESULTS: RefCell<Vec<(String, Result<(), String>)>> = const { RefCell::new(Vec::new()) };
}

#[component]
fn window_content() {
    let theme: ThemeHandle = context!();
    effect!(|| HANDLE.with(|handle| *handle.borrow_mut() = Some(theme.get())));
    view! { container(width: 300.0, height: 200.0, background: tokens::color::BACKGROUND) }
}

fn root(app: &mut App) {
    app.mount::<WindowContent>(WindowContentProps {});
}

fn main() -> ExitCode {
    let options = MacRunnerOptions {
        window: WindowOptions::default()
            .with_title("xui appearance")
            .with_inner_size(WindowSize::Logical(Size::new(300.0, 200.0))),
        ..MacRunnerOptions::default()
    };
    let runner = MacRunner::with_options(
        |_| {
            (
                App::new(root),
                FBackend::new(),
                MockRenderBackend::default(),
            )
        },
        options,
    );

    // Each step changes the appearance, and the check after it runs once the
    // change has been through a frame.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1000));
        for &(name, step, expected) in STEPS {
            DispatchQueue::main().exec_async(step);
            std::thread::sleep(Duration::from_millis(400));
            DispatchQueue::main().exec_async(move || check(name, expected));
            std::thread::sleep(Duration::from_millis(100));
        }
        DispatchQueue::main().exec_async(|| {
            NSApplication::sharedApplication(mtm()).terminate(None);
        });
    });

    if let Err(error) = runner.run() {
        eprintln!("the runner failed: {error}");
        return ExitCode::FAILURE;
    }

    let results = RESULTS.with(|results| results.take());
    let mut failed = 0;
    for (name, result) in &results {
        match result {
            Ok(()) => println!("ok    {name}"),
            Err(detail) => {
                failed += 1;
                println!("FAIL  {name}: {detail}");
            }
        }
    }
    println!("{} checks, {failed} failed", results.len());
    if results.is_empty() || failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// What the theme should be after each step; `None` is the system's.
const STEPS: &[(&str, fn(), Option<ColorScheme>)] = &[
    ("the window starts in the system's", || {}, None),
    (
        "dark when the application is set to dark",
        || set_appearance(Some(unsafe { NSAppearanceNameDarkAqua })),
        Some(ColorScheme::Dark),
    ),
    (
        "light when the application is set to light",
        || set_appearance(Some(unsafe { NSAppearanceNameAqua })),
        Some(ColorScheme::Light),
    ),
    (
        "the system's again when the application follows it",
        || set_appearance(None),
        None,
    ),
];

fn set_appearance(name: Option<&objc2_foundation::NSString>) {
    let appearance = name.and_then(NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm()).setAppearance(appearance.as_deref());
}

/// The scheme the application is drawn in, read from AppKit directly.
fn system_scheme() -> ColorScheme {
    let (aqua, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    let best = NSApplication::sharedApplication(mtm())
        .effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[aqua, dark]));
    match best {
        Some(name) if name.isEqualToString(dark) => ColorScheme::Dark,
        _ => ColorScheme::Light,
    }
}

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("steps run on the main thread")
}

fn check(name: &'static str, expected: Option<ColorScheme>) {
    let seen = HANDLE.with(|handle| {
        handle
            .borrow()
            .as_ref()
            .map(|handle| handle.current().scheme)
    });
    let expected = expected.unwrap_or_else(system_scheme);
    let result = match (seen, expected) {
        (Some(seen), expected) if seen == expected => Ok(()),
        (seen, expected) => Err(format!("saw {seen:?}, expected {expected:?}")),
    };
    RESULTS.with(|results| {
        results
            .borrow_mut()
            .push((format!("the theme is {name}"), result));
    });
}
