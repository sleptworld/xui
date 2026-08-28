//! `winit` window and event-loop integration for `xui`.
//!
//! Glues an `xui::app::App` to a `winit` window and a render backend. Translates
//! raw `winit` events into `xui_interface::events::RawEvent`s and drives the
//! `GuiRuntime`.
//!
//! # Features
//!
//! - `skia` (default) — pulls in `xui-skia` and re-exports `SkiaBackend` and
//!   `SkiaTextBackend`; enables the `runner` function.
//! - `wgpu` — enables the optional `wgpu` renderer module (`WGPUBackend`,
//!   `TexturePool`, `TextureLease`, ...).
//!
//! See `xui-example-app` for the standard application setup.

mod device;
pub mod error;
mod runner;
pub mod sdf;
mod translate;

#[cfg(feature = "wgpu")]
pub(crate) mod renders;
#[cfg(feature = "wgpu")]
mod wgpu;

#[cfg(feature = "skia")]
pub use runner::runner;
pub use runner::{WinitBackendInitError, WinitRunError, WinitRunner, WinitRunnerOptions};
pub use sdf::UI_SHADER_WGSL;
pub use xui_text_engine::FontSet;
pub use translate::{
    translate_mouse_button, translate_mouse_wheel, translate_named_key, translate_physical_key,
    translate_window_event,
};

#[cfg(feature = "wgpu")]
pub use wgpu::{
    LayerCacheStats, TextureLease, TexturePool, TexturePoolError, TexturePoolOptions,
    TexturePoolStats, TextureRequest, WGPUBackend, WgpuBackendInitError, WgpuBackendOptions,
};

#[cfg(feature = "skia")]
pub use xui_skia::{
    SkiaBackend, SkiaBackendError, SkiaBackendOptions, SkiaFontId, SkiaGlyphKey,
    SkiaLayerCacheStats, SkiaParagraphState, SkiaTextBackend,
};
