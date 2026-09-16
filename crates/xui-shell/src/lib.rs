//! Platform host layer for `xui`: the contract between the runtime and
//! whatever owns the window and the event loop.
//!
//! A host -- `xui-winit` today, a native AppKit or Win32 host later -- is split
//! in two:
//!
//! - The platform half, which the host writes: open a window, run the event
//!   loop, translate native events into [`ShellEvent`]s, and implement
//!   [`PlatformWindow`] for its window type.
//! - The platform-independent half, which is [`Shell`]: modifier and button
//!   state, window visibility, revealing the window after its first frame,
//!   and pushing the runtime's `PlatformOutput` (cursor, IME) back out.
//!
//! Render backends only need a [`SurfaceTarget`] -- raw window/display handles
//! plus a size and scale factor -- so they never see the host's window type.
//!
//! [`WindowOptions`] describes the window an application asks for, in terms
//! every host can map onto its own window creation.

mod shell;
mod window;

pub use raw_window_handle;
pub use shell::{KeyInput, Shell, ShellControl, ShellEvent, ShellOptions};
pub use window::{
    MacOsWindowOptions, PhysicalSize, PlatformWindow, SurfaceTarget, WindowOptions, WindowSize,
};
