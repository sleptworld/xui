//! Vello rendering backend for `xui`.
//!
//! Implements `xui_core::render::RenderBackend` on `vello` + `wgpu`, as a
//! sibling of `xui-skia`. Window hosts hand it an `Arc<dyn SurfaceTarget>`, as
//! they do for Skia, so any host can use it.
//!
//! # Who does what
//!
//! **Vello draws**: shapes, paths, gradients, shadows, images, and the clips
//! around them, and glyph runs it cannot draw from bitmaps. It composes
//! nothing.
//!
//! **`xui-wgpu-text` draws text** from glyph bitmaps when the text backend has
//! a rasterizer ([`VelloBackendOptions::bitmap_text`]): text whose transform is
//! a translation and a uniform scale, cut by nothing but rectangles, and that
//! fits the glyph atlas ([`VelloBackendOptions::glyph_atlas`]; text that does
//! not, even at the atlas's limit, is outlines for that frame). Its
//! glyphs, caret and decorations wait for the band they were drawn in to close
//! and are drawn over it then, so a band is not cut for every text; a later
//! draw in the band that overlaps one closes the band first. Where the cuts
//! fall changes nothing text looks like (see "Colour space").
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
//! One working space, everywhere: **sRGB-encoded values, premultiplied**. It
//! is the space Vello blends in inside a band, which nothing here can change,
//! and the one the Skia backend and CSS compose in, so every way of drawing
//! the same thing gives the same colour:
//!
//! | Stage | What it does to colour |
//! |---|---|
//! | Colours in the scene | sRGB-encoded, straight alpha, as styles give them |
//! | Vello, inside a band | blends shape over shape on encoded values |
//! | A band joining the composition | premultiplied, then source-over |
//! | Text from bitmaps (`xui-wgpu-text`) | source-over on encoded values |
//! | Layer composites, opacity, blend modes | on encoded values |
//! | Filters: blur, colour matrix, shadow, backdrop | on encoded values |
//! | External texture, 8-bit unorm | taken as encoded |
//! | External texture, sRGB or float format | sampled as linear light, encoded once |
//! | Out to the frame | unpremultiplied, 8 bits |
//!
//! Nothing between a band and the frame converts, so cutting a band, isolating
//! a layer, drawing text before or after some other draw, or composing a frame
//! by the fast path or in full never changes a pixel's colour. Work textures
//! are half-float only for precision across chains of passes (a blur cut into
//! stages, a backdrop under a backdrop); what they hold is the same encoded
//! values. The cost of the choice is the one every encoded-space renderer
//! pays: a blur between two saturated colours, or a half-transparent colour
//! over another, is darker in the middle than mixing light would be (half of
//! red over blue is 128 rather than 188).
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
pub use xui_wgpu_text::AtlasOptions;
pub use context::WgpuContext;
pub use encode::Unsupported;
pub use error::VelloBackendError;
/// The `wgpu` the backend renders with, for canvas GPU painters on its device.
pub use wgpu;
