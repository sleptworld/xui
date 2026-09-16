use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};
use xui_core::App;
use xui_core::app::AppRenderError;
use xui_core::render::RenderBackend;
use xui_core::text::TextHost;
use xui_interface::TextBackend;
use xui_shell::{Shell, ShellOptions, WindowOptions};

use crate::host::{self, EventSink, Host};
use crate::window::{self, WinWindow};

#[derive(Debug, Clone)]
pub struct WinRunnerOptions {
    /// The window to open. It is created hidden and shown once its first frame
    /// is painted, unless `window.visible` is false.
    ///
    /// `WindowOptions::transparent` and the `macos` group are ignored here.
    pub window: WindowOptions,
    /// Whether a close request -- the close button, Alt+F4 -- ends the run,
    /// rather than only being reported to the runtime.
    pub exit_on_close_requested: bool,
}

impl Default for WinRunnerOptions {
    fn default() -> Self {
        Self {
            window: WindowOptions::default(),
            exit_on_close_requested: true,
        }
    }
}

pub type WinBackendInitError = Box<dyn Error + Send + Sync>;

pub enum WinRunError<E> {
    Window(windows::core::Error),
    BackendInit(WinBackendInitError),
    Render(E),
}

impl<E: fmt::Debug> fmt::Debug for WinRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(error) => f.debug_tuple("Window").field(error).finish(),
            Self::BackendInit(error) => f.debug_tuple("BackendInit").field(error).finish(),
            Self::Render(error) => f.debug_tuple("Render").field(error).finish(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for WinRunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(error) => write!(f, "window creation error: {error}"),
            Self::BackendInit(error) => write!(f, "render backend initialization error: {error}"),
            Self::Render(error) => write!(f, "render backend error: {error}"),
        }
    }
}

impl<E> Error for WinRunError<E>
where
    E: fmt::Debug + fmt::Display + Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Window(error) => Some(error),
            Self::BackendInit(error) => Some(error.as_ref()),
            Self::Render(error) => Some(error),
        }
    }
}

type BackendFactory<B, T> =
    Box<dyn FnOnce(Arc<WinWindow>) -> Result<(App, T, B), WinBackendInitError> + 'static>;

/// Hosts an [`App`] in a native Win32 window.
pub struct WinRunner<B: RenderBackend<TextHost<T>>, T: TextBackend> {
    factory: BackendFactory<B, T>,
    options: WinRunnerOptions,
}

impl<B, T> WinRunner<B, T>
where
    B: RenderBackend<TextHost<T>> + 'static,
    T: TextBackend + 'static,
{
    pub fn with_options<F>(factory: F, options: WinRunnerOptions) -> Self
    where
        F: FnOnce(Arc<WinWindow>) -> (App, T, B) + 'static,
    {
        Self::with_fallible_options(
            move |window| Ok::<(App, T, B), std::convert::Infallible>(factory(window)),
            options,
        )
    }

    pub fn with_fallible_options<F, E>(factory: F, options: WinRunnerOptions) -> Self
    where
        F: FnOnce(Arc<WinWindow>) -> Result<(App, T, B), E> + 'static,
        E: Error + Send + Sync + 'static,
    {
        Self {
            factory: Box::new(move |window| {
                factory(window).map_err(|error| Box::new(error) as WinBackendInitError)
            }),
            options,
        }
    }

    /// Runs until the shell exits.
    ///
    /// Must be called on the thread that should own the window: Win32 delivers
    /// a window's messages only to the thread that created it.
    pub fn run(self) -> Result<(), WinRunError<AppRenderError<B::Error>>> {
        // Before any window exists, so the process is told its windows scale
        // themselves. Failure means a manifest already said so, which is the
        // same answer.
        // SAFETY: no preconditions.
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        let Self { factory, options } = self;
        let window = Arc::new(WinWindow::new(&options.window).map_err(WinRunError::Window)?);
        let (app, text, backend) = factory(window.clone()).map_err(WinRunError::BackendInit)?;
        app.set_async_wake_callback(host::wake);

        // Routed before the shell exists: starting it shows the window, and
        // Windows reports that synchronously. Those events wait in the queue.
        let host = Rc::new(Host::<B, T>::new());
        let weak = Rc::downgrade(&host);
        let sink: Weak<dyn EventSink> = weak;
        host::set_current(Some(sink));

        let shell_options = ShellOptions {
            visible: options.window.visible,
            exit_on_close_requested: options.exit_on_close_requested,
        };
        let (shell, control) = Shell::new(window.clone(), app, text, backend, shell_options);
        host.install(shell, control);
        if options.window.maximized && options.window.visible {
            window::maximize(window.handle());
        }

        message_loop();

        host::set_current(None);
        let error = host.take_render_error();
        host.shut_down();
        drop(window);
        match error {
            Some(error) => Err(WinRunError::Render(error)),
            None => Ok(()),
        }
    }
}

fn message_loop() {
    let mut msg = MSG::default();
    loop {
        // SAFETY: `msg` is a valid out-parameter. `GetMessage` returns 0 for
        // WM_QUIT and -1 for an error, and blocks until a message arrives --
        // which is what makes an idle window cost nothing.
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 <= 0 {
            return;
        }
        // SAFETY: `msg` was filled by `GetMessage`. `TranslateMessage` is what
        // posts the `WM_CHAR` the key handler reads.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(feature = "skia")]
pub fn runner(
    app: xui_core::app::ComponentFn,
    options: Option<WinRunnerOptions>,
) -> WinRunner<xui_skia::SkiaBackend<xui_f::FBackend>, xui_f::FBackend> {
    let app = App::new(app);
    WinRunner::with_fallible_options(
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
