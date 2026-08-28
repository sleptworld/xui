//! Skia rendering backend for `xui`.
//!
//! Implements `xui::render::RenderBackend` (and the text/glyph rasterization
//! path) on top of `skia-safe`. Used by default through `xui-winit`'s `skia`
//! feature; on macOS it drives a Metal surface via `objc2-metal`.
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
mod damage;
mod error;
#[cfg(target_os = "macos")]
mod metal;
mod stats;
mod text;

pub use backend::{SkiaBackend, SkiaBackendOptions};
pub use cache::SkiaLayerCacheStats;
pub use error::SkiaBackendError;
pub use stats::SkiaFrameStats;
pub use text::{SkiaFontId, SkiaGlyphKey, SkiaParagraphState, SkiaTextBackend};
