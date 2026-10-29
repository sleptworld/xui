//! A text input and the clipboard, end to end: a real `text_input` widget in a
//! real `Shell`, with a window that records what the host was asked to do and
//! answers as scripted.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use xui_core::prelude::*;
use xui_core::render::MockRenderBackend;
use xui_core::widgets::TextController;
use xui_macros::component;
use xui_f::FBackend;
use xui_interface::events::{KeyState, PhysicalKey};
use xui_interface::{
    ClipboardContent, ClipboardTarget, Modifiers, PlatformCommand, PlatformReply, PlatformRequest,
    RequestId, commands,
};
use xui_shell::{
    KeyInput, Performed, PhysicalSize, PlatformWindow, Shell, ShellEvent, ShellOptions,
    SurfaceTarget,
};

struct State {
    script: VecDeque<Performed>,
    performed: Vec<PlatformCommand>,
}

struct FakeWindow(Mutex<State>);

impl FakeWindow {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap()
    }

    fn performed(&self) -> Vec<PlatformCommand> {
        self.state().performed.clone()
    }

    fn writes(&self) -> Vec<String> {
        self.performed()
            .into_iter()
            .filter_map(|command| match command {
                PlatformCommand::WriteClipboard {
                    content: ClipboardContent::Text(text),
                    ..
                } => Some(text),
                _ => None,
            })
            .collect()
    }

    fn requests(&self) -> Vec<(RequestId, PlatformRequest)> {
        self.performed()
            .into_iter()
            .filter_map(|command| match command {
                PlatformCommand::Request(id, request) => Some((id, request)),
                _ => None,
            })
            .collect()
    }
}

impl HasWindowHandle for FakeWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::NotSupported)
    }
}

impl HasDisplayHandle for FakeWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Err(HandleError::NotSupported)
    }
}

impl SurfaceTarget for FakeWindow {
    fn surface_size(&self) -> PhysicalSize {
        PhysicalSize::new(800, 600)
    }

    fn scale_factor(&self) -> f64 {
        2.0
    }
}

impl PlatformWindow for FakeWindow {
    fn request_redraw(&self) {}
    fn set_visible(&self, _visible: bool) {}
    fn focus(&self) {}
    fn is_minimized(&self) -> Option<bool> {
        Some(false)
    }
    fn set_cursor(&self, _cursor: CursorIcon) {}
    fn set_ime_allowed(&self, _allowed: bool) {}
    fn set_ime_cursor_area(&self, _area: Rect) {}

    fn perform(&self, command: PlatformCommand) -> Performed {
        let mut state = self.state();
        state.performed.push(command);
        state.script.pop_front().unwrap_or(Performed::Unsupported)
    }
}

thread_local! {
    static CONTROLLER: RefCell<Option<TextController>> = const { RefCell::new(None) };
}

#[component]
fn field() {
    let controller: TextController = CONTROLLER.with(|slot| slot.borrow().clone().expect("controller is set"));
    view! { text_input(controller: controller.clone(), width: 200.0, height: 24.0) }
}

fn root(app: &mut App) {
    app.mount::<Field>(FieldProps {});
}

type TestShell = Shell<FakeWindow, MockRenderBackend, FBackend>;

/// A focused text input holding `text`.
fn start(text: &str) -> (TestShell, Arc<FakeWindow>, TextController) {
    let controller = TextController::with_text(text);
    CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
    let window = Arc::new(FakeWindow(Mutex::new(State {
        script: VecDeque::new(),
        performed: Vec::new(),
    })));
    let (mut shell, _) = Shell::new(
        Arc::clone(&window),
        App::new(root),
        FBackend::new(),
        MockRenderBackend::default(),
        ShellOptions::default(),
    );
    // Pressing in the input focuses it.
    let _ = shell.handle_event(ShellEvent::PointerMoved {
        position: Point::new(10.0, 10.0),
        device_id: None,
    });
    for pressed in [true, false] {
        let _ = shell.handle_event(ShellEvent::PointerButton {
            button: PointerButton::Primary,
            pressed,
            device_id: None,
        });
    }
    (shell, window, controller)
}

fn select(controller: &TextController, base: usize, extent: usize) {
    controller
        .set_selection(TextSelection::new(base, extent))
        .unwrap();
}

fn command(shell: &mut TestShell, command: CommandId) -> bool {
    shell
        .dispatch_event(Instant::now(), ShellEvent::Command(command))
        .consumed
}

fn primary() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers {
            meta: true,
            ..Modifiers::default()
        }
    } else {
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        }
    }
}

fn chord(shell: &mut TestShell, key: PhysicalKey) -> bool {
    let _ = shell.handle_event(ShellEvent::ModifiersChanged(primary()));
    let outcome = shell.dispatch_event(
        Instant::now(),
        ShellEvent::Keyboard(KeyInput {
            physical_key: key,
            named_key: None,
            state: KeyState::Down,
            text: None,
            is_repeat: false,
        }),
    );
    let _ = shell.handle_event(ShellEvent::ModifiersChanged(Modifiers::default()));
    outcome.consumed
}

fn clipboard(text: &str) -> PlatformReply {
    PlatformReply::Clipboard(Some(ClipboardContent::Text(text.to_owned())))
}

// ───────────────────────────── copy and cut ─────────────────────────────

#[test]
fn copy_writes_the_selection_to_the_clipboard() {
    let (mut shell, window, controller) = start("hello world");
    select(&controller, 6, 11);

    assert!(command(&mut shell, commands::COPY));

    assert_eq!(window.writes(), ["world"]);
    assert_eq!(controller.text(), "hello world", "copying changes nothing");
}

#[test]
fn copy_with_nothing_selected_leaves_the_clipboard_alone() {
    let (mut shell, window, controller) = start("hello");
    select(&controller, 3, 3);

    assert!(
        command(&mut shell, commands::COPY),
        "handled, so nothing else beeps"
    );
    assert!(window.writes().is_empty());
}

#[test]
fn a_backwards_selection_copies_the_same_text() {
    let (mut shell, window, controller) = start("hello world");
    select(&controller, 11, 6);

    command(&mut shell, commands::COPY);

    assert_eq!(window.writes(), ["world"]);
}

#[test]
fn cut_writes_the_selection_and_removes_it() {
    let (mut shell, window, controller) = start("hello world");
    select(&controller, 5, 11);

    assert!(command(&mut shell, commands::CUT));

    assert_eq!(window.writes(), [" world"]);
    assert_eq!(controller.text(), "hello");
}

#[test]
fn text_with_multibyte_characters_is_selected_by_character() {
    // h é l l o _ 世 界 _ o k: characters 5..8 are " 世界", though they are 9 bytes
    // wide.
    let (mut shell, window, controller) = start("héllo 世界 ok");
    select(&controller, 5, 8);

    command(&mut shell, commands::COPY);

    assert_eq!(window.writes(), [" 世界"]);
}

// ───────────────────────────── paste ─────────────────────────────

#[test]
fn paste_asks_the_host_and_inserts_what_comes_back_at_the_caret() {
    let (mut shell, window, controller) = start("ac");
    select(&controller, 1, 1);
    window
        .state()
        .script
        .push_back(Performed::Reply(clipboard("b")));

    assert!(command(&mut shell, commands::PASTE));

    assert_eq!(
        window
            .requests()
            .first()
            .map(|(_, request)| request.clone()),
        Some(PlatformRequest::ReadClipboard(ClipboardTarget::General))
    );
    assert_eq!(controller.text(), "abc");
    let caret = controller.selection();
    assert_eq!(
        (caret.base, caret.extent),
        (2, 2),
        "the caret follows the paste"
    );
}

#[test]
fn paste_over_a_selection_replaces_it() {
    let (mut shell, window, controller) = start("hello world");
    select(&controller, 6, 11);
    window
        .state()
        .script
        .push_back(Performed::Reply(clipboard("there")));

    command(&mut shell, commands::PASTE);

    assert_eq!(controller.text(), "hello there");
}

#[test]
fn a_reply_that_arrives_later_is_still_pasted() {
    let (mut shell, window, controller) = start("x");
    select(&controller, 1, 1);
    window.state().script.push_back(Performed::Pending);

    command(&mut shell, commands::PASTE);
    assert_eq!(controller.text(), "x", "still waiting on the host");

    let (id, _) = window.requests().pop().expect("a request was made");
    let _ = shell.handle_event(ShellEvent::Reply(id, clipboard("yz")));

    assert_eq!(controller.text(), "xyz");
}

#[test]
fn a_single_line_input_turns_line_breaks_and_tabs_into_spaces() {
    let (mut shell, window, controller) = start("");
    window
        .state()
        .script
        .push_back(Performed::Reply(clipboard("one\r\ntwo\n\nthree\tfour")));

    command(&mut shell, commands::PASTE);

    assert_eq!(controller.text(), "one two three four");
}

#[test]
fn pasting_nothing_changes_nothing() {
    for reply in [
        PlatformReply::Clipboard(None),
        PlatformReply::Unsupported,
        clipboard(""),
        clipboard("\n"),
    ] {
        let (mut shell, window, controller) = start("keep");
        window.state().script.push_back(Performed::Reply(reply));
        command(&mut shell, commands::PASTE);
        assert_eq!(controller.text(), "keep");
    }
}

#[test]
fn a_host_with_no_clipboard_leaves_the_text_alone() {
    // The default answer to a request is "unsupported".
    let (mut shell, _window, controller) = start("keep");
    command(&mut shell, commands::PASTE);
    assert_eq!(controller.text(), "keep");
}

#[test]
fn a_reply_nobody_asked_for_is_dropped_before_it_reaches_the_input() {
    // Not the widget's own id check, which `a_second_paste_is_the_one_that_counts`
    // covers: an id nothing registered never gets as far as the widget.
    let (mut shell, window, controller) = start("keep");
    window.state().script.push_back(Performed::Pending);
    command(&mut shell, commands::PASTE);

    let _ = shell.handle_event(ShellEvent::Reply(RequestId(9_999), clipboard("intruder")));

    assert_eq!(controller.text(), "keep");
}

#[test]
fn a_second_paste_is_the_one_that_counts() {
    let (mut shell, window, controller) = start("");
    window
        .state()
        .script
        .extend([Performed::Pending, Performed::Pending]);
    command(&mut shell, commands::PASTE);
    command(&mut shell, commands::PASTE);
    let requests = window.requests();
    let (first, second) = (requests[0].0, requests[1].0);

    let _ = shell.handle_event(ShellEvent::Reply(first, clipboard("old")));
    assert_eq!(controller.text(), "", "the first request was superseded");
    let _ = shell.handle_event(ShellEvent::Reply(second, clipboard("new")));
    assert_eq!(controller.text(), "new");
}

// ───────────────────────────── select all, undo, redo ─────────────────────────────

#[test]
fn select_all_selects_everything() {
    let (mut shell, _window, controller) = start("hello");
    select(&controller, 2, 2);

    assert!(command(&mut shell, commands::SELECT_ALL));

    let selection = controller.selection();
    assert_eq!((selection.base, selection.extent), (0, 5));
}

#[test]
fn undo_and_redo_commands_step_through_edits() {
    let (mut shell, window, controller) = start("");
    window
        .state()
        .script
        .push_back(Performed::Reply(clipboard("pasted")));
    command(&mut shell, commands::PASTE);
    assert_eq!(controller.text(), "pasted");

    assert!(command(&mut shell, commands::UNDO));
    assert_eq!(controller.text(), "");

    assert!(command(&mut shell, commands::REDO));
    assert_eq!(controller.text(), "pasted");
}

#[test]
fn a_command_the_input_does_not_know_is_not_consumed() {
    let (mut shell, _window, _controller) = start("hello");
    assert!(!command(&mut shell, CommandId("app.save")));
}

// ───────────────────────────── the same actions from the keyboard ─────────────────────────────

#[test]
fn the_copy_key_does_what_the_copy_command_does_and_is_consumed() {
    let (mut shell, window, controller) = start("hello world");
    select(&controller, 0, 5);

    // Consumed is what lets a host skip its menu bar's own Copy.
    assert!(chord(&mut shell, PhysicalKey::KeyC));

    assert_eq!(window.writes(), ["hello"]);
}

#[test]
fn the_cut_and_paste_keys_work_too() {
    let (mut shell, window, controller) = start("hello world");
    select(&controller, 0, 6);
    assert!(chord(&mut shell, PhysicalKey::KeyX));
    assert_eq!(window.writes(), ["hello "]);
    assert_eq!(controller.text(), "world");

    window
        .state()
        .script
        .push_back(Performed::Reply(clipboard("brave ")));
    assert!(chord(&mut shell, PhysicalKey::KeyV));
    assert_eq!(controller.text(), "brave world");
}
