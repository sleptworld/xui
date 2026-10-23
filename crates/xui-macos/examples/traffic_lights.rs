//! Checks that `MacOsWindowOptions::btn_margin` keeps the window buttons where
//! it puts them, through what makes AppKit lay its title bar out again.
//!
//! ```sh
//! cargo run -p xui-macos --example traffic_lights
//! ```
//!
//! Opens a window for a few seconds, resizes, zooms, retitles and deactivates
//! it, reads where the close button is after each, prints one line per check,
//! and exits non-zero if any failed. Needs a logged-in desktop session.

use std::cell::RefCell;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use dispatch2::DispatchQueue;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSApplication, NSWindowButton};
use objc2_foundation::NSSize;
use xui_core::App;
use xui_core::prelude::*;
use xui_core::render::MockRenderBackend;
use xui_f::FBackend;
use xui_interface::Size;
use xui_macos::{
    MacOsWindowOptions, MacRunner, MacRunnerOptions, MacWindow, WindowOptions, WindowSize,
};
use xui_shell::PlatformWindow;

/// Where the close button's top-left corner should be, from the window's.
const MARGIN: (i32, i32) = (20, 18);

thread_local! {
    static WINDOW: RefCell<Option<Arc<MacWindow>>> = const { RefCell::new(None) };
    static RESULTS: RefCell<Vec<(String, Result<(), String>)>> = const { RefCell::new(Vec::new()) };
}

fn root(_: &mut HookContext<'_>) -> ElementDesc {
    container()
        .with::<container::width>(500.0)
        .with::<container::height>(400.0)
        .into_element_desc(Vec::new())
}

fn main() -> ExitCode {
    let options = MacRunnerOptions {
        window: WindowOptions {
            macos: MacOsWindowOptions {
                title_hidden: true,
                titlebar_transparent: true,
                fullsize_content_view: true,
                btn_margin: Some(MARGIN),
            },
            ..WindowOptions::default()
                .with_title("xui traffic lights")
                .with_inner_size(WindowSize::Logical(Size::new(500.0, 400.0)))
        },
        ..MacRunnerOptions::default()
    };
    let runner = MacRunner::with_options(
        |window| {
            WINDOW.with(|slot| *slot.borrow_mut() = Some(window));
            (
                App::new(root),
                FBackend::new(),
                MockRenderBackend::default(),
            )
        },
        options,
    );

    // Each step changes the window, and the check after it runs once AppKit has
    // had its turn of the run loop to lay the title bar out again.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1000));
        for &(name, step) in STEPS {
            DispatchQueue::main().exec_async(step);
            std::thread::sleep(Duration::from_millis(400));
            DispatchQueue::main().exec_async(move || check(name));
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

const STEPS: &[(&str, fn())] = &[
    ("after the window is shown", || {}),
    ("after a resize", || {
        window().setContentSize(NSSize::new(640.0, 480.0));
    }),
    ("after zooming", || window().zoom(None)),
    ("after zooming back", || window().zoom(None)),
    ("after the title changes", || {
        WINDOW.with(|slot| {
            let window = slot.borrow();
            let window = window.as_ref().expect("the window is open");
            window.set_title("xui traffic lights, retitled");
        });
    }),
    ("after the window resigns key", || {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm()).hide(None);
    }),
    ("after it becomes key again", || {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm()).activateIgnoringOtherApps(true);
        NSApplication::sharedApplication(mtm()).unhide(None);
        window().makeKeyAndOrderFront(None);
    }),
];

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("steps run on the main thread")
}

fn window() -> objc2::rc::Retained<objc2_app_kit::NSWindow> {
    WINDOW.with(|slot| {
        let window = slot.borrow();
        let window = window.as_ref().expect("the window is open");
        window.ns_window(mtm()).retain()
    })
}

/// The close button's top-left corner, from the window's, in points.
fn close_button_origin() -> Option<(f64, f64)> {
    let window = window();
    let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
    let rect = close.convertRect_toView(close.bounds(), None);
    let top = window.frame().size.height - (rect.origin.y + rect.size.height);
    Some((rect.origin.x, top))
}

fn check(name: &'static str) {
    let expected = (f64::from(MARGIN.0), f64::from(MARGIN.1));
    let result = match close_button_origin() {
        Some(origin)
            if (origin.0 - expected.0).abs() < 0.5 && (origin.1 - expected.1).abs() < 0.5 =>
        {
            Ok(())
        }
        Some(origin) => Err(format!("close button at {origin:?}, expected {expected:?}")),
        None => Err("no close button".to_owned()),
    };
    RESULTS.with(|results| {
        results
            .borrow_mut()
            .push((format!("the buttons stay at the margin {name}"), result));
    });
}
