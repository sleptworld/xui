//! The shell's contract with a host, exercised through its public API only: a
//! window that records what it was asked to do, and a handful of small
//! applications that observe what reached them.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use xui_core::app::ComponentFn;
use xui_core::prelude::*;
use xui_core::render::MockRenderBackend;
use xui_core::ticker::{Tick, Ticker};
use xui_f::FBackend;
use xui_interface::events::{EventResult, KeyState, NamedKey, PhysicalKey};
use xui_interface::{
    Appearance, ClipboardContent, ClipboardTarget, ColorScheme, ContextMenuTrigger, DragEvent,
    DropAccepts, DropEffect, DropTarget, GestureKind, GesturePhase, HostCapabilities, MenuBar,
    PlatformCommand, PlatformError, PlatformReply, PlatformRequest, PublishedSurface, RegionRect,
    RequestId, WindowRegion, WindowState,
};
use xui_shell::{
    FrameInfo, KeyInput, Performed, PhysicalSize, PlatformWindow, Shell, ShellControl, ShellEvent,
    ShellOptions, SurfaceTarget, TouchPhase,
};

#[derive(Debug, Clone, PartialEq)]
enum Call {
    RequestRedraw,
    SetCursor(CursorIcon),
    SetTitle(String),
    SetMenuBar(Option<MenuBar>),
    Perform(PlatformCommand),
}

struct State {
    size: PhysicalSize,
    capabilities: HostCapabilities,
    appearance: Appearance,
    /// What `perform` answers, in order. Empty means "not supported".
    script: VecDeque<Performed>,
    calls: Vec<Call>,
}

struct FakeWindow(Mutex<State>);

impl FakeWindow {
    fn new() -> Self {
        Self(Mutex::new(State {
            size: PhysicalSize::new(800, 600),
            capabilities: HostCapabilities::NONE,
            appearance: Appearance::default(),
            script: VecDeque::new(),
            calls: Vec::new(),
        }))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap()
    }

    fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }

    fn performed(&self) -> Vec<PlatformCommand> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Perform(command) => Some(command),
                _ => None,
            })
            .collect()
    }

    fn script(&self, answers: impl IntoIterator<Item = Performed>) {
        self.state().script.extend(answers);
    }

    fn record(&self, call: Call) {
        self.state().calls.push(call);
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
        self.state().size
    }

    fn scale_factor(&self) -> f64 {
        2.0
    }
}

impl PlatformWindow for FakeWindow {
    fn request_redraw(&self) {
        self.record(Call::RequestRedraw);
    }
    fn set_visible(&self, _visible: bool) {}
    fn focus(&self) {}
    fn is_minimized(&self) -> Option<bool> {
        Some(false)
    }
    fn set_cursor(&self, cursor: CursorIcon) {
        self.record(Call::SetCursor(cursor));
    }
    fn set_ime_allowed(&self, _allowed: bool) {}
    fn set_ime_cursor_area(&self, _area: Rect) {}

    fn capabilities(&self) -> HostCapabilities {
        self.state().capabilities
    }

    fn appearance(&self) -> Appearance {
        self.state().appearance
    }

    fn set_title(&self, title: &str) {
        self.record(Call::SetTitle(title.to_owned()));
    }

    fn set_menu_bar(&self, menu_bar: Option<&MenuBar>) {
        self.record(Call::SetMenuBar(menu_bar.cloned()));
    }

    fn perform(&self, command: PlatformCommand) -> Performed {
        let mut state = self.state();
        state.calls.push(Call::Perform(command));
        state.script.pop_front().unwrap_or(Performed::Unsupported)
    }
}

type TestShell = Shell<FakeWindow, MockRenderBackend, FBackend>;

fn empty(_cx: &mut HookContext<'_>) -> ElementDesc {
    container().into_element_desc(Vec::new())
}

fn start(root: ComponentFn) -> (TestShell, Arc<FakeWindow>) {
    start_with(root, |_| {})
}

fn start_with(
    root: ComponentFn,
    configure: impl FnOnce(&FakeWindow),
) -> (TestShell, Arc<FakeWindow>) {
    let window = Arc::new(FakeWindow::new());
    configure(&window);
    let (shell, control) = Shell::new(
        Arc::clone(&window),
        App::new(root),
        FBackend::new(),
        MockRenderBackend::default(),
        ShellOptions::default(),
    );
    assert_eq!(control, ShellControl::Continue);
    (shell, window)
}

/// Any event will do to run the shell's settle step.
fn nudge(shell: &mut TestShell) {
    let _ = shell.handle_event(ShellEvent::Focused(true));
}

fn platform(shell: &TestShell) -> xui_core::platform::PlatformHandle {
    shell.runtime().app().platform().clone()
}

fn clipboard_read() -> PlatformRequest {
    PlatformRequest::ReadClipboard(ClipboardTarget::General)
}

fn text(value: &str) -> PlatformReply {
    PlatformReply::Clipboard(Some(ClipboardContent::Text(value.to_owned())))
}

// ───────────────────────────── host → application ─────────────────────────────

#[test]
fn the_application_is_told_what_the_host_can_do_and_how_the_system_looks() {
    let capabilities = HostCapabilities::CLIPBOARD | HostCapabilities::FULLSCREEN;
    let appearance = Appearance {
        color_scheme: ColorScheme::Dark,
        high_contrast: true,
        ..Appearance::default()
    };
    let (shell, _) = start_with(empty, |window| {
        window.state().capabilities = capabilities;
        window.state().appearance = appearance;
    });

    let handle = platform(&shell);
    assert_eq!(handle.capabilities(), capabilities);
    assert_eq!(handle.appearance(), appearance);
}

#[test]
fn appearance_and_window_state_changes_reach_the_application() {
    let (mut shell, _) = start(empty);
    let handle = platform(&shell);

    let dark = Appearance {
        color_scheme: ColorScheme::Dark,
        ..Appearance::default()
    };
    let _ = shell.handle_event(ShellEvent::AppearanceChanged(dark));
    let _ = shell.handle_event(ShellEvent::WindowStateChanged(WindowState {
        fullscreen: true,
        maximized: false,
    }));

    assert_eq!(handle.appearance(), dark);
    assert!(handle.window_state().fullscreen);
}

#[test]
fn gestures_and_drags_are_accepted_even_though_no_widget_consumes_them_yet() {
    let (mut shell, _) = start(empty);
    for event in [
        ShellEvent::Gesture {
            kind: GestureKind::Magnify(0.1),
            phase: GesturePhase::Changed,
            position: Point::new(10.0, 10.0),
        },
        ShellEvent::Drag(DragEvent::Leave),
    ] {
        assert_eq!(shell.handle_event(event), ShellControl::Continue);
    }
}

// ───────────────────────────── state the runtime wants ─────────────────────────────

#[test]
fn a_title_is_applied_once_and_again_only_when_it_changes() {
    let (mut shell, window) = start(empty);
    let handle = platform(&shell);

    handle.set_title("Report");
    nudge(&mut shell);
    nudge(&mut shell);
    handle.set_title("Report v2");
    nudge(&mut shell);

    let titles: Vec<_> = window
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::SetTitle(title) => Some(title),
            _ => None,
        })
        .collect();
    assert_eq!(titles, ["Report", "Report v2"]);
}

#[test]
fn the_menu_bar_is_applied_and_can_be_handed_back_to_the_host() {
    let (mut shell, window) = start(empty);
    let handle = platform(&shell);

    handle.set_menu_bar(Some(MenuBar::default()));
    nudge(&mut shell);
    handle.set_menu_bar(None);
    nudge(&mut shell);

    let menus: Vec<_> = window
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::SetMenuBar(menu) => Some(menu),
            _ => None,
        })
        .collect();
    assert_eq!(menus, [Some(MenuBar::default()), None]);
}

#[test]
fn published_surface_follows_what_the_application_sets_and_is_readable_without_the_shell() {
    let (mut shell, _) = start(empty);
    let published = shell.published();
    assert_eq!(
        published.region_at(Point::new(5.0, 5.0)),
        WindowRegion::Content
    );

    platform(&shell).set_surface(PublishedSurface {
        regions: Arc::from([RegionRect {
            rect: Rect::new(0.0, 0.0, 100.0, 30.0),
            region: WindowRegion::Caption,
        }]),
        drop_targets: Arc::from([DropTarget {
            rect: Rect::new(0.0, 100.0, 100.0, 100.0),
            accepts: DropAccepts {
                files: true,
                text: false,
            },
            effect: DropEffect::Copy,
        }]),
        text_document: None,
    });
    nudge(&mut shell);

    // The handle was taken before the surface changed, and still sees it.
    assert_eq!(
        published.region_at(Point::new(5.0, 5.0)),
        WindowRegion::Caption
    );
    let files = DropAccepts {
        files: true,
        text: false,
    };
    assert_eq!(
        published.drop_effect_at(Point::new(5.0, 150.0), files),
        DropEffect::Copy
    );
}

// ───────────────────────────── commands and requests ─────────────────────────────

#[test]
fn commands_reach_the_host_in_order_after_the_event_that_queued_them() {
    let (mut shell, window) = start(empty);
    window.script([Performed::Done, Performed::Done]);
    let handle = platform(&shell);

    handle.send(PlatformCommand::OpenUrl("https://example.com".into()));
    handle.send(PlatformCommand::WriteClipboard {
        target: ClipboardTarget::General,
        content: ClipboardContent::Text("copied".into()),
    });
    assert!(
        window.performed().is_empty(),
        "nothing is sent until the shell settles"
    );

    nudge(&mut shell);

    assert_eq!(
        window.performed(),
        vec![
            PlatformCommand::OpenUrl("https://example.com".into()),
            PlatformCommand::WriteClipboard {
                target: ClipboardTarget::General,
                content: ClipboardContent::Text("copied".into()),
            },
        ]
    );

    nudge(&mut shell);
    assert_eq!(window.performed().len(), 2, "and each is sent once");
}

#[test]
fn a_command_the_host_does_not_implement_is_dropped_quietly() {
    let (mut shell, window) = start(empty);
    platform(&shell).send(PlatformCommand::OpenUrl("https://example.com".into()));
    nudge(&mut shell);
    assert_eq!(window.performed().len(), 1);
}

/// Collects the replies a set of requests receive, in the order they arrive.
#[derive(Clone, Default)]
struct Replies(Rc<RefCell<Vec<PlatformReply>>>);

impl Replies {
    fn request(&self, shell: &TestShell, request: PlatformRequest) -> RequestId {
        let replies = Rc::clone(&self.0);
        platform(shell).request(request, move |reply| replies.borrow_mut().push(reply))
    }

    fn all(&self) -> Vec<PlatformReply> {
        self.0.borrow().clone()
    }
}

#[test]
fn a_request_the_host_does_not_support_is_answered_so_its_caller_is_not_left_waiting() {
    let (mut shell, _) = start(empty);
    let replies = Replies::default();
    let _ = replies.request(&shell, clipboard_read());

    nudge(&mut shell);

    assert_eq!(replies.all(), vec![PlatformReply::Unsupported]);
}

#[test]
fn a_reply_given_on_the_spot_is_delivered_after_the_whole_round_has_been_performed() {
    let (mut shell, window) = start(empty);
    window.script([Performed::Reply(text("pasted")), Performed::Done]);

    // Records how many commands the host had been handed when the reply landed.
    let performed_at_reply = Rc::new(Cell::new(0));
    let handle = platform(&shell);
    handle.request(clipboard_read(), {
        let window = Arc::clone(&window);
        let performed_at_reply = Rc::clone(&performed_at_reply);
        move |_| performed_at_reply.set(window.performed().len())
    });
    handle.send(PlatformCommand::OpenUrl("https://example.com".into()));

    nudge(&mut shell);

    assert_eq!(
        performed_at_reply.get(),
        2,
        "the reply was not delivered from inside `perform`, where the runtime is mid-handover"
    );
}

#[test]
fn a_pending_request_is_answered_by_a_later_reply_event() {
    let (mut shell, window) = start(empty);
    window.script([Performed::Pending]);
    let replies = Replies::default();
    let id = replies.request(&shell, clipboard_read());

    nudge(&mut shell);
    assert!(replies.all().is_empty(), "still waiting on the host");

    let _ = shell.handle_event(ShellEvent::Reply(id, text("later")));
    assert_eq!(replies.all(), vec![text("later")]);
}

#[test]
fn cancelling_a_request_tells_the_host_and_drops_a_reply_that_arrives_anyway() {
    let (mut shell, window) = start(empty);
    window.script([Performed::Pending, Performed::Done]);
    let replies = Replies::default();
    let id = replies.request(&shell, clipboard_read());
    nudge(&mut shell);

    assert!(platform(&shell).cancel(id));
    nudge(&mut shell);
    assert_eq!(
        window.performed().last(),
        Some(&PlatformCommand::Cancel(id))
    );

    let _ = shell.handle_event(ShellEvent::Reply(id, text("too late")));
    assert!(replies.all().is_empty());
}

#[test]
fn a_failed_request_reports_why_to_its_caller() {
    let (mut shell, window) = start(empty);
    let error = PlatformError::new("the clipboard is locked");
    window.script([Performed::Failed(error.clone())]);
    let replies = Replies::default();
    let _ = replies.request(&shell, clipboard_read());

    nudge(&mut shell);

    assert_eq!(replies.all(), vec![PlatformReply::Failed(error)]);
}

#[test]
fn a_request_a_host_completes_without_replying_fails_instead_of_hanging() {
    let (mut shell, window) = start(empty);
    window.script([Performed::Done]);
    let replies = Replies::default();
    let _ = replies.request(&shell, clipboard_read());

    nudge(&mut shell);

    assert!(matches!(
        replies.all().as_slice(),
        [PlatformReply::Failed(_)]
    ));
}

#[test]
fn a_reply_handler_that_asks_again_is_served_in_the_same_settle() {
    let (mut shell, window) = start(empty);
    window.script([
        Performed::Reply(text("first")),
        Performed::Reply(text("second")),
    ]);
    let handle = platform(&shell);
    let seen = Rc::new(RefCell::new(Vec::new()));

    handle.request(clipboard_read(), {
        let handle = handle.clone();
        let seen = Rc::clone(&seen);
        move |reply| {
            seen.borrow_mut().push(reply);
            let seen = Rc::clone(&seen);
            handle.request(clipboard_read(), move |reply| seen.borrow_mut().push(reply));
        }
    });

    nudge(&mut shell);

    assert_eq!(*seen.borrow(), vec![text("first"), text("second")]);
}

#[test]
fn a_request_loop_that_never_settles_is_cut_off_rather_than_spun_on() {
    let (mut shell, _) = start(empty);
    let handle = platform(&shell);
    let count = Rc::new(Cell::new(0_u32));

    fn again(handle: xui_core::platform::PlatformHandle, count: Rc<Cell<u32>>) {
        let next = handle.clone();
        handle.request(clipboard_read(), move |_| {
            count.set(count.get() + 1);
            again(next, Rc::clone(&count));
        });
    }
    again(handle, Rc::clone(&count));

    nudge(&mut shell);

    assert!(
        (2..100).contains(&count.get()),
        "answered more than once, and then stopped: {}",
        count.get()
    );
}

// ───────────────────────────── input routes ─────────────────────────────

thread_local! {
    static LEAVES: Cell<u32> = const { Cell::new(0) };
    static PRESS_CANCELS: Cell<u32> = const { Cell::new(0) };
    static CONTEXT_TRIGGERS: RefCell<Vec<ContextMenuTrigger>> = const { RefCell::new(Vec::new()) };
    static PRESSES: RefCell<Vec<(u64, bool)>> = const { RefCell::new(Vec::new()) };
    static STAMPS: RefCell<Vec<Instant>> = const { RefCell::new(Vec::new()) };
    static COMMANDS: RefCell<Vec<(CommandId, bool)>> = const { RefCell::new(Vec::new()) };
    static FRAMES: RefCell<Vec<xui_core::clock::FrameTime>> = const { RefCell::new(Vec::new()) };
    static TICKER: RefCell<Option<Ticker>> = const { RefCell::new(None) };
}

/// A 200x200 logical box that watches everything the shell can send to it.
fn watcher(_cx: &mut HookContext<'_>) -> ElementDesc {
    container()
        .style(Style::new().width(200.0).height(200.0))
        .on_pointer_leave(|_, _| LEAVES.with(|leaves| leaves.set(leaves.get() + 1)))
        .on_press_cancel(|_, _| PRESS_CANCELS.with(|cancels| cancels.set(cancels.get() + 1)))
        .on_press_start(|event, _| {
            PRESSES.with(|presses| {
                presses
                    .borrow_mut()
                    .push((event.pointer.pointer_id.0, event.pointer.is_primary));
            });
        })
        .on_context_menu(|event, _| {
            CONTEXT_TRIGGERS.with(|triggers| triggers.borrow_mut().push(event.trigger));
        })
        .on_pointer_move(|event, _| {
            STAMPS.with(|stamps| stamps.borrow_mut().push(event.meta.timestamp))
        })
        .on_command(|event, _| {
            COMMANDS.with(|commands| {
                commands
                    .borrow_mut()
                    .push((event.command, event.shortcut.is_none()));
            });
            EventResult::Consumed
        })
        .into_element_desc(Vec::new())
}

fn reset_watcher() {
    LEAVES.with(|leaves| leaves.set(0));
    PRESS_CANCELS.with(|cancels| cancels.set(0));
    CONTEXT_TRIGGERS.with(|triggers| triggers.borrow_mut().clear());
    PRESSES.with(|presses| presses.borrow_mut().clear());
    STAMPS.with(|stamps| stamps.borrow_mut().clear());
    COMMANDS.with(|commands| commands.borrow_mut().clear());
}

fn pointer_at(shell: &mut TestShell, x: f32, y: f32) {
    let _ = shell.handle_event(ShellEvent::PointerMoved {
        position: Point::new(x, y),
        device_id: None,
    });
}

#[test]
fn pointer_left_ends_hover() {
    reset_watcher();
    let (mut shell, _) = start(watcher);

    pointer_at(&mut shell, 20.0, 20.0);
    assert_eq!(LEAVES.with(Cell::get), 0);

    let _ = shell.handle_event(ShellEvent::PointerLeft);
    assert_eq!(LEAVES.with(Cell::get), 1);
}

#[test]
fn a_cancelled_pointer_cancels_the_press_it_was_holding() {
    reset_watcher();
    let (mut shell, _) = start(watcher);

    pointer_at(&mut shell, 20.0, 20.0);
    let _ = shell.handle_event(ShellEvent::PointerButton {
        button: PointerButton::Primary,
        pressed: true,
        device_id: None,
    });
    let _ = shell.handle_event(ShellEvent::PointerCancelled);

    assert_eq!(PRESS_CANCELS.with(Cell::get), 1);
}

#[test]
fn a_context_menu_request_reaches_the_node_under_it() {
    reset_watcher();
    let (mut shell, _) = start(watcher);

    let _ = shell.handle_event(ShellEvent::ContextMenuRequested {
        position: Some(Point::new(30.0, 30.0)),
        trigger: ContextMenuTrigger::PointerSecondary,
    });

    assert_eq!(
        CONTEXT_TRIGGERS.with(|triggers| triggers.borrow().clone()),
        [ContextMenuTrigger::PointerSecondary]
    );
}

#[test]
fn a_finger_is_a_pointer_of_its_own_and_not_the_primary_one() {
    reset_watcher();
    let (mut shell, _) = start(watcher);

    for phase in [TouchPhase::Started, TouchPhase::Ended] {
        let _ = shell.handle_event(ShellEvent::Touch {
            id: 7,
            phase,
            position: Point::new(40.0, 40.0),
            device_id: None,
        });
    }

    assert_eq!(
        PRESSES.with(|presses| presses.borrow().clone()),
        [(8, false)],
        "finger 7 is pointer 8; pointer 0 is the mouse"
    );
}

#[test]
fn events_carry_the_time_the_host_gave_not_the_time_they_were_handled() {
    reset_watcher();
    let (mut shell, _) = start(watcher);
    let earlier = Instant::now() - Duration::from_secs(5);

    let _ = shell.dispatch_event(
        earlier,
        ShellEvent::PointerMoved {
            position: Point::new(20.0, 20.0),
            device_id: None,
        },
    );

    assert_eq!(STAMPS.with(|stamps| stamps.borrow().clone()), [earlier]);
}

#[test]
fn a_platform_command_is_raised_with_no_shortcut_and_reports_it_was_consumed() {
    reset_watcher();
    let (mut shell, _) = start(watcher);

    let outcome = shell.dispatch_event(
        Instant::now(),
        ShellEvent::Command(xui_interface::commands::COPY),
    );

    assert!(outcome.consumed);
    assert_eq!(
        COMMANDS.with(|commands| commands.borrow().clone()),
        [(xui_interface::commands::COPY, true)]
    );
}

fn key(named: NamedKey) -> ShellEvent {
    ShellEvent::Keyboard(KeyInput {
        physical_key: PhysicalKey::Unidentified,
        named_key: Some(named),
        state: KeyState::Down,
        text: None,
        is_repeat: false,
    })
}

fn shortcut_root(_cx: &mut HookContext<'_>) -> ElementDesc {
    container()
        .style(Style::new().width(100.0).height(100.0))
        .tab_index(0)
        .shortcut(Shortcut::named(NamedKey::End), CommandId("test.end"))
        .on_command(|_, _| EventResult::Consumed)
        .into_element_desc(Vec::new())
}

#[test]
fn a_key_reports_whether_the_runtime_acted_on_it() {
    let (mut shell, _) = start(shortcut_root);
    let _ = shell.dispatch_event(Instant::now(), key(NamedKey::Tab));

    let bound = shell.dispatch_event(Instant::now(), key(NamedKey::End));
    let unbound = shell.dispatch_event(Instant::now(), key(NamedKey::F(5)));

    assert!(bound.consumed, "End is a shortcut of the focused node");
    assert!(!unbound.consumed, "nothing claims F5, so the host may");
}

// ───────────────────────────── redraw and resize ─────────────────────────────

fn redraws(window: &FakeWindow) -> usize {
    window
        .calls()
        .iter()
        .filter(|call| **call == Call::RequestRedraw)
        .count()
}

#[test]
fn a_resize_during_a_live_resize_is_painted_at_once() {
    let (mut shell, window) = start(empty);
    let _ = shell.handle_event(ShellEvent::LiveResize(true));
    let frames_before = shell.runtime().backend().frames;
    let redraws_before = redraws(&window);

    window.state().size = PhysicalSize::new(1000, 700);
    let _ = shell.handle_event(ShellEvent::Resized(PhysicalSize::new(1000, 700)));

    assert_eq!(shell.runtime().backend().frames, frames_before + 1);
    assert_eq!(
        shell.runtime().backend().frame_size,
        Some(Size::new(500.0, 350.0))
    );
    assert_eq!(
        redraws_before,
        redraws(&window),
        "it was drawn already, so no second frame is requested"
    );
}

#[test]
fn outside_a_live_resize_a_resize_asks_for_a_frame_instead_of_painting() {
    let (mut shell, window) = start(empty);
    let frames_before = shell.runtime().backend().frames;
    let redraws_before = redraws(&window);

    window.state().size = PhysicalSize::new(1000, 700);
    let _ = shell.handle_event(ShellEvent::Resized(PhysicalSize::new(1000, 700)));

    assert_eq!(shell.runtime().backend().frames, frames_before);
    assert!(redraws(&window) > redraws_before);

    let _ = shell.handle_event(ShellEvent::LiveResize(true));
    let _ = shell.handle_event(ShellEvent::LiveResize(false));
    window.state().size = PhysicalSize::new(900, 700);
    let frames = shell.runtime().backend().frames;
    let _ = shell.handle_event(ShellEvent::Resized(PhysicalSize::new(900, 700)));
    assert_eq!(
        shell.runtime().backend().frames,
        frames,
        "a finished live resize no longer paints on its own"
    );
}

fn ticking(cx: &mut HookContext<'_>) -> ElementDesc {
    let ticker = cx.use_ticker(|frame| {
        FRAMES.with(|frames| frames.borrow_mut().push(frame));
        Tick::Continue
    });
    TICKER.with(|slot| *slot.borrow_mut() = Some(ticker));
    container().into_element_desc(Vec::new())
}

#[test]
fn a_frame_is_animated_to_the_time_the_host_says_it_will_be_shown() {
    FRAMES.with(|frames| frames.borrow_mut().clear());
    let (mut shell, _) = start(ticking);
    FRAMES.with(|frames| frames.borrow_mut().clear());

    let target = Instant::now() + Duration::from_secs(1);
    for step in 0..3 {
        let _ = shell.handle_event(ShellEvent::FrameTick(FrameInfo {
            target: Some(target + Duration::from_millis(8 * step)),
            interval: Some(Duration::from_millis(8)),
        }));
    }

    let deltas: Vec<_> = FRAMES.with(|frames| frames.borrow().iter().map(|f| f.delta()).collect());
    assert_eq!(
        deltas.last(),
        Some(&Duration::from_millis(8)),
        "spacing came from the targets, not from when the frames were built: {deltas:?}"
    );
}
