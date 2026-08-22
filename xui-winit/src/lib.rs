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
