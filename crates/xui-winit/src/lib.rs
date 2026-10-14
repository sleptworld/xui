//! `winit` window and event-loop integration for `xui`.
//!
//! The winit host for `xui-shell`: owns the event loop, opens the window from
//! `xui_shell::WindowOptions`, translates `winit` events into
//! `xui_shell::ShellEvent`s and implements `xui_shell::PlatformWindow` for
//! [`WinitWindow`]. Everything platform-independent -- input state, window
//! visibility, platform output -- is `xui_shell::Shell`'s.
//!
//! # Features
//!
//! - `vello` (default) — pulls in `xui-vello` and re-exports `VelloBackend`;
//!   enables the `runner` function.
//! - `skia` — pulls in `xui-skia` and re-exports `SkiaBackend` and
//!   `SkiaTextBackend`; enables the `skia_runner` function. Independent of
//!   `vello`.
//! - `wgpu` — enables the optional `wgpu` renderer module (`WGPUBackend`,
//!   `TexturePool`, `TextureLease`, ...) and `wgpu_runner`.
//!
//! See `xui-example-app` for the standard application setup.

mod device;
pub mod error;
mod runner;
pub mod sdf;
mod translate;
mod window;

#[cfg(feature = "wgpu")]
pub(crate) mod renders;
#[cfg(feature = "wgpu")]
mod wgpu;

#[cfg(feature = "vello")]
pub use runner::runner;
#[cfg(feature = "skia")]
pub use runner::skia_runner;
#[cfg(feature = "wgpu")]
pub use runner::wgpu_runner;
pub use runner::{WinitBackendInitError, WinitRunError, WinitRunner, WinitRunnerOptions};
pub use sdf::UI_SHADER_WGSL;
pub use translate::{
    translate_mouse_button, translate_mouse_wheel, translate_named_key, translate_physical_key,
    translate_window_event,
};
pub use window::{WinitWindow, window_attributes};
pub use xui_shell::{MacOsWindowOptions, PhysicalSize, WindowOptions, WindowSize};

#[cfg(feature = "wgpu")]
pub use wgpu::{
    LayerCacheStats, TextureLease, TexturePool, TexturePoolError, TexturePoolOptions,
    TexturePoolStats, TextureRequest, WGPUBackend, WgpuBackendInitError, WgpuBackendOptions,
};

#[cfg(feature = "vello")]
pub use xui_vello::{VelloBackend, VelloBackendOptions};

#[cfg(feature = "skia")]
pub use xui_skia::{
    SkiaBackend, SkiaBackendError, SkiaBackendOptions, SkiaFont, SkiaFontId, SkiaFonts,
    SkiaLayerCacheStats, SkiaParagraphState, SkiaTextBackend,
};
