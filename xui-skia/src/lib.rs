//! Skia rendering backend for `xui`.
//!
//! Implements `xui::render::RenderBackend` (and the text/glyph rasterization
//! path) on top of `skia-safe`. Used by default through `xui-winit`'s `skia`
//! feature; on macOS it drives a Metal surface via `objc2-metal`.
//!
//! Presentation is selected at runtime by [`present::WindowPresenter`]: Metal on
//! macOS, Direct3D 12 on Windows, Vulkan on Linux, and a `softbuffer` CPU blit
//! everywhere as the fallback when no GPU context can be created.
//!
//! - `SkiaBackend` — the render backend; generic over a `TextBackend`
//!   (defaults to `SkiaTextBackend`).
//! - `SkiaBackendOptions` — clear color and layer-cache budget.
//! - `SkiaTextBackend` — `xui_interface::TextBackend` backed by Skia paragraphs.
//! - `SkiaFontId`, `SkiaGlyphKey`, `SkiaParagraphState` — glyph/paragraph keys.
//! - `SkiaFrameStats`, `SkiaLayerCacheStats` — per-frame and cache statistics.
//!
//! Applications construct a `SkiaBackend` indirectly through `xui-winit`.

mod backend;
mod cache;
#[cfg(target_os = "windows")]
mod d3d;
mod damage;
mod error;
#[cfg(target_os = "macos")]
mod metal;
mod present;
mod stats;
mod text;
#[cfg(target_os = "linux")]
mod vulkan;

pub use backend::{SkiaBackend, SkiaBackendOptions, SkiaOptimizations};
pub use cache::SkiaLayerCacheStats;
pub use error::SkiaBackendError;
pub use stats::SkiaFrameStats;
pub use text::{SkiaFontId, SkiaGlyphKey, SkiaParagraphState, SkiaTextBackend};
