//! Platform host layer for `xui`: the contract between the runtime and
//! whatever owns the window and the event loop.
//!
//! A host -- `xui-winit`, or a native AppKit or Win32 host -- is split in two:
//!
//! - The platform half, which the host writes: open a window, run the event
//!   loop, translate native events into [`ShellEvent`]s, and implement
//!   [`PlatformWindow`] for its window type.
//! - The platform-independent half, which is [`Shell`]: modifier and button
//!   state, window visibility, revealing the window after its first frame,
//!   and carrying everything else between the runtime and the host.
//!
//! # The channels
//!
//! - **Events in.** [`Shell::dispatch_event`] takes a [`ShellEvent`] and the
//!   instant it happened, and answers with a [`ShellOutcome`]: whether to keep
//!   running, and whether the runtime consumed the event, which is what lets a
//!   host decide if the native layer still gets it.
//! - **State out.** The runtime's `PlatformOutput` -- cursor, IME session, window
//!   title, menu bar -- is diffed and applied through [`PlatformWindow`]. The
//!   parts a native callback may need on the spot (window regions, drop targets)
//!   are also readable at any time through [`PublishedHandle`], without
//!   borrowing the shell.
//! - **Commands and requests out.** One-shot actions the application asked for --
//!   a clipboard write, a dialog, a window command -- reach the host through
//!   [`PlatformWindow::perform`], which says what it did with each
//!   ([`Performed`]). A request's answer comes back as [`ShellEvent::Reply`], or
//!   inline.
//!
//! A host whose native callbacks can arrive while it is inside the shell -- all
//! of them can -- should hold a [`ShellHandle`], which queues those events
//! instead of requiring the host to.
//!
//! Render backends only need a [`SurfaceTarget`] -- raw window/display handles
//! plus a size and scale factor -- so they never see the host's window type.
//!
//! [`WindowOptions`] describes the window an application asks for, in terms
//! every host can map onto its own window creation.
//!
//! The design, and what is and is not built yet, is in `docs/shell-api.md`.

mod handle;
mod published;
mod shell;
mod window;

pub use raw_window_handle;
pub use handle::ShellHandle;
pub use published::PublishedHandle;
pub use shell::{
    FrameInfo, KeyInput, Shell, ShellControl, ShellEvent, ShellOptions, ShellOutcome, TouchPhase,
};
pub use window::{
    MacOsWindowOptions, Performed, PhysicalSize, PlatformWindow, SurfaceTarget, WindowOptions,
    WindowSize,
};
