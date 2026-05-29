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

pub struct GlyphAtlas<A> {
    glyph_map: HashMap<GlyphKey, Option<(A, Placement)>>,
    scx: ScaleContext,
    scaled_image: GlyphImage,
}

impl<A> GlyphAtlas<A> {
    pub fn new() -> Self {
        Self {
            glyph_map: HashMap::new(),
            scaled_image: GlyphImage::new(),
            scx: ScaleContext::new(),
        }
    }

    pub fn session<'a, R>(
        &'a mut self,
        text_run_style: &'a TextRunStyle,
        writer: &'a mut R,
    ) -> GlyphAtlasSession<'a, R, A>
    where
        R: FontRenderBackend<Allocation = A>,
    {
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
            glyph_map: &mut self.glyph_map,
            writer,
            font,
            quant_size,
            scaler,
            scaled_image: &mut self.scaled_image,
        }
    }
}

impl<A> Default for GlyphAtlas<A> {
    fn default() -> Self {
        Self::new()
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

pub struct GlyphAtlasSession<'a, R: FontRenderBackend<Allocation = A>, A> {
    glyph_map: &'a mut HashMap<GlyphKey, Option<(A, Placement)>>,
    writer: &'a mut R,
    font: FontRef<'a>,
    quant_size: u16,
    scaled_image: &'a mut GlyphImage,
    scaler: Scaler<'a>,
}

impl<'a, R, A> GlyphAtlasSession<'a, R, A>
where
    R: FontRenderBackend<Allocation = A>,
{
    pub fn get(&mut self, id: GlyphId, x: f32, y: f32) -> &Option<(A, Placement)> {
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
                .format(Format::CustomSubpixel([0.3, 0.0, -0.3]))
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

                let alloc = self.writer.write_bitmap(&bitmap).ok()?;

                Some((alloc, p))
            } else {
                None
            }
        })
    }
}
