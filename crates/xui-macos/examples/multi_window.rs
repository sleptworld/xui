//! An end-to-end check of windows beyond the main one, against real AppKit
//! and the real Vello backend.
//!
//! ```sh
//! cargo run -p xui-macos --example multi_window
//! ```
//!
//! The app mounts a `window(..) { .. }` block with its root, so a second
//! window opens at launch. The check finds it on screen, presses its close
//! button, sees the block's `on_close_requested` close it while the main
//! window stays, and quits. One line per check; non-zero if any failed. Needs
//! a logged-in desktop session with the screen unlocked.

use std::cell::RefCell;
use std::process::ExitCode;
use std::time::Duration;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSWindow};
use xui_core::prelude::*;
use xui_interface::Size;
use xui_macos::{MacRunnerOptions, WindowOptions, WindowSize};
use xui_macros::component;

thread_local! {
    static RESULTS: RefCell<Vec<(&'static str, Result<(), String>)>> =
        const { RefCell::new(Vec::new()) };
}

fn record(name: &'static str, ok: bool, detail: impl FnOnce() -> String) {
    let result = if ok { Ok(()) } else { Err(detail()) };
    RESULTS.with(|results| results.borrow_mut().push((name, result)));
}

#[component]
fn two_windows() {
    let open: bool = state!(true);
    view! {
        column {
            text("main")
            if *open {
                window(
                    title: "Second",
                    size: (320.0, 200.0),
                    on_close_requested: |cx| open.set(cx, false),
                ) {
                    text("second")
                }
            }
        }
    }
}

fn root(app: &mut App) {
    app.mount::<TwoWindows>(TwoWindowsProps {});
}

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("steps run on the main thread")
}

/// The on-screen window titled `title`, if there is one.
fn visible_window(title: &str) -> Option<Retained<NSWindow>> {
    NSApplication::sharedApplication(mtm())
        .windows()
        .iter()
        .find(|window| window.isVisible() && window.title().to_string() == title)
}

fn step_second_is_open() {
    let main = visible_window("Main");
    let second = visible_window("Second");
    record("the main window is on screen", main.is_some(), || {
        "no visible window titled Main".into()
    });
    record(
        "a window block mounted with the root opened a second window",
        second.is_some(),
        || "no visible window titled Second".into(),
    );
    if let Some(second) = second {
        let size = second.contentView().map(|view| view.frame().size);
        record(
            "the second window has the size its block asked for",
            size.is_some_and(|size| size.width == 320.0 && size.height == 200.0),
            || format!("content size {size:?}"),
        );
    }
}

fn step_close_second() {
    if let Some(second) = visible_window("Second") {
        // As the close button does: through `windowShouldClose:`.
        second.performClose(None);
    }
}

fn step_second_is_closed() {
    record(
        "the close button closed the second window through its block",
        visible_window("Second").is_none(),
        || "a window titled Second is still on screen".into(),
    );
    record(
        "the main window stays when the second closes",
        visible_window("Main").is_some(),
        || "no visible window titled Main".into(),
    );
}

fn step_quit() {
    // Quit is a close request for the main window, which ends the run.
    NSApplication::sharedApplication(mtm()).terminate(None);
}

const STEPS: &[fn()] = &[
    step_second_is_open,
    step_close_second,
    step_second_is_closed,
    step_quit,
];

fn main() -> ExitCode {
    let options = MacRunnerOptions {
        window: WindowOptions::default()
            .with_title("Main")
            .with_inner_size(WindowSize::Logical(Size::new(400.0, 300.0))),
        ..MacRunnerOptions::default()
    };

    // One step at a time on the main thread, with the run loop free between
    // them: closing the window takes the frame after the close request.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        for &step in STEPS {
            DispatchQueue::main().exec_async(step);
            std::thread::sleep(Duration::from_millis(500));
        }
    });

    if let Err(error) = xui_macos::runner(root, Some(options)).run() {
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
