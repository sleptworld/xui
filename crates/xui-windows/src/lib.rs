//! Native Win32 host for `xui`: the Windows counterpart of `xui-macos`.
//!
//! Implements the platform half of the `xui-shell` contract directly on Win32 --
//! a window class with its own window procedure, a `GetMessage` loop, and
//! translation of `WM_*` messages into `xui_shell::ShellEvent`s.
//! `xui_shell::Shell` does everything else, exactly as it does under winit and
//! AppKit, and render backends see only [`WinWindow`]'s raw handles.
//!
//! What the native path gives that winit does not:
//!
//! - text input through IMM32, with preedit, commits and a candidate-window
//!   anchor placed from the runtime's cursor area;
//! - direct access to the `HWND` ([`WinWindow::hwnd`]) for anything Win32
//!   offers beyond `xui_shell::WindowOptions`.
//!
//! Per-Monitor V2 DPI awareness is requested at startup, so `WM_DPICHANGED`
//! carries a real scale change through to the runtime.
//!
//! One window per run. The crate is empty on other platforms.
//!
//! # Features
//!
//! - `skia` (default) — [`runner`], on `xui-skia` with the `xui-f` text backend.
//! - `skia-wgpu` — Skia on a wgpu-owned device, so a canvas GPU painter can
//!   composite; see `xui-winit`'s feature of the same name.

#![cfg(windows)]

mod app;
mod cursor;
mod host;
mod ime;
mod keycode;
mod window;
mod wndproc;

#[cfg(feature = "skia")]
pub use app::runner;
pub use app::{WinBackendInitError, WinRunError, WinRunner, WinRunnerOptions};
pub use window::WinWindow;
pub use xui_shell::{MacOsWindowOptions, PhysicalSize, WindowOptions, WindowSize};
