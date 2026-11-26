//! Windows beyond the main one, through the shell's public API: an opener
//! that records what it was asked to make, and fake native windows that
//! record what they were asked to do.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use xui_core::prelude::*;
use xui_core::render::MockRenderBackend;
use xui_f::FBackend;
use xui_interface::{PlatformError, PointerButton, WindowId, WindowOptions};
use xui_macros::component;
use xui_shell::{
    PhysicalSize, PlatformWindow, PublishedHandle, Shell, ShellEvent, ShellOptions, SurfaceTarget,
    WindowOpener,
};

#[derive(Debug, Clone, PartialEq)]
enum Call {
    RequestRedraw,
    SetVisible(bool),
    SetTitle(String),
}

#[derive(Default)]
struct FakeWindow(Mutex<Vec<Call>>);

impl FakeWindow {
    fn record(&self, call: Call) {
        self.0.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<Call> {
        self.0.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.0.lock().unwrap().clear();
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
    fn request_redraw(&self) {
        self.record(Call::RequestRedraw);
    }
    fn set_visible(&self, visible: bool) {
        self.record(Call::SetVisible(visible));
    }
    fn focus(&self) {}
    fn is_minimized(&self) -> Option<bool> {
        Some(false)
    }
    fn set_cursor(&self, _cursor: CursorIcon) {}
    fn set_ime_allowed(&self, _allowed: bool) {}
    fn set_ime_cursor_area(&self, _area: Rect) {}
    fn set_title(&self, title: &str) {
        self.record(Call::SetTitle(title.to_owned()));
    }
}

/// What the opener was asked to do, shared with the test.
#[derive(Default)]
struct Opened {
    windows: Vec<(WindowId, String, Arc<FakeWindow>)>,
    closed: Vec<WindowId>,
}

struct FakeOpener(Rc<RefCell<Opened>>);

impl WindowOpener<FakeWindow> for FakeOpener {
    fn open(
        &mut self,
        window: WindowId,
        options: &WindowOptions,
        _published: &PublishedHandle,
    ) -> Result<Arc<FakeWindow>, PlatformError> {
        let native = Arc::new(FakeWindow::default());
        self.0
            .borrow_mut()
            .windows
            .push((window, options.title.clone(), Arc::clone(&native)));
        Ok(native)
    }

    fn close(&mut self, window: WindowId, _native: &Arc<FakeWindow>) {
        self.0.borrow_mut().closed.push(window);
    }
}

type TestShell = Shell<FakeWindow, MockRenderBackend, FBackend>;

#[component]
fn settings_button() {
    let open: bool = state!(false);
    let clicks: i32 = state!(0);
    view! {
        column {
            container(width: 100.0, height: 100.0, on_click: |cx| open.set(cx, true))
            text(format!("clicks {clicks}"))
            if *open {
                window(title: "Settings", on_close_requested: |cx| open.set(cx, false)) {
                    container(
                        width: 100.0,
                        height: 100.0,
                        on_click: |cx| clicks.update(cx, |c| *c += 1),
                    )
                }
            }
        }
    }
}

fn start() -> (TestShell, Arc<FakeWindow>) {
    let window = Arc::new(FakeWindow::default());
    let app = App::new(|app| {
        app.mount::<SettingsButton>(SettingsButtonProps {});
    });
    let (shell, _) = Shell::new(
        Arc::clone(&window),
        app,
        FBackend::new(),
        MockRenderBackend::default(),
        ShellOptions::default(),
    );
    (shell, window)
}

/// A click at `(50, 50)`, logical, in `window`.
fn click(shell: &mut TestShell, window: WindowId) {
    let at = Instant::now();
    let position = Point::new(50.0, 50.0);
    for event in [
        ShellEvent::PointerMoved {
            position,
            device_id: None,
        },
        ShellEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            device_id: None,
        },
        ShellEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
            device_id: None,
        },
    ] {
        let _ = shell.dispatch_window_event(window, at, event);
    }
    let _ = shell.dispatch_window_event(window, at, ShellEvent::RedrawRequested);
}

/// The windows the app has open beside the main one.
fn others(shell: &TestShell) -> Vec<WindowId> {
    let main = shell.main_window();
    shell
        .runtime()
        .app()
        .window_ids()
        .into_iter()
        .filter(|&window| window != main)
        .collect()
}

#[test]
fn without_an_opener_a_window_block_has_no_window_on_screen() {
    let (mut shell, _) = start();
    let main = shell.main_window();
    click(&mut shell, main);

    let [settings] = others(&shell)[..] else {
        panic!("the block opened one window");
    };
    assert!(shell.native_window(settings).is_none());
    assert!(!shell.runtime().is_attached(settings));
    assert!(
        !shell.runtime().app().window_visible(settings),
        "a window no host made is not on screen"
    );
}

#[test]
fn an_opener_makes_the_window_paints_it_and_closes_it_when_asked() {
    let opened = Rc::new(RefCell::new(Opened::default()));
    let (shell, main_native) = start();
    let mut shell = shell.with_opener(FakeOpener(Rc::clone(&opened)));
    let main = shell.main_window();
    click(&mut shell, main);

    // Made hidden, painted, then shown, and on screen from then on.
    let (settings, title, native) = {
        let opened = opened.borrow();
        let [(window, title, native)] = &opened.windows[..] else {
            panic!("expected one window made, got {}", opened.windows.len());
        };
        (*window, title.clone(), Arc::clone(native))
    };
    assert_eq!(title, "Settings");
    assert_eq!(others(&shell), [settings]);
    assert!(Arc::ptr_eq(shell.native_window(settings).unwrap(), &native));
    // Drawn by the backend that draws the main window.
    assert_eq!(shell.runtime().backend().frames_of(settings), 1);
    assert!(
        shell
            .runtime()
            .app()
            .platform()
            .capabilities()
            .contains(xui_interface::HostCapabilities::MULTI_WINDOW)
    );
    assert!(native.calls().contains(&Call::SetVisible(true)));
    assert!(shell.runtime().app().window_visible(settings));

    // Its input is its own: the click lands on its button, which changes
    // the text in the main window, so the main window is asked to redraw.
    main_native.clear();
    click(&mut shell, settings);
    assert!(main_native.calls().contains(&Call::RequestRedraw));
    assert_eq!(
        others(&shell),
        [settings],
        "the main window's button was not hit"
    );

    // Its title goes to its own native window.
    shell
        .runtime()
        .app()
        .window_platform(settings)
        .set_title("Preferences");
    let _ = shell.dispatch_window_event(settings, Instant::now(), ShellEvent::RedrawRequested);
    assert!(
        native
            .calls()
            .contains(&Call::SetTitle("Preferences".into()))
    );
    assert!(
        !main_native
            .calls()
            .contains(&Call::SetTitle("Preferences".into()))
    );

    // Its close button asks the app, whose handler writes what keeps the
    // block mounted; the next frame unmounts it, which closes the window.
    main_native.clear();
    let _ = shell.dispatch_window_event(settings, Instant::now(), ShellEvent::CloseRequested);
    assert!(main_native.calls().contains(&Call::RequestRedraw));
    assert!(opened.borrow().closed.is_empty());
    let _ = shell.dispatch_event(Instant::now(), ShellEvent::RedrawRequested);
    assert_eq!(opened.borrow().closed, [settings]);
    assert!(shell.native_window(settings).is_none());
    assert!(!shell.runtime().is_attached(settings));
    assert_eq!(shell.runtime().backend().removed, [settings]);
    assert!(others(&shell).is_empty());
}

#[component]
fn inspector() {
    view! {
        window(title: "Inspector") { container() }
    }
}

#[test]
fn a_window_asked_for_before_the_opener_was_given_is_opened_with_it() {
    let window = Arc::new(FakeWindow::default());
    let app = App::new(|app| {
        app.mount::<Inspector>(InspectorProps {});
    });
    let (shell, _) = Shell::new(
        window,
        app,
        FBackend::new(),
        MockRenderBackend::default(),
        ShellOptions::default(),
    );
    let opened = Rc::new(RefCell::new(Opened::default()));
    let shell = shell.with_opener(FakeOpener(Rc::clone(&opened)));

    let opened = opened.borrow();
    let [(inspector, title, _)] = &opened.windows[..] else {
        panic!("expected the inspector made, got {}", opened.windows.len());
    };
    assert_eq!(title, "Inspector");
    assert!(shell.runtime().app().window_visible(*inspector));
}
