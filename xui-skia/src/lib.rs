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
