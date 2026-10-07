//! Vello rendering backend for `xui`.
//!
//! Implements `xui_core::render::RenderBackend` on `vello` + `wgpu`, as a
//! sibling of `xui-skia`. Vello rasterizes the frame's shapes, paths and (once
//! wired) glyph runs and images into one `Rgba8Unorm` texture, which is blitted
//! onto the swapchain image.
//!
//! The device is owned here and exposed through
//! [`VelloBackend::wgpu_context`], so a canvas `GpuPainter` can render on the
//! same device and have its texture composited, exactly as with `xui-skia`'s
//! `wgpu` feature.
//!
//! # Status
//!
//! Skeleton. Drawn today: solid-fill/stroked shapes, paths (fill, stroke,
//! dash, cap, join), rectangular clips. Everything else a frame can contain is
//! reported once through `log::warn!` and skipped; see [`Unsupported`] and
//! `docs/vello-backend.md` for the gap list.
//!
//! Vello has no filter model, so layer effects (`PassOp` blur, backdrop, masks)
//! are not a Vello feature. They belong to a separate WGSL pass stage that
//! consumes `xui-render-graph`'s `LayerRenderPlan`, which is why that crate is
//! already a dependency.

mod backend;
mod context;
mod encode;
mod error;

pub use backend::{VelloBackend, VelloBackendOptions};
pub use context::WgpuContext;
pub use encode::Unsupported;
pub use error::VelloBackendError;
