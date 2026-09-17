use std::cell::{Cell, RefCell};
use std::error::Error;
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply, NSEvent, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem,
    NSResponder,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, ns_string};
use xui_core::App;
use xui_core::app::AppRenderError;
use xui_core::render::RenderBackend;
use xui_core::text::TextHost;
use xui_interface::TextBackend;
use xui_shell::{Shell, ShellEvent, ShellOptions, WindowOptions};

use crate::host::{self, EventSink, Host};
use crate::window::MacWindow;

#[derive(Debug, Clone)]
pub struct MacRunnerOptions {
    /// The window to open. It is created hidden and shown once its first frame
    /// is painted, unless `window.visible` is false.
    pub window: WindowOptions,
    /// Whether a close request -- the close button, Quit, Cmd+Q -- ends the
    /// run, rather than only being reported to the runtime.
    pub exit_on_close_requested: bool,
}

impl Default for MacRunnerOptions {
    fn default() -> Self {
        Self {
            window: WindowOptions::default(),
            exit_on_close_requested: true,
        }
    }
}

pub type MacBackendInitError = Box<dyn Error + Send + Sync>;

pub enum MacRunError<E> {
    /// AppKit can only be driven from the main thread.
    NotMainThread,
    BackendInit(MacBackendInitError),
    Render(E),
}

impl<E: fmt::Debug> fmt::Debug for MacRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMainThread => f.write_str("NotMainThread"),
            Self::BackendInit(error) => f.debug_tuple("BackendInit").field(error).finish(),
            Self::Render(error) => f.debug_tuple("Render").field(error).finish(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for MacRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMainThread => {
                f.write_str("the AppKit runner must be started on the main thread")
            }
            Self::BackendInit(error) => write!(f, "render backend initialization error: {error}"),
            Self::Render(error) => write!(f, "render backend error: {error}"),
        }
    }
}

impl<E> Error for MacRunError<E>
where
    E: fmt::Debug + fmt::Display + Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotMainThread => None,
            Self::BackendInit(error) => Some(error.as_ref()),
            Self::Render(error) => Some(error),
        }
    }
}

type BackendFactory<B, T> =
    Box<dyn FnOnce(Arc<MacWindow>) -> Result<(App, T, B), MacBackendInitError> + 'static>;

/// Hosts an [`App`] in a native AppKit window.
pub struct MacRunner<B: RenderBackend<TextHost<T>>, T: TextBackend> {
    factory: BackendFactory<B, T>,
    options: MacRunnerOptions,
}

impl<B, T> MacRunner<B, T>
where
    B: RenderBackend<TextHost<T>> + 'static,
    T: TextBackend + 'static,
{
    pub fn with_options<F>(factory: F, options: MacRunnerOptions) -> Self
    where
        F: FnOnce(Arc<MacWindow>) -> (App, T, B) + 'static,
    {
        Self::with_fallible_options(
            move |window| Ok::<(App, T, B), std::convert::Infallible>(factory(window)),
            options,
        )
    }

    pub fn with_fallible_options<F, E>(factory: F, options: MacRunnerOptions) -> Self
    where
        F: FnOnce(Arc<MacWindow>) -> Result<(App, T, B), E> + 'static,
        E: Error + Send + Sync + 'static,
    {
        Self {
            factory: Box::new(move |window| {
                factory(window).map_err(|error| Box::new(error) as MacBackendInitError)
            }),
            options,
        }
    }

    /// Runs until the shell exits.
    ///
    /// Must be called on the main thread, once: AppKit's application object
    /// does not start a second time.
    pub fn run(self) -> Result<(), MacRunError<AppRenderError<B::Error>>> {
        let mtm = MainThreadMarker::new().ok_or(MacRunError::NotMainThread)?;
        let app = XuiApplication::shared(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        app.setMainMenu(Some(&main_menu(mtm)));

        let state = Rc::new(Launch::<B, T> {
            host: RefCell::new(None),
            init_error: RefCell::new(None),
        });
        let Self { factory, options } = self;
        let launch = {
            let state = state.clone();
            move || launch(mtm, factory, options, &state)
        };
        // Windows are opened from `applicationDidFinishLaunching:`: before it,
        // an unbundled app cannot be activated.
        let delegate = AppDelegate::new(mtm, Box::new(launch));
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        app.run();
        app.setDelegate(None);
        host::set_current(None);

        if let Some(error) = state.init_error.take() {
            return Err(MacRunError::BackendInit(error));
        }
        if let Some(host) = state.host.take() {
            let error = host.take_render_error();
            host.shut_down();
            if let Some(error) = error {
                return Err(MacRunError::Render(error));
            }
        }
        Ok(())
    }
}

/// What the launch callback leaves behind for `run` to collect.
struct Launch<B, T>
where
    B: RenderBackend<TextHost<T>>,
    T: TextBackend,
{
    host: RefCell<Option<Rc<Host<B, T>>>>,
    init_error: RefCell<Option<MacBackendInitError>>,
}

fn launch<B, T>(
    mtm: MainThreadMarker,
    factory: BackendFactory<B, T>,
    options: MacRunnerOptions,
    state: &Launch<B, T>,
) where
    B: RenderBackend<TextHost<T>> + 'static,
    T: TextBackend + 'static,
{
    let window = MacWindow::new(mtm, &options.window);
    let (app, text, backend) = match factory(window.clone()) {
        Ok(parts) => parts,
        Err(error) => {
            state.init_error.replace(Some(error));
            host::stop_app(mtm);
            return;
        }
    };
    app.set_async_wake_callback(host::wake);

    // Routed before the shell exists: starting it shows the window, and AppKit
    // reports that synchronously. Those events wait in the host's queue.
    let host = Rc::new(Host::<B, T>::new());
    let weak = Rc::downgrade(&host);
    let sink: Weak<dyn EventSink> = weak;
    window.attach(mtm, sink.clone());
    host::set_current(Some(sink));

    let shell_options = ShellOptions {
        visible: options.window.visible,
        exit_on_close_requested: options.exit_on_close_requested,
    };
    let (shell, control) = Shell::new(window, app, text, backend, shell_options);
    host.install(shell, control);
    state.host.replace(Some(host));
}

fn main_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu_bar = NSMenu::new(mtm);
    let app_item = NSMenuItem::new(mtm);
    menu_bar.addItem(&app_item);

    let app_menu = NSMenu::new(mtm);
    // SAFETY: `terminate:` is an NSApplication action taking the sender, as a
    // menu item's action must. It reaches `applicationShouldTerminate:` below.
    let quit = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Quit"),
            Some(sel!(terminate:)),
            ns_string!("q"),
        )
    };
    app_menu.addItem(&quit);
    app_item.setSubmenu(Some(&app_menu));
    menu_bar
}

define_class!(
    // SAFETY: NSApplication is made to be subclassed -- that is how an
    // application customizes event dispatch -- and `XuiApplication` neither
    // implements `Drop` nor declares instance variables.
    #[unsafe(super(NSApplication, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "XuiApplication"]
    struct XuiApplication;

    unsafe impl NSObjectProtocol for XuiApplication {}

    impl XuiApplication {
        /// Delivers the `keyUp:` that AppKit drops while Command is held.
        ///
        /// `-[NSApplication sendEvent:]` does not route a key-up to the key
        /// window when Command is down, so a shortcut's release is never seen
        /// and the runtime would keep the key marked as held.
        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            if event.r#type() == NSEventType::KeyUp
                && event
                    .modifierFlags()
                    .contains(NSEventModifierFlags::Command)
                && let Some(window) = self.keyWindow()
            {
                window.sendEvent(event);
                return;
            }
            unsafe { msg_send![super(self), sendEvent: event] }
        }
    }
);

impl XuiApplication {
    /// The shared application, as an `XuiApplication`.
    ///
    /// `sharedApplication` instantiates the class it is sent to, so this makes
    /// the process's `NSApp` one of ours -- as long as nothing has created it
    /// first. If something has, that instance comes back and the `sendEvent:`
    /// override simply is not in play.
    fn shared(mtm: MainThreadMarker) -> Retained<NSApplication> {
        let _ = mtm;
        // SAFETY: `sharedApplication` returns the shared NSApplication.
        unsafe { msg_send![Self::class(), sharedApplication] }
    }
}

struct AppDelegateIvars {
    launch: Cell<Option<Box<dyn FnOnce()>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `AppDelegate`
    // does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "XuiAppDelegate"]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            if let Some(launch) = self.ivars().launch.take() {
                launch();
            }
        }

        /// Quit, from the menu or the Dock, is a close request like the close
        /// button: the shell decides, and ends the run itself instead of
        /// AppKit exiting the process underneath it.
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
            host::send_current(ShellEvent::CloseRequested);
            NSApplicationTerminateReply::TerminateCancel
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker, launch: Box<dyn FnOnce()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars {
            launch: Cell::new(Some(launch)),
        });
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(feature = "skia")]
pub fn runner(
    app: xui_core::app::ComponentFn,
    options: Option<MacRunnerOptions>,
) -> MacRunner<xui_skia::SkiaBackend<xui_f::FBackend>, xui_f::FBackend> {
    let app = App::new(app);
    MacRunner::with_fallible_options(
        move |window| -> Result<_, std::io::Error> {
            #[allow(unused_mut)]
            let mut app = app;
            let backend = xui_skia::SkiaBackend::<xui_f::FBackend>::new(window)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            // See `xui_winit::runner`: a canvas GPU painter draws on the device
            // Skia composites from, when Skia is on a shared one.
            #[cfg(feature = "skia-wgpu")]
            if let Some(context) = backend.wgpu_context() {
                app.set_gpu_context(xui_core::widgets::CanvasGpuContext::new(
                    context.device().clone(),
                    context.queue().clone(),
                ));
            }
            Ok((app, xui_f::FBackend::new(), backend))
        },
        options.unwrap_or_default(),
    )
}
