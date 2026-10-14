//! A swash glyph rasterizer for any xui text backend.
//!
//! [`SwashRasterizer`] implements [`GlyphRasterizer`] over any
//! [`FontCollection`]: it reads a face's bytes through [`FontData`] and turns
//! a [`GlyphRequest`] into a coverage or colour bitmap in device pixels. It is
//! `Send + Sync` and takes `&self`, so a render thread can rasterize while
//! the backend keeps shaping on another.
//!
//! Bitmaps are cached by request (physical size, subpixel offset, raster
//! options), within a byte budget, and dropped when the collection's epoch
//! moves. Packing them into textures is the renderer's business.

mod cache;

use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Mutex, PoisonError},
};

use swash::{
    FontRef as SwashFontRef,
    scale::{
        Render, ScaleContext, Source, StrikeWith,
        image::{Content, Image},
    },
    zeno::{Format, Vector},
};
use xui_interface::{
    FontCollection, FontData, GlyphImage, GlyphRasterizer, GlyphRequest, RasterizedGlyphFormat,
};

use cache::BitmapCache;

/// Largest physical em size that is rasterized. A glyph this size is already
/// most of an atlas page; larger requests are refused instead of allocating
/// bitmaps of hundreds of megabytes.
const MAX_PPEM: f32 = 1024.0;

/// Colour sources first, so emoji faces that also carry monochrome outlines
/// render in colour.
const SOURCES: &[Source] = &[
    Source::ColorOutline(0),
    Source::ColorBitmap(StrikeWith::BestFit),
    Source::Outline,
];

/// A rendered glyph still in the rasterizer's scratch buffer, laid out as a
/// [`GlyphImage`].
#[derive(Debug, Clone, Copy)]
struct Bitmap<'a> {
    pub format: RasterizedGlyphFormat,
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
}

impl Bitmap<'_> {
    fn to_image(self) -> GlyphImage {
        GlyphImage {
            format: self.format,
            width: self.width,
            height: self.height,
            left: self.left,
            top: self.top,
            pixels: self.pixels.into(),
        }
    }
}

/// Where swash finds a face: its table directory offset and the cache key its
/// scaler caches are indexed by. `FontRef::from_index` mints a fresh key on
/// every call, which would make each glyph re-parse the face, so the key is
/// made once per face and reused.
#[derive(Clone, Copy)]
struct FaceKey {
    offset: u32,
    key: swash::CacheKey,
}

/// Mutable rendering state: swash's scaler caches and the output buffer. One
/// is checked out of the pool per render, so threads never share one.
struct Scratch<F> {
    context: ScaleContext,
    /// Swash keys of the faces seen under `font_epoch`.
    faces: HashMap<F, Option<FaceKey>>,
    font_epoch: u64,
    image: Image,
}

impl<F: Copy + Eq + Hash> Scratch<F> {
    fn new() -> Self {
        Self {
            context: ScaleContext::new(),
            faces: HashMap::new(),
            font_epoch: 0,
            image: Image::new(),
        }
    }

    /// Renders `request`, or returns `None` when the face cannot be read or
    /// the glyph has no ink (whitespace, an empty outline).
    fn render<C: FontCollection<FontId = F>>(
        &mut self,
        fonts: &C,
        font_epoch: u64,
        request: &GlyphRequest<F>,
    ) -> Option<Bitmap<'_>> {
        if self.font_epoch != font_epoch {
            self.faces.clear();
            self.font_epoch = font_epoch;
        }
        let data = fonts.font(request.font)?;
        let bytes = data.bytes();
        let face_key = *self.faces.entry(request.font).or_insert_with(|| {
            SwashFontRef::from_index(bytes, data.index() as usize).map(|font| FaceKey {
                offset: font.offset,
                key: font.key,
            })
        });
        let face_key = face_key?;
        let font = SwashFontRef {
            data: bytes,
            offset: face_key.offset,
            key: face_key.key,
        };
        let glyph_id = u16::try_from(request.glyph).ok()?;
        let ppem = request.ppem();
        if !(ppem > 0.0 && ppem <= MAX_PPEM) {
            return None;
        }

        let mut scaler = self
            .context
            .builder(font)
            .size(ppem)
            .hint(request.options.hinting)
            .build();
        self.image.clear();
        let rendered = Render::new(SOURCES)
            .format(if request.options.subpixel_antialiasing {
                Format::Subpixel
            } else {
                Format::Alpha
            })
            .offset(Vector::new(request.subpixel_x.as_f32(), 0.0))
            .render_into(&mut scaler, glyph_id, &mut self.image);
        let placement = self.image.placement;
        if !rendered || placement.width == 0 || placement.height == 0 {
            return None;
        }
        let format = match self.image.content {
            Content::Mask => RasterizedGlyphFormat::Mask,
            Content::SubpixelMask => RasterizedGlyphFormat::SubpixelMask,
            Content::Color => RasterizedGlyphFormat::Color,
        };
        // Colour outlines are composited premultiplied; embedded colour
        // bitmaps (sbix, CBDT) decode straight from PNG and are not.
        if format == RasterizedGlyphFormat::Color
            && matches!(
                self.image.source,
                Source::ColorBitmap(_) | Source::Bitmap(_)
            )
        {
            premultiply(&mut self.image.data);
        }
        let bitmap = Bitmap {
            format,
            left: placement.left,
            top: placement.top,
            width: placement.width,
            height: placement.height,
            pixels: &self.image.data,
        };
        let expected =
            bitmap.width as usize * bitmap.height as usize * bitmap.format.bytes_per_pixel();
        (bitmap.pixels.len() >= expected).then(|| Bitmap {
            pixels: &bitmap.pixels[..expected],
            ..bitmap
        })
    }
}

/// Renders glyphs of a [`FontCollection`] with swash, from any thread.
///
/// Backends hold one behind an `Arc` and hand it out from
/// `TextBackend::rasterizer`.
pub struct SwashRasterizer<C: FontCollection> {
    fonts: Arc<C>,
    /// Keyed on the font epoch too, so a bitmap rendered for an id before the
    /// collection changed is never served for whatever the id means now.
    bitmaps: BitmapCache<(u64, GlyphRequest<C::FontId>)>,
    /// Idle scratch states; grows to the number of threads rendering at once.
    scratch: Mutex<Vec<Scratch<C::FontId>>>,
}

impl<C: FontCollection> SwashRasterizer<C> {
    pub fn new(fonts: Arc<C>) -> Self {
        Self::with_cache_budget(fonts, cache::DEFAULT_BUDGET_BYTES)
    }

    /// `budget_bytes` bounds the bitmaps kept for `GlyphRasterizer::rasterize`.
    pub fn with_cache_budget(fonts: Arc<C>, budget_bytes: u64) -> Self {
        Self {
            fonts,
            bitmaps: BitmapCache::new(budget_bytes),
            scratch: Mutex::new(Vec::new()),
        }
    }

    pub fn fonts(&self) -> &Arc<C> {
        &self.fonts
    }

    /// Renders `request` without caching and hands the bitmap to `consume`
    /// while it still lives in the scratch buffer.
    fn render<R>(
        &self,
        request: &GlyphRequest<C::FontId>,
        consume: impl FnOnce(Option<Bitmap<'_>>) -> R,
    ) -> R {
        let mut scratch = self
            .scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .unwrap_or_else(Scratch::new);
        let epoch = self.fonts.epoch();
        let result = consume(scratch.render(&*self.fonts, epoch, request));
        self.scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scratch);
        result
    }
}

impl<C: FontCollection> GlyphRasterizer for SwashRasterizer<C> {
    type FontId = C::FontId;

    fn rasterize(&self, request: &GlyphRequest<C::FontId>) -> Option<GlyphImage> {
        let key = (self.fonts.epoch(), *request);
        if let Some(cached) = self.bitmaps.get(&key) {
            return cached;
        }
        let image = self.render(request, |bitmap| bitmap.map(Bitmap::to_image));
        self.bitmaps.insert(key, image.clone());
        image
    }
}

fn premultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplies_straight_alpha() {
        let mut pixels = [255, 128, 0, 128, 10, 20, 30, 255, 200, 200, 200, 0];
        premultiply(&mut pixels);
        assert_eq!(pixels, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
    }
}
