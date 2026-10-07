//! An end-to-end check of the macOS host against real AppKit.
//!
//! ```sh
//! cargo run -p xui-macos --example smoke
//! ```
//!
//! Opens a window for a few seconds, drives it with synthesized `NSEvent`s and
//! platform commands, asserts what came out, prints one line per check, and exits
//! non-zero if any failed. Needs a logged-in desktop session, and takes over the
//! clipboard briefly (it puts the original back). It does not toggle fullscreen,
//! open URLs, or start a window drag, because those are visible and, for the
//! drag, modal.

use std::cell::RefCell;
use std::convert::Infallible;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSPasteboard, NSPasteboardTypeString,
};
use objc2_foundation::{NSPoint, NSProcessInfo, NSString};
use xui_core::App;
use xui_core::platform::PlatformHandle;
use xui_core::prelude::*;
use xui_core::render::{BuiltFrame, MockRenderBackend, RenderBackend};
use xui_core::ticker::{Tick, Ticker};
use xui_core::widgets::{TextController, TextSelection, text_input};
use xui_f::FBackend;
use xui_interface::events::{EventResult, PhysicalKey};
use xui_interface::{
    ClipboardContent, ClipboardTarget, ContextMenuTrigger, PlatformCommand, PlatformReply,
    PlatformRequest, ShortcutModifiers, Size, WindowCommand, commands,
};
use xui_macos::{MacRunner, MacRunnerOptions, MacWindow, WindowOptions, WindowSize};
use xui_shell::{PlatformWindow, SurfaceTarget};

/// What the application inside the window saw.
#[derive(Default)]
struct Seen {
    commands: Vec<(CommandId, bool)>,
    context_triggers: Vec<ContextMenuTrigger>,
    press_buttons: Vec<Option<PointerButton>>,
    move_stamps: Vec<Instant>,
    leaves: u32,
    replies: Vec<PlatformReply>,
}

struct Smoke {
    window: std::sync::Arc<MacWindow>,
    platform: PlatformHandle,
    seen: Seen,
    results: Vec<(&'static str, Result<(), String>)>,
    original_clipboard: Option<String>,
    /// Set when the window cannot be seen, so nothing further runs.
    aborted: bool,
}

thread_local! {
    static SMOKE: RefCell<Option<Smoke>> = const { RefCell::new(None) };
    /// Each frame the backend was asked to paint: its size, and whether the host
    /// said a resize was in progress at that moment.
    static PAINTS: RefCell<Vec<(Size<f32>, bool)>> = const { RefCell::new(Vec::new()) };
    /// The delta of every frame the application's ticker saw.
    static TICKS: RefCell<Vec<Duration>> = const { RefCell::new(Vec::new()) };
    static TICKER: RefCell<Option<Ticker>> = const { RefCell::new(None) };
    static INPUT: TextController = TextController::new();
}

/// A backend that paints nothing and notes when it was asked to, and what the
/// host said about the window at that moment.
struct Probe {
    inner: MockRenderBackend,
    window: Arc<MacWindow>,
}

impl<T> RenderBackend<T> for Probe {
    type Error = Infallible;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Infallible> {
        PAINTS.with(|paints| {
            paints
                .borrow_mut()
                .push((size, self.window.in_live_resize()));
        });
        RenderBackend::<T>::begin_frame(&mut self.inner, size)
    }

    fn submit(&mut self, frame: &BuiltFrame, text: &mut T) -> Result<(), Infallible> {
        RenderBackend::<T>::submit(&mut self.inner, frame, text)
    }

    fn end_frame(&mut self) -> Result<(), Infallible> {
        RenderBackend::<T>::end_frame(&mut self.inner)
    }
}

fn with<R>(f: impl FnOnce(&mut Smoke) -> R) -> R {
    SMOKE.with(|slot| f(slot.borrow_mut().as_mut().expect("smoke state is set")))
}

fn record(name: &'static str, ok: bool, detail: impl FnOnce() -> String) {
    let result = if ok { Ok(()) } else { Err(detail()) };
    with(|smoke| smoke.results.push((name, result)));
}

fn root(cx: &mut HookContext<'_>) -> ElementDesc {
    // Keeps frames coming at the display's rate, and records how far apart the
    // host says they are.
    let ticker = cx.use_ticker(|frame| {
        TICKS.with(|ticks| ticks.borrow_mut().push(frame.delta()));
        Tick::Continue
    });
    TICKER.with(|slot| *slot.borrow_mut() = Some(ticker));
    container()
        .style(Style::new().width(500.0).height(400.0))
        .tab_index(0)
        .shortcut(
            Shortcut::physical(PhysicalKey::KeyK).modifiers(ShortcutModifiers {
                primary: true,
                ..ShortcutModifiers::default()
            }),
            CommandId("smoke.k"),
        )
        .on_command(|event, _| {
            with(|smoke| {
                smoke
                    .seen
                    .commands
                    .push((event.command, event.shortcut.is_some()));
            });
            EventResult::Consumed
        })
        .on_context_menu(|event, _| with(|smoke| smoke.seen.context_triggers.push(event.trigger)))
        .on_press_start(|event, _| {
            with(|smoke| smoke.seen.press_buttons.push(event.pointer.button))
        })
        .on_pointer_move(|event, _| with(|smoke| smoke.seen.move_stamps.push(event.meta.timestamp)))
        .on_pointer_leave(|_, _| with(|smoke| smoke.seen.leaves += 1))
        .into_element_desc(vec![
            text_input()
                .controller(INPUT.with(Clone::clone))
                .style(Style::new().width(200.0).height(24.0))
                .into_element_desc(),
        ])
}

fn main() -> ExitCode {
    let options = MacRunnerOptions {
        window: WindowOptions::default()
            .with_title("xui smoke")
            .with_inner_size(WindowSize::Logical(Size::new(500.0, 400.0))),
        ..MacRunnerOptions::default()
    };
    let runner = MacRunner::with_options(
        |window| {
            let app = App::new(root);
            SMOKE.with(|slot| {
                *slot.borrow_mut() = Some(Smoke {
                    window: window.clone(),
                    platform: app.platform().clone(),
                    seen: Seen::default(),
                    results: Vec::new(),
                    original_clipboard: None,
                    aborted: false,
                });
            });
            let backend = Probe {
                inner: MockRenderBackend::default(),
                window,
            };
            (app, FBackend::new(), backend)
        },
        options,
    );

    // The steps run on the main thread, one after another, with the run loop
    // free in between: a command the application leaves is only flushed when the
    // shell next settles, so a step that asks and the step that checks are apart.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        for &step in STEPS {
            DispatchQueue::main().exec_async(move || {
                if !with(|smoke| smoke.aborted) {
                    step();
                }
            });
            std::thread::sleep(Duration::from_millis(350));
        }
    });

    if let Err(error) = runner.run() {
        eprintln!("the runner failed: {error}");
        return ExitCode::FAILURE;
    }

    let results = SMOKE.with(|slot| slot.borrow_mut().take().map(|smoke| smoke.results));
    let results = results.unwrap_or_default();
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

const STEPS: &[fn()] = &[
    step_focus,
    step_menu_structure,
    step_edit_menu_command,
    step_bound_shortcut_wins,
    step_ask_title_and_clipboard,
    step_check_title_and_clipboard,
    step_pointer_events,
    step_native_timestamps,
    step_text_focus,
    step_text_copy,
    step_text_paste_key,
    step_text_menu_route,
    step_quiet_begin,
    step_quiet_idle,
    step_quiet_idle,
    step_quiet_end,
    step_external_resize,
    step_check_external_resize,
    step_window_commands,
    step_check_window_commands,
    step_finish,
];

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("steps run on the main thread")
}

fn nudge() {
    // Any frame settles the shell, which is what flushes queued commands.
    with(|smoke| smoke.window.request_redraw());
}

fn uptime() -> f64 {
    NSProcessInfo::processInfo().systemUptime()
}

fn key_event(
    characters: &str,
    code: u16,
    flags: NSEventModifierFlags,
    stamp: f64,
) -> objc2::rc::Retained<NSEvent> {
    let number = with(|smoke| smoke.window.ns_window(mtm()).windowNumber());
    NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        NSPoint::new(0.0, 0.0),
        flags,
        stamp,
        number,
        None,
        &NSString::from_str(characters),
        &NSString::from_str(characters),
        false,
        code,
    )
    .expect("a key event")
}

fn mouse_event(
    kind: NSEventType,
    flags: NSEventModifierFlags,
    stamp: f64,
) -> objc2::rc::Retained<NSEvent> {
    mouse_event_at(kind, flags, stamp, NSPoint::new(50.0, 300.0))
}

fn mouse_event_at(
    kind: NSEventType,
    flags: NSEventModifierFlags,
    stamp: f64,
    at: NSPoint,
) -> objc2::rc::Retained<NSEvent> {
    let number = with(|smoke| smoke.window.ns_window(mtm()).windowNumber());
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        at,
        flags,
        stamp,
        number,
        None,
        0,
        1,
        1.0,
    )
    .expect("a mouse event")
}

const KEY_A: u16 = 0;
const KEY_Z: u16 = 6;
const KEY_V: u16 = 9;
const KEY_TAB: u16 = 48;
const KEY_C: u16 = 8;
const KEY_K: u16 = 40;

fn step_focus() {
    let mtm = mtm();
    let window = with(|smoke| smoke.window.clone());
    window.ns_window(mtm).makeKeyAndOrderFront(None);

    // Frames come from the display link, which AppKit does not run for a window
    // nobody can see: a locked screen, a sleeping display, another Space. Every
    // check that waits for a frame would then fail for a reason that has nothing
    // to do with the host, so say so first, plainly.
    let ns_window = window.ns_window(mtm);
    let visible = ns_window
        .occlusionState()
        .contains(objc2_app_kit::NSWindowOcclusionState::Visible);
    let active = NSApplication::sharedApplication(mtm).isActive();
    println!(
        "info  window visible on screen: {visible}, key: {}, application active: {active}",
        ns_window.isKeyWindow()
    );
    record(
        "the window is visible on screen, so frames and key events can be driven",
        visible,
        || {
            "the window is occluded: the screen is locked or asleep, or the window is covered"
                .to_owned()
        },
    );
    if !visible {
        // Nothing below can run honestly, and some of it would take the clipboard.
        println!("info  stopping: run this with the screen on and unlocked");
        with(|smoke| smoke.aborted = true);
        NSApplication::sharedApplication(mtm).terminate(None);
        return;
    }

    let saved = NSPasteboard::generalPasteboard()
        // SAFETY: a constant AppKit defines.
        .stringForType(unsafe { NSPasteboardTypeString })
        .map(|text| text.to_string());
    with(|smoke| smoke.original_clipboard = saved);

    // A plain Tab goes to `keyDown:` and focuses the root, which the keyboard
    // shortcuts below need.
    NSApplication::sharedApplication(mtm).sendEvent(&key_event(
        "\t",
        KEY_TAB,
        NSEventModifierFlags::empty(),
        uptime(),
    ));
}

fn step_menu_structure() {
    let mtm = mtm();
    let menu = NSApplication::sharedApplication(mtm).mainMenu();
    let titles: Vec<String> = menu
        .as_ref()
        .map(|menu| {
            menu.itemArray()
                .iter()
                .filter_map(|item| item.submenu())
                .map(|submenu| submenu.title().to_string())
                .collect()
        })
        .unwrap_or_default();
    record(
        "the menu bar is the application menu, Edit and Window",
        titles.len() == 3 && titles[1] == "Edit" && titles[2] == "Window",
        || format!("{titles:?}"),
    );

    let copy = menu.as_ref().and_then(|menu| {
        let edit = menu.itemArray().iter().nth(1)?.submenu()?;
        edit.itemArray()
            .iter()
            .find(|item| item.title().to_string() == "Copy")
    });
    let described = copy.as_ref().map(|item| {
        (
            item.keyEquivalent().to_string(),
            item.keyEquivalentModifierMask(),
            item.target().is_none(),
        )
    });
    record(
        "Copy is Cmd+C and has no target, so the responder chain answers it",
        matches!(&described, Some((key, mask, true))
            if key == "c" && *mask == NSEventModifierFlags::Command),
        || format!("{described:?}"),
    );

    let view = with(|smoke| smoke.window.ns_view(mtm).trackingAreas().count());
    record(
        "the view tracks the pointer entering and leaving",
        view == 1,
        || format!("{view} tracking areas"),
    );
}

fn step_edit_menu_command() {
    let mtm = mtm();
    with(|smoke| smoke.seen.commands.clear());
    // Nothing binds Cmd+C, so the runtime declines it and AppKit goes on to the
    // Edit menu, whose Copy walks the responder chain to the view.
    NSApplication::sharedApplication(mtm).sendEvent(&key_event(
        "c",
        KEY_C,
        NSEventModifierFlags::Command,
        uptime(),
    ));
}

fn step_bound_shortcut_wins() {
    let mtm = mtm();
    let got = with(|smoke| std::mem::take(&mut smoke.seen.commands));
    // `false` is "no shortcut behind it": the platform raised the command.
    record(
        "Cmd+C reached the runtime as the standard Copy command, raised by the platform",
        got == [(commands::COPY, false)],
        || format!("{got:?}"),
    );

    // Cmd+K is bound on the focused node: the runtime takes it in
    // `performKeyEquivalent:`, before any menu, and it must arrive exactly once.
    NSApplication::sharedApplication(mtm).sendEvent(&key_event(
        "k",
        KEY_K,
        NSEventModifierFlags::Command,
        uptime(),
    ));
    let got = with(|smoke| std::mem::take(&mut smoke.seen.commands));
    record(
        "a bound Cmd+K reaches the runtime once, as its own command with its shortcut",
        got == [(CommandId("smoke.k"), true)],
        || format!("{got:?}"),
    );
}

fn step_ask_title_and_clipboard() {
    let platform = with(|smoke| smoke.platform.clone());
    let marker = format!("xui-smoke-{}", std::process::id());
    platform.set_title("smoke title");
    platform.send(PlatformCommand::WriteClipboard {
        target: ClipboardTarget::General,
        content: ClipboardContent::Text(marker),
    });
    nudge();
}

fn step_check_title_and_clipboard() {
    let mtm = mtm();
    let title = with(|smoke| smoke.window.ns_window(mtm).title().to_string());
    record(
        "set_title reached the window",
        title == "smoke title",
        || format!("{title:?}"),
    );

    let expected = format!("xui-smoke-{}", std::process::id());
    let on_board = NSPasteboard::generalPasteboard()
        // SAFETY: a constant AppKit defines.
        .stringForType(unsafe { NSPasteboardTypeString })
        .map(|text| text.to_string());
    record(
        "a clipboard write reached the pasteboard",
        on_board.as_deref() == Some(expected.as_str()),
        || format!("{on_board:?}"),
    );

    // And read it back through the request path, which replies on the spot.
    let platform = with(|smoke| smoke.platform.clone());
    platform.request(
        PlatformRequest::ReadClipboard(ClipboardTarget::General),
        |reply| with(|smoke| smoke.seen.replies.push(reply)),
    );
    platform.request(
        PlatformRequest::ReadClipboard(ClipboardTarget::Selection),
        |reply| with(|smoke| smoke.seen.replies.push(reply)),
    );
    nudge();
}

fn step_pointer_events() {
    let mtm = mtm();
    let replies = with(|smoke| std::mem::take(&mut smoke.seen.replies));
    let expected = format!("xui-smoke-{}", std::process::id());
    record(
        "a clipboard read answers with what is on the pasteboard",
        replies.first()
            == Some(&PlatformReply::Clipboard(Some(ClipboardContent::Text(
                expected,
            )))),
        || format!("{replies:?}"),
    );
    record(
        "the primary selection, which macOS lacks, is answered Unsupported",
        replies.get(1) == Some(&PlatformReply::Unsupported),
        || format!("{replies:?}"),
    );

    let view = with(|smoke| smoke.window.clone());
    let view = view.ns_view(mtm);
    with(|smoke| {
        smoke.seen.context_triggers.clear();
        smoke.seen.press_buttons.clear();
        smoke.seen.leaves = 0;
    });

    // Hover first, so there is something to leave.
    view.mouseMoved(&mouse_event(
        NSEventType::MouseMoved,
        NSEventModifierFlags::empty(),
        uptime(),
    ));

    view.rightMouseDown(&mouse_event(
        NSEventType::RightMouseDown,
        NSEventModifierFlags::empty(),
        uptime(),
    ));
    view.rightMouseUp(&mouse_event(
        NSEventType::RightMouseUp,
        NSEventModifierFlags::empty(),
        uptime(),
    ));
    let triggers = with(|smoke| smoke.seen.context_triggers.clone());
    record(
        "a right click requests a context menu",
        triggers == [ContextMenuTrigger::PointerSecondary],
        || format!("{triggers:?}"),
    );

    view.mouseDown(&mouse_event(
        NSEventType::LeftMouseDown,
        NSEventModifierFlags::Control,
        uptime(),
    ));
    view.mouseUp(&mouse_event(
        NSEventType::LeftMouseUp,
        NSEventModifierFlags::Control,
        uptime(),
    ));
    let (triggers, buttons) = with(|smoke| {
        (
            smoke.seen.context_triggers.clone(),
            smoke.seen.press_buttons.clone(),
        )
    });
    record(
        "a control-click is a right click, press and release",
        triggers.len() == 2 && buttons.last() == Some(&Some(PointerButton::Secondary)),
        || format!("{triggers:?} {buttons:?}"),
    );

    // The exit event is made by AppKit from the tracking area; here the handler is
    // called directly with one that looks like it.
    let exit = unsafe {
        NSEvent::enterExitEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_trackingNumber_userData(
            NSEventType::MouseExited,
            NSPoint::new(-5.0, 300.0),
            NSEventModifierFlags::empty(),
            uptime(),
            with(|smoke| smoke.window.ns_window(mtm).windowNumber()),
            None,
            0,
            0,
            std::ptr::null_mut(),
        )
    }
    .expect("an exit event");
    view.mouseExited(&exit);
    let leaves = with(|smoke| smoke.seen.leaves);
    record(
        "the pointer leaving the window ends hover",
        leaves >= 1,
        || format!("{leaves} leave events"),
    );
}

fn step_native_timestamps() {
    let mtm = mtm();
    with(|smoke| smoke.seen.move_stamps.clear());
    let view = with(|smoke| smoke.window.clone());
    // Seen from AppKit as two seconds old, at a spot the pointer is not already
    // at, or the move would be a no-op: the runtime should be told it is old.
    let event_at = mouse_event_at(
        NSEventType::MouseMoved,
        NSEventModifierFlags::empty(),
        uptime() - 2.0,
        NSPoint::new(120.0, 250.0),
    );
    view.ns_view(mtm).mouseMoved(&event_at);
    let stamps = with(|smoke| smoke.seen.move_stamps.clone());
    let age = stamps
        .last()
        .map(|stamp| Instant::now().saturating_duration_since(*stamp));
    record(
        "an event carries the time AppKit gave it, not the time it was handled",
        age.is_some_and(|age| {
            age > Duration::from_millis(1900) && age < Duration::from_millis(2400)
        }),
        || format!("{age:?}"),
    );
}

/// What the text input holds, for the checks below.
fn input_text() -> String {
    INPUT.with(|controller| controller.text())
}

fn put_on_clipboard(text: &str) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: a constant AppKit defines.
    let _ =
        pasteboard.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
}

fn on_clipboard() -> Option<String> {
    NSPasteboard::generalPasteboard()
        // SAFETY: a constant AppKit defines.
        .stringForType(unsafe { NSPasteboardTypeString })
        .map(|text| text.to_string())
}

fn command_key(characters: &str, code: u16) {
    NSApplication::sharedApplication(mtm()).sendEvent(&key_event(
        characters,
        code,
        NSEventModifierFlags::Command,
        uptime(),
    ));
}

/// Clicking in the input focuses it, as it would a user's press.
fn step_text_focus() {
    let mtm = mtm();
    INPUT.with(|controller| controller.set_text("xui-smoke-hello"));
    let window = with(|smoke| smoke.window.clone());
    let view = window.ns_view(mtm);
    // The input is the top-left 200x24 of a 400-high flipped view, which is the
    // bottom-left corner's mirror in window coordinates.
    let at = NSPoint::new(20.0, 388.0);
    for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
        let event = mouse_event_at(kind, NSEventModifierFlags::empty(), uptime(), at);
        if kind == NSEventType::LeftMouseDown {
            view.mouseDown(&event);
        } else {
            view.mouseUp(&event);
        }
    }
}

fn step_text_copy() {
    // Select all, then copy: both are Command keys the text input claims before
    // the menu bar would see them.
    command_key("a", KEY_A);
    let selection = INPUT.with(|controller| controller.selection());
    record(
        "Cmd+A in a focused text input selects everything",
        (selection.base, selection.extent) == (0, "xui-smoke-hello".chars().count()),
        || format!("{selection:?}"),
    );

    put_on_clipboard("xui-smoke-before");
    command_key("c", KEY_C);
    let board = on_clipboard();
    record(
        "Cmd+C in a text input puts the selection on the real pasteboard",
        board.as_deref() == Some("xui-smoke-hello"),
        || format!("{board:?}"),
    );
}

fn step_text_paste_key() {
    put_on_clipboard("xui-smoke-pasted\nacross two lines");
    // The whole text is still selected, so the paste replaces it.
    command_key("v", KEY_V);
    let text = input_text();
    record(
        "Cmd+V pastes from the real pasteboard over the selection, as a single line",
        text == "xui-smoke-pasted across two lines",
        || format!("{text:?}"),
    );

    command_key("z", KEY_Z);
    let text = input_text();
    record("Cmd+Z undoes the paste", text == "xui-smoke-hello", || {
        format!("{text:?}")
    });
}

/// The same actions through the Edit menu's items instead of the keys: what a
/// user gets by choosing them with the mouse, and what the keys fall back to when
/// nothing claims them.
fn step_text_menu_route() {
    let mtm = mtm();
    let app = NSApplication::sharedApplication(mtm);
    let edit = app
        .mainMenu()
        .and_then(|menu| menu.itemArray().iter().nth(1))
        .and_then(|item| item.submenu());
    let paste_index = edit.as_ref().and_then(|edit| {
        edit.itemArray()
            .iter()
            .position(|item| item.title().to_string() == "Paste")
    });
    let (Some(edit), Some(paste_index)) = (edit, paste_index) else {
        record("the Edit menu has a Paste item", false, || {
            "not found".to_owned()
        });
        return;
    };

    INPUT.with(|controller| {
        controller.set_text("xui-smoke-menu: ");
        controller
            .set_selection(TextSelection::collapsed("xui-smoke-menu: ".chars().count()))
            .unwrap();
    });
    put_on_clipboard("xui-smoke-from-the-menu");
    edit.performActionForItemAtIndex(paste_index as isize);

    let text = input_text();
    record(
        "choosing Edit > Paste pastes into the focused text input",
        text == "xui-smoke-menu: xui-smoke-from-the-menu",
        || format!("{text:?}"),
    );
}

/// Nothing is sent to the window for a second or so, so the spacing of frames is
/// the display link's alone: no input, no resize, no zoom animation to disturb it.
fn step_quiet_begin() {
    TICKS.with(|ticks| ticks.borrow_mut().clear());
}

fn step_quiet_idle() {}

fn step_quiet_end() {
    let deltas: Vec<f64> = TICKS.with(|ticks| {
        ticks
            .borrow()
            .iter()
            // The first frames after a start have no previous frame to measure
            // against.
            .skip(3)
            .filter(|delta| !delta.is_zero())
            .map(Duration::as_secs_f64)
            .collect()
    });
    if deltas.len() > 30 {
        let mean = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let variance = deltas.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / deltas.len() as f64;
        let (min, max) = deltas
            .iter()
            .fold((f64::MAX, 0.0_f64), |(lo, hi), d| (lo.min(*d), hi.max(*d)));
        println!(
            "info  frame spacing over {} frames: mean {:.2} ms, stddev {:.3} ms, min {:.2}, max {:.2}",
            deltas.len(),
            mean * 1e3,
            variance.sqrt() * 1e3,
            min * 1e3,
            max * 1e3
        );
        record(
            "frames are spaced like the display's refresh, with little jitter",
            (0.004..0.040).contains(&mean) && variance.sqrt() < 0.004,
            || format!("mean {mean:.4}s stddev {:.4}s", variance.sqrt()),
        );
    } else {
        record(
            "enough frames were seen to measure their spacing",
            false,
            || format!("{} frames", deltas.len()),
        );
    }
}

/// A resize that comes from outside the shell, as a drag does, rather than from a
/// command it is itself running: AppKit calls `windowDidResize:` synchronously,
/// and the shell paints inside it.
fn step_external_resize() {
    let mtm = mtm();
    PAINTS.with(|paints| paints.borrow_mut().clear());
    let window = with(|smoke| smoke.window.clone());
    window
        .ns_window(mtm)
        .setContentSize(objc2_foundation::NSSize::new(700.0, 450.0));
}

fn step_check_external_resize() {
    let paints = PAINTS.with(|paints| paints.borrow().clone());
    record(
        "a resize that arrives from outside is painted inside the resize callback",
        paints
            .iter()
            .any(|(size, during)| *during && size.width == 700.0 && size.height == 450.0),
        || format!("{paints:?}"),
    );
}

fn step_window_commands() {
    PAINTS.with(|paints| paints.borrow_mut().clear());
    let platform = with(|smoke| smoke.platform.clone());
    platform.send(PlatformCommand::Window(WindowCommand::SetAlwaysOnTop(true)));
    platform.send(PlatformCommand::Window(WindowCommand::SetInnerSize(
        Size::new(620.0, 430.0),
    )));
    platform.send(PlatformCommand::Window(WindowCommand::SetMaximized(true)));
    nudge();
}

fn step_check_window_commands() {
    let mtm = mtm();
    let (level, zoomed, reported) = with(|smoke| {
        let window = smoke.window.ns_window(mtm);
        (
            window.level(),
            window.isZoomed(),
            smoke.platform.window_state(),
        )
    });
    record("always-on-top raised the window level", level == 3, || {
        format!("level {level}")
    });

    let content = with(|smoke| smoke.window.ns_view(mtm).frame().size);
    record(
        "SetInnerSize resized the content area",
        // Zooming right after changes the size again, so only that it grew.
        content.width >= 620.0 && content.height >= 430.0,
        || format!("{}x{}", content.width, content.height),
    );
    record("a window the runtime zoomed is zoomed", zoomed, || {
        "not zoomed".to_owned()
    });
    record(
        "the application is told the window is maximized",
        reported.maximized,
        || format!("{reported:?}"),
    );

    // Back to ordinary, and the same through the platform's own report.
    let platform = with(|smoke| smoke.platform.clone());
    platform.send(PlatformCommand::Window(WindowCommand::SetAlwaysOnTop(
        false,
    )));
    platform.send(PlatformCommand::Window(WindowCommand::SetMaximized(false)));
    nudge();
}

fn step_finish() {
    let mtm = mtm();
    let (level, maximized) = with(|smoke| {
        (
            smoke.window.ns_window(mtm).level(),
            smoke.platform.window_state().maximized,
        )
    });
    record("always-on-top comes off again", level == 0, || {
        format!("level {level}")
    });
    record("and the maximized state is cleared", !maximized, || {
        "still maximized".to_owned()
    });

    // Put the clipboard back, but only if it still holds what this run put there:
    // never overwrite something the user copied since.
    let pasteboard = NSPasteboard::generalPasteboard();
    // SAFETY: a constant AppKit defines.
    let current = pasteboard
        .stringForType(unsafe { NSPasteboardTypeString })
        .map(|text| text.to_string());
    if current
        .as_deref()
        .is_some_and(|text| text.starts_with("xui-smoke-"))
    {
        pasteboard.clearContents();
        if let Some(original) = with(|smoke| smoke.original_clipboard.take())
            .filter(|text| !text.starts_with("xui-smoke-"))
        {
            // SAFETY: a constant AppKit defines.
            let _ = pasteboard.setString_forType(&NSString::from_str(&original), unsafe {
                NSPasteboardTypeString
            });
        }
    }
    NSApplication::sharedApplication(mtm).terminate(None);
}
