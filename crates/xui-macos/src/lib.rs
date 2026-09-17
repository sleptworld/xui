//! Native AppKit host for `xui`: the macOS counterpart of `xui-winit`.
//!
//! Implements the platform half of the `xui-shell` contract directly on
//! AppKit -- an `NSApplication` run loop, an `NSWindow` whose content view is
//! an `NSView` subclass, and translation of AppKit events into
//! `xui_shell::ShellEvent`s. `xui_shell::Shell` does everything else, exactly
//! as it does under winit, and render backends see only [`MacWindow`]'s raw
//! handles.
//!
//! What the native path gives that winit does not:
//!
//! - trackpad momentum: wheel events carry `is_inertial` from the event's
//!   momentum phase;
//! - text input through `NSTextInputClient`, with marked text, commits and a
//!   candidate-window anchor;
//! - frames paced by the display: a `CADisplayLink` on the view (macOS 14 and
//!   later), which ticks only while a frame is pending, with the main queue
//!   carrying redraws on older systems;
//! - key releases that AppKit drops while Command is held, recovered by
//!   subclassing `NSApplication` -- so `NSApp` must not have been created
//!   before [`MacRunner::run`];
//! - direct access to the `NSWindow`/`NSView` ([`MacWindow::ns_window`],
//!   [`MacWindow::ns_view`]) for anything AppKit offers beyond
//!   `xui_shell::WindowOptions`.
//!
//! One window per run. The crate is empty on other platforms.
//!
//! # Features
//!
//! - `skia` (default) — [`runner`], on `xui-skia` with the `xui-f` text backend.
//! - `skia-wgpu` — Skia on a wgpu-owned device, so a canvas GPU painter can
//!   composite; see `xui-winit`'s feature of the same name.

#![cfg(target_os = "macos")]

mod app;
mod cursor;
mod host;
mod keycode;
mod view;
mod window;

#[cfg(feature = "skia")]
pub use app::runner;
pub use app::{MacBackendInitError, MacRunError, MacRunner, MacRunnerOptions};
pub use window::MacWindow;
pub use xui_shell::{MacOsWindowOptions, PhysicalSize, WindowOptions, WindowSize};

#[cfg(feature = "skia-wgpu")]
pub use xui_skia::{WgpuContext, WgpuImage, wgpu};
