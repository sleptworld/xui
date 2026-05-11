use crate::typ::TextRunStyle;
use std::collections::HashMap;
use swash::{
    CacheKey, FontRef, GlyphId,
    scale::{Render, ScaleContext, Scaler, Source, StrikeWith, image::Image as GlyphImage},
    zeno::{Format, Placement, Vector},
};
pub trait FontRenderBackend {
    type Error;
    type Allocation;

    fn write_bitmap(&mut self, bitmap: &RendedGlyphBitmap)
    -> Result<Self::Allocation, Self::Error>;

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

const SOURCES: &'static [Source] = &[
    Source::ColorBitmap(StrikeWith::BestFit),
    Source::ColorOutline(0),
    Source::Outline,
];

pub struct GlyphAtlas<R: FontRenderBackend> {
    glyph_map: HashMap<GlyphKey, Option<(R::Allocation, Placement)>>,
    atlas: TextureAtlas<R>,
    scx: ScaleContext,
    scaled_image: GlyphImage,
}

impl<R: FontRenderBackend> GlyphAtlas<R> {
    pub fn new(backend: R) -> Self {
        Self {
            glyph_map: HashMap::new(),
            atlas: TextureAtlas::new(backend),
            scaled_image: GlyphImage::new(),
            scx: ScaleContext::new(),
        }
    }

    pub fn session<'a>(&'a mut self, text_run_style: &'a TextRunStyle) -> GlyphAtlasSession<'a, R> {
        let font = text_run_style.font;
        let quant_size = (text_run_style.font_size * 32.) as u16;
        let hint = cfg!(not(target_os = "macos"));

        let scaler = self
            .scx
            .builder(font)
            .hint(hint)
            .size(text_run_style.font_size)
            .normalized_coords(text_run_style.font_coords)
            .build();

        GlyphAtlasSession {
            atlas: &mut self.atlas,
            glyph_map: &mut self.glyph_map,
            font,
            quant_size,
            scaler,
            scaled_image: &mut self.scaled_image,
        }
    }
}

#[derive(Hash, Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SubpixelOffset {
    Zero = 0,
    Quarter = 1,
    Half = 2,
    ThreeQuarters = 3,
}

impl SubpixelOffset {
    // Skia quantizes subpixel offsets into 1/4 increments.
    // Given the absolute position, return the quantized increment
    fn quantize(pos: f32) -> Self {
        // Following the conventions of Gecko and Skia, we want
        // to quantize the subpixel position, such that abs(pos) gives:
        // [0.0, 0.125) -> Zero
        // [0.125, 0.375) -> Quarter
        // [0.375, 0.625) -> Half
        // [0.625, 0.875) -> ThreeQuarters,
        // [0.875, 1.0) -> Zero
        // The unit tests below check for this.
        let apos = ((pos - pos.floor()) * 8.0) as i32;
        match apos {
            1..=2 => SubpixelOffset::Quarter,
            3..=4 => SubpixelOffset::Half,
            5..=6 => SubpixelOffset::ThreeQuarters,
            _ => SubpixelOffset::Zero,
        }
    }

    fn to_f32(self) -> f32 {
        match self {
            SubpixelOffset::Zero => 0.0,
            SubpixelOffset::Quarter => 0.25,
            SubpixelOffset::Half => 0.5,
            SubpixelOffset::ThreeQuarters => 0.75,
        }
    }
}

#[derive(Hash, Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GlyphKey {
    pub font_id: CacheKey,
    pub id: u16,
    pub subpx: [SubpixelOffset; 2],
    pub size: u16,
}

pub struct RendedGlyphBitmap {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub placement: Placement,
}

pub struct TextureAtlas<R: FontRenderBackend> {
    backend: R,
}

impl<R: FontRenderBackend> TextureAtlas<R> {
    fn new(backend: R) -> Self {
        Self { backend }
    }
}

pub struct GlyphAtlasSession<'a, R: FontRenderBackend> {
    atlas: &'a mut TextureAtlas<R>,
    glyph_map: &'a mut HashMap<GlyphKey, Option<(R::Allocation, Placement)>>,
    font: FontRef<'a>,
    quant_size: u16,
    scaled_image: &'a mut GlyphImage,
    scaler: Scaler<'a>,
}

impl<'a, R: FontRenderBackend> GlyphAtlasSession<'a, R> {
    pub fn get(&mut self, id: GlyphId, x: f32, y: f32) -> &Option<(R::Allocation, Placement)> {
        let subpx = [SubpixelOffset::quantize(x), SubpixelOffset::quantize(y)];

        let key = GlyphKey {
            font_id: self.font.key,
            id,
            subpx,
            size: self.quant_size,
        };

        self.glyph_map.entry(key).or_insert_with(|| {
            let embolden = {
                #[cfg(target_os = "macos")]
                {
                    0.25
                }
                #[cfg(not(target_os = "macos"))]
                {
                    0.0
                }
            };
            self.scaled_image.data.clear();

            if Render::new(SOURCES)
                .format(Format::Subpixel)
                .offset(Vector::new(subpx[0].to_f32(), subpx[1].to_f32()))
                .embolden(embolden)
                .render_into(&mut self.scaler, id, self.scaled_image)
            {
                let p = self.scaled_image.placement;
                let w = p.width as u16;
                let h = p.height as u16;

                let bitmap = RendedGlyphBitmap {
                    data: self.scaled_image.data.drain(..).collect(),
                    width: w as u32,
                    height: h as u32,
                    placement: p,
                };

                let alloc = self.atlas.backend.write_bitmap(&bitmap).ok()?;

                Some((alloc, p))
            } else {
                None
            }
        })
    }
}
