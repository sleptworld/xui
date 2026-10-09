//! Vello rendering backend for `xui`.
//!
//! Implements `xui_core::render::RenderBackend` on `vello` + `wgpu`, as a
//! sibling of `xui-skia`. Window hosts hand it an `Arc<dyn SurfaceTarget>`, as
//! they do for Skia, so any host can use it.
//!
//! # Who does what
//!
//! **Vello draws**: shapes, paths, gradients, shadows, glyph runs, images, and
//! the clips around them. It composes nothing.
//!
//! **This crate composes**, with its own `wgpu` passes
//! ([`filter.wgsl`](../src/filter.wgsl)), following the frame's layer tree:
//!
//! - A target is a list of 2D *bands* (one Vello render each, on transparent),
//!   textures and child layers, composed in paint order.
//! - A child layer that composes as a group would (an opacity, a blend mode,
//!   effects, a backdrop) is *isolated*: its content is a target of its own,
//!   drawn only where its plan reads it, and its `xui-render-graph` plan runs
//!   every pass after that, the composite into the parent included. The parent
//!   as composed so far is the plan's destination and, at any depth, what its
//!   backdrop sees: over what the parent inherits from beneath it, when the
//!   parent lets that through (`BackdropIsolation::Passthrough`, the default).
//!   That is what the parent's own backdrop saw, with the parent's backdrop
//!   applied, taken into the parent's space, as far as the parent reaches; a
//!   layer that inherits it is drawn whole, and not cached.
//! - Which layers are isolated is `xui-core`'s decision, made once: it already
//!   draws in place every layer that would compose as its content does.
//! - *External textures* (a canvas GPU painter's target, or one registered
//!   with [`VelloBackend::set_wgpu_texture`]) are sampled where they are.
//!
//! A clip is Vello's inside a band. Where a texture or an isolated layer cuts
//! one, the band closes the clip and the next band opens it again, and the
//! cut is clipped by the compositor, per pixel: rectangles, and up to four
//! rounded or rotated clips, in its shader; a path, and any clip past those
//! four, through a coverage mask that Vello fills, so the edge is Vello's own.
//!
//! # Colour space
//!
//! The render graph's working space is linear light, and everything this crate
//! computes is: filters, backdrops, and the composition of bands and textures
//! run on premultiplied, half-float, linear textures. Vello's output (sRGB
//! encoded, straight alpha) is decoded on the way in and encoded on the way out.
//! What Vello itself blends inside a band, shape over shape, stays in the
//! encoded space it works in, so a half-transparent *shape* and a
//! half-transparent *layer* or *texture* over the same white do not come out
//! the same (128 against 188). An sRGB or float external texture is already
//! linear when sampled; any other format is taken to be sRGB-encoded.
//!
//! # Between frames
//!
//! The frame lives in a texture, and a tracker shared with `xui-skia`
//! (`xui_core::render::damage`) says what changed, so only that region is drawn
//! again and copied in. Images, fonts, paths, masks and filter textures are
//! cached within byte budgets ([`VelloBackendOptions`], [`ResourceStats`]).
//!
//! A layer that asks to be cached (`CachePolicy::Auto` or `Always`, as a
//! scroll page does) keeps its composed content in a texture of its whole
//! content bounds. A later frame draws into it only what the damage tracker
//! says changed inside the layer, and nothing at all when nothing did, so
//! scrolling such a layer is a composite. A layer with effects keeps their
//! result as well: the layer branch of its plan, run apart over the content
//! bounds grown by the effects, recomputed only where the content changed, or
//! whole when the effects did; the layer is then composited from it by its
//! program with that branch cut out (`LayerProgram::without_layer_effects`).
//! The backdrop and the composite still run every frame the layer is drawn,
//! since they read what is beneath it. A cached layer that changes while it
//! is not drawn is dropped, and drawn afresh the next time.
//!
//! The device is owned here and exposed through [`VelloBackend::wgpu_context`],
//! so a canvas `GpuPainter` renders on the same device it is composed on.
//!
//! # Not done
//!
//! Dashed and dotted shape strokes; masks whose image is only a key into an
//! asset cache; and a layer that does not ask to be cached is drawn and
//! filtered again every frame it is in. A
//! backdrop below the root's direct children redraws the whole frame when
//! anything changes, since the damage region cannot be grown to what it reads.

mod backend;
mod context;
mod encode;
mod error;
mod filter;
mod image;
mod layer_cache;
mod lru;
mod paths;
mod text;
mod texture;

pub use backend::{FrameStats, ResourceStats, VelloBackend, VelloBackendOptions};
pub use context::WgpuContext;
pub use encode::Unsupported;
pub use error::VelloBackendError;
