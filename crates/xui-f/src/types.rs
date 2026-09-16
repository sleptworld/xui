use xui_interface::RasterizedGlyph;

/// Stable identifier for a face registered in an `FBackend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FFontId(pub(crate) u32);

impl FFontId {
    /// Sentinel returned when supplied font data cannot be parsed.
    pub const INVALID: Self = Self(u32::MAX);

    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Renderer-facing identity of a shaped glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FGlyphKey {
    pub font_id: FFontId,
    pub glyph_id: u32,
    pub font_size_bits: u32,
}

pub(crate) fn no_rasterized_glyph() -> Option<RasterizedGlyph> {
    None
}
