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
//! With the optional `wgpu` feature and `XUI_SKIA_WGPU=1`, Skia instead runs on
//! a wgpu-owned device and swapchain, which is what lets a caller's own
//! `wgpu::Texture` be composited by Skia. See [`wgpu_surface`].
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
#[cfg(feature = "wgpu")]
mod wgpu_surface;

pub use backend::{SkiaBackend, SkiaBackendOptions, SkiaOptimizations};
pub use cache::SkiaLayerCacheStats;
pub use error::SkiaBackendError;
pub use stats::SkiaFrameStats;
pub use text::{SkiaFontId, SkiaGlyphKey, SkiaParagraphState, SkiaTextBackend};
#[cfg(feature = "wgpu")]
pub use wgpu_surface::{IMPORTABLE_TEXTURE_USAGES, WgpuContext, WgpuImage};
